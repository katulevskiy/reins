//! The connections the daemon made since it started: per kind (`git`, `api`, `ssh`) and target
//! (`github.com/me/app`, `openai → api.openai.com`), how many, when last and how the last one went. Kept in memory
//! only; the control API's `overview` hands it to the desktop app.

use std::collections::HashMap;
use std::sync::{Mutex, PoisonError};

use serde::{Deserialize, Serialize};

/// The most targets kept (the oldest is dropped first).
const MAX_TARGETS: usize = 200;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Connection {
    pub kind: String,
    pub target: String,
    pub count: u64,
    /// Unix seconds.
    pub last_at: i64,
    /// How the last one went (`200`, `push approved`, `denied`).
    pub last: String,
}

#[derive(Default)]
pub struct Stats {
    map: Mutex<HashMap<(String, String), Connection>>,
}

impl Stats {
    pub fn record(&self, kind: &str, target: &str, last: &str) {
        let mut map = self.map.lock().unwrap_or_else(PoisonError::into_inner);
        let now = crate::now_unix();
        let c = map.entry((kind.to_owned(), target.to_owned())).or_insert_with(|| Connection {
            kind: kind.to_owned(),
            target: target.to_owned(),
            count: 0,
            last_at: now,
            last: String::new(),
        });
        c.count += 1;
        c.last_at = now;
        last.clone_into(&mut c.last);
        if map.len() > MAX_TARGETS
            && let Some(oldest) = map.iter().min_by_key(|(_, c)| c.last_at).map(|(k, _)| k.clone())
        {
            map.remove(&oldest);
        }
    }

    /// Most recent first.
    #[must_use]
    pub fn list(&self) -> Vec<Connection> {
        let mut v: Vec<Connection> =
            self.map.lock().unwrap_or_else(PoisonError::into_inner).values().cloned().collect();
        v.sort_by(|a, b| b.last_at.cmp(&a.last_at).then_with(|| a.target.cmp(&b.target)));
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_per_kind_and_target() {
        let s = Stats::default();
        s.record("api", "openai", "200");
        s.record("api", "openai", "401");
        s.record("git", "github.com/me/app", "fetch");
        let list = s.list();
        assert_eq!(list.len(), 2);
        let api = list.iter().find(|c| c.kind == "api").unwrap();
        assert_eq!((api.count, api.last.as_str()), (2, "401"));
    }
}
