//! Process-wide memory admission and adaptive tile residency. Native measurements are safe
//! `sysinfo` calls; web clients retain their explicit MB limit without filesystem assumptions.

#[cfg(not(target_arch = "wasm32"))]
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, PoisonError};

const MIB: u64 = 1 << 20;

#[derive(Clone, Copy, Debug, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub total_bytes: u64,
    pub available_bytes: u64,
    pub process_resident_bytes: u64,
    /// Address-space/virtual usage; this is not a portable measure of committed memory.
    pub process_virtual_bytes: u64,
}

#[derive(Clone, Copy, Debug)]
pub struct Policy {
    /// None keeps the pre-streaming absolute MB setting.
    pub percent: Option<u8>,
    pub absolute_bytes: u64,
}

impl Default for Policy {
    fn default() -> Self {
        Self { percent: None, absolute_bytes: 8 << 30 }
    }
}

#[derive(Clone, Copy, Debug, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Budgets {
    pub process_ceiling_bytes: u64,
    pub effective_process_bytes: u64,
    pub tile_bytes: u64,
    pub transient_bytes: u64,
}

/// Other applications and untiled buffers count against the same ceiling. Keep both system
/// headroom and a working reserve; virtual address-space size is deliberately not treated as RSS.
pub fn budgets(policy: Policy, sample: Snapshot, resident_tiles: u64) -> Budgets {
    let ceiling = match (policy.percent, sample.total_bytes) {
        (Some(p), total) if total > 0 => total / 100 * u64::from(p.clamp(1, 95)),
        _ => policy.absolute_bytes,
    }
    .max(MIB);
    let headroom = (sample.total_bytes / 20).max(256 * MIB);
    let effective = if sample.total_bytes == 0 {
        ceiling
    } else {
        ceiling.min(sample.process_resident_bytes.saturating_add(sample.available_bytes).saturating_sub(headroom))
    }
    .max(MIB);
    let transient = (effective / 8).clamp(MIB, 2 << 30);
    let other = sample.process_resident_bytes.saturating_sub(resident_tiles);
    let tiles = effective.saturating_sub(other).saturating_sub(transient).max(MIB);
    Budgets { process_ceiling_bytes: ceiling, effective_process_bytes: effective, tile_bytes: tiles, transient_bytes: transient }
}

static POLICY: Mutex<Policy> = Mutex::new(Policy { percent: None, absolute_bytes: 8 << 30 });
static CURRENT: Mutex<(Snapshot, Budgets)> = Mutex::new((
    Snapshot { total_bytes: 0, available_bytes: 0, process_resident_bytes: 0, process_virtual_bytes: 0 },
    Budgets { process_ceiling_bytes: 8 << 30, effective_process_bytes: 8 << 30, tile_bytes: 7 << 30, transient_bytes: 1 << 30 },
));
static RESERVED: AtomicU64 = AtomicU64::new(0);
static CONFIGURATION: AtomicU64 = AtomicU64::new(0);

pub fn configuration_generation() -> u64 {
    CONFIGURATION.load(Ordering::Relaxed)
}

#[cfg(not(target_arch = "wasm32"))]
pub fn measure() -> Snapshot {
    use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};
    static SYSTEM: OnceLock<Mutex<System>> = OnceLock::new();
    let mut s = SYSTEM.get_or_init(|| Mutex::new(System::new())).lock().unwrap_or_else(PoisonError::into_inner);
    s.refresh_memory();
    let pid = Pid::from_u32(std::process::id());
    s.refresh_processes_specifics(ProcessesToUpdate::Some(&[pid]), true, ProcessRefreshKind::nothing().with_memory());
    Snapshot {
        total_bytes: s.total_memory(),
        available_bytes: s.available_memory(),
        process_resident_bytes: s.process(pid).map_or(0, |p| p.memory()),
        process_virtual_bytes: s.process(pid).map_or(0, |p| p.virtual_memory()),
    }
}

#[cfg(target_arch = "wasm32")]
pub fn measure() -> Snapshot {
    Snapshot::default()
}

pub fn refresh() -> Budgets {
    let policy = *POLICY.lock().unwrap_or_else(PoisonError::into_inner);
    let sample = measure();
    let b = budgets(policy, sample, crate::spill::stats().resident_bytes as u64);
    *CURRENT.lock().unwrap_or_else(PoisonError::into_inner) = (sample, b);
    // The test override remains fixed even when process pressure changes.
    let tiles = std::env::var("PHOTOCRAFT_TILE_BUDGET_MB").ok().and_then(|s| s.parse::<u64>().ok()).map_or(b.tile_bytes, |mb| mb.saturating_mul(MIB));
    crate::spill::set_budget(usize::try_from(tiles).unwrap_or(usize::MAX));
    b
}

pub fn configure(policy: Policy) -> Budgets {
    *POLICY.lock().unwrap_or_else(PoisonError::into_inner) = policy;
    CONFIGURATION.fetch_add(1, Ordering::Relaxed);
    refresh()
}

pub fn stats() -> (Snapshot, Budgets, u64) {
    let (s, b) = *CURRENT.lock().unwrap_or_else(PoisonError::into_inner);
    (s, b, RESERVED.load(Ordering::Relaxed))
}

/// A bounded temporary working set. Callers retain the guard until the allocation is released.
pub struct Reservation(u64);

