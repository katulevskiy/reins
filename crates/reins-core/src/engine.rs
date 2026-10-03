//! The engine behind the exported `ReinsCore` object (contracts §D): session
//! management, the sync/push entry points and the read-only views. Request
//! processing lives in `handler.rs`, approvals in `approval.rs`.

use std::collections::HashSet;
use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use reins_proto::device::{DeviceRegistration, codes};
use reins_proto::ids::{ConnectionId, GrantId};
use zeroize::Zeroizing;

use crate::crypto::{Kdf, new_account_keys};
use crate::gmail::{self, GmailClient, Probe};
use crate::http::{self, ServerUrl};
use crate::phone_api::{ApiFailure, check_id};
use crate::session::{Session, api_call};
use crate::store::{Store, StoredSession, unix_now};
use crate::types::{
    AccountView, ActivityEntry, ConnectionView, GmailStatus, GrantView, PendingItem, PendingKind, SessionInfo,
};
use crate::vault::VaultClient;
use crate::views::{self, ParkedRequest};
use crate::{CoreError, GoogleTokenProvider, KeyWrapper, Notifier};

/// Bitwarden's shortest master password for a new account.
pub const MIN_MASTER_PASSWORD_CHARS: usize = 12;
/// Longest account email (Bitwarden's limit is 256; 254 is the longest address SMTP carries).
const MAX_EMAIL_BYTES: usize = 254;
/// What a new account derives its master key with: PBKDF2-SHA256 with 600 000 iterations, the default of the
/// Bitwarden clients and of Vaultwarden.
pub const NEW_ACCOUNT_KDF: Kdf = Kdf::Pbkdf2 {
    iterations: 600_000,
};

/// Settings that tests override; the exported constructor uses the defaults.
#[derive(Clone, Debug)]
pub struct CoreConfig {
    pub gmail_base: String,
    pub calendar_base: String,
    pub people_base: String,
    pub github_base: String,
    /// The API bases of the other git hosts (`https://gitlab.com/api/v4`, ...): tests point them at fake servers.
    pub gitlab_base: String,
    pub codeberg_base: String,
    pub bitbucket_base: String,
    /// Telegram's application credentials (my.telegram.org); 0 / empty when this build has none.
    pub telegram_api_id: i32,
    pub telegram_api_hash: String,
    pub backoff_base: Duration,
    /// Where Autopilot's model is downloaded from (`<base>/<id>/<file>`).
    pub models_base: String,
    /// The models this build trusts (`autopilot::model::KNOWN_MODELS`); tests pin their own.
    pub models: Vec<crate::autopilot::model::KnownModel>,
    /// A connection younger than this never gets automatic decisions (10 minutes; tests shorten it).
    pub new_connection_secs: i64,
}

impl Default for CoreConfig {
    fn default() -> Self {
        Self {
            gmail_base: gmail::DEFAULT_BASE.to_owned(),
            calendar_base: crate::connector::calendar::CALENDAR_BASE.to_owned(),
            people_base: crate::connector::calendar::PEOPLE_BASE.to_owned(),
            github_base: crate::connector::github::GITHUB_BASE.to_owned(),
            gitlab_base: crate::connector::githost::GITLAB_BASE.to_owned(),
            codeberg_base: crate::connector::githost::CODEBERG_BASE.to_owned(),
            bitbucket_base: crate::connector::githost::BITBUCKET_BASE.to_owned(),
            telegram_api_id: 0,
            telegram_api_hash: String::new(),
            backoff_base: Duration::from_millis(500),
            models_base: crate::autopilot::model::DEFAULT_MODELS_BASE.to_owned(),
            models: crate::autopilot::model::KNOWN_MODELS.to_vec(),
            new_connection_secs: crate::autopilot::gates::NEW_CONNECTION_SECS,
        }
    }
}

/// Removes an id from the in-flight set when dropped.
pub struct InFlight {
    set: Arc<Mutex<HashSet<String>>>,
    id: String,
}

impl Drop for InFlight {
    fn drop(&mut self) {
        self.set.lock().unwrap_or_else(PoisonError::into_inner).remove(&self.id);
    }
}

