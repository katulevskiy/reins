//! The smaller pieces (HANDOFF §8, `Components.png`, the desktop boards' CSS): plates, hairline rows, switches,
//! segmented controls, pills, tags, the mode chip, kind tiles, the timer, keys, section labels and the empty state.

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, Div, ElementId, FontWeight, Hsla, InteractiveElement as _, IntoElement, ParentElement as _,
    SharedString, Stateful, StatefulInteractiveElement as _, Styled as _, div, px,
};

use super::button::PRESSED;
use crate::theme::text::tracked;
use crate::theme::{Field, MONO, Palette, SANS, NextText as _, Tone, motion};
use crate::ui::icons::{Glyph, Illustration, Mark, glyph, illustration};
use crate::ui::parts::ring;

/// A raised plate on paper: `paper-raised`, a 1 px `rule`, 20 px corners, 18 × 20 inside.
pub fn plate(pal: Palette) -> Div {
    div()
        .flex()
        .flex_col()
        .px(px(20.0))
        .py(px(18.0))
        .rounded(px(20.0))
        .bg(pal.paper_raised())
        .border_1()
        .border_color(pal.rule())
}

/// A section label: mono capitals, tracked, `ink-meta` ("LATEST").
pub fn section_label(text: impl Into<SharedString>, pal: Palette) -> Div {
    tracked(text).text_color(pal.ink_meta())
}

/// A hairline row: 50 px at least, a `rule` under it (`last` has none). Rows divide lists; they are never cards.
pub fn hairline_row(pal: Palette, last: bool) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(12.0))
        .min_h(px(50.0))
        .py(px(6.0))
        .min_w(px(0.0))
        .text_size(px(14.0))
        .when(!last, |r| r.border_b_1().border_color(pal.rule()))
}

/// A row's title and the meta line under it.
pub fn row_lines(title: impl Into<SharedString>, meta: Option<SharedString>, pal: Palette) -> Div {
    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_w(px(0.0))
        .child(
            div()
                .min_w(px(0.0))
                .truncate()
                .font_family(SANS)
                .text_size(px(14.5))
                .line_height(px(19.0))
                .font_weight(FontWeight::MEDIUM)
                .text_color(pal.ink())
                .child(title.into()),
        )
        .when_some(meta, |c, m| {
            c.child(
                div()
                    .min_w(px(0.0))
                    .truncate()
                    .text_size(px(12.5))
                    .line_height(px(17.0))
                    .text_color(pal.ink_meta())
                    .child(m),
            )
        })
}

/// A fact as a row: the key on the left in `ink-2`, the value on the right in `ink`.
pub fn fact_row(key: impl Into<SharedString>, value: impl IntoElement, pal: Palette, last: bool) -> Div {
    hairline_row(pal, last).child(div().flex_1().text_color(pal.ink_2()).child(key.into())).child(
        div()
            .flex()
            .items_center()
            .gap(px(8.0))
            .justify_end()
            .text_color(pal.ink())
            .font_weight(FontWeight::MEDIUM)
            .child(value),
    )
}

/// A switch (40 × 24, the desktop boards' `.sw`): `violet-fill` when on, `paper-sunken` when off, a white knob.
pub fn switch(id: impl Into<ElementId>, on: bool, pal: Palette) -> Stateful<Div> {
    let (w, h, knob) = (40.0, 24.0, 20.0);
    div()
        .id(id.into())
        .flex()
        .flex_none()
        .items_center()
        .w(px(w))
        .h(px(h))
        .px(px((h - knob) / 2.0))
        .rounded_full()
        .bg(if on {
            pal.violet_fill()
        } else {
            pal.paper_sunken()
        })
        .when(!on && pal.dark, |s| s.border_1().border_color(pal.rule_2()))
        .when_else(on, gpui::Styled::justify_end, gpui::Styled::justify_start)
        .cursor_pointer()
        .active(|s| s.opacity(PRESSED))
        .child(div().size(px(knob)).rounded_full().bg(gpui::white()).shadow_sm())
}

