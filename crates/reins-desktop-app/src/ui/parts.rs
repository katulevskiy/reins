//! Building blocks: cards, rows, buttons, toggles, chips, badges, headers. Calm, like the phone apps: Geist, hairline
//! cards, one violet accent.

use std::sync::{Arc, LazyLock};
use std::time::Duration;

use gpui::prelude::FluentBuilder as _;
use gpui::{
    Animation, AnimationExt as _, Div, ElementId, FontWeight, Hsla, Image, ImageFormat, InteractiveElement as _,
    IntoElement, ParentElement as _, SharedString, Stateful, Styled as _, div, img, pulsating_between, px,
};
use reins_desktop::journal::Outcome;

use crate::model::Step;
use crate::theme::{MONO, Palette};
use crate::tray::Look;

static MARK: LazyLock<Arc<Image>> = LazyLock::new(|| {
    Arc::new(Image::from_bytes(ImageFormat::Png, include_bytes!("../../assets/icons/app-256.png").to_vec()))
});

pub fn mark(side: f32) -> impl IntoElement {
    img(MARK.clone()).size(px(side)).flex_none()
}

pub fn card(pal: Palette) -> Div {
    div().flex().flex_col().bg(pal.elevated).border_1().border_color(pal.hairline).rounded(px(12.0)).overflow_hidden()
}

pub fn caption(text: impl Into<SharedString>, pal: Palette) -> Div {
    div().text_size(px(12.0)).line_height(px(17.0)).text_color(pal.secondary).child(text.into())
}

/// A small line under a caption: tertiary, 11.5 px.
pub fn fine(text: impl Into<SharedString>, pal: Palette) -> Div {
    div().text_size(px(11.5)).line_height(px(16.0)).text_color(pal.tertiary).child(text.into())
}

/// One line that ends in "…" when it does not fit.
pub fn one_line(text: impl Into<SharedString>) -> Div {
    div().min_w(px(0.0)).truncate().child(text.into())
}

pub fn mono(text: impl Into<SharedString>, pal: Palette) -> Div {
    div().font_family(MONO).text_size(px(12.0)).line_height(px(17.0)).text_color(pal.text).child(text.into())
}

/// The title of a section of the status window, with what it is for.
pub fn page_header(title: &'static str, explain: impl Into<SharedString>, pal: Palette) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(px(4.0))
        .child(div().text_size(px(22.0)).line_height(px(28.0)).font_weight(FontWeight::SEMIBOLD).child(title))
        .child(div().text_size(px(13.0)).line_height(px(19.0)).text_color(pal.secondary).child(explain.into()))
}

/// A titled block of a page: its title, an optional line about it, then `body` (usually a card).
pub fn group(title: impl Into<SharedString>, about: Option<&str>, body: impl IntoElement, pal: Palette) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(px(8.0))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(2.0))
                .px(px(2.0))
                .child(div().text_size(px(13.5)).font_weight(FontWeight::SEMIBOLD).child(title.into()))
                .when_some(about.map(str::to_owned), |d, a| d.child(caption(a, pal))),
        )
        .child(body)
}

pub fn button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    pal: Palette,
    primary: bool,
    enabled: bool,
) -> Stateful<Div> {
    let (bg, fg) = if primary {
        (pal.accent, pal.on_accent)
    } else {
        (pal.control_fill, pal.text)
    };
    let b = div()
        .id(id.into())
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .h(px(32.0))
        .px(px(14.0))
        .rounded(px(9.0))
        .bg(bg)
        .text_color(fg)
        .text_size(px(13.0))
        .font_weight(FontWeight::MEDIUM)
        .whitespace_nowrap()
        .child(label.into());
    if enabled {
        b.cursor_pointer().hover(|s| s.opacity(0.85))
    } else {
        b.opacity(0.45)
    }
}

pub fn link(id: impl Into<ElementId>, label: impl Into<SharedString>, pal: Palette) -> Stateful<Div> {
    div()
        .id(id.into())
        .text_size(px(12.0))
        .text_color(pal.accent)
        .whitespace_nowrap()
        .cursor_pointer()
        .hover(|s| s.opacity(0.75))
        .child(label.into())
}

pub fn row(pal: Palette, first: bool) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(12.0))
        .px(px(16.0))
        .py(px(11.0))
        .min_w(px(0.0))
        .when(!first, |r| r.border_t_1().border_color(pal.hairline))
}

/// A label with a line under it, taking the room a row has.
pub fn labelled(label: impl Into<SharedString>, detail: Option<String>, pal: Palette) -> Div {
    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_w(px(0.0))
        .gap(px(1.0))
        .child(div().font_weight(FontWeight::MEDIUM).child(label.into()))
        .when_some(detail.filter(|d| !d.is_empty()), |d, text| d.child(caption(text, pal)))
}

