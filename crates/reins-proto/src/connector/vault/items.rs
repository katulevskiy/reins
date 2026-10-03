//! Vault tools: items (logins, notes, cards, identities, SSH keys), folders, the trash and the archive.

use crate::connector::{Effect, LIMIT, ToolSpec, VAULT, bool_p, choice_p, json_p, list_p, str_p, text_p, tool};

/// The item types, as an AI names them.
const TYPES: &[&str] = &["login", "note", "card", "identity", "ssh_key"];

/// What `vault_get` can fetch.
const FIELDS: &[&str] = &[
    "username",
    "totp",
    "password",
    "notes",
    "uri",
    "card_number",
    "card_code",
    "card_holder",
    "card_expiry",
    "identity",
    "identity_ssn",
    "identity_passport",
    "identity_license",
    "ssh_private_key",
    "ssh_public_key",
    "ssh_fingerprint",
    "custom",
    "password_history",
];

const ITEM_HELP: &str = "The id of the item, from vault_search.";

const FIELDS_HELP: &str = "The type-specific content, an object. login: {username, password, totp (a base32 secret or an \
    otpauth:// address), uris (list of addresses)}. note: {} (the text goes in `notes`). card: {holder, brand, number, \
    exp_month, exp_year, code}. identity: {title, first_name, middle_name, last_name, address1, address2, address3, \
    city, state, postal_code, country, company, email, phone, ssn, username, passport_number, license_number}. \
    ssh_key: {private_key, public_key, fingerprint (worked out from the public key when left out)}. Every value is \
    a string (exp_month and exp_year may be numbers). Everything is encrypted on the phone before it reaches the vault.";

const CUSTOM_HELP: &str = "Custom fields, a list of {name, value, type} where type is text (default), hidden or boolean \
    (value true or false).";

