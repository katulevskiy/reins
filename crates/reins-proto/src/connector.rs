//! The tools of every integration (Telegram, calendars, contacts, SMS, GitHub, the vault, and Gmail besides its search,
//! read and send, which are `ToolCall`s of their own).
//!
//! Each tool is described once, as data: its name, what it does to the integration (list, read, search or write),
//! and its parameters with their limits. The server turns that into the MCP tool list and validates arguments with it;
//! the phone receives the validated call and runs it. Nothing here does IO.

use std::sync::OnceLock;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

pub const GMAIL: &str = "gmail";
pub const TELEGRAM: &str = "telegram";
pub const GCALENDAR: &str = "gcalendar";
pub const GCONTACTS: &str = "gcontacts";
pub const DEVICE_CALENDAR: &str = "device_calendar";
pub const DEVICE_CONTACTS: &str = "device_contacts";
pub const SMS: &str = "sms";
pub const GITHUB: &str = "github";
pub const VAULT: &str = "vault";
/// The desktop app itself (`reins ask`).
pub const DESKTOP: &str = "desktop";
pub const GITLAB: &str = "gitlab";
pub const CODEBERG: &str = "codeberg";
pub const BITBUCKET: &str = "bitbucket";

/// Longest text an argument may carry (messages, comments, event descriptions).
pub const MAX_TEXT_LEN: usize = 8_000;
pub const MAX_ACCOUNT_LEN: usize = 100;

mod desktop;
pub use desktop::{MAX_SESSION_ITEMS, MAX_SESSION_SECS, MIN_SESSION_SECS, SESSION_END_OP, SESSION_OP};
mod github;
pub mod gmail;
mod vault;

/// What a call does to the integration, which decides how it is approved.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Effect {
    /// Lists what exists (chats, calendars, repositories): names only.
    List,
    /// Fetches the content of something.
    Read,
    /// Looks for content.
    Search,
    /// Changes something or sends something out.
    Write,
}

impl Effect {
    /// The kind of access a grant gives: `list`, `read` or `write` (a search is a read).
    #[must_use]
    pub fn access(self) -> &'static str {
        match self {
            Self::List => "list",
            Self::Read | Self::Search => "read",
            Self::Write => "write",
        }
    }
}

/// A validated call to an integration, as relayed to the phone.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnectorCall {
    /// "telegram", "gcalendar", ...
    pub service: String,
    /// "read", "send", "list_events", ...
    pub op: String,
    #[serde(default)]
    pub args: Map<String, Value>,
}

impl ConnectorCall {
    #[must_use]
    pub fn str_arg(&self, name: &str) -> Option<&str> {
        self.args.get(name).and_then(Value::as_str)
    }

    #[must_use]
    pub fn int_arg(&self, name: &str) -> Option<i64> {
        self.args.get(name).and_then(Value::as_i64)
    }

    #[must_use]
    pub fn bool_arg(&self, name: &str) -> Option<bool> {
        self.args.get(name).and_then(Value::as_bool)
    }

    #[must_use]
    pub fn list_arg(&self, name: &str) -> Vec<&str> {
        self.args
            .get(name)
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default()
    }

    /// The tool description this call belongs to (`None` for a call no tool describes).
    #[must_use]
    pub fn spec(&self) -> Option<&'static ToolSpec> {
        specs().iter().find(|s| s.service == self.service && s.op == self.op)
    }

    #[must_use]
    pub fn effect(&self) -> Option<Effect> {
        self.spec().map(|s| s.effect)
    }

    /// The thing the call is about (the chat, the calendar, the repository), when it names one.
    #[must_use]
    pub fn resource(&self) -> Option<String> {
        let name = self.spec()?.resource_param?;
        self.str_arg(name).map(str::to_owned)
    }

    /// Re-checks a call received from the network against its tool description (the phone never trusts a call).
    pub fn normalized(self) -> Result<Self, String> {
        let spec = self.spec().ok_or_else(|| format!("unknown operation {}.{}", self.service, self.op))?;
        spec.parse_args(&Value::Object(self.args)).map(|(call, _)| call)
    }
}

