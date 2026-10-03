//! GitHub tools: workflows and their runs, variables and secrets, security alerts, and the user's own account.

use crate::connector::{
    Effect, GITHUB, LIMIT, Param, ToolSpec, bool_p, choice_p, int_p, json_p, list_p, map_p, str_p, text_p, tool,
};

const REPO: Param = str_p("repo", 140, true, "owner/name.");
const WORKFLOW: Param =
    str_p("workflow", 100, true, "The workflow: its numeric id or its file name (ci.yml), from github_workflow_list.");
const ENVIRONMENT: Param =
    str_p("environment", 255, false, "An environment name (github_environment_list). Omit for the repository's own.");

const fn required(p: Param) -> Param {
    Param {
        required: true,
        ..p
    }
}

const fn id(name: &'static str, description: &'static str) -> Param {
    required(int_p(name, 1, i64::MAX, None, description))
}

const RUN: Param = id("run_id", "The workflow run id, from github_run_list.");
const JOB: Param = id("job_id", "The job id, from github_run_jobs.");
const ARTIFACT: Param = id("artifact_id", "The artifact id, from github_artifact_list.");
const DEBUG: Param = bool_p("enable_debug_logging", "Run again with debug logging on.");
const TAIL: Param = int_p("tail_lines", 1, 5_000, None, "Only the last this many lines of the log.");
const COMMENT: Param = str_p("comment", 1_000, false, "A short note recorded with the change.");

const RUN_STATUS: &[&str] = &[
    "queued",
    "in_progress",
    "completed",
    "waiting",
    "requested",
    "pending",
    "success",
    "failure",
    "cancelled",
    "skipped",
    "timed_out",
    "neutral",
    "action_required",
    "stale",
    "startup_failure",
];
const SEVERITY: &[&str] = &["critical", "high", "medium", "low"];

pub(super) fn tools() -> Vec<ToolSpec> {
    let mut all = Vec::new();
    all.extend(workflows());
    all.extend(runs());
    all.extend(jobs_and_artifacts());
    all.extend(variables_and_secrets());
    all.extend(environments_and_caches());
    all.extend(security());
    all.extend(account());
    all.extend(notifications_and_gists());
    all.extend(request());
    all
}

fn workflows() -> Vec<ToolSpec> {
    use Effect::{List, Read, Write};
    vec![
        tool(
            "github_workflow_list",
            GITHUB,
            "workflow_list",
            List,
            "List GitHub workflows",
            "Lists the workflows (GitHub Actions) of a repository: id, name, file path and whether each is active. \
             The workflow files themselves are edited with github_file_put.",
            vec![REPO, LIMIT],
            Some("repo"),
        ),
        tool(
            "github_workflow_get",
            GITHUB,
            "workflow_get",
            Read,
            "Read a GitHub workflow",
            "Reads the details of one workflow: name, file path, state and links.",
            vec![REPO, WORKFLOW],
            Some("repo"),
        ),
        tool(
            "github_workflow_usage",
            GITHUB,
            "workflow_usage",
            Read,
            "Read the usage of a GitHub workflow",
            "Reads the billable running time of one workflow per operating system (GitHub may not report it for every \
             repository).",
            vec![REPO, WORKFLOW],
            Some("repo"),
        ),
        tool(
            "github_workflow_dispatch",
            GITHUB,
            "workflow_dispatch",
            Write,
            "Run a GitHub workflow",
            "Starts a workflow that has a workflow_dispatch trigger, on a branch or tag, with optional inputs. This \
             runs code in the repository's CI. The user approves on their phone.",
            vec![
                REPO,
                WORKFLOW,
                str_p("ref", 250, true, "The branch or tag to run the workflow on."),
                map_p("inputs", 25, 1_024, "The workflow's inputs, name to value (all text)."),
            ],
            Some("repo"),
        )
        .in_class("actions"),
        tool(
            "github_workflow_enable",
            GITHUB,
            "workflow_enable",
            Write,
            "Enable a GitHub workflow",
            "Turns a disabled workflow back on. The user approves on their phone.",
            vec![REPO, WORKFLOW],
            Some("repo"),
        )
        .in_class("actions"),
        tool(
            "github_workflow_disable",
            GITHUB,
            "workflow_disable",
            Write,
            "Disable a GitHub workflow",
            "Turns a workflow off so it no longer runs on its triggers. The user approves on their phone.",
            vec![REPO, WORKFLOW],
            Some("repo"),
        )
        .in_class("actions"),
    ]
}

