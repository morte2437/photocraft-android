//! Direct Selection tool (A, #790), the Convert Point tool (#3204), the Pens' ⌘/Ctrl (Direct
//! Selection: the Pen, Freeform Pen and Curvature Pen) and the Pen's ⌥/Alt (Convert Point)
//! modifiers, as in Photoshop. It edits the paths the canvas shows: the targeted vector mask, the
//! active shape layer's path or the active type layer's Type Path, and the work path.
//!
//! * Click an anchor to select it (⇧ adds or removes, ⌥ selects its whole subpath), drag on empty
//!   canvas to marquee-select anchors, click empty canvas to deselect.
//! * Drag selected anchors to move them, a direction handle to turn it (a smooth point keeps its
//!   handles collinear; ⌥ breaks them apart), or a segment to bend it (straight segments move).
//!   The arrow keys nudge the selected anchors 1 px (⇧ 10 px), one history step per press.
//! * Convert Point (the tool, or Pen + ⌥): click an anchor to make it a corner, drag from it to
//!   pull out smooth handles, drag a handle to move it alone. The tool selects the anchor it
//!   converts, so its handles show; a press on empty canvas or a segment does nothing, and ⌘/Ctrl
//!   is Direct Selection, as with the Pen.
//! * While a Pen path is being drawn, ⌘/Ctrl edits it in place (its anchors, handles and
//!   segments) and the Pen carries on drawing from its last anchor once ⌘/Ctrl is let go; a
//!   ⌘/Ctrl-press anywhere else ends it, as in Photoshop (#4022).
//!
//! A drag previews with `photocraft_vector::edit` and commits once on release through the engine
//! (`path.moveAnchors`, `path.moveHandle`, `path.bendSegment`, `path.convertPoint`): one history
//! step per drag, and the same commands drive it over MCP and the control channel. The Pen path
//! being drawn is not in the document yet: its edits apply to it directly, without a command.

use egui::{Color32, Modifiers, Pos2};
use photocraft_doc::vector::Path;
use photocraft_geom::Point;
use photocraft_vector::edit::{self, Handle, Hit};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::canvas::ToolEvent;
use crate::state::Tool;

/// A path Direct Selection edits.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PathRef {
    Work,
    /// A layer's shape path, or its vector mask.
    Layer(u64),
    /// A type layer's Type Path (#9114), apart from a vector mask the layer may have.
    TypePath(u64),
    /// A saved path, by its index in the document's `paths`: the one selected in the Paths panel.
    Saved(usize),
    /// The Pen path being drawn (`ui.pen`): edited in place, not through the engine.
    Pen,
}

impl PathRef {
    /// The engine's target params for this path. A saved path goes by its name; one that is gone
    /// gets an empty name, which the engine refuses.
    pub(crate) fn params(self, app: &PhotocraftApp) -> Value {
        match self {
            PathRef::Work => json!({"name": "work"}),
            PathRef::Layer(id) => json!({"name": "layer", "layer": id}),
            PathRef::TypePath(id) => json!({"name": "layer", "layer": id, "typePath": true}),
            PathRef::Saved(i) => {
                let name = app.session.active().and_then(|st| st.doc.paths.get(i)).map(|p| p.name.clone()).unwrap_or_default();
                json!({"name": name})
            }
            // Never sent: edits to the Pen path being drawn don't go through the engine.
            PathRef::Pen => json!({}),
        }
    }
}

/// Direct Selection state: which anchors are selected and the drag in progress.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DirectSelection {
    /// The path whose anchors are selected.
    pub target: Option<PathRef>,
    /// Selected anchors as `[subpath, knot]`.
    pub anchors: Vec<[usize; 2]>,
    /// A clicked segment (`[subpath, knot]` it leaves): its two handles show.
    pub segment: Option<[usize; 2]>,
    #[serde(skip)]
    pub drag: Option<Drag>,
    /// The history step the drag's commit joins (`coalesce`): a Curvature Pen click on a segment
    /// adds the anchor it then drags, and both are one step.
    #[serde(skip)]
    pub coalesce: Option<String>,
    #[serde(skip)]
    pub(crate) owner: Option<Owner>,
    #[serde(skip)]
    pub(crate) cancelled_drag: bool,
}

/// Runtime selection ownership is never restored from settings onto another document.
#[derive(Clone, Debug)]
pub(crate) struct Owner {
    document: photocraft_doc::DocId,
    source: std::sync::Weak<photocraft_doc::Document>,
    revision: u64,
    layer: Option<photocraft_doc::LayerId>,
    selected_path: Option<String>,
    vector_mask: bool,
    tool: Tool,
}

impl PartialEq for Owner {
    fn eq(&self, other: &Self) -> bool {
        self.source.ptr_eq(&other.source)
            && (self.document, self.revision, self.layer, &self.selected_path, self.vector_mask, self.tool)
                == (other.document, other.revision, other.layer, &other.selected_path, other.vector_mask, other.tool)
    }
}

