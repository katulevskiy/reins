//! GitHub: files, commits, tags and releases.
//!
//! Permissions: anything about one branch (files, commits, pushing) is the resource `owner/repo@branch`; tags, releases
//! and assets are `owner/repo`. A read at a `ref` is about a branch only when the ref names one; a tag or a commit SHA
//! is read as `owner/repo`.

#![allow(clippy::assigning_clones, reason = "items are built from empty defaults; a clone_into gains nothing")]

use std::collections::BTreeMap;
use std::fmt::Write as _;

use data_encoding::{BASE64, HEXLOWER};
use reins_proto::connector::ConnectorCall;
use reqwest::Method;
use serde_json::{Map, Value, json};

use super::{GitHub, Options, Preview, explain, parents, ref_arg, ref_ok, repo_arg, resource, resource_label};
use crate::blob::{self, DELIVER_KEY};
use crate::connector::Item;
use crate::connector::calendar::{limit, segment};
use crate::{CoreError, text};

/// The most characters of a file (or asset) handed to the AI as text.
const MAX_FILE_TEXT: usize = 100_000;
/// The most characters of a diff or a commit body handed to the AI.
const MAX_DIFF: usize = 60_000;
/// The largest content decoded from base64 or downloaded.
const MAX_BINARY: usize = 2_000_000;
/// The largest total of the files of one commit.
const MAX_COMMIT_TOTAL: usize = 4_000_000;
/// How much of a text change the preview shows.
const SNIPPET: usize = 300;
/// The media type of a file's raw content.
const RAW: &str = "application/vnd.github.raw+json";
/// How much of the AI's own text (message, notes) the preview shows.
const PREVIEW_TEXT: usize = 1_500;
const MAX_FILES: usize = 100;

fn bad(message: impl Into<String>) -> CoreError {
    CoreError::service(message)
}

/// Lists and reads. `None` when the operation is not one of this area's.
pub(super) async fn fetch(gh: &GitHub, token: &str, call: &ConnectorCall) -> Option<Result<Vec<Item>, CoreError>> {
    Some(match call.op.as_str() {
        "file_get" => file_get(gh, token, call).await,
        "dir_list" => dir_list(gh, token, call).await,
        "tree_get" => tree_get(gh, token, call).await,
        "blob_get" => blob_get(gh, token, call).await,
        "archive_link" => archive_link(gh, token, call).await,
        "commit_list" => commit_list(gh, token, call).await,
        "commit_get" => commit_get(gh, token, call).await,
        "commit_compare" => commit_compare(gh, token, call).await,
        "checks_get" => checks_get(gh, token, call).await,
        "tag_list" => tag_list(gh, token, call).await,
        "tag_get" => tag_get(gh, token, call).await,
        "release_list" => release_list(gh, token, call).await,
        "release_get" => release_one(gh, token, call, Which::Id).await,
        "release_latest" => release_one(gh, token, call, Which::Latest).await,
        "release_by_tag" => release_one(gh, token, call, Which::Tag).await,
        "release_notes_generate" => release_notes(gh, token, call).await,
        "release_asset_list" => asset_list(gh, token, call).await,
        "release_asset_download" => asset_download(gh, token, call).await,
        _ => return None,
    })
}

/// What a write would do. `None` when the operation is not one of this area's.
pub(super) async fn preview(gh: &GitHub, token: &str, call: &ConnectorCall) -> Option<Result<Preview, CoreError>> {
    Some(match call.op.as_str() {
        "file_put" => preview_file_put(gh, token, call).await,
        "file_delete" => preview_file_delete(gh, token, call).await,
        "commit_files" => preview_commit_files(gh, token, call).await,
        "tag_create" => preview_tag_create(gh, token, call).await,
        "tag_delete" => preview_tag_delete(gh, token, call).await,
        "release_create" => preview_release_create(gh, token, call).await,
        "release_update" => preview_release_update(gh, token, call).await,
        "release_delete" => preview_release_delete(gh, token, call).await,
        "release_asset_upload" => preview_asset_upload(gh, token, call).await,
        "release_asset_update" => preview_asset_update(gh, token, call).await,
        "release_asset_delete" => preview_asset_delete(gh, token, call).await,
        _ => return None,
    })
}

/// Does the write. `None` when the operation is not one of this area's.
pub(super) async fn perform(gh: &GitHub, token: &str, call: &ConnectorCall) -> Option<Result<Value, CoreError>> {
    Some(match call.op.as_str() {
        "file_put" => perform_file_put(gh, token, call).await,
        "file_delete" => perform_file_delete(gh, token, call).await,
        "commit_files" => perform_commit_files(gh, token, call).await,
        "tag_create" => perform_tag_create(gh, token, call).await,
        "tag_delete" => perform_tag_delete(gh, token, call).await,
        "release_create" => perform_release_create(gh, token, call).await,
        "release_update" => perform_release_update(gh, token, call).await,
        "release_delete" => perform_release_delete(gh, token, call).await,
        "release_asset_upload" => perform_asset_upload(gh, token, call).await,
        "release_asset_update" => perform_asset_update(gh, token, call).await,
        "release_asset_delete" => perform_asset_delete(gh, token, call).await,
        _ => return None,
    })
}

// ---- validation and small helpers ---------------------------------------------------------------------------------

/// A file path inside a repository: at most 1024 bytes, relative, no `.`/`..`/`.git` segment, no empty segment, no
/// backslash or control character.
fn path_ok(path: &str) -> bool {
    !path.is_empty()
        && path.len() <= 1_024
        && !path.starts_with('/')
        && !path.chars().any(|c| c.is_control() || c == '\\')
        && path.split('/').all(|s| !s.is_empty() && s != "." && s != ".." && !s.eq_ignore_ascii_case(".git"))
}

fn path_message(name: &str) -> String {
    format!(
        "`{name}` is not a valid file path: use a relative path such as src/main.rs (no leading or trailing slash, no \
         `..`, no backslash or control characters, at most 1024 bytes)."
    )
}

fn required_path(call: &ConnectorCall, name: &str) -> Result<String, CoreError> {
    match call.str_arg(name) {
        Some(p) if path_ok(p) => Ok(p.to_owned()),
        _ => Err(bad(path_message(name))),
    }
}

fn optional_path(call: &ConnectorCall, name: &str) -> Result<Option<String>, CoreError> {
    match call.str_arg(name) {
        None => Ok(None),
        Some(p) if path_ok(p) => Ok(Some(p.to_owned())),
        Some(_) => Err(bad(path_message(name))),
    }
}

/// Each segment of a path percent-encoded, slashes kept.
fn enc_path(path: &str) -> String {
    path.split('/').map(segment).collect::<Vec<_>>().join("/")
}

fn required_ref(call: &ConnectorCall, name: &str) -> Result<String, CoreError> {
    ref_arg(call, name)?.ok_or_else(|| bad(format!("`{name}` is required.")))
}

fn is_full_sha(s: &str) -> bool {
    matches!(s.len(), 40 | 64) && s.bytes().all(|b| b.is_ascii_hexdigit())
}

fn sha_arg(call: &ConnectorCall, name: &str) -> Result<Option<String>, CoreError> {
    match call.str_arg(name) {
        None => Ok(None),
        Some(s) if is_full_sha(s) => Ok(Some(s.to_ascii_lowercase())),
        Some(_) => Err(bad(format!("`{name}` must be a full git sha (40 hexadecimal characters)."))),
    }
}

fn id_arg(call: &ConnectorCall, name: &str) -> Result<i64, CoreError> {
    call.int_arg(name).filter(|n| *n > 0).ok_or_else(|| bad(format!("`{name}` must be a positive number.")))
}

fn short(sha: &str) -> &str {
    text::truncate_bytes(sha, 7)
}

fn str_of<'a>(v: &'a Value, key: &str) -> &'a str {
    v[key].as_str().unwrap_or_default()
}

/// The first line of a commit message or release title, for lists.
fn first_line(message: &str) -> String {
    text::one_line(message.lines().next().unwrap_or_default())
}

/// At most `max` characters, with a marker when cut.
fn clip(s: &str, max: usize) -> String {
    let cut = text::truncate_chars(s, max);
    if cut.len() < s.len() {
        format!("{cut} [...cut, {} characters in all]", s.chars().count())
    } else {
        cut
    }
}

fn decode_b64(name: &str, raw: &str) -> Result<Vec<u8>, CoreError> {
    let clean: String = raw.chars().filter(|c| !c.is_whitespace()).collect();
    if clean.len() > MAX_BINARY / 3 * 4 + 8 {
        return Err(bad(format!("`{name}` is larger than 2 MB once decoded.")));
    }
    let bytes = BASE64.decode(clean.as_bytes()).map_err(|_| bad(format!("`{name}` is not valid base64.")))?;
    if bytes.len() > MAX_BINARY {
        return Err(bad(format!("`{name}` is larger than 2 MB once decoded.")));
    }
    Ok(bytes)
}

/// The content of a file given as `content` (text) or `content_base64`, exactly one.
fn content_arg(call: &ConnectorCall) -> Result<Vec<u8>, CoreError> {
    match (call.str_arg("content"), call.str_arg("content_base64")) {
        (Some(_), Some(_)) => Err(bad("Give either `content` or `content_base64`, not both.")),
        (None, None) => Err(bad("Give the file's content as `content` (text) or `content_base64`.")),
        (Some(t), None) => {
            if t.len() > MAX_BINARY {
                return Err(bad("`content` is larger than 2 MB."));
            }
            Ok(t.as_bytes().to_vec())
        }
        (None, Some(b)) => decode_b64("content_base64", b),
    }
}

/// The file of a write: given in the call, or uploaded through the server (`blob`, see [`crate::blob`]).
enum Source {
    Bytes(Vec<u8>),
    Blob(String),
}

/// The content of `github_file_put`: inline (`content_arg`) or an upload.
fn file_source(call: &ConnectorCall) -> Result<Source, CoreError> {
    match call.str_arg("blob").filter(|b| !b.is_empty()) {
        Some(_) if call.str_arg("content").is_some() || call.str_arg("content_base64").is_some() => {
            Err(bad("Give either the content or `blob`, not both."))
        }
        Some(id) => Ok(Source::Blob(id.to_owned())),
        None => content_arg(call).map(Source::Bytes),
    }
}

/// What the server answered for a file it sent on to GitHub ([`blob::send`]), as GitHub's JSON.
fn sent(result: &reins_proto::blob::BlobSendResult) -> Result<Value, CoreError> {
    if (200..300).contains(&result.status) {
        return Ok(serde_json::from_str(&result.body).unwrap_or(Value::Null));
    }
    let status = reqwest::StatusCode::from_u16(result.status).unwrap_or(reqwest::StatusCode::BAD_GATEWAY);
    Err(explain(status, &result.body))
}

/// UTF-8 text without NUL bytes, else `None` (binary).
fn as_text(bytes: &[u8]) -> Option<&str> {
    std::str::from_utf8(bytes).ok().filter(|t| !t.contains('\0'))
}

/// The id git gives the content as a blob.
fn git_blob_sha(bytes: &[u8]) -> String {
    let mut ctx = ring::digest::Context::new(&ring::digest::SHA1_FOR_LEGACY_USE_ONLY);
    ctx.update(format!("blob {}\0", bytes.len()).as_bytes());
    ctx.update(bytes);
    HEXLOWER.encode(ctx.finish().as_ref())
}

