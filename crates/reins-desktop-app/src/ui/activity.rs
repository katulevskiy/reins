//! Activity (`Desk-Activity.png`): this computer's log of requests (journal) as a list, and the request picked in it
//! as a plate beside. The list has what waits for the phone first, then the days; its filter is an inverse button
//! that opens a menu of outcomes and kinds, each outcome with how many. The plate: the integration's icon and the
//! label line with the time, the request in the serif with its target in italic and its details in mono, how it ended
//! and where, the facts as rows, a push's commits (each opening into its files and their diff), and the request as a
//! terminal showed it, in a mono well.

use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::prelude::FluentBuilder as _;
use gpui::{
    Anchor, AnyElement, Context, Div, FontWeight, Hsla, InteractiveElement as _, IntoElement, ParentElement as _,
    SharedString, Stateful, StatefulInteractiveElement as _, Styled as _, div, px,
};
use reins_desktop::journal::{Decider, Entry, Kind, NetRecord, Outcome, Tally};

use reins_desktop::pushes::{CommitInfo, FileStatus, Push, RefChange, RefUpdate};
use reins_proto::attention::{self, Attention, Level};

use super::diff::{self, visible};
use super::icons::{Glyph, glyph};
use super::parts::{FactLook, fact_rows, italic, phone_qr, wrapped};
use super::story::{Ask, Fact, FactStyle, answer, transcript};
use super::{Data, ROWS, Root, kit};
use crate::backend::ACTIVITY_LIMIT;
use crate::format;
use crate::model::Model;
use crate::theme::{MONO, NextText as _, Palette, SANS};

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
    /// Activity (`DeskNext-Home.png` 03): the wallpaper behind the title, the days as frosted panes (each with its
    /// counts, folding away), and the request picked in a panel on the right.
    pub(super) fn activity(&mut self, d: &Data, pal: Palette, cx: &mut Context<'_, Self>) -> AnyElement {
        let all = d.activity();
        let (outcome, kind) = (self.outcome_filter, self.kind_filter);
        let shown: Vec<&Entry> = all.iter().filter(|e| matches(e, outcome, kind)).collect();
        let total = shown.len();
        // What waits comes first, then the rest by day (the log is newest first).
        let (waiting, rest): (Vec<&Entry>, Vec<&Entry>) =
            shown.iter().take(self.rows_shown).partition(|e| e.outcome == Outcome::Waiting);
        let selected = self.selected.as_ref().and_then(|id| all.iter().find(|e| &e.id == id));
        let glass = kit::Glass::over(true, !d.saved.solid_panes);
        let surface = Self::row_surface(glass, pal);

        let header = div()
            .relative()
            .flex()
            .items_end()
            .gap(px(12.0))
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .title()
                    .text_color(if pal.dark {
                        gpui::white()
                    } else {
                        pal.ink()
                    })
                    .child("Activity"),
            )
            .when(!all.is_empty(), |h| h.child(self.filter_button(all, pal, cx)));

        let mut list = div().relative().flex().flex_col().gap(px(14.0)).mt(px(super::home::UNDER_TITLE));
        if all.is_empty() {
            list = list.child(kit::empty_state("Nothing asked yet.", 280.0, pal)).child(kit::pane(
                Self::test_panel(d, true, pal, cx),
                glass,
                kit::pane::RADIUS,
                pal,
            ));
        } else if total == 0 {
            list = list.child(div().text_size(px(14.0)).text_color(pal.ink_meta()).child("No matches"));
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
            for (label, entries) in groups {
                let open = (label == "Waiting" || label == "Today") != self.days_toggled.contains(&label);
                let t = Tally::of(entries.iter().copied());
                // What the phone decided without the user counts on its own, as on Home.
                let yes = (t.approved + t.allowed).saturating_sub(t.auto - t.auto_denied);
                let no = (t.denied + t.blocked).saturating_sub(t.auto_denied);
                let count = |g: Glyph, n: usize, color: Hsla| {
                    div()
                        .flex()
                        .items_center()
                        .gap(px(3.0))
                        .text_color(color)
                        .child(glyph(g, 13.0, color))
                        .child(n.to_string())
                };
                let toggle = label.clone();
                let head = div()
                    .id(SharedString::from(format!("day-{label}")))
                    .flex()
                    .items_center()
                    .gap(px(10.0))
                    .px(px(14.0))
                    .pt(px(12.0))
                    .pb(px(if open {
                        4.0
                    } else {
                        12.0
                    }))
                    .label()
                    .text_color(pal.ink_2())
                    .cursor_pointer()
                    .child(div().flex_1().child(label.clone()))
                    .when(yes > 0, |c| c.child(count(Glyph::Check, yes, pal.success)))
                    .when(t.auto > 0, |c| c.child(count(Glyph::Autopilot, t.auto, pal.violet())))
                    .when(no > 0, |c| c.child(count(Glyph::Close, no, pal.danger)))
                    .when(t.timed_out > 0, |c| c.child(count(Glyph::TimerOff, t.timed_out, pal.warning)))
                    .child(glyph(
                        if open {
                            Glyph::ChevronDown
                        } else {
                            Glyph::ChevronRight
                        },
                        14.0,
                        pal.ink_meta(),
                    ))
                    .on_click(Self::on_view(cx, move |this, _| {
                        if !this.days_toggled.remove(&toggle) {
                            this.days_toggled.insert(toggle.clone());
                        }
                    }));
                let mut rows: Vec<AnyElement> = Vec::new();
                if open {
                    for e in &entries {
                        let id = e.id.clone();
                        let picked = sel.as_ref() == Some(&e.id);
                        rows.push(
                            Self::entry_row(e, "activity", d, pal, picked, surface)
                                .on_click(Self::on_view(cx, move |this, _| this.selected = Some(id.clone())))
                                .into_any_element(),
                        );
                    }
                }
                list = list.child(kit::pane(
                    div()
                        .flex()
                        .flex_col()
                        .when(open, |p| p.pb(px(4.0)))
                        .child(head)
                        .children(kit::pane::with_rules(rows, 62.0, glass, pal)),
                    glass,
                    kit::pane::RADIUS,
                    pal,
                ));
            }
            let mut foot = div().flex().items_center().gap(px(12.0)).px(px(6.0)).child(
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
            list = list.child(foot);
        }

        let page = div().id("activity-list").relative().flex_1().min_w(px(0.0)).h_full().overflow_y_scroll().child(
            div()
                .relative()
                .w_full()
                .min_h_full()
                .child(kit::wallpaper::band(d.saved.wallpaper(pal.dark), 360.0, pal))
                .child(div().relative().px(px(super::home::SIDE)).pt(px(40.0)).pb(px(40.0)).child(header).child(list)),
        );
        let mut out = div().relative().flex().size_full().child(page);
        if let Some(e) = selected {
            self.load_push(e, cx);
            out = out.child(self.entry_detail(e, d, pal, cx));
        }
        out.into_any_element()
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
            .gap(px(6.0))
            .h(px(36.0))
            .px(px(13.0))
            .rounded_full()
            .bg(pal.paper_raised())
            .border_1()
            .border_color(pal.rule())
            .text_color(pal.ink())
            .text_size(px(14.0))
            .font_weight(FontWeight::SEMIBOLD)
            .whitespace_nowrap()
            .cursor_pointer()
            .active(|s| s.opacity(kit::button::PRESSED))
            .child(glyph(Glyph::Filter, 15.0, pal.ink()))
            .child(filter_label(outcome, kind))
            .child(glyph(
                if open {
                    Glyph::ChevronUp
                } else {
                    Glyph::ChevronDown
                },
                13.0,
                pal.ink_2(),
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

    /// The push kept for `e`, read when it is picked or waits (again once it is answered: a push that was not
    /// approved is not kept); the commits and files open are another request's no more.
    pub(super) fn load_push(&mut self, e: &Entry, cx: &Context<'_, Self>) {
        if self.push.as_ref().is_some_and(|(id, outcome, _)| *id == e.id && *outcome == e.outcome) {
            return;
        }
        if self.push.as_ref().is_some_and(|(id, _, _)| *id != e.id) {
            self.commits_open.clear();
            self.files_open.clear();
        }
        let kept = is_push(e).then(|| self.model.read(cx).kept_push(&e.id)).flatten();
        self.push = Some((e.id.clone(), e.outcome, kept.map(Arc::new)));
    }

    /// One request in the panel on the right (420 px; 600 for a push's code): its icon and Close, the request with its
    /// target at 600, how it ended, its facts, and a push's commits with the one picked opened.
    fn entry_detail(&self, e: &Entry, d: &Data, pal: Palette, cx: &mut Context<'_, Self>) -> AnyElement {
        let s = Ask::of(e);
        let (ic, tone, said) = answer(e, &d.phone());
        let verdict = pal.tone(tone);
        let mark = match e.outcome {
            Outcome::Denied | Outcome::Blocked => Glyph::Close,
            Outcome::Approved | Outcome::Allowed => Glyph::Check,
            _ => ic,
        };
        let push = self.kept_for(e);
        let pushed = matches!(e.outcome, Outcome::Approved | Outcome::Allowed);
        let host = push.as_ref().filter(|_| pushed).and_then(|p| {
            let branch = p.summary.updates.iter().find(|u| u.change != RefChange::Delete).and_then(|u| u.branch())?;
            diff::commits_link(&p.repo, branch)
        });
        let took = e.ended_at.map(|end| format::duration(end - e.at));
        let net = e.nets.as_ref().map(|n| Self::net_panel(n, d, pal, cx));
        let mut panel = div()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .child(div().mr(px(4.0)).mb(px(4.0)).child(kit::request_icon(
                        s.mark,
                        s.badge,
                        s.field,
                        30.0,
                        pal.paper_raised(),
                        pal,
                    )))
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.0))
                            .truncate()
                            .text_size(px(12.5))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(pal.ink_meta())
                            .child(format!("{} · {}", s.label(), format::when(d.now, e.at))),
                    )
                    .when_some(host, |h, link| {
                        let url = link.url.clone();
                        h.child(
                            div()
                                .id("open-host")
                                .text_size(px(14.5))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(pal.ink_2())
                                .cursor_pointer()
                                .hover(|s| s.text_color(pal.ink()))
                                .child(format!("Open on {}", link.name))
                                .on_click(move |_, _, cx| cx.open_url(&url)),
                        )
                    })
                    .child(
                        kit::icon_button("detail-close", Glyph::Close, 32.0, pal)
                            .on_click(Self::on_view(cx, |this, _| this.selected = None)),
                    ),
            )
            .child(div().mt(px(14.0)).ask_small().text_color(pal.ink()).child(italic(&s.sentence)))
            .when_some(s.details.clone(), |c, details| {
                c.child(
                    div()
                        .mt(px(6.0))
                        .font_family(MONO)
                        .text_size(px(13.0))
                        .line_height(px(19.0))
                        .text_color(pal.ink_meta())
                        .child(wrapped(&details)),
                )
            })
            .when_some(e.effects.as_ref().map(effects_text), |c, text| {
                c.child(
                    div()
                        .mt(px(10.0))
                        .text_size(px(13.0))
                        .line_height(px(19.0))
                        .text_color(pal.ink_meta())
                        .child(wrapped(&text)),
                )
            })
            .when_some(net, gpui::ParentElement::child)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .mt(px(8.0))
                    .text_size(px(14.5))
                    .text_color(pal.ink_2())
                    .child(glyph(mark, 16.0, verdict))
                    .child(match took {
                        Some(t) if e.outcome != Outcome::Waiting => format!("{said} · {t}"),
                        _ => said,
                    }),
            );
        // Decided on the phone without the user: who, and why, and "This was wrong".
        if let Some(why) = Self::why_panel(e, d, pal, cx) {
            panel = panel.child(why);
        }
        if e.outcome == Outcome::Waiting {
            let (total, left) = d.wait_left(e);
            panel = panel.child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap(px(12.0))
                    .mt(px(12.0))
                    .child(kit::timer(left, i64::try_from(total).unwrap_or(i64::MAX), pal.ink()))
                    .when_some(super::autopilot::lean_pill(e, false, pal), gpui::ParentElement::child)
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
                panel = panel.child(div().mt(px(12.0)).child(phone_qr(pal)));
            }
        }

        let fact = |key: &str, value: String| Fact {
            key: key.to_owned(),
            value,
            style: FactStyle::Text,
            tail: None,
        };
        let mut rows = Vec::new();
        if let Some(who) = decided(e).filter(|_| e.decider != Decider::Phone) {
            rows.push(fact("Decided", who.to_owned()));
        }
        rows.extend(s.facts);
        if !rows.is_empty() {
            panel = panel.child(kit::section(None, "Details", None, pal).px(px(0.0)).pt(px(20.0))).child(fact_rows(
                &rows,
                FactLook::Detail,
                kit::Glass::Solid,
                pal,
            ));
        }
        if let Some(push) = &push {
            let ci = self.picked_commit(push);
            panel = panel.child(div().mt(px(6.0)).child(self.commit_detail(push, ci, true, pal, cx)));
            panel = panel.child(Self::commit_list(push, ci, kit::Glass::Solid, pal, cx));
        }
        // The request as a terminal showed it, last and quiet.
        let mut log = div()
            .flex()
            .flex_col()
            .px(px(14.0))
            .py(px(10.0))
            .rounded(px(14.0))
            .bg(pal.paper_sunken())
            .font_family(MONO)
            .text_size(px(12.5))
            .line_height(px(19.0))
            .text_color(pal.ink_2())
            .min_w(px(0.0))
            .overflow_hidden();
        for line in transcript(e, &d.phone()) {
            log = log.child(div().min_w(px(0.0)).child(SharedString::from(line)));
        }
        panel = panel.child(kit::section(None, "Log", None, pal).px(px(0.0)).pt(px(20.0))).child(log);
        let width = if push.is_some() {
            600.0
        } else {
            420.0
        };
        div()
            .id("activity-detail")
            .flex_none()
            .w(px(width))
            .h_full()
            .overflow_y_scroll()
            .bg(pal.paper_raised())
            .border_l_1()
            .border_color(pal.rule())
            .shadow(vec![
                gpui::BoxShadow::new(px(-30.0), px(0.0), crate::theme::rgba(0, 0.18))
                    .blur_radius(px(60.0))
                    .spread_radius(px(-40.0)),
            ])
            .child(div().px(px(24.0)).pt(px(26.0)).pb(px(30.0)).child(panel))
            .into_any_element()
    }

    /// The push read for `e` ([`Self::load_push`]), when it lists commits.
    pub(super) fn kept_for(&self, e: &Entry) -> Option<Arc<Push>> {
        self.push
            .as_ref()
            .filter(|(id, _, _)| *id == e.id)
            .and_then(|(_, _, push)| push.clone())
            .filter(|p| p.summary.updates.iter().any(|u| !u.commits.is_empty()))
    }

    /// The commit of `push` that shows opened: the one picked, else the first worth a closer look, else the first.
    pub(super) fn picked_commit(&self, push: &Push) -> usize {
        let all: Vec<&CommitInfo> = push.summary.updates.iter().flat_map(|u| &u.commits).collect();
        if let Some(ci) = self.commits_open.iter().next().copied().filter(|ci| *ci < all.len()) {
            return ci;
        }
        flagged_first(&all, |c| attention::commit(c)).first().map_or(0, |(ci, _)| *ci)
    }

    /// A push's commits (`DeskNext-Ask.png`): "Commits" with how many and how many to look at, then a pane of rows
    /// (subject, hash, lines, why it deserves a closer look); a row opens that commit beside. The commits worth a
    /// closer look ([`attention`]) come first; each still opens by its place in the push. All of it is the
    /// repository's text: shown through [`diff::visible`], never interpreted.
    pub(super) fn commit_list(
        push: &Push,
        picked: usize,
        glass: kit::Glass,
        pal: Palette,
        cx: &mut Context<'_, Self>,
    ) -> Div {
        let updates = &push.summary.updates;
        let total: u64 = updates.iter().map(|u| u64::from(u.commit_count)).sum();
        let all: Vec<&CommitInfo> = updates.iter().flat_map(|u| &u.commits).collect();
        let rows = flagged_first(&all, |c| attention::commit(c));
        let flagged = rows.iter().filter(|(_, mark)| mark.is_some()).count();
        let mut items: Vec<AnyElement> = Vec::new();
        for (ci, mark) in rows {
            let c = all[ci];
            let row = kit::pane_row()
                .id(("commit", ci))
                .min_h(px(58.0))
                .cursor_pointer()
                .when_else(
                    ci == picked,
                    |r| {
                        r.bg(pal.violet().opacity(if pal.dark {
                            0.16
                        } else {
                            0.10
                        }))
                    },
                    |r| r.hover(|h| h.bg(pal.ink().opacity(0.035))),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_w(px(0.0))
                        .child(div().row().text_color(pal.ink()).min_w(px(0.0)).truncate().child(visible(&c.subject)))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(8.0))
                                .mt(px(2.0))
                                .child(
                                    div()
                                        .font_family(MONO)
                                        .text_size(px(12.5))
                                        .text_color(pal.ink_meta())
                                        .child(visible(&c.sha.chars().take(8).collect::<String>())),
                                )
                                .when_some(lines(c.additions, c.deletions), |r, (a, d)| {
                                    r.child(added_removed(&a, &d, pal))
                                }),
                        ),
                )
                .when_some(mark.as_ref(), |r, mark| r.child(flag_pill(reasons(mark), mark.level, pal)))
                .child(kit::pane::chevron(pal))
                .on_click(Self::on_view(cx, move |this, _| {
                    this.commits_open.clear();
                    this.commits_open.insert(ci);
                }));
            items.push(row.into_any_element());
        }
        let shown = all.len() as u64;
        if total > shown {
            items.push(
                kit::pane_row()
                    .child(div().flex_1().row().text_color(pal.ink()).child(format!("+{} more", total - shown)))
                    .into_any_element(),
            );
        }
        let (count, look) = commits_header(total, all.len(), flagged);
        let end = div()
            .flex()
            .items_center()
            .gap(px(8.0))
            .when(updates.iter().any(rewrites_history), |h| {
                h.child(flag_pill("Rewrites history".to_owned(), Level::Close, pal))
            })
            .child(div().text_color(pal.ink_2()).child(format!(
                "{}{}",
                count.trim_start_matches("Commits · "),
                look.map(|l| format!(" {l}")).unwrap_or_default()
            )))
            .into_any_element();
        div()
            .flex()
            .flex_col()
            .child(kit::section(Some(Glyph::GitCommit), "Commits", Some(end), pal))
            .child(kit::group(items, 14.0, glass, pal))
    }

    /// One commit opened (`DeskNext-Ask.png`, `DeskNext-Pushes.png`): its hash (and subject, `titled`), then each file
    /// with why it deserves a closer look and its lines; the diff of the files worth a look shown, the others behind
    /// "Show the diff". Hidden characters are shown and named, never applied.
    pub(super) fn commit_detail(
        &self,
        push: &Push,
        ci: usize,
        titled: bool,
        pal: Palette,
        cx: &mut Context<'_, Self>,
    ) -> Div {
        let all: Vec<&CommitInfo> = push.summary.updates.iter().flat_map(|u| &u.commits).collect();
        let Some(c) = all.get(ci).copied() else {
            return div();
        };
        let hash = visible(&c.sha.chars().take(8).collect::<String>());
        let head = if titled {
            format!("{hash} · {}", visible(&c.subject))
        } else {
            hash
        };
        let mut files = div().flex().flex_col().gap(px(10.0));
        for (fi, mark) in flagged_first(&c.files, attention::file) {
            let f = &c.files[fi];
            let open = mark.is_some() || self.files_open.contains(&(ci, fi));
            let tag = match (f.binary, f.status, &f.patch) {
                (true, _, _) => Some("binary"),
                (_, FileStatus::Added, _) => Some("new"),
                (_, FileStatus::Deleted, _) => Some("deleted"),
                (_, _, None) => Some("no diff kept"),
                _ => None,
            };
            let line = div()
                .flex()
                .items_center()
                .gap(px(10.0))
                .min_w(px(0.0))
                .child(glyph(Glyph::File, 16.0, pal.ink_2()))
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.0))
                        .truncate()
                        .font_family(MONO)
                        .text_size(px(13.5))
                        .text_color(pal.ink())
                        .child(visible(&f.path)),
                )
                .when_some(mark.as_ref(), |r, mark| r.child(flag_pill(reasons(mark), mark.level, pal)))
                .when_some(tag, |r, t| {
                    r.child(div().flex_none().text_size(px(12.5)).text_color(pal.ink_meta()).child(t))
                })
                .when_some(lines(f.additions.map(u64::from), f.deletions.map(u64::from)), |r, (a, d)| {
                    r.child(added_removed(&a, &d, pal))
                });
            files = files.child(line);
            match f.patch.as_deref() {
                Some(patch) if open => {
                    files = files.child(patch_block(patch, f.patch_cut, pal));
                    if patch.chars().any(diff::hidden) {
                        files = files.child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(6.0))
                                .text_size(px(13.5))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(pal.warning)
                                .child(glyph(Glyph::Warning, 14.0, pal.warning))
                                .child("A hidden character is in a line. It is shown, never applied."),
                        );
                    }
                }
                Some(_) => {
                    files = files.child(kit::pane(
                        kit::pane::clickable(kit::pane_row(), SharedString::from(format!("diff-{ci}-{fi}")), pal)
                            .child(div().flex_1().row().text_color(pal.ink()).child("Show the diff"))
                            .child(kit::pane::chevron(pal))
                            .on_click(Self::on_view(cx, move |this, _| {
                                this.files_open.insert((ci, fi));
                            })),
                        kit::Glass::Solid,
                        14.0,
                        pal,
                    ));
                }
                None => {}
            }
        }
        let listed = c.files.len();
        if let Some(more) = c.files_changed.map(|n| n as usize).filter(|n| *n > listed).map(|n| n - listed) {
            files = files.child(div().text_size(px(13.0)).text_color(pal.ink_meta()).child(if more == 1 {
                "and 1 more file".to_owned()
            } else {
                format!("and {more} more files")
            }));
        }
        div().flex().flex_col().child(kit::section(Some(Glyph::File), head, None, pal)).child(if titled {
            files
        } else {
            div().child(kit::pane(files.px(px(20.0)).py(px(18.0)), kit::Glass::Solid, kit::pane::RADIUS, pal))
        })
    }
}

