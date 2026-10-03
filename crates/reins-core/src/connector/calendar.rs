//! Google Calendar and Google Contacts, through their REST APIs with the phone's own Google authorization.

use std::sync::Arc;
use std::time::Duration;

use reins_proto::connector::{ConnectorCall, GCALENDAR, GCONTACTS};
use reqwest::Method;
use serde::Deserialize;
use serde_json::{Map, Value, json};

use super::{Connector, Item, Preview};
use crate::google::GoogleApi;
use crate::store::unix_now;
use crate::types::GmailStatus;
use crate::{CoreError, GoogleTokenProvider, text};

pub const CALENDAR_BASE: &str = "https://www.googleapis.com/calendar/v3";
pub const PEOPLE_BASE: &str = "https://people.googleapis.com/v1";

/// The calendar id the AI leaves out or calls "primary".
const PRIMARY: &str = "primary";

/// Percent-encodes one path segment (calendar ids contain `@` and `#`).
pub(crate) fn segment(raw: &str) -> String {
    raw.bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
}

pub(crate) fn limit(call: &ConnectorCall) -> usize {
    usize::try_from(call.int_arg("limit").unwrap_or(20).clamp(1, 50)).unwrap_or(20)
}

/// Whether a time carries its own zone (`Z` or `+02:00`), which an AI must give for a moment on the clock.
fn has_zone(when: &str) -> bool {
    let Some((_, clock)) = when.split_once(['T', 't', ' ']) else {
        return true;
    };
    clock.ends_with(['Z', 'z']) || clock.rfind(['+', '-']).is_some_and(|i| i > 0)
}

/// A date or moment from the AI → (unix seconds, is a bare date).
fn when(name: &str, text_: &str) -> Result<(i64, bool), CoreError> {
    let bare = !text_.contains(['T', 't', ' ']);
    if !has_zone(text_) {
        return Err(CoreError::service(format!(
            "`{name}` needs a time zone: write it like 2026-10-05T14:00:00+02:00 or ...Z."
        )));
    }
    let secs = text::parse_when(text_)
        .ok_or_else(|| CoreError::service(format!("`{name}` is not a date or time: {text_:?}.")))?;
    Ok((secs, bare))
}

fn date_only(unix: i64) -> String {
    text::iso_utc(unix)[..10].to_owned()
}

pub struct GoogleCalendar {
    http: reqwest::Client,
    base: String,
    token: Arc<dyn GoogleTokenProvider>,
    backoff: Duration,
}

impl GoogleCalendar {
    pub fn new(http: reqwest::Client, base: &str, token: Arc<dyn GoogleTokenProvider>, backoff: Duration) -> Self {
        Self {
            http,
            base: base.to_owned(),
            token,
            backoff,
        }
    }

    fn api(&self, account: &str) -> GoogleApi {
        GoogleApi::new(self.http.clone(), &self.base, Arc::clone(&self.token), account, GCALENDAR, self.backoff)
    }

    /// The address a calendar sign-in belongs to (the id of the primary calendar).
    pub async fn identify(&self, hint: &str) -> Result<String, CoreError> {
        #[derive(Deserialize)]
        struct Calendar {
            id: String,
        }
        let text = self.api(hint).request(&Method::GET, "/calendars/primary", &[], None).await?;
        let calendar: Calendar =
            serde_json::from_str(&text).map_err(|_| CoreError::service("Google Calendar sent an unexpected answer"))?;
        Ok(calendar.id.trim().to_lowercase())
    }

    /// A calendar's name for showing, falling back to its id.
    async fn calendar_name(&self, api: &GoogleApi, id: &str) -> String {
        if id == PRIMARY {
            return "Main calendar".to_owned();
        }
        let path = format!("/users/me/calendarList/{}", segment(id));
        match api.request(&Method::GET, &path, &[], None).await {
            Ok(body) => serde_json::from_str::<Value>(&body)
                .ok()
                .and_then(|v| v["summaryOverride"].as_str().or_else(|| v["summary"].as_str()).map(str::to_owned))
                .unwrap_or_else(|| id.to_owned()),
            Err(_) => id.to_owned(),
        }
    }

