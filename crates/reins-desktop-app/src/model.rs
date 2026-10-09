//! The app's state and what it does: which screen (a step of the welcome flow, or the status window and its
//! section) the window shows, the pairing in progress, the first-time setup, the health checks and the test to the
//! phone, the status the window and the tray icon show, pausing, and the settings the window changes. One [`Model`]
//! per app (a GPUI global); the window and the tray are views of it.

use std::sync::Arc;
use std::time::{Duration, Instant};

use futures::StreamExt as _;
use gpui::{
    App, AppContext as _, Bounds, Context, Entity, Global, Pixels, Task, TitlebarOptions, WindowBounds, WindowHandle,
    WindowKind, WindowOptions, point, px, size,
};
use gpui_tokio::Tokio;
use reins_desktop::ask::Answer;
use reins_desktop::doctor::Check;
use reins_desktop::harness::Harness;
use reins_desktop::journal::{Entry, Outcome};
use reins_desktop::settings::GuardList;

use crate::backend::{Backend, DaemonState, Setting, Snapshot, Update};
use crate::demo::Demo;
use crate::health::{self, Fix, Health, Test};
use crate::pairing::{self, Approved, DemoFlow, DeviceCode, DeviceFlow, Poll, ServerFlow};
use crate::pause::{Pause, PauseFor};
use crate::single::Instance;
use crate::state::{Place, Rect, Saved};
use crate::tray::{Action, Look, Shown, Tray};
use crate::welcome::{self, Stage, ToolRow};
use crate::{Args, autostart, ui, upgrade};

/// How often the status is read again.
const REFRESH: Duration = Duration::from_secs(3);
/// How often the update feed is asked.
const UPDATE_CHECK: Duration = Duration::from_hours(6);
/// How long "Approved on …" stays before the setup screen.
const APPROVED_PAUSE: Duration = Duration::from_millis(1600);
/// How long the ticked steps of "Turn on Reins" stay before "Reins is on".
const TURNED_ON_PAUSE: Duration = Duration::from_millis(900);
/// How often the health checks look whether they are due.
const HEALTH_TICK: Duration = Duration::from_secs(5);
/// `--demo`: how long the phone takes to answer the test.
const DEMO_ANSWER: Duration = Duration::from_secs(4);
/// The longest "what" the tray's first line quotes.
const TRAY_WHAT: usize = 60;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Screen {
    /// The welcome flow, at this step.
    Welcome(Stage),
    Status,
}

/// The status window's sections (the sidebar).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Section {
    Overview,
    Activity,
    Connections,
    Keys,
    Rules,
    Settings,
}

impl Section {
    pub const ALL: [Self; 6] =
        [Self::Overview, Self::Activity, Self::Connections, Self::Keys, Self::Rules, Self::Settings];

    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Overview => "Overview",
            Self::Activity => "Activity",
            Self::Connections => "Connections",
            Self::Keys => "Keys & secrets",
            Self::Rules => "Rules",
            Self::Settings => "Settings",
        }
    }

    /// As `REINS_DEMO_SECTION` names it.
    #[must_use]
    pub fn id(self) -> &'static str {
        match self {
            Self::Overview => "overview",
            Self::Activity => "activity",
            Self::Connections => "connections",
            Self::Keys => "keys",
            Self::Rules => "rules",
            Self::Settings => "settings",
        }
    }

    #[must_use]
    pub fn from_id(id: &str) -> Option<Self> {
        let id = id.trim().to_ascii_lowercase();
        Self::ALL.into_iter().find(|s| s.id() == id)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Pairing {
    Starting,
    Code(DeviceCode),
    /// The browser sign-in is open; the phone shows this computer's key.
    Browser,
    Approved {
        phone: Option<String>,
    },
    Failed(String),
    /// This build cannot pair by QR code: the browser sign-in leads.
    Unavailable,
}

/// The tray's first line while requests wait on the phone (newest first): "Waiting on your phone: <what>", and how
/// many more.
#[must_use]
pub fn waiting_line(activity: &[Entry]) -> Option<String> {
    let mut waiting = activity.iter().filter(|e| e.outcome == Outcome::Waiting);
    let first = waiting.next()?;
    let what = first.what.trim();
    let what = if what.chars().count() > TRAY_WHAT {
        format!("{}…", what.chars().take(TRAY_WHAT - 1).collect::<String>().trim_end())
    } else {
        what.to_owned()
    };
    Some(match waiting.count() {
        0 => format!("Waiting on your phone: {what}"),
        more => format!("Waiting on your phone: {what} (+{more} more)"),
    })
}

/// What "Turn on Reins" does besides starting the background service.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TurnOn {
    Git,
    Autostart,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Step {
    Running,
    Done(String),
    Failed(String),
}

#[allow(clippy::struct_excessive_bools, reason = "independent flags the window shows")]
pub struct Model {
    backend: Arc<Backend>,
    pub args: Args,
    instance: Instance,
    runtime: tokio::runtime::Handle,
    pub saved: Saved,
    pub snapshot: Option<Arc<Snapshot>>,
    pub screen: Screen,
    pub section: Section,
    pub pairing: Pairing,
    pub fingerprint: Option<String>,
    /// The AI tools step's rows.
    pub tools: Vec<ToolRow>,
    /// "Turn on Reins" sends git through Reins too.
    pub turn_git: bool,
    /// "Turn on Reins" opens Reins at login too.
    pub turn_autostart: bool,
    pub setup_steps: Vec<(String, Step)>,
    pub setup_running: bool,
    /// The health checks (`reins doctor`).
    pub health: Health,
    /// "Send a test to my phone".
    pub test: Test,
    pub update: Option<Update>,
    /// "Restart to update" is installing it.
    pub updating: bool,
    /// The last thing that went wrong, shown until the next action.
    pub notice: Option<String>,
    /// Something done that the user should know (not an error), shown until the next action.
    pub note: Option<String>,
    /// The QR code that opens Activity on the phone is showing.
    pub show_phone_qr: bool,
    /// "Use another server" is open.
    pub show_server: bool,
    pub server_input: String,
    pub autostart: bool,
    /// `--demo`: the pretend pairing went through.
    demo_paired: bool,
    /// `--demo`: the made-up activity and connections.
    demo: Option<Demo>,
    /// `--demo`: the checks fixed with the health card's buttons.
    demo_fixed: Vec<String>,
    /// The window is open (the health checks run only then).
    window_open: bool,
    /// The window moved or changed size since `app.json` was written.
    place_dirty: bool,
    /// The minute the window last showed (relative times change with it).
    minute: i64,
    pairing_task: Option<Task<()>>,
    window: Option<WindowHandle<ui::Root>>,
    tray: Option<Tray>,
    tasks: Vec<Task<()>>,
}

struct GlobalModel(Entity<Model>);

impl Global for GlobalModel {}

/// Whether the Dock shows Reins: only while its window is open (a menu bar app otherwise).
#[cfg(target_os = "macos")]
fn show_in_dock(visible: bool) {
    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy};
    if let Some(mtm) = MainThreadMarker::new() {
        let policy = if visible {
            NSApplicationActivationPolicy::Regular
        } else {
            NSApplicationActivationPolicy::Accessory
        };
        NSApplication::sharedApplication(mtm).setActivationPolicy(policy);
    }
}

