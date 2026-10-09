//! The object Kotlin holds (contracts §D). Every method runs on the core runtime
//! (`rt::run`), so the calling coroutine never blocks and a cancelled coroutine
//! never aborts work that already reached the server.

use std::path::PathBuf;
use std::sync::Arc;

use zeroize::Zeroizing;

use crate::autopilot::{
    AutopilotMode, AutopilotSettings, DownloadProgress, ModelRuntime, ModelStatus, Preset, ProfileView, SuggestionView,
    Verdict,
};
use crate::connector::device::{DeviceBridge, DeviceCalendar, DeviceContacts, Sms};
use crate::connector::{Connector, LoginProgress};
use crate::engine::{CoreConfig, Engine};
use crate::types::{
    AccountKeys, AccountView, ActivityEntry, ApprovalChoice, ApprovalKind, ApprovalView, ConnectionView, EmailContent,
    GmailStatus, GrantView, JoinProgress, JoinStart, JoinView, PairingView, PendingItem, ServiceView, SessionInfo,
    SsoOutcome, SsoStart, StandingGrant,
};
use crate::vault_passkey::{VaultPasskeyOptions, VaultPasskeyView};
use crate::{CoreError, GoogleTokenProvider, KeyWrapper, Notifier, rt};

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

    /// The signed-in session, if any.
    pub async fn session(&self) -> Option<SessionInfo> {
        let runtime = Arc::clone(&self.runtime);
        rt::run(async move { runtime.session().await }).await
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
            engine.ensure_active()?;
            let result = engine.run_account(async { engine.answer_join(&id, approve).await }).await;
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

    /// Like `handle_push` without Autopilot's pass, for a process that cannot run the model (the iOS notification
    /// extension): the item waits unjudged, and the app's next pass judges it.
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

    /// Items parked for the user, newest first.
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

    pub async fn approve(&self, request_id: String, choice: ApprovalChoice) -> Result<(), CoreError> {
        let runtime = Arc::clone(&self.runtime);
        let generation = runtime.generation();
        rt::run(async move {
            let engine = runtime.prepare(generation).await?;
            engine.ensure_active()?;
            let result = engine.run_account(async { engine.approve(&request_id, choice).await }).await;
            runtime.finish(&engine, result)
        })
        .await
    }

    pub async fn deny(&self, request_id: String) -> Result<(), CoreError> {
        let runtime = Arc::clone(&self.runtime);
        let generation = runtime.generation();
        rt::run(async move {
            let engine = runtime.prepare(generation).await?;
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
            let result = engine.run_account(async { engine.pairing_view(&pairing_id) }).await;
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

    pub async fn answer_pairing(
        &self,
        pairing_id: String,
        approve: bool,
        chosen_code: Option<u8>,
        label: Option<String>,
    ) -> Result<(), CoreError> {
        let runtime = Arc::clone(&self.runtime);
        let generation = runtime.generation();
        rt::run(async move {
            let engine = runtime.prepare(generation).await?;
            engine.ensure_active()?;
            let result = engine
                .run_account(async { engine.answer_pairing(&pairing_id, approve, chosen_code, label).await })
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
            let result = engine.run_account(async { engine.revoke_grant(&grant_id) }).await;
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
            engine.ensure_active()?;
            let result = engine.run_account(async { engine.revoke_connection(&connection_id).await }).await;
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
            engine.ensure_active()?;
            let result = engine.run_account(async { engine.set_connection_icon(&connection_id, icon) }).await;
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
            engine.ensure_active()?;
            let result = engine.run_account(async { engine.resume_grant(&grant_id, duration_secs) }).await;
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
            let result = engine.run_account(async { engine.delete_grant(&grant_id) }).await;
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
            engine.ensure_active()?;
            let result = engine.run_account(async { engine.answer_blob(&id, approve).await }).await;
            runtime.finish(&engine, result)
        })
        .await
    }

    // ---- Autopilot (spec 2026-10-01 §9) ------------------------------------------------------------------------------

    /// Gives the core the app's ONNX runtime. Without one, Assisted and Auto judge nothing (requests wait); Bypass
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
    /// its default, or makes the connection follow the global mode. `Bypass` lasts `minutes` (default 15, at most 60)
    /// and is refused for a connection paired less than 10 minutes ago; `Lockdown` also denies what is waiting.
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
            engine.ensure_active()?;
            let result =
                engine.run_account(async { engine.set_autopilot_mode(connection_id, mode, minutes).await }).await;
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
            engine.ensure_active()?;
            let result = engine.run_account(async { engine.set_preset(&profile_id, preset) }).await;
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
