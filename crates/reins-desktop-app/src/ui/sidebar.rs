//! The status window's sidebar: the mark and the state at the top, the sections, and pausing at the bottom.

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, Context, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _, SharedString,
    StatefulInteractiveElement as _, Styled as _, div, px,
};

use super::parts::{button, caption, fine, mark, pill, waiting_dot};
use super::{Data, Root};
use crate::model::{Model, Section};
use crate::pause::PauseFor;
use crate::shortcuts;
use crate::theme::Palette;

const WIDTH: f32 = 228.0;

impl Root {
    pub(super) fn sidebar(&self, d: &Data, pal: Palette, cx: &mut Context<'_, Self>) -> AnyElement {
        let waiting = d.waiting().count();
        let mut nav = div().flex().flex_col().gap(px(2.0)).px(px(10.0));
        for (i, section) in Section::ALL.into_iter().enumerate() {
            let selected = d.section == section;
            let badge = (section == Section::Activity && waiting > 0).then_some(waiting);
            let group_name = SharedString::from(format!("nav-{}", section.id()));
            nav = nav.child(
                div()
                    .id(group_name.clone())
                    .group(group_name.clone())
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .h(px(32.0))
                    .px(px(10.0))
                    .rounded(px(8.0))
                    .cursor_pointer()
                    .text_size(px(13.0))
                    .when_else(
                        selected,
                        |r| {
                            r.bg(pal.selected)
                                .border_1()
                                .border_color(pal.hairline)
                                .shadow_xs()
                                .text_color(pal.text)
                                .font_weight(FontWeight::MEDIUM)
                        },
                        |r| r.text_color(pal.secondary).hover(|s| s.bg(pal.control_fill).text_color(pal.text)),
                    )
                    .child(div().flex_1().child(section.label()))
                    // Its shortcut, while the pointer is on it.
                    .when(badge.is_none(), |r| {
                        r.child(
                            div()
                                .text_size(px(11.0))
                                .text_color(pal.tertiary.opacity(0.0))
                                .group_hover(group_name.clone(), |s| s.text_color(pal.tertiary))
                                .child(shortcuts::hint(&(i + 1).to_string())),
                        )
                    })
                    .when_some(badge, |r, n| {
                        r.child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(5.0))
                                .px(px(7.0))
                                .h(px(18.0))
                                .rounded_full()
                                .bg(pal.accent_soft)
                                .text_color(pal.accent)
                                .text_size(px(11.0))
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(waiting_dot("nav-waiting", pal.accent, 5.0))
                                .child(n.to_string()),
                        )
                    })
                    .on_click(Self::go(cx, section)),
            );
        }

        div()
            .flex()
            .flex_col()
            .flex_none()
            .w(px(WIDTH))
            .h_full()
            .bg(pal.sidebar)
            .border_r_1()
            .border_color(pal.hairline)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.0))
                    .px(px(18.0))
                    .pt(px(if cfg!(target_os = "macos") {
                        44.0
                    } else {
                        20.0
                    }))
                    .pb(px(18.0))
                    .child(mark(26.0))
                    .child(div().flex_1().text_size(px(15.0)).font_weight(FontWeight::SEMIBOLD).child("Reins"))
                    .child(pill(d.look, pal)),
            )
            .child(nav)
            .child(div().flex_1())
            .child(self.pause_box(d, pal, cx))
            .child(fine(format!("Reins {}", reins_desktop::update::VERSION), pal).px(px(20.0)).pb(px(14.0)).pt(px(8.0)))
            .into_any_element()
    }

    /// The bottom of the sidebar: pause (with its lengths), or the pause and Resume.
    fn pause_box(&self, d: &Data, pal: Palette, cx: &mut Context<'_, Self>) -> AnyElement {
        let frame = div()
            .mx(px(10.0))
            .p(px(12.0))
            .flex()
            .flex_col()
            .gap(px(8.0))
            .rounded(px(10.0))
            .bg(pal.elevated)
            .border_1()
            .border_color(pal.hairline);
        if let Some(left) = d.pause.short(d.now) {
            return frame
                .border_color(pal.warning.opacity(0.35))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(7.0))
                        .child(div().size(px(7.0)).rounded_full().bg(pal.warning))
                        .child(div().font_weight(FontWeight::MEDIUM).child("git is paused"))
                        .child(div().flex_1())
                        .child(fine(left, pal)),
                )
                .child(caption("Hooks and MCP tools still ask your phone.", pal))
                .child(
                    button("side-resume", "Resume", pal, true, true)
                        .w_full()
                        .on_click(Self::on_model(cx, Model::resume)),
                )
                .into_any_element();
        }
        let open = self.pause_menu && d.paired;
        let mut b = frame;
        if open {
            b = b.child(fine("Send git straight to the hosts for", pal));
            let mut lengths = div().flex().flex_col().gap(px(1.0));
            for length in PauseFor::ALL {
                lengths = lengths.child(
                    div()
                        .id(SharedString::from(format!("side-{}", length.id())))
                        .flex()
                        .items_center()
                        .h(px(28.0))
                        .px(px(8.0))
                        .rounded(px(6.0))
                        .cursor_pointer()
                        .hover(|s| s.bg(pal.control_fill))
                        .child(length.label())
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.pause_menu = false;
                            this.model.update(cx, |m, cx| m.pause_for(length, cx));
                        })),
                );
            }
            b = b.child(lengths);
        }
        let label = if open {
            "Cancel"
        } else {
            "Pause git…"
        };
        b.child(
            button("side-pause", label, pal, false, d.paired)
                .w_full()
                .when(d.paired, |btn| btn.on_click(Self::on_view(cx, |this, _| this.pause_menu = !this.pause_menu))),
        )
        .into_any_element()
    }
}