/// Whether `e` is a push to a host (not a fetch): the kind of request that keeps its commits.
/// Where a safety net stands in Activity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum NetState {
    /// It can be restored.
    Restore,
    Restoring,
    Restored,
    /// Past its keeping time.
    Gone,
}

fn net_state(net: &NetRecord, now: i64, restored: bool, restoring: bool) -> NetState {
    if restored {
        NetState::Restored
    } else if restoring {
        NetState::Restoring
    } else if now >= net.until {
        NetState::Gone
    } else {
        NetState::Restore
    }
}

impl Root {
    /// "Netted · Restore": what the safety net kept, and putting it back.
    fn net_panel(net: &NetRecord, d: &Data, pal: Palette, cx: &mut Context<'_, Self>) -> AnyElement {
        let state = net_state(
            net,
            d.now,
            d.restored_nets.contains(&net.plan),
            d.restoring_net.as_deref() == Some(net.plan.as_str()),
        );
        let label = match state {
            NetState::Restore | NetState::Restoring => "Restore",
            NetState::Restored => "Restored",
            NetState::Gone => "Kept copy gone",
        };
        let glyph_for = if state == NetState::Restored {
            Glyph::Check
        } else {
            Glyph::Undo
        };
        let button = kit::button("net-restore", kit::Look::Soft, kit::Size::Small, Some(glyph_for), label, pal);
        let button = if state == NetState::Restore && d.restoring_net.is_none() {
            let plan = net.plan.clone();
            button.on_click(Self::on_model(cx, move |m, cx| m.restore_net(plan.clone(), cx)))
        } else {
            button.opacity(if state == NetState::Restored {
                1.0
            } else {
                kit::button::DISABLED
            })
        };
        div()
            .flex()
            .flex_col()
            .gap(px(6.0))
            .mt(px(10.0))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .text_size(px(13.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(pal.ink_2())
                    .child(glyph(Glyph::Undo, 14.0, pal.ink_2()))
                    .child("Netted"),
            )
            .child(
                div()
                    .text_size(px(13.0))
                    .line_height(px(19.0))
                    .text_color(pal.ink_meta())
                    .child(wrapped(&net.lines.join("\n"))),
            )
            .child(div().flex().child(button))
            .into_any_element()
    }
}

/// What a request does, as the consequence engine read it on this computer: the level, then one line per effect, and
/// what could not be read. Decides nothing yet (Decisions v2 P1).
fn effects_text(s: &reins_policy::effects::Summary) -> String {
    use reins_policy::effects::Certainty;
    let mut out = vec![match s.certainty {
        Certainty::Full => format!("What it does · {}", s.level.label()),
        Certainty::Partial => format!("What it does · {} · partly read", s.level.label()),
        Certainty::None => "What it does · couldn't be read".to_owned(),
    }];
    out.extend(s.lines.iter().cloned());
    if let Some(first) = s.unresolved.first() {
        let more = s.unresolved.len() - 1;
        out.push(if more > 0 {
            format!("Couldn't read {first} and {more} more")
        } else {
            format!("Couldn't read {first}")
        });
    }
    out.join("\n")
}

fn is_push(e: &Entry) -> bool {
    e.kind == Kind::Git && e.what.starts_with("push to ")
}

/// The rows in the order they show: Close first, then Look, then the rest, each band in the push's order. Every row
/// keeps its index in `items`, which is what opens it.
fn flagged_first<T>(items: &[T], mark: impl Fn(&T) -> Option<Attention>) -> Vec<(usize, Option<Attention>)> {
    let mut rows: Vec<(usize, Option<Attention>)> = items.iter().map(mark).enumerate().collect();
    rows.sort_by_key(|(_, mark)| match mark.as_ref().map(|m| m.level) {
        Some(Level::Close) => 0,
        Some(Level::Look) => 1,
        None => 2,
    });
    rows
}

/// The header's count ("Commits · 8") and, when some deserve a closer look, how many ("· 2 to look at"; "· 2 of 50
/// shown to look at" when the push has more commits than it lists, since only the listed ones were read).
fn commits_header(total: u64, listed: usize, flagged: usize) -> (String, Option<String>) {
    let look = match flagged {
        0 => None,
        n if listed as u64 >= total => Some(format!("· {n} to look at")),
        n => Some(format!("· {n} of {listed} shown to look at")),
    };
    (format!("Commits · {total}"), look)
}

/// The mark's fixed labels, heaviest first: "Changes CI · Hidden characters". Never the item's own text.
fn reasons(mark: &Attention) -> String {
    mark.reasons.iter().map(|r| r.label.as_str()).collect::<Vec<_>>().join(" · ")
}

/// A ref the push moves to a commit that is not ahead of the old one (or could not tell): a force push.
fn rewrites_history(u: &RefUpdate) -> bool {
    u.change == RefChange::Update && u.fast_forward != Some(true)
}

/// How a mark reads: Close in `warning` at medium weight; Look quieter, in `ink-2`. Red stays for danger and holds.
fn flag_style(level: Level, pal: Palette) -> (Hsla, FontWeight) {
    match level {
        Level::Close => (pal.warning, FontWeight::MEDIUM),
        Level::Look => (pal.ink_2(), FontWeight::NORMAL),
    }
}

/// Why something deserves a closer look, as a pill (24 px): Close in `warning` on itself at 13%, Look in `ink-2` on
/// `paper-sunken`.
fn flag_pill(text: String, level: Level, pal: Palette) -> Div {
    let color = flag_style(level, pal).0;
    let bg = match level {
        Level::Close => color.opacity(0.13),
        Level::Look => pal.paper_sunken(),
    };
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap(px(4.0))
        .h(px(24.0))
        .px(px(9.0))
        .rounded_full()
        .bg(bg)
        .font_family(SANS)
        .text_size(px(12.5))
        .font_weight(FontWeight::SEMIBOLD)
        .whitespace_nowrap()
        .text_color(color)
        .child(glyph(Glyph::Warning, 12.0, color))
        .child(text)
}

/// `+a` and `−b`, when both were counted.
fn lines(additions: Option<u64>, deletions: Option<u64>) -> Option<(String, String)> {
    Some((format!("+{}", additions?), format!("−{}", deletions?)))
}

/// Lines added in green and removed in red, in mono.
fn added_removed(added: &str, removed: &str, pal: Palette) -> Div {
    div()
        .flex()
        .flex_none()
        .gap(px(6.0))
        .font_family(MONO)
        .text_size(px(12.5))
        .font_weight(FontWeight::MEDIUM)
        .child(div().text_color(pal.success).child(added.to_owned()))
        .child(div().text_color(pal.danger).child(removed.to_owned()))
}

/// A file's diff as plain text (`.diff`): mono 12.5 on `paper-sunken`, hunk headers in violet, removed lines on a red
/// wash, added ones on a green one; every line made [`visible`], its hidden characters marked in amber, and wrapped
/// whole.
fn patch_block(patch: &str, cut: bool, pal: Palette) -> Div {
    let mut block = div()
        .flex()
        .flex_col()
        .rounded(px(14.0))
        .bg(pal.paper_sunken())
        .font_family(MONO)
        .text_size(px(12.5))
        .line_height(px(20.0))
        .min_w(px(0.0))
        .overflow_hidden();
    for line in patch.lines() {
        let (color, wash, hunk) = match line.chars().next() {
            Some('+') => (pal.ink(), Some(pal.success.opacity(0.16)), false),
            Some('-') => (pal.ink(), Some(pal.danger.opacity(0.14)), false),
            Some('@') => (pal.violet(), None, true),
            _ => (pal.ink(), None, false),
        };
        block = block.child(
            div()
                .px(px(14.0))
                .when(hunk, |l| l.py(px(6.0)))
                .min_w(px(0.0))
                .text_color(color)
                .when_some(wash, gpui::Styled::bg)
                .child(marked(&super::parts::wrap_points(&visible(line)), pal)),
        );
    }
    if cut {
        block = block.child(
            div()
                .px(px(14.0))
                .py(px(6.0))
                .font_family(SANS)
                .text_color(pal.ink_meta())
                .child("The rest of this diff was not kept."),
        );
    }
    block
}

/// A [`visible`] line with each named hidden character (`⟨U+202E⟩`) marked: dark on amber.
fn marked(line: &str, pal: Palette) -> gpui::StyledText {
    let mut highlights = Vec::new();
    let mut at = 0;
    while let Some(start) = line[at..].find('⟨').map(|i| i + at) {
        let Some(end) = line[start..].find('⟩').map(|i| i + start + '⟩'.len_utf8()) else {
            break;
        };
        highlights.push((
            start..end,
            gpui::HighlightStyle {
                color: Some(crate::theme::token(crate::theme::tokens::INK, false)),
                background_color: Some(crate::theme::Field::Warn.field().opacity(if pal.dark {
                    0.9
                } else {
                    0.75
                })),
                ..gpui::HighlightStyle::default()
            },
        ));
        at = end;
    }
    gpui::StyledText::new(line.to_owned()).with_highlights(highlights)
}

#[cfg(test)]
mod tests {
    use reins_desktop::pushes::FileChange;

