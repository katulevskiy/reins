//! Slack: the org's own Slack app (bot scopes `chat:write`, `users:read`, `users:read.email`, `im:write`). A
//! Connection admin posts its bot token and signing secret (sealed); a two-person change turns it on.
//!
//! - **Link:** `users.lookupByEmail`, by the member's work email.
//! - **Post:** `conversations.open` with the member, then `chat.postMessage`: one section (the line), a context (the
//!   deadline) and, for L0–L2, a URL button "Review" to the signed web approval. L3 says "Open the Reins app".
//! - **Update:** `chat.update` with the outcome.
//! - **Events:** `POST /reins/org/slack/events` verifies `X-Slack-Signature` (`v0=` HMAC-SHA256 of `v0:<ts>:<body>`)
//!   within five minutes, answers `url_verification`, acknowledges everything else, and decides nothing.

use rocket::{
    Route,
    data::{Data, ToByteUnit},
    http::Status,
    request::{FromRequest, Outcome, Request},
    serde::json::Json,
};
use serde_json::{Value, json};
use zeroize::Zeroizing;

use super::{ApprovalChannel, Message, api_base, due_text, http};
use crate::{
    api::reins::{now_unix, org::enabled},
    db::{DbConn, models::reins_org_routing as store},
};

/// How old a signed event may be.
pub const MAX_SKEW: i64 = 300;
const API: &str = "https://slack.com/api";
const API_ENV: &str = "REINS_TEST_SLACK_API";

/// The org's Slack app.
#[derive(Clone)]
pub struct SlackConfig {
    pub bot_token: Zeroizing<String>,
    pub signing_secret: Zeroizing<String>,
}

impl SlackConfig {
    #[must_use]
    pub fn from_value(v: &Value) -> Option<Self> {
        let s = |k: &str| v.get(k).and_then(Value::as_str).map(str::trim).filter(|x| !x.is_empty() && x.len() <= 512);
        let token = s("bot_token").filter(|t| t.starts_with("xoxb-"))?;
        Some(Self {
            bot_token: Zeroizing::new(token.to_owned()),
            signing_secret: Zeroizing::new(s("signing_secret")?.to_owned()),
        })
    }
}

pub struct Slack {
    cfg: SlackConfig,
    base: String,
}

impl Slack {
    #[must_use]
    pub fn new(cfg: SlackConfig) -> Self {
        Self::with_base(cfg, &api_base(API, API_ENV))
    }

    #[must_use]
    pub fn with_base(cfg: SlackConfig, base: &str) -> Self {
        Self {
            cfg,
            base: base.trim_end_matches('/').to_owned(),
        }
    }

    async fn call(&self, method: &str, body: &Value) -> Result<Value, String> {
        let resp = http()?
            .post(format!("{}/{method}", self.base))
            .bearer_auth(self.cfg.bot_token.as_str())
            .json(body)
            .send()
            .await
            .map_err(|e| format!("Slack {method}: {e}"))?;
        let v: Value = resp.json().await.map_err(|e| format!("Slack {method}: {e}"))?;
        if v["ok"] == Value::Bool(true) {
            Ok(v)
        } else {
            Err(format!("Slack {method}: {}", v["error"].as_str().unwrap_or("error")))
        }
    }
}

/// The Block Kit of a review message: the line, the deadline, and the Review button only with a link (L0–L2).
#[must_use]
pub fn blocks(msg: &Message, url: Option<&str>) -> Value {
    let mut blocks = vec![
        json!({"type": "section", "text": {"type": "plain_text", "text": msg.line}}),
        json!({"type": "context", "elements": [{"type": "plain_text", "text": due_text(msg.due)}]}),
    ];
    match url.filter(|_| msg.web) {
        Some(u) => blocks.push(json!({"type": "actions", "elements": [
            {"type": "button", "text": {"type": "plain_text", "text": "Review"}, "url": u, "style": "primary",
             "action_id": "review"}]})),
        None => {
            blocks.push(json!({"type": "context", "elements": [{"type": "plain_text", "text": "Open the Reins app"}]}));
        }
    }
    Value::Array(blocks)
}

#[rocket::async_trait]
impl ApprovalChannel for Slack {
    async fn link(&self, email: &str) -> Result<String, String> {
        let mut u = url::Url::parse(&format!("{}/users.lookupByEmail", self.base)).map_err(|e| e.to_string())?;
        u.query_pairs_mut().append_pair("email", email);
        let resp = http()?
            .get(u)
            .bearer_auth(self.cfg.bot_token.as_str())
            .send()
            .await
            .map_err(|e| format!("Slack users.lookupByEmail: {e}"))?;
        let v: Value = resp.json().await.map_err(|e| e.to_string())?;
        v["user"]["id"]
            .as_str()
            .filter(|_| v["ok"] == Value::Bool(true))
            .map(str::to_owned)
            .ok_or_else(|| "no such Slack user".to_owned())
    }