fn runs() -> Vec<ToolSpec> {
    use Effect::{Read, Write};
    vec![
        tool(
            "github_run_list",
            GITHUB,
            "run_list",
            Read,
            "List GitHub workflow runs",
            "Lists workflow runs of a repository, newest first, optionally of one workflow and filtered.",
            vec![
                REPO,
                str_p("workflow", 100, false, "Only runs of this workflow (id or file name)."),
                str_p("branch", 250, false, "Only runs on this branch."),
                str_p("event", 50, false, "Only runs started by this event (push, pull_request, schedule, ...)."),
                choice_p("status", RUN_STATUS, false, "Only runs with this status or conclusion."),
                str_p("actor", 100, false, "Only runs started by this user."),
                LIMIT,
            ],
            Some("repo"),
        ),
        tool(
            "github_run_get",
            GITHUB,
            "run_get",
            Read,
            "Read a GitHub workflow run",
            "Reads one workflow run: status, conclusion, commit, branch, actor, timing and attempt.",
            vec![REPO, RUN, int_p("attempt", 1, 1_000, None, "A specific attempt; default the latest.")],
            Some("repo"),
        ),
        tool(
            "github_run_attempts",
            GITHUB,
            "run_attempts",
            Read,
            "List the attempts of a GitHub workflow run",
            "Lists every attempt of a workflow run (the first run and each re-run), newest first, up to 20.",
            vec![REPO, RUN],
            Some("repo"),
        ),
        tool(
            "github_run_rerun",
            GITHUB,
            "run_rerun",
            Write,
            "Re-run a GitHub workflow run",
            "Runs a whole workflow run again. The user approves on their phone.",
            vec![REPO, RUN, DEBUG],
            Some("repo"),
        )
        .in_class("actions"),
        tool(
            "github_run_rerun_failed",
            GITHUB,
            "run_rerun_failed",
            Write,
            "Re-run the failed jobs of a GitHub run",
            "Runs again only the failed jobs of a workflow run (and the jobs that depend on them). The user approves on \
             their phone.",
            vec![REPO, RUN, DEBUG],
            Some("repo"),
        )
        .in_class("actions"),
        tool(
            "github_job_rerun",
            GITHUB,
            "job_rerun",
            Write,
            "Re-run one GitHub workflow job",
            "Runs a single job of a workflow run again (and the jobs that depend on it). The user approves on their phone.",
            vec![REPO, JOB, DEBUG],
            Some("repo"),
        )
        .in_class("actions"),
        tool(
            "github_run_cancel",
            GITHUB,
            "run_cancel",
            Write,
            "Cancel a GitHub workflow run",
            "Asks GitHub to cancel a workflow run that is running or queued. The user approves on their phone.",
            vec![REPO, RUN],
            Some("repo"),
        )
        .in_class("actions"),
        tool(
            "github_run_force_cancel",
            GITHUB,
            "run_force_cancel",
            Write,
            "Force-cancel a GitHub workflow run",
            "Cancels a workflow run immediately, skipping the always() and cleanup steps; use only when a normal cancel \
             does not work. The user is asked every time.",
            vec![REPO, RUN],
            Some("repo"),
        )
        .in_class("actions")
        .once(),
        tool(
            "github_run_delete",
            GITHUB,
            "run_delete",
            Write,
            "Delete a GitHub workflow run",
            "Deletes a finished workflow run with its logs. The user approves on their phone.",
            vec![REPO, RUN],
            Some("repo"),
        )
        .in_class("actions"),
        tool(
            "github_run_approve",
            GITHUB,
            "run_approve",
            Write,
            "Approve a GitHub run from a fork",
            "Approves a workflow run waiting on a pull request from a fork, which lets that code run in the \
             repository's CI. The user is asked every time.",
            vec![REPO, RUN],
            Some("repo"),
        )
        .in_class("actions")
        .once(),
        tool(
            "github_run_pending_deployments",
            GITHUB,
            "run_pending_deployments",
            Read,
            "List the deployments a GitHub run waits for",
            "Lists the environments a workflow run is waiting to deploy to, and whether the user may review them.",
            vec![REPO, RUN],
            Some("repo"),
        ),
        tool(
            "github_deployment_review",
            GITHUB,
            "deployment_review",
            Write,
            "Approve or reject a GitHub deployment",
            "Approves or rejects the deployment of a workflow run to one or more environments that require review. \
             The user is asked every time.",
            vec![
                REPO,
                RUN,
                Param {
                    required: true,
                    ..list_p(
                        "environments",
                        20,
                        255,
                        "Names of the environments to review, from github_run_pending_deployments.",
                    )
                },
                choice_p("state", &["approved", "rejected"], true, "Approve or reject."),
                Param {
                    required: true,
                    ..str_p("comment", 1_000, false, "Why.")
                },
            ],
            Some("repo"),
        )
        .in_class("actions")
        .once(),
    ]
}

