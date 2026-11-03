//! Signing git commits and tags with an SSH key (`gpg.format = ssh`), as `ssh-keygen -Y sign` does it: the
//! [SSHSIG](https://github.com/openssh/openssh-portable/blob/master/PROTOCOL.sshsig) format, and the git objects a
//! signature may be made for.
//!
//! The phone gets the whole object git asks to sign (not only its hash), so that it can show what it signs; it hashes
//! the object itself and signs [`signed_data`]. The desktop app wraps the signature in [`armor`], which is what git
//! stores in the commit. Both sides build the same bytes from here.

use data_encoding::BASE64;
use sha2::{Digest as _, Sha256, Sha512};

/// The only namespace signed: git's.
pub const GIT_NAMESPACE: &str = "git";
/// The largest object the phone takes (its base64 is at most 65,536 characters).
pub const MAX_OBJECT: usize = 49_152;
const MAGIC: &[u8] = b"SSHSIG";
const VERSION: u32 = 1;
const BEGIN: &str = "-----BEGIN SSH SIGNATURE-----";
const END: &str = "-----END SSH SIGNATURE-----";
/// ssh-keygen's line length in the armor.
const LINE: usize = 70;

/// The message hash of a signature.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hash {
    Sha256,
    Sha512,
}

impl Hash {
    #[must_use]
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "sha256" => Some(Self::Sha256),
            "sha512" => Some(Self::Sha512),
            _ => None,
        }
    }

    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Sha256 => "sha256",
            Self::Sha512 => "sha512",
        }
    }

    #[must_use]
    pub fn digest(self, message: &[u8]) -> Vec<u8> {
        match self {
            Self::Sha256 => Sha256::digest(message).to_vec(),
            Self::Sha512 => Sha512::digest(message).to_vec(),
        }
    }
}

fn string(out: &mut Vec<u8>, s: &[u8]) {
    out.extend_from_slice(&u32::try_from(s.len()).unwrap_or(u32::MAX).to_be_bytes());
    out.extend_from_slice(s);
}

/// What the key signs for `message` (PROTOCOL.sshsig §"Signed Data"): the magic, the namespace, the hash's name and
/// the message's hash.
#[must_use]
pub fn signed_data(namespace: &str, hash: Hash, message: &[u8]) -> Vec<u8> {
    let mut out = MAGIC.to_vec();
    string(&mut out, namespace.as_bytes());
    string(&mut out, b"");
    string(&mut out, hash.name().as_bytes());
    string(&mut out, &hash.digest(message));
    out
}

/// The armored signature (`-----BEGIN SSH SIGNATURE-----`…) for the public key blob `key` and the SSH signature blob
/// `signature` (`string algorithm, string signature`), as `ssh-keygen -Y sign` writes it.
#[must_use]
pub fn armor(key: &[u8], namespace: &str, hash: Hash, signature: &[u8]) -> String {
    let mut blob = MAGIC.to_vec();
    blob.extend_from_slice(&VERSION.to_be_bytes());
    string(&mut blob, key);
    string(&mut blob, namespace.as_bytes());
    string(&mut blob, b"");
    string(&mut blob, hash.name().as_bytes());
    string(&mut blob, signature);
    let b64 = BASE64.encode(&blob);
    let mut out = String::with_capacity(b64.len() + b64.len() / LINE + BEGIN.len() + END.len() + 4);
    out.push_str(BEGIN);
    out.push('\n');
    for chunk in b64.as_bytes().chunks(LINE) {
        out.push_str(&String::from_utf8_lossy(chunk));
        out.push('\n');
    }
    out.push_str(END);
    out.push('\n');
    out
}

/// What kind of git object is signed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ObjectKind {
    Commit,
    Tag,
}

/// A git commit or annotated tag as git hands it to the signing program (without a signature).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GitObject {
    pub kind: ObjectKind,
    /// A commit's tree; a tag's object.
    pub target: String,
    /// A tag's object type (`commit`, …).
    pub target_type: Option<String>,
    /// A tag's name.
    pub tag: Option<String>,
    pub parents: usize,
    /// "Name <email>" of the author (commit) or tagger (tag).
    pub author: String,
    /// Unix seconds (0 when it does not parse).
    pub date: i64,
    /// The committer, when it is not the author.
    pub committer: Option<String>,
    /// The message's first line.
    pub subject: String,
    /// The rest of the message (after the blank line that ends the subject), as it is.
    pub body: String,
    /// Something in it changes how the text reads without being seen (bidi controls, zero-width characters).
    pub hidden_chars: bool,
}

