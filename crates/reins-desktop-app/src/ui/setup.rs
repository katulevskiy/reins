//! Steps 2 and 3 of the welcome flow, and its end: connect the AI tools (their real logos, one click each, undoable),
//! turn Reins on (the background service, git, opening at login, each step as it goes), and "Reins is on" with a
//! test to the phone. One sentence each, the important words bold; what goes on, bottom right.

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, Context, Div, FontWeight, InteractiveElement as _, IntoElement, ParentElement, SharedString,
    StatefulInteractiveElement as _, Styled as _, div, px,
};
use reins_desktop::harness::after_add_note;

use super::connections::harness_mark;
use super::icons::{Icon, Mark, icon};
use super::parts::{
    Btn, big, btn, caption, card, chip, help, link, logo, row, row_text, solid_tile, stage, step_line, toggle, warning,
    welcome_title,
};
use super::story::Words;
use super::{Data, Root};
use crate::model::{Model, Step, TurnOn};
use crate::theme::{Palette, Tone};
use crate::welcome::{self, ToolRow};

/// Words of a sentence: `(text, bold)`.
fn words(parts: &[(&str, bool)]) -> Vec<Words> {
    parts
        .iter()
        .map(|&(t, b)| Words {
            text: t.to_owned(),
            bold: b,
        })
        .collect()
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
                Some(div().text_color(pal.danger).child(e.clone()).into_any_element())
            } else if r.connected && r.undoable {
                Some(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(2.0))
                        .text_color(pal.warning)
                        .child(format!("Restart {} to finish", h.label()))
                        .child(help(h.id(), after_add_note(h), d, pal, cx))
                        .into_any_element(),
                )
            } else if !r.installed && !r.connected {
                Some("Not installed".into_any_element())
            } else {
                None
            };
            let right: AnyElement = if r.busy {
                div().text_size(px(12.0)).text_color(pal.tertiary).child("Connecting…").into_any_element()
            } else if r.connected {
                div()
                    .flex()
                    .items_center()
                    .gap(px(12.0))
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
                        .text_size(px(12.0))
                        .text_color(pal.tertiary)
                        .on_click(Self::on_model(cx, move |m, cx| m.undo_tool(h, cx))),
                    )
                    .child(chip(Some(Icon::Check), "Connected", pal.success))
                    .into_any_element()
            } else if r.installed {
                let b = btn(SharedString::from(format!("connect-{}", h.id())), "Connect", Btn::Accent, pal);
                if location.is_some() {
                    b.opacity(0.45).cursor_default().into_any_element()
                } else {
                    b.on_click(Self::on_model(cx, move |m, cx| m.connect_tool(h, cx))).into_any_element()
                }
            } else {
                div().into_any_element()
            };
            let mut line = row(pal, i == 0)
                .min_h(px(54.0))
                .child(logo(harness_mark(h), pal))
                .child(row_text(h.label(), detail, pal))
                .child(right);
            if !r.installed && !r.connected {
                line = line.opacity(0.5);
            }
            list = list.child(line);
        }

        let any_connected = rows.iter().any(|r| r.connected);
        let forward = div()
            .flex()
            .items_center()
            .gap(px(10.0))
            .when(connectable.len() > 1, |f| {
                f.child(
                    btn("connect-all", "Connect all", Btn::Plain, pal).on_click(Self::on_model(cx, Model::connect_all)),
                )
            })
            .child(
                big(btn(
                    "tools-continue",
                    if any_connected || connectable.is_empty() {
                        "Continue"
                    } else {
                        "Skip for now"
                    },
                    if any_connected || connectable.is_empty() {
                        Btn::Primary
                    } else {
                        Btn::Plain
                    },
                    pal,
                ))
                .on_click(Self::on_model(cx, Model::tools_continue)),
            );

        div()
            .flex()
            .flex_col()
            .gap(px(16.0))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(px(6.0))
                    .child(welcome_title(
                        &words(&[("Connect your ", false), ("AI tools", true)]),
                        Some(help(
                            "tools",
                            "Reins adds itself to each one: its tools reach your phone, and risky commands wait for \
                             your OK. Undo takes it out again.",
                            d,
                            pal,
                            cx,
                        )),
                        pal,
                    ))
                    .child(
                        div()
                            .text_size(px(13.0))
                            .text_color(pal.tertiary)
                            .child(welcome::tools_line(&rows, super::parts::this_computer())),
                    ),
            )
            .when_some(location, |p, problem| p.child(warning(problem, pal)))
            .child(div().pt(px(4.0)).child(list))
            .child(div().pt(px(2.0)).child(actions(None, forward)))
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
            let option = |id: &'static str,
                          ic: Icon,
                          tone: Tone,
                          title: &'static str,
                          about: &'static str,
                          what: Option<(TurnOn, bool)>| {
                let r = row(pal, id == "turn-service")
                    .id(id)
                    .min_h(px(54.0))
                    .child(solid_tile(ic, tone, pal, 30.0))
                    .child(row_text(title, None, pal))
                    .child(help(id, about, d, pal, cx));
                match what {
                    Some((what, on)) => r
                        .cursor_pointer()
                        .child(toggle(on, pal))
                        .on_click(Self::on_model(cx, move |m, cx| m.toggle_turn_on(what, cx))),
                    None => r.child(div().text_size(px(12.0)).text_color(pal.tertiary).child("Always")),
                }
            };
            card(pal)
                .child(option(
                    "turn-service",
                    Icon::Server,
                    Tone::Violet,
                    "Background service",
                    "Hooks, git and API keys go through it. It starts again when you log in.",
                    None,
                ))
                .child(option(
                    "turn-git",
                    Icon::GitBranch,
                    Tone::Teal,
                    "Send git through Reins",
                    "Pushes to GitHub wait for your OK on your phone; agents never see your token.",
                    Some((TurnOn::Git, git)),
                ))
                .child(option(
                    "turn-login",
                    Icon::Power,
                    Tone::Pink,
                    "Open at login",
                    "The shield in the menu bar or tray shows that Reins is on.",
                    Some((TurnOn::Autostart, login)),
                ))
        } else {
            let mut progress = card(pal);
            for (i, (label, step)) in steps.iter().enumerate() {
                progress = progress.child(step_line(label, step, pal, i == 0));
            }
            progress
        };

        let back = (!running && (!finished || failed)).then(|| {
            btn("turn-back", "Back", Btn::Plain, pal).on_click(Self::on_model(cx, Model::setup_back)).into_any_element()
        });
        let forward: AnyElement = if finished && failed {
            div()
                .flex()
                .gap(px(8.0))
                .child(
                    btn("skip", "Continue anyway", Btn::Plain, pal).on_click(Self::on_model(cx, Model::finish_setup)),
                )
                .child(big(btn("again", "Try again", Btn::Primary, pal)).on_click(Self::on_model(cx, Model::run_setup)))
                .into_any_element()
        } else {
            let label = if running {
                "Turning on…"
            } else if finished {
                "Reins is on"
            } else {
                "Turn on"
            };
            let enabled = !running && !finished && location.is_none();
            let b = big(btn("run-setup", label, Btn::Primary, pal));
            if enabled {
                b.on_click(Self::on_model(cx, Model::run_setup)).into_any_element()
            } else {
                b.opacity(0.55).cursor_default().into_any_element()
            }
        };

        let mut page = div()
            .flex()
            .flex_col()
            .gap(px(16.0))
            .child(div().flex().justify_center().child(stage(
                Mark::Icon(Icon::Laptop),
                Mark::Icon(Icon::ShieldCheck),
                None,
                pal.accent,
                56.0,
                pal,
            )))
            .child(welcome_title(
                &words(&[("Turn ", false), ("Reins", true), (" on", false)]),
                Some(help(
                    "turn-on",
                    "Reins runs quietly in the background and asks your phone whenever an agent wants something risky.",
                    d,
                    pal,
                    cx,
                )),
                pal,
            ));
        page = page.child(div().pt(px(4.0)).child(body));
        if let Some(problem) = location {
            page = page.child(warning(problem, pal));
        }
        page = page.child(div().pt(px(2.0)).child(actions(back, forward)));
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
        let mut lines: Vec<(Icon, bool, String)> = vec![(
            Icon::Smartphone,
            true,
            phone.map_or_else(|| "Paired with your phone".to_owned(), |p| format!("Paired with {p}")),
        )];
        lines.push(match tools {
            Some(names) => (Icon::Cable, true, format!("{names} connected")),
            None => (Icon::Cable, false, "No AI tool connected".to_owned()),
        });
        if git {
            lines.push((Icon::GitBranch, true, "git through Reins".to_owned()));
        }
        if login {
            lines.push((Icon::Power, true, "Opens at login".to_owned()));
        }
        for (i, (ic, ok, text)) in lines.into_iter().enumerate() {
            summary = summary.child(
                row(pal, i == 0)
                    .child(super::parts::tile(
                        ic,
                        if ok {
                            pal.success
                        } else {
                            pal.tertiary
                        },
                        30.0,
                    ))
                    .child(div().flex_1().min_w(px(0.0)).font_weight(FontWeight::MEDIUM).child(text))
                    .child(if ok {
                        icon(Icon::CircleCheck, 16.0, pal.success).into_any_element()
                    } else {
                        div().into_any_element()
                    }),
            );
        }

        let hero = div()
            .flex()
            .flex_col()
            .items_center()
            .gap(px(16.0))
            .child(stage(Mark::Icon(Icon::Laptop), Mark::Icon(Icon::Smartphone), None, pal.success, 56.0, pal))
            .child(welcome_title(
                &words(&[("Reins is ", false), ("on", true)]),
                Some(help("done", "Your phone now approves what your AI agents do on this computer.", d, pal, cx)),
                pal,
            ));

        div()
            .flex()
            .flex_col()
            .gap(px(14.0))
            .child(hero)
            .child(div().pt(px(4.0)).child(summary))
            .when(restart, |p| {
                p.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.0))
                        .px(px(4.0))
                        .text_size(px(12.5))
                        .text_color(pal.warning)
                        .child(icon(Icon::TriangleAlert, 14.0, pal.warning))
                        .child("Restart the AI tools you connected to finish"),
                )
            })
            .child(card(pal).child(Self::test_panel(d, true, pal, cx)))
            .child(actions(
                None,
                big(btn(
                    "open-reins",
                    "Open Reins",
                    if test_ok {
                        Btn::Primary
                    } else {
                        Btn::Plain
                    },
                    pal,
                ))
                .on_click(Self::on_model(cx, Model::finish_setup)),
            ))
            .into_any_element()
    }
}
