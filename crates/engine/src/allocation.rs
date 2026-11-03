//! Fallible buffers used by memory-intensive document operations.

use crate::{EngineError, Result};

#[cfg(test)]
thread_local! {
    static FAIL_NEXT: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static FAIL_AFTER: std::cell::Cell<Option<usize>> = const { std::cell::Cell::new(None) };
}

/// Reserve and initialize a buffer without invoking the allocator's abort-on-OOM path.
pub(crate) fn filled<T: Clone>(len: usize, value: T, purpose: &str) -> Result<Vec<T>> {
    let mut out = capacity(len, purpose)?;
    out.resize(len, value);
    Ok(out)
}

/// Reserve a vector for a known number of operation results.
pub(crate) fn capacity<T>(len: usize, purpose: &str) -> Result<Vec<T>> {
    let bytes = len.checked_mul(std::mem::size_of::<T>()).ok_or_else(|| error(purpose, "requested size overflowed"))?;
    #[cfg(test)]
    if failpoint_hits() {
        return Err(error(purpose, "not enough memory"));
    }
    let mut out = Vec::new();
    out.try_reserve_exact(len).map_err(|_| error(purpose, &format!("not enough memory for {bytes} bytes")))?;
    Ok(out)
}

/// Check an operation boundary where downstream encoders/decoders allocate internally.
/// The test-only failpoint lets command tests prove their transaction boundary.
pub(crate) fn checkpoint(purpose: &str) -> Result<()> {
    #[cfg(test)]
    if failpoint_hits() {
        return Err(error(purpose, "not enough memory"));
    }
    let _ = purpose;
    Ok(())
}

/// Copy a region into an explicitly reserved buffer.
pub(crate) fn read_region(surface: &photocraft_raster::Surface, area: photocraft_geom::Rect, purpose: &str) -> Result<Vec<f32>> {
    let len = (area.width() as usize)
        .checked_mul(area.height() as usize)
        .and_then(|n| n.checked_mul(surface.channels()))
        .ok_or_else(|| error(purpose, "requested size overflowed"))?;
    let mut out = filled(len, 0.0, purpose)?;
    surface.read_region_into(area, &mut out);
    Ok(out)
}

pub(crate) fn error(purpose: &str, detail: &str) -> EngineError {
    EngineError::Other(format!("not enough memory for {purpose} ({detail}); the document was not changed"))
}

#[cfg(test)]
pub(crate) fn fail_next_for_test() {
    FAIL_NEXT.with(|flag| flag.set(true));
}

/// Fail the fallible allocation or checkpoint after `skip` clean attempts, then stop.
/// `fail_after_for_test(0)` is equivalent to [`fail_next_for_test`].
#[cfg(test)]
pub(crate) fn fail_after_for_test(skip: usize) {
    FAIL_AFTER.with(|flag| flag.set(Some(skip)));
}

/// Whether a test failpoint trips here: the single-shot flag, or the counted flag
/// once its `skip` attempts have passed cleanly earlier. The shot does not advance the
/// counted flag: a one-shot failure is not itself a clean attempt.
#[cfg(test)]
fn failpoint_hits() -> bool {
    if FAIL_NEXT.with(|flag| flag.replace(false)) {
        return true;
    }
    FAIL_AFTER.with(|flag| match flag.get() {
        Some(0) => {
            flag.set(None);
            true
        }
        Some(rest) => {
            flag.set(Some(rest - 1));
            false
        }
        None => false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allocation_failure_is_reported_without_panicking() {
        fail_next_for_test();
        let result = filled::<u8>(16, 0, "test image");
        assert!(result.as_ref().is_err_and(|e| e.to_string().contains("not enough memory for test image")));
    }

    #[test]
    fn fail_after_trips_only_on_the_counted_attempt() {
        fail_after_for_test(2);
        assert!(filled::<u8>(4, 0, "one").is_ok(), "the counted failpoint lets the first attempts pass cleanly");
        assert!(filled::<u8>(4, 0, "two").is_ok());
        assert!(filled::<u8>(4, 0, "three").is_err_and(|e| e.to_string().contains("not enough memory for three")));
        // The flag is consumed by the trip: later attempts are clean again.
        assert!(filled::<u8>(4, 0, "four").is_ok());
    }

    #[test]
    fn the_one_shot_does_not_spend_the_counted_failpoint() {
        // One-shot and counted flags both armed: the shot trips the first attempt, and that
        // failure is not a clean attempt, so the counter still has one to go past (review note
        // on #7353). Afterwards the counted attempt trips as usual.
        fail_after_for_test(1);
        fail_next_for_test();
        assert!(filled::<u8>(4, 0, "shot").is_err_and(|e| e.to_string().contains("not enough memory for shot")));
        assert!(filled::<u8>(4, 0, "skipped").is_ok(), "the counted attempt has not run out of clean attempts");
        assert!(filled::<u8>(4, 0, "counted").is_err_and(|e| e.to_string().contains("not enough memory for counted")));
        assert!(filled::<u8>(4, 0, "done").is_ok());
    }
}
