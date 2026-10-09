//! When each account's approval device was last heard from, in memory: every call it makes as the approval device
//! counts, and an open long-poll means it is listening right now. Used to tell an AI quickly and plainly that the
//! phone cannot be reached ("last seen 2 h ago") instead of letting it wait for nothing.

use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard, PoisonError};

/// A phone heard from this recently (its app polls between long-polls) counts as reachable without a push.
pub const RECENT_SECS: i64 = 10;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Seen {
    at: i64,
    listening: usize,
}

/// Where an account's approval device stands right now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PhoneSeen {
    /// A long-poll is open: the app is running and gets requests at once.
    Listening,
    /// Last heard from this many seconds ago.
    Ago(i64),
    /// Not since this server started.
    Never,
}

impl PhoneSeen {
    /// Heard from just now or listening: a request reaches it without a push.
    pub fn recent(self) -> bool {
        matches!(self, Self::Listening) || matches!(self, Self::Ago(s) if s <= RECENT_SECS)
    }

    /// "a minute ago", "2 hours ago", for an AI's message; `None` when it is listening.
    pub fn phrase(self) -> Option<String> {
        let ago = match self {
            Self::Listening => return None,
            Self::Never => return Some("not since the Reins server last restarted".to_owned()),
            Self::Ago(s) => s.max(0),
        };
        let (n, unit) = match ago {
            0..60 => return Some("less than a minute ago".to_owned()),
            60..3_600 => (ago / 60, "minute"),
            3_600..86_400 => (ago / 3_600, "hour"),
            _ => (ago / 86_400, "day"),
        };
        Some(format!(
            "{n} {unit}{} ago",
            if n == 1 {
                ""
            } else {
                "s"
            }
        ))
    }
}

#[derive(Default)]
pub struct Presence {
    seen: Mutex<HashMap<String, Seen>>,
}

/// Held while a long-poll of `user` is open; the phone counts as seen when it ends too.
pub struct Listening<'a> {
    presence: &'a Presence,
    user: String,
}

impl Drop for Listening<'_> {
    fn drop(&mut self) {
        let mut seen = self.presence.lock();
        if let Some(s) = seen.get_mut(&self.user) {
            s.listening = s.listening.saturating_sub(1);
            s.at = crate::api::reins::now_unix();
        }
    }
}

impl Presence {
    fn lock(&self) -> MutexGuard<'_, HashMap<String, Seen>> {
        self.seen.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The approval device of `user` made a call.
    pub fn seen(&self, user: &str, now: i64) {
        self.lock().entry(user.to_owned()).or_default().at = now;
    }

    /// A long-poll of `user` opens; it counts until the guard is dropped.
    pub fn listen(&self, user: &str, now: i64) -> Listening<'_> {
        let mut seen = self.lock();
        let s = seen.entry(user.to_owned()).or_default();
        s.at = now;
        s.listening += 1;
        Listening {
            presence: self,
            user: user.to_owned(),
        }
    }

    /// When the approval device of `user` was last heard from (Unix seconds); `None` since this server started.
    pub fn last_seen(&self, user: &str) -> Option<i64> {
        self.lock().get(user).map(|s| s.at)
    }

    pub fn status(&self, user: &str, now: i64) -> PhoneSeen {
        match self.lock().get(user) {
            None => PhoneSeen::Never,
            Some(s) if s.listening > 0 => PhoneSeen::Listening,
            Some(s) => PhoneSeen::Ago(now - s.at),
        }
    }

    /// A deleted account.
    pub fn forget(&self, user: &str) {
        self.lock().remove(user);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn listening_seen_and_never() {
        let p = Presence::default();
        assert_eq!(p.status("u", 100), PhoneSeen::Never);
        p.seen("u", 100);
        assert_eq!(p.status("u", 110), PhoneSeen::Ago(10));
        assert!(p.status("u", 110).recent());
        assert!(!p.status("u", 111).recent());
        {
            let _poll = p.listen("u", 200);
            assert_eq!(p.status("u", 900), PhoneSeen::Listening);
            assert!(p.status("u", 900).recent());
        }
        assert!(matches!(p.status("u", 205), PhoneSeen::Ago(_)), "the poll ended");
        p.forget("u");
        assert_eq!(p.status("u", 205), PhoneSeen::Never);
    }

    #[test]
    fn ages_read_plainly() {
        assert_eq!(PhoneSeen::Ago(5).phrase().unwrap(), "less than a minute ago");
        assert_eq!(PhoneSeen::Ago(60).phrase().unwrap(), "1 minute ago");
        assert_eq!(PhoneSeen::Ago(2 * 3_600 + 5).phrase().unwrap(), "2 hours ago");
        assert_eq!(PhoneSeen::Ago(3 * 86_400).phrase().unwrap(), "3 days ago");
        assert_eq!(PhoneSeen::Listening.phrase(), None);
        assert!(PhoneSeen::Never.phrase().unwrap().contains("restarted"));
    }
}
