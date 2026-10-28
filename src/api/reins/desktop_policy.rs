//! The phone-signed desktop policy (Decisions v2, 08 R1) on its way to each paired computer. The phone posts a
//! snapshot, and a "tighten now" when one is urgent, each sealed to one computer's pinned key
//! (`POST /reins/api/desktop/policy`); the computer's daemon picks them up (`GET /reins/desktop/policy`), waiting a
//! while for news. The server cannot open them: it keeps them by connection, version and kind only, in memory.

use std::{
    collections::HashMap,
    sync::{Mutex, MutexGuard, PoisonError},
    time::Duration,
};

use reins_proto::desktop::{MAX_POLICY_CHARS, MAX_TIGHTENS, MailedPolicy, POLICY_TTL_SECS, PolicyKind};
use tokio::sync::watch;

use super::now_unix;

/// The most characters of sealed policy this server keeps in all, across computers. A post beyond it is dropped (the
/// computer then asks the phone, as it does without a snapshot).
pub const MAX_TOTAL_POLICY_CHARS: usize = 128 * 1024 * 1024;

/// Per desktop connection: the newest snapshot and the tightenings newer than it, until the computer picks them up.
/// Memory only, each kept [`POLICY_TTL_SECS`] after it was posted; a server that restarts forgot them, and the phone
/// posts again within the hour.
///
/// Every change happens under one lock, so a post and a pick-up never see half of another.
#[derive(Default)]
pub struct Policies {
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    by_connection: HashMap<String, Mailbox>,
    /// The characters of every message kept, for [`MAX_TOTAL_POLICY_CHARS`].
    chars: usize,
}

struct Mailbox {
    /// The newest snapshot.
    snapshot: Option<Kept>,
    /// Tightenings newer than the snapshot, by version, oldest first.
    tightens: Vec<Kept>,
    /// Bumped on every accepted post: wakes the computer's pick-up.
    changes: watch::Sender<u64>,
}

struct Kept {
    version: u64,
    sealed: String,
    until: i64,
}

impl Mailbox {
    fn new() -> Self {
        Self {
            snapshot: None,
            tightens: Vec::new(),
            changes: watch::Sender::new(0),
        }
    }

    fn chars(&self) -> usize {
        self.snapshot.iter().chain(&self.tightens).map(|k| k.sealed.len()).sum()
    }

    /// Drops what expired; returns how many characters went.
    fn expire(&mut self, now: i64) -> usize {
        let before = self.chars();
        if self.snapshot.as_ref().is_some_and(|k| k.until <= now) {
            self.snapshot = None;
        }
        self.tightens.retain(|k| k.until > now);
        before - self.chars()
    }

    fn is_idle(&self) -> bool {
        self.snapshot.is_none() && self.tightens.is_empty() && self.changes.receiver_count() == 0
    }
}

impl Policies {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Keeps `item` for `connection` posted at `now`; whether it was kept. A snapshot replaces the kept one only when
    /// newer, and drops the tightenings it covers; a tightening is kept only when newer than the snapshot, at most
    /// [`MAX_TIGHTENS`] (the oldest go). An empty or oversized blob, a version already kept, or a post beyond
    /// [`MAX_TOTAL_POLICY_CHARS`] is ignored.
    pub fn post(&self, connection: &str, item: MailedPolicy, now: i64) -> bool {
        if item.sealed.is_empty() || item.sealed.len() > MAX_POLICY_CHARS {
            return false;
        }
        let mut state = self.lock();
        let State {
            by_connection,
            chars,
        } = &mut *state;
        let mailbox = by_connection.entry(connection.to_owned()).or_insert_with(Mailbox::new);
        *chars -= mailbox.expire(now);
        let floor = mailbox.snapshot.as_ref().map(|k| k.version);
        if floor.is_some_and(|v| item.version <= v)
            || (item.kind == PolicyKind::Tighten && mailbox.tightens.iter().any(|k| k.version == item.version))
        {
            return false;
        }
        if *chars + item.sealed.len() > MAX_TOTAL_POLICY_CHARS {
            warn!("Dropped a desktop policy: this server holds {chars} characters of them already");
            return false;
        }
        let before = mailbox.chars();
        let kept = Kept {
            version: item.version,
            sealed: item.sealed,
            until: now + POLICY_TTL_SECS,
        };
        match item.kind {
            PolicyKind::Snapshot => {
                mailbox.tightens.retain(|k| k.version > kept.version);
                mailbox.snapshot = Some(kept);
            }
            PolicyKind::Tighten => {
                let at = mailbox.tightens.partition_point(|k| k.version < kept.version);
                mailbox.tightens.insert(at, kept);
                let excess = mailbox.tightens.len().saturating_sub(MAX_TIGHTENS);
                mailbox.tightens.drain(..excess);
            }
        }
        *chars = *chars - before + mailbox.chars();
        mailbox.changes.send_modify(|v| *v = v.wrapping_add(1));
        true
    }

