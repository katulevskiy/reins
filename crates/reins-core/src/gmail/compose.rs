//! RFC 2822 message builder for `messages.send`.
//!
//! Headers are assembled only from values that already passed
//! `OutgoingEmail::normalized` (bare ASCII addresses, no control characters) or
//! that are re-encoded here (subject → RFC 2047, body → base64), so no input can
//! start a new header line. There is no `From` header: Gmail fills in the
//! authenticated sender. A draft may carry one file: the message is then
//! `multipart/mixed`, with the file's name and type cleaned the same way.

use data_encoding::{BASE64, BASE64URL};
use reins_proto::gmail::OutgoingEmail;

use crate::CoreError;

const MAX_MESSAGE_ID_LEN: usize = 998;
/// Subjects longer than this are always encoded and folded.
const MAX_PLAIN_SUBJECT_BYTES: usize = 900;
/// Raw bytes per RFC 2047 encoded word (base64 of 45 bytes = 60 chars; word = 72 chars).
const ENCODED_WORD_CHUNK: usize = 45;

/// What Gmail needs to keep a reply in its conversation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReplyContext {
    pub thread_id: String,
    /// The original's `Message-ID` header value, `<id@host>`.
    pub message_id: String,
    /// The original's `References` header value, if any.
    pub references: Option<String>,
}

/// A syntactically valid `<id@host>` message id, or `None`.
pub fn valid_message_id(raw: &str) -> Option<&str> {
    let id = raw.trim();
    let inner = id.strip_prefix('<')?.strip_suffix('>')?;
    let ok = id.len() <= MAX_MESSAGE_ID_LEN
        && inner.matches('@').count() == 1
        && !inner.starts_with('@')
        && !inner.ends_with('@')
        && inner.bytes().all(|b| b.is_ascii_graphic() && !matches!(b, b'<' | b'>' | b'(' | b')' | b'\\' | b'"' | b','));
    ok.then_some(id)
}

/// All valid message ids in a `References` value, in order.
fn valid_references(raw: &str) -> Vec<&str> {
    raw.split_whitespace().filter_map(valid_message_id).collect()
}

/// The `Subject` header value: plain when safe, else RFC 2047 `B` encoded words.
pub fn encode_subject(subject: &str) -> String {
    let plain = subject.is_ascii()
        && !subject.contains("=?")
        && subject.len() <= MAX_PLAIN_SUBJECT_BYTES
        && subject.bytes().all(|b| b == b' ' || b.is_ascii_graphic());
    if plain {
        return subject.to_owned();
    }
    let mut words = Vec::new();
    let mut chunk = String::new();
    for c in subject.chars() {
        if chunk.len() + c.len_utf8() > ENCODED_WORD_CHUNK {
            words.push(std::mem::take(&mut chunk));
        }
        chunk.push(c);
    }
    if !chunk.is_empty() || words.is_empty() {
        words.push(chunk);
    }
    words.iter().map(|w| format!("=?UTF-8?B?{}?=", BASE64.encode(w.as_bytes()))).collect::<Vec<_>>().join("\r\n ")
}

fn crlf(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n").replace('\n', "\r\n")
}

fn wrapped_base64(text: &str) -> String {
    wrapped_bytes(text.as_bytes())
}

fn wrapped_bytes(data: &[u8]) -> String {
    let encoded = BASE64.encode(data);
    encoded.as_bytes().chunks(76).map(|c| String::from_utf8_lossy(c).into_owned()).collect::<Vec<_>>().join("\r\n")
}

/// A file attached to a draft.
pub struct Attachment {
    pub name: String,
    pub content_type: String,
    pub data: Vec<u8>,
}

/// Separates the parts. Base64 lines never start with `-`, so no part can contain it.
const BOUNDARY: &str = "=_reins_part";

/// A MIME type as given, when it is one (`application/pdf`), else the generic one.
fn clean_type(raw: &str) -> String {
    let t = raw.trim().to_ascii_lowercase();
    let token =
        |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'+' | b'-'));
    match t.split_once('/') {
        Some((a, b)) if token(a) && token(b) && t.len() <= 100 => t,
        _ => "application/octet-stream".to_owned(),
    }
}

/// The file name for `filename="..."`: plain when it is plain ASCII, else one RFC 2047 encoded word (what Gmail itself
/// writes). Control characters and path separators never reach the header.
fn filename_param(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .filter(|c| !c.is_control())
        .map(|c| {
            if matches!(c, '/' | '\\') {
                '_'
            } else {
                c
            }
        })
        .collect();
    let cleaned: String = cleaned.trim().chars().take(200).collect();
    if cleaned.is_empty() {
        return "file".to_owned();
    }
    if cleaned.bytes().all(|b| b == b' ' || (b.is_ascii_graphic() && b != b'"')) && !cleaned.contains("=?") {
        cleaned
    } else {
        format!("=?UTF-8?B?{}?=", BASE64.encode(cleaned.as_bytes()))
    }
}

