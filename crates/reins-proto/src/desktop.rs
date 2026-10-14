//! The Reins desktop app and the phone: which git access the app asks for, what a push does (built by the app from
//! the exact bytes git sends, shown on the phone), and the credential the phone seals to the app.
//!
//! Trust: the app's public key is pinned on the phone when the app is paired (the user compares [`key_fingerprint`] on
//! both screens). The phone seals every credential to that key, so the server relaying it never sees the token. The
//! sealed [`CredentialGrant`] echoes the app's `nonce` and, for a push, the [`push_digest`], so an old answer cannot
//! be replayed for another request.

use data_encoding::BASE64URL_NOPAD;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const GIT_FETCH_TOOL: &str = "github_git_fetch";
pub const GIT_PUSH_TOOL: &str = "github_git_push";
pub const GIT_TAG_PUSH_TOOL: &str = "github_git_tag_push";
pub const GIT_FETCH_OP: &str = "git_fetch";
pub const GIT_PUSH_OP: &str = "git_push";
pub const GIT_TAG_PUSH_OP: &str = "git_tag_push";

/// The read tool of `service` (`github_git_fetch`, `gitlab_git_fetch`, ...).
#[must_use]
pub fn fetch_tool_for(service: &str) -> String {
    format!("{service}_{GIT_FETCH_OP}")
}

/// Field of the phone's answer that holds the sealed [`CredentialGrant`] (base64url, no padding): in the single item of
/// a fetch (`items[0].sealed`), at the top of a push's data (`sealed`).
pub const SEALED_FIELD: &str = "sealed";

/// How long a read credential may be used by the app.
pub const FETCH_LEASE_SECS: i64 = 3600;
/// How long a push credential may be used: for the one push it was approved for.
pub const PUSH_LEASE_SECS: i64 = 600;

pub const MAX_UPDATES: usize = 20;
pub const MAX_COMMITS: usize = 50;
pub const MAX_FILES: usize = 300;
pub const MAX_PUSH_OPTIONS: usize = 10;
pub const MAX_NOTES: usize = 10;
/// Files listed for one commit (its own change against its first parent).
pub const MAX_COMMIT_FILES: usize = 100;
/// Lines of one file's patch, hunk headers included; a longer change is cut (`patch_cut`).
pub const MAX_PATCH_LINES: usize = 400;
/// Characters of one patch line; a longer line is cut with "…".
pub const MAX_PATCH_LINE: usize = 240;
/// Largest push summary the desktop app sends (the tool's `summary` argument allows a little more).
pub const MAX_SUMMARY_BYTES: usize = 190_000;
const MAX_REF: usize = 250;
const MAX_PATH: usize = 1_024;
const MAX_LINE: usize = 200;

/// All-zero object id: the old side of a created ref, the new side of a deleted one.
pub const ZERO_OID: &str = "0000000000000000000000000000000000000000";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RefChange {
    Create,
    Update,
    Delete,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileStatus {
    Added,
    Modified,
    Deleted,
    /// A file became a link, a submodule or the other way round.
    TypeChanged,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommitInfo {
    pub sha: String,
    /// First line of the message.
    pub subject: String,
    /// "Name <email>".
    pub author: String,
    /// Author time, unix seconds.
    pub date: i64,
    /// What this commit itself changes, against its first parent (all added for a root commit): the files, at most
    /// [`MAX_COMMIT_FILES`], and how many there are; `None` when they were not compared.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub files_changed: Option<u32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub files: Vec<FileChange>,
    /// Lines the commit adds and removes; `None` when not every file could be counted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub additions: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deletions: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileChange {
    pub path: String,
    pub status: FileStatus,
    /// Lines added and removed; `None` when not counted (binary, too large, or unavailable).
    #[serde(default)]
    pub additions: Option<u32>,
    #[serde(default)]
    pub deletions: Option<u32>,
    #[serde(default)]
    pub binary: bool,
    /// The change as a unified diff without its file headers: `@@ -a,b +c,d @@` lines, then one line per line of
    /// the files, starting with ' ', '+' or '-' (three lines of context). At most [`MAX_PATCH_LINES`] lines of at
    /// most [`MAX_PATCH_LINE`] characters, no control characters but tabs. `None` when not shown (binary, too large,
    /// over the push's budget, or the file of a whole ref rather than of one commit).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub patch: Option<String>,
    /// `patch` stops before the end of the change.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub patch_cut: bool,
}

impl FileChange {
    /// A file without a patch.
    #[must_use]
    pub fn new(path: String, status: FileStatus, additions: Option<u32>, deletions: Option<u32>, binary: bool) -> Self {
        Self {
            path,
            status,
            additions,
            deletions,
            binary,
            patch: None,
            patch_cut: false,
        }
    }
}

/// One line of a patch is valid: a hunk header or a line of context, added or removed, without control characters.
fn patch_ok(patch: &str) -> Result<(), String> {
    let lines: Vec<&str> = patch.split('\n').collect();
    if lines.len() > MAX_PATCH_LINES {
        return Err(format!("a patch has at most {MAX_PATCH_LINES} lines"));
    }
    for line in lines {
        let ok_start = line.starts_with("@@ ") || line.starts_with([' ', '+', '-']) || line.starts_with('\\');
        if !ok_start {
            return Err("a patch line starts with @@, ' ', '+', '-' or '\\'".to_owned());
        }
        if line.chars().count() > MAX_PATCH_LINE + 1 || line.chars().any(|c| c.is_control() && c != '\t') {
            return Err(format!("a patch line is at most {MAX_PATCH_LINE} characters, without control characters"));
        }
    }
    Ok(())
}

/// One ref the push changes, as git sent it, with what the app found out about it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RefUpdate {
    /// Full name: `refs/heads/main`, `refs/tags/v1.0`.
    pub name: String,
    pub change: RefChange,
    /// 40 hex digits; [`ZERO_OID`] for a created ref.
    pub old: String,
    /// 40 hex digits; [`ZERO_OID`] for a deleted ref.
    pub new: String,
    /// `Some(true)`: only adds commits. `Some(false)`: rewrites history (a force push). `None`: could not tell, which
    /// is treated like a force push. `None` also for a created or deleted ref.
    #[serde(default)]
    pub fast_forward: Option<bool>,
    /// Commits the ref gains (reachable from `new`, not from `old`); may exceed `commits.len()`.
    pub commit_count: u32,
    /// Newest first, at most [`MAX_COMMITS`].
    #[serde(default)]
    pub commits: Vec<CommitInfo>,
    /// Files that differ between `old` and `new` (for a created ref: against the default branch when known, else 0);
    /// may exceed `files.len()`.
    pub files_changed: u32,
    /// At most [`MAX_FILES`].
    #[serde(default)]
    pub files: Vec<FileChange>,
    #[serde(default)]
    pub additions: Option<u64>,
    #[serde(default)]
    pub deletions: Option<u64>,
}

impl RefUpdate {
    /// The branch name, for `refs/heads/…`.
    #[must_use]
    pub fn branch(&self) -> Option<&str> {
        self.name.strip_prefix("refs/heads/")
    }

    /// The tag name, for `refs/tags/…`.
    #[must_use]
    pub fn tag(&self) -> Option<&str> {
        self.name.strip_prefix("refs/tags/")
    }

    /// Rewrites or drops history, or could not be checked: never covered by a standing permission.
    #[must_use]
    pub fn is_risky(&self) -> bool {
        match self.change {
            RefChange::Delete => true,
            RefChange::Create => false,
            // Moving an existing tag rewrites what a release points at.
            RefChange::Update => self.tag().is_some() || self.fast_forward != Some(true),
        }
    }
}

/// What a push does, built by the desktop app from the commands and pack git sent.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PushSummary {
    /// In the order git sent them; at most [`MAX_UPDATES`].
    pub updates: Vec<RefUpdate>,
    /// Size of the pack git sent.
    pub pack_bytes: u64,
    #[serde(default)]
    pub push_options: Vec<String>,
    /// What the app could not work out ("File list unavailable: …"), shown to the user.
    #[serde(default)]
    pub notes: Vec<String>,
}

/// A list [`PushSummary::fit`] can shorten.
#[derive(Clone, Copy)]
enum List {
    Files,
    Commits,
    CommitFiles(usize),
}

