//! Git: the exact facts of a push through the desktop proxy (01 §2.2), and the `git` and `gh` program models.

use reins_proto::desktop::{PushSummary, RefChange};

use super::level::Analysis;
use super::model::{Certainty, Channel, CodeSource, Effect, InfraChange, NetMethod};
use super::shell::{Exp, Input, Walk};

/// Branch names taken as the default when the repository's own is not known (`docs/autopilot-risk.md`).
const LONG_LIVED: &[&str] =
    &["main", "master", "trunk", "develop", "development", "production", "prod", "staging", "stable"];

/// Whether `branch` is (or may be) the default branch, given the default the caller knows.
#[must_use]
pub fn is_default(branch: &str, known_default: Option<&str>) -> bool {
    let b = branch.trim_start_matches("refs/heads/");
    known_default.is_some_and(|d| d == b)
        || LONG_LIVED.contains(&b)
        || b.starts_with("release/")
        || b.starts_with("release-")
}

/// A push as the proxy saw it: each ref's old and new tips, fast-forward or not, the host's default branch.
#[must_use]
pub fn analyze_push(repo: &str, summary: &PushSummary) -> Analysis {
    let mut effects = Vec::new();
    let mut certainty = Certainty::Full;
    let mut unresolved = Vec::new();
    let default = summary.default_branch.as_deref();
    for u in &summary.updates {
        let default_branch = if u.name.starts_with("refs/heads/") {
            Some(is_default(&u.name, default))
        } else {
            Some(false)
        };
        match u.change {
            RefChange::Delete => effects.push(Effect::GitRefDelete {
                repo: repo.to_owned(),
                refname: u.name.clone(),
                default_branch,
            }),
            RefChange::Create | RefChange::Update => {
                let rewrites = match (u.change, u.fast_forward) {
                    (RefChange::Create, _) => Some(false),
                    (_, Some(ff)) => Some(!ff),
                    (_, None) => {
                        certainty = Certainty::Partial;
                        unresolved.push(format!("whether the push to {} rewrites history", u.name));
                        None
                    }
                };
                effects.push(Effect::GitPush {
                    repo: repo.to_owned(),
                    refname: u.name.clone(),
                    default_branch,
                    rewrites,
                    commits: u.commit_count,
                });
            }
        }
    }
    if summary.push_options.iter().any(|o| o.starts_with("ci.skip") || o.contains("merge_request")) {
        effects.push(Effect::Message {
            channel: Channel::GitHub,
            to: vec![repo.to_owned()],
            attachments: 0,
        });
    }
    Analysis::new(effects, certainty, unresolved)
}

/// A clone or fetch through the proxy.
#[must_use]
pub fn analyze_git_read(repo: &str) -> Analysis {
    Analysis::new(
        vec![Effect::DataRead {
            service: "git".to_owned(),
            resource: repo.to_owned(),
        }],
        Certainty::Full,
        Vec::new(),
    )
}

/// Configuration keys that make git run a program.
const RUNS: &[&str] = &[
    "core.sshcommand",
    "core.pager",
    "core.editor",
    "core.fsmonitor",
    "core.hookspath",
    "core.gitproxy",
    "core.askpass",
    "sequence.editor",
    "diff.external",
    "credential.helper",
    "uploadpack.packobjectshook",
    "gpg.program",
    "ssh.variant",
    "protocol.ext.allow",
];

fn runs_program(setting: &str) -> bool {
    let s = setting.to_ascii_lowercase();
    let key = s.split_once('=').map_or(s.as_str(), |(k, _)| k);
    RUNS.iter().any(|r| key == *r || key.starts_with(&format!("{r}.")))
        || key.ends_with(".command")
        || (key.starts_with("alias.") && s.contains('!'))
}

fn flag(args: &[Exp], names: &[&str]) -> bool {
    args.iter().any(|a| {
        names.contains(&a.text.as_str())
            || (a.text.starts_with('-')
                && !a.text.starts_with("--")
                && names.iter().any(|n| {
                    n.len() == 2 && n.starts_with('-') && !n.starts_with("--") && a.text[1..].contains(&n[1..])
                }))
    })
}

fn operands(args: &[Exp]) -> Vec<&Exp> {
    let mut out = Vec::new();
    let mut options = true;
    for a in args {
        if options && a.text == "--" {
            options = false;
        } else if options && a.text.starts_with('-') {
        } else {
            out.push(a);
        }
    }
    out
}

