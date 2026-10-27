//! Reins server: MCP endpoint, OAuth 2.1 authorization server for AI clients,
//! phone API and in-memory relay (spec §4, contracts §A and §C).

pub mod account_delete;
pub mod account_reset;
pub mod account_state;
pub mod apns;
pub mod blob;
pub mod blob_io;
pub mod blob_routes;
pub mod circles;
pub mod desktop_routes;
pub mod device_api;
pub mod device_flow;
pub mod fcm;
pub mod join;
pub mod limits;
pub mod live;
pub mod mcp;
pub mod mcp_routes;
pub mod mcp_tools;
pub mod oauth;
pub mod oauth_routes;
pub mod oauth_state;
pub mod org;
pub mod outbound;
pub mod pages;
pub mod pairing;
pub mod presence;
pub mod proxy_call;
pub mod push;
pub mod relay;
pub mod roles;
pub mod sniff;
pub mod ticket_routes;
pub mod tickets;
pub mod tools;
pub mod ttl;
pub mod vault_passkeys;
pub mod webhooks;
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
    /// Named in the MCP instructions; every tool is listed regardless, and every call goes to the phone, which knows
    /// first when an account is added. Unknown until the phone reports after the server started.
    services: std::sync::Mutex<std::collections::HashMap<String, Vec<String>>>,
    /// Per user: the MCP servers added on the phone, as last reported (validated).
    mcp_servers: std::sync::Mutex<std::collections::HashMap<String, Arc<Vec<McpServerReport>>>>,
    /// Per user: the phone's Autopilot modes as last reported (`PUT /reins/api/modes`) and when; shown to the desktop
    /// apps, never used to decide anything here.
    modes: std::sync::Mutex<std::collections::HashMap<String, (reins_proto::autopilot::ModesReport, i64)>>,
    /// When each account's approval device was last heard from.
    pub presence: presence::Presence,
    /// Per desktop connection: sealed "drop your secrets" notes from the phone, until its daemon picks them up.
    pub revocations: Revocations,
    /// Viewers asking to become primary (phone roles).
    pub handovers: roles::HandoverHub,
    /// Per account with phone roles: what wakes the members' long-polls, and the op log's head.
    pub live: live::LiveHub,
    /// Tickets (deferred decisions): what wakes their waits and the phone, and results passing through.
    pub tickets: tickets::TicketHub,
    /// Circles: which accounts' phones have news (ends their long poll).
    pub circles: circles::CircleNews,
}

/// Sealed lease revocations for desktop apps (Reins Secrets): the phone posts one per paired computer, the computer's
/// daemon picks them up and acknowledges them once applied. Memory only, each kept a day; the server cannot open them.
///
/// Every change to a computer's mailbox happens under one lock, so a post, a pick-up and an acknowledgement never see
/// half of another. Nothing is lost silently:
///
/// - a revocation of everything (`all`) supersedes whatever came before it and is never pushed out by others; the
///   others are kept up to [`reins_proto::desktop::MAX_REVOCATIONS`], and when one has to go the mailbox says so
///   ([`reins_proto::desktop::LostRevocations`]), which makes the daemon drop every lease made before then;
/// - a server that restarts forgot what it held: every mailbox starts out saying so, as of the start;
/// - an acknowledgement only marks what one reader (one daemon run) applied, by the ids the server gave: it never
///   removes a revocation from another reader, and one posted meanwhile has an id no acknowledgement names.
pub struct Revocations {
    /// When this server started, under an id: what every mailbox reports as possibly lost until acknowledged.
    boot: (String, i64),
    by_connection: std::sync::Mutex<std::collections::HashMap<String, Mailbox>>,
}

impl Default for Revocations {
    fn default() -> Self {
        Self {
            boot: (uuid::Uuid::new_v4().to_string(), now_unix()),
            by_connection: std::sync::Mutex::default(),
        }
    }
}

struct Mailbox {
    /// The newest revocation of everything.
    all: Option<Mailed>,
    /// Revocations of a project or an environment since then, oldest first.
    some: Vec<Mailed>,
    loss: Loss,
}

struct Mailed {
    id: String,
    sealed: String,
    until: i64,
    read_by: std::collections::BTreeSet<String>,
}

