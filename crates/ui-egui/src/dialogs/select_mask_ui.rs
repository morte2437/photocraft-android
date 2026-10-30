//! Select › Select and Mask…: Photoshop's Select and Mask workspace (#7069, #8148, TOOL-214-1), a
//! full-window workspace like Content-Aware Fill's.
//!
//! Laid out as Photoshop's: the tool's options bar across the top, the tools on the left (Quick
//! Selection W, Refine Edge Brush R, Brush B, Object Selection W, Lasso and Polygonal Lasso L in
//! one slot, Hand H, Zoom Z; Shift+W and Shift+L cycle), the document under its tab in the middle,
//! shown in the chosen view, and the Properties panel on the right: Remember Settings, View Mode
//! (the views with their letters, Show Edge J, Show Original P, Real-Time Refinement, High Quality
//! Preview, Transparency or Opacity, Overlay's colour and what it indicates), Refine Mode, Edge
//! Detection, Global Refinements (with Clear Selection and Invert) and Output Settings, then
//! Reset, OK and Cancel. The view keys (O M V A T K Y, F cycles, hold X for the image) work as in
//! the dialog this replaces.
//!
//! What the workspace holds is plain data: the tools' edits (`select.refineEdge`'s `edits`), the
//! Refine Edge Brush strokes (`refineStrokes`) and the settings. OK runs `select.refineEdge` with
//! them, one history step; Cancel drops them; Ctrl+Z undoes the last edit or stroke. The preview
//! runs the same edits ([`photocraft_engine::select_mask_edits`]) on a reduced copy (full size
//! with High Quality Preview) in the background and keeps the selection after the edits it has
//! applied, so a new stroke costs only that stroke; then it shows the refined selection in the
//! view ([`crate::select_mask_view::preview_proxy`]). With Real-Time Refinement the stroke being
//! drawn joins the preview while the pointer moves; without, when it is released.
//!
//! Not here yet: Refine Hair (#2671) and Object Aware (a learned model, #2788) are shown but greyed
//! out, and Photoshop's Presets.

use std::sync::Arc;

use egui::{Align2, Color32, FontId, Pos2, Rect as ERect, Sense, Stroke, TextureHandle, pos2, vec2};
use photocraft_doc::{DocId, Document, LayerId};
use photocraft_engine::select_mask_edits::{self, MAX_EDIT_POINTS, MAX_EDITS};
use photocraft_engine::smartselect_cmds::{MAX_STROKE_POINTS, MAX_STROKES};
use photocraft_geom::Rect;
use photocraft_raster::Surface;
use serde_json::{Map, Value, json};

use crate::PhotocraftApp;
use crate::canvas::ToolEvent;
use crate::select_mask_view::Mode;
use crate::theme::Tokens;
use crate::widgets;

/// Longest side of the reduced copy the preview runs on (px), unless High Quality Preview.
const PROXY_SIDE: usize = 1600;
const LEFT_W: f32 = 48.0;
const BAR_H: f32 = 36.0;
const TAB_H: f32 = 28.0;
const PANEL_W: f32 = 350.0;
const UNDO_LIMIT: usize = 100;
/// Marching ants with more edges than this are not drawn.
const MAX_ANT_EDGES: usize = 200_000;
/// Bound the temporary dense mask used for marching ants.
const MAX_ANT_PIXELS: usize = 4_000_000;
/// The `prefs.dialogs` key under which Remember Settings keeps the settings.
const REMEMBERED: &str = "select.refineEdge.workspace";
const CMD: &str = crate::select_mask_view::COMMAND;
/// The Output To targets, in Photoshop's order.
const OUTPUTS: [&str; 6] = ["selection", "layerMask", "newLayer", "newLayerWithMask", "newDocument", "newDocumentWithMask"];
/// Photoshop's brush sizes (px).
const SIZES: std::ops::RangeInclusive<f64> = 1.0..=5000.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SmTool {
    Quick,
    RefineEdge,
    Brush,
    Object,
    Lasso,
    PolygonalLasso,
    Hand,
    Zoom,
}

impl SmTool {
    fn name(self) -> &'static str {
        match self {
            SmTool::Quick => "quick",
            SmTool::RefineEdge => "refineEdge",
            SmTool::Brush => "brush",
            SmTool::Object => "object",
            SmTool::Lasso => "lasso",
            SmTool::PolygonalLasso => "polygonalLasso",
            SmTool::Hand => "hand",
            SmTool::Zoom => "zoom",
        }
    }

    fn from_name(s: &str) -> Option<SmTool> {
        Some(match s {
            "quick" => SmTool::Quick,
            "refineEdge" => SmTool::RefineEdge,
            "brush" => SmTool::Brush,
            "object" => SmTool::Object,
            "lasso" => SmTool::Lasso,
            "polygonalLasso" => SmTool::PolygonalLasso,
            "hand" => SmTool::Hand,
            "zoom" => SmTool::Zoom,
            _ => return None,
        })
    }

    fn icon(self) -> &'static str {
        match self {
            SmTool::Quick => "quick-selection",
            SmTool::RefineEdge => "brush-cleaning",
            SmTool::Brush => "brush",
            SmTool::Object => "square-dashed-mouse-pointer",
            SmTool::Lasso => "lasso",
            SmTool::PolygonalLasso => "pentagon",
            SmTool::Hand => "hand",
            SmTool::Zoom => "zoom-in",
        }
    }

    fn tip(self) -> String {
        let (label, key) = match self {
            SmTool::Quick => (tl!("Quick Selection Tool"), "W"),
            SmTool::RefineEdge => (tl!("Refine Edge Brush Tool"), "R"),
            SmTool::Brush => (tl!("Brush Tool"), "B"),
            SmTool::Object => (tl!("Object Selection Tool"), "W"),
            SmTool::Lasso => (tl!("Lasso Tool"), "L"),
            SmTool::PolygonalLasso => (tl!("Polygonal Lasso Tool"), "L"),
            SmTool::Hand => (tl!("Hand Tool"), "H"),
            SmTool::Zoom => (tl!("Zoom Tool"), "Z"),
        };
        format!("{label} ({key})")
    }

    /// Paints with a round brush (size from `[` and `]`).
    fn brushes(self) -> bool {
        matches!(self, SmTool::Quick | SmTool::RefineEdge | SmTool::Brush)
    }

    /// Adds to or subtracts from the selection (the options bar's + and −; Alt swaps them).
    fn edits(self) -> bool {
        !matches!(self, SmTool::Hand | SmTool::Zoom)
    }
}

/// The Properties panel's settings (kept between uses with Remember Settings).
#[derive(Clone, Debug, PartialEq)]
pub struct SmOpts {
    pub view: Mode,
    /// Each view's Transparency / Opacity (percent), in [`Mode::ALL`] order.
    pub opacity: [f32; 7],
    pub show_edge: bool,
    pub show_original: bool,
    /// Real-Time Refinement: the stroke being drawn joins the preview while the pointer moves.
    pub real_time: bool,
    /// High Quality Preview: the preview runs on the document at full size.
    pub high_quality: bool,
    /// Overlay's colour and whether it marks the selected areas (else the masked ones).
    pub overlay_color: [u8; 3],
    pub indicates_selected: bool,
    pub radius: f32,
    pub smart_radius: bool,
    pub smooth: f32,
    pub feather: f32,
    pub contrast: f32,
    pub shift_edge: f32,
    pub decontaminate: bool,
    pub amount: f32,
    pub output: String,
    pub remember: bool,
}

impl Default for SmOpts {
    fn default() -> Self {
        SmOpts {
            view: Mode::OnionSkin,
            opacity: Mode::ALL.map(|m| m.default_opacity().unwrap_or(100.0)),
            show_edge: false,
            show_original: false,
            real_time: true,
            high_quality: false,
            overlay_color: [255, 0, 0],
            indicates_selected: false,
            radius: 0.0,
            smart_radius: false,
            smooth: 0.0,
            feather: 0.0,
            contrast: 0.0,
            shift_edge: 0.0,
            decontaminate: false,
            amount: 100.0,
            output: "selection".into(),
            remember: false,
        }
    }
}

fn mode_index(m: Mode) -> usize {
    Mode::ALL.iter().position(|x| *x == m).unwrap_or(0)
}

impl SmOpts {
    /// Applies the settings named in `v` (control channel and remembered keys); others stay.
    pub fn apply(&mut self, v: &Value) -> Result<(), String> {
        let flag = |k: &str| v.get(k).and_then(Value::as_bool);
        for (k, slot) in [
            ("showEdge", &mut self.show_edge),
            ("showOriginal", &mut self.show_original),
            ("realTimeRefinement", &mut self.real_time),
            ("highQualityPreview", &mut self.high_quality),
            ("smartRadius", &mut self.smart_radius),
            ("decontaminate", &mut self.decontaminate),
            ("remember", &mut self.remember),
        ] {
            if let Some(b) = flag(k) {
                *slot = b;
            }
        }
        for (k, slot, lo, hi) in [
            ("radius", &mut self.radius, 0.0, 250.0),
            ("smooth", &mut self.smooth, 0.0, 100.0),
            ("feather", &mut self.feather, 0.0, 1000.0),
            ("contrast", &mut self.contrast, 0.0, 100.0),
            ("shiftEdge", &mut self.shift_edge, -100.0, 100.0),
            ("amount", &mut self.amount, 0.0, 100.0),
        ] {
            if let Some(n) = v.get(k).and_then(Value::as_f64).filter(|n| n.is_finite()) {
                *slot = (n as f32).clamp(lo, hi);
            }
        }
        if let Some(m) = v.get("view").and_then(Value::as_str) {
            self.view = Mode::from_id(m).ok_or_else(|| format!("bad view `{m}` ({})", Mode::ALL.map(|m| m.id()).join("|")))?;
        }
        match v.get("opacity") {
            // A number: the current view's; an object: per view.
            Some(Value::Number(n)) => {
                if let Some(n) = n.as_f64().filter(|n| n.is_finite()) {
                    self.opacity[mode_index(self.view)] = (n as f32).clamp(0.0, 100.0);
                }
            }
            Some(Value::Object(o)) => {
                for (k, n) in o {
                    if let (Some(m), Some(n)) = (Mode::from_id(k), n.as_f64().filter(|n| n.is_finite())) {
                        self.opacity[mode_index(m)] = (n as f32).clamp(0.0, 100.0);
                    }
                }
            }
            _ => {}
        }
        if let Some(c) = v.get("overlayColor").and_then(Value::as_array).filter(|c| c.len() == 3) {
            let ch = |i: usize| c.get(i).and_then(Value::as_u64).map(|v| v.min(255) as u8);
            if let (Some(r), Some(g), Some(b)) = (ch(0), ch(1), ch(2)) {
                self.overlay_color = [r, g, b];
            }
        }
        match v.get("indicates").and_then(Value::as_str) {
            None => {}
            Some("masked") => self.indicates_selected = false,
            Some("selected") => self.indicates_selected = true,
            Some(s) => return Err(format!("bad indicates `{s}` (masked|selected)")),
        }
        if let Some(o) = v.get("output").and_then(Value::as_str) {
            if !OUTPUTS.contains(&o) {
                return Err(format!("bad output `{o}` ({})", OUTPUTS.join("|")));
            }
            self.output = o.to_string();
        }
        self.follow_decontaminate();
        Ok(())
    }

