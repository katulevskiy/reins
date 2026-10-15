//! `reins run`'s output filter: every released value in the command's stdout and stderr becomes
//! `[vault:project/env/NAME]`, as it is and in the forms a program commonly prints it in:
//!
//! - base64, standard and URL-safe, with or without padding;
//! - percent-encoded, in upper or lower case hex, with a space as `%20` or `+`, leaving raw only the unreserved
//!   characters, those and `/`, those of `encodeURIComponent` (`!'()*` too), or those of `encodeURI`;
//! - JSON-escaped (`\n`, `\"`, `\\`, controls as `\u00XX`), with or without `\/`, and with every non-ASCII
//!   character as `\uXXXX` (as `ensure_ascii` writes it), which is how a multi-line or PEM value shows in JSON.
//!
//! Building the filter fails rather than letting anything through unfiltered ([`Redactor::new`]): a run that asked
//! for redaction does not start without it.
//!
//! The output is streamed: nothing is held back except the end of a chunk that could still become a value with the next
//! chunk (the longest suffix that is the start of one), so prompts and progress lines still show. Values shorter than
//! [`MIN_LEN`] bytes are not looked for (`1`, `true` would wreck any output); a value split by something else (a value
//! inside a longer base64 text) is not recognised.

use aho_corasick::{AhoCorasick, MatchKind};
use data_encoding::{BASE64, BASE64_NOPAD, BASE64URL, BASE64URL_NOPAD};
use zeroize::Zeroizing;

/// The shortest value looked for.
pub const MIN_LEN: usize = 4;

/// What to look for and what to write instead.
pub struct Redactor {
    finder: Option<AhoCorasick>,
    patterns: Vec<Zeroizing<Vec<u8>>>,
    labels: Vec<String>,
}

/// The characters percent-encoding leaves raw, besides letters and digits, in the sets programs use: RFC 3986's
/// unreserved ones; those and `/` (paths); `encodeURIComponent`'s; `encodeURI`'s.
const RAW_SETS: [&[u8]; 4] = [b"-._~", b"-._~/", b"-._~!'()*", b"-._~!'()*;,/?:@&=+$#"];

/// `value` percent-encoded: `raw` (besides letters and digits) left as it is, the rest as `%XX` (`lower`: lower-case
/// hex), a space as `+` when `plus`.
fn percent(value: &[u8], raw: &[u8], lower: bool, plus: bool) -> Vec<u8> {
    let mut out = Vec::with_capacity(value.len() * 3);
    for b in value {
        if b.is_ascii_alphanumeric() || raw.contains(b) {
            out.push(*b);
        } else if *b == b' ' && plus {
            out.push(b'+');
        } else if lower {
            out.extend_from_slice(format!("%{b:02x}").as_bytes());
        } else {
            out.extend_from_slice(format!("%{b:02X}").as_bytes());
        }
    }
    out
}

/// `value` as inside a JSON string: `"`, `\\` and controls escaped (`\n`, `\t`, … or `\u00XX`); `slash`: `/` as `\/`;
/// `ascii`: every non-ASCII character as `\uXXXX` (a surrogate pair above U+FFFF).
fn json_escaped(value: &str, slash: bool, ascii: bool, upper: bool) -> Vec<u8> {
    let hex = |u: u32| {
        if upper {
            format!("\\u{u:04X}")
        } else {
            format!("\\u{u:04x}")
        }
    };
    let mut out = String::with_capacity(value.len() + 8);
    for c in value.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '/' if slash => out.push_str("\\/"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => out.push_str(&hex(c as u32)),
            c if ascii && !c.is_ascii() => {
                let mut units = [0u16; 2];
                for u in c.encode_utf16(&mut units) {
                    out.push_str(&hex(u32::from(*u)));
                }
            }
            c => out.push(c),
        }
    }
    out.into_bytes()
}

/// Every form of `value` looked for.
fn forms(value: &str) -> Vec<Vec<u8>> {
    let raw = value.as_bytes();
    let mut all = vec![
        raw.to_vec(),
        BASE64.encode(raw).into_bytes(),
        BASE64_NOPAD.encode(raw).into_bytes(),
        BASE64URL.encode(raw).into_bytes(),
        BASE64URL_NOPAD.encode(raw).into_bytes(),
    ];
    for set in RAW_SETS {
        for lower in [false, true] {
            for plus in [false, true] {
                all.push(percent(raw, set, lower, plus));
            }
        }
    }
    for slash in [false, true] {
        for ascii in [false, true] {
            for upper in [false, true] {
                all.push(json_escaped(value, slash, ascii, upper));
            }
        }
    }
    all
}

