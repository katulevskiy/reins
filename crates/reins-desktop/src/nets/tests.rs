use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;

use super::*;

struct Env {
    _dir: tempfile::TempDir,
    store: Store,
    work: PathBuf,
}

fn env() -> Env {
    let dir = tempfile::tempdir().unwrap();
    let work = dir.path().join("work");
    std::fs::create_dir_all(&work).unwrap();
    let store = Store::at(&dir.path().join("nets"));
    store
        .begin(&PlanRecord {
            id: "plan01".to_owned(),
            at: crate::now_unix(),
            key: "abcd".to_owned(),
            ..PlanRecord::default()
        })
        .unwrap();
    Env {
        work: std::fs::canonicalize(&work).unwrap(),
        store,
        _dir: dir,
    }
}

fn args(s: &str) -> Vec<OsString> {
    s.split_whitespace().map(OsString::from).collect()
}

fn no_scratch(_: &Path) -> bool {
    false
}

fn rm_in(e: &Env, line: &str) -> (u8, String, String) {
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let code = rm::rm(&e.store, "plan01", &args(line), &e.work, &rm::Scratch(&no_scratch), &mut out, &mut err);
    (code, String::from_utf8(out).unwrap(), String::from_utf8(err).unwrap())
}

#[test]
fn rm_moves_into_the_trash_and_restore_brings_it_back() {
    let e = env();
    std::fs::create_dir_all(e.work.join("src/old")).unwrap();
    std::fs::write(e.work.join("src/old/a.rs"), "fn a() {}").unwrap();
    std::fs::write(e.work.join("notes.md"), "n").unwrap();
    let (code, out, err) = rm_in(&e, "-rfv src/old notes.md");
    assert_eq!((code, err.as_str()), (0, ""));
    assert_eq!(out, "removed directory 'src/old'\nremoved 'notes.md'\n");
    assert!(!e.work.join("src/old").exists());
    let lines = restore(&e.store, "plan01").unwrap();
    assert_eq!(lines.len(), 2, "{lines:?}");
    assert_eq!(std::fs::read_to_string(e.work.join("src/old/a.rs")).unwrap(), "fn a() {}");
    assert!(e.work.join("notes.md").exists());
    assert_eq!(restore(&e.store, "plan01").unwrap(), vec!["Already restored".to_owned()], "again: nothing changes");
}

#[test]
fn rm_follows_gnu_rm() {
    let e = env();
    assert_eq!(rm_in(&e, "missing").0, 1);
    assert!(rm_in(&e, "missing").2.contains("No such file or directory"));
    assert_eq!(rm_in(&e, "-f missing").0, 0);
    assert_eq!(rm_in(&e, "-f").0, 0);
    assert_eq!(rm_in(&e, "").2, "rm: missing operand\n");
    std::fs::create_dir(e.work.join("d")).unwrap();
    assert!(rm_in(&e, "d").2.contains("Is a directory"));
    assert_eq!(rm_in(&e, "-d d").0, 0, "an empty folder with -d");
    std::fs::create_dir_all(e.work.join("d2/x")).unwrap();
    assert!(rm_in(&e, "-d d2").2.contains("Directory not empty"));
    assert!(rm_in(&e, "-rf .").2.contains("refusing to remove"));
    assert!(rm_in(&e, "-rf d2/..").2.contains("refusing to remove"));
    assert!(rm_in(&e, "-rf /").2.contains("dangerous"));
    // One failure does not stop the others.
    std::fs::write(e.work.join("ok"), "x").unwrap();
    let (code, _, _) = rm_in(&e, "nope ok");
    assert_eq!(code, 1);
    assert!(!e.work.join("ok").exists());
    // An empty name: not there (fine with -f); a trailing slash on a file: not a folder.
    assert_eq!(rm_in(&e, "-f ''").0, 0);
    std::fs::write(e.work.join("plain"), "x").unwrap();
    let (code, _, err) = {
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let code = rm::rm(
            &e.store,
            "plan01",
            &[OsString::from("-f"), OsString::from("")],
            &e.work,
            &rm::Scratch(&no_scratch),
            &mut out,
            &mut err,
        );
        (code, out, String::from_utf8(err).unwrap())
    };
    assert_eq!((code, err.as_str()), (0, ""));
    let (code, _, err) = rm_in(&e, "plain/");
    assert_eq!(code, 1);
    assert!(err.contains("Not a directory"), "{err}");
    assert!(e.work.join("plain").exists());
    // Options after operands, as GNU rm reads them.
    std::fs::create_dir_all(e.work.join("late/x")).unwrap();
    assert_eq!(rm_in(&e, "late -r").0, 0);
    assert!(!e.work.join("late").exists());
}

