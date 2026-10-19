//! Typed permissions facade. Authentication generation and owner confirmation match all other native core APIs.

use std::sync::Arc;

use super::ReinsCore;
use crate::permission_types::{
    AccountPermissionPreset, AccountPermissionsDraft, AccountPermissionsView, DelegationChange, DelegationDraft,
    DelegationView, PermissionCapability, PermissionPreview, PermissionScopeOptions,
};
use crate::{CoreError, OwnerCheck, rt};

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
