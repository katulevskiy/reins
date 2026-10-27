//! The object Kotlin holds (contracts §D). Every method runs on the core runtime
//! (`rt::run`), so the calling coroutine never blocks and a cancelled coroutine
//! never aborts work that already reached the server.

mod payments;
mod permissions;

use std::path::PathBuf;
use std::sync::Arc;

use reins_proto::oplog::Restriction;
use zeroize::Zeroizing;

use crate::autopilot::{
    AutoQuestion, AutoSetup, AutopilotMode, AutopilotSettings, AutopilotStats, CellChoice, DownloadProgress,
    FactChoice, ModelRuntime, ModelStatus, ModelUpdateOutcome, OvernightRisky, Preset, ProfileView, SuggestionView,
    ThresholdReplay, Verdict, ZoneChoice,
};
use crate::connector::device::{DeviceBridge, DeviceCalendar, DeviceContacts, Sms};
use crate::connector::{Connector, LoginProgress};
use crate::engine::{CoreConfig, Engine};
use crate::live_sync::{LiveSyncState, ProposalPreview, ProposalView};
use crate::roles::{
    EnrollProgress, EnrollStart, EnrollView, HandoverProgress, HandoverView, PhoneRole, RecoveryPreview, RosterSummary,
};
use crate::types::{
    AccountKeys, AccountView, ActivityEntry, ApprovalChoice, ApprovalKind, ApprovalView, ConnectionView, DeviceView,
    EmailContent, GmailStatus, GrantView, JoinProgress, JoinStart, JoinView, PairingView, PendingItem, SecretsRevoked,
    ServerInfo, ServiceView, SessionInfo, SsoOutcome, SsoStart, StandingGrant, StartingPolicy,
};
use crate::vault_editor::{VaultItemDetail, VaultItemInput, VaultItemSummary, VaultSshKey};
use crate::vault_passkey::{VaultPasskeyOptions, VaultPasskeyView};
use crate::{CoreError, GoogleTokenProvider, KeyWrapper, Notifier, OwnerCheck, StoreLockGuard, rt};

/// Installs the platform's guard over the store's cross-process file lock, and how long taking the lock may wait for
/// another process before a call fails as busy (iOS: the app and its notification extension share the store in the
/// App Group container; see [`StoreLockGuard`]). Call once, before the first `ReinsCore` in the process.
#[uniffi::export]
pub fn install_store_lock_guard(guard: Arc<dyn StoreLockGuard>, wait_ms: u32) {
    crate::store::install_lock_guard(guard, std::time::Duration::from_millis(u64::from(wait_ms)));
}

#[derive(uniffi::Object)]
pub struct ReinsCore {
    pub(crate) runtime: Arc<crate::account_runtime::AccountRuntime>,
}

impl ReinsCore {
    /// The integrations that live on this phone, when it can run them and this build of the app offers them.
    fn device_connectors(device: Option<Arc<dyn DeviceBridge>>) -> Vec<Arc<dyn Connector>> {
        let Some(bridge) = device else {
            return Vec::new();
        };
        let offered = bridge.services();
        let calendar: Arc<dyn Connector> = Arc::new(DeviceCalendar::new(Arc::clone(&bridge)));
        let contacts: Arc<dyn Connector> = Arc::new(DeviceContacts::new(Arc::clone(&bridge)));
        let sms: Arc<dyn Connector> = Arc::new(Sms::new(bridge));
        [calendar, contacts, sms].into_iter().filter(|c| offered.iter().any(|s| s == c.service())).collect()
    }

    /// Rust-only constructor with test overrides (Gmail base URL, backoff).
    pub fn with_config(
        data_dir: &str,
        keys: &dyn KeyWrapper,
        google: Arc<dyn GoogleTokenProvider>,
        notifier: Arc<dyn Notifier>,
        cfg: CoreConfig,
    ) -> Result<Arc<Self>, CoreError> {
        Self::with_connectors(data_dir, keys, google, notifier, cfg, Vec::new())
    }

    /// Like [`ReinsCore::with_config`], with these integrations added (tests bring their own).
    pub fn with_connectors(
        data_dir: &str,
        keys: &dyn KeyWrapper,
        google: Arc<dyn GoogleTokenProvider>,
        notifier: Arc<dyn Notifier>,
        cfg: CoreConfig,
        connectors: Vec<Arc<dyn Connector>>,
    ) -> Result<Arc<Self>, CoreError> {
        let runtime = crate::account_runtime::AccountRuntime::new(
            PathBuf::from(data_dir),
            keys,
            google,
            notifier,
            cfg,
            connectors,
        )?;
        Ok(Arc::new(Self {
            runtime,
        }))
    }

    /// The engine, for Rust integration tests.
    pub fn engine(&self) -> Arc<Engine> {
        self.runtime.engine()
    }
}

#[uniffi::export]
impl ReinsCore {
    #[uniffi::constructor]
    #[expect(clippy::needless_pass_by_value, reason = "UniFFI hands constructor arguments over by value")]
    pub fn new(
        data_dir: String,
        keys: Arc<dyn KeyWrapper>,
        google: Arc<dyn GoogleTokenProvider>,
        notifier: Arc<dyn Notifier>,
        device: Option<Arc<dyn DeviceBridge>>,
        telegram_api_id: i32,
        telegram_api_hash: String,
    ) -> Result<Arc<Self>, CoreError> {
        let cfg = CoreConfig {
            telegram_api_id,
            telegram_api_hash,
            ..CoreConfig::default()
        };
        Self::with_connectors(&data_dir, keys.as_ref(), google, notifier, cfg, Self::device_connectors(device))
    }

    /// The signed-in session; `None` when signed out. An error says the data could not be read now (another process
    /// holds it, the app is being suspended, a failure in the core): it says nothing about the session, and the same
    /// call later does.
    pub async fn session(&self) -> Result<Option<SessionInfo>, CoreError> {
        let runtime = Arc::clone(&self.runtime);
        rt::run_caught(async move { runtime.session().await }).await
    }

    /// Whether the app is in the background, where iOS gives it seconds and ends a process that grows: Autopilot's
    /// pass then does not load the model; what it would judge waits for the next pass in front (Bypass and Lockdown
    /// still decide, and a model already loaded still judges).
    pub fn set_background(&self, background: bool) {
        self.runtime.set_background(background);
    }

    pub async fn login(
        &self,
        server_url: String,
        email: String,
        password: String,
        totp: Option<String>,
    ) -> Result<SessionInfo, CoreError> {
        let runtime = Arc::clone(&self.runtime);
        let password = Zeroizing::new(password);
        rt::run(async move { runtime.login(&server_url, &email, password, totp).await }).await
    }

    /// Starts a sign-in through the server's SSO ("Continue": Google, Apple, GitHub or an email code on
    /// app.reins2fa.com). Open `url` in ASWebAuthenticationSession / a Custom Tab, wait for `callback_scheme`, then
    /// call `sso_finish` with the URL it came back with and this `state` and `verifier`.
    pub async fn sso_begin(&self, server_url: String) -> Result<SsoStart, CoreError> {
        let runtime = Arc::clone(&self.runtime);
        let generation = runtime.generation();
        rt::run(async move {
            let engine = runtime.prepare(generation).await?;
            engine.sso_begin(&server_url)
        })
        .await
    }

    /// "Add work account": like `sso_begin`, sent straight to the organization that verified `domain` (`acme.com`)
    /// through its SSO or Team provider. Run it on the work profile's own core (`ProfileRegistry`), then `sso_finish`.
    pub async fn sso_begin_work(&self, server_url: String, domain: String) -> Result<SsoStart, CoreError> {
        let runtime = Arc::clone(&self.runtime);
        let generation = runtime.generation();
        rt::run(async move {
            let engine = runtime.prepare(generation).await?;
            engine.sso_begin_work(&server_url, &domain)
        })
        .await
    }

    /// The tag this account's pushes carry (`None` signed out): the profile a push is for.
    pub async fn push_tag(&self) -> Result<Option<String>, CoreError> {
        let runtime = Arc::clone(&self.runtime);
        let generation = runtime.generation();
        rt::run(async move {
            let engine = runtime.prepare(generation).await?;
            engine.push_tag().await
        })
        .await
    }

    /// Whether `server_url` offers organizations ("Add work account" is shown only then).
    pub async fn orgs_offered(&self, server_url: String) -> Result<bool, CoreError> {
        let runtime = Arc::clone(&self.runtime);
        let generation = runtime.generation();
        rt::run(async move {
            let engine = runtime.prepare(generation).await?;
            engine.orgs_offered(&server_url).await
        })
        .await
    }

    /// The signed-in account's organization (a work profile's); `None` for a Personal account.
    pub async fn org_membership(&self) -> Result<Option<crate::org::OrgView>, CoreError> {
        let runtime = Arc::clone(&self.runtime);
        let generation = runtime.generation();
        rt::run(async move {
            let engine = runtime.prepare(generation).await?;
            engine.ensure_active()?;
            let result = engine.run_account(async { engine.org_membership().await }).await;
            runtime.finish(&engine, result)
        })
        .await
    }

    /// A fresh challenge for enrolling this device to the work account's org: sign
    /// `org_enrollment_message(challenge)` with the device key, then call `org_enroll`.
    pub async fn org_challenge(&self) -> Result<String, CoreError> {
        let runtime = Arc::clone(&self.runtime);
        let generation = runtime.generation();
        rt::run(async move {
            let engine = runtime.prepare(generation).await?;
            engine.ensure_active()?;
            let result = engine.run_account(async { engine.org_challenge().await }).await;
            runtime.finish(&engine, result)
        })
        .await
    }

    /// Enrolls this device to the work account's org (06 §6): the device key from secure hardware, its signature
    /// over the challenge, the platform's attestations and the posture.
    pub async fn org_enroll(&self, challenge: String, input: crate::org::EnrollInput) -> Result<(), CoreError> {
        let runtime = Arc::clone(&self.runtime);
        let generation = runtime.generation();
        rt::run(async move {
            let engine = runtime.prepare(generation).await?;
            engine.ensure_active()?;
            let result = engine.run_account(async { engine.org_enroll(challenge, input).await }).await;
            runtime.finish(&engine, result)
        })
        .await
    }

    /// Reports the enrolled device's posture now (at start-up and when it changes).
    pub async fn org_report_posture(&self, posture: crate::org::PostureInput) -> Result<(), CoreError> {
        let runtime = Arc::clone(&self.runtime);
        let generation = runtime.generation();
        rt::run(async move {
            let engine = runtime.prepare(generation).await?;
            engine.ensure_active()?;
            let result = engine.run_account(async { engine.org_report_posture(posture).await }).await;
            runtime.finish(&engine, result)
        })
        .await
    }

    /// What `server_url` offers before signing in: whether "Continue" (browser sign-in) works there, and whether it
    /// can wake each phone app. Unknown values are `None`; an unreachable server is no error.
    pub async fn server_info(&self, server_url: String) -> Result<ServerInfo, CoreError> {
        let runtime = Arc::clone(&self.runtime);
        let generation = runtime.generation();
        rt::run(async move {
            let engine = runtime.prepare(generation).await?;
            engine.server_info(&server_url).await
        })
        .await
    }

    /// Finishes the sign-in, keeps the session like `login`, and makes (a new account) or opens the account's keys.
    /// `keys` is `Locked` when another phone or the recovery code has what opens them.
    pub async fn sso_finish(
        &self,
        server_url: String,
        callback_url: String,
        state: String,
        verifier: String,
    ) -> Result<SsoOutcome, CoreError> {
        let runtime = Arc::clone(&self.runtime);
        let verifier = Zeroizing::new(verifier);
        rt::run(async move { runtime.sso_finish(&server_url, &callback_url, &state, verifier).await }).await
    }

