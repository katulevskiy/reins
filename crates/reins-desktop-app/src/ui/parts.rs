//! Building blocks the screens share: tiles, the (?) and its bubble, links, rings that count down, the sentence with
//! its target at 600, a page's title, a request's facts as a pane of rows, mono text that wraps where it may, the
//! welcome flow's steps. Icons are the Reins glyphs, logos the brands' own marks ([`super::icons`]).

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, Context, Div, ElementId, FontWeight, HighlightStyle, Hsla, InteractiveElement as _, IntoElement,
    ParentElement, PathBuilder, SharedString, Stateful, StatefulInteractiveElement as _, Styled as _, StyledText,
    anchored, canvas, deferred, div, point, px,
};

use super::icons::{Glyph, Mark, avatar, glyph};
use super::story::{FactStyle, Words};
use super::{Data, Root};
use crate::theme::{MONO, NextText as _, Palette, motion};
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

/// [`help`] where the window's [`Data`] is not at hand (a row a list draws): `open` says whether its bubble shows.
pub fn help_open(
    id: &'static str,
    text: impl Into<SharedString>,
    open: bool,
    pal: Palette,
    cx: &Context<'_, Root>,
) -> Stateful<Div> {
    let text = text.into();
    #[allow(clippy::cast_precision_loss, reason = "a few hundred characters")]
    let width = (text.len() as f32).mul_add(6.3, 28.0).clamp(150.0, 320.0);
    help_bubble(id, move || text.into_any_element(), width, open, pal, cx)
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
    help_bubble(id, content, width, d.help == Some(id), pal, cx)
}

/// The (?) and, while `open`, its bubble.
fn help_bubble(
    id: &'static str,
    content: impl FnOnce() -> AnyElement,
    width: f32,
    open: bool,
    pal: Palette,
    cx: &Context<'_, Root>,
) -> Stateful<Div> {
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
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(pal.ink_2())
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

/// A sentence with its important words (the target) at 600 and in `ink` (the boards' `em`); the rest in the caller's
/// colour and weight.
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
                    font_weight: Some(FontWeight::SEMIBOLD),
                    ..HighlightStyle::default()
                },
            ));
        }
    }
    StyledText::new(text).with_highlights(highlights)
}

/// Words with the important ones at 600 and nothing else changed (the colour is the caller's: white on a field).
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
                    font_weight: Some(FontWeight::SEMIBOLD),
                    ..HighlightStyle::default()
                },
            ));
        }
    }
    StyledText::new(text).with_highlights(highlights)
}

/// The line under a page's title (`.dsub`): 14.5 in `ink-2`.
pub fn mood(text: impl Into<SharedString>, pal: Palette) -> Div {
    div().mt(px(6.0)).mood().text_color(pal.ink_2()).child(text.into())
}

/// A page's title (`words`, the important ones at 600) and the line under it; `right` beside them, at the middle (a
/// button).
pub fn page_title(words: &[Words], line: Option<&str>, right: Option<AnyElement>, pal: Palette) -> Div {
    div()
        .flex()
        .flex_wrap()
        .items_center()
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

/// Where a request's facts are shown, and so how: the rows' height and text, the hairlines.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FactLook {
    /// Home, under "What will happen" beside the latest requests: rows at least 48 px tall, a hairline between rows.
    Home,
    /// A request's page in Activity: a hairline under every row.
    Detail,
}

/// The facts' key column: as wide as the longest key ("Answered in", "Repository") and no wider.
const FACT_KEY_WIDTH: f32 = 96.0;

/// The narrowest a fact's value gets beside its key; with less room left, the value goes under the key, the row's
/// whole width.
const FACT_VALUE_MIN: f32 = 160.0;

