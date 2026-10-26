//! Steps 2 and 3 of the welcome flow, and its end (`Desk-Setup.png`): connect the AI tools (their real logos, one click
//! each, undoable), turn Reins on (the background service, git, opening at login, each step as it goes), and "Reins is
//! *on*" over the pond, with a test to the phone. One serif sentence each, plain rows, what goes on bottom right.

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, Context, Div, InteractiveElement as _, IntoElement, ParentElement, SharedString, Stateful,
    StatefulInteractiveElement as _, Styled as _, div, px,
};
use reins_desktop::harness::after_add_note;

use super::connections::harness_mark;
use super::icons::{Glyph, glyph};
use super::parts::{caption, help, quiet_link, waiting_dot, warning, welcome_title};
use super::story::{bold, plain};
use super::{Data, Root, kit};
use crate::model::{Model, Step, TurnOn};
use crate::theme::{NextText as _, Palette};
use crate::welcome::{self, ToolRow};

/// A step's own button (`.btn` on the board: 54 px, 16.5).
fn step_button(
    id: impl Into<gpui::ElementId>,
    look: kit::Look,
    label: impl Into<SharedString>,
    pal: Palette,
) -> Stateful<Div> {
    kit::button(id, look, kit::Size::Large, None, label, pal)
}

/// The row of buttons at the bottom of a step: what goes back on the left, what goes on at the right.
fn actions(back: Option<AnyElement>, forward: impl IntoElement) -> Div {
    div()
        .flex()
        .w_full()
        .items_center()
        .gap(px(10.0))
        .pt(px(8.0))
        .child(div().flex().flex_1().when_some(back, ParentElement::child))
        .child(forward)
}

