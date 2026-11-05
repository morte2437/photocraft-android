//! Filter › Liquify…: a full-window dialog like Photoshop's (tool strip on the left, properties on
//! the right, preview in the middle, OK / Cancel).
//!
//! The preview is a CPU proxy: the layer box-downsampled once to at most 1600 px
//! (`ProxyImage`), re-rendered through the displacement field only where a dab changed it
//! (bilinear field and source), and uploaded with a partial texture update. A dab plus its
//! update costs well under a millisecond on a 24 MP layer (see `bench_liquify`). Strokes are
//! recorded exactly as the engine replays them (`stroke_begin` + `stroke_segment` per point), so
//! OK runs `filter.liquify` with the same strokes and gets the same field.

use egui::{Align2, Color32, FontId, Pos2, Rect as ERect, Sense, Stroke, TextureHandle, pos2, vec2};
use photocraft_algo::liquify::{LiquifyField, LiquifyStroke, LiquifyTool, ProxyImage, auto_cell};
use photocraft_doc::{DocId, Document, LayerId};
use photocraft_geom::Rect;
use photocraft_raster::Surface;
use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::canvas::ToolEvent;
use crate::theme::Tokens;
use crate::widgets;

/// Longest side of the preview proxy (px).
const PROXY_SIDE: usize = 1600;
const MAX_VIEW_PREVIEW_PIXELS: usize = 8 * 1024 * 1024;
const LEFT_W: f32 = 48.0;
const RIGHT_W: f32 = 292.0;
const REDO_STACK_LIMIT: usize = 100;

/// Native component sampling keeps the other components and opacity at the destination. Work
/// stays bounded by the dirty proxy/viewport rectangle; a dab never scans the full source.
#[allow(clippy::too_many_arguments)]
fn render_components(
    source: &Surface,
    source_bounds: Rect,
    field: &LiquifyField,
    scale: f64,
    target: &Value,
    bounds: Rect,
    dirty: Rect,
    out: &mut [[u8; 4]],
) -> bool {
    let Ok(Some(colors)) = photocraft_engine::color_channels::explicit(&json!({"target":target}), source.format().mode.color_channels()) else { return false };
    let width = bounds.width() as usize;
    if source_bounds.is_empty() || bounds.is_empty() || !scale.is_finite() || scale <= 0.0 || width.checked_mul(bounds.height() as usize) != Some(out.len()) {
        return false;
    }
    let fmt = source.format();
    let mut native = vec![0.0; fmt.channels()];
    let dirty = dirty.intersect(&bounds);
    for y in dirty.y0..dirty.y1 {
        for x in dirty.x0..dirty.x1 {
            let px = f64::from(field.bounds.x0) + (f64::from(x.saturating_sub(source_bounds.x0)) + 0.5) * scale;
            let py = f64::from(field.bounds.y0) + (f64::from(y.saturating_sub(source_bounds.y0)) + 0.5) * scale;
            let displacement = field.sample(px, py);
            let sx = (f64::from(x) + displacement[0] / scale).clamp(f64::from(source_bounds.x0), f64::from(source_bounds.x1 - 1));
            let sy = (f64::from(y) + displacement[1] / scale).clamp(f64::from(source_bounds.y0), f64::from(source_bounds.y1 - 1));
            let (x0, y0) = (sx.floor() as i32, sy.floor() as i32);
            let (fx, fy) = ((sx - f64::from(x0)) as f32, (sy - f64::from(y0)) as f32);
            let taps = [
                (x0, y0, (1.0 - fx) * (1.0 - fy)),
                (x0.saturating_add(1).min(source_bounds.x1 - 1), y0, fx * (1.0 - fy)),
                (x0, y0.saturating_add(1).min(source_bounds.y1 - 1), (1.0 - fx) * fy),
                (x0.saturating_add(1).min(source_bounds.x1 - 1), y0.saturating_add(1).min(source_bounds.y1 - 1), fx * fy),
            ];
            let alpha_index = fmt.mode.color_channels();
            let alpha = |tx, ty| if fmt.alpha { source.sample_channel(tx, ty, alpha_index) } else { 1.0 };
            let warped_alpha: f32 = taps.iter().map(|(tx, ty, w)| alpha(*tx, *ty) * w).sum();
            for (channel, value) in native.iter_mut().enumerate() {
                *value = if colors.contains(channel) {
                    if warped_alpha > 0.0 {
                        taps.iter().map(|(tx, ty, w)| source.sample_channel(*tx, *ty, channel) * alpha(*tx, *ty) * w).sum::<f32>() / warped_alpha
                    } else {
                        0.0
                    }
                } else {
                    source.sample_channel(x, y, channel)
                };
            }
            let rgba = photocraft_raster::to_rgba(&fmt, &native);
            let a = rgba[3].clamp(0.0, 1.0);
            let encoded = [rgba[0] * a, rgba[1] * a, rgba[2] * a, a].map(|v| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8);
            let index = (y - bounds.y0) as usize * width + (x - bounds.x0) as usize;
            if let Some(pixel) = out.get_mut(index) {
                *pixel = encoded;
            }
        }
    }
    true
}

/// Options shown in the properties panel (and settable over the control channel).
#[derive(Clone, Debug)]
pub struct LiquifyOpts {
    pub tool: LiquifyTool,
    pub size: f32,
    pub density: f32,
    pub pressure: f32,
    pub rate: f32,
    pub show_mesh: bool,
    /// 0 small, 1 medium, 2 large.
    pub mesh_size: usize,
    pub show_mask: bool,
    pub show_backdrop: bool,
    /// Show Backdrop › Use: the layer shown as the backdrop (its id), `None` for All Layers.
    pub backdrop_layer: Option<u64>,
    pub backdrop_mode: BackdropMode,
    pub backdrop_opacity: f32,
    pub reconstruct_amount: f32,
}

/// Show Backdrop › Mode, as Photoshop draws them (#7609): In Front lays the backdrop over the
/// image at the Opacity, Behind lays the image over the backdrop at the Opacity, and Blend fades
/// the image to 100 − Opacity under the backdrop at the Opacity.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BackdropMode {
    #[default]
    InFront,
    Behind,
    Blend,
}

impl BackdropMode {
    const ALL: [(BackdropMode, &str); 3] = [(BackdropMode::InFront, "inFront"), (BackdropMode::Behind, "behind"), (BackdropMode::Blend, "blend")];

    fn key(self) -> &'static str {
        Self::ALL.iter().find(|(m, _)| *m == self).map_or("inFront", |(_, k)| k)
    }
}

impl Default for LiquifyOpts {
    fn default() -> Self {
        LiquifyOpts {
            tool: LiquifyTool::ForwardWarp,
            size: 100.0,
            density: 50.0,
            pressure: 100.0,
            rate: 80.0,
            show_mesh: false,
            mesh_size: 1,
            show_mask: true,
            show_backdrop: false,
            backdrop_layer: None,
            backdrop_mode: BackdropMode::InFront,
            backdrop_opacity: 50.0,
            reconstruct_amount: 100.0,
        }
    }
}

/// The `prefs.dialogs` key under which the brush settings are remembered between uses and
/// launches (#418).
const REMEMBERED: &str = "filter.liquify";

impl LiquifyOpts {
    /// Applies the settings named in `v` (the control channel's and the remembered keys),
    /// clamped to their ranges; the others are left as they are. A bad `tool` is an error, after
    /// everything else has been applied.
    pub fn apply(&mut self, v: &Value) -> Result<(), String> {
        let num = |k: &str| v.get(k).and_then(Value::as_f64).filter(|n| n.is_finite()).map(|n| n as f32);
        let flag = |k: &str| v.get(k).and_then(Value::as_bool);
        if let Some(n) = num("size") {
            self.size = n.clamp(1.0, 15000.0);
        }
        for (k, slot) in
            [("density", &mut self.density), ("pressure", &mut self.pressure), ("rate", &mut self.rate), ("backdropOpacity", &mut self.backdrop_opacity)]
        {
            if let Some(n) = num(k) {
                *slot = n.clamp(0.0, 100.0);
            }
        }
        for (k, slot) in [("showMesh", &mut self.show_mesh), ("showMask", &mut self.show_mask), ("showBackdrop", &mut self.show_backdrop)] {
            if let Some(b) = flag(k) {
                *slot = b;
            }
        }
        // `"all"` (or anything but a layer id) is All Layers.
        if let Some(u) = v.get("backdropUse") {
            self.backdrop_layer = u.as_u64();
        }
        if let Some(m) = v.get("backdropMode").and_then(Value::as_str).and_then(|m| BackdropMode::ALL.iter().find(|(_, k)| *k == m)) {
            self.backdrop_mode = m.0;
        }
        if let Some(m) = v.get("meshSize").and_then(Value::as_str) {
            self.mesh_size = match m {
                "small" => 0,
                "large" => 2,
                _ => 1,
            };
        }
        if let Some(t) = v.get("tool") {
            let tool: LiquifyTool = serde_json::from_value(t.clone()).map_err(|e| format!("bad tool: {e}"))?;
            // Strokes can twirl counterclockwise, but no tool does: hold Alt with Twirl Clockwise.
            if !LiquifyTool::ALL.contains(&tool) {
                return Err(format!("{t} is not a Liquify tool: choose twirlCw and hold Alt (Option) to twirl counterclockwise"));
            }
            self.tool = tool;
        }
        Ok(())
    }

