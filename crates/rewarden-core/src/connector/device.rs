//! What lives on the phone itself: its calendars, its contacts and its text messages. The Kotlin side reads and writes
//! them through Android's own providers (after the user allowed that); this side decides what an AI may see.

use std::sync::Arc;

use rewarden_proto::connector::{ConnectorCall, DEVICE_CALENDAR, DEVICE_CONTACTS, SMS};
use serde_json::{Map, Value, json};

use super::{Connector, Item, Preview, looks_like_code};
use crate::store::unix_now;
use crate::types::GmailStatus;
use crate::{CoreError, ForeignError, text};

/// The one account every on-phone integration has.
pub const PHONE_ACCOUNT: &str = "this phone";

#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct DeviceEvent {
    pub id: String,
    pub calendar_id: String,
    pub calendar: String,
    pub title: String,
    /// Unix seconds.
    pub start: i64,
    pub end: i64,
    pub all_day: bool,
    pub location: String,
    pub description: String,
}

#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct NewDeviceEvent {
    pub title: String,
    pub start: i64,
    pub end: i64,
    pub all_day: bool,
    pub location: String,
    pub description: String,
}

#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct DeviceContact {
    pub id: String,
    pub name: String,
    pub organization: String,
    pub emails: Vec<String>,
    pub phones: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct SmsThread {
    pub id: String,
    /// The other party's number.
    pub address: String,
    /// Their name in the contacts, when there is one.
    pub name: String,
    pub snippet: String,
    pub date: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct SmsMessage {
    pub id: String,
    pub address: String,
    pub body: String,
    pub date: i64,
    pub outgoing: bool,
}

/// Implemented in Kotlin over Android's content providers. Every method may fail with `NeedsUserInteraction` when
/// the permission it needs has not been granted.
#[uniffi::export(with_foreign)]
pub trait DeviceBridge: Send + Sync {
    /// The on-phone integrations this build of the app offers ("device_calendar", "device_contacts", "sms"). The core
    /// runs only these; the others are listed as not available (the Google Play build has no text messages).
    fn services(&self) -> Vec<String>;
    /// Whether the permission for one on-phone integration ("device_calendar", "device_contacts", "sms") is granted.
    fn permitted(&self, service: String) -> bool;
    fn calendar_events(
        &self,
        from: i64,
        to: i64,
        query: Option<String>,
        limit: u32,
    ) -> Result<Vec<DeviceEvent>, ForeignError>;
    /// Adds the event to the phone's default writable calendar and returns its id.
    fn calendar_create(&self, event: NewDeviceEvent) -> Result<String, ForeignError>;
    fn contacts_search(&self, query: String, limit: u32) -> Result<Vec<DeviceContact>, ForeignError>;
    fn sms_threads(&self, limit: u32) -> Result<Vec<SmsThread>, ForeignError>;
    fn sms_messages(&self, thread: String, limit: u32) -> Result<Vec<SmsMessage>, ForeignError>;
    fn sms_send(&self, to: String, text: String) -> Result<(), ForeignError>;
}

fn map_foreign(e: ForeignError) -> CoreError {
    match e {
        ForeignError::NeedsUserInteraction => {
            CoreError::needs_attention("Rewarden needs permission on the phone for this")
        }
        ForeignError::Failed {
            reason,
        } => CoreError::service(text::one_line(&reason)),
    }
}

/// Runs a blocking call into Kotlin off the async threads.
async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, ForeignError> + Send + 'static,
) -> Result<T, CoreError> {
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|_| CoreError::service("the phone did not answer"))?
        .map_err(map_foreign)
}

fn limit(call: &ConnectorCall) -> u32 {
    u32::try_from(call.int_arg("limit").unwrap_or(20).clamp(1, 50)).unwrap_or(20)
}

fn status_of(bridge: &Arc<dyn DeviceBridge>, service: &str) -> GmailStatus {
    if bridge.permitted(service.to_owned()) {
        GmailStatus::Ready
    } else {
        GmailStatus::NeedsConsent
    }
}

/// `+1 (555) 010-0100` → `+15550100100`; anything that is not a phone number is refused.
pub fn normalize_number(raw: &str) -> Result<String, CoreError> {
    let trimmed = raw.trim();
    let plus = trimmed.starts_with('+');
    if trimmed.chars().any(|c| !(c.is_ascii_digit() || matches!(c, '+' | ' ' | '-' | '(' | ')' | '.')))
        || trimmed[usize::from(plus)..].contains('+')
    {
        return Err(CoreError::service("That is not a phone number."));
    }
    let digits: String = trimmed.chars().filter(char::is_ascii_digit).collect();
    if !(3..=15).contains(&digits.len()) {
        return Err(CoreError::service("That is not a phone number."));
    }
    Ok(if plus {
        format!("+{digits}")
    } else {
        digits
    })
}

fn parse_time(name: &str, raw: &str) -> Result<(i64, bool), CoreError> {
    let bare = !raw.contains(['T', 't', ' ']);
    text::parse_when(raw)
        .map(|t| (t, bare))
        .ok_or_else(|| CoreError::service(format!("`{name}` is not a date or time: {raw:?}.")))
}

