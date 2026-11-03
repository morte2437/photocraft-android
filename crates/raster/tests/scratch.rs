//! The scratch pool is process-wide, so these tests live in their own binary (nothing else
//! allocates tiles here) and take a lock to run one at a time.

use photocraft_raster::scratch::*;
use std::sync::{Mutex, PoisonError};

static SERIAL: Mutex<()> = Mutex::new(());

/// Lets a miss evict every pooled byte for the test's lifetime, so sizes can be counted.
struct NoFloor;
impl NoFloor {
    /// Sets the miss floor to zero for this guard's lifetime.
    fn new() -> Self {
        set_miss_floor(0);
        NoFloor
    }
}
impl Drop for NoFloor {
    fn drop(&mut self) {
        set_miss_floor(DEFAULT_MISS_FLOOR);
    }
}
/// Locks the process-wide pool tests against concurrent access.
fn lock_serial() -> std::sync::MutexGuard<'static, ()> {
    SERIAL.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Runs a test alone with idle release off, so the idle thread can't empty the pool between a
/// `give` and the checks after it; idle release comes back on when dropped.
struct Serial {
    _lock: std::sync::MutexGuard<'static, ()>,
}
impl Drop for Serial {
    fn drop(&mut self) {
        set_idle_release(true);
    }
}
/// Serializes a test and disables idle release until the guard is dropped.
fn serial() -> Serial {
    let _lock = lock_serial();
    set_idle_release(false);
    Serial { _lock }
}

#[test]
/// Verifies pooled buffers are reused by pointer, retain capacity, and are refilled to the requested length.
fn reuse_returns_same_capacity_and_refills() {
    let _g = serial();
    release();
    let mut v = take(100_000, 0.0f32);
    v.iter_mut().for_each(|x| *x = 7.0);
    let ptr = v.as_ptr();
    give(v);
    assert_eq!(retained_bytes(), 400_000);
    let w = take(100_000, 1.5f32);
    assert_eq!(w.as_ptr(), ptr);
    assert_eq!(w.len(), 100_000);
    assert!(w.iter().all(|&x| x == 1.5));
    assert_eq!(retained_bytes(), 0);
    // A smaller request within 2x reuses it too, and is sized exactly.
    give(w);
    let s = take(60_000, 0.0f32);
    assert_eq!(s.as_ptr(), ptr);
    assert_eq!(s.len(), 60_000);
    release();
}

#[test]
/// Verifies oversized or non-exact buffers are discarded instead of reused.
fn too_big_a_buffer_is_not_reused() {
    let _g = serial();
    release();
    let _floor = NoFloor::new();
    give(vec![0u8; 1_000_000]);
    assert_eq!(take_exact(1_000_000, 0u8).len(), 1_000_000);
    assert_eq!(retained_bytes(), 0);
    give(vec![0u8; 1_000_000]);
    // More than 2x (or not exact): allocated fresh, and the miss drops the unusable buffer.
    assert_eq!(take(100_000, 0u8).capacity(), 100_000);
    assert_eq!(retained_bytes(), 0);
    give(vec![0u8; 1_000_000]);
    assert_eq!(take_exact(999_999, 0u8).capacity(), 999_999);
    assert_eq!(retained_bytes(), 0);
    release();
}

#[test]
/// Verifies exact try-takes reuse a matching buffer and succeed on a miss.
fn try_take_exact_reuses_and_allocates() {
    let _g = serial();
    release();
    let buf = vec![0u8; 200_000];
    let ptr = buf.as_ptr();
    give(buf);
    let v = try_take_exact(200_000, 9u8);
    assert!(v.as_ref().is_some_and(|v| v.len() == 200_000 && v.iter().all(|&b| b == 9)));
    assert_eq!(v.as_ref().map(|v| v.as_ptr()), Some(ptr));
    assert_eq!(retained_bytes(), 0);
    assert!(try_take_exact(300_000, 0u8).is_some());
    release();
}

#[test]
/// Verifies buffers below the pooling threshold are not retained.
fn small_buffers_are_not_retained() {
    let _g = serial();
    release();
    give(vec![0u8; MIN_POOLED_BYTES - 1]);
    give(vec![0.0f32; 100]);
    assert_eq!(retained_bytes(), 0);
    assert_eq!(take(10, 3u8), vec![3u8; 10]);
}

#[test]
/// Verifies zero-length take and give operations remain empty.
fn zero_length() {
    let _g = serial();
    assert!(take(0, 0u8).is_empty());
    give(Vec::<u8>::new());
    assert!(take_exact(0, [0.0f32; 4]).is_empty());
}

