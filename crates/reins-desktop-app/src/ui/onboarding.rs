//! Step 1 of the welcome flow: pair this computer with the phone. This computer → the phone, one sentence, then the QR
//! code and its user code beside the three things to do (open Reins, scan, tap the number), and this computer's key
//! to compare, in one line with a shield. The browser sign-in and another server are the quieter ways.

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, Context, Div, FontWeight, InteractiveElement as _, IntoElement, ParentElement,
    StatefulInteractiveElement as _, Styled as _, Window, div, px,
};

use super::field::Field;
use super::icons::{Glyph, Mark, glyph};
use super::parts::{
    Btn, btn, caption, card, check_circle, chip_plain, help, link, stage, this_computer, waiting_dot, warning,
    welcome_title,
};
use super::story::Words;
use super::{Data, Root};
use crate::format::host;
use crate::model::{Model, Pairing};
use crate::theme::{MONO, Palette};

/// The QR code's side, and the white frame around it.
const QR: f32 = 152.0;
const QR_FRAME: f32 = 12.0;

/// A number in a ring: a step to do.
fn number(n: usize, pal: Palette) -> Div {
    div()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(px(22.0))
        .rounded_full()
        .border(px(1.5))
        .border_color(pal.violet())
        .bg(pal.violet().opacity(0.15))
        .text_color(pal.violet())
        .text_size(px(11.5))
        .font_weight(FontWeight::SEMIBOLD)
        .child(n.to_string())
}

/// One of the numbered things to do, and what goes at its end.
fn numbered(n: usize, title: impl IntoElement, end: Option<AnyElement>, pal: Palette) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(10.0))
        .min_h(px(26.0))
        .text_size(px(14.0))
        .child(number(n, pal))
        .child(div().flex_1().min_w(px(0.0)).child(title))
        .when_some(end, ParentElement::child)
}

/// This computer's key, to compare with the one the phone shows: one line with a shield.
fn key_line(fingerprint: &str, pal: Palette) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(10.0))
        .px(px(12.0))
        .py(px(10.0))
        .rounded(px(12.0))
        .bg(pal.paper_sunken())
        .border_1()
        .border_color(pal.rule())
        .child(glyph(Glyph::Shield, 18.0, pal.success))
        .child(
            div()
                .flex()
                .flex_col()
                .child(
                    div()
                        .text_size(px(11.5))
                        .text_color(pal.ink_meta())
                        .child(format!("{}'s key", capital(this_computer()))),
                )
                .child(
                    div()
                        .font_family(MONO)
                        .text_size(px(14.0))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(fingerprint.to_owned()),
                ),
        )
        .child(div().flex_1())
        .child(div().text_size(px(12.0)).text_color(pal.ink_meta()).child("must match your phone"))
}

fn capital(s: &str) -> String {
    let mut c = s.chars();
    c.next().map_or_else(String::new, |f| f.to_uppercase().chain(c).collect())
}

/// A centred message in the pairing card (signing in through the browser, an error, approved).
fn centred() -> Div {
    div().flex().flex_col().items_center().justify_center().gap(px(12.0)).w_full().min_h(px(220.0)).p(px(28.0))
}

