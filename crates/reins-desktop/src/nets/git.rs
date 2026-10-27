//! git with a net: the shapes `reins_policy::effects::nets::git_shape` reads (branch deletes, hard resets, worktree
//! removals, cleans, force pushes). Each keeps what it would lose first, then runs git with exactly the words it got
//! (a clean moves exactly what `git clean -n` lists into the trash instead of running git clean). Any other shape, or a
//! net that can't be made, runs nothing.

use std::ffi::OsString;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use reins_policy::effects::nets::{GitNet, GitShape, git_shape};

use super::store::{Item, NetError, RefBackup, Store, WorktreeInfo};

/// Exit code when the net refuses to run anything.
pub const REFUSED: u8 = 125;

/// The git a netted command runs and its environment, as the hook recorded them in the plan (`reins net run` sets it
/// once, before anything runs): an absolute program, and exactly the hook's `GIT_*`, `HOME` and `XDG_*` values, so
/// nothing the command's own environment says can change which git runs or what it reads.
#[derive(Clone, Debug, Default)]
pub struct GitEnv {
    pub program: String,
    pub vars: Vec<(String, String)>,
}

static GIT_ENV: std::sync::OnceLock<GitEnv> = std::sync::OnceLock::new();

/// Sets the git and environment every git this process runs uses (once; later calls change nothing).
pub fn use_env(env: GitEnv) {
    let _set = GIT_ENV.set(env);
}

/// A git command with the plan's program and environment (without one set: `git` as found, as the process has it).
fn git_command() -> Command {
    let Some(env) = GIT_ENV.get() else {
        return Command::new("git");
    };
    let mut c = Command::new(if env.program.is_empty() {
        "git"
    } else {
        env.program.as_str()
    });
    for (k, _) in std::env::vars_os() {
        let k = k.to_string_lossy();
        if k.starts_with("GIT_") || k.starts_with("XDG_") || k.starts_with("REINS_") || k == "HOME" {
            c.env_remove(k.as_ref());
        }
    }
    c.envs(env.vars.iter().map(|(k, v)| (k.as_str(), v.as_str())));
    c
}

fn git_in(globals: &[String], cwd: &Path) -> Command {
    let mut c = git_command();
    c.current_dir(cwd).args(globals);
    c
}

/// Runs git quietly; its trimmed stdout when it succeeds.
fn read(globals: &[String], cwd: &Path, args: &[&str]) -> Option<String> {
    let out = git_in(globals, cwd).args(args).stdin(Stdio::null()).stderr(Stdio::null()).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim_end_matches('\n').to_owned())
}

/// Runs git with the user's own words, its output passed through; its exit code.
fn run_as_written(args: &[String], cwd: &Path) -> u8 {
    match git_command().current_dir(cwd).args(args).status() {
        Ok(s) => exit_code(s),
        Err(_) => 127,
    }
}

#[cfg(unix)]
fn exit_code(s: std::process::ExitStatus) -> u8 {
    use std::os::unix::process::ExitStatusExt as _;
    s.code().map_or_else(
        || s.signal().map_or(1, |n| 128u8.wrapping_add(u8::try_from(n).unwrap_or(1))),
        |c| u8::try_from(c).unwrap_or(1),
    )
}

#[cfg(not(unix))]
fn exit_code(s: std::process::ExitStatus) -> u8 {
    s.code().map_or(1, |c| u8::try_from(c).unwrap_or(1))
}

/// The folder git works in: `cwd` moved by each `-C`.
fn base(globals: &[String], cwd: &Path) -> PathBuf {
    let mut b = cwd.to_path_buf();
    let mut i = 0;
    while i + 1 < globals.len() {
        if globals[i] == "-C" {
            b = b.join(&globals[i + 1]);
        }
        i += 2;
    }
    b
}

/// The repository's shared git folder, absolute (backup refs live there, for every worktree).
fn common_dir(globals: &[String], cwd: &Path) -> Option<String> {
    let d = read(globals, cwd, &["rev-parse", "--path-format=absolute", "--git-common-dir"])?;
    (!d.is_empty()).then_some(d)
}

