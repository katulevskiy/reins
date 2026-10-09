//! Settings: notifications, opening at login, the command line tool, the account and pairing, the background service,
//! the version, and quitting.

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, Context, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, div, px,
};

use super::parts::{button, caption, card, fine, group, info_row, kbd, labelled, link, page_header, row, toggle};
use super::{Data, Root};
use crate::backend::{DaemonState, Setting};
use crate::format::host;
use crate::model::Model;
use crate::shortcuts;
use crate::theme::{MONO, Palette};

impl Root {
    pub(super) fn settings(d: &Data, pal: Palette, cx: &mut Context<'_, Self>) -> AnyElement {
        let mut page = div().flex().flex_col().gap(px(22.0)).child(page_header(
            "Settings",
            "How Reins behaves on this computer, and the phone it answers to.",
            pal,
        ));
        page = page.when_some(d.notice.clone(), |p, n| p.child(caption(n, pal).text_color(pal.danger)));
        page = page.when_some(d.note.clone(), |p, n| p.child(caption(n, pal)));

        // Notifications.
        let notify = d.config().map(|c| c.notify.clone()).unwrap_or_default();
        let (phone, sound) = (notify.phone, notify.sound);
        page = page.child(group(
            "Notifications",
            None,
            card(pal)
                .child(
                    row(pal, true)
                        .id("notify-phone")
                        .cursor_pointer()
                        .child(labelled(
                            "Notify me on this computer when my phone needs me",
                            Some("A notification here whenever a request waits for your phone".to_owned()),
                            pal,
                        ))
                        .child(toggle(phone, pal))
                        .on_click(Self::on_model(cx, move |m, cx| {
                            m.change(
                                Setting::Notify {
                                    phone: !phone,
                                    sound,
                                },
                                cx,
                            );
                        })),
                )
                .child(
                    row(pal, false)
                        .id("notify-sound")
                        .child(labelled("Play a sound", Some("A short chime with the notification".to_owned()), pal))
                        .child(toggle(sound && phone, pal))
                        .when_else(
                            phone,
                            |r| {
                                r.cursor_pointer().on_click(Self::on_model(cx, move |m, cx| {
                                    m.change(
                                        Setting::Notify {
                                            phone,
                                            sound: !sound,
                                        },
                                        cx,
                                    );
                                }))
                            },
                            |r| r.opacity(0.5),
                        ),
                ),
            pal,
        ));

        // This computer.
        let autostart = d.autostart;
        page = page.child(group(
            "This computer",
            None,
            card(pal)
                .child(
                    row(pal, true)
                        .id("autostart")
                        .cursor_pointer()
                        .child(labelled(
                            "Open Reins when you log in",
                            Some("The shield in the menu bar or tray shows that Reins is on".to_owned()),
                            pal,
                        ))
                        .child(toggle(autostart, pal))
                        .on_click(Self::on_model(cx, Model::toggle_autostart)),
                )
                .child(
                    row(pal, false)
                        .child(labelled(
                            "Command line tool",
                            Some("reins in ~/.local/bin, for reins run and your scripts".to_owned()),
                            pal,
                        ))
                        .child(
                            button("cli", "Install", pal, false, true).on_click(Self::on_model(cx, Model::install_cli)),
                        ),
                )
                .child(Self::service_row(d, pal, cx))
                .child(info_row("git", Self::git_line(d), pal, false)),
            pal,
        ));

        // Account.
        let account = d.saved.account.clone().unwrap_or_else(|| host(&d.server));
        let phone_name = d.saved.phone.clone().unwrap_or_else(|| "Your phone".to_owned());
        let mut acct = card(pal)
            .child(info_row("Account", account, pal, true))
            .child(info_row("Approves", phone_name, pal, false))
            .child(info_row("Server", host(&d.server), pal, false));
        if let Some(fp) = d.fingerprint.clone() {
            acct = acct.child(
                row(pal, false)
                    .justify_between()
                    .child(div().flex_none().text_color(pal.secondary).child("This computer's key"))
                    .child(div().font_family(MONO).font_weight(FontWeight::MEDIUM).child(fp)),
            );
        }
        acct = acct.child(
            row(pal, false)
                .child(
                    caption(
                        if d.paired {
                            "Disconnecting forgets the session here; remove the computer on your phone too."
                        } else {
                            "This computer is not paired with your phone."
                        },
                        pal,
                    )
                    .flex_1()
                    .min_w(px(0.0)),
                )
                .child(
                    button(
                        "sign-out",
                        if d.paired {
                            "Disconnect"
                        } else {
                            "Pair again"
                        },
                        pal,
                        !d.paired,
                        true,
                    )
                    .on_click(Self::on_model(cx, Model::sign_out)),
                ),
        );
        page = page.child(group("Account", None, acct, pal));

        // Health.
        page = page.child(group(
            "Health",
            Some("The checks `reins doctor` runs: pairing, the server, the clock, the service, git and your AI tools."),
            Self::checks_row(d, pal, cx),
            pal,
        ));

        // Keyboard shortcuts, two columns.
        let list = shortcuts::list();
        let half = list.len().div_ceil(2);
        let column = |items: &[(String, String)]| {
            let mut col = div().flex().flex_col().flex_1().min_w(px(220.0));
            for (i, (keys, what)) in items.iter().enumerate() {
                col = col.child(
                    row(pal, i == 0)
                        .py(px(8.0))
                        .child(div().flex_1().min_w(px(0.0)).text_color(pal.secondary).child(what.clone()))
                        .child(kbd(keys.clone(), pal)),
                );
            }
            col
        };
        page = page.child(group(
            "Keyboard shortcuts",
            None,
            card(pal)
                .flex_row()
                .flex_wrap()
                .child(column(&list[..half]))
                .child(div().w(px(1.0)).bg(pal.hairline))
                .child(column(&list[half..])),
            pal,
        ));

        // Version and quit.
        page = page.child(
            div()
                .flex()
                .items_center()
                .gap(px(14.0))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_w(px(0.0))
                        .child(caption(format!("Reins {}", reins_desktop::update::LONG_VERSION), pal))
                        .child(fine("Quitting hides the shield; the background service keeps running.", pal)),
                )
                .when(d.update.is_none(), |r| {
                    r.child(
                        link("download", "Downloads", pal).on_click(|_, _, cx| cx.open_url(&crate::links::download())),
                    )
                })
                .child(button("quit", "Quit Reins", pal, false, true).on_click(Self::on_model(cx, Model::quit))),
        );
        page.into_any_element()
    }

    fn service_row(d: &Data, pal: Palette, cx: &mut Context<'_, Self>) -> AnyElement {
        let (state, running) = match d.snapshot.as_ref().map(|s| &s.daemon) {
            Some(DaemonState::Running {
                pending: 0,
            }) => ("Running".to_owned(), true),
            Some(DaemonState::Running {
                pending,
            }) => (format!("Running, {pending} waiting for you here"), true),
            Some(DaemonState::Stopped) => ("Not running".to_owned(), false),
            Some(DaemonState::Unknown(e)) => (e.clone(), false),
            None => ("Checking…".to_owned(), true),
        };
        let r = row(pal, false).child(labelled("Background service", Some(state), pal));
        if running {
            r.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .text_size(px(12.0))
                    .text_color(pal.success)
                    .child(div().size(px(7.0)).rounded_full().bg(pal.success))
                    .child("On"),
            )
            .into_any_element()
        } else {
            r.child(
                button("start-service", "Start", pal, true, true).on_click(Self::on_model(cx, Model::start_service)),
            )
            .into_any_element()
        }
    }

    fn git_line(d: &Data) -> String {
        match d.snapshot.as_ref() {
            Some(s) if !s.git_routed.is_empty() => format!("{} through Reins", s.git_routed.join(", ")),
            Some(_) if d.pause.on() => "Paused: straight to the hosts".to_owned(),
            Some(_) => "Straight to the hosts".to_owned(),
            None => String::new(),
        }
    }
}