/// The type and limits of one argument.
#[derive(Clone, Copy, Debug)]
pub enum Kind {
    Str {
        max: usize,
    },
    Int {
        min: i64,
        max: i64,
        default: Option<i64>,
    },
    Bool,
    List {
        max_items: usize,
        max_len: usize,
    },
    Choice(&'static [&'static str]),
    /// Free text kept exactly as given (file contents, a description): no trimming, line breaks of any kind allowed.
    Text {
        max: usize,
    },
    /// Names and values, all text (workflow inputs, custom fields).
    Map {
        max_items: usize,
        max_key: usize,
        max_val: usize,
    },
    /// Structured data the operation checks itself (a list of files to commit): any JSON, bounded in size.
    Json {
        max_len: usize,
    },
}

#[derive(Clone, Copy, Debug)]
pub struct Param {
    pub name: &'static str,
    pub kind: Kind,
    pub required: bool,
    pub description: &'static str,
}

/// One MCP tool.
#[derive(Clone, Debug)]
pub struct ToolSpec {
    pub tool: &'static str,
    pub service: &'static str,
    pub op: &'static str,
    pub effect: Effect,
    pub title: &'static str,
    pub description: &'static str,
    pub params: Vec<Param>,
    /// Which argument names the thing the call is about, when one does.
    pub resource_param: Option<&'static str>,
    /// The kind of change a write is (an issue, code, a release, ...), so that a standing permission can name the kinds
    /// it allows. Empty for integrations that have no such kinds and for reads.
    pub class: &'static str,
    /// Asked for every time, whatever the user says: destructive or far-reaching changes (deleting a repository, making
    /// it public, adding a collaborator, deleting an item for good). A standing permission never covers it.
    pub once_only: bool,
    /// Only the paired Reins desktop app may call it (its answer is a credential sealed to the app's key). Never
    /// listed to an AI and never accepted over MCP; the phone checks the caller's key was pinned at pairing.
    pub desktop_only: bool,
}

impl ToolSpec {
    /// The kind of change this tool makes, for permissions (see [`ToolSpec::class`]).
    #[must_use]
    pub fn in_class(mut self, class: &'static str) -> Self {
        self.class = class;
        self
    }

    /// Marks the tool as asked for every time (see [`ToolSpec::once_only`]).
    #[must_use]
    pub fn once(mut self) -> Self {
        self.once_only = true;
        self
    }

    /// Marks the tool as callable by the desktop app only (see [`ToolSpec::desktop_only`]).
    #[must_use]
    pub fn desktop(mut self) -> Self {
        self.desktop_only = true;
        self
    }
}

/// One kind of change an integration's writes fall into; a permission can allow some kinds and not others.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClassInfo {
    pub id: &'static str,
    pub label: &'static str,
}

/// The kinds of change a service has (empty when it has none).
#[must_use]
pub fn classes(service: &str) -> &'static [ClassInfo] {
    match service {
        GMAIL => gmail::CLASSES,
        GITHUB => github::CLASSES,
        VAULT => vault::CLASSES,
        GITLAB | CODEBERG | BITBUCKET => desktop::HOST_CLASSES,
        _ => &[],
    }
}

fn control_free(s: &str) -> bool {
    !s.chars().any(|c| c.is_control() && c != '\n' && c != '\t')
}