#[test]
fn unknown_options_delete_nothing_and_are_remembered() {
    let e = env();
    std::fs::write(e.work.join("f"), "x").unwrap();
    let (code, _, err) = rm_in(&e, "-i f");
    assert_eq!(code, 1);
    assert!(err.contains("can't be netted"), "{err}");
    assert!(e.work.join("f").exists());
    assert!(e.store.failed_recently("abcd", crate::now_unix()));
}

#[test]
fn trailing_slash_symlink_is_refused() {
    let e = env();
    std::fs::create_dir(e.work.join("real")).unwrap();
    std::fs::write(e.work.join("real/keep"), "x").unwrap();
    std::os::unix::fs::symlink(e.work.join("real"), e.work.join("link")).unwrap();
    let (code, _, err) = rm_in(&e, "-rf link/");
    assert_eq!(code, 1);
    assert!(err.contains("symlink"), "{err}");
    assert!(e.work.join("real/keep").exists());
    assert_eq!(rm_in(&e, "-rf link").0, 0, "the link itself is fine");
    assert!(e.work.join("real/keep").exists());
}

#[test]
fn scratch_is_deleted_for_real() {
    let e = env();
    std::fs::create_dir_all(e.work.join("tmp/x")).unwrap();
    let scratch = |p: &Path| p.to_string_lossy().contains("/work/tmp");
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let code = rm::rm(&e.store, "plan01", &args("-rf tmp/x"), &e.work, &rm::Scratch(&scratch), &mut out, &mut err);
    assert_eq!(code, 0);
    assert!(!e.work.join("tmp/x").exists());
    assert!(e.store.items("plan01").is_empty());
}

#[test]
fn a_full_disk_deletes_nothing() {
    let e = env();
    std::fs::write(e.work.join("f"), "x").unwrap();
    let full = e.store.clone().with_fault(Fault::DiskFull);
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let code = rm::rm(&full, "plan01", &args("-f f"), &e.work, &rm::Scratch(&no_scratch), &mut out, &mut err);
    assert_eq!(code, 1);
    assert!(String::from_utf8(err).unwrap().contains("was not deleted"));
    assert!(e.work.join("f").exists());
    // Nothing can be recorded on a full disk: the next plan sees it and plans no net.
    assert!(!full.trashable(&e.work.join("f")));
}

// ---- git ----