    use super::*;

    fn entry(kind: Kind, outcome: Outcome) -> Entry {
        let mut e = Entry::new(kind, "x");
        e.outcome = outcome;
        e
    }

    #[test]
    fn netted_rows_offer_restore_until_the_copy_is_gone() {
        let net = NetRecord {
            plan: "nplan01".to_owned(),
            kinds: vec![reins_policy::effects::nets::NetKind::Trash],
            lines: vec!["Keeps 2 deleted paths in the Reins trash for 7 days".to_owned()],
            until: 1_000,
        };
        assert_eq!(net_state(&net, 10, false, false), NetState::Restore);
        assert_eq!(net_state(&net, 10, false, true), NetState::Restoring);
        assert_eq!(net_state(&net, 10, true, false), NetState::Restored);
        assert_eq!(net_state(&net, 1_000, false, false), NetState::Gone);
        assert_eq!(net_state(&net, 1_000, true, false), NetState::Restored);
    }

    #[test]
    fn effects_read_as_a_level_and_plain_lines() {
        let ctx = reins_policy::effects::Context::new(
            "/home/me/Documents/dev/app",
            reins_policy::effects::Workspace::standard("/home/me"),
        );
        let s = reins_policy::effects::analyze_command("rm -rf target/a target/b && rm -rf \"$X\"", &ctx).summary();
        let text = effects_text(&s);
        // A delete of a place not known may take anything: it can't be undone.
        assert!(text.starts_with("What it does · Can't be undone · partly read"), "{text}");
        assert!(text.contains("Deletes 2 paths in build output"), "{text}");
        assert!(text.contains("Couldn't read the value of $X"), "{text}");
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

    fn file(path: &str, diff: Option<&str>) -> FileChange {
        FileChange {
            patch: diff.map(str::to_owned),
            ..FileChange::new(path.to_owned(), FileStatus::Modified, Some(3), Some(1), false)
        }
    }

    fn commit(i: usize, subject: &str, files: Vec<FileChange>) -> CommitInfo {
        CommitInfo {
            sha: format!("{i:040x}"),
            subject: subject.to_owned(),
            author: "Dana Reyes <dana@acme.dev>".to_owned(),
            date: 1_800_000_000,
            files_changed: u32::try_from(files.len()).ok(),
            additions: Some(3),
            deletions: Some(1),
            files,
        }
    }

    /// The header counts the commits worth a closer look; out of the listed ones when the push has more.
    #[test]
    fn the_header_says_how_many_to_look_at() {
        let header = |total, listed, flagged| {
            let (count, look) = commits_header(total, listed, flagged);
            look.map_or(count.clone(), |look| format!("{count} {look}"))
        };
        assert_eq!(header(8, 8, 2), "Commits · 8 · 2 to look at");
        assert_eq!(header(8, 8, 0), "Commits · 8");
        assert_eq!(header(1, 1, 1), "Commits · 1 · 1 to look at");
        assert_eq!(header(120, 50, 2), "Commits · 120 · 2 of 50 shown to look at");
        assert_eq!(header(120, 50, 0), "Commits · 120");
    }

    /// Close rows first, then Look, then the rest, each band in the push's order; every row keeps the index that
    /// opens it.
    #[test]
    fn flagged_rows_come_first_and_keep_their_index() {
        let commits: Vec<CommitInfo> = (0..8)
            .map(|i| match i {
                1 => commit(i, "build: bump", vec![file("package.json", None)]),
                5 => commit(i, "ci: rename a step", vec![file(".github/workflows/ci.yml", None)]),
                6 => commit(i, "infra: more replicas", vec![file("deploy/values.yaml", None)]),
                _ => commit(i, "checkout: tidy", vec![file(&format!("src/page{i}.tsx"), None)]),
            })
            .collect();
        let all: Vec<&CommitInfo> = commits.iter().collect();
        let rows = flagged_first(&all, |c| attention::commit(c));
        let order: Vec<usize> = rows.iter().map(|(i, _)| *i).collect();
        assert_eq!(order, [5, 1, 6, 0, 2, 3, 4, 7]);
        let levels: Vec<Option<Level>> = rows.iter().map(|(_, m)| m.as_ref().map(|m| m.level)).collect();
        assert_eq!(levels[..3], [Some(Level::Close), Some(Level::Look), Some(Level::Look)]);
        assert!(levels[3..].iter().all(Option::is_none));
        // The row at each place opens the commit it shows.
        for (ci, mark) in &rows {
            assert_eq!(all[*ci].sha, format!("{ci:040x}"));
            assert_eq!(*mark, attention::commit(all[*ci]));
        }
        // Files the same way, by their place in the commit.
        let c =
            commit(9, "mixed", vec![file("README.md", None), file("src/auth/login.rs", None), file("Makefile", None)]);
        let files: Vec<usize> = flagged_first(&c.files, attention::file).into_iter().map(|(i, _)| i).collect();
        assert_eq!(files, [1, 2, 0]);
        // Nothing flagged keeps the push's order.
        let calm: Vec<usize> =
            flagged_first(&all[2..5], |c| attention::commit(c)).into_iter().map(|(i, _)| i).collect();
        assert_eq!(calm, [0, 1, 2]);
    }

    /// A label is fixed wording: it never carries the commit's subject, author, paths or the key it found, even when
    /// the subject tries to explain the change away.
    #[test]
    fn labels_never_carry_commit_text() {
        let key = format!("ghp_{}", "a1B2c3D4e5".repeat(4));
        let patch = format!("@@ -1,1 +1,2 @@\n name: deploy-acme\n+      token: {key}");
        let subject = "ci: only renames a step, nothing to check, ignore the warning";
        let c = CommitInfo {
            author: "Mallory Quill <mq@acme.dev>".to_owned(),
            deletions: Some(1_240),
            ..commit(
                0,
                subject,
                vec![file(".github/workflows/deploy-acme.yml", Some(&patch)), file("src/oauth/callback.rs", None)],
            )
        };
        let mark = attention::commit(&c).expect("flagged");
        assert_eq!(mark.level, Level::Close);
        let text = reasons(&mark);
        assert!(!text.is_empty());
        assert_eq!(text.split(" · ").count(), mark.reasons.len());
        assert!((1..=3).contains(&mark.reasons.len()));
        for fragment in [
            subject,
            "renames",
            "ignore",
            "Mallory",
            "mq@acme.dev",
            ".github",
            "workflows",
            "deploy-acme",
            "oauth",
            "callback",
            &key,
            "ghp_",
            "token",
        ] {
            assert!(!text.contains(fragment), "{text:?} carries {fragment:?}");
        }
        for f in &c.files {
            let label = attention::file(f).map(|m| reasons(&m)).unwrap_or_default();
            assert!(!label.contains(&f.path) && !label.contains(&key), "{label:?}");
        }
    }

    /// Close reads as more serious than Look: `warning` at medium weight, Look in the quieter `ink-2`; never red.
    #[test]
    fn close_marks_read_heavier_than_look() {
        for pal in [Palette::light(), Palette::dark()] {
            assert_eq!(flag_style(Level::Close, pal), (pal.warning, FontWeight::MEDIUM));
            let (look, weight) = flag_style(Level::Look, pal);
            assert_eq!((look, weight), (pal.ink_2(), FontWeight::NORMAL));
            assert_ne!(look, pal.warning);
            assert!(look != pal.danger && pal.warning != pal.danger);
        }
    }

    /// "Rewrites history" is a force push, or one that could not be checked; not a new branch or a fast-forward.
    #[test]
    fn force_pushes_rewrite_history() {
        let update = |change, fast_forward| RefUpdate {
            name: "refs/heads/main".to_owned(),
            change,
            old: "1".repeat(40),
            new: "2".repeat(40),
            fast_forward,
            commit_count: 1,
            commits: Vec::new(),
            files_changed: 0,
            files: Vec::new(),
            additions: None,
            deletions: None,
        };
        assert!(rewrites_history(&update(RefChange::Update, Some(false))));
        assert!(rewrites_history(&update(RefChange::Update, None)));
        assert!(!rewrites_history(&update(RefChange::Update, Some(true))));
        assert!(!rewrites_history(&update(RefChange::Create, None)));
        assert!(!rewrites_history(&update(RefChange::Delete, None)));
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
