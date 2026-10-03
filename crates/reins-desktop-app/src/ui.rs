//! The window: onboarding (pair with the phone), the first-time setup (harnesses, service, git), and the status.
//! Small and calm, like the phone apps: Geist, cool neutrals, one violet accent.

use std::sync::{Arc, LazyLock};
use std::time::Duration;

use gpui::{
    Animation, AnimationExt as _, AnyElement, ClickEvent, Context, Div, ElementId, Entity, FocusHandle, FontWeight,
    Image, ImageFormat, InteractiveElement as _, IntoElement, KeyDownEvent, ParentElement as _, Render, SharedString,
    StatefulInteractiveElement as _, Styled as _, Subscription, Window, div, img, pulsating_between, px,
};
use reins_desktop::harness::Harness;

use crate::backend::DaemonState;
use crate::model::{Model, Pairing, Screen, Step};
use crate::theme::{FONT, MONO, Palette};
use crate::tray::Look;

pub const WIDTH: f32 = 440.0;
pub const HEIGHT: f32 = 700.0;

static MARK: LazyLock<Arc<Image>> = LazyLock::new(|| {
    Arc::new(Image::from_bytes(ImageFormat::Png, include_bytes!("../assets/icons/app-256.png").to_vec()))
});

pub struct Root {
    model: Entity<Model>,
    server_field: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl Root {
    pub fn new(model: Entity<Model>, window: &mut Window, cx: &mut Context<'_, Self>) -> Self {
        let subscriptions = vec![
            cx.observe(&model, |_, _, cx| cx.notify()),
            cx.observe_window_appearance(window, |_, _, cx| cx.notify()),
        ];
        Self {
            model,
            server_field: cx.focus_handle(),
            _subscriptions: subscriptions,
        }
    }

    /// A click handler that runs `f` on the model.
    fn on_model(
        cx: &Context<'_, Self>,
        f: impl Fn(&mut Model, &mut Context<'_, Model>) + 'static,
    ) -> impl Fn(&ClickEvent, &mut Window, &mut gpui::App) + 'static {
        cx.listener(move |this, _: &ClickEvent, _, cx| this.model.update(cx, |m, cx| f(m, cx)))
    }
}

// Building blocks.

fn mark(side: f32) -> impl IntoElement {
    img(MARK.clone()).size(px(side)).flex_none()
}

fn card(pal: Palette) -> Div {
    div().flex().flex_col().bg(pal.elevated).border_1().border_color(pal.hairline).rounded(px(14.0)).overflow_hidden()
}

fn caption(text: impl Into<SharedString>, pal: Palette) -> Div {
    div().text_size(px(12.0)).line_height(px(17.0)).text_color(pal.secondary).child(text.into())
}

fn section_title(text: &'static str, pal: Palette) -> Div {
    div()
        .px(px(4.0))
        .pb(px(6.0))
        .text_size(px(11.5))
        .font_weight(FontWeight::MEDIUM)
        .text_color(pal.tertiary)
        .child(text)
}

fn button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    pal: Palette,
    primary: bool,
    enabled: bool,
) -> gpui::Stateful<Div> {
    let (bg, fg) = if primary {
        (pal.accent, pal.on_accent)
    } else {
        (pal.control_fill, pal.text)
    };
    let b = div()
        .id(id.into())
        .flex()
        .items_center()
        .justify_center()
        .h(px(34.0))
        .px(px(14.0))
        .rounded(px(10.0))
        .bg(bg)
        .text_color(fg)
        .text_size(px(13.0))
        .font_weight(FontWeight::MEDIUM)
        .child(label.into());
    if enabled {
        b.cursor_pointer().hover(|s| s.opacity(0.88))
    } else {
        b.opacity(0.45)
    }
}

fn link(id: impl Into<ElementId>, label: impl Into<SharedString>, pal: Palette) -> gpui::Stateful<Div> {
    div()
        .id(id.into())
        .text_size(px(12.0))
        .text_color(pal.accent)
        .cursor_pointer()
        .hover(|s| s.opacity(0.75))
        .child(label.into())
}