/// `git …` run by a shell.
#[allow(clippy::too_many_lines, reason = "one arm per subcommand family")]
pub(super) fn shell_git(w: &mut Walk<'_>, args: &[Exp]) -> Input {
    let mut k = 0;
    let saved_cwd = w.cwd.clone();
    while let Some(a) = args.get(k) {
        match a.text.as_str() {
            "-C" => {
                if let Some(d) = args.get(k + 1) {
                    w.note(d);
                    w.cwd = super::workspace::normalize(&d.text, &w.cwd.clone(), &w.ctx.workspace.home);
                }
                k += 2;
            }
            "-c" => {
                if let Some(s) = args.get(k + 1)
                    && (runs_program(&s.text) || !s.known())
                {
                    w.effect(Effect::Exec {
                        program: "a program set in git's configuration".to_owned(),
                        source: CodeSource::Unknown,
                    });
                    w.lower(Certainty::None, "a program git's configuration runs");
                }
                k += 2;
            }
            "--git-dir" | "--work-tree" | "--namespace" => k += 2,
            // `--config-env name=VAR`: a setting whose value comes from the environment, unseen here.
            t if t == "--config-env" || t.starts_with("--config-env=") => {
                let setting = t
                    .strip_prefix("--config-env=")
                    .map(str::to_owned)
                    .or_else(|| args.get(k + 1).map(|a| a.text.clone()));
                let key = setting.as_deref().unwrap_or_default();
                if runs_program(key)
                    || setting.is_none()
                    || args.get(k + 1).is_some_and(|a| t == "--config-env" && !a.known())
                {
                    w.lower(Certainty::None, "a program git's configuration runs (from the environment)");
                } else {
                    w.lower(Certainty::None, format!("a git setting from the environment ({key})"));
                }
                k += if t == "--config-env" {
                    2
                } else {
                    1
                };
            }
            "--no-pager"
            | "-P"
            | "--paginate"
            | "-p"
            | "--bare"
            | "--no-replace-objects"
            | "--literal-pathspecs"
            | "--glob-pathspecs"
            | "--noglob-pathspecs"
            | "--icase-pathspecs"
            | "--no-optional-locks"
            | "--version"
            | "--help"
            | "--no-lazy-fetch"
            | "--no-advice" => k += 1,
            t if t.starts_with("--git-dir=") || t.starts_with("--work-tree=") || t.starts_with("--namespace=") => {
                k += 1;
            }
            // Any other global option (`--exec-path=…` picks the programs git runs): not read.
            t if t.starts_with('-') => {
                w.lower(Certainty::None, format!("the git option {t}"));
                k += 1;
            }
            _ => break,
        }
    }
    let Some(sub) = args.get(k).map(|a| a.text.clone()) else {
        w.cwd = saved_cwd;
        return Input::None;
    };
    let rest = &args[k + 1..];
    let worktree = w.repo_root().unwrap_or_else(|| w.cwd.clone());
    let wt = Exp::lit(&worktree);
    // Configuration given through the environment on this line: not read.
    for prefix in [
        "GIT_CONFIG_COUNT",
        "GIT_CONFIG_KEY_",
        "GIT_CONFIG_VALUE_",
        "GIT_CONFIG_PARAMETERS",
        "GIT_CONFIG_GLOBAL",
        "GIT_CONFIG_SYSTEM",
        "GIT_EXEC_PATH",
        "GIT_CONFIG",
    ] {
        if w.has_var_prefix(prefix) {
            w.lower(Certainty::None, format!("git configuration from the environment ({prefix})"));
        }
    }
    // Programs git runs from its environment (`GIT_SEQUENCE_EDITOR=… git rebase -i`): read as commands.
    for var in [
        "GIT_EDITOR",
        "GIT_SEQUENCE_EDITOR",
        "GIT_SSH_COMMAND",
        "GIT_SSH",
        "GIT_EXTERNAL_DIFF",
        "GIT_PAGER",
        "GIT_ASKPASS",
        "GIT_PROXY_COMMAND",
    ] {
        match w.var_state(var) {
            Some(Some(cmd)) if !matches!(cmd.as_str(), "" | "true" | ":" | "cat" | "less" | "ssh") => {
                w.run_script(&cmd, Vec::new());
            }
            Some(None) => w.lower(Certainty::None, format!("the program ${var} names")),
            _ => {}
        }
    }
    // Commands git itself runs: read as commands.
    let run_value = |w: &mut Walk<'_>, names: &[&str]| {
        let mut k = 0;
        while k < rest.len() {
            let t = rest[k].text.as_str();
            let value = names.iter().find_map(|n| {
                if t == *n {
                    rest.get(k + 1).cloned()
                } else {
                    t.strip_prefix(&format!("{n}=")).map(|v| Exp {
                        text: v.to_owned(),
                        ..rest[k].clone()
                    })
                }
            });
            if let Some(v) = value {
                if v.known() {
                    w.run_script(&v.text, Vec::new());
                } else {
                    w.lower(Certainty::None, format!("a command git runs ({t})"));
                }
            }
            k += 1;
        }
    };
    match sub.as_str() {
        // `git bisect run cmd args…`: the command's own words, as they are.
        "bisect" => {
            if rest.first().is_some_and(|a| a.text == "run") {
                git_command(w, &rest[1..], "git bisect run");
            }
        }
        // `git submodule [--quiet] foreach [--recursive] cmd…`: one word is a shell script, more are a command.
        "submodule" if rest.iter().any(|a| a.text == "foreach") => {
            let mut at = rest.iter().position(|a| a.text == "foreach").map_or(rest.len(), |p| p + 1);
            while rest.get(at).is_some_and(|a| matches!(a.text.as_str(), "--recursive" | "--quiet" | "-q")) {
                at += 1;
            }
            git_command(w, &rest[at..], "git submodule foreach");
        }
        "rebase" => {
            rebase(w, rest);
            local_change(w, &sub, rest, &worktree);
        }
        "filter-branch" => {
            run_value(
                w,
                &[
                    "--tree-filter",
                    "--index-filter",
                    "--msg-filter",
                    "--env-filter",
                    "--commit-filter",
                    "--parent-filter",
                    "--tag-name-filter",
                    "--setup",
                ],
            );
            local_change(w, &sub, rest, &worktree);
        }
        "filter-repo" if rest.iter().any(|a| a.text.ends_with("-callback") || a.text.contains("-callback=")) => {
            w.lower(Certainty::None, "Python code git filter-repo runs");
        }
        "status" | "log" | "diff" | "show" | "rev-parse" | "ls-files" | "ls-tree" | "cat-file" | "describe"
        | "blame" | "grep" | "shortlog" | "merge-base" | "rev-list" | "for-each-ref" | "name-rev" | "symbolic-ref"
        | "show-ref" | "whatchanged" | "range-diff" | "cherry" | "check-ignore" | "check-attr" | "count-objects"
        | "fsck" | "verify-commit" | "verify-tag" | "help" | "version" | "var" | "diff-tree" | "diff-index"
        | "diff-files" | "annotate" | "format-patch" | "bundle" | "archive" | "notes" | "ls-remote"
        | "request-pull" | "instaweb" => {
            if sub == "ls-remote" {
                w.effect(Effect::Network {
                    host: "a git remote".to_owned(),
                    method: NetMethod::Read,
                });
            }
            if (sub == "format-patch" || sub == "archive" || sub == "bundle")
                && let Some(o) = rest
                    .iter()
                    .position(|a| a.text == "-o" || a.text == "--output" || a.text == "--output-directory")
                    .and_then(|p| rest.get(p + 1))
            {
                let p = w.path(o);
                w.write(p);
            }
        }
        "fetch" | "pull" | "clone" | "remote" | "submodule" | "lfs" | "maintenance" | "gc" | "prune" | "repack"
        | "pack-refs" | "fetch-pack" => {
            if matches!(sub.as_str(), "fetch" | "pull" | "clone")
                || (sub == "submodule" && rest.iter().any(|a| a.text == "update"))
            {
                w.effect(Effect::Network {
                    host: rest
                        .iter()
                        .find_map(|a| super::programs::url_host(&a.text))
                        .unwrap_or_else(|| "a git remote".to_owned()),
                    method: NetMethod::Read,
                });
            }
            if sub == "clone" {
                let ops = operands(rest);
                let dir = ops.get(1).map(|d| (*d).clone()).or_else(|| {
                    ops.first().map(|u| {
                        Exp::lit(
                            u.text
                                .trim_end_matches('/')
                                .trim_end_matches(".git")
                                .rsplit(['/', ':'])
                                .next()
                                .unwrap_or("repo"),
                        )
                    })
                });
                if let Some(d) = dir {
                    let p = w.path(&d);
                    w.write(p);
                }
            }
            if sub == "pull" {
                let p = w.path(&wt);
                w.write(p);
            }
            if sub == "gc" && rest.iter().any(|a| a.text.starts_with("--prune")) || sub == "prune" {
                // Unreachable objects go: what a reflog could have restored.
                let p = w.path(&Exp::lit(&format!("{worktree}/.git")));
                w.effect(Effect::FileDelete {
                    paths: vec![p],
                    recursive: true,
                });
            }
        }
        "add" | "commit" | "stash" | "tag" | "checkout" | "switch" | "branch" | "merge" | "cherry-pick" | "revert"
        | "am" | "apply" | "mv" | "rm" | "restore" | "reset" | "clean" | "worktree" | "config" | "update-ref"
        | "reflog" | "replace" | "filter-repo" | "sparse-checkout" | "init" | "update-index" | "commit-tree"
        | "hash-object" | "read-tree" | "write-tree" | "mktag" | "mktree" | "rerere" => {
            local_change(w, &sub, rest, &worktree);
        }
        "push" | "send-pack" => push(w, rest),
        // Anything else (an alias, `difftool`, `send-email`, a third-party `git-foo`): not read.
        _ => {
            w.effect(Effect::Exec {
                program: format!("git {sub}"),
                source: CodeSource::Unknown,
            });
            w.lower(Certainty::None, format!("what `git {sub}` does"));
        }
    }
    w.cwd = saved_cwd;
    Input::Opaque("git".to_owned())
}