/// Name and email of an author or tagger; both or neither.
fn person_arg(call: &ConnectorCall, name_key: &str, email_key: &str) -> Result<Option<Value>, CoreError> {
    match (call.str_arg(name_key), call.str_arg(email_key)) {
        (None, None) => Ok(None),
        (Some(name), Some(email)) => {
            let clean =
                |s: &str, extra: &[char]| !s.is_empty() && !s.chars().any(|c| c.is_control() || extra.contains(&c));
            if !clean(name, &['<', '>']) || !clean(email, &['<', '>', ' ']) || !email.contains('@') {
                return Err(bad(format!("`{name_key}` and `{email_key}` must be a plain name and an email address.")));
            }
            Ok(Some(json!({"name": name, "email": email})))
        }
        _ => Err(bad(format!("Give `{name_key}` and `{email_key}` together, or neither."))),
    }
}

fn person_line(person: &Value, what: &str) -> String {
    format!("{what}: {} <{}>", text::one_line(str_of(person, "name")), text::one_line(str_of(person, "email")))
}

/// A description of some content for a preview: "text, 120 bytes" and, for text, its start.
fn describe(bytes: &[u8]) -> (String, Option<String>) {
    match as_text(bytes) {
        Some(t) => {
            let lines = t.lines().count();
            (
                format!(
                    "text, {} bytes, {lines} line{}",
                    bytes.len(),
                    if lines == 1 {
                        ""
                    } else {
                        "s"
                    }
                ),
                Some(text::neutralize(&clip(t, SNIPPET))),
            )
        }
        None => (format!("binary, {} bytes", bytes.len()), None),
    }
}

fn status_letter(status: &str) -> &'static str {
    match status {
        "added" => "A",
        "removed" => "D",
        "renamed" => "R",
        "copied" => "C",
        _ => "M",
    }
}

/// The changed files of a commit or comparison as text: a summary line each, then their patches.
fn files_text(files: &[Value]) -> String {
    let mut out = String::new();
    for f in files {
        let name = str_of(f, "filename");
        let renamed = f["previous_filename"].as_str().map_or_else(String::new, |p| format!("{p} -> "));
        writeln!(
            out,
            "{} {renamed}{name} (+{} -{})",
            status_letter(str_of(f, "status")),
            f["additions"].as_i64().unwrap_or(0),
            f["deletions"].as_i64().unwrap_or(0)
        )
        .ok();
    }
    for f in files {
        if let Some(patch) = f["patch"].as_str() {
            write!(out, "\n--- {}\n{patch}\n", str_of(f, "filename")).ok();
        }
    }
    out
}

/// Text handed to the AI capped at `max` characters; the flag says it was cut.
fn capped(s: &str, max: usize) -> (String, bool) {
    let cut = text::truncate_chars(s, max);
    let was_cut = cut.len() < s.len();
    (cut, was_cut)
}

/// `GET path`; a 404 is `None`.
async fn get_opt(gh: &GitHub, token: &str, path: &str, query: &[(&str, String)]) -> Result<Option<Value>, CoreError> {
    let options = Options {
        allow: &[404],
        ..Options::default()
    };
    let reply = gh.send(token, Method::GET, path, query, None, &options).await?;
    Ok((reply.status != 404).then_some(reply.json))
}

/// The default branch of a repository.
async fn default_branch(gh: &GitHub, token: &str, repo: &str) -> Result<String, CoreError> {
    let v = gh.call(token, Method::GET, &format!("/repos/{repo}"), &[], None).await?;
    v["default_branch"]
        .as_str()
        .filter(|b| ref_ok(b))
        .map(str::to_owned)
        .ok_or_else(|| bad("GitHub did not say which branch is the default one."))
}

/// The commit a branch points to, when the branch exists.
async fn branch_head(gh: &GitHub, token: &str, repo: &str, branch: &str) -> Result<Option<String>, CoreError> {
    let v = get_opt(gh, token, &format!("/repos/{repo}/git/ref/heads/{}", enc_path(branch)), &[]).await?;
    Ok(v.and_then(|v| v["object"]["sha"].as_str().map(str::to_owned)))
}

/// What the permission for a read is about.
struct Scope {
    resource: String,
    label: String,
    parents: Vec<(String, String)>,
}

impl Scope {
    fn new(repo: &str, branch: Option<&str>) -> Self {
        Self {
            resource: resource(repo, branch),
            label: resource_label(repo, branch),
            parents: parents(repo, branch),
        }
    }

    fn item(&self, id: String) -> Item {
        Item {
            id,
            resource: self.resource.clone(),
            resource_label: self.label.clone(),
            parents: self.parents.clone(),
            ..Item::default()
        }
    }
}

/// `owner/repo@branch` when `git_ref` names a branch of the repository; `owner/repo` for no ref, a tag or a commit.
async fn scope(gh: &GitHub, token: &str, repo: &str, git_ref: Option<&str>) -> Result<Scope, CoreError> {
    if let Some(r) = git_ref.filter(|r| !is_full_sha(r))
        && branch_head(gh, token, repo, r).await?.is_some()
    {
        return Ok(Scope::new(repo, Some(r)));
    }
    Ok(Scope::new(repo, None))
}

/// Asks for a path that answers with a redirect and returns where it leads. The token goes to GitHub only.
async fn redirect_location(gh: &GitHub, token: &str, path: &str, accept: &str) -> Result<String, CoreError> {
    let resp = gh
        .http
        .get(format!("{}{path}", gh.base))
        .bearer_auth(token)
        .header("Accept", accept)
        .header("X-GitHub-Api-Version", "2022-11-28")
        .header("User-Agent", "reins")
        .send()
        .await?;
    let status = resp.status();
    if !status.is_redirection() {
        let body = resp.text().await.unwrap_or_default();
        return if status.is_success() {
            Err(bad("GitHub did not say where the download is."))
        } else {
            Err(explain(status, &body))
        };
    }
    let location = resp
        .headers()
        .get("location")
        .and_then(|h| h.to_str().ok())
        .ok_or_else(|| bad("GitHub did not say where the download is."))?
        .to_owned();
    let url = url::Url::parse(&location).map_err(|_| bad("GitHub sent an unusable download address."))?;
    let same_origin = url::Url::parse(&gh.base).is_ok_and(|b| b.origin() == url.origin());
    if url.scheme() != "https" && !same_origin {
        return Err(bad("GitHub sent a download address that is not https."));
    }
    Ok(location)
}

/// Downloads `url` (no token is sent), at most `MAX_BINARY` bytes.
async fn download(gh: &GitHub, url: &str) -> Result<Vec<u8>, CoreError> {
    let mut resp = gh.http.get(url).header("User-Agent", "reins").send().await?;
    if !resp.status().is_success() {
        return Err(bad(format!("The download failed (status {}).", resp.status().as_u16())));
    }
    let mut out = Vec::new();
    while let Some(chunk) = resp.chunk().await? {
        out.extend_from_slice(&chunk);
        if out.len() > MAX_BINARY {
            return Err(bad("The file is larger than 2 MB."));
        }
    }
    Ok(out)
}

/// An item carrying some content: text in the body, binary as base64 in `content_base64`, too large as metadata.
fn content_item(
    scope: &Scope,
    id: String,
    title: &str,
    bytes: Option<&[u8]>,
    size: u64,
    mut extra: Map<String, Value>,
) -> Item {
    let mut item = scope.item(id);
    item.title = title.to_owned();
    extra.insert("size".to_owned(), json!(size));
    let text_part = bytes.and_then(|b| as_text(b).map(|t| capped(t, MAX_FILE_TEXT)));
    if let Some((body, truncated)) = text_part {
        let cut = if truncated {
            " · cut"
        } else {
            ""
        };
        item.snippet = format!("text · {size} bytes{cut}");
        item.body = Some(body);
        extra.insert("encoding".to_owned(), json!("text"));
        if truncated {
            extra.insert("truncated".to_owned(), json!(true));
        }
    } else if let Some(b) = bytes {
        item.snippet = format!("binary · {size} bytes · handed over as base64");
        extra.insert("encoding".to_owned(), json!("base64"));
        extra.insert("content_base64".to_owned(), json!(BASE64.encode(b)));
    } else {
        item.snippet = format!("{size} bytes · too large to hand over (over 2 MB)");
        extra.insert("encoding".to_owned(), json!("none"));
        extra.insert("too_large".to_owned(), json!(true));
    }
    item.extra = extra;
    item
}

/// A blob's bytes (`None` when larger than 2 MB or than the reply cap) and its size.
async fn blob_fetch(gh: &GitHub, token: &str, repo: &str, sha: &str) -> Result<(Option<Vec<u8>>, u64), CoreError> {
    let v = gh.call(token, Method::GET, &format!("/repos/{repo}/git/blobs/{sha}"), &[], None).await?;
    if v.is_null() {
        return Ok((None, 0));
    }
    let size = v["size"].as_u64().unwrap_or(0);
    if size > MAX_BINARY as u64 {
        return Ok((None, size));
    }
    let content = str_of(&v, "content");
    let bytes = if str_of(&v, "encoding") == "base64" {
        decode_b64("blob content", content)?
    } else {
        content.as_bytes().to_vec()
    };
    Ok((Some(bytes), size))
}

// ---- contents -------------------------------------------------------------------------------------------------------

async fn file_get(gh: &GitHub, token: &str, call: &ConnectorCall) -> Result<Vec<Item>, CoreError> {
    let repo = repo_arg(call)?;
    let path = required_path(call, "path")?;
    let git_ref = ref_arg(call, "ref")?;
    let sc = scope(gh, token, &repo, git_ref.as_deref()).await?;
    let query: Vec<(&str, String)> = git_ref.iter().map(|r| ("ref", r.clone())).collect();
    let v = gh.call(token, Method::GET, &format!("/repos/{repo}/contents/{}", enc_path(&path)), &query, None).await?;
    if v.is_array() {
        return Err(bad(format!("`{path}` is a directory; use github_dir_list.")));
    }
    let mut extra = Map::new();
    extra.insert("path".to_owned(), json!(path));
    extra.insert("sha".to_owned(), v["sha"].clone());
    extra.insert("url".to_owned(), v["html_url"].clone());
    if let Some(r) = &git_ref {
        extra.insert("ref".to_owned(), json!(r));
    }
    let size = v["size"].as_u64().unwrap_or(0);
    let id = format!("{repo}:{path}");
    match str_of(&v, "type") {
        // Too large to hand over inline: a download link once released (GitHub sends the raw file to the server).
        "file" if blob::as_link(size) => {
            let mut url = url::Url::parse(&format!("{}/repos/{repo}/contents/{}", gh.base, enc_path(&path)))
                .map_err(|_| bad("That path cannot be downloaded."))?;
            if let Some(r) = &git_ref {
                url.query_pairs_mut().append_pair("ref", r);
            }
            let name = path.rsplit('/').next().unwrap_or("file");
            let mut item = sc.item(id);
            item.title.clone_from(&path);
            item.snippet = format!("file · {size} bytes");
            extra.insert("size".to_owned(), json!(size));
            extra.insert(DELIVER_KEY.to_owned(), blob::fetch_marker(url.as_str(), RAW, name, size));
            item.extra = extra;
            Ok(vec![item])
        }
        "file" => {
            let inline = str_of(&v, "encoding") == "base64" && !str_of(&v, "content").is_empty();
            let bytes = if inline {
                Some(decode_b64("file content", str_of(&v, "content"))?)
            } else if size == 0 {
                Some(Vec::new())
            } else if size <= MAX_BINARY as u64 {
                blob_fetch(gh, token, &repo, str_of(&v, "sha")).await?.0
            } else {
                None
            };
            Ok(vec![content_item(&sc, id, &path, bytes.as_deref(), size, extra)])
        }
        kind @ ("symlink" | "submodule") => {
            let mut item = sc.item(id);
            item.title = path.clone();
            item.snippet = format!("{kind} · not a regular file");
            extra.insert("type".to_owned(), json!(kind));
            for key in ["target", "submodule_git_url"] {
                if let Some(t) = v[key].as_str() {
                    extra.insert(key.to_owned(), json!(t));
                }
            }
            item.extra = extra;
            Ok(vec![item])
        }
        _ => Err(bad(format!("`{path}` is not a file."))),
    }
}

