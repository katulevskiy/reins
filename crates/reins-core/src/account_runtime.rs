//! The security boundary shared by every mobile API call. Each authenticated account gets a separate runtime,
//! key, encrypted database, connector sessions, and generation. Old work never follows a mutable store pointer.
use crate::{
    CoreError, GoogleTokenProvider, KeyWrapper, Notifier,
    connector::Connector,
    crypto::VaultKey,
    engine::{CoreConfig, Engine},
    session::{Session, account_id_key},
    sso,
    store::{Owner, Store},
    types::{SessionInfo, SsoOutcome},
};
use reins_proto::connector::VAULT;
use std::{
    path::PathBuf,
    sync::{Arc, Mutex, PoisonError},
};
use zeroize::Zeroizing;
const KEY_CACHE: &str = "reins.owner-vault-key";
const SECRET_CACHE: &str = "reins.owner-account-secret";

pub(crate) struct AccountRuntime {
    pub(crate) current: Mutex<Arc<Engine>>,
    control: Arc<Store>,
    directory: PathBuf,
    google: Arc<dyn GoogleTokenProvider>,
    notifier: Arc<dyn Notifier>,
    cfg: CoreConfig,
    connectors: Vec<Arc<dyn Connector>>,
    transition: tokio::sync::Mutex<()>,
    model: Mutex<Option<Arc<dyn crate::autopilot::ModelRuntime>>>,
    initialized: std::sync::atomic::AtomicBool,
    generation: std::sync::atomic::AtomicU64,
}
impl AccountRuntime {
    pub fn new(
        directory: PathBuf,
        keys: &dyn KeyWrapper,
        google: Arc<dyn GoogleTokenProvider>,
        notifier: Arc<dyn Notifier>,
        cfg: CoreConfig,
        connectors: Vec<Arc<dyn Connector>>,
    ) -> Result<Arc<Self>, CoreError> {
        let control = Arc::new(Store::open(&directory, keys)?);
        let store = Arc::new(Store::ephemeral(Some(Arc::clone(&control)))?);
        if let Ok(Some(encoded)) = control.meta_get("active-owner")
            && let Ok(owner) = serde_json::from_str::<Owner>(&encoded)
            && let Ok(Some(saved)) = control.load_session()
            && owner.server == saved.server_url
            && let Ok(server) = crate::http::ServerUrl::parse(&saved.server_url)
            && control.meta_get(&account_id_key(&server, &saved.email))?.as_deref() == Some(&owner.user_id)
        {
            store.bind_control_owner(owner);
        }
        let engine = Engine::with_store(
            &directory,
            store,
            Arc::clone(&google),
            Arc::clone(&notifier),
            cfg.clone(),
            connectors.clone(),
        )?;
        Ok(Arc::new(Self {
            current: Mutex::new(engine),
            control,
            directory,
            google,
            notifier,
            cfg,
            connectors,
            transition: tokio::sync::Mutex::new(()),
            model: Mutex::new(None),
            generation: std::sync::atomic::AtomicU64::new(0),
            initialized: std::sync::atomic::AtomicBool::new(false),
        }))
    }
    pub fn engine(&self) -> Arc<Engine> {
        Arc::clone(&self.current.lock().unwrap_or_else(PoisonError::into_inner))
    }
    fn build(&self, store: Arc<Store>) -> Result<Arc<Engine>, CoreError> {
        let engine = Engine::with_store(
            &self.directory,
            store,
            Arc::clone(&self.google),
            Arc::clone(&self.notifier),
            self.cfg.clone(),
            self.connectors.clone(),
        )?;
        if let Some(model) = self.model.lock().unwrap_or_else(PoisonError::into_inner).as_ref() {
            engine.set_model_runtime(Arc::clone(model));
        }
        Ok(engine)
    }
    pub fn set_model(&self, model: Arc<dyn crate::autopilot::ModelRuntime>) {
        self.engine().set_model_runtime(Arc::clone(&model));
        *self.model.lock().unwrap_or_else(PoisonError::into_inner) = Some(model);
    }
    pub async fn session(&self) -> Option<SessionInfo> {
        let _guard = self.transition.lock().await;
        let engine = self.engine();
        self.control.load_session().ok().flatten()?;
        if engine.store.owner.is_none()
            && let Ok(Some(encoded)) = self.control.meta_get("active-owner")
            && let Ok(owner) = serde_json::from_str::<Owner>(&encoded)
            && let Ok(Some(key)) = self.control.secret_get(KEY_CACHE, &owner.id())
            && let Ok(key) = VaultKey::from_bytes(&key)
            && let Ok(session) = engine.session()
            && owner.server == session.server.as_str()
            && self.control.meta_get(&account_id_key(&session.server, &session.email())).ok().flatten().as_deref()
                == Some(&owner.user_id)
            && self.activate(&engine, owner, &key).await.is_err()
        {
            return None;
        }
        self.initialized.store(true, std::sync::atomic::Ordering::Release);
        let current = self.engine();
        if current.ensure_active().is_err() {
            return None;
        }
        current.session_info()
    }
    pub fn generation(&self) -> u64 {
        self.generation.load(std::sync::atomic::Ordering::Acquire)
    }
    pub async fn prepare(&self, generation: u64) -> Result<Arc<Engine>, CoreError> {
        if generation != self.generation() {
            return Err(CoreError::NotLoggedIn);
        }
        if !self.initialized.load(std::sync::atomic::Ordering::Acquire) {
            self.session().await;
        }
        if generation != self.generation() {
            return Err(CoreError::NotLoggedIn);
        }
        let engine = self.engine();
        engine.ensure_active()?;
        Ok(engine)
    }
    fn owner(session: &Session, id: String) -> Owner {
        Owner {
            server: session.server.as_str().to_owned(),
            user_id: id,
        }
    }
    fn cache_into(&self, store: &Store, owner: &Owner, email: &str) -> Result<(), CoreError> {
        if let Some(raw) = self.control.secret_get(SECRET_CACHE, &owner.id())? {
            store.secret_put(sso::SECRET_SERVICE, &owner.user_id, &raw)?;
        }
        if let Some(raw) = self.control.secret_get(KEY_CACHE, &owner.id())? {
            store.secret_put(VAULT, email, &raw)?;
        }
        Ok(())
    }
    async fn activate(&self, source: &Arc<Engine>, owner: Owner, key: &VaultKey) -> Result<Arc<Engine>, CoreError> {
        let session = source.session()?;
        let email = session.email();
        let secret = source
            .store
            .secret_get(sso::SECRET_SERVICE, &owner.user_id)?
            .or(self.control.secret_get(SECRET_CACHE, &owner.id())?);
        self.control.bind_account_session(&session.stored().await, &owner)?;
        self.control.secret_put(KEY_CACHE, &owner.id(), &key.to_bytes())?;
        if let Some(secret) = &secret {
            self.control.secret_put(SECRET_CACHE, &owner.id(), secret)?;
        }
        let store =
            Arc::new(Store::account(Arc::clone(&self.control), &self.directory.join("accounts"), owner.clone(), key)?);
        // Installation keys are excluded from remote snapshots. Only this verified account's keys are installed.
        if let Some(secret) = secret {
            store.secret_put(sso::SECRET_SERVICE, &owner.user_id, &secret)?;
        }
        let engine = self.build(Arc::clone(&store))?;
        engine.set_session(Some(Arc::new(session.rebind(store).await)));

        self.initialized.store(true, std::sync::atomic::Ordering::Release);
        let previous = {
            let mut slot = self.current.lock().unwrap_or_else(PoisonError::into_inner);
            std::mem::replace(&mut *slot, Arc::clone(&engine))
        };
        if !Arc::ptr_eq(&previous, &engine) {
            previous.retire()?;
        }
        engine.restore_account_state().await?;
        engine.keep_vault_key(&email, key)?;
        Ok(engine)
    }
    pub async fn sso_finish(
        &self,
        server_url: &str,
        callback_url: &str,
        state: &str,
        verifier: Zeroizing<String>,
    ) -> Result<SsoOutcome, CoreError> {
        let _guard = self.transition.lock().await;
        self.generation.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
        let server = crate::http::ServerUrl::parse(server_url)?;
        let code = sso::callback_code(callback_url, state)?;
        let signed =
            sso::exchange(&crate::http::client()?, &server, &code, &verifier, &self.control.device_id()?).await?;
        let owner = Owner {
            server: server.as_str().to_owned(),
            user_id: signed.user_id.clone(),
        };
        let store = Arc::new(Store::ephemeral(Some(Arc::clone(&self.control)))?);
        self.cache_into(&store, &owner, &signed.email)?;
        // Legacy keys have a verified server/email -> subject binding. Do not migrate unowned history or tokens.
        if store.secret_get(sso::SECRET_SERVICE, &owner.user_id)?.is_none()
            && self.control.meta_get(&account_id_key(&server, &signed.email))?.as_deref()
                == Some(owner.user_id.as_str())
            && let Some(raw) = self.control.secret_get(sso::SECRET_SERVICE, &owner.user_id)?
        {
            store.secret_put(sso::SECRET_SERVICE, &owner.user_id, &raw)?;
        }
        let source = self.build(store)?;
        let outcome = source.finish_sso(&server, signed).await?;
        if let Some(raw) = source.store.secret_get(VAULT, &outcome.session.email)? {
            self.activate(&source, owner, &VaultKey::from_bytes(&raw)?).await?;
        } else {
            self.control.bind_account_session(&source.session()?.stored().await, &owner)?;
            source.store.bind_control_owner(owner);
            let previous = {
                let mut slot = self.current.lock().unwrap_or_else(PoisonError::into_inner);
                std::mem::replace(&mut *slot, source)
            };
            previous.retire()?;
        }
        Ok(outcome)
    }
    pub async fn login(
        &self,
        server_url: &str,
        email: &str,
        password: Zeroizing<String>,
        totp: Option<String>,
    ) -> Result<SessionInfo, CoreError> {
        let _guard = self.transition.lock().await;
        self.generation.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
        let source = self.build(Arc::new(Store::ephemeral(Some(Arc::clone(&self.control)))?))?;
        let info = source.login(server_url, email, password.clone(), totp).await?;
        self.finish_password_login(&source, info, &password).await
    }
    pub async fn create_account(
        &self,
        server_url: &str,
        email: &str,
        password: Zeroizing<String>,
    ) -> Result<SessionInfo, CoreError> {
        let _guard = self.transition.lock().await;
        self.generation.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
        let source = self.build(Arc::new(Store::ephemeral(Some(Arc::clone(&self.control)))?))?;
        let info = source.create_account(server_url, email, password.clone()).await?;
        self.finish_password_login(&source, info, &password).await
    }
    async fn finish_password_login(
        &self,
        source: &Arc<Engine>,
        info: SessionInfo,
        password: &str,
    ) -> Result<SessionInfo, CoreError> {
        let session = source.session()?;
        let id = session.account_user_id().await?;
        let owner = Self::owner(&session, id);
        self.cache_into(&source.store, &owner, &info.email)?;
        if source.store.secret_get(VAULT, &info.email)?.is_none() {
            source.add_token_account(VAULT, password).await?;
        }
        let raw = source.store.secret_get(VAULT, &info.email)?.ok_or(CoreError::NotLoggedIn)?;
        self.activate(source, owner, &VaultKey::from_bytes(&raw)?).await?;
        Ok(info)
    }
    pub async fn activate_unlocked(&self) -> Result<(), CoreError> {
        let _guard = self.transition.lock().await;
        let source = self.engine();
        if source.store.owner.is_some() {
            return Ok(());
        }
        let session = source.session()?;
        let owner = Self::owner(&session, session.account_user_id().await?);
        let raw = source.store.secret_get(VAULT, &session.email())?.ok_or(CoreError::NotLoggedIn)?;
        self.activate(&source, owner, &VaultKey::from_bytes(&raw)?).await?;
        Ok(())
    }
    pub async fn logout(&self) -> Result<(), CoreError> {
        self.logout_with_browser(false).await.map(drop)
    }
    pub async fn logout_with_browser(&self, browser: bool) -> Result<Option<String>, CoreError> {
        let _guard = self.transition.lock().await;
        self.generation.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
        let previous = self.engine();
        // Best-effort remote upload; the durable local ciphertext remains available even while offline.
        drop(tokio::time::timeout(std::time::Duration::from_secs(2), previous.sync_account_state()).await);
        // Bounded, best-effort provider logout preparation. Local ciphertext and account isolation must still
        // be locked when offline, talking to an older server, or unable to refresh an expired access token.
        let browser_url = if browser {
            let get_url = async {
                let session = previous.session()?;
                crate::session::api_call!(&session, |api| api.logout_browser_url())
                    .map_err(crate::phone_api::ApiFailure::into_core)
            };
            tokio::time::timeout(std::time::Duration::from_secs(3), get_url).await.ok().and_then(Result::ok).flatten()
        } else {
            None
        };
        previous.ensure_active()?;
        self.control.clear_session_for(previous.store.bound_owner().as_ref())?;
        previous.retire()?;
        let fresh = self.build(Arc::new(Store::ephemeral(Some(Arc::clone(&self.control)))?))?;
        *self.current.lock().unwrap_or_else(PoisonError::into_inner) = fresh;
        Ok(browser_url)
    }
    pub fn check_engine(&self, engine: &Arc<Engine>) -> Result<(), CoreError> {
        engine.ensure_active()?;
        if !Arc::ptr_eq(&self.engine(), engine) {
            return Err(CoreError::NotLoggedIn);
        }
        Ok(())
    }
    pub async fn finish<T>(&self, engine: &Arc<Engine>, result: Result<T, CoreError>) -> Result<T, CoreError> {
        engine.ensure_active()?;
        if !Arc::ptr_eq(&self.engine(), engine) {
            return Err(CoreError::NotLoggedIn);
        }
        engine.store.flush()?;
        if engine.store.owner.is_some() {
            engine.run_account(engine.sync_account_state()).await.ok();
        }
        engine.ensure_active()?;
        if !Arc::ptr_eq(&self.engine(), engine) {
            return Err(CoreError::NotLoggedIn);
        }
        result
    }
}

/// Native notifications must obey the same account generation as data and keys.
pub(crate) struct ScopedNotifier {
    pub target: Arc<dyn Notifier>,
    pub store: std::sync::Weak<Store>,
}
impl ScopedNotifier {
    fn active(&self) -> bool {
        self.store.upgrade().is_some_and(|store| store.check_owner().is_ok())
    }
}
impl Notifier for ScopedNotifier {
    fn item_pending(&self, item: crate::types::PendingItem) {
        if self.active() {
            self.target.item_pending(item);
        }
    }
    fn item_resolved(&self, id: String) {
        if self.active() {
            self.target.item_resolved(id);
        }
    }
    fn auto_decided(&self, decision: crate::autopilot::AutoDecisionView) {
        if self.active() {
            self.target.auto_decided(decision);
        }
    }
    fn autopilot_changed(&self, event: crate::autopilot::AutopilotEvent) {
        if self.active() {
            self.target.autopilot_changed(event);
        }
    }
}
