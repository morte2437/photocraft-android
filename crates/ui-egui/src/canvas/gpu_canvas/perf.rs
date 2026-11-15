use super::budget::video_memory;
use super::setup::Refresh;
use super::texels::LayerFit;
use super::wgpu;

/// Milliseconds since the process (or page) started, from a monotonic clock.
///
/// `web_time::Instant` is `std::time::Instant` natively and `performance.now()` on wasm, so frame
/// caps, autosave intervals, preference retry backoff, stroke traces and screenshot deadlines all
/// measure durations the same way on both. The old wasm value was a constant 0, which made every
/// interval comparison (`now >= deadline`) permanently false on the web.
pub fn now_ms() -> f64 {
    static START: std::sync::OnceLock<web_time::Instant> = std::sync::OnceLock::new();
    START.get_or_init(web_time::Instant::now).elapsed().as_secs_f64() * 1000.0
}

/// UI-thread timings (CPU side), exponentially smoothed.
#[derive(Clone, Debug, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Perf {
    /// Average CPU time building the UI per frame (ms).
    pub ui_ms: f64,
    /// Worst UI frame of the last ~second (ms).
    pub ui_max_ms: f64,
    /// Last canvas refresh, CPU side (ms): the CPU composite on the CPU path, and on the GPU
    /// path the cost of *encoding and submitting* the composite — not of running it, which is
    /// `gpu_ms`.
    pub composite_ms: f64,
    /// Time spent on a failed GPU composite attempt before CPU fallback.
    pub gpu_attempt_ms: f64,
    /// Last canvas refresh: conversion + upload (ms).
    pub upload_ms: f64,
    /// Last canvas refresh kind: "full", "rect" or "none".
    pub last_refresh: &'static str,
    /// Pixels in the last refresh.
    pub last_refresh_px: u64,
    /// Whether the GPU canvas path is active.
    pub gpu: bool,
    /// Layer tiles uploaded by the last GPU composite.
    pub gpu_uploads: u64,
    /// How long the GPU spent executing a recent composite (ms), from timestamp queries read
    /// back after the fact. `None` on the CPU compositor, on a device without timestamp queries
    /// and on wasm — there is then no honest number to report, and `composite_ms` is not one.
    /// Excludes layer-tile uploads and end-of-batch work (see `photocraft_gpu::timing`), so it
    /// is a few per cent under the GPU's full share of a warm refresh and further under a cold
    /// one.
    pub gpu_ms: Option<f64>,
    /// Pixels of the refresh `gpu_ms` measured, which is a frame or two behind
    /// `last_refresh_px` and may be a different size.
    pub gpu_ms_px: u64,
    /// Why the active view is drawn on the CPU (None = GPU composited): the current
    /// [`crate::render_path`] reason as text, written only by `render_path`.
    pub gpu_fallback: Option<String>,
    /// Adapter, backend, driver and fallback state (Help › System Info).
    pub gpu_info: GpuInfo,
    /// GPU memory the compositor may hold for layer pages and effect maps (MB; see
    /// [`memory_budget`]).
    pub gpu_budget_mb: u64,
    /// The Memory Usage allowance and the GPU Memory preference (bytes) `gpu_budget_mb` was
    /// computed from.
    #[serde(skip)]
    pub gpu_budget_allowance: (u64, u64),
    /// The layer pages the last full refresh needed against the part of the budget they may
    /// hold; when they don't fit, edits that redraw the whole document do it on the CPU.
    pub gpu_layer_fit: Option<LayerFit>,
    /// Last command run through the app and its duration (ms).
    pub last_command: String,
    pub command_ms: f64,
    /// Most recent cost of occasional UI work (outline tracing, thumbnails, …), in ms.
    pub spans: std::collections::BTreeMap<&'static str, f64>,
    #[serde(skip)]
    max_decay: f64,
    /// This frame's stages, gathered as it runs (see `frame_stages`).
    #[serde(skip)]
    pub(crate) stage: crate::frame_stages::FrameStages,
}