fn jobs_and_artifacts() -> Vec<ToolSpec> {
    use Effect::{List, Read, Write};
    vec![
        tool(
            "github_run_jobs",
            GITHUB,
            "run_jobs",
            Read,
            "List the jobs of a GitHub workflow run",
            "Lists the jobs of a workflow run with their status, conclusion and steps (which step failed).",
            vec![REPO, RUN, choice_p("filter", &["latest", "all"], false, "latest (default) or all attempts."), LIMIT],
            Some("repo"),
        ),
        tool(
            "github_job_get",
            GITHUB,
            "job_get",
            Read,
            "Read a GitHub workflow job",
            "Reads one job: status, conclusion, runner and its steps.",
            vec![REPO, JOB],
            Some("repo"),
        ),
        tool(
            "github_job_logs",
            GITHUB,
            "job_logs",
            Read,
            "Read the log of a GitHub job",
            "Reads the log text of one job, biased to the end where failures show (at most 60000 characters; \
             `truncated` says when the start was cut). Nothing is fetched with the token except the redirect.",
            vec![REPO, JOB, TAIL],
            Some("repo"),
        ),
        tool(
            "github_run_logs",
            GITHUB,
            "run_logs",
            Read,
            "Read the logs of a GitHub workflow run",
            "Reads the end of the log of every job of a workflow run (no zip; about 60000 characters in all, shared \
             among the jobs, failed jobs first).",
            vec![REPO, RUN, TAIL],
            Some("repo"),
        ),
        tool(
            "github_artifact_list",
            GITHUB,
            "artifact_list",
            List,
            "List GitHub Actions artifacts",
            "Lists the build artifacts of a repository, or of one workflow run: name, size and expiry.",
            vec![
                REPO,
                int_p("run_id", 1, i64::MAX, None, "Only the artifacts of this workflow run."),
                str_p("name", 200, false, "Only artifacts with this exact name."),
                LIMIT,
            ],
            Some("repo"),
        ),
        tool(
            "github_artifact_download_url",
            GITHUB,
            "artifact_download_url",
            Read,
            "Get a download link for a GitHub artifact",
            "Returns a temporary link (valid for about a minute) to download an artifact zip; the file itself is not \
             fetched. The user approves the link, which is kept out of the activity log.",
            vec![REPO, ARTIFACT],
            Some("repo"),
        ),
        tool(
            "github_artifact_delete",
            GITHUB,
            "artifact_delete",
            Write,
            "Delete a GitHub artifact",
            "Deletes one build artifact. The user approves on their phone.",
            vec![REPO, ARTIFACT],
            Some("repo"),
        )
        .in_class("actions"),
    ]
}

