//! Home (`Desk-Home.png`, `Desk-Waiting.png`). Calm: "Good evening" and the mood line with "Send a test", the plate
//! that says what Reins does for this computer, three plates (Health, Work session, Today), what a plate opens (the
//! checks, the work session form) and the latest requests. While a request waits on the phone, it is the screen: a
//! field of its colour across the top, under the title bar (who asks, the label line, the ask with its target in
//! italic, where to answer and the time left, the QR code to open it on the phone), then what will happen and the
//! latest requests.

use gpui::prelude::FluentBuilder as _;
use gpui::{
    Animation, AnimationExt as _, AnyElement, Context, Div, FontWeight, InteractiveElement as _, IntoElement,
    ParentElement, SharedString, Stateful, StatefulInteractiveElement as _, Styled as _, Window, div, px,
};
use reins_desktop::doctor::Level;
use reins_desktop::journal::{Entry, Outcome, Tally};

use super::icons::{Glyph, Mark, glyph};
use super::parts::{FactLook, caption, dots, fact_rows, italic, link, mood, sentence};
use super::sidebar::clock_left;
use super::story::{Ask, Words, who_from};
use super::{Data, Root, kit};
use crate::format;
use crate::health::{self, Test};
use crate::model::{Model, Section};
use crate::theme::motion::{self, Entrance, curve, enter};
use crate::theme::{Field, MONO, Palette, SERIF, SignalText as _};
use crate::tray::Look;

/// How many recent requests Home shows (calm), and beside a waiting request.
const RECENT: usize = 5;
const RECENT_WAITING: usize = 3;
/// The page's sides (the boards' `.main`).
pub const SIDE: f32 = 44.0;
/// Under a waiting request, "What will happen" and the latest requests sit side by side while each has this much.
const WAITING_COLUMN_MIN: f32 = 300.0;

/// Today's requests (since local midnight).
#[must_use]
pub fn today(entries: &[Entry], now: i64) -> Tally {
    let start = format::start_of_day(now);
    Tally::of(entries.iter().filter(|e| e.at >= start))
}

/// When a row's request was: "now", "4 min", "2 h", "Oct 3".
#[must_use]
pub fn short_ago(now: i64, at: i64) -> String {
    let secs = (now - at).max(0);
    match secs {
        0..60 => "now".to_owned(),
        60..3_600 => format!("{} min", secs / 60),
        3_600..86_400 => format!("{} h", secs / 3_600),
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

/// The mood line under the greeting.
fn mood_line(d: &Data) -> &'static str {
    match d.look {
        Look::Attention if !d.paired => "This computer is not paired yet.",
        Look::Attention => "Something needs you before Reins can ask.",
        Look::Paused if d.pause.on() => "Pushes skip your phone until you resume.",
        Look::Paused => "git goes straight to the hosts.",
        Look::On | Look::Waiting => "All quiet. Nothing waits on your phone.",
    }
}

/// A short serif line with its last words in italic: "Fix the *login bug*" (the last two words of three or more,
/// the last of two).
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

/// How a row ends: a word for every outcome but an approval (a check).
fn ending(e: &Entry, pal: Palette) -> AnyElement {
    let word = |w: &'static str, color| {
        div().flex_none().text_size(px(12.0)).font_weight(FontWeight::SEMIBOLD).text_color(color).child(w)
    };
    match e.outcome {
        Outcome::Approved => glyph(Glyph::Check, 16.0, pal.success).into_any_element(),
        Outcome::Denied => word("Denied", pal.danger).into_any_element(),
        Outcome::Blocked => word("Blocked", pal.danger).into_any_element(),
        Outcome::Failed => word("Failed", pal.danger).into_any_element(),
        Outcome::TimedOut => word("No answer", pal.warning).into_any_element(),
        Outcome::Allowed => word("By rules", pal.ink_meta()).into_any_element(),
        Outcome::Waiting => word("Waiting", pal.violet()).into_any_element(),
    }
}

