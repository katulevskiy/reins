//! Settings (`Desk-Settings.png`): who this computer is paired as (the account, the server and this computer's key,
//! which phone approves, the service), then two columns: This computer (open at login, the command line tool,
//! notifications, the health checks, Appearance) and Keyboard and Account (disconnect, held to confirm; quit); the
//! version and the links at the bottom.

use std::time::Instant;

use gpui::prelude::FluentBuilder as _;
use gpui::{
    Anchor, AnyElement, Context, Div, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _,
    SharedString, StatefulInteractiveElement as _, Styled as _, div, px,
};

use super::health::checked;
use super::hold::Held;
use super::icons::{Glyph, glyph};
use super::kit::hold::HoldLook;
use super::parts::{caption, columns, list_row, mono, section, this_computer, title_meta};
use super::{Data, Root, kit};
use crate::backend::{DaemonState, Setting};
use crate::format::host;
use crate::health;
use crate::model::{Model, Section};
use crate::shortcuts;
use crate::theme::{Appearance, MONO, Palette, DISPLAY, NextText as _, Tone};

/// The keyboard card's lines: the sections in one line ("Home … Settings", "⌘1 – ⌘6"), then the rest but Quit.
#[must_use]
pub fn keyboard_lines() -> Vec<(String, String)> {
    let list = shortcuts::list();
    let n = Section::ALL.len();
    let mut out = vec![(
        format!("{} … {}", Section::ALL[0].label(), Section::ALL[n - 1].label()),
        format!("{} – {}", list[0].0, list[n - 1].0),
    )];
    // Quit is under Account, with its key.
    let quit = shortcuts::hint("q");
    out.extend(list[n..].iter().filter(|(keys, _)| *keys != quit).map(|(keys, what)| (what.clone(), keys.clone())));
    out
}

/// What the Appearance row says under its title.
#[must_use]
pub fn appearance_line(a: Appearance) -> &'static str {
    match a {
        Appearance::System if cfg!(target_os = "macos") => "Follows macOS",
        Appearance::System if cfg!(windows) => "Follows Windows",
        Appearance::System => "Follows the system",
        Appearance::Light => "Light paper, always",
        Appearance::Dark => "Dark paper, always",
    }
}

/// The Name row's lines: the name the phone shows for this computer (the one the user gave it there, else its own),
/// and, when the user gave it one, what it calls itself, so that a name never hides which computer this is.
#[must_use]
pub fn name_lines(own: &str, given: Option<&str>) -> (String, Option<String>) {
    match given.filter(|g| *g != own) {
        Some(g) => (g.to_owned(), Some(format!("Calls itself {own}"))),
        None => (own.to_owned(), None),
    }
}

/// "This Mac", "This computer": the section's label.
fn this_computer_label() -> &'static str {
    if cfg!(target_os = "macos") {
        "This Mac"
    } else {
        "This computer"
    }
}