    /// "Reset the vault" on the Unlock screen, for a `Locked` account whose other phone and recovery code are both
    /// lost: everything in the vault is deleted and the account gets new keys, as a new account does (`keys` is
    /// `Created`; record the new recovery code). Start a sign-in with `sso_begin` and open it in a browser session that
    /// does not reuse the last one (the person signs in again to confirm), then hand its callback here. An `Invalid`
    /// error (a sign-in to another account, or one that ran out) leaves the account as it was.
    pub async fn reset_account(
        &self,
        server_url: String,
        callback_url: String,
        state: String,
        verifier: String,
    ) -> Result<SsoOutcome, CoreError> {
        let runtime = Arc::clone(&self.runtime);
        let verifier = Zeroizing::new(verifier);
        let generation = runtime.generation();
        rt::run(async move {
            runtime.prepare(generation).await?;
            runtime.reset_account(&server_url, &callback_url, &state, verifier).await
        })
        .await
    }

    /// A problem syncing with the account's other phones, for a banner the user can dismiss (`None`: nothing to show).
    /// Sync never keeps the app closed or signed out; this is all it ever shows.
    pub async fn account_sync_notice(&self) -> Result<Option<String>, CoreError> {
        let runtime = Arc::clone(&self.runtime);
        let generation = runtime.generation();
        rt::run_caught(async move {
            let engine = match runtime.prepare(generation).await {
                Err(CoreError::NotLoggedIn) => return Ok(None),
                engine => engine?,
            };
            engine.account_sync_notice()
        })
        .await
    }

    /// The banner of [`Self::account_sync_notice`] was dismissed.
    pub async fn dismiss_account_sync_notice(&self) -> Result<(), CoreError> {
        let runtime = Arc::clone(&self.runtime);
        let generation = runtime.generation();
        rt::run(async move {
            let engine = runtime.prepare(generation).await?;
            let result = engine.dismiss_account_sync_notice();
            runtime.finish(&engine, result)
        })
        .await
    }

    /// Only when the user asks: brings back this phone's own changes that a sync without a merge base kept aside (the
    /// notice says so), merged with the data as it is now by the sync's rules (never a grant only the backup has).
    /// False when there is nothing to bring back. What comes back is uploaded like any change.
    pub async fn restore_account_sync_backup(&self) -> Result<bool, CoreError> {
        let runtime = Arc::clone(&self.runtime);
        let generation = runtime.generation();
        rt::run(async move {
            let engine = runtime.prepare(generation).await?;
            engine.require_decider()?;
            let result = engine.restore_account_sync_backup();
            runtime.finish(&engine, result)
        })
        .await
    }

    /// Whether this phone can open the signed-in account's vault.
    pub async fn account_keys(&self) -> Result<AccountKeys, CoreError> {
        let runtime = Arc::clone(&self.runtime);
        let generation = runtime.generation();
        rt::run(async move {
            let engine = runtime.prepare(generation).await?;
            engine.account_keys().await
        })
        .await
    }

    /// Opens a `Locked` account with its recovery code (case, spaces and dashes do not count), or with the master
    /// password of an account made with one.
    pub async fn unlock_account(&self, code_or_password: String) -> Result<(), CoreError> {
        let runtime = Arc::clone(&self.runtime);
        let secret = Zeroizing::new(code_or_password);
        let generation = runtime.generation();
        rt::run(async move {
            let engine = runtime.prepare(generation).await?;
            engine.run_account(engine.unlock_account(secret)).await?;
            runtime.check_engine(&engine)?;
            runtime.activate_unlocked().await
        })
        .await
    }

    /// What the app hands the platform's passkey UI to make a vault passkey or use one (see `vault_passkey`).
    pub async fn vault_passkey_options(&self) -> Result<VaultPasskeyOptions, CoreError> {
        let runtime = Arc::clone(&self.runtime);
        let generation = runtime.generation();
        rt::run(async move {
            let engine = runtime.prepare(generation).await?;
            let options = engine.run_account(engine.vault_passkey_options()).await?;
            runtime.check_engine(&engine)?;
            Ok(options)
        })
        .await
    }

    /// The passkeys that open the account's vault.
    pub async fn vault_passkeys(&self) -> Result<Vec<VaultPasskeyView>, CoreError> {
        let runtime = Arc::clone(&self.runtime);
        let generation = runtime.generation();
        rt::run(async move {
            let engine = runtime.prepare(generation).await?;
            let list = engine.run_account(engine.vault_passkeys()).await?;
            runtime.check_engine(&engine)?;
            Ok(list)
        })
        .await
    }

    /// The passkey just made (its credential id and PRF output) opens the vault from now on. Only on a phone whose
    /// vault is open.
    pub async fn add_vault_passkey(
        &self,
        credential_id: Vec<u8>,
        prf_output: Vec<u8>,
        name: String,
    ) -> Result<Vec<VaultPasskeyView>, CoreError> {
        let runtime = Arc::clone(&self.runtime);
        let prf = Zeroizing::new(prf_output);
        let generation = runtime.generation();
        rt::run(async move {
            let engine = runtime.prepare(generation).await?;
            engine.require_decider()?;
            let list = engine.run_account(engine.add_vault_passkey(&credential_id, &prf, &name)).await?;
            runtime.check_engine(&engine)?;
            Ok(list)
        })
        .await
    }

    pub async fn remove_vault_passkey(&self, credential_id: Vec<u8>) -> Result<Vec<VaultPasskeyView>, CoreError> {
        let runtime = Arc::clone(&self.runtime);
        let generation = runtime.generation();
        rt::run(async move {
            let engine = runtime.prepare(generation).await?;
            engine.require_decider()?;
            let list = engine.run_account(engine.remove_vault_passkey(&credential_id)).await?;
            runtime.check_engine(&engine)?;
            Ok(list)
        })
        .await
    }

    /// Opens a `Locked` account with a passkey it added for its vault (the credential id and PRF output the
    /// platform's passkey UI returned), as the recovery code does.
    pub async fn unlock_with_vault_passkey(
        &self,
        credential_id: Vec<u8>,
        prf_output: Vec<u8>,
    ) -> Result<(), CoreError> {
        let runtime = Arc::clone(&self.runtime);
        let prf = Zeroizing::new(prf_output);
        let generation = runtime.generation();
        rt::run(async move {
            let engine = runtime.prepare(generation).await?;
            engine.run_account(engine.unlock_with_vault_passkey(&credential_id, &prf)).await?;
            runtime.check_engine(&engine)?;
            runtime.activate_unlocked().await
        })
        .await
    }

    /// The account's recovery code (`ABCD-EFGH-...`, 13 groups), when this phone keeps its secret. Ask for biometrics
    /// before showing it.
    pub async fn account_recovery_code(&self) -> Result<String, CoreError> {
        let runtime = Arc::clone(&self.runtime);
        let generation = runtime.generation();
        rt::run(async move {
            let engine = runtime.prepare(generation).await?;
            let result = engine.run_account(async { engine.account_recovery_code().await }).await;
            runtime.check_engine(&engine)?;
            result
        })
        .await
    }

    /// "Add another phone", on the new phone (keys `Locked`): asks the account's approval device for its keys. Show
    /// `code` and ask the user to check that the other phone shows the same, then call `join_poll` every few
    /// seconds.
    pub async fn join_begin(&self, device_name: String) -> Result<JoinStart, CoreError> {
        let runtime = Arc::clone(&self.runtime);
        let generation = runtime.generation();
        rt::run(async move {
            let engine = runtime.prepare(generation).await?;
            engine.join_begin(&device_name).await
        })
        .await
    }

    /// Where the request stands; `Joined` once the keys are open on this phone. `NotFound` when none is open.
    pub async fn join_poll(&self) -> Result<JoinProgress, CoreError> {
        let runtime = Arc::clone(&self.runtime);
        let generation = runtime.generation();
        rt::run(async move {
            let engine = runtime.prepare(generation).await?;
            let progress = engine.run_account(engine.join_poll()).await?;
            runtime.check_engine(&engine)?;
            if progress == JoinProgress::Joined {
                runtime.activate_unlocked().await?;
            }
            Ok(progress)
        })
        .await
    }

    pub async fn join_cancel(&self) -> Result<(), CoreError> {
        let runtime = Arc::clone(&self.runtime);
        let generation = runtime.generation();
        rt::run(async move {
            let engine = runtime.prepare(generation).await?;
            engine.join_cancel()
        })
        .await
    }

    /// On the approval device: whether the phone asking (`PendingKind::Join`) asks to view the account as a second
    /// phone (`enroll_view`, then `enroll_confirm`) or for its keys (`join_view`, then `answer_join`).
    pub async fn join_is_enrollment(&self, id: String) -> Result<bool, CoreError> {
        let runtime = Arc::clone(&self.runtime);
        let generation = runtime.generation();
        rt::run(async move {
            let engine = runtime.prepare(generation).await?;
            engine.run_account(async { engine.join_is_enrollment(&id) }).await
        })
        .await
    }

    /// On the approval device: the phone asking (`PendingKind::Join`), with the code it should show.
    pub async fn join_view(&self, id: String) -> Result<JoinView, CoreError> {
        let runtime = Arc::clone(&self.runtime);
        let generation = runtime.generation();
        rt::run(async move {
            let engine = runtime.prepare(generation).await?;
            engine.ensure_active()?;
            let result = engine.run_account(async { engine.join_view(&id) }).await;
            runtime.finish(&engine, result)
        })
        .await
    }

    /// On the approval device: gives the account's keys to the asking phone (sealed to it), or refuses. Ask for
    /// biometrics before approving.
    pub async fn answer_join(&self, id: String, approve: bool) -> Result<(), CoreError> {
        let runtime = Arc::clone(&self.runtime);
        let generation = runtime.generation();
        rt::run(async move {
            let engine = runtime.prepare(generation).await?;
            engine.require_decider()?;
            engine.ensure_active()?;
            let result = engine.run_account(async { engine.answer_join(&id, approve).await }).await;
            runtime.finish(&engine, result)
        })
        .await
    }

    // ---- phone roles: one primary decides, viewers watch ----------------------------------------------------------

    /// What this phone is on the signed-in account: `Legacy` while the account has no roster (or this phone is not in
    /// it), else `Primary`, `Viewer` or `Removed`. From what this phone verified last; no network.
    pub async fn phone_role(&self) -> Result<PhoneRole, CoreError> {
        let runtime = Arc::clone(&self.runtime);
        let generation = runtime.generation();
        rt::run(async move {
            let engine = runtime.prepare(generation).await?;
            engine.run_account(engine.phone_role()).await
        })
        .await
    }

    /// One cycle of the live sync on an account with phone roles: sends this phone's changes, applies the log, checks
    /// the roster and the primary's heartbeat. `wait_secs` (at most 25) long-polls for news when there is none; a
    /// viewer's foreground loop calls it again and again. On an account without phone roles it only says so.
    pub async fn live_sync(&self, wait_secs: u32) -> Result<LiveSyncState, CoreError> {
        let runtime = Arc::clone(&self.runtime);
        let generation = runtime.generation();
        rt::run(async move {
            let engine = runtime.prepare(generation).await?;
            engine.ensure_active()?;
            let result = engine.run_account(engine.live_sync(wait_secs)).await;
            runtime.finish(&engine, result)
        })
        .await
    }

    /// Where the live view stood after the last cycle (no network): the "Live" / "last seen" bar.
    pub async fn live_sync_state(&self) -> Result<LiveSyncState, CoreError> {
        let runtime = Arc::clone(&self.runtime);
        let generation = runtime.generation();
        rt::run(async move {
            let engine = runtime.prepare(generation).await?;
            engine.run_account(async { engine.live_sync_state() }).await
        })
        .await
    }

    /// On a viewer: proposes an account, a token, an MCP server or an icon (`rows_json`: a list of
    /// `{"table", "value"}`) for the primary to keep or discard. Returns its id.
    pub async fn proposal_create(&self, kind: String, title: String, rows_json: String) -> Result<String, CoreError> {
        let runtime = Arc::clone(&self.runtime);
        let generation = runtime.generation();
        rt::run(async move {
            let engine = runtime.prepare(generation).await?;
            engine.ensure_active()?;
            let result = engine.run_account(engine.proposal_create(&kind, &title, &rows_json)).await;
            runtime.finish(&engine, result)
        })
        .await
    }

    /// On the viewer that proposed it: takes a proposal back.
    pub async fn proposal_withdraw(&self, id: String) -> Result<(), CoreError> {
        let runtime = Arc::clone(&self.runtime);
        let generation = runtime.generation();
        rt::run(async move {
            let engine = runtime.prepare(generation).await?;
            engine.ensure_active()?;
            let result = engine.run_account(engine.proposal_withdraw(&id)).await;
            runtime.finish(&engine, result)
        })
        .await
    }

