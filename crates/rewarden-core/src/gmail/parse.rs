//! Gmail message JSON → the views the policy engine and the AI see.
//!
//! Every value here comes from a stranger's email: addresses are parsed one
//! header at a time with CR/LF removed, `From` must be exactly one mailbox,
//! bodies never come from attachments or nested messages, and all display
//! strings are stripped of bidi controls (see `text`).

use data_encoding::BASE64URL_NOPAD;
use mail_parser::MessageParser;
use rewarden_policy::MessageFacts;
use rewarden_proto::gmail::{MessageFull, MessageSummary};
use rewarden_proto::normalize_address;

use super::model::{GmailMessage, Header, Part};
use crate::text;

/// Bodies longer than this are cut (bytes).
pub const MAX_BODY_BYTES: usize = 100_000;
const MAX_PART_DEPTH: usize = 20;
const MAX_SUBJECT_CHARS: usize = 300;
const MAX_SNIPPET_CHARS: usize = 300;
const MAX_NAME_CHARS: usize = 100;

/// A bare address and the (untrusted) display name that came with it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mailbox {
    pub address: String,
    pub name: Option<String>,
}

/// Parses one header value into mailboxes. Anything that is not a valid bare
/// address is dropped, so hostile input can only ever shrink the result.
pub fn parse_mailboxes(header: &str, value: &str) -> Vec<Mailbox> {
    let flat: String = value
        .chars()
        .map(|c| {
            if c.is_control() {
                ' '
            } else {
                c
            }
        })
        .collect();
    let raw = format!("{header}: {flat}\r\n\r\n");
    let Some(message) = MessageParser::default().parse(raw.as_bytes()) else {
        return Vec::new();
    };
    let list = match header {
        "From" => message.from(),
        "To" => message.to(),
        "Cc" => message.cc(),
        _ => None,
    };
    let Some(list) = list else {
        return Vec::new();
    };
    // RFC 5322 group syntax (`name: a@x, b@y;`) is how injected text such as
    // "a@bank.com\r\nFrom: ceo@bank.com" turns into a "second" mailbox: a sender is never a group.
    if header == "From" && !matches!(list, mail_parser::Address::List(_)) {
        return Vec::new();
    }
    list.iter()
        .filter_map(|a| {
            let address = normalize_address(a.address()?).ok()?;
            let name = a
                .name()
                .map(text::one_line)
                .map(|n| text::truncate_chars(&n, MAX_NAME_CHARS))
                .filter(|n| !n.is_empty());
            Some(Mailbox {
                address,
                name,
            })
        })
        .collect()
}

fn header_values<'a>(headers: &'a [Header], name: &'a str) -> impl Iterator<Item = &'a str> {
    headers.iter().filter(move |h| h.name.eq_ignore_ascii_case(name)).map(|h| h.value.as_str())
}

/// The sender: exactly one `From` header holding exactly one mailbox, else `None`.
pub fn sender(headers: &[Header]) -> Option<Mailbox> {
    let mut values = header_values(headers, "From");
    let only = values.next()?;
    if values.next().is_some() {
        return None;
    }
    let mut boxes = parse_mailboxes("From", only);
    if boxes.len() == 1 {
        boxes.pop()
    } else {
        None
    }
}

fn recipients(headers: &[Header], name: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for value in header_values(headers, name) {
        for mailbox in parse_mailboxes(name, value) {
            if !out.contains(&mailbox.address) {
                out.push(mailbox.address);
            }
        }
    }
    out
}

fn decode_data(data: &str) -> Option<Vec<u8>> {
    BASE64URL_NOPAD.decode(data.trim_end_matches('=').as_bytes()).ok()
}

fn charset_of(part: &Part) -> Option<String> {
    let content_type = header_values(&part.headers, "Content-Type").next()?;
    content_type
        .split(';')
        .skip(1)
        .filter_map(|p| p.trim().split_once('='))
        .find(|(k, _)| k.trim().eq_ignore_ascii_case("charset"))
        .map(|(_, v)| v.trim().trim_matches('"').to_owned())
}

/// UTF-8 when valid, else the declared charset, else lossy UTF-8.
fn decode_text(bytes: &[u8], charset: Option<&str>) -> String {
    if let Ok(s) = std::str::from_utf8(bytes) {
        return s.to_owned();
    }
    if let Some(encoding) = charset.and_then(|c| encoding_rs::Encoding::for_label(c.as_bytes())) {
        return encoding.decode(bytes).0.into_owned();
    }
    String::from_utf8_lossy(bytes).into_owned()
}

struct Found {
    plain: Option<String>,
    html: Option<String>,
}

