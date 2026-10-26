//! Connections (`DeskNext-Connections.png`), in two columns of panes: the AI tools Reins is in and the APIs called
//! with keys from the phone on the left; the git hosts sent through it (with the repositories reached), the MCP tool
//! calls and SSH sign-ins on the right. Each with its real logo on a round plate and a line under it. An AI tool opens
//! its own page: what it asked lately, and Disconnect, held.

use std::collections::BTreeMap;

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, Context, Div, IntoElement, ParentElement as _, SharedString, StatefulInteractiveElement as _,
    Styled as _, div, px,
};
use reins_desktop::harness::Harness;
use reins_desktop::journal::{Entry, Kind};
use reins_desktop::stats::Connection;

use super::hold::Held;
use super::home::short_ago;
use super::icons::{Brand, Glyph, Mark};
use super::parts::{caption, page_title, this_computer};
use super::story::{agent_mark, plain};
use super::{Data, Root, kit};
use crate::backend::Setting;
use crate::format;
use crate::model::Section;
use crate::theme::{MONO, NextText as _, Palette};

/// MCP tool calls of one AI tool.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ToolCalls {
    pub calls: u64,
    pub last_at: i64,
    /// The tools called, most recent first, each once.
    pub tools: Vec<String>,
}

/// The MCP tool calls in the activity log, by the AI tool that made them.
#[must_use]
pub fn mcp_by_source(entries: &[Entry]) -> BTreeMap<String, ToolCalls> {
    let mut by: BTreeMap<String, ToolCalls> = BTreeMap::new();
    // Newest first: the first seen is the latest.
    for e in entries.iter().filter(|e| e.kind == Kind::Mcp) {
        let source = e.source.clone().unwrap_or_else(|| "An AI tool".to_owned());
        let t = by.entry(source).or_default();
        t.calls += 1;
        t.last_at = t.last_at.max(e.at);
        if let Some(tool) = &e.service
            && !t.tools.contains(tool)
        {
            t.tools.push(tool.clone());
        }
    }
    by
}

/// The repositories of `host` git reached (`github.com/acme/web` → `acme/web`).
#[must_use]
pub fn repos_of<'a>(connections: &'a [Connection], host: &str) -> Vec<(&'a str, &'a Connection)> {
    connections
        .iter()
        .filter(|c| c.kind == "git")
        .filter_map(|c| {
            let rest = c.target.strip_prefix(host)?.strip_prefix('/')?;
            Some((rest, c))
        })
        .collect()
}

/// An AI tool's logo (Codex has no mark in Simple Icons: a terminal).
#[must_use]
pub fn harness_mark(h: Harness) -> Mark {
    match h {
        Harness::ClaudeCode => Mark::Brand(Brand::Claude),
        Harness::Gemini => Mark::Brand(Brand::Gemini),
        Harness::Cursor => Mark::Brand(Brand::Cursor),
        Harness::Codex => Mark::Glyph(Glyph::Terminal),
    }
}

/// An API's host, from its base URL (`https://api.openai.com/v1` → `api.openai.com/v1`).
fn api_host(base: &str) -> String {
    base.split("://").nth(1).unwrap_or(base).trim_end_matches('/').to_owned()
}

/// "14 requests · 2 min ago".
fn usage(count: u64, last_at: i64, now: i64, one: &str, many: &str) -> String {
    format!("{} · {}", format::count(count, one, many), format::ago(now, last_at))
}

