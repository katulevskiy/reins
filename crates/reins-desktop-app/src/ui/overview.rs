//! Home (Overview before Signal): a hero first (the request waiting on the phone, with its timer and "Open on
//! phone"; or, when calm, this computer → the phone and what is protected), then three tiles (Health, Work session,
//! Today), what a tile opens (the checks, the work session form), and the latest requests as the phone shows them.

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, Context, FontWeight, InteractiveElement as _, IntoElement, ParentElement, SharedString,
    StatefulInteractiveElement as _, Styled as _, Window, div, px,
};
use reins_desktop::doctor::Level;
use reins_desktop::journal::{Entry, Tally};

use super::icons::{Icon, Mark, icon};
use super::parts::{
    Btn, btn, btn_icon, caption, card, chip, chip_plain, group_title, level_color, link, phone_qr, ring, sentence,
    small, stage, stat,
};
use super::sidebar::clock_left;
use super::story::{Ending, Story, ending};
use super::{Data, Root};
use crate::format;
use crate::health::{self, Test};
use crate::model::{Model, Section};
use crate::theme::{Palette, SERIF, Tone};
use crate::tray::Look;

/// How many recent requests the overview shows.
const RECENT: usize = 5;

/// Today's requests (since local midnight).
#[must_use]
pub fn today(entries: &[Entry], now: i64) -> Tally {
    let start = format::start_of_day(now);
    Tally::of(entries.iter().filter(|e| e.at >= start))
}

/// When a row's request was: "now", "4 min", "2 h", "Oct 3".
#[must_use]
pub fn short_ago(now: i64, at: i64) -> String {
    let secs = (now - at).max(0);
    match secs {
        0..60 => "now".to_owned(),
        60..3_600 => format!("{} min", secs / 60),
        3_600..86_400 => format!("{} h", secs / 3_600),
        _ => format::date(at),
    }
}

/// A Home tile's value: serif 30 ("All good", "Not running").
fn stat_value() -> gpui::Div {
    div().mt(px(8.0)).font_family(SERIF).text_size(px(30.0)).line_height(px(34.0)).font_weight(FontWeight::NORMAL)
}

/// A hero's frame: a plate on paper (Signal has no gradients; colour is for decisions, which phase 2 makes fields).
fn hero_frame(color: gpui::Hsla, pal: Palette) -> gpui::Div {
    let _ = color;
    div()
        .flex()
        .flex_wrap()
        .items_center()
        .gap(px(28.0))
        .px(px(28.0))
        .py(px(26.0))
        .rounded(px(20.0))
        .bg(pal.paper_raised())
        .border_1()
        .border_color(pal.rule())
}

impl Root {
    pub(super) fn overview(
        &mut self,
        d: &Data,
        pal: Palette,
        window: &Window,
        cx: &mut Context<'_, Self>,
    ) -> AnyElement {
        let mut page = div().flex().flex_col().gap(px(14.0)).child(Self::hero(d, pal, cx));
        if d.show_qr {
            page = page.child(phone_qr(pal));
        }
        if let Some(banner) = Self::update_banner(d, pal, cx) {
            page = page.child(banner);
        }
        page = page.when_some(d.notice.clone(), |p, n| p.child(caption(n, pal).px(px(4.0)).text_color(pal.danger)));
        page = page.when_some(d.note.clone(), |p, n| p.child(caption(n, pal).px(px(4.0))));
        if !matches!(d.test, Test::Idle) {
            page = page.child(card(pal).child(Self::test_panel(d, true, pal, cx)));
        }

        // The tiles.
        page = page.child(
            div()
                .flex()
                .flex_wrap()
                .gap(px(10.0))
                .child(self.health_tile(d, pal, cx))
                .child(Self::work_tile(d, pal, cx))
                .child(Self::today_tile(d, pal)),
        );
        // What a tile opens.
        if self.health_open {
            page = page.child(Self::health_card(d, pal, cx));
        }
        if d.work_session().is_none() && d.work.asking.is_none() && d.work.form.is_some() {
            page = page.child(self.work_card(d, pal, window, cx));
        }

        // The latest requests.
        let recent: Vec<&Entry> = d.activity().iter().take(RECENT).collect();
        let body = if recent.is_empty() {
            card(pal).child(super::parts::empty("Nothing asked yet", pal))
        } else {
            let mut list = card(pal);
            for (i, e) in recent.iter().enumerate() {
                list = list.child(
                    Self::history_row(e, "overview", d, pal, i == 0, false)
                        .on_click(Self::open_entry(cx, e.id.clone())),
                );
            }
            list
        };
        page = page.child(
            div()
                .flex()
                .flex_col()
                .gap(px(8.0))
                .pt(px(6.0))
                .child(group_title("Latest", pal).when(!recent.is_empty(), |r| {
                    r.child(link("see-all", "See all", pal).on_click(Self::go(cx, Section::Activity)))
                }))
                .child(body),
        );
        page.into_any_element()
    }