fn variables_and_secrets() -> Vec<ToolSpec> {
    use Effect::{List, Read, Write};
    const NAME: Param =
        str_p("name", 100, true, "The name: letters, digits and underscores, not starting with GITHUB_.");
    vec![
        tool(
            "github_variable_list",
            GITHUB,
            "variable_list",
            Read,
            "List GitHub Actions variables",
            "Lists the Actions variables (name and value) of a repository or one of its environments.",
            vec![REPO, ENVIRONMENT, LIMIT],
            Some("repo"),
        ),
        tool(
            "github_variable_get",
            GITHUB,
            "variable_get",
            Read,
            "Read a GitHub Actions variable",
            "Reads one Actions variable of a repository or environment.",
            vec![REPO, ENVIRONMENT, NAME],
            Some("repo"),
        ),
        tool(
            "github_variable_create",
            GITHUB,
            "variable_create",
            Write,
            "Create a GitHub Actions variable",
            "Creates an Actions variable in a repository or environment. Variables are not secret: use \
             github_secret_set for secrets. The user approves on their phone.",
            vec![
                REPO,
                ENVIRONMENT,
                NAME,
                text_p("value", 48_000, true, "The value (plain text, visible to anyone who can read the repository)."),
            ],
            Some("repo"),
        )
        .in_class("actions"),
        tool(
            "github_variable_update",
            GITHUB,
            "variable_update",
            Write,
            "Change a GitHub Actions variable",
            "Sets a new value for an existing Actions variable. The user sees the old and the new value and approves on \
             their phone.",
            vec![REPO, ENVIRONMENT, NAME, text_p("value", 48_000, true, "The new value.")],
            Some("repo"),
        )
        .in_class("actions"),
        tool(
            "github_variable_delete",
            GITHUB,
            "variable_delete",
            Write,
            "Delete a GitHub Actions variable",
            "Deletes an Actions variable. The user approves on their phone.",
            vec![REPO, ENVIRONMENT, NAME],
            Some("repo"),
        )
        .in_class("actions"),
        tool(
            "github_secret_list",
            GITHUB,
            "secret_list",
            List,
            "List GitHub Actions secrets",
            "Lists the names (never the values) of the Actions secrets of a repository or one of its environments.",
            vec![REPO, ENVIRONMENT, LIMIT],
            Some("repo"),
        ),
        tool(
            "github_secret_set",
            GITHUB,
            "secret_set",
            Write,
            "Set a GitHub Actions secret",
            "Creates or replaces an Actions secret in a repository or environment. The value is encrypted on the phone \
             with the repository's public key before it is sent, and is never shown, logged or returned. The user is \
             asked every time.",
            vec![
                REPO,
                ENVIRONMENT,
                NAME,
                text_p("value", 48_000, true, "The secret value."),
            ],
            Some("repo"),
        )
        .in_class("settings")
        .once(),
        tool(
            "github_secret_delete",
            GITHUB,
            "secret_delete",
            Write,
            "Delete a GitHub Actions secret",
            "Deletes an Actions secret from a repository or environment. The user is asked every time.",
            vec![REPO, ENVIRONMENT, NAME],
            Some("repo"),
        )
        .in_class("settings")
        .once(),
    ]
}

fn environments_and_caches() -> Vec<ToolSpec> {
    use Effect::{List, Read, Write};
    const ENV: Param = str_p("environment", 255, true, "The environment name.");
    vec![
        tool(
            "github_environment_list",
            GITHUB,
            "environment_list",
            List,
            "List GitHub environments",
            "Lists the deployment environments of a repository.",
            vec![REPO, LIMIT],
            Some("repo"),
        ),
        tool(
            "github_environment_get",
            GITHUB,
            "environment_get",
            Read,
            "Read a GitHub environment",
            "Reads an environment's protection rules: wait timer, required reviewers and branch policy.",
            vec![REPO, ENV],
            Some("repo"),
        ),
        tool(
            "github_environment_set",
            GITHUB,
            "environment_set",
            Write,
            "Create or change a GitHub environment",
            "Creates an environment or changes the protection rules of an existing one; only the fields given change. \
             The user is asked every time.",
            vec![
                REPO,
                ENV,
                int_p("wait_timer", 0, 43_200, None, "Minutes to wait before a deployment proceeds."),
                bool_p("prevent_self_review", "Whether the person who started a deployment may approve it."),
                list_p("reviewers", 6, 100, "GitHub logins of the required reviewers (replaces the current ones)."),
                choice_p(
                    "branch_policy",
                    &["any", "protected", "custom"],
                    false,
                    "Which branches may deploy: any, protected branches only, or custom branch policies.",
                ),
            ],
            Some("repo"),
        )
        .in_class("settings")
        .once(),
        tool(
            "github_environment_delete",
            GITHUB,
            "environment_delete",
            Write,
            "Delete a GitHub environment",
            "Deletes an environment with its secrets and variables. The user is asked every time.",
            vec![REPO, ENV],
            Some("repo"),
        )
        .in_class("settings")
        .once(),
        tool(
            "github_cache_list",
            GITHUB,
            "cache_list",
            List,
            "List GitHub Actions caches",
            "Lists the Actions caches of a repository: key, branch, size and last use.",
            vec![
                REPO,
                str_p("key", 200, false, "Only caches whose key starts with this."),
                str_p("ref", 250, false, "Only caches of this ref (refs/heads/main)."),
                LIMIT,
            ],
            Some("repo"),
        ),
        tool(
            "github_cache_delete",
            GITHUB,
            "cache_delete",
            Write,
            "Delete GitHub Actions caches",
            "Deletes one cache by id, or every cache with an exact key (optionally of one ref). Give the id or the key. \
             The user approves on their phone.",
            vec![
                REPO,
                int_p("cache_id", 1, i64::MAX, None, "The id of one cache, from github_cache_list."),
                str_p("key", 200, false, "Delete every cache with exactly this key."),
                str_p("ref", 250, false, "With key: only the caches of this ref."),
            ],
            Some("repo"),
        )
        .in_class("actions"),
    ]
}