/// Keeps `oid` under a new backup ref of `plan` and records it.
fn keep_ref(
    store: &Store,
    plan: &str,
    globals: &[String],
    cwd: &Path,
    oid: &str,
    what: &str,
    name: &str,
) -> Result<(), String> {
    let repo = common_dir(globals, cwd).ok_or("not in a git repository")?;
    let backup = Store::backup_ref(plan);
    read(&[], cwd, &["--git-dir", &repo, "update-ref", &backup, oid]).ok_or("git could not write the backup ref")?;
    store
        .record_ref(
            plan,
            &RefBackup {
                at: crate::now_unix(),
                repo,
                backup,
                oid: oid.to_owned(),
                what: what.to_owned(),
                name: name.to_owned(),
            },
        )
        .map_err(|e| e.to_string())
}

/// Runs `git ARGS` in `cwd` with the net of `plan`. Returns the exit code.
pub fn git(store: &Store, plan: &str, args: &[OsString], cwd: &Path, err: &mut dyn Write) -> u8 {
    let Some(words) = args.iter().map(|a| a.to_str().map(str::to_owned)).collect::<Option<Vec<String>>>() else {
        let _said = writeln!(err, "reins: git words that are not text can't be netted; nothing was run.");
        return REFUSED;
    };
    let Some(shape) = git_shape(&words) else {
        let _said =
            writeln!(err, "reins: this git command can't be netted; nothing was run. Run it again to be asked.");
        store.fail_plan(plan, "not a git command the net knows");
        return REFUSED;
    };
    let mut words = words;
    let kept = match shape.net {
        GitNet::BranchDelete => branch_delete(store, plan, &shape, cwd),
        GitNet::ResetHard => return reset_hard(store, plan, &shape, &words, cwd, err),
        GitNet::WorktreeRemove => return worktree_remove(store, plan, &shape, &words, cwd, err),
        GitNet::Clean => return clean(store, plan, &shape, cwd, err),
        // The push is pinned to the tips it kept: its words, with a lease per destination after `push`.
        // Kept first, then pinned: the leases go after everything the user wrote.
        GitNet::ForcePush => force_push(store, plan, &shape, cwd).and_then(|kept| pins(&shape, cwd, &kept)).map(|p| {
            words.extend(p);
        }),
    };
    if let Err(why) = kept {
        let _said = writeln!(
            err,
            "reins: couldn't keep a backup ({why}), so git was not run. Run the command again to be asked."
        );
        store.fail_plan(plan, &why);
        return REFUSED;
    }
    run_as_written(&words, cwd)
}

fn operands(rest: &[String]) -> Vec<&str> {
    rest.iter().map(String::as_str).filter(|a| !a.starts_with('-')).collect()
}

fn branch_delete(store: &Store, plan: &str, shape: &GitShape, cwd: &Path) -> Result<(), String> {
    let remote = shape.rest.iter().any(|a| a == "-r" || a == "--remotes");
    for name in operands(&shape.rest) {
        let full = if remote {
            format!("refs/remotes/{name}")
        } else {
            format!("refs/heads/{name}")
        };
        // A branch that is not there is git's own error to give; nothing to keep.
        if let Some(oid) =
            read(&shape.globals, cwd, &["rev-parse", "--verify", "--quiet", &format!("{full}^{{commit}}")])
        {
            let what = if remote {
                "remote-branch"
            } else {
                "branch"
            };
            keep_ref(store, plan, &shape.globals, cwd, &oid, what, name)?;
        }
    }
    Ok(())
}

/// NUL-separated output as names.
fn names(text: &str) -> Vec<String> {
    text.split('\0').filter(|s| !s.is_empty()).map(str::to_owned).collect()
}

