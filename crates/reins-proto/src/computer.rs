//! A computer's name, as the desktop app gives it when it pairs and as the phone shows it.
//!
//! The desktop app registers with the server as `<computer name> (<OS>)` ("M2A (macOS)"): the server keeps only a
//! client name, and the phone reads the computer's name and its platform back out of it ([`parse_client_name`]).
//! Older apps registered as `Reins desktop app on <hostname>` ("Reins desktop app on Mac.localdomain"); those read
//! as the hostname without its domain ("Mac"), the platform guessed from it when the name says.

/// The desktop platforms, each with a logo on the phone.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Os {
    MacOs,
    Windows,
    Linux,
}

impl Os {
    pub const ALL: [Self; 3] = [Self::MacOs, Self::Windows, Self::Linux];

    /// What the phone stores and draws: `macos`, `windows`, `linux`.
    #[must_use]
    pub const fn key(self) -> &'static str {
        match self {
            Self::MacOs => "macos",
            Self::Windows => "windows",
            Self::Linux => "linux",
        }
    }

    /// The name in the client name's marker: `macOS`, `Windows`, `Linux`.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::MacOs => "macOS",
            Self::Windows => "Windows",
            Self::Linux => "Linux",
        }
    }

    #[must_use]
    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|o| o.key() == key)
    }

    /// The platform this binary was built for, if it is one of the three.
    #[must_use]
    pub const fn current() -> Option<Self> {
        if cfg!(target_os = "macos") {
            Some(Self::MacOs)
        } else if cfg!(windows) {
            Some(Self::Windows)
        } else if cfg!(target_os = "linux") {
            Some(Self::Linux)
        } else {
            None
        }
    }
}

/// The prefix older desktop apps put before the hostname.
pub const LEGACY_PREFIX: &str = "Reins desktop app on ";

/// The longest computer name kept (characters).
pub const MAX_NAME_CHARS: usize = 60;

/// Domain suffixes taken off a name that has spaces in it (one without spaces is a hostname: everything after its
/// first dot goes).
const DOMAIN_SUFFIXES: &[&str] = &[".localdomain", ".local", ".lan", ".home.arpa", ".home", ".internal", ".localhost"];

/// A computer's name as people say it: one line, no control characters, without the domain a hostname carries
/// ("Mac.localdomain" → "Mac", "honeypot.lan" → "honeypot", "Dana's MacBook.local" → "Dana's MacBook"), at most
/// [`MAX_NAME_CHARS`]. Empty when nothing is left.
#[must_use]
pub fn clean_name(name: &str) -> String {
    let one: String = name
        .chars()
        .map(|c| {
            if c.is_control() {
                ' '
            } else {
                c
            }
        })
        .collect();
    let mut n = one.split_whitespace().collect::<Vec<_>>().join(" ");
    if n.contains(' ') {
        loop {
            let lower = n.to_ascii_lowercase();
            let Some(suffix) = DOMAIN_SUFFIXES.iter().find(|s| lower.ends_with(*s) && lower.len() > s.len()) else {
                break;
            };
            n.truncate(n.len() - suffix.len());
            n = n.trim_end().to_owned();
        }
    } else if let Some(dot) = n.find('.') {
        // A hostname: its first label is the computer's ("m2a.example.com" → "m2a").
        n.truncate(dot);
    }
    n.trim_matches(|c: char| c == '.' || c.is_whitespace()).chars().take(MAX_NAME_CHARS).collect()
}

/// The client name the desktop app registers with: the computer's name, then its platform in brackets
/// ("M2A (macOS)"). "Computer" when the name is empty.
#[must_use]
pub fn client_name(computer: &str, os: Option<Os>) -> String {
    let name = clean_name(computer);
    let name = if name.is_empty() {
        "Computer".to_owned()
    } else {
        name
    };
    match os {
        Some(os) => format!("{name} ({})", os.name()),
        None => name,
    }
}

/// A computer's name and platform from a client name or a connection's label: the desktop app's own
/// ("M2A (macOS)" → "M2A", macOS), an older app's ("Reins desktop app on Mac.localdomain" → "Mac", macOS by its
/// name), or anything else, unchanged and without a platform.
#[must_use]
pub fn parse_client_name(name: &str) -> (String, Option<Os>) {
    let trimmed = name.trim();
    for os in Os::ALL {
        let marker = format!(" ({})", os.name());
        if let Some(rest) = trimmed.strip_suffix(&marker)
            && !rest.trim().is_empty()
        {
            let (inner, _) = parse_client_name(rest);
            return (inner, Some(os));
        }
    }
    if trimmed.eq_ignore_ascii_case(LEGACY_PREFIX.trim_end()) {
        return ("Computer".to_owned(), None);
    }
    if let Some(host) = strip_prefix_ignore_case(trimmed, LEGACY_PREFIX) {
        let host = host.trim();
        if host.is_empty() || host.eq_ignore_ascii_case("this computer") {
            return ("Computer".to_owned(), None);
        }
        let os = guess_os(host);
        let name = clean_name(host);
        return (
            if name.is_empty() {
                "Computer".to_owned()
            } else {
                name
            },
            os,
        );
    }
    (trimmed.to_owned(), None)
}