fn is_oid(s: &str) -> bool {
    s.len() == 40 && s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn clean(s: &str, max: usize, what: &str) -> Result<(), String> {
    if s.is_empty() || s.chars().count() > max {
        return Err(format!("{what} must be 1..={max} characters"));
    }
    if s.chars().any(char::is_control) {
        return Err(format!("{what} must not contain control characters"));
    }
    Ok(())
}

fn file_ok(f: &FileChange) -> Result<(), String> {
    clean(&f.path, MAX_PATH, "a file path")?;
    match &f.patch {
        Some(p) => patch_ok(p),
        None if f.patch_cut => Err("a patch is cut but missing".to_owned()),
        None => Ok(()),
    }
}

impl PushSummary {
    /// Bounds and formats; the phone refuses a push whose summary does not pass.
    pub fn validate(&self) -> Result<(), String> {
        if self.updates.is_empty() || self.updates.len() > MAX_UPDATES {
            return Err(format!("a push changes 1..={MAX_UPDATES} refs"));
        }
        if self.push_options.len() > MAX_PUSH_OPTIONS || self.notes.len() > MAX_NOTES {
            return Err("too many push options or notes".to_owned());
        }
        for o in &self.push_options {
            clean(o, MAX_LINE, "a push option")?;
        }
        for n in &self.notes {
            clean(n, MAX_LINE, "a note")?;
        }
        let mut seen = std::collections::BTreeSet::new();
        for u in &self.updates {
            clean(&u.name, MAX_REF, "a ref name")?;
            if !u.name.starts_with("refs/") || u.name.contains("..") || u.name.contains(' ') {
                return Err(format!("{} is not a ref name", u.name));
            }
            if !seen.insert(&u.name) {
                return Err(format!("{} appears twice", u.name));
            }
            if !is_oid(&u.old) || !is_oid(&u.new) {
                return Err("object ids are 40 lowercase hex digits".to_owned());
            }
            let expected = match (u.old == ZERO_OID, u.new == ZERO_OID) {
                (true, false) => RefChange::Create,
                (false, true) => RefChange::Delete,
                (false, false) => RefChange::Update,
                (true, true) => return Err("a ref update needs an old or a new object".to_owned()),
            };
            if u.change != expected {
                return Err(format!("{} is a {expected:?}, not a {:?}", u.name, u.change));
            }
            if u.commits.len() > MAX_COMMITS || u.files.len() > MAX_FILES {
                return Err("too many commits or files listed".to_owned());
            }
            if (u.commits.len() as u64) > u64::from(u.commit_count)
                || (u.files.len() as u64) > u64::from(u.files_changed)
            {
                return Err("more commits or files listed than counted".to_owned());
            }
            for c in &u.commits {
                if !is_oid(&c.sha) {
                    return Err("commit ids are 40 lowercase hex digits".to_owned());
                }
                if c.subject.chars().count() > MAX_LINE || c.subject.chars().any(char::is_control) {
                    return Err("a commit subject is one line of at most 200 characters".to_owned());
                }
                clean(&c.author, MAX_LINE, "a commit author")?;
                if c.files.len() > MAX_COMMIT_FILES || (c.files.len() as u64) > u64::from(c.files_changed.unwrap_or(0))
                {
                    return Err("too many files listed for a commit".to_owned());
                }
                for f in &c.files {
                    file_ok(f)?;
                }
            }
            for f in &u.files {
                file_ok(f)?;
            }
        }
        Ok(())
    }

    /// Shortens the summary until its JSON is at most `max` bytes, and says so in a note: first the patches (largest
    /// first), then the commit and file lists (longest first; the counts stay). `false` when even empty lists do not
    /// fit.
    pub fn fit(&mut self, max: usize) -> bool {
        const PATCHES_NOTE: &str = "Some changes are not shown line by line: the push is too large.";
        const LISTS_NOTE: &str = "Some commits and files are not listed: the push is too large to show in full.";
        let size = |s: &Self| serde_json::to_vec(s).map_or(usize::MAX, |v| v.len());
        let mut estimate = size(self);
        if estimate <= max {
            return true;
        }
        // The note is part of the JSON it has to fit in, so it goes in first.
        let has_patches =
            self.updates.iter().flat_map(|u| &u.commits).flat_map(|c| &c.files).any(|f| f.patch.is_some());
        let noted = |s: &Self, note: &str| s.notes.iter().any(|n| n == note);
        if has_patches && !noted(self, PATCHES_NOTE) && !noted(self, LISTS_NOTE) && self.notes.len() < MAX_NOTES {
            self.notes.push(PATCHES_NOTE.to_owned());
            estimate += PATCHES_NOTE.len() + 3;
        }
        // Each patch dropped takes its JSON string and key off the estimate; the size is measured again after.
        while estimate > max {
            let largest = self
                .updates
                .iter_mut()
                .flat_map(|u| u.commits.iter_mut())
                .flat_map(|c| c.files.iter_mut())
                .filter(|f| f.patch.is_some())
                .max_by_key(|f| f.patch.as_ref().map_or(0, String::len));
            let Some(file) = largest else {
                break;
            };
            let json = serde_json::to_string(&file.patch).map_or(0, |s| s.len());
            estimate = estimate.saturating_sub(json + r#","patch":"#.len());
            if file.patch_cut {
                estimate = estimate.saturating_sub(r#","patch_cut":true"#.len());
            }
            file.patch = None;
            file.patch_cut = false;
        }
        if size(self) > max && !noted(self, LISTS_NOTE) {
            // Lists go too: one note says both.
            self.notes.retain(|n| n != PATCHES_NOTE);
            if self.notes.len() < MAX_NOTES {
                self.notes.push(LISTS_NOTE.to_owned());
            }
        }
        while size(self) > max {
            // The longest list (a ref's files, its commits, a commit's files) loses half.
            let Some((u, list)) = self
                .updates
                .iter()
                .enumerate()
                .flat_map(|(i, u)| {
                    [(u.files.len(), i, List::Files), (u.commits.len(), i, List::Commits)]
                        .into_iter()
                        .chain(u.commits.iter().enumerate().map(move |(j, c)| (c.files.len(), i, List::CommitFiles(j))))
                })
                .max_by_key(|(len, _, _)| *len)
                .filter(|(len, _, _)| *len > 0)
                .map(|(_, i, list)| (i, list))
            else {
                return false;
            };
            let update = &mut self.updates[u];
            match list {
                List::Files => update.files.truncate(update.files.len() / 2),
                List::Commits => update.commits.truncate(update.commits.len() / 2),
                List::CommitFiles(j) => {
                    let files = &mut update.commits[j].files;
                    files.truncate(files.len() / 2);
                }
            }
        }
        true
    }

    /// The tool a push to `service` asks for (`github`, `gitlab`, `codeberg`, `bitbucket`).
    #[must_use]
    pub fn tool_for(&self, service: &str) -> String {
        let op = if self.updates.iter().all(|u| u.tag().is_some()) {
            GIT_TAG_PUSH_OP
        } else {
            GIT_PUSH_OP
        };
        format!("{service}_{op}")
    }

    /// The tool a push asks for: tags only is a tag push, anything with a branch (or another ref) a push.
    #[must_use]
    pub fn tool(&self) -> &'static str {
        if self.updates.iter().all(|u| u.tag().is_some()) {
            GIT_TAG_PUSH_TOOL
        } else {
            GIT_PUSH_TOOL
        }
    }

    /// What a permission for this push has to cover: `repo@branch` for one branch, the whole `repo` otherwise.
    #[must_use]
    pub fn resource(&self, repo: &str) -> String {
        match self.updates.as_slice() {
            [only] => match only.branch() {
                Some(branch) => format!("{repo}@{branch}"),
                None => repo.to_owned(),
            },
            _ => repo.to_owned(),
        }
    }

    /// Asked for every time: a force push, a deletion, a moved tag, or several branches at once.
    #[must_use]
    pub fn once_only(&self) -> bool {
        self.updates.iter().any(RefUpdate::is_risky)
            || self.updates.iter().filter(|u| u.branch().is_some()).count() > 1
            || self.updates.iter().any(|u| u.branch().is_none() && u.tag().is_none())
    }
}

/// What the phone seals to the desktop app: a GitHub credential for one repository and one purpose.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CredentialGrant {
    pub v: u32,
    /// The nonce of the request this answers.
    pub nonce: String,
    /// For a push: the [`push_digest`] the user approved. The app refuses a grant whose digest differs.
    #[serde(default)]
    pub digest: Option<String>,
    /// owner/name.
    pub repo: String,
    /// "read" or "write".
    pub access: String,
    /// HTTP basic user name to go with the token ("x-access-token").
    pub username: String,
    pub token: String,
    /// Unix seconds.
    pub expires_at: i64,
}

impl std::fmt::Debug for CredentialGrant {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CredentialGrant")
            .field("nonce", &self.nonce)
            .field("digest", &self.digest)
            .field("repo", &self.repo)
            .field("access", &self.access)
            .field("expires_at", &self.expires_at)
            .finish_non_exhaustive()
    }
}

/// What the phone seals to the desktop app for `desktop_ask` (the decision, bound to the question's nonce).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AskAnswer {
    pub v: u32,
    pub nonce: String,
    pub approved: bool,
}

/// What the phone seals to the desktop app for `vault_secret_release` (each requested `item/field` and its value) and
/// for `vault_env_release` (each variable of the environment and its value, with the environment it came from).
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecretGrant {
    pub v: u32,
    pub nonce: String,
    /// In the order requested (`vault_env_release` with every variable: sorted by name).
    pub secrets: Vec<(String, String)>,
    /// Unix seconds; the desktop app forgets them then.
    pub expires_at: i64,
    /// `vault_env_release`: the project and environment released.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub env: Option<String>,
    /// `vault_env_release`: the environment's version (the vault item's revision date).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revision: Option<String>,
    /// `vault_env_release`: who it was released to (`person`, `agent:<harness>`), as the phone approved it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runner: Option<String>,
    /// `vault_env_release`: the project's folder on the computer (its canonical path) it was released for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root: Option<String>,
    /// `vault_env_release`: the commands the desktop app may hand these values to until they expire, without asking
    /// again: exactly the command approved, or the argument prefix the user chose on the phone. The variables it may
    /// hand them for are the names in `secrets`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<CommandScope>,
}

impl std::fmt::Debug for SecretGrant {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let names: Vec<&str> = self.secrets.iter().map(|(name, _)| name.as_str()).collect();
        f.debug_struct("SecretGrant")
            .field("nonce", &self.nonce)
            .field("secrets", &names)
            .field("expires_at", &self.expires_at)
            .finish_non_exhaustive()
    }
}

/// What the phone seals to the desktop app for `vault_ssh_sign`: the SSH signature blob (`string alg, string sig`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SshSignature {
    pub v: u32,
    pub nonce: String,
    pub signature_base64: String,
}

/// How the phone's refusal of a `vault_secret_store` it cannot open starts (a box for another phone's key, after a new
/// phone or the app reinstalled). The server could send it too, so the desktop app never acts on it by itself: it tells
/// the user, who checks the new phone's key with `reins vault add --new-phone`.
pub const PHONE_KEY_CHANGED: &str = "This was sealed to another phone's key.";

/// The direction a box between the desktop app and the phone goes (its `dir`). Both directions use the same
/// X25519 key pair, so a box is refused when it was made for the other direction.
pub const TO_PHONE: &str = "desktop-to-phone";
pub const TO_DESKTOP: &str = "phone-to-desktop";

/// Text the phone or the server sent, made safe to print in a terminal: control characters (escape sequences) and
/// bidirectional overrides become spaces, and it is cut to 300 characters. The server can write any such text, so the
/// desktop app also says where it came from.
#[must_use]
pub fn printable(text: &str) -> String {
    let bidi =
        |c: char| matches!(c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}' | '\u{200e}' | '\u{200f}' | '\u{061c}');
    let clean: String = text
        .chars()
        .map(|c| {
            if c.is_control() || bidi(c) {
                ' '
            } else {
                c
            }
        })
        .collect();
    let clean = clean.split_whitespace().collect::<Vec<_>>().join(" ");
    if clean.chars().count() > 300 {
        format!("{}…", clean.chars().take(299).collect::<String>())
    } else {
        clean
    }
}

/// How long a `vault_secret_store` box is good for after the desktop app made it (seconds): the phone refuses an older
/// one, and remembers the nonces it saw for as long, so a captured request cannot be sent again.
pub const SECRET_STORE_TTL_SECS: i64 = 15 * 60;

/// What the desktop app sends the phone for `vault_secret_store` (`reins vault add`): the value to keep, bound to the
/// request's nonce, when it was made, the item's name, the field, the kind of a new item and whether an existing item
/// may be changed. It travels in a box from the app's key to the phone's key (X25519, XSalsa20-Poly1305,
/// `nonce || ciphertext`): the server relays it without being able to open or alter it, and the phone, which pinned the
/// app's key, knows the paired app wrote it. The value is wiped when this is dropped.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecretToStore {
    pub v: u32,
    /// [`TO_PHONE`]: the two directions share one key, so each box says which way it goes.
    pub dir: String,
    pub nonce: String,
    /// Unix seconds, on the computer.
    pub created_at: i64,
    pub name: String,
    pub field: String,
    /// `api-key`, `login`, `note` or `ssh`: the kind of a new item.
    pub kind: String,
    /// The user asked to change an existing item (`--replace`); without it the phone only makes new items.
    pub replace: bool,
    pub value: String,
}

impl Drop for SecretToStore {
    fn drop(&mut self) {
        zeroize::Zeroize::zeroize(&mut self.value);
    }
}

impl std::fmt::Debug for SecretToStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SecretToStore").field("nonce", &self.nonce).field("name", &self.name).finish_non_exhaustive()
    }
}

/// Four digits both the terminal and the phone show for a value (`reins vault add`), so that the user can see the
/// phone got the value they meant, not one an AI on the computer sent in its place. Not a secret on its own: about
/// 13 bits of a hash of the value.
#[must_use]
pub fn value_check(value: &str) -> String {
    let mut h = Sha256::new();
    h.update(b"reins-vault-value/1");
    h.update(value.as_bytes());
    let d = h.finalize();
    format!("{:04}", u16::from_be_bytes([d[0], d[1]]) % 10_000)
}

