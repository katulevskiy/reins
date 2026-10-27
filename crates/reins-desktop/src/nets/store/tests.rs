use super::*;

fn store() -> (tempfile::TempDir, Store, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("nets");
    let work = dir.path().join("work");
    std::fs::create_dir_all(&work).unwrap();
    let s = Store::at(&root);
    (dir, s, std::fs::canonicalize(&work).unwrap())
}

#[test]
fn trash_and_restore_bring_it_back() {
    let (_d, s, work) = store();
    std::fs::create_dir_all(work.join("a/b")).unwrap();
    std::fs::write(work.join("a/b/f.txt"), "hi").unwrap();
    let item = s.trash("plan01", &work.join("a"), None).unwrap();
    assert!(!work.join("a").exists());
    assert_eq!(item.what, "dir");
    assert_eq!(s.items("plan01").len(), 1);
    let line = s.restore_item("plan01", &item).unwrap();
    assert!(line.contains("Put back"));
    assert_eq!(std::fs::read_to_string(work.join("a/b/f.txt")).unwrap(), "hi");
    assert!(s.items("plan01").is_empty());
}

#[test]
fn symlinks_move_as_links() {
    let (_d, s, work) = store();
    std::fs::create_dir(work.join("target")).unwrap();
    std::fs::write(work.join("target/keep"), "x").unwrap();
    std::os::unix::fs::symlink(work.join("target"), work.join("link")).unwrap();
    let item = s.trash("plan01", &work.join("link"), None).unwrap();
    assert_eq!(item.what, "symlink");
    assert!(work.join("target/keep").exists(), "the link's target is untouched");
    assert!(std::fs::symlink_metadata(work.join("link")).is_err());
}

#[test]
fn the_trash_itself_and_above_are_refused() {
    let (d, s, work) = store();
    std::fs::write(work.join("f"), "x").unwrap();
    s.trash("plan01", &work.join("f"), None).unwrap();
    let root = std::fs::canonicalize(s.root()).unwrap();
    assert!(matches!(s.trash("plan02", &root, None), Err(NetError::Unsafe(_))));
    assert!(matches!(s.trash("plan02", &root.join("plan01"), None), Err(NetError::Unsafe(_))));
    assert!(matches!(s.trash("plan02", d.path(), None), Err(NetError::Unsafe(_))));
    assert!(!s.trashable(&root.join("plan01")));
    assert!(!s.trashable(d.path()));
    assert!(s.trashable(&work.join("anything")));
    // Through a symlink that leads into the trash.
    std::os::unix::fs::symlink(&root, work.join("sneaky")).unwrap();
    assert!(!s.trashable(&work.join("sneaky/plan01")));
    assert!(s.trash("plan02", &work.join("sneaky/plan01"), None).is_err());
    assert!(root.join("plan01").exists());
}

fn split(p: &Path) -> Option<u64> {
    Some(u64::from(p.to_string_lossy().contains("/work")))
}

#[test]
fn another_device_moves_nothing() {
    let (_d, s, work) = store();
    std::fs::write(work.join("f"), "x").unwrap();
    let s = s.with_devices(split);
    assert!(matches!(s.trash("plan01", &work.join("f"), None), Err(NetError::OtherDisk)));
    assert!(!s.trashable(&work.join("f")));
    assert!(work.join("f").exists());
}

#[test]
fn disk_full_moves_nothing() {
    let (_d, s, work) = store();
    std::fs::write(work.join("f"), "x").unwrap();
    let full = s.with_fault(Fault::DiskFull);
    assert!(full.trash("plan01", &work.join("f"), None).is_err());
    assert!(work.join("f").exists());
    assert!(full.items("plan01").is_empty(), "no half item left");
    assert!(full.snapshot("plan01", &work.join("f"), 1 << 20).is_err());
    assert!(
        full.record_ref(
            "plan01",
            &RefBackup {
                at: 0,
                repo: String::new(),
                backup: String::new(),
                oid: String::new(),
                what: "branch".to_owned(),
                name: "x".to_owned()
            }
        )
        .is_err()
    );
}

#[test]
fn concurrent_trashes_do_not_collide() {
    let (_d, s, work) = store();
    for i in 0..32 {
        std::fs::write(work.join(format!("f{i}")), i.to_string()).unwrap();
    }
    std::thread::scope(|scope| {
        for t in 0..8 {
            let (s, work) = (&s, &work);
            scope.spawn(move || {
                for i in (t..32).step_by(8) {
                    s.trash("plan01", &work.join(format!("f{i}")), None).unwrap();
                }
            });
        }
    });
    assert_eq!(s.items("plan01").len(), 32);
    // The same path twice at once: one wins, the other finds it gone.
    std::fs::write(work.join("g"), "x").unwrap();
    let results: Vec<bool> = std::thread::scope(|scope| {
        let mut hs = Vec::new();
        for _ in 0..4 {
            hs.push(scope.spawn(|| s.trash("plan02", &work.join("g"), None).is_ok()));
        }
        hs.into_iter().map(|h| h.join().unwrap()).collect()
    });
    assert_eq!(results.iter().filter(|ok| **ok).count(), 1, "{results:?}");
    assert_eq!(s.items("plan02").len(), 1);
}

