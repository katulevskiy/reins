//! Reins server: MCP endpoint, OAuth 2.1 authorization server for AI clients,
//! phone API and in-memory relay (spec §4, contracts §A and §C).

pub mod account_delete;
pub mod account_reset;
pub mod account_state;
pub mod apns;
pub mod blob;
pub mod blob_io;
pub mod blob_routes;
pub mod desktop_routes;
pub mod device_api;
pub mod device_flow;
pub mod fcm;
pub mod join;
pub mod limits;
pub mod mcp;
pub mod mcp_routes;
pub mod mcp_tools;
pub mod oauth;
pub mod oauth_routes;
pub mod oauth_state;
pub mod outbound;
pub mod pages;
pub mod pairing;
pub mod proxy_call;
pub mod push;
pub mod relay;
pub mod sniff;
pub mod tools;
pub mod ttl;
pub mod vault_passkeys;
pub mod workos_sync;

use std::{
    sync::{Arc, LazyLock},
    time::Duration,
};

use reins_proto::{device::Pending, remote_mcp::McpServerReport};
use rocket::{Catcher, Route};

use self::{
    blob::{BlobHub, BlobLimits},
    join::JoinHub,
    pairing::PairingHub,
    relay::{ItemSignal, RelayHub},
};
use crate::CONFIG;

/// Lifetime of relay requests, pairings and late results (contracts §A).
pub const ITEM_TTL: Duration = Duration::from_secs(600);
/// Lifetime of an authorize-page session (spec §4.1).
pub const SESSION_TTL: Duration = Duration::from_secs(300);
/// Lifetime of an authorization code (spec §4.5).
pub const CODE_TTL: Duration = Duration::from_secs(60);
/// How long a fetched CIMD document is trusted (spec §4.5).
pub const CIMD_TTL: Duration = Duration::from_secs(3600);
/// MCP access token lifetime (seconds).
pub const ACCESS_TOKEN_SECS: i64 = 3600;
/// MCP refresh token lifetime (seconds): 30 days.
pub const REFRESH_TOKEN_SECS: i64 = 30 * 24 * 3600;
/// Upper bound for `REINS_RELAY_WAIT_SECS` (ChatGPT's hard tool timeout is 60 s).
pub const MAX_RELAY_WAIT_SECS: u64 = 55;

/// Relay timing (spec §4.3). Tests construct it directly.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Timing {
    /// Maximum time a tool call waits for the phone's answer.
    pub relay_wait: Duration,
    /// A request not delivered to the phone within this time is reported as offline.
    pub offline: Duration,
}

impl Timing {
    pub fn from_config() -> Self {
        Self {
            relay_wait: Duration::from_secs(CONFIG.reins_relay_wait_secs()),
            offline: Duration::from_secs(CONFIG.reins_offline_secs()),
        }
    }
}

/// All in-memory relay state (requests, results, pairings) of this server instance.
pub struct Hub {
    pub signal: Arc<ItemSignal>,
    pub relay: RelayHub,
    pub pairings: PairingHub,
    /// Files held for one operation (files spec, S1).
    pub blobs: BlobHub,
    /// Phones asking the approval device for the account secret.
    pub joins: JoinHub,
    /// Per user: the integrations that have an account on the approval device (as the phone last reported them).
    /// Unknown (the phone has not reported since the server started) means every tool is listed.
    services: std::sync::Mutex<std::collections::HashMap<String, Vec<String>>>,
    /// Per user: the MCP servers added on the phone, as last reported (validated).
    mcp_servers: std::sync::Mutex<std::collections::HashMap<String, Arc<Vec<McpServerReport>>>>,
}

impl Hub {
    /// A hub with the default file quotas (tests).
    #[cfg(test)]
    pub fn new(timing: Timing) -> Self {
        Self::with_limits(timing, relay::DEFAULT_MAX_QUEUED, BlobLimits::default())
    }

    /// `max_queued`: unanswered relay requests per account (`0`: unlimited).
    pub fn with_limits(timing: Timing, max_queued: usize, blob_limits: BlobLimits) -> Self {
        let signal = Arc::new(ItemSignal::new());
        Self {
            relay: RelayHub::new(timing, Arc::clone(&signal)).with_max_queued(max_queued),
            pairings: PairingHub::new(Arc::clone(&signal)),
            blobs: BlobHub::with_limits(Arc::clone(&signal), blob_limits),
            joins: JoinHub::new(Arc::clone(&signal)),
            signal,
            services: std::sync::Mutex::default(),
            mcp_servers: std::sync::Mutex::default(),
        }
    }