fn hex_id(s: &str) -> bool {
    matches!(s.len(), 40 | 64) && s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// `Name <email> 1700000000 +0100` → ("Name <email>", 1700000000).
fn ident(value: &str) -> Option<(String, i64)> {
    let close = value.rfind('>')?;
    let (who, when) = value.split_at(close + 1);
    if !who.contains('<') {
        return None;
    }
    let date = when.split_whitespace().next().and_then(|t| t.parse().ok()).unwrap_or(0);
    Some((who.trim().to_owned(), date))
}

/// A character that changes how text reads without being seen: bidi controls, zero-width and other invisible format
/// characters (soft hyphen, joiners, invisible operators, fillers, tag characters, the BOM), line and paragraph
/// separators. The same list as the core's `text::is_invisible`, plus U+2028/U+2029.
#[must_use]
pub fn hidden_char(c: char) -> bool {
    matches!(
        c,
        '\u{061C}'
            | '\u{200E}'
            | '\u{200F}'
            | '\u{202A}'..='\u{202E}'
            | '\u{2066}'..='\u{2069}'
            | '\u{00AD}'
            | '\u{034F}'
            | '\u{115F}'
            | '\u{1160}'
            | '\u{17B4}'
            | '\u{17B5}'
            | '\u{180B}'..='\u{180F}'
            | '\u{200B}'..='\u{200D}'
            | '\u{2028}'
            | '\u{2029}'
            | '\u{2060}'..='\u{2065}'
            | '\u{206A}'..='\u{206F}'
            | '\u{3164}'
            | '\u{FEFF}'
            | '\u{FFA0}'
            | '\u{E0000}'..='\u{E007F}'
    )
}

/// git's subject and body of a message: leading blank lines skipped, the first paragraph joined into one line (as
/// `git log --oneline` shows it), the rest after the blank line that ends it.
fn subject_and_body(message: &str) -> (String, String) {
    let lines: Vec<&str> = message.split('\n').map(|l| l.trim_end_matches('\r')).collect();
    let start = lines.iter().position(|l| !l.trim().is_empty()).unwrap_or(lines.len());
    let end = lines[start..].iter().position(|l| l.trim().is_empty()).map_or(lines.len(), |n| start + n);
    let subject = lines[start..end].iter().map(|l| l.trim()).collect::<Vec<_>>().join(" ");
    let rest = lines.get(end..).unwrap_or_default();
    let body_start = rest.iter().position(|l| !l.trim().is_empty()).unwrap_or(rest.len());
    (subject, rest[body_start..].join("\n"))
}

/// Reads `bytes` as a git commit or annotated tag; `None` for anything else (or an object that is signed already).
/// Headers git writes and this does not name (`encoding`, `mergetag`, `extra`, …) are allowed after the known ones;
/// continuation lines (` …`) belong to the header before them.
#[must_use]
pub fn parse_object(bytes: &[u8]) -> Option<GitObject> {
    if bytes.len() > MAX_OBJECT || bytes.contains(&0) {
        return None;
    }
    let text = std::str::from_utf8(bytes).ok()?;
    let (head, message) = text.split_once("\n\n").unwrap_or((text.strip_suffix('\n').unwrap_or(text), ""));
    let mut headers: Vec<(&str, &str)> = Vec::new();
    for line in head.split('\n') {
        if line.starts_with(' ') {
            // A continuation (a `mergetag`'s lines): part of the header before it.
            headers.last()?;
            continue;
        }
        let (name, value) = line.split_once(' ')?;
        if name.is_empty() {
            return None;
        }
        headers.push((name, value));
    }
    if headers.iter().any(|(n, _)| matches!(*n, "gpgsig" | "gpgsig-sha256")) {
        return None;
    }
    // Another encoding would make git and the hosts show other text than the phone does.
    if headers.iter().any(|(n, v)| {
        *n == "encoding" && !v.trim().eq_ignore_ascii_case("utf-8") && !v.trim().eq_ignore_ascii_case("utf8")
    }) {
        return None;
    }
    let mut it = headers.iter().copied().peekable();
    let (first, target) = it.next()?;
    let kind = match first {
        "tree" => ObjectKind::Commit,
        "object" => ObjectKind::Tag,
        _ => return None,
    };
    if !hex_id(target) {
        return None;
    }
    let mut out = GitObject {
        kind,
        target: target.to_owned(),
        target_type: None,
        tag: None,
        parents: 0,
        author: String::new(),
        date: 0,
        committer: None,
        subject: String::new(),
        body: String::new(),
        hidden_chars: false,
    };
    match kind {
        ObjectKind::Commit => {
            while let Some((_, p)) = it.next_if(|(n, _)| *n == "parent") {
                if !hex_id(p) {
                    return None;
                }
                out.parents += 1;
            }
            let (author, date) = it.next().filter(|(n, _)| *n == "author").and_then(|(_, v)| ident(v))?;
            let (committer, _) = it.next().filter(|(n, _)| *n == "committer").and_then(|(_, v)| ident(v))?;
            out.committer = (committer != author).then_some(committer);
            out.author = author;
            out.date = date;
        }
        ObjectKind::Tag => {
            let (_, t) = it.next().filter(|(n, _)| *n == "type")?;
            let (_, name) = it.next().filter(|(n, _)| *n == "tag")?;
            if t.is_empty() || name.is_empty() {
                return None;
            }
            out.target_type = Some(t.to_owned());
            out.tag = Some(name.to_owned());
            // A tag made without a tagger (old git) has none.
            if let Some((_, v)) = it.next_if(|(n, _)| *n == "tagger") {
                let (who, date) = ident(v)?;
                out.author = who;
                out.date = date;
            }
        }
    }
    // What is left are headers this does not name; the known ones must not repeat.
    if it.any(|(n, _)| matches!(n, "tree" | "parent" | "author" | "committer" | "object" | "type" | "tag" | "tagger")) {
        return None;
    }
    (out.subject, out.body) = subject_and_body(message);
    out.hidden_chars = text.chars().any(hidden_char);
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const TREE: &str = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";
    const PARENT: &str = "1b36b5df0123456789abcdef0123456789abcdef";

    fn commit(extra: &str, message: &str) -> String {
        format!(
            "tree {TREE}\nparent {PARENT}\nauthor Ada <ada@example.com> 1700000000 +0100\ncommitter Ada \
             <ada@example.com> 1700000000 +0100\n{extra}\n{message}"
        )
    }

    #[test]
    fn a_commit_is_read() {
        let c = parse_object(commit("", "fix: the thing\n\nLonger text.\n").as_bytes()).unwrap();
        assert_eq!(c.kind, ObjectKind::Commit);
        assert_eq!((c.target.as_str(), c.parents), (TREE, 1));
        assert_eq!((c.author.as_str(), c.date, c.committer), ("Ada <ada@example.com>", 1_700_000_000, None));
        assert_eq!((c.subject.as_str(), c.body.as_str()), ("fix: the thing", "Longer text.\n"));
        assert!(!c.hidden_chars);
        let mergetag = commit("mergetag object x\n type commit\n tag v1\nencoding UTF-8\n", "merge\n");
        assert_eq!(parse_object(mergetag.as_bytes()).unwrap().subject, "merge");
        // git's subject: the first paragraph, after any blank lines.
        let c = parse_object(commit("", "\n\nBump deps\nand disable auth\n\nWhy.\n").as_bytes()).unwrap();
        assert_eq!((c.subject.as_str(), c.body.as_str()), ("Bump deps and disable auth", "Why.\n"));
        assert!(parse_object(commit("encoding ISO-8859-1\n", "x\n").as_bytes()).is_none());
    }

    #[test]
    fn a_tag_is_read() {
        let t = format!("object {PARENT}\ntype commit\ntag v1.2.3\ntagger Bo <bo@x.org> 1700000001 +0000\n\nv1.2.3\n");
        let t = parse_object(t.as_bytes()).unwrap();
        assert_eq!(t.kind, ObjectKind::Tag);
        assert_eq!((t.tag.as_deref(), t.target_type.as_deref()), (Some("v1.2.3"), Some("commit")));
        assert_eq!((t.author.as_str(), t.subject.as_str()), ("Bo <bo@x.org>", "v1.2.3"));
    }

    #[test]
    fn anything_else_is_not_a_git_object() {
        for bad in [
            String::new(),
            "release v1.2.3\n".to_owned(),
            "SSHSIG\0\0\0\x01".to_owned(),
            commit("gpgsig -----BEGIN SSH SIGNATURE-----\n x\n", "signed already\n"),
            commit("author Eve <e@x> 1 +0000\n", "two authors\n"),
            commit("", "ok\n").replace(TREE, "nothex"),
            format!("tree {TREE}\nauthor Ada <a@x> 1 +0000\n\nno committer\n"),
            format!("blob {TREE}\n\nx\n"),
        ] {
            assert!(parse_object(bad.as_bytes()).is_none(), "{bad:?}");
        }
        assert!(parse_object(&vec![b'a'; MAX_OBJECT + 1]).is_none());
    }

    #[test]
    fn hidden_characters_are_noticed() {
        let c = parse_object(commit("", "fix\u{202e}txt.exe\n").as_bytes()).unwrap();
        assert!(c.hidden_chars);
        for hidden in ['\u{00ad}', '\u{2028}', '\u{e0041}', '\u{3164}'] {
            assert!(hidden_char(hidden), "{hidden:?}");
        }
        assert!(!hidden_char('a') && !hidden_char('\u{fe0f}'));
    }

    #[test]
    fn the_signed_data_and_armor_follow_protocol_sshsig() {
        let data = signed_data("git", Hash::Sha512, b"msg");
        assert!(data.starts_with(b"SSHSIG\0\0\0\x03git\0\0\0\0\0\0\0\x06sha512\0\0\0\x40"));
        assert_eq!(data.len(), 6 + 7 + 4 + 10 + 4 + 64);
        let a = armor(b"KEY", "git", Hash::Sha512, b"SIG");
        assert!(a.starts_with("-----BEGIN SSH SIGNATURE-----\n") && a.ends_with("\n-----END SSH SIGNATURE-----\n"));
        let body: String = a.lines().filter(|l| !l.starts_with("-----")).collect();
        assert!(a.lines().all(|l| l.len() <= 70));
        let blob = BASE64.decode(body.as_bytes()).unwrap();
        assert!(blob.starts_with(b"SSHSIG\0\0\0\x01\0\0\0\x03KEY\0\0\0\x03git"));
        assert!(blob.ends_with(b"\0\0\0\x06sha512\0\0\0\x03SIG"));
    }
}