/// A plate's label, value (serif 30) and the line under it.
fn plate_value(text: impl Into<SharedString>) -> Div {
    div()
        .mt(px(10.0))
        .font_family(SERIF)
        .text_size(px(30.0))
        .line_height(px(34.0))
        .min_w(px(0.0))
        .truncate()
        .child(text.into())
}

fn plate_line(text: impl Into<SharedString>, pal: Palette) -> Div {
    div()
        .flex_1()
        .min_w(px(0.0))
        .truncate()
        .text_size(px(12.5))
        .line_height(px(17.0))
        .text_color(pal.ink_meta())
        .child(text.into())
}

impl Root {
    /// Home, its own padding: the waiting field runs to the content's edges, under the title bar.
    pub(super) fn home(&mut self, d: &Data, pal: Palette, window: &Window, cx: &mut Context<'_, Self>) -> AnyElement {
        if let Some(e) = d.waiting().last() {
            return self.home_waiting(e, d, pal, cx);
        }
        let device = if cfg!(target_os = "macos") {
            "Mac"
        } else {
            "computer"
        };
        let sending = matches!(d.test, Test::Waiting(_));
        let test =
            kit::button("hero-test", kit::Look::Outline, kit::Size::Regular, Some(Glyph::Send), "Send a test", pal);
        let test = if d.paired && !sending {
            test.on_click(Self::on_model(cx, Model::send_test))
        } else {
            kit::disabled(test)
        };
        let header = div()
            .flex()
            .flex_wrap()
            .items_end()
            .gap(px(16.0))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w(px(260.0))
                    .child(div().title().text_color(pal.ink()).child(greeting(d.now)))
                    .child(mood(mood_line(d), pal)),
            )
            .child(test);

        let mut page = div().flex().flex_col().gap(px(26.0)).child(header);
        if let Some(banner) = Self::update_banner(d, pal, cx) {
            page = page.child(banner);
        }
        page = page.when_some(d.notice.clone(), |p, n| p.child(caption(n, pal).text_color(pal.danger)));
        page = page.when_some(d.note.clone(), |p, n| p.child(caption(n, pal)));
        if !matches!(d.test, Test::Idle) {
            page = page.child(kit::plate(pal).p(px(0.0)).child(Self::test_panel(d, true, pal, cx)));
        }
        page = page.child(Self::hero(d, device, pal, cx));

        // The plates.
        page = page.child(
            div()
                .grid()
                .grid_cols(3)
                .gap(px(14.0))
                .child(self.health_tile(d, pal, cx))
                .child(Self::work_tile(d, pal, cx))
                .child(Self::today_tile(d, pal)),
        );
        // What a plate opens.
        if self.health_open {
            page = page.child(Self::health_card(d, pal, cx));
        }
        if d.work_session().is_none() && (d.work.asking.is_some() || d.work.form.is_some()) {
            page = page.child(self.work_card(d, pal, window, cx));
        }

