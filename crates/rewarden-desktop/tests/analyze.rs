//! Push analysis against repositories and packs made by the real git CLI.

mod analyze_util;

use std::path::Path;

use analyze_util::{Fixture, ZERO, cmd, git, git_bytes, lines, oid};
use rewarden_desktop::git::object::hash_object;
use rewarden_desktop::git::pack::Pack;
use rewarden_desktop::git::{Command, NoRemote, Remote, analyze_push};
use rewarden_proto::desktop::{FileChange, FileStatus, PushSummary, RefChange, RefUpdate};

async fn analyze(commands: &[Command], pack: Option<&Path>, remote: &dyn Remote) -> PushSummary {
    let s = analyze_push("o/r", commands, &[], pack, remote).await;
    s.validate().unwrap_or_else(|e| panic!("{e}: {s:#?}"));
    s
}

fn file<'a>(u: &'a RefUpdate, path: &str) -> &'a FileChange {
    u.files.iter().find(|f| f.path == path).unwrap_or_else(|| panic!("{path} not in {:#?}", u.files))
}

fn subjects(u: &RefUpdate) -> Vec<&str> {
    u.commits.iter().map(|c| c.subject.as_str()).collect()
}

/// Server at `base`: README and a big file.
fn base_repo() -> (Fixture, String) {
    let f = Fixture::new();
    let base =
        f.commit(&[("README.md", Some(b"# Project\n\nHello.\n")), ("src/big.txt", Some(&lines(400, "v1")))], "Start");
    f.publish("main");
    (f, base)
}

#[tokio::test]
async fn a_fast_forward_lists_commits_files_and_lines() {
    let (f, base) = base_repo();
    let mut big = lines(400, "v1");
    big.extend_from_slice(b"one more line\n");
    f.commit(&[("src/big.txt", Some(&big)), ("src/new.rs", Some(b"fn main() {}\n"))], "Add a line and main");
    let new =
        f.commit(&[("README.md", Some(b"# Project\n\nHello, world.\nBye.\n"))], "Reword the readme\n\nWith a body.");
    let pack = f.push_pack(&new, &[&base]);

    let s = analyze(&[cmd(&base, &new, "refs/heads/main")], Some(pack.path()), &f.remote()).await;
    assert!(s.notes.is_empty(), "{:?}", s.notes);
    assert!(s.pack_bytes > 0);
    let u = &s.updates[0];
    assert_eq!((u.change, u.fast_forward, u.commit_count), (RefChange::Update, Some(true), 2));
    assert_eq!(subjects(u), ["Reword the readme", "Add a line and main"]);
    assert_eq!(u.commits[0].author, "Ada Lovelace <ada@example.com>");
    assert_eq!(u.commits[0].sha, new);
    assert!(u.commits[0].date > u.commits[1].date);
    assert_eq!(u.files_changed, 3);
    let paths: Vec<_> = u.files.iter().map(|f| f.path.as_str()).collect();
    assert_eq!(paths, ["README.md", "src/big.txt", "src/new.rs"]);
    let readme = file(u, "README.md");
    assert_eq!((readme.status, readme.additions, readme.deletions), (FileStatus::Modified, Some(2), Some(1)));
    let big = file(u, "src/big.txt");
    assert_eq!((big.status, big.additions, big.deletions, big.binary), (FileStatus::Modified, Some(1), Some(0), false));
    let main = file(u, "src/new.rs");
    assert_eq!((main.status, main.additions, main.deletions), (FileStatus::Added, Some(1), Some(0)));
    assert_eq!((u.additions, u.deletions), (Some(4), Some(1)));
    assert!(!s.once_only());
}