fn reset_hard(store: &Store, plan: &str, shape: &GitShape, words: &[String], cwd: &Path, err: &mut dyn Write) -> u8 {
    let target = operands(&shape.rest).first().copied().unwrap_or("HEAD").to_owned();
    let (Some(top), Some(head)) = (
        read(&shape.globals, cwd, &["rev-parse", "--show-toplevel"]),
        read(&shape.globals, cwd, &["rev-parse", "--verify", "--quiet", "HEAD"]),
    ) else {
        // Not a repository, or no commit yet: git says so, and loses nothing.
        return run_as_written(words, cwd);
    };
    if read(&shape.globals, cwd, &["rev-parse", "--verify", "--quiet", &format!("{target}^{{commit}}")]).is_none() {
        return run_as_written(words, cwd);
    }
    let top = PathBuf::from(top);
    let branch =
        read(&shape.globals, cwd, &["symbolic-ref", "--short", "-q", "HEAD"]).unwrap_or_else(|| "HEAD".to_owned());
    let kept = keep_before_reset(store, plan, shape, cwd, &top, &head, &branch, &target);
    let items = match kept {
        Ok(items) => items,
        Err(why) => {
            let _said = writeln!(
                err,
                "reins: couldn't keep a backup ({why}), so git was not run. Run the command again to be asked."
            );
            store.fail_plan(plan, &why);
            return REFUSED;
        }
    };
    let code = run_as_written(words, cwd);
    if code != 0 {
        // git failed: what was moved aside goes back.
        for i in &items {
            let _back = store.restore_item(plan, i);
        }
    }
    code
}

/// What a hard reset would lose, kept: HEAD's commit, the work tree's changes, and the untracked files the target
/// tracks (moved into the trash; put back if a later step fails).
#[allow(clippy::too_many_arguments, reason = "the reset's facts, read once")]
fn keep_before_reset(
    store: &Store,
    plan: &str,
    shape: &GitShape,
    cwd: &Path,
    top: &Path,
    head: &str,
    branch: &str,
    target: &str,
) -> Result<Vec<Item>, String> {
    keep_ref(store, plan, &shape.globals, cwd, head, "head", branch)?;
    let ident: Vec<String> =
        ["-c", "user.name=Reins", "-c", "user.email=nets@reins.invalid"].map(str::to_owned).to_vec();
    let stash =
        read(&[&shape.globals[..], &ident[..]].concat(), cwd, &["stash", "create"]).ok_or("git stash create failed")?;
    if !stash.is_empty() {
        keep_ref(store, plan, &shape.globals, cwd, &stash, "stash", branch)?;
    }
    let top_globals = vec!["-C".to_owned(), top.to_string_lossy().into_owned()];
    let mut items = Vec::new();
    // What the reset writes over without git knowing it: a path the target tracks (or a folder above one) that is on
    // disk but not in the index, ignored or not. Read from the target's list and the index, never by listing every
    // untracked file (a `node_modules` would be walked for nothing).
    let tracked = read(&top_globals, cwd, &["ls-tree", "-r", "-z", "--name-only", target])
        .map(|t| names(&t))
        .ok_or("git ls-tree failed")?;
    let index: std::collections::HashSet<String> = read(&top_globals, cwd, &["ls-files", "-z"])
        .map(|t| names(&t))
        .ok_or("git ls-files failed")?
        .into_iter()
        .collect();
    let index_dirs: std::collections::HashSet<String> =
        index.iter().flat_map(|f| f.match_indices('/').map(move |(i, _)| f[..i].to_owned())).collect();
    let mut seen = std::collections::HashSet::new();
    let mut untracked = Vec::new();
    for t in &tracked {
        // A folder above it is only in the way when it is not a folder on disk (a file or a link where git needs one).
        let ancestors = t.match_indices('/').map(|(i, _)| (&t[..i], false)).chain(std::iter::once((t.as_str(), true)));
        for (p, whole) in ancestors {
            if !seen.insert(p.to_owned()) || index.contains(p) || index_dirs.contains(p) {
                continue;
            }
            match std::fs::symlink_metadata(top.join(p)) {
                Ok(m) if whole || !m.is_dir() => {
                    untracked.push(p.to_owned());
                    break;
                }
                _ => {}
            }
        }
    }
    for f in &untracked {
        match store.trash(plan, &top.join(f), None) {
            Ok(i) => items.push(i),
            Err(e) => {
                for i in &items {
                    let _back = store.restore_item(plan, i);
                }
                return Err(format!("{f}: {e}"));
            }
        }
    }
    Ok(items)
}

