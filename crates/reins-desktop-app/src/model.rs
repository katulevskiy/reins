//! The app's state and what it does: which screen (and, in the status window, which section) the window shows, the
//! pairing in progress, the first-time setup, the status the window and the tray icon show, pausing, and the
//! settings the window changes. One [`Model`] per app (a GPUI global); the window and the tray
//! are views of it.

use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt as _;
use gpui::{
    App, AppContext as _, Bounds, Context, Entity, Global, Task, TitlebarOptions, WindowBounds, WindowHandle,
    WindowKind, WindowOptions, point, px, size,
};
use gpui_tokio::Tokio;
use reins_desktop::harness::Harness;

use reins_desktop::settings::GuardList;

use crate::backend::{Backend, DaemonState, Setting, Snapshot, Update};
use crate::demo::Demo;
use crate::pairing::{self, Approved, DemoFlow, DeviceCode, DeviceFlow, Poll, ServerFlow};
use crate::pause::{Pause, PauseFor};
use crate::single::Instance;
use crate::state::Saved;
use crate::tray::{Action, Look, Shown, Tray};
use crate::{Args, autostart, ui, upgrade};

/// How often the status is read again.
const REFRESH: Duration = Duration::from_secs(3);
/// How often the update feed is asked.
const UPDATE_CHECK: Duration = Duration::from_hours(6);
/// How long "Approved on …" stays before the setup screen.
const APPROVED_PAUSE: Duration = Duration::from_millis(1600);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Screen {
    Onboarding,
    Setup,
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

/// One line of the first-time setup's checklist.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SetupItem {
    pub what: SetupWhat,
    pub checked: bool,
    /// Can be ticked (a harness that is not installed cannot).
    pub available: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SetupWhat {
    Harness(Harness),
    Git,
    Autostart,
}

impl SetupItem {
    #[must_use]
    pub fn label(&self) -> String {
        match self.what {
            SetupWhat::Harness(h) => h.label().to_owned(),
            SetupWhat::Git => "Send git through Reins".to_owned(),
            SetupWhat::Autostart => "Open Reins when you log in".to_owned(),
        }
    }

    #[must_use]
    pub fn detail(&self) -> &'static str {
        match self.what {
            SetupWhat::Harness(_) if !self.available => "Not installed on this computer",
            SetupWhat::Harness(_) => "Its tools reach your phone; risky commands wait for you",
            SetupWhat::Git => "Pushes and clones of GitHub wait for your OK; agents never see your token",
            SetupWhat::Autostart => "A shield in the menu bar shows that Reins is on",
        }
    }
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
    pub setup_items: Vec<SetupItem>,
    pub setup_steps: Vec<(String, Step)>,
    pub setup_running: bool,
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
        // `--demo` with `REINS_DEMO_SCREEN=status`: straight to the status window, as if paired and set up.
        let demo_status = args.demo && std::env::var("REINS_DEMO_SCREEN").is_ok_and(|v| v.trim() == "status");
        if demo_status {
            saved.setup_done = true;
            saved.account.get_or_insert_with(|| "dana@acme.dev".to_owned());
            saved.phone.get_or_insert_with(|| "Dana's iPhone".to_owned());
        }
        let section = std::env::var("REINS_DEMO_SECTION")
            .ok()
            .filter(|_| args.demo)
            .and_then(|s| Section::from_id(&s))
            .unwrap_or(Section::Overview);
        let paired = demo_status || backend.paired_server().is_some();
        let screen = if !paired {
            Screen::Onboarding
        } else if !saved.setup_done {
            Screen::Setup
        } else {
            Screen::Status
        };
        let home = backend.home().to_path_buf();
        let runtime = Tokio::handle(cx);
        let demo = args.demo.then(|| Demo::new(reins_desktop::now_unix()));
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
                setup_items: Vec::new(),
                setup_steps: Vec::new(),
                setup_running: false,
                update: None,
                updating: false,
                notice: None,
                note: None,
                show_phone_qr: false,
                show_server: false,
                autostart: autostart::enabled(&home),
                demo_paired: demo_status,
                demo,
                minute: 0,
                pairing_task: None,
                window: None,
                tray: None,
                tasks: Vec::new(),
            };
            model.start_tray(cx);
            model.start_loops(cx);
            match screen {
                Screen::Onboarding => model.start_pairing(cx),
                Screen::Setup => model.prepare_setup(),
                Screen::Status => {}
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
            Action::Quit => cx.quit(),
        }
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
                    })
                    .is_err();
                if gone {
                    break;
                }
                cx.background_executor().timer(REFRESH).await;
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
        if snapshot.server.is_none() && !self.demo_paired && self.screen != Screen::Onboarding {
            self.screen = Screen::Onboarding;
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
        if let Some(line) = self.pause().line(reins_desktop::now_unix()) {
            return (Look::Paused, line);
        }
        match self.snapshot.as_ref().map(|s| &s.daemon) {
            Some(DaemonState::Stopped) if self.saved.setup_done => {
                (Look::Attention, "The background service is not running".to_owned())
            }
            Some(DaemonState::Unknown(_)) => (Look::Attention, "Cannot reach the background service".to_owned()),
            _ if self.snapshot.as_ref().is_some_and(|s| s.git_routed.is_empty()) && self.saved.setup_done => {
                (Look::Paused, "Paused: git goes to GitHub directly".to_owned())
            }
            _ => (Look::On, "Reins is on".to_owned()),
        }
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
        let bounds = Bounds::centered(None, size(px(ui::WIDTH), px(ui::HEIGHT)), cx);
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
                model.update(cx, |m, _| m.window = Some(handle));
                let id = handle.window_id();
                cx.on_window_closed(move |_, closed| {
                    if closed == id {
                        show_in_dock(false);
                    }
                })
                .detach();
            }
            Err(e) => log::warn!("cannot open the window: {e}"),
        }
        cx.activate(true);
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
                    if std::time::Instant::now() >= code.expires_at {
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
                m.screen = Screen::Setup;
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

    fn prepare_setup(&mut self) {
        let mut items: Vec<SetupItem> = self
            .backend
            .harnesses()
            .into_iter()
            .map(|row| SetupItem {
                what: SetupWhat::Harness(row.harness),
                checked: row.found,
                available: row.found,
            })
            .collect();
        // Installed ones first.
        items.sort_by_key(|i| !i.available);
        items.push(SetupItem {
            what: SetupWhat::Git,
            checked: true,
            available: true,
        });
        items.push(SetupItem {
            what: SetupWhat::Autostart,
            checked: true,
            available: true,
        });
        self.setup_items = items;
        self.setup_steps.clear();
        self.setup_running = false;
    }

    pub fn toggle_item(&mut self, index: usize, cx: &mut Context<'_, Self>) {
        if let Some(item) = self.setup_items.get_mut(index)
            && item.available
            && !self.setup_running
        {
            item.checked = !item.checked;
            cx.notify();
        }
    }

    /// Whether the setup ran and ended (successfully or not).
    #[must_use]
    pub fn setup_finished(&self) -> bool {
        !self.setup_running && !self.setup_steps.is_empty()
    }

    pub fn run_setup(&mut self, cx: &mut Context<'_, Self>) {
        if self.setup_running {
            return;
        }
        self.setup_running = true;
        self.setup_steps.clear();
        self.notice = None;
        cx.notify();
        let items = self.setup_items.clone();
        let backend = Arc::clone(&self.backend);
        let home = self.backend.home().to_path_buf();
        self.tasks.push(cx.spawn(async move |this, cx| {
            let step = |this: &gpui::WeakEntity<Self>, cx: &mut gpui::AsyncApp, label: String, state: Step| {
                let _gone = this.update(cx, |m, cx| {
                    match m.setup_steps.iter_mut().find(|(l, _)| *l == label) {
                        Some(entry) => entry.1 = state,
                        None => m.setup_steps.push((label, state)),
                    }
                    cx.notify();
                });
            };
            // An AppImage's own path changes every run: the command line tool goes to ~/.local/bin first.
            if std::env::var_os("APPIMAGE").is_some() {
                let label = "Install the command line tool".to_owned();
                step(&this, cx, label.clone(), Step::Running);
                let state = match backend.install_cli() {
                    Ok(p) => Step::Done(p.display().to_string()),
                    Err(e) => Step::Failed(e),
                };
                step(&this, cx, label, state);
            }
            for item in items.iter().filter(|i| i.checked && i.available) {
                match item.what {
                    SetupWhat::Harness(h) => {
                        let label = format!("Add Reins to {}", h.label());
                        step(&this, cx, label.clone(), Step::Running);
                        let state = match backend.set_harness(h, true) {
                            Ok(()) => Step::Done(reins_desktop::harness::after_add_note(h).to_owned()),
                            Err(e) => Step::Failed(e),
                        };
                        step(&this, cx, label, state);
                    }
                    SetupWhat::Autostart => {
                        let label = "Open Reins when you log in".to_owned();
                        let state = match autostart::set(&home, true) {
                            Ok(()) => Step::Done(String::new()),
                            Err(e) => Step::Failed(e),
                        };
                        let _gone = this.update(cx, |m, _| m.autostart = matches!(state, Step::Done(_)));
                        step(&this, cx, label, state);
                    }
                    SetupWhat::Git => {}
                }
            }
            let label = "Start the background service".to_owned();
            step(&this, cx, label.clone(), Step::Running);
            let service_backend = Arc::clone(&backend);
            let started = Tokio::spawn(cx, async move { service_backend.start_service().await })
                .await
                .unwrap_or_else(|e| Err(e.to_string()));
            let service_ok = started.is_ok();
            step(&this, cx, label, started.map_or_else(Step::Failed, Step::Done));
            if service_ok && items.iter().any(|i| i.what == SetupWhat::Git && i.checked) {
                let label = "Send git through Reins".to_owned();
                step(&this, cx, label.clone(), Step::Running);
                let git_backend = Arc::clone(&backend);
                let routed = Tokio::spawn(cx, async move { git_backend.resume().await })
                    .await
                    .unwrap_or_else(|e| Err(e.to_string()));
                step(
                    &this,
                    cx,
                    label,
                    routed.map_or_else(Step::Failed, |()| {
                        Step::Done("GitHub pushes and clones ask your phone".to_owned())
                    }),
                );
            }
            let _gone = this.update(cx, |m, cx| {
                m.setup_running = false;
                let failed = m.setup_steps.iter().any(|(_, s)| matches!(s, Step::Failed(_)));
                if !failed {
                    m.saved.setup_done = true;
                    m.saved.unpause();
                    m.persist();
                }
                m.refresh_now(cx);
                cx.notify();
            });
        }));
    }

    pub fn finish_setup(&mut self, cx: &mut Context<'_, Self>) {
        self.saved.setup_done = true;
        self.persist();
        self.screen = Screen::Status;
        self.section = Section::Overview;
        cx.notify();
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
            self.act(async move { backend.resume().await }, cx);
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
        if let Err(e) = self.backend.sign_out() {
            self.notice = Some(e);
        }
        self.demo_paired = false;
        self.saved.setup_done = false;
        self.saved.unpause();
        self.saved.phone = None;
        self.saved.account = None;
        self.persist();
        self.screen = Screen::Onboarding;
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
            Self::Attention => "Needs you",
        }
    }
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
}
