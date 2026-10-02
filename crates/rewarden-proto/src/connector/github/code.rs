//! GitHub tools: files, commits, tags and releases.

use crate::connector::{Effect, GITHUB, LIMIT, Param, ToolSpec, bool_p, choice_p, int_p, json_p, str_p, text_p, tool};

const REPO: Param = str_p("repo", 140, true, "owner/name.");
const REF: Param = str_p(
    "ref",
    250,
    false,
    "A branch, tag or commit SHA. Default: the repository's default branch. Reading a branch is covered by a permission \
     for that branch; a tag or a commit SHA is covered by a permission for the whole repository.",
);
const RELEASE_ID: Param =
    int_p("release_id", 1, i64::MAX, None, "The release id (from the release list or get tools).");
const ASSET_ID: Param = int_p("asset_id", 1, i64::MAX, None, "The asset id (from github_release_asset_list).");
const AUTHOR_NAME: Param = str_p("author_name", 100, false, "Author name; give it together with author_email.");
const AUTHOR_EMAIL: Param = str_p(
    "author_email",
    254,
    false,
    "Author email; give it together with author_name. Without both, GitHub uses the token's user.",
);
const BRANCH: Param = str_p(
    "branch",
    250,
    false,
    "The branch to change. Default: the repository's default branch (the preview says so when it is).",
);
const MESSAGE: Param = text_p("message", 5_000, true, "The commit message.");
const PATH: Param = str_p(
    "path",
    1_024,
    true,
    "Path of the file inside the repository, e.g. `src/main.rs` or `.github/workflows/ci.yml` (no leading slash, no `..`).",
);
const TAG: Param = str_p("tag", 250, true, "The tag name, e.g. v1.2.0.");