impl ToolSpec {
    /// The JSON schema of the tool's arguments (with the common `account`).
    #[must_use]
    pub fn input_schema(&self) -> Value {
        let mut properties = Map::new();
        properties.insert(
            "account".to_owned(),
            json!({"type": "string", "maxLength": MAX_ACCOUNT_LEN,
                "description": "Which account of this integration to use, when the user has several. Optional with one."}),
        );
        let mut required = Vec::new();
        for p in &self.params {
            let mut schema = match p.kind {
                Kind::Str {
                    max,
                }
                | Kind::Text {
                    max,
                } => json!({"type": "string", "maxLength": max}),
                Kind::Int {
                    min,
                    max,
                    default,
                } => {
                    let mut v = json!({"type": "integer", "minimum": min, "maximum": max});
                    if let Some(d) = default {
                        v["default"] = json!(d);
                    }
                    v
                }
                Kind::Bool => json!({"type": "boolean"}),
                Kind::List {
                    max_items,
                    max_len,
                } => json!({"type": "array", "items": {"type": "string", "maxLength": max_len}, "maxItems": max_items}),
                Kind::Choice(options) => json!({"type": "string", "enum": options}),
                Kind::Map {
                    max_items,
                    max_val,
                    ..
                } => json!({"type": "object", "additionalProperties": {"type": "string", "maxLength": max_val},
                    "maxProperties": max_items}),
                Kind::Json {
                    ..
                } => json!({}),
            };
            schema["description"] = json!(p.description);
            properties.insert(p.name.to_owned(), schema);
            if p.required {
                required.push(p.name);
            }
        }
        json!({"type": "object", "properties": properties, "required": required, "additionalProperties": false})
    }

    /// The MCP annotations: writes are not read-only.
    #[must_use]
    pub fn annotations(&self) -> Value {
        match self.effect {
            Effect::Write => json!({"readOnlyHint": false, "destructiveHint": self.once_only, "openWorldHint": true}),
            _ => json!({"readOnlyHint": true, "openWorldHint": false}),
        }
    }

    /// Validates and normalizes the arguments of a tool call. Unknown arguments are refused, never dropped. Returns the
    /// call and the account the AI named, if any.
    pub fn parse(&self, arguments: &Value) -> Result<(ConnectorCall, Option<String>), String> {
        self.parse_args(arguments)
    }

    fn parse_args(&self, arguments: &Value) -> Result<(ConnectorCall, Option<String>), String> {
        let given = arguments.as_object().ok_or_else(|| "Tool arguments must be a JSON object.".to_owned())?;
        let mut account = None;
        let mut args = Map::new();
        for (key, value) in given {
            if key == "account" {
                account = match value {
                    Value::Null => None,
                    Value::String(s) => Some(normalize_account_name(s)?),
                    _ => return Err("`account` must be a string.".to_owned()),
                };
                continue;
            }
            if !self.params.iter().any(|p| p.name == key) {
                let allowed: Vec<&str> = std::iter::once("account").chain(self.params.iter().map(|p| p.name)).collect();
                return Err(format!("Unknown property `{key}`. Allowed properties: {}.", allowed.join(", ")));
            }
        }
        for p in &self.params {
            match given.get(p.name).filter(|v| !v.is_null()) {
                None => match p.kind {
                    Kind::Int {
                        default: Some(d),
                        ..
                    } => {
                        args.insert(p.name.to_owned(), json!(d));
                    }
                    _ if p.required => return Err(format!("`{}` is required.", p.name)),
                    _ => {}
                },
                Some(value) => {
                    args.insert(p.name.to_owned(), check(p, value)?);
                }
            }
        }
        Ok((
            ConnectorCall {
                service: self.service.to_owned(),
                op: self.op.to_owned(),
                args,
            },
            account,
        ))
    }
}