/// The filter could not be built: the run does not start.
#[derive(Debug, thiserror::Error)]
#[error("the output filter could not be built ({0}); nothing was run")]
pub struct BuildError(String);

impl Redactor {
    /// `secrets`: `(label, value)`, the label without brackets (`vault:acme/dev/KEY`). An error when the filter
    /// cannot be built: the caller then runs nothing (it never runs the command unfiltered instead).
    pub fn new(secrets: &[(String, &str)]) -> Result<Self, BuildError> {
        let mut patterns: Vec<Zeroizing<Vec<u8>>> = Vec::new();
        let mut labels = Vec::new();
        for (label, value) in secrets {
            if value.len() < MIN_LEN {
                continue;
            }
            for form in forms(value) {
                let form = Zeroizing::new(form);
                if form.len() >= MIN_LEN && !patterns.iter().any(|p| **p == *form) {
                    patterns.push(form);
                    labels.push(format!("[{label}]"));
                }
            }
        }
        let finder = if patterns.is_empty() {
            None
        } else {
            Some(
                AhoCorasick::builder()
                    .match_kind(MatchKind::LeftmostLongest)
                    .build(patterns.iter().map(|p| p.as_slice()))
                    .map_err(|e| BuildError(e.to_string()))?,
            )
        };
        Ok(Self {
            finder,
            patterns,
            labels,
        })
    }

    /// Whether there is anything to look for.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.finder.is_none()
    }

    /// A filter for one output stream.
    #[must_use]
    pub fn stream(&self) -> Stream<'_> {
        Stream {
            redactor: self,
            carry: Zeroizing::new(Vec::new()),
        }
    }

    /// Where the end of `buf` may still become a value: the first index whose suffix is the start of a longer pattern.
    fn hold_point(&self, buf: &[u8]) -> usize {
        let longest = self.patterns.iter().map(|p| p.len()).max().unwrap_or(0);
        let from = buf.len().saturating_sub(longest.saturating_sub(1));
        (from..buf.len())
            .find(|&i| {
                let tail = &buf[i..];
                self.patterns.iter().any(|p| p.len() > tail.len() && p.starts_with(tail))
            })
            .unwrap_or(buf.len())
    }

    /// Replaces what is certain in `buf`; returns the output and how much of `buf` it used.
    fn replace(&self, buf: &[u8], last: bool) -> (Vec<u8>, usize) {
        let Some(finder) = &self.finder else {
            return (buf.to_vec(), buf.len());
        };
        let hold = if last {
            buf.len()
        } else {
            self.hold_point(buf)
        };
        let mut out = Vec::with_capacity(buf.len());
        let mut at = 0;
        for m in finder.find_iter(buf) {
            if m.start() >= hold {
                break;
            }
            out.extend_from_slice(&buf[at..m.start()]);
            out.extend_from_slice(self.labels[m.pattern().as_usize()].as_bytes());
            at = m.end();
        }
        let end = hold.max(at);
        if at < end {
            out.extend_from_slice(&buf[at..end]);
        }
        (out, end)
    }
}

/// One output stream being filtered.
pub struct Stream<'a> {
    redactor: &'a Redactor,
    /// The end of the last chunk that could still become a value.
    carry: Zeroizing<Vec<u8>>,
}

