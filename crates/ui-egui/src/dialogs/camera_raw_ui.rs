//! Filter › Camera Raw Filter…: a large dialog like Adobe Camera Raw's (preview on the left,
//! edit panels on the right: Light, Color, Effects, Curve, Color Mixer, Color Grading, Detail).
//!
//! The preview runs [`photocraft_algo::camera_raw::develop`] on a CPU proxy of the layer
//! (≤ 900 px, `pixel_scale` keeps pixel radii true to the full image), recomputed when a
//! control changes: on a worker for the dialog's own controls, so a drag never waits for it, and
//! on the UI thread for the control channel, so a reply describes its own settings. Settings
//! whose stages are all per pixel develop on the GPU instead where it can (`camera_raw_gpu`).
//! OK runs `filter.cameraRaw` with the non-default settings, so the result is one history step
//! (or a smart filter on a smart object) and exactly replayable.
//!
//! Control channel: `ui.menu.invoke {"id":"filter.cameraRaw","params":{"ui":{"set":{…},
//! "commit":true | "cancel":true}}}`; the reply describes the dialog. `{"ui":{"saveSettings":true}}`
//! and `{"ui":{"loadSettings":true}}` run Save Settings… / Load Settings… (Camera Raw `.xmp`
//! settings files, see [`photocraft_io::camera_raw_xmp`]) through the file dialog.

use egui::{Align2, Color32, FontId, Pos2, Rect as ERect, Sense, Stroke, TextureHandle, pos2, vec2};
use photocraft_algo::camera_raw::{CameraRaw, Wheel, curve_lut};
use photocraft_algo::histogram::RgbHistogram;

use crate::camera_raw_scope_ui::{MAX_SAMPLERS, ToneZone};

use photocraft_doc::LayerId;
use photocraft_geom::Rect;
use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::theme::Tokens;
use crate::widgets;

mod controls;
mod convert;
mod menu;
mod open;
mod proxy;
mod sidecar;

pub use controls::show;
#[cfg(test)]
use convert::open_pixels;
pub use menu::menu;
#[cfg(test)]
use open::{Coverage, commit_open_raw, document_index, finish_redevelop, open_raw, replace_raw_document};
pub use open::{
    REDEVELOP_JOB, RawOpen, Redevelop, on_redevelop_event, open, open_smart_filter, opened_without_dialog, queue_open_dialog, wants_open_dialog,
    without_develop_note,
};
use open::{Target, non_defaults};
use proxy::{ProxyFrame, ProxyJob};
pub use sidecar::sidecar_path;
#[cfg(test)]
use sidecar::{existing_sidecar_is_ours, read_bounded};

const PROXY_SIDE: usize = 900;
/// Smallest run of proxy pixels one rayon task takes in the per-pixel passes: below it the
/// split costs more than the work.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) const PARALLEL_MIN_PIXELS: usize = 16 * 1024;
const PANEL_W: f32 = 330.0;
const BANDS: [&str; 8] = ["Red", "Orange", "Yellow", "Green", "Aqua", "Blue", "Purple", "Magenta"];