/// `git worktree list --porcelain`: (path, head, branch, locked) per worktree, the main one first.
fn worktrees(globals: &[String], cwd: &Path) -> Vec<(String, String, Option<String>, bool)> {
    let text = read(globals, cwd, &["worktree", "list", "--porcelain"]).unwrap_or_default();
    text.split("\n\n")
        .filter_map(|block| {
            let mut path = None;
            let mut head = String::new();
            let mut branch = None;
            let mut locked = false;
            for l in block.lines() {
                if let Some(p) = l.strip_prefix("worktree ") {
                    path = Some(p.to_owned());
                } else if let Some(h) = l.strip_prefix("HEAD ") {
                    h.clone_into(&mut head);
                } else if let Some(b) = l.strip_prefix("branch refs/heads/") {
                    branch = Some(b.to_owned());
                } else if l == "locked" || l.starts_with("locked ") {
                    locked = true;
                }
            }
            path.map(|p| (p, head, branch, locked))
        })
        .collect()
}

fn worktree_remove(
    store: &Store,
    plan: &str,
    shape: &GitShape,
    words: &[String],
    cwd: &Path,
    err: &mut dyn Write,
) -> u8 {
    let force: usize = shape
        .rest
        .iter()
        .map(|a| match a.as_str() {
            "-f" | "--force" => 1,
            "-ff" => 2,
            _ => 0,
        })
        .sum();
    let Some(arg) = operands(&shape.rest).get(1).map(|a| (*a).to_owned()) else {
        return refuse(store, plan, err, "no worktree named");
    };
    let list = worktrees(&shape.globals, cwd);
    let Some(index) = resolve_worktree(&list, &base(&shape.globals, cwd), &arg) else {
        // git would look further (or fail); the net only removes what it resolved itself.
        return refuse(store, plan, err, &format!("{arg} is not a worktree Reins can find"));
    };
    let (path, head, branch, locked) = &list[index];
    if index == 0 || (*locked && force < 2) {
        // git refuses the main worktree, and a locked one without a second --force: nothing is lost.
        return run_as_written(words, cwd);
    }
    let Ok(real) = std::fs::canonicalize(path) else {
        // Its folder is gone already: git removes only its record, nothing on disk is lost.
        return run_as_written(words, cwd);
    };
    let info = WorktreeInfo {
        repo: list[0].0.clone(),
        head: head.clone(),
        branch: branch.clone(),
    };
    let item = match store.trash(plan, &real, Some(info)) {
        Ok(i) => i,
        Err(e) => {
            let _said = writeln!(
                err,
                "reins: couldn't keep a copy of the worktree ({e}), so git was not run. Run the command again to be asked."
            );
            store.fail_plan(plan, &e.to_string());
            return REFUSED;
        }
    };
    // The folder is in the trash: git now only drops its record of the worktree.
    let code = run_as_written(words, cwd);
    if code != 0 {
        let _back = store.restore_item(plan, &item);
    }
    code
}

/// Which of `list` (`worktree list --porcelain`, the main one first) `arg` names, as git reads it: the worktree at
/// that path (from `base`), else the only one whose path ends with `/arg`. `None`: none, or more than one.
fn resolve_worktree(list: &[(String, String, Option<String>, bool)], base: &Path, arg: &str) -> Option<usize> {
    let target = base.join(arg);
    let real = std::fs::canonicalize(&target).ok();
    let lexical = reins_policy::effects::normalize(&target.to_string_lossy(), "/", "");
    if let Some(i) = list.iter().position(|w| {
        Path::new(&w.0) == Path::new(&lexical)
            || real.as_deref().is_some_and(|r| std::fs::canonicalize(&w.0).ok().as_deref() == Some(r))
    }) {
        return Some(i);
    }
    let suffix = format!("/{}", arg.trim_end_matches('/'));
    let ends: Vec<usize> =
        list.iter().enumerate().skip(1).filter(|(_, w)| w.0.ends_with(&suffix)).map(|(i, _)| i).collect();
    (ends.len() == 1).then(|| ends[0])
}

/// Runs nothing: says why, records the failure (the same command is asked without a net next time).
fn refuse(store: &Store, plan: &str, err: &mut dyn Write, why: &str) -> u8 {
    let _said = writeln!(err, "reins: {why}, so git was not run. Run the command again to be asked.");
    store.fail_plan(plan, why);
    REFUSED
}

