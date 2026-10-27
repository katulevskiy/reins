//! The names the user gives connections ("Claude (work)", "Home Mac"), and how every screen shows them.
//!
//! - **Which name.** A connection goes by the name the user gave it when it has one, else by its own: the name the
//!   server keeps for it as [`crate::avatar::shown_label`] shows it (what the AI or computer called itself, or a name
//!   typed when it paired with an older version). Every view the apps draw a connection's name from goes through
//!   [`Names`], so the Connections list, Activity rows, waiting cards, notifications, widgets and Live Activities agree.
//! - **Where it is kept.** In the account state (`Store::set_connection_name`), sealed with the account's key and
//!   synced to every phone of the account: the server never has it. A computer is sent a copy sealed to its own key
//!   (`Engine::send_computer_name`), which only the desktop app can open, so that it shows the same name.
//! - **Trusted and untrusted.** What a connection calls itself is chosen by whoever is on the other side: it stays
//!   cleaned as before ([`original_name`]) and is shown as a quiet fact under the name on the connection's page, so a
//!   name the user gave never hides what the connection is, and it never becomes the user's name by itself. A name
//!   the user typed is trusted, but is still one line of at most [`MAX_NAME_CHARS`] characters with no control or
//!   invisible characters ([`typed_name`]).
//! - **The same AI twice.** Nothing an AI client sends says which of its accounts connects: two Claude.ai accounts
//!   look the same (one client id, one name, one redirect host). A new connection of a client already connected is
//!   offered a name of its own ("Claude 2", [`suggested_name`]) for the user to change on the spot.

use std::collections::HashMap;

use crate::text;
use crate::types::{ActivityEntry, ApprovalView, BlobView, GrantView, PendingItem};

/// The longest name, typed or not.
pub const MAX_NAME_CHARS: usize = 64;

/// A name the user typed, as it is kept: one line, without control or invisible characters (a zero-width space
/// could make two names look the same), at most [`MAX_NAME_CHARS`] characters. `None` when nothing is left: no name,
/// the connection goes by its own.
#[must_use]
pub fn typed_name(raw: &str) -> Option<String> {
    let visible: String = raw.chars().filter(|c| !text::is_invisible(*c)).collect();
    let line = text::truncate_chars(&text::one_line(&visible), MAX_NAME_CHARS);
    let line = line.trim();
    (!line.is_empty()).then(|| line.to_owned())
}

/// What a connection called itself, as its page shows it under the name: untrusted, so cleaned like every name from
/// the other side (one line, at most [`MAX_NAME_CHARS`] characters), and kept whole otherwise ("M2A (macOS)").
#[must_use]
pub fn original_name(client_name: &str) -> String {
    text::truncate_chars(&text::one_line(client_name), MAX_NAME_CHARS)
}

/// The largest number a connection is told apart by ("Claude 2", ...).
pub const MAX_NUMBER: u32 = 999;

/// `own` (a connection's own name as shown) with its number: "Claude 2". The number keeps its room: the name is cut,
/// never the number.
#[must_use]
pub fn numbered(own: &str, number: u32) -> String {
    let suffix = format!(" {number}");
    let room = MAX_NAME_CHARS.saturating_sub(suffix.chars().count());
    format!("{}{suffix}", text::truncate_chars(own, room).trim_end())
}

/// The number a new connection is told apart by when a connection of the same client is there already (`same_client`
/// of them): the first of 2, 3, ... that makes a name no connection goes by (`taken`, compared without case). `None`
/// for the first of its client.
#[must_use]
pub fn suggested_number(base: &str, same_client: usize, taken: &[String]) -> Option<u32> {
    if same_client == 0 {
        return None;
    }
    let base = crate::avatar::shown_label(base);
    let taken: Vec<String> = taken.iter().map(|t| t.to_lowercase()).collect();
    let first = u32::try_from(same_client + 1).unwrap_or(MAX_NUMBER).min(MAX_NUMBER);
    // One of these is free: there are fewer names taken than numbers tried.
    let last = u32::try_from(taken.len()).unwrap_or(MAX_NUMBER).saturating_add(first).min(MAX_NUMBER);
    Some((first..=last).find(|&n| !taken.contains(&numbered(&base, n).to_lowercase())).unwrap_or(first))
}

/// The name a new connection is offered when it pairs: its own (`base`, cleaned), or, with a number
/// ([`suggested_number`]), "Claude 2".
#[must_use]
pub fn suggested_name(base: &str, same_client: usize, taken: &[String]) -> String {
    let own = crate::avatar::shown_label(base);
    match suggested_number(base, same_client, taken) {
        Some(n) => numbered(&own, n),
        None => own,
    }
}

/// Whether two connections are the same client: the same name (as shown, without case) from the same host.
#[must_use]
pub fn same_client(name: &str, host: &str, other_name: &str, other_host: &str) -> bool {
    let shown = |n: &str| crate::avatar::shown_label(n).to_lowercase();
    !host.is_empty() && host.eq_ignore_ascii_case(other_host) && shown(name) == shown(other_name)
}

