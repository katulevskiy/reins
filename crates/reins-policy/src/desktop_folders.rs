//! The folders a computer's agents work in, as the phone signs them for it (`desktop_policy::DesktopPolicy::roots` and
//! `scratch`). The computer proposes its own `[workspace]` roots and scratch folders, canonicalized there
//! ([`reins_proto::desktop::FolderProposal`]); the owner confirms each on the phone (a loosening: the owner check), and
//! only then does it count on the computer. Removing one only tightens.
//!
//! A folder is refused, on the computer before it proposes it, on the phone before it shows or signs it, and on the
//! computer again before it decides with it, when it is not an absolute canonical path, when it is as broad as a whole
//! disk, the home folder or a folder above it (`/`, `~`, `/home/<user>`, `/Users/<user>`), or when it is or holds a
//! folder of keys or Reins' own state ([`refusal`]).
//!
//! Pure: the computer resolves symlinks itself and passes the results in.

use reins_proto::desktop::{Folder, FolderKind, MAX_FOLDER_CHARS};

use crate::effects::normalize;

/// Why a folder is never signed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refused {
    /// Not an absolute path (`~/x`, `x`, `C:\x`).
    NotAbsolute,
    /// `.`, `..`, an empty part or a trailing `/`: not what the computer resolved.
    NotCanonical,
    /// `/`, a whole disk, the home folder or a folder above it.
    TooBroad,
    /// Keys (`~/.ssh`, `~/.gnupg`, ...) or Reins' own state, or a folder holding them.
    Sensitive,
    TooLong,
    /// Control characters.
    Unprintable,
}

impl Refused {
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::NotAbsolute => "not an absolute path",
            Self::NotCanonical => "not a canonical path",
            Self::TooBroad => "too broad",
            Self::Sensitive => "holds keys or Reins' own files",
            Self::TooLong => "too long",
            Self::Unprintable => "unprintable",
        }
    }
}

/// Folders whose children are whole homes or disks: `/home/<user>`, `/Users/<user>`, `/mnt/<disk>`, ...
const BROAD_PARENTS: &[&str] = &[
    "home",
    "users",
    "mnt",
    "media",
    "volumes",
    "run",
    "var",
    "usr",
    "etc",
    "private",
    "system",
    "library",
    "applications",
    "proc",
    "sys",
    "dev",
    "boot",
    "nix",
    "snap",
    "opt",
    "srv",
];

/// Parts that name a folder of keys or credentials.
const KEY_PARTS: &[&str] = &[".ssh", ".gnupg", ".aws", ".azure", ".kube", ".docker", ".password-store", "keychains"];

/// Reins' own folders below the home (its state, the pinned keys, its configuration).
const REINS_UNDER_HOME: &[&str] = &[
    ".ssh",
    ".gnupg",
    ".aws",
    ".config/reins",
    ".local/state/reins",
    ".local/share/reins",
    "Library/Application Support/Reins",
    "Library/Application Support/dev.reins.desktop",
];

fn is_glob(part: &str) -> bool {
    part.contains(['*', '?', '['])
}

fn parts(path: &str) -> Vec<&str> {
    path.split('/').filter(|p| !p.is_empty()).collect()
}

/// The parts of `path` before its first glob part (`/a/b/*/c` → `/a/b`).
#[must_use]
pub fn literal_prefix(path: &str) -> String {
    let literal: Vec<&str> = parts(path).into_iter().take_while(|p| !is_glob(p)).collect();
    format!("/{}", literal.join("/"))
}

/// `path` is `root` or inside it, both absolute and lexical (no globs in `root`'s parts are expanded here: a pattern
/// root holds what its parts match).
fn holds(root: &str, path: &str) -> bool {
    let (r, p) = (parts(root), parts(path));
    r.len() <= p.len() && r.iter().zip(&p).all(|(a, b)| a == b || (is_glob(a) && glob_part(a, b)))
}

/// One path part against one glob part (`*` and `?`; a `[` set is read as a literal).
fn glob_part(pattern: &str, text: &str) -> bool {
    fn go(p: &[char], t: &[char]) -> bool {
        match (p.first(), t.first()) {
            (None, None) => true,
            (Some('*'), _) => go(&p[1..], t) || (!t.is_empty() && go(p, &t[1..])),
            (Some('?'), Some(_)) => go(&p[1..], &t[1..]),
            (Some(a), Some(b)) if a == b => go(&p[1..], &t[1..]),
            _ => false,
        }
    }
    let (p, t): (Vec<char>, Vec<char>) = (pattern.chars().collect(), text.chars().collect());
    go(&p, &t)
}

