//! Process-wide, capped pool of large scratch buffers.
//!
//! With glibc's mmap threshold pinned (`MALLOC_TRIM_THRESHOLD_`, which is how PhotoCraft returns
//! memory to the OS), every allocation over 128 KiB is a fresh `mmap` that is page-faulted in and
//! `munmap`ed on free. Tiles, compositor bands and filter scratch are all that size, so a single
//! viewport render took 350× the page faults. This pool keeps freed buffers (at least
//! [`MIN_POOLED_BYTES`] of capacity) on per-type free lists and hands them back out, so the
//! steady state allocates nothing.
//!
//! Retention is capped by [`set_budget`] (default [`DEFAULT_BUDGET`]); [`release`] drops
//! everything, which is what returns the memory to the OS. The engine releases when a document
//! closes; off wasm a background thread also releases once the pool has gone unused for
//! [`IDLE_RELEASE_SECS`], so every host (app, CLI, tests) gives memory back the same way.

use std::collections::{BTreeMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Mutex, PoisonError};

/// Buffers with less capacity than this are cheap from the heap and are never retained.
pub const MIN_POOLED_BYTES: usize = 64 * 1024;
/// Default cap on the bytes the pool retains: about the working set of a 36 MP filter (tiles,
/// halo images and result bands); a browser's wasm heap never shrinks, so there it is small.
pub const DEFAULT_BUDGET: usize = if cfg!(target_arch = "wasm32") { 256 << 20 } else { 2 << 30 };

/// Pooled bytes a miss never evicts. A filter's output tiles are all misses (the old tiles stay
/// in the undo history); evicting every byte they need also dropped the scratch planes the next
/// tile reuses. On a 36 MP Gaussian Blur with the trim threshold set, 256 MiB keeps the timings
/// at glibc's defaults (128 MiB: r 40 ~17% slower) for ~10% more peak memory (0: ~20% slower).
pub const DEFAULT_MISS_FLOOR: usize = if cfg!(target_arch = "wasm32") { 64 << 20 } else { 256 << 20 };

static BUDGET: AtomicUsize = AtomicUsize::new(DEFAULT_BUDGET);
static MISS_FLOOR: AtomicUsize = AtomicUsize::new(DEFAULT_MISS_FLOOR);
static RETAINED: AtomicUsize = AtomicUsize::new(0);
static USES: AtomicU64 = AtomicU64::new(0);
static IDLE_RELEASE: AtomicBool = AtomicBool::new(true);

/// Seconds without a [`take`] or [`give`] after which the idle thread releases the pool.
pub const IDLE_RELEASE_SECS: u64 = 2;

/// Starts the idle-release thread once (off wasm, where there are threads). If the thread
/// can't be spawned the pool still works; it is then released only by [`release`].
fn start_idle_release() {
    #[cfg(not(target_arch = "wasm32"))]
    {
        static STARTED: std::sync::Once = std::sync::Once::new();
        STARTED.call_once(|| {
            let _ = std::thread::Builder::new().name("scratch-idle".into()).spawn(|| {
                let mut seen = uses();
                loop {
                    std::thread::sleep(std::time::Duration::from_secs(IDLE_RELEASE_SECS));
                    let now = uses();
                    if now == seen {
                        // Decided under the lock, so `set_idle_release(false)` can't return
                        // while a release it should have stopped is about to start.
                        let old = {
                            let mut pool = lock();
                            // A `give` or `take` since `now` was read means the pool is in use.
                            (uses() == now && retained_bytes() > 0 && IDLE_RELEASE.load(Ordering::Relaxed)).then(|| empty(&mut pool))
                        };
                        drop(old);
                    }
                    seen = now;
                }
            });
        });
    }
}

/// One free list: buffers keyed by capacity (in elements), each with the sequence number it
/// was returned at (oldest first within a capacity).
#[doc(hidden)]
pub type FreeList<T> = BTreeMap<usize, VecDeque<(u64, Vec<T>)>>;

/// All the free lists behind one lock, so the oldest buffer of any type can be evicted.
/// `order` maps every pooled buffer's sequence number to its type and capacity, so the oldest
/// is found in O(log n); best-fit lookups are a range query on the capacity key.
#[doc(hidden)]
pub struct Pool {
    seq: u64,
    order: BTreeMap<u64, (Tag, usize)>,
    u8s: FreeList<u8>,
    u16s: FreeList<u16>,
    f32s: FreeList<f32>,
    px4: FreeList<[f32; 4]>,
}

static POOL: Mutex<Pool> =
    Mutex::new(Pool { seq: 0, order: BTreeMap::new(), u8s: BTreeMap::new(), u16s: BTreeMap::new(), f32s: BTreeMap::new(), px4: BTreeMap::new() });

/// Which free list a buffer lives on.
#[doc(hidden)]
#[derive(Clone, Copy)]
pub enum Tag {
    U8,
    U16,
    F32,
    Px4,
}

/// A buffer evicted from the pool, dropped by the caller once the lock is released.
#[allow(dead_code)]
enum Evicted {
    U8(Vec<u8>),
    U16(Vec<u16>),
    F32(Vec<f32>),
    Px4(Vec<[f32; 4]>),
}

