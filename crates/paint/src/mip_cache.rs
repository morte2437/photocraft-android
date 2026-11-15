//! Prepared sampled tips shared across strokes and symmetry passes. Identity follows the
//! immutable sample allocation, not preset names or brush settings: copy-on-write edits and
//! loading a stored tip's full bitmap must prepare fresh mips.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, OnceLock, PoisonError, Weak};

use crate::tile::{GrayTile, Mips};

// One maximum-size 5000² tip and its float pyramid fit (~175 MiB). Count both the mip
// buffers and source storage: even a weak Arc key reserves its allocation until eviction.
// Active strokes may keep using evicted pyramids through their own Arcs.
const MAX_BYTES: usize = 256 * 1024 * 1024;
const MAX_ENTRIES: usize = 64;
static CACHE: OnceLock<Mutex<MipCache>> = OnceLock::new();

struct Entry {
    width: u32,
    height: u32,
    // A weak key detects dropped sources and prevents allocation reuse for a different tip.
    // Arc::make_mut dissociates weak pointers on an in-place edit.
    samples: Weak<[u16]>,
    mips: Arc<Mips>,
    bytes: usize,
}

struct MipCache {
    entries: VecDeque<Entry>,
    bytes: usize,
    max_bytes: usize,
    max_entries: usize,
}

impl MipCache {
    fn new(max_bytes: usize, max_entries: usize) -> Self {
        Self { entries: VecDeque::new(), bytes: 0, max_bytes, max_entries }
    }

    fn get(&mut self, tip: &GrayTile) -> Option<Arc<Mips>> {
        let samples = Arc::downgrade(&tip.data);
        let index = self.entries.iter().position(|e| e.width == tip.width && e.height == tip.height && e.samples.ptr_eq(&samples))?;
        let entry = self.entries.remove(index)?;
        let mips = Arc::clone(&entry.mips);
        self.entries.push_back(entry);
        Some(mips)
    }

    fn insert(&mut self, tip: &GrayTile, mips: Arc<Mips>) -> Arc<Mips> {
        // Another caller may have finished preparing the same tip while this one built it.
        if let Some(existing) = self.get(tip) {
            return existing;
        }
        self.entries.retain(|e| e.samples.strong_count() > 0);
        self.bytes = self.entries.iter().fold(0usize, |n, e| n.saturating_add(e.bytes));
        let source_bytes = tip.data.len().saturating_mul(size_of::<u16>());
        let bytes = mips.levels.iter().fold(source_bytes, |n, (_, _, values)| n.saturating_add(values.capacity().saturating_mul(size_of::<f32>())));
        // An oversized tip still renders normally, without evicting useful cached tips or
        // extending the retention budget. Invalid tips retain Mips::new's ordinary fallback.
        if bytes > self.max_bytes || self.max_entries == 0 || !tip.is_valid() {
            return mips;
        }
        while self.bytes > self.max_bytes - bytes || self.entries.len() >= self.max_entries {
            let Some(old) = self.entries.pop_front() else { break };
            self.bytes = self.bytes.saturating_sub(old.bytes);
        }
        self.bytes = self.bytes.saturating_add(bytes);
        self.entries.push_back(Entry { width: tip.width, height: tip.height, samples: Arc::downgrade(&tip.data), mips: Arc::clone(&mips), bytes });
        mips
    }

    fn purge(&mut self) -> usize {
        self.entries.clear();
        std::mem::take(&mut self.bytes)
    }
}

fn cached(cache: &Mutex<MipCache>, tip: &GrayTile) -> Arc<Mips> {
    if let Some(mips) = cache.lock().unwrap_or_else(PoisonError::into_inner).get(tip) {
        return mips;
    }
    // Preparing a large pyramid must not hold the global lock: hits for other brushes and
    // unrelated documents can proceed. Concurrent misses may build twice, but publish one Arc.
    let mips = Arc::new(Mips::new(tip));
    cache.lock().unwrap_or_else(PoisonError::into_inner).insert(tip, mips)
}

pub(crate) fn tip_mips(tip: &GrayTile) -> Arc<Mips> {
    cached(CACHE.get_or_init(|| Mutex::new(MipCache::new(MAX_BYTES, MAX_ENTRIES))), tip)
}

/// Sample and mip buffer bytes retained by the prepared-tip cache, including weak-key storage.
pub fn mip_cache_bytes() -> usize {
    CACHE.get().map_or(0, |cache| cache.lock().unwrap_or_else(PoisonError::into_inner).bytes)
}