    fn event_item(calendar: &str, label: &str, event: &Value) -> Item {
        let moment = |v: &Value| -> (i64, String) {
            if let Some(dt) = v["dateTime"].as_str() {
                (text::parse_when(dt).unwrap_or(0), dt.to_owned())
            } else if let Some(d) = v["date"].as_str() {
                (text::parse_when(d).unwrap_or(0), d.to_owned())
            } else {
                (0, String::new())
            }
        };
        let (start, start_text) = moment(&event["start"]);
        let (_, end_text) = moment(&event["end"]);
        let title = event["summary"].as_str().unwrap_or("(no title)");
        let location = event["location"].as_str().unwrap_or_default();
        let attendees: Vec<String> = event["attendees"]
            .as_array()
            .map(|a| a.iter().filter_map(|p| p["email"].as_str()).map(str::to_owned).collect())
            .unwrap_or_default();
        let mut extra = Map::new();
        extra.insert("start".to_owned(), json!(start_text));
        extra.insert("end".to_owned(), json!(end_text));
        if !location.is_empty() {
            extra.insert("location".to_owned(), json!(text::one_line(location)));
        }
        if !attendees.is_empty() {
            extra.insert("attendees".to_owned(), json!(attendees));
        }
        if let Some(link) = event["htmlLink"].as_str() {
            extra.insert("link".to_owned(), json!(link));
        }
        let mut body = String::new();
        if let Some(description) = event["description"].as_str() {
            body.push_str(description);
        }
        let snippet =
            [start_text.as_str(), location].iter().filter(|s| !s.is_empty()).copied().collect::<Vec<_>>().join(" · ");
        Item {
            id: event["id"].as_str().unwrap_or_default().to_owned(),
            resource: calendar.to_owned(),
            resource_label: label.to_owned(),
            from: event["organizer"]["displayName"]
                .as_str()
                .or_else(|| event["organizer"]["email"].as_str())
                .unwrap_or_default()
                .to_owned(),
            title: text::one_line(title),
            snippet,
            date: start,
            body: Some(body).filter(|b| !b.is_empty()),
            sensitive: false,
            secret: false,
            parents: Vec::new(),
            extra,
        }
    }

    fn event_body(call: &ConnectorCall) -> Result<Value, CoreError> {
        let title = call.str_arg("title").unwrap_or_default();
        let (start, start_bare) = when("start", call.str_arg("start").unwrap_or_default())?;
        let (end, end_bare) = match call.str_arg("end") {
            Some(end) => when("end", end)?,
            None if start_bare => (start + 86_400, true),
            None => (start + 3_600, false),
        };
        if end_bare != start_bare {
            return Err(CoreError::service("`start` and `end` must both be dates or both be times."));
        }
        if end <= start {
            return Err(CoreError::service("The event must end after it starts."));
        }
        let moment = |secs: i64| {
            if start_bare {
                json!({"date": date_only(secs)})
            } else {
                json!({"dateTime": text::iso_utc(secs)})
            }
        };
        let mut body = json!({"summary": title, "start": moment(start), "end": moment(end)});
        if let Some(description) = call.str_arg("description") {
            body["description"] = json!(description);
        }
        if let Some(location) = call.str_arg("location") {
            body["location"] = json!(location);
        }
        let attendees = call.list_arg("attendees");
        if !attendees.is_empty() {
            let checked: Result<Vec<Value>, _> = attendees
                .iter()
                .map(|a| reins_proto::normalize_address(a).map(|email| json!({"email": email})))
                .collect();
            body["attendees"] = json!(checked.map_err(|e| CoreError::service(e.to_string()))?);
        }
        Ok(body)
    }

