//! Step 1 of the welcome flow (`Desk-Setup.png`): pair this computer with the phone. One sentence, then the QR code
//! and its user code beside the three things to do (open Reins, scan, tap the number), and this computer's key to
//! compare, on a plate. The browser sign-in and another server are the quieter ways, as links under it.

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, Context, Div, IntoElement, ParentElement as _, StatefulInteractiveElement as _, Styled as _, Window,
    div, px,
};

use super::field::Field;
use super::icons::{Glyph, glyph};
use super::parts::{caption, help, quiet_link, waiting_dot, warning, welcome_title};
use super::story::{bold, plain};
use super::{Data, Root, kit};
use crate::format::host;
use crate::model::{Model, Pairing};
use crate::theme::{MONO, NextText as _, Palette, Tone};

/// The QR code's side, and the white frame around it (164 px in all, the board's).
const QR: f32 = 150.0;
const QR_FRAME: f32 = 12.0;

/// A step to do: its number in a small sunken circle.
fn number(n: usize, pal: Palette) -> Div {
    div()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(px(26.0))
        .rounded_full()
        .bg(pal.paper_sunken())
        .text_size(px(13.0))
        .text_color(pal.ink_2())
        .child(n.to_string())
}

/// One of the numbered things to do.
fn numbered(n: usize, what: impl IntoElement, pal: Palette) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(12.0))
        .min_h(px(28.0))
        .text_size(px(15.5))
        .text_color(pal.ink())
        .child(number(n, pal))
        .child(div().flex().items_center().gap(px(10.0)).min_w(px(0.0)).child(what))
}

/// "This Mac", "This computer".
fn this_computer_cap() -> &'static str {
    if cfg!(target_os = "macos") {
        "This Mac"
    } else {
        "This computer"
    }
}

/// This computer's key, to compare with the one the phone shows: a pane row with the digits in mono.
fn key_line(fingerprint: &str, pal: Palette) -> AnyElement {
    let row = kit::pane_row()
        .min_h(px(52.0))
        .child(
            div()
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .size(px(34.0))
                .rounded(px(10.0))
                .bg(pal.success.opacity(0.16))
                .child(glyph(Glyph::Fingerprint, 17.0, pal.success)),
        )
        .child(
            div()
                .flex_1()
                .min_w(px(0.0))
                .text_size(px(15.0))
                .font_weight(gpui::FontWeight::MEDIUM)
                .text_color(pal.ink())
                .child(format!("{}'s key", this_computer_cap())),
        )
        .child(
            div()
                .font_family(MONO)
                .text_size(px(20.0))
                .line_height(px(26.0))
                .text_color(pal.ink())
                .child(spaced(fingerprint)),
        );
    kit::pane(row, kit::Glass::Solid, kit::pane::RADIUS, pal)
}

/// The digits set a hair apart, as the board tracks them (GPUI has no letter-spacing): hair spaces between them.
#[must_use]
pub fn spaced(digits: &str) -> String {
    let mut out = String::new();
    for c in digits.chars() {
        if c != ' ' && out.chars().last().is_some_and(|p| p != ' ') {
            out.push('\u{200A}');
        }
        out.push(c);
    }
    out
}

/// A centred message where the code was (signing in through the browser, an error, approved).
fn centred() -> Div {
    div().flex().flex_col().items_center().justify_center().gap(px(12.0)).w_full().min_h(px(200.0)).py(px(12.0))
}

