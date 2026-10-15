//! Keys & secrets: the APIs called with a key from the phone (held ones with a ring and until when), the `reins run`
//! profiles (variables and their vault references, never values), and the SSH agent with its keys. Nothing secret is
//! on this computer to show.

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, Context, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _, SharedString,
    StatefulInteractiveElement as _, Styled as _, div, px,
};
use reins_desktop::control::ApiInfo;

use super::icons::{Icon, icon};
use super::parts::{
    Btn, btn_icon, caption, card, chip_lead, chip_plain, code_block, empty, empty_state, group, help, help_with,
    page_header, ring, row, row_text, tag, tile, toggle,
};
use super::{Data, Root};
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

/// A mono line under a row's title.
fn mono_line(text: impl Into<SharedString>) -> AnyElement {
    div().font_family(MONO).text_size(px(11.5)).min_w(px(0.0)).truncate().child(text.into()).into_any_element()
}

impl Root {
    pub(super) fn keys(&self, d: &Data, pal: Palette, cx: &mut Context<'_, Self>) -> AnyElement {
        const ABOUT: &str = "Keys and secrets stay in the vault on your phone. Reins asks for one when a program needs \
                             it, and only the names and vault references are kept here.";
        let config_file = std::path::PathBuf::from(&d.config_file);
        let open_config = |id: &'static str| {
            let file = config_file.clone();
            btn_icon(id, Icon::FileCode, "Open config.toml", Btn::Plain, pal)
                .on_click(move |_, _, cx| cx.open_with_system(&file))
        };
        let Some(config) = d.config() else {
            let mut page =
                div().flex().flex_col().gap(px(18.0)).child(page_header("Keys", Some(help("keys", ABOUT, d, pal, cx))));
            page = page.when_some(d.notice.clone(), |p, n| p.child(caption(n, pal).text_color(pal.danger)));
            let why = d
                .snapshot
                .as_ref()
                .and_then(|s| s.config.as_ref().err().cloned())
                .unwrap_or_else(|| "Reading the settings…".to_owned());
            return page.child(card(pal).child(empty(why, pal))).into_any_element();
        };
        let overview = d.overview().cloned().unwrap_or_default();
        let listen = d.snapshot.as_ref().map_or_else(|| config.listen.to_string(), |s| s.listen());

        // What it is and how to use it, in the (?): the page itself lists what there is.
        let lines = how_to(
            &listen,
            config.api.first().map(|a| a.name.as_str()),
            config.run.profiles.keys().next().map(String::as_str),
            overview.ssh_socket.as_deref().filter(|_| config.ssh.enabled),
        );
        let about = move || {
            div().flex().flex_col().gap(px(10.0)).child(ABOUT).child(code_block(&lines, pal)).into_any_element()
        };
        let mut page = div().flex().flex_col().gap(px(18.0)).child(
            page_header("Keys", Some(help_with("keys", about, 520.0, d, pal, cx))).child(open_config("open-config")),
        );
        page = page.when_some(d.notice.clone(), |p, n| p.child(caption(n, pal).px(px(4.0)).text_color(pal.danger)));
        page = page.when_some(d.note.clone(), |p, n| p.child(caption(n, pal).px(px(4.0))));
        page = page.child(Self::secrets_hero(d, pal, cx));

        // APIs.
        let apis = if config.api.is_empty() {
            card(pal).child(
                empty_state(
                    Icon::KeyRound,
                    "No API keys yet",
                    Some(help(
                        "keys-api-empty",
                        "Give a program an API without giving it the key: it calls the API through Reins, and the key \
                         comes from the vault on your phone after you approve. Add an [[api]] entry to config.toml.",
                        d,
                        pal,
                        cx,
                    )),
                    pal,
                )
                .child(div().w_full().max_w(px(460.0)).pt(px(4.0)).child(code_block(
                    &[
                        "[[api]]".to_owned(),
                        "name = \"openai\"".to_owned(),
                        "base = \"https://api.openai.com/v1\"".to_owned(),
                        "secret = \"vault:OpenAI/api key\"".to_owned(),
                    ],
                    pal,
                ))),
            )
        } else {
            let mut list = card(pal);
            for (i, a) in config.api.iter().enumerate() {
                let state: AnyElement = match lease_until(&overview.apis, &a.name, d.now) {
                    Some(until) => {
                        let lease = i64::from(a.lease_secs).max(1);
                        #[allow(clippy::cast_precision_loss, reason = "seconds of a day")]
                        let left = (until - d.now) as f32 / lease as f32;
                        chip_lead(
                            ring(left, pal.success, pal.success.opacity(0.2), 14.0, 2.0),
                            format!("Held until {}", format::clock(until)),
                            pal.success,
                        )
                        .into_any_element()
                    }
                    None => chip_plain(None, "Asks next time", pal).into_any_element(),
                };
                list = list.child(
                    row(pal, i == 0)
                        .child(tile(Icon::KeyRound, pal.orange(), 30.0))
                        .child(row_text(
                            a.name.clone(),
                            Some(mono_line(format!("{} · held {}", a.secret, format::span(a.lease_secs.into())))),
                            pal,
                        ))
                        .child(state),
                );
            }
            list
        };
        page = page.child(group(
            "API keys",
            Some(help(
                "keys-api",
                "Programs call these through Reins; the key is added on the way and never shown to them. After you \
                 approve, it is held for the time shown.",
                d,
                pal,
                cx,
            )),
            apis,
            pal,
        ));

