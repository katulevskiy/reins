//! Audit streaming (`docs/design/decisions-v2/06-trust-and-compliance.md` §5.3): the org's audit chain, event by
//! event, to its SIEM (Splunk, Datadog, Elastic, Microsoft Sentinel), and in signed segments to S3 or GCS with object
//! lock.
//!
//! - **Configured like O3's channels.** A Security admin posts a sink's configuration ([`install`]): validated,
//!   sealed under the tenant data key, and off. Turning it on is a two-person, device-signed admin change
//!   (`AdminAction::SetAuditSink`) naming the configuration's digest, so exactly what was shown is what turns on. A
//!   changed configuration turns the sink off and lapses pending enables: nobody can swap the destination of an
//!   approved stream.
//! - **Delivered at least once, in order.** The sweeper ([`push_all`]) sends the events after the sink's `sent_seq`
//!   and moves it only when the destination accepted them. SIEM records carry `seq` and `hash` (and Elastic's `_id`
//!   is `<org>-<seq>`), so a resend is a duplicate the destination can drop. Object storage gets only whole segments
//!   between signed checkpoints, each a self-contained export that `reins-audit verify --anchor` checks alone.
//! - **Never another tenant's.** Every read is by org; every destination is the org's own configuration.
//! - **Quiet.** Deliveries aren't audit events (an event per delivery would never let the chain rest); a sink starting
//!   or stopping to fail is (`audit.sink_failing`, `audit.sink_recovered`).
//!
//! Requests go through `api::reins::outbound` (https, public addresses only, no redirects). Errors keep the host and
//! status only; secrets are never logged or shown back.

pub mod datadog;
pub mod elastic;
pub mod s3;
pub mod sentinel;
pub mod splunk;

use std::time::Duration;

use diesel::Connection;

use reins_proto::{
    managed::{AuditEvent, canonical_json},
    org::{AdminAction, SinkKind},
};
use reqwest::{Method, header::HeaderMap};
use serde_json::{Value, json};
use zeroize::Zeroizing;

use super::{OrgError, audit, kms};
use crate::{
    api::reins::outbound,
    db::{
        DbConnInner, DbPool,
        models::{
            reins_org,
            reins_org_compliance::{self, ReinsOrgAuditSink},
            reins_org_event, reins_org_exec,
        },
    },
};

/// Events per SIEM delivery.
pub const BATCH: i64 = 500;
/// Bytes per SIEM delivery (about; one event may cross it).
pub const BATCH_BYTES: usize = 1024 * 1024;
/// Events an object-storage segment aims to stay under (it always ends at a checkpoint).
pub const SEGMENT_EVENTS: i64 = 5000;
/// A delivery's total time.
const TIMEOUT: Duration = Duration::from_secs(20);

/// The audit event kinds of streaming.
pub mod kinds {
    pub const INSTALLED: &str = "audit.sink_installed";
    pub const CHANGED: &str = "audit.sink_changed";
    pub const FAILING: &str = "audit.sink_failing";
    pub const RECOVERED: &str = "audit.sink_recovered";
}

fn aad(org: &str, kind: SinkKind) -> String {
    format!("{org}/audit-sink/{}", kind.as_str())
}

/// A sink's configuration, validated.
#[derive(Clone, Debug)]
pub enum SinkConfig {
    Splunk(splunk::Config),
    Datadog(datadog::Config),
    Elastic(elastic::Config),
    Sentinel(sentinel::Config),
    S3(s3::Config),
}

/// Text fields: present, not too long, no control characters.
pub(crate) fn field_ok(s: &str, max: usize) -> bool {
    !s.trim().is_empty() && s.len() <= max && !s.chars().any(char::is_control)
}

/// A destination URL an admin gave: https and not a private address ([`outbound::check_url`]), no query.
pub(crate) fn url_ok(raw: &str) -> Result<String, OrgError> {
    let url = outbound::check_url(raw, outbound::allow_loopback())
        .map_err(|_| OrgError::Invalid("Use an https URL on a public address."))?;
    if url.query().is_some() {
        return Err(OrgError::Invalid("Leave the query out of the URL."));
    }
    Ok(url.as_str().trim_end_matches('/').to_owned())
}