// ---- calendar --------------------------------------------------------------------------------------------------------

pub struct DeviceCalendar {
    bridge: Arc<dyn DeviceBridge>,
}

impl DeviceCalendar {
    pub fn new(bridge: Arc<dyn DeviceBridge>) -> Self {
        Self {
            bridge,
        }
    }

    fn new_event(call: &ConnectorCall) -> Result<NewDeviceEvent, CoreError> {
        let (start, all_day) = parse_time("start", call.str_arg("start").unwrap_or_default())?;
        let end = match call.str_arg("end") {
            Some(end) => parse_time("end", end)?.0,
            None if all_day => start + 86_400,
            None => start + 3_600,
        };
        if end <= start {
            return Err(CoreError::service("The event must end after it starts."));
        }
        Ok(NewDeviceEvent {
            title: call.str_arg("title").unwrap_or_default().to_owned(),
            start,
            end,
            all_day,
            location: call.str_arg("location").unwrap_or_default().to_owned(),
            description: call.str_arg("description").unwrap_or_default().to_owned(),
        })
    }
}

#[async_trait::async_trait]
impl Connector for DeviceCalendar {
    fn service(&self) -> &'static str {
        DEVICE_CALENDAR
    }

    async fn fetch(&self, _account: &str, call: &ConnectorCall) -> Result<Vec<Item>, CoreError> {
        let from = call.str_arg("from").map_or(Ok(unix_now()), |t| parse_time("from", t).map(|p| p.0))?;
        let to = call.str_arg("to").map_or(Ok(from + 30 * 86_400), |t| parse_time("to", t).map(|p| p.0))?;
        let (query, n, bridge) = (call.str_arg("query").map(str::to_owned), limit(call), Arc::clone(&self.bridge));
        let events = blocking(move || bridge.calendar_events(from, to, query, n)).await?;
        Ok(events
            .into_iter()
            .map(|e| {
                let mut extra = Map::new();
                extra.insert("start".to_owned(), json!(text::iso_utc(e.start)));
                extra.insert("end".to_owned(), json!(text::iso_utc(e.end)));
                extra.insert("all_day".to_owned(), json!(e.all_day));
                if !e.location.is_empty() {
                    extra.insert("location".to_owned(), json!(text::one_line(&e.location)));
                }
                Item {
                    id: e.id,
                    resource: e.calendar_id,
                    resource_label: text::one_line(&e.calendar),
                    from: text::one_line(&e.calendar),
                    title: text::one_line(&e.title),
                    snippet: [text::iso_utc(e.start), text::one_line(&e.location)]
                        .into_iter()
                        .filter(|s| !s.is_empty())
                        .collect::<Vec<_>>()
                        .join(" · "),
                    date: e.start,
                    body: Some(e.description).filter(|d| !d.is_empty()),
                    sensitive: false,
                    secret: false,
                    parents: Vec::new(),
                    extra,
                }
            })
            .collect())
    }

    async fn preview(&self, _account: &str, call: &ConnectorCall) -> Result<Preview, CoreError> {
        if call.op != "create_event" {
            return Err(CoreError::service("the phone's calendar cannot do that"));
        }
        let e = Self::new_event(call)?;
        let when = if e.all_day {
            format!("{} (all day)", text::iso_utc(e.start)[..10].to_owned())
        } else {
            format!("{} to {}", text::iso_utc(e.start), text::iso_utc(e.end))
        };
        let mut lines = vec![format!("Add to the phone's calendar: {}", e.title), when];
        if !e.location.is_empty() {
            lines.push(format!("Where: {}", e.location));
        }
        if !e.description.is_empty() {
            lines.push(text::truncate_chars(&e.description, 300));
        }
        Ok(Preview {
            resource: "default".to_owned(),
            resource_label: "The phone's calendar".to_owned(),
            lines,
            ..Preview::default()
        })
    }

    async fn perform(&self, _account: &str, call: &ConnectorCall) -> Result<Value, CoreError> {
        if call.op != "create_event" {
            return Err(CoreError::service("the phone's calendar cannot do that"));
        }
        let event = Self::new_event(call)?;
        let bridge = Arc::clone(&self.bridge);
        let id = blocking(move || bridge.calendar_create(event)).await?;
        Ok(json!({"created": true, "event_id": id}))
    }

    async fn status(&self, _account: &str) -> GmailStatus {
        status_of(&self.bridge, DEVICE_CALENDAR)
    }
}

// ---- contacts --------------------------------------------------------------------------------------------------------

pub struct DeviceContacts {
    bridge: Arc<dyn DeviceBridge>,
}

impl DeviceContacts {
    pub fn new(bridge: Arc<dyn DeviceBridge>) -> Self {
        Self {
            bridge,
        }
    }
}

