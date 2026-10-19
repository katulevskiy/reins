//! Typed permissions facade. Authentication generation and owner confirmation match all other native core APIs.

use std::sync::Arc;

use super::ReinsCore;
use crate::permission_types::{
    AccountPermissionPreset, AccountPermissionsDraft, AccountPermissionsView, DelegationChange, DelegationDraft,
    DelegationView, PermissionCapability, PermissionPreview, PermissionScopeOptions,
};
use crate::{CoreError, OwnerCheck, RosterSummary, rt};
use zeroize::Zeroizing;

macro_rules! on_engine {
    ($core:ident, |$engine:ident| $body:expr) => {{
        let runtime = Arc::clone(&$core.runtime);
        let generation = runtime.generation();
        rt::run(async move {
            let $engine = runtime.prepare(generation).await?;
            $engine.ensure_active()?;
            let result = $engine.run_account(async { $body }).await;
            runtime.finish(&$engine, result)
        })
        .await
    }};
}

#[uniffi::export]
impl ReinsCore {
    /// Explicit legacy-account setup. The owner callback runs after preparing this account's engine, so native
    /// code can reject a stale screen/session before consent. Logout retires that engine and cancels the attempt.
    /// Confirm that recovery information is saved before authorizing; existing rosters require their normal flow.
    pub async fn roster_create_authorized(
        &self,
        device_name: String,
        code_or_password: Option<String>,
        owner: Arc<dyn OwnerCheck>,
    ) -> Result<RosterSummary, CoreError> {
        let typed = code_or_password.map(Zeroizing::new);
        on_engine!(self, |engine| {
            let session = engine.session()?;
            let reason = format!(
                "Enable automatic permission setup for {} with this phone as the verified primary. Confirm that your recovery information is saved. Account permissions still require a separate explicit save.",
                session.email()
            );
            if !owner.confirm(reason).await.map_err(|_| CoreError::invalid("the owner check failed"))? {
                return Err(CoreError::invalid("the owner did not confirm primary phone setup"));
            }
            engine.ensure_active()?;
            engine.roster_create(&device_name, typed).await
        })
    }

    pub async fn account_permissions(
        &self,
        service: String,
        account: String,
    ) -> Result<AccountPermissionsView, CoreError> {
        on_engine!(self, |engine| engine.account_permissions(&service, &account).await)
    }

    pub async fn account_permission_draft(
        &self,
        service: String,
        account: String,
        preset: AccountPermissionPreset,
    ) -> Result<AccountPermissionsDraft, CoreError> {
        on_engine!(self, |engine| engine.account_permission_draft(&service, &account, preset).await)
    }

    pub async fn permission_scope_options(
        &self,
        service: String,
        account: String,
        capability: PermissionCapability,
        query: String,
        limit: u32,
    ) -> Result<PermissionScopeOptions, CoreError> {
        on_engine!(self, |engine| engine.permission_scope_options(&service, &account, capability, &query, limit))
    }

    pub async fn preview_account_permissions(
        &self,
        draft: AccountPermissionsDraft,
    ) -> Result<PermissionPreview, CoreError> {
        on_engine!(self, |engine| engine.preview_account_permissions(&draft))
    }

    pub async fn save_account_permissions(
        &self,
        draft: AccountPermissionsDraft,
        owner: Arc<dyn OwnerCheck>,
    ) -> Result<AccountPermissionsView, CoreError> {
        on_engine!(self, |engine| engine.save_account_permissions(&draft, owner.as_ref()).await)
    }

    pub async fn delegations(&self) -> Result<Vec<DelegationView>, CoreError> {
        on_engine!(self, |engine| engine.delegations())
    }

    pub async fn delegation_draft(&self, service: String, account: String) -> Result<DelegationDraft, CoreError> {
        on_engine!(self, |engine| engine.delegation_draft(&service, &account).await)
    }

    pub async fn preview_delegation(&self, draft: DelegationDraft) -> Result<PermissionPreview, CoreError> {
        on_engine!(self, |engine| engine.preview_delegation(&draft))
    }

    pub async fn issue_delegation(
        &self,
        draft: DelegationDraft,
        owner: Arc<dyn OwnerCheck>,
    ) -> Result<DelegationView, CoreError> {
        on_engine!(self, |engine| engine.issue_delegation(&draft, owner.as_ref()).await)
    }

    pub async fn change_delegation(
        &self,
        id: String,
        expected_revision: u64,
        change: DelegationChange,
        owner: Arc<dyn OwnerCheck>,
    ) -> Result<DelegationView, CoreError> {
        on_engine!(self, |engine| engine.change_delegation(&id, expected_revision, change, owner.as_ref()).await)
    }
}