#[tokio::test]
async fn a_force_push_after_a_rebase_rewrites_history() {
    let (f, base) = base_repo();
    let published = f.commit(&[("a.txt", Some(b"a\n"))], "Published work");
    f.publish("main");
    git(&f.work, &["reset", "-q", "--hard", &base]);
    let rewritten = f.commit(&[("a.txt", Some(b"A\n"))], "Rewritten work");
    let pack = f.push_pack(&rewritten, &[&published]);

    let s = analyze(&[cmd(&published, &rewritten, "refs/heads/main")], Some(pack.path()), &f.remote()).await;
    let u = &s.updates[0];
    assert_eq!((u.fast_forward, u.commit_count), (Some(false), 1));
    assert_eq!(subjects(u), ["Rewritten work"]);
    // Files compare the old tip with the new one.
    let a = file(u, "a.txt");
    assert_eq!((a.status, a.additions, a.deletions), (FileStatus::Modified, Some(1), Some(1)));
    assert!(s.once_only());

    // Without the server, nobody can tell: unknown, asked like a force push.
    let s = analyze(&[cmd(&published, &rewritten, "refs/heads/main")], Some(pack.path()), &NoRemote).await;
    assert_eq!(s.updates[0].fast_forward, None);
    assert!(s.notes.iter().any(|n| n.contains("rewrites history is unknown")), "{:?}", s.notes);
    assert!(s.once_only());
}

#[tokio::test]
async fn a_new_branch_is_compared_with_where_it_starts() {
    let (f, base) = base_repo();
    git(&f.work, &["checkout", "-q", "-b", "feature"]);
    f.commit(&[("feature.txt", Some(b"one\n"))], "Start the feature");
    let tip = f.commit(&[("feature.txt", Some(b"one\ntwo\n")), ("README.md", None)], "Finish the feature");
    let pack = f.push_pack(&tip, &[&base]);

    let s = analyze(&[cmd(ZERO, &tip, "refs/heads/feature")], Some(pack.path()), &f.remote()).await;
    let u = &s.updates[0];
    assert!(s.notes.is_empty(), "{:?}", s.notes);
    assert_eq!((u.change, u.fast_forward, u.commit_count), (RefChange::Create, None, 2));
    assert_eq!(u.files_changed, 2);
    let feature = file(u, "feature.txt");
    assert_eq!((feature.status, feature.additions), (FileStatus::Added, Some(2)));
    let readme = file(u, "README.md");
    assert_eq!((readme.status, readme.deletions), (FileStatus::Deleted, Some(3)));
    assert_eq!(s.resource("o/r"), "o/r@feature");

    // A branch made at a commit the server has brings nothing.
    let s = analyze(&[cmd(ZERO, &base, "refs/heads/copy")], None, &f.remote()).await;
    let u = &s.updates[0];
    assert_eq!((u.commit_count, u.files_changed, u.additions), (0, 0, Some(0)));
    assert!(s.notes.is_empty(), "{:?}", s.notes);
}

#[tokio::test]
async fn new_history_without_a_parent_is_noted() {
    let f = Fixture::new();
    let root = f.commit(&[("a.txt", Some(b"a\n"))], "Root");
    let pack = f.push_pack(&root, &[]);
    let s = analyze(&[cmd(ZERO, &root, "refs/heads/main")], Some(pack.path()), &f.remote()).await;
    let u = &s.updates[0];
    assert_eq!((u.commit_count, u.files_changed), (1, 0));
    assert!(s.notes.iter().any(|n| n.contains("no parent on GitHub")), "{:?}", s.notes);
}

#[tokio::test]
async fn a_deletion_has_no_pack() {
    let (f, base) = base_repo();
    let s = analyze(&[cmd(&base, ZERO, "refs/heads/old")], None, &f.remote()).await;
    let u = &s.updates[0];
    assert_eq!((u.change, u.fast_forward, u.commit_count, u.files_changed), (RefChange::Delete, None, 0, 0));
    assert_eq!(s.pack_bytes, 0);
    assert!(s.once_only());
}