    /// Decontaminating needs new pixels, so (as in Photoshop) it outputs to a new layer or document.
    fn follow_decontaminate(&mut self) {
        if self.decontaminate && matches!(self.output.as_str(), "selection" | "layerMask") {
            self.output = "newLayerWithMask".into();
        }
    }

    /// The settings as [`SmOpts::apply`] reads them.
    pub fn to_json(&self) -> Value {
        let opacity: Map<String, Value> = Mode::ALL.iter().zip(self.opacity).map(|(m, o)| (m.id().to_string(), json!(o))).collect();
        json!({
            "view": self.view.id(),
            "opacity": opacity,
            "showEdge": self.show_edge,
            "showOriginal": self.show_original,
            "realTimeRefinement": self.real_time,
            "highQualityPreview": self.high_quality,
            "overlayColor": self.overlay_color,
            "indicates": if self.indicates_selected { "selected" } else { "masked" },
            "radius": self.radius,
            "smartRadius": self.smart_radius,
            "smooth": self.smooth,
            "feather": self.feather,
            "contrast": self.contrast,
            "shiftEdge": self.shift_edge,
            "decontaminate": self.decontaminate,
            "amount": self.amount,
            "output": self.output,
            "remember": self.remember,
        })
    }

    /// `select.refineEdge`'s refinement and output params.
    fn refine_params(&self) -> Map<String, Value> {
        let mut p = Map::new();
        p.insert("radius".into(), json!(self.radius));
        p.insert("smartRadius".into(), json!(self.smart_radius));
        p.insert("smooth".into(), json!(self.smooth));
        p.insert("feather".into(), json!(self.feather));
        p.insert("contrast".into(), json!(self.contrast));
        p.insert("shiftEdge".into(), json!(self.shift_edge));
        p.insert("decontaminate".into(), json!(self.decontaminate));
        p.insert("amount".into(), json!(self.amount));
        p.insert("output".into(), json!(self.output));
        p
    }

    /// The preview's view (`select_mask_view`'s `__view`).
    fn view_params(&self) -> Value {
        let c = self.overlay_color.map(|v| f32::from(v) / 255.0);
        json!({
            "mode": self.view.id(),
            "opacity": self.opacity[mode_index(self.view)] / 100.0,
            "showEdge": self.show_edge,
            "showOriginal": self.show_original,
            "color": c,
            "indicates": if self.indicates_selected { "selected" } else { "masked" },
        })
    }
}

/// One undoable step.
#[derive(Clone, Debug)]
enum Step {
    Edit(Value),
    Stroke(Value),
    /// A reset, with what it replaced.
    State {
        edits: Vec<Value>,
        strokes: Vec<Value>,
        opts: Box<SmOpts>,
    },
}

/// A press-and-drag on the view with a tool, in document px.
#[derive(Clone, Debug)]
struct Gesture {
    tool: SmTool,
    add: bool,
    points: Vec<[f64; 2]>,
}

/// The selection after the first `edits` (document coordinates) on the copy reduced by `k`.
#[derive(Clone)]
struct BaseCache {
    k: u32,
    /// The document revision the base was computed from.
    revision: u64,
    edits: Vec<Value>,
    base: Option<Surface>,
}

/// A computed preview, ready to upload.
struct Shown {
    image: egui::ColorImage,
    /// Marching ants (document px) in the Marching Ants view.
    ants: Option<Vec<[[f32; 2]; 2]>>,
    /// Reduced-copy pixels at least half selected.
    selected: usize,
    /// How many points of the stroke being drawn it includes ([`usize::MAX`]: every stroke is in).
    trail: usize,
    /// The document px the image covers (the reduced copy's size × `k`).
    covers: [f32; 2],
}

#[cfg(not(target_arch = "wasm32"))]
type Outcome = Result<(Shown, BaseCache, Arc<Document>), String>;

/// The preview being computed or shown.
#[derive(Default)]
struct Preview {
    /// Bumped whenever the state changes; a result for an older generation is dropped.
    want: u64,
    #[cfg(not(target_arch = "wasm32"))]
    running: Option<(u64, u64, std::sync::mpsc::Receiver<Outcome>, photocraft_engine::jobs::JobCtx)>,
    shown: u64,
    pending: Option<Shown>,
    tex: Option<TextureHandle>,
    ants: Option<Vec<[[f32; 2]; 2]>>,
    covers: [f32; 2],
    selected: usize,
    /// [`Shown::trail`] of the preview shown.
    trail: usize,
    cache: Option<BaseCache>,
    /// The reduced copy of the document (its factor and the revision it was made from).
    proxy: Option<(u32, u64, Arc<Document>)>,
    error: Option<String>,
}

pub struct SelectMaskWorkspace {
    /// The document and layer the workspace was opened on: its work applies to them only.
    layer: Option<LayerId>,
    pub(crate) document: DocId,
    canvas: Rect,
    /// The document revision the textures were made from.
    revision: u64,
    /// The targeted layer mask is what is refined (`"target": "mask"`).
    target_mask: bool,
    pub opts: SmOpts,
    pub tool: SmTool,
    /// The tools W and L pick: the last of each slot used.
    w_tool: SmTool,
    l_tool: SmTool,
    pub add: bool,
    pub quick_size: f64,
    pub refine_size: f64,
    pub brush_size: f64,
    pub hardness: f32,
    pub sample_all: bool,
    pub edits: Vec<Value>,
    pub strokes: Vec<Value>,
    undo: Vec<Step>,
    redo: Vec<Step>,
    gesture: Option<Gesture>,
    /// The stroke last drawn, until a preview with it arrives (its newest part is drawn over the
    /// preview meanwhile, so painting looks continuous).
    trail: Option<Gesture>,
    /// The Polygonal Lasso's vertices so far, and whether it adds.
    poly: Option<(bool, Vec<[f64; 2]>)>,
    /// X is held: the image without the view.
    hide_view: bool,
    /// Document px per reduced-copy px (High Quality Preview uses 1).
    k: u32,
    pressed: bool,
    /// The device's texture side limit, as the context reports it each frame.
    texture_limit: usize,
    image_tex: Option<TextureHandle>,
    zoom: f32,
    center: [f64; 2],
    hover: Option<[f64; 2]>,
    preview: Preview,
    pub error: Option<String>,
}

impl SelectMaskWorkspace {
    pub fn describe(&self) -> Value {
        json!({
            "tool": self.tool.name(),
            "mode": if self.add { "add" } else { "subtract" },
            "size": self.size(),
            "hardness": self.hardness,
            "sampleAllLayers": self.sample_all,
            "edits": self.edits.len(),
            "strokes": self.strokes.len(),
            "undo": self.undo.len(),
            "redo": self.redo.len(),
            "polygon": self.poly.as_ref().map_or(0, |(_, p)| p.len()),
            "settings": self.opts.to_json(),
            "proxyScale": self.proxy_k(),
            "selectedProxyPixels": self.preview.selected,
            "previewReady": (self.preview.tex.is_some() || self.preview.pending.is_some()) && self.preview.shown == self.preview.want,
            "previewError": self.preview.error,
            "error": self.error,
        })
    }

    /// The factor the preview runs at.
    fn proxy_k(&self) -> u32 {
        if self.opts.high_quality { 1 } else { self.k }
    }

    /// The current brush tool's size (Quick Selection, Refine Edge Brush, Brush; else the brush's).
    fn size(&self) -> f64 {
        match self.tool {
            SmTool::Quick => self.quick_size,
            SmTool::RefineEdge => self.refine_size,
            _ => self.brush_size,
        }
    }

    fn set_size(&mut self, v: f64) {
        let v = if v.is_finite() { v.round().clamp(*SIZES.start(), *SIZES.end()) } else { return };
        match self.tool {
            SmTool::Quick => self.quick_size = v,
            SmTool::RefineEdge => self.refine_size = v,
            SmTool::Brush => self.brush_size = v,
            _ => {}
        }
    }

    fn pick(&mut self, t: SmTool) {
        self.tool = t;
        match t {
            SmTool::Quick | SmTool::Object => self.w_tool = t,
            SmTool::Lasso | SmTool::PolygonalLasso => self.l_tool = t,
            _ => {}
        }
        if t != SmTool::PolygonalLasso {
            self.poly = None;
        }
    }

    /// The preview is out of date.
    fn stale(&mut self) {
        self.preview.want += 1;
    }

    fn push(&mut self, s: Step) {
        self.redo.clear();
        if self.undo.len() == UNDO_LIMIT {
            self.undo.remove(0);
        }
        self.undo.push(s);
    }

    fn points_in(list: &[Value]) -> usize {
        list.iter().map(|e| e.get("points").and_then(Value::as_array).map_or(0, Vec::len)).sum()
    }

    /// An item the engine's limits refuse: say why, and drop what was drawn for it.
    fn reject(&mut self, why: String) {
        self.error = Some(why);
        self.trail = None;
        self.stale();
    }

    /// Adds a tool edit (undoable), within the engine's limits.
    fn add_edit(&mut self, e: Value) {
        if self.edits.len() >= MAX_EDITS {
            self.reject(format!("at most {MAX_EDITS} edits: undo some, or reset"));
            return;
        }
        if Self::points_in(&self.edits) + Self::points_in(std::slice::from_ref(&e)) > MAX_EDIT_POINTS {
            self.reject(format!("at most {MAX_EDIT_POINTS} points in all edits: undo some, or reset"));
            return;
        }
        self.error = None;
        self.edits.push(e.clone());
        self.push(Step::Edit(e));
        self.stale();
    }

    /// Adds a Refine Edge Brush stroke (undoable), within the engine's limits.
    fn add_stroke(&mut self, s: Value) {
        if self.strokes.len() >= MAX_STROKES {
            self.reject(format!("at most {MAX_STROKES} Refine Edge Brush strokes: undo some, or reset"));
            return;
        }
        if Self::points_in(&self.strokes) + Self::points_in(std::slice::from_ref(&s)) > MAX_STROKE_POINTS {
            self.reject(format!("at most {MAX_STROKE_POINTS} points in all Refine Edge Brush strokes: undo some, or reset"));
            return;
        }
        self.error = None;
        self.strokes.push(s.clone());
        self.push(Step::Stroke(s));
        self.stale();
    }

