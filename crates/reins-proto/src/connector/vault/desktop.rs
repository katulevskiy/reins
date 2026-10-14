//! Vault tools only the paired Reins desktop app may call: secrets for one command or API route (`reins run`,
//! the API proxy), and SSH signing (`reins`'s SSH agent). The private SSH key never leaves the phone: the phone signs.

use crate::connector::{
    ConnectorCall, Effect, Param, ToolSpec, VAULT, bool_p, choice_p, int_p, json_p, list_p, str_p, text_p, tool,
};
use crate::desktop::{
    ENV_RELEASE_TOOL, MAX_ARG, MAX_ARGS, MAX_ENV_VARS, MAX_STORE_ENVS, MAX_STORE_SSH, MAX_STORE_TOKENS,
    SECRETS_LIST_TOOL, SECRETS_STORE_TOOL,
};

const CLIENT_KEY: Param =
    str_p("client_key", 64, true, "The desktop app's public key (X25519, base64url without padding).");
const NONCE: Param = str_p("nonce", 64, true, "Random, chosen by the desktop app; echoed inside the sealed answer.");

/// `vault_env_release`'s `args`: a list of strings, each at most [`MAX_ARG`] characters without control characters,
/// kept exactly (an argument may be empty or start with a space).
pub fn env_release_args(call: &ConnectorCall) -> Result<Vec<String>, String> {
    let list = match call.args.get("args") {
        None => return Ok(Vec::new()),
        Some(serde_json::Value::Array(list)) => list,
        Some(_) => return Err("`args` must be a list of strings.".to_owned()),
    };
    if list.len() > MAX_ARGS {
        return Err(format!("`args` has at most {MAX_ARGS} entries."));
    }
    list.iter()
        .map(|a| match a.as_str() {
            Some(s) if s.chars().count() <= MAX_ARG && !s.chars().any(char::is_control) => Ok(s.to_owned()),
            _ => Err(format!("Each of `args` is a string of at most {MAX_ARG} characters.")),
        })
        .collect()
}

fn check_env_release(call: &ConnectorCall) -> Result<(), String> {
    env_release_args(call).map(drop)?;
    let runner = call.str_arg("runner").unwrap_or_default();
    if !crate::desktop::runner_ok(runner) {
        return Err("`runner` is `person` or `agent:<harness>`.".to_owned());
    }
    for name in ["project", "env"] {
        if !crate::desktop::name_ok(call.str_arg(name).unwrap_or_default()) {
            return Err(format!("`{name}` is not a project or environment name."));
        }
    }
    if !call.list_arg("fields").iter().all(|f| crate::desktop::var_ok(f)) {
        return Err("`fields` are variable names.".to_owned());
    }
    Ok(())
}

