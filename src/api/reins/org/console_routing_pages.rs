//! The O3 console pages: routing, channels, the SLA report and templates. The console's look; every value escaped; no
//! script.

#![allow(clippy::format_collect, clippy::format_push_string, reason = "small server-rendered pages")]

use reins_proto::org::{ChannelKind, OnCallProvider};

use super::{
    console_pages::{Frame, hidden_csrf, page},
    routing::sla::GroupSla,
    templates::Template,
};
use crate::{
    api::reins::pages::escape_html as e,
    db::models::reins_org_routing::{ChannelSummary, OnCallSummary},
};

/// What the routing page shows.
pub struct RoutingView {
    /// (group id, name, backup's name, window in minutes).
    pub groups: Vec<(String, String, Option<String>, Option<i64>)>,
    pub group_choices: Vec<(String, String)>,
    /// (member email, manager email, source).
    pub managers: Vec<(String, String, String)>,
    /// (membership id, email) of active members.
    pub members: Vec<(String, String)>,
    pub team_tier: bool,
    /// (app id, installation id, key posted).
    pub reader: Option<(i64, i64, bool)>,
    pub linked_logins: usize,
    pub oncall: Vec<OnCallSummary>,
}

fn org_url(f: &Frame<'_>) -> String {
    f.org.map(|(id, _)| format!("{}{}", e(f.base), e(id))).unwrap_or_default()
}

fn sub_nav(f: &Frame<'_>) -> String {
    let o = org_url(f);
    format!(
        "<p><a href=\"{o}/routing\">Routing</a> · <a href=\"{o}/channels\">Channels</a> · \
<a href=\"{o}/reports/sla\">SLA</a> · <a href=\"{o}/templates\">Templates</a></p>"
    )
}

fn options(items: &[(String, String)], blank: &str) -> String {
    let mut out = format!("<option value=\"\">{}</option>", e(blank));
    for (v, label) in items {
        out.push_str(&format!("<option value=\"{}\">{}</option>", e(v), e(label)));
    }
    out
}

#[must_use]
pub fn routing(f: &Frame<'_>, v: &RoutingView) -> String {
    let o = org_url(f);
    let csrf = hidden_csrf(f.csrf);
    let groups: String = v
        .groups
        .iter()
        .map(|(id, name, backup, window)| {
            format!(
                "<tr><td>{}</td><td>{}</td><td>{}</td><td><form method=\"post\" action=\"{o}/routing/group\">{csrf}\
<input type=\"hidden\" name=\"group\" value=\"{}\"><select name=\"backup\">{}</select>\
<input name=\"window_mins\" placeholder=\"min\" size=\"4\"><button class=\"quiet\" type=\"submit\">Set</button></form></td></tr>",
                e(name),
                backup.as_deref().map_or_else(|| "—".to_owned(), e),
                window.map_or_else(|| "—".to_owned(), |w| format!("{w} min")),
                e(id),
                options(&v.group_choices, "No backup"),
            )
        })
        .collect();
    let managers: String = v
        .managers
        .iter()
        .map(|(m, mgr, src)| format!("<tr><td>{}</td><td>{}</td><td class=\"meta\">{}</td></tr>", e(m), e(mgr), e(src)))
        .collect();
    let set_manager = if v.team_tier {
        format!(
            "<form method=\"post\" action=\"{o}/routing/manager\">{csrf}<select name=\"member\">{}</select>\
<select name=\"manager\">{}</select><button class=\"quiet\" type=\"submit\">Set</button></form>",
            options(&v.members, "Member"),
            options(&v.members, "No manager"),
        )
    } else {
        "<p class=\"meta\">From your directory.</p>".to_owned()
    };
    let reader = v.reader.map_or_else(
        || "<p class=\"meta\">Not set up.</p>".to_owned(),
        |(app, inst, key)| {
            format!(
                "<p class=\"mono\">App {app} · installation {inst}</p><p><span class=\"pill{}\">{}</span>\
<span class=\"pill\">{} linked</span></p>",
                if key {
                    " ok"
                } else {
                    ""
                },
                if key {
                    "Key set"
                } else {
                    "No key"
                },
                v.linked_logins
            )
        },
    );
    let providers: Vec<(String, String)> =
        OnCallProvider::ALL.iter().map(|p| (p.as_str().to_owned(), p.label().to_owned())).collect();
    let oncall: String = v
        .oncall
        .iter()
        .map(|s| {
            format!(
                "<tr><td class=\"mono\">{a}</td><td>{p}</td><td class=\"mono\">{sch}</td><td>{k}</td><td>\
<form method=\"post\" action=\"{o}/routing/oncall/key\">{csrf}<input type=\"hidden\" name=\"alias\" value=\"{a}\">\
<input type=\"password\" name=\"key\" placeholder=\"API key\" autocomplete=\"off\"><button class=\"quiet\" \
type=\"submit\">Save</button></form></td></tr>",
                a = e(&s.alias),
                p = e(&OnCallProvider::parse(&s.provider).map_or_else(|| s.provider.clone(), |p| p.label().to_owned())),
                sch = e(&s.schedule_id),
                k = if s.has_key {
                    "<span class=\"pill ok\">Key set</span>"
                } else {
                    "<span class=\"pill\">No key</span>"
                },
            )
        })
        .collect();
    let body = format!(
        "{nav}<section><h2>Groups</h2><table><tr><th>Group</th><th>Backup</th><th>Window</th><th></th></tr>{groups}</table>\
</section><section><h2>Managers</h2><table>{managers}</table>{set_manager}</section>\
<section><h2>GitHub reader</h2>{reader}<form method=\"post\" action=\"{o}/routing/reader\">{csrf}\
<input name=\"app_id\" placeholder=\"App id\" size=\"8\"><input name=\"installation_id\" placeholder=\"Installation\" \
size=\"10\"><button class=\"quiet\" type=\"submit\">Set</button></form>\
<form method=\"post\" action=\"{o}/routing/reader/key\">{csrf}<input type=\"password\" name=\"pem\" \
placeholder=\"Private key (PEM)\" autocomplete=\"off\"><input type=\"password\" name=\"webhook_secret\" \
placeholder=\"Webhook secret\" autocomplete=\"off\"><button class=\"quiet\" type=\"submit\">Save key</button></form></section>\
<section><h2>On-call</h2><table>{oncall}</table><form method=\"post\" action=\"{o}/routing/oncall\">{csrf}\
<input name=\"alias\" placeholder=\"payments\" size=\"10\"><select name=\"provider\">{prov}</select>\
<input name=\"schedule_id\" placeholder=\"Schedule id\" size=\"12\"><button class=\"quiet\" type=\"submit\">Add</button>\
</form></section>",
        nav = sub_nav(f),
        prov = options(&providers, "Provider"),
    );
    page("Routing", Some(f), &body, f.base)
}

