//! Building blocks, in the phone apps' language: cards with hairlines, rows with coloured icon squares, toggles,
//! chips, segmented controls, the (?) and its bubble, rings that count down, and the "stage" (who → what) of a
//! request. Icons are Lucide's, logos Simple Icons' ([`super::icons`]); no glyph stands in for an icon.

use std::sync::{Arc, LazyLock};
use std::time::Duration;

use gpui::prelude::FluentBuilder as _;
use gpui::{
    Animation, AnimationExt as _, AnyElement, Context, Div, ElementId, FontWeight, HighlightStyle, Hsla, Image,
    ImageFormat, InteractiveElement as _, IntoElement, ParentElement, PathBuilder, SharedString, Stateful,
    StatefulInteractiveElement as _, Styled as _, StyledText, anchored, canvas, deferred, div, img, point,
    pulsating_between, px,
};
use reins_desktop::doctor::Level;

use super::icons::{Icon, Mark, icon};
use super::story::Words;
use super::{Data, Root};
use crate::model::Step;
use crate::theme::{MONO, Palette, Tone};
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

/// A soft square of `color` with an icon in it (a kind, a rule, a setting).
pub fn tile(i: Icon, color: Hsla, side: f32) -> Div {
    div()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(px(side))
        .rounded(px(side * 0.3))
        .bg(color.opacity(0.14))
        .child(icon(i, side * 0.53, color))
}

/// A solid square of `tone` with a white icon (a selected section, a setting of "This computer").
pub fn solid_tile(i: Icon, tone: Tone, pal: Palette, side: f32) -> Div {
    div()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(px(side))
        .rounded(px(side * 0.3))
        .bg(pal.tone(tone))
        .child(icon(i, side * 0.5, pal.on_solid(tone)))
}

/// A logo in its neutral square (an AI tool, a git host, an API).
pub fn logo(m: Mark, pal: Palette) -> Div {
    div()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(px(30.0))
        .rounded(px(9.0))
        .bg(pal.tile())
        .border_1()
        .border_color(pal.tile_border())
        .child(m.render(17.0, pal.text, pal))
}

pub fn card(pal: Palette) -> Div {
    div().flex().flex_col().bg(pal.elevated).border_1().border_color(pal.hairline).rounded(px(16.0)).overflow_hidden()
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
            |m| m.hover(|s| s.bg(pal.control_fill)).child(icon(Icon::CircleHelp, 16.0, pal.tertiary)),
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
        .rounded(px(12.0))
        .bg(pal.popover())
        .border_1()
        .border_color(pal.popover_border())
        .shadow_lg()
        .text_size(px(12.0))
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

/// The title of a section of the status window, with room after it (a (?), buttons).
pub fn page_header(title: &'static str, help: Option<Stateful<Div>>) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(10.0))
        .min_h(px(32.0))
        .child(
            div()
                .flex_1()
                .min_w(px(0.0))
                .text_size(px(24.0))
                .line_height(px(30.0))
                .font_weight(FontWeight::BOLD)
                .child(title),
        )
        .when_some(help, ParentElement::child)
}

/// A block's small capitals title ("LATEST"), with room after it for a link.
pub fn group_title(title: &str, pal: Palette) -> Div {
    div().flex().items_center().gap(px(8.0)).px(px(4.0)).h(px(18.0)).child(
        div()
            .flex_1()
            .min_w(px(0.0))
            .text_size(px(12.0))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(pal.tertiary)
            .child(title.to_uppercase()),
    )
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

/// What a button looks like.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Btn {
    /// A grey button.
    Plain,
    /// Violet: connect, start.
    Accent,
    /// White on dark: continue, resume, confirm.
    Primary,
    /// Red text on a red tint: end, disconnect.
    Danger,
}

/// A button's frame in `look`, and the colour of what goes on it.
fn btn_frame(id: impl Into<ElementId>, look: Btn, pal: Palette) -> (Stateful<Div>, Hsla) {
    let (bg, fg) = match look {
        Btn::Plain => (pal.button(), pal.text),
        Btn::Accent => (pal.accent, pal.on_accent),
        Btn::Primary => (pal.primary(), pal.on_primary()),
        Btn::Danger => (pal.danger.opacity(0.14), pal.danger),
    };
    let b = div()
        .id(id.into())
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .gap(px(6.0))
        .h(px(32.0))
        .px(px(13.0))
        .rounded(px(9.0))
        .bg(bg)
        .text_color(fg)
        .text_size(px(13.0))
        .font_weight(FontWeight::SEMIBOLD)
        .whitespace_nowrap()
        .cursor_pointer()
        .hover(|s| s.opacity(0.85));
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
        b.opacity(0.45).cursor_default()
    }
}

/// A smaller [`btn`], for tiles and chips' rows.
pub fn small(b: Stateful<Div>) -> Stateful<Div> {
    b.h(px(26.0)).px(px(10.0)).rounded(px(8.0)).text_size(px(12.0))
}

