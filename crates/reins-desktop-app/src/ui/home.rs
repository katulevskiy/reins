//! Home (`DeskNext-Home.png`, `DeskNext-Ask.png`). Calm: the wallpaper behind "Good evening", faded into the page,
//! with "Send a test"; then Today (the day's requests, with how many were approved and denied) and three cards beside
//! it, Health, Work session and This computer, all on frosted panes over the art. While a request waits on the phone it
//! takes the top: a field of its colour (the timer, Hide, who asks, the ask, its target, the QR code to answer on the
//! phone), then what will happen and, for a push, its commits with the one worth a look opened.

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, Context, Div, FontWeight, Hsla, InteractiveElement as _, IntoElement, ParentElement, SharedString,
    Stateful, StatefulInteractiveElement as _, Styled as _, Window, div, px,
};
use reins_desktop::doctor::Level;
use reins_desktop::journal::{Decider, Entry, Outcome, Tally};

use super::icons::{Glyph, glyph};
use super::parts::{FactLook, caption, fact_rows, italic, link, sentence};
use super::sidebar::clock_left;
use super::story::{Ask, Words};
use super::{Data, Root, kit};
use crate::format;
use crate::health::{self, Test};
use crate::model::{Model, Section};
use crate::theme::motion::{self, Entrance, curve, enter};
use crate::theme::{Field, MONO, NextText as _, Palette};
use crate::tray::Look;
use gpui::{Animation, AnimationExt as _};
use kit::Glass;

/// How many requests Today shows.
const RECENT: usize = 5;
/// The page's sides and the space above the title (the boards' `.dt` on Home).
pub const SIDE: f32 = 40.0;
const TOP: f32 = 40.0;
/// The wallpaper's height behind the title, and how far below the title the panes start.
const WALL: f32 = 360.0;
pub const UNDER_TITLE: f32 = 110.0;
/// The cards' column.
const CARDS: f32 = 300.0;
/// Under a waiting request, the two columns sit side by side while each has this much.
const WAITING_COLUMN_MIN: f32 = 300.0;

/// Today's requests (since local midnight).
#[must_use]
pub fn today(entries: &[Entry], now: i64) -> Tally {
    let start = format::start_of_day(now);
    Tally::of(entries.iter().filter(|e| e.at >= start))
}

/// When a row's request was, as the boards write it: "now", "4m", "2h", "Oct 3".
#[must_use]
pub fn short_ago(now: i64, at: i64) -> String {
    let secs = (now - at).max(0);
    match secs {
        0..60 => "now".to_owned(),
        60..3_600 => format!("{}m", secs / 60),
        3_600..86_400 => format!("{}h", secs / 3_600),
        _ => format::date(at),
    }
}

/// "Good morning", "Good afternoon", "Good evening", by the local hour.
#[must_use]
pub fn greeting(now: i64) -> &'static str {
    match (now - format::start_of_day(now)) / 3_600 {
        5..12 => "Good morning",
        12..18 => "Good afternoon",
        _ => "Good evening",
    }
}

/// The line under the greeting.
fn mood_line(d: &Data) -> &'static str {
    match d.look {
        Look::Attention if !d.paired => "This computer is not paired yet.",
        Look::Attention => "Something needs you before Reins can ask.",
        Look::Paused if d.pause.on() => "Pushes skip your phone until you resume.",
        Look::Paused => "git goes straight to the hosts.",
        Look::On | Look::Waiting => "All quiet. Nothing waits on your phone.",
    }
}

/// A short line with its last words at 600: "Fix the **login bug**" (the last two words of three or more, the last
/// of two).
#[must_use]
pub fn italic_tail(text: &str) -> Vec<Words> {
    let words: Vec<&str> = text.split_whitespace().collect();
    let tail = match words.len() {
        0 | 1 => 0,
        2 => 1,
        _ => 2,
    };
    let (head, end) = words.split_at(words.len() - tail);
    let mut out = Vec::new();
    if !head.is_empty() {
        out.push(Words {
            text: format!(
                "{}{}",
                head.join(" "),
                if end.is_empty() {
                    ""
                } else {
                    " "
                }
            ),
            bold: false,
        });
    }
    if !end.is_empty() {
        out.push(Words {
            text: end.join(" "),
            bold: true,
        });
    }
    out
}

/// A small tag at a row's end (`.otag`): a glyph and a word in `color` on itself at 12%.
fn otag(g: Glyph, word: &'static str, color: Hsla) -> Div {
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap(px(3.0))
        .h(px(20.0))
        .px(px(7.0))
        .rounded_full()
        .bg(color.opacity(0.12))
        .text_color(color)
        .text_size(px(11.5))
        .font_weight(FontWeight::SEMIBOLD)
        .whitespace_nowrap()
        .child(glyph(g, 11.0, color))
        .child(word)
}

