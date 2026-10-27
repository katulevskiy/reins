//! Safety nets (`docs/design/decisions-v2/01-decision-engine.md` §5, 08 R2): what Reins does first so that a request
//! can be undone, and the level the request has once it is netted.
//!
//! A shell command is netted by rewriting it: a **command word** `rm` or `git` (for a nettable git subcommand) gets a
//! prefix, so `rm -rf "$D"` runs as `'/usr/bin/reins' net run --plan ID -- rm -rf "$D"`. Nothing else in the text
//! changes, so quoting, expansion, redirections, loops and functions behave as before, and Reins' `net run` does the
//! same thing with a copy kept (the Reins trash, a backup ref).
//!
//! Two checks make that safe, both with the engine's own reader:
//! - **The rewrite proof.** The rewritten text is parsed again and the prefixes taken out; the syntax tree must be the
//!   original's, with exactly the expected simple commands changed. Otherwise there is no net.
//! - **Attribution.** The original command is read again with the rewritten command words made no-ops (`:`): a delete
//!   that is gone was the rewritten command's, so it is netted. One still there (`xargs rm`, `sh -c 'rm …'`,
//!   `find -delete`, code that deletes) is not, and keeps its level.
//!
//! Pure and IO-free like the rest of the engine: what only the computer knows (whether a path is on the trash's
//! filesystem) comes in through [`NetFacts`].

use super::model::{Area, Certainty, Effect, Level};
use super::{Analysis, Context, analyze_command, analyze_files, level_of};
use crate::shell::ast::{self, Command, Part, Script, Simple, Word};

/// What kind of net.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NetKind {
    /// Deleted paths are moved into the Reins trash.
    Trash,
    /// A backup ref is kept before git drops or rewrites one.
    GitBackup,
    /// A copy of a file is kept before it is overwritten.
    Snapshot,
}

impl NetKind {
    /// The words a person reads.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Trash => "Keeps deleted files in the Reins trash",
            Self::GitBackup => "Keeps a backup ref first",
            Self::Snapshot => "Keeps a copy of changed files",
        }
    }
}

/// What the computer knows about a path's place for a net.
pub trait NetFacts {
    /// `abs` can be moved into the Reins trash: it is on the trash's filesystem, and it is neither the trash, inside
    /// it, nor one of its ancestors.
    fn trashable(&self, abs: &str) -> bool;
}

/// No facts: nothing can be trashed (nets that need no trash still apply).
pub struct NoFacts;

impl NetFacts for NoFacts {
    fn trashable(&self, _abs: &str) -> bool {
        false
    }
}

/// One filesystem for everything, with the trash at `trash` (the replay, tests).
pub struct SameDisk {
    pub trash: String,
}

impl NetFacts for SameDisk {
    fn trashable(&self, abs: &str) -> bool {
        trash_safe(abs, &self.trash)
    }
}

/// `abs` is absolute and is not `/`, the trash, inside it or above it.
#[must_use]
pub fn trash_safe(abs: &str, trash: &str) -> bool {
    let p = abs.trim_end_matches('/');
    let t = trash.trim_end_matches('/');
    abs.starts_with('/')
        && !p.is_empty()
        && !p.split('/').any(|c| c == "..")
        && p != t
        && !p.starts_with(&format!("{t}/"))
        && !t.starts_with(&format!("{p}/"))
}

/// A net a plan carries, in words.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NetUse {
    pub kind: NetKind,
    /// "Keeps 2 deleted paths in the Reins trash".
    pub what: String,
}

/// What a request becomes with its nets.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NetPlan {
    /// The command to run instead (for file tools: the request as it was).
    pub rewritten: String,
    pub nets: Vec<NetUse>,
    /// The request without nets.
    pub before: Analysis,
    /// The same effects, levelled with the nets: what a policy decides on when the net is applied.
    pub after: Analysis,
    /// The paths whose delete or write is netted (normalized), and the refs that get a backup.
    pub netted_paths: Vec<String>,
    pub netted_refs: Vec<String>,
}

impl NetPlan {
    /// One line per net, for the question and Activity.
    #[must_use]
    pub fn lines(&self) -> Vec<String> {
        self.nets.iter().map(|n| n.what.clone()).collect()
    }