    /// What `g` adds when it ends: a tool edit or a Refine Edge Brush stroke (`None`: nothing).
    fn finish(&self, g: &Gesture) -> Option<Result<Value, Value>> {
        let round = |p: &[f64; 2]| json!([(p[0] * 10.0).round() / 10.0, (p[1] * 10.0).round() / 10.0]);
        let pts: Vec<Value> = g.points.iter().map(round).collect();
        let mode = if g.add { "add" } else { "subtract" };
        let first = g.points.first()?;
        Some(match g.tool {
            SmTool::Quick => Ok(json!({"tool": "quick", "points": pts, "size": self.quick_size, "mode": mode, "sampleAllLayers": self.sample_all})),
            SmTool::Brush => Ok(json!({"tool": "brush", "points": pts, "size": self.brush_size, "hardness": self.hardness, "mode": mode})),
            SmTool::RefineEdge => Err(json!({"points": pts, "size": self.refine_size, "mode": mode})),
            SmTool::Lasso | SmTool::PolygonalLasso if pts.len() >= 3 => Ok(json!({"tool": "lasso", "points": pts, "mode": mode})),
            SmTool::Object => {
                let last = g.points.last().unwrap_or(first);
                let (w, h) = (last[0] - first[0], last[1] - first[1]);
                // A click (or a drag of a couple of pixels) picks the object under the pointer.
                if w.abs().max(h.abs()) * f64::from(self.zoom.max(0.01)) < 4.0 {
                    Ok(json!({"tool": "object", "point": round(first), "mode": mode, "sampleAllLayers": self.sample_all}))
                } else {
                    let r = |v: f64| (v * 10.0).round() / 10.0;
                    Ok(json!({"tool": "object", "rect": [r(first[0]), r(first[1]), r(w), r(h)], "mode": mode, "sampleAllLayers": self.sample_all}))
                }
            }
            _ => return None,
        })
    }

    fn end_gesture(&mut self) {
        let Some(g) = self.gesture.take() else { return };
        self.trail = Some(g.clone());
        match self.finish(&g) {
            Some(Ok(e)) => self.add_edit(e),
            Some(Err(s)) => self.add_stroke(s),
            None => self.stale(),
        }
    }

    /// The Polygonal Lasso's polygon closes into an edit.
    fn close_polygon(&mut self) {
        if let Some((add, points)) = self.poly.take()
            && points.len() >= 3
        {
            self.gesture = Some(Gesture { tool: SmTool::PolygonalLasso, add, points });
            self.end_gesture();
        }
    }

    fn begin(&mut self, p: [f64; 2], alt: bool) {
        let add = self.add != alt;
        match self.tool {
            SmTool::PolygonalLasso => {
                let near = |q: &[f64; 2]| (q[0] - p[0]).hypot(q[1] - p[1]) * f64::from(self.zoom.max(0.01)) < 6.0;
                match &mut self.poly {
                    // Clicking the first vertex again closes the polygon.
                    Some((_, pts)) if pts.len() >= 3 && pts.first().is_some_and(near) => self.close_polygon(),
                    Some((_, pts)) => pts.push(p),
                    None => self.poly = Some((add, vec![p])),
                }
            }
            t if t.edits() => {
                self.gesture = Some(Gesture { tool: t, add, points: vec![p] });
                self.trail = None;
                self.preview.trail = 0;
            }
            _ => {}
        }
    }

    fn extend(&mut self, p: [f64; 2]) {
        let step = if self.tool.brushes() { (self.size() / 8.0).max(1.0) } else { 0.5 };
        let real_time = self.opts.real_time;
        let Some(g) = self.gesture.as_mut() else { return };
        match g.tool {
            // Object Selection keeps the press and the current corner.
            SmTool::Object => {
                g.points.truncate(1);
                g.points.push(p);
            }
            _ => {
                if g.points.last().is_some_and(|l| (l[0] - p[0]).hypot(l[1] - p[1]) < step) {
                    return;
                }
                g.points.push(p);
            }
        }
        if real_time {
            self.stale();
        }
    }
}

/// Undo the last edit, stroke or reset.
fn undo(w: &mut SelectMaskWorkspace) {
    w.end_gesture();
    w.poly = None;
    let Some(step) = w.undo.pop() else { return };
    let back = match step {
        Step::Edit(e) => {
            w.edits.pop();
            Step::Edit(e)
        }
        Step::Stroke(s) => {
            w.strokes.pop();
            Step::Stroke(s)
        }
        Step::State { edits, strokes, opts } => {
            let now = Step::State {
                edits: std::mem::replace(&mut w.edits, edits),
                strokes: std::mem::replace(&mut w.strokes, strokes),
                opts: Box::new(w.opts.clone()),
            };
            w.opts = *opts;
            now
        }
    };
    w.redo.push(back);
    w.error = None;
    w.stale();
}

fn redo(w: &mut SelectMaskWorkspace) {
    w.end_gesture();
    let Some(step) = w.redo.pop() else { return };
    let back = match step {
        Step::Edit(e) => {
            w.edits.push(e.clone());
            Step::Edit(e)
        }
        Step::Stroke(s) => {
            w.strokes.push(s.clone());
            Step::Stroke(s)
        }
        Step::State { edits, strokes, opts } => {
            let now = Step::State {
                edits: std::mem::replace(&mut w.edits, edits),
                strokes: std::mem::replace(&mut w.strokes, strokes),
                opts: Box::new(w.opts.clone()),
            };
            w.opts = *opts;
            now
        }
    };
    w.undo.push(back);
    w.error = None;
    w.stale();
}

/// Reset (the panel's ↺): the settings back to Photoshop's defaults and the edits and strokes
/// dropped, as one undoable step.
fn reset(w: &mut SelectMaskWorkspace) {
    w.end_gesture();
    w.poly = None;
    let opts = SmOpts { remember: w.opts.remember, ..SmOpts::default() };
    let prev =
        Step::State { edits: std::mem::take(&mut w.edits), strokes: std::mem::take(&mut w.strokes), opts: Box::new(std::mem::replace(&mut w.opts, opts)) };
    w.push(prev);
    w.error = None;
    w.stale();
}

/// Opens the workspace on the selection (or the active layer's mask).
pub fn open(app: &mut PhotocraftApp) -> Result<(), String> {
    // Opening again would drop the open workspace's work: edit it with `ui`, or cancel it.
    if app.distort.select_mask.is_some() {
        return Err("Select and Mask is already open".into());
    }
    if let Some(Err(why)) = photocraft_engine::commands::find(CMD).map(|c| (c.enabled)(&app.session)) {
        return Err(format!("Select and Mask needs a selection or a layer mask ({why})"));
    }
    // A targeted layer mask is what is refined (Photoshop opens on it from the mask's thumbnail).
    let target_mask = app.layer_mask_targeted();
    let st = app.session.active().ok_or("no document")?;
    let document = st.doc.id;
    let layer = st.active_layer.filter(|id| st.doc.layer(*id).is_some());
    let canvas = st.doc.bounds();
    let revision = st.revision;
    let mask_only = st.doc.selection.is_none() && layer.and_then(|id| st.doc.layer(id)).is_some_and(|l| l.mask.is_some());
    let k = u32::try_from((canvas.width().max(canvas.height()) as usize).div_ceil(PROXY_SIDE).max(1)).unwrap_or(u32::MAX);
    let mut opts = SmOpts::default();
    match app.session.prefs().dialogs.get(REMEMBERED) {
        Some(saved) if saved.get("remember").and_then(Value::as_bool) == Some(true) => {
            let _ = opts.apply(saved);
        }
        _ if mask_only || target_mask => opts.output = "layerMask".into(),
        _ => {}
    }
    let mut w = SelectMaskWorkspace {
        layer,
        document,
        canvas,
        revision,
        target_mask,
        opts,
        tool: SmTool::Quick,
        w_tool: SmTool::Quick,
        l_tool: SmTool::Lasso,
        add: true,
        quick_size: 30.0,
        refine_size: 30.0,
        brush_size: 30.0,
        hardness: 100.0,
        sample_all: false,
        edits: Vec::new(),
        strokes: Vec::new(),
        undo: Vec::new(),
        redo: Vec::new(),
        gesture: None,
        trail: None,
        poly: None,
        hide_view: false,
        k,
        pressed: false,
        texture_limit: 2048,
        image_tex: None,
        zoom: 0.0,
        center: [f64::from(canvas.x0 + canvas.x1) / 2.0, f64::from(canvas.y0 + canvas.y1) / 2.0],
        hover: None,
        preview: Preview::default(),
        error: None,
    };
    w.stale();
    app.distort.select_mask = Some(w);
    Ok(())
}

/// Why the open workspace's work no longer applies: the active document or layer is not the one it
/// was opened on (another tab, or another layer selected by a shortcut or the control channel).
/// `select.refineEdge` works on the active document and layer, so its edits must not run there.
pub(crate) fn stale(app: &PhotocraftApp) -> Option<&'static str> {
    let w = app.distort.select_mask.as_ref()?;
    let Some(st) = app.session.active().filter(|st| st.doc.id == w.document) else {
        return Some(tl!("Select and Mask was closed: its document is no longer the active one"));
    };
    let layer = st.active_layer.filter(|id| st.doc.layer(*id).is_some());
    (layer != w.layer).then_some(tl!("Select and Mask was closed: another layer was selected"))
}

/// Closes a [`stale`] workspace (its work is dropped, the new target untouched): the reason.
fn drop_stale(app: &mut PhotocraftApp) -> Result<(), String> {
    match stale(app) {
        Some(why) => {
            close(app);
            app.ui.status = why.into();
            app.ui.status_error = true;
            Err(why.into())
        }
        None => Ok(()),
    }
}

/// The params OK runs `select.refineEdge` with.
fn commit_params(w: &SelectMaskWorkspace) -> Value {
    let mut p = w.opts.refine_params();
    p.insert("sampleAllLayers".into(), json!(w.sample_all));
    if !w.edits.is_empty() {
        p.insert("edits".into(), Value::Array(w.edits.clone()));
    }
    if !w.strokes.is_empty() {
        p.insert("refineStrokes".into(), Value::Array(w.strokes.clone()));
    }
    if w.target_mask {
        p.insert("target".into(), json!("mask"));
    }
    Value::Object(p)
}

fn remember(app: &mut PhotocraftApp, opts: &SmOpts) {
    let v = if opts.remember { opts.to_json() } else { json!({"remember": false}) };
    app.session.prefs.edit(|p| p.dialogs.insert(REMEMBERED.into(), v));
}

