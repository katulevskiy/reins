//! Microsoft Teams, through the org's **own single-tenant** Azure Bot and Teams app (Microsoft no longer creates new
//! multi-tenant bots). A Connection admin posts `{app_id, app_secret, tenant_id, teams_app_id, service_url?}`, sealed;
//! a two-person change turns it on.
//!
//! - **Tokens:** client credentials at `login.microsoftonline.com/<tenant>`: `api.botframework.com/.default` for the
//!   connector, `graph.microsoft.com/.default` for Graph (`TeamsAppInstallation.ReadWriteSelfForUser.All`,
//!   `User.Read.All`).
//! - **Link:** Graph `GET /users/<email>`, install the org's Teams app for the member (an existing install is fine),
//!   then open a 1:1 conversation with the bot; the conversation is kept.
//! - **Post / update:** an Adaptive Card (the line, the deadline, `Action.OpenUrl` "Review" for L0–L2 only), and `PUT`
//!   of the activity with the outcome.
//! - **Inbound** `POST /reins/org/teams/messages` needs a valid Bot Framework JWT (issuer `https://api.botframework.com`,
//!   audience the org's bot); it keeps a `conversationUpdate`'s `serviceUrl` (Microsoft's hosts only) and decides
//!   nothing.

use std::{
    collections::HashMap,
    sync::{LazyLock, Mutex},
};

use rocket::{
    Route,
    data::{Data, ToByteUnit},
    http::Status,
    request::{FromRequest, Outcome, Request},
};
use serde_json::{Value, json};
use zeroize::Zeroizing;

use super::{ApprovalChannel, Message, api_base, due_text, http};
use crate::{
    api::reins::{
        now_unix,
        org::{OrgError, enabled},
    },
    db::{DbConn, models::reins_org_routing as store},
};

const LOGIN: &str = "https://login.microsoftonline.com";
const GRAPH: &str = "https://graph.microsoft.com/v1.0";
const CONNECTOR: &str = "https://smba.trafficmanager.net/teams";
const OPENID: &str = "https://login.botframework.com/v1/.well-known/openidconfiguration";
/// The issuer of Bot Framework tokens.
pub const ISSUER: &str = "https://api.botframework.com";
const BOT_SCOPE: &str = "https://api.botframework.com/.default";
const GRAPH_SCOPE: &str = "https://graph.microsoft.com/.default";

/// The org's bot.
#[derive(Clone)]
pub struct TeamsConfig {
    pub app_id: String,
    pub app_secret: Zeroizing<String>,
    pub tenant_id: String,
    pub teams_app_id: String,
    pub service_url: Option<String>,
}

fn id_ok(s: &str) -> bool {
    !s.is_empty() && s.len() <= 100 && s.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_'))
}

/// The Bot Framework connector hosts Teams uses (public, GCC, GCC High, DoD); a `serviceUrl` is taken only for these,
/// exactly.
pub const CONNECTOR_HOSTS: &[&str] = &[
    "smba.trafficmanager.net",
    "smba.infra.gcc.teams.microsoft.com",
    "smba.infra.gov.teams.microsoft.us",
    "smba.infra.dod.teams.microsoft.us",
];

/// A connector URL Microsoft runs: `https`, one of [`CONNECTOR_HOSTS`] exactly, no user or port.
#[must_use]
pub fn service_url_ok(url: &str) -> bool {
    url::Url::parse(url).is_ok_and(|u| {
        u.scheme() == "https"
            && u.username().is_empty()
            && u.port().is_none()
            && u.host_str().is_some_and(|h| CONNECTOR_HOSTS.contains(&h))
    })
}