    pub fn set_services(&self, user: &str, services: Vec<String>) {
        self.services.lock().unwrap_or_else(std::sync::PoisonError::into_inner).insert(user.to_owned(), services);
    }

    /// The integrations `user`'s phone reported, if it has.
    pub fn services_of(&self, user: &str) -> Option<Vec<String>> {
        self.services.lock().unwrap_or_else(std::sync::PoisonError::into_inner).get(user).cloned()
    }

    /// Replaces `user`'s MCP servers; reports that break the contract's bounds are dropped.
    pub fn set_mcp_servers(&self, user: &str, reports: Vec<McpServerReport>) {
        let reports = Arc::new(mcp_tools::sanitize_reports(reports));
        self.mcp_servers.lock().unwrap_or_else(std::sync::PoisonError::into_inner).insert(user.to_owned(), reports);
    }

    /// The MCP servers `user`'s phone reported (none until it has).
    pub fn mcp_servers_of(&self, user: &str) -> Arc<Vec<McpServerReport>> {
        self.mcp_servers
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(user)
            .cloned()
            .unwrap_or_default()
    }

    /// A2: undelivered requests, pairings and uploads awaiting a decision of `user`; if there are none, waits up to
    /// `wait` and returns as soon as one appears. Returned items are marked delivered.
    pub async fn pending(&self, user: &str, wait: Duration) -> Pending {
        let deadline = tokio::time::Instant::now() + wait;
        let mut rx = self.signal.subscribe();
        // The phone polls often: a cheap moment to delete expired files between the hourly purges.
        self.blobs.purge(now_unix());
        loop {
            let pending = Pending {
                account_email: None,
                requests: self.relay.take_undelivered(user),
                pairings: self.pairings.take_undelivered(user),
                blobs: self.blobs.take_undelivered(user, now_unix()),
                joins: self.joins.take_undelivered(user, now_unix()),
            };
            if !pending.is_empty() || tokio::time::Instant::now() >= deadline {
                return pending;
            }
            if matches!(tokio::time::timeout_at(deadline, rx.changed()).await, Ok(Err(_))) {
                tokio::time::sleep_until(deadline).await;
            }
        }
    }

    /// Drops what the hub remembers about a deleted account: its reported integrations and MCP servers, the requests,
    /// pairings and "add another phone" requests waiting for it, and its files (deleted from disk).
    pub fn forget_user(&self, user: &str) {
        self.services.lock().unwrap_or_else(std::sync::PoisonError::into_inner).remove(user);
        self.mcp_servers.lock().unwrap_or_else(std::sync::PoisonError::into_inner).remove(user);
        self.relay.forget_user(user);
        self.pairings.forget_user(user);
        self.joins.forget_user(user);
        self.blobs.remove_user(user);
    }

    pub fn purge(&self) {
        self.relay.purge();
        self.pairings.purge();
        self.blobs.purge(now_unix());
        self.joins.purge(now_unix());
    }
}

/// The process-wide hub (spec §2: one server instance per deployment).
pub static HUB: LazyLock<Hub> = LazyLock::new(|| {
    let max_queued = usize::try_from(CONFIG.reins_account_max_queued()).unwrap_or(usize::MAX);
    Hub::with_limits(Timing::from_config(), max_queued, BlobLimits::from_config())
});

pub fn enabled() -> bool {
    CONFIG.reins_enabled()
}

/// Scheduled housekeeping: expired refresh tokens and in-memory entries.
pub async fn purge(pool: crate::db::DbPool) {
    debug!("Purging Reins state");
    HUB.purge();
    oauth_state::OAUTH.purge();
    device_flow::DEVICE_GRANTS.purge();
    limits::retain_recent();
    if let Ok(conn) = pool.get().await {
        if let Err(e) = crate::db::models::ReinsRefreshToken::delete_expired(now_unix(), &conn).await {
            error!("Failed to purge Reins refresh tokens: {e:?}");
        }
    } else {
        error!("Failed to get DB connection while purging Reins state");
    }
}

