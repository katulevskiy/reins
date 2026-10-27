#![allow(clippy::format_collect, reason = "pages are built from rows, as in console_pages")]

//! The console's O2 pages (HTML): the org policy, recent decisions with the 05 §10 basics, the audit chain and its
//! export, executors and retention. Same look and rules as [`super::console_pages`]: every value escaped, no script.

use super::console_pages::{Frame, hidden_csrf, icon, page, retention_text, svg};
use crate::api::reins::pages::escape_html as e;

/// The active policy, as the console shows it.
pub struct ActivePolicy {
    pub revision: i64,
    pub hash: String,
    pub activated_at: String,
}

pub struct DraftRow {
    pub revision: i64,
    pub hash: String,
    pub state: String,
}

pub struct PolicyView<'a> {
    pub active: Option<ActivePolicy>,
    pub drafts: Vec<DraftRow>,
    pub draft_text: &'a str,
    pub retention_days: u32,
    pub can_author: bool,
    pub can_activate: bool,
}

fn short(hash: &str) -> &str {
    hash.get(..12).unwrap_or(hash)
}

pub fn policy(frame: &Frame<'_>, v: &PolicyView<'_>) -> String {
    let org = frame.org.map(|(id, _)| format!("{}{}", e(frame.base), e(id))).unwrap_or_default();
    let csrf = hidden_csrf(frame.csrf);
    let active = v.active.as_ref().map_or_else(
        || "<p class=\"meta\">No active policy. Agents can't act until one is.</p>".to_owned(),
        |a| {
            format!(
                "<p><span class=\"pill ok\">{}Revision {}</span> <span class=\"mono\">{}</span> \
<span class=\"meta\">since {}</span></p>",
                svg(icon::CHECK),
                a.revision,
                e(short(&a.hash)),
                e(&a.activated_at)
            )
        },
    );
    let drafts: String = v
        .drafts
        .iter()
        .map(|d| {
            let activate = if v.can_activate && d.state == "draft" {
                format!(
                    "<form method=\"post\" action=\"{org}/policy/activate\">{csrf}\
<input type=\"hidden\" name=\"revision\" value=\"{}\"><input type=\"hidden\" name=\"hash\" value=\"{}\">\
<button type=\"submit\">Activate</button></form>",
                    d.revision,
                    e(&d.hash)
                )
            } else {
                String::new()
            };
            format!(
                "<tr><td class=\"mono\">{}</td><td class=\"mono\">{}</td><td>{}</td><td>{activate}</td></tr>",
                d.revision,
                e(short(&d.hash)),
                e(&d.state)
            )
        })
        .collect();
    let author = if v.can_author {
        format!(
            "<section><h2>Draft</h2><form method=\"post\" action=\"{org}/policy/draft\" style=\"display:block\">{csrf}\
<textarea name=\"body\" rows=\"18\" spellcheck=\"false\" class=\"mono\" style=\"width:100%;border-radius:9px;\
padding:10px;border:0;box-shadow:0 0 0 1px var(--rule);background:var(--paper)\">{}</textarea>\
<p><button type=\"submit\">Save draft</button></p></form></section>",
            e(v.draft_text)
        )
    } else {
        String::new()
    };
    let retention = if v.can_activate {
        let opt = |d: u32| {
            let sel = if d == v.retention_days {
                " selected"
            } else {
                ""
            };
            format!("<option value=\"{d}\"{sel}>{}</option>", retention_text(d))
        };
        format!(
            "<section><h2>Request bodies</h2><form method=\"post\" action=\"{org}/retention\">{csrf}\
<select name=\"days\">{}{}{}</select><button class=\"quiet\" type=\"submit\">Keep</button></form></section>",
            opt(0),
            opt(30),
            opt(365)
        )
    } else {
        format!("<section><h2>Request bodies</h2><p>{}</p></section>", retention_text(v.retention_days))
    };
    let body = format!(
        "<div class=\"row\"><h1>{}Policy</h1></div><section><h2>Active</h2>{active}</section>\
<section><h2>Revisions</h2><table><tr><th>#</th><th>Hash</th><th>State</th><th></th></tr>{drafts}</table>\
<p class=\"meta\">Activation needs two Security admins, each on their phone.</p></section>{author}{retention}",
        svg(icon::SHIELD)
    );
    page("Policy", Some(frame), &body, frame.base)
}

