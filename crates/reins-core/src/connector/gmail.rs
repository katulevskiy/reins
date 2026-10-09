//! Gmail beyond search, read and send: organizing messages (archive, labels, read, star, spam), Trash, labels, drafts
//! with a file, attachments, filters and the vacation reply, through the Gmail REST API with the phone's own Google
//! authorization. Nothing here deletes mail for good: Trash keeps it for 30 days, and filters never forward.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use data_encoding::BASE64;
use futures::stream::{self, StreamExt};
use reins_proto::connector::gmail::{MAX_DRAFT_FILE_BYTES, MAX_ORGANIZE_IDS, MAX_TRASH_IDS};
use reins_proto::connector::{ConnectorCall, GMAIL};
use reins_proto::gmail::OutgoingEmail;
use reqwest::Method;
use serde_json::{Map, Value, json};

use super::{Connector, Item, Preview};
use crate::gmail::compose::{self, Attachment};
use crate::gmail::model::{GmailMessage, Part};
use crate::gmail::{GmailClient, Probe, parse};
use crate::google::GoogleApi;
use crate::types::GmailStatus;
use crate::{CoreError, GoogleTokenProvider, text};

/// The largest attachment handed to the AI (Gmail's own limit for a message).
const MAX_ATTACHMENT_BYTES: u64 = 25 * 1024 * 1024;
/// Messages fetched to show what an organizing call is about.
const PREVIEW_SAMPLE: usize = 30;
/// Examples listed in an approval.
const PREVIEW_EXAMPLES: usize = 5;
/// Concurrent Trash requests.
const TRASH_CONCURRENCY: usize = 8;
/// Labels Gmail keeps for itself that only their own actions may set (a label call could otherwise send to Spam or
/// Trash, or fake a sent message).
const RESERVED_LABELS: [&str; 5] = ["SPAM", "TRASH", "DRAFT", "SENT", "CHAT"];

const MAILBOX: &str = "mailbox";
const LABELS: &str = "labels";
const DRAFTS: &str = "drafts";
const FILTERS: &str = "filters";
const VACATION: &str = "vacation";

fn unexpected() -> CoreError {
    CoreError::service("Gmail sent an unexpected answer")
}

fn json_of(body: &str) -> Result<Value, CoreError> {
    serde_json::from_str(body).map_err(|_| unexpected())
}

fn plural(n: usize, one: &str) -> String {
    if n == 1 {
        format!("1 {one}")
    } else {
        format!("{n} {one}s")
    }
}

fn size_text(bytes: u64) -> String {
    crate::blob::size_text(bytes)
}

/// A Gmail message id as Gmail makes them: letters and digits.
fn message_id(raw: &str) -> Result<String, CoreError> {
    let id = raw.trim();
    if id.is_empty() || id.len() > 64 || !id.bytes().all(|b| b.is_ascii_alphanumeric()) {
        return Err(CoreError::service(format!("{id:?} is not a Gmail message id (they come from gmail_search).")));
    }
    Ok(id.to_owned())
}

/// The message ids of a call, checked, each once, in the order given.
fn message_ids(call: &ConnectorCall, max: usize) -> Result<Vec<String>, CoreError> {
    let mut ids: Vec<String> = Vec::new();
    for raw in call.list_arg("message_ids") {
        let id = message_id(raw)?;
        if !ids.contains(&id) {
            ids.push(id);
        }
    }
    if ids.is_empty() {
        return Err(CoreError::service("`message_ids` needs at least one message id."));
    }
    if ids.len() > max {
        return Err(CoreError::service(format!("At most {max} messages at a time.")));
    }
    Ok(ids)
}

#[derive(Clone, Debug)]
struct Label {
    id: String,
    name: String,
    system: bool,
}

/// What an organizing action does: labels added, labels removed, and how the approval says it.
struct Change {
    add: Vec<String>,
    remove: Vec<String>,
    verb: String,
    note: &'static str,
    undo: &'static str,
}

fn change(add: &[&str], remove: &[&str], verb: &str, note: &'static str, undo: &'static str) -> Change {
    Change {
        add: add.iter().map(|s| (*s).to_owned()).collect(),
        remove: remove.iter().map(|s| (*s).to_owned()).collect(),
        verb: verb.to_owned(),
        note,
        undo,
    }
}

pub struct GmailTools {
    http: reqwest::Client,
    base: String,
    token: Arc<dyn GoogleTokenProvider>,
    backoff: Duration,
}

impl GmailTools {
    pub fn new(http: reqwest::Client, base: &str, token: Arc<dyn GoogleTokenProvider>, backoff: Duration) -> Self {
        Self {
            http,
            base: base.to_owned(),
            token,
            backoff,
        }
    }

    fn api(&self, account: &str) -> GoogleApi {
        GoogleApi::new(self.http.clone(), &self.base, Arc::clone(&self.token), account, GMAIL, self.backoff)
    }

    fn client(&self, account: &str) -> GmailClient {
        GmailClient::new(self.http.clone(), &self.base, Arc::clone(&self.token), account, self.backoff)
    }