#[allow(clippy::too_many_lines, reason = "one description per tool")]
pub(super) fn tools() -> Vec<ToolSpec> {
    #[allow(unused_imports, reason = "the areas use whichever effects they have tools for")]
    use Effect::{List, Read, Search, Write};
    vec![
        // ---- contents ----
        tool(
            "github_file_get",
            GITHUB,
            "file_get",
            Read,
            "Read a file from GitHub",
            "Reads one file of a repository. Text comes back as text (at most 100000 characters, `truncated` says when \
             it was cut); a binary file comes back as base64 in `content_base64` when it is at most 2 MB, otherwise \
             only its size and sha. The user chooses what to share.",
            vec![REPO, PATH, REF],
            Some("repo"),
        ),
        tool(
            "github_dir_list",
            GITHUB,
            "dir_list",
            List,
            "List a directory of a GitHub repository",
            "Lists the files and folders directly inside one directory (name, kind, size). Use github_tree_get for the \
             whole tree.",
            vec![
                REPO,
                str_p("path", 1_024, false, "The directory. Default: the top of the repository."),
                REF,
                int_p("limit", 1, 200, Some(100), "How many entries at most."),
            ],
            Some("repo"),
        ),
        tool(
            "github_tree_get",
            GITHUB,
            "tree_get",
            Read,
            "Read the file tree of a GitHub repository",
            "Lists the paths of a repository at a ref as one text (one `type size path` line each), recursively by \
             default. Cut when the repository is large (`truncated`).",
            vec![
                REPO,
                REF,
                str_p("path", 1_024, false, "Only entries under this directory. Default: everything."),
                bool_p("recursive", "Go into sub-directories. Default: true."),
                int_p("limit", 1, 5_000, Some(1_000), "How many entries at most."),
            ],
            Some("repo"),
        ),
        tool(
            "github_blob_get",
            GITHUB,
            "blob_get",
            Read,
            "Read a git blob from GitHub",
            "Reads the raw content of a git blob by its sha (from a tree or a commit). Text comes back as text, binary \
             as base64 when at most 2 MB, otherwise only its size.",
            vec![REPO, str_p("sha", 64, true, "The blob sha (40 hexadecimal characters).")],
            Some("repo"),
        ),
        tool(
            "github_archive_link",
            GITHUB,
            "archive_link",
            Read,
            "Get a download link for a GitHub repository",
            "Returns a short-lived URL that downloads the repository at a ref as a zip or tar.gz archive. Nothing is \
             downloaded here.",
            vec![
                REPO,
                REF,
                choice_p("format", &["zip", "tar"], false, "zip or tar (a tar.gz). Default: zip."),
            ],
            Some("repo"),
        ),
        // ---- commits ----
        tool(
            "github_commit_list",
            GITHUB,
            "commit_list",
            Read,
            "List GitHub commits",
            "Lists commits, newest first, optionally only those touching a path, by an author, or in a time range.",
            vec![
                REPO,
                str_p("ref", 250, false, "Branch, tag or commit SHA to start from. Default: the default branch."),
                str_p("path", 1_024, false, "Only commits that touch this file or directory."),
                str_p("author", 100, false, "Only commits by this GitHub login or email address."),
                str_p("since", 40, false, "Only commits after this date or moment (2026-10-05 or 2026-10-05T14:00:00Z)."),
                str_p("until", 40, false, "Only commits before this date or moment."),
                LIMIT,
            ],
            Some("repo"),
        ),
        tool(
            "github_commit_get",
            GITHUB,
            "commit_get",
            Read,
            "Read a GitHub commit",
            "Reads one commit: message, author, statistics and the changed files with their patches (at most 60000 \
             characters in all, `truncated` says when).",
            vec![REPO, str_p("ref", 250, true, "A commit SHA, or a branch or tag name.")],
            Some("repo"),
        ),
        tool(
            "github_commit_compare",
            GITHUB,
            "commit_compare",
            Read,
            "Compare two GitHub refs",
            "Compares two branches, tags or commits: how far `head` is ahead of and behind `base`, the commits and the \
             changed files with patches (at most 60000 characters, `truncated` says when).",
            vec![
                REPO,
                str_p("base", 250, true, "The base: branch, tag or commit SHA."),
                str_p("head", 250, true, "The head: branch, tag or commit SHA."),
            ],
            Some("repo"),
        ),
        tool(
            "github_checks_get",
            GITHUB,
            "checks_get",
            Read,
            "Read the checks and statuses of a GitHub ref",
            "Summarises the commit statuses and check runs (CI results) of a commit, branch or tag: overall state and \
             each check with its outcome.",
            vec![REPO, str_p("ref", 250, true, "A commit SHA, or a branch or tag name.")],
            Some("repo"),
        ),
        // ---- writing code ----
        tool(
            "github_file_put",
            GITHUB,
            "file_put",
            Write,
            "Create or change a file on GitHub",
            "Commits one file to a branch: creates it, or replaces it when it exists (the current version is looked up \
             for you; pass `sha` to guard against a change you have not seen). Give the content either as `content` \
             (text) or `content_base64` (at most 2 MB decoded). This is how a workflow file under .github/workflows is \
             edited. It never forces anything and never creates the branch (use github_commit_files with base_branch \
             for that). The commit goes straight to the branch: when that is the default branch, the preview says so.",
            vec![
                REPO,
                PATH,
                text_p("content", 2_000_000, false, "The new content of the file as text, kept exactly."),
                text_p("content_base64", 3_000_000, false, "The new content as base64 (for binary files), at most 2 MB decoded."),
                str_p("blob", 64, false, "A file uploaded to the link the phone gave, instead of content_base64 (for files over 2 MB)."),
                MESSAGE,
                BRANCH,
                str_p("sha", 64, false, "The blob sha of the version you are replacing (from github_file_get); omit to use the current one."),
                AUTHOR_NAME,
                AUTHOR_EMAIL,
            ],
            Some("repo"),
        )
        .in_class("code"),
        tool(
            "github_file_delete",
            GITHUB,
            "file_delete",
            Write,
            "Delete a file on GitHub",
            "Commits the deletion of one file on a branch. The user approves on their phone and sees the file and the branch.",
            vec![
                REPO,
                PATH,
                MESSAGE,
                BRANCH,
                str_p("sha", 64, false, "The blob sha of the file you are deleting; omit to use the current one."),
                AUTHOR_NAME,
                AUTHOR_EMAIL,
            ],
            Some("repo"),
        )
        .in_class("code"),
        tool(
            "github_commit_files",
            GITHUB,
            "commit_files",
            Write,
            "Commit several files to GitHub at once",
            "Makes one commit that adds, changes and deletes several files, through the git data API. `files` is a list \
             (at most 100) of objects: {\"path\": \"src/a.rs\", \"content\": \"text\"} or {\"path\": ..., \
             \"content_base64\": \"...\"} to write a file (each at most 2 MB decoded, 4 MB in all), or {\"path\": ..., \
             \"delete\": true} to delete one; an optional \"mode\" is \"100644\" (default) or \"100755\" (executable). \
             The commit is added to `branch` (default: the default branch) as a normal fast-forward; nothing is ever \
             forced. When `branch` does not exist and `base_branch` is given, the branch is created from the head of \
             base_branch with the commit on top. The preview lists every path with its action and size.",
            vec![
                REPO,
                MESSAGE,
                json_p("files", 6_000_000, true, "The list of files, see the description."),
                BRANCH,
                str_p("base_branch", 250, false, "Create `branch` from this branch when `branch` does not exist yet."),
                AUTHOR_NAME,
                AUTHOR_EMAIL,
            ],
            Some("repo"),
        )
        .in_class("code"),
        // ---- tags ----
        tool(
            "github_tag_list",
            GITHUB,
            "tag_list",
            List,
            "List the tags of a GitHub repository",
            "Lists the tags of a repository (name and the commit each points to), newest first as GitHub orders them.",
            vec![REPO, LIMIT],
            Some("repo"),
        ),
        tool(
            "github_tag_get",
            GITHUB,
            "tag_get",
            Read,
            "Read a GitHub tag",
            "Reads one tag: the commit it points to and, for an annotated tag, its message and tagger.",
            vec![REPO, TAG],
            Some("repo"),
        ),
        tool(
            "github_tag_create",
            GITHUB,
            "tag_create",
            Write,
            "Create a GitHub tag",
            "Creates a tag on a commit. Without `message` it is a lightweight tag; with `message` it is an annotated tag. \
             `target` is a commit SHA, branch or tag; default: the head of the default branch. An existing tag is never \
             moved. The user approves on their phone.",
            vec![
                REPO,
                TAG,
                str_p("target", 250, false, "Commit SHA, branch or tag to tag. Default: the default branch."),
                text_p("message", 5_000, false, "Makes an annotated tag with this message."),
                str_p("tagger_name", 100, false, "For an annotated tag; give it together with tagger_email."),
                str_p("tagger_email", 254, false, "For an annotated tag; give it together with tagger_name."),
            ],
            Some("repo"),
        )
        .in_class("releases"),
        tool(
            "github_tag_delete",
            GITHUB,
            "tag_delete",
            Write,
            "Delete a GitHub tag",
            "Deletes a tag. A release made from it stays (delete it separately). The user approves on their phone.",
            vec![REPO, TAG],
            Some("repo"),
        )
        .in_class("releases"),
        // ---- releases ----
        tool(
            "github_release_list",
            GITHUB,
            "release_list",
            Read,
            "List GitHub releases",
            "Lists the releases of a repository, newest first (id, tag, name, draft and prerelease flags, asset count).",
            vec![REPO, LIMIT],
            Some("repo"),
        ),
        tool(
            "github_release_get",
            GITHUB,
            "release_get",
            Read,
            "Read a GitHub release",
            "Reads one release by id: name, tag, notes and its assets.",
            vec![REPO, RELEASE_ID],
            Some("repo"),
        ),
        tool(
            "github_release_latest",
            GITHUB,
            "release_latest",
            Read,
            "Read the latest GitHub release",
            "Reads the latest published release (not a draft or prerelease) of a repository.",
            vec![REPO],
            Some("repo"),
        ),
        tool(
            "github_release_by_tag",
            GITHUB,
            "release_by_tag",
            Read,
            "Read the GitHub release of a tag",
            "Reads the release made from a tag.",
            vec![REPO, TAG],
            Some("repo"),
        ),
        tool(
            "github_release_notes_generate",
            GITHUB,
            "release_notes_generate",
            Read,
            "Generate GitHub release notes",
            "Asks GitHub to write release notes (a name and a Markdown body listing the changes) for a tag between two \
             points. Nothing is created; use the text with github_release_create.",
            vec![
                REPO,
                str_p("tag_name", 250, true, "The tag the notes are for (need not exist yet)."),
                str_p("target_commitish", 250, false, "Where the tag would be created if it does not exist. Default: the default branch."),
                str_p("previous_tag_name", 250, false, "The tag to start from. Default: the previous release."),
            ],
            Some("repo"),
        ),
        tool(
            "github_release_create",
            GITHUB,
            "release_create",
            Write,
            "Create a GitHub release",
            "Creates a release for a tag. When the tag does not exist, GitHub creates it at `target_commitish` (default: \
             the head of the default branch). Set `generate_release_notes` to let GitHub write the body. Publishes \
             immediately unless `draft` is true. The user approves on their phone.",
            vec![
                REPO,
                str_p("tag_name", 250, true, "The tag of the release, e.g. v1.2.0."),
                str_p("target_commitish", 250, false, "Branch or commit SHA the tag is created at when it does not exist."),
                str_p("name", 300, false, "The title of the release."),
                text_p("body", 65_000, false, "The release notes (Markdown)."),
                bool_p("draft", "Create it as an unpublished draft."),
                bool_p("prerelease", "Mark it as a prerelease."),
                bool_p("generate_release_notes", "Let GitHub write the notes (added to `body` when both are given)."),
                choice_p("make_latest", &["true", "false", "legacy"], false, "Whether it becomes the latest release."),
            ],
            Some("repo"),
        )
        .in_class("releases"),
        tool(
            "github_release_update",
            GITHUB,
            "release_update",
            Write,
            "Change a GitHub release",
            "Changes the fields given of a release (tag, target, name, notes, draft, prerelease, latest); everything \
             else stays. At least one field is required. The user approves on their phone.",
            vec![
                REPO,
                RELEASE_ID,
                str_p("tag_name", 250, false, "A new tag for the release."),
                str_p("target_commitish", 250, false, "A new target for the tag, when it does not exist yet."),
                str_p("name", 300, false, "A new title."),
                text_p("body", 65_000, false, "New notes (Markdown), replacing the old."),
                bool_p("draft", "true keeps or makes it a draft; false publishes it."),
                bool_p("prerelease", "Whether it is a prerelease."),
                choice_p("make_latest", &["true", "false", "legacy"], false, "Whether it becomes the latest release."),
            ],
            Some("repo"),
        )
        .in_class("releases"),
        tool(
            "github_release_delete",
            GITHUB,
            "release_delete",
            Write,
            "Delete a GitHub release",
            "Deletes a release and its assets; the tag stays. The user approves on their phone.",
            vec![REPO, RELEASE_ID],
            Some("repo"),
        )
        .in_class("releases"),
        tool(
            "github_release_asset_list",
            GITHUB,
            "release_asset_list",
            List,
            "List the files attached to a GitHub release",
            "Lists the assets of a release (id, name, size, content type, download count).",
            vec![REPO, RELEASE_ID],
            Some("repo"),
        ),
        tool(
            "github_release_asset_upload",
            GITHUB,
            "release_asset_upload",
            Write,
            "Attach a file to a GitHub release",
            "Uploads a file as an asset of a release. Small files: `content_base64` (at most 2 MB decoded). Larger \
             files: call without content and the phone answers with an upload link; upload there and call again with \
             `blob`. An asset with the same name must not exist yet. The user approves on their phone.",
            vec![
                REPO,
                RELEASE_ID,
                str_p("name", 200, true, "The file name shown on the release, e.g. app-1.2.0.apk."),
                text_p("content_base64", 3_000_000, false, "The file as base64, at most 2 MB decoded."),
                str_p("blob", 64, false, "A file uploaded to the link the phone gave, instead of content_base64 (for files over 2 MB)."),
                str_p("label", 200, false, "A label shown instead of the name."),
                str_p("content_type", 100, false, "The media type, e.g. application/zip. Default: application/octet-stream."),
            ],
            Some("repo"),
        )
        .in_class("releases"),
        tool(
            "github_release_asset_update",
            GITHUB,
            "release_asset_update",
            Write,
            "Rename a GitHub release asset",
            "Changes the name and/or label of an asset; at least one is required. The user approves on their phone.",
            vec![
                REPO,
                ASSET_ID,
                str_p("name", 200, false, "The new file name."),
                str_p("label", 200, false, "The new label."),
            ],
            Some("repo"),
        )
        .in_class("releases"),
        tool(
            "github_release_asset_delete",
            GITHUB,
            "release_asset_delete",
            Write,
            "Delete a GitHub release asset",
            "Deletes one file attached to a release. The user approves on their phone.",
            vec![REPO, ASSET_ID],
            Some("repo"),
        )
        .in_class("releases"),
        tool(
            "github_release_asset_download",
            GITHUB,
            "release_asset_download",
            Read,
            "Download a GitHub release asset",
            "Downloads one asset of a release: text as text (at most 100000 characters), binary as base64 when at most \
             2 MB, otherwise only its metadata. The user chooses what to share.",
            vec![REPO, ASSET_ID],
            Some("repo"),
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_write_has_a_class_and_no_read_has() {
        for t in tools() {
            assert_eq!(t.effect == Effect::Write, !t.class.is_empty(), "{}", t.tool);
            assert!(!t.once_only, "{}", t.tool);
            assert!(t.tool.strip_prefix("github_") == Some(t.op), "{}", t.tool);
        }
    }

    #[test]
    fn code_changes_are_class_code_and_releases_are_class_releases() {
        for t in tools() {
            let expected = match t.op {
                "file_put" | "file_delete" | "commit_files" => "code",
                op if t.effect == Effect::Write => {
                    assert!(op.starts_with("tag_") || op.starts_with("release_"), "{op}");
                    "releases"
                }
                _ => "",
            };
            assert_eq!(t.class, expected, "{}", t.tool);
        }
    }

    #[test]
    fn the_file_list_and_contents_are_bounded() {
        let put = tools().into_iter().find(|t| t.op == "file_put").unwrap();
        let too_long =
            serde_json::json!({"repo": "a/b", "path": "x", "message": "m", "content_base64": "A".repeat(3_000_001)});
        assert!(put.parse(&too_long).is_err());
        let commit = tools().into_iter().find(|t| t.op == "commit_files").unwrap();
        assert!(commit.parse(&serde_json::json!({"repo": "a/b", "message": "m"})).unwrap_err().contains("files"));
        let ok = serde_json::json!({"repo": "a/b", "message": "m", "files": [{"path": "a", "content": "x"}]});
        assert!(commit.parse(&ok).is_ok());
    }
}
