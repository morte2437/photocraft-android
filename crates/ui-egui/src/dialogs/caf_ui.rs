//! Edit › Content-Aware Fill…: Photoshop's Content-Aware Fill workspace, a full-window dialog
//! like Liquify's.
//!
//! Measured on Photoshop 25.4 (black box) and laid out the same way: the tools on the left
//! (Sampling Brush, Lasso, Hand, Zoom) with their options bar above, the document in the middle
//! with the sampling area tinted (green, 50 %) and the fill area outlined, a Preview panel, and
//! the Content-Aware Fill panel (Sampling Area Overlay, Sampling Area Options, Fill Settings,
//! Output Settings) with OK, Apply and Cancel.
//!
//! * **Sampling Brush**: a hard round brush. In Auto and Rectangular it subtracts from the
//!   sampling area, in Custom (which starts empty) it adds; Alt reverses. Painting keeps the
//!   sampling option, and added strokes may reach past the sampling window. Its starting size is
//!   a tenth of the window's side (20 px for a 50 px selection).
//! * **Lasso**: edits the fill area itself (New, Add, Subtract, Intersect; Expand and Contract by
//!   a few pixels). The window and the Auto area follow the new fill area; brush strokes stay.
//! * **Preview**: the fill computed in the background after each change, opening framed on the
//!   sampling window.
//! * **Apply** fills (one history step) and keeps the workspace open; **OK** fills and closes;
//!   **Cancel** closes, keeping what was applied. Ctrl+Z undoes the last stroke or lasso edit.
//!
//! What the workspace holds is plain data (the sampling option, brush strokes, lasso edits and
//! fill settings); OK and Apply run `edit.contentAwareFill` with it, and the preview runs the same
//! plan ([`photocraft_engine::edit_menu_cmds::content_aware_plan`]), so the preview is exactly
//! what gets written. The settings are remembered for the next time (`prefs.dialogs`).

use egui::{Align2, Color32, FontId, Pos2, Rect as ERect, Sense, Stroke, TextureHandle, pos2, vec2};
use photocraft_algo::content_aware::{SamplingStroke, paint_strokes};
use photocraft_doc::LayerId;
use photocraft_engine::edit_menu_cmds::{MAX_CAF_EDITS, MAX_CAF_POINTS};
use photocraft_geom::Rect;
use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::canvas::ToolEvent;
use crate::theme::Tokens;
use crate::widgets;

mod controls;
mod lifecycle;
mod preview;
mod view;

pub use controls::{control, keys, pointer};
pub use lifecycle::{apply, cancel, commit, open};
#[cfg(test)]
use preview::plan_params;
use preview::{Base, Preview, outline, points_limit};
pub use view::show;
#[cfg(test)]
use view::view_image;

/// Longest side of the document proxy shown in the view (px).
const PROXY_SIDE: usize = 1600;
const LEFT_W: f32 = 48.0;
const BAR_H: f32 = 36.0;
const PANEL_W: f32 = 300.0;
const PREVIEW_W: f32 = 380.0;
const UNDO_LIMIT: usize = 100;
/// Outlines with more edges than this are drawn at the proxy's resolution.
const MAX_OUTLINE_EDGES: usize = 200_000;
/// Photoshop's overlay colour as it shows (a 50 % tint of grey 200 reads 171, 204, 153).
const OVERLAY_COLOR: [u8; 3] = [142, 208, 106];
/// The `prefs.dialogs` key under which the settings are remembered.
const REMEMBERED: &str = "edit.contentAwareFill.workspace";
const CMD: &str = "edit.contentAwareFill";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CafTool {
    Brush,
    Lasso,
    Hand,
    Zoom,
}

impl CafTool {
    fn name(self) -> &'static str {
        match self {
            CafTool::Brush => "brush",
            CafTool::Lasso => "lasso",
            CafTool::Hand => "hand",
            CafTool::Zoom => "zoom",
        }
    }
    fn from_name(s: &str) -> Option<CafTool> {
        Some(match s {
            "brush" => CafTool::Brush,
            "lasso" => CafTool::Lasso,
            "hand" => CafTool::Hand,
            "zoom" => CafTool::Zoom,
            _ => return None,
        })
    }
}

