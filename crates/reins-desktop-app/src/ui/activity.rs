//! Activity (`Desk-Activity.png`): this computer's log of requests (journal) as a list, and the request picked in it
//! as a plate beside. The list has what waits for the phone first, then the days; its filter is an inverse button
//! that opens a menu of outcomes and kinds, each outcome with how many. The plate: the kind tile and the label line
//! with the time, the request in the serif with its target in italic, how it ended and where, the facts as rows, and
//! the request as a terminal showed it, in a mono well.

use std::time::{Duration, Instant};

use gpui::prelude::FluentBuilder as _;
use gpui::{
    Anchor, AnyElement, Context, Div, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _,
    SharedString, Stateful, StatefulInteractiveElement as _, Styled as _, div, px,
};
use reins_desktop::journal::{Decider, Entry, Kind, Outcome};

use super::home::label_line;
use super::icons::{Glyph, glyph};
use super::parts::{fact_rows, italic, phone_qr};
use super::story::{Fact, Story, answer, facts, transcript};
use super::{Data, ROWS, Root, kit};
use crate::backend::ACTIVITY_LIMIT;
use crate::format;
use crate::model::Model;
use crate::theme::{MONO, Palette, SERIF, SignalText as _};

/// The outcome filters, in order: the board's, then the rarer ones (shown when there are some).
const OUTCOMES: [(Option<Outcome>, &str); 8] = [
    (None, "All"),
    (Some(Outcome::Approved), "Approved"),
    (Some(Outcome::Denied), "Denied"),
    (Some(Outcome::TimedOut), "Timed out"),
    (Some(Outcome::Allowed), "Allowed by rules"),
    (Some(Outcome::Waiting), "Waiting"),
    (Some(Outcome::Failed), "Failed"),
    (Some(Outcome::Blocked), "Blocked"),
];

/// The kind filters, in the board's order.
const KINDS: [Kind; 8] =
    [Kind::Git, Kind::Command, Kind::File, Kind::Ssh, Kind::Api, Kind::Secrets, Kind::Mcp, Kind::Ask];

/// How many of [`OUTCOMES`] always show.
const ALWAYS: usize = 5;

/// A menu closed by a press outside it: a press on its trigger right then does not open it again.
pub const REOPEN_GUARD: Duration = Duration::from_millis(250);

/// Whether `e` passes the filters (`None`: any).
#[must_use]
pub fn matches(e: &Entry, outcome: Option<Outcome>, kind: Option<Kind>) -> bool {
    outcome.is_none_or(|o| e.outcome == o) && kind.is_none_or(|k| e.kind == k)
}

/// Who decided, in a few words; nothing when the phone answered (the usual, and the verdict says how).
#[must_use]
pub fn decided(e: &Entry) -> Option<&'static str> {
    match (e.decider, e.outcome) {
        (Decider::Settings, _) => Some("by your rules"),
        (Decider::Local, _) => Some("on this computer"),
        (Decider::Phone, Outcome::TimedOut) => Some("no answer"),
        (Decider::Phone, Outcome::Failed) => Some("couldn't ask"),
        (Decider::Phone, _) => None,
    }
}

/// The day a group of rows is: "Today", "Yesterday", "Oct 3".
#[must_use]
pub fn day(now: i64, at: i64) -> String {
    let today = format::start_of_day(now);
    if at >= today {
        "Today".to_owned()
    } else if at >= format::start_of_day(today - 1) {
        "Yesterday".to_owned()
    } else {
        format::date(at)
    }
}

/// The filter button's label: what it shows.
fn filter_label(outcome: Option<Outcome>, kind: Option<Kind>) -> String {
    let name = |o: Outcome| OUTCOMES.iter().find(|(x, _)| *x == Some(o)).map_or(o.label(), |(_, l)| l);
    match (outcome, kind) {
        (None, None) => "All".to_owned(),
        (Some(o), None) => name(o).to_owned(),
        (None, Some(k)) => k.label().to_owned(),
        (Some(o), Some(k)) => format!("{} · {}", name(o), k.label()),
    }
}

/// The list's last line: "3 of 142 · newest 500 are kept here".
fn footer(shown: usize, total: usize) -> String {
    format!("{shown} of {total} · newest {ACTIVITY_LIMIT} are kept here")
}