async fn dir_list(gh: &GitHub, token: &str, call: &ConnectorCall) -> Result<Vec<Item>, CoreError> {
    let repo = repo_arg(call)?;
    let path = optional_path(call, "path")?;
    let git_ref = ref_arg(call, "ref")?;
    let sc = scope(gh, token, &repo, git_ref.as_deref()).await?;
    let query: Vec<(&str, String)> = git_ref.iter().map(|r| ("ref", r.clone())).collect();
    let api = path
        .as_deref()
        .map_or_else(|| format!("/repos/{repo}/contents"), |p| format!("/repos/{repo}/contents/{}", enc_path(p)));
    let v = gh.call(token, Method::GET, &api, &query, None).await?;
    let entries = v.as_array().ok_or_else(|| bad("That path is a file; use github_file_get."))?;
    let max = usize::try_from(call.int_arg("limit").unwrap_or(100).clamp(1, 200)).unwrap_or(100);
    Ok(entries
        .iter()
        .take(max)
        .map(|e| {
            let kind = str_of(e, "type");
            let mut item = sc.item(str_of(e, "path").to_owned());
            item.title = str_of(e, "name").to_owned();
            item.from = kind.to_owned();
            item.snippet = if kind == "file" {
                format!("file · {} bytes", e["size"].as_u64().unwrap_or(0))
            } else {
                kind.to_owned()
            };
            item.extra = json!({"path": e["path"], "type": kind, "size": e["size"], "sha": e["sha"]})
                .as_object()
                .cloned()
                .unwrap_or_default();
            item
        })
        .collect())
}

async fn tree_get(gh: &GitHub, token: &str, call: &ConnectorCall) -> Result<Vec<Item>, CoreError> {
    let repo = repo_arg(call)?;
    let git_ref = ref_arg(call, "ref")?;
    let sub = optional_path(call, "path")?;
    let recursive = call.bool_arg("recursive").unwrap_or(true);
    let max = usize::try_from(call.int_arg("limit").unwrap_or(1_000).clamp(1, 5_000)).unwrap_or(1_000);
    let sc = scope(gh, token, &repo, git_ref.as_deref()).await?;
    let at = match &git_ref {
        Some(r) => r.clone(),
        None => default_branch(gh, token, &repo).await?,
    };
    let deep = recursive || sub.is_some();
    let query: Vec<(&str, String)> = if deep {
        vec![("recursive", "1".to_owned())]
    } else {
        Vec::new()
    };
    let v = gh.call(token, Method::GET, &format!("/repos/{repo}/git/trees/{}", enc_path(&at)), &query, None).await?;
    let prefix = sub.as_ref().map(|p| format!("{p}/"));
    let mut body = String::new();
    let mut count = 0usize;
    let mut more = v["truncated"].as_bool() == Some(true);
    for entry in v["tree"].as_array().into_iter().flatten() {
        let path = str_of(entry, "path");
        let shown = match &prefix {
            Some(p) => match path.strip_prefix(p.as_str()) {
                Some(rest) => rest,
                None => continue,
            },
            None => path,
        };
        if !recursive && shown.contains('/') {
            continue;
        }
        if count == max {
            more = true;
            break;
        }
        count += 1;
        let size = entry["size"].as_u64().map_or_else(|| "-".to_owned(), |s| s.to_string());
        writeln!(body, "{} {size} {path}", str_of(entry, "type")).ok();
    }
    let (body, cut) = capped(&body, MAX_FILE_TEXT);
    let more = more || cut;
    let mut item = sc.item(format!("{repo}:tree"));
    item.title = format!("Files of {repo} at {at}");
    item.snippet = format!(
        "{count} entries{}",
        if more {
            " (list cut)"
        } else {
            ""
        }
    );
    item.body = Some(body);
    item.extra = json!({"ref": at, "entries": count, "truncated": more, "sha": v["sha"]})
        .as_object()
        .cloned()
        .unwrap_or_default();
    Ok(vec![item])
}

async fn blob_get(gh: &GitHub, token: &str, call: &ConnectorCall) -> Result<Vec<Item>, CoreError> {
    let repo = repo_arg(call)?;
    let sha = sha_arg(call, "sha")?.ok_or_else(|| bad("`sha` is required."))?;
    let (bytes, size) = blob_fetch(gh, token, &repo, &sha).await?;
    let size = bytes.as_ref().map_or(size, |b| b.len() as u64);
    let mut extra = Map::new();
    extra.insert("sha".to_owned(), json!(sha));
    Ok(vec![content_item(
        &Scope::new(&repo, None),
        format!("{repo}:{sha}"),
        &format!("blob {}", short(&sha)),
        bytes.as_deref(),
        size,
        extra,
    )])
}

async fn archive_link(gh: &GitHub, token: &str, call: &ConnectorCall) -> Result<Vec<Item>, CoreError> {
    let repo = repo_arg(call)?;
    let git_ref = ref_arg(call, "ref")?;
    let tar = call.str_arg("format") == Some("tar");
    let kind = if tar {
        "tarball"
    } else {
        "zipball"
    };
    let sc = scope(gh, token, &repo, git_ref.as_deref()).await?;
    let mut api = format!("/repos/{repo}/{kind}");
    if let Some(r) = &git_ref {
        write!(api, "/{}", enc_path(r)).ok();
    }
    let url = redirect_location(gh, token, &api, "application/vnd.github+json").await?;
    let mut item = sc.item(format!("{repo}:archive"));
    item.title = format!(
        "{} archive of {repo}",
        if tar {
            "tar.gz"
        } else {
            "zip"
        }
    );
    item.snippet = "Download link (valid for a short time)".to_owned();
    item.extra =
        json!({"url": url, "format": if tar { "tar.gz" } else { "zip" }, "ref": git_ref, "expires": "shortly"})
            .as_object()
            .cloned()
            .unwrap_or_default();
    Ok(vec![item])
}

// ---- commits --------------------------------------------------------------------------------------------------------

fn moment_arg(call: &ConnectorCall, name: &str) -> Result<Option<String>, CoreError> {
    match call.str_arg(name) {
        None => Ok(None),
        Some(s) => text::parse_when(s)
            .map(|t| Some(text::iso_utc(t)))
            .ok_or_else(|| bad(format!("`{name}` must be a date (2026-10-05) or a moment (2026-10-05T14:00:00Z)."))),
    }
}

async fn commit_list(gh: &GitHub, token: &str, call: &ConnectorCall) -> Result<Vec<Item>, CoreError> {
    let repo = repo_arg(call)?;
    let git_ref = ref_arg(call, "ref")?;
    let mut query = vec![("per_page", limit(call).to_string())];
    if let Some(r) = &git_ref {
        query.push(("sha", r.clone()));
    }
    if let Some(p) = optional_path(call, "path")? {
        query.push(("path", p));
    }
    if let Some(a) = call.str_arg("author") {
        query.push(("author", a.to_owned()));
    }
    for key in ["since", "until"] {
        if let Some(t) = moment_arg(call, key)? {
            query.push((key, t));
        }
    }
    let sc = scope(gh, token, &repo, git_ref.as_deref()).await?;
    let v = gh.call(token, Method::GET, &format!("/repos/{repo}/commits"), &query, None).await?;
    Ok(v.as_array()
        .into_iter()
        .flatten()
        .map(|c| {
            let sha = str_of(c, "sha");
            let login = c["author"]["login"].as_str().unwrap_or_else(|| str_of(&c["commit"]["author"], "name"));
            let mut item = sc.item(sha.to_owned());
            item.title = first_line(str_of(&c["commit"], "message"));
            item.from = text::one_line(login);
            item.snippet = format!("{} · {}", short(sha), item.from);
            item.date = c["commit"]["author"]["date"].as_str().and_then(text::parse_when).unwrap_or(0);
            item.extra = json!({"sha": sha, "url": c["html_url"], "author_login": c["author"]["login"]})
                .as_object()
                .cloned()
                .unwrap_or_default();
            item
        })
        .collect())
}

async fn commit_get(gh: &GitHub, token: &str, call: &ConnectorCall) -> Result<Vec<Item>, CoreError> {
    let repo = repo_arg(call)?;
    let git_ref = required_ref(call, "ref")?;
    let sc = scope(gh, token, &repo, Some(&git_ref)).await?;
    let c = gh.call(token, Method::GET, &format!("/repos/{repo}/commits/{}", enc_path(&git_ref)), &[], None).await?;
    let sha = str_of(&c, "sha");
    let commit = &c["commit"];
    let files = c["files"].as_array().map(Vec::as_slice).unwrap_or_default();
    let mut body = format!(
        "{}\n\nAuthor: {} <{}>\nStats: +{} -{} in {} file(s)\n\n",
        str_of(commit, "message"),
        text::one_line(str_of(&commit["author"], "name")),
        text::one_line(str_of(&commit["author"], "email")),
        c["stats"]["additions"].as_i64().unwrap_or(0),
        c["stats"]["deletions"].as_i64().unwrap_or(0),
        files.len()
    );
    body.push_str(&files_text(files));
    let (body, truncated) = capped(&body, MAX_DIFF);
    let mut item = sc.item(sha.to_owned());
    item.title = first_line(str_of(commit, "message"));
    item.from = text::one_line(str_of(&commit["author"], "name"));
    item.snippet = format!("{} · {} file(s) changed", short(sha), files.len());
    item.date = commit["author"]["date"].as_str().and_then(text::parse_when).unwrap_or(0);
    item.body = Some(body);
    item.extra = json!({
        "sha": sha, "url": c["html_url"], "truncated": truncated, "stats": c["stats"],
        "parents": c["parents"].as_array().map(|p| p.iter().map(|x| x["sha"].clone()).collect::<Vec<_>>()),
    })
    .as_object()
    .cloned()
    .unwrap_or_default();
    Ok(vec![item])
}

async fn commit_compare(gh: &GitHub, token: &str, call: &ConnectorCall) -> Result<Vec<Item>, CoreError> {
    let repo = repo_arg(call)?;
    let base = required_ref(call, "base")?;
    let head = required_ref(call, "head")?;
    let path = format!("/repos/{repo}/compare/{}...{}", enc_path(&base), enc_path(&head));
    let v = gh.call(token, Method::GET, &path, &[("per_page", "100".to_owned())], None).await?;
    let files = v["files"].as_array().map(Vec::as_slice).unwrap_or_default();
    let commits = v["commits"].as_array().map(Vec::as_slice).unwrap_or_default();
    let mut body = format!(
        "{head} is {} ahead of and {} behind {base} (status: {}).\n\nCommits ({} in all, {} shown):\n",
        v["ahead_by"].as_i64().unwrap_or(0),
        v["behind_by"].as_i64().unwrap_or(0),
        str_of(&v, "status"),
        v["total_commits"].as_i64().unwrap_or(0),
        commits.len().min(50)
    );
    for c in commits.iter().take(50) {
        writeln!(body, "{} {}", short(str_of(c, "sha")), first_line(str_of(&c["commit"], "message"))).ok();
    }
    write!(body, "\nFiles ({}):\n{}", files.len(), files_text(files)).ok();
    let (body, truncated) = capped(&body, MAX_DIFF);
    let mut item = Scope::new(&repo, None).item(format!("{repo}:{base}...{head}"));
    item.title = format!("{base}...{head}");
    item.snippet = format!(
        "{} · ahead {} · behind {} · {} file(s)",
        str_of(&v, "status"),
        v["ahead_by"].as_i64().unwrap_or(0),
        v["behind_by"].as_i64().unwrap_or(0),
        files.len()
    );
    item.body = Some(body);
    item.extra = json!({
        "status": v["status"], "ahead_by": v["ahead_by"], "behind_by": v["behind_by"],
        "total_commits": v["total_commits"], "files_changed": files.len(), "truncated": truncated,
        "merge_base": v["merge_base_commit"]["sha"], "url": v["html_url"],
    })
    .as_object()
    .cloned()
    .unwrap_or_default();
    Ok(vec![item])
}

