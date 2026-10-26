//! The window. Before pairing and setup: the welcome flow, one calm column under its steps (pair, AI tools, turn on,
//! done). Afterwards the status window: a sidebar (what Reins is doing, the sections, pausing) and the section it
//! selects: Home, Activity, Connections, Keys, Rules, Settings. In Reins Signal ([`crate::theme`]): paper, ink and
//! hairlines, a field of colour only where something needs a decision, the Signal glyphs and real logos; its
//! components are in [`kit`] (and, with `--demo`, in the gallery). Keyboard shortcuts ([`crate::shortcuts`]) work
//! everywhere in it.

mod activity;
mod connections;
mod diff;
mod field;
mod gallery;
mod health;
mod hold;
mod home;
pub mod icons;
mod keys;
pub mod kit;
mod onboarding;
mod parts;
mod rules;
mod secrets;
mod settings;
mod setup;
mod sidebar;
mod story;
mod work;

/// A request in one sentence, for the notification that says to check the phone.
pub(crate) use story::{banner, spoken};

/// A request's sentence as plain text, for the tray.
pub(crate) fn ask_line(e: &Entry) -> String {
    story::Ask::of(e).line()
}

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

use gpui::{
    AnyElement, App, ClickEvent, Context, Entity, FocusHandle, InteractiveElement as _, IntoElement, KeyDownEvent,
    ParentElement as _, Render, StatefulInteractiveElement as _, Styled as _, Subscription, Task, Window, div, px,
};
use reins_desktop::config::Config;
use reins_desktop::control::Overview;
use reins_desktop::journal::{Entry, Kind, Outcome};
use reins_desktop::pushes::Push;

use crate::backend::{Snapshot, Update};
use crate::health::{Health, Test};
use crate::model::{Model, Screen, Section};
use crate::pause::{Pause, PauseFor};
use crate::shortcuts::{self, Shortcut};
use crate::state::Saved;
use crate::theme::Palette;
use crate::tray::Look;
use crate::welcome::Stage;
use crate::work::Work;

/// The window's first size, and the smallest it can be.
pub const WIDTH: f32 = 1000.0;
pub const HEIGHT: f32 = 680.0;
pub const MIN_WIDTH: f32 = 780.0;
pub const MIN_HEIGHT: f32 = 560.0;

/// How many activity rows show before "Show more".
const ROWS: usize = 60;
/// The space above a page's title (the boards' `.main` padding).
const TOP: f32 = 34.0;