#[async_trait::async_trait]
impl Connector for DeviceContacts {
    fn service(&self) -> &'static str {
        DEVICE_CONTACTS
    }

    async fn fetch(&self, _account: &str, call: &ConnectorCall) -> Result<Vec<Item>, CoreError> {
        let (query, n, bridge) =
            (call.str_arg("query").unwrap_or_default().to_owned(), limit(call), Arc::clone(&self.bridge));
        let found = blocking(move || bridge.contacts_search(query, n)).await?;
        Ok(found
            .into_iter()
            .map(|c| {
                let mut extra = Map::new();
                extra.insert("emails".to_owned(), json!(c.emails));
                extra.insert("phones".to_owned(), json!(c.phones));
                Item {
                    id: c.id,
                    resource: "contacts".to_owned(),
                    resource_label: "Contacts".to_owned(),
                    from: text::one_line(&c.organization),
                    title: text::one_line(&c.name),
                    snippet: text::one_line(
                        &c.emails.iter().chain(c.phones.iter()).cloned().collect::<Vec<_>>().join(" · "),
                    ),
                    extra,
                    ..Item::default()
                }
            })
            .collect())
    }

    async fn status(&self, _account: &str) -> GmailStatus {
        status_of(&self.bridge, DEVICE_CONTACTS)
    }
}

// ---- text messages ---------------------------------------------------------------------------------------------------

pub struct Sms {
    bridge: Arc<dyn DeviceBridge>,
}

impl Sms {
    pub fn new(bridge: Arc<dyn DeviceBridge>) -> Self {
        Self {
            bridge,
        }
    }
}

#[async_trait::async_trait]
impl Connector for Sms {
    fn service(&self) -> &'static str {
        SMS
    }

    async fn fetch(&self, _account: &str, call: &ConnectorCall) -> Result<Vec<Item>, CoreError> {
        let (n, bridge) = (limit(call), Arc::clone(&self.bridge));
        match call.op.as_str() {
            "list_threads" => {
                let threads = blocking(move || bridge.sms_threads(n)).await?;
                Ok(threads
                    .into_iter()
                    .map(|t| {
                        let who = if t.name.is_empty() {
                            t.address.clone()
                        } else {
                            t.name.clone()
                        };
                        let mut extra = Map::new();
                        extra.insert("number".to_owned(), json!(t.address));
                        Item {
                            id: t.id.clone(),
                            resource: t.id,
                            resource_label: text::one_line(&who),
                            from: text::one_line(&t.address),
                            title: text::one_line(&who),
                            // The last text may be a login code; a list never shows it.
                            snippet: if looks_like_code(&t.snippet) {
                                String::new()
                            } else {
                                text::truncate_chars(&text::one_line(&t.snippet), 100)
                            },
                            date: t.date,
                            extra,
                            ..Item::default()
                        }
                    })
                    .collect())
            }
            "read" => {
                let thread = call.str_arg("thread").unwrap_or_default().to_owned();
                let label = thread.clone();
                let messages = blocking(move || bridge.sms_messages(thread, n)).await?;
                Ok(messages
                    .into_iter()
                    .map(|m| Item {
                        id: format!("{label}:{}", m.id),
                        resource: label.clone(),
                        resource_label: text::one_line(&m.address),
                        from: if m.outgoing {
                            "me".to_owned()
                        } else {
                            text::one_line(&m.address)
                        },
                        snippet: text::truncate_chars(&text::one_line(&m.body), 160),
                        sensitive: !m.outgoing && looks_like_code(&m.body),
                        body: Some(m.body),
                        date: m.date,
                        ..Item::default()
                    })
                    .collect())
            }
            other => Err(CoreError::service(format!("text messages cannot {other}"))),
        }
    }

    async fn preview(&self, _account: &str, call: &ConnectorCall) -> Result<Preview, CoreError> {
        let to = normalize_number(call.str_arg("to").unwrap_or_default())?;
        Ok(Preview {
            resource: to.clone(),
            resource_label: to.clone(),
            lines: vec![format!("Text {to}"), call.str_arg("text").unwrap_or_default().to_owned()],
            ..Preview::default()
        })
    }

    async fn perform(&self, _account: &str, call: &ConnectorCall) -> Result<Value, CoreError> {
        let to = normalize_number(call.str_arg("to").unwrap_or_default())?;
        let (body, bridge) = (call.str_arg("text").unwrap_or_default().to_owned(), Arc::clone(&self.bridge));
        let number = to.clone();
        blocking(move || bridge.sms_send(number, body)).await?;
        Ok(json!({"sent": true, "to": to}))
    }

    async fn status(&self, _account: &str) -> GmailStatus {
        status_of(&self.bridge, SMS)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn phone_numbers_are_normalised_and_junk_is_refused() {
        assert_eq!(normalize_number(" +1 (555) 010-0100 ").unwrap(), "+15550100100");
        assert_eq!(normalize_number("555.0100").unwrap(), "5550100");
        for bad in ["", "12", "abc", "+1+2", "1234567890123456", "555;rm", "5550100 x"] {
            assert!(normalize_number(bad).is_err(), "{bad:?}");
        }
    }
}
