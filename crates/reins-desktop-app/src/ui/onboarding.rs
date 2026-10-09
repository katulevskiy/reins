//! Onboarding: pair this computer with the phone (a QR code, or the browser sign-in), or use another server.

use gpui::{
    AnyElement, Context, FontWeight, IntoElement, ParentElement as _, StatefulInteractiveElement as _, Styled as _,
    Window, div, px,
};

use super::Root;
use super::field::Field;
use super::parts::{button, caption, card, link, mark, waiting_dot, warning};
use crate::format::host;
use crate::model::{Model, Pairing};
use crate::theme::{MONO, Palette};

impl Root {
    pub(super) fn onboarding(&self, pal: Palette, window: &mut Window, cx: &mut Context<'_, Self>) -> AnyElement {
        let m = self.model.read(cx);
        let pairing = m.pairing.clone();
        let fingerprint = m.fingerprint.clone().unwrap_or_default();
        let show_server = m.show_server;
        let notice = m.notice.clone();
        let location = crate::backend::Backend::location_problem();
        let server = m.server();

        let mut connect = card(pal).p(px(20.0)).items_center().gap(px(12.0));
        connect =
            connect.child(div().text_size(px(15.0)).font_weight(FontWeight::SEMIBOLD).child("Connect with your phone"));
        if matches!(pairing, Pairing::Starting | Pairing::Code(_)) {
            connect = connect.child(
                caption("Open Reins on your phone and scan this code, or point your phone's camera at it.", pal)
                    .text_center(),
            );
        }
        match &pairing {
            Pairing::Starting => {
                connect = connect.child(
                    div()
                        .flex()
                        .items_center()
                        .justify_center()
                        .size(px(196.0))
                        .rounded(px(14.0))
                        .bg(pal.control_fill)
                        .child(caption("Getting a code…", pal)),
                );
            }
            Pairing::Code(code) => {
                connect = connect
                    .child(
                        div()
                            .p(px(6.0))
                            .rounded(px(14.0))
                            .bg(gpui::white())
                            .border_1()
                            .border_color(pal.hairline)
                            .child(crate::qr::element(&code.qr_url, 184.0, gpui::black(), gpui::white())),
                    )
                    .child(
                        div()
                            .font_family(MONO)
                            .text_size(px(22.0))
                            .line_height(px(28.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(code.user_code.clone()),
                    )
                    .child(
                        div().flex().items_center().gap(px(8.0)).child(waiting_dot("waiting", pal.accent, 7.0)).child(
                            caption(
                                code.confirm_code.map_or_else(
                                    || "Waiting for your phone…".to_owned(),
                                    |n| format!("Waiting for your phone. Then tap {n} there."),
                                ),
                                pal,
                            ),
                        ),
                    );
            }
            Pairing::Approved {
                phone,
            } => {
                let on = phone
                    .as_deref()
                    .map_or_else(|| "Approved on your phone".to_owned(), |p| format!("Approved on {p}"));
                connect = connect.child(
                    div()
                        .flex()
                        .flex_col()
                        .items_center()
                        .justify_center()
                        .gap(px(10.0))
                        .w_full()
                        .h(px(196.0))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .justify_center()
                                .size(px(56.0))
                                .rounded_full()
                                .bg(pal.success.opacity(0.14))
                                .text_color(pal.success)
                                .text_size(px(26.0))
                                .child("✓"),
                        )
                        .child(div().text_size(px(15.0)).font_weight(FontWeight::SEMIBOLD).text_center().child(on)),
                );
            }
            Pairing::Browser => {
                connect = connect.child(
                    div()
                        .flex()
                        .flex_col()
                        .items_center()
                        .gap(px(10.0))
                        .py(px(24.0))
                        .child(div().flex().items_center().gap(px(8.0)).child(waiting_dot("waiting", pal.accent, 7.0)).child("Finish signing in in your browser"))
                        .child(
                            caption(
                                format!("Your phone then shows this computer's key, {fingerprint}. Approve only if it is the same."),
                                pal,
                            )
                            .text_center(),
                        )
                        .child(link("back-to-qr", "Use the QR code instead", pal).on_click(Self::on_model(cx, Model::start_pairing))),
                );
            }
            Pairing::Failed(e) => {
                connect = connect.child(
                    div()
                        .flex()
                        .flex_col()
                        .items_center()
                        .gap(px(12.0))
                        .py(px(24.0))
                        .child(caption(e.clone(), pal).text_color(pal.danger).text_center())
                        .child(
                            button("retry", "Try again", pal, true, true)
                                .on_click(Self::on_model(cx, Model::start_pairing)),
                        ),
                );
            }
            Pairing::Unavailable => {
                connect = connect.child(
                    div()
                        .flex()
                        .flex_col()
                        .items_center()
                        .gap(px(12.0))
                        .py(px(18.0))
                        .child(caption(crate::pairing::UNAVAILABLE, pal).text_center())
                        .child(
                            button("browser-primary", "Sign in with browser", pal, true, true)
                                .on_click(Self::on_model(cx, Model::sign_in_with_browser)),
                        ),
                );
            }
        }
        if matches!(pairing, Pairing::Code(_)) && !fingerprint.is_empty() {
            connect = connect.child(
                caption(
                    format!("This computer's key is {fingerprint}. Approve only if your phone shows the same."),
                    pal,
                )
                .text_size(px(11.5))
                .text_color(pal.tertiary)
                .text_center(),
            );
        }

        let mut page = div().flex().flex_col().items_center().gap(px(14.0)).child(mark(44.0)).child(
            div()
                .flex()
                .flex_col()
                .items_center()
                .gap(px(4.0))
                .child(div().text_size(px(22.0)).line_height(px(28.0)).font_weight(FontWeight::SEMIBOLD).child("Reins"))
                .child(caption("Reins keeps your AI agents on a leash.", pal).text_size(px(13.5))),
        );
        if let Some(problem) = location {
            page = page.child(warning(problem, pal));
        }
        page = page.child(connect.w_full()).child(
            div().flex().flex_col().items_center().gap(px(2.0)).child(caption("No phone app yet?", pal)).child(
                link("get-app", "Get Reins for iPhone or Android", pal)
                    .on_click(|_, _, cx| cx.open_url(crate::links::PHONE_APP)),
            ),
        );
        let mut more = div().flex().items_center().justify_center().gap(px(16.0));
        if !matches!(pairing, Pairing::Browser | Pairing::Unavailable | Pairing::Approved { .. }) {
            more = more.child(
                link("browser", "Sign in with browser", pal).on_click(Self::on_model(cx, Model::sign_in_with_browser)),
            );
        }
        more = more.child(
            link(
                "other-server",
                if show_server {
                    "Hide server"
                } else {
                    "Use another server"
                },
                pal,
            )
            .text_color(pal.tertiary)
            .on_click(Self::on_model(cx, Model::toggle_server)),
        );
        page = page.child(more);
        if show_server {
            page = page.child(self.server_box(pal, &server, window, cx));
        }
        if let Some(n) = notice {
            page = page.child(caption(n, pal).text_color(pal.danger).text_center());
        }
        page.into_any_element()
    }

    fn server_box(&self, pal: Palette, server: &str, window: &Window, cx: &mut Context<'_, Self>) -> AnyElement {
        div()
            .w_full()
            .flex()
            .flex_col()
            .gap(px(6.0))
            .child(caption(format!("For a Reins server you run yourself. Now: {}", host(server)), pal))
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