pub fn try_reserve(bytes: u64) -> Result<Reservation, String> {
    let (_, b, _) = stats();
    RESERVED
        .try_update(Ordering::AcqRel, Ordering::Relaxed, |n| n.checked_add(bytes).filter(|next| *next <= b.transient_bytes))
        .map_err(|_| "not enough working memory; reduce the operation or wait for other jobs".to_string())?;
    Ok(Reservation(bytes))
}

/// Admit a legacy operation that needs a whole buffer, including concurrent working sets and
/// the current process RSS. Streaming workers keep the smaller [`try_reserve`] allowance.
/// This deliberately refuses work before an allocation rather than relying on OS paging.
pub fn try_reserve_operation(bytes: u64) -> Result<Reservation, String> {
    let b = refresh();
    let (sample, _, _) = stats();
    // Resident tiles are evictable when spilling is on, so only the untiled part of RSS is
    // charged. The transient allowance is always admissible: it is what the budget reserves
    // for working sets, so normal-sized operations are never refused by a high RSS sample.
    let evictable = if crate::spill::enabled() { crate::spill::stats().resident_bytes as u64 } else { 0 };
    let untiled = sample.process_resident_bytes.saturating_sub(evictable);
    let available = if sample.total_bytes == 0 { b.transient_bytes } else { b.effective_process_bytes.saturating_sub(untiled).max(b.transient_bytes) };
    let refused = || "not enough working memory for this operation; reduce its region or use a streaming format".to_string();
    if bytes > available {
        return Err(refused());
    }
    // It fits on its own: wait (bounded) for concurrent operations to return their working
    // sets instead of failing an operation that only overlapped another one.
    #[cfg(not(target_arch = "wasm32"))]
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    loop {
        if RESERVED.try_update(Ordering::AcqRel, Ordering::Relaxed, |n| n.checked_add(bytes).filter(|next| *next <= available)).is_ok() {
            return Ok(Reservation(bytes));
        }
        #[cfg(target_arch = "wasm32")]
        return Err(refused());
        #[cfg(not(target_arch = "wasm32"))]
        {
            let now = std::time::Instant::now();
            if now >= deadline {
                return Err(refused());
            }
            let guard = RELEASED.0.lock().unwrap_or_else(PoisonError::into_inner);
            // Re-checked after the wake-up; a missed notification costs at most this slice.
            let slice = deadline.saturating_duration_since(now).min(std::time::Duration::from_millis(50));
            drop(RELEASED.1.wait_timeout(guard, slice).unwrap_or_else(PoisonError::into_inner));
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
static RELEASED: (Mutex<()>, std::sync::Condvar) = (Mutex::new(()), std::sync::Condvar::new());

impl Drop for Reservation {
    fn drop(&mut self) {
        RESERVED.fetch_sub(self.0, Ordering::AcqRel);
        #[cfg(not(target_arch = "wasm32"))]
        {
            let _guard = RELEASED.0.lock().unwrap_or_else(PoisonError::into_inner);
            RELEASED.1.notify_all();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ceiling_includes_non_tile_memory_and_other_apps() {
        let s = Snapshot { total_bytes: 64 << 30, available_bytes: 10 << 30, process_resident_bytes: 20 << 30, process_virtual_bytes: 100 << 30 };
        let b = budgets(Policy { percent: Some(90), absolute_bytes: 8 << 30 }, s, 15 << 30);
        assert!(b.process_ceiling_bytes > 57 << 30);
        assert!(b.effective_process_bytes < 30 << 30);
        assert!(b.tile_bytes + (5 << 30) + b.transient_bytes <= b.effective_process_bytes);
    }
    #[test]
    fn old_absolute_setting_and_missing_measurements_work() {
        let b = budgets(Policy { percent: None, absolute_bytes: 512 * MIB }, Snapshot::default(), 0);
        assert_eq!(b.process_ceiling_bytes, 512 * MIB);
        assert_eq!(b.tile_bytes + b.transient_bytes, b.effective_process_bytes);
    }
    #[test]
    fn hostile_percent_and_pressure_do_not_overflow() {
        let b = budgets(Policy { percent: Some(255), absolute_bytes: u64::MAX }, Snapshot { total_bytes: u64::MAX, ..Snapshot::default() }, u64::MAX);
        assert!(b.process_ceiling_bytes < u64::MAX);
        assert_eq!(b.tile_bytes, MIB);
    }
    #[test]
    fn an_operation_that_fits_alone_waits_for_a_concurrent_one() {
        let available = refresh().transient_bytes;
        let first = try_reserve_operation(available / 2 + 1).unwrap();
        let releaser = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(100));
            drop(first);
        });
        let second = try_reserve_operation(available / 2 + 1);
        releaser.join().unwrap();
        assert!(second.is_ok(), "overlapping work is serialized, not refused");
        assert!(try_reserve_operation(u64::MAX).is_err(), "work that can never fit is refused at once");
    }
    #[test]
    fn reservations_are_bounded_and_returned() {
        let before = stats().2;
        let r = try_reserve(1024).unwrap();
        assert_eq!(stats().2, before + 1024);
        assert!(try_reserve(u64::MAX).is_err());
        drop(r);
        assert_eq!(stats().2, before);
    }
}