/// Text for an Adaptive Card `TextBlock`, made literal: one line (no breaks), its markdown escaped (no emphasis, no
/// links, no lists), and nothing Teams would auto-link (`://`, `www.`).
#[must_use]
pub fn plain(text: &str) -> String {
    let one_line: String = text
        .chars()
        .map(|c| {
            if matches!(c, '\n' | '\r' | '\u{2028}' | '\u{2029}') {
                ' '
            } else {
                c
            }
        })
        .collect();
    let unlinked = one_line.replace("://", ": //").replace("www.", "www .").replace("WWW.", "WWW .");
    let mut out = String::with_capacity(unlinked.len() + 8);
    for (i, ch) in unlinked.chars().enumerate() {
        if matches!(ch, '\\' | '*' | '_' | '[' | ']' | '(' | ')' | '#' | '`' | '~' | '>' | '<' | '|' | '!')
            || (i == 0 && matches!(ch, '-' | '+'))
        {
            out.push('\\');
        }
        out.push(ch);
    }
    // A number then a dot at the start would be a list.
    if let Some(dot) = out.find('.')
        && dot > 0
        && out[..dot].chars().all(|c| c.is_ascii_digit())
    {
        out.insert(dot, '\\');
    }
    out
}

impl TeamsConfig {
    #[must_use]
    pub fn from_value(v: &Value) -> Option<Self> {
        let s = |k: &str| v.get(k).and_then(Value::as_str).map(str::trim).filter(|x| !x.is_empty());
        let service_url = s("service_url").map(str::to_owned);
        if service_url.as_deref().is_some_and(|u| !service_url_ok(u)) {
            return None;
        }
        Some(Self {
            // Bot ids compare lowercase everywhere (stored, claimed, and against a token's audience).
            app_id: s("app_id").filter(|x| id_ok(x))?.to_ascii_lowercase(),
            app_secret: Zeroizing::new(s("app_secret").filter(|x| x.len() <= 512)?.to_owned()),
            tenant_id: s("tenant_id").filter(|x| id_ok(x))?.to_owned(),
            teams_app_id: s("teams_app_id").filter(|x| id_ok(x))?.to_owned(),
            service_url,
        })
    }
}

/// Proof that an install holds a bot's secret: Microsoft issued a client-credentials token for its app id, tenant
/// and secret. Only [`verify`] (or tests) makes one.
pub struct Proof {
    app_id: String,
    tenant_id: String,
    secret_hash: String,
}

fn secret_hash(cfg: &TeamsConfig) -> String {
    reins_proto::canon::hex32(&reins_proto::canon::sha256(cfg.app_secret.as_bytes()))
}

impl Proof {
    /// Whether this proves `cfg` (the same bot, tenant and secret).
    #[must_use]
    pub fn proves(&self, cfg: &TeamsConfig) -> bool {
        self.app_id == cfg.app_id && self.tenant_id == cfg.tenant_id && self.secret_hash == secret_hash(cfg)
    }

    #[cfg(test)]
    #[must_use]
    pub fn for_tests(cfg: &TeamsConfig) -> Self {
        Self {
            app_id: cfg.app_id.clone(),
            tenant_id: cfg.tenant_id.clone(),
            secret_hash: secret_hash(cfg),
        }
    }
}

/// Asks Microsoft for a token with the posted id, tenant and secret: a proof the installer holds the bot.
pub async fn verify(cfg: TeamsConfig) -> Result<Proof, String> {
    let teams = Teams::new(cfg.clone());
    teams.token(BOT_SCOPE).await?;
    Ok(Proof {
        app_id: cfg.app_id.clone(),
        tenant_id: cfg.tenant_id.clone(),
        secret_hash: secret_hash(&cfg),
    })
}

/// The API bases (tests: loopback mocks).
#[derive(Clone)]
pub struct Bases {
    pub login: String,
    pub graph: String,
    pub connector: String,
}

impl Bases {
    #[must_use]
    pub fn live(service_url: Option<&str>) -> Self {
        Self {
            login: api_base(LOGIN, "REINS_TEST_MS_LOGIN_API"),
            graph: api_base(GRAPH, "REINS_TEST_MS_GRAPH_API"),
            connector: api_base(service_url.unwrap_or(CONNECTOR), "REINS_TEST_BOT_CONNECTOR_API"),
        }
    }
}

pub struct Teams {
    cfg: TeamsConfig,
    bases: Bases,
}

/// (login base, tenant, app id, SHA-256 of the app secret, scope) → (token, expires at): one install's token never
/// serves another's.
type Tokens = HashMap<(String, String), (Zeroizing<String>, i64)>;