    async fn labels(&self, api: &GoogleApi) -> Result<Vec<Label>, CoreError> {
        let v = json_of(&api.request(&Method::GET, "/users/me/labels", &[], None).await?)?;
        Ok(v["labels"]
            .as_array()
            .map(|list| {
                list.iter()
                    .map(|l| Label {
                        id: l["id"].as_str().unwrap_or_default().to_owned(),
                        name: l["name"].as_str().unwrap_or_default().to_owned(),
                        system: l["type"] == "system",
                    })
                    .filter(|l| !l.id.is_empty())
                    .collect()
            })
            .unwrap_or_default())
    }

    /// A label the AI named, by id or by name (without case).
    async fn find_label(&self, api: &GoogleApi, given: &str) -> Result<Label, CoreError> {
        let given = given.trim();
        let labels = self.labels(api).await?;
        labels
            .iter()
            .find(|l| l.id == given)
            .or_else(|| labels.iter().find(|l| l.name.eq_ignore_ascii_case(given)))
            .cloned()
            .ok_or_else(|| {
                CoreError::service(format!(
                    "There is no label {given:?}. See gmail_list_labels, or create it with gmail_create_label."
                ))
            })
    }

    /// A label the user made (Gmail's own cannot be renamed, deleted or set by hand).
    async fn user_label(&self, api: &GoogleApi, given: &str) -> Result<Label, CoreError> {
        let label = self.find_label(api, given).await?;
        if label.system {
            return Err(CoreError::service(format!(
                "{} is one of Gmail's own labels; it cannot be changed.",
                label.name
            )));
        }
        Ok(label)
    }

    async fn organize_change(&self, api: &GoogleApi, call: &ConnectorCall) -> Result<Change, CoreError> {
        let action = call.str_arg("action").unwrap_or_default();
        Ok(match action {
            "archive" => {
                change(&[], &["INBOX"], "Archive", "They leave the Inbox and stay in All Mail.", "move_to_inbox")
            }
            "move_to_inbox" => change(&["INBOX"], &[], "Move to the Inbox", "", "archive"),
            "mark_read" => change(&[], &["UNREAD"], "Mark as read", "", "mark_unread"),
            "mark_unread" => change(&["UNREAD"], &[], "Mark as unread", "", "mark_read"),
            "star" => change(&["STARRED"], &[], "Star", "", "unstar"),
            "unstar" => change(&[], &["STARRED"], "Remove the star from", "", "star"),
            "mark_important" => change(&["IMPORTANT"], &[], "Mark as important", "", "mark_not_important"),
            "mark_not_important" => change(&[], &["IMPORTANT"], "Mark as not important", "", "mark_important"),
            "spam" => change(&["SPAM"], &["INBOX"], "Move to Spam", "Gmail learns from it.", "not_spam"),
            "not_spam" => change(&["INBOX"], &["SPAM"], "Move out of Spam into the Inbox", "", "spam"),
            "add_label" | "remove_label" => {
                let given = call
                    .str_arg("label")
                    .filter(|l| !l.is_empty())
                    .ok_or_else(|| CoreError::service(format!("`{action}` needs `label`.")))?;
                let label = self.find_label(api, given).await?;
                if RESERVED_LABELS.contains(&label.id.as_str()) {
                    return Err(CoreError::service(format!(
                        "{} cannot be set as a label; use the spam action or gmail_trash.",
                        label.name
                    )));
                }
                if action == "add_label" {
                    Change {
                        add: vec![label.id],
                        remove: Vec::new(),
                        verb: format!("Label \"{}\":", label.name),
                        note: "",
                        undo: "remove_label",
                    }
                } else {
                    Change {
                        add: Vec::new(),
                        remove: vec![label.id],
                        verb: format!("Remove the label \"{}\" from", label.name),
                        note: "",
                        undo: "add_label",
                    }
                }
            }
            other => return Err(CoreError::service(format!("Unknown action {other:?}."))),
        })
    }

    /// The approval lines for a call on many messages: who they are from and a few of them.
    async fn sample_lines(&self, account: &str, ids: &[String]) -> Result<Vec<String>, CoreError> {
        let sample: Vec<String> = ids.iter().take(PREVIEW_SAMPLE).cloned().collect();
        let found = self.client(account).fetch(&sample, false).await?;
        if found.is_empty() {
            return Err(CoreError::service("None of these messages exist (any more)."));
        }
        let mut senders: BTreeMap<String, usize> = BTreeMap::new();
        for m in &found {
            *senders.entry(m.summary.from.clone()).or_default() += 1;
        }
        let mut ranked: Vec<(String, usize)> = senders.into_iter().collect();
        ranked.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        let shown: Vec<String> = ranked.iter().take(4).map(|(from, n)| format!("{from} ({n})")).collect();
        let more = if ranked.len() > 4 {
            format!(", and {} more senders", ranked.len() - 4)
        } else {
            String::new()
        };
        let of = if ids.len() > found.len() {
            format!(" (among the first {})", found.len())
        } else {
            String::new()
        };
        let mut lines = vec![format!("From{of}: {}{more}", shown.join(", "))];
        for m in found.iter().take(PREVIEW_EXAMPLES) {
            let subject = if m.summary.subject.is_empty() {
                "(no subject)"
            } else {
                &m.summary.subject
            };
            lines.push(format!("• {} — {}", m.summary.from, text::truncate_chars(subject, 120)));
        }
        if ids.len() > PREVIEW_EXAMPLES {
            lines.push(format!("…and {} more", ids.len() - PREVIEW_EXAMPLES.min(found.len())));
        }
        Ok(lines)
    }

