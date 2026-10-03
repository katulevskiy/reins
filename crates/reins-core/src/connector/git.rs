//! Git on the user's computer through the Reins desktop app, for every git host (GitHub, GitLab, Codeberg,
//! Bitbucket). The app asks for a fetch (read) or a push (write); the answer is the account's token as a
//! [`CredentialGrant`] sealed to the app's key (pinned when it was paired, checked by the flow before this runs),
//! together with the request's nonce and, for a push, the digest of exactly what is pushed. What differs per host (how
//! the repository is looked up, the HTTP user name that goes with the token) stays with the host; this module holds the
//! rest.
//!
//! Permissions: a fetch is a read of the repository; a push is a write of `repo@branch` for one branch, else of the
//! repository (see `PushSummary::resource`), asked every time when it rewrites or drops history. A repository is in
//! its owner (GitLab: in each group above it), so a permission can be given for those too.

use reins_proto::connector::ConnectorCall;
use reins_proto::desktop::{
    CredentialGrant, FETCH_LEASE_SECS, GIT_PUSH_OP, GIT_TAG_PUSH_OP, PUSH_LEASE_SECS, PushSummary, RefChange,
    RefUpdate, SEALED_FIELD,
};
use serde::Deserialize as _;
use serde_json::{Map, Value, json};
use zeroize::Zeroizing;

use super::github::ref_ok;
use super::sealed::{client_key, nonce_arg, seal};
use super::{Item, Preview};
use crate::store::unix_now;
use crate::{CoreError, text};

/// How many commit subjects a push preview lists.
const PREVIEW_COMMITS: usize = 5;
/// The longest repository path (the tools' `repo` argument allows this much).
const MAX_REPO: usize = 250;
/// The most groups a GitLab path may have above the repository.
const MAX_DEPTH: usize = 20;

fn bad(message: impl Into<String>) -> CoreError {
    CoreError::service(message)
}

/// Whether the call is a push or a tag push (the same operations for every host).
pub(crate) fn is_push(call: &ConnectorCall) -> bool {
    matches!(call.op.as_str(), GIT_PUSH_OP | GIT_TAG_PUSH_OP)
}

// ---- repositories ---------------------------------------------------------------------------------------------------

/// One part of a repository path: the characters every host allows, never `.` or `..`.
fn segment_ok(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 100
        && s != "."
        && s != ".."
        && s.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
}

/// `owner/name`, or with `nested` (GitLab) `group/subgroup/.../name`.
pub(crate) fn repo_ok(repo: &str, nested: bool) -> bool {
    let parts: Vec<&str> = repo.split('/').collect();
    let depth_ok = if nested {
        (2..=MAX_DEPTH).contains(&parts.len())
    } else {
        parts.len() == 2
    };
    depth_ok && repo.len() <= MAX_REPO && parts.iter().all(|p| segment_ok(p)) && !repo.contains("..")
}

/// The call's `repo`, checked.
pub(crate) fn repo_arg(call: &ConnectorCall, nested: bool) -> Result<String, CoreError> {
    let repo = call.str_arg("repo").unwrap_or_default();
    if repo_ok(repo, nested) {
        Ok(repo.to_owned())
    } else if nested {
        Err(bad("`repo` must look like group/name or group/subgroup/name."))
    } else {
        Err(bad("`repo` must look like owner/name."))
    }
}

/// The wider things `repo` (or a branch of it) belongs to, nearest first: the repository itself for a branch, then each
/// group or owner above it.
pub(crate) fn parents(repo: &str, branch: Option<&str>) -> Vec<(String, String)> {
    let mut out = Vec::new();
    if branch.is_some() {
        out.push((repo.to_owned(), format!("Any branch of {repo}")));
    }
    let mut owner = repo;
    while let Some((above, _)) = owner.rsplit_once('/') {
        out.push((above.to_owned(), format!("Every repository of {above}")));
        owner = above;
    }
    out
}

fn resource_label(repo: &str, branch: Option<&str>) -> String {
    branch.map_or_else(|| repo.to_owned(), |b| format!("{repo}, branch {b}"))
}

// ---- arguments ------------------------------------------------------------------------------------------------------