pub struct RequestRow {
    pub at: String,
    pub member: String,
    pub agent: String,
    pub tool: String,
    pub line: String,
    pub state: String,
}

pub struct Counts {
    pub total: i64,
    pub auto: i64,
    pub reviewed: i64,
    pub denied: i64,
    pub expired: i64,
    pub median: Option<i64>,
}

fn duration(secs: i64) -> String {
    match secs {
        s if s < 120 => format!("{s} s"),
        s if s < 7200 => format!("{} min", s / 60),
        s => format!("{} h", s / 3600),
    }
}

pub fn requests(frame: &Frame<'_>, rows: &[RequestRow], c: &Counts) -> String {
    let ask_rate = if c.total > 0 {
        let tenths = c.reviewed * 1000 / c.total;
        format!("{}.{}%", tenths / 10, tenths % 10)
    } else {
        "—".to_owned()
    };
    let stat = |label: &str, value: String| format!("<span class=\"pill\">{} <b>{}</b></span>", e(label), e(&value));
    let stats = [
        stat("Requests", c.total.to_string()),
        stat("Auto", c.auto.to_string()),
        stat("Asked", ask_rate),
        stat("Denied", c.denied.to_string()),
        stat("Expired", c.expired.to_string()),
        stat("Median decision", c.median.map_or_else(|| "—".to_owned(), duration)),
    ]
    .concat();
    let list: String = rows
        .iter()
        .map(|r| {
            format!(
                "<tr><td class=\"meta\">{}</td><td>{}</td><td class=\"meta\">{}</td><td class=\"mono\">{}</td>\
<td>{}</td><td>{}</td></tr>",
                e(&r.at),
                e(&r.member),
                e(&r.agent),
                e(&r.tool),
                e(&r.line),
                e(&r.state)
            )
        })
        .collect();
    let body = format!(
        "<div class=\"row\"><h1>{}Requests</h1></div><section><h2>Last 7 days</h2><p>{stats}</p></section>\
<section><table><tr><th>When</th><th>Member</th><th>Agent</th><th>Tool</th><th>Decision</th><th>State</th></tr>{list}\
</table></section>",
        svg(icon::LIST)
    );
    page("Requests", Some(frame), &body, frame.base)
}

pub struct AuditView {
    pub intact: bool,
    pub head: Option<i64>,
    pub checkpoint: Option<(i64, String)>,
    pub decision_key: (String, String),
    pub issuer_key: (String, String),
    pub kms: String,
}

pub fn audit(frame: &Frame<'_>, v: &AuditView) -> String {
    let org = frame.org.map(|(id, _)| format!("{}{}", e(frame.base), e(id))).unwrap_or_default();
    let chain = if v.intact {
        format!("<span class=\"pill ok\">{}Chain intact</span>", svg(icon::CHECK))
    } else {
        format!("<span class=\"pill bad\">{}Chain broken</span>", svg(icon::WARNING))
    };
    let checkpoint =
        v.checkpoint.as_ref().map_or_else(|| "none yet".to_owned(), |(seq, at)| format!("event {seq} · {}", e(at)));
    let key = |k: &(String, String)| format!("<div class=\"record mono\">{}={}</div>", e(&k.0), e(&k.1));
    let body = format!(
        "<div class=\"row\"><h1>{}Audit</h1>{chain}</div><section><h2>Chain</h2><p>{} events · last checkpoint \
{checkpoint}</p><p><a class=\"button\" href=\"{org}/audit/export\">Export</a></p></section>\
<section><h2>Verify</h2><p class=\"meta\">reins-audit verify export.jsonl --key &lt;decision key&gt;</p>\
<h2>Decision service key</h2>{}<h2>Issuer key</h2>{}<p class=\"meta\">Keys sealed by {}</p></section>",
        svg(icon::LIST),
        v.head.unwrap_or(0),
        key(&v.decision_key),
        key(&v.issuer_key),
        e(&v.kms),
    );
    page("Audit", Some(frame), &body, frame.base)
}

pub struct ExecutorRow {
    pub id: String,
    pub mode: String,
    pub last_seen: String,
    pub removed: bool,
}