    /// The kinds of net, once each.
    #[must_use]
    pub fn kinds(&self) -> Vec<NetKind> {
        let mut k: Vec<NetKind> = self.nets.iter().map(|n| n.kind).collect();
        k.dedup();
        k
    }
}

/// A shell word quoted for any POSIX shell (and zsh, fish): `'…'`, a `'` inside as `'\''`.
#[must_use]
pub fn quote(word: &str) -> String {
    format!("'{}'", word.replace('\'', "'\\''"))
}

/// The prefix put before a netted command word.
#[must_use]
pub fn prefix(reins: &str, plan: &str) -> String {
    format!("{} net run --plan {plan} -- ", quote(reins))
}

/// A plan id: what may follow `--plan` (letters, digits, `-`, `_`; 6 to 64 of them).
#[must_use]
pub fn valid_plan_id(id: &str) -> bool {
    (6..=64).contains(&id.len())
        && id.as_bytes()[0].is_ascii_alphanumeric()
        && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

// ---- The rewrite and its proof ----

/// Calls `f` on every simple command in `s`, words and redirections included (substitutions inside them too).
fn each_simple(s: &mut Script, f: &mut dyn FnMut(&mut Simple)) {
    for ao in &mut s.0 {
        for p in &mut ao.pipelines {
            for c in &mut p.commands {
                each_in_command(c, f);
            }
        }
    }
}

fn each_in_word(w: &mut Word, f: &mut dyn FnMut(&mut Simple)) {
    for part in &mut w.0 {
        match part {
            Part::CmdSub(s) | Part::ProcSub(s) => each_simple(s, f),
            Part::Default(_, w) => each_in_word(w, f),
            Part::Brace(alts) => {
                for a in alts {
                    each_in_word(a, f);
                }
            }
            Part::Lit(_)
            | Part::Quoted(_)
            | Part::Var(_)
            | Part::Param(_)
            | Part::Arith(_)
            | Part::Glob(_)
            | Part::Tilde(_) => {}
        }
    }
}

fn each_in_redirects(r: &mut [ast::Redirect], f: &mut dyn FnMut(&mut Simple)) {
    for x in r {
        each_in_word(&mut x.target, f);
    }
}

fn each_in_command(c: &mut Command, f: &mut dyn FnMut(&mut Simple)) {
    match c {
        Command::Simple(s) => {
            for (_, w) in &mut s.assigns {
                each_in_word(w, f);
            }
            for w in &mut s.words {
                each_in_word(w, f);
            }
            each_in_redirects(&mut s.redirects, f);
            for h in &mut s.heredocs {
                if let Some(w) = &mut h.word {
                    each_in_word(w, f);
                }
            }
            f(s);
        }
        Command::Subshell(s, r) | Command::Group(s, r) => {
            each_simple(s, f);
            each_in_redirects(r, f);
        }
        Command::If {
            branches,
            otherwise,
            redirects,
        } => {
            for (c, b) in branches {
                each_simple(c, f);
                each_simple(b, f);
            }
            if let Some(o) = otherwise {
                each_simple(o, f);
            }
            each_in_redirects(redirects, f);
        }
        Command::For {
            words,
            body,
            redirects,
            ..
        } => {
            for w in words.iter_mut().flatten() {
                each_in_word(w, f);
            }
            each_simple(body, f);
            each_in_redirects(redirects, f);
        }
        Command::While {
            cond,
            body,
            redirects,
        } => {
            each_simple(cond, f);
            each_simple(body, f);
            each_in_redirects(redirects, f);
        }
        Command::Case {
            word,
            arms,
            redirects,
        } => {
            each_in_word(word, f);
            for (pats, body) in arms {
                for p in pats {
                    each_in_word(p, f);
                }
                each_simple(body, f);
            }
            each_in_redirects(redirects, f);
        }
        Command::Function {
            body,
            ..
        } => each_in_command(body, f),
    }
}

/// Takes the net prefixes out of `script`: every simple command whose words start with exactly the prefix's words.
/// Returns how many, and the stripped commands' words (literal or as written).
fn strip(script: &mut Script, reins: &str, plan: &str) -> Vec<Simple> {
    let want = ["net", "run", "--plan", plan, "--"];
    let mut stripped = Vec::new();
    each_simple(script, &mut |s: &mut Simple| {
        let head: Vec<Option<String>> = s.words.iter().take(6).map(Word::literal).collect();
        if head.len() == 6
            && head[0].as_deref() == Some(reins)
            && head[1..].iter().zip(want).all(|(w, x)| w.as_deref() == Some(x))
        {
            s.words.drain(..6);
            stripped.push(s.clone());
        }
    });
    stripped
}

/// Calls `f` on every command in `s`, everywhere one can be: lists, pipelines, compound commands, function bodies of
/// any shape, and substitutions in words, assignments, redirections and here-strings.
fn visit_script(s: &Script, f: &mut dyn FnMut(&Command)) {
    for ao in &s.0 {
        for p in &ao.pipelines {
            for c in &p.commands {
                visit_command(c, f);
            }
        }
    }
}

fn visit_word(w: &Word, f: &mut dyn FnMut(&Command)) {
    for part in &w.0 {
        match part {
            Part::CmdSub(s) | Part::ProcSub(s) => visit_script(s, f),
            Part::Default(_, w) => visit_word(w, f),
            Part::Brace(alts) => alts.iter().for_each(|a| visit_word(a, f)),
            Part::Lit(_)
            | Part::Quoted(_)
            | Part::Var(_)
            | Part::Param(_)
            | Part::Arith(_)
            | Part::Glob(_)
            | Part::Tilde(_) => {}
        }
    }
}

fn visit_redirects(r: &[ast::Redirect], f: &mut dyn FnMut(&Command)) {
    for x in r {
        visit_word(&x.target, f);
    }
}

fn visit_command(c: &Command, f: &mut dyn FnMut(&Command)) {
    f(c);
    match c {
        Command::Simple(s) => {
            s.assigns.iter().for_each(|(_, w)| visit_word(w, f));
            s.words.iter().for_each(|w| visit_word(w, f));
            visit_redirects(&s.redirects, f);
            s.heredocs.iter().filter_map(|h| h.word.as_ref()).for_each(|w| visit_word(w, f));
        }
        Command::Subshell(b, r) | Command::Group(b, r) => {
            visit_script(b, f);
            visit_redirects(r, f);
        }
        Command::If {
            branches,
            otherwise,
            redirects,
        } => {
            for (a, b) in branches {
                visit_script(a, f);
                visit_script(b, f);
            }
            if let Some(o) = otherwise {
                visit_script(o, f);
            }
            visit_redirects(redirects, f);
        }
        Command::For {
            words,
            body,
            redirects,
            ..
        } => {
            words.iter().flatten().for_each(|w| visit_word(w, f));
            visit_script(body, f);
            visit_redirects(redirects, f);
        }
        Command::While {
            cond,
            body,
            redirects,
        } => {
            visit_script(cond, f);
            visit_script(body, f);
            visit_redirects(redirects, f);
        }
        Command::Case {
            word,
            arms,
            redirects,
        } => {
            visit_word(word, f);
            for (pats, body) in arms {
                pats.iter().for_each(|p| visit_word(p, f));
                visit_script(body, f);
            }
            visit_redirects(redirects, f);
        }
        Command::Function {
            body,
            ..
        } => visit_command(body, f),
    }
}

/// Programs that change what a command word runs, or run text as code: with any of them in a script, nothing in it
/// is netted (an alias, a disabled builtin, a function made by `eval` or a sourced file could make `rm`, `git` or the
/// net's own prefix run something else).
const SHADOWING: &[&str] = &["eval", "source", ".", "alias", "unalias", "enable", "hash", "builtin", "command", "trap"];

/// Whether `script` could give a netted command word, or the net's own prefix, another meaning. When in doubt it
/// says yes: the request then keeps its level, which is the safe outcome.
/// - a function named `word`, `reins`, the reins program's file name, a name with a `/` (bash allows one, and it would
///   take a quoted absolute path), or one of [`SHADOWING`], anywhere (substitutions and function bodies included);
/// - any of [`SHADOWING`] run anywhere;
/// - a command word that is not literal (`$E`, `$(…)`): it could be any of them.
fn shadowed(script: &Script, word: &str, reins: &str) -> bool {
    let base = reins.rsplit('/').next().unwrap_or(reins);
    let mut found = false;
    visit_script(script, &mut |c: &Command| match c {
        Command::Function {
            name,
            ..
        } => {
            if name == word
                || name == "reins"
                || name == base
                || name.contains('/')
                || SHADOWING.contains(&name.as_str())
            {
                found = true;
            }
        }
        Command::Simple(s) => match s.words.first().map(Word::literal) {
            Some(Some(w)) if SHADOWING.contains(&w.as_str()) || w == reins || w == base || w == "reins" => {
                found = true;
            }
            Some(None) => found = true,
            _ => {}
        },
        _ => {}
    });
    found
}

/// Places in `text` where `word` stands alone as a token: after the start, a blank, `;&|(){}!` or a backtick, and
/// before the end, a blank or `;&|)`. Candidates only: the proof decides.
fn candidates(text: &str, word: &str) -> Vec<usize> {
    let b = text.as_bytes();
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(i) = text[from..].find(word).map(|i| i + from) {
        let before_ok = i == 0
            || matches!(b[i - 1], b' ' | b'\t' | b'\n' | b';' | b'&' | b'|' | b'(' | b')' | b'{' | b'}' | b'!' | b'`');
        let end = i + word.len();
        let after_ok = end == b.len() || matches!(b[end], b' ' | b'\t' | b'\n' | b';' | b'&' | b'|' | b')');
        if before_ok && after_ok {
            out.push(i);
        }
        from = i + word.len();
    }
    out
}

/// `text` with `insert` put at each of `at` (byte offsets into `text`).
fn insert_at(text: &str, at: &[usize], insert: &str) -> String {
    let mut sorted = at.to_vec();
    sorted.sort_unstable();
    let mut out = String::with_capacity(text.len() + insert.len() * at.len());
    let mut last = 0;
    for i in sorted {
        out.push_str(&text[last..i]);
        out.push_str(insert);
        last = i;
    }
    out.push_str(&text[last..]);
    out
}

/// `text` with the word of length `len` at each of `at` replaced by `with`.
fn replace_at(text: &str, at: &[usize], len: usize, with: &str) -> String {
    let mut sorted = at.to_vec();
    sorted.sort_unstable();
    let mut out = String::with_capacity(text.len());
    let mut last = 0;
    for i in sorted {
        out.push_str(&text[last..i]);
        out.push_str(with);
        last = i + len;
    }
    out.push_str(&text[last..]);
    out
}

/// Whether `rewritten` is `original` with exactly `count` simple commands prefixed, each passing `wanted` once the
/// prefix is out.
fn proven(
    original: &Script,
    rewritten: &str,
    reins: &str,
    plan: &str,
    count: usize,
    wanted: &dyn Fn(&Simple) -> bool,
) -> bool {
    let parsed = ast::parse(rewritten);
    if !parsed.complete {
        return false;
    }
    let mut tree = parsed.script;
    let stripped = strip(&mut tree, reins, plan);
    stripped.len() == count && stripped.iter().all(wanted) && &tree == original
}

/// A rewrite of `command`: each command word `word` (`rm`, `git`) whose simple command passes `wanted` gets the net
/// prefix. `None` when the command can't be read completely, or nothing qualifies. Returns the rewritten text and the
/// byte offsets of the rewritten words in `command`.
#[must_use]
pub fn rewrite(
    command: &str,
    word: &str,
    reins: &str,
    plan: &str,
    wanted: &dyn Fn(&Simple) -> bool,
) -> Option<(String, Vec<usize>)> {
    if command.len() > ast::MAX_INPUT || !valid_plan_id(plan) || reins.is_empty() {
        return None;
    }
    let original = ast::parse(command);
    if !original.complete || shadowed(&original.script, word, reins) {
        return None;
    }
    let pre = prefix(reins, plan);
    let kept: Vec<usize> = candidates(command, word)
        .into_iter()
        .filter(|&i| proven(&original.script, &insert_at(command, &[i], &pre), reins, plan, 1, wanted))
        .collect();
    if kept.is_empty() {
        return None;
    }
    let all = insert_at(command, &kept, &pre);
    proven(&original.script, &all, reins, plan, kept.len(), wanted).then_some((all, kept))
}

// ---- What both the planner and `reins net run` accept ----

/// `rm`'s options as both sides read them (GNU semantics: options anywhere before `--`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[allow(clippy::struct_excessive_bools, reason = "one per rm option")]
pub struct RmArgs {
    pub recursive: bool,
    pub force: bool,
    pub dir: bool,
    pub verbose: bool,
    pub operands: Vec<String>,
}

/// Reads `rm`'s arguments (after the program name). `Err` for an option the net does not model (`-i`, `-I`,
/// `--no-preserve-root`, `--one-file-system`, …): the net never guesses.
pub fn rm_args(args: &[String]) -> Result<RmArgs, String> {
    let mut r = RmArgs::default();
    let mut options = true;
    for a in args {
        if options && a == "--" {
            options = false;
        } else if options && a.starts_with("--") {
            match a.as_str() {
                "--recursive" => r.recursive = true,
                "--force" => r.force = true,
                "--dir" => r.dir = true,
                "--verbose" => r.verbose = true,
                "--interactive=never" | "--preserve-root" => {}
                other => return Err(format!("rm option {other}")),
            }
        } else if options && a.starts_with('-') && a.len() > 1 {
            for c in a[1..].chars() {
                match c {
                    'r' | 'R' => r.recursive = true,
                    'f' => r.force = true,
                    'd' => r.dir = true,
                    'v' => r.verbose = true,
                    other => return Err(format!("rm option -{other}")),
                }
            }
        } else {
            r.operands.push(a.clone());
        }
    }
    Ok(r)
}

/// A git command a net covers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GitNet {
    /// `git branch -D|-d|--delete [-f] [-r] NAMES…`
    BranchDelete,
    /// `git reset --hard [-q] [COMMIT]`
    ResetHard,
    /// `git worktree remove --force… PATH`
    WorktreeRemove,
    /// `git clean -f[dxXq]… [-e PAT] [-- PATHS]`
    Clean,
    /// `git push --force… REMOTE REFSPEC…` (or a `+refspec`)
    ForcePush,
}

