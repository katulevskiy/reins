//! The health checks (`reins doctor` in the window: the overall state, what to look at and a button where the window
//! can fix it), opened from Home's Health tile; and "Send a test" (`reins test`), on Home, in the welcome
//! flow's last step and in an empty Activity.

use gpui::{
    AnyElement, Context, Div, FontWeight, IntoElement, ParentElement as _, SharedString,
    StatefulInteractiveElement as _, Styled as _, div, px,
};
use reins_desktop::doctor::Level;

use super::icons::{Icon, icon};
use super::parts::{Btn, big, btn, btn_icon, caption, card, fine, group, help, level_mark, row, row_text, waiting_dot};
use super::{Data, Root};
use crate::format;
use crate::health::{self, Test};
use crate::model::Model;
use crate::shortcuts;
use crate::theme::Palette;

/// "Checked just now", "Checked 3 min ago".
pub fn checked(d: &Data) -> Option<String> {
    d.health.at.map(|at| format!("Checked {}", format::ago(d.now, at)))
}

impl Root {
    /// The "Check again" button (Checking… while the checks run).
    pub(super) fn check_again(
        id: &'static str,
        label: &'static str,
        d: &Data,
        pal: Palette,
        cx: &mut Context<'_, Self>,
    ) -> AnyElement {
        if d.health.running {
            btn(id, "Checking…", Btn::Plain, pal).opacity(0.6).cursor_default().into_any_element()
        } else {
            btn(id, label, Btn::Plain, pal).on_click(Self::on_model(cx, Model::run_checks)).into_any_element()
        }
    }

    /// The checks, opened from the Health tile: the overall state, then each check that is not OK with its fix.
    pub(super) fn health_card(d: &Data, pal: Palette, cx: &mut Context<'_, Self>) -> AnyElement {
        let checks = d.health.checks.as_deref();
        let (level, title, detail) = match (checks, &d.health.error) {
            (_, Some(e)) => (Level::Fail, "Cannot run the checks".to_owned(), e.clone()),
            (Some(c), None) => {
                let s = health::summary(c);
                (s.level, s.title, s.detail)
            }
            (None, None) => (Level::Skip, "Checking…".to_owned(), String::new()),
        };
        let mark = if checks.is_none() && d.health.error.is_none() {
            div()
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .size(px(30.0))
                .rounded(px(9.0))
                .bg(pal.violet().opacity(0.12))
                .child(waiting_dot("health-first", pal.violet(), 8.0))
        } else {
            level_mark(level, 30.0, pal)
        };
        let under = [Some(detail), checked(d)].into_iter().flatten().filter(|s| !s.is_empty()).collect::<Vec<_>>();
        let head = row(pal, true)
            .child(mark)
            .child(row_text(title, (!under.is_empty()).then(|| under.join(" · ").into_any_element()), pal))
            .child(Self::check_again("check-again", "Check again", d, pal, cx));

        let mut body = card(pal).child(head);
        for (i, c) in checks.map(health::to_look_at).unwrap_or_default().into_iter().enumerate() {
            let fix = health::fix_for(c);
            let mut text = div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w(px(0.0))
                .gap(px(2.0))
                .child(div().font_weight(FontWeight::MEDIUM).child(c.label.clone()))
                .child(caption(c.detail.clone(), pal));
            // The window's button says it better than the command line's way.
            if fix.is_none()
                && let Some(how) = &c.fix
            {
                text = text.child(
                    div()
                        .flex()
                        .items_start()
                        .gap(px(6.0))
                        .child(div().pt(px(2.0)).child(icon(Icon::ArrowRight, 12.0, pal.ink_meta())))
                        .child(fine(how.clone(), pal)),
                );
            }
            let mut line = row(pal, false).items_start().child(level_mark(c.level, 30.0, pal)).child(text);
            if let Some(fix) = fix {
                line = line.child(
                    btn(SharedString::from(format!("fix-{i}-{}", c.id)), fix.label(), Btn::Accent, pal)
                        .on_click(Self::on_model(cx, move |m, cx| m.fix(fix, cx))),
                );
            }
            body = body.child(line);
        }
        group(
            "Health checks",
            Some(help(
                "health",
                format!(
                    "Reins checks every minute that your agents reach your phone ({} checks now).",
                    shortcuts::hint("r")
                ),
                d,
                pal,
                cx,
            )),
            body,
            pal,
        )
        .into_any_element()
    }

    /// "Send a test" and how it went. `inline`: in a page among other things (a normal button), not the one thing on
    /// a screen.
    pub(super) fn test_panel(d: &Data, inline: bool, pal: Palette, cx: &mut Context<'_, Self>) -> Div {
        let (line, button_label, enabled): (AnyElement, &str, bool) = match &d.test {
            Test::Idle => (
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.0))
                    .child(super::parts::tile(Icon::Send, pal.violet(), 30.0))
                    .child(div().font_weight(FontWeight::MEDIUM).child("See it work"))
                    .child(help(
                        "test",
                        "Sends a harmless question to your phone. Approve or deny it; nothing happens either way.",
                        d,
                        pal,
                        cx,
                    ))
                    .into_any_element(),
                "Send a test",
                d.paired,
            ),
            Test::Waiting(sent) => {
                let left = health::seconds_left(sent.elapsed());
                (
                    div()
                        .flex()
                        .items_center()
                        .gap(px(10.0))
                        .child(
                            div()
                                .flex()
                                .flex_none()
                                .items_center()
                                .justify_center()
                                .size(px(30.0))
                                .rounded(px(9.0))
                                .bg(pal.violet().opacity(0.12))
                                .child(waiting_dot("test-waiting", pal.violet(), 8.0)),
                        )
                        .child(row_text("Check your phone", Some(format!("{left} s left").into_any_element()), pal))
                        .into_any_element(),
                    "Waiting…",
                    false,
                )
            }
            Test::Ended(answer, timed_out) => {
                let here = inline && d.section == crate::model::Section::Home;
                let (ok, text) = health::test_result(answer, *timed_out, here);
                let color = if ok {
                    pal.success
                } else {
                    pal.danger
                };
                (
                    div()
                        .flex()
                        .items_center()
                        .gap(px(10.0))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(color)
                        .child(super::parts::tile(
                            if ok {
                                Icon::CircleCheck
                            } else {
                                Icon::CircleX
                            },
                            color,
                            30.0,
                        ))
                        .child(div().flex_1().min_w(px(0.0)).child(text))
                        .into_any_element(),
                    "Send another",
                    d.paired,
                )
            }
        };
        let b = btn_icon(
            "send-test",
            Icon::Send,
            button_label,
            if matches!(d.test, Test::Idle) {
                Btn::Accent
            } else {
                Btn::Plain
            },
            pal,
        );
        let b = if inline {
            b
        } else {
            big(b)
        };
        div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(14.0))
            .px(px(14.0))
            .py(px(12.0))
            .child(div().flex_1().min_w(px(240.0)).child(line))
            .child(if enabled {
                b.on_click(Self::on_model(cx, Model::send_test))
            } else {
                b.opacity(0.5).cursor_default()
            })
    }
}