/// A command git runs from words (`bisect run`, `submodule foreach`), the way git runs it: when the first word holds
/// a shell character or a space, git runs `sh -c '<first> "$@"'` with the other words as arguments; otherwise the words
/// are the program and its arguments.
fn git_command(w: &mut Walk<'_>, words: &[Exp], what: &str) {
    const SHELL: &[char] = &[
        '|', '&', ';', '<', '>', '(', ')', '$', '`', '\\', '"', '\'', ' ', '\t', '\n', '*', '?', '[', '#', '~', '=',
        '%',
    ];
    let Some((first, rest)) = words.split_first() else {
        w.lower(Certainty::None, format!("{what} without a command"));
        return;
    };
    if !first.known() {
        w.note(first);
        w.lower(Certainty::None, format!("the command {what} runs, with values that are not in the command"));
        return;
    }
    if first.text.contains(SHELL) {
        if rest.iter().any(|a| !a.known()) {
            w.lower(Certainty::None, format!("the arguments {what} passes, with values that are not in the command"));
        }
        // Git appends `"$@"`: the other words are the script's arguments, and its last command's too.
        w.run_script(&format!("{} \"$@\"", first.text), rest.to_vec());
    } else {
        let _out: Input = super::programs::run(w, words, Input::None, false);
    }
}