impl SinkConfig {
    /// Parses and checks `config` as `kind`'s configuration.
    pub fn parse(kind: SinkKind, config: &Value) -> Result<Self, OrgError> {
        let bad = |_| OrgError::Invalid("Fill in every field.");
        let cfg = match kind {
            SinkKind::Splunk => Self::Splunk(serde_json::from_value(config.clone()).map_err(bad)?),
            SinkKind::Datadog => Self::Datadog(serde_json::from_value(config.clone()).map_err(bad)?),
            SinkKind::Elastic => Self::Elastic(serde_json::from_value(config.clone()).map_err(bad)?),
            SinkKind::Sentinel => Self::Sentinel(serde_json::from_value(config.clone()).map_err(bad)?),
            SinkKind::S3 => Self::S3(serde_json::from_value(config.clone()).map_err(bad)?),
        };
        match &cfg {
            Self::Splunk(c) => c.check()?,
            Self::Datadog(c) => c.check()?,
            Self::Elastic(c) => c.check()?,
            Self::Sentinel(c) => c.check()?,
            Self::S3(c) => c.check()?,
        }
        Ok(cfg)
    }

    /// What the console may show of it: never a token, key or secret.
    #[must_use]
    pub fn public(&self) -> Vec<(&'static str, String)> {
        match self {
            Self::Splunk(c) => vec![("URL", c.url.clone()), ("Index", c.index.clone().unwrap_or_default())],
            Self::Datadog(c) => vec![("Site", c.site.clone())],
            Self::Elastic(c) => vec![("URL", c.url.clone()), ("Index", c.index.clone())],
            Self::Sentinel(c) => {
                vec![("Endpoint", c.endpoint.clone()), ("Rule", c.dcr_id.clone()), ("Stream", c.stream.clone())]
            }
            Self::S3(c) => vec![
                ("Bucket", c.bucket.clone()),
                ("Prefix", c.prefix.clone()),
                ("Region", c.region.clone()),
                ("Lock (days)", c.lock_days.to_string()),
            ],
        }
    }
}

/// SHA-256 (hex) of a configuration's canonical JSON.
#[must_use]
pub fn digest_of(config: &Value) -> String {
    reins_proto::canon::hex32(&reins_proto::canon::sha256(canonical_json(config).as_bytes()))
}

/// Seals and stores a sink's configuration as a Security admin posted it. It stays off (or, when it was on and changed,
/// goes off) until a two-person change turns it on; pending enables of an earlier configuration lapse.
pub fn install(c: &mut DbConnInner, org: &str, kind: SinkKind, config: &Value, now: i64) -> Result<(), OrgError> {
    let parsed = SinkConfig::parse(kind, config)?;
    let digest = digest_of(config);
    c.transaction(|c| {
        let before = reins_org_compliance::q_sink(c, org, kind.as_str())?;
        let changed = before.as_ref().is_none_or(|b| !crate::crypto::ct_eq(&b.digest, &digest));
        if changed {
            for ch in reins_org::q_pending_changes(c, org)? {
                let ours = serde_json::from_str::<AdminAction>(&ch.action)
                    .is_ok_and(|a| matches!(a, AdminAction::SetAuditSink { sink, enabled: true, .. } if sink == kind));
                if ours {
                    reins_org::q_decide(c, &ch.uuid, reins_org::CHANGE_REJECTED, "system:reinstall", now)?;
                    super::event(c, org, "change.rejected", None, Some(&ch.uuid), &json!({"why": "reinstalled"}), now)?;
                }
            }
        }
        let was_on = before.as_ref().is_some_and(|b| b.enabled);
        let text = Zeroizing::new(canonical_json(config));
        let (key_id, sealed) = kms::seal_data(c, org, &aad(org, kind), text.as_bytes(), now)?;
        let row = ReinsOrgAuditSink {
            org_uuid: org.to_owned(),
            kind: kind.as_str().to_owned(),
            key_id,
            sealed,
            digest: digest.clone(),
            enabled: was_on && !changed,
            sent_seq: before.as_ref().map_or(0, |b| b.sent_seq),
            last_ok_at: before.as_ref().and_then(|b| b.last_ok_at),
            last_error: None,
            failing: false,
            updated_at: now,
        };
        reins_org_compliance::q_save_sink(c, &row)?;
        audit::record(
            c,
            org,
            kinds::INSTALLED,
            None,
            Some(kind.as_str()),
            &json!({"sink": kind.as_str(), "digest": digest, "turned_off": was_on && changed,
                    "destination": parsed.public().into_iter().map(|(k, v)| format!("{k}: {v}")).collect::<Vec<_>>()}),
            now,
        )?;
        Ok(())
    })
}

