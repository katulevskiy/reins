//! The status window's sidebar (`Desk-Home.png`): the mark, a plate that says what Reins is doing (green on, violet
//! waiting, red paused, amber when it needs you), the sections, and pausing git at the bottom (a red field while
//! paused: the desktop's bypass); and the "Pause git" sheet (15 min, 1 hour, 1 day, or until you resume, held to
//! confirm).

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, Context, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _, SharedString,
    StatefulInteractiveElement as _, Styled as _, div, px,
};

use super::hold::Held;
use super::icons::{Glyph, Icon, icon};
use super::parts::{help, look_color, ring};
use super::{Data, Root, kit};
use crate::format;
use crate::model::{Model, Section};
use crate::pause::PauseFor;
use crate::shortcuts;
use crate::theme::{Field, Palette, SERIF, SignalText as _};
use crate::tray::Look;

const WIDTH: f32 = 236.0;

/// The lengths the sheet offers.
const LENGTHS: [PauseFor; 4] = [PauseFor::Quarter, PauseFor::Hour, PauseFor::Day, PauseFor::Manual];

impl Section {
    /// The section's glyph (in ink; Signal colours only what needs a decision).
    #[must_use]
    pub fn icon(self) -> Icon {
        match self {
            Self::Home => Icon::LayoutDashboard,
            Self::Activity => Icon::Activity,
            Self::Connections => Icon::Cable,
            Self::Keys => Icon::KeyRound,
            Self::Rules => Icon::ShieldCheck,
            Self::Settings => Icon::Settings,
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
        Look::Attention => (d.line.clone(), "Start it in Home".to_owned(), Icon::TriangleAlert),
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
        let entry = |id: SharedString, ic: Icon, label: &'static str, selected: bool, end: AnyElement| {
            div()
                .id(id)
                .flex()
                .items_center()
                .gap(px(10.0))
                .h(px(38.0))
                .px(px(10.0))
                .rounded(px(12.0))
                .cursor_pointer()
                .text_size(px(14.0))
                .active(|s| s.opacity(kit::button::PRESSED))
                .when_else(
                    selected,
                    |r| {
                        r.bg(pal.paper_raised())
                            .border_1()
                            .border_color(pal.rule())
                            .text_color(pal.ink())
                            .font_weight(FontWeight::SEMIBOLD)
                    },
                    |r| {
                        r.text_color(pal.ink_2()).hover(|s| s.bg(pal.paper_sunken().opacity(0.7)).text_color(pal.ink()))
                    },
                )
                .child(icon(
                    ic,
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
        for (i, section) in Section::ALL.into_iter().enumerate() {
            let selected = d.section == section && !d.gallery;
            let badge = (section == Section::Activity && waiting > 0).then_some(waiting);
            let end = match badge {
                Some(n) => div()
                    .flex()
                    .items_center()
                    .justify_center()
                    .min_w(px(20.0))
                    .h(px(18.0))
                    .px(px(6.0))
                    .rounded_full()
                    .bg(pal.violet_fill())
                    .text_color(pal.on_field())
                    .text_size(px(11.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(n.to_string())
                    .into_any_element(),
                None => kit::kbd(shortcuts::hint(&(i + 1).to_string()), pal).into_any_element(),
            };
            nav = nav.child(
                entry(
                    SharedString::from(format!("nav-{}", section.id())),
                    section.icon(),
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
                entry("nav-gallery".into(), Icon::Package, "Gallery", d.gallery, div().into_any_element()).on_click(
                    Self::on_view(cx, |this, _| {
                        this.gallery = true;
                    }),
                ),
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
                    .child(super::parts::mark(26.0))
                    .child(div().font_family(SERIF).text_size(px(24.0)).line_height(px(28.0)).child("Reins")),
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

    /// What Reins is doing: a plate with a tile tinted by it; a click opens Home, where the rest is.
    fn status_card(d: &Data, pal: Palette, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let color = look_color(d.look, pal);
        let (title, line, ic) = status_words(d);
        div()
            .id("status-card")
            .flex()
            .items_center()
            .gap(px(10.0))
            .p(px(12.0))
            .rounded(px(16.0))
            .bg(pal.paper_raised())
            .border_1()
            .border_color(pal.rule())
            .cursor_pointer()
            .active(|s| s.opacity(kit::button::PRESSED))
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .size(px(32.0))
                    .rounded(px(10.0))
                    .bg(color.opacity(0.13))
                    .child(icon(ic, 18.0, color)),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w(px(0.0))
                    .child(div().text_size(px(13.5)).font_weight(FontWeight::SEMIBOLD).truncate().child(title))
                    .child(div().text_size(px(12.0)).text_color(pal.ink_meta()).truncate().child(line)),
            )
            .on_click(Self::go(cx, Section::Home))
    }

    /// The bottom of the sidebar: "Pause git"; while paused, the desktop's bypass: a red field with the time left and
    /// Resume.
    fn pause_box(d: &Data, pal: Palette, cx: &mut Context<'_, Self>) -> AnyElement {
        if let Some(left) = d.pause.short(d.now) {
            let fraction = d.saved.pause_left(d.now);
            let on = pal.on_field();
            let mark: AnyElement = match fraction {
                Some(f) => ring(f, on, on.opacity(0.25), 26.0, 2.6).into_any_element(),
                None => div()
                    .flex()
                    .items_center()
                    .justify_center()
                    .size(px(26.0))
                    .rounded_full()
                    .border_2()
                    .border_color(on)
                    .child(icon(Icon::Infinity, 13.0, on))
                    .into_any_element(),
            };
            return kit::field_surface(Field::Delete, 20.0, 20.0)
                .flex()
                .flex_col()
                .gap(px(10.0))
                .p(px(12.0))
                .text_color(on)
                .child(
                    div()
                        .relative()
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
                                .child(div().text_size(px(13.5)).font_weight(FontWeight::SEMIBOLD).child("git paused"))
                                .child(div().text_size(px(12.0)).text_color(pal.on_field_2()).truncate().child(left)),
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
                    kit::button(
                        "side-resume",
                        kit::Look::OnField,
                        kit::Size::Regular,
                        Some(Glyph::Play),
                        "Resume",
                        pal,
                    )
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

    /// The "Pause git" sheet, over a dimmed window: the lengths, then Cancel and the button that pauses (held, for
    /// "Until I resume").
    pub(super) fn pause_sheet(&self, d: &Data, pal: Palette, cx: &mut Context<'_, Self>) -> AnyElement {
        let choice = self.pause_choice;
        let manual = choice == PauseFor::Manual;
        let mut lengths = kit::segmented(pal).w_full();
        for length in LENGTHS {
            let on = choice == length;
            let label = match length {
                PauseFor::Day => "1 day",
                other => other.short(),
            };
            let mut s = kit::segment(SharedString::from(format!("sheet-{}", length.id())), "", on, pal);
            if length == PauseFor::Manual {
                s = s.text_color(pal.danger).child(icon(Icon::Infinity, 14.0, pal.danger));
            }
            lengths = lengths.child(s.child(label).on_click(Self::on_view(cx, move |this, _| {
                this.pause_choice = length;
                this.hold = None;
            })));
        }
        let confirm: AnyElement = if manual {
            self.hold_button("sheet-hold", Held::PauseUntilResume, Icon::Pause, "Hold to pause", "Paused", pal, cx)
                .flex_1()
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
            .flex_1()
            .on_click(cx.listener(move |this, _, _, cx| {
                this.pause_sheet = false;
                this.model.update(cx, |m, cx| m.pause_for(choice, cx));
            }))
            .into_any_element()
        };
        let sheet = kit::plate(pal)
            .occlude()
            .relative()
            .w(px(380.0))
            .gap(px(14.0))
            .p(px(22.0))
            .shadow_lg()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(6.0))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(6.0))
                            .child(div().sheet_title().child(kit::ask_line("Pause ", "git")))
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
                            .text_size(px(14.0))
                            .line_height(px(20.0))
                            .text_color(pal.ink_2())
                            .child("Pushes go straight to the hosts without asking. Hooks and MCP tools still ask."),
                    ),
            )
            .child(lengths)
            .child(
                div()
                    .flex()
                    .gap(px(8.0))
                    .child(
                        kit::button("sheet-cancel", kit::Look::Outline, kit::Size::Regular, None, "Cancel", pal)
                            .flex_1()
                            .on_click(Self::on_view(cx, |this, _| {
                                this.pause_sheet = false;
                                this.hold = None;
                            })),
                    )
                    .child(div().flex().flex_1().min_w(px(190.0)).child(confirm)),
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
            .child(div().absolute().left(px(20.0)).bottom(px(64.0)).child(crate::theme::motion::enter(
                sheet,
                "pause-sheet",
                crate::theme::motion::Entrance::Sheet,
            )))
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
        let icons: std::collections::HashSet<_> = Section::ALL.iter().map(|s| s.icon().glyph()).collect();
        assert_eq!(icons.len(), Section::ALL.len());
    }

    #[test]
    fn the_sheet_offers_until_resume_last() {
        assert_eq!(LENGTHS.last(), Some(&PauseFor::Manual));
        assert!(LENGTHS.iter().all(|l| PauseFor::ALL.contains(l)));
    }
}