#[tokio::test]
async fn tags_point_at_their_commit() {
    let (f, base) = base_repo();
    let tip = f.commit(&[("CHANGELOG", Some(b"1.0\n"))], "Release 1.0");
    git(&f.work, &["tag", "-a", "v1.0", "-m", "Version 1.0"]);
    git(&f.work, &["tag", "light"]);
    let tag = f.rev("refs/tags/v1.0");
    assert_ne!(tag, tip);

    // An annotated tag of a new commit: the tag object and the commit are in the pack.
    let pack = f.push_pack(&tag, &[&base]);
    let s = analyze(&[cmd(ZERO, &tag, "refs/tags/v1.0")], Some(pack.path()), &f.remote()).await;
    let u = &s.updates[0];
    assert_eq!(u.new, tag);
    assert_eq!((u.commit_count, u.files_changed), (1, 1));
    assert_eq!(subjects(u), ["Release 1.0"]);
    assert_eq!(s.tool(), rewarden_proto::desktop::GIT_TAG_PUSH_TOOL);
    assert!(!s.once_only());

    // A lightweight tag and a branch together.
    let pack = f.push_pack(&tip, &[&base]);
    let s = analyze(
        &[cmd(&base, &tip, "refs/heads/main"), cmd(ZERO, &tip, "refs/tags/light")],
        Some(pack.path()),
        &f.remote(),
    )
    .await;
    assert_eq!(s.updates[1].commit_count, 1);
    assert_eq!(s.updates[0].fast_forward, Some(true));

    // An annotated tag of a commit the server has: only the tag object is new.
    f.publish("main");
    let pack = f.push_pack(&tag, &[&tip]);
    let s = analyze(&[cmd(ZERO, &tag, "refs/tags/v1.0")], Some(pack.path()), &f.remote()).await;
    assert_eq!((s.updates[0].commit_count, s.updates[0].files_changed), (0, 0));
    assert!(s.notes.is_empty(), "{:?}", s.notes);
}

#[tokio::test]
async fn a_moved_tag_is_risky_even_forward() {
    let (f, base) = base_repo();
    git(&f.work, &["tag", "v1"]);
    f.publish("refs/tags/v1");
    let tip = f.commit(&[("x", Some(b"x\n"))], "Later");
    let pack = f.push_pack(&tip, &[&base]);
    let s = analyze(&[cmd(&base, &tip, "refs/tags/v1")], Some(pack.path()), &f.remote()).await;
    assert_eq!(s.updates[0].fast_forward, Some(true));
    assert!(s.once_only());
    // Back to an older commit the server has: no pack, not a fast-forward.
    f.publish(&format!("{tip}:refs/heads/published"));
    let s = analyze(&[cmd(&tip, &base, "refs/tags/v1")], None, &f.remote()).await;
    assert_eq!(s.updates[0].fast_forward, Some(false));
}

#[tokio::test]
async fn moving_a_branch_to_a_commit_the_server_has_asks_its_ancestry() {
    let (f, base) = base_repo();
    let tip = f.commit(&[("x", Some(b"x\n"))], "Later");
    f.publish("main:refs/heads/other");
    let s = analyze(&[cmd(&base, &tip, "refs/heads/main")], None, &f.remote()).await;
    let u = &s.updates[0];
    assert_eq!((u.fast_forward, u.commit_count, u.files_changed), (Some(true), 0, 1));
    let s = analyze(&[cmd(&tip, &base, "refs/heads/other")], None, &f.remote()).await;
    assert_eq!(s.updates[0].fast_forward, Some(false));
    assert_eq!(file(&s.updates[0], "x").status, FileStatus::Deleted);
}

