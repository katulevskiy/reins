//! Keys (`DeskNext-Keys.png`): the APIs called with a key from the phone (held until when, or asking next time), the
//! `reins run` profiles (variables and their vault references, never values), the SSH agent with its keys, and how to
//! use them. Nothing secret is on this computer to show. Finding the secrets lying around and moving them to the
//! vault starts here too ([`super::secrets`]).

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, Context, Div, IntoElement, ParentElement as _, SharedString, StatefulInteractiveElement as _,
    Styled as _, div, px,
};
use reins_desktop::control::ApiInfo;

use super::icons::{Glyph, glyph};
use super::parts::{caption, page_title};
use super::story::plain;
use super::{Data, Root, kit};
use crate::backend::Setting;
use crate::format;
use crate::theme::{MONO, Palette};

/// Until when the key of API `name` is held, if it is now.
#[must_use]
pub fn lease_until(apis: &[ApiInfo], name: &str, now: i64) -> Option<i64> {
    apis.iter().find(|a| a.name == name)?.leased_until.filter(|&u| u > now)
}

/// The lines of "How to use them".
#[must_use]
pub fn how_to(listen: &str, api: Option<&str>, profile: Option<&str>, socket: Option<&str>) -> Vec<String> {
    let api = api.unwrap_or("openai");
    let mut lines = vec![
        "# An API, its key from your phone".to_owned(),
        format!("{}_BASE_URL=http://{listen}/api/{api}", api.to_ascii_uppercase().replace('-', "_")),
        "# A program with secrets".to_owned(),
        format!("reins run --profile {} -- ./deploy.sh", profile.unwrap_or("deploy")),
    ];
    if let Some(socket) = socket {
        lines.push("# ssh with your phone's keys".to_owned());
        lines.push(format!("export SSH_AUTH_SOCK={socket}"));
    }
    lines
}

/// A key's fingerprint as the board shows it: its start and end, "SHA256:x1Pq7nE…9fZk".
#[must_use]
pub fn short_fingerprint(fp: &str) -> String {
    let chars: Vec<char> = fp.chars().collect();
    if chars.len() <= 24 {
        return fp.to_owned();
    }
    let (head, tail): (String, String) = (chars[..14].iter().collect(), chars[chars.len() - 4..].iter().collect());
    format!("{head}…{tail}")
}

/// A sample of `config.toml`, in a well, for an empty section.
fn sample(lines: &[&str], pal: Palette) -> Div {
    let mut well = div()
        .flex()
        .flex_col()
        .mt(px(8.0))
        .px(px(14.0))
        .py(px(12.0))
        .rounded(px(14.0))
        .bg(pal.paper_sunken())
        .font_family(MONO)
        .text_size(px(12.5))
        .line_height(px(19.0))
        .text_color(pal.ink_2());
    for l in lines {
        well = well.child(div().whitespace_nowrap().child((*l).to_owned()));
    }
    well
}

impl Root {
    /// Keys (`DeskNext-Keys.png`): the scan on top, then API keys and run profiles on the left, the SSH agent and the
    /// lines to paste on the right.
    pub(super) fn keys(&self, d: &Data, pal: Palette, cx: &mut Context<'_, Self>) -> AnyElement {
        const MOOD: &str = "Secrets live on your phone. Only their names live here.";
        let file = std::path::PathBuf::from(&d.config_file);
        let open = |_id: &'static str| {
            let file = file.clone();
            move |_: &gpui::ClickEvent, _: &mut gpui::Window, cx: &mut gpui::App| cx.open_with_system(&file)
        };
        let open_config =
            kit::button("open-config", kit::Look::Soft, kit::Size::Regular, Some(Glyph::File), "Open config.toml", pal)
                .on_click(open("open-config"));
        let title = page_title(&[plain("Keys")], Some(MOOD), Some(open_config.into_any_element()), pal);
        let Some(config) = d.config() else {
            let why = d
                .snapshot
                .as_ref()
                .and_then(|s| s.config.as_ref().err().cloned())
                .unwrap_or_else(|| "Reading the settings…".to_owned());
            return div()
                .flex()
                .flex_col()
                .gap(px(20.0))
                .child(title)
                .when_some(d.notice.clone(), |p, n| p.child(caption(n, pal).text_color(pal.danger)))
                .child(caption(why, pal))
                .into_any_element();
        };
        let glass = kit::Glass::Solid;
        let overview = d.overview().cloned().unwrap_or_default();
        let listen = d.snapshot.as_ref().map_or_else(|| config.listen.to_string(), |s| s.listen());
        let mut page = div().flex().flex_col().child(title);
        page = page.when_some(d.notice.clone(), |p, n| p.child(caption(n, pal).mt(px(12.0)).text_color(pal.danger)));
        page = page.when_some(d.note.clone(), |p, n| p.child(caption(n, pal).mt(px(12.0))));
        page = page.child(div().mt(px(20.0)).child(Self::secrets_hero(d, pal, cx)));
        let quiet = |text: &str, lines: &[&str]| -> AnyElement {
            let mut rows = vec![
                div()
                    .px(px(14.0))
                    .py(px(14.0))
                    .text_size(px(14.5))
                    .text_color(pal.ink_meta())
                    .child(text.to_owned())
                    .into_any_element(),
            ];
            rows.push(sample(lines, pal).mx(px(14.0)).mb(px(14.0)).mt(px(0.0)).into_any_element());
            kit::pane(div().flex().flex_col().children(rows), glass, kit::pane::RADIUS, pal)
        };