pub(super) fn tools() -> Vec<ToolSpec> {
    #[allow(unused_imports, reason = "the areas use whichever effects they have tools for")]
    use Effect::{List, Read, Search, Write};
    vec![
        tool(
            "vault_search",
            VAULT,
            "search",
            Search,
            "Search the password vault",
            "Finds items in the user's vault by name, username, holder or website. By default only logins that are not \
             in the trash or the archive are searched; use `type`, `folder`, `state` and `favorite` to narrow or \
             widen. Shows names, usernames and websites only, never passwords, codes or other secrets. Leave `query` \
             out to list everything that matches the filters. The user ticks what to show you on their phone.",
            vec![
                str_p("query", 100, false, "Part of a name, username, holder or website."),
                choice_p(
                    "type",
                    &["login", "note", "card", "identity", "ssh_key", "any"],
                    false,
                    "Only this kind of item; `any` for all kinds. Default: login.",
                ),
                str_p(
                    "folder",
                    100,
                    false,
                    "Only items in this folder (its id or exact name), or `none` for no folder.",
                ),
                choice_p(
                    "state",
                    &["active", "trash", "archived", "all"],
                    false,
                    "Which items: active (default), those in the trash, the archived ones, or all.",
                ),
                bool_p("favorite", "true: only favorites; false: only items that are not favorites."),
                LIMIT,
            ],
            None,
        ),
        tool(
            "vault_folders_list",
            VAULT,
            "folders_list",
            List,
            "List vault folders",
            "Lists the user's folders (id, name and how many items of each type they hold), including `none` for \
             items without a folder. Use the ids with vault_search, vault_item_move and vault_item_create.",
            vec![],
            None,
        ),
        tool(
            "vault_item_view",
            VAULT,
            "item_view",
            Read,
            "View a vault item without its secrets",
            "Shows an overview of one item: type, name, folder, favorite, state, dates, websites, username, the length \
             of the notes, card brand, last four digits and expiry, the main identity fields (no ID numbers), the SSH \
             public key and fingerprint, custom field names and types, attachment names and sizes and how many old \
             passwords are kept. Never any secret. Use vault_get for a secret.",
            vec![str_p("item", 100, true, ITEM_HELP)],
            Some("item"),
        ),
        tool(
            "vault_get",
            VAULT,
            "get",
            Read,
            "Get a field from the password vault",
            "Fetches one field of one item: a login's username, password, one-time code or website; the notes; a \
             card's number, security code, holder or expiry; an identity as text or its social security, passport or \
             license number; an SSH key's private key, public key or fingerprint; custom fields; the old passwords. \
             The user approves every time on their phone and sees which field, not its value, and secrets are never \
             covered by a standing permission. Only the website, the SSH public key and fingerprint, the card holder \
             and the card expiry are ordinary reads.",
            vec![
                str_p("item", 100, true, ITEM_HELP),
                choice_p("field", FIELDS, true, "What to fetch."),
                str_p(
                    "custom_field",
                    100,
                    false,
                    "With field=custom: the name of one custom field. Left out, all custom fields are fetched.",
                ),
            ],
            Some("item"),
        ),
        tool(
            "vault_attachment_get",
            VAULT,
            "attachment_get",
            Read,
            "Get a file attached to a vault item",
            "Downloads one attachment of an item and decrypts it on the phone. The content arrives base64-encoded \
             (files over 2 MB are not sent, only their name and size). The user approves every time on their phone \
             and sees the file name, and it is never covered by a standing permission.",
            vec![
                str_p("item", 100, true, ITEM_HELP),
                str_p("attachment", 300, true, "The attachment: its id or exact file name, from vault_item_view."),
            ],
            Some("item"),
        ),
        tool(
            "vault_item_create",
            VAULT,
            "item_create",
            Write,
            "Create a vault item",
            "Adds a login, secure note, card, identity or SSH key to the user's vault. Give `type`, `name` and the \
             type's content in `fields`. The user sees every field on their phone except secrets, of which only the \
             length is shown, and approves unless a standing permission covers the folder and type.",
            vec![
                choice_p("type", TYPES, true, "The kind of item."),
                str_p("name", 300, true, "The item's name."),
                text_p("notes", 10_000, false, "Free-text notes."),
                bool_p("favorite", "Mark the item as a favorite."),
                str_p("folder", 100, false, "The folder (its id or exact name). Default: no folder."),
                json_p("fields", 20_000, false, FIELDS_HELP),
                json_p("custom_fields", 20_000, false, CUSTOM_HELP),
                bool_p("reprompt", "Ask for the master password again before showing the item in a Bitwarden app."),
            ],
            Some("folder"),
        )
        .in_class("items"),
        tool(
            "vault_item_update",
            VAULT,
            "item_update",
            Write,
            "Edit a vault item",
            "Changes the given parts of an item and leaves the rest as it is; give at least one. `fields` holds only \
             the type-specific values to change (an empty string clears one; `uris` replaces the list of websites). \
             `custom_fields` adds or changes custom fields by name; `remove_custom_fields` removes some. The user \
             sees old and new values on their phone, secrets only as 'changes'. An item's type cannot change.",
            vec![
                str_p("item", 100, true, ITEM_HELP),
                str_p("name", 300, false, "The new name."),
                text_p("notes", 10_000, false, "The new notes; an empty string clears them."),
                bool_p("favorite", "true or false to change the favorite mark."),
                str_p("folder", 100, false, "Move to this folder (its id or exact name), or `none`."),
                json_p("fields", 20_000, false, FIELDS_HELP),
                json_p("custom_fields", 20_000, false, CUSTOM_HELP),
                list_p("remove_custom_fields", 50, 100, "Names of custom fields to remove."),
                bool_p("reprompt", "Ask for the master password again before showing the item in a Bitwarden app."),
            ],
            Some("item"),
        )
        .in_class("items"),
        tool(
            "vault_attachment_add",
            VAULT,
            "attachment_add",
            Write,
            "Attach a file to a vault item",
            "Adds a file to an item. `content_base64` is the file in base64 (at most 2 MB once decoded, 3,000,000 \
             characters of base64). The phone encrypts it before upload. The user sees the file name and size and \
             approves.",
            vec![
                str_p("item", 100, true, ITEM_HELP),
                str_p("file_name", 200, true, "The file name to show, without a path."),
                text_p("content_base64", 3_000_000, false, "The file, base64-encoded."),
                str_p(
                    "blob",
                    64,
                    false,
                    "A file uploaded to the link the phone gave, instead of content_base64 (for files over 2 MB).",
                ),
            ],
            Some("item"),
        )
        .in_class("items"),
        tool(
            "vault_attachment_delete",
            VAULT,
            "attachment_delete",
            Write,
            "Delete a file attached to a vault item",
            "Removes one attachment from an item for good. The user approves on their phone.",
            vec![
                str_p("item", 100, true, ITEM_HELP),
                str_p("attachment", 300, true, "The attachment: its id or exact file name, from vault_item_view."),
            ],
            Some("item"),
        )
        .in_class("items"),
        tool(
            "vault_item_favorite",
            VAULT,
            "item_favorite",
            Write,
            "Favorite or unfavorite a vault item",
            "Marks an item as a favorite, or removes the mark. The user approves on their phone unless a standing \
             permission covers it.",
            vec![str_p("item", 100, true, ITEM_HELP), bool_p("favorite", "false removes the mark. Default: true.")],
            Some("item"),
        )
        .in_class("organize"),
        tool(
            "vault_item_move",
            VAULT,
            "item_move",
            Write,
            "Move a vault item to a folder",
            "Moves one item to a folder, or out of all folders with `none`.",
            vec![
                str_p("item", 100, true, ITEM_HELP),
                str_p("folder", 100, true, "The target folder: its id or exact name, or `none`."),
            ],
            Some("item"),
        )
        .in_class("organize"),
        tool(
            "vault_item_trash",
            VAULT,
            "item_trash",
            Write,
            "Move a vault item to the trash",
            "Moves one item to the trash, from where the user (or vault_item_restore) can get it back.",
            vec![str_p("item", 100, true, ITEM_HELP)],
            Some("item"),
        )
        .in_class("organize"),
        tool(
            "vault_item_restore",
            VAULT,
            "item_restore",
            Write,
            "Restore a vault item from the trash",
            "Takes one item out of the trash.",
            vec![str_p("item", 100, true, "The id of the item in the trash, from vault_search with state=trash.")],
            Some("item"),
        )
        .in_class("organize"),
        tool(
            "vault_item_archive",
            VAULT,
            "item_archive",
            Write,
            "Archive a vault item",
            "Moves one item to the archive, out of the everyday lists.",
            vec![str_p("item", 100, true, ITEM_HELP)],
            Some("item"),
        )
        .in_class("organize"),
        tool(
            "vault_item_unarchive",
            VAULT,
            "item_unarchive",
            Write,
            "Unarchive a vault item",
            "Takes one item out of the archive.",
            vec![str_p("item", 100, true, "The id of the archived item, from vault_search with state=archived.")],
            Some("item"),
        )
        .in_class("organize"),
        tool(
            "vault_item_delete",
            VAULT,
            "item_delete",
            Write,
            "Delete a vault item permanently",
            "Deletes one item for good, with its attachments; it cannot be brought back. The user must approve on \
             their phone every time; no standing permission covers this.",
            vec![str_p("item", 100, true, ITEM_HELP)],
            Some("item"),
        )
        .in_class("organize")
        .once(),
        tool(
            "vault_trash_empty",
            VAULT,
            "trash_empty",
            Write,
            "Empty the vault trash",
            "Permanently deletes every item in the trash. The user must approve on their phone every time and sees \
             how many items go; no standing permission covers this.",
            vec![],
            None,
        )
        .in_class("organize")
        .once(),
        tool(
            "vault_folder_create",
            VAULT,
            "folder_create",
            Write,
            "Create a vault folder",
            "Adds a folder. The user approves on their phone unless a standing permission covers it.",
            vec![str_p("name", 100, true, "The folder's name.")],
            None,
        )
        .in_class("organize"),
        tool(
            "vault_folder_rename",
            VAULT,
            "folder_rename",
            Write,
            "Rename a vault folder",
            "Gives a folder a new name.",
            vec![
                str_p("folder", 100, true, "The folder: its id or exact name, from vault_folders_list."),
                str_p("name", 100, true, "The new name."),
            ],
            Some("folder"),
        )
        .in_class("organize"),
        tool(
            "vault_folder_delete",
            VAULT,
            "folder_delete",
            Write,
            "Delete a vault folder",
            "Deletes a folder. The items in it are kept and end up without a folder.",
            vec![str_p("folder", 100, true, "The folder: its id or exact name, from vault_folders_list.")],
            Some("folder"),
        )
        .in_class("organize"),
        tool(
            "vault_item_clone",
            VAULT,
            "item_clone",
            Write,
            "Copy a vault item",
            "Makes a new item from an existing one, with a new name (default: the old name with ' - Clone'). The \
             secrets are copied on the phone and never shown or sent to you; attachments and old passwords are not \
             copied. The user approves on their phone.",
            vec![
                str_p("item", 100, true, "The id of the item to copy, from vault_search."),
                str_p("name", 300, false, "The new item's name."),
                str_p(
                    "folder",
                    100,
                    false,
                    "Put the copy in this folder (id or exact name, or `none`). Default: the same.",
                ),
            ],
            Some("item"),
        )
        .in_class("items"),
    ]
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::connector::vault::CLASSES;

    #[test]
    fn writes_have_a_known_class_and_only_permanent_deletions_are_once_only() {
        let all = tools();
        assert_eq!(all.len(), 21);
        for spec in &all {
            assert!(
                spec.tool.starts_with("vault_") && spec.tool.trim_start_matches("vault_") == spec.op,
                "{}",
                spec.tool
            );
            if spec.effect == Effect::Write {
                assert!(CLASSES.iter().any(|c| c.id == spec.class), "{} has class {:?}", spec.tool, spec.class);
                assert_eq!(spec.once_only, matches!(spec.op, "item_delete" | "trash_empty"), "{}", spec.tool);
            } else {
                assert!(spec.class.is_empty() && !spec.once_only, "{}", spec.tool);
            }
            assert!(spec.params.iter().all(|p| !p.description.is_empty()), "{}", spec.tool);
        }
    }

    #[test]
    fn the_legacy_tools_keep_their_shape_and_the_new_arguments_are_checked() {
        let all = tools();
        let find = |op: &str| all.iter().find(|t| t.op == op).unwrap();
        let (call, _) = find("search").parse(&json!({"query": "git"})).unwrap();
        assert_eq!((call.str_arg("query"), call.int_arg("limit")), (Some("git"), Some(20)));
        assert!(find("search").parse(&json!({"state": "deleted"})).is_err());
        assert!(find("get").parse(&json!({"item": "a", "field": "password"})).is_ok());
        assert!(find("get").parse(&json!({"item": "a", "field": "card_number", "custom_field": "x"})).is_ok());
        assert!(find("get").parse(&json!({"item": "a"})).unwrap_err().contains("`field` is required"));
        let create = find("item_create");
        assert!(create.parse(&json!({"type": "login", "name": "x", "fields": {"password": "p"}})).is_ok());
        assert!(create.parse(&json!({"type": "login", "name": "x", "fields": "x".repeat(20_001)})).is_err());
        // Without content the phone answers with an upload link; `blob` names an uploaded file.
        assert!(find("attachment_add").parse(&json!({"item": "a", "file_name": "f"})).is_ok());
        assert!(find("attachment_add").parse(&json!({"item": "a", "file_name": "f", "blob": "b".repeat(20)})).is_ok());
    }
}