pub struct Engine {
    pub(crate) store: Arc<Store>,
    pub(crate) http: reqwest::Client,
    pub(crate) cfg: CoreConfig,
    pub(crate) google: Arc<dyn GoogleTokenProvider>,
    pub(crate) notifier: Arc<dyn Notifier>,
    /// The integrations besides Gmail that this phone can run.
    pub(crate) connectors: crate::connector::Registry,
    session: crate::connector::vault::SessionSlot,
    /// What was last reported to the server, so that a report is sent only when it changes.
    reported_services: Mutex<Option<reins_proto::device::ServicesReport>>,
    /// Universal MCP: sessions with the added servers.
    pub(crate) mcp: crate::mcp::McpState,
    inflight: Arc<Mutex<HashSet<String>>>,
    /// Where the store and the model packages live.
    pub(crate) data_dir: std::path::PathBuf,
    /// Autopilot: the model runtime, the opened package, downloads.
    pub(crate) autopilot: crate::autopilot::State,
}

impl Engine {
    pub fn new(
        data_dir: &Path,
        keys: &dyn KeyWrapper,
        google: Arc<dyn GoogleTokenProvider>,
        notifier: Arc<dyn Notifier>,
        cfg: CoreConfig,
    ) -> Result<Arc<Self>, CoreError> {
        Self::with_connectors(data_dir, keys, google, notifier, cfg, Vec::new())
    }

    /// Like [`Engine::new`], with these integrations added (or replacing the built-in ones): how tests bring their own.
    pub fn with_connectors(
        data_dir: &Path,
        keys: &dyn KeyWrapper,
        google: Arc<dyn GoogleTokenProvider>,
        notifier: Arc<dyn Notifier>,
        cfg: CoreConfig,
        extra: Vec<Arc<dyn crate::connector::Connector>>,
    ) -> Result<Arc<Self>, CoreError> {
        let store = Arc::new(Store::open(data_dir, keys)?);
        let http = http::client()?;
        let session = match store.load_session()? {
            Some(saved) => {
                if let Ok(server) = ServerUrl::parse(&saved.server_url) {
                    Some(Arc::new(Session::new(
                        http.clone(),
                        Arc::clone(&store),
                        server,
                        saved.email,
                        saved.refresh_token,
                        None,
                    )))
                } else {
                    store.clear_session()?;
                    None
                }
            }
            None => None,
        };
        let session: crate::connector::vault::SessionSlot = Arc::new(Mutex::new(session));
        let mut connectors = crate::connector::Registry::default();
        connectors.add(Arc::new(crate::connector::vault::Vault::new(Arc::clone(&session), Arc::clone(&store))));
        connectors.add(Arc::new(crate::connector::calendar::GoogleCalendar::new(
            http.clone(),
            &cfg.calendar_base,
            Arc::clone(&google),
            cfg.backoff_base,
        )));
        connectors.add(Arc::new(crate::connector::calendar::GoogleContacts::new(
            http.clone(),
            &cfg.people_base,
            Arc::clone(&google),
            cfg.backoff_base,
        )));
        connectors.add(Arc::new(crate::connector::github::GitHub::new(
            http.clone(),
            &cfg.github_base,
            Arc::clone(&store),
            cfg.backoff_base,
        )));
        connectors.add(Arc::new(crate::connector::telegram::Telegram::new(Arc::new(
            crate::connector::telegram_client::Grammers::new(
                cfg.telegram_api_id,
                &cfg.telegram_api_hash,
                Arc::clone(&store),
            ),
        ))));
        connectors.add(Arc::new(crate::connector::desktop::Desktop));
        connectors.add(Arc::new(crate::connector::githost::GitLab::new(
            http.clone(),
            &cfg.gitlab_base,
            Arc::clone(&store),
            cfg.backoff_base,
        )));
        connectors.add(Arc::new(crate::connector::githost::Codeberg::new(
            http.clone(),
            &cfg.codeberg_base,
            Arc::clone(&store),
            cfg.backoff_base,
        )));
        connectors.add(Arc::new(crate::connector::githost::Bitbucket::new(
            http.clone(),
            &cfg.bitbucket_base,
            Arc::clone(&store),
            cfg.backoff_base,
        )));
        for connector in extra {
            connectors.add(connector);
        }
        Ok(Arc::new(Self {
            store,
            http,
            cfg,
            google,
            notifier,
            connectors,
            session,
            reported_services: Mutex::new(None),
            mcp: crate::mcp::McpState::default(),
            inflight: Arc::new(Mutex::new(HashSet::new())),
            data_dir: data_dir.to_path_buf(),
            autopilot: crate::autopilot::State::default(),
        }))
    }