    /// The settings as [`LiquifyOpts::apply`] reads them.
    pub fn to_json(&self) -> Value {
        let mesh = ["small", "medium", "large"].get(self.mesh_size).copied().unwrap_or("medium");
        json!({
            "tool": self.tool,
            "size": self.size,
            "density": self.density,
            "pressure": self.pressure,
            "rate": self.rate,
            "showMesh": self.show_mesh,
            "meshSize": mesh,
            "showMask": self.show_mask,
            "showBackdrop": self.show_backdrop,
            "backdropUse": self.backdrop_layer.map_or(json!("all"), |id| json!(id)),
            "backdropMode": self.backdrop_mode.key(),
            "backdropOpacity": self.backdrop_opacity,
        })
    }
}

pub struct LiquifyDialog {
    /// Document this dialog belongs to (#5115).
    pub document: DocId,
    pub layer: LayerId,
    layer_name: String,
    canvas: Rect,
    cell: f64,
    pub field: LiquifyField,
    pub strokes: Vec<LiquifyStroke>,
    redo: Vec<LiquifyStroke>,
    /// The stroke being drawn and its last point.
    cur: Option<(LiquifyStroke, [f64; 3])>,
    /// The lasso polygon being drawn (document px) and whether it thaws (Alt held at pointer-down).
    lasso: Option<(bool, Vec<[f64; 2]>)>,
    proxy: ProxyImage,
    source: Surface,
    target: Value,
    native_proxy: Option<Surface>,
    out: Vec<[u8; 4]>,
    tex: Option<TextureHandle>,
    /// Proxy pixels to re-render and upload (x0, y0, x1, y1).
    dirty: Option<[usize; 4]>,
    /// Document-pixel preview used above 100% zoom.
    view_bounds: Option<Rect>,
    view_out: Vec<[u8; 4]>,
    view_source_scratch: Vec<[f32; 4]>,
    view_tex: Option<TextureHandle>,
    view_dirty: Option<Rect>,
    /// The backdrop and the Use it shows (see [`LiquifyOpts::backdrop_layer`]).
    backdrop: Option<(Option<u64>, TextureHandle)>,
    mask_tex: Option<TextureHandle>,
    mask_dirty: bool,
    pub opts: LiquifyOpts,
    /// Screen px per document px (0 = fit on next frame) and the view centre (document px).
    zoom: f32,
    center: [f64; 2],
    hover: Option<[f64; 2]>,
    /// Alt+right-drag (or Control+Alt-drag) resizing the brush: screen anchor, starting size, starting density.
    brush_resize: Option<(Pos2, f32, f32)>,
    last_dab: f64,
    /// Last dab + proxy update time (ms), shown in the footer.
    pub dab_ms: f64,
    /// CPU time spent refreshing the zoomed-in source-resolution preview.
    pub preview_ms: f64,
    /// The smart filter being edited: OK updates it instead of adding a new Liquify filter.
    smart_filter: Option<usize>,
}

fn union(a: Option<[usize; 4]>, b: [usize; 4]) -> Option<[usize; 4]> {
    if b[0] >= b[2] || b[1] >= b[3] {
        return a;
    }
    Some(match a {
        Some(a) => [a[0].min(b[0]), a[1].min(b[1]), a[2].max(b[2]), a[3].max(b[3])],
        None => b,
    })
}

fn union_rect(a: Option<Rect>, b: Rect) -> Option<Rect> {
    if b.is_empty() {
        return a;
    }
    Some(match a {
        Some(a) => Rect::new(a.x0.min(b.x0), a.y0.min(b.y0), a.x1.max(b.x1), a.y1.max(b.y1)),
        None => b,
    })
}

impl LiquifyDialog {
    pub fn describe(&self) -> Value {
        json!({
            "document": self.document.0,
            "layer": self.layer.0,
            "tool": self.opts.tool,
            "size": self.opts.size,
            "strokes": self.strokes.len() + usize::from(self.cur.is_some()),
            "redo": self.redo.len(),
            "meshSize": self.cell,
            "maxDisplacement": self.field.max_displacement(),
            "proxy": [self.proxy.w, self.proxy.h],
            "dabMs": self.dab_ms,
            "fullResolutionPreview": self.view_bounds.is_some(),
            "previewMs": self.preview_ms,
            "smartFilter": self.smart_filter,
        })
    }

    fn template(&self, tool: LiquifyTool) -> LiquifyStroke {
        let o = &self.opts;
        LiquifyStroke {
            tool,
            size: f64::from(o.size),
            density: f64::from(o.density),
            pressure: f64::from(o.pressure),
            rate: f64::from(o.rate),
            points: Vec::new(),
            amount: None,
        }
    }

    fn mark(&mut self, doc_rect: Rect, mask: bool) {
        if doc_rect.is_empty() {
            return;
        }
        self.dirty = union(self.dirty, self.proxy.proxy_rect(doc_rect));
        self.view_dirty = union_rect(self.view_dirty, doc_rect);
        self.mask_dirty |= mask;
    }

    /// Starts a stroke; `alt` draws with the selected tool's opposite (see [`alt_tool`]).
    fn begin(&mut self, p: [f64; 3], now: f64, alt: bool) {
        // A stroke still open has already changed the field; record it so undo can replay it (#8055).
        self.end();
        self.redo.clear();
        let tool = if alt { alt_tool(self.opts.tool) } else { self.opts.tool };
        let mut s = self.template(tool);
        s.points.push(p.to_vec());
        let t0 = crate::gpu_canvas::now_ms();
        let d = self.field.stroke_begin(&s, p);
        self.mark(d, matches!(s.tool, LiquifyTool::Freeze | LiquifyTool::Thaw));
        self.render_dirty();
        self.dab_ms = crate::gpu_canvas::now_ms() - t0;
        self.cur = Some((s, p));
        self.last_dab = now;
    }

    fn extend(&mut self, p: [f64; 3], now: f64) {
        let Some((mut s, last)) = self.cur.take() else { return };
        let t0 = crate::gpu_canvas::now_ms();
        let d = self.field.stroke_segment(&s, last, p);
        s.points.push(p.to_vec());
        self.mark(d, matches!(s.tool, LiquifyTool::Freeze | LiquifyTool::Thaw));
        self.render_dirty();
        self.dab_ms = crate::gpu_canvas::now_ms() - t0;
        self.cur = Some((s, p));
        self.last_dab = now;
    }

    fn end(&mut self) {
        if let Some((s, _)) = self.cur.take() {
            self.strokes.push(s);
        }
    }

    /// Closes the lasso polygon into the freeze mask, recorded like any stroke (so undo, replay
    /// and OK all treat it the same as Freeze/Thaw brush work).
    fn close_lasso(&mut self, subtract: bool, mut pts: Vec<[f64; 2]>) {
        if pts.len() < 3 {
            return;
        }
        if let (Some(&first), Some(&last)) = (pts.first(), pts.last())
            && ((first[0] - last[0]).abs() > 0.5 || (first[1] - last[1]).abs() > 0.5)
        {
            pts.push(first);
        }
        self.redo.clear();
        let mut s = self.template(LiquifyTool::LassoMask);
        s.amount = Some(if subtract { 0.0 } else { 1.0 });
        s.points = pts.into_iter().map(|p| p.to_vec()).collect();
        let d = self.field.apply_stroke(&s);
        self.strokes.push(s);
        self.mark(d, true);
        self.render_dirty();
    }

