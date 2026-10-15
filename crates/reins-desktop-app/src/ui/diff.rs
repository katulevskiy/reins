//! A pushed commit's diff, shown safely, and where the commits are on the host.
//!
//! Diff text is the repository's content, so it is untrusted. It is only ever shown as plain text, and the
//! characters that could change how it reads are made visible instead of acting:
//! - control characters, as their Control Pictures (`␛`);
//! - bidirectional overrides and isolates, zero-width characters and line separators, by their code point
//!   (`⟨U+202E⟩`).
//!
//! The same characters are left out of what a sentence quotes ([`hidden`]).

use std::fmt::Write as _;

/// What the host calls its own page of a branch's commits, for "Open on GitHub".
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostLink {
    pub name: &'static str,
    pub url: String,
}

/// A character that changes how text around it reads, or is not seen at all: bidirectional marks, overrides and
/// isolates, zero-width and joining characters, line and paragraph separators, soft hyphens, invisible fillers,
/// variation selectors and tags. C1 controls too ([`char::is_control`] covers C0, DEL and C1).
#[must_use]
pub fn hidden(c: char) -> bool {
    c.is_control()
        || matches!(
            c,
            '\u{00ad}'
                | '\u{034f}'
                | '\u{061c}'
                | '\u{115f}'
                | '\u{1160}'
                | '\u{17b4}'
                | '\u{17b5}'
                | '\u{180b}'..='\u{180f}'
                | '\u{200b}'..='\u{200f}'
                | '\u{2028}'..='\u{202e}'
                | '\u{2060}'..='\u{206f}'
                | '\u{3164}'
                | '\u{fe00}'..='\u{fe0f}'
                | '\u{feff}'
                | '\u{ffa0}'
                | '\u{fff0}'..='\u{fffb}'
                | '\u{e0000}'..='\u{e007f}'
        )
}

/// `line` with every character that could act shown as itself instead.
#[must_use]
pub fn visible(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    for c in line.chars() {
        match c {
            // Tabs only indent.
            '\t' => out.push_str("    "),
            '\u{0}'..='\u{1f}' => out.push(char::from_u32(0x2400 + u32::from(c)).unwrap_or('\u{fffd}')),
            '\u{7f}' => out.push('\u{2421}'),
            c if hidden(c) => {
                let _infallible = write!(out, "⟨U+{:04X}⟩", u32::from(c));
            }
            _ => out.push(c),
        }
    }
    out
}

/// A path segment of a URL: letters, digits and `-._~` as they are, the rest percent-encoded (`/` kept between
/// segments by the caller).
fn segment(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
            out.push(char::from(b));
        } else {
            let _infallible = write!(out, "%{b:02X}");
        }
    }
    out
}

/// The host's page of `branch`'s commits in `repo` (`github.com/acme/web`), on the hosts Reins knows.
#[must_use]
pub fn commits_link(repo: &str, branch: &str) -> Option<HostLink> {
    let (host, path) = repo.split_once('/')?;
    let path: Vec<String> = path.split('/').filter(|p| !p.is_empty()).map(segment).collect();
    let branch: Vec<String> = branch.split('/').map(segment).collect();
    if path.is_empty() || branch.iter().any(String::is_empty) {
        return None;
    }
    let (path, branch) = (path.join("/"), branch.join("/"));
    let (name, url) = match host.to_ascii_lowercase().as_str() {
        "github.com" => ("GitHub", format!("https://github.com/{path}/commits/{branch}")),
        "gitlab.com" => ("GitLab", format!("https://gitlab.com/{path}/-/commits/{branch}")),
        "codeberg.org" => ("Codeberg", format!("https://codeberg.org/{path}/commits/branch/{branch}")),
        "bitbucket.org" => ("Bitbucket", format!("https://bitbucket.org/{path}/commits/branch/{branch}")),
        _ => return None,
    };
    Some(HostLink {
        name,
        url,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn controls_and_bidi_are_shown_never_acted_on() {
        assert_eq!(visible("a\u{1b}[31mred"), "a\u{241b}[31mred");
        assert_eq!(visible("x\u{0}y\u{7f}"), "x\u{2400}y\u{2421}");
        assert_eq!(visible("if (admin\u{202e} \u{2066}"), "if (admin⟨U+202E⟩ ⟨U+2066⟩");
        assert_eq!(visible("zero\u{200b}width\u{feff}"), "zero⟨U+200B⟩width⟨U+FEFF⟩");
        assert_eq!(visible("\tindent"), "    indent");
        assert_eq!(visible("plain ünïcode → ok"), "plain ünïcode → ok");
        for c in visible("\u{1}\u{85}\u{2028}\u{2069}\u{61c}").chars() {
            assert!(!c.is_control(), "{c:?}");
            assert!(!matches!(c, '\u{2028}'..='\u{202e}' | '\u{2066}'..='\u{2069}' | '\u{061c}'), "{c:?}");
        }
    }

    #[test]
    fn hosts_link_to_the_branchs_commits() {
        assert_eq!(
            commits_link("github.com/katulevskiy/reins", "signal/desktop-release").unwrap().url,
            "https://github.com/katulevskiy/reins/commits/signal/desktop-release"
        );
        let gitlab = commits_link("gitlab.com/group/sub/proj", "feature/x").unwrap();
        assert_eq!(
            (gitlab.name, gitlab.url.as_str()),
            ("GitLab", "https://gitlab.com/group/sub/proj/-/commits/feature/x")
        );
        assert_eq!(
            commits_link("github.com/o/r", "we ird#?").unwrap().url,
            "https://github.com/o/r/commits/we%20ird%23%3F"
        );
        assert_eq!(commits_link("git.example.com/o/r", "main"), None);
        assert_eq!(commits_link("github.com/o/r", "a//b"), None);
        assert_eq!(commits_link("github.com", "main"), None);
    }
}
