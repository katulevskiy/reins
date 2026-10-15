//! Expiring map on tokio's clock, so timing is testable with a paused clock (plan Decision 4).

use std::{collections::HashMap, hash::Hash, time::Duration};

use tokio::time::Instant;

/// Returned by [`TtlMap::insert`] when the map already holds `capacity` live entries.
#[derive(Debug, PartialEq, Eq)]
pub struct Full;

/// Entries expire `ttl` after insertion. Expired entries are invisible at once and
/// physically dropped on the next `insert` or `purge`.
pub struct TtlMap<K, V> {
    ttl: Duration,
    capacity: usize,
    entries: HashMap<K, (Instant, V)>,
}

impl<K: Eq + Hash, V> TtlMap<K, V> {
    pub fn new(ttl: Duration, capacity: usize) -> Self {
        Self {
            ttl,
            capacity,
            entries: HashMap::new(),
        }
    }

    /// Inserts or replaces `key`; the entry lives until `now + ttl`.
    pub fn insert(&mut self, key: K, value: V) -> Result<(), Full> {
        self.purge();
        if self.entries.len() >= self.capacity && !self.entries.contains_key(&key) {
            return Err(Full);
        }
        self.entries.insert(key, (Instant::now() + self.ttl, value));
        Ok(())
    }

    /// Inserts or replaces `key`; when the map is full, the entry closest to expiring (the oldest) makes room. Only
    /// for maps whose entries may be dropped early without harm: caches and rate-limit marks.
    pub fn insert_evicting(&mut self, key: K, value: V)
    where
        K: Clone,
    {
        self.purge();
        if self.entries.len() >= self.capacity && !self.entries.contains_key(&key) {
            let oldest = self.entries.iter().min_by_key(|(_, (expires, _))| *expires).map(|(k, _)| k.clone());
            if let Some(oldest) = oldest {
                self.entries.remove(&oldest);
            }
        }
        if self.capacity > 0 {
            self.entries.insert(key, (Instant::now() + self.ttl, value));
        }
    }

    pub fn get(&self, key: &K) -> Option<&V> {
        let now = Instant::now();
        self.entries.get(key).filter(|(expires, _)| now < *expires).map(|(_, v)| v)
    }

    pub fn get_mut(&mut self, key: &K) -> Option<&mut V> {
        let now = Instant::now();
        self.entries.get_mut(key).filter(|(expires, _)| now < *expires).map(|(_, v)| v)
    }

    pub fn remove(&mut self, key: &K) -> Option<V> {
        let (expires, value) = self.entries.remove(key)?;
        (Instant::now() < expires).then_some(value)
    }

    /// Live values, in unspecified order.
    pub fn values(&self) -> impl Iterator<Item = &V> {
        let now = Instant::now();
        self.entries.values().filter(move |(expires, _)| now < *expires).map(|(_, v)| v)
    }

    /// Live values, mutable, in unspecified order.
    pub fn values_mut(&mut self) -> impl Iterator<Item = &mut V> {
        let now = Instant::now();
        self.entries.values_mut().filter(move |(expires, _)| now < *expires).map(|(_, v)| v)
    }

    pub fn purge(&mut self) {
        let now = Instant::now();
        self.entries.retain(|_, (expires, _)| now < *expires);
    }

    /// Drops every entry `keep` says no to (and the expired ones).
    pub fn retain(&mut self, mut keep: impl FnMut(&V) -> bool) {
        let now = Instant::now();
        self.entries.retain(|_, (expires, value)| now < *expires && keep(value));
    }

    /// Number of live entries.
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.values().count()
    }

    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[tokio::test(start_paused = true)]
    async fn entries_expire_exactly_at_ttl() {
        let mut m = TtlMap::new(Duration::from_secs(60), 10);
        m.insert("a", 1).unwrap();
        tokio::time::advance(Duration::from_secs(59)).await;
        assert_eq!(m.get(&"a"), Some(&1));
        assert_eq!(m.len(), 1);
        tokio::time::advance(Duration::from_secs(1)).await;
        assert_eq!(m.get(&"a"), None);
        assert!(m.is_empty());
        assert_eq!(m.remove(&"a"), None);
    }

    #[tokio::test(start_paused = true)]
    async fn capacity_counts_only_live_entries() {
        let mut m = TtlMap::new(Duration::from_secs(10), 2);
        m.insert("a", 1).unwrap();
        m.insert("b", 2).unwrap();
        assert_eq!(m.insert("c", 3), Err(Full));
        m.insert("a", 10).unwrap();
        assert_eq!(m.get(&"a"), Some(&10));
        tokio::time::advance(Duration::from_secs(10)).await;
        m.insert("c", 3).unwrap();
        assert_eq!(m.values().copied().collect::<Vec<_>>(), vec![3]);
    }

    #[tokio::test(start_paused = true)]
    async fn an_evicting_insert_drops_the_oldest_entry_when_full() {
        let mut m = TtlMap::new(Duration::from_secs(10), 2);
        m.insert_evicting("a", 1);
        tokio::time::advance(Duration::from_secs(1)).await;
        m.insert_evicting("b", 2);
        tokio::time::advance(Duration::from_secs(1)).await;
        m.insert_evicting("c", 3);
        assert_eq!((m.get(&"a"), m.get(&"b"), m.get(&"c")), (None, Some(&2), Some(&3)));
        m.insert_evicting("b", 20);
        assert_eq!((m.get(&"b"), m.get(&"c"), m.len()), (Some(&20), Some(&3), 2), "replacing evicts nothing");
        let mut none = TtlMap::new(Duration::from_secs(10), 0);
        none.insert_evicting("a", 1);
        assert!(none.is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn remove_returns_a_live_value_once() {
        let mut m = TtlMap::new(Duration::from_secs(10), 2);
        m.insert("a", 1).unwrap();
        *m.get_mut(&"a").unwrap() += 1;
        assert_eq!(m.remove(&"a"), Some(2));
        assert_eq!(m.remove(&"a"), None);
        m.insert("b", 1).unwrap();
        tokio::time::advance(Duration::from_secs(10)).await;
        assert!(m.get_mut(&"b").is_none());
    }
}
