//! Step 1 of the welcome flow: pair this computer with the phone. The QR code and its user code beside the three
//! things to do (open Reins, scan, tap the number), and this computer's key to compare. The browser sign-in and
//! another server are the quieter ways.

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, Context, Div, FontWeight, IntoElement, ParentElement, StatefulInteractiveElement as _, Styled as _,
    Window, div, px,
};

use super::field::Field;
use super::parts::{button, caption, card, check_circle, fine, help, link, waiting_dot, warning, welcome_title};
use super::{Data, Root};
use crate::format::host;
use crate::model::{Model, Pairing};
use crate::theme::{MONO, Palette};

/// The QR code's side, and the white frame around it.
const QR: f32 = 188.0;
const QR_FRAME: f32 = 10.0;

/// One of the numbered things to do.
fn numbered(n: usize, title: impl IntoElement, about: Option<AnyElement>, pal: Palette) -> Div {
    div()
        .flex()
        .items_start()
        .gap(px(12.0))
        .child(
            div()
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .size(px(22.0))
                .mt(px(-1.0))
                .rounded_full()
                .bg(pal.accent_soft)
                .text_color(pal.accent)
                .text_size(px(11.5))
                .font_weight(FontWeight::SEMIBOLD)
                .child(n.to_string()),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w(px(0.0))
                .gap(px(2.0))
                .child(div().text_size(px(13.5)).font_weight(FontWeight::MEDIUM).child(title))
                .when_some(about, ParentElement::child),
        )
}

/// This computer's key, to compare with the one the phone shows.
fn key_box(fingerprint: &str, pal: Palette) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(px(4.0))
        .px(px(14.0))
        .py(px(11.0))
        .rounded(px(10.0))
        .bg(pal.control_fill)
        .child(fine("This computer's key", pal))
        .child(
            div()
                .font_family(MONO)
                .text_size(px(14.0))
                .line_height(px(20.0))
                .font_weight(FontWeight::SEMIBOLD)
                .child(fingerprint.to_owned()),
        )
        .child(caption("Approve only if your phone shows the same key.", pal))
}