/// How a row ended, under its time: a check, a cross, or a tag for what no one answered.
fn outcome_mark(e: &Entry, pal: Palette) -> AnyElement {
    match e.outcome {
        Outcome::Approved => glyph(Glyph::Check, 17.0, pal.success).into_any_element(),
        Outcome::Denied | Outcome::Blocked => glyph(Glyph::Close, 17.0, pal.danger).into_any_element(),
        Outcome::Failed => glyph(Glyph::CircleAlert, 17.0, pal.danger).into_any_element(),
        Outcome::TimedOut => glyph(Glyph::TimerOff, 17.0, pal.warning).into_any_element(),
        Outcome::Allowed => otag(Glyph::Shield, "Rules", pal.ink_2()).into_any_element(),
        Outcome::Waiting => otag(Glyph::Phone, "Waiting", pal.violet()).into_any_element(),
    }
}

/// The amber line under a row's sentence (`.risk`), for what did not go as asked.
fn risk_line(e: &Entry, pal: Palette) -> Option<Div> {
    let (text, color) = match (e.outcome, e.decider) {
        (Outcome::TimedOut, _) => ("No answer, so it was denied".to_owned(), pal.warning),
        (Outcome::Failed, _) => (e.reason.clone().unwrap_or_else(|| "Couldn't ask your phone".to_owned()), pal.danger),
        (Outcome::Blocked, _) => ("Blocked by your rules".to_owned(), pal.warning),
        (Outcome::Denied | Outcome::Approved, Decider::Local) => {
            ("Answered on this computer".to_owned(), pal.ink_meta())
        }
        _ => return None,
    };
    Some(
        div()
            .flex()
            .items_center()
            .gap(px(5.0))
            .mt(px(3.0))
            .text_size(px(12.5))
            .font_weight(FontWeight::MEDIUM)
            .text_color(color)
            .min_w(px(0.0))
            .child(glyph(Glyph::Warning, 13.0, color))
            .child(div().min_w(px(0.0)).truncate().child(text)),
    )
}

/// A card's label (`.dlab`: 12.5, 600, `ink-meta`).
fn card_label(text: impl Into<SharedString>, color: Hsla) -> Div {
    div().text_size(px(12.5)).font_weight(FontWeight::SEMIBOLD).text_color(color).child(text.into())
}

/// A card's value (`--s: 34`) with what acts on it at its right.
fn card_value(text: impl Into<SharedString>, action: Option<AnyElement>, color: Hsla) -> Div {
    div()
        .flex()
        .items_end()
        .gap(px(10.0))
        .mt(px(6.0))
        .child(div().flex_1().min_w(px(0.0)).stat().text_color(color).truncate().child(text.into()))
        .when_some(action, ParentElement::child)
}

/// A card's line under its value (13.5, `ink-2`).
fn card_line(text: impl Into<SharedString>, color: Hsla) -> Div {
    div().mt(px(4.0)).text_size(px(13.5)).line_height(px(18.0)).text_color(color).truncate().child(text.into())
}

impl Root {
    /// Home, its own padding: the wallpaper and a waiting field run to the content's edges, under the title bar.
    pub(super) fn home(&mut self, d: &Data, pal: Palette, window: &Window, cx: &mut Context<'_, Self>) -> AnyElement {
        if let Some(e) = d.waiting().last().filter(|e| self.hidden_ask.as_deref() != Some(e.id.as_str())) {
            return self.home_waiting(e, d, pal, cx);
        }
        let glass = Glass::over(true, !d.saved.solid_panes);
        let sending = matches!(d.test, Test::Waiting(_));
        let test = kit::button("hero-test", kit::Look::Soft, kit::Size::Regular, Some(Glyph::Send), "Send a test", pal);
        let test = if d.paired && !sending {
            test.on_click(Self::on_model(cx, Model::send_test))
        } else {
            kit::disabled(test)
        };
        let over_art = |c: Hsla| {
            if pal.dark {
                gpui::white()
            } else {
                c
            }
        };
        let header = div()
            .relative()
            .flex()
            .items_end()
            .gap(px(12.0))
            .px(px(SIDE))
            .pt(px(TOP))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w(px(0.0))
                    .child(div().title().text_color(over_art(pal.ink())).child(greeting(d.now)))
                    .child(
                        div()
                            .mt(px(6.0))
                            .mood()
                            .text_color(if pal.dark {
                                crate::theme::rgba(0x00E4_E4EA, 1.0)
                            } else {
                                pal.ink_2()
                            })
                            .child(mood_line(d)),
                    ),
            )
            .child(test);

