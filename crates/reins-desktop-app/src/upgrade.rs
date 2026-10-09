//! "Restart to update": a newer Reins in place of this one. The backend has downloaded the installer listed in the
//! signed `app.json` and checked it ([`reins_desktop::update::Updater::fetch_app`]); this puts it where the running
//! copy is and starts the new one once this one has quit (before that, the one-Reins lock would send it away).
//!
//! | system  | replaced         | how                                                                                  |
//! |---------|------------------|--------------------------------------------------------------------------------------|
//! | macOS   | `Reins.app`      | the disk image mounted read-only, the app copied next to the running one (`ditto`), the two swapped by renaming; the old one goes at the next start. Where its folder cannot be written, the disk image is opened for the user instead |
//! | Windows | the installation | the MSI (per user, no administrator) with `msiexec /passive` once this app has quit, then Reins again |
//! | Linux   | the AppImage     | the new one written next to it and renamed over it                                   |
//!
//! Any other copy (a tarball, an AppImage in a folder it cannot write, a build from source) only links to the download
//! page. The new app restarts the background service so that it runs the new `reins` (`Backend::after_update`).

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// This copy of Reins, as an update replaces it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    /// macOS: the running `Reins.app`.
    Bundle(PathBuf),
    /// Windows: the `reins-app.exe` the MSI installed.
    Installed(PathBuf),
    /// Linux: the running AppImage.
    AppImage(PathBuf),
}

impl Target {
    #[must_use]
    pub fn path(&self) -> &Path {
        match self {
            Self::Bundle(p) | Self::Installed(p) | Self::AppImage(p) => p,
        }
    }
}

/// What installing did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Installed {
    /// The new Reins is in place and starts once this one has quit.
    Restarting,
    /// The disk image is open for the user to drag Reins to Applications.
    Opened,
}

/// This copy, when it can install an update itself.
#[must_use]
pub fn target() -> Option<Target> {
    if cfg!(target_os = "macos") {
        let exe = reins_desktop::update::current_executable().ok()?;
        bundle_of(&exe).map(Target::Bundle)
    } else if cfg!(windows) {
        let exe = reins_desktop::update::current_executable().ok()?;
        installed_by_msi(&exe).then_some(Target::Installed(exe))
    } else {
        let appimage = PathBuf::from(std::env::var_os("APPIMAGE").filter(|a| !a.is_empty())?);
        (appimage.is_file() && can_write(appimage.parent()?)).then_some(Target::AppImage(appimage))
    }
}

/// The `.app` bundle whose program `exe` is (`…/Reins.app/Contents/MacOS/reins-app`).
fn bundle_of(exe: &Path) -> Option<PathBuf> {
    let macos = exe.parent()?;
    let contents = macos.parent()?;
    let bundle = contents.parent()?;
    let is_bundle = macos.file_name()? == "MacOS"
        && contents.file_name()? == "Contents"
        && bundle.extension().is_some_and(|e| e.eq_ignore_ascii_case("app"));
    is_bundle.then(|| bundle.to_path_buf())
}

/// Whether the MSI installed `exe` (it records where, in `HKCU\Software\Reins\Desktop`).
#[cfg(windows)]
fn installed_by_msi(exe: &Path) -> bool {
    use winreg::RegKey;
    use winreg::enums::{HKEY_CURRENT_USER, KEY_READ};
    let recorded: Option<String> = RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey_with_flags(r"Software\Reins\Desktop", KEY_READ)
        .and_then(|k| k.get_value("App"))
        .ok();
    recorded.is_some_and(|r| r.eq_ignore_ascii_case(&exe.to_string_lossy()))
}

#[cfg(not(windows))]
fn installed_by_msi(_exe: &Path) -> bool {
    false
}

/// Whether a file can be made in `dir`.
fn can_write(dir: &Path) -> bool {
    let probe = dir.join(format!(".reins-write-test-{}", std::process::id()));
    let made = std::fs::OpenOptions::new().write(true).create_new(true).open(&probe).is_ok();
    if made {
        std::fs::remove_file(&probe).ok();
    }
    made
}

/// Installs `installer` over `target` and, unless it only opened the disk image, arranges for the new Reins to start
/// once this process has ended. Blocks (mounting and copying take a few seconds).
pub fn install(target: &Target, installer: &Path) -> Result<Installed, String> {
    match target {
        Target::Bundle(bundle) => install_bundle(bundle, installer),
        Target::Installed(exe) => {
            install_msi_after_exit(installer, exe)?;
            Ok(Installed::Restarting)
        }
        Target::AppImage(appimage) => {
            replace_appimage(appimage, installer)?;
            start_after_exit(appimage, &[])?;
            Ok(Installed::Restarting)
        }
    }
}

