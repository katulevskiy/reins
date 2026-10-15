//! Building blocks the screens use today, restyled in Reins Signal (phase 1: the same layout, Signal's colours,
//! type, glyphs and press states; phase 2 moves the screens onto [`super::kit`]): plates with hairlines, rows with
//! kind tiles, switches, pills, segmented controls, the (?) and its bubble, rings that count down, and the "stage"
//! (who → what) of a request. Icons are the Signal glyphs, logos the brands' own marks ([`super::icons`]).

use std::sync::{Arc, LazyLock};

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, Context, Div, ElementId, FontStyle, FontWeight, HighlightStyle, Hsla, Image, ImageFormat,
    InteractiveElement as _, IntoElement, ParentElement, PathBuilder, SharedString, Stateful,
    StatefulInteractiveElement as _, Styled as _, StyledText, anchored, canvas, deferred, div, img, point, px,
};
use reins_desktop::doctor::Level;

use super::icons::{Icon, Mark, avatar, icon};
use super::story::Words;
use super::{Data, Root};
use crate::model::Step;
use crate::theme::text::tracked;
use crate::theme::{MONO, Palette, SERIF, SignalText as _, Tone, motion};
use crate::tray::Look;
use crate::welcome::Stage;

static MARK: LazyLock<Arc<Image>> = LazyLock::new(|| {
    Arc::new(Image::from_bytes(ImageFormat::Png, include_bytes!("../../assets/icons/app-256.png").to_vec()))
});

/// The Reins mark (the app icon).
pub fn mark(side: f32) -> impl IntoElement {
    img(MARK.clone()).size(px(side)).flex_none()
}

/// "this Mac", "this computer": what the window calls the computer it runs on.
#[must_use]
pub fn this_computer() -> &'static str {
    if cfg!(target_os = "macos") {
        "this Mac"
    } else {
        "this computer"
    }
}

/// A check mark.
pub fn check(side: f32, color: Hsla) -> impl IntoElement {
    icon(Icon::Check, side, color)
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
        .child(icon(Icon::Check, side * 0.5, color))
}

/// A kind tile: an icon in `color` over `color` at 13% (a kind, a rule, a setting).
pub fn tile(i: Icon, color: Hsla, side: f32) -> Div {
    div()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(px(side))
        .rounded(px(side * 0.31))
        .bg(color.opacity(0.13))
        .child(icon(i, side * 0.53, color))
}

/// A kind tile of `tone` (a neutral one is `paper-sunken` under `ink`).
pub fn tone_tile(i: Icon, tone: Tone, pal: Palette, side: f32) -> Div {
    super::kit::kind_tile(i.glyph(), tone, side, pal)
}

/// What was a solid square of colour (a setting of "This computer"): Signal colours only what needs a decision, so
/// a neutral tile, `ink` on `paper-sunken`.
pub fn solid_tile(i: Icon, _tone: Tone, pal: Palette, side: f32) -> Div {
    tone_tile(i, Tone::Neutral, pal, side)
}

/// A logo (an AI tool, a git host, an API): a brand's mark in a white circle, an icon in a neutral tile.
pub fn logo(m: Mark, pal: Palette) -> Div {
    match m {
        Mark::Brand(b) => avatar(b, 30.0, pal),
        Mark::Icon(i) => tone_tile(i, Tone::Neutral, pal, 30.0),
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
            |m| m.bg(pal.accent_soft).child(icon(Icon::CircleHelp, 16.0, pal.accent)),
            |m| m.hover(|s| s.bg(pal.control_fill)).child(icon(Icon::CircleHelp, 16.0, pal.ink_faint())),
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
        .bg(pal.popover())
        .border_1()
        .border_color(pal.popover_border())
        .shadow_lg()
        .text_size(px(12.5))
        .line_height(px(17.0))
        .font_weight(FontWeight::NORMAL)
        .text_color(pal.text)
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

/// A block's label ("LATEST"): mono capitals, tracked, `ink-meta`; with room after it for a link.
pub fn group_title(title: &str, pal: Palette) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(8.0))
        .px(px(4.0))
        .h(px(18.0))
        .child(div().flex_1().min_w(px(0.0)).child(tracked(title.to_owned()).text_color(pal.ink_meta())))
}

/// A titled block of a page: its small capitals title (and a (?) about it), then `body` (usually a card).
pub fn group(title: &str, help: Option<Stateful<Div>>, body: impl IntoElement, pal: Palette) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(px(8.0))
        .child(group_title(title, pal).when_some(help, ParentElement::child))
        .child(body)
}

/// What a button looks like (Signal's looks: [`super::kit::Look`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Btn {
    /// Soft: `paper-sunken`.
    Plain,
    /// `violet-fill`: connect, start.
    Accent,
    /// `inverse`: continue, resume, confirm.
    Primary,
    /// Red on `danger-wash`: end, disconnect.
    Danger,
}

