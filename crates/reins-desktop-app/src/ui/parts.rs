//! Building blocks the screens use today, restyled in Reins Signal (phase 1: the same layout, Signal's colours,
//! type, glyphs and press states; phase 2 moves the screens onto [`super::kit`]): plates with hairlines, rows with
//! kind tiles, switches, pills, segmented controls, the (?) and its bubble, rings that count down, and the "stage"
//! (who → what) of a request. Icons are the Signal glyphs, logos the brands' own marks ([`super::icons`]).

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, Context, Div, ElementId, FontStyle, FontWeight, HighlightStyle, Hsla, InteractiveElement as _,
    IntoElement, ParentElement, PathBuilder, SharedString, Stateful, StatefulInteractiveElement as _, Styled as _,
    StyledText, anchored, canvas, deferred, div, point, px,
};

use super::icons::{Glyph, Mark, avatar, glyph};
use super::story::Words;
use super::{Data, Root};
use crate::theme::text::tracked;
use crate::theme::{MONO, Palette, SERIF, SignalText as _, Tone, motion};
use crate::welcome::Stage;

/// "this Mac", "this computer": what the window calls the computer it runs on.
#[must_use]
pub fn this_computer() -> &'static str {
    if cfg!(target_os = "macos") {
        "this Mac"
    } else {
        "this computer"
    }
}

/// A check in a soft circle of `color`: something went well.
pub fn check_circle(side: f32, color: Hsla) -> Div {
    div()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(px(side))
        .rounded_full()
        .bg(color.opacity(0.14))
        .child(glyph(Glyph::Check, side * 0.5, color))
}

/// A kind tile: an icon in `color` over `color` at 13% (a kind, a rule, a setting).
pub fn tile(i: Glyph, color: Hsla, side: f32) -> Div {
    div()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(px(side))
        .rounded(px(side * 0.31))
        .bg(color.opacity(0.13))
        .child(glyph(i, side * 0.53, color))
}

/// A kind tile of `tone` (a neutral one is `paper-sunken` under `ink`).
pub fn tone_tile(i: Glyph, tone: Tone, pal: Palette, side: f32) -> Div {
    super::kit::kind_tile(i, tone, side, pal)
}

/// A logo (an AI tool, a git host, an API): a brand's mark in a white circle, an icon in a neutral tile.
pub fn logo(m: Mark, pal: Palette) -> Div {
    match m {
        Mark::Brand(b) => avatar(b, 30.0, pal),
        Mark::Glyph(i) => tone_tile(i, Tone::Neutral, pal, 30.0),
    }
}

/// A plate of rows: `paper-raised`, a `rule` hairline, 20 px corners.
pub fn card(pal: Palette) -> Div {
    div()
        .flex()
        .flex_col()
        .bg(pal.paper_raised())
        .border_1()
        .border_color(pal.rule())
        .rounded(px(20.0))
        .overflow_hidden()
}

pub fn caption(text: impl Into<SharedString>, pal: Palette) -> Div {
    div().text_size(px(12.0)).line_height(px(17.0)).text_color(pal.ink_2()).child(text.into())
}

/// A small line under a caption: tertiary, 11.5 px.
pub fn fine(text: impl Into<SharedString>, pal: Palette) -> Div {
    div().text_size(px(11.5)).line_height(px(16.0)).text_color(pal.ink_meta()).child(text.into())
}

/// One line that ends in "…" when it does not fit.
pub fn one_line(text: impl Into<SharedString>) -> Div {
    div().min_w(px(0.0)).truncate().child(text.into())
}

/// The (?): a circle-help icon with its own hit target; a click shows `text` in a bubble under it, for whoever looks
/// for the longer explanation. A click anywhere else, Esc or another page closes it. One is open at a time
/// ([`Root::help`]).
pub fn help(
    id: &'static str,
    text: impl Into<SharedString>,
    d: &Data,
    pal: Palette,
    cx: &Context<'_, Root>,
) -> Stateful<Div> {
    let text = text.into();
    // As wide as the words need, up to a comfortable line.
    #[allow(clippy::cast_precision_loss, reason = "a few hundred characters")]
    let width = (text.len() as f32).mul_add(6.3, 28.0).clamp(150.0, 320.0);
    help_with(id, move || text.into_any_element(), width, d, pal, cx)
}