/// OK: runs `select.refineEdge` and closes. A failure leaves the workspace open with the error in
/// its panel, so nothing painted is lost.
pub fn commit(app: &mut PhotocraftApp) -> Result<Value, String> {
    drop_stale(app)?;
    let w = app.distort.select_mask.as_mut().ok_or("Select and Mask is not open")?;
    w.end_gesture();
    w.close_polygon();
    let (p, opts) = (commit_params(w), w.opts.clone());
    match app.run(CMD, p) {
        Ok(v) => {
            remember(app, &opts);
            close(app);
            Ok(v)
        }
        Err(e) => {
            if let Some(w) = app.distort.select_mask.as_mut() {
                w.error = Some(e.clone());
            }
            app.ui.status = e.clone();
            app.ui.status_error = true;
            Err(e)
        }
    }
}

/// Cancel (button, Esc): closes; the document is untouched.
pub fn cancel(app: &mut PhotocraftApp) {
    close(app);
}

fn close(app: &mut PhotocraftApp) {
    if let Some(w) = app.distort.select_mask.take() {
        cancel_preview(w);
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn cancel_preview(w: SelectMaskWorkspace) {
    if let Some((_, _, _, job)) = w.preview.running {
        job.cancel();
    }
}

#[cfg(target_arch = "wasm32")]
fn cancel_preview(_: SelectMaskWorkspace) {}

/// Enter: closes a Polygonal Lasso polygon, else OK. True when it closed a polygon.
pub fn enter(app: &mut PhotocraftApp) -> bool {
    let Some(w) = app.distort.select_mask.as_mut() else { return false };
    if w.poly.is_some() {
        w.close_polygon();
        return true;
    }
    let _ = commit(app);
    false
}

/// Esc: drops a Polygonal Lasso polygon being drawn, else Cancel.
pub fn escape(app: &mut PhotocraftApp) {
    match app.distort.select_mask.as_mut() {
        Some(w) if w.poly.is_some() => w.poly = None,
        _ => cancel(app),
    }
}

/// Control channel: `select.selectAndMask {"ui": {...}}` while the workspace is open.
pub fn control(app: &mut PhotocraftApp, ui: &Value) -> Result<Value, String> {
    let flag = |k: &str| ui.get(k).and_then(Value::as_bool) == Some(true);
    if flag("cancel") {
        cancel(app);
        return Ok(json!({"cancelled": true}));
    }
    drop_stale(app)?;
    {
        let w = app.distort.select_mask.as_mut().ok_or("Select and Mask is not open")?;
        let before = w.opts.clone();
        // Everything is read before anything changes, so rejected params change nothing.
        let mut o = w.opts.clone();
        o.apply(ui)?;
        let tool = match ui.get("tool").and_then(Value::as_str) {
            Some(t) => Some(SmTool::from_name(t).ok_or(format!("bad tool `{t}` (quick|refineEdge|brush|object|lasso|polygonalLasso|hand|zoom)"))?),
            None => None,
        };
        let add = match ui.get("mode").and_then(Value::as_str) {
            None => None,
            Some("add") => Some(true),
            Some("subtract") => Some(false),
            Some(m) => return Err(format!("bad mode `{m}` (add|subtract)")),
        };
        let stroke = match ui.get("stroke") {
            Some(s) => {
                let pts: Vec<[f64; 2]> =
                    serde_json::from_value(s.get("points").cloned().unwrap_or(Value::Null)).map_err(|e| format!("bad stroke points: {e}"))?;
                if pts.iter().any(|p| !p[0].is_finite() || !p[1].is_finite()) {
                    return Err("bad stroke points: not finite".into());
                }
                Some((pts, s.get("alt").and_then(Value::as_bool).unwrap_or(false)))
            }
            None => None,
        };
        if let Some(t) = tool {
            w.pick(t);
        }
        w.opts = o;
        if let Some(add) = add {
            w.add = add;
        }
        if let Some(n) = ui.get("size").and_then(Value::as_f64) {
            w.set_size(n);
        }
        if let Some(n) = ui.get("hardness").and_then(Value::as_f64).filter(|n| n.is_finite()) {
            w.hardness = (n as f32).clamp(0.0, 100.0);
        }
        if let Some(b) = ui.get("sampleAllLayers").and_then(Value::as_bool) {
            w.sample_all = b;
        }
        if let Some(b) = ui.get("hideView").and_then(Value::as_bool) {
            w.hide_view = b;
        }
        if w.opts != before {
            w.stale();
        }
        if let Some((pts, alt)) = stroke {
            if w.tool == SmTool::PolygonalLasso {
                w.poly = Some((w.add != alt, pts));
                w.close_polygon();
            } else if let Some((first, rest)) = pts.split_first() {
                w.begin(*first, alt);
                for p in rest {
                    w.extend(*p);
                }
                w.end_gesture();
            }
        }
        let sample_all = w.sample_all;
        for (key, edit) in [
            ("selectSubject", json!({"tool": "subject", "sampleAllLayers": sample_all})),
            ("clearSelection", json!({"tool": "clear"})),
            ("invert", json!({"tool": "invert"})),
        ] {
            if flag(key) {
                w.add_edit(edit);
            }
        }
        if flag("reset") {
            reset(w);
        }
        if flag("undo") {
            undo(w);
        }
        if flag("redo") {
            redo(w);
        }
    }
    if flag("waitPreview") {
        wait_preview(app)?;
    }
    if flag("commit") {
        return commit(app).map(|v| json!({"committed": true, "result": v}));
    }
    Ok(app.distort.select_mask.as_ref().map(SelectMaskWorkspace::describe).unwrap_or(Value::Null))
}

/// `e` (document coordinates) on the copy reduced by `k`: points, sizes and rectangles scale by
/// `1 / k`.
fn scale_edit(e: &Value, k: u32) -> Value {
    let k = f64::from(k.max(1));
    let mut e = e.clone();
    let Some(o) = e.as_object_mut() else { return e };
    let scale = |v: &mut Value| {
        if let Some(n) = v.as_f64() {
            *v = json!(n / k);
        }
    };
    if let Some(pts) = o.get_mut("points").and_then(Value::as_array_mut) {
        for p in pts.iter_mut().filter_map(Value::as_array_mut) {
            p.iter_mut().for_each(scale);
        }
    }
    for key in ["point", "rect"] {
        if let Some(a) = o.get_mut(key).and_then(Value::as_array_mut) {
            a.iter_mut().for_each(scale);
        }
    }
    if let Some(size) = o.get("size").and_then(Value::as_f64) {
        o.insert("size".into(), json!((size / k).max(1.0)));
    }
    e
}

/// What a preview computation needs (moved to a worker thread).
struct Job {
    doc: Arc<Document>,
    revision: u64,
    layer: Option<LayerId>,
    k: u32,
    target_mask: bool,
    /// The committed edits and, with Real-Time Refinement, the one being drawn.
    edits: Vec<Value>,
    extra: Option<Value>,
    /// The refinement params with `__view` and `refineStrokes`.
    params: Value,
    cache: Option<BaseCache>,
    proxy: Option<Arc<Document>>,
    display: Option<Arc<photocraft_engine::display_color::CanvasDisplay>>,
    texture_limit: usize,
    /// [`Shown::trail`] of this preview.
    trail: usize,
}

fn job(app: &PhotocraftApp) -> Option<Job> {
    if stale(app).is_some() {
        return None;
    }
    let w = app.distort.select_mask.as_ref()?;
    let st = app.session.active()?;
    let k = w.proxy_k();
    let mut params = w.opts.refine_params();
    params.insert("sampleAllLayers".into(), json!(w.sample_all));
    params.insert("__view".into(), w.opts.view_params());
    let gesture = w.gesture.as_ref().and_then(|g| w.finish(g));
    let mut strokes = w.strokes.clone();
    let mut extra = None;
    match gesture {
        Some(Ok(e)) if w.opts.real_time => extra = Some(e),
        Some(Err(s)) if w.opts.real_time => strokes.push(s),
        _ => {}
    }
    if !strokes.is_empty() {
        params.insert("refineStrokes".into(), Value::Array(strokes));
    }
    if w.target_mask {
        params.insert("target".into(), json!("mask"));
    }
    let proxy = w.preview.proxy.as_ref().filter(|(pk, rev, _)| *pk == k && *rev == st.revision).map(|(_, _, d)| Arc::clone(d));
    Some(Job {
        doc: Arc::clone(&st.doc),
        revision: st.revision,
        layer: w.layer,
        k,
        target_mask: w.target_mask,
        edits: w.edits.clone(),
        extra,
        params: Value::Object(params),
        cache: w.preview.cache.clone(),
        proxy,
        display: crate::canvas::canvas_display(app, &st.doc, None).0,
        texture_limit: w.texture_limit,
        trail: match &w.gesture {
            Some(g) if w.opts.real_time => g.points.len(),
            Some(_) => 0,
            None => usize::MAX,
        },
    })
}

/// Unit edges between selected (≥ ½) and unselected pixels of `m` (`w × h`, reduced by `k`), in
/// document px; `None` past [`MAX_ANT_EDGES`].
fn ant_edges(m: &[f32], w: usize, h: usize, k: f32) -> Option<Vec<[[f32; 2]; 2]>> {
    let at = |x: i64, y: i64| x >= 0 && y >= 0 && (x as usize) < w && (y as usize) < h && m.get(y as usize * w + x as usize).is_some_and(|v| *v >= 0.5);
    let mut out = Vec::new();
    for y in 0..h as i64 {
        for x in 0..w as i64 {
            if !at(x, y) {
                continue;
            }
            let (fx, fy) = (x as f32, y as f32);
            for (n, a, b) in [
                (at(x - 1, y), [fx, fy], [fx, fy + 1.0]),
                (at(x + 1, y), [fx + 1.0, fy], [fx + 1.0, fy + 1.0]),
                (at(x, y - 1), [fx, fy], [fx + 1.0, fy]),
                (at(x, y + 1), [fx, fy + 1.0], [fx + 1.0, fy + 1.0]),
            ] {
                if !n {
                    out.push([[a[0] * k, a[1] * k], [b[0] * k, b[1] * k]]);
                }
            }
            if out.len() > MAX_ANT_EDGES {
                return None;
            }
        }
    }
    Some(out)
}

/// Count selected pixels without allocating a canvas-sized mask. Missing tiles use the surface's
/// default value, including an inverted selection whose default is selected.
fn selected_pixels(selection: Option<&Surface>, area: Rect) -> usize {
    let Some(surface) = selection else { return 0 };
    let default_selected = surface.default_pixel().first().is_some_and(|v| *v >= 0.5);
    let area_pixels = (area.width() as usize).saturating_mul(area.height() as usize);
    let mut count = if default_selected { area_pixels } else { 0 };
    for (coord, _) in surface.tiles() {
        let tile_area = coord.rect().intersect(&area);
        if tile_area.is_empty() {
            continue;
        }
        let pixels = (tile_area.width() as usize).saturating_mul(tile_area.height() as usize);
        if default_selected {
            count = count.saturating_sub(pixels);
        }
        let values = photocraft_algo::selection::mask_from_surface(Some(surface), tile_area);
        count = count.saturating_add(values.iter().filter(|v| **v >= 0.5).count());
    }
    count
}

/// Generate ants only when the dense temporary mask fits the preview budget.
fn preview_ants(selection: Option<&Surface>, area: Rect, k: f32) -> Option<Vec<[[f32; 2]; 2]>> {
    let (w, h) = (area.width() as usize, area.height() as usize);
    if w.checked_mul(h).is_none_or(|n| n > MAX_ANT_PIXELS) {
        return sparse_preview_ants(selection?, area, k);
    }
    let coverage = photocraft_algo::selection::mask_from_surface(selection, area);
    ant_edges(&coverage, w, h, k)
}

/// Follow sparse selections on large canvases without scanning the empty canvas. A selected
/// default or too many stored pixels falls back to omitting ants, as for too many edges.
fn sparse_preview_ants(surface: &Surface, area: Rect, k: f32) -> Option<Vec<[[f32; 2]; 2]>> {
    if surface.default_pixel().first().is_some_and(|v| *v >= 0.5) {
        return None;
    }
    let mut work = 0usize;
    let mut edges = Vec::new();
    let mut sample = vec![0.0; surface.channels()];
    for (coord, _) in surface.tiles() {
        let tile = coord.rect().intersect(&area);
        if tile.is_empty() {
            continue;
        }
        work = work.checked_add((tile.width() as usize).checked_mul(tile.height() as usize)?)?;
        if work > MAX_ANT_PIXELS {
            return None;
        }
        let coverage = photocraft_algo::selection::mask_from_surface(Some(surface), tile);
        for y in tile.y0..tile.y1 {
            for x in tile.x0..tile.x1 {
                let index = (y - tile.y0) as usize * tile.width() as usize + (x - tile.x0) as usize;
                if !coverage.get(index).is_some_and(|v| *v >= 0.5) {
                    continue;
                }
                let (fx, fy) = ((x - area.x0) as f32, (y - area.y0) as f32);
                for (nx, ny, a, b) in [
                    (x - 1, y, [fx, fy], [fx, fy + 1.0]),
                    (x + 1, y, [fx + 1.0, fy], [fx + 1.0, fy + 1.0]),
                    (x, y - 1, [fx, fy], [fx + 1.0, fy]),
                    (x, y + 1, [fx, fy + 1.0], [fx + 1.0, fy + 1.0]),
                ] {
                    let selected = if nx < area.x0 || nx >= area.x1 || ny < area.y0 || ny >= area.y1 {
                        false
                    } else {
                        surface.read_pixel(nx, ny, &mut sample);
                        sample.first().is_some_and(|v| *v >= 0.5)
                    };
                    if !selected {
                        edges.push([[a[0] * k, a[1] * k], [b[0] * k, b[1] * k]]);
                        if edges.len() > MAX_ANT_EDGES {
                            return None;
                        }
                    }
                }
            }
        }
    }
    Some(edges)
}

/// The preview of `job`: the edits applied (from where the cache left off), the edge refined and
/// the result shown in the view, as an image no wider than the device allows.
fn compute(job: Job, cancel: Option<&photocraft_engine::jobs::JobCtx>) -> Result<(Shown, BaseCache, Arc<Document>), String> {
    let k = job.k.max(1);
    let proxy = job.proxy.unwrap_or_else(|| Arc::new(crate::proxy::proxy_document(&job.doc, k)));
    let mut doc = (*proxy).clone();
    let layer = job.layer.filter(|id| doc.layer(*id).is_some());
    let mut params = job.params;
    let start = || {
        let mask = layer.and_then(|id| doc.layer(id)).and_then(|l| l.mask.as_ref()).map(|m| m.surface.clone());
        if job.target_mask { mask.or_else(|| doc.selection.clone()) } else { doc.selection.clone().or(mask) }
    };
    let scaled = |list: &[Value]| -> Result<Vec<select_mask_edits::SelectionEdit>, String> {
        let list: Vec<Value> = list.iter().map(|e| scale_edit(e, k)).collect();
        select_mask_edits::parse(&json!({ "edits": list })).map_err(|e| e.to_string())
    };
    // With no edits yet, the selection Select and Mask starts from is where the first one starts.
    let mut cache = BaseCache { k, revision: job.revision, edits: Vec::new(), base: start() };
    if !job.edits.is_empty() || job.extra.is_some() {
        // The selection after the edits already applied, when they still lead the list.
        let (from, done) = match job.cache.filter(|c| c.k == k && c.revision == job.revision && job.edits.starts_with(&c.edits)) {
            Some(c) => (c.base, c.edits.len()),
            None => (start(), 0),
        };
        let rest = scaled(job.edits.get(done..).unwrap_or_default())?;
        let base = select_mask_edits::apply(&doc, layer, from, &rest);
        cache = BaseCache { k, revision: job.revision, edits: job.edits.clone(), base: base.clone() };
        let base = match &job.extra {
            Some(e) => select_mask_edits::apply(&doc, layer, base, &scaled(std::slice::from_ref(e))?),
            None => base,
        };
        // Nothing selected is an empty selection, not "use the layer mask".
        doc.selection = Some(base.unwrap_or_else(|| Surface::new(photocraft_color::PixelFormat::GRAY8)));
        if let Some(o) = params.as_object_mut() {
            o.remove("target");
        }
    }
    let shown = crate::select_mask_view::preview_proxy(doc, layer, &params, k, cancel);
    if cancel.is_some_and(photocraft_engine::jobs::JobCtx::cancelled) {
        return Err("cancelled".into());
    }
    let shown = shown.ok_or("the preview failed")?;
    let (pw, ph) = (shown.size.width as usize, shown.size.height as usize);
    let f = crate::filter_preview_ui::texture_factor(shown.size.width, shown.size.height, job.texture_limit) as usize;
    let (tw, th) = (pw.div_ceil(f.max(1)), ph.div_ceil(f.max(1)));
    let buf = photocraft_compose::render_reduced(&shown, u32::try_from(tw).unwrap_or(1), u32::try_from(th).unwrap_or(1));
    let image = crate::canvas::display_image(job.display.as_deref(), &buf);
    let selected = selected_pixels(shown.selection.as_ref(), shown.bounds());
    let mode = params.get("__view").and_then(|v| v.get("mode")).and_then(Value::as_str);
    let ants = if mode == Some(Mode::MarchingAnts.id()) { preview_ants(shown.selection.as_ref(), shown.bounds(), k as f32) } else { None };
    let covers = [(pw as f32) * k as f32, (ph as f32) * k as f32];
    Ok((Shown { image, ants, selected, trail: job.trail, covers }, cache, proxy))
}

/// Takes the preview of state `generation` (made from document `revision`): the image when it is
/// still the state wanted (or a stroke is being drawn: Real-Time Refinement shows the latest one
/// it has), and the selection after the edits and the reduced copy for the next preview.
fn take_result(w: &mut SelectMaskWorkspace, generation: u64, revision: u64, r: Result<(Shown, BaseCache, Arc<Document>), String>) {
    // Made from pixels the document no longer has (even mid-stroke).
    if revision != w.revision {
        return;
    }
    let pv = &mut w.preview;
    match r {
        Ok((shown, cache, proxy)) => {
            pv.proxy = Some((cache.k, revision, proxy));
            pv.cache = Some(cache);
            if generation == pv.want || w.pressed {
                pv.trail = shown.trail;
                if shown.trail == usize::MAX {
                    w.trail = None;
                }
                pv.shown = generation;
                pv.selected = shown.selected;
                pv.error = None;
                pv.pending = Some(shown);
            }
        }
        Err(e) if e == "cancelled" => {}
        Err(e) => {
            pv.shown = generation;
            pv.error = Some(e);
        }
    }
}

/// Computes the preview now, on this thread (tests and the control channel).
fn wait_preview(app: &mut PhotocraftApp) -> Result<(), String> {
    drop_stale(app)?;
    follow_revision(app);
    let j = job(app).ok_or("Select and Mask is not open")?;
    let revision = j.revision;
    let r = compute(j, None);
    let w = app.distort.select_mask.as_mut().ok_or("Select and Mask is not open")?;
    let err = r.as_ref().err().cloned();
    let generation = w.preview.want;
    take_result(w, generation, revision, r);
    err.map_or(Ok(()), Err)
}

/// Starts a background preview when the shown one is out of date, and collects a finished one.
/// While a stroke is drawn with Real-Time Refinement, a running preview is let finish (cancelling
/// it on every move would never show one) and the next starts from the latest state.
#[cfg(not(target_arch = "wasm32"))]
fn drive_preview(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let Some(w) = app.distort.select_mask.as_mut() else { return };
    if let Some((generation, revision, rx, _)) = &w.preview.running {
        match rx.try_recv() {
            Ok(r) => {
                let (generation, revision) = (*generation, *revision);
                w.preview.running = None;
                take_result(w, generation, revision, r);
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {
                if *generation != w.preview.want
                    && !w.pressed
                    && let Some((_, _, _, job)) = &w.preview.running
                {
                    job.cancel();
                }
                ctx.request_repaint_after(std::time::Duration::from_millis(30));
                return;
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => w.preview.running = None,
        }
    }
    let Some(w) = app.distort.select_mask.as_ref() else { return };
    let waiting = w.pressed && !w.opts.real_time;
    if w.preview.running.is_some() || w.preview.shown == w.preview.want || waiting {
        return;
    }
    let generation = w.preview.want;
    let Some(j) = job(app) else { return };
    let revision = j.revision;
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    let ctl = photocraft_engine::jobs::JobCtx::new();
    let work = ctl.clone();
    let repaint = ctx.clone();
    let spawned = std::thread::Builder::new().name("select-mask-preview".into()).spawn(move || {
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| compute(j, Some(&work)))).unwrap_or_else(|_| Err("the preview failed".into()));
        let _ = tx.send(r);
        repaint.request_repaint();
    });
    let Some(w) = app.distort.select_mask.as_mut() else { return };
    match spawned {
        Ok(_) => w.preview.running = Some((generation, revision, rx, ctl)),
        Err(e) => w.preview.error = Some(format!("could not start the preview: {e}")),
    }
}

/// The web build has no threads: the preview is computed when the pointer comes up.
#[cfg(target_arch = "wasm32")]
fn drive_preview(app: &mut PhotocraftApp, _ctx: &egui::Context) {
    let due = app.distort.select_mask.as_ref().is_some_and(|w| w.preview.shown != w.preview.want && !w.pressed);
    if due {
        let _ = wait_preview(app);
    }
}

/// Pointer in document coordinates (from the view or the control channel).
pub fn pointer(app: &mut PhotocraftApp, ev: ToolEvent, mods: egui::Modifiers) {
    if drop_stale(app).is_err() {
        return;
    }
    let Some(w) = app.distort.select_mask.as_mut() else { return };
    match ev {
        ToolEvent::Down { x, y, .. } if w.tool == SmTool::Zoom => {
            let f = if mods.alt { 0.5 } else { 2.0 };
            w.zoom = (w.zoom * f).clamp(0.01, 32.0);
            w.center = [x, y];
        }
        ToolEvent::Down { x, y, .. } => w.begin([x, y], mods.alt),
        ToolEvent::Move { x, y, .. } => {
            w.hover = Some([x, y]);
            w.extend([x, y]);
        }
        ToolEvent::Up { x, y } => {
            w.extend([x, y]);
            w.end_gesture();
        }
    }
}

/// The workspace's keys (unless a text field has the keyboard): tools, brush size, views, undo.
pub fn keys(app: &mut PhotocraftApp, ctx: &egui::Context) {
    use egui::{Key, Modifiers};
    let Some(w) = app.distort.select_mask.as_mut() else { return };
    let cmd_shift = Modifiers::COMMAND | Modifiers::SHIFT;
    if ctx.input_mut(|i| i.consume_key(cmd_shift, Key::Z)) {
        redo(w);
        return;
    }
    if ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::Z)) {
        undo(w);
        return;
    }
    if egui::Popup::is_any_open(ctx) {
        w.hide_view = false;
        return;
    }
    let pressed = |k: Key, m: Modifiers| ctx.input_mut(|i| i.consume_key(m, k));
    if pressed(Key::W, Modifiers::SHIFT) {
        w.pick(if w.w_tool == SmTool::Quick { SmTool::Object } else { SmTool::Quick });
    }
    if pressed(Key::L, Modifiers::SHIFT) {
        w.pick(if w.l_tool == SmTool::Lasso { SmTool::PolygonalLasso } else { SmTool::Lasso });
    }
    for (k, t) in
        [(Key::W, w.w_tool), (Key::R, SmTool::RefineEdge), (Key::B, SmTool::Brush), (Key::L, w.l_tool), (Key::H, SmTool::Hand), (Key::Z, SmTool::Zoom)]
    {
        if pressed(k, Modifiers::NONE) {
            w.pick(t);
        }
    }
    if pressed(Key::OpenBracket, Modifiers::NONE) {
        let s = w.size();
        w.set_size((s * 0.9).min(s - 1.0));
    }
    if pressed(Key::CloseBracket, Modifiers::NONE) {
        let s = w.size();
        w.set_size((s * 1.1).max(s + 1.0));
    }
    if (pressed(Key::Backspace, Modifiers::NONE) || pressed(Key::Delete, Modifiers::NONE))
        && let Some((_, pts)) = w.poly.as_mut()
    {
        pts.pop();
        if pts.is_empty() {
            w.poly = None;
        }
    }
    // The views: their letters, F cycles, J Show Edge, P Show Original; holding X shows the image.
    let before = w.opts.clone();
    if pressed(Key::F, Modifiers::NONE) {
        w.opts.view = w.opts.view.next();
    }
    for m in Mode::ALL {
        if pressed(m.key(), Modifiers::NONE) {
            w.opts.view = m;
        }
    }
    if pressed(Key::J, Modifiers::NONE) {
        w.opts.show_edge = !w.opts.show_edge;
    }
    if pressed(Key::P, Modifiers::NONE) {
        w.opts.show_original = !w.opts.show_original;
    }
    if w.opts != before {
        w.stale();
    }
    w.hide_view = ctx.input(|i| i.key_down(Key::X) && !i.modifiers.command && !i.modifiers.alt && !i.modifiers.ctrl);
}