    /// The proposals waiting for the primary (on every phone of the roster).
    pub async fn proposals(&self) -> Result<Vec<ProposalView>, CoreError> {
        let runtime = Arc::clone(&self.runtime);
        let generation = runtime.generation();
        rt::run(async move {
            let engine = runtime.prepare(generation).await?;
            engine.run_account(engine.proposals()).await
        })
        .await
    }

    /// On the primary: what a proposal really adds (opened with the credential key). Show Keep / Discard from this,
    /// never from the proposal's kind and title, and hand its digest to `proposal_keep`.
    pub async fn proposal_preview(&self, id: String) -> Result<ProposalPreview, CoreError> {
        let runtime = Arc::clone(&self.runtime);
        let generation = runtime.generation();
        rt::run(async move {
            let engine = runtime.prepare(generation).await?;
            engine.require_decider()?;
            engine.run_account(engine.proposal_preview(&id)).await
        })
        .await
    }

    /// On the primary, behind biometrics: keeps a proposal as it was previewed (`digest`): what it adds becomes the
    /// primary's own, unopened by anyone else.
    pub async fn proposal_keep(&self, id: String, digest: String) -> Result<(), CoreError> {
        let runtime = Arc::clone(&self.runtime);
        let generation = runtime.generation();
        rt::run(async move {
            let engine = runtime.prepare(generation).await?;
            engine.require_decider()?;
            engine.ensure_active()?;
            let result = engine.run_account(engine.proposal_keep(&id, &digest)).await;
            runtime.finish(&engine, result)
        })
        .await
    }

    /// On the primary: discards a proposal.
    pub async fn proposal_discard(&self, id: String) -> Result<(), CoreError> {
        let runtime = Arc::clone(&self.runtime);
        let generation = runtime.generation();
        rt::run(async move {
            let engine = runtime.prepare(generation).await?;
            engine.require_decider()?;
            engine.ensure_active()?;
            let result = engine.run_account(engine.proposal_discard(&id)).await;
            runtime.finish(&engine, result)
        })
        .await
    }

    /// The account's phones as this phone verified them (`refresh`: fetched and verified again first). `None` on a
    /// legacy account.
    pub async fn roster_summary(&self, refresh: bool) -> Result<Option<RosterSummary>, CoreError> {
        let runtime = Arc::clone(&self.runtime);
        let generation = runtime.generation();
        rt::run(async move {
            let engine = runtime.prepare(generation).await?;
            engine.run_account(engine.roster_summary(refresh)).await
        })
        .await
    }

    /// On the approval device: turns phone roles on for the account, with this phone as primary (`device_name`, e.g.
    /// "iPhone"). `code_or_password`: only for an account made with a master password (this phone keeps no recovery
    /// code to seal the recovery wrap with). Ask whether the recovery code is saved first.
    pub async fn roster_create(
        &self,
        device_name: String,
        code_or_password: Option<String>,
    ) -> Result<RosterSummary, CoreError> {
        let runtime = Arc::clone(&self.runtime);
        let typed = code_or_password.map(Zeroizing::new);
        let generation = runtime.generation();
        rt::run(async move {
            let engine = runtime.prepare(generation).await?;
            let result = engine.run_account(engine.roster_create(&device_name, typed)).await;
            runtime.finish(&engine, result)
        })
        .await
    }

    /// On a phone that should view the account: asks its primary to enroll it. Then call `enroll_poll` every few
    /// seconds; it shows the code to compare once the primary answered.
    pub async fn enroll_begin(&self, device_name: String) -> Result<EnrollStart, CoreError> {
        let runtime = Arc::clone(&self.runtime);
        let generation = runtime.generation();
        rt::run(async move {
            let engine = runtime.prepare(generation).await?;
            engine.run_account(engine.enroll_begin(&device_name)).await
        })
        .await
    }

    /// Where this phone's enrollment stands: `Compare` shows the six digits; `Enrolled` once it views the account
    /// (with the vault open when the account has a recovery code). `NotFound` when none is open.
    pub async fn enroll_poll(&self) -> Result<EnrollProgress, CoreError> {
        let runtime = Arc::clone(&self.runtime);
        let generation = runtime.generation();
        rt::run(async move {
            let engine = runtime.prepare(generation).await?;
            let progress = engine.run_account(engine.enroll_poll()).await?;
            runtime.check_engine(&engine)?;
            if progress == EnrollProgress::Enrolled {
                // The vault opens when the answer carried the account secret (an account with a master password signs
                // in with it instead).
                match runtime.activate_unlocked().await {
                    Ok(()) | Err(CoreError::NotLoggedIn) => {}
                    Err(e) => return Err(e),
                }
            }
            Ok(progress)
        })
        .await
    }

    pub async fn enroll_cancel(&self) -> Result<(), CoreError> {
        let runtime = Arc::clone(&self.runtime);
        let generation = runtime.generation();
        rt::run(async move {
            let engine = runtime.prepare(generation).await?;
            engine.run_account(engine.enroll_cancel()).await
        })
        .await
    }

    /// On the primary: a phone asking to view the account (a parked `Join` item). The first call answers it with this
    /// phone's challenge; call again until `code` is there, then show it next to the other phone's.
    pub async fn enroll_view(&self, id: String) -> Result<EnrollView, CoreError> {
        let runtime = Arc::clone(&self.runtime);
        let generation = runtime.generation();
        rt::run(async move {
            let engine = runtime.prepare(generation).await?;
            let result = engine.run_account(engine.enroll_view(&id)).await;
            runtime.finish(&engine, result)
        })
        .await
    }

    /// On the primary, after the user compared the codes (and biometrics): enrolls the phone as a viewer, or declines.
    pub async fn enroll_confirm(&self, id: String, approve: bool) -> Result<(), CoreError> {
        let runtime = Arc::clone(&self.runtime);
        let generation = runtime.generation();
        rt::run(async move {
            let engine = runtime.prepare(generation).await?;
            let result = engine.run_account(engine.enroll_confirm(&id, approve)).await;
            runtime.finish(&engine, result)
        })
        .await
    }

    /// On a viewer: "Decide on this phone". Returns the request's id; follow with `handover_poll`.
    pub async fn handover_request(&self) -> Result<String, CoreError> {
        let runtime = Arc::clone(&self.runtime);
        let generation = runtime.generation();
        rt::run(async move {
            let engine = runtime.prepare(generation).await?;
            engine.run_account(engine.handover_request()).await
        })
        .await
    }

    /// Where this phone's handover request stands; `Done` once it is the primary (it verified and kept the keys the
    /// old primary sealed to it). `NotFound` when none is open.
    pub async fn handover_poll(&self) -> Result<HandoverProgress, CoreError> {
        let runtime = Arc::clone(&self.runtime);
        let generation = runtime.generation();
        rt::run(async move {
            let engine = runtime.prepare(generation).await?;
            let result = engine.run_account(engine.handover_poll()).await;
            runtime.finish(&engine, result)
        })
        .await
    }

    /// On the primary: the viewers asking to decide, each verified (signed by that viewer's roster key) and named as
    /// the roster names it.
    pub async fn handover_requests(&self) -> Result<Vec<HandoverView>, CoreError> {
        let runtime = Arc::clone(&self.runtime);
        let generation = runtime.generation();
        rt::run(async move {
            let engine = runtime.prepare(generation).await?;
            engine.run_account(engine.handover_requests()).await
        })
        .await
    }

    /// On the primary, after biometrics: makes the viewer that asked (`id`) the primary. `device_id` is the phone the
    /// user saw in `handover_requests` (the request must be from it). This phone becomes a viewer.
    pub async fn handover_approve(&self, id: String, device_id: String) -> Result<(), CoreError> {
        let runtime = Arc::clone(&self.runtime);
        let generation = runtime.generation();
        rt::run(async move {
            let engine = runtime.prepare(generation).await?;
            let result = engine.run_account(engine.handover_approve(&id, &device_id)).await;
            runtime.finish(&engine, result)
        })
        .await
    }

    pub async fn handover_deny(&self, id: String) -> Result<(), CoreError> {
        let runtime = Arc::clone(&self.runtime);
        let generation = runtime.generation();
        rt::run(async move {
            let engine = runtime.prepare(generation).await?;
            engine.run_account(engine.handover_deny(&id)).await
        })
        .await
    }

    /// "<primary> lost? Use the recovery code", step 1: what the recovery would make of the account's phones (this
    /// phone primary, the viewers kept, the primary removed). Show it; the user confirms before `roster_recover`.
    pub async fn roster_recover_preview(&self, code_or_password: String) -> Result<RecoveryPreview, CoreError> {
        let runtime = Arc::clone(&self.runtime);
        let typed = Zeroizing::new(code_or_password);
        let generation = runtime.generation();
        rt::run(async move {
            let engine = runtime.prepare(generation).await?;
            engine.run_account(engine.roster_recover_preview(typed)).await
        })
        .await
    }

    /// Step 2, once the user confirmed the preview: with the recovery code (or the master password) this phone
    /// becomes the primary under a new owner key; the old primary is removed. `tip` is the preview's: if the account's
    /// phones changed since, nothing is signed. `confirmation`: the token of the `RecoveryNeedsConfirmation` the
    /// user accepted (none at first).
    pub async fn roster_recover(
        &self,
        code_or_password: String,
        device_name: String,
        tip: String,
        confirmation: Option<String>,
    ) -> Result<RosterSummary, CoreError> {
        let runtime = Arc::clone(&self.runtime);
        let typed = Zeroizing::new(code_or_password);
        let generation = runtime.generation();
        rt::run(async move {
            let engine = runtime.prepare(generation).await?;
            let summary =
                engine.run_account(engine.roster_recover(typed, &device_name, &tip, confirmation.as_deref())).await?;
            runtime.check_engine(&engine)?;
            match runtime.activate_unlocked().await {
                Ok(()) | Err(CoreError::NotLoggedIn) => {}
                Err(e) => return Err(e),
            }
            Ok(summary)
        })
        .await
    }

    /// On the primary: takes a viewer off the account (it reads nothing written from now on).
    pub async fn roster_remove_member(&self, device_id: String) -> Result<(), CoreError> {
        let runtime = Arc::clone(&self.runtime);
        let generation = runtime.generation();
        rt::run(async move {
            let engine = runtime.prepare(generation).await?;
            let result = engine.run_account(engine.roster_remove_member(&device_id)).await;
            runtime.finish(&engine, result)
        })
        .await
    }

    /// On the primary, after the account's master password changed: seals the recovery wrap again for it.
    pub async fn roster_rewrap(&self, new_password: String) -> Result<(), CoreError> {
        let runtime = Arc::clone(&self.runtime);
        let typed = Zeroizing::new(new_password);
        let generation = runtime.generation();
        rt::run(async move {
            let engine = runtime.prepare(generation).await?;
            let result = engine.run_account(engine.roster_rewrap(typed)).await;
            runtime.finish(&engine, result)
        })
        .await
    }

    /// Creates a Bitwarden-compatible account on the server (PBKDF2-SHA256 with 600 000 iterations, a fresh vault key
    /// and RSA key pair), then signs in to it like `login`. A master password shorter than 12 characters, a taken
    /// email or closed sign-ups are `Invalid` with a message for the user.
    pub async fn create_account(
        &self,
        server_url: String,
        email: String,
        password: String,
    ) -> Result<SessionInfo, CoreError> {
        let runtime = Arc::clone(&self.runtime);
        let password = Zeroizing::new(password);
        rt::run(async move { runtime.create_account(&server_url, &email, password).await }).await
    }

    pub async fn logout(&self) -> Result<(), CoreError> {
        let runtime = Arc::clone(&self.runtime);
        rt::run(async move { runtime.logout().await }).await
    }

    /// Sign out locally and revoke this device on the server. Open the returned URL in the sign-in browser to
    /// end its AuthKit session too. Network failure never prevents local sign-out; `None` means no URL is available.
    pub async fn logout_with_browser(&self) -> Result<Option<String>, CoreError> {
        let runtime = Arc::clone(&self.runtime);
        rt::run(async move { runtime.logout_with_browser(true).await }).await
    }