struct Loss {
    id: String,
    at: i64,
    read_by: std::collections::BTreeSet<String>,
    /// The restart, not revocations dropped.
    boot: bool,
}

/// The most readers recorded per revocation (a computer runs one daemon; more only from copies of its state).
const MAX_READERS: usize = 16;

impl Mailbox {
    fn new(boot: &(String, i64)) -> Self {
        Self {
            all: None,
            some: Vec::new(),
            loss: Loss {
                id: boot.0.clone(),
                at: boot.1,
                read_by: std::collections::BTreeSet::new(),
                boot: true,
            },
        }
    }

    fn expire(&mut self, now: i64) {
        if self.all.as_ref().is_some_and(|m| m.until <= now) {
            self.all = None;
        }
        self.some.retain(|m| m.until > now);
    }
}

/// The longest sealed revocation kept.
pub const MAX_REVOCATION_CHARS: usize = 2_000;

fn mark(read_by: &mut std::collections::BTreeSet<String>, reader: &str) {
    if read_by.len() < MAX_READERS {
        read_by.insert(reader.to_owned());
    }
}

impl Revocations {
    fn lock(&self) -> std::sync::MutexGuard<'_, std::collections::HashMap<String, Mailbox>> {
        self.by_connection.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    pub fn post(&self, connection: &str, sealed: String, all: bool, now: i64) {
        if sealed.is_empty() || sealed.len() > MAX_REVOCATION_CHARS {
            return;
        }
        let mut boxes = self.lock();
        let mailbox = boxes.entry(connection.to_owned()).or_insert_with(|| Mailbox::new(&self.boot));
        mailbox.expire(now);
        let mailed = Mailed {
            id: uuid::Uuid::new_v4().to_string(),
            sealed,
            until: now + reins_proto::desktop::REVOCATION_TTL_SECS,
            read_by: std::collections::BTreeSet::new(),
        };
        if all {
            mailbox.some.clear();
            mailbox.all = Some(mailed);
            return;
        }
        mailbox.some.push(mailed);
        let excess = mailbox.some.len().saturating_sub(reins_proto::desktop::MAX_REVOCATIONS);
        if excess > 0 {
            mailbox.some.drain(..excess);
            mailbox.loss = Loss {
                id: uuid::Uuid::new_v4().to_string(),
                at: now,
                read_by: std::collections::BTreeSet::new(),
                boot: false,
            };
        }
    }

    /// What waits for `reader` of `connection`: the revocation of everything first, then the others oldest first, and
    /// a loss it has not acknowledged.
    pub fn peek(&self, connection: &str, reader: &str, now: i64) -> reins_proto::desktop::RevocationMail {
        let mut boxes = self.lock();
        let mailbox = boxes.entry(connection.to_owned()).or_insert_with(|| Mailbox::new(&self.boot));
        mailbox.expire(now);
        reins_proto::desktop::RevocationMail {
            items: mailbox
                .all
                .iter()
                .chain(&mailbox.some)
                .filter(|m| !m.read_by.contains(reader))
                .map(|m| reins_proto::desktop::MailedRevocation {
                    id: m.id.clone(),
                    sealed: m.sealed.clone(),
                })
                .collect(),
            lost: (!mailbox.loss.read_by.contains(reader)).then(|| reins_proto::desktop::LostRevocations {
                id: mailbox.loss.id.clone(),
                at: mailbox.loss.at,
            }),
        }
    }

    /// Marks what `ack.reader` of `connection` applied: those exact ids, nothing else.
    pub fn ack(&self, connection: &str, ack: &reins_proto::desktop::RevocationAck) {
        let mut boxes = self.lock();
        let Some(mailbox) = boxes.get_mut(connection) else {
            return;
        };
        for m in mailbox.all.iter_mut().chain(mailbox.some.iter_mut()) {
            if ack.ids.contains(&m.id) {
                mark(&mut m.read_by, &ack.reader);
            }
        }
        if ack.lost.as_deref() == Some(mailbox.loss.id.as_str()) {
            mark(&mut mailbox.loss.read_by, &ack.reader);
        }
    }

