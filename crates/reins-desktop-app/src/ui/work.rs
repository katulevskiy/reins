//! Overview's work session card: start one (how long, what for, what to read, which branches to push to), wait for
//! the phone, then the session running (its time left, what it allows) with "End now".

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, Context, Div, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _, SharedString,
    Stateful, StatefulInteractiveElement as _, Styled as _, Window, div, px,
};
use reins_desktop::doctor::Level;
use reins_desktop::work_session::Session;

use super::field::Field;
use super::parts::{button, caption, card, check, fine, group, level_mark, row, segment, segmented, waiting_dot};
use super::{Data, Root};
use crate::format;
use crate::model::Model;
use crate::theme::{FONT, MONO, Palette};
use crate::work::{self, Asking, Form, Length, PushRow, READS};

/// What a work session leaves out, whatever it allows.
const STILL_ASKS: &str = "Force pushes, deleting, the vault and purchases still ask.";
/// The width of the form's labels.
const LABEL: f32 = 112.0;

/// A form line: its label on the left, `body` beside it.
fn form_line(label: &'static str, body: impl IntoElement, pal: Palette, first: bool) -> Div {
    row(pal, first)
        .items_start()
        .py(px(12.0))
        .child(
            div()
                .flex_none()
                .w(px(LABEL))
                .pt(px(6.0))
                .text_color(pal.secondary)
                .font_weight(FontWeight::MEDIUM)
                .child(label),
        )
        .child(div().flex().flex_col().flex_1().min_w(px(0.0)).gap(px(8.0)).child(body))
}

/// An integration to read: on (with a check) or off.
fn read_chip(id: &'static str, label: &'static str, on: bool, pal: Palette) -> Stateful<Div> {
    div()
        .id(SharedString::from(format!("session-read-{id}")))
        .flex()
        .flex_none()
        .items_center()
        .gap(px(6.0))
        .h(px(28.0))
        .px(px(12.0))
        .rounded_full()
        .border_1()
        .text_size(px(12.5))
        .font_weight(FontWeight::MEDIUM)
        .cursor_pointer()
        .when_else(
            on,
            |c| c.bg(pal.accent_soft).border_color(pal.accent.opacity(0.4)).text_color(pal.accent),
            |c| {
                c.bg(pal.elevated)
                    .border_color(pal.hairline)
                    .text_color(pal.secondary)
                    .hover(|s| s.bg(pal.control_fill).text_color(pal.text))
            },
        )
        .when(on, |c| c.child(check(12.0, pal.accent)))
        .child(label)
}

/// The round mark at the start of the card's first line.
fn badge(inner: impl IntoElement, pal: Palette) -> Div {
    div()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(px(30.0))
        .rounded_full()
        .bg(pal.accent_soft)
        .child(inner)
}

/// A title with a line under it, taking the room a row has.
fn titled(title: impl Into<SharedString>, under: impl Into<SharedString>, pal: Palette) -> Div {
    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_w(px(0.0))
        .gap(px(1.0))
        .child(div().text_size(px(14.0)).font_weight(FontWeight::SEMIBOLD).child(title.into()))
        .child(caption(under, pal))
}

impl Root {
    /// The card: the session running, the request waiting on the phone, why it did not start, the form, or the
    /// invitation to start one.
    pub(super) fn work_card(
        &mut self,
        d: &Data,
        pal: Palette,
        window: &Window,
        cx: &mut Context<'_, Self>,
    ) -> AnyElement {
        let body = if let Some(session) = d.work_session() {
            Self::work_running(session, d, pal, cx)
        } else if let Some(asking) = &d.work.asking {
            Self::work_waiting(asking, pal)
        } else if let (Some(why), Some(form)) = (&d.work.failed, &d.work.form) {
            Self::work_failed(why, form, pal, cx)
        } else if let Some(form) = &d.work.form {
            self.work_form(form, pal, window, cx)
        } else {
            Self::work_invite(d, pal, cx)
        };
        group("Work session", None, body, pal).into_any_element()
    }

