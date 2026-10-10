//! Steps 2 and 3 of the welcome flow, and its end: connect the AI tools (one click each, undoable), turn Reins on
//! (the background service, git, opening at login, each step as it goes), and "Reins is on" with a test to the
//! phone.

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, Context, Div, FontWeight, InteractiveElement as _, IntoElement, ParentElement, SharedString,
    StatefulInteractiveElement as _, Styled as _, div, px,
};
use reins_desktop::harness::{Harness, after_add_note};

use super::parts::{
    big, button, caption, card, check, check_circle, fine, help, labelled_help, link, row, step_line, toggle, warning,
    welcome_title,
};
use super::{Data, Root};
use crate::model::{Model, Step, TurnOn};
use crate::theme::Palette;
use crate::welcome::{self, ToolRow};

/// The tile before an AI tool's name: its initials.
fn tool_tile(h: Harness, pal: Palette) -> Div {
    let initials = match h {
        Harness::ClaudeCode => "CC",
        Harness::Codex => "Cx",
        Harness::Gemini => "G",
        Harness::Cursor => "Cu",
    };
    div()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(px(34.0))
        .rounded(px(9.0))
        .bg(pal.control_fill)
        .text_color(pal.secondary)
        .text_size(px(12.0))
        .font_weight(FontWeight::SEMIBOLD)
        .child(initials)
}

/// "✓ Connected" in green.
fn connected_mark(pal: Palette) -> Div {
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap(px(6.0))
        .text_color(pal.success)
        .font_weight(FontWeight::MEDIUM)
        .child(check(14.0, pal.success))
        .child("Connected")
}

/// The row of buttons at the bottom of a step: what goes back on the left, what goes on at the right.
fn actions(back: Option<AnyElement>, forward: impl IntoElement) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(10.0))
        .child(div().flex().flex_1().when_some(back, ParentElement::child))
        .child(forward)
}

