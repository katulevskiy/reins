//! Vault tools only the paired Rewarden desktop app may call: secrets for one command or API route (`rewarden run`,
//! the API proxy), and SSH signing (`rewarden`'s SSH agent). The private SSH key never leaves the phone: the phone signs.

use crate::connector::{Effect, Param, ToolSpec, VAULT, int_p, list_p, str_p, text_p, tool};

const CLIENT_KEY: Param =
    str_p("client_key", 64, true, "The desktop app's public key (X25519, base64url without padding).");
const NONCE: Param = str_p("nonce", 64, true, "Random, chosen by the desktop app; echoed inside the sealed answer.");

pub(super) fn tools() -> Vec<ToolSpec> {
    vec![
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
    ]
}