/// A path as `git clean -n` prints it: plain, or C-quoted (`"a\tb"`).
fn unquote(s: &str) -> Option<String> {
    let Some(inner) = s.strip_prefix('"').and_then(|r| r.strip_suffix('"')) else {
        return Some(s.to_owned());
    };
    let mut bytes = Vec::new();
    let mut it = inner.bytes().peekable();
    while let Some(b) = it.next() {
        if b != b'\\' {
            bytes.push(b);
            continue;
        }
        let e = it.next()?;
        bytes.push(match e {
            b'n' => b'\n',
            b't' => b'\t',
            b'r' => b'\r',
            b'a' => 7,
            b'b' => 8,
            b'f' => 12,
            b'v' => 11,
            b'\\' => b'\\',
            b'"' => b'"',
            b'0'..=b'7' => {
                let mut v = u32::from(e - b'0');
                for _ in 0..2 {
                    let d = *it.peek()?;
                    if !(b'0'..=b'7').contains(&d) {
                        return None;
                    }
                    v = v * 8 + u32::from(d - b'0');
                    it.next();
                }
                u8::try_from(v).ok()?
            }
            _ => return None,
        });
    }
    String::from_utf8(bytes).ok()
}

fn clean(store: &Store, plan: &str, shape: &GitShape, cwd: &Path, err: &mut dyn Write) -> u8 {
    let mut quiet = false;
    let mut dry: Vec<String> = vec!["clean".to_owned(), "-n".to_owned()];
    let mut options = true;
    for a in &shape.rest {
        if !options || !a.starts_with('-') || a.starts_with("--exclude=") {
            dry.push(a.clone());
            continue;
        }
        match a.as_str() {
            "--" => {
                options = false;
                dry.push(a.clone());
            }
            "--force" => {}
            "--quiet" => quiet = true,
            "-e" => dry.push(a.clone()),
            short if short.len() > 1 && short[1..].chars().all(|c| matches!(c, 'f' | 'd' | 'x' | 'X' | 'q')) => {
                quiet |= short.contains('q');
                let kept: String = short[1..].chars().filter(|c| matches!(c, 'd' | 'x' | 'X')).collect();
                if !kept.is_empty() {
                    dry.push(format!("-{kept}"));
                }
            }
            other => dry.push(other.to_owned()),
        }
    }
    let dry_refs: Vec<&str> = dry.iter().map(String::as_str).collect();
    let Some(list) = read(&shape.globals, cwd, &dry_refs) else {
        let _said =
            writeln!(err, "reins: git clean -n failed, so nothing was removed. Run the command again to be asked.");
        store.fail_plan(plan, "git clean -n failed");
        return REFUSED;
    };
    let at = base(&shape.globals, cwd);
    let mut paths = Vec::new();
    for line in list.lines() {
        let Some(p) = line.strip_prefix("Would remove ") else {
            continue;
        };
        let Some(p) = unquote(p) else {
            let _said = writeln!(err, "reins: couldn't read git's list of files, so nothing was removed.");
            store.fail_plan(plan, "an unreadable name in git clean -n");
            return REFUSED;
        };
        paths.push(p);
    }
    for p in paths {
        match store.trash(plan, &at.join(p.trim_end_matches('/')), None) {
            Ok(_) | Err(NetError::Missing) => {
                if !quiet {
                    let _said = writeln!(std::io::stdout(), "Removing {p}");
                }
            }
            Err(e) => {
                let _said = writeln!(
                    err,
                    "reins: couldn't keep a copy of '{p}' ({e}), so it was not removed. Run the command again to be asked."
                );
                store.fail_plan(plan, &e.to_string());
                return 1;
            }
        }
    }
    0
}