    /// Applies a whole-field operation (Reconstruct…, mask buttons) as a stroke.
    fn global(&mut self, tool: LiquifyTool, amount: Option<f64>) {
        self.end();
        self.redo.clear();
        let mut s = LiquifyStroke::new(tool, 1.0);
        s.amount = amount;
        let d = self.field.apply_stroke(&s);
        self.strokes.push(s);
        self.mark(d, true);
        self.render_dirty();
    }

    fn rebuild(&mut self) {
        self.field = LiquifyField::from_strokes(self.canvas, self.cell, &self.strokes);
        self.dirty = Some([0, 0, self.proxy.w, self.proxy.h]);
        // Undo rebuilds the whole field. The zoomed viewport is a separate buffer; leaving it
        // clean kept the pre-undo pixels on screen until the next zoom (#4100).
        self.view_dirty = self.view_bounds;
        self.mask_dirty = true;
        self.render_dirty();
    }

    fn undo(&mut self) {
        self.end();
        if let Some(stroke) = self.strokes.pop() {
            if self.redo.len() == REDO_STACK_LIMIT {
                self.redo.remove(0);
            }
            self.redo.push(stroke);
            self.rebuild();
        }
    }

    fn redo(&mut self) {
        self.end();
        if let Some(stroke) = self.redo.pop() {
            let d = self.field.apply_stroke(&stroke);
            self.strokes.push(stroke);
            self.mark(d, true);
            self.render_dirty();
        }
    }

    fn restore_all(&mut self) {
        self.cur = None;
        self.lasso = None;
        self.strokes.clear();
        self.redo.clear();
        self.rebuild();
    }

    /// Re-renders the dirty proxy pixels into `out` (the texture upload happens in `show`).
    fn render_dirty(&mut self) {
        if let Some(r) = self.dirty {
            if let Some(source) = &self.native_proxy {
                let bounds = Rect::new(0, 0, self.proxy.w as i32, self.proxy.h as i32);
                let dirty = Rect::new(r[0] as i32, r[1] as i32, r[2] as i32, r[3] as i32);
                render_components(source, bounds, &self.field, self.proxy.scale, &self.target, bounds, dirty, &mut self.out);
            } else {
                self.proxy.render(&self.field, r, &mut self.out);
            }
        }
    }

    /// Builds or updates the visible source-resolution viewport. At ordinary zoom the whole-layer
    /// proxy remains cheaper; above 100%, a full-resolution crop is enough to fill the preview.
    fn update_view_preview(&mut self, ctx: &egui::Context, area: ERect) {
        if self.zoom <= 1.0 {
            return;
        }
        let half_w = f64::from(area.width() / self.zoom) * 0.5;
        let half_h = f64::from(area.height() / self.zoom) * 0.5;
        let clamp_x = |x: f64| x.max(f64::from(self.canvas.x0)).min(f64::from(self.canvas.x1)) as i32;
        let clamp_y = |y: f64| y.max(f64::from(self.canvas.y0)).min(f64::from(self.canvas.y1)) as i32;
        let bounds = Rect::new(
            clamp_x((self.center[0] - half_w).floor()),
            clamp_y((self.center[1] - half_h).floor()),
            clamp_x((self.center[0] + half_w).ceil()),
            clamp_y((self.center[1] + half_h).ceil()),
        );
        if bounds.is_empty() {
            self.view_bounds = None;
            self.view_tex = None;
            self.view_dirty = None;
            return;
        }
        let (w, h) = (bounds.width() as usize, bounds.height() as usize);
        let Some(len) = w.checked_mul(h).filter(|n| *n <= MAX_VIEW_PREVIEW_PIXELS) else {
            self.view_bounds = None;
            self.view_tex = None;
            self.view_dirty = None;
            return;
        };
        if self.view_bounds != Some(bounds) || self.view_out.len() != len {
            if self.view_out.try_reserve(len.saturating_sub(self.view_out.len())).is_err() {
                self.view_bounds = None;
                self.view_tex = None;
                self.view_dirty = None;
                return;
            }
            self.view_out.resize(len, [0; 4]);
            self.view_bounds = Some(bounds);
            self.view_tex = None;
            self.view_dirty = Some(bounds);
        }
        let Some(dirty) = self.view_dirty.take().map(|r| r.intersect(&bounds)) else { return };
        if dirty.is_empty() {
            return;
        }
        let started = crate::gpu_canvas::now_ms();
        let rendered = if self.native_proxy.is_some() {
            render_components(&self.source, self.canvas, &self.field, 1.0, &self.target, bounds, dirty, &mut self.view_out)
        } else {
            ProxyImage::render_full_resolution(&self.source, &self.field, bounds, dirty, &mut self.view_out, &mut self.view_source_scratch)
        };
        if !rendered {
            self.view_bounds = None;
            self.view_tex = None;
            self.preview_ms = crate::gpu_canvas::now_ms() - started;
            return;
        }
        let x0 = (dirty.x0 - bounds.x0) as usize;
        let y0 = (dirty.y0 - bounds.y0) as usize;
        let x1 = (dirty.x1 - bounds.x0) as usize;
        let y1 = (dirty.y1 - bounds.y0) as usize;
        let mut pixels = Vec::with_capacity((x1 - x0) * (y1 - y0));
        for y in y0..y1 {
            if let Some(row) = self.view_out.get(y * w + x0..y * w + x1) {
                pixels.extend(row.iter().map(|p| Color32::from_rgba_premultiplied(p[0], p[1], p[2], p[3])));
            }
        }
        let image = egui::ColorImage::new([x1 - x0, y1 - y0], pixels);
        match &mut self.view_tex {
            Some(tex) => tex.set_partial([x0, y0], image, egui::TextureOptions::NEAREST),
            None => {
                // A new viewport always starts dirty over its full bounds.
                self.view_tex = Some(ctx.load_texture(
                    "liquify-detail",
                    {
                        let pixels = self.view_out.iter().map(|p| Color32::from_rgba_premultiplied(p[0], p[1], p[2], p[3])).collect();
                        egui::ColorImage::new([w, h], pixels)
                    },
                    egui::TextureOptions::NEAREST,
                ));
            }
        }
        self.preview_ms = crate::gpu_canvas::now_ms() - started;
    }

    fn upload(&mut self, ctx: &egui::Context) {
        let (w, h) = (self.proxy.w, self.proxy.h);
        if self.tex.is_none() {
            self.dirty = Some([0, 0, w, h]);
            self.render_dirty();
        }
        match (&mut self.tex, self.dirty.take()) {
            (None, _) => {
                let img = egui::ColorImage::new([w, h], self.out.iter().map(|p| Color32::from_rgba_premultiplied(p[0], p[1], p[2], p[3])).collect());
                self.tex = Some(ctx.load_texture("liquify-proxy", img, egui::TextureOptions::LINEAR));
            }
            (Some(tex), Some(r)) => {
                let (x0, y0, x1, y1) = (r[0].min(w), r[1].min(h), r[2].min(w), r[3].min(h));
                if x1 > x0 && y1 > y0 {
                    let mut px = Vec::with_capacity((x1 - x0) * (y1 - y0));
                    for y in y0..y1 {
                        px.extend(self.out[y * w + x0..y * w + x1].iter().map(|p| Color32::from_rgba_premultiplied(p[0], p[1], p[2], p[3])));
                    }
                    tex.set_partial([x0, y0], egui::ColorImage::new([x1 - x0, y1 - y0], px), egui::TextureOptions::LINEAR);
                }
            }
            _ => {}
        }
        if self.mask_dirty || self.mask_tex.is_none() {
            self.mask_dirty = false;
            let f = &self.field;
            let img = egui::ColorImage::new(
                [f.w, f.h],
                f.freeze.iter().map(|v| Color32::from_rgba_unmultiplied(230, 40, 40, (v.clamp(0.0, 1.0) * 140.0) as u8)).collect(),
            );
            match &mut self.mask_tex {
                Some(t) => t.set(img, egui::TextureOptions::LINEAR),
                None => self.mask_tex = Some(ctx.load_texture("liquify-mask", img, egui::TextureOptions::LINEAR)),
            }
        }
    }
}

/// Opens the dialog on the active layer.
pub fn open(app: &mut PhotocraftApp, ctx: &egui::Context) -> Result<(), String> {
    let (layer, surf, _) = crate::distort_ui::active_filter_target(app)?;
    let canvas = app.session.active().ok_or("no document")?.doc.bounds();
    open_on(app, ctx, layer, surf, Vec::new(), auto_cell(canvas), None)
}