#[cfg(not(target_os = "macos"))]
fn show_in_dock(_visible: bool) {}

impl Model {
    pub fn init(backend: Arc<Backend>, args: Args, instance: Instance, cx: &mut App) {
        let mut saved = Saved::load(&backend.paths().state_dir);
        // `--demo` with `REINS_DEMO_SCREEN`: straight to a screen, as if paired (and set up, for the status window).
        let demo_screen = std::env::var("REINS_DEMO_SCREEN").ok().filter(|_| args.demo).and_then(|s| demo_screen(&s));
        let demo_paired = demo_screen.is_some_and(|s| s != Screen::Welcome(Stage::Pair));
        if demo_paired {
            saved.setup_done = matches!(demo_screen, Some(Screen::Status | Screen::Welcome(Stage::Done)));
            saved.account.get_or_insert_with(|| "dana@acme.dev".to_owned());
            saved.phone.get_or_insert_with(|| "Dana's iPhone".to_owned());
        }
        let section = std::env::var("REINS_DEMO_SECTION")
            .ok()
            .filter(|_| args.demo)
            .and_then(|s| Section::from_id(&s))
            .unwrap_or(Section::Overview);
        let paired = demo_paired || backend.paired_server().is_some();
        let screen = demo_screen.filter(|_| demo_paired).unwrap_or_else(|| {
            Stage::resume(paired, saved.setup_done, saved.setup_step).map_or(Screen::Status, Screen::Welcome)
        });
        let home = backend.home().to_path_buf();
        let runtime = Tokio::handle(cx);
        let demo = args.demo.then(|| {
            let demo = Demo::new(reins_desktop::now_unix());
            if std::env::var_os("REINS_DEMO_EMPTY").is_some_and(|v| !v.is_empty() && v != "0") {
                demo.empty()
            } else {
                demo
            }
        });
        let demo_test = args.demo && std::env::var_os("REINS_DEMO_TEST").is_some_and(|v| !v.is_empty() && v != "0");
        let model = cx.new(|cx| {
            let mut model = Self {
                fingerprint: backend.fingerprint().ok(),
                backend,
                args,
                instance,
                runtime,
                server_input: saved.server.clone().unwrap_or_default(),
                saved,
                snapshot: None,
                screen,
                section,
                pairing: Pairing::Starting,
                tools: Vec::new(),
                turn_git: true,
                turn_autostart: true,
                setup_steps: Vec::new(),
                setup_running: false,
                health: Health::default(),
                test: Test::Idle,
                update: None,
                updating: false,
                notice: None,
                note: None,
                show_phone_qr: false,
                show_server: false,
                autostart: autostart::enabled(&home),
                demo_paired,
                demo,
                demo_fixed: Vec::new(),
                window_open: false,
                place_dirty: false,
                minute: 0,
                pairing_task: None,
                window: None,
                tray: None,
                tasks: Vec::new(),
            };
            model.start_tray(cx);
            model.start_loops(cx);
            match screen {
                Screen::Welcome(Stage::Pair) => model.start_pairing(cx),
                Screen::Welcome(stage) => {
                    model.prepare_setup();
                    if model.args.demo {
                        model.demo_setup(stage, cx);
                    }
                }
                Screen::Status => {}
            }
            if demo_test {
                model.send_test(cx);
            }
            model
        });
        cx.set_global(GlobalModel(model.clone()));
        let background = model.read(cx).args.background;
        if !background || screen != Screen::Status {
            Self::show_window(cx);
        } else {
            show_in_dock(false);
        }
    }

    fn start_tray(&mut self, cx: &mut Context<'_, Self>) {
        let (tx, mut rx) = futures::channel::mpsc::unbounded::<Action>();
        match Tray::new(tx, &self.runtime) {
            Ok(t) => self.tray = Some(t),
            Err(e) => log::warn!("no tray icon: {e}"),
        }
        self.tasks.push(cx.spawn(async move |this, cx| {
            while let Some(action) = rx.next().await {
                let gone = this.update(cx, |model, cx| model.on_tray(action, cx)).is_err();
                if gone {
                    break;
                }
            }
        }));
    }

    fn on_tray(&mut self, action: Action, cx: &mut Context<'_, Self>) {
        match action {
            Action::Status | Action::Open => {
                if self.screen == Screen::Status {
                    self.show_phone_qr = false;
                    if action == Action::Status {
                        self.section = Section::Overview;
                        cx.notify();
                    }
                }
                cx.defer(Self::show_window);
            }
            Action::Pause(length) => self.pause_for(length, cx),
            Action::Resume => self.resume(cx),
            Action::Quit => self.quit(cx),
        }
    }

    /// Quits Reins (the background service keeps running), writing where the window was first.
    pub fn quit(&mut self, cx: &mut Context<'_, Self>) {
        if self.place_dirty {
            self.persist();
        }
        cx.quit();
    }