    fn work_invite(d: &Data, pal: Palette, cx: &mut Context<'_, Self>) -> Div {
        let start = button("session-start", "Start a work session", pal, false, d.paired);
        card(pal).child(
            row(pal, true)
                .flex_wrap()
                .py(px(14.0))
                .child(
                    caption(
                        "Approve once on your phone and your AI tools work without asking for a while. Force pushes, \
                         deleting, the vault and purchases still ask.",
                        pal,
                    )
                    .flex_1()
                    .min_w(px(260.0)),
                )
                .child(if d.paired {
                    start.on_click(Self::on_model(cx, Model::open_work_form))
                } else {
                    start
                }),
        )
    }

    fn work_form(&mut self, form: &Form, pal: Palette, window: &Window, cx: &mut Context<'_, Self>) -> Div {
        while self.branch_fields.len() < form.repos.len() {
            self.branch_fields.push(cx.focus_handle());
        }

        let mut lengths = segmented(pal);
        for length in Length::ALL {
            lengths = lengths.child(
                segment(
                    SharedString::from(format!("session-{}", length.id())),
                    length.label(),
                    form.length == length,
                    pal,
                )
                .on_click(Self::on_model(cx, move |m, cx| m.set_work_length(length, cx))),
            );
        }

        let reason = self.text_field(Field::SessionReason, work::DEFAULT_REASON, pal, window, cx).font_family(FONT);

        let mut reads = div().flex().flex_wrap().gap(px(8.0));
        for (id, label) in READS {
            reads = reads.child(
                read_chip(id, label, form.read.contains(&id), pal)
                    .on_click(Self::on_model(cx, move |m, cx| m.toggle_work_read(id, cx))),
            );
        }

        let push: AnyElement =
            if form.repos.is_empty() {
                div()
                    .pt(px(6.0))
                    .child(caption(
                        "No repositories yet: git has not reached one through Reins lately. `reins allow 2h` in a \
                     repository adds its branch.",
                        pal,
                    ))
                    .into_any_element()
            } else {
                let mut rows = div().flex().flex_col().gap(px(8.0));
                for (i, repo) in form.repos.iter().enumerate() {
                    let Ok(slot) = u8::try_from(i) else {
                        break;
                    };
                    let refused = match form.push_row(i) {
                        PushRow::Refused(why) => Some(why),
                        PushRow::Empty | PushRow::Push(_) => None,
                    };
                    let line = div()
                        .flex()
                        .items_center()
                        .gap(px(10.0))
                        .child(
                            div()
                                .flex_none()
                                .w(px(210.0))
                                .truncate()
                                .font_family(MONO)
                                .text_size(px(12.0))
                                .child(repo.label()),
                        )
                        .child(
                            self.text_field(Field::SessionBranch(slot), "branch to push to", pal, window, cx)
                                .when(refused.is_some(), |f| f.border_color(pal.danger.opacity(0.6))),
                        );
                    rows =
                        rows.child(
                            div().flex().flex_col().gap(px(4.0)).child(line).when_some(refused, |c, why| {
                                c.child(fine(why, pal).pl(px(220.0)).text_color(pal.danger))
                            }),
                        );
                }
                rows.child(fine("Only rows with a branch are included.", pal)).into_any_element()
            };

        let can_ask = form.request().is_some();
        let ask = button("session-ask", "Ask my phone", pal, true, can_ask);
        card(pal)
            .child(form_line("How long", lengths, pal, true))
            .child(form_line("What it's for", div().flex().child(reason), pal, false))
            .child(form_line(
                "Read",
                div()
                    .flex()
                    .flex_col()
                    .gap(px(6.0))
                    .child(reads)
                    .child(fine("Integrations not connected on your phone are left out.", pal)),
                pal,
                false,
            ))
            .child(form_line("Push with git", push, pal, false))
            .child(
                row(pal, false)
                    .py(px(12.0))
                    .bg(pal.background.opacity(0.5))
                    .child(fine(STILL_ASKS, pal).flex_1().min_w(px(0.0)))
                    .child(
                        button("session-cancel", "Cancel", pal, false, true)
                            .on_click(Self::on_model(cx, Model::close_work_form)),
                    )
                    .child(if can_ask {
                        ask.on_click(Self::on_model(cx, Model::start_work_session))
                    } else {
                        ask
                    }),
            )
    }