        // API keys.
        let apis = if config.api.is_empty() {
            quiet(
                "No API keys yet",
                &[
                    "[[api]]",
                    "name = \"openai\"",
                    "base = \"https://api.openai.com/v1\"",
                    "secret = \"vault:OpenAI/api key\"",
                ],
            )
        } else {
            let items = config
                .api
                .iter()
                .map(|a| {
                    let state = match lease_until(&overview.apis, &a.name, d.now) {
                        Some(until) => kit::status_pill(
                            Some(Glyph::Timer),
                            format!("Held until {}", format::clock(until)),
                            Some(pal.success),
                            pal,
                        ),
                        None => kit::status_pill(Some(Glyph::Phone), "Asks next time", Some(pal.ink_2()), pal),
                    };
                    kit::pane_row()
                        .child(kit::pane::tile(Glyph::Card, pal))
                        .child(kit::title_sub(
                            a.name.clone(),
                            Some(
                                div()
                                    .child(format!("{} · held {}", a.secret, format::span(a.lease_secs.into())))
                                    .into_any_element(),
                            ),
                            pal,
                        ))
                        .child(state)
                        .into_any_element()
                })
                .collect::<Vec<_>>();
            kit::group(items, kit::pane::INSET, glass, pal)
        };
        let mut left = div()
            .flex()
            .flex_col()
            .child(kit::section(
                Some(Glyph::Card),
                "API keys",
                Some(kit::section_count(
                    Some(config.api.len()),
                    Some(kit::pane::plus("add-api-key", pal).on_click(open("add-api-key"))),
                    pal,
                )),
                pal,
            ))
            .child(apis);