/// What the phone seals to the desktop app for `vault_phone_key`: the public key secrets for the vault are sealed to.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PhoneKey {
    pub v: u32,
    pub nonce: String,
    /// X25519, base64url without padding.
    pub public_key: String,
}

/// What the phone answers a `vault_secret_store` it carried out, in a box from its inbox key to the desktop app's key:
/// only the phone the app pinned can have written it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoreAck {
    pub v: u32,
    /// [`TO_DESKTOP`].
    pub dir: String,
    pub nonce: String,
    pub name: String,
    pub field: String,
    /// A new item was made (else the field of an existing one changed).
    pub created: bool,
    /// Where the earlier value was kept, when one was replaced ("the password history", "the hidden field ...").
    pub kept: Option<String>,
}

/// What the phone answers `vault_names`, in a box from its inbox key to the desktop app's key: the names of the vault's
/// items, by kind.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VaultNames {
    pub v: u32,
    /// [`TO_DESKTOP`].
    pub dir: String,
    pub nonce: String,
    /// `(name, kind)`, the kind as the vault tools name it (`login`, `note`, `card`, `identity`, `ssh_key`).
    pub items: Vec<(String, String)>,
}

// ---- Reins Secrets (docs/secrets.md) --------------------------------------------------------------------------------

/// The tools of Reins Secrets (all desktop-only).
pub const ENV_RELEASE_TOOL: &str = "vault_env_release";
pub const SECRETS_STORE_TOOL: &str = "vault_secrets_store";
pub const SECRETS_LIST_TOOL: &str = "vault_secrets_list";
pub const TOKEN_STORE_TOOL: &str = "integration_token_store";
/// The vault folder of Reins Secrets; a project is the folder `Reins/<project>`.
pub const SECRETS_FOLDER: &str = "Reins";
/// The hidden custom field that marks a Secure Note as an environment, and its value.
pub const ENV_KIND_FIELD: &str = "reins.kind";
pub const ENV_KIND: &str = "env/1";
/// The hidden custom field that names the environment (its identity; the item's name is only shown).
pub const ENV_NAME_FIELD: &str = "reins.env";
/// Custom fields whose names start with this belong to Reins, not to the environment's variables.
pub const RESERVED_PREFIX: &str = "reins.";
/// The most variables one release or one environment holds, and the most bytes of values one release carries.
pub const MAX_ENV_VARS: usize = 200;
pub const MAX_ENV_BYTES: usize = 64 * 1024;
/// The most environments one `vault_secrets_store` changes, and the most bytes of values it carries.
pub const MAX_STORE_ENVS: usize = 20;
pub const MAX_STORE_BYTES: usize = 128 * 1024;
/// How long a store box is good for (as [`SECRET_STORE_TTL_SECS`]).
pub const SECRETS_STORE_TTL_SECS: i64 = SECRET_STORE_TTL_SECS;
/// The runner of a person at the computer; an agent's is `agent:<harness>` (`agent:claude-code`).
pub const PERSON: &str = "person";

/// Whether `runner` is `person` or `agent:<harness>` (harness: 1 to 40 of `a-z 0-9 - _ .`).
#[must_use]
pub fn runner_ok(runner: &str) -> bool {
    runner == PERSON
        || runner.strip_prefix("agent:").is_some_and(|h| {
            !h.is_empty()
                && h.len() <= 40
                && h.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'-' | b'_' | b'.'))
        })
}

/// A project or environment name: 1 to 64 of `A-Z a-z 0-9 . _ -`, not starting with `.` or `-` (never a `/`, so
/// `project/env` is one item name).
#[must_use]
pub fn name_ok(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && !name.starts_with(['.', '-'])
        && name.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

/// An environment variable name: a letter or `_`, then letters, digits and `_`, at most 100.
#[must_use]
pub fn var_ok(name: &str) -> bool {
    let mut bytes = name.bytes();
    bytes.next().is_some_and(|b| b.is_ascii_alphabetic() || b == b'_')
        && bytes.all(|b| b.is_ascii_alphanumeric() || b == b'_')
        && name.len() <= 100
}

/// The most arguments a command of a release may have, and the longest one.
pub const MAX_ARGS: usize = 64;
pub const MAX_ARG: usize = 1_000;
/// The longest exact command a standing permission may name, as shown (`bash -c <code>`, `python -c <code>` among
/// them): the phone shows all of it, never cut short. A longer one is asked every time.
pub const MAX_REMEMBERED_CODE: usize = 300;

/// The commands a release (or a standing permission for secrets) covers. `reins run` never uses a shell: a command is
/// the program and its argument list as spawned, compared as whole arguments, never as text.
///
/// - exact (`prefix` false): this program with exactly these arguments;
/// - prefix (`prefix` true): this program with these first arguments, then zero or more whole arguments (`npm run dev *`
///   covers `npm run dev --watch`, never `npm run devx`). Closed by construction: a prefix is allowed only for a
///   program and first arguments on an allow-list ([`GRAMMARS`]), and the arguments the `*` stands for only when each
///   is a plain word or an option that program's grammar lists as safe ([`tail_refusal`]). Everything else is allowed
///   exactly only.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CommandScope {
    /// The program, by its absolute path when the computer could resolve it (so another `npm` elsewhere on PATH is
    /// another program).
    pub program: String,
    pub args: Vec<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub prefix: bool,
}

/// One word of a grammar's fixed part: a literal (`run`), or a name the project defines (a script, a target).
#[derive(Clone, Copy, Debug)]
pub enum Word {
    Lit(&'static str),
    /// A plain name: letters, digits and `_ . : -`, not starting with `-` or `+`.
    Name,
}

/// The prefixes one program may be remembered by, and the options the `*` may stand for.
#[derive(Debug)]
pub struct Grammar {
    /// The program's names, as [`program_name`] gives them.
    pub programs: &'static [&'static str],
    /// The fixed parts a prefix may have (`[run, <script>]`: `npm run dev *`). An empty one: the program alone.
    pub prefixes: &'static [&'static [Word]],
    /// The options the `*` may stand for (`--release`; `--features=x` and `-j8` match `--features` and `-j`).
    /// Anything else that starts with `-` or `+` is refused there.
    pub options: &'static [&'static str],
}

const CARGO_OPTIONS: &[&str] = &[
    "--release",
    "-r",
    "--workspace",
    "--all",
    "--all-targets",
    "--all-features",
    "--no-default-features",
    "--features",
    "-F",
    "-p",
    "--package",
    "--bin",
    "--bins",
    "--lib",
    "--tests",
    "--test",
    "--examples",
    "--example",
    "--benches",
    "--bench",
    "--locked",
    "--frozen",
    "--offline",
    "-q",
    "--quiet",
    "-v",
    "-vv",
    "--verbose",
    "-j",
    "--jobs",
    "--no-fail-fast",
    "--doc",
    "--exclude",
    "--profile",
    "--message-format",
    "--color",
    "--keep-going",
    "--no-run",
    "--timings",
];
const PYTEST_OPTIONS: &[&str] = &[
    "-v",
    "-vv",
    "-q",
    "-x",
    "--exitfirst",
    "-k",
    "-m",
    "-s",
    "--lf",
    "--last-failed",
    "--ff",
    "--failed-first",
    "-n",
    "--maxfail",
    "--tb",
    "--durations",
    "--no-header",
    "-ra",
    "-rA",
    "--co",
    "--collect-only",
];

/// Every program a prefix may be remembered for, with its grammar. Not here: allowed exactly only.
pub const GRAMMARS: &[Grammar] = &[
    Grammar {
        programs: &["cargo"],
        prefixes: &[
            &[Word::Lit("build")],
            &[Word::Lit("test")],
            &[Word::Lit("run")],
            &[Word::Lit("check")],
            &[Word::Lit("clippy")],
        ],
        options: CARGO_OPTIONS,
    },
    Grammar {
        programs: &["npm", "pnpm", "yarn"],
        prefixes: &[&[Word::Lit("run"), Word::Name], &[Word::Lit("test")], &[Word::Lit("start")]],
        options: &["--silent", "-s", "--if-present"],
    },
    Grammar {
        programs: &["make", "gmake"],
        prefixes: &[&[Word::Name]],
        options: &["-j", "--jobs", "-k", "--keep-going", "-s", "--silent", "--quiet", "-n", "--dry-run", "-B", "-O"],
    },
    Grammar {
        programs: &["go"],
        prefixes: &[&[Word::Lit("build")], &[Word::Lit("test")], &[Word::Lit("run")], &[Word::Lit("vet")]],
        options: &[
            "-v",
            "-race",
            "-count",
            "-run",
            "-timeout",
            "-short",
            "-cover",
            "-tags",
            "-p",
            "-failfast",
            "-bench",
            "-benchmem",
            "-trimpath",
        ],
    },
    Grammar {
        programs: &["pytest", "py.test"],
        prefixes: &[&[]],
        options: PYTEST_OPTIONS,
    },
    Grammar {
        programs: &["uv"],
        prefixes: &[&[Word::Lit("run"), Word::Lit("pytest")]],
        options: PYTEST_OPTIONS,
    },
    Grammar {
        programs: &["rails"],
        prefixes: &[&[Word::Lit("server")], &[Word::Lit("s")], &[Word::Lit("test")], &[Word::Lit("t")]],
        options: &["-p", "--port", "-b", "--binding", "-e", "--environment"],
    },
    Grammar {
        programs: &["docker"],
        prefixes: &[&[Word::Lit("compose"), Word::Lit("up")]],
        options: &[
            "-d",
            "--detach",
            "--build",
            "--no-build",
            "--no-deps",
            "--force-recreate",
            "--remove-orphans",
            "--wait",
        ],
    },
];

/// The grammar of `program`, if a prefix may be remembered for it.
#[must_use]
pub fn grammar(program: &str) -> Option<&'static Grammar> {
    if doubtful_name(program) {
        return None;
    }
    let name = program_name(program);
    GRAMMARS.iter().find(|g| g.programs.contains(&name.as_str()))
}

fn plain_name(w: &str) -> bool {
    w.chars().next().is_some_and(|c| c.is_ascii_alphanumeric() || c == '_')
        && w.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | ':' | '-'))
}

/// The program's name as compared with the lists above: its file name, lower case on every platform (macOS and
/// Windows find `NPM` and `Bash` as `npm` and `bash`), without the trailing dots and spaces Windows ignores, a Windows
/// or script extension, a `-cli` or `-preview` ending, and without a version (`python3.12`, `node22`, `bash5.2` and
/// `php8.3` are `python`, `node`, `bash` and `php`). A package manager installed as a script resolves to that script:
/// `npm` is `…/npm/bin/npm-cli.js`, `pnpm` is `…/pnpm/bin/pnpm.cjs`, and both are still `npm` and `pnpm` here.
#[must_use]
pub fn program_name(program: &str) -> String {
    let file = program.trim_end_matches(['.', ' ']);
    let file = file.rsplit(['/', '\\']).next().unwrap_or(file).to_lowercase();
    let mut name = file.trim_end_matches(['.', ' ']);
    loop {
        let before = name;
        name = [".exe", ".cmd", ".bat", ".com", ".ps1", ".js", ".cjs", ".mjs", ".py", ".rb", ".pl", ".sh"]
            .iter()
            .find_map(|x| name.strip_suffix(x))
            .unwrap_or(name);
        name = name.trim_end_matches(|c: char| c.is_ascii_digit() || matches!(c, '.' | ' '));
        name = ["-cli", "-preview"].iter().find_map(|x| name.strip_suffix(x)).unwrap_or(name);
        if name == before {
            break;
        }
    }
    if name.is_empty() {
        file.clone()
    } else {
        name.to_owned()
    }
}

