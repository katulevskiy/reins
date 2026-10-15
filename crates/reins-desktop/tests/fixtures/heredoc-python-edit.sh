cd /home/user/Documents/dev/reins-secrets-a/crates/reins-desktop/src/secrets_import && python3 - <<'EOF'
p='tests.rs'
s=open(p).read()
def rep(a,b):
    global s
    assert a in s, a
    s=s.replace(a,b)
rep('''    fn trash(&self, path: &Path) -> Result<(), String> {
        let mut t = self.trashed.lock().unwrap();
        let n: usize = t.values().map(Vec::len).sum();
        let to = self.trash_dir.join(n.to_string());
        std::fs::rename(path, &to).map_err(|e| format!("{}: {e}", path.display()))?;
        t.entry(path.to_path_buf()).or_default().push(to);
        Ok(())
    }

    fn restore(&self, path: &Path) -> Result<(), String> {
        let from = self.trashed.lock().unwrap().get_mut(path).and_then(Vec::pop).ok_or("not in the trash")?;
        std::fs::rename(from, path).map_err(|e| e.to_string())
    }''','''    fn trash(&self, path: &Path) -> Result<Option<String>, String> {
        let mut t = self.trashed.lock().unwrap();
        let n: usize = t.values().map(Vec::len).sum::<usize>() + self.purged.lock().unwrap().len();
        let to = self.trash_dir.join(format!("item-{n}"));
        std::fs::rename(path, &to).map_err(|e| format!("{}: {e}", path.display()))?;
        t.entry(path.to_path_buf()).or_default().push(to);
        Ok(Some(format!("item-{n}")))
    }

    fn restore(&self, path: &Path, trash_id: Option<&str>) -> Result<(), String> {
        let mut t = self.trashed.lock().unwrap();
        let items = t.get_mut(path).ok_or("not in the trash")?;
        let at = items
            .iter()
            .position(|i| i.file_name().and_then(|n| n.to_str()) == trash_id)
            .ok_or("not that item")?;
        let from = items.remove(at);
        assert!(std::fs::symlink_metadata(path).is_err(), "never over a file");
        std::fs::rename(from, path).map_err(|e| e.to_string())
    }

    fn purge(&self, path: &Path, trash_id: Option<&str>) -> Result<(), String> {
        let mut t = self.trashed.lock().unwrap();
        let items = t.get_mut(path).ok_or("not in the trash")?;
        let at = items
            .iter()
            .position(|i| i.file_name().and_then(|n| n.to_str()) == trash_id)
            .ok_or("not that item")?;
        let item = items.remove(at);
        std::fs::remove_file(&item).map_err(|e| e.to_string())?;
        self.purged.lock().unwrap().push(item);
        Ok(())
    }''')
rep('''    trashed: Mutex<BTreeMap<PathBuf, Vec<PathBuf>>>,
    signs: bool,''','''    trashed: Mutex<BTreeMap<PathBuf, Vec<PathBuf>>>,
    purged: Mutex<Vec<PathBuf>>,
    signs: bool,''')
rep('''            trashed: Mutex::default(),
            signs,''','''            trashed: Mutex::default(),
            purged: Mutex::default(),
            signs,''')
# undo test
rep('''    let (loaded_path, mut receipt) = load_receipt(&ctx(&fx).state_dir, "last").unwrap();
    assert_eq!(loaded_path, path);
    let lines = undo(&path, &mut receipt, &machine).unwrap();''','''    let c = ctx(&fx);
    let (loaded_path, mut receipt) = receipt::load(&c.state_dir, "last").unwrap();
    assert_eq!(loaded_path, path);
    let lines = receipt::undo(&c.home, &c.state_dir, &path, &mut receipt, &machine).unwrap();''')
rep('''    let (_, mut again) = load_receipt(&ctx(&fx).state_dir, path.to_str().unwrap()).unwrap();
    assert!(again.undone_at.is_some());
    assert!(undo(&path, &mut again, &machine).is_err());''','''    let (_, mut again) = receipt::load(&c.state_dir, path.to_str().unwrap()).unwrap();
    assert!(again.undone_at.is_some());
    assert!(receipt::undo(&c.home, &c.state_dir, &path, &mut again, &machine).is_err());''')
rep('''    let lines = finish(&path, &mut receipt, &signing).await.unwrap();''','''    let c = ctx(&fx);
    let lines = receipt::finish(&c.home, &c.state_dir, &path, &mut receipt, &signing).await.unwrap();''')
rep('''    // The key stays until the agent signs with it; `finish` tries again.
    assert!(home.join(".ssh/id_ed25519").exists());''','''    // The key stays until the agent signs with it (and says it is still on disk); `finish` tries again.
    assert!(home.join(".ssh/id_ed25519").exists());
    assert!(receipt.moved.iter().filter(|m| m.to.starts_with("Reins/ssh/")).all(|m| m.on_disk));''')