/// A git command split: the global options to keep (`-C DIR`, `-c K=V`), the net, and the subcommand's words.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GitShape {
    pub globals: Vec<String>,
    pub net: GitNet,
    /// The words after the subcommand (for `worktree remove`: after `remove`).
    pub rest: Vec<String>,
}

fn branch_like(s: &str) -> bool {
    !s.is_empty()
        && !s.starts_with(['-', '/', '.'])
        && !s.ends_with(['/', '.'])
        && !s.contains("..")
        && !s.contains("@{")
        && !s.to_ascii_lowercase().ends_with(".lock")
        && s.chars().all(|c| c.is_ascii_graphic() && !matches!(c, '~' | '^' | ':' | '?' | '*' | '[' | '\\'))
}

/// The net for git's arguments (after `git`), if they are one of the shapes the net models exactly.
#[must_use]
pub fn git_shape(args: &[String]) -> Option<GitShape> {
    let mut globals = Vec::new();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "-C" | "-c" => {
                globals.push(args[i].clone());
                globals.push(args.get(i + 1)?.clone());
                i += 2;
            }
            _ => break,
        }
    }
    let sub = args.get(i)?.as_str();
    let rest: Vec<String> = args[i + 1..].to_vec();
    let net = match sub {
        "branch" => {
            let mut delete = false;
            let mut names = 0;
            for a in &rest {
                match a.as_str() {
                    "-D" | "-d" | "--delete" => delete = true,
                    "-f" | "--force" | "-q" | "--quiet" | "-r" | "--remotes" => {}
                    n if branch_like(n) => names += 1,
                    _ => return None,
                }
            }
            (delete && names > 0).then_some(GitNet::BranchDelete)?
        }
        "reset" => {
            let mut hard = false;
            let mut commits = 0;
            for a in &rest {
                match a.as_str() {
                    "--hard" => hard = true,
                    "-q" | "--quiet" => {}
                    c if !c.starts_with('-') && c.chars().all(|c| c.is_ascii_graphic()) => commits += 1,
                    _ => return None,
                }
            }
            (hard && commits <= 1).then_some(GitNet::ResetHard)?
        }
        "worktree" => {
            if rest.first().map(String::as_str) != Some("remove") {
                return None;
            }
            let mut force = 0;
            let mut paths = 0;
            for a in &rest[1..] {
                match a.as_str() {
                    "-f" | "--force" => force += 1,
                    "-ff" => force += 2,
                    p if !p.starts_with('-') => paths += 1,
                    _ => return None,
                }
            }
            (force > 0 && paths == 1).then_some(GitNet::WorktreeRemove)?
        }
        "clean" => {
            let mut force = false;
            let mut j = 0;
            let mut dashdash = false;
            while j < rest.len() {
                let a = rest[j].as_str();
                if dashdash || !a.starts_with('-') {
                    // A pathspec: plain paths only (no magic).
                    if a.starts_with(':') {
                        return None;
                    }
                } else if a == "--" {
                    dashdash = true;
                } else if a == "--force" {
                    force = true;
                } else if a == "--quiet" {
                } else if a == "-e" {
                    j += 1;
                    rest.get(j)?;
                } else if a.starts_with("--exclude=") {
                } else if a.len() > 1 && a[1..].chars().all(|c| matches!(c, 'f' | 'd' | 'x' | 'X' | 'q')) {
                    force |= a.contains('f');
                } else {
                    return None;
                }
                j += 1;
            }
            force.then_some(GitNet::Clean)?
        }
        "push" => {
            let mut force = false;
            let mut ops: Vec<&str> = Vec::new();
            for a in &rest {
                match a.as_str() {
                    "-f" | "--force" | "--force-with-lease" | "--force-if-includes" => force = true,
                    "-u" | "--set-upstream" | "-q" | "--quiet" | "-v" | "--verbose" | "--no-verify" => {}
                    l if l.starts_with("--force-with-lease=") => force = true,
                    o if !o.starts_with('-') => ops.push(o),
                    _ => return None,
                }
            }
            let (remote, specs) = ops.split_first()?;
            if !branch_like(remote) || specs.is_empty() {
                return None;
            }
            for s in specs {
                let plus = s.starts_with('+');
                force |= plus;
                let s = s.trim_start_matches('+');
                let (src, dst) = s.split_once(':').unwrap_or((s, s));
                let dst = dst.strip_prefix("refs/heads/").unwrap_or(dst);
                if !(branch_like(src) || src == "HEAD") || !branch_like(dst) || dst.starts_with("refs/") {
                    return None;
                }
            }
            force.then_some(GitNet::ForcePush)?
        }
        _ => return None,
    };
    Some(GitShape {
        globals,
        net,
        rest,
    })
}

