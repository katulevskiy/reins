//! Vault tools only the paired Reins desktop app may call: secrets for one command or API route (`reins run`,
//! the API proxy), and SSH signing (`reins`'s SSH agent). The private SSH key never leaves the phone: the phone signs.

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
            "Saves a value typed on the computer (`reins vault add`) in a vault item, boxed from the desktop app's key \
             to the phone's key: the server cannot read or change it. Creates the item, or changes the field of the item with that exact name.",
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
            Effect::Read,
            "List vault item names for the computer",
            "The names and kinds of the vault's items (never a value), for `reins vault list`.",
            vec![CLIENT_KEY, NONCE],
            None,
        )
        .desktop(),
    ]
}