        let recent: Vec<&Entry> = d.activity().iter().take(RECENT).collect();
        page = page.child(Self::latest(&recent, "home", d, pal, cx));
        div().w_full().px(px(SIDE)).pt(px(34.0)).pb(px(40.0)).child(page).into_any_element()
    }

    /// "LATEST", "See all" and the rows (or a quiet line when there are none).
    fn latest(recent: &[&Entry], scope: &'static str, d: &Data, pal: Palette, cx: &mut Context<'_, Self>) -> Div {
        let mut rows = div().flex().flex_col().mt(px(6.0));
        if recent.is_empty() {
            rows = rows.child(caption("Nothing asked yet", pal).text_color(pal.ink_meta()).py(px(12.0)));
        }
        for (i, e) in recent.iter().enumerate() {
            rows = rows.child(
                Self::entry_row(e, scope, d, pal, i + 1 == recent.len(), None)
                    .on_click(Self::open_entry(cx, e.id.clone())),
            );
        }
        div()
            .flex()
            .flex_col()
            .child(div().flex().items_center().justify_between().child(kit::section_label("Latest", pal)).when(
                !recent.is_empty(),
                |r| {
                    r.child(
                        link(SharedString::from(format!("see-all-{scope}")), "See all", pal)
                            .on_click(Self::go(cx, Section::Activity)),
                    )
                },
            ))
            .child(rows)
    }

    /// One request as a row: its kind tile, a line, who and what kind, when, how it ended. `selected`: Activity's row
    /// (a raised pill when picked, the time as a clock); `None`: Home's (a hairline under it, the time as an age).
    pub(super) fn entry_row(
        e: &Entry,
        scope: &'static str,
        d: &Data,
        pal: Palette,
        last: bool,
        selected: Option<bool>,
    ) -> Stateful<Div> {
        let s = Ask::of(e);
        // The icon's badge is ringed in what the row sits on.
        let surface = if selected == Some(true) {
            pal.paper_raised()
        } else {
            pal.paper()
        };
        // Under the sentence, the machine's details in mono; else who and what kind.
        let meta = match &s.details {
            Some(details) => div().font_family(MONO).text_size(px(12.0)).child(details.clone()),
            None => div().text_size(px(12.5)).child(format!("{} · {}", s.who, s.kind)),
        };
        let when = match selected {
            Some(_) if e.at >= format::start_of_day(d.now) - 86_400 => format::clock(e.at),
            Some(_) => format::date(e.at),
            None => short_ago(d.now, e.at),
        };
        let row = div()
            .id(SharedString::from(format!("{scope}-{}", e.id)))
            .flex()
            .items_center()
            .gap(px(12.0))
            .min_h(px(if selected.is_some() {
                52.0
            } else {
                50.0
            }))
            .py(px(6.0))
            .min_w(px(0.0))
            .cursor_pointer()
            .child(kit::request_icon(s.mark, s.badge, s.field, 32.0, surface, pal))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w(px(0.0))
                    .child(
                        div()
                            .min_w(px(0.0))
                            .truncate()
                            .text_size(px(if selected.is_some() {
                                14.0
                            } else {
                                14.5
                            }))
                            .line_height(px(19.0))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(pal.ink())
                            .child(s.line()),
                    )
                    .child(meta.min_w(px(0.0)).truncate().line_height(px(17.0)).text_color(pal.ink_meta())),
            )
            .child(div().flex_none().text_size(px(12.5)).text_color(pal.ink_meta()).child(when))
            .child(div().flex().flex_none().items_center().child(ending(e, pal)));
        match selected {
            // Activity's: a pill when picked; its hairline hidden under it.
            Some(picked) => row.relative().px(px(10.0)).mx(px(-10.0)).rounded(px(12.0)).when_else(
                picked,
                |r| r.bg(pal.paper_raised()).border_1().border_color(pal.rule()),
                |r| {
                    r.hover(|h| h.bg(pal.paper_sunken().opacity(0.5))).when(!last, |r| {
                        r.child(div().absolute().left_0().right_0().bottom_0().h(px(1.0)).bg(pal.rule()))
                    })
                },
            ),
            None => row.when(!last, |r| r.border_b_1().border_color(pal.rule())),
        }
    }

    /// The plate under the greeting: what Reins does here (or what stops it), with what is protected as chips.
    fn hero(d: &Data, device: &str, pal: Palette, cx: &mut Context<'_, Self>) -> AnyElement {
        let it = |t: &str, i: bool| Words {
            text: t.to_owned(),
            bold: i,
        };
        let (words, action, last_dot): (Vec<Words>, Option<AnyElement>, gpui::Hsla) = match d.look {
            Look::Attention if !d.paired => (
                vec![
                    it("Pair ", false),
                    it("this ", true),
                    it(device, true),
                    it(" with ", false),
                    it("your phone", true),
                ],
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
                pal.warning,
            ),
            Look::Attention => (
                vec![it("The ", false), it("background service", true), it(" isn't running", false)],
                Some(
                    kit::button("hero-start", kit::Look::Inverse, kit::Size::Regular, Some(Glyph::Play), "Start", pal)
                        .on_click(Self::on_model(cx, Model::start_service))
                        .into_any_element(),
                ),
                pal.warning,
            ),
            Look::Paused => (
                vec![it("git goes ", false), it("straight to the hosts", true), it(" for now", false)],
                d.pause.on().then(|| {
                    kit::button("hero-resume", kit::Look::Inverse, kit::Size::Regular, Some(Glyph::Play), "Resume", pal)
                        .on_click(Self::on_model(cx, Model::resume))
                        .into_any_element()
                }),
                pal.danger,
            ),
            Look::On | Look::Waiting => (
                vec![
                    it("Your phone approves what ", false),
                    it("AI agents", true),
                    it(" do on ", false),
                    it("this ", true),
                    it(device, true),
                ],
                None,
                pal.success,
            ),
        };
        // What is protected, as chips.
        let mut chips = div().flex().flex_wrap().gap(px(8.0)).mt(px(12.0));
        if matches!(d.look, Look::On | Look::Waiting) {
            let tools = d.snapshot.as_ref().map_or(0, |s| s.harnesses.iter().filter(|h| h.added).count());
            if tools > 0 {
                chips = chips.child(kit::pill(
                    Some(Glyph::Check),
                    format::count(tools as u64, "AI tool", "AI tools"),
                    kit::PillLook::Soft,
                    pal,
                ));
            }
            if d.snapshot.as_ref().is_some_and(|s| !s.git_routed.is_empty()) {
                chips = chips.child(kit::pill(None, "git through Reins", kit::PillLook::Soft, pal));
            }
            if d.config().is_some_and(|c| c.ssh.enabled) {
                chips = chips.child(kit::pill(None, "SSH keys from your phone", kit::PillLook::Soft, pal));
            }
        } else if let Some(left) = d.pause.short(d.now) {
            chips = chips.child(kit::pill(Some(Glyph::Pause), left, kit::PillLook::Soft, pal));
        }
        let stage = div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(10.0))
            .child(device_tile(Glyph::Laptop, pal))
            .child(dots(last_dot, pal))
            .child(device_tile(Glyph::Phone, pal));
        kit::plate(pal)
            .flex_row()
            .flex_wrap()
            .items_center()
            .gap(px(28.0))
            .px(px(28.0))
            .py(px(26.0))
            .child(stage)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w(px(280.0))
                    .child(
                        div()
                            .font_family(SERIF)
                            .text_size(px(30.0))
                            .line_height(px(33.0))
                            .text_color(pal.ink())
                            .child(sentence(&words, pal)),
                    )
                    .child(chips),
            )
            .when_some(action, ParentElement::child)
            .into_any_element()
    }

    /// Home while a request waits: its field, then what will happen and the latest requests.
    fn home_waiting(&mut self, e: &Entry, d: &Data, pal: Palette, cx: &mut Context<'_, Self>) -> AnyElement {
        let s = Ask::of(e);
        // A push's commits, files and diffs, as the desktop app read them from the pack it holds.
        self.load_push(e, cx);
        let commits = self.kept_for(e).map(|push| self.commits(&push, false, pal, cx));
        let field = s.field;
        let on = pal.on_field();
        let (total, left) = d.wait_left(e);
        let more = d.waiting().count().saturating_sub(1);

        let agent = match s.agent {
            Mark::Brand(b) => super::icons::avatar(b, 24.0, pal).border_0().into_any_element(),
            Mark::Glyph(i) => div()
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .size(px(24.0))
                .rounded_full()
                .bg(pal.on_field_wash())
                .child(glyph(i, 14.0, on))
                .into_any_element(),
        };
        let who = who_from(e);
        let (name, from) = who.split_once(" · ").map_or((who.as_str(), None), |(n, f)| (n, Some(f)));
        let who_line = div().flex().items_center().gap(px(10.0)).child(agent).child(
            div()
                .flex()
                .min_w(px(0.0))
                .text_size(px(14.0))
                .font_weight(FontWeight::MEDIUM)
                .child(name.to_owned())
                .when_some(from, |l, f| {
                    l.child(
                        div().min_w(px(0.0)).truncate().text_color(pal.on_field_2()).child(format!("\u{a0}·\u{a0}{f}")),
                    )
                }),
        );
        let label = s.label();
        // The ask follows the field by 60 ms.
        let ask_id = SharedString::from(format!("ask-{}", e.id));
        let ask =
            div().font_family(SERIF).text_size(px(60.0)).line_height(px(58.0)).text_color(on).child(italic(&s.order));
        let ask: AnyElement = if motion::reduce_motion() {
            ask.into_any_element()
        } else {
            let (delay, run) = (motion::ASK_DELAY.as_secs_f32(), motion::FIELD_IN.as_secs_f32());
            let ease = curve(motion::EASE_OUT);
            ask.with_animation(ask_id, Animation::new(motion::ASK_DELAY + motion::FIELD_IN), move |el, t| {
                let p = ease(((t * (delay + run) - delay) / run).clamp(0.0, 1.0));
                el.opacity(p)
            })
            .into_any_element()
        };
        let mut timer_row = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(14.0))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .text_size(px(14.5))
                    .text_color(pal.on_field_2())
                    .child(glyph(Glyph::Phone, 16.0, pal.on_field_2()))
                    .child(format!("Answer on {}", d.phone())),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .font_family(MONO)
                    .text_size(px(14.0))
                    .text_color(on)
                    .child(kit::timer(left, i64::try_from(total).unwrap_or(i64::MAX), on))
                    .when(left >= 15, |t| t.child("left")),
            );
        if more > 0 {
            timer_row = timer_row
                .child(div().text_size(px(14.0)).text_color(pal.on_field_2()).child(format!("+{more} more waiting")));
        }
        let qr = div()
            .relative()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(14.0))
            .p(px(12.0))
            .rounded(px(20.0))
            .bg(gpui::white().opacity(0.14))
            .child(div().flex_none().p(px(8.0)).rounded(px(12.0)).bg(gpui::white()).child(crate::qr::element(
                crate::links::PHONE_ACTIVITY,
                76.0,
                gpui::black(),
                gpui::white(),
            )))
            .child(
                div()
                    .max_w(px(150.0))
                    .text_size(px(13.5))
                    .line_height(px(19.0))
                    .text_color(on)
                    .child(format!("Scan to open it on your phone. {} can't approve.", this_computer_cap())),
            );
        let top = if cfg!(target_os = "macos") {
            40.0
        } else {
            34.0
        };
        let surface = kit::field_surface(field, 0.0, kit::field::FIELD_RADIUS)
            .flex()
            .flex_wrap()
            .items_end()
            .gap(px(32.0))
            .px(px(SIDE))
            .pt(px(top + 6.0))
            .pb(px(34.0))
            .text_color(on)
            .child(
                div()
                    .relative()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w(px(300.0))
                    .gap(px(14.0))
                    .child(who_line)
                    .child(kit::field_label(label, pal))
                    .child(ask)
                    // The machine's details on their own mono line, never in the serif.
                    .when_some(s.details.clone(), |c, details| {
                        c.child(
                            div()
                                .font_family(MONO)
                                .text_size(px(14.0))
                                .line_height(px(20.0))
                                .text_color(pal.on_field_2())
                                .child(super::parts::wrapped(&details)),
                        )
                    })
                    .child(timer_row),
            )
            .child(qr);
        let surface = enter(surface, SharedString::from(format!("field-{}", e.id)), Entrance::Field);

        let rows = s.facts;
        let what = div()
            .flex()
            .flex_col()
            .child(kit::section_label("What will happen", pal))
            .child(fact_rows(&rows, FactLook::Home, pal).mt(px(6.0)))
            .when_some(commits, |w, c| w.child(c.mt(px(18.0))));
        let recent: Vec<&Entry> =
            d.activity().iter().filter(|e| e.outcome != Outcome::Waiting).take(RECENT_WAITING).collect();
        let mut below = div().flex().flex_col().gap(px(24.0));
        below = below.when_some(d.notice.clone(), |p, n| p.child(caption(n, pal).text_color(pal.danger)));
        below = below.when_some(d.note.clone(), |p, n| p.child(caption(n, pal)));
        // Side by side while each has room, else the latest under what will happen (a narrow window).
        below = below.child(
            div()
                .flex()
                .flex_wrap()
                .items_start()
                .gap(px(32.0))
                .child(what.flex_1().min_w(px(WAITING_COLUMN_MIN)))
                .child(
                    div().flex_1().min_w(px(WAITING_COLUMN_MIN)).child(Self::latest(&recent, "waiting", d, pal, cx)),
                ),
        );
        div()
            .w_full()
            .flex()
            .flex_col()
            .child(surface)
            .child(div().px(px(SIDE)).pt(px(24.0)).pb(px(40.0)).child(below))
            .into_any_element()
    }

    /// Health: all good, or how many things to look at with the first one's fix. A click opens the checks.
    fn health_tile(&self, d: &Data, pal: Palette, cx: &mut Context<'_, Self>) -> AnyElement {
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
        let flagged = matches!(level, Level::Warn | Level::Fail);
        let mut under = div().flex().items_center().gap(px(8.0)).mt(px(4.0)).min_h(px(30.0)).child(
            plate_line(line, pal).when(flagged, |l| {
                l.text_color(if level == Level::Fail {
                    pal.danger
                } else {
                    pal.warning
                })
            }),
        );
        if let Some(fix) = fix {
            under = under.child(
                kit::button("tile-fix", kit::Look::Soft, kit::Size::Small, None, fix.label(), pal).on_click(
                    cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        this.model.update(cx, |m, cx| m.fix(fix, cx));
                    }),
                ),
            );
        }
        kit::plate(pal)
            .id("tile-health")
            .min_w(px(0.0))
            .cursor_pointer()
            .when(self.health_open, |t| t.border_color(pal.rule_2()))
            .hover(|s| s.border_color(pal.rule_2()))
            .child(kit::section_label("Health", pal))
            .child(plate_value(value).text_color(pal.ink()))
            .child(under)
            .on_click(Self::on_view(cx, |this, _| this.health_open = !this.health_open))
            .into_any_element()
    }

    /// Work session: Start; the request on the phone; or, running, a violet field (a session is a bundle of grants).
    fn work_tile(d: &Data, pal: Palette, cx: &mut Context<'_, Self>) -> AnyElement {
        let under = || div().flex().items_center().justify_between().gap(px(8.0)).mt(px(4.0)).min_h(px(30.0));
        if let Some(session) = d.work_session() {
            let on = pal.on_field();
            let actions: AnyElement = if d.work.ending {
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
            let allows = session.allows.join(" · ");
            return kit::field_surface(Field::Grant, 20.0, 20.0)
                .flex()
                .flex_col()
                .min_w(px(0.0))
                .px(px(20.0))
                .py(px(18.0))
                .text_color(on)
                .child(div().relative().child(kit::field_label("Work session · running", pal)))
                .child(
                    div()
                        .relative()
                        .mt(px(8.0))
                        .font_family(SERIF)
                        .text_size(px(28.0))
                        .line_height(px(30.0))
                        .min_w(px(0.0))
                        .truncate()
                        .child(italic(&italic_tail(&session.reason))),
                )
                .child(
                    div()
                        .relative()
                        .mt(px(4.0))
                        .text_size(px(13.0))
                        .line_height(px(18.0))
                        .text_color(pal.on_field_2())
                        .truncate()
                        .child(format!(
                            "{} left · until {}",
                            crate::work::left(session, d.now),
                            format::clock(session.expires_at)
                        )),
                )
                .child(
                    div()
                        .relative()
                        .flex()
                        .items_center()
                        .gap(px(8.0))
                        .mt(px(8.0))
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.0))
                                .truncate()
                                .text_size(px(12.5))
                                .text_color(pal.on_field_2())
                                .child(allows),
                        )
                        .child(actions),
                )
                .into_any_element();
        }
        let tile = kit::plate(pal).min_w(px(0.0)).child(kit::section_label("Work session", pal));
        if let Some(asking) = &d.work.asking {
            let left = i64::try_from(asking.left(std::time::Instant::now())).unwrap_or(0);
            return tile
                .child(plate_value("On your phone").text_color(pal.ink()))
                .child(under().child(plate_line(asking.request.reason.clone(), pal)).child(
                    div().font_family(MONO).text_size(px(12.5)).text_color(pal.violet()).child(clock_left(left)),
                ))
                .into_any_element();
        }
        if let Some(why) = d.work.failed.clone() {
            return tile
                .child(plate_value("Not started").text_color(pal.ink()))
                .child(under().child(plate_line(why, pal).text_color(pal.danger)))
                .into_any_element();
        }
        let open = d.work.form.is_some();
        let start = kit::button(
            "session-start",
            kit::Look::Soft,
            kit::Size::Small,
            None,
            if open {
                "Cancel"
            } else {
                "Start"
            },
            pal,
        );
        tile.child(plate_value("Not running").text_color(pal.ink()))
            .child(under().child(plate_line("Approve once, work freely", pal)).child(if !d.paired {
                kit::disabled(start)
            } else if open {
                start.on_click(Self::on_model(cx, Model::close_work_form))
            } else {
                start.on_click(Self::on_model(cx, Model::open_work_form))
            }))
            .into_any_element()
    }

    /// Today: approved, denied, no answer, in serif numbers.
    fn today_tile(d: &Data, pal: Palette) -> AnyElement {
        let t = today(d.activity(), d.now);
        let n = |v: usize, color: gpui::Hsla| {
            div()
                .text_color(if v == 0 {
                    pal.ink_meta()
                } else {
                    color
                })
                .child(v.to_string())
        };
        kit::plate(pal)
            .min_w(px(0.0))
            .child(kit::section_label("Today", pal))
            .child(
                div()
                    .flex()
                    .items_baseline()
                    .gap(px(18.0))
                    .mt(px(10.0))
                    .font_family(SERIF)
                    .text_size(px(30.0))
                    .line_height(px(34.0))
                    .child(n(t.approved, pal.success))
                    .child(n(t.denied, pal.danger))
                    .child(n(t.timed_out, pal.warning)),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .mt(px(4.0))
                    .min_h(px(30.0))
                    .child(plate_line("approved · denied · no answer", pal)),
            )
            .into_any_element()
    }

    fn update_banner(d: &Data, pal: Palette, cx: &mut Context<'_, Self>) -> Option<AnyElement> {
        let update = d.update.as_ref()?;
        let banner = kit::plate(pal).flex_row().items_center().gap(px(12.0)).py(px(12.0)).child(glyph(
            Glyph::Download,
            18.0,
            pal.ink(),
        ));
        Some(
            if update.ready.is_some() {
                let label = if d.updating {
                    "Restarting…"
                } else {
                    "Restart to update"
                };
                let restart = kit::button("restart-update", kit::Look::Inverse, kit::Size::Small, None, label, pal);
                banner
                    .child(div().flex_1().text_size(px(14.0)).child(format!("Reins {} is ready", update.version)))
                    .child(if d.updating {
                        kit::disabled(restart)
                    } else {
                        restart.on_click(Self::on_model(cx, Model::restart_to_update))
                    })
            } else {
                banner
                    .child(div().flex_1().text_size(px(14.0)).child(format!("Reins {} is out", update.version)))
                    .child(
                        link("download", "Download", pal).on_click(|_, _, cx| cx.open_url(&crate::links::download())),
                    )
            }
            .into_any_element(),
        )
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

/// One end of the calm plate's "this computer → your phone": a 52 px neutral tile.
fn device_tile(g: Glyph, pal: Palette) -> Div {
    div()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(px(52.0))
        .rounded(px(16.0))
        .bg(pal.paper_sunken())
        .child(glyph(g, 22.0, pal.ink()))
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
        assert_eq!(short_ago(now, now - 4 * 60), "4 min");
        assert_eq!(short_ago(now, now - 2 * 3_600), "2 h");
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
    fn the_last_words_go_italic() {
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