    fn draft_email(call: &ConnectorCall) -> Result<OutgoingEmail, CoreError> {
        OutgoingEmail {
            to: call.list_arg("to").into_iter().map(str::to_owned).collect(),
            cc: call.list_arg("cc").into_iter().map(str::to_owned).collect(),
            subject: call.str_arg("subject").unwrap_or_default().to_owned(),
            body: call.str_arg("body").unwrap_or_default().to_owned(),
            reply_to_message_id: call.str_arg("reply_to_message_id").map(str::to_owned),
        }
        .normalized()
        .map_err(|e| CoreError::service(e.to_string()))
    }

    /// The file a draft carries, read from the call or the upload.
    async fn draft_file(call: &ConnectorCall) -> Result<Option<(Attachment, Option<String>)>, CoreError> {
        let Some(name) = call.str_arg("file_name") else {
            if call.args.contains_key("content_base64") || call.args.contains_key("blob") {
                return Err(CoreError::service("Give `file_name` with the file."));
            }
            return Ok(None);
        };
        let (data, upload) = match (call.str_arg("content_base64"), call.str_arg("blob")) {
            (Some(_), Some(_)) => return Err(CoreError::service("Give either `content_base64` or `blob`, not both.")),
            (Some(b64), None) => {
                let compact: String = b64.split_whitespace().collect();
                let data = BASE64
                    .decode(compact.as_bytes())
                    .map_err(|_| CoreError::service("`content_base64` is not valid base64."))?;
                (data, None)
            }
            (None, Some(id)) => (crate::blob::read(id, MAX_DRAFT_FILE_BYTES).await?.to_vec(), Some(id.to_owned())),
            (None, None) => return Err(CoreError::service("The file has not been uploaded.")),
        };
        if data.len() as u64 > MAX_DRAFT_FILE_BYTES {
            return Err(CoreError::service(format!(
                "The file is larger than {}, which a draft cannot carry.",
                size_text(MAX_DRAFT_FILE_BYTES)
            )));
        }
        Ok(Some((
            Attachment {
                name: name.to_owned(),
                content_type: type_of(name).to_owned(),
                data,
            },
            upload,
        )))
    }

    async fn message(&self, api: &GoogleApi, id: &str) -> Result<GmailMessage, CoreError> {
        let path = format!("/users/me/messages/{id}");
        let body = api.request(&Method::GET, &path, &[("format", "full".to_owned())], None).await?;
        serde_json::from_str(&body).map_err(|_| unexpected())
    }

    async fn filter_lines(&self, api: &GoogleApi, filter: &Value) -> (String, String) {
        let names: BTreeMap<String, String> =
            self.labels(api).await.unwrap_or_default().into_iter().map(|l| (l.id, l.name)).collect();
        filter_text(filter, &names)
    }
}

/// Moves one message to Trash or out of it (`verb` is `trash` or `untrash`).
async fn move_one(api: &GoogleApi, id: String, verb: &str) -> (String, Result<String, CoreError>) {
    let result = api.request(&Method::POST, &format!("/users/me/messages/{id}/{verb}"), &[], None).await;
    (id, result)
}

/// A MIME type from a file name's extension.
fn type_of(name: &str) -> &'static str {
    let ext = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
    match ext.as_str() {
        "pdf" => "application/pdf",
        "txt" | "md" | "log" => "text/plain",
        "csv" => "text/csv",
        "html" | "htm" => "text/html",
        "json" => "application/json",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "zip" => "application/zip",
        "doc" => "application/msword",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "xls" => "application/vnd.ms-excel",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "ppt" => "application/vnd.ms-powerpoint",
        "pptx" => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        "ics" => "text/calendar",
        _ => "application/octet-stream",
    }
}

/// Every attached file of a message, depth first.
fn attached_parts<'a>(part: &'a Part, out: &mut Vec<&'a Part>) {
    if !part.filename.is_empty() {
        out.push(part);
        return;
    }
    for child in &part.parts {
        attached_parts(child, out);
    }
}

