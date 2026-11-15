//! Free Transform (⌘T): bounding box with handles over the canvas, live preview, commit via the
//! engine's `edit.transform` (one quad for scale/rotate/skew/distort/perspective).
//!
//! Gestures follow Photoshop CC: corner and edge drags scale proportionally (⇧ for free: an edge
//! then stretches its one axis), ⌥ scales about the reference point, ⌘-drag a corner distorts
//! (⌘⌥⇧: perspective), ⌘-drag an edge skews (⇧ along the edge), drag outside rotates (⇧ snaps to
//! 15°), drag inside moves (⇧ locks to 8 directions), the reference point can be dragged and
//! ⌥-click puts it under the pointer. Arrow keys nudge the box (move_mods.rs). ↩ commits, Esc
//! cancels. Undo and Redo step through the session's own changes (`Steps`), not the document's
//! history.

use std::sync::Arc;

use egui::{Color32, CursorIcon, Pos2, Stroke, pos2, vec2};
use photocraft_algo::transform::Homography;
use photocraft_doc::{DocId, Document, LayerContent, LayerId};
use photocraft_engine::jobs::{JobEvent, JobId, JobOutcome};
use photocraft_geom::warp::{BezierMesh, Warp, WarpStyle};
use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::{ToolEvent, ViewXform};
use crate::state::{MadeLayer, Tool, TransformMode, TransformSession};

mod begin;
mod commit;
mod cylinder;
mod drag_modes;
mod draw;
mod options_bar;
mod pointer;
mod preview;
mod warp;

pub(crate) use begin::begin_content_aware;
pub(crate) use begin::moving_layers;
#[cfg(test)]
pub(crate) use begin::moving_texel;
pub use begin::{WARP_ONE_LAYER, begin, begin_copy, begin_placed, begin_selection, begin_warp, enter_warp, leave_warp, warps_active_layer, warps_type};
pub(crate) use commit::{SAVE_FOR_WEB, committing, nudge};
pub use commit::{cancel, commit, end_if_left, follow_edits, intercept, is_enabled, on_job_event, track_steps};
use cylinder::CylinderHandle;
#[cfg(test)]
use cylinder::{cylinder_cursor, cylinder_default, cylinder_dragged, cylinder_handles, cylinder_hit, cylinder_values};
#[cfg(test)]
use drag_modes::{HandleIndicator, handle_indicator, keep_convex, turns};
pub(crate) use drag_modes::{apply_drag, corner_distorts, legacy_mods};
#[cfg(test)]
pub(crate) use drag_modes::{apply_mode_drag, unlinked_mods};
pub(crate) use draw::draw_handle_cursor;
#[cfg(test)]
use draw::scale_values;
#[cfg(test)]
pub(crate) use draw::{Readout, live_readout, styled_frame};
pub(crate) use draw::{carries_selection_of, moves_selection, selection_ants};
pub use draw::{draw_content, draw_overlay};
#[cfg(test)]
use draw::{front_vertices, handle_anchor};
pub(crate) use options_bar::position_size_fields;
#[cfg(test)]
use options_bar::{
    POSITION_LIMIT_PX, Parts, SCALE_LIMIT_PCT, SIZE_UNIT_KEY, SKEW_LIMIT_DEG, matching_anchor, readout, rotate_about_pivot, scale_about_pivot, scale_limit_pct,
    set_size_unit, size_pct, size_shown, size_unit, warp_grid_id, warp_style_options,
};
pub use options_bar::{options_bar, reset_warp};
pub(crate) use pointer::{Gesture, Hit, box_hit, hit, rotate_cursor_dir};
#[cfg(test)]
use pointer::{accepts_outside_click, carry_pivot, near_rotate_corner, point_on_quad, rotation_degrees};
pub use pointer::{cursor, pointer};
pub(crate) use preview::solo_layer;
use preview::{Above, composite_image, show_only};
#[cfg(test)]
use preview::{covering, preview_image};
pub use warp::{edit_session_warp, split};
#[cfg(test)]
use warp::{on_section_edge, single_patch, split_cursor, surface_at, warp_hit, warp_pointer};

