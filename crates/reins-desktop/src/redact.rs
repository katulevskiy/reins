//! `reins run`'s output filter: every released value in the command's stdout and stderr, as it is and in its base64
//! (standard and URL-safe, with or without padding) and percent-encoded forms, becomes `[vault:project/env/NAME]`.
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

/// Every byte but the unreserved ones (RFC 3986) as `%XX`.
fn percent(value: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(value.len() * 3);
    for b in value {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(*b);
        } else {
            out.extend_from_slice(format!("%{b:02X}").as_bytes());
        }
    }
    out
}

impl Redactor {
    /// `secrets`: `(label, value)`, the label without brackets (`vault:acme/dev/KEY`).
    #[must_use]
    pub fn new(secrets: &[(String, &str)]) -> Self {
        let mut patterns: Vec<Zeroizing<Vec<u8>>> = Vec::new();
        let mut labels = Vec::new();
        for (label, value) in secrets {
            let raw = value.as_bytes();
            if raw.len() < MIN_LEN {
                continue;
            }
            let forms = [
                raw.to_vec(),
                BASE64.encode(raw).into_bytes(),
                BASE64_NOPAD.encode(raw).into_bytes(),
                BASE64URL.encode(raw).into_bytes(),
                BASE64URL_NOPAD.encode(raw).into_bytes(),
                percent(raw),
            ];
            for form in forms {
                let form = Zeroizing::new(form);
                if form.len() >= MIN_LEN && !patterns.iter().any(|p| **p == *form) {
                    patterns.push(form);
                    labels.push(format!("[{label}]"));
                }
            }
        }
        let finder = (!patterns.is_empty())
            .then(|| {
                AhoCorasick::builder()
                    .match_kind(MatchKind::LeftmostLongest)
                    .build(patterns.iter().map(|p| p.as_slice()))
                    .ok()
            })
            .flatten();
        Self {
            finder,
            patterns,
            labels,
        }
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
        assert!(Redactor::new(&[]).is_empty());
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
        let r = Redactor::new(&[("vault:a/b/A".to_owned(), "secret"), ("vault:a/b/B".to_owned(), "secret-longer")]);
        assert_eq!(all_at_once(&r, "secret-longer secret"), "[vault:a/b/B] [vault:a/b/A]");
        let mut s = r.stream();
        let mut out = s.push(b"x secret-lo");
        out.extend(s.push(b"nger y"));
        out.extend(s.finish());
        assert_eq!(String::from_utf8(out).unwrap(), "x [vault:a/b/B] y");
    }
}