/// The facts of a request as a pane of rows (`.kv`): the key (13.5, 500, `ink-meta`) in a column of its own width, the
/// value (15.5, 500, `ink`; mono 14 for the machine's) taking the rest of the row, or the whole row under the key when
/// the row is narrow.
pub fn fact_rows(rows: &[super::story::Fact], look: FactLook, glass: super::kit::Glass, pal: Palette) -> AnyElement {
    let _ = look;
    let mut out = Vec::new();
    for f in rows {
        let key = div()
            .flex_none()
            .w(px(FACT_KEY_WIDTH))
            .text_size(px(13.5))
            .font_weight(FontWeight::MEDIUM)
            .text_color(pal.ink_meta())
            .child(f.key.clone());
        let value = div().flex_1().min_w(px(FACT_VALUE_MIN)).text_color(pal.ink()).font_weight(FontWeight::MEDIUM);
        let value = match f.style {
            // Each part may wrap inside the row (`min_w(0)`) rather than push past it.
            FactStyle::Text => value.text_size(px(15.5)).child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_baseline()
                    .gap_x(px(6.0))
                    .child(div().min_w(px(0.0)).child(f.value.clone()))
                    .when_some(f.tail.clone(), |v, t| {
                        v.child(
                            div()
                                .min_w(px(0.0))
                                .font_family(MONO)
                                .text_size(px(14.0))
                                .text_color(pal.ink_2())
                                .child(wrap_points(&format!("· {t}"))),
                        )
                    }),
            ),
            // The machine's, whole: long values wrap after a `/`, a `-`, a `=`..., never cut.
            FactStyle::Mono => {
                value.font_family(MONO).text_size(px(14.0)).line_height(px(20.0)).child(wrapped(&f.value))
            }
            FactStyle::Changes => value.text_size(px(15.5)).child(changes(&f.value, pal)),
        };
        out.push(
            div()
                .flex()
                .flex_wrap()
                .items_baseline()
                .gap_x(px(12.0))
                .gap_y(px(2.0))
                .min_h(px(50.0))
                .px(px(16.0))
                .py(px(14.0))
                .child(key)
                .child(value)
                .into_any_element(),
        );
    }
    super::kit::group(out, 16.0, glass, pal)
}

/// Mono text that may be long (a command, a path): each line its own block, wrapped whole where the line may break
/// ([`wrap_points`]), at any character only inside a run too long for the width.
pub fn wrapped(text: &str) -> Div {
    let mut block = div().flex().flex_col().min_w(px(0.0));
    for line in text.lines() {
        block = block.child(div().min_w(px(0.0)).child(wrap_points(line)));
    }
    block
}

/// Characters a path, a command or a URL may break after: `/home/dana/` `code/`, `acme-` `web`, `--branch=` `main`,
/// `android.` `service.` `Tile`.
const BREAK_AFTER: &[char] = &['/', '\\', '-', '_', '.', '=', ',', ';', ':', '&', '|', '?'];

/// `line` with a place to break after each separator ([`BREAK_AFTER`]) that ends a run of them and has more text after
/// it: `https://` stays together, and a `-`, `_` or `.` breaks only between letters or digits (`acme-web`, never
/// `--force` or `./`). GPUI's wrapper breaks before a non-word character that follows another one, so two zero-width
/// spaces make a break that shows nothing; a line with nowhere to break still breaks at the character that does not
/// fit.
pub fn wrap_points(line: &str) -> String {
    let chars: Vec<char> = line.chars().collect();
    let mut out = String::with_capacity(line.len() + 8);
    for (i, &c) in chars.iter().enumerate() {
        out.push(c);
        let next = chars.get(i + 1).copied();
        let before = i.checked_sub(1).map(|p| chars[p]);
        let more = next.is_some_and(|n| !n.is_whitespace() && !BREAK_AFTER.contains(&n));
        let between_words = !matches!(c, '-' | '_' | '.')
            || (before.is_some_and(char::is_alphanumeric) && next.is_some_and(char::is_alphanumeric));
        if BREAK_AFTER.contains(&c) && more && between_words {
            out.push_str("\u{200B}\u{200B}");
        }
    }
    out
}

