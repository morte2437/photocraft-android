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
mod drag_modes;
mod draw;
mod options_bar;
mod pointer;
mod preview;
mod warp;

pub(crate) use begin::begin_content_aware;
pub use begin::{begin, begin_copy, begin_placed, begin_selection, begin_warp, enter_warp, leave_warp, warps_type};
pub(crate) use commit::{SAVE_FOR_WEB, committing, nudge};
pub use commit::{cancel, commit, end_if_left, follow_edits, intercept, is_enabled, on_job_event, track_steps};
pub(crate) use drag_modes::{apply_drag, corner_distorts, legacy_mods};
#[cfg(test)]
pub(crate) use drag_modes::{apply_mode_drag, unlinked_mods};
#[cfg(test)]
use drag_modes::{keep_convex, turns};
#[cfg(test)]
pub(crate) use draw::{Readout, live_readout, styled_frame};
pub(crate) use draw::{carries_selection_of, draw_selection};
pub use draw::{draw_content, draw_overlay};
#[cfg(test)]
use draw::{front_vertices, handle_anchor};
pub(crate) use options_bar::position_size_fields;
#[cfg(test)]
use options_bar::{
    POSITION_LIMIT_PX, Parts, SCALE_LIMIT_PCT, SIZE_UNIT_KEY, SKEW_LIMIT_DEG, matching_anchor, readout, rotate_about_pivot, scale_about_pivot, scale_limit_pct,
    set_size_unit, size_pct, size_shown, size_unit, warp_grid_id,
};
pub use options_bar::{options_bar, reset_warp};
pub(crate) use pointer::{Gesture, Hit, hit, rotate_cursor_dir};
#[cfg(test)]
use pointer::{accepts_outside_click, carry_pivot, near_rotate_corner, point_on_quad, rotation_degrees};
pub use pointer::{cursor, pointer};
use preview::Above;
pub(crate) use preview::solo_layer;
#[cfg(test)]
use preview::{covering, preview_image, show_only};
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
}

/// What [`follow_edits`] needs to rebuild the preview document.
#[derive(Clone, Copy, Debug)]
struct Follow {
    /// The document revision `doc` was built from.
    revision: u64,
    /// The moving layer is hidden in `doc` (nothing was lifted out of it).
    hidden: bool,
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

fn corners(r: [f64; 4]) -> [[f64; 2]; 4] {
    [[r[0], r[1]], [r[2], r[1]], [r[2], r[3]], [r[0], r[3]]]
}

#[cfg(test)]
mod tests;
