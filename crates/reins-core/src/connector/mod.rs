//! Integrations (and Gmail beyond its search, read and send). Each one turns the validated call of a tool into a list of [`Item`]s (to read, list or
//! search) or into a [`Preview`] and then an action (to write); the engine does the rest the same way for all of them:
//! it checks the grants, asks the user, records the activity and answers the AI.

pub mod calendar;
pub mod desktop;
pub mod device;
pub mod flow;
pub(crate) mod git;
pub mod githost;
pub mod github;
pub mod gmail;
pub(crate) mod sealed;
pub mod telegram;
pub mod telegram_client;
pub mod vault;

use std::collections::BTreeMap;
use std::sync::Arc;

use reins_proto::connector::{ConnectorCall, Effect};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::types::GmailStatus;
use crate::{CoreError, text};

/// What is kept of one item's text in the parked request and in the activity (so the details can show it).
pub const MAX_ITEM_TEXT: usize = 4_000;

/// One thing an integration returned: a chat, a message, an event, an issue, a contact.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Item {
    /// Unique among the items of one call.
    pub id: String,
    /// The id of the chat, calendar or repository it belongs to (grants cover resources). For a listing, its own id.
    pub resource: String,
    pub resource_label: String,
    pub from: String,
    pub title: String,
    /// A line of it, for lists.
    pub snippet: String,
    /// Unix seconds; 0 when it has none.
    pub date: i64,
    /// The full text, when the call fetches content.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    /// Must never be shared without the user ticking it: not covered by grants, unticked by default.
    #[serde(default)]
    pub sensitive: bool,
    /// `body` is a secret (a password): it is handed to the AI once released, but never shown in the approval or
    /// kept in the activity; the user sees the snippet, which says what it is.
    #[serde(default)]
    pub secret: bool,
    /// More fields handed to the AI as they are (a location, a link, a number).
    #[serde(default)]
    pub extra: Map<String, Value>,
    /// The wider things `resource` belongs to, nearest first, each with its name; see [`Preview::parents`].
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parents: Vec<(String, String)>,
}

impl Item {
    /// The text shown to the user and kept in the activity.
    pub fn text(&self) -> String {
        let shown = if self.secret {
            &self.snippet
        } else {
            self.body.as_deref().unwrap_or(&self.snippet)
        };
        text::truncate_chars(&text::neutralize(shown), MAX_ITEM_TEXT)
    }
}

/// What a write would do, spelled out for the user.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Preview {
    /// The chat, calendar or repository it happens in (grants cover it).
    pub resource: String,
    pub resource_label: String,
    /// One line each: the recipient and the text, the event and when.
    pub lines: Vec<String>,
    /// The wider things `resource` belongs to, nearest first, each with its name (`owner/repo@main` is in `owner/repo`,
    /// which is in `owner`). A standing permission can be given for any of them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parents: Vec<(String, String)>,
    /// Asked for every time, whatever the tool says: this particular change is too far-reaching to remember.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub once_only: bool,
    /// The kind of change this particular call is, when the tool alone does not say (`github_api_write`): used instead
    /// of the tool's class for permissions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub class: Option<String>,
    /// The uploaded file the write uses (`blob=<id>`), as the server described it; shown with the preview.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blob: Option<reins_proto::blob::BlobInfo>,
}

#[async_trait::async_trait]
pub trait Connector: Send + Sync {
    fn service(&self) -> &'static str;

    /// Lists, reads or searches. Nothing is changed.
    async fn fetch(&self, _account: &str, _call: &ConnectorCall) -> Result<Vec<Item>, CoreError> {
        Err(CoreError::service("this integration cannot do that"))
    }

    /// What a write would do, without doing it.
    async fn preview(&self, _account: &str, _call: &ConnectorCall) -> Result<Preview, CoreError> {
        Err(CoreError::service("this integration cannot do that"))
    }