    async fn post(&self, to: &str, msg: &Message, url: Option<&str>) -> Result<String, String> {
        let open = self.call("conversations.open", &json!({"users": to})).await?;
        let channel = open["channel"]["id"].as_str().ok_or("no channel")?.to_owned();
        let sent = self
            .call("chat.postMessage", &json!({"channel": channel, "text": msg.line, "blocks": blocks(msg, url)}))
            .await?;
        let ts = sent["ts"].as_str().ok_or("no ts")?;
        Ok(format!("{channel}:{ts}"))
    }

    async fn update(&self, external_ref: &str, outcome: &str) -> Result<(), String> {
        let (channel, ts) = external_ref.split_once(':').ok_or("bad reference")?;
        self.call(
            "chat.update",
            &json!({"channel": channel, "ts": ts, "text": outcome,
                    "blocks": [{"type": "section", "text": {"type": "plain_text", "text": outcome}}]}),
        )
        .await
        .map(drop)
    }
}

/// Whether a Slack event is signed by `secret` and fresh: `v0=` + hex HMAC-SHA256(secret, `v0:<ts>:<body>`), with
/// `|now − ts| ≤` [`MAX_SKEW`], compared in constant time.
#[must_use]
pub fn signature_ok(secret: &str, ts: &str, body: &[u8], signature: &str, now: i64) -> bool {
    let Ok(t) = ts.parse::<i64>() else {
        return false;
    };
    if (now - t).abs() > MAX_SKEW {
        return false;
    }
    let key = ring::hmac::Key::new(ring::hmac::HMAC_SHA256, secret.as_bytes());
    let mut msg = format!("v0:{ts}:").into_bytes();
    msg.extend_from_slice(body);
    let Some(hex) = signature.strip_prefix("v0=") else {
        return false;
    };
    let Ok(given) = data_encoding::HEXLOWER_PERMISSIVE.decode(hex.as_bytes()) else {
        return false;
    };
    ring::hmac::verify(&key, &msg, &given).is_ok()
}

/// The signature headers of an event.
pub struct SlackHeaders {
    ts: String,
    signature: String,
}

#[rocket::async_trait]
impl<'r> FromRequest<'r> for SlackHeaders {
    type Error = ();

    async fn from_request(req: &'r Request<'_>) -> Outcome<Self, ()> {
        let h = |k: &str| req.headers().get_one(k).map(str::to_owned);
        match (h("X-Slack-Request-Timestamp"), h("X-Slack-Signature")) {
            (Some(ts), Some(signature)) => Outcome::Success(Self {
                ts,
                signature,
            }),
            _ => Outcome::Error((Status::Unauthorized, ())),
        }
    }
}

pub fn routes() -> Vec<Route> {
    routes![events]
}

/// What an event asks for: the `url_verification` challenge, or nothing.
#[must_use]
pub fn answer(body: &[u8]) -> Value {
    // Interactive payloads come form-encoded (`payload=…`); events as JSON. Neither decides anything.
    match serde_json::from_slice::<Value>(body) {
        Ok(v) if v["type"] == "url_verification" => json!({"challenge": v["challenge"]}),
        _ => json!({}),
    }
}

#[post("/reins/org/slack/events", data = "<data>", rank = 1)]
async fn events(headers: SlackHeaders, data: Data<'_>, conn: DbConn) -> Result<Json<Value>, Status> {
    if !enabled() {
        return Err(Status::NotFound);
    }
    let body = data.open(64.kibibytes()).into_bytes().await.map_err(|_| Status::BadRequest)?;
    if !body.is_complete() {
        return Err(Status::PayloadTooLarge);
    }
    let body = body.into_inner();
    let (ts, sig, bytes, now) = (headers.ts, headers.signature, body.clone(), now_unix());
    // Whichever org's Slack app signed it; none: refused, and nothing changes.
    let signed = conn
        .run(move |c| -> Result<bool, crate::api::reins::org::OrgError> {
            for row in store::q_channels_of_kind(c, "slack")? {
                if let Some(cfg) = super::config(c, &row.org_uuid, reins_proto::org::ChannelKind::Slack, true)?
                    && let Some(cfg) =
                        serde_json::from_str::<Value>(&cfg).ok().and_then(|v| SlackConfig::from_value(&v))
                    && signature_ok(&cfg.signing_secret, &ts, &bytes, &sig, now)
                {
                    return Ok(true);
                }
            }
            Ok(false)
        })
        .await
        .unwrap_or(false);
    if !signed {
        return Err(Status::Unauthorized);
    }
    Ok(Json(answer(&body)))
}