fn security() -> Vec<ToolSpec> {
    use Effect::{List, Read, Write};
    const NUMBER: Param = id("number", "The alert number.");
    vec![
        tool(
            "github_dependabot_alert_list",
            GITHUB,
            "dependabot_alert_list",
            List,
            "List Dependabot alerts",
            "Lists the Dependabot (vulnerable dependency) alerts of a repository.",
            vec![
                REPO,
                choice_p("state", &["open", "dismissed", "fixed", "auto_dismissed"], false, "Default: open."),
                choice_p("severity", SEVERITY, false, "Only this severity."),
                str_p("package", 200, false, "Only this package."),
                LIMIT,
            ],
            Some("repo"),
        ),
        tool(
            "github_dependabot_alert_get",
            GITHUB,
            "dependabot_alert_get",
            Read,
            "Read a Dependabot alert",
            "Reads one Dependabot alert: the advisory, the affected package and version, and the fix.",
            vec![REPO, NUMBER],
            Some("repo"),
        ),
        tool(
            "github_dependabot_alert_update",
            GITHUB,
            "dependabot_alert_update",
            Write,
            "Dismiss or reopen a Dependabot alert",
            "Dismisses a Dependabot alert with a reason, or reopens it. The user approves on their phone.",
            vec![
                REPO,
                NUMBER,
                Param {
                    required: true,
                    ..choice_p("state", &["open", "dismissed"], false, "The new state.")
                },
                choice_p(
                    "reason",
                    &["fix_started", "inaccurate", "no_bandwidth", "not_used", "tolerable_risk"],
                    false,
                    "Why it is dismissed (required to dismiss).",
                ),
                COMMENT,
            ],
            Some("repo"),
        )
        .in_class("settings"),
        tool(
            "github_code_scanning_alert_list",
            GITHUB,
            "code_scanning_alert_list",
            List,
            "List code scanning alerts",
            "Lists the code scanning alerts of a repository.",
            vec![
                REPO,
                choice_p("state", &["open", "closed", "dismissed", "fixed"], false, "Default: open."),
                choice_p("severity", &["critical", "high", "medium", "low", "warning", "error", "note"], false, "Only this severity."),
                str_p("ref", 250, false, "Only alerts on this ref (refs/heads/main)."),
                LIMIT,
            ],
            Some("repo"),
        ),
        tool(
            "github_code_scanning_alert_get",
            GITHUB,
            "code_scanning_alert_get",
            Read,
            "Read a code scanning alert",
            "Reads one code scanning alert: the rule, the location and the message.",
            vec![REPO, NUMBER],
            Some("repo"),
        ),
        tool(
            "github_code_scanning_alert_update",
            GITHUB,
            "code_scanning_alert_update",
            Write,
            "Dismiss or reopen a code scanning alert",
            "Dismisses a code scanning alert with a reason, or reopens it. The user approves on their phone.",
            vec![
                REPO,
                NUMBER,
                Param {
                    required: true,
                    ..choice_p("state", &["open", "dismissed"], false, "The new state.")
                },
                choice_p(
                    "reason",
                    &["false positive", "won't fix", "used in tests"],
                    false,
                    "Why it is dismissed (required to dismiss).",
                ),
                COMMENT,
            ],
            Some("repo"),
        )
        .in_class("settings"),
        tool(
            "github_secret_scanning_alert_list",
            GITHUB,
            "secret_scanning_alert_list",
            Read,
            "List secret scanning alerts",
            "Lists the secret scanning alerts of a repository. The alerts contain the leaked secret itself, so the user \
             must tick each one on their phone; it is handed over but kept out of the activity log.",
            vec![
                REPO,
                choice_p("state", &["open", "resolved"], false, "Default: open."),
                str_p("secret_type", 200, false, "Only this secret type (github_personal_access_token, ...)."),
                LIMIT,
            ],
            Some("repo"),
        ),
        tool(
            "github_secret_scanning_alert_get",
            GITHUB,
            "secret_scanning_alert_get",
            Read,
            "Read a secret scanning alert",
            "Reads one secret scanning alert with the leaked secret and where it was found. The user must tick it on \
             their phone; the secret is kept out of the activity log.",
            vec![REPO, NUMBER],
            Some("repo"),
        ),
        tool(
            "github_secret_scanning_alert_update",
            GITHUB,
            "secret_scanning_alert_update",
            Write,
            "Resolve or reopen a secret scanning alert",
            "Marks a secret scanning alert resolved with a resolution, or reopens it. The user approves on their phone.",
            vec![
                REPO,
                NUMBER,
                Param {
                    required: true,
                    ..choice_p("state", &["open", "resolved"], false, "The new state.")
                },
                choice_p(
                    "resolution",
                    &["false_positive", "wont_fix", "revoked", "used_in_tests"],
                    false,
                    "Why it is resolved (required to resolve).",
                ),
                COMMENT,
            ],
            Some("repo"),
        )
        .in_class("settings"),
        tool(
            "github_security_advisory_list",
            GITHUB,
            "security_advisory_list",
            Read,
            "List repository security advisories",
            "Lists the security advisories published or drafted for a repository.",
            vec![
                REPO,
                choice_p("state", &["triage", "draft", "published", "closed"], false, "Only this state."),
                LIMIT,
            ],
            Some("repo"),
        ),
        tool(
            "github_dependency_sbom",
            GITHUB,
            "dependency_sbom",
            Read,
            "Read the dependencies of a GitHub repository",
            "Reads the software bill of materials (dependency graph) of a repository as a list of packages, cut at \
             60000 characters.",
            vec![REPO],
            Some("repo"),
        ),
    ]
}