impl Root {
    /// Connections (`DeskNext-Connections.png`): two columns of panes, each section with how many and its +.
    pub(super) fn connections(&self, d: &Data, pal: Palette, cx: &mut Context<'_, Self>) -> AnyElement {
        if let Some(h) = self.tool_page {
            return self.tool_page(h, d, pal, cx);
        }
        let glass = kit::Glass::Solid;
        let empty_conns = Vec::new();
        let conns = d.overview().map_or(&empty_conns, |o| &o.connections);
        let mut page = div().flex().flex_col().child(page_title(
            &[plain("Connections")],
            Some(&format!("What reaches out through Reins on {}.", this_computer())),
            None,
            pal,
        ));
        page = page.when_some(d.notice.clone(), |p, n| p.child(caption(n, pal).mt(px(12.0)).text_color(pal.danger)));
        page = page.when_some(d.note.clone(), |p, n| p.child(caption(n, pal).mt(px(12.0))));
        let mut left = div().flex().flex_col();
        let mut right = div().flex().flex_col();
        let quiet = |text: &str| {
            kit::group(
                [div()
                    .px(px(14.0))
                    .py(px(16.0))
                    .text_size(px(14.5))
                    .text_color(pal.ink_meta())
                    .child(text.to_owned())
                    .into_any_element()],
                14.0,
                glass,
                pal,
            )
        };

        // AI tools.
        if let Some(s) = &d.snapshot {
            let mut rows = s.harnesses.clone();
            rows.sort_by_key(|r| !(r.found || r.added));
            let mut items = Vec::new();
            for r in &rows {
                let h: Harness = r.harness;
                let used = d.activity().iter().find(|e| e.source.as_deref() == Some(h.label())).map(|e| e.at);
                let (line, warn) = match (r.found, r.added, r.complete) {
                    (_, true, true) => ("Hook and MCP".to_owned(), false),
                    (_, true, false) => ("Partly connected: repair it".to_owned(), true),
                    (true, false, _) => ("Installed, not connected".to_owned(), false),
                    (false, false, _) => ("Not installed".to_owned(), false),
                };
                let line = match used.filter(|_| r.added && r.complete) {
                    Some(at) => format!("{line} · used {}", format::ago(d.now, at)),
                    None => line,
                };
                let sub = div().when(warn, |m| m.text_color(pal.warning)).child(line).into_any_element();
                let mut item = kit::pane_row()
                    .child(kit::pane::avatar(harness_mark(h), 32.0, pal))
                    .child(kit::title_sub(h.label(), Some(sub), pal));
                if r.added && !r.complete {
                    item = item.child(
                        kit::button(
                            SharedString::from(format!("repair-{}", h.id())),
                            kit::Look::Soft,
                            kit::Size::Small,
                            None,
                            "Repair",
                            pal,
                        )
                        .on_click(Self::on_model(cx, move |m, cx| m.set_harness(h, true, cx))),
                    );
                }
                let item: AnyElement = if r.found || r.added {
                    let on = r.added;
                    let item =
                        item.child(kit::switch(SharedString::from(format!("harness-{}", h.id())), on, pal).on_click(
                            cx.listener(move |this, _, _, cx| {
                                cx.stop_propagation();
                                this.model.update(cx, |m, cx| m.set_harness(h, !on, cx));
                            }),
                        ));
                    if on {
                        kit::pane::clickable(item, SharedString::from(format!("tool-{}", h.id())), pal)
                            .on_click(Self::on_view(cx, move |this, _| this.tool_page = Some(h)))
                            .into_any_element()
                    } else {
                        item.into_any_element()
                    }
                } else {
                    item.opacity(0.6).into_any_element()
                };
                items.push(item);
            }
            let added = rows.iter().filter(|r| r.added).count();
            left = left
                .child(kit::section(
                    Some(Glyph::Globe),
                    "AI tools",
                    Some(kit::section_count(Some(added), None, pal)),
                    pal,
                ))
                .child(kit::group(items, kit::pane::INSET, glass, pal));
        }

        // APIs.
        let apis: Vec<&Connection> = conns.iter().filter(|c| c.kind == "api").collect();
        let configured = d.config().map_or(0, |c| c.api.len());
        let to_keys = kit::pane::plus("add-api", pal).on_click(Self::go(cx, Section::Keys));
        left = left.child(kit::section(
            Some(Glyph::Key),
            "APIs · keys from your phone",
            Some(kit::section_count(Some(configured.max(apis.len())), Some(to_keys), pal)),
            pal,
        ));
        left = left.child(if apis.is_empty() {
            quiet(if configured == 0 {
                "No APIs yet"
            } else {
                "No calls yet"
            })
        } else {
            let mut items = Vec::new();
            for c in &apis {
                let base =
                    d.config().and_then(|cfg| cfg.api.iter().find(|a| a.name == c.target)).map(|a| a.base.clone());
                let name = base.as_deref().map_or_else(|| c.target.clone(), api_host);
                items.push(
                    kit::pane::clickable(kit::pane_row(), SharedString::from(format!("api-{}", c.target)), pal)
                        .child(kit::pane::tile(Glyph::Globe, pal))
                        .child(kit::title_sub(name, None, pal))
                        .child(kit::pane::row_value(format::count(c.count, "call", "calls"), pal))
                        .child(kit::pane::chevron(pal))
                        .on_click(Self::go(cx, Section::Keys))
                        .into_any_element(),
                );
            }
            kit::group(items, kit::pane::INSET, glass, pal)
        });

        // git hosts.
        if let Some(config) = d.config() {
            let routed = d.snapshot.as_ref().map(|s| s.git_routed.clone()).unwrap_or_default();
            let all = config.git_hosts().unwrap_or_default();
            let mut items = Vec::new();
            for h in &all {
                let repos = repos_of(conns, &h.host);
                let total: u64 = repos.iter().map(|(_, c)| c.count).sum();
                let latest = repos.iter().map(|(_, c)| c.last_at).max();
                let mut about = Vec::new();
                match latest {
                    Some(at) => about.push(usage(total, at, d.now, "request", "requests")),
                    None if h.enabled => about.push("Nothing yet".to_owned()),
                    None => about.push("Direct, not through Reins".to_owned()),
                }
                let not_set = h.enabled && !d.pause.on() && !routed.is_empty() && !routed.contains(&h.host);
                if not_set {
                    about.push("not set up in git yet".to_owned());
                }
                let (host, on) = (h.host.clone(), h.enabled);
                let mark = Brand::of_host(&h.host).map_or(Mark::Glyph(Glyph::GitBranch), Mark::Brand);
                let mut block =
                    div().flex().flex_col().child(
                        kit::pane_row()
                            .child(kit::pane::avatar(mark, 32.0, pal))
                            .child(kit::title_sub(
                                h.host.clone(),
                                Some(
                                    div()
                                        .when(not_set, |t| t.text_color(pal.warning))
                                        .child(about.join(" · "))
                                        .into_any_element(),
                                ),
                                pal,
                            ))
                            .child(kit::switch(SharedString::from(format!("host-{}", h.host)), on, pal).on_click(
                                Self::on_model(cx, move |m, cx| m.change(Setting::Host(host.clone(), !on), cx)),
                            )),
                    );
                if !repos.is_empty() {
                    let mut repo_rows = div().flex().flex_col().pl(px(60.0)).pr(px(14.0)).pb(px(8.0));
                    for (repo, c) in repos.iter().take(8) {
                        repo_rows = repo_rows.child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(8.0))
                                .min_w(px(0.0))
                                .font_family(MONO)
                                .text_size(px(13.0))
                                .line_height(px(22.0))
                                .text_color(pal.ink_2())
                                .child(div().flex_1().min_w(px(0.0)).truncate().child((*repo).to_owned()))
                                .child(div().flex_none().child(format!(
                                    "{} · {}",
                                    c.count,
                                    short_ago(d.now, c.last_at)
                                ))),
                        );
                    }
                    block = block.child(repo_rows);
                }
                items.push(block.into_any_element());
            }
            let file = std::path::PathBuf::from(&d.config_file);
            let add_host = kit::pane::plus("add-host", pal).on_click(move |_, _, cx| cx.open_with_system(&file));
            right = right
                .child(kit::section(
                    Some(Glyph::GitBranch),
                    if d.pause.on() {
                        "git · paused, straight to the hosts"
                    } else {
                        "git · pushes wait for your phone"
                    },
                    Some(kit::section_count(Some(all.iter().filter(|h| h.enabled).count()), Some(add_host), pal)),
                    pal,
                ))
                .child(kit::group(items, kit::pane::INSET, glass, pal));
        }

        // MCP tools.
        let mut mcp: Vec<(String, ToolCalls)> = mcp_by_source(d.activity()).into_iter().collect();
        mcp.sort_by_key(|(_, t)| std::cmp::Reverse(t.last_at));
        right = right.child(kit::section(Some(Glyph::Link), "MCP tool calls", None, pal));
        right = right.child(if mcp.is_empty() {
            quiet("No calls yet")
        } else {
            let mut items = Vec::new();
            for (source, t) in &mcp {
                let mut tools = t.tools.iter().take(2).cloned().collect::<Vec<_>>().join(", ");
                if t.tools.len() > 2 {
                    tools = format!("{tools} +{}", t.tools.len() - 2);
                }
                let source_name = source.clone();
                items.push(
                    kit::pane::clickable(kit::pane_row(), SharedString::from(format!("mcp-{source}")), pal)
                        .child(kit::pane::avatar(agent_mark(Some(source)), 32.0, pal))
                        .child(kit::title_sub(source.clone(), Some(div().child(tools).into_any_element()), pal))
                        .child(kit::pane::row_value(format::count(t.calls, "call", "calls"), pal))
                        .child(kit::pane::chevron(pal))
                        .on_click(Self::on_view(cx, move |this, cx| {
                            this.kind_filter = Some(Kind::Mcp);
                            this.outcome_filter = None;
                            let _ = &source_name;
                            this.model.update(cx, |m, cx| m.show_section(Section::Activity, cx));
                        }))
                        .into_any_element(),
                );
            }
            kit::group(items, kit::pane::INSET, glass, pal)
        });

        // SSH.
        let ssh: Vec<&Connection> = conns.iter().filter(|c| c.kind == "ssh").collect();
        let ssh_on = d.config().is_some_and(|c| c.ssh.enabled);
        right = right.child(kit::section(Some(Glyph::Terminal), "SSH sign-ins", None, pal));
        right = right.child(if ssh.is_empty() && !ssh_on {
            kit::group(
                [kit::pane::clickable(kit::pane_row(), "to-ssh", pal)
                    .child(kit::pane::tile(Glyph::Terminal, pal))
                    .child(kit::title_sub("SSH agent off", None, pal))
                    .child(kit::pane::row_value("Turn on", pal))
                    .child(kit::pane::chevron(pal))
                    .on_click(Self::go(cx, Section::Keys))
                    .into_any_element()],
                kit::pane::INSET,
                glass,
                pal,
            )
        } else if ssh.is_empty() {
            quiet("No sign-ins yet")
        } else {
            let items = ssh
                .iter()
                .map(|c| {
                    kit::pane_row()
                        .child(kit::pane::tile(Glyph::Terminal, pal))
                        .child(kit::title_sub(
                            c.target.clone(),
                            Some(div().child(format::count(c.count, "sign-in", "sign-ins")).into_any_element()),
                            pal,
                        ))
                        .child(kit::pane::row_value(short_ago(d.now, c.last_at), pal))
                        .into_any_element()
                })
                .collect::<Vec<_>>();
            kit::group(items, kit::pane::INSET, glass, pal)
        });
        if let Some(started) = d.overview().map(|o| o.started_at).filter(|&s| s > 0) {
            right = right.child(
                div()
                    .px(px(6.0))
                    .pt(px(12.0))
                    .text_size(px(12.5))
                    .text_color(pal.ink_meta())
                    .child(format!("Counts since the service started, {}.", format::ago(d.now, started))),
            );
        }

        page.child(div().mt(px(4.0)).child(two_columns(left, right))).into_any_element()
    }

    /// One AI tool (`DeskNext-Connections.png` 02): its mark and name, how it is connected, what it asked lately, and
    /// Disconnect, held, as the quiet last row.
    fn tool_page(&self, h: Harness, d: &Data, pal: Palette, cx: &mut Context<'_, Self>) -> AnyElement {
        let row = d.snapshot.as_ref().and_then(|s| s.harnesses.iter().find(|r| r.harness == h).cloned());
        let how = match row.as_ref().map(|r| (r.added, r.complete)) {
            Some((true, true)) => "Hook and MCP",
            Some((true, false)) => "Partly connected",
            _ => "Not connected",
        };
        let back = kit::icon_button("tool-back", Glyph::ChevronLeft, 36.0, pal)
            .on_click(Self::on_view(cx, |this, _| this.tool_page = None));
        let head = div()
            .flex()
            .items_center()
            .gap(px(14.0))
            .child(back)
            .child(kit::pane::avatar(harness_mark(h), 48.0, pal))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .min_w(px(0.0))
                    .child(div().title().text_color(pal.ink()).child(h.label()))
                    .child(
                        div().mt(px(4.0)).mood().text_color(pal.ink_2()).child(format!("{how} · {}", this_computer())),
                    ),
            );
        let glass = kit::Glass::Solid;
        let recent: Vec<AnyElement> = d
            .activity()
            .iter()
            .filter(|e| e.source.as_deref() == Some(h.label()))
            .take(6)
            .map(|e| {
                Self::entry_row(e, "tool", d, pal, false, pal.paper_raised())
                    .on_click(Self::open_entry(cx, e.id.clone()))
                    .into_any_element()
            })
            .collect();
        let recent = if recent.is_empty() {
            kit::group(
                [div()
                    .px(px(14.0))
                    .py(px(16.0))
                    .text_size(px(14.5))
                    .text_color(pal.ink_meta())
                    .child("Nothing asked yet")
                    .into_any_element()],
                14.0,
                glass,
                pal,
            )
        } else {
            kit::group(recent, kit::pane::INSET, glass, pal)
        };
        let mut left = div().flex().flex_col();
        if row.as_ref().is_some_and(|r| r.added && !r.complete) {
            left = left.child(kit::section(None, "Repair", None, pal)).child(kit::group(
                [kit::pane::clickable(kit::pane_row(), "tool-repair", pal)
                    .child(kit::pane::tile(Glyph::Refresh, pal))
                    .child(kit::title_sub(
                        "Repair",
                        Some(div().child("Add the hook and the MCP server again").into_any_element()),
                        pal,
                    ))
                    .on_click(Self::on_model(cx, move |m, cx| m.set_harness(h, true, cx)))
                    .into_any_element()],
                kit::pane::INSET,
                glass,
                pal,
            ));
        }
        if row.as_ref().is_some_and(|r| r.added) {
            left = left.child(div().mt(px(18.0)).child(self.hold_row(
                "tool-disconnect",
                Held::DisconnectTool(h),
                Glyph::Unlink,
                format!("Disconnect {}", h.label()),
                "Hold to disconnect. It stops asking through Reins.",
                pal,
                cx,
            )));
        }
        let right =
            div().flex().flex_col().child(kit::section(Some(Glyph::Activity), "Recent", None, pal)).child(recent);
        div().flex().flex_col().child(head).child(div().mt(px(8.0)).child(two_columns(left, right))).into_any_element()
    }
}