impl Root {
    pub(super) fn tools_step(&self, d: &Data, pal: Palette, cx: &mut Context<'_, Self>) -> AnyElement {
        let m = self.model.read(cx);
        let rows: Vec<ToolRow> = m.tools.clone();
        // A copy of Reins that will not be there after a restart must not be written into the tools' settings.
        let location = crate::backend::Backend::location_problem();
        let connectable = if location.is_some() {
            Vec::new()
        } else {
            welcome::connectable(&rows)
        };

        let mut list = card(pal);
        for (i, r) in rows.iter().enumerate() {
            let h = r.harness;
            // A line only when there is something to do or to know: why it failed, or the restart it needs.
            let detail: Option<AnyElement> = if let Some(e) = &r.error {
                Some(caption(e.clone(), pal).text_color(pal.danger).into_any_element())
            } else if r.connected && r.undoable {
                Some(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.0))
                        .child(caption("Restart it to finish", pal))
                        .child(help(h.id(), after_add_note(h), d, pal, cx))
                        .into_any_element(),
                )
            } else {
                None
            };
            let right: AnyElement = if r.busy {
                fine("Connecting…", pal).into_any_element()
            } else if r.connected {
                div()
                    .flex()
                    .items_center()
                    .gap(px(14.0))
                    .child(connected_mark(pal))
                    .child(
                        link(
                            SharedString::from(format!("undo-{}", h.id())),
                            if r.undoable {
                                "Undo"
                            } else {
                                "Remove"
                            },
                            pal,
                        )
                        .text_color(pal.secondary)
                        .on_click(Self::on_model(cx, move |m, cx| m.undo_tool(h, cx))),
                    )
                    .into_any_element()
            } else if r.installed && location.is_some() {
                button(SharedString::from(format!("connect-{}", h.id())), "Connect", pal, true, false)
                    .into_any_element()
            } else if r.installed {
                button(SharedString::from(format!("connect-{}", h.id())), "Connect", pal, true, true)
                    .on_click(Self::on_model(cx, move |m, cx| m.connect_tool(h, cx)))
                    .into_any_element()
            } else {
                fine("Not installed", pal).into_any_element()
            };
            let mut line = row(pal, i == 0)
                .py(px(13.0))
                .child(tool_tile(h, pal))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_w(px(0.0))
                        .gap(px(2.0))
                        .child(div().text_size(px(13.5)).font_weight(FontWeight::MEDIUM).child(h.label()))
                        .when_some(detail, ParentElement::child),
                )
                .child(right);
            if !r.installed && !r.connected {
                line = line.opacity(0.5);
            }
            list = list.child(line);
        }

        let header = div()
            .flex()
            .items_center()
            .gap(px(12.0))
            .px(px(2.0))
            .child(caption(welcome::tools_line(&rows), pal).flex_1().min_w(px(0.0)))
            .when(connectable.len() > 1, |h| {
                h.child(
                    button("connect-all", "Connect all", pal, false, true)
                        .on_click(Self::on_model(cx, Model::connect_all)),
                )
            });
        let any_connected = rows.iter().any(|r| r.connected);
        let forward = big(button(
            "tools-continue",
            if any_connected || connectable.is_empty() {
                "Continue"
            } else {
                "Skip for now"
            },
            pal,
            any_connected || connectable.is_empty(),
            true,
        ))
        .on_click(Self::on_model(cx, Model::tools_continue));

        div()
            .flex()
            .flex_col()
            .gap(px(16.0))
            .child(welcome_title(
                "Connect your AI tools",
                Some(help(
                    "tools",
                    "Reins adds itself to each one: its tools reach your phone, and risky commands wait for your OK. \
                     Undo takes it out again.",
                    d,
                    pal,
                    cx,
                )),
            ))
            .when_some(location, |p, problem| p.child(warning(problem, pal)))
            .child(div().flex().flex_col().gap(px(8.0)).pt(px(6.0)).child(header).child(list))
            .child(div().pt(px(6.0)).child(actions(None, forward)))
            .into_any_element()
    }

    pub(super) fn turn_on_step(&self, d: &Data, pal: Palette, cx: &mut Context<'_, Self>) -> AnyElement {
        let m = self.model.read(cx);
        let steps = m.setup_steps.clone();
        let running = m.setup_running;
        let finished = m.setup_finished();
        let failed = steps.iter().any(|(_, s)| matches!(s, Step::Failed(_)));
        let (git, login) = (m.turn_git, m.turn_autostart);
        let notice = m.notice.clone();
        let location = crate::backend::Backend::location_problem();

        let body = if steps.is_empty() {
            let option = |id: &'static str, title: &'static str, about: &'static str, what: Option<(TurnOn, bool)>| {
                let r = row(pal, id == "turn-service").id(id).py(px(13.0)).child(labelled_help(
                    title,
                    None,
                    Some(help(id, about, d, pal, cx)),
                    pal,
                ));
                match what {
                    Some((what, on)) => r
                        .cursor_pointer()
                        .child(toggle(on, pal))
                        .on_click(Self::on_model(cx, move |m, cx| m.toggle_turn_on(what, cx))),
                    None => r.child(fine("Always", pal)),
                }
            };
            card(pal)
                .child(option(
                    "turn-service",
                    "Background service",
                    "Hooks, git and API keys go through it. It starts again when you log in.",
                    None,
                ))
                .child(option(
                    "turn-git",
                    "Send git through Reins",
                    "Pushes to GitHub wait for your OK on your phone; agents never see your token.",
                    Some((TurnOn::Git, git)),
                ))
                .child(option(
                    "turn-login",
                    "Open at login",
                    "The shield in the menu bar or tray shows that Reins is on.",
                    Some((TurnOn::Autostart, login)),
                ))
        } else {
            let mut progress = card(pal);
            for (i, (label, step)) in steps.iter().enumerate() {
                progress = progress.child(step_line(label, step, pal, i == 0).py(px(13.0)));
            }
            progress
        };

        let back = (!running && (!finished || failed)).then(|| {
            button("turn-back", "Back", pal, false, true)
                .on_click(Self::on_model(cx, Model::setup_back))
                .into_any_element()
        });
        let forward: AnyElement = if finished && failed {
            div()
                .flex()
                .gap(px(8.0))
                .child(
                    button("skip", "Continue anyway", pal, false, true)
                        .on_click(Self::on_model(cx, Model::finish_setup)),
                )
                .child(
                    big(button("again", "Try again", pal, true, true)).on_click(Self::on_model(cx, Model::run_setup)),
                )
                .into_any_element()
        } else {
            let label = if running {
                "Turning on…"
            } else if finished {
                "Reins is on"
            } else {
                "Turn on Reins"
            };
            let enabled = !running && !finished && location.is_none();
            let b = big(button("run-setup", label, pal, true, enabled));
            if enabled {
                b.on_click(Self::on_model(cx, Model::run_setup)).into_any_element()
            } else {
                b.into_any_element()
            }
        };

        let mut page = div().flex().flex_col().gap(px(16.0)).child(welcome_title(
            "Turn on Reins",
            Some(help(
                "turn-on",
                "Reins runs quietly in the background and asks your phone whenever an agent wants something risky.",
                d,
                pal,
                cx,
            )),
        ));
        page = page.child(div().pt(px(6.0)).child(body));
        if let Some(problem) = location {
            page = page.child(warning(problem, pal));
        }
        page = page.child(div().pt(px(6.0)).child(actions(back, forward)));
        if let Some(n) = notice {
            page = page.child(caption(n, pal).text_color(pal.danger));
        }
        page.into_any_element()
    }

    pub(super) fn done_step(&self, d: &Data, pal: Palette, cx: &mut Context<'_, Self>) -> AnyElement {
        let m = self.model.read(cx);
        let tools = welcome::connected_names(&m.tools);
        let restart = m.tools.iter().any(|r| r.undoable);
        let git = m.turn_git;
        let login = m.autostart;
        let phone = d.saved.phone.clone();
        let test_ok = matches!(&d.test, crate::health::Test::Ended(a, _) if !matches!(a, reins_desktop::ask::Answer::Unanswered(_)));

        let mut summary = card(pal);
        let mut lines: Vec<(bool, String)> =
            vec![(true, phone.map_or_else(|| "Paired with your phone".to_owned(), |p| format!("Paired with {p}")))];
        lines.push(match tools {
            Some(names) => (true, format!("{names} connected")),
            None => (false, "No AI tool connected".to_owned()),
        });
        if git {
            lines.push((true, "git through Reins".to_owned()));
        }
        if login {
            lines.push((true, "Opens at login".to_owned()));
        }
        for (i, (ok, text)) in lines.into_iter().enumerate() {
            summary = summary.child(
                row(pal, i == 0)
                    .py(px(10.0))
                    .child(div().flex().flex_none().items_center().w(px(16.0)).child(if ok {
                        check(15.0, pal.success).into_any_element()
                    } else {
                        div().text_color(pal.tertiary).child("–").into_any_element()
                    }))
                    .child(div().flex_1().min_w(px(0.0)).child(text)),
            );
        }

        let hero = div().flex().flex_col().items_center().gap(px(12.0)).child(check_circle(68.0, pal.success)).child(
            welcome_title(
                "Reins is on",
                Some(help("done", "Your phone now approves what your AI agents do on this computer.", d, pal, cx)),
            ),
        );

        div()
            .flex()
            .flex_col()
            .gap(px(18.0))
            .child(hero)
            .child(summary)
            .when(restart, |p| p.child(fine("Restart the AI tools you connected to finish.", pal).px(px(2.0))))
            .child(card(pal).child(Self::test_panel(d, false, pal, cx)))
            .child(
                div().flex().justify_center().child(
                    big(button("open-reins", "Open Reins", pal, test_ok, true))
                        .on_click(Self::on_model(cx, Model::finish_setup)),
                ),
            )
            .into_any_element()
    }
}