/// Which Split button is armed. The guide follows the pointer and the split is added on release.
#[derive(Clone, Copy, PartialEq, Eq)]
enum SplitTool {
    Cross,
    Vertical,
    Horizontal,
}

impl SplitTool {
    fn command(self) -> &'static str {
        match self {
            SplitTool::Cross => "edit.transform.splitWarpCrosswise",
            SplitTool::Vertical => "edit.transform.splitWarpVertically",
            SplitTool::Horizontal => "edit.transform.splitWarpHorizontally",
        }
    }

    fn icon(self) -> &'static str {
        match self {
            SplitTool::Cross => "cross",
            SplitTool::Vertical => "vertical",
            SplitTool::Horizontal => "horizontal",
        }
    }
}

/// What a Warp-mode press grabbed. `pts0` is the mesh's control grid at the press.
enum WarpDrag {
    /// An anchor or handle: it follows the pointer (an anchor takes its handles along).
    Point { index: usize, start: [f64; 2], pts0: Vec<[f64; 2]> },
    /// Anywhere else on the grid: the surface under the pointer follows it.
    Surface { s: f64, t: f64, start: [f64; 2], pts0: Vec<[f64; 2]> },
    /// A cylinder's handle, or its surface (which turns the wrap); `values0` are its seven values
    /// at the press (cylinder.rs).
    Cylinder { handle: CylinderHandle, start: [f64; 2], values0: [f64; 7] },
}

/// Opens a prepared box once its job has landed (see [`Preparing`]).
type FinishPrepared = Box<dyn FnOnce(&mut PhotocraftApp, begin::Prepared) -> Result<(), String>>;

/// A large layer's box being prepared as a job (`begin::on_prepared` opens it when it lands).
pub(crate) struct Preparing {
    job: JobId,
    out: Arc<std::sync::Mutex<Option<begin::Prepared>>>,
    finish: FinishPrepared,
    /// The document and layer the box is for.
    layer: (DocId, LayerId),
    /// The layer was made for this box (⌥⌘T's copy, a placed file): the opened session gets the
    /// marker, and a box that never opens takes the layer back, as Esc would.
    made: Option<MadeLayer>,
}

/// The canvas key of an open box's preview document (`display_doc`).
pub(crate) fn preview_key(session: u64) -> u64 {
    (1 << 40) + session
}

/// Where the document under an open box differs from the document itself: the moving layer
/// (hidden, or its selected pixels lifted out) and the layers drawn over the box instead
/// ([`preview::Above`]). Kept from when the box opens until the next one, so opening, canceling
/// and committing redraw that area instead of the whole canvas.
#[derive(Clone, Copy, Debug)]
pub(crate) struct OpenArea {
    pub doc: DocId,
    pub key: u64,
    /// The document revision the preview was built from.
    pub revision: u64,
    pub area: photocraft_geom::Rect,
}

/// The canvas damage between the document and an open box's preview of it, either way, at the
/// revision the box opened on; on a commit (one revision later) also what the commit changed.
/// `None` (redraw everything) for anything else, e.g. a box that followed an edit while open.
pub(crate) fn damage(
    app: &PhotocraftApp,
    doc: DocId,
    seen: (u64, u64),
    now: (u64, u64),
    last_damage: Option<photocraft_geom::Rect>,
) -> Option<photocraft_geom::Rect> {
    let o = app.transform_area.filter(|o| o.doc == doc)?;
    let opening = seen.1 == 0 && now.1 == o.key && seen.0 == o.revision && now.0 == o.revision;
    let canceled = seen.1 == o.key && now.1 == 0 && seen.0 == o.revision && now.0 == o.revision;
    if opening || canceled {
        return Some(o.area);
    }
    // Committed: the box's preview, replaced by the document with the transform applied.
    if seen.1 == o.key && now.1 == 0 && seen.0 == o.revision && now.0 == o.revision + 1 {
        let done = last_damage?;
        return Some(if done.is_empty() { o.area } else { o.area.union(&done) });
    }
    None
}