impl Btn {
    /// Signal's look for it.
    #[must_use]
    pub fn look(self) -> super::kit::Look {
        match self {
            Self::Plain => super::kit::Look::Soft,
            Self::Accent => super::kit::Look::Violet,
            Self::Primary => super::kit::Look::Inverse,
            Self::Danger => super::kit::Look::DangerWash,
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
    i: Icon,
    label: impl Into<SharedString>,
    look: Btn,
    pal: Palette,
) -> Stateful<Div> {
    let (b, fg) = btn_frame(id, look, pal);
    b.child(icon(i, 14.0, fg)).child(label.into())
}

/// A button: violet when `primary`, grey otherwise; faded and inert when not `enabled`.
pub fn button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    pal: Palette,
    primary: bool,
    enabled: bool,
) -> Stateful<Div> {
    let b = btn(
        id,
        label,
        if primary {
            Btn::Accent
        } else {
            Btn::Plain
        },
        pal,
    );
    if enabled {
        b
    } else {
        super::kit::disabled(b)
    }
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
        .text_color(pal.accent)
        .whitespace_nowrap()
        .cursor_pointer()
        .hover(|s| s.opacity(0.75))
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
        .when(!first, |r| r.border_t_1().border_color(pal.hairline))
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
            c.child(div().text_size(px(12.0)).line_height(px(16.0)).text_color(pal.tertiary).min_w(px(0.0)).child(u))
        })
}

/// A switch: `violet-fill` on, `paper-sunken` off.
pub fn toggle(on: bool, pal: Palette) -> Div {
    let knob = div().size(px(17.0)).rounded_full().bg(gpui::white()).shadow_sm();
    div()
        .flex()
        .flex_none()
        .items_center()
        .w(px(36.0))
        .h(px(21.0))
        .px(px(2.0))
        .rounded_full()
        .bg(if on {
            pal.violet_fill()
        } else {
            pal.paper_sunken()
        })
        .when(!on && pal.dark, |t| t.border_1().border_color(pal.rule_2()))
        .when_else(on, Div::justify_end, Div::justify_start)
        .child(knob)
}

/// What Reins is doing, as a colour (as the shield says it): green on, violet waiting, red paused (the desktop's
/// bypass), amber when it needs you.
pub fn look_color(look: Look, pal: Palette) -> Hsla {
    match look {
        Look::On => pal.success,
        Look::Paused => pal.danger,
        Look::Waiting => pal.violet(),
        Look::Attention => pal.warning,
    }
}

/// A dot that breathes (35% to 100%, still under Reduce Motion): something waits.
pub fn waiting_dot(id: impl Into<ElementId>, color: Hsla, side: f32) -> impl IntoElement {
    motion::breathe(div().flex_none().size(px(side)).rounded_full().bg(color), id)
}

/// A chip: an icon and a few words, tinted with `color`.
pub fn chip(i: Option<Icon>, label: impl Into<SharedString>, color: Hsla) -> Div {
    chip_frame(i.is_some(), color).when_some(i, |c, i| c.child(icon(i, 13.0, color))).child(label.into())
}