/// What the app renders with and why (Help › System Info, Preferences › Performance, the `perf`
/// control-channel output).
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GpuInfo {
    /// Adapter name ("" before the window opens).
    pub adapter: String,
    /// wgpu backend of the window's device ("vulkan", "dx12", "metal", "gl", …).
    pub backend: String,
    /// Device type ("discreteGpu", "integratedGpu", "cpu", …).
    pub device_type: String,
    /// Driver name and version as the driver reports them.
    pub driver: String,
    /// The adapter's own memory in MB ([`video_memory`]): discrete GPUs where the platform
    /// reports it. It caps the GPU memory budgets.
    pub video_memory_mb: Option<u64>,
    /// `performance.gpuBackend` at launch.
    pub preference: String,
    /// Every graphics adapter the window could have used, by name (Preferences › Performance ›
    /// Graphics Adapter lists them). Empty in hosts that don't set it.
    pub adapters: Vec<String>,
    /// The backend startup chose ("auto", "dx12", "cpu", …, or "env" for `WGPU_BACKEND`).
    pub selected: String,
    /// Why startup didn't use the preference as is (previous start crashed, `--safe-gpu`,
    /// `WGPU_BACKEND`, Intel on Windows).
    pub fallback: Option<String>,
    /// "gpu" (wgpu canvas and compositor) or "cpu" (CPU compositor, egui textures).
    pub canvas: String,
    /// Why the GPU canvas was dropped during this session (device lost or a GPU error).
    pub lost: Option<String>,
    /// Present mode and frame latency ("fifo, latency 2"; launch overrides marked, see the
    /// app's `presentation`). Empty in hosts that don't set it.
    pub present: String,
}

impl GpuInfo {
    /// Record the adapters the instance found, without duplicates (one GPU shows up once per backend).
    pub fn set_available(&mut self, adapters: &[wgpu::Adapter]) {
        self.adapters.clear();
        for a in adapters {
            let name = a.get_info().name;
            if !name.is_empty() && !self.adapters.contains(&name) {
                self.adapters.push(name);
            }
        }
    }

    /// Fill the adapter fields from `info`.
    pub fn set_adapter(&mut self, info: &wgpu::AdapterInfo) {
        self.adapter = info.name.clone();
        self.backend = backend_name(info.backend).to_string();
        self.device_type = match info.device_type {
            wgpu::DeviceType::DiscreteGpu => "discreteGpu",
            wgpu::DeviceType::IntegratedGpu => "integratedGpu",
            wgpu::DeviceType::VirtualGpu => "virtualGpu",
            wgpu::DeviceType::Cpu => "cpu",
            wgpu::DeviceType::Other => "other",
        }
        .to_string();
        self.driver = [info.driver.as_str(), info.driver_info.as_str()].iter().filter(|s| !s.is_empty()).copied().collect::<Vec<_>>().join(" ");
        self.video_memory_mb = video_memory(info).map(|b| b >> 20);
    }