/// Re-opens Liquify smart filter `index` of smart object `layer` with its recorded strokes and
/// mesh size, previewing on the pixels below it; OK updates that filter
/// (`layer.smartFilter.setParams`).
pub fn open_smart_filter(app: &mut PhotocraftApp, ctx: &egui::Context, layer: LayerId, index: usize) -> Result<(), String> {
    use photocraft_engine::distort_cmds::{LIQUIFY, liquify_params};
    let st = app.session.active().ok_or("no document")?;
    let Some(photocraft_doc::LayerContent::Smart(sm)) = st.doc.layer(layer).map(|l| &l.content) else {
        return Err("the layer is not a smart object".into());
    };
    let filter = sm.smart_filters.get(index).ok_or("no such smart filter")?;
    if filter.command != LIQUIFY {
        return Err("the smart filter is not a Liquify filter".into());
    }
    let (strokes, cell) = liquify_params(LIQUIFY, &filter.params, st.doc.bounds()).map_err(|e| e.to_string())?;
    let surf = photocraft_engine::smart_cmds::render_below_filter(&st.doc, sm, index)
        .map_err(|e| e.to_string())?
        .ok_or("the smart object's contents are unavailable (missing linked file?)")?;
    open_on(app, ctx, layer, surf, strokes, cell, Some(index))
}

fn open_on(
    app: &mut PhotocraftApp,
    ctx: &egui::Context,
    layer: LayerId,
    surf: Surface,
    strokes: Vec<LiquifyStroke>,
    cell: f64,
    smart_filter: Option<usize>,
) -> Result<(), String> {
    let st = app.session.active().ok_or("no document")?;
    let canvas = st.doc.bounds();
    let name = st.doc.layer(layer).map(|l| l.name.clone()).unwrap_or_default();
    let proxy = ProxyImage::new(&surf, canvas, PROXY_SIDE);
    let target = if smart_filter.is_some() { json!("pixels") } else { crate::distort_ui::colour_set_target(app) };
    let colors = photocraft_engine::color_channels::explicit(&json!({"target":target}), surf.format().mode.color_channels()).map_err(|e| e.to_string())?;
    let native_proxy = colors.map(|_| crate::gallery_ui::reduce(&surf, canvas, proxy.scale as u32));
    let out = vec![[0u8; 4]; proxy.w * proxy.h];
    let center = [f64::from(canvas.x0 + canvas.x1) / 2.0, f64::from(canvas.y0 + canvas.y1) / 2.0];
    let mut d = LiquifyDialog {
        document: st.doc.id,
        layer,
        layer_name: name,
        canvas,
        cell,
        field: LiquifyField::from_strokes(canvas, cell, &strokes),
        strokes,
        redo: Vec::new(),
        cur: None,
        lasso: None,
        proxy,
        source: surf,
        target,
        native_proxy,
        out,
        tex: None,
        dirty: None,
        view_bounds: None,
        view_out: Vec::new(),
        view_source_scratch: Vec::new(),
        view_tex: None,
        view_dirty: None,
        backdrop: None,
        mask_tex: None,
        mask_dirty: true,
        opts: LiquifyOpts::default(),
        zoom: 0.0,
        center,
        hover: None,
        brush_resize: None,
        last_dab: 0.0,
        dab_ms: 0.0,
        preview_ms: 0.0,
        smart_filter,
    };
    // A sensible starting brush (about a tenth of the image), then the settings last used, so
    // Liquify opens as it was left (#418). Saved values are clamped; a bad one is skipped.
    d.opts.size = ((canvas.width().max(canvas.height()) as f32) / 10.0).round().clamp(10.0, 1500.0);
    if let Some(saved) = app.session.prefs().dialogs.get(REMEMBERED) {
        let _ = d.opts.apply(saved);
    }
    // Layer ids belong to one document: a remembered backdrop Use starts at All Layers.
    d.opts.backdrop_layer = None;
    d.upload(ctx);
    app.distort.liquify = Some(d);
    Ok(())
}

/// Keeps the brush settings for the next time Liquify opens, also after a restart.
fn remember(app: &mut PhotocraftApp, opts: &LiquifyOpts) {
    let v = opts.to_json();
    app.session.prefs.edit(|p| p.dialogs.insert(REMEMBERED.into(), v));
}

/// Cancel (button, Esc or the control channel): the document is untouched; the brush settings
/// are kept.
pub fn cancel(app: &mut PhotocraftApp) {
    if let Some(d) = app.distort.liquify.take() {
        remember(app, &d.opts);
    }
}

/// OK: runs `filter.liquify` with the recorded strokes (one history step). When the dialog edits
/// a smart filter, OK updates that filter instead (also when Restore All left no strokes). The
/// dialog closes either way. `app.run` shows a failed command's error in the status bar, and
/// `commit` returns it.
pub fn commit(app: &mut PhotocraftApp) -> Result<(), String> {
    let Some(mut d) = app.distort.liquify.take() else { return Ok(()) };
    remember(app, &d.opts);
    d.end();
    if let Some(index) = d.smart_filter {
        let params = json!({"strokes": d.strokes, "meshSize": d.cell});
        return app.run("layer.smartFilter.setParams", json!({"layer": d.layer.0, "index": index, "params": params})).map(|_| ());
    }
    if d.strokes.is_empty() {
        return Ok(());
    }
    let p = crate::distort_ui::with_filter_target(json!({"strokes": d.strokes, "meshSize": d.cell, "layer": d.layer.0}), &d.target);
    app.run("filter.liquify", p).map(|_| ())
}

/// Control channel: `filter.liquify {"ui": {...}}` while the dialog is open.
pub fn control(app: &mut PhotocraftApp, ui: &Value) -> Result<Value, String> {
    if ui.get("commit").and_then(Value::as_bool) == Some(true) {
        commit(app)?;
        return Ok(json!({"committed": true}));
    }
    if ui.get("cancel").and_then(Value::as_bool) == Some(true) {
        cancel(app);
        return Ok(json!({"cancelled": true}));
    }
    // A `backdropUse` that names no layer of the document is All Layers, as the Use list shows it.
    let missing = |id: u64| app.session.active().is_none_or(|st| st.doc.layer(LayerId(id)).is_none());
    let unknown_use = ui.get("backdropUse").and_then(Value::as_u64).is_some_and(missing);
    let d = app.distort.liquify.as_mut().ok_or(tl!("Liquify is not open"))?;
    // `apply` takes every setting it can before a bad one fails it, so normalize first.
    let applied = d.opts.apply(ui);
    if unknown_use {
        d.opts.backdrop_layer = None;
    }
    applied?;
    let num = |k: &str| ui.get(k).and_then(Value::as_f64).map(|v| v as f32);
    let flag = |k: &str| ui.get(k).and_then(Value::as_bool);
    if flag("undo") == Some(true) {
        d.undo();
    }
    if flag("redo") == Some(true) {
        d.redo();
    }
    if flag("restoreAll") == Some(true) {
        d.restore_all();
    }
    if let Some(a) = num("reconstruct") {
        d.global(LiquifyTool::ReconstructAll, Some(f64::from(a)));
    }
    for (k, t) in [("maskAll", LiquifyTool::FreezeAll), ("maskNone", LiquifyTool::ThawAll), ("invertMask", LiquifyTool::InvertFreeze)] {
        if flag(k) == Some(true) {
            d.global(t, None);
        }
    }
    Ok(d.describe())
}

/// The Alt (Option) variant of a Liquify brush: Twirl Clockwise twirls counterclockwise (the
/// reference application has no separate counterclockwise tool) and Pucker and Bloat swap.
/// Other tools are unchanged.
fn alt_tool(t: LiquifyTool) -> LiquifyTool {
    match t {
        LiquifyTool::TwirlCw => LiquifyTool::TwirlCcw,
        LiquifyTool::TwirlCcw => LiquifyTool::TwirlCw,
        LiquifyTool::Pucker => LiquifyTool::Bloat,
        LiquifyTool::Bloat => LiquifyTool::Pucker,
        t => t,
    }
}

/// Where the brush outline is drawn: under the pointer, except during an Alt-drag of the brush
/// size (`resize`), when it stays at the drag's anchor and only grows or shrinks, as the brush
/// tools' Alt-drag does outside Liquify (#7884).
fn brush_circle_at(resize: Option<(Pos2, f32, f32)>, hover: Option<Pos2>) -> Option<Pos2> {
    resize.map(|(anchor, ..)| anchor).or(hover)
}