/// A view: screen px per document px and the document point at the centre of `area`.
#[derive(Clone, Copy)]
struct View {
    area: ERect,
    zoom: f32,
    center: [f64; 2],
}

impl View {
    fn to_screen(self, p: [f64; 2]) -> Pos2 {
        let c = self.area.center();
        pos2(c.x + ((p[0] - self.center[0]) as f32) * self.zoom, c.y + ((p[1] - self.center[1]) as f32) * self.zoom)
    }
    fn to_doc(self, s: Pos2) -> [f64; 2] {
        let c = self.area.center();
        [self.center[0] + f64::from((s.x - c.x) / self.zoom), self.center[1] + f64::from((s.y - c.y) / self.zoom)]
    }
    fn rect(self, r: Rect) -> ERect {
        ERect::from_min_max(self.to_screen([f64::from(r.x0), f64::from(r.y0)]), self.to_screen([f64::from(r.x1), f64::from(r.y1)]))
    }
}

/// Upload what changed: the document (for X and until the first preview) and a finished preview.
fn upload(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let need_image = app.distort.select_mask.as_ref().is_some_and(|w| w.image_tex.is_none());
    if need_image {
        let img = app.session.active().map(|st| {
            let (cw, ch) = (st.doc.size.width, st.doc.size.height);
            let f = crate::filter_preview_ui::texture_factor(cw, ch, PROXY_SIDE);
            let buf = photocraft_compose::render_reduced(&st.doc, cw.div_ceil(f.max(1)), ch.div_ceil(f.max(1)));
            let (display, _) = crate::canvas::canvas_display(app, &st.doc, None);
            crate::canvas::display_image(display.as_deref(), &buf)
        });
        if let (Some(w), Some(img)) = (app.distort.select_mask.as_mut(), img) {
            w.image_tex = Some(ctx.load_texture("select-mask-image", img, egui::TextureOptions::LINEAR));
        }
    }
    let Some(w) = app.distort.select_mask.as_mut() else { return };
    if let Some(s) = w.preview.pending.take() {
        match &mut w.preview.tex {
            Some(t) => t.set(s.image, egui::TextureOptions::LINEAR),
            None => w.preview.tex = Some(ctx.load_texture("select-mask-preview", s.image, egui::TextureOptions::LINEAR)),
        }
        w.preview.ants = s.ants;
        w.preview.covers = s.covers;
    }
}

