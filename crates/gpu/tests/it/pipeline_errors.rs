//! #6419: the compositor's pipelines are built on several threads. A pipeline the device can't
//! build is still an `Err` from `try_new_with_format`, not a panic from wgpu's default error
//! handler (error scopes are per thread). Skips when no GPU adapter exists.

use std::sync::Arc;

use photocraft_color::{Color, ColorMode, SampleType};
use photocraft_doc::Document;
use photocraft_geom::Size;
use photocraft_gpu::{Compositor, render_to_vec};

use crate::GPU_LOCK;

fn device() -> Option<(wgpu::Adapter, wgpu::Device, wgpu::Queue)> {
    let instance = wgpu::Instance::default();
    let Ok(adapter) = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())) else {
        eprintln!("skipping: no adapter");
        return None;
    };
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).ok()?;
    Some((adapter, device, queue))
}

#[test]
fn a_pipeline_built_on_another_thread_fails_as_an_error() {
    let _lock = GPU_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let Some((adapter, device, _queue)) = device() else { return };
    // Never a render target, so every accumulation pipeline fails validation.
    let Err(e) = Compositor::try_new_with_format(&device, wgpu::TextureFormat::Rgb9e5Ufloat) else {
        panic!("a compositor accumulating into Rgb9e5Ufloat");
    };
    assert!(e.0.contains("couldn't build the compositor's pipelines"), "{e:?}");
    // The device is still usable for a compositor it can build.
    assert!(Compositor::try_new_with_format(&device, Compositor::preferred_acc_format(&adapter)).is_ok());
}

/// `new_with_format` on a device whose errors go to the caller's handler: a pipeline that failed
/// to build stays in the compositor as an invalid handle, so a draw reports GPU errors instead of
/// panicking on a missing pipeline.
#[test]
fn a_compositor_with_failed_pipelines_draws_without_panicking() {
    let _lock = GPU_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let Some((_adapter, device, queue)) = device() else { return };
    // As the app does (`DeviceHealth::watch`): GPU errors go to a handler, not wgpu's default panic.
    device.on_uncaptured_error(Arc::new(|_: wgpu::Error| {}));
    let mut comp = Compositor::new_with_format(&device, wgpu::TextureFormat::Rgb9e5Ufloat);
    let doc = Document::with_background("t", Size::new(8, 8), ColorMode::Rgb, SampleType::U8, Color::rgba(0.5, 0.5, 0.5, 1.0));
    // Drawing into an unrenderable format can't succeed, but must not panic.
    let _ = render_to_vec(&mut comp, &device, &queue, &doc, doc.bounds());
}