    fn start_loops(&mut self, cx: &mut Context<'_, Self>) {
        self.tasks.push(cx.spawn(async move |this, cx| {
            loop {
                let Ok(backend) = this.read_with(cx, |m, _| Arc::clone(&m.backend)) else {
                    break;
                };
                let snapshot = Tokio::spawn(cx, async move { backend.snapshot().await }).await;
                let gone = this
                    .update(cx, |model, cx| {
                        if let Ok(s) = snapshot {
                            model.set_snapshot(s, cx);
                        }
                        if model.instance.take_show_request() {
                            cx.defer(Self::show_window);
                        }
                        if model.place_dirty {
                            model.persist();
                        }
                    })
                    .is_err();
                if gone {
                    break;
                }
                cx.background_executor().timer(REFRESH).await;
            }
        }));
        // The health checks: on the status window while it is open, once a minute (and on demand).
        self.tasks.push(cx.spawn(async move |this, cx| {
            loop {
                let Ok(()) = this.update(cx, |m, cx| {
                    if m.screen == Screen::Status && m.window_open && m.health.due(reins_desktop::now_unix()) {
                        m.run_checks(cx);
                    }
                }) else {
                    break;
                };
                cx.background_executor().timer(HEALTH_TICK).await;
            }
        }));
        self.tasks.push(cx.spawn(async move |this, cx| {
            // The first start after "Restart to update": the service runs the new `reins`.
            if let Ok((backend, setup_done)) = this.read_with(cx, |m, _| (Arc::clone(&m.backend), m.saved.setup_done)) {
                match Tokio::spawn(cx, async move { backend.after_update(setup_done).await }).await {
                    Ok(Ok(Some(done))) => {
                        log::info!("{done}");
                        let _gone = this.update(cx, Self::refresh_now);
                    }
                    Ok(Ok(None)) => {}
                    Ok(Err(e)) => log::warn!("after the update: {e}"),
                    Err(e) => log::warn!("after the update: {e}"),
                }
            }
            loop {
                let Ok(backend) = this.read_with(cx, |m, _| Arc::clone(&m.backend)) else {
                    break;
                };
                let found = Tokio::spawn(cx, async move { backend.check_update().await })
                    .await
                    .unwrap_or_else(|e| Err(e.to_string()));
                if this
                    .update(cx, |model, cx| match found {
                        Ok(found) => {
                            model.update = found;
                            cx.notify();
                        }
                        // Shown as it was; the next check tries again.
                        Err(e) => log::info!("update check: {e}"),
                    })
                    .is_err()
                {
                    break;
                }
                cx.background_executor().timer(UPDATE_CHECK).await;
            }
        }));
    }

    fn set_snapshot(&mut self, mut snapshot: Snapshot, cx: &mut Context<'_, Self>) {
        let now = reins_desktop::now_unix();
        // A timed pause runs out: git goes through Reins again. "Until I resume" never does.
        if self.saved.pause_ran_out(now) {
            self.resume(cx);
        }
        if let Some(demo) = self.demo {
            demo.fill(&mut snapshot, self.pause().on());
        }
        // Signed out elsewhere (`reins logout`, or the phone removed the connection).
        if snapshot.server.is_none() && !self.demo_paired && self.screen != Screen::Welcome(Stage::Pair) {
            self.screen = Screen::Welcome(Stage::Pair);
            self.start_pairing(cx);
        }
        if self.snapshot.as_deref() != Some(&snapshot) {
            if self.fingerprint.is_none() {
                self.fingerprint.clone_from(&snapshot.fingerprint);
            }
            self.snapshot = Some(Arc::new(snapshot));
            cx.notify();
        } else if now / 60 != self.minute {
            // "2 min ago" and "Paused for 12 min more" move on.
            cx.notify();
        }
        self.minute = now / 60;
        self.refresh_tray();
    }

    /// Where `config.toml` is.
    #[must_use]
    pub fn config_file(&self) -> std::path::PathBuf {
        self.backend.paths().config_file()
    }

    /// Paired with a server (or, with `--demo`, pretending).
    #[must_use]
    pub fn paired(&self) -> bool {
        self.demo_paired
            || self.snapshot.as_ref().map_or_else(|| self.backend.paired_server().is_some(), |s| s.server.is_some())
    }

    /// The server shown and paired with.
    #[must_use]
    pub fn server(&self) -> String {
        self.snapshot.as_ref().and_then(|s| s.server.clone()).unwrap_or_else(|| Backend::server(&self.saved))
    }

    /// Whether git is paused now, and until when.
    #[must_use]
    pub fn pause(&self) -> Pause {
        Pause::of(&self.saved, reins_desktop::now_unix())
    }

    /// The state in a few words, and the icon for it.
    #[must_use]
    pub fn status_line(&self) -> (Look, String) {
        if !self.paired() {
            return (Look::Attention, "Not connected to your phone".to_owned());
        }
        let paused = self.pause().line(reins_desktop::now_unix());
        let daemon = match self.snapshot.as_ref().map(|s| &s.daemon) {
            Some(DaemonState::Stopped) if self.saved.setup_done => Some("The background service is not running"),
            Some(DaemonState::Unknown(_)) => Some("Cannot reach the background service"),
            _ => None,
        };
        // A request left "waiting" by a service that stopped must not hide that it stopped.
        if paused.is_none()
            && let Some(problem) = daemon
        {
            return (Look::Attention, problem.to_owned());
        }
        if let Some(line) = self.snapshot.as_ref().and_then(|s| waiting_line(&s.activity)) {
            return (Look::Waiting, line);
        }
        if let Some(line) = paused {
            return (Look::Paused, line);
        }
        if self.snapshot.as_ref().is_some_and(|s| s.git_routed.is_empty()) && self.saved.setup_done {
            return (Look::Paused, "Paused: git goes to GitHub directly".to_owned());
        }
        (Look::On, "Reins is on".to_owned())
    }

    fn refresh_tray(&mut self) {
        let (look, status) = self.status_line();
        let paused = look == Look::Paused;
        let shown = Shown {
            look,
            status,
            can_pause: self.paired() && !paused,
            can_resume: self.paired() && paused,
        };
        if let Some(tray) = self.tray.as_mut() {
            tray.show(&shown);
        }
    }

    // The window.