    /// Deletes the signed-in account for good: on the server its vault, devices, approval device, AI and desktop
    /// connections, files and stored state (and its WorkOS user, on a server that signs in through WorkOS); then signs
    /// out and removes this phone's encrypted copy of the account. `confirm_email` is the account's email as the user
    /// typed it. Errors leave everything as it was: `Invalid` carries a message for the user (a wrong email, another
    /// phone approves for the account, the last owner of an organization, try again later).
    pub async fn delete_account(&self, confirm_email: String) -> Result<(), CoreError> {
        let runtime = Arc::clone(&self.runtime);
        rt::run(async move { runtime.delete_account(&confirm_email).await }).await
    }

    pub async fn register_device(&self, fcm_token: Option<String>) -> Result<(), CoreError> {
        let runtime = Arc::clone(&self.runtime);
        let generation = runtime.generation();
        rt::run(async move {
            let engine = runtime.prepare(generation).await?;
            engine.ensure_active()?;
            let result = engine.run_account(async { engine.register_device(fcm_token).await }).await;
            runtime.finish(&engine, result)
        })
        .await
    }

    /// Background entry point from FCM (`kind` is `req`, `pair`, `blob` or `replaced`).
    pub async fn handle_push(&self, kind: String, id: String) -> Result<(), CoreError> {
        let runtime = Arc::clone(&self.runtime);
        let generation = runtime.generation();
        rt::run(async move {
            let engine = runtime.prepare(generation).await?;
            engine.ensure_active()?;
            let result = engine.run_account(async { engine.handle_push(&kind, &id).await }).await;
            runtime.finish_synced(&engine, result).await
        })
        .await
    }

    /// Like `handle_push`, for a process that cannot run the model (the iOS notification extension): Bypass and
    /// Lockdown decide at once; what Manual or Auto would judge waits unjudged, and the app's next pass judges it.
    pub async fn handle_push_deferring_autopilot(&self, kind: String, id: String) -> Result<(), CoreError> {
        let runtime = Arc::clone(&self.runtime);
        let generation = runtime.generation();
        rt::run(async move {
            let engine = runtime.prepare(generation).await?;
            engine.ensure_active()?;
            let result = engine.run_account(async { engine.handle_push_deferring_autopilot(&kind, &id).await }).await;
            runtime.finish_synced(&engine, result).await
        })
        .await
    }

    /// Foreground long-poll, then everything received is processed like a push.
    pub async fn sync(&self, wait_secs: u32) -> Result<Vec<PendingItem>, CoreError> {
        let runtime = Arc::clone(&self.runtime);
        let generation = runtime.generation();
        rt::run(async move {
            let engine = runtime.prepare(generation).await?;
            engine.ensure_active()?;
            let result = engine.run_account(async { engine.sync(wait_secs).await }).await;
            runtime.finish(&engine, result)
        })
        .await
    }

    /// What waits for the owner, newest first: the only items an app presents, notifies, counts or badges. What a
    /// bypass, Lockdown, Auto or Overnight has still to look at is not here (see `deciding`).
    pub async fn pending(&self) -> Result<Vec<PendingItem>, CoreError> {
        let runtime = Arc::clone(&self.runtime);
        let generation = runtime.generation();
        rt::run(async move {
            let engine = runtime.prepare(generation).await?;
            engine.ensure_active()?;
            let result = engine.run_account(async { engine.pending() }).await;
            runtime.finish(&engine, result)
        })
        .await
    }

    /// Parked requests and uploads an automatic decider (a bypass, Lockdown, Auto, Overnight) has still to look at,
    /// newest first: never an ask for the owner. The notification extension, which cannot run the model, words the
    /// notification of such a push from it ("open Reins to let Autopilot decide") instead of an ask.
    pub async fn deciding(&self) -> Result<Vec<PendingItem>, CoreError> {
        let runtime = Arc::clone(&self.runtime);
        let generation = runtime.generation();
        rt::run(async move {
            let engine = runtime.prepare(generation).await?;
            engine.ensure_active()?;
            let result = engine.run_account(async { engine.deciding() }).await;
            runtime.finish(&engine, result)
        })
        .await
    }

    pub async fn approval_view(&self, request_id: String) -> Result<ApprovalView, CoreError> {
        let runtime = Arc::clone(&self.runtime);
        let generation = runtime.generation();
        rt::run(async move {
            let engine = runtime.prepare(generation).await?;
            engine.ensure_active()?;
            let result = engine.run_account(async { engine.approval_view(&request_id) }).await;
            runtime.finish(&engine, result)
        })
        .await
    }

    /// An Activity entry's request as its approval showed it (read-only; `None` for older entries). On a device that
    /// did not show it, a push still comes back, from the copy synced with the entry (`git` set, little else).
    pub async fn activity_approval(&self, entry_id: i64) -> Result<Option<ApprovalView>, CoreError> {
        let runtime = Arc::clone(&self.runtime);
        let generation = runtime.generation();
        rt::run(async move {
            let engine = runtime.prepare(generation).await?;
            engine.ensure_active()?;
            let result = engine.run_account(async { engine.activity_approval(entry_id) }).await;
            runtime.finish(&engine, result)
        })
        .await
    }

    /// An Activity entry's push, the best copy this device has: commits, files with lines added and removed, and the
    /// changes line by line where kept (`ActivityPush.diffs`, `diffs_not_kept`). `None` when the entry is no push.
    pub async fn activity_push(&self, entry_id: i64) -> Result<Option<crate::types::ActivityPush>, CoreError> {
        let runtime = Arc::clone(&self.runtime);
        let generation = runtime.generation();
        rt::run(async move {
            let engine = runtime.prepare(generation).await?;
            engine.ensure_active()?;
            let result = engine.run_account(async { engine.activity_push(entry_id) }).await;
            runtime.finish(&engine, result)
        })
        .await
    }

    /// One commit of an Activity entry's push (`short_sha` from its `GitCommitView`), with its files and their changes
    /// line by line, loaded from GitHub with this device's own connection: for a push whose changes were not kept
    /// here. Only commits of that push; nothing is asked, logged or shown to an AI.
    pub async fn activity_commit_change(
        &self,
        entry_id: i64,
        short_sha: String,
    ) -> Result<crate::types::GitCommitView, CoreError> {
        let runtime = Arc::clone(&self.runtime);
        let generation = runtime.generation();
        rt::run(async move {
            let engine = runtime.prepare(generation).await?;
            engine.ensure_active()?;
            let result = engine.run_account(async { engine.activity_commit_change(entry_id, &short_sha).await }).await;
            runtime.finish(&engine, result)
        })
        .await
    }

    pub async fn approve(&self, request_id: String, choice: ApprovalChoice) -> Result<(), CoreError> {
        let runtime = Arc::clone(&self.runtime);
        let generation = runtime.generation();
        rt::run(async move {
            let engine = runtime.prepare(generation).await?;
            engine.require_decider()?;
            engine.ensure_active()?;
            let result = engine.run_account(async { engine.approve(&request_id, choice).await }).await;
            runtime.finish(&engine, result)
        })
        .await
    }

    /// Later: an Instant request becomes a ticket (the agent is told at once and carries on); it waits under
    /// "Waiting for you" until its deadline.
    pub async fn decide_later(&self, request_id: String) -> Result<(), CoreError> {
        let runtime = Arc::clone(&self.runtime);
        let generation = runtime.generation();
        rt::run(async move {
            let engine = runtime.prepare(generation).await?;
            engine.ensure_active()?;
            let result = engine.run_account(async { engine.decide_later(&request_id).await }).await;
            runtime.finish(&engine, result)
        })
        .await
    }

    /// The twelve digits naming this phone's signing key ("4821 9930 1274"): what agents and the desktop app pin to
    /// check the events and webhooks this phone signs. Shown under a connection's More.
    pub async fn signing_key_fingerprint(&self) -> Result<String, CoreError> {
        let runtime = Arc::clone(&self.runtime);
        let generation = runtime.generation();
        rt::run(async move {
            let engine = runtime.prepare(generation).await?;
            let result = engine.run_account(async { engine.signing_key_fingerprint() }).await;
            runtime.finish(&engine, result)
        })
        .await
    }

    /// The user refused, with an optional note the agent receives ("Use the staging bucket").
    pub async fn deny_with_note(&self, request_id: String, note: Option<String>) -> Result<(), CoreError> {
        let runtime = Arc::clone(&self.runtime);
        let generation = runtime.generation();
        rt::run(async move {
            let engine = runtime.prepare(generation).await?;
            engine.require_decider()?;
            engine.ensure_active()?;
            let result = engine.run_account(async { engine.deny_with_note(&request_id, note).await }).await;
            runtime.finish(&engine, result)
        })
        .await
    }

    /// "Why?": asks the agent; the request waits as a ticket until it answers (shown as the agent's words).
    pub async fn ask_why(&self, request_id: String, question: String) -> Result<(), CoreError> {
        let runtime = Arc::clone(&self.runtime);
        let generation = runtime.generation();
        rt::run(async move {
            let engine = runtime.prepare(generation).await?;
            engine.ensure_active()?;
            let result = engine.run_account(async { engine.ask_why(&request_id, &question).await }).await;
            runtime.finish(&engine, result)
        })
        .await
    }

    /// Approves a request as the approval sheet would untouched, without opening it (a notification's Approve, "Approve
    /// all"). Refused for what is asked every time (`ApprovalView.quick` is `None` for those).
    pub async fn approve_quick(&self, request_id: String) -> Result<(), CoreError> {
        let runtime = Arc::clone(&self.runtime);
        let generation = runtime.generation();
        rt::run(async move {
            let engine = runtime.prepare(generation).await?;
            engine.require_decider()?;
            engine.ensure_active()?;
            let result = engine.run_account(async { engine.approve_quick(&request_id).await }).await;
            runtime.finish(&engine, result)
        })
        .await
    }

    /// What a newly connected AI may do before it asked for anything; `None` until the user chose (acts as asking).
    pub async fn starting_policy(&self) -> Result<Option<StartingPolicy>, CoreError> {
        let runtime = Arc::clone(&self.runtime);
        let generation = runtime.generation();
        rt::run(async move {
            let engine = runtime.prepare(generation).await?;
            engine.ensure_active()?;
            let result = engine.starting_policy();
            runtime.finish(&engine, result)
        })
        .await
    }

    /// Chosen during setup and changed under Grants; applies to connections made from then on.
    pub async fn set_starting_policy(&self, policy: StartingPolicy) -> Result<(), CoreError> {
        let runtime = Arc::clone(&self.runtime);
        let generation = runtime.generation();
        rt::run(async move {
            let engine = runtime.prepare(generation).await?;
            engine.require_decider()?;
            engine.ensure_active()?;
            let result = engine.set_starting_policy(policy);
            runtime.finish(&engine, result)
        })
        .await
    }

    pub async fn deny(&self, request_id: String) -> Result<(), CoreError> {
        let runtime = Arc::clone(&self.runtime);
        let generation = runtime.generation();
        rt::run(async move {
            let engine = runtime.prepare(generation).await?;
            engine.require_decider()?;
            engine.ensure_active()?;
            let result = engine.run_account(async { engine.deny(&request_id).await }).await;
            runtime.finish(&engine, result)
        })
        .await
    }

    pub async fn pairing_view(&self, pairing_id: String) -> Result<PairingView, CoreError> {
        let runtime = Arc::clone(&self.runtime);
        let generation = runtime.generation();
        rt::run(async move {
            let engine = runtime.prepare(generation).await?;
            engine.ensure_active()?;
            let result = engine.run_account(async { engine.pairing_view(&pairing_id).await }).await;
            runtime.finish(&engine, result)
        })
        .await
    }

    /// The pairing a computer's code stands for (`BCDF-GHJK`, scanned from its QR code or opened from a link; case,
    /// spaces and dashes do not count). It is parked like a pushed pairing and answered with `answer_pairing`. An
    /// unknown or expired code is `NotFound`.
    pub async fn pairing_by_code(&self, user_code: String) -> Result<PairingView, CoreError> {
        let runtime = Arc::clone(&self.runtime);
        let generation = runtime.generation();
        rt::run(async move {
            let engine = runtime.prepare(generation).await?;
            engine.ensure_active()?;
            let result = engine.run_account(async { engine.pairing_by_code(&user_code).await }).await;
            runtime.finish(&engine, result)
        })
        .await
    }

