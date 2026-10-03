//! What the server already has, for describing a push: objects the pack builds on, old trees and blobs, and ancestry.
//! The proxy only builds a [`GitHubRemote`] with [`GitHubRemote::new`] and passes it to
//! [`crate::git::analyze_push`].

use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use reqwest::StatusCode;
use reqwest::header::{ACCEPT, AUTHORIZATION, HeaderValue, USER_AGENT};
use serde::Deserialize;

use crate::auth::Credential;
use crate::git::object::{ObjectKind, Oid, TreeEntry, hash_object, parse_commit, serialize_tree, sort_tree};

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum RemoteError {
    /// The object is larger than the caller's limit.
    #[error("the object is too large")]
    TooLarge,
    /// No more requests for this push.
    #[error("the request budget for this push is used up")]
    Budget,
    #[error("{0}")]
    Failed(String),
}

/// Tree and parents of a commit on the server.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommitMeta {
    pub tree: Oid,
    pub parents: Vec<Oid>,
}

#[async_trait::async_trait]
pub trait Remote: Send + Sync {
    /// The raw object `oid` (`kind`: what the caller expects, when it knows), at most `max_len` bytes
    /// ([`RemoteError::TooLarge`] beyond). `Ok(None)` when the server does not have it or cannot give it raw. Callers
    /// check the id themselves.
    async fn object(
        &self,
        oid: &Oid,
        kind: Option<ObjectKind>,
        max_len: u64,
    ) -> Result<Option<(ObjectKind, Vec<u8>)>, RemoteError>;

    /// Tree and parents of commit `oid`.
    async fn commit(&self, oid: &Oid) -> Result<Option<CommitMeta>, RemoteError> {
        match self.object(oid, Some(ObjectKind::Commit), 1 << 20).await? {
            Some((ObjectKind::Commit, data)) if hash_object(ObjectKind::Commit, &data) == *oid => {
                let c = parse_commit(&data).map_err(RemoteError::Failed)?;
                Ok(Some(CommitMeta {
                    tree: c.tree,
                    parents: c.parents,
                }))
            }
            _ => Ok(None),
        }
    }

    /// Whether `ancestor` is an ancestor of (or equal to) `descendant` on the server; `None` when it cannot tell.
    async fn is_ancestor(&self, ancestor: &Oid, descendant: &Oid) -> Result<Option<bool>, RemoteError>;
}

/// Knows nothing (an unknown host, or no credential).
pub struct NoRemote;

#[async_trait::async_trait]
impl Remote for NoRemote {
    async fn object(
        &self,
        _: &Oid,
        _: Option<ObjectKind>,
        _: u64,
    ) -> Result<Option<(ObjectKind, Vec<u8>)>, RemoteError> {
        Ok(None)
    }

    async fn is_ancestor(&self, _: &Oid, _: &Oid) -> Result<Option<bool>, RemoteError> {
        Ok(None)
    }
}

/// Requests one push analysis may make.
pub const REQUEST_BUDGET: u32 = 200;
const TIMEOUT: Duration = Duration::from_secs(15);
/// Largest JSON answer read (a tree of tens of thousands of entries, a compare).
const MAX_JSON: u64 = 16 << 20;
const API_VERSION: &str = "2022-11-28";
const JSON: &str = "application/vnd.github+json";
const RAW: &str = "application/vnd.github.raw";

/// The GitHub REST API for one repository. Commits cannot be had raw (GitHub gives their metadata, from which the exact
/// bytes cannot be rebuilt), so they never serve as delta bases; trees are rebuilt and kept only when their id matches.
pub struct GitHubRemote {
    http: reqwest::Client,
    api_base: String,
    repo: String,
    authorization: Option<zeroize::Zeroizing<String>>,
    budget: AtomicU32,
}

impl GitHubRemote {
    /// `api_base`: `https://api.github.com`; `repo`: `owner/name`; `credential`: for private repositories.
    #[must_use]
    pub fn new(http: reqwest::Client, api_base: &str, repo: &str, credential: Option<&Credential>) -> Self {
        Self {
            http,
            api_base: api_base.trim_end_matches('/').to_owned(),
            repo: repo.to_owned(),
            authorization: credential.map(Credential::authorization),
            budget: AtomicU32::new(REQUEST_BUDGET),
        }
    }

    /// At most `requests` requests instead of [`REQUEST_BUDGET`].
    #[must_use]
    pub fn with_budget(self, requests: u32) -> Self {
        self.budget.store(requests, Ordering::Relaxed);
        self
    }

    fn take_request(&self) -> Result<(), RemoteError> {
        let mut left = self.budget.load(Ordering::Relaxed);
        loop {
            let next = left.checked_sub(1).ok_or(RemoteError::Budget)?;
            match self.budget.compare_exchange_weak(left, next, Ordering::Relaxed, Ordering::Relaxed) {
                Ok(_) => return Ok(()),
                Err(now) => left = now,
            }
        }
    }