fn check(p: &Param, value: &Value) -> Result<Value, String> {
    let name = p.name;
    match p.kind {
        Kind::Str {
            max,
        } => {
            let s = value.as_str().ok_or_else(|| format!("`{name}` must be a string."))?;
            let s = s.trim();
            // Characters, as the schema's `maxLength` counts them (and as clients cut text), not bytes.
            if (p.required && s.is_empty()) || s.chars().count() > max || !control_free(s) {
                return Err(format!(
                    "`{name}` must be {}..={max} characters without control characters.",
                    u8::from(p.required)
                ));
            }
            Ok(json!(s))
        }
        Kind::Int {
            min,
            max,
            ..
        } => {
            let n = value.as_i64().ok_or_else(|| format!("`{name}` must be an integer."))?;
            if !(min..=max).contains(&n) {
                return Err(format!("`{name}` must be {min}..={max}."));
            }
            Ok(json!(n))
        }
        Kind::Bool => value.as_bool().map(|b| json!(b)).ok_or_else(|| format!("`{name}` must be true or false.")),
        Kind::List {
            max_items,
            max_len,
        } => {
            let items = value.as_array().ok_or_else(|| format!("`{name}` must be an array of strings."))?;
            if items.len() > max_items {
                return Err(format!("`{name}` may have at most {max_items} entries."));
            }
            let mut out = Vec::new();
            for item in items {
                let s = item.as_str().ok_or_else(|| format!("`{name}` must contain only strings."))?.trim();
                if s.is_empty() || s.chars().count() > max_len || !control_free(s) {
                    return Err(format!("Entries of `{name}` must be 1..={max_len} characters."));
                }
                out.push(json!(s));
            }
            Ok(Value::Array(out))
        }
        Kind::Choice(options) => {
            let s = value.as_str().ok_or_else(|| format!("`{name}` must be a string."))?.trim().to_ascii_lowercase();
            if options.contains(&s.as_str()) {
                Ok(json!(s))
            } else {
                Err(format!("`{name}` must be one of: {}.", options.join(", ")))
            }
        }
        Kind::Text {
            max,
        } => {
            let s = value.as_str().ok_or_else(|| format!("`{name}` must be a string."))?;
            if s.chars().count() > max
                || (p.required && s.is_empty())
                || s.chars().any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
            {
                return Err(format!("`{name}` must be at most {max} characters, without control characters."));
            }
            Ok(json!(s))
        }
        Kind::Map {
            max_items,
            max_key,
            max_val,
        } => {
            let map = value.as_object().ok_or_else(|| format!("`{name}` must be an object of strings."))?;
            if map.len() > max_items {
                return Err(format!("`{name}` may have at most {max_items} entries."));
            }
            let mut out = Map::new();
            for (k, v) in map {
                let v = v.as_str().ok_or_else(|| format!("`{name}` must contain only strings."))?;
                if k.is_empty()
                    || k.chars().count() > max_key
                    || v.chars().count() > max_val
                    || !control_free(k)
                    || !control_free(v)
                {
                    return Err(format!("Entries of `{name}` are too long or contain control characters."));
                }
                out.insert(k.clone(), json!(v));
            }
            Ok(Value::Object(out))
        }
        Kind::Json {
            max_len,
        } => {
            if value.to_string().len() > max_len {
                return Err(format!("`{name}` is larger than {max_len} bytes."));
            }
            Ok(value.clone())
        }
    }
}

/// An account name as an AI may give it: an address, a phone number, a login. Compared without case.
pub fn normalize_account_name(raw: &str) -> Result<String, String> {
    let s = raw.trim().to_lowercase();
    if s.is_empty() || s.len() > MAX_ACCOUNT_LEN || s.chars().any(char::is_control) {
        return Err(format!("`account` must be 1..={MAX_ACCOUNT_LEN} characters without control characters."));
    }
    Ok(s)
}

pub(crate) const fn str_p(name: &'static str, max: usize, required: bool, description: &'static str) -> Param {
    Param {
        name,
        kind: Kind::Str {
            max,
        },
        required,
        description,
    }
}

pub(crate) const fn int_p(
    name: &'static str,
    min: i64,
    max: i64,
    default: Option<i64>,
    description: &'static str,
) -> Param {
    Param {
        name,
        kind: Kind::Int {
            min,
            max,
            default,
        },
        required: false,
        description,
    }
}

pub(crate) const fn list_p(name: &'static str, max_items: usize, max_len: usize, description: &'static str) -> Param {
    Param {
        name,
        kind: Kind::List {
            max_items,
            max_len,
        },
        required: false,
        description,
    }
}

