//! "Check your phone": a desktop notification, with a short chime, when a request waits for the user's OK on the
//! phone, so a question asked by an agent in a background terminal is not missed (`[notify]` in `config.toml`).
//!
//! Linux: `notify-send` (any notification daemon) and the chime through `pw-play`, `paplay` or `aplay`, whichever is
//! installed. macOS: a notification from `osascript` with the system's Glass sound. Windows: a toast from PowerShell,
//! and the chime through `System.Media.SoundPlayer`. Everything runs detached: the caller never waits for it, and a
//! missing tool only means no notification.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use serde::{Deserialize, Serialize};

/// How long a request waits for the phone before "Check your phone" shows, by default: what a mode (Bypass, Auto,
/// Overnight, Lockdown) or a standing permission on the phone decides in that time shows nothing here.
pub const DEFAULT_DELAY_MS: u64 = 3_000;
/// The longest that wait may be.
pub const MAX_DELAY_MS: u64 = 15_000;

/// `[notify]`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct NotifyConfig {
    /// A desktop notification when a request waits for your phone.
    pub phone: bool,
    /// With a chime.
    pub sound: bool,
    /// Milliseconds a request waits before the notification shows (at most [`MAX_DELAY_MS`]): one the phone decides
    /// by itself meanwhile needs nobody, and shows nothing.
    pub delay_ms: u64,
}

impl Default for NotifyConfig {
    fn default() -> Self {
        Self {
            phone: true,
            sound: true,
            delay_ms: DEFAULT_DELAY_MS,
        }
    }
}

impl NotifyConfig {
    /// [`Self::delay_ms`], at most [`MAX_DELAY_MS`].
    #[must_use]
    pub fn delay(&self) -> Duration {
        Duration::from_millis(self.delay_ms.min(MAX_DELAY_MS))
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.delay_ms > MAX_DELAY_MS {
            return Err(format!("`notify.delay_ms` must be at most {MAX_DELAY_MS}"));
        }
        Ok(())
    }
}

/// The notification's text (`Desk-Extras.png`): it only says where to look, "Check your phone", over the request in
/// one sentence ("Claude Code wants to push 3 commits to acme/web"), on one short line.
#[must_use]
pub fn text(sentence: &str) -> (String, String) {
    let sentence = sentence.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut chars = sentence.chars();
    let sentence: String = chars.next().map(|c| c.to_uppercase().chain(chars).collect()).unwrap_or_default();
    let body: String = if sentence.chars().count() > 160 {
        let mut s: String = sentence.chars().take(159).collect();
        s.push('…');
        s
    } else {
        sentence
    };
    ("Check your phone".to_owned(), body)
}

/// The file the Reins app keeps fresh while it runs: it then shows "Check your phone" itself, through its own
/// notification permission (on macOS the only way a notification shows up at all), and the asking processes stay quiet.
const CLAIM: &str = "app-notifies";
/// A claim older than this is a Reins app that quit or hangs.
const CLAIM_FRESH: Duration = Duration::from_secs(20);

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
static AUTH_PROBLEM: Mutex<Option<String>> = Mutex::new(None);

/// Looks at a log line of the UI framework for a refused notification authorization (it only logs it).
pub fn observe_log(target: &str, message: &str) {
    if target.starts_with("gpui") && message.contains("notification authorization") {
        *AUTH_PROBLEM.lock().unwrap_or_else(PoisonError::into_inner) = Some(message.to_owned());
    }
}

