//! The window. Before pairing and setup: one calm column (onboarding, then the first-time setup). Afterwards the
//! status window: a sidebar (the state, the sections, pausing) and the section it selects: Overview, Activity,
//! Connections, Keys & secrets, Rules, Settings. Like the phone apps: Geist, cool neutrals, one violet accent.

mod activity;
mod connections;
mod field;
mod keys;
mod onboarding;
mod overview;
mod parts;
mod rules;
mod settings;
mod setup;
mod sidebar;

use std::collections::HashSet;
use std::sync::Arc;

use gpui::{
    AnyElement, App, ClickEvent, Context, Entity, FocusHandle, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, StatefulInteractiveElement as _, Styled as _, Subscription, Window, div, px,
};
use reins_desktop::config::Config;
use reins_desktop::control::Overview;
use reins_desktop::journal::{Entry, Kind, Outcome};

use crate::backend::{Snapshot, Update};
use crate::model::{Model, Screen, Section};
use crate::pause::Pause;
use crate::state::Saved;
use crate::theme::{FONT, Palette};
use crate::tray::Look;

/// The window's first size, and the smallest it can be.
pub const WIDTH: f32 = 1000.0;
pub const HEIGHT: f32 = 680.0;
pub const MIN_WIDTH: f32 = 780.0;
pub const MIN_HEIGHT: f32 = 560.0;

/// How many activity rows show before "Show more".
const ROWS: usize = 60;

pub struct Root {
    model: Entity<Model>,
    server_field: FocusHandle,
    /// The inputs under Rules, one per guard list ([`field::slot`]).
    guard_fields: [FocusHandle; 4],
    guard_inputs: [String; 4],
    /// Activity's filters: one outcome, one kind (`None`: all).
    outcome_filter: Option<Outcome>,
    kind_filter: Option<Kind>,
    /// Activity rows open to their detail.
    expanded: HashSet<String>,
    rows_shown: usize,
    /// The sidebar's pause lengths are showing.
    pause_menu: bool,
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
    section: Section,
    /// Where `config.toml` is, for the empty states that say to edit it.
    config_file: String,
    now: i64,
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
            section: m.section,
            config_file: m.config_file().display().to_string(),
            now: reins_desktop::now_unix(),
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

    /// Requests waiting for the phone now.
    fn waiting(&self) -> impl Iterator<Item = &Entry> {
        self.activity().iter().filter(|e| e.outcome == Outcome::Waiting)
    }
}

impl Root {
    pub fn new(model: Entity<Model>, window: &mut Window, cx: &mut Context<'_, Self>) -> Self {
        let subscriptions = vec![
            cx.observe(&model, |_, _, cx| cx.notify()),
            cx.observe_window_appearance(window, |_, _, cx| cx.notify()),
        ];
        Self {
            model,
            server_field: cx.focus_handle(),
            guard_fields: [cx.focus_handle(), cx.focus_handle(), cx.focus_handle(), cx.focus_handle()],
            guard_inputs: Default::default(),
            outcome_filter: None,
            kind_filter: None,
            expanded: HashSet::new(),
            rows_shown: ROWS,
            pause_menu: false,
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

    /// Goes to `section`.
    fn go(cx: &Context<'_, Self>, section: Section) -> impl Fn(&ClickEvent, &mut Window, &mut App) + 'static {
        Self::on_model(cx, move |m, cx| m.show_section(section, cx))
    }

    /// The status window: sidebar and section.
    fn status(&mut self, d: &Data, pal: Palette, window: &mut Window, cx: &mut Context<'_, Self>) -> AnyElement {
        let page = match d.section {
            Section::Overview => self.overview(d, pal, cx),
            Section::Activity => self.activity(d, pal, cx),
            Section::Connections => Self::connections(d, pal, cx),
            Section::Keys => Self::keys(d, pal, cx),
            Section::Rules => self.rules(d, pal, window, cx),
            Section::Settings => Self::settings(d, pal, cx),
        };
        div()
            .size_full()
            .flex()
            .flex_row()
            .child(self.sidebar(d, pal, cx))
            .child(
                div().id(("content", d.section as usize)).flex_1().min_w(px(0.0)).h_full().overflow_y_scroll().child(
                    div()
                        .w_full()
                        .max_w(px(860.0))
                        .mx_auto()
                        .px(px(36.0))
                        .pt(px(if cfg!(target_os = "macos") {
                            40.0
                        } else {
                            30.0
                        }))
                        .pb(px(40.0))
                        .child(page),
                ),
            )
            .into_any_element()
    }
}

impl Render for Root {
    fn render(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let pal = Palette::of(window);
        let d = Data::of(self.model.read(cx));
        let screen = self.model.read(cx).screen;
        let body = match screen {
            Screen::Onboarding => Some(self.onboarding(pal, window, cx)),
            Screen::Setup => Some(self.setup(pal, cx)),
            Screen::Status => None,
        };
        let root = div()
            .size_full()
            .flex()
            .flex_col()
            .bg(pal.background)
            .text_color(pal.text)
            .font_family(FONT)
            .text_size(px(13.0))
            .line_height(px(18.0));
        match body {
            Some(body) => root
                // The macOS title bar is transparent: leave room for the traffic lights.
                .child(div().flex_none().h(px(if cfg!(target_os = "macos") {
                    34.0
                } else {
                    6.0
                })))
                .child(
                    div()
                        .id("column")
                        .flex_1()
                        .overflow_y_scroll()
                        .child(div().w_full().max_w(px(460.0)).mx_auto().px(px(22.0)).py(px(22.0)).child(body)),
                ),
            None => root.child(self.status(&d, pal, window, cx)),
        }
    }
}
