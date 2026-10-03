//! Git over HTTPS through the Reins desktop app. These are not AI tools: the desktop app's git proxy asks for them
//! when a git client fetches or pushes, and the phone answers with the GitHub credential sealed to the app's key.

use crate::connector::{Effect, GITHUB, Param, ToolSpec, json_p, str_p, tool};
use crate::desktop::{
    GIT_FETCH_OP, GIT_FETCH_TOOL, GIT_PUSH_OP, GIT_PUSH_TOOL, GIT_TAG_PUSH_OP, GIT_TAG_PUSH_TOOL, MAX_SUMMARY_BYTES,
};

const REPO: Param = str_p("repo", 140, true, "owner/name.");
const CLIENT_KEY: Param =
    str_p("client_key", 64, true, "The desktop app's public key (X25519, base64url without padding).");
const NONCE: Param = str_p("nonce", 64, true, "Random, chosen by the desktop app; echoed inside the sealed answer.");
const DIGEST: Param = str_p("digest", 64, true, "SHA-256 (hex) of exactly what is pushed, see desktop::push_digest.");
const SUMMARY: Param =
    json_p("summary", MAX_SUMMARY_BYTES + 10_000, true, "What the push does: a desktop::PushSummary.");

pub(super) fn tools() -> Vec<ToolSpec> {
    vec![
        tool(
            GIT_FETCH_TOOL,
            GITHUB,
            GIT_FETCH_OP,
            Effect::Read,
            "Clone and fetch with git",
            "Read access to one repository for git on the user's computer, for up to an hour.",
            vec![REPO, CLIENT_KEY, NONCE],
            Some("repo"),
        )
        .desktop(),
        tool(
            GIT_PUSH_TOOL,
            GITHUB,
            GIT_PUSH_OP,
            Effect::Write,
            "Push with git",
            "Pushes commits to branches of one repository from git on the user's computer.",
            vec![REPO, CLIENT_KEY, NONCE, DIGEST, SUMMARY],
            Some("repo"),
        )
        .in_class("code")
        .desktop(),
        tool(
            GIT_TAG_PUSH_TOOL,
            GITHUB,
            GIT_TAG_PUSH_OP,
            Effect::Write,
            "Push tags with git",
            "Pushes tags to one repository from git on the user's computer.",
            vec![REPO, CLIENT_KEY, NONCE, DIGEST, SUMMARY],
            Some("repo"),
        )
        .in_class("releases")
        .desktop(),
    ]
}