/// `git rebase`'s options. `-x`/`--exec` in every form (`-x cmd`, `-xcmd`, `-ix cmd`, `--exec=cmd`, `--exe cmd`)
/// runs a command, read as one; any option not on this list is not read.
#[allow(clippy::too_many_lines, reason = "one case per option form")]
#[allow(clippy::many_single_char_names, reason = "w(alk), k, a, t, c, o, v as in the other option readers")]
fn rebase(w: &mut Walk<'_>, rest: &[Exp]) {
    const LONG: &[&str] = &[
        "interactive",
        "onto",
        "root",
        "continue",
        "abort",
        "skip",
        "quit",
        "edit-todo",
        "show-current-patch",
        "autosquash",
        "no-autosquash",
        "autostash",
        "no-autostash",
        "quiet",
        "verbose",
        "stat",
        "no-stat",
        "keep-base",
        "update-refs",
        "no-update-refs",
        "committer-date-is-author-date",
        "ignore-date",
        "reset-author-date",
        "signoff",
        "no-verify",
        "verify",
        "force-rebase",
        "no-ff",
        "fork-point",
        "no-fork-point",
        "merge",
        "strategy",
        "strategy-option",
        "empty",
        "rebase-merges",
        "no-rebase-merges",
        "apply",
        "whitespace",
        "reapply-cherry-picks",
        "no-reapply-cherry-picks",
        "allow-empty-message",
        "keep-empty",
        "no-keep-empty",
        "exec",
        "gpg-sign",
        "no-gpg-sign",
        "trailer",
        "context",
    ];
    // Options that take the next word when no `=value` is attached.
    const LONG_VALUE: &[&str] = &["onto", "strategy", "strategy-option", "exec", "trailer"];
    let exec = |w: &mut Walk<'_>, v: Option<Exp>| match v {
        Some(v) if v.known() => w.run_script(&v.text, Vec::new()),
        Some(_) => {
            w.lower(Certainty::None, "a command git rebase --exec runs, with values that are not in the command");
        }
        None => w.lower(Certainty::None, "git rebase --exec without a command"),
    };
    let mut k = 0;
    while k < rest.len() {
        let a = &rest[k];
        let t = a.text.as_str();
        k += 1;
        if t == "--" {
            break;
        }
        if let Some(long) = t.strip_prefix("--") {
            let (name, value) = long.split_once('=').map_or((long, None), |(n, v)| (n, Some(v)));
            // Git takes any unambiguous prefix of a long option.
            let matches: Vec<&str> = LONG.iter().copied().filter(|o| *o == name || o.starts_with(name)).collect();
            let option = if matches.contains(&name) {
                Some(name)
            } else if matches.len() == 1 {
                Some(matches[0])
            } else {
                None
            };
            match option {
                Some(o) => {
                    let v = value
                        .map(|v| Exp {
                            text: v.to_owned(),
                            ..a.clone()
                        })
                        .or_else(|| {
                            LONG_VALUE.contains(&o).then(|| {
                                k += 1;
                                rest.get(k - 1).cloned()
                            })?
                        });
                    if o == "exec" {
                        exec(w, v);
                    } else if o == "strategy"
                        && v.as_ref().is_none_or(|v| {
                            !matches!(v.text.as_str(), "ort" | "recursive" | "resolve" | "octopus" | "ours" | "subtree")
                        })
                    {
                        // A custom strategy is a program (`git-merge-<name>`).
                        w.lower(Certainty::None, "a merge strategy program git rebase runs");
                    }
                }
                None => w.lower(Certainty::None, format!("the git rebase option --{name}")),
            }
            continue;
        }
        let Some(short) = t.strip_prefix('-').filter(|s| !s.is_empty()) else {
            continue;
        };
        for (i, c) in short.char_indices() {
            let attached = &short[i + c.len_utf8()..];
            match c {
                'i' | 'q' | 'v' | 'n' | 'f' | 'm' | 'r' | 'k' => {}
                'x' => {
                    let v = if attached.is_empty() {
                        k += 1;
                        rest.get(k - 1).cloned()
                    } else {
                        Some(Exp {
                            text: attached.to_owned(),
                            ..a.clone()
                        })
                    };
                    exec(w, v);
                    break;
                }
                's' | 'X' => {
                    if attached.is_empty() {
                        k += 1;
                    }
                    if c == 's' {
                        let v = if attached.is_empty() {
                            rest.get(k - 1).map(|v| v.text.as_str())
                        } else {
                            Some(attached)
                        };
                        if v.is_none_or(|v| {
                            !matches!(v, "ort" | "recursive" | "resolve" | "octopus" | "ours" | "subtree")
                        }) {
                            w.lower(Certainty::None, "a merge strategy program git rebase runs");
                        }
                    }
                    break;
                }
                // `-C<n>`, `-S[<key>]`: the rest is its value.
                'C' | 'S' => break,
                other => {
                    w.lower(Certainty::None, format!("the git rebase option -{other}"));
                    break;
                }
            }
        }
    }
}

