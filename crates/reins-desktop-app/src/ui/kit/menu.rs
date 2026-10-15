//! Menus that open from their trigger and replace system pickers: `paper-raised`, 20 px corners, a hairline and a
//! soft shadow; rows of a tile, a title and a line; the current one on `paper-sunken`. They open over [`MENU_OPEN`]
//! with a fade and a 4 px slide from the trigger (GPUI cannot scale a div, so they do not grow from 0.92).
//!
//! [`MENU_OPEN`]: crate::theme::motion::MENU_OPEN

use gpui::prelude::FluentBuilder as _;
use gpui::{
    Anchor, AnyElement, BoxShadow, Div, ElementId, FontWeight, Hsla, InteractiveElement as _, IntoElement,
    ParentElement as _, SharedString, Stateful, StatefulInteractiveElement as _, Styled as _, anchored, deferred, div,
    point, px,
};

use super::button::PRESSED;
use crate::theme::motion::{Entrance, enter};
use crate::theme::{Palette, SANS};
use crate::ui::icons::{Glyph, glyph};

/// The menu's frame (290 px wide unless the caller says otherwise).
pub fn menu(pal: Palette) -> Div {
    div()
        .occlude()
        .flex()
        .flex_col()
        .w(px(290.0))
        .p(px(6.0))
        .rounded(px(20.0))
        .bg(pal.paper_raised())
        .border_1()
        .border_color(pal.rule())
        .shadow(vec![BoxShadow {
            color: gpui::black().opacity(if pal.dark {
                0.5
            } else {
                0.2
            }),
            offset: point(px(0.0), px(24.0)),
            blur_radius: px(48.0),
            spread_radius: px(-16.0),
            inset: false,
        }])
}

/// A menu row: a 32 px tile (`tile` fill, `ink` glyph), a title and a line; `current` on `paper-sunken` with a check;
/// a destructive one in red.
#[allow(clippy::too_many_arguments, reason = "a row's parts")]
pub fn menu_item(
    id: impl Into<ElementId>,
    g: Glyph,
    colors: (Hsla, Hsla),
    title: impl Into<SharedString>,
    line: Option<SharedString>,
    current: bool,
    danger: bool,
    pal: Palette,
) -> Stateful<Div> {
    div()
        .id(id.into())
        .flex()
        .items_center()
        .gap(px(12.0))
        .min_h(px(52.0))
        .px(px(12.0))
        .py(px(6.0))
        .rounded(px(14.0))
        .cursor_pointer()
        .when_else(current, |r| r.bg(pal.paper_sunken()), |r| r.hover(|h| h.bg(pal.paper_sunken().opacity(0.6))))
        .active(|s| s.opacity(PRESSED))
        .child(
            div()
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .size(px(32.0))
                .rounded(px(10.0))
                .bg(colors.0)
                .child(glyph(g, 17.0, colors.1)),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w(px(0.0))
                .font_family(SANS)
                .child(
                    div()
                        .text_size(px(15.0))
                        .line_height(px(19.0))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(if danger {
                            pal.danger
                        } else {
                            pal.ink()
                        })
                        .child(title.into()),
                )
                .when_some(line, |c, l| {
                    c.child(div().text_size(px(12.5)).line_height(px(16.0)).text_color(pal.ink_meta()).child(l))
                }),
        )
        .when(current, |r| r.child(glyph(Glyph::Check, 16.0, pal.violet())))
        .when(danger && !current, |r| r.child(glyph(Glyph::ChevronRight, 14.0, pal.ink_faint())))
}

/// A hairline between groups of rows.
pub fn menu_rule(pal: Palette) -> Div {
    div().h(px(1.0)).mx(px(10.0)).my(px(4.0)).bg(pal.rule())
}

/// `content` opened from its trigger: anchored under it (at `corner` of the trigger, pushed in from the window's
/// edges), drawn above the rest, entering with a fade and a short slide. The caller puts this in the (relative)
/// trigger.
pub fn open_menu(id: impl Into<ElementId>, content: Div, corner: Anchor) -> AnyElement {
    let top_right = matches!(corner, Anchor::TopRight | Anchor::BottomRight);
    div()
        .absolute()
        .top_full()
        .when_else(top_right, gpui::Styled::right_0, gpui::Styled::left_0)
        .size_0()
        .child(
            deferred(
                anchored()
                    .anchor(corner)
                    .offset(point(px(0.0), px(6.0)))
                    .snap_to_window_with_margin(px(12.0))
                    .child(enter(content.relative(), id, Entrance::Menu)),
            )
            .with_priority(1),
        )
        .into_any_element()
}