#[test]
/// Verifies the byte budget evicts oldest buffers and rejects buffers larger than the budget.
fn budget_evicts_oldest() {
    let _g = serial();
    release();
    set_budget(250_000);
    let (a, b) = (vec![0u8; 100_000], vec![0u8; 100_000]);
    let pb = b.as_ptr();
    give(a);
    give(b);
    give(vec![0u8; 100_000]);
    assert_eq!(retained_bytes(), 200_000);
    // The oldest was evicted; the survivors include b.
    let first = take_exact(100_000, 0u8);
    let second = take_exact(100_000, 0u8);
    assert!(first.as_ptr() == pb || second.as_ptr() == pb);
    // Larger than the whole budget: dropped.
    give(vec![0u8; 300_000]);
    assert_eq!(retained_bytes(), 0);
    set_budget(DEFAULT_BUDGET);
    release();
}

#[test]
/// Verifies release empties pooled buffers of several element types.
fn release_empties() {
    let _g = serial();
    give(vec![0u8; 200_000]);
    give(vec![0.0f32; 200_000]);
    give(vec![[0.0f32; 4]; 100_000]);
    assert!(retained_bytes() > 0);
    release();
    assert_eq!(retained_bytes(), 0);
}

#[test]
/// Verifies the usage counter advances for both take and give.
fn uses_counts_calls() {
    let _g = serial();
    let before = uses();
    give(take(10, 0u8));
    assert_eq!(uses(), before + 2);
}

#[test]
/// Verifies dropping a `Buf` returns its storage to the pool.
fn buf_returns_on_drop() {
    let _g = serial();
    release();
    {
        let mut b = Buf::new(150_000, 0.0f32);
        b[3] = 1.0;
        assert_eq!(b.len(), 150_000);
    }
    assert_eq!(retained_bytes(), 600_000);
    release();
}

#[test]
/// Verifies concurrent take and give operations preserve initialized values and finish cleanly.
fn concurrent_smoke() {
    let _g = serial();
    release();
    let handles: Vec<_> = (0..8u8)
        .map(|t| {
            std::thread::spawn(move || {
                for i in 0..200 {
                    let v = take(70_000 + i % 5, t);
                    assert!(v.iter().all(|&x| x == t));
                    give(v);
                }
            })
        })
        .collect();
    for h in handles {
        assert!(h.join().is_ok());
    }
    release();
    assert_eq!(retained_bytes(), 0);
}

#[test]
/// Verifies budget eviction uses insertion age across different element types.
fn eviction_is_oldest_first_across_types() {
    let _g = serial();
    release();
    set_budget(500_000);
    give(vec![0u8; 300_000]);
    give(vec![0.0f32; 50_000]);
    assert_eq!(retained_bytes(), 500_000);
    // Needs 200 000 more: the older u8 buffer goes, the f32 one stays.
    give(vec![[0.0f32; 4]; 12_500]);
    assert_eq!(retained_bytes(), 400_000);
    assert_eq!(take_exact(50_000, 1.0f32).len(), 50_000);
    assert_eq!(retained_bytes(), 200_000);
    set_budget(DEFAULT_BUDGET);
    release();
}

#[test]
/// Verifies reuse chooses the smallest eligible capacity and exact takes require an exact match.
fn best_fit_picks_the_smallest_fitting_capacity() {
    let _g = serial();
    release();
    let _floor = NoFloor::new();
    for n in [180_000usize, 120_000, 150_000, 300_000] {
        give(vec![0u8; n]);
    }
    // Within 2x of 100 000: 120 000 is the tightest fit, then 150 000, then 180 000.
    assert_eq!(take(100_000, 0u8).capacity(), 120_000);
    assert_eq!(take(100_000, 0u8).capacity(), 150_000);
    assert_eq!(take(100_000, 0u8).capacity(), 180_000);
    // 300 000 is more than 2x: allocated fresh, and the miss drops the pooled one.
    assert_eq!(take(100_000, 0u8).capacity(), 100_000);
    assert_eq!(retained_bytes(), 0);
    // An exact capacity beats a smaller-slack scan.
    give(vec![0u8; 100_000]);
    assert_eq!(take_exact(100_000, 0u8).capacity(), 100_000);
    assert_eq!(take_exact(250_000, 0u8).capacity(), 250_000);
    release();
}

