//! Keys & secrets: the APIs called with a key from the phone, the `reins run` profiles (variables and their vault
//! references, never values), and the SSH agent. Nothing secret is on this computer to show.

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, Context, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, div, px,
};
use reins_desktop::control::ApiInfo;

use super::parts::{
    caption, card, code_block, empty, fine, group, labelled, mono, one_line, page_header, row, tag, toggle,
};
use super::{Data, Root};
use crate::backend::Setting;
use crate::format;
use crate::theme::{MONO, Palette};

/// Whether the key of API `name` is held now, in words.
#[must_use]
pub fn lease_state(apis: &[ApiInfo], name: &str, now: i64) -> Option<String> {
    let until = apis.iter().find(|a| a.name == name)?.leased_until.filter(|&u| u > now)?;
    Some(format!("Key held until {}", format::clock(until)))
}

/// The lines of "How to use them".
#[must_use]
pub fn how_to(listen: &str, api: Option<&str>, profile: Option<&str>, socket: Option<&str>) -> Vec<String> {
    let api = api.unwrap_or("openai");
    let mut lines = vec![
        "# An API: Reins adds the key from your phone".to_owned(),
        format!("{}_BASE_URL=http://{listen}/api/{api}", api.to_ascii_uppercase().replace('-', "_")),
        "# A program with secrets in its environment".to_owned(),
        format!("reins run --profile {} -- ./deploy.sh", profile.unwrap_or("deploy")),
    ];
    if let Some(socket) = socket {
        lines.push("# ssh with keys from your phone".to_owned());
        lines.push(format!("export SSH_AUTH_SOCK={socket}"));
    }
    lines
}

impl Root {
    pub(super) fn keys(d: &Data, pal: Palette, cx: &mut Context<'_, Self>) -> AnyElement {
        let mut page = div().flex().flex_col().gap(px(22.0)).child(page_header(
            "Keys & secrets",
            "Keys and secrets stay in the vault on your phone. Reins asks for one when a program needs it, and only \
             the names and vault references are kept here.",
            pal,
        ));
        page = page.when_some(d.notice.clone(), |p, n| p.child(caption(n, pal).text_color(pal.danger)));
        page = page.when_some(d.note.clone(), |p, n| p.child(caption(n, pal)));
        let Some(config) = d.config() else {
            let why = d
                .snapshot
                .as_ref()
                .and_then(|s| s.config.as_ref().err().cloned())
                .unwrap_or_else(|| "Reading the settings…".to_owned());
            return page.child(card(pal).child(empty(why, pal))).into_any_element();
        };
        let overview = d.overview().cloned().unwrap_or_default();
        let listen = d.snapshot.as_ref().map_or_else(|| config.listen.to_string(), |s| s.listen());

        page = page.child(group(
            "How to use them",
            None,
            code_block(
                &how_to(
                    &listen,
                    config.api.first().map(|a| a.name.as_str()),
                    config.run.profiles.keys().next().map(String::as_str),
                    overview.ssh_socket.as_deref().filter(|_| config.ssh.enabled),
                ),
                pal,
            ),
            pal,
        ));

        // APIs.
        let apis = if config.api.is_empty() {
            card(pal).child(empty(
                format!(
                    "No APIs yet. Add an [[api]] entry (name, base, secret = \"vault:Item/field\") to {}.",
                    d.config_file
                ),
                pal,
            ))
        } else {
            let mut list = card(pal);
            for (i, a) in config.api.iter().enumerate() {
                let state = lease_state(&overview.apis, &a.name, d.now);
                list = list.child(
                    row(pal, i == 0)
                        .items_start()
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .flex_1()
                                .min_w(px(0.0))
                                .gap(px(4.0))
                                .child(
                                    div()
                                        .flex()
                                        .items_baseline()
                                        .gap(px(8.0))
                                        .min_w(px(0.0))
                                        .child(
                                            div().flex_none().font_weight(FontWeight::SEMIBOLD).child(a.name.clone()),
                                        )
                                        .child(
                                            one_line(a.base.clone())
                                                .font_family(MONO)
                                                .text_size(px(11.5))
                                                .text_color(pal.secondary),
                                        ),
                                )
                                .child(
                                    div()
                                        .flex()
                                        .flex_wrap()
                                        .items_center()
                                        .gap(px(8.0))
                                        .child(tag(a.secret.clone(), pal))
                                        .child(fine(
                                            format!("held {} after you approve", format::span(a.lease_secs.into())),
                                            pal,
                                        ))
                                        .child(fine(format!("http://{listen}/api/{}", a.name), pal).font_family(MONO)),
                                ),
                        )
                        .child(match state {
                            Some(s) => div()
                                .flex_none()
                                .flex()
                                .items_center()
                                .gap(px(6.0))
                                .text_size(px(12.0))
                                .text_color(pal.success)
                                .child(div().size(px(6.0)).rounded_full().bg(pal.success))
                                .child(s),
                            None => fine("Asked on next use", pal).flex_none(),
                        }),
                );
            }
            list
        };
        page = page.child(group(
            "API keys",
            Some("Programs call these through Reins; the key is added on the way and never shown to them."),
            apis,
            pal,
        ));

