//! Records and enums crossing the UniFFI boundary (contracts §D).

#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct SessionInfo {
    pub server_url: String,
    pub email: String,
}

/// A sign-in through the server's SSO, started: open `url` in the browser session (ASWebAuthenticationSession, a
/// Custom Tab) and wait for `callback_scheme`; keep `state` and `verifier` for `sso_finish`.
#[derive(Clone, PartialEq, Eq, uniffi::Record)]
pub struct SsoStart {
    pub url: String,
    pub callback_scheme: String,
    pub state: String,
    pub verifier: String,
}

impl std::fmt::Debug for SsoStart {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SsoStart").field("url", &self.url).finish_non_exhaustive()
    }
}

/// Whether this phone can open the account's vault.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum AccountKeys {
    /// A new account: this phone just made its keys and keeps their secret.
    Created,
    /// This phone keeps what opens the keys.
    Unlocked,
    /// The account has keys this phone cannot open yet: approve it from the phone that has them, or enter the
    /// recovery code (or the master password of an account made with one).
    Locked,
}

#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct SsoOutcome {
    pub session: SessionInfo,
    pub keys: AccountKeys,
}

/// This phone asked the approval device for the account's keys: show `code` ("482 193") and ask the user to check
/// that the other phone shows the same.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct JoinStart {
    pub id: String,
    pub code: String,
    pub expires_at: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum JoinProgress {
    /// Not answered yet: ask again in a few seconds.
    Waiting,
    /// Approved: the account's keys are open on this phone.
    Joined,
    Denied,
    Expired,
}

/// Another phone asking the approval device for the account's keys (`PendingKind::Join`).
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct JoinView {
    pub id: String,
    pub device_name: String,
    /// The code the other phone shows, computed here from the key the server relayed.
    pub code: String,
    pub created_at: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum PendingKind {
    Request,
    Pairing,
    /// A file an AI uploaded with `reins_upload`, waiting for the user's decision (see [`BlobView`]).
    Blob,
    /// Another phone of the account asks for its keys ("Add another phone", see [`JoinView`]).
    Join,
}

#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct PendingItem {
    pub kind: PendingKind,
    pub id: String,
    /// Plain-language headline (used by notifications); the app builds its own from the fields below.
    pub title: String,
    pub subtitle: String,
    pub created_at: i64,
    pub connection_id: String,
    pub connection_label: String,
    /// "search" | "read" | "send" | "grant" | "pair"
    pub action: String,
    /// How many messages / recipients the operation covers (1 for grants and pairings).
    pub count: u32,
    /// The connector this is about, e.g. "gmail", and the account within it when known.
    pub service: String,
    pub account: Option<String>,
    /// Unix seconds until which the AI is still waiting; later approvals are "late" (see `approve`).
    pub wait_until: Option<i64>,
    /// The operation on a connector besides Gmail ("read", "send", ...), else empty.
    pub op: String,
    /// The operation in a few words ("Read Telegram messages"), else empty (Gmail, pairings).
    pub op_title: String,
    /// What Autopilot would do, in one line for the notification ("Autopilot would approve · 97%"); `None` when it
    /// did not judge (Manual mode, no model).
    pub suggestion: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum ApprovalKind {
    Search,
    Read,
    Send,
    /// The AI asks for a standing permission in advance.
    Grant,
    /// The AI asks to see which accounts an integration has.
    Accounts,
    /// The AI lists, reads or searches in another integration: the items found are ticked by the user.
    Fetch,
    /// The AI changes something in another integration (sends a message, adds an event): the user reads the preview.
    Write,
}

/// A thing an approval is about: a chat, a calendar, a repository.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct ResourceView {
    pub id: String,
    pub label: String,
    /// Not something the request touched but a wider thing it is part of (the repository of a branch, its owner): a
    /// standing permission can be given for it, and it is not offered by default.
    pub wider: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct MessageView {
    pub id: String,
    pub from: String,
    pub subject: String,
    pub date: i64,
    pub snippet: String,
    pub covered_by_grant: bool,
    /// Looks like something that must never be shared by default (a login code, a password).
    pub sensitive: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct EmailView {
    pub to: Vec<String>,
    pub cc: Vec<String>,
    pub subject: String,
    pub body: String,
}

#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct ApprovalView {
    pub request_id: String,
    pub connection_id: String,
    pub connection_label: String,
    pub kind: ApprovalKind,
    pub query: Option<String>,
    pub messages: Vec<MessageView>,
    pub email: Option<EmailView>,
    pub created_at: i64,
    pub service: String,
    pub account: Option<String>,
    pub wait_until: Option<i64>,
    pub grant: Option<GrantRequestView>,
    /// How many messages or recipients the request covers (what it asked for, not what was found).
    pub count: u32,
    /// Accounts request: every account of the integration.
    pub accounts: Vec<String>,
    /// Accounts request: those of `accounts` this AI was already allowed to see (shown ticked and locked).
    pub shared_accounts: Vec<String>,
    /// Another integration: the operation ("read", "send", ...), else empty.
    pub op: String,
    /// Another integration: the things (chats, calendars, ...) the request touches; a standing permission can cover them.
    pub resources: Vec<ResourceView>,
    /// A write: what will be done, one line each (the chat and the text, the event and its time).
    pub preview: Vec<String>,
    /// Nothing here may be remembered (a password, a destructive change): the permission is asked for every time.
    pub no_standing: bool,
    /// The operation in a few words, else empty.
    pub op_title: String,
    /// What kind of thing it is: "list" | "read" | "search" | "write" | "send" | "grant" | "accounts".
    pub action: String,
    /// A change to another integration: which kind of change this is (an id from `classes`), else empty.
    pub class: String,
    /// A change to another integration: the kinds of change a standing permission can allow.
    pub classes: Vec<ClassOption>,
    /// A push from git on the user's computer (through the Reins desktop app): what it changes, ref by ref.
    pub git: Option<GitPushView>,
    /// A write that uses a file the AI uploaded through the server: the file, as the server saw it.
    pub blob: Option<BlobView>,
    /// A call to a tool of an MCP server the user added.
    pub mcp: Option<crate::mcp::McpCallView>,
    /// A yes-or-no question from the desktop app (`reins ask`, a harness hook).
    pub ask: Option<AskView>,
    /// Vault secrets the desktop app asks for, for one command or API route (names only, never the values).
    pub secrets: Option<SecretReleaseView>,
    /// An SSH sign-in the desktop app's SSH agent asks the phone to sign.
    pub ssh: Option<SshSignView>,
}

/// A question from the desktop app; approving it answers yes.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct AskView {
    /// One line.
    pub question: String,
    /// What exactly would happen (the command, the files); may have several lines.
    pub detail: Option<String>,
    /// What a standing answer may cover (`command:git push --force`); `None`: a question without a topic.
    pub topic: Option<String>,
}