/// A segmented control's track: a `paper-sunken` pill (3 px inside, the desktop boards' `.seg`).
pub fn segmented(pal: Palette) -> Div {
    div().flex().flex_none().items_center().p(px(3.0)).gap(px(2.0)).rounded_full().bg(pal.paper_sunken())
}

/// One choice of it (30 px, 13): the selected one a `paper-raised` pill with a hairline.
pub fn segment(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    selected: bool,
    pal: Palette,
) -> Stateful<Div> {
    div()
        .id(id.into())
        .flex()
        .flex_1()
        .items_center()
        .justify_center()
        .gap(px(4.0))
        .h(px(30.0))
        .px(px(14.0))
        .rounded_full()
        .font_family(SANS)
        .text_size(px(13.0))
        .font_weight(FontWeight::MEDIUM)
        .whitespace_nowrap()
        .cursor_pointer()
        .active(|s| s.opacity(PRESSED))
        .when_else(
            selected,
            |s| s.bg(pal.paper_raised()).border_1().border_color(pal.rule()).text_color(pal.ink()).shadow_xs(),
            |s| s.text_color(pal.ink_2()).hover(|h| h.text_color(pal.ink())),
        )
        .child(label.into())
}

/// A pill: always a word, sometimes a glyph before it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PillLook {
    /// `paper-raised` with a hairline: a filter, "Manual".
    Plain,
    /// `inverse`: the selected filter.
    Selected,
    /// `violet-fill`: "Auto".
    Violet,
    /// The amber field: "Lockdown".
    Amber,
    /// The red field: "Bypass", with its breathing dot.
    Red,
    /// `paper-sunken`, `ink-2`: what the desktop boards call a chip ("git through Reins").
    Soft,
}

/// A pill (28 px), `look`, with `label` and maybe a glyph.
pub fn pill(lead: Option<Glyph>, label: impl Into<SharedString>, look: PillLook, pal: Palette) -> Div {
    let fg = pill_colors(look, pal).1;
    pill_frame(look, pal).when_some(lead, |p, g| p.child(glyph(g, 14.0, fg))).child(label.into())
}

fn pill_colors(look: PillLook, pal: Palette) -> (Hsla, Hsla) {
    match look {
        PillLook::Plain => (pal.paper_raised(), pal.ink()),
        PillLook::Selected => (pal.inverse(), pal.on_inverse()),
        PillLook::Violet => (pal.violet_fill(), pal.on_field()),
        PillLook::Amber => (Field::Warn.field(), pal.on_field()),
        PillLook::Red => (Field::Delete.field(), pal.on_field()),
        PillLook::Soft => (pal.paper_sunken(), pal.ink_2()),
    }
}

/// A pill without its contents.
fn pill_frame(look: PillLook, pal: Palette) -> Div {
    let (bg, fg) = pill_colors(look, pal);
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap(px(6.0))
        .h(px(28.0))
        .px(px(11.0))
        .rounded_full()
        .bg(bg)
        .when(look == PillLook::Plain, |p| p.border_1().border_color(pal.rule()))
        .text_color(fg)
        .font_family(SANS)
        .text_size(px(12.5))
        .font_weight(FontWeight::MEDIUM)
        .whitespace_nowrap()
}

/// The mode chip in every header.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Manual,
    Auto,
    Lockdown,
    /// Bypass, with the time left ("14:05").
    Bypass(&'static str),
}

