//! The Rewarden desktop app and the phone: which git access the app asks for, what a push does (built by the app from
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
            }
            for f in &u.files {
                clean(&f.path, MAX_PATH, "a file path")?;
            }
        }
        Ok(())
    }

    /// Shortens the commit and file lists (longest first, the counts stay) until the JSON is at most `max` bytes, and
    /// says so in a note. `false` when even empty lists do not fit.
    pub fn fit(&mut self, max: usize) -> bool {
        let size = |s: &Self| serde_json::to_vec(s).map_or(usize::MAX, |v| v.len());
        let mut trimmed = false;
        while size(self) > max {
            let Some((u, files)) = self
                .updates
                .iter()
                .enumerate()
                .flat_map(|(i, u)| [(u.files.len(), i, true), (u.commits.len(), i, false)])
                .max_by_key(|(len, _, _)| *len)
                .filter(|(len, _, _)| *len > 0)
                .map(|(_, i, files)| (i, files))
            else {
                return false;
            };
            let update = &mut self.updates[u];
            if files {
                update.files.truncate(update.files.len() / 2);
            } else {
                update.commits.truncate(update.commits.len() / 2);
            }
            trimmed = true;
        }
        if trimmed && self.notes.len() < MAX_NOTES {
            self.notes.push("Some commits and files are not listed: the push is too large to show in full.".to_owned());
        }
        size(self) <= max
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

/// What the phone seals to the desktop app for `vault_secret_release`: each requested `item/field` and its value.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecretGrant {
    pub v: u32,
    pub nonce: String,
    /// In the order requested.
    pub secrets: Vec<(String, String)>,
    /// Unix seconds; the desktop app forgets them then.
    pub expires_at: i64,
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
    h.update(b"rewarden-git-push/1\n");
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
    let raw = decode_key(key)?;
    let mut h = Sha256::new();
    h.update(b"rewarden-desktop-key/1");
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
        overcount.commits.push(CommitInfo {
            sha: A.into(),
            subject: "x".into(),
            author: "a <a@b>".into(),
            date: 0,
        });
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
            .map(|i| FileChange {
                path: format!("{}/{i}.rs", "deep/".repeat(150)),
                status: FileStatus::Modified,
                additions: Some(1),
                deletions: Some(1),
                binary: false,
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
    fn fingerprints_are_stable_eight_digits() {
        let key = encode_key(&[7u8; 32]);
        let fp = key_fingerprint(&key).unwrap();
        assert_eq!(fp.len(), 9);
        assert_eq!(fp, key_fingerprint(&key).unwrap());
        assert_ne!(fp, key_fingerprint(&encode_key(&[8u8; 32])).unwrap());
        assert!(key_fingerprint("short").is_none());
        assert_eq!(decode_key(&key), Some([7u8; 32]));
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