/// Alt+right-drag, and Control+Alt-drag, change the Liquify brush instead of painting (#4702).
fn brush_resize_gesture(mods: egui::Modifiers, secondary: bool) -> bool {
    if secondary { crate::brush_resize::is_right_gesture(mods) } else { crate::brush_resize::is_gesture(mods) }
}

/// Horizontal motion sets the diameter (the circle's edge follows the pointer). Vertical motion
/// sets density, Liquify's edge falloff, over the same 200 px travel the canvas uses for hardness.
fn resized_brush(start_size: f32, start_density: f32, dx: f32, dy: f32, zoom: f32) -> (f32, f32) {
    // Liquify stores density as 0–100; the shared brush gesture works on normalized hardness.
    // Use the same zoom-corrected screen movement and size limits as the main canvas.
    let zoom = if zoom.is_finite() && zoom > 0.0 { zoom } else { 1.0 };
    let (size, density) = crate::brush_resize::resized((start_size, start_density / 100.0), f64::from(dx / zoom), f64::from(dy / zoom), zoom);
    (size, (density * 100.0).round())
}

/// Pointer in document coordinates (from the preview or the control channel). Alt at
/// pointer-down subtracts from the freeze mask (lasso), reverses a twirl and swaps Pucker/Bloat;
/// Shift adds to the mask, same as no modifier.
pub fn pointer(app: &mut PhotocraftApp, ev: ToolEvent, mods: egui::Modifiers) {
    let now = crate::gpu_canvas::now_ms();
    let Some(d) = app.distort.liquify.as_mut() else { return };
    match ev {
        ToolEvent::Down { x, y, pressure } => {
            if d.opts.tool == LiquifyTool::LassoMask {
                d.lasso = Some((mods.alt, vec![[x, y]]));
            } else {
                d.begin([x, y, f64::from(pressure)], now, mods.alt);
            }
        }
        ToolEvent::Move { x, y, pressure } => {
            d.hover = Some([x, y]);
            if let Some((_, pts)) = &mut d.lasso {
                if pts.last().is_none_or(|&l| l[0] != x || l[1] != y) {
                    pts.push([x, y]);
                }
            } else if d.cur.is_some() {
                d.extend([x, y, f64::from(pressure)], now);
            }
        }
        ToolEvent::Up { x, y } => {
            if let Some((subtract, pts)) = d.lasso.take() {
                d.close_lasso(subtract, pts);
                return;
            }
            if let Some((_, last)) = d.cur
                && (last[0] != x || last[1] != y)
            {
                d.extend([x, y, last[2]], now);
            }
            d.end();
        }
    }
}

/// Ctrl+Z / Ctrl+Shift+Z inside Liquify, including while a slider's text field is focused.
pub fn history_keys(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let Some(d) = app.distort.liquify.as_mut() else { return };
    let cmd_shift = egui::Modifiers::COMMAND | egui::Modifiers::SHIFT;
    if ctx.input_mut(|i| i.consume_key(cmd_shift, egui::Key::Z)) {
        d.redo();
    } else if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::Z)) {
        d.undo();
    }
}

pub fn keys(app: &mut PhotocraftApp, ctx: &egui::Context) {
    history_keys(app, ctx);
    // ⌘+ / ⌘− / ⌘0 / ⌘1 (or their overrides) zoom the preview, not the canvas behind it (#7210).
    let nav = if app.distort.liquify.is_some() { crate::shortcuts::nav_shortcut(app, ctx) } else { None };
    let Some(d) = app.distort.liquify.as_mut() else { return };
    if let Some(id) = nav {
        d.zoom = match id {
            "view.zoomIn" => crate::zoom_levels::step_in_range(d.zoom, 1, ZOOM_MIN, ZOOM_MAX),
            "view.zoomOut" => crate::zoom_levels::step_in_range(d.zoom, -1, ZOOM_MIN, ZOOM_MAX),
            // Fit on Screen: the preview fits and centres the image on its next frame.
            "view.fitOnScreen" => 0.0,
            _ => 1.0,
        };
    }
    // Ctrl+H (Hide Extras) toggles the red freeze-mask overlay; the mask itself stays active.
    if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::H)) {
        d.opts.show_mask = !d.opts.show_mask;
    }
    // Ctrl+I inverts the freeze mask (the Invert All button); Ctrl+D clears it (None).
    if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::I)) {
        d.global(LiquifyTool::InvertFreeze, None);
    }
    if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::D)) {
        d.global(LiquifyTool::ThawAll, None);
    }
    if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::OpenBracket)) {
        // At least one pixel, or 2 px * 0.9 rounds back to 2 and `[` never reaches 1.
        d.opts.size = (d.opts.size * 0.9).round().min(d.opts.size - 1.0).clamp(1.0, 15000.0);
    }
    if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::CloseBracket)) {
        d.opts.size = (d.opts.size * 1.1).round().max(d.opts.size + 1.0).min(15000.0);
    }
    let tools = [
        (egui::Key::W, LiquifyTool::ForwardWarp),
        (egui::Key::R, LiquifyTool::Reconstruct),
        (egui::Key::E, LiquifyTool::Smooth),
        (egui::Key::C, LiquifyTool::TwirlCw),
        (egui::Key::S, LiquifyTool::Pucker),
        (egui::Key::B, LiquifyTool::Bloat),
        (egui::Key::O, LiquifyTool::PushLeft),
        (egui::Key::F, LiquifyTool::Freeze),
        (egui::Key::D, LiquifyTool::Thaw),
        (egui::Key::L, LiquifyTool::LassoMask),
    ];
    for (k, t) in tools {
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, k)) {
            d.opts.tool = t;
        }
    }
}

fn tool_icon(t: LiquifyTool) -> &'static str {
    match t {
        LiquifyTool::ForwardWarp => "pointer",
        LiquifyTool::Reconstruct => "brush-cleaning",
        LiquifyTool::Smooth => "droplet",
        LiquifyTool::TwirlCw => "rotate-cw",
        LiquifyTool::Pucker => "scan",
        LiquifyTool::Bloat => "maximize-2",
        LiquifyTool::PushLeft => "chevrons-left",
        LiquifyTool::Freeze => "lock",
        LiquifyTool::Thaw => "lock-open",
        LiquifyTool::LassoMask => "lasso",
        _ => "circle",
    }
}

fn shortcut(t: LiquifyTool) -> &'static str {
    match t {
        LiquifyTool::ForwardWarp => "W",
        LiquifyTool::Reconstruct => "R",
        LiquifyTool::Smooth => "E",
        LiquifyTool::TwirlCw => "C",
        LiquifyTool::Pucker => "S",
        LiquifyTool::Bloat => "B",
        LiquifyTool::PushLeft => "O",
        LiquifyTool::Freeze => "F",
        LiquifyTool::Thaw => "D",
        LiquifyTool::LassoMask => "L",
        _ => "",
    }
}

/// The backdrop for Show Backdrop › Use `of`: the document as it is (All Layers, the liquified
/// layer's own pixels included, as in Photoshop) or that one layer alone. A layer that is gone
/// shows All Layers.
fn load_backdrop(app: &PhotocraftApp, ctx: &egui::Context, of: Option<u64>, canvas: Rect) -> Option<TextureHandle> {
    let st = app.session.active()?;
    let buf = photocraft_compose::render(&backdrop_doc(&st.doc, of), canvas);
    let mut s = photocraft_raster::Surface::new(photocraft_color::PixelFormat::RGBA8);
    let flat: Vec<f32> = buf.px.iter().flat_map(|p| *p).collect();
    s.write_region(canvas, &flat);
    Some(ctx.load_texture("liquify-backdrop", crate::distort_ui::surface_image(&s, canvas, PROXY_SIDE), egui::TextureOptions::LINEAR))
}

/// The document [`load_backdrop`] renders for Use `of`.
fn backdrop_doc(doc: &Document, of: Option<u64>) -> std::borrow::Cow<'_, Document> {
    match of.map(LayerId).filter(|id| doc.layer(*id).is_some()) {
        Some(id) => {
            let mut d = doc.clone();
            crate::transform_tool::solo_layer(&mut d.layers, id);
            // A hidden layer, or one in a hidden group, shows too: Use lists every layer.
            reveal(&mut d.layers, id);
            std::borrow::Cow::Owned(d)
        }
        None => std::borrow::Cow::Borrowed(doc),
    }
}

