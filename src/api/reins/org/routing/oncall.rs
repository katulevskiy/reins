//! On-call reviewers (05 §6: "On-call: from PagerDuty, Opsgenie or Incident.io schedules"), asked of the provider's
//! API when a request needs them, never taken from the agent.
//!
//! - Each source is an org alias (`payments`) naming a provider and a schedule, set by a two-person admin change
//!   (`AdminAction::SetOnCallSource`). Its API key is posted apart, sealed under the tenant's data key, and never
//!   shown again.
//! - Whoever the schedule names is matched to an active member by exact email. An email that isn't a member's is
//!   unresolved, not widened.
//! - An error, or no answer within [`TIMEOUT`], answers nobody: the obligation escalates. Answers are kept 60 s.

use std::{
    collections::{BTreeSet, HashMap},
    sync::{LazyLock, Mutex},
    time::Duration,
};

use reins_proto::org::OnCallProvider;
use serde_json::Value;
use zeroize::Zeroizing;

use super::Resolution;
use crate::{
    api::reins::org::{OrgError, kms},
    db::{
        DbConnInner, DbPool,
        models::{reins_org, reins_org_routing},
    },
};

pub const TIMEOUT: Duration = Duration::from_secs(5);
const CACHE_SECS: i64 = 60;

/// A provider's API base: its own, or (tests only, with `REINS_TEST_ALLOW_LOOPBACK`) `REINS_TEST_<PROVIDER>_API`.
#[must_use]
pub fn api_base(p: OnCallProvider) -> String {
    let (default, env) = match p {
        OnCallProvider::Pagerduty => ("https://api.pagerduty.com", "REINS_TEST_PAGERDUTY_API"),
        OnCallProvider::Opsgenie => ("https://api.opsgenie.com", "REINS_TEST_OPSGENIE_API"),
        OnCallProvider::IncidentIo => ("https://api.incident.io", "REINS_TEST_INCIDENT_IO_API"),
    };
    if crate::api::reins::outbound::allow_loopback()
        && let Ok(b) = std::env::var(env)
        && !b.trim().is_empty()
    {
        return b.trim().trim_end_matches('/').to_owned();
    }
    default.to_owned()
}

fn rfc3339(t: i64) -> String {
    chrono::DateTime::from_timestamp(t, 0)
        .map(|d| d.to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
        .unwrap_or_default()
}

/// Who a schedule names at `at`: lowercase emails, and SHA-256 (hex) of the provider's response as evidence.
pub async fn on_call(
    provider: OnCallProvider,
    base: &str,
    key: &str,
    schedule_id: &str,
    at: i64,
) -> Result<(Vec<String>, String), String> {
    let client = reqwest::Client::builder()
        .timeout(TIMEOUT)
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| e.to_string())?;
    let req = match provider {
        OnCallProvider::Pagerduty => client
            .get(with_query(
                &format!("{base}/oncalls"),
                &[
                    ("schedule_ids[]", schedule_id),
                    ("since", &rfc3339(at)),
                    ("until", &rfc3339(at + 60)),
                    ("include[]", "users"),
                ],
            )?)
            .header("Authorization", format!("Token token={key}"))
            .header("Accept", "application/vnd.pagerduty+json;version=2"),
        OnCallProvider::Opsgenie => client
            .get(with_query(
                &format!("{base}/v2/schedules/{}/on-calls", urlenc(schedule_id)),
                &[("flat", "true"), ("date", &rfc3339(at))],
            )?)
            .header("Authorization", format!("GenieKey {key}")),
        OnCallProvider::IncidentIo => client
            .get(with_query(
                &format!("{base}/v2/schedule_entries"),
                &[
                    ("schedule_id", schedule_id),
                    ("entry_window_start", &rfc3339(at)),
                    ("entry_window_end", &rfc3339(at + 60)),
                ],
            )?)
            .bearer_auth(key),
    };
    let resp = req.send().await.map_err(|e| format!("{}: {e}", provider.label()))?;
    if !resp.status().is_success() {
        return Err(format!("{}: {}", provider.label(), resp.status()));
    }
    let bytes = resp.bytes().await.map_err(|e| e.to_string())?;
    let evidence = reins_proto::canon::hex32(&reins_proto::canon::sha256(&bytes));
    let v: Value = serde_json::from_slice(&bytes).map_err(|_| format!("{}: not JSON", provider.label()))?;
    let emails: Vec<&Value> = match provider {
        OnCallProvider::Pagerduty => {
            v["oncalls"].as_array().into_iter().flatten().map(|o| &o["user"]["email"]).collect()
        }
        OnCallProvider::Opsgenie => v["data"]["onCallRecipients"].as_array().into_iter().flatten().collect(),
        OnCallProvider::IncidentIo => {
            v["schedule_entries"]["final"].as_array().into_iter().flatten().map(|e| &e["user"]["email"]).collect()
        }
    };
    let mut out: Vec<String> =
        emails.into_iter().filter_map(Value::as_str).map(|e| e.trim().to_ascii_lowercase()).collect();
    out.sort();
    out.dedup();
    Ok((out, evidence))
}

