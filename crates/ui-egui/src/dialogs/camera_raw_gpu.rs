//! Camera Raw's preview develop on the GPU (#6916). When every stage the settings use reads
//! only its own pixel, the proxy develops in one wgpu pass (`photocraft_gpu::CameraRawDevelop`)
//! on the canvas's device and is read back for the histogram, scopes and display conversion.
//! Spatial stages, Rendering Mode CPU, `PHOTOCRAFT_CPU_CAMERA_RAW`, no GPU canvas, a software
//! adapter, a device fault or a failed develop all keep the CPU develop on the worker.
//!
//! The pipeline builds on a thread the first time a dialog opens (DX12's shader compiler takes
//! about 0.35 s, #6419); frames develop on the CPU until it is ready.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use eframe::wgpu;
use photocraft_algo::camera_raw::CameraRaw;
use photocraft_gpu::{CameraRawDevelop, DeviceHealth};

use crate::PhotocraftApp;

/// The built pass, shared by every Camera Raw dialog on the device (never built on the web).
#[derive(Clone)]
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
pub(crate) struct Pass(Arc<Mutex<CameraRawDevelop>>);

/// A device's pass: unset while it builds, `None` when the device can't run it.
pub(crate) type Slot = Arc<OnceLock<Option<Pass>>>;

impl Pass {
    /// Whether `adapter` may run the pass: not a software adapter (the CPU develop is faster
    /// there), one that renders `Rgba32Float`, and not on the web (no blocking readback yet).
    fn usable(adapter: &wgpu::Adapter) -> bool {
        if cfg!(target_arch = "wasm32") {
            return false;
        }
        if adapter.get_info().device_type == wgpu::DeviceType::Cpu {
            log::info!("Camera Raw develops on the CPU: software graphics adapter");
            return false;
        }
        if !CameraRawDevelop::supported(adapter) {
            log::info!("Camera Raw develops on the CPU: the adapter can't render Rgba32Float");
            return false;
        }
        true
    }

    /// Builds the pass into `slot`, on a thread where there are threads.
    pub(crate) fn start(adapter: &wgpu::Adapter, device: &wgpu::Device, slot: &Slot) {
        if !Self::usable(adapter) {
            let _ = (slot.set(None), device);
            return;
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let (on, out) = (device.clone(), slot.clone());
            let spawned = std::thread::Builder::new().name("camera-raw-pipeline".into()).spawn(move || {
                let pass = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| Self::build_on(&on))).unwrap_or_else(|_| {
                    log::warn!("building the Camera Raw pipeline panicked; Camera Raw develops on the CPU");
                    None
                });
                let _ = out.set(pass);
            });
            if spawned.is_err() {
                let _ = slot.set(Self::build_on(device));
            }
        }
    }

    /// The pass on `device`, whatever its adapter.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    pub(crate) fn build_on(device: &wgpu::Device) -> Option<Self> {
        match CameraRawDevelop::new(device) {
            Ok(pass) => Some(Pass(Arc::new(Mutex::new(pass)))),
            Err(e) => {
                log::warn!("{e}; Camera Raw develops on the CPU");
                None
            }
        }
    }
}

/// The dialog's GPU develop, if the app may use one.
pub(crate) fn for_app(app: &PhotocraftApp) -> Option<CameraRawGpu> {
    let cpu = app.session.prefs().performance.effective_rendering_mode() == photocraft_engine::prefs::RenderingMode::Cpu;
    if cpu || std::env::var_os("PHOTOCRAFT_CPU_CAMERA_RAW").is_some() {
        return None;
    }
    app.gpu.as_ref()?.camera_raw()
}

/// A key for a dialog's proxy pixels, unique in the process: the pass keeps the last source it
/// uploaded, and a new dialog's proxy may reuse a closed one's memory.
pub(crate) fn source_key() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

/// The source key and the settings the black and white points depend on.
type PointsKey = (u64, [u32; 5]);
/// The last [`PointsKey`] and its points.
type Points = Option<(PointsKey, Option<(f32, f32)>)>;

/// One dialog's handle on the pass (cheap to clone into each proxy job).
#[derive(Clone)]
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
pub(crate) struct CameraRawGpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
    health: DeviceHealth,
    pass: Slot,
    /// Set after a develop failed: the dialog stays on the CPU.
    failed: Arc<AtomicBool>,
    /// Whites and Blacks adapt to the image; its points change only with these settings.
    points: Arc<Mutex<Points>>,
}

impl CameraRawGpu {
    pub(crate) fn new(device: wgpu::Device, queue: wgpu::Queue, health: DeviceHealth, pass: Slot) -> Self {
        CameraRawGpu { device, queue, health, pass, failed: Arc::default(), points: Arc::default() }
    }