/// A section of the Properties panel: a bold title that folds it, open by default.
fn section(ui: &mut egui::Ui, id: &str, title: &str, body: impl FnOnce(&mut egui::Ui)) {
    egui::CollapsingHeader::new(egui::RichText::new(title).strong()).id_salt(id).default_open(true).show(ui, body);
}

/// The options bar of the current tool.
fn options_bar(ui: &mut egui::Ui, w: &mut SelectMaskWorkspace) -> Option<Value> {
    let mut edit = None;
    crate::icons::button(ui, w.tool.icon(), 26.0, false, &w.tool.tip());
    ui.add_space(6.0);
    if w.tool.edits() {
        if crate::icons::button(ui, "plus", 26.0, w.add, tl!("Add")).clicked() {
            w.add = true;
        }
        if crate::icons::button(ui, "minus", 26.0, !w.add, tl!("Subtract")).clicked() {
            w.add = false;
        }
        ui.add_space(8.0);
    }
    if w.tool.brushes() {
        ui.label(tl!("Size:"));
        let mut size = w.size() as f32;
        if widgets::value_field(ui, &mut size, 1.0..=5000.0, " px", 64.0).changed() {
            w.set_size(f64::from(size));
        }
        ui.add_space(6.0);
    }
    if w.tool == SmTool::Brush {
        ui.label(tl!("Hardness:"));
        widgets::value_field(ui, &mut w.hardness, 0.0..=100.0, "%", 56.0);
        ui.add_space(6.0);
    }
    if matches!(w.tool, SmTool::Quick | SmTool::RefineEdge | SmTool::Object) && widgets::checkbox(ui, &mut w.sample_all, tl!("Sample All Layers")).changed() {
        w.stale();
    }
    if w.tool.edits() {
        ui.add_space(8.0);
        if widgets::secondary_button(ui, tl!("Select Subject"), 0.0).clicked() {
            edit = Some(json!({"tool": "subject", "sampleAllLayers": w.sample_all}));
        }
        // Refine Hair needs hair detection PhotoCraft doesn't have yet (#2671): greyed, saying so.
        ui.add_enabled_ui(false, |ui| widgets::secondary_button(ui, tl!("Refine Hair"), 0.0))
            .inner
            .on_disabled_hover_text(tl!("Refine Hair needs hair detection, which isn't available yet"));
    }
    edit
}