#[must_use]
pub fn channels(f: &Frame<'_>, rows: &[ChannelSummary], server: &str) -> String {
    let o = org_url(f);
    let csrf = hidden_csrf(f.csrf);
    let mut sections = String::new();
    for kind in ChannelKind::ALL {
        let row = rows.iter().find(|r| r.kind == kind.as_str());
        let (installed, on) = (row.is_some_and(|r| r.installed), row.is_some_and(|r| r.enabled));
        let state = match (installed, on) {
            (true, true) => "<span class=\"pill ok\">On</span>",
            (true, false) => "<span class=\"pill\">Off</span>",
            _ => "<span class=\"pill\">Not installed</span>",
        };
        let fields = match kind {
            ChannelKind::Slack => {
                "<input type=\"password\" name=\"bot_token\" placeholder=\"Bot token (xoxb-…)\" autocomplete=\"off\">\
<input type=\"password\" name=\"signing_secret\" placeholder=\"Signing secret\" autocomplete=\"off\">"
            }
            ChannelKind::Teams => {
                "<input name=\"app_id\" placeholder=\"Bot app id\"><input type=\"password\" name=\"app_secret\" \
placeholder=\"Bot secret\" autocomplete=\"off\"><input name=\"tenant_id\" placeholder=\"Tenant id\">\
<input name=\"teams_app_id\" placeholder=\"Teams app id\">"
            }
        };
        let endpoint = match kind {
            ChannelKind::Slack => format!("{}/reins/org/slack/events", e(server)),
            ChannelKind::Teams => format!("{}/reins/org/teams/messages", e(server)),
        };
        let toggle = if installed {
            format!(
                "<form method=\"post\" action=\"{o}/channels/toggle\">{csrf}<input type=\"hidden\" name=\"kind\" value=\"{k}\">\
<input type=\"hidden\" name=\"enabled\" value=\"{v}\"><button class=\"{c}\" type=\"submit\">{t}</button></form>",
                k = kind.as_str(),
                v = !on,
                c = if on {
                    "danger"
                } else {
                    "quiet"
                },
                t = if on {
                    "Turn off"
                } else {
                    "Turn on"
                },
            )
        } else {
            String::new()
        };
        sections.push_str(&format!(
            "<section><div class=\"row\"><h2>{label}</h2>{state}</div><p class=\"meta mono\">{endpoint}</p>\
<form method=\"post\" action=\"{o}/channels/install\">{csrf}<input type=\"hidden\" name=\"kind\" value=\"{k}\">{fields}\
<button class=\"quiet\" type=\"submit\">Install</button></form>{toggle}</section>",
            label = kind.label(),
            k = kind.as_str(),
        ));
    }
    page(
        "Channels",
        Some(f),
        &format!("{}<p class=\"meta\">L0–L2 only. L3 needs the app.</p>{sections}", sub_nav(f)),
        f.base,
    )
}