        // Run profiles.
        let profiles = if config.run.profiles.is_empty() {
            quiet(
                "No run profiles yet",
                &["[run.profiles.deploy]", "env = { AWS_SECRET_ACCESS_KEY = \"vault:AWS deploy/password\" }"],
            )
        } else {
            let mut items = Vec::new();
            for (name, p) in &config.run.profiles {
                let open_row = self.profile_open.as_deref() == Some(name.as_str());
                let key = name.clone();
                let mut block = div().flex().flex_col().child(
                    kit::pane::clickable(kit::pane_row(), SharedString::from(format!("profile-{name}")), pal)
                        .child(kit::pane::tile(Glyph::Play, pal))
                        .child(kit::title_sub(
                            name.clone(),
                            p.purpose.clone().filter(|s| !s.is_empty()).map(|s| div().child(s).into_any_element()),
                            pal,
                        ))
                        .child(kit::pane::row_value(format::count(p.env.len() as u64, "secret", "secrets"), pal))
                        .child(glyph(
                            if open_row {
                                Glyph::ChevronDown
                            } else {
                                Glyph::ChevronRight
                            },
                            14.0,
                            pal.ink_faint(),
                        ))
                        .on_click(Self::on_view(cx, move |this, _| {
                            this.profile_open = if this.profile_open.as_deref() == Some(key.as_str()) {
                                None
                            } else {
                                Some(key.clone())
                            };
                        })),
                );
                if open_row {
                    let mut vars = div().flex().flex_col().pl(px(62.0)).pr(px(14.0)).pb(px(10.0));
                    for (var, reference) in &p.env {
                        vars = vars.child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(8.0))
                                .py(px(3.0))
                                .min_w(px(0.0))
                                .font_family(MONO)
                                .text_size(px(13.0))
                                .text_color(pal.ink_2())
                                .child(div().flex_1().min_w(px(0.0)).truncate().child(var.clone()))
                                .child(glyph(Glyph::ArrowBack, 13.0, pal.ink_faint()))
                                .child(
                                    div().flex_1().min_w(px(0.0)).child(
                                        div()
                                            .max_w_full()
                                            .px(px(6.0))
                                            .py(px(2.0))
                                            .rounded(px(6.0))
                                            .bg(pal.violet().opacity(0.10))
                                            .text_color(pal.violet())
                                            .truncate()
                                            .child(reference.clone()),
                                    ),
                                ),
                        );
                    }
                    block = block.child(vars);
                }
                items.push(block.into_any_element());
            }
            kit::group(items, kit::pane::INSET, glass, pal)
        };
        left = left
            .child(kit::section(
                Some(Glyph::Play),
                "Run profiles",
                Some(kit::section_count(
                    Some(config.run.profiles.len()),
                    Some(kit::pane::plus("add-profile", pal).on_click(open("add-profile"))),
                    pal,
                )),
                pal,
            ))
            .child(profiles);

        // SSH agent.
        let on = config.ssh.enabled;
        let socket = overview.ssh_socket.clone().unwrap_or_else(|| "starts with the service".to_owned());
        let keys = if on {
            overview.ssh_keys.clone()
        } else {
            Vec::new()
        };
        let mut ssh = vec![
            kit::pane_row()
                .child(kit::pane::tile(Glyph::Terminal, pal))
                .child(kit::title_sub(
                    "SSH keys from your phone",
                    on.then(|| div().child(socket).into_any_element()),
                    pal,
                ))
                .child(
                    kit::switch("ssh-toggle", on, pal)
                        .on_click(Self::on_model(cx, move |m, cx| m.change(Setting::Ssh(!on), cx))),
                )
                .into_any_element(),
        ];
        if on && keys.is_empty() {
            ssh.push(
                kit::pane_row()
                    .child(div().flex_none().w(px(34.0)))
                    .child(div().text_size(px(14.0)).text_color(pal.ink_meta()).child("Keys show after the first use"))
                    .into_any_element(),
            );
        }
        for k in &keys {
            ssh.push(
                kit::pane_row()
                    .child(kit::pane::tile(Glyph::Key, pal))
                    .child(kit::title_sub(
                        k.name.clone(),
                        Some(div().child(short_fingerprint(&k.fingerprint)).into_any_element()),
                        pal,
                    ))
                    .into_any_element(),
            );
        }
        let mut right = div()
            .flex()
            .flex_col()
            .child(kit::section(Some(Glyph::Terminal), "SSH agent", None, pal))
            .child(kit::group(ssh, kit::pane::INSET, glass, pal));

        // Use them.
        let lines = how_to(
            &listen,
            config.api.first().map(|a| a.name.as_str()),
            config.run.profiles.keys().next().map(String::as_str),
            overview.ssh_socket.as_deref().filter(|_| config.ssh.enabled),
        );
        let mut code = div()
            .flex()
            .flex_col()
            .py(px(10.0))
            .rounded(px(14.0))
            .bg(pal.paper_sunken())
            .font_family(MONO)
            .text_size(px(12.5))
            .line_height(px(20.0))
            .text_color(pal.ink())
            .overflow_hidden();
        for l in lines {
            let comment = l.starts_with('#');
            code = code
                .child(div().px(px(14.0)).whitespace_nowrap().when(comment, |c| c.text_color(pal.ink_meta())).child(l));
        }
        right = right.child(kit::section(None, "Use them", None, pal)).child(code);

        page.child(super::connections::two_columns(left, right)).into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn leases_show_while_they_last() {
        let apis = [ApiInfo {
            name: "openai".to_owned(),
            base: "https://api.openai.com/v1".to_owned(),
            leased_until: Some(2_000),
        }];
        assert_eq!(lease_until(&apis, "openai", 1_000), Some(2_000));
        assert_eq!(lease_until(&apis, "openai", 2_001), None);
        assert_eq!(lease_until(&apis, "other", 1_000), None);
    }

    #[test]
    fn fingerprints_keep_their_ends() {
        assert_eq!(short_fingerprint("SHA256:x1Pq7nEaaaaaaaaaaaaaaaaaaaaaaaaa9fZk"), "SHA256:x1Pq7nE…9fZk");
        assert_eq!(short_fingerprint("SHA256:short"), "SHA256:short");
    }

    #[test]
    fn how_to_uses_the_daemons_address() {
        let lines = how_to("127.0.0.1:7457", Some("my-api"), None, Some("/run/agent.sock"));
        assert!(lines.contains(&"MY_API_BASE_URL=http://127.0.0.1:7457/api/my-api".to_owned()));
        assert!(lines.contains(&"reins run --profile deploy -- ./deploy.sh".to_owned()));
        assert!(lines.contains(&"export SSH_AUTH_SOCK=/run/agent.sock".to_owned()));
        assert_eq!(how_to("127.0.0.1:1", None, Some("ci"), None)[1], "OPENAI_BASE_URL=http://127.0.0.1:1/api/openai");
    }
}