/// Preview state that isn't serialisable: the document without the transformed pixels, and
/// full-resolution textures of those pixels (transform_tex.rs).
pub struct TransformPreview {
    pub session: u64,
    pub doc: Arc<Document>,
    pub texture: crate::transform_tex::PreviewTextures,
    pub opacity: f32,
    /// The layers stacked over the moving one, drawn over its pixels ([`Above`]).
    above: Option<Above>,
    /// A layer with a style: its pixels composited with the style as the box has them now,
    /// drawn instead of the bare pixels (transform_fx.rs, #3653).
    styled: Option<crate::transform_fx::StyledPreview>,
    gesture: Option<Gesture>,
    /// Warp-mode drag in progress.
    warp_drag: Option<WarpDrag>,
    /// Armed split tool. None while the buttons are off and control points drag as usual.
    split_tool: Option<SplitTool>,
    /// Document point the split guide is following (a drag, or the hover while placing).
    split_pointer: Option<[f64; 2]>,
    split_placing: bool,
    /// Option-click places a split without a button armed.
    split_quick: bool,
    steps: Steps,
    /// The tool the session began with: picking another one applies the transform.
    tool: Tool,
    /// Free Transform Path: the path as it was, drawn through the box instead of pixels.
    path: Option<photocraft_doc::vector::Path>,
    /// A layer's session keeps `doc` and `opacity` up to date with edits made while the box is
    /// open ([`follow_edits`]); `None` for a lone mask or channel and Transform Selection.
    follow: Option<Follow>,
    /// The commit running as a background job (#1015): the moving pixels stay on screen, without
    /// the box, until it lands; Esc cancels it. The session ends with the job.
    committing: Option<JobId>,
    /// Content-Aware Scale's live preview (cas_transform.rs).
    pub(crate) content_aware: Option<crate::cas_transform::Live>,
    /// While a smart object's stored warp is edited (begin.rs `preview_without_smart_warp`):
    /// the texels and box of the mode the session is not in. Warp mode shows the source without
    /// the warp over the warp box, Free Transform the layer's render with it over its own box;
    /// leaving or re-entering Warp mode, and Undo across it, swap them.
    other_texels: Option<OtherTexels>,
    /// A box over several layers (#6501): their composite, which `texture` holds.
    several: Option<Several>,
}

/// The moving pixels of a box over several layers ([`begin_several`]): the layers composited on
/// their own, with their styles, opacity and masks. Composited again when an edit made while the
/// box is open changes how they look ([`follow_edits`]), since nothing is tinted at draw time.
struct Several {
    ctx: egui::Context,
    name: String,
    max_side: usize,
    ids: Vec<LayerId>,
    /// The pixels a selection lifted out of moving pixel layers, as the box opened. Every other
    /// moving layer (groups, type, shapes, smart objects, pixel layers without a selection) moves
    /// whole, so the live document holds what it moves, children and their looks included.
    lifted: Vec<(LayerId, photocraft_raster::Surface)>,
    /// The document showing only the moving layers, as last composited.
    moving: Document,
    /// The box's frame.
    frame: photocraft_geom::Rect,
    /// The document area the texture covers: the frame grown by the reach of the moving layers'
    /// styles, so a shadow, glow or outside stroke past their content moves with the box.
    rect: photocraft_geom::Rect,
}

impl Several {
    /// `live` showing only the layers `ids`, with the `lifted` pixels in place of their layers'.
    /// Unclipped: a clipping base that isn't moving would clip them away (#5239).
    fn shown(live: &Document, ids: &[LayerId], lifted: &[(LayerId, photocraft_raster::Surface)]) -> Document {
        let mut d = live.clone();
        show_only(&mut d.layers, ids);
        for id in ids {
            if let Some(path) = d.path_of(*id) {
                for n in 1..path.len() {
                    if let Some(ancestor) = path.get(..n).and_then(|p| d.layer_at_mut(p)) {
                        ancestor.clipped = false;
                    }
                }
            }
            if let Some(l) = d.layer_mut(*id) {
                l.clipped = false;
            }
        }
        for (id, surf) in lifted {
            if let Some(s) = d.layer_mut(*id).and_then(|l| l.surface_mut()) {
                *s = surf.clone();
            }
        }
        d
    }

