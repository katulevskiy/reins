//! Settings (`Desk-Settings.png`): who this computer is paired as (the account, the server and this computer's key,
//! which phone approves, the service), then two columns: This computer (open at login, the command line tool,
//! notifications, the health checks, Appearance) and Keyboard and Account (disconnect, held to confirm; quit); the
//! version and the links at the bottom.

use std::time::Instant;

use gpui::prelude::FluentBuilder as _;
use gpui::{
    Anchor, AnyElement, Context, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _, SharedString,
    StatefulInteractiveElement as _, Styled as _, div, px,
};

use super::hold::Held;
use super::icons::{Glyph, glyph};
use super::parts::{caption, this_computer};
use super::{Data, Root, kit};
use crate::backend::{DaemonState, Setting};
use crate::format::host;
use crate::health;
use crate::model::{Model, Section};
use crate::shortcuts;
use crate::theme::{Appearance, MONO, NextText as _, Palette};

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
    /// Settings (`DeskNext-Settings.png`): who this computer is paired as, then two columns of panes: this computer
    /// and its look; the keyboard, and Disconnect, held, as the quiet last row.
    pub(super) fn settings(&self, d: &Data, pal: Palette, cx: &mut Context<'_, Self>) -> AnyElement {
        if self.wallpaper_page {
            return self.wallpaper_page(d, pal, cx);
        }
        let glass = kit::Glass::Solid;
        let mut page = div()
            .flex()
            .flex_col()
            .child(div().title().text_color(pal.ink()).child("Settings"))
            .child(div().mt(px(20.0)).child(Self::account_card(d, pal, cx)));
        page = page.when_some(d.notice.clone(), |p, n| p.child(caption(n, pal).mt(px(12.0)).text_color(pal.danger)));
        page = page.when_some(d.note.clone(), |p, n| p.child(caption(n, pal).mt(px(12.0))));

        // This computer.
        let tile = |g: Glyph| kit::pane::tile(g, pal);
        let value = |t: String| kit::pane::row_value(t, pal);
        let autostart = d.autostart;
        let notify = d.config().map(|c| c.notify.clone()).unwrap_or_default();
        let (phone, sound) = (notify.phone, notify.sound);
        let notify_word = match (phone, sound) {
            (true, true) => "With sound",
            (true, false) => "On",
            (false, _) => "Off",
        };
        let health_word = match (&d.health.checks, &d.health.error) {
            (_, Some(_)) => "Can't check".to_owned(),
            (Some(c), None) => health::summary(c).title,
            (None, None) => "Not checked".to_owned(),
        };
        let (name, itself) = name_lines(&d.shown_as, d.phone_name.as_deref());
        let mut rows: Vec<AnyElement> = Vec::new();
        let mut name_block = div().flex().flex_col().child(
            kit::pane::clickable(kit::pane_row(), "name", pal)
                .child(tile(Glyph::Laptop))
                .child(kit::title_sub("Name", itself.map(|i| div().child(i).into_any_element()), pal))
                .child(value(name))
                .child(glyph(
                    if self.name_open {
                        Glyph::ChevronDown
                    } else {
                        Glyph::ChevronRight
                    },
                    14.0,
                    pal.ink_faint(),
                ))
                .on_click(Self::on_view(cx, |this, _| this.name_open = !this.name_open)),
        );
        if self.name_open {
            // The phone keeps the name (synced to the user's phones, sealed); a name typed here could be anyone's on
            // this computer, so the row points to the phone.
            name_block = name_block.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .pl(px(kit::pane::INSET))
                    .pb(px(12.0))
                    .text_size(px(13.0))
                    .text_color(pal.ink_meta())
                    .child(glyph(Glyph::Phone, 13.0, pal.ink_meta()))
                    .child(format!("Rename on {}", d.phone())),
            );
        }
        rows.push(name_block.into_any_element());
        rows.push(
            kit::pane_row()
                .child(tile(Glyph::Power))
                .child(kit::title_sub("Open at login", None, pal))
                .child(kit::switch("autostart", autostart, pal).on_click(Self::on_model(cx, Model::toggle_autostart)))
                .into_any_element(),
        );
        rows.push(
            kit::pane_row()
                .child(tile(Glyph::Terminal))
                .child(kit::title_sub("Command line tool", None, pal))
                .child(if d.cli_installed {
                    div()
                        .font_family(MONO)
                        .text_size(px(13.0))
                        .text_color(pal.ink_2())
                        .child("~/.local/bin/reins")
                        .into_any_element()
                } else {
                    kit::button("cli", kit::Look::Soft, kit::Size::Small, None, "Install", pal)
                        .on_click(Self::on_model(cx, Model::install_cli))
                        .into_any_element()
                })
                .into_any_element(),
        );
        let mut notify_block = div().flex().flex_col().child(
            kit::pane::clickable(kit::pane_row(), "notify", pal)
                .child(tile(Glyph::Bell))
                .child(kit::title_sub("Notifications", None, pal))
                .child(value(notify_word.to_owned()))
                .child(glyph(
                    if self.notify_open {
                        Glyph::ChevronDown
                    } else {
                        Glyph::ChevronRight
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
                    .min_h(px(44.0))
                    .pl(px(kit::pane::INSET))
                    .pr(px(14.0))
                    .text_size(px(14.5))
                    .text_color(pal.ink())
                    .child(label)
                    .child(kit::switch(SharedString::from(format!("{id}-switch")), on, pal))
                    .when(!enabled, |r| r.opacity(0.4))
            };
            notify_block = notify_block
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
        rows.push(notify_block.into_any_element());
        rows.push(
            kit::pane::clickable(kit::pane_row(), "health-row", pal)
                .child(tile(Glyph::Heart))
                .child(kit::title_sub("Health checks", None, pal))
                .child(value(health_word))
                .child(kit::pane::chevron(pal))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.health_open = true;
                    this.model.update(cx, |m, cx| m.show_section(Section::Home, cx));
                    cx.notify();
                }))
                .into_any_element(),
        );
        let computer = kit::group(rows, kit::pane::INSET, glass, pal);

        // Look.
        let (dark_w, light_w) = (d.saved.wallpaper(true), d.saved.wallpaper(false));
        let thumb = |w: kit::Wallpaper| {
            div()
                .flex_none()
                .w(px(26.0))
                .h(px(34.0))
                .rounded(px(6.0))
                .overflow_hidden()
                .border_1()
                .border_color(pal.rule())
                .child(kit::wallpaper::art(w, 0.5, 0.0).corners(6.0).thumbnail())
        };
        let grain_on = !d.saved.flat_fields;
        let look = kit::group(
            [
                kit::pane_row()
                    .child(tile(Glyph::Appearance))
                    .child(kit::title_sub("Appearance", None, pal))
                    .child(self.appearance_menu(d.saved.appearance, pal, cx))
                    .into_any_element(),
                kit::pane::clickable(kit::pane_row(), "wallpaper-row", pal)
                    .child(tile(Glyph::Appearance))
                    .child(kit::title_sub("Wallpaper", None, pal))
                    .child(value(format!("{} · {}", dark_w.label(), light_w.label())))
                    .child(thumb(dark_w))
                    .child(thumb(light_w))
                    .child(kit::pane::chevron(pal))
                    .on_click(Self::on_view(cx, |this, _| this.wallpaper_page = true))
                    .into_any_element(),
                kit::pane_row()
                    .child(tile(Glyph::Appearance))
                    .child(kit::title_sub("Grain on fields", None, pal))
                    .child(
                        kit::switch("grain", grain_on, pal)
                            .on_click(Self::on_model(cx, move |m, cx| m.set_grain(!grain_on, cx))),
                    )
                    .into_any_element(),
            ],
            kit::pane::INSET,
            glass,
            pal,
        );

        // Keyboard.
        // The board's short names: "Sections", "Close" (and what closing does).
        let keys = keyboard_lines()
            .into_iter()
            .enumerate()
            .map(|(i, (what, k))| match what.as_str() {
                _ if i == 0 => ("Sections".to_owned(), k),
                "Close window, keep Reins on" => ("Close".to_owned(), format!("{k}, Reins keeps running")),
                "Close what is open" => ("Back".to_owned(), k),
                _ => (what, k),
            })
            .chain(std::iter::once(("Quit".to_owned(), shortcuts::hint("q"))))
            .map(|(what, k)| {
                div()
                    .flex()
                    .items_center()
                    .gap(px(12.0))
                    .min_h(px(50.0))
                    .px(px(16.0))
                    .child(
                        div()
                            .w(px(96.0))
                            .flex_none()
                            .text_size(px(13.5))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(pal.ink_meta())
                            .child(what),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.0))
                            .truncate()
                            .font_family(MONO)
                            .text_size(px(13.5))
                            .text_color(pal.ink())
                            .child(k),
                    )
                    .into_any_element()
            })
            .collect::<Vec<_>>();
        let keys = kit::group(keys, 16.0, glass, pal);

        // Disconnect, held; or pair again.
        let disconnect: AnyElement = if d.paired {
            self.hold_row(
                "disconnect",
                Held::Disconnect,
                Glyph::Unlink,
                format!("Disconnect {}", this_computer()),
                "Hold to disconnect. git and keys stop going through Reins.",
                pal,
                cx,
            )
        } else {
            kit::button("pair-again", kit::Look::Inverse, kit::Size::Regular, Some(Glyph::Phone), "Pair again", pal)
                .w_full()
                .on_click(Self::on_model(cx, Model::sign_out))
                .into_any_element()
        };

        let left = div()
            .flex()
            .flex_col()
            .child(kit::section(Some(Glyph::Laptop), this_computer_label(), None, pal))
            .child(computer)
            .child(kit::section(Some(Glyph::Appearance), "Look", None, pal))
            .child(look);
        let right = div()
            .flex()
            .flex_col()
            .child(kit::section(Some(Glyph::Terminal), "Keyboard", None, pal))
            .child(keys)
            .child(div().mt(px(18.0)).child(disconnect));
        page = page.child(super::connections::two_columns(left, right));

        // The version and the links.
        let footer_link = |id: &'static str, label: &'static str, url: String| {
            div()
                .id(id)
                .cursor_pointer()
                .text_color(pal.ink_meta())
                .hover(|s| s.text_color(pal.ink()))
                .child(label)
                .on_click(move |_, _, cx| cx.open_url(&url))
        };
        page = page.child(
            div()
                .flex()
                .gap(px(5.0))
                .pt(px(22.0))
                .px(px(6.0))
                .text_size(px(12.5))
                .text_color(pal.ink_faint())
                .child(footer_link("downloads", "Downloads", crate::links::download()))
                .child("·")
                .child(footer_link("terms", "Terms", crate::links::TERMS.to_owned()))
                .child("·")
                .child(footer_link("privacy", "Privacy", crate::links::PRIVACY.to_owned())),
        );
        page.into_any_element()
    }

    /// Settings → Wallpaper (`DeskNext-Settings.png` 02): the set for dark or for light paper, a preview, and Frosted
    /// panes.
    fn wallpaper_page(&self, d: &Data, pal: Palette, cx: &mut Context<'_, Self>) -> AnyElement {
        let dark = self.wallpaper_dark_slot;
        let current = d.saved.wallpaper(dark);
        let back = kit::icon_button("wallpaper-back", Glyph::ChevronLeft, 36.0, pal)
            .on_click(Self::on_view(cx, |this, _| this.wallpaper_page = false));
        let head = div().flex().items_center().gap(px(14.0)).child(back).child(
            div().flex().flex_col().child(div().title().text_color(pal.ink()).child("Wallpaper")).child(
                div()
                    .mt(px(6.0))
                    .mood()
                    .text_color(pal.ink_2())
                    .child("One for dark, one for light. Shown behind Home and Activity."),
            ),
        );
        let mut slots = kit::segmented(pal).w_full();
        for (i, (label, g, is_dark)) in
            [("Dark", Glyph::Moon, true), ("Light", Glyph::Appearance, false)].into_iter().enumerate()
        {
            slots = slots.child(
                kit::segment(("wall-slot", i), "", dark == is_dark, pal)
                    .child(glyph(
                        g,
                        14.0,
                        if dark == is_dark {
                            pal.ink()
                        } else {
                            pal.ink_2()
                        },
                    ))
                    .child(label)
                    .on_click(Self::on_view(cx, move |this, _| this.wallpaper_dark_slot = is_dark)),
            );
        }
        let mut grid = div().flex().flex_wrap().gap(px(14.0)).mt(px(16.0));
        for w in kit::Wallpaper::ALL {
            let on = w == current;
            grid = grid.child(
                div()
                    .id(SharedString::from(format!("wallpaper-{}", w.id())))
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(px(8.0))
                    .w(px(130.0))
                    .cursor_pointer()
                    .child(
                        div()
                            .relative()
                            .w(px(130.0))
                            .h(px(100.0))
                            .rounded(px(14.0))
                            .when_else(
                                on,
                                |t| t.border_2().border_color(pal.violet()),
                                |t| t.border_1().border_color(pal.rule()),
                            )
                            .child(kit::wallpaper::art(w, 0.5, 0.0).corners(13.0).thumbnail())
                            .when(on, |t| {
                                t.child(
                                    div()
                                        .absolute()
                                        .right(px(6.0))
                                        .bottom(px(6.0))
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .size(px(20.0))
                                        .rounded_full()
                                        .bg(pal.violet_fill())
                                        .child(glyph(Glyph::Check, 12.0, gpui::white())),
                                )
                            }),
                    )
                    .child(div().text_size(px(13.5)).text_color(pal.ink_2()).child(w.label()))
                    .on_click(Self::on_model(cx, move |m, cx| m.set_wallpaper(dark, w, cx))),
            );
        }
        let frosted = !d.saved.solid_panes;
        let preview_pal = if dark {
            Palette::dark()
        } else {
            Palette::light()
        };
        let preview = div()
            .relative()
            .h(px(300.0))
            .rounded(px(20.0))
            .overflow_hidden()
            .bg(preview_pal.paper())
            .child(kit::wallpaper::band(current, 300.0, preview_pal))
            .child(
                div().absolute().left(px(24.0)).right(px(24.0)).bottom(px(24.0)).child(kit::pane(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(10.0))
                        .px(px(14.0))
                        .py(px(14.0))
                        .text_size(px(15.0))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(preview_pal.ink())
                        .child(glyph(Glyph::ShieldCheck, 18.0, preview_pal.success))
                        .child("All quiet"),
                    kit::Glass::over(true, frosted),
                    16.0,
                    preview_pal,
                )),
            );
        let toggle = kit::group(
            [kit::pane_row()
                .child(kit::pane::tile(Glyph::Appearance, pal))
                .child(kit::title_sub("Frosted panes", None, pal))
                .child(
                    kit::switch("frosted", frosted, pal)
                        .on_click(Self::on_model(cx, move |m, cx| m.set_frosted_panes(!frosted, cx))),
                )
                .into_any_element()],
            kit::pane::INSET,
            kit::Glass::Solid,
            pal,
        );
        let os = if cfg!(target_os = "macos") {
            "With Reduce Transparency on in macOS, panes turn solid."
        } else if cfg!(windows) {
            "With transparency effects off in Windows, panes turn solid."
        } else {
            "With reduced transparency on, panes turn solid."
        };
        let left = div().flex().flex_col().mt(px(20.0)).child(slots).child(grid);
        let right = div()
            .flex()
            .flex_col()
            .child(kit::section(None, "Preview", None, pal))
            .child(preview)
            .child(div().mt(px(14.0)).child(toggle))
            .child(div().px(px(6.0)).pt(px(8.0)).text_size(px(13.5)).text_color(pal.ink_meta()).child(os));
        div().flex().flex_col().child(head).child(super::connections::two_columns(left, right)).into_any_element()
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
        let word = |g: Glyph, text: String, color: gpui::Hsla| {
            div()
                .flex()
                .items_center()
                .gap(px(5.0))
                .text_size(px(13.5))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(color)
                .child(glyph(g, 14.0, color))
                .child(text)
        };
        let mut pills = div().flex().flex_wrap().items_center().gap(px(16.0));
        pills = if d.paired {
            pills.child(word(Glyph::ShieldCheck, format!("{} approves", d.phone()), pal.success))
        } else {
            pills.child(word(Glyph::Unlink, "Not paired".to_owned(), pal.danger))
        };
        pills = match service {
            Ok(()) => pills.child(word(Glyph::Server, "Service on".to_owned(), pal.success)),
            Err(why) => pills.child(word(Glyph::Server, why, pal.danger)).child(
                kit::button("start-service", kit::Look::Inverse, kit::Size::Small, None, "Start", pal)
                    .on_click(Self::on_model(cx, Model::start_service)),
            ),
        };
        let initial = account.chars().next().map(|c| c.to_uppercase().to_string()).unwrap_or_default();
        let mut line = line;
        if let Some(key) = key {
            line = format!("{line} · {key}");
        }
        let card = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(16.0))
            .px(px(20.0))
            .py(px(18.0))
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .size(px(48.0))
                    .rounded_full()
                    .bg(pal.success.opacity(0.16))
                    .text_size(px(20.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(pal.success)
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
                            .text_size(px(18.0))
                            .line_height(px(23.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(pal.ink())
                            .truncate()
                            .child(account),
                    )
                    // The key compared when pairing stays whole: the line wraps rather than cutting it.
                    .child(
                        div().mt(px(2.0)).font_family(MONO).text_size(px(13.0)).text_color(pal.ink_meta()).child(line),
                    ),
            )
            .child(pills);
        kit::pane(card, kit::Glass::Solid, kit::pane::RADIUS, pal)
    }
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
