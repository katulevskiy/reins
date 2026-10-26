//! The health checks (`reins doctor` in the window), opened from Home's Health plate (`Desk-Extras.png`): "1 thing
//! *to look at*" in the serif with Check again, then every check as a plain row with a word for its state and, where
//! the window can, the button that fixes it. And "Send a test" (`reins test`): how it went, on Home, in the welcome
//! flow's last step and in an empty Activity.

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, Context, Div, FontWeight, IntoElement, ParentElement as _, SharedString,
    StatefulInteractiveElement as _, Styled as _, div, px,
};
use reins_desktop::doctor::Level;

use super::icons::{Glyph, glyph};
use super::parts::{help, italic, waiting_dot};
use super::story::{Words, bold, plain};
use super::{Data, Root, kit};
use crate::format;
use crate::health::{self, Test};
use crate::model::Model;
use crate::shortcuts;
use crate::theme::{MONO, Palette, DISPLAY, Tone};

/// "Checked just now", "Checked 3 min ago".
pub fn checked(d: &Data) -> Option<String> {
    d.health.at.map(|at| format!("Checked {}", format::ago(d.now, at)))
}

/// The checks' title in the serif, its last words in italic: "1 thing *to look at*", "All *good*".
#[must_use]
pub fn headline(title: &str) -> Vec<Words> {
    for tail in [" to look at", " to fix", " good", " fixing"] {
        if let Some(head) = title.strip_suffix(tail) {
            return vec![plain(format!("{head} ")), bold(tail.trim_start())];
        }
    }
    vec![plain(title)]
}

/// A check's state as a glyph in its colour.
fn level_glyph(level: Level, pal: Palette) -> AnyElement {
    let (g, color) = match level {
        Level::Ok => (Glyph::Check, pal.success),
        Level::Warn => (Glyph::Warning, pal.warning),
        Level::Fail => (Glyph::CircleClose, pal.danger),
        Level::Skip => (Glyph::Minus, pal.ink_faint()),
    };
    glyph(g, 16.0, color).into_any_element()
}

impl Root {
    /// "Check again" (Checking… while the checks run).
    pub(super) fn check_again(
        id: &'static str,
        label: &'static str,
        d: &Data,
        pal: Palette,
        cx: &mut Context<'_, Self>,
    ) -> AnyElement {
        let b = |label: &'static str| kit::button(id, kit::Look::Soft, kit::Size::Small, None, label, pal).h(px(34.0));
        if d.health.running {
            kit::disabled(b("Checking…")).into_any_element()
        } else {
            b(label).on_click(Self::on_model(cx, Model::run_checks)).into_any_element()
        }
    }

