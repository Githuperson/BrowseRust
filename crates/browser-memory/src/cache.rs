//! Memory-efficient storage primitives used by browser-owned data (favicons,
//! history metadata, thumbnails, extension assets…). These are *browser* caches;
//! Servo's own image/font/HTTP caches are controlled through its preferences and
//! `NetworkManager` (see docs/MEMORY_OPTIMISATION.md).

use std::collections::{HashMap, VecDeque};
use std::hash::{BuildHasher, Hash, Hasher};
use std::sync::Arc;

/// Byte-weighted LRU cache. Total weight never exceeds `capacity` after any
/// operation; a single item heavier than the whole capacity is not stored.
pub struct BoundedCache<K, V> {
    capacity: usize,
    used: usize,
    map: HashMap<K, (V, usize)>,
    order: VecDeque<K>, // front = least recently used
}

impl<K: Hash + Eq + Clone, V> BoundedCache<K, V> {
    pub fn new(capacity: usize) -> Self {
        Self { capacity, used: 0, map: HashMap::new(), order: VecDeque::new() }
    }

    pub fn used(&self) -> usize {
        self.used
    }
    pub fn capacity(&self) -> usize {
        self.capacity
    }
    pub fn len(&self) -> usize {
        self.map.len()
    }
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    fn touch(&mut self, k: &K) {
        if let Some(p) = self.order.iter().position(|x| x == k) {
            let key = self.order.remove(p).expect("position valid");
            self.order.push_back(key);
        }
    }

    pub fn get(&mut self, k: &K) -> Option<&V> {
        if self.map.contains_key(k) {
            self.touch(k);
            self.map.get(k).map(|(v, _)| v)
        } else {
            None
        }
    }

    pub fn insert(&mut self, k: K, v: V, weight: usize) {
        self.remove(&k);
        if weight > self.capacity {
            return;
        }
        self.used += weight;
        self.map.insert(k.clone(), (v, weight));
        self.order.push_back(k);
        self.evict_to(self.capacity);
    }

    pub fn remove(&mut self, k: &K) -> Option<V> {
        let (v, w) = self.map.remove(k)?;
        self.used -= w;
        if let Some(p) = self.order.iter().position(|x| x == k) {
            self.order.remove(p);
        }
        Some(v)
    }

    fn evict_to(&mut self, target: usize) {
        while self.used > target {
            let Some(k) = self.order.pop_front() else { break };
            if let Some((_, w)) = self.map.remove(&k) {
                self.used -= w;
            }
        }
    }

    /// Shrink to a new capacity (used when a budget drops under memory pressure).
    pub fn resize(&mut self, capacity: usize) {
        self.capacity = capacity;
        self.evict_to(capacity);
    }

    /// Drop everything above `fraction` (0.0..=1.0) of capacity, LRU first.
    pub fn trim_to_fraction(&mut self, fraction: f64) {
        self.evict_to((self.capacity as f64 * fraction.clamp(0.0, 1.0)) as usize);
    }

    pub fn clear(&mut self) {
        self.map.clear();
        self.order.clear();
        self.used = 0;
    }
}

/// Deduplicates strings (origins, titles, content types) into shared `Arc<str>`s.
#[derive(Default)]
pub struct Interner {
    set: HashMap<Arc<str>, ()>,
}

impl Interner {
    pub fn intern(&mut self, s: &str) -> Arc<str> {
        if let Some((k, _)) = self.set.get_key_value(s) {
            return k.clone();
        }
        let a: Arc<str> = Arc::from(s);
        self.set.insert(a.clone(), ());
        a
    }

    /// Drop entries nobody else references.
    pub fn sweep(&mut self) -> usize {
        let before = self.set.len();
        self.set.retain(|k, _| Arc::strong_count(k) > 1);
        before - self.set.len()
    }

    pub fn len(&self) -> usize {
        self.set.len()
    }
    pub fn is_empty(&self) -> bool {
        self.set.is_empty()
    }
}

/// Content-addressed immutable blob store: identical bytes are stored once and
/// shared via `Arc<[u8]>`. Hash collisions are handled by comparing contents.
#[derive(Default)]
pub struct BlobStore {
    hasher: std::collections::hash_map::RandomState,
    buckets: HashMap<u64, Vec<Arc<[u8]>>>,
}

impl BlobStore {
    pub fn put(&mut self, bytes: &[u8]) -> Arc<[u8]> {
        let mut h = self.hasher.build_hasher();
        bytes.hash(&mut h);
        let key = h.finish();
        let bucket = self.buckets.entry(key).or_default();
        if let Some(b) = bucket.iter().find(|b| &***b == bytes) {
            return b.clone();
        }
        let a: Arc<[u8]> = Arc::from(bytes);
        bucket.push(a.clone());
        a
    }

    /// Free blobs no longer referenced outside the store. Returns bytes freed.
    pub fn sweep(&mut self) -> usize {
        let mut freed = 0;
        for b in self.buckets.values_mut() {
            b.retain(|a| {
                let keep = Arc::strong_count(a) > 1;
                if !keep {
                    freed += a.len();
                }
                keep
            });
        }
        self.buckets.retain(|_, b| !b.is_empty());
        freed
    }

    pub fn stored_bytes(&self) -> usize {
        self.buckets.values().flatten().map(|b| b.len()).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lru_evicts_least_recently_used_and_respects_capacity() {
        let mut c = BoundedCache::new(100);
        c.insert("a", 1, 40);
        c.insert("b", 2, 40);
        assert_eq!(c.get(&"a"), Some(&1)); // a is now most recent
        c.insert("c", 3, 40); // must evict b
        assert!(c.used() <= 100);
        assert_eq!(c.get(&"b"), None);
        assert_eq!(c.get(&"a"), Some(&1));
        assert_eq!(c.get(&"c"), Some(&3));
    }

    #[test]
    fn oversized_item_is_not_stored_and_replace_updates_weight() {
        let mut c = BoundedCache::new(50);
        c.insert("big", 0, 500);
        assert!(c.is_empty());
        c.insert("x", 1, 30);
        c.insert("x", 2, 10);
        assert_eq!(c.used(), 10);
        assert_eq!(c.get(&"x"), Some(&2));
    }

    #[test]
    fn resize_and_trim_shrink_under_pressure() {
        let mut c = BoundedCache::new(1000);
        for i in 0..10 {
            c.insert(i, i, 100);
        }
        c.trim_to_fraction(0.5);
        assert!(c.used() <= 500);
        c.resize(100);
        assert!(c.used() <= 100);
        assert_eq!(c.get(&9), Some(&9), "most recent survives");
    }

    #[test]
    fn interner_shares_and_sweeps() {
        let mut i = Interner::default();
        let a = i.intern("example.com");
        let b = i.intern("example.com");
        assert!(Arc::ptr_eq(&a, &b));
        assert_eq!(i.len(), 1);
        drop((a, b));
        assert_eq!(i.sweep(), 1);
        assert!(i.is_empty());
    }

    #[test]
    fn blob_store_deduplicates_identical_content() {
        let mut s = BlobStore::default();
        let a = s.put(b"favicon-bytes");
        let b = s.put(b"favicon-bytes");
        let c = s.put(b"other");
        assert!(Arc::ptr_eq(&a, &b));
        assert!(!Arc::ptr_eq(&a, &c));
        assert_eq!(s.stored_bytes(), 13 + 5, "stored once");
        drop((a, b));
        assert_eq!(s.sweep(), 13);
        assert_eq!(s.stored_bytes(), 5);
        drop(c);
    }
}
