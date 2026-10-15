//! Keys (`Desk-Keys.png`): the APIs called with a key from the phone (held until when, or asking next time), the
//! `reins run` profiles (variables and their vault references, never values), the SSH agent with its keys, and how to
//! use them. Nothing secret is on this computer to show. Finding the secrets lying around and moving them to the
//! vault starts here too ([`super::secrets`]).

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, Context, Div, InteractiveElement as _, IntoElement, ParentElement as _, SharedString,
    StatefulInteractiveElement as _, Styled as _, div, px,
};
use reins_desktop::control::ApiInfo;

use super::icons::{Glyph, glyph};
use super::parts::{caption, columns, help, list_row, mono, page_title, section, title_meta};
use super::story::plain;
use super::{Data, Root, kit};
use crate::backend::Setting;
use crate::format;
use crate::theme::{MONO, Palette, Tone};

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

/// A mono line under a row's title.
fn mono_line(text: impl Into<SharedString>) -> AnyElement {
    mono(text, 12.0).into_any_element()
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
    pub(super) fn keys(&self, d: &Data, pal: Palette, cx: &mut Context<'_, Self>) -> AnyElement {
        const MOOD: &str = "Secrets live on your phone. Only their names live here.";
        let file = std::path::PathBuf::from(&d.config_file);
        let open_config = kit::button(
            "open-config",
            kit::Look::Outline,
            kit::Size::Small,
            Some(Glyph::File),
            "Open config.toml",
            pal,
        )
        .h(px(36.0))
        .px(px(14.0))
        .text_size(px(13.5))
        .on_click(move |_, _, cx| cx.open_with_system(&file));
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
                .gap(px(22.0))
                .child(title)
                .when_some(d.notice.clone(), |p, n| p.child(caption(n, pal).text_color(pal.danger)))
                .child(caption(why, pal))
                .into_any_element();
        };
        let overview = d.overview().cloned().unwrap_or_default();
        let listen = d.snapshot.as_ref().map_or_else(|| config.listen.to_string(), |s| s.listen());
        let mut page = div().flex().flex_col().gap(px(22.0)).child(title);
        page = page.when_some(d.notice.clone(), |p, n| p.child(caption(n, pal).text_color(pal.danger)));
        page = page.when_some(d.note.clone(), |p, n| p.child(caption(n, pal)));
        page = page.child(Self::secrets_hero(d, pal, cx));

        // API keys.
        let quiet =
            |text: &str| div().pt(px(14.0)).text_size(px(13.0)).text_color(pal.ink_meta()).child(text.to_owned());
        let apis = if config.api.is_empty() {
            div().flex().flex_col().child(quiet("No API keys yet")).child(sample(
                &[
                    "[[api]]",
                    "name = \"openai\"",
                    "base = \"https://api.openai.com/v1\"",
                    "secret = \"vault:OpenAI/api key\"",
                ],
                pal,
            ))
        } else {
            let mut list = div().flex().flex_col();
            for (i, a) in config.api.iter().enumerate() {
                let state = match lease_until(&overview.apis, &a.name, d.now) {
                    Some(until) => {
                        kit::status_pill(None, format!("Held until {}", format::clock(until)), Some(pal.success), pal)
                    }
                    None => kit::status_pill(None, "Asks next time", None, pal),
                };
                list = list.child(
                    list_row(pal, i + 1 == config.api.len())
                        .child(kit::kind_tile(Glyph::Key, Tone::Neutral, 32.0, pal))
                        .child(title_meta(
                            a.name.clone(),
                            Some(mono_line(format!("{} · held {}", a.secret, format::span(a.lease_secs.into())))),
                            pal,
                        ))
                        .child(state),
                );
            }
            list
        };
        let mut left = div().flex().flex_col().gap(px(24.0)).child(section(
            "API keys",
            Some(help(
                "keys-api",
                "Programs call these through Reins; the key is added on the way and never shown to them. After you \
                 approve, it is held for the time shown. Add one as an [[api]] entry in config.toml.",
                d,
                pal,
                cx,
            )),
            apis,
            pal,
        ));

        // Run profiles.
        let profiles = if config.run.profiles.is_empty() {
            div().flex().flex_col().child(quiet("No run profiles yet")).child(sample(
                &["[run.profiles.deploy]", "env = { AWS_SECRET_ACCESS_KEY = \"vault:AWS deploy/password\" }"],
                pal,
            ))
        } else {
            let mut list = div().flex().flex_col();
            let n = config.run.profiles.len();
            for (i, (name, p)) in config.run.profiles.iter().enumerate() {
                let is_last = i + 1 == n;
                let open = self.profile_open.as_deref() == Some(name.as_str());
                let mut under = p.purpose.clone().unwrap_or_default();
                if !under.is_empty() {
                    under.push_str(" · ");
                }
                under.push_str(&format::count(p.env.len() as u64, "secret", "secrets"));
                let key = name.clone();
                list = list.child(
                    list_row(pal, is_last || open)
                        .id(SharedString::from(format!("profile-{name}")))
                        .cursor_pointer()
                        .when(open, gpui::Styled::border_b_0)
                        .child(kit::kind_tile(Glyph::Terminal, Tone::Neutral, 32.0, pal))
                        .child(title_meta(mono(name.clone(), 14.0), Some(under.into_any_element()), pal))
                        .child(glyph(
                            if open {
                                Glyph::ChevronUp
                            } else {
                                Glyph::ChevronDown
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
                if open {
                    let mut vars = div()
                        .flex()
                        .flex_col()
                        .pb(px(10.0))
                        .when(!is_last, |v| v.border_b_1().border_color(pal.rule()));
                    for (var, reference) in &p.env {
                        vars = vars.child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(8.0))
                                .py(px(4.0))
                                .pl(px(44.0))
                                .min_w(px(0.0))
                                .font_family(MONO)
                                .text_size(px(12.5))
                                .text_color(pal.ink_2())
                                .child(div().flex_1().min_w(px(0.0)).truncate().child(var.clone()))
                                .child(div().flex_none().w(px(20.0)).text_color(pal.ink_faint()).child("←"))
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
                    list = list.child(vars);
                }
            }
            list
        };
        left = left.child(section(
            "Run profiles",
            Some(help(
                "keys-run",
                "Variables a program gets from your vault for one run (reins run --profile deploy -- ./deploy.sh), \
                 after you approve on your phone.",
                d,
                pal,
                cx,
            )),
            profiles,
            pal,
        ));

        // SSH agent.
        let on = config.ssh.enabled;
        let socket = overview.ssh_socket.clone().unwrap_or_else(|| "starts with the service".to_owned());
        let keys = if on {
            overview.ssh_keys.clone()
        } else {
            Vec::new()
        };
        let mut ssh = div().flex().flex_col().child(
            list_row(pal, !on)
                .child(kit::kind_tile(Glyph::Terminal, Tone::Neutral, 32.0, pal))
                .child(title_meta("SSH keys from your phone", on.then(|| mono_line(socket)), pal))
                .child(
                    kit::switch("ssh-toggle", on, pal)
                        .on_click(Self::on_model(cx, move |m, cx| m.change(Setting::Ssh(!on), cx))),
                ),
        );
        if on && keys.is_empty() {
            ssh = ssh.child(
                quiet("Keys show after the first use").pl(px(44.0)).pb(px(12.0)).border_b_1().border_color(pal.rule()),
            );
        }
        for k in &keys {
            ssh = ssh.child(list_row(pal, false).child(div().flex_none().w(px(32.0))).child(title_meta(
                div().text_size(px(14.0)).child(k.name.clone()),
                Some(mono(short_fingerprint(&k.fingerprint), 11.5).into_any_element()),
                pal,
            )));
        }
        let mut right = div().flex().flex_col().gap(px(24.0)).child(section(
            "SSH agent",
            Some(help(
                "keys-ssh",
                "ssh signs in with keys from your phone and asks it each time. Changing it restarts the background \
                 service.",
                d,
                pal,
                cx,
            )),
            ssh,
            pal,
        ));

        // How to use them.
        let lines = how_to(
            &listen,
            config.api.first().map(|a| a.name.as_str()),
            config.run.profiles.keys().next().map(String::as_str),
            overview.ssh_socket.as_deref().filter(|_| config.ssh.enabled),
        );
        let mut code =
            div().flex().flex_col().font_family(MONO).text_size(px(12.5)).line_height(px(20.0)).text_color(pal.ink_2());
        for l in lines {
            let comment = l.starts_with('#');
            code = code.child(div().whitespace_nowrap().when(comment, |c| c.text_color(pal.ink_meta())).child(l));
        }
        right = right.child(kit::plate(pal).gap(px(10.0)).child(kit::section_label("Use them", pal)).child(code));

        page.child(columns(left, right, 400.0)).into_any_element()
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