        // Run profiles.
        let profiles = if config.run.profiles.is_empty() {
            card(pal).child(
                empty_state(
                    Icon::Rocket,
                    "No run profiles yet",
                    Some(help(
                        "keys-run-empty",
                        "A profile names the secrets a program gets in its environment for one run (`reins run \
                         --profile deploy -- ./deploy.sh`), each from your vault after you approve. Add one to \
                         config.toml.",
                        d,
                        pal,
                        cx,
                    )),
                    pal,
                )
                .child(div().w_full().max_w(px(460.0)).pt(px(4.0)).child(code_block(
                    &[
                        "[run.profiles.deploy]".to_owned(),
                        "env = { AWS_SECRET_ACCESS_KEY = \"vault:AWS deploy/password\" }".to_owned(),
                    ],
                    pal,
                ))),
            )
        } else {
            let mut list = card(pal);
            for (i, (name, p)) in config.run.profiles.iter().enumerate() {
                let open = self.profile_open.as_deref() == Some(name.as_str());
                let mut under = p.purpose.clone().unwrap_or_default();
                if !under.is_empty() {
                    under.push_str(" · ");
                }
                under.push_str(&format::count(p.env.len() as u64, "secret", "secrets"));
                let key = name.clone();
                list = list.child(
                    row(pal, i == 0)
                        .id(SharedString::from(format!("profile-{name}")))
                        .cursor_pointer()
                        .hover(|s| s.bg(pal.control_fill.opacity(0.5)))
                        .child(tile(Icon::Rocket, pal.accent, 30.0))
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .flex_1()
                                .min_w(px(0.0))
                                .gap(px(2.0))
                                .child(div().font_family(MONO).font_weight(FontWeight::MEDIUM).child(name.clone()))
                                .child(div().text_size(px(12.0)).text_color(pal.tertiary).truncate().child(under)),
                        )
                        .child(icon(
                            if open {
                                Icon::X
                            } else {
                                Icon::ChevronRight
                            },
                            16.0,
                            pal.tertiary,
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
                    let mut vars = div().flex().flex_col().gap(px(7.0)).pl(px(56.0)).pr(px(14.0)).pb(px(12.0));
                    for (var, reference) in &p.env {
                        vars = vars.child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(8.0))
                                .min_w(px(0.0))
                                .child(
                                    div()
                                        .flex_none()
                                        .w(px(200.0))
                                        .truncate()
                                        .font_family(MONO)
                                        .text_size(px(12.0))
                                        .text_color(pal.secondary)
                                        .child(var.clone()),
                                )
                                .child(icon(Icon::ArrowLeft, 12.0, pal.tertiary))
                                .child(tag(reference.clone(), pal)),
                        );
                    }
                    list = list.child(vars);
                }
            }
            list
        };
        page = page.child(group(
            "Run profiles",
            Some(help(
                "keys-run",
                "Variables a program gets from your vault for one run (reins run), after you approve on your phone.",
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
        let mut ssh = card(pal).child(
            row(pal, true)
                .id("ssh-toggle")
                .cursor_pointer()
                .child(tile(Icon::Terminal, pal.teal(), 30.0))
                .child(row_text("SSH keys from your phone", on.then(|| mono_line(socket)), pal))
                .child(help(
                    "keys-ssh",
                    "ssh signs in with keys from your phone and asks it each time. Changing it restarts the background \
                     service.",
                    d,
                    pal,
                    cx,
                ))
                .child(toggle(on, pal))
                .on_click(Self::on_model(cx, move |m, cx| m.change(Setting::Ssh(!on), cx))),
        );
        if on {
            if overview.ssh_keys.is_empty() {
                ssh =
                    ssh.child(row(pal, false).child(
                        div().text_size(px(12.5)).text_color(pal.tertiary).child("Keys show after the first use"),
                    ));
            } else {
                for k in &overview.ssh_keys {
                    ssh =
                        ssh.child(
                            row(pal, false)
                                .child(tile(Icon::KeyRound, pal.gray(), 30.0).bg(pal.tile()))
                                .child(row_text(k.name.clone(), Some(mono_line(k.fingerprint.clone())), pal)),
                        );
                }
            }
        }
        page = page.child(group("SSH agent", None, ssh, pal));
        page.into_any_element()
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
    fn how_to_uses_the_daemons_address() {
        let lines = how_to("127.0.0.1:7457", Some("my-api"), None, Some("/run/agent.sock"));
        assert!(lines.contains(&"MY_API_BASE_URL=http://127.0.0.1:7457/api/my-api".to_owned()));
        assert!(lines.contains(&"reins run --profile deploy -- ./deploy.sh".to_owned()));
        assert!(lines.contains(&"export SSH_AUTH_SOCK=/run/agent.sock".to_owned()));
        assert_eq!(how_to("127.0.0.1:1", None, Some("ci"), None)[1], "OPENAI_BASE_URL=http://127.0.0.1:1/api/openai");
    }
}
