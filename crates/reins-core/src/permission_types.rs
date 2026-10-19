//! Typed native permissions contract; implemented by the core, with one policy representation underneath.

#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum PermissionMode {
    Ask,
    Auto,
    Block,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum AccountPermissionPreset {
    ReviewEveryRequest,
    ReadAssist,
    Custom,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum PermissionCapability {
    MailSearch,
    MailRead,
    MailSend,
    IntegrationRead,
    IntegrationWrite,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum PermissionScopeKind {
    AllInAccount,
    SelectedResources,
    SelectedRecipients,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum PermissionController {
    Owner,
    Organization,
    UnsupportedAdapter,
    Conflict,
}

#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct PermissionRestriction {
    pub controller: PermissionController,
    pub policy_id: Option<String>,
    pub reason: String,
}

#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct PermissionScopeSelection {
    pub kind: PermissionScopeKind,
    pub ids: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct CapabilityPermissionInput {
    pub capability: PermissionCapability,
    pub mode: PermissionMode,
    pub scope: PermissionScopeSelection,
}

#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct PermissionCapabilityView {
    pub capability: PermissionCapability,
    pub title: String,
    pub description: String,
    pub supported_modes: Vec<PermissionMode>,
    pub supported_scopes: Vec<PermissionScopeKind>,
    pub configured_mode: Option<PermissionMode>,
    pub scope: PermissionScopeSelection,
    pub scope_summary: String,
    pub effective_reason: String,
    pub restrictions: Vec<PermissionRestriction>,
}

#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct PermissionAgentView {
    pub connection_id: String,
    pub label: String,
    pub has_account_access: bool,
    pub selected: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct AccountPermissionsView {
    pub service: String,
    pub account: String,
    pub account_id: String,
    pub revision: u64,
    pub managed: bool,
    pub conflicted: bool,
    pub capabilities: Vec<PermissionCapabilityView>,
    pub agents: Vec<PermissionAgentView>,
    pub available_presets: Vec<AccountPermissionPreset>,
    pub expires_at: Option<i64>,
    pub maximum_duration_secs: u64,
    pub delegation_maximum_duration_secs: u64,
    pub used: u64,
    pub usage_verified: bool,
    pub remaining_uses: Option<u64>,
    pub scope_summary: String,
    pub restrictions: Vec<PermissionRestriction>,
}

#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct AccountPermissionsDraft {
    pub service: String,
    pub account: String,
    pub expected_revision: u64,
    pub preset: AccountPermissionPreset,
    pub agent_ids: Vec<String>,
    /// Explicit owner admission of selected agents; never applies to newly paired agents automatically.
    pub grant_account_access: bool,
    pub expires_at: Option<i64>,
    pub max_uses: Option<u64>,
    pub rules: Vec<CapabilityPermissionInput>,
}

#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct PermissionPreview {
    pub revision: u64,
    pub summary: String,
    pub warnings: Vec<String>,
    /// Counts only observed retained requests; no count means the required historical facts were unavailable.
    pub eligible_history_count: Option<u32>,
    pub review_history_count: Option<u32>,
    pub blocked_history_count: Option<u32>,
    pub history_complete: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct PermissionScopeOption {
    pub id: String,
    pub label: String,
    pub detail: String,
    pub service: String,
    pub account: String,
    pub kind: PermissionScopeKind,
}

#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct PermissionScopeOptions {
    pub kinds: Vec<PermissionScopeKind>,
    pub options: Vec<PermissionScopeOption>,
    /// Typed address entry is canonicalized/validated by the core, never parsed as policy code or a domain glob.
    pub allows_address_entry: bool,
    pub unsupported_reason: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum DelegationState {
    Active,
    Paused,
    Expired,
    Revoked,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum DelegationChange {
    Resume,
    Pause,
    Revoke,
}

#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct DelegationDraft {
    pub service: String,
    pub account: String,
    pub expected_revision: u64,
    pub agent_ids: Vec<String>,
    pub grant_account_access: bool,
    pub rules: Vec<CapabilityPermissionInput>,
    pub expires_at: i64,
    pub max_uses: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct DelegationView {
    pub id: String,
    /// Aggregate CAS revision: any concurrent policy edit makes this view stale.
    pub revision: u64,
    pub title: String,
    pub service: String,
    pub account: String,
    pub state: DelegationState,
    pub expires_at: i64,
    pub agent_ids: Vec<String>,
    pub rules: Vec<CapabilityPermissionInput>,
    /// Includes attempted operations with an unresolved external outcome; these are not silently refunded.
    pub used: u64,
    pub usage_verified: bool,
    pub unresolved: u64,
    pub max_uses: u64,
    pub scope_summary: String,
    pub effective_reason: String,
    pub restrictions: Vec<PermissionRestriction>,
}