    fn work_waiting(asking: &Asking, pal: Palette) -> Div {
        let left = i64::try_from(asking.left(std::time::Instant::now())).unwrap_or(0);
        card(pal).border_color(pal.accent.opacity(0.35)).child(
            row(pal, true)
                .py(px(14.0))
                .child(badge(waiting_dot("session-waiting", pal.accent, 9.0), pal))
                .child(titled(
                    "Waiting for your phone… approve the work session there",
                    format!("{} · {}", asking.request.reason, work::summary(&asking.request)),
                    pal,
                ))
                .child(
                    div()
                        .flex_none()
                        .text_size(px(12.0))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(pal.accent)
                        .child(format!("{} left", format::duration(left))),
                ),
        )
    }

    fn work_failed(why: &str, form: &Form, pal: Palette, cx: &mut Context<'_, Self>) -> Div {
        let summary = form.request().map(|r| work::summary(&r)).unwrap_or_default();
        card(pal).border_color(pal.danger.opacity(0.35)).child(
            row(pal, true)
                .flex_wrap()
                .py(px(14.0))
                .child(level_mark(Level::Fail, 30.0, pal))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_w(px(240.0))
                        .gap(px(1.0))
                        .child(
                            div()
                                .text_size(px(14.0))
                                .font_weight(FontWeight::SEMIBOLD)
                                .child("The work session did not start"),
                        )
                        .child(caption(why.to_owned(), pal).text_color(pal.danger))
                        .when(!summary.is_empty(), |c| c.child(fine(summary, pal))),
                )
                .child(
                    div()
                        .flex()
                        .flex_none()
                        .gap(px(8.0))
                        .child(
                            button("session-change", "Change", pal, false, true)
                                .on_click(Self::on_model(cx, Model::edit_work_request)),
                        )
                        .child(
                            button("session-retry", "Try again", pal, true, true)
                                .on_click(Self::on_model(cx, Model::start_work_session)),
                        ),
                ),
        )
    }

    fn work_running(session: &Session, d: &Data, pal: Palette, cx: &mut Context<'_, Self>) -> Div {
        let head = row(pal, true)
            .py(px(14.0))
            .child(badge(div().size(px(10.0)).rounded_full().bg(pal.accent), pal))
            .child(titled(
                session.reason.clone(),
                format!("Until {} · started {}", format::clock(session.expires_at), format::clock(session.started_at)),
                pal,
            ))
            .child(
                div()
                    .flex_none()
                    .text_size(px(15.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(pal.accent)
                    .child(format!("{} left", work::left(session, d.now))),
            );
        let mut body = card(pal).border_color(pal.accent.opacity(0.35)).child(head);
        let mut allows = div().flex().flex_col().gap(px(6.0)).child(fine("Allowed without asking", pal));
        for line in &session.allows {
            allows = allows.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .child(check(13.0, pal.success))
                    .child(div().min_w(px(0.0)).truncate().child(line.clone())),
            );
        }
        if !session.skipped.is_empty() {
            allows = allows
                .child(fine(format!("Left out (not connected on your phone): {}", session.skipped.join(", ")), pal));
        }
        body = body.child(row(pal, false).items_start().py(px(12.0)).pl(px(58.0)).child(allows));
        if let Some(e) = &d.work.end_error {
            body = body.child(
                row(pal, false)
                    .pl(px(58.0))
                    .child(caption(format!("Could not end it: {e}"), pal).text_color(pal.danger)),
            );
        }
        let footer = row(pal, false).py(px(12.0)).bg(pal.background.opacity(0.5));
        let footer = if d.work.ending {
            footer.child(caption("Ending the session on your phone…", pal).flex_1()).child(button(
                "session-ending",
                "Ending…",
                pal,
                false,
                false,
            ))
        } else if d.work.confirm_end {
            footer
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.0))
                        .font_weight(FontWeight::MEDIUM)
                        .child("End the session? Its permissions end on your phone now."),
                )
                .child(
                    button("session-keep", "Keep", pal, false, true)
                        .on_click(Self::on_model(cx, Model::keep_work_session)),
                )
                .child(
                    button("session-end", "End", pal, true, true)
                        .bg(pal.danger)
                        .on_click(Self::on_model(cx, Model::end_work_session)),
                )
        } else {
            footer.child(fine(STILL_ASKS, pal).flex_1().min_w(px(0.0))).child(
                button("session-end-now", "End now", pal, false, true)
                    .on_click(Self::on_model(cx, Model::confirm_end_work_session)),
            )
        };
        body.child(footer)
    }
}