/// Secrets handed to the desktop app for one command or API route.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct SecretReleaseView {
    /// What will use them, as the desktop app said (`npm run deploy`, `API openai`).
    pub command: String,
    pub purpose: Option<String>,
    /// One per secret, in order: the item's name and the field ("GitHub · password").
    pub items: Vec<String>,
    /// How long the desktop app may keep them.
    pub lease_secs: u64,
}

/// One SSH sign-in, signed on the phone with a vault key.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct SshSignView {
    pub key_name: String,
    /// `SHA256:…`
    pub key_fingerprint: String,
    /// The server's name, when the desktop app knows it.
    pub host: Option<String>,
    /// The server's host key fingerprint, when known.
    pub host_key: Option<String>,
}

/// A push as the desktop app worked it out from the bytes git sent; the approval is bound to exactly those bytes.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct GitPushView {
    /// owner/name.
    pub repo: String,
    /// Size of the pack git sent.
    pub pack_bytes: u64,
    /// What the desktop app could not work out ("File list unavailable: ...").
    pub notes: Vec<String>,
    pub refs: Vec<GitRefView>,
}

/// One branch or tag a push changes.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct GitRefView {
    /// Full name: `refs/heads/main`, `refs/tags/v1.0`.
    pub name: String,
    /// "branch" | "tag" | "other"
    pub kind: String,
    /// `main`, `v1.0` (the full name for other refs).
    pub short_name: String,
    /// "create" | "update" | "delete"
    pub change: String,
    /// An update that does not only add commits: it rewrites history, or that could not be checked.
    pub force: bool,
    /// The desktop app could not tell whether the update only adds commits (treated as a force push).
    pub force_unknown: bool,
    /// Commits the ref gains; may be more than `commits` lists.
    pub commit_count: u32,
    /// Newest first.
    pub commits: Vec<GitCommitView>,
    /// Files that differ; may be more than `files` lists.
    pub files_changed: u32,
    pub files: Vec<GitFileView>,
    /// Lines added and removed in all files; `None` when not every file could be counted.
    pub additions: Option<u64>,
    pub deletions: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct GitCommitView {
    /// The first 7 hex digits.
    pub short_sha: String,
    pub subject: String,
    /// "Name <email>".
    pub author: String,
}