/// The mode chip: Manual plain, Auto violet, Lockdown amber, Bypass red with a dot that breathes.
pub fn mode_chip(id: &'static str, mode: Mode, pal: Palette) -> Div {
    match mode {
        Mode::Manual => pill(Some(Glyph::Person), "Manual", PillLook::Plain, pal),
        Mode::Auto => pill(Some(Glyph::Autopilot), "Auto", PillLook::Violet, pal),
        Mode::Lockdown => pill(Some(Glyph::Lock), "Lockdown", PillLook::Amber, pal),
        Mode::Bypass(left) => pill_frame(PillLook::Red, pal)
            .child(motion::breathe(div().size(px(8.0)).rounded_full().bg(pal.on_field()), id))
            .child(div().font_family(MONO).child(left))
            .child("Bypass"),
    }
}

/// A small tag (11.5, 600): its colour at 13% under it ("New", "Auto").
pub fn tag(label: impl Into<SharedString>, color: Hsla) -> Div {
    div()
        .flex_none()
        .px(px(8.0))
        .py(px(3.0))
        .rounded_full()
        .bg(color.opacity(0.13))
        .text_color(color)
        .tag()
        .whitespace_nowrap()
        .child(label.into())
}

/// A kind tile: the kind's glyph in its ink over the ink at 13% (32 px, 10 px corners on the desktop; 38/11 on the
/// phone); a neutral one is `paper-sunken` under `ink`.
pub fn kind_tile(g: Glyph, tone: Tone, side: f32, pal: Palette) -> Div {
    let ink = match tone {
        Tone::Neutral => pal.ink(),
        other => pal.tone(other),
    };
    div()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(px(side))
        .rounded(px(side * 0.31))
        .bg(pal.tone_tile(tone))
        .child(glyph(g, side * 0.53, ink))
}

/// A request's timer: a ring that drains and the time left in mono; under 15 s, a white pill with red digits. `on`
/// is the colour it sits on (white on a field, ink on paper).
pub fn timer(left_secs: i64, total_secs: i64, on: Hsla) -> Div {
    let left = left_secs.max(0);
    let clock = format!("{}:{:02}", left / 60, left % 60);
    if left < 15 {
        return div()
            .flex()
            .flex_none()
            .items_center()
            .h(px(26.0))
            .px(px(10.0))
            .rounded_full()
            .bg(gpui::white())
            .text_color(Field::Delete.field())
            .font_family(MONO)
            .text_size(px(13.0))
            .font_weight(FontWeight::MEDIUM)
            .child(clock);
    }
    #[allow(clippy::cast_precision_loss, reason = "seconds")]
    let fraction = left as f32 / total_secs.max(1) as f32;
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap(px(6.0))
        .text_color(on)
        .font_family(MONO)
        .text_size(px(13.0))
        .child(ring(fraction, on, on.opacity(0.25), 14.0, 1.8))
        .child(clock)
}

/// A service's or an AI tool's mark on a 32 px white tile with a hairline (the boards' `.mk`), the logo at 58%; a
/// glyph instead sits on `paper-sunken` in `ink`.
pub fn logo_tile(m: Mark, pal: Palette) -> Div {
    let tile = div().flex().flex_none().items_center().justify_center().size(px(32.0)).rounded(px(10.0));
    match m {
        Mark::Brand(b) => tile
            .bg(gpui::white())
            .border_1()
            .border_color(pal.rule())
            .child(gpui::img(b.path()).size(gpui::relative(0.58)).flex_none()),
        Mark::Glyph(i) => tile.bg(pal.paper_sunken()).child(glyph(i, 17.0, pal.ink())),
    }
}