    /// GET `/repos/{repo}/{path}`; `Ok(None)` for 404 and 422 (GitHub's answer for an object of another kind).
    async fn get(&self, path: &str, accept: &str, max_len: u64) -> Result<Option<Vec<u8>>, RemoteError> {
        self.take_request()?;
        let mut req = self
            .http
            .get(format!("{}/repos/{}/{path}", self.api_base, self.repo))
            .timeout(TIMEOUT)
            .header(ACCEPT, accept)
            .header("X-GitHub-Api-Version", API_VERSION)
            .header(USER_AGENT, crate::http::USER_AGENT);
        if let Some(auth) = &self.authorization {
            let mut value = HeaderValue::from_str(auth)
                .map_err(|_| RemoteError::Failed("the credential is malformed".to_owned()))?;
            value.set_sensitive(true);
            req = req.header(AUTHORIZATION, value);
        }
        let failed = |e: reqwest::Error| RemoteError::Failed(format!("GitHub request failed: {}", e.without_url()));
        let mut resp = req.send().await.map_err(failed)?;
        match resp.status() {
            StatusCode::OK => {}
            StatusCode::NOT_FOUND | StatusCode::UNPROCESSABLE_ENTITY => return Ok(None),
            s => return Err(RemoteError::Failed(format!("GitHub answered {}", s.as_u16()))),
        }
        if resp.content_length().is_some_and(|n| n > max_len) {
            return Err(RemoteError::TooLarge);
        }
        let mut body = Vec::new();
        while let Some(chunk) = resp.chunk().await.map_err(failed)? {
            if (body.len() + chunk.len()) as u64 > max_len {
                return Err(RemoteError::TooLarge);
            }
            body.extend_from_slice(&chunk);
        }
        Ok(Some(body))
    }

    async fn json<T: for<'de> Deserialize<'de>>(&self, path: &str) -> Result<Option<T>, RemoteError> {
        let max = MAX_JSON;
        let Some(body) = self.get(path, JSON, max).await.map_err(|e| match e {
            RemoteError::TooLarge => RemoteError::Failed("GitHub's answer is too large".to_owned()),
            e => e,
        })?
        else {
            return Ok(None);
        };
        serde_json::from_slice(&body)
            .map(Some)
            .map_err(|e| RemoteError::Failed(format!("GitHub's answer is malformed: {e}")))
    }

    async fn blob(&self, oid: &Oid, max_len: u64) -> Result<Option<Vec<u8>>, RemoteError> {
        let Some(data) = self.get(&format!("git/blobs/{oid}"), RAW, max_len).await? else {
            return Ok(None);
        };
        if hash_object(ObjectKind::Blob, &data) != *oid {
            return Err(RemoteError::Failed("GitHub sent a blob that does not match its id".to_owned()));
        }
        Ok(Some(data))
    }

    async fn tree(&self, oid: &Oid, max_len: u64) -> Result<Option<Vec<u8>>, RemoteError> {
        let Some(tree) = self.json::<TreeJson>(&format!("git/trees/{oid}")).await? else {
            return Ok(None);
        };
        let raw = raw_tree(oid, tree)?;
        if raw.len() as u64 > max_len {
            return Err(RemoteError::TooLarge);
        }
        Ok(Some(raw))
    }
}

#[derive(Deserialize)]
pub(crate) struct TreeJson {
    #[serde(default)]
    truncated: bool,
    tree: Vec<TreeItem>,
}

#[derive(Deserialize)]
struct TreeItem {
    path: String,
    mode: String,
    sha: String,
}

#[derive(Deserialize)]
struct ShaJson {
    sha: String,
}

#[derive(Deserialize)]
struct CommitJson {
    tree: ShaJson,
    parents: Vec<ShaJson>,
}

#[derive(Deserialize)]
struct CompareJson {
    status: String,
}

/// Rebuilds the raw tree from GitHub's JSON; accepted only when it hashes to `oid` (in GitHub's order, or git's).
pub(crate) fn raw_tree(oid: &Oid, json: TreeJson) -> Result<Vec<u8>, RemoteError> {
    let mismatch = || RemoteError::Failed(format!("GitHub's tree {oid} does not match its id"));
    if json.truncated {
        return Err(mismatch());
    }
    let mut entries = json
        .tree
        .into_iter()
        .map(|item| {
            Some(TreeEntry {
                mode: u32::from_str_radix(&item.mode, 8).ok()?,
                name: item.path.into_bytes(),
                oid: Oid::from_hex(&item.sha)?,
            })
        })
        .collect::<Option<Vec<_>>>()
        .ok_or_else(mismatch)?;
    let raw = serialize_tree(&entries);
    if hash_object(ObjectKind::Tree, &raw) == *oid {
        return Ok(raw);
    }
    sort_tree(&mut entries);
    let raw = serialize_tree(&entries);
    if hash_object(ObjectKind::Tree, &raw) == *oid {
        Ok(raw)
    } else {
        Err(mismatch())
    }
}

