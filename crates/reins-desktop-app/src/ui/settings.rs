//! Settings: the account card first (who, the server and this computer's key, which phone approves, the background
//! service), then This computer (open at login, the command line tool, notifications, the health checks), Keyboard,
//! Account (disconnect, held to confirm; quit), and the version with the links.

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, Context, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, div, px,
};

use super::health::checked;
use super::hold::Held;
use super::icons::{Icon, icon};
use super::kit::hold::{fill, progress_ring};
use super::parts::{
    Btn, btn, caption, card, chip, group, kbd, page_header, row, row_text, solid_tile, this_computer, tile, toggle,
};
use super::{Data, Root};
use crate::backend::{DaemonState, Setting};
use crate::format::host;
use crate::health;
use crate::model::{Model, Section};
use crate::shortcuts;
use crate::theme::{MONO, Palette, Tone};

/// The keyboard card's lines: the sections in one line ("Home … Settings", "⌘1 – ⌘6"), then the rest.
#[must_use]
pub fn keyboard_lines() -> Vec<(String, String)> {
    let list = shortcuts::list();
    let n = Section::ALL.len();
    let mut out = vec![(
        format!("{} … {}", Section::ALL[0].label(), Section::ALL[n - 1].label()),
        format!("{} – {}", list[0].0, list[n - 1].0),
    )];
    out.extend(list[n..].iter().map(|(keys, what)| (what.clone(), keys.clone())));
    out
}