fn digest_arg(call: &ConnectorCall) -> Result<String, CoreError> {
    match call.str_arg("digest") {
        Some(d) if d.len() == 64 && d.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) => {
            Ok(d.to_owned())
        }
        _ => Err(bad("`digest` must be a SHA-256 in lowercase hex.")),
    }
}

/// The push summary of a call (an object, or the same as a JSON string), validated.
pub(crate) fn summary_arg(call: &ConnectorCall) -> Result<PushSummary, CoreError> {
    let raw = call.args.get("summary").ok_or_else(|| bad("`summary` is required."))?;
    let parsed = match raw {
        Value::String(s) => serde_json::from_str::<PushSummary>(s),
        other => PushSummary::deserialize(other),
    };
    let summary = parsed.map_err(|e| {
        bad(format!("`summary` is not a push summary: {}", text::truncate_chars(&text::one_line(&e.to_string()), 200)))
    })?;
    summary.validate().map_err(|e| bad(format!("The push summary was refused: {e}.")))?;
    Ok(summary)
}

/// The summary, consistent with the tool asked for: tags only go through a tag push, anything with a branch through
/// a push, and nothing but branches and tags at all.
pub(crate) fn checked_summary(call: &ConnectorCall) -> Result<PushSummary, CoreError> {
    let summary = summary_arg(call)?;
    let tool = call.spec().map_or("", |s| s.tool);
    if summary.tool_for(&call.service) != tool {
        let service = &call.service;
        return Err(bad(if call.op == GIT_TAG_PUSH_OP {
            format!("A tag push changes tags only; a push with branches is {service}_{GIT_PUSH_OP}.")
        } else {
            format!("A push of tags only is {service}_{GIT_TAG_PUSH_OP}.")
        }));
    }
    for u in &summary.updates {
        let Some(short) = u.branch().or_else(|| u.tag()) else {
            return Err(bad(format!("Only branches and tags can be pushed through Reins, not {}.", u.name)));
        };
        if !ref_ok(short) {
            return Err(bad(format!("{} is not a valid branch or tag name.", u.name)));
        }
    }
    Ok(summary)
}

// ---- credentials ----------------------------------------------------------------------------------------------------

/// The credential for `repo`, sealed to the app's key. The clear grant is wiped afterwards.
pub(crate) fn grant(
    call: &ConnectorCall,
    repo: &str,
    username: &str,
    token: &str,
    access: &str,
    digest: Option<String>,
    expires_at: i64,
) -> Result<String, CoreError> {
    let key = client_key(call)?;
    let grant = CredentialGrant {
        v: 1,
        nonce: nonce_arg(call)?,
        digest,
        repo: repo.to_owned(),
        access: access.to_owned(),
        username: username.to_owned(),
        token: token.to_owned(),
        expires_at,
    };
    let sealed = seal(key, &grant);
    // The grant holds the token in the clear: wipe it rather than leave it to the allocator.
    let _wiped = Zeroizing::new(grant.token);
    sealed
}

/// The answer to a fetch the host said the token can reach: one item for the repository, with the sealed read
/// credential.
pub(crate) fn fetch_item(
    call: &ConnectorCall,
    repo: &str,
    private: bool,
    username: &str,
    token: &str,
) -> Result<Item, CoreError> {
    let expires_at = unix_now().saturating_add(FETCH_LEASE_SECS);
    let sealed = grant(call, repo, username, token, "read", None, expires_at)?;
    let mut extra = Map::new();
    extra.insert(SEALED_FIELD.to_owned(), json!(sealed));
    extra.insert("expires_at".to_owned(), json!(expires_at));
    let label = if private {
        format!("{repo} (private)")
    } else {
        repo.to_owned()
    };
    Ok(Item {
        id: repo.to_owned(),
        resource: repo.to_owned(),
        resource_label: label,
        title: format!("Clone and fetch {repo}"),
        snippet: "Git on your computer can read this repository for 1 hour.".to_owned(),
        extra,
        parents: parents(repo, None),
        ..Item::default()
    })
}

/// Checks the arguments a fetch needs besides the repository, before the host is asked anything.
pub(crate) fn check_fetch(call: &ConnectorCall) -> Result<(), CoreError> {
    nonce_arg(call)?;
    client_key(call).map(drop)
}

// ---- push -----------------------------------------------------------------------------------------------------------

fn plural(n: u64, one: &str, many: &str) -> String {
    if n == 1 {
        format!("1 {one}")
    } else {
        format!("{n} {many}")
    }
}