/// Scheduled job: deletes expired files from disk (every minute by default; the hourly purge is too coarse for files
/// of up to 1 GiB that live at most an hour).
pub fn purge_blobs() {
    HUB.blobs.purge(now_unix());
}

/// Current time as unix seconds (the unit of every Reins timestamp).
pub fn now_unix() -> i64 {
    chrono::Utc::now().timestamp()
}

/// Routes mounted at `{domain_path}/` (they carry their full paths: `/mcp`, `/reins/...`).
pub fn routes() -> Vec<Route> {
    if !enabled() {
        return Vec::new();
    }
    // Called once at launch: files held by a previous run are deleted now, not when the next file arrives.
    if let Err(e) = HUB.blobs.prepare() {
        error!("Reins blob store unavailable: {e}");
    }
    warn_about_public_settings();
    let mut routes = device_api::routes();
    routes.extend(account_state::routes());
    routes.extend(account_delete::routes());
    routes.extend(account_reset::routes());
    routes.extend(oauth_routes::routes());
    routes.extend(mcp_routes::routes());
    routes.extend(desktop_routes::routes());
    routes.extend(blob_routes::routes());
    routes.extend(proxy_call::routes());
    routes.extend(pages::routes());
    routes.extend(join::routes());
    routes.extend(workos_sync::routes());
    routes.extend(vault_passkeys::routes());
    routes
}

/// Settings that are fine on a private server but not on a public one, named once at launch.
fn warn_about_public_settings() {
    if outbound::allow_loopback() {
        warn!(
            "`{}` is set: Reins may send requests to http://127.0.0.1. This is for tests only; never set it on a \
             real server",
            outbound::ALLOW_LOOPBACK_ENV
        );
    }
    let public = url::Url::parse(&CONFIG.domain()).is_ok_and(|u| match u.host() {
        Some(url::Host::Domain(d)) => !d.eq_ignore_ascii_case("localhost"),
        Some(url::Host::Ipv4(ip)) => !ip.is_loopback(),
        Some(url::Host::Ipv6(ip)) => !ip.is_loopback(),
        None => false,
    });
    if public && CONFIG.signups_allowed() && !CONFIG.signups_verify() {
        warn!(
            "Reins is enabled with open sign-ups that need no email verification: anyone can create accounts \
             (each with its own file and call quotas). On a public server set SIGNUPS_VERIFY=true (needs SMTP) or \
             restrict SIGNUPS_ALLOWED / SIGNUPS_DOMAINS_WHITELIST"
        );
    }
}

/// The request path as it may be logged: a blob capability URL loses its secret.
pub fn loggable_path(path: &str) -> std::borrow::Cow<'_, str> {
    match path.find(blob_routes::PUBLIC_PREFIX) {
        Some(at) => format!("{}{}…", &path[..at], blob_routes::PUBLIC_PREFIX).into(),
        None => path.into(),
    }
}

/// OAuth/RFC 9728 discovery documents and the phone apps' link associations, mounted at the server root `/`.
pub fn well_known_routes() -> Vec<Route> {
    if !enabled() {
        return Vec::new();
    }
    let mut routes = oauth_routes::well_known_routes();
    routes.extend(pages::app_link_routes());
    routes
}

/// Catchers registered at `{domain_path}/reins/api`.
pub fn catchers() -> Vec<Catcher> {
    if !enabled() {
        return Vec::new();
    }
    device_api::catchers()
}