    /// A Gmail client acting as `account` (empty: the phone's default Google account).
    pub(crate) fn gmail_for(&self, account: &str) -> GmailClient {
        GmailClient::new(
            self.http.clone(),
            &self.cfg.gmail_base,
            Arc::clone(&self.google),
            account,
            self.cfg.backoff_base,
        )
    }

    /// The connected Gmail addresses, oldest first.
    pub(crate) fn gmail_accounts(&self) -> Vec<String> {
        self.store
            .accounts()
            .unwrap_or_default()
            .into_iter()
            .filter(|a| a.service == views::SERVICE_GMAIL)
            .map(|a| a.account)
            .collect()
    }

    /// The accounts connected to one integration.
    pub(crate) fn accounts_of(&self, service: &str) -> Vec<String> {
        self.store
            .accounts()
            .unwrap_or_default()
            .into_iter()
            .filter(|a| a.service == service)
            .map(|a| a.account)
            .collect()
    }

    /// The integrations that have at least one account, in the order they were first connected.
    pub(crate) fn integrations(&self) -> Vec<String> {
        let mut services: Vec<String> = Vec::new();
        for account in self.store.accounts().unwrap_or_default() {
            if !services.contains(&account.service) {
                services.push(account.service);
            }
        }
        services
    }

    /// The account a call is about: the one named, or the only one connected. The error is what the AI is told.
    ///
    /// A phone that connected Gmail before accounts were tracked has none registered yet: its default Google
    /// account is looked up once and registered here.
    pub(crate) async fn resolve_account(&self, named: Option<&str>) -> Result<String, String> {
        let mut accounts = self.gmail_accounts();
        if accounts.is_empty() {
            match self.gmail_for("").account().await {
                Ok(Some(found)) => {
                    self.store.add_account(views::SERVICE_GMAIL, &found, unix_now()).map_err(|e| e.to_string())?;
                    accounts = self.gmail_accounts();
                }
                Ok(None) => {}
                Err(e) => return Err(crate::handler::ai_message(&e)),
            }
        }
        match (named, accounts.as_slice()) {
            (_, []) => Err(crate::handler::ai_message(&CoreError::GmailNeedsConsent)),
            (Some(name), _) => accounts.iter().find(|a| a.eq_ignore_ascii_case(name)).cloned().ok_or_else(|| {
                format!("The account {name} is not connected to Reins. Call reins_list_accounts with service=\"gmail\" to ask the user to share their accounts, and pick one of those.")
            }),
            (None, [only]) => Ok(only.clone()),
            (None, _) => Err(
                "Several accounts are connected. Call reins_list_accounts with service=\"gmail\" (the user has to allow it) and pass the one you want as `account`.".to_owned(),
            ),
        }
    }

    /// Registers an account of any integration once it has been checked (a session that works, a token that was
    /// accepted). The name is kept in lower case.
    pub fn register_account(&self, service: &str, account: &str) -> Result<AccountView, CoreError> {
        let account = account.trim().to_lowercase();
        if account.is_empty() || account.len() > reins_proto::connector::MAX_ACCOUNT_LEN {
            return Err(CoreError::invalid("that is not a usable account name"));
        }
        let now = unix_now();
        self.store.add_account(service, &account, now)?;
        Ok(AccountView {
            service: service.to_owned(),
            account,
            added_at: now,
        })
    }

    /// The accounts the user connected.
    pub fn accounts(&self) -> Result<Vec<AccountView>, CoreError> {
        Ok(self
            .store
            .accounts()?
            .into_iter()
            .map(|a| AccountView {
                service: a.service,
                account: a.account,
                added_at: a.added_at,
            })
            .collect())
    }

