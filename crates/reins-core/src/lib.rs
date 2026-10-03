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
pub mod api;
pub mod approval;
pub mod autopilot;
pub mod blob;
pub mod connector;
pub mod crypto;
pub mod engine;
pub mod error;
pub mod gmail;
pub mod google;
pub mod handler;
pub mod http;
pub mod join;
pub mod mcp;
pub mod phone_api;
pub mod rt;
pub mod services;
pub mod session;
pub mod sso;
pub mod store;
pub mod text;
pub mod traits;
pub mod types;
pub mod vault;
pub mod views;

pub use api::ReinsCore;
pub use autopilot::{
    AutoDecisionView, AutopilotEvent, AutopilotMode, AutopilotNote, AutopilotSettings, ClassView, ConnectionAutopilot,
    DownloadProgress, ModelInput, ModelOutput, ModelRuntime, ModelState, ModelStatus, NeighbourView, Preset,
    ProfileView, SuggestionView, Verdict,
};
pub use connector::LoginProgress;
pub use connector::device::{DeviceBridge, DeviceContact, DeviceEvent, NewDeviceEvent, SmsMessage, SmsThread};
pub use engine::{CoreConfig, Engine};
pub use error::{CoreError, ForeignError};
pub use mcp::{McpAddStep, McpCallView, McpServerView, McpToolView};
pub use traits::{GoogleTokenProvider, KeyWrapper, Notifier};
pub use types::{AccountKeys, JoinProgress, JoinStart, JoinView, SsoOutcome, SsoStart};
pub use types::{
    AccountView, ActivityEntry, ActivityInfo, ActivityMessage, ApprovalChoice, ApprovalKind, ApprovalView, BlobView,
    ConnectionView, EmailContent, EmailView, GitCommitView, GitFileView, GitPushView, GitRefView, GmailStatus,
    GrantRequestView, GrantScopeChoice, GrantView, MessageView, PairingView, PendingItem, PendingKind, ResourceView,
    ServiceView, SessionInfo, StandingGrant,
};
pub use types::{AskView, SecretReleaseView, SshSignView};