/// Whether a program's name leaves any doubt about what it is, so that no list can be trusted to know it: anything
/// but ASCII letters, digits and `-_.+` (a look-alike letter from another script, a Windows 8.3 short name such as
/// `PYTHON~1.EXE`), or nothing left.
#[must_use]
pub fn doubtful_name(program: &str) -> bool {
    let file = program.trim_end_matches(['.', ' ']);
    let file = file.rsplit(['/', '\\']).next().unwrap_or(file);
    file.is_empty() || !file.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '+'))
}

/// [`prefix_refusal`] for a program known by several names: as the user typed it (`npm`), each link on the way to it,
/// and as the computer resolved it (`…/npm-cli.js`). Refused unless every one of them is the same program on the
/// allow-list: any doubt about which program runs means exact only.
#[must_use]
pub fn prefix_refusal_as(names: &[&str], program: &str, args: &[String]) -> Option<String> {
    if let Some(why) = prefix_refusal(program, args) {
        return Some(why);
    }
    let name = program_name(program);
    names
        .iter()
        .find(|n| doubtful_name(n) || program_name(n) != name)
        .map(|n| format!("{n} is not plainly {name}: only the exact command can be allowed"))
}

/// Why `program` with the fixed first arguments `fixed` may not be a prefix pattern; `None` when its grammar
/// ([`GRAMMARS`]) allows exactly these.
#[must_use]
pub fn prefix_refusal(program: &str, fixed: &[String]) -> Option<String> {
    let name = program_name(program);
    let Some(g) = grammar(program) else {
        return Some(format!("{name} can be allowed only exactly"));
    };
    let fits = |words: &[Word]| {
        words.len() == fixed.len()
            && words.iter().zip(fixed).all(|(w, a)| match w {
                Word::Lit(l) => a == l,
                Word::Name => plain_name(a),
            })
    };
    if g.prefixes.iter().any(|p| fits(p)) {
        None
    } else {
        Some(format!("{name} {} * is not a pattern Reins allows: only the exact command", fixed.join(" ")))
    }
}

/// Why the arguments `tail` may not stand for the `*` after `program` `fixed`; `None` when every one is a plain word
/// (not a path out of the project: absolute, `~`, or with a `..` part; not a `NAME=value` assignment; not `--`) or an
/// option the program's grammar lists as safe. Options that run other code or read other configuration (`cargo
/// --config`, `npm --script-shell`, `make -f`, `pytest -p`, `go -toolexec`, …) are not listed, so they need the exact
/// command.
#[must_use]
pub fn tail_refusal(program: &str, tail: &[String]) -> Option<String> {
    let Some(g) = grammar(program) else {
        return Some("no pattern for this program".to_owned());
    };
    for a in tail {
        if a == "--" {
            return Some("arguments after `--` are allowed only exactly".to_owned());
        }
        if a.starts_with('-') || a.starts_with('+') {
            let key = a.split_once('=').map_or(a.as_str(), |(k, _)| k);
            let short_count = g
                .options
                .iter()
                .any(|o| o.len() == 2 && a.len() > 2 && a.starts_with(o) && a[2..].chars().all(|c| c.is_ascii_digit()));
            if !g.options.contains(&key) && !short_count {
                return Some(format!("{a} is allowed only in the exact command"));
            }
            continue;
        }
        let assignment = a.split_once('=').is_some_and(|(k, _)| {
            !k.is_empty()
                && k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
                && !k.starts_with(char::is_numeric)
        });
        let outside = a.starts_with('/')
            || a.starts_with('\\')
            || a.starts_with('~')
            || a.get(1..2) == Some(":")
            || a.split(['/', '\\']).any(|part| part == "..");
        if assignment || outside {
            return Some(format!("{a} is allowed only in the exact command"));
        }
    }
    None
}

fn clean_arg(a: &str) -> bool {
    a.chars().count() <= MAX_ARG && !a.chars().any(char::is_control)
}

impl CommandScope {
    #[must_use]
    pub fn exact(program: &str, args: &[String]) -> Self {
        Self {
            program: program.to_owned(),
            args: args.to_vec(),
            prefix: false,
        }
    }

    /// Bounds, and for a prefix [`prefix_refusal`].
    pub fn validate(&self) -> Result<(), String> {
        if self.program.trim().is_empty() || !clean_arg(&self.program) {
            return Err("a command needs a program".to_owned());
        }
        if self.args.len() > MAX_ARGS || !self.args.iter().all(|a| clean_arg(a)) {
            return Err(format!("a command has at most {MAX_ARGS} arguments of at most {MAX_ARG} characters"));
        }
        match self.prefix.then(|| prefix_refusal(&self.program, &self.args)).flatten() {
            Some(why) => Err(why),
            None => Ok(()),
        }
    }

    /// Whether `program` with `args` is covered: the same program, then the same arguments (all of them, or these
    /// first ones and any whole arguments after). A prefix that fails [`prefix_refusal`] covers nothing.
    #[must_use]
    pub fn matches(&self, program: &str, args: &[String]) -> bool {
        if program != self.program {
            return false;
        }
        if self.prefix {
            prefix_refusal(&self.program, &self.args).is_none()
                && args.starts_with(&self.args)
                && tail_refusal(&self.program, &args[self.args.len()..]).is_none()
        } else {
            args == self.args.as_slice()
        }
    }

    /// Whether every command this one covers is covered by `wider` too.
    #[must_use]
    pub fn within(&self, wider: &Self) -> bool {
        if self.program != wider.program {
            return false;
        }
        if wider.prefix {
            prefix_refusal(&wider.program, &wider.args).is_none()
                && self.args.starts_with(&wider.args)
                && tail_refusal(&wider.program, &self.args[wider.args.len()..]).is_none()
        } else {
            !self.prefix && self.args == wider.args
        }
    }

    /// The patterns that can be offered for this exact command, narrowest first: the command itself, then each
    /// shorter prefix its grammar allows and that covers it.
    #[must_use]
    pub fn offers(&self) -> Vec<Self> {
        self.offers_as(&[])
    }

    /// Whether a standing permission may name this command: a prefix its grammar allows by every name of the
    /// program ([`prefix_refusal_as`]); an exact command when the phone can show all of it ([`MAX_REMEMBERED_CODE`]
    /// characters), else it is approved once at a time.
    #[must_use]
    pub fn rememberable(&self, names: &[&str]) -> bool {
        if self.prefix {
            return prefix_refusal_as(names, &self.program, &self.args).is_none();
        }
        self.shown().chars().count() <= MAX_REMEMBERED_CODE
    }

    /// [`CommandScope::offers`], refusing what either name of the program refuses ([`prefix_refusal_as`]): `typed` is
    /// the program as the user typed it.
    #[must_use]
    pub fn offers_as(&self, names: &[&str]) -> Vec<Self> {
        let exact = Self::exact(&self.program, &self.args);
        if !exact.rememberable(names) {
            return Vec::new();
        }
        let mut out = vec![exact];
        for k in (0..=self.args.len()).rev() {
            let fixed = &self.args[..k];
            if prefix_refusal_as(names, &self.program, fixed).is_none()
                && tail_refusal(&self.program, &self.args[k..]).is_none()
            {
                out.push(Self {
                    program: self.program.clone(),
                    args: fixed.to_vec(),
                    prefix: true,
                });
            }
        }
        out
    }

    /// As shown to the user: the program's name and the arguments (quoted when they hold spaces or quotes), then ` *`
    /// for a prefix.
    #[must_use]
    pub fn shown(&self) -> String {
        let quote = |a: &str| {
            if a.is_empty() || a.chars().any(|c| c.is_whitespace() || matches!(c, '"' | '\'' | '*')) {
                format!("'{}'", a.replace('\'', r"'\''"))
            } else {
                a.to_owned()
            }
        };
        let file = self.program.rsplit(['/', '\\']).next().unwrap_or(&self.program);
        let mut words: Vec<String> = std::iter::once(quote(file)).chain(self.args.iter().map(|a| quote(a))).collect();
        if self.prefix {
            words.push("*".to_owned());
        }
        words.join(" ")
    }
}

/// Changes to one environment, as `vault_secrets_store` carries them (in [`SecretsToStore`]). Values are wiped when
/// this is dropped.
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnvChanges {
    pub project: String,
    pub env: String,
    /// Variables to set: a new name is added, an existing one replaced (its earlier value kept in the item's history).
    #[serde(default)]
    pub set: Vec<(String, String)>,
    /// Variables to remove (their values kept in the item's history).
    #[serde(default)]
    pub unset: Vec<String>,
    /// `(from, to)`: variables to rename.
    #[serde(default)]
    pub rename: Vec<(String, String)>,
    /// Another environment of the same project whose variables are copied in first, on the phone (`reins secrets
    /// cp`): the values never leave it. Names `set` here win.
    #[serde(default)]
    pub copy_from: Option<String>,
    /// A `set` may replace a variable that has another value; without it the phone refuses the whole store.
    #[serde(default)]
    pub replace: bool,
}

impl Drop for EnvChanges {
    fn drop(&mut self) {
        for (_, v) in &mut self.set {
            zeroize::Zeroize::zeroize(v);
        }
    }
}

impl std::fmt::Debug for EnvChanges {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let names: Vec<&str> = self.set.iter().map(|(n, _)| n.as_str()).collect();
        f.debug_struct("EnvChanges")
            .field("project", &self.project)
            .field("env", &self.env)
            .field("set", &names)
            .field("unset", &self.unset)
            .field("rename", &self.rename)
            .field("copy_from", &self.copy_from)
            .field("replace", &self.replace)
            .finish()
    }
}

impl EnvChanges {
    /// No changes yet to `project/env` (fill in `set`, `unset`, …; the type wipes its values when dropped, so it is
    /// not built with `..Default::default()`).
    #[must_use]
    pub fn new(project: &str, env: &str) -> Self {
        Self {
            project: project.to_owned(),
            env: env.to_owned(),
            set: Vec::new(),
            unset: Vec::new(),
            rename: Vec::new(),
            copy_from: None,
            replace: false,
        }
    }

    /// `project/env`: the vault item's name, and what the call names beside the box.
    #[must_use]
    pub fn item(&self) -> String {
        format!("{}/{}", self.project, self.env)
    }