/// What a filter matches and what it does, in words.
fn filter_text(filter: &Value, label_names: &BTreeMap<String, String>) -> (String, String) {
    let c = &filter["criteria"];
    let mut matches = Vec::new();
    for (key, word) in [("from", "from"), ("to", "to"), ("subject", "subject"), ("query", "matching")] {
        if let Some(v) = c[key].as_str().filter(|v| !v.is_empty()) {
            matches.push(format!("{word} {}", text::one_line(v)));
        }
    }
    if c["hasAttachment"].as_bool() == Some(true) {
        matches.push("with attachments".to_owned());
    }
    let a = &filter["action"];
    let ids = |key: &str| -> Vec<String> {
        a[key].as_array().map(|l| l.iter().filter_map(Value::as_str).map(str::to_owned).collect()).unwrap_or_default()
    };
    let (add, remove) = (ids("addLabelIds"), ids("removeLabelIds"));
    let mut does = Vec::new();
    for (id, word) in [("INBOX", "skip the Inbox"), ("UNREAD", "mark read"), ("SPAM", "never send to Spam")] {
        if remove.iter().any(|r| r == id) {
            does.push(word.to_owned());
        }
    }
    for id in &add {
        does.push(match id.as_str() {
            "STARRED" => "star it".to_owned(),
            "IMPORTANT" => "mark important".to_owned(),
            "TRASH" => "move to Trash".to_owned(),
            other => format!("label \"{}\"", label_names.get(other).map_or(other, String::as_str)),
        });
    }
    if a["forward"].as_str().is_some_and(|f| !f.is_empty()) {
        does.push(format!("forward to {}", text::one_line(a["forward"].as_str().unwrap_or_default())));
    }
    (format!("Mail {}", matches.join(", ")), does.join(", "))
}

/// A date or a moment from the AI, as Gmail's milliseconds.
fn millis(name: &str, raw: Option<&str>) -> Result<Option<String>, CoreError> {
    raw.map(|t| {
        text::parse_when(t)
            .map(|secs| (secs * 1000).to_string())
            .ok_or_else(|| CoreError::service(format!("`{name}` is not a date or time: {t:?}.")))
    })
    .transpose()
}