/// The digest of `kind`'s stored configuration in `org`, if any.
pub fn config_digest(c: &mut DbConnInner, org: &str, kind: SinkKind) -> Result<Option<String>, OrgError> {
    Ok(reins_org_compliance::q_sink(c, org, kind.as_str())?.map(|r| r.digest))
}

/// The stored configuration of `kind` in `org` (opened), for delivery or the console.
pub fn config(c: &mut DbConnInner, row: &ReinsOrgAuditSink) -> Result<SinkConfig, OrgError> {
    let kind = SinkKind::parse(&row.kind).ok_or(OrgError::NotFound)?;
    let plain = kms::open_data(c, &row.org_uuid, &row.key_id, &aad(&row.org_uuid, kind), &row.sealed)?;
    let v: Value =
        serde_json::from_slice(&plain).map_err(|e| OrgError::Internal(format!("stored sink config: {e}")))?;
    SinkConfig::parse(kind, &v)
}

/// Validates `SetAuditSink` (inside the admin change's transaction): turning on needs the configuration it was
/// proposed for.
pub fn validate(
    c: &mut DbConnInner,
    org: &str,
    kind: SinkKind,
    enabled: bool,
    proposed: Option<&str>,
) -> Result<(), OrgError> {
    if !enabled {
        return Ok(());
    }
    let Some(current) = config_digest(c, org, kind)? else {
        return Err(OrgError::Invalid("Set it up first."));
    };
    if proposed.is_none_or(|d| !crate::crypto::ct_eq(d, &current)) {
        return Err(OrgError::Invalid("It was changed since. Propose it again."));
    }
    Ok(())
}

/// Where a SIEM sink starts on its first enable: the whole kept chain. Object storage starts at the prune anchor
/// (a checkpoint) or the genesis.
fn start_seq(c: &mut DbConnInner, org: &str) -> Result<i64, OrgError> {
    Ok(match reins_org_event::q_first(c, org)? {
        Some((first, _)) if first > 1 => first,
        _ => 0,
    })
}

/// Applies `SetAuditSink`.
pub fn apply_admin(
    c: &mut DbConnInner,
    org: &str,
    kind: SinkKind,
    enabled: bool,
    actor: &str,
    now: i64,
) -> Result<(), OrgError> {
    let Some(mut row) = reins_org_compliance::q_sink(c, org, kind.as_str())? else {
        return if enabled {
            Err(OrgError::Invalid("Set it up first."))
        } else {
            Ok(())
        };
    };
    if enabled && !row.enabled && row.sent_seq == 0 {
        let start = start_seq(c, org)?;
        // A SIEM gets events after `sent_seq`; the anchor event itself is in the checkpoint, not the stream.
        row.sent_seq = start;
    }
    row.enabled = enabled;
    row.failing = false;
    row.last_error = None;
    row.updated_at = now;
    reins_org_compliance::q_save_sink(c, &row)?;
    audit::record(
        c,
        org,
        kinds::CHANGED,
        Some(actor),
        Some(kind.as_str()),
        &json!({"sink": kind.as_str(), "enabled": enabled, "digest": row.digest}),
        now,
    )?;
    Ok(())
}

/// One delivery, read from the database and sent outside any transaction.
pub struct Job {
    pub org: String,
    pub kind: SinkKind,
    pub digest: String,
    pub config: SinkConfig,
    pub payload: Payload,
    /// The last sequence number this delivery covers.
    pub last: i64,
}

pub enum Payload {
    /// Events, oldest first (SIEMs).
    Events(Vec<AuditEvent>),
    /// A whole segment export, after the checkpoint at `after` (0: the genesis) up to the checkpoint at `to`.
    Segment {
        after: i64,
        to: i64,
        text: String,
    },
}

