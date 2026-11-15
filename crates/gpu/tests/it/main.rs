//! The crate's integration tests, linked into one test binary.

/// Concurrent wgpu instances in one process segfault on some drivers (RADV), so every test that
/// creates a device takes this lock and holds it until the device is dropped.
static GPU_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// The wgpu instance every test uses (`WGPU_BACKEND` picks its backends), kept for the whole
/// run: the last instance going away tears the Vulkan loader down, and an instance made while
/// that happens can crash in libvulkan.
pub(crate) fn instance() -> wgpu::Instance {
    static KEPT: std::sync::OnceLock<wgpu::Instance> = std::sync::OnceLock::new();
    KEPT.get_or_init(|| {
        INSTANCES_MADE.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env())
    })
    .clone()
}

/// How many instances [`instance`] has made: one for the whole run.
static INSTANCES_MADE: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// [`instance`] makes its instance once, however many tests and threads ask for it: a fresh one
/// per test could crash in libvulkan while another goes away.
#[test]
fn the_instance_is_made_once() {
    let threads: Vec<_> = (0..4).map(|_| std::thread::spawn(|| drop(instance()))).collect();
    for t in threads {
        assert!(t.join().is_ok());
    }
    drop(instance());
    assert_eq!(INSTANCES_MADE.load(std::sync::atomic::Ordering::SeqCst), 1);
}

mod bounded_readback;
mod camera_raw;
mod device_loss;
mod parity;
mod pipeline_errors;
mod pipelines;
mod timing;
mod uniform_limits;
mod vector_mask;