        // The left column: what needs saying, then Today and what a card opens.
        let mut left = div().flex().flex_col().gap(px(14.0)).flex_1().min_w(px(360.0));
        if let Some(banner) = Self::update_banner(d, glass, pal, cx) {
            left = left.child(banner);
        }
        if let Some(a) = Self::attention(d, glass, pal, cx) {
            left = left.child(a);
        }
        left = left.when_some(d.notice.clone(), |p, n| p.child(caption(n, pal).text_color(pal.danger)));
        left = left.when_some(d.note.clone(), |p, n| p.child(caption(n, pal)));
        if !matches!(d.test, Test::Idle) {
            left = left.child(kit::pane(Self::test_panel(d, true, pal, cx), glass, kit::pane::RADIUS, pal));
        }
        left = left.child(Self::today_pane(d, glass, pal, cx));
        if self.health_open {
            left = left.child(Self::health_card(d, pal, cx));
        }
        if d.work_session().is_none() && (d.work.asking.is_some() || d.work.form.is_some()) {
            left = left.child(self.work_card(d, pal, window, cx));
        }
        let cards = div()
            .flex()
            .flex_col()
            .gap(px(14.0))
            .flex_1()
            .min_w(px(260.0))
            .max_w(px(CARDS))
            .child(Self::health_tile(d, glass, pal, cx))
            .child(Self::work_tile(d, glass, pal, cx))
            .child(Self::computer_tile(d, glass, pal, cx));
        let content = div()
            .relative()
            .flex()
            .flex_wrap()
            .items_start()
            .gap(px(22.0))
            .px(px(SIDE))
            .pt(px(UNDER_TITLE))
            .pb(px(40.0))
            .child(left)
            .child(cards);
        div()
            .relative()
            .w_full()
            .min_h_full()
            .child(kit::wallpaper::band(d.saved.wallpaper(pal.dark), WALL, pal))
            .child(header)
            .child(content)
            .into_any_element()
    }

    /// Today (`.grp.pf2` on the board): its counts, and the day's requests (the latest ones, when none came today).
    fn today_pane(d: &Data, glass: Glass, pal: Palette, cx: &mut Context<'_, Self>) -> AnyElement {
        let start = format::start_of_day(d.now);
        let todays: Vec<&Entry> =
            d.activity().iter().filter(|e| e.at >= start && e.outcome != Outcome::Waiting).take(RECENT).collect();
        let (label, rows): (&str, Vec<&Entry>) = if todays.is_empty() {
            ("Latest", d.activity().iter().filter(|e| e.outcome != Outcome::Waiting).take(RECENT).collect())
        } else {
            ("Today", todays)
        };
        let t = today(d.activity(), d.now);
        let count = |g: Glyph, n: usize, color: Hsla| {
            div().flex().items_center().gap(px(3.0)).text_color(color).child(glyph(g, 13.0, color)).child(n.to_string())
        };
        let counts = div()
            .flex()
            .items_center()
            .gap(px(10.0))
            .when(t.approved + t.allowed > 0, |c| c.child(count(Glyph::Check, t.approved + t.allowed, pal.success)))
            .when(t.denied + t.blocked > 0, |c| c.child(count(Glyph::Close, t.denied + t.blocked, pal.danger)))
            .when(t.timed_out > 0, |c| c.child(count(Glyph::TimerOff, t.timed_out, pal.warning)))
            .child(glyph(Glyph::ChevronRight, 14.0, pal.ink_meta()));
        let head = div()
            .id("today-head")
            .flex()
            .items_center()
            .gap(px(6.0))
            .px(px(14.0))
            .pt(px(12.0))
            .pb(px(4.0))
            .label()
            .text_color(pal.ink_2())
            .cursor_pointer()
            .child(div().flex_1().child(label))
            .child(counts)
            .on_click(Self::go(cx, Section::Activity));
        let mut items: Vec<AnyElement> = Vec::new();
        let surface = Self::row_surface(glass, pal);
        for e in &rows {
            items.push(
                Self::entry_row(e, "home", d, pal, false, surface)
                    .on_click(Self::open_entry(cx, e.id.clone()))
                    .into_any_element(),
            );
        }
        if items.is_empty() {
            items.push(
                div()
                    .px(px(14.0))
                    .py(px(16.0))
                    .text_size(px(14.5))
                    .text_color(pal.ink_meta())
                    .child("Nothing asked yet.")
                    .into_any_element(),
            );
        }
        kit::pane(
            div().flex().flex_col().pb(px(4.0)).child(head).children(kit::pane::with_rules(items, 62.0, glass, pal)),
            glass,
            kit::pane::RADIUS,
            pal,
        )
    }

    /// What a request icon's badge is ringed in, on a pane of `glass`.
    pub(super) fn row_surface(glass: Glass, pal: Palette) -> Hsla {
        match glass {
            Glass::Solid => pal.paper_raised(),
            Glass::Frosted if pal.dark => crate::theme::rgba(0x0024_2429, 1.0),
            Glass::Frosted => crate::theme::rgba(0x00F4_F4F6, 1.0),
        }
    }

    /// One request as a row (`.r`): its icon, its sentence with the target at 600 (and a line when it did not go as
    /// asked), when, and how it ended. `selected`: Activity's picked row, washed in violet.
    pub(super) fn entry_row(
        e: &Entry,
        scope: &'static str,
        d: &Data,
        pal: Palette,
        selected: bool,
        surface: Hsla,
    ) -> Stateful<Div> {
        let s = Ask::of(e);
        let when = if e.at >= format::start_of_day(d.now) {
            short_ago(d.now, e.at)
        } else {
            format::clock(e.at)
        };
        kit::pane_row()
            .id(SharedString::from(format!("{scope}-{}", e.id)))
            .min_h(px(56.0))
            .cursor_pointer()
            .when_else(
                selected,
                |r| {
                    r.bg(pal.violet().opacity(if pal.dark {
                        0.16
                    } else {
                        0.10
                    }))
                },
                |r| r.hover(|h| h.bg(pal.ink().opacity(0.035))),
            )
            .child(div().mr(px(5.0)).mb(px(5.0)).child(kit::request_icon(s.mark, s.badge, s.field, 36.0, surface, pal)))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w(px(0.0))
                    .child(
                        div()
                            .text_size(px(15.0))
                            .line_height(px(20.0))
                            .text_color(pal.ink())
                            .line_clamp(3)
                            .child(sentence(&s.sentence, pal)),
                    )
                    .when_some(risk_line(e, pal), ParentElement::child),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_none()
                    .items_end()
                    .gap(px(4.0))
                    .min_w(px(34.0))
                    .child(
                        div()
                            .text_size(px(12.5))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(pal.ink_meta())
                            .whitespace_nowrap()
                            .child(when),
                    )
                    .child(outcome_mark(e, pal)),
            )
    }

    /// A card on the right (`.grp.pf2`, 16 × 18 inside): its label, its value, its line.
    fn card(content: Div, glass: Glass, pal: Palette) -> AnyElement {
        kit::pane(content.px(px(18.0)).py(px(16.0)), glass, kit::pane::RADIUS, pal)
    }

    /// Health: all good, or how many things to look at with the first one's fix. A click opens the checks.
    fn health_tile(d: &Data, glass: Glass, pal: Palette, cx: &mut Context<'_, Self>) -> AnyElement {
        let checks = d.health.checks.as_deref();
        let (level, value, line) = match (checks, &d.health.error) {
            (_, Some(e)) => (Level::Fail, "Can't check".to_owned(), e.clone()),
            (Some(c), None) => {
                let s = health::summary(c);
                let first = health::to_look_at(c).first().map(|c| format!("{} · {}", c.label, c.detail));
                let ago = d.health.at.map(|at| format::ago(d.now, at));
                let line = first.unwrap_or_else(|| match (s.detail.is_empty(), ago) {
                    (false, Some(ago)) => format!("{} · {ago}", s.detail),
                    (false, None) => s.detail.clone(),
                    (true, Some(ago)) => format!("Checked {ago}"),
                    (true, None) => String::new(),
                });
                let value = match s.level {
                    Level::Ok => "All good".to_owned(),
                    _ => format!("{} to look at", health::to_look_at(c).len()),
                };
                (s.level, value, line)
            }
            (None, None) => (Level::Skip, "Checking…".to_owned(), String::new()),
        };
        let fix = checks.and_then(|c| health::to_look_at(c).first().and_then(|c| health::fix_for(c)));
        let fix = fix.map(|fix| {
            kit::button("tile-fix", kit::Look::Soft, kit::Size::Small, None, fix.label(), pal)
                .on_click(cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    this.model.update(cx, |m, cx| m.fix(fix, cx));
                }))
                .into_any_element()
        });
        let line_color = match level {
            Level::Fail => pal.danger,
            Level::Warn => pal.warning,
            _ => pal.ink_2(),
        };
        let content = div()
            .flex()
            .flex_col()
            .child(card_label("Health", pal.ink_meta()))
            .child(card_value(value, fix, pal.ink()))
            .child(card_line(line, line_color));
        div()
            .id("tile-health")
            .cursor_pointer()
            .active(|s| s.opacity(kit::button::PRESSED))
            .child(Self::card(content, glass, pal))
            .on_click(Self::on_view(cx, |this, _| this.health_open = !this.health_open))
            .into_any_element()
    }

    /// Work session: Start; the request on the phone; or, running, a violet field (a session is a bundle of grants).
    fn work_tile(d: &Data, glass: Glass, pal: Palette, cx: &mut Context<'_, Self>) -> AnyElement {
        if let Some(session) = d.work_session() {
            let on = pal.on_field();
            let action: AnyElement = if d.work.ending {
                kit::disabled(kit::button("tile-ending", kit::Look::Wash, kit::Size::Small, None, "Ending…", pal))
                    .into_any_element()
            } else if d.work.confirm_end {
                div()
                    .flex()
                    .gap(px(6.0))
                    .child(
                        kit::button("tile-keep", kit::Look::Wash, kit::Size::Small, None, "Keep", pal)
                            .on_click(Self::on_model(cx, Model::keep_work_session)),
                    )
                    .child(
                        kit::button("tile-end", kit::Look::OnField, kit::Size::Small, None, "End it", pal)
                            .on_click(Self::on_model(cx, Model::end_work_session)),
                    )
                    .into_any_element()
            } else {
                kit::button("tile-end-now", kit::Look::Wash, kit::Size::Small, None, "End now", pal)
                    .on_click(Self::on_model(cx, Model::confirm_end_work_session))
                    .into_any_element()
            };
            return kit::field_surface(Field::Grant, kit::pane::RADIUS, kit::pane::RADIUS)
                .flex()
                .flex_col()
                .px(px(18.0))
                .py(px(16.0))
                .text_color(on)
                .child(div().relative().child(card_label("Work session", pal.on_field_2())))
                .child(
                    div().relative().child(
                        div()
                            .flex()
                            .items_end()
                            .gap(px(10.0))
                            .mt(px(6.0))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w(px(0.0))
                                    .stat()
                                    .truncate()
                                    .child(italic(&italic_tail(&session.reason))),
                            )
                            .child(action),
                    ),
                )
                .child(div().relative().child(card_line(
                    format!("{} left · until {}", crate::work::left(session, d.now), format::clock(session.expires_at)),
                    pal.on_field_2(),
                )))
                .into_any_element();
        }
        if let Some(asking) = &d.work.asking {
            let left = i64::try_from(asking.left(std::time::Instant::now())).unwrap_or(0);
            let clock = div()
                .font_family(MONO)
                .text_size(px(13.5))
                .text_color(pal.violet())
                .child(clock_left(left))
                .into_any_element();
            return Self::card(
                div()
                    .flex()
                    .flex_col()
                    .child(card_label("Work session", pal.ink_meta()))
                    .child(card_value("On your phone", Some(clock), pal.ink()))
                    .child(card_line(asking.request.reason.clone(), pal.ink_2())),
                glass,
                pal,
            );
        }
        let open = d.work.form.is_some();
        let start = kit::button(
            "session-start",
            kit::Look::Soft,
            kit::Size::Regular,
            None,
            if open {
                "Cancel"
            } else {
                "Start"
            },
            pal,
        );
        let start = if !d.paired {
            kit::disabled(start)
        } else if open {
            start.on_click(Self::on_model(cx, Model::close_work_form))
        } else {
            start.on_click(Self::on_model(cx, Model::open_work_form))
        };
        let (value, line, line_color) = match d.work.failed.clone() {
            Some(why) => ("Not started", why, pal.danger),
            None => ("Not running", "Approve once, then work freely".to_owned(), pal.ink_2()),
        };
        Self::card(
            div()
                .flex()
                .flex_col()
                .child(card_label("Work session", pal.ink_meta()))
                .child(card_value(value, Some(start.into_any_element()), pal.ink()))
                .child(card_line(line, line_color)),
            glass,
            pal,
        )
    }

    /// This computer: how many AI tools go through Reins, and what else does. A click opens Connections.
    fn computer_tile(d: &Data, glass: Glass, pal: Palette, cx: &mut Context<'_, Self>) -> AnyElement {
        let tools = d.snapshot.as_ref().map_or(0, |s| s.harnesses.iter().filter(|h| h.added).count());
        let git = d.snapshot.as_ref().is_some_and(|s| !s.git_routed.is_empty());
        let ssh = d.config().is_some_and(|c| c.ssh.enabled);
        let line = match (git, ssh) {
            (true, true) => "git and ssh go through Reins",
            (true, false) => "git goes through Reins",
            (false, true) => "ssh goes through Reins",
            (false, false) => "git goes straight to the hosts",
        };
        div()
            .id("tile-computer")
            .cursor_pointer()
            .active(|s| s.opacity(kit::button::PRESSED))
            .child(Self::card(
                div()
                    .flex()
                    .flex_col()
                    .child(card_label("This computer", pal.ink_meta()))
                    .child(card_value(format::count(tools as u64, "AI tool", "AI tools"), None, pal.ink()))
                    .child(card_line(line, pal.ink_2())),
                glass,
                pal,
            ))
            .on_click(Self::go(cx, Section::Connections))
            .into_any_element()
    }

    /// What stops Reins, as a pane with the button that fixes it: not paired, the service down, git paused.
    fn attention(d: &Data, glass: Glass, pal: Palette, cx: &mut Context<'_, Self>) -> Option<AnyElement> {
        let (g, color, title, line, action): (Glyph, Hsla, &str, String, Option<AnyElement>) = match d.look {
            Look::Attention if !d.paired => (
                Glyph::Unlink,
                pal.warning,
                "Not paired",
                "Pair this computer with your phone".to_owned(),
                Some(
                    kit::button(
                        "hero-pair",
                        kit::Look::Inverse,
                        kit::Size::Regular,
                        Some(Glyph::Phone),
                        "Pair again",
                        pal,
                    )
                    .on_click(Self::on_model(cx, Model::sign_out))
                    .into_any_element(),
                ),
            ),
            Look::Attention => (
                Glyph::Warning,
                pal.warning,
                "The background service isn't running",
                d.line.clone(),
                Some(
                    kit::button("hero-start", kit::Look::Inverse, kit::Size::Regular, Some(Glyph::Play), "Start", pal)
                        .on_click(Self::on_model(cx, Model::start_service))
                        .into_any_element(),
                ),
            ),
            Look::Paused => (
                Glyph::Pause,
                pal.danger,
                "git goes straight to the hosts",
                d.pause.short(d.now).unwrap_or_else(|| "Turn hosts on in Connections".to_owned()),
                d.pause.on().then(|| {
                    kit::button("hero-resume", kit::Look::Inverse, kit::Size::Regular, Some(Glyph::Play), "Resume", pal)
                        .on_click(Self::on_model(cx, Model::resume))
                        .into_any_element()
                }),
            ),
            Look::On | Look::Waiting => return None,
        };
        let row = kit::pane_row()
            .min_h(px(64.0))
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .size(px(34.0))
                    .rounded(px(10.0))
                    .bg(color.opacity(0.14))
                    .child(glyph(g, 17.0, color)),
            )
            .child(kit::title_sub(title.to_owned(), Some(div().child(line).into_any_element()), pal))
            .when_some(action, ParentElement::child);
        Some(kit::pane(row, glass, kit::pane::RADIUS, pal))
    }

    /// Home while a request waits: its field, then what will happen and, for a push, its commits and the one opened.
    fn home_waiting(&mut self, e: &Entry, d: &Data, pal: Palette, cx: &mut Context<'_, Self>) -> AnyElement {
        let s = Ask::of(e);
        // A push's commits, files and diffs, as the desktop app read them from the pack it holds.
        self.load_push(e, cx);
        let push = self.kept_for(e);
        let glass = Glass::Solid;
        let field = Self::ask_field(e, d, pal, cx);

        let what = div().flex().flex_col().child(kit::section(None, "What will happen", None, pal)).child(fact_rows(
            &s.facts,
            FactLook::Home,
            glass,
            pal,
        ));
        let (left, right): (Div, AnyElement) = match push {
            Some(push) => {
                let ci = self.picked_commit(&push);
                (
                    what.child(Self::commit_list(&push, ci, glass, pal, cx)),
                    self.commit_detail(&push, ci, false, pal, cx).into_any_element(),
                )
            }
            None => (
                what,
                div()
                    .flex()
                    .flex_col()
                    .child(kit::section(None, "Today", None, pal))
                    .child(Self::today_list(d, glass, pal, cx))
                    .into_any_element(),
            ),
        };
        let mut below = div().flex().flex_col().gap(px(14.0));
        below = below.when_some(d.notice.clone(), |p, n| p.child(caption(n, pal).text_color(pal.danger)));
        below = below.when_some(d.note.clone(), |p, n| p.child(caption(n, pal)));
        below = below.child(
            div()
                .flex()
                .flex_wrap()
                .items_start()
                .gap(px(28.0))
                .child(left.flex_1().min_w(px(WAITING_COLUMN_MIN)))
                .child(div().flex_1().min_w(px(WAITING_COLUMN_MIN)).child(right)),
        );
        div()
            .w_full()
            .flex()
            .flex_col()
            .child(field)
            .child(div().px(px(SIDE)).pt(px(6.0)).pb(px(40.0)).child(below))
            .into_any_element()
    }

    /// The latest requests as a pane (beside a waiting request that is not a push).
    fn today_list(d: &Data, glass: Glass, pal: Palette, cx: &mut Context<'_, Self>) -> AnyElement {
        let surface = Self::row_surface(glass, pal);
        let rows: Vec<AnyElement> = d
            .activity()
            .iter()
            .filter(|e| e.outcome != Outcome::Waiting)
            .take(3)
            .map(|e| {
                Self::entry_row(e, "waiting", d, pal, false, surface)
                    .on_click(Self::open_entry(cx, e.id.clone()))
                    .into_any_element()
            })
            .collect();
        kit::group(rows, 62.0, glass, pal)
    }

    /// The ask (`.fld` at the top of the page): the timer and Hide, who asks, the sentence with its target, the
    /// machine's details in mono, and the QR code that opens it on the phone (this computer cannot approve).
    pub(super) fn ask_field(e: &Entry, d: &Data, pal: Palette, cx: &mut Context<'_, Self>) -> AnyElement {
        let s = Ask::of(e);
        let on = pal.on_field();
        let (total, left) = d.wait_left(e);
        let more = d.waiting().count().saturating_sub(1);
        let top = div()
            .relative()
            .flex()
            .items_center()
            .gap(px(10.0))
            .child(kit::timer(left, i64::try_from(total).unwrap_or(i64::MAX), on))
            .when(more > 0, |t| {
                t.child(div().text_size(px(13.5)).text_color(pal.on_field_2()).child(format!("+{more} more waiting")))
            })
            .child(div().flex_1())
            .child({
                let id = e.id.clone();
                div()
                    .id("ask-hide")
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .size(px(32.0))
                    .rounded_full()
                    .bg(pal.on_field_wash())
                    .cursor_pointer()
                    .active(|s| s.opacity(kit::button::PRESSED))
                    .child(glyph(Glyph::Close, 18.0, on))
                    .on_click(Self::on_view(cx, move |this, _| this.hidden_ask = Some(id.clone())))
            });
        let agent = kit::pane::avatar(s.agent, 22.0, pal);
        // Who asks is the requester's own word: its own run, cleaned, on one line and capped, never merged into a
        // sentence of Reins'; the folder it asks from beside it in mono.
        let who = super::story::middle(&super::story::clean(&s.who), 40);
        let dir = s.dir.as_deref().filter(|d| !d.is_empty()).map(|d| super::story::middle(&super::story::clean(d), 48));
        let service = div()
            .flex()
            .items_center()
            .gap(px(8.0))
            .mt(px(10.0))
            .min_w(px(0.0))
            .text_size(px(15.0))
            .font_weight(FontWeight::SEMIBOLD)
            .child(div().mr(px(4.0)).mb(px(4.0)).child(kit::request_icon(
                s.mark,
                s.badge,
                s.field,
                28.0,
                s.field.field(),
                pal,
            )))
            .child(div().opacity(0.6).child("·"))
            .child(agent)
            .child(div().flex_shrink(1.0).min_w(px(0.0)).truncate().whitespace_nowrap().child(who))
            .child(match dir {
                Some(dir) => div()
                    .min_w(px(0.0))
                    .truncate()
                    .whitespace_nowrap()
                    .font_family(MONO)
                    .text_size(px(13.5))
                    .font_weight(FontWeight::NORMAL)
                    .text_color(on.opacity(0.85))
                    .child(dir),
                None => div()
                    .flex_none()
                    .font_weight(FontWeight::NORMAL)
                    .text_color(on.opacity(0.85))
                    .child(format!("on {}", super::parts::this_computer())),
            });
        // The ask follows the field by 60 ms.
        let ask = div().mt(px(8.0)).ask_xl().text_color(on).child(italic(&s.order));
        let ask: AnyElement = if motion::reduce_motion() {
            ask.into_any_element()
        } else {
            let (delay, run) = (motion::ASK_DELAY.as_secs_f32(), motion::FIELD_IN.as_secs_f32());
            let ease = curve(motion::EASE_OUT);
            ask.with_animation(
                SharedString::from(format!("ask-{}", e.id)),
                Animation::new(motion::ASK_DELAY + motion::FIELD_IN),
                move |el, t| el.opacity(ease(((t * (delay + run) - delay) / run).clamp(0.0, 1.0))),
            )
            .into_any_element()
        };
        let words = div()
            .relative()
            .flex()
            .flex_col()
            .flex_1()
            .min_w(px(300.0))
            .child(service)
            .child(ask)
            // The machine's details on their own mono line.
            .when_some(s.details.clone(), |c, details| {
                c.child(
                    div()
                        .mt(px(8.0))
                        .font_family(MONO)
                        .text_size(px(14.0))
                        .line_height(px(20.0))
                        .text_color(on.opacity(0.85))
                        .child(super::parts::wrapped(&details)),
                )
            });
        let qr = div()
            .relative()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(16.0))
            .p(px(16.0))
            .rounded(px(20.0))
            .bg(gpui::white().opacity(0.14))
            .child(div().flex_none().p(px(8.0)).rounded(px(10.0)).bg(gpui::white()).child(crate::qr::element(
                crate::links::PHONE_ACTIVITY,
                104.0,
                crate::theme::rgba(0x0011_1111, 1.0),
                gpui::white(),
            )))
            .child(
                div()
                    .w(px(170.0))
                    .child(
                        div()
                            .text_size(px(15.0))
                            .line_height(px(20.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(format!("Answer on {}", d.phone())),
                    )
                    .child(
                        div()
                            .mt(px(4.0))
                            .text_size(px(13.5))
                            .line_height(px(18.0))
                            .text_color(on.opacity(0.82))
                            .child(format!("{} can't approve. Scan to open it there.", this_computer_cap())),
                    ),
            );
        let top_pad = if cfg!(target_os = "macos") {
            30.0
        } else {
            22.0
        };
        let surface = kit::field_surface(s.field, 0.0, 28.0)
            .flex()
            .flex_col()
            .px(px(SIDE))
            .pt(px(top_pad))
            .pb(px(28.0))
            .text_color(on)
            .child(top)
            .child(div().relative().flex().flex_wrap().items_end().gap(px(30.0)).child(words).child(qr));
        enter(surface, SharedString::from(format!("field-{}", e.id)), Entrance::Field).into_any_element()
    }

    /// A new version: ready to restart into, or out to download.
    fn update_banner(d: &Data, glass: Glass, pal: Palette, cx: &mut Context<'_, Self>) -> Option<AnyElement> {
        let update = d.update.as_ref()?;
        let (title, action): (String, AnyElement) = if update.ready.is_some() {
            let label = if d.updating {
                "Restarting…"
            } else {
                "Restart to update"
            };
            let restart = kit::button("restart-update", kit::Look::Inverse, kit::Size::Regular, None, label, pal);
            (
                format!("Reins {} is ready", update.version),
                if d.updating {
                    kit::disabled(restart)
                } else {
                    restart.on_click(Self::on_model(cx, Model::restart_to_update))
                }
                .into_any_element(),
            )
        } else {
            (
                format!("Reins {} is out", update.version),
                link("download", "Download", pal)
                    .on_click(|_, _, cx| cx.open_url(&crate::links::download()))
                    .into_any_element(),
            )
        };
        let row = kit::pane_row()
            .child(kit::pane::tile(Glyph::Download, pal))
            .child(kit::title_sub(title, None, pal))
            .child(action);
        Some(kit::pane(row, glass, kit::pane::RADIUS, pal))
    }
}

