//! Any GitHub REST call, for what no dedicated tool covers. The phone works out from the method and path which
//! repository (or organization, or the account) it touches and what kind of change it is, so the usual permissions
//! apply; far-reaching paths (deleting a repository, visibility, collaborators, webhooks, keys, branch protection) are
//! asked every time.

use crate::connector::{Effect, GITHUB, Param, ToolSpec, choice_p, json_p, map_p, str_p, tool};

const PATH: Param = str_p(
    "path",
    500,
    true,
    "The REST path, starting with `/`, e.g. /repos/owner/name/pulls/12/requested_reviewers. No host, no `..`.",
);
const QUERY: Param = map_p("query", 30, 500, "Query parameters, e.g. {\"per_page\": \"50\", \"state\": \"open\"}.");

pub(super) fn tools() -> Vec<ToolSpec> {
    vec![
        tool(
            "github_api_read",
            GITHUB,
            "api_read",
            Effect::Read,
            "Call the GitHub API (read)",
            "A GET request to any GitHub REST endpoint (https://docs.github.com/rest) not covered by another tool. \
             The answer is the JSON GitHub returns (large answers are cut, binary ones come as a download link). Reads \
             of one repository are covered by a read permission for it.",
            vec![PATH, QUERY],
            None,
        ),
        tool(
            "github_api_write",
            GITHUB,
            "api_write",
            Effect::Write,
            "Call the GitHub API (change)",
            "A POST, PUT, PATCH or DELETE to any GitHub REST endpoint not covered by another tool. The user sees the \
             method, the path and the body. Changes to repository settings, collaborators, webhooks, keys, branch \
             protection or deletions are asked every time.",
            vec![
                choice_p(
                    "method",
                    &["post", "put", "patch", "delete"],
                    true,
                    "The HTTP method: post, put, patch or delete.",
                ),
                PATH,
                QUERY,
                json_p("body", 512 * 1024, false, "The JSON body, when the endpoint takes one."),
            ],
            None,
        ),
    ]
}
