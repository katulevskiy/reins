//! Overview: the state in a sentence, what waits for the phone now, today's answers, pausing, the latest requests.

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, Context, FontWeight, IntoElement, ParentElement, SharedString, StatefulInteractiveElement as _,
    Styled as _, div, px,
};
use reins_desktop::journal::{Entry, Tally};

use super::parts::{
    button, caption, card, fine, group, link, look_color, one_line, phone_qr, segment, segmented, stat_tile,
    waiting_dot,
};
use super::{Data, Root};
use crate::format;
use crate::model::{Model, Section};
use crate::pause::{Pause, PauseFor};
use crate::theme::Palette;
use crate::tray::Look;

/// How many recent requests the overview shows.
const RECENT: usize = 6;

/// Today's requests (since local midnight).
#[must_use]
pub fn today(entries: &[Entry], now: i64) -> Tally {
    let start = format::start_of_day(now);
    Tally::of(entries.iter().filter(|e| e.at >= start))
}

impl Root {
    pub(super) fn overview(&mut self, d: &Data, pal: Palette, cx: &mut Context<'_, Self>) -> AnyElement {
        let mut page = div().flex().flex_col().gap(px(22.0)).child(Self::hero(d, pal, cx));
        if d.show_qr {
            page = page.child(phone_qr(pal));
        }
        if let Some(banner) = Self::update_banner(d, pal, cx) {
            page = page.child(banner);
        }
        page = page.when_some(d.notice.clone(), |p, n| p.child(caption(n, pal).text_color(pal.danger)));
        page = page.when_some(d.note.clone(), |p, n| p.child(caption(n, pal)));

        // Waiting on the phone.
        let waiting: Vec<&Entry> = d.waiting().collect();
        if !waiting.is_empty() {
            let mut list = card(pal).border_color(pal.accent.opacity(0.35));
            for (i, e) in waiting.iter().enumerate() {
                let who = e.source.clone().unwrap_or_else(|| e.kind.label().to_owned());
                list = list.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(12.0))
                        .px(px(16.0))
                        .py(px(12.0))
                        .min_w(px(0.0))
                        .when(i > 0, |r| r.border_t_1().border_color(pal.hairline))
                        .child(waiting_dot(SharedString::from(format!("wait-{}", e.id)), pal.accent, 9.0))
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .flex_1()
                                .min_w(px(0.0))
                                .child(one_line(e.what.clone()).font_weight(FontWeight::MEDIUM))
                                .child(
                                    one_line(format!("{who} · {}", e.kind.label()))
                                        .text_size(px(12.0))
                                        .text_color(pal.secondary),
                                ),
                        )
                        .child(
                            div()
                                .flex_none()
                                .text_size(px(12.0))
                                .text_color(pal.accent)
                                .font_weight(FontWeight::MEDIUM)
                                .child(format!("waiting {}", format::duration(d.now - e.at))),
                        ),
                );
            }
            page = page.child(group(
                "Waiting on your phone",
                Some("Open Reins on your phone to approve or deny. Nothing goes ahead until you answer."),
                list,
                pal,
            ));
        }

        // Today.
        let t = today(d.activity(), d.now);
        page = page.child(group(
            "Today on this computer",
            None,
            div()
                .flex()
                .gap(px(10.0))
                .child(stat_tile(t.approved, "Approved", pal.success, pal))
                .child(stat_tile(t.denied, "Denied", pal.danger, pal))
                .child(stat_tile(t.timed_out, "Timed out", pal.warning, pal))
                .child(stat_tile(t.failed, "Failed", pal.tertiary, pal)),
            pal,
        ));

        page = page.child(Self::pause_card(d, pal, cx));

        // The latest requests.
        let recent: Vec<&Entry> = d.activity().iter().take(RECENT).collect();
        let body = if recent.is_empty() {
            card(pal).child(super::parts::empty(
                "Nothing asked yet. Requests from your AI tools, git and ssh on this computer show up here.",
                pal,
            ))
        } else {
            let mut list = card(pal);
            for (i, e) in recent.iter().enumerate() {
                list = list.child(self.entry_row(e, "overview", d, pal, i == 0, cx));
            }
            list
        };
        page = page.child(
            div()
                .flex()
                .flex_col()
                .gap(px(8.0))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .px(px(2.0))
                        .child(div().flex_1().text_size(px(13.5)).font_weight(FontWeight::SEMIBOLD).child("Latest"))
                        .when(!recent.is_empty(), |r| {
                            r.child(link("see-all", "See all activity", pal).on_click(Self::go(cx, Section::Activity)))
                        }),
                )
                .child(body),
        );
        page.into_any_element()
    }

    /// The state in a sentence, with what fixes it.
    fn hero(d: &Data, pal: Palette, cx: &mut Context<'_, Self>) -> AnyElement {
        let color = look_color(d.look, pal);
        let sub = match d.look {
            Look::On => "Your phone approves what your AI agents do on this computer.",
            Look::Paused if d.pause == Pause::Manual => {
                "git goes straight to the hosts until you resume. Hooks and MCP tools still ask your phone."
            }
            Look::Paused if d.pause.on() => {
                "git goes straight to the hosts until the pause ends. Hooks and MCP tools still ask your phone."
            }
            Look::Paused => "git goes straight to the hosts. Resume to send it through Reins again.",
            Look::Attention if !d.paired => "Pair this computer with your phone again.",
            Look::Attention => "Start the background service so git and your agents reach your phone.",
        };
        let action = match d.look {
            Look::Paused => {
                Some(button("hero-resume", "Resume", pal, true, true).on_click(Self::on_model(cx, Model::resume)))
            }
            Look::Attention if !d.paired => {
                Some(button("hero-pair", "Pair again", pal, true, true).on_click(Self::on_model(cx, Model::sign_out)))
            }
            Look::Attention => Some(
                button("hero-start", "Start service", pal, true, true)
                    .on_click(Self::on_model(cx, Model::start_service)),
            ),
            Look::On => None,
        };
        // The buttons go under the sentence when the window is narrow.
        card(pal)
            .flex_row()
            .flex_wrap()
            .items_center()
            .gap(px(16.0))
            .px(px(20.0))
            .py(px(18.0))
            .child(
                div()
                    .flex()
                    .flex_1()
                    .items_center()
                    .gap(px(16.0))
                    .min_w(px(300.0))
                    .child(
                        div()
                            .flex()
                            .flex_none()
                            .items_center()
                            .justify_center()
                            .size(px(40.0))
                            .rounded_full()
                            .bg(color.opacity(0.12))
                            .child(div().size(px(14.0)).rounded_full().bg(color)),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_w(px(0.0))
                            .gap(px(3.0))
                            .child(
                                div()
                                    .text_size(px(20.0))
                                    .line_height(px(26.0))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(d.line.clone()),
                            )
                            .child(caption(sub, pal).text_size(px(12.5))),
                    ),
            )
            .child(
                div().flex().flex_none().items_center().gap(px(8.0)).when_some(action, ParentElement::child).child(
                    button(
                        "hero-qr",
                        if d.show_qr {
                            "Hide code"
                        } else {
                            "Activity on phone"
                        },
                        pal,
                        false,
                        true,
                    )
                    .on_click(Self::on_model(cx, Model::toggle_phone_qr)),
                ),
            )
            .into_any_element()
    }

    fn pause_card(d: &Data, pal: Palette, cx: &mut Context<'_, Self>) -> AnyElement {
        let (title, about, label) = if d.pause.on() {
            (
                "Change the pause",
                "A new length starts now and replaces the pause. Resume sends git through Reins again.",
                "Pause instead for",
            )
        } else {
            (
                "Pause git",
                "Pausing sends git straight to GitHub and the other hosts. Hooks and MCP tools still ask your phone.",
                "Pause for",
            )
        };
        let mut choices = segmented(pal);
        for length in PauseFor::ALL {
            let s = segment(SharedString::from(format!("pause-{}", length.id())), length.short(), false, pal);
            choices = choices.child(if d.paired {
                s.on_click(Self::on_model(cx, move |m, cx| m.pause_for(length, cx)))
            } else {
                s.opacity(0.45)
            });
        }
        let body = card(pal)
            .flex_row()
            .flex_wrap()
            .items_center()
            .gap(px(12.0))
            .px(px(16.0))
            .py(px(12.0))
            .child(div().flex_1().min_w(px(120.0)).child(fine(label, pal).text_size(px(12.5))))
            .child(choices);
        group(title, Some(about), body, pal).into_any_element()
    }

    fn update_banner(d: &Data, pal: Palette, cx: &mut Context<'_, Self>) -> Option<AnyElement> {
        let update = d.update.as_ref()?;
        let banner = card(pal)
            .p(px(12.0))
            .pl(px(16.0))
            .flex_row()
            .items_center()
            .gap(px(10.0))
            .bg(pal.accent_soft)
            .border_color(pal.accent.opacity(0.25));
        Some(
            if update.ready.is_some() {
                let label = if d.updating {
                    "Restarting…"
                } else {
                    "Restart to update"
                };
                let restart = button("restart-update", label, pal, true, !d.updating);
                banner.child(div().flex_1().child(format!("Reins {} is ready.", update.version))).child(if d.updating {
                    restart
                } else {
                    restart.on_click(Self::on_model(cx, Model::restart_to_update))
                })
            } else {
                banner.child(div().flex_1().child(format!("Reins {} is available.", update.version))).child(
                    link("download", "Download", pal).on_click(|_, _, cx| cx.open_url(&crate::links::download())),
                )
            }
            .into_any_element(),
        )
    }
}

#[cfg(test)]
mod tests {
    use reins_desktop::journal::{Kind, Outcome};

    use super::*;

    #[test]
    fn today_counts_only_since_midnight() {
        // Six hours into a local day, whatever the time zone.
        let now = format::start_of_day(1_800_000_000) + 6 * 3_600;
        let mut old = Entry::new(Kind::Git, "old");
        old.at = format::start_of_day(now) - 10;
        old.outcome = Outcome::Approved;
        let mut new = Entry::new(Kind::Git, "new");
        new.at = now - 5;
        new.outcome = Outcome::Approved;
        let mut denied = new.clone();
        denied.outcome = Outcome::Denied;
        let t = today(&[new, denied, old], now);
        assert_eq!((t.approved, t.denied), (1, 1));
    }
}
