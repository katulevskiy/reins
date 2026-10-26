//! Buttons (HANDOFF §8, `Components.png`): pills in six looks. Pressed content dims to 55%, disabled to 40%; focus is
//! a 2 px ring at a 3 px offset, violet on paper and white on a field.

use gpui::prelude::FluentBuilder as _;
use gpui::{
    Div, ElementId, Hsla, InteractiveElement as _, IntoElement, ParentElement as _, SharedString, Stateful,
    StatefulInteractiveElement as _, Styled as _, div, px,
};

use crate::theme::{Palette, SANS};
use crate::ui::icons::{Glyph, glyph};

/// Pressed content's opacity.
pub const PRESSED: f32 = 0.55;
/// Disabled content's opacity.
pub const DISABLED: f32 = 0.4;

/// What a button looks like.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Look {
    /// `inverse`: the action ("Approve", "Send").
    Inverse,
    /// A 1.5 px `rule-2` outline: "Deny".
    Outline,
    /// `paper-sunken`: "This was wrong", "Start".
    Soft,
    /// Red on `danger-wash`: "End now".
    DangerWash,
    /// `violet-fill`: "Allow" by Autopilot.
    Violet,
    /// White with ink, on a field: "Review".
    OnField,
    /// `on-field-wash`, on a field: "Deny" there.
    Wash,
}

impl Look {
    pub const ALL: [Self; 7] =
        [Self::Inverse, Self::Outline, Self::Soft, Self::DangerWash, Self::Violet, Self::OnField, Self::Wash];

    /// Its fill and what goes on it.
    #[must_use]
    pub fn colors(self, pal: Palette) -> (Hsla, Hsla) {
        match self {
            Self::Inverse => (pal.inverse(), pal.on_inverse()),
            Self::Outline => (gpui::transparent_black(), pal.ink()),
            Self::Soft => (pal.paper_sunken(), pal.ink()),
            Self::DangerWash => (pal.danger_wash(), pal.danger),
            Self::Violet => (pal.violet_fill(), pal.on_field()),
            Self::OnField => (gpui::white(), ink_light()),
            Self::Wash => (pal.on_field_wash(), pal.on_field()),
        }
    }

    /// Sits on a field (its focus ring is white).
    #[must_use]
    pub fn on_field(self) -> bool {
        matches!(self, Self::OnField | Self::Wash)
    }
}

/// Light `ink`: the text of a white button on a field is always dark.
fn ink_light() -> Hsla {
    crate::theme::token(crate::theme::tokens::INK, false)
}

/// How big.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Size {
    /// The phone's decision buttons: 54 px, 16.5.
    Large,
    /// The desktop's: 42 px, 15 (`.btn.sm` on the Next desktop boards).
    Regular,
    /// In rows and plates: 30 px, 13 (`.btn.xs`).
    Small,
}

impl Size {
    /// Height, text size, side padding, glyph size.
    #[must_use]
    pub fn metrics(self) -> (f32, f32, f32, f32) {
        match self {
            Self::Large => (54.0, 16.5, 22.0, 18.0),
            Self::Regular => (42.0, 15.0, 16.0, 18.0),
            Self::Small => (30.0, 13.0, 12.0, 14.0),
        }
    }
}

/// A button: a pill of `look`, `size`, with `label` (and a glyph before it). It dims to 55% while pressed.
pub fn button(
    id: impl Into<ElementId>,
    look: Look,
    size: Size,
    lead: Option<Glyph>,
    label: impl Into<SharedString>,
    pal: Palette,
) -> Stateful<Div> {
    let (bg, fg) = look.colors(pal);
    let (h, text, pad, g) = size.metrics();
    div()
        .id(id.into())
        .relative()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .gap(px(7.0))
        .h(px(h))
        .px(px(pad))
        .rounded_full()
        .bg(bg)
        .when(look == Look::Outline, |b| b.border(px(1.5)).border_color(pal.rule_2()))
        .text_color(fg)
        .font_family(SANS)
        .text_size(px(text))
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .whitespace_nowrap()
        .cursor_pointer()
        .active(|s| s.opacity(PRESSED))
        .when_some(lead, |b, l| b.child(glyph(l, g, fg)))
        .child(label.into())
}

/// A round icon button (`.ib`): 40 px (or `side`) on `paper-raised` with a hairline, a glyph in ink. It carries no
/// word, so it needs a label for screen readers where it is used.
pub fn icon_button(id: impl Into<ElementId>, g: Glyph, side: f32, pal: Palette) -> Stateful<Div> {
    div()
        .id(id.into())
        .relative()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(px(side))
        .rounded_full()
        .bg(pal.paper_raised())
        .border_1()
        .border_color(pal.rule())
        .cursor_pointer()
        .active(|s| s.opacity(PRESSED))
        .child(glyph(g, side * 0.45, pal.ink()))
}

/// `b` disabled: 40%, no pointer. (Leave its click handler off.)
pub fn disabled(b: Stateful<Div>) -> Stateful<Div> {
    b.opacity(DISABLED).cursor_default()
}

/// The focus ring around an element of corner radius `radius`: 2 px, 3 px out from its edge; violet on paper, white
/// on a field. A child of the (relative) element; it takes no room.
pub fn focus_ring(radius: f32, on_field: bool, pal: Palette) -> impl IntoElement {
    let out = 5.0;
    div()
        .absolute()
        .top(px(-out))
        .left(px(-out))
        .right(px(-out))
        .bottom(px(-out))
        .rounded(px(radius + out))
        .border_2()
        .border_color(if on_field {
            pal.on_field()
        } else {
            pal.violet()
        })
}

/// `b` (a pill `height` px high) showing focus.
pub fn focused(b: Stateful<Div>, height: f32, on_field: bool, pal: Palette) -> Stateful<Div> {
    b.child(focus_ring(height / 2.0, on_field, pal))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn looks_have_their_colours() {
        for pal in [Palette::light(), Palette::dark()] {
            assert_eq!(Look::Inverse.colors(pal), (pal.inverse(), pal.on_inverse()));
            assert_eq!(Look::DangerWash.colors(pal).1, pal.danger);
            // On a field the white button keeps dark ink in both themes.
            assert_eq!(Look::OnField.colors(pal).1, ink_light());
        }
        assert!((PRESSED - 0.55).abs() < f32::EPSILON && (DISABLED - 0.4).abs() < f32::EPSILON);
    }
}