    /// Shows the window (opening it when it is closed) and brings Reins forward.
    pub fn show_window(cx: &mut App) {
        let Some(model) = cx.try_global::<GlobalModel>().map(|g| g.0.clone()) else {
            return;
        };
        show_in_dock(true);
        if let Some(handle) = model.read(cx).window
            && handle.update(cx, |_, window, _| window.activate_window()).is_ok()
        {
            cx.activate(true);
            return;
        }
        let bounds = Self::window_bounds(model.read(cx).saved.window, cx);
        let min = size(px(ui::MIN_WIDTH), px(ui::MIN_HEIGHT));
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: Some(TitlebarOptions {
                title: Some("Reins".into()),
                appears_transparent: cfg!(target_os = "macos"),
                traffic_light_position: Some(point(px(14.0), px(16.0))),
            }),
            kind: WindowKind::Normal,
            is_resizable: true,
            is_minimizable: true,
            app_id: Some("reins".to_owned()),
            window_min_size: Some(min),
            ..WindowOptions::default()
        };
        let window_model = model.clone();
        match cx.open_window(options, |window, cx| cx.new(|cx| ui::Root::new(window_model, window, cx))) {
            Ok(handle) => {
                model.update(cx, |m, _| {
                    m.window = Some(handle);
                    m.window_open = true;
                });
                let id = handle.window_id();
                cx.on_window_closed(move |cx, closed| {
                    if closed == id {
                        show_in_dock(false);
                        model.update(cx, |m, _| {
                            m.window_open = false;
                            if m.place_dirty {
                                m.persist();
                            }
                        });
                    }
                })
                .detach();
            }
            Err(e) => log::warn!("cannot open the window: {e}"),
        }
        cx.activate(true);
    }

    /// Where the window opens: where it was (fitted to the displays there are now), else centred at its first size.
    fn window_bounds(place: Option<Place>, cx: &App) -> Bounds<Pixels> {
        let Some(place) = place else {
            return Bounds::centered(None, size(px(ui::WIDTH), px(ui::HEIGHT)), cx);
        };
        let displays: Vec<Rect> = cx
            .displays()
            .iter()
            .map(|d| {
                let b = d.visible_bounds();
                (b.origin.x.as_f32(), b.origin.y.as_f32(), b.size.width.as_f32(), b.size.height.as_f32())
            })
            .collect();
        let (origin, (w, h)) = place.fit(&displays, (ui::MIN_WIDTH, ui::MIN_HEIGHT));
        match origin {
            Some((x, y)) => Bounds {
                origin: point(px(x), px(y)),
                size: size(px(w), px(h)),
            },
            None => Bounds::centered(None, size(px(w), px(h)), cx),
        }
    }

    /// The window moved or changed size: remembered (written to `app.json` with the next refresh, or on close).
    pub fn note_window(&mut self, bounds: Bounds<Pixels>) {
        let place = Place {
            x: bounds.origin.x.as_f32(),
            y: bounds.origin.y.as_f32(),
            width: bounds.size.width.as_f32(),
            height: bounds.size.height.as_f32(),
        };
        // JSON cannot hold NaN or infinity, and a window this small is one being created or minimised.
        let sane = [place.x, place.y, place.width, place.height].iter().all(|v| v.is_finite())
            && place.width >= 200.0
            && place.height >= 200.0;
        if sane && self.saved.window != Some(place) {
            self.saved.window = Some(place);
            self.place_dirty = true;
        }
    }

    // Pairing.

    fn flow(&self) -> Arc<dyn DeviceFlow> {
        if self.args.demo {
            Arc::new(DemoFlow {
                approve_after: Duration::from_secs(6),
                started: std::sync::Mutex::new(None),
            })
        } else {
            Arc::new(ServerFlow::new(Arc::clone(&self.backend), Backend::server(&self.saved)))
        }
    }

    pub fn start_pairing(&mut self, cx: &mut Context<'_, Self>) {
        self.pairing = Pairing::Starting;
        self.notice = None;
        cx.notify();
        let flow = self.flow();
        self.pairing_task = Some(cx.spawn(async move |this, cx| {
            loop {
                let start_flow = Arc::clone(&flow);
                let started = Tokio::spawn(cx, async move { start_flow.start().await })
                    .await
                    .unwrap_or_else(|e| Err(e.to_string()));
                let code = match started {
                    Ok(code) => code,
                    Err(e) => {
                        let state = if e == pairing::UNAVAILABLE {
                            Pairing::Unavailable
                        } else {
                            Pairing::Failed(e)
                        };
                        let _gone = this.update(cx, |m, cx| m.set_pairing(state, cx));
                        return;
                    }
                };
                if this.update(cx, |m, cx| m.set_pairing(Pairing::Code(code.clone()), cx)).is_err() {
                    return;
                }
                loop {
                    cx.background_executor().timer(code.interval).await;
                    if Instant::now() >= code.expires_at {
                        break;
                    }
                    let poll_flow = Arc::clone(&flow);
                    let polled = Tokio::spawn(cx, async move { poll_flow.poll().await })
                        .await
                        .unwrap_or_else(|e| Err(e.to_string()));
                    match polled {
                        Ok(Poll::Waiting) => {}
                        Ok(Poll::Approved(approved)) => {
                            let _gone = this.update(cx, |m, cx| m.approved(approved, cx));
                            return;
                        }
                        Ok(Poll::Ended(why)) => {
                            let _gone = this.update(cx, |m, cx| m.set_pairing(Pairing::Failed(why), cx));
                            return;
                        }
                        // The network may come back; keep the code until it runs out.
                        Err(e) => log::info!("pairing: {e}"),
                    }
                }
            }
        }));
    }

    fn set_pairing(&mut self, pairing: Pairing, cx: &mut Context<'_, Self>) {
        self.pairing = pairing;
        cx.notify();
    }

    pub fn sign_in_with_browser(&mut self, cx: &mut Context<'_, Self>) {
        self.set_pairing(Pairing::Browser, cx);
        let backend = Arc::clone(&self.backend);
        let server = Backend::server(&self.saved);
        let demo = self.args.demo;
        self.pairing_task = Some(cx.spawn(async move |this, cx| {
            let result = if demo {
                cx.background_executor().timer(Duration::from_secs(3)).await;
                Ok(server)
            } else {
                Tokio::spawn(cx, async move { backend.sign_in_with_browser(&server).await })
                    .await
                    .unwrap_or_else(|e| Err(e.to_string()))
            };
            let _gone = this.update(cx, |m, cx| match result {
                Ok(server) => m.approved(
                    Approved {
                        server,
                        phone: None,
                        account: None,
                    },
                    cx,
                ),
                Err(e) => m.set_pairing(Pairing::Failed(e), cx),
            });
        }));
    }

    fn approved(&mut self, approved: Approved, cx: &mut Context<'_, Self>) {
        log::info!("paired with {}", approved.server);
        self.demo_paired |= self.args.demo;
        self.saved.phone.clone_from(&approved.phone);
        self.saved.account.clone_from(&approved.account);
        self.saved.setup_done = false;
        self.saved.setup_step = Stage::Tools;
        self.persist();
        self.set_pairing(
            Pairing::Approved {
                phone: approved.phone,
            },
            cx,
        );
        cx.show_system_notification(gpui::SystemNotification {
            tag: "reins-paired".into(),
            title: "Reins is connected".into(),
            body: "Your phone approves what your AI agents do from now on.".into(),
            actions: Vec::new(),
        });
        self.pairing_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(APPROVED_PAUSE).await;
            let _gone = this.update(cx, |m, cx| {
                m.screen = Screen::Welcome(Stage::Tools);
                m.prepare_setup();
                cx.notify();
            });
        }));
    }

    pub fn toggle_server(&mut self, cx: &mut Context<'_, Self>) {
        self.show_server = !self.show_server;
        cx.notify();
    }

    /// Pairs with the server typed under "Use another server" (empty: the default one).
    pub fn use_server(&mut self, cx: &mut Context<'_, Self>) {
        let typed = self.server_input.trim();
        if typed.is_empty() {
            self.saved.server = None;
        } else {
            match reins_desktop::server::server_base(typed) {
                Ok(base) => {
                    self.server_input.clone_from(&base);
                    self.saved.server = Some(base);
                }
                Err(e) => {
                    self.notice = Some(e);
                    cx.notify();
                    return;
                }
            }
        }
        self.persist();
        self.show_server = false;
        self.start_pairing(cx);
    }

    // The first-time setup.

    /// The AI tools step's rows, as this computer has them (in the demo, as if three were installed).
    fn prepare_setup(&mut self) {
        let demo = self.args.demo;
        let mut rows: Vec<ToolRow> = self
            .backend
            .harnesses()
            .into_iter()
            .map(|r| {
                let sample = demo && matches!(r.harness, Harness::ClaudeCode | Harness::Codex | Harness::Cursor);
                ToolRow::new(r.harness, r.found || sample, r.complete)
            })
            .collect();
        welcome::sort(&mut rows);
        self.tools = rows;
        self.setup_steps.clear();
        self.setup_running = false;
    }

    /// `--demo` straight at a later step: what the earlier ones would have done.
    fn demo_setup(&mut self, stage: Stage, cx: &mut Context<'_, Self>) {
        let connected = std::env::var("REINS_DEMO_SCREEN").is_ok_and(|v| v.trim() == "tools-connected");
        if (stage > Stage::Tools || connected)
            && let Some(row) = self.tools.iter_mut().find(|r| r.harness == Harness::ClaudeCode)
        {
            row.connected = true;
            row.undoable = connected;
        }
        if stage > Stage::Tools
            && let Some(row) = self.tools.iter_mut().find(|r| r.harness == Harness::Codex)
        {
            row.connected = true;
        }
        if stage == Stage::Done {
            self.autostart = true;
        }
        if std::env::var("REINS_DEMO_SCREEN").is_ok_and(|v| v.trim() == "turning-on") {
            self.run_setup(cx);
        }
        cx.notify();
    }

    /// Moves the welcome flow to `stage`, remembering it.
    fn go_to(&mut self, stage: Stage, cx: &mut Context<'_, Self>) {
        self.screen = Screen::Welcome(stage);
        if matches!(stage, Stage::Tools | Stage::TurnOn) {
            self.saved.setup_step = stage;
            self.persist();
        }
        self.notice = None;
        cx.notify();
    }

    /// Connects one AI tool (adds Reins to its settings) right away.
    pub fn connect_tool(&mut self, h: Harness, cx: &mut Context<'_, Self>) {
        let demo = self.args.demo;
        let Some(row) = self.tools.iter_mut().find(|r| r.harness == h) else {
            return;
        };
        if !row.can_connect() {
            return;
        }
        row.error = None;
        if demo {
            // Nothing changes on the computer in the demo; it takes a moment all the same.
            row.busy = true;
            cx.notify();
            cx.spawn(async move |this, cx| {
                cx.background_executor().timer(Duration::from_millis(450)).await;
                let _gone = this.update(cx, |m, cx| {
                    if let Some(row) = m.tools.iter_mut().find(|r| r.harness == h) {
                        row.busy = false;
                        row.connected = true;
                        row.undoable = true;
                    }
                    cx.notify();
                });
            })
            .detach();
            return;
        }
        match self.backend.set_harness(h, true) {
            Ok(()) => {
                row.connected = true;
                row.undoable = true;
            }
            Err(e) => row.error = Some(e),
        }
        cx.notify();
    }

    /// Undo: takes Reins out of an AI tool connected on this step.
    pub fn undo_tool(&mut self, h: Harness, cx: &mut Context<'_, Self>) {
        let demo = self.args.demo;
        let Some(row) = self.tools.iter_mut().find(|r| r.harness == h && r.connected && !r.busy) else {
            return;
        };
        row.error = None;
        let removed = if demo {
            Ok(())
        } else {
            self.backend.set_harness(h, false)
        };
        match removed {
            Ok(()) => {
                row.connected = false;
                row.undoable = false;
            }
            Err(e) => row.error = Some(e),
        }
        cx.notify();
    }

    pub fn connect_all(&mut self, cx: &mut Context<'_, Self>) {
        for h in welcome::connectable(&self.tools) {
            self.connect_tool(h, cx);
        }
    }

    /// From the AI tools to "Turn on Reins".
    pub fn tools_continue(&mut self, cx: &mut Context<'_, Self>) {
        self.go_to(Stage::Tools.next(), cx);
    }

    /// Back to the AI tools (not while turning on).
    pub fn setup_back(&mut self, cx: &mut Context<'_, Self>) {
        if let Screen::Welcome(stage) = self.screen
            && let Some(back) = stage.back()
            && !self.setup_running
        {
            self.setup_steps.clear();
            self.go_to(back, cx);
        }
    }

    pub fn toggle_turn_on(&mut self, what: TurnOn, cx: &mut Context<'_, Self>) {
        if self.setup_running {
            return;
        }
        match what {
            TurnOn::Git => self.turn_git = !self.turn_git,
            TurnOn::Autostart => self.turn_autostart = !self.turn_autostart,
        }
        cx.notify();
    }

    /// Whether the setup ran and ended (successfully or not).
    #[must_use]
    pub fn setup_finished(&self) -> bool {
        !self.setup_running && !self.setup_steps.is_empty()
    }

    /// "Turn on Reins": starts the background service, sends git through it, opens Reins at login; each step shows
    /// as it goes. When all went well, "Reins is on".
    pub fn run_setup(&mut self, cx: &mut Context<'_, Self>) {
        if self.setup_running {
            return;
        }
        self.setup_running = true;
        self.setup_steps.clear();
        self.notice = None;
        cx.notify();
        let (git, login) = (self.turn_git, self.turn_autostart);
        let backend = Arc::clone(&self.backend);
        let home = self.backend.home().to_path_buf();
        let demo = self.args.demo;
        self.tasks.push(cx.spawn(async move |this, cx| {
            let step = |this: &gpui::WeakEntity<Self>, cx: &mut gpui::AsyncApp, label: &str, state: Step| {
                let _gone = this.update(cx, |m, cx| {
                    match m.setup_steps.iter_mut().find(|(l, _)| l == label) {
                        Some(entry) => entry.1 = state,
                        None => m.setup_steps.push((label.to_owned(), state)),
                    }
                    cx.notify();
                });
            };
            // The demo shows the steps going by; nothing changes on the computer.
            let pause = async |cx: &mut gpui::AsyncApp| {
                if demo {
                    cx.background_executor().timer(Duration::from_millis(700)).await;
                }
            };
            // An AppImage's own path changes every run: the command line tool goes to ~/.local/bin first.
            if std::env::var_os("APPIMAGE").is_some() && !demo {
                let label = "Install the command line tool";
                step(&this, cx, label, Step::Running);
                let state = match backend.install_cli() {
                    Ok(p) => Step::Done(p.display().to_string()),
                    Err(e) => Step::Failed(e),
                };
                step(&this, cx, label, state);
            }
            let label = "Start the background service";
            step(&this, cx, label, Step::Running);
            pause(cx).await;
            let started = if demo {
                Ok("The background service runs at login.".to_owned())
            } else {
                let service_backend = Arc::clone(&backend);
                Tokio::spawn(cx, async move { service_backend.start_service().await })
                    .await
                    .unwrap_or_else(|e| Err(e.to_string()))
            };
            let service_ok = started.is_ok();
            step(&this, cx, label, started.map_or_else(Step::Failed, Step::Done));
            if service_ok && git {
                let label = "Send git through Reins";
                step(&this, cx, label, Step::Running);
                pause(cx).await;
                let routed = if demo {
                    Ok(String::new())
                } else {
                    let git_backend = Arc::clone(&backend);
                    Tokio::spawn(cx, async move { git_backend.resume().await })
                        .await
                        .unwrap_or_else(|e| Err(e.to_string()))
                        .map(Option::unwrap_or_default)
                };
                step(
                    &this,
                    cx,
                    label,
                    routed.map_or_else(Step::Failed, |direct| {
                        Step::Done(if direct.is_empty() {
                            "GitHub pushes and clones ask your phone".to_owned()
                        } else {
                            direct
                        })
                    }),
                );
            }
            if login {
                let label = "Open Reins when you log in";
                step(&this, cx, label, Step::Running);
                pause(cx).await;
                let state = match if demo {
                    Ok(())
                } else {
                    autostart::set(&home, true)
                } {
                    Ok(()) => Step::Done("The shield in the menu bar or tray shows that Reins is on".to_owned()),
                    Err(e) => Step::Failed(e),
                };
                let _gone = this.update(cx, |m, _| m.autostart |= matches!(state, Step::Done(_)));
                step(&this, cx, label, state);
            }
            let ok = this
                .update(cx, |m, cx| {
                    m.setup_running = false;
                    let failed = m.setup_steps.iter().any(|(_, s)| matches!(s, Step::Failed(_)));
                    if !failed {
                        m.saved.setup_done = true;
                        m.saved.setup_step = Stage::Done;
                        m.saved.unpause();
                        m.persist();
                    }
                    m.refresh_now(cx);
                    cx.notify();
                    !failed
                })
                .unwrap_or(false);
            if ok {
                cx.background_executor().timer(TURNED_ON_PAUSE).await;
                let _gone = this.update(cx, |m, cx| m.go_to(Stage::Done, cx));
            }
        }));
    }

    /// "Open Reins" (or "Continue anyway"): the status window.
    pub fn finish_setup(&mut self, cx: &mut Context<'_, Self>) {
        self.saved.setup_done = true;
        self.saved.setup_step = Stage::Done;
        self.persist();
        self.screen = Screen::Status;
        self.section = Section::Overview;
        self.notice = None;
        cx.notify();
    }

    // The health checks and the test.

    /// Runs the health checks (`reins doctor`) now, unless they are running.
    pub fn run_checks(&mut self, cx: &mut Context<'_, Self>) {
        if self.health.running {
            return;
        }
        self.health.running = true;
        cx.notify();
        if self.args.demo {
            let fixed = self.demo_fixed.clone();
            cx.spawn(async move |this, cx| {
                cx.background_executor().timer(Duration::from_millis(700)).await;
                let _gone = this.update(cx, |m, cx| m.set_checks(Ok(health::demo_checks(&fixed)), cx));
            })
            .detach();
            return;
        }
        let doctor = match self.backend.doctor() {
            Ok(d) => d,
            Err(e) => {
                self.set_checks(Err(e), cx);
                return;
            }
        };
        cx.spawn(async move |this, cx| {
            let checks = Tokio::spawn(cx, async move { doctor.run().await }).await.map_err(|e| e.to_string());
            let _gone = this.update(cx, |m, cx| m.set_checks(checks, cx));
        })
        .detach();
    }

    fn set_checks(&mut self, checks: Result<Vec<Check>, String>, cx: &mut Context<'_, Self>) {
        self.health.running = false;
        self.health.at = Some(reins_desktop::now_unix());
        match checks {
            Ok(checks) => {
                self.health.checks = Some(checks);
                self.health.error = None;
            }
            Err(e) => self.health.error = Some(e),
        }
        cx.notify();
    }

    /// Ctrl/Cmd+R: reads the status again and runs the checks.
    pub fn refresh_all(&mut self, cx: &mut Context<'_, Self>) {
        self.refresh_now(cx);
        if self.screen == Screen::Status {
            self.run_checks(cx);
        }
    }

    /// A health card button: does the fix, then checks again.
    pub fn fix(&mut self, fix: Fix, cx: &mut Context<'_, Self>) {
        if self.args.demo {
            if let Fix::Connect(h) = fix {
                self.demo_fixed.push(format!("harness:{}", h.id()));
                self.note = Some(reins_desktop::harness::after_add_note(h).to_owned());
            }
            self.run_checks(cx);
            return;
        }
        let backend = Arc::clone(&self.backend);
        match fix {
            Fix::Start => self.act_with(async move { backend.start_service().await.map(Some) }, true, cx),
            Fix::Restart => self.act_with(async move { backend.restart_service().await }, true, cx),
            Fix::Resume => {
                self.saved.unpause();
                self.persist();
                self.act_with(async move { backend.resume().await }, true, cx);
            }
            Fix::Connect(h) => self.act_with(
                async move {
                    backend.set_harness(h, true).map(|()| Some(reins_desktop::harness::after_add_note(h).to_owned()))
                },
                true,
                cx,
            ),
            Fix::PairAgain => self.sign_out(cx),
        }
    }

    /// "Send a test to my phone" (`reins test`): a harmless question; the answer shows where the button was.
    pub fn send_test(&mut self, cx: &mut Context<'_, Self>) {
        if matches!(self.test, Test::Waiting(_)) || !self.paired() {
            return;
        }
        let sent = Instant::now();
        self.test = Test::Waiting(sent);
        cx.notify();
        // The countdown.
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_secs(1)).await;
                let waiting = this.update(cx, |m, cx| {
                    let waiting = m.test == Test::Waiting(sent);
                    if waiting {
                        cx.notify();
                    }
                    waiting
                });
                if !waiting.unwrap_or(false) {
                    break;
                }
            }
        })
        .detach();
        let demo = self.args.demo;
        let backend = Arc::clone(&self.backend);
        cx.spawn(async move |this, cx| {
            let ended = if demo {
                cx.background_executor().timer(DEMO_ANSWER).await;
                Ok((Answer::Yes, false))
            } else {
                Tokio::spawn(cx, async move { backend.send_test().await }).await.unwrap_or_else(|e| Err(e.to_string()))
            };
            let _gone = this.update(cx, |m, cx| {
                let (answer, timed_out) = ended.unwrap_or_else(|e| (Answer::Unanswered(e), false));
                m.test = Test::Ended(answer, timed_out);
                m.refresh_now(cx);
                cx.notify();
            });
        })
        .detach();
    }

    pub fn show_section(&mut self, section: Section, cx: &mut Context<'_, Self>) {
        if self.section != section {
            self.section = section;
            self.show_phone_qr = false;
            cx.notify();
        }
    }

    // The status window's actions.

    fn persist(&mut self) {
        // `--demo` keeps its pretend pairing, setup and pause to itself: written here, they would end up in a real
        // install's `app.json` (setup done, a made-up phone and account).
        if self.args.demo {
            return;
        }
        if let Err(e) = self.saved.save(&self.backend.paths().state_dir) {
            log::warn!("cannot save the app's state: {e}");
        }
    }

    /// Reads the status again now (after an action).
    fn refresh_now(&mut self, cx: &mut Context<'_, Self>) {
        let backend = Arc::clone(&self.backend);
        cx.spawn(async move |this, cx| {
            let snapshot = Tokio::spawn(cx, async move { backend.snapshot().await }).await;
            if let Ok(s) = snapshot {
                let _gone = this.update(cx, |m, cx| m.set_snapshot(s, cx));
            }
        })
        .detach();
    }

    /// Runs `work` on the tokio runtime; shows its error, then reads the status again.
    fn act<F>(&mut self, work: F, cx: &mut Context<'_, Self>)
    where
        F: Future<Output = Result<(), String>> + Send + 'static,
    {
        self.act_noting(async move { work.await.map(|()| None) }, cx);
    }

    /// [`Self::act`] for work that may have something to say.
    fn act_noting<F>(&mut self, work: F, cx: &mut Context<'_, Self>)
    where
        F: Future<Output = Result<Option<String>, String>> + Send + 'static,
    {
        self.act_with(work, false, cx);
    }

    /// [`Self::act_noting`], running the health checks again afterwards when `recheck`.
    fn act_with<F>(&mut self, work: F, recheck: bool, cx: &mut Context<'_, Self>)
    where
        F: Future<Output = Result<Option<String>, String>> + Send + 'static,
    {
        self.notice = None;
        self.note = None;
        cx.spawn(async move |this, cx| {
            let result = Tokio::spawn(cx, work).await.unwrap_or_else(|e| Err(e.to_string()));
            let _gone = this.update(cx, |m, cx| {
                match result {
                    Ok(note) => m.note = note,
                    Err(e) => m.notice = Some(e),
                }
                m.refresh_now(cx);
                if recheck {
                    m.run_checks(cx);
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Pauses git's routing for `length` (or until the user resumes). Hooks and MCP tools still ask the phone.
    pub fn pause_for(&mut self, length: PauseFor, cx: &mut Context<'_, Self>) {
        self.saved.pause(length, reins_desktop::now_unix());
        self.persist();
        if self.args.demo {
            // Nothing changes on the computer in the demo.
            self.refresh_now(cx);
        } else {
            let backend = Arc::clone(&self.backend);
            self.act(async move { backend.pause() }, cx);
        }
        self.refresh_tray();
        cx.notify();
    }

    pub fn resume(&mut self, cx: &mut Context<'_, Self>) {
        self.saved.unpause();
        self.persist();
        if self.args.demo {
            self.refresh_now(cx);
        } else {
            let backend = Arc::clone(&self.backend);
            self.act_noting(async move { backend.resume().await }, cx);
        }
        self.refresh_tray();
        cx.notify();
    }

    /// Writes one setting to `config.toml` (restarting the background service when the daemon needs it).
    pub fn change(&mut self, setting: Setting, cx: &mut Context<'_, Self>) {
        // The demo changes only a stand-in home's settings, never the real ones, and restarts nothing.
        let demo = self.args.demo;
        if demo && std::env::var_os("REINS_HOME").is_none_or(|h| h.is_empty()) {
            self.notice = Some("The demo changes settings only with REINS_HOME set.".to_owned());
            cx.notify();
            return;
        }
        let backend = Arc::clone(&self.backend);
        let paused = self.pause().on();
        self.act_noting(async move { backend.change(setting, paused, !demo).await }, cx);
    }

    /// A guard list as `config.toml` has it now.
    #[must_use]
    pub fn guard_list(&self, list: GuardList) -> Vec<String> {
        let Some(Ok(config)) = self.snapshot.as_ref().map(|s| &s.config) else {
            return Vec::new();
        };
        let g = &config.guard;
        match list {
            GuardList::Commands => g.commands.clone(),
            GuardList::Files => g.files.clone(),
            GuardList::AllowCommands => g.allow_commands.clone(),
            GuardList::AllowFiles => g.allow_files.clone(),
        }
    }

    /// Adds `item` to a guard list (nothing when it is empty or already there).
    pub fn add_guard_item(&mut self, list: GuardList, item: &str, cx: &mut Context<'_, Self>) {
        let mut items = self.guard_list(list);
        let item = item.trim();
        if item.is_empty() || items.iter().any(|i| i == item) {
            return;
        }
        items.push(item.to_owned());
        self.change(Setting::GuardList(list, items), cx);
    }

    pub fn remove_guard_item(&mut self, list: GuardList, item: &str, cx: &mut Context<'_, Self>) {
        let mut items = self.guard_list(list);
        items.retain(|i| i != item);
        self.change(Setting::GuardList(list, items), cx);
    }

    pub fn set_harness(&mut self, h: Harness, on: bool, cx: &mut Context<'_, Self>) {
        let backend = Arc::clone(&self.backend);
        self.act(async move { backend.set_harness(h, on) }, cx);
    }

    pub fn toggle_autostart(&mut self, cx: &mut Context<'_, Self>) {
        let home = self.backend.home().to_path_buf();
        match autostart::set(&home, !self.autostart) {
            Ok(()) => self.autostart = !self.autostart,
            Err(e) => self.notice = Some(e),
        }
        cx.notify();
    }

    pub fn install_cli(&mut self, cx: &mut Context<'_, Self>) {
        self.notice = None;
        self.note = None;
        match self.backend.install_cli() {
            Ok(p) => self.note = Some(format!("Installed `reins` at {}.", p.display())),
            Err(e) => self.notice = Some(e),
        }
        cx.notify();
    }

    pub fn start_service(&mut self, cx: &mut Context<'_, Self>) {
        let backend = Arc::clone(&self.backend);
        self.act_noting(async move { backend.start_service().await.map(Some) }, cx);
    }

    pub fn sign_out(&mut self, cx: &mut Context<'_, Self>) {
        if !self.args.demo {
            let backend = Arc::clone(&self.backend);
            self.act_noting(async move { backend.sign_out().await }, cx);
        }
        self.demo_paired = false;
        self.saved.setup_done = false;
        self.saved.setup_step = Stage::Pair;
        self.saved.unpause();
        self.saved.phone = None;
        self.saved.account = None;
        self.persist();
        self.health = Health::default();
        self.test = Test::Idle;
        self.tools.clear();
        self.screen = Screen::Welcome(Stage::Pair);
        self.start_pairing(cx);
        self.refresh_now(cx);
    }

    pub fn toggle_phone_qr(&mut self, cx: &mut Context<'_, Self>) {
        self.show_phone_qr = !self.show_phone_qr;
        cx.notify();
    }

    /// "Restart to update": installs the downloaded update and quits; the new Reins starts when this one has ended.
    pub fn restart_to_update(&mut self, cx: &mut Context<'_, Self>) {
        let Some(ready) = self.update.as_ref().and_then(|u| u.ready.clone()) else {
            return;
        };
        if self.updating {
            return;
        }
        self.updating = true;
        self.notice = None;
        cx.notify();
        let backend = Arc::clone(&self.backend);
        self.tasks.push(cx.spawn(async move |this, cx| {
            let installed = cx.background_executor().spawn(async move { backend.install_update(&ready) }).await;
            let _gone = this.update(cx, |m, cx| {
                match installed {
                    Ok(upgrade::Installed::Restarting) => {
                        log::info!("updating: Reins starts again when this one has quit");
                        cx.quit();
                        return;
                    }
                    Ok(upgrade::Installed::Opened) => {
                        m.notice = Some("Quit Reins, then drag the new Reins to Applications.".to_owned());
                    }
                    Err(e) => m.notice = Some(e),
                }
                m.updating = false;
                cx.notify();
            });
        }));
    }
}

impl Look {
    /// The word for the status pill.
    #[must_use]
    pub fn word(self) -> &'static str {
        match self {
            Self::On => "On",
            Self::Paused => "Paused",
            Self::Waiting => "Waiting",
            Self::Attention => "Needs you",
        }
    }
}

/// `REINS_DEMO_SCREEN`: `pair`, `tools` (`tools-connected`: one connected just now), `turn-on` (`turning-on`: and
/// turning on), `done` or `status`.
fn demo_screen(name: &str) -> Option<Screen> {
    Some(match name.trim().to_ascii_lowercase().replace('_', "-").as_str() {
        "status" => Screen::Status,
        "pair" | "onboarding" => Screen::Welcome(Stage::Pair),
        "tools" | "tools-connected" | "setup" => Screen::Welcome(Stage::Tools),
        "turn-on" | "turning-on" => Screen::Welcome(Stage::TurnOn),
        "done" => Screen::Welcome(Stage::Done),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sections_are_named_by_their_ids() {
        for s in Section::ALL {
            assert_eq!(Section::from_id(s.id()), Some(s));
            assert!(!s.label().is_empty());
        }
        assert_eq!(Section::from_id(" Rules "), Some(Section::Rules));
        assert_eq!(Section::from_id("nope"), None);
    }

    #[test]
    fn the_tray_says_what_waits_on_the_phone() {
        let waiting = |what: &str| {
            let mut e = Entry::new(reins_desktop::journal::Kind::Command, what);
            e.outcome = Outcome::Waiting;
            e
        };
        let mut done = Entry::new(reins_desktop::journal::Kind::Git, "push");
        done.outcome = Outcome::Approved;
        assert_eq!(waiting_line(std::slice::from_ref(&done)), None);
        assert_eq!(
            waiting_line(&[done.clone(), waiting("Deploy now?")]).as_deref(),
            Some("Waiting on your phone: Deploy now?")
        );
        assert_eq!(
            waiting_line(&[waiting("a"), done, waiting("b"), waiting("c")]).as_deref(),
            Some("Waiting on your phone: a (+2 more)")
        );
        let long = waiting_line(&[waiting(&"x".repeat(200))]).unwrap();
        assert!(long.ends_with('…') && long.chars().count() < 100, "{long}");
    }

    #[test]
    fn demo_screens_by_name() {
        assert_eq!(demo_screen("status"), Some(Screen::Status));
        assert_eq!(demo_screen(" Turn_On "), Some(Screen::Welcome(Stage::TurnOn)));
        assert_eq!(demo_screen("tools-connected"), Some(Screen::Welcome(Stage::Tools)));
        assert_eq!(demo_screen("done"), Some(Screen::Welcome(Stage::Done)));
        assert_eq!(demo_screen("nope"), None);
    }
}