/// Cross-field checks for the `reins` config group; only called when Reins is enabled.
pub fn validate_settings(
    domain: &str,
    domain_set: bool,
    wait_secs: u64,
    offline_secs: u64,
    fcm_path: &str,
    apns: &apns::Settings,
) -> Result<(), String> {
    if !domain_set {
        return Err("`REINS_ENABLED` requires `DOMAIN` to be set to the public URL of this server".to_owned());
    }
    let url = url::Url::parse(domain).map_err(|e| format!("`DOMAIN` is not a valid URL: {e}"))?;
    let loopback = match url.host() {
        Some(url::Host::Domain(d)) => d.eq_ignore_ascii_case("localhost"),
        Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
        Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
        None => false,
    };
    if url.scheme() != "https" && !(url.scheme() == "http" && loopback) {
        return Err(
            "`REINS_ENABLED` requires an https:// `DOMAIN` (http is only allowed for localhost, 127.0.0.1 and [::1])"
                .to_owned(),
        );
    }
    if !(1..=MAX_RELAY_WAIT_SECS).contains(&wait_secs) {
        return Err(format!(
            "`REINS_RELAY_WAIT_SECS` must be between 1 and {MAX_RELAY_WAIT_SECS} (ChatGPT aborts tool calls after 60 s)"
        ));
    }
    if offline_secs == 0 || offline_secs > wait_secs {
        return Err("`REINS_OFFLINE_SECS` must be at least 1 and at most `REINS_RELAY_WAIT_SECS`".to_owned());
    }
    if !fcm_path.is_empty() {
        fcm::ServiceAccount::from_file(fcm_path).map_err(|e| format!("`REINS_FCM_SERVICE_ACCOUNT`: {e}"))?;
    }
    apns.load()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const OK_DOMAIN: &str = "https://reins.example.com";
    const NO_APNS: apns::Settings = apns::Settings {
        key_file: String::new(),
        key_id: String::new(),
        team_id: String::new(),
        topic: String::new(),
    };

    #[test]
    fn accepts_defaults() {
        assert_eq!(validate_settings(OK_DOMAIN, true, 45, 10, "", &NO_APNS), Ok(()));
        assert_eq!(validate_settings("http://127.0.0.1:8000", true, 45, 10, "", &NO_APNS), Ok(()));
        assert_eq!(validate_settings("http://localhost:8000/vw", true, 4, 2, "", &NO_APNS), Ok(()));
        assert_eq!(validate_settings("http://[::1]:8000", true, 55, 55, "", &NO_APNS), Ok(()));
    }

    #[test]
    fn requires_explicit_https_domain() {
        assert!(validate_settings(OK_DOMAIN, false, 45, 10, "", &NO_APNS).unwrap_err().contains("DOMAIN"));
        assert!(
            validate_settings("http://reins.example.com", true, 45, 10, "", &NO_APNS).unwrap_err().contains("https")
        );
        assert!(validate_settings("not a url", true, 45, 10, "", &NO_APNS).is_err());
    }

    #[test]
    fn timing_bounds() {
        assert!(validate_settings(OK_DOMAIN, true, 0, 0, "", &NO_APNS).unwrap_err().contains("REINS_RELAY_WAIT_SECS"));
        assert!(
            validate_settings(OK_DOMAIN, true, 56, 10, "", &NO_APNS).unwrap_err().contains("REINS_RELAY_WAIT_SECS")
        );
        assert!(validate_settings(OK_DOMAIN, true, 45, 0, "", &NO_APNS).unwrap_err().contains("REINS_OFFLINE_SECS"));
        assert!(validate_settings(OK_DOMAIN, true, 10, 11, "", &NO_APNS).unwrap_err().contains("REINS_OFFLINE_SECS"));
    }

    #[test]
    fn fcm_path_must_exist_when_set() {
        let missing = std::env::temp_dir().join("reins-missing-service-account.json");
        let err = validate_settings(OK_DOMAIN, true, 45, 10, missing.to_str().unwrap(), &NO_APNS).unwrap_err();
        assert!(err.contains("REINS_FCM_SERVICE_ACCOUNT"), "{err}");
    }

    #[test]
    fn apns_settings_are_checked_at_startup() {
        let partial = apns::Settings {
            key_id: "ABC123DEFG".to_owned(),
            ..NO_APNS
        };
        let err = validate_settings(OK_DOMAIN, true, 45, 10, "", &partial).unwrap_err();
        assert!(err.contains("REINS_APNS_KEY_FILE"), "{err}");
        let missing = apns::Settings {
            key_file: "/nonexistent/AuthKey_ABC123DEFG.p8".to_owned(),
            team_id: "DEF123GHIJ".to_owned(),
            topic: apns::DEFAULT_TOPIC.to_owned(),
            ..partial
        };
        let err = validate_settings(OK_DOMAIN, true, 45, 10, "", &missing).unwrap_err();
        assert!(err.contains("REINS_APNS_KEY_FILE") && err.contains("/nonexistent"), "{err}");
    }

    #[test]
    fn blob_secrets_are_kept_out_of_logged_paths() {
        assert_eq!(loggable_path("/reins/blob/c2VjcmV0LXNlY3JldA"), "/reins/blob/\u{2026}");
        assert_eq!(loggable_path("/vw/reins/blob/abc"), "/vw/reins/blob/\u{2026}");
        assert_eq!(loggable_path("/reins/api/blobs/abc"), "/reins/api/blobs/abc");
        assert_eq!(loggable_path("/mcp"), "/mcp");
    }

    #[test]
    fn ttl_constants_match_spec() {
        assert_eq!(ITEM_TTL.as_secs(), 600);
        assert_eq!(SESSION_TTL.as_secs(), 300);
        assert_eq!(CODE_TTL.as_secs(), 60);
        assert_eq!(CIMD_TTL.as_secs(), 3600);
        assert_eq!(ACCESS_TOKEN_SECS, 3600);
        assert_eq!(REFRESH_TOKEN_SECS, 30 * 24 * 3600);
    }
}

