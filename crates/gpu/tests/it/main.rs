//! The crate's integration tests, linked into one test binary.

/// Concurrent wgpu instances in one process segfault on some drivers (RADV), so every test that
/// creates a device takes this lock and holds it until the device is dropped.
static GPU_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

mod device_loss;
mod parity;
mod timing;
mod uniform_limits;
mod vector_mask;
