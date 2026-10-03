//! Git objects: ids, hashing, and the parts of commits, trees and tags a push description needs.

use std::fmt;

use sha1::{Digest, Sha1};

/// A SHA-1 object id.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Oid(pub [u8; 20]);

impl Oid {
    pub const ZERO: Self = Self([0; 20]);

    /// 40 hex digits (either case).
    #[must_use]
    pub fn from_hex(s: &str) -> Option<Self> {
        let s = s.as_bytes();
        if s.len() != 40 {
            return None;
        }
        let mut out = [0u8; 20];
        for (i, [hi, lo]) in s.as_chunks::<2>().0.iter().enumerate() {
            out[i] = (hex_digit(*hi)? << 4) | hex_digit(*lo)?;
        }
        Some(Self(out))
    }

    #[must_use]
    pub fn from_bytes(b: &[u8]) -> Option<Self> {
        b.try_into().ok().map(Self)
    }

    /// Lowercase hex.
    #[must_use]
    pub fn to_hex(&self) -> String {
        const DIGITS: &[u8; 16] = b"0123456789abcdef";
        let mut s = String::with_capacity(40);
        for b in self.0 {
            s.push(char::from(DIGITS[usize::from(b >> 4)]));
            s.push(char::from(DIGITS[usize::from(b & 0xf)]));
        }
        s
    }

    #[must_use]
    pub fn is_zero(&self) -> bool {
        *self == Self::ZERO
    }
}

fn hex_digit(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

impl fmt::Display for Oid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_hex())
    }
}

impl fmt::Debug for Oid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Oid({})", self.to_hex())
    }
}

/// The kind of a git object.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ObjectKind {
    Commit,
    Tree,
    Blob,
    Tag,
}

impl ObjectKind {
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Commit => "commit",
            Self::Tree => "tree",
            Self::Blob => "blob",
            Self::Tag => "tag",
        }
    }

    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "commit" => Some(Self::Commit),
            "tree" => Some(Self::Tree),
            "blob" => Some(Self::Blob),
            "tag" => Some(Self::Tag),
            _ => None,
        }
    }
}

/// Incremental object hashing, for objects streamed out of a pack.
pub struct Hasher(Sha1);

impl Hasher {
    #[must_use]
    pub fn new(kind: ObjectKind, len: u64) -> Self {
        let mut h = Sha1::new();
        h.update(format!("{} {len}\0", kind.name()).as_bytes());
        Self(h)
    }

    pub fn update(&mut self, data: &[u8]) {
        self.0.update(data);
    }

    #[must_use]
    pub fn finish(self) -> Oid {
        Oid(self.0.finalize().into())
    }
}

/// The id git gives an object: SHA-1 of `"<kind> <len>\0"` and the content.
#[must_use]
pub fn hash_object(kind: ObjectKind, data: &[u8]) -> Oid {
    let mut h = Hasher::new(kind, data.len() as u64);
    h.update(data);
    h.finish()
}

/// What a commit says about itself that a push description shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Commit {
    pub tree: Oid,
    pub parents: Vec<Oid>,
    /// "Name <email>" as written (not cleaned).
    pub author: String,
    /// Author time, unix seconds.
    pub author_time: i64,
    /// Committer time, unix seconds: the order `git log` lists commits in.
    pub commit_time: i64,
    /// First line of the message.
    pub subject: String,
}