        // Run profiles.
        let profiles = if config.run.profiles.is_empty() {
            card(pal).child(empty(
                format!(
                    "No profiles yet. Add [run.profiles.<name>] with env = {{ VAR = \"vault:Item/field\" }} to {}.",
                    d.config_file
                ),
                pal,
            ))
        } else {
            let mut list = card(pal);
            for (i, (name, p)) in config.run.profiles.iter().enumerate() {
                let mut block = div()
                    .flex()
                    .flex_col()
                    .gap(px(6.0))
                    .px(px(16.0))
                    .py(px(12.0))
                    .when(i > 0, |b| b.border_t_1().border_color(pal.hairline))
                    .child(
                        div()
                            .flex()
                            .items_baseline()
                            .gap(px(10.0))
                            .child(mono(name.clone(), pal).font_weight(FontWeight::MEDIUM).text_size(px(13.0)))
                            .when_some(p.purpose.clone(), |r, purpose| {
                                r.child(caption(purpose, pal).min_w(px(0.0)).truncate())
                            }),
                    );
                for (var, reference) in &p.env {
                    block = block.child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(10.0))
                            .pl(px(2.0))
                            .child(mono(var.clone(), pal).text_color(pal.secondary).w(px(220.0)).flex_none().truncate())
                            .child(fine("←", pal))
                            .child(tag(reference.clone(), pal)),
                    );
                }
                list = list.child(block);
            }
            list
        };
        page = page.child(group(
            "reins run profiles",
            Some("Variables a program gets from your vault for one run, after you approve on your phone."),
            profiles,
            pal,
        ));

        // SSH agent.
        let on = config.ssh.enabled;
        let mut ssh = card(pal).child(
            row(pal, true)
                .id("ssh-toggle")
                .cursor_pointer()
                .child(labelled(
                    "Sign SSH logins with keys from your phone",
                    Some(
                        "ssh asks your phone each time it signs in. Changing it restarts the background service."
                            .to_owned(),
                    ),
                    pal,
                ))
                .child(toggle(on, pal))
                .on_click(Self::on_model(cx, move |m, cx| m.change(Setting::Ssh(!on), cx))),
        );
        if on {
            ssh = ssh.child(
                row(pal, false)
                    .child(div().flex_none().text_color(pal.secondary).child("Socket"))
                    .child(div().flex_1())
                    .child(
                        one_line(
                            overview
                                .ssh_socket
                                .clone()
                                .unwrap_or_else(|| "starts with the background service".to_owned()),
                        )
                        .text_ellipsis_start()
                        .font_family(MONO)
                        .text_size(px(12.0)),
                    ),
            );
            if overview.ssh_keys.is_empty() {
                ssh = ssh.child(row(pal, false).child(fine("Keys are listed after the first ssh use.", pal)));
            } else {
                for k in &overview.ssh_keys {
                    ssh = ssh.child(
                        row(pal, false)
                            .child(div().flex_none().font_weight(FontWeight::MEDIUM).child(k.name.clone()))
                            .child(div().flex_1())
                            .child(
                                one_line(k.fingerprint.clone())
                                    .font_family(MONO)
                                    .text_size(px(11.5))
                                    .text_color(pal.secondary),
                            ),
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
        assert!(lease_state(&apis, "openai", 1_000).unwrap().starts_with("Key held until "));
        assert_eq!(lease_state(&apis, "openai", 2_001), None);
        assert_eq!(lease_state(&apis, "other", 1_000), None);
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