/// "This Mac", "This computer".
fn this_computer_cap() -> &'static str {
    if cfg!(target_os = "macos") {
        "This Mac"
    } else {
        "This computer"
    }
}

#[cfg(test)]
mod tests {
    use reins_desktop::journal::{Kind, Outcome};

    use super::*;

    #[test]
    fn today_counts_only_since_midnight() {
        // Six hours into a local day, whatever the time zone.
        let now = format::start_of_day(1_800_000_000) + 6 * 3_600;
        let mut old = Entry::new(Kind::Git, "old");
        old.at = format::start_of_day(now) - 10;
        old.outcome = Outcome::Approved;
        let mut new = Entry::new(Kind::Git, "new");
        new.at = now - 5;
        new.outcome = Outcome::Approved;
        let mut denied = new.clone();
        denied.outcome = Outcome::Denied;
        let t = today(&[new, denied, old], now);
        assert_eq!((t.approved, t.denied), (1, 1));
    }

    #[test]
    fn rows_say_when_in_few_words() {
        let now = 1_800_000_000;
        assert_eq!(short_ago(now, now - 10), "now");
        assert_eq!(short_ago(now, now - 4 * 60), "4m");
        assert_eq!(short_ago(now, now - 2 * 3_600), "2h");
        assert_eq!(short_ago(now, now - 3 * 86_400), format::date(now - 3 * 86_400));
    }

    #[test]
    fn the_greeting_follows_the_hour() {
        let midnight = format::start_of_day(1_800_000_000);
        assert_eq!(greeting(midnight + 8 * 3_600), "Good morning");
        assert_eq!(greeting(midnight + 14 * 3_600), "Good afternoon");
        assert_eq!(greeting(midnight + 21 * 3_600), "Good evening");
        assert_eq!(greeting(midnight + 2 * 3_600), "Good evening");
    }

    #[test]
    fn the_last_words_go_bold() {
        let said = |text: &str| {
            italic_tail(text)
                .iter()
                .map(|w| {
                    if w.bold {
                        format!("*{}*", w.text)
                    } else {
                        w.text.clone()
                    }
                })
                .collect::<String>()
        };
        assert_eq!(said("Fix the login bug"), "Fix the *login bug*");
        assert_eq!(said("Deploy staging"), "Deploy *staging*");
        assert_eq!(said("Refactor"), "Refactor");
        assert_eq!(said(""), "");
    }
}
