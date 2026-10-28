//! The console's O5 routes: evidence reports (SOC 2, ISO 27001) built from a verified export, the `reins-audit --keys`
//! file, stored checkpoints with their RFC 3161 timestamps, and audit streaming (set up by a Security admin, turned on
//! or off through a two-person device-signed change). Each page re-checks the viewer's capability; each POST carries
//! the session's CSRF token.

use std::collections::HashMap;

use reins_audit::{
    TrustedKey,
    evidence::{self, Framework, Period},
};
use reins_proto::org::{AdminAction, Capability, SinkKind};
use rocket::{
    Request, Response, Route, State,
    form::Form,
    http::{ContentType, CookieJar},
    response::{self, Redirect, Responder},
};
use serde_json::{Map, Value};

use super::{
    audit, can,
    console::{self, Page, admin_of, check_csrf, flash, not_found, org_name, run, to_home, to_org},
    console_audit_pages::{self as pages, SinkRow},
    console_managed::signed_change,
    console_pages::{Frame, Tab},
    kms, sinks,
};
use crate::{
    api::reins::now_unix,
    db::{
        DbConn, DbPool,
        models::{reins_org_compliance, reins_org_exec},
    },
};

pub fn routes() -> Vec<Route> {
    routes![report, keys_file, checkpoint_file, streaming, install, toggle]
}

const AUDIT_VIEW: &[Capability] = &[Capability::ViewAdminEvents];
const STREAM_VIEW: &[Capability] = &[Capability::ActivatePolicy, Capability::ViewAdminEvents];
const STREAM_MANAGE: &[Capability] = &[Capability::ActivatePolicy];

fn when(t: i64) -> String {
    chrono::DateTime::from_timestamp(t, 0).map(|d| d.format("%Y-%m-%d %H:%M UTC").to_string()).unwrap_or_default()
}

/// A file download.
pub struct Download {
    name: String,
    content_type: ContentType,
    body: Vec<u8>,
}

impl<'r> Responder<'r, 'static> for Download {
    fn respond_to(self, _: &'r Request<'_>) -> response::Result<'static> {
        Response::build()
            .header(self.content_type)
            .raw_header("Content-Disposition", format!("attachment; filename=\"{}\"", self.name))
            .raw_header("Cache-Control", "no-store")
            .raw_header("X-Content-Type-Options", "nosniff")
            .sized_body(self.body.len(), std::io::Cursor::new(self.body))
            .ok()
    }
}

fn short(org: &str) -> &str {
    org.get(..8).unwrap_or(org)
}

/// A page saying why (an export that doesn't verify).
fn problem(text: &str) -> Page {
    Page(super::console_pages::page(
        "Audit",
        None,
        &format!("<section><h1>Audit</h1><p>{}</p></section>", crate::api::reins::pages::escape_html(text)),
        &console::base(),
    ))
}

/// An evidence report for auditors: the org's export is built (and recorded, as any export), verified with the org's
/// decision service key, and only then turned into evidence, exactly as `reins-audit report` does offline.
#[get("/reins/org/<org>/audit/report?<framework>&<from>&<to>&<format>")]
async fn report(
    org: &str,
    framework: Option<&str>,
    from: Option<&str>,
    to: Option<&str>,
    format: Option<&str>,
    cookies: &CookieJar<'_>,
    conn: DbConn,
) -> Result<Download, Page> {
    let Some(s) = admin_of(cookies, &conn, org, AUDIT_VIEW).await else {
        return Err(not_found());
    };
    let framework = Framework::parse(framework.unwrap_or("soc2")).ok_or_else(not_found)?;
    let day = |d: Option<&str>| match d.filter(|d| !d.is_empty()) {
        None => Ok(None),
        Some(d) => evidence::day_start(d).map(Some).ok_or_else(|| problem("Dates are YYYY-MM-DD.")),
    };
    let period = Period {
        from: day(from)?,
        to: day(to)?.map(|t| t + 86_400),
    };
    let json = format == Some("json");
    let (o, u, now) = (org.to_owned(), s.user.clone(), now_unix());
    let (text, decision) = run(&conn, move |c| {
        let text = audit::export(c, &o, None, Some(&u), now)?;
        Ok((text, kms::public_keys(c, &o, now)?.decision))
    })
    .await
    .map_err(|_| not_found())?;
    let key = TrustedKey::parse(&format!("{}={}", decision.0, decision.1)).ok_or_else(not_found)?;
    let (verified, events) = reins_audit::verify_collect(text.as_bytes(), &[key], None)
        .map_err(|e| problem(&format!("The audit log doesn't verify: {e}.")))?;
    let ev = evidence::build(framework, &verified, &events, period);
    let ext = if json {
        "json"
    } else {
        "md"
    };
    let body = if json {
        serde_json::to_vec_pretty(&ev).unwrap_or_default()
    } else {
        evidence::render_markdown(&ev).into_bytes()
    };
    Ok(Download {
        name: format!("reins-{}-evidence-{}-{now}.{ext}", framework_slug(framework), short(org)),
        content_type: if json {
            ContentType::JSON
        } else {
            ContentType::new("text", "markdown")
        },
        body,
    })
}