    /// Connects a Google account the user just authorized. `hint` is the address the phone authorized; the
    /// address Gmail reports for it is what gets stored.
    pub async fn add_account(&self, hint: &str) -> Result<AccountView, CoreError> {
        let hint = reins_proto::gmail::normalize_account(hint).map_err(|e| CoreError::invalid(e.to_string()))?;
        let found = self.gmail_for(&hint).account().await?.unwrap_or(hint);
        let now = unix_now();
        self.store.add_account(views::SERVICE_GMAIL, &found, now)?;
        Ok(AccountView {
            service: views::SERVICE_GMAIL.to_owned(),
            account: found,
            added_at: now,
        })
    }

    /// Disconnects a Gmail account: its grants go, and requests waiting for it are answered with an error.
    pub async fn remove_account(&self, account: &str) -> Result<(), CoreError> {
        self.remove_service_account(views::SERVICE_GMAIL, account).await
    }

    /// Whether one account can be used right now.
    pub async fn account_status(&self, account: &str) -> GmailStatus {
        match self.gmail_for(account).probe().await {
            Probe::Ready => GmailStatus::Ready,
            Probe::NeedsConsent => GmailStatus::NeedsConsent,
            Probe::Unavailable(message) => GmailStatus::Unavailable {
                message,
            },
        }
    }

    fn session_slot(&self) -> MutexGuard<'_, Option<Arc<Session>>> {
        self.session.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Replaces the signed-in session (a finished sign-in).
    pub(crate) fn set_session(&self, session: Option<Arc<Session>>) {
        *self.session_slot() = session;
    }

    pub(crate) fn session(&self) -> Result<Arc<Session>, CoreError> {
        self.session_slot().clone().ok_or(CoreError::NotLoggedIn)
    }

    /// Claims `id` for processing; `None` when it is being processed or already known.
    pub(crate) fn begin(&self, id: &str) -> Result<Option<InFlight>, CoreError> {
        let mut set = self.inflight.lock().unwrap_or_else(PoisonError::into_inner);
        if set.contains(id) || self.store.is_known(id)? {
            return Ok(None);
        }
        set.insert(id.to_owned());
        Ok(Some(InFlight {
            set: Arc::clone(&self.inflight),
            id: id.to_owned(),
        }))
    }

    /// Like `begin` for an id that is already parked (approve/deny): only serializes callers.
    pub(crate) fn claim(&self, id: &str) -> Option<InFlight> {
        let mut set = self.inflight.lock().unwrap_or_else(PoisonError::into_inner);
        if !set.insert(id.to_owned()) {
            return None;
        }
        Some(InFlight {
            set: Arc::clone(&self.inflight),
            id: id.to_owned(),
        })
    }

    // ---- session -------------------------------------------------------------------

    pub fn session_info(&self) -> Option<SessionInfo> {
        self.session_slot().as_ref().map(|s| SessionInfo {
            server_url: s.server.as_str().to_owned(),
            email: s.email(),
        })
    }

    pub async fn login(
        &self,
        server_url: &str,
        email: &str,
        password: Zeroizing<String>,
        totp: Option<String>,
    ) -> Result<SessionInfo, CoreError> {
        let server = ServerUrl::parse(server_url)?;
        let email = email.trim().to_lowercase();
        if !email.contains('@') {
            return Err(CoreError::invalid("enter your account email address"));
        }
        let device_id = self.store.device_id()?;
        let (tokens, proof) =
            VaultClient::new(&self.http, &server).login(&email, password, totp.as_deref(), &device_id).await?;
        self.store.save_session(&StoredSession {
            server_url: server.as_str().to_owned(),
            email: email.clone(),
            refresh_token: tokens.refresh_token.clone(),
        })?;
        let info = SessionInfo {
            server_url: server.as_str().to_owned(),
            email: email.clone(),
        };
        let session = Session::from_login(self.http.clone(), Arc::clone(&self.store), server, email, tokens);
        session.keep_proof(proof);
        *self.session_slot() = Some(Arc::new(session));
        Ok(info)
    }