/// [`help`] with something else than a line in its bubble (built only while open), `width` wide.
pub fn help_with(
    id: &'static str,
    content: impl FnOnce() -> AnyElement,
    width: f32,
    d: &Data,
    pal: Palette,
    cx: &Context<'_, Root>,
) -> Stateful<Div> {
    let open = d.help == Some(id);
    let mark = div()
        .id(SharedString::from(format!("help-{id}")))
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(px(22.0))
        .rounded_full()
        .cursor_pointer()
        .when_else(
            open,
            |m| m.bg(pal.violet().opacity(0.12)).child(glyph(Glyph::Help, 16.0, pal.violet())),
            |m| m.hover(|s| s.bg(pal.paper_sunken())).child(glyph(Glyph::Help, 16.0, pal.ink_faint())),
        )
        .on_click(cx.listener(move |this, _, _, cx| {
            // A (?) often sits in a row that is itself a toggle: the click is the (?)'s alone.
            cx.stop_propagation();
            this.help = if this.help == Some(id) {
                None
            } else {
                Some(id)
            };
            cx.notify();
        }));
    if !open {
        return mark;
    }
    let bubble = div()
        .occlude()
        .w(px(width))
        .px(px(12.0))
        .py(px(9.0))
        .rounded(px(14.0))
        .bg(pal.paper_raised())
        .border_1()
        .border_color(pal.rule())
        .shadow_lg()
        .text_size(px(12.5))
        .line_height(px(17.0))
        .font_weight(FontWeight::NORMAL)
        .text_color(pal.ink())
        .whitespace_normal()
        .child(content());
    mark.on_mouse_down_out(cx.listener(|this, _, _, cx| {
        this.help = None;
        cx.notify();
    }))
    .relative()
    // From the (?)'s top left corner: centred in it, the bubble would start half its size up and to the left.
    .child(
        div().absolute().top_0().left_0().size_0().child(
            deferred(anchored().offset(point(px(-8.0), px(26.0))).snap_to_window_with_margin(px(12.0)).child(bubble))
                .with_priority(1),
        ),
    )
}

/// The title of a section of the status window, in the serif (`title`, 44/44), with room after it (a (?), buttons).
pub fn page_header(title: &'static str, help: Option<Stateful<Div>>) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(10.0))
        .min_h(px(44.0))
        .child(div().flex_1().min_w(px(0.0)).title().child(title))
        .when_some(help, ParentElement::child)
}

/// What a button looks like (Signal's looks: [`super::kit::Look`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Btn {
    /// Soft: `paper-sunken`.
    Plain,
    /// `inverse`: continue, resume, confirm.
    Primary,
}

impl Btn {
    /// Signal's look for it.
    #[must_use]
    pub fn look(self) -> super::kit::Look {
        match self {
            Self::Plain => super::kit::Look::Soft,
            Self::Primary => super::kit::Look::Inverse,
        }
    }
}

/// A button's frame in `look` (a pill, 32 px; dims to 55% while pressed), and the colour of what goes on it.
fn btn_frame(id: impl Into<ElementId>, look: Btn, pal: Palette) -> (Stateful<Div>, Hsla) {
    let (bg, fg) = look.look().colors(pal);
    let b = div()
        .id(id.into())
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .gap(px(6.0))
        .h(px(32.0))
        .px(px(14.0))
        .rounded_full()
        .bg(bg)
        .text_color(fg)
        .text_size(px(13.0))
        .font_weight(FontWeight::SEMIBOLD)
        .whitespace_nowrap()
        .cursor_pointer()
        .active(|s| s.opacity(super::kit::button::PRESSED));
    (b, fg)
}

/// A button: 32 px, semibold, in `look`.
pub fn btn(id: impl Into<ElementId>, label: impl Into<SharedString>, look: Btn, pal: Palette) -> Stateful<Div> {
    btn_frame(id, look, pal).0.child(label.into())
}

/// [`btn`] with an icon before its label.
pub fn btn_icon(
    id: impl Into<ElementId>,
    i: Glyph,
    label: impl Into<SharedString>,
    look: Btn,
    pal: Palette,
) -> Stateful<Div> {
    let (b, fg) = btn_frame(id, look, pal);
    b.child(glyph(i, 14.0, fg)).child(label.into())
}

/// A smaller [`btn`], for tiles and chips' rows.
pub fn small(b: Stateful<Div>) -> Stateful<Div> {
    b.h(px(28.0)).px(px(12.0)).text_size(px(12.5))
}