// ---- Planning ----

/// Which simple commands a rewrite takes.
type Wanted = fn(&Simple) -> bool;

/// The words of a simple command, literal where they are, `None` for the others.
fn word_texts(s: &Simple) -> Vec<Option<String>> {
    s.words.iter().map(Word::literal).collect()
}

/// An `rm` the net can run: the program is the literal word `rm`, and every literal option is one the net models.
/// Words that are not literal are read at run time (an option they turn into that the net doesn't model is refused
/// there, deleting nothing).
fn rm_wanted(s: &Simple) -> bool {
    let w = word_texts(s);
    if w.first().cloned().flatten().as_deref() != Some("rm") {
        return false;
    }
    let mut options = true;
    for a in &w[1..] {
        match a {
            Some(a) if options && a == "--" => options = false,
            Some(a) if options && a.starts_with('-') && a.len() > 1 && rm_args(std::slice::from_ref(a)).is_err() => {
                return false;
            }
            _ => {}
        }
    }
    true
}

/// A `git` of one of [`git_shape`]'s shapes. The program, the subcommand and every option are literal; a word that is
/// not (`$w` in a loop) stands for an operand, and `reins net run` checks the shape again on the words it really gets,
/// running nothing when they are not one.
fn git_wanted(s: &Simple) -> bool {
    let w: Vec<String> = word_texts(s).into_iter().map(|w| w.unwrap_or_else(|| "reins-operand".to_owned())).collect();
    w.first().map(String::as_str) == Some("git")
        && w.get(1).is_some_and(|g| g != "reins-operand")
        && git_shape(&w[1..]).is_some()
}