    fn calendar_arg(call: &ConnectorCall) -> String {
        call.str_arg("calendar").filter(|c| !c.is_empty()).unwrap_or(PRIMARY).to_owned()
    }
}

#[async_trait::async_trait]
impl Connector for GoogleCalendar {
    fn service(&self) -> &'static str {
        GCALENDAR
    }

    async fn identify(&self, hint: &str) -> Result<String, CoreError> {
        Self::identify(self, hint).await
    }

    async fn fetch(&self, account: &str, call: &ConnectorCall) -> Result<Vec<Item>, CoreError> {
        let api = self.api(account);
        match call.op.as_str() {
            "list_calendars" => {
                let body = api
                    .request(&Method::GET, "/users/me/calendarList", &[("maxResults", "100".to_owned())], None)
                    .await?;
                let v: Value = serde_json::from_str(&body)
                    .map_err(|_| CoreError::service("Google Calendar sent an unexpected answer"))?;
                Ok(v["items"]
                    .as_array()
                    .map(|list| {
                        list.iter()
                            .map(|c| {
                                let id = if c["primary"].as_bool() == Some(true) {
                                    PRIMARY
                                } else {
                                    c["id"].as_str().unwrap_or_default()
                                };
                                let name = c["summaryOverride"]
                                    .as_str()
                                    .or_else(|| c["summary"].as_str())
                                    .unwrap_or("(unnamed)");
                                Item {
                                    id: id.to_owned(),
                                    resource: id.to_owned(),
                                    resource_label: text::one_line(name),
                                    title: text::one_line(name),
                                    from: c["accessRole"].as_str().unwrap_or_default().to_owned(),
                                    ..Item::default()
                                }
                            })
                            .collect()
                    })
                    .unwrap_or_default())
            }
            "list_events" => {
                let calendar = Self::calendar_arg(call);
                let now = unix_now();
                let from = call.str_arg("from").map_or(Ok(now), |t| when("from", t).map(|w| w.0))?;
                let to = call.str_arg("to").map_or(Ok(from + 30 * 86_400), |t| when("to", t).map(|w| w.0))?;
                let mut query = vec![
                    ("timeMin", text::iso_utc(from)),
                    ("timeMax", text::iso_utc(to)),
                    ("singleEvents", "true".to_owned()),
                    ("orderBy", "startTime".to_owned()),
                    ("maxResults", limit(call).to_string()),
                ];
                if let Some(q) = call.str_arg("query") {
                    query.push(("q", q.to_owned()));
                }
                let body = api
                    .request(&Method::GET, &format!("/calendars/{}/events", segment(&calendar)), &query, None)
                    .await?;
                let v: Value = serde_json::from_str(&body)
                    .map_err(|_| CoreError::service("Google Calendar sent an unexpected answer"))?;
                let label = self.calendar_name(&api, &calendar).await;
                Ok(v["items"]
                    .as_array()
                    .map(|events| {
                        events
                            .iter()
                            .filter(|e| e["status"] != "cancelled")
                            .map(|e| Self::event_item(&calendar, &label, e))
                            .collect()
                    })
                    .unwrap_or_default())
            }
            other => Err(CoreError::service(format!("Google Calendar cannot {other}"))),
        }
    }

    async fn preview(&self, account: &str, call: &ConnectorCall) -> Result<Preview, CoreError> {
        let api = self.api(account);
        let calendar = Self::calendar_arg(call);
        let label = self.calendar_name(&api, &calendar).await;
        match call.op.as_str() {
            "create_event" => {
                let body = Self::event_body(call)?;
                let when_line = if let Some(d) = body["start"]["date"].as_str() {
                    format!("{d} (all day)")
                } else {
                    format!(
                        "{} to {}",
                        body["start"]["dateTime"].as_str().unwrap_or_default(),
                        body["end"]["dateTime"].as_str().unwrap_or_default()
                    )
                };
                let mut lines =
                    vec![format!("Add to {label}: {}", body["summary"].as_str().unwrap_or_default()), when_line];
                if let Some(l) = body["location"].as_str() {
                    lines.push(format!("Where: {l}"));
                }
                if let Some(attendees) = body["attendees"].as_array() {
                    let names: Vec<&str> = attendees.iter().filter_map(|a| a["email"].as_str()).collect();
                    lines.push(format!("Invites: {}", names.join(", ")));
                }
                if let Some(d) = body["description"].as_str() {
                    lines.push(text::truncate_chars(d, 300));
                }
                Ok(Preview {
                    resource: calendar,
                    resource_label: label,
                    lines,
                    ..Preview::default()
                })
            }
            "delete_event" => {
                let id = call.str_arg("event_id").unwrap_or_default();
                let path = format!("/calendars/{}/events/{}", segment(&calendar), segment(id));
                let body = api.request(&Method::GET, &path, &[], None).await?;
                let v: Value = serde_json::from_str(&body)
                    .map_err(|_| CoreError::service("Google Calendar sent an unexpected answer"))?;
                let item = Self::event_item(&calendar, &label, &v);
                Ok(Preview {
                    resource: calendar,
                    resource_label: label.clone(),
                    lines: vec![format!("Delete from {label}: {}", item.title), item.snippet],
                    ..Preview::default()
                })
            }
            other => Err(CoreError::service(format!("Google Calendar cannot {other}"))),
        }
    }

    async fn perform(&self, account: &str, call: &ConnectorCall) -> Result<Value, CoreError> {
        let api = self.api(account);
        let calendar = Self::calendar_arg(call);
        match call.op.as_str() {
            "create_event" => {
                let body = Self::event_body(call)?;
                let query = if body.get("attendees").is_some() {
                    vec![("sendUpdates", "all".to_owned())]
                } else {
                    Vec::new()
                };
                let path = format!("/calendars/{}/events", segment(&calendar));
                let created = api.request(&Method::POST, &path, &query, Some(&body)).await?;
                let v: Value = serde_json::from_str(&created).unwrap_or(Value::Null);
                Ok(json!({"created": true, "event_id": v["id"], "link": v["htmlLink"]}))
            }
            "delete_event" => {
                let id = call.str_arg("event_id").unwrap_or_default();
                let path = format!("/calendars/{}/events/{}", segment(&calendar), segment(id));
                api.request(&Method::DELETE, &path, &[], None).await?;
                Ok(json!({"deleted": true}))
            }
            other => Err(CoreError::service(format!("Google Calendar cannot {other}"))),
        }
    }

    async fn status(&self, account: &str) -> GmailStatus {
        match self
            .api(account)
            .request(&Method::GET, "/users/me/calendarList", &[("maxResults", "1".to_owned())], None)
            .await
        {
            Ok(_) => GmailStatus::Ready,
            Err(CoreError::GmailNeedsConsent) => GmailStatus::NeedsConsent,
            Err(e) => GmailStatus::Unavailable {
                message: e.to_string(),
            },
        }
    }
}

