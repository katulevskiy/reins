//! Tools only the paired Reins desktop app may call, that belong to no other integration: asking the user anything
//! (`reins ask`, harness hooks), and git through the desktop app for hosts besides GitHub.

use crate::connector::{
    BITBUCKET, CODEBERG, ClassInfo, DESKTOP, Effect, GITLAB, Param, ToolSpec, choice_p, int_p, json_p, list_p, str_p,
    text_p, tool,
};

/// `integration_token_store`: a git host's token for the phone's integration.
pub const TOKEN_STORE_OP: &str = "token_store";
/// The integrations a token can be handed to, and the host each serves.
pub const TOKEN_HOSTS: &[(&str, &str)] =
    &[("github", "github.com"), ("gitlab", "gitlab.com"), ("codeberg", "codeberg.org"), ("bitbucket", "bitbucket.org")];

/// `desktop_session`: a work session, many permissions approved at once.
pub const SESSION_OP: &str = "session";
/// `desktop_session_end`: the session's permissions end now.
pub const SESSION_END_OP: &str = "session_end";
/// `desktop_autopilot_mode`: a change of the phone's global mode, asked from the desktop app. The phone applies what
/// is stricter at once (Lockdown, a stricter mode, ending a bypass or Overnight) and asks its owner for anything looser.
/// A bypass and Overnight start on the phone only.
pub const AUTOPILOT_MODE_OP: &str = "autopilot_mode";
/// The modes the desktop app may ask for: `stop` ends the running bypass or Overnight.
pub const AUTOPILOT_MODES: &[&str] = &["manual", "auto", "lockdown", "stop"];
/// `desktop_autopilot_correct`: "This was wrong" for a request Autopilot decided. Teaching it to deny applies at once;
/// teaching it to approve is asked.
pub const AUTOPILOT_CORRECT_OP: &str = "autopilot_correct";

/// The shortest and longest a work session may run.
pub const MIN_SESSION_SECS: i64 = 15 * 60;
pub const MAX_SESSION_SECS: i64 = 12 * 3_600;
/// The most permissions one session asks for.
pub const MAX_SESSION_ITEMS: usize = 20;

const CLIENT_KEY: Param =
    str_p("client_key", 64, true, "The desktop app's public key (X25519, base64url without padding).");
const NONCE: Param = str_p("nonce", 64, true, "Random, chosen by the desktop app; echoed inside the sealed answer.");
/// [`crate::autopilot::DesktopAsk`] boxed from the app's key to the phone's (base64url `nonce || ciphertext`).
const PROOF: Param = str_p(
    "proof",
    4096,
    false,
    "Proof that the paired desktop app itself asks: the call boxed from its key to the phone's key.",
);
const REPO: Param = str_p("repo", 250, true, "The repository path: owner/name (GitLab: group/subgroup/name).");
const DIGEST: Param = str_p("digest", 64, true, "SHA-256 (hex) of exactly what is pushed, see desktop::push_digest.");
const SUMMARY: Param = json_p("summary", 200_000, true, "What the push does: a desktop::PushSummary.");

/// The kinds of change of the git hosts besides GitHub.
pub(super) const HOST_CLASSES: &[ClassInfo] = &[
    ClassInfo {
        id: "code",
        label: "Commits and branches",
    },
    ClassInfo {
        id: "releases",
        label: "Tags",
    },
];

fn host_tools(service: &'static str, fetch: &'static str, push: &'static str, tag_push: &'static str) -> Vec<ToolSpec> {
    vec![
        tool(
            fetch,
            service,
            crate::desktop::GIT_FETCH_OP,
            Effect::Read,
            "Clone and fetch with git",
            "Read access to one repository for git on the user's computer, for up to an hour.",
            vec![REPO, CLIENT_KEY, NONCE],
            Some("repo"),
        )
        .desktop(),
        tool(
            push,
            service,
            crate::desktop::GIT_PUSH_OP,
            Effect::Write,
            "Push with git",
            "Push commits to branches of one repository from git on the user's computer.",
            vec![REPO, CLIENT_KEY, NONCE, DIGEST, SUMMARY],
            Some("repo"),
        )
        .in_class("code")
        .desktop(),
        tool(
            tag_push,
            service,
            crate::desktop::GIT_TAG_PUSH_OP,
            Effect::Write,
            "Push tags with git",
            "Push tags to one repository from git on the user's computer.",
            vec![REPO, CLIENT_KEY, NONCE, DIGEST, SUMMARY],
            Some("repo"),
        )
        .in_class("releases")
        .desktop(),
    ]
}