/// The paths a set of effects deletes (normalized as the engine wrote them).
fn deleted_paths(a: &Analysis) -> Vec<String> {
    a.effects
        .iter()
        .flat_map(|e| match e {
            Effect::FileDelete {
                paths,
                ..
            } => paths.iter().map(|p| p.path.clone()).collect(),
            _ => Vec::new(),
        })
        .collect()
}

fn ref_keys(a: &Analysis) -> Vec<String> {
    a.effects
        .iter()
        .filter_map(|e| match e {
            Effect::GitRefDelete {
                repo,
                refname,
                ..
            }
            | Effect::GitPush {
                repo,
                refname,
                rewrites: Some(true) | None,
                ..
            } => Some(format!("{repo} {refname}")),
            _ => None,
        })
        .collect()
}

/// An area whose delete or overwrite a net lowers to recoverable (01 §2.5: workspace source and the home folder).
fn lowerable(area: &Area) -> bool {
    matches!(area, Area::Source | Area::Home)
}

/// One effect's level when the paths in `netted` are netted (deletes and writes there are recoverable).
#[must_use]
pub fn netted_level(e: &Effect, netted: &[String]) -> Level {
    match e {
        Effect::FileDelete {
            paths,
            recursive,
        } => paths
            .iter()
            .map(|p| {
                if lowerable(&p.area) && netted.contains(&p.path) {
                    Level::L1
                } else {
                    level_of(&Effect::FileDelete {
                        paths: vec![p.clone()],
                        recursive: *recursive,
                    })
                }
            })
            .max()
            .unwrap_or(Level::L0),
        Effect::FileWrite {
            path,
        } if lowerable(&path.area) && netted.contains(&path.path) => Level::L1.min(level_of(e)),
        _ => level_of(e),
    }
}