impl Stream<'_> {
    /// The output for `chunk`: everything certain so far.
    pub fn push(&mut self, chunk: &[u8]) -> Vec<u8> {
        let mut buf = Zeroizing::new(std::mem::take(&mut *self.carry));
        buf.extend_from_slice(chunk);
        let (out, used) = self.redactor.replace(&buf, false);
        self.carry = Zeroizing::new(buf[used..].to_vec());
        out
    }

    /// The rest, at the end of the stream.
    pub fn finish(&mut self) -> Vec<u8> {
        let buf = Zeroizing::new(std::mem::take(&mut *self.carry));
        self.redactor.replace(&buf, true).0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn redactor() -> Redactor {
        Redactor::new(&[
            ("vault:acme/dev/KEY".to_owned(), "sk-live-12345"),
            ("vault:acme/dev/PASS".to_owned(), "p@ss word/+"),
            ("vault:acme/dev/SHORT".to_owned(), "abc"),
        ])
        .unwrap()
    }

    fn all_at_once(r: &Redactor, text: &str) -> String {
        let mut s = r.stream();
        let mut out = s.push(text.as_bytes());
        out.extend(s.finish());
        String::from_utf8(out).unwrap()
    }

    #[test]
    fn values_and_their_encodings_are_replaced() {
        let r = redactor();
        assert_eq!(all_at_once(&r, "key=sk-live-12345!"), "key=[vault:acme/dev/KEY]!");
        let b64 = BASE64.encode(b"sk-live-12345");
        assert_eq!(all_at_once(&r, &format!("auth {b64}")), "auth [vault:acme/dev/KEY]");
        let url = BASE64URL_NOPAD.encode(b"p@ss word/+");
        assert_eq!(all_at_once(&r, &format!("t={url}")), "t=[vault:acme/dev/PASS]");
        assert_eq!(all_at_once(&r, "pw=p%40ss%20word%2F%2B"), "pw=[vault:acme/dev/PASS]");
        assert_eq!(all_at_once(&r, "abc is too short to look for"), "abc is too short to look for");
        assert!(Redactor::new(&[]).unwrap().is_empty());
    }

    #[test]
    fn the_usual_encodings_of_urls_and_json_are_replaced_too() {
        let r = redactor();
        for shown in [
            "p%40ss+word%2F%2B",   // a form: + for a space
            "p%40ss%20word/%2B",   // a path: / left raw
            "p%40ss%20word%2f%2b", // lower-case hex
            "p%40ss+word%2f%2b",   // both
            "p%40ss%20word%2F%2B", // encodeURIComponent leaves nothing of these raw
            "p@ss%20word/+",       // encodeURI
        ] {
            assert_eq!(all_at_once(&r, &format!("u={shown}")), "u=[vault:acme/dev/PASS]", "{shown}");
        }
        let pem = "-----BEGIN KEY-----\nMIIBVQ/x+y\n-----END KEY-----\n";
        let quoted = "\"we said \"hi\" é 🙂\"";
        let r = Redactor::new(&[("vault:a/b/PEM".to_owned(), pem), ("vault:a/b/Q".to_owned(), quoted)]).unwrap();
        let json = serde_json::to_string(pem).unwrap();
        assert_eq!(all_at_once(&r, &format!("{{\"key\":{json}}}")), "{\"key\":\"[vault:a/b/PEM]\"}");
        let slashed = json.replace('/', "\\/");
        assert_eq!(all_at_once(&r, &slashed), "\"[vault:a/b/PEM]\"");
        let ascii = r#""\"we said \"hi\" \u00e9 \ud83d\ude42\"""#;
        assert_eq!(all_at_once(&r, ascii), "\"[vault:a/b/Q]\"");
        assert_eq!(
            all_at_once(&r, &ascii.replace("u00e9", "u00E9").replace("ud83d\\ude42", "uD83D\\uDE42")),
            "\"[vault:a/b/Q]\""
        );
    }

    #[test]
    fn a_value_split_across_chunks_is_still_caught_and_nothing_else_waits() {
        let r = redactor();
        for split in 1.."token sk-live-12345 end".len() {
            let text = "token sk-live-12345 end";
            let mut s = r.stream();
            let mut out = s.push(&text.as_bytes()[..split]);
            out.extend(s.push(&text.as_bytes()[split..]));
            out.extend(s.finish());
            assert_eq!(String::from_utf8(out).unwrap(), "token [vault:acme/dev/KEY] end", "split at {split}");
        }
        // A prompt that cannot become a value shows at once.
        let mut s = r.stream();
        assert_eq!(s.push(b"Password: "), b"Password: ");
        // The start of a value waits for the next chunk; what it turns out to be decides.
        assert_eq!(s.push(b"sk-li"), b"");
        assert_eq!(s.push(b"fe\n"), b"sk-life\n");
    }

    #[test]
    fn the_longest_value_wins() {
        let r = Redactor::new(&[("vault:a/b/A".to_owned(), "secret"), ("vault:a/b/B".to_owned(), "secret-longer")])
            .unwrap();
        assert_eq!(all_at_once(&r, "secret-longer secret"), "[vault:a/b/B] [vault:a/b/A]");
        let mut s = r.stream();
        let mut out = s.push(b"x secret-lo");
        out.extend(s.push(b"nger y"));
        out.extend(s.finish());
        assert_eq!(String::from_utf8(out).unwrap(), "x [vault:a/b/B] y");
    }
}