    /// Forgets what is a day old, and mailboxes with nothing left but the restart (which a new one reports too).
    pub fn purge(&self, now: i64) {
        self.lock().retain(|_, mailbox| {
            mailbox.expire(now);
            mailbox.all.is_some() || !mailbox.some.is_empty() || !mailbox.loss.boot
        });
    }
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
            handovers: roles::HandoverHub::new(Arc::clone(&signal)),
            live: live::LiveHub::default(),
            tickets: tickets::TicketHub::default(),
            circles: circles::CircleNews::new(Arc::clone(&signal)),
            signal,
            services: std::sync::Mutex::default(),
            mcp_servers: std::sync::Mutex::default(),
            modes: std::sync::Mutex::default(),
            presence: presence::Presence::default(),
            revocations: Revocations::default(),
        }
    }

    /// When `user`'s approval phone last asked this server for anything (Unix seconds), since this server started.
    pub fn phone_seen(&self, user: &str) -> Option<i64> {
        self.presence.last_seen(user)
    }

    pub fn set_services(&self, user: &str, services: Vec<String>) {
        self.services.lock().unwrap_or_else(std::sync::PoisonError::into_inner).insert(user.to_owned(), services);
    }

    /// The integrations `user`'s phone reported, if it has.
    pub fn services_of(&self, user: &str) -> Option<Vec<String>> {
        self.services.lock().unwrap_or_else(std::sync::PoisonError::into_inner).get(user).cloned()
    }

    /// Keeps the modes `user`'s phone reported at `now`.
    pub fn set_modes(&self, user: &str, report: reins_proto::autopilot::ModesReport, now: i64) {
        self.modes.lock().unwrap_or_else(std::sync::PoisonError::into_inner).insert(user.to_owned(), (report, now));
    }

    /// What one connection of `user` is shown of the phone's modes: for a desktop app the phone paired, the global
    /// mode and its own; for any other connection, or until the phone has reported them since this server started,
    /// nothing.
    pub fn modes_for(&self, user: &str, connection_id: &str) -> Option<reins_proto::autopilot::DesktopModes> {
        let modes = self.modes.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        modes.get(user).and_then(|(report, at)| report.for_connection(connection_id, *at))
    }

    /// Whether `connection_id` is one of `user`'s desktop apps, as the phone last listed them (it pinned their keys
    /// when pairing). Anything else, an AI's connection included, is not: and nothing the phone says for the
    /// desktop apps (modes, who decided, where Autopilot leans) reaches it.
    pub fn is_desktop(&self, user: &str, connection_id: &str) -> bool {
        let modes = self.modes.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        modes.get(user).is_some_and(|(report, _)| report.desktops.iter().any(|d| d == connection_id))
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
    /// `wait` and returns as soon as one appears. Returned items are marked delivered. `log_after` (accounts with a
    /// roster): also returns as soon as the op log's head passes it.
    pub async fn pending(&self, user: &str, wait: Duration, log_after: Option<u64>) -> Pending {
        let deadline = tokio::time::Instant::now() + wait;
        let mut rx = self.signal.subscribe();
        let mut log = log_after.map(|_| self.live.subscribe(user));
        // The phone is listening while this poll is open: requests reach it without a push.
        let _listening = self.presence.listen(user, now_unix());
        // The phone polls often: a cheap moment to delete expired files between the hourly purges.
        self.blobs.purge(now_unix());
        loop {
            let pending = Pending {
                account_email: None,
                requests: self.relay.take_undelivered(user),
                pairings: self.pairings.take_undelivered(user),
                blobs: self.blobs.take_undelivered(user, now_unix()),
                joins: self.joins.take_undelivered(user, now_unix()),
                handovers: self.handovers.take_undelivered(user, now_unix()),
                log_head: log_after.and(self.live.head(user)),
                tickets: vec![],
                instant_window_secs: None,
                circles: self.circles.take(user),
            };
            let log_moved = pending.log_head.zip(log_after).is_some_and(|(head, after)| head > after);
            // Ticket news is fetched by the caller (it needs the database); it ends the wait.
            let ticket_news = self.tickets.take_news(user);
            if !pending.is_empty() || log_moved || ticket_news || tokio::time::Instant::now() >= deadline {
                return pending;
            }
            let log_changed = async {
                match log.as_mut() {
                    Some(log) => log.changed().await.is_ok(),
                    None => std::future::pending().await,
                }
            };
            tokio::select! {
                changed = rx.changed() => if changed.is_err() { tokio::time::sleep_until(deadline).await },
                alive = log_changed => if !alive { tokio::time::sleep_until(deadline).await },
                () = tokio::time::sleep_until(deadline) => {}
            }
        }
    }

    /// Drops what the hub remembers about a deleted account: its reported integrations and MCP servers, the requests,
    /// pairings and "add another phone" requests waiting for it, and its files (deleted from disk).
    pub fn forget_user(&self, user: &str) {
        self.services.lock().unwrap_or_else(std::sync::PoisonError::into_inner).remove(user);
        self.mcp_servers.lock().unwrap_or_else(std::sync::PoisonError::into_inner).remove(user);
        self.modes.lock().unwrap_or_else(std::sync::PoisonError::into_inner).remove(user);
        self.relay.forget_user(user);
        self.tickets.forget_user(user);
        self.circles.forget(user);
        self.presence.forget(user);
        self.pairings.forget_user(user);
        self.joins.forget_user(user);
        self.handovers.forget_user(user);
        self.live.forget(user);
        self.blobs.remove_user(user);
    }

    pub fn purge(&self) {
        self.relay.purge();
        self.pairings.purge();
        self.blobs.purge(now_unix());
        self.joins.purge(now_unix());
        self.revocations.purge(now_unix());
        self.handovers.purge(now_unix());
        self.live.purge();
    }
}