impl Root {
    pub(super) fn settings(&self, d: &Data, pal: Palette, cx: &mut Context<'_, Self>) -> AnyElement {
        let mut page = div().flex().flex_col().gap(px(18.0)).child(page_header("Settings", None));
        page = page.child(Self::account_card(d, pal, cx));
        page = page.when_some(d.notice.clone(), |p, n| p.child(caption(n, pal).px(px(4.0)).text_color(pal.danger)));
        page = page.when_some(d.note.clone(), |p, n| p.child(caption(n, pal).px(px(4.0))));

        // This computer.
        let autostart = d.autostart;
        let notify = d.config().map(|c| c.notify.clone()).unwrap_or_default();
        let (phone, sound) = (notify.phone, notify.sound);
        let notify_word = match (phone, sound) {
            (true, true) => "With sound",
            (true, false) => "On",
            (false, _) => "Off",
        };
        let health_line = match (&d.health.checks, &d.health.error) {
            (_, Some(e)) => e.clone(),
            (Some(c), None) => {
                let s = health::summary(c).title;
                checked(d).map_or(s.clone(), |at| format!("{s} · {}", at.trim_start_matches("Checked ")))
            }
            (None, None) => "Not checked yet".to_owned(),
        };
        let mut computer = card(pal)
            .child(
                row(pal, true)
                    .id("autostart")
                    .cursor_pointer()
                    .child(solid_tile(Icon::Power, Tone::Neutral, pal, 30.0))
                    .child(row_text("Open at login", None, pal))
                    .child(toggle(autostart, pal))
                    .on_click(Self::on_model(cx, Model::toggle_autostart)),
            )
            .child(
                row(pal, false)
                    .child(solid_tile(Icon::SquareTerminal, Tone::Neutral, pal, 30.0))
                    .child(row_text(
                        "Command line tool",
                        Some(
                            div().font_family(MONO).text_size(px(11.5)).child("~/.local/bin/reins").into_any_element(),
                        ),
                        pal,
                    ))
                    .child(if d.cli_installed {
                        chip(Some(Icon::Check), "Installed", pal.success).into_any_element()
                    } else {
                        btn("cli", "Install", Btn::Plain, pal)
                            .on_click(Self::on_model(cx, Model::install_cli))
                            .into_any_element()
                    }),
            )
            .child(
                row(pal, false)
                    .id("notify")
                    .cursor_pointer()
                    .child(solid_tile(Icon::Bell, Tone::Neutral, pal, 30.0))
                    .child(row_text("Notifications", None, pal))
                    .child(div().text_size(px(13.0)).text_color(pal.ink_2()).child(notify_word))
                    .child(icon(
                        if self.notify_open {
                            Icon::X
                        } else {
                            Icon::ChevronRight
                        },
                        16.0,
                        pal.ink_meta(),
                    ))
                    .on_click(Self::on_view(cx, |this, _| this.notify_open = !this.notify_open)),
            );
        if self.notify_open {
            let sub = |id: &'static str, label: &'static str, on: bool, enabled: bool| {
                row(pal, false)
                    .id(id)
                    .pl(px(56.0))
                    .min_h(px(44.0))
                    .child(row_text(label, None, pal))
                    .child(toggle(on, pal))
                    .when(!enabled, |r| r.opacity(0.5))
            };
            computer = computer
                .child(sub("notify-phone", "Notify me here", phone, true).cursor_pointer().on_click(Self::on_model(
                    cx,
                    move |m, cx| {
                        m.change(
                            Setting::Notify {
                                phone: !phone,
                                sound,
                            },
                            cx,
                        );
                    },
                )))
                .child({
                    let r = sub("notify-sound", "Play a sound", sound && phone, phone);
                    if phone {
                        r.cursor_pointer().on_click(Self::on_model(cx, move |m, cx| {
                            m.change(
                                Setting::Notify {
                                    phone,
                                    sound: !sound,
                                },
                                cx,
                            );
                        }))
                    } else {
                        r
                    }
                });
        }
        computer = computer.child(
            row(pal, false)
                .child(solid_tile(Icon::HeartPulse, Tone::Neutral, pal, 30.0))
                .child(row_text("Health checks", Some(health_line.into_any_element()), pal))
                .child(Self::check_again("run-checks", "Run", d, pal, cx)),
        );
        let picked = d.saved.appearance;
        let mut appearance = super::parts::segmented(pal);
        for a in crate::theme::Appearance::ALL {
            appearance = appearance.child(
                super::parts::segment(("appearance", a as usize), a.label(), a == picked, pal)
                    .on_click(Self::on_model(cx, move |m, cx| m.set_appearance(a, cx))),
            );
        }
        computer = computer.child(
            row(pal, false)
                .child(solid_tile(Icon::Settings, Tone::Neutral, pal, 30.0))
                .child(row_text("Appearance", None, pal))
                .child(appearance),
        );

        // Keyboard.
        let mut keys = div().flex().flex_col().gap(px(8.0)).px(px(14.0)).py(px(12.0));
        for (what, k) in keyboard_lines() {
            keys = keys.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(12.0))
                    .text_size(px(13.0))
                    .child(div().flex_1().min_w(px(0.0)).truncate().text_color(pal.ink_2()).child(what))
                    .child(kbd(k, pal)),
            );
        }

        // Account.
        let disconnect = if d.paired {
            let area = row(pal, true).id("disconnect").relative().overflow_hidden();
            let area = Self::hold_area(area, Held::Disconnect, cx);
            match self.hold_progress(Held::Disconnect) {
                Some(progress) => area
                    .child(fill(progress, pal.danger_fill_2()))
                    .child(div().relative().child(tile(Icon::Unlink, pal.danger, 30.0)))
                    .child(
                        div()
                            .relative()
                            .flex_1()
                            .text_color(pal.danger)
                            .font_weight(FontWeight::MEDIUM)
                            .child("Keep holding"),
                    )
                    .child(div().relative().child(progress_ring(progress, pal.danger, pal.danger_fill_2(), 16.0))),
                None => area
                    .hover(|s| s.bg(pal.danger.opacity(0.05)))
                    .child(tile(Icon::Unlink, pal.danger, 30.0))
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.0))
                            .text_color(pal.danger)
                            .font_weight(FontWeight::MEDIUM)
                            .child(format!("Disconnect {}", this_computer())),
                    )
                    .child(div().text_size(px(12.0)).text_color(pal.ink_meta()).child("Hold")),
            }
            .into_any_element()
        } else {
            row(pal, true)
                .id("pair-again")
                .cursor_pointer()
                .child(tile(Icon::Smartphone, pal.violet(), 30.0))
                .child(row_text("Pair again", None, pal))
                .child(icon(Icon::ChevronRight, 16.0, pal.ink_meta()))
                .on_click(Self::on_model(cx, Model::sign_out))
                .into_any_element()
        };
        let account = card(pal).child(disconnect).child(
            row(pal, false)
                .id("quit")
                .cursor_pointer()
                .hover(|s| s.bg(pal.paper_sunken().opacity(0.5)))
                .child(tile(Icon::LogOut, pal.ink_2(), 30.0))
                .child(row_text("Quit Reins", None, pal))
                .child(div().text_size(px(12.0)).text_color(pal.ink_meta()).child("service keeps running"))
                .on_click(Self::on_model(cx, Model::quit)),
        );

        page = page.child(
            div()
                .flex()
                .flex_wrap()
                .gap(px(16.0))
                .child(div().flex_1().min_w(px(280.0)).child(group("This computer", None, computer, pal)))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(18.0))
                        .flex_1()
                        .min_w(px(280.0))
                        .child(group("Keyboard", None, card(pal).child(keys), pal))
                        .child(group("Account", None, account, pal)),
                ),
        );

        // The version and the links.
        let footer_link = |id: &'static str, label: &'static str, url: String| {
            div()
                .id(id)
                .cursor_pointer()
                .hover(|s| s.text_color(pal.ink_2()))
                .child(label)
                .on_click(move |_, _, cx| cx.open_url(&url))
        };
        page = page.child(
            div()
                .flex()
                .justify_center()
                .gap(px(5.0))
                .pt(px(4.0))
                .text_size(px(12.0))
                .text_color(pal.ink_meta())
                .child(format!("Reins {}", reins_desktop::update::VERSION))
                .child("·")
                .child(footer_link("downloads", "Downloads", crate::links::download()))
                .child("·")
                .child(footer_link("terms", "Terms", crate::links::TERMS.to_owned()))
                .child("·")
                .child(footer_link("privacy", "Privacy", crate::links::PRIVACY.to_owned())),
        );
        page.into_any_element()
    }

    /// Who this computer is paired as, the server and its key, which phone approves, and the service.
    fn account_card(d: &Data, pal: Palette, cx: &mut Context<'_, Self>) -> AnyElement {
        let account = d.saved.account.clone().unwrap_or_else(|| host(&d.server));
        let mut line = host(&d.server);
        if let Some(fp) = &d.fingerprint {
            line = format!("{line} · key {fp}");
        }
        let service = match d.snapshot.as_ref().map(|s| &s.daemon) {
            Some(DaemonState::Running {
                ..
            })
            | None => Ok(()),
            Some(DaemonState::Stopped) => Err("Service off".to_owned()),
            Some(DaemonState::Unknown(_)) => Err("Service unreachable".to_owned()),
        };
        let mut chips = div().flex().flex_wrap().items_center().gap(px(8.0));
        chips = if d.paired {
            chips.child(chip(Some(Icon::Smartphone), format!("{} approves", d.phone()), pal.success))
        } else {
            chips.child(chip(Some(Icon::Unlink), "Not paired", pal.danger))
        };
        chips = match service {
            Ok(()) => chips.child(chip(Some(Icon::Server), "Service on", pal.success)),
            Err(why) => chips.child(chip(Some(Icon::Server), why, pal.danger)).child(
                btn("start-service", "Start", Btn::Accent, pal)
                    .h(px(26.0))
                    .on_click(Self::on_model(cx, Model::start_service)),
            ),
        };
        let initial = account.chars().next().map(|c| c.to_uppercase().to_string()).unwrap_or_default();
        div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(14.0))
            .p(px(16.0))
            .rounded(px(20.0))
            .bg(pal.paper_raised())
            .border_1()
            .border_color(pal.rule())
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .size(px(46.0))
                    .rounded_full()
                    .bg(pal.paper_sunken())
                    .font_family(crate::theme::SERIF)
                    .text_size(px(24.0))
                    .text_color(pal.ink())
                    .child(initial),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w(px(200.0))
                    .gap(px(2.0))
                    .child(div().text_size(px(15.5)).font_weight(FontWeight::SEMIBOLD).truncate().child(account))
                    .child(div().font_family(MONO).text_size(px(12.0)).text_color(pal.ink_2()).truncate().child(line)),
            )
            .child(chips)
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_keyboard_card_folds_the_sections_into_one_line() {
        let lines = keyboard_lines();
        assert_eq!(lines[0].0, "Home … Settings");
        assert!(lines[0].1.contains('–'));
        assert_eq!(lines.len(), shortcuts::list().len() - Section::ALL.len() + 1);
    }
}