/// The RFC 2822 text of `email`, ready to be base64url encoded.
pub fn rfc822(email: &OutgoingEmail, reply: Option<&ReplyContext>) -> Result<String, CoreError> {
    rfc822_with(email, reply, None)
}

/// The RFC 2822 text of `email` with an optional file attached.
pub fn rfc822_with(
    email: &OutgoingEmail,
    reply: Option<&ReplyContext>,
    attachment: Option<&Attachment>,
) -> Result<String, CoreError> {
    // Defense in depth: never build headers from an unvalidated email.
    let email = email.clone().normalized().map_err(|e| CoreError::invalid(e.to_string()))?;
    let mut headers = vec!["MIME-Version: 1.0".to_owned(), format!("To: {}", email.to.join(", "))];
    if !email.cc.is_empty() {
        headers.push(format!("Cc: {}", email.cc.join(", ")));
    }
    headers.push(format!("Subject: {}", encode_subject(&email.subject)));
    if let Some(reply) = reply
        && let Some(message_id) = valid_message_id(&reply.message_id)
    {
        headers.push(format!("In-Reply-To: {message_id}"));
        let mut refs: Vec<&str> = reply.references.as_deref().map(valid_references).unwrap_or_default();
        if refs.last() != Some(&message_id) {
            refs.push(message_id);
        }
        headers.push(format!("References: {}", refs.join(" ")));
    }
    let text = wrapped_base64(&crlf(&email.body));
    let Some(file) = attachment else {
        headers.push("Content-Type: text/plain; charset=UTF-8".to_owned());
        headers.push("Content-Transfer-Encoding: base64".to_owned());
        return Ok(format!("{}\r\n\r\n{text}\r\n", headers.join("\r\n")));
    };
    headers.push(format!("Content-Type: multipart/mixed; boundary=\"{BOUNDARY}\""));
    let content_type = clean_type(&file.content_type);
    let name = filename_param(&file.name);
    Ok(format!(
        "{}\r\n\r\n--{BOUNDARY}\r\nContent-Type: text/plain; charset=UTF-8\r\nContent-Transfer-Encoding: base64\r\n\r\n\
         {text}\r\n--{BOUNDARY}\r\nContent-Type: {content_type}; name=\"{name}\"\r\nContent-Disposition: attachment; filename=\"{name}\"\r\n\
         Content-Transfer-Encoding: base64\r\n\r\n{}\r\n--{BOUNDARY}--\r\n",
        headers.join("\r\n"),
        wrapped_bytes(&file.data)
    ))
}

/// The `raw` field of `messages.send`: the whole message, base64url (padded).
pub fn raw(email: &OutgoingEmail, reply: Option<&ReplyContext>) -> Result<String, CoreError> {
    raw_with(email, reply, None)
}

/// [`raw`] with an optional file attached.
pub fn raw_with(
    email: &OutgoingEmail,
    reply: Option<&ReplyContext>,
    attachment: Option<&Attachment>,
) -> Result<String, CoreError> {
    Ok(BASE64URL.encode(rfc822_with(email, reply, attachment)?.as_bytes()))
}

#[cfg(test)]
mod tests {
    use mail_parser::{MessageParser, MimeHeaders};

    use super::*;

    fn email(subject: &str, body: &str) -> OutgoingEmail {
        OutgoingEmail {
            to: vec!["alice@work.com".to_owned(), "bob@work.com".to_owned()],
            cc: vec!["carol@work.com".to_owned()],
            subject: subject.to_owned(),
            body: body.to_owned(),
            reply_to_message_id: None,
        }
    }

    fn reply() -> ReplyContext {
        ReplyContext {
            thread_id: "t1".to_owned(),
            message_id: "<orig@mail.example.com>".to_owned(),
            references: Some("<a@x.com> garbage <b@x.com>".to_owned()),
        }
    }

    #[test]
    fn message_round_trips_through_a_real_parser() {
        let text = rfc822(&email("Weekly report", "Hello\nSecond line with é and 日本語"), None).unwrap();
        assert!(text.contains("\r\nTo: alice@work.com, bob@work.com\r\n"));
        assert!(text.contains("\r\nCc: carol@work.com\r\n"));
        assert!(!text.to_lowercase().contains("\r\nfrom:"), "Gmail fills in the sender");
        assert!(!text.contains("In-Reply-To"));
        let parsed = MessageParser::default().parse(text.as_bytes()).unwrap();
        assert_eq!(parsed.subject(), Some("Weekly report"));
        assert_eq!(parsed.body_text(0).unwrap().trim_end(), "Hello\r\nSecond line with é and 日本語");
        assert_eq!(parsed.to().unwrap().iter().count(), 2);
    }

