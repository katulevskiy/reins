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

/// What a server offers, read before signing in (`/api/config`). `None`: the server does not say (an older one).
#[derive(Clone, Debug, Default, PartialEq, Eq, uniffi::Record)]
pub struct ServerInfo {
    /// "Continue" (sign-in in the browser, SSO) works here; when false, only email and master password do.
    pub browser_sign_in: Option<bool>,
    /// The server can wake the Android app; when false, requests show up only while the app is open.
    pub push_android: Option<bool>,
    /// The server can wake the iOS app.
    pub push_ios: Option<bool>,
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
    /// did not judge (no model).
    pub suggestion: Option<String>,
    /// What approving does, in one sentence (see [`ApprovalView::headline`]); empty for pairings and other phones.
    #[uniffi(default)]
    pub headline: String,
    /// Can be approved without opening it, from the notification or with "Approve all" (`approve_quick`): it is not
    /// asked every time and nothing in it needs a closer look.
    #[uniffi(default)]
    pub quick: bool,
    /// What the request is, as every screen shows it (see [`RequestLabel`]); `None` for pairings and other phones.
    #[uniffi(default)]
    pub label: Option<RequestLabel>,
}

/// What a request is, the same on every screen: the thing it touches (the tile), what it does to it (the badge on the
/// tile's corner) and the request itself in a few words, "Run `git push --force origin main`". Built by the core from
/// the call (`crate::titles`), so a waiting card, its Activity row and the entry's page agree.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize, uniffi::Record)]
pub struct RequestLabel {
    /// What it touches: "terminal" (a command), "file", "secrets" (the vault's secrets for a command), "ssh", "desktop"
    /// (a question from the computer), "mcp:<server id>" (a tool of an MCP server), else the integration's id ("github",
    /// "gmail", "gcalendar", "telegram", ...).
    pub provider: String,
    /// What it does, the badge: "run", "read", "write", "send", "delete", "push", "secrets", "search", "grant" or "ask".
    pub action: String,
    /// The words before the subject ("Run", "Push 3 commits to", "Send an email to"); the whole title when there is no
    /// subject.
    pub lead: String,
    /// What it is about, set apart: the command, the path, the repository, the address. May be empty.
    pub subject: String,
    /// The subject is code (a command, a path, a tool's name): shown in a monospaced font.
    pub code: bool,
    /// After the subject (" and 2 more"); usually empty.
    #[serde(default)]
    pub tail: String,
}

impl RequestLabel {
    /// The title as one plain line.
    #[must_use]
    pub fn title(&self) -> String {
        let mut s = self.lead.clone();
        if !self.subject.is_empty() {
            if !s.is_empty() {
                s.push(' ');
            }
            s.push_str(&self.subject);
        }
        s.push_str(&self.tail);
        s
    }
}

/// What an Activity entry asked, in full (the entry's "What it asked" and "Full request"). Never a secret's value.
#[derive(Clone, Debug, Default, PartialEq, Eq, uniffi::Record)]
pub struct ActivityRequest {
    /// The agent as it named itself ("Claude Code"); empty when only the connection is known.
    pub agent: String,
    /// The folder it ran in, else empty.
    pub cwd: String,
    /// Why it was asked rather than let through: the rule it matched or the reason, else empty.
    pub why: String,
    /// The complete command or tool call with its arguments, several lines (at most 8 KB).
    pub full: String,
    /// What the agent said it was for, else empty.
    pub purpose: String,
    /// When it was asked (Unix seconds); 0 when not known.
    pub asked_at: i64,
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
    /// What approving does, in one sentence ("Claude gets the 3 emails found for \"from:bank\"."). It may quote
    /// what the AI sent or a service returned.
    #[uniffi(default)]
    pub headline: String,
    /// One-tap answers; `None` for what is asked every time (the hard floor).
    #[uniffi(default)]
    pub quick: Option<QuickApproval>,
    /// A purchase: the cart like a receipt, and what may pay for it (approve it with `approve_purchase`).
    #[uniffi(default)]
    pub purchase: Option<crate::connector::payments::PurchaseView>,
    /// Reins Secrets: the variables of a project's environment for one command on the computer (names only).
    #[uniffi(default)]
    pub env_release: Option<EnvReleaseView>,
    /// Reins Secrets: variables the computer sends to be saved, environment by environment (names only).
    #[uniffi(default)]
    pub secrets_store: Option<SecretsStoreView>,
    /// A git host's token the computer hands to the phone's integration of that host.
    #[uniffi(default)]
    pub token_store: Option<TokenStoreView>,
}