    /// Creates an account on the server (Bitwarden-compatible: its vault opens in any Bitwarden client), then signs in
    /// to it exactly like [`Engine::login`].
    pub async fn create_account(
        &self,
        server_url: &str,
        email: &str,
        password: Zeroizing<String>,
    ) -> Result<SessionInfo, CoreError> {
        let server = ServerUrl::parse(server_url)?;
        let email = email.trim().to_lowercase();
        let looks_like_email = email.split_once('@').is_some_and(|(local, domain)| {
            !local.is_empty() && domain.contains('.') && !domain.starts_with('.') && !domain.ends_with('.')
        });
        if !looks_like_email || email.len() > MAX_EMAIL_BYTES || email.chars().any(char::is_whitespace) {
            return Err(CoreError::invalid("Enter a valid email address."));
        }
        if password.chars().count() < MIN_MASTER_PASSWORD_CHARS {
            return Err(CoreError::invalid(format!(
                "The master password needs at least {MIN_MASTER_PASSWORD_CHARS} characters."
            )));
        }
        let (secret, salt) = (password.clone(), email.clone());
        let keys = tokio::task::spawn_blocking(move || new_account_keys(&secret, &salt, NEW_ACCOUNT_KDF))
            .await
            .map_err(|_| CoreError::storage("key derivation was interrupted"))??;
        VaultClient::new(&self.http, &server).register(&email, &keys, NEW_ACCOUNT_KDF).await?;
        self.login(server.as_str(), &email, password, None).await
    }

    /// Signs out. Grants, audit log and the device id are kept; parked items and the desktop apps' keys are dropped
    /// (a desktop app is paired again after signing in).
    pub fn logout(&self) -> Result<(), CoreError> {
        *self.session_slot() = None;
        self.store.clear_session()?;
        self.store.clear_desktop_keys()?;
        let now = unix_now();
        for row in self.store.pending_rows(now)? {
            self.notifier.item_resolved(row.id);
        }
        self.store.delete_all_pending()
    }

    /// Makes this phone the account's approval device. When another device approves for the account, the server wants
    /// a proof that this one may take over: the other device's yes to this phone's "add another phone" request (the
    /// server remembers it), or the master password hash of the account secret this phone keeps or of the password it
    /// signed in or unlocked with. Without one: [`CoreError::OtherApprovalDevice`], and the app offers those two ways.
    pub async fn register_device(&self, fcm_token: Option<String>) -> Result<(), CoreError> {
        let session = self.session()?;
        let mut registration = DeviceRegistration {
            fcm_token,
            master_password_hash: None,
        };
        match api_call!(&session, |api| api.register_device(&registration)) {
            Ok(_) => {
                session.forget_proof();
                return Ok(());
            }
            Err(e) if !is_takeover_refusal(&e) => return Err(e.into_core()),
            Err(_) => {}
        }
        let Some(proof) = self.takeover_proof(&session).await? else {
            return Err(CoreError::OtherApprovalDevice);
        };
        registration.master_password_hash = Some(proof.to_string());
        let result = api_call!(&session, |api| api.register_device(&registration));
        if let Some(sent) = registration.master_password_hash.take() {
            drop(Zeroizing::new(sent));
        }
        match result {
            Ok(_) => {
                session.forget_proof();
                Ok(())
            }
            Err(e) if is_takeover_refusal(&e) => Err(CoreError::OtherApprovalDevice),
            Err(e) => Err(e.into_core()),
        }
    }

    /// The master password hash that lets this phone take the approval role: of the password this session signed in
    /// or unlocked with, else of the account secret this phone keeps.
    async fn takeover_proof(&self, session: &Session) -> Result<Option<Zeroizing<String>>, CoreError> {
        if let Some(proof) = session.proof() {
            return Ok(Some(proof));
        }
        let Some(secret) = self.signed_in_secret().await? else {
            return Ok(None);
        };
        let hash = tokio::task::spawn_blocking(move || -> Result<Zeroizing<String>, CoreError> {
            let master = secret.master_key()?;
            Ok(secret.master_password_hash(&master))
        })
        .await
        .map_err(|_| CoreError::storage("key derivation was interrupted"))??;
        Ok(Some(hash))
    }