/// Release prepared-tip cache ownership and return its accounted buffer bytes. Active strokes
/// keep their shared pyramids and continue painting unchanged; the next stroke can prepare anew.
pub fn purge_mip_cache() -> usize {
    CACHE.get().map_or(0, |cache| cache.lock().unwrap_or_else(PoisonError::into_inner).purge())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tip(w: u32, h: u32) -> GrayTile {
        GrayTile::from_fn(w, h, |x, y| ((x * 13 + y * 7) % 31) as f32 / 30.0)
    }

    #[test]
    fn a_repeated_tip_reuses_every_level_after_the_first_stroke_drops() {
        let cache = Mutex::new(MipCache::new(MAX_BYTES, MAX_ENTRIES));
        let tip = tip(31, 17);
        let first = cached(&cache, &tip);
        let weak = Arc::downgrade(&first);
        assert_eq!(first.levels, Mips::new(&tip).levels);
        drop(first);
        let next = cached(&cache, &tip.clone());
        assert!(Arc::ptr_eq(&weak.upgrade().unwrap(), &next));
        assert_eq!(cache.lock().unwrap().entries.len(), 1);
    }

    #[test]
    fn dimensions_and_cow_sample_edits_invalidate_cached_mips() {
        let cache = Mutex::new(MipCache::new(MAX_BYTES, MAX_ENTRIES));
        let mut original = tip(8, 4);
        let first = cached(&cache, &original);
        let reshaped = GrayTile { width: 4, height: 8, data: Arc::clone(&original.data) };
        let changed_shape = cached(&cache, &reshaped);
        assert!(!Arc::ptr_eq(&first, &changed_shape));
        assert_eq!(changed_shape.levels, Mips::new(&reshaped).levels);
        let saved = original.clone();
        Arc::make_mut(&mut original.data)[0] = 65535;
        let changed_samples = cached(&cache, &original);
        assert!(!Arc::ptr_eq(&first, &changed_samples));
        assert_eq!(changed_samples.levels, Mips::new(&original).levels);
        assert!(Arc::ptr_eq(&first, &cached(&cache, &saved)));
        // With no other strong source owner, make_mut must also invalidate a weak cache key.
        let mut sole_owner = tip(5, 3);
        let old = cached(&cache, &sole_owner);
        Arc::make_mut(&mut sole_owner.data)[0] = 65535;
        assert!(!Arc::ptr_eq(&old, &cached(&cache, &sole_owner)));
    }

    #[test]
    fn retention_is_lru_bounded_by_both_bytes_and_entry_count() {
        let (a, b, c) = (tip(4, 4), tip(4, 4), tip(4, 4));
        let bytes = 16 * size_of::<u16>() + (16 + 4 + 1) * size_of::<f32>();
        let cache = Mutex::new(MipCache::new(2 * bytes, 64));
        let ma = cached(&cache, &a);
        let mb = cached(&cache, &b);
        assert!(Arc::ptr_eq(&ma, &cached(&cache, &a)));
        cached(&cache, &c);
        assert!(cache.lock().unwrap().get(&b).is_none());
        assert!(Arc::ptr_eq(&ma, &cached(&cache, &a)));
        assert_eq!(cache.lock().unwrap().bytes, 2 * bytes);
        assert_eq!(mb.levels, Mips::new(&b).levels, "eviction leaves active strokes valid");
        let cache = Mutex::new(MipCache::new(MAX_BYTES, 1));
        cached(&cache, &a);
        cached(&cache, &b);
        assert_eq!(cache.lock().unwrap().entries.len(), 1);
        assert!(cache.lock().unwrap().get(&a).is_none());
    }

    #[test]
    fn dead_sources_are_pruned_and_oversized_or_invalid_tips_bypass_the_cache() {
        let cache = Mutex::new(MipCache::new(116, 8));
        let small = tip(4, 4);
        let weak = Arc::downgrade(&small.data);
        cached(&cache, &small);
        drop(small);
        assert!(weak.upgrade().is_none());
        let live = tip(4, 4);
        let kept = cached(&cache, &live);
        assert_eq!(cache.lock().unwrap().entries.len(), 1);
        let large = tip(8, 8);
        let uncached = cached(&cache, &large);
        assert!(!Arc::ptr_eq(&uncached, &cached(&cache, &large)));
        assert!(Arc::ptr_eq(&kept, &cached(&cache, &live)));
        assert_eq!(cache.lock().unwrap().bytes, 116);
        let invalid = GrayTile { width: u32::MAX, height: u32::MAX, data: vec![1].into() };
        assert_eq!(cached(&cache, &invalid).levels, Mips::new(&invalid).levels);
        assert_eq!(cache.lock().unwrap().entries.len(), 1);
    }

    #[test]
    fn purging_releases_retention_without_invalidating_an_active_stroke() {
        let cache = Mutex::new(MipCache::new(MAX_BYTES, MAX_ENTRIES));
        let tip = tip(8, 4);
        let active = cached(&cache, &tip);
        assert!(cache.lock().unwrap().purge() > 0);
        assert_eq!(cache.lock().unwrap().bytes, 0);
        assert!(cache.lock().unwrap().entries.is_empty());
        let next = cached(&cache, &tip);
        assert!(!Arc::ptr_eq(&active, &next));
        assert_eq!(active.levels, next.levels);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn a_poisoned_cache_remains_usable() {
        let cache = Arc::new(Mutex::new(MipCache::new(MAX_BYTES, MAX_ENTRIES)));
        let other = Arc::clone(&cache);
        assert!(
            std::thread::spawn(move || {
                let _guard = other.lock().unwrap();
                panic!("test cache poison recovery");
            })
            .join()
            .is_err()
        );
        let tip = tip(4, 4);
        let first = cached(&cache, &tip);
        assert!(Arc::ptr_eq(&first, &cached(&cache, &tip)));
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn simultaneous_callers_publish_and_reuse_one_pyramid() {
        let cache = Arc::new(Mutex::new(MipCache::new(MAX_BYTES, MAX_ENTRIES)));
        let barrier = Arc::new(std::sync::Barrier::new(4));
        let tip = tip(257, 129);
        let workers: Vec<_> = (0..4)
            .map(|_| {
                let (cache, barrier, tip) = (Arc::clone(&cache), Arc::clone(&barrier), tip.clone());
                std::thread::spawn(move || {
                    barrier.wait();
                    cached(&cache, &tip)
                })
            })
            .collect();
        let results: Vec<_> = workers.into_iter().map(|w| w.join().unwrap()).collect();
        assert!(results.iter().all(|m| Arc::ptr_eq(m, &results[0])));
        assert_eq!(cache.lock().unwrap().entries.len(), 1);
    }
}