#[test]
fn restore_refuses_to_overwrite_or_escape() {
    let (_d, s, work) = store();
    std::fs::create_dir(work.join("dir")).unwrap();
    std::fs::write(work.join("dir/f"), "old").unwrap();
    let item = s.trash("plan01", &work.join("dir/f"), None).unwrap();
    std::fs::write(work.join("dir/f"), "new").unwrap();
    assert!(s.restore_item("plan01", &item).unwrap_err().contains("there again"));
    std::fs::remove_file(work.join("dir/f")).unwrap();
    // The folder became a symlink to somewhere else.
    std::fs::create_dir(work.join("elsewhere")).unwrap();
    std::fs::remove_dir(work.join("dir")).unwrap();
    std::os::unix::fs::symlink(work.join("elsewhere"), work.join("dir")).unwrap();
    assert!(s.restore_item("plan01", &item).unwrap_err().contains("somewhere else"));
    assert!(!work.join("elsewhere/f").exists());
    // An edited record that points elsewhere.
    let mut forged = item.clone();
    forged.original = "/etc/../tmp/x".to_owned();
    assert!(s.restore_item("plan01", &forged).is_err());
    forged.original = format!("{}/x", work.display());
    forged.parent = "/etc".to_owned();
    assert!(s.restore_item("plan01", &forged).is_err(), "the parent must be the original's");
    let mut into_trash = item;
    into_trash.parent = std::fs::canonicalize(s.root()).unwrap().to_string_lossy().into_owned();
    into_trash.original = format!("{}/plan01", into_trash.parent);
    assert!(s.restore_item("plan01", &into_trash).is_err());
}

#[test]
fn snapshot_and_restore_keep_the_current_copy() {
    let (_d, s, work) = store();
    std::fs::write(work.join("notes.md"), "v1").unwrap();
    let item = s.snapshot("plan01", &work.join("notes.md"), 1 << 20).unwrap().unwrap();
    std::fs::write(work.join("notes.md"), "v2").unwrap();
    s.restore_item("plan01", &item).unwrap();
    assert_eq!(std::fs::read_to_string(work.join("notes.md")).unwrap(), "v1");
    let kept = s.items("plan01");
    assert_eq!(kept.len(), 1, "v2 is kept as a snapshot");
    assert_eq!(std::fs::read_to_string(s.root().join("plan01/items").join(&kept[0].id).join("data")).unwrap(), "v2");
    assert!(s.snapshot("plan01", &work.join("new-file"), 1 << 20).unwrap().is_none());
    assert!(matches!(s.snapshot("plan01", &work.join("notes.md"), 1), Err(NetError::TooLarge)));
}

#[test]
fn prune_keeps_what_is_young() {
    let (_d, s, work) = store();
    std::fs::write(work.join("f"), "x").unwrap();
    std::fs::write(work.join("g"), "x").unwrap();
    s.trash("plan01", &work.join("f"), None).unwrap();
    s.trash("plan02", &work.join("g"), None).unwrap();
    s.record_ref(
        "plan02",
        &RefBackup {
            at: crate::now_unix(),
            repo: "/nowhere".to_owned(),
            backup: "refs/reins/backup/plan02/1".to_owned(),
            oid: "0".repeat(40),
            what: "branch".to_owned(),
            name: "x".to_owned(),
        },
    )
    .unwrap();
    let now = crate::now_unix();
    assert_eq!(s.prune(now, &|_| true), 0);
    assert_eq!(s.list().len(), 2);
    // Eight days on: the trash is gone, the ref is kept for 30 days.
    assert_eq!(s.prune(now + 8 * DAY, &|_| true), 2);
    assert!(s.items("plan01").is_empty());
    assert_eq!(s.refs("plan02").len(), 1);
    assert!(!s.root().join("plan01").exists());
    assert_eq!(s.prune(now + 31 * DAY, &|_| true), 1);
    assert!(s.list().is_empty());
}

#[test]
fn failures_last_an_hour() {
    let (_d, s, _work) = store();
    let key = "ab12";
    assert!(!s.failed_recently(key, crate::now_unix()));
    s.fail(key, "disk full");
    assert!(s.failed_recently(key, crate::now_unix()));
    assert!(!s.failed_recently(key, crate::now_unix() + FAILED_SECS));
    s.fail("../escape", "x");
    assert!(!s.root().join("escape").exists());
}
