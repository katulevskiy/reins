//! GitHub tools: repositories, branches and repository settings and access.

use crate::connector::{Effect, GITHUB, LIMIT, Param, ToolSpec, bool_p, choice_p, int_p, list_p, str_p, tool};

const REPO: Param = str_p("repo", 140, true, "owner/name.");
const BRANCH: Param = str_p("branch", 250, true, "The branch name.");
const PERMISSIONS: &[&str] = &["pull", "triage", "push", "maintain", "admin"];

/// An integer argument that must be given.
const fn required(mut p: Param) -> Param {
    p.required = true;
    p
}

pub(super) fn tools() -> Vec<ToolSpec> {
    let mut all = repository_tools();
    all.extend(branch_tools());
    all.extend(access_tools());
    all.extend(integration_tools());
    all.extend(traffic_tools());
    all
}

#[allow(clippy::too_many_lines, reason = "one registry of tools")]
fn repository_tools() -> Vec<ToolSpec> {
    use Effect::{List, Read, Write};
    vec![
        tool(
            "github_list_repos",
            GITHUB,
            "list_repos",
            List,
            "List GitHub repositories",
            "Lists repositories (name, visibility, description), most recently pushed first. By default the ones the \
             user's GitHub token can reach; `org` lists an organization's, `owner` a user's (public ones).",
            vec![
                str_p("query", 100, false, "Only repositories whose name contains this text."),
                str_p("org", 100, false, "List the repositories of this organization instead."),
                str_p("owner", 100, false, "List the public repositories of this user instead."),
                choice_p("visibility", &["all", "public", "private"], false, "Only public or only private ones."),
                choice_p(
                    "affiliation",
                    &["owner", "collaborator", "organization_member"],
                    false,
                    "Only repositories the user owns, collaborates on, or reaches through an organization \
                     (own listing only).",
                ),
                LIMIT,
            ],
            None,
        ),
        tool(
            "github_org_repo_list",
            GITHUB,
            "org_repo_list",
            List,
            "List the repositories of a GitHub organization",
            "Lists the repositories of one organization that the token can see, most recently pushed first.",
            vec![
                str_p("org", 100, true, "The organization."),
                choice_p("type", &["all", "public", "private", "forks", "sources", "member"], false, "Default: all."),
                str_p("query", 100, false, "Only repositories whose name contains this text."),
                LIMIT,
            ],
            Some("org"),
        ),
        tool(
            "github_repo_get",
            GITHUB,
            "repo_get",
            Read,
            "Read a GitHub repository's details",
            "Reads one repository: description, topics, default branch, visibility, size, languages, counts, license, \
             whether it is a fork (and of what) and which features are on.",
            vec![REPO],
            Some("repo"),
        ),
        tool(
            "github_repo_create",
            GITHUB,
            "repo_create",
            Write,
            "Create a GitHub repository",
            "Creates a repository for the user, or in an organization with `org`. It is private unless `private` is \
             false. The user sees the name and settings and approves on their phone.",
            vec![
                str_p("name", 100, true, "The repository name."),
                str_p("org", 100, false, "Create it in this organization instead of the user's account."),
                str_p("description", 350, false, "A short description."),
                str_p("homepage", 255, false, "A URL."),
                bool_p("private", "Default: true (private). Give false to make it public."),
                bool_p("auto_init", "Create an initial commit with an empty README."),
                str_p("gitignore_template", 60, false, "A .gitignore template name such as Rust or Node."),
                str_p("license_template", 60, false, "A license keyword such as mit or apache-2.0."),
            ],
            Some("name"),
        )
        .in_class("settings"),
        tool(
            "github_repo_update",
            GITHUB,
            "repo_update",
            Write,
            "Change GitHub repository settings",
            "Changes the settings given (description, homepage, default branch, features, merge options, archiving); \
             everything else stays. Visibility is changed with github_repo_visibility_set and topics with \
             github_repo_topics_set. An archived repository cannot be unarchived through the API. The user sees the old \
             and new values and approves on their phone.",
            vec![
                REPO,
                str_p("description", 350, false, "New description (empty text clears it)."),
                str_p("homepage", 255, false, "New homepage URL (empty text clears it)."),
                str_p("default_branch", 250, false, "Make this existing branch the default branch."),
                bool_p("has_issues", "Turn issues on or off."),
                bool_p("has_projects", "Turn projects on or off."),
                bool_p("has_wiki", "Turn the wiki on or off."),
                bool_p("allow_merge_commit", "Allow merge commits on pull requests."),
                bool_p("allow_squash_merge", "Allow squash merging."),
                bool_p("allow_rebase_merge", "Allow rebase merging."),
                bool_p("allow_auto_merge", "Allow auto-merge on pull requests."),
                bool_p("delete_branch_on_merge", "Delete head branches after a pull request is merged."),
                bool_p("archived", "Give true to archive the repository (read-only). It cannot be undone via the API."),
            ],
            Some("repo"),
        )
        .in_class("settings"),
        tool(
            "github_repo_fork",
            GITHUB,
            "repo_fork",
            Write,
            "Fork a GitHub repository",
            "Forks a repository into the user's account, or into `org`. The fork is made in the background by GitHub. \
             The user approves on their phone.",
            vec![
                REPO,
                str_p("org", 100, false, "Fork into this organization instead of the user's account."),
                str_p("name", 100, false, "Name the fork differently."),
                bool_p("default_branch_only", "Copy only the default branch."),
            ],
            Some("repo"),
        )
        .in_class("settings"),
        tool(
            "github_repo_forks_list",
            GITHUB,
            "repo_forks_list",
            List,
            "List forks of a GitHub repository",
            "Lists the forks of a repository.",
            vec![
                REPO,
                choice_p("sort", &["newest", "oldest", "stargazers", "watchers"], false, "Default: newest."),
                LIMIT,
            ],
            Some("repo"),
        ),
        tool(
            "github_repo_delete",
            GITHUB,
            "repo_delete",
            Write,
            "Delete a GitHub repository",
            "PERMANENTLY deletes a repository with its issues, pull requests and wiki. The token needs the delete \
             permission. The user sees what is deleted and approves on their phone, every time (a standing permission \
             never covers it).",
            vec![REPO],
            Some("repo"),
        )
        .in_class("settings")
        .once(),
        tool(
            "github_repo_transfer",
            GITHUB,
            "repo_transfer",
            Write,
            "Transfer a GitHub repository",
            "Starts moving a repository to another user or organization (optionally renaming it). The new owner may \
             have to accept. The user approves on their phone, every time (a standing permission never covers it).",
            vec![
                REPO,
                str_p("new_owner", 100, true, "The user or organization that receives the repository."),
                str_p("new_name", 100, false, "A new name for the repository."),
            ],
            Some("repo"),
        )
        .in_class("settings")
        .once(),
        tool(
            "github_repo_visibility_set",
            GITHUB,
            "repo_visibility_set",
            Write,
            "Make a GitHub repository public or private",
            "Changes who can see a repository. Making it public exposes all its code and history. The user approves \
             on their phone, every time (a standing permission never covers it).",
            vec![REPO, choice_p("visibility", &["public", "private"], true, "The new visibility.")],
            Some("repo"),
        )
        .in_class("settings")
        .once(),
        tool(
            "github_repo_languages",
            GITHUB,
            "repo_languages",
            Read,
            "Read the languages of a GitHub repository",
            "Reads the languages used in a repository with their share (bytes and percent).",
            vec![REPO],
            Some("repo"),
        ),
        tool(
            "github_repo_contributors",
            GITHUB,
            "repo_contributors",
            Read,
            "List contributors of a GitHub repository",
            "Lists the people who committed to a repository with their number of commits, most first.",
            vec![REPO, LIMIT],
            Some("repo"),
        ),
        tool(
            "github_repo_topics_get",
            GITHUB,
            "repo_topics_get",
            Read,
            "Read the topics of a GitHub repository",
            "Reads the topics (tags) of a repository.",
            vec![REPO],
            Some("repo"),
        ),
        tool(
            "github_repo_topics_set",
            GITHUB,
            "repo_topics_set",
            Write,
            "Set the topics of a GitHub repository",
            "Replaces all topics of a repository with the given ones (lowercase letters, digits and hyphens, at most \
             20, 50 characters each; an empty list removes them all). The user approves on their phone.",
            vec![REPO, list_p("topics", 20, 50, "The complete new list of topics.")],
            Some("repo"),
        )
        .in_class("settings"),
        tool(
            "github_repo_readme_get",
            GITHUB,
            "repo_readme_get",
            Read,
            "Read the README of a GitHub repository",
            "Reads the README of a repository (its default branch) as text, at most 100000 characters.",
            vec![REPO],
            Some("repo"),
        ),
    ]
}