fn strip_prefix_ignore_case<'a>(s: &'a str, prefix: &str) -> Option<&'a str> {
    let head = s.get(..prefix.len())?;
    head.eq_ignore_ascii_case(prefix).then(|| &s[prefix.len()..])
}

/// The platform a bare hostname gives away: a Mac's own default names ("Mac", "Danas-MacBook-Pro", "iMac",
/// "Mac-mini", "Mac.localdomain") and Windows' generated ones ("DESKTOP-4F2K9QZ", "LAPTOP-1A2B3C4D").
fn guess_os(host: &str) -> Option<Os> {
    let first = host.split('.').next().unwrap_or_default().to_ascii_lowercase();
    let words: Vec<&str> = first.split(['-', '_', ' ', '\'']).filter(|w| !w.is_empty()).collect();
    if words.iter().any(|w| matches!(*w, "mac" | "imac" | "macbook" | "macmini" | "macpro" | "macstudio"))
        || first.contains("macbook")
    {
        return Some(Os::MacOs);
    }
    let windows_generated = |prefix: &str| {
        first
            .strip_prefix(prefix)
            .is_some_and(|rest| rest.len() >= 6 && rest.chars().all(|c| c.is_ascii_alphanumeric()))
    };
    if windows_generated("desktop-") || windows_generated("laptop-") || windows_generated("win-") {
        return Some(Os::Windows);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_hostname_loses_its_domain() {
        assert_eq!(clean_name("Mac.localdomain"), "Mac");
        assert_eq!(clean_name("M2A.local"), "M2A");
        assert_eq!(clean_name("honeypot"), "honeypot");
        assert_eq!(clean_name("honeypot.lan"), "honeypot");
        assert_eq!(clean_name("ws-01.corp.example.com"), "ws-01");
        assert_eq!(clean_name("Dana's MacBook Pro.local"), "Dana's MacBook Pro");
        assert_eq!(clean_name("Dana's MacBook Pro"), "Dana's MacBook Pro");
        assert_eq!(clean_name("  Studio \n Mac  "), "Studio Mac");
        assert_eq!(clean_name(".localdomain"), "");
        assert_eq!(clean_name(""), "");
        assert_eq!(clean_name(&"x".repeat(100)).len(), MAX_NAME_CHARS);
    }

    #[test]
    fn the_desktop_app_names_itself_after_the_computer_and_its_platform() {
        assert_eq!(client_name("M2A", Some(Os::MacOs)), "M2A (macOS)");
        assert_eq!(client_name("Mac.localdomain", Some(Os::MacOs)), "Mac (macOS)");
        assert_eq!(client_name("honeypot", Some(Os::Linux)), "honeypot (Linux)");
        assert_eq!(client_name("DESKTOP-4F2K9QZ", Some(Os::Windows)), "DESKTOP-4F2K9QZ (Windows)");
        assert_eq!(client_name("", Some(Os::Linux)), "Computer (Linux)");
        assert_eq!(client_name("box", None), "box");
    }

    #[test]
    fn the_phone_reads_the_name_and_the_platform_back() {
        for os in Os::ALL {
            for name in ["M2A", "honeypot", "Dana's MacBook Pro"] {
                assert_eq!(parse_client_name(&client_name(name, Some(os))), (name.to_owned(), Some(os)));
            }
        }
        assert_eq!(parse_client_name("Claude"), ("Claude".to_owned(), None));
        assert_eq!(parse_client_name("ChatGPT (beta)"), ("ChatGPT (beta)".to_owned(), None));
        assert_eq!(parse_client_name(" (macOS)"), ("(macOS)".to_owned(), None));
    }

    #[test]
    fn an_older_apps_long_name_becomes_the_computers() {
        assert_eq!(parse_client_name("Reins desktop app on Mac.localdomain"), ("Mac".to_owned(), Some(Os::MacOs)));
        assert_eq!(parse_client_name("Reins desktop app on honeypot"), ("honeypot".to_owned(), None));
        assert_eq!(
            parse_client_name("Reins desktop app on Danas-MacBook-Pro.local"),
            ("Danas-MacBook-Pro".to_owned(), Some(Os::MacOs))
        );
        assert_eq!(
            parse_client_name("Reins desktop app on DESKTOP-4F2K9QZ"),
            ("DESKTOP-4F2K9QZ".to_owned(), Some(Os::Windows))
        );
        assert_eq!(parse_client_name("reins desktop app on studio"), ("studio".to_owned(), None));
        assert_eq!(parse_client_name("Reins desktop app on this computer"), ("Computer".to_owned(), None));
        assert_eq!(parse_client_name("Reins desktop app on "), ("Computer".to_owned(), None));
        // A name that merely mentions a Mac is not a Mac's default name.
        assert_eq!(parse_client_name("Reins desktop app on macarena").1, None);
    }
}