/// The names the user gave connections, and the numbers the core gave second connections of a client, by connection
/// id, applied to the views the apps draw.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Names {
    given: HashMap<String, String>,
    numbers: HashMap<String, u32>,
}

impl Names {
    /// The names and numbers as kept (`Store::connection_names`, `Store::connection_numbers`). Each name is cleaned
    /// again: one that came from another phone is held to the same rules, and one that cannot be a name is left out;
    /// so is a number out of range.
    #[must_use]
    pub fn new(kept: HashMap<String, String>, numbers: HashMap<String, u32>) -> Self {
        Self {
            given: kept.into_iter().filter_map(|(id, name)| Some((id, typed_name(&name)?))).collect(),
            numbers: numbers.into_iter().filter(|(_, n)| (2..=MAX_NUMBER).contains(n)).collect(),
        }
    }

    /// The name the user gave a connection.
    #[must_use]
    pub fn get(&self, connection_id: &str) -> Option<&str> {
        self.given.get(connection_id).map(String::as_str)
    }

    /// The number the core told a connection apart by (a second account of a client), if any. Not the user's: it
    /// belongs to the connection's own name.
    #[must_use]
    pub fn number(&self, connection_id: &str) -> Option<u32> {
        self.numbers.get(connection_id).copied()
    }

    /// A connection's own name with its number when it has one ("Claude 2"): what it goes by without a name of the
    /// user's. `own` is its own name, already cleaned.
    #[must_use]
    pub fn own(&self, connection_id: &str, own: &str) -> String {
        match self.number(connection_id) {
            Some(n) if !own.is_empty() => numbered(own, n),
            _ => own.to_owned(),
        }
    }

    /// A connection's name as shown: the user's, else its own with its number ([`Self::own`]).
    #[must_use]
    pub fn label(&self, connection_id: &str, own: &str) -> String {
        self.get(connection_id).map_or_else(|| self.own(connection_id, own), str::to_owned)
    }

    /// Puts the name `connection_id` is shown by in `label` (which holds its own name), and at the start of each of
    /// `lines` that start with its own name ("Claude wants to read 2 emails").
    fn rename(&self, connection_id: &str, label: &mut String, lines: &mut [&mut String]) {
        if label.is_empty() {
            return;
        }
        let name = self.label(connection_id, label);
        if name == *label {
            return;
        }
        for line in lines.iter_mut() {
            if let Some(rest) = line.strip_prefix(label.as_str()) {
                **line = format!("{name}{rest}");
            }
        }
        *label = name;
    }

    pub fn item(&self, item: &mut PendingItem) {
        let id = item.connection_id.clone();
        self.rename(&id, &mut item.connection_label, &mut [&mut item.title, &mut item.headline]);
    }

    pub fn approval(&self, view: &mut ApprovalView) {
        let id = view.connection_id.clone();
        self.rename(&id, &mut view.connection_label, &mut [&mut view.headline]);
        if let Some(blob) = view.blob.as_mut() {
            self.rename(&id, &mut blob.connection_label, &mut []);
        }
    }

    pub fn entry(&self, entry: &mut ActivityEntry) {
        let id = entry.connection_id.clone();
        self.rename(&id, &mut entry.connection_label, &mut []);
    }

    pub fn grant(&self, grant: &mut GrantView) {
        let id = grant.connection_id.clone();
        self.rename(&id, &mut grant.connection_label, &mut []);
    }

    /// A file an AI uploaded (`connection_id`: the connection that uploaded it).
    pub fn blob(&self, connection_id: &str, blob: &mut BlobView) {
        self.rename(connection_id, &mut blob.connection_label, &mut []);
    }