#[allow(clippy::struct_excessive_bools, reason = "independent things open in the window")]
pub struct Root {
    model: Entity<Model>,
    /// The window's own focus, so the keyboard shortcuts work before anything else is focused.
    focus: FocusHandle,
    server_field: FocusHandle,
    /// The inputs under Rules, one per guard list ([`field::slot`]).
    guard_fields: [FocusHandle; 4],
    guard_inputs: [String; 4],
    /// Each Rules card's field adds a file (else a command), as picked; and why the last entry was refused.
    guard_file: [bool; 2],
    guard_error: [Option<String>; 2],
    /// The work session form's fields: what it is for, and a branch for each repository (made as rows appear).
    session_reason: FocusHandle,
    branch_fields: Vec<FocusHandle>,
    /// The secrets review's field for the phone key's digits.
    phone_key_field: FocusHandle,
    /// The secrets review's list (only the rows in view are drawn), and the rows it was made for.
    scan_list: gpui::ListState,
    scan_list_of: usize,
    /// Activity's filters: one outcome, one kind (`None`: all), and whether their popover is open.
    outcome_filter: Option<Outcome>,
    kind_filter: Option<Kind>,
    filter_open: bool,
    /// When a press outside closed a menu (a press on its trigger then does not reopen it).
    menu_closed: Option<std::time::Instant>,
    /// The request Activity shows on the right (by journal id; `None`: the first one).
    selected: Option<String>,
    rows_shown: usize,
    /// The push kept for the request Activity shows (its id and how it ended when read), read once when picked.
    push: Option<(String, Outcome, Option<Arc<Push>>)>,
    /// That push's commits open to their files, and its files open to their diff (commit, file; commits counted
    /// across its refs).
    commits_open: BTreeSet<usize>,
    files_open: BTreeSet<(usize, usize)>,
    /// The "Pause git" sheet is open, and the length picked in it.
    pause_sheet: bool,
    pause_choice: PauseFor,
    /// A hold to confirm in progress ([`hold`]), and how many presses there were (each its own fill).
    hold: Option<hold::Hold>,
    presses: u64,
    /// Home's Health tile is open to its checks.
    health_open: bool,
    /// Settings shows its Wallpaper page, for dark paper's wallpaper (else light's).
    wallpaper_page: bool,
    wallpaper_dark_slot: bool,
    /// The welcome was left (Set up, or Other server): the pairing step shows.
    welcomed: bool,
    /// The AI tool Connections shows on a page of its own.
    tool_page: Option<reins_desktop::harness::Harness>,
    /// The waiting request whose field Home hides (Hide on the field).
    hidden_ask: Option<String>,
    /// Activity's days folded or unfolded from how they start (Today open, the rest folded).
    days_toggled: BTreeSet<String>,
    /// Settings' Notifications row is open to its two switches.
    notify_open: bool,
    /// Settings' Name row is open to its field.
    name_open: bool,
    /// The run profile open to its variables (Keys & secrets).
    profile_open: Option<String>,
    /// The (?) open now ([`parts::help`]), and the page it is on: another page closes it.
    help: Option<&'static str>,
    help_page: (Screen, Section),
    /// The sidebar's pill under the selected section, and where it slid from.
    nav: sidebar::NavPill,
    /// Settings' Appearance menu is open.
    appearance_open: bool,
    /// Renders once a second while something counts down in seconds (a request's timer, a hold).
    tick: Option<Task<()>>,
    /// The component gallery shows instead of a section (`--demo` only), and what its controls are set to.
    gallery: bool,
    gallery_state: gallery::State,
    gallery_held: bool,
    _subscriptions: Vec<Subscription>,
}

/// What a render reads from the model, read once.
#[allow(clippy::struct_excessive_bools, reason = "independent flags the window shows")]
struct Data {
    snapshot: Option<Arc<Snapshot>>,
    saved: Saved,
    look: Look,
    line: String,
    pause: Pause,
    paired: bool,
    server: String,
    fingerprint: Option<String>,
    update: Option<Update>,
    updating: bool,
    notice: Option<String>,
    note: Option<String>,
    show_qr: bool,
    autostart: bool,
    cli_installed: bool,
    section: Section,
    /// Where `config.toml` is, for the empty states that say to edit it.
    config_file: String,
    health: Health,
    test: Test,
    work: Work,
    now: i64,
    /// The (?) open now.
    help: Option<&'static str>,
    /// Keys & secrets' scan and move.
    secrets: crate::secrets::Flow,
    /// This computer's name, and its machine project.
    computer: String,
    machine: String,
    /// This computer's own name, and the one the user gave it on the phone (which the phone shows instead).
    shown_as: String,
    phone_name: Option<String>,
    /// `--demo`, and the gallery showing.
    demo: bool,
    gallery: bool,
}

impl Data {
    fn of(m: &Model) -> Self {
        let (look, line) = m.status_line();
        Self {
            snapshot: m.snapshot.clone(),
            saved: m.saved.clone(),
            look,
            line,
            pause: m.pause(),
            paired: m.paired(),
            server: m.server(),
            fingerprint: m.fingerprint.clone(),
            update: m.update.clone(),
            updating: m.updating,
            notice: m.notice.clone(),
            note: m.note.clone(),
            show_qr: m.show_phone_qr,
            autostart: m.autostart,
            cli_installed: m.cli_installed,
            section: m.section,
            config_file: m.config_file().display().to_string(),
            health: m.health.clone(),
            test: m.test.clone(),
            work: m.work.clone(),
            now: reins_desktop::now_unix(),
            help: None,
            secrets: m.secrets.clone(),
            computer: m.computer.clone(),
            machine: m.machine.clone(),
            shown_as: m.shown_as.clone(),
            phone_name: m.phone_name.clone(),
            demo: m.demo(),
            gallery: false,
        }
    }

