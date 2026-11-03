//! Process-wide serialization gate for PSD/PSB pixel decoding.
//!
//! Prevents concurrent PSD imports from spending the same headroom at the same time,
//! using cancellation-aware waiting outside the UI thread. Reentrant/nested decodes on
//! the same thread are admitted without deadlocking and share the enclosing attempt's
//! budget. Approval prompts never hold this lock.
//!
//! Nested callers check cancellation and take the reentrant guard before calling
//! `consume_attempt_allowance`, so a cancelled or refused nested import never spends its
//! parent's remaining allowance; a failed charge deducts nothing.

use std::cell::Cell;
use std::marker::PhantomData;
use std::sync::{Condvar, Mutex, OnceLock};
use std::thread::ThreadId;
#[cfg(any(test, not(target_arch = "wasm32")))]
use std::time::Duration;

use crate::IoError;

struct GateState {
    active: bool,
    owner: Option<ThreadId>,
}

static GATE: OnceLock<(Mutex<GateState>, Condvar)> = OnceLock::new();

/// Returns the process-wide decode gate, creating its mutex and condition variable on first use.
fn get_gate() -> &'static (Mutex<GateState>, Condvar) {
    GATE.get_or_init(|| (Mutex::new(GateState { active: false, owner: None }), Condvar::new()))
}

thread_local! {
    static NESTED_DEPTH: Cell<usize> = const { Cell::new(0) };
    static ATTEMPT_ALLOWANCE: Cell<Option<u64>> = const { Cell::new(None) };
}

/// RAII guard releasing decode admission when dropped.
/// Non-Send and non-Sync to prevent cross-thread leakage or premature release.
pub(crate) struct AdmissionGuard {
    _marker: PhantomData<*const ()>,
}

impl Drop for AdmissionGuard {
    fn drop(&mut self) {
        let depth = NESTED_DEPTH.with(|d| {
            let val = d.get().saturating_sub(1);
            d.set(val);
            val
        });
        if depth == 0 {
            ATTEMPT_ALLOWANCE.with(|a| a.set(None));
            let (mutex, cvar) = get_gate();
            let mut state = mutex.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            state.active = false;
            state.owner = None;
            cvar.notify_one();
        }
    }
}

/// Returns whether the current thread currently holds decode admission.
pub(crate) fn is_holding_admission() -> bool {
    NESTED_DEPTH.with(|d| d.get() > 0)
}

/// Sets the remaining memory allowance for the active import attempt on the current thread.
pub(crate) fn set_active_attempt_budget(allowance: u64) {
    ATTEMPT_ALLOWANCE.with(|a| a.set(Some(allowance)));
}

/// Consumes `bytes` from the enclosing attempt's remaining allowance, or returns
/// [`IoError::PsdImportBudget`] if the required amount exceeds what remains.
///
/// `u64::MAX` is a format estimate's saturated overflow sentinel, not a requirement: it is
/// refused even when the remaining allowance is also `u64::MAX` (the native unknown-memory
/// attempt cap), and a refusal never spends the parent's allowance.
pub(crate) fn consume_attempt_allowance(bytes: u64) -> Result<(), IoError> {
    ATTEMPT_ALLOWANCE.with(|a| match a.get() {
        Some(remaining) => {
            if bytes == u64::MAX || bytes > remaining {
                return Err(IoError::PsdImportBudget { required: bytes, limit: remaining });
            }
            a.set(Some(remaining - bytes));
            Ok(())
        }
        None => {
            // Unknown allowance during reentrant gate must fail closed.
            Err(IoError::PsdImportBudget { required: bytes, limit: 0 })
        }
    })
}

/// The enclosing attempt's remaining allowance, for tests that assert a nested import was
/// charged instead of resetting the parent attempt's budget.
#[cfg(test)]
pub(crate) fn active_attempt_remaining() -> Option<u64> {
    ATTEMPT_ALLOWANCE.with(|a| a.get())
}