/// `before` levelled with `netted` paths: the same effects, certainty, floor and unreadable parts.
fn levelled(before: &Analysis, netted: &[String]) -> Analysis {
    let level = before.effects.iter().map(|e| netted_level(e, netted)).max().unwrap_or(Level::L0);
    // Partial certainty is still the worst plausible effect in the workspace (01 §2.4).
    let level = if before.certainty == Certainty::Full {
        level
    } else {
        level.max(Level::L1)
    };
    Analysis {
        level,
        ..before.clone()
    }
}

fn count(n: usize, one: &str, many: &str) -> String {
    if n == 1 {
        format!("1 {one}")
    } else {
        format!("{n} {many}")
    }
}

/// Plans the nets for a shell command run in `ctx`. `reins` is the absolute path of the `reins` program the rewritten
/// command calls, `plan` the plan's id. `None`: no net applies (the request keeps its level).
#[must_use]
pub fn plan_command(command: &str, ctx: &Context, facts: &dyn NetFacts, reins: &str, plan: &str) -> Option<NetPlan> {
    let before = analyze_command(command, ctx);
    let before_paths = deleted_paths(&before);
    let before_refs = ref_keys(&before);
    // Each candidate command word on its own: what it alone deletes or drops (the analysis without it).
    let mut kept: Vec<(usize, usize)> = Vec::new(); // (offset, word length)
    let mut netted_paths: Vec<String> = Vec::new();
    let mut netted_refs: Vec<String> = Vec::new();
    let mut kinds: Vec<NetKind> = Vec::new();
    let words: [(&str, Wanted); 2] = [("rm", rm_wanted), ("git", git_wanted)];
    for (word, wanted) in words {
        let Some((_, offsets)) = rewrite(command, word, reins, plan, &wanted) else {
            continue;
        };
        for at in offsets {
            let masked = analyze_command(&replace_at(command, &[at], word.len(), ":"), ctx);
            let still = deleted_paths(&masked);
            let still_refs = ref_keys(&masked);
            let mine: Vec<String> = before_paths.iter().filter(|p| !still.contains(p)).cloned().collect();
            let refs: Vec<String> = before_refs.iter().filter(|r| !still_refs.contains(r)).cloned().collect();
            // A delete the net keeps must be trashable; one that can't be makes the whole command un-netted (the
            // runtime would refuse it and delete nothing, which is not the command the agent wrote).
            // Scratch (the OS temp folders, `~/.cache`, configured scratch roots) is deleted for real at run time.
            if mine
                .iter()
                .any(|p| p.starts_with('/') && path_area(&before, p) != Some(Area::Scratch) && !facts.trashable(p))
            {
                continue;
            }
            let worth = mine.iter().any(|p| path_area(&before, p).is_some_and(|a| lowerable(&a))) || !refs.is_empty();
            if !worth {
                continue;
            }
            kept.push((at, word.len()));
            let shape = (word == "git").then(|| shape_at(command, at)).flatten();
            match shape {
                Some(GitNet::BranchDelete | GitNet::ResetHard | GitNet::ForcePush) => kinds.push(NetKind::GitBackup),
                Some(GitNet::WorktreeRemove | GitNet::Clean) | None => kinds.push(NetKind::Trash),
            }
            netted_paths.extend(mine);
            netted_refs.extend(refs);
        }
    }
    if kept.is_empty() {
        return None;
    }
    let offsets: Vec<usize> = kept.iter().map(|(at, _)| *at).collect();
    let rewritten = insert_at(command, &offsets, &prefix(reins, plan));
    // The whole rewrite, proven once more: every kept word, nothing else.
    let original = ast::parse(command);
    let any = |s: &Simple| rm_wanted(s) || git_wanted(s);
    if !proven(&original.script, &rewritten, reins, plan, kept.len(), &any) {
        return None;
    }
    netted_paths.sort();
    netted_paths.dedup();
    netted_refs.sort();
    netted_refs.dedup();
    let after = levelled(&before, &netted_paths);
    let mut nets = Vec::new();
    kinds.sort_by_key(|k| *k as u8);
    kinds.dedup();
    for k in kinds {
        let what = match k {
            NetKind::Trash => format!(
                "Keeps {} in the Reins trash for 7 days",
                count(netted_paths.len(), "deleted path", "deleted paths")
            ),
            NetKind::GitBackup => "Keeps a backup ref for 30 days first".to_owned(),
            NetKind::Snapshot => k.label().to_owned(),
        };
        nets.push(NetUse {
            kind: k,
            what,
        });
    }
    Some(NetPlan {
        rewritten,
        nets,
        before,
        after,
        netted_paths,
        netted_refs,
    })
}