/// Local repository changes: most are recoverable from the reflog; the ones that discard work in the worktree are
/// deletes there.
#[allow(clippy::too_many_lines, reason = "one arm per subcommand")]
fn local_change(w: &mut Walk<'_>, sub: &str, rest: &[Exp], worktree: &str) {
    let wt = Exp::lit(worktree);
    let discard = |w: &mut Walk<'_>, paths: Vec<Exp>| {
        let refs: Vec<_> = paths.iter().map(|p| w.path(p)).collect();
        w.effect(Effect::FileDelete {
            paths: refs,
            recursive: true,
        });
    };
    let ops = operands(rest);
    match sub {
        "reset" => {
            if rest.iter().any(|a| a.text == "--hard" || a.text == "--merge" || a.text == "--keep") {
                discard(w, vec![wt]);
            }
        }
        "clean" => {
            if flag(rest, &["-f", "--force"]) && !flag(rest, &["-n", "--dry-run"]) {
                let paths = if ops.is_empty() {
                    vec![wt]
                } else {
                    ops.into_iter().cloned().collect()
                };
                discard(w, paths);
            }
        }
        "checkout" | "restore" => {
            let after_dashdash = rest.iter().position(|a| a.text == "--").map(|p| rest[p + 1..].to_vec());
            if flag(rest, &["-f", "--force"])
                || (sub == "restore"
                    && !rest.iter().any(|a| a.text == "--staged" && !rest.iter().any(|b| b.text == "--worktree")))
            {
                let paths = after_dashdash.unwrap_or_else(|| ops.iter().map(|o| (*o).clone()).collect());
                let paths = if paths.is_empty() {
                    vec![wt]
                } else {
                    paths
                };
                discard(w, paths);
            } else if let Some(paths) = after_dashdash.filter(|p| !p.is_empty()) {
                discard(w, paths);
            } else if ops.iter().any(|o| o.text == ".") {
                discard(w, vec![wt]);
            }
        }
        "stash" => {
            if ops.first().is_some_and(|o| matches!(o.text.as_str(), "drop" | "clear")) {
                w.effect(Effect::GitRefDelete {
                    repo: String::new(),
                    refname: "refs/stash".to_owned(),
                    default_branch: Some(false),
                });
            }
        }
        "branch" => {
            let delete = flag(rest, &["-D", "-d", "--delete"]);
            if delete {
                for b in ops {
                    let name = b.text.clone();
                    w.note(b);
                    let default = is_default(&name, w.ctx.default_branch.as_deref());
                    w.effect(Effect::GitRefDelete {
                        repo: String::new(),
                        refname: format!("refs/heads/{name}"),
                        default_branch: Some(default),
                    });
                }
            } else if flag(rest, &["-m", "-M", "--move"]) {
                if let Some(old) = (ops.len() >= 2).then(|| ops[0]) {
                    w.effect(Effect::GitRefDelete {
                        repo: String::new(),
                        refname: format!("refs/heads/{}", old.text),
                        default_branch: Some(is_default(&old.text, w.ctx.default_branch.as_deref())),
                    });
                }
            } else if flag(rest, &["-f", "--force"])
                && let Some(b) = ops.first()
            {
                // Moves an existing branch: its old tip is in the reflog.
                let default = is_default(&b.text, w.ctx.default_branch.as_deref());
                w.effect(Effect::GitRefDelete {
                    repo: String::new(),
                    refname: format!("refs/heads/{}", b.text),
                    default_branch: Some(default),
                });
            }
        }
        "tag" => {
            if flag(rest, &["-d", "--delete"]) {
                for t in ops {
                    w.effect(Effect::GitRefDelete {
                        repo: String::new(),
                        refname: format!("refs/tags/{}", t.text),
                        default_branch: Some(false),
                    });
                }
            }
        }
        "worktree" => {
            if ops.first().is_some_and(|o| o.text == "remove") {
                for t in ops.iter().skip(1) {
                    let p = w.path(t);
                    // Its commits stay in the repository, but `--force` drops uncommitted work too, and a path that is
                    // not a known worktree may not be one at all: recoverable only without force, in a known
                    // worktree. Its area is where it really is.
                    let force = rest.iter().any(|a| a.text == "--force" || a.text == "-f" || a.text == "-ff");
                    w.effect(Effect::FileDelete {
                        paths: vec![super::model::PathRef {
                            recoverable: !force && p.recoverable,
                            ..p
                        }],
                        recursive: true,
                    });
                }
            } else if ops.first().is_some_and(|o| o.text == "add")
                && let Some(dir) = ops.get(1)
            {
                let p = w.path(dir);
                w.write(p);
            }
        }
        "rm" => {
            if !rest.iter().any(|a| a.text == "--cached") {
                let paths: Vec<Exp> = ops.into_iter().cloned().collect();
                if !paths.is_empty() {
                    discard(w, paths);
                }
            }
        }
        "config" => {
            let setting: Vec<&str> = ops.iter().map(|o| o.text.as_str()).collect();
            if let Some(key) = setting.first()
                && setting.len() >= 2
                && runs_program(key)
            {
                w.effect(Effect::Exec {
                    program: format!("a program set as {key}"),
                    source: CodeSource::Unknown,
                });
                w.lower(Certainty::None, format!("the program `{key}` runs"));
            }
        }
        "update-ref" => {
            if flag(rest, &["-d"])
                && let Some(r) = ops.first()
            {
                w.effect(Effect::GitRefDelete {
                    repo: String::new(),
                    refname: r.text.clone(),
                    default_branch: Some(is_default(&r.text, w.ctx.default_branch.as_deref())),
                });
            }
        }
        "reflog" if ops.first().is_some_and(|o| matches!(o.text.as_str(), "expire" | "delete")) => {
            let p = w.path(&Exp::lit(&format!("{worktree}/.git")));
            w.effect(Effect::FileDelete {
                paths: vec![p],
                recursive: true,
            });
        }
        _ => {}
    }
}