    /// Develops `px` (`w × h`) with `p`, or `None` when the CPU must: a spatial stage, the pass
    /// still building, a fault, or a GPU error (logged; the dialog then stays on the CPU). `key` names the pixels when they
    /// are the dialog's unchanged proxy ([`source_key`]), so they upload once. Neutral settings
    /// are `None` too: the CPU develop returns them at once, a GPU round trip would only cost.
    pub(crate) fn develop(&self, px: &[[f32; 4]], key: Option<u64>, w: usize, h: usize, p: &CameraRaw, float: bool) -> Option<Vec<[f32; 4]>> {
        if p.spatial() || p.is_identity() || self.failed.load(Ordering::Relaxed) || !self.health.is_ok() {
            return None;
        }
        #[cfg(target_arch = "wasm32")]
        {
            let _ = (px, key, w, h, float);
            None
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let Some(Some(pass)) = self.pass.get() else { return None };
            let points = self.points(px, key, w, h, p);
            let stages = photocraft_algo::camera_raw::per_pixel(p, w, h, float, points)?;
            let size = [u32::try_from(w).ok()?, u32::try_from(h).ok()?];
            let mut pass = pass.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            match pass.develop(&self.device, &self.queue, Some(&self.health), px, key, size, &stages) {
                Ok(out) => Some(out),
                Err(e) => {
                    log::warn!("{}; Camera Raw develops on the CPU", e.0);
                    self.failed.store(true, Ordering::Relaxed);
                    None
                }
            }
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn points(&self, px: &[[f32; 4]], key: Option<u64>, w: usize, h: usize, p: &CameraRaw) -> Option<(f32, f32)> {
        if p.whites == 0.0 && p.blacks == 0.0 {
            return None;
        }
        let Some(key) = key else { return photocraft_algo::camera_raw::white_black_points(px, w, h, p) };
        let key = (key, [p.temperature, p.tint, p.exposure, p.contrast, p.encoding_gamma].map(f32::to_bits));
        let mut cache = self.points.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some((k, points)) = *cache
            && k == key
        {
            return points;
        }
        let points = photocraft_algo::camera_raw::white_black_points(px, w, h, p);
        *cache = Some((key, points));
        points
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_algo::camera_raw::develop;
    use serde_json::json;

    /// The pass evaluates develop's expressions in f32 (see `photocraft-gpu`'s Camera Raw tests).
    const TOL: f32 = 1.0 / 4096.0;

    /// The app on egui_kittest's device (a software adapter where there is one), or `None`.
    fn gpu_app(depth: u32) -> Option<(PhotocraftApp, egui::Context)> {
        let Ok(rs) = std::panic::catch_unwind(|| egui_kittest::wgpu::create_render_state(crate::gpu_canvas::wgpu_setup(), Default::default())) else {
            eprintln!("skipping: no GPU adapter");
            return None;
        };
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.set_wgpu(rs);
        app.gpu.as_ref()?;
        app.run("file.new", json!({"width": 300, "height": 200, "depth": depth})).unwrap();
        app.run("layer.new.layer", json!({})).unwrap();
        app.run("filter.render.clouds", json!({"seed": 5})).unwrap();
        app.run("select.rect", json!({"x": 40, "y": 30, "width": 120, "height": 90})).unwrap();
        app.run("edit.fill", json!({"color": "#cc4422"})).unwrap();
        app.run("select.deselect", json!({})).unwrap();
        Some((app, egui::Context::default()))
    }

    /// The canvas's pass, built even on a software adapter (which the app leaves on the CPU).
    fn force_pass(app: &PhotocraftApp) {
        app.gpu.as_ref().expect("GPU canvas").force_camera_raw_pass();
    }

    fn developed(app: &mut PhotocraftApp, ctx: &egui::Context, p: CameraRaw) -> (&'static str, Vec<[f32; 4]>, Vec<[f32; 4]>) {
        let d = app.camera_raw.as_mut().expect("dialog");
        d.params = p;
        d.render(ctx);
        let mut cpu = d.proxy.to_vec();
        let mut q = d.params.clone();
        q.pixel_scale = (d.pw as f32 / d.full_w.max(1) as f32).min(1.0);
        develop(&mut cpu, d.pw, d.ph, &q, d.float);
        (d.develop_backend, d.processed.clone(), cpu)
    }

    #[test]
    fn the_pass_is_built_when_camera_raw_first_opens_not_with_the_canvas() {
        let _gpu = photocraft_testkit::gpu_lock();
        let Some((mut app, ctx)) = gpu_app(8) else { return };
        let canvas = app.gpu.clone().expect("GPU canvas");
        assert!(!canvas.camera_raw_started(), "no Camera Raw pipeline at launch (#6419)");
        crate::camera_raw_ui::open(&mut app, &ctx).unwrap();
        assert!(canvas.camera_raw_started(), "built (or refused) from the first open");
        assert_eq!(app.camera_raw.as_ref().map(|d| d.develop_backend), Some("cpu"), "the first frame doesn't wait for the pipeline");
        canvas.wait_camera_raw_pass();
        let software = canvas.adapter_info().device_type == wgpu::DeviceType::Cpu;
        let d = app.camera_raw.as_ref().expect("dialog");
        assert_eq!(d.gpu.is_none(), software, "a software adapter keeps the CPU develop");
    }

    #[test]
    fn per_pixel_settings_develop_on_the_gpu_like_the_cpu_and_the_rest_stays_on_the_cpu() {
        let _gpu = photocraft_testkit::gpu_lock();
        for depth in [8, 16, 32] {
            let Some((mut app, ctx)) = gpu_app(depth) else { return };
            force_pass(&app);
            crate::camera_raw_ui::open(&mut app, &ctx).unwrap();
            if app.camera_raw.as_ref().is_some_and(|d| d.gpu.is_none()) {
                eprintln!("skipping: the device can't run the Camera Raw pass");
                return;
            }
            let per_pixel = CameraRaw {
                temperature: 20.0,
                exposure: 0.4,
                contrast: 30.0,
                whites: 25.0,
                blacks: -20.0,
                vibrance: 40.0,
                curve_lights: 25.0,
                grade_shadows: photocraft_algo::camera_raw::Wheel { hue: 220.0, sat: 40.0, lum: 0.0 },
                vignette_amount: -40.0,
                grain_amount: 30.0,
                ..Default::default()
            };
            let (backend, gpu, cpu) = developed(&mut app, &ctx, per_pixel.clone());
            assert_eq!(backend, "gpu", "{depth}-bit");
            let worst = gpu.iter().zip(&cpu).flat_map(|(a, b)| (0..4).map(move |c| (a[c] - b[c]).abs())).fold(0.0f32, f32::max);
            assert!(worst <= TOL, "{depth}-bit: off by {worst}");
            let described = app.camera_raw.as_ref().map(|d| d.describe(&app.ui.camera_raw_scope, &app.ui.camera_raw_preview)).unwrap();
            assert_eq!(described["developBackend"], "gpu");
            // A spatial stage develops the whole frame on the CPU.
            let (backend, shown, cpu) = developed(&mut app, &ctx, CameraRaw { shadows: 40.0, ..per_pixel.clone() });
            assert_eq!((backend, shown == cpu), ("cpu", true), "{depth}-bit");
            // A device fault: the CPU, from the next frame on.
            app.gpu.as_ref().unwrap().health().mark(photocraft_gpu::Fault::Lost("test".into()));
            let (backend, shown, cpu) = developed(&mut app, &ctx, per_pixel);
            assert_eq!((backend, shown == cpu), ("cpu", true), "{depth}-bit after a fault");
        }
    }

    #[test]
    fn neutral_settings_skip_the_gpu_round_trip() {
        let _gpu = photocraft_testkit::gpu_lock();
        let Some((mut app, ctx)) = gpu_app(8) else { return };
        force_pass(&app);
        crate::camera_raw_ui::open(&mut app, &ctx).unwrap();
        if app.camera_raw.as_ref().is_some_and(|d| d.gpu.is_none()) {
            eprintln!("skipping: the device can't run the Camera Raw pass");
            return;
        }
        let (backend, _, _) = developed(&mut app, &ctx, CameraRaw { exposure: 0.5, ..Default::default() });
        assert_eq!(backend, "gpu", "a per-pixel setting develops on the GPU");
        let (backend, shown, cpu) = developed(&mut app, &ctx, CameraRaw::default());
        assert_eq!((backend, shown == cpu), ("cpu", true), "default settings: the CPU returns them unchanged");
    }

    #[test]
    fn rendering_mode_cpu_keeps_the_cpu_develop() {
        let _gpu = photocraft_testkit::gpu_lock();
        let Some((mut app, ctx)) = gpu_app(8) else { return };
        force_pass(&app);
        app.run("prefs.set", json!({"path": "performance.renderingMode", "value": "cpu"})).unwrap();
        crate::camera_raw_ui::open(&mut app, &ctx).unwrap();
        let (backend, shown, cpu) = developed(&mut app, &ctx, CameraRaw { exposure: 0.5, ..Default::default() });
        assert_eq!((backend, shown == cpu), ("cpu", true));
    }
}