fn with_query(url: &str, pairs: &[(&str, &str)]) -> Result<url::Url, String> {
    let mut u = url::Url::parse(url).map_err(|e| e.to_string())?;
    u.query_pairs_mut().extend_pairs(pairs);
    Ok(u)
}

fn urlenc(s: &str) -> String {
    url::form_urlencoded::byte_serialize(s.as_bytes()).collect()
}

/// The active members of `org` with exactly these emails; the others are unresolved.
pub fn map_emails(
    c: &mut DbConnInner,
    org: &str,
    emails: &[String],
) -> Result<(BTreeSet<String>, Vec<String>), OrgError> {
    use crate::db::schema::users;
    use diesel::prelude::*;
    let mut members = BTreeSet::new();
    let mut unknown = 0;
    for e in emails {
        let user: Option<String> = users::table.filter(users::email.eq(e)).select(users::uuid).first(c).optional()?;
        match user {
            Some(u) if reins_org::q_member_state(c, org, &u)?.is_some_and(|m| m.is_active()) => {
                members.insert(u);
            }
            _ => unknown += 1,
        }
    }
    let unresolved = if unknown > 0 {
        vec![format!("{unknown} on call not in the organization")]
    } else {
        Vec::new()
    };
    Ok((members, unresolved))
}

fn aad(org: &str, alias: &str) -> String {
    format!("{org}/oncall/{alias}")
}

/// Seals and stores an on-call source's API key (the source must be set up already).
#[cfg_attr(not(test), allow(dead_code, reason = "the console posts keys (routing page)"))]
pub fn set_key(c: &mut DbConnInner, org: &str, alias: &str, key: &str, now: i64) -> Result<(), OrgError> {
    let key = key.trim();
    if key.is_empty() || key.len() > 512 || key.chars().any(char::is_control) {
        return Err(OrgError::Invalid("That isn't an API key."));
    }
    let (key_id, sealed) = kms::seal_data(c, org, &aad(org, alias), key.as_bytes(), now)?;
    if !reins_org_routing::q_set_oncall_key(c, org, alias, &key_id, &sealed)? {
        return Err(OrgError::Invalid("Set up this on-call source first."));
    }
    Ok(())
}

/// A source ready to ask: (provider, schedule, key).
fn source(
    c: &mut DbConnInner,
    org: &str,
    alias: &str,
) -> Result<Option<(OnCallProvider, String, Zeroizing<String>)>, OrgError> {
    let Some(row) = reins_org_routing::q_oncall_for_use(c, org, alias)? else {
        return Ok(None);
    };
    let (Some(provider), Some(key_id), Some(sealed)) =
        (OnCallProvider::parse(&row.provider), row.key_id.as_deref(), row.sealed.as_deref())
    else {
        return Ok(None);
    };
    let key = kms::open_data(c, org, key_id, &aad(org, alias), sealed)?;
    let key = Zeroizing::new(String::from_utf8(key.to_vec()).map_err(|_| OrgError::Internal("key isn't text".into()))?);
    Ok(Some((provider, row.schedule_id, key)))
}