    /// Names and counts, as the phone checks them.
    pub fn validate(&self) -> Result<(), String> {
        let item = self.item();
        if !name_ok(&self.project) || !name_ok(&self.env) {
            return Err(format!("`{}` is not a project/environment name", printable(&item)));
        }
        if self.set.is_empty() && self.unset.is_empty() && self.rename.is_empty() && self.copy_from.is_none() {
            return Err(format!("nothing to change in {item}"));
        }
        let mut seen = std::collections::BTreeSet::new();
        let names =
            self.set.iter().map(|(n, _)| n).chain(&self.unset).chain(self.rename.iter().flat_map(|(a, b)| [a, b]));
        for name in names {
            if !var_ok(name) {
                return Err(format!("`{}` in {item} is not a variable name", printable(name)));
            }
        }
        for name in self.set.iter().map(|(n, _)| n).chain(&self.unset) {
            if !seen.insert(name) {
                return Err(format!("{name} is changed twice in {item}"));
            }
        }
        if self.set.len() + self.rename.len() > MAX_ENV_VARS {
            return Err(format!("at most {MAX_ENV_VARS} variables per environment"));
        }
        if let Some(from) = &self.copy_from
            && (!name_ok(from) || from == &self.env)
        {
            return Err(format!("{item} cannot be copied from `{}`", printable(from)));
        }
        Ok(())
    }
}

/// An SSH private key for the phone to keep as an SSH key item in the vault folder `Reins/ssh` (import). The key is
/// wiped when this is dropped.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SshKeyToStore {
    /// The item's name.
    pub name: String,
    /// OpenSSH or PEM, without a passphrase.
    pub private_key: String,
}

impl Drop for SshKeyToStore {
    fn drop(&mut self) {
        zeroize::Zeroize::zeroize(&mut self.private_key);
    }
}

impl std::fmt::Debug for SshKeyToStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SshKeyToStore").field("name", &self.name).finish_non_exhaustive()
    }
}

/// A git host's token for the phone's integration of that host (GitHub, GitLab, Codeberg, Bitbucket). Wiped on drop.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostToken {
    /// `github.com`, `gitlab.com`, `codeberg.org` or `bitbucket.org` ([`crate::connector::TOKEN_HOSTS`]).
    pub host: String,
    /// The user the token was found for, when the source says (Bitbucket signs in as `user:token`).
    #[serde(default)]
    pub user: Option<String>,
    /// What the computer knows of its scopes; shown, not trusted.
    #[serde(default)]
    pub scopes: Vec<String>,
    pub token: String,
}

impl Drop for HostToken {
    fn drop(&mut self) {
        zeroize::Zeroize::zeroize(&mut self.token);
    }
}

impl std::fmt::Debug for HostToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HostToken").field("host", &self.host).field("user", &self.user).finish_non_exhaustive()
    }
}

impl HostToken {
    /// The integration this host's token is for.
    #[must_use]
    pub fn service(&self) -> Option<&'static str> {
        let host = self.host.to_ascii_lowercase();
        crate::connector::TOKEN_HOSTS.iter().find(|(_, h)| *h == host).map(|(s, _)| *s)
    }
}

/// The most SSH keys and tokens one store carries.
pub const MAX_STORE_SSH: usize = 20;
pub const MAX_STORE_TOKENS: usize = 10;

/// A whole `vault_secrets_store`: something to save, at most [`MAX_STORE_ENVS`] distinct environments (each valid),
/// [`MAX_STORE_SSH`] SSH keys with distinct names and [`MAX_STORE_TOKENS`] tokens for hosts the phone serves, at most
/// [`MAX_STORE_BYTES`] of values in all.
pub fn validate_batch(envs: &[EnvChanges], ssh: &[SshKeyToStore], tokens: &[HostToken]) -> Result<(), String> {
    if envs.is_empty() && ssh.is_empty() && tokens.is_empty() {
        return Err("nothing to save".to_owned());
    }
    if envs.len() > MAX_STORE_ENVS || ssh.len() > MAX_STORE_SSH || tokens.len() > MAX_STORE_TOKENS {
        return Err(format!(
            "one store saves at most {MAX_STORE_ENVS} environments, {MAX_STORE_SSH} SSH keys and {MAX_STORE_TOKENS} tokens"
        ));
    }
    let mut items = std::collections::BTreeSet::new();
    let mut bytes = 0usize;
    for e in envs {
        e.validate()?;
        if !items.insert(e.item()) {
            return Err(format!("{} appears twice", e.item()));
        }
        bytes += e.set.iter().map(|(n, v)| n.len() + v.len()).sum::<usize>();
    }
    let mut names = std::collections::BTreeSet::new();
    for k in ssh {
        let name = k.name.trim();
        if name.is_empty() || name.chars().count() > 200 || name.chars().any(char::is_control) {
            return Err("an SSH key needs a name of at most 200 characters".to_owned());
        }
        if !names.insert(name) {
            return Err(format!("the SSH key {} appears twice", printable(name)));
        }
        if k.private_key.trim().is_empty() {
            return Err(format!("the SSH key {} is empty", printable(name)));
        }
        bytes += k.private_key.len();
    }
    for t in tokens {
        if t.service().is_none() {
            return Err(format!("{} is not a git host the phone serves", printable(&t.host)));
        }
        if t.token.trim().is_empty() || t.token.len() > 4_000 || t.user.as_ref().is_some_and(|u| u.len() > 200) {
            return Err(format!("the token for {} is empty or too long", t.host));
        }
        bytes += t.token.len();
    }
    if bytes > MAX_STORE_BYTES {
        return Err(format!("at most {} KB of values in one store", MAX_STORE_BYTES / 1024));
    }
    Ok(())
}

/// [`validate_batch`] for environments alone.
pub fn validate_store(envs: &[EnvChanges]) -> Result<(), String> {
    if envs.is_empty() {
        return Err(format!("one store changes 1 to {MAX_STORE_ENVS} environments"));
    }
    validate_batch(envs, &[], &[])
}

/// What the desktop app sends the phone for `vault_secrets_store`: changes to one or more environments, in a box from
/// the app's key to the phone's key (as [`SecretToStore`]). One approval, one set of [`secrets_check`] digits.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecretsToStore {
    pub v: u32,
    /// [`TO_PHONE`].
    pub dir: String,
    pub nonce: String,
    /// Unix seconds, on the computer.
    pub created_at: i64,
    pub envs: Vec<EnvChanges>,
    /// SSH keys for the folder `Reins/ssh`.
    #[serde(default)]
    pub ssh: Vec<SshKeyToStore>,
    /// Git hosts' tokens for the phone's integrations.
    #[serde(default)]
    pub tokens: Vec<HostToken>,
}

impl std::fmt::Debug for SecretsToStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SecretsToStore")
            .field("nonce", &self.nonce)
            .field("envs", &self.envs)
            .field("ssh", &self.ssh)
            .field("tokens", &self.tokens)
            .finish_non_exhaustive()
    }
}

/// Four digits both the computer and the phone show for a whole store, so that a batch an AI on the computer sent in
/// the user's place does not pass for theirs. Like [`value_check`]: about 13 bits, not a secret on its own.
///
/// The canonical form, independent of order: one entry per value, `("env", "<project>/<env>", NAME, value)` for every
/// variable set, `("ssh", name, "", private_key)` for every SSH key, `("token", host, user or "", token)` for every
/// token; the entries sorted; SHA-256 over `reins-secrets-batch/1` then each part of each entry as its length (8 bytes,
/// big-endian) and its bytes; the first two bytes of the hash as a big-endian number, modulo 10 000, in four digits.
#[must_use]
pub fn batch_check(envs: &[EnvChanges], ssh: &[SshKeyToStore], tokens: &[HostToken]) -> String {
    let mut all: Vec<[String; 4]> = Vec::new();
    for e in envs {
        for (n, v) in &e.set {
            all.push(["env".to_owned(), e.item(), n.clone(), v.clone()]);
        }
    }
    for k in ssh {
        all.push(["ssh".to_owned(), k.name.clone(), String::new(), k.private_key.clone()]);
    }
    for t in tokens {
        all.push(["token".to_owned(), t.host.clone(), t.user.clone().unwrap_or_default(), t.token.clone()]);
    }
    all.sort_unstable();
    let mut h = Sha256::new();
    h.update(b"reins-secrets-batch/1");
    for entry in &mut all {
        for part in entry.iter_mut() {
            h.update((part.len() as u64).to_be_bytes());
            h.update(part.as_bytes());
            zeroize::Zeroize::zeroize(part);
        }
    }
    let d = h.finalize();
    format!("{:04}", u16::from_be_bytes([d[0], d[1]]) % 10_000)
}

/// [`batch_check`] of environments alone.
#[must_use]
pub fn secrets_check(envs: &[EnvChanges]) -> String {
    batch_check(envs, &[], &[])
}

/// What happened to one environment, as the phone acknowledges a `vault_secrets_store`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnvStored {
    pub project: String,
    pub env: String,
    /// The environment was made (its folder too, when it was missing).
    pub created: bool,
    pub added: u32,
    pub replaced: u32,
    pub removed: u32,
    pub renamed: u32,
    pub copied: u32,
    /// The environment's new version.
    pub revision: String,
    /// Why it was not changed (the rest of the store is unaffected).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// What happened to one SSH key of a store.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SshStored {
    pub name: String,
    /// `SHA256:…` of the key kept; empty when it failed.
    pub fingerprint: String,
    /// Why it was not kept (the rest of the store is unaffected).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// What happened to one token of a store.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenStored {
    pub host: String,
    /// The integration (`github`, …) and the account the token signs in as; empty when it failed.
    pub service: String,
    pub account: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// What the phone answers a `vault_secrets_store` it carried out, boxed from its inbox key (as [`StoreAck`]): one
/// entry per environment, SSH key and token, in the order sent.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecretsStoreAck {
    pub v: u32,
    /// [`TO_DESKTOP`].
    pub dir: String,
    pub nonce: String,
    pub envs: Vec<EnvStored>,
    #[serde(default)]
    pub ssh: Vec<SshStored>,
    #[serde(default)]
    pub tokens: Vec<TokenStored>,
}

/// What the desktop app sends the phone for `integration_token_store`: a git host's token (found by import: `gh`,
/// `.git-credentials`) for the phone's own integration (GitHub, GitLab, Codeberg, Bitbucket), boxed to the phone's key.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenToStore {
    pub v: u32,
    /// [`TO_PHONE`].
    pub dir: String,
    pub nonce: String,
    pub created_at: i64,
    /// `github`, `gitlab`, `codeberg` or `bitbucket`.
    pub service: String,
    /// The host it was found for (`github.com`).
    pub host: String,
    /// What the computer found out about the token's scopes (`repo`, `read:org`); shown, not trusted.
    #[serde(default)]
    pub scopes: Vec<String>,
    pub token: String,
}

impl Drop for TokenToStore {
    fn drop(&mut self) {
        zeroize::Zeroize::zeroize(&mut self.token);
    }
}

impl std::fmt::Debug for TokenToStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TokenToStore")
            .field("nonce", &self.nonce)
            .field("service", &self.service)
            .field("host", &self.host)
            .finish_non_exhaustive()
    }
}

/// What the phone answers an `integration_token_store` it carried out, boxed from its inbox key.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenStoreAck {
    pub v: u32,
    /// [`TO_DESKTOP`].
    pub dir: String,
    pub nonce: String,
    pub service: String,
    /// The account the token signs in as, as the integration lists it.
    pub account: String,
}

/// One environment as `vault_secrets_list` names it: never a value.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnvListing {
    pub name: String,
    /// The environment's version (the vault item's revision date).
    pub revision: String,
    /// Its variables' names, sorted.
    pub variables: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectListing {
    pub name: String,
    pub envs: Vec<EnvListing>,
}