/// The Properties panel. Returns the tool edit its buttons made (Clear Selection, Invert).
fn properties(ui: &mut egui::Ui, w: &mut SelectMaskWorkspace, t: &Tokens) -> Option<Value> {
    let mut edit = None;
    ui.spacing_mut().item_spacing.y = 6.0;
    widgets::checkbox(ui, &mut w.opts.remember, tl!("Remember Settings"));
    widgets::hairline(ui);
    section(ui, "sm-view", tl!("View Mode"), |ui| {
        ui.horizontal_top(|ui| {
            ui.vertical(|ui| {
                ui.label(egui::RichText::new(tl!("View")).color(t.text_dim));
                let labels: Vec<String> = Mode::ALL.iter().map(|m| format!("{} ({:?})", m.label(), m.key())).collect();
                let options: Vec<(Mode, &str)> = Mode::ALL.iter().zip(&labels).map(|(m, l)| (*m, l.as_str())).collect();
                // The labels carry the view's letter, already translated.
                widgets::dropdown_literal(ui, "sm-view-mode", tl!("View"), &mut w.opts.view, &options, 112.0);
            });
            // Beside the view list, as Photoshop's; long labels are cut rather than widening the
            // panel past its edge (every row below lines up on that edge).
            let room = ui.available_width();
            ui.vertical(|ui| {
                ui.set_max_width(room);
                ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
                widgets::checkbox(ui, &mut w.opts.show_edge, &format!("{} (J)", tl!("Show Edge")));
                widgets::checkbox(ui, &mut w.opts.show_original, &format!("{} (P)", tl!("Show Original")));
                widgets::checkbox(ui, &mut w.opts.real_time, tl!("Real-Time Refinement"));
                widgets::checkbox(ui, &mut w.opts.high_quality, tl!("High Quality Preview"));
            });
        });
        let i = mode_index(w.opts.view);
        if w.opts.view.default_opacity().is_some() {
            let label = if w.opts.view == Mode::OnionSkin { tl!("Transparency") } else { tl!("Opacity") };
            if let Some(o) = w.opts.opacity.get_mut(i) {
                widgets::slider_row(ui, label, o, 0.0..=100.0, "%", None);
            }
        }
        if w.opts.view == Mode::Overlay {
            ui.horizontal(|ui| {
                ui.label(tl!("Color"));
                widgets::color_edit_button_srgb(ui, &mut w.opts.overlay_color);
                ui.add_space(8.0);
                ui.label(tl!("Indicates"));
                widgets::dropdown(
                    ui,
                    "sm-indicates",
                    tl!("Indicates"),
                    &mut w.opts.indicates_selected,
                    &[(false, tl!("Masked Areas")), (true, tl!("Selected Areas"))],
                    120.0,
                );
            });
        }
    });
    section(ui, "sm-refine-mode", tl!("Refine Mode"), |ui| {
        ui.horizontal(|ui| {
            let _ = ui.selectable_label(true, tl!("Color Aware"));
            // Object Aware needs a learned segmentation model (#2788).
            ui.add_enabled_ui(false, |ui| ui.selectable_label(false, tl!("Object Aware")))
                .inner
                .on_disabled_hover_text(tl!("Object Aware needs a segmentation model, which isn't available yet"));
        });
    });
    section(ui, "sm-edge", tl!("Edge Detection"), |ui| {
        widgets::slider_row(ui, tl!("Radius"), &mut w.opts.radius, 0.0..=250.0, " px", None);
        widgets::checkbox(ui, &mut w.opts.smart_radius, tl!("Smart Radius"));
    });
    section(ui, "sm-global", tl!("Global Refinements"), |ui| {
        widgets::slider_row(ui, tl!("Smooth"), &mut w.opts.smooth, 0.0..=100.0, "", None);
        widgets::slider_row(ui, tl!("Feather"), &mut w.opts.feather, 0.0..=1000.0, " px", None);
        widgets::slider_row(ui, tl!("Contrast"), &mut w.opts.contrast, 0.0..=100.0, "%", None);
        widgets::slider_row(ui, tl!("Shift Edge"), &mut w.opts.shift_edge, -100.0..=100.0, "%", None);
        ui.horizontal(|ui| {
            if widgets::secondary_button(ui, tl!("Clear Selection"), 0.0).clicked() {
                edit = Some(json!({"tool": "clear"}));
            }
            if widgets::secondary_button(ui, tl!("Invert"), 0.0).clicked() {
                edit = Some(json!({"tool": "invert"}));
            }
        });
    });
    section(ui, "sm-output", tl!("Output Settings"), |ui| {
        widgets::checkbox(ui, &mut w.opts.decontaminate, tl!("Decontaminate Colors"));
        ui.add_enabled_ui(w.opts.decontaminate, |ui| widgets::slider_row(ui, tl!("Amount"), &mut w.opts.amount, 0.0..=100.0, "%", None));
        w.opts.follow_decontaminate();
        ui.horizontal(|ui| {
            ui.label(tl!("Output To"));
            let labels =
                [tl!("Selection"), tl!("Layer Mask"), tl!("New Layer"), tl!("New Layer With Mask"), tl!("New Document"), tl!("New Document With Mask")];
            // Decontaminating makes new pixels: only the new layer and document targets apply.
            let options: Vec<(String, &str)> = OUTPUTS
                .iter()
                .zip(labels)
                .filter(|(o, _)| !w.opts.decontaminate || !matches!(**o, "selection" | "layerMask"))
                .map(|(o, l)| (o.to_string(), l))
                .collect();
            widgets::dropdown(ui, "sm-output-to", tl!("Output To"), &mut w.opts.output, &options, 170.0);
        });
    });
    if let Some(e) = w.error.as_ref().or(w.preview.error.as_ref()) {
        ui.label(egui::RichText::new(e.as_str()).color(t.danger).size(11.0));
    }
    edit
}

/// The document changed under the workspace: new textures, a new preview, and nothing of the old
/// one shown or reused.
fn follow_revision(app: &mut PhotocraftApp) {
    let rev = app.session.active().map(|s| s.revision);
    if let Some(w) = app.distort.select_mask.as_mut()
        && Some(w.revision) != rev
    {
        w.revision = rev.unwrap_or(w.revision);
        w.image_tex = None;
        w.preview.tex = None;
        w.preview.pending = None;
        w.preview.cache = None;
        w.stale();
    }
}

/// Draws the document, the refined edge, the stroke being drawn and the polygon in progress.
fn paint_canvas(ui: &mut egui::Ui, ctx: &egui::Context, w: &mut SelectMaskWorkspace, t: &Tokens, painter: &egui::Painter, resp: &egui::Response, area: ERect) {
    let v = View { area, zoom: w.zoom, center: w.center };
    let clip = painter.with_clip_rect(area);
    let img = v.rect(w.canvas);
    let uv = ERect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
    widgets::checker(&clip, img.intersect(area), 8.0);
    let preview = w.preview.tex.as_ref().filter(|_| !w.hide_view);
    match (preview, &w.image_tex) {
        (Some(tex), _) => {
            // The reduced copy's last row and column may reach past the canvas.
            let covers = ERect::from_min_size(img.min, vec2(w.preview.covers[0] * v.zoom, w.preview.covers[1] * v.zoom));
            clip.with_clip_rect(img.intersect(area)).image(tex.id(), covers, uv, Color32::WHITE);
        }
        (None, Some(tex)) => {
            clip.image(tex.id(), img, uv, Color32::WHITE);
        }
        (None, None) => {}
    }
    // Marching Ants: the refined selection's edge, black dashes on white.
    if !w.hide_view
        && w.opts.view == Mode::MarchingAnts
        && let Some(edges) = &w.preview.ants
    {
        let phase = (ui.input(|i| i.time) * 8.0) as usize;
        for e in edges {
            let (a, b) = (v.to_screen([f64::from(e[0][0]), f64::from(e[0][1])]), v.to_screen([f64::from(e[1][0]), f64::from(e[1][1])]));
            let dark = ((e[0][0] + e[0][1]) as usize / 4 + phase).is_multiple_of(2);
            clip.line_segment([a, b], Stroke::new(1.0, if dark { Color32::BLACK } else { Color32::WHITE }));
        }
        ctx.request_repaint_after(std::time::Duration::from_millis(120));
    }
    if w.preview.shown != w.preview.want && !w.pressed {
        let sp = ERect::from_min_size(area.right_bottom() - vec2(26.0, 26.0), vec2(18.0, 18.0));
        ui.put(sp, egui::Spinner::new().size(16.0));
    }
    // The stroke being drawn: the part the preview doesn't show yet (all of it without
    // Real-Time Refinement), joined to the part it does, so the stroke follows the pointer.
    let ink = Color32::from_rgba_unmultiplied(t.accent.r(), t.accent.g(), t.accent.b(), 110);
    if let Some(g) = w.gesture.as_ref().or(w.trail.as_ref()) {
        let line: Vec<Pos2> = g.points.iter().map(|p| v.to_screen(*p)).collect();
        match g.tool {
            SmTool::Quick | SmTool::Brush | SmTool::RefineEdge if w.preview.trail != usize::MAX => {
                let tail = line.get(w.preview.trail.saturating_sub(1)..).unwrap_or_default();
                let r = w.size() as f32 * 0.5 * v.zoom;
                for p in tail {
                    clip.circle_filled(*p, r, ink);
                }
                if tail.len() > 1 {
                    clip.add(egui::Shape::line(tail.to_vec(), Stroke::new(2.0 * r, ink)));
                }
            }
            SmTool::Lasso => {
                clip.add(egui::Shape::line(line, Stroke::new(1.0, t.accent)));
            }
            SmTool::Object => {
                if let (Some(a), Some(b)) = (line.first(), line.last()) {
                    clip.rect_stroke(ERect::from_two_pos(*a, *b), 0.0, Stroke::new(1.0, t.accent), egui::StrokeKind::Middle);
                }
            }
            _ => {}
        }
    }
    if let Some((_, pts)) = &w.poly {
        let mut line: Vec<Pos2> = pts.iter().map(|p| v.to_screen(*p)).collect();
        if let Some(hp) = resp.hover_pos() {
            line.push(hp);
        }
        clip.add(egui::Shape::line(line, Stroke::new(1.0, t.accent)));
    }
}

/// The cursor over the document, and the image's outline.
fn paint_cursor(ctx: &egui::Context, w: &mut SelectMaskWorkspace, clip: &egui::Painter, resp: &egui::Response, v: View, pan: bool, alt: bool) {
    let (img, t) = (v.rect(w.canvas), Tokens::get(ctx));
    // The cursor: Photoshop's hand while panning (closed while dragging), the brush's circle
    // (with + or −), the zoom glass, else a crosshair.
    if let Some(hp) = resp.hover_pos() {
        w.hover = Some(v.to_doc(hp));
    }
    if pan && (resp.dragged() || resp.is_pointer_button_down_on()) {
        ctx.set_cursor_icon(egui::CursorIcon::Grabbing);
    } else if let Some(hp) = resp.hover_pos() {
        if pan {
            ctx.set_cursor_icon(egui::CursorIcon::Grab);
        } else if w.tool.brushes() {
            let r = (w.size() as f32 * 0.5 * v.zoom).max(2.0);
            clip.circle_stroke(hp, r, Stroke::new(1.5, Color32::BLACK));
            clip.circle_stroke(hp, r, Stroke::new(0.75, Color32::WHITE));
            let add = w.add != alt;
            for s in [Stroke::new(2.5, Color32::from_black_alpha(140)), Stroke::new(1.0, Color32::from_white_alpha(220))] {
                clip.line_segment([hp - vec2(3.0, 0.0), hp + vec2(3.0, 0.0)], s);
                if add {
                    clip.line_segment([hp - vec2(0.0, 3.0), hp + vec2(0.0, 3.0)], s);
                }
            }
            ctx.set_cursor_icon(egui::CursorIcon::None);
        } else {
            ctx.set_cursor_icon(match w.tool {
                SmTool::Zoom if alt => egui::CursorIcon::ZoomOut,
                SmTool::Zoom => egui::CursorIcon::ZoomIn,
                _ => egui::CursorIcon::Crosshair,
            });
        }
    }
    clip.rect_stroke(img, 0.0, Stroke::new(1.0, t.separator), egui::StrokeKind::Outside);
}

