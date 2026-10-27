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

fn git_in(globals: &[String], cwd: &Path) -> Command {
    let mut c = Command::new("git");
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
    match Command::new("git").current_dir(cwd).args(args).status() {
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
    let kept = match shape.net {
        GitNet::BranchDelete => branch_delete(store, plan, &shape, cwd),
        GitNet::ResetHard => return reset_hard(store, plan, &shape, &words, cwd, err),
        GitNet::WorktreeRemove => return worktree_remove(store, plan, &shape, &words, cwd, err),
        GitNet::Clean => return clean(store, plan, &shape, cwd, err),
        GitNet::ForcePush => force_push(store, plan, &shape, cwd),
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
    let untracked = read(&top_globals, cwd, &["ls-files", "-z", "--others"]).map(|t| names(&t)).unwrap_or_default();
    let mut items = Vec::new();
    if untracked.is_empty() {
        return Ok(items);
    }
    let tracked = read(&top_globals, cwd, &["ls-tree", "-r", "-z", "--name-only", target])
        .map(|t| names(&t))
        .ok_or("git ls-tree failed")?;
    for f in untracked.iter().filter(|f| tracked.contains(f)) {
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
    let Some(target) = operands(&shape.rest).get(1).map(|t| base(&shape.globals, cwd).join(t)) else {
        return run_as_written(words, cwd);
    };
    let Ok(real) = std::fs::canonicalize(&target) else {
        // Not there: git removes only its record, nothing on disk is lost.
        return run_as_written(words, cwd);
    };
    let list = worktrees(&shape.globals, cwd);
    let found =
        list.iter().enumerate().find(|(_, w)| std::fs::canonicalize(&w.0).ok().as_deref() == Some(real.as_path()));
    let Some((index, (_, head, branch, locked))) = found else {
        // Not a worktree of this repository: git refuses it.
        return run_as_written(words, cwd);
    };
    if index == 0 || (*locked && force < 2) {
        return run_as_written(words, cwd);
    }
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

fn force_push(store: &Store, plan: &str, shape: &GitShape, cwd: &Path) -> Result<(), String> {
    let ops = operands(&shape.rest);
    let Some((remote, specs)) = ops.split_first() else {
        return Err("no remote".to_owned());
    };
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
        let probe = git_in(&shape.globals, cwd)
            .args(["ls-remote", "--exit-code", remote, &full])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map_err(|e| e.to_string())?;
        match probe.code() {
            // No such branch there yet: nothing to lose.
            Some(2) => continue,
            Some(0) => {}
            _ => return Err(format!("couldn't read {dst} on {remote}")),
        }
        let backup = Store::backup_ref(plan);
        read(&shape.globals, cwd, &["fetch", "--no-tags", "--quiet", remote, &format!("+{full}:{backup}")])
            .ok_or_else(|| format!("couldn't fetch {dst} from {remote}"))?;
        let oid = read(&shape.globals, cwd, &["rev-parse", "--verify", &backup]).ok_or("the fetched tip is missing")?;
        let repo = common_dir(&shape.globals, cwd).ok_or("not in a git repository")?;
        store
            .record_ref(
                plan,
                &RefBackup {
                    at: crate::now_unix(),
                    repo,
                    backup,
                    oid,
                    what: "remote".to_owned(),
                    name: format!("{remote}/{dst}"),
                },
            )
            .map_err(|e| e.to_string())?;
    }
    Ok(())
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
    let original = Path::new(&item.original);
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
