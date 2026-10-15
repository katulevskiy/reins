//! Activity: this computer's log of requests (journal) as a list and the request picked in it. The list has what waits
//! for the phone first, then the days; a filter picks outcomes and kinds. The detail is the phone's hero (who →
//! what, one sentence, how it ended and where), the values (asked, answered in, the command or tool) and the detail
//! in a mono well.

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, Context, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _, SharedString,
    StatefulInteractiveElement as _, Styled as _, anchored, deferred, div, point, px, relative,
};
use reins_desktop::journal::{Decider, Entry, Kind, Outcome};

use super::icons::{Icon, Mark};
use super::parts::{
    Btn, btn, btn_icon, caption, card, chip, empty, empty_state, filter_chip, fine, group_title, help, info_row,
    phone_qr, popover, ring, sentence, stage, well,
};
use super::sidebar::clock_left;
use super::story::{Story, answer};
use super::{Data, ROWS, Root};
use crate::format;
use crate::model::Model;
use crate::theme::{MONO, Palette, SERIF, Tone};

/// The outcome filters, in order.
const OUTCOMES: [(Option<Outcome>, &str); 6] = [
    (None, "All"),
    (Some(Outcome::Waiting), "Waiting"),
    (Some(Outcome::Approved), "Approved"),
    (Some(Outcome::Denied), "Denied"),
    (Some(Outcome::TimedOut), "Timed out"),
    (Some(Outcome::Failed), "Failed"),
];

/// Whether `e` passes the filters (`None`: any).
#[must_use]
pub fn matches(e: &Entry, outcome: Option<Outcome>, kind: Option<Kind>) -> bool {
    outcome.is_none_or(|o| e.outcome == o) && kind.is_none_or(|k| e.kind == k)
}

/// Who decided, in a few words; nothing when the phone answered (the usual, and the chip says how).
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
    match (outcome, kind) {
        (None, None) => "All".to_owned(),
        (Some(o), None) => o.label().to_owned(),
        (None, Some(k)) => k.label().to_owned(),
        (Some(o), Some(k)) => format!("{} · {}", o.label(), k.label()),
    }
}

/// "Today, 14:32:05".
fn asked_at(now: i64, at: i64) -> String {
    let when = format::when(now, at);
    when.strip_prefix("today ").map_or(when.clone(), |time| format!("Today, {time}"))
}

impl Root {
    pub(super) fn activity(&mut self, d: &Data, pal: Palette, top: f32, cx: &mut Context<'_, Self>) -> AnyElement {
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
            .gap(px(10.0))
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .text_size(px(44.0))
                    .line_height(px(44.0))
                    .font_family(SERIF)
                    .font_weight(FontWeight::NORMAL)
                    .child("Activity"),
            )
            .child(help(
                "activity",
                "Everything asked for on this computer, and how it ended. Approving always happens on your phone; \
                 Reins only keeps the record here, the newest 500.",
                d,
                pal,
                cx,
            ))
            .when(!all.is_empty(), |h| h.child(self.filter_button(all, pal, cx)));