/// Removes what an update left next to `target`: the bundle it replaced, a copy it did not finish (best effort).
pub fn remove_leftovers(target: &Target) {
    for old in leftovers(target.path()) {
        if old.is_dir() {
            std::fs::remove_dir_all(&old).ok();
        } else {
            std::fs::remove_file(&old).ok();
        }
    }
}

/// The names [`swap_bundle`] and [`replace_appimage`] use next to `path`.
fn leftovers(path: &Path) -> Vec<PathBuf> {
    let (Some(dir), Some(name)) = (path.parent(), path.file_name().and_then(OsStr::to_str)) else {
        return Vec::new();
    };
    let prefixes = [format!(".{name}.old-"), format!(".{name}.new-")];
    let next = format!("{name}.new");
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter(|e| {
            let file = e.file_name().to_string_lossy().into_owned();
            file == next || prefixes.iter().any(|p| file.starts_with(p.as_str()))
        })
        .map(|e| e.path())
        .collect()
}

fn run(program: &str, args: &[&OsStr]) -> Result<(), String> {
    let out = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("cannot run {program}: {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(format!("{program} failed: {}", String::from_utf8_lossy(&out.stderr).trim()))
    }
}

/// macOS: the app from the disk image `dmg` in place of `bundle`, or the disk image opened when its folder cannot be
/// written (an Applications folder of another user's).
fn install_bundle(bundle: &Path, dmg: &Path) -> Result<Installed, String> {
    let dir = bundle.parent().ok_or("the app has no folder")?;
    if !can_write(dir) {
        run("open", &[dmg.as_os_str()])?;
        return Ok(Installed::Opened);
    }
    let mount = std::env::temp_dir().join(format!("reins-update-{}", std::process::id()));
    std::fs::create_dir_all(&mount).map_err(|e| format!("{}: {e}", mount.display()))?;
    let attach = [
        OsStr::new("attach"),
        OsStr::new("-nobrowse"),
        OsStr::new("-readonly"),
        OsStr::new("-noautoopen"),
        OsStr::new("-mountpoint"),
        mount.as_os_str(),
        dmg.as_os_str(),
    ];
    run("hdiutil", &attach)?;
    let staged = stage_bundle(&mount, bundle);
    if run("hdiutil", &[OsStr::new("detach"), mount.as_os_str(), OsStr::new("-quiet")]).is_err() {
        run("hdiutil", &[OsStr::new("detach"), mount.as_os_str(), OsStr::new("-force")]).ok();
    }
    std::fs::remove_dir(&mount).ok();
    swap_bundle(bundle, &staged?)?;
    start_after_exit(Path::new("/usr/bin/open"), &[OsStr::new("-n"), bundle.as_os_str()])?;
    Ok(Installed::Restarting)
}

/// Copies the app on the mounted disk image next to `bundle` (`.Reins.app.new-<pid>`). Returns the copy.
fn stage_bundle(mount: &Path, bundle: &Path) -> Result<PathBuf, String> {
    let source = std::fs::read_dir(mount)
        .map_err(|e| format!("{}: {e}", mount.display()))?
        .flatten()
        .map(|e| e.path())
        .find(|p| p.extension().is_some_and(|e| e == "app") && p.join("Contents/MacOS/reins-app").is_file())
        .ok_or("the update has no Reins app in it")?;
    let dir = bundle.parent().ok_or("the app has no folder")?;
    let name = bundle.file_name().and_then(OsStr::to_str).ok_or("the app has no name")?;
    let staged = dir.join(format!(".{name}.new-{}", std::process::id()));
    std::fs::remove_dir_all(&staged).ok();
    // ditto keeps the code signature, extended attributes and links.
    if let Err(e) = run("ditto", &[source.as_os_str(), staged.as_os_str()]) {
        std::fs::remove_dir_all(&staged).ok();
        return Err(e);
    }
    Ok(staged)
}

/// Puts `staged` where `bundle` is: the old bundle is renamed aside (`.Reins.app.old-<pid>`, removed at the next
/// start), the new one renamed into its place. Should that fail, the old one is put back.
fn swap_bundle(bundle: &Path, staged: &Path) -> Result<(), String> {
    let dir = bundle.parent().ok_or("the app has no folder")?;
    let name = bundle.file_name().and_then(OsStr::to_str).ok_or("the app has no name")?;
    let old = dir.join(format!(".{name}.old-{}", std::process::id()));
    // One an earlier update left (removed at the next start, which has not come yet) would be in the way.
    std::fs::remove_dir_all(&old).ok();
    std::fs::rename(bundle, &old).map_err(|e| format!("cannot move {} aside: {e}", bundle.display()))?;
    if let Err(e) = std::fs::rename(staged, bundle) {
        std::fs::rename(&old, bundle).ok();
        std::fs::remove_dir_all(staged).ok();
        return Err(format!("cannot replace {}: {e}", bundle.display()));
    }
    Ok(())
}