/// Keeps what a force push would overwrite: for each destination branch, the remote's tip now, fetched from the push
/// URL into a backup ref (by URL, so no remote-tracking ref moves and a bare `--force-with-lease` still means what it
/// did). Returns the leases that pin the push to exactly those tips (`refs/heads/X:<oid>`, or `refs/heads/X:` for one
/// that isn't there yet), so a tip that moves in between makes the push fail instead of going unkept.
fn force_push(
    store: &Store,
    plan: &str,
    shape: &GitShape,
    cwd: &Path,
) -> Result<Vec<(String, Option<String>)>, String> {
    let ops = operands(&shape.rest);
    let Some((remote, specs)) = ops.split_first() else {
        return Err("no remote".to_owned());
    };
    let url = match read(&shape.globals, cwd, &["remote", "get-url", "--push", "--all", remote]) {
        Some(urls) => {
            let urls: Vec<&str> = urls.lines().filter(|l| !l.trim().is_empty()).collect();
            match urls.as_slice() {
                [one] => (*one).to_owned(),
                _ => return Err(format!("{remote} pushes to several places")),
            }
        }
        // Not a configured remote: a URL or a path given as it is.
        None => (*remote).to_owned(),
    };
    let mut leases = Vec::new();
    for spec in specs {
        let s = spec.trim_start_matches('+');
        let (src, dst) = s.split_once(':').unwrap_or((s, s));
        let dst = dst.strip_prefix("refs/heads/").unwrap_or(dst).to_owned();
        let dst = if dst == "HEAD" || src == "HEAD" && !s.contains(':') {
            read(&shape.globals, cwd, &["symbolic-ref", "--short", "-q", "HEAD"]).ok_or("HEAD is not on a branch")?
        } else {
            dst
        };
        let full = format!("refs/heads/{dst}");
        let tag = format!("refs/tags/{dst}");
        let listed = read(&shape.globals, cwd, &["ls-remote", &url, &full, &tag])
            .ok_or_else(|| format!("couldn't read {dst} on {remote}"))?;
        let mut head = None;
        for line in listed.lines() {
            match line.split_once('\t') {
                Some((oid, name)) if name == full => head = Some(oid.to_owned()),
                // A bare name a tag there answers to too: git may update the tag, which the net doesn't keep.
                Some((_, name)) if name == tag => return Err(format!("{dst} is also a tag on {remote}")),
                _ => {}
            }
        }
        let Some(old) = head else {
            // No such branch there yet: nothing to lose, and it must still not be there when the push lands.
            leases.push((dst, None));
            continue;
        };
        let backup = Store::backup_ref(plan);
        read(
            &shape.globals,
            cwd,
            &["fetch", "--no-tags", "--no-write-fetch-head", "--quiet", &url, &format!("+{full}:{backup}")],
        )
        .ok_or_else(|| format!("couldn't fetch {dst} from {remote}"))?;
        let oid = read(&shape.globals, cwd, &["rev-parse", "--verify", &backup]).ok_or("the fetched tip is missing")?;
        if oid != old {
            return Err(format!("{dst} moved on {remote} while it was kept"));
        }
        let repo = common_dir(&shape.globals, cwd).ok_or("not in a git repository")?;
        store
            .record_ref(
                plan,
                &RefBackup {
                    at: crate::now_unix(),
                    repo,
                    backup,
                    oid: oid.clone(),
                    what: "remote".to_owned(),
                    name: format!("{remote}/{dst}"),
                },
            )
            .map_err(|e| e.to_string())?;
        leases.push((dst, Some(oid)));
    }
    Ok(leases)
}

/// The leases that pin a force push to what the net kept, appended after the user's own words (so nothing the user
/// wrote can undo them). Per destination branch:
/// - the user's own `--force-with-lease=<ref>:<oid>` for it: theirs decides, no pin;
/// - the user's bare `--force-with-lease`, or `--force-with-lease=<ref>`: pinned to what that lease expects, the
///   remote-tracking ref (no tracking ref: not netted);
/// - no lease: pinned to the tip the net kept (or to "not there" for a new branch).
pub(crate) fn pins(shape: &GitShape, cwd: &Path, kept: &[(String, Option<String>)]) -> Result<Vec<String>, String> {
    let remote = operands(&shape.rest).first().copied().unwrap_or_default().to_owned();
    let short = |r: &str| r.strip_prefix("refs/heads/").unwrap_or(r).to_owned();
    let mut bare = false;
    let mut tracked: Vec<String> = Vec::new();
    let mut explicit: Vec<String> = Vec::new();
    for a in &shape.rest {
        if a == "--force-with-lease" {
            bare = true;
        } else if let Some(spec) = a.strip_prefix("--force-with-lease=") {
            match spec.split_once(':') {
                Some((r, _)) => explicit.push(short(r)),
                None => tracked.push(short(spec)),
            }
        }
    }
    let mut out = Vec::new();
    for (dst, oid) in kept {
        if explicit.contains(dst) {
            continue;
        }
        let expect = if bare || tracked.contains(dst) {
            let tracking = format!("refs/remotes/{remote}/{dst}");
            Some(
                read(&shape.globals, cwd, &["rev-parse", "--verify", "--quiet", &tracking])
                    .ok_or_else(|| format!("no {tracking} for the lease to go by"))?,
            )
        } else {
            oid.clone()
        };
        out.push(format!("--force-with-lease=refs/heads/{dst}:{}", expect.unwrap_or_default()));
    }
    Ok(out)
}