#[tokio::test]
async fn thin_packs_build_on_the_servers_objects() {
    let (f, base) = base_repo();
    let mut big = lines(400, "v1");
    big.splice(0..0, b"inserted at the top\n".iter().copied());
    let new = f.commit(&[("src/big.txt", Some(&big))], "Edit the big file");
    let pack = f.push_pack(&new, &[&base]);

    let opened = Pack::open(pack.path()).unwrap();
    assert!(!opened.missing_bases().is_empty(), "git made no thin delta");
    assert!(opened.delta_counts().1 > 0);

    let s = analyze(&[cmd(&base, &new, "refs/heads/main")], Some(pack.path()), &f.remote()).await;
    assert!(s.notes.is_empty(), "{:?}", s.notes);
    let big = file(&s.updates[0], "src/big.txt");
    assert_eq!((big.additions, big.deletions), (Some(1), Some(0)));

    // Without the server's objects, what builds on them is unknown, and said.
    let s = analyze(&[cmd(&base, &new, "refs/heads/main")], Some(pack.path()), &NoRemote).await;
    let u = &s.updates[0];
    assert_eq!(u.commit_count, 1);
    assert!(s.notes.iter().any(|n| n.contains("could not be read")), "{:?}", s.notes);
    assert_eq!(u.additions, None);
}

/// Every object `git rev-list --objects` puts in the pack reads back with the right content.
fn assert_pack_matches_git(f: &Fixture, pack: &Path, new: &str, have: &str) {
    let mut opened = Pack::open(pack).unwrap();
    assert_eq!(opened.unresolved(), 0);
    let listed = git(&f.work, &["rev-list", "--objects", new, &format!("^{have}")]);
    let mut n = 0;
    for line in listed.lines() {
        let hex = line.split(' ').next().unwrap();
        let (kind, data) = opened.read(&oid(hex)).unwrap().unwrap_or_else(|| panic!("{line} missing"));
        assert_eq!(hash_object(kind, &data), oid(hex));
        assert_eq!(&*data, git_bytes(&f.work, &["cat-file", kind.name(), hex], None).as_slice());
        n += 1;
    }
    assert_eq!(n, opened.object_count());
}

#[tokio::test]
async fn ofs_and_ref_deltas_inside_the_pack() {
    let (f, base) = base_repo();
    for i in 0..6 {
        let mut big = lines(400, "v1");
        big.extend_from_slice(format!("revision {i}\n").as_bytes());
        f.commit(&[("src/big.txt", Some(&big)), ("src/copy.txt", Some(&big))], &format!("Revision {i}"));
    }
    let new = f.head();
    let deltas = ["--window=50", "--depth=50", "--no-reuse-delta"];
    let ofs = f.pack(&new, &[&base], &[deltas.as_slice(), &["--delta-base-offset"]].concat());
    let refs = f.pack(&new, &[&base], &deltas);
    let (o, _) = Pack::open(ofs.path()).unwrap().delta_counts();
    let (o2, r2) = Pack::open(refs.path()).unwrap().delta_counts();
    assert!(o > 0, "no OFS deltas");
    assert!(o2 == 0 && r2 > 0, "no REF deltas");
    assert_pack_matches_git(&f, ofs.path(), &new, &base);
    assert_pack_matches_git(&f, refs.path(), &new, &base);

    for pack in [&ofs, &refs] {
        let s = analyze(&[cmd(&base, &new, "refs/heads/main")], Some(pack.path()), &f.remote()).await;
        let u = &s.updates[0];
        assert_eq!((u.commit_count, u.fast_forward), (6, Some(true)));
        assert_eq!(subjects(u)[0], "Revision 5");
        assert_eq!((file(u, "src/big.txt").additions, file(u, "src/copy.txt").additions), (Some(1), Some(401)));
    }
}