async fn checks_get(gh: &GitHub, token: &str, call: &ConnectorCall) -> Result<Vec<Item>, CoreError> {
    let repo = repo_arg(call)?;
    let git_ref = required_ref(call, "ref")?;
    let sc = scope(gh, token, &repo, Some(&git_ref)).await?;
    let enc = enc_path(&git_ref);
    let status = gh.call(token, Method::GET, &format!("/repos/{repo}/commits/{enc}/status"), &[], None).await?;
    let runs = gh
        .call(
            token,
            Method::GET,
            &format!("/repos/{repo}/commits/{enc}/check-runs"),
            &[("per_page", "100".to_owned())],
            None,
        )
        .await?;
    let statuses = status["statuses"].as_array().map(Vec::as_slice).unwrap_or_default();
    let check_runs = runs["check_runs"].as_array().map(Vec::as_slice).unwrap_or_default();
    let (mut ok, mut failed, mut pending) = (0, 0, 0);
    let mut body = String::new();
    for s in statuses {
        match str_of(s, "state") {
            "success" => ok += 1,
            "pending" => pending += 1,
            _ => failed += 1,
        }
        writeln!(
            body,
            "status {}: {} {}",
            text::one_line(str_of(s, "context")),
            str_of(s, "state"),
            text::one_line(str_of(s, "description"))
        )
        .ok();
    }
    for r in check_runs {
        match (str_of(r, "status"), str_of(r, "conclusion")) {
            ("completed", "success" | "neutral" | "skipped") => ok += 1,
            ("completed", _) => failed += 1,
            _ => pending += 1,
        }
        let outcome = r["conclusion"].as_str().unwrap_or_else(|| str_of(r, "status"));
        writeln!(body, "check {}: {outcome}", text::one_line(str_of(r, "name"))).ok();
    }
    let sha = str_of(&status, "sha");
    let mut item = sc.item(format!("{repo}:{git_ref}:checks"));
    item.title = format!("Checks of {git_ref}");
    item.snippet = format!("{ok} passed, {failed} failed, {pending} pending");
    item.body = Some(body);
    item.extra = json!({
        "sha": sha, "state": status["state"], "passed": ok, "failed": failed, "pending": pending,
        "statuses": statuses.iter().map(|s| json!({"context": s["context"], "state": s["state"], "url": s["target_url"]})).collect::<Vec<_>>(),
        "check_runs": check_runs.iter().map(|r| json!({"name": r["name"], "status": r["status"], "conclusion": r["conclusion"], "url": r["html_url"]})).collect::<Vec<_>>(),
    })
    .as_object()
    .cloned()
    .unwrap_or_default();
    Ok(vec![item])
}

// ---- tags -----------------------------------------------------------------------------------------------------------

async fn tag_list(gh: &GitHub, token: &str, call: &ConnectorCall) -> Result<Vec<Item>, CoreError> {
    let repo = repo_arg(call)?;
    let sc = Scope::new(&repo, None);
    let v = gh
        .call(token, Method::GET, &format!("/repos/{repo}/tags"), &[("per_page", limit(call).to_string())], None)
        .await?;
    Ok(v.as_array()
        .into_iter()
        .flatten()
        .map(|t| {
            let name = str_of(t, "name");
            let sha = str_of(&t["commit"], "sha");
            let mut item = sc.item(name.to_owned());
            item.title = name.to_owned();
            item.snippet = format!("commit {}", short(sha));
            item.extra = json!({"name": name, "commit_sha": sha}).as_object().cloned().unwrap_or_default();
            item
        })
        .collect())
}

async fn tag_get(gh: &GitHub, token: &str, call: &ConnectorCall) -> Result<Vec<Item>, CoreError> {
    let repo = repo_arg(call)?;
    let tag = required_ref(call, "tag")?;
    let r = gh.call(token, Method::GET, &format!("/repos/{repo}/git/ref/tags/{}", enc_path(&tag)), &[], None).await?;
    let object_sha = str_of(&r["object"], "sha").to_owned();
    let mut item = Scope::new(&repo, None).item(tag.clone());
    item.title = tag.clone();
    if str_of(&r["object"], "type") == "tag" {
        let t = gh.call(token, Method::GET, &format!("/repos/{repo}/git/tags/{object_sha}"), &[], None).await?;
        let commit_sha = str_of(&t["object"], "sha");
        item.snippet = format!("annotated tag · commit {}", short(commit_sha));
        item.from = text::one_line(str_of(&t["tagger"], "name"));
        item.date = t["tagger"]["date"].as_str().and_then(text::parse_when).unwrap_or(0);
        item.body = Some(str_of(&t, "message").to_owned()).filter(|m| !m.is_empty());
        item.extra = json!({
            "name": tag, "annotated": true, "tag_sha": object_sha, "commit_sha": commit_sha, "object_type": t["object"]["type"],
            "tagger": {"name": t["tagger"]["name"], "email": t["tagger"]["email"]},
        })
        .as_object()
        .cloned()
        .unwrap_or_default();
    } else {
        item.snippet = format!("lightweight tag · commit {}", short(&object_sha));
        item.extra =
            json!({"name": tag, "annotated": false, "commit_sha": object_sha}).as_object().cloned().unwrap_or_default();
    }
    Ok(vec![item])
}

/// The full sha of a commit, branch or tag, checked to be a commit that exists.
async fn resolve_sha(gh: &GitHub, token: &str, repo: &str, target: &str) -> Result<String, CoreError> {
    let options = Options {
        accept: Some("application/vnd.github.sha"),
        ..Options::default()
    };
    let reply = gh
        .send(token, Method::GET, &format!("/repos/{repo}/commits/{}", enc_path(target)), &[], None, &options)
        .await?;
    let sha = reply.text.trim().to_ascii_lowercase();
    if is_full_sha(&sha) {
        Ok(sha)
    } else {
        Err(bad(format!("`{target}` does not name a commit.")))
    }
}

async fn tag_target(gh: &GitHub, token: &str, repo: &str, call: &ConnectorCall) -> Result<String, CoreError> {
    let target = match ref_arg(call, "target")? {
        Some(t) => t,
        None => default_branch(gh, token, repo).await?,
    };
    resolve_sha(gh, token, repo, &target).await
}

async fn preview_tag_create(gh: &GitHub, token: &str, call: &ConnectorCall) -> Result<Preview, CoreError> {
    let repo = repo_arg(call)?;
    let tag = required_ref(call, "tag")?;
    let message = call.str_arg("message").filter(|m| !m.trim().is_empty());
    let tagger = person_arg(call, "tagger_name", "tagger_email")?;
    if tagger.is_some() && message.is_none() {
        return Err(bad("A tagger can only be given for an annotated tag: add a `message`."));
    }
    if let Some(existing) = get_opt(gh, token, &format!("/repos/{repo}/git/ref/tags/{}", enc_path(&tag)), &[]).await? {
        return Err(bad(format!(
            "The tag `{tag}` already exists (at {}); a tag is never moved. Delete it first if you mean to replace it.",
            short(str_of(&existing["object"], "sha"))
        )));
    }
    let sha = tag_target(gh, token, &repo, call).await?;
    let commit = gh.call(token, Method::GET, &format!("/repos/{repo}/git/commits/{sha}"), &[], None).await?;
    let kind = if message.is_some() {
        "annotated"
    } else {
        "lightweight"
    };
    let mut lines = vec![
        format!(
            "Create the {kind} tag `{tag}` in {repo} at commit {} ({})",
            short(&sha),
            first_line(str_of(&commit, "message"))
        ),
        format!("Points to {sha}."),
    ];
    if let Some(m) = message {
        lines.push(format!("Tag message:\n{}", clip(m, PREVIEW_TEXT)));
    }
    if let Some(t) = &tagger {
        lines.push(person_line(t, "Tagger"));
    }
    Ok(Preview {
        resource: resource(&repo, None),
        resource_label: resource_label(&repo, None),
        parents: parents(&repo, None),
        lines,
        once_only: false,
        ..Preview::default()
    })
}

async fn perform_tag_create(gh: &GitHub, token: &str, call: &ConnectorCall) -> Result<Value, CoreError> {
    let repo = repo_arg(call)?;
    let tag = required_ref(call, "tag")?;
    let message = call.str_arg("message").filter(|m| !m.trim().is_empty());
    let tagger = person_arg(call, "tagger_name", "tagger_email")?;
    if tagger.is_some() && message.is_none() {
        return Err(bad("A tagger can only be given for an annotated tag: add a `message`."));
    }
    let commit_sha = tag_target(gh, token, &repo, call).await?;
    let ref_sha = if let Some(m) = message {
        let mut body = json!({"tag": tag, "message": m, "object": commit_sha, "type": "commit"});
        if let Some(t) = tagger {
            body["tagger"] = t;
        }
        let made = gh.call(token, Method::POST, &format!("/repos/{repo}/git/tags"), &[], Some(&body)).await?;
        made["sha"].as_str().ok_or_else(|| bad("GitHub did not return the tag object."))?.to_owned()
    } else {
        commit_sha.clone()
    };
    let body = json!({"ref": format!("refs/tags/{tag}"), "sha": ref_sha});
    let made = gh.call(token, Method::POST, &format!("/repos/{repo}/git/refs"), &[], Some(&body)).await?;
    Ok(
        json!({"created": true, "tag": tag, "ref": made["ref"], "commit_sha": commit_sha, "annotated": message.is_some()}),
    )
}

async fn preview_tag_delete(gh: &GitHub, token: &str, call: &ConnectorCall) -> Result<Preview, CoreError> {
    let repo = repo_arg(call)?;
    let tag = required_ref(call, "tag")?;
    let r = gh.call(token, Method::GET, &format!("/repos/{repo}/git/ref/tags/{}", enc_path(&tag)), &[], None).await?;
    let release = get_opt(gh, token, &format!("/repos/{repo}/releases/tags/{}", enc_path(&tag)), &[]).await?;
    let mut lines = vec![
        format!("Delete the tag `{tag}` in {repo} (it points to {})", short(str_of(&r["object"], "sha"))),
        "The commit it points to is not deleted.".to_owned(),
    ];
    if let Some(rel) = release {
        lines.push(format!(
            "The release \"{}\" was made from this tag; it is not deleted but loses its tag.",
            text::one_line(if str_of(&rel, "name").is_empty() {
                str_of(&rel, "tag_name")
            } else {
                str_of(&rel, "name")
            })
        ));
    }
    Ok(Preview {
        resource: resource(&repo, None),
        resource_label: resource_label(&repo, None),
        parents: parents(&repo, None),
        lines,
        once_only: false,
        ..Preview::default()
    })
}

async fn perform_tag_delete(gh: &GitHub, token: &str, call: &ConnectorCall) -> Result<Value, CoreError> {
    let repo = repo_arg(call)?;
    let tag = required_ref(call, "tag")?;
    gh.call(token, Method::DELETE, &format!("/repos/{repo}/git/refs/tags/{}", enc_path(&tag)), &[], None).await?;
    Ok(json!({"deleted": true, "tag": tag}))
}