    /// Answers a pairing. `label` is what the name field holds; it becomes the user's name for the connection only
    /// when `name_edited` (the user changed the field) and it is not the name the field started with.
    pub async fn answer_pairing(
        &self,
        pairing_id: String,
        approve: bool,
        chosen_code: Option<u8>,
        label: Option<String>,
        name_edited: bool,
    ) -> Result<(), CoreError> {
        let runtime = Arc::clone(&self.runtime);
        let generation = runtime.generation();
        rt::run(async move {
            let engine = runtime.prepare(generation).await?;
            engine.require_decider()?;
            engine.ensure_active()?;
            let result = engine
                .run_account(async {
                    engine.answer_pairing(&pairing_id, approve, chosen_code, label, name_edited).await
                })
                .await;
            runtime.finish(&engine, result)
        })
        .await
    }

    pub async fn grants(&self) -> Result<Vec<GrantView>, CoreError> {
        let runtime = Arc::clone(&self.runtime);
        let generation = runtime.generation();
        rt::run(async move {
            let engine = runtime.prepare(generation).await?;
            engine.ensure_active()?;
            let result = engine.run_account(async { engine.grants() }).await;
            runtime.finish(&engine, result)
        })
        .await
    }

    pub async fn revoke_grant(&self, grant_id: String) -> Result<(), CoreError> {
        let runtime = Arc::clone(&self.runtime);
        let generation = runtime.generation();
        rt::run(async move {
            let engine = runtime.prepare(generation).await?;
            engine.ensure_active()?;
            let result = engine
                .run_account(async {
                    engine.revoke_grant(&grant_id)?;
                    engine
                        .brake_out(Restriction::RevokeGrant {
                            grant_id: grant_id.clone(),
                        })
                        .await
                })
                .await;
            runtime.finish(&engine, result)
        })
        .await
    }

    pub async fn connections(&self) -> Result<Vec<ConnectionView>, CoreError> {
        let runtime = Arc::clone(&self.runtime);
        let generation = runtime.generation();
        rt::run(async move {
            let engine = runtime.prepare(generation).await?;
            engine.ensure_active()?;
            let result = engine.run_account(async { engine.connections().await }).await;
            runtime.finish(&engine, result)
        })
        .await
    }

    pub async fn revoke_connection(&self, connection_id: String) -> Result<(), CoreError> {
        let runtime = Arc::clone(&self.runtime);
        let generation = runtime.generation();
        rt::run(async move {
            let engine = runtime.prepare(generation).await?;
            engine.require_decider()?;
            engine.ensure_active()?;
            let result = engine.run_account(async { engine.revoke_connection(&connection_id).await }).await;
            runtime.finish(&engine, result)
        })
        .await
    }

    /// The devices signed in to the account (Settings > Devices).
    pub async fn devices(&self) -> Result<Vec<DeviceView>, CoreError> {
        let runtime = Arc::clone(&self.runtime);
        let generation = runtime.generation();
        rt::run(async move {
            let engine = runtime.prepare(generation).await?;
            engine.ensure_active()?;
            let result = engine.run_account(async { engine.devices().await }).await;
            runtime.finish(&engine, result)
        })
        .await
    }

    /// Signs another device of the account out (a lost phone), with the recovery code or master password the user
    /// typed now: it can no longer sync or answer for the account.
    pub async fn sign_out_device(&self, device_id: String, code_or_password: String) -> Result<(), CoreError> {
        let runtime = Arc::clone(&self.runtime);
        let generation = runtime.generation();
        let typed = Zeroizing::new(code_or_password);
        rt::run(async move {
            let engine = runtime.prepare(generation).await?;
            engine.require_decider()?;
            engine.ensure_active()?;
            let result = engine.run_account(async { engine.sign_out_device(&device_id, typed).await }).await;
            runtime.finish(&engine, result)
        })
        .await
    }

    /// The vault's items in use whose name, username or website contains `query` (empty: all), by name.
    pub async fn vault_items(&self, query: String) -> Result<Vec<VaultItemSummary>, CoreError> {
        let runtime = Arc::clone(&self.runtime);
        let generation = runtime.generation();
        rt::run(async move {
            let engine = runtime.prepare(generation).await?;
            let result = engine.run_account(async { engine.vault_items(&query).await }).await;
            runtime.finish(&engine, result)
        })
        .await
    }

    /// One item: the values of its plain fields, which fields are secret, and how the desktop app names it.
    pub async fn vault_item(&self, id: String) -> Result<VaultItemDetail, CoreError> {
        let runtime = Arc::clone(&self.runtime);
        let generation = runtime.generation();
        rt::run(async move {
            let engine = runtime.prepare(generation).await?;
            let result = engine.run_account(async { engine.vault_item(&id).await }).await;
            runtime.finish(&engine, result)
        })
        .await
    }

    /// The value of one field (`VaultField::key`), after the app checked the screen lock.
    pub async fn vault_reveal(&self, id: String, key: String) -> Result<String, CoreError> {
        let runtime = Arc::clone(&self.runtime);
        let generation = runtime.generation();
        rt::run(async move {
            let engine = runtime.prepare(generation).await?;
            let result = engine.run_account(async { engine.vault_reveal(&id, &key).await }).await;
            runtime.finish(&engine, result)
        })
        .await
    }

    /// Creates an item; its id.
    pub async fn vault_create(&self, input: VaultItemInput) -> Result<String, CoreError> {
        let runtime = Arc::clone(&self.runtime);
        let generation = runtime.generation();
        rt::run(async move {
            let engine = runtime.prepare(generation).await?;
            engine.require_decider()?;
            let result = engine.run_account(async { engine.vault_create(&input).await }).await;
            runtime.finish(&engine, result)
        })
        .await
    }

    /// Renames an item and changes the fields given.
    pub async fn vault_update(&self, id: String, input: VaultItemInput) -> Result<(), CoreError> {
        let runtime = Arc::clone(&self.runtime);
        let generation = runtime.generation();
        rt::run(async move {
            let engine = runtime.prepare(generation).await?;
            engine.require_decider()?;
            let result = engine.run_account(async { engine.vault_update(&id, &input).await }).await;
            runtime.finish(&engine, result)
        })
        .await
    }

    /// Deletes an item for good.
    pub async fn vault_delete(&self, id: String) -> Result<(), CoreError> {
        let runtime = Arc::clone(&self.runtime);
        let generation = runtime.generation();
        rt::run(async move {
            let engine = runtime.prepare(generation).await?;
            engine.require_decider()?;
            let result = engine.run_account(async { engine.vault_delete(&id).await }).await;
            runtime.finish(&engine, result)
        })
        .await
    }

    /// Makes an Ed25519 key on the phone and keeps it as an SSH key item named `name`.
    pub async fn vault_generate_ssh_key(&self, name: String) -> Result<VaultSshKey, CoreError> {
        let runtime = Arc::clone(&self.runtime);
        let generation = runtime.generation();
        rt::run(async move {
            let engine = runtime.prepare(generation).await?;
            engine.require_decider()?;
            let result = engine.run_account(async { engine.vault_generate_ssh_key(&name).await }).await;
            runtime.finish(&engine, result)
        })
        .await
    }

    /// The eight digits of this phone's key, which `reins vault add` asks to compare once ("4821 9930").
    pub async fn phone_key_fingerprint(&self) -> Result<String, CoreError> {
        let runtime = Arc::clone(&self.runtime);
        let generation = runtime.generation();
        rt::run(async move {
            let engine = runtime.prepare(generation).await?;
            let result = engine.run_account(async { engine.phone_key_fingerprint() }).await;
            runtime.finish(&engine, result)
        })
        .await
    }

    /// Reins Secrets: whether `project/env` is asked every time (`Some`), or left to the computer's reins.toml and the
    /// environment's name (`None`: production environments and `confirm = "always"` are asked every time).
    pub async fn set_secrets_always_ask(
        &self,
        project: String,
        env: String,
        always: Option<bool>,
    ) -> Result<(), CoreError> {
        let runtime = Arc::clone(&self.runtime);
        let generation = runtime.generation();
        rt::run(async move {
            let engine = runtime.prepare(generation).await?;
            engine.require_decider()?;
            let result = engine.run_account(async { engine.set_secrets_always_ask(&project, &env, always) }).await;
            runtime.finish(&engine, result)
        })
        .await
    }

    /// Reins Secrets: "Revoke on all computers". The permissions for secrets of `project` (and `env`; neither: all of
    /// them) end, and every paired computer is told to drop the values it holds. Returns how many computers were told.
    pub async fn revoke_secrets_everywhere(
        &self,
        project: Option<String>,
        env: Option<String>,
    ) -> Result<SecretsRevoked, CoreError> {
        let runtime = Arc::clone(&self.runtime);
        let generation = runtime.generation();
        rt::run(async move {
            let engine = runtime.prepare(generation).await?;
            engine.require_decider()?;
            engine.ensure_active()?;
            let result = engine
                .run_account(async { engine.revoke_secrets_everywhere(project.as_deref(), env.as_deref()).await })
                .await;
            runtime.finish(&engine, result)
        })
        .await
    }

    /// Remembers the icon picked for a connection (`None` = derive it from the name).
    pub async fn set_connection_icon(&self, connection_id: String, icon: Option<String>) -> Result<(), CoreError> {
        let runtime = Arc::clone(&self.runtime);
        let generation = runtime.generation();
        rt::run(async move {
            let engine = runtime.prepare(generation).await?;
            engine.require_decider()?;
            engine.ensure_active()?;
            let result = engine.run_account(async { engine.set_connection_icon(&connection_id, icon) }).await;
            runtime.finish(&engine, result)
        })
        .await
    }

    /// Changes a connection's blobatar to `shape` (below `blobatar_shape_count()`) and `colour` (an index of
    /// `blobatar_colours()`), keeping its figure. Returns the one kept.
    pub async fn set_connection_blobatar(
        &self,
        connection_id: String,
        shape: u8,
        colour: u8,
    ) -> Result<crate::avatar::Blobatar, CoreError> {
        let runtime = Arc::clone(&self.runtime);
        let generation = runtime.generation();
        rt::run(async move {
            let engine = runtime.prepare(generation).await?;
            engine.require_decider()?;
            engine.ensure_active()?;
            let result =
                engine.run_account(async { engine.set_connection_blobatar(&connection_id, shape, colour) }).await;
            runtime.finish(&engine, result)
        })
        .await
    }

    /// "Shuffle": a new random blobatar for a connection (another figure, shape and colour). Returns it.
    pub async fn shuffle_connection_blobatar(
        &self,
        connection_id: String,
    ) -> Result<crate::avatar::Blobatar, CoreError> {
        let runtime = Arc::clone(&self.runtime);
        let generation = runtime.generation();
        rt::run(async move {
            let engine = runtime.prepare(generation).await?;
            engine.require_decider()?;
            engine.ensure_active()?;
            let result = engine.run_account(async { engine.shuffle_connection_blobatar(&connection_id) }).await;
            runtime.finish(&engine, result)
        })
        .await
    }

    /// Gives a connection the user's own name for it ("Claude (work)"), kept with the account and shown everywhere
    /// instead of its own; `None` or an empty name takes it away. One line of at most 64 characters, without control
    /// or invisible characters. Returns the name kept (`None`: it goes by its own name).
    pub async fn rename_connection(
        &self,
        connection_id: String,
        name: Option<String>,
    ) -> Result<Option<String>, CoreError> {
        let runtime = Arc::clone(&self.runtime);
        let generation = runtime.generation();
        rt::run(async move {
            let engine = runtime.prepare(generation).await?;
            engine.require_decider()?;
            engine.ensure_active()?;
            let result = engine.run_account(async { engine.rename_connection(&connection_id, name).await }).await;
            runtime.finish(&engine, result)
        })
        .await
    }