#[allow(clippy::too_many_lines, reason = "one registry of tools")]
fn branch_tools() -> Vec<ToolSpec> {
    use Effect::{List, Read, Write};
    vec![
        tool(
            "github_branch_list",
            GITHUB,
            "branch_list",
            List,
            "List branches of a GitHub repository",
            "Lists the branches of a repository (name, protected or not).",
            vec![REPO, bool_p("protected", "Only protected (true) or only unprotected (false) branches."), LIMIT],
            Some("repo"),
        ),
        tool(
            "github_branch_get",
            GITHUB,
            "branch_get",
            Read,
            "Read a GitHub branch",
            "Reads one branch: its head commit (sha, message, author, date) and whether it is protected.",
            vec![REPO, BRANCH],
            Some("repo"),
        ),
        tool(
            "github_branch_create",
            GITHUB,
            "branch_create",
            Write,
            "Create a GitHub branch",
            "Creates a new branch pointing at the head of another branch, a tag or a commit sha (default: the default \
             branch). The user approves on their phone.",
            vec![
                REPO,
                str_p("branch", 250, true, "The name of the new branch."),
                str_p("from", 250, false, "A branch, tag or full commit sha to start from. Default: the default branch."),
            ],
            Some("repo"),
        )
        .in_class("code"),
        tool(
            "github_branch_delete",
            GITHUB,
            "branch_delete",
            Write,
            "Delete a GitHub branch",
            "Deletes a branch (not the default branch, not a protected one). The commits stay in git for a while. The \
             user sees the branch's head and approves on their phone.",
            vec![REPO, BRANCH],
            Some("repo"),
        )
        .in_class("code"),
        tool(
            "github_branch_rename",
            GITHUB,
            "branch_rename",
            Write,
            "Rename a GitHub branch",
            "Renames a branch; open pull requests follow it. Renaming the default branch is asked for every time. The \
             user approves on their phone.",
            vec![REPO, BRANCH, str_p("new_name", 250, true, "The new branch name.")],
            Some("repo"),
        )
        .in_class("code"),
        tool(
            "github_branch_merge",
            GITHUB,
            "branch_merge",
            Write,
            "Merge one GitHub branch into another",
            "Merges `head` (a branch or commit sha) into the branch `base` with a merge commit, without a pull request. \
             The user sees the commits that would be merged and approves on their phone. Conflicts are reported, not \
             resolved.",
            vec![
                REPO,
                str_p("base", 250, true, "The branch that receives the changes."),
                str_p("head", 250, true, "The branch or commit sha to merge in."),
                str_p("commit_message", 1_000, false, "The merge commit message."),
            ],
            Some("repo"),
        )
        .in_class("code"),
        tool(
            "github_branch_protection_get",
            GITHUB,
            "branch_protection_get",
            Read,
            "Read the protection of a GitHub branch",
            "Reads the protection rules of a branch (required checks, reviews, force-push and deletion rules). Needs \
             admin access to the repository.",
            vec![REPO, BRANCH],
            Some("repo"),
        ),
        tool(
            "github_branch_protection_set",
            GITHUB,
            "branch_protection_set",
            Write,
            "Set the protection of a GitHub branch",
            "Replaces the protection rules of a branch: every rule not given here is turned off. `required_approvals` \
             (0 to 6) requires pull request reviews; `status_check_contexts` lists checks that must pass. \
             `restrict_push_users` and `restrict_push_teams` (organization repositories) limit who can push. The user \
             sees the old and new rules and approves on their phone, every time (a standing permission never covers \
             it).",
            vec![
                REPO,
                BRANCH,
                list_p("status_check_contexts", 50, 200, "Names of the checks that must pass before merging."),
                bool_p("strict_status_checks", "Branches must be up to date before merging (with status checks)."),
                int_p("required_approvals", 0, 6, None, "Approving reviews required; 0 or missing means no review rule."),
                bool_p("dismiss_stale_reviews", "New commits dismiss earlier approvals (with required_approvals)."),
                bool_p("require_code_owner_reviews", "A code owner must approve (with required_approvals)."),
                bool_p("enforce_admins", "The rules apply to administrators too."),
                bool_p("required_linear_history", "Forbid merge commits."),
                bool_p("allow_force_pushes", "Allow force pushes."),
                bool_p("allow_deletions", "Allow deleting the branch."),
                bool_p("required_conversation_resolution", "All review conversations must be resolved."),
                bool_p("lock_branch", "Make the branch read-only."),
                list_p("restrict_push_users", 50, 100, "Only these users may push (logins)."),
                list_p("restrict_push_teams", 50, 100, "Only these teams may push (team slugs)."),
            ],
            Some("repo"),
        )
        .in_class("settings")
        .once(),
        tool(
            "github_branch_protection_delete",
            GITHUB,
            "branch_protection_delete",
            Write,
            "Remove the protection of a GitHub branch",
            "Removes all protection rules of a branch. The user approves on their phone, every time (a standing \
             permission never covers it).",
            vec![REPO, BRANCH],
            Some("repo"),
        )
        .in_class("settings")
        .once(),
        tool(
            "github_ruleset_list",
            GITHUB,
            "ruleset_list",
            Read,
            "List the rulesets of a GitHub repository",
            "Lists the rulesets of a repository (name, target, enforcement).",
            vec![REPO, LIMIT],
            Some("repo"),
        ),
        tool(
            "github_ruleset_get",
            GITHUB,
            "ruleset_get",
            Read,
            "Read a GitHub ruleset",
            "Reads one ruleset: which branches or tags it targets and its rules.",
            vec![REPO, required(int_p("ruleset_id", 1, i64::MAX, None, "The ruleset id from github_ruleset_list."))],
            Some("repo"),
        ),
    ]
}