#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct GitFileView {
    pub path: String,
    /// "added" | "modified" | "deleted" | "type_changed"
    pub status: String,
    /// `None` when not counted (binary, too large).
    pub additions: Option<u32>,
    pub deletions: Option<u32>,
    pub binary: bool,
}

/// One kind of change a standing permission can allow (issues, code, releases, ...).
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct ClassOption {
    pub id: String,
    pub label: String,
}

/// A permission an AI asks for, spelled out for the user.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct GrantRequestView {
    /// "read" | "send"
    pub action: String,
    pub summary: String,
    pub reason: String,
    pub duration_secs: u64,
    pub max_uses: Option<u32>,
    /// "narrow" | "broad" | "everything": how much mail or how many recipients it opens up.
    pub breadth: String,
    /// One line per limit, e.g. "From @bank.com".
    pub lines: Vec<String>,
}

/// What the user approved. For Search/Read: `selected_message_ids` ⊆ the view's
/// messages (covered ones are always included). For Send: ignored.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct ApprovalChoice {
    pub selected_message_ids: Vec<String>,
    pub standing: Option<StandingGrant>,
}

/// Optional standing grant created alongside the approval.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct StandingGrant {
    pub duration_secs: Option<u64>,
    pub max_uses: Option<u32>,
    pub scope: GrantScopeChoice,
}

/// Read grants: any combination of the read fields (at least one). Send grants: recipients.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct GrantScopeChoice {
    /// read: every message, any search or read, for a limited time (needs `duration_secs`, at most 30 days);
    /// cannot be combined with the other read fields
    pub all_mail: bool,
    /// read: `message_ids` = the selected ids
    pub selected_messages_only: bool,
    /// read: `from` Exact
    pub sender_addresses: Vec<String>,
    /// read: `from` Domain
    pub sender_domains: Vec<String>,
    /// read/send: subject contains this literal text (`Pattern::literal`)
    pub subject_pattern: Option<String>,
    /// send: recipients Exact
    pub recipient_addresses: Vec<String>,
    /// send: recipients Domain
    pub recipient_domains: Vec<String>,
    /// Another integration: the ids of the things (chats, calendars, ...) the permission covers. With `all_mail` it
    /// covers every one of them, for a limited time.
    pub resources: Vec<String>,
    /// Another integration, a permission to change things: the kinds of change it allows (see `ApprovalView.classes`).
    /// Empty = the kind of the request itself.
    pub classes: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct PairingView {
    pub id: String,
    pub client_name: String,
    pub client_host: String,
    pub choices: Vec<u8>,
    pub created_at: i64,
    /// The Reins desktop app asks to pair: the eight digits ("4821 9930") of its key, which the computer shows
    /// too. `None` for an AI client (or a key that cannot be used).
    pub key_fingerprint: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct GrantView {
    pub id: String,
    pub connection_id: String,
    pub connection_label: String,
    /// "read" | "send"
    pub action: String,
    pub summary: String,
    pub expires_at: Option<i64>,
    pub max_uses: Option<u32>,
    pub uses: u32,
    pub created_at: i64,
    pub last_used_at: Option<i64>,
    /// "approval" | "ai_request" | "user" | "retry"
    pub origin: String,
    pub service: String,
    pub account: Option<String>,
    /// One line per limit, e.g. "Senders: @bank.com".
    pub lines: Vec<String>,
    /// Not expired, not used up, not deleted.
    pub active: bool,
    /// "active" | "expired" | "used_up" | "revoked": why a grant that is not active has ended.
    pub state: String,
    /// Covers every email: it can only be resumed for a week at most.
    pub all_mail: bool,
    /// The scope in the form the grant editor uses, when the grant is simple enough to be edited (senders, domains,
    /// a subject text, or recipients); `None` for grants tied to specific emails or to other kinds of limits.
    pub editable_scope: Option<GrantScopeChoice>,
}

/// An account the user connected to a service.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct AccountView {
    /// "gmail"
    pub service: String,
    pub account: String,
    pub added_at: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct ConnectionView {
    pub id: String,
    pub label: String,
    pub client_host: String,
    pub created_at: i64,
    pub last_used_at: Option<i64>,
    /// The icon the user picked (`None` = derived from the name by the app).
    pub icon: Option<String>,
}

#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct ActivityEntry {
    /// Stable, increasing row number.
    pub id: i64,
    pub at: i64,
    pub connection_id: String,
    pub connection_label: String,
    /// "search" | "read" | "send" | "grant" | "pair" | "request"
    pub action: String,
    /// "released" | "sent" | "granted" | "denied" | "error"
    pub outcome: String,
    pub detail: String,
    pub grant_id: Option<String>,
    pub service: String,
    pub account: Option<String>,
    pub count: u32,
    pub info: ActivityInfo,
    /// The operation on a connector besides Gmail ("read", "send", ...), else empty.
    pub op: String,
    /// The operation in a few words ("Read Telegram messages"), else empty.
    pub op_title: String,
    /// Who decided: "" (the user, or a grant), "autopilot", "bypass" or "lockdown".
    pub decided_by: String,
    /// What Autopilot knew (the "Automatic" filter and badge, "This was wrong").
    pub autopilot: Option<crate::autopilot::AutopilotNote>,
}