/// What a sink has to send now, if anything.
pub fn prepare(c: &mut DbConnInner, org: &str, kind: SinkKind, now: i64) -> Result<Option<Job>, OrgError> {
    let Some(row) = reins_org_compliance::q_sink(c, org, kind.as_str())?.filter(|r| r.enabled) else {
        return Ok(None);
    };
    let config = config(c, &row)?;
    let start = start_seq(c, org)?;
    let payload = if kind == SinkKind::S3 {
        // From the last segment's end, or from the prune anchor when that checkpoint was pruned since.
        let mut after = row.sent_seq;
        if after < start || (after > 0 && reins_org_exec::q_checkpoint(c, org, after)?.is_none()) {
            after = start;
        }
        let ahead = reins_org_exec::q_checkpoints(c, org, after)?;
        let Some(first) = ahead.first() else {
            return Ok(None);
        };
        let to = ahead.iter().rev().find(|cp| cp.seq - after <= SEGMENT_EVENTS).unwrap_or(first).seq;
        let text = audit::segment(c, org, (after > 0).then_some(after), to, now)?;
        Payload::Segment {
            after,
            to,
            text,
        }
    } else {
        let after = row.sent_seq.max(start);
        let mut events = Vec::new();
        let mut bytes = 0;
        for e in reins_org_event::q_events_from(c, org, after, BATCH)? {
            if bytes > BATCH_BYTES {
                break;
            }
            bytes += e.detail.len() + 256;
            events.push(e.to_audit());
        }
        if events.is_empty() {
            return Ok(None);
        }
        Payload::Events(events)
    };
    let last = match &payload {
        Payload::Events(events) => events.last().map_or(0, |e| e.seq),
        Payload::Segment {
            to,
            ..
        } => *to,
    };
    Ok(Some(Job {
        org: org.to_owned(),
        kind,
        digest: row.digest,
        config,
        payload,
        last,
    }))
}

/// Records a delivery's outcome: `sent_seq` moves only on success, and only for the configuration that was sent.
pub fn settle(c: &mut DbConnInner, job: &Job, outcome: Result<(), String>, now: i64) -> Result<(), OrgError> {
    c.transaction(|c| {
        let Some(mut row) = reins_org_compliance::q_sink(c, &job.org, job.kind.as_str())? else {
            return Ok(());
        };
        if !row.enabled || row.digest != job.digest {
            return Ok(());
        }
        let was_failing = row.failing;
        match outcome {
            Ok(()) => {
                row.sent_seq = row.sent_seq.max(job.last);
                row.last_ok_at = Some(now);
                row.last_error = None;
                row.failing = false;
                row.updated_at = now;
                reins_org_compliance::q_save_sink(c, &row)?;
                if was_failing {
                    audit::record(
                        c,
                        &job.org,
                        kinds::RECOVERED,
                        None,
                        Some(job.kind.as_str()),
                        &json!({"sink": job.kind.as_str()}),
                        now,
                    )?;
                }
            }
            Err(why) => {
                row.last_error = Some(why.clone());
                row.failing = true;
                row.updated_at = now;
                reins_org_compliance::q_save_sink(c, &row)?;
                if !was_failing {
                    audit::record(
                        c,
                        &job.org,
                        kinds::FAILING,
                        None,
                        Some(job.kind.as_str()),
                        &json!({"sink": job.kind.as_str(), "error": why}),
                        now,
                    )?;
                }
            }
        }
        Ok(())
    })
}

/// A transport's answer: the status and up to 1 MiB of the body.
pub type Sent<'a> = std::pin::Pin<Box<dyn Future<Output = Result<(u16, Vec<u8>), String>> + Send + 'a>>;

/// How a delivery reaches the network: through [`outbound`]'s guard in the server, plainly in unit tests (whose mock
/// servers are on loopback over http).
pub trait Transport: Send + Sync {
    fn send<'a>(&'a self, method: Method, url: &'a str, headers: HeaderMap, body: Vec<u8>) -> Sent<'a>;
}

/// The server's transport: https to public addresses only.
pub struct Guarded;

