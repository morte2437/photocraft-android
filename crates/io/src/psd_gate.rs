//! Process-wide serialization gate for PSD/PSB pixel decoding.
//!
//! Prevents concurrent PSD imports from spending the same headroom at the same time,
//! using cancellation-aware waiting outside the UI thread. Reentrant/nested decodes on
//! the same thread are admitted without deadlocking and share the enclosing attempt's
//! budget. Approval prompts never hold this lock.

use std::cell::Cell;
use std::sync::{Condvar, Mutex, OnceLock};
use std::thread::ThreadId;
use std::time::Duration;

use crate::IoError;

struct GateState {
    active: bool,
    owner: Option<ThreadId>,
}

static GATE: OnceLock<(Mutex<GateState>, Condvar)> = OnceLock::new();

fn get_gate() -> &'static (Mutex<GateState>, Condvar) {
    GATE.get_or_init(|| (Mutex::new(GateState { active: false, owner: None }), Condvar::new()))
}

thread_local! {
    static NESTED_DEPTH: Cell<usize> = const { Cell::new(0) };
}

/// RAII guard releasing decode admission when dropped.
pub struct AdmissionGuard {
    is_top_level: bool,
}

impl Drop for AdmissionGuard {
    fn drop(&mut self) {
        if self.is_top_level {
            let (mutex, cvar) = get_gate();
            let mut state = mutex.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            state.active = false;
            state.owner = None;
            cvar.notify_one();
        } else {
            NESTED_DEPTH.with(|d| d.set(d.get().saturating_sub(1)));
        }
    }
}

/// Acquires process-wide admission to decode PSD pixels, or awaits completion of an active decode.
///
/// If the current thread already holds admission (a nested/reentrant decode), it is admitted
/// immediately without reacquiring the mutex or waiting on the condvar. Otherwise, it polls
/// `ctl.check()` every 50 ms while waiting so cancellation halts the wait immediately.
pub fn acquire_decode_admission(ctl: &photocraft_raster::Interrupt) -> Result<AdmissionGuard, IoError> {
    let current_thread = std::thread::current().id();

    let is_reentrant = {
        let (mutex, _) = get_gate();
        let state = mutex.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        state.active && state.owner == Some(current_thread)
    };

    if is_reentrant {
        NESTED_DEPTH.with(|d| d.set(d.get() + 1));
        return Ok(AdmissionGuard { is_top_level: false });
    }

    let (mutex, cvar) = get_gate();
    let mut state = mutex.lock().unwrap_or_else(std::sync::PoisonError::into_inner);

    while state.active {
        ctl.check().map_err(|_| IoError::Cancelled)?;
        let (next_state, _) = cvar.wait_timeout(state, Duration::from_millis(50)).unwrap_or_else(std::sync::PoisonError::into_inner);
        state = next_state;
    }

    ctl.check().map_err(|_| IoError::Cancelled)?;
    state.active = true;
    state.owner = Some(current_thread);

    Ok(AdmissionGuard { is_top_level: true })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reentrant_admission_does_not_deadlock() {
        let ctl = photocraft_raster::Interrupt::NONE;
        let guard1 = acquire_decode_admission(&ctl).unwrap();
        assert!(guard1.is_top_level);

        // Nested admission on the same thread
        let guard2 = acquire_decode_admission(&ctl).unwrap();
        assert!(!guard2.is_top_level);

        drop(guard2);
        drop(guard1);

        // Can acquire again after release
        let guard3 = acquire_decode_admission(&ctl).unwrap();
        assert!(guard3.is_top_level);
    }
}