#[test]
/// Verifies repeated budget overflow evicts oldest entries across mixed types and capacities.
fn eviction_walks_oldest_first_through_mixed_types_and_capacities() {
    let _g = serial();
    release();
    let _floor = NoFloor::new();
    set_budget(700_000);
    // Oldest to newest, 100 000 bytes each, interleaved types and two u8 capacities.
    give(vec![0u8; 100_000]);
    let f = vec![0.0f32; 25_000];
    give(f);
    give(vec![0u8; 100_000]);
    give(vec![0u16; 50_000]);
    let newest = vec![0u8; 150_000];
    let np = newest.as_ptr();
    give(newest);
    assert_eq!(retained_bytes(), 550_000);
    // 200 000 more overflows by 50 000: the oldest (u8) goes.
    give(vec![[0.0f32; 4]; 12_500]);
    assert_eq!(retained_bytes(), 650_000);
    // 100 000 more: the next oldest (the f32 buffer) goes.
    give(vec![0u8; 100_000]);
    assert_eq!(retained_bytes(), 650_000);
    // 200 000 more: the u8 then the u16 buffer go.
    give(vec![0u8; 200_000]);
    assert_eq!(retained_bytes(), 650_000);
    // The newest u8 buffer of the old batch is still pooled; the f32 one was evicted.
    let again = take_exact(150_000, 0u8);
    assert_eq!(again.as_ptr(), np);
    assert_eq!(retained_bytes(), 500_000);
    // The evicted f32 buffer is a miss: the oldest buffers, at least its 100 000 bytes, leave
    // (the 200 000-byte [f32; 4] one).
    assert_eq!(take_exact(25_000, 0.0f32).len(), 25_000);
    assert_eq!(retained_bytes(), 300_000);
    set_budget(DEFAULT_BUDGET);
    release();
}

#[test]
/// Verifies the idle-release worker clears an unused pooled buffer.
fn idle_pool_is_released_by_the_background_thread() {
    // Idle release stays on here: it is what this test checks.
    let _g = lock_serial();
    release();
    give(vec![0u8; MIN_POOLED_BYTES]);
    assert_eq!(retained_bytes(), MIN_POOLED_BYTES);
    // Released after one full idle interval without use (at most two from the give).
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3 * IDLE_RELEASE_SECS + 1);
    while retained_bytes() > 0 && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    assert_eq!(retained_bytes(), 0);
}

#[test]
/// Verifies a miss discards pooled bytes in proportion to its allocation and a hit preserves the pool.
fn a_miss_drops_as_many_pooled_bytes_as_it_allocates() {
    let _g = serial();
    release();
    let _floor = NoFloor::new();
    // Three 64 KiB f32 buffers a byte request can't use.
    for _ in 0..3 {
        give(vec![0.0f32; MIN_POOLED_BYTES / 4]);
    }
    assert_eq!(retained_bytes(), 3 * MIN_POOLED_BYTES);
    // A 128 KiB miss drops the two oldest, so pool plus live buffers stay at 192 KiB.
    let v = take(2 * MIN_POOLED_BYTES, 0u8);
    assert_eq!(retained_bytes(), MIN_POOLED_BYTES);
    give(v);
    // A hit drops nothing.
    let w = take(MIN_POOLED_BYTES / 4, 0.0f32);
    assert_eq!(retained_bytes(), 2 * MIN_POOLED_BYTES);
    drop(w);
    release();
}

#[test]
/// Verifies miss eviction stops when the configured retained-byte floor is reached.
fn a_miss_keeps_the_floor() {
    let _g = serial();
    release();
    set_miss_floor(2 * MIN_POOLED_BYTES);
    for _ in 0..3 {
        give(vec![0.0f32; MIN_POOLED_BYTES / 4]);
    }
    // A 192 KiB miss would drop all three; only one goes, down to the floor.
    assert_eq!(take(3 * MIN_POOLED_BYTES, 0u8).len(), 3 * MIN_POOLED_BYTES);
    assert_eq!(retained_bytes(), 2 * MIN_POOLED_BYTES);
    set_miss_floor(DEFAULT_MISS_FLOOR);
    release();
}

#[test]
/// Verifies a failed oversized reservation leaves the pool empty.
fn a_failed_reservation_releases_the_pool() {
    let _g = serial();
    release();
    // Unmatched buffers under the miss floor stay pooled on a miss.
    give(vec![0u8; 4 * MIN_POOLED_BYTES]);
    assert!(retained_bytes() > 0);
    // No allocator can reserve this: the call fails, but not before the pool was emptied. (The
    // retry after the release isn't observable here: it needs an allocator that fails once.)
    assert!(try_take_exact(isize::MAX as usize, 0u8).is_none());
    assert_eq!(retained_bytes(), 0);
}

#[test]
/// Verifies a successful miss preserves unmatched buffers within the miss floor.
fn a_successful_miss_keeps_unmatched_pooled_buffers() {
    let _g = serial();
    release();
    give(vec![0u8; 4 * MIN_POOLED_BYTES]);
    let v = try_take_exact(2 * MIN_POOLED_BYTES, 0u8);
    assert!(v.as_ref().is_some_and(|v| v.len() == 2 * MIN_POOLED_BYTES && v.iter().all(|&b| b == 0)));
    assert_eq!(retained_bytes(), 4 * MIN_POOLED_BYTES, "within the miss floor, nothing is dropped");
    release();
}