    /// Creates a permission ahead of any request (`kind` is `Read` or `Send`).
    pub async fn create_grant(
        &self,
        connection_id: String,
        account: String,
        kind: ApprovalKind,
        standing: StandingGrant,
    ) -> Result<(), CoreError> {
        let runtime = Arc::clone(&self.runtime);
        let generation = runtime.generation();
        rt::run(async move {
            let engine = runtime.prepare(generation).await?;
            engine.require_decider()?;
            engine.ensure_active()?;
            let result =
                engine.run_account(async { engine.create_grant(&connection_id, &account, kind, standing).await }).await;
            runtime.finish(&engine, result)
        })
        .await
    }

    /// Starts a grant that ended (expired, used up or deleted) over for `duration_secs`.
    pub async fn resume_grant(&self, grant_id: String, duration_secs: u64) -> Result<(), CoreError> {
        let runtime = Arc::clone(&self.runtime);
        let generation = runtime.generation();
        rt::run(async move {
            let engine = runtime.prepare(generation).await?;
            engine.require_decider()?;
            engine.ensure_active()?;
            let result = engine.run_account(async { engine.resume_grant(&grant_id, duration_secs) }).await;
            runtime.finish(&engine, result)
        })
        .await
    }

    /// Lets an active grant run for `duration_secs` from now instead of until its end; its uses so far stay.
    pub async fn extend_grant(&self, grant_id: String, duration_secs: u64) -> Result<(), CoreError> {
        let runtime = Arc::clone(&self.runtime);
        let generation = runtime.generation();
        rt::run(async move {
            let engine = runtime.prepare(generation).await?;
            engine.require_decider()?;
            engine.ensure_active()?;
            let result = engine.run_account(async { engine.extend_grant(&grant_id, duration_secs) }).await;
            runtime.finish(&engine, result)
        })
        .await
    }

    /// Starts an ended grant over with changes: the period, the number of uses and who or what it covers.
    pub async fn resume_grant_edited(&self, grant_id: String, standing: StandingGrant) -> Result<(), CoreError> {
        let runtime = Arc::clone(&self.runtime);
        let generation = runtime.generation();
        rt::run(async move {
            let engine = runtime.prepare(generation).await?;
            engine.require_decider()?;
            engine.ensure_active()?;
            let result = engine.run_account(async { engine.resume_grant_edited(&grant_id, &standing) }).await;
            runtime.finish(&engine, result)
        })
        .await
    }

    /// Opens one email listed in the activity: fetched from Gmail now, `account` being the entry's account.
    pub async fn fetch_email(&self, account: Option<String>, message_id: String) -> Result<EmailContent, CoreError> {
        let runtime = Arc::clone(&self.runtime);
        let generation = runtime.generation();
        rt::run(async move {
            let engine = runtime.prepare(generation).await?;
            engine.ensure_active()?;
            let result = engine.run_account(async { engine.fetch_email(account.as_deref(), &message_id).await }).await;
            runtime.finish(&engine, result)
        })
        .await
    }

    /// Removes an ended (expired, used-up or revoked) grant for good.
    pub async fn delete_grant(&self, grant_id: String) -> Result<(), CoreError> {
        let runtime = Arc::clone(&self.runtime);
        let generation = runtime.generation();
        rt::run(async move {
            let engine = runtime.prepare(generation).await?;
            engine.ensure_active()?;
            let result = engine
                .run_account(async {
                    engine.delete_grant(&grant_id)?;
                    engine
                        .brake_out(Restriction::RevokeGrant {
                            grant_id: grant_id.clone(),
                        })
                        .await
                })
                .await;
            runtime.finish(&engine, result)
        })
        .await
    }

    /// The accounts the user connected, oldest first.
    pub async fn accounts(&self) -> Result<Vec<AccountView>, CoreError> {
        let runtime = Arc::clone(&self.runtime);
        let generation = runtime.generation();
        rt::run(async move {
            let engine = runtime.prepare(generation).await?;
            engine.ensure_active()?;
            let result = engine.run_account(async { engine.accounts() }).await;
            runtime.finish(&engine, result)
        })
        .await
    }

    /// Connects a Google account that was just authorized on this phone (`hint` is its address).
    pub async fn add_account(&self, hint: String) -> Result<AccountView, CoreError> {
        let runtime = Arc::clone(&self.runtime);
        let generation = runtime.generation();
        rt::run(async move {
            let engine = runtime.prepare(generation).await?;
            engine.require_decider()?;
            engine.ensure_active()?;
            let result = engine.run_account(async { engine.add_account(&hint).await }).await;
            runtime.finish(&engine, result)
        })
        .await
    }

    /// Disconnects an account: its grants are deleted and requests waiting for it are answered with an error.
    pub async fn remove_account(&self, account: String) -> Result<(), CoreError> {
        let runtime = Arc::clone(&self.runtime);
        let generation = runtime.generation();
        rt::run(async move {
            let engine = runtime.prepare(generation).await?;
            engine.require_decider()?;
            engine.ensure_active()?;
            let result = engine.run_account(async { engine.remove_account(&account).await }).await;
            runtime.finish(&engine, result)
        })
        .await
    }

    /// Whether one connected account can be used right now.
    pub async fn account_status(&self, account: String) -> GmailStatus {
        let runtime = Arc::clone(&self.runtime);
        let engine = runtime.engine();
        rt::run(async move { engine.account_status(&account).await }).await
    }

    /// Every integration with its accounts and how an account is added to it.
    pub async fn services(&self) -> Result<Vec<ServiceView>, CoreError> {
        let runtime = Arc::clone(&self.runtime);
        let generation = runtime.generation();
        rt::run(async move {
            let engine = runtime.prepare(generation).await?;
            engine.ensure_active()?;
            let result = engine.run_account(async { engine.services() }).await;
            runtime.finish(&engine, result)
        })
        .await
    }

    /// Adds a Google account of a service just authorized (`hint` is its address), or this phone's calendar, contacts
    /// or messages once Android allowed them.
    pub async fn add_service_account(&self, service: String, hint: String) -> Result<AccountView, CoreError> {
        let runtime = Arc::clone(&self.runtime);
        let generation = runtime.generation();
        rt::run(async move {
            let engine = runtime.prepare(generation).await?;
            engine.require_decider()?;
            engine.ensure_active()?;
            let result = engine.run_account(async { engine.add_service_account(&service, &hint).await }).await;
            runtime.finish(&engine, result)
        })
        .await
    }

    /// Adds an account from a pasted access token (GitHub).
    pub async fn add_token_account(&self, service: String, token: String) -> Result<AccountView, CoreError> {
        let runtime = Arc::clone(&self.runtime);
        let token = Zeroizing::new(token);
        let generation = runtime.generation();
        rt::run(async move {
            let engine = runtime.prepare(generation).await?;
            engine.require_decider()?;
            engine.ensure_active()?;
            let result = engine.run_account(async { engine.add_token_account(&service, &token).await }).await;
            runtime.finish(&engine, result)
        })
        .await
    }

    /// Phone-number sign-in (Telegram), step 1: the login code is sent.
    pub async fn login_begin(&self, service: String, phone: String) -> Result<(), CoreError> {
        let runtime = Arc::clone(&self.runtime);
        let generation = runtime.generation();
        rt::run(async move {
            let engine = runtime.prepare(generation).await?;
            engine.require_decider()?;
            engine.ensure_active()?;
            let result = engine.run_account(async { engine.login_begin(&service, &phone).await }).await;
            runtime.finish(&engine, result)
        })
        .await
    }

    /// Step 2: the code. Done adds the account; otherwise the account's password is needed.
    pub async fn login_code(&self, service: String, code: String) -> Result<LoginProgress, CoreError> {
        let runtime = Arc::clone(&self.runtime);
        let generation = runtime.generation();
        rt::run(async move {
            let engine = runtime.prepare(generation).await?;
            engine.require_decider()?;
            engine.ensure_active()?;
            let result = engine.run_account(async { engine.login_code(&service, &code).await }).await;
            runtime.finish(&engine, result)
        })
        .await
    }

    /// Step 3, for accounts with two-step verification.
    pub async fn login_password(&self, service: String, password: String) -> Result<AccountView, CoreError> {
        let runtime = Arc::clone(&self.runtime);
        let password = Zeroizing::new(password);
        let generation = runtime.generation();
        rt::run(async move {
            let engine = runtime.prepare(generation).await?;
            engine.require_decider()?;
            engine.ensure_active()?;
            let result = engine.run_account(async { engine.login_password(&service, &password).await }).await;
            runtime.finish(&engine, result)
        })
        .await
    }

    /// Disconnects an account of any integration.
    pub async fn remove_service_account(&self, service: String, account: String) -> Result<(), CoreError> {
        let runtime = Arc::clone(&self.runtime);
        let generation = runtime.generation();
        rt::run(async move {
            let engine = runtime.prepare(generation).await?;
            engine.require_decider()?;
            engine.ensure_active()?;
            let result = engine.run_account(async { engine.remove_service_account(&service, &account).await }).await;
            runtime.finish(&engine, result)
        })
        .await
    }

    /// Whether one account of an integration can be used right now.
    pub async fn service_account_status(&self, service: String, account: String) -> GmailStatus {
        let runtime = Arc::clone(&self.runtime);
        let engine = runtime.engine();
        rt::run(async move { engine.service_account_status(&service, &account).await }).await
    }

    pub async fn activity(&self, limit: u32) -> Result<Vec<ActivityEntry>, CoreError> {
        let runtime = Arc::clone(&self.runtime);
        let generation = runtime.generation();
        rt::run(async move {
            let engine = runtime.prepare(generation).await?;
            engine.ensure_active()?;
            let result = engine.run_account(async { engine.activity(limit) }).await;
            runtime.finish(&engine, result)
        })
        .await
    }

    /// One Activity entry by its id, however far back (`activity` reads only the newest): to open an entry a day or a
    /// search brought in. `None` when this device has no such entry.
    pub async fn activity_entry(&self, entry_id: i64) -> Result<Option<ActivityEntry>, CoreError> {
        let runtime = Arc::clone(&self.runtime);
        let generation = runtime.generation();
        rt::run(async move {
            let engine = runtime.prepare(generation).await?;
            engine.ensure_active()?;
            let result = engine.run_account(async { engine.activity_entry(entry_id) }).await;
            runtime.finish(&engine, result)
        })
        .await
    }

    /// Searches the Activity on this device (nothing is sent anywhere): one page of the entries matching `query`,
    /// newest first, `limit` at most 200, starting after `after` (the previous page's `next`). Each hit says where it
    /// matched outside its title, with the matching words set apart; `terms` are the words to set apart in titles.
    pub async fn search_activity(
        &self,
        query: crate::types::ActivityQuery,
        after: Option<crate::types::ActivityCursor>,
        limit: u32,
    ) -> Result<crate::types::ActivityPage, CoreError> {
        let runtime = Arc::clone(&self.runtime);
        let generation = runtime.generation();
        rt::run(async move {
            let engine = runtime.prepare(generation).await?;
            engine.ensure_active()?;
            let result = engine.run_account(async { engine.search_activity(&query, after, limit) }).await;
            runtime.finish(&engine, result)
        })
        .await
    }

    /// When each entry matching `query` was logged, how it ended and who decided, newest first: to group the Activity
    /// by day (in the phone's time zone) and count each day without loading the entries.
    pub async fn activity_stamps(
        &self,
        query: crate::types::ActivityQuery,
    ) -> Result<Vec<crate::types::ActivityStamp>, CoreError> {
        let runtime = Arc::clone(&self.runtime);
        let generation = runtime.generation();
        rt::run(async move {
            let engine = runtime.prepare(generation).await?;
            engine.ensure_active()?;
            let result = engine.run_account(async { engine.activity_stamps(&query) }).await;
            runtime.finish(&engine, result)
        })
        .await
    }

    /// The integrations, kinds and connections the Activity holds: the search filters' choices.
    pub async fn activity_facets(&self) -> Result<crate::types::ActivityFacets, CoreError> {
        let runtime = Arc::clone(&self.runtime);
        let generation = runtime.generation();
        rt::run(async move {
            let engine = runtime.prepare(generation).await?;
            engine.ensure_active()?;
            let result = engine.run_account(async { engine.activity_facets() }).await;
            runtime.finish(&engine, result)
        })
        .await
    }

