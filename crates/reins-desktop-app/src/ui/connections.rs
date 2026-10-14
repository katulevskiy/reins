//! Connections, in two columns: the AI tools Reins is in, the APIs called with keys from the phone and the MCP tool
//! calls on the left; the git hosts sent through it (with the repositories reached) and SSH sign-ins on the right.
//! Each with its real logo and a status line under it.

use std::collections::BTreeMap;

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, Context, Div, InteractiveElement as _, IntoElement, ParentElement as _, SharedString,
    StatefulInteractiveElement as _, Styled as _, div, px,
};
use reins_desktop::harness::Harness;
use reins_desktop::journal::{Entry, Kind};
use reins_desktop::stats::Connection;

use super::icons::{Brand, Icon, Mark, icon};
use super::parts::{
    caption, card, empty, empty_state, fine, group, help, link, logo, one_line, page_header, row, row_text, toggle,
};
use super::story::agent_mark;
use super::{Data, Root};
use crate::backend::Setting;
use crate::format;
use crate::model::Section;
use crate::theme::{MONO, Palette};

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
        Harness::Codex => Mark::Icon(Icon::SquareTerminal),
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

/// A two-column page: `left` and `right`, side by side while there is room, else one under the other.
fn columns(left: Div, right: Div) -> Div {
    div().flex().flex_wrap().gap(px(16.0)).child(left.flex_1().min_w(px(280.0))).child(right.flex_1().min_w(px(280.0)))
}