// ---- writing code ---------------------------------------------------------------------------------------------------

/// The branch a change goes to, as far as the preview knows it.
struct Target {
    repo: String,
    branch: String,
    default: String,
    /// The commit the branch points to; `None` when the branch does not exist.
    head: Option<String>,
}

impl Target {
    fn is_default(&self) -> bool {
        self.branch == self.default
    }

    fn preview(&self, lines: Vec<String>) -> Preview {
        Preview {
            resource: resource(&self.repo, Some(&self.branch)),
            resource_label: resource_label(&self.repo, Some(&self.branch)),
            parents: parents(&self.repo, Some(&self.branch)),
            lines,
            once_only: false,
            ..Preview::default()
        }
    }

    /// "on branch feature of octo/cat" or "on main, the default branch of octo/cat".
    fn place(&self) -> String {
        if self.is_default() {
            format!("on {}, the default branch of {}", self.branch, self.repo)
        } else {
            format!("on branch {} of {}", self.branch, self.repo)
        }
    }

    /// The lines saying which head the commit advances and whether that is the default branch.
    fn head_lines(&self) -> Vec<String> {
        let mut lines = Vec::new();
        match &self.head {
            Some(h) => lines.push(format!("Advances {} from commit {}.", self.branch, short(h))),
            None => lines.push(format!("The branch {} does not exist yet.", self.branch)),
        }
        if self.is_default() {
            lines.push(format!("This pushes directly to the default branch {}.", self.branch));
        }
        lines
    }
}

async fn target(gh: &GitHub, token: &str, repo: &str, branch: Option<String>) -> Result<Target, CoreError> {
    let default = default_branch(gh, token, repo).await?;
    let branch = branch.unwrap_or_else(|| default.clone());
    let head = branch_head(gh, token, repo, &branch).await?;
    Ok(Target {
        repo: repo.to_owned(),
        branch,
        default,
        head,
    })
}

/// The branch to change when performing: the one given, else the default branch.
async fn perform_branch(gh: &GitHub, token: &str, repo: &str, call: &ConnectorCall) -> Result<String, CoreError> {
    match ref_arg(call, "branch")? {
        Some(b) => Ok(b),
        None => default_branch(gh, token, repo).await,
    }
}

/// The file at `path` on the branch: its blob sha and size. `None` when there is none.
async fn existing_file(
    gh: &GitHub,
    token: &str,
    repo: &str,
    path: &str,
    branch: &str,
) -> Result<Option<(String, u64)>, CoreError> {
    let api = format!("/repos/{repo}/contents/{}", enc_path(path));
    let Some(v) = get_opt(gh, token, &api, &[("ref", branch.to_owned())]).await? else {
        return Ok(None);
    };
    if v.is_array() {
        return Err(bad(format!("`{path}` is a directory, not a file.")));
    }
    if str_of(&v, "type") != "file" {
        return Err(bad(format!("`{path}` is a {}, not a regular file.", str_of(&v, "type"))));
    }
    Ok(Some((str_of(&v, "sha").to_owned(), v["size"].as_u64().unwrap_or(0))))
}

fn message_of(call: &ConnectorCall) -> Result<&str, CoreError> {
    call.str_arg("message")
        .filter(|m| !m.trim().is_empty())
        .ok_or_else(|| bad("`message` (the commit message) is required."))
}

fn missing_branch(t: &Target) -> CoreError {
    bad(format!(
        "The branch `{}` does not exist in {}. This tool never creates branches; use github_commit_files with \
         `base_branch` to create it with the commit.",
        t.branch, t.repo
    ))
}

async fn preview_file_put(gh: &GitHub, token: &str, call: &ConnectorCall) -> Result<Preview, CoreError> {
    let repo = repo_arg(call)?;
    let path = required_path(call, "path")?;
    let source = file_source(call)?;
    let message = message_of(call)?;
    let given = sha_arg(call, "sha")?;
    let author = person_arg(call, "author_name", "author_email")?;
    let t = target(gh, token, &repo, ref_arg(call, "branch")?).await?;
    if t.head.is_none() {
        return Err(missing_branch(&t));
    }
    let existing = existing_file(gh, token, &repo, &path, &t.branch).await?;
    match (&existing, &given) {
        (Some((current, _)), Some(g)) if current != g => {
            return Err(bad(format!(
                "`{path}` has changed on {}: its current sha is {current}, not {g}. Read it again before replacing it.",
                t.branch
            )));
        }
        (None, Some(_)) => {
            return Err(bad(format!(
                "`{path}` does not exist on {}, so there is no version to replace: drop `sha`.",
                t.branch
            )));
        }
        (Some((current, _)), None) if matches!(&source, Source::Bytes(b) if *current == git_blob_sha(b)) => {
            return Err(bad(format!(
                "`{path}` already has exactly this content on {}; there is nothing to commit.",
                t.branch
            )));
        }
        _ => {}
    }
    let (what, snippet) = match &source {
        Source::Bytes(bytes) => describe(bytes),
        Source::Blob(_) => ("the uploaded file (below)".to_owned(), None),
    };
    let mut lines = vec![format!(
        "{} `{path}` {}",
        if existing.is_some() {
            "Replace the file"
        } else {
            "Create the file"
        },
        t.place()
    )];
    lines.extend(t.head_lines());
    lines.push(format!("Commit message:\n{}", clip(message, PREVIEW_TEXT)));
    match &existing {
        Some((_, old)) => lines.push(format!("New content: {what} (was {old} bytes)")),
        None => lines.push(format!("New content: {what}")),
    }
    if let Some(s) = snippet {
        lines.push(format!("Starts with:\n{s}"));
    }
    if let Some(a) = &author {
        lines.push(person_line(a, "Author"));
    }
    Ok(t.preview(lines))
}

async fn perform_file_put(gh: &GitHub, token: &str, call: &ConnectorCall) -> Result<Value, CoreError> {
    let repo = repo_arg(call)?;
    let path = required_path(call, "path")?;
    let source = file_source(call)?;
    let message = message_of(call)?;
    let branch = perform_branch(gh, token, &repo, call).await?;
    let author = person_arg(call, "author_name", "author_email")?;
    let existing = existing_file(gh, token, &repo, &path, &branch).await?;
    let sha = sha_arg(call, "sha")?.or_else(|| existing.as_ref().map(|(s, _)| s.clone()));
    let mut body = json!({"message": message, "branch": branch});
    if let Some(s) = &sha {
        body["sha"] = json!(s);
    }
    if let Some(a) = author {
        body["author"] = a;
    }
    let api = format!("/repos/{repo}/contents/{}", enc_path(&path));
    let made = match source {
        Source::Bytes(bytes) => {
            body["content"] = json!(BASE64.encode(&bytes));
            gh.call(token, Method::PUT, &api, &[], Some(&body)).await?
        }
        // The server puts the file, base64, into `content` of this body on its way to GitHub.
        Source::Blob(id) => {
            let send = reins_proto::blob::BlobSend {
                v: reins_proto::PROTOCOL_VERSION,
                method: "PUT".to_owned(),
                url: format!("{}{api}", gh.base),
                headers: GitHub::headers(token, "application/vnd.github+json"),
                body: reins_proto::blob::SendBody::JsonBase64 {
                    json: body.as_object().cloned().unwrap_or_default(),
                    field: "content".to_owned(),
                },
            };
            sent(&blob::send(&id, &send).await?)?
        }
    };
    Ok(json!({
        "committed": true, "created": existing.is_none(), "path": path, "branch": branch,
        "commit_sha": made["commit"]["sha"], "content_sha": made["content"]["sha"], "url": made["commit"]["html_url"],
    }))
}

async fn preview_file_delete(gh: &GitHub, token: &str, call: &ConnectorCall) -> Result<Preview, CoreError> {
    let repo = repo_arg(call)?;
    let path = required_path(call, "path")?;
    let message = message_of(call)?;
    let given = sha_arg(call, "sha")?;
    let author = person_arg(call, "author_name", "author_email")?;
    let t = target(gh, token, &repo, ref_arg(call, "branch")?).await?;
    if t.head.is_none() {
        return Err(missing_branch(&t));
    }
    let Some((current, size)) = existing_file(gh, token, &repo, &path, &t.branch).await? else {
        return Err(bad(format!("`{path}` does not exist on {}; there is nothing to delete.", t.branch)));
    };
    if given.as_ref().is_some_and(|g| *g != current) {
        return Err(bad(format!(
            "`{path}` has changed on {}: its current sha is {current}. Read it again first.",
            t.branch
        )));
    }
    let mut lines = vec![format!("Delete the file `{path}` ({size} bytes) {}", t.place())];
    lines.extend(t.head_lines());
    lines.push(format!("Commit message:\n{}", clip(message, PREVIEW_TEXT)));
    if let Some(a) = &author {
        lines.push(person_line(a, "Author"));
    }
    Ok(t.preview(lines))
}

async fn perform_file_delete(gh: &GitHub, token: &str, call: &ConnectorCall) -> Result<Value, CoreError> {
    let repo = repo_arg(call)?;
    let path = required_path(call, "path")?;
    let message = message_of(call)?;
    let branch = perform_branch(gh, token, &repo, call).await?;
    let author = person_arg(call, "author_name", "author_email")?;
    let sha = match sha_arg(call, "sha")? {
        Some(s) => s,
        None => {
            existing_file(gh, token, &repo, &path, &branch)
                .await?
                .ok_or_else(|| bad(format!("`{path}` does not exist on {branch}; there is nothing to delete.")))?
                .0
        }
    };
    let mut body = json!({"message": message, "sha": sha, "branch": branch});
    if let Some(a) = author {
        body["author"] = a;
    }
    let made = gh
        .call(token, Method::DELETE, &format!("/repos/{repo}/contents/{}", enc_path(&path)), &[], Some(&body))
        .await?;
    Ok(
        json!({"committed": true, "deleted": true, "path": path, "branch": branch, "commit_sha": made["commit"]["sha"], "url": made["commit"]["html_url"]}),
    )
}

/// One file of `commit_files`.
struct Change {
    path: String,
    mode: &'static str,
    /// `None` deletes the file.
    data: Option<Vec<u8>>,
}