    /// Asks the token provider for a token and reports whether Gmail is usable.
    pub async fn gmail_status(&self) -> GmailStatus {
        let runtime = Arc::clone(&self.runtime);
        let engine = runtime.engine();
        rt::run(async move { engine.gmail_status().await }).await
    }

    /// A file an AI uploaded (`PendingKind::Blob`), as the server described it.
    pub async fn blob_view(&self, id: String) -> Result<crate::types::BlobView, CoreError> {
        let runtime = Arc::clone(&self.runtime);
        let generation = runtime.generation();
        rt::run(async move {
            let engine = runtime.prepare(generation).await?;
            engine.ensure_active()?;
            let result = engine.run_account(async { engine.blob_view(&id) }).await;
            runtime.finish(&engine, result)
        })
        .await
    }

    /// The user's decision on an uploaded file: approved, its download link works; refused, the server deletes it.
    pub async fn answer_blob(&self, id: String, approve: bool) -> Result<(), CoreError> {
        let runtime = Arc::clone(&self.runtime);
        let generation = runtime.generation();
        rt::run(async move {
            let engine = runtime.prepare(generation).await?;
            engine.require_decider()?;
            engine.ensure_active()?;
            let result = engine.run_account(async { engine.answer_blob(&id, approve).await }).await;
            runtime.finish(&engine, result)
        })
        .await
    }

    // ---- Autopilot (spec 2026-10-01 §9) ------------------------------------------------------------------------------

    /// Gives the core the app's ONNX runtime. Without one, Manual and Auto judge nothing (requests wait); Bypass
    /// and Lockdown work regardless. Replaces (and unloads) an earlier runtime.
    pub fn set_model_runtime(&self, runtime: Arc<dyn ModelRuntime>) {
        self.runtime.set_model(runtime);
    }

    /// The global mode, the connections' own settings, the default profile and the model.
    pub async fn autopilot_settings(&self) -> Result<AutopilotSettings, CoreError> {
        let runtime = Arc::clone(&self.runtime);
        let generation = runtime.generation();
        rt::run(async move {
            let engine = runtime.prepare(generation).await?;
            engine.ensure_active()?;
            let result = engine.run_account(async { engine.autopilot_settings() }).await;
            runtime.finish(&engine, result)
        })
        .await
    }

    /// Sets the global mode (`connection_id` = `None`) or one connection's. `mode` = `None` returns the global mode to
    /// its default, or makes the connection follow the global mode. `Bypass` lasts `minutes` (default 15, at most a
    /// year; `start_bypass` also starts one without an end) and is refused for a connection paired less than 10
    /// minutes ago; `Lockdown` also denies what is waiting.
    pub async fn set_autopilot_mode(
        &self,
        connection_id: Option<String>,
        mode: Option<AutopilotMode>,
        minutes: Option<u32>,
    ) -> Result<(), CoreError> {
        let runtime = Arc::clone(&self.runtime);
        let generation = runtime.generation();
        rt::run(async move {
            let engine = runtime.prepare(generation).await?;
            // Lockdown and stopping a running bypass are the brake: any member may pull it (Q2).
            let brake = engine.is_brake(connection_id.as_deref(), mode)?;
            if !brake {
                engine.require_decider()?;
            }
            engine.ensure_active()?;
            let restriction = if mode == Some(AutopilotMode::Lockdown) {
                Restriction::Lockdown
            } else {
                Restriction::StopBypass
            };
            let result = engine
                .run_account(async {
                    engine.set_autopilot_mode(connection_id, mode, minutes).await?;
                    if brake {
                        engine.brake_out(restriction).await?;
                    }
                    Ok(())
                })
                .await;
            runtime.finish(&engine, result)
        })
        .await
    }

    /// Starts a bypass of every AI (`connection_id` = `None`) or of one, for `minutes` (1 minute to a year) or, with
    /// `minutes` = `None`, until the user turns it off (`bypass_until` then reads `i64::MAX`). Everything is approved
    /// at once, what already waits too; only new connections and other phones still ask. A single connection paired
    /// less than 10 minutes ago cannot be bypassed. Stop it with `set_autopilot_mode` (the mode it interrupted).
    pub async fn start_bypass(&self, connection_id: Option<String>, minutes: Option<u32>) -> Result<(), CoreError> {
        let runtime = Arc::clone(&self.runtime);
        let generation = runtime.generation();
        rt::run(async move {
            let engine = runtime.prepare(generation).await?;
            engine.require_decider()?;
            engine.ensure_active()?;
            let result = engine.run_account(async { engine.start_bypass(connection_id, minutes).await }).await;
            runtime.finish(&engine, result)
        })
        .await
    }

    /// Starts Overnight (`docs/autopilot-risk.md`) for every AI (`connection_id` = `None`) or one, for `minutes` (1
    /// minute to a year) or, with `minutes` = `None`, until the user turns it off (`overnight_until` then reads
    /// `i64::MAX`). It decides like Auto, and weighs the destructive part of the hard floor with the risk model instead
    /// of always asking; what the model rates catastrophic, a tripwire, or what the phone cannot read with certainty is
    /// held: it waits for the user, or is denied (`set_overnight_when_risky`); what already waits is judged too. At
    /// its end it returns to the mode it interrupted; stop it earlier with `set_autopilot_mode`. Without a model that
    /// rates risk it decides like Auto. Every start, restart or change of its length asks `owner` first (the core asks
    /// it, with its own words); without the owner's confirmation nothing changes. Turning it off needs no check.
    pub async fn start_overnight(
        &self,
        connection_id: Option<String>,
        minutes: Option<u32>,
        owner: Arc<dyn OwnerCheck>,
    ) -> Result<(), CoreError> {
        let runtime = Arc::clone(&self.runtime);
        let generation = runtime.generation();
        rt::run(async move {
            let engine = runtime.prepare(generation).await?;
            engine.require_decider()?;
            engine.ensure_active()?;
            let result = engine
                .run_account(async { engine.start_overnight(connection_id, minutes, owner.as_ref()).await })
                .await;
            runtime.finish(&engine, result)
        })
        .await
    }

    /// What Overnight does with a request its risk model rates catastrophic: wait for the user (the default) or deny.
    /// A change either way asks `owner` first.
    pub async fn set_overnight_when_risky(
        &self,
        risky: OvernightRisky,
        owner: Arc<dyn OwnerCheck>,
    ) -> Result<(), CoreError> {
        let runtime = Arc::clone(&self.runtime);
        let generation = runtime.generation();
        rt::run(async move {
            let engine = runtime.prepare(generation).await?;
            engine.require_decider()?;
            engine.ensure_active()?;
            let result =
                engine.run_account(async { engine.set_overnight_when_risky(risky, owner.as_ref()).await }).await;
            runtime.finish(&engine, result)
        })
        .await
    }

    /// The last `days` (1 to 30) of Autopilot: requests scored, how often it agreed with the user, kinds on Auto.
    pub async fn autopilot_stats(&self, days: u32) -> Result<AutopilotStats, CoreError> {
        let runtime = Arc::clone(&self.runtime);
        let generation = runtime.generation();
        rt::run(async move {
            let engine = runtime.prepare(generation).await?;
            engine.ensure_active()?;
            let result = engine.run_account(async { engine.autopilot_stats(days) }).await;
            runtime.finish(&engine, result)
        })
        .await
    }

    /// Auto's setup (Decisions v2): the owner's world (facts and proposals) and the matrix with real names.
    pub async fn auto_setup(&self) -> Result<AutoSetup, CoreError> {
        let runtime = Arc::clone(&self.runtime);
        let generation = runtime.generation();
        rt::run(async move {
            let engine = runtime.prepare(generation).await?;
            engine.require_decider()?;
            engine.ensure_active()?;
            let result = engine.run_account(async { engine.auto_setup() }).await;
            runtime.finish(&engine, result)
        })
        .await
    }

    /// Sets the zone of one thing in the owner's world. Toward Mine asks `owner` first; toward Outside needs nothing.
    pub async fn set_world_zone(
        &self,
        kind: FactChoice,
        key: String,
        zone: ZoneChoice,
        owner: Arc<dyn OwnerCheck>,
    ) -> Result<(), CoreError> {
        let runtime = Arc::clone(&self.runtime);
        let generation = runtime.generation();
        rt::run(async move {
            let engine = runtime.prepare(generation).await?;
            engine.require_decider()?;
            engine.ensure_active()?;
            let result =
                engine.run_account(async { engine.set_world_zone(kind, &key, zone, owner.as_ref()).await }).await;
            runtime.finish(&engine, result)
        })
        .await
    }

    /// Sets one matrix cell (`level` 0 to 3). Looser asks `owner` first.
    pub async fn set_matrix_cell(
        &self,
        zone: ZoneChoice,
        level: u8,
        cell: CellChoice,
        owner: Arc<dyn OwnerCheck>,
    ) -> Result<(), CoreError> {
        let runtime = Arc::clone(&self.runtime);
        let generation = runtime.generation();
        rt::run(async move {
            let engine = runtime.prepare(generation).await?;
            engine.require_decider()?;
            engine.ensure_active()?;
            let result =
                engine.run_account(async { engine.set_matrix_cell(zone, level, cell, owner.as_ref()).await }).await;
            runtime.finish(&engine, result)
        })
        .await
    }

    /// Finishes Auto's setup and turns Auto on, after `owner` confirms.
    pub async fn finish_auto_setup(&self, owner: Arc<dyn OwnerCheck>) -> Result<(), CoreError> {
        let runtime = Arc::clone(&self.runtime);
        let generation = runtime.generation();
        rt::run(async move {
            let engine = runtime.prepare(generation).await?;
            engine.require_decider()?;
            engine.ensure_active()?;
            let result = engine.run_account(async { engine.finish_auto_setup(owner.as_ref()).await }).await;
            runtime.finish(&engine, result)
        })
        .await
    }

    /// Lets the matrix decide (asks `owner`), or returns it to shadow.
    pub async fn set_matrix_decides(&self, on: bool, owner: Arc<dyn OwnerCheck>) -> Result<(), CoreError> {
        let runtime = Arc::clone(&self.runtime);
        let generation = runtime.generation();
        rt::run(async move {
            let engine = runtime.prepare(generation).await?;
            engine.require_decider()?;
            engine.ensure_active()?;
            let result = engine.run_account(async { engine.set_matrix_decides(on, owner.as_ref()).await }).await;
            runtime.finish(&engine, result)
        })
        .await
    }

    /// The open "Auto needs information" questions, newest first. They are listed, never notified.
    pub async fn auto_questions(&self) -> Result<Vec<AutoQuestion>, CoreError> {
        let runtime = Arc::clone(&self.runtime);
        let generation = runtime.generation();
        rt::run(async move {
            let engine = runtime.prepare(generation).await?;
            engine.ensure_active()?;
            let result = engine.run_account(async { engine.auto_questions() }).await;
            runtime.finish(&engine, result)
        })
        .await
    }

    /// Answers a question with a zone (anything but Outside asks `owner` first).
    pub async fn answer_auto_question(
        &self,
        id: String,
        zone: ZoneChoice,
        owner: Arc<dyn OwnerCheck>,
    ) -> Result<(), CoreError> {
        let runtime = Arc::clone(&self.runtime);
        let generation = runtime.generation();
        rt::run(async move {
            let engine = runtime.prepare(generation).await?;
            engine.require_decider()?;
            engine.ensure_active()?;
            let result =
                engine.run_account(async { engine.answer_auto_question(&id, zone, owner.as_ref()).await }).await;
            runtime.finish(&engine, result)
        })
        .await
    }

    /// Dismisses a question: the thing stays Outside and isn't asked about again.
    pub async fn dismiss_auto_question(&self, id: String) -> Result<(), CoreError> {
        let runtime = Arc::clone(&self.runtime);
        let generation = runtime.generation();
        rt::run(async move {
            let engine = runtime.prepare(generation).await?;
            engine.require_decider()?;
            engine.ensure_active()?;
            let result = engine.run_account(async { engine.dismiss_auto_question(&id) }).await;
            runtime.finish(&engine, result)
        })
        .await
    }