/// The mark at the start of a row of "Turn on" and "Done": a check, a breathing dot while it runs, a cross.
fn state_mark(step: Option<&Step>, id: &str, pal: Palette) -> AnyElement {
    let slot = div().flex().flex_none().items_center().justify_center().size(px(18.0));
    match step {
        Some(Step::Running) => {
            slot.child(waiting_dot(SharedString::from(format!("step-{id}")), pal.violet(), 8.0)).into_any_element()
        }
        Some(Step::Failed(_)) => slot.child(glyph(Glyph::Close, 16.0, pal.danger)).into_any_element(),
        Some(Step::Done(_)) | None => slot.child(glyph(Glyph::Check, 16.0, pal.success)).into_any_element(),
    }
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

        let mut items: Vec<AnyElement> = Vec::new();
        for r in &rows {
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
            } else {
                None
            };
            let right: AnyElement = if r.busy {
                div().text_size(px(12.5)).text_color(pal.ink_meta()).child("Connecting…").into_any_element()
            } else if r.connected {
                div()
                    .flex()
                    .items_center()
                    .gap(px(14.0))
                    .child(
                        quiet_link(
                            SharedString::from(format!("undo-{}", h.id())),
                            if r.undoable {
                                "Undo"
                            } else {
                                "Remove"
                            },
                            13.0,
                            pal,
                        )
                        .on_click(Self::on_model(cx, move |m, cx| m.undo_tool(h, cx))),
                    )
                    .child(kit::status_pill(Some(Glyph::Check), "Connected", Some(pal.success), pal))
                    .into_any_element()
            } else if r.installed {
                let b = kit::button(
                    SharedString::from(format!("connect-{}", h.id())),
                    kit::Look::Inverse,
                    kit::Size::Regular,
                    None,
                    "Connect",
                    pal,
                );
                if location.is_some() {
                    kit::disabled(b).into_any_element()
                } else {
                    b.on_click(Self::on_model(cx, move |m, cx| m.connect_tool(h, cx))).into_any_element()
                }
            } else if !r.installed {
                kit::pane::row_value("Not installed", pal).into_any_element()
            } else {
                div().into_any_element()
            };
            let line = kit::pane_row()
                .min_h(px(58.0))
                .child(kit::pane::avatar(harness_mark(h), 32.0, pal))
                .child(kit::title_sub(h.label(), detail, pal))
                .child(right)
                .when(!r.installed && !r.connected, |l| l.opacity(0.5));
            items.push(line.into_any_element());
        }
        let list = kit::group(items, kit::pane::INSET, kit::Glass::Solid, pal);

        let any_connected = rows.iter().any(|r| r.connected);
        let forward = div()
            .flex()
            .items_center()
            .gap(px(10.0))
            .when(connectable.len() > 1, |f| {
                f.child(
                    step_button("connect-all", kit::Look::Soft, "Connect all", pal)
                        .on_click(Self::on_model(cx, Model::connect_all)),
                )
            })
            .child(
                step_button(
                    "tools-continue",
                    if any_connected || connectable.is_empty() {
                        kit::Look::Inverse
                    } else {
                        kit::Look::Soft
                    },
                    if any_connected || connectable.is_empty() {
                        "Continue"
                    } else {
                        "Skip for now"
                    },
                    pal,
                )
                .on_click(Self::on_model(cx, Model::tools_continue)),
            );

        div()
            .flex()
            .flex_col()
            .items_center()
            .gap(px(20.0))
            .child(welcome_title(
                &[plain("Connect your "), bold("AI tools")],
                Some(help(
                    "tools",
                    "Reins adds itself to each one: its tools reach your phone, and risky commands wait for your OK.",
                    d,
                    pal,
                    cx,
                )),
                pal,
            ))
            .child(div().mt(px(-12.0)).text_size(px(15.0)).text_color(pal.ink_2()).text_center().child(format!(
                "{}. Reins adds itself to each; Undo takes it out again.",
                welcome::tools_line(&rows, super::parts::this_computer())
            )))
            .when_some(location, |p, problem| p.child(warning(problem, pal)))
            .child(list)
            .child(actions(None, forward))
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
            let shield = if cfg!(target_os = "macos") {
                "Adds the shield to the menu bar"
            } else {
                "Adds the shield to the tray"
            };
            let option = |id: &'static str,
                          g: Glyph,
                          title: &'static str,
                          line: &'static str,
                          about: &'static str,
                          what: Option<(TurnOn, bool)>,
                          last: bool| {
                let _ = last;
                let r = kit::pane_row()
                    .id(id)
                    .child(kit::pane::tile(g, pal))
                    .child(kit::title_sub(title, Some(line.into_any_element()), pal))
                    .child(help(id, about, d, pal, cx));
                match what {
                    Some((what, on)) => r
                        .cursor_pointer()
                        .child(kit::switch(SharedString::from(format!("{id}-switch")), on, pal))
                        .on_click(Self::on_model(cx, move |m, cx| m.toggle_turn_on(what, cx)))
                        .into_any_element(),
                    None => r.child(glyph(Glyph::Check, 16.0, pal.success)).into_any_element(),
                }
            };
            kit::group(
                [
                    option(
                        "turn-service",
                        Glyph::Server,
                        "Start the background service",
                        "Runs at login",
                        "Hooks, git and API keys go through it. It starts again when you log in.",
                        None,
                        false,
                    ),
                    option(
                        "turn-git",
                        Glyph::GitBranch,
                        "Send git through Reins",
                        "Pushes ask your phone",
                        "Pushes to GitHub wait for your OK on your phone; agents never see your token.",
                        Some((TurnOn::Git, git)),
                        false,
                    ),
                    option(
                        "turn-login",
                        Glyph::Power,
                        "Open at login",
                        shield,
                        "The shield in the menu bar or tray shows that Reins is on.",
                        Some((TurnOn::Autostart, login)),
                        true,
                    ),
                ],
                kit::pane::INSET,
                kit::Glass::Solid,
                pal,
            )
        } else {
            let mut progress: Vec<AnyElement> = Vec::new();
            for (i, (label, step)) in steps.iter().enumerate() {
                let line = match step {
                    Step::Running => None,
                    Step::Done(t) | Step::Failed(t) => (!t.is_empty()).then(|| t.clone()),
                };
                let failed_line = matches!(step, Step::Failed(_));
                let _ = i;
                progress.push(
                    kit::pane_row()
                        .child(
                            div().flex().flex_none().items_center().justify_center().size(px(34.0)).child(state_mark(
                                Some(step),
                                label,
                                pal,
                            )),
                        )
                        .child(kit::title_sub(
                            label.clone(),
                            line.map(|l| {
                                div().when(failed_line, |t| t.text_color(pal.danger)).child(l).into_any_element()
                            }),
                            pal,
                        ))
                        .into_any_element(),
                );
            }
            kit::group(progress, kit::pane::INSET, kit::Glass::Solid, pal)
        };

        let back = (!running && (!finished || failed)).then(|| {
            step_button("turn-back", kit::Look::Soft, "Back", pal)
                .on_click(Self::on_model(cx, Model::setup_back))
                .into_any_element()
        });
        let forward: AnyElement = if finished && failed {
            div()
                .flex()
                .gap(px(10.0))
                .child(
                    step_button("skip", kit::Look::Soft, "Continue anyway", pal)
                        .on_click(Self::on_model(cx, Model::finish_setup)),
                )
                .child(
                    step_button("again", kit::Look::Inverse, "Try again", pal)
                        .on_click(Self::on_model(cx, Model::run_setup)),
                )
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
            let b = step_button("run-setup", kit::Look::Inverse, label, pal);
            if enabled {
                b.on_click(Self::on_model(cx, Model::run_setup)).into_any_element()
            } else {
                b.opacity(kit::button::PRESSED).cursor_default().into_any_element()
            }
        };

        let mut page = div().flex().flex_col().items_center().gap(px(20.0)).child(welcome_title(
            &[plain("Turn "), bold("Reins"), plain(" on")],
            Some(help(
                "turn-on",
                "Reins runs quietly in the background and asks your phone whenever an agent wants something risky.",
                d,
                pal,
                cx,
            )),
            pal,
        ));
        page = page.child(body);
        if let Some(problem) = location {
            page = page.child(warning(problem, pal));
        }
        page = page.child(actions(back, forward));
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

        // What is on, as plain rows with a check; what still needs a step, in amber on the right.
        let mut lines: Vec<(Glyph, bool, String, Option<&str>)> = vec![(
            Glyph::Phone,
            true,
            phone.map_or_else(|| "Paired with your phone".to_owned(), |p| format!("Paired with {p}")),
            None,
        )];
        lines.push(match tools {
            Some(names) => (Glyph::Link, true, format!("{names} connected"), restart.then_some("restart it to finish")),
            None => (Glyph::Link, false, "No AI tool connected".to_owned(), None),
        });
        match (git, login) {
            (true, true) => lines.push((Glyph::GitBranch, true, "git through Reins · opens at login".to_owned(), None)),
            (true, false) => lines.push((Glyph::GitBranch, true, "git through Reins".to_owned(), None)),
            (false, true) => lines.push((Glyph::Power, true, "Opens at login".to_owned(), None)),
            (false, false) => {}
        }
        let summary = kit::group(
            lines.into_iter().map(|(g, ok, text, note)| {
                kit::pane_row()
                    .child(kit::pane::tile(g, pal))
                    .child(kit::title_sub(
                        text,
                        note.map(|n| div().text_color(pal.warning).child(n).into_any_element()),
                        pal,
                    ))
                    .child(glyph(
                        if ok {
                            Glyph::Check
                        } else {
                            Glyph::Minus
                        },
                        16.0,
                        if ok {
                            pal.success
                        } else {
                            pal.ink_faint()
                        },
                    ))
                    .into_any_element()
            }),
            kit::pane::INSET,
            kit::Glass::Solid,
            pal,
        );

        div()
            .flex()
            .flex_col()
            .items_center()
            .gap(px(14.0))
            .child(div().ask().text_color(pal.ink()).text_center().child("Reins is on"))
            .child(div().w_full().mt(px(6.0)).child(summary))
            .child(div().w_full().child(kit::pane(
                Self::test_panel(d, true, pal, cx),
                kit::Glass::Solid,
                kit::pane::RADIUS,
                pal,
            )))
            .child(actions(
                None,
                step_button("open-reins", kit::Look::Inverse, "Open Reins", pal)
                    .on_click(Self::on_model(cx, Model::finish_setup)),
            ))
            .into_any_element()
    }
}