    /// What waits for `connection` newer than `after`: the snapshot, then the tightenings, by version.
    pub fn peek(&self, connection: &str, after: u64, now: i64) -> Vec<MailedPolicy> {
        let mut state = self.lock();
        let State {
            by_connection,
            chars,
        } = &mut *state;
        let Some(mailbox) = by_connection.get_mut(connection) else {
            return Vec::new();
        };
        *chars -= mailbox.expire(now);
        let snapshot = mailbox.snapshot.iter().map(|k| (PolicyKind::Snapshot, k));
        let tightens = mailbox.tightens.iter().map(|k| (PolicyKind::Tighten, k));
        snapshot
            .chain(tightens)
            .filter(|(_, k)| k.version > after)
            .map(|(kind, k)| MailedPolicy {
                version: k.version,
                kind,
                sealed: k.sealed.clone(),
            })
            .collect()
    }

    /// What changes when a post for `connection` is kept.
    pub fn watch(&self, connection: &str) -> watch::Receiver<u64> {
        self.lock().by_connection.entry(connection.to_owned()).or_insert_with(Mailbox::new).changes.subscribe()
    }

    /// [`Self::peek`], waiting up to `wait` for a post when nothing newer than `after` waits yet.
    pub async fn wait(&self, connection: &str, after: u64, wait: Duration) -> Vec<MailedPolicy> {
        let deadline = tokio::time::Instant::now() + wait;
        let mut rx = self.watch(connection);
        loop {
            rx.borrow_and_update();
            let items = self.peek(connection, after, now_unix());
            if !items.is_empty() {
                return items;
            }
            match tokio::time::timeout_at(deadline, rx.changed()).await {
                Ok(Ok(())) => {}
                Ok(Err(_)) | Err(_) => return self.peek(connection, after, now_unix()),
            }
        }
    }