#[allow(clippy::too_many_lines, reason = "one registry of tools")]
fn access_tools() -> Vec<ToolSpec> {
    use Effect::{Read, Write};
    vec![
        tool(
            "github_collaborator_list",
            GITHUB,
            "collaborator_list",
            Read,
            "List collaborators of a GitHub repository",
            "Lists the people with access to a repository and their permission. Needs push access.",
            vec![REPO, choice_p("affiliation", &["all", "direct", "outside"], false, "Default: all."), LIMIT],
            Some("repo"),
        ),
        tool(
            "github_collaborator_add",
            GITHUB,
            "collaborator_add",
            Write,
            "Add a collaborator to a GitHub repository",
            "Invites a user to a repository with a permission (pull = read, triage, push = write, maintain, admin). \
             The user approves on their phone, every time (a standing permission never covers it).",
            vec![
                REPO,
                str_p("username", 100, true, "The GitHub login to invite."),
                choice_p("permission", PERMISSIONS, true, "The permission to give."),
            ],
            Some("repo"),
        )
        .in_class("settings")
        .once(),
        tool(
            "github_collaborator_remove",
            GITHUB,
            "collaborator_remove",
            Write,
            "Remove a collaborator from a GitHub repository",
            "Removes a user's direct access to a repository. The user approves on their phone, every time (a standing \
             permission never covers it).",
            vec![REPO, str_p("username", 100, true, "The GitHub login to remove.")],
            Some("repo"),
        )
        .in_class("settings")
        .once(),
        tool(
            "github_invitation_list",
            GITHUB,
            "invitation_list",
            Read,
            "List pending invitations of a GitHub repository",
            "Lists the collaborator invitations that have not been accepted yet.",
            vec![REPO, LIMIT],
            Some("repo"),
        ),
        tool(
            "github_invitation_cancel",
            GITHUB,
            "invitation_cancel",
            Write,
            "Cancel a GitHub repository invitation",
            "Cancels a pending collaborator invitation. The user approves on their phone, every time (a standing \
             permission never covers it).",
            vec![
                REPO,
                required(int_p("invitation_id", 1, i64::MAX, None, "The invitation id from github_invitation_list.")),
            ],
            Some("repo"),
        )
        .in_class("settings")
        .once(),
        tool(
            "github_repo_team_list",
            GITHUB,
            "repo_team_list",
            Read,
            "List the teams of a GitHub repository",
            "Lists the teams that have access to an organization's repository, with their permission.",
            vec![REPO, LIMIT],
            Some("repo"),
        ),
        tool(
            "github_repo_team_add",
            GITHUB,
            "repo_team_add",
            Write,
            "Give a team access to a GitHub repository",
            "Gives an organization team access to one of the organization's repositories (or changes its permission). \
             The user approves on their phone, every time (a standing permission never covers it).",
            vec![
                REPO,
                str_p("team", 100, true, "The team slug (from github_repo_team_list or the team's URL)."),
                choice_p("permission", PERMISSIONS, true, "The permission to give."),
            ],
            Some("repo"),
        )
        .in_class("settings")
        .once(),
        tool(
            "github_repo_team_remove",
            GITHUB,
            "repo_team_remove",
            Write,
            "Remove a team's access to a GitHub repository",
            "Removes an organization team from a repository. The user approves on their phone, every time (a standing \
             permission never covers it).",
            vec![REPO, str_p("team", 100, true, "The team slug.")],
            Some("repo"),
        )
        .in_class("settings")
        .once(),
    ]
}