/// A request's icon (`ASK-TEXT.md`): the integration's mark (a logo on its white plate, or a glyph on `paper-sunken`
/// for what happens on this computer), and the action on a round badge at its bottom-right corner, white on the kind's
/// field colour, ringed in `surface` (what the icon sits on) so it stands apart from the mark.
pub fn request_icon(mark: Mark, badge: Glyph, field: Field, side: f32, surface: Hsla, pal: Palette) -> Div {
    let plate = div().flex().flex_none().items_center().justify_center().size(px(side)).rounded(px(side * 0.31));
    let plate = match mark {
        Mark::Brand(b) => plate
            .bg(gpui::white())
            .border_1()
            .border_color(pal.rule())
            .child(gpui::img(b.path()).size(gpui::relative(0.58)).flex_none()),
        Mark::Glyph(g) => plate.bg(pal.paper_sunken()).child(glyph(g, side * 0.53, pal.ink())),
    };
    let dot = (side * 0.5).round();
    let ring = if side >= 36.0 {
        2.0
    } else {
        1.5
    };
    div().relative().flex_none().size(px(side)).child(plate).child(
        div()
            .absolute()
            .right(px(-dot * 0.22))
            .bottom(px(-dot * 0.22))
            .flex()
            .items_center()
            .justify_center()
            .size(px(dot))
            .rounded_full()
            .bg(field.field())
            .border(px(ring))
            .border_color(surface)
            .child(glyph(badge, dot * 0.56, gpui::white())),
    )
}

/// A key cap in a list of shortcuts ("⌘R", "Esc"): mono 12 on `paper-sunken`, `ink-2`.
pub fn key_cap(text: impl Into<SharedString>, pal: Palette) -> Div {
    div()
        .flex_none()
        .px(px(8.0))
        .py(px(3.0))
        .rounded(px(7.0))
        .bg(pal.paper_sunken())
        .font_family(MONO)
        .text_size(px(12.0))
        .line_height(px(16.0))
        .text_color(pal.ink_2())
        .whitespace_nowrap()
        .child(text.into())
}

/// A state as a small pill (11.5, 600): `color` on itself at 13% ("Held until 22:34", "Installed"), or `ink-2` on
/// `paper-sunken` when `color` is `None` ("Asks next time"); a glyph before the word when given.
pub fn status_pill(lead: Option<Glyph>, label: impl Into<SharedString>, color: Option<Hsla>, pal: Palette) -> Div {
    let (bg, fg) = color.map_or((pal.paper_sunken(), pal.ink_2()), |c| (c.opacity(0.13), c));
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap(px(4.0))
        .px(px(9.0))
        .py(px(4.0))
        .rounded_full()
        .bg(bg)
        .text_color(fg)
        .tag()
        .whitespace_nowrap()
        .when_some(lead, |p, g| p.child(glyph(g, 12.0, fg)))
        .child(label.into())
}

/// A key on the keyboard ("⌘1", "Ctrl+1"): mono, `ink-meta`.
pub fn kbd(text: impl Into<SharedString>, pal: Palette) -> Div {
    div()
        .flex_none()
        .font_family(MONO)
        .text_size(px(11.0))
        .text_color(pal.ink_faint())
        .whitespace_nowrap()
        .child(text.into())
}

/// The empty state: the pond, and one quiet line in the serif's italic ("All quiet on the water.").
pub fn empty_state(mood: impl Into<SharedString>, width: f32, pal: Palette) -> Div {
    div()
        .flex()
        .flex_col()
        .items_center()
        .gap(px(14.0))
        .py(px(16.0))
        .child(illustration(Illustration::Pond, width, pal))
        .child(div().mood().text_color(pal.ink_2()).text_center().child(mood.into()))
}

/// A row of a field's label line ("GITHUB · PUSH") in `on-field-2`.
pub fn field_label(text: impl Into<SharedString>, pal: Palette) -> Div {
    tracked(text).text_color(pal.on_field_2())
}

/// A serif line with the words after `plain` in italic: "Push 8 commits to *octo/app*".
pub fn ask_line(plain: &str, target: &str) -> AnyElement {
    let text = format!("{plain}{target}");
    let start = plain.len();
    gpui::StyledText::new(text)
        .with_highlights([(
            start..start + target.len(),
            gpui::HighlightStyle {
                font_style: Some(gpui::FontStyle::Italic),
                ..gpui::HighlightStyle::default()
            },
        )])
        .into_any_element()
}