/// A bigger button, for the one thing a screen is for (the desktop's 38 px).
pub fn big(b: Stateful<Div>) -> Stateful<Div> {
    b.h(px(38.0)).px(px(18.0)).text_size(px(14.0))
}

pub fn link(id: impl Into<ElementId>, label: impl Into<SharedString>, pal: Palette) -> Stateful<Div> {
    div()
        .id(id.into())
        .text_size(px(13.0))
        .font_weight(FontWeight::MEDIUM)
        .text_color(pal.violet())
        .whitespace_nowrap()
        .cursor_pointer()
        .hover(|s| s.opacity(0.75))
        .child(label.into())
}

/// A quiet link (`Desk-Setup.png`'s "Use the browser", "Undo"): `ink-2`, underlined, `size` px.
pub fn quiet_link(id: impl Into<ElementId>, label: impl Into<SharedString>, size: f32, pal: Palette) -> Stateful<Div> {
    div()
        .id(id.into())
        .text_size(px(size))
        .text_color(pal.ink_2())
        .underline()
        .whitespace_nowrap()
        .cursor_pointer()
        .hover(|s| s.text_color(pal.ink()))
        .active(|s| s.opacity(super::kit::button::PRESSED))
        .child(label.into())
}

/// A row of a card: 50 px at least, a hairline above all but the first.
pub fn row(pal: Palette, first: bool) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(12.0))
        .px(px(14.0))
        .py(px(10.0))
        .min_h(px(50.0))
        .min_w(px(0.0))
        .text_size(px(14.0))
        .when(!first, |r| r.border_t_1().border_color(pal.rule()))
}

/// A row's text: a title and, under it, a smaller line.
pub fn row_text(title: impl Into<SharedString>, under: Option<AnyElement>, pal: Palette) -> Div {
    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_w(px(0.0))
        .gap(px(2.0))
        .child(one_line(title).font_weight(FontWeight::MEDIUM))
        .when_some(under, |c, u| {
            c.child(div().text_size(px(12.0)).line_height(px(16.0)).text_color(pal.ink_meta()).min_w(px(0.0)).child(u))
        })
}

/// A dot that breathes (35% to 100%, still under Reduce Motion): something waits.
pub fn waiting_dot(id: impl Into<ElementId>, color: Hsla, side: f32) -> impl IntoElement {
    motion::breathe(div().flex_none().size(px(side)).rounded_full().bg(color), id)
}

/// A chip: an icon and a few words, tinted with `color`.
pub fn chip(i: Option<Glyph>, label: impl Into<SharedString>, color: Hsla) -> Div {
    chip_frame(i.is_some(), color).when_some(i, |c, i| c.child(glyph(i, 13.0, color))).child(label.into())
}

fn chip_frame(lead: bool, color: Hsla) -> Div {
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap(px(6.0))
        .h(px(26.0))
        .pl(px(if lead {
            8.0
        } else {
            10.0
        }))
        .pr(px(10.0))
        .rounded_full()
        .bg(color.opacity(0.13))
        .text_color(color)
        .text_size(px(12.0))
        .font_weight(FontWeight::SEMIBOLD)
        .whitespace_nowrap()
}

/// A grey chip ("Asks next time", "By your rules").
pub fn chip_plain(i: Option<Glyph>, label: impl Into<SharedString>, pal: Palette) -> Div {
    chip(i, label, pal.ink_2()).bg(pal.paper_sunken())
}