pub struct CameraRawDialog {
    pub layer: LayerId,
    layer_name: String,
    pub params: CameraRaw,
    /// Settings when the dialog opened (stored settings of an edited smart filter).
    original: CameraRaw,
    target: Target,
    curve_state: crate::state::PointCurveState,
    curve_rect: Option<ERect>,
    /// Proxy pixels (straight RGBA) and size.
    pub(crate) proxy: std::sync::Arc<Vec<[f32; 4]>>,
    pub(crate) selection: Option<std::sync::Arc<Vec<f32>>>,
    pub(crate) pw: usize,
    pub(crate) ph: usize,
    pub(crate) full_w: usize,
    full_h: usize,
    preview_ppp: f32,
    pub(crate) detail: crate::camera_raw_detail_ui::DetailPreview,
    display: std::sync::Arc<photocraft_engine::display_color::CanvasDisplay>,
    pub(crate) viewport: Option<ERect>,
    pub(crate) float: bool,
    tex: Option<TextureHandle>,
    before_tex: Option<TextureHandle>,
    before_display_key: Option<u64>,
    pub(crate) before_histogram: RgbHistogram,
    pub(crate) processed: Vec<[f32; 4]>,
    pub(crate) lab_transform: std::sync::Arc<photocraft_cms::Transform>,
    pub(crate) scope_transform: std::sync::Arc<photocraft_cms::Transform>,
    pub(crate) scope: crate::camera_raw_scope_ui::ScopeView,
    pub(crate) after_histogram: RgbHistogram,
    pub(crate) preview_revision: u64,
    histogram_ms: f64,
    pub(crate) dirty: bool,
    pub show_before: bool,
    mixer_tab: usize,
    pub render_ms: f64,
    /// Set for the open-time dialog ([`Target::OpenRaw`]).
    raw_open: Option<RawOpen>,
    /// The open-time dialog's camera tone curve: Shadows / Highlights act on the raw data there,
    /// in linear light before this curve, and the preview undoes it to show the same.
    raw_curve: Option<std::sync::Arc<Vec<[f32; 2]>>>,
    /// The proxy develop running on a worker ([`Self::request_render`]).
    pending: Option<std::sync::mpsc::Receiver<Result<ProxyFrame, String>>>,
    /// The last worker failure the status bar shows, with the preview revision it failed on.
    preview_error: Option<(String, u64)>,
    /// The GPU develop, when the app may use one.
    pub(crate) gpu: Option<crate::camera_raw_gpu::CameraRawGpu>,
    /// Names [`Self::proxy`] for the GPU pass's upload cache.
    source_key: u64,
    /// The last frame's develop alone, and where it ran ("gpu" or "cpu").
    develop_ms: f64,
    pub(crate) develop_backend: &'static str,
    /// Settings before each finished edit, so Ctrl+Z works inside the dialog (#4100).
    undo: Vec<CameraRaw>,
    redo: Vec<CameraRaw>,
    /// Params at pointer-down. One drag is one undo step.
    edit_origin: Option<CameraRaw>,
}