/// The lasso's selection modes, in options-bar order.
const LASSO_OPS: [&str; 4] = ["new", "add", "subtract", "intersect"];

/// The panel's settings (remembered between uses).
#[derive(Clone, Debug, PartialEq)]
pub struct CafOpts {
    pub show_overlay: bool,
    /// Overlay opacity, 0..100.
    pub opacity: f32,
    pub color: [u8; 3],
    /// The overlay marks what is left out (Indicates: Excluded Area) rather than the sampling area.
    pub excluded: bool,
    /// auto | rectangular | custom
    pub sampling: String,
    pub sample_all: bool,
    /// default | none | high | veryHigh
    pub color_adaptation: String,
    /// none | low | medium | high | full
    pub rotation: String,
    pub scale: bool,
    pub mirror: bool,
    /// current | new | duplicate (Photoshop opens on New Layer)
    pub output: String,
}

impl Default for CafOpts {
    fn default() -> Self {
        CafOpts {
            show_overlay: true,
            opacity: 50.0,
            color: OVERLAY_COLOR,
            excluded: false,
            sampling: "auto".into(),
            sample_all: false,
            color_adaptation: "default".into(),
            rotation: "none".into(),
            scale: false,
            mirror: false,
            output: "new".into(),
        }
    }
}

impl CafOpts {
    /// Applies the settings named in `v` (control channel and remembered keys); others stay.
    pub fn apply(&mut self, v: &Value) -> Result<(), String> {
        let flag = |k: &str| v.get(k).and_then(Value::as_bool);
        let text = |k: &str| v.get(k).and_then(Value::as_str);
        for (k, slot) in [
            ("showOverlay", &mut self.show_overlay),
            ("excluded", &mut self.excluded),
            ("sampleAllLayers", &mut self.sample_all),
            ("scale", &mut self.scale),
            ("mirror", &mut self.mirror),
        ] {
            if let Some(b) = flag(k) {
                *slot = b;
            }
        }
        if let Some(n) = v.get("opacity").and_then(Value::as_f64).filter(|n| n.is_finite()) {
            self.opacity = (n as f32).clamp(0.0, 100.0);
        }
        if let Some(c) = v.get("color").and_then(Value::as_array).filter(|c| c.len() == 3) {
            let ch = |i: usize| c[i].as_u64().map(|v| v.min(255) as u8);
            if let (Some(r), Some(g), Some(b)) = (ch(0), ch(1), ch(2)) {
                self.color = [r, g, b];
            }
        }
        let pick = |k: &str, allowed: &[&str]| -> Result<Option<String>, String> {
            match text(k) {
                None => Ok(None),
                Some(s) if allowed.contains(&s) => Ok(Some(s.to_string())),
                Some(s) => Err(format!("bad {k} `{s}` ({})", allowed.join("|"))),
            }
        };
        if let Some(s) = pick("sampling", &["auto", "rectangular", "custom"])? {
            self.sampling = s;
        }
        if let Some(s) = pick("colorAdaptation", &["none", "default", "high", "veryHigh"])? {
            self.color_adaptation = s;
        }
        if let Some(s) = pick("rotationAdaptation", &["none", "low", "medium", "high", "full"])? {
            self.rotation = s;
        }
        if let Some(s) = pick("output", &["current", "new", "duplicate"])? {
            self.output = s;
        }
        Ok(())
    }

    /// The settings as [`CafOpts::apply`] reads them.
    pub fn to_json(&self) -> Value {
        json!({
            "showOverlay": self.show_overlay,
            "opacity": self.opacity,
            "color": self.color,
            "excluded": self.excluded,
            "sampling": self.sampling,
            "sampleAllLayers": self.sample_all,
            "colorAdaptation": self.color_adaptation,
            "rotationAdaptation": self.rotation,
            "scale": self.scale,
            "mirror": self.mirror,
            "output": self.output,
        })
    }
}