/// A filter's choice in the menu: its label and how many (mono), `inverse` when picked.
fn choice(id: impl Into<gpui::ElementId>, label: &str, count: Option<usize>, on: bool, pal: Palette) -> Stateful<Div> {
    div()
        .id(id.into())
        .flex()
        .flex_none()
        .items_center()
        .gap(px(6.0))
        .h(px(30.0))
        .px(px(11.0))
        .rounded_full()
        .text_size(px(13.0))
        .font_weight(FontWeight::MEDIUM)
        .whitespace_nowrap()
        .cursor_pointer()
        .active(|s| s.opacity(kit::button::PRESSED))
        .when_else(
            on,
            |c| c.bg(pal.inverse()).text_color(pal.on_inverse()),
            |c| c.bg(pal.paper_sunken()).text_color(pal.ink_2()).hover(|s| s.text_color(pal.ink())),
        )
        .child(label.to_owned())
        .when_some(count, |c, n| c.child(div().font_family(MONO).text_size(px(11.5)).opacity(0.7).child(n.to_string())))
}

impl Root {
    pub(super) fn activity(&mut self, d: &Data, pal: Palette, cx: &mut Context<'_, Self>) -> AnyElement {
        let all = d.activity();
        let (outcome, kind) = (self.outcome_filter, self.kind_filter);
        let shown: Vec<&Entry> = all.iter().filter(|e| matches(e, outcome, kind)).collect();
        let total = shown.len();
        // What waits comes first, then the rest by day (the log is newest first).
        let (waiting, rest): (Vec<&Entry>, Vec<&Entry>) =
            shown.iter().take(self.rows_shown).partition(|e| e.outcome == Outcome::Waiting);
        let selected = self
            .selected
            .as_ref()
            .and_then(|id| shown.iter().find(|e| &e.id == id))
            .or_else(|| waiting.first().or_else(|| rest.first()))
            .copied();

        let header = div()
            .flex()
            .items_center()
            .gap(px(12.0))
            .child(div().flex_1().min_w(px(0.0)).title().text_color(pal.ink()).child("Activity"))
            .when(!all.is_empty(), |h| h.child(self.filter_button(all, pal, cx)));

        let mut list = div().flex().flex_col().gap(px(16.0)).child(header);
        if all.is_empty() {
            list = list
                .child(kit::empty_state("Nothing asked yet.", 280.0, pal))
                .child(kit::plate(pal).p(px(0.0)).child(Self::test_panel(d, true, pal, cx)));
        } else if total == 0 {
            list = list.child(div().text_size(px(13.0)).text_color(pal.ink_meta()).child("No matches"));
        } else {
            let sel = selected.map(|e| e.id.clone());
            let mut groups: Vec<(String, Vec<&Entry>)> = Vec::new();
            if !waiting.is_empty() {
                groups.push(("Waiting".to_owned(), waiting));
            }
            for e in rest {
                let label = day(d.now, e.at);
                match groups.last_mut() {
                    Some((l, entries)) if *l == label => entries.push(e),
                    _ => groups.push((label, vec![e])),
                }
            }
            let mut body = div().flex().flex_col().gap(px(18.0));
            for (label, entries) in groups {
                let mut rows = div().flex().flex_col().mt(px(6.0));
                for (i, e) in entries.iter().enumerate() {
                    let id = e.id.clone();
                    let picked = sel.as_ref() == Some(&e.id);
                    rows = rows.child(
                        Self::entry_row(e, "activity", d, pal, i + 1 == entries.len(), Some(picked))
                            .on_click(Self::on_view(cx, move |this, _| this.selected = Some(id.clone()))),
                    );
                }
                body = body.child(div().flex().flex_col().child(kit::section_label(label, pal)).child(rows));
            }
            let mut foot = div().flex().items_center().gap(px(12.0)).child(
                div()
                    .flex_1()
                    .text_size(px(12.5))
                    .text_color(pal.ink_meta())
                    .child(footer(total.min(self.rows_shown), all.len())),
            );
            if total > self.rows_shown {
                foot = foot.child(
                    kit::button("more", kit::Look::Soft, kit::Size::Small, None, "Show more", pal)
                        .on_click(Self::on_view(cx, |this, _| this.rows_shown += ROWS)),
                );
            }
            list = list.child(body.child(foot));
        }

        let page =
            div().flex().flex_wrap().items_start().gap(px(32.0)).child(div().flex_1().min_w(px(360.0)).child(list));
        match selected {
            Some(e) => page.child(div().flex_1().min_w(px(360.0)).child(Self::entry_detail(e, d, pal, cx))),
            None => page,
        }
        .into_any_element()
    }