/// The refused notification authorization this app saw, if any.
#[must_use]
pub fn authorization_problem() -> Option<String> {
    AUTH_PROBLEM.lock().unwrap_or_else(PoisonError::into_inner).clone()
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
/// Returns at once: `None` when nothing was shown, else the notification, to [`Shown::withdraw`] once answered.
#[must_use]
pub fn phone_waiting(config: &NotifyConfig, state_dir: &Path, what: &str) -> Option<Shown> {
    if !config.phone || app_notifies(state_dir) {
        return None;
    }
    let (title, body) = text(what);
    let chime = config.sound.then(|| chime_file(state_dir)).flatten();
    Some(show(&title, &body, config.sound, chime.as_deref()))
}

/// A notification this process showed. Where the platform lets a notification be taken down (a freedesktop
/// notification daemon, through `notify-send --print-id`), [`Shown::withdraw`] does; elsewhere it stays until it
/// expires or is dismissed.
#[derive(Clone, Debug, Default)]
pub struct Shown(Arc<Mutex<Handle>>);

#[derive(Debug, Default)]
struct Handle {
    /// The daemon's id for it, once `notify-send` printed it.
    id: Option<u32>,
    /// Taken down already (before its id was known: it goes as soon as it is).
    withdrawn: bool,
}

impl Shown {
    /// Takes the notification down where the platform allows; returns at once.
    pub fn withdraw(&self) {
        let id = {
            let mut h = self.0.lock().unwrap_or_else(PoisonError::into_inner);
            h.withdrawn = true;
            h.id.take()
        };
        if let Some(id) = id {
            close(id);
        }
    }

    /// The daemon gave the notification `id` (closed at once when it was withdrawn meanwhile).
    #[cfg_attr(not(all(unix, not(target_os = "macos"))), allow(dead_code))]
    fn got_id(&self, id: u32) {
        let withdrawn = {
            let mut h = self.0.lock().unwrap_or_else(PoisonError::into_inner);
            if !h.withdrawn {
                h.id = Some(id);
            }
            h.withdrawn
        };
        if withdrawn {
            close(id);
        }
    }

    #[cfg_attr(not(all(unix, not(target_os = "macos"))), allow(dead_code))]
    fn withdrawn(&self) -> bool {
        self.0.lock().unwrap_or_else(PoisonError::into_inner).withdrawn
    }
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

/// Plays `wav` (a sound the caller brings, as the phone apps' chimes) without waiting: written once to the state
/// directory as `name`, then played as [`chime`] plays its own (`afplay` on macOS).
pub fn play_wav(state_dir: &Path, name: &str, wav: &[u8]) {
    let Some(file) = sound_file(state_dir, name, wav) else {
        return;
    };
    #[cfg(target_os = "macos")]
    {
        let mut c = std::process::Command::new("afplay");
        c.arg(&file);
        detach(c);
    }
    #[cfg(not(target_os = "macos"))]
    play(&file);
}

/// The chime as a WAV file in the state directory (written once).
fn chime_file(state_dir: &Path) -> Option<PathBuf> {
    sound_file(state_dir, "chime.wav", &chime_wav())
}

/// `wav` as the file `name` in the state directory, written when it is not there yet (or differs in length).
fn sound_file(state_dir: &Path, name: &str, wav: &[u8]) -> Option<PathBuf> {
    // A plain file name: nothing outside the state directory.
    if name.contains(['/', '\\']) || name.starts_with('.') {
        return None;
    }
    let path = state_dir.join(name);
    if std::fs::metadata(&path).map(|m| m.len()).ok() != Some(wav.len() as u64) {
        std::fs::create_dir_all(state_dir).ok()?;
        std::fs::write(&path, wav).ok()?;
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
fn show(title: &str, body: &str, sound: bool, chime: Option<&Path>) -> Shown {
    let notify_send = |print_id: bool| {
        let mut n = std::process::Command::new("notify-send");
        n.args([
            "--app-name=Reins",
            "--urgency=normal",
            "--icon=dialog-password",
            "--category=im.received",
            "--expire-time=15000",
        ]);
        if print_id {
            n.arg("--print-id");
        }
        if sound {
            // Daemons that play sounds themselves (GNOME, KDE) use this one; the chime below covers the rest.
            n.arg("--hint=string:sound-name:message-new-instant");
        }
        n.arg(title).arg(body.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;"));
        n
    };
    let shown = Shown::default();
    let mut n = notify_send(true);
    n.stdin(std::process::Stdio::null()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::null());
    match n.spawn() {
        Ok(child) => {
            let shown = shown.clone();
            let again = notify_send(false);
            // Reads the id it prints (and reaps it): a long-running daemon keeps no zombies.
            std::thread::spawn(move || match child.wait_with_output() {
                Ok(out) if out.status.success() => {
                    if let Some(id) = notification_id(&out.stdout) {
                        shown.got_id(id);
                    }
                }
                // A notify-send before `--print-id` (libnotify 0.7.9): shown without, so it cannot be taken down.
                Ok(_) if !shown.withdrawn() => detach(again),
                Ok(_) => {}
                Err(e) => log::debug!("notification: {e}"),
            });
        }
        Err(e) => log::debug!("notification: {e}"),
    }
    if let Some(file) = chime {
        play(file);
    }
    shown
}

/// The id `notify-send --print-id` printed.
#[cfg_attr(not(all(unix, not(target_os = "macos"))), allow(dead_code))]
fn notification_id(stdout: &[u8]) -> Option<u32> {
    std::str::from_utf8(stdout).ok()?.lines().next()?.trim().parse().ok().filter(|id| *id != 0)
}

/// Asks the notification daemon to take notification `id` down (`CloseNotification`), through `gdbus` (GLib, which
/// `notify-send` comes with) or systemd's `busctl`.
#[cfg(all(unix, not(target_os = "macos")))]
fn close(id: u32) {
    const DEST: &str = "org.freedesktop.Notifications";
    const OBJECT: &str = "/org/freedesktop/Notifications";
    let id = id.to_string();
    if on_path("gdbus") {
        let mut c = std::process::Command::new("gdbus");
        c.args(["call", "--session", "--dest", DEST, "--object-path", OBJECT, "--method"])
            .arg(format!("{DEST}.CloseNotification"))
            .arg(id);
        detach(c);
    } else if on_path("busctl") {
        let mut c = std::process::Command::new("busctl");
        c.args(["--user", "call", DEST, OBJECT, DEST, "CloseNotification", "u"]).arg(id);
        detach(c);
    }
}

/// Elsewhere a notification shown from another program cannot be taken down: it stays until it expires.
#[cfg(not(all(unix, not(target_os = "macos"))))]
fn close(_id: u32) {}

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
fn show(title: &str, body: &str, sound: bool, _chime: Option<&Path>) -> Shown {
    let mut c = std::process::Command::new("osascript");
    // The text goes in as arguments, never into the script.
    let script = if sound {
        "display notification (item 2 of argv) with title (item 1 of argv) sound name \"Glass\""
    } else {
        "display notification (item 2 of argv) with title (item 1 of argv)"
    };
    c.args(["-e", "on run argv", "-e", script, "-e", "end run", title, body]);
    detach(c);
    Shown::default()
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
fn show(title: &str, body: &str, _sound: bool, chime: Option<&Path>) -> Shown {
    let mut c = std::process::Command::new(crate::win::system32(r"WindowsPowerShell\v1.0\powershell.exe"));
    c.args(["-NoProfile", "-NonInteractive", "-Command", TOAST_SCRIPT])
        .env("REINS_NOTIFY_TITLE", title)
        .env("REINS_NOTIFY_TEXT", body);
    if let Some(file) = chime {
        c.env("REINS_NOTIFY_CHIME", file);
    }
    crate::win::hidden(&mut c);
    detach(c);
    Shown::default()
}

#[cfg(not(any(unix, windows)))]
fn show(_title: &str, _body: &str, _sound: bool, _chime: Option<&Path>) -> Shown {
    Shown::default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_text_names_what_waits_on_one_short_line() {
        let (title, body) = text("Claude Code wants to push\n  3 commits to acme/web");
        assert_eq!(title, "Check your phone");
        assert_eq!(body, "Claude Code wants to push 3 commits to acme/web");
        assert_eq!(text("push to acme/web (main)").1, "Push to acme/web (main)");
        assert_eq!(text("").1, "");
        let (_, long) = text(&"x".repeat(500));
        assert!(long.chars().count() < 220 && long.ends_with('…'));
    }

    #[test]
    fn the_delay_defaults_to_three_seconds_and_stays_short() {
        let config: NotifyConfig = toml::from_str("phone = true").unwrap();
        assert_eq!(config.delay(), Duration::from_secs(3));
        let long = NotifyConfig {
            delay_ms: 60_000,
            ..NotifyConfig::default()
        };
        assert_eq!(long.delay(), Duration::from_secs(15));
        assert!(long.validate().is_err());
        assert!(NotifyConfig::default().validate().is_ok());
    }

    #[test]
    fn the_printed_id_is_read_and_a_withdrawal_before_it_is_kept() {
        assert_eq!(notification_id(b"42\n"), Some(42));
        assert_eq!(notification_id(b"0\n"), None);
        assert_eq!(notification_id(b""), None);
        assert_eq!(notification_id(b"nope"), None);
        // Withdrawn before notify-send printed its id: remembered, so the id is closed as soon as it comes.
        let shown = Shown::default();
        assert!(!shown.withdrawn());
        shown.withdraw();
        assert!(shown.withdrawn());
        assert_eq!(shown.0.lock().unwrap().id, None);
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

    /// A sound the caller brings is written once, by a plain name, in the state directory only.
    #[test]
    fn brought_sounds_stay_in_the_state_directory() {
        let dir = tempfile::tempdir().unwrap();
        let file = sound_file(dir.path(), "chime-done.wav", b"RIFF1234").unwrap();
        assert_eq!(file, dir.path().join("chime-done.wav"));
        assert_eq!(std::fs::read(&file).unwrap(), b"RIFF1234");
        for bad in ["../x.wav", "a/b.wav", ".hidden", "a\\b.wav"] {
            assert!(sound_file(dir.path(), bad, b"x").is_none(), "{bad}");
        }
    }
}