/// A centred message in the pairing card (signing in through the browser, an error, approved).
fn centred(pal: Palette) -> Div {
    div()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap(px(12.0))
        .w_full()
        .min_h(px(260.0))
        .p(px(28.0))
        .bg(pal.elevated)
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
                            .p(px(QR_FRAME))
                            .rounded(px(16.0))
                            .bg(gpui::white())
                            .border_1()
                            .border_color(pal.hairline)
                            .child(crate::qr::element(&c.qr_url, QR, gpui::black(), gpui::white())),
                        None => div()
                            .flex()
                            .items_center()
                            .justify_center()
                            .size(px(QR + 2.0 * QR_FRAME))
                            .rounded(px(16.0))
                            .bg(pal.control_fill)
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(8.0))
                                    .child(waiting_dot("getting-code", pal.accent, 7.0))
                                    .child(caption("Getting a code…", pal)),
                            ),
                    })
                    .child(
                        div()
                            .font_family(MONO)
                            .text_size(px(19.0))
                            .line_height(px(24.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(if code.is_some() {
                                pal.text
                            } else {
                                pal.tertiary
                            })
                            .child(code.as_ref().map_or_else(|| "····-····".to_owned(), |c| c.user_code.clone())),
                    );
                // "Tap [37] on your phone", the number as the phone shows it.
                let tap_title = match code.as_ref().and_then(|c| c.confirm_code) {
                    Some(n) => div()
                        .flex()
                        .items_center()
                        .gap(px(7.0))
                        .child("Tap")
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .justify_center()
                                .min_w(px(30.0))
                                .h(px(24.0))
                                .px(px(7.0))
                                .rounded(px(7.0))
                                .bg(pal.accent)
                                .text_color(pal.on_accent)
                                .text_size(px(14.0))
                                .font_weight(FontWeight::BOLD)
                                .child(n.to_string()),
                        )
                        .child("on your phone"),
                    None => div().flex().items_center().child("Approve on your phone"),
                }
                .child(div().w(px(1.0)))
                .child(help(
                    "pair-number",
                    "The number makes sure you pair this computer and no other.",
                    d,
                    pal,
                    cx,
                ));
                let steps = div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w(px(0.0))
                    .gap(px(16.0))
                    .child(numbered(
                        1,
                        "Open Reins on your phone",
                        Some(
                            link("get-app", "Get the app", pal)
                                .on_click(|_, _, cx| cx.open_url(crate::links::PHONE_APP))
                                .into_any_element(),
                        ),
                        pal,
                    ))
                    .child(numbered(2, "Scan this code", None, pal))
                    .child(numbered(3, tap_title, None, pal))
                    .when(!fingerprint.is_empty(), |s| s.child(key_box(&fingerprint, pal)))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.0))
                            .child(waiting_dot("waiting", pal.accent, 7.0))
                            .child(caption("Waiting for your phone…", pal)),
                    );
                card(pal).flex_row().items_center().gap(px(32.0)).p(px(28.0)).child(qr).child(steps).into_any_element()
            }
            Pairing::Approved {
                phone,
            } => {
                let on = phone
                    .as_deref()
                    .map_or_else(|| "Approved on your phone".to_owned(), |p| format!("Approved on {p}"));
                card(pal)
                    .child(
                        centred(pal)
                            .child(check_circle(60.0, pal.success))
                            .child(div().text_size(px(17.0)).font_weight(FontWeight::SEMIBOLD).text_center().child(on)),
                    )
                    .into_any_element()
            }
            Pairing::Browser => card(pal)
                .child(
                    centred(pal)
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(8.0))
                                .text_size(px(15.0))
                                .font_weight(FontWeight::MEDIUM)
                                .child(waiting_dot("waiting", pal.accent, 8.0))
                                .child("Finish in your browser"),
                        )
                        .when(!fingerprint.is_empty(), |c| {
                            c.child(div().w(px(340.0)).child(key_box(&fingerprint, pal)))
                        })
                        .child(
                            link("back-to-qr", "Use the QR code", pal)
                                .on_click(Self::on_model(cx, Model::start_pairing)),
                        ),
                )
                .into_any_element(),
            Pairing::Failed(e) => card(pal)
                .child(
                    centred(pal)
                        .child(div().text_size(px(15.0)).font_weight(FontWeight::MEDIUM).child("Pairing failed"))
                        .child(caption(e.clone(), pal).text_color(pal.danger).text_center().max_w(px(460.0)))
                        .child(
                            button("retry", "Try again", pal, true, true)
                                .on_click(Self::on_model(cx, Model::start_pairing)),
                        ),
                )
                .into_any_element(),
            Pairing::Unavailable => card(pal)
                .child(
                    centred(pal).child(caption(crate::pairing::UNAVAILABLE, pal).text_center().max_w(px(460.0))).child(
                        button("browser-primary", "Sign in with browser", pal, true, true)
                            .on_click(Self::on_model(cx, Model::sign_in_with_browser)),
                    ),
                )
                .into_any_element(),
        };

        let mut page = div().flex().flex_col().items_center().gap(px(20.0)).child(welcome_title(
            "Pair with your phone",
            Some(help(
                "pair",
                "Your phone approves what your AI agents do on this computer. Pairing takes a few seconds.",
                d,
                pal,
                cx,
            )),
        ));
        if let Some(problem) = location {
            page = page.child(warning(problem, pal));
        }
        page = page.child(div().w_full().child(body));
        let mut more = div().flex().items_center().justify_center().gap(px(18.0));
        if !matches!(pairing, Pairing::Browser | Pairing::Unavailable | Pairing::Approved { .. }) {
            more = more.child(
                link("browser", "Use the browser", pal)
                    .text_color(pal.secondary)
                    .on_click(Self::on_model(cx, Model::sign_in_with_browser)),
            );
        }
        if !matches!(pairing, Pairing::Approved { .. }) {
            more = more.child(
                link(
                    "other-server",
                    if show_server {
                        "Hide server"
                    } else {
                        "Other server"
                    },
                    pal,
                )
                .text_color(pal.tertiary)
                .on_click(Self::on_model(cx, Model::toggle_server)),
            );
        }
        page = page.child(more);
        if show_server {
            page = page.child(div().w_full().max_w(px(460.0)).child(self.server_box(d, pal, &server, window, cx)));
        }
        if let Some(n) = notice {
            page = page.child(caption(n, pal).text_color(pal.danger).text_center());
        }
        page.into_any_element()
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
                    .gap(px(6.0))
                    .child(caption(format!("Now: {}", host(server)), pal))
                    .child(help("server", "For a Reins server you run yourself.", d, pal, cx)),
            )
            .child(
                div()
                    .flex()
                    .gap(px(8.0))
                    .child(self.text_field(Field::Server, crate::links::SERVER, pal, window, cx))
                    .child(
                        button("use-server", "Use", pal, false, true).on_click(Self::on_model(cx, Model::use_server)),
                    ),
            )
            .into_any_element()
    }
}
