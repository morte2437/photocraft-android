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
pub(crate) use overlay::selected_saved_path_index;
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
pub(crate) use pen::{PATH_OPS, pen_path, pen_replace};
pub use pen::{
    cancel_stale, closes_path_at, freeform_finish, pen_break_handle, pen_commit, pen_down, pen_end, pen_grab_handle, pen_move, pen_undo_last_point, pen_up,
};
#[cfg(test)]
use pen::{closing, pen_path_op};
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
    /// Space moves the anchor and both handles together during a Pen drag.
    #[serde(skip)]
    pub reposition: bool,
    #[serde(skip)]
    pub last_pointer: Option<[f64; 2]>,
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
    /// The document and target the first anchor was placed on; a draft without one (restored
    /// from settings) is never committed.
    #[serde(skip)]
    pub(crate) owner: Option<PenOwner>,
    /// The last commit failed: the draft stays as it was and isn't retried on its own.
    #[serde(skip)]
    pub(crate) commit_failed: bool,
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
    /// A document path's name ("work" or a saved path's), or "layer" for `layer`'s.
    pub path: String,
    /// The shape layer (Shape mode) or targeted vector mask whose path it is.
    #[serde(default)]
    pub layer: Option<u64>,
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
mod pen_path_ops_tests;

#[cfg(test)]
mod paint_tests;

pub(crate) use pen::freeform_locked_shape;

/// Where a Pen draft goes, captured at its first anchor.
#[derive(Clone, Debug)]
pub(crate) struct PenOwner {
    document: DocId,
    revision: u64,
    source: Weak<Document>,
    target: PenTarget,
    options: ToolOptions,
    layer: Option<photocraft_doc::LayerId>,
    vector_mask: bool,
    foreground: [f32; 4],
    background: [f32; 4],
}

impl PartialEq for PenOwner {
    fn eq(&self, other: &Self) -> bool {
        self.document == other.document && self.revision == other.revision && self.source.ptr_eq(&other.source) && self.target == other.target
    }
}

#[derive(Clone, Debug, PartialEq)]
enum PenTarget {
    /// A new shape layer, with the paint chosen when drawing began.
    Shape {
        fill: Value,
        stroke: Value,
        open_stroke: Value,
    },
    VectorMask(u64),
    /// The work path or a saved path; `append` adds the draft as a subpath (#4721).
    Path {
        name: String,
        append: bool,
    },
}

/// Where the Pen draws now: a new shape layer (Shape mode), else the work or saved path selected
/// in the Paths panel (the new path joins it, #4721), else the targeted vector mask, else a new
/// Work Path that replaces the old one.
pub(crate) fn capture_pen_owner(app: &PhotocraftApp) -> Option<PenOwner> {
    let st = app.session.active()?;
    let target = if app.ui.tool_options.vector_mode == "shape" {
        let mut open_stroke = app.ui.tool_options.shape_stroke.params();
        open_stroke["width"] = json!(app.ui.tool_options.stroke_width.max(1.0));
        open_stroke["color"] = json!(app.session.tools.foreground);
        PenTarget::Shape { fill: fill_param(app), stroke: stroke_param(app), open_stroke }
    } else if let Some((name, _)) = selected_document_geometry(app) {
        PenTarget::Path { name, append: true }
    } else if let Some(id) = targeted_vector_mask_id(app) {
        PenTarget::VectorMask(id)
    } else {
        PenTarget::Path { name: "work".into(), append: false }
    };
    Some(PenOwner {
        document: st.doc.id,
        revision: st.revision,
        source: Arc::downgrade(&st.doc),
        target,
        options: app.ui.tool_options.clone(),
        layer: st.active_layer,
        vector_mask: app.ui.vector_mask_target,
        foreground: app.session.tools.foreground,
        background: app.session.tools.background,
    })
}

impl PenOwner {
    fn valid(&self, app: &PhotocraftApp) -> bool {
        app.session.active().is_some_and(|st| st.doc.id == self.document && st.revision == self.revision && self.source.ptr_eq(&Arc::downgrade(&st.doc)))
    }
}

use photocraft_doc::DocId;
use std::sync::{Arc, Weak};

use path_ops::{selected_document_geometry, targeted_vector_mask_id};
pub(crate) use pen::{finish_pen_if_left, freeform_finish_owned};
use shapes::{fill_param, stroke_param};

/// Commit a valid source-owned draft on a tab switch; otherwise cancel with an actionable diagnostic.
pub(crate) fn cancel_stale_pen(app: &mut PhotocraftApp) -> bool {
    let Some(pen) = app.ui.pen.as_ref() else { return false };
    if pen.owner.as_ref().is_some_and(|o| o.valid(app)) {
        return false;
    }
    if pen.commit_failed {
        return false;
    }
    let owner = pen.owner.clone();
    if let Some(owner) = owner.as_ref() {
        let index = app
            .session
            .documents()
            .iter()
            .position(|st| st.doc.id == owner.document && st.revision == owner.revision && owner.source.ptr_eq(&Arc::downgrade(&st.doc)));
        if let Some(index) = index {
            let active = app.session.active_index();
            let saved_ui = app.ui.clone();
            let draft = app.ui.pen.take();
            app.session.set_active(index);
            app.ui.pen = draft;
            // This source matches the immutable owner; normal commit validation still applies.
            let result = pen::try_pen_commit(app, false);
            if let Some(active) = active {
                app.session.set_active(active);
            }
            let failed = app.ui.pen.take();
            let message = app.ui.status.clone();
            // The temporary source commit ran sync_views on that document. Rebind its runtime
            // tracker before restoring the destination UI, so the next frame cannot clear
            // an unrelated saved-path selection in the destination tab.
            app.path_document = app.session.active().map(|st| (st.doc.id, st.revision, Arc::downgrade(&st.doc)));
            app.ui = saved_ui;
            app.ui.pen = if result.is_ok() { None } else { failed };
            if result.is_ok() && app.ui.direct_selection.target == Some(crate::direct_select::PathRef::Pen) {
                app.ui.direct_selection = Default::default();
            }
            if result.is_err() {
                app.ui.status = message;
                app.ui.status_error = true;
            }
            app.ui.pen_doc = app.ui.pen.as_ref().and_then(|p| p.owner.as_ref().map(|o| o.document));
            return true;
        }
    }
    app.ui.pen = None;
    app.ui.pen_doc = None;
    if app.ui.direct_selection.target == Some(crate::direct_select::PathRef::Pen) {
        app.ui.direct_selection = Default::default();
    }
    app.ui.status = "Pen draft cancelled because its source document or path changed; newer work was preserved".into();
    app.ui.status_error = true;
    true
}