impl Owner {
    pub(crate) fn current(app: &PhotocraftApp) -> Option<Self> {
        let st = app.session.active()?;
        Some(Self {
            document: st.doc.id,
            source: std::sync::Arc::downgrade(&st.doc),
            revision: st.revision,
            layer: st.active_layer,
            selected_path: app.ui.selected_path.clone(),
            vector_mask: app.ui.vector_mask_target || st.channel_view.target == photocraft_engine::channel_cmds::ChannelTarget::VectorMask,
            tool: app.ui.tool,
        })
    }

    fn same_target(&self, app: &PhotocraftApp) -> bool {
        self.same_path_target(app) && app.ui.tool == self.tool
    }

    fn same_path_target(&self, app: &PhotocraftApp) -> bool {
        app.session.active().is_some_and(|st| {
            st.doc.id == self.document
                && st.active_layer == self.layer
                && (app.ui.vector_mask_target || st.channel_view.target == photocraft_engine::channel_cmds::ChannelTarget::VectorMask) == self.vector_mask
        }) && app.ui.selected_path == self.selected_path
    }

    fn valid(&self, app: &PhotocraftApp) -> bool {
        self.valid_path(app) && app.ui.tool == self.tool
    }

    fn valid_path(&self, app: &PhotocraftApp) -> bool {
        self.same_path_target(app)
            && app.session.active().is_some_and(|st| st.revision == self.revision && self.source.ptr_eq(&std::sync::Arc::downgrade(&st.doc)))
    }
}

/// Is `ds` still about the document, revision and target it was made on? Convert Point works on
/// the handles selected with Direct Selection, so an idle selection crosses those two tools; a
/// drag keeps its own tool.
pub(crate) fn still_valid(app: &PhotocraftApp, ds: &DirectSelection) -> bool {
    ds.owner.as_ref().is_some_and(|owner| owner.valid(app) || (ds.drag.is_none() && owns(owner.tool) && owns(app.ui.tool) && owner.valid_path(app)))
}

/// Clear stale anchors and gestures, keeping the pointer until the old release arrives. True
/// when a drag was cancelled.
pub(crate) fn cancel_stale(app: &mut PhotocraftApp) -> bool {
    let ds = &app.ui.direct_selection;
    let empty = ds.target.is_none() && ds.anchors.is_empty() && ds.segment.is_none() && ds.drag.is_none();
    if (ds.owner.is_none() && empty) || still_valid(app, ds) {
        return false;
    }
    let cancelled_drag = ds.cancelled_drag || ds.drag.is_some();
    app.ui.direct_selection = DirectSelection { cancelled_drag, ..Default::default() };
    cancelled_drag
}