/// One command a permission for secrets can cover, as the approval offers it.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, uniffi::Record)]
pub struct CommandOption {
    /// What `SecretsScopeChoice.command` names: `exact`, or `prefix:<n>` (the program and its first n arguments, then
    /// any whole arguments).
    pub id: String,
    /// `npm run dev`, `npm run *`.
    pub label: String,
    pub prefix: bool,
}

/// The variables of one environment of a project, for one command on the computer (`reins run`).
#[allow(clippy::struct_excessive_bools, reason = "a release is described by this many independent facts")]
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, uniffi::Record)]
pub struct EnvReleaseView {
    pub project: String,
    pub env: String,
    /// `person`, or `agent:<harness>`.
    pub runner: String,
    /// "You", or the agent's name ("Claude Code").
    pub runner_label: String,
    pub agent: bool,
    /// The project's folder on the computer.
    pub root: String,
    /// The command as it runs (program name and arguments).
    pub command: String,
    /// The program's full path, as the computer resolved it.
    pub program: String,
    /// Why the program deserves a second look, for a warning row ("In a temporary folder", "Inside the project
    /// folder", "Not the one on your PATH"); `command` then shows its whole path too.
    #[uniffi(default)]
    pub program_warning: Option<String>,
    /// The variables' names (never a value), sorted.
    pub variables: Vec<String>,
    /// Every variable of the environment was asked for.
    pub all_variables: bool,
    pub lease_secs: u64,
    pub purpose: Option<String>,
    /// What a remembered approval can cover, narrowest first (the exact command first).
    pub commands: Vec<CommandOption>,
    /// The value is printed in the terminal (`reins secrets get`): asked every time.
    pub print: bool,
    /// The environment is asked every time (`confirm = "always"` in reins.toml, a production environment, or your
    /// choice); `always_ask_set` says you chose it on the phone.
    pub always_ask: bool,
    pub always_ask_set: bool,
}

/// One environment of a `vault_secrets_store`.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, uniffi::Record)]
pub struct EnvStoreView {
    pub project: String,
    pub env: String,
    /// The environment does not exist yet and is made.
    pub new_env: bool,
    pub added: Vec<String>,
    pub replaced: Vec<String>,
    pub removed: Vec<String>,
    /// "OLD → NEW".
    pub renamed: Vec<String>,
    /// Variables copied in from another environment of the project, and from which.
    pub copied_from: Option<String>,
    pub copied: u32,
}

/// Variables the computer sends to be saved in the vault (`reins secrets set/push`, import): one approval.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, uniffi::Record)]
pub struct SecretsStoreView {
    pub envs: Vec<EnvStoreView>,
    /// Four digits the terminal shows too.
    pub check: String,
    /// Values in all (variables set, SSH keys, tokens).
    pub total: u32,
    /// SSH keys kept in the folder Reins/ssh.
    #[uniffi(default)]
    pub ssh: Vec<SshStoreView>,
    /// Git hosts' tokens handed to the phone's integrations (`check` empty: the batch has one).
    #[uniffi(default)]
    pub tokens: Vec<TokenStoreView>,
}

/// An SSH key of a store.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, uniffi::Record)]
pub struct SshStoreView {
    pub name: String,
    /// `SHA256:…`
    pub fingerprint: String,
}

/// A git host's token the computer hands to the phone's integration.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, uniffi::Record)]
pub struct TokenStoreView {
    /// `github`, `gitlab`, `codeberg`, `bitbucket`.
    pub service: String,
    pub host: String,
    /// As the computer reported them (not checked).
    pub scopes: Vec<String>,
    /// Four digits of the token, which the terminal shows too.
    pub check: String,
}