impl Root {
    pub(super) fn settings(&self, d: &Data, pal: Palette, cx: &mut Context<'_, Self>) -> AnyElement {
        let mut page = div()
            .flex()
            .flex_col()
            .gap(px(24.0))
            .child(div().title().text_color(pal.ink()).child("Settings"))
            .child(Self::account_card(d, pal, cx));
        page = page.when_some(d.notice.clone(), |p, n| p.child(caption(n, pal).text_color(pal.danger)));
        page = page.when_some(d.note.clone(), |p, n| p.child(caption(n, pal)));

        // This computer.
        let tile = |g: Glyph| kit::kind_tile(g, Tone::Neutral, 32.0, pal);
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
        let shield = if cfg!(target_os = "macos") {
            "The shield in the menu bar shows Reins is on"
        } else {
            "The shield in the tray shows Reins is on"
        };
        let (name, itself) = name_lines(&d.shown_as, d.phone_name.as_deref());
        let name_meta = div()
            .flex()
            .gap(px(6.0))
            .min_w(px(0.0))
            .child(div().truncate().child(name.clone()))
            .when_some(itself, |m, i| m.child(div().text_color(pal.ink_faint()).truncate().child(i)));
        let mut computer = div().flex().flex_col().child(
            list_row(pal, self.name_open)
                .id("name")
                .cursor_pointer()
                .child(tile(Glyph::Laptop))
                .child(title_meta("Name", Some(name_meta.into_any_element()), pal))
                .child(glyph(
                    if self.name_open {
                        Glyph::ChevronUp
                    } else {
                        Glyph::ChevronDown
                    },
                    14.0,
                    pal.ink_faint(),
                ))
                .on_click(Self::on_view(cx, |this, _| this.name_open = !this.name_open)),
        );
        if self.name_open {
            // The phone keeps the name (synced to the user's phones, sealed); a name typed here could be anyone's on
            // this computer, so the field shows it and points to the phone.
            computer = computer.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(8.0))
                    .pl(px(44.0))
                    .pb(px(12.0))
                    .border_b_1()
                    .border_color(pal.rule())
                    .child(
                        div()
                            .id("name-field")
                            .h(px(36.0))
                            .px(px(12.0))
                            .flex()
                            .items_center()
                            .rounded(px(10.0))
                            .border_1()
                            .border_color(pal.rule())
                            .bg(pal.paper_sunken())
                            .text_size(px(14.0))
                            .text_color(pal.ink())
                            .truncate()
                            .child(name),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(6.0))
                            .text_size(px(12.5))
                            .text_color(pal.ink_meta())
                            .child(glyph(Glyph::Phone, 13.0, pal.ink_meta()))
                            .child(format!("Rename on {}", d.phone())),
                    ),
            );
        }
        computer = computer
            .child(
                list_row(pal, false)
                    .child(tile(Glyph::Power))
                    .child(title_meta("Open at login", Some(shield.into_any_element()), pal))
                    .child(
                        kit::switch("autostart", autostart, pal).on_click(Self::on_model(cx, Model::toggle_autostart)),
                    ),
            )
            .child(
                list_row(pal, false)
                    .child(tile(Glyph::Terminal))
                    .child(title_meta(
                        "Command line tool",
                        Some(mono("~/.local/bin/reins", 12.0).into_any_element()),
                        pal,
                    ))
                    .child(if d.cli_installed {
                        kit::status_pill(None, "Installed", Some(pal.success), pal).into_any_element()
                    } else {
                        kit::button("cli", kit::Look::Soft, kit::Size::Small, None, "Install", pal)
                            .h(px(34.0))
                            .on_click(Self::on_model(cx, Model::install_cli))
                            .into_any_element()
                    }),
            )
            .child(
                list_row(pal, self.notify_open)
                    .id("notify")
                    .cursor_pointer()
                    .child(tile(Glyph::Bell))
                    .child(title_meta("Notifications", Some(notify_word.into_any_element()), pal))
                    .child(glyph(
                        if self.notify_open {
                            Glyph::ChevronUp
                        } else {
                            Glyph::ChevronDown
                        },
                        14.0,
                        pal.ink_faint(),
                    ))
                    .on_click(Self::on_view(cx, |this, _| this.notify_open = !this.notify_open)),
            );
        if self.notify_open {
            let sub = |id: &'static str, label: &'static str, on: bool, enabled: bool| {
                div()
                    .id(id)
                    .flex()
                    .items_center()
                    .justify_between()
                    .py(px(8.0))
                    .text_size(px(14.0))
                    .text_color(pal.ink())
                    .child(label)
                    .child(kit::switch(SharedString::from(format!("{id}-switch")), on, pal))
                    .when(!enabled, |r| r.opacity(0.4))
            };
            computer = computer.child(
                div()
                    .flex()
                    .flex_col()
                    .pl(px(44.0))
                    .pb(px(8.0))
                    .border_b_1()
                    .border_color(pal.rule())
                    .child(sub("notify-phone", "Notify me here", phone, true).cursor_pointer().on_click(
                        Self::on_model(cx, move |m, cx| {
                            m.change(
                                Setting::Notify {
                                    phone: !phone,
                                    sound,
                                },
                                cx,
                            );
                        }),
                    ))
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
                    }),
            );
        }
        computer = computer
            .child(
                list_row(pal, false)
                    .child(tile(Glyph::Heart))
                    .child(title_meta("Health checks", Some(health_line.into_any_element()), pal))
                    .child(Self::check_again("run-checks", "Run", d, pal, cx)),
            )
            .child(
                list_row(pal, false)
                    .child(tile(Glyph::Appearance))
                    .child(title_meta("Appearance", Some(appearance_line(d.saved.appearance).into_any_element()), pal))
                    .child(self.appearance_menu(d.saved.appearance, pal, cx)),
            );

        // Keyboard.
        let lines = keyboard_lines();
        let mut keys = div().flex().flex_col();
        for (what, k) in &lines {
            keys = keys.child(
                list_row(pal, false)
                    .child(div().flex_1().min_w(px(0.0)).truncate().text_color(pal.ink()).child(what.clone()))
                    .child(kit::key_cap(k.clone(), pal)),
            );
        }

        // Account.
        let disconnect: AnyElement = if d.paired {
            let look = self.hold_progress(Held::Disconnect).map_or(HoldLook::Idle, HoldLook::Filling);
            Self::hold_area(
                kit::hold_button(
                    "disconnect",
                    Glyph::Unlink,
                    format!("Hold to disconnect {}", this_computer()),
                    "Disconnected",
                    look,
                    48.0,
                    pal,
                )
                .w_full()
                .mt(px(8.0))
                .text_size(px(14.5)),
                Held::Disconnect,
                cx,
            )
            .into_any_element()
        } else {
            kit::button("pair-again", kit::Look::Inverse, kit::Size::Regular, Some(Glyph::Phone), "Pair again", pal)
                .w_full()
                .mt(px(8.0))
                .on_click(Self::on_model(cx, Model::sign_out))
                .into_any_element()
        };
        let account = div().flex().flex_col().child(disconnect).child(
            list_row(pal, true)
                .id("quit")
                .cursor_pointer()
                .child(tile(Glyph::Quit))
                .child(title_meta("Quit Reins", Some("The service keeps running".into_any_element()), pal))
                .child(kit::key_cap(shortcuts::hint("q"), pal))
                .on_click(Self::on_model(cx, Model::quit)),
        );

        page = page.child(columns(
            div().flex().flex_col().child(section(this_computer_label(), None, computer, pal)),
            div()
                .flex()
                .flex_col()
                .gap(px(24.0))
                .child(section("Keyboard", None, keys, pal))
                .child(section("Account", None, account, pal)),
            380.0,
        ));

        // The version and the links.
        let footer_link = |id: &'static str, label: &'static str, url: String| {
            div()
                .id(id)
                .cursor_pointer()
                .text_color(pal.ink_2())
                .underline()
                .hover(|s| s.text_color(pal.ink()))
                .child(label)
                .on_click(move |_, _, cx| cx.open_url(&url))
        };
        page = page.child(
            div()
                .flex()
                .gap(px(5.0))
                .pt(px(12.0))
                .text_size(px(12.5))
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

    /// Appearance's value and the menu it opens: System, Light, Dark.
    fn appearance_menu(&self, picked: Appearance, pal: Palette, cx: &mut Context<'_, Self>) -> AnyElement {
        let open = self.appearance_open;
        let trigger = div()
            .id("appearance")
            .relative()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(4.0))
            .h(px(32.0))
            .px(px(8.0))
            .rounded(px(10.0))
            .cursor_pointer()
            .text_size(px(14.0))
            .text_color(pal.ink())
            .hover(|s| s.bg(pal.paper_sunken()))
            .active(|s| s.opacity(kit::button::PRESSED))
            .child(picked.label())
            .child(glyph(
                if open {
                    Glyph::ChevronUp
                } else {
                    Glyph::ChevronDown
                },
                14.0,
                pal.ink_2(),
            ))
            .on_click(Self::on_view(cx, |this, _| {
                let just_closed = this.menu_closed.is_some_and(|at| at.elapsed() < super::activity::REOPEN_GUARD);
                this.appearance_open = !this.appearance_open && !just_closed;
            }));
        if !open {
            return trigger.into_any_element();
        }
        let mut menu = kit::menu(pal).w(px(250.0));
        for a in Appearance::ALL {
            let g = match a {
                Appearance::System => Glyph::Appearance,
                Appearance::Light => Glyph::Star,
                Appearance::Dark => Glyph::Moon,
            };
            menu = menu.child(
                kit::menu_item(
                    ("appearance-item", a as usize),
                    g,
                    (pal.paper_sunken(), pal.ink()),
                    a.label(),
                    Some(appearance_line(a).into()),
                    a == picked,
                    false,
                    pal,
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.appearance_open = false;
                    this.model.update(cx, |m, cx| m.set_appearance(a, cx));
                    cx.notify();
                })),
            );
        }
        let menu = menu.on_mouse_down_out(cx.listener(|this, _, _, cx| {
            this.appearance_open = false;
            this.menu_closed = Some(Instant::now());
            cx.notify();
        }));
        trigger.child(kit::open_menu("appearance-menu", menu, Anchor::TopRight)).into_any_element()
    }

    /// Who this computer is paired as (the account, and the name the phone shows for it), the server and its key,
    /// which phone approves, and the service; a hairline under it.
    fn account_card(d: &Data, pal: Palette, cx: &mut Context<'_, Self>) -> AnyElement {
        let account = d.saved.account.clone().unwrap_or_else(|| host(&d.server));
        // The name the phone knows this computer by and the server; under them, on a line of its own that is never
        // cut, the key compared when pairing.
        let line = format!("{} · {}", name_lines(&d.shown_as, d.phone_name.as_deref()).0, host(&d.server));
        let key = d.fingerprint.as_ref().map(|fp| format!("key {fp}"));
        let service = match d.snapshot.as_ref().map(|s| &s.daemon) {
            Some(DaemonState::Running {
                ..
            })
            | None => Ok(()),
            Some(DaemonState::Stopped) => Err("Service off".to_owned()),
            Some(DaemonState::Unknown(_)) => Err("Service unreachable".to_owned()),
        };
        let mut pills = div().flex().flex_wrap().items_center().gap(px(16.0));
        pills = if d.paired {
            pills.child(kit::status_pill(Some(Glyph::Phone), format!("{} approves", d.phone()), Some(pal.success), pal))
        } else {
            pills.child(kit::status_pill(Some(Glyph::Unlink), "Not paired", Some(pal.danger), pal))
        };
        pills = match service {
            Ok(()) => pills.child(kit::status_pill(Some(Glyph::Server), "Service on", Some(pal.success), pal)),
            Err(why) => pills.child(kit::status_pill(Some(Glyph::Server), why, Some(pal.danger), pal)).child(
                kit::button("start-service", kit::Look::Inverse, kit::Size::Small, None, "Start", pal)
                    .on_click(Self::on_model(cx, Model::start_service)),
            ),
        };
        let initial = account.chars().next().map(|c| c.to_uppercase().to_string()).unwrap_or_default();
        div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(16.0))
            .pb(px(20.0))
            .border_b_1()
            .border_color(pal.rule())
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .size(px(56.0))
                    .rounded_full()
                    .bg(pal.paper_sunken())
                    .font_family(DISPLAY)
                    .text_size(px(30.0))
                    .text_color(pal.ink())
                    .child(initial),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w(px(220.0))
                    .child(
                        div()
                            .text_size(px(17.0))
                            .line_height(px(22.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(pal.ink())
                            .truncate()
                            .child(account),
                    )
                    .child(mono(line, 12.5).text_color(pal.ink_meta()))
                    .when_some(key, |col, key| col.child(fingerprint_line(key, pal))),
            )
            .child(pills)
            .into_any_element()
    }
}

