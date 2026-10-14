//! The status window's sidebar: the mark, a card that says what Reins is doing (green on, violet waiting, yellow
//! paused, red when it needs you), the sections with their coloured icon squares, and pausing git at the bottom; and
//! the "Pause git" sheet (15 min, 1 hour, 1 day, or until you resume, held to confirm).

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, Context, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _, SharedString,
    StatefulInteractiveElement as _, Styled as _, div, px,
};

use super::hold::Held;
use super::icons::{Icon, icon};
use super::parts::{Btn, btn, btn_icon, help, look_color, popover, ring, segment_frame, segmented, solid_tile, tile};
use super::{Data, Root};
use crate::format;
use crate::model::{Model, Section};
use crate::pause::PauseFor;
use crate::shortcuts;
use crate::theme::{Palette, Tone};
use crate::tray::Look;

const WIDTH: f32 = 232.0;

/// The lengths the sheet offers.
const LENGTHS: [PauseFor; 4] = [PauseFor::Quarter, PauseFor::Hour, PauseFor::Day, PauseFor::Manual];

impl Section {
    /// The section's icon and colour, as phone Settings' rows have.
    #[must_use]
    pub fn icon(self) -> (Icon, Tone) {
        match self {
            Self::Overview => (Icon::LayoutDashboard, Tone::Violet),
            Self::Activity => (Icon::Activity, Tone::Blue),
            Self::Connections => (Icon::Cable, Tone::Teal),
            Self::Keys => (Icon::KeyRound, Tone::Orange),
            Self::Rules => (Icon::ShieldCheck, Tone::Pink),
            Self::Settings => (Icon::Settings, Tone::Gray),
        }
    }
}

/// The status card's title and line.
fn status_words(d: &Data) -> (String, String, Icon) {
    let waiting = d.waiting().count();
    match d.look {
        Look::Waiting => {
            let oldest = d.waiting().last().map(|e| d.wait_left(e).1).unwrap_or_default();
            let n = format::count(waiting as u64, "request", "requests");
            ("Waiting on your phone".to_owned(), format!("{n} · {}", clock_left(oldest)), Icon::Smartphone)
        }
        Look::Paused => {
            let left = d.pause.short(d.now).unwrap_or_else(|| "git goes direct".to_owned());
            ("git paused".to_owned(), left, Icon::Pause)
        }
        Look::Attention if !d.paired => ("Not paired".to_owned(), "Pair with your phone".to_owned(), Icon::Unlink),
        Look::Attention => (d.line.clone(), "Start it in Overview".to_owned(), Icon::TriangleAlert),
        Look::On => ("Reins is on".to_owned(), format!("{} approves", d.phone()), Icon::ShieldCheck),
    }
}

/// Seconds left as a clock: "1:08".
#[must_use]
pub fn clock_left(secs: i64) -> String {
    let secs = secs.max(0);
    format!("{}:{:02}", secs / 60, secs % 60)
}