fn short(sha: &str) -> String {
    sha.chars().take(7).collect()
}

/// One line per ref: "Push 3 commits to main", "Force push to main: rewrites history", "Tag v1.2 → 1a2b3c4".
fn ref_line(u: &RefUpdate) -> String {
    let commits = plural(u64::from(u.commit_count), "commit", "commits");
    let line = match (u.branch(), u.tag(), u.change) {
        (Some(b), _, RefChange::Create) => format!("Create branch {b} with {commits}"),
        (Some(b), _, RefChange::Delete) => format!("Delete branch {b}"),
        (Some(b), _, RefChange::Update) => match u.fast_forward {
            Some(true) => format!("Push {commits} to {b}"),
            Some(false) => format!("Force push to {b}: rewrites history"),
            None => format!("Force push to {b}: may rewrite history"),
        },
        (_, Some(t), RefChange::Create) => format!("Tag {t} \u{2192} {}", short(&u.new)),
        (_, Some(t), RefChange::Update) => format!("Move tag {t} \u{2192} {}", short(&u.new)),
        (_, Some(t), RefChange::Delete) => format!("Delete tag {t}"),
        _ => format!("Change {}", u.name),
    };
    text::one_line(&line)
}

/// "+A −D in N files" over every ref that changes files; without the line counts when one is missing.
fn totals_line(summary: &PushSummary) -> Option<String> {
    let changing: Vec<&RefUpdate> = summary.updates.iter().filter(|u| u.files_changed > 0).collect();
    let files: u64 = changing.iter().map(|u| u64::from(u.files_changed)).sum();
    if files == 0 {
        return None;
    }
    let files_text = plural(files, "file", "files");
    let added: Option<u64> = changing.iter().map(|u| u.additions).sum();
    let removed: Option<u64> = changing.iter().map(|u| u.deletions).sum();
    Some(match (added, removed) {
        (Some(a), Some(d)) => format!("+{a} \u{2212}{d} in {files_text}"),
        _ => format!("{files_text} changed"),
    })
}

/// What a push to `repo` does, for the user.
pub(crate) fn preview_push(call: &ConnectorCall, repo: &str) -> Result<Preview, CoreError> {
    client_key(call)?;
    nonce_arg(call)?;
    digest_arg(call)?;
    let summary = checked_summary(call)?;
    let branch = match summary.updates.as_slice() {
        [only] => only.branch(),
        _ => None,
    };
    let mut lines: Vec<String> = summary.updates.iter().map(ref_line).collect();
    lines.extend(summary.updates.iter().flat_map(|u| &u.commits).take(PREVIEW_COMMITS).map(|c| {
        let subject = text::one_line(&c.subject);
        if subject.is_empty() {
            format!("{} (no message)", short(&c.sha))
        } else {
            subject
        }
    }));
    lines.extend(totals_line(&summary));
    Ok(Preview {
        resource: summary.resource(repo),
        resource_label: resource_label(repo, branch),
        lines,
        parents: parents(repo, branch),
        once_only: summary.once_only(),
        ..Preview::default()
    })
}

/// The push credential, bound to the digest the user approved.
pub(crate) fn perform_push(call: &ConnectorCall, repo: &str, username: &str, token: &str) -> Result<Value, CoreError> {
    checked_summary(call)?;
    let digest = digest_arg(call)?;
    let sealed = grant(call, repo, username, token, "write", Some(digest), unix_now().saturating_add(PUSH_LEASE_SECS))?;
    Ok(json!({ SEALED_FIELD: sealed }))
}

#[cfg(test)]
mod tests {
    use reins_proto::desktop::{ZERO_OID, encode_key};