    /// Forgets what expired, and computers with nothing kept that nobody waits on.
    pub fn purge(&self, now: i64) {
        let mut state = self.lock();
        let State {
            by_connection,
            chars,
        } = &mut *state;
        by_connection.retain(|_, mailbox| {
            *chars -= mailbox.expire(now);
            !mailbox.is_idle()
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(version: u64, kind: PolicyKind, sealed: &str) -> MailedPolicy {
        MailedPolicy {
            version,
            kind,
            sealed: sealed.to_owned(),
        }
    }

    fn snapshot(version: u64) -> MailedPolicy {
        item(version, PolicyKind::Snapshot, &format!("s{version}"))
    }

    fn tighten(version: u64) -> MailedPolicy {
        item(version, PolicyKind::Tighten, &format!("t{version}"))
    }

    fn sealed(items: &[MailedPolicy]) -> Vec<String> {
        items.iter().map(|i| i.sealed.clone()).collect()
    }

    #[test]
    fn policy_mailbox_keeps_the_newest_and_newer_tightenings() {
        let p = Policies::default();
        assert!(p.post("c1", tighten(5), 100), "a tightening applies without a snapshot");
        assert!(p.post("c1", snapshot(10), 100));
        assert!(p.post("c1", tighten(12), 100));
        assert!(p.post("c1", tighten(11), 100));
        assert!(p.post("c2", snapshot(3), 100));
        let all = p.peek("c1", 0, 101);
        assert_eq!(sealed(&all), ["s10", "t11", "t12"], "the snapshot covers t5; then by version");
        assert_eq!(all[0].kind, PolicyKind::Snapshot);
        assert_eq!(all[1].kind, PolicyKind::Tighten);
        assert_eq!(sealed(&p.peek("c1", 10, 101)), ["t11", "t12"], "only what is newer than `after`");
        assert_eq!(sealed(&p.peek("c1", 11, 101)), ["t12"]);
        assert!(p.peek("c1", 12, 101).is_empty());
        assert_eq!(sealed(&p.peek("c2", 0, 101)), ["s3"], "each computer its own");
        assert!(p.peek("c3", 0, 101).is_empty());
        // A newer snapshot drops the tightenings it covers.
        assert!(p.post("c1", snapshot(11), 102));
        assert_eq!(sealed(&p.peek("c1", 0, 103)), ["s11", "t12"]);
        // At most MAX_TIGHTENS tightenings: the oldest go.
        for v in 100..100 + MAX_TIGHTENS as u64 + 3 {
            assert!(p.post("c1", tighten(v), 104));
        }
        let mail = p.peek("c1", 0, 105);
        assert_eq!(mail.len(), 1 + MAX_TIGHTENS);
        assert_eq!(mail[0].sealed, "s11", "the snapshot is never pushed out");
        assert_eq!(mail[1].sealed, "t103");
        assert_eq!(mail[MAX_TIGHTENS].sealed, format!("t{}", 100 + MAX_TIGHTENS + 2));
    }

    #[test]
    fn older_versions_never_replace() {
        let p = Policies::default();
        assert!(p.post("c1", snapshot(10), 100));
        assert!(!p.post("c1", snapshot(9), 100), "older");
        assert!(!p.post("c1", item(10, PolicyKind::Snapshot, "other"), 100), "the same version");
        assert!(!p.post("c1", tighten(10), 100), "a tightening the snapshot covers");
        assert!(!p.post("c1", tighten(3), 100));
        assert!(p.post("c1", tighten(20), 100));
        assert!(!p.post("c1", item(20, PolicyKind::Tighten, "again"), 100), "a tightening already kept");
        assert_eq!(sealed(&p.peek("c1", 0, 101)), ["s10", "t20"]);
    }

    #[tokio::test]
    async fn a_waiter_wakes_on_a_post() {
        let p = std::sync::Arc::new(Policies::default());
        let now = now_unix();
        assert!(p.post("c1", snapshot(10), now));
        // Something newer than `after` already waits: no wait at all.
        let started = tokio::time::Instant::now();
        assert_eq!(sealed(&p.wait("c1", 0, Duration::from_secs(20)).await), ["s10"]);
        assert!(started.elapsed() < Duration::from_secs(5));
        // Nothing newer: the wait ends with the next accepted post, not before.
        let waiter = tokio::spawn({
            let p = std::sync::Arc::clone(&p);
            async move { p.wait("c1", 10, Duration::from_secs(20)).await }
        });
        while p.lock().by_connection.get("c1").is_none_or(|m| m.changes.receiver_count() == 0) {
            tokio::task::yield_now().await;
        }
        assert!(!p.post("c1", snapshot(9), now), "an ignored post wakes nobody");
        assert!(p.post("c2", snapshot(11), now), "nor does another computer's");
        assert!(p.post("c1", tighten(11), now));
        let woke = tokio::time::timeout(Duration::from_secs(5), waiter).await.expect("woke").expect("joined");
        assert_eq!(sealed(&woke), ["t11"]);
        // Nothing comes: an empty answer at the deadline.
        assert!(p.wait("c1", 11, Duration::from_millis(50)).await.is_empty());
        // The waiter gone, purging forgets nothing kept.
        p.purge(now);
        assert_eq!(p.peek("c1", 0, now).len(), 2);
    }

    #[test]
    fn oversized_or_empty_blobs_are_ignored() {
        let p = Policies::default();
        assert!(!p.post("c1", item(1, PolicyKind::Snapshot, ""), 100));
        assert!(!p.post("c1", item(2, PolicyKind::Snapshot, &"x".repeat(MAX_POLICY_CHARS + 1)), 100));
        assert!(!p.post("c1", item(3, PolicyKind::Tighten, ""), 100));
        assert!(p.peek("c1", 0, 101).is_empty());
        assert!(p.post("c1", item(4, PolicyKind::Snapshot, &"x".repeat(MAX_POLICY_CHARS)), 100), "at the limit");
        assert_eq!(p.lock().chars, MAX_POLICY_CHARS);
    }

    #[test]
    fn items_expire() {
        let p = Policies::default();
        assert!(p.post("c1", snapshot(10), 100));
        assert!(p.post("c1", tighten(11), 200));
        let ttl = POLICY_TTL_SECS;
        assert_eq!(sealed(&p.peek("c1", 0, 99 + ttl)), ["s10", "t11"]);
        assert_eq!(sealed(&p.peek("c1", 0, 100 + ttl)), ["t11"], "a day after it was posted");
        assert!(p.peek("c1", 0, 200 + ttl).is_empty());
        assert_eq!(p.lock().chars, 0);
        // An expired snapshot no longer holds older versions back (the computer refuses a rollback itself).
        assert!(p.post("c1", snapshot(5), 300 + ttl));
        p.purge(300 + 2 * ttl);
        assert!(p.lock().by_connection.is_empty(), "nothing kept, nobody waiting");
        assert_eq!(p.lock().chars, 0);
    }
}