/// The process-wide hub (spec §2: one server instance per deployment).
pub static HUB: LazyLock<Hub> = LazyLock::new(|| {
    let max_queued = usize::try_from(CONFIG.reins_account_max_queued()).unwrap_or(usize::MAX);
    let mut hub = Hub::with_limits(Timing::from_config(), max_queued, BlobLimits::from_config());
    hub.relay = hub.relay.with_instant_window(tickets::instant_window());
    hub
});

pub fn enabled() -> bool {
    CONFIG.reins_enabled()
}

/// Scheduled housekeeping: expired refresh tokens and in-memory entries.
pub async fn purge(pool: crate::db::DbPool) {
    debug!("Purging Reins state");
    HUB.purge();
    oauth_state::OAUTH.purge();
    oauth_routes::purge_fetch_sites();
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
    routes.extend(ticket_routes::routes());
    routes.extend(circles::routes());
    routes.extend(blob_routes::routes());
    routes.extend(proxy_call::routes());
    routes.extend(pages::routes());
    routes.extend(join::routes());
    routes.extend(workos_sync::routes());
    routes.extend(vault_passkeys::routes());
    routes.extend(roles::routes());
    routes.extend(live::routes());
    routes.extend(org::routes());
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
        let p = hub.pending("u1", Duration::from_secs(25), None).await;
        assert_eq!((p.requests.len(), p.pairings.len()), (1, 1));
        assert_eq!(start.elapsed(), Duration::ZERO);
        assert!(hub.pending("u1", Duration::ZERO, None).await.is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn pending_long_polls_until_an_item_for_this_user_arrives() {
        let hub = test_hub();
        let start = Instant::now();
        let (p, ()) = tokio::join!(hub.pending("u1", Duration::from_secs(25), None), async {
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
        assert!(hub.pending("u1", Duration::ZERO, None).await.is_empty());
        assert!(hub.relay.fetch("u1", &request.id).is_none());
        assert_eq!(hub.services_of("u1"), None);
        let p = hub.pending("u2", Duration::ZERO, None).await;
        assert_eq!((p.requests.len(), p.pairings.len()), (1, 1));
        assert!(hub.services_of("u2").is_some());
    }

    #[tokio::test(start_paused = true)]
    async fn pending_gives_up_after_the_wait() {
        let hub = test_hub();
        let start = Instant::now();
        assert!(hub.pending("u1", Duration::from_secs(25), None).await.is_empty());
        assert_eq!(start.elapsed(), Duration::from_secs(25));
    }
}