    pub fn decision(&self, view: &mut crate::autopilot::AutoDecisionView) {
        let id = view.connection_id.clone();
        self.rename(&id, &mut view.connection_label, &mut []);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_typed_name_is_one_clean_line() {
        assert_eq!(typed_name("  Claude (work) ").as_deref(), Some("Claude (work)"));
        assert_eq!(typed_name("Home\nMac\t").as_deref(), Some("Home Mac"));
        assert_eq!(typed_name("Cl\u{200b}aude").as_deref(), Some("Claude"), "no zero-width characters");
        assert_eq!(typed_name("\u{202e}krow").as_deref(), Some("krow"), "no bidi controls");
        assert_eq!(typed_name("a\u{0}b\u{7}c").as_deref(), Some("a b c"), "no control characters");
        assert_eq!(typed_name(&"x".repeat(100)).map(|n| n.chars().count()), Some(MAX_NAME_CHARS));
        assert_eq!(typed_name(&format!("{} y", "x".repeat(63))).as_deref(), Some("x".repeat(63).as_str()));
        for empty in ["", "   ", "\n\t", "\u{200b}\u{feff}"] {
            assert_eq!(typed_name(empty), None, "{empty:?}");
        }
        // Typed by the user: kept as typed, never read as a computer's "<name> (<platform>)".
        assert_eq!(typed_name("Work box (Linux)").as_deref(), Some("Work box (Linux)"));
    }

    #[test]
    fn the_original_name_is_cleaned_like_any_untrusted_name() {
        assert_eq!(original_name("M2A (macOS)"), "M2A (macOS)");
        assert_eq!(original_name("Claude\n\u{202e}evil"), "Claude evil");
        assert_eq!(original_name(&"z".repeat(200)).chars().count(), MAX_NAME_CHARS);
    }

    #[test]
    fn the_users_name_wins_everywhere_and_its_own_shows_otherwise() {
        let names = Names::new(
            HashMap::from([
                ("c1".to_owned(), "Claude (work)".to_owned()),
                ("c2".to_owned(), "  \u{200b} ".to_owned()),
                ("c4".to_owned(), "Mine".to_owned()),
            ]),
            HashMap::from([("c3".to_owned(), 2), ("c4".to_owned(), 3), ("c5".to_owned(), 0), ("c6".to_owned(), 5000)]),
        );
        assert_eq!(names.label("c1", "Claude"), "Claude (work)");
        assert_eq!(names.label("c2", "ChatGPT"), "ChatGPT", "a name that cleans to nothing is no name");
        assert_eq!(names.get("c2"), None);
        assert_eq!(names.label("c9", "Grok"), "Grok");
        // A number belongs to its own name, never the user's: shown with it, and a name of the user's wins.
        assert_eq!((names.label("c3", "Claude"), names.get("c3")), ("Claude 2".to_owned(), None));
        assert_eq!(names.own("c4", "Claude"), "Claude 3");
        assert_eq!(names.label("c4", "Claude"), "Mine");
        assert_eq!(names.label("c5", "Claude"), "Claude", "no number 0");
        assert_eq!(names.label("c6", "Claude"), "Claude", "nor one out of range");
    }

    #[test]
    fn a_waiting_item_is_renamed_in_its_title_and_headline() {
        let names = Names::new(HashMap::from([("c1".to_owned(), "Claude (work)".to_owned())]), HashMap::new());
        let mut item = PendingItem {
            ticket: None,
            can_defer: false,
            parked_until: None,
            task_id: None,
            task_title: None,
            batchable: false,
            task_proposal: None,
            kind: crate::types::PendingKind::Request,
            id: "r1".to_owned(),
            title: "Claude wants to read 2 emails".to_owned(),
            subtitle: "Claude".to_owned(),
            created_at: 1,
            connection_id: "c1".to_owned(),
            connection_label: "Claude".to_owned(),
            action: "read".to_owned(),
            count: 2,
            service: "gmail".to_owned(),
            account: None,
            wait_until: None,
            op: String::new(),
            op_title: String::new(),
            suggestion: None,
            headline: "Claude gets 2 emails.".to_owned(),
            quick: false,
            label: None,
        };
        let mut other = item.clone();
        other.connection_id = "c9".to_owned();
        names.item(&mut item);
        assert_eq!(item.connection_label, "Claude (work)");
        assert_eq!(item.title, "Claude (work) wants to read 2 emails");
        assert_eq!(item.headline, "Claude (work) gets 2 emails.");
        assert_eq!(item.subtitle, "Claude", "only what starts with the name");
        let before = other.clone();
        names.item(&mut other);
        assert_eq!(other, before, "another connection keeps its own name");
    }

    #[test]
    fn a_second_account_of_the_same_ai_is_offered_a_number() {
        assert_eq!(suggested_name("Claude", 0, &[]), "Claude");
        assert_eq!(suggested_name("Claude", 1, &["Claude".to_owned()]), "Claude 2");
        assert_eq!(
            suggested_name("Claude", 1, &["claude 2".to_owned()]),
            "Claude 3",
            "a name taken (without case) is skipped"
        );
        assert_eq!(suggested_name("Claude", 2, &["Claude".to_owned(), "Claude (work)".to_owned()]), "Claude 3");
        assert_eq!(suggested_name("M2A (macOS)", 1, &[]), "M2A 2", "a computer by its own name");
        assert_eq!(suggested_number("Claude", 0, &[]), None);
        assert_eq!(suggested_number("Claude", 1, &["Claude 2".to_owned()]), Some(3));
        let long = "n".repeat(80);
        let s = suggested_name(&long, 1, &[]);
        assert!(s.ends_with(" 2") && s.chars().count() == MAX_NAME_CHARS, "{s}");
    }

    #[test]
    fn the_same_client_is_the_same_name_from_the_same_host() {
        assert!(same_client("Claude", "claude.ai", "claude", "Claude.ai"));
        assert!(!same_client("Claude", "claude.ai", "Claude", "evil.example"), "a name alone is not the client");
        assert!(!same_client("Claude", "claude.ai", "ChatGPT", "claude.ai"));
        assert!(!same_client("Claude", "", "Claude", ""), "no host, nothing to compare");
        assert!(same_client("M2A (macOS)", "localhost", "M2A", "localhost"), "a computer by its own name");
    }
}