impl Transport for Guarded {
    fn send<'a>(&'a self, method: Method, url: &'a str, headers: HeaderMap, body: Vec<u8>) -> Sent<'a> {
        Box::pin(async move {
            let response =
                outbound::request_once(method, url, headers, body, TIMEOUT).await.map_err(|e| e.to_string())?;
            let status = response.status().as_u16();
            let (body, _) = outbound::read_limited(response, 1024 * 1024).await.map_err(|e| e.to_string())?;
            Ok((status, body))
        })
    }
}

/// `host` of `url`, for error texts.
pub(crate) fn host_of(url: &str) -> String {
    url::Url::parse(url).ok().and_then(|u| u.host_str().map(str::to_owned)).unwrap_or_else(|| "the destination".into())
}

/// A non-2xx answer in words: host and status only.
pub(crate) fn refused(url: &str, status: u16) -> String {
    format!("{} answered {status}", host_of(url))
}

/// Builds a header map from known-good names and values (a bad value, e.g. a token with a newline, is refused).
pub(crate) fn headers(pairs: &[(&str, String)]) -> Result<HeaderMap, String> {
    let mut map = HeaderMap::new();
    for (k, v) in pairs {
        let name = reqwest::header::HeaderName::from_bytes(k.as_bytes()).map_err(|_| format!("bad header {k}"))?;
        let value = reqwest::header::HeaderValue::from_str(v).map_err(|_| format!("bad value for {k}"))?;
        map.insert(name, value);
    }
    Ok(map)
}

/// Sends one job.
pub async fn deliver(t: &dyn Transport, job: &Job, now: i64) -> Result<(), String> {
    match (&job.config, &job.payload) {
        (SinkConfig::Splunk(c), Payload::Events(e)) => splunk::deliver(t, c, e).await,
        (SinkConfig::Datadog(c), Payload::Events(e)) => datadog::deliver(t, c, &job.org, e).await,
        (SinkConfig::Elastic(c), Payload::Events(e)) => elastic::deliver(t, c, e).await,
        (SinkConfig::Sentinel(c), Payload::Events(e)) => sentinel::deliver(t, c, e).await,
        (
            SinkConfig::S3(c),
            Payload::Segment {
                after,
                to,
                text,
            },
        ) => s3::deliver(t, c, &job.org, *after, *to, text, now).await,
        _ => Err("the payload doesn't fit the sink".to_owned()),
    }
}

/// One pass over every enabled sink (the sweeper's): read, send, record.
pub async fn push_all(pool: &DbPool, now: i64) {
    let Ok(conn) = pool.get().await else {
        return;
    };
    let sinks = match conn.run(reins_org_compliance::q_enabled_sinks).await {
        Ok(s) => s,
        Err(e) => {
            warn!("Audit streaming: {e:?}");
            return;
        }
    };
    for row in sinks {
        let Some(kind) = SinkKind::parse(&row.kind) else {
            continue;
        };
        let org = row.org_uuid.clone();
        let job = match conn.run(move |c| prepare(c, &org, kind, now)).await {
            Ok(Some(job)) => job,
            Ok(None) => continue,
            Err(e) => {
                warn!("Audit streaming to {} for {}: {e:?}", kind.as_str(), row.org_uuid);
                continue;
            }
        };
        let outcome = deliver(&Guarded, &job, now).await;
        if let Err(e) = &outcome {
            warn!("Audit streaming to {} for {}: {e}", kind.as_str(), job.org);
        }
        if let Err(e) = conn.run(move |c| settle(c, &job, outcome, now)).await {
            warn!("Audit streaming: recording a delivery: {e:?}");
        }
    }
}

#[cfg(test)]
pub(crate) mod testing {
    use super::*;

    /// Plain http, for mock servers on loopback.
    pub struct Plain;

    impl Transport for Plain {
        fn send<'a>(&'a self, method: Method, url: &'a str, headers: HeaderMap, body: Vec<u8>) -> Sent<'a> {
            Box::pin(async move {
                let r = reqwest::Client::new()
                    .request(method, url)
                    .headers(headers)
                    .body(body)
                    .send()
                    .await
                    .map_err(|e| e.to_string())?;
                let status = r.status().as_u16();
                Ok((status, r.bytes().await.map_err(|e| e.to_string())?.to_vec()))
            })
        }
    }

