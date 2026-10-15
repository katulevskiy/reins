//! Connections (`Desk-Connections.png`), in two columns of hairline rows: the AI tools Reins is in, the APIs called
//! with keys from the phone and the MCP tool calls on the left; the git hosts sent through it (with the repositories
//! reached) and SSH sign-ins on the right. Each with its real logo on a white tile and a line under it.

use std::collections::BTreeMap;

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, Context, InteractiveElement as _, IntoElement, ParentElement as _, SharedString,
    StatefulInteractiveElement as _, Styled as _, div, px,
};
use reins_desktop::harness::Harness;
use reins_desktop::journal::{Entry, Kind};
use reins_desktop::stats::Connection;

use super::icons::{Brand, Glyph, Icon, Mark, glyph};
use super::parts::{caption, columns, help, link, list_row, mono, page_title, section, this_computer, title_meta};
use super::story::{agent_mark, plain};
use super::{Data, Root, kit};
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

impl Root {
    pub(super) fn connections(d: &Data, pal: Palette, cx: &mut Context<'_, Self>) -> AnyElement {
        let empty_conns = Vec::new();
        let conns = d.overview().map_or(&empty_conns, |o| &o.connections);
        let mut page = div().flex().flex_col().gap(px(22.0)).child(page_title(
            &[plain("Connections")],
            Some(&format!("What reaches the outside through Reins on {}.", this_computer())),
            None,
            pal,
        ));
        page = page.when_some(d.notice.clone(), |p, n| p.child(caption(n, pal).text_color(pal.danger)));
        page = page.when_some(d.note.clone(), |p, n| p.child(caption(n, pal)));
        let mut left = div().flex().flex_col().gap(px(22.0));
        let mut right = div().flex().flex_col().gap(px(22.0));

        // AI tools.
        if let Some(s) = &d.snapshot {
            let mut tools = div().flex().flex_col();
            let mut rows = s.harnesses.clone();
            rows.sort_by_key(|r| !(r.found || r.added));
            let n = rows.len();
            for (i, r) in rows.into_iter().enumerate() {
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
                let meta = div().when(warn, |m| m.text_color(pal.warning)).child(line).into_any_element();
                let mut item = list_row(pal, i + 1 == n).child(kit::logo_tile(harness_mark(h), pal)).child(title_meta(
                    h.label(),
                    Some(meta),
                    pal,
                ));
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
                        .h(px(28.0))
                        .on_click(Self::on_model(cx, move |m, cx| m.set_harness(h, true, cx))),
                    );
                }
                if r.found || r.added {
                    let on = r.added;
                    item = item.child(
                        kit::switch(SharedString::from(format!("harness-{}", h.id())), on, pal)
                            .on_click(Self::on_model(cx, move |m, cx| m.set_harness(h, !on, cx))),
                    );
                } else {
                    item = item.opacity(0.6);
                }
                tools = tools.child(item);
            }
            left = left.child(section(
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
        let quiet =
            |text: &str| div().py(px(14.0)).text_size(px(13.0)).text_color(pal.ink_meta()).child(text.to_owned());
        let api_body = if apis.is_empty() {
            let none = d.config().is_none_or(|c| c.api.is_empty());
            div()
                .flex()
                .items_center()
                .gap(px(12.0))
                .child(
                    quiet(if none {
                        "No APIs yet"
                    } else {
                        "No calls yet"
                    })
                    .flex_1(),
                )
                .child(
                    link(
                        "to-keys",
                        if none {
                            "Add an API key"
                        } else {
                            "API keys"
                        },
                        pal,
                    )
                    .on_click(Self::go(cx, Section::Keys)),
                )
        } else {
            let mut list = div().flex().flex_col();
            for (i, c) in apis.iter().enumerate() {
                let base =
                    d.config().and_then(|cfg| cfg.api.iter().find(|a| a.name == c.target)).map(|a| a.base.clone());
                let name = base.as_deref().map_or_else(|| c.target.clone(), api_host);
                let mark = if name.contains("anthropic") {
                    Mark::Brand(Brand::Anthropic)
                } else if name.contains("openai") {
                    Mark::Brand(Brand::OpenAI)
                } else {
                    Mark::Icon(Icon::Globe)
                };
                list = list.child(
                    list_row(pal, i + 1 == apis.len())
                        .id(SharedString::from(format!("api-{}", c.target)))
                        .cursor_pointer()
                        .hover(|s| s.bg(pal.paper_sunken().opacity(0.5)))
                        .child(kit::logo_tile(mark, pal))
                        .child(title_meta(
                            mono(name, 13.5),
                            Some(usage(c.count, c.last_at, d.now, "call", "calls").into_any_element()),
                            pal,
                        ))
                        .child(glyph(Glyph::ChevronRight, 14.0, pal.ink_faint()))
                        .on_click(Self::go(cx, Section::Keys)),
                );
            }
            list
        };
        left = left.child(section(
            "APIs · keys from your phone",
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
        let mut mcp: Vec<(String, ToolCalls)> = mcp_by_source(d.activity()).into_iter().collect();
        mcp.sort_by_key(|(_, t)| std::cmp::Reverse(t.last_at));
        let mcp_body = if mcp.is_empty() {
            quiet("No calls yet")
        } else {
            let mut list = div().flex().flex_col();
            for (i, (source, t)) in mcp.iter().enumerate() {
                let mut tools = t.tools.iter().take(2).cloned().collect::<Vec<_>>().join(", ");
                if t.tools.len() > 2 {
                    tools = format!("{tools} +{}", t.tools.len() - 2);
                }
                list = list.child(
                    list_row(pal, i + 1 == mcp.len())
                        .child(kit::logo_tile(agent_mark(Some(source)), pal))
                        .child(title_meta(source.clone(), Some(mono(tools, 12.0).into_any_element()), pal))
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .flex_none()
                                .items_end()
                                .child(
                                    div()
                                        .text_size(px(13.5))
                                        .text_color(pal.ink())
                                        .child(format::count(t.calls, "call", "calls")),
                                )
                                .child(
                                    div()
                                        .text_size(px(12.5))
                                        .text_color(pal.ink_meta())
                                        .child(super::home::short_ago(d.now, t.last_at)),
                                ),
                        ),
                );
            }
            list
        };
        left = left.child(section(
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
            let mut hosts = div().flex().flex_col();
            let routed = d.snapshot.as_ref().map(|s| s.git_routed.clone()).unwrap_or_default();
            let all = config.git_hosts().unwrap_or_default();
            for (i, h) in all.iter().enumerate() {
                let last = i + 1 == all.len();
                let repos = repos_of(conns, &h.host);
                let total: u64 = repos.iter().map(|(_, c)| c.count).sum();
                let latest = repos.iter().map(|(_, c)| c.last_at).max();
                let mut about = Vec::new();
                match latest {
                    Some(at) => about.push(usage(total, at, d.now, "request", "requests")),
                    None if h.enabled => about.push("Nothing yet".to_owned()),
                    None => about.push("Direct · not through Reins".to_owned()),
                }
                let not_set = h.enabled && !d.pause.on() && !routed.is_empty() && !routed.contains(&h.host);
                if not_set {
                    about.push("not set up in git yet".to_owned());
                }
                let (host, on) = (h.host.clone(), h.enabled);
                let mark = Brand::of_host(&h.host).map_or(Mark::Icon(Icon::GitBranch), Mark::Brand);
                hosts =
                    hosts.child(
                        list_row(pal, last || !repos.is_empty())
                            .child(kit::logo_tile(mark, pal))
                            .child(title_meta(
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
                    let mut repo_rows =
                        div().flex().flex_col().pb(px(10.0)).when(!last, |r| r.border_b_1().border_color(pal.rule()));
                    for (repo, c) in repos.iter().take(8) {
                        repo_rows = repo_rows.child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(8.0))
                                .py(px(3.0))
                                .pl(px(44.0))
                                .min_w(px(0.0))
                                .font_family(MONO)
                                .text_size(px(12.5))
                                .text_color(pal.ink_2())
                                .child(div().flex_1().min_w(px(0.0)).truncate().child((*repo).to_owned()))
                                .child(div().flex_none().child(format!(
                                    "{} · {}",
                                    c.count,
                                    super::home::short_ago(d.now, c.last_at)
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
            right = right.child(section(
                "git · pushes wait for your phone",
                Some(help("conn-git", about, d, pal, cx)),
                hosts,
                pal,
            ));
        }

        // SSH.
        let ssh: Vec<&Connection> = conns.iter().filter(|c| c.kind == "ssh").collect();
        let ssh_on = d.config().is_some_and(|c| c.ssh.enabled);
        let ssh_body = if ssh.is_empty() && !ssh_on {
            div()
                .flex()
                .items_center()
                .gap(px(12.0))
                .child(quiet("SSH agent off").flex_1())
                .child(link("to-ssh", "Turn on", pal).on_click(Self::go(cx, Section::Keys)))
        } else if ssh.is_empty() {
            quiet("No sign-ins yet")
        } else {
            let mut list = div().flex().flex_col();
            for (i, c) in ssh.iter().enumerate() {
                let glyph_of = if c.target.contains("github") || c.target.contains("gitlab") {
                    Icon::Terminal
                } else {
                    Icon::Server
                };
                list = list.child(
                    list_row(pal, i + 1 == ssh.len())
                        .child(kit::logo_tile(Mark::Icon(glyph_of), pal))
                        .child(title_meta(
                            mono(c.target.clone(), 13.5),
                            Some(format::count(c.count, "sign-in", "sign-ins").into_any_element()),
                            pal,
                        ))
                        .child(
                            div()
                                .flex_none()
                                .text_size(px(12.5))
                                .text_color(pal.ink_meta())
                                .child(super::home::short_ago(d.now, c.last_at)),
                        ),
                );
            }
            list
        };
        right = right.child(section(
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
                div()
                    .text_size(px(12.5))
                    .text_color(pal.ink_meta())
                    .child(format!("Counts since the service started, {}.", format::ago(d.now, started))),
            );
        }

        page.child(columns(left, right, 380.0)).into_any_element()
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