/// What the phone answers `vault_secrets_list`, boxed from its inbox key: the projects, environments and variable
/// names (of one project when the call names it).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecretsListing {
    pub v: u32,
    /// [`TO_DESKTOP`].
    pub dir: String,
    pub nonce: String,
    pub projects: Vec<ProjectListing>,
}

/// "Drop the secrets you hold": sent by the phone (Revoke on all computers) to every paired desktop app, sealed to its
/// key, picked up by its daemon. Anyone who knows the app's public key could make one, so it only ever takes leases
/// away. `project`/`env` narrow it; neither: every lease.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LeaseRevocation {
    pub v: u32,
    /// [`TO_DESKTOP`].
    pub dir: String,
    /// Random: the same revocation is applied once.
    pub id: String,
    /// Unix seconds, on the phone.
    pub issued_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub env: Option<String>,
}

/// The most revocations the server keeps for one computer until its daemon picks them up, and for how long.
pub const MAX_REVOCATIONS: usize = 20;
pub const REVOCATION_TTL_SECS: i64 = 24 * 3_600;

/// One [`LeaseRevocation`] sealed to one paired computer (`POST /reins/api/desktop/revocations`, from the phone).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SealedRevocation {
    pub connection_id: String,
    /// The [`LeaseRevocation`] JSON in a sealed box to the computer's pinned key (base64url).
    pub sealed: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RevocationPost {
    pub items: Vec<SealedRevocation>,
}

/// What a computer's daemon picks up (`GET /reins/desktop/revocations`): the sealed revocations for it, oldest first.
/// Picking them up removes them from the server.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RevocationMail {
    pub items: Vec<String>,
}