/// `+128 −31 · 9 files`: the additions green and the removals red in mono, the rest in `ink-2`.
pub fn changes(text: &str, pal: Palette) -> Div {
    let mut line = div().flex().flex_wrap().items_baseline().gap_x(px(6.0));
    for part in text.split_whitespace() {
        let (color, mono) = if part.starts_with('+') {
            (pal.success, true)
        } else if part.starts_with(['−', '-']) && part.len() > 1 {
            (pal.danger, true)
        } else {
            (pal.ink_2(), false)
        };
        let word = div().text_color(color).child(part.to_owned());
        line = line.child(if mono {
            word.font_family(MONO).text_size(px(14.0))
        } else {
            word
        });
    }
    line
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

/// The welcome flow's steps (`.dstep` in `DeskNext-Setup.dc.html`): "1 Pair — 2 AI tools — 3 Turn on — 4 Done".
/// Behind: a green dot with a check; here: an ink dot with its number; ahead: a hairline ring.
pub fn step_indicator(current: Stage, pal: Palette) -> Div {
    let mut bar = div().flex().items_center().justify_center().gap(px(8.0));
    for (i, stage) in Stage::ALL.into_iter().enumerate() {
        let behind = stage < current;
        let here = stage == current;
        if i > 0 {
            bar = bar.child(div().flex_none().w(px(60.0)).h(px(1.0)).mx(px(4.0)).bg(pal.rule_2()));
        }
        let number = stage.number().unwrap_or(i + 1).to_string();
        let circle = div()
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .size(px(24.0))
            .rounded_full()
            .text_size(px(12.5))
            .font_weight(FontWeight::SEMIBOLD);
        let circle = if behind {
            circle.bg(pal.success).child(glyph(Glyph::Check, 13.0, gpui::white()))
        } else if here {
            circle.bg(pal.ink()).text_color(pal.paper()).child(number)
        } else {
            circle.border(px(1.5)).border_color(pal.rule_2()).text_color(pal.ink_meta()).child(number)
        };
        bar = bar.child(
            div().flex().flex_none().items_center().gap(px(8.0)).child(circle).child(
                div()
                    .text_size(px(13.5))
                    .font_weight(FontWeight::MEDIUM)
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

/// The sentence a screen of the welcome flow is about (`--s: 40`), centred, its target at 600; and a (?).
pub fn welcome_title(words: &[Words], help: Option<Stateful<Div>>, pal: Palette) -> Div {
    div()
        .flex()
        .items_center()
        .justify_center()
        .gap(px(8.0))
        .mt(px(6.0))
        .child(div().step_title().text_color(pal.ink()).text_center().child(italic(words)))
        .when_some(help, ParentElement::child)
}

#[cfg(test)]
mod tests {
    use super::wrap_points;

    /// Where `line` may break, as `|`.
    fn breaks(line: &str) -> String {
        let marked = wrap_points(line).replace("\u{200B}\u{200B}", "|");
        assert!(!marked.contains('\u{200B}'), "{marked:?}");
        marked
    }

    #[test]
    fn paths_and_commands_break_after_their_separators() {
        assert_eq!(breaks("/home/dana/code/acme-web"), "/|home/|dana/|code/|acme-|web");
        assert_eq!(breaks("git push --force origin main"), "git push --force origin main");
        assert_eq!(breaks("rm -rf ./build_out"), "rm -rf ./|build_|out");
        assert_eq!(breaks("git push origin HEAD:refs/heads/main"), "git push origin HEAD:|refs/|heads/|main");
        assert_eq!(breaks("curl https://example.com/a?b=1&c=2"), "curl https://|example.|com/|a?|b=|1&|c=|2");
        assert_eq!(breaks(r"C:\Users\dana"), r"C:\|Users\|dana");
        assert_eq!(breaks("javap android.service.Tile"), "javap android.|service.|Tile");
    }

    #[test]
    fn nothing_to_break_after_is_left_as_it_is() {
        for line in ["", "-", "/", "a3f9c2e1b7d4", "trailing/", "dash-", "x --", "a - b", "cd ..", "end."] {
            assert_eq!(breaks(line), line);
        }
    }
}