static TOKENS: LazyLock<Mutex<Tokens>> = LazyLock::new(Mutex::default);

impl Teams {
    #[must_use]
    pub fn new(cfg: TeamsConfig) -> Self {
        let bases = Bases::live(cfg.service_url.as_deref());
        Self::with_bases(cfg, bases)
    }

    #[must_use]
    pub fn with_bases(cfg: TeamsConfig, mut bases: Bases) -> Self {
        for b in [&mut bases.login, &mut bases.graph, &mut bases.connector] {
            *b = b.trim_end_matches('/').to_owned();
        }
        Self {
            cfg,
            bases,
        }
    }

    async fn token(&self, scope: &str) -> Result<Zeroizing<String>, String> {
        let now = now_unix();
        let secret = reins_proto::canon::hex32(&reins_proto::canon::sha256(self.cfg.app_secret.as_bytes()));
        let key =
            (format!("{}|{}|{}|{secret}", self.bases.login, self.cfg.tenant_id, self.cfg.app_id), scope.to_owned());
        if let Some((t, exp)) = TOKENS.lock().unwrap_or_else(std::sync::PoisonError::into_inner).get(&key)
            && *exp > now
        {
            return Ok(t.clone());
        }
        let resp = http()?
            .post(format!("{}/{}/oauth2/v2.0/token", self.bases.login, self.cfg.tenant_id))
            .form(&[
                ("grant_type", "client_credentials"),
                ("client_id", self.cfg.app_id.as_str()),
                ("client_secret", self.cfg.app_secret.as_str()),
                ("scope", scope),
            ])
            .send()
            .await
            .map_err(|e| format!("Microsoft sign-in: {e}"))?;
        if !resp.status().is_success() {
            return Err(format!("Microsoft sign-in: {}", resp.status()));
        }
        let v: Value = resp.json().await.map_err(|e| e.to_string())?;
        let t = Zeroizing::new(v["access_token"].as_str().ok_or("no token")?.to_owned());
        let life = v["expires_in"].as_i64().unwrap_or(3600);
        TOKENS.lock().unwrap_or_else(std::sync::PoisonError::into_inner).insert(key, (t.clone(), now + life - 60));
        Ok(t)
    }

    async fn send(&self, req: reqwest::RequestBuilder, scope: &str) -> Result<(reqwest::StatusCode, Value), String> {
        let token = self.token(scope).await?;
        let resp = req.bearer_auth(token.as_str()).send().await.map_err(|e| e.to_string())?;
        let status = resp.status();
        Ok((status, resp.json().await.unwrap_or(Value::Null)))
    }
}

/// The Adaptive Card of a review: the line, the deadline, and "Review" only with a link (L0–L2).
#[must_use]
pub fn card(msg: &Message, url: Option<&str>) -> Value {
    let mut body = vec![
        json!({"type": "TextBlock", "text": plain(&msg.line), "wrap": true}),
        json!({"type": "TextBlock", "text": due_text(msg.due), "isSubtle": true, "spacing": "Small"}),
    ];
    let actions = match url.filter(|_| msg.web) {
        Some(u) => vec![json!({"type": "Action.OpenUrl", "title": "Review", "url": u})],
        None => {
            body.push(json!({"type": "TextBlock", "text": "Open the Reins app", "isSubtle": true}));
            Vec::new()
        }
    };
    json!({"type": "AdaptiveCard", "version": "1.4", "body": body, "actions": actions,
           "$schema": "http://adaptivecards.io/schemas/adaptive-card.json"})
}

/// A message with only the card (no `text`: nothing outside the card is rendered as markdown).
fn activity(content: &Value) -> Value {
    json!({"type": "message",
           "attachments": [{"contentType": "application/vnd.microsoft.card.adaptive", "content": content}]})
}

