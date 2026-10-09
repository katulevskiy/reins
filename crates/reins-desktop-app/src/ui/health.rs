//! The health card (`reins doctor` in the window: the overall state, what to look at and a button where the window
//! can fix it) and "Send a test to my phone" (`reins test`), on Overview, in the welcome flow's last step and in an
//! empty Activity.

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, Context, Div, FontWeight, IntoElement, ParentElement as _, SharedString,
    StatefulInteractiveElement as _, Styled as _, div, px,
};
use reins_desktop::doctor::Level;

use super::parts::{big, button, caption, card, check, fine, group, level_mark, row, waiting_dot};
use super::{Data, Root};
use crate::format;
use crate::health::{self, Test};
use crate::model::Model;
use crate::shortcuts;
use crate::theme::Palette;

/// "Checked just now", "Checked 3 min ago".
fn checked(d: &Data) -> Option<String> {
    d.health.at.map(|at| format!("Checked {}", format::ago(d.now, at)))
}

impl Root {
    /// The "Check again" button (Running… while the checks run).
    fn check_again(
        id: &'static str,
        label: &'static str,
        d: &Data,
        pal: Palette,
        cx: &mut Context<'_, Self>,
    ) -> AnyElement {
        if d.health.running {
            button(id, "Checking…", pal, false, false).into_any_element()
        } else {
            button(id, label, pal, false, true).on_click(Self::on_model(cx, Model::run_checks)).into_any_element()
        }
    }

    /// Overview's health card: the overall state, then each check that is not OK with its fix, then the test.
    pub(super) fn health_card(d: &Data, pal: Palette, cx: &mut Context<'_, Self>) -> AnyElement {
        let checks = d.health.checks.as_deref();
        let (level, title, detail) = match (checks, &d.health.error) {
            (_, Some(e)) => (Level::Fail, "Cannot run the checks".to_owned(), e.clone()),
            (Some(c), None) => {
                let s = health::summary(c);
                (s.level, s.title, s.detail)
            }
            (None, None) => (
                Level::Skip,
                "Checking…".to_owned(),
                "Is everything set up so your agents reach your phone?".to_owned(),
            ),
        };
        let mark = if checks.is_none() && d.health.error.is_none() {
            div()
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .size(px(30.0))
                .rounded_full()
                .bg(pal.accent_soft)
                .child(waiting_dot("health-first", pal.accent, 8.0))
        } else {
            level_mark(level, 30.0, pal)
        };
        let head = row(pal, true)
            .py(px(14.0))
            .child(mark)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w(px(0.0))
                    .gap(px(1.0))
                    .child(div().text_size(px(15.0)).font_weight(FontWeight::SEMIBOLD).child(title))
                    .child(caption(detail, pal)),
            )
            .when_some(checked(d), |r, at| r.child(fine(at, pal).flex_none()))
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
                text = text.child(fine(format!("→ {how}"), pal));
            }
            let mut line = row(pal, false)
                .items_start()
                .py(px(12.0))
                .pl(px(22.0))
                .child(div().pt(px(1.0)).child(level_mark(c.level, 18.0, pal)));
            line = line.child(text);
            if let Some(fix) = fix {
                line = line.child(
                    button(SharedString::from(format!("fix-{i}-{}", c.id)), fix.label(), pal, true, true)
                        .on_click(Self::on_model(cx, move |m, cx| m.fix(fix, cx))),
                );
            }
            body = body.child(line);
        }
        body = body.child(div().border_t_1().border_color(pal.hairline).child(Self::test_panel(d, true, pal, cx)));
        group(
            "Health",
            Some(&format!(
                "Reins checks every minute that your agents reach your phone ({} checks now).",
                shortcuts::hint("r")
            )),
            body,
            pal,
        )
        .into_any_element()
    }

    /// "Send a test to my phone" and how it went. `checks_above`: the health checks are above it on the page.
    pub(super) fn test_panel(d: &Data, checks_above: bool, pal: Palette, cx: &mut Context<'_, Self>) -> Div {
        let (line, button_label, enabled): (AnyElement, &str, bool) = match &d.test {
            Test::Idle => (
                div()
                    .flex()
                    .flex_col()
                    .gap(px(2.0))
                    .child(div().font_weight(FontWeight::MEDIUM).child("See it work"))
                    .child(caption(
                        "Sends a harmless question to your phone. Approve or deny it; nothing happens either way.",
                        pal,
                    ))
                    .into_any_element(),
                "Send a test to my phone",
                d.paired,
            ),
            Test::Waiting(sent) => {
                let left = health::seconds_left(sent.elapsed());
                (
                    div()
                        .flex()
                        .items_start()
                        .gap(px(10.0))
                        .child(div().pt(px(5.0)).child(waiting_dot("test-waiting", pal.accent, 8.0)))
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .gap(px(2.0))
                                .child(
                                    div()
                                        .font_weight(FontWeight::MEDIUM)
                                        .child("Sent. Open Reins on your phone and approve or deny it…"),
                                )
                                .child(fine(format!("{left} s left"), pal)),
                        )
                        .into_any_element(),
                    "Waiting…",
                    false,
                )
            }
            Test::Ended(answer, timed_out) => {
                let (ok, text) = health::test_result(answer, *timed_out, checks_above);
                let color = if ok {
                    pal.success
                } else {
                    pal.danger
                };
                // The ✓ the line starts with is drawn (Geist has none).
                let text = text.strip_prefix("✓ ").map(str::to_owned).unwrap_or(text);
                let mark = if ok {
                    check(16.0, color).into_any_element()
                } else {
                    level_mark(Level::Fail, 16.0, pal).into_any_element()
                };
                (
                    div()
                        .flex()
                        .items_start()
                        .gap(px(8.0))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(color)
                        .child(div().pt(px(1.0)).child(mark))
                        .child(div().flex_1().min_w(px(0.0)).child(text))
                        .into_any_element(),
                    "Send another",
                    d.paired,
                )
            }
        };
        let b = button("send-test", button_label, pal, matches!(d.test, Test::Idle), enabled);
        let b = if checks_above {
            b
        } else {
            big(b)
        };
        div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(16.0))
            .px(px(16.0))
            .py(px(14.0))
            .child(div().flex_1().min_w(px(240.0)).child(line))
            .child(if enabled {
                b.on_click(Self::on_model(cx, Model::send_test))
            } else {
                b
            })
    }

    /// Settings' "Run checks": the overall state in a line and the button.
    pub(super) fn checks_row(d: &Data, pal: Palette, cx: &mut Context<'_, Self>) -> AnyElement {
        let line = match (&d.health.checks, &d.health.error) {
            (_, Some(e)) => e.clone(),
            (Some(c), None) => {
                let s = health::summary(c);
                format!("{}. {}", s.title, s.detail)
            }
            (None, None) => "Not checked yet.".to_owned(),
        };
        let level = d.health.checks.as_deref().map(reins_desktop::doctor::overall);
        card(pal)
            .child(
                row(pal, true)
                    .when_some(level, |r, l| r.child(level_mark(l, 22.0, pal)))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_w(px(0.0))
                            .gap(px(1.0))
                            .child(div().font_weight(FontWeight::MEDIUM).child("Health checks"))
                            .child(caption(line, pal))
                            .when_some(checked(d), |c, at| c.child(fine(at, pal))),
                    )
                    .child(Self::check_again("run-checks", "Run checks", d, pal, cx)),
            )
            .into_any_element()
    }
}