/// (org, alias) → (asked at, answer).
type Answers = HashMap<(String, String), (i64, Resolution)>;

static CACHE: LazyLock<Mutex<Answers>> = LazyLock::new(Mutex::default);

/// Who is on call for `alias` at `now`, as members.
pub async fn current(pool: &DbPool, org: &str, alias: &str, now: i64) -> Resolution {
    let key = (org.to_owned(), alias.to_owned());
    if let Some((at, r)) = CACHE.lock().unwrap_or_else(std::sync::PoisonError::into_inner).get(&key)
        && now - at < CACHE_SECS
    {
        return r.clone();
    }
    let unavailable = || Resolution::nobody("oncall", &format!("on-call: {alias} unavailable"));
    let Ok(conn) = pool.get().await else {
        return unavailable();
    };
    let (o, a) = (org.to_owned(), alias.to_owned());
    let Ok(Some((provider, schedule, api_key))) = conn.run(move |c| source(c, &o, &a)).await else {
        return unavailable();
    };
    drop(conn);
    let answer = tokio::time::timeout(TIMEOUT, on_call(provider, &api_base(provider), &api_key, &schedule, now)).await;
    let (emails, evidence) = match answer {
        Ok(Ok(x)) => x,
        Ok(Err(e)) => {
            info!("Org {org}: on-call {alias}: {e}");
            return unavailable();
        }
        Err(_) => return unavailable(),
    };
    let Ok(conn) = pool.get().await else {
        return unavailable();
    };
    let o = org.to_owned();
    let Ok((members, unresolved)) = conn.run(move |c| map_emails(c, &o, &emails)).await else {
        return unavailable();
    };
    let r = Resolution {
        members,
        unresolved,
        source: format!("oncall:{}", provider.as_str()),
        evidence,
    };
    CACHE.lock().unwrap_or_else(std::sync::PoisonError::into_inner).insert(key, (now, r.clone()));
    r
}

/// An on-call alias: 1-64 of `a-z`, `0-9`, `.`, `_`, `-`.
#[must_use]
pub fn alias_ok(a: &str) -> bool {
    !a.is_empty()
        && a.len() <= 64
        && a.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'.' | b'_' | b'-'))
}

/// A provider's schedule id: 1-128 of letters, digits, `-` and `_`.
#[must_use]
pub fn schedule_ok(s: &str) -> bool {
    !s.is_empty() && s.len() <= 128 && s.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
}