/// SHA-256 (lowercase hex) binding an approval to exactly what git sends: the repository, every `old new ref`
/// command in order, the pack bytes and the push options.
#[must_use]
pub fn push_digest(
    repo: &str,
    commands: &[(String, String, String)],
    pack_sha256_hex: &str,
    options: &[String],
) -> String {
    let mut h = Sha256::new();
    h.update(b"reins-git-push/1\n");
    h.update(format!("repo {repo}\n").as_bytes());
    for (old, new, name) in commands {
        h.update(format!("update {old} {new} {name}\n").as_bytes());
    }
    h.update(format!("pack {pack_sha256_hex}\n").as_bytes());
    for o in options {
        h.update(format!("option {o}\n").as_bytes());
    }
    hex(&h.finalize())
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes.iter().fold(String::with_capacity(bytes.len() * 2), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

/// A desktop app public key as sent (base64url of 32 bytes, no padding).
#[must_use]
pub fn decode_key(key: &str) -> Option<[u8; 32]> {
    let bytes = BASE64URL_NOPAD.decode(key.as_bytes()).ok()?;
    bytes.try_into().ok()
}

#[must_use]
pub fn encode_key(key: &[u8; 32]) -> String {
    BASE64URL_NOPAD.encode(key)
}

/// Eight digits the user compares on the computer and on the phone when pairing the desktop app ("4821 9930").
#[must_use]
pub fn key_fingerprint(key: &str) -> Option<String> {
    eight_digits(b"reins-desktop-key/1", key)
}

/// Twelve digits the user types on the computer before it first seals a secret to the phone's key (`reins vault add`):
/// "4821-9930-1274". Longer than [`key_fingerprint`] and written with dashes, so that the desktop app's own eight
/// digits are never taken for it; the key only travels sealed, so a server cannot know which twelve to aim for.
#[must_use]
pub fn phone_key_fingerprint(key: &str) -> Option<String> {
    let raw = decode_key(key)?;
    let mut h = Sha256::new();
    h.update(b"reins-phone-key/2");
    h.update(raw);
    let d = h.finalize();
    let n = u64::from_be_bytes([0, 0, d[0], d[1], d[2], d[3], d[4], d[5]]) % 1_000_000_000_000;
    Some(format!("{:04}-{:04}-{:04}", n / 100_000_000, n / 10_000 % 10_000, n % 10_000))
}

fn eight_digits(domain: &[u8], key: &str) -> Option<String> {
    let raw = decode_key(key)?;
    let mut h = Sha256::new();
    h.update(domain);
    h.update(raw);
    let d = h.finalize();
    let n = u64::from_be_bytes([0, 0, 0, d[0], d[1], d[2], d[3], d[4]]) % 100_000_000;
    Some(format!("{:04} {:04}", n / 10_000, n % 10_000))
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: &str = "1111111111111111111111111111111111111111";
    const B: &str = "2222222222222222222222222222222222222222";

    fn update(name: &str, old: &str, new: &str, ff: Option<bool>) -> RefUpdate {
        let change = match (old == ZERO_OID, new == ZERO_OID) {
            (true, _) => RefChange::Create,
            (_, true) => RefChange::Delete,
            _ => RefChange::Update,
        };
        RefUpdate {
            name: name.to_owned(),
            change,
            old: old.to_owned(),
            new: new.to_owned(),
            fast_forward: ff,
            commit_count: 0,
            commits: vec![],
            files_changed: 0,
            files: vec![],
            additions: None,
            deletions: None,
        }
    }

    fn commit(sha: &str) -> CommitInfo {
        CommitInfo {
            sha: sha.to_owned(),
            subject: "x".into(),
            author: "a <a@b>".into(),
            date: 0,
            files_changed: None,
            files: vec![],
            additions: None,
            deletions: None,
        }
    }

    fn summary(updates: Vec<RefUpdate>) -> PushSummary {
        PushSummary {
            updates,
            ..PushSummary::default()
        }
    }

    #[test]
    fn one_branch_is_its_own_resource_and_fast_forwards_can_be_remembered() {
        let s = summary(vec![update("refs/heads/feature/x", A, B, Some(true))]);
        s.validate().unwrap();
        assert_eq!(s.resource("o/r"), "o/r@feature/x");
        assert_eq!(s.tool(), GIT_PUSH_TOOL);
        assert!(!s.once_only());
    }

    #[test]
    fn force_pushes_deletions_moved_tags_and_several_branches_are_asked_every_time() {
        for s in [
            summary(vec![update("refs/heads/main", A, B, Some(false))]),
            summary(vec![update("refs/heads/main", A, B, None)]),
            summary(vec![update("refs/heads/old", A, ZERO_OID, None)]),
            summary(vec![update("refs/tags/v1", A, B, Some(true))]),
            summary(vec![update("refs/heads/a", ZERO_OID, B, None), update("refs/heads/b", ZERO_OID, B, None)]),
            summary(vec![update("refs/notes/commits", A, B, Some(true))]),
        ] {
            s.validate().unwrap();
            assert!(s.once_only(), "{s:?}");
        }
        let tags = summary(vec![update("refs/tags/v1", ZERO_OID, A, None), update("refs/tags/v2", ZERO_OID, B, None)]);
        assert!(!tags.once_only());
        assert_eq!(tags.tool(), GIT_TAG_PUSH_TOOL);
        assert_eq!(tags.resource("o/r"), "o/r");
        let branch_and_tag =
            summary(vec![update("refs/heads/main", A, B, Some(true)), update("refs/tags/v1", ZERO_OID, B, None)]);
        assert_eq!(branch_and_tag.tool(), GIT_PUSH_TOOL);
        assert_eq!(branch_and_tag.resource("o/r"), "o/r");
        assert!(!branch_and_tag.once_only());
    }

    #[test]
    fn validation_rejects_lies_and_garbage() {
        let mut wrong_kind = update("refs/heads/main", A, B, Some(true));
        wrong_kind.change = RefChange::Create;
        let mut overcount = update("refs/heads/main", A, B, Some(true));
        overcount.commits.push(commit(A));
        for bad in [
            summary(vec![]),
            summary(vec![wrong_kind]),
            summary(vec![overcount]),
            summary(vec![update("main", A, B, Some(true))]),
            summary(vec![update("refs/heads/a\u{7}", A, B, Some(true))]),
            summary(vec![update("refs/heads/a", "ABC", B, Some(true))]),
            summary(vec![update("refs/heads/a", A, B, Some(true)), update("refs/heads/a", A, B, Some(true))]),
            summary(vec![update("refs/heads/a", ZERO_OID, ZERO_OID, None)]),
        ] {
            assert!(bad.validate().is_err(), "{bad:?}");
        }
    }

    #[test]
    fn oversized_summaries_are_shortened_but_keep_their_counts() {
        let mut big = update("refs/heads/main", A, B, Some(true));
        big.files = (0..MAX_FILES)
            .map(|i| {
                FileChange::new(
                    format!("{}/{i}.rs", "deep/".repeat(150)),
                    FileStatus::Modified,
                    Some(1),
                    Some(1),
                    false,
                )
            })
            .collect();
        big.files_changed = 4_000;
        let mut s = summary(vec![big]);
        assert!(serde_json::to_vec(&s).unwrap().len() > MAX_SUMMARY_BYTES);
        assert!(s.fit(MAX_SUMMARY_BYTES));
        assert!(serde_json::to_vec(&s).unwrap().len() <= MAX_SUMMARY_BYTES);
        assert!(!s.updates[0].files.is_empty());
        assert_eq!(s.updates[0].files_changed, 4_000);
        assert_eq!(s.notes.len(), 1);
        s.validate().unwrap();
        let mut small = summary(vec![update("refs/heads/main", A, B, Some(true))]);
        assert!(small.fit(MAX_SUMMARY_BYTES));
        assert!(small.notes.is_empty(), "nothing trimmed, nothing said");
        assert!(!small.fit(10));
    }

    fn with_patch(file: &str, diff: &str) -> FileChange {
        FileChange {
            patch: Some(diff.to_owned()),
            ..FileChange::new(file.to_owned(), FileStatus::Modified, Some(1), Some(1), false)
        }
    }

    #[test]
    fn commits_carry_their_own_files_and_patches_within_bounds() {
        let mut u = update("refs/heads/main", A, B, Some(true));
        u.commit_count = 1;
        let mut c = commit(B);
        c.files_changed = Some(2);
        c.files = vec![
            with_patch("a.rs", "@@ -1,2 +1,2 @@\n context\n-old\n+new\n\\ No newline at end of file"),
            FileChange::new("logo.png".into(), FileStatus::Added, None, None, true),
        ];
        u.commits.push(c);
        summary(vec![u.clone()]).validate().unwrap();

        // Old summaries (no commit files, no patches) still read, and new fields stay out of the JSON when empty.
        let old = r#"{"sha":"1111111111111111111111111111111111111111","subject":"x","author":"a <a@b>","date":0}"#;
        let parsed: CommitInfo = serde_json::from_str(old).unwrap();
        assert_eq!((parsed.files_changed, parsed.files.len()), (None, 0));
        assert_eq!(serde_json::to_string(&parsed).unwrap(), old);

        let bad = |edit: &dyn Fn(&mut CommitInfo)| {
            let mut v = u.clone();
            edit(&mut v.commits[0]);
            summary(vec![v]).validate().is_err()
        };
        assert!(bad(&|c| c.files_changed = Some(1)), "more files listed than counted");
        assert!(bad(&|c| c.files[0].patch = Some("no prefix".into())));
        assert!(bad(&|c| c.files[0].patch = Some("+a\u{1b}[31m".into())), "control characters");
        assert!(bad(&|c| c.files[0].patch = Some(format!("+{}", "x".repeat(MAX_PATCH_LINE + 1)))));
        assert!(bad(&|c| c.files[0].patch = Some(vec!["+a"; MAX_PATCH_LINES + 1].join("\n"))));
        assert!(
            bad(&|c| {
                c.files[1].patch_cut = true;
            }),
            "cut without a patch"
        );
        assert!(bad(&|c| {
            c.files_changed = Some(500);
            c.files = vec![FileChange::new("f".into(), FileStatus::Added, None, None, false); MAX_COMMIT_FILES + 1];
        }));
        assert!(!bad(&|c| c.files[0].patch = Some("+\ttabs are fine".into())));
    }

    #[test]
    fn fitting_drops_patches_before_any_list() {
        let mut u = update("refs/heads/main", A, B, Some(true));
        u.commit_count = 3;
        let patch = vec!["+".to_owned() + &"y".repeat(200); MAX_PATCH_LINES].join("\n");
        for _ in 0..3 {
            let mut c = commit(B);
            c.files_changed = Some(2);
            c.files = vec![with_patch("big.rs", &patch), with_patch("small.rs", "@@ -1 +1 @@\n-a\n+b")];
            u.commits.push(c);
        }
        let mut s = summary(vec![u]);
        let full = serde_json::to_vec(&s).unwrap().len();
        assert!(full > 200_000);
        assert!(s.fit(full - 50_000));
        let u = &s.updates[0];
        assert_eq!(u.commits.len(), 3, "every commit stays");
        assert!(u.commits.iter().all(|c| c.files.len() == 2), "and every file");
        assert!(u.commits.iter().all(|c| c.files[1].patch.is_some()), "small patches stay");
        assert!(u.commits.iter().any(|c| c.files[0].patch.is_none()), "large ones go first");
        assert_eq!(s.notes, ["Some changes are not shown line by line: the push is too large."]);
        s.validate().unwrap();
        // Still too large without any patch: the lists shrink too.
        assert!(s.fit(600));
        assert!(serde_json::to_vec(&s).unwrap().len() <= 600);
        assert_eq!(s.notes, ["Some commits and files are not listed: the push is too large to show in full."]);
    }

    #[test]
    fn digest_covers_every_part() {
        let cmds = vec![(A.to_owned(), B.to_owned(), "refs/heads/main".to_owned())];
        let d = push_digest("o/r", &cmds, "ab", &[]);
        assert_eq!(d.len(), 64);
        assert_eq!(d, push_digest("o/r", &cmds, "ab", &[]));
        assert_ne!(d, push_digest("o/x", &cmds, "ab", &[]));
        assert_ne!(d, push_digest("o/r", &cmds, "ac", &[]));
        assert_ne!(d, push_digest("o/r", &cmds, "ab", &["ci.skip".to_owned()]));
        let other = vec![(A.to_owned(), B.to_owned(), "refs/heads/dev".to_owned())];
        assert_ne!(d, push_digest("o/r", &other, "ab", &[]));
    }

    #[test]
    fn relayed_text_cannot_steer_the_terminal() {
        assert_eq!(printable("ok\u{1b}[2J\u{1b}]0;title\u{7}done"), "ok [2J ]0;title done");
        assert_eq!(printable("a\u{202e}b\nc"), "a b c");
        assert_eq!(printable(&"x".repeat(400)).chars().count(), 300);
    }

    #[test]
    fn fingerprints_are_stable_eight_digits() {
        let key = encode_key(&[7u8; 32]);
        let fp = key_fingerprint(&key).unwrap();
        assert_eq!(fp.len(), 9);
        assert_eq!(fp, key_fingerprint(&key).unwrap());
        assert_ne!(fp, key_fingerprint(&encode_key(&[8u8; 32])).unwrap());
        assert!(key_fingerprint("short").is_none());
        assert_eq!(decode_key(&key), Some([7u8; 32]));
    }

    fn changes(project: &str, env: &str, set: &[(&str, &str)]) -> EnvChanges {
        let mut c = EnvChanges::new(project, env);
        c.set = set.iter().map(|(n, v)| ((*n).to_owned(), (*v).to_owned())).collect();
        c
    }

    #[test]
    fn the_batch_check_covers_every_value_name_and_environment_in_any_order() {
        let a = changes("acme", "dev", &[("A", "1"), ("B", "2")]);
        let check = secrets_check(std::slice::from_ref(&a));
        assert_eq!(check.len(), 4);
        assert_eq!(check, secrets_check(&[changes("acme", "dev", &[("B", "2"), ("A", "1")])]));
        assert_ne!(check, secrets_check(&[changes("acme", "dev", &[("A", "1"), ("B", "3")])]));
        assert_ne!(check, secrets_check(&[changes("acme", "prod", &[("A", "1"), ("B", "2")])]));
        assert_ne!(check, secrets_check(&[changes("acme", "dev", &[("A", "1"), ("C", "2")])]));
        // Boundaries are part of the hash: "A"+"12" is not "A1"+"2".
        assert_ne!(
            secrets_check(&[changes("acme", "dev", &[("A", "12")])]),
            secrets_check(&[changes("acme", "dev", &[("A1", "2")])])
        );
        assert!(!format!("{a:?}").contains("\"1\""), "values never show in Debug");
    }

    fn ssh(name: &str, key: &str) -> SshKeyToStore {
        SshKeyToStore {
            name: name.into(),
            private_key: key.into(),
        }
    }

    fn token(host: &str, value: &str) -> HostToken {
        HostToken {
            host: host.into(),
            user: None,
            scopes: vec![],
            token: value.into(),
        }
    }

    #[test]
    fn the_batch_check_covers_ssh_keys_and_tokens_in_any_order() {
        let envs = [changes("acme", "dev", &[("A", "1")])];
        let keys = [ssh("deploy", "KEY-1"), ssh("other", "KEY-2")];
        let tokens = [token("github.com", "ghp_1")];
        let check = batch_check(&envs, &keys, &tokens);
        assert_eq!(check, batch_check(&envs, &[ssh("other", "KEY-2"), ssh("deploy", "KEY-1")], &tokens));
        assert_ne!(check, batch_check(&envs, &keys, &[token("github.com", "ghp_2")]));
        assert_ne!(check, batch_check(&envs, &[ssh("deploy", "KEY-1")], &tokens));
        assert_ne!(check, batch_check(&envs, &keys, &[]));
        assert_eq!(secrets_check(&envs), batch_check(&envs, &[], &[]));
        validate_batch(&[], &keys, &tokens).unwrap();
        assert!(validate_batch(&[], &[], &[]).is_err());
        assert!(validate_batch(&[], &[ssh("a", "K"), ssh("a", "K")], &[]).is_err());
        assert!(validate_batch(&[], &[], &[token("example.com", "t")]).is_err());
        assert_eq!(token("GitHub.com", "t").service(), Some("github"));
        assert!(!format!("{:?} {:?}", keys[0], tokens[0]).contains("KEY-1"));
    }

    #[test]
    fn stores_are_checked_before_they_are_sealed() {
        validate_store(&[changes("acme", "dev", &[("A", "1")])]).unwrap();
        let mut copy = changes("acme", "staging", &[]);
        copy.copy_from = Some("dev".into());
        validate_store(&[copy]).unwrap();
        let mut bad_copy = changes("acme", "dev", &[]);
        bad_copy.copy_from = Some("dev".into());
        let mut renamed = changes("acme", "dev", &[]);
        renamed.rename = vec![("A".into(), "1B".into())];
        let mut twice = changes("acme", "dev", &[("A", "1")]);
        twice.unset = vec!["A".into()];
        for (bad, why) in [
            (vec![], "empty"),
            (vec![changes("acme", "dev", &[])], "nothing to change"),
            (vec![changes("a/b", "dev", &[("A", "1")])], "name"),
            (vec![changes("acme", "dev", &[("1A", "1")])], "variable"),
            (vec![changes("acme", "dev", &[("A", "1"), ("A", "2")])], "twice"),
            (vec![twice], "set and unset"),
            (vec![renamed], "rename target"),
            (vec![bad_copy], "copy from itself"),
            (vec![changes("acme", "dev", &[("A", "1")]), changes("acme", "dev", &[("B", "1")])], "env twice"),
            (vec![changes("acme", "dev", &[("A", &"x".repeat(MAX_STORE_BYTES))])], "too large"),
        ] {
            assert!(validate_store(&bad).is_err(), "{why}");
        }
        assert!(name_ok("acme-web") && !name_ok(".x") && !name_ok("a b"));
        assert!(var_ok("_X1") && !var_ok("1X") && !var_ok("A-B"));
    }

    fn argv(words: &[&str]) -> Vec<String> {
        words.iter().map(|w| (*w).to_owned()).collect()
    }

    fn prefix(program: &str, args: &[&str]) -> CommandScope {
        CommandScope {
            program: program.to_owned(),
            args: argv(args),
            prefix: true,
        }
    }

    #[test]
    fn commands_match_as_whole_arguments() {
        let npm = "/usr/bin/npm";
        let run = prefix(npm, &["run", "test"]);
        run.validate().unwrap();
        assert!(run.matches(npm, &argv(&["run", "test", "--silent", "unit"])));
        assert!(run.matches(npm, &argv(&["run", "test"])), "zero extra arguments");
        assert!(!run.matches(npm, &argv(&["run", "testx"])), "never a partial argument");
        assert!(!run.matches(npm, &argv(&["exec", "run", "test"])));
        assert!(!run.matches("/home/me/evil/npm", &argv(&["run", "test"])), "another npm on PATH");
        let exact = CommandScope::exact(npm, &argv(&["run", "test"]));
        assert!(exact.matches(npm, &argv(&["run", "test"])));
        assert!(!exact.matches(npm, &argv(&["run", "dev"])));
        assert!(!exact.matches(npm, &argv(&["run", "test", "--", "x"])));
        // An argument with a space is one argument: it is never split into two.
        let spaced = CommandScope::exact("/bin/echo", &argv(&["a b"]));
        assert!(spaced.matches("/bin/echo", &argv(&["a b"])) && !spaced.matches("/bin/echo", &argv(&["a", "b"])));
        assert_eq!(spaced.shown(), "echo 'a b'");
        assert_eq!(run.shown(), "npm run test *");
        assert!(exact.within(&run) && !run.within(&exact));
    }

    #[test]
    fn a_prefix_is_allowed_only_for_programs_and_subcommands_on_the_allow_list() {
        for (program, args) in [
            ("/usr/bin/cargo", vec!["build"]),
            ("/usr/bin/cargo", vec!["test"]),
            ("/usr/bin/cargo", vec!["run"]),
            ("/usr/bin/cargo", vec!["check"]),
            ("/usr/bin/cargo", vec!["clippy"]),
            ("/usr/bin/npm", vec!["run", "dev"]),
            ("/usr/lib/node_modules/npm/bin/npm-cli.js", vec!["run", "dev"]),
            ("/usr/bin/pnpm", vec!["run", "build:prod"]),
            ("/usr/bin/yarn", vec!["test"]),
            ("/usr/bin/make", vec!["deploy"]),
            ("/usr/bin/go", vec!["test"]),
            ("/usr/bin/pytest", vec![]),
            ("/usr/bin/uv", vec!["run", "pytest"]),
            ("/home/me/app/bin/rails", vec!["server"]),
            ("/usr/bin/docker", vec!["compose", "up"]),
            ("NPM", vec!["run", "dev"]),
        ] {
            prefix(program, &args).validate().unwrap_or_else(|e| panic!("{program} {args:?}: {e}"));
        }
        // Everything else is exact only: shells, interpreters, launchers, wrappers, and what the list does not name.
        for (program, args) in [
            ("/bin/bash", vec!["-c"]),
            ("/usr/bin/python3.12", vec!["manage.py"]),
            ("/usr/bin/node", vec!["server.js"]),
            ("/usr/bin/npx", vec!["vite"]),
            ("/usr/bin/env", vec!["FOO=1"]),
            ("/usr/bin/sudo", vec!["make"]),
            ("/usr/bin/ssh", vec!["host"]),
            ("/usr/bin/docker", vec!["run"]),
            ("/usr/bin/uv", vec!["run"]),
            ("/usr/bin/deno", vec!["run"]),
            ("C:\\Windows\\System32\\cmd.exe", vec!["/c"]),
            ("/usr/bin/npm", vec!["exec"]),
            ("/usr/bin/npm", vec!["run"]),
            ("/usr/bin/npm", vec!["run", "--script-shell"]),
            ("/usr/bin/npm", vec![]),
            ("/usr/bin/pnpm", vec!["dlx"]),
            ("/usr/bin/cargo", vec!["+nightly"]),
            ("/usr/bin/cargo", vec!["install"]),
            ("/usr/bin/make", vec!["-f"]),
            ("/usr/bin/make", vec!["SHELL=/tmp/x"]),
            ("/usr/bin/git", vec!["push"]),
            ("/opt/tools/runner", vec!["x"]),
            ("/usr/bin/n\u{440}m", vec!["run", "dev"]),
            ("C:\\PROGRA~1\\nodejs\\NPM~1.CMD", vec!["run", "dev"]),
        ] {
            let p = prefix(program, &args);
            assert!(p.validate().is_err(), "{program} {args:?}");
            let mut more = argv(&args);
            more.push("anything".to_owned());
            assert!(!p.matches(program, &more), "a refused prefix covers nothing: {program} {args:?}");
        }
        CommandScope::exact("/bin/bash", &argv(&["-c", "make"])).validate().unwrap();
    }

    #[test]
    fn what_the_star_stands_for_runs_no_other_code() {
        let ok = |program: &str, fixed: &[&str], tail: &[&str]| {
            prefix(program, fixed).matches(program, &argv(&[fixed, tail].concat()))
        };
        // Plain words and the grammar's safe options.
        assert!(ok("/usr/bin/cargo", &["test"], &["--release", "-p", "core", "some_test", "-j8", "--features=x"]));
        assert!(ok("/usr/bin/npm", &["run", "dev"], &["--silent", "src/app.ts"]));
        assert!(ok("/usr/bin/make", &["build"], &["-j4", "docs"]));
        assert!(ok("/usr/bin/go", &["test"], &["-race", "-run=TestX", "./pkg/..."]));
        assert!(ok("/usr/bin/pytest", &[], &["-k", "login and not slow", "tests/test_auth.py"]));
        // Options that run other code or read other configuration, assignments, `--`, paths out of the project.
        for (program, fixed, tail) in [
            ("/usr/bin/cargo", vec!["test"], vec!["--config", "build.rustc-wrapper='/tmp/x'"]),
            ("/usr/bin/cargo", vec!["test"], vec!["-Zunstable-options"]),
            ("/usr/bin/cargo", vec!["build"], vec!["+nightly"]),
            ("/usr/bin/cargo", vec!["build"], vec!["--manifest-path", "x"]),
            ("/usr/bin/cargo", vec!["run"], vec!["--", "--anything"]),
            ("/usr/bin/npm", vec!["run", "dev"], vec!["--script-shell=/tmp/sh"]),
            ("/usr/bin/npm", vec!["run", "dev"], vec!["--node-options=--require=/tmp/x.js"]),
            ("/usr/bin/npm", vec!["run", "dev"], vec!["--prefix", "/tmp"]),
            ("/usr/bin/make", vec!["build"], vec!["-f", "evil.mk"]),
            ("/usr/bin/make", vec!["build"], vec!["-C", "x"]),
            ("/usr/bin/make", vec!["build"], vec!["--eval=x"]),
            ("/usr/bin/make", vec!["build"], vec!["SHELL=/tmp/sh"]),
            ("/usr/bin/pytest", vec![], vec!["-p", "evil"]),
            ("/usr/bin/pytest", vec![], vec!["--rootdir=/tmp"]),
            ("/usr/bin/pytest", vec![], vec!["-c", "x.ini"]),
            ("/usr/bin/pytest", vec![], vec!["--pyargs", "x"]),
            ("/usr/bin/pytest", vec![], vec!["/tmp/evil_test.py"]),
            ("/usr/bin/pytest", vec![], vec!["../other/test.py"]),
            ("/usr/bin/pytest", vec![], vec!["~/x.py"]),
            ("/usr/bin/go", vec!["test"], vec!["-exec", "/tmp/x"]),
            ("/usr/bin/go", vec!["build"], vec!["-toolexec=/tmp/x"]),
            ("/usr/bin/go", vec!["build"], vec!["-ldflags=-X main.v=1"]),
            ("/usr/bin/docker", vec!["compose", "up"], vec!["-f", "/tmp/c.yml"]),
        ] {
            assert!(!ok(program, &fixed, &tail), "{program} {fixed:?} {tail:?}");
            assert!(tail_refusal(program, &argv(&tail)).is_some(), "{program} {tail:?}");
        }
    }

    #[test]
    fn the_offers_are_the_command_then_each_allowed_prefix() {
        let c = CommandScope::exact("/usr/bin/npm", &argv(&["run", "dev", "--silent"]));
        let shown: Vec<String> = c.offers().iter().map(CommandScope::shown).collect();
        assert_eq!(shown, ["npm run dev --silent", "npm run dev *"]);
        let plain = CommandScope::exact("/usr/bin/npm", &argv(&["run", "dev"]));
        assert_eq!(
            plain.offers().iter().map(CommandScope::shown).collect::<Vec<_>>(),
            ["npm run dev", "npm run dev *"]
        );
        let shell = CommandScope::exact("/bin/sh", &argv(&["-c", "x"]));
        assert_eq!(shell.offers(), vec![shell], "a shell is only ever exact");
        let cargo = CommandScope::exact("/usr/bin/cargo", &argv(&["run", "--", "x"]));
        assert_eq!(cargo.offers().len(), 1, "a prefix that would not cover it is not offered");
        let pytest = CommandScope::exact("/usr/bin/pytest", &argv(&["-x", "tests"]));
        assert_eq!(
            pytest.offers().iter().map(CommandScope::shown).collect::<Vec<_>>(),
            ["pytest -x tests", "pytest *"]
        );
    }

    #[test]
    fn a_program_is_judged_by_every_name_and_any_doubt_means_exact() {
        for (path, name) in [
            ("/usr/lib/node_modules/npm/bin/npm-cli.js", "npm"),
            ("/home/me/.nvm/versions/node/v22.1.0/lib/node_modules/npm/bin/npx-cli.js", "npx"),
            ("/usr/lib/node_modules/yarn/bin/yarn.js", "yarn"),
            ("/usr/lib/node_modules/pnpm/bin/pnpm.cjs", "pnpm"),
            ("C:\\nodejs\\npm.cmd", "npm"),
            ("/usr/bin/python3.12", "python"),
            ("node22", "node"),
            ("bash5.2", "bash"),
            ("pwsh-preview", "pwsh"),
            ("php8.3", "php"),
            ("CMD.EXE", "cmd"),
            ("python.exe.", "python"),
            ("bash. . ", "bash"),
            ("NPM", "npm"),
            ("-cli", "-cli"),
        ] {
            assert_eq!(program_name(path), name, "{path}");
        }
        let npm = "/usr/lib/node_modules/npm/bin/npm-cli.js";
        assert!(prefix_refusal_as(&["npm", "./npm"], npm, &argv(&["run", "dev"])).is_none(), "the same program");
        // Typed or linked by another name: which program runs is in doubt.
        assert!(prefix_refusal_as(&["deploy"], "/usr/bin/make", &argv(&["build"])).is_some());
        assert!(prefix_refusal_as(&["npm", "env", "npm-cli.js"], npm, &argv(&["run", "dev"])).is_some());
        assert!(prefix_refusal_as(&["n\u{440}m"], npm, &argv(&["run", "dev"])).is_some());
        let linked = CommandScope::exact("/usr/bin/make", &argv(&["build", "-j4"]));
        assert_eq!(linked.offers().len(), 2);
        assert_eq!(linked.offers_as(&["deploy"]).len(), 1);
        for name in ["/usr/bin/n\u{440}m", "C:\\PROGRA~1\\nodejs\\NPM~1.CMD", "PYTHON~1.EXE", "/usr/bin/", ""] {
            assert!(doubtful_name(name), "{name:?}");
        }
        assert!(!doubtful_name("/usr/local/bin/my-tool_2.0+x"));
    }

    #[test]
    fn an_exact_command_is_remembered_only_when_all_of_it_shows() {
        let short = CommandScope::exact("/bin/bash", &argv(&["-c", "make deploy"]));
        assert!(short.rememberable(&[]));
        assert_eq!(short.offers(), vec![short]);
        let long = CommandScope::exact("/usr/bin/python3", &argv(&["-c", &"x = 1; ".repeat(60)]));
        assert!(!long.rememberable(&[]));
        assert!(long.offers().is_empty(), "approved once at a time");
        long.validate().unwrap();
    }

    #[test]
    fn runners_are_a_person_or_a_named_agent() {
        for ok in ["person", "agent:claude-code", "agent:codex", "agent:gemini_cli"] {
            assert!(runner_ok(ok), "{ok}");
        }
        for bad in ["", "agent:", "agent:Claude", "agent:a b", "robot", "person "] {
            assert!(!runner_ok(bad), "{bad}");
        }
    }

    #[test]
    fn a_secret_grant_without_an_environment_reads_as_before() {
        let old = r#"{"v":1,"nonce":"n","secrets":[["A/password","x"]],"expires_at":5}"#;
        let g: SecretGrant = serde_json::from_str(old).unwrap();
        assert_eq!((&g.project, &g.env, &g.revision, &g.command), (&None, &None, &None, &None));
        assert_eq!((&g.runner, &g.root), (&None, &None));
        assert_eq!(serde_json::to_string(&g).unwrap(), old);
    }

    #[test]
    fn credential_debug_never_shows_the_token() {
        let g = CredentialGrant {
            v: 1,
            nonce: "n".into(),
            digest: None,
            repo: "o/r".into(),
            access: "read".into(),
            username: "x-access-token".into(),
            token: "ghp_secret".into(),
            expires_at: 0,
        };
        assert!(!format!("{g:?}").contains("ghp_secret"));
    }
}