pub fn executors(
    frame: &Frame<'_>,
    rows: &[ExecutorRow],
    can_manage: bool,
    pins: &(String, String, String, String),
) -> String {
    let org = frame.org.map(|(id, _)| format!("{}{}", e(frame.base), e(id))).unwrap_or_default();
    let csrf = hidden_csrf(frame.csrf);
    let list: String = rows
        .iter()
        .map(|r| {
            let remove = if can_manage && !r.removed {
                format!(
                    "<form method=\"post\" action=\"{org}/executors/remove\">{csrf}\
<input type=\"hidden\" name=\"executor_id\" value=\"{}\"><button class=\"danger\" type=\"submit\">Remove</button></form>",
                    e(&r.id)
                )
            } else {
                String::new()
            };
            format!(
                "<tr><td class=\"mono\">{}</td><td>{}</td><td class=\"meta\">{}</td><td>{}</td></tr>",
                e(&r.id),
                e(&r.mode),
                if r.removed { "removed".to_owned() } else { e(&r.last_seen) },
                remove
            )
        })
        .collect();
    let add = if can_manage {
        format!(
            "<section><h2>Register</h2><form method=\"post\" action=\"{org}/executors\">{csrf}\
<input name=\"executor_id\" placeholder=\"name\" required><select name=\"mode\"><option value=\"cloud\">Reins Cloud\
</option><option value=\"dedicated\">Dedicated</option><option value=\"self_hosted\">Self-hosted</option></select>\
<input name=\"public_key\" placeholder=\"public key\" required class=\"mono\"><button type=\"submit\">Register</button>\
</form></section>"
        )
    } else {
        String::new()
    };
    let body = format!(
        "<div class=\"row\"><h1>{}Executors</h1></div><section><table><tr><th>Name</th><th>Mode</th><th>Last seen</th>\
<th></th></tr>{list}</table></section>{add}<section><h2>Pin in the executor</h2>\
<div class=\"record mono\">issuer {}={}</div><div class=\"record mono\">decision {}={}</div></section>",
        svg(icon::SHIELD),
        e(&pins.0),
        e(&pins.1),
        e(&pins.2),
        e(&pins.3),
    );
    page("Executors", Some(frame), &body, frame.base)
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
            tab: Tab::Policy,
            flash: None,
        }
    }

    #[test]
    fn values_are_escaped() {
        let rows = [RequestRow {
            at: "<b>".into(),
            member: "<script>".into(),
            agent: "a\"b".into(),
            tool: "github_create_issue".into(),
            line: "Ask · Work · <x>".into(),
            state: "done".into(),
        }];
        let c = Counts {
            total: 2,
            auto: 1,
            reviewed: 1,
            denied: 0,
            expired: 0,
            median: Some(90),
        };
        let html = requests(&frame(), &rows, &c);
        assert!(!html.contains("<script>") && !html.contains("<x>"));
        assert!(html.contains("50.0%") && html.contains("90 s"));
        let v = PolicyView {
            active: None,
            drafts: vec![],
            draft_text: "</textarea><script>",
            retention_days: 30,
            can_author: true,
            can_activate: true,
        };
        assert!(!policy(&frame(), &v).contains("</textarea><script>"));
    }

    #[test]
    fn the_export_link_and_keys_show_on_the_audit_page() {
        let v = AuditView {
            intact: true,
            head: Some(12),
            checkpoint: Some((10, "today".into())),
            decision_key: ("dec-1".into(), "KEY".into()),
            issuer_key: ("iss-1".into(), "IKEY".into()),
            kms: "local:ab".into(),
        };
        let html = audit(&frame(), &v);
        assert!(html.contains("/reins/org/o1/audit/export"));
        assert!(html.contains("dec-1=KEY") && html.contains("Chain intact"));
    }

    #[test]
    fn drafts_offer_activation_only_to_activators() {
        let mut v = PolicyView {
            active: Some(ActivePolicy {
                revision: 1,
                hash: "abcdef0123456789".into(),
                activated_at: "today".into(),
            }),
            drafts: vec![DraftRow {
                revision: 2,
                hash: "ff".into(),
                state: "draft".into(),
            }],
            draft_text: "{}",
            retention_days: 365,
            can_author: true,
            can_activate: true,
        };
        assert!(policy(&frame(), &v).contains("policy/activate"));
        v.can_activate = false;
        assert!(!policy(&frame(), &v).contains("policy/activate"));
        assert!(policy(&frame(), &v).contains("1 year"));
    }
}