/// A ring that shows how much is left: `fraction` of it in `color`, the rest a faint track. Starts at the top and
/// goes clockwise.
pub fn ring(fraction: f32, color: Hsla, track: Hsla, side: f32, stroke: f32) -> impl IntoElement {
    let fraction = fraction.clamp(0.0, 1.0);
    canvas(
        |_, _, _| {},
        move |bounds, (), window, _| {
            let s = bounds.size.width.as_f32().min(bounds.size.height.as_f32());
            let r = (s - stroke) / 2.0;
            let c = point(bounds.origin.x + px(s / 2.0), bounds.origin.y + px(s / 2.0));
            let at = |turn: f32| {
                let a = turn.mul_add(std::f32::consts::TAU, -std::f32::consts::FRAC_PI_2);
                point(c.x + px(r * a.cos()), c.y + px(r * a.sin()))
            };
            // The track: a full circle, in two halves (an arc cannot end where it starts).
            let mut path = PathBuilder::stroke(px(stroke));
            path.move_to(at(0.0));
            path.arc_to(point(px(r), px(r)), px(0.0), false, true, at(0.5));
            path.arc_to(point(px(r), px(r)), px(0.0), false, true, at(0.999));
            if let Ok(p) = path.build() {
                window.paint_path(p, track);
            }
            if fraction > 0.001 {
                let mut arc = PathBuilder::stroke(px(stroke));
                arc.move_to(at(0.0));
                let end = fraction.min(0.999);
                if end > 0.5 {
                    arc.arc_to(point(px(r), px(r)), px(0.0), false, true, at(0.5));
                }
                arc.arc_to(point(px(r), px(r)), px(0.0), false, true, at(end));
                if let Ok(p) = arc.build() {
                    window.paint_path(p, color);
                }
            }
        },
    )
    .size(px(side))
    .flex_none()
}

/// Three dots between two tiles; the last in `color` (where it goes).
pub fn dots(color: Hsla, pal: Palette) -> Div {
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap(px(4.0))
        .child(div().size(px(5.0)).rounded_full().bg(pal.ink_faint()))
        .child(div().size(px(5.0)).rounded_full().bg(pal.ink_faint()))
        .child(div().size(px(5.0)).rounded_full().bg(color))
}

/// One of the stage's squares: a logo or an icon, and maybe a badge on its corner.
pub fn stage_tile(m: Mark, side: f32, badge: Option<(Glyph, Hsla)>, behind: Hsla, pal: Palette) -> Div {
    div()
        .relative()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(px(side))
        .rounded(px(side * 0.31))
        .bg(pal.paper_sunken())
        .child(match m {
            // A logo sits in its white circle, so dark marks stay visible on dark paper.
            Mark::Brand(b) => avatar(b, side * 0.62, pal).border_0().into_any_element(),
            Mark::Glyph(_) => m.render(side * 0.5, pal.ink()),
        })
        .when_some(badge, |t, (i, color)| {
            t.child(
                div()
                    .absolute()
                    .right(px(-6.0))
                    .bottom(px(-6.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .size(px(22.0))
                    .rounded(px(7.0))
                    .border_2()
                    .border_color(behind)
                    .bg(pal.paper_raised().blend(color.opacity(0.18)))
                    .child(glyph(i, 12.0, color)),
            )
        })
}

/// Who → what: two squares and three dots between them, the last dot in `color`.
pub fn stage(from: Mark, to: Mark, badge: Option<(Glyph, Hsla)>, color: Hsla, side: f32, pal: Palette) -> Div {
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap(px(12.0))
        .child(stage_tile(from, side, None, pal.paper(), pal))
        .child(dots(color, pal))
        .child(stage_tile(to, side, badge, pal.paper_raised(), pal))
}

/// A sentence with its important words (the target) in italic and `ink`; the rest in the caller's colour. Set it in
/// the serif ([`SERIF`]): the italic is Instrument Serif's own.
pub fn sentence(words: &[Words], pal: Palette) -> StyledText {
    let mut text = String::new();
    let mut highlights = Vec::new();
    for w in words {
        let start = text.len();
        text.push_str(&w.text);
        if w.bold {
            highlights.push((
                start..text.len(),
                HighlightStyle {
                    color: Some(pal.ink()),
                    font_style: Some(FontStyle::Italic),
                    ..HighlightStyle::default()
                },
            ));
        }
    }
    StyledText::new(text).with_highlights(highlights)
}

/// Words with the important ones in italic and nothing else changed (the colour is the caller's: white on a field).
pub fn italic(words: &[Words]) -> StyledText {
    let mut text = String::new();
    let mut highlights = Vec::new();
    for w in words {
        let start = text.len();
        text.push_str(&w.text);
        if w.bold {
            highlights.push((
                start..text.len(),
                HighlightStyle {
                    font_style: Some(FontStyle::Italic),
                    ..HighlightStyle::default()
                },
            ));
        }
    }
    StyledText::new(text).with_highlights(highlights)
}

/// The quiet line under a page's title: serif italic 21 in `ink-2` (the desktop boards' size).
pub fn mood(text: impl Into<SharedString>, pal: Palette) -> Div {
    div()
        .mt(px(8.0))
        .font_family(SERIF)
        .text_size(px(21.0))
        .line_height(px(26.0))
        .italic()
        .text_color(pal.ink_2())
        .child(text.into())
}

/// A page's title (serif 44, `words` with the italic ones in italic) and the mood line under it; `right` beside them,
/// at the bottom (a button).
pub fn page_title(words: &[Words], line: Option<&str>, right: Option<AnyElement>, pal: Palette) -> Div {
    div()
        .flex()
        .flex_wrap()
        .items_end()
        .gap(px(16.0))
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w(px(260.0))
                .child(div().title().text_color(pal.ink()).child(italic(words)))
                .when_some(line, |c, l| c.child(mood(l.to_owned(), pal))),
        )
        .when_some(right, ParentElement::child)
}

/// A row of a page's list (the desktop boards' `.row`): 54 px at least, a hairline under it but the last. Rows divide
/// lists; they are never cards.
pub fn list_row(pal: Palette, last: bool) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(12.0))
        .min_h(px(54.0))
        .py(px(6.0))
        .min_w(px(0.0))
        .text_size(px(14.0))
        .when(!last, |r| r.border_b_1().border_color(pal.rule()))
}