/// A chip with something else before its words (a ring).
pub fn chip_lead(lead: impl IntoElement, label: impl Into<SharedString>, color: Hsla) -> Div {
    chip_frame(true, color).child(lead).child(label.into())
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
pub fn chip_plain(i: Option<Icon>, label: impl Into<SharedString>, pal: Palette) -> Div {
    chip(i, label, pal.ink_2()).bg(pal.paper_sunken())
}

/// A filter's choice: its label, how many, selected or not.
pub fn filter_chip(
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
        .active(|s| s.opacity(super::kit::button::PRESSED))
        .when_else(
            selected,
            |c| c.bg(pal.inverse()).text_color(pal.on_inverse()),
            |c| {
                c.bg(pal.paper_raised())
                    .border_1()
                    .border_color(pal.rule())
                    .text_color(pal.ink_2())
                    .hover(|s| s.text_color(pal.ink()))
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

/// The track of a segmented control: a `paper-sunken` pill.
pub fn segmented(pal: Palette) -> Div {
    div().flex().flex_none().items_center().p(px(3.0)).gap(px(2.0)).rounded_full().bg(pal.paper_sunken())
}

/// One choice of a segmented control, without its label (for one with an icon too).
pub fn segment_frame(id: impl Into<ElementId>, selected: bool, pal: Palette) -> Stateful<Div> {
    div()
        .id(id.into())
        .flex()
        .flex_1()
        .items_center()
        .justify_center()
        .gap(px(4.0))
        .h(px(30.0))
        .px(px(10.0))
        .rounded_full()
        .text_size(px(13.0))
        .font_weight(FontWeight::MEDIUM)
        .whitespace_nowrap()
        .cursor_pointer()
        .active(|s| s.opacity(super::kit::button::PRESSED))
        .when_else(
            selected,
            |s| s.bg(pal.paper_raised()).border_1().border_color(pal.rule()).text_color(pal.ink()).shadow_xs(),
            |s| s.text_color(pal.ink_2()).hover(|h| h.text_color(pal.ink())),
        )
}

/// One choice of a segmented control.
pub fn segment(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    selected: bool,
    pal: Palette,
) -> Stateful<Div> {
    segment_frame(id, selected, pal).child(label.into())
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
pub fn stage_tile(m: Mark, side: f32, badge: Option<(Icon, Hsla)>, behind: Hsla, pal: Palette) -> Div {
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
            Mark::Icon(_) => m.render(side * 0.5, pal.ink()),
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
                    .child(icon(i, 12.0, color)),
            )
        })
}

/// Who → what: two squares and three dots between them, the last dot in `color`.
pub fn stage(from: Mark, to: Mark, badge: Option<(Icon, Hsla)>, color: Hsla, side: f32, pal: Palette) -> Div {
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap(px(12.0))
        .child(stage_tile(from, side, None, pal.background, pal))
        .child(dots(color, pal))
        .child(stage_tile(to, side, badge, pal.elevated, pal))
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

/// A block of mono text in a well (a request's detail, a code sample).
pub fn well(pal: Palette) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(px(3.0))
        .px(px(14.0))
        .py(px(12.0))
        .rounded(px(14.0))
        .bg(pal.well())
        .border_1()
        .border_color(pal.hairline)
        .font_family(MONO)
        .text_size(px(12.5))
        .line_height(px(19.0))
        .text_color(pal.text)
        .min_w(px(0.0))
        .overflow_hidden()
}

/// A label on the left, its value on the right.
pub fn info_row(label: &str, value: impl IntoElement, pal: Palette, first: bool) -> Div {
    row(pal, first)
        .child(div().flex_1().text_color(pal.ink_2()).child(label.to_owned()))
        .child(div().min_w(px(0.0)).flex().justify_end().text_color(pal.text).child(value))
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
        .child(icon(Icon::TriangleAlert, 16.0, pal.warning))
        .child(caption(text, pal).text_color(pal.text).flex_1().min_w(px(0.0)))
}

/// Lines of code to copy, in a well.
pub fn code_block(lines: &[String], pal: Palette) -> Div {
    let mut block = well(pal).text_size(px(12.0)).line_height(px(18.0));
    for line in lines {
        block = block.child(div().whitespace_nowrap().child(line.clone()));
    }
    block
}

/// What a card says while it has nothing to show.
pub fn empty(text: impl Into<SharedString>, pal: Palette) -> Div {
    div().px(px(14.0)).py(px(16.0)).child(caption(text, pal).text_color(pal.tertiary))
}

/// A small mono tag (`vault:OpenAI/api key`).
pub fn tag(text: impl Into<SharedString>, pal: Palette) -> Div {
    div()
        .flex_none()
        .px(px(7.0))
        .py(px(1.0))
        .rounded(px(6.0))
        .bg(pal.paper_sunken())
        .font_family(MONO)
        .text_size(px(11.5))
        .line_height(px(17.0))
        .text_color(pal.secondary)
        .whitespace_nowrap()
        .child(text.into())
}

/// A step of "Turn on Reins" as it goes: a breathing dot, a check, or a cross, and what happened.
pub fn step_line(label: &str, step: &Step, pal: Palette, first: bool) -> Div {
    let (mark, detail): (AnyElement, String) = match step {
        Step::Running => (
            waiting_dot(SharedString::from(format!("step-{label}")), pal.accent, 8.0).into_any_element(),
            String::new(),
        ),
        Step::Done(d) => (icon(Icon::CircleCheck, 18.0, pal.success).into_any_element(), d.clone()),
        Step::Failed(e) => (icon(Icon::CircleX, 18.0, pal.danger).into_any_element(), e.clone()),
    };
    row(pal, first)
        .child(div().flex().flex_none().items_center().justify_center().size(px(30.0)).child(mark))
        .child(row_text(label.to_owned(), (!detail.is_empty()).then(|| detail.into_any_element()), pal))
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
                .child(icon(Icon::Smartphone, 16.0, pal.secondary))
                .child(caption("Scan to open Activity on your phone", pal)),
        )
}

/// The colour of a health check's level.
pub fn level_color(level: Level, pal: Palette) -> Hsla {
    match level {
        Level::Ok => pal.success,
        Level::Warn => pal.warning,
        Level::Fail => pal.danger,
        Level::Skip => pal.tertiary,
    }
}

/// A check's level as an icon in a soft square of its colour.
pub fn level_mark(level: Level, side: f32, pal: Palette) -> Div {
    let i = match level {
        Level::Ok => Icon::CircleCheck,
        Level::Warn => Icon::TriangleAlert,
        Level::Fail => Icon::CircleX,
        Level::Skip => Icon::CircleAlert,
    };
    tile(i, level_color(level, pal), side)
}

/// The welcome flow's steps: "1 Pair — 2 AI tools — 3 Turn on — Done", the ones behind ticked.
pub fn step_indicator(current: Stage, pal: Palette) -> Div {
    let mut bar = div().flex().items_center().justify_center().gap(px(12.0));
    for (i, stage) in Stage::ALL.into_iter().enumerate() {
        let behind = stage < current || current == Stage::Done;
        let here = stage == current;
        if i > 0 {
            bar = bar.child(div().w(px(36.0)).h(px(1.5)).rounded_full().bg(if stage <= current {
                pal.violet_fill()
            } else {
                pal.rule_2()
            }));
        }
        let mark: AnyElement = match stage.number() {
            _ if behind || (here && stage == Stage::Done) => icon(Icon::Check, 12.0, pal.on_accent).into_any_element(),
            Some(n) => n.to_string().into_any_element(),
            None => div().into_any_element(),
        };
        let circle = div()
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .size(px(22.0))
            .rounded_full()
            .text_size(px(11.5))
            .font_weight(FontWeight::SEMIBOLD)
            .when_else(
                behind || (here && stage == Stage::Done),
                |c| c.bg(pal.violet_fill()),
                |c| {
                    c.border(px(1.5)).when_else(
                        here,
                        |c| c.border_color(pal.violet()).bg(pal.violet().opacity(0.13)).text_color(pal.violet()),
                        |c| c.border_color(pal.rule_2()).text_color(pal.ink_meta()),
                    )
                },
            )
            .child(mark);
        bar = bar.child(
            div().flex().items_center().gap(px(8.0)).child(circle).child(
                // One weight for every label: a bolder current step would widen the bar and shift it sideways.
                div()
                    .text_size(px(12.5))
                    .whitespace_nowrap()
                    .text_color(if here {
                        pal.text
                    } else {
                        pal.tertiary
                    })
                    .child(stage.label()),
            ),
        );
    }
    bar
}

/// The sentence a screen of the welcome flow is about, centred, in the serif with its target in italic; and a (?).
pub fn welcome_title(words: &[Words], help: Option<Stateful<Div>>, pal: Palette) -> Div {
    div()
        .flex()
        .items_center()
        .justify_center()
        .gap(px(8.0))
        .child(
            div()
                .font_family(SERIF)
                .text_size(px(32.0))
                .line_height(px(36.0))
                .font_weight(FontWeight::NORMAL)
                .text_color(pal.ink())
                .text_center()
                .child(sentence(words, pal)),
        )
        .when_some(help, ParentElement::child)
}

/// A card's empty state: an icon in a soft square and a title (with a (?) about it); the caller adds an action.
pub fn empty_state(i: Icon, title: impl Into<SharedString>, help: Option<Stateful<Div>>, pal: Palette) -> Div {
    div()
        .flex()
        .flex_col()
        .items_center()
        .gap(px(10.0))
        .px(px(28.0))
        .py(px(24.0))
        .child(tone_tile(i, Tone::Neutral, pal, 40.0))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(4.0))
                .child(div().text_size(px(14.0)).font_weight(FontWeight::SEMIBOLD).text_center().child(title.into()))
                .when_some(help, ParentElement::child),
        )
}

/// A key on the keyboard ("Ctrl+1").
pub fn kbd(text: impl Into<SharedString>, pal: Palette) -> Div {
    div()
        .flex_none()
        .font_family(MONO)
        .text_size(px(12.0))
        .text_color(pal.ink_2())
        .whitespace_nowrap()
        .child(text.into())
}

/// A tile on Home: a plate with its label (mono capitals, tracked), a value, and a line or an action under it.
pub fn stat(i: Icon, color: Hsla, label: &str, pal: Palette) -> Div {
    let _ = (i, color);
    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_w(px(0.0))
        .gap(px(3.0))
        .px(px(20.0))
        .py(px(18.0))
        .rounded(px(20.0))
        .bg(pal.paper_raised())
        .border_1()
        .border_color(pal.rule())
        .child(tracked(label.to_owned()).text_color(pal.ink_meta()))
}

/// A popover's frame: a menu's paper ([`super::kit::menu`]), with room inside.
pub fn popover(pal: Palette) -> Div {
    super::kit::menu(pal).w_auto().p(px(14.0))
}