pub(crate) const fn choice_p(
    name: &'static str,
    options: &'static [&'static str],
    required: bool,
    description: &'static str,
) -> Param {
    Param {
        name,
        kind: Kind::Choice(options),
        required,
        description,
    }
}

#[allow(dead_code, reason = "helpers for the tool areas")]
pub(crate) const fn bool_p(name: &'static str, description: &'static str) -> Param {
    Param {
        name,
        kind: Kind::Bool,
        required: false,
        description,
    }
}

#[allow(dead_code, reason = "helpers for the tool areas")]
pub(crate) const fn text_p(name: &'static str, max: usize, required: bool, description: &'static str) -> Param {
    Param {
        name,
        kind: Kind::Text {
            max,
        },
        required,
        description,
    }
}

#[allow(dead_code, reason = "helpers for the tool areas")]
pub(crate) const fn map_p(name: &'static str, max_items: usize, max_val: usize, description: &'static str) -> Param {
    Param {
        name,
        kind: Kind::Map {
            max_items,
            max_key: 100,
            max_val,
        },
        required: false,
        description,
    }
}

#[allow(dead_code, reason = "helpers for the tool areas")]
pub(crate) const fn json_p(name: &'static str, max_len: usize, required: bool, description: &'static str) -> Param {
    Param {
        name,
        kind: Kind::Json {
            max_len,
        },
        required,
        description,
    }
}

pub(crate) const LIMIT: Param = int_p("limit", 1, 50, Some(20), "How many results at most.");
pub(crate) const WHEN_HELP: &str = "A date (2026-10-05) or a date and time with offset (2026-10-05T14:00:00+02:00).";

#[allow(clippy::too_many_arguments, reason = "a tool is described by this many independent facts")]
pub(crate) fn tool(
    tool: &'static str,
    service: &'static str,
    op: &'static str,
    effect: Effect,
    title: &'static str,
    description: &'static str,
    params: Vec<Param>,
    resource_param: Option<&'static str>,
) -> ToolSpec {
    ToolSpec {
        tool,
        service,
        op,
        effect,
        title,
        description,
        params,
        resource_param,
        class: "",
        once_only: false,
        desktop_only: false,
    }
}

/// Every tool of every integration, in a fixed order (`tools/list` must be deterministic).
#[must_use]
pub fn specs() -> &'static [ToolSpec] {
    static SPECS: OnceLock<Vec<ToolSpec>> = OnceLock::new();
    SPECS.get_or_init(build_specs)
}

#[must_use]
pub fn spec_for_tool(name: &str) -> Option<&'static ToolSpec> {
    specs().iter().find(|s| s.tool == name)
}

const SENSITIVE: &str = " The user approves on their phone, and sees exactly what is shared.";

