//! Data-parallel helpers: rayon on native targets, sequential on wasm.

use std::sync::{Mutex, PoisonError};

/// Calls `f(index, chunk)` for each `chunk`-sized piece of `buf`.
pub(crate) fn chunks_mut<T: Send>(buf: &mut [T], chunk: usize, f: impl Fn(usize, &mut [T]) + Sync + Send) {
    let chunk = chunk.max(1);
    #[cfg(not(target_arch = "wasm32"))]
    {
        use rayon::prelude::*;
        buf.par_chunks_mut(chunk).enumerate().for_each(|(i, c)| f(i, c));
    }
    #[cfg(target_arch = "wasm32")]
    buf.chunks_mut(chunk).enumerate().for_each(|(i, c)| f(i, c));
}

/// Like [`chunks_mut`], with `f(state, index, chunk)` borrowing a state: a chunk takes a free one
/// or makes one with `init`, and gives it back when done, so rayon keeps its fine-grained
/// splitting and stealing while at most one state per worker is ever made (`f` must not itself
/// wait on rayon work). Rayon's `for_each_init` would make one per stolen piece, which on an idle
/// pool is often every chunk. A state no chunk still waiting can take is dropped at once, so
/// near the end they go as their workers finish rather than all at the very end.
pub(crate) fn chunks_mut_pooled<T: Send, S: Send>(
    buf: &mut [T],
    chunk: usize,
    init: impl Fn() -> S + Sync + Send,
    f: impl Fn(&mut S, usize, &mut [T]) + Sync + Send,
) {
    let chunk = chunk.max(1);
    // The free states and the number of chunks that have not taken one yet.
    let pool = Mutex::new((Vec::new(), buf.len().div_ceil(chunk)));
    chunks_mut(buf, chunk, |i, c| {
        let taken = {
            let mut pool = pool.lock().unwrap_or_else(PoisonError::into_inner);
            pool.1 = pool.1.saturating_sub(1);
            pool.0.pop()
        };
        let mut state = taken.unwrap_or_else(&init);
        f(&mut state, i, c);
        let mut pool = pool.lock().unwrap_or_else(PoisonError::into_inner);
        if pool.0.len() < pool.1 {
            pool.0.push(state);
        }
    });
}

/// Native worker count (the browser builds run sequentially).
pub(crate) fn workers() -> usize {
    #[cfg(not(target_arch = "wasm32"))]
    {
        rayon::current_num_threads()
    }
    #[cfg(target_arch = "wasm32")]
    {
        1
    }
}

/// Processes normal-sized chunks with at most `jobs` live scratch buffers.
pub(crate) fn chunks_mut_limited<T: Send>(buf: &mut [T], chunk: usize, jobs: usize, f: impl Fn(usize, &mut [T]) + Sync + Send) {
    let chunk = chunk.max(1);
    // Each worker holds one band's scratch at a time. When the normal worker
    // count fits, preserve Rayon's fine-grained load balancing.
    if jobs >= workers() {
        chunks_mut(buf, chunk, f);
        return;
    }
    let per_group = buf.len().div_ceil(chunk).div_ceil(jobs.max(1)).max(1);
    chunks_mut(buf, chunk.saturating_mul(per_group), |group, part| {
        for (local, c) in part.chunks_mut(chunk).enumerate() {
            f(group * per_group + local, c);
        }
    });
}

/// Maps `f` over `0..n`, keeping order.
pub(crate) fn map<T: Send>(n: usize, f: impl Fn(usize) -> T + Sync + Send) -> Vec<T> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        use rayon::prelude::*;
        (0..n).into_par_iter().map(f).collect()
    }
    #[cfg(target_arch = "wasm32")]
    (0..n).map(f).collect()
}

/// Rows per parallel band for an image `width` samples wide (about 64 K samples a band).
pub(crate) fn band_rows(width: usize) -> usize {
    (65536 / width.max(1)).clamp(1, 256)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Every chunk is visited once with its own index (the last one short), and the chunks share
    /// the states: a 4-worker pool makes at most 4 for 101 chunks, an empty buffer none.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn chunks_mut_pooled_makes_one_state_per_worker_at_most() {
        let pool = rayon::ThreadPoolBuilder::new().num_threads(4).build().unwrap();
        pool.install(|| {
            let mut buf = vec![0usize; 1003];
            let inits = AtomicUsize::new(0);
            let runs = AtomicUsize::new(0);
            let init = || {
                inits.fetch_add(1, Ordering::Relaxed);
                0usize
            };
            chunks_mut_pooled(&mut buf, 10, init, |used, i, c| {
                *used += 1;
                runs.fetch_add(1, Ordering::Relaxed);
                c.iter_mut().for_each(|v| *v += i + 1);
            });
            assert!(buf.iter().enumerate().all(|(k, &v)| v == k / 10 + 1));
            assert_eq!(runs.load(Ordering::Relaxed), 101);
            let n = inits.load(Ordering::Relaxed);
            assert!((1..=4).contains(&n), "{n} states for 101 chunks on 4 workers");
            chunks_mut_pooled(&mut [0u8; 0], 4, init, |_, _, _| {});
            assert_eq!(inits.load(Ordering::Relaxed), n, "no state for an empty buffer");
        });
    }
}