/// Rebind a selection to the revision its own command made; another target or tool drops it.
pub(crate) fn keep_after_command(app: &mut PhotocraftApp, mut selection: DirectSelection) {
    if selection.owner.as_ref().is_some_and(|owner| owner.same_target(app)) {
        selection.owner = Owner::current(app);
        app.ui.direct_selection = selection;
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Drag {
    /// Editing `target`: `base` is its path at the press, `preview` the edit so far.
    Edit { target: PathRef, base: Path, edit: Edit, start: [f64; 2], end: [f64; 2], preview: Option<Path> },
    /// Marquee-selecting anchors (⇧ adds to the selection).
    Marquee { start: [f64; 2], end: [f64; 2], add: bool },
}

#[derive(Clone, Debug, PartialEq)]
pub enum Edit {
    Anchors(Vec<[usize; 2]>),
    /// The Curvature Pen dragging anchors: the smooth knots around them reflow.
    Curvature(Vec<[usize; 2]>),
    /// A handle, where it was at the press, and whether it moves alone (⌥).
    Handle([usize; 2], Handle, Point, bool),
    Segment([usize; 2], f64),
    /// Pen + ⌥ on an anchor: a click makes a corner, a drag pulls out smooth handles.
    Convert([usize; 2]),
}

impl Edit {
    /// The drag's end with ⇧ held: moving anchors locks to multiples of 45°, as in Photoshop (#5475).
    fn constrain(&self, start: [f64; 2], p: [f64; 2], shift: bool) -> [f64; 2] {
        if shift && matches!(self, Edit::Anchors(_)) { crate::stroke_constraint::snap45(start, p) } else { p }
    }

    /// The edit for a drag from `start` to `end`, as a command and its params (without target).
    fn command(&self, start: [f64; 2], end: [f64; 2]) -> (&'static str, Value) {
        let d = [end[0] - start[0], end[1] - start[1]];
        match self {
            Edit::Anchors(a) => ("path.moveAnchors", json!({"anchors": a, "move": d})),
            Edit::Curvature(a) => ("path.moveAnchors", json!({"anchors": a, "move": d, "curvature": true})),
            Edit::Handle([s, k], h, from, independent) => {
                ("path.moveHandle", json!({"subpath": s, "knot": k, "handle": h.name(), "to": [from.x + d[0], from.y + d[1]], "independent": independent}))
            }
            Edit::Segment([s, k], t) => ("path.bendSegment", json!({"subpath": s, "knot": k, "t": t, "move": d})),
            Edit::Convert([s, k]) => ("path.convertPoint", json!({"subpath": s, "knot": k, "out": end})),
        }
    }

    /// `base` edited as the command would: the drag preview.
    fn preview(&self, base: &Path, start: [f64; 2], end: [f64; 2]) -> Option<Path> {
        let d = [end[0] - start[0], end[1] - start[1]];
        let mut p = base.clone();
        match self {
            Edit::Anchors(a) => edit::move_anchors(&mut p, a, d),
            Edit::Curvature(a) => edit::move_anchors_curvature(&mut p, a, d),
            Edit::Handle(k, h, from, independent) => edit::move_handle(&mut p, *k, *h, Point::new(from.x + d[0], from.y + d[1]), *independent),
            Edit::Segment(k, t) => edit::bend_segment(&mut p, *k, *t, d),
            Edit::Convert(k) => edit::convert_point(&mut p, *k, Some(Point::new(end[0], end[1]))),
        }
        .ok()?;
        Some(p)
    }
}

/// Is Direct Selection what the pointer does now: its own tool, or a Pen (the Pen, Freeform Pen
/// or Curvature Pen), an anchor point tool or Convert Point with ⌘/Ctrl held.
fn direct(tool: Tool, mods: Modifiers) -> bool {
    tool == Tool::DirectSelection
        || (matches!(tool, Tool::Pen | Tool::FreeformPen | Tool::CurvaturePen | Tool::AddAnchorPoint | Tool::DeleteAnchorPoint | Tool::ConvertPoint)
            && mods.command)
}

/// Is Convert Point what the pointer does now: its own tool, or the Pen with ⌥ held while no path
/// is being drawn. ⌘/Ctrl wins over both (Direct Selection).
fn converting(tool: Tool, mods: Modifiers, pen_idle: bool) -> bool {
    !mods.command && (tool == Tool::ConvertPoint || (tool == Tool::Pen && mods.alt && pen_idle))
}

/// Is the Curvature Pen idle (no path being drawn, no ⌘/Ctrl)? Then it edits the active path in
/// place, as in Photoshop: a press on one of its anchors drags it, with no modifier.
fn curvature_idle(app: &PhotocraftApp, mods: Modifiers) -> bool {
    app.ui.tool == Tool::CurvaturePen && !mods.command && app.ui.pen.as_ref().is_none_or(|p| p.knots.is_empty())
}

/// The tools that take every pointer event on the canvas, not just a press that hits a path.
fn owns(tool: Tool) -> bool {
    matches!(tool, Tool::DirectSelection | Tool::ConvertPoint)
}

/// Does the canvas draw Direct Selection's overlay instead of the plain path outlines? The Add /
/// Delete Anchor Point tools show it too: they edit the same paths.
pub fn shows(app: &PhotocraftApp, mods: Modifiers) -> bool {
    direct(app.ui.tool, mods)
        || curvature_idle(app, mods)
        || app.ui.direct_selection.drag.is_some()
        || matches!(app.ui.tool, Tool::AddAnchorPoint | Tool::DeleteAnchorPoint | Tool::ConvertPoint)
}

/// The active paths, front first: the targeted vector mask, the active shape layer's path or the
/// active type layer's Type Path (#9114), then the work path. The Pen's Auto Add/Delete edits these
/// (`anchor_tools::pen_auto`).
pub(crate) fn active_paths(app: &PhotocraftApp) -> Vec<(PathRef, Path)> {
    let layer = crate::vector_ui::targeted_vector_mask(app)
        .or_else(|| crate::vector_ui::active_shape_path(app))
        .map(|(id, p)| (PathRef::Layer(id), p))
        .or_else(|| crate::vector_ui::active_type_path(app).map(|(id, p)| (PathRef::TypePath(id), p)));
    let saved = crate::vector_ui::selected_saved_path_index(app).map(|(i, p)| (PathRef::Saved(i), p));
    let work = app.session.active().and_then(|st| st.doc.work_path.clone()).filter(|_| crate::vector_ui::work_path_shown(app)).map(|p| (PathRef::Work, p));
    let mut out: Vec<_> = saved.into_iter().chain(layer).chain(work).collect();
    if app.ui.selected_path.as_deref() == Some("work")
        && let Some(index) = out.iter().position(|(target, _)| *target == PathRef::Work)
    {
        out.swap(0, index);
    }
    out.retain(|(target, _)| writable(app, target));
    out
}

/// The paths Direct Selection edits, front first: the Pen path being drawn, the active paths, every
/// visible shape layer, then the work path.
pub(crate) fn candidates(app: &PhotocraftApp) -> Vec<(PathRef, Path)> {
    let active = app.session.active().map(|st| st.doc.id);
    let pen = app
        .ui
        .pen
        .as_ref()
        .filter(|_| app.ui.pen_doc.is_none_or(|owner| Some(owner) == active))
        .filter(|p| !p.dragging && !p.knots.is_empty())
        .map(|p| (PathRef::Pen, crate::vector_ui::pen_path(p)));
    let mut out: Vec<_> = pen.into_iter().chain(active_paths(app)).collect();
    if let Some(st) = app.session.active() {
        for (path, _, l) in st.doc.walk().iter().rev() {
            if !l.visible || !(1..=path.len()).all(|n| st.doc.layer_at(&path[..n]).is_some_and(|a| a.visible)) {
                continue;
            }
            if let photocraft_doc::LayerContent::Shape(sh) = &l.content
                && !out.iter().any(|(r, _)| *r == PathRef::Layer(l.id.0))
            {
                out.push((PathRef::Layer(l.id.0), sh.path.clone()));
            }
        }
        if !out.iter().any(|(r, _)| *r == PathRef::Work) {
            out.extend(st.doc.work_path.clone().map(|p| (PathRef::Work, p)));
        }
    }
    out
}

/// Can `target` be edited: a layer's path only while the layer is not locked.
pub(crate) fn writable(app: &PhotocraftApp, target: &PathRef) -> bool {
    match target {
        PathRef::Layer(id) | PathRef::TypePath(id) => app.session.active().is_some_and(|st| {
            let id = photocraft_doc::LayerId(*id);
            let locks = st.doc.effective_locks(id);
            st.doc.layer(id).is_some() && !locks.all && !locks.pixels
        }),
        _ => app.session.active().is_some(),
    }
}

/// The frontmost editable path under `p` and what is hit on it: the handles shown on a selection
/// still valid count when `handles` is set. The hover and the press share it.
pub(crate) fn hit_target(app: &PhotocraftApp, p: [f64; 2], handles: bool) -> Option<(PathRef, Path, Hit)> {
    let tol = 6.0 / f64::from(app.point_zoom().max(0.01));
    let ds = &app.ui.direct_selection;
    let valid = handles && still_valid(app, ds);
    candidates(app).into_iter().filter(|(r, _)| writable(app, r)).find_map(|(r, path)| {
        let shown = if valid && ds.target.as_ref() == Some(&r) { shown_handles(ds, &path) } else { Vec::new() };
        edit::hit(&path, Point::new(p[0], p[1]), tol, &shown).map(|h| (r, path, h))
    })
}

fn retracted(k: &photocraft_doc::Knot, h: Handle) -> bool {
    let c = if h == Handle::In { k.in_ctrl } else { k.out_ctrl };
    (c.x - k.anchor.x).abs() < 1e-9 && (c.y - k.anchor.y).abs() < 1e-9
}

/// The handles shown on `path`: both of every selected anchor and the two of a clicked segment
/// (retracted handles sit on their anchor and don't show).
fn shown_handles(ds: &DirectSelection, path: &Path) -> Vec<([usize; 2], Handle)> {
    let knot = |r: [usize; 2]| path.subpaths.get(r[0]).and_then(|s| s.knots.get(r[1]));
    let mut out: Vec<([usize; 2], Handle)> = ds.anchors.iter().flat_map(|&r| [(r, Handle::In), (r, Handle::Out)]).collect();
    if let Some([s, k]) = ds.segment
        && let Some(sp) = path.subpaths.get(s)
    {
        let n = sp.knots.len();
        let next = k.checked_add(1).filter(|&j| j < n).or((sp.closed && n > 1).then_some(0));
        out.push(([s, k], Handle::Out));
        out.extend(next.map(|j| ([s, j], Handle::In)));
    }
    out.retain(|&(r, h)| knot(r).is_some_and(|k| !retracted(k, h)));
    out
}

/// Canvas pointer events for Direct Selection and the Pen's ⌘ / ⌥ modes; false when the event is
/// not theirs.
pub fn pointer(app: &mut PhotocraftApp, ev: ToolEvent, mods: Modifiers) -> bool {
    cancel_stale(app);
    if app.ui.direct_selection.cancelled_drag {
        // The rest of a gesture cancelled on another document or target goes nowhere.
        match ev {
            ToolEvent::Down { .. } => app.ui.direct_selection.cancelled_drag = false,
            ToolEvent::Up { .. } => {
                app.ui.direct_selection.cancelled_drag = false;
                return true;
            }
            ToolEvent::Move { .. } => return true,
        }
    }
    let tool = app.ui.tool;
    match ev {
        ToolEvent::Down { x, y, .. } => {
            if curvature_idle(app, mods) {
                return curvature_press(app, [x, y]);
            }
            let pen_idle = app.ui.pen.is_none();
            // Pen + ⌥ is Convert Point only while no path is being drawn.
            let convert = converting(tool, mods, pen_idle);
            if !(direct(tool, mods) || convert) {
                return false;
            }
            if !pen_idle && tool != Tool::DirectSelection && !hits_pen(app, [x, y]) {
                // ⌘-click off the path being drawn ends it, which becomes editable, as in
                // Photoshop. A press on it edits it in place and the Pen carries on (#4022). A
                // refused commit keeps the draft and the press does nothing more.
                if !crate::vector_ui::pen_commit(app, false) {
                    app.ui.direct_selection.cancelled_drag = true;
                    return true;
                }
            }
            if tool == Tool::FreeformPen {
                // The same for a Freeform Pen path waiting for its next ⌥-click.
                crate::lasso_ui::commit(app);
                if app.ui.pen.is_some() {
                    app.ui.direct_selection.cancelled_drag = true;
                    return true;
                }
            }
            press(app, [x, y], mods, convert)
        }
        ToolEvent::Move { x, y, .. } => {
            let ds = &mut app.ui.direct_selection;
            match &mut ds.drag {
                Some(Drag::Edit { base, edit, start, end, preview, .. }) => {
                    *end = edit.constrain(*start, [x, y], mods.shift);
                    *preview = edit.preview(base, *start, *end);
                }
                Some(Drag::Marquee { end, .. }) => *end = [x, y],
                None => return owns(tool),
            }
            true
        }
        ToolEvent::Up { x, y } => match app.ui.direct_selection.drag.take() {
            Some(Drag::Edit { target, base, edit, start, .. }) => {
                let end = edit.constrain(start, [x, y], mods.shift);
                release(app, target, &base, &edit, start, end);
                app.ui.direct_selection.coalesce = None;
                true
            }
            Some(Drag::Marquee { start, add, .. }) => {
                marquee(app, start, [x, y], add);
                true
            }
            None => owns(tool),
        },
    }
}

/// Does a press at `p` hit the Pen path being drawn: one of its anchors, shown handles or segments?
fn hits_pen(app: &PhotocraftApp, p: [f64; 2]) -> bool {
    hit_target(app, p, true).is_some_and(|(r, ..)| r == PathRef::Pen)
}

/// A press: pick what's under the pointer and start a drag. False lets the Pen handle a ⌥-press
/// that hit nothing; the Convert Point tool has nothing else to do, so it keeps the press.
fn press(app: &mut PhotocraftApp, p: [f64; 2], mods: Modifiers, convert: bool) -> bool {
    let own_tool = app.ui.tool == Tool::ConvertPoint;
    let Some((target, base, hit)) = hit_target(app, p, true) else {
        if convert {
            return own_tool;
        }
        let owner = Owner::current(app);
        let ds = &mut app.ui.direct_selection;
        ds.drag = Some(Drag::Marquee { start: p, end: p, add: mods.shift });
        ds.owner = owner;
        return true;
    };
    if let PathRef::Layer(id) | PathRef::TypePath(id) = target
        && app.session.active().and_then(|st| st.active_layer).map(|l| l.0) != Some(id)
    {
        let _ = app.run("layer.select", json!({"layer": id}));
    }
    let owner = Owner::current(app);
    let ds = &mut app.ui.direct_selection;
    if ds.target.as_ref() != Some(&target) {
        *ds = DirectSelection { target: Some(target), ..Default::default() };
    }
    ds.owner = owner;
    let edit = match hit {
        Hit::Handle(k, h) => {
            let kn = base.subpaths.get(k[0]).and_then(|s| s.knots.get(k[1]));
            let from = kn.map_or(Point::new(p[0], p[1]), |kn| if h == Handle::In { kn.in_ctrl } else { kn.out_ctrl });
            // Converting moves a handle on its own, as ⌥ does with Direct Selection.
            Edit::Handle(k, h, from, mods.alt || convert)
        }
        Hit::Anchor(k) if convert => {
            if own_tool {
                // The tool selects the anchor it works on, so the anchor's handles show.
                ds.segment = None;
                ds.anchors = vec![k];
            }
            Edit::Convert(k)
        }
        // Pen + ⌥ on a segment does nothing.
        Hit::Segment(..) if convert => return true,
        Hit::Anchor(k) => {
            ds.segment = None;
            if mods.alt {
                ds.anchors = edit::subpath_anchors(&base, k[0]);
            } else if mods.shift {
                // ⇧-click toggles the anchor; a click that deselects drags nothing.
                if let Some(i) = ds.anchors.iter().position(|a| *a == k) {
                    ds.anchors.remove(i);
                    return true;
                }
                ds.anchors.push(k);
            } else if !ds.anchors.contains(&k) {
                ds.anchors = vec![k];
            }
            Edit::Anchors(ds.anchors.clone())
        }
        Hit::Segment(k, _) if mods.alt => {
            ds.segment = None;
            ds.anchors = edit::subpath_anchors(&base, k[0]);
            Edit::Anchors(ds.anchors.clone())
        }
        Hit::Segment(k, t) => {
            ds.anchors.clear();
            ds.segment = Some(k);
            Edit::Segment(k, t)
        }
    };
    ds.drag = Some(Drag::Edit { target, base, edit, start: p, end: p, preview: None });
    true
}

/// What the idle Curvature Pen has under document point `p` on the active paths: an anchor or a
/// segment, with the path it is on. Handles don't count.
fn curvature_target(app: &PhotocraftApp, p: [f64; 2]) -> Option<(PathRef, Path, Hit)> {
    let tol = 6.0 / f64::from(app.point_zoom().max(0.01));
    active_paths(app).into_iter().find_map(|(r, path)| match edit::hit(&path, Point::new(p[0], p[1]), tol, &[]) {
        Some(h @ (Hit::Anchor(_) | Hit::Segment(..))) => Some((r, path, h)),
        _ => None,
    })
}

/// What a Curvature Pen press at `p` would do to an existing point, for its cursor: the anchor
/// under it (idle: one of the active path's; drawing: one of the path being drawn), or the active
/// path's segment it would add an anchor to.
pub fn curvature_hover(app: &PhotocraftApp, p: [f64; 2], mods: Modifiers) -> Option<Hit> {
    if app.ui.tool != Tool::CurvaturePen || mods.command {
        return None;
    }
    if !curvature_idle(app, mods) {
        return crate::vector_ui::curvature_anchor_at(app, p[0], p[1]).map(|i| Hit::Anchor([0, i]));
    }
    curvature_target(app, p).map(|(.., h)| h)
}

/// The idle Curvature Pen's double-click on an anchor of the active path: smooth ↔ corner, and
/// the subpath reflows (one `path.convertPoint` with `curvature`).
pub(crate) fn curvature_toggle_active(app: &mut PhotocraftApp, p: [f64; 2]) {
    let Some((target, _, Hit::Anchor([s, k]))) = curvature_target(app, p) else { return };
    let mut params = json!({"subpath": s, "knot": k, "curvature": true});
    if let (Some(o), Some(tp)) = (params.as_object_mut(), target.params(app).as_object()) {
        o.extend(tp.clone());
    }
    if let Err(e) = app.run("path.convertPoint", params) {
        app.ui.status = e;
        app.ui.status_error = true;
    }
    app.ui.direct_selection = DirectSelection { target: Some(target), anchors: vec![[s, k]], ..Default::default() };
}

/// Curvature Pen press with no path being drawn, on the active path: an anchor is grabbed and
/// dragged; a segment gets a new anchor there (`path.addAnchor`), dragged the same way. Either
/// way the curve reflows smoothly through the moved anchor, as in Photoshop (`path.moveAnchors`
/// with `curvature` on release), and an added anchor and its drag are one history step. False
/// when the press hits neither, so it starts a new path.
fn curvature_press(app: &mut PhotocraftApp, p: [f64; 2]) -> bool {
    let Some((target, mut base, hit)) = curvature_target(app, p) else { return false };
    let mut coalesce = None;
    let k = match hit {
        Hit::Anchor(k) => k,
        Hit::Segment([s, k], t) => {
            let key = format!("curvature-add-{}", app.ui.alloc_id());
            let mut params = json!({"subpath": s, "knot": k, "t": t.clamp(0.01, 0.99), "coalesce": key});
            if let (Some(o), Some(tp)) = (params.as_object_mut(), target.params(app).as_object()) {
                o.extend(tp.clone());
            }
            let added = match app.run("path.addAnchor", params) {
                Ok(v) => v,
                Err(e) => {
                    app.ui.status = e;
                    app.ui.status_error = true;
                    return true;
                }
            };
            let n = |f: &str| added["anchor"][f].as_u64().and_then(|v| usize::try_from(v).ok());
            let (Some(s), Some(k), Some(path)) = (n("subpath"), n("knot"), active_paths(app).into_iter().find(|(r, _)| *r == target).map(|(_, p)| p)) else {
                return true;
            };
            base = path;
            coalesce = Some(key);
            [s, k]
        }
        Hit::Handle(..) => return false,
    };
    app.ui.direct_selection = DirectSelection { target: Some(target), anchors: vec![k], coalesce, owner: Owner::current(app), ..Default::default() };
    app.ui.direct_selection.drag = Some(Drag::Edit { target, base, edit: Edit::Curvature(vec![k]), start: p, end: p, preview: None });
    true
}

/// End of an edit drag: commit it as one command. A click (no movement) commits nothing, except
/// Pen + ⌥ on an anchor, which makes it a corner. The Pen path being drawn takes the edit directly.
fn release(app: &mut PhotocraftApp, target: PathRef, base: &Path, edit: &Edit, start: [f64; 2], end: [f64; 2]) {
    let moved = (end[0] - start[0]).hypot(end[1] - start[1]) * f64::from(app.point_zoom()) >= 1.0;
    if target == PathRef::Pen {
        let edited = match edit {
            Edit::Convert(k) if !moved => {
                let mut p = base.clone();
                edit::convert_point(&mut p, *k, None).ok().map(|_| p)
            }
            _ if !moved => None,
            e => e.preview(base, start, end),
        };
        if let Some(p) = edited {
            crate::vector_ui::pen_replace(app, &p);
        }
        return;
    }
    let (id, mut params) = match edit {
        Edit::Convert([s, k]) if !moved => ("path.convertPoint", json!({"subpath": s, "knot": k})),
        _ if !moved => return,
        e => e.command(start, end),
    };
    if let (Some(p), Some(t)) = (params.as_object_mut(), target.params(app).as_object()) {
        p.extend(t.clone());
    }
    if let (Some(p), Some(key)) = (params.as_object_mut(), app.ui.direct_selection.coalesce.take()) {
        p.insert("coalesce".into(), json!(key));
    }
    // sync_views checks ownership while the command runs: keep this selection off the live UI
    // until its own command is done, then bind it to the revision that made.
    let selection = std::mem::take(&mut app.ui.direct_selection);
    match app.run(id, params) {
        Ok(_) => keep_after_command(app, selection),
        Err(e) => {
            if selection.owner.as_ref().is_some_and(|owner| owner.valid(app)) {
                app.ui.direct_selection = selection;
            }
            app.ui.status = e;
            app.ui.status_error = true;
        }
    }
}

/// End of a marquee: select the anchors inside it on the frontmost path that has any (⇧ adds).
/// A click on empty canvas deselects.
fn marquee(app: &mut PhotocraftApp, start: [f64; 2], end: [f64; 2], add: bool) {
    let rect = [start[0], start[1], end[0], end[1]];
    let hits = candidates(app).into_iter().map(|(r, path)| (r, edit::anchors_in(&path, rect))).find(|(_, a)| !a.is_empty());
    let owner = Owner::current(app);
    let ds = &mut app.ui.direct_selection;
    match hits {
        Some((r, anchors)) if add && ds.target.as_ref() == Some(&r) => {
            for a in anchors {
                if !ds.anchors.contains(&a) {
                    ds.anchors.push(a);
                }
            }
            ds.segment = None;
        }
        Some((r, anchors)) => *ds = DirectSelection { target: Some(r), anchors, ..Default::default() },
        None if add => {}
        None => {
            ds.anchors.clear();
            ds.segment = None;
        }
    }
    ds.owner = owner;
}

/// Arrow keys with Direct Selection and selected anchors (#4486), as in Photoshop: nudge them
/// 1 px (⇧ 10 px) through `path.moveAnchors`, one history step per press. Not while a drag is
/// in progress. Returns true when a key was used.
pub fn arrow_keys(app: &mut PhotocraftApp, ctx: &egui::Context) -> bool {
    cancel_stale(app);
    let ds = &app.ui.direct_selection;
    let Some(target) = ds.target.filter(|_| app.ui.tool == Tool::DirectSelection && !ds.anchors.is_empty() && ds.drag.is_none()) else {
        return false;
    };
    // The anchors belong to a path the canvas no longer offers (another saved path picked in the
    // Paths panel, the path deleted): drop them rather than nudge that path, or another one that
    // took its index.
    if !candidates(app).iter().any(|(r, _)| *r == target) {
        app.ui.direct_selection = DirectSelection::default();
        return false;
    }
    let anchors = app.ui.direct_selection.anchors.clone();
    use egui::Key;
    let keys = [(Key::ArrowLeft, -1.0, 0.0), (Key::ArrowRight, 1.0, 0.0), (Key::ArrowUp, 0.0, -1.0), (Key::ArrowDown, 0.0, 1.0)];
    for (key, ux, uy) in keys {
        // ⇧ first: `consume_key` ignores an extra ⇧.
        for mods in [Modifiers::SHIFT, Modifiers::NONE] {
            if ctx.input_mut(|i| i.consume_key(mods, key)) {
                let k = if mods.shift { 10.0 } else { 1.0 };
                let mut params = json!({"anchors": anchors, "move": [ux * k, uy * k]});
                if let (Some(p), Some(t)) = (params.as_object_mut(), target.params(app).as_object()) {
                    p.extend(t.clone());
                }
                // The nudge makes a new revision: the selection follows it, as after a drag.
                let selection = std::mem::take(&mut app.ui.direct_selection);
                match app.run("path.moveAnchors", params) {
                    Ok(_) => keep_after_command(app, selection),
                    Err(e) => {
                        app.ui.direct_selection = selection;
                        app.ui.status = e;
                        app.ui.status_error = true;
                    }
                }
                return true;
            }
        }
    }
    false
}

/// Paths with their anchors (selected ones filled), the shown handles, the drag's preview in
/// place of its path, and the marquee. `paths` false (View › Show › Target Path off) leaves only
/// the marquee.
pub fn draw_overlay(app: &PhotocraftApp, painter: &egui::Painter, to_scr: &dyn Fn([f64; 2]) -> Pos2, accent: Color32, paths: bool) {
    let ds = &app.ui.direct_selection;
    // The idle Curvature Pen shows the anchors of the path it edits, not every shape's.
    let shown = if !paths {
        Vec::new()
    } else if curvature_idle(app, painter.ctx().input(|i| i.modifiers)) {
        active_paths(app)
    } else {
        candidates(app)
    };
    for (r, path) in shown {
        let path = match &ds.drag {
            Some(Drag::Edit { target, preview: Some(p), .. }) if *target == r => p.clone(),
            _ => path,
        };
        crate::vector_ui::draw_outline(painter, &path, to_scr, accent);
        let ours = ds.target.as_ref() == Some(&r);
        for &(k, h) in if ours { shown_handles(ds, &path) } else { Vec::new() }.iter() {
            if let Some(kn) = path.subpaths.get(k[0]).and_then(|s| s.knots.get(k[1])) {
                let c = if h == Handle::In { kn.in_ctrl } else { kn.out_ctrl };
                crate::vector_ui::draw_handle(painter, to_scr([kn.anchor.x, kn.anchor.y]), to_scr([c.x, c.y]), accent);
            }
        }
        for (s, sp) in path.subpaths.iter().enumerate() {
            for (k, kn) in sp.knots.iter().enumerate() {
                crate::vector_ui::draw_anchor(painter, to_scr([kn.anchor.x, kn.anchor.y]), ours && ds.anchors.contains(&[s, k]), accent);
            }
        }
    }
    if let Some(Drag::Marquee { start, end, .. }) = &ds.drag {
        let pts = [*start, [end[0], start[1]], *end, [start[0], end[1]]].map(to_scr);
        crate::tool_feedback::draw_ants(painter, &pts, true);
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod freeform_command_tests {
    use super::*;

    /// Is a Direct Selection drag under way on the document and target it started on?
    fn captured_drag(app: &PhotocraftApp) -> bool {
        app.ui.direct_selection.drag.is_some() && app.ui.direct_selection.owner.as_ref().is_some_and(|owner| owner.valid(app))
    }

    fn app(tool: Tool) -> PhotocraftApp {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
        for name in ["work", "Saved drawing"] {
            app.run("path.set", json!({"name": name, "path":{"subpaths":[{"knots":[[20,20],[120,20]]}]}})).unwrap();
        }
        app.sync_views();
        app.ui.tool = tool;
        app.ui.selected_path = Some("Saved drawing".into());
        app
    }

    #[test]
    fn command_with_freeform_and_curvature_edits_the_selected_saved_path_after_modifier_release() {
        for tool in [Tool::FreeformPen, Tool::CurvaturePen] {
            let mut app = app(tool);
            let original = app.session.active().unwrap().doc.clone();
            let n = app.session.active().unwrap().history.past_len();
            let command = Modifiers { command: true, ctrl: true, ..Default::default() };
            assert!(pointer(&mut app, ToolEvent::Down { x: 20.0, y: 20.0, pressure: 1.0 }, command));
            assert_eq!(app.ui.direct_selection.target, Some(PathRef::Saved(0)));
            assert!(captured_drag(&app));
            assert!(pointer(&mut app, ToolEvent::Move { x: 30.0, y: 40.0, pressure: 1.0 }, Modifiers::NONE));
            assert!(pointer(&mut app, ToolEvent::Up { x: 30.0, y: 40.0 }, Modifiers::NONE));
            let st = app.session.active().unwrap();
            assert_eq!(st.doc.work_path, original.work_path);
            assert_eq!(st.doc.paths[0].path.subpaths[0].knots[0].anchor, Point::new(30.0, 40.0));
            assert_eq!(st.history.past_len(), n + 1);
            assert!(app.ui.direct_selection.owner.as_ref().unwrap().valid(&app));
            assert!(!captured_drag(&app));
            assert!(app.session.undo());
            assert_eq!(app.session.active().unwrap().doc.paths, original.paths);
        }
    }

    #[test]
    fn command_cannot_edit_an_existing_path_when_the_freeform_trace_failed_to_finish() {
        let mut app = app(Tool::FreeformPen);
        let original = app.session.active().unwrap().doc.clone();
        let n = app.session.active().unwrap().history.past_len();
        for [x, y] in [[40.0, 60.0], [1e40, 60.0], [140.0, 80.0]] {
            assert!(crate::lasso_ui::pointer(&mut app, ToolEvent::Down { x, y, pressure: 1.0 }, Modifiers::ALT));
            assert!(crate::lasso_ui::pointer(&mut app, ToolEvent::Up { x, y }, Modifiers::ALT));
        }
        let command = Modifiers { command: true, ctrl: true, ..Default::default() };
        assert!(pointer(&mut app, ToolEvent::Down { x: 20.0, y: 20.0, pressure: 1.0 }, command));
        let pen = app.ui.pen.as_ref().expect("failed Freeform commit must preserve its fitted draft");
        assert!(pen.commit_failed && pen.owner.is_some());
        assert!(app.ui.direct_selection.drag.is_none());
        assert!(app.ui.direct_selection.cancelled_drag);
        assert!(pointer(&mut app, ToolEvent::Up { x: 30.0, y: 40.0 }, command));
        assert!(app.ui.direct_selection.drag.is_none());
        assert_eq!(app.session.active().unwrap().doc.paths, original.paths);
        assert_eq!(app.session.active().unwrap().doc.work_path, original.work_path);
        assert_eq!(app.session.active().unwrap().history.past_len(), n);
    }
}