    /// Tells the server which integrations have an account here and which MCP servers were added with their tools,
    /// when that changed since the last report, so that it lists only those tools to the AIs. A failure is retried
    /// with the next sync.
    pub(crate) async fn report_services(&self, session: &Session) {
        let mut services = self.integrations();
        services.sort();
        let report = reins_proto::device::ServicesReport {
            services,
            mcp: self.mcp_reports(),
        };
        if self.reported_services.lock().unwrap_or_else(PoisonError::into_inner).as_ref() == Some(&report) {
            return;
        }
        match api_call!(session, |api| api.put_services(&report)) {
            Ok(()) => *self.reported_services.lock().unwrap_or_else(PoisonError::into_inner) = Some(report),
            Err(e) => log::warn!("could not report the integrations: {}", e.into_core()),
        }
    }

    // ---- intake --------------------------------------------------------------------

    /// FCM entry point. `kind` is `req`, `pair`, `blob` or `replaced`; the payload is only an id. What was parked then
    /// goes through Autopilot's pass (which tells the app about what waits).
    pub async fn handle_push(&self, kind: &str, id: &str) -> Result<(), CoreError> {
        let result = self.handle_push_inner(kind, id).await;
        self.autopilot_pass().await;
        result
    }

    /// Like [`Engine::handle_push`], without Autopilot's pass: for a process that cannot run the model (the iOS
    /// notification extension). The item is parked and listed by `pending`, but not judged; the next pass in a
    /// process that has the model (the app's sync or push) judges it, instead of this one recording that it could not.
    pub async fn handle_push_deferring_autopilot(&self, kind: &str, id: &str) -> Result<(), CoreError> {
        self.handle_push_inner(kind, id).await
    }

    async fn handle_push_inner(&self, kind: &str, id: &str) -> Result<(), CoreError> {
        match kind {
            "replaced" => {
                log::info!("this device is no longer the Reins approval device");
                Ok(())
            }
            "blob" => self.handle_blob_push(id).await,
            "join" => self.fetch_and_park_join(id).await,
            "req" | "pair" => {
                check_id(id)?;
                let session = self.session()?;
                let Some(_guard) = self.begin(id)? else {
                    return Ok(());
                };
                if kind == "req" {
                    let fetched = api_call!(&session, |api| api.get_request(id));
                    match fetched {
                        Ok(request) => self.process_request_unguarded(&session, request).await,
                        Err(ApiFailure::Status {
                            status: 404,
                            ..
                        }) => Ok(()),
                        Err(e) => Err(e.into_core()),
                    }
                } else {
                    let fetched = api_call!(&session, |api| api.get_pairing(id));
                    match fetched {
                        Ok(pairing) => self.park_pairing(pairing),
                        Err(ApiFailure::Status {
                            status: 404,
                            ..
                        }) => Ok(()),
                        Err(e) => Err(e.into_core()),
                    }
                }
            }
            _ => Err(CoreError::invalid("unknown push type")),
        }
    }

    /// Long-polls the server (foreground channel) and processes what arrives, then runs Autopilot's pass.
    pub async fn sync(&self, wait_secs: u32) -> Result<Vec<PendingItem>, CoreError> {
        let result = self.sync_inner(wait_secs).await;
        self.autopilot_pass().await;
        result?;
        self.pending()
    }

    async fn sync_inner(&self, wait_secs: u32) -> Result<(), CoreError> {
        let session = self.session()?;
        self.report_services(&session).await;
        let pending = api_call!(&session, |api| api.pending(wait_secs)).map_err(ApiFailure::into_core)?;
        if let Some(email) = pending.account_email.as_deref() {
            session.adopt_account_email(email).await?;
        }
        for request in pending.requests {
            if let Err(e) = self.process_request(&session, request).await {
                log::warn!("could not process a request: {e}");
            }
        }
        for pairing in pending.pairings {
            let Some(_guard) = self.begin(&pairing.id.0)? else {
                continue;
            };
            if let Err(e) = self.park_pairing(pairing) {
                log::warn!("could not park a pairing: {e}");
            }
        }
        for join in pending.joins {
            let Some(_guard) = self.begin(&join.id)? else {
                continue;
            };
            if let Err(e) = self.park_join(&join) {
                log::warn!("could not park a request from another phone: {e}");
            }
        }
        self.park_blobs(pending.blobs)
    }

    // ---- read-only views -----------------------------------------------------------