impl CameraRawDialog {
    pub fn describe(&self, view: &crate::state::CameraRawScopeState, navigation: &crate::state::CameraRawPreviewState) -> Value {
        let h = self.histogram();
        let mut result = json!({"layer": self.layer.0, "params": serde_json::to_value(&self.params).unwrap_or(Value::Null), "proxy": [self.pw, self.ph],
            "renderMs": self.render_ms, "developMs": self.develop_ms, "developBackend": self.develop_backend, "histogramMs": self.histogram_ms, "previewRevision": self.preview_revision, "before": self.show_before,
            "histogram": {"source": if self.show_before { "before" } else { "after" }, "approximate": true, "size": [self.pw, self.ph],
                "sampleDomain": "preview-rgb", "range": [0, 1], "red": h.channels[0].as_slice(), "green": h.channels[1].as_slice(), "blue": h.channels[2].as_slice(),
                "samples": h.samples, "transparent": h.transparent, "invalid": h.invalid, "underflow": h.underflow, "overflow": h.overflow, "shadows": h.shadows, "highlights": h.highlights}});
        result["smartFilter"] = json!(match self.target {
            Target::SmartFilter(index) => Some(index),
            Target::NewFilter | Target::OpenRaw => None,
        });
        result["openingRaw"] = json!(self.raw_open.as_ref().map(|r| json!({"name": r.name, "path": r.path, "demosaic": r.demosaic.id()})));
        result["view"] = json!(navigation);
        result["sourceSize"] = json!([self.full_w, self.full_h]);
        result["previewPending"] = json!(self.render_pending());
        result["previewApproximate"] = json!(!self.detail.ready && (self.pw != self.full_w || self.ph != self.full_h));
        result["detailPending"] = json!(self.detail.pending());
        result["detailError"] = json!(self.detail.error);
        result["viewportRect"] = json!(self.viewport.map(|r| [r.left(), r.top(), r.right(), r.bottom()]));
        result["zoom"] = json!(self.viewport.map(|r| navigation.scale(r, vec2(self.full_w as f32, self.full_h as f32) / self.preview_ppp)));
        result["scope"] = serde_json::to_value(view).unwrap_or(Value::Null);
        result["hasScopeSelection"] = json!(self.selection.is_some());
        result["vectorscope"] = json!(
            self.scope
                .cache
                .as_ref()
                .map(|(_, before, selected, data)| json!({"samples":data.samples,"before":before,"selectedRegion":selected,"bins":data.counts}))
        );
        result["scopeMs"] = json!(self.scope.ms);
        result["scopeRevision"] = json!(self.scope.revision);
        result["overlayRevision"] = json!(self.scope.overlay_key.map(|k| k.0));
        result["pointerReadout"] = self.scope.pointer_sample.and_then(|p| self.readout(p, view.lab)).unwrap_or(Value::Null);
        result["samplerReadouts"] = json!(view.samplers.iter().take(MAX_SAMPLERS).map(|p| self.readout(*p, view.lab)).collect::<Vec<_>>());
        result["hoveredZone"] = json!(self.scope.hovered_zone);
        result["previewRect"] = json!(self.scope.preview_rect.map(|r| [r.left(), r.top(), r.right(), r.bottom()]));
        result["curveState"] = json!(self.curve_state);
        result["curveRect"] = json!(self.curve_rect.map(|r| [r.left(), r.top(), r.right(), r.bottom()]));
        result["hoverSample"] = json!(self.hover_sample(view.vectorscope));
        result["vectorscopeRect"] = json!(self.scope.vectorscope_rect.map(|r| [r.left(), r.top(), r.right(), r.bottom()]));
        result["scopeRect"] = json!(self.scope.rect.map(|r| [r.left(), r.top(), r.right(), r.bottom()]));
        result
    }

    pub(crate) fn histogram(&self) -> &RgbHistogram {
        if self.show_before { &self.before_histogram } else { &self.after_histogram }
    }

    /// Params for the engine: only the settings that differ from the defaults.
    pub fn command_params(&self) -> Value {
        non_defaults(&self.params)
    }

    fn image(px: &[[f32; 4]], w: usize, h: usize, display: &photocraft_engine::display_color::CanvasDisplay) -> egui::ColorImage {
        // Bands copied straight from `px`, converted in parallel (#4682), not a whole copy of the
        // proxy converted on one thread.
        crate::camera_raw_detail_ui::display_image(Rect::new(0, 0, w as i32, h as i32), display, |r, out| {
            let start = (r.y0.max(0) as usize).saturating_mul(w);
            if let Some(src) = px.get(start..start.saturating_add(out.len())) {
                out.copy_from_slice(src);
            }
        })
    }

    /// Develops the proxy with the current settings and shows it, on this thread. The control
    /// channel and the first frame use this, so a reply always describes its own settings; the
    /// dialog's own controls go through [`Self::request_render`].
    pub(crate) fn render(&mut self, ctx: &egui::Context) {
        // A render still running on the worker has older settings than this one.
        self.pending = None;
        let frame = self.proxy_job().run();
        self.publish(ctx, frame);
        self.dirty = false;
    }

    fn proxy_job(&self) -> ProxyJob {
        ProxyJob {
            proxy: self.proxy.clone(),
            selection: self.selection.clone(),
            raw_curve: self.raw_curve.clone(),
            params: self.params.clone(),
            float: self.float,
            size: (self.pw, self.ph),
            full_w: self.full_w,
            display: self.display.clone(),
            gpu: self.gpu.clone(),
            source_key: self.source_key,
        }
    }