/// Two columns 28 px apart while each has 340 px, else one under the other.
pub fn two_columns(left: Div, right: Div) -> Div {
    div()
        .flex()
        .flex_wrap()
        .items_start()
        .gap_x(px(28.0))
        .child(left.flex_1().min_w(px(340.0)))
        .child(right.flex_1().min_w(px(340.0)))
}

#[cfg(test)]
mod tests {
    use reins_desktop::journal::Outcome;

    use super::*;

    fn conn(kind: &str, target: &str, count: u64) -> Connection {
        Connection {
            kind: kind.to_owned(),
            target: target.to_owned(),
            count,
            last_at: 100,
            last: "read".to_owned(),
        }
    }

    #[test]
    fn repositories_belong_to_their_host() {
        let conns = [
            conn("git", "github.com/acme/web", 3),
            conn("git", "github.community/x/y", 1),
            conn("git", "gitlab.com/acme/infra", 2),
            conn("api", "github.com/acme/web", 9),
        ];
        let repos = repos_of(&conns, "github.com");
        assert_eq!(repos.len(), 1);
        assert_eq!(repos[0].0, "acme/web");
        assert_eq!(repos_of(&conns, "gitlab.com")[0].0, "acme/infra");
    }

    #[test]
    fn mcp_calls_are_grouped_by_tool() {
        let call = |source: &str, tool: &str, at: i64| {
            let mut e = Entry::new(Kind::Mcp, tool).source(Some(source)).service(Some(tool));
            e.at = at;
            e.outcome = Outcome::Approved;
            e
        };
        let entries = [
            call("Codex", "gmail_send", 30),
            call("Codex", "gmail_read", 20),
            call("Codex", "gmail_send", 10),
            call("Claude Code", "calendar_create_event", 5),
            Entry::new(Kind::Git, "push"),
        ];
        let by = mcp_by_source(&entries);
        assert_eq!(by.len(), 2);
        let codex = &by["Codex"];
        assert_eq!((codex.calls, codex.last_at), (3, 30));
        assert_eq!(codex.tools, vec!["gmail_send", "gmail_read"]);
    }

    #[test]
    fn every_ai_tool_has_a_logo_and_apis_their_host() {
        assert_eq!(harness_mark(Harness::ClaudeCode), Mark::Brand(Brand::Claude));
        assert_eq!(harness_mark(Harness::Codex), Mark::Glyph(Glyph::Terminal));
        assert_eq!(api_host("https://api.openai.com/v1/"), "api.openai.com/v1");
    }
}