    /// Does the write the user approved and says what happened.
    async fn perform(&self, _account: &str, _call: &ConnectorCall) -> Result<Value, CoreError> {
        Err(CoreError::service("this integration cannot do that"))
    }

    /// The headers a download from this integration needs (its token), so that the server can fetch a large result in
    /// the phone's name, for that one request (see [`crate::blob`]). Never logged or kept.
    async fn fetch_headers(&self, _account: &str) -> Result<Vec<(String, String)>, CoreError> {
        Err(CoreError::service("this integration cannot hand files over as links"))
    }

    /// Several accounts are connected and the caller cannot name one (git on the user's computer): the account that
    /// can do this call. `None` when the integration cannot tell; an error when none of them can.
    async fn choose_account(&self, _accounts: &[String], _call: &ConnectorCall) -> Result<Option<String>, CoreError> {
        Ok(None)
    }

    /// Whether the account works right now.
    async fn status(&self, account: &str) -> GmailStatus;

    /// Why this integration cannot be used in this build, when it cannot (a missing credential).
    fn unavailable(&self) -> Option<String> {
        None
    }

    /// The account is being removed: sign out, delete secrets.
    async fn forget(&self, _account: &str) {}

    /// A Google account the phone just authorized: the address it really is (checked with the service).
    async fn identify(&self, _hint: &str) -> Result<String, CoreError> {
        Err(CoreError::invalid("this integration is not added that way"))
    }

    /// A pasted access token: checks it, keeps it, and returns the account it belongs to.
    async fn sign_in_token(&self, _token: &str) -> Result<String, CoreError> {
        Err(CoreError::invalid("this integration is not added that way"))
    }

    /// A phone-number sign-in, step 1: ask for the login code.
    async fn login_begin(&self, _phone: &str) -> Result<(), CoreError> {
        Err(CoreError::invalid("this integration is not added that way"))
    }

    /// Step 2: the code that arrived.
    async fn login_code(&self, _code: &str) -> Result<LoginProgress, CoreError> {
        Err(CoreError::invalid("this integration is not added that way"))
    }

    /// Step 3, when the account has two-step verification: its password. Returns the account.
    async fn login_password(&self, _password: &str) -> Result<String, CoreError> {
        Err(CoreError::invalid("this integration is not added that way"))
    }
}

/// Where a sign-in stands.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum LoginProgress {
    /// The account has two-step verification: its password is needed.
    NeedsPassword {
        hint: Option<String>,
    },
    /// Signed in as this account.
    Done {
        account: String,
    },
}

/// The integrations this phone can run, by service id.
#[derive(Default)]
pub struct Registry {
    map: BTreeMap<&'static str, Arc<dyn Connector>>,
}

impl Registry {
    pub fn add(&mut self, connector: Arc<dyn Connector>) {
        self.map.insert(connector.service(), connector);
    }

    pub fn get(&self, service: &str) -> Option<Arc<dyn Connector>> {
        self.map.get(service).cloned()
    }

    pub fn services(&self) -> Vec<&'static str> {
        self.map.keys().copied().collect()
    }
}

/// Whether a text looks like a login or verification code, which must never be handed over without the user ticking it.
pub fn looks_like_code(text: &str) -> bool {
    const WORDS: [&str; 13] = [
        "code",
        "otp",
        "verification",
        "verify",
        "passcode",
        "one-time",
        "one time",
        "login",
        "log in",
        "sign in",
        "password",
        "pin",
        "2fa",
    ];
    let lower = text.to_lowercase();
    let has_word = WORDS.iter().any(|w| {
        // "pin" and "2fa" only as whole words: "spinning" is not a PIN.
        if matches!(*w, "pin" | "otp" | "2fa") {
            lower.split(|c: char| !c.is_alphanumeric()).any(|word| word == *w)
        } else {
            lower.contains(w)
        }
    });
    if !has_word {
        return false;
    }
    // A run of 4 to 10 digits, possibly written in groups ("123 456", "12-34-56").
    let mut digits = 0usize;
    let mut best = 0usize;
    let mut pending_sep = false;
    for c in lower.chars() {
        if c.is_ascii_digit() {
            digits += 1;
            pending_sep = false;
            best = best.max(digits);
        } else if (c == ' ' || c == '-') && digits > 0 && !pending_sep {
            pending_sep = true;
        } else {
            digits = 0;
            pending_sep = false;
        }
    }
    (4..=10).contains(&best)
}