    fn publish(&mut self, ctx: &egui::Context, frame: ProxyFrame) {
        match &mut self.tex {
            Some(t) => t.set(frame.image, egui::TextureOptions::LINEAR),
            None => self.tex = Some(ctx.load_texture("camera-raw-after", frame.image, egui::TextureOptions::LINEAR)),
        }
        if self.before_display_key != Some(frame.display.key) {
            let before = Self::image(&self.proxy, self.pw, self.ph, &frame.display);
            match &mut self.before_tex {
                Some(t) => t.set(before, egui::TextureOptions::LINEAR),
                None => self.before_tex = Some(ctx.load_texture("camera-raw-before", before, egui::TextureOptions::LINEAR)),
            }
            self.before_display_key = Some(frame.display.key);
        }
        self.after_histogram = frame.histogram;
        self.histogram_ms = frame.histogram_ms;
        self.processed = frame.px;
        self.scope.cache = None;
        self.render_ms = frame.ms;
        self.develop_ms = frame.develop_ms;
        self.develop_backend = frame.backend;
        self.preview_revision = self.preview_revision.saturating_add(1);
    }

    /// Starts developing the proxy with the current settings on a worker, unless one is already
    /// running: then [`Self::poll_render`] starts the next one with the settings of that moment.
    /// A slider drag never waits for the develop, which takes hundreds of milliseconds with
    /// Shadows or Highlights set (#5964); the preview shows the latest finished frame.
    fn request_render(&mut self, ctx: &egui::Context) {
        if !self.dirty || self.pending.is_some() {
            return;
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let job = self.proxy_job();
            let (tx, rx) = std::sync::mpsc::channel::<Result<ProxyFrame, String>>();
            let repaint = ctx.clone();
            let spawned = std::thread::Builder::new().name("camera-raw-proxy".into()).spawn(move || {
                let frame = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| job.run())).map_err(|_| "Camera Raw preview failed".to_string());
                // The dialog may have closed or rendered on its own meanwhile; then nobody listens.
                if tx.send(frame).is_ok() {
                    repaint.request_repaint();
                }
            });
            match spawned {
                Ok(_) => {
                    self.pending = Some(rx);
                    self.dirty = false;
                }
                // No thread to spare: develop here rather than show stale settings.
                Err(_) => self.render(ctx),
            }
        }
        #[cfg(target_arch = "wasm32")]
        self.render(ctx);
    }

    /// Shows the worker's frame once it is done. `Err` when the develop failed; the preview keeps
    /// the last frame then.
    fn poll_render(&mut self, ctx: &egui::Context) -> Result<(), String> {
        let Some(rx) = &self.pending else { return Ok(()) };
        let frame = match rx.try_recv() {
            Ok(frame) => frame,
            Err(std::sync::mpsc::TryRecvError::Empty) => return Ok(()),
            Err(std::sync::mpsc::TryRecvError::Disconnected) => Err("Camera Raw preview worker stopped".to_string()),
        };
        self.pending = None;
        self.publish(ctx, frame?);
        Ok(())
    }

    pub(crate) fn render_pending(&self) -> bool {
        self.pending.is_some()
    }

    /// A newer frame means the worker failure is over: takes its message down, unless something
    /// else has reported since.
    fn clear_recovered_error(&mut self, ui: &mut crate::UiState) {
        let Some((e, revision)) = &self.preview_error else { return };
        if *revision == self.preview_revision {
            return;
        }
        if ui.status_error && ui.status == *e {
            ui.status.clear();
            ui.status_error = false;
        }
        self.preview_error = None;
    }
}

// The shared Curve editor must never produce a curve `CameraRaw::validate` rejects.
const _: () = assert!(crate::point_curve::MAX_POINTS == photocraft_algo::camera_raw::MAX_CURVE_POINTS && crate::point_curve::MIN_GAP >= 1.0);

#[cfg(test)]
mod tests;

#[cfg(test)]
mod identity_tests;