impl Root {
    pub(super) fn sidebar(d: &Data, pal: Palette, cx: &mut Context<'_, Self>) -> AnyElement {
        let waiting = d.waiting().count();
        let mut nav = div().flex().flex_col().gap(px(2.0));
        for (i, section) in Section::ALL.into_iter().enumerate() {
            let selected = d.section == section;
            let (ic, tone) = section.icon();
            let badge = (section == Section::Activity && waiting > 0).then_some(waiting);
            let square = if selected {
                solid_tile(ic, tone, pal, 24.0)
            } else {
                tile(ic, pal.tone(tone), 24.0)
            };
            nav = nav.child(
                div()
                    .id(SharedString::from(format!("nav-{}", section.id())))
                    .flex()
                    .items_center()
                    .gap(px(10.0))
                    .h(px(36.0))
                    .px(px(8.0))
                    .rounded(px(9.0))
                    .cursor_pointer()
                    .text_size(px(13.5))
                    .when_else(
                        selected,
                        |r| {
                            r.bg(pal.selected)
                                .border_1()
                                .border_color(if pal.dark {
                                    pal.tile_border().opacity(0.7)
                                } else {
                                    pal.hairline
                                })
                                .when(!pal.dark, gpui::Styled::shadow_xs)
                                .text_color(pal.text)
                                .font_weight(FontWeight::MEDIUM)
                        },
                        |r| r.text_color(pal.secondary).hover(|s| s.bg(pal.control_fill).text_color(pal.text)),
                    )
                    .child(square)
                    .child(div().flex_1().min_w(px(0.0)).truncate().child(section.label()))
                    .child(match badge {
                        Some(n) => div()
                            .flex()
                            .items_center()
                            .justify_center()
                            .min_w(px(20.0))
                            .h(px(18.0))
                            .px(px(6.0))
                            .rounded_full()
                            .bg(pal.accent)
                            .text_color(pal.on_accent)
                            .text_size(px(11.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(n.to_string()),
                        None => div()
                            .text_size(px(11.0))
                            .text_color(pal.tertiary.opacity(0.7))
                            .child(shortcuts::hint(&(i + 1).to_string())),
                    })
                    .on_click(Self::go(cx, section)),
            );
        }

        div()
            .flex()
            .flex_col()
            .flex_none()
            .w(px(WIDTH))
            .h_full()
            .px(px(12.0))
            .pb(px(12.0))
            .pt(px(if cfg!(target_os = "macos") {
                46.0
            } else {
                18.0
            }))
            .bg(pal.sidebar)
            .border_r_1()
            .border_color(pal.hairline)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(9.0))
                    .px(px(8.0))
                    .pb(px(12.0))
                    .child(super::parts::mark(26.0))
                    .child(div().text_size(px(15.0)).font_weight(FontWeight::SEMIBOLD).child("Reins")),
            )
            .child(Self::status_card(d, pal, cx))
            .child(nav)
            .child(div().flex_1())
            .child(Self::pause_box(d, pal, cx))
            .child(
                div()
                    .px(px(8.0))
                    .pt(px(10.0))
                    .text_size(px(11.0))
                    .text_color(pal.tertiary.opacity(0.75))
                    .child(format!("Reins {}", reins_desktop::update::VERSION)),
            )
            .into_any_element()
    }