#[cfg(all(test, sqlite))]
mod tests {
    use serde_json::json;
    use wiremock::matchers::{header, method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::api::reins::org::{
        kms::tests::use_test_kms,
        testing::{add_org, add_user, db, member},
    };

    async fn server() -> MockServer {
        rustls::crypto::ring::default_provider().install_default().ok();
        MockServer::start().await
    }

    fn org() -> DbConnInner {
        let mut c = db();
        use_test_kms();
        add_org(&mut c, "o1");
        member(&mut c, "o1", "ola", &[]);
        add_user(&mut c, "ola", "ola@member.example");
        c
    }

    #[tokio::test]
    async fn pagerduty_on_call_maps_to_members() {
        let mut c = org();
        let pd = server().await;
        Mock::given(method("GET"))
            .and(path("/oncalls"))
            .and(query_param("schedule_ids[]", "PSCHED1"))
            .and(query_param("include[]", "users"))
            .and(header("authorization", "Token token=pd-key"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"oncalls": [
                {"user": {"email": "Ola@Member.Example"}}, {"user": {"email": "ola@member.example"}}]})))
            .mount(&pd)
            .await;
        let (emails, evidence) =
            on_call(OnCallProvider::Pagerduty, &pd.uri(), "pd-key", "PSCHED1", 1_700_000_000).await.unwrap();
        assert_eq!(emails, ["ola@member.example"]);
        assert_eq!(evidence.len(), 64);
        let (members, unresolved) = map_emails(&mut c, "o1", &emails).unwrap();
        assert_eq!(members, ["ola".to_owned()].into());
        assert!(unresolved.is_empty());
    }

    #[tokio::test]
    async fn opsgenie_and_incident_io_parse() {
        let og = server().await;
        Mock::given(method("GET"))
            .and(path("/v2/schedules/sched-1/on-calls"))
            .and(query_param("flat", "true"))
            .and(header("authorization", "GenieKey og-key"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!({"data": {"onCallRecipients": ["a@acme.com"]}})),
            )
            .mount(&og)
            .await;
        let (emails, _) = on_call(OnCallProvider::Opsgenie, &og.uri(), "og-key", "sched-1", 1).await.unwrap();
        assert_eq!(emails, ["a@acme.com"]);
        let io = server().await;
        Mock::given(method("GET"))
            .and(path("/v2/schedule_entries"))
            .and(query_param("schedule_id", "01S"))
            .and(header("authorization", "Bearer io-key"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"schedule_entries": {"final": [
                {"user": {"email": "b@acme.com"}}]}})))
            .mount(&io)
            .await;
        let (emails, _) = on_call(OnCallProvider::IncidentIo, &io.uri(), "io-key", "01S", 1).await.unwrap();
        assert_eq!(emails, ["b@acme.com"]);
    }

    #[test]
    fn an_unknown_email_is_unresolved_not_widened() {
        let mut c = org();
        add_user(&mut c, "outsider", "outsider@else.example");
        let (members, unresolved) =
            map_emails(&mut c, "o1", &["outsider@else.example".to_owned(), "nobody@x.example".to_owned()]).unwrap();
        assert!(members.is_empty());
        assert_eq!(unresolved, ["2 on call not in the organization"]);
    }

    #[tokio::test]
    async fn provider_errors_escalate() {
        let pd = server().await;
        Mock::given(method("GET")).respond_with(ResponseTemplate::new(503)).mount(&pd).await;
        assert!(on_call(OnCallProvider::Pagerduty, &pd.uri(), "k", "P", 1).await.is_err());
        // A slow provider is no answer either.
        let slow = server().await;
        Mock::given(method("GET"))
            .respond_with(
                ResponseTemplate::new(200).set_delay(TIMEOUT + Duration::from_secs(1)).set_body_json(json!({})),
            )
            .mount(&slow)
            .await;
        assert!(on_call(OnCallProvider::Pagerduty, &slow.uri(), "k", "P", 1).await.is_err());
        // Not JSON, or a 200 naming nobody, resolves to nobody (then the obligation escalates, routing::tests).
        let empty = server().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"oncalls": []})))
            .mount(&empty)
            .await;
        assert!(on_call(OnCallProvider::Pagerduty, &empty.uri(), "k", "P", 1).await.unwrap().0.is_empty());
    }

    #[test]
    fn keys_are_sealed_and_never_listed() {
        let mut c = org();
        assert!(set_key(&mut c, "o1", "payments", "pd-secret-key", 5).is_err(), "no source yet");
        reins_org_routing::q_set_oncall_source(&mut c, "o1", "payments", "pagerduty", "P1").unwrap();
        set_key(&mut c, "o1", "payments", "pd-secret-key", 5).unwrap();
        let row = reins_org_routing::q_oncall_for_use(&mut c, "o1", "payments").unwrap().unwrap();
        assert!(!row.sealed.as_deref().unwrap().contains("pd-secret-key"));
        let listed = reins_org_routing::q_oncalls(&mut c, "o1").unwrap();
        assert!(listed[0].has_key && !format!("{listed:?}").contains("pd-secret"));
        let (p, schedule, key) = source(&mut c, "o1", "payments").unwrap().unwrap();
        assert_eq!((p, schedule.as_str(), key.as_str()), (OnCallProvider::Pagerduty, "P1", "pd-secret-key"));
        // Bound to its alias: another alias's sealed key doesn't open as this one.
        reins_org_routing::q_set_oncall_source(&mut c, "o1", "infra", "pagerduty", "P2").unwrap();
        reins_org_routing::q_set_oncall_key(
            &mut c,
            "o1",
            "infra",
            row.key_id.as_deref().unwrap(),
            row.sealed.as_deref().unwrap(),
        )
        .unwrap();
        assert!(source(&mut c, "o1", "infra").is_err());
    }
}