/// `git push`: the refs it names, whether it forces, and whether a target is the default branch. The proxy sees the
/// exact refs when the push goes through Reins; this is the shell's reading of the same request.
fn push(w: &mut Walk<'_>, rest: &[Exp]) {
    let force = rest.iter().any(|a| {
        a.text == "-f"
            || a.text.starts_with("--force")
            || (a.text.starts_with('-') && !a.text.starts_with("--") && a.text.contains('f'))
    });
    let delete = rest.iter().any(|a| a.text == "-d" || a.text == "--delete");
    let mirror = rest.iter().any(|a| matches!(a.text.as_str(), "--mirror" | "--prune" | "--all" | "--branches"));
    let tags = rest.iter().any(|a| a.text == "--tags" || a.text == "--follow-tags");
    let value_opts = ["--repo", "-o", "--push-option", "--receive-pack", "--exec", "--signed"];
    let mut ops: Vec<&Exp> = Vec::new();
    let mut k = 0;
    while k < rest.len() {
        let t = rest[k].text.as_str();
        if value_opts.contains(&t) {
            k += 2;
            continue;
        }
        if !t.starts_with('-') || t == "-" {
            ops.push(&rest[k]);
        }
        k += 1;
    }
    let remote = ops.first().map_or_else(|| "origin".to_owned(), |r| r.text.clone());
    let repo = super::programs::url_host(&remote).map_or(remote.clone(), |_| remote.clone());
    let current = w.ctx.branch.clone();
    let default = w.ctx.default_branch.clone();
    let mut refspecs: Vec<Exp> = ops.iter().skip(1).map(|e| (*e).clone()).collect();
    if mirror {
        w.effect(Effect::GitPush {
            repo: repo.clone(),
            refname: "every branch".to_owned(),
            default_branch: Some(true),
            rewrites: Some(force || rest.iter().any(|a| a.text == "--mirror" || a.text == "--prune")),
            commits: 0,
        });
        if rest.iter().any(|a| a.text == "--mirror" || a.text == "--prune") {
            w.effect(Effect::GitRefDelete {
                repo,
                refname: "branches missing here".to_owned(),
                default_branch: None,
            });
        }
        return;
    }
    if refspecs.is_empty() {
        if tags {
            w.effect(Effect::GitPush {
                repo,
                refname: "refs/tags/*".to_owned(),
                default_branch: Some(false),
                rewrites: Some(force),
                commits: 0,
            });
            return;
        }
        refspecs.push(Exp::lit("HEAD"));
    }
    for spec in refspecs {
        w.note(&spec);
        let text = spec.text.clone();
        let plus = text.starts_with('+');
        let text = text.trim_start_matches('+');
        let (src, dst) = match text.split_once(':') {
            Some((s, d)) => (s.to_owned(), d.to_owned()),
            None => (text.to_owned(), text.to_owned()),
        };
        let resolve = |name: &str| -> Option<String> {
            if name == "HEAD" || name == "@" {
                current.clone()
            } else {
                Some(name.to_owned())
            }
        };
        let dst_name = if dst.is_empty() {
            None
        } else {
            resolve(&dst)
        };
        let short = dst_name.as_deref().map(|d| d.trim_start_matches("refs/heads/").to_owned());
        let is_tag = dst.starts_with("refs/tags/")
            || (!dst.contains('/') && dst.starts_with('v') && dst[1..].starts_with(|c: char| c.is_ascii_digit()));
        let default_branch = if is_tag {
            Some(false)
        } else {
            short.as_deref().map(|d| is_default(d, default.as_deref()))
        };
        if delete || src.is_empty() {
            w.effect(Effect::GitRefDelete {
                repo: repo.clone(),
                refname: dst_name.map_or_else(
                    || "a branch".to_owned(),
                    |d| {
                        if d.starts_with("refs/") {
                            d
                        } else {
                            format!("refs/heads/{d}")
                        }
                    },
                ),
                default_branch: default_branch.or(Some(false)),
            });
            continue;
        }
        let refname = match &short {
            Some(s) if is_tag => format!("refs/tags/{}", s.trim_start_matches("refs/tags/")),
            Some(s) => format!("refs/heads/{s}"),
            None => "the current branch".to_owned(),
        };
        w.effect(Effect::GitPush {
            repo: repo.clone(),
            refname,
            default_branch,
            rewrites: Some(force || plus),
            commits: 0,
        });
    }
}

