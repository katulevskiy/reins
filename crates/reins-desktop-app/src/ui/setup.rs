//! The first-time setup: add Reins to the AI tools found here, start the background service, send git through it.

use gpui::{
    AnyElement, Context, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, div, px,
};

use super::Root;
use super::parts::{button, caption, card, checkbox, mark, row, step_line, warning};
use crate::model::{Model, Step};
use crate::theme::Palette;

impl Root {
    pub(super) fn setup(&self, pal: Palette, cx: &mut Context<'_, Self>) -> AnyElement {
        let m = self.model.read(cx);
        let items = m.setup_items.clone();
        let steps = m.setup_steps.clone();
        let running = m.setup_running;
        let finished = m.setup_finished();
        let failed = steps.iter().any(|(_, s)| matches!(s, Step::Failed(_)));
        let notice = m.notice.clone();
        let location = crate::backend::Backend::location_problem();

        let mut list = card(pal);
        for (i, item) in items.iter().enumerate() {
            let (checked, available) = (item.checked, item.available);
            let mut r = row(pal, i == 0).id(("setup-item", i)).child(checkbox(checked, available, pal)).child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w(px(0.0))
                    .gap(px(1.0))
                    .child(div().font_weight(FontWeight::MEDIUM).child(item.label()))
                    .child(caption(item.detail(), pal).text_size(px(11.5))),
            );
            if available && !running && !finished {
                r = r.cursor_pointer().on_click(Self::on_model(cx, move |m, cx| m.toggle_item(i, cx)));
            } else if !available {
                r = r.opacity(0.6);
            }
            list = list.child(r);
        }

        let mut page = div()
            .flex()
            .flex_col()
            .gap(px(14.0))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.0))
                    .child(mark(30.0))
                    .child(div().text_size(px(15.0)).font_weight(FontWeight::SEMIBOLD).child("Almost done")),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(4.0))
                    .child(div().text_size(px(19.0)).line_height(px(25.0)).font_weight(FontWeight::SEMIBOLD).child("Add Reins to your AI tools"))
                    .child(caption(
                        "Reins found these on this computer and adds itself to each one. Untick anything you want to leave alone.",
                        pal,
                    )),
            );
        if steps.is_empty() {
            page = page.child(list);
        } else {
            let mut progress = card(pal);
            for (i, (label, step)) in steps.iter().enumerate() {
                progress = progress.child(step_line(label, step, pal, i == 0));
            }
            page = page.child(progress);
        }
        let actions = if finished && !failed {
            div()
                .flex()
                .flex_col()
                .gap(px(8.0))
                .child(caption("All set. Restart your AI tools so they pick up Reins.", pal))
                .child(button("done", "Done", pal, true, true).on_click(Self::on_model(cx, Model::finish_setup)))
        } else if finished {
            div()
                .flex()
                .gap(px(8.0))
                .child(
                    button("again", "Try again", pal, true, true)
                        .flex_1()
                        .on_click(Self::on_model(cx, Model::run_setup)),
                )
                .child(
                    button("skip", "Continue anyway", pal, false, true)
                        .on_click(Self::on_model(cx, Model::finish_setup)),
                )
        } else {
            let label = if running {
                "Setting up…"
            } else {
                "Set up Reins"
            };
            let b = button("run-setup", label, pal, true, !running && location.is_none());
            div().flex().flex_col().child(if running || location.is_some() {
                b
            } else {
                b.on_click(Self::on_model(cx, Model::run_setup))
            })
        };
        if let Some(problem) = location {
            page = page.child(warning(problem, pal));
        }
        page = page.child(actions);
        if let Some(n) = notice {
            page = page.child(caption(n, pal).text_color(pal.danger));
        }
        page.into_any_element()
    }
}