/// A row's title (14.5, 500) and its meta line (12.5, `ink-meta`) under it.
pub fn title_meta(title: impl IntoElement, meta: Option<AnyElement>, pal: Palette) -> Div {
    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_w(px(0.0))
        .child(
            div()
                .min_w(px(0.0))
                .truncate()
                .text_size(px(14.5))
                .line_height(px(19.0))
                .font_weight(FontWeight::MEDIUM)
                .text_color(pal.ink())
                .child(title),
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

/// A mono line (a path, a vault reference, a SHA) as a row's title or meta.
pub fn mono(text: impl Into<SharedString>, size: f32) -> Div {
    div().min_w(px(0.0)).truncate().font_family(MONO).text_size(px(size)).child(text.into())
}

/// A section of a page: its label (and a (?) about it), then `body` 4 px under.
pub fn section(label: &str, help: Option<Stateful<Div>>, body: impl IntoElement, pal: Palette) -> Div {
    div()
        .flex()
        .flex_col()
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(6.0))
                .h(px(22.0))
                .child(tracked(label.to_owned()).text_color(pal.ink_meta()))
                .when_some(help, ParentElement::child),
        )
        .child(div().mt(px(2.0)).child(body))
}

/// Two columns 40 px apart while each has 380 px, else one under the other (the boards' `auto-fit` grids).
pub fn columns(left: Div, right: Div, min: f32) -> Div {
    div()
        .flex()
        .flex_wrap()
        .items_start()
        .gap(px(40.0))
        .child(left.flex_1().min_w(px(min)))
        .child(right.flex_1().min_w(px(min)))
}

/// The facts of a request as hairline rows: the key in `ink-2` (`key_width` wide, or pushing the value right), the
/// value in `ink`, mono for the machine's.
pub fn fact_rows(rows: &[super::story::Fact], min_h: f32, key_width: Option<f32>, pal: Palette) -> Div {
    let mut list = div().flex().flex_col();
    for (i, f) in rows.iter().enumerate() {
        let key = div().flex_none().text_color(pal.ink_2()).child(f.key.clone());
        let key = match key_width {
            Some(w) => key.w(px(w)),
            None => key.flex_1(),
        };
        let value = div()
            .min_w(px(0.0))
            .text_color(pal.ink())
            .when_else(
                f.mono,
                |v| v.font_family(MONO).text_size(px(13.0)),
                |v| {
                    v.text_size(if key_width.is_some() {
                        px(14.0)
                    } else {
                        px(13.5)
                    })
                },
            )
            .when(key_width.is_some(), gpui::Styled::flex_1)
            .child(f.value.clone());
        list = list.child(
            div()
                .flex()
                .items_center()
                .gap(px(12.0))
                .min_h(px(min_h))
                .py(px(6.0))
                .text_size(px(14.0))
                .when(i + 1 < rows.len() || key_width.is_some(), |r| r.border_b_1().border_color(pal.rule()))
                .child(key)
                .child(value),
        );
    }
    list
}