/// The compare API's `status` for `ancestor...descendant`.
pub(crate) fn ancestry(status: &str) -> Option<bool> {
    match status {
        "ahead" | "identical" => Some(true),
        "behind" | "diverged" => Some(false),
        _ => None,
    }
}

#[async_trait::async_trait]
impl Remote for GitHubRemote {
    async fn object(
        &self,
        oid: &Oid,
        kind: Option<ObjectKind>,
        max_len: u64,
    ) -> Result<Option<(ObjectKind, Vec<u8>)>, RemoteError> {
        if matches!(kind, None | Some(ObjectKind::Blob))
            && let Some(data) = self.blob(oid, max_len).await?
        {
            return Ok(Some((ObjectKind::Blob, data)));
        }
        if matches!(kind, None | Some(ObjectKind::Tree))
            && let Some(data) = self.tree(oid, max_len).await?
        {
            return Ok(Some((ObjectKind::Tree, data)));
        }
        Ok(None)
    }

    async fn commit(&self, oid: &Oid) -> Result<Option<CommitMeta>, RemoteError> {
        let Some(c) = self.json::<CommitJson>(&format!("git/commits/{oid}")).await? else {
            return Ok(None);
        };
        let bad = || RemoteError::Failed("GitHub's commit is malformed".to_owned());
        Ok(Some(CommitMeta {
            tree: Oid::from_hex(&c.tree.sha).ok_or_else(bad)?,
            parents: c.parents.iter().map(|p| Oid::from_hex(&p.sha)).collect::<Option<_>>().ok_or_else(bad)?,
        }))
    }

    async fn is_ancestor(&self, ancestor: &Oid, descendant: &Oid) -> Result<Option<bool>, RemoteError> {
        if ancestor == descendant {
            return Ok(Some(true));
        }
        let c = self.json::<CompareJson>(&format!("compare/{ancestor}...{descendant}?per_page=1")).await?;
        Ok(c.and_then(|c| ancestry(&c.status)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::object::mode;

    fn item(path: &str, mode: &str, sha: &Oid) -> TreeItem {
        TreeItem {
            path: path.to_owned(),
            mode: mode.to_owned(),
            sha: sha.to_hex(),
        }
    }

    #[test]
    fn trees_are_rebuilt_from_json_and_checked() {
        let (a, b) = (Oid([1; 20]), Oid([2; 20]));
        let entries = vec![
            TreeEntry {
                mode: mode::BLOB,
                name: b"a.txt".to_vec(),
                oid: a,
            },
            TreeEntry {
                mode: mode::TREE,
                name: b"a".to_vec(),
                oid: b,
            },
        ];
        let oid = hash_object(ObjectKind::Tree, &serialize_tree(&entries));
        let json = |items| TreeJson {
            truncated: false,
            tree: items,
        };
        // GitHub writes a tree's mode as 040000; the raw format has 40000.
        let raw = raw_tree(&oid, json(vec![item("a.txt", "100644", &a), item("a", "040000", &b)])).unwrap();
        assert_eq!(raw, serialize_tree(&entries));
        // Another order is put back in git's.
        raw_tree(&oid, json(vec![item("a", "040000", &b), item("a.txt", "100644", &a)])).unwrap();
        for bad in [
            json(vec![item("a.txt", "100755", &a), item("a", "040000", &b)]),
            json(vec![item("a.txt", "100644", &b), item("a", "040000", &b)]),
            json(vec![item("a.txt", "100644", &a)]),
            json(vec![item("a.txt", "10064x", &a), item("a", "040000", &b)]),
            TreeJson {
                truncated: true,
                tree: vec![item("a.txt", "100644", &a), item("a", "040000", &b)],
            },
        ] {
            assert!(raw_tree(&oid, bad).is_err());
        }
    }

    #[test]
    fn compare_statuses_map_to_ancestry() {
        assert_eq!(ancestry("ahead"), Some(true));
        assert_eq!(ancestry("identical"), Some(true));
        assert_eq!(ancestry("behind"), Some(false));
        assert_eq!(ancestry("diverged"), Some(false));
        assert_eq!(ancestry("weird"), None);
    }

    #[tokio::test]
    async fn no_remote_knows_nothing() {
        let oid = Oid([1; 20]);
        assert_eq!(NoRemote.object(&oid, None, 10).await, Ok(None));
        assert_eq!(NoRemote.commit(&oid).await, Ok(None));
        assert_eq!(NoRemote.is_ancestor(&oid, &oid).await, Ok(None));
    }
}