#[async_trait::async_trait]
impl Connector for GmailTools {
    fn service(&self) -> &'static str {
        GMAIL
    }

    async fn fetch(&self, account: &str, call: &ConnectorCall) -> Result<Vec<Item>, CoreError> {
        let api = self.api(account);
        match call.op.as_str() {
            "list_labels" => Ok(self
                .labels(&api)
                .await?
                .into_iter()
                .map(|l| Item {
                    id: l.id.clone(),
                    resource: LABELS.to_owned(),
                    resource_label: "Labels".to_owned(),
                    title: text::one_line(&l.name),
                    from: if l.system {
                        "Gmail"
                    } else {
                        "user"
                    }
                    .to_owned(),
                    ..Item::default()
                })
                .collect()),
            "list_attachments" => {
                let id = message_id(call.str_arg("message_id").unwrap_or_default())?;
                let message = self.message(&api, &id).await?;
                let subject = parse::parse_message(&message, false).summary.subject;
                let mut parts = Vec::new();
                if let Some(payload) = &message.payload {
                    attached_parts(payload, &mut parts);
                }
                Ok(parts
                    .into_iter()
                    .map(|p| {
                        let size = p.body.as_ref().map_or(0, |b| b.size);
                        let mut extra = Map::new();
                        extra.insert("part".to_owned(), json!(p.part_id));
                        extra.insert("mime_type".to_owned(), json!(p.mime_type));
                        extra.insert("size".to_owned(), json!(size));
                        Item {
                            id: p.part_id.clone(),
                            resource: id.clone(),
                            resource_label: text::one_line(&subject),
                            title: text::one_line(&p.filename),
                            snippet: format!("{} · {}", text::one_line(&p.mime_type), size_text(size)),
                            extra,
                            ..Item::default()
                        }
                    })
                    .collect())
            }
            "get_attachment" => {
                let id = message_id(call.str_arg("message_id").unwrap_or_default())?;
                let wanted = call.str_arg("part").unwrap_or_default();
                let message = self.message(&api, &id).await?;
                let subject = parse::parse_message(&message, false).summary.subject;
                let mut parts = Vec::new();
                if let Some(payload) = &message.payload {
                    attached_parts(payload, &mut parts);
                }
                let part = parts.into_iter().find(|p| p.part_id == wanted).ok_or_else(|| {
                    CoreError::service("That message has no such attachment; see gmail_list_attachments.")
                })?;
                let body = part.body.as_ref();
                if body.map_or(0, |b| b.size) > MAX_ATTACHMENT_BYTES {
                    return Err(CoreError::service("The attachment is larger than 25 MB."));
                }
                let encoded = match (body.and_then(|b| b.data.clone()), body.and_then(|b| b.attachment_id.clone())) {
                    (Some(data), _) => data,
                    (None, Some(aid)) => {
                        let path = format!("/users/me/messages/{id}/attachments/{}", super::calendar::segment(&aid));
                        let v = json_of(&api.request(&Method::GET, &path, &[], None).await?)?;
                        v["data"].as_str().ok_or_else(unexpected)?.to_owned()
                    }
                    (None, None) => String::new(),
                };
                let data = parse::decode_data(&encoded).ok_or_else(unexpected)?;
                let name = text::one_line(&part.filename);
                let mut extra = Map::new();
                extra.insert("name".to_owned(), json!(name));
                extra.insert("mime_type".to_owned(), json!(part.mime_type));
                extra.insert("encoding".to_owned(), json!("base64"));
                extra.insert("size".to_owned(), json!(data.len()));
                let mut item = Item {
                    id: part.part_id.clone(),
                    resource: id.clone(),
                    resource_label: text::one_line(&subject),
                    title: name.clone(),
                    snippet: format!("{name} · {} · {}", text::one_line(&part.mime_type), size_text(data.len() as u64)),
                    body: Some(BASE64.encode(&data)),
                    // The file itself is not text to show in the approval or keep in the activity: the user sees
                    // its name, type and size.
                    secret: true,
                    extra,
                    ..Item::default()
                };
                if crate::blob::as_link(data.len() as u64) {
                    item.extra.insert(crate::blob::DELIVER_KEY.to_owned(), crate::blob::output_marker(&name));
                }
                Ok(vec![item])
            }
            "list_filters" => {
                let v = json_of(&api.request(&Method::GET, "/users/me/settings/filters", &[], None).await?)?;
                let names: BTreeMap<String, String> =
                    self.labels(&api).await.unwrap_or_default().into_iter().map(|l| (l.id, l.name)).collect();
                Ok(v["filter"]
                    .as_array()
                    .map(|list| {
                        list.iter()
                            .map(|f| {
                                let (matches, does) = filter_text(f, &names);
                                Item {
                                    id: f["id"].as_str().unwrap_or_default().to_owned(),
                                    resource: FILTERS.to_owned(),
                                    resource_label: "Filters".to_owned(),
                                    title: matches,
                                    snippet: does,
                                    ..Item::default()
                                }
                            })
                            .collect()
                    })
                    .unwrap_or_default())
            }
            "get_vacation" => {
                let v = json_of(&api.request(&Method::GET, "/users/me/settings/vacation", &[], None).await?)?;
                let on = v["enableAutoReply"].as_bool() == Some(true);
                let when = |key: &str| {
                    v[key].as_str().and_then(|ms| ms.parse::<i64>().ok()).map(|ms| text::iso_utc(ms / 1000))
                };
                let mut extra = Map::new();
                extra.insert("enabled".to_owned(), json!(on));
                extra.insert("contacts_only".to_owned(), json!(v["restrictToContacts"].as_bool() == Some(true)));
                if let Some(start) = when("startTime") {
                    extra.insert("start".to_owned(), json!(start));
                }
                if let Some(end) = when("endTime") {
                    extra.insert("end".to_owned(), json!(end));
                }
                let body = v["responseBodyPlainText"]
                    .as_str()
                    .map(str::to_owned)
                    .or_else(|| v["responseBodyHtml"].as_str().map(text::html_to_text))
                    .unwrap_or_default();
                Ok(vec![Item {
                    id: VACATION.to_owned(),
                    resource: VACATION.to_owned(),
                    resource_label: "Vacation reply".to_owned(),
                    title: text::one_line(v["responseSubject"].as_str().unwrap_or_default()),
                    snippet: if on {
                        "On"
                    } else {
                        "Off"
                    }
                    .to_owned(),
                    body: Some(body).filter(|b| !b.is_empty()),
                    extra,
                    ..Item::default()
                }])
            }
            other => Err(CoreError::service(format!("Gmail cannot {other} here"))),
        }
    }

    async fn preview(&self, account: &str, call: &ConnectorCall) -> Result<Preview, CoreError> {
        let api = self.api(account);
        let mailbox = |lines: Vec<String>| Preview {
            resource: MAILBOX.to_owned(),
            resource_label: "Mailbox".to_owned(),
            lines,
            ..Preview::default()
        };
        let at = |resource: &str, label: &str, lines: Vec<String>| Preview {
            resource: resource.to_owned(),
            resource_label: label.to_owned(),
            lines,
            ..Preview::default()
        };
        match call.op.as_str() {
            "organize" => {
                let ids = message_ids(call, MAX_ORGANIZE_IDS)?;
                let change = self.organize_change(&api, call).await?;
                let mut lines = vec![format!("{} {}", change.verb, plural(ids.len(), "message"))];
                if !change.note.is_empty() {
                    lines.push(change.note.to_owned());
                }
                lines.extend(self.sample_lines(account, &ids).await?);
                Ok(mailbox(lines))
            }
            "trash" => {
                let ids = message_ids(call, MAX_TRASH_IDS)?;
                let restore = call.str_arg("action") == Some("restore");
                let mut lines = vec![if restore {
                    format!("Move {} out of Trash", plural(ids.len(), "message"))
                } else {
                    format!("Move {} to Trash", plural(ids.len(), "message"))
                }];
                if !restore {
                    lines.push("Gmail keeps them in Trash for 30 days; nothing is deleted for good.".to_owned());
                }
                lines.extend(self.sample_lines(account, &ids).await?);
                Ok(mailbox(lines))
            }
            "create_label" => {
                let name = call.str_arg("name").unwrap_or_default().trim().to_owned();
                if self.labels(&api).await?.iter().any(|l| l.name.eq_ignore_ascii_case(&name)) {
                    return Err(CoreError::service(format!("The label {name:?} exists already.")));
                }
                Ok(at(LABELS, "Labels", vec![format!("Create the label \"{name}\"")]))
            }
            "rename_label" => {
                let label = self.user_label(&api, call.str_arg("label").unwrap_or_default()).await?;
                let new_name = call.str_arg("new_name").unwrap_or_default();
                Ok(at(LABELS, "Labels", vec![format!("Rename the label \"{}\" to \"{new_name}\"", label.name)]))
            }
            "delete_label" => {
                let label = self.user_label(&api, call.str_arg("label").unwrap_or_default()).await?;
                let v = json_of(
                    &api.request(
                        &Method::GET,
                        &format!("/users/me/labels/{}", super::calendar::segment(&label.id)),
                        &[],
                        None,
                    )
                    .await?,
                )?;
                let total = v["messagesTotal"].as_u64().unwrap_or(0);
                Ok(at(
                    LABELS,
                    "Labels",
                    vec![
                        format!("Delete the label \"{}\"", label.name),
                        format!(
                            "{} lose this label; nothing else about them changes.",
                            plural(usize::try_from(total).unwrap_or(usize::MAX), "message")
                        ),
                    ],
                ))
            }
            "create_draft" => {
                let email = Self::draft_email(call)?;
                let mut lines = vec![format!("Draft to: {}", email.to.join(", "))];
                if !email.cc.is_empty() {
                    lines.push(format!("Cc: {}", email.cc.join(", ")));
                }
                lines.push(format!("Subject: {}", text::one_line(&email.subject)));
                if email.reply_to_message_id.is_some() {
                    lines.push("A reply in the same conversation".to_owned());
                }
                lines.push(text::truncate_chars(&email.body, 1_500));
                if let (Some(name), Some(b64)) = (call.str_arg("file_name"), call.str_arg("content_base64")) {
                    let bytes = b64.split_whitespace().map(str::len).sum::<usize>() / 4 * 3;
                    lines.push(format!("Attached: {} · about {}", text::one_line(name), size_text(bytes as u64)));
                }
                lines.push("Nothing is sent: it waits in Drafts for you to send from Gmail.".to_owned());
                Ok(at(DRAFTS, "Drafts", lines))
            }
            "create_filter" => {
                let (body, _) = self.filter_body(&api, call).await?;
                let (matches, does) = self.filter_lines(&api, &body).await;
                Ok(at(
                    FILTERS,
                    "Filters",
                    vec![
                        format!("New filter: {matches}"),
                        format!("Then: {does}"),
                        "Gmail applies it to new mail from now on.".to_owned(),
                    ],
                ))
            }
            "delete_filter" => {
                let id = call.str_arg("filter_id").unwrap_or_default();
                let path = format!("/users/me/settings/filters/{}", super::calendar::segment(id));
                let filter = json_of(&api.request(&Method::GET, &path, &[], None).await?)?;
                let (matches, does) = self.filter_lines(&api, &filter).await;
                Ok(at(FILTERS, "Filters", vec![format!("Delete the filter: {matches}"), format!("It did: {does}")]))
            }
            "set_vacation" => {
                let body = vacation_body(call)?;
                let lines = if body["enableAutoReply"] == true {
                    let mut lines = vec![format!(
                        "Turn the vacation reply on: \"{}\"",
                        text::one_line(body["responseSubject"].as_str().unwrap_or_default())
                    )];
                    let when = |key: &str| {
                        body[key].as_str().and_then(|ms| ms.parse::<i64>().ok()).map(|ms| text::iso_utc(ms / 1000))
                    };
                    lines.push(format!(
                        "From {} until {}",
                        when("startTime").unwrap_or_else(|| "now".to_owned()),
                        when("endTime").unwrap_or_else(|| "turned off".to_owned())
                    ));
                    if body["restrictToContacts"] == true {
                        lines.push("Only to people in your contacts".to_owned());
                    }
                    lines.push(text::truncate_chars(body["responseBodyPlainText"].as_str().unwrap_or_default(), 1_500));
                    lines
                } else {
                    vec!["Turn the vacation reply off".to_owned()]
                };
                Ok(at(VACATION, "Vacation reply", lines))
            }
            other => Err(CoreError::service(format!("Gmail cannot {other} here"))),
        }
    }

    async fn perform(&self, account: &str, call: &ConnectorCall) -> Result<Value, CoreError> {
        let api = self.api(account);
        match call.op.as_str() {
            "organize" => {
                let ids = message_ids(call, MAX_ORGANIZE_IDS)?;
                let change = self.organize_change(&api, call).await?;
                let body = json!({"ids": ids, "addLabelIds": change.add, "removeLabelIds": change.remove});
                api.request(&Method::POST, "/users/me/messages/batchModify", &[], Some(&body)).await?;
                Ok(json!({"changed": ids.len(), "action": call.str_arg("action"), "undo_with": change.undo}))
            }
            "trash" => {
                let ids = message_ids(call, MAX_TRASH_IDS)?;
                let verb = if call.str_arg("action") == Some("restore") {
                    "untrash"
                } else {
                    "trash"
                };
                let moves: Vec<_> = ids.into_iter().map(|id| move_one(&api, id, verb)).collect();
                let results: Vec<(String, Result<String, CoreError>)> =
                    stream::iter(moves).buffer_unordered(TRASH_CONCURRENCY).collect().await;
                let total = results.len();
                let mut failed = Vec::new();
                let mut first_error = None;
                for (id, result) in results {
                    if let Err(e) = result {
                        failed.push(id);
                        first_error.get_or_insert(e);
                    }
                }
                if let Some(e) = first_error
                    && failed.len() == total
                {
                    return Err(e);
                }
                let undo = if verb == "trash" {
                    "restore"
                } else {
                    "trash"
                };
                Ok(json!({"moved": total - failed.len(), "failed": failed, "undo_with": undo}))
            }
            "create_label" => {
                let name = call.str_arg("name").unwrap_or_default().trim();
                let body = json!({"name": name, "labelListVisibility": "labelShow", "messageListVisibility": "show"});
                let v = json_of(&api.request(&Method::POST, "/users/me/labels", &[], Some(&body)).await?)?;
                Ok(json!({"created": true, "label_id": v["id"], "name": v["name"]}))
            }
            "rename_label" => {
                let label = self.user_label(&api, call.str_arg("label").unwrap_or_default()).await?;
                let body = json!({"name": call.str_arg("new_name").unwrap_or_default()});
                let path = format!("/users/me/labels/{}", super::calendar::segment(&label.id));
                api.request(&Method::PATCH, &path, &[], Some(&body)).await?;
                Ok(json!({"renamed": true, "label_id": label.id}))
            }
            "delete_label" => {
                let label = self.user_label(&api, call.str_arg("label").unwrap_or_default()).await?;
                let path = format!("/users/me/labels/{}", super::calendar::segment(&label.id));
                api.request(&Method::DELETE, &path, &[], None).await?;
                Ok(json!({"deleted": true}))
            }
            "create_draft" => {
                let email = Self::draft_email(call)?;
                let file = Self::draft_file(call).await?;
                let reply = match &email.reply_to_message_id {
                    Some(id) => self.client(account).reply_context(id).await,
                    None => None,
                };
                let raw = compose::raw_with(&email, reply.as_ref(), file.as_ref().map(|(a, _)| a))?;
                let mut message = json!({"raw": raw});
                if let Some(reply) = &reply {
                    message["threadId"] = json!(reply.thread_id);
                }
                let v = json_of(
                    &api.request(&Method::POST, "/users/me/drafts", &[], Some(&json!({"message": message}))).await?,
                )?;
                if let Some((_, Some(upload))) = &file {
                    crate::blob::used(upload).await;
                }
                Ok(json!({"saved": true, "draft_id": v["id"], "message_id": v["message"]["id"]}))
            }
            "create_filter" => {
                let (body, _) = self.filter_body(&api, call).await?;
                let v = json_of(&api.request(&Method::POST, "/users/me/settings/filters", &[], Some(&body)).await?)?;
                Ok(json!({"created": true, "filter_id": v["id"]}))
            }
            "delete_filter" => {
                let id = call.str_arg("filter_id").unwrap_or_default();
                let path = format!("/users/me/settings/filters/{}", super::calendar::segment(id));
                api.request(&Method::DELETE, &path, &[], None).await?;
                Ok(json!({"deleted": true}))
            }
            "set_vacation" => {
                let body = vacation_body(call)?;
                api.request(&Method::PUT, "/users/me/settings/vacation", &[], Some(&body)).await?;
                Ok(json!({"enabled": body["enableAutoReply"]}))
            }
            other => Err(CoreError::service(format!("Gmail cannot {other} here"))),
        }
    }

    async fn status(&self, account: &str) -> GmailStatus {
        match self.client(account).probe().await {
            Probe::Ready => GmailStatus::Ready,
            Probe::NeedsConsent => GmailStatus::NeedsConsent,
            Probe::Unavailable(message) => GmailStatus::Unavailable {
                message,
            },
        }
    }
}