fn framework_slug(f: Framework) -> &'static str {
    match f {
        Framework::Soc2 => "soc2",
        Framework::Iso27001 => "iso27001",
    }
}

/// The key file `reins-audit verify --keys` reads: the org and its decision service and issuer keys.
#[get("/reins/org/<org>/audit/keys.json")]
async fn keys_file(org: &str, cookies: &CookieJar<'_>, conn: DbConn) -> Result<Download, Page> {
    if admin_of(cookies, &conn, org, AUDIT_VIEW).await.is_none() {
        return Err(not_found());
    }
    let (o, now) = (org.to_owned(), now_unix());
    let keys = run(&conn, move |c| kms::public_keys(c, &o, now)).await.map_err(|_| not_found())?;
    let body = serde_json::json!({
        "org": org,
        "decision": format!("{}={}", keys.decision.0, keys.decision.1),
        "issuer": format!("{}={}", keys.issuer.0, keys.issuer.1),
    });
    Ok(Download {
        name: format!("reins-audit-keys-{}.json", short(org)),
        content_type: ContentType::JSON,
        body: serde_json::to_vec_pretty(&body).unwrap_or_default(),
    })
}

/// A stored checkpoint (`<seq>.json`, the signed envelope byte for byte, which is what was timestamped) or its RFC 3161
/// timestamp response (`<seq>.tsr`), for `openssl ts -verify -data <seq>.json -in <seq>.tsr`.
#[get("/reins/org/<org>/audit/checkpoints/<file>")]
async fn checkpoint_file(org: &str, file: &str, cookies: &CookieJar<'_>, conn: DbConn) -> Result<Download, Page> {
    if admin_of(cookies, &conn, org, AUDIT_VIEW).await.is_none() {
        return Err(not_found());
    }
    let (seq, ext) = file.split_once('.').ok_or_else(not_found)?;
    let seq: i64 = seq.parse().map_err(|_| not_found())?;
    let o = org.to_owned();
    let (body, content_type) = match ext {
        "json" => {
            let cp =
                run(&conn, move |c| Ok(reins_org_exec::q_checkpoint(c, &o, seq)?)).await.map_err(|_| not_found())?;
            (cp.ok_or_else(not_found)?.envelope.into_bytes(), ContentType::JSON)
        }
        "tsr" => {
            let w = run(&conn, move |c| Ok(reins_org_compliance::q_witness(c, &o, seq)?))
                .await
                .map_err(|_| not_found())?
                .ok_or_else(not_found)?;
            let der = data_encoding::BASE64.decode(w.response.as_bytes()).map_err(|_| not_found())?;
            (der, ContentType::new("application", "timestamp-reply"))
        }
        _ => return Err(not_found()),
    };
    Ok(Download {
        name: format!("reins-checkpoint-{}-{seq}.{ext}", short(org)),
        content_type,
        body,
    })
}