impl Root {
    pub(super) fn connections(d: &Data, pal: Palette, cx: &mut Context<'_, Self>) -> AnyElement {
        let empty_conns = Vec::new();
        let conns = d.overview().map_or(&empty_conns, |o| &o.connections);
        let mut page = div().flex().flex_col().gap(px(18.0)).child(page_header(
            "Connections",
            Some(help(
                "connections",
                "What reaches the outside through Reins on this computer, and the AI tools it watches over.",
                d,
                pal,
                cx,
            )),
        ));
        page = page.when_some(d.notice.clone(), |p, n| p.child(caption(n, pal).px(px(4.0)).text_color(pal.danger)));
        page = page.when_some(d.note.clone(), |p, n| p.child(caption(n, pal).px(px(4.0))));
        let mut left = div().flex().flex_col().gap(px(20.0));
        let mut right = div().flex().flex_col().gap(px(20.0));

        // AI tools.
        if let Some(s) = &d.snapshot {
            let mut tools = card(pal);
            let mut rows = s.harnesses.clone();
            rows.sort_by_key(|r| !(r.found || r.added));
            for (i, r) in rows.into_iter().enumerate() {
                let h: Harness = r.harness;
                let used = d.activity().iter().find(|e| e.source.as_deref() == Some(h.label())).map(|e| e.at);
                let (line, color) = match (r.found, r.added, r.complete) {
                    (_, true, true) => ("Hook + MCP".to_owned(), pal.tertiary),
                    (_, true, false) => ("Partly connected: turn off and on".to_owned(), pal.warning),
                    (true, false, _) => ("Installed, not connected".to_owned(), pal.warning),
                    (false, false, _) => ("Not installed".to_owned(), pal.tertiary),
                };
                let line = match used.filter(|_| r.added) {
                    Some(at) => format!("{line} · used {}", format::ago(d.now, at)),
                    None => line,
                };
                let mut item = row(pal, i == 0)
                    .id(SharedString::from(format!("harness-{}", h.id())))
                    .child(logo(harness_mark(h), pal))
                    .child(row_text(h.label(), Some(div().text_color(color).child(line).into_any_element()), pal));
                if r.found || r.added {
                    let on = r.added;
                    item = item
                        .child(toggle(on, pal))
                        .cursor_pointer()
                        .on_click(Self::on_model(cx, move |m, cx| m.set_harness(h, !on, cx)));
                } else {
                    item = item.opacity(0.6);
                }
                tools = tools.child(item);
            }
            left = left.child(group(
                "AI tools",
                Some(help(
                    "conn-tools",
                    "Reins adds itself to each one: an MCP server for its tools and a hook before risky commands. \
                     Install one, then turn it on here.",
                    d,
                    pal,
                    cx,
                )),
                tools,
                pal,
            ));
        }

        // APIs.
        let apis: Vec<&Connection> = conns.iter().filter(|c| c.kind == "api").collect();
        let api_body = if apis.is_empty() && d.config().is_none_or(|c| c.api.is_empty()) {
            card(pal).child(
                empty_state(Icon::Globe, "No APIs yet", None, pal)
                    .child(link("to-keys", "Add an API key", pal).on_click(Self::go(cx, Section::Keys))),
            )
        } else if apis.is_empty() {
            card(pal).child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(12.0))
                    .pr(px(14.0))
                    .child(empty("No calls yet", pal).flex_1())
                    .child(link("to-keys", "API keys", pal).on_click(Self::go(cx, Section::Keys))),
            )
        } else {
            let mut list = card(pal);
            for (i, c) in apis.iter().enumerate() {
                let base =
                    d.config().and_then(|cfg| cfg.api.iter().find(|a| a.name == c.target)).map(|a| a.base.clone());
                let name = base.as_deref().map_or_else(|| c.target.clone(), api_host);
                let mark = if name.contains("anthropic") {
                    Mark::Brand(Brand::Anthropic)
                } else {
                    Mark::Icon(Icon::Globe)
                };
                list = list.child(
                    row(pal, i == 0)
                        .id(SharedString::from(format!("api-{}", c.target)))
                        .cursor_pointer()
                        .hover(|s| s.bg(pal.control_fill.opacity(0.5)))
                        .child(logo(mark, pal))
                        .child(row_text(
                            name,
                            Some(usage(c.count, c.last_at, d.now, "call", "calls").into_any_element()),
                            pal,
                        ))
                        .child(icon(Icon::ChevronRight, 16.0, pal.tertiary))
                        .on_click(Self::go(cx, Section::Keys)),
                );
            }
            list
        };
        left = left.child(group(
            "APIs",
            Some(help(
                "conn-apis",
                "A program can call an API (OpenAI, Anthropic, …) through Reins with a key that stays in the vault on \
                 your phone: the key is asked for once, then held for a while.",
                d,
                pal,
                cx,
            )),
            api_body,
            pal,
        ));

        // MCP tools.
        let mcp = mcp_by_source(d.activity());
        let mcp_body = if mcp.is_empty() {
            card(pal).child(empty("No calls yet", pal))
        } else {
            let mut list = card(pal);
            let mut mcp: Vec<(String, ToolCalls)> = mcp.into_iter().collect();
            mcp.sort_by_key(|(_, t)| std::cmp::Reverse(t.last_at));
            for (i, (source, t)) in mcp.iter().enumerate() {
                let mut tools = t.tools.iter().take(3).cloned().collect::<Vec<_>>().join(", ");
                if t.tools.len() > 3 {
                    tools = format!("{tools} +{}", t.tools.len() - 3);
                }
                list = list.child(
                    row(pal, i == 0)
                        .child(logo(agent_mark(Some(source)), pal))
                        .child(row_text(
                            source.clone(),
                            Some(div().font_family(MONO).text_size(px(11.5)).child(tools).into_any_element()),
                            pal,
                        ))
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .flex_none()
                                .items_end()
                                .child(
                                    div()
                                        .text_size(px(12.0))
                                        .text_color(pal.secondary)
                                        .child(format::count(t.calls, "call", "calls")),
                                )
                                .child(fine(format::ago(d.now, t.last_at), pal)),
                        ),
                );
            }
            list
        };
        left = left.child(group(
            "MCP tool calls",
            Some(help(
                "conn-mcp",
                "Tools your AI agents called through Reins (your Gmail, calendar, GitHub, …), from this computer's \
                 activity log.",
                d,
                pal,
                cx,
            )),
            mcp_body,
            pal,
        ));

        // git hosts.
        if let Some(config) = d.config() {
            let mut hosts = card(pal);
            let routed = d.snapshot.as_ref().map(|s| s.git_routed.clone()).unwrap_or_default();
            for (i, h) in config.git_hosts().unwrap_or_default().into_iter().enumerate() {
                let repos = repos_of(conns, &h.host);
                let total: u64 = repos.iter().map(|(_, c)| c.count).sum();
                let last = repos.iter().map(|(_, c)| c.last_at).max();
                let mut about = Vec::new();
                match last {
                    Some(at) => about.push(usage(total, at, d.now, "request", "requests")),
                    None if h.enabled => about.push("Nothing yet".to_owned()),
                    None => about.push("Direct".to_owned()),
                }
                let not_set = h.enabled && !d.pause.on() && !routed.is_empty() && !routed.contains(&h.host);
                if not_set {
                    about.push("not set up in git yet".to_owned());
                }
                let (host, on) = (h.host.clone(), h.enabled);
                let mark = Brand::of_host(&h.host).map_or(Mark::Icon(Icon::GitBranch), Mark::Brand);
                hosts = hosts.child(
                    row(pal, i == 0)
                        .id(SharedString::from(format!("host-{}", h.host)))
                        .cursor_pointer()
                        .child(logo(mark, pal))
                        .child(row_text(
                            h.host.clone(),
                            Some(
                                div()
                                    .when(not_set, |t| t.text_color(pal.warning))
                                    .child(about.join(" · "))
                                    .into_any_element(),
                            ),
                            pal,
                        ))
                        .child(toggle(on, pal))
                        .on_click(Self::on_model(cx, move |m, cx| m.change(Setting::Host(host.clone(), !on), cx))),
                );
                if !repos.is_empty() {
                    let mut repo_rows =
                        div().flex().flex_col().gap(px(7.0)).pl(px(56.0)).pr(px(14.0)).pb(px(11.0)).pt(px(1.0));
                    for (repo, c) in repos.iter().take(8) {
                        repo_rows = repo_rows.child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(8.0))
                                .min_w(px(0.0))
                                .text_size(px(12.5))
                                .child(one_line((*repo).to_owned()).flex_1().font_family(MONO))
                                .child(div().flex_none().text_color(pal.tertiary).child(format!(
                                    "{} · {}",
                                    c.count,
                                    super::overview::short_ago(d.now, c.last_at)
                                ))),
                        );
                    }
                    hosts = hosts.child(repo_rows);
                }
            }
            let about = if d.pause.on() {
                "Paused: git goes straight to the hosts for now. Pushes wait for your phone again when you resume."
            } else {
                "Clones, fetches and pushes for the hosts turned on go through Reins: pushes wait for your phone, and \
                 AI agents never see your token. Changing a host restarts the background service."
            };
            right = right.child(group("git", Some(help("conn-git", about, d, pal, cx)), hosts, pal));
        }

        // SSH.
        let ssh: Vec<&Connection> = conns.iter().filter(|c| c.kind == "ssh").collect();
        let ssh_on = d.config().is_some_and(|c| c.ssh.enabled);
        let ssh_body = if ssh.is_empty() && !ssh_on {
            card(pal).child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(12.0))
                    .pr(px(14.0))
                    .child(empty("SSH agent off", pal).flex_1())
                    .child(link("to-ssh", "Turn on", pal).on_click(Self::go(cx, Section::Keys))),
            )
        } else if ssh.is_empty() {
            card(pal).child(empty("No sign-ins yet", pal))
        } else {
            let mut list = card(pal);
            for (i, c) in ssh.iter().enumerate() {
                list = list.child(
                    row(pal, i == 0)
                        .child(logo(Mark::Icon(Icon::Terminal), pal))
                        .child(row_text(
                            c.target.clone(),
                            Some(usage(c.count, c.last_at, d.now, "sign-in", "sign-ins").into_any_element()),
                            pal,
                        ))
                        .child(icon(Icon::CircleCheck, 16.0, pal.success)),
                );
            }
            list
        };
        right = right.child(group(
            "SSH sign-ins",
            Some(help(
                "conn-ssh",
                "Servers ssh signed in to with a key from your phone, each sign-in approved there.",
                d,
                pal,
                cx,
            )),
            ssh_body,
            pal,
        ));
        if let Some(started) = d.overview().map(|o| o.started_at).filter(|&s| s > 0) {
            right = right.child(
                fine(format!("Counts since the service started, {}", format::ago(d.now, started)), pal).px(px(4.0)),
            );
        }

        page.child(columns(left, right)).into_any_element()
    }
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
        assert_eq!(harness_mark(Harness::Codex), Mark::Icon(Icon::SquareTerminal));
        assert_eq!(api_host("https://api.openai.com/v1/"), "api.openai.com/v1");
    }
}