fn walk(part: &Part, depth: usize, found: &mut Found) {
    if depth > MAX_PART_DEPTH || (found.plain.is_some() && found.html.is_some()) {
        return;
    }
    let mime = part.mime_type.to_ascii_lowercase();
    // Never descend into attached/forwarded messages and never use attachments.
    if mime.starts_with("message/") || !part.filename.is_empty() {
        return;
    }
    let body = part.body.as_ref();
    if body.and_then(|b| b.attachment_id.as_ref()).is_some() {
        return;
    }
    if mime == "text/plain" || mime == "text/html" {
        let text = body
            .and_then(|b| b.data.as_deref())
            .and_then(decode_data)
            .map(|bytes| decode_text(&bytes, charset_of(part).as_deref()))
            .unwrap_or_default();
        let slot = if mime == "text/plain" {
            &mut found.plain
        } else {
            &mut found.html
        };
        if slot.is_none() {
            *slot = Some(text);
        }
        return;
    }
    for child in &part.parts {
        walk(child, depth + 1, found);
    }
}

/// The message text: the first non-attachment `text/plain`, else the first
/// non-attachment `text/html` reduced to text. Capped at [`MAX_BODY_BYTES`].
pub fn body_text(payload: &Part) -> String {
    let mut found = Found {
        plain: None,
        html: None,
    };
    walk(payload, 0, &mut found);
    let raw = match (found.plain, found.html) {
        (Some(plain), _) if !plain.trim().is_empty() => plain,
        (_, Some(html)) => text::html_to_text(&html),
        (Some(plain), None) => plain,
        (None, None) => String::new(),
    };
    text::truncate_bytes(&text::neutralize(&raw), MAX_BODY_BYTES).to_owned()
}

/// A parsed message: what the AI may be shown, and what the policy evaluates.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParsedMessage {
    pub summary: MessageSummary,
    pub facts: MessageFacts,
    /// Only when the message was fetched with `format=full`.
    pub body: Option<String>,
}

impl ParsedMessage {
    /// The AI-facing form of a fully fetched message.
    pub fn into_full(self) -> MessageFull {
        MessageFull {
            body_text: self.body.unwrap_or_default(),
            summary: self.summary,
        }
    }
}

fn internal_seconds(m: &GmailMessage) -> i64 {
    m.internal_date.as_deref().and_then(|d| d.trim().parse::<i64>().ok()).map_or(0, |ms| ms / 1000)
}

