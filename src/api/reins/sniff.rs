//! What a held file is, worked out from its bytes while they stream in (never from what the uploader claims), and the
//! preview the user sees before deciding. Pure: no IO.

use data_encoding::BASE64;
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use reins_proto::blob::{BlobPreview, MAX_PREVIEW_IMAGE, MAX_PREVIEW_TEXT};

/// The start of the file kept for the preview: a whole small image, or enough bytes for the text head.
const HEAD_LIMIT: usize = MAX_PREVIEW_IMAGE + 1;
/// Longest file name put in a `Content-Disposition` header.
const MAX_HEADER_NAME: usize = 100;

pub const TEXT_TYPE: &str = "text/plain; charset=utf-8";
pub const BINARY_TYPE: &str = "application/octet-stream";

/// A type recognised by its magic bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Kind {
    pub mime: &'static str,
    pub description: &'static str,
    /// Shown as a picture on the phone.
    pub image: bool,
}

const fn kind(mime: &'static str, description: &'static str, image: bool) -> Kind {
    Kind {
        mime,
        description,
        image,
    }
}

/// The type of a file from its first bytes, if it is one we recognise.
pub fn magic(head: &[u8]) -> Option<Kind> {
    let starts = |prefix: &[u8]| head.starts_with(prefix);
    if starts(b"\x89PNG\r\n\x1a\n") {
        Some(kind("image/png", "PNG image", true))
    } else if starts(b"\xff\xd8\xff") {
        Some(kind("image/jpeg", "JPEG image", true))
    } else if starts(b"GIF87a") || starts(b"GIF89a") {
        Some(kind("image/gif", "GIF image", true))
    } else if starts(b"RIFF") && head.get(8..12) == Some(b"WEBP") {
        Some(kind("image/webp", "WebP image", true))
    } else if starts(b"%PDF-") {
        Some(kind("application/pdf", "PDF document", false))
    } else if starts(b"PK\x03\x04") || starts(b"PK\x05\x06") || starts(b"PK\x07\x08") {
        Some(kind("application/zip", "ZIP archive", false))
    } else if starts(b"\x1f\x8b") {
        Some(kind("application/gzip", "gzip archive", false))
    } else if starts(b"\x7fELF") {
        Some(kind("application/x-executable", "ELF executable", false))
    } else if [
        &b"\xfe\xed\xfa\xce"[..],
        b"\xfe\xed\xfa\xcf",
        b"\xce\xfa\xed\xfe",
        b"\xcf\xfa\xed\xfe",
        b"\xca\xfe\xba\xbe",
    ]
    .iter()
    .any(|m| starts(m))
    {
        Some(kind("application/x-mach-binary", "Mach-O executable", false))
    } else if head.get(4..8) == Some(b"ftyp") {
        Some(kind("video/mp4", "MP4 video", false))
    } else {
        None
    }
}

/// Incremental UTF-8 check for a stream cut at arbitrary byte boundaries. NUL bytes count as binary.
#[derive(Debug, Default)]
struct Utf8Check {
    /// The start of a character split by a chunk boundary (at most 3 bytes).
    pending: Vec<u8>,
    invalid: bool,
}

impl Utf8Check {
    fn feed(&mut self, chunk: &[u8]) {
        if self.invalid {
            return;
        }
        if chunk.contains(&0) {
            self.invalid = true;
            return;
        }
        let joined;
        let bytes = if self.pending.is_empty() {
            chunk
        } else {
            joined = [self.pending.as_slice(), chunk].concat();
            &joined
        };
        match std::str::from_utf8(bytes) {
            Ok(_) => self.pending.clear(),
            Err(e) if e.error_len().is_none() => self.pending = bytes[e.valid_up_to()..].to_vec(),
            Err(_) => self.invalid = true,
        }
    }

    fn is_text(&self) -> bool {
        !self.invalid && self.pending.is_empty()
    }
}

/// Watches a file stream go by and describes it at the end.
#[derive(Debug, Default)]
pub struct Sniffer {
    head: Vec<u8>,
    utf8: Utf8Check,
    size: u64,
}

/// What [`Sniffer::finish`] found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sniffed {
    pub content_type: String,
    pub preview: BlobPreview,
}