pub struct GoogleContacts {
    http: reqwest::Client,
    base: String,
    token: Arc<dyn GoogleTokenProvider>,
    backoff: Duration,
}

impl GoogleContacts {
    pub fn new(http: reqwest::Client, base: &str, token: Arc<dyn GoogleTokenProvider>, backoff: Duration) -> Self {
        Self {
            http,
            base: base.to_owned(),
            token,
            backoff,
        }
    }

    fn api(&self, account: &str) -> GoogleApi {
        GoogleApi::new(self.http.clone(), &self.base, Arc::clone(&self.token), account, GCONTACTS, self.backoff)
    }

    /// Checks that the contacts can be read with this sign-in. The account is the address the phone authorized: the
    /// contacts permission does not cover the profile (where Google would name the address), and the token is bound to
    /// that address anyway.
    pub async fn identify(&self, hint: &str) -> Result<String, CoreError> {
        let account = hint.trim().to_lowercase();
        if !account.contains('@') {
            return Err(CoreError::invalid("choose a Google account"));
        }
        self.probe(&account).await?;
        Ok(account)
    }

    async fn probe(&self, account: &str) -> Result<(), CoreError> {
        self.api(account)
            .request(
                &Method::GET,
                "/people/me/connections",
                &[("personFields", "names".to_owned()), ("pageSize", "1".to_owned())],
                None,
            )
            .await
            .map(|_| ())
    }
}

