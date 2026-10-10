//! "Check your phone": a desktop notification, with a short chime, when a request waits for the user's OK on the
//! phone, so a question asked by an agent in a background terminal is not missed (`[notify]` in `config.toml`).
//!
//! Linux: `notify-send` (any notification daemon) and the chime through `pw-play`, `paplay` or `aplay`, whichever is
//! installed. macOS: a notification from `osascript` with the system's Glass sound. Windows: a toast from PowerShell,
//! and the chime through `System.Media.SoundPlayer`. Everything runs detached: the caller never waits for it, and a
//! missing tool only means no notification.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// `[notify]`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct NotifyConfig {
    /// A desktop notification when a request waits for your phone.
    pub phone: bool,
    /// With a chime.
    pub sound: bool,
}

impl Default for NotifyConfig {
    fn default() -> Self {
        Self {
            phone: true,
            sound: true,
        }
    }
}

/// The notification's text: the title, and the body naming what waits.
#[must_use]
pub fn text(what: &str) -> (String, String) {
    let what = what.split_whitespace().collect::<Vec<_>>().join(" ");
    let what: String = if what.chars().count() > 160 {
        let mut s: String = what.chars().take(159).collect();
        s.push('…');
        s
    } else {
        what
    };
    ("Check your phone".to_owned(), format!("Reins 2FA is waiting for your OK: {what}"))
}

/// The file the Reins app keeps fresh while it runs: it then shows "Check your phone" itself, through its own
/// notification permission (on macOS the only way a notification shows up at all), and the asking processes stay quiet.
const CLAIM: &str = "app-notifies";
/// A claim older than this is a Reins app that quit or hangs.
const CLAIM_FRESH: std::time::Duration = std::time::Duration::from_secs(20);

/// The app shows the notifications from now on (call it every few seconds while it runs).
pub fn claim(state_dir: &Path) {
    if let Err(e) = std::fs::write(state_dir.join(CLAIM), std::process::id().to_string()) {
        log::debug!("notification claim: {e}");
    }
}

/// The app no longer shows them (it quits).
pub fn release(state_dir: &Path) {
    std::fs::remove_file(state_dir.join(CLAIM)).ok();
}

/// Why the app's own notifications cannot show, when the system refused them (the user turned them off for Reins, or
/// macOS does not accept this copy of the app: one run from the disk image, or moved aside by Gatekeeper).
static AUTH_PROBLEM: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

/// Looks at a log line of the UI framework for a refused notification authorization (it only logs it).
pub fn observe_log(target: &str, message: &str) {
    if target.starts_with("gpui") && message.contains("notification authorization") {
        *AUTH_PROBLEM.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(message.to_owned());
    }
}

/// The refused notification authorization this app saw, if any.
#[must_use]
pub fn authorization_problem() -> Option<String> {
    AUTH_PROBLEM.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone()
}

/// Whether a running Reins app shows the notifications.
#[must_use]
pub fn app_notifies(state_dir: &Path) -> bool {
    std::fs::metadata(state_dir.join(CLAIM))
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.elapsed().ok())
        .is_some_and(|age| age < CLAIM_FRESH)
}

/// Shows "Check your phone" for `what` (and plays the chime), as `config` says, unless the Reins app shows it.
/// Returns at once.
pub fn phone_waiting(config: &NotifyConfig, state_dir: &Path, what: &str) {
    if !config.phone || app_notifies(state_dir) {
        return;
    }
    let (title, body) = text(what);
    let chime = config.sound.then(|| chime_file(state_dir)).flatten();
    show(&title, &body, config.sound, chime.as_deref());
}

/// Only the chime (the app plays it next to its own notification): the system's Glass sound on macOS, the chime file
/// elsewhere. Returns at once.
pub fn chime(state_dir: &Path) {
    #[cfg(target_os = "macos")]
    {
        let _ = state_dir;
        let mut c = std::process::Command::new("afplay");
        c.arg("/System/Library/Sounds/Glass.aiff");
        detach(c);
    }
    #[cfg(not(target_os = "macos"))]
    if let Some(file) = chime_file(state_dir) {
        play(&file);
    }
}

/// The chime as a WAV file in the state directory (written once).
fn chime_file(state_dir: &Path) -> Option<PathBuf> {
    let path = state_dir.join("chime.wav");
    let wav = chime_wav();
    if std::fs::metadata(&path).map(|m| m.len()).ok() != Some(wav.len() as u64) {
        std::fs::create_dir_all(state_dir).ok()?;
        std::fs::write(&path, &wav).ok()?;
    }
    Some(path)
}