impl Sniffer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn feed(&mut self, chunk: &[u8]) {
        let room = HEAD_LIMIT.saturating_sub(self.head.len());
        self.head.extend_from_slice(&chunk[..room.min(chunk.len())]);
        self.utf8.feed(chunk);
        self.size = self.size.saturating_add(chunk.len() as u64);
    }

    pub fn finish(self) -> Sniffed {
        if let Some(kind) = magic(&self.head) {
            let preview = if kind.image {
                let whole = usize::try_from(self.size).is_ok_and(|s| s <= MAX_PREVIEW_IMAGE);
                BlobPreview::Image {
                    mime: kind.mime.to_owned(),
                    data_base64: whole.then(|| BASE64.encode(&self.head)),
                }
            } else {
                BlobPreview::Binary {
                    description: kind.description.to_owned(),
                }
            };
            return Sniffed {
                content_type: kind.mime.to_owned(),
                preview,
            };
        }
        if self.utf8.is_text() {
            let valid = match std::str::from_utf8(&self.head) {
                Ok(s) => s,
                // The kept head may end inside a character.
                Err(e) => std::str::from_utf8(&self.head[..e.valid_up_to()]).unwrap_or_default(),
            };
            let head: String = valid.chars().take(MAX_PREVIEW_TEXT).collect();
            let truncated = (head.len() as u64) < self.size;
            return Sniffed {
                content_type: TEXT_TYPE.to_owned(),
                preview: BlobPreview::Text {
                    head,
                    truncated,
                },
            };
        }
        Sniffed {
            content_type: BINARY_TYPE.to_owned(),
            preview: BlobPreview::Binary {
                description: "data".to_owned(),
            },
        }
    }
}

/// Sniffs bytes held in memory.
pub fn sniff(bytes: &[u8]) -> Sniffed {
    let mut sniffer = Sniffer::new();
    sniffer.feed(bytes);
    sniffer.finish()
}

/// A file name as the AI gave it: shown and used for downloads, never as a path.
pub fn valid_file_name(name: &str) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty()
        || name.chars().count() > 200
        || name.contains(['/', '\\'])
        || name.chars().any(char::is_control)
        || name == "."
        || name == ".."
    {
        return Err("`name` must be a file name of 1..=200 characters, without a path".to_owned());
    }
    Ok(name.to_owned())
}

/// The ASCII form of a file name for the `filename=` parameter: letters, digits, `.`, `-`, `_`; no leading dot.
pub fn ascii_file_name(name: &str) -> String {
    let mapped: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') {
                c
            } else {
                '_'
            }
        })
        .collect();
    let mut clean = mapped.trim_start_matches('.').to_owned();
    clean.truncate(MAX_HEADER_NAME);
    if clean.trim_matches('_').is_empty() {
        "download".to_owned()
    } else {
        clean
    }
}

/// Characters kept as they are in an RFC 8187 `filename*` value.
const ATTR_CHAR: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'!')
    .remove(b'#')
    .remove(b'$')
    .remove(b'&')
    .remove(b'+')
    .remove(b'-')
    .remove(b'.')
    .remove(b'^')
    .remove(b'_')
    .remove(b'`')
    .remove(b'|')
    .remove(b'~');

