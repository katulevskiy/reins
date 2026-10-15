//! Home's work session card (`Desk-Extras.png`): the form (how long, what for, what to read, which branches to push
//! to), then the request waiting on the phone, or why it did not start. A running session is Home's Work session
//! plate turned into a violet field ([`super::home`]): a session is a bundle of grants.

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, Context, Div, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _, SharedString,
    Stateful, StatefulInteractiveElement as _, Styled as _, Window, div, px,
};

use super::field::Field;
use super::icons::{Glyph, glyph};
use super::parts::{caption, help, waiting_dot};
use super::{Data, Root, kit};
use crate::format;
use crate::model::Model;
use crate::theme::{MONO, Palette, SANS};
use crate::work::{self, Asking, Form, Length, PushRow, READS};

/// What a work session leaves out, whatever it allows.
const STILL_ASKS: &str = "Force push, delete, vault and purchases still ask.";
/// The width of the form's labels.
const LABEL: f32 = 110.0;

/// A repository as the form names it: without its host ("github.com/acme/web" → "acme/web").
#[must_use]
pub fn repo_name(label: &str) -> String {
    match label.split_once('/') {
        Some((host, rest)) if host.contains('.') && !rest.is_empty() => rest.to_owned(),
        _ => label.to_owned(),
    }
}

/// A form line: its label on the left, `body` beside it.
fn form_line(label: &'static str, body: impl IntoElement, pal: Palette) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(16.0))
        .child(div().flex_none().w(px(LABEL)).text_size(px(14.0)).text_color(pal.ink_2()).child(label))
        .child(div().flex().flex_col().flex_1().min_w(px(0.0)).gap(px(8.0)).child(body))
}

/// An integration to read: on (inverse, with a check) or off.
fn read_chip(id: &'static str, label: &'static str, on: bool, pal: Palette) -> Stateful<Div> {
    div()
        .id(SharedString::from(format!("session-read-{id}")))
        .flex()
        .flex_none()
        .items_center()
        .gap(px(6.0))
        .h(px(30.0))
        .px(px(12.0))
        .rounded_full()
        .text_size(px(13.0))
        .font_weight(FontWeight::MEDIUM)
        .cursor_pointer()
        .active(|s| s.opacity(kit::button::PRESSED))
        .when_else(
            on,
            |c| c.bg(pal.inverse()).text_color(pal.on_inverse()).child(glyph(Glyph::Check, 13.0, pal.on_inverse())),
            |c| c.bg(pal.paper_sunken()).text_color(pal.ink_2()).hover(|s| s.text_color(pal.ink())),
        )
        .child(label)
}

