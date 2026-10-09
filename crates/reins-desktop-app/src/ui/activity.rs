//! Activity: this computer's log of requests (journal), filtered by outcome and kind; a row opens to its detail.

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, Context, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _, SharedString,
    StatefulInteractiveElement as _, Styled as _, div, px,
};
use reins_desktop::journal::{Decider, Entry, Kind, Outcome};

use super::parts::{button, caption, card, chip, empty, fine, one_line, outcome_badge, page_header, phone_qr};
use super::{Data, ROWS, Root};
use crate::format;
use crate::model::{Model, Section};
use crate::theme::{MONO, Palette};

/// The outcome chips, in order.
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

/// Who decided, in a few words.
#[must_use]
pub fn decided(e: &Entry) -> &'static str {
    match (e.decider, e.outcome) {
        (Decider::Settings, _) => "by your rules",
        (Decider::Local, _) => "on this computer",
        (Decider::Phone, Outcome::Waiting) => "asking your phone",
        (Decider::Phone, Outcome::TimedOut) => "no answer from your phone",
        (Decider::Phone, Outcome::Failed) => "could not ask your phone",
        (Decider::Phone, _) => "on your phone",
    }
}

/// The service, as said in a row: a hook's rule without its `command:` or `file:` prefix.
#[must_use]
pub fn service_text(e: &Entry) -> Option<String> {
    let s = e.service.as_deref()?;
    let s = s.strip_prefix("command:").or_else(|| s.strip_prefix("file:")).unwrap_or(s);
    Some(s.to_owned())
}

/// "Command · Claude Code · git push --force*".
fn meta(e: &Entry) -> String {
    let kind = e.kind.label();
    let mut parts = vec![kind.to_owned()];
    // `git` asked by git, `ssh` by ssh: said once.
    parts.extend(e.source.clone().filter(|s| !s.eq_ignore_ascii_case(kind)));
    parts.extend(service_text(e).filter(|s| Some(s) != e.source.as_ref() && !s.eq_ignore_ascii_case(kind)));
    parts.join(" · ")
}