    pub fn events(org: &str, n: i64) -> Vec<AuditEvent> {
        let mut prev = reins_proto::org::GENESIS_HASH.to_owned();
        (1..=n)
            .map(|seq| {
                let mut e = AuditEvent {
                    org: org.into(),
                    seq,
                    at: 1_793_102_400 + seq,
                    kind: "decision.auto".into(),
                    actor: Some("u1".into()),
                    subject: None,
                    detail: format!("{{\"n\":{seq}}}"),
                    prev_hash: prev.clone(),
                    hash: String::new(),
                };
                e.hash = e.expected_hash();
                prev.clone_from(&e.hash);
                e
            })
            .collect()
    }
}

#[cfg(all(test, sqlite))]
mod tests {
    use reins_proto::org::Role;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{method, path},
    };

    use super::{testing::Plain, *};
    use crate::api::reins::org::{admin, kms::tests::use_test_kms, testing as t};

    fn splunk(url: &str) -> Value {
        json!({"url": url, "token": "hec-secret"})
    }

    /// o1 with two Security admins (u1, u2), some events.
    fn org() -> DbConnInner {
        use_test_kms();
        let mut c = t::db();
        t::add_org(&mut c, "o1");
        t::member(&mut c, "o1", "u1", &[Role::SecurityAdmin]);
        t::member(&mut c, "o1", "u2", &[Role::SecurityAdmin]);
        for i in 0..3 {
            audit::record(&mut c, "o1", "decision.auto", Some("u1"), None, &json!({"i": i}), 10 + i).unwrap();
        }
        c
    }

    fn turn_on(c: &mut DbConnInner, kind: SinkKind) {
        let digest = config_digest(c, "o1", kind).unwrap().unwrap();
        let on = AdminAction::SetAuditSink {
            sink: kind,
            enabled: true,
            config: Some(digest),
        };
        let id = match admin::propose(c, "o1", "u1", &on, None, 20).unwrap().0 {
            admin::Proposed::Pending(id) => id,
            admin::Proposed::Applied => panic!("needs two people"),
        };
        admin::approve(c, "o1", &id, "u2", None, 21).unwrap();
    }

    #[test]
    fn a_sink_stays_off_until_two_people_turn_it_on() {
        let mut c = org();
        install(&mut c, "o1", SinkKind::Splunk, &splunk("https://splunk.example.com:8088"), 15).unwrap();
        assert!(prepare(&mut c, "o1", SinkKind::Splunk, 16).unwrap().is_none(), "off after install");
        // A proposal naming another configuration is refused.
        let stale = AdminAction::SetAuditSink {
            sink: SinkKind::Splunk,
            enabled: true,
            config: Some("0".repeat(64)),
        };
        assert!(admin::propose(&mut c, "o1", "u1", &stale, None, 17).is_err());
        turn_on(&mut c, SinkKind::Splunk);
        let job = prepare(&mut c, "o1", SinkKind::Splunk, 22).unwrap().unwrap();
        let Payload::Events(events) = &job.payload else {
            panic!("events")
        };
        assert_eq!(events.first().unwrap().seq, 1, "the whole chain");
        // The stored configuration is sealed: the secret isn't in the row.
        let row = reins_org_compliance::q_sink(&mut c, "o1", "splunk").unwrap().unwrap();
        assert!(!row.sealed.contains("hec-secret"));
    }

    #[test]
    fn reinstalling_turns_a_sink_off() {
        let mut c = org();
        install(&mut c, "o1", SinkKind::Splunk, &splunk("https://splunk.example.com:8088"), 15).unwrap();
        // A pending enable of the first configuration…
        let digest = config_digest(&mut c, "o1", SinkKind::Splunk).unwrap().unwrap();
        let on = AdminAction::SetAuditSink {
            sink: SinkKind::Splunk,
            enabled: true,
            config: Some(digest.clone()),
        };
        let admin::Proposed::Pending(pending) = admin::propose(&mut c, "o1", "u1", &on, None, 16).unwrap().0 else {
            panic!("pending")
        };
        // …lapses when the destination changes.
        install(&mut c, "o1", SinkKind::Splunk, &splunk("https://evil.example.net:8088"), 17).unwrap();
        assert!(admin::approve(&mut c, "o1", &pending, "u2", None, 18).is_err());
        turn_on(&mut c, SinkKind::Splunk);
        assert!(reins_org_compliance::q_sink(&mut c, "o1", "splunk").unwrap().unwrap().enabled);
        install(&mut c, "o1", SinkKind::Splunk, &splunk("https://other.example.net:8088"), 30).unwrap();
        assert!(!reins_org_compliance::q_sink(&mut c, "o1", "splunk").unwrap().unwrap().enabled, "changed: off");
        // The same configuration again changes nothing.
        turn_on(&mut c, SinkKind::Splunk);
        install(&mut c, "o1", SinkKind::Splunk, &splunk("https://other.example.net:8088"), 40).unwrap();
        assert!(reins_org_compliance::q_sink(&mut c, "o1", "splunk").unwrap().unwrap().enabled);
    }

    #[test]
    fn a_private_or_http_url_is_refused() {
        let mut c = org();
        for url in ["http://splunk.example.com", "https://10.1.2.3:8088", "https://169.254.169.254", "not a url"] {
            assert!(install(&mut c, "o1", SinkKind::Splunk, &splunk(url), 15).is_err(), "{url}");
        }
        let elastic = json!({"url": "https://127.0.0.1:9200", "api_key": "k", "index": "reins"});
        assert!(install(&mut c, "o1", SinkKind::Elastic, &elastic, 15).is_err());
        assert!(reins_org_compliance::q_sinks_of(&mut c, "o1").unwrap().is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn splunk_gets_every_event_once_and_the_cursor_follows() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/services/collector/event"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;
        let mut c = org();
        install(&mut c, "o1", SinkKind::Splunk, &splunk("https://splunk.example.com:8088"), 15).unwrap();
        turn_on(&mut c, SinkKind::Splunk);
        let mut job = prepare(&mut c, "o1", SinkKind::Splunk, 30).unwrap().unwrap();
        let SinkConfig::Splunk(cfg) = &mut job.config else {
            panic!()
        };
        cfg.url = server.uri();
        deliver(&Plain, &job, 30).await.unwrap();
        settle(&mut c, &job, Ok(()), 30).unwrap();
        let sent = reins_org_compliance::q_sink(&mut c, "o1", "splunk").unwrap().unwrap().sent_seq;
        assert_eq!(sent, job.last);
        // Only what came after.
        let next = prepare(&mut c, "o1", SinkKind::Splunk, 31).unwrap();
        assert!(next.is_none_or(|j| matches!(&j.payload, Payload::Events(e) if e[0].seq == sent + 1)));
        let received = server.received_requests().await.unwrap();
        let body = String::from_utf8(received[0].body.clone()).unwrap();
        assert_eq!(body.matches("\"sourcetype\":\"reins:audit\"").count(), usize::try_from(job.last).unwrap());
        assert_eq!(received[0].headers.get("authorization").unwrap(), "Splunk hec-secret");
    }

    #[test]
    fn a_failed_push_keeps_the_cursor_and_records_failing_once() {
        let mut c = org();
        install(&mut c, "o1", SinkKind::Splunk, &splunk("https://splunk.example.com:8088"), 15).unwrap();
        turn_on(&mut c, SinkKind::Splunk);
        let failing = |c: &mut DbConnInner| {
            reins_org_event::q_events(c, "o1", None, 100)
                .unwrap()
                .into_iter()
                .filter(|e| e.kind == kinds::FAILING)
                .count()
        };
        for now in [30, 31] {
            let job = prepare(&mut c, "o1", SinkKind::Splunk, now).unwrap().unwrap();
            settle(&mut c, &job, Err("splunk.example.com answered 503".into()), now).unwrap();
        }
        let row = reins_org_compliance::q_sink(&mut c, "o1", "splunk").unwrap().unwrap();
        assert_eq!(row.sent_seq, 0);
        assert!(row.failing);
        assert_eq!(failing(&mut c), 1, "recorded once");
        let job = prepare(&mut c, "o1", SinkKind::Splunk, 32).unwrap().unwrap();
        settle(&mut c, &job, Ok(()), 32).unwrap();
        let kinds_now: Vec<String> =
            reins_org_event::q_events(&mut c, "o1", None, 1).unwrap().into_iter().map(|e| e.kind).collect();
        assert_eq!(kinds_now, [kinds::RECOVERED]);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn s3_objects_are_whole_segments_signed_and_locked() {
        use wiremock::matchers::header;
        let server = MockServer::start().await;
        Mock::given(method("PUT"))
            .and(header("x-amz-object-lock-mode", "COMPLIANCE"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;
        let mut c = org();
        let s3 = json!({"region": "eu-west-1", "bucket": "acme-audit", "prefix": "reins/",
                        "access_key_id": "AKID", "secret_access_key": "s3-secret", "lock_days": 30});
        install(&mut c, "o1", SinkKind::S3, &s3, 15).unwrap();
        turn_on(&mut c, SinkKind::S3);
        assert!(prepare(&mut c, "o1", SinkKind::S3, 22).unwrap().is_none(), "nothing until a checkpoint");
        let first = audit::checkpoint(&mut c, "o1", 23).unwrap().unwrap();
        let keys = vec![kms::tests::trusted(&kms::signing_key(&mut c, "o1", kms::KeyPurpose::Decision, 1).unwrap())];
        let mut anchor = None;
        for round in 0..2 {
            let mut job = prepare(&mut c, "o1", SinkKind::S3, 30 + round).unwrap().unwrap();
            let SinkConfig::S3(cfg) = &mut job.config else {
                panic!()
            };
            cfg.endpoint = Some(server.uri());
            let Payload::Segment {
                after,
                to,
                text,
            } = &job.payload
            else {
                panic!()
            };
            let report = reins_audit::verify(text.as_bytes(), &keys, anchor.as_ref()).unwrap();
            assert_eq!((report.first_seq, report.last_seq), (after + 1, *to));
            deliver(&Plain, &job, 30).await.unwrap();
            settle(&mut c, &job, Ok(()), 30).unwrap();
            anchor = Some(reins_proto::managed::Checkpoint {
                org: "o1".into(),
                seq: *to,
                hash: reins_org_exec::q_checkpoint(&mut c, "o1", *to).unwrap().unwrap().hash,
                at: 0,
            });
            // More events and a checkpoint for the second round.
            audit::record(&mut c, "o1", "decision.auto", None, None, &json!({"r": round}), 40).unwrap();
            audit::checkpoint(&mut c, "o1", 41 + round).unwrap().unwrap();
        }
        let puts = server.received_requests().await.unwrap();
        assert_eq!(puts.len(), 2);
        assert_eq!(puts[0].url.path(), format!("/acme-audit/reins/o1/000000000000-{:012}.jsonl", first.seq));
        let auth = puts[0].headers.get("authorization").unwrap().to_str().unwrap();
        assert!(auth.starts_with("AWS4-HMAC-SHA256 Credential=AKID/"), "{auth}");
        assert!(puts[0].headers.get("content-md5").is_some());
        assert!(puts[1].url.path().starts_with(&format!("/acme-audit/reins/o1/{:012}-", first.seq)));
    }

    #[test]
    fn no_sink_ever_sees_another_tenant() {
        let mut c = org();
        t::add_org(&mut c, "o2");
        for i in 0..4 {
            audit::record(&mut c, "o2", "decision.auto", None, None, &json!({"other": i}), 10 + i).unwrap();
        }
        install(&mut c, "o1", SinkKind::Splunk, &splunk("https://splunk.example.com:8088"), 15).unwrap();
        turn_on(&mut c, SinkKind::Splunk);
        let job = prepare(&mut c, "o1", SinkKind::Splunk, 30).unwrap().unwrap();
        let Payload::Events(events) = &job.payload else {
            panic!()
        };
        assert!(events.iter().all(|e| e.org == "o1"));
        assert!(prepare(&mut c, "o2", SinkKind::Splunk, 30).unwrap().is_none());
        // Another tenant's sealed configuration doesn't open as this one's.
        let mut row = reins_org_compliance::q_sink(&mut c, "o1", "splunk").unwrap().unwrap();
        row.org_uuid = "o2".into();
        assert!(config(&mut c, &row).is_err());
    }
}