#[rocket::async_trait]
impl ApprovalChannel for Teams {
    async fn link(&self, email: &str) -> Result<String, String> {
        let g = &self.bases.graph;
        let enc: String = url::form_urlencoded::byte_serialize(email.as_bytes()).collect();
        let (status, user) = self.send(http()?.get(format!("{g}/users/{enc}")), GRAPH_SCOPE).await?;
        let aad = user["id"].as_str().filter(|_| status.is_success()).ok_or("no such Teams user")?.to_owned();
        let install = json!({"teamsApp@odata.bind": format!("{g}/appCatalogs/teamsApps/{}", self.cfg.teams_app_id)});
        let (status, _) = self
            .send(http()?.post(format!("{g}/users/{aad}/teamwork/installedApps")).json(&install), GRAPH_SCOPE)
            .await?;
        // 409: already installed.
        if !status.is_success() && status.as_u16() != 409 {
            return Err(format!("Teams app install: {status}"));
        }
        let conv = json!({"isGroup": false, "bot": {"id": format!("28:{}", self.cfg.app_id)},
                          "members": [{"id": aad}], "channelData": {"tenant": {"id": self.cfg.tenant_id}}});
        let (status, v) = self
            .send(http()?.post(format!("{}/v3/conversations", self.bases.connector)).json(&conv), BOT_SCOPE)
            .await?;
        v["id"]
            .as_str()
            .filter(|_| status.is_success())
            .map(str::to_owned)
            .ok_or_else(|| format!("Teams conversation: {status}"))
    }

    async fn post(&self, to: &str, msg: &Message, url: Option<&str>) -> Result<String, String> {
        let body = activity(&card(msg, url));
        let req = http()?.post(format!("{}/v3/conversations/{to}/activities", self.bases.connector)).json(&body);
        let (status, v) = self.send(req, BOT_SCOPE).await?;
        let id = v["id"].as_str().filter(|_| status.is_success()).ok_or_else(|| format!("Teams post: {status}"))?;
        Ok(format!("{to}|{id}"))
    }

    async fn update(&self, external_ref: &str, outcome: &str) -> Result<(), String> {
        let (conv, id) = external_ref.rsplit_once('|').ok_or("bad reference")?;
        let content = json!({"type": "AdaptiveCard", "version": "1.4",
                             "body": [{"type": "TextBlock", "text": plain(outcome), "wrap": true}]});
        let req = http()?
            .put(format!("{}/v3/conversations/{conv}/activities/{id}", self.bases.connector))
            .json(&activity(&content));
        let (status, _) = self.send(req, BOT_SCOPE).await?;
        if status.is_success() {
            Ok(())
        } else {
            Err(format!("Teams update: {status}"))
        }
    }
}

/// Validates a Bot Framework token for `activity` against `jwks` (the OpenID metadata's keys): RS256, issuer
/// [`ISSUER`], audience one of `app_ids`, not expired, a signing key endorsed for the activity's `channelId`, and a
/// `serviceUrl` claim equal to the activity's. Returns the audience (the org's bot).
pub fn validate_token(token: &str, jwks: &Value, app_ids: &[String], activity: &Value) -> Result<String, String> {
    use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode, decode_header};
    if app_ids.is_empty() {
        return Err("no bot".to_owned());
    }
    let header = decode_header(token).map_err(|e| e.to_string())?;
    if header.alg != Algorithm::RS256 {
        return Err("not RS256".to_owned());
    }
    let kid = header.kid.ok_or("no key id")?;
    let key =
        jwks["keys"].as_array().into_iter().flatten().find(|k| k["kid"].as_str() == Some(&kid)).ok_or("unknown key")?;
    let (Some(n), Some(e)) = (key["n"].as_str(), key["e"].as_str()) else {
        return Err("not an RSA key".to_owned());
    };
    let channel = activity["channelId"].as_str().ok_or("no channel")?;
    if !key["endorsements"].as_array().is_some_and(|e| e.iter().any(|x| x.as_str() == Some(channel))) {
        return Err("key not endorsed for this channel".to_owned());
    }
    let dk = DecodingKey::from_rsa_components(n, e).map_err(|e| e.to_string())?;
    let mut v = Validation::new(Algorithm::RS256);
    v.set_issuer(&[ISSUER]);
    // The audience is checked below, case-insensitively, against the (lowercase) bot ids.
    v.validate_aud = false;
    v.leeway = 300;
    let data = decode::<Value>(token, &dk, &v).map_err(|e| e.to_string())?;
    let aud = data.claims["aud"].as_str().map(str::to_ascii_lowercase).ok_or("no audience")?;
    if !app_ids.iter().any(|a| a.eq_ignore_ascii_case(&aud)) {
        return Err("another bot".to_owned());
    }
    let claimed = data.claims["serviceUrl"].as_str().map(|u| u.trim_end_matches('/'));
    if claimed != activity["serviceUrl"].as_str().map(|u| u.trim_end_matches('/')) {
        return Err("serviceUrl doesn't match the token".to_owned());
    }
    Ok(aud)
}

