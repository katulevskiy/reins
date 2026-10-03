//! Windows only: the windowless copy of the real `rewarden.exe` runs, and the background service (the user's `Run` key,
//! the copy, the daemon started at once, its log) comes and goes with `rewarden resume` and `rewarden service
//! uninstall`. The service test writes the real `HKCU\…\Run\Reins` value, so it only runs when
//! `REWARDEN_TEST_WINDOWS_SERVICE=1` (the CI job sets it; a developer's own installation would be replaced).
#![cfg(windows)]

use std::path::Path;
use std::process::{Command, Output};
use std::time::{Duration, Instant};

use rewarden_desktop::win;

const EXE: &str = env!("CARGO_BIN_EXE_rewarden");

#[test]
fn the_windowless_copy_of_rewarden_still_runs() {
    let bytes = std::fs::read(EXE).unwrap();
    assert_eq!(win::pe_subsystem(&bytes), Some(win::SUBSYSTEM_CONSOLE));
    let gui = win::gui_copy(&bytes).unwrap();
    assert_eq!(win::pe_subsystem(&gui), Some(win::SUBSYSTEM_GUI));
    let dir = tempfile::tempdir().unwrap();
    let copy = dir.path().join("rewarden-daemon.exe");
    std::fs::write(&copy, gui).unwrap();
    // A GUI program has no console, but writes to the pipes it is given.
    let out = Command::new(&copy).arg("--version").output().unwrap();
    assert!(out.status.success(), "{out:?}");
    assert!(String::from_utf8_lossy(&out.stdout).starts_with("rewarden "), "{out:?}");
}

fn text(o: &Output) -> String {
    format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr))
}

#[test]
fn the_background_service_starts_at_once_and_goes_away_again() {
    if std::env::var_os("REWARDEN_TEST_WINDOWS_SERVICE").is_none() {
        eprintln!("skipped: set REWARDEN_TEST_WINDOWS_SERVICE=1 (writes HKCU\\...\\Run\\Reins)");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let config = dir.path().join("config");
    let state = dir.path().join("state");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&config).unwrap();
    let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    std::fs::write(config.join("config.toml"), format!("listen = \"127.0.0.1:{port}\"\nmode = \"local\"\n")).unwrap();
    let run = |args: &[&str]| {
        Command::new(EXE)
            .args(args)
            .env("HOME", &home)
            .env("USERPROFILE", &home)
            .env("GIT_CONFIG_GLOBAL", home.join(".gitconfig"))
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("REWARDEN_CONFIG_DIR", &config)
            .env("REWARDEN_STATE_DIR", &state)
            .output()
            .unwrap()
    };
    let resumed = run(&["resume"]);
    let cleanup = || drop(run(&["service", "uninstall"]));
    if !resumed.status.success() {
        cleanup();
        panic!("resume: {}", text(&resumed));
    }
    assert!(text(&resumed).contains("Started the background service"), "{}", text(&resumed));
    assert!(text(&resumed).contains("through Rewarden"), "{}", text(&resumed));

    let deadline = Instant::now() + Duration::from_secs(20);
    let status = loop {
        let s = text(&run(&["status"]));
        if s.contains("running on 127.0.0.1:") || Instant::now() > deadline {
            break s;
        }
        std::thread::sleep(Duration::from_millis(200));
    };
    let daemon = state.join("rewarden-daemon.exe");
    let checks = || {
        assert!(status.contains("running on 127.0.0.1:"), "{status}");
        assert!(status.contains("Service:       starts at logon"), "{status}");
        assert_eq!(win::pe_subsystem(&std::fs::read(&daemon).unwrap()), Some(win::SUBSYSTEM_GUI));
        let log = std::fs::read_to_string(state.join("daemon.log")).unwrap_or_default();
        assert!(log.contains("listening on"), "{log}");
        let reg = Command::new("reg")
            .args(["query", r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run", "/v", "Reins"])
            .output()
            .unwrap();
        let value = win::reg_value(&String::from_utf8_lossy(&reg.stdout), "Reins").unwrap_or_default();
        assert!(value.to_lowercase().contains(&daemon.display().to_string().to_lowercase()), "{value}");
    };
    if let Err(panic) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(checks)) {
        cleanup();
        std::panic::resume_unwind(panic);
    }

    assert!(text(&run(&["pause"])).contains("Paused"));
    let removed = run(&["service", "uninstall"]);
    assert!(text(&removed).contains("Stopped and removed the service."), "{}", text(&removed));
    let status = text(&run(&["status"]));
    assert!(status.contains("not running"), "{status}");
    assert!(status.contains("Service:       not installed"), "{status}");
    assert!(!Path::new(&state.join("control.token")).exists(), "the daemon stopped cleanly");
    assert!(text(&run(&["service", "uninstall"])).contains("was not installed"));
}