/// Makes layer `id` and the groups holding it visible; whether it was found.
fn reveal(layers: &mut [photocraft_doc::Layer], id: LayerId) -> bool {
    layers.iter_mut().any(|l| {
        let found = l.id == id
            || match &mut l.content {
                photocraft_doc::LayerContent::Group(g) => reveal(&mut g.children, id),
                _ => false,
            };
        if found {
            l.visible = true;
        }
        found
    })
}

/// How the preview layers the image and the backdrop texture `T`: the image's tint, and the
/// backdrop drawn behind it or in front of it, with its tint.
#[derive(Debug, PartialEq)]
struct Layering<T> {
    image: Color32,
    behind: Option<(T, Color32)>,
    front: Option<(T, Color32)>,
}

/// The [`Layering`] for the backdrop `backdrop` (when shown) in `mode` at `opacity` (0..1):
/// Photoshop's modes, measured on its Liquify (#7609).
fn backdrop_layering<T: Copy>(backdrop: Option<T>, mode: BackdropMode, opacity: f32) -> Layering<T> {
    let alpha = |a: f32| Color32::from_white_alpha((a.clamp(0.0, 1.0) * 255.0).round() as u8);
    let (image, behind, front) = match (backdrop, mode) {
        (None, _) => (Color32::WHITE, None, None),
        (Some(b), BackdropMode::InFront) => (Color32::WHITE, None, Some((b, alpha(opacity)))),
        (Some(b), BackdropMode::Behind) => (alpha(opacity), Some((b, Color32::WHITE)), None),
        (Some(b), BackdropMode::Blend) => (alpha(1.0 - opacity), None, Some((b, alpha(opacity)))),
    };
    Layering { image, behind, front }
}

/// Draws the dialog (a full-window layer over the app).
fn draw_tool_strip(ui: &mut egui::Ui, painter: &egui::Painter, t: &Tokens, d: &mut LiquifyDialog, body: ERect) -> ERect {
    // Left tool strip.
    let left = ERect::from_min_size(body.min, vec2(LEFT_W, body.height()));
    painter.rect_filled(left, 0.0, t.dock);
    painter.line_segment([left.right_top(), left.right_bottom()], Stroke::new(1.0, t.separator));
    let mut strip = ui.new_child(egui::UiBuilder::new().max_rect(left.shrink2(vec2(6.0, 8.0))));
    strip.spacing_mut().item_spacing.y = 4.0;
    for tool in LiquifyTool::ALL {
        let tip = match tool {
            // The lasso works on the same freeze mask as Freeze/Thaw.
            LiquifyTool::LassoMask => tl!("Freeze Lasso: drag to freeze an area, Alt-drag to thaw it (L)").to_string(),
            _ => format!("{} ({})", tl!(tool.label()), shortcut(tool)),
        };
        if crate::icons::button(&mut strip, tool_icon(tool), 34.0, d.opts.tool == tool, &tip).clicked() {
            d.opts.tool = tool;
        }
        if matches!(tool, LiquifyTool::Smooth | LiquifyTool::PushLeft | LiquifyTool::Thaw) {
            strip.add_space(6.0);
        }
    }
    left
}

/// The preview's wheel and trackpad input: zoom about the pointer, or pan.
fn apply_preview_wheel(
    ctx: &egui::Context,
    d: &mut LiquifyDialog,
    area: ERect,
    resp: &egui::Response,
    wheel_id: egui::Id,
    pinch_zooms: bool,
    animated_zoom: bool,
) {
    let wheel = crate::wheel_nav::read(ctx, true, pinch_zooms, animated_zoom, wheel_id, resp.hovered());
    if let Some(hp) = resp.hover_pos() {
        let zoom = match wheel {
            Some(crate::wheel_nav::Wheel::Zoom(f)) => {
                let z = d.zoom * f;
                // At a limit, drop the rest of an eased notch, or it would swallow the first notch back.
                if z != z.clamp(ZOOM_MIN, ZOOM_MAX) {
                    crate::wheel_nav::settle(ctx, wheel_id);
                }
                Some(z.clamp(ZOOM_MIN, ZOOM_MAX))
            }
            Some(crate::wheel_nav::Wheel::SnapZoom(n)) => {
                Some((0..n.unsigned_abs()).fold(d.zoom, |z, _| crate::zoom_levels::step_in_range(z, n.signum(), ZOOM_MIN, ZOOM_MAX)))
            }
            Some(crate::wheel_nav::Wheel::Pan(v)) => {
                d.center = [d.center[0] - f64::from(v.x / d.zoom), d.center[1] - f64::from(v.y / d.zoom)];
                None
            }
            None => None,
        };
        if let Some(z) = zoom {
            let before = to_doc(d, area, hp);
            d.zoom = z;
            let after = to_doc(d, area, hp);
            d.center = [d.center[0] + before[0] - after[0], d.center[1] + before[1] - after[1]];
        }
    }
}