/// The Bot Framework's signing keys (cached a day).
static KEYS: LazyLock<Mutex<Option<(i64, Value)>>> = LazyLock::new(Mutex::default);

async fn signing_keys() -> Result<Value, String> {
    let now = now_unix();
    if let Some((at, keys)) = KEYS.lock().unwrap_or_else(std::sync::PoisonError::into_inner).as_ref()
        && now - at < 86_400
    {
        return Ok(keys.clone());
    }
    let meta: Value = http()?
        .get(api_base(OPENID, "REINS_TEST_BOT_OPENID"))
        .send()
        .await
        .map_err(|e| e.to_string())?
        .json()
        .await
        .map_err(|e| e.to_string())?;
    let jwks_uri = meta["jwks_uri"].as_str().ok_or("no jwks_uri")?;
    let keys: Value =
        http()?.get(jwks_uri).send().await.map_err(|e| e.to_string())?.json().await.map_err(|e| e.to_string())?;
    *KEYS.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some((now, keys.clone()));
    Ok(keys)
}

pub struct Bearer(String);

#[rocket::async_trait]
impl<'r> FromRequest<'r> for Bearer {
    type Error = ();

    async fn from_request(req: &'r Request<'_>) -> Outcome<Self, ()> {
        match req.headers().get_one("Authorization").and_then(|h| h.strip_prefix("Bearer ")) {
            Some(t) => Outcome::Success(Self(t.trim().to_owned())),
            None => Outcome::Error((Status::Unauthorized, ())),
        }
    }
}

pub fn routes() -> Vec<Route> {
    routes![messages]
}

/// The orgs' bots: (org, config).
pub fn bots(c: &mut crate::db::DbConnInner) -> Result<Vec<(String, TeamsConfig)>, OrgError> {
    let mut out = Vec::new();
    for row in store::q_installed_of_kind(c, "teams")? {
        if let Some(cfg) = super::config(c, &row.org_uuid, reins_proto::org::ChannelKind::Teams, true)?
            && let Some(cfg) = serde_json::from_str::<Value>(&cfg).ok().and_then(|v| TeamsConfig::from_value(&v))
        {
            out.push((row.org_uuid, cfg));
        }
    }
    Ok(out)
}

/// What an activity changes: a `conversationUpdate`'s `serviceUrl` (Microsoft's hosts only), else nothing.
#[must_use]
pub fn service_url_of(activity: &Value) -> Option<String> {
    (activity["type"] == "conversationUpdate")
        .then(|| activity["serviceUrl"].as_str().map(|u| u.trim_end_matches('/').to_owned()))
        .flatten()
        .filter(|u| service_url_ok(u))
}