/// What "Revoke on all computers" did: the permissions it ended on this phone, the computers told, and those not told
/// yet (by name). A note that could not be delivered is kept and sent again on each sync until the server takes it;
/// the permissions ended on this phone either way.
#[derive(Clone, Debug, Default, PartialEq, Eq, uniffi::Record)]
pub struct SecretsRevoked {
    pub ended: u32,
    pub told: u32,
    /// "Not yet delivered to <name>" for each of these.
    pub not_delivered: Vec<String>,
}

/// A standing permission for secrets (`ApprovalView.env_release`): which of the offered commands it covers, and
/// whether every variable of the environment or only those asked for.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct SecretsScopeChoice {
    /// An id of `EnvReleaseView.commands`.
    pub command: String,
    pub all_variables: bool,
}

/// What a newly connected AI may do before it asked for anything (see `crate::starter`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum StartingPolicy {
    /// Everything it asks waits for the user. The default.
    AskEveryTime,
    /// For its first hour it may see the headlines of what its searches and lists find (Gmail: sender, subject and
    /// date; elsewhere: names and titles), as grants under Grants. Opening anything, sending, changing, the vault,
    /// codes and passwords, and the hard floor still ask.
    HeadlinesFirstHour,
}

/// One-tap answers to a request that is not asked every time (see `crate::quick`).
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct QuickApproval {
    /// Nothing needs a closer look or a choice, so it can be approved from the notification or with "Approve all"
    /// (`approve_quick`). False when something found looks like a code or a password.
    pub from_notification: bool,
    /// What "Approve and allow for a while" adds to the approval: a permission for this connection, this kind of
    /// request and this target (the account, the chats, the repository, the tool, the question's topic), for
    /// `duration_secs`. `None` when the request cannot be remembered.
    pub allow: Option<StandingGrant>,
    /// What `allow` covers, in a few words ("searching and reading me@gmail.com"); empty without `allow`.
    pub allow_what: String,
    /// How often the user approved the same thing from this connection in the last 24 hours.
    pub repeats: u32,
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
    /// The guard rule the question matches, as the phone reads its command or files (else as the computer reports
    /// it); `None` when there is none.
    #[uniffi(default)]
    pub rule: Option<AskRuleView>,
    /// The phone could not read the question with certainty (an unbalanced quote, a detail cut short): Autopilot
    /// leaves it to the user in Manual and Auto.
    #[uniffi(default)]
    pub uncertain: bool,
    /// It cannot be undone: held to approve and shown in red. (A question asked every time for another reason, a
    /// private key or an unreadable command, is an ordinary yes or no that is never remembered.)
    #[uniffi(default)]
    pub destructive: bool,
    /// The guard group everything the command (or the file list) does falls in, which Autopilot learns on its own;
    /// `None`: the generic kind.
    #[uniffi(default)]
    pub kind: Option<String>,
}

/// The guard rule a desktop question matches.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct AskRuleView {
    /// What the rule is about: its group's label (".env files", "Rewriting git history"), or "Your rule".
    pub label: String,
    /// The rule itself (`.env`, `git push --force*`).
    pub pattern: String,
    /// What in the command matched it (`app/.env`, `git push --force origin main`).
    pub matched: String,
    /// The built-in group (`env-files`, `git-history`); `None` for the user's own rules.
    pub group: Option<String>,
    /// What it covers cannot be undone: asked every time, held to approve, and never judged by Autopilot in Manual or
    /// Auto.
    pub destructive: bool,
    /// Only the computer says so: the phone did not find this rule in the command itself (the user's own rule, or a
    /// group this phone does not know). Shown as reported; it can make a question stricter, never looser.
    pub reported: bool,
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
    /// Author time, unix seconds (0 when not known).
    #[uniffi(default)]
    pub date: i64,
    /// What the commit itself changes (against its first parent): how many files, and the first of them with their
    /// lines; `None` when the desktop app did not compare them.
    #[uniffi(default)]
    pub files_changed: Option<u32>,
    #[uniffi(default)]
    pub files: Vec<GitFileView>,
    /// Lines added and removed in all its files; `None` when not every file could be counted.
    #[uniffi(default)]
    pub additions: Option<u64>,
    #[uniffi(default)]
    pub deletions: Option<u64>,
    /// The subject, the author or one of its files holds characters that change how text reads without being seen
    /// (bidi controls, zero-width characters): what is shown may not be what the code does. Show a warning.
    #[uniffi(default)]
    pub hidden_chars: bool,
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
    /// A commit's file: the change line by line, as the desktop app sent it (empty when not shown: binary, too large,
    /// or over the push's budget). Text from the user's repository: show it as untrusted.
    #[uniffi(default)]
    pub diff: Vec<DiffLineView>,
    /// `diff` stops before the end of the change.
    #[uniffi(default)]
    pub diff_cut: bool,
    /// The path or a line of `diff` held invisible characters (bidi controls, zero-width characters); in `diff` each
    /// is shown as U+FFFD. Show a warning.
    #[uniffi(default)]
    pub hidden_chars: bool,
}