fn parse_changes(call: &ConnectorCall) -> Result<Vec<Change>, CoreError> {
    let list = call
        .args
        .get("files")
        .and_then(Value::as_array)
        .ok_or_else(|| bad("`files` must be a list of {path, content | content_base64 | delete} objects."))?;
    if list.is_empty() || list.len() > MAX_FILES {
        return Err(bad(format!("`files` must have between 1 and {MAX_FILES} entries.")));
    }
    let mut out: Vec<Change> = Vec::new();
    let mut total = 0usize;
    for (i, entry) in list.iter().enumerate() {
        let n = i + 1;
        let obj = entry.as_object().ok_or_else(|| bad(format!("files[{n}] must be an object.")))?;
        if let Some(k) =
            obj.keys().find(|k| !matches!(k.as_str(), "path" | "content" | "content_base64" | "delete" | "mode"))
        {
            return Err(bad(format!(
                "files[{n}] has an unknown key `{k}` (allowed: path, content, content_base64, delete, mode)."
            )));
        }
        let path = obj
            .get("path")
            .and_then(Value::as_str)
            .filter(|p| path_ok(p))
            .ok_or_else(|| bad(format!("files[{n}]: {}", path_message("path"))))?;
        let text_content = obj.get("content");
        let b64 = obj.get("content_base64");
        let delete = obj.get("delete");
        if delete.is_some_and(|d| d != &json!(true) && d != &json!(false)) {
            return Err(bad(format!("files[{n}]: `delete` must be true or false.")));
        }
        let deleting = delete == Some(&json!(true));
        let given = usize::from(text_content.is_some()) + usize::from(b64.is_some()) + usize::from(deleting);
        if given != 1 {
            return Err(bad(format!(
                "files[{n}] (`{path}`) needs exactly one of `content`, `content_base64` or `delete: true`."
            )));
        }
        let mode = match obj.get("mode").map(|m| m.as_str()) {
            None => "100644",
            Some(Some("100644")) if !deleting => "100644",
            Some(Some("100755")) if !deleting => "100755",
            Some(_) => {
                return Err(bad(format!(
                    "files[{n}]: `mode` must be \"100644\" or \"100755\", and only for a file that is written."
                )));
            }
        };
        let data = if deleting {
            None
        } else if let Some(t) = text_content {
            let t = t.as_str().ok_or_else(|| bad(format!("files[{n}]: `content` must be text.")))?;
            if t.len() > MAX_BINARY {
                return Err(bad(format!("files[{n}] (`{path}`) is larger than 2 MB.")));
            }
            Some(t.as_bytes().to_vec())
        } else {
            let b = b64
                .and_then(Value::as_str)
                .ok_or_else(|| bad(format!("files[{n}]: `content_base64` must be text.")))?;
            Some(decode_b64(&format!("files[{n}].content_base64"), b)?)
        };
        total += data.as_ref().map_or(0, Vec::len);
        if total > MAX_COMMIT_TOTAL {
            return Err(bad("The files of one commit may total at most 4 MB."));
        }
        if let Some(clash) = out.iter().find(|c| {
            c.path == path || c.path.starts_with(&format!("{path}/")) || path.starts_with(&format!("{}/", c.path))
        }) {
            return Err(bad(format!(
                "files[{n}] (`{path}`) clashes with `{}`: a path may appear once and cannot be both a file and a directory.",
                clash.path
            )));
        }
        out.push(Change {
            path: path.to_owned(),
            mode,
            data,
        });
    }
    Ok(out)
}

/// What the branch already holds: path → (blob sha, size), and whether the listing is complete.
async fn existing_tree(
    gh: &GitHub,
    token: &str,
    repo: &str,
    commit: &str,
) -> Result<(BTreeMap<String, (String, u64)>, bool), CoreError> {
    let v = gh
        .call(token, Method::GET, &format!("/repos/{repo}/git/trees/{commit}"), &[("recursive", "1".to_owned())], None)
        .await?;
    let map = v["tree"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|e| str_of(e, "type") == "blob")
        .map(|e| (str_of(e, "path").to_owned(), (str_of(e, "sha").to_owned(), e["size"].as_u64().unwrap_or(0))))
        .collect();
    Ok((map, v["truncated"].as_bool() != Some(true)))
}

/// The branch a commit goes on and the commit it is built on: the branch's own head, or, for a branch to be created,
/// the head of `base_branch`.
struct Basis {
    parent: String,
    creates: bool,
}

async fn basis(
    gh: &GitHub,
    token: &str,
    repo: &str,
    branch: &str,
    head: Option<String>,
    call: &ConnectorCall,
) -> Result<Basis, CoreError> {
    if let Some(parent) = head {
        return Ok(Basis {
            parent,
            creates: false,
        });
    }
    let Some(base) = ref_arg(call, "base_branch")? else {
        return Err(bad(format!(
            "The branch `{branch}` does not exist in {repo}. Give `base_branch` to create it from another branch."
        )));
    };
    match branch_head(gh, token, repo, &base).await? {
        Some(parent) => Ok(Basis {
            parent,
            creates: true,
        }),
        None => Err(bad(format!("`base_branch` `{base}` does not exist in {repo}."))),
    }
}

async fn preview_commit_files(gh: &GitHub, token: &str, call: &ConnectorCall) -> Result<Preview, CoreError> {
    let repo = repo_arg(call)?;
    let message = message_of(call)?;
    let changes = parse_changes(call)?;
    let author = person_arg(call, "author_name", "author_email")?;
    let t = target(gh, token, &repo, ref_arg(call, "branch")?).await?;
    let base = basis(gh, token, &repo, &t.branch, t.head.clone(), call).await?;
    let (existing, complete) = existing_tree(gh, token, &repo, &base.parent).await?;
    let (mut n_new, mut n_changed, mut n_deleted, mut n_same) = (0, 0, 0, 0);
    let mut file_lines = Vec::new();
    for c in &changes {
        let old = existing.get(&c.path);
        match &c.data {
            None => {
                if old.is_none() && complete {
                    return Err(bad(format!(
                        "`{}` does not exist on {}, so it cannot be n_deleted.",
                        c.path,
                        base_name(&t, call)
                    )));
                }
                n_deleted += 1;
                file_lines.push(format!(
                    "  delete {}{}",
                    c.path,
                    old.map_or_else(String::new, |(_, s)| format!(" ({s} bytes)"))
                ));
            }
            Some(bytes) => {
                let (what, snippet) = describe(bytes);
                let action = match old {
                    Some((sha, _)) if *sha == git_blob_sha(bytes) => {
                        n_same += 1;
                        "unchanged"
                    }
                    Some(_) => {
                        n_changed += 1;
                        "update"
                    }
                    None if complete => {
                        n_new += 1;
                        "create"
                    }
                    None => {
                        n_new += 1;
                        "write"
                    }
                };
                let mut line = format!(
                    "  {action} {} ({what}{})",
                    c.path,
                    if c.mode == "100755" {
                        ", executable"
                    } else {
                        ""
                    }
                );
                if let (Some(s), true) = (snippet, action != "unchanged") {
                    write!(line, "\n{s}").ok();
                }
                file_lines.push(line);
            }
        }
    }
    if n_new + n_changed + n_deleted == 0 {
        return Err(bad(format!(
            "Every file already has exactly this content on {}; there is nothing to commit.",
            base_name(&t, call)
        )));
    }
    let mut lines = vec![format!(
        "Commit {} file{} {}: {n_new} new, {n_changed} changed, {n_deleted} deleted{}",
        changes.len(),
        if changes.len() == 1 {
            ""
        } else {
            "s"
        },
        t.place(),
        if n_same > 0 {
            format!(", {n_same} unchanged")
        } else {
            String::new()
        }
    )];
    if base.creates {
        let from = ref_arg(call, "base_branch")?.unwrap_or_default();
        lines.push(format!(
            "Creates the new branch {} from {from} (commit {}) with this commit on top.",
            t.branch,
            short(&base.parent)
        ));
        if t.is_default() {
            lines.push(format!("It becomes the default branch {}.", t.branch));
        }
    } else {
        lines.extend(t.head_lines());
        if call.str_arg("base_branch").is_some() {
            lines.push("`base_branch` is ignored: the branch already exists.".to_owned());
        }
    }
    lines.push(format!("Commit message:\n{}", clip(message, PREVIEW_TEXT)));
    if !complete {
        lines.push(
            "The repository is too large to check which files already exist; \"write\" may be a create or an update."
                .to_owned(),
        );
    }
    lines.push("Files:".to_owned());
    lines.extend(file_lines);
    if let Some(a) = &author {
        lines.push(person_line(a, "Author"));
    }
    Ok(t.preview(lines))
}

/// The name of the branch the files are compared with, for messages.
fn base_name(t: &Target, call: &ConnectorCall) -> String {
    if t.head.is_some() {
        t.branch.clone()
    } else {
        call.str_arg("base_branch").unwrap_or(&t.branch).to_owned()
    }
}

async fn perform_commit_files(gh: &GitHub, token: &str, call: &ConnectorCall) -> Result<Value, CoreError> {
    let repo = repo_arg(call)?;
    let message = message_of(call)?;
    let changes = parse_changes(call)?;
    let author = person_arg(call, "author_name", "author_email")?;
    let branch = perform_branch(gh, token, &repo, call).await?;
    let head = branch_head(gh, token, &repo, &branch).await?;
    let base = basis(gh, token, &repo, &branch, head, call).await?;
    let parent_commit =
        gh.call(token, Method::GET, &format!("/repos/{repo}/git/commits/{}", base.parent), &[], None).await?;
    let base_tree = str_of(&parent_commit["tree"], "sha").to_owned();
    if base_tree.is_empty() {
        return Err(bad("GitHub did not say which tree the branch has."));
    }
    let mut entries = Vec::new();
    for c in &changes {
        match &c.data {
            None => entries.push(json!({"path": c.path, "mode": "100644", "type": "blob", "sha": null})),
            Some(bytes) => {
                let body = json!({"content": BASE64.encode(bytes), "encoding": "base64"});
                let blob = gh.call(token, Method::POST, &format!("/repos/{repo}/git/blobs"), &[], Some(&body)).await?;
                let sha = blob["sha"].as_str().ok_or_else(|| bad("GitHub did not return the new file's id."))?;
                entries.push(json!({"path": c.path, "mode": c.mode, "type": "blob", "sha": sha}));
            }
        }
    }
    let body = json!({"base_tree": base_tree, "tree": entries});
    let tree = gh.call(token, Method::POST, &format!("/repos/{repo}/git/trees"), &[], Some(&body)).await?;
    let tree_sha = tree["sha"].as_str().ok_or_else(|| bad("GitHub did not return the new tree."))?;
    let mut body = json!({"message": message, "tree": tree_sha, "parents": [base.parent]});
    if let Some(a) = author {
        body["author"] = a;
    }
    let commit = gh.call(token, Method::POST, &format!("/repos/{repo}/git/commits"), &[], Some(&body)).await?;
    let commit_sha = commit["sha"].as_str().ok_or_else(|| bad("GitHub did not return the new commit."))?.to_owned();
    if base.creates {
        let body = json!({"ref": format!("refs/heads/{branch}"), "sha": commit_sha});
        gh.call(token, Method::POST, &format!("/repos/{repo}/git/refs"), &[], Some(&body)).await?;
    } else {
        let body = json!({"sha": commit_sha, "force": false});
        gh.call(token, Method::PATCH, &format!("/repos/{repo}/git/refs/heads/{}", enc_path(&branch)), &[], Some(&body))
            .await?;
    }
    Ok(json!({
        "committed": true, "branch": branch, "created_branch": base.creates, "commit_sha": commit_sha,
        "files": changes.len(), "url": commit["html_url"],
    }))
}

// ---- releases -------------------------------------------------------------------------------------------------------

fn release_item(sc: &Scope, r: &Value, full: bool) -> Item {
    let tag = str_of(r, "tag_name");
    let name = str_of(r, "name");
    let assets = r["assets"].as_array().map(Vec::as_slice).unwrap_or_default();
    let state = if r["draft"].as_bool() == Some(true) {
        "draft"
    } else if r["prerelease"].as_bool() == Some(true) {
        "prerelease"
    } else {
        "published"
    };
    let mut item = sc.item(r["id"].as_i64().unwrap_or(0).to_string());
    item.title = text::one_line(if name.is_empty() {
        tag
    } else {
        name
    });
    item.from = text::one_line(str_of(&r["author"], "login"));
    item.snippet = format!("{tag} · {state} · {} asset(s)", assets.len());
    item.date = r["published_at"].as_str().or(r["created_at"].as_str()).and_then(text::parse_when).unwrap_or(0);
    let mut extra = json!({
        "id": r["id"], "tag_name": tag, "name": name, "draft": r["draft"], "prerelease": r["prerelease"],
        "target_commitish": r["target_commitish"], "url": r["html_url"], "asset_count": assets.len(),
    })
    .as_object()
    .cloned()
    .unwrap_or_default();
    if full {
        let (body, truncated) = capped(str_of(r, "body"), MAX_DIFF);
        item.body = Some(body).filter(|b| !b.is_empty());
        if truncated {
            extra.insert("truncated".to_owned(), json!(true));
        }
        extra.insert("assets".to_owned(), Value::Array(assets.iter().map(asset_summary).collect()));
    }
    item.extra = extra;
    item
}