    #[test]
    fn non_ascii_and_suspicious_subjects_are_encoded() {
        assert_eq!(encode_subject("Plain subject"), "Plain subject");
        for subject in ["Café ☕", "=?UTF-8?B?QQ==?= trick", &"a".repeat(950)] {
            let encoded = encode_subject(subject);
            assert!(encoded.starts_with("=?UTF-8?B?"), "{subject:?} -> {encoded:?}");
            for line in encoded.split("\r\n ") {
                assert!(line.len() <= 75, "encoded word too long: {line}");
            }
            let text = rfc822(&email(subject, "b"), None).unwrap();
            let parsed = MessageParser::default().parse(text.as_bytes()).unwrap();
            assert_eq!(parsed.subject(), Some(subject), "round trip");
        }
    }

    #[test]
    fn nothing_can_start_a_new_header() {
        // Every hostile field is rejected before any header is written.
        let mut e = email("Hi", "b");
        e.to = vec!["a@b.com\r\nBcc: eve@evil.com".to_owned()];
        assert!(rfc822(&e, None).is_err());
        let mut e = email("Hi\r\nBcc: eve@evil.com", "b");
        e.cc = vec![];
        assert!(rfc822(&e, None).is_err());
        let mut e = email("Hi", "b");
        e.cc = vec!["Bob <bob@x.com>".to_owned()];
        assert!(rfc822(&e, None).is_err());
        // A body that looks like headers stays inside the base64 body.
        let text = rfc822(&email("Hi", "x\r\nBcc: eve@evil.com\r\n\r\ny"), None).unwrap();
        assert!(!text.contains("Bcc"));
        let parsed = MessageParser::default().parse(text.as_bytes()).unwrap();
        assert!(parsed.header("Bcc").is_none());
    }

    #[test]
    fn replies_carry_valid_threading_headers_only() {
        let text = rfc822(&email("Re: x", "b"), Some(&reply())).unwrap();
        assert!(text.contains("\r\nIn-Reply-To: <orig@mail.example.com>\r\n"));
        assert!(text.contains("\r\nReferences: <a@x.com> <b@x.com> <orig@mail.example.com>\r\n"), "{text}");
        let mut bad = reply();
        bad.message_id = "<x@y>\r\nBcc: eve@evil.com".to_owned();
        let text = rfc822(&email("Re: x", "b"), Some(&bad)).unwrap();
        assert!(!text.contains("In-Reply-To") && !text.contains("Bcc"));
        assert_eq!(valid_message_id("<a@b>"), Some("<a@b>"));
        for bad in ["a@b", "<ab>", "<a@@b>", "<@b>", "<a@>", "<a b@c>", "<a@b>>", "", "<a\"@b>"] {
            assert_eq!(valid_message_id(bad), None, "{bad}");
        }
    }

    #[test]
    fn raw_is_padded_base64url_of_the_message() {
        let raw = raw(&email("Hi", "b"), None).unwrap();
        assert!(raw.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'=')));
        let decoded = BASE64URL.decode(raw.as_bytes()).unwrap();
        assert!(String::from_utf8(decoded).unwrap().contains("Subject: Hi"));
    }

    #[test]
    fn a_file_rides_along_as_a_second_part() {
        let file = Attachment {
            name: "Rechnung März \"final\"/../x.pdf".to_owned(),
            content_type: "Application/PDF".to_owned(),
            data: b"%PDF-1.4 bytes".to_vec(),
        };
        let text = rfc822_with(&email("Invoice", "See attached"), Some(&reply()), Some(&file)).unwrap();
        assert!(text.contains("In-Reply-To: <orig@mail.example.com>"));
        let parsed = MessageParser::default().parse(text.as_bytes()).unwrap();
        assert_eq!(parsed.body_text(0).unwrap().trim_end(), "See attached");
        let attached = parsed.attachment(0).unwrap();
        assert_eq!(attached.contents(), b"%PDF-1.4 bytes");
        assert_eq!(attached.attachment_name(), Some("Rechnung März \"final\"_.._x.pdf"));
        assert!(text.contains("Content-Type: application/pdf;"));
        let odd = Attachment {
            name: "a\r\nBcc: eve@evil.com".to_owned(),
            content_type: "text/html\r\nBcc: x".to_owned(),
            data: Vec::new(),
        };
        let text = rfc822_with(&email("x", "y"), None, Some(&odd)).unwrap();
        assert!(!text.contains("\r\nBcc") && text.contains("application/octet-stream"), "{text}");
        assert!(MessageParser::default().parse(text.as_bytes()).unwrap().header("Bcc").is_none());
    }

    #[test]
    fn long_bodies_wrap_at_76_columns() {
        let text = rfc822(&email("Hi", &"word ".repeat(200)), None).unwrap();
        let body = text.split("\r\n\r\n").nth(1).unwrap();
        assert!(body.lines().all(|l| l.len() <= 76));
        let parsed = MessageParser::default().parse(text.as_bytes()).unwrap();
        assert_eq!(parsed.body_text(0).unwrap().trim_end(), "word ".repeat(200).trim_end());
    }
}