/// Two soft bell notes a fifth apart (E6 then B6), 16-bit mono at 22,050 Hz, about 0.7 s.
#[must_use]
pub fn chime_wav() -> Vec<u8> {
    const RATE: u32 = 22_050;
    let notes: [(f32, f32, f32); 2] = [(1318.5, 0.0, 0.45), (1975.5, 0.14, 0.55)];
    let total = 0.72_f32;
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss, clippy::cast_precision_loss)]
    let samples = (total * RATE as f32) as usize;
    let mut pcm = Vec::with_capacity(samples * 2);
    for i in 0..samples {
        #[allow(clippy::cast_precision_loss)]
        let t = i as f32 / RATE as f32;
        let mut v = 0.0_f32;
        for (freq, start, len) in notes {
            let dt = t - start;
            if dt < 0.0 || dt > len {
                continue;
            }
            // A quick attack, then an exponential fade; a quiet octave above for a bell's shimmer.
            let attack = (dt / 0.008).min(1.0);
            let fade = (-dt * 7.5).exp();
            let phase = std::f32::consts::TAU * freq * dt;
            v += attack * fade * (phase.sin() + 0.18 * (2.0 * phase).sin());
        }
        #[allow(clippy::cast_possible_truncation)]
        let s = (v * 0.28 * f32::from(i16::MAX)).clamp(f32::from(i16::MIN), f32::from(i16::MAX)) as i16;
        pcm.extend_from_slice(&s.to_le_bytes());
    }
    let data_len = u32::try_from(pcm.len()).unwrap_or(0);
    let mut wav = Vec::with_capacity(44 + pcm.len());
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + data_len).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes()); // PCM
    wav.extend_from_slice(&1u16.to_le_bytes()); // mono
    wav.extend_from_slice(&RATE.to_le_bytes());
    wav.extend_from_slice(&(RATE * 2).to_le_bytes());
    wav.extend_from_slice(&2u16.to_le_bytes());
    wav.extend_from_slice(&16u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_len.to_le_bytes());
    wav.extend_from_slice(&pcm);
    wav
}