    /// Composite the moving layers over their frame and their styles' reach.
    fn render(&mut self) -> crate::transform_tex::PreviewTextures {
        self.rect = self.frame.inflate(style_reach(&self.moving, &self.ids));
        let (image, uv) = composite_image(&self.moving, self.rect, self.max_side);
        crate::transform_tex::PreviewTextures::new(&self.ctx, self.name.clone(), image, uv)
    }
}

/// How far past their content the styles of the layers `ids` can draw, in document pixels: their
/// own, the layers' inside them and the groups' around them, since a group's style draws around
/// its children's composite and so adds to theirs.
fn style_reach(doc: &Document, ids: &[LayerId]) -> i32 {
    let reach_of =
        |l: &photocraft_doc::Layer| if l.visible && photocraft_compose::effects::has_effects(l) { photocraft_compose::effects::margin(l).max(0) } else { 0 };
    // The groups around each layer: everything on its path but itself.
    let around =
        |id: LayerId| doc.path_of(id).map_or(0, |p| (1..p.len()).filter_map(|n| doc.layer_at(p.get(..n)?)).fold(0i32, |r, g| r.saturating_add(reach_of(g))));
    // An explicit stack rather than recursion: documents can nest deeply.
    let mut stack: Vec<(&photocraft_doc::Layer, i32)> = ids.iter().filter_map(|id| Some((doc.layer(*id)?, around(*id)))).collect();
    let mut reach = 0;
    while let Some((l, outer)) = stack.pop() {
        let here = outer.saturating_add(reach_of(l));
        reach = reach.max(here);
        if let LayerContent::Group(g) = &l.content {
            stack.extend(g.children.iter().map(|c| (c, here)));
        }
    }
    reach
}

/// The preview texels of the transform mode the session is not in, and the box they cover
/// ([`TransformPreview::other_texels`]).
struct OtherTexels {
    texture: crate::transform_tex::PreviewTextures,
    rect: [f64; 4],
    quad: [[f64; 2]; 4],
}

/// Swaps the session's texels and box with the other mode's (`t` holds the box the texels
/// cover), when there are any: the stored warp's edit shows the source without the warp, the
/// box the render with it.
fn swap_mode_texels(t: &mut TransformSession, pv: &mut TransformPreview) {
    if let Some(o) = pv.other_texels.as_mut() {
        std::mem::swap(&mut pv.texture, &mut o.texture);
        std::mem::swap(&mut t.rect, &mut o.rect);
        std::mem::swap(&mut t.quad, &mut o.quad);
    }
}

/// What [`follow_edits`] needs to rebuild the preview document.
#[derive(Clone, Debug)]
struct Follow {
    /// The document revision `doc` was built from.
    revision: u64,
    /// The moving layers hidden in `doc` (nothing was lifted out of them).
    hidden: Vec<LayerId>,
    /// The Layers panel's selection when the box opened, sorted; `None` when the box doesn't
    /// follow it. Any change to it applies the box (`end_if_left`, #11101).
    selected: Option<Vec<LayerId>>,
}

/// Photoshop's alert when Free Transform (⌘T, Edit › Transform, the Move tool's transform controls)
/// starts on a layer locked in place, by its own lock or a group's.
pub const LOCKED: &str = "Could not complete the Free Transform command because the layer is locked.";

/// Why Edit › Transform's modes and Warp refuse while Content-Aware Scale's box is open.
pub const CAS_ONLY_SCALES: &str = "Content-Aware Scale only scales: commit or cancel it first";

