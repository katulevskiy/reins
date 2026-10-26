//! The phone's Autopilot in the window (`DeskNext-Rules.png`, `DeskNext-Home.png` 03, `DeskNext-MenuBar.png`): the
//! picker at the sidebar's foot (Manual or Auto, "For a while": Overnight, Lockdown, Bypass; Stop while one runs),
//! the Bypass sheet, the band a running mode lays over every page (its name, its clock, Stop), the tag on a row the
//! phone decided without the user, "Why Autopilot allowed" with "This was wrong", and where Autopilot leans on a
//! request that waits.
//!
//! None of it changes a mode here: every change is a request to the phone ([`Model::change_mode`]), which applies
//! what is stricter and asks its owner, held to approve, for anything looser.

use gpui::prelude::FluentBuilder as _;
use gpui::{
    Anchor, AnyElement, Context, Div, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _,
    SharedString, StatefulInteractiveElement as _, Styled as _, anchored, deferred, div, point, px,
};
use reins_desktop::autopilot::{ModeChoice, is_stricter};
use reins_desktop::journal::{Entry, Outcome};
use reins_proto::autopilot::{DecidedBy, Mode};

use super::icons::{Glyph, glyph};
use super::{Data, Root, kit};
use crate::modes::{self, Running};
use crate::theme::motion::{Entrance, enter};
use crate::theme::{Field, MONO, NextText as _, Palette};

/// The lengths the Bypass sheet offers, in minutes (the phone allows up to a day).
pub const BYPASS_LENGTHS: [(u16, &str); 4] = [(15, "15 min"), (60, "1 hour"), (240, "4 hours"), (1_440, "1 day")];

impl Data {
    /// The mode that runs on the phone now for this computer.
    pub(super) fn running(&self) -> Option<Running> {
        modes::running(self.modes.as_ref(), self.now)
    }
}

/// A running mode's field: red for Bypass, violet for Overnight, amber for Lockdown.
#[must_use]
pub fn field_of(r: Running) -> Field {
    match r {
        Running::Bypass {
            ..
        } => Field::Delete,
        Running::Overnight {
            ..
        } => Field::Grant,
        Running::Lockdown => Field::Warn,
    }
}

/// A running mode's glyph.
#[must_use]
pub fn glyph_of(r: Running) -> Glyph {
    match r {
        Running::Bypass {
            ..
        } => Glyph::ShieldAlert,
        Running::Overnight {
            ..
        } => Glyph::Moon,
        Running::Lockdown => Glyph::Lock,
    }
}

/// The tag at the end of a row the phone decided without the user (`.otag`): "Auto" in violet (red when it denied),
/// "Bypass" in red, "Lockdown" in amber, "Overnight" in violet.
pub fn decided_tag(e: &Entry, pal: Palette) -> Option<Div> {
    let a = e.autopilot.as_ref()?;
    let denied = !matches!(e.outcome, Outcome::Approved | Outcome::Allowed);
    let (g, color) = match a.by {
        DecidedBy::Autopilot if denied => (Glyph::Autopilot, pal.danger),
        DecidedBy::Autopilot => (Glyph::Autopilot, pal.violet()),
        DecidedBy::Overnight => (Glyph::Moon, pal.violet()),
        DecidedBy::Bypass => (Glyph::ShieldAlert, pal.danger),
        DecidedBy::Lockdown => (Glyph::Lock, pal.warning),
    };
    Some(
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(3.0))
            .h(px(20.0))
            .px(px(7.0))
            .rounded_full()
            .bg(color.opacity(0.12))
            .text_color(color)
            .text_size(px(11.5))
            .font_weight(FontWeight::SEMIBOLD)
            .whitespace_nowrap()
            .child(glyph(g, 11.0, color))
            .child(modes::tag(a)),
    )
}

/// Where Autopilot leans on a request that waits, as a quiet pill: "Autopilot would allow · 97%". `on_field`: drawn
/// on a field's colour.
pub fn lean_pill(e: &Entry, on_field: bool, pal: Palette) -> Option<Div> {
    let l = e.leaning.as_ref().filter(|_| e.outcome == Outcome::Waiting)?;
    let (words, confidence) = modes::leaning(*l);
    let (bg, fg) = if on_field {
        (pal.on_field_wash(), pal.on_field())
    } else {
        (pal.violet().opacity(0.12), pal.violet())
    };
    Some(
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(6.0))
            .h(px(26.0))
            .px(px(10.0))
            .rounded_full()
            .bg(bg)
            .text_color(fg)
            .text_size(px(12.5))
            .font_weight(FontWeight::SEMIBOLD)
            .whitespace_nowrap()
            .child(glyph(Glyph::Autopilot, 13.0, fg))
            .child(words)
            .child(div().font_family(MONO).child(format!("{confidence}%"))),
    )
}

