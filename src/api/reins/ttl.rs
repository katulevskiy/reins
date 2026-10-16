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
    /// Expiry, insertion number (orders entries inserted at the same instant), value.
    entries: HashMap<K, (Instant, u64, V)>,
    inserted: u64,
}

impl<K: Eq + Hash, V> TtlMap<K, V> {
    pub fn new(ttl: Duration, capacity: usize) -> Self {
        Self {
            ttl,
            capacity,
            entries: HashMap::new(),
            inserted: 0,
        }
    }

    /// Inserts or replaces `key`; the entry lives until `now + ttl`.
    pub fn insert(&mut self, key: K, value: V) -> Result<(), Full> {
        self.purge();
        if self.entries.len() >= self.capacity && !self.entries.contains_key(&key) {
            return Err(Full);
        }
        self.put(key, value);
        Ok(())
    }

    fn put(&mut self, key: K, value: V) {
        self.inserted += 1;
        self.entries.insert(key, (Instant::now() + self.ttl, self.inserted, value));
    }

    /// Inserts or replaces `key`; when the map is full, the entry closest to expiring (the oldest) makes room. Only
    /// for maps whose entries may be dropped early without harm: caches and rate-limit marks.
    pub fn insert_evicting(&mut self, key: K, value: V)
    where
        K: Clone,
    {
        self.purge();
        if self.entries.len() >= self.capacity && !self.entries.contains_key(&key) {
            let oldest = self.entries.iter().min_by_key(|(_, (expires, n, _))| (*expires, *n)).map(|(k, _)| k.clone());
            if let Some(oldest) = oldest {
                self.entries.remove(&oldest);
            }
        }
        if self.capacity > 0 {
            self.put(key, value);
        }
    }

    pub fn get(&self, key: &K) -> Option<&V> {
        let now = Instant::now();
        self.entries.get(key).filter(|(expires, _, _)| now < *expires).map(|(_, _, v)| v)
    }

    pub fn get_mut(&mut self, key: &K) -> Option<&mut V> {
        let now = Instant::now();
        self.entries.get_mut(key).filter(|(expires, _, _)| now < *expires).map(|(_, _, v)| v)
    }

    pub fn remove(&mut self, key: &K) -> Option<V> {
        let (expires, _, value) = self.entries.remove(key)?;
        (Instant::now() < expires).then_some(value)
    }

    /// Live values, in unspecified order.
    pub fn values(&self) -> impl Iterator<Item = &V> {
        let now = Instant::now();
        self.entries.values().filter(move |(expires, _, _)| now < *expires).map(|(_, _, v)| v)
    }

    /// Live values, mutable, in unspecified order.
    pub fn values_mut(&mut self) -> impl Iterator<Item = &mut V> {
        let now = Instant::now();
        self.entries.values_mut().filter(move |(expires, _, _)| now < *expires).map(|(_, _, v)| v)
    }

    pub fn purge(&mut self) {
        let now = Instant::now();
        self.entries.retain(|_, (expires, _, _)| now < *expires);
    }

    /// Drops every entry `keep` says no to (and the expired ones).
    pub fn retain(&mut self, mut keep: impl FnMut(&V) -> bool) {
        let now = Instant::now();
        self.entries.retain(|_, (expires, _, value)| now < *expires && keep(value));
    }

    /// Whether a new key fits. Expired entries do not count.
    pub fn has_room(&self) -> bool {
        self.live_count() < self.capacity
    }

    fn live_count(&self) -> usize {
        let now = Instant::now();
        self.entries.iter().filter(|(_, (expires, _, _))| now < *expires).count()
    }

    /// The live key closest to expiring for which `pred` holds: the oldest such entry.
    pub fn oldest_where(&self, mut pred: impl FnMut(&V) -> bool) -> Option<K>
    where
        K: Clone,
    {
        let now = Instant::now();
        self.entries
            .iter()
            .filter(|(_, (expires, _, value))| now < *expires && pred(value))
            .min_by_key(|(_, (expires, n, _))| (*expires, *n))
            .map(|(key, _)| key.clone())
    }

    /// Drops the oldest live entry for which `pred` holds. `false` when none does.
    pub fn evict_where(&mut self, pred: impl FnMut(&V) -> bool) -> bool
    where
        K: Clone,
    {
        let Some(key) = self.oldest_where(pred) else {
            return false;
        };
        // `remove` would report an entry that expires this instant as missing, after deleting it. Drop by key.
        self.entries.remove(&key).is_some()
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
        // Inserted at the same instant: still the first one goes first.
        let mut same = TtlMap::new(Duration::from_secs(10), 2);
        for i in 0..5 {
            same.insert_evicting(i, i);
        }
        assert_eq!((same.get(&2), same.get(&3), same.get(&4)), (None, Some(&3), Some(&4)));
        let mut none = TtlMap::new(Duration::from_secs(10), 0);
        none.insert_evicting("a", 1);
        assert!(none.is_empty());
        assert!(!none.has_room());
    }

    #[tokio::test(start_paused = true)]
    async fn oldest_where_is_the_oldest_live_match() {
        let mut m = TtlMap::new(Duration::from_secs(10), 4);
        m.insert("a", 1).unwrap();
        tokio::time::advance(Duration::from_secs(1)).await;
        m.insert("b", 2).unwrap();
        m.insert("c", 1).unwrap();
        assert!(m.has_room());
        assert_eq!(m.oldest_where(|v| *v == 1), Some("a"));
        assert_eq!(m.oldest_where(|v| *v == 9), None);
        assert!(m.evict_where(|v| *v == 1));
        assert_eq!((m.get(&"a"), m.get(&"b"), m.get(&"c")), (None, Some(&2), Some(&1)));
        tokio::time::advance(Duration::from_secs(9)).await;
        assert_eq!(m.oldest_where(|v| *v == 1), Some("c"), "a has expired");
        assert!(!m.evict_where(|v| *v == 9));
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