    /// The checks, opened from the Health plate: the summary, then every check.
    pub(super) fn health_card(d: &Data, pal: Palette, cx: &mut Context<'_, Self>) -> AnyElement {
        let checks = d.health.checks.as_deref();
        let title = match (checks, &d.health.error) {
            (_, Some(_)) => "Cannot run the checks".to_owned(),
            (Some(c), None) => health::summary(c).title,
            (None, None) => "Checking…".to_owned(),
        };
        let head = div()
            .flex()
            .items_center()
            .gap(px(12.0))
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .font_family(DISPLAY)
                    .text_size(px(32.0))
                    .line_height(px(34.0))
                    .text_color(pal.ink())
                    .child(italic(&headline(&title))),
            )
            .child(help(
                "health",
                format!(
                    "Reins checks every minute that your agents reach your phone ({} checks now).",
                    shortcuts::hint("r")
                ),
                d,
                pal,
                cx,
            ))
            .child(Self::check_again("check-again", "Check again", d, pal, cx));
        let mut rows = div().flex().flex_col();
        if let Some(e) = &d.health.error {
            rows = rows.child(div().text_size(px(13.0)).text_color(pal.danger).child(e.clone()));
        } else if checks.is_none() {
            rows = rows.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .text_size(px(13.0))
                    .text_color(pal.ink_meta())
                    .child(waiting_dot("health-first", pal.violet(), 8.0))
                    .child("Checking…"),
            );
        }
        // What to look at first, then what passed.
        let mut list: Vec<&reins_desktop::doctor::Check> = checks.unwrap_or_default().iter().collect();
        list.sort_by_key(|c| std::cmp::Reverse(c.level));
        let n = list.len();
        for (i, c) in list.into_iter().enumerate() {
            let fix = health::fix_for(c);
            let flagged = matches!(c.level, Level::Warn | Level::Fail);
            let value_color = match c.level {
                Level::Warn => pal.warning,
                Level::Fail => pal.danger,
                Level::Ok | Level::Skip => pal.ink_meta(),
            };
            let mono = c.detail.chars().any(|ch| ch.is_ascii_digit())
                && c.detail.contains(['.', ':'])
                && !c.detail.contains(' ');
            let mut row = div()
                .flex()
                .items_center()
                .gap(px(12.0))
                .min_h(px(46.0))
                .py(px(4.0))
                .text_size(px(14.0))
                .when(i + 1 < n, |r| r.border_b_1().border_color(pal.rule()))
                .child(level_glyph(c.level, pal))
                .child(div().flex_1().min_w(px(0.0)).truncate().text_color(pal.ink()).child(c.label.clone()))
                .child(
                    div()
                        .min_w(px(0.0))
                        .max_w(gpui::relative(0.5))
                        .truncate()
                        .text_size(px(12.5))
                        .text_color(value_color)
                        .when(mono, |v| v.font_family(MONO))
                        .child(c.detail.clone()),
                );
            if let Some(fix) = fix {
                row = row.child(
                    kit::button(
                        SharedString::from(format!("fix-{i}-{}", c.id)),
                        kit::Look::Soft,
                        kit::Size::Small,
                        None,
                        fix.label(),
                        pal,
                    )
                    .on_click(Self::on_model(cx, move |m, cx| m.fix(fix, cx))),
                );
            } else if flagged && let Some(how) = &c.fix {
                row = row.child(help_text(i, how, d, pal, cx));
            }
            rows = rows.child(row);
        }
        kit::plate(pal).p(px(24.0)).gap(px(16.0)).child(head).child(rows).into_any_element()
    }

    /// "Send a test" and how it went. `inline`: in a page among other things.
    pub(super) fn test_panel(d: &Data, inline: bool, pal: Palette, cx: &mut Context<'_, Self>) -> Div {
        let (line, button_label, enabled): (AnyElement, &str, bool) = match &d.test {
            Test::Idle => (
                div()
                    .flex()
                    .flex_col()
                    .child(div().text_size(px(14.5)).font_weight(FontWeight::MEDIUM).child("See it work"))
                    .child(
                        div()
                            .text_size(px(12.5))
                            .line_height(px(17.0))
                            .text_color(pal.ink_meta())
                            .child("Sends a harmless question to your phone"),
                    )
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
                                .size(px(32.0))
                                .rounded(px(10.0))
                                .bg(pal.violet().opacity(0.12))
                                .child(waiting_dot("test-waiting", pal.violet(), 8.0)),
                        )
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .child(
                                    div().text_size(px(14.5)).font_weight(FontWeight::MEDIUM).child("Check your phone"),
                                )
                                .child(
                                    div()
                                        .text_size(px(12.5))
                                        .text_color(pal.ink_meta())
                                        .child(format!("{left} s left")),
                                ),
                        )
                        .into_any_element(),
                    "Waiting…",
                    false,
                )
            }
            Test::Ended(answer, timed_out) => {
                let here = inline && d.section == crate::model::Section::Home;
                let (ok, text) = health::test_result(answer, *timed_out, here);
                let (g, tone) = if ok {
                    (Glyph::CircleCheck, Tone::Success)
                } else {
                    (Glyph::CircleClose, Tone::Field(crate::theme::Field::Delete))
                };
                (
                    div()
                        .flex()
                        .items_center()
                        .gap(px(10.0))
                        .text_size(px(14.0))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(pal.tone(tone))
                        .child(kit::kind_tile(g, tone, 32.0, pal))
                        .child(div().flex_1().min_w(px(0.0)).child(text))
                        .into_any_element(),
                    "Send another",
                    d.paired,
                )
            }
        };
        let b = kit::button("send-test", kit::Look::Soft, kit::Size::Small, None, button_label, pal);
        div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(14.0))
            .px(px(if inline {
                16.0
            } else {
                0.0
            }))
            .py(px(12.0))
            .child(div().flex_1().min_w(px(240.0)).child(line))
            .child(if enabled {
                b.on_click(Self::on_model(cx, Model::send_test))
            } else {
                kit::disabled(b)
            })
    }
}

/// The (?)s of the checks fixed by hand, by row.
const FIX_HELP: [&str; 8] = [
    "health-fix-0",
    "health-fix-1",
    "health-fix-2",
    "health-fix-3",
    "health-fix-4",
    "health-fix-5",
    "health-fix-6",
    "health-fix-7",
];

/// A check's way to fix it by hand, behind a (?).
fn help_text(i: usize, how: &str, d: &Data, pal: Palette, cx: &Context<'_, Root>) -> AnyElement {
    let id = FIX_HELP[i.min(FIX_HELP.len() - 1)];
    help(id, how.to_owned(), d, pal, cx).into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn headlines_put_their_last_words_in_italic() {
        let said = |t: &str| {
            headline(t)
                .iter()
                .map(|w| {
                    if w.bold {
                        format!("*{}*", w.text)
                    } else {
                        w.text.clone()
                    }
                })
                .collect::<String>()
        };
        assert_eq!(said("1 thing to look at"), "1 thing *to look at*");
        assert_eq!(said("All good"), "All *good*");
        assert_eq!(said("Needs fixing"), "Needs *fixing*");
        assert_eq!(said("Checking…"), "Checking…");
    }
}