    pub fn pending(&self) -> Result<Vec<PendingItem>, CoreError> {
        let mut items = Vec::new();
        for row in self.store.pending_rows(unix_now())? {
            let item = match row.kind {
                PendingKind::Request => {
                    serde_json::from_slice::<ParkedRequest>(&row.payload).ok().map(|p| views::request_item(&p))
                }
                PendingKind::Pairing => serde_json::from_slice(&row.payload).ok().map(|p| views::pairing_item(&p)),
                PendingKind::Blob => Self::blob_pending_item(&row.payload),
                PendingKind::Join => serde_json::from_slice(&row.payload).ok().map(|j| crate::join::join_item(&j)),
            };
            if let Some(mut item) = item {
                item.suggestion = self
                    .store
                    .ap_suggestion(&item.id)
                    .ok()
                    .flatten()
                    .and_then(|s| crate::autopilot::suggestion_line(&s));
                items.push(item);
            }
        }
        Ok(items)
    }

    pub fn grants(&self) -> Result<Vec<GrantView>, CoreError> {
        Ok(self.store.grants()?.iter().map(views::grant_view).collect())
    }

    /// Starts an ended grant over with the changes the user made: how long, how many uses, and who or what it covers.
    /// The kind (read or send) stays as it was.
    pub fn resume_grant_edited(&self, grant_id: &str, standing: &crate::types::StandingGrant) -> Result<(), CoreError> {
        let secs = standing.duration_secs.ok_or_else(|| CoreError::invalid("choose for how long"))?;
        if !(reins_proto::gmail::MIN_GRANT_SECS..=reins_proto::gmail::MAX_GRANT_SECS).contains(&secs) {
            return Err(CoreError::invalid("choose a period between a minute and 30 days"));
        }
        if standing.scope.all_mail && secs > reins_proto::gmail::MAX_ANY_GRANT_SECS {
            return Err(CoreError::invalid("access to all mail can be resumed for a week at most"));
        }
        let now = unix_now();
        let resumed = self.store.resume_grant_with(&GrantId(grant_id.to_owned()), now, |old| {
            let kind = match old.scope {
                reins_policy::Scope::Read(_) => crate::types::ApprovalKind::Read,
                reins_policy::Scope::Send(_) => crate::types::ApprovalKind::Send,
                reins_policy::Scope::Accounts(_) | reins_policy::Scope::Service(_) => {
                    return Err(CoreError::invalid("this grant can only be resumed as it was"));
                }
            };
            crate::approval::build_standing_grant(standing, kind, &[], &old.connection_id, now)
        })?;
        if resumed {
            Ok(())
        } else {
            Err(CoreError::NotFound)
        }
    }

    /// One email of an activity entry, fetched from Gmail now (only the id was kept). `account` is the entry's account.
    pub async fn fetch_email(
        &self,
        account: Option<&str>,
        message_id: &str,
    ) -> Result<crate::types::EmailContent, CoreError> {
        let account = self.resolve_account(account).await.map_err(CoreError::gmail)?;
        let mut found = self.gmail_for(&account).fetch(&[message_id.to_owned()], true).await?;
        let Some(parsed) = found.pop() else {
            return Err(CoreError::gmail("that email is no longer in Gmail"));
        };
        let full = parsed.into_full();
        let m = full.summary;
        let sender = match &m.from_name {
            Some(name) if !name.trim().is_empty() => format!("{} <{}>", crate::text::one_line(name), m.from),
            _ => m.from.clone(),
        };
        Ok(crate::types::EmailContent {
            id: m.id,
            from: sender,
            to: m.to,
            cc: m.cc,
            subject: crate::text::one_line(&m.subject),
            date: m.date,
            body: crate::text::neutralize(&full.body_text),
        })
    }

    /// Removes an ended grant for good.
    pub fn delete_grant(&self, grant_id: &str) -> Result<(), CoreError> {
        if self.store.delete_ended_grant(&GrantId(grant_id.to_owned()), unix_now())? {
            Ok(())
        } else {
            Err(CoreError::NotFound)
        }
    }