/// A bigger button, for the one thing a screen is for.
pub fn big(b: Stateful<Div>) -> Stateful<Div> {
    b.h(px(38.0)).px(px(20.0)).rounded(px(11.0)).text_size(px(13.5))
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
        .text_size(px(13.5))
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

pub fn toggle(on: bool, pal: Palette) -> Div {
    let knob = div().size(px(16.0)).rounded_full().bg(gpui::white()).shadow_sm();
    div()
        .flex()
        .flex_none()
        .items_center()
        .w(px(36.0))
        .h(px(21.0))
        .px(px(2.5))
        .rounded_full()
        .bg(if on {
            pal.accent
        } else if pal.dark {
            pal.tile_border()
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
        Look::Waiting => pal.accent,
        Look::Attention => pal.danger,
    }
}

/// A dot that breathes: something waits.
pub fn waiting_dot(id: impl Into<ElementId>, color: Hsla, side: f32) -> impl IntoElement {
    div().flex_none().size(px(side)).rounded_full().bg(color).with_animation(
        id,
        Animation::new(Duration::from_millis(1600)).repeat().with_easing(pulsating_between(0.25, 1.0)),
        gpui::Styled::opacity,
    )
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
    chip(i, label, pal.secondary).bg(pal.control_fill)
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
        .when_else(
            selected,
            |c| c.bg(pal.accent_soft).text_color(pal.accent),
            |c| c.bg(pal.control_fill).text_color(pal.secondary).hover(|s| s.text_color(pal.text)),
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

/// The track of a segmented control.
pub fn segmented(pal: Palette) -> Div {
    div().flex().flex_none().items_center().p(px(3.0)).gap(px(2.0)).rounded(px(11.0)).bg(if pal.dark {
        pal.well()
    } else {
        pal.control_fill
    })
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
        .rounded(px(8.0))
        .text_size(px(12.5))
        .font_weight(FontWeight::SEMIBOLD)
        .whitespace_nowrap()
        .cursor_pointer()
        .when_else(
            selected,
            |s| s.bg(pal.raised()).text_color(pal.text).when(!pal.dark, gpui::Styled::shadow_xs),
            |s| s.text_color(pal.secondary).hover(|h| h.text_color(pal.text)),
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
        .child(div().size(px(4.0)).rounded_full().bg(pal.tertiary))
        .child(div().size(px(4.0)).rounded_full().bg(pal.tertiary))
        .child(div().size(px(4.0)).rounded_full().bg(color))
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
        .bg(pal.tile())
        .border_1()
        .border_color(pal.tile_border())
        .child(m.render(side * 0.5, pal.text, pal))
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
                    .bg(pal.tile().blend(color.opacity(0.28)))
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

/// A sentence with its important words in bold and the text colour; the rest in the secondary colour.
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
                    color: Some(pal.text),
                    font_weight: Some(FontWeight::SEMIBOLD),
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
        .rounded(px(12.0))
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
        .child(div().flex_1().text_color(pal.tertiary).child(label.to_owned()))
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
        .rounded(px(5.0))
        .bg(pal.control_fill)
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
                pal.accent
            } else {
                pal.tile_border()
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
                |c| c.bg(pal.accent),
                |c| {
                    c.border(px(1.5)).when_else(
                        here,
                        |c| c.border_color(pal.accent).bg(pal.accent.opacity(0.15)).text_color(pal.accent),
                        |c| c.border_color(pal.tile_border().blend(pal.tertiary.opacity(0.4))).text_color(pal.tertiary),
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

/// The sentence a screen of the welcome flow is about, centred, its important words bold; and a (?).
pub fn welcome_title(words: &[Words], help: Option<Stateful<Div>>, pal: Palette) -> Div {
    div()
        .flex()
        .items_center()
        .justify_center()
        .gap(px(8.0))
        .child(
            div()
                .text_size(px(24.0))
                .line_height(px(31.0))
                .font_weight(FontWeight::MEDIUM)
                .text_color(pal.secondary)
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
        .child(tile(i, pal.accent, 40.0))
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
    div().flex_none().font_family(MONO).text_size(px(12.0)).text_color(pal.text).whitespace_nowrap().child(text.into())
}

/// A tile on Overview: a small capitals label with an icon, a value, and a line or an action under it.
pub fn stat(i: Icon, color: Hsla, label: &str, pal: Palette) -> Div {
    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_w(px(0.0))
        .gap(px(3.0))
        .p(px(14.0))
        .rounded(px(16.0))
        .bg(pal.elevated)
        .border_1()
        .border_color(pal.hairline)
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(6.0))
                .text_size(px(11.0))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(pal.tertiary)
                .child(icon(i, 13.0, color))
                .child(label.to_uppercase()),
        )
}

/// A popover's frame.
pub fn popover(pal: Palette) -> Div {
    div()
        .occlude()
        .flex()
        .flex_col()
        .p(px(14.0))
        .rounded(px(16.0))
        .bg(pal.popover())
        .border_1()
        .border_color(pal.popover_border())
        .shadow_lg()
}