#[cfg(test)]
mod hub_tests {
    use reins_proto::gmail::ToolCall;
    use tokio::time::{Instant, sleep};

    use super::*;

    fn test_hub() -> Hub {
        Hub::new(Timing {
            relay_wait: Duration::from_secs(45),
            offline: Duration::from_secs(10),
        })
    }

    fn client() -> pairing::PairingClient {
        pairing::PairingClient {
            client_id: "cid".to_owned(),
            client_name: "Claude".to_owned(),
            client_host: "claude.ai".to_owned(),
            client_key: None,
        }
    }

    fn read_call() -> ToolCall {
        ToolCall::GmailRead {
            message_ids: vec!["m1".to_owned()],
        }
    }

    #[tokio::test(start_paused = true)]
    async fn pending_returns_queued_items_immediately_and_only_once() {
        let hub = test_hub();
        hub.pairings.start("u1", client(), 0).unwrap();
        hub.relay.submit("u1", &"c1".into(), "Claude", read_call(), None, 0).unwrap();
        let start = Instant::now();
        let p = hub.pending("u1", Duration::from_secs(25)).await;
        assert_eq!((p.requests.len(), p.pairings.len()), (1, 1));
        assert_eq!(start.elapsed(), Duration::ZERO);
        assert!(hub.pending("u1", Duration::ZERO).await.is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn pending_long_polls_until_an_item_for_this_user_arrives() {
        let hub = test_hub();
        let start = Instant::now();
        let (p, ()) = tokio::join!(hub.pending("u1", Duration::from_secs(25)), async {
            sleep(Duration::from_secs(3)).await;
            hub.pairings.start("u2", client(), 0).unwrap();
            sleep(Duration::from_secs(2)).await;
            hub.relay.submit("u1", &"c1".into(), "Claude", read_call(), None, 0).unwrap();
        });
        assert_eq!(p.requests.len(), 1);
        assert!(p.pairings.is_empty());
        assert_eq!(start.elapsed(), Duration::from_secs(5));
    }

    #[tokio::test(start_paused = true)]
    async fn a_forgotten_user_has_nothing_waiting_and_others_keep_theirs() {
        let hub = test_hub();
        for user in ["u1", "u2"] {
            hub.pairings.start(user, client(), 0).unwrap();
            hub.relay.submit(user, &"c1".into(), "Work AI", read_call(), None, 0).unwrap();
            hub.set_services(user, vec!["gmail".to_owned()]);
        }
        let request = hub.relay.submit("u1", &"c1".into(), "Work AI", read_call(), None, 0).unwrap();
        hub.forget_user("u1");
        assert!(hub.pending("u1", Duration::ZERO).await.is_empty());
        assert!(hub.relay.fetch("u1", &request.id).is_none());
        assert_eq!(hub.services_of("u1"), None);
        let p = hub.pending("u2", Duration::ZERO).await;
        assert_eq!((p.requests.len(), p.pairings.len()), (1, 1));
        assert!(hub.services_of("u2").is_some());
    }

    #[tokio::test(start_paused = true)]
    async fn pending_gives_up_after_the_wait() {
        let hub = test_hub();
        let start = Instant::now();
        assert!(hub.pending("u1", Duration::from_secs(25)).await.is_empty());
        assert_eq!(start.elapsed(), Duration::from_secs(25));
    }
}