#[tokio::test]
async fn binary_and_type_changed_files() {
    let (f, _) = base_repo();
    std::os::unix::fs::symlink("README.md", f.work.join("link")).unwrap();
    let mid = f.commit(&[("logo.png", Some(b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR"))], "Add a logo and a link");
    f.publish("main");
    std::fs::remove_file(f.work.join("link")).unwrap();
    let new =
        f.commit(&[("link", Some(b"now a file\n")), ("logo.png", Some(b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR2"))], "Swap");
    let pack = f.push_pack(&new, &[&mid]);
    let s = analyze(&[cmd(&mid, &new, "refs/heads/main")], Some(pack.path()), &f.remote()).await;
    let u = &s.updates[0];
    let logo = file(u, "logo.png");
    assert_eq!((logo.status, logo.binary, logo.additions, logo.deletions), (FileStatus::Modified, true, None, None));
    let link = file(u, "link");
    assert_eq!((link.status, link.additions, link.deletions), (FileStatus::TypeChanged, Some(1), Some(1)));
    // Binary files have no lines: the totals still count the text.
    assert_eq!((u.additions, u.deletions), (Some(1), Some(1)));
}

#[tokio::test]
async fn submodules_have_no_lines() {
    let (f, base) = base_repo();
    f.commit(&[("notes.txt", Some(b"n\n"))], "A file");
    git(&f.work, &["update-index", "--add", "--cacheinfo", &format!("160000,{base},vendor/lib")]);
    git(&f.work, &["commit", "-q", "-m", "And a submodule"]);
    let new = f.head();
    let pack = f.push_pack(&new, &[&base]);
    let s = analyze(&[cmd(&base, &new, "refs/heads/main")], Some(pack.path()), &f.remote()).await;
    let u = &s.updates[0];
    let sub = file(u, "vendor/lib");
    assert_eq!((sub.status, sub.additions, sub.binary), (FileStatus::Added, None, false));
    assert_eq!((u.additions, u.deletions), (Some(1), Some(0)));
    assert!(s.notes.is_empty(), "{:?}", s.notes);
}

#[tokio::test]
async fn deep_paths_and_directories_replaced_by_files() {
    let (f, _) = base_repo();
    let deep = (0..40).fold(String::new(), |p, i| p + &format!("d{i}/")) + "leaf.txt";
    let mid = f.commit(&[(deep.as_str(), Some(b"leaf\n")), ("dir/inner.txt", Some(b"inner\n"))], "Deep");
    f.publish("main");
    f.commit(&[(deep.as_str(), Some(b"leaf\nmore\n")), ("dir", None)], "Deeper");
    let new = f.commit(&[("dir", Some(b"a file now\n"))], "Dir to file");
    let pack = f.push_pack(&new, &[&mid]);
    let s = analyze(&[cmd(&mid, &new, "refs/heads/main")], Some(pack.path()), &f.remote()).await;
    let u = &s.updates[0];
    assert_eq!(u.commit_count, 2);
    assert_eq!(file(u, &deep).additions, Some(1));
    assert_eq!(file(u, "dir/inner.txt").status, FileStatus::Deleted);
    assert_eq!(file(u, "dir").status, FileStatus::Added);
    assert_eq!(u.files_changed, 3);
}

#[tokio::test]
async fn long_histories_and_many_files_are_capped() {
    let (f, base) = base_repo();
    let many: Vec<(String, Vec<u8>)> =
        (0..350).map(|i| (format!("files/f{i:03}.txt"), format!("{i}\n").into_bytes())).collect();
    let refs: Vec<(&str, Option<&[u8]>)> = many.iter().map(|(p, c)| (p.as_str(), Some(c.as_slice()))).collect();
    f.commit(&refs, "Many files");
    for i in 0..59 {
        f.commit(&[], &format!("Commit {i}"));
    }
    let new = f.head();
    let pack = f.push_pack(&new, &[&base]);
    let s = analyze(&[cmd(&base, &new, "refs/heads/main")], Some(pack.path()), &f.remote()).await;
    let u = &s.updates[0];
    assert_eq!((u.commit_count, u.commits.len()), (60, 50));
    assert_eq!(subjects(u)[0], "Commit 58");
    assert_eq!((u.files_changed, u.files.len()), (350, 300));
    assert_eq!((u.additions, u.deletions), (Some(350), Some(0)));
}

#[tokio::test]
async fn damaged_packs_are_notes_not_failures() {
    let (f, base) = base_repo();
    let new = f.commit(&[("a.txt", Some(&lines(200, "a")))], "A");
    let pack = f.push_pack(&new, &[&base]);
    let bytes = std::fs::read(pack.path()).unwrap();

    let truncated = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(truncated.path(), &bytes[..bytes.len() / 2]).unwrap();
    let mut flipped = bytes.clone();
    let mid = flipped.len() / 2;
    flipped[mid] ^= 0x55;
    let corrupt = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(corrupt.path(), &flipped).unwrap();
    let not_a_pack = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(not_a_pack.path(), b"hello").unwrap();

    for p in [truncated.path(), corrupt.path(), not_a_pack.path(), Path::new("/nonexistent/pack")] {
        let s = analyze(&[cmd(&base, &new, "refs/heads/main")], Some(p), &f.remote()).await;
        let u = &s.updates[0];
        assert!(s.notes.iter().any(|n| n.starts_with("The pack could not be read")), "{:?}", s.notes);
        // The new commit is not on the server: its ancestry is unknown.
        assert_eq!((u.commit_count, u.fast_forward), (0, None));
    }
}

#[tokio::test]
async fn objects_missing_everywhere_are_notes() {
    let (f, base) = base_repo();
    git(&f.work, &["checkout", "-q", "-b", "feature"]);
    let tip = f.commit(&[("x", Some(b"x\n"))], "On a base the server does not know");
    // The pack leaves out `base`, and the remote knows nothing of it.
    let pack = f.push_pack(&tip, &[&base]);
    let s = analyze(&[cmd(ZERO, &tip, "refs/heads/feature")], Some(pack.path()), &NoRemote).await;
    let u = &s.updates[0];
    assert_eq!(u.commit_count, 1);
    assert_eq!(u.files_changed, 0);
    assert!(s.notes.iter().any(|n| n.contains("files could not be compared")), "{:?}", s.notes);
}

#[tokio::test]
async fn options_and_odd_commands_still_validate() {
    let (f, base) = base_repo();
    let options: Vec<String> = (0..12).map(|i| format!("ci.skip{i}\u{7}")).collect();
    let s = analyze_push("o/r", &[cmd(&base, &base, "refs/heads/main")], &options, None, &f.remote()).await;
    s.validate().unwrap();
    assert_eq!(s.push_options.len(), 10);
    assert_eq!(s.push_options[0], "ci.skip0");
    assert_eq!(s.updates[0].fast_forward, Some(true));
    assert!(s.notes.iter().any(|n| n.contains("more push options")));

    // SHA-256 ids cannot be described: the summary says so and does not validate (the push is refused).
    let long = "a".repeat(64);
    let s = analyze_push("o/r", &[cmd(&long, &long, "refs/heads/main")], &[], None, &NoRemote).await;
    assert!(s.validate().is_err());
    assert!(s.notes.iter().any(|n| n.contains("SHA-256")));
}

/// A large push (thousands of files, tens of MiB): run with `--release -- --ignored --nocapture` to see the time.
#[tokio::test]
#[ignore = "slow; a performance check"]
async fn a_large_push_is_read_quickly() {
    let (f, base) = base_repo();
    for batch in 0..4 {
        let files: Vec<(String, Vec<u8>)> = (0..1000)
            .map(|i| (format!("b{batch}/d{}/f{i}.txt", i % 37), lines(150 + i % 50, &format!("{batch}-{i}"))))
            .collect();
        let refs: Vec<(&str, Option<&[u8]>)> = files.iter().map(|(p, c)| (p.as_str(), Some(c.as_slice()))).collect();
        f.commit(&refs, &format!("Batch {batch}"));
    }
    let new = f.head();
    let pack = f.push_pack(&new, &[&base]);
    let started = std::time::Instant::now();
    let s = analyze(&[cmd(&base, &new, "refs/heads/main")], Some(pack.path()), &f.remote()).await;
    let took = started.elapsed();
    eprintln!("{} pack bytes, {} files: {took:?}", s.pack_bytes, s.updates[0].files_changed);
    assert_eq!(s.updates[0].files_changed, 4000);
    assert!(took < std::time::Duration::from_secs(30));
}