/// One line of a file's change.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum DiffLineKind {
    /// `@@ -12,9 +12,22 @@ …`: where the next lines are.
    Hunk,
    Context,
    Add,
    Delete,
    /// "No newline at end of file".
    Note,
}

#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct DiffLineView {
    pub kind: DiffLineKind,
    /// The line's number in the old file (context and deleted lines) and in the new one (context and added lines).
    pub old_line: Option<u32>,
    pub new_line: Option<u32>,
    /// The line without its sign (the whole header for a hunk), tabs as four spaces, control and invisible characters
    /// (bidi controls, zero-width characters) as U+FFFD so they can be seen.
    pub text: String,
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
    /// Reins Secrets: the command and variables a permission for an environment covers. `None`: the exact command and
    /// the variables asked for.
    #[uniffi(default)]
    pub secrets: Option<SecretsScopeChoice>,
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
    /// "read" | "send" | "accounts" | "list" | "write" | "headlines" (what searches and lists find, metadata only)
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
    /// A computer (the Reins desktop app): the eight digits of the key pinned when it paired. `None` for an AI app.
    pub key_fingerprint: Option<String>,
}

/// What a device signed in to the account is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum DeviceKind {
    /// A phone or tablet (the Reins app, or a Bitwarden app on it).
    Phone,
    /// A Bitwarden desktop app or command line.
    Computer,
    /// The web vault or a browser extension.
    Browser,
    Other,
}

/// One device signed in to the account (Settings > Devices).
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct DeviceView {
    pub id: String,
    pub name: String,
    pub kind: DeviceKind,
    /// "Android", "iOS", "Chrome extension", "Web vault", ...
    pub platform: String,
    pub created_at: i64,
    /// When it last signed in or renewed its sign-in.
    pub last_seen_at: i64,
    /// The account's approval device: the one requests go to.
    pub approval: bool,
    pub this_device: bool,
}

impl DeviceView {
    /// Bitwarden's device types (`DeviceType` in its server).
    pub(crate) fn kind_of(code: i32) -> (DeviceKind, &'static str) {
        match code {
            0 | 15 => (DeviceKind::Phone, "Android"),
            1 => (DeviceKind::Phone, "iOS"),
            2 => (DeviceKind::Browser, "Chrome extension"),
            3 => (DeviceKind::Browser, "Firefox extension"),
            4 => (DeviceKind::Browser, "Opera extension"),
            5 => (DeviceKind::Browser, "Edge extension"),
            19 => (DeviceKind::Browser, "Vivaldi extension"),
            20 => (DeviceKind::Browser, "Safari extension"),
            6 | 16 => (DeviceKind::Computer, "Bitwarden for Windows"),
            7 => (DeviceKind::Computer, "Bitwarden for macOS"),
            8 => (DeviceKind::Computer, "Bitwarden for Linux"),
            23..=25 => (DeviceKind::Computer, "Bitwarden command line"),
            9..=14 | 17 | 18 => (DeviceKind::Browser, "Web vault"),
            _ => (DeviceKind::Other, "Other"),
        }
    }
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
    /// What it was, as its row and its page show it.
    #[uniffi(default)]
    pub label: Option<RequestLabel>,
    /// What it asked, in full; `None` for entries logged before it was kept (and for pairings, grants that ended, ...).
    #[uniffi(default)]
    pub request: Option<ActivityRequest>,
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