    /// "Ask me next time" on something Auto did: an exception for that kind of thing, and a 24-hour pause.
    pub async fn ask_me_next_time(&self, request_id: String) -> Result<(), CoreError> {
        let runtime = Arc::clone(&self.runtime);
        let generation = runtime.generation();
        rt::run(async move {
            let engine = runtime.prepare(generation).await?;
            engine.require_decider()?;
            engine.ensure_active()?;
            let result = engine.run_account(async { engine.ask_me_next_time(&request_id) }).await;
            runtime.finish(&engine, result)
        })
        .await
    }

    /// Whether the app's download job waits for Wi-Fi (kept here; the job honours it).
    pub async fn set_autopilot_wifi_only(&self, wifi_only: bool) -> Result<(), CoreError> {
        let runtime = Arc::clone(&self.runtime);
        let generation = runtime.generation();
        rt::run(async move {
            let engine = runtime.prepare(generation).await?;
            engine.ensure_active()?;
            let result = engine.run_account(async { engine.set_autopilot_wifi_only(wifi_only) }).await;
            runtime.finish(&engine, result)
        })
        .await
    }

    /// Every profile with its classes; Personal (the default) and Work are created on first use.
    pub async fn autopilot_profiles(&self) -> Result<Vec<ProfileView>, CoreError> {
        let runtime = Arc::clone(&self.runtime);
        let generation = runtime.generation();
        rt::run(async move {
            let engine = runtime.prepare(generation).await?;
            engine.ensure_active()?;
            let result = engine.run_account(async { engine.autopilot_profiles() }).await;
            runtime.finish(&engine, result)
        })
        .await
    }

    pub async fn create_profile(&self, name: String, icon: Option<String>) -> Result<ProfileView, CoreError> {
        let runtime = Arc::clone(&self.runtime);
        let generation = runtime.generation();
        rt::run(async move {
            let engine = runtime.prepare(generation).await?;
            engine.require_decider()?;
            engine.ensure_active()?;
            let result = engine.run_account(async { engine.create_profile(&name, icon) }).await;
            runtime.finish(&engine, result)
        })
        .await
    }

    pub async fn rename_profile(
        &self,
        profile_id: String,
        name: String,
        icon: Option<String>,
    ) -> Result<(), CoreError> {
        let runtime = Arc::clone(&self.runtime);
        let generation = runtime.generation();
        rt::run(async move {
            let engine = runtime.prepare(generation).await?;
            engine.require_decider()?;
            engine.ensure_active()?;
            let result = engine.run_account(async { engine.rename_profile(&profile_id, &name, icon) }).await;
            runtime.finish(&engine, result)
        })
        .await
    }

    /// Deletes a profile and everything it learned (not the last one); its connections use the default profile.
    pub async fn delete_profile(&self, profile_id: String) -> Result<(), CoreError> {
        let runtime = Arc::clone(&self.runtime);
        let generation = runtime.generation();
        rt::run(async move {
            let engine = runtime.prepare(generation).await?;
            engine.require_decider()?;
            engine.ensure_active()?;
            let result = engine.run_account(async { engine.delete_profile(&profile_id) }).await;
            runtime.finish(&engine, result)
        })
        .await
    }

    /// Forgets what a profile learned: memory, adapter, classes locked or unlocked by hand.
    pub async fn reset_profile(&self, profile_id: String) -> Result<(), CoreError> {
        let runtime = Arc::clone(&self.runtime);
        let generation = runtime.generation();
        rt::run(async move {
            let engine = runtime.prepare(generation).await?;
            engine.require_decider()?;
            engine.ensure_active()?;
            let result = engine.run_account(async { engine.reset_profile(&profile_id) }).await;
            runtime.finish(&engine, result)
        })
        .await
    }

    /// The profile of every connection without one of its own.
    pub async fn set_default_profile(&self, profile_id: String) -> Result<(), CoreError> {
        let runtime = Arc::clone(&self.runtime);
        let generation = runtime.generation();
        rt::run(async move {
            let engine = runtime.prepare(generation).await?;
            engine.require_decider()?;
            engine.ensure_active()?;
            let result = engine.run_account(async { engine.set_default_profile(&profile_id) }).await;
            runtime.finish(&engine, result)
        })
        .await
    }

    /// The profile a connection's decisions train (`None` = the default profile).
    pub async fn assign_profile(&self, connection_id: String, profile_id: Option<String>) -> Result<(), CoreError> {
        let runtime = Arc::clone(&self.runtime);
        let generation = runtime.generation();
        rt::run(async move {
            let engine = runtime.prepare(generation).await?;
            engine.require_decider()?;
            engine.ensure_active()?;
            let result = engine.run_account(async { engine.assign_profile(&connection_id, profile_id) }).await;
            runtime.finish(&engine, result)
        })
        .await
    }

    /// Locks (`Some(true)`) or unlocks (`Some(false)`, with a warning in the app) a class by hand; `None` lets the
    /// numbers decide again.
    pub async fn set_class_lock(
        &self,
        profile_id: String,
        class_key: String,
        locked: Option<bool>,
    ) -> Result<(), CoreError> {
        let runtime = Arc::clone(&self.runtime);
        let generation = runtime.generation();
        rt::run(async move {
            let engine = runtime.prepare(generation).await?;
            engine.require_decider()?;
            engine.ensure_active()?;
            let result = engine.run_account(async { engine.set_class_lock(&profile_id, &class_key, locked) }).await;
            runtime.finish(&engine, result)
        })
        .await
    }

    pub async fn set_preset(&self, profile_id: String, preset: Preset) -> Result<(), CoreError> {
        let runtime = Arc::clone(&self.runtime);
        let generation = runtime.generation();
        rt::run(async move {
            let engine = runtime.prepare(generation).await?;
            engine.require_decider()?;
            engine.ensure_active()?;
            let result = engine.run_account(async { engine.set_preset(&profile_id, preset) }).await;
            runtime.finish(&engine, result)
        })
        .await
    }

    /// A profile's own thresholds (the `Custom` preset), each clamped to 0.80–0.99.
    pub async fn set_thresholds(&self, profile_id: String, approve_at: f32, deny_at: f32) -> Result<(), CoreError> {
        let runtime = Arc::clone(&self.runtime);
        let generation = runtime.generation();
        rt::run(async move {
            let engine = runtime.prepare(generation).await?;
            engine.require_decider()?;
            engine.ensure_active()?;
            let result = engine.run_account(async { engine.set_thresholds(&profile_id, approve_at, deny_at) }).await;
            runtime.finish(&engine, result)
        })
        .await
    }

    /// What the profile's last 30 days would have been at these thresholds: decided, asked, and decided otherwise
    /// than the user wanted.
    pub async fn threshold_replay(
        &self,
        profile_id: String,
        approve_at: f32,
        deny_at: f32,
    ) -> Result<ThresholdReplay, CoreError> {
        let runtime = Arc::clone(&self.runtime);
        let generation = runtime.generation();
        rt::run(async move {
            let engine = runtime.prepare(generation).await?;
            engine.ensure_active()?;
            let result = engine.run_account(async { engine.threshold_replay(&profile_id, approve_at, deny_at) }).await;
            runtime.finish(&engine, result)
        })
        .await
    }

    /// What Autopilot made of a waiting request, for the approval screen (`None` before it looked at it).
    pub async fn autopilot_suggestion(&self, request_id: String) -> Result<Option<SuggestionView>, CoreError> {
        let runtime = Arc::clone(&self.runtime);
        let generation = runtime.generation();
        rt::run(async move {
            let engine = runtime.prepare(generation).await?;
            engine.ensure_active()?;
            let result = engine.run_account(async { engine.autopilot_suggestion(&request_id) }).await;
            runtime.finish(&engine, result)
        })
        .await
    }

    /// "This was wrong" on an activity entry (`should_have` is `Approve` or `Deny`).
    pub async fn correct_decision(&self, activity_id: i64, should_have: Verdict) -> Result<(), CoreError> {
        let runtime = Arc::clone(&self.runtime);
        let generation = runtime.generation();
        rt::run(async move {
            let engine = runtime.prepare(generation).await?;
            engine.require_decider()?;
            engine.ensure_active()?;
            let result = engine.run_account(async { engine.correct_decision(activity_id, should_have).await }).await;
            runtime.finish(&engine, result)
        })
        .await
    }

    pub async fn model_status(&self) -> ModelStatus {
        let runtime = Arc::clone(&self.runtime);
        let engine = runtime.engine();
        rt::run(async move { engine.model_status() }).await
    }

    /// Downloads the model and checks it against the hashes built into the core; nothing is kept if it does not match.
    pub async fn download_model(&self, progress: Arc<dyn DownloadProgress>) -> Result<ModelStatus, CoreError> {
        let runtime = Arc::clone(&self.runtime);
        let generation = runtime.generation();
        rt::run(async move {
            let engine = runtime.prepare(generation).await?;
            engine.ensure_active()?;
            let result = engine.run_account(async { engine.download_model(progress).await }).await;
            runtime.finish(&engine, result)
        })
        .await
    }

    /// Updates the model when a newer one this build trusts is published: downloaded (continuing an interrupted
    /// download), checked against the hashes built into the core, run once, and only then put in use, between
    /// decisions. The model in use stays if anything fails. `automatic`: the app's own check, which does nothing when
    /// "Update automatically" is off or the newer model already failed; else the user's "Check now". The app calls it
    /// only when the network allows (Wi-Fi, not Low Power or Low Data mode, for an automatic one).
    pub async fn update_model(
        &self,
        automatic: bool,
        progress: Arc<dyn DownloadProgress>,
    ) -> Result<ModelUpdateOutcome, CoreError> {
        let runtime = Arc::clone(&self.runtime);
        let generation = runtime.generation();
        rt::run(async move {
            let engine = runtime.prepare(generation).await?;
            engine.ensure_active()?;
            let result = engine.run_account(async { engine.update_model(automatic, progress).await }).await;
            runtime.finish(&engine, result)
        })
        .await
    }

    /// Stops the model download running now; it ends with `CoreError::Network` and keeps what arrived, so the next
    /// `update_model` goes on from there. The app calls it when the phone leaves Wi-Fi during an automatic update (or
    /// Low Power or Low Data Mode comes on). A downloaded model already being checked is not stopped.
    pub async fn pause_model_download(&self) {
        let runtime = Arc::clone(&self.runtime);
        let engine = runtime.engine();
        rt::run(async move { engine.pause_model_download() }).await;
    }

    /// An automatic update has something to do (asks nobody): the app then waits for Wi-Fi to call `update_model`.
    pub async fn model_update_pending(&self) -> bool {
        let runtime = Arc::clone(&self.runtime);
        let engine = runtime.engine();
        rt::run(async move { engine.model_update_pending() }).await
    }

    /// "Update automatically" (on by default).
    pub async fn set_autopilot_auto_update(&self, on: bool) -> Result<(), CoreError> {
        let runtime = Arc::clone(&self.runtime);
        let generation = runtime.generation();
        rt::run(async move {
            let engine = runtime.prepare(generation).await?;
            engine.ensure_active()?;
            let result = engine.run_account(async { engine.set_autopilot_auto_update(on) }).await;
            runtime.finish(&engine, result)
        })
        .await
    }

    pub async fn delete_model(&self) -> Result<(), CoreError> {
        let runtime = Arc::clone(&self.runtime);
        let generation = runtime.generation();
        rt::run(async move {
            let engine = runtime.prepare(generation).await?;
            engine.ensure_active()?;
            let result = engine.run_account(async { engine.delete_model() }).await;
            runtime.finish(&engine, result)
        })
        .await
    }

    /// The "Try it" playground: how a profile judges a situation typed in the §4 format (nothing is kept).
    pub async fn autopilot_evaluate(
        &self,
        profile_id: Option<String>,
        situation: String,
    ) -> Result<SuggestionView, CoreError> {
        let runtime = Arc::clone(&self.runtime);
        let generation = runtime.generation();
        rt::run(async move {
            let engine = runtime.prepare(generation).await?;
            engine.ensure_active()?;
            let result = engine.run_account(async { engine.autopilot_evaluate(profile_id, situation).await }).await;
            runtime.finish(&engine, result)
        })
        .await
    }
}