/// Parses a raw commit. Only `tree` is required; a missing or odd author or time gives empty or 0.
pub fn parse_commit(data: &[u8]) -> Result<Commit, String> {
    let (headers, message) = split_message(data);
    let mut tree = None;
    let mut parents = Vec::new();
    let mut author = (String::new(), 0);
    let mut commit_time = 0;
    for line in headers.split(|&b| b == b'\n') {
        // Continuation lines (gpgsig, mergetag) start with a space and are not headers.
        let Some((key, value)) = split_header(line) else {
            continue;
        };
        match key {
            b"tree" if tree.is_none() => tree = Some(parse_oid(value).ok_or("a commit has a malformed tree id")?),
            b"parent" => parents.push(parse_oid(value).ok_or("a commit has a malformed parent id")?),
            b"author" => author = parse_signature(value),
            b"committer" => commit_time = parse_signature(value).1,
            _ => {}
        }
    }
    let tree = tree.ok_or("a commit has no tree")?;
    let subject = message.split(|&b| b == b'\n').find(|l| !l.iter().all(u8::is_ascii_whitespace)).unwrap_or_default();
    Ok(Commit {
        tree,
        parents,
        author: author.0,
        author_time: author.1,
        commit_time,
        subject: String::from_utf8_lossy(subject).trim().to_owned(),
    })
}

/// An annotated tag: what it points at.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tag {
    pub object: Oid,
    pub kind: ObjectKind,
    pub name: String,
}

pub fn parse_tag(data: &[u8]) -> Result<Tag, String> {
    let (headers, _) = split_message(data);
    let (mut object, mut kind, mut name) = (None, None, String::new());
    for line in headers.split(|&b| b == b'\n') {
        let Some((key, value)) = split_header(line) else {
            continue;
        };
        match key {
            b"object" => object = parse_oid(value),
            b"type" => kind = std::str::from_utf8(value).ok().and_then(ObjectKind::from_name),
            b"tag" => name = String::from_utf8_lossy(value).into_owned(),
            _ => {}
        }
    }
    Ok(Tag {
        object: object.ok_or("a tag has no valid object")?,
        kind: kind.ok_or("a tag has no valid type")?,
        name,
    })
}

fn split_message(data: &[u8]) -> (&[u8], &[u8]) {
    match data.windows(2).position(|w| w == b"\n\n") {
        Some(i) => (&data[..i], &data[i + 2..]),
        None => (data, &[]),
    }
}

fn split_header(line: &[u8]) -> Option<(&[u8], &[u8])> {
    if line.first() == Some(&b' ') {
        return None;
    }
    let i = line.iter().position(|&b| b == b' ')?;
    Some((&line[..i], &line[i + 1..]))
}

fn parse_oid(value: &[u8]) -> Option<Oid> {
    Oid::from_hex(std::str::from_utf8(value).ok()?)
}

/// `Name <email> 1700000000 +0100` → ("Name <email>", 1700000000).
fn parse_signature(value: &[u8]) -> (String, i64) {
    let Some(end) = value.iter().rposition(|&b| b == b'>') else {
        return (String::from_utf8_lossy(value).trim().to_owned(), 0);
    };
    let who = String::from_utf8_lossy(&value[..=end]).trim().to_owned();
    let time = std::str::from_utf8(&value[end + 1..])
        .ok()
        .and_then(|rest| rest.split_whitespace().next())
        .and_then(|t| t.parse().ok())
        .unwrap_or(0);
    (who, time)
}

/// Mode bits git uses in trees.
pub mod mode {
    pub const TREE: u32 = 0o040_000;
    pub const BLOB: u32 = 0o100_644;
    pub const EXEC: u32 = 0o100_755;
    pub const LINK: u32 = 0o120_000;
    pub const GITLINK: u32 = 0o160_000;
}

/// What a tree entry is, from its mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntryKind {
    Tree,
    File,
    Link,
    Submodule,
}