/// The git net of the command that starts at `at` (for its words only).
fn shape_at(command: &str, at: usize) -> Option<GitNet> {
    let parsed = ast::parse(&command[at..]);
    let first = parsed.script.0.first()?.pipelines.first()?.commands.first()?;
    match first {
        Command::Simple(s) => {
            let w: Vec<String> =
                word_texts(s).into_iter().map(|w| w.unwrap_or_else(|| "reins-operand".to_owned())).collect();
            git_shape(w.get(1..)?).map(|g| g.net)
        }
        _ => None,
    }
}

fn path_area(a: &Analysis, path: &str) -> Option<Area> {
    a.effects.iter().find_map(|e| match e {
        Effect::FileDelete {
            paths,
            ..
        } => paths.iter().find(|p| p.path == path).map(|p| p.area.clone()),
        Effect::FileWrite {
            path: p,
        } if p.path == path => Some(p.area.clone()),
        _ => None,
    })
}

/// Plans snapshots for a file tool that writes `paths` (Write, Edit, apply_patch, Delete): each one in workspace source
/// or the home folder that is trashable (the snapshot store is the trash's filesystem) is netted. The hook takes the
/// snapshots right before the tool runs, so nothing is rewritten.
#[must_use]
pub fn plan_files(paths: &[&str], ctx: &Context, facts: &dyn NetFacts) -> Option<NetPlan> {
    let before = analyze_files(paths, true, ctx);
    let netted: Vec<String> = before
        .effects
        .iter()
        .filter_map(|e| match e {
            Effect::FileWrite {
                path,
            } if lowerable(&path.area) && facts.trashable(&path.path) => Some(path.path.clone()),
            Effect::FileDelete {
                paths,
                ..
            } => paths.iter().find(|p| lowerable(&p.area) && facts.trashable(&p.path)).map(|p| p.path.clone()),
            _ => None,
        })
        .collect();
    if netted.is_empty() {
        return None;
    }
    let after = levelled(&before, &netted);
    Some(NetPlan {
        rewritten: String::new(),
        nets: vec![NetUse {
            kind: NetKind::Snapshot,
            what: format!("Keeps a copy of {} for 7 days", count(netted.len(), "file", "files")),
        }],
        before,
        after,
        netted_paths: netted,
        netted_refs: Vec::new(),
    })
}

#[cfg(test)]
mod tests;