fn dur(secs: u64) -> String {
    match secs {
        0 => "—".to_owned(),
        s if s < 60 => format!("{s} s"),
        s if s < 3600 => format!("{} min", s / 60),
        s => format!("{:.1} h", f64::from(u32::try_from(s).unwrap_or(u32::MAX)) / 3600.0),
    }
}

#[must_use]
pub fn sla(f: &Frame<'_>, days: i64, overall: &GroupSla, groups: &[GroupSla]) -> String {
    let o = org_url(f);
    let row = |g: &GroupSla| {
        format!(
            "<tr><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>",
            e(&g.group),
            g.decided,
            dur(g.median_secs),
            dur(g.p90_secs),
            g.expired,
            g.escalated
        )
    };
    let rows: String = std::iter::once(overall).chain(groups).map(row).collect();
    let other = if days == 7 {
        30
    } else {
        7
    };
    page(
        "Time to decision",
        Some(f),
        &format!(
            "{}<section><div class=\"row\"><h1>Time to decision</h1><a href=\"{o}/reports/sla?days={other}\">{other} days</a>\
</div><p class=\"meta\">Last {days} days</p><table><tr><th>Who</th><th>Decided</th><th>Median</th><th>p90</th>\
<th>Expired</th><th>Escalated</th></tr>{rows}</table></section>",
            sub_nav(f)
        ),
        f.base,
    )
}

#[must_use]
pub fn templates(f: &Frame<'_>, chosen: Template, params: &str, preview: Option<&str>) -> String {
    let o = org_url(f);
    let opts: String = Template::ALL
        .iter()
        .map(|t| {
            format!(
                "<option value=\"{}\"{}>{}</option>",
                t.as_str(),
                if *t == chosen {
                    " selected"
                } else {
                    ""
                },
                t.label()
            )
        })
        .collect();
    let preview = preview
        .map(|p| format!("<section><h2>Preview</h2><pre class=\"record mono\">{}</pre></section>", e(p)))
        .unwrap_or_default();
    page(
        "Templates",
        Some(f),
        &format!(
            "{}<section><h1>Templates</h1><form method=\"post\" action=\"{o}/templates\" style=\"display:flex;\
flex-direction:column;align-items:stretch\">{}<select name=\"template\">{opts}</select>\
<textarea name=\"params\" rows=\"14\" class=\"mono\">{}</textarea><div class=\"row\">\
<button class=\"quiet\" name=\"op\" value=\"preview\" type=\"submit\">Preview</button>\
<button name=\"op\" value=\"draft\" type=\"submit\">Create draft</button></div></form></section>{preview}",
            sub_nav(f),
            hidden_csrf(f.csrf),
            e(params)
        ),
        f.base,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::reins::org::console_pages::Tab;

    fn frame() -> Frame<'static> {
        Frame {
            base: "/reins/org/",
            email: "dana@acme.com",
            csrf: "tok",
            org: Some(("o1", "Acme")),
            tab: Tab::Routing,
            flash: None,
        }
    }

    #[test]
    fn values_are_escaped() {
        let v = RoutingView {
            groups: vec![("g1".into(), "<b>Ops</b>".into(), Some("\"Back\"".into()), Some(10))],
            group_choices: vec![("g1".into(), "<b>Ops</b>".into())],
            managers: vec![("<x>@acme.com".into(), "m@acme.com".into(), "directory".into())],
            members: vec![("m1".into(), "<i>lee</i>".into())],
            team_tier: true,
            reader: Some((1, 2, true)),
            linked_logins: 3,
            oncall: vec![OnCallSummary {
                alias: "<script>".into(),
                provider: "pagerduty".into(),
                schedule_id: "P\"1".into(),
                has_key: true,
            }],
        };
        let html = routing(&frame(), &v);
        for raw in ["<b>Ops", "<x>@", "<i>lee", "<script>", "P\"1"] {
            assert!(!html.contains(raw), "{raw}");
        }
        assert!(html.contains("&lt;b&gt;Ops") && !html.contains("<script"));
        let html = templates(&frame(), Template::Finance, "{\"a\": \"</textarea><script>\"}", Some("<pre>"));
        assert!(!html.contains("</textarea><script>") && !html.contains("<script"));
    }

    #[test]
    fn the_sla_table_shows_medians() {
        let overall = GroupSla {
            group: "All".into(),
            decided: 3,
            median_secs: 120,
            p90_secs: 5400,
            expired: 1,
            escalated: 2,
        };
        let sec = GroupSla {
            group: "Security".into(),
            ..overall.clone()
        };
        let html = sla(&frame(), 7, &overall, &[sec]);
        assert!(html.contains("2 min") && html.contains("1.5 h") && html.contains("Security"), "{html}");
        assert!(html.contains("days=30"));
    }
}