    /// The filter: an inverse button, and while open its menu of outcomes and kinds.
    fn filter_button(&self, all: &[Entry], pal: Palette, cx: &mut Context<'_, Self>) -> AnyElement {
        let (outcome, kind) = (self.outcome_filter, self.kind_filter);
        let open = self.filter_open;
        let trigger = div()
            .id("activity-filter")
            .relative()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(7.0))
            .h(px(36.0))
            .px(px(14.0))
            .rounded_full()
            .bg(pal.inverse())
            .text_color(pal.on_inverse())
            .text_size(px(13.5))
            .font_weight(FontWeight::SEMIBOLD)
            .whitespace_nowrap()
            .cursor_pointer()
            .active(|s| s.opacity(kit::button::PRESSED))
            .child(glyph(Glyph::Filter, 16.0, pal.on_inverse()))
            .child(filter_label(outcome, kind))
            .child(glyph(
                if open {
                    Glyph::ChevronUp
                } else {
                    Glyph::ChevronDown
                },
                14.0,
                pal.on_inverse(),
            ))
            .on_click(Self::on_view(cx, |this, _| {
                let just_closed = this.menu_closed.is_some_and(|at| at.elapsed() < REOPEN_GUARD);
                this.filter_open = !this.filter_open && !just_closed;
            }));
        if !open {
            return trigger.into_any_element();
        }
        let mut outcomes = div().flex().flex_wrap().gap(px(6.0));
        for (i, (o, label)) in OUTCOMES.into_iter().enumerate() {
            let n = all.iter().filter(|e| matches(e, o, kind)).count();
            if i >= ALWAYS && n == 0 && outcome != o {
                continue;
            }
            outcomes = outcomes.child(choice(("outcome", i), label, Some(n), outcome == o, pal).on_click(
                Self::on_view(cx, move |this, _| {
                    this.outcome_filter = o;
                    this.rows_shown = ROWS;
                }),
            ));
        }
        let mut kinds = div().flex().flex_wrap().gap(px(6.0)).child(
            choice("kind-all", "All kinds", None, kind.is_none(), pal).on_click(Self::on_view(cx, |this, _| {
                this.kind_filter = None;
                this.rows_shown = ROWS;
            })),
        );
        for (i, k) in KINDS.into_iter().enumerate() {
            kinds = kinds.child(choice(("kind", i), k.label(), None, kind == Some(k), pal).on_click(Self::on_view(
                cx,
                move |this, _| {
                    this.kind_filter = Some(k);
                    this.rows_shown = ROWS;
                },
            )));
        }
        let menu = kit::menu(pal)
            .w(px(340.0))
            .p(px(16.0))
            .gap(px(12.0))
            .child(kit::section_label("Outcome", pal))
            .child(outcomes)
            .child(kit::section_label("Kind", pal).mt(px(4.0)))
            .child(kinds)
            .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                this.filter_open = false;
                this.menu_closed = Some(Instant::now());
                cx.notify();
            }));
        trigger.child(kit::open_menu("activity-filter-menu", menu, Anchor::TopRight)).into_any_element()
    }

    /// One request, as a plate: the kind tile and the label line, the request in the serif, how it ended, the facts
    /// and the transcript.
    fn entry_detail(e: &Entry, d: &Data, pal: Palette, cx: &mut Context<'_, Self>) -> AnyElement {
        let s = Story::of(e);
        let (ic, tone, said) = answer(e, &d.phone());
        let verdict = pal.tone(tone);
        let mark = match e.outcome {
            Outcome::Denied | Outcome::Blocked => Glyph::Close,
            Outcome::Approved | Outcome::Allowed => Glyph::Check,
            _ => ic,
        };
        let mut plate = div()
            .flex()
            .flex_col()
            .gap(px(14.0))
            .px(px(28.0))
            .py(px(26.0))
            .rounded(px(24.0))
            .bg(pal.paper_raised())
            .border_1()
            .border_color(pal.rule())
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.0))
                    .child(kit::kind_tile(s.icon, s.tone, 40.0, pal).rounded(px(12.0)))
                    .child(kit::section_label(format!("{} · {}", label_line(&s), format::when(d.now, e.at)), pal)),
            )
            .child(
                div()
                    .font_family(SERIF)
                    .text_size(px(38.0))
                    .line_height(px(40.0))
                    .text_color(pal.ink())
                    .child(italic(&s.sentence)),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .text_size(px(14.5))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(verdict)
                    .child(glyph(mark, 16.0, verdict))
                    .child(said),
            );
        if e.outcome == Outcome::Waiting {
            let (total, left) = d.wait_left(e);
            plate = plate.child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap(px(12.0))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(6.0))
                            .font_family(MONO)
                            .text_size(px(14.0))
                            .child(kit::timer(left, i64::try_from(total).unwrap_or(i64::MAX), pal.ink()))
                            .when(left >= 15, |t| t.child("left")),
                    )
                    .child(
                        kit::button(
                            "detail-qr",
                            kit::Look::Soft,
                            kit::Size::Small,
                            Some(Glyph::QrCode),
                            if d.show_qr {
                                "Hide code"
                            } else {
                                "Open on phone"
                            },
                            pal,
                        )
                        .on_click(Self::on_model(cx, Model::toggle_phone_qr)),
                    ),
            );
            if d.show_qr {
                plate = plate.child(phone_qr(pal));
            }
        }

        let fact = |key: &str, value: String| Fact {
            key: key.to_owned(),
            value,
            mono: false,
        };
        let mut rows = Vec::new();
        if let Some(end) = e.ended_at {
            let key = match e.outcome {
                Outcome::Approved | Outcome::Denied => "Answered in",
                Outcome::TimedOut => "Waited",
                _ => "Took",
            };
            rows.push(fact(key, format::duration(end - e.at)));
        } else if e.outcome == Outcome::Waiting {
            rows.push(fact("Waiting", format::duration(d.now - e.at)));
        }
        if let Some(who) = decided(e).filter(|_| e.decider != Decider::Phone) {
            rows.push(fact("Decided", who.to_owned()));
        }
        rows.extend(facts(e).0);
        plate = plate.child(fact_rows(&rows, 0.0, Some(120.0), pal));

        let mut well = div()
            .flex()
            .flex_col()
            .px(px(16.0))
            .py(px(14.0))
            .rounded(px(14.0))
            .bg(pal.paper())
            .font_family(MONO)
            .text_size(px(12.5))
            .line_height(px(19.0))
            .text_color(pal.ink_2())
            .min_w(px(0.0))
            .overflow_hidden();
        for line in transcript(e, &d.phone()) {
            well = well.child(div().min_w(px(0.0)).child(SharedString::from(line)));
        }
        plate.child(well).into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(kind: Kind, outcome: Outcome) -> Entry {
        let mut e = Entry::new(kind, "x");
        e.outcome = outcome;
        e
    }

    #[test]
    fn filters_combine() {
        let entries = [
            entry(Kind::Git, Outcome::Approved),
            entry(Kind::Git, Outcome::Denied),
            entry(Kind::Mcp, Outcome::Approved),
            entry(Kind::Command, Outcome::Waiting),
        ];
        let count = |o, k| entries.iter().filter(|e| matches(e, o, k)).count();
        assert_eq!(count(None, None), 4);
        assert_eq!(count(Some(Outcome::Approved), None), 2);
        assert_eq!(count(None, Some(Kind::Git)), 2);
        assert_eq!(count(Some(Outcome::Approved), Some(Kind::Git)), 1);
        assert_eq!(count(Some(Outcome::TimedOut), None), 0);
        assert_eq!(filter_label(None, None), "All");
        assert_eq!(filter_label(Some(Outcome::Denied), Some(Kind::Git)), "Denied · git");
        assert_eq!(filter_label(Some(Outcome::Allowed), None), "Allowed by rules");
    }

    #[test]
    fn rows_say_who_decided() {
        let e = entry(Kind::Command, Outcome::Denied).source(Some("Claude Code"));
        assert_eq!(decided(&e), None);
        let rules = entry(Kind::Command, Outcome::Allowed).decider(Decider::Settings);
        assert_eq!(decided(&rules), Some("by your rules"));
        assert_eq!(decided(&entry(Kind::Ask, Outcome::Waiting)), None);
        assert_eq!(decided(&entry(Kind::Ask, Outcome::TimedOut)), Some("no answer"));
    }

    #[test]
    fn rows_are_grouped_by_day() {
        let now = format::start_of_day(1_800_000_000) + 10 * 3_600;
        assert_eq!(day(now, now - 60), "Today");
        assert_eq!(day(now, format::start_of_day(now) - 60), "Yesterday");
        assert_eq!(day(now, now - 5 * 86_400), format::date(now - 5 * 86_400));
    }

    /// The board's outcomes come first, every outcome has a filter, and the list says how much is kept.
    #[test]
    fn outcome_filters_start_with_the_boards() {
        assert_eq!(OUTCOMES[0], (None, "All"));
        let labels: Vec<&str> = OUTCOMES[..ALWAYS].iter().map(|(_, l)| *l).collect();
        assert_eq!(labels, ["All", "Approved", "Denied", "Timed out", "Allowed by rules"]);
        let outcomes: std::collections::HashSet<_> = OUTCOMES.iter().filter_map(|(o, _)| *o).collect();
        assert_eq!(outcomes.len(), OUTCOMES.len() - 1);
        assert_eq!(footer(3, 142), "3 of 142 · newest 500 are kept here");
        let kinds: std::collections::HashSet<_> = KINDS.into_iter().collect();
        assert!(Kind::ALL.iter().all(|k| kinds.contains(k)), "every kind has a filter");
    }
}