impl GmailTools {
    /// The filter Gmail is sent: conditions and actions from the call, never a forward. Returns it and the label it
    /// applies, if any.
    async fn filter_body(&self, api: &GoogleApi, call: &ConnectorCall) -> Result<(Value, Option<Label>), CoreError> {
        let mut criteria = Map::new();
        for (arg, key) in [("from", "from"), ("to", "to"), ("subject", "subject"), ("query", "query")] {
            if let Some(v) = call.str_arg(arg).filter(|v| !v.is_empty()) {
                criteria.insert(key.to_owned(), json!(v));
            }
        }
        if call.bool_arg("has_attachment") == Some(true) {
            criteria.insert("hasAttachment".to_owned(), json!(true));
        }
        if criteria.is_empty() {
            return Err(CoreError::service("A filter needs a condition: from, to, subject, query or has_attachment."));
        }
        let on = |name: &str| call.bool_arg(name) == Some(true);
        let mut add: Vec<String> = Vec::new();
        let mut remove: Vec<String> = Vec::new();
        if on("skip_inbox") {
            remove.push("INBOX".to_owned());
        }
        if on("mark_read") {
            remove.push("UNREAD".to_owned());
        }
        if on("never_spam") {
            remove.push("SPAM".to_owned());
        }
        if on("star") {
            add.push("STARRED".to_owned());
        }
        if on("important") {
            add.push("IMPORTANT".to_owned());
        }
        if on("trash") {
            add.push("TRASH".to_owned());
        }
        let label = match call.str_arg("add_label").filter(|l| !l.is_empty()) {
            Some(given) => {
                let label = self.find_label(api, given).await?;
                if RESERVED_LABELS.contains(&label.id.as_str()) {
                    return Err(CoreError::service(format!("A filter cannot set {} as a label.", label.name)));
                }
                add.push(label.id.clone());
                Some(label)
            }
            None => None,
        };
        if add.is_empty() && remove.is_empty() {
            return Err(CoreError::service(
                "A filter needs an action: skip_inbox, mark_read, star, important, never_spam, trash or add_label.",
            ));
        }
        let body = json!({"criteria": criteria, "action": {"addLabelIds": add, "removeLabelIds": remove}});
        Ok((body, label))
    }
}

