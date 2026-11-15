//! One GPU harness at a time per test process.
//!
//! `cargo test` runs a binary's tests on parallel threads, and several graphics drivers crash
//! when one process drives wgpu devices from more than one thread at once:
//!
//! - **WARP** (Windows): egui_kittest prefers a CPU adapter, which on Windows is WARP over DX12,
//!   so every kittest GPU test there runs on it. Two or more WARP devices rendering in parallel
//!   fault inside `d3d10warp.dll` (`STATUS_ACCESS_VIOLATION`); the GitHub Windows runners have no
//!   GPU, so CI runs on WARP too.
//! - **Mesa** llvmpipe over GL and RADV (#194).
//! - **libvulkan** with the NVIDIA and lavapipe drivers both installed: `wgpu::Instance::new` on
//!   several threads segfaults.
//!
//! Separately, the **Intel Vulkan driver** on Windows (`igvk64.dll`) crashes a process that so
//! much as creates a wgpu instance with Vulkan in it (#4), even on one thread. The app leaves
//! Vulkan out on Intel; tests use wgpu's defaults, so on such a machine run them with
//! `WGPU_BACKEND=dx12`. Tests that build their own instance read it through
//! `InstanceDescriptor::new_without_display_handle_from_env`; `wgpu::Instance::default()`
//! ignores it.
//!
//! So every test that builds a GPU harness (or its own wgpu device) holds [`gpu_lock`] for its
//! whole run, harness included. Under nextest each test is its own process and the lock costs
//! nothing; `.config/nextest.toml` caps how many of those run at once instead.
//!
//! The lock alone left a segfault in libvulkan about once in 20 runs of a large test binary: the
//! last wgpu instance going away tears Vulkan down, and the next one can find it half gone.
//! [`gpu_lock`] keeps one instance for the rest of the process, so that never happens.

use std::sync::{Mutex, MutexGuard, PoisonError};

static GPU_LOCK: Mutex<()> = Mutex::new(());

/// Hold this for as long as the test uses a GPU harness or device: take it on the test's first
/// line (`let _gpu = gpu_lock();`) so it is released last. A test that failed while holding it
/// doesn't block the rest.
pub fn gpu_lock() -> MutexGuard<'static, ()> {
    let guard = GPU_LOCK.lock().unwrap_or_else(PoisonError::into_inner);
    #[cfg(target_os = "linux")]
    keep_vulkan();
    guard
}

/// Creates the Vulkan instance that stays for the rest of the process, unless `WGPU_BACKEND`
/// leaves Vulkan out (to keep a broken Vulkan driver out of the tests).
#[cfg(target_os = "linux")]
fn keep_vulkan() {
    use wgpu::{Backends, Instance, InstanceDescriptor};
    static KEPT: std::sync::OnceLock<Option<Instance>> = std::sync::OnceLock::new();
    KEPT.get_or_init(|| {
        kept_backends(Backends::from_env()).map(|backends| Instance::new(InstanceDescriptor { backends, ..InstanceDescriptor::new_without_display_handle() }))
    });
}

/// The backends of the kept instance for a `WGPU_BACKEND` of `env`: Vulkan, if it allows it.
#[cfg(target_os = "linux")]
fn kept_backends(env: Option<wgpu::Backends>) -> Option<wgpu::Backends> {
    let vulkan = env.unwrap_or(wgpu::Backends::VULKAN) & wgpu::Backends::VULKAN;
    (!vulkan.is_empty()).then_some(vulkan)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Without the kept instance, creating and dropping instances on several threads segfaults in
    /// libvulkan within a second, an abort no test can catch. Skipped when `WGPU_BACKEND` leaves
    /// Vulkan out: nothing is kept then, and that is how a broken driver is kept out of the tests.
    #[cfg(target_os = "linux")]
    #[test]
    fn instances_can_come_and_go_on_several_threads() {
        use wgpu::{Backends, Instance, InstanceDescriptor};
        if kept_backends(Backends::from_env()).is_none() {
            eprintln!("skipping: WGPU_BACKEND leaves Vulkan out");
            return;
        }
        drop(gpu_lock());
        let threads: Vec<_> = (0..8)
            .map(|_| {
                std::thread::spawn(|| {
                    for _ in 0..10 {
                        drop(Instance::new(InstanceDescriptor { backends: Backends::VULKAN, ..InstanceDescriptor::new_without_display_handle() }));
                    }
                })
            })
            .collect();
        for t in threads {
            assert!(t.join().is_ok());
        }
    }

    /// `WGPU_BACKEND=gl` keeps Vulkan out of the tests: no Vulkan instance is kept then.
    #[cfg(target_os = "linux")]
    #[test]
    fn the_kept_instance_follows_wgpu_backend() {
        use wgpu::Backends;
        assert_eq!(kept_backends(None), Some(Backends::VULKAN));
        assert_eq!(kept_backends(Some(Backends::GL)), None);
        assert_eq!(kept_backends(Some(Backends::GL | Backends::VULKAN)), Some(Backends::VULKAN));
    }

    #[test]
    fn a_panicked_holder_does_not_block_the_next_test() {
        let _ = std::thread::spawn(|| {
            let _gpu = gpu_lock();
            panic!("a GPU test failed");
        })
        .join();
        drop(gpu_lock());
    }

    #[test]
    fn holders_take_turns() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static INSIDE: AtomicUsize = AtomicUsize::new(0);
        static MAX: AtomicUsize = AtomicUsize::new(0);
        let threads: Vec<_> = (0..8)
            .map(|_| {
                std::thread::spawn(|| {
                    let _gpu = gpu_lock();
                    let now = INSIDE.fetch_add(1, Ordering::SeqCst) + 1;
                    MAX.fetch_max(now, Ordering::SeqCst);
                    std::thread::sleep(std::time::Duration::from_millis(2));
                    INSIDE.fetch_sub(1, Ordering::SeqCst);
                })
            })
            .collect();
        for t in threads {
            assert!(t.join().is_ok());
        }
        assert_eq!(MAX.load(Ordering::SeqCst), 1);
    }
}