fn asset_summary(a: &Value) -> Value {
    json!({
        "id": a["id"], "name": a["name"], "label": a["label"], "size": a["size"], "content_type": a["content_type"],
        "download_count": a["download_count"], "state": a["state"], "url": a["browser_download_url"],
    })
}

async fn release_list(gh: &GitHub, token: &str, call: &ConnectorCall) -> Result<Vec<Item>, CoreError> {
    let repo = repo_arg(call)?;
    let sc = Scope::new(&repo, None);
    let v = gh
        .call(token, Method::GET, &format!("/repos/{repo}/releases"), &[("per_page", limit(call).to_string())], None)
        .await?;
    Ok(v.as_array().into_iter().flatten().map(|r| release_item(&sc, r, false)).collect())
}

#[derive(Clone, Copy)]
enum Which {
    Id,
    Latest,
    Tag,
}

async fn release_one(gh: &GitHub, token: &str, call: &ConnectorCall, which: Which) -> Result<Vec<Item>, CoreError> {
    let repo = repo_arg(call)?;
    let path = match which {
        Which::Id => format!("/repos/{repo}/releases/{}", id_arg(call, "release_id")?),
        Which::Latest => format!("/repos/{repo}/releases/latest"),
        Which::Tag => format!("/repos/{repo}/releases/tags/{}", enc_path(&required_ref(call, "tag")?)),
    };
    let r = gh.call(token, Method::GET, &path, &[], None).await?;
    Ok(vec![release_item(&Scope::new(&repo, None), &r, true)])
}

async fn release_notes(gh: &GitHub, token: &str, call: &ConnectorCall) -> Result<Vec<Item>, CoreError> {
    let repo = repo_arg(call)?;
    let tag = required_ref(call, "tag_name")?;
    let mut body = json!({"tag_name": tag});
    if let Some(t) = ref_arg(call, "target_commitish")? {
        body["target_commitish"] = json!(t);
    }
    if let Some(t) = ref_arg(call, "previous_tag_name")? {
        body["previous_tag_name"] = json!(t);
    }
    let v = gh.call(token, Method::POST, &format!("/repos/{repo}/releases/generate-notes"), &[], Some(&body)).await?;
    let (notes, truncated) = capped(str_of(&v, "body"), MAX_DIFF);
    let mut item = Scope::new(&repo, None).item(format!("{repo}:{tag}:notes"));
    item.title = text::one_line(str_of(&v, "name"));
    item.snippet = format!("Generated release notes for {tag}");
    item.body = Some(notes);
    item.extra =
        json!({"tag_name": tag, "name": v["name"], "truncated": truncated}).as_object().cloned().unwrap_or_default();
    Ok(vec![item])
}

async fn asset_list(gh: &GitHub, token: &str, call: &ConnectorCall) -> Result<Vec<Item>, CoreError> {
    let repo = repo_arg(call)?;
    let id = id_arg(call, "release_id")?;
    let sc = Scope::new(&repo, None);
    let assets = gh
        .pages(token, &format!("/repos/{repo}/releases/{id}/assets"), &[("per_page", "100".to_owned())], None, 200)
        .await?;
    Ok(assets
        .iter()
        .map(|a| {
            let mut item = sc.item(a["id"].as_i64().unwrap_or(0).to_string());
            item.title = text::one_line(str_of(a, "name"));
            item.snippet = format!(
                "{} bytes · {} · {} download(s)",
                a["size"].as_u64().unwrap_or(0),
                str_of(a, "content_type"),
                a["download_count"].as_u64().unwrap_or(0)
            );
            item.date = a["updated_at"].as_str().and_then(text::parse_when).unwrap_or(0);
            item.extra = asset_summary(a).as_object().cloned().unwrap_or_default();
            item
        })
        .collect())
}

async fn asset_download(gh: &GitHub, token: &str, call: &ConnectorCall) -> Result<Vec<Item>, CoreError> {
    let repo = repo_arg(call)?;
    let id = id_arg(call, "asset_id")?;
    let path = format!("/repos/{repo}/releases/assets/{id}");
    let meta = gh.call(token, Method::GET, &path, &[], None).await?;
    let size = meta["size"].as_u64().unwrap_or(0);
    if blob::as_link(size) {
        let mut item = Scope::new(&repo, None).item(format!("{repo}:asset:{id}"));
        item.title = str_of(&meta, "name").to_owned();
        item.snippet = format!("{size} bytes");
        item.extra = asset_summary(&meta).as_object().cloned().unwrap_or_default();
        let marker = blob::fetch_marker(&format!("{}{path}", gh.base), "application/octet-stream", &item.title, size);
        item.extra.insert(DELIVER_KEY.to_owned(), marker);
        return Ok(vec![item]);
    }
    let bytes = if size > MAX_BINARY as u64 {
        None
    } else {
        let url = redirect_location(gh, token, &path, "application/octet-stream").await?;
        Some(download(gh, &url).await?)
    };
    let size = bytes.as_ref().map_or(size, |b| b.len() as u64);
    let mut extra = asset_summary(&meta).as_object().cloned().unwrap_or_default();
    extra.remove("size");
    Ok(vec![content_item(
        &Scope::new(&repo, None),
        format!("{repo}:asset:{id}"),
        str_of(&meta, "name"),
        bytes.as_deref(),
        size,
        extra,
    )])
}

/// A file name for an asset: no slash, backslash or control character, not `.`/`..`, at most 200 bytes.
fn asset_name_ok(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 200
        && name != "."
        && name != ".."
        && !name.chars().any(|c| c.is_control() || matches!(c, '/' | '\\'))
}

fn asset_name_arg(call: &ConnectorCall) -> Result<Option<String>, CoreError> {
    match call.str_arg("name") {
        None => Ok(None),
        Some(n) if asset_name_ok(n) => Ok(Some(n.to_owned())),
        Some(_) => Err(bad("`name` must be a file name without slashes or control characters (at most 200 bytes).")),
    }
}

fn content_type_ok(ct: &str) -> bool {
    ct.len() <= 100
        && ct.matches('/').count() == 1
        && !ct.starts_with('/')
        && !ct.ends_with('/')
        && ct.bytes().all(|b| b.is_ascii_alphanumeric() || b"!#$&^_.+-/".contains(&b))
}

fn repo_preview(repo: &str, lines: Vec<String>) -> Preview {
    Preview {
        resource: resource(repo, None),
        resource_label: resource_label(repo, None),
        parents: parents(repo, None),
        lines,
        once_only: false,
        ..Preview::default()
    }
}

/// A release field as a string for "old → new" lines.
fn flag(v: &Value) -> String {
    v.as_bool().map_or_else(|| "unset".to_owned(), |b| b.to_string())
}

async fn release_by_id(gh: &GitHub, token: &str, repo: &str, id: i64) -> Result<Value, CoreError> {
    gh.call(token, Method::GET, &format!("/repos/{repo}/releases/{id}"), &[], None).await
}

fn release_name(r: &Value) -> String {
    text::one_line(if str_of(r, "name").is_empty() {
        str_of(r, "tag_name")
    } else {
        str_of(r, "name")
    })
}

/// The fields of a release create or update, checked, as the request body.
fn release_fields(call: &ConnectorCall) -> Result<Map<String, Value>, CoreError> {
    let mut body = Map::new();
    for key in ["tag_name", "target_commitish"] {
        if let Some(v) = ref_arg(call, key)? {
            body.insert(key.to_owned(), json!(v));
        }
    }
    for key in ["name", "body", "make_latest"] {
        if let Some(v) = call.str_arg(key) {
            body.insert(key.to_owned(), json!(v));
        }
    }
    for key in ["draft", "prerelease", "generate_release_notes"] {
        if let Some(v) = call.bool_arg(key) {
            body.insert(key.to_owned(), json!(v));
        }
    }
    Ok(body)
}

async fn preview_release_create(gh: &GitHub, token: &str, call: &ConnectorCall) -> Result<Preview, CoreError> {
    let repo = repo_arg(call)?;
    release_fields(call)?;
    let tag = required_ref(call, "tag_name")?;
    if let Some(r) = get_opt(gh, token, &format!("/repos/{repo}/releases/tags/{}", enc_path(&tag)), &[]).await? {
        return Err(bad(format!(
            "A release for the tag `{tag}` already exists (id {}); use github_release_update.",
            r["id"]
        )));
    }
    let existing_tag = get_opt(gh, token, &format!("/repos/{repo}/git/ref/tags/{}", enc_path(&tag)), &[]).await?;
    let tag_line = if let Some(t) = existing_tag {
        format!("Uses the existing tag `{tag}` (commit {}).", short(str_of(&t["object"], "sha")))
    } else {
        let at = match ref_arg(call, "target_commitish")? {
            Some(t) => t,
            None => default_branch(gh, token, &repo).await?,
        };
        format!("The tag `{tag}` does not exist: GitHub creates it at {at}.")
    };
    let draft = call.bool_arg("draft") == Some(true);
    let name = call.str_arg("name").unwrap_or(&tag);
    let mut lines = vec![format!(
        "{} release \"{}\" for tag `{tag}` in {repo}{}",
        if draft {
            "Create a draft"
        } else {
            "Publish"
        },
        text::one_line(name),
        if call.bool_arg("prerelease") == Some(true) {
            " (marked as a prerelease)"
        } else {
            ""
        }
    )];
    lines.push(tag_line);
    if draft {
        lines.push("It stays unpublished until it is published.".to_owned());
    }
    if let Some(m) = call.str_arg("make_latest") {
        lines.push(format!("make_latest: {m}"));
    }
    if call.bool_arg("generate_release_notes") == Some(true) {
        lines.push("GitHub generates the release notes.".to_owned());
    }
    if let Some(b) = call.str_arg("body") {
        lines.push(format!("Notes:\n{}", clip(b, PREVIEW_TEXT)));
    }
    Ok(repo_preview(&repo, lines))
}

async fn perform_release_create(gh: &GitHub, token: &str, call: &ConnectorCall) -> Result<Value, CoreError> {
    let repo = repo_arg(call)?;
    let body = Value::Object(release_fields(call)?);
    let r = gh.call(token, Method::POST, &format!("/repos/{repo}/releases"), &[], Some(&body)).await?;
    Ok(json!({
        "created": true, "id": r["id"], "tag_name": r["tag_name"], "name": r["name"], "draft": r["draft"],
        "prerelease": r["prerelease"], "url": r["html_url"],
    }))
}

async fn preview_release_update(gh: &GitHub, token: &str, call: &ConnectorCall) -> Result<Preview, CoreError> {
    let repo = repo_arg(call)?;
    let id = id_arg(call, "release_id")?;
    let fields = release_fields(call)?;
    if fields.is_empty() {
        return Err(bad("Give at least one field to change."));
    }
    let r = release_by_id(gh, token, &repo, id).await?;
    let mut lines =
        vec![format!("Change the release \"{}\" (tag `{}`) of {repo}", release_name(&r), str_of(&r, "tag_name"))];
    for key in ["tag_name", "target_commitish", "name"] {
        if let Some(v) = fields.get(key).and_then(Value::as_str) {
            lines.push(format!("{key}: '{}' -> '{}'", text::one_line(str_of(&r, key)), text::one_line(v)));
        }
    }
    for key in ["draft", "prerelease"] {
        if let Some(v) = fields.get(key) {
            lines.push(format!("{key}: {} -> {}", flag(&r[key]), flag(v)));
        }
    }
    if fields.get("draft") == Some(&json!(false)) && r["draft"].as_bool() == Some(true) {
        lines.push("This publishes the release.".to_owned());
    }
    if let Some(m) = fields.get("make_latest").and_then(Value::as_str) {
        lines.push(format!("make_latest: {m}"));
    }
    if let Some(b) = fields.get("body").and_then(Value::as_str) {
        lines.push(format!(
            "Notes are replaced ({} -> {} characters):\n{}",
            str_of(&r, "body").chars().count(),
            b.chars().count(),
            clip(b, PREVIEW_TEXT)
        ));
    }
    Ok(repo_preview(&repo, lines))
}

