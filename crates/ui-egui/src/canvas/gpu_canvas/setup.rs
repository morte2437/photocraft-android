use super::texels::texels;
use super::{egui_wgpu, wgpu};
use crate::render_path::CpuReason;

/// A CPU composite as the canvas texture stores it (`display`'s encoding; as is without one).
pub(super) fn texture_buffer<'a>(
    display: Option<&photocraft_engine::display_color::CanvasDisplay>,
    buf: &'a photocraft_compose::Buffer,
) -> std::borrow::Cow<'a, photocraft_compose::Buffer> {
    match display {
        Some(d) => d.texture_buffer(buf),
        None => std::borrow::Cow::Borrowed(buf),
    }
}

/// A CPU composite `buf` as texels of a `format` canvas texture, in `display`'s encoding: what
/// [`GpuCanvas::upload_rect`] takes. Runs anywhere (a redraw's worker converts its own pieces).
pub fn display_texels(
    format: wgpu::TextureFormat,
    display: Option<&photocraft_engine::display_color::CanvasDisplay>,
    buf: &photocraft_compose::Buffer,
) -> Vec<u8> {
    texels(format, &texture_buffer(display, buf).px)
}

/// What a [`GpuCanvas::refresh`] did.
#[derive(Clone, Debug, Default)]
pub struct Refresh {
    /// "gpu-rect", "gpu-full" (wgpu compositor), "rect" or "full" (CPU compositor); "lost" or
    /// [`REFRESH_OVER_BUDGET`] when nothing was drawn and the caller should use the CPU canvas.
    pub kind: &'static str,
    /// Pixels refreshed.
    pub px: u64,
    pub composite_ms: f64,
    /// Time spent on a failed GPU composite attempt before CPU fallback.
    pub gpu_attempt_ms: f64,
    pub upload_ms: f64,
    /// Layer tiles the wgpu compositor uploaded.
    pub uploads: u64,
    /// How long the GPU spent executing a recent wgpu composite, once its timestamps came back
    /// (a frame or two after that refresh, so usually an earlier one than this). It carries its
    /// own pixel count and is never this refresh's unless they match. `None` when the device has
    /// no timestamp queries, on the CPU compositor, and on wasm: no number at all is reported
    /// rather than `composite_ms`, which measures encoding and submitting, not executing. It
    /// leaves out layer-tile uploads (`uploads` above counts them), so a first refresh of a
    /// document under-reports by more than the few per cent of a warm one.
    pub gpu: Option<photocraft_gpu::GpuTiming>,
    /// Why the wgpu compositor wasn't used.
    pub fallback: Option<CpuReason>,
}

/// How the app creates its wgpu device: egui's defaults clamped to the adapter's supported limits,
/// with the adapter's own texture and buffer sizes rather than egui's 8192 px cap, so large
/// documents stay on the GPU compositor.
pub fn wgpu_setup() -> egui_wgpu::WgpuSetup {
    let mut setup = egui_wgpu::WgpuSetup::without_display_handle();
    use_adapter_limits(&mut setup);
    setup
}

/// Make `setup` request the adapter's own texture and buffer size limits instead of egui's
/// default 8192 px cap (which sent every document wider or taller than 8192 px to the CPU
/// compositor). Other limits use egui's defaults (WebGL2's on GL), clamped to adapter support;
/// features stay egui's default.
pub fn use_adapter_limits(setup: &mut egui_wgpu::WgpuSetup) {
    use_adapter_limits_with(setup, |_| {});
}

/// [`use_adapter_limits`], calling `on_adapter` with the chosen adapter just before the device
/// is requested (the desktop app records it in its crash-safe startup marker).
pub fn use_adapter_limits_with(setup: &mut egui_wgpu::WgpuSetup, on_adapter: impl Fn(&wgpu::Adapter) + Send + Sync + 'static) {
    if let egui_wgpu::WgpuSetup::CreateNew(create) = setup {
        create.device_descriptor = std::sync::Arc::new(move |adapter| {
            on_adapter(adapter);
            device_descriptor(adapter)
        });
    }
}

/// How the app asks for its device: egui's defaults, the adapter's texture and buffer size
/// limits ([`device_limits`]), and timestamp queries where the adapter has them, so the canvas
/// can report how long the GPU actually spends compositing ([`photocraft_gpu::GpuTimer`]).
/// A bench or test that builds its own device should use this rather than [`device_limits`]
/// alone, or it measures a device the app never creates. (`bevel_bench`, `fx_bench` and
/// `interactive_bench` still call [`device_limits`] directly and so report no GPU time.)
pub fn device_descriptor(adapter: &wgpu::Adapter) -> wgpu::DeviceDescriptor<'static> {
    wgpu::DeviceDescriptor {
        label: Some("photocraft wgpu device"),
        required_limits: device_limits(adapter),
        required_features: timing_features(adapter),
        ..Default::default()
    }
}

/// The timestamp-query features when the adapter supports all of them, else none: asking for a
/// feature an adapter lacks fails device creation, and a canvas without GPU timing is far better
/// than no canvas.
///
/// `PHOTOCRAFT_NO_GPU_TIMING` suppresses the request as well as the timer
/// ([`photocraft_gpu::Compositor::measure_gpu_time`]), so the variable also rescues a driver
/// that advertises the features and then fails or misbehaves on device creation — not only one
/// whose timestamps come back wrong.
pub fn timing_features(adapter: &wgpu::Adapter) -> wgpu::Features {
    let want = photocraft_gpu::timing::FEATURES;
    if std::env::var_os("PHOTOCRAFT_NO_GPU_TIMING").is_some() {
        return wgpu::Features::empty();
    }
    if adapter.features().contains(want) { want } else { wgpu::Features::empty() }
}

/// egui's base limits (WebGL2's on GL), clamped to adapter support, with its maximum texture and buffer sizes.
pub fn device_limits(adapter: &wgpu::Adapter) -> wgpu::Limits {
    device_limits_for(adapter.get_info().backend, adapter.limits())
}

pub(super) fn device_limits_for(backend: wgpu::Backend, supported: wgpu::Limits) -> wgpu::Limits {
    let base = if backend == wgpu::Backend::Gl { wgpu::Limits::downlevel_webgl2_defaults() } else { wgpu::Limits::default() };
    let base = base.or_worse_values_from(&supported);
    wgpu::Limits { max_texture_dimension_2d: supported.max_texture_dimension_2d, max_buffer_size: supported.max_buffer_size, ..base }
}