#[async_trait::async_trait]
impl Connector for GoogleContacts {
    fn service(&self) -> &'static str {
        GCONTACTS
    }

    async fn fetch(&self, account: &str, call: &ConnectorCall) -> Result<Vec<Item>, CoreError> {
        if call.op != "search" {
            return Err(CoreError::service("Google Contacts cannot do that"));
        }
        let api = self.api(account);
        let query = call.str_arg("query").unwrap_or_default().to_owned();
        let params = vec![
            ("query", query),
            ("readMask", "names,emailAddresses,phoneNumbers,organizations".to_owned()),
            ("pageSize", limit(call).min(30).to_string()),
        ];
        let mut body = api.request(&Method::GET, "/people:searchContacts", &params, None).await?;
        let mut v: Value =
            serde_json::from_str(&body).map_err(|_| CoreError::service("Google Contacts sent an unexpected answer"))?;
        if v["results"].as_array().is_none_or(Vec::is_empty) {
            // Google builds its search cache on the first empty-query request; the answer before that can be empty.
            let warm = [("query", String::new()), ("readMask", "names".to_owned()), ("pageSize", "1".to_owned())];
            api.request(&Method::GET, "/people:searchContacts", &warm, None).await.ok();
            body = api.request(&Method::GET, "/people:searchContacts", &params, None).await?;
            v = serde_json::from_str(&body)
                .map_err(|_| CoreError::service("Google Contacts sent an unexpected answer"))?;
        }
        Ok(v["results"]
            .as_array()
            .map(|results| {
                results
                    .iter()
                    .map(|r| {
                        let p = &r["person"];
                        let emails = strings(&p["emailAddresses"], "value");
                        let phones = strings(&p["phoneNumbers"], "value");
                        let name = p["names"][0]["displayName"].as_str().unwrap_or("(no name)");
                        let org = p["organizations"][0]["name"].as_str().unwrap_or_default();
                        let mut extra = Map::new();
                        extra.insert("emails".to_owned(), json!(emails));
                        extra.insert("phones".to_owned(), json!(phones));
                        Item {
                            id: p["resourceName"].as_str().unwrap_or_default().to_owned(),
                            resource: "contacts".to_owned(),
                            resource_label: "Contacts".to_owned(),
                            from: text::one_line(org),
                            title: text::one_line(name),
                            snippet: text::one_line(
                                &emails.iter().chain(phones.iter()).cloned().collect::<Vec<_>>().join(" · "),
                            ),
                            extra,
                            ..Item::default()
                        }
                    })
                    .collect()
            })
            .unwrap_or_default())
    }

    async fn identify(&self, hint: &str) -> Result<String, CoreError> {
        Self::identify(self, hint).await
    }

    async fn status(&self, account: &str) -> GmailStatus {
        match self.probe(account).await {
            Ok(()) => GmailStatus::Ready,
            Err(CoreError::GmailNeedsConsent) => GmailStatus::NeedsConsent,
            Err(e) => GmailStatus::Unavailable {
                message: e.to_string(),
            },
        }
    }
}

fn strings(list: &Value, key: &str) -> Vec<String> {
    list.as_array().map(|a| a.iter().filter_map(|v| v[key].as_str()).map(text::one_line).collect()).unwrap_or_default()
}
