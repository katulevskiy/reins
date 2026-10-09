//! The Gmail tools besides search, read and send (which are `ToolCall`s of their own): organizing messages, Trash,
//! labels, drafts, attachments, filters and the vacation reply. A permission for a write names the kinds of change it
//! allows (`CLASSES`).

use super::Effect::{List, Read, Write};
use super::{ClassInfo, GMAIL, Kind, Param, ToolSpec, bool_p, choice_p, list_p, str_p, text_p, tool};

/// The kinds of change a Gmail write falls into, for permissions.
pub(super) const CLASSES: &[ClassInfo] = &[
    ClassInfo {
        id: "organize",
        label: "Archive, labels, read, star and spam",
    },
    ClassInfo {
        id: "trash",
        label: "Move to Trash and back",
    },
    ClassInfo {
        id: "labels",
        label: "Create, rename and delete labels",
    },
    ClassInfo {
        id: "drafts",
        label: "Drafts",
    },
    ClassInfo {
        id: "settings",
        label: "Filters and vacation reply",
    },
];

/// Messages one organizing call may change (Gmail's batch limit is 1,000).
pub const MAX_ORGANIZE_IDS: usize = 500;
/// Messages one Trash call may move (each is a request of its own).
pub const MAX_TRASH_IDS: usize = 100;
/// The largest file a draft takes (Gmail takes the whole message as JSON).
pub const MAX_DRAFT_FILE_BYTES: u64 = 4 * 1024 * 1024;

/// What `gmail_organize` can do to messages.
pub const ORGANIZE_ACTIONS: &[&str] = &[
    "archive",
    "move_to_inbox",
    "mark_read",
    "mark_unread",
    "star",
    "unstar",
    "mark_important",
    "mark_not_important",
    "spam",
    "not_spam",
    "add_label",
    "remove_label",
];

const fn message_ids(max_items: usize) -> Param {
    Param {
        name: "message_ids",
        kind: Kind::List {
            max_items,
            max_len: 64,
        },
        required: true,
        description: "Gmail message ids, from gmail_search.",
    }
}

const LABEL_HELP: &str =
    "A label: its name as Gmail shows it (Receipts, Work/Clients) or its id from gmail_list_labels.";