pub(super) fn tools() -> Vec<ToolSpec> {
    vec![
        tool(
            ENV_RELEASE_TOOL,
            VAULT,
            "env_release",
            Effect::Write,
            "Use a project's secrets on the computer",
            "Hands the variables of one environment of a project (Reins Secrets) to the desktop app for one command, \
             sealed to its key. The user sees who runs what where, and the variables' names, never a value.",
            vec![
                str_p("project", 64, true, "The project (the vault folder Reins/<project>)."),
                str_p("env", 64, true, "The environment (the item <project>/<env> in that folder)."),
                list_p("fields", MAX_ENV_VARS, 100, "The variables; left out: every variable of the environment."),
                str_p("program", 1_000, true, "The program reins run starts, by its absolute path when known."),
                str_p("typed", 1_000, false, "The program as typed (`npm`), when it was not a path."),
                choice_p(
                    "path_check",
                    &[crate::desktop::PATH_SAME, crate::desktop::PATH_OTHER],
                    false,
                    "Whether the desktop app's own login PATH finds the same program for the name typed.",
                ),
                json_p("args", 70_000, false, "Its arguments, exactly as passed (a list of strings; no shell)."),
                str_p("root", 1_000, true, "The project's folder on the computer (canonical path)."),
                str_p("runner", 50, true, "Who runs it: `person`, or `agent:<harness>`."),
                int_p("lease_secs", 60, 86_400, Some(3_600), "How long the desktop app may keep them in memory."),
                str_p("purpose", 300, false, "Why, in the user's words or the agent's."),
                choice_p("confirm", &["default", "always"], false, "`always`: reins.toml asks every time."),
                bool_p("print", "The value is printed in the terminal (`reins secrets get`): always asked."),
                CLIENT_KEY,
                NONCE,
            ],
            None,
        )
        .in_class("secrets")
        .checked(check_env_release)
        .desktop(),
        tool(
            SECRETS_STORE_TOOL,
            VAULT,
            "secrets_store",
            Effect::Write,
            "Save a project's secrets from the computer",
            "Sets, removes, renames and copies variables of environments of projects (Reins Secrets), keeps SSH keys \
             and hands git hosts' tokens to the phone's integrations, sent by the desktop app in a box only the phone \
             opens. One approval for the batch; the user compares four check digits with the computer.",
            vec![
                list_p("envs", MAX_STORE_ENVS, 130, "The environments changed: `project/env`, as in the box."),
                list_p("ssh", MAX_STORE_SSH, 200, "The SSH keys saved in Reins/ssh, by name, as in the box."),
                list_p("hosts", MAX_STORE_TOKENS, 100, "The git hosts whose tokens go to the phone, as in the box."),
                text_p("sealed", 480_000, true, "The changes, boxed to the phone's key (base64url of nonce and ciphertext)."),
                CLIENT_KEY,
                NONCE,
            ],
            None,
        )
        .in_class("items")
        .once()
        .desktop(),
        tool(
            SECRETS_LIST_TOOL,
            VAULT,
            "secrets_list",
            Effect::Write,
            "Show the names of a project's secrets on the computer",
            "The projects, environments and variable names of Reins Secrets (never a value), for `reins secrets ls`.",
            vec![str_p("project", 64, false, "Only this project."), CLIENT_KEY, NONCE],
            Some("project"),
        )
        .in_class("secrets")
        .desktop(),
        tool(
            "vault_secret_release",
            VAULT,
            "secret_release",
            Effect::Write,
            "Use secrets on the computer",
            "Hands secrets from the vault to the desktop app for one command or API route, sealed to its key. `secrets` \
             lists `item/field` references (field: password, username, totp, notes, or a custom field name).",
            vec![
                list_p("secrets", 10, 300, "`item/field` references: an item id or exact name, then the field."),
                str_p("command", 500, true, "What will use them, as shown to the user (`npm run deploy`, `API openai`)."),
                str_p("purpose", 300, false, "Why, in the user's words or the agent's."),
                int_p("lease_secs", 60, 86_400, Some(3_600), "How long the desktop app may keep them in memory."),
                CLIENT_KEY,
                NONCE,
            ],
            None,
        )
        .in_class("secrets")
        .desktop(),
        tool(
            "vault_ssh_keys",
            VAULT,
            "ssh_keys",
            Effect::Read,
            "List SSH keys for the computer",
            "The public halves of the vault's SSH keys, for the desktop app's SSH agent.",
            vec![CLIENT_KEY, NONCE],
            None,
        )
        .desktop(),
        tool(
            "vault_ssh_sign",
            VAULT,
            "ssh_sign",
            Effect::Write,
            "Sign in to a server with SSH",
            "Signs one SSH authentication with a vault SSH key, on the phone. The user sees the key and the server.",
            vec![
                str_p("key", 100, true, "The key's SHA256 fingerprint (`SHA256:…`)."),
                text_p("data_base64", 65_536, true, "The data the SSH client asks to sign, base64."),
                int_p("flags", 0, 255, Some(0), "SSH agent signature flags (2: rsa-sha2-256, 4: rsa-sha2-512)."),
                str_p("host", 255, false, "The server's name, when the desktop app knows it."),
                str_p("host_key", 100, false, "The server's host key fingerprint, when the SSH client said."),
                CLIENT_KEY,
                NONCE,
            ],
            Some("key"),
        )
        .in_class("ssh")
        .desktop(),
        tool(
            "vault_phone_key",
            VAULT,
            "phone_key",
            Effect::Write,
            "Let the computer save secrets to the vault",
            "The phone's own public key, which `reins vault add` seals a secret to so that only the phone can open it. \
             The user compares its fingerprint on both screens once.",
            vec![CLIENT_KEY, NONCE],
            None,
        )
        .once()
        .desktop(),
        tool(
            "vault_secret_store",
            VAULT,
            "secret_store",
            Effect::Write,
            "Save a secret from the computer",
            "Saves a value sent by `reins vault add` in a vault item, boxed from the desktop app's key to the phone's \
             key: the server cannot read or change it. Makes a new item; changes the field of the item with that \
             name only when the box says to replace it.",
            vec![
                str_p("name", 300, true, "The item's exact name."),
                str_p("kind", 20, true, "`api-key`, `login`, `note` or `ssh`."),
                str_p("field", 100, true, "`password`, `username`, `notes`, `private_key`, or a custom field's name."),
                text_p("sealed", 40_000, true, "The value, boxed to the phone's key (base64url of nonce and ciphertext)."),
                CLIENT_KEY,
                NONCE,
            ],
            Some("name"),
        )
        .in_class("items")
        .once()
        .desktop(),
        tool(
            "vault_names",
            VAULT,
            "names",
            Effect::Write,
            "Show the vault item names on the computer",
            "The names and kinds of the vault's items (never a value), for `reins vault list`. Asked every time.",
            vec![CLIENT_KEY, NONCE],
            None,
        )
        .once()
        .desktop(),
    ]
}
