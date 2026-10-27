//! Reins phone core: Vaultwarden login, phone API client, encrypted local
//! store, Gmail connector and the request/approval engine, exported to Kotlin
//! through UniFFI (contracts §D).
//!
//! Internal modules are public so integration tests can drive them directly.
//! `must_use_candidate` is allowed crate-wide because those modules are an
//! internal API, not a published library surface.
#![allow(clippy::must_use_candidate)]

uniffi::setup_scaffolding!();

pub mod account;
mod account_runtime;
mod account_sync;
pub mod api;
pub mod approval;
pub mod authorization;
pub mod authorization_trace;
pub mod autopilot;
pub mod avatar;
pub mod blob;
pub mod circles;
pub mod command_presentation;
pub mod connector;
pub mod crypto;
pub mod engine;
pub mod error;
pub mod gmail;
pub mod google;
pub mod handler;
pub mod http;
pub mod join;
pub mod live_sync;
pub mod mcp;
pub mod names;
pub mod org;
pub mod permission_types;
pub mod permissions;
pub mod phone_api;
pub mod profiles;
pub mod quick;
pub mod roles;
pub mod rt;
mod secrets;
pub mod services;
pub mod session;
pub mod sso;
pub mod starter;
pub mod store;
pub mod text;
pub mod tickets;
pub mod titles;
pub mod traits;
pub mod types;
pub mod vault;
pub mod vault_editor;
pub mod vault_passkey;
pub mod views;
pub mod work_session;

pub use api::ReinsCore;
pub use autopilot::{
    AutoDecisionView, AutopilotEvent, AutopilotMode, AutopilotNote, AutopilotSettings, AutopilotStats, ClassView,
    ConnectionAutopilot, DigestGroup, DownloadProgress, ModelInput, ModelOutput, ModelRuntime, ModelState, ModelStatus,
    ModelUpdateOutcome, NeighbourView, OVERNIGHT_NAME, OvernightDigest, Preset, ProfileView, RiskLevel, RiskView,
    SuggestionView, TaskView, ThresholdReplay, Verdict,
};
pub use circles::{
    CircleDraft, CircleEvent, CircleInviteProgress, CircleInviteStart, CircleInviteView, CircleJoinStage,
    CircleLinkState, CircleNoticeView, CirclePerson, CircleProgress, CircleReviewState, CircleReviewView, CircleRole,
    CircleScopePreset,
};
pub use connector::LoginProgress;
pub use connector::device::{DeviceBridge, DeviceContact, DeviceEvent, NewDeviceEvent, SmsMessage, SmsThread};
pub use engine::{CoreConfig, Engine};
pub use error::{CoreError, ForeignError};
pub use mcp::{McpAddStep, McpCallView, McpServerView, McpToolView};
pub use roles::{
    EnrollProgress, EnrollStart, EnrollView, HandoverProgress, HandoverView, PhoneRole, RecoveryPreview,
    RosterMemberView, RosterSummary,
};
pub use traits::{GoogleTokenProvider, KeyWrapper, Notifier, OwnerCheck, StoreLockGuard};
pub use types::{AccountKeys, JoinProgress, JoinStart, JoinView, ServerInfo, SsoOutcome, SsoStart};
pub use types::{
    AccountView, ActivityEntry, ActivityInfo, ActivityMessage, ActivityPush, ActivityRequest, ApprovalChoice,
    ApprovalKind, ApprovalView, Attention, AttentionLevel, AttentionReason, BlobView, ConnectionView, DeviceKind,
    DeviceView, DiffLineKind, DiffLineView, EmailContent, EmailView, GitCommitView, GitFileView, GitPushView,
    GitRefView, GmailStatus, GrantRequestView, GrantScopeChoice, GrantView, MessageView, PairingView, PendingItem,
    PendingKind, QuickApproval, ReasonCode, ResourceView, ServiceView, SessionInfo, StandingGrant, StartingPolicy,
};
pub use types::{
    ActivityConnection, ActivityCursor, ActivityDecider, ActivityFacets, ActivityField, ActivityHighlight, ActivityHit,
    ActivityOutcome, ActivityPage, ActivityQuery, ActivitySnippet, ActivityStamp,
};
pub use types::{
    AskRuleView, AskView, CommandOption, EnvReleaseView, EnvStoreView, SecretReleaseView, SecretsRevoked,
    SecretsScopeChoice, SecretsStoreView, SshSignView, SshStoreView, TokenStoreView,
};