/// `gh …`.
#[allow(clippy::too_many_lines, reason = "one arm per command family")]
pub(super) fn shell_gh(w: &mut Walk<'_>, args: &[Exp]) -> Input {
    // `gh -R owner/name pr …`: the global option before the command.
    let mut words: Vec<&str> = Vec::new();
    let mut k = 0;
    while k < args.len() {
        if matches!(args[k].text.as_str(), "-R" | "--repo") && words.is_empty() {
            k += 2;
            continue;
        }
        words.push(args[k].text.as_str());
        k += 1;
    }
    let repo = args
        .iter()
        .position(|a| a.text == "-R" || a.text == "--repo")
        .and_then(|p| args.get(p + 1))
        .map_or_else(|| "this repository".to_owned(), |r| r.text.clone());
    let read = |w: &mut Walk<'_>, what: &str| {
        w.effect(Effect::DataRead {
            service: "GitHub".to_owned(),
            resource: what.to_owned(),
        });
    };
    let message = |w: &mut Walk<'_>| {
        w.effect(Effect::Message {
            channel: Channel::GitHub,
            to: vec![repo.clone()],
            attachments: 0,
        });
    };
    match words.as_slice() {
        [
            "pr" | "issue",
            "create" | "comment" | "review" | "edit" | "close" | "reopen" | "ready" | "lock" | "unlock" | "transfer"
            | "pin" | "develop",
            ..,
        ] => message(w),
        ["pr", "merge", ..] => {
            let base =
                args.iter().position(|a| a.text == "--base").and_then(|p| args.get(p + 1)).map(|b| b.text.clone());
            w.effect(Effect::GitPush {
                repo: repo.clone(),
                refname: base
                    .clone()
                    .map_or_else(|| "the pull request's base".to_owned(), |b| format!("refs/heads/{b}")),
                default_branch: Some(base.as_deref().is_none_or(|b| is_default(b, None))),
                rewrites: Some(false),
                commits: 0,
            });
            message(w);
            if words.contains(&"--delete-branch") || words.contains(&"-d") {
                w.effect(Effect::GitRefDelete {
                    repo: repo.clone(),
                    refname: "the pull request's branch".to_owned(),
                    default_branch: Some(false),
                });
            }
        }
        [
            "pr" | "issue" | "run" | "repo" | "release" | "workflow" | "search" | "label" | "gist" | "cache"
            | "ruleset" | "status" | "browse" | "org" | "project" | "variable" | "codespace",
            "list" | "view" | "diff" | "checks" | "status" | "watch" | "download" | "clone" | "browse",
            ..,
        ]
        | ["status" | "search" | "browse" | "--version" | "version", ..] => {
            if words.get(1) == Some(&"clone") {
                let p = w.path(&Exp::lit("."));
                w.write(p);
            }
            read(w, words.first().copied().unwrap_or("data"));
        }
        ["auth", "token", ..] => {
            w.effect(Effect::SecretOut {
                secret: "the GitHub token".to_owned(),
                destination: super::level::AGENT.to_owned(),
            });
        }
        ["auth", "status" | "setup-git", ..] => read(w, "sign-in status"),
        ["auth", ..] => w.effect(Effect::Permission {
            resource: "GitHub sign-in".to_owned(),
            change: words.get(1).copied().unwrap_or("auth").to_owned(),
        }),
        ["release", "create" | "upload" | "edit", ..] => w.effect(Effect::Publish {
            target: format!("a release of {repo}"),
        }),
        ["release" | "repo", "delete", ..] | ["repo", "archive" | "rename" | "transfer", ..] => {
            w.effect(Effect::Infra {
                target: format!("{} of {repo}", words.first().copied().unwrap_or("something")),
                change: InfraChange::Destroy,
                local: false,
            });
        }
        ["repo", "create" | "fork", ..] => {
            if words.contains(&"--public") {
                w.effect(Effect::Publish {
                    target: "a public repository".to_owned(),
                });
            } else {
                w.effect(Effect::DataWrite {
                    service: "GitHub".to_owned(),
                    what: "Creates a repository".to_owned(),
                    lost: false,
                });
            }
        }
        ["repo", "edit", ..] => {
            if words.iter().any(|w| w.contains("visibility")) {
                w.effect(Effect::Publish {
                    target: format!("{repo} (visibility)"),
                });
            } else {
                w.effect(Effect::DataWrite {
                    service: "GitHub".to_owned(),
                    what: format!("Changes the settings of {repo}"),
                    lost: true,
                });
            }
        }
        ["secret" | "ssh-key" | "gpg-key" | "ruleset", "set" | "add" | "delete" | "remove" | "create", ..]
        | ["repo", "deploy-key", ..] => w.effect(Effect::Permission {
            resource: repo.clone(),
            change: format!("{} {}", words[0], words[1]),
        }),
        ["variable", "set" | "delete", ..] => w.effect(Effect::DataWrite {
            service: "GitHub".to_owned(),
            what: format!("Changes CI variables of {repo}"),
            lost: true,
        }),
        ["workflow", "run" | "enable" | "disable", ..] | ["run", "rerun", ..] => w.effect(Effect::Exec {
            program: format!("a workflow on {repo}"),
            source: CodeSource::Ci,
        }),
        ["run", "cancel" | "delete", ..] | ["cache", "delete", ..] | ["label", "create" | "edit" | "delete", ..] => w
            .effect(Effect::DataWrite {
                service: "GitHub".to_owned(),
                what: format!("{} {} on {repo}", words[0], words[1]),
                lost: words[1] != "create",
            }),
        ["gist", "create", ..] => {
            if words.contains(&"--public") || words.contains(&"-p") {
                w.effect(Effect::Publish {
                    target: "a public gist".to_owned(),
                });
            } else {
                w.effect(Effect::DataWrite {
                    service: "GitHub".to_owned(),
                    what: "Creates a secret gist".to_owned(),
                    lost: false,
                });
            }
        }
        ["api", ..] => {
            let method = args
                .iter()
                .position(|a| a.text == "-X" || a.text == "--method")
                .and_then(|p| args.get(p + 1))
                .map(|m| m.text.to_ascii_uppercase());
            let fields =
                args.iter().any(|a| matches!(a.text.as_str(), "-f" | "-F" | "--field" | "--raw-field" | "--input"));
            let method = method.unwrap_or_else(|| {
                if fields {
                    "POST".to_owned()
                } else {
                    "GET".to_owned()
                }
            });
            // The endpoint: the first word that is neither an option nor an option's value.
            let takes_value = [
                "-X",
                "--method",
                "-f",
                "-F",
                "--field",
                "--raw-field",
                "-H",
                "--header",
                "--input",
                "-q",
                "--jq",
                "-t",
                "--template",
                "--hostname",
                "--cache",
                "-p",
                "--preview",
            ];
            let mut path = String::new();
            let mut k = 1;
            while k < args.len() {
                let t = args[k].text.as_str();
                if takes_value.contains(&t) {
                    k += 2;
                    continue;
                }
                if !t.starts_with('-') {
                    t.clone_into(&mut path);
                    break;
                }
                k += 1;
            }
            let a = super::connectors::github_api(&method, &path);
            for e in a.effects {
                w.effect(e);
            }
            for u in a.unresolved {
                w.lower(a.certainty, u);
            }
        }
        [] => {}
        _ => {
            w.effect(Effect::Exec {
                program: format!("gh {}", words[0]),
                source: CodeSource::Unknown,
            });
            w.lower(
                Certainty::None,
                format!("what `gh {}` does", words.iter().take(2).copied().collect::<Vec<_>>().join(" ")),
            );
        }
    }
    Input::Opaque("gh".to_owned())
}