fn account() -> Vec<ToolSpec> {
    use Effect::{List, Read, Write};
    const USER: Param = str_p("username", 100, true, "A GitHub login.");
    const ORG: Param = str_p("org", 100, true, "An organisation login.");
    vec![
        tool(
            "github_user_me",
            GITHUB,
            "user_me",
            Read,
            "Read your GitHub account",
            "Reads the profile of the GitHub account the user connected: login, name, plan and counts.",
            vec![],
            None,
        ),
        tool(
            "github_user_get",
            GITHUB,
            "user_get",
            Read,
            "Read a GitHub user",
            "Reads the public profile of a GitHub user or organisation.",
            vec![USER],
            None,
        ),
        tool(
            "github_org_list",
            GITHUB,
            "org_list",
            List,
            "List your GitHub organisations",
            "Lists the organisations the user belongs to.",
            vec![LIMIT],
            None,
        ),
        tool(
            "github_org_get",
            GITHUB,
            "org_get",
            Read,
            "Read a GitHub organisation",
            "Reads an organisation's public profile.",
            vec![ORG],
            None,
        ),
        tool(
            "github_org_repos",
            GITHUB,
            "org_repos",
            List,
            "List the repositories of a GitHub organisation",
            "Lists the repositories of an organisation that the token can see.",
            vec![ORG, LIMIT],
            None,
        ),
        tool(
            "github_org_members",
            GITHUB,
            "org_members",
            List,
            "List the members of a GitHub organisation",
            "Lists the logins of the members of an organisation that the token can see.",
            vec![ORG, LIMIT],
            None,
        ),
        tool(
            "github_org_teams",
            GITHUB,
            "org_teams",
            List,
            "List the teams of a GitHub organisation",
            "Lists the teams of an organisation that the token can see.",
            vec![ORG, LIMIT],
            None,
        ),
        tool(
            "github_starred_list",
            GITHUB,
            "starred_list",
            List,
            "List starred GitHub repositories",
            "Lists the repositories the user (or another user) has starred.",
            vec![str_p("username", 100, false, "Another user; default: the user."), LIMIT],
            None,
        ),
        tool(
            "github_star_add",
            GITHUB,
            "star_add",
            Write,
            "Star a GitHub repository",
            "Stars a repository for the user. The user approves on their phone.",
            vec![REPO],
            None,
        )
        .in_class("account"),
        tool(
            "github_star_remove",
            GITHUB,
            "star_remove",
            Write,
            "Unstar a GitHub repository",
            "Removes the user's star from a repository. The user approves on their phone.",
            vec![REPO],
            None,
        )
        .in_class("account"),
        tool(
            "github_watch_add",
            GITHUB,
            "watch_add",
            Write,
            "Watch a GitHub repository",
            "Subscribes the user to the notifications of a repository (or ignores it). The user approves on their phone.",
            vec![REPO, bool_p("ignored", "Ignore all notifications of the repository instead of watching it.")],
            None,
        )
        .in_class("account"),
        tool(
            "github_watch_remove",
            GITHUB,
            "watch_remove",
            Write,
            "Stop watching a GitHub repository",
            "Removes the user's subscription to a repository. The user approves on their phone.",
            vec![REPO],
            None,
        )
        .in_class("account"),
        tool(
            "github_follow_add",
            GITHUB,
            "follow_add",
            Write,
            "Follow a GitHub user",
            "Makes the user follow another GitHub user. The user approves on their phone.",
            vec![USER],
            None,
        )
        .in_class("account"),
        tool(
            "github_follow_remove",
            GITHUB,
            "follow_remove",
            Write,
            "Unfollow a GitHub user",
            "Makes the user stop following a GitHub user. The user approves on their phone.",
            vec![USER],
            None,
        )
        .in_class("account"),
        tool(
            "github_ssh_key_list",
            GITHUB,
            "ssh_key_list",
            List,
            "List your GitHub SSH keys",
            "Lists the titles and ids of the SSH keys on the user's account (never key material).",
            vec![LIMIT],
            None,
        ),
        tool(
            "github_gpg_key_list",
            GITHUB,
            "gpg_key_list",
            List,
            "List your GitHub GPG keys",
            "Lists the key ids and email addresses of the GPG keys on the user's account (never key material).",
            vec![LIMIT],
            None,
        ),
        tool(
            "github_rate_limit",
            GITHUB,
            "rate_limit",
            Read,
            "Read the GitHub rate limits",
            "Reads how many GitHub API calls remain in each limit.",
            vec![],
            None,
        ),
    ]
}