impl Root {
    pub(super) fn onboarding(
        &self,
        d: &Data,
        pal: Palette,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> AnyElement {
        let m = self.model.read(cx);
        let pairing = m.pairing.clone();
        let fingerprint = m.fingerprint.clone().unwrap_or_default();
        let show_server = m.show_server;
        let notice = m.notice.clone();
        let location = crate::backend::Backend::location_problem();
        let server = m.server();

        let body: AnyElement = match &pairing {
            Pairing::Starting | Pairing::Code(_) => {
                let code = match &pairing {
                    Pairing::Code(c) => Some(c.clone()),
                    _ => None,
                };
                // The QR code and the code under it.
                let qr = div()
                    .flex()
                    .flex_col()
                    .flex_none()
                    .items_center()
                    .gap(px(10.0))
                    .child(match &code {
                        Some(c) => div()
                            .p(px(if pal.dark {
                                QR_FRAME
                            } else {
                                0.0
                            }))
                            .rounded(px(14.0))
                            .bg(gpui::white())
                            .child(crate::qr::element(&c.qr_url, QR, gpui::black(), gpui::white())),
                        None => div()
                            .flex()
                            .items_center()
                            .justify_center()
                            .size(px(QR + 2.0 * QR_FRAME))
                            .rounded(px(18.0))
                            .bg(pal.paper_sunken())
                            .child(waiting_dot("getting-code", pal.violet(), 8.0)),
                    })
                    .child(
                        div()
                            .font_family(MONO)
                            .text_size(px(16.0))
                            .line_height(px(20.0))
                            .font_weight(gpui::FontWeight::MEDIUM)
                            .child(spaced(
                                &code.as_ref().map_or_else(|| "····-····".to_owned(), |c| c.user_code.clone()),
                            ))
                            .text_color(if code.is_some() {
                                pal.ink()
                            } else {
                                pal.ink_meta()
                            }),
                    );
                // "Tap [42] there", the number as the phone shows it.
                let tap: AnyElement = match code.as_ref().and_then(|c| c.confirm_code) {
                    Some(n) => div()
                        .flex()
                        .items_center()
                        .gap(px(10.0))
                        .child("Tap")
                        .child(div().font_weight(gpui::FontWeight::SEMIBOLD).child(n.to_string()))
                        .child("there")
                        .child(help(
                            "pair-number",
                            "The number makes sure you pair this computer and no other.",
                            d,
                            pal,
                            cx,
                        ))
                        .into_any_element(),
                    None => "Approve on your phone".into_any_element(),
                };
                let steps = div()
                    .flex()
                    .flex_col()
                    .max_w(px(240.0))
                    .gap(px(14.0))
                    .child(numbered(1, "Open Reins on your phone", pal))
                    .child(numbered(2, "Scan this code", pal))
                    .child(numbered(3, tap, pal));
                div()
                    .flex()
                    .flex_col()
                    .w_full()
                    .gap(px(12.0))
                    .child(kit::pane(
                        div()
                            .flex()
                            .flex_wrap()
                            .items_center()
                            .gap(px(36.0))
                            .px(px(30.0))
                            .py(px(28.0))
                            .child(qr)
                            .child(steps),
                        kit::Glass::Solid,
                        kit::pane::RADIUS,
                        pal,
                    ))
                    .when(!fingerprint.is_empty(), |c| c.child(key_line(&fingerprint, pal)))
                    .into_any_element()
            }
            Pairing::Approved {
                phone,
            } => {
                let on = phone
                    .as_deref()
                    .map_or_else(|| "Approved on your phone".to_owned(), |p| format!("Approved on {p}"));
                centred()
                    .child(kit::kind_tile(Glyph::Check, Tone::Success, 56.0, pal).rounded_full())
                    .child(div().text_size(px(16.0)).text_color(pal.ink()).text_center().child(on))
                    .into_any_element()
            }
            Pairing::Browser => centred()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.0))
                        .text_size(px(15.0))
                        .text_color(pal.ink())
                        .child(waiting_dot("waiting", pal.violet(), 8.0))
                        .child("Finish in your browser"),
                )
                .when(!fingerprint.is_empty(), |c| c.child(key_line(&fingerprint, pal)))
                .child(
                    quiet_link("back-to-qr", "Use the QR code", 13.5, pal)
                        .on_click(Self::on_model(cx, Model::start_pairing)),
                )
                .into_any_element(),
            Pairing::Failed(e) => centred()
                .child(kit::kind_tile(Glyph::CircleClose, Tone::Field(crate::theme::Field::Delete), 40.0, pal))
                .child(div().text_size(px(15.0)).text_color(pal.ink()).child("Pairing failed"))
                .child(caption(e.clone(), pal).text_color(pal.danger).text_center().max_w(px(440.0)))
                .child(
                    kit::button("retry", kit::Look::Inverse, kit::Size::Regular, None, "Try again", pal)
                        .on_click(Self::on_model(cx, Model::start_pairing)),
                )
                .into_any_element(),
            Pairing::Unavailable => centred()
                .child(caption(crate::pairing::UNAVAILABLE, pal).text_center().max_w(px(440.0)))
                .child(
                    kit::button(
                        "browser-primary",
                        kit::Look::Inverse,
                        kit::Size::Regular,
                        None,
                        "Sign in with browser",
                        pal,
                    )
                    .on_click(Self::on_model(cx, Model::sign_in_with_browser)),
                )
                .into_any_element(),
        };

        let device = if cfg!(target_os = "macos") {
            "Mac"
        } else {
            "computer"
        };
        let mut page = div().flex().flex_col().items_center().gap(px(20.0)).child(welcome_title(
            &[plain(format!("Pair this {device} with ")), bold("your phone")],
            Some(help(
                "pair",
                "Your phone approves what your AI agents do on this computer. Pairing takes a few seconds.",
                d,
                pal,
                cx,
            )),
            pal,
        ));
        if let Some(problem) = location {
            page = page.child(warning(problem, pal));
        }
        page = page.child(body);
        let mut more = div().flex().flex_wrap().items_center().justify_center().gap(px(18.0));
        if !matches!(pairing, Pairing::Approved { .. }) {
            more = more.child(
                quiet_link("get-app", "Get the app", 15.0, pal)
                    .on_click(|_, _, cx| cx.open_url(crate::links::PHONE_APP)),
            );
        }
        if !matches!(pairing, Pairing::Browser | Pairing::Unavailable | Pairing::Approved { .. }) {
            more = more.child(
                quiet_link("browser", "Use the browser", 15.0, pal)
                    .on_click(Self::on_model(cx, Model::sign_in_with_browser)),
            );
        }
        if !matches!(pairing, Pairing::Approved { .. }) {
            more = more.child(
                quiet_link(
                    "other-server",
                    if show_server {
                        "Hide server"
                    } else {
                        "Other server"
                    },
                    15.0,
                    pal,
                )
                .on_click(Self::on_model(cx, Model::toggle_server)),
            );
        }
        page = page.child(more);
        if show_server {
            page = page.child(div().w_full().max_w(px(440.0)).child(self.server_box(d, pal, &server, window, cx)));
        }
        if let Some(n) = notice {
            page = page.child(caption(n, pal).text_color(pal.danger).text_center());
        }
        page.into_any_element()
    }

    /// The welcome (`DeskNext-Setup.png` 01): the wallpaper filling the window, and on the frosted pane the line
    /// the phone says too, with Set up this computer and Other server.
    pub(super) fn welcome_hero(d: &Data, pal: Palette, cx: &mut Context<'_, Self>) -> AnyElement {
        let device = if cfg!(target_os = "macos") {
            "Set up this Mac"
        } else {
            "Set up this computer"
        };
        let accent = crate::theme::Field::Accounts.kind(&pal);
        let card = div()
            .flex()
            .flex_col()
            .px(px(30.0))
            .pt(px(26.0))
            .pb(px(24.0))
            .child(div().hero().text_color(pal.ink()).child("Your AI asks,"))
            .child(div().hero().text_color(accent).child("You decide."))
            .child(
                div()
                    .mt(px(14.0))
                    .text_size(px(15.5))
                    .line_height(px(21.0))
                    .text_color(pal.ink_2())
                    .child("Reins sits between your AI tools and what they touch. Your phone answers."),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(12.0))
                    .mt(px(22.0))
                    .child(
                        kit::button("welcome-start", kit::Look::Inverse, kit::Size::Large, None, device, pal)
                            .flex_1()
                            .on_click(Self::on_view(cx, |this, _| this.welcomed = true)),
                    )
                    .child(
                        kit::button(
                            "welcome-server",
                            kit::Look::Soft,
                            kit::Size::Large,
                            Some(Glyph::Server),
                            "Other server",
                            pal,
                        )
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.welcomed = true;
                            this.model.update(cx, |m, cx| {
                                if !m.show_server {
                                    m.toggle_server(cx);
                                }
                            });
                            cx.notify();
                        })),
                    ),
            );
        let glass = kit::Glass::over(true, !d.saved.solid_panes);
        div()
            .relative()
            .size_full()
            .child(kit::wallpaper::fill(d.saved.wallpaper(pal.dark), 300.0, pal))
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .flex()
                    .flex_col()
                    .justify_end()
                    .items_center()
                    .pb(px(100.0))
                    .px(px(40.0))
                    .child(div().w_full().max_w(px(620.0)).child(kit::pane(card, glass, 22.0, pal))),
            )
            .into_any_element()
    }

    fn server_box(
        &self,
        d: &Data,
        pal: Palette,
        server: &str,
        window: &Window,
        cx: &mut Context<'_, Self>,
    ) -> AnyElement {
        div()
            .w_full()
            .flex()
            .flex_col()
            .gap(px(6.0))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(4.0))
                    .child(caption(format!("Now: {}", host(server)), pal))
                    .child(help("server", "For a Reins server you run yourself.", d, pal, cx)),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .child(self.text_field(Field::Server, crate::links::SERVER, pal, window, cx))
                    .child(
                        kit::button("use-server", kit::Look::Soft, kit::Size::Regular, None, "Use", pal)
                            .on_click(Self::on_model(cx, Model::use_server)),
                    ),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_key_is_set_a_hair_apart() {
        assert_eq!(spaced("7712 0458"), "7\u{200A}7\u{200A}1\u{200A}2 0\u{200A}4\u{200A}5\u{200A}8");
        assert_eq!(spaced(""), "");
    }
}