pub(super) fn tools() -> Vec<ToolSpec> {
    vec![
        tool(
            "gmail_organize",
            GMAIL,
            "organize",
            Write,
            "Organize Gmail messages",
            "Archives, labels, stars, marks read or unread, or moves to Spam (and back) the messages you name; get \
             their ids with gmail_search. Nothing is deleted. The user sees the action, how many messages and \
             examples on their phone and approves, unless they allowed organizing their mail for a while. To undo, \
             call it again with the opposite action (archive ↔ move_to_inbox, add_label ↔ remove_label, spam ↔ \
             not_spam).",
            vec![
                message_ids(MAX_ORGANIZE_IDS),
                choice_p("action", ORGANIZE_ACTIONS, true, "What to do to every message."),
                str_p("label", 225, false, "For add_label and remove_label: the label (it must exist)."),
            ],
            None,
        )
        .in_class("organize"),
        tool(
            "gmail_trash",
            GMAIL,
            "trash",
            Write,
            "Move Gmail messages to Trash",
            "Moves the messages you name to Trash, where Gmail keeps them for 30 days, or (`restore`) back out of \
             it. Messages are never deleted for good. The user approves on their phone.",
            vec![
                message_ids(MAX_TRASH_IDS),
                choice_p("action", &["trash", "restore"], false, "trash (the default) or restore."),
            ],
            None,
        )
        .in_class("trash"),
        tool(
            "gmail_list_labels",
            GMAIL,
            "list_labels",
            List,
            "List Gmail labels",
            "Lists the labels of the user's Gmail (name, id, and whether Gmail or the user made it).",
            vec![],
            None,
        ),
        tool(
            "gmail_create_label",
            GMAIL,
            "create_label",
            Write,
            "Create a Gmail label",
            "Creates a label. Nest it with a slash (Work/Clients). The user approves on their phone.",
            vec![str_p("name", 225, true, "The label's name.")],
            None,
        )
        .in_class("labels"),
        tool(
            "gmail_rename_label",
            GMAIL,
            "rename_label",
            Write,
            "Rename a Gmail label",
            "Renames one of the user's labels. The user approves on their phone.",
            vec![str_p("label", 225, true, LABEL_HELP), str_p("new_name", 225, true, "The new name.")],
            None,
        )
        .in_class("labels"),
        tool(
            "gmail_delete_label",
            GMAIL,
            "delete_label",
            Write,
            "Delete a Gmail label",
            "Deletes one of the user's labels; the messages keep everything else. Asked for every time.",
            vec![str_p("label", 225, true, LABEL_HELP)],
            None,
        )
        .in_class("labels")
        .once(),
        tool(
            "gmail_create_draft",
            GMAIL,
            "create_draft",
            Write,
            "Save a Gmail draft",
            "Saves an email as a draft in the user's Gmail, for them to review and send from Gmail themselves; \
             nothing is sent. It can carry one file of up to 4 MB: give `file_name` with `content_base64` for a small \
             file (under about 700 KB), or `file_name` alone to get an upload link. The user approves on their phone.",
            vec![
                list_p("to", 50, 254, "Recipient email addresses."),
                list_p("cc", 50, 254, "Cc email addresses."),
                str_p("subject", 998, true, "Subject line."),
                text_p("body", 100_000, true, "Plain-text body."),
                str_p("reply_to_message_id", 64, false, "Gmail message id this replies to, to keep the thread."),
                str_p("file_name", 200, false, "The attached file's name, without a path."),
                text_p("content_base64", 1_000_000, false, "The attached file, base64-encoded (small files)."),
                str_p("blob", 64, false, "A file uploaded to the link the phone gave, instead of content_base64."),
            ],
            None,
        )
        .in_class("drafts"),
        tool(
            "gmail_list_attachments",
            GMAIL,
            "list_attachments",
            List,
            "List a Gmail message's attachments",
            "Lists the files attached to one message (name, type, size and `part`); fetch one with \
             gmail_get_attachment.",
            vec![str_p("message_id", 64, true, "The message id, from gmail_search.")],
            None,
        ),
        tool(
            "gmail_get_attachment",
            GMAIL,
            "get_attachment",
            Read,
            "Download a Gmail attachment",
            "Downloads one attached file, base64-encoded (large files come as a download link). The user sees its \
             name and size and approves on their phone.",
            vec![
                str_p("message_id", 64, true, "The message id."),
                str_p("part", 32, true, "The attachment's `part`, from gmail_list_attachments."),
            ],
            None,
        ),
        tool(
            "gmail_list_filters",
            GMAIL,
            "list_filters",
            List,
            "List Gmail filters",
            "Lists the user's Gmail filters: what each matches and what it does.",
            vec![],
            None,
        ),
        tool(
            "gmail_create_filter",
            GMAIL,
            "create_filter",
            Write,
            "Create a Gmail filter",
            "Creates a filter Gmail applies to new mail. Give at least one condition (from, to, subject, query, \
             has_attachment) and one action. Filters never forward mail. Asked for every time.",
            vec![
                str_p("from", 254, false, "Mail from this address or domain."),
                str_p("to", 254, false, "Mail to this address."),
                str_p("subject", 255, false, "Words in the subject."),
                str_p("query", 1_000, false, "Any Gmail search, for everything else."),
                bool_p("has_attachment", "Only mail with attachments."),
                bool_p("skip_inbox", "Archive it (skip the Inbox)."),
                bool_p("mark_read", "Mark it read."),
                bool_p("star", "Star it."),
                bool_p("important", "Mark it important."),
                bool_p("never_spam", "Never send it to Spam."),
                bool_p("trash", "Move it to Trash."),
                str_p("add_label", 225, false, "Apply this label (it must exist)."),
            ],
            None,
        )
        .in_class("settings")
        .once(),
        tool(
            "gmail_delete_filter",
            GMAIL,
            "delete_filter",
            Write,
            "Delete a Gmail filter",
            "Deletes one filter. Asked for every time.",
            vec![str_p("filter_id", 100, true, "The filter's id, from gmail_list_filters.")],
            None,
        )
        .in_class("settings")
        .once(),
        tool(
            "gmail_get_vacation",
            GMAIL,
            "get_vacation",
            Read,
            "Read the Gmail vacation reply",
            "Reads the user's vacation reply: whether it is on, its text and dates.",
            vec![],
            None,
        ),
        tool(
            "gmail_set_vacation",
            GMAIL,
            "set_vacation",
            Write,
            "Set the Gmail vacation reply",
            "Turns the vacation reply on (with its subject, text and optional dates) or off. Asked for every time.",
            vec![
                Param {
                    name: "enabled",
                    kind: Kind::Bool,
                    required: true,
                    description: "On or off.",
                },
                str_p("subject", 255, false, "The reply's subject."),
                text_p("body", 10_000, false, "The reply's text (required to turn it on)."),
                str_p("start", 40, false, "When it starts (a date, or a date and time with offset); default now."),
                str_p("end", 40, false, "When it ends; default until turned off."),
                bool_p("contacts_only", "Reply only to people in the user's contacts."),
            ],
            None,
        )
        .in_class("settings")
        .once(),
    ]
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::super::{classes, spec_for_tool};
    use super::*;

    #[test]
    fn every_write_has_a_known_class_and_no_read_has_one() {
        let known: Vec<&str> = classes(GMAIL).iter().map(|c| c.id).collect();
        for spec in tools() {
            assert!(spec.tool.starts_with("gmail_"), "{}: the server lists gmail_* tools under Gmail", spec.tool);
            if spec.effect == Write {
                assert!(known.contains(&spec.class), "{} has no known class", spec.tool);
            } else {
                assert!(spec.class.is_empty(), "{} reads but has a class", spec.tool);
            }
        }
    }

    #[test]
    fn settings_and_label_deletion_are_asked_every_time() {
        for name in ["gmail_create_filter", "gmail_delete_filter", "gmail_set_vacation", "gmail_delete_label"] {
            assert!(spec_for_tool(name).unwrap().once_only, "{name}");
        }
        for name in ["gmail_organize", "gmail_trash", "gmail_create_draft", "gmail_create_label"] {
            assert!(!spec_for_tool(name).unwrap().once_only, "{name}");
        }
    }

    #[test]
    fn arguments_are_checked() {
        let organize = spec_for_tool("gmail_organize").unwrap();
        let (call, _) = organize.parse(&json!({"message_ids": ["a1"], "action": "ARCHIVE"})).unwrap();
        assert_eq!(call.str_arg("action"), Some("archive"));
        assert!(organize.parse(&json!({"message_ids": ["a1"], "action": "delete"})).is_err());
        assert!(organize.parse(&json!({"action": "archive"})).unwrap_err().contains("message_ids"));
        let too_many: Vec<String> = (0..=MAX_ORGANIZE_IDS).map(|i| format!("m{i}")).collect();
        assert!(organize.parse(&json!({"message_ids": too_many, "action": "archive"})).is_err());
        let filter = spec_for_tool("gmail_create_filter").unwrap();
        assert!(filter.parse(&json!({"from": "a@b.com", "forward": "eve@evil.com"})).unwrap_err().contains("Unknown"));
        let vacation = spec_for_tool("gmail_set_vacation").unwrap();
        assert!(vacation.parse(&json!({})).unwrap_err().contains("`enabled` is required"));
    }
}