/// Pan, zoom-click and tool gestures on the document, as events for `pointer`.
#[allow(clippy::too_many_arguments)]
fn canvas_input(
    ui: &egui::Ui,
    ctx: &egui::Context,
    w: &mut SelectMaskWorkspace,
    resp: &egui::Response,
    v: View,
    pan: bool,
    events: &mut Vec<ToolEvent>,
    mods: &mut egui::Modifiers,
) {
    if pan && resp.dragged() {
        let dl = resp.drag_delta();
        w.center = [w.center[0] - f64::from(dl.x / w.zoom), w.center[1] - f64::from(dl.y / w.zoom)];
    } else if w.tool == SmTool::PolygonalLasso && !pan {
        *mods = ui.input(|i| i.modifiers);
        if resp.double_clicked() {
            w.close_polygon();
        } else if resp.clicked()
            && let Some(pp) = resp.interact_pointer_pos()
        {
            let q = v.to_doc(pp);
            events.push(ToolEvent::Down { x: q[0], y: q[1], pressure: 1.0 });
        }
    } else if !pan
        && resp.is_pointer_button_down_on()
        && let Some(pp) = resp.interact_pointer_pos()
    {
        let q = v.to_doc(pp);
        *mods = ui.input(|i| i.modifiers);
        if w.pressed {
            events.push(ToolEvent::Move { x: q[0], y: q[1], pressure: 1.0 });
            ctx.request_repaint();
        } else {
            w.pressed = true;
            events.push(ToolEvent::Down { x: q[0], y: q[1], pressure: 1.0 });
        }
    }
    if w.pressed && !resp.is_pointer_button_down_on() {
        w.pressed = false;
        let q = w.hover.unwrap_or(w.center);
        events.push(ToolEvent::Up { x: q[0], y: q[1] });
    }
}

/// Draws the workspace (a full-window layer over the app).
pub fn show(app: &mut PhotocraftApp, ctx: &egui::Context) {
    follow_revision(app);
    upload(app, ctx);
    let title = app.distort.select_mask.as_ref().map(|w| {
        crate::canvas::tab_title_at(
            app,
            app.session.documents().iter().position(|d| d.doc.id == w.document).unwrap_or(0),
            if w.zoom > 0.0 { w.zoom * 100.0 } else { 100.0 },
        )
    });
    let t = Tokens::get(ctx);
    let screen = ctx.content_rect();
    let mut action: Option<&str> = None;
    let mut events: Vec<ToolEvent> = Vec::new();
    let mut mods = egui::Modifiers::NONE;
    let mut edit: Option<Value> = None;
    let mut do_reset = false;
    egui::Area::new(egui::Id::unique("select-mask-workspace")).order(egui::Order::Foreground).fixed_pos(screen.min).show(ctx, |ui| {
        let Some(w) = app.distort.select_mask.as_mut() else { return };
        let before = (w.opts.clone(), w.sample_all);
        w.texture_limit = ctx.input(|i| i.max_texture_side);
        let (full, _) = ui.allocate_exact_size(screen.size(), Sense::hover());
        let painter = ui.painter().clone();
        painter.rect_filled(full, 0.0, t.chrome);
        // The options bar across the top, as Photoshop's.
        let bar = ERect::from_min_size(full.min, vec2(full.width(), BAR_H));
        painter.rect_filled(bar, 0.0, t.dock);
        painter.line_segment([bar.left_bottom(), bar.right_bottom()], Stroke::new(1.0, t.separator));
        let mut bui = ui.new_child(egui::UiBuilder::new().max_rect(bar.shrink2(vec2(8.0, 4.0))).layout(egui::Layout::left_to_right(egui::Align::Center)));
        if let Some(e) = options_bar(&mut bui, w) {
            edit = Some(e);
        }
        let body = ERect::from_min_max(pos2(full.left(), bar.bottom()), full.max);
        // The tools on the left: Lasso and Polygonal Lasso share a slot (right-click to pick).
        let left = ERect::from_min_size(body.min, vec2(LEFT_W, body.height()));
        painter.rect_filled(left, 0.0, t.dock);
        painter.line_segment([left.right_top(), left.right_bottom()], Stroke::new(1.0, t.separator));
        let mut strip = ui.new_child(egui::UiBuilder::new().max_rect(left.shrink2(vec2(6.0, 8.0))));
        strip.spacing_mut().item_spacing.y = 4.0;
        for slot in [SmTool::Quick, SmTool::RefineEdge, SmTool::Brush, SmTool::Object, w.l_tool, SmTool::Hand, SmTool::Zoom] {
            let r = crate::icons::button(&mut strip, slot.icon(), 34.0, w.tool == slot, &slot.tip());
            if r.clicked() {
                w.pick(slot);
            }
            if matches!(slot, SmTool::Lasso | SmTool::PolygonalLasso) {
                // The slot's flyout triangle, as the main toolbar's.
                let c = r.rect.right_bottom() - vec2(3.0, 3.0);
                painter.add(egui::Shape::convex_polygon(vec![c, c - vec2(5.0, 0.0), c - vec2(0.0, 5.0)], t.icon, Stroke::NONE));
                r.context_menu(|ui| {
                    for l in [SmTool::Lasso, SmTool::PolygonalLasso] {
                        if ui.selectable_label(w.tool == l, l.tip()).clicked() {
                            w.pick(l);
                            ui.close();
                        }
                    }
                });
            }
        }
        // The Properties panel on the right.
        let right = ERect::from_min_size(pos2(body.right() - PANEL_W, body.top()), vec2(PANEL_W, body.height()));
        painter.rect_filled(right, 0.0, t.dock);
        painter.line_segment([right.left_top(), right.left_bottom()], Stroke::new(1.0, t.separator));
        let head = ERect::from_min_size(right.min, vec2(PANEL_W, TAB_H));
        painter.line_segment([head.left_bottom(), head.right_bottom()], Stroke::new(1.0, t.separator));
        painter.text(head.left_center() + vec2(12.0, 0.0), Align2::LEFT_CENTER, tl!("Properties"), FontId::proportional(12.0), t.text);
        let mut props = ui.new_child(
            egui::UiBuilder::new().max_rect(ERect::from_min_max(head.left_bottom() + vec2(14.0, 10.0), pos2(right.right() - 14.0, right.bottom() - 56.0))),
        );
        egui::ScrollArea::vertical().id_salt("sm-props").show(&mut props, |ui| {
            if let Some(e) = properties(ui, w, &t) {
                edit = Some(e);
            }
        });
        // Reset at the left, OK and Cancel against the right edge.
        let foot = ERect::from_min_size(pos2(right.left() + 10.0, right.bottom() - 46.0), vec2(PANEL_W - 20.0, 32.0));
        painter.line_segment([pos2(right.left(), foot.top() - 6.0), pos2(right.right(), foot.top() - 6.0)], Stroke::new(1.0, t.separator));
        let mut fb = ui.new_child(egui::UiBuilder::new().max_rect(foot).layout(egui::Layout::right_to_left(egui::Align::Center)));
        if widgets::secondary_button(&mut fb, tl!("Cancel"), 70.0).clicked() {
            action = Some("cancel");
        }
        if widgets::primary_button(&mut fb, tl!("OK"), 64.0).clicked() {
            action = Some("ok");
        }
        fb.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
            if crate::icons::button(ui, "undo-2", 24.0, false, tl!("Reset")).clicked() {
                do_reset = true;
            }
        });
        // The document under its tab.
        let doc_area = ERect::from_min_max(pos2(left.right(), body.top()), pos2(right.left(), body.bottom()));
        let tab = ERect::from_min_size(doc_area.min, vec2(doc_area.width(), TAB_H));
        painter.rect_filled(tab, 0.0, t.dock);
        painter.line_segment([tab.left_bottom(), tab.right_bottom()], Stroke::new(1.0, t.separator));
        let title = title.clone().unwrap_or_default();
        let galley = painter.layout_no_wrap(title, FontId::proportional(12.0), t.text);
        let tab_w = (galley.size().x + 28.0).min(tab.width());
        let active_tab = ERect::from_min_size(tab.min, vec2(tab_w, TAB_H));
        painter.rect_filled(active_tab, 0.0, t.canvas);
        painter.galley(pos2(active_tab.left() + 14.0, active_tab.center().y - galley.size().y / 2.0), galley, t.text);
        let area = ERect::from_min_max(pos2(doc_area.left(), tab.bottom()), doc_area.max);
        painter.rect_filled(area, 0.0, t.canvas);
        let (cw, chh) = (w.canvas.width() as f32, w.canvas.height() as f32);
        if w.zoom <= 0.0 {
            w.zoom = ((area.width() - 40.0) / cw.max(1.0)).min((area.height() - 40.0) / chh.max(1.0)).clamp(0.01, 4.0);
        }
        let resp = ui.interact(area, egui::Id::unique("select-mask-view"), Sense::click_and_drag());
        // Over the image the keys are the workspace's: a panel button clicked last keeps the focus
        // otherwise, and Space (held to pan) would press it again. A number being typed keeps it.
        if resp.hovered()
            && !ctx.text_edit_focused()
            && let Some(focus) = ctx.memory(|m| m.focused())
        {
            ctx.memory_mut(|m| m.surrender_focus(focus));
        }
        if resp.hovered() {
            let scroll = ui.input(|i| i.smooth_scroll_delta.y);
            if scroll != 0.0
                && let Some(hp) = resp.hover_pos()
            {
                let v = View { area, zoom: w.zoom, center: w.center };
                let at = v.to_doc(hp);
                w.zoom = (w.zoom * (scroll / 200.0).exp()).clamp(0.01, 32.0);
                let after = View { area, zoom: w.zoom, center: w.center }.to_doc(hp);
                w.center = [w.center[0] + at[0] - after[0], w.center[1] + at[1] - after[1]];
            }
        }
        let v = View { area, zoom: w.zoom, center: w.center };
        paint_canvas(ui, ctx, w, &t, &painter, &resp, area);
        // Space, the middle button or the Hand pan. The Polygonal Lasso takes clicks: each adds a
        // vertex, a double-click closes.
        let pan = ui.input(|i| i.key_down(egui::Key::Space) || i.pointer.middle_down()) || w.tool == SmTool::Hand;
        paint_cursor(ctx, w, &painter.with_clip_rect(area), &resp, v, pan, ui.input(|i| i.modifiers.alt));
        canvas_input(ui, ctx, w, &resp, v, pan, &mut events, &mut mods);
        if (w.opts.clone(), w.sample_all) != before {
            w.stale();
        }
    });
    for ev in events {
        pointer(app, ev, mods);
    }
    if let Some(w) = app.distort.select_mask.as_mut() {
        if let Some(e) = edit {
            w.add_edit(e);
        }
        if do_reset {
            reset(w);
        }
    }
    drive_preview(app, ctx);
    match action {
        Some("ok") => {
            let _ = commit(app);
        }
        Some("cancel") => cancel(app),
        _ => {}
    }
}

#[cfg(test)]
mod tests;