fn row(pal: Palette, first: bool) -> Div {
    let r = div().flex().items_center().gap(px(10.0)).px(px(14.0)).py(px(10.0));
    if first {
        r
    } else {
        r.border_t_1().border_color(pal.hairline)
    }
}

fn checkbox(checked: bool, enabled: bool, pal: Palette) -> Div {
    let b = div().flex().flex_none().items_center().justify_center().size(px(18.0)).rounded(px(5.0));
    let b = if checked {
        b.bg(pal.accent).text_color(pal.on_accent).text_size(px(12.0)).child("✓")
    } else {
        b.border_1().border_color(pal.tertiary)
    };
    if enabled {
        b
    } else {
        b.opacity(0.4)
    }
}

fn toggle(on: bool, pal: Palette) -> Div {
    let knob = div().size(px(16.0)).rounded_full().bg(gpui::white()).shadow_sm();
    div()
        .flex()
        .flex_none()
        .items_center()
        .w(px(34.0))
        .h(px(20.0))
        .p(px(2.0))
        .rounded_full()
        .bg(if on {
            pal.accent
        } else {
            pal.control_fill
        })
        .when_on(on)
        .child(knob)
}

trait WhenOn {
    fn when_on(self, on: bool) -> Self;
}

impl WhenOn for Div {
    fn when_on(self, on: bool) -> Self {
        if on {
            self.justify_end()
        } else {
            self.justify_start()
        }
    }
}

fn pill(look: Look, pal: Palette) -> Div {
    let color = match look {
        Look::On => pal.success,
        Look::Paused => pal.warning,
        Look::Attention => pal.danger,
    };
    div()
        .flex()
        .items_center()
        .gap(px(6.0))
        .px(px(9.0))
        .py(px(3.0))
        .rounded_full()
        .bg(color.opacity(0.12))
        .text_color(color)
        .text_size(px(11.5))
        .font_weight(FontWeight::MEDIUM)
        .child(div().size(px(6.0)).rounded_full().bg(color))
        .child(look.word())
}

fn waiting_dot(pal: Palette) -> impl IntoElement {
    div().size(px(7.0)).rounded_full().bg(pal.accent).with_animation(
        "waiting",
        Animation::new(Duration::from_millis(1600)).repeat().with_easing(pulsating_between(0.25, 1.0)),
        gpui::Styled::opacity,
    )
}

fn step_line(label: &str, step: &Step, pal: Palette, first: bool) -> Div {
    let (icon, color, detail) = match step {
        Step::Running => ("…", pal.accent, String::new()),
        Step::Done(d) => ("✓", pal.success, d.clone()),
        Step::Failed(e) => ("!", pal.danger, e.clone()),
    };
    row(pal, first)
        .items_start()
        .child(div().w(px(14.0)).text_color(color).font_weight(FontWeight::SEMIBOLD).child(icon))
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w(px(0.0))
                .gap(px(2.0))
                .child(div().text_size(px(13.0)).child(label.to_owned()))
                .when_some_detail(detail, pal),
        )
}

trait Detail {
    fn when_some_detail(self, detail: String, pal: Palette) -> Self;
}

impl Detail for Div {
    fn when_some_detail(self, detail: String, pal: Palette) -> Self {
        if detail.is_empty() {
            self
        } else {
            self.child(caption(detail, pal))
        }
    }
}

/// `https://app.reins2fa.com` → `app.reins2fa.com`.
fn host(server: &str) -> String {
    server.split("://").nth(1).unwrap_or(server).trim_end_matches('/').to_owned()
}

