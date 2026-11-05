//! How much memory can still be allocated, asked of the allocator itself.
//!
//! On the web (`wasm32-unknown-unknown`) a failed infallible allocation aborts the module and the
//! page freezes with no message, and stable Rust cannot turn that into an error. The allocations
//! made with `try_reserve*` do survive, though: the allocator hands back null when the browser
//! refuses to grow the linear memory, and the caller gets an error. So before a big operation
//! (place, resize, re-render a smart object, save) the operation asks [`ensure`] whether its
//! estimated need can be allocated right now, and fails with a message the user can act on
//! instead of trapping part-way.
//!
//! The answer is a trial reservation: one `Vec::try_reserve_exact`, freed at once. It sees the
//! real ceiling of this browser and device (a phone may stop well below 4 GiB) and the
//! fragmentation of the heap, costs nothing per allocation, and needs no allocator wrapper. The
//! freed block returns to the allocator's free list, so the operation reuses it. A reservation
//! is not written to, so the browser commits no physical pages for it.
//!
//! [`below`], which the low-memory warning asks every few seconds, does not reserve anything:
//! wasm linear memory never shrinks, so a periodic trial would raise the module's high-water
//! mark by the amount it asks about, for nothing. It reads the size of linear memory instead.
//!
//! Native targets overcommit, so the question has no useful answer there: every check passes and
//! desktop behaviour is unchanged.

use std::sync::atomic::{AtomicU64, Ordering};

use crate::AllocationError;

/// Room kept free after an operation's own need: frames, egui, notices and the error path.
pub const RESERVE: u64 = 128 << 20;

/// Marks [`simulate_available`] as off.
const OFF: u64 = u64::MAX;

static SIMULATED: AtomicU64 = AtomicU64::new(OFF);

/// Pretends that only `bytes` can be allocated (`None` stops pretending). The simulation is
/// process-wide and applies on every target. It is for tests and the web build's `?free-mb`
/// diagnostic flag only, so the gates and the low-memory warning can be seen without allocating
/// gigabytes.
pub fn simulate_available(bytes: Option<u64>) {
    SIMULATED.store(bytes.unwrap_or(OFF), Ordering::SeqCst);
}

fn simulated() -> Option<u64> {
    match SIMULATED.load(Ordering::SeqCst) {
        OFF => None,
        n => Some(n),
    }
}

/// The most linear memory a wasm32 module can address.
#[cfg(target_arch = "wasm32")]
const WASM32_CEILING: u64 = 4 << 30;

/// `true` when `bytes` can be allocated in one piece right now. Always `true` off the web.
fn can_allocate(bytes: u64) -> bool {
    if let Some(available) = simulated() {
        return bytes <= available;
    }
    trial(bytes)
}

#[cfg(target_arch = "wasm32")]
fn trial(bytes: u64) -> bool {
    let Ok(bytes) = usize::try_from(bytes) else { return false };
    let mut probe: Vec<u8> = Vec::new();
    probe.try_reserve_exact(bytes).is_ok()
}

#[cfg(not(target_arch = "wasm32"))]
fn trial(_bytes: u64) -> bool {
    true
}

/// `Ok` when `need` bytes (plus [`RESERVE`]) can be allocated right now. On the web, this tries
/// to reserve that much and frees it at once. On other targets it always passes (unless a
/// simulation is set).
pub fn ensure(need: u64) -> Result<(), AllocationError> {
    let total = need.saturating_add(RESERVE);
    let bytes = usize::try_from(total).map_err(|_| AllocationError::SizeOverflow)?;
    if can_allocate(total) { Ok(()) } else { Err(AllocationError::NotEnoughMemory { bytes }) }
}

/// `true` while the module's linear memory is within `bytes` of the 4 GiB wasm32 ceiling (web
/// only; always `false` elsewhere unless a simulation is set). Allocates nothing, unlike
/// [`ensure`]. Linear memory is a high-water mark, so this errs towards `true`: room freed inside
/// the allocator after a peak doesn't count. A browser that stops growing memory below 4 GiB isn't
/// seen either; the operation checks still are.
pub fn below(bytes: u64) -> bool {
    if let Some(available) = simulated() {
        return bytes > available;
    }
    linear_memory_left().is_some_and(|left| left < bytes)
}

#[cfg(target_arch = "wasm32")]
fn linear_memory_left() -> Option<u64> {
    let used = (core::arch::wasm32::memory_size::<0>() as u64).saturating_mul(64 << 10);
    Some(WASM32_CEILING.saturating_sub(used))
}

#[cfg(not(target_arch = "wasm32"))]
fn linear_memory_left() -> Option<u64> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    static LOCK: Mutex<()> = Mutex::new(());

    /// Serializes the tests (the simulation is global) and resets it when each one ends.
    struct Guard(#[allow(dead_code)] std::sync::MutexGuard<'static, ()>);

    impl Guard {
        fn new() -> Self {
            let guard = Guard(LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner));
            simulate_available(None);
            guard
        }
    }

    impl Drop for Guard {
        fn drop(&mut self) {
            simulate_available(None);
        }
    }

    const MIB: u64 = 1 << 20;

    #[test]
    fn passes_on_native_without_simulation() {
        let _g = Guard::new();
        assert!(ensure(10 << 30).is_ok());
        assert!(!below(64 << 30));
    }

    #[test]
    fn simulated_room_counts_the_reserve() {
        let _g = Guard::new();
        // 300 MiB of room: a 10 MiB need plus the 128 MiB reserve fits, a 200 MiB need does not.
        simulate_available(Some(300 * MIB));
        assert!(ensure(10 * MIB).is_ok());
        assert!(matches!(ensure(200 * MIB), Err(AllocationError::NotEnoughMemory { .. })));
        assert!(!below(300 * MIB));
        assert!(below(301 * MIB));
    }

    #[test]
    fn huge_needs_do_not_overflow() {
        let _g = Guard::new();
        let _ = ensure(u64::MAX);
        simulate_available(Some(100 * MIB));
        assert!(ensure(u64::MAX).is_err());
    }

    #[test]
    fn simulation_can_be_switched_off() {
        let _g = Guard::new();
        simulate_available(Some(0));
        assert!(ensure(0).is_err());
        simulate_available(None);
        assert!(ensure(0).is_ok());
    }
}
