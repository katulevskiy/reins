//! Next's panes (`.grp` in `DeskNext-System.dc.html`): rows grouped on `paper-raised` with 20 px corners, a hairline
//! ring on light paper, hairlines between the rows that start after the row's tile. Over a wallpaper (Home, Activity)
//! a pane is the "readable frosted" glass (`.grp.pf2`): the wallpaper blurred under a thin fill, a lit top edge, the
//! hairlines short of both edges. Reduce Transparency, or Frosted panes off in Settings, turns the glass solid; the
//! wallpaper stays.
//!
//! The blur is GPUI's backdrop blur (the zui fork's `paint_backdrop_blur`: Metal, DirectX, wgpu), painted first in
//! the pane's own layer so the pane's contents always land on top of it. Saturate and brightness have no GPUI
//! equivalent: the fill is a touch denser instead.

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, App, BoxShadow, Bounds, Corners, Div, Element, ElementId, GlobalElementId, Hsla,
    InspectorElementId, InteractiveElement as _, IntoElement, LayoutId, ParentElement as _, Pixels, SharedString,
    Stateful, StatefulInteractiveElement as _, Styled as _, Window, div, px,
};

use super::button::PRESSED;
use crate::theme::{NextText as _, Palette, rgba};
use crate::ui::icons::{Glyph, Mark, glyph};

/// A pane's corners.
pub const RADIUS: f32 = 20.0;
/// The blur under glass (`.pf2`: 40 px).
pub const BLUR: f32 = 40.0;
/// Where a row's hairline starts: after a 34 px tile (14 + 34 + 12 + 2).
pub const INSET: f32 = 62.0;

/// What a pane is made of.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Glass {
    /// `paper-raised`: the panes on paper, and every pane under Reduce Transparency.
    #[default]
    Solid,
    /// Frosted glass over the wallpaper.
    Frosted,
}

impl Glass {
    /// Frosted where `over_art` and transparency is allowed (the OS setting and Settings' Frosted panes).
    #[must_use]
    pub fn over(over_art: bool, panes_frosted: bool) -> Self {
        if over_art && panes_frosted && !crate::theme::motion::reduce_transparency() {
            Self::Frosted
        } else {
            Self::Solid
        }
    }
}

/// A pane: `content` (rows, or anything) on raised paper or glass, its corners `radius`.
pub fn pane(content: impl IntoElement, glass: Glass, radius: f32, pal: Palette) -> AnyElement {
    let base = div().relative().flex().flex_col().rounded(px(radius)).overflow_hidden();
    match glass {
        Glass::Solid => base
            .bg(pal.paper_raised())
            .when(!pal.dark, |p| p.border_1().border_color(pal.rule()))
            .child(content)
            .into_any_element(),
        Glass::Frosted => {
            let (fill, edge, ring, drop) = if pal.dark {
                (rgba(0x0018_181D, 0.56), rgba(0x00FF_FFFF, 0.16), rgba(0x00FF_FFFF, 0.08), rgba(0, 0.3))
            } else {
                (rgba(0x00FF_FFFF, 0.46), rgba(0x00FF_FFFF, 0.8), rgba(0x00FF_FFFF, 0.4), rgba(0, 0.06))
            };
            frosted(
                radius,
                BLUR,
                base.bg(fill)
                    .shadow(vec![
                        BoxShadow::new(px(0.0), px(1.0), edge).inset(),
                        BoxShadow::new(px(0.0), px(0.0), ring).spread_radius(px(1.0)).inset(),
                        BoxShadow::new(px(0.0), px(8.0), drop).blur_radius(px(30.0)),
                    ])
                    .child(content),
            )
            .into_any_element()
        }
    }
}

/// A pane of rows (`.grp`), with the hairlines between them starting at `inset` (62 after a tile, 14 without one).
pub fn group(rows: impl IntoIterator<Item = AnyElement>, inset: f32, glass: Glass, pal: Palette) -> AnyElement {
    pane(div().flex().flex_col().children(with_rules(rows, inset, glass, pal)), glass, RADIUS, pal)
}

/// `rows` with a hairline between each two.
pub fn with_rules(
    rows: impl IntoIterator<Item = AnyElement>,
    inset: f32,
    glass: Glass,
    pal: Palette,
) -> Vec<AnyElement> {
    let mut out = Vec::new();
    for (i, r) in rows.into_iter().enumerate() {
        if i > 0 {
            out.push(rule(inset, glass, pal).into_any_element());
        }
        out.push(r);
    }
    out
}

/// The hairline between two rows of a pane: `rule` on paper; on glass a softer one that stops 16 px short.
pub fn rule(inset: f32, glass: Glass, pal: Palette) -> Div {
    let line = div().flex_none().h(px(1.0)).ml(px(inset));
    match glass {
        Glass::Solid => line.bg(pal.rule()),
        Glass::Frosted => line.mr(px(16.0)).bg(if pal.dark {
            rgba(0x00FF_FFFF, 0.09)
        } else {
            rgba(0, 0.08)
        }),
    }
}

/// A row of a pane (`.win .r`): 50 px at least, 8 × 14 inside, 12 between its parts.
pub fn pane_row() -> Div {
    div().relative().flex().items_center().gap(px(12.0)).min_h(px(50.0)).px(px(14.0)).py(px(8.0)).min_w(px(0.0))
}