/// Starts `cmd` without waiting for it (a thread reaps it, so a long-running daemon keeps no zombies).
fn detach(mut cmd: std::process::Command) {
    cmd.stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
    match cmd.spawn() {
        Ok(mut child) => {
            std::thread::spawn(move || {
                let _reaped = child.wait();
            });
        }
        Err(e) => log::debug!("notification: {e}"),
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
fn on_path(program: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|p| std::env::split_paths(&p).any(|d| d.join(program).is_file()))
}

#[cfg(all(unix, not(target_os = "macos")))]
fn show(title: &str, body: &str, sound: bool, chime: Option<&Path>) {
    let mut n = std::process::Command::new("notify-send");
    n.args([
        "--app-name=Reins",
        "--urgency=normal",
        "--icon=dialog-password",
        "--category=im.received",
        "--expire-time=15000",
    ]);
    if sound {
        // Daemons that play sounds themselves (GNOME, KDE) use this one; the chime below covers the rest.
        n.arg("--hint=string:sound-name:message-new-instant");
    }
    n.arg(title).arg(body.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;"));
    detach(n);
    if let Some(file) = chime {
        play(file);
    }
}

/// Plays a WAV file with the first player installed.
#[cfg(all(unix, not(target_os = "macos")))]
fn play(file: &Path) {
    if let Some(player) = ["pw-play", "paplay", "aplay"].into_iter().find(|p| on_path(p)) {
        let mut c = std::process::Command::new(player);
        if player == "aplay" {
            c.arg("-q");
        }
        c.arg(file);
        detach(c);
    }
}

#[cfg(windows)]
fn play(file: &Path) {
    let mut c = std::process::Command::new(crate::win::system32(r"WindowsPowerShell\v1.0\powershell.exe"));
    c.args([
        "-NoProfile",
        "-NonInteractive",
        "-Command",
        "(New-Object System.Media.SoundPlayer $env:REINS_NOTIFY_CHIME).PlaySync()",
    ])
    .env("REINS_NOTIFY_CHIME", file);
    crate::win::hidden(&mut c);
    detach(c);
}

#[cfg(not(any(unix, windows)))]
fn play(_file: &Path) {}

#[cfg(target_os = "macos")]
fn show(title: &str, body: &str, sound: bool, _chime: Option<&Path>) {
    let mut c = std::process::Command::new("osascript");
    // The text goes in as arguments, never into the script.
    let script = if sound {
        "display notification (item 2 of argv) with title (item 1 of argv) sound name \"Glass\""
    } else {
        "display notification (item 2 of argv) with title (item 1 of argv)"
    };
    c.args(["-e", "on run argv", "-e", script, "-e", "end run", title, body]);
    detach(c);
}

/// A toast from PowerShell's own app id (Windows 10 and 11 show it without a registered app), then the chime.
#[cfg(windows)]
const TOAST_SCRIPT: &str = "$ErrorActionPreference = 'SilentlyContinue'; \
    [Windows.UI.Notifications.ToastNotificationManager, Windows.UI.Notifications, ContentType = WindowsRuntime] > $null; \
    [Windows.Data.Xml.Dom.XmlDocument, Windows.Data.Xml.Dom.XmlDocument, ContentType = WindowsRuntime] > $null; \
    $x = New-Object Windows.Data.Xml.Dom.XmlDocument; \
    $x.LoadXml('<toast><visual><binding template=\"ToastGeneric\"><text></text><text></text></binding></visual><audio silent=\"true\"/></toast>'); \
    $t = $x.GetElementsByTagName('text'); \
    $t.Item(0).AppendChild($x.CreateTextNode($env:REINS_NOTIFY_TITLE)) > $null; \
    $t.Item(1).AppendChild($x.CreateTextNode($env:REINS_NOTIFY_TEXT)) > $null; \
    $id = '{1AC14E77-02E7-4E5D-B744-2EB1AE5198B7}\\WindowsPowerShell\\v1.0\\powershell.exe'; \
    [Windows.UI.Notifications.ToastNotificationManager]::CreateToastNotifier($id).Show([Windows.UI.Notifications.ToastNotification]::new($x)); \
    if ($env:REINS_NOTIFY_CHIME) { (New-Object System.Media.SoundPlayer $env:REINS_NOTIFY_CHIME).PlaySync() }";

#[cfg(windows)]
fn show(title: &str, body: &str, _sound: bool, chime: Option<&Path>) {
    let mut c = std::process::Command::new(crate::win::system32(r"WindowsPowerShell\v1.0\powershell.exe"));
    c.args(["-NoProfile", "-NonInteractive", "-Command", TOAST_SCRIPT])
        .env("REINS_NOTIFY_TITLE", title)
        .env("REINS_NOTIFY_TEXT", body);
    if let Some(file) = chime {
        c.env("REINS_NOTIFY_CHIME", file);
    }
    crate::win::hidden(&mut c);
    detach(c);
}

#[cfg(not(any(unix, windows)))]
fn show(_title: &str, _body: &str, _sound: bool, _chime: Option<&Path>) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_text_names_what_waits_on_one_short_line() {
        let (title, body) = text("Claude Code wants to run:\n  git push --force");
        assert_eq!(title, "Check your phone");
        assert_eq!(body, "Reins 2FA is waiting for your OK: Claude Code wants to run: git push --force");
        let (_, long) = text(&"x".repeat(500));
        assert!(long.chars().count() < 220 && long.ends_with('…'));
    }

    #[test]
    fn a_refused_authorization_is_noticed_in_the_frameworks_log_only() {
        observe_log("reins_app", "system notification authorization denied");
        assert_eq!(authorization_problem(), None);
        observe_log("gpui_macos::system_notifications", "system notification authorization denied");
        assert_eq!(authorization_problem().as_deref(), Some("system notification authorization denied"));
    }

    #[test]
    fn a_fresh_claim_means_the_app_shows_the_notification() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!app_notifies(dir.path()));
        claim(dir.path());
        assert!(app_notifies(dir.path()));
        release(dir.path());
        assert!(!app_notifies(dir.path()));
    }

    #[test]
    fn the_chime_is_a_short_valid_wav_written_once() {
        let wav = chime_wav();
        assert_eq!(&wav[..4], b"RIFF");
        assert_eq!(&wav[8..16], b"WAVEfmt ");
        let data = u32::from_le_bytes(wav[40..44].try_into().unwrap()) as usize;
        assert_eq!(wav.len(), 44 + data);
        assert!(wav.len() > 20_000 && wav.len() < 60_000, "{}", wav.len());
        // Not silent, and not clipped at full scale.
        let peak = wav[44..].chunks(2).map(|c| i16::from_le_bytes([c[0], c[1]]).unsigned_abs()).max().unwrap();
        assert!(peak > 3_000 && peak < 32_000, "{peak}");
        let dir = tempfile::tempdir().unwrap();
        let file = chime_file(dir.path()).unwrap();
        assert_eq!(std::fs::read(&file).unwrap(), wav);
    }
}