fn notifications_and_gists() -> Vec<ToolSpec> {
    use Effect::{List, Read, Write};
    const GIST: Param = str_p("gist_id", 64, true, "The gist id, from github_gist_list.");
    vec![
        tool(
            "github_notification_list",
            GITHUB,
            "notification_list",
            Read,
            "List GitHub notifications",
            "Lists the user's GitHub notifications (unread by default): what, where and why.",
            vec![
                bool_p("all", "Include notifications already read."),
                bool_p("participating", "Only those where the user is directly involved."),
                str_p("repo", 140, false, "Only notifications of this repository (owner/name)."),
                LIMIT,
            ],
            None,
        ),
        tool(
            "github_notification_mark_read",
            GITHUB,
            "notification_mark_read",
            Write,
            "Mark GitHub notifications as read",
            "Marks one notification thread as read, or every notification (of one repository if given) when no thread \
             is named. The user approves on their phone.",
            vec![
                str_p("thread_id", 30, false, "The thread id from github_notification_list. Omit to mark all."),
                str_p("repo", 140, false, "With no thread: only this repository's notifications."),
            ],
            None,
        )
        .in_class("account"),
        tool(
            "github_gist_list",
            GITHUB,
            "gist_list",
            List,
            "List GitHub gists",
            "Lists the user's gists (or another user's public ones): id, description and file names.",
            vec![str_p("username", 100, false, "Another user; default: the user."), LIMIT],
            None,
        ),
        tool(
            "github_gist_get",
            GITHUB,
            "gist_get",
            Read,
            "Read a GitHub gist",
            "Reads a gist with the text of its files (each file at most 20000 characters).",
            vec![GIST],
            None,
        ),
        tool(
            "github_gist_create",
            GITHUB,
            "gist_create",
            Write,
            "Create a GitHub gist",
            "Creates a gist. It is secret (unlisted) unless public is true. The user sees the files and approves on \
             their phone.",
            vec![
                str_p("description", 500, false, "What the gist is."),
                Param {
                    required: true,
                    ..json_p(
                        "files",
                        400_000,
                        false,
                        "An object: file name to the file's text, e.g. {\"a.txt\": \"hello\"}.",
                    )
                },
                bool_p("public", "Make the gist public and listed. Default false."),
            ],
            None,
        )
        .in_class("account"),
        tool(
            "github_gist_update",
            GITHUB,
            "gist_update",
            Write,
            "Change a GitHub gist",
            "Changes a gist's description and files. In files, a name to text adds or replaces a file, a name to null \
             deletes it. The user approves on their phone.",
            vec![
                GIST,
                str_p("description", 500, false, "The new description."),
                json_p("files", 400_000, false, "An object: file name to new text, or to null to delete the file."),
            ],
            None,
        )
        .in_class("account"),
        tool(
            "github_gist_delete",
            GITHUB,
            "gist_delete",
            Write,
            "Delete a GitHub gist",
            "Deletes a gist for good. The user approves on their phone.",
            vec![GIST],
            None,
        )
        .in_class("account"),
        tool(
            "github_gist_star",
            GITHUB,
            "gist_star",
            Write,
            "Star or unstar a GitHub gist",
            "Stars a gist for the user, or removes the star. The user approves on their phone.",
            vec![GIST, bool_p("unstar", "Remove the star instead of adding it.")],
            None,
        )
        .in_class("account"),
    ]
}