/// Element types that have a free list. Implemented for the buffers the hot paths use.
pub trait Pooled: Copy + Send + 'static {
    #[doc(hidden)]
    const TAG: Tag;
    #[doc(hidden)]
    fn list(pool: &mut Pool) -> &mut FreeList<Self>;
}

macro_rules! pooled {
    ($($t:ty => $field:ident, $tag:ident),*) => {$(
        impl Pooled for $t {
            const TAG: Tag = Tag::$tag;
            fn list(pool: &mut Pool) -> &mut FreeList<Self> {
                &mut pool.$field
            }
        }
    )*};
}
pooled!(u8 => u8s, U8, u16 => u16s, U16, f32 => f32s, F32, [f32; 4] => px4, Px4);

fn bytes_of<T>(v: &Vec<T>) -> usize {
    v.capacity().saturating_mul(std::mem::size_of::<T>())
}

fn lock() -> std::sync::MutexGuard<'static, Pool> {
    POOL.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Remove the oldest buffer of capacity `cap` from `list`.
fn drop_oldest<T>(list: &mut FreeList<T>, cap: usize) -> Option<Vec<T>> {
    let queue = list.get_mut(&cap)?;
    let v = queue.pop_front()?.1;
    if queue.is_empty() {
        list.remove(&cap);
    }
    Some(v)
}

impl Pool {
    /// Remove the buffer returned longest ago, of any type. `None` when the pool is empty.
    fn evict_oldest(&mut self) -> Option<Evicted> {
        let (_, (tag, cap)) = self.order.pop_first()?;
        let evicted = match tag {
            Tag::U8 => drop_oldest(&mut self.u8s, cap).map(Evicted::U8),
            Tag::U16 => drop_oldest(&mut self.u16s, cap).map(Evicted::U16),
            Tag::F32 => drop_oldest(&mut self.f32s, cap).map(Evicted::F32),
            Tag::Px4 => drop_oldest(&mut self.px4, cap).map(Evicted::Px4),
        };
        if let Some(e) = &evicted {
            let freed = match e {
                Evicted::U8(v) => bytes_of(v),
                Evicted::U16(v) => bytes_of(v),
                Evicted::F32(v) => bytes_of(v),
                Evicted::Px4(v) => bytes_of(v),
            };
            RETAINED.fetch_sub(freed, Ordering::Relaxed);
        }
        evicted
    }
}

/// A `Vec` of `len` copies of `fill`, reusing a pooled buffer when one fits: the one whose
/// capacity equals `len`, else the smallest with capacity in `len..=2*len`.
pub fn take<T: Pooled>(len: usize, fill: T) -> Vec<T> {
    take_with(len, fill, 2)
}

/// An empty `Vec` with room for at least `cap` elements, reusing a pooled buffer when one fits.
pub fn take_cap<T: Pooled>(cap: usize) -> Vec<T> {
    USES.fetch_add(1, Ordering::Relaxed);
    match pop::<T>(cap, 2) {
        Some(mut v) => {
            v.clear();
            v
        }
        None => Vec::with_capacity(cap),
    }
}

/// Like [`take`] but only reuses a buffer whose capacity is exactly `len`, so
/// `into_boxed_slice` never reallocates.
pub fn take_exact<T: Pooled>(len: usize, fill: T) -> Vec<T> {
    take_with(len, fill, 1)
}

/// Pop the best-fitting free buffer: capacity `len`, else the smallest up to `len * slack`.
///
/// On a miss the caller allocates `len` fresh, so the oldest free buffers of at least that many
/// bytes are dropped first, down to [`set_miss_floor`]: memory the pool can't reuse for this request
/// goes back instead of piling up beside the new buffer, so pool plus live buffers exceed what
/// the program would hold without the pool by at most the floor (#10479).
fn pop<T: Pooled>(len: usize, slack: usize) -> Option<Vec<T>> {
    let need = len.saturating_mul(std::mem::size_of::<T>());
    if need < MIN_POOLED_BYTES {
        return None;
    }
    let mut evicted = Vec::new();
    let hit = {
        let mut pool = lock();
        let max = len.saturating_mul(slack);
        let list = T::list(&mut pool);
        let cap = list.range(len..=max).next().map(|(&cap, _)| cap);
        // The newest buffer of that capacity is the one most likely still in cache.
        let found = cap.and_then(|cap| {
            let queue = list.get_mut(&cap)?;
            let entry = queue.pop_back();
            if queue.is_empty() {
                list.remove(&cap);
            }
            entry
        });
        match found {
            Some((seq, v)) => {
                pool.order.remove(&seq);
                RETAINED.fetch_sub(bytes_of(&v), Ordering::Relaxed);
                Some(v)
            }
            None => {
                let target = RETAINED.load(Ordering::Relaxed).saturating_sub(need).max(MISS_FLOOR.load(Ordering::Relaxed));
                while RETAINED.load(Ordering::Relaxed) > target {
                    match pool.evict_oldest() {
                        Some(e) => evicted.push(e),
                        None => break,
                    }
                }
                None
            }
        }
    };
    // Freed outside the lock.
    drop(evicted);
    hit
}