pub fn show(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let t = Tokens::get(ctx);
    let screen = ctx.content_rect();
    // The backdrop is built when it's first shown and again when its Use changes.
    let wanted = app.distort.liquify.as_ref().filter(|d| d.opts.show_backdrop && d.backdrop.as_ref().is_none_or(|(of, _)| *of != d.opts.backdrop_layer));
    if let Some((of, canvas)) = wanted.map(|d| (d.opts.backdrop_layer, d.canvas)) {
        let tex = load_backdrop(app, ctx, of, canvas);
        if let Some(d) = app.distort.liquify.as_mut() {
            d.backdrop = tex.map(|t| (of, t));
        }
    }
    // Show Backdrop › Use: All Layers or one layer, bottom to top as Photoshop lists them.
    let backdrop_layers: Vec<(Option<u64>, String)> = std::iter::once((None, tl!("All Layers").to_string()))
        .chain(app.session.active().into_iter().flat_map(|st| st.doc.walk().into_iter().map(|(_, _, l)| (Some(l.id.0), l.name.clone())).collect::<Vec<_>>()))
        .collect();
    let mut action: Option<&str> = None;
    let mut events: Vec<ToolEvent> = Vec::new();
    let mut mods = egui::Modifiers::NONE;
    let pinch_zooms = app.session.prefs().enhanced_controls.zoom_with_trackpad_pinch;
    let animated_zoom = app.session.prefs().general.animated_zoom;
    egui::Area::new(egui::Id::unique("liquify-dialog")).order(egui::Order::Foreground).fixed_pos(screen.min).show(ctx, |ui| {
        let Some(d) = app.distort.liquify.as_mut() else { return };
        d.upload(ctx);
        let (full, _) = ui.allocate_exact_size(screen.size(), Sense::hover());
        let painter = ui.painter().clone();
        painter.rect_filled(full, 0.0, t.chrome);
        // Title strip.
        let title = ERect::from_min_size(full.min, vec2(full.width(), 30.0));
        painter.rect_filled(title, 0.0, t.dock);
        painter.line_segment([title.left_bottom(), title.right_bottom()], Stroke::new(1.0, t.separator));
        let pct = if d.zoom > 0.0 { d.zoom * 100.0 } else { 100.0 };
        painter.text(
            title.center(),
            Align2::CENTER_CENTER,
            format!("{} ({}, {:.0}%)", tl!("Liquify…").trim_end_matches('…'), d.layer_name, pct),
            FontId::proportional(13.0),
            t.text,
        );
        let body = ERect::from_min_max(pos2(full.left(), title.bottom()), full.max);
        let left = draw_tool_strip(ui, &painter, &t, d, body);
        // Right properties panel.
        let right = ERect::from_min_size(pos2(body.right() - RIGHT_W, body.top()), vec2(RIGHT_W, body.height()));
        painter.rect_filled(right, 0.0, t.dock);
        painter.line_segment([right.left_top(), right.left_bottom()], Stroke::new(1.0, t.separator));
        let mut props = ui.new_child(egui::UiBuilder::new().max_rect(right.shrink2(vec2(14.0, 12.0))));
        egui::ScrollArea::vertical().id_salt("liquify-props").show(&mut props, |ui| {
            ui.spacing_mut().item_spacing.y = 6.0;
            widgets::section_label(ui, tl!("Brush Tool Options"));
            widgets::slider_row(ui, tl!("Size"), &mut d.opts.size, 1.0..=3000.0, "", None);
            widgets::slider_row(ui, tl!("Density"), &mut d.opts.density, 0.0..=100.0, "", None);
            widgets::slider_row(ui, tl!("Pressure"), &mut d.opts.pressure, 0.0..=100.0, "", None);
            widgets::slider_row(ui, tl!("Rate"), &mut d.opts.rate, 0.0..=100.0, "", None);
            widgets::hairline(ui);
            widgets::section_label(ui, tl!("Brush Reconstruct Options"));
            widgets::slider_row(ui, tl!("Amount"), &mut d.opts.reconstruct_amount, 0.0..=100.0, "%", None);
            ui.horizontal(|ui| {
                if widgets::secondary_button(ui, tl!("Reconstruct"), 110.0).clicked() {
                    let a = f64::from(d.opts.reconstruct_amount);
                    d.global(LiquifyTool::ReconstructAll, Some(a));
                }
                if widgets::secondary_button(ui, tl!("Restore All"), 110.0).clicked() {
                    d.restore_all();
                }
            });
            widgets::hairline(ui);
            widgets::section_label(ui, tl!("Mask Options"));
            ui.horizontal(|ui| {
                if widgets::secondary_button(ui, tl!("None"), 70.0).clicked() {
                    d.global(LiquifyTool::ThawAll, None);
                }
                if widgets::secondary_button(ui, tl!("Mask All"), 80.0).clicked() {
                    d.global(LiquifyTool::FreezeAll, None);
                }
                if widgets::secondary_button(ui, tl!("Invert All"), 80.0).clicked() {
                    d.global(LiquifyTool::InvertFreeze, None);
                }
            });
            widgets::hairline(ui);
            widgets::section_label(ui, tl!("View Options"));
            widgets::checkbox(ui, &mut d.opts.show_mesh, tl!("Show Mesh"));
            ui.horizontal(|ui| {
                ui.label(tl!("Mesh Size"));
                widgets::dropdown(
                    ui,
                    "liquify-mesh-size",
                    tl!("Mesh Size"),
                    &mut d.opts.mesh_size,
                    &[(0, tl!("Small")), (1, tl!("Medium")), (2, tl!("Large"))],
                    110.0,
                );
            });
            widgets::checkbox(ui, &mut d.opts.show_mask, tl!("Show Mask"));
            widgets::checkbox(ui, &mut d.opts.show_backdrop, tl!("Show Backdrop"));
            if d.opts.show_backdrop {
                ui.horizontal(|ui| {
                    ui.label(tl!("Use"));
                    let names: Vec<(Option<u64>, &str)> = backdrop_layers.iter().map(|(id, n)| (*id, n.as_str())).collect();
                    widgets::dropdown_literal(ui, "liquify-backdrop-use", tl!("Layer"), &mut d.opts.backdrop_layer, &names, 150.0);
                });
                ui.horizontal(|ui| {
                    ui.label(tl!("Mode"));
                    let modes = [(BackdropMode::InFront, tl!("In Front")), (BackdropMode::Behind, tl!("Behind")), (BackdropMode::Blend, tl!("Blend"))];
                    widgets::dropdown(ui, "liquify-backdrop-mode", tl!("Mode"), &mut d.opts.backdrop_mode, &modes, 150.0);
                });
                widgets::slider_row(ui, tl!("Opacity"), &mut d.opts.backdrop_opacity, 0.0..=100.0, "", None);
            }
            widgets::hairline(ui);
            ui.label(
                egui::RichText::new(format!(
                    "{} stroke(s) · field {} px/node · dab {:.2} ms · preview {:.2} ms",
                    d.strokes.len(),
                    d.cell,
                    d.dab_ms,
                    d.preview_ms
                ))
                .color(t.text_faint)
                .size(11.0),
            );
        });
        // Footer buttons.
        let foot = ERect::from_min_size(pos2(right.left() + 14.0, right.bottom() - 48.0), vec2(RIGHT_W - 28.0, 32.0));
        let mut fb = ui.new_child(egui::UiBuilder::new().max_rect(foot).layout(egui::Layout::right_to_left(egui::Align::Center)));
        if let Some(role) = widgets::dialog_buttons(
            &mut fb,
            &[
                widgets::DialogButton::new(widgets::ButtonRole::Default, tl!("OK"), 84.0),
                widgets::DialogButton::new(widgets::ButtonRole::Cancel, tl!("Cancel"), 84.0),
            ],
        ) {
            action = Some(if role == widgets::ButtonRole::Default { "ok" } else { "cancel" });
        }
        // Preview.
        let area = ERect::from_min_max(pos2(left.right(), body.top()), pos2(right.left(), body.bottom()));
        painter.rect_filled(area, 0.0, t.canvas);
        let (cw, ch) = (d.canvas.width() as f32, d.canvas.height() as f32);
        if d.zoom <= 0.0 {
            d.zoom = ((area.width() - 40.0) / cw).min((area.height() - 40.0) / ch).clamp(ZOOM_MIN, 4.0);
            d.center = [f64::from(d.canvas.x0) + f64::from(cw) / 2.0, f64::from(d.canvas.y0) + f64::from(ch) / 2.0];
        }
        let resp = ui.interact(area, egui::Id::unique("liquify-preview"), Sense::click_and_drag());
        // The wheel zooms about the pointer, ×1.1 a notch, and so does a trackpad pinch (unless
        // Preferences › Enhanced Controls › Zoom with Trackpad Pinch is off); a sideways scroll
        // pans (#7210). Read every frame, as `wheel_nav` asks, so its modifier state follows.
        let wheel_id = egui::Id::unique("liquify-preview-wheel");
        apply_preview_wheel(ctx, d, area, &resp, wheel_id, pinch_zooms, animated_zoom);
        d.update_view_preview(ctx, area);
        let img = ERect::from_min_max(
            to_screen(d, area, [f64::from(d.canvas.x0), f64::from(d.canvas.y0)]),
            to_screen(d, area, [f64::from(d.canvas.x1), f64::from(d.canvas.y1)]),
        );
        let clip = painter.with_clip_rect(area);
        widgets::checker(&clip, img.intersect(area), 8.0);
        let uv = ERect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
        let backdrop = d.backdrop.as_ref().filter(|_| d.opts.show_backdrop).map(|(_, t)| t.id());
        let Layering { image: image_tint, behind, front } = backdrop_layering(backdrop, d.opts.backdrop_mode, d.opts.backdrop_opacity / 100.0);
        if let Some((b, c)) = behind {
            clip.image(b, img, uv, c);
        }
        if d.zoom > 1.0
            && let (Some(bounds), Some(tex)) = (d.view_bounds, &d.view_tex)
        {
            let detail = ERect::from_min_max(
                to_screen(d, area, [f64::from(bounds.x0), f64::from(bounds.y0)]),
                to_screen(d, area, [f64::from(bounds.x1), f64::from(bounds.y1)]),
            );
            clip.image(tex.id(), detail, uv, image_tint);
        } else {
            // The proxy covers whole proxy pixels, a little past the canvas edge.
            let pr = ERect::from_min_max(
                img.min,
                img.min + vec2(d.proxy.w as f32 * d.proxy.scale as f32 * d.zoom, d.proxy.h as f32 * d.proxy.scale as f32 * d.zoom),
            );
            if let Some(tex) = &d.tex {
                clip.image(tex.id(), pr, uv, image_tint);
            }
        }
        if let Some((b, c)) = front {
            clip.image(b, img, uv, c);
        }
        if d.opts.show_mask
            && let Some(m) = &d.mask_tex
        {
            let f = &d.field;
            let mr = ERect::from_min_size(
                img.min - vec2(0.5, 0.5) * d.zoom * f.cell as f32 + vec2(0.5, 0.5) * d.zoom,
                vec2(f.w as f32, f.h as f32) * f.cell as f32 * d.zoom,
            );
            clip.image(m.id(), mr, uv, Color32::WHITE);
        }
        if d.opts.show_mesh {
            draw_mesh(d, &clip, area);
        }
        // The lasso polygon being drawn (danger colour freezes, accent thaws; the mask overlay paints the
        // applied area red like the Freeze brush).
        if let Some((subtract, pts)) = &d.lasso
            && pts.len() > 1
        {
            let color = if *subtract { t.accent } else { t.danger };
            let line: Vec<Pos2> = pts.iter().map(|p| to_screen(d, area, *p)).collect();
            clip.add(egui::Shape::line(line, Stroke::new(1.0, color)));
        }
        // Brush outline (the lasso has none). While Alt-dragging its size it stays where the drag
        // started, so only its size changes (#7884).
        if let Some(hp) = brush_circle_at(d.brush_resize, resp.hover_pos()) {
            if d.opts.tool != LiquifyTool::LassoMask {
                let center = d.brush_resize.map_or(hp, |(anchor, _, _)| anchor);
                let r = d.opts.size * 0.5 * d.zoom;
                if d.brush_resize.is_some() {
                    crate::brush_resize::liquify_tip_preview(
                        &clip,
                        center,
                        r,
                        d.opts.density / 100.0,
                        Color32::from_rgba_unmultiplied(t.accent.r(), t.accent.g(), t.accent.b(), 110),
                    );
                    ui.ctx().set_cursor_icon(egui::CursorIcon::None);
                    crate::canvas::draw_readout(
                        ui.ctx(),
                        "liquify-brush-resize-readout",
                        hp,
                        [tl!("Diameter:"), tl!("Density:")],
                        [format!("{} px", d.opts.size.round() as i64), format!("{}%", d.opts.density.round() as i64)],
                    );
                }
                clip.circle_stroke(center, r, Stroke::new(1.5, Color32::BLACK));
                clip.circle_stroke(center, r, Stroke::new(0.75, Color32::WHITE));
            }
            d.hover = Some(to_doc(d, area, hp));
        }
        clip.rect_stroke(img, 0.0, Stroke::new(1.0, t.separator), egui::StrokeKind::Outside);
        // Mouse → strokes (Space or middle button pans). Alt+right-drag resizes the brush (#4702).
        let (pan, secondary, mods_now) =
            ui.input(|i| (i.key_down(egui::Key::Space) || i.pointer.middle_down(), i.pointer.button_down(egui::PointerButton::Secondary), i.modifiers));
        // From the press, not once the drag is recognised a few points later: that first frame
        // would otherwise start a stroke.
        let resize = d.brush_resize.is_some() || (resp.is_pointer_button_down_on() && brush_resize_gesture(mods_now, secondary));
        if resize {
            // Anchored where the button went down, not where the drag was recognised a few
            // points later, so the outline doesn't jump when the gesture starts.
            if d.brush_resize.is_none()
                && let Some(p) = ui.input(|i| i.pointer.press_origin()).or_else(|| resp.interact_pointer_pos())
            {
                d.brush_resize = Some((p, d.opts.size, d.opts.density));
            }
            if let (Some((anchor, size0, dens0)), Some(p)) = (d.brush_resize, resp.interact_pointer_pos()) {
                let delta = p - anchor;
                let (size, density) = resized_brush(size0, dens0, delta.x, delta.y, d.zoom);
                d.opts.size = size;
                d.opts.density = density;
            }
            if resp.drag_stopped() || !ui.input(|i| i.pointer.any_down()) {
                d.brush_resize = None;
            }
            ctx.request_repaint();
        } else if pan && resp.dragged() {
            let dl = resp.drag_delta();
            d.center = [d.center[0] - f64::from(dl.x / d.zoom), d.center[1] - f64::from(dl.y / d.zoom)];
        } else if let Some(pp) = resp.interact_pointer_pos() {
            let q = to_doc(d, area, pp);
            let pr = 1.0;
            mods = ui.input(|i| i.modifiers);
            if d.lasso.is_some() {
                // A lasso in progress: collect the polygon, no brush dabs.
                if resp.dragged() {
                    events.push(ToolEvent::Move { x: q[0], y: q[1], pressure: pr });
                    ctx.request_repaint();
                }
            } else if (resp.drag_started() || resp.is_pointer_button_down_on()) && d.cur.is_none() {
                // The stroke already began at the press; egui's later drag start must not restart it (#8055).
                events.push(ToolEvent::Down { x: q[0], y: q[1], pressure: pr });
            } else if resp.dragged() || resp.is_pointer_button_down_on() {
                let moved = d.cur.as_ref().is_some_and(|(_, l)| l[0] != q[0] || l[1] != q[1]);
                // Stationary tools keep working while the brush is held still (30 dabs/s).
                let held = d.opts.tool.is_stationary() && crate::gpu_canvas::now_ms() - d.last_dab > 33.0;
                if moved || held {
                    events.push(ToolEvent::Move { x: q[0], y: q[1], pressure: pr });
                }
                ctx.request_repaint();
            }
        }
        let primary_down = ui.input(|i| i.pointer.primary_down());
        let lasso_up = d.lasso.is_some() && !primary_down;
        if resp.drag_stopped() || lasso_up || (d.cur.is_some() && !primary_down) {
            let q = d.lasso.as_ref().and_then(|(_, pts)| pts.last().copied()).or_else(|| d.cur.as_ref().map(|(_, l)| [l[0], l[1]])).unwrap_or_default();
            events.push(ToolEvent::Up { x: q[0], y: q[1] });
        }
    });
    for ev in events {
        pointer(app, ev, mods);
    }
    match action {
        Some("ok") => {
            let _ = commit(app);
        }
        Some("cancel") => cancel(app),
        _ => {}
    }
}

