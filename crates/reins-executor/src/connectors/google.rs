//! The org's Google Workspace service account (domain-wide delegation). It acts as the requesting member only: the
//! token's `sub` is that member's email from the signed roster, never anything the request says, and each token carries
//! only the scope its tool needs.

use ring::signature::RsaKeyPair;
use serde_json::{Map, Value, json};
use zeroize::Zeroizing;

use super::{
    Connector, arg_list, arg_str, format_date, format_rfc3339, http_client, jwt_rs256, only_args, parse_date,
    parse_rfc3339, required_str,
};

/// The tools the Google Workspace connector runs.
pub const TOOLS: [&str; 3] = ["gmail_send", "calendar_list_events", "calendar_create_event"];

const SCOPE_SEND: &str = "https://www.googleapis.com/auth/gmail.send";
const SCOPE_CALENDAR_READ: &str = "https://www.googleapis.com/auth/calendar.readonly";
const SCOPE_CALENDAR_EVENTS: &str = "https://www.googleapis.com/auth/calendar.events";

pub struct GoogleWorkspace {
    client_email: String,
    key: RsaKeyPair,
    token_uri: String,
    api_base: String,
    http: reqwest::Client,
}

/// A plain email address: no display name, nothing a header could be split with.
fn address_ok(a: &str) -> bool {
    let Some((local, domain)) = a.split_once('@') else {
        return false;
    };
    !local.is_empty()
        && domain.contains('.')
        && !domain.contains('@')
        && a.len() <= 254
        && !a.chars().any(|c| c.is_whitespace() || c.is_control() || "<>,;:\"()[]\\".contains(c))
}

/// A header value: RFC 2047 encoded when it isn't plain ASCII.
fn header_text(s: &str) -> String {
    if s.bytes().all(|b| (0x20..0x7f).contains(&b)) {
        s.to_owned()
    } else {
        format!("=?UTF-8?B?{}?=", data_encoding::BASE64.encode(s.as_bytes()))
    }
}

/// The RFC 822 message of an approved `gmail_send`, built from `to`, `cc`, `subject` and `body` only.
pub fn rfc822(from: &str, args: &Value) -> Result<String, String> {
    only_args(args, &["to", "cc", "subject", "body"])?;
    let to = arg_list(args, "to")?;
    let cc = arg_list(args, "cc")?;
    if to.is_empty() {
        return Err("`to` needs at least one recipient".to_owned());
    }
    if let Some(bad) = to.iter().chain(&cc).find(|a| !address_ok(a)) {
        return Err(format!("{bad:?} is not a plain email address"));
    }
    let subject = arg_str(args, "subject").ok_or("`subject` is required")?;
    if subject.chars().any(char::is_control) {
        return Err("the subject has control characters".to_owned());
    }
    let body = arg_str(args, "body").ok_or("`body` is required")?;
    let cc_line = if cc.is_empty() {
        String::new()
    } else {
        format!("Cc: {}\r\n", cc.join(", "))
    };
    let mut m = format!(
        "From: {from}\r\nTo: {}\r\n{cc_line}Subject: {}\r\nMIME-Version: 1.0\r\nContent-Type: text/plain; \
         charset=UTF-8\r\nContent-Transfer-Encoding: base64\r\n\r\n",
        to.join(", "),
        header_text(subject)
    );
    let encoded = data_encoding::BASE64.encode(body.as_bytes());
    for line in encoded.as_bytes().chunks(76) {
        m.push_str(std::str::from_utf8(line).unwrap_or_default());
        m.push_str("\r\n");
    }
    Ok(m)
}

/// A `from`/`to` bound as Google wants it (RFC 3339): a date is its midnight UTC.
fn bound(s: &str) -> Result<String, String> {
    if let Some(days) = parse_date(s) {
        return Ok(format_rfc3339(days * 86_400));
    }
    parse_rfc3339(s).map(format_rfc3339).ok_or_else(|| format!("{s:?} is not a date or a date and time"))
}