/// The vacation settings Gmail is sent.
fn vacation_body(call: &ConnectorCall) -> Result<Value, CoreError> {
    let enabled = call.bool_arg("enabled") == Some(true);
    if !enabled {
        return Ok(json!({"enableAutoReply": false}));
    }
    let body = call
        .str_arg("body")
        .filter(|b| !b.trim().is_empty())
        .ok_or_else(|| CoreError::service("Give the reply's `body` to turn the vacation reply on."))?;
    let mut v = json!({
        "enableAutoReply": true,
        "responseSubject": call.str_arg("subject").unwrap_or_default(),
        "responseBodyPlainText": body,
        "restrictToContacts": call.bool_arg("contacts_only") == Some(true),
    });
    let start = millis("start", call.str_arg("start"))?;
    let end = millis("end", call.str_arg("end"))?;
    if let (Some(s), Some(e)) = (&start, &end)
        && s.parse::<i64>().unwrap_or(0) >= e.parse::<i64>().unwrap_or(0)
    {
        return Err(CoreError::service("The vacation reply must end after it starts."));
    }
    if let Some(start) = start {
        v["startTime"] = json!(start);
    }
    if let Some(end) = end {
        v["endTime"] = json!(end);
    }
    Ok(v)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(op: &str, args: Value) -> ConnectorCall {
        let Value::Object(args) = args else {
            panic!("arguments are an object");
        };
        ConnectorCall {
            service: GMAIL.to_owned(),
            op: op.to_owned(),
            args,
        }
    }

    #[test]
    fn message_ids_are_checked_and_deduplicated() {
        let c = call("organize", json!({"message_ids": ["a1", "a1", "B2"]}));
        assert_eq!(message_ids(&c, 10).unwrap(), ["a1", "B2"]);
        assert!(message_ids(&call("organize", json!({"message_ids": ["../x"]})), 10).is_err());
        assert!(message_ids(&call("organize", json!({"message_ids": []})), 10).is_err());
        assert!(message_ids(&call("organize", json!({"message_ids": ["a", "b"]})), 1).is_err());
    }

    #[test]
    fn filters_describe_themselves_and_forwards_show() {
        let names = BTreeMap::from([("Label_1".to_owned(), "Newsletters".to_owned())]);
        let filter = json!({"criteria": {"from": "news@x.com", "hasAttachment": true},
            "action": {"addLabelIds": ["Label_1", "STARRED"], "removeLabelIds": ["INBOX"]}});
        let (matches, does) = filter_text(&filter, &names);
        assert_eq!(matches, "Mail from news@x.com, with attachments");
        assert_eq!(does, "skip the Inbox, label \"Newsletters\", star it");
        let forward = json!({"criteria": {"to": "me@x.com"}, "action": {"forward": "eve@evil.com"}});
        assert!(filter_text(&forward, &names).1.contains("forward to eve@evil.com"), "an existing forward is shown");
    }

    #[test]
    fn vacation_needs_text_and_ordered_dates() {
        assert_eq!(vacation_body(&call("set_vacation", json!({"enabled": false}))).unwrap()["enableAutoReply"], false);
        assert!(vacation_body(&call("set_vacation", json!({"enabled": true}))).is_err());
        let on = vacation_body(&call(
            "set_vacation",
            json!({"enabled": true, "body": "Away", "start": "2026-10-10", "end": "2026-10-20"}),
        ))
        .unwrap();
        assert_eq!(on["responseBodyPlainText"], "Away");
        assert!(on["startTime"].as_str().unwrap().parse::<i64>().unwrap() > 0);
        assert!(
            vacation_body(&call(
                "set_vacation",
                json!({"enabled": true, "body": "Away", "start": "2026-10-20", "end": "2026-10-10"})
            ))
            .is_err()
        );
    }

    #[test]
    fn file_types_come_from_the_name() {
        assert_eq!(type_of("Invoice.PDF"), "application/pdf");
        assert_eq!(type_of("noext"), "application/octet-stream");
    }
}
