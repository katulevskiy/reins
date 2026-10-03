//! Vault tools: Sends (text and files shared by link), and the password, passphrase and username generator.
//!
//! The resource of a Send is `sends/{id}` (creating one: `sends`); the generator's is `generator`. Sends are the
//! `sends` class of change.

use crate::connector::{Effect, LIMIT, ToolSpec, VAULT, bool_p, choice_p, int_p, str_p, text_p, tool};

pub(super) fn tools() -> Vec<ToolSpec> {
    use Effect::{List, Read, Write};
    vec![
        tool(
            "vault_send_list",
            VAULT,
            "send_list",
            List,
            "List Bitwarden Sends",
            "Lists the user's Sends (text or files shared by link): name, type, how often each was opened and the most \
             allowed, expiration and deletion dates, whether it is disabled, has a password or hides the creator's \
             email. Never links or content; use vault_send_get for those. The user chooses which Sends to show you.",
            vec![str_p("query", 100, false, "Only Sends whose name contains this text."), LIMIT],
            None,
        ),
        tool(
            "vault_send_get",
            VAULT,
            "send_get",
            Read,
            "Get a Bitwarden Send with its link",
            "Fetches one Send: its text (or the file's name and size, never the file itself) and the link anyone can \
             open it with. The user approves every time on their phone; a standing permission never covers it. The \
             password of a Send cannot be read back.",
            vec![str_p("send", 100, true, "The id of the Send, from vault_send_list.")],
            Some("send"),
        ),
        tool(
            "vault_send_create",
            VAULT,
            "send_create",
            Write,
            "Create a Bitwarden Send",
            "Creates a Send that is encrypted on the phone and shared by a link, and returns the link once. A text \
             Send needs `text`; a file Send needs `file_name` and `content_base64`. Without `delete_in_days` it is \
             deleted after 7 days; the vault server allows 31 days at most. The user sees the name, the limits and \
             the start of the text (not a hidden text, not the password) and approves on their phone unless they \
             granted a standing permission for Sends.",
            vec![
                choice_p("type", &["text", "file"], true, "text or file."),
                str_p("name", 300, true, "The name of the Send, as the user sees it in their vault."),
                text_p("notes", 1_000, false, "Private notes, only the user sees them."),
                text_p("text", 65_000, false, "The text to share (type text)."),
                bool_p("hidden", "Text Sends: hide the text until the recipient chooses to show it."),
                str_p("file_name", 200, false, "The name the file has for the recipient (type file)."),
                text_p(
                    "content_base64",
                    3_000_000,
                    false,
                    "The content of the file, base64 encoded (type file). At most 2 MB once decoded.",
                ),
                str_p("blob", 64, false, "A file uploaded to the link the phone gave, instead of content_base64 (for files over 2 MB)."),
                text_p("password", 200, false, "Recipients must enter this password to open the Send."),
                int_p("max_access_count", 1, 1_000_000, None, "The Send stops opening after this many views."),
                int_p(
                    "expires_in_hours",
                    1,
                    8_760,
                    None,
                    "The Send stops opening this many hours from now (not with expiration_date).",
                ),
                str_p(
                    "expiration_date",
                    40,
                    false,
                    "When the Send stops opening: a date (2026-10-05) or a moment with offset (2026-10-05T14:00:00+02:00).",
                ),
                int_p(
                    "delete_in_days",
                    1,
                    31,
                    None,
                    "The Send is deleted this many days from now, 1 to 31 (default 7; not with deletion_date).",
                ),
                str_p(
                    "deletion_date",
                    40,
                    false,
                    "When the Send is deleted: a date or a moment with offset, less than 31 days from now.",
                ),
                bool_p("disabled", "Create it switched off: nobody can open it until the user enables it."),
                bool_p("hide_email", "Do not show the user's email address to recipients."),
            ],
            None,
        )
        .in_class("sends"),
        tool(
            "vault_send_update",
            VAULT,
            "send_update",
            Write,
            "Change a Bitwarden Send",
            "Changes only the fields given on an existing Send; the type and a file cannot change, and the link stays \
             the same. Give at least one field. The user sees the old and new values on their phone (not passwords, \
             not a hidden text) and approves unless they granted a standing permission for Sends.",
            vec![
                str_p("send", 100, true, "The id of the Send, from vault_send_list."),
                str_p("name", 300, false, "New name."),
                text_p("notes", 1_000, false, "New notes; an empty string removes them."),
                text_p("text", 65_000, false, "New text (text Sends only)."),
                bool_p("hidden", "Hide or show the text until the recipient chooses (text Sends only)."),
                int_p(
                    "max_access_count",
                    0,
                    1_000_000,
                    None,
                    "New limit of views; 0 removes the limit.",
                ),
                int_p(
                    "expires_in_hours",
                    0,
                    8_760,
                    None,
                    "The Send stops opening this many hours from now; 0 removes the expiration.",
                ),
                str_p("expiration_date", 40, false, "When the Send stops opening: a date or a moment with offset."),
                int_p("delete_in_days", 1, 31, None, "Delete this many days from now, 1 to 31."),
                str_p(
                    "deletion_date",
                    40,
                    false,
                    "When the Send is deleted: a date or a moment with offset, less than 31 days from now.",
                ),
                text_p("password", 200, false, "A new password for the Send (use vault_send_remove_password to remove it)."),
                bool_p("disabled", "Switch the Send off (true) or on (false)."),
                bool_p("hide_email", "Hide (true) or show (false) the user's email address to recipients."),
            ],
            Some("send"),
        )
        .in_class("sends"),
        tool(
            "vault_send_remove_password",
            VAULT,
            "send_remove_password",
            Write,
            "Remove the password of a Bitwarden Send",
            "Removes the password of a Send, so anyone with the link can open it. The user approves on their phone \
             unless they granted a standing permission for Sends.",
            vec![str_p("send", 100, true, "The id of the Send, from vault_send_list.")],
            Some("send"),
        )
        .in_class("sends"),
        tool(
            "vault_send_delete",
            VAULT,
            "send_delete",
            Write,
            "Delete a Bitwarden Send",
            "Deletes a Send for good; its link stops working. The user approves on their phone unless they granted a \
             standing permission for Sends.",
            vec![str_p("send", 100, true, "The id of the Send, from vault_send_list.")],
            Some("send"),
        )
        .in_class("sends"),
        tool(
            "vault_generate_password",
            VAULT,
            "generate_password",
            Read,
            "Generate a random password",
            "Generates a random password on the user's phone (nothing is sent anywhere and nothing is stored). Every \
             character class that is on appears at least once (or `min_*` times) as far as the length allows. The user \
             approves on their phone unless they granted a standing permission for the generator; they see that a \
             password was generated, not the password.",
            vec![
                int_p("length", 5, 128, Some(16), "How many characters."),
                bool_p("uppercase", "Use A-Z (default true)."),
                bool_p("lowercase", "Use a-z (default true)."),
                bool_p("numbers", "Use 0-9 (default true)."),
                bool_p("symbols", "Use !@#$%^&*()-_=+[]{};:,.? (default true)."),
                int_p("min_uppercase", 0, 128, None, "At least this many capital letters."),
                int_p("min_lowercase", 0, 128, None, "At least this many lowercase letters."),
                int_p("min_numbers", 0, 128, None, "At least this many digits."),
                int_p("min_symbols", 0, 128, None, "At least this many symbols."),
                bool_p("avoid_ambiguous", "Leave out characters that look alike: I l 1 O 0."),
                text_p("exclude", 100, false, "Characters that must never appear."),
            ],
            None,
        ),
        tool(
            "vault_generate_passphrase",
            VAULT,
            "generate_passphrase",
            Read,
            "Generate a random passphrase",
            "Generates a passphrase of random words from the EFF large word list (7776 words) on the user's phone. \
             The user approves on their phone unless they granted a standing permission for the generator; they see \
             that a passphrase was generated, not the passphrase.",
            vec![
                int_p("words", 3, 20, Some(6), "How many words."),
                text_p("separator", 10, false, "Put between the words (default \"-\"; may be empty)."),
                bool_p("capitalize", "Capitalize each word (default false)."),
                bool_p("include_number", "Add a random digit to the end of one random word (default false)."),
            ],
            None,
        ),
        tool(
            "vault_generate_username",
            VAULT,
            "generate_username",
            Read,
            "Generate a random username",
            "Generates a username on the user's phone: a random word (optionally capitalized, with a number), a \
             plus-addressed email (name+random@domain from the user's own address) or a catch-all email (random@domain). \
             The user approves on their phone unless they granted a standing permission for the generator.",
            vec![
                choice_p(
                    "type",
                    &["random_word", "plus_addressed_email", "catch_all_email"],
                    true,
                    "Which kind of username.",
                ),
                bool_p("capitalize", "random_word: capitalize the word."),
                bool_p("include_number", "random_word: add four random digits."),
                str_p("email", 254, false, "plus_addressed_email: the user's own email address."),
                str_p("domain", 253, false, "catch_all_email: the domain that accepts every address."),
            ],
            None,
        ),
        tool(
            "vault_generate_check_password",
            VAULT,
            "generate_check_password",
            Read,
            "Check the strength of a password",
            "Estimates how strong a password is (length, character classes, entropy, a short list of very common \
             passwords) on the user's phone. The password is not sent to any service, not looked up in breach \
             databases, not stored and not shown to the user. The user approves on their phone unless they granted a \
             standing permission for the generator.",
            vec![text_p("password", 1_000, true, "The password to check.")],
            None,
        ),
    ]
}