/// The JSON handed to the AI for released items.
pub fn items_json(effect: Effect, items: &[&Item]) -> Value {
    let rendered: Vec<Value> = items
        .iter()
        .map(|item| {
            let mut o = Map::new();
            o.insert("id".to_owned(), json!(item.id));
            if effect != Effect::List && !item.resource.is_empty() {
                o.insert("in".to_owned(), json!({"id": item.resource, "name": item.resource_label}));
            }
            for (key, value) in [("from", &item.from), ("title", &item.title)] {
                if !value.is_empty() {
                    o.insert(key.to_owned(), json!(value));
                }
            }
            let text = if effect == Effect::Read {
                item.body.as_deref().unwrap_or(&item.snippet)
            } else {
                &item.snippet
            };
            if !text.is_empty() {
                o.insert("text".to_owned(), json!(text::neutralize(text)));
            }
            if item.date != 0 {
                o.insert("date".to_owned(), json!(text::iso_utc(item.date)));
            }
            for (key, value) in &item.extra {
                o.entry(key.clone()).or_insert_with(|| value.clone());
            }
            Value::Object(o)
        })
        .collect();
    json!({ "items": rendered })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn released_items_carry_what_the_ai_needs_and_nothing_empty() {
        let item = Item {
            id: "9:1".to_owned(),
            resource: "9".to_owned(),
            resource_label: "Family".to_owned(),
            from: "Anna".to_owned(),
            snippet: "hi".to_owned(),
            body: Some("hi there\u{202e}".to_owned()),
            date: 1_700_000_000,
            extra: json!({"url": "https://x"}).as_object().unwrap().clone(),
            ..Item::default()
        };
        let read = items_json(Effect::Read, &[&item]);
        assert_eq!(
            read["items"][0],
            json!({"id": "9:1", "in": {"id": "9", "name": "Family"}, "from": "Anna", "text": "hi there",
                   "date": "2023-11-14T22:13:20Z", "url": "https://x"})
        );
        let search = items_json(Effect::Search, &[&item]);
        assert_eq!(search["items"][0]["text"], "hi", "a search shows the snippet, a read the whole text");
        let list = items_json(
            Effect::List,
            &[&Item {
                id: "9".into(),
                title: "Family".into(),
                ..Item::default()
            }],
        );
        assert_eq!(list["items"][0], json!({"id": "9", "title": "Family"}));
    }

    #[test]
    fn login_codes_are_recognised_and_ordinary_numbers_are_not() {
        for code in [
            "Your code is 481516",
            "Login code: 123-456. Do not share it.",
            "G-123456 is your Google verification code",
            "OTP 8842 valid for 5 minutes",
            "Use 12 34 56 to sign in",
            "Your PIN is 4821",
        ] {
            assert!(looks_like_code(code), "{code}");
        }
        for fine in [
            "Dinner at 8?",
            "The meeting is on 2026-10-05 at 14:00",
            "Call me on 555 0100",
            "verification of the results is pending",
            "we are spinning up 1234 servers",
            "password manager comparison",
            "Order 12345678901234 shipped",
        ] {
            assert!(!looks_like_code(fine), "{fine}");
        }
    }

    #[test]
    fn kept_text_is_bounded_and_safe() {
        let long = Item {
            body: Some("x".repeat(MAX_ITEM_TEXT * 2)),
            ..Item::default()
        };
        assert_eq!(long.text().chars().count(), MAX_ITEM_TEXT);
        assert_eq!(
            Item {
                snippet: "a\u{202e}b".into(),
                ..Item::default()
            }
            .text(),
            "ab"
        );
    }
}