impl EntryKind {
    #[must_use]
    pub fn of(mode: u32) -> Self {
        match mode & 0o170_000 {
            0o040_000 => Self::Tree,
            0o120_000 => Self::Link,
            0o160_000 => Self::Submodule,
            _ => Self::File,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TreeEntry {
    pub mode: u32,
    pub name: Vec<u8>,
    pub oid: Oid,
}

impl TreeEntry {
    #[must_use]
    pub fn kind(&self) -> EntryKind {
        EntryKind::of(self.mode)
    }
}

/// Parses a raw tree: `<octal mode> <name>\0<20-byte id>` repeated.
pub fn parse_tree(data: &[u8]) -> Result<Vec<TreeEntry>, String> {
    let mut out = Vec::new();
    let mut rest = data;
    while !rest.is_empty() {
        let space = rest.iter().position(|&b| b == b' ').ok_or("a tree entry has no mode")?;
        let mode_text = &rest[..space];
        if mode_text.is_empty() || mode_text.len() > 7 || !mode_text.iter().all(|b| (b'0'..=b'7').contains(b)) {
            return Err("a tree entry has a malformed mode".to_owned());
        }
        let mode = mode_text.iter().fold(0u32, |m, &b| (m << 3) | u32::from(b - b'0'));
        rest = &rest[space + 1..];
        let nul = rest.iter().position(|&b| b == 0).ok_or("a tree entry has no name end")?;
        let name = &rest[..nul];
        if name.is_empty() || name.contains(&b'/') {
            return Err("a tree entry has a malformed name".to_owned());
        }
        let oid = rest.get(nul + 1..nul + 21).and_then(Oid::from_bytes).ok_or("a tree entry is truncated")?;
        out.push(TreeEntry {
            mode,
            name: name.to_vec(),
            oid,
        });
        rest = &rest[nul + 21..];
    }
    Ok(out)
}

/// The raw form of a tree, in the order given: modes in octal without leading zeros (`40000` for a tree).
#[must_use]
pub fn serialize_tree(entries: &[TreeEntry]) -> Vec<u8> {
    let mut out = Vec::with_capacity(entries.len() * 40);
    for e in entries {
        out.extend_from_slice(format!("{:o} ", e.mode).as_bytes());
        out.extend_from_slice(&e.name);
        out.push(0);
        out.extend_from_slice(&e.oid.0);
    }
    out
}

/// Sorts entries the way git stores them: by name, a tree's name compared as if it ended with `/`.
pub fn sort_tree(entries: &mut [TreeEntry]) {
    fn key(e: &TreeEntry) -> impl Iterator<Item = u8> + '_ {
        e.name.iter().copied().chain((e.kind() == EntryKind::Tree).then_some(b'/'))
    }
    entries.sort_by(|a, b| key(a).cmp(key(b)));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_round_trip_and_reject_garbage() {
        let hex = "e69de29bb2d1d6434b8b29ae775ad8c2e48c5391";
        let oid = Oid::from_hex(hex).unwrap();
        assert_eq!(oid.to_hex(), hex);
        assert_eq!(Oid::from_hex(&hex.to_uppercase()), Some(oid));
        for bad in ["", "e69d", &hex[..39], &format!("{hex}0"), "g69de29bb2d1d6434b8b29ae775ad8c2e48c5391"] {
            assert_eq!(Oid::from_hex(bad), None, "{bad}");
        }
        assert!(Oid::ZERO.is_zero());
        assert_eq!(Oid::ZERO.to_hex(), rewarden_proto::desktop::ZERO_OID);
    }

    #[test]
    fn hashing_matches_git() {
        // `git hash-object -t blob /dev/null` and `printf 'hello\n' | git hash-object --stdin`.
        assert_eq!(hash_object(ObjectKind::Blob, b"").to_hex(), "e69de29bb2d1d6434b8b29ae775ad8c2e48c5391");
        assert_eq!(hash_object(ObjectKind::Blob, b"hello\n").to_hex(), "ce013625030ba8dba906f756967f9e9ca394464a");
        assert_eq!(hash_object(ObjectKind::Tree, b"").to_hex(), "4b825dc642cb6eb9a060e54bf8d69288fbee4904");
    }

    #[test]
    fn commits_parse_with_signatures_and_odd_headers() {
        let raw = b"tree 4b825dc642cb6eb9a060e54bf8d69288fbee4904\n\
parent 1111111111111111111111111111111111111111\n\
parent 2222222222222222222222222222222222222222\n\
author Ada Lovelace <ada@example.com> 1700000000 +0100\n\
committer C <c@example.com> 1700000100 -0500\n\
gpgsig -----BEGIN PGP SIGNATURE-----\n \n tree 3333333333333333333333333333333333333333\n -----END PGP SIGNATURE-----\n\
\n\n\nFix the thing\n\nLonger body.\n";
        let c = parse_commit(raw).unwrap();
        assert_eq!(c.tree.to_hex(), "4b825dc642cb6eb9a060e54bf8d69288fbee4904");
        assert_eq!(c.parents.len(), 2);
        assert_eq!(c.author, "Ada Lovelace <ada@example.com>");
        assert_eq!(c.author_time, 1_700_000_000);
        assert_eq!(c.commit_time, 1_700_000_100);
        assert_eq!(c.subject, "Fix the thing");

        let bare = parse_commit(b"tree 4b825dc642cb6eb9a060e54bf8d69288fbee4904\nauthor nobody\n").unwrap();
        assert_eq!((bare.author.as_str(), bare.author_time, bare.subject.as_str()), ("nobody", 0, ""));
        assert!(parse_commit(b"parent 1111111111111111111111111111111111111111\n\nx").is_err());
        assert!(parse_commit(b"tree nope\n\nx").is_err());
        assert!(parse_commit(b"").is_err());
    }

    #[test]
    fn tags_parse() {
        let raw = b"object 4b825dc642cb6eb9a060e54bf8d69288fbee4904\ntype commit\ntag v1.0\ntagger T <t@x> 1 +0000\n\nRelease\n";
        let t = parse_tag(raw).unwrap();
        assert_eq!((t.kind, t.name.as_str()), (ObjectKind::Commit, "v1.0"));
        assert!(parse_tag(b"object 4b825dc642cb6eb9a060e54bf8d69288fbee4904\ntype sock\n").is_err());
        assert!(parse_tag(b"type commit\n").is_err());
    }

    #[test]
    fn trees_round_trip_and_reject_garbage() {
        let a = Oid([1; 20]);
        let mut entries = vec![
            TreeEntry {
                mode: mode::BLOB,
                name: b"a.txt".to_vec(),
                oid: a,
            },
            TreeEntry {
                mode: mode::TREE,
                name: b"a".to_vec(),
                oid: a,
            },
            TreeEntry {
                mode: mode::GITLINK,
                name: b"a-b".to_vec(),
                oid: a,
            },
        ];
        sort_tree(&mut entries);
        // "a-b" < "a.txt" < "a/" in bytes.
        let names: Vec<_> = entries.iter().map(|e| e.name.clone()).collect();
        assert_eq!(names, [b"a-b".to_vec(), b"a.txt".to_vec(), b"a".to_vec()]);
        let raw = serialize_tree(&entries);
        assert!(raw.windows(6).any(|w| w == b"40000 "));
        assert_eq!(parse_tree(&raw).unwrap(), entries);
        assert_eq!(entries[2].kind(), EntryKind::Tree);
        assert_eq!(entries[0].kind(), EntryKind::Submodule);
        assert_eq!(EntryKind::of(mode::LINK), EntryKind::Link);
        assert_eq!(EntryKind::of(mode::EXEC), EntryKind::File);

        for bad in
            [&raw[..raw.len() - 1], b"100644 a\0", b"10064x a\0aaaaaaaaaaaaaaaaaaaa", b"100644 \0aaaaaaaaaaaaaaaaaaaa"]
        {
            assert!(parse_tree(bad).is_err());
        }
        assert!(parse_tree(b"100644 a/b\0aaaaaaaaaaaaaaaaaaaa").is_err());
        assert!(parse_tree(b"").unwrap().is_empty());
    }
}