/// One undoable step.
#[derive(Clone, Debug)]
enum Step {
    Stroke(SamplingStroke),
    Fill(Value),
    /// A change of the sampling option (or a reset), with what it replaced.
    State {
        sampling: String,
        strokes: Vec<SamplingStroke>,
        fill_edits: Vec<Value>,
    },
}

fn stroke_json(s: &SamplingStroke) -> Value {
    json!({"mode": if s.add { "add" } else { "subtract" }, "size": s.size, "points": s.points})
}

pub struct CafWorkspace {
    pub layer: LayerId,
    layer_name: String,
    canvas: Rect,
    /// The document revision the textures and the base were made from.
    revision: u64,
    pub opts: CafOpts,
    pub tool: CafTool,
    pub brush_add: bool,
    pub brush_size: f64,
    pub lasso_op: usize,
    pub expand_by: f32,
    pub strokes: Vec<SamplingStroke>,
    pub fill_edits: Vec<Value>,
    undo: Vec<Step>,
    redo: Vec<Step>,
    cur: Option<SamplingStroke>,
    /// The lasso polygon being drawn and its selection mode.
    lasso: Option<(usize, Vec<[f64; 2]>)>,
    base: Base,
    /// Fill-area outline edges (document px), or `None` when there are too many.
    outline: Option<Vec<[[f32; 2]; 2]>>,
    /// Document px per proxy px, and the proxy size.
    k: usize,
    pw: usize,
    ph: usize,
    /// Per proxy pixel: the sampling option's area (before the strokes), the fill area, and the
    /// sampling area after the strokes so far (the first painted with them in order, as the
    /// engine's `paint_strokes` paints the full-resolution one).
    base_px: Vec<bool>,
    hole_px: Vec<bool>,
    mask: Vec<bool>,
    /// The pointer is down on the view with a tool: one press is one `Down`, egui's drag (which
    /// starts a few pixels later) continues it.
    pressed: bool,
    /// The device's texture side limit, as the context reports it each frame.
    texture_limit: usize,
    image_tex: Option<TextureHandle>,
    overlay_tex: Option<TextureHandle>,
    overlay_dirty: bool,
    zoom: f32,
    center: [f64; 2],
    hover: Option<[f64; 2]>,
    preview: Preview,
    pub error: Option<String>,
}

impl CafWorkspace {
    pub fn describe(&self) -> Value {
        let sampled = self.mask.iter().zip(&self.hole_px).filter(|(m, h)| **m && !**h).count();
        json!({
            "layer": self.layer.0,
            "tool": self.tool.name(),
            "brushSize": self.brush_size,
            "brushMode": if self.brush_add { "add" } else { "subtract" },
            "lassoMode": LASSO_OPS[self.lasso_op],
            "expandBy": self.expand_by,
            "strokes": self.strokes.len() + usize::from(self.cur.is_some()),
            "fillEdits": self.fill_edits.len(),
            "undo": self.undo.len(),
            "redo": self.redo.len(),
            "window": [self.base.window.x0, self.base.window.y0, self.base.window.width(), self.base.window.height()],
            "fillArea": self.base.hole.iter().filter(|h| **h).count(),
            "sampledProxyPixels": sampled,
            "proxyScale": self.k,
            "settings": self.opts.to_json(),
            "previewReady": self.preview.tex.is_some() && self.preview.shown == self.preview.want,
            "previewError": self.preview.error,
            "error": self.error,
        })
    }

    /// The proxy, in proxy px.
    fn proxy_area(&self) -> Rect {
        Rect::new(0, 0, self.pw as i32, self.ph as i32)
    }

    /// `s` in proxy coordinates: proxy pixel `(px, py)` is centred on document point
    /// `canvas.x0 + (px + 0.5) k`, so the stroke's points and size scale by `1 / k`.
    fn proxy_stroke(&self, s: &SamplingStroke) -> SamplingStroke {
        let k = self.k as f64;
        let (ox, oy) = (f64::from(self.canvas.x0), f64::from(self.canvas.y0));
        SamplingStroke { add: s.add, size: s.size / k, points: s.points.iter().map(|p| [(p[0] - ox) / k, (p[1] - oy) / k]).collect() }
    }

