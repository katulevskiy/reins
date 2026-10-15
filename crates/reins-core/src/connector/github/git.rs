//! Git on the user's computer, through the Reins desktop app (see `connector::git` for what every host shares).
//! GitHub looks the repository up with the token and expects the token with the user name `x-access-token`.

use reins_proto::connector::ConnectorCall;
use reins_proto::desktop::{
    CommitInfo, FileChange, FileStatus, GIT_FETCH_OP, MAX_COMMIT_FILES, MAX_PATCH_LINE, MAX_PATCH_LINES,
};
use reqwest::Method;
use serde_json::Value;

use super::{GitHub, Preview, repo_arg};
use crate::CoreError;
use crate::connector::Item;
use crate::connector::git;

pub(crate) use crate::connector::git::summary_arg;

/// The HTTP basic user name GitHub expects with a token.
const USERNAME: &str = "x-access-token";

/// Lists and reads. `None` when the operation is not one of this area's.
pub(super) async fn fetch(gh: &GitHub, token: &str, call: &ConnectorCall) -> Option<Result<Vec<Item>, CoreError>> {
    (call.op == GIT_FETCH_OP).then_some(())?;
    Some(git_fetch(gh, token, call).await)
}

/// What a write would do. `None` when the operation is not one of this area's.
pub(super) fn preview(call: &ConnectorCall) -> Option<Result<Preview, CoreError>> {
    git::is_push(call).then(|| git::preview_push(call, &repo_arg(call)?))
}

/// Does the write: hands out the push credential. `None` when the operation is not one of this area's.
pub(super) fn perform(token: &str, call: &ConnectorCall) -> Option<Result<Value, CoreError>> {
    git::is_push(call).then(|| git::perform_push(call, &repo_arg(call)?, USERNAME, token))
}

/// The most bytes of patches kept for one commit loaded for an Activity entry, as the desktop app's own budget.
const PATCH_BUDGET: usize = 100_000;
/// A commit's subject and author, and a file's path, at most as long as in a push's summary.
const MAX_LINE: usize = 200;
const MAX_PATH: usize = 1_024;

/// One commit of `repo` with its files and their patches (an Activity entry's push whose changes were not kept on this
/// device), capped as the desktop app caps a push's summary.
pub(super) async fn commit_change(gh: &GitHub, token: &str, repo: &str, sha: &str) -> Result<CommitInfo, CoreError> {
    if !super::repo_ok(repo)
        || sha.len() != 40
        || !sha.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(CoreError::invalid("not a commit of a GitHub repository"));
    }
    let c = gh.call(token, Method::GET, &format!("/repos/{repo}/commits/{sha}"), &[], None).await?;
    commit_info(sha, &c)
}

/// GitHub's answer for commit `sha` as a push summary's commit.
fn commit_info(sha: &str, c: &Value) -> Result<CommitInfo, CoreError> {
    if c["sha"].as_str() != Some(sha) {
        return Err(CoreError::service("GitHub answered with another commit"));
    }
    // Kept as GitHub sent it, but cut: cleaned (and hidden characters flagged) when shown, as a push's are.
    let text = |v: &Value| v.as_str().unwrap_or_default().to_owned();
    let line = |s: String| crate::text::truncate_chars(&s, MAX_LINE);
    let commit = &c["commit"];
    let subject = commit["message"].as_str().unwrap_or_default().lines().next().unwrap_or_default().to_owned();
    let files = c["files"].as_array().map(Vec::as_slice).unwrap_or_default();
    let mut budget = PATCH_BUDGET;
    let count = |v: &Value| v.as_u64().and_then(|n| u32::try_from(n).ok());
    Ok(CommitInfo {
        sha: sha.to_owned(),
        subject: line(subject),
        author: line(format!("{} <{}>", text(&commit["author"]["name"]), text(&commit["author"]["email"]))),
        date: commit["author"]["date"].as_str().and_then(crate::text::parse_when).unwrap_or(0),
        files_changed: u32::try_from(files.len()).ok(),
        files: files
            .iter()
            .take(MAX_COMMIT_FILES)
            .map(|f| {
                let status = match f["status"].as_str() {
                    Some("added") => FileStatus::Added,
                    Some("removed") => FileStatus::Deleted,
                    Some("changed") => FileStatus::TypeChanged,
                    _ => FileStatus::Modified,
                };
                let path = crate::text::truncate_chars(f["filename"].as_str().unwrap_or_default(), MAX_PATH);
                let mut file = FileChange::new(path, status, count(&f["additions"]), count(&f["deletions"]), false);
                if let Some(raw) = f["patch"].as_str() {
                    let (lines, cut) = capped_patch(raw);
                    if lines.len() <= budget {
                        budget -= lines.len();
                        file.patch = Some(lines);
                        file.patch_cut = cut;
                    }
                }
                file
            })
            .collect(),
        additions: c["stats"]["additions"].as_u64(),
        deletions: c["stats"]["deletions"].as_u64(),
    })
}