        let mut list = div().flex().flex_col().gap(px(8.0)).child(header);
        if all.is_empty() {
            list = list.child(
                card(pal)
                    .mt(px(10.0))
                    .child(empty_state(
                        Icon::Activity,
                        "Nothing asked yet",
                        Some(help(
                            "activity-empty",
                            "When an AI agent, git or ssh on this computer asks for something, it shows up here: what \
                             it was, who asked, and how your phone answered. Send a test to see one arrive.",
                            d,
                            pal,
                            cx,
                        )),
                        pal,
                    ))
                    .child(div().border_t_1().border_color(pal.rule()).child(Self::test_panel(d, true, pal, cx))),
            );
        } else if total == 0 {
            list = list.child(card(pal).mt(px(10.0)).child(empty("No matches", pal)));
        } else {
            let open = |e: &Entry| {
                let id = e.id.clone();
                move |this: &mut Self, _: &mut Context<'_, Self>| this.selected = Some(id.clone())
            };
            let sel = selected.map(|e| e.id.clone());
            if !waiting.is_empty() {
                let mut c = card(pal);
                for (i, e) in waiting.iter().enumerate() {
                    c = c.child(
                        Self::history_row(e, "activity", d, pal, i == 0, sel.as_ref() == Some(&e.id))
                            .on_click(Self::on_view(cx, open(e))),
                    );
                }
                list = list.child(group_title("Waiting", pal).mt(px(10.0))).child(c);
            }
            let mut days: Vec<(String, Vec<&Entry>)> = Vec::new();
            for e in rest {
                let label = day(d.now, e.at);
                match days.last_mut() {
                    Some((l, entries)) if *l == label => entries.push(e),
                    _ => days.push((label, vec![e])),
                }
            }
            for (label, entries) in days {
                let mut c = card(pal);
                for (i, e) in entries.iter().enumerate() {
                    c = c.child(
                        Self::history_row(e, "activity", d, pal, i == 0, sel.as_ref() == Some(&e.id))
                            .on_click(Self::on_view(cx, open(e))),
                    );
                }
                list = list.child(group_title(&label, pal).mt(px(10.0))).child(c);
            }
            if total > self.rows_shown {
                list = list.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(12.0))
                        .pt(px(6.0))
                        .child(fine(format!("{} of {total}", self.rows_shown), pal).flex_1())
                        .child(
                            btn("more", "Show more", Btn::Plain, pal)
                                .on_click(Self::on_view(cx, |this, _| this.rows_shown += ROWS)),
                        ),
                );
            } else {
                list =
                    list.child(fine(format::count(total as u64, "request", "requests"), pal).px(px(4.0)).pt(px(4.0)));
            }
        }

        let left = div()
            .id("activity-list")
            .flex_1()
            .min_w(px(260.0))
            .h_full()
            .overflow_y_scroll()
            .child(div().px(px(28.0)).pt(px(top)).pb(px(30.0)).child(list));
        let Some(e) = selected else {
            return div().size_full().flex().child(left).into_any_element();
        };
        let detail_bg = if pal.dark {
            gpui::rgb(0x0009_090B).into()
        } else {
            pal.paper().blend(gpui::white().opacity(0.5))
        };
        let right = div()
            .id("activity-detail")
            .w(relative(0.47))
            .min_w(px(260.0))
            .max_w(px(540.0))
            .h_full()
            .overflow_y_scroll()
            .bg(detail_bg)
            .border_l_1()
            .border_color(pal.rule())
            .child(div().px(px(26.0)).pt(px(top)).pb(px(30.0)).child(Self::entry_detail(e, d, pal, cx)));
        div().size_full().flex().child(left).child(right).into_any_element()
    }

    /// The filter button and, while open, its popover: outcomes, then kinds, each with how many.
    fn filter_button(&self, all: &[Entry], pal: Palette, cx: &mut Context<'_, Self>) -> AnyElement {
        let (outcome, kind) = (self.outcome_filter, self.kind_filter);
        let filtered = outcome.is_some() || kind.is_some();
        let mut b = btn_icon("activity-filter", Icon::ListFilter, filter_label(outcome, kind), Btn::Plain, pal)
            .when(filtered, |b| b.bg(pal.violet().opacity(0.12)).text_color(pal.violet()))
            .on_click(Self::on_view(cx, |this, _| this.filter_open = !this.filter_open));
        if !self.filter_open {
            return b.into_any_element();
        }
        let mut outcomes = div().flex().flex_wrap().gap(px(6.0));
        for (i, (o, label)) in OUTCOMES.into_iter().enumerate() {
            let n = all.iter().filter(|e| matches(e, o, kind)).count();
            outcomes = outcomes.child(filter_chip(("outcome", i), label, Some(n), outcome == o, pal).on_click(
                Self::on_view(cx, move |this, _| {
                    this.outcome_filter = o;
                    this.rows_shown = ROWS;
                }),
            ));
        }
        let mut kinds = div().flex().flex_wrap().gap(px(6.0)).child(
            filter_chip("kind-all", "All kinds", None, kind.is_none(), pal).on_click(Self::on_view(cx, |this, _| {
                this.kind_filter = None;
                this.rows_shown = ROWS;
            })),
        );
        for (i, k) in Kind::ALL.into_iter().enumerate() {
            let n = all.iter().filter(|e| matches(e, outcome, Some(k))).count();
            kinds = kinds.child(filter_chip(("kind", i), k.label(), Some(n), kind == Some(k), pal).on_click(
                Self::on_view(cx, move |this, _| {
                    this.kind_filter = Some(k);
                    this.rows_shown = ROWS;
                }),
            ));
        }
        let pop = popover(pal)
            .w(px(360.0))
            .gap(px(12.0))
            .child(group_title("Outcome", pal).px(px(0.0)))
            .child(outcomes)
            .child(group_title("Kind", pal).px(px(0.0)))
            .child(kinds);
        b = b.on_mouse_down_out(cx.listener(|this, _, _, cx| {
            this.filter_open = false;
            cx.notify();
        }));
        b.relative()
            .child(
                div().absolute().top_0().right_0().size_0().child(
                    deferred(
                        anchored().offset(point(px(-360.0), px(40.0))).snap_to_window_with_margin(px(12.0)).child(pop),
                    )
                    .with_priority(1),
                ),
            )
            .into_any_element()
    }

    /// One request, as the phone shows it.
    fn entry_detail(e: &Entry, d: &Data, pal: Palette, cx: &mut Context<'_, Self>) -> AnyElement {
        let mut s = Story::of(e);
        if s.target == Mark::Icon(Icon::GitBranch)
            && let Some(b) = d.git_host_mark()
        {
            s.target = Mark::Brand(b);
        }
        let color = if s.tone == Tone::Neutral {
            pal.violet()
        } else {
            pal.tone(s.tone)
        };
        let (ic, tone, said) = answer(e, &d.phone());
        let mut hero = div()
            .flex()
            .flex_col()
            .items_center()
            .gap(px(12.0))
            .p(px(18.0))
            .rounded(px(18.0))
            .bg(pal.paper_raised())
            .border_1()
            .border_color(pal.rule())
            .child(stage(s.agent, s.target, Some((s.badge, color)), color, 46.0, pal))
            .child(
                div()
                    .w_full()
                    .text_center()
                    .font_family(SERIF)
                    .text_size(px(26.0))
                    .line_height(px(30.0))
                    .font_weight(FontWeight::NORMAL)
                    .text_color(pal.ink())
                    .child(sentence(&s.sentence, pal)),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .justify_center()
                    .gap(px(8.0))
                    .when_some(s.risk, |c, risk| c.child(chip(Some(Icon::TriangleAlert), risk, color)))
                    .child(chip(Some(ic), said, pal.tone(tone))),
            );
        if e.outcome == Outcome::Waiting {
            let (total, left) = d.wait_left(e);
            #[allow(clippy::cast_precision_loss, reason = "minutes of seconds")]
            let fraction = left as f32 / total as f32;
            hero = hero.child(
                div()
                    .flex()
                    .flex_wrap()
                    .justify_center()
                    .items_center()
                    .gap(px(10.0))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(7.0))
                            .h(px(32.0))
                            .pl(px(8.0))
                            .pr(px(11.0))
                            .rounded_full()
                            .bg(pal.paper_sunken())
                            .text_size(px(13.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(pal.ink_2())
                            .child(ring(fraction, pal.ink_2(), pal.rule(), 18.0, 2.4))
                            .child(clock_left(left)),
                    )
                    .child(
                        btn_icon(
                            "detail-qr",
                            Icon::QrCode,
                            if d.show_qr {
                                "Hide code"
                            } else {
                                "Open on phone"
                            },
                            Btn::Plain,
                            pal,
                        )
                        .on_click(Self::on_model(cx, Model::toggle_phone_qr)),
                    ),
            );
        }

        let mut facts = card(pal).child(info_row("Asked", asked_at(d.now, e.at), pal, true));
        if let Some(end) = e.ended_at {
            let label = match e.outcome {
                Outcome::Approved | Outcome::Denied => "Answered in",
                Outcome::TimedOut => "Waited",
                _ => "Took",
            };
            facts = facts.child(info_row(label, format::duration(end - e.at), pal, false));
        } else if e.outcome == Outcome::Waiting {
            facts = facts.child(info_row("Waiting", format::duration(d.now - e.at), pal, false));
        }
        if let Some(who) = decided(e).filter(|_| e.decider != Decider::Phone) {
            facts = facts.child(info_row("Decided", who, pal, false));
        }
        if let Some((label, value)) = s.fact {
            facts = facts.child(info_row(
                label,
                div().min_w(px(0.0)).truncate().font_family(MONO).text_size(px(12.5)).child(value),
                pal,
                false,
            ));
        }
        let mut page = div().flex().flex_col().gap(px(12.0)).child(hero);
        if d.show_qr {
            page = page.child(phone_qr(pal));
        }
        page = page.child(facts);
        if let Some(reason) = &e.reason {
            let color = if matches!(e.outcome, Outcome::Denied | Outcome::Failed | Outcome::Blocked) {
                pal.danger
            } else {
                pal.ink_2()
            };
            page = page.child(caption(reason.clone(), pal).px(px(4.0)).text_color(color));
        }
        if let Some(detail) = &e.detail {
            let mut w = well(pal);
            for line in detail.lines() {
                w = w.child(if line.trim().is_empty() {
                    div().h(px(6.0))
                } else {
                    div().min_w(px(0.0)).child(SharedString::from(line.to_owned()))
                });
            }
            page = page.child(w);
        }
        page.into_any_element()
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
        assert!(asked_at(now, now - 60).starts_with("Today, "));
    }

    #[test]
    fn outcome_filters_start_with_all() {
        assert_eq!(OUTCOMES[0], (None, "All"));
        assert!(OUTCOMES[1..].iter().all(|(o, label)| o.is_some_and(|o| o.label() == *label)));
    }
}