/// Why `path` (a computer's folder, with `home` that computer's home folder; empty when not known) is never signed;
/// `None`: it may be.
#[must_use]
pub fn refusal(path: &str, home: &str) -> Option<Refused> {
    if path.chars().any(char::is_control) {
        return Some(Refused::Unprintable);
    }
    if path.len() > MAX_FOLDER_CHARS {
        return Some(Refused::TooLong);
    }
    if !path.starts_with('/') {
        return Some(Refused::NotAbsolute);
    }
    if path != "/"
        && (normalize(path, "/", "") != path || path.split('/').skip(1).any(|p| p.is_empty() || p == "." || p == ".."))
    {
        return Some(Refused::NotCanonical);
    }
    let home = home.trim_end_matches('/');
    let prefix = literal_prefix(path);
    let lit = parts(&prefix);
    let lower: Vec<String> = lit.iter().map(|p| p.to_ascii_lowercase()).collect();
    let broad = lit.len() < 2
        || (lit.len() == 2 && BROAD_PARENTS.contains(&lower[0].as_str()))
        || (lit.len() == 3 && lower[0] == "var" && lower[1] == "home")
        || (!home.is_empty() && holds(&prefix, home));
    if broad {
        return Some(Refused::TooBroad);
    }
    let keys = parts(path).iter().any(|p| KEY_PARTS.contains(&p.to_ascii_lowercase().as_str()));
    let reins = !home.is_empty()
        && REINS_UNDER_HOME.iter().any(|r| {
            let own = format!("{home}/{r}");
            holds(path, &own) || holds(&own, path)
        });
    let reins_anywhere = {
        let p = path.to_ascii_lowercase();
        ["/.config/reins", "/.local/state/reins", "/.local/share/reins", "/application support/reins"]
            .iter()
            .any(|r| p.contains(&format!("{r}/")) || p.ends_with(r))
    };
    if keys || reins || reins_anywhere {
        return Some(Refused::Sensitive);
    }
    None
}

/// `path` (absolute) is `root` or inside it, `root` read with `home` for `~`.
#[must_use]
pub fn inside(root: &str, path: &str, home: &str) -> bool {
    let (r, p) = (normalize(root, "/", home), normalize(path, "/", home));
    let r = r.trim_end_matches('/');
    !r.is_empty() && r.starts_with('/') && (p == r || p.starts_with(&format!("{r}/")))
}

/// `a` and `b` name the same folder (lexically, `~` read as `home`).
#[must_use]
pub fn same(a: &str, b: &str, home: &str) -> bool {
    normalize(a, "/", home).trim_end_matches('/') == normalize(b, "/", home).trim_end_matches('/')
}

/// Whether `folder` already counts with `roots` and `scratch` signed: a root inside a signed root; a scratch folder
/// signed as such, or strictly inside a signed root.
#[must_use]
pub fn counted(folder: &Folder, roots: &[String], scratch: &[String], home: &str) -> bool {
    match folder.kind {
        FolderKind::Root => roots.iter().any(|r| inside(r, &folder.path, home)),
        FolderKind::Scratch => {
            scratch.iter().any(|s| same(s, &folder.path, home))
                || roots.iter().any(|r| inside(r, &folder.path, home) && !same(r, &folder.path, home))
        }
    }
}