/// Linux: `new` in place of the AppImage `appimage`: copied next to it (`<name>.new`), made executable, renamed over
/// it. The running one keeps its mount of the old file.
fn replace_appimage(appimage: &Path, new: &Path) -> Result<(), String> {
    let name = appimage.file_name().and_then(OsStr::to_str).ok_or("the AppImage has no name")?;
    let next = appimage.with_file_name(format!("{name}.new"));
    let put = || -> std::io::Result<()> {
        std::fs::copy(new, &next)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::set_permissions(&next, std::fs::Permissions::from_mode(0o755))?;
        }
        std::fs::File::open(&next)?.sync_all()?;
        std::fs::rename(&next, appimage)
    };
    put().map_err(|e| {
        std::fs::remove_file(&next).ok();
        format!("cannot replace {}: {e}", appimage.display())
    })
}

/// Waits for the process `$0` to end (a minute at most), then runs the rest of the arguments.
const START_AFTER_EXIT: &str =
    r#"i=0; while kill -0 "$0" 2>/dev/null && [ "$i" -lt 300 ]; do sleep 0.2; i=$((i + 1)); done; exec "$@""#;

/// The command that runs `program` with `args` once the process `pid` has ended.
fn after_exit(pid: u32, program: &Path, args: &[&OsStr]) -> Command {
    let mut cmd = Command::new("/bin/sh");
    cmd.arg("-c")
        .arg(START_AFTER_EXIT)
        .arg(pid.to_string())
        .arg(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(&mut cmd, 0);
    cmd
}

/// Starts `program` once this process has ended (macOS and Linux).
fn start_after_exit(program: &Path, args: &[&OsStr]) -> Result<(), String> {
    after_exit(std::process::id(), program, args)
        .spawn()
        .map(drop)
        .map_err(|e| format!("cannot start {} again: {e}", program.display()))
}

/// What installs the MSI once the app has quit and starts the installed app again, installed or not (Windows). The
/// process to wait for, msiexec, its arguments and the app come from the environment, never from the script.
#[cfg_attr(not(windows), allow(dead_code))]
pub const MSI_SCRIPT: &str = "Wait-Process -Id $env:REINS_UPDATE_PID -Timeout 60 -ErrorAction SilentlyContinue; \
     Start-Process -FilePath $env:REINS_UPDATE_MSIEXEC -ArgumentList $env:REINS_UPDATE_ARGS -Wait; \
     Start-Process -FilePath $env:REINS_UPDATE_APP";

/// msiexec's arguments: install `msi` with a progress bar only, and never restart Windows.
#[cfg_attr(not(windows), allow(dead_code))]
fn msi_args(msi: &Path) -> String {
    format!("/i {} /passive /norestart", reins_desktop::win::command_line_arg(&msi.to_string_lossy()))
}

#[cfg(windows)]
fn install_msi_after_exit(msi: &Path, app: &Path) -> Result<(), String> {
    use std::os::windows::process::CommandExt as _;

    use reins_desktop::win;
    let mut cmd = Command::new(win::system32(r"WindowsPowerShell\v1.0\powershell.exe"));
    cmd.args(["-NoProfile", "-NonInteractive", "-Command", MSI_SCRIPT])
        .env("REINS_UPDATE_PID", std::process::id().to_string())
        .env("REINS_UPDATE_MSIEXEC", win::system32("msiexec.exe"))
        .env("REINS_UPDATE_ARGS", msi_args(msi))
        .env("REINS_UPDATE_APP", app)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let flags = win::CREATE_NO_WINDOW | win::CREATE_NEW_PROCESS_GROUP;
    if cmd.creation_flags(flags | win::CREATE_BREAKAWAY_FROM_JOB).spawn().is_ok() {
        return Ok(());
    }
    cmd.creation_flags(flags).spawn().map(drop).map_err(|e| format!("cannot start the installer: {e}"))
}

#[cfg(not(windows))]
fn install_msi_after_exit(_msi: &Path, _app: &Path) -> Result<(), String> {
    Err("an MSI installs on Windows only".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_program_inside_a_bundle_has_one() {
        assert_eq!(
            bundle_of(Path::new("/Applications/Reins.app/Contents/MacOS/reins-app")),
            Some(PathBuf::from("/Applications/Reins.app"))
        );
        assert_eq!(
            bundle_of(Path::new("/Users/me/Apps/Reins 2.app/Contents/MacOS/reins-app")),
            Some(PathBuf::from("/Users/me/Apps/Reins 2.app"))
        );
        assert_eq!(bundle_of(Path::new("/usr/local/bin/reins-app")), None);
        assert_eq!(bundle_of(Path::new("/opt/Reins/Contents/MacOS/reins-app")), None, "not an .app");
        assert_eq!(bundle_of(Path::new("/x/Reins.app/Contents/Resources/reins-app")), None);
    }

    #[test]
    fn the_bundle_is_swapped_whole_and_put_back_when_that_fails() {
        let dir = tempfile::tempdir().unwrap();
        let bundle = dir.path().join("Reins.app");
        std::fs::create_dir_all(bundle.join("Contents/MacOS")).unwrap();
        std::fs::write(bundle.join("Contents/MacOS/reins-app"), b"old").unwrap();
        let staged = dir.path().join(format!(".Reins.app.new-{}", std::process::id()));
        std::fs::create_dir_all(staged.join("Contents/MacOS")).unwrap();
        std::fs::write(staged.join("Contents/MacOS/reins-app"), b"new").unwrap();
        swap_bundle(&bundle, &staged).unwrap();
        assert_eq!(std::fs::read(bundle.join("Contents/MacOS/reins-app")).unwrap(), b"new");
        assert!(!staged.exists());
        let left = leftovers(&bundle);
        assert_eq!(left.len(), 1, "{left:?}");
        assert!(left[0].file_name().unwrap().to_string_lossy().starts_with(".Reins.app.old-"));

        // Nothing staged: the running bundle stays where it is.
        let err = swap_bundle(&bundle, &dir.path().join("missing")).unwrap_err();
        assert!(err.contains("cannot replace"), "{err}");
        assert_eq!(std::fs::read(bundle.join("Contents/MacOS/reins-app")).unwrap(), b"new");

        std::fs::write(dir.path().join("Notes.app.old"), b"someone else's").unwrap();
        remove_leftovers(&Target::Bundle(bundle));
        let mut names: Vec<String> =
            std::fs::read_dir(dir.path()).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into()).collect();
        names.sort();
        assert_eq!(names, ["Notes.app.old", "Reins.app"]);
    }

    #[cfg(unix)]
    #[test]
    fn the_appimage_is_replaced_and_stays_executable() {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = tempfile::tempdir().unwrap();
        let appimage = dir.path().join("Reins.AppImage");
        std::fs::write(&appimage, b"old").unwrap();
        let cached = dir.path().join("cache.AppImage");
        std::fs::write(&cached, b"new").unwrap();
        std::fs::set_permissions(&cached, std::fs::Permissions::from_mode(0o600)).unwrap();
        replace_appimage(&appimage, &cached).unwrap();
        assert_eq!(std::fs::read(&appimage).unwrap(), b"new");
        assert_eq!(std::fs::metadata(&appimage).unwrap().permissions().mode() & 0o777, 0o755);
        assert!(cached.exists(), "the cached copy is kept");
        assert!(leftovers(&appimage).is_empty());
        assert!(can_write(dir.path()));

        let err = replace_appimage(&appimage, &dir.path().join("missing")).unwrap_err();
        assert!(err.contains("cannot replace"), "{err}");
        assert_eq!(std::fs::read(&appimage).unwrap(), b"new");
        assert!(leftovers(&appimage).is_empty(), "no half-written copy");
    }

    #[cfg(unix)]
    #[test]
    fn the_new_copy_starts_only_after_the_old_one_ended() {
        let dir = tempfile::tempdir().unwrap();
        let mark = dir.path().join("started");
        let mut old = Command::new("sleep").arg("0.5").spawn().unwrap();
        let script = format!("echo started > '{}'", mark.display());
        let mut waiter =
            after_exit(old.id(), Path::new("/bin/sh"), &[OsStr::new("-c"), OsStr::new(&script)]).spawn().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(200));
        assert!(!mark.exists(), "started while the old one still ran");
        old.wait().unwrap();
        assert!(waiter.wait().unwrap().success());
        assert!(mark.exists());
    }

    #[test]
    fn the_msi_runs_quietly_and_the_script_takes_everything_from_the_environment() {
        let args = msi_args(Path::new(r"C:\Users\Jo Doe\AppData\Local\reins\updates\Reins-1-Windows-x64.msi"));
        assert_eq!(
            args,
            r#"/i "C:\Users\Jo Doe\AppData\Local\reins\updates\Reins-1-Windows-x64.msi" /passive /norestart"#
        );
        for var in
            ["$env:REINS_UPDATE_PID", "$env:REINS_UPDATE_MSIEXEC", "$env:REINS_UPDATE_ARGS", "$env:REINS_UPDATE_APP"]
        {
            assert!(MSI_SCRIPT.contains(var), "{var}");
        }
        let wait = MSI_SCRIPT.find("Wait-Process").unwrap();
        let msi = MSI_SCRIPT.find("-Wait;").unwrap();
        let app = MSI_SCRIPT.rfind("Start-Process").unwrap();
        assert!(wait < msi && msi < app, "quit, then install, then start");
    }
}
