//! Vector tools: Shape tools (U), Pen (P), Path Selection (A), path overlays and the Paths panel.
//! Direct Selection (A) lives in `direct_select`.
//! All edits go through the engine's `shape.*` / `path.*` commands.

#[cfg(test)]
use egui::Pos2;
use egui::{Color32, Rect, Sense, Stroke, pos2, vec2};
use photocraft_doc::vector::Path;
use photocraft_doc::{Document, LayerContent};
use photocraft_geom::Affine;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::canvas::ViewXform;
use crate::shape_gradient_ui::GradientChoices;
use crate::shape_paint_ui::{PaintChange, paint_param, swatch};
use crate::state::{Tool, ToolOptions};
use crate::theme::Tokens;

mod curvature;
mod options;
mod overlay;
mod path_ops;
mod paths_panel;
mod pen;
mod shapes;

pub(crate) use curvature::curvature_anchor_at;
pub use curvature::{curvature_down, curvature_move, curvature_toggle, curvature_up};
#[cfg(test)]
use options::{corner_radii_patch, solid_paint};
pub(crate) use options::{enabled_stroke_width, paint_gesture};
pub use options::{options_bar, shape_properties};
pub use overlay::draw_overlay;
#[cfg(test)]
use overlay::{MAX_CURVE_PIECES, bezier};
#[cfg(test)]
pub(crate) use overlay::{curve_points, path_lines, selected_saved_path, shows_anchors};
pub(crate) use overlay::{draw_anchor, draw_handle, draw_outline};
#[cfg(test)]
use path_ops::target_path;
pub use path_ops::{active_path_name, path_selection_finish, path_to_selection, pick_shape_at, selected_document_path, takes_path_mask, with_active_path};
pub(crate) use path_ops::{
    active_shape_path, active_type_path, delete_key_deletes_work_path, deselect_loaded_path, free_transform_path, targeted_vector_mask, work_path_shown,
};
#[cfg(test)]
use paths_panel::path_context_actions;
pub use paths_panel::{PathEntry, PathRow, path_rows, paths_footer, paths_panel};
#[cfg(test)]
pub(crate) use pen::over_first_anchor;
#[cfg(test)]
use pen::pen_to_json;
pub use pen::{cancel_stale, closes_path_at, freeform_finish, pen_break_handle, pen_commit, pen_down, pen_grab_handle, pen_move, pen_undo_last_point, pen_up};
pub(crate) use pen::{pen_path, pen_replace};
pub use shapes::{create_shape, draw_shape_preview, finish_shape, is_shape_tool, shape_bounds};
#[cfg(test)]
use shapes::{shape_geometry, shape_preview};

/// Pen tool path under construction: knots as [anchor, in, out] (document px).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct PenPath {
    pub knots: Vec<[[f64; 2]; 3]>,
    /// Indices of cusp knots whose incoming and outgoing handles are not linked.
    #[serde(default)]
    pub unlinked: Vec<usize>,
    #[serde(skip)]
    pub dragging: bool,
    /// Re-dragging the final anchor changes its outgoing control only; the incoming curve stays put.
    #[serde(skip)]
    pub adjusting_last: bool,
    /// ⌥/Alt-dragging the final anchor's incoming handle: it turns alone (#6077).
    #[serde(skip)]
    pub adjusting_in: bool,
    /// Drawn with the Curvature Pen: handles follow the anchors (`curvature_refresh`) and
    /// `unlinked` lists its corner points.
    #[serde(default)]
    pub curvature: bool,
    /// Curvature Pen: the anchor being dragged.
    #[serde(skip)]
    pub moving: Option<usize>,
    /// Drawing carries on from an endpoint of this open subpath of a document path (#6593): the
    /// commit replaces that subpath instead of adding one.
    #[serde(default)]
    pub resumes: Option<Resumed>,
}

/// The subpath a Pen path carries on: where it is and what it was when drawing resumed. The
/// commit replaces it only while it is still that subpath of that document; otherwise (another
/// document, or the path changed meanwhile) the drawing is added as a new subpath.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Resumed {
    pub doc: u64,
    pub path: String,
    pub index: usize,
    pub was: photocraft_doc::vector::Subpath,
}

impl PenPath {
    /// Does `tool` keep drawing this path? Another tool finishes it.
    pub fn drawn_with(&self, tool: Tool) -> bool {
        tool == if self.curvature { Tool::CurvaturePen } else { Tool::Pen }
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod path_selection_tests;

#[cfg(test)]
mod pen_draw_tests;

#[cfg(test)]
mod paint_tests;