#[post("/reins/org/teams/messages", data = "<data>")]
async fn messages(bearer: Bearer, data: Data<'_>, conn: DbConn) -> Status {
    if !enabled() {
        return Status::NotFound;
    }
    let Ok(body) = data.open(64.kibibytes()).into_bytes().await else {
        return Status::BadRequest;
    };
    let Ok(keys) = signing_keys().await else {
        return Status::ServiceUnavailable;
    };
    let Ok(bots) = conn.run(bots).await else {
        return Status::InternalServerError;
    };
    let ids: Vec<String> = bots.iter().map(|(_, c)| c.app_id.clone()).collect();
    let activity: Value = serde_json::from_slice(&body).unwrap_or_default();
    let Ok(aud) = validate_token(&bearer.0, &keys, &ids, &activity) else {
        return Status::Unauthorized;
    };
    // An app id is one org's (enforced at install): exactly one bot matches, or nothing changes.
    let mut mine = bots.into_iter().filter(|(_, c)| c.app_id == aud);
    let (Some(bot), None) = (mine.next(), mine.next()) else {
        return Status::Ok;
    };
    if let Some(url) = service_url_of(&activity)
        && let (org, cfg) = bot
        && cfg.service_url.as_deref() != Some(url.as_str())
    {
        let v = json!({"app_id": cfg.app_id, "app_secret": cfg.app_secret.as_str(), "tenant_id": cfg.tenant_id,
                       "teams_app_id": cfg.teams_app_id, "service_url": url});
        let now = now_unix();
        drop(conn.run(move |c| super::install(c, &org, reins_proto::org::ChannelKind::Teams, &v, None, now)).await);
    }
    Status::Ok
}

