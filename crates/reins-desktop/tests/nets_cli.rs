//! `reins net`: a netted `rm` through the built program, then `reins net list` and `reins net restore`.
#![cfg(unix)]

use std::path::Path;
use std::process::{Command, Output};

fn reins(dir: &Path, cwd: &Path, args: &[&str]) -> Output {
    let home = dir.join("home");
    Command::new(env!("CARGO_BIN_EXE_reins"))
        .args(args)
        .current_dir(cwd)
        .envs(reins_desktop::config::home_env(&home))
        .env("REINS_CONFIG_DIR", dir.join("config"))
        .env("REINS_STATE_DIR", dir.join("state"))
        .env("REINS_NETS_DIR", dir.join("home/nets"))
        .output()
        .unwrap()
}

fn text(o: &Output) -> String {
    format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr))
}

#[test]
fn net_run_rm_then_restore() {
    let dir = tempfile::tempdir().unwrap();
    let work = dir.path().join("home/work");
    std::fs::create_dir_all(work.join("proj/src")).unwrap();
    std::fs::write(work.join("proj/src/main.rs"), "fn main() {}").unwrap();
    let out = reins(dir.path(), &work, &["net", "run", "--plan", "plan01", "--", "rm", "-rf", "proj"]);
    assert!(out.status.success(), "{}", text(&out));
    assert!(!work.join("proj").exists());
    let out = reins(dir.path(), &work, &["net", "list"]);
    assert!(text(&out).contains("plan01"), "{}", text(&out));
    let out = reins(dir.path(), &work, &["net", "restore", "plan01"]);
    assert!(out.status.success(), "{}", text(&out));
    assert_eq!(std::fs::read_to_string(work.join("proj/src/main.rs")).unwrap(), "fn main() {}");
    // Only rm and git; a bad plan id runs nothing.
    let out = reins(dir.path(), &work, &["net", "run", "--plan", "plan01", "--", "sh", "-c", "true"]);
    assert_eq!(out.status.code(), Some(125));
    let out = reins(dir.path(), &work, &["net", "run", "--plan", "../x", "--", "rm", "-rf", "proj"]);
    assert_eq!(out.status.code(), Some(125));
    assert!(work.join("proj").exists());
}