    /// Paints `s` (a stroke, or one segment of the one being drawn) into the proxy's sampling
    /// area: only the pixels within the brush of its path are visited, whatever came before.
    fn paint(&mut self, s: &SamplingStroke) {
        let (s, area) = (self.proxy_stroke(s), self.proxy_area());
        paint_strokes(&mut self.mask, area, &[s]);
        self.overlay_dirty = true;
    }

    /// Takes a new base: its option's area and fill area at the proxy's pixel centres, then the
    /// strokes over it.
    fn set_base(&mut self, b: Base) {
        self.outline = outline(&b);
        let k = self.k as f64;
        for py in 0..self.ph {
            for px in 0..self.pw {
                let (x, y) = (f64::from(self.canvas.x0) + (px as f64 + 0.5) * k, f64::from(self.canvas.y0) + (py as f64 + 0.5) * k);
                let i = b.index(x, y);
                let at = |v: &[bool]| i.is_some_and(|i| v.get(i).copied().unwrap_or(false));
                self.base_px[py * self.pw + px] = at(&b.base);
                self.hole_px[py * self.pw + px] = at(&b.hole);
            }
        }
        self.base = b;
        self.restate_all();
    }

    /// Rebuilds the proxy's sampling area: the base painted with every stroke in order. Each
    /// stroke costs its own pixels, so this is linear in the strokes, not strokes × pixels.
    fn restate_all(&mut self) {
        self.mask.clone_from(&self.base_px);
        let strokes: Vec<SamplingStroke> = self.strokes.iter().chain(self.cur.iter()).map(|s| self.proxy_stroke(s)).collect();
        let area = self.proxy_area();
        paint_strokes(&mut self.mask, area, &strokes);
        self.overlay_dirty = true;
    }

    /// Points in the strokes and lasso edits so far, against the engine's budget.
    fn points_used(&self) -> usize {
        let strokes: usize = self.strokes.iter().chain(self.cur.iter()).map(|s| s.points.len()).sum();
        let lassos: usize = self.fill_edits.iter().map(|e| e.get("points").and_then(Value::as_array).map_or(0, Vec::len)).sum();
        strokes + lassos
    }

    /// The preview is out of date.
    fn stale(&mut self) {
        self.preview.want += 1;
    }

    fn begin_stroke(&mut self, p: [f64; 2], alt: bool) {
        // The engine takes at most MAX_CAF_EDITS strokes and MAX_CAF_POINTS points in all: the
        // brush stops there, so Apply and the preview never fail on what was painted.
        if self.strokes.len() >= MAX_CAF_EDITS {
            self.error = Some(format!("at most {MAX_CAF_EDITS} sampling strokes: undo some, or reset the sampling area"));
            return;
        }
        if self.points_used() >= MAX_CAF_POINTS {
            self.error = Some(points_limit());
            return;
        }
        self.error = None;
        let add = self.brush_add != alt;
        let s = SamplingStroke { add, size: self.brush_size, points: vec![p] };
        self.paint(&s);
        self.cur = Some(s);
    }

    fn extend_stroke(&mut self, p: [f64; 2]) {
        if self.cur.is_none() {
            return;
        }
        if self.points_used() >= MAX_CAF_POINTS {
            self.error = Some(points_limit());
            return;
        }
        let Some(s) = self.cur.as_mut() else { return };
        if s.points.last().is_some_and(|l| l[0] == p[0] && l[1] == p[1]) {
            return;
        }
        let last = *s.points.last().unwrap_or(&p);
        s.points.push(p);
        let seg = SamplingStroke { add: s.add, size: s.size, points: vec![last, p] };
        self.paint(&seg);
    }

    fn end_stroke(&mut self) {
        if let Some(s) = self.cur.take() {
            self.strokes.push(s.clone());
            self.push(Step::Stroke(s));
            self.stale();
        }
    }

    fn push(&mut self, s: Step) {
        self.redo.clear();
        if self.undo.len() == UNDO_LIMIT {
            self.undo.remove(0);
        }
        self.undo.push(s);
    }
}

#[cfg(test)]
mod tests;