impl Root {
    /// One request. `scope` keeps ids apart where the same entry shows twice (Overview and Activity).
    pub(super) fn entry_row(
        &self,
        e: &Entry,
        scope: &'static str,
        d: &Data,
        pal: Palette,
        first: bool,
        cx: &mut Context<'_, Self>,
    ) -> AnyElement {
        let open = scope == "activity" && self.expanded.contains(&e.id);
        let id = e.id.clone();
        let head = div()
            .id(SharedString::from(format!("{scope}-{}", e.id)))
            .flex()
            .items_center()
            .gap(px(14.0))
            .px(px(16.0))
            .py(px(10.0))
            .min_w(px(0.0))
            .cursor_pointer()
            .hover(|s| s.bg(pal.control_fill.opacity(0.5)))
            .child(outcome_badge(SharedString::from(format!("{scope}-badge-{}", e.id)), e.outcome, pal))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w(px(0.0))
                    .gap(px(1.0))
                    .child(one_line(e.what.clone()).font_weight(FontWeight::MEDIUM))
                    .child(one_line(meta(e)).text_size(px(12.0)).text_color(pal.secondary)),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_none()
                    .items_end()
                    .gap(px(1.0))
                    .child(div().text_size(px(12.0)).text_color(pal.secondary).child(format::ago(d.now, e.at)))
                    .child(fine(decided(e), pal)),
            )
            .on_click(cx.listener(move |this, _, _, cx| {
                if scope == "activity" {
                    if !this.expanded.remove(&id) {
                        this.expanded.insert(id.clone());
                    }
                    cx.notify();
                } else {
                    // From the overview: open it in Activity.
                    this.expanded.insert(id.clone());
                    this.outcome_filter = None;
                    this.kind_filter = None;
                    this.model.update(cx, |m, cx| m.show_section(Section::Activity, cx));
                }
            }));
        div()
            .flex()
            .flex_col()
            .min_w(px(0.0))
            .when(!first, |r| r.border_t_1().border_color(pal.hairline))
            .child(head)
            .when(open, |r| r.child(Self::entry_detail(e, d.now, pal)))
            .into_any_element()
    }

    fn entry_detail(e: &Entry, now: i64, pal: Palette) -> AnyElement {
        let mut facts = vec![format!("Asked {}", format::when(now, e.at))];
        match e.ended_at {
            Some(end) => {
                facts.push(format!("ended {}", format::when(now, end)));
                facts.push(format!("took {}", format::duration(end - e.at)));
            }
            None if e.outcome == Outcome::Waiting => facts.push(format!("waiting {}", format::duration(now - e.at))),
            None => {}
        }
        let mut body = div().flex().flex_col().gap(px(8.0)).pl(px(116.0)).pr(px(16.0)).pb(px(14.0)).min_w(px(0.0));
        if let Some(detail) = &e.detail {
            body = body.child(
                div()
                    .px(px(12.0))
                    .py(px(10.0))
                    .rounded(px(8.0))
                    .bg(pal.control_fill)
                    .font_family(MONO)
                    .text_size(px(12.0))
                    .line_height(px(17.0))
                    .text_color(pal.text)
                    .min_w(px(0.0))
                    .child(detail.clone()),
            );
        }
        if let Some(reason) = &e.reason {
            let color = if matches!(e.outcome, Outcome::Denied | Outcome::Failed | Outcome::Blocked) {
                pal.danger
            } else {
                pal.secondary
            };
            body = body.child(caption(reason.clone(), pal).text_color(color));
        }
        body.child(fine(facts.join(" · "), pal)).into_any_element()
    }

    pub(super) fn activity(&mut self, d: &Data, pal: Palette, cx: &mut Context<'_, Self>) -> AnyElement {
        let all = d.activity();
        let (outcome, kind) = (self.outcome_filter, self.kind_filter);

        let mut outcomes = div().flex().flex_wrap().gap(px(6.0));
        for (i, (o, label)) in OUTCOMES.into_iter().enumerate() {
            let n = all.iter().filter(|e| matches(e, o, kind)).count();
            outcomes = outcomes.child(chip(("outcome", i), label, Some(n), outcome == o, pal).on_click(Self::on_view(
                cx,
                move |this, _| {
                    this.outcome_filter = o;
                    this.rows_shown = ROWS;
                },
            )));
        }
        let mut kinds = div().flex().flex_wrap().gap(px(6.0)).child(
            chip("kind-all", "Every kind", None, kind.is_none(), pal).on_click(Self::on_view(cx, |this, _| {
                this.kind_filter = None;
                this.rows_shown = ROWS;
            })),
        );
        for (i, k) in Kind::ALL.into_iter().enumerate() {
            let n = all.iter().filter(|e| matches(e, outcome, Some(k))).count();
            kinds = kinds.child(chip(("kind", i), k.label(), Some(n), kind == Some(k), pal).on_click(Self::on_view(
                cx,
                move |this, _| {
                    this.kind_filter = Some(k);
                    this.rows_shown = ROWS;
                },
            )));
        }

        let shown: Vec<&Entry> = all.iter().filter(|e| matches(e, outcome, kind)).collect();
        let total = shown.len();
        let list = if all.is_empty() {
            card(pal).child(empty(
                "Nothing yet. When an AI agent, git or ssh on this computer asks for something, it shows up here: \
                 what it was, who asked, and how it ended.",
                pal,
            ))
        } else if total == 0 {
            card(pal).child(empty("Nothing matches these filters.", pal))
        } else {
            let mut list = card(pal);
            for (i, e) in shown.iter().take(self.rows_shown).enumerate() {
                list = list.child(self.entry_row(e, "activity", d, pal, i == 0, cx));
            }
            list
        };

        let mut page = div()
            .flex()
            .flex_col()
            .gap(px(18.0))
            .child(
                div()
                    .flex()
                    .items_start()
                    .gap(px(16.0))
                    .child(
                        page_header(
                            "Activity",
                            "Everything asked for on this computer, and how it ended. Approving always happens on \
                             your phone; Reins only keeps the record here.",
                            pal,
                        )
                        .flex_1()
                        .min_w(px(0.0)),
                    )
                    .child(
                        button(
                            "activity-qr",
                            if d.show_qr {
                                "Hide code"
                            } else {
                                "Open on phone"
                            },
                            pal,
                            false,
                            true,
                        )
                        .on_click(Self::on_model(cx, Model::toggle_phone_qr)),
                    ),
            )
            .when(d.show_qr, |p| p.child(phone_qr(pal)))
            .child(div().flex().flex_col().gap(px(8.0)).child(outcomes).child(kinds))
            .child(list);
        if total > self.rows_shown {
            page = page.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(12.0))
                    .child(fine(format!("Showing {} of {total}", self.rows_shown), pal))
                    .child(
                        button("more", "Show more", pal, false, true)
                            .on_click(Self::on_view(cx, |this, _| this.rows_shown += ROWS)),
                    ),
            );
        } else if total > 0 {
            page = page.child(fine(
                format!("{} · the newest 500 are kept here", format::count(total as u64, "request", "requests")),
                pal,
            ));
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
    }

    #[test]
    fn rows_say_who_decided_and_what_for() {
        let mut e = entry(Kind::Command, Outcome::Denied).source(Some("Claude Code"));
        e.service = Some("command:rm -r".to_owned());
        assert_eq!(decided(&e), "on your phone");
        assert_eq!(service_text(&e).as_deref(), Some("rm -r"));
        assert_eq!(meta(&e), "Command · Claude Code · rm -r");
        let rules = entry(Kind::Command, Outcome::Allowed).decider(Decider::Settings);
        assert_eq!(decided(&rules), "by your rules");
        assert_eq!(decided(&entry(Kind::Ask, Outcome::Waiting)), "asking your phone");
        // A source that is also the kind or the service is said once.
        let ssh = entry(Kind::Ssh, Outcome::Approved).source(Some("ssh")).service(Some("github.com"));
        assert_eq!(meta(&ssh), "SSH · github.com");
        let ask = entry(Kind::Ask, Outcome::Approved).source(Some("Codex")).service(Some("Codex"));
        assert_eq!(meta(&ask), "Question · Codex");
    }

    #[test]
    fn outcome_chips_start_with_all() {
        assert_eq!(OUTCOMES[0], (None, "All"));
        assert!(OUTCOMES[1..].iter().all(|(o, label)| o.is_some_and(|o| o.label() == *label)));
    }
}