impl Render for Root {
    fn render(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let pal = Palette::of(window);
        let screen = self.model.read(cx).screen;
        let body = match screen {
            Screen::Onboarding => self.onboarding(pal, window, cx),
            Screen::Setup => self.setup(pal, cx),
            Screen::Status => self.status(pal, cx),
        };
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(pal.background)
            .text_color(pal.text)
            .font_family(FONT)
            .text_size(px(13.0))
            .line_height(px(18.0))
            // The macOS title bar is transparent: leave room for the traffic lights.
            .child(div().flex_none().h(px(if cfg!(target_os = "macos") {
                34.0
            } else {
                6.0
            })))
            .child(div().id("content").flex_1().overflow_y_scroll().px(px(22.0)).pb(px(22.0)).child(body))
    }
}

impl Root {
    fn onboarding(&self, pal: Palette, window: &mut Window, cx: &mut Context<'_, Self>) -> AnyElement {
        let m = self.model.read(cx);
        let pairing = m.pairing.clone();
        let fingerprint = m.fingerprint.clone().unwrap_or_default();
        let show_server = m.show_server;
        let server_input = m.server_input.clone();
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
                    .child(div().flex().items_center().gap(px(8.0)).child(waiting_dot(pal)).child(caption(
                        code.confirm_code.map_or_else(
                            || "Waiting for your phone…".to_owned(),
                            |n| format!("Waiting for your phone. Then tap {n} there."),
                        ),
                        pal,
                    )));
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
                        .child(div().flex().items_center().gap(px(8.0)).child(waiting_dot(pal)).child("Finish signing in in your browser"))
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
            page = page.child(self.server_field(pal, &server_input, &server, window, cx));
        }
        if let Some(n) = notice {
            page = page.child(caption(n, pal).text_color(pal.danger).text_center());
        }
        page.into_any_element()
    }

    fn server_field(
        &self,
        pal: Palette,
        input: &str,
        server: &str,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> AnyElement {
        let focused = self.server_field.is_focused(window);
        let shown: SharedString = if input.is_empty() && !focused {
            crate::links::SERVER.into()
        } else {
            format!(
                "{input}{}",
                if focused {
                    "▏"
                } else {
                    ""
                }
            )
            .into()
        };
        let field = div()
            .id("server-field")
            .track_focus(&self.server_field)
            .flex_1()
            .h(px(32.0))
            .px(px(10.0))
            .flex()
            .items_center()
            .rounded(px(9.0))
            .bg(pal.elevated)
            .border_1()
            .border_color(if focused {
                pal.accent
            } else {
                pal.hairline
            })
            .font_family(MONO)
            .text_size(px(12.0))
            .text_color(if input.is_empty() && !focused {
                pal.tertiary
            } else {
                pal.text
            })
            .cursor_text()
            .child(shown)
            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| window.focus(&this.server_field, cx)))
            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, _, cx| {
                let ks = ev.keystroke.clone();
                let pasted = (ks.modifiers.secondary() && ks.key == "v")
                    .then(|| cx.read_from_clipboard().and_then(|c| c.text()))
                    .flatten();
                this.model.update(cx, |m, cx| {
                    match ks.key.as_str() {
                        "backspace" => {
                            m.server_input.pop();
                        }
                        "enter" => {
                            m.use_server(cx);
                            return;
                        }
                        _ if pasted.is_some() => m.server_input.push_str(pasted.as_deref().unwrap_or("").trim()),
                        _ if !ks.modifiers.platform && !ks.modifiers.control => {
                            if let Some(ch) = ks.key_char.as_deref().filter(|c| !c.chars().any(char::is_control)) {
                                m.server_input.push_str(ch);
                            }
                        }
                        _ => return,
                    }
                    cx.notify();
                });
                cx.stop_propagation();
            }));
        div()
            .w_full()
            .flex()
            .flex_col()
            .gap(px(6.0))
            .child(caption(format!("For a Reins server you run yourself. Now: {}", host(server)), pal))
            .child(
                div().flex().gap(px(8.0)).child(field).child(
                    button("use-server", "Use", pal, false, true).on_click(Self::on_model(cx, Model::use_server)),
                ),
            )
            .into_any_element()
    }

    fn setup(&self, pal: Palette, cx: &mut Context<'_, Self>) -> AnyElement {
        let m = self.model.read(cx);
        let items = m.setup_items.clone();
        let steps = m.setup_steps.clone();
        let running = m.setup_running;
        let finished = m.setup_finished();
        let failed = steps.iter().any(|(_, s)| matches!(s, Step::Failed(_)));
        let notice = m.notice.clone();
        let location = crate::backend::Backend::location_problem();

        let mut list = card(pal);
        for (i, item) in items.iter().enumerate() {
            let (checked, available) = (item.checked, item.available);
            let mut r = row(pal, i == 0).id(("setup-item", i)).child(checkbox(checked, available, pal)).child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w(px(0.0))
                    .gap(px(1.0))
                    .child(div().font_weight(FontWeight::MEDIUM).child(item.label()))
                    .child(caption(item.detail(), pal).text_size(px(11.5))),
            );
            if available && !running && !finished {
                r = r.cursor_pointer().on_click(Self::on_model(cx, move |m, cx| m.toggle_item(i, cx)));
            } else if !available {
                r = r.opacity(0.6);
            }
            list = list.child(r);
        }

        let mut page = div()
            .flex()
            .flex_col()
            .gap(px(14.0))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.0))
                    .child(mark(30.0))
                    .child(div().text_size(px(15.0)).font_weight(FontWeight::SEMIBOLD).child("Almost done")),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(4.0))
                    .child(div().text_size(px(19.0)).line_height(px(25.0)).font_weight(FontWeight::SEMIBOLD).child("Add Reins to your AI tools"))
                    .child(caption(
                        "Reins found these on this computer and adds itself to each one. Untick anything you want to leave alone.",
                        pal,
                    )),
            );
        if steps.is_empty() {
            page = page.child(list);
        } else {
            let mut progress = card(pal);
            for (i, (label, step)) in steps.iter().enumerate() {
                progress = progress.child(step_line(label, step, pal, i == 0));
            }
            page = page.child(progress);
        }
        let actions = if finished && !failed {
            div()
                .flex()
                .flex_col()
                .gap(px(8.0))
                .child(caption("All set. Restart your AI tools so they pick up Reins.", pal))
                .child(button("done", "Done", pal, true, true).on_click(Self::on_model(cx, Model::finish_setup)))
        } else if finished {
            div()
                .flex()
                .gap(px(8.0))
                .child(
                    button("again", "Try again", pal, true, true)
                        .flex_1()
                        .on_click(Self::on_model(cx, Model::run_setup)),
                )
                .child(
                    button("skip", "Continue anyway", pal, false, true)
                        .on_click(Self::on_model(cx, Model::finish_setup)),
                )
        } else {
            let label = if running {
                "Setting up…"
            } else {
                "Set up Reins"
            };
            let b = button("run-setup", label, pal, true, !running && location.is_none());
            div().flex().flex_col().child(if running || location.is_some() {
                b
            } else {
                b.on_click(Self::on_model(cx, Model::run_setup))
            })
        };
        if let Some(problem) = location {
            page = page.child(warning(problem, pal));
        }
        page = page.child(actions);
        if let Some(n) = notice {
            page = page.child(caption(n, pal).text_color(pal.danger));
        }
        page.into_any_element()
    }

    fn status(&self, pal: Palette, cx: &mut Context<'_, Self>) -> AnyElement {
        let m = self.model.read(cx);
        let (look, line) = m.status_line();
        let snapshot = m.snapshot.clone();
        let saved = m.saved.clone();
        let server = m.server();
        let fingerprint = m.fingerprint.clone();
        let update = m.update.clone();
        let notice = m.notice.clone();
        let show_qr = m.show_phone_qr;
        let autostart = m.autostart;
        let paused = look == Look::Paused;
        let paired = m.paired();

        let sub = match look {
            Look::On => "Your phone approves what your AI agents do.",
            Look::Paused => "git talks to GitHub directly until you resume. Hooks and tools still ask your phone.",
            Look::Attention if !paired => "Pair this computer with your phone again.",
            Look::Attention => "Start the background service so git and your agents reach your phone.",
        };

        let mut page = div()
            .flex()
            .flex_col()
            .gap(px(16.0))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.0))
                    .child(mark(30.0))
                    .child(div().flex_1().text_size(px(15.0)).font_weight(FontWeight::SEMIBOLD).child("Reins"))
                    .child(pill(look, pal)),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(4.0))
                    .child(
                        div().text_size(px(19.0)).line_height(px(25.0)).font_weight(FontWeight::SEMIBOLD).child(line),
                    )
                    .child(caption(sub, pal)),
            );

        // Pause, resume, activity.
        let mut actions = div().flex().gap(px(8.0));
        actions = if paused {
            actions
                .child(button("resume", "Resume", pal, true, true).flex_1().on_click(Self::on_model(cx, Model::resume)))
        } else {
            actions.child(
                button("pause", "Pause for 1 hour", pal, false, paired)
                    .flex_1()
                    .on_click(Self::on_model(cx, Model::pause_hour)),
            )
        };
        actions = actions.child(
            button("activity", "Open activity on phone", pal, false, true)
                .flex_1()
                .on_click(Self::on_model(cx, Model::toggle_phone_qr)),
        );
        page = page.child(actions);
        if show_qr {
            page = page.child(
                card(pal)
                    .p(px(16.0))
                    .flex_row()
                    .items_center()
                    .gap(px(14.0))
                    .child(
                        div()
                            .p(px(4.0))
                            .rounded(px(10.0))
                            .bg(gpui::white())
                            .child(crate::qr::element(crate::links::PHONE_ACTIVITY, 96.0, gpui::black(), gpui::white())),
                    )
                    .child(caption(
                        "Point your phone's camera at this code to open Activity in Reins: everything your agents asked for, and what you answered.",
                        pal,
                    ).flex_1().min_w(px(0.0))),
            );
        }
        if let Some(version) = update {
            page = page.child(
                card(pal)
                    .p(px(12.0))
                    .flex_row()
                    .items_center()
                    .gap(px(10.0))
                    .bg(pal.accent_soft)
                    .border_color(pal.accent.opacity(0.25))
                    .child(div().flex_1().child(format!("Reins {version} is available.")))
                    .child(
                        link("download", "Download", pal).on_click(|_, _, cx| cx.open_url(&crate::links::download())),
                    ),
            );
        }

        // Account.
        let account = saved.account.clone().unwrap_or_else(|| host(&server));
        let phone = saved.phone.unwrap_or_else(|| "Your phone".to_owned());
        let mut connected =
            card(pal).child(info_row("Account", &account, pal, true)).child(info_row("Approves", &phone, pal, false));
        if let Some(fp) = fingerprint {
            connected = connected.child(info_row("This computer's key", &fp, pal, false));
        }
        connected = connected.child(
            row(pal, false).child(div().flex_1()).child(
                link(
                    "sign-out",
                    if paired {
                        "Disconnect"
                    } else {
                        "Pair again"
                    },
                    pal,
                )
                .on_click(Self::on_model(cx, Model::sign_out)),
            ),
        );
        page = page.child(div().flex().flex_col().child(section_title("CONNECTED", pal)).child(connected));

        // Harnesses.
        if let Some(s) = &snapshot {
            let mut tools = card(pal);
            let mut first = true;
            let mut rows = s.harnesses.clone();
            rows.sort_by_key(|r| !(r.found || r.added));
            for r in rows {
                let h: Harness = r.harness;
                let detail = match (r.found, r.added) {
                    (_, true) => "Reins is in its settings",
                    (true, false) => "Reins is not in its settings",
                    (false, false) => "Not installed",
                };
                let mut line = row(pal, first).id(SharedString::from(format!("harness-{}", h.id()))).child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_w(px(0.0))
                        .child(div().font_weight(FontWeight::MEDIUM).child(h.label()))
                        .child(caption(detail, pal).text_size(px(11.5))),
                );
                if r.found || r.added {
                    let on = r.added;
                    line = line
                        .child(toggle(on, pal))
                        .cursor_pointer()
                        .on_click(Self::on_model(cx, move |m, cx| m.set_harness(h, !on, cx)));
                } else {
                    line = line.opacity(0.55);
                }
                tools = tools.child(line);
                first = false;
            }
            page = page.child(div().flex().flex_col().child(section_title("AI TOOLS", pal)).child(tools));

            // The service and git.
            let (service, running) = match &s.daemon {
                DaemonState::Running {
                    pending: 0,
                } => ("Running".to_owned(), true),
                DaemonState::Running {
                    pending,
                } => (format!("Running, {pending} waiting for you here"), true),
                DaemonState::Stopped => ("Not running".to_owned(), false),
                DaemonState::Unknown(e) => (e.clone(), false),
            };
            let git = if s.git_routed.is_empty() {
                "Direct to the git hosts".to_owned()
            } else {
                format!("{} through Reins", s.git_routed.join(", "))
            };
            let mut svc = card(pal).child(info_row("Background service", &service, pal, true));
            if !running {
                svc = svc.child(
                    row(pal, false).child(div().flex_1()).child(
                        button("start-service", "Start", pal, true, true)
                            .on_click(Self::on_model(cx, Model::start_service)),
                    ),
                );
            }
            svc = svc.child(info_row("git", &git, pal, false));
            page = page.child(div().flex().flex_col().child(section_title("THIS COMPUTER", pal)).child(svc));
        }

        if let Some(n) = notice {
            page = page.child(caption(n, pal).text_color(pal.danger));
        }

        // Footer.
        page = page.child(
            div()
                .flex()
                .flex_col()
                .gap(px(10.0))
                .child(
                    div()
                        .id("autostart")
                        .flex()
                        .items_center()
                        .gap(px(10.0))
                        .cursor_pointer()
                        .child(toggle(autostart, pal))
                        .child(caption("Open Reins when you log in", pal).text_color(pal.text))
                        .on_click(Self::on_model(cx, Model::toggle_autostart)),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(16.0))
                        .child(
                            link("cli", "Install command line tool", pal)
                                .on_click(Self::on_model(cx, Model::install_cli)),
                        )
                        .child(div().flex_1())
                        .child(
                            link("quit", "Quit Reins", pal).text_color(pal.secondary).on_click(|_, _, cx| cx.quit()),
                        ),
                )
                .child(
                    caption(
                        format!(
                            "Reins {}. Quitting hides the shield; the background service keeps running.",
                            reins_desktop::update::VERSION
                        ),
                        pal,
                    )
                    .text_size(px(11.0))
                    .text_color(pal.tertiary),
                ),
        );
        page.into_any_element()
    }
}

/// A note that something needs doing first.
fn warning(text: &'static str, pal: Palette) -> Div {
    card(pal)
        .w_full()
        .p(px(12.0))
        .bg(pal.warning.opacity(0.1))
        .border_color(pal.warning.opacity(0.3))
        .child(caption(text, pal).text_color(pal.text))
}

fn info_row(label: &str, value: &str, pal: Palette, first: bool) -> Div {
    row(pal, first)
        .justify_between()
        .child(div().text_color(pal.secondary).child(label.to_owned()))
        .child(div().text_color(pal.text).font_weight(FontWeight::MEDIUM).child(value.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hosts_are_shown_without_the_scheme() {
        assert_eq!(host("https://app.reins2fa.com"), "app.reins2fa.com");
        assert_eq!(host("http://localhost:8000/"), "localhost:8000");
        assert_eq!(host("reins.example.org"), "reins.example.org");
    }

    #[test]
    fn setup_items_say_what_they_do() {
        use crate::model::SetupWhat;
        let item = crate::model::SetupItem {
            what: SetupWhat::Harness(Harness::Codex),
            checked: false,
            available: false,
        };
        assert_eq!(item.label(), "Codex");
        assert!(item.detail().contains("Not installed"));
    }
}