    /// Human-readable lines for Help › System Info.
    pub fn lines(&self) -> Vec<String> {
        let or = |s: &str, d: &str| if s.is_empty() || (s == "auto" && d == "auto") { tl!(d).to_string() } else { s.to_string() };
        let mut v = vec![
            {
                let template = "Graphics adapter: {adapter}";
                crate::i18n::fmt(tl!(template), &[("adapter", &or(&self.adapter, "unknown"))])
            },
            {
                let template = "Backend: {backend} ({type})";
                crate::i18n::fmt(tl!(template), &[("backend", &or(&self.backend, "unknown")), ("type", &or(&self.device_type, "unknown type"))])
            },
            {
                let template = "Driver: {driver}";
                crate::i18n::fmt(tl!(template), &[("driver", &or(&self.driver, "unknown"))])
            },
        ];
        // The card's own memory caps the GPU budgets, so it explains a huge canvas falling back
        // to the CPU. Only discrete cards report one (integrated GPUs share system memory).
        if let Some(mb) = self.video_memory_mb {
            let template = "Graphics memory: {size}";
            v.push(crate::i18n::fmt(tl!(template), &[("size", &crate::chrome_ui::fmt_bytes(mb.saturating_mul(1 << 20)))]));
        }
        v.extend([
            {
                let template = "GPU backend preference: {backend}";
                crate::i18n::fmt(tl!(template), &[("backend", &or(&self.preference, "auto"))])
            },
            {
                let template = "Selected at launch: {backend}";
                crate::i18n::fmt(tl!(template), &[("backend", &or(&self.selected, "auto"))])
            },
            {
                let template = "Image compositor: {processor}";
                crate::i18n::fmt(tl!(template), &[("processor", if self.canvas == "gpu" { "GPU" } else { "CPU" })])
            },
            {
                let template = "Window renderer: {renderer}";
                let renderer = if self.device_type == "cpu" {
                    "Software adapter"
                } else if self.adapter.is_empty() {
                    "Unknown"
                } else {
                    "Graphics adapter"
                };
                crate::i18n::fmt(tl!(template), &[("renderer", tl!(renderer))])
            },
        ]);
        if let Some(f) = &self.fallback {
            v.push({
                let template = "Fallback: {reason}";
                crate::i18n::fmt(tl!(template), &[("reason", f)])
            });
        }
        if let Some(l) = &self.lost {
            v.push({
                let template = "This session: {reason}";
                crate::i18n::fmt(tl!(template), &[("reason", l)])
            });
        }
        v
    }
}

/// The preference name of a wgpu backend.
pub fn backend_name(b: wgpu::Backend) -> &'static str {
    match b {
        wgpu::Backend::Vulkan => "vulkan",
        wgpu::Backend::Dx12 => "dx12",
        wgpu::Backend::Metal => "metal",
        wgpu::Backend::Gl => "gl",
        wgpu::Backend::BrowserWebGpu => "webgpu",
        wgpu::Backend::Noop => "noop",
    }
}

impl Perf {
    pub fn record(&mut self, kind: &'static str, px: u64, composite_ms: f64, upload_ms: f64) {
        self.stage.refresh(kind, px, composite_ms, upload_ms);
        self.last_refresh = kind;
        self.last_refresh_px = px;
        self.composite_ms = composite_ms;
        self.upload_ms = upload_ms;
        // A refresh that didn't come through `record_refresh` or `record_gpu_attempt` had no failed
        // GPU attempt: the last one's time would be read as this refresh's.
        self.gpu_attempt_ms = 0.0;
        if !kind.starts_with("gpu") {
            self.gpu_uploads = 0;
            // The CPU compositor did this one: there is no GPU time, and the last GPU
            // refresh's would be read as this refresh's.
            self.gpu_ms = None;
            self.gpu_ms_px = 0;
        }
    }

    /// Record a canvas refresh and any GPU measurement that came back with it.
    pub fn record_refresh(&mut self, r: &Refresh) {
        self.record(r.kind, r.px, r.composite_ms, r.upload_ms);
        self.record_gpu_attempt(r.gpu_attempt_ms);
        if let Some(t) = r.gpu {
            self.record_gpu(t);
        }
    }

    /// Record the time a failed GPU composite attempt took before the CPU did the refresh just
    /// recorded, apart from that refresh's own CPU time.
    pub fn record_gpu_attempt(&mut self, ms: f64) {
        self.gpu_attempt_ms = ms;
        self.stage.composite_ms += ms;
    }

    /// Record a GPU composite measurement that came back (see [`photocraft_gpu::GpuTiming`]).
    pub fn record_gpu(&mut self, t: photocraft_gpu::GpuTiming) {
        self.gpu_ms = Some(t.ms);
        self.gpu_ms_px = t.px;
    }
    pub fn span(&mut self, name: &'static str, ms: f64) {
        self.spans.insert(name, (ms * 100.0).round() / 100.0);
    }
    pub fn frame(&mut self, ms: f64) {
        self.ui_ms = if self.ui_ms == 0.0 { ms } else { self.ui_ms * 0.9 + ms * 0.1 };
        self.max_decay += 1.0;
        if ms > self.ui_max_ms || self.max_decay > 60.0 {
            self.ui_max_ms = ms;
            self.max_decay = 0.0;
        }
    }
}