/// One message of a released search or read (headers only).
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct ActivityMessage {
    /// The Gmail message id, so the email can be opened; empty for entries logged before ids were kept.
    pub id: String,
    /// Another integration: what was shared of this item (the message text, the event details); empty for Gmail.
    pub text: String,
    pub from: String,
    pub subject: String,
    pub date: i64,
}

/// Everything the activity screen can show for one entry.
#[derive(Clone, Debug, Default, PartialEq, Eq, uniffi::Record)]
pub struct ActivityInfo {
    pub query: Option<String>,
    pub messages: Vec<ActivityMessage>,
    pub email: Option<EmailView>,
    pub note: Option<String>,
    pub grant_summary: Option<String>,
    /// Accounts request: exactly the addresses that were shown to the AI.
    pub accounts: Vec<String>,
}

/// One email opened from the activity, fetched from Gmail when asked for (nothing of it is kept on the phone).
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct EmailContent {
    pub id: String,
    pub from: String,
    pub to: Vec<String>,
    pub cc: Vec<String>,
    pub subject: String,
    pub date: i64,
    /// Plain text, made safe to show.
    pub body: String,
}

#[derive(Clone, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum GmailStatus {
    Ready,
    NeedsConsent,
    Unavailable {
        message: String,
    },
}

/// One integration as the "Integrations" screen shows it.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct ServiceView {
    /// "gmail", "telegram", "gcalendar", ...
    pub service: String,
    pub name: String,
    /// How an account is added: "google" (sign in with Google), "telegram" (phone number and code), "token" (paste
    /// an access token), "device" (allow access on this phone), "vault" (the vault this phone is signed in to).
    pub kind: String,
    /// Can be used in this build; false when something it needs is missing (see `note`).
    pub available: bool,
    pub note: Option<String>,
    pub accounts: Vec<AccountView>,
}

/// A file held by the server for one operation, as the user sees it before deciding: an upload waiting for approval
/// (`PendingKind::Blob`), or the file a write uses (`ApprovalView.blob`). Everything here was worked out by the server
/// from the bytes, not taken from the AI, except `name` and `purpose`.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct BlobView {
    pub id: String,
    pub connection_label: String,
    /// The file name the AI gave (shown only, never used as a path).
    pub name: String,
    /// Bytes.
    pub size: u64,
    /// Sniffed from the bytes ("application/pdf", "text/plain", "application/octet-stream").
    pub content_type: String,
    /// Lowercase hex.
    pub sha256: String,
    /// What it is for, in words: the reason the AI gave for an upload, or the tool that will use it.
    pub purpose: String,
    /// The start of a text file, or what kind of file it looks like ("PDF document").
    pub preview_text: Option<String>,
    /// An image file (PNG, JPEG, GIF, WebP) small enough to be shown whole.
    pub preview_image: Option<Vec<u8>>,
    pub created_at: i64,
    /// Unix seconds; the server deletes the file then.
    pub expires_at: i64,
}