/// Acquires process-wide admission to decode PSD pixels, or awaits completion of an active decode.
///
/// If the current thread already holds admission (a nested/reentrant decode), it is admitted
/// immediately without reacquiring the mutex or waiting on the condvar. Otherwise, it polls
/// `ctl.check()` every 50 ms while waiting so cancellation halts the wait immediately.
pub(crate) fn acquire_decode_admission(ctl: &photocraft_raster::Interrupt) -> Result<AdmissionGuard, IoError> {
    let current_thread = std::thread::current().id();
    let (mutex, _cvar) = get_gate();
    let mut state = mutex.lock().unwrap_or_else(std::sync::PoisonError::into_inner);

    if state.active && state.owner == Some(current_thread) {
        ctl.check().map_err(|_| IoError::Cancelled)?;
        NESTED_DEPTH.with(|d| d.set(d.get().saturating_add(1)));
        return Ok(AdmissionGuard { _marker: PhantomData });
    }

    #[cfg(target_arch = "wasm32")]
    if state.active {
        // Stateless wasm is single-threaded: contention cannot be resolved by waiting on Condvar.
        return Err(IoError::Unsupported("concurrent PSD decoding is not supported on wasm".into()));
    }

    #[cfg(not(target_arch = "wasm32"))]
    while state.active {
        ctl.check().map_err(|_| IoError::Cancelled)?;
        let (next_state, _) = _cvar.wait_timeout(state, Duration::from_millis(50)).unwrap_or_else(std::sync::PoisonError::into_inner);
        state = next_state;
    }

    ctl.check().map_err(|_| IoError::Cancelled)?;
    state.active = true;
    state.owner = Some(current_thread);
    NESTED_DEPTH.with(|d| d.set(1));

    Ok(AdmissionGuard { _marker: PhantomData })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    #[test]
    fn reentrant_admission_does_not_deadlock() {
        let ctl = photocraft_raster::Interrupt::NONE;
        let guard1 = acquire_decode_admission(&ctl).unwrap();
        assert!(is_holding_admission());

        // Nested admission on the same thread
        let guard2 = acquire_decode_admission(&ctl).unwrap();
        assert!(is_holding_admission());

        drop(guard2);
        assert!(is_holding_admission());
        drop(guard1);
        assert!(!is_holding_admission());

        // Can acquire again after release
        let _guard3 = acquire_decode_admission(&ctl).unwrap();
        assert!(is_holding_admission());
    }

    #[test]
    fn unknown_allowance_during_reentrant_gate_fails_closed() {
        let ctl = photocraft_raster::Interrupt::NONE;
        let _guard = acquire_decode_admission(&ctl).unwrap();
        // Allowance is None (unknown)
        let res = consume_attempt_allowance(100);
        assert!(matches!(res, Err(IoError::PsdImportBudget { limit: 0, .. })));
    }

    #[test]
    fn nested_allowance_consumption_deducts_and_caps() {
        let ctl = photocraft_raster::Interrupt::NONE;
        let _guard = acquire_decode_admission(&ctl).unwrap();
        set_active_attempt_budget(5000);

        assert!(consume_attempt_allowance(2000).is_ok());
        assert_eq!(ATTEMPT_ALLOWANCE.with(|a| a.get()), Some(3000));

        assert!(consume_attempt_allowance(3001).is_err());
        assert!(consume_attempt_allowance(1000).is_ok());
        assert_eq!(ATTEMPT_ALLOWANCE.with(|a| a.get()), Some(2000));
    }

    /// A saturated estimate is not a charge, even when the parent's allowance is the same
    /// saturated value: it is refused and spends nothing.
    #[test]
    fn a_saturated_nested_charge_is_refused_and_spends_nothing() {
        let ctl = photocraft_raster::Interrupt::NONE;
        let _guard = acquire_decode_admission(&ctl).unwrap();
        set_active_attempt_budget(u64::MAX);

        let err = consume_attempt_allowance(u64::MAX).expect_err("the overflow sentinel is never a charge");
        assert!(matches!(err, IoError::PsdImportBudget { required, limit } if required == u64::MAX && limit == u64::MAX));
        assert_eq!(active_attempt_remaining(), Some(u64::MAX), "a refused sentinel charge leaves the parent's allowance untouched");

        // One byte below the sentinel is an ordinary, enormous charge.
        assert!(consume_attempt_allowance(u64::MAX - 1).is_ok());
        assert_eq!(active_attempt_remaining(), Some(1));
    }

    #[test]
    fn gate_serializes_concurrent_threads() {
        let ctl = photocraft_raster::Interrupt::NONE;
        let guard = acquire_decode_admission(&ctl).unwrap();

        let thread_started = Arc::new(AtomicBool::new(false));
        let thread_acquired = Arc::new(AtomicBool::new(false));

        let t_started = Arc::clone(&thread_started);
        let t_acquired = Arc::clone(&thread_acquired);
        let handle = std::thread::spawn(move || {
            let ctl = photocraft_raster::Interrupt::NONE;
            t_started.store(true, Ordering::SeqCst);
            let g = acquire_decode_admission(&ctl).unwrap();
            t_acquired.store(true, Ordering::SeqCst);
            drop(g);
        });

        while !thread_started.load(Ordering::SeqCst) {
            std::thread::yield_now();
        }
        // Give background thread time to reach condvar wait
        std::thread::sleep(Duration::from_millis(60));
        assert!(!thread_acquired.load(Ordering::SeqCst), "second thread must block while guard held");

        drop(guard);
        handle.join().unwrap();
        assert!(thread_acquired.load(Ordering::SeqCst), "second thread must acquire after release");
    }

    #[test]
    fn blocked_wait_aborts_on_cancellation() {
        let ctl = photocraft_raster::Interrupt::NONE;
        let guard = acquire_decode_admission(&ctl).unwrap();

        let cancelled = Arc::new(AtomicBool::new(false));
        let c_flag = Arc::clone(&cancelled);

        let flag_for_thread = Arc::clone(&cancelled);
        let handle = std::thread::spawn(move || {
            let cancelled = || flag_for_thread.load(Ordering::SeqCst);
            let progress = |_: f32| {};
            let interrupt = photocraft_raster::Interrupt::new(&cancelled, &progress);
            acquire_decode_admission(&interrupt).map(drop)
        });

        // Let thread block on condvar
        std::thread::sleep(Duration::from_millis(60));
        // Cancel waiting thread
        c_flag.store(true, Ordering::SeqCst);

        let res = handle.join().unwrap();
        assert!(matches!(res, Err(IoError::Cancelled)), "waiting thread must abort with Cancelled");

        // First thread releases guard
        drop(guard);

        // Subsequent acquire succeeds cleanly
        let next_guard = acquire_decode_admission(&ctl);
        assert!(next_guard.is_ok(), "gate must remain functional after cancelled wait");
    }
}