    fn activity(&self) -> &[Entry] {
        self.snapshot.as_ref().map_or(&[], |s| s.activity.as_slice())
    }

    fn config(&self) -> Option<&Config> {
        self.snapshot.as_ref().and_then(|s| s.config.as_ref().ok()).map(AsRef::as_ref)
    }

    fn overview(&self) -> Option<&Overview> {
        self.snapshot.as_ref().map(|s| s.overview.as_ref())
    }

    /// The work session running now.
    fn work_session(&self) -> Option<&reins_desktop::work_session::Session> {
        crate::work::running(self.snapshot.as_ref().and_then(|s| s.work_session.as_ref()), self.now)
    }

    /// Requests waiting for the phone now.
    fn waiting(&self) -> impl Iterator<Item = &Entry> {
        self.activity().iter().filter(|e| e.outcome == Outcome::Waiting)
    }

    /// The phone that approves ("Dana's iPhone"), or "your phone".
    fn phone(&self) -> String {
        self.saved.phone.clone().unwrap_or_else(|| "your phone".to_owned())
    }

    /// How long the phone has to answer `e`, in seconds, and how much of it is left.
    fn wait_left(&self, e: &Entry) -> (u64, i64) {
        let (hooks, approvals) = self.config().map_or((120, 120), |c| (c.guard.timeout_secs, c.approval_timeout_secs));
        let total = story::wait_secs(e.kind, hooks, approvals).max(1);
        (total, (i64::try_from(total).unwrap_or(i64::MAX) - (self.now - e.at)).max(0))
    }
}