fn take_with<T: Pooled>(len: usize, fill: T, slack: usize) -> Vec<T> {
    USES.fetch_add(1, Ordering::Relaxed);
    if len == 0 {
        return Vec::new();
    }
    match pop::<T>(len, slack) {
        Some(mut v) => {
            v.clear();
            v.resize(len, fill);
            v
        }
        None => vec![fill; len],
    }
}

/// Fallible [`take_exact`]: on a pool miss the heap allocation is `try_reserve_exact`, so
/// `None` means the allocation failed rather than aborting.
pub fn try_take_exact<T: Pooled>(len: usize, fill: T) -> Option<Vec<T>> {
    USES.fetch_add(1, Ordering::Relaxed);
    let mut v = match pop::<T>(len, 1) {
        Some(v) => v,
        None => {
            let mut v = Vec::new();
            if v.try_reserve_exact(len).is_err() {
                // The pool may hold up to the miss floor of buffers this request couldn't use:
                // give them back, then try once more before failing the caller.
                release();
                v.try_reserve_exact(len).ok()?;
            }
            v
        }
    };
    v.clear();
    v.resize(len, fill);
    Some(v)
}

/// Return a buffer to the pool. Small buffers, and buffers bigger than the whole budget, are
/// dropped; otherwise the buffers returned longest ago (of any type) make room for it.
pub fn give<T: Pooled>(v: Vec<T>) {
    USES.fetch_add(1, Ordering::Relaxed);
    let bytes = bytes_of(&v);
    if bytes < MIN_POOLED_BYTES {
        return;
    }
    let mut evicted = Vec::new();
    {
        let mut pool = lock();
        // Read under the lock, which `set_budget` also takes, so no insert uses a stale budget.
        let budget = BUDGET.load(Ordering::Relaxed);
        if bytes > budget {
            return;
        }
        start_idle_release();
        while RETAINED.load(Ordering::Relaxed).saturating_add(bytes) > budget {
            match pool.evict_oldest() {
                Some(e) => evicted.push(e),
                None => break,
            }
        }
        RETAINED.fetch_add(bytes, Ordering::Relaxed);
        pool.seq += 1;
        let seq = pool.seq;
        let cap = v.capacity();
        pool.order.insert(seq, (T::TAG, cap));
        T::list(&mut pool).entry(cap).or_default().push_back((seq, v));
    }
    // Freed outside the lock.
    drop(evicted);
}

/// Set the cap on retained bytes (existing entries beyond it go on the next [`give`]).
pub fn set_budget(bytes: usize) {
    let _pool = lock();
    BUDGET.store(bytes, Ordering::Relaxed);
}

/// Set the pooled bytes a miss never evicts (default [`DEFAULT_MISS_FLOOR`]).
pub fn set_miss_floor(bytes: usize) {
    MISS_FLOOR.store(bytes, Ordering::Relaxed);
}

/// Let the idle thread release the pool (the default) or not. Tests that check
/// [`retained_bytes`] turn it off, so a slow runner can't have the pool emptied under them.
pub fn set_idle_release(on: bool) {
    // Under the lock: once this returns, the idle thread can't be midway into a release.
    let _pool = lock();
    IDLE_RELEASE.store(on, Ordering::Relaxed);
}

/// Bytes currently held by the pool.
pub fn retained_bytes() -> usize {
    RETAINED.load(Ordering::Relaxed)
}

/// Number of [`take`]/[`give`] calls so far; a changing value means the pool is in use.
pub fn uses() -> u64 {
    USES.load(Ordering::Relaxed)
}

/// Drop every pooled buffer, handing the memory back to the OS.
pub fn release() {
    let old = empty(&mut lock());
    // Freed outside the lock.
    drop(old);
}

/// Everything `pool` holds, swapped out of it (to be freed once the lock is dropped).
fn empty(pool: &mut Pool) -> Pool {
    RETAINED.store(0, Ordering::Relaxed);
    let seq = pool.seq;
    std::mem::replace(pool, Pool { seq, order: BTreeMap::new(), u8s: BTreeMap::new(), u16s: BTreeMap::new(), f32s: BTreeMap::new(), px4: BTreeMap::new() })
}

/// A pooled buffer that derefs to a slice and returns itself to the pool on drop.
pub struct Buf<T: Pooled>(Vec<T>);

impl<T: Pooled> Buf<T> {
    /// `len` copies of `fill` from the pool.
    pub fn new(len: usize, fill: T) -> Self {
        Buf(take(len, fill))
    }
}

impl<T: Pooled> std::ops::Deref for Buf<T> {
    type Target = [T];
    fn deref(&self) -> &[T] {
        &self.0
    }
}

impl<T: Pooled> std::ops::DerefMut for Buf<T> {
    fn deref_mut(&mut self) -> &mut [T] {
        &mut self.0
    }
}

impl<T: Pooled> Drop for Buf<T> {
    fn drop(&mut self) {
        give(std::mem::take(&mut self.0));
    }
}
