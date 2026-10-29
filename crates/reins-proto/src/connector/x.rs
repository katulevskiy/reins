//! X (formerly Twitter), through the official X API v2 only (<https://docs.x.com/x-api/introduction>).
//!
//! Every action names the OAuth 2.0 scopes it needs and its risk. Writes are shown on the phone exactly as they would
//! be sent, and approved there.

use super::{
    Effect::{List, Read, Search, Write},
    RiskLevel, ToolSpec, X, int_p, list_p, str_p, text_p, tool,
};

/// Every scope the connector asks for when an account is added
/// (<https://docs.x.com/resources/fundamentals/authentication/oauth-2-0/authorization-code>).
pub const SCOPES: &[&str] = &[
    "tweet.read",
    "tweet.write",
    "users.read",
    "follows.read",
    "follows.write",
    "like.read",
    "like.write",
    "mute.read",
    "mute.write",
    "block.read",
    "block.write",
    "bookmark.read",
    "bookmark.write",
    "dm.read",
    "dm.write",
    "media.write",
    "offline.access",
];

/// Longest post text the API accepts from an account without X Premium (longer posts need Premium).
pub const MAX_POST_CHARS: usize = 280;
/// Longest text this connector sends in one post (accounts with X Premium may post up to 25 000 characters).
const MAX_TEXT: usize = 25_000;
/// Media attached to one post: up to 4 photos, or 1 GIF, or 1 video (<https://docs.x.com/x-api/media/introduction>).
pub const MAX_MEDIA: usize = 4;
/// One media file as base64, up to 5 MB of image (<https://docs.x.com/x-api/media/introduction>).
pub const MAX_MEDIA_BASE64: usize = 7_000_000;
/// Posts in one thread.
pub const MAX_THREAD: usize = 25;

const R: &[&str] = &["tweet.read", "users.read"];
const LIMIT: super::Param = int_p("limit", 5, 100, Some(20), "How many results at most (5 to 100).");
const POST: super::Param = str_p("post", 30, true, "The post's id (the number at the end of its x.com link).");
const USER: super::Param = str_p("user", 50, true, "The account: @username or its numeric id.");
const MEDIA: super::Param =
    list_p("media", MAX_MEDIA, 30, "Media ids from x_media_upload: up to 4 photos, or one GIF.");