/// The folders of a proposal the phone may show, in order, once each: refused ones and those already counted with
/// `roots` and `scratch` left out.
#[must_use]
pub fn open(proposed: &[Folder], roots: &[String], scratch: &[String], home: &str) -> Vec<Folder> {
    let mut out: Vec<Folder> = Vec::new();
    for f in proposed {
        if refusal(&f.path, home).is_none() && !counted(f, roots, scratch, home) && !out.contains(f) {
            out.push(f.clone());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOME: &str = "/home/me";

    fn root(path: &str) -> Folder {
        Folder {
            path: path.to_owned(),
            kind: FolderKind::Root,
        }
    }

    fn scratch(path: &str) -> Folder {
        Folder {
            path: path.to_owned(),
            kind: FolderKind::Scratch,
        }
    }

    #[test]
    fn broad_roots_are_refused() {
        for broad in [
            "/",
            "/home",
            "/home/me",
            "/home/other",
            "/Users/me",
            "/var/home/me",
            "/mnt/data",
            "/Volumes/Disk",
            "/usr/lib",
            "/etc",
            "/tmp",
            "/home/*",
            "/home/me/*",
            "/*/dev",
        ] {
            assert_eq!(refusal(broad, HOME), Some(Refused::TooBroad), "{broad}");
        }
        // Not knowing the computer's home doesn't let it through.
        assert_eq!(refusal("/Users/me", ""), Some(Refused::TooBroad));
        assert_eq!(refusal("/home/me", ""), Some(Refused::TooBroad));
        for fine in [
            "/home/me/Documents/dev",
            "/home/me/.claude/projects/*/memory",
            "/mnt/data/dev",
            "/work/projects",
            "/Users/me/code",
        ] {
            assert_eq!(refusal(fine, HOME), None, "{fine}");
        }
    }

    #[test]
    fn only_canonical_absolute_paths() {
        assert_eq!(refusal("~", HOME), Some(Refused::NotAbsolute));
        assert_eq!(refusal("~/Documents/dev", HOME), Some(Refused::NotAbsolute));
        assert_eq!(refusal("Documents", HOME), Some(Refused::NotAbsolute));
        assert_eq!(refusal("C:\\Users\\me\\dev", HOME), Some(Refused::NotAbsolute));
        assert_eq!(refusal("/home/me/dev/../..", HOME), Some(Refused::NotCanonical));
        assert_eq!(refusal("/home/me/./dev", HOME), Some(Refused::NotCanonical));
        assert_eq!(refusal("/home/me//dev", HOME), Some(Refused::NotCanonical));
        assert_eq!(refusal("/home/me/dev/", HOME), Some(Refused::NotCanonical));
        assert_eq!(refusal("/home/me/dev\u{1b}[2J", HOME), Some(Refused::Unprintable));
        assert_eq!(refusal(&format!("/home/me/{}", "a".repeat(600)), HOME), Some(Refused::TooLong));
    }

    #[test]
    fn keys_and_reins_state_are_refused() {
        for sensitive in [
            "/home/me/.ssh",
            "/home/me/dev/.ssh",
            "/home/me/.gnupg/private",
            "/home/me/.config",
            "/home/me/.config/reins",
            "/home/me/.local",
            "/home/me/.local/state/reins/x",
            "/home/me/Library/Application Support/Reins",
            "/data/.aws",
        ] {
            assert_eq!(refusal(sensitive, HOME), Some(Refused::Sensitive), "{sensitive}");
        }
        assert_eq!(refusal("/home/me/.config/zed", HOME), None, "another app's settings are the owner's to say");
    }

    #[test]
    fn counted_and_open_folders() {
        let roots = vec!["~/Documents/dev".to_owned()];
        let signed_scratch = vec!["/home/me/.claude/projects/*/memory".to_owned()];
        assert!(counted(&root("/home/me/Documents/dev"), &roots, &[], HOME));
        assert!(counted(&root("/home/me/Documents/dev/app"), &roots, &[], HOME));
        assert!(!counted(&root("/home/me/work"), &roots, &[], HOME));
        assert!(counted(&scratch("/home/me/Documents/dev/tmp"), &roots, &[], HOME), "strictly inside a root");
        assert!(!counted(&scratch("/home/me/Documents/dev"), &roots, &[], HOME), "a root is no scratch folder");
        assert!(counted(&scratch("/home/me/.claude/projects/*/memory"), &roots, &signed_scratch, HOME));
        let proposed = vec![
            root("/home/me/Documents/dev"),
            root("/home/me"),
            scratch("/home/me/.claude/projects/*/memory"),
            scratch("/home/me/.claude/projects/*/memory"),
            scratch("/home/me/.zeron/uploads"),
            root("/mnt/data/dev"),
        ];
        assert_eq!(
            open(&proposed, &roots, &[], HOME),
            [scratch("/home/me/.claude/projects/*/memory"), scratch("/home/me/.zeron/uploads"), root("/mnt/data/dev")]
        );
        assert_eq!(open(&proposed, &roots, &signed_scratch, HOME).len(), 2);
    }

    #[test]
    fn globs_match_part_by_part() {
        assert!(holds("/home/me/.claude/projects/*/memory", "/home/me/.claude/projects/app/memory/x.md"));
        assert!(!holds("/home/me/.claude/projects/*/memory", "/home/me/.claude/projects/app/other"));
        assert_eq!(literal_prefix("/home/me/.claude/projects/*/memory"), "/home/me/.claude/projects");
    }
}