/// An event's start or end: `{date}` for a whole day, `{dateTime}` otherwise.
fn when(s: &str) -> Result<(Value, i64, bool), String> {
    if let Some(days) = parse_date(s) {
        return Ok((json!({"date": format_date(days)}), days, true));
    }
    let t = parse_rfc3339(s).ok_or_else(|| format!("{s:?} is not a date or a date and time"))?;
    Ok((json!({"dateTime": s}), t, false))
}

fn calendar_id(args: &Value) -> Result<String, String> {
    let id = arg_str(args, "calendar").filter(|s| !s.is_empty()).unwrap_or("primary");
    if id.len() > 200 || id.chars().any(char::is_control) {
        return Err("`calendar` is not a calendar id".to_owned());
    }
    Ok(id.to_owned())
}

impl GoogleWorkspace {
    pub fn new(client_email: &str, key_pem: &str, token_uri: &str, api_base: &str) -> Result<Self, String> {
        Ok(Self {
            client_email: client_email.to_owned(),
            key: super::rsa_from_pem(key_pem)?,
            token_uri: token_uri.to_owned(),
            api_base: api_base.trim_end_matches('/').to_owned(),
            http: http_client()?,
        })
    }

    /// From a service account's JSON key; `token_uri` overrides the key's.
    pub fn from_service_account(json_key: &str, api_base: &str, token_uri: Option<&str>) -> Result<Self, String> {
        let v: Value = serde_json::from_str(json_key).map_err(|_| "the service account key is not JSON".to_owned())?;
        let field = |k: &str| v.get(k).and_then(Value::as_str).ok_or_else(|| format!("the key has no {k}"));
        let pem = Zeroizing::new(field("private_key")?.to_owned());
        let uri = match token_uri {
            Some(u) => u.to_owned(),
            None => field("token_uri")?.to_owned(),
        };
        Self::new(field("client_email")?, &pem, &uri, api_base)
    }

    /// An access token acting as `member_email`, with `scope` only (the JWT bearer grant, RFC 7523).
    pub async fn token(&self, member_email: &str, scope: &str) -> Result<Zeroizing<String>, String> {
        let now = crate::now();
        let assertion = jwt_rs256(
            &self.key,
            &json!({
                "iss": self.client_email,
                "sub": member_email,
                "scope": scope,
                "aud": self.token_uri,
                "iat": now,
                "exp": now + 3600,
            }),
        )?;
        let resp = self
            .http
            .post(&self.token_uri)
            .form(&[("grant_type", "urn:ietf:params:oauth:grant-type:jwt-bearer"), ("assertion", assertion.as_str())])
            .send()
            .await
            .map_err(|e| format!("Google: {}", e.without_url()))?;
        if !resp.status().is_success() {
            return Err(format!("Google refused the service account token ({})", resp.status()));
        }
        let v: Value = resp.json().await.map_err(|_| "Google's token answer is not JSON".to_owned())?;
        v.get("access_token")
            .and_then(Value::as_str)
            .map(|t| Zeroizing::new(t.to_owned()))
            .ok_or_else(|| "Google's token answer has no access token".to_owned())
    }

    async fn send(&self, req: reqwest::RequestBuilder) -> Result<Value, String> {
        let resp = req.send().await.map_err(|e| format!("Google: {}", e.without_url()))?;
        let status = resp.status();
        let v: Value = resp.json().await.unwrap_or(Value::Null);
        if !status.is_success() {
            let msg = v.pointer("/error/message").and_then(Value::as_str).unwrap_or_default();
            return Err(format!("Google answered {status} {msg}").trim_end().to_owned());
        }
        Ok(v)
    }

    fn calendar_url(&self, calendar: &str) -> Result<reqwest::Url, String> {
        let mut url = reqwest::Url::parse(&self.api_base).map_err(|_| "the Google API base is not a URL".to_owned())?;
        url.path_segments_mut().map_err(|()| "the Google API base is not a URL".to_owned())?.pop_if_empty().extend([
            "calendar",
            "v3",
            "calendars",
            calendar,
            "events",
        ]);
        Ok(url)
    }