/// Grab radius of the box's handles, in screen points. Generous, so a corner is easy to catch;
/// just beyond it, outside the box, a drag rotates.
pub const HANDLE_PX: f64 = 12.0;

/// [`HANDLE_PX`] in document pixels at the current zoom.
pub fn handle_tolerance(app: &PhotocraftApp) -> f64 {
    HANDLE_PX / app.point_zoom().max(0.01) as f64
}

/// End a handle drag whose release a modal sampler consumed, preserving the transform. Its snapping
/// ends too, so the canvas stops drawing the drag's alignment lines.
pub(crate) fn release_handle_drag(app: &mut PhotocraftApp) {
    if let Some(preview) = app.transform_preview.as_mut() {
        preview.gesture = None;
    }
    app.prefs_rt.snap = None;
    app.prefs_rt.snap_lines.clear();
}

/// The box as one undo step restores it.
#[derive(Clone, Debug, PartialEq)]
struct Step {
    rect: [f64; 4],
    quad: [[f64; 2]; 4],
    pivot: [f64; 2],
    warp: Option<Warp>,
}

impl Step {
    fn of(t: &TransformSession) -> Self {
        Step { rect: t.rect, quad: t.quad, pivot: t.pivot, warp: t.warp.clone() }
    }

    fn restore(self, t: &mut TransformSession) {
        (t.rect, t.quad, t.pivot, t.warp) = (self.rect, self.quad, self.pivot, self.warp);
    }
}

/// The session's own history: inside Free Transform, Undo steps back through the
/// handle drags, nudges and option edits rather than the document's history.
#[derive(Default)]
struct Steps {
    /// The box when it last came to rest (set when the session starts).
    settled: Option<Step>,
    undo: Vec<Step>,
    redo: Vec<Step>,
    /// When the latest arrow-key nudge came (egui time) and the box it left.
    nudged: Option<(f64, Step)>,
    /// The nudge that moved the box since it last came to rest follows another one closely: it
    /// joins that one's step instead of adding its own (#2259).
    fold: bool,
}

impl Steps {
    /// An arrow-key nudge at `now` moved the box from `before` to `after`. With `pause` (seconds,
    /// `None`: nudges aren't bundled) it folds into the previous nudge's step when it came within
    /// that time of it and found the box as that nudge left it, so a drag, an option edit, Undo
    /// or Redo in between start a step of their own.
    fn nudged(&mut self, before: &Step, after: Step, now: f64, pause: Option<f64>) {
        self.fold = pause.is_some_and(|w| self.nudged.as_ref().is_some_and(|(at, left)| left == before && now >= *at && now - at <= w));
        self.nudged = pause.map(|_| (now, after));
    }

    /// Record a step if the box changed since it last came to rest.
    fn settle(&mut self, now: Step) {
        if self.settled.as_ref() == Some(&now) {
            return;
        }
        if std::mem::take(&mut self.fold) && self.settled.is_some() {
            self.settled = Some(now);
            return;
        }
        if let Some(prev) = self.settled.replace(now) {
            self.undo.push(prev);
            self.redo.clear();
        }
    }

    /// Something to undo, counting a change not yet recorded (a nudge this frame).
    fn can_undo(&self, now: &Step) -> bool {
        !self.undo.is_empty() || self.settled.as_ref().is_some_and(|p| p != now)
    }
}

pub(crate) fn corners(r: [f64; 4]) -> [[f64; 2]; 4] {
    [[r[0], r[1]], [r[2], r[1]], [r[2], r[3]], [r[0], r[3]]]
}

/// The transform choices supported by the options bar, using its remembered choice or the general default.
pub(crate) fn default_interpolation(app: &PhotocraftApp) -> &'static str {
    use photocraft_engine::prefs::Interpolation;
    match app.ui.tool_options.transform_interpolation.unwrap_or(app.session.prefs().general.image_interpolation) {
        Interpolation::Nearest => "nearest",
        Interpolation::Bilinear => "bilinear",
        _ => "bicubic",
    }
}

#[cfg(test)]
mod tests;