fn build_specs() -> Vec<ToolSpec> {
    use Effect::{List, Read, Search, Write};
    let mut all = vec![
        // ---- Telegram (the user's own account) ----
        tool(
            "telegram_list_chats",
            TELEGRAM,
            "list_chats",
            List,
            "List Telegram chats",
            "Lists the user's Telegram chats (name, kind and id), newest activity first. The user chooses which chats \
             to show you; use the ids with telegram_read, telegram_search and telegram_send.",
            vec![str_p("query", 100, false, "Only chats whose name contains this text."), LIMIT],
            None,
        ),
        tool(
            "telegram_read",
            TELEGRAM,
            "read",
            Read,
            "Read Telegram messages",
            "Reads the latest messages of one Telegram chat, newest first.",
            vec![
                str_p("chat", 100, true, "The chat: an id from telegram_list_chats, an @username or the exact name."),
                LIMIT,
            ],
            Some("chat"),
        ),
        tool(
            "telegram_search",
            TELEGRAM,
            "search",
            Search,
            "Search Telegram messages",
            "Searches messages for a text, in one chat or across all of them.",
            vec![
                str_p("query", 200, true, "The text to look for."),
                str_p("chat", 100, false, "Restrict the search to this chat."),
                LIMIT,
            ],
            Some("chat"),
        ),
        tool(
            "telegram_send",
            TELEGRAM,
            "send",
            Write,
            "Send a Telegram message",
            "Sends a text message from the user's Telegram account. The user always sees the chat and the exact text \
             on their phone and must approve unless they granted a standing permission for that chat.",
            vec![
                str_p("chat", 100, true, "The chat: an id, an @username or the exact name."),
                str_p("text", 4_000, true, "The message."),
                int_p("reply_to", 1, i64::MAX, None, "Id of a message to reply to."),
            ],
            Some("chat"),
        ),
        // ---- Google Calendar ----
        tool(
            "calendar_list_calendars",
            GCALENDAR,
            "list_calendars",
            List,
            "List Google calendars",
            "Lists the user's Google calendars (name and id).",
            vec![],
            None,
        ),
        tool(
            "calendar_list_events",
            GCALENDAR,
            "list_events",
            Read,
            "List Google Calendar events",
            "Lists events of one calendar between two moments, soonest first.",
            vec![
                str_p(
                    "calendar",
                    200,
                    false,
                    "A calendar id from calendar_list_calendars. Default: the main calendar.",
                ),
                str_p("from", 40, false, WHEN_HELP),
                str_p("to", 40, false, WHEN_HELP),
                str_p("query", 200, false, "Only events containing this text."),
                LIMIT,
            ],
            Some("calendar"),
        ),
        tool(
            "calendar_create_event",
            GCALENDAR,
            "create_event",
            Write,
            "Create a Google Calendar event",
            "Adds an event to a calendar. The user sees it and approves on their phone unless they granted a standing \
             permission for that calendar.",
            vec![
                str_p("title", 300, true, "The event title."),
                str_p("start", 40, true, WHEN_HELP),
                str_p("end", 40, false, "When it ends; default one hour after the start (a day for a date)."),
                str_p("calendar", 200, false, "Default: the main calendar."),
                str_p("description", 4_000, false, "Notes."),
                str_p("location", 300, false, "Where."),
                list_p("attendees", 20, 254, "Email addresses to invite."),
            ],
            Some("calendar"),
        ),
        tool(
            "calendar_delete_event",
            GCALENDAR,
            "delete_event",
            Write,
            "Delete a Google Calendar event",
            "Deletes an event. The user approves on their phone.",
            vec![
                str_p("event_id", 200, true, "The id of the event, from calendar_list_events."),
                str_p("calendar", 200, false, "Default: the main calendar."),
            ],
            Some("calendar"),
        ),
        // ---- Google Contacts ----
        tool(
            "contacts_search",
            GCONTACTS,
            "search",
            Search,
            "Search Google contacts",
            "Finds contacts by name, email or phone number.",
            vec![str_p("query", 200, true, "Part of a name, an email or a number."), LIMIT],
            None,
        ),
        // ---- On the phone itself ----
        tool(
            "device_calendar_list_events",
            DEVICE_CALENDAR,
            "list_events",
            Read,
            "List events in the phone's calendar",
            "Lists events from the calendars on the user's phone (whatever accounts sync there), soonest first.",
            vec![
                str_p("from", 40, false, WHEN_HELP),
                str_p("to", 40, false, WHEN_HELP),
                str_p("query", 200, false, "Only events containing this text."),
                LIMIT,
            ],
            None,
        ),
        tool(
            "device_calendar_create_event",
            DEVICE_CALENDAR,
            "create_event",
            Write,
            "Add an event to the phone's calendar",
            "Adds an event to the phone's default calendar. The user approves on their phone.",
            vec![
                str_p("title", 300, true, "The event title."),
                str_p("start", 40, true, WHEN_HELP),
                str_p("end", 40, false, "When it ends; default one hour after the start."),
                str_p("description", 4_000, false, "Notes."),
                str_p("location", 300, false, "Where."),
            ],
            None,
        ),
        tool(
            "device_contacts_search",
            DEVICE_CONTACTS,
            "search",
            Search,
            "Search the phone's contacts",
            "Finds contacts stored on the user's phone by name, email or phone number.",
            vec![str_p("query", 200, true, "Part of a name, an email or a number."), LIMIT],
            None,
        ),
        tool(
            "sms_list_threads",
            SMS,
            "list_threads",
            List,
            "List SMS conversations",
            "Lists the user's text-message conversations (who with, latest activity).",
            vec![LIMIT],
            None,
        ),
        tool(
            "sms_read",
            SMS,
            "read",
            Read,
            "Read text messages",
            "Reads the latest text messages of one conversation. Messages that look like login or verification \
             codes are never shared without the user ticking them one by one.",
            vec![str_p("thread", 100, true, "A conversation id from sms_list_threads."), LIMIT],
            Some("thread"),
        ),
        tool(
            "sms_send",
            SMS,
            "send",
            Write,
            "Send a text message",
            "Sends an SMS from the user's phone. The user always approves the number and the text.",
            vec![str_p("to", 40, true, "The phone number."), str_p("text", 1_000, true, "The message.")],
            Some("to"),
        ),
    ];
    all.extend(gmail::tools());
    all.extend(github::tools());
    all.extend(vault::tools());
    all.extend(desktop::tools());
    all
}

