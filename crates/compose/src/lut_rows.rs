//! Adjustment LUT rows (4096 entries each) kept between renders, keyed by a hash of what they're
//! built from. The GPU planner rebuilt the Levels tables on every plan, 1–2 ms per layer (#11498);
//! callers hash exactly the builder's inputs, so settings that don't change the rows share an
//! entry. Most recently used first, at most [`CAPACITY`] entries
//! (32 × 3 rows × 16 KiB = 1.5 MiB); [`purge_cache`] empties it (Edit › Purge › All).

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, OnceLock};

/// Entries in one LUT row.
pub const ROW_LEN: usize = 4096;

/// The LUT rows of one adjustment state.
pub type Rows = Vec<[f32; ROW_LEN]>;

/// Adjustment states the shared cache remembers.
pub const CAPACITY: usize = 32;

/// A most-recently-used list of row sets by key.
#[derive(Debug)]
pub struct RowCache {
    entries: VecDeque<(u64, Arc<Rows>)>,
    capacity: usize,
    generation: u64,
}

impl RowCache {
    /// An empty cache holding at most `capacity` row sets.
    pub fn new(capacity: usize) -> Self {
        Self { entries: VecDeque::new(), capacity, generation: 0 }
    }

    /// The rows stored for `key`, now the most recently used.
    fn get(&mut self, key: u64) -> Option<Arc<Rows>> {
        let entry = self.entries.iter().position(|(k, _)| *k == key).and_then(|i| self.entries.remove(i))?;
        let rows = Arc::clone(&entry.1);
        self.entries.push_front(entry);
        Some(rows)
    }

    /// Store `rows` for `key` (unless another builder got there first), dropping the oldest entry
    /// when full.
    fn insert(&mut self, key: u64, rows: Arc<Rows>) {
        if !self.entries.iter().any(|(k, _)| *k == key) {
            self.entries.push_front((key, rows));
            self.entries.truncate(self.capacity);
        }
    }

    /// Row sets held.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether nothing is held.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Bytes of rows held.
    pub fn bytes(&self) -> usize {
        self.entries.iter().map(|(_, r)| r.len().saturating_mul(std::mem::size_of::<[f32; ROW_LEN]>())).sum()
    }

    /// Drop every entry, returning the bytes they held.
    pub fn clear(&mut self) -> usize {
        let freed = self.bytes();
        self.entries.clear();
        self.generation = self.generation.wrapping_add(1);
        freed
    }
}

/// The rows for `key` in `cache`, built by `build` only when it doesn't hold them. `build` runs
/// outside the lock, so other renders never wait on it.
pub fn cached_in(cache: &Mutex<RowCache>, key: u64, build: impl FnOnce() -> Rows) -> Rows {
    let (hit, generation) = {
        let mut cache = cache.lock().unwrap_or_else(|e| e.into_inner());
        (cache.get(key), cache.generation)
    };
    if let Some(rows) = hit {
        return rows.to_vec();
    }
    let rows = build();
    let mut cache = cache.lock().unwrap_or_else(|e| e.into_inner());
    // A purge must not be undone by a build that was already in flight.
    if cache.generation == generation {
        cache.insert(key, Arc::new(rows.clone()));
    }
    rows
}

fn cache() -> &'static Mutex<RowCache> {
    static C: OnceLock<Mutex<RowCache>> = OnceLock::new();
    C.get_or_init(|| Mutex::new(RowCache::new(CAPACITY)))
}

/// [`cached_in`] the shared cache.
pub fn cached(key: u64, build: impl FnOnce() -> Rows) -> Rows {
    cached_in(cache(), key, build)
}

/// Bytes of rows the shared cache holds.
pub fn cache_bytes() -> usize {
    cache().lock().unwrap_or_else(|e| e.into_inner()).bytes()
}

/// Drop every cached row set (Edit › Purge › All), returning the bytes they held.
pub fn purge_cache() -> usize {
    cache().lock().unwrap_or_else(|e| e.into_inner()).clear()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    fn rows(v: f32) -> Rows {
        vec![[v; ROW_LEN]; 3]
    }

    #[test]
    fn a_known_key_is_served_without_building() {
        let cache = Mutex::new(RowCache::new(4));
        let builds = Cell::new(0);
        let get = |key: u64, v: f32| {
            cached_in(&cache, key, || {
                builds.set(builds.get() + 1);
                rows(v)
            })
        };
        assert_eq!(get(1, 0.25)[2][7], 0.25);
        assert_eq!(get(1, 0.75)[2][7], 0.25, "the stored rows, not a rebuild");
        assert_eq!(builds.get(), 1);
        get(2, 0.5);
        assert_eq!(builds.get(), 2);
    }

    #[test]
    fn the_least_recently_used_entry_is_dropped_first() {
        let cache = Mutex::new(RowCache::new(CAPACITY));
        let builds = Cell::new(0);
        let get = |key: u64| {
            cached_in(&cache, key, || {
                builds.set(builds.get() + 1);
                rows(0.0)
            })
        };
        // A slider drag through many states keeps only the most recent ones.
        for key in 0..200 {
            get(key);
        }
        assert_eq!(cache.lock().map(|c| c.len()).unwrap_or(0), CAPACITY);
        assert_eq!(builds.get(), 200);
        // Key 168 is the oldest kept: reading it makes 169 the oldest, which the next new key drops.
        get(168);
        get(1000);
        assert_eq!(builds.get(), 201);
        get(168);
        assert_eq!(builds.get(), 201, "kept: it was used recently");
        get(169);
        assert_eq!(builds.get(), 202, "dropped: the oldest when the cache was full");
    }

    #[test]
    fn purge_discards_in_flight_builds() {
        use std::sync::mpsc;
        let cache = Mutex::new(RowCache::new(CAPACITY));
        let (started_tx, started_rx) = mpsc::channel();
        let (resume_tx, resume_rx) = mpsc::channel();
        std::thread::scope(|scope| {
            let cache_ref = &cache;
            scope.spawn(move || {
                cached_in(cache_ref, 123, || {
                    started_tx.send(()).unwrap();
                    resume_rx.recv().unwrap();
                    rows(0.5)
                })
            });
            started_rx.recv().unwrap();
            cache.lock().unwrap().clear();
            resume_tx.send(()).unwrap();
        });
        assert!(cache.lock().unwrap().is_empty(), "a build started before purge must not refill the cache");
    }

    #[test]
    fn clearing_reports_the_bytes_held() {
        let mut cache = RowCache::new(CAPACITY);
        cache.insert(1, Arc::new(rows(0.0)));
        cache.insert(2, Arc::new(vec![[0.0; ROW_LEN]]));
        assert_eq!(cache.bytes(), 4 * ROW_LEN * 4);
        assert_eq!(cache.clear(), 4 * ROW_LEN * 4);
        assert!(cache.is_empty());
        assert_eq!(cache.bytes(), 0);
    }
}