/// The preview's zoom range (1 = the dialog's 100 %). Liquify keeps its own zoom, apart from the
/// canvas behind it.
const ZOOM_MIN: f32 = 0.01;
const ZOOM_MAX: f32 = 32.0;

fn to_screen(d: &LiquifyDialog, area: ERect, p: [f64; 2]) -> Pos2 {
    area.center() + vec2(((p[0] - d.center[0]) as f32) * d.zoom, ((p[1] - d.center[1]) as f32) * d.zoom)
}

fn to_doc(d: &LiquifyDialog, area: ERect, p: Pos2) -> [f64; 2] {
    let v = (p - area.center()) / d.zoom;
    [d.center[0] + f64::from(v.x), d.center[1] + f64::from(v.y)]
}

/// The deformed mesh: where the source grid lines land. Each output sample knows its source
/// position `p + d(p)`, so a source line `x = c` is the iso-line of that function, traced with
/// marching squares on a sample grid four times finer than the mesh (exact for any field, unlike
/// inverting the field point by point, which fails where it folds).
fn draw_mesh(d: &LiquifyDialog, painter: &egui::Painter, area: ERect) {
    let c = d.canvas;
    let cells = [64.0, 40.0, 24.0][d.opts.mesh_size.min(2)];
    let step = f64::from(c.width().max(c.height())) / cells;
    let h = step / 4.0;
    let nx = (f64::from(c.width()) / h).ceil() as usize + 1;
    let ny = (f64::from(c.height()) / h).ceil() as usize + 1;
    let (x0, y0) = (f64::from(c.x0), f64::from(c.y0));
    let mut src = Vec::with_capacity(nx * ny);
    for j in 0..ny {
        for i in 0..nx {
            let p = [x0 + i as f64 * h, y0 + j as f64 * h];
            let v = d.field.sample(p[0], p[1]);
            src.push([p[0] + v[0], p[1] + v[1]]);
        }
    }
    let stroke = Stroke::new(0.75, Color32::from_rgba_unmultiplied(90, 90, 90, 210));
    let pos = |i: usize, j: usize| [x0 + i as f64 * h, y0 + j as f64 * h];
    for axis in [0usize, 1] {
        let origin = if axis == 0 { x0 } else { y0 };
        for jj in 0..ny - 1 {
            for ii in 0..nx - 1 {
                let corners = [(ii, jj), (ii + 1, jj), (ii + 1, jj + 1), (ii, jj + 1)];
                let vals = corners.map(|(i, j)| src[j * nx + i][axis]);
                let (lo, hi) = (vals.iter().copied().fold(f64::MAX, f64::min), vals.iter().copied().fold(f64::MIN, f64::max));
                let mut k = ((lo - origin) / step).ceil() as i64;
                while origin + k as f64 * step <= hi {
                    let level = origin + k as f64 * step;
                    let mut pts: Vec<[f64; 2]> = Vec::with_capacity(4);
                    for e in 0..4 {
                        let (a, b) = (vals[e] - level, vals[(e + 1) % 4] - level);
                        if (a <= 0.0) != (b <= 0.0) {
                            let t = a / (a - b);
                            let (pa, pb) = (pos(corners[e].0, corners[e].1), pos(corners[(e + 1) % 4].0, corners[(e + 1) % 4].1));
                            pts.push([pa[0] + (pb[0] - pa[0]) * t, pa[1] + (pb[1] - pa[1]) * t]);
                        }
                    }
                    for seg in pts.as_chunks::<2>().0 {
                        painter.line_segment([to_screen(d, area, seg[0]), to_screen(d, area, seg[1])], stroke);
                    }
                    k += 1;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests;
