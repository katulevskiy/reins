#![allow(clippy::format_collect, reason = "pages are built from rows, as in console_pages")]

//! The console's O5 page (HTML): audit streaming. Same look and rules as [`super::console_pages`]: every value escaped,
//! no script, and never a token, key or secret shown back.

use reins_proto::org::SinkKind;

use super::console_pages::{Frame, hidden_csrf, icon, page, svg};
use crate::api::reins::pages::escape_html as e;

/// One sink as the streaming page shows it.
pub struct SinkRow {
    pub kind: SinkKind,
    /// Set up (a configuration is stored).
    pub installed: bool,
    pub enabled: bool,
    pub failing: bool,
    /// What may be shown of the configuration ([`super::sinks::SinkConfig::public`]).
    pub public: Vec<(&'static str, String)>,
    pub sent_seq: i64,
    pub last_ok: Option<String>,
    pub last_error: Option<String>,
}

/// The form fields of a sink's setup: (name, label, secret).
fn fields(kind: SinkKind) -> &'static [(&'static str, &'static str, bool)] {
    match kind {
        SinkKind::Splunk => &[("url", "Collector URL", false), ("token", "HEC token", true), ("index", "Index", false)],
        SinkKind::Datadog => &[("site", "Site", false), ("api_key", "API key", true)],
        SinkKind::Elastic => &[("url", "URL", false), ("api_key", "API key", true), ("index", "Index", false)],
        SinkKind::Sentinel => &[
            ("tenant_id", "Tenant", false),
            ("client_id", "App (client) id", false),
            ("client_secret", "Client secret", true),
            ("endpoint", "Ingestion endpoint", false),
            ("dcr_id", "Rule id (dcr-…)", false),
            ("stream", "Stream (Custom-…)", false),
        ],
        SinkKind::S3 => &[
            ("endpoint", "Endpoint (optional)", false),
            ("region", "Region", false),
            ("bucket", "Bucket", false),
            ("prefix", "Prefix", false),
            ("access_key_id", "Access key id", false),
            ("secret_access_key", "Secret key", true),
            ("lock_days", "Lock days", false),
        ],
    }
}

/// The names a sink's setup form posts, so the route reads only those.
#[must_use]
pub fn field_names(kind: SinkKind) -> Vec<&'static str> {
    fields(kind).iter().map(|(n, _, _)| *n).collect()
}

pub fn streaming(frame: &Frame<'_>, rows: &[SinkRow], can_manage: bool) -> String {
    let org = frame.org.map(|(id, _)| format!("{}{}", e(frame.base), e(id))).unwrap_or_default();
    let csrf = hidden_csrf(frame.csrf);
    let sections: String = rows
        .iter()
        .map(|r| {
            let k = r.kind.as_str();
            let state = match (r.installed, r.enabled, r.failing) {
                (false, ..) => "<span class=\"pill\">Not set up</span>".to_owned(),
                (true, false, _) => "<span class=\"pill\">Off</span>".to_owned(),
                (true, true, false) => format!("<span class=\"pill ok\">{}On</span>", svg(icon::CHECK)),
                (true, true, true) => format!("<span class=\"pill bad\">{}Failing</span>", svg(icon::WARNING)),
            };
            let shown: String = r
                .public
                .iter()
                .filter(|(_, v)| !v.is_empty())
                .map(|(label, v)| {
                    format!("<div class=\"meta\">{} <span class=\"mono\">{}</span></div>", e(label), e(v))
                })
                .collect();
            let delivery = if r.installed {
                format!(
                    "<p class=\"meta\">Sent to event {}{}{}</p>",
                    r.sent_seq,
                    r.last_ok.as_deref().map(|t| format!(" · {}", e(t))).unwrap_or_default(),
                    r.last_error.as_deref().map(|err| format!(" · {}", e(err))).unwrap_or_default()
                )
            } else {
                String::new()
            };
            let toggle = if can_manage && r.installed {
                let (op, label, class) = if r.enabled {
                    ("off", "Turn off", "danger")
                } else {
                    ("on", "Turn on", "quiet")
                };
                format!(
                    "<form method=\"post\" action=\"{org}/audit/streaming/{k}/{op}\">{csrf}\
<button class=\"{class}\" type=\"submit\">{label}</button></form>"
                )
            } else {
                String::new()
            };
            let setup = if can_manage {
                let inputs: String = fields(r.kind)
                    .iter()
                    .map(|(name, label, secret)| {
                        let kind = if *secret {
                            "password"
                        } else {
                            "text"
                        };
                        format!(
                            "<input type=\"{kind}\" name=\"{name}\" placeholder=\"{}\" autocomplete=\"off\">",
                            e(label)
                        )
                    })
                    .collect();
                format!(
                    "<details><summary class=\"meta\">Set up</summary><form method=\"post\" \
action=\"{org}/audit/streaming/{k}\">{csrf}{inputs}<button class=\"quiet\" type=\"submit\">Save</button></form>\
</details>"
                )
            } else {
                String::new()
            };
            format!(
                "<section><div class=\"row\"><h2>{}</h2>{state}</div>{shown}{delivery}<div class=\"row\">{toggle}</div>\
{setup}</section>",
                e(r.kind.label())
            )
        })
        .collect();
    let body = format!(
        "<div class=\"row\"><h1>{}Streaming</h1><a href=\"{org}/audit\">Audit</a></div>\
<p class=\"meta\">Turning on or off takes two admins' phones. Turn off to change.</p>{sections}",
        svg(icon::LIST)
    );
    page("Streaming", Some(frame), &body, frame.base)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::reins::org::{console_pages::Tab, sinks::SinkConfig};

    fn frame() -> Frame<'static> {
        Frame {
            base: "/reins/org/",
            email: "kim@acme.com",
            csrf: "c",
            org: Some(("o1", "Acme")),
            tab: Tab::Audit,
            flash: None,
        }
    }

    #[test]
    fn the_streaming_page_never_shows_secrets() {
        let cfg = SinkConfig::parse(
            SinkKind::Sentinel,
            &serde_json::json!({"tenant_id": "t1", "client_id": "c1", "client_secret": "very-secret",
                "endpoint": "https://dce.example.com", "dcr_id": "dcr-1", "stream": "Custom-Reins"}),
        )
        .unwrap();
        let rows = vec![SinkRow {
            kind: SinkKind::Sentinel,
            installed: true,
            enabled: true,
            failing: true,
            public: cfg.public(),
            sent_seq: 41,
            last_ok: None,
            last_error: Some("dce.example.com answered 403".into()),
        }];
        let html = streaming(&frame(), &rows, true);
        assert!(!html.contains("very-secret"), "{html}");
        assert!(html.contains("https://dce.example.com") && html.contains("Failing") && html.contains("event 41"));
        assert!(html.contains("type=\"password\" name=\"client_secret\""));
        assert!(html.contains("/reins/org/o1/audit/streaming/sentinel/off"));
        // A viewer without the capability sees no forms.
        assert!(!streaming(&frame(), &rows, false).contains("audit/streaming/sentinel"));
    }
}
