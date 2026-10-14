//! The window. Before pairing and setup: the welcome flow, one calm column under its steps (pair, AI tools, turn on,
//! done). Afterwards the status window: a sidebar (what Reins is doing, the sections, pausing) and the section it
//! selects: Overview, Activity, Connections, Keys & secrets, Rules, Settings. Like the phone apps: Geist, cool
//! neutrals, one violet accent, coloured icon squares, Lucide icons and real logos. Keyboard shortcuts
//! ([`crate::shortcuts`]) work everywhere in it.

mod activity;
mod connections;
mod field;
mod health;
mod hold;
pub mod icons;
mod keys;
mod onboarding;
mod overview;
mod parts;
mod rules;
mod settings;
mod setup;
mod sidebar;
mod story;
mod work;

use std::sync::Arc;
use std::time::Duration;

use gpui::{
    AnyElement, App, ClickEvent, Context, Entity, FocusHandle, InteractiveElement as _, IntoElement, KeyDownEvent,
    ParentElement as _, Render, StatefulInteractiveElement as _, Styled as _, Subscription, Task, Window, div, px,
};
use reins_desktop::config::Config;
use reins_desktop::control::Overview;
use reins_desktop::journal::{Entry, Kind, Outcome};

use crate::backend::{Snapshot, Update};
use crate::health::{Health, Test};
use crate::model::{Model, Screen, Section};
use crate::pause::{Pause, PauseFor};
use crate::shortcuts::{self, Shortcut};
use crate::state::Saved;
use crate::theme::{FONT, Palette};
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

#[allow(clippy::struct_excessive_bools, reason = "independent things open in the window")]
pub struct Root {
    model: Entity<Model>,
    /// The window's own focus, so the keyboard shortcuts work before anything else is focused.
    focus: FocusHandle,
    server_field: FocusHandle,
    /// The inputs under Rules, one per guard list ([`field::slot`]).
    guard_fields: [FocusHandle; 4],
    guard_inputs: [String; 4],
    /// The work session form's fields: what it is for, and a branch for each repository (made as rows appear).
    session_reason: FocusHandle,
    branch_fields: Vec<FocusHandle>,
    /// Activity's filters: one outcome, one kind (`None`: all), and whether their popover is open.
    outcome_filter: Option<Outcome>,
    kind_filter: Option<Kind>,
    filter_open: bool,
    /// The request Activity shows on the right (by journal id; `None`: the first one).
    selected: Option<String>,
    rows_shown: usize,
    /// The "Pause git" sheet is open, and the length picked in it.
    pause_sheet: bool,
    pause_choice: PauseFor,
    /// A hold to confirm in progress ([`hold`]), and how many presses there were (each its own fill).
    hold: Option<hold::Hold>,
    presses: u64,
    /// Overview's Health tile is open to its checks.
    health_open: bool,
    /// Settings' Notifications row is open to its two switches.
    notify_open: bool,
    /// The run profile open to its variables (Keys & secrets).
    profile_open: Option<String>,
    /// The (?) open now ([`parts::help`]), and the page it is on: another page closes it.
    help: Option<&'static str>,
    help_page: (Screen, Section),
    /// Renders once a second while something counts down in seconds (a request's timer, a hold).
    tick: Option<Task<()>>,
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

    /// The mark of the git host when there is one (a force-push's target).
    fn git_host_mark(&self) -> Option<icons::Brand> {
        let s = self.snapshot.as_ref()?;
        match s.git_routed.as_slice() {
            [one] => icons::Brand::of_host(one),
            _ => None,
        }
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
        // `--demo` with `REINS_DEMO_HELP=<id>`: that (?) open, for screenshots.
        let help = std::env::var("REINS_DEMO_HELP")
            .ok()
            .filter(|h| demo && !h.is_empty())
            .map(|h| &*Box::leak(h.into_boxed_str()));
        Self {
            model,
            focus,
            server_field: cx.focus_handle(),
            guard_fields: [cx.focus_handle(), cx.focus_handle(), cx.focus_handle(), cx.focus_handle()],
            guard_inputs: Default::default(),
            session_reason: cx.focus_handle(),
            branch_fields: Vec::new(),
            outcome_filter: None,
            kind_filter: None,
            filter_open: false,
            selected: None,
            rows_shown: ROWS,
            pause_sheet: false,
            pause_choice: PauseFor::Hour,
            hold: None,
            presses: 0,
            health_open: false,
            notify_open: false,
            profile_open: None,
            help,
            help_page,
            tick: None,
            _subscriptions: subscriptions,
        }
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
                let open = self.pause_sheet || self.filter_open || self.help.is_some() || self.hold.is_some();
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
        let top = if cfg!(target_os = "macos") {
            40.0
        } else {
            30.0
        };
        let content: AnyElement = if d.section == Section::Activity {
            // List and detail side by side, each scrolling on its own.
            self.activity(d, pal, top, cx)
        } else {
            let page = match d.section {
                Section::Overview | Section::Activity => self.overview(d, pal, window, cx),
                Section::Connections => Self::connections(d, pal, cx),
                Section::Keys => self.keys(d, pal, cx),
                Section::Rules => self.rules(d, pal, window, cx),
                Section::Settings => self.settings(d, pal, cx),
            };
            div()
                .id(("content", d.section as usize))
                .size_full()
                .overflow_y_scroll()
                .child(div().w_full().max_w(px(900.0)).mx_auto().px(px(34.0)).pt(px(top)).pb(px(34.0)).child(page))
                .into_any_element()
        };
        let mut root = div()
            .relative()
            .size_full()
            .flex()
            .flex_row()
            .child(Self::sidebar(d, pal, cx))
            .child(div().flex_1().min_w(px(0.0)).h_full().child(content));
        if self.pause_sheet && d.paired {
            root = root.child(self.pause_sheet(d, pal, cx));
        }
        root.into_any_element()
    }
}

impl Render for Root {
    fn render(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let pal = Palette::of(window);
        let screen = self.model.read(cx).screen;
        let page = (screen, self.model.read(cx).section);
        if page != self.help_page {
            self.help_page = page;
            self.help = None;
            self.filter_open = false;
        }
        let mut d = Data::of(self.model.read(cx));
        d.help = self.help;
        // Seconds count while a request waits, a work session is being asked for, or the test waits.
        self.keep_ticking(
            matches!(d.test, Test::Waiting(_))
                || (screen == Screen::Status && (d.waiting().next().is_some() || d.work.asking.is_some())),
            window,
            cx,
        );
        let body = match screen {
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
            .bg(pal.background)
            .text_color(pal.text)
            .font_family(FONT)
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
                            .px(px(32.0))
                            .pt(px(WELCOME_TOP))
                            .pb(px(28.0))
                            .child(parts::step_indicator(stage, pal))
                            .child(div().w_full().max_w(px(stage_width(stage))).child(body)),
                    ),
                ),
            None => root.child(self.status(&d, pal, window, cx)),
        }
    }
}

/// The welcome flow's space above the step indicator.
const WELCOME_TOP: f32 = 40.0;

/// How wide a step of the welcome flow is: the pairing has the QR code beside the steps.
fn stage_width(stage: Stage) -> f32 {
    match stage {
        Stage::Pair => 660.0,
        Stage::Tools | Stage::TurnOn | Stage::Done => 560.0,
    }
}