/// "Pair this Mac with **your phone**".
fn pair_words() -> Vec<Words> {
    let device = if cfg!(target_os = "macos") {
        "Mac"
    } else {
        "computer"
    };
    vec![
        Words {
            text: format!("Pair this {device} with "),
            bold: false,
        },
        Words {
            text: "your phone".to_owned(),
            bold: true,
        },
    ]
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
                        Some(c) => div().p(px(QR_FRAME)).rounded(px(16.0)).bg(gpui::white()).child(crate::qr::element(
                            &c.qr_url,
                            QR,
                            gpui::black(),
                            gpui::white(),
                        )),
                        None => div()
                            .flex()
                            .items_center()
                            .justify_center()
                            .size(px(QR + 2.0 * QR_FRAME))
                            .rounded(px(16.0))
                            .bg(pal.paper_sunken())
                            .child(waiting_dot("getting-code", pal.violet(), 8.0)),
                    })
                    .child(
                        div()
                            .font_family(MONO)
                            .text_size(px(17.0))
                            .line_height(px(22.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(if code.is_some() {
                                pal.ink()
                            } else {
                                pal.ink_meta()
                            })
                            .child(code.as_ref().map_or_else(|| "····-····".to_owned(), |c| c.user_code.clone())),
                    );
                // "Tap [37] on your phone", the number as the phone shows it.
                let tap = match code.as_ref().and_then(|c| c.confirm_code) {
                    Some(n) => div()
                        .flex()
                        .items_center()
                        .gap(px(8.0))
                        .child("Tap")
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .justify_center()
                                .h(px(24.0))
                                .px(px(10.0))
                                .rounded_full()
                                .bg(pal.violet_fill())
                                .text_color(pal.on_field())
                                .text_size(px(13.0))
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(n.to_string()),
                        )
                        .child("on your phone"),
                    None => div().flex().items_center().child("Approve on your phone"),
                }
                .child(help(
                    "pair-number",
                    "The number makes sure you pair this computer and no other.",
                    d,
                    pal,
                    cx,
                ));
                let get_app = chip_plain(Some(Glyph::Download), "Get the app", pal)
                    .id("get-app")
                    .cursor_pointer()
                    .hover(|s| s.text_color(pal.ink()))
                    .on_click(|_, _, cx| cx.open_url(crate::links::PHONE_APP));
                let steps = div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w(px(0.0))
                    .gap(px(14.0))
                    .child(numbered(1, "Open Reins on your phone", Some(get_app.into_any_element()), pal))
                    .child(numbered(2, "Scan this code", None, pal))
                    .child(numbered(3, tap, None, pal))
                    .when(!fingerprint.is_empty(), |s| s.child(key_line(&fingerprint, pal)));
                card(pal).flex_row().items_center().gap(px(28.0)).p(px(22.0)).child(qr).child(steps).into_any_element()
            }
            Pairing::Approved {
                phone,
            } => {
                let on = phone
                    .as_deref()
                    .map_or_else(|| "Approved on your phone".to_owned(), |p| format!("Approved on {p}"));
                card(pal)
                    .child(
                        centred()
                            .child(check_circle(60.0, pal.success))
                            .child(div().text_size(px(17.0)).font_weight(FontWeight::SEMIBOLD).text_center().child(on)),
                    )
                    .into_any_element()
            }
            Pairing::Browser => card(pal)
                .child(
                    centred()
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(8.0))
                                .text_size(px(15.0))
                                .font_weight(FontWeight::MEDIUM)
                                .child(waiting_dot("waiting", pal.violet(), 8.0))
                                .child("Finish in your browser"),
                        )
                        .when(!fingerprint.is_empty(), |c| {
                            c.child(div().w(px(380.0)).child(key_line(&fingerprint, pal)))
                        })
                        .child(
                            link("back-to-qr", "Use the QR code", pal)
                                .on_click(Self::on_model(cx, Model::start_pairing)),
                        ),
                )
                .into_any_element(),
            Pairing::Failed(e) => card(pal)
                .child(
                    centred()
                        .child(super::parts::tile(Glyph::CircleClose, pal.danger, 40.0))
                        .child(div().text_size(px(15.0)).font_weight(FontWeight::MEDIUM).child("Pairing failed"))
                        .child(caption(e.clone(), pal).text_color(pal.danger).text_center().max_w(px(460.0)))
                        .child(
                            btn("retry", "Try again", Btn::Accent, pal)
                                .on_click(Self::on_model(cx, Model::start_pairing)),
                        ),
                )
                .into_any_element(),
            Pairing::Unavailable => card(pal)
                .child(
                    centred().child(caption(crate::pairing::UNAVAILABLE, pal).text_center().max_w(px(460.0))).child(
                        btn("browser-primary", "Sign in with browser", Btn::Accent, pal)
                            .on_click(Self::on_model(cx, Model::sign_in_with_browser)),
                    ),
                )
                .into_any_element(),
        };

        let mut page = div()
            .flex()
            .flex_col()
            .items_center()
            .gap(px(16.0))
            .child(stage(Mark::Glyph(Glyph::Laptop), Mark::Glyph(Glyph::Phone), None, pal.violet(), 56.0, pal))
            .child(welcome_title(
                &pair_words(),
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
        page = page.child(div().w_full().pt(px(6.0)).child(body));
        let mut more = div().flex().items_center().justify_center().gap(px(18.0)).text_size(px(13.0));
        if !matches!(pairing, Pairing::Browser | Pairing::Unavailable | Pairing::Approved { .. }) {
            more = more.child(
                link("browser", "Use the browser", pal)
                    .text_color(pal.ink_2())
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
                .text_color(pal.ink_meta())
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
                    .gap(px(4.0))
                    .child(caption(format!("Now: {}", host(server)), pal))
                    .child(help("server", "For a Reins server you run yourself.", d, pal, cx)),
            )
            .child(
                div()
                    .flex()
                    .gap(px(8.0))
                    .child(self.text_field(Field::Server, crate::links::SERVER, pal, window, cx))
                    .child(
                        btn("use-server", "Use", Btn::Plain, pal)
                            .h(px(34.0))
                            .on_click(Self::on_model(cx, Model::use_server)),
                    ),
            )
            .into_any_element()
    }
}