#[get("/reins/org/<org>/audit/streaming")]
async fn streaming(org: &str, cookies: &CookieJar<'_>, conn: DbConn) -> Page {
    let Some(s) = admin_of(cookies, &conn, org, STREAM_VIEW).await else {
        return not_found();
    };
    let (o, u) = (org.to_owned(), s.user.clone());
    let loaded = run(&conn, move |c| {
        let stored = reins_org_compliance::q_sinks_of(c, &o)?;
        let mut rows = Vec::new();
        for kind in SinkKind::ALL {
            let row = stored.iter().find(|r| r.kind == kind.as_str());
            let public = match row {
                Some(r) => sinks::config(c, r).map(|cfg| cfg.public()).unwrap_or_default(),
                None => Vec::new(),
            };
            rows.push(SinkRow {
                kind,
                installed: row.is_some(),
                enabled: row.is_some_and(|r| r.enabled),
                failing: row.is_some_and(|r| r.failing),
                public,
                sent_seq: row.map_or(0, |r| r.sent_seq),
                last_ok: row.and_then(|r| r.last_ok_at).map(when),
                last_error: row.and_then(|r| r.last_error.clone()),
            });
        }
        Ok((rows, can(c, &o, &u, Capability::ActivatePolicy)?))
    })
    .await;
    let (Ok((rows, can_manage)), Ok(name)) = (loaded, org_name(&conn, org).await) else {
        return not_found();
    };
    let base = console::base();
    let frame = Frame {
        base: &base,
        email: &s.email,
        csrf: &s.csrf,
        org: Some((org, &name)),
        tab: Tab::Audit,
        flash: s.flash.as_ref().map(|(t, b)| (t.as_str(), *b)),
    };
    Page(pages::streaming(&frame, &rows, can_manage))
}

/// A sink's configuration from its setup form: only its own fields, empty ones left out, `lock_days` a number.
fn config_of(kind: SinkKind, form: &HashMap<String, String>) -> Value {
    let mut out = Map::new();
    for name in pages::field_names(kind) {
        let Some(v) = form.get(name).map(|v| v.trim()).filter(|v| !v.is_empty()) else {
            continue;
        };
        let value = if name == "lock_days" {
            v.parse::<u32>().map_or_else(|_| Value::String(v.to_owned()), Value::from)
        } else {
            Value::String(v.to_owned())
        };
        out.insert(name.to_owned(), value);
    }
    Value::Object(out)
}

#[post("/reins/org/<org>/audit/streaming/<kind>", data = "<form>")]
async fn install(
    org: &str,
    kind: &str,
    form: Form<HashMap<String, String>>,
    cookies: &CookieJar<'_>,
    conn: DbConn,
) -> Redirect {
    let Some(s) = admin_of(cookies, &conn, org, STREAM_MANAGE).await else {
        return to_home();
    };
    let Some(kind) = SinkKind::parse(kind) else {
        return to_org(org, "/audit/streaming");
    };
    if !form.get("csrf").is_some_and(|t| check_csrf(&s, t)) {
        return to_org(org, "/audit/streaming");
    }
    let config = config_of(kind, &form);
    let (o, now) = (org.to_owned(), now_unix());
    match run(&conn, move |c| sinks::install(c, &o, kind, &config, now)).await {
        Ok(()) => flash(&s.sid, "Saved. Turn it on to start.", false),
        Err(e) => flash(&s.sid, e.message(), true),
    }
    to_org(org, "/audit/streaming")
}

#[derive(FromForm)]
struct CsrfForm {
    csrf: String,
}

#[post("/reins/org/<org>/audit/streaming/<kind>/<op>", data = "<form>")]
async fn toggle(
    org: &str,
    kind: &str,
    op: &str,
    form: Form<CsrfForm>,
    cookies: &CookieJar<'_>,
    conn: DbConn,
    pool: &State<DbPool>,
) -> Redirect {
    let Some(s) = admin_of(cookies, &conn, org, STREAM_MANAGE).await else {
        return to_home();
    };
    let (Some(kind), true) = (SinkKind::parse(kind), check_csrf(&s, &form.csrf)) else {
        return to_org(org, "/audit/streaming");
    };
    let enabled = op == "on";
    let o = org.to_owned();
    let config = if enabled {
        run(&conn, move |c| sinks::config_digest(c, &o, kind)).await.ok().flatten()
    } else {
        None
    };
    signed_change(
        &conn,
        pool,
        org,
        &s,
        AdminAction::SetAuditSink {
            sink: kind,
            enabled,
            config,
        },
    )
    .await;
    to_org(org, "/audit/streaming")
}