/// A block of mono text in a well (a request's detail, a code sample).
pub fn well(pal: Palette) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(px(3.0))
        .px(px(14.0))
        .py(px(12.0))
        .rounded(px(14.0))
        .bg(pal.paper_sunken())
        .border_1()
        .border_color(pal.rule())
        .font_family(MONO)
        .text_size(px(12.5))
        .line_height(px(19.0))
        .text_color(pal.ink())
        .min_w(px(0.0))
        .overflow_hidden()
}

/// A note that something needs doing first.
pub fn warning(text: impl Into<SharedString>, pal: Palette) -> Div {
    card(pal)
        .w_full()
        .flex_row()
        .items_center()
        .gap(px(10.0))
        .p(px(12.0))
        .bg(pal.warning.opacity(0.08))
        .border_color(pal.warning.opacity(0.3))
        .child(glyph(Glyph::Warning, 16.0, pal.warning))
        .child(caption(text, pal).text_color(pal.ink()).flex_1().min_w(px(0.0)))
}

/// What a card says while it has nothing to show.
pub fn empty(text: impl Into<SharedString>, pal: Palette) -> Div {
    div().px(px(14.0)).py(px(16.0)).child(caption(text, pal).text_color(pal.ink_meta()))
}

/// The QR code that opens Activity in the phone app.
pub fn phone_qr(pal: Palette) -> Div {
    card(pal)
        .p(px(14.0))
        .flex_row()
        .items_center()
        .gap(px(16.0))
        .child(div().flex_none().p(px(6.0)).rounded(px(12.0)).bg(gpui::white()).child(crate::qr::element(
            crate::links::PHONE_ACTIVITY,
            104.0,
            gpui::black(),
            gpui::white(),
        )))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(8.0))
                .flex_1()
                .min_w(px(0.0))
                .child(glyph(Glyph::Phone, 16.0, pal.ink_2()))
                .child(caption("Scan to open Activity on your phone", pal)),
        )
}

/// The welcome flow's steps (`Desk-Setup.png`): "1 Pair — 2 AI tools — 3 Turn on — ✓ Done". Behind: ink dots with a
/// check; here: an ink ring; ahead: a hairline ring. Numbers in mono.
pub fn step_indicator(current: Stage, pal: Palette) -> Div {
    let mut bar = div().flex().items_center().justify_center().gap(px(8.0));
    for (i, stage) in Stage::ALL.into_iter().enumerate() {
        let behind = stage < current || (current == Stage::Done && stage != Stage::Done);
        let here = stage == current;
        if i > 0 {
            bar = bar.child(div().flex_none().w(px(28.0)).h(px(1.5)).bg(pal.rule_2()));
        }
        let (ink, ring) = if behind {
            (pal.paper(), None)
        } else if here {
            (pal.ink(), Some(pal.ink()))
        } else {
            (pal.ink_meta(), Some(pal.rule_2()))
        };
        // Behind, and Done itself: a check; the rest their number.
        let mark: AnyElement = match stage.number() {
            Some(n) if !behind => n.to_string().into_any_element(),
            _ => glyph(Glyph::Check, 12.0, ink).into_any_element(),
        };
        let circle = div()
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .size(px(22.0))
            .rounded_full()
            .font_family(MONO)
            .text_size(px(11.0))
            .text_color(ink)
            .when_some(ring, |c, r| c.border(px(1.5)).border_color(r))
            .when(behind, |c| c.bg(pal.ink()))
            .child(mark);
        bar = bar.child(
            div().flex().flex_none().items_center().gap(px(8.0)).child(circle).child(
                // One weight for every label: a bolder current step would widen the bar and shift it sideways.
                div()
                    .text_size(px(12.5))
                    .whitespace_nowrap()
                    .text_color(if here {
                        pal.ink()
                    } else {
                        pal.ink_meta()
                    })
                    .child(stage.label()),
            ),
        );
    }
    bar
}

/// The sentence a screen of the welcome flow is about: the serif at 40/42, centred, its target in italic; and a (?).
pub fn welcome_title(words: &[Words], help: Option<Stateful<Div>>, pal: Palette) -> Div {
    div()
        .flex()
        .items_center()
        .justify_center()
        .gap(px(8.0))
        .mt(px(6.0))
        .child(
            div()
                .font_family(SERIF)
                .text_size(px(40.0))
                .line_height(px(42.0))
                .font_weight(FontWeight::NORMAL)
                .text_color(pal.ink())
                .text_center()
                .child(italic(words)),
        )
        .when_some(help, ParentElement::child)
}