pub fn parse_message(m: &GmailMessage, with_body: bool) -> ParsedMessage {
    let headers: &[Header] = m.payload.as_ref().map_or(&[], |p| p.headers.as_slice());
    let from = sender(headers);
    let to = recipients(headers, "To");
    let cc = recipients(headers, "Cc");
    let subject_raw = header_values(headers, "Subject").next().unwrap_or("");
    let subject = text::truncate_chars(&text::one_line(subject_raw), MAX_SUBJECT_CHARS);
    let date = internal_seconds(m);
    let body = if with_body {
        m.payload.as_ref().map(body_text)
    } else {
        None
    };
    let mut all_recipients = to.clone();
    all_recipients.extend(cc.iter().filter(|a| !to.contains(a)).cloned());
    let from_address = from.as_ref().map(|f| f.address.clone()).unwrap_or_default();
    ParsedMessage {
        summary: MessageSummary {
            id: m.id.clone(),
            thread_id: m.thread_id.clone(),
            from: from_address.clone(),
            from_name: from.and_then(|f| f.name),
            to,
            cc,
            subject: subject.clone(),
            date,
            snippet: text::truncate_chars(&text::one_line(&m.snippet), MAX_SNIPPET_CHARS),
        },
        facts: MessageFacts {
            id: m.id.clone(),
            from: from_address,
            to: all_recipients,
            subject,
            body: body.clone(),
            labels: m.label_ids.clone(),
            date,
        },
        body,
    }
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::*;

    fn h(name: &str, value: &str) -> Header {
        Header {
            name: name.to_owned(),
            value: value.to_owned(),
        }
    }

    fn b64(s: &str) -> String {
        BASE64URL_NOPAD.encode(s.as_bytes())
    }

    #[expect(clippy::needless_pass_by_value, reason = "test helper; keeps call sites terse")]
    fn message(headers: Value, payload_extra: Value) -> GmailMessage {
        let mut payload = json!({"mimeType": "multipart/alternative", "headers": headers});
        if let (Some(p), Some(extra)) = (payload.as_object_mut(), payload_extra.as_object()) {
            p.extend(extra.clone());
        }
        serde_json::from_value(json!({
            "id": "m1", "threadId": "t1", "labelIds": ["INBOX", "Label_7"], "snippet": "Hello \u{202E}there",
            "internalDate": "1700000000123", "payload": payload
        }))
        .unwrap()
    }

    fn from_only(value: &str) -> String {
        sender(&[h("From", value)]).map(|m| m.address).unwrap_or_default()
    }

    #[test]
    fn plain_and_named_mailboxes() {
        assert_eq!(from_only("alerts@bank.com"), "alerts@bank.com");
        assert_eq!(from_only("Bank Alerts <Alerts@Bank.COM>"), "alerts@bank.com");
        assert_eq!(from_only("\"Bank, Inc.\" <a@bank.com>"), "a@bank.com");
        assert_eq!(sender(&[h("From", "Bank Alerts <a@bank.com>")]).unwrap().name.as_deref(), Some("Bank Alerts"));
        assert_eq!(from_only("=?UTF-8?B?QmFuayDDqQ==?= <a@bank.com>"), "a@bank.com");
        assert_eq!(
            sender(&[h("From", "=?UTF-8?B?QmFuayDDqQ==?= <a@bank.com>")]).unwrap().name.as_deref(),
            Some("Bank é")
        );
        assert_eq!(from_only("a@bank.com (Bank)"), "a@bank.com");
    }

    #[test]
    fn a_forged_display_name_never_becomes_the_sender() {
        // The classic: the name looks like a trusted address, the mailbox is the attacker's.
        assert_eq!(from_only("\"alice@bank.com\" <eve@evil.com>"), "eve@evil.com");
        // Unquoted, the "name" parses as a second mailbox: ambiguous, so no sender. Either way never alice.
        for forged in ["alice@bank.com <eve@evil.com>", "<eve@evil.com> alice@bank.com"] {
            let got = from_only(forged);
            assert!(got.is_empty() || got == "eve@evil.com", "{forged:?} gave {got:?}");
        }
    }

    #[test]
    fn ambiguous_senders_are_empty() {
        for hostile in [
            "a@bank.com, eve@evil.com",
            "Group: a@bank.com, b@bank.com;",
            "undisclosed-recipients:;",
            "not an address",
            "",
            "a@bank.com\r\nFrom: ceo@bank.com",
            "x\r\nFrom: ceo@bank.com",
            "<a@bank.com><eve@evil.com>",
        ] {
            let got = from_only(hostile);
            assert!(got.is_empty() || got == "a@bank.com", "{hostile:?} gave {got:?}");
            assert_ne!(got, "ceo@bank.com", "{hostile:?} injected a header");
            if hostile.contains("From:") {
                assert!(got.is_empty(), "{hostile:?} must not produce a sender, got {got:?}");
            }
        }
        // Two From headers: no sender at all.
        assert!(sender(&[h("From", "a@bank.com"), h("From", "eve@evil.com")]).is_none());
        assert!(sender(&[h("from", "a@bank.com"), h("FROM", "a@bank.com")]).is_none(), "case-insensitive");
        assert!(sender(&[]).is_none());
    }

    #[test]
    fn bidi_controls_cannot_reorder_an_address() {
        let got = from_only("\u{202E}moc.knab@a\u{202C} <a@bank.com>");
        assert!(got.is_empty() || got == "a@bank.com", "{got:?}");
        assert!(got.is_ascii());
        let m =
            parse_message(&message(json!([{"name": "Subject", "value": "Pay \u{202E}gnp.exe now"}]), json!({})), false);
        assert_eq!(m.summary.subject, "Pay gnp.exe now");
        assert_eq!(m.summary.snippet, "Hello there");
    }

    #[test]
    fn recipients_are_bare_deduplicated_and_split_by_header() {
        let headers = json!([
            {"name": "From", "value": "Me <me@example.com>"},
            {"name": "To", "value": "Bob <bob@x.com>, alice@x.com, Bob <bob@x.com>"},
            {"name": "Cc", "value": "Carol <carol@x.com>, bob@x.com"},
            {"name": "Subject", "value": "  Quarterly\n report  "}
        ]);
        let p = parse_message(&message(headers, json!({})), false);
        assert_eq!(p.summary.to, vec!["bob@x.com", "alice@x.com"]);
        assert_eq!(p.summary.cc, vec!["carol@x.com", "bob@x.com"]);
        assert_eq!(
            p.facts.to,
            vec!["bob@x.com", "alice@x.com", "carol@x.com"],
            "facts.to is To + Cc without duplicates"
        );
        assert_eq!(p.summary.subject, "Quarterly report");
        assert_eq!((p.summary.date, p.facts.date), (1_700_000_000, 1_700_000_000));
        assert_eq!(p.facts.labels, vec!["INBOX", "Label_7"]);
        assert!(p.body.is_none() && p.facts.body.is_none());
    }

    #[test]
    fn invalid_recipient_entries_are_dropped_not_fatal() {
        let headers = json!([{"name": "To", "value": "good@x.com, \"weird\u{202E}\" <bad@@x.com>, jörg@x.com"}]);
        let p = parse_message(&message(headers, json!({})), false);
        assert_eq!(p.summary.to, vec!["good@x.com"]);
    }

    fn part(mime: &str, text: &str) -> Value {
        json!({"mimeType": mime, "body": {"size": text.len(), "data": b64(text)}})
    }

    #[test]
    fn body_prefers_first_plain_text() {
        let payload = json!({"parts": [part("text/plain", "Plain body"), part("text/html", "<p>HTML body</p>")]});
        let p = parse_message(&message(json!([]), payload), true);
        assert_eq!(p.body.as_deref(), Some("Plain body"));
        assert_eq!(p.facts.body.as_deref(), Some("Plain body"));
        assert_eq!(p.into_full().body_text, "Plain body");
        let html_only = json!({"parts": [part("text/html", "<p>Only <b>HTML</b></p>")]});
        assert_eq!(parse_message(&message(json!([]), html_only), true).body.as_deref(), Some("Only HTML"));
        let single = json!({"mimeType": "text/plain", "body": {"data": b64("single part")}});
        assert_eq!(parse_message(&message(json!([]), single), true).body.as_deref(), Some("single part"));
    }

    #[test]
    fn attachments_and_nested_messages_are_never_the_body() {
        let attachment =
            json!({"mimeType": "text/plain", "filename": "notes.txt", "body": {"data": b64("ATTACHED SECRET")}});
        let by_id = json!({"mimeType": "text/plain", "body": {"attachmentId": "ANGjdJ"}});
        let forwarded = json!({"mimeType": "message/rfc822", "parts": [part("text/plain", "FORWARDED SECRET")]});
        let payload = json!({"parts": [attachment, by_id, forwarded]});
        let p = parse_message(&message(json!([]), payload), true);
        assert_eq!(p.body.as_deref(), Some(""));
        let payload = json!({"parts": [
            {"mimeType": "text/plain", "filename": "x.txt", "body": {"data": b64("ATTACHED")}},
            part("text/plain", "Real body")]});
        assert_eq!(parse_message(&message(json!([]), payload), true).body.as_deref(), Some("Real body"));
    }

    #[test]
    fn nesting_is_bounded_and_bodies_are_capped() {
        let mut deep = part("text/plain", "too deep");
        for _ in 0..(MAX_PART_DEPTH + 5) {
            deep = json!({"mimeType": "multipart/mixed", "parts": [deep]});
        }
        assert_eq!(parse_message(&message(json!([]), json!({"parts": [deep]})), true).body.as_deref(), Some(""));
        let big = "é".repeat(MAX_BODY_BYTES);
        let capped =
            parse_message(&message(json!([]), json!({"parts": [part("text/plain", &big)]})), true).body.unwrap();
        assert!(capped.len() <= MAX_BODY_BYTES && capped.chars().all(|c| c == 'é'));
    }

    #[test]
    fn charsets_and_bidi_in_bodies() {
        let latin1 = BASE64URL_NOPAD.encode(&[b'c', b'a', b'f', 0xE9]);
        let payload = json!({"mimeType": "text/plain",
            "headers": [{"name": "Content-Type", "value": "text/plain; charset=\"ISO-8859-1\""}],
            "body": {"data": latin1}});
        assert_eq!(parse_message(&message(json!([]), payload), true).body.as_deref(), Some("café"));
        let bidi = json!({"mimeType": "text/plain", "body": {"data": b64("pay\u{202E}gnp.exe")}});
        assert_eq!(parse_message(&message(json!([]), bidi), true).body.as_deref(), Some("paygnp.exe"));
        let garbage = json!({"mimeType": "text/plain", "body": {"data": "!!!not base64!!!"}});
        assert_eq!(parse_message(&message(json!([]), garbage), true).body.as_deref(), Some(""));
    }

    #[test]
    fn missing_pieces_do_not_panic() {
        let bare: GmailMessage = serde_json::from_value(json!({"id": "m9"})).unwrap();
        let p = parse_message(&bare, true);
        assert_eq!((p.summary.from.as_str(), p.summary.date, p.summary.subject.as_str()), ("", 0, ""));
        assert_eq!(p.body, None);
        let odd: GmailMessage = serde_json::from_value(json!({"id": "m8", "internalDate": "not a number"})).unwrap();
        assert_eq!(parse_message(&odd, false).summary.date, 0);
    }
}