/// What the note under the sensitive tools says; the server appends it to descriptions it renders.
#[must_use]
pub fn sensitive_note() -> &'static str {
    SENSITIVE
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lengths_are_characters_like_the_schema_says() {
        let ask = spec_for_tool("desktop_ask").unwrap();
        let key = "k".repeat(43);
        // What the desktop app sends for a long command: 299 characters and an ellipsis, 302 bytes.
        let cut = format!("{}…", "x".repeat(299));
        assert!(cut.len() > 300);
        let detail = "é".repeat(8_000);
        let args = json!({"question": cut, "detail": detail, "client_key": key, "nonce": "n"});
        ask.parse(&args).unwrap();
        let long = json!({"question": "x".repeat(301), "client_key": key, "nonce": "n"});
        assert!(ask.parse(&long).unwrap_err().contains("characters"));
        let too_much = json!({"question": "q", "detail": "é".repeat(8_001), "client_key": key, "nonce": "n"});
        assert!(ask.parse(&too_much).unwrap_err().contains("8000 characters"));
    }

    #[test]
    fn every_tool_is_named_once_and_described_completely() {
        let mut names = std::collections::BTreeSet::new();
        for s in specs() {
            assert!(names.insert(s.tool), "{} twice", s.tool);
            assert!(!s.description.is_empty() && !s.title.is_empty());
            let schema = s.input_schema();
            assert_eq!(schema["additionalProperties"], false);
            for required in schema["required"].as_array().unwrap() {
                assert!(schema["properties"].get(required.as_str().unwrap()).is_some(), "{}", s.tool);
            }
            if let Some(resource) = s.resource_param {
                assert!(s.params.iter().any(|p| p.name == resource), "{} names a missing resource argument", s.tool);
            }
            assert!(spec_for_tool(s.tool).is_some());
            assert!(
                ConnectorCall {
                    service: s.service.to_owned(),
                    op: s.op.to_owned(),
                    args: Map::new()
                }
                .spec()
                .is_some()
            );
        }
        let ops: std::collections::BTreeSet<_> = specs().iter().map(|s| (s.service, s.op)).collect();
        assert_eq!(ops.len(), specs().len(), "a service and operation identify exactly one tool");
    }

    #[test]
    fn arguments_are_checked_defaulted_and_normalized() {
        let read = spec_for_tool("telegram_read").unwrap();
        let (call, account) = read.parse(&json!({"chat": " @anna ", "account": " +1555 "})).unwrap();
        assert_eq!(
            (call.str_arg("chat"), call.int_arg("limit"), account.as_deref()),
            (Some("@anna"), Some(20), Some("+1555"))
        );
        assert_eq!(call.effect(), Some(Effect::Read));
        assert_eq!(call.resource().as_deref(), Some("@anna"));
        assert!(read.parse(&json!({})).unwrap_err().contains("`chat` is required"));
        assert!(read.parse(&json!({"chat": "x", "limit": 51})).unwrap_err().contains("1..=50"));
        assert!(read.parse(&json!({"chat": "x", "limit": "5"})).unwrap_err().contains("integer"));
        assert!(read.parse(&json!({"chat": "x", "bcc": 1})).unwrap_err().contains("Unknown property `bcc`"));
        assert!(read.parse(&json!({"chat": "a\u{7}b"})).unwrap_err().contains("control"));
        assert!(read.parse(&json!({"chat": "x", "account": 5})).unwrap_err().contains("string"));
        assert!(read.parse(&json!([])).unwrap_err().contains("object"));
    }

    #[test]
    fn lists_choices_and_bools_are_validated() {
        let issues = spec_for_tool("github_list_issues").unwrap();
        let (call, _) = issues.parse(&json!({"repo": "a/b", "state": "CLOSED"})).unwrap();
        assert_eq!(call.str_arg("state"), Some("closed"));
        assert!(issues.parse(&json!({"repo": "a/b", "state": "merged"})).unwrap_err().contains("one of"));
        let create = spec_for_tool("calendar_create_event").unwrap();
        let (call, _) = create.parse(&json!({"title": "T", "start": "2026-10-05", "attendees": ["a@b.com"]})).unwrap();
        assert_eq!(call.list_arg("attendees"), ["a@b.com"]);
        assert!(
            create.parse(&json!({"title": "T", "start": "x", "attendees": [1]})).unwrap_err().contains("only strings")
        );
        let too_many: Vec<String> = (0..21).map(|i| format!("{i}@b.com")).collect();
        assert!(
            create
                .parse(&json!({"title": "T", "start": "x", "attendees": too_many}))
                .unwrap_err()
                .contains("at most 20")
        );
    }

    #[test]
    fn a_relayed_call_is_rechecked_and_unknown_ones_refused() {
        let call = ConnectorCall {
            service: TELEGRAM.to_owned(),
            op: "send".to_owned(),
            args: json!({"chat": "Anna", "text": "hi"}).as_object().unwrap().clone(),
        };
        assert_eq!(call.clone().normalized().unwrap().int_arg("reply_to"), None);
        let mut bad = call.clone();
        bad.args.insert("extra".to_owned(), json!(1));
        assert!(bad.normalized().is_err());
        let unknown = ConnectorCall {
            service: "nope".to_owned(),
            ..call
        };
        assert!(unknown.normalized().unwrap_err().contains("unknown operation"));
    }

    #[test]
    fn the_wire_format_is_flat_and_stable() {
        let call = ConnectorCall {
            service: GITHUB.to_owned(),
            op: "get_issue".to_owned(),
            args: json!({"repo": "a/b", "number": 3}).as_object().unwrap().clone(),
        };
        let v = serde_json::to_value(&call).unwrap();
        assert_eq!(v, json!({"service": "github", "op": "get_issue", "args": {"repo": "a/b", "number": 3}}));
        assert_eq!(serde_json::from_value::<ConnectorCall>(v).unwrap(), call);
        assert_eq!(Effect::Search.access(), "read");
        assert_eq!(Effect::Write.access(), "write");
        assert_eq!(Effect::List.access(), "list");
    }

    #[test]
    fn account_names_are_lenient_but_clean() {
        assert_eq!(normalize_account_name(" Octo-Cat ").unwrap(), "octo-cat");
        assert!(normalize_account_name("").is_err());
        assert!(normalize_account_name("a\nb").is_err());
        assert!(normalize_account_name(&"x".repeat(101)).is_err());
    }
}