    /// Starts a grant that ended (expired, used up or deleted) over for `duration_secs`.
    pub fn resume_grant(&self, grant_id: &str, duration_secs: u64) -> Result<(), CoreError> {
        if !(reins_proto::gmail::MIN_GRANT_SECS..=reins_proto::gmail::MAX_GRANT_SECS).contains(&duration_secs) {
            return Err(CoreError::invalid("choose a period between a minute and 30 days"));
        }
        let secs = i64::try_from(duration_secs).unwrap_or(i64::MAX);
        if self.store.resume_grant(&GrantId(grant_id.to_owned()), secs, unix_now())? {
            Ok(())
        } else {
            Err(CoreError::NotFound)
        }
    }

    pub fn revoke_grant(&self, grant_id: &str) -> Result<(), CoreError> {
        if self.store.revoke_grant(&GrantId(grant_id.to_owned()))? {
            Ok(())
        } else {
            Err(CoreError::NotFound)
        }
    }

    pub async fn connections(&self) -> Result<Vec<ConnectionView>, CoreError> {
        let session = self.session()?;
        let list = api_call!(&session, |api| api.connections()).map_err(ApiFailure::into_core)?;
        Ok(list
            .connections
            .into_iter()
            .map(|c| ConnectionView {
                icon: self.store.connection_icon(&c.id.0).ok().flatten(),
                id: c.id.0,
                label: crate::text::truncate_chars(&crate::text::one_line(&c.label), 64),
                client_host: crate::text::one_line(&c.client_host),
                created_at: c.created_at,
                last_used_at: c.last_used_at,
            })
            .collect())
    }

    /// Remembers the icon the user picked for a connection (`None` = automatic).
    pub fn set_connection_icon(&self, connection_id: &str, icon: Option<String>) -> Result<(), CoreError> {
        check_id(connection_id)?;
        let icon = icon.map(|i| crate::text::truncate_chars(&crate::text::one_line(&i), 32)).filter(|i| !i.is_empty());
        self.store.set_connection_icon(connection_id, icon.as_deref())
    }

    /// Revokes a connection at the server, its grants here and its parked requests.
    pub async fn revoke_connection(&self, connection_id: &str) -> Result<(), CoreError> {
        check_id(connection_id)?;
        let session = self.session()?;
        match api_call!(&session, |api| api.delete_connection(connection_id)) {
            Ok(())
            | Err(ApiFailure::Status {
                status: 404,
                ..
            }) => {}
            Err(e) => return Err(e.into_core()),
        }
        let connection = ConnectionId(connection_id.to_owned());
        self.store.remove_connection_grants(&connection)?;
        self.store.remove_desktop_key(connection_id)?;
        self.store.set_connection_icon(connection_id, None)?;
        for row in self.store.pending_rows(unix_now())? {
            if row.kind != PendingKind::Request {
                continue;
            }
            let matches = serde_json::from_slice::<ParkedRequest>(&row.payload)
                .is_ok_and(|p| p.request.connection_id == connection);
            if matches && self.store.remove_pending(&row.id)? {
                self.notifier.item_resolved(row.id);
            }
        }
        Ok(())
    }

    pub fn activity(&self, limit: u32) -> Result<Vec<ActivityEntry>, CoreError> {
        Ok(self.store.activity(limit.clamp(1, 500))?.iter().map(views::activity_entry).collect())
    }

    /// Gmail as a whole: ready when every connected account is; otherwise the first problem found.
    /// With no account registered yet, the phone's default Google account is tried (and registered).
    pub async fn gmail_status(&self) -> GmailStatus {
        if self.gmail_accounts().is_empty() {
            return match self.resolve_account(None).await {
                Ok(_) => GmailStatus::Ready,
                Err(_) => self.account_status("").await,
            };
        }
        for account in self.gmail_accounts() {
            let status = self.account_status(&account).await;
            if status != GmailStatus::Ready {
                return status;
            }
        }
        GmailStatus::Ready
    }
}

/// `PUT /device` refused: another device approves for the account, and this one's proof was missing or wrong.
fn is_takeover_refusal(e: &ApiFailure) -> bool {
    matches!(e, ApiFailure::Status { status: 403, code, .. } if code == codes::PROOF_REQUIRED || code == codes::WRONG_PROOF)
}