#[cfg(test)]
mod tests {
    use reins_proto::desktop::{RefUpdate, ZERO_OID};

    use super::*;
    use crate::effects::Level;

    fn update(name: &str, change: RefChange, ff: Option<bool>) -> RefUpdate {
        RefUpdate {
            name: name.to_owned(),
            change,
            old: "a".repeat(40),
            new: if change == RefChange::Delete {
                ZERO_OID.to_owned()
            } else {
                "b".repeat(40)
            },
            fast_forward: ff,
            commit_count: 2,
            commits: Vec::new(),
            files_changed: 1,
            files: Vec::new(),
            additions: None,
            deletions: None,
        }
    }

    fn summary(updates: Vec<RefUpdate>, default: Option<&str>) -> PushSummary {
        PushSummary {
            updates,
            default_branch: default.map(str::to_owned),
            ..PushSummary::default()
        }
    }

    #[test]
    fn push_facts_map_to_levels() {
        let level = |u: RefUpdate, d: Option<&str>| analyze_push("github.com/dkat/reins", &summary(vec![u], d)).level;
        assert_eq!(level(update("refs/heads/feat/x", RefChange::Update, Some(true)), Some("main")), Level::L0);
        assert_eq!(level(update("refs/heads/feat/x", RefChange::Create, None), Some("main")), Level::L0);
        assert_eq!(level(update("refs/heads/feat/x", RefChange::Update, Some(false)), Some("main")), Level::L1);
        assert_eq!(level(update("refs/heads/main", RefChange::Update, Some(true)), Some("main")), Level::L2);
        assert_eq!(level(update("refs/heads/main", RefChange::Update, Some(false)), Some("main")), Level::L3);
        assert_eq!(
            level(update("refs/heads/trunk", RefChange::Update, Some(false)), None),
            Level::L3,
            "a long-lived name is the default when not known"
        );
        assert_eq!(level(update("refs/heads/feat/x", RefChange::Delete, None), Some("main")), Level::L1);
        assert_eq!(level(update("refs/heads/main", RefChange::Delete, None), Some("main")), Level::L3);
        assert_eq!(level(update("refs/tags/v1.0.0", RefChange::Create, None), Some("main")), Level::L2);
        let unknown =
            analyze_push("r", &summary(vec![update("refs/heads/feat/x", RefChange::Update, None)], Some("main")));
        assert_eq!((unknown.level, unknown.certainty), (Level::L1, Certainty::Partial));
        assert_eq!(analyze_git_read("github.com/dkat/reins").level, Level::L0);
    }
}