#[cfg(test)]
mod tests {
    use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
    use wiremock::matchers::{body_string_contains, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;

    fn cfg() -> TeamsConfig {
        TeamsConfig::from_value(&json!({"app_id": "bot-1", "app_secret": "s", "tenant_id": "t-1",
                                        "teams_app_id": "app-1"}))
        .unwrap()
    }

    async fn microsoft() -> (MockServer, Teams) {
        rustls::crypto::ring::default_provider().install_default().ok();
        let ms = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/t-1/oauth2/v2.0/token"))
            .and(body_string_contains("grant_type=client_credentials"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"access_token": "tok", "expires_in": 3600})))
            .mount(&ms)
            .await;
        let bases = Bases {
            login: ms.uri(),
            graph: format!("{}/graph", ms.uri()),
            connector: format!("{}/bot", ms.uri()),
        };
        (ms, Teams::with_bases(cfg(), bases))
    }

    #[tokio::test]
    async fn members_are_linked_by_email_and_installed() {
        let (ms, teams) = microsoft().await;
        Mock::given(method("GET"))
            .and(path("/graph/users/kim%40acme.com"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": "aad-kim"})))
            .mount(&ms)
            .await;
        Mock::given(method("POST"))
            .and(path("/graph/users/aad-kim/teamwork/installedApps"))
            .respond_with(ResponseTemplate::new(409))
            .mount(&ms)
            .await;
        Mock::given(method("POST"))
            .and(path("/bot/v3/conversations"))
            .and(body_string_contains("aad-kim"))
            .respond_with(ResponseTemplate::new(201).set_body_json(json!({"id": "a:conv-kim"})))
            .mount(&ms)
            .await;
        assert_eq!(teams.link("kim@acme.com").await.unwrap(), "a:conv-kim");
        // Unknown in the tenant: no link.
        assert!(teams.link("nobody@acme.com").await.is_err());
    }

    fn msg(web: bool) -> Message {
        Message {
            line: "lee@acme.com · Claude · GitPush acme/prod · L2".into(),
            due: 0,
            web,
        }
    }

    #[test]
    fn l3_cards_have_no_review_link() {
        let c = card(&msg(false), Some("https://x/approve/t"));
        assert!(c["actions"].as_array().unwrap().is_empty() && !c.to_string().contains("approve/t"));
        let c = card(&msg(true), Some("https://x/approve/t"));
        assert_eq!(c["actions"][0]["url"], "https://x/approve/t");
    }

    #[tokio::test]
    async fn decisions_update_the_card() {
        let (ms, teams) = microsoft().await;
        Mock::given(method("POST"))
            .and(path("/bot/v3/conversations/a:conv/activities"))
            .respond_with(ResponseTemplate::new(201).set_body_json(json!({"id": "act-1"})))
            .mount(&ms)
            .await;
        Mock::given(method("PUT"))
            .and(path("/bot/v3/conversations/a:conv/activities/act-1"))
            .and(body_string_contains("Expired"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": "act-1"})))
            .mount(&ms)
            .await;
        let r = teams.post("a:conv", &msg(true), Some("https://x/approve/t")).await.unwrap();
        assert_eq!(r, "a:conv|act-1");
        teams.update(&r, "Expired").await.unwrap();
        let reqs = ms.received_requests().await.unwrap();
        assert_eq!(reqs.iter().filter(|r| r.method.as_str() == "PUT").count(), 1);
        assert!(
            reqs.iter().filter(|r| r.url.path().starts_with("/bot")).all(|r| r
                .headers
                .get("authorization")
                .and_then(|h| h.to_str().ok())
                == Some("Bearer tok"))
        );
    }

    #[test]
    fn activities_need_a_valid_bot_framework_token() {
        let rsa = openssl::rsa::Rsa::generate(2048).unwrap();
        let pem = rsa.private_key_to_pem().unwrap();
        let b64 = data_encoding::BASE64URL_NOPAD;
        let key = |kid: &str, endorsed: &str| {
            json!({"kty": "RSA", "kid": kid, "n": b64.encode(&rsa.n().to_vec()), "e": b64.encode(&rsa.e().to_vec()),
                   "endorsements": [endorsed]})
        };
        let jwks = json!({"keys": [key("k1", "msteams"), key("k9", "skype")]});
        let now = now_unix();
        let svc = "https://smba.trafficmanager.net/emea/";
        let token = |iss: &str, aud: &str, exp: i64, kid: &str, key: &[u8], service: &str| {
            let mut h = Header::new(Algorithm::RS256);
            h.kid = Some(kid.to_owned());
            let claims = json!({"iss": iss, "aud": aud, "exp": exp, "nbf": now - 10, "serviceUrl": service});
            encode(&h, &claims, &EncodingKey::from_rsa_pem(key).unwrap()).unwrap()
        };
        let good = |kid: &str| token(ISSUER, "bot-1", now + 600, kid, &pem, svc);
        let act = json!({"type": "conversationUpdate", "channelId": "msteams", "serviceUrl": svc});
        let bots = vec!["bot-1".to_owned()];
        assert_eq!(validate_token(&good("k1"), &jwks, &bots, &act).unwrap(), "bot-1");
        // Another issuer, another bot, expired, an unknown key, another signer: refused.
        let bad = [
            token("https://evil.example", "bot-1", now + 600, "k1", &pem, svc),
            token(ISSUER, "bot-2", now + 600, "k1", &pem, svc),
            token(ISSUER, "bot-1", now - 3600, "k1", &pem, svc),
            token(ISSUER, "bot-1", now + 600, "k2", &pem, svc),
            token(
                ISSUER,
                "bot-1",
                now + 600,
                "k1",
                &openssl::rsa::Rsa::generate(2048).unwrap().private_key_to_pem().unwrap(),
                svc,
            ),
            "not.a.jwt".to_owned(),
        ];
        for t in &bad {
            assert!(validate_token(t, &jwks, &bots, &act).is_err());
        }
        assert!(validate_token(&good("k1"), &jwks, &[], &act).is_err(), "no bot");
        // A key endorsed for another channel, or a serviceUrl the token doesn't carry: refused.
        assert!(validate_token(&good("k9"), &jwks, &bots, &act).is_err(), "endorsement");
        let moved = json!({"type": "conversationUpdate", "channelId": "msteams", "serviceUrl": "https://smba.trafficmanager.net/amer/"});
        assert!(validate_token(&good("k1"), &jwks, &bots, &moved).is_err(), "serviceUrl claim");
        // Activities change nothing but a Microsoft serviceUrl.
        assert_eq!(service_url_of(&act), Some("https://smba.trafficmanager.net/emea".to_owned()));
        let msg = json!({"type": "message", "text": "approve", "serviceUrl": svc});
        assert_eq!(service_url_of(&msg), None);
    }

    #[test]
    fn only_exact_connector_hosts_are_kept() {
        assert!(service_url_ok("https://smba.trafficmanager.net/teams/"));
        assert!(service_url_ok("https://smba.infra.gcc.teams.microsoft.com/teams"));
        for bad in [
            "https://evil.trafficmanager.net/x",
            "https://attacker.botframework.com/",
            "http://smba.trafficmanager.net/teams",
            "https://smba.trafficmanager.net:8443/teams",
            "https://user@smba.trafficmanager.net/teams",
            "https://smba.trafficmanager.net.evil.example/",
        ] {
            assert!(!service_url_ok(bad), "{bad}");
        }
        assert!(
            TeamsConfig::from_value(&json!({"app_id": "b", "app_secret": "s", "tenant_id": "t", "teams_app_id": "a",
                                                "service_url": "https://evil.trafficmanager.net/"}))
            .is_none()
        );
    }

    #[test]
    fn card_text_is_literal() {
        let line = "lee@acme.com · [Click here](https://evil.example) · **urgent** · 1. `x`";
        let c = card(
            &Message {
                line: line.into(),
                due: 0,
                web: true,
            },
            None,
        );
        let text = c["body"][0]["text"].as_str().unwrap();
        assert!(!text.contains("[Click here](") && text.contains("\\[Click here\\]\\("), "{text}");
        assert!(text.contains("\\*\\*urgent\\*\\*"));
        assert_eq!(plain("1. first"), "1\\. first");
        assert_eq!(plain("- item"), "\\- item");
        // Only the card is sent: no markdown `text` beside it.
        assert!(activity(&c).get("text").is_none());
    }

    #[tokio::test]
    async fn tokens_are_per_install() {
        let (ms, teams) = microsoft().await;
        Mock::given(method("POST"))
            .and(path("/t-2/oauth2/v2.0/token"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!({"access_token": "tok-2", "expires_in": 3600})),
            )
            .mount(&ms)
            .await;
        // The same app id in another tenant (or with another secret) never reuses the first one's token.
        let other = TeamsConfig::from_value(&json!({"app_id": "bot-1", "app_secret": "s2", "tenant_id": "t-2",
                                                   "teams_app_id": "app-1"}))
        .unwrap();
        let other = Teams::with_bases(
            other,
            Bases {
                login: ms.uri(),
                graph: ms.uri(),
                connector: ms.uri(),
            },
        );
        assert_eq!(teams.token(BOT_SCOPE).await.unwrap().as_str(), "tok");
        assert_eq!(other.token(BOT_SCOPE).await.unwrap().as_str(), "tok-2");
        assert_eq!(teams.token(BOT_SCOPE).await.unwrap().as_str(), "tok", "cached for its own install");
    }

    #[test]
    fn card_text_is_one_line_without_links() {
        let p = plain("ok\n- item\r\n2. two\nsee https://evil.example or www.evil.example");
        assert!(!p.contains('\n') && !p.contains('\r'), "{p}");
        assert!(!p.contains("://") && !p.contains("www."), "{p}");
        // Line starts are gone with the breaks, so nothing after them becomes a list.
        assert!(p.contains("ok - item  2. two"), "{p}");
        assert_eq!(plain("+ x"), "\\+ x");
    }

    #[tokio::test]
    async fn an_install_proves_it_holds_the_bot() {
        rustls::crypto::ring::default_provider().install_default().ok();
        let ms = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/t-7/oauth2/v2.0/token"))
            .and(body_string_contains("client_secret=right"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"access_token": "tok", "expires_in": 3600})))
            .mount(&ms)
            .await;
        Mock::given(method("POST"))
            .and(path("/t-7/oauth2/v2.0/token"))
            .respond_with(ResponseTemplate::new(401))
            .mount(&ms)
            .await;
        let cfg = |secret: &str| {
            TeamsConfig::from_value(
                &json!({"app_id": "bot-7", "app_secret": secret, "tenant_id": "t-7", "teams_app_id": "a"}),
            )
            .unwrap()
        };
        let bases = || Bases {
            login: ms.uri(),
            graph: ms.uri(),
            connector: ms.uri(),
        };
        assert!(Teams::with_bases(cfg("wrong"), bases()).token(BOT_SCOPE).await.is_err());
        assert!(Teams::with_bases(cfg("right"), bases()).token(BOT_SCOPE).await.is_ok());
        // A proof is for its own bot, tenant and secret only.
        let p = Proof::for_tests(&cfg("right"));
        assert!(p.proves(&cfg("right")) && !p.proves(&cfg("wrong")));
    }
}