/// `Content-Disposition` for a download: always an attachment, so a browser never renders it.
pub fn content_disposition(name: &str) -> String {
    let unicode: String = name.chars().filter(|c| !c.is_control() && !matches!(c, '/' | '\\')).take(200).collect();
    format!(
        "attachment; filename=\"{}\"; filename*=UTF-8''{}",
        ascii_file_name(name),
        utf8_percent_encode(&unicode, ATTR_CHAR)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kind_of(bytes: &[u8]) -> Option<&'static str> {
        magic(bytes).map(|k| k.mime)
    }

    #[test]
    fn magic_bytes_name_the_common_types() {
        assert_eq!(kind_of(b"\x89PNG\r\n\x1a\n\0\0"), Some("image/png"));
        assert_eq!(kind_of(b"\xff\xd8\xff\xe0"), Some("image/jpeg"));
        assert_eq!(kind_of(b"GIF89a.."), Some("image/gif"));
        assert_eq!(kind_of(b"RIFF\x10\0\0\0WEBPVP8 "), Some("image/webp"));
        assert_eq!(kind_of(b"RIFF\x10\0\0\0WAVEfmt "), None);
        assert_eq!(kind_of(b"%PDF-1.7\n"), Some("application/pdf"));
        assert_eq!(kind_of(b"PK\x03\x04rest"), Some("application/zip"));
        assert_eq!(kind_of(b"\x1f\x8b\x08\0"), Some("application/gzip"));
        assert_eq!(kind_of(b"\x7fELF\x02\x01"), Some("application/x-executable"));
        assert_eq!(kind_of(b"\xcf\xfa\xed\xfe\x07"), Some("application/x-mach-binary"));
        assert_eq!(kind_of(b"\0\0\0\x18ftypmp42"), Some("video/mp4"));
        assert_eq!(kind_of(b"hello"), None);
        assert_eq!(kind_of(b""), None);
    }

    #[test]
    fn text_is_previewed_by_its_head() {
        let s = sniff(b"line one\nline two\n");
        assert_eq!(s.content_type, TEXT_TYPE);
        assert_eq!(
            s.preview,
            BlobPreview::Text {
                head: "line one\nline two\n".into(),
                truncated: false
            }
        );
        let long = "\u{e9}".repeat(MAX_PREVIEW_TEXT + 10);
        let BlobPreview::Text {
            head,
            truncated,
        } = sniff(long.as_bytes()).preview
        else {
            panic!("text expected")
        };
        assert_eq!((head.chars().count(), truncated), (MAX_PREVIEW_TEXT, true));
    }

    #[test]
    fn utf8_split_across_chunks_is_still_text_but_broken_utf8_is_not() {
        let text = "caf\u{e9} \u{1f600} ok".as_bytes();
        for cut in 0..text.len() {
            let mut s = Sniffer::new();
            s.feed(&text[..cut]);
            s.feed(&text[cut..]);
            assert_eq!(s.finish().content_type, TEXT_TYPE, "cut at {cut}");
        }
        let mut cut_short = Sniffer::new();
        cut_short.feed(&"\u{1f600}".as_bytes()[..2]);
        assert_eq!(cut_short.finish().content_type, BINARY_TYPE, "a file may not end inside a character");
        assert_eq!(sniff(b"\xff\xfe garbage").content_type, BINARY_TYPE);
        assert_eq!(sniff(b"text with a \0 NUL").content_type, BINARY_TYPE);
        assert_eq!(
            sniff(b"\0\x01\x02").preview,
            BlobPreview::Binary {
                description: "data".into()
            }
        );
    }

    #[test]
    fn small_images_are_shown_whole_and_large_ones_only_named() {
        let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
        png.extend_from_slice(&[7u8; 100]);
        let s = sniff(&png);
        assert_eq!(
            s.preview,
            BlobPreview::Image {
                mime: "image/png".into(),
                data_base64: Some(BASE64.encode(&png))
            }
        );
        png.resize(MAX_PREVIEW_IMAGE + 1, 7);
        let mut streamed = Sniffer::new();
        for chunk in png.chunks(10_000) {
            streamed.feed(chunk);
        }
        assert_eq!(
            streamed.finish().preview,
            BlobPreview::Image {
                mime: "image/png".into(),
                data_base64: None
            }
        );
        assert_eq!(
            sniff(b"%PDF-1.4").preview,
            BlobPreview::Binary {
                description: "PDF document".into()
            }
        );
    }

    #[test]
    fn file_names_are_checked_and_made_header_safe() {
        assert_eq!(valid_file_name(" report.pdf ").unwrap(), "report.pdf");
        for bad in ["", "  ", "../etc/passwd", "a\\b", "a\nb", "..", &"x".repeat(201)] {
            assert!(valid_file_name(bad).is_err(), "{bad:?}");
        }
        assert_eq!(ascii_file_name("r\u{e9}sum\u{e9} 2026.pdf"), "r_sum__2026.pdf");
        assert_eq!(ascii_file_name("..hidden"), "hidden");
        assert_eq!(ascii_file_name("\u{1f600}"), "download");
        assert_eq!(ascii_file_name(&"a".repeat(300)).len(), MAX_HEADER_NAME);
        let header = content_disposition("a \"b\"\r\n.txt");
        assert!(header.starts_with("attachment; filename=\"a__b___.txt\"; filename*=UTF-8''"), "{header}");
        assert!(!header.contains('\r') && !header.contains('\n'), "{header}");
        assert!(content_disposition("r\u{e9}.txt").ends_with("filename*=UTF-8''r%C3%A9.txt"));
    }
}