    use super::*;

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
            commit_count: 1,
            commits: vec![],
            files_changed: 0,
            files: vec![],
            additions: None,
            deletions: None,
        }
    }

    fn call(service: &str, op: &str, summary: &Value) -> ConnectorCall {
        let key = encode_key(crypto_box::SecretKey::generate(&mut crypto_box::aead::OsRng).public_key().as_bytes());
        ConnectorCall {
            service: service.to_owned(),
            op: op.to_owned(),
            args: json!({"repo": "me/app", "client_key": key, "nonce": "n1", "digest": "a".repeat(64),
                         "summary": summary})
            .as_object()
            .unwrap()
            .clone(),
        }
    }

    fn summary(updates: Vec<RefUpdate>) -> Value {
        serde_json::to_value(PushSummary {
            updates,
            ..PushSummary::default()
        })
        .unwrap()
    }

    #[test]
    fn each_ref_has_a_line_and_counts_are_spelled_out() {
        let a = "a".repeat(40);
        let b = "b".repeat(40);
        let mut one = update("refs/heads/main", &a, &b, Some(true));
        assert_eq!(ref_line(&one), "Push 1 commit to main");
        one.commit_count = 0;
        assert_eq!(ref_line(&one), "Push 0 commits to main");
        assert_eq!(ref_line(&update("refs/heads/x", ZERO_OID, &b, None)), "Create branch x with 1 commit");
        assert_eq!(ref_line(&update("refs/tags/v1", &a, ZERO_OID, None)), "Delete tag v1");
        let mut s: PushSummary =
            serde_json::from_value(summary(vec![update("refs/heads/main", &a, &b, Some(true))])).unwrap();
        assert_eq!(totals_line(&s), None, "no files, no line");
        s.updates[0].files_changed = 1;
        assert_eq!(totals_line(&s).as_deref(), Some("1 file changed"));
        s.updates[0].additions = Some(4);
        s.updates[0].deletions = Some(0);
        assert_eq!(totals_line(&s).as_deref(), Some("+4 \u{2212}0 in 1 file"));
    }

    #[test]
    fn refs_must_be_branches_or_tags_with_names_git_allows() {
        let a = "a".repeat(40);
        let b = "b".repeat(40);
        let ok = summary(vec![update("refs/heads/ok", &a, &b, Some(true))]);
        assert!(checked_summary(&call("github", GIT_PUSH_OP, &ok)).is_ok());
        assert!(checked_summary(&call("gitlab", GIT_PUSH_OP, &ok)).is_ok());
        for bad in ["refs/heads/a*b", "refs/heads/-x", "refs/heads/x.lock"] {
            assert!(
                checked_summary(&call("github", GIT_PUSH_OP, &summary(vec![update(bad, &a, &b, Some(true))]))).is_err(),
                "{bad}"
            );
        }
        let tags = summary(vec![update("refs/tags/v1", ZERO_OID, &b, None)]);
        let err = checked_summary(&call("codeberg", GIT_PUSH_OP, &tags)).unwrap_err();
        assert!(err.to_string().contains("codeberg_git_tag_push"), "{err}");
    }

    #[test]
    fn a_push_credential_needs_a_digest_and_never_shows_the_token() {
        let a = "a".repeat(40);
        let b = "b".repeat(40);
        let mut c = call("github", GIT_PUSH_OP, &summary(vec![update("refs/heads/main", &a, &b, Some(true))]));
        let done = perform_push(&c, "me/app", "x-access-token", "ghp_secret").unwrap();
        assert!(!done.to_string().contains("ghp_secret"));
        assert_eq!(done.as_object().unwrap().len(), 1);
        c.args.insert("digest".to_owned(), json!("A".repeat(64)));
        assert!(perform_push(&c, "me/app", "x-access-token", "ghp_secret").is_err(), "upper case hex is refused");
    }

    #[test]
    fn repository_paths_and_their_owners() {
        for ok in ["a/b", "Org/Repo.js", "a-b/c_d.e"] {
            assert!(repo_ok(ok, false), "{ok}");
        }
        for bad in ["a", "a/b/c", "a/", "/a", "a/..", "a b/c", "a/b?x", "a%2Fb/c", "./a"] {
            assert!(!repo_ok(bad, false), "{bad}");
        }
        assert!(repo_ok("group/sub/deeper/app", true));
        assert!(!repo_ok("group//app", true));
        assert!(!repo_ok("a/".repeat(25).trim_end_matches('/'), true), "too deep");
        assert_eq!(
            parents("g/s/app", Some("main")),
            [
                ("g/s/app".to_owned(), "Any branch of g/s/app".to_owned()),
                ("g/s".to_owned(), "Every repository of g/s".to_owned()),
                ("g".to_owned(), "Every repository of g".to_owned()),
            ]
        );
        assert_eq!(parents("me/app", None), [("me".to_owned(), "Every repository of me".to_owned())]);
    }
}