/// A patch as the desktop app sends one: at most [`MAX_PATCH_LINES`] lines (`true`: cut) of at most [`MAX_PATCH_LINE`]
/// characters after the sign (the last one "…"), control characters but tabs replaced.
fn capped_patch(raw: &str) -> (String, bool) {
    let lines: Vec<&str> = raw.split('\n').collect();
    let cut = lines.len() > MAX_PATCH_LINES;
    let out: Vec<String> = lines
        .iter()
        .take(MAX_PATCH_LINES)
        .map(|l| {
            let l = l.strip_suffix('\r').unwrap_or(l);
            let mut chars = l.chars();
            let sign: String = chars.next().into_iter().collect();
            let rest: Vec<char> = chars.collect();
            let mut s = sign;
            // A hunk header is kept whole (up to the line limit), like any other line.
            for (i, ch) in rest.iter().enumerate() {
                if rest.len() > MAX_PATCH_LINE && i == MAX_PATCH_LINE - 1 {
                    s.push('…');
                    break;
                }
                s.push(if ch.is_control() && *ch != '\t' {
                    '\u{fffd}'
                } else {
                    *ch
                });
            }
            s
        })
        .collect();
    (out.join("\n"), cut)
}

async fn git_fetch(gh: &GitHub, token: &str, call: &ConnectorCall) -> Result<Vec<Item>, CoreError> {
    let repo = repo_arg(call)?;
    git::check_fetch(call)?;
    // The token must reach the repository; otherwise the usual error says why.
    let found = gh.call(token, Method::GET, &format!("/repos/{repo}"), &[], None).await?;
    let private = found["private"].as_bool() == Some(true);
    Ok(vec![git::fetch_item(call, &repo, private, USERNAME, token)?])
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHA: &str = "abcdef0123456789abcdef0123456789abcdef01";

    #[test]
    fn a_loaded_commit_is_capped_like_a_push_summary() {
        let long = format!("@@ -1 +1 @@\n+{}\n+tab\there\u{1b}[0m", "y".repeat(500));
        let many = (0..MAX_PATCH_LINES + 50).map(|i| format!("+l{i}")).collect::<Vec<_>>().join("\n");
        let answer = serde_json::json!({
            "sha": SHA,
            "commit": {"message": "Fix the thing\n\nbody", "author": {"name": "Ann\u{202E}", "email": "ann@example.com",
                "date": "2026-10-01T10:00:00Z"}},
            "stats": {"additions": 3, "deletions": 1},
            "files": [
                {"filename": "src/a.rs", "status": "modified", "additions": 2, "deletions": 1, "patch": long},
                {"filename": "big.rs", "status": "added", "additions": 500, "deletions": 0,
                    "patch": format!("@@ -0,0 +1,500 @@\n{many}")},
                {"filename": "logo.png", "status": "removed", "additions": 0, "deletions": 0},
            ],
        });
        let c = commit_info(SHA, &answer).unwrap();
        assert_eq!(c.subject, "Fix the thing");
        assert!(c.date > 0);
        assert_eq!((c.files_changed, c.additions, c.deletions), (Some(3), Some(3), Some(1)));
        let lines: Vec<&str> = c.files[0].patch.as_deref().unwrap().lines().collect();
        assert_eq!(lines[1].chars().count(), MAX_PATCH_LINE + 1, "the sign and the line, cut with …");
        assert!(lines[1].ends_with('…'));
        assert_eq!(lines[2], "+tab\there\u{fffd}[0m", "control characters but tabs replaced");
        assert!(c.files[1].patch_cut);
        assert_eq!(c.files[1].patch.as_deref().unwrap().split('\n').count(), MAX_PATCH_LINES);
        assert_eq!((c.files[2].status, c.files[2].patch.as_deref()), (FileStatus::Deleted, None));
        // Shown through the same view as a push's commit: invisible characters flagged.
        let view = crate::views::commit_view(&c);
        assert!(view.hidden_chars);
        assert_eq!(view.files[0].diff[0].kind, crate::types::DiffLineKind::Hunk);
        assert!(commit_info("0000000000000000000000000000000000000000", &answer).is_err(), "another commit");
    }

    #[test]
    fn patches_past_the_budget_are_left_out() {
        let patch = format!("@@ -1 +1 @@\n{}", vec![format!("+{}", "x".repeat(200)); 399].join("\n"));
        let files: Vec<Value> = (0..MAX_COMMIT_FILES + 10)
            .map(|i| serde_json::json!({"filename": format!("f{i}"), "status": "modified", "patch": patch}))
            .collect();
        let c = commit_info(SHA, &serde_json::json!({"sha": SHA, "files": files})).unwrap();
        assert_eq!(c.files.len(), MAX_COMMIT_FILES);
        let kept: usize = c.files.iter().filter_map(|f| f.patch.as_ref()).map(String::len).sum();
        assert!(kept <= PATCH_BUDGET && kept > 0);
        assert!(c.files.last().unwrap().patch.is_none());
    }
}