async fn perform_release_update(gh: &GitHub, token: &str, call: &ConnectorCall) -> Result<Value, CoreError> {
    let repo = repo_arg(call)?;
    let id = id_arg(call, "release_id")?;
    let fields = release_fields(call)?;
    if fields.is_empty() {
        return Err(bad("Give at least one field to change."));
    }
    let r = gh
        .call(token, Method::PATCH, &format!("/repos/{repo}/releases/{id}"), &[], Some(&Value::Object(fields)))
        .await?;
    Ok(json!({
        "updated": true, "id": r["id"], "tag_name": r["tag_name"], "name": r["name"], "draft": r["draft"],
        "prerelease": r["prerelease"], "url": r["html_url"],
    }))
}

async fn preview_release_delete(gh: &GitHub, token: &str, call: &ConnectorCall) -> Result<Preview, CoreError> {
    let repo = repo_arg(call)?;
    let r = release_by_id(gh, token, &repo, id_arg(call, "release_id")?).await?;
    let assets = r["assets"].as_array().map_or(0, Vec::len);
    let lines = vec![
        format!(
            "Delete the {} release \"{}\" (tag `{}`) of {repo} with its {assets} asset(s)",
            if r["draft"].as_bool() == Some(true) {
                "draft"
            } else {
                "published"
            },
            release_name(&r),
            str_of(&r, "tag_name")
        ),
        "The tag itself is not deleted.".to_owned(),
    ];
    Ok(repo_preview(&repo, lines))
}

async fn perform_release_delete(gh: &GitHub, token: &str, call: &ConnectorCall) -> Result<Value, CoreError> {
    let repo = repo_arg(call)?;
    let id = id_arg(call, "release_id")?;
    gh.call(token, Method::DELETE, &format!("/repos/{repo}/releases/{id}"), &[], None).await?;
    Ok(json!({"deleted": true, "id": id}))
}

struct Upload {
    name: String,
    label: Option<String>,
    content_type: String,
    source: Source,
}

fn upload_args(call: &ConnectorCall) -> Result<Upload, CoreError> {
    let name = asset_name_arg(call)?.ok_or_else(|| bad("`name` is required."))?;
    let source = match (call.str_arg("blob").filter(|b| !b.is_empty()), call.str_arg("content_base64")) {
        (Some(_), Some(_)) => return Err(bad("Give either `content_base64` or `blob`, not both.")),
        (Some(id), None) => Source::Blob(id.to_owned()),
        (None, encoded) => {
            let bytes = decode_b64("content_base64", encoded.ok_or_else(|| bad("`content_base64` is required."))?)?;
            if bytes.is_empty() {
                return Err(bad("The file is empty; GitHub does not accept empty assets."));
            }
            Source::Bytes(bytes)
        }
    };
    let content_type = call.str_arg("content_type").unwrap_or("application/octet-stream");
    if !content_type_ok(content_type) {
        return Err(bad("`content_type` must look like application/zip."));
    }
    Ok(Upload {
        name,
        label: call.str_arg("label").map(str::to_owned),
        content_type: content_type.to_owned(),
        source,
    })
}

async fn preview_asset_upload(gh: &GitHub, token: &str, call: &ConnectorCall) -> Result<Preview, CoreError> {
    let repo = repo_arg(call)?;
    let id = id_arg(call, "release_id")?;
    let up = upload_args(call)?;
    let r = release_by_id(gh, token, &repo, id).await?;
    if r["assets"].as_array().into_iter().flatten().any(|a| str_of(a, "name") == up.name) {
        return Err(bad(format!(
            "The release already has an asset named `{}`; delete it first or use another name.",
            up.name
        )));
    }
    let what = match &up.source {
        Source::Bytes(bytes) => describe(bytes).0,
        Source::Blob(_) => "the uploaded file (below)".to_owned(),
    };
    let mut lines = vec![format!(
        "Attach `{}` to the release \"{}\" (tag `{}`) of {repo}",
        up.name,
        release_name(&r),
        str_of(&r, "tag_name")
    )];
    lines.push(format!("File: {what}, type {}", up.content_type));
    if let Some(l) = &up.label {
        lines.push(format!("Label: {}", text::one_line(l)));
    }
    if r["draft"].as_bool() != Some(true) {
        lines.push("The release is published: the file becomes downloadable at once.".to_owned());
    }
    Ok(repo_preview(&repo, lines))
}

async fn perform_asset_upload(gh: &GitHub, token: &str, call: &ConnectorCall) -> Result<Value, CoreError> {
    let repo = repo_arg(call)?;
    let id = id_arg(call, "release_id")?;
    let up = upload_args(call)?;
    let mut query = vec![("name", up.name.clone())];
    if let Some(l) = &up.label {
        query.push(("label", l.clone()));
    }
    let api = format!("/repos/{repo}/releases/{id}/assets");
    let made = match up.source {
        Source::Bytes(bytes) => {
            let options = Options {
                upload: true,
                raw: Some((up.content_type.as_str(), bytes)),
                ..Options::default()
            };
            gh.send(token, Method::POST, &api, &query, None, &options).await?.json
        }
        // The server streams the uploaded file to GitHub's uploads host as the body.
        Source::Blob(blob_id) => {
            let mut url = url::Url::parse(&format!("{}{api}", gh.upload_base))
                .map_err(|_| bad("The release cannot take uploads."))?;
            for (k, v) in &query {
                url.query_pairs_mut().append_pair(k, v);
            }
            let mut headers = GitHub::headers(token, "application/vnd.github+json");
            headers.push(("Content-Type".to_owned(), up.content_type.clone()));
            let send = reins_proto::blob::BlobSend {
                v: reins_proto::PROTOCOL_VERSION,
                method: "POST".to_owned(),
                url: url.into(),
                headers,
                body: reins_proto::blob::SendBody::Raw,
            };
            sent(&blob::send(&blob_id, &send).await?)?
        }
    };
    Ok(json!({
        "uploaded": true, "id": made["id"], "name": made["name"], "size": made["size"], "state": made["state"],
        "url": made["browser_download_url"],
    }))
}

async fn asset_by_id(gh: &GitHub, token: &str, repo: &str, id: i64) -> Result<Value, CoreError> {
    gh.call(token, Method::GET, &format!("/repos/{repo}/releases/assets/{id}"), &[], None).await
}

fn asset_update_fields(call: &ConnectorCall) -> Result<Map<String, Value>, CoreError> {
    let mut body = Map::new();
    if let Some(n) = asset_name_arg(call)? {
        body.insert("name".to_owned(), json!(n));
    }
    if let Some(l) = call.str_arg("label") {
        body.insert("label".to_owned(), json!(l));
    }
    if body.is_empty() {
        return Err(bad("Give a new `name` and/or `label`."));
    }
    Ok(body)
}

async fn preview_asset_update(gh: &GitHub, token: &str, call: &ConnectorCall) -> Result<Preview, CoreError> {
    let repo = repo_arg(call)?;
    let fields = asset_update_fields(call)?;
    let a = asset_by_id(gh, token, &repo, id_arg(call, "asset_id")?).await?;
    let mut lines = vec![format!("Change the release asset `{}` of {repo}", text::one_line(str_of(&a, "name")))];
    for key in ["name", "label"] {
        if let Some(v) = fields.get(key).and_then(Value::as_str) {
            lines.push(format!("{key}: '{}' -> '{}'", text::one_line(str_of(&a, key)), text::one_line(v)));
        }
    }
    Ok(repo_preview(&repo, lines))
}

async fn perform_asset_update(gh: &GitHub, token: &str, call: &ConnectorCall) -> Result<Value, CoreError> {
    let repo = repo_arg(call)?;
    let id = id_arg(call, "asset_id")?;
    let fields = asset_update_fields(call)?;
    let a = gh
        .call(token, Method::PATCH, &format!("/repos/{repo}/releases/assets/{id}"), &[], Some(&Value::Object(fields)))
        .await?;
    Ok(
        json!({"updated": true, "id": a["id"], "name": a["name"], "label": a["label"], "url": a["browser_download_url"]}),
    )
}

async fn preview_asset_delete(gh: &GitHub, token: &str, call: &ConnectorCall) -> Result<Preview, CoreError> {
    let repo = repo_arg(call)?;
    let a = asset_by_id(gh, token, &repo, id_arg(call, "asset_id")?).await?;
    let lines = vec![format!(
        "Delete the release asset `{}` ({} bytes, {} download(s)) of {repo}",
        text::one_line(str_of(&a, "name")),
        a["size"].as_u64().unwrap_or(0),
        a["download_count"].as_u64().unwrap_or(0)
    )];
    Ok(repo_preview(&repo, lines))
}

async fn perform_asset_delete(gh: &GitHub, token: &str, call: &ConnectorCall) -> Result<Value, CoreError> {
    let repo = repo_arg(call)?;
    let id = id_arg(call, "asset_id")?;
    gh.call(token, Method::DELETE, &format!("/repos/{repo}/releases/assets/{id}"), &[], None).await?;
    Ok(json!({"deleted": true, "id": id}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_paths_are_checked_strictly() {
        for ok in ["a", "src/main.rs", ".github/workflows/ci.yml", "dir/a..b", "with space/x y.txt", "ünï/cödé.md"]
        {
            assert!(path_ok(ok), "{ok}");
        }
        let long = "a/".repeat(600);
        for bad in [
            "",
            "/abs",
            "a/",
            "a//b",
            "../x",
            "a/../b",
            "a/./b",
            "./a",
            "a\\b",
            "a\nb",
            "a\u{7}",
            ".git/config",
            "x/.GIT/y",
            "..",
            &long,
        ] {
            assert!(!path_ok(bad), "{bad:?}");
        }
        assert!(path_ok(&"a".repeat(1_024)) && !path_ok(&"a".repeat(1_025)));
    }

    #[test]
    fn paths_are_encoded_segment_by_segment() {
        assert_eq!(enc_path("a b/c#d/é.txt"), "a%20b/c%23d/%C3%A9.txt");
        assert_eq!(enc_path("feature/x"), "feature/x");
    }

    #[test]
    fn blob_ids_match_git() {
        assert_eq!(git_blob_sha(b""), "e69de29bb2d1d6434b8b29ae775ad8c2e48c5391");
        assert_eq!(git_blob_sha(b"hello\n"), "ce013625030ba8dba906f756967f9e9ca394464a");
    }

    #[test]
    fn asset_names_and_types() {
        assert!(asset_name_ok("app-1.0.apk") && asset_name_ok("my file (1).zip"));
        for bad in ["", "..", ".", "a/b", "a\\b", "a\nb"] {
            assert!(!asset_name_ok(bad), "{bad:?}");
        }
        assert!(content_type_ok("application/zip") && content_type_ok("application/vnd.android.package-archive"));
        for bad in ["zip", "/zip", "a/b/c", "a/b\r\nX: y", "text/plain; charset=utf-8", ""] {
            assert!(!content_type_ok(bad), "{bad:?}");
        }
    }
}