    async fn gmail_send(&self, member: &str, args: &Value) -> Result<Value, String> {
        let raw = rfc822(member, args)?;
        let token = self.token(member, SCOPE_SEND).await?;
        let v = self
            .send(
                self.http
                    .post(format!("{}/gmail/v1/users/me/messages/send", self.api_base))
                    .bearer_auth(token.as_str())
                    .json(&json!({"raw": data_encoding::BASE64URL_NOPAD.encode(raw.as_bytes())})),
            )
            .await?;
        Ok(json!({"id": v.get("id"), "thread_id": v.get("threadId")}))
    }

    async fn list_events(&self, member: &str, args: &Value) -> Result<Value, String> {
        only_args(args, &["calendar", "from", "to", "query", "limit"])?;
        let url = self.calendar_url(&calendar_id(args)?)?;
        let limit = match args.get("limit") {
            None | Some(Value::Null) => 20,
            Some(v) => v.as_i64().filter(|n| (1..=50).contains(n)).ok_or("`limit` is 1 to 50")?,
        };
        let mut query = vec![
            ("singleEvents".to_owned(), "true".to_owned()),
            ("orderBy".to_owned(), "startTime".to_owned()),
            ("maxResults".to_owned(), limit.to_string()),
        ];
        let from = match arg_str(args, "from") {
            Some(s) => bound(s)?,
            None => format_rfc3339(crate::now()),
        };
        query.push(("timeMin".to_owned(), from));
        if let Some(s) = arg_str(args, "to") {
            query.push(("timeMax".to_owned(), bound(s)?));
        }
        if let Some(q) = arg_str(args, "query").filter(|q| !q.is_empty()) {
            query.push(("q".to_owned(), q.to_owned()));
        }
        let token = self.token(member, SCOPE_CALENDAR_READ).await?;
        let v = self.send(self.http.get(url).bearer_auth(token.as_str()).query(&query)).await?;
        let events: Vec<Value> = v
            .get("items")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .map(|e| {
                        let at = |k: &str| e.get(k).and_then(|s| s.get("dateTime").or_else(|| s.get("date"))).cloned();
                        json!({
                            "id": e.get("id"),
                            "title": e.get("summary"),
                            "start": at("start"),
                            "end": at("end"),
                            "location": e.get("location"),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        Ok(json!({"events": events}))
    }

    async fn create_event(&self, member: &str, args: &Value) -> Result<Value, String> {
        only_args(args, &["title", "start", "end", "calendar", "description", "location", "attendees"])?;
        let url = self.calendar_url(&calendar_id(args)?)?;
        let (start, at, whole_day) = when(required_str(args, "start")?)?;
        let end = match arg_str(args, "end") {
            Some(e) => when(e)?.0,
            None if whole_day => json!({"date": format_date(at + 1)}),
            None => json!({"dateTime": format_rfc3339(at + 3600)}),
        };
        let mut event = Map::new();
        event.insert("summary".into(), json!(required_str(args, "title")?));
        event.insert("start".into(), start);
        event.insert("end".into(), end);
        for (arg, field) in [("description", "description"), ("location", "location")] {
            if let Some(v) = arg_str(args, arg) {
                event.insert(field.into(), json!(v));
            }
        }
        let attendees = arg_list(args, "attendees")?;
        if let Some(bad) = attendees.iter().find(|a| !address_ok(a)) {
            return Err(format!("{bad:?} is not a plain email address"));
        }
        if !attendees.is_empty() {
            event.insert("attendees".into(), attendees.iter().map(|a| json!({"email": a})).collect());
        }
        let token = self.token(member, SCOPE_CALENDAR_EVENTS).await?;
        let v = self.send(self.http.post(url).bearer_auth(token.as_str()).json(&Value::Object(event))).await?;
        Ok(json!({"id": v.get("id"), "html_link": v.get("htmlLink")}))
    }
}

#[async_trait::async_trait]
impl Connector for GoogleWorkspace {
    fn handles(&self, tool: &str) -> bool {
        TOOLS.contains(&tool)
    }

    async fn run(&self, member_email: &str, tool: &str, args: &Value) -> Result<Value, String> {
        if !address_ok(member_email) {
            return Err("the requester has no usable email in the roster".to_owned());
        }
        match tool {
            "gmail_send" => self.gmail_send(member_email, args).await,
            "calendar_list_events" => self.list_events(member_email, args).await,
            "calendar_create_event" => self.create_event(member_email, args).await,
            other => Err(format!("Google Workspace does not run {other}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::connectors::testing::{jwt_claims, test_pem};

    async fn google() -> (MockServer, GoogleWorkspace) {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"access_token": "ya29.member"})))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/gmail/v1/users/me/messages/send"))
            .and(header("authorization", "Bearer ya29.member"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": "m1", "threadId": "t1"})))
            .mount(&server)
            .await;
        let key = json!({
            "client_email": "executor@acme-prod.iam.gserviceaccount.com",
            "private_key": test_pem(),
            "token_uri": "https://oauth2.googleapis.com/token",
        });
        let gw = GoogleWorkspace::from_service_account(
            &key.to_string(),
            &server.uri(),
            Some(&format!("{}/token", server.uri())),
        )
        .unwrap();
        (server, gw)
    }

    async fn requests_to(server: &MockServer, p: &str) -> Vec<wiremock::Request> {
        server.received_requests().await.unwrap().into_iter().filter(|r| r.url.path() == p).collect()
    }

    fn send_args() -> Value {
        json!({"to": ["ops@acme.com"], "cc": ["kim@acme.com"], "subject": "Deploy", "body": "Shipping now."})
    }

    #[tokio::test]
    async fn impersonates_only_the_requesting_member() {
        let (server, gw) = google().await;
        let out = gw.run("lee@acme.com", "gmail_send", &send_args()).await.unwrap();
        assert_eq!(out, json!({"id": "m1", "thread_id": "t1"}));
        let tokens = requests_to(&server, "/token").await;
        assert_eq!(tokens.len(), 1);
        let form: Vec<(String, String)> =
            reqwest::Url::parse(&format!("http://x/?{}", String::from_utf8_lossy(&tokens[0].body)))
                .unwrap()
                .query_pairs()
                .into_owned()
                .collect();
        assert!(form.contains(&("grant_type".into(), "urn:ietf:params:oauth:grant-type:jwt-bearer".into())));
        let assertion = &form.iter().find(|(k, _)| k == "assertion").unwrap().1;
        let claims = jwt_claims(assertion);
        assert_eq!(claims["sub"], "lee@acme.com");
        assert_eq!(claims["iss"], "executor@acme-prod.iam.gserviceaccount.com");
        assert_eq!(claims["aud"], format!("{}/token", server.uri()));
        assert_eq!(claims["scope"], SCOPE_SEND);

        // The arguments can't name someone else to act as: they are refused before any token is asked for.
        for extra in
            [json!({"from": "ceo@acme.com"}), json!({"sub": "ceo@acme.com"}), json!({"account": 1, "user": "x"})]
        {
            let mut args = send_args();
            args.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
            assert!(gw.run("lee@acme.com", "gmail_send", &args).await.is_err(), "{args}");
        }
        assert_eq!(requests_to(&server, "/token").await.len(), 1);
    }

    #[tokio::test]
    async fn gmail_send_builds_the_message_from_the_approved_args_only() {
        let (server, gw) = google().await;
        gw.run("lee@acme.com", "gmail_send", &send_args()).await.unwrap();
        let sent = &requests_to(&server, "/gmail/v1/users/me/messages/send").await[0];
        let raw = serde_json::from_slice::<Value>(&sent.body).unwrap()["raw"].as_str().unwrap().to_owned();
        let text = String::from_utf8(data_encoding::BASE64URL_NOPAD.decode(raw.as_bytes()).unwrap()).unwrap();
        let (head, body) = text.split_once("\r\n\r\n").unwrap();
        assert_eq!(
            head,
            "From: lee@acme.com\r\nTo: ops@acme.com\r\nCc: kim@acme.com\r\nSubject: Deploy\r\nMIME-Version: 1.0\r\n\
             Content-Type: text/plain; charset=UTF-8\r\nContent-Transfer-Encoding: base64"
        );
        let decoded = data_encoding::BASE64.decode(body.replace("\r\n", "").as_bytes()).unwrap();
        assert_eq!(decoded, b"Shipping now.");

        // Nothing outside to/cc/subject/body reaches the message: Bcc and header injection are refused.
        let bad = [
            json!({"to": ["a@acme.com"], "subject": "s", "body": "b", "bcc": ["x@evil.com"]}),
            json!({"to": ["a@acme.com\r\nBcc: x@evil.com"], "subject": "s", "body": "b"}),
            json!({"to": ["a@acme.com"], "subject": "s\r\nBcc: x@evil.com", "body": "b"}),
            json!({"to": ["Eve <x@evil.com>"], "subject": "s", "body": "b"}),
            json!({"to": [], "subject": "s", "body": "b"}),
        ];
        for args in bad {
            assert!(gw.run("lee@acme.com", "gmail_send", &args).await.is_err(), "{args}");
        }
        assert_eq!(requests_to(&server, "/gmail/v1/users/me/messages/send").await.len(), 1);
        assert_eq!(header_text("Grüße"), "=?UTF-8?B?R3LDvMOfZQ==?=");
    }

    #[tokio::test]
    async fn calendar_events_are_listed_and_created_as_the_member() {
        let (server, gw) = google().await;
        Mock::given(method("GET"))
            .and(path("/calendar/v3/calendars/primary/events"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"items": [
                {"id": "e1", "summary": "Standup", "start": {"dateTime": "2026-10-27T09:00:00Z"},
                 "end": {"dateTime": "2026-10-27T09:15:00Z"}, "attendees": [{"email": "x@acme.com"}]}
            ]})))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/calendar/v3/calendars/team@acme.com/events"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": "e2", "htmlLink": "l"})))
            .mount(&server)
            .await;
        let listed = gw
            .run("lee@acme.com", "calendar_list_events", &json!({"from": "2026-10-27", "to": "2026-10-28"}))
            .await
            .unwrap();
        assert_eq!(listed["events"][0]["title"], "Standup");
        let get = &requests_to(&server, "/calendar/v3/calendars/primary/events").await[0];
        let q: Vec<(String, String)> = get.url.query_pairs().into_owned().collect();
        assert!(q.contains(&("timeMin".into(), "2026-10-27T00:00:00Z".into())), "{q:?}");
        let created = gw
            .run(
                "lee@acme.com",
                "calendar_create_event",
                &json!({"title": "Review", "start": "2026-10-28T10:00:00Z", "calendar": "team@acme.com"}),
            )
            .await
            .unwrap();
        assert_eq!(created, json!({"id": "e2", "html_link": "l"}));
        let post = &requests_to(&server, "/calendar/v3/calendars/team@acme.com/events").await[0];
        assert_eq!(
            serde_json::from_slice::<Value>(&post.body).unwrap(),
            json!({"summary": "Review", "start": {"dateTime": "2026-10-28T10:00:00Z"},
                   "end": {"dateTime": "2026-10-28T11:00:00Z"}})
        );
        let scopes: Vec<String> = requests_to(&server, "/token")
            .await
            .iter()
            .map(|r| {
                let body = String::from_utf8_lossy(&r.body).into_owned();
                let a = body.split("assertion=").nth(1).unwrap().to_owned();
                jwt_claims(&a)["scope"].as_str().unwrap().to_owned()
            })
            .collect();
        assert_eq!(scopes, [SCOPE_CALENDAR_READ, SCOPE_CALENDAR_EVENTS]);
    }
}