impl Root {
    pub fn new(model: Entity<Model>, window: &mut Window, cx: &mut Context<'_, Self>) -> Self {
        let subscriptions = vec![
            cx.observe(&model, |_, _, cx| cx.notify()),
            cx.observe_window_appearance(window, |_, _, cx| cx.notify()),
            // Where the window is and how big, to open it there next time.
            cx.observe_window_bounds(window, |this, window, cx| {
                // Tiling Wayland compositors report a tiled window as maximized, with the size it first had: there
                // the window's size now is the one to keep. Elsewhere, a maximized window keeps its size before.
                let bounds = if cfg!(target_os = "linux") {
                    window.bounds()
                } else {
                    window.window_bounds().get_bounds()
                };
                this.model.update(cx, |m, _| m.note_window(bounds));
            }),
        ];
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        let (help_page, demo) = {
            let m = model.read(cx);
            ((m.screen, m.section), m.demo())
        };
        // `--demo` with `REINS_DEMO_HELP=<id>`: that (?) open, `REINS_DEMO_PAUSE=1`: the pause sheet open (`manual`:
        // at "Until I resume"), `REINS_DEMO_SELECT=<id>`: that request in Activity, `REINS_DEMO_DIFF=<c>:<f>`: its push's
        // commit c open, and file f of it, for screenshots.
        let demo_var = |name: &str| std::env::var(name).ok().filter(|h| demo && !h.is_empty());
        let help = demo_var("REINS_DEMO_HELP").map(|h| &*Box::leak(h.into_boxed_str()));
        let pause_demo = demo_var("REINS_DEMO_PAUSE");
        let mut root = Self {
            model,
            focus,
            server_field: cx.focus_handle(),
            guard_fields: [cx.focus_handle(), cx.focus_handle(), cx.focus_handle(), cx.focus_handle()],
            guard_inputs: Default::default(),
            guard_file: [false; 2],
            guard_error: [None, None],
            session_reason: cx.focus_handle(),
            branch_fields: Vec::new(),
            phone_key_field: cx.focus_handle(),
            scan_list: gpui::ListState::new(0, gpui::ListAlignment::Top, px(600.0)),
            scan_list_of: 0,
            outcome_filter: None,
            kind_filter: None,
            filter_open: demo_var("REINS_DEMO_FILTER").is_some(),
            menu_closed: None,
            selected: demo_var("REINS_DEMO_SELECT"),
            rows_shown: ROWS,
            push: None,
            commits_open: demo_var("REINS_DEMO_DIFF")
                .and_then(|d| d.split(':').next().and_then(|c| c.parse().ok()))
                .into_iter()
                .collect(),
            files_open: demo_var("REINS_DEMO_DIFF")
                .and_then(|d| d.split_once(':').and_then(|(c, f)| Some((c.parse().ok()?, f.parse().ok()?))))
                .into_iter()
                .collect(),
            pause_sheet: pause_demo.is_some(),
            pause_choice: if matches!(pause_demo.as_deref(), Some("manual" | "hold")) {
                PauseFor::Manual
            } else {
                PauseFor::Hour
            },
            hold: None,
            presses: 0,
            health_open: demo_var("REINS_DEMO_HEALTH").is_some(),
            hidden_ask: None,
            // `--demo` shows the welcome only when `REINS_DEMO_SCREEN=welcome` asks for it.
            welcomed: demo && std::env::var("REINS_DEMO_SCREEN").is_ok_and(|v| v.trim() != "welcome"),
            // `REINS_DEMO_WALLPAPER=dark|light`: Settings' Wallpaper page, for that paper.
            wallpaper_page: demo_var("REINS_DEMO_WALLPAPER").is_some(),
            wallpaper_dark_slot: demo_var("REINS_DEMO_WALLPAPER").is_none_or(|v| v != "light"),
            // `REINS_DEMO_TOOL=<id>`: that AI tool's page under Connections.
            tool_page: demo_var("REINS_DEMO_TOOL").and_then(|id| {
                use reins_desktop::harness::Harness;
                [Harness::ClaudeCode, Harness::Codex, Harness::Gemini, Harness::Cursor].into_iter().find(|h| h.id() == id)
            }),
            days_toggled: BTreeSet::new(),
            notify_open: false,
            name_open: demo_var("REINS_DEMO_NAME_OPEN").is_some(),
            profile_open: None,
            help,
            help_page,
            nav: sidebar::NavPill::default(),
            appearance_open: false,
            tick: None,
            gallery: demo_var("REINS_DEMO_SECTION").is_some_and(|s| s.trim().eq_ignore_ascii_case("gallery")),
            gallery_state: gallery::State::default(),
            gallery_held: false,
            _subscriptions: subscriptions,
        };
        // `REINS_DEMO_PAUSE=hold`: "Until I resume", held from the start (it pauses when the hold ends).
        if pause_demo.as_deref() == Some("hold") {
            root.pause_choice = PauseFor::Manual;
            root.hold_start(hold::Held::PauseUntilResume, cx);
        }
        root
    }

    /// A click handler that runs `f` on the model.
    fn on_model(
        cx: &Context<'_, Self>,
        f: impl Fn(&mut Model, &mut Context<'_, Model>) + 'static,
    ) -> impl Fn(&ClickEvent, &mut Window, &mut App) + 'static {
        cx.listener(move |this, _: &ClickEvent, _, cx| this.model.update(cx, |m, cx| f(m, cx)))
    }

    /// A click handler that changes this view's own state.
    fn on_view(
        cx: &Context<'_, Self>,
        f: impl Fn(&mut Self, &mut Context<'_, Self>) + 'static,
    ) -> impl Fn(&ClickEvent, &mut Window, &mut App) + 'static {
        cx.listener(move |this, _: &ClickEvent, _, cx| {
            f(this, cx);
            cx.notify();
        })
    }