pub(super) fn tools() -> Vec<ToolSpec> {
    vec![
        // Reads.
        tool(
            "x_me",
            X,
            "me",
            Read,
            "Show the X account",
            "Show the connected X account: its id, @username, name and profile.",
            vec![],
            None,
        )
        .scoped(R),
        tool(
            "x_user",
            X,
            "user",
            Read,
            "Look up an X account",
            "Look up an X account's profile by @username or id.",
            vec![USER],
            Some("user"),
        )
        .scoped(R),
        tool("x_post_get", X, "post_get", Read, "Read an X post", "Read one post by its id.", vec![POST], Some("post"))
            .scoped(R),
        tool(
            "x_home_timeline",
            X,
            "home_timeline",
            List,
            "Read the X home timeline",
            "The latest posts from the accounts the user follows, and the user's own, newest first.",
            vec![LIMIT],
            None,
        )
        .scoped(R),
        tool(
            "x_mentions",
            X,
            "mentions",
            List,
            "Read X mentions",
            "The latest posts that mention the user, newest first.",
            vec![LIMIT],
            None,
        )
        .scoped(R),
        tool(
            "x_search",
            X,
            "search",
            Search,
            "Search X",
            "Search posts from the last 7 days (X's recent search; the query syntax is X's own, up to 512 characters).",
            vec![str_p("query", 512, true, "The search query, e.g. `from:xdevelopers -is:retweet`."), LIMIT],
            None,
        )
        .scoped(R),
        tool(
            "x_bookmarks",
            X,
            "bookmarks",
            List,
            "List X bookmarks",
            "The user's bookmarked posts, newest first.",
            vec![LIMIT],
            None,
        )
        .scoped(&["bookmark.read", "tweet.read", "users.read"]),
        tool(
            "x_dm_list",
            X,
            "dm_list",
            Read,
            "Read X direct messages",
            "The latest direct messages, newest first: with one account when `user` is given, else from every \
             conversation. Each message is shown to the user before it is shared.",
            vec![str_p("user", 50, false, "Only the conversation with this account: @username or id."), LIMIT],
            None,
        )
        .scoped(&["dm.read", "tweet.read", "users.read"]),
        // Writes: each is shown to the user exactly as it would be sent, and approved on the phone.
        tool(
            "x_media_upload",
            X,
            "media_upload",
            Write,
            "Upload media to X",
            "Upload a photo or GIF for a post or a direct message; returns its media id. Nothing is published until \
             a post or a message uses it.",
            vec![
                text_p("data", MAX_MEDIA_BASE64, true, "The file, base64."),
                super::choice_p(
                    "media_type",
                    &["image/jpeg", "image/png", "image/webp", "image/gif"],
                    true,
                    "The file's type.",
                ),
                super::choice_p(
                    "for",
                    &["post", "dm"],
                    false,
                    "What it is for: a post (the default) or a direct message.",
                ),
            ],
            None,
        )
        .scoped(&["media.write", "tweet.write", "users.read"])
        .risk(RiskLevel::Medium),
        tool(
            "x_post",
            X,
            "post",
            Write,
            "Post on X",
            "Publish a post from the user's X account, optionally with media or quoting another post. Quoting \
             (`quote`) needs X's Enterprise plan.",
            vec![
                text_p("text", MAX_TEXT, false, "The post's text (280 characters unless the account has X Premium)."),
                MEDIA,
                str_p("quote", 30, false, "Quote this post (its id). Needs X's Enterprise plan."),
            ],
            None,
        )
        .scoped(&["tweet.write", "tweet.read", "users.read"])
        .risk(RiskLevel::High),
        tool(
            "x_reply",
            X,
            "reply",
            Write,
            "Reply on X",
            "Publish a reply to a post from the user's X account.",
            vec![
                str_p("reply_to", 30, true, "The post to reply to (its id)."),
                text_p("text", MAX_TEXT, true, "The reply's text."),
                MEDIA,
            ],
            Some("reply_to"),
        )
        .scoped(&["tweet.write", "tweet.read", "users.read"])
        .risk(RiskLevel::High),
        tool(
            "x_thread",
            X,
            "thread",
            Write,
            "Post an X thread",
            "Publish a thread: the first post, then each next one as a reply to the one before.",
            vec![list_p("posts", MAX_THREAD, MAX_TEXT, "The posts' texts, in order (2 to 25).")],
            None,
        )
        .scoped(&["tweet.write", "tweet.read", "users.read"])
        .risk(RiskLevel::High),
        tool(
            "x_post_delete",
            X,
            "post_delete",
            Write,
            "Delete an X post",
            "Delete one of the user's own posts for good.",
            vec![POST],
            Some("post"),
        )
        .scoped(&["tweet.write", "tweet.read", "users.read"])
        .risk(RiskLevel::High),
        tool("x_like", X, "like", Write, "Like an X post", "Like a post.", vec![POST], Some("post")).scoped(&[
            "like.write",
            "tweet.read",
            "users.read",
        ]),
        tool("x_unlike", X, "unlike", Write, "Unlike an X post", "Take back a like.", vec![POST], Some("post"))
            .scoped(&["like.write", "tweet.read", "users.read"]),
        tool(
            "x_repost",
            X,
            "repost",
            Write,
            "Repost on X",
            "Repost a post to the user's followers.",
            vec![POST],
            Some("post"),
        )
        .scoped(&["tweet.write", "tweet.read", "users.read"]),
        tool("x_unrepost", X, "unrepost", Write, "Undo an X repost", "Undo a repost.", vec![POST], Some("post"))
            .scoped(&["tweet.write", "tweet.read", "users.read"]),
        tool(
            "x_bookmark_add",
            X,
            "bookmark_add",
            Write,
            "Bookmark an X post",
            "Add a post to the user's bookmarks (only the user sees them).",
            vec![POST],
            Some("post"),
        )
        .scoped(&["bookmark.write", "tweet.read", "users.read"]),
        tool(
            "x_bookmark_remove",
            X,
            "bookmark_remove",
            Write,
            "Remove an X bookmark",
            "Remove a post from the user's bookmarks.",
            vec![POST],
            Some("post"),
        )
        .scoped(&["bookmark.write", "tweet.read", "users.read"])
        .risk(RiskLevel::Medium),
        tool("x_follow", X, "follow", Write, "Follow on X", "Follow an account.", vec![USER], Some("user")).scoped(&[
            "follows.write",
            "tweet.read",
            "users.read",
        ]),
        tool("x_unfollow", X, "unfollow", Write, "Unfollow on X", "Unfollow an account.", vec![USER], Some("user"))
            .scoped(&["follows.write", "tweet.read", "users.read"]),
        tool("x_mute", X, "mute", Write, "Mute on X", "Mute an account.", vec![USER], Some("user")).scoped(&[
            "mute.write",
            "tweet.read",
            "users.read",
        ]),
        tool("x_unmute", X, "unmute", Write, "Unmute on X", "Unmute an account.", vec![USER], Some("user")).scoped(&[
            "mute.write",
            "tweet.read",
            "users.read",
        ]),
        tool(
            "x_block",
            X,
            "block",
            Write,
            "Block on X",
            "Block an account. Needs X's Enterprise plan.",
            vec![USER],
            Some("user"),
        )
        .scoped(&["block.write", "tweet.read", "users.read"])
        .risk(RiskLevel::High),
        tool(
            "x_unblock",
            X,
            "unblock",
            Write,
            "Unblock on X",
            "Unblock an account. Needs X's Enterprise plan.",
            vec![USER],
            Some("user"),
        )
        .scoped(&["block.write", "tweet.read", "users.read"]),
        tool(
            "x_dm_send",
            X,
            "dm_send",
            Write,
            "Send an X direct message",
            "Send a direct message from the user's X account to one account.",
            vec![
                USER,
                text_p("text", 10_000, false, "The message."),
                list_p("media", 1, 30, "A media id from x_media_upload with `for` = dm (one at most)."),
            ],
            Some("user"),
        )
        .scoped(&["dm.write", "dm.read", "tweet.read", "users.read"])
        .risk(RiskLevel::High),
    ]
}