#[allow(clippy::too_many_lines, reason = "one registry of tools")]
fn integration_tools() -> Vec<ToolSpec> {
    use Effect::{Read, Write};
    const HOOK: Param = required(int_p("hook_id", 1, i64::MAX, None, "The webhook id from github_webhook_list."));
    vec![
        tool(
            "github_webhook_list",
            GITHUB,
            "webhook_list",
            Read,
            "List webhooks of a GitHub repository",
            "Lists the webhooks of a repository (id, events, active, where they deliver to; the address is shortened \
             because it may contain a secret). Needs admin access.",
            vec![REPO, LIMIT],
            Some("repo"),
        ),
        tool(
            "github_webhook_get",
            GITHUB,
            "webhook_get",
            Read,
            "Read a GitHub webhook",
            "Reads one webhook: events, active, content type, last delivery status. The address is shortened and the \
             secret never shown.",
            vec![REPO, HOOK],
            Some("repo"),
        ),
        tool(
            "github_webhook_create",
            GITHUB,
            "webhook_create",
            Write,
            "Create a GitHub webhook",
            "Adds a webhook that sends the chosen events of a repository to an https address. The user sees the \
             address and events and approves on their phone, every time (a standing permission never covers it).",
            vec![
                REPO,
                str_p("url", 500, true, "Where deliveries go (https)."),
                list_p("events", 30, 50, "Event names such as push or pull_request. Default: push."),
                choice_p("content_type", &["json", "form"], false, "Default: json."),
                str_p("secret", 200, false, "A secret to sign deliveries with. It is never shown again."),
                bool_p("insecure_ssl", "Skip checking the address's certificate. Default: false."),
                bool_p("active", "Whether deliveries are sent. Default: true."),
            ],
            Some("repo"),
        )
        .in_class("settings")
        .once(),
        tool(
            "github_webhook_update",
            GITHUB,
            "webhook_update",
            Write,
            "Change a GitHub webhook",
            "Changes the fields given of a webhook (address, events, content type, secret, active). The user approves \
             on their phone, every time (a standing permission never covers it).",
            vec![
                REPO,
                HOOK,
                str_p("url", 500, false, "New address (https)."),
                list_p("events", 30, 50, "The complete new list of events."),
                choice_p("content_type", &["json", "form"], false, "New content type."),
                str_p("secret", 200, false, "New signing secret. It is never shown."),
                bool_p("insecure_ssl", "Skip checking the address's certificate."),
                bool_p("active", "Turn deliveries on or off."),
            ],
            Some("repo"),
        )
        .in_class("settings")
        .once(),
        tool(
            "github_webhook_delete",
            GITHUB,
            "webhook_delete",
            Write,
            "Delete a GitHub webhook",
            "Deletes a webhook. The user approves on their phone, every time (a standing permission never covers it).",
            vec![REPO, HOOK],
            Some("repo"),
        )
        .in_class("settings")
        .once(),
        tool(
            "github_webhook_ping",
            GITHUB,
            "webhook_ping",
            Write,
            "Ping a GitHub webhook",
            "Makes GitHub send a test delivery to a webhook's address. The user approves on their phone, every time (a \
             standing permission never covers it).",
            vec![REPO, HOOK],
            Some("repo"),
        )
        .in_class("settings")
        .once(),
        tool(
            "github_deploy_key_list",
            GITHUB,
            "deploy_key_list",
            Read,
            "List deploy keys of a GitHub repository",
            "Lists the deploy keys (public SSH keys) of a repository.",
            vec![REPO, LIMIT],
            Some("repo"),
        ),
        tool(
            "github_deploy_key_add",
            GITHUB,
            "deploy_key_add",
            Write,
            "Add a deploy key to a GitHub repository",
            "Adds a public SSH key that can read (or, with read_only false, also write to) a repository. The user \
             approves on their phone, every time (a standing permission never covers it).",
            vec![
                REPO,
                str_p("title", 100, true, "A name for the key."),
                str_p("key", 2_000, true, "The public key, e.g. `ssh-ed25519 AAAA...`."),
                bool_p("read_only", "Default: true. Give false to allow pushing."),
            ],
            Some("repo"),
        )
        .in_class("settings")
        .once(),
        tool(
            "github_deploy_key_delete",
            GITHUB,
            "deploy_key_delete",
            Write,
            "Delete a deploy key of a GitHub repository",
            "Removes a deploy key. The user approves on their phone, every time (a standing permission never covers \
             it).",
            vec![REPO, required(int_p("key_id", 1, i64::MAX, None, "The key id from github_deploy_key_list."))],
            Some("repo"),
        )
        .in_class("settings")
        .once(),
    ]
}

fn traffic_tools() -> Vec<ToolSpec> {
    use Effect::Read;
    vec![
        tool(
            "github_traffic_clones",
            GITHUB,
            "traffic_clones",
            Read,
            "Read the clone traffic of a GitHub repository",
            "Reads how often a repository was cloned in the last 14 days (total and unique, per day or week). Needs \
             push access.",
            vec![REPO, choice_p("per", &["day", "week"], false, "Default: day.")],
            Some("repo"),
        ),
        tool(
            "github_traffic_views",
            GITHUB,
            "traffic_views",
            Read,
            "Read the page views of a GitHub repository",
            "Reads how often a repository's page was viewed in the last 14 days (total and unique, per day or week). \
             Needs push access.",
            vec![REPO, choice_p("per", &["day", "week"], false, "Default: day.")],
            Some("repo"),
        ),
        tool(
            "github_commit_activity",
            GITHUB,
            "commit_activity",
            Read,
            "Read the commit activity of a GitHub repository",
            "Reads the number of commits per week over the last year. GitHub computes this in the background: if it \
             says so, ask again in a moment.",
            vec![REPO],
            Some("repo"),
        ),
    ]
}