    /// What Reins is doing, tinted by it; a click opens Overview, where the rest is.
    fn status_card(d: &Data, pal: Palette, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let color = look_color(d.look, pal);
        let (title, line, ic) = status_words(d);
        div()
            .id("status-card")
            .flex()
            .items_center()
            .gap(px(10.0))
            .mb(px(14.0))
            .px(px(12.0))
            .py(px(11.0))
            .rounded(px(14.0))
            .bg(color.opacity(0.08))
            .border_1()
            .border_color(color.opacity(0.22))
            .cursor_pointer()
            .hover(|s| s.bg(color.opacity(0.12)))
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .size(px(30.0))
                    .rounded(px(9.0))
                    .bg(color.opacity(0.16))
                    .child(icon(ic, 17.0, color)),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w(px(0.0))
                    .child(div().text_size(px(13.0)).font_weight(FontWeight::SEMIBOLD).truncate().child(title))
                    .child(div().text_size(px(11.5)).text_color(pal.tertiary).truncate().child(line)),
            )
            .on_click(Self::go(cx, Section::Overview))
    }

    /// The bottom of the sidebar: "Pause git"; while paused, a yellow box with the time left and Resume.
    fn pause_box(d: &Data, pal: Palette, cx: &mut Context<'_, Self>) -> AnyElement {
        let frame = div()
            .flex()
            .flex_col()
            .p(px(10.0))
            .rounded(px(14.0))
            .bg(pal.elevated)
            .border_1()
            .border_color(pal.hairline);
        if let Some(left) = d.pause.short(d.now) {
            let fraction = d.saved.pause_left(d.now);
            let mark: AnyElement = match fraction {
                Some(f) => ring(f, pal.warning, pal.warning.opacity(0.2), 26.0, 2.6).into_any_element(),
                None => div()
                    .flex()
                    .items_center()
                    .justify_center()
                    .size(px(26.0))
                    .rounded_full()
                    .border_2()
                    .border_color(pal.warning)
                    .child(icon(Icon::Infinity, 13.0, pal.warning))
                    .into_any_element(),
            };
            return frame
                .bg(pal.warning.opacity(0.06))
                .border_color(pal.warning.opacity(0.35))
                .gap(px(10.0))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(9.0))
                        .child(mark)
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .flex_1()
                                .min_w(px(0.0))
                                .child(div().text_size(px(13.0)).font_weight(FontWeight::SEMIBOLD).child("git paused"))
                                .child(div().text_size(px(11.5)).text_color(pal.tertiary).truncate().child(left)),
                        )
                        .child(help(
                            "side-paused",
                            "git goes straight to the hosts. Hooks and MCP tools still ask your phone.",
                            d,
                            pal,
                            cx,
                        )),
                )
                .child(
                    btn_icon("side-resume", Icon::Play, "Resume", Btn::Primary, pal)
                        .h(px(34.0))
                        .rounded(px(10.0))
                        .w_full()
                        .on_click(Self::on_model(cx, Model::resume)),
                )
                .into_any_element();
        }
        let b = btn_icon("side-pause", Icon::Pause, "Pause git", Btn::Plain, pal)
            .h(px(34.0))
            .rounded(px(10.0))
            .w_full()
            .bg(pal.tile());
        frame
            .child(if d.paired {
                b.on_click(Self::on_view(cx, |this, _| {
                    this.pause_sheet = !this.pause_sheet;
                }))
            } else {
                b.opacity(0.45).cursor_default()
            })
            .into_any_element()
    }

    /// The "Pause git" sheet, over a dimmed window: the lengths, then Cancel and the button that pauses (held, for
    /// "Until I resume").
    pub(super) fn pause_sheet(&self, d: &Data, pal: Palette, cx: &mut Context<'_, Self>) -> AnyElement {
        let choice = self.pause_choice;
        let manual = choice == PauseFor::Manual;
        let mut lengths = segmented(pal).w_full();
        for length in LENGTHS {
            let on = choice == length;
            let label = match length {
                PauseFor::Day => "1 day",
                other => other.short(),
            };
            let mut s = segment_frame(SharedString::from(format!("sheet-{}", length.id())), on, pal);
            if length == PauseFor::Manual {
                s = s.text_color(pal.danger).child(icon(Icon::Infinity, 13.0, pal.danger));
            }
            lengths = lengths.child(s.child(label).on_click(Self::on_view(cx, move |this, _| {
                this.pause_choice = length;
                this.hold = None;
            })));
        }
        let confirm: AnyElement = if manual {
            self.hold_button("sheet-hold", Held::PauseUntilResume, Icon::Pause, "Hold to pause", pal, cx)
                .flex_1()
                .into_any_element()
        } else {
            btn_icon("sheet-pause", Icon::Pause, format!("Pause for {}", choice_words(choice)), Btn::Primary, pal)
                .h(px(38.0))
                .rounded(px(11.0))
                .flex_1()
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.pause_sheet = false;
                    this.model.update(cx, |m, cx| m.pause_for(choice, cx));
                }))
                .into_any_element()
        };
        let sheet = popover(pal)
            .absolute()
            .left(px(20.0))
            .bottom(px(64.0))
            .w(px(340.0))
            .gap(px(12.0))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(3.0))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(4.0))
                            .child(div().text_size(px(15.0)).font_weight(FontWeight::SEMIBOLD).child("Pause git"))
                            .child(help(
                                "pause",
                                "git goes straight to GitHub and the other hosts until the pause ends or you resume. \
                                 Pushes are not asked about meanwhile.",
                                d,
                                pal,
                                cx,
                            )),
                    )
                    .child(
                        div()
                            .text_size(px(12.5))
                            .text_color(pal.tertiary)
                            .child("Pushes go straight to the hosts. Hooks and MCP still ask."),
                    ),
            )
            .child(lengths)
            .child(
                div()
                    .flex()
                    .gap(px(8.0))
                    .child(
                        btn("sheet-cancel", "Cancel", Btn::Plain, pal).h(px(38.0)).rounded(px(11.0)).flex_1().on_click(
                            Self::on_view(cx, |this, _| {
                                this.pause_sheet = false;
                                this.hold = None;
                            }),
                        ),
                    )
                    .child(div().flex().flex_1().min_w(px(170.0)).child(confirm)),
            );
        div()
            .absolute()
            .inset_0()
            .child(
                div()
                    .id("sheet-scrim")
                    .absolute()
                    .inset_0()
                    .bg(gpui::black().opacity(if pal.dark {
                        0.45
                    } else {
                        0.25
                    }))
                    .on_click(Self::on_view(cx, |this, _| {
                        this.pause_sheet = false;
                        this.hold = None;
                    })),
            )
            .child(sheet)
            .into_any_element()
    }
}

/// "15 minutes", "1 hour", "1 day".
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
    fn every_section_has_its_own_icon() {
        let icons: std::collections::HashSet<_> = Section::ALL.iter().map(|s| s.icon().0).collect();
        assert_eq!(icons.len(), Section::ALL.len());
    }

    #[test]
    fn the_sheet_offers_until_resume_last() {
        assert_eq!(LENGTHS.last(), Some(&PauseFor::Manual));
        assert!(LENGTHS.iter().all(|l| PauseFor::ALL.contains(l)));
    }
}