pub fn checkbox(checked: bool, enabled: bool, pal: Palette) -> Div {
    let b = div().flex().flex_none().items_center().justify_center().size(px(18.0)).rounded(px(5.0));
    let b = if checked {
        b.bg(pal.accent).text_color(pal.on_accent).text_size(px(12.0)).child("✓")
    } else {
        b.border_1().border_color(pal.tertiary)
    };
    if enabled {
        b
    } else {
        b.opacity(0.4)
    }
}

pub fn toggle(on: bool, pal: Palette) -> Div {
    let knob = div().size(px(16.0)).rounded_full().bg(gpui::white()).shadow_sm();
    div()
        .flex()
        .flex_none()
        .items_center()
        .w(px(34.0))
        .h(px(20.0))
        .p(px(2.0))
        .rounded_full()
        .bg(if on {
            pal.accent
        } else {
            pal.control_fill
        })
        .when_else(on, Div::justify_end, Div::justify_start)
        .child(knob)
}

pub fn look_color(look: Look, pal: Palette) -> Hsla {
    match look {
        Look::On => pal.success,
        Look::Paused => pal.warning,
        Look::Attention => pal.danger,
    }
}

pub fn pill(look: Look, pal: Palette) -> Div {
    let color = look_color(look, pal);
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap(px(6.0))
        .px(px(9.0))
        .py(px(3.0))
        .rounded_full()
        .bg(color.opacity(0.12))
        .text_color(color)
        .text_size(px(11.5))
        .font_weight(FontWeight::MEDIUM)
        .child(div().size(px(6.0)).rounded_full().bg(color))
        .child(look.word())
}

/// A dot that breathes: something waits.
pub fn waiting_dot(id: impl Into<ElementId>, color: Hsla, side: f32) -> impl IntoElement {
    div().flex_none().size(px(side)).rounded_full().bg(color).with_animation(
        id,
        Animation::new(Duration::from_millis(1600)).repeat().with_easing(pulsating_between(0.25, 1.0)),
        gpui::Styled::opacity,
    )
}

/// The colour an outcome is shown in.
pub fn outcome_color(outcome: Outcome, pal: Palette) -> Hsla {
    match outcome {
        Outcome::Waiting => pal.accent,
        Outcome::Approved => pal.success,
        Outcome::Denied => pal.danger,
        Outcome::TimedOut => pal.warning,
        Outcome::Failed => pal.danger.opacity(0.8),
        Outcome::Allowed | Outcome::Blocked => pal.secondary,
    }
}

/// "Approved", "Denied", … in its colour; a waiting one breathes.
pub fn outcome_badge(id: impl Into<ElementId>, outcome: Outcome, pal: Palette) -> Div {
    let color = outcome_color(outcome, pal);
    let failed = outcome == Outcome::Failed;
    let badge = div()
        .flex()
        .flex_none()
        .items_center()
        .gap(px(5.0))
        .w(px(86.0))
        .h(px(22.0))
        .px(px(8.0))
        .rounded_full()
        .text_color(color)
        .text_size(px(11.5))
        .font_weight(FontWeight::MEDIUM)
        .when_else(failed, |b| b.border_1().border_color(color.opacity(0.45)), |b| b.bg(color.opacity(0.12)));
    let badge = if outcome == Outcome::Waiting {
        badge.child(waiting_dot(id, color, 6.0))
    } else {
        badge.child(div().flex_none().size(px(6.0)).rounded_full().bg(color))
    };
    badge.child(outcome.label())
}

/// A filter chip: its label, how many, selected or not.
pub fn chip(
    id: impl Into<ElementId>,
    label: &str,
    count: Option<usize>,
    selected: bool,
    pal: Palette,
) -> Stateful<Div> {
    div()
        .id(id.into())
        .flex()
        .flex_none()
        .items_center()
        .gap(px(6.0))
        .h(px(26.0))
        .px(px(10.0))
        .rounded_full()
        .text_size(px(12.0))
        .font_weight(FontWeight::MEDIUM)
        .cursor_pointer()
        .border_1()
        .when_else(
            selected,
            |c| c.bg(pal.accent_soft).border_color(pal.accent.opacity(0.35)).text_color(pal.accent),
            |c| {
                c.border_color(pal.hairline)
                    .bg(pal.elevated)
                    .text_color(pal.secondary)
                    .hover(|s| s.bg(pal.control_fill))
            },
        )
        .child(label.to_owned())
        .when_some(count, |c, n| {
            c.child(
                div()
                    .text_size(px(11.0))
                    .opacity(if n == 0 {
                        0.5
                    } else {
                        0.8
                    })
                    .child(n.to_string()),
            )
        })
}

/// The frame of a segmented control.
pub fn segmented(pal: Palette) -> Div {
    div().flex().flex_none().items_center().p(px(2.0)).gap(px(2.0)).rounded(px(9.0)).bg(pal.control_fill)
}