    /// A keyboard shortcut ([`shortcuts::of`]); text fields keep the keys they use.
    fn on_key(&mut self, ev: &KeyDownEvent, window: &mut Window, cx: &mut Context<'_, Self>) {
        let Some(shortcut) = shortcuts::of(&ev.keystroke) else {
            return;
        };
        let status = self.model.read(cx).screen == Screen::Status;
        match shortcut {
            Shortcut::Section(section) if status => self.model.update(cx, |m, cx| m.show_section(section, cx)),
            Shortcut::Section(_) => return,
            Shortcut::Refresh => self.model.update(cx, Model::refresh_all),
            Shortcut::Close => window.remove_window(),
            Shortcut::Quit => self.model.update(cx, Model::quit),
            Shortcut::Escape => {
                let open = self.pause_sheet
                    || self.filter_open
                    || self.appearance_open
                    || self.help.is_some()
                    || self.hold.is_some();
                if !open {
                    // Nothing open here: the phone's QR code, if it shows.
                    self.model.update(cx, |m, cx| {
                        if m.show_phone_qr {
                            m.toggle_phone_qr(cx);
                        }
                    });
                }
                self.pause_sheet = false;
                self.filter_open = false;
                self.appearance_open = false;
                self.help = None;
                self.hold = None;
                // Out of a text field, so the next shortcuts are not typed into it.
                window.focus(&self.focus, cx);
            }
        }
        cx.notify();
        cx.stop_propagation();
    }