impl Root {
    /// The sidebar's foot (`.picker`): Manual or Auto (picked: ink for Manual, violet for Auto), or Stop while a
    /// bypass or Overnight runs; "For a while" with its three modes; a change waiting on the phone says so.
    pub(super) fn mode_picker(&self, d: &Data, pal: Palette, cx: &mut Context<'_, Self>) -> AnyElement {
        let running = d.running();
        let mut foot = div().flex().flex_col().gap(px(4.0)).mb(px(4.0));
        let enabled = d.paired && d.mode_asked.is_none();
        if let Some(
            r @ (Running::Bypass {
                ..
            }
            | Running::Overnight {
                ..
            }),
        ) = running
        {
            let color = pal.tone(crate::theme::Tone::Field(field_of(r)));
            let label = format!("Stop {}", r.name().to_lowercase());
            foot = foot.child(
                div()
                    .id("side-stop-mode")
                    .flex()
                    .items_center()
                    .justify_center()
                    .gap(px(6.0))
                    .h(px(38.0))
                    .rounded_full()
                    .bg(pal.paper_raised())
                    .border_1()
                    .border_color(pal.rule())
                    .text_size(px(13.5))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(color)
                    .when(enabled, |b| {
                        b.cursor_pointer()
                            .active(|s| s.opacity(kit::button::PRESSED))
                            .on_click(Self::on_model(cx, |m, cx| m.change_mode(ModeChoice::Stop, cx)))
                    })
                    .when(!enabled, |b| b.opacity(kit::button::DISABLED))
                    .child(glyph(Glyph::Close, 14.0, color))
                    .child(label),
            );
        } else {
            let picked = modes::picked(d.modes.as_ref());
            let seg = |id: &'static str, g: Glyph, label: &'static str, mode: Mode, choice: ModeChoice| {
                let on = picked == Some(mode);
                let (bg, fg) = match (on, mode) {
                    (true, Mode::Auto) => (Some(pal.violet_fill()), pal.on_field()),
                    (true, _) => (Some(pal.inverse()), pal.on_inverse()),
                    (false, _) => (None, pal.ink_2()),
                };
                div()
                    .id(id)
                    .flex()
                    .flex_1()
                    .items_center()
                    .justify_center()
                    .gap(px(5.0))
                    .h(px(30.0))
                    .rounded_full()
                    .when_some(bg, gpui::Styled::bg)
                    .text_color(fg)
                    .text_size(px(13.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .whitespace_nowrap()
                    .child(glyph(g, 13.0, fg))
                    .child(label)
                    .when(enabled && !on, |s| {
                        s.cursor_pointer()
                            .hover(|h| h.text_color(pal.ink()))
                            .active(|s| s.opacity(kit::button::PRESSED))
                            .on_click(Self::on_model(cx, move |m, cx| m.change_mode(choice, cx)))
                    })
            };
            foot = foot.child(
                div()
                    .flex()
                    .items_center()
                    .p(px(3.0))
                    .gap(px(2.0))
                    .rounded_full()
                    .bg(pal.paper_sunken())
                    .when(!d.paired, |p| p.opacity(kit::button::DISABLED))
                    .child(seg("side-manual", Glyph::Person, "Manual", Mode::Manual, ModeChoice::Manual))
                    .child(seg("side-auto", Glyph::Autopilot, "Auto", Mode::Auto, ModeChoice::Auto)),
            );
        }
        // A change the phone's owner has to approve: where.
        if let Some(asked) = d.mode_asked.filter(|c| !is_stricter(*c, d.modes.as_ref())) {
            foot = foot.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .px(px(10.0))
                    .text_size(px(12.0))
                    .line_height(px(16.0))
                    .text_color(pal.violet())
                    .child(glyph(Glyph::Phone, 13.0, pal.violet()))
                    .child(div().min_w(px(0.0)).truncate().child(format!(
                        "{}: confirm on {}",
                        short(asked),
                        d.phone()
                    ))),
            );
        }
        foot.child(self.for_a_while(d, enabled, pal, cx)).into_any_element()
    }

    /// "For a while" (`.fw`): its three modes as dots; it opens the menu of them.
    fn for_a_while(&self, d: &Data, enabled: bool, pal: Palette, cx: &mut Context<'_, Self>) -> AnyElement {
        let dot = |g: Glyph, color: gpui::Hsla| {
            div().flex().items_center().justify_center().size(px(18.0)).rounded_full().bg(color).child(glyph(
                g,
                11.0,
                pal.on_field(),
            ))
        };
        let open = self.while_open;
        let row = div()
            .id("side-for-a-while")
            .relative()
            .flex()
            .items_center()
            .gap(px(8.0))
            .h(px(34.0))
            .px(px(10.0))
            .rounded(px(10.0))
            .text_size(px(13.5))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(pal.ink_2())
            .child(glyph(Glyph::Timer, 14.0, pal.ink_2()))
            .child(div().flex_1().child("For a while"))
            .child(
                div()
                    .flex()
                    .gap(px(3.0))
                    .child(dot(Glyph::Moon, pal.tone(crate::theme::Tone::Field(Field::Grant))))
                    .child(dot(Glyph::Lock, pal.tone(crate::theme::Tone::Field(Field::Warn))))
                    .child(dot(Glyph::ShieldAlert, pal.tone(crate::theme::Tone::Field(Field::Delete)))),
            );
        if !enabled {
            return row.opacity(kit::button::DISABLED).into_any_element();
        }
        let row = row.cursor_pointer().hover(|s| s.text_color(pal.ink())).on_click(Self::on_view(cx, |this, _| {
            let just_closed = this.menu_closed.is_some_and(|at| at.elapsed() < super::activity::REOPEN_GUARD);
            this.while_open = !this.while_open && !just_closed;
        }));
        if !open {
            return row.into_any_element();
        }
        let lockdown_on = d.running() == Some(Running::Lockdown);
        let tile = |f: Field| (pal.tone(crate::theme::Tone::Field(f)), pal.on_field());
        let menu = kit::menu(pal)
            .w(px(280.0))
            .child(
                kit::menu_item(
                    "while-overnight",
                    Glyph::Moon,
                    tile(Field::Grant),
                    "Overnight",
                    Some("Starts on your phone".into()),
                    matches!(d.running(), Some(Running::Overnight { .. })),
                    false,
                    pal,
                )
                .opacity(0.6)
                .cursor_default(),
            )
            .child(
                kit::menu_item(
                    "while-lockdown",
                    Glyph::Lock,
                    tile(Field::Warn),
                    "Lockdown",
                    Some("Everything is denied".into()),
                    lockdown_on,
                    false,
                    pal,
                )
                .when(!lockdown_on, |i| {
                    i.on_click(cx.listener(|this, _, _, cx| {
                        this.while_open = false;
                        this.model.update(cx, |m, cx| m.change_mode(ModeChoice::Lockdown, cx));
                    }))
                }),
            )
            .child(
                kit::menu_item(
                    "while-bypass",
                    Glyph::ShieldAlert,
                    tile(Field::Delete),
                    "Bypass",
                    Some("Needs your phone".into()),
                    false,
                    true,
                    pal,
                )
                .on_click(Self::on_view(cx, |this, _| {
                    this.while_open = false;
                    this.bypass_sheet = true;
                })),
            )
            .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                this.while_open = false;
                this.menu_closed = Some(std::time::Instant::now());
                cx.notify();
            }));
        // Above the row: the sidebar's foot has no room under it.
        row.child(
            div().absolute().top_0().left_0().size_0().child(
                deferred(
                    anchored()
                        .anchor(Anchor::BottomLeft)
                        .offset(point(px(0.0), px(-6.0)))
                        .snap_to_window_with_margin(px(12.0))
                        .child(enter(menu.relative(), "while-menu", Entrance::Menu)),
                )
                .with_priority(1),
            ),
        )
        .into_any_element()
    }

    /// The Bypass sheet, as the boards draw it: its name in red, what still asks, the lengths, and the button that
    /// asks the phone (a bypass starts only once its owner approves it there).
    pub(super) fn bypass_sheet(&self, d: &Data, pal: Palette, cx: &mut Context<'_, Self>) -> AnyElement {
        let minutes = self.bypass_minutes;
        let mut lengths = kit::segmented(pal).w_full();
        for (m, label) in BYPASS_LENGTHS {
            lengths = lengths.child(
                kit::segment(SharedString::from(format!("bypass-{m}")), label, minutes == m, pal)
                    .px(px(10.0))
                    .on_click(Self::on_view(cx, move |this, _| this.bypass_minutes = m)),
            );
        }
        let close = |this: &mut Self, _: &mut Context<'_, Self>| this.bypass_sheet = false;
        let sheet = div()
            .occlude()
            .relative()
            .flex()
            .flex_col()
            .w(px(480.0))
            .px(px(24.0))
            .py(px(22.0))
            .rounded(px(22.0))
            .bg(pal.paper_raised())
            .border_1()
            .border_color(pal.rule())
            .shadow_lg()
            .child(div().signal(crate::theme::text::MODE_TITLE).text_color(pal.danger).child("Bypass"))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap(px(6.0))
                    .mt(px(10.0))
                    .child(
                        div()
                            .text_size(px(13.5))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(pal.ink_2())
                            .child("Still asks"),
                    )
                    .child(kit::status_pill(Some(Glyph::Plus), "New AI", None, pal))
                    .child(kit::status_pill(Some(Glyph::Phone), "New phone", None, pal)),
            )
            .child(
                div()
                    .mt(px(10.0))
                    .text_size(px(14.0))
                    .line_height(px(20.0))
                    .text_color(pal.ink_2())
                    .child("Everything else is approved without asking, for every AI."),
            )
            .child(div().mt(px(16.0)).child(lengths))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.0))
                    .mt(px(14.0))
                    .px(px(14.0))
                    .py(px(10.0))
                    .rounded(px(14.0))
                    .bg(pal.paper_sunken())
                    .child(glyph(Glyph::FaceId, 18.0, pal.ink_2()))
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.0))
                            .text_size(px(13.5))
                            .line_height(px(18.0))
                            .text_color(pal.ink_2())
                            .child(format!("Confirm on {}", d.phone())),
                    ),
            )
            .child(
                div()
                    .flex()
                    .gap(px(8.0))
                    .mt(px(18.0))
                    .child(
                        div().flex().flex_none().w(gpui::relative(0.41)).child(
                            kit::button("bypass-cancel", kit::Look::Soft, kit::Size::Regular, None, "Cancel", pal)
                                .w_full()
                                .on_click(Self::on_view(cx, close)),
                        ),
                    )
                    .child(
                        div().flex().flex_1().min_w(px(0.0)).child(
                            kit::button(
                                "bypass-ask",
                                kit::Look::Inverse,
                                kit::Size::Regular,
                                Some(Glyph::Phone),
                                "Ask my phone",
                                pal,
                            )
                            .w_full()
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.bypass_sheet = false;
                                this.model.update(cx, |m, cx| m.change_mode(ModeChoice::Bypass(minutes), cx));
                            })),
                        ),
                    ),
            );
        div()
            .absolute()
            .inset_0()
            .child(div().id("bypass-scrim").absolute().inset_0().bg(pal.scrim()).on_click(Self::on_view(cx, close)))
            .child(div().absolute().inset_0().flex().justify_center().items_start().pt(px(180.0)).child(enter(
                sheet,
                "bypass-sheet",
                Entrance::Sheet,
            )))
            .into_any_element()
    }

    /// The band a running mode lays over every page (`.mtop`): its field, its name and clock, what still asks, and
    /// Stop (stricter, so the phone applies it at once). Lockdown's way out is asked on the phone.
    pub(super) fn mode_band(d: &Data, pal: Palette, cx: &mut Context<'_, Self>) -> Option<AnyElement> {
        let r = d.running().filter(|_| d.paired)?;
        let enabled = d.mode_asked.is_none();
        let (label, look, g, choice) = match r {
            Running::Lockdown => ("Leave", kit::Look::Wash, Some(Glyph::Phone), ModeChoice::Manual),
            _ => ("Stop now", kit::Look::OnField, None, ModeChoice::Stop),
        };
        let action = kit::button("band-stop", look, kit::Size::Small, g, label, pal)
            .when(enabled, |b| b.on_click(Self::on_model(cx, move |m, cx| m.change_mode(choice, cx))))
            .when(!enabled, |b| b.opacity(kit::button::DISABLED));
        let line = match r {
            Running::Bypass {
                ..
            } => Some("Still asks: new AIs, new phones"),
            Running::Overnight {
                ..
            } => Some("Auto for agents left working"),
            Running::Lockdown => Some("Everything is denied"),
        };
        Some(
            kit::field_surface(field_of(r), 0.0, 0.0)
                .flex()
                .flex_none()
                .items_center()
                .gap(px(10.0))
                .h(px(56.0))
                .px(px(24.0))
                .text_color(pal.on_field())
                .child(glyph(glyph_of(r), 18.0, pal.on_field()))
                .child(div().text_size(px(15.0)).font_weight(FontWeight::SEMIBOLD).child(r.name()))
                .when_some(r.clock(d.now).filter(|_| r.until() != Some(i64::MAX)), |b, clock| {
                    b.child(div().font_family(MONO).text_size(px(15.0)).font_weight(FontWeight::SEMIBOLD).child(clock))
                })
                .when_some(line, |b, line| {
                    b.child(
                        div()
                            .flex_1()
                            .min_w(px(0.0))
                            .truncate()
                            .text_size(px(13.0))
                            .text_color(pal.on_field_2())
                            .child(line),
                    )
                })
                .child(action)
                .into_any_element(),
        )
    }

    /// Who decided `e` on the phone without the user, and why (`DeskNext-Home.png` 03): the title, the verdict, its
    /// reasons (each on its own line, as the phone gave them), and "This was wrong" when it can be taught.
    pub(super) fn why_panel(e: &Entry, d: &Data, pal: Palette, cx: &mut Context<'_, Self>) -> Option<AnyElement> {
        let a = e.autopilot.as_ref()?;
        let tone = if matches!(e.outcome, Outcome::Approved | Outcome::Allowed) {
            pal.violet()
        } else {
            pal.danger
        };
        let mut panel = div()
            .flex()
            .flex_col()
            .mt(px(20.0))
            .child(
                div()
                    .text_size(px(17.0))
                    .line_height(px(22.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(pal.ink())
                    .child(modes::why_title(a, e.outcome)),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .mt(px(6.0))
                    .when_some(decided_tag(e, pal), gpui::ParentElement::child)
                    .child(
                        div()
                            .text_size(px(13.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(tone)
                            .child(modes::verdict(a, e.outcome)),
                    ),
            );
        if !a.reasons.is_empty() {
            panel = panel.child(kit::section(None, "Why", None, pal).px(px(0.0)).pt(px(14.0)));
            for (i, reason) in a.reasons.iter().enumerate() {
                panel = panel.child(
                    div()
                        .id(("why", i))
                        .flex()
                        .items_center()
                        .gap(px(10.0))
                        .py(px(8.0))
                        .when(i > 0, |r| r.border_t_1().border_color(pal.rule()))
                        .child(glyph(Glyph::Check, 15.0, pal.ink_2()))
                        // The phone's words, on their own line, capped when they were made safe to show.
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.0))
                                .truncate()
                                .text_size(px(14.0))
                                .text_color(pal.ink())
                                .child(SharedString::from(reason.clone())),
                        ),
                );
            }
        }
        if let Some((request_id, approve)) = modes::correction(e) {
            let sent = d.corrected.contains(&request_id);
            let sending = d.correcting.as_deref() == Some(request_id.as_str());
            let (label, line) = if sent {
                ("Told Autopilot", "Next time it asks you")
            } else if approve {
                ("This was wrong", "It should have allowed it: asked on your phone")
            } else {
                ("This was wrong", "Next time it asks you")
            };
            let button = kit::button(
                "why-wrong",
                kit::Look::Soft,
                kit::Size::Small,
                Some(if sent {
                    Glyph::Check
                } else {
                    Glyph::Autopilot
                }),
                label,
                pal,
            );
            let button = if sent || sending || d.correcting.is_some() {
                button.opacity(if sent {
                    1.0
                } else {
                    kit::button::DISABLED
                })
            } else {
                button.on_click(Self::on_model(cx, move |m, cx| m.correct_autopilot(request_id.clone(), approve, cx)))
            };
            panel = panel.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.0))
                    .mt(px(12.0))
                    .child(button)
                    .child(div().text_size(px(12.5)).text_color(pal.ink_meta()).child(line)),
            );
        }
        Some(panel.into_any_element())
    }
}

/// A change of mode in a word or two, for "confirm on your phone".
fn short(choice: ModeChoice) -> String {
    match choice {
        ModeChoice::Manual => "Manual".to_owned(),
        ModeChoice::Auto => "Auto".to_owned(),
        ModeChoice::Lockdown => "Lockdown".to_owned(),
        ModeChoice::Bypass(m) => format!("Bypass, {}", reins_desktop::work_session::span(u64::from(m) * 60)),
        ModeChoice::Stop => "Stop".to_owned(),
    }
}
