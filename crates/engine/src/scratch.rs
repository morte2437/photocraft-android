//! Native tile storage configuration, kept out of the preference data model.

use crate::prefs::Preferences;

/// A web session has no filesystem or memory-supervision thread.
#[cfg(target_arch = "wasm32")]
#[derive(Default)]
pub(crate) struct Runtime;

#[cfg(not(target_arch = "wasm32"))]
pub(crate) use native::Runtime;

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use std::sync::{Arc, Condvar, Mutex, OnceLock, PoisonError, Weak};
    use std::thread::JoinHandle;
    use std::time::{Duration, Instant};

    // Weak ownership: the process-wide cache cannot keep a session's worker alive.
    pub(crate) struct Runtime {
        _worker: Arc<Worker>,
    }

    impl Default for Runtime {
        fn default() -> Self {
            static SHARED: OnceLock<Mutex<Weak<Worker>>> = OnceLock::new();
            let mut shared = SHARED.get_or_init(Mutex::default).lock().unwrap_or_else(PoisonError::into_inner);
            let worker = shared.upgrade().unwrap_or_else(|| {
                let worker = Arc::new(Worker::start());
                *shared = Arc::downgrade(&worker);
                worker
            });
            Self { _worker: worker }
        }
    }

    struct Worker {
        stop: Arc<(Mutex<bool>, Condvar)>,
        thread: Option<JoinHandle<()>>,
    }

    impl Worker {
        fn start() -> Self {
            let stop = Arc::new((Mutex::new(false), Condvar::new()));
            let flag = stop.clone();
            let thread = std::thread::Builder::new()
                .name("pc-memory-residency".into())
                .spawn(move || {
                    let _maintenance = photocraft_raster::spill::background_maintenance();
                    let mut measured = Instant::now();
                    loop {
                        let (lock, wake) = &*flag;
                        let stopped = lock.lock().unwrap_or_else(PoisonError::into_inner);
                        let (stopped, _) = wake.wait_timeout_while(stopped, Duration::from_millis(100), |stop| !*stop).unwrap_or_else(PoisonError::into_inner);
                        if *stopped {
                            break;
                        }
                        drop(stopped);
                        if measured.elapsed() >= Duration::from_secs(2) {
                            photocraft_raster::memory::refresh();
                            measured = Instant::now();
                        }
                        photocraft_raster::spill::maintain_budget();
                    }
                })
                .ok();
            // If spawning fails, raster's synchronous fallback still enforces the budget.
            Self { stop, thread }
        }
    }

    impl Drop for Worker {
        fn drop(&mut self) {
            let (lock, wake) = &*self.stop;
            *lock.lock().unwrap_or_else(PoisonError::into_inner) = true;
            wake.notify_all();
            if let Some(thread) = self.thread.take() {
                let _ = thread.join();
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn workers_stop_join_and_can_restart() {
            for _ in 0..3 {
                let worker = Worker::start();
                assert!(worker.thread.is_some());
                let stop = worker.stop.clone();
                let began = Instant::now();
                drop(worker);
                assert!(*stop.0.lock().unwrap());
                assert!(began.elapsed() < Duration::from_secs(2));
            }
        }

        #[test]
        fn sessions_share_supervision_without_static_ownership() {
            let a = Runtime::default();
            let b = Runtime::default();
            assert!(Arc::ptr_eq(&a._worker, &b._worker));
            let weak = Arc::downgrade(&a._worker);
            drop((a, b));
            // Other engine tests can hold the shared runtime concurrently; a private worker
            // proves the same ownership contract without depending on their timing.
            let worker = Arc::new(Worker::start());
            let private = Arc::downgrade(&worker);
            drop(worker);
            assert!(private.upgrade().is_none());
            drop(weak);
        }
    }
}

pub(crate) fn configure(prefs: &Preferences) {
    let budget = photocraft_raster::memory::configure(photocraft_raster::memory::Policy {
        percent: prefs.performance.memory_usage_percent,
        // The web build's cap applies here too (effective_memory_usage_mb).
        absolute_bytes: u64::from(prefs.performance.effective_memory_usage_mb()).saturating_mul(1 << 20),
    });
    #[cfg(not(target_arch = "wasm32"))]
    {
        let dir = prefs
            .scratch_disks
            .disks
            .iter()
            .find(|d| d.enabled && !d.path.trim().is_empty())
            .map(|d| if d.path.trim() == "(system temp)" { std::env::temp_dir() } else { std::path::PathBuf::from(d.path.trim()) });
        // An unusable or full directory pauses eviction without dropping resident pixels and
        // is retried with backoff; History's byte limit applies meanwhile (sync_scratch_state).
        // The reason is exposed in ui.inspect and the application's status bar.
        let tiles = if std::env::var_os("PHOTOCRAFT_TILE_BUDGET_MB").is_some() {
            prefs.performance.tile_budget_bytes()
        } else {
            usize::try_from(budget.tile_bytes).unwrap_or(usize::MAX)
        };
        let limit = prefs.scratch_disks.size_limit_mb;
        photocraft_raster::spill::set_size_cap((limit > 0).then(|| u64::from(limit).saturating_mul(1 << 20)));
        let _ = photocraft_raster::spill::configure(dir.as_deref(), tiles);
    }
    #[cfg(target_arch = "wasm32")]
    let _ = (prefs, budget);
}