/// The key's eight digits as the phone shows them, whole at any width: they wrap rather than being cut.
fn fingerprint_line(key: String, pal: Palette) -> Div {
    div().flex_none().font_family(MONO).text_size(px(12.5)).text_color(pal.ink_meta()).child(key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_name_is_the_phones_and_what_it_calls_itself_stays_in_sight() {
        assert_eq!(name_lines("M2A", None), ("M2A".to_owned(), None));
        assert_eq!(name_lines("M2A", Some("M2A")), ("M2A".to_owned(), None));
        assert_eq!(name_lines("M2A", Some("Work Mac")), ("Work Mac".to_owned(), Some("Calls itself M2A".to_owned())));
    }

    #[test]
    fn the_keyboard_card_folds_the_sections_into_one_line() {
        let lines = keyboard_lines();
        assert_eq!(lines[0].0, "Home … Settings");
        assert!(lines[0].1.contains('–'));
        assert_eq!(lines.len(), shortcuts::list().len() - Section::ALL.len());
        assert!(lines.iter().all(|(what, _)| what != "Quit Reins"));
    }

    #[test]
    fn appearance_says_what_it_follows() {
        assert!(appearance_line(Appearance::System).starts_with("Follows"));
        assert_ne!(appearance_line(Appearance::Light), appearance_line(Appearance::Dark));
    }
}