    /// Goes to `section`.
    fn go(cx: &Context<'_, Self>, section: Section) -> impl Fn(&ClickEvent, &mut Window, &mut App) + 'static {
        Self::on_model(cx, move |m, cx| m.show_section(section, cx))
    }

    /// Opens `id` in Activity.
    fn open_entry(cx: &Context<'_, Self>, id: String) -> impl Fn(&ClickEvent, &mut Window, &mut App) + 'static {
        cx.listener(move |this, _: &ClickEvent, _, cx| {
            this.selected = Some(id.clone());
            this.outcome_filter = None;
            this.kind_filter = None;
            this.model.update(cx, |m, cx| m.show_section(Section::Activity, cx));
            cx.notify();
        })
    }

    /// Renders again in a second while `needed`, so timers count down.
    fn keep_ticking(&mut self, needed: bool, window: &Window, cx: &mut Context<'_, Self>) {
        if !needed {
            self.tick = None;
            return;
        }
        if self.tick.is_none() {
            self.tick = Some(cx.spawn_in(window, async move |this, cx| {
                loop {
                    cx.background_executor().timer(Duration::from_secs(1)).await;
                    if this.update(cx, |_, cx| cx.notify()).is_err() {
                        break;
                    }
                }
            }));
        }
    }

    /// The status window: sidebar and section; the pause sheet over both.
    fn status(&mut self, d: &Data, pal: Palette, window: &mut Window, cx: &mut Context<'_, Self>) -> AnyElement {
        let secrets_page = if d.section == Section::Keys {
            self.secrets_page(d, pal, window, cx)
        } else {
            None
        };
        // A page in the boards' margins (`.main`: 34 above, 44 at the sides, 40 below), scrolling.
        let scroll = |id: gpui::ElementId, page: AnyElement, padded: bool| {
            div()
                .id(id)
                .size_full()
                .overflow_y_scroll()
                .child(if padded {
                    div().w_full().px(px(home::SIDE)).pt(px(TOP)).pb(px(40.0)).child(page).into_any_element()
                } else {
                    page
                })
                .into_any_element()
        };
        let content: AnyElement = if self.gallery && d.demo {
            scroll("gallery".into(), self.gallery(d, pal, window, cx).into_any_element(), true)
        } else if let Some((page, true)) = secrets_page {
            // The review: its list and its panel side by side.
            page
        } else if let Some((page, false)) = secrets_page {
            scroll("secrets-content".into(), page, true)
        } else {
            let id: gpui::ElementId = ("content", d.section as usize).into();
            match d.section {
                // Home pads itself: a waiting request's field runs to the edges, under the title bar.
                Section::Home => scroll(id, self.home(d, pal, window, cx), false),
                // Activity scrolls its list itself, beside the panel of the request picked.
                Section::Activity => self.activity(d, pal, cx),
                Section::Connections => scroll(id, self.connections(d, pal, cx), true),
                Section::Keys => scroll(id, self.keys(d, pal, cx), true),
                Section::Rules => scroll(id, self.rules(d, pal, window, cx), true),
                Section::Settings => scroll(id, self.settings(d, pal, cx), true),
            }
        };
        let mut root = div()
            .relative()
            .size_full()
            .flex()
            .flex_row()
            .child(self.sidebar(d, pal, cx))
            .child(div().flex_1().min_w(px(0.0)).h_full().child(content));
        if self.pause_sheet && d.paired {
            root = root.child(self.pause_sheet(pal, cx));
        }
        root.into_any_element()
    }
}

impl Render for Root {
    fn render(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let pal = Palette::of(window, self.model.read(cx).saved.appearance);
        kit::field::set_grain(!self.model.read(cx).saved.flat_fields);
        let screen = self.model.read(cx).screen;
        let page = (screen, self.model.read(cx).section);
        if page != self.help_page {
            self.help_page = page;
            self.help = None;
            self.filter_open = false;
            self.tool_page = None;
            self.wallpaper_page = false;
        }
        // The tray's "Until I resume…": the pause sheet, there, to be held.
        if self.model.read(cx).pause_hold_asked {
            self.model.update(cx, |m, _| m.pause_hold_asked = false);
            self.pause_sheet = true;
            self.pause_choice = PauseFor::Manual;
            self.hold = None;
        }
        let mut d = Data::of(self.model.read(cx));
        d.help = self.help;
        d.gallery = self.gallery && d.demo;
        // A hold's fill moves with the time held: the next frame while it does.
        self.hold_frame(window);
        // Seconds count while a request waits, a work session is being asked for, or the test waits.
        self.keep_ticking(
            matches!(d.test, Test::Waiting(_))
                || (screen == Screen::Status && (d.waiting().next().is_some() || d.work.asking.is_some())),
            window,
            cx,
        );
        let hero = screen == Screen::Welcome(Stage::Pair)
            && !self.welcomed
            && !matches!(self.model.read(cx).pairing, crate::model::Pairing::Approved { .. });
        let body = match screen {
            Screen::Welcome(Stage::Pair) if hero => None,
            Screen::Welcome(Stage::Pair) => Some((Stage::Pair, self.onboarding(&d, pal, window, cx))),
            Screen::Welcome(Stage::Tools) => Some((Stage::Tools, self.tools_step(&d, pal, cx))),
            Screen::Welcome(Stage::TurnOn) => Some((Stage::TurnOn, self.turn_on_step(&d, pal, cx))),
            Screen::Welcome(Stage::Done) => Some((Stage::Done, self.done_step(&d, pal, cx))),
            Screen::Status => None,
        };
        let root = div()
            .id("root")
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::on_key))
            .size_full()
            .flex()
            .flex_col()
            .bg(pal.paper())
            .text_color(pal.ink())
            .font(crate::theme::root_font())
            .text_size(px(13.0))
            .line_height(px(18.0));
        match body {
            Some((stage, body)) => root
                // The macOS title bar is transparent: leave room for the traffic lights.
                .child(div().flex_none().h(px(if cfg!(target_os = "macos") {
                    34.0
                } else {
                    6.0
                })))
                .child(
                    div().id("column").flex_1().overflow_y_scroll().child(
                        // Top-aligned, not centred: the step indicator stays put from step to step, whatever each
                        // step's height.
                        div()
                            .w_full()
                            .min_h_full()
                            .flex()
                            .flex_col()
                            .items_center()
                            .justify_start()
                            .gap(px(28.0))
                            .px(px(40.0))
                            .pt(px(WELCOME_TOP))
                            .pb(px(36.0))
                            .child(parts::step_indicator(stage, pal))
                            .child(div().w_full().max_w(px(stage_width(stage))).child(body)),
                    ),
                ),
            None if hero => root.child(self.welcome_hero(&d, pal, cx)),
            None => root.child(self.status(&d, pal, window, cx)),
        }
    }
}

/// The welcome flow's space above the step indicator.
const WELCOME_TOP: f32 = 64.0;

/// How wide a step of the welcome flow is (`Desk-Setup.png`: a 580 px window, 40 px either side).
fn stage_width(_stage: Stage) -> f32 {
    620.0
}