impl Root {
    /// The card: the request waiting on the phone, why it did not start, or the form.
    pub(super) fn work_card(
        &mut self,
        d: &Data,
        pal: Palette,
        window: &Window,
        cx: &mut Context<'_, Self>,
    ) -> AnyElement {
        let body = if let Some(asking) = &d.work.asking {
            Self::work_waiting(asking, pal)
        } else if let (Some(why), Some(form)) = (&d.work.failed, &d.work.form) {
            Self::work_failed(why, form, pal, cx)
        } else if let Some(form) = &d.work.form {
            self.work_form(form, d, pal, window, cx)
        } else {
            return div().into_any_element();
        };
        let about = "Your phone approves a whole session once: for a while, your agents push to the branches you name, \
                     read what you pick and call those tools without asking each time.";
        div()
            .flex()
            .flex_col()
            .gap(px(8.0))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .child(kit::section_label("Work session · ask once, work freely", pal))
                    .child(help("session", about, d, pal, cx)),
            )
            .child(body)
            .into_any_element()
    }

    fn work_form(&mut self, form: &Form, d: &Data, pal: Palette, window: &Window, cx: &mut Context<'_, Self>) -> Div {
        while self.branch_fields.len() < form.repos.len() {
            self.branch_fields.push(cx.focus_handle());
        }

        let mut lengths = kit::segmented(pal).w_full();
        for length in Length::ALL {
            lengths = lengths.child(
                kit::segment(
                    SharedString::from(format!("session-{}", length.id())),
                    length.label(),
                    form.length == length,
                    pal,
                )
                .h(px(32.0))
                .px(px(10.0))
                .on_click(Self::on_model(cx, move |m, cx| m.set_work_length(length, cx))),
            );
        }

        let reason = self
            .text_field(Field::SessionReason, work::DEFAULT_REASON, pal, window, cx)
            .bg(pal.paper_raised())
            .font_family(SANS)
            .text_size(px(14.0));

        let mut reads = div().flex().flex_wrap().gap(px(6.0));
        for (id, label) in READS {
            reads = reads.child(
                read_chip(id, label, form.read.contains(&id), pal)
                    .on_click(Self::on_model(cx, move |m, cx| m.toggle_work_read(id, cx))),
            );
        }

        let push: AnyElement = if form.repos.is_empty() {
            div()
                .flex()
                .items_center()
                .gap(px(6.0))
                .child(caption("No repositories yet", pal).text_color(pal.ink_meta()))
                .child(help(
                    "session-repos",
                    "git has not reached one through Reins lately. `reins allow 2h` in a repository adds its branch.",
                    d,
                    pal,
                    cx,
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
                    .gap(px(8.0))
                    .child(
                        div()
                            .flex_none()
                            .w(px(LABEL))
                            .truncate()
                            .font_family(MONO)
                            .text_size(px(13.0))
                            .text_color(pal.ink())
                            .child(repo_name(&repo.label())),
                    )
                    .child(
                        self.text_field(Field::SessionBranch(slot), "branch to push to", pal, window, cx)
                            .bg(pal.paper_raised())
                            .text_size(px(13.0))
                            .when(refused.is_some(), |f| f.border_color(pal.danger.opacity(0.6))),
                    );
                rows = rows.child(div().flex().flex_col().gap(px(4.0)).child(line).when_some(refused, |c, why| {
                    c.child(div().pl(px(LABEL + 8.0)).text_size(px(12.5)).text_color(pal.danger).child(why))
                }));
            }
            rows.into_any_element()
        };

        let can_ask = form.request().is_some();
        let ask =
            kit::button("session-ask", kit::Look::Inverse, kit::Size::Regular, Some(Glyph::Phone), "Ask my phone", pal);
        kit::plate(pal)
            .p(px(24.0))
            .gap(px(14.0))
            .child(form_line("How long", lengths, pal))
            .child(form_line("What for", div().flex().child(reason), pal))
            .child(form_line("Read", reads, pal))
            .child(form_line("Push to", push, pal))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap(px(12.0))
                    .pt(px(14.0))
                    .border_t_1()
                    .border_color(pal.rule())
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(200.0))
                            .text_size(px(12.5))
                            .text_color(pal.ink_meta())
                            .child(STILL_ASKS),
                    )
                    .child(
                        kit::button("session-cancel", kit::Look::Soft, kit::Size::Regular, None, "Cancel", pal)
                            .on_click(Self::on_model(cx, Model::close_work_form)),
                    )
                    .child(if can_ask {
                        ask.on_click(Self::on_model(cx, Model::start_work_session))
                    } else {
                        kit::disabled(ask)
                    }),
            )
    }

    fn work_waiting(asking: &Asking, pal: Palette) -> Div {
        let left = i64::try_from(asking.left(std::time::Instant::now())).unwrap_or(0);
        kit::plate(pal)
            .flex_row()
            .items_center()
            .gap(px(12.0))
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .size(px(32.0))
                    .rounded(px(10.0))
                    .bg(pal.violet().opacity(0.12))
                    .child(waiting_dot("session-waiting", pal.violet(), 9.0)),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w(px(0.0))
                    .child(
                        div()
                            .text_size(px(14.5))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(pal.ink())
                            .child("Approve it on your phone"),
                    )
                    .child(div().truncate().text_size(px(12.5)).text_color(pal.ink_meta()).child(format!(
                        "{} · {}",
                        asking.request.reason,
                        work::summary(&asking.request)
                    ))),
            )
            .child(
                div()
                    .flex_none()
                    .font_family(MONO)
                    .text_size(px(13.0))
                    .text_color(pal.violet())
                    .child(format!("{} left", format::duration(left))),
            )
    }

    fn work_failed(why: &str, form: &Form, pal: Palette, cx: &mut Context<'_, Self>) -> Div {
        let summary = form.request().map(|r| work::summary(&r)).unwrap_or_default();
        kit::plate(pal)
            .flex_row()
            .flex_wrap()
            .items_center()
            .gap(px(12.0))
            .child(kit::kind_tile(
                Glyph::CircleClose,
                crate::theme::Tone::Field(crate::theme::Field::Delete),
                32.0,
                pal,
            ))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w(px(240.0))
                    .child(
                        div()
                            .text_size(px(14.5))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(pal.ink())
                            .child("Not started"),
                    )
                    .child(div().text_size(px(12.5)).text_color(pal.danger).child(why.to_owned()))
                    .when(!summary.is_empty(), |c| {
                        c.child(div().text_size(px(12.5)).text_color(pal.ink_meta()).child(summary))
                    }),
            )
            .child(
                div()
                    .flex()
                    .flex_none()
                    .gap(px(8.0))
                    .child(
                        kit::button("session-change", kit::Look::Soft, kit::Size::Regular, None, "Change", pal)
                            .on_click(Self::on_model(cx, Model::edit_work_request)),
                    )
                    .child(
                        kit::button("session-retry", kit::Look::Inverse, kit::Size::Regular, None, "Try again", pal)
                            .on_click(Self::on_model(cx, Model::start_work_session)),
                    ),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repositories_are_named_without_their_host() {
        assert_eq!(repo_name("github.com/acme/web"), "acme/web");
        assert_eq!(repo_name("acme/web"), "acme/web");
        assert_eq!(repo_name("local"), "local");
    }
}