pub(super) fn tools() -> Vec<ToolSpec> {
    let mut all = vec![
        tool(
            "desktop_ask",
            DESKTOP,
            "ask",
            Effect::Write,
            "Ask you on your phone",
            "A yes-or-no question from the desktop app (`reins ask`, or a harness hook before a command).",
            vec![
                str_p("question", 300, true, "The question, one line."),
                text_p("detail", 8_000, false, "What exactly would happen (the command, the files)."),
                text_p(
                    "net",
                    400,
                    false,
                    "The safety net the desktop app applies if approved, one line per net (set by its hooks only).",
                ),
                str_p(
                    "topic",
                    100,
                    false,
                    "What a standing answer may cover (`command:git push --force`); default: ask.",
                ),
                str_p(
                    "digest",
                    64,
                    false,
                    "The command's digest: a deferred yes lets exactly this command through once.",
                ),
                CLIENT_KEY,
                NONCE,
            ],
            Some("topic"),
        )
        .desktop(),
    ];
    all.push(
        tool(
            "desktop_session",
            DESKTOP,
            SESSION_OP,
            Effect::Write,
            "Start a work session",
            "Permissions for a while, approved at once: reading integrations, and pushing to named branches with git. \
             Never covers what is asked for every time (force pushes, deleting, the vault, purchases).",
            vec![
                int_p(
                    "duration_secs",
                    MIN_SESSION_SECS,
                    MAX_SESSION_SECS,
                    None,
                    "How long the session lasts (15 minutes to 12 hours).",
                ),
                str_p("reason", 300, true, "What the session is for, in the user's words."),
                list_p(
                    "read",
                    MAX_SESSION_ITEMS,
                    40,
                    "Integrations to read (search, list, read) during the session: `gmail`, `gcalendar`, `github`, ...",
                ),
                list_p(
                    "push",
                    MAX_SESSION_ITEMS,
                    300,
                    "Branches git may push to: `<service>:<repo>@<branch>` (`github:me/app@feature/login`).",
                ),
                CLIENT_KEY,
                NONCE,
            ],
            None,
        )
        .once()
        .desktop(),
    );
    all.push(
        tool(
            "desktop_session_end",
            DESKTOP,
            SESSION_END_OP,
            Effect::Write,
            "End a work session",
            "End the permissions of a work session now. Takes access away only, so it is never asked.",
            vec![list_p("grants", 50, 64, "The session's permission ids."), CLIENT_KEY, NONCE],
            None,
        )
        .desktop(),
    );
    all.push(
        tool(
            "desktop_autopilot_mode",
            DESKTOP,
            AUTOPILOT_MODE_OP,
            Effect::Write,
            "Change Autopilot's mode",
            "Set the phone's mode for every AI: Manual, Auto or Lockdown, or stop the bypass or Overnight that runs. \
             What is stricter applies at once; anything looser is asked on the phone, and only when the desktop app \
             proved it asked (`proof`).",
            vec![
                choice_p("mode", AUTOPILOT_MODES, true, "The mode; `stop` ends the running bypass or Overnight."),
                PROOF,
                CLIENT_KEY,
                NONCE,
            ],
            None,
        )
        .once()
        .desktop(),
    );
    all.push(
        tool(
            "desktop_autopilot_correct",
            DESKTOP,
            AUTOPILOT_CORRECT_OP,
            Effect::Write,
            "Tell Autopilot it was wrong",
            "\"This was wrong\" for a request of this computer that Autopilot decided: what it should have done. \
             Teaching it to deny applies at once; teaching it to approve is asked on the phone.",
            vec![
                str_p("request_id", 100, true, "The request Autopilot decided."),
                choice_p("should_have", &["approve", "deny"], true, "What it should have done."),
                PROOF,
                CLIENT_KEY,
                NONCE,
            ],
            None,
        )
        .once()
        .desktop(),
    );
    all.push(
        tool(
            crate::desktop::TOKEN_STORE_TOOL,
            DESKTOP,
            TOKEN_STORE_OP,
            Effect::Write,
            "Add a git host account from the computer",
            "Hand a git host's token found on the computer (gh, .git-credentials) to the phone's integration of that \
             host, in a box only the phone opens.",
            vec![
                choice_p("service", &["github", "gitlab", "codeberg", "bitbucket"], true, "The integration."),
                str_p("host", 255, true, "The host the token was found for."),
                list_p("scopes", 30, 100, "What the computer knows of the token's scopes (shown, not trusted)."),
                text_p(
                    "sealed",
                    20_000,
                    true,
                    "The token, boxed to the phone's key (base64url of nonce and ciphertext).",
                ),
                CLIENT_KEY,
                NONCE,
            ],
            None,
        )
        .once()
        .desktop(),
    );
    all.extend(host_tools(GITLAB, "gitlab_git_fetch", "gitlab_git_push", "gitlab_git_tag_push"));
    all.extend(host_tools(CODEBERG, "codeberg_git_fetch", "codeberg_git_push", "codeberg_git_tag_push"));
    all.extend(host_tools(BITBUCKET, "bitbucket_git_fetch", "bitbucket_git_push", "bitbucket_git_tag_push"));
    all
}