    /// One request as the phone lists it: a coloured kind square, a line, who and what kind, when, how it ended.
    /// `scope` keeps ids apart where the same entry shows twice.
    pub(super) fn history_row(
        e: &Entry,
        scope: &'static str,
        d: &Data,
        pal: Palette,
        first: bool,
        selected: bool,
    ) -> gpui::Stateful<gpui::Div> {
        let s = Story::of(e);
        let color = pal.tone(s.tone);
        let end: AnyElement = match ending(e) {
            Ending::Icon(i, tone) => icon(i, 16.0, pal.tone(tone)).into_any_element(),
            Ending::Rules => chip_plain(None, "By your rules", pal).h(px(22.0)).px(px(8.0)).into_any_element(),
            Ending::Waiting => chip(None, "Waiting", pal.accent).h(px(22.0)).px(px(9.0)).into_any_element(),
        };
        div()
            .id(SharedString::from(format!("{scope}-{}", e.id)))
            .relative()
            .flex()
            .items_center()
            .gap(px(12.0))
            .px(px(14.0))
            .py(px(10.0))
            .min_w(px(0.0))
            .cursor_pointer()
            .when(!first, |r| r.border_t_1().border_color(pal.hairline))
            .when_else(
                selected,
                |r| {
                    r.bg(pal.accent.opacity(0.10))
                        .child(div().absolute().left_0().top_0().bottom_0().w(px(3.0)).bg(pal.violet_fill()))
                },
                |r| r.hover(|h| h.bg(pal.control_fill.opacity(0.6))),
            )
            .child(super::parts::tile(s.icon, color, 30.0))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w(px(0.0))
                    .gap(px(2.0))
                    .child(
                        div()
                            .min_w(px(0.0))
                            .truncate()
                            .text_size(px(13.5))
                            .font_weight(FontWeight::MEDIUM)
                            .child(s.title),
                    )
                    .child(div().min_w(px(0.0)).truncate().text_size(px(12.0)).text_color(pal.tertiary).child(s.who)),
            )
            .child(div().flex_none().text_size(px(12.0)).text_color(pal.tertiary).child(short_ago(d.now, e.at)))
            .child(div().flex().flex_none().items_center().child(end))
    }

    /// The hero: the request waiting (the oldest first), else what Reins does for this computer.
    fn hero(d: &Data, pal: Palette, cx: &mut Context<'_, Self>) -> AnyElement {
        if let Some(e) = d.waiting().last() {
            return Self::waiting_hero(e, d, pal, cx);
        }
        let device = if cfg!(target_os = "macos") {
            "Mac"
        } else {
            "computer"
        };
        let (color, words, action): (gpui::Hsla, Vec<(&str, bool)>, Option<AnyElement>) = match d.look {
            Look::Attention if !d.paired => (
                pal.danger,
                vec![("Pair ", false), ("this ", true), (device, true), (" with ", false), ("your phone", true)],
                Some(
                    btn_icon("hero-pair", Icon::Smartphone, "Pair again", Btn::Primary, pal)
                        .on_click(Self::on_model(cx, Model::sign_out))
                        .into_any_element(),
                ),
            ),
            Look::Attention => (
                pal.danger,
                vec![("The ", false), ("background service", true), (" isn't running", false)],
                Some(
                    btn_icon("hero-start", Icon::Play, "Start", Btn::Primary, pal)
                        .on_click(Self::on_model(cx, Model::start_service))
                        .into_any_element(),
                ),
            ),
            Look::Paused => (
                pal.warning,
                vec![("git goes ", false), ("straight to the hosts", true), (" for now", false)],
                Some(
                    btn_icon("hero-resume", Icon::Play, "Resume", Btn::Primary, pal)
                        .on_click(Self::on_model(cx, Model::resume))
                        .into_any_element(),
                ),
            ),
            Look::On | Look::Waiting => (
                pal.success,
                vec![
                    ("Your phone approves what ", false),
                    ("AI agents", true),
                    (" do on ", false),
                    ("this ", true),
                    (device, true),
                ],
                None,
            ),
        };
        let words: Vec<super::story::Words> = words
            .into_iter()
            .map(|(t, b)| super::story::Words {
                text: t.to_owned(),
                bold: b,
            })
            .collect();
        // What is protected, as chips.
        let mut chips = div().flex().flex_wrap().gap(px(8.0));
        if d.look == Look::On {
            let tools = d.snapshot.as_ref().map_or(0, |s| s.harnesses.iter().filter(|h| h.added).count());
            if tools > 0 {
                chips = chips.child(
                    chip_plain(Some(Icon::Check), format::count(tools as u64, "AI tool", "AI tools"), pal)
                        .h(px(28.0))
                        .font_weight(FontWeight::MEDIUM),
                );
            }
            if d.snapshot.as_ref().is_some_and(|s| !s.git_routed.is_empty()) {
                chips = chips.child(chip_plain(None, "git via Reins", pal).h(px(28.0)).font_weight(FontWeight::MEDIUM));
            }
            if d.config().is_some_and(|c| c.ssh.enabled) {
                chips = chips.child(chip_plain(None, "SSH keys", pal).h(px(28.0)).font_weight(FontWeight::MEDIUM));
            }
        } else if d.look == Look::Paused
            && let Some(left) = d.pause.short(d.now)
        {
            chips = chips.child(chip(Some(Icon::Pause), left, pal.warning));
        }
        let action = action.unwrap_or_else(|| {
            let sending = matches!(d.test, Test::Waiting(_));
            let b = btn_icon("hero-test", Icon::Send, "Send a test", Btn::Plain, pal);
            if d.paired && !sending {
                b.on_click(Self::on_model(cx, Model::send_test)).into_any_element()
            } else {
                b.opacity(0.45).cursor_default().into_any_element()
            }
        });
        hero_frame(color, pal)
            .child(stage(Mark::Icon(Icon::Laptop), Mark::Icon(Icon::Smartphone), None, color, 48.0, pal))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w(px(260.0))
                    .gap(px(10.0))
                    .child(
                        div()
                            .font_family(SERIF)
                            .text_size(px(30.0))
                            .line_height(px(33.0))
                            .font_weight(FontWeight::NORMAL)
                            .text_color(pal.ink())
                            .child(sentence(&words, pal)),
                    )
                    .child(chips),
            )
            .child(action)
            .into_any_element()
    }

    /// A request waiting on the phone: its stage, its sentence, its risk, the timer, "Open on phone".
    fn waiting_hero(e: &Entry, d: &Data, pal: Palette, cx: &mut Context<'_, Self>) -> AnyElement {
        let mut s = Story::of(e);
        if s.target == Mark::Icon(Icon::GitBranch)
            && let Some(b) = d.git_host_mark()
        {
            s.target = Mark::Brand(b);
        }
        let color = if s.tone == Tone::Neutral {
            pal.accent
        } else {
            pal.tone(s.tone)
        };
        let (total, left) = d.wait_left(e);
        #[allow(clippy::cast_precision_loss, reason = "minutes of seconds")]
        let fraction = left as f32 / total as f32;
        let more = d.waiting().count().saturating_sub(1);
        let timer = div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(7.0))
            .h(px(30.0))
            .pl(px(8.0))
            .pr(px(11.0))
            .rounded_full()
            .bg(pal.control_fill)
            .text_size(px(13.0))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(if left < 20 {
                pal.danger
            } else {
                pal.secondary
            })
            .child(ring(fraction, pal.secondary, pal.tile_border(), 18.0, 2.4))
            .child(clock_left(left));
        let mut chips = div()
            .flex()
            .flex_wrap()
            .gap(px(8.0))
            .when_some(s.risk, |c, risk| c.child(chip(Some(Icon::TriangleAlert), risk, color)))
            .child(chip_plain(Some(Icon::Smartphone), "Answer on your phone", pal));
        if more > 0 {
            chips = chips.child(chip_plain(None, format!("+{more} more"), pal));
        }
        hero_frame(color, pal)
            .child(stage(s.agent, s.target, Some((s.badge, color)), color, 48.0, pal))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w(px(240.0))
                    .gap(px(10.0))
                    .child(
                        div()
                            .font_family(SERIF)
                            .text_size(px(30.0))
                            .line_height(px(33.0))
                            .font_weight(FontWeight::NORMAL)
                            .text_color(pal.ink())
                            .child(sentence(&s.sentence, pal)),
                    )
                    .child(chips),
            )
            .child(
                div().flex().flex_col().flex_none().items_end().gap(px(10.0)).child(timer).child(
                    btn_icon(
                        "hero-qr",
                        Icon::QrCode,
                        if d.show_qr {
                            "Hide code"
                        } else {
                            "Open on phone"
                        },
                        Btn::Plain,
                        pal,
                    )
                    .on_click(Self::on_model(cx, Model::toggle_phone_qr)),
                ),
            )
            .into_any_element()
    }

    /// Health: all good, or how many things to look at (yellow) with the first one's fix.
    fn health_tile(&self, d: &Data, pal: Palette, cx: &mut Context<'_, Self>) -> AnyElement {
        let checks = d.health.checks.as_deref();
        let (level, value, line) = match (checks, &d.health.error) {
            (_, Some(e)) => (Level::Fail, "Can't check".to_owned(), e.clone()),
            (Some(c), None) => {
                let s = health::summary(c);
                let first = health::to_look_at(c).first().map(|c| format!("{} · {}", c.label, c.detail));
                let line = first.unwrap_or_else(|| {
                    d.health.at.map_or_else(String::new, |at| format!("Checked {}", format::ago(d.now, at)))
                });
                let value = match s.level {
                    Level::Ok => "All good".to_owned(),
                    _ => format!("{} to look at", health::to_look_at(c).len()),
                };
                (s.level, value, line)
            }
            (None, None) => (Level::Skip, "Checking…".to_owned(), String::new()),
        };
        let fix = checks.and_then(|c| health::to_look_at(c).first().and_then(|c| health::fix_for(c)));
        let attention = matches!(level, Level::Warn | Level::Fail);
        let color = if attention {
            level_color(level, pal)
        } else {
            pal.success
        };
        let mut tile = stat(Icon::HeartPulse, color, "Health", pal)
            .id("tile-health")
            .min_w(px(190.0))
            .cursor_pointer()
            .hover(|s| s.border_color(color.opacity(0.4)))
            .when(attention, |t| t.border_color(color.opacity(0.45)))
            .child(stat_value().when(attention, |v| v.text_color(color)).child(value));
        let mut under =
            div().flex().items_center().gap(px(8.0)).min_h(px(26.0)).child(
                div().flex_1().min_w(px(0.0)).truncate().text_size(px(12.0)).text_color(pal.tertiary).child(line),
            );
        if let Some(fix) = fix {
            under = under.child(small(btn("tile-fix", fix.label(), Btn::Accent, pal)).on_click(cx.listener(
                move |this, _, _, cx| {
                    cx.stop_propagation();
                    this.model.update(cx, |m, cx| m.fix(fix, cx));
                },
            )));
        }
        tile = tile.child(under).on_click(Self::on_view(cx, |this, _| this.health_open = !this.health_open));
        if self.health_open {
            tile = tile.border_color(color.opacity(0.5));
        }
        tile.into_any_element()
    }

    /// Work session: Start; the request on the phone; or the session running, its ring, reason, time left and End.
    fn work_tile(d: &Data, pal: Palette, cx: &mut Context<'_, Self>) -> AnyElement {
        let tile = stat(Icon::Timer, pal.accent, "Work session", pal).min_w(px(190.0));
        let value = |text: String| stat_value().min_w(px(0.0)).truncate().child(text);
        let under = || div().flex().items_center().gap(px(8.0)).min_h(px(26.0));
        let line = |text: String| {
            div().flex_1().min_w(px(0.0)).truncate().text_size(px(12.0)).text_color(pal.tertiary).child(text)
        };
        if let Some(session) = d.work_session() {
            let total = (session.expires_at - session.started_at).max(1);
            #[allow(clippy::cast_precision_loss, reason = "seconds of a day")]
            let fraction = session.left(d.now) as f32 / total as f32;
            let end: AnyElement = if d.work.ending {
                small(btn("tile-ending", "Ending…", Btn::Plain, pal)).opacity(0.6).into_any_element()
            } else if d.work.confirm_end {
                div()
                    .flex()
                    .gap(px(6.0))
                    .child(
                        small(btn("tile-keep", "Keep", Btn::Plain, pal))
                            .on_click(Self::on_model(cx, Model::keep_work_session)),
                    )
                    .child(
                        small(btn("tile-end", "End", Btn::Danger, pal))
                            .on_click(Self::on_model(cx, Model::end_work_session)),
                    )
                    .into_any_element()
            } else {
                small(btn("tile-end-now", "End", Btn::Danger, pal))
                    .on_click(Self::on_model(cx, Model::confirm_end_work_session))
                    .into_any_element()
            };
            return tile
                .border_color(pal.accent.opacity(0.3))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(10.0))
                        .mt(px(6.0))
                        .child(ring(fraction, pal.accent, pal.tile_border(), 26.0, 3.0))
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .flex_1()
                                .min_w(px(0.0))
                                .child(
                                    div()
                                        .text_size(px(14.0))
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .truncate()
                                        .child(session.reason.clone()),
                                )
                                .child(
                                    div()
                                        .text_size(px(12.0))
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(pal.accent)
                                        .child(format!("{} left", crate::work::left(session, d.now))),
                                ),
                        ),
                )
                .child(under().child(line(format!("Until {}", format::clock(session.expires_at)))).child(end))
                .into_any_element();
        }
        if let Some(asking) = &d.work.asking {
            let left = i64::try_from(asking.left(std::time::Instant::now())).unwrap_or(0);
            return tile
                .border_color(pal.accent.opacity(0.3))
                .child(value("On your phone".to_owned()))
                .child(
                    under().child(line(asking.request.reason.clone())).child(
                        div()
                            .text_size(px(12.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(pal.accent)
                            .child(clock_left(left)),
                    ),
                )
                .into_any_element();
        }
        if d.work.failed.is_some() {
            return tile
                .border_color(pal.danger.opacity(0.3))
                .child(value("Not started".to_owned()))
                .child(under().child(line(d.work.failed.clone().unwrap_or_default())))
                .into_any_element();
        }
        let open = d.work.form.is_some();
        let start = small(btn(
            "session-start",
            if open {
                "Cancel"
            } else {
                "Start"
            },
            Btn::Plain,
            pal,
        ));
        tile.child(value("Not running".to_owned()))
            .child(under().child(line("Approve once, work freely".to_owned())).child(if !d.paired {
                start.opacity(0.45).cursor_default()
            } else if open {
                start.on_click(Self::on_model(cx, Model::close_work_form))
            } else {
                start.on_click(Self::on_model(cx, Model::open_work_form))
            }))
            .into_any_element()
    }

    /// Today: approved, denied, no answer, in colour.
    fn today_tile(d: &Data, pal: Palette) -> AnyElement {
        let t = today(d.activity(), d.now);
        let n = |v: usize, color: gpui::Hsla| {
            div()
                .text_color(if v == 0 {
                    pal.ink_meta()
                } else {
                    color
                })
                .child(v.to_string())
        };
        let line = "approved · denied · no answer";
        stat(Icon::Calendar, pal.blue(), "Today", pal)
            .min_w(px(190.0))
            .child(
                div()
                    .flex()
                    .gap(px(18.0))
                    .mt(px(8.0))
                    .font_family(SERIF)
                    .text_size(px(30.0))
                    .line_height(px(34.0))
                    .child(n(t.approved, pal.success))
                    .child(n(t.denied, pal.danger))
                    .child(n(t.timed_out, pal.warning)),
            )
            .child(div().flex().items_center().min_h(px(26.0)).text_size(px(12.0)).text_color(pal.tertiary).child(line))
            .into_any_element()
    }

    fn update_banner(d: &Data, pal: Palette, cx: &mut Context<'_, Self>) -> Option<AnyElement> {
        let update = d.update.as_ref()?;
        let banner = card(pal)
            .p(px(12.0))
            .pl(px(14.0))
            .flex_row()
            .items_center()
            .gap(px(10.0))
            .bg(pal.accent_soft)
            .border_color(pal.accent.opacity(0.25))
            .child(icon(Icon::Download, 16.0, pal.accent));
        Some(
            if update.ready.is_some() {
                let label = if d.updating {
                    "Restarting…"
                } else {
                    "Restart to update"
                };
                let restart = btn("restart-update", label, Btn::Accent, pal);
                banner.child(div().flex_1().child(format!("Reins {} is ready", update.version))).child(if d.updating {
                    restart.opacity(0.6)
                } else {
                    restart.on_click(Self::on_model(cx, Model::restart_to_update))
                })
            } else {
                banner.child(div().flex_1().child(format!("Reins {} is out", update.version))).child(
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

    #[test]
    fn rows_say_when_in_few_words() {
        let now = 1_800_000_000;
        assert_eq!(short_ago(now, now - 10), "now");
        assert_eq!(short_ago(now, now - 4 * 60), "4 min");
        assert_eq!(short_ago(now, now - 2 * 3_600), "2 h");
        assert_eq!(short_ago(now, now - 3 * 86_400), format::date(now - 3 * 86_400));
    }
}