/// One choice of a segmented control.
pub fn segment(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    selected: bool,
    pal: Palette,
) -> Stateful<Div> {
    div()
        .id(id.into())
        .flex()
        .items_center()
        .justify_center()
        .h(px(26.0))
        .px(px(11.0))
        .rounded(px(7.0))
        .text_size(px(12.0))
        .font_weight(FontWeight::MEDIUM)
        .whitespace_nowrap()
        .cursor_pointer()
        .when_else(
            selected,
            |s| s.bg(pal.elevated).text_color(pal.text).shadow_xs(),
            |s| s.text_color(pal.secondary).hover(|h| h.text_color(pal.text)),
        )
        .child(label.into())
}

/// A number and what it counts.
pub fn stat_tile(value: usize, label: &'static str, color: Hsla, pal: Palette) -> Div {
    card(pal)
        .flex_1()
        .min_w(px(0.0))
        .px(px(16.0))
        .py(px(14.0))
        .gap(px(2.0))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(7.0))
                .child(div().size(px(7.0)).rounded_full().bg(color))
                .child(div().text_size(px(12.0)).text_color(pal.secondary).child(label)),
        )
        .child(
            div()
                .text_size(px(24.0))
                .line_height(px(30.0))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(if value == 0 {
                    pal.tertiary
                } else {
                    pal.text
                })
                .child(value.to_string()),
        )
}

pub fn info_row(label: &str, value: impl Into<SharedString>, pal: Palette, first: bool) -> Div {
    row(pal, first)
        .justify_between()
        .child(div().flex_none().text_color(pal.secondary).child(label.to_owned()))
        .child(div().min_w(px(0.0)).text_color(pal.text).font_weight(FontWeight::MEDIUM).truncate().child(value.into()))
}

/// A note that something needs doing first.
pub fn warning(text: impl Into<SharedString>, pal: Palette) -> Div {
    card(pal)
        .w_full()
        .p(px(12.0))
        .bg(pal.warning.opacity(0.1))
        .border_color(pal.warning.opacity(0.3))
        .child(caption(text, pal).text_color(pal.text))
}

/// Lines of code to copy, in a well.
pub fn code_block(lines: &[String], pal: Palette) -> Div {
    let mut block = div()
        .flex()
        .flex_col()
        .gap(px(4.0))
        .px(px(14.0))
        .py(px(12.0))
        .rounded(px(9.0))
        .bg(pal.control_fill)
        .overflow_hidden();
    for line in lines {
        block = block.child(mono(line.clone(), pal).whitespace_nowrap());
    }
    block
}

/// What a card says while it has nothing to show.
pub fn empty(text: impl Into<SharedString>, pal: Palette) -> Div {
    div().px(px(16.0)).py(px(18.0)).child(caption(text, pal).text_color(pal.tertiary))
}

/// A small mono tag (`vault:OpenAI/api key`).
pub fn tag(text: impl Into<SharedString>, pal: Palette) -> Div {
    div()
        .flex_none()
        .px(px(7.0))
        .py(px(1.0))
        .rounded(px(5.0))
        .bg(pal.control_fill)
        .font_family(MONO)
        .text_size(px(11.5))
        .line_height(px(17.0))
        .text_color(pal.secondary)
        .whitespace_nowrap()
        .child(text.into())
}

pub fn step_line(label: &str, step: &Step, pal: Palette, first: bool) -> Div {
    let (icon, color, detail) = match step {
        Step::Running => ("…", pal.accent, String::new()),
        Step::Done(d) => ("✓", pal.success, d.clone()),
        Step::Failed(e) => ("!", pal.danger, e.clone()),
    };
    row(pal, first)
        .items_start()
        .child(div().w(px(14.0)).text_color(color).font_weight(FontWeight::SEMIBOLD).child(icon))
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w(px(0.0))
                .gap(px(2.0))
                .child(div().text_size(px(13.0)).child(label.to_owned()))
                .when(!detail.is_empty(), |d| d.child(caption(detail, pal))),
        )
}

/// The QR code that opens Activity in the phone app.
pub fn phone_qr(pal: Palette) -> Div {
    card(pal)
        .p(px(16.0))
        .flex_row()
        .items_center()
        .gap(px(16.0))
        .child(div().flex_none().p(px(4.0)).rounded(px(10.0)).bg(gpui::white()).child(crate::qr::element(
            crate::links::PHONE_ACTIVITY,
            96.0,
            gpui::black(),
            gpui::white(),
        )))
        .child(
            caption(
                "Point your phone's camera at this code to open Activity in Reins: everything your agents asked for, \
                 on every computer, and what you answered.",
                pal,
            )
            .flex_1()
            .min_w(px(0.0)),
        )
}