fn git(dir: &Path, a: &[&str]) -> String {
    let out = Command::new("git")
        .current_dir(dir)
        .args([
            "-c",
            "user.name=T",
            "-c",
            "user.email=t@example.com",
            "-c",
            "commit.gpgsign=false",
            "-c",
            "init.defaultBranch=main",
        ])
        .args(a)
        .output()
        .unwrap();
    assert!(out.status.success(), "git {a:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8(out.stdout).unwrap().trim().to_owned()
}

fn repo(e: &Env) -> PathBuf {
    let r = e.work.join("repo");
    std::fs::create_dir(&r).unwrap();
    git(&r, &["init", "-q"]);
    std::fs::write(r.join("a.txt"), "1").unwrap();
    git(&r, &["add", "."]);
    git(&r, &["commit", "-qm", "one"]);
    r
}

fn net_git(e: &Env, cwd: &Path, line: &str) -> (u8, String) {
    let mut err = Vec::new();
    let code = git::git(&e.store, "plan01", &args(line), cwd, &mut err);
    (code, String::from_utf8(err).unwrap())
}

#[test]
fn branch_delete_keeps_a_backup_and_restores() {
    let e = env();
    let r = repo(&e);
    git(&r, &["checkout", "-qb", "feat/x"]);
    std::fs::write(r.join("b.txt"), "2").unwrap();
    git(&r, &["add", "."]);
    git(&r, &["commit", "-qm", "two"]);
    let tip = git(&r, &["rev-parse", "HEAD"]);
    git(&r, &["checkout", "-q", "main"]);
    assert_eq!(net_git(&e, &r, "branch -D feat/x").0, 0);
    assert!(git(&r, &["branch", "--list", "feat/x"]).is_empty());
    let refs = e.store.refs("plan01");
    assert_eq!((refs.len(), refs[0].oid.as_str()), (1, tip.as_str()));
    restore(&e.store, "plan01").unwrap();
    assert_eq!(git(&r, &["rev-parse", "feat/x"]), tip);
}

#[test]
fn reset_hard_keeps_head_worktree_and_colliding_files() {
    let e = env();
    let r = repo(&e);
    let one = git(&r, &["rev-parse", "HEAD"]);
    std::fs::write(r.join("c.txt"), "tracked later").unwrap();
    git(&r, &["add", "."]);
    git(&r, &["commit", "-qm", "two"]);
    let two = git(&r, &["rev-parse", "HEAD"]);
    git(&r, &["reset", "-q", "--hard", &one]);
    // Now: c.txt is untracked here and tracked by `two`; a.txt has a change.
    std::fs::write(r.join("c.txt"), "my untracked work").unwrap();
    std::fs::write(r.join("a.txt"), "changed").unwrap();
    assert_eq!(net_git(&e, &r, &format!("reset --hard {two}")).0, 0);
    assert_eq!(std::fs::read_to_string(r.join("c.txt")).unwrap(), "tracked later");
    assert_eq!(std::fs::read_to_string(r.join("a.txt")).unwrap(), "1");
    let kinds: Vec<String> = e.store.refs("plan01").iter().map(|r| r.what.clone()).collect();
    assert_eq!(kinds, vec!["head", "stash"]);
    assert_eq!(e.store.items("plan01").len(), 1, "the colliding untracked file");
    std::fs::remove_file(r.join("c.txt")).unwrap();
    let lines = restore(&e.store, "plan01").unwrap();
    assert!(lines.iter().any(|l| l.contains("stash")), "{lines:?}");
    assert_eq!(std::fs::read_to_string(r.join("c.txt")).unwrap(), "my untracked work");
    git(&r, &["stash", "pop", "-q"]);
    assert_eq!(std::fs::read_to_string(r.join("a.txt")).unwrap(), "changed");
}

#[test]
fn worktree_remove_force_goes_to_the_trash_and_comes_back() {
    let e = env();
    let r = repo(&e);
    let w = e.work.join("wt");
    git(&r, &["worktree", "add", "-q", w.to_str().unwrap(), "-b", "wip"]);
    std::fs::write(w.join("untracked.txt"), "precious").unwrap();
    std::fs::write(w.join("a.txt"), "dirty").unwrap();
    assert_eq!(net_git(&e, &r, "worktree remove --force ../wt").0, 0);
    assert!(!w.exists());
    assert!(!git(&r, &["worktree", "list"]).contains("/wt "), "git dropped its record");
    restore(&e.store, "plan01").unwrap();
    assert_eq!(std::fs::read_to_string(w.join("untracked.txt")).unwrap(), "precious");
    assert_eq!(std::fs::read_to_string(w.join("a.txt")).unwrap(), "dirty");
    assert!(git(&r, &["worktree", "list"]).contains("[wip]"));
    assert_eq!(git(&w, &["status", "--porcelain"]), "M a.txt\n?? untracked.txt".to_owned());
}

#[test]
fn a_worktree_named_by_its_ending_is_resolved_and_an_unknown_one_refused() {
    let e = env();
    let r = repo(&e);
    let w = e.work.join("deep").join("wt-name");
    std::fs::create_dir_all(w.parent().unwrap()).unwrap();
    git(&r, &["worktree", "add", "-q", w.to_str().unwrap()]);
    std::fs::write(w.join("untracked.txt"), "keep").unwrap();
    // Not a path from here: git resolves it by the ending of the worktree's path; so does the net.
    assert_eq!(net_git(&e, &r, "worktree remove --force wt-name").0, 0);
    assert!(!w.exists());
    assert_eq!(e.store.items("plan01").len(), 1, "kept, not removed un-netted");
    // A name nothing resolves: nothing runs.
    let (code, err) = net_git(&e, &r, "worktree remove --force no-such-tree");
    assert_eq!(code, git::REFUSED, "{err}");
}

#[test]
fn locked_or_main_worktrees_are_left_to_git() {
    let e = env();
    let r = repo(&e);
    let w = e.work.join("wt");
    git(&r, &["worktree", "add", "-q", w.to_str().unwrap()]);
    git(&r, &["worktree", "lock", w.to_str().unwrap()]);
    let (code, _) = net_git(&e, &r, "worktree remove --force ../wt");
    assert_ne!(code, 0, "git refuses a locked worktree with one --force");
    assert!(w.exists());
    assert!(e.store.items("plan01").is_empty());
    assert_ne!(net_git(&e, &r, "worktree remove --force .").0, 0, "the main worktree");
    assert!(r.join("a.txt").exists());
}

#[test]
fn clean_moves_exactly_the_dry_run() {
    let e = env();
    let r = repo(&e);
    std::fs::write(r.join(".gitignore"), "ignored.log\n").unwrap();
    git(&r, &["add", "."]);
    git(&r, &["commit", "-qm", "ignore"]);
    std::fs::write(r.join("ignored.log"), "x").unwrap();
    std::fs::write(r.join("new file.txt"), "x").unwrap();
    std::fs::create_dir(r.join("newdir")).unwrap();
    std::fs::write(r.join("newdir/f"), "x").unwrap();
    assert_eq!(net_git(&e, &r, "clean -fdq").0, 0);
    assert!(!r.join("new file.txt").exists());
    assert!(!r.join("newdir").exists());
    assert!(r.join("ignored.log").exists(), "no -x: ignored files stay");
    assert_eq!(e.store.items("plan01").len(), 2);
    restore(&e.store, "plan01").unwrap();
    assert!(r.join("new file.txt").exists() && r.join("newdir/f").exists());
}

#[test]
fn force_push_keeps_the_old_remote_tip() {
    let e = env();
    let r = repo(&e);
    let bare = e.work.join("remote.git");
    git(&e.work, &["init", "-q", "--bare", bare.to_str().unwrap()]);
    git(&r, &["remote", "add", "origin", bare.to_str().unwrap()]);
    git(&r, &["checkout", "-qb", "feat"]);
    std::fs::write(r.join("b.txt"), "2").unwrap();
    git(&r, &["add", "."]);
    git(&r, &["commit", "-qm", "two"]);
    let old = git(&r, &["rev-parse", "HEAD"]);
    git(&r, &["push", "-q", "origin", "feat"]);
    git(&r, &["reset", "-q", "--hard", "HEAD~1"]);
    std::fs::write(r.join("c.txt"), "3").unwrap();
    git(&r, &["add", "."]);
    git(&r, &["commit", "-qm", "rewritten"]);
    assert_eq!(net_git(&e, &r, "push -q --force origin feat").0, 0);
    let refs = e.store.refs("plan01");
    assert_eq!((refs.len(), refs[0].oid.as_str(), refs[0].what.as_str()), (1, old.as_str(), "remote"));
    // A new branch has nothing to keep.
    assert_eq!(net_git(&e, &r, "push -q --force origin feat:brand-new").0, 0);
    assert_eq!(e.store.refs("plan01").len(), 1);
    restore(&e.store, "plan01").unwrap();
    assert_eq!(git(&r, &["rev-parse", "reins/restored/origin/feat"]), old);
}

#[test]
fn a_force_push_is_pinned_to_the_tip_it_kept() {
    let e = env();
    let r = repo(&e);
    let bare = e.work.join("remote.git");
    git(&e.work, &["init", "-q", "--bare", bare.to_str().unwrap()]);
    git(&r, &["remote", "add", "origin", bare.to_str().unwrap()]);
    git(&r, &["push", "-q", "origin", "main:feat"]);
    git(&r, &["fetch", "-q", "origin"]);
    let tracking = git(&r, &["rev-parse", "refs/remotes/origin/feat"]);
    // Someone else moves feat on the remote; our tracking ref doesn't know.
    let other = e.work.join("other");
    git(&e.work, &["clone", "-q", bare.to_str().unwrap(), other.to_str().unwrap()]);
    git(&other, &["checkout", "-q", "feat"]);
    std::fs::write(other.join("theirs.txt"), "t").unwrap();
    git(&other, &["add", "."]);
    git(&other, &["commit", "-qm", "theirs"]);
    git(&other, &["push", "-q", "origin", "feat"]);
    let theirs = git(&other, &["rev-parse", "HEAD"]);
    // A bare --force-with-lease still means "as my tracking ref says": the backup fetch must not move it.
    let (code, _) = net_git(&e, &r, "push -q --force-with-lease origin main:feat");
    assert_ne!(code, 0, "their commit is not overwritten");
    assert_eq!(git(&r, &["rev-parse", "refs/remotes/origin/feat"]), tracking, "the tracking ref did not move");
    assert_eq!(git(&e.work, &["--git-dir", bare.to_str().unwrap(), "rev-parse", "feat"]), theirs);
    // A plain --force: kept, then pinned to the kept tip, so it lands.
    assert_eq!(net_git(&e, &r, "push -q --force origin main:feat").0, 0);
    assert!(e.store.refs("plan01").iter().any(|b| b.oid == theirs));
}

#[test]
fn a_force_push_to_a_name_that_is_a_tag_there_runs_nothing() {
    let e = env();
    let r = repo(&e);
    let bare = e.work.join("remote.git");
    git(&e.work, &["init", "-q", "--bare", bare.to_str().unwrap()]);
    git(&r, &["remote", "add", "origin", bare.to_str().unwrap()]);
    git(&r, &["tag", "v1"]);
    git(&r, &["push", "-q", "origin", "v1"]);
    let (code, err) = net_git(&e, &r, "push -q -f origin v1");
    assert_eq!(code, git::REFUSED, "{err}");
    assert!(err.contains("tag"), "{err}");
}

#[test]
fn unknown_shapes_run_nothing() {
    let e = env();
    let r = repo(&e);
    for line in ["status", "branch -m main other", "push --mirror origin", "clean -n", "--git-dir=x branch -D main"] {
        let (code, err) = net_git(&e, &r, line);
        assert_eq!(code, git::REFUSED, "{line}");
        assert!(err.contains("nothing was run"), "{err}");
    }
    assert_eq!(git(&r, &["branch", "--show-current"]), "main");
}

#[test]
fn net_run_goes_by_its_plan() {
    let e = env();
    let now = crate::now_unix();
    let plan = PlanRecord {
        id: "plan02".to_owned(),
        at: now,
        programs: vec!["rm".to_owned()],
        cwds: vec![e.work.to_string_lossy().into_owned()],
        ..PlanRecord::default()
    };
    assert_eq!(refusal(Some(&plan), "rm", &e.work, now), None);
    assert!(refusal(None, "rm", &e.work, now).is_some(), "no plan");
    assert!(refusal(Some(&plan), "git", &e.work, now).is_some(), "another program");
    assert!(refusal(Some(&plan), "rm", &e.work.join(".."), now).is_some(), "another folder");
    assert!(refusal(Some(&plan), "rm", &e.work, now + PLAN_SECS + 1).is_some(), "too old");
}