/// A row's title (15, 500) and the line under it (13, `ink-meta`).
pub fn title_sub(title: impl IntoElement, sub: Option<AnyElement>, pal: Palette) -> Div {
    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_w(px(0.0))
        .child(div().row().text_color(pal.ink()).min_w(px(0.0)).child(title))
        .when_some(sub, |c, s| c.child(div().meta().mt(px(1.0)).text_color(pal.ink_meta()).min_w(px(0.0)).child(s)))
}

/// A section line (`.sec`): a glyph, its words (13, 600, `ink-2`), and on the right whatever `end` is (a count, a +).
pub fn section(lead: Option<Glyph>, text: impl Into<SharedString>, end: Option<AnyElement>, pal: Palette) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(6.0))
        .px(px(6.0))
        .pt(px(22.0))
        .pb(px(8.0))
        .label()
        .text_color(pal.ink_2())
        .when_some(lead, |s, g| s.child(glyph(g, 15.0, pal.ink_2())))
        .child(div().min_w(px(0.0)).truncate().child(text.into()))
        .when_some(end, |s, e| s.child(div().ml_auto().flex().items_center().gap(px(10.0)).child(e)))
}

/// A count on a section line, and its + (`.ib`, 28 px) when there is one.
pub fn section_count(count: Option<usize>, plus: Option<Stateful<Div>>, pal: Palette) -> AnyElement {
    div()
        .flex()
        .items_center()
        .gap(px(4.0))
        .when_some(count, |c, n| c.child(div().text_color(pal.ink_2()).child(n.to_string())))
        .when_some(plus, |c, p| c.child(p.ml(px(4.0))))
        .into_any_element()
}

/// The + of a section (28 px round, a hairline ring).
pub fn plus(id: impl Into<ElementId>, pal: Palette) -> Stateful<Div> {
    super::icon_button(id, Glyph::Plus, 28.0, pal)
}

/// A neutral tile (`.tile2`): 34 px, 10 px corners, `paper-sunken`, the glyph in `ink-2`.
pub fn tile(g: Glyph, pal: Palette) -> Div {
    div()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(px(34.0))
        .rounded(px(10.0))
        .bg(pal.paper_sunken())
        .child(glyph(g, 17.0, pal.ink_2()))
}

/// A brand's or a tool's mark on a round plate (`.avp`): white (`#1d1d21` on dark paper, where one-colour logos
/// turn white), a hairline ring, the mark at 56%.
pub fn avatar(mark: Mark, side: f32, pal: Palette) -> Div {
    let plate = div()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(px(side))
        .rounded_full()
        .overflow_hidden()
        .bg(if pal.dark {
            rgba(0x001D_1D21, 1.0)
        } else {
            gpui::white()
        })
        .border_1()
        .border_color(pal.rule());
    match mark {
        Mark::Brand(b) => plate.child(crate::ui::icons::brand_on(b, side * 0.56, pal)),
        Mark::Glyph(g) => plate.child(glyph(g, side * 0.5, pal.ink_2())),
    }
}

/// The chevron at a row's end (14, `ink-faint`).
pub fn chevron(pal: Palette) -> impl IntoElement {
    glyph(Glyph::ChevronRight, 14.0, pal.ink_faint())
}

/// A value at a row's end (`.rv`: 15, `ink-2`).
pub fn row_value(text: impl Into<SharedString>, pal: Palette) -> Div {
    div().flex_none().text_size(px(15.0)).text_color(pal.ink_2()).whitespace_nowrap().child(text.into())
}

/// A clickable row: a hover wash and the pressed dim.
pub fn clickable(row: Div, id: impl Into<ElementId>, pal: Palette) -> Stateful<Div> {
    row.id(id.into()).cursor_pointer().hover(|s| s.bg(pal.ink().opacity(0.035))).active(|s| s.opacity(PRESSED))
}

/// A quiet last row that does something serious (`Disconnect`, `Delete key`, `Forget this`): a tile of `color` at
/// 13%, the words in `color`, a line under them.
pub fn quiet_row(g: Glyph, title: impl Into<SharedString>, sub: impl Into<SharedString>, color: Hsla, pal: Palette) -> Div {
    pane_row()
        .child(
            div()
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .size(px(34.0))
                .rounded(px(10.0))
                .bg(color.opacity(0.13))
                .child(glyph(g, 17.0, color)),
        )
        .child(title_sub(
            div().text_color(color).child(title.into()),
            Some(div().child(sub.into()).into_any_element()),
            pal,
        ))
}

/// Paints `child` frosted: its own scene layer, the backdrop blurred inside `radius` corners first, then the child
/// on top (one draw order, so nothing of the pane is ever blurred away).
pub fn frosted(radius: f32, blur: f32, child: impl IntoElement) -> Frosted {
    Frosted {
        radius,
        blur,
        child: child.into_any_element(),
    }
}

pub struct Frosted {
    radius: f32,
    blur: f32,
    child: AnyElement,
}

impl IntoElement for Frosted {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

impl Element for Frosted {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        (self.child.request_layout(window, cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _layout: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        self.child.prepaint(window, cx);
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _layout: &mut (),
        _prepaint: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        window.paint_layer(bounds, |window| {
            window.paint_backdrop_blur(bounds, Corners::all(px(self.radius)), px(self.blur));
            self.child.paint(window, cx);
        });
    }
}