rep('''    assert!(!home.join(".ssh/id_ed25519").exists() && receipt.pending_keys.is_empty());''','''    assert!(!home.join(".ssh/id_ed25519").exists() && receipt.pending_keys.is_empty());
    assert!(receipt.moved.iter().filter(|m| m.to.starts_with("Reins/ssh/")).all(|m| !m.on_disk));''')
rep('''            to: "vault:acme-web/development/OPENAI_API_KEY".to_owned(),
        }],''','''            to: "vault:acme-web/development/OPENAI_API_KEY".to_owned(),
            on_disk: false,
        }],''')
rep('''            Change::Replaced {
                path: PathBuf::from("/h/.npmrc"),
            },
            Change::Trashed {
                path: PathBuf::from("/h/dev/acme-web/.env"),
            },''','''            Change::Replaced {
                path: PathBuf::from("/h/.npmrc"),
                trash_id: Some("item-1".to_owned()),
            },
            Change::Trashed {
                path: PathBuf::from("/h/dev/acme-web/.env"),
                trash_id: None,
            },''')
rep('''        pending_keys: vec![],
        undone_at: None,
    };''','''        pending_keys: vec![],
        undone_at: None,
        purged_at: None,
    };''')
rep('''        "moved": [{"item": "dotenv-0123/OPENAI_API_KEY", "finding": "dotenv-0123", "to": "vault:acme-web/development/OPENAI_API_KEY"}],''','''        "moved": [{"item": "dotenv-0123/OPENAI_API_KEY", "finding": "dotenv-0123", "to": "vault:acme-web/development/OPENAI_API_KEY", "on_disk": false}],''')
rep('''            {"kind": "replaced", "path": "/h/.npmrc"},''','''            {"kind": "replaced", "path": "/h/.npmrc", "trash_id": "item-1"},''')
s+='''
#[tokio::test]
async fn a_receipt_is_authenticated_and_only_its_own_folder_counts() {
    let fx = fixture();
    let report = report(&fx);
    let selected = select(&report, &[], true).unwrap();
    let machine = FakeMachine::new(fx.dir.path(), true);
    let (_, path) = run(&fx, &report, &selected, false, &FakeWriter::default(), &machine).await.unwrap();
    let path = path.unwrap();
    let c = ctx(&fx);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        assert_eq!(std::fs::metadata(path.parent().unwrap()).unwrap().permissions().mode() & 0o777, 0o700);
    }
    // Changed by anything else: refused, and nothing is undone.
    let text = std::fs::read_to_string(&path).unwrap();
    let tampered = text.replacen("/dev/acme-web/.env\\"", "/.ssh/authorized_keys\\"", 1);
    assert_ne!(tampered, text);
    std::fs::write(&path, &tampered).unwrap();
    let err = receipt::load(&c.state_dir, "last").unwrap_err();
    assert!(err.contains("refused"), "{err}");
    std::fs::write(&path, &text).unwrap();
    assert!(receipt::load(&c.state_dir, "last").is_ok());
    // A copy elsewhere, even a valid one, is not a receipt.
    let elsewhere = fx.dir.path().join("receipt.json");
    std::fs::write(&elsewhere, &text).unwrap();
    assert!(receipt::load(&c.state_dir, elsewhere.to_str().unwrap()).is_err());
    // Another install's key does not verify it.
    std::fs::write(c.state_dir.join("import-receipts.key"), [9u8; 32]).unwrap();
    assert!(receipt::load(&c.state_dir, "last").is_err());
}

#[test]
fn undo_touches_only_plain_places_in_the_home_folder() {
    let fx = fixture();
    let home = fx.home();
    assert!(receipt::check_place(&home, &home.join("dev/acme-web/.env")).is_ok());
    for bad in [
        home.join("dev/../../outside"),
        PathBuf::from("/etc/passwd"),
        PathBuf::from("relative/.env"),
        home.join("dev/./acme-web/.env"),
        home.clone(),
    ] {
        assert!(receipt::check_place(&home, &bad).is_err(), "{}", bad.display());
    }
    #[cfg(unix)]
    {
        let outside = fx.dir.path().join("outside");
        std::fs::create_dir_all(&outside).unwrap();
        std::os::unix::fs::symlink(&outside, home.join("dev/linked")).unwrap();
        std::os::unix::fs::symlink(home.join("dev/acme-web"), home.join("dev/inner-link")).unwrap();
        let err = receipt::check_place(&home, &home.join("dev/linked/.env")).unwrap_err();
        assert!(err.contains("symbolic link") || err.contains("outside"), "{err}");
        assert!(receipt::check_place(&home, &home.join("dev/inner-link/.env")).unwrap_err().contains("symbolic link"));
    }
}

#[tokio::test]
async fn undo_never_overwrites_a_file_that_is_back() {
    let fx = fixture();
    let report = report(&fx);
    let selected = select(&report, &[], true).unwrap();
    let machine = FakeMachine::new(fx.dir.path(), true);
    let (_, path) = run(&fx, &report, &selected, false, &FakeWriter::default(), &machine).await.unwrap();
    let path = path.unwrap();
    let env = fx.home().join("dev/acme-web/.env");
    std::fs::write(&env, "NEW=1\\n").unwrap();
    let c = ctx(&fx);
    let (_, mut r) = receipt::load(&c.state_dir, "last").unwrap();
    let lines = receipt::undo(&c.home, &c.state_dir, &path, &mut r, &machine).unwrap();
    assert!(lines.iter().any(|l| l.starts_with("not undone") && l.contains("not overwritten")), "{lines:?}");
    assert_eq!(std::fs::read_to_string(&env).unwrap(), "NEW=1\\n");
    assert!(r.undone_at.is_none(), "not all of it was undone");
}

#[tokio::test]
async fn new_files_hold_no_secret_and_keep_their_permissions() {
    let fx = fixture();
    let home = fx.home();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(home.join(".aws/credentials"), std::fs::Permissions::from_mode(0o600)).unwrap();
        std::fs::set_permissions(home.join(".npmrc"), std::fs::Permissions::from_mode(0o640)).unwrap();
    }
    let report = report(&fx);
    let selected = select(&report, &[], true).unwrap();
    let machine = FakeMachine::new(fx.dir.path(), true);
    let (receipt, _) = run(&fx, &report, &selected, false, &FakeWriter::default(), &machine).await.unwrap();
    // Still on disk, and said so: the tools that read only their own file.
    let kept = [".docker/config.json", ".kube/config", ".config/gcloud/application_default_credentials.json", ".netrc"];
    for (path, text) in snapshot(&home) {
        if kept.iter().any(|k| path.ends_with(k)) {
            continue;
        }
        let text = String::from_utf8_lossy(&text).into_owned();
        for secret in &fx.secrets {
            assert!(!leaks(&text, secret), "a value in {}", path.display());
        }
    }
    let on_disk: BTreeSet<&str> = receipt.moved.iter().filter(|m| m.on_disk).map(|m| m.finding.as_str()).collect();
    let kept_ids: BTreeSet<&str> = report
        .findings
        .iter()
        .filter(|f| matches!(f.source, Source::Docker | Source::Kube | Source::Gcp | Source::Netrc))
        .map(|f| f.id.as_str())
        .collect();
    assert_eq!(on_disk, kept_ids);
    assert!(receipt.done.iter().filter(|d| !d.ok).all(|d| d.how.starts_with("still on disk")));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mode = |p: &str| std::fs::metadata(home.join(p)).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(".aws/credentials"), 0o600);
        assert_eq!(mode(".npmrc"), 0o640);
        assert_eq!(mode("dev/acme-web/.env.reins"), 0o644);
        assert_eq!(mode("dev/acme-web/reins.toml"), 0o644);
    }
    // No temporary file is left behind.
    assert!(!snapshot(&home).keys().any(|p| p.to_string_lossy().contains(".reins-")));
}

#[tokio::test]
async fn the_originals_can_be_deleted_from_the_trash_for_good() {
    let fx = fixture();
    let report = report(&fx);
    let selected = select(&report, &[], true).unwrap();
    let machine = FakeMachine::new(fx.dir.path(), true);
    let (receipt, path) = run(&fx, &report, &selected, false, &FakeWriter::default(), &machine).await.unwrap();
    let path = path.unwrap();
    let n = receipt.trashed().len();
    assert!(n > 5);
    let c = ctx(&fx);
    let (_, mut r) = receipt::load(&c.state_dir, "last").unwrap();
    let lines = receipt::purge(&c.state_dir, &path, &mut r, &machine).unwrap();
    assert_eq!(lines.len(), n);
    assert_eq!(machine.purged.lock().unwrap().len(), n);
    assert!(std::fs::read_dir(&machine.trash_dir).unwrap().next().is_none(), "the trash holds none of them");
    let (_, mut r) = receipt::load(&c.state_dir, "last").unwrap();
    assert!(r.purged_at.is_some() && receipt::purge(&c.state_dir, &path, &mut r, &machine).is_err());
}
'''
open(p,'w').write(s)
EOF
cd ../../../.. && export PATH=$HOME/.rustup/toolchains/1.98.1-x86_64-unknown-linux-gnu/bin:$PATH CARGO_TARGET_DIR=$PWD/target && cargo test -p reins-desktop --lib secrets 2>&1 | grep -E "^(error|warning)|^\s+-->|panicked|left|right|test result" | cut -c1-400 | head -30