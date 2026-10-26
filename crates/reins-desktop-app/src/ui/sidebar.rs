//! The status window's sidebar (`Desk-Home.png`, `Desk-Waiting.png`, `Desk-Extras.png`): the mark and the serif
//! "Reins", a card that says what Reins is doing (paper while all is well; a field when something needs you: the
//! waiting request's colour, red while git is paused, amber when broken, violet while a work session runs), the
//! sections in monochrome glyphs with their keys (the selected one a raised pill that slides between them), and at the
//! bottom "Pause git", or, while paused, the red card with the time left and Resume. And the "Pause git" sheet: 15 min,
//! 1 hour, 1 day, or until you resume (held to confirm).

use gpui::prelude::FluentBuilder as _;
use gpui::{
    Animation, AnimationExt as _, AnyElement, Context, FontWeight, InteractiveElement as _, IntoElement,
    ParentElement as _, SharedString, StatefulInteractiveElement as _, Styled as _, div, px,
};

use super::hold::Held;
use super::icons::{Brand, Glyph, brand, glyph};
use super::parts::ring;
use super::story::field_of;
use super::{Data, Root, kit};
use crate::format;
use crate::model::{Model, Section};
use crate::pause::PauseFor;
use crate::shortcuts;
use crate::theme::motion::{self, Entrance, curve, enter};
use crate::theme::{Field, MONO, Palette, DISPLAY, NextText as _};
use crate::tray::Look;

const WIDTH: f32 = 236.0;
/// A section's row, and the distance from one row to the next.
const ROW: f32 = 38.0;
const PITCH: f32 = 40.0;

/// The lengths the sheet offers.
const LENGTHS: [PauseFor; 4] = [PauseFor::Quarter, PauseFor::Hour, PauseFor::Day, PauseFor::Manual];

impl Section {
    /// The section's glyph (in ink: Signal colours only what needs a decision).
    #[must_use]
    pub fn glyph(self) -> Glyph {
        match self {
            Self::Home => Glyph::Home,
            Self::Activity => Glyph::Activity,
            Self::Connections => Glyph::Link,
            Self::Keys => Glyph::Key,
            Self::Rules => Glyph::Shield,
            Self::Settings => Glyph::Settings,
        }
    }
}

/// Where the selected section's pill is, and where it slid from.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NavPill {
    from: usize,
    at: usize,
    /// How many times it moved: each move is its own slide.
    moves: u64,
}

impl NavPill {
    /// The pill goes to row `to`; true when it moved.
    pub fn go(&mut self, to: usize) -> bool {
        if to == self.at {
            return false;
        }
        self.from = self.at;
        self.at = to;
        self.moves += 1;
        true
    }
}

/// What the status card says: a title, a line (and a clock after it, in mono), a glyph, and the field it is (`None`:
/// paper, all is well).
#[derive(Clone, Debug, PartialEq)]
struct Card {
    title: String,
    line: String,
    clock: Option<String>,
    glyph: Glyph,
    field: Option<Field>,
}

/// The status card for `d`: a request waiting first, then a pause, something broken, a work session, or all well.
fn card_of(d: &Data) -> Card {
    let card = |title: &str, line: String, glyph: Glyph, field: Option<Field>| Card {
        title: title.to_owned(),
        line,
        clock: None,
        glyph,
        field,
    };
    match d.look {
        Look::Waiting => {
            let oldest = d.waiting().last();
            let left = oldest.map(|e| d.wait_left(e).1).unwrap_or_default();
            Card {
                clock: Some(clock_left(left)),
                ..card(
                    "Waiting on your phone",
                    format::count(d.waiting().count() as u64, "request", "requests"),
                    Glyph::Phone,
                    Some(oldest.map_or(Field::Push, field_of)),
                )
            }
        }
        Look::Paused if d.pause.on() => {
            card("Pushes go direct", "Hooks and MCP still ask".to_owned(), Glyph::Pause, Some(Field::Delete))
        }
        Look::Paused => {
            card("git goes direct", "Turn hosts on in Connections".to_owned(), Glyph::Warning, Some(Field::Warn))
        }
        Look::Attention if !d.paired => {
            card("Not paired", "Pair with your phone".to_owned(), Glyph::Unlink, Some(Field::Warn))
        }
        Look::Attention => Card {
            title: d.line.clone(),
            ..card("", "Start it in Home".to_owned(), Glyph::Warning, Some(Field::Warn))
        },
        Look::On => match d.work_session() {
            Some(s) => card(
                "Work session",
                format!("{} left · {}", crate::work::left(s, d.now), s.reason),
                Glyph::Timer,
                Some(Field::Grant),
            ),
            None => card("Reins is on", format!("{} approves", d.phone()), Glyph::ShieldCheck, None),
        },
    }
}

/// Seconds left as a clock: "1:08".
#[must_use]
pub fn clock_left(secs: i64) -> String {
    let secs = secs.max(0);
    format!("{}:{:02}", secs / 60, secs % 60)
}

impl Root {
    pub(super) fn sidebar(&mut self, d: &Data, pal: Palette, cx: &mut Context<'_, Self>) -> AnyElement {
        let waiting = d.waiting().count();
        let badge_field = d.waiting().last().map_or(Field::Push, field_of);
        let selected_row = if d.gallery {
            Section::ALL.len()
        } else {
            Section::ALL.iter().position(|s| *s == d.section).unwrap_or(0)
        };
        self.nav.go(selected_row);

        let entry = |id: SharedString, g: Glyph, label: &'static str, selected: bool, end: AnyElement| {
            div()
                .id(id)
                .relative()
                .flex()
                .items_center()
                .gap(px(10.0))
                .h(px(ROW))
                .px(px(10.0))
                .rounded(px(12.0))
                .cursor_pointer()
                .text_size(px(14.0))
                .active(|s| s.opacity(kit::button::PRESSED))
                .when_else(
                    selected,
                    |r| r.text_color(pal.ink()).font_weight(FontWeight::SEMIBOLD),
                    |r| {
                        r.text_color(pal.ink_2()).hover(|s| s.bg(pal.paper_sunken().opacity(0.7)).text_color(pal.ink()))
                    },
                )
                .child(glyph(
                    g,
                    18.0,
                    if selected {
                        pal.ink()
                    } else {
                        pal.ink_2()
                    },
                ))
                .child(div().flex_1().min_w(px(0.0)).truncate().child(label))
                .child(end)
        };

        // The pill under the selected row: it slides from the row before (a cross-fade under Reduce Motion).
        let pill = div()
            .absolute()
            .left_0()
            .right_0()
            .h(px(ROW))
            .rounded(px(12.0))
            .bg(pal.paper_raised())
            .border_1()
            .border_color(pal.rule());
        #[allow(clippy::cast_precision_loss, reason = "a handful of rows")]
        let (from, to) = (self.nav.from as f32 * PITCH, self.nav.at as f32 * PITCH);
        let pill: AnyElement = if self.nav.moves == 0 {
            pill.top(px(to)).into_any_element()
        } else if motion::reduce_motion() {
            pill.top(px(to))
                .with_animation(("nav-pill", self.nav.moves), Animation::new(motion::FADE), gpui::Styled::opacity)
                .into_any_element()
        } else {
            pill.with_animation(
                ("nav-pill", self.nav.moves),
                Animation::new(motion::TAB_PILL).with_easing(curve(motion::EASE_OUT)),
                move |el, t| el.top(px((to - from).mul_add(t, from))),
            )
            .into_any_element()
        };

        let mut nav = div().relative().flex().flex_col().gap(px(PITCH - ROW)).child(pill);
        for (i, section) in Section::ALL.into_iter().enumerate() {
            let selected = i == selected_row;
            let end = if section == Section::Activity && waiting > 0 {
                div()
                    .flex()
                    .items_center()
                    .justify_center()
                    .min_w(px(20.0))
                    .h(px(18.0))
                    .px(px(6.0))
                    .rounded_full()
                    .bg(badge_field.field())
                    .text_color(pal.on_field())
                    .text_size(px(11.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(waiting.to_string())
                    .into_any_element()
            } else {
                kit::kbd(shortcuts::hint(&(i + 1).to_string()), pal).into_any_element()
            };
            nav = nav.child(
                entry(
                    SharedString::from(format!("nav-{}", section.id())),
                    section.glyph(),
                    section.label(),
                    selected,
                    end,
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.gallery = false;
                    this.model.update(cx, |m, cx| m.show_section(section, cx));
                    cx.notify();
                })),
            );
        }
        if d.demo {
            // The component gallery, in the demo only.
            nav = nav.child(
                entry("nav-gallery".into(), Glyph::Package, "Gallery", d.gallery, div().into_any_element())
                    .on_click(Self::on_view(cx, |this, _| this.gallery = true)),
            );
        }

        div()
            .flex()
            .flex_col()
            .flex_none()
            .gap(px(16.0))
            .w(px(WIDTH))
            .h_full()
            .px(px(14.0))
            .pb(px(18.0))
            .pt(px(if cfg!(target_os = "macos") {
                46.0
            } else {
                18.0
            }))
            .bg(pal.side())
            .border_r_1()
            .border_color(pal.rule())
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.0))
                    .px(px(6.0))
                    .pt(px(4.0))
                    .child(brand(Brand::Reins, 26.0).rounded(px(7.0)))
                    .child(div().font_family(DISPLAY).text_size(px(24.0)).line_height(px(28.0)).child("Reins")),
            )
            .child(Self::status_card(d, pal, cx))
            .child(nav)
            .child(div().flex_1())
            .child(
                div().flex().flex_col().gap(px(10.0)).child(Self::pause_box(d, pal, cx)).child(
                    div()
                        .px(px(6.0))
                        .text_size(px(11.5))
                        .text_color(pal.ink_meta())
                        .child(format!("Reins {}", reins_desktop::update::VERSION)),
                ),
            )
            .into_any_element()
    }

    /// What Reins is doing: on paper while all is well, a field when something needs you. A click opens Home.
    fn status_card(d: &Data, pal: Palette, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let c = card_of(d);
        let (title_color, line_color) = match c.field {
            Some(_) => (pal.on_field(), pal.on_field_2()),
            None => (pal.ink(), pal.ink_meta()),
        };
        let tile = match c.field {
            Some(_) => div()
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .size(px(32.0))
                .rounded(px(10.0))
                .bg(pal.on_field_wash())
                .child(glyph(c.glyph, 18.0, pal.on_field())),
            None => kit::kind_tile(c.glyph, crate::theme::Tone::Success, 32.0, pal),
        };
        let words = div()
            .relative()
            .flex()
            .flex_col()
            .flex_1()
            .min_w(px(0.0))
            .child(
                div()
                    .text_size(px(13.5))
                    .line_height(px(17.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(title_color)
                    .line_clamp(2)
                    .child(c.title),
            )
            .child(
                div()
                    .flex()
                    .min_w(px(0.0))
                    .text_size(px(12.0))
                    .line_height(px(16.0))
                    .text_color(line_color)
                    .child(div().min_w(px(0.0)).truncate().child(c.line))
                    .when_some(c.clock, |l, clock| l.child(" · ").child(div().font_family(MONO).child(clock))),
            );
        let frame = match c.field {
            Some(f) => kit::field_surface(f, 16.0, 16.0),
            None => div().bg(pal.paper_raised()).rounded(px(16.0)).border_1().border_color(pal.rule()),
        };
        frame
            .id("status-card")
            .flex()
            .items_center()
            .gap(px(10.0))
            .p(px(12.0))
            .cursor_pointer()
            .active(|s| s.opacity(kit::button::PRESSED))
            .child(div().relative().child(tile))
            .child(words)
            .on_click(Self::go(cx, Section::Home))
    }

    /// The bottom of the sidebar: "Pause git"; while paused, the desktop's bypass: a red field with the time left
    /// and Resume.
    fn pause_box(d: &Data, pal: Palette, cx: &mut Context<'_, Self>) -> AnyElement {
        if let Some(left) = d.pause.short(d.now) {
            let on = pal.on_field();
            let mark: AnyElement = match d.saved.pause_left(d.now) {
                Some(f) => ring(f, on, on.opacity(0.3), 22.0, 2.4).into_any_element(),
                None => div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .size(px(22.0))
                    .rounded_full()
                    .border_2()
                    .border_color(on)
                    .child(glyph(Glyph::Infinity, 12.0, on))
                    .into_any_element(),
            };
            return kit::field_surface(Field::Delete, 18.0, 18.0)
                .flex()
                .flex_col()
                .gap(px(10.0))
                .p(px(14.0))
                .text_color(on)
                .child(
                    div().relative().flex().items_center().gap(px(10.0)).child(mark).child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_w(px(0.0))
                            .child(
                                div()
                                    .text_size(px(14.0))
                                    .line_height(px(18.0))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child("git paused"),
                            )
                            .child(
                                div()
                                    .text_size(px(12.0))
                                    .line_height(px(16.0))
                                    .text_color(pal.on_field_2())
                                    .truncate()
                                    .child(left),
                            ),
                    ),
                )
                .child(
                    kit::button(
                        "side-resume",
                        kit::Look::OnField,
                        kit::Size::Regular,
                        Some(Glyph::Play),
                        "Resume",
                        pal,
                    )
                    .h(px(34.0))
                    .w_full()
                    .on_click(Self::on_model(cx, Model::resume)),
                )
                .into_any_element();
        }
        let b = kit::button("side-pause", kit::Look::Soft, kit::Size::Regular, Some(Glyph::Pause), "Pause git", pal)
            .w_full();
        if d.paired {
            b.on_click(Self::on_view(cx, |this, _| {
                this.pause_sheet = !this.pause_sheet;
            }))
            .into_any_element()
        } else {
            kit::disabled(b).into_any_element()
        }
    }

    /// The "Pause git" sheet, rising over a dimmed window: the lengths (the last, until you resume, in red and held to
    /// confirm), then Cancel and the button that pauses.
    pub(super) fn pause_sheet(&self, pal: Palette, cx: &mut Context<'_, Self>) -> AnyElement {
        let choice = self.pause_choice;
        let mut lengths = kit::segmented(pal).w_full();
        for length in LENGTHS {
            let on = choice == length;
            let label = match length {
                PauseFor::Day => "1 day",
                PauseFor::Manual => "Until resume",
                other => other.short(),
            };
            let mut s = kit::segment(SharedString::from(format!("sheet-{}", length.id())), "", on, pal)
                .h(px(32.0))
                .px(px(10.0));
            if length == PauseFor::Manual {
                s = s.text_color(pal.danger).child(glyph(Glyph::Infinity, 14.0, pal.danger));
            }
            lengths = lengths.child(s.child(label).on_click(Self::on_view(cx, move |this, _| {
                this.pause_choice = length;
                this.hold = None;
            })));
        }
        let confirm: AnyElement = if choice == PauseFor::Manual {
            self.hold_button("sheet-hold", Held::PauseUntilResume, Glyph::Pause, "Hold to pause", "Paused", pal, cx)
                .w_full()
                .into_any_element()
        } else {
            kit::button(
                "sheet-pause",
                kit::Look::Inverse,
                kit::Size::Regular,
                Some(Glyph::Pause),
                format!("Pause for {}", choice_words(choice)),
                pal,
            )
            .w_full()
            .on_click(cx.listener(move |this, _, _, cx| {
                this.pause_sheet = false;
                this.model.update(cx, |m, cx| m.pause_for(choice, cx));
            }))
            .into_any_element()
        };
        let sheet = div()
            .occlude()
            .relative()
            .flex()
            .flex_col()
            .gap(px(12.0))
            .w(px(440.0))
            .p(px(24.0))
            .rounded(px(24.0))
            .bg(pal.paper_raised())
            .border_1()
            .border_color(pal.rule())
            .shadow_lg()
            .child(div().sheet_title().text_color(pal.ink()).child(kit::ask_line("Pause ", "git")))
            .child(
                div()
                    .text_size(px(14.0))
                    .line_height(px(20.0))
                    .text_color(pal.ink_2())
                    .child("Pushes go straight to the hosts without asking. Hooks and MCP tools still ask your phone."),
            )
            .child(lengths)
            .child(
                div()
                    .flex()
                    .gap(px(8.0))
                    .mt(px(4.0))
                    // Cancel and the action, 1 : 1.4.
                    .child(
                        div().flex().flex_none().w(gpui::relative(0.41)).child(
                            kit::button("sheet-cancel", kit::Look::Outline, kit::Size::Regular, None, "Cancel", pal)
                                .w_full()
                                .on_click(Self::on_view(cx, |this, _| {
                                    this.pause_sheet = false;
                                    this.hold = None;
                                })),
                        ),
                    )
                    .child(div().flex().flex_1().min_w(px(0.0)).child(confirm)),
            );
        div()
            .absolute()
            .inset_0()
            .child(div().id("sheet-scrim").absolute().inset_0().bg(pal.scrim()).on_click(Self::on_view(
                cx,
                |this, _| {
                    this.pause_sheet = false;
                    this.hold = None;
                },
            )))
            .child(div().absolute().left(px(20.0)).bottom(px(64.0)).child(enter(sheet, "pause-sheet", Entrance::Sheet)))
            .into_any_element()
    }
}

/// "15 min", "1 hour", "1 day".
fn choice_words(length: PauseFor) -> &'static str {
    match length {
        PauseFor::Quarter => "15 min",
        PauseFor::Hour => "1 hour",
        PauseFor::FourHours => "4 hours",
        PauseFor::Day => "1 day",
        PauseFor::Manual => "good",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clocks_count_down() {
        assert_eq!(clock_left(68), "1:08");
        assert_eq!(clock_left(0), "0:00");
        assert_eq!(clock_left(-5), "0:00");
        assert_eq!(clock_left(600), "10:00");
    }

    #[test]
    fn every_section_has_its_own_glyph() {
        let glyphs: std::collections::HashSet<_> = Section::ALL.iter().map(|s| s.glyph()).collect();
        assert_eq!(glyphs.len(), Section::ALL.len());
    }

    #[test]
    fn the_sheet_offers_until_resume_last() {
        assert_eq!(LENGTHS.last(), Some(&PauseFor::Manual));
        assert!(LENGTHS.iter().all(|l| PauseFor::ALL.contains(l)));
    }

    /// The pill slides only when the selection moves, each move its own slide from where it was.
    #[test]
    fn the_pill_slides_from_where_it_was() {
        let mut pill = NavPill::default();
        assert!(!pill.go(0));
        assert!(pill.go(3));
        assert_eq!((pill.from, pill.at, pill.moves), (0, 3, 1));
        assert!(!pill.go(3));
        assert!(pill.go(1));
        assert_eq!((pill.from, pill.at, pill.moves), (3, 1, 2));
    }
}