#[cfg(test)]
mod tests {
    use wiremock::matchers::{body_partial_json, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;

    fn sign(secret: &str, ts: &str, body: &[u8]) -> String {
        let key = ring::hmac::Key::new(ring::hmac::HMAC_SHA256, secret.as_bytes());
        let mut msg = format!("v0:{ts}:").into_bytes();
        msg.extend_from_slice(body);
        format!("v0={}", data_encoding::HEXLOWER.encode(ring::hmac::sign(&key, &msg).as_ref()))
    }

    #[test]
    fn signatures_are_verified_and_replays_refused() {
        let body = br#"{"type":"url_verification","challenge":"c1"}"#;
        let now = 1_700_000_000;
        let ts = now.to_string();
        let sig = sign("s3cret", &ts, body);
        assert!(signature_ok("s3cret", &ts, body, &sig, now));
        assert!(!signature_ok("other", &ts, body, &sig, now), "another secret");
        assert!(!signature_ok("s3cret", &ts, b"{}", &sig, now), "another body");
        assert!(!signature_ok("s3cret", &ts, body, &sig, now + MAX_SKEW + 1), "a replay");
        assert!(!signature_ok("s3cret", &ts, body, &sig.replace("v0=", "v1="), now));
        assert!(!signature_ok("s3cret", "x", body, &sig, now));
        assert_eq!(answer(body), json!({"challenge": "c1"}));
        // An interactive payload is acknowledged and nothing more.
        assert_eq!(answer(b"payload=%7B%22type%22%3A%22block_actions%22%7D"), json!({}));
    }

    fn msg(web: bool) -> Message {
        Message {
            line: "lee@acme.com · Claude · GitPush acme/prod · L2".into(),
            due: 0,
            web,
        }
    }

    #[test]
    fn l3_has_no_web_button() {
        let b = blocks(&msg(false), Some("https://reins.example/reins/org/approve/t"));
        assert!(!b.to_string().contains("approve/t") && b.to_string().contains("Open the Reins app"));
        let b = blocks(&msg(true), Some("https://reins.example/reins/org/approve/t"));
        assert!(b.to_string().contains("approve/t") && b.to_string().contains("Review"));
        let b = blocks(&msg(true), None);
        assert!(!b.to_string().contains("\"button\""));
    }

    fn cfg() -> SlackConfig {
        SlackConfig::from_value(&json!({"bot_token": "xoxb-1", "signing_secret": "s"})).unwrap()
    }

    #[tokio::test]
    async fn decisions_update_every_posted_message() {
        rustls::crypto::ring::default_provider().install_default().ok();
        let api = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/conversations.open"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok": true, "channel": {"id": "D1"}})))
            .mount(&api)
            .await;
        let mut n = 0;
        for ts in ["1.1", "1.2"] {
            n += 1;
            Mock::given(method("POST"))
                .and(path("/chat.postMessage"))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok": true, "ts": ts})))
                .up_to_n_times(1)
                .with_priority(n)
                .mount(&api)
                .await;
        }
        Mock::given(method("POST"))
            .and(path("/chat.update"))
            .and(body_partial_json(json!({"text": "Approved by kim@acme.com"})))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok": true})))
            .mount(&api)
            .await;
        let slack = Slack::with_base(cfg(), &api.uri());
        let a = slack.post("U1", &msg(true), Some("https://x/approve/t1")).await.unwrap();
        let b = slack.post("U2", &msg(true), Some("https://x/approve/t2")).await.unwrap();
        assert_eq!((a.as_str(), b.as_str()), ("D1:1.1", "D1:1.2"));
        for r in [&a, &b] {
            slack.update(r, "Approved by kim@acme.com").await.unwrap();
        }
        let updates =
            api.received_requests().await.unwrap().into_iter().filter(|r| r.url.path() == "/chat.update").count();
        assert_eq!(updates, 2);
        // No message body ever left: only the line.
        let posted: Vec<Value> = api
            .received_requests()
            .await
            .unwrap()
            .into_iter()
            .filter(|r| r.url.path() == "/chat.postMessage")
            .map(|r| serde_json::from_slice(&r.body).unwrap())
            .collect();
        assert!(posted.iter().all(|p| p["text"] == msg(true).line));
    }
}
