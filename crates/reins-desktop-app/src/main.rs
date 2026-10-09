//! Reins: the desktop app for people who never open a terminal. It pairs this computer with the phone (a QR code the
//! phone scans), adds Reins to every AI harness it finds, keeps the background service (`reins daemon`) running
//! and sends git through it, and sits in the tray or menu bar afterwards.
//!
//! Everything goes through the `reins_desktop` library in this process, or through the running daemon's control API
//! (`reins_desktop::control`); the app never runs the `reins` command for its normal work. The bundled `reins`
//! program is what the harnesses and the background service run.
#![cfg_attr(windows, windows_subsystem = "windows")]

mod autostart;
mod backend;
mod demo;
mod format;
mod model;
mod pairing;
mod pause;
mod qr;
mod single;
mod state;
mod theme;
mod tray;
mod ui;
mod upgrade;

use std::sync::Arc;

use gpui::{App, QuitMode};

/// Where the app's links go.
pub mod links {
    /// The Reins server every client uses unless the user picks another one.
    pub const SERVER: &str = "https://app.reins2fa.com";
    /// The phone apps (iPhone and Android).
    pub const PHONE_APP: &str = "https://reins2fa.com/app";
    /// The download page of this computer's build.
    #[must_use]
    pub fn download() -> String {
        let platform = if cfg!(target_os = "macos") {
            "macos"
        } else if cfg!(windows) {
            "windows"
        } else {
            "linux"
        };
        format!("https://reins2fa.com/download/{platform}")
    }
    /// Opens the Activity screen of the phone app (the phone's camera reads it from a QR code).
    pub const PHONE_ACTIVITY: &str = "reins://home";
}

/// How the app was started.
#[derive(Clone, Debug, Default)]
pub struct Args {
    /// Started at login: no window unless something needs the user.
    pub background: bool,
    /// A pretend pairing (a made-up code the "phone" approves after a few seconds) and made-up activity, for
    /// screenshots and trying the app without an account. Nothing is sent anywhere. `REINS_DEMO_SCREEN=status` opens
    /// the status window straight away, `REINS_DEMO_SECTION=<overview|activity|connections|keys|rules|settings>`
    /// at that section.
    pub demo: bool,
}

impl Args {
    fn parse() -> Result<Self, String> {
        let mut args = Self::default();
        for arg in std::env::args().skip(1) {
            match arg.as_str() {
                "--background" => args.background = true,
                "--demo" => args.demo = true,
                // The Windows installer starts the app with it; a normal start.
                "--installed" => {}
                "--version" | "-V" => {
                    println!("Reins {}", reins_desktop::update::LONG_VERSION);
                    std::process::exit(0);
                }
                "--help" | "-h" => {
                    println!(
                        "Reins {}\n\nUsage: reins-app [--background] [--demo]\n\n  --background  started at login: stay in the tray unless something needs you\n  --demo        pretend pairing and activity, for trying the app (nothing is sent)",
                        reins_desktop::update::LONG_VERSION
                    );
                    std::process::exit(0);
                }
                // macOS passes `-psn_…` to apps started from Finder on old systems.
                other if other.starts_with("-psn_") => {}
                other => return Err(format!("unknown argument `{other}` (see --help)")),
            }
        }
        args.demo |= std::env::var_os("REINS_DEMO").is_some_and(|v| !v.is_empty() && v != "0");
        Ok(args)
    }
}

fn main() {
    let args = match Args::parse() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("reins-app: {e}");
            std::process::exit(2);
        }
    };
    let backend = match backend::Backend::from_env() {
        Ok(b) => Arc::new(b),
        Err(e) => {
            eprintln!("reins-app: {e}");
            std::process::exit(1);
        }
    };
    // An app started from the desktop has no terminal: the log goes to `app.log` in the state directory as well.
    if let Err(e) = backend.paths().ensure() {
        eprintln!("reins-app: {e}");
    }
    reins_desktop::daemon::init_logging(Some(&backend.paths().state_dir.join("app.log")));
    // One Reins per user: a second start asks the first one to show its window, then ends.
    let Some(instance) = single::Instance::acquire(&backend.paths().state_dir) else {
        log::info!("Reins is already running; asked it to show its window");
        return;
    };

    let runtime = match tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("reins-app: cannot start: {e}");
            std::process::exit(1);
        }
    };
    let handle = runtime.handle().clone();

    let app = gpui_platform::application().with_quit_mode(QuitMode::Explicit);
    app.on_reopen(model::Model::show_window);
    app.run(move |cx: &mut App| {
        gpui_tokio::init_from_handle(cx, handle);
        theme::register_fonts(cx);
        model::Model::init(backend, args, instance, cx);
    });
    drop(runtime);
}