fn request() -> Vec<ToolSpec> {
    use Effect::{Read, Write};
    const PATH: Param = str_p(
        "path",
        1_000,
        true,
        "The API path, starting with /repos/owner/name/..., /user, /users/, /orgs/, /search/, /gists, /notifications or \
         /rate_limit. No `..`, no `?` (use `query`), no encoded slashes. Credentials, keys, emails and apps are refused.",
    );
    const QUERY: Param = map_p("query", 20, 500, "Query parameters, name to value.");
    vec![
        tool(
            "github_request_read",
            GITHUB,
            "request_read",
            Read,
            "Read anything from the GitHub API",
            "The last resort when no other GitHub tool fits: a GET request to the GitHub REST API. Under /repos/ the user \
             ticks the result like any read; anything else is never covered by a standing permission. Prefer the \
             specific tools.",
            vec![PATH, QUERY],
            None,
        ),
        tool(
            "github_request_write",
            GITHUB,
            "request_write",
            Write,
            "Change anything through the GitHub API",
            "The last resort when no other GitHub tool fits: a POST, PUT, PATCH or DELETE request to the GitHub REST API. \
             The user sees the method, path and body and is asked every time. Prefer the specific tools.",
            vec![
                choice_p("method", &["post", "put", "patch", "delete"], true, "The HTTP method."),
                PATH,
                QUERY,
                json_p("body", 65_000, false, "The JSON body of the request."),
            ],
            None,
        )
        .in_class("settings")
        .once(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::connector::spec_for_tool;

    #[test]
    fn every_write_has_a_class_and_every_read_none() {
        for s in tools() {
            match s.effect {
                Effect::Write => assert!(!s.class.is_empty(), "{} needs a class", s.tool),
                _ => assert!(s.class.is_empty() && !s.once_only, "{} is a read", s.tool),
            }
            assert!(s.tool.strip_prefix("github_") == Some(s.op), "{}", s.tool);
            assert!(s.params.iter().all(|p| !p.description.is_empty()), "{}", s.tool);
        }
    }

    #[test]
    fn the_far_reaching_changes_are_asked_every_time() {
        for name in [
            "github_secret_set",
            "github_secret_delete",
            "github_environment_set",
            "github_environment_delete",
            "github_run_force_cancel",
            "github_run_approve",
            "github_deployment_review",
            "github_request_write",
        ] {
            assert!(spec_for_tool(name).unwrap().once_only, "{name}");
        }
        for name in ["github_workflow_dispatch", "github_variable_create", "github_star_add"] {
            assert!(!spec_for_tool(name).unwrap().once_only, "{name}");
        }
        assert_eq!(spec_for_tool("github_star_add").unwrap().class, "account");
        assert_eq!(spec_for_tool("github_variable_update").unwrap().class, "actions");
    }
}