/// A branch name that is free in `repo`: `name`, else `name-restored`, `name-restored-2`, …
fn free_branch(repo: &str, name: &str) -> String {
    let taken = |n: &str| {
        read(&[], Path::new("/"), &["--git-dir", repo, "rev-parse", "--verify", "--quiet", &format!("refs/heads/{n}")])
            .is_some()
    };
    if !taken(name) {
        return name.to_owned();
    }
    let mut n = format!("{name}-restored");
    let mut i = 2;
    while taken(&n) {
        n = format!("{name}-restored-{i}");
        i += 1;
    }
    n
}

/// Brings back what a backup ref kept, without changing anything else: a deleted branch as itself (or
/// `<name>-restored`), the commit before a hard reset as a branch, the work tree's changes as a stash entry, a remote
/// branch's old tip as a local branch. Nothing is pushed.
pub fn restore_ref(plan: &str, r: &RefBackup) -> Result<String, String> {
    let root = Path::new("/");
    let git = |args: &[&str]| -> Result<(), String> {
        let mut all = vec!["--git-dir", r.repo.as_str()];
        all.extend_from_slice(args);
        read(&[], root, &all).map(|_| ()).ok_or_else(|| format!("git {} failed", args.join(" ")))
    };
    match r.what.as_str() {
        "branch" => {
            let name = free_branch(&r.repo, &r.name);
            git(&["branch", &name, &r.oid])?;
            Ok(format!("Branch {name} is back"))
        }
        "stash" => {
            git(&["stash", "store", "-m", &format!("reins: work tree before reset --hard ({plan})"), &r.oid])?;
            Ok("The changes before the reset are in git stash".to_owned())
        }
        "head" | "remote" | "remote-branch" => {
            let base = match r.what.as_str() {
                "head" => format!("reins/before-reset-{plan}"),
                _ => format!("reins/restored/{}", r.name),
            };
            let name = free_branch(&r.repo, &base);
            git(&["branch", &name, &r.oid])?;
            Ok(format!("Branch {name} holds what was there"))
        }
        other => Err(format!("a backup of an unknown kind ({other})")),
    }
}

/// Puts a trashed worktree back as a worktree: `git worktree add --no-checkout` at its place (its branch when free,
/// else detached at its commit), its files moved back in, and the index read again. Falls back to a plain folder.
pub fn restore_worktree(store: &Store, plan: &str, item: &Item, wt: &WorktreeInfo) -> Result<String, String> {
    let dir = store.root().join(plan).join("items").join(&item.id);
    let data = dir.join("data");
    let (original, _) = store.place_of(item)?;
    let original = original.as_path();
    if std::fs::symlink_metadata(original).is_ok() {
        return Err(format!("{} is there again; move it away first", item.original));
    }
    let repo = Path::new(&wt.repo);
    let added = wt
        .branch
        .as_deref()
        .and_then(|b| read(&[], repo, &["worktree", "add", "--no-checkout", &item.original, b]))
        .or_else(|| read(&[], repo, &["worktree", "add", "--no-checkout", "--detach", &item.original, &wt.head]));
    if added.is_none() {
        return store.restore_item(plan, item).map(|l| format!("{l} (as a folder: git could not take it back)"));
    }
    for e in std::fs::read_dir(&data).map_err(|e| e.to_string())?.flatten() {
        if e.file_name() == ".git" {
            continue;
        }
        std::fs::rename(e.path(), original.join(e.file_name())).map_err(|e| e.to_string())?;
    }
    let _indexed = read(&[], original, &["reset", "-q"]);
    let _done = std::fs::remove_dir_all(&dir);
    Ok(format!("Worktree {} is back", item.original))
}
