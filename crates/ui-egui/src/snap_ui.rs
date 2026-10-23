//! Snapping in the shell: builds [`photocraft_engine::snap`] targets from View › Snap To and the
//! document, snaps tool gestures (Move tool, marquees, crop, shapes, pen anchors, Free Transform
//! handles and guides), and draws smart guides (magenta alignment lines) while layers move.
//!
//! Snapping is on when View › Snap (⇧⌘;) is; holding Ctrl while dragging turns it off, except when
//! Ctrl is the corner-distortion modifier. Smart guides follow View › Show › Smart Guides and also snap the Move
//! tool to other layers' edges and centres. The threshold is 8 screen pixels at the current zoom.

use egui::{Color32, Stroke, pos2};
use photocraft_doc::LayerId;
use photocraft_engine::snap::{SnapKind, SnapLine, SnapOptions, SnapTargets, layer_rect, union};

use crate::PhotocraftApp;
use crate::canvas::{ToolEvent, ViewXform};
use crate::state::Tool;

/// Snap distance in screen pixels.
pub const SNAP_PX: f64 = 8.0;

/// What the current drag snaps.
#[derive(Clone, Debug, PartialEq)]
pub enum Gesture {
    /// A point (marquee corner, crop or shape corner, pen anchor, transform handle).
    Point,
    /// A transform corner: distortion snaps the corner itself, keeping the handle grab offset.
    TransformCorner { index: usize, point: [f64; 2] },
    /// Layers being moved: their bounds at the start of the drag.
    Move { rect: [f64; 4] },
    /// Free Transform box dragged from inside: its bounds at the start of the drag.
    TransformMove { rect: [f64; 4] },
    /// A selection outline (or its floating piece) dragged with a selection tool: its bounds at
    /// the start of the drag.
    SelectionMove { rect: [f64; 4] },
    /// A marquee, Object Selection box or shape being drawn, repositioned with Space held
    /// (`hold_keys`): its bounds when Space went down.
    DrawMove { rect: [f64; 4] },
    /// A guide being moved with the Move tool.
    Guide,
}

/// Snapping state of the drag in progress.
#[derive(Clone, Debug)]
pub struct ActiveSnap {
    pub gesture: Gesture,
    pub start: [f64; 2],
    pub targets: SnapTargets,
    /// Layer alignments only (smart guides), when View › Snap and Smart Guides are on.
    pub smart: SnapTargets,
    pub disabled: bool,
}

/// Snap To options from the View menu and the Guides/Grid preferences.
pub fn options(app: &PhotocraftApp) -> SnapOptions {
    let st = &app.ui.view.snap_to;
    let e = &app.ui.extras;
    let grid_step = app.session.active().map_or([18.0; 2], |d| {
        let g = &app.session.prefs().guides_grid_and_slices;
        let ppi = app.session.prefs().units_and_rulers.point_size.per_inch();
        crate::rulers::grid_major_px(g, d.doc.resolution_dpi.max(1.0) as f64, [d.doc.size.width as f64, d.doc.size.height as f64], ppi)
            .map(|step| step / g.subdivisions.max(1) as f64)
    });
    SnapOptions {
        guides: st.guides && e.guides && app.ui.view.shows(app.ui.view.show.canvas_guides),
        grid: st.grid && e.grid,
        layers: st.layers,
        document: st.document_bounds,
        selection: true,
        grid_step,
    }
}

/// Snap tolerance in document pixels at the current zoom.
pub fn tolerance(app: &PhotocraftApp) -> f64 {
    SNAP_PX / app.point_zoom().max(0.01) as f64
}

fn smart_on(app: &PhotocraftApp) -> bool {
    app.ui.view.shows(app.ui.view.show.smart_guides)
}

fn snap_on(app: &PhotocraftApp) -> bool {
    app.ui.extras.snap
}

/// Targets of the active document (smart = layer alignments only).
fn build(app: &PhotocraftApp, exclude: &[LayerId], smart: bool) -> SnapTargets {
    let Some(st) = app.session.active() else { return SnapTargets::default() };
    let opts =
        if smart { SnapOptions { guides: false, grid: false, layers: true, document: true, selection: false, grid_step: [0.0; 2] } } else { options(app) };
    let t = SnapTargets::from_document(&st.doc, &opts, exclude);
    if smart { t.filtered(SnapKind::is_smart) } else { t }
}

/// What a move with `tool` in effect drags: the selection's bounds where its pixels float
/// (`move_ui`), else the union of the selected layers' bounds.
fn moving_rect(app: &PhotocraftApp, tool: Tool) -> Option<[f64; 4]> {
    let st = app.session.active()?;
    if crate::move_ui::moves_selected_pixels_with(app, tool) {
        return selection_rect(app);
    }
    union(st.selected_layers().into_iter().filter_map(|id| layer_rect(&st.doc, id)))
}

/// The bounds of the pixels a selection moves, where they are shown: its floating piece's outline
/// at its offset (#7538), else (nothing cut yet, or a mask or empty piece) the selection's bounds.
fn selection_rect(app: &PhotocraftApp) -> Option<[f64; 4]> {
    let st = app.session.active()?;
    let sel = st.doc.selection.as_ref()?;
    let r = match photocraft_engine::float_cmds::floating(st) {
        Some(f) => f.parts.outline().unwrap_or(sel).content_bounds().translate(f.offset.0, f.offset.1),
        None => sel.content_bounds(),
    };
    (!r.is_empty()).then(|| [r.x0, r.y0, r.x1, r.y1].map(f64::from))
}

/// A press cut the selected pixels after snapping began (`filter_event` runs before the tools):
/// the move now snaps the cut piece's outline, not the selection it was cut with (#7538).
pub(crate) fn follow_cut(app: &mut PhotocraftApp) {
    let Some(rect) = selection_rect(app) else { return };
    if let Some(snap) = app.prefs_rt.snap.as_mut() {
        match &mut snap.gesture {
            Gesture::Move { rect: r } | Gesture::SelectionMove { rect: r } => *r = rect,
            _ => {}
        }
    }
}

fn is_point_tool(t: Tool) -> bool {
    matches!(
        t,
        Tool::RectMarquee
            | Tool::EllipseMarquee
            | Tool::SingleRowMarquee
            | Tool::SingleColumnMarquee
            | Tool::Crop
            | Tool::ObjectSelection
            | Tool::Pen
            | Tool::CurvaturePen
            | Tool::Rectangle
            | Tool::EllipseShape
            | Tool::Triangle
            | Tool::Polygon
            | Tool::Line
            | Tool::Type
            | Tool::VerticalType
            | Tool::HorizontalTypeMask
            | Tool::VerticalTypeMask
    )
}

fn quad_rect(q: &[[f64; 2]; 4]) -> [f64; 4] {
    let xs = q.iter().map(|p| p[0]);
    let ys = q.iter().map(|p| p[1]);
    [xs.clone().fold(f64::MAX, f64::min), ys.clone().fold(f64::MAX, f64::min), xs.fold(f64::MIN, f64::max), ys.fold(f64::MIN, f64::max)]
}

/// Is `p` inside the quad (even-odd)?
fn in_quad(q: &[[f64; 2]; 4], p: [f64; 2]) -> bool {
    let mut inside = false;
    for i in 0..4 {
        let (a, b) = (q[i], q[(i + 3) % 4]);
        if (a[1] > p[1]) != (b[1] > p[1]) && p[0] < (b[0] - a[0]) * (p[1] - a[1]) / (b[1] - a[1]) + a[0] {
            inside = !inside;
        }
    }
    inside
}

/// Near a Free Transform corner or edge handle (within its grab radius)?
fn near_handle(q: &[[f64; 2]; 4], p: [f64; 2], tol: f64) -> bool {
    (0..4).any(|i| {
        let (a, b) = (q[i], q[(i + 1) % 4]);
        let mid = [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0];
        [a, mid].iter().any(|h| (h[0] - p[0]).hypot(h[1] - p[1]) <= tol)
    })
}

/// Photoshop: holding Ctrl (Windows) / Control (macOS) *during* a drag temporarily turns snapping
/// off. Only the held state while dragging counts: Ctrl at the press belongs to the tool (Ctrl-click
/// auto-selects with the Move tool), and ⌘ on the Mac never overrides snapping.
fn override_held(mods: egui::Modifiers) -> bool {
    mods.ctrl && !mods.mac_cmd
}

fn corner_distorts(app: &PhotocraftApp, gesture: &Gesture, mods: egui::Modifiers) -> bool {
    matches!(gesture, Gesture::TransformCorner { index, .. } if crate::transform_tool::corner_distorts(app, *index, mods))
}

fn override_for_gesture(app: &PhotocraftApp, mods: egui::Modifiers) -> bool {
    override_held(mods) && !(mods.command && app.prefs_rt.snap.as_ref().is_some_and(|s| corner_distorts(app, &s.gesture, mods)))
}

/// The Move tool's press at `p` picked other layers (Auto-Select): the drag moves those, so its
/// box, what it snaps and what it leaves out of the targets are theirs, not the ones selected
/// before (a group or an empty layer selected before had no box at all).
pub(crate) fn retarget_move(app: &mut PhotocraftApp, p: [f64; 2], mods: egui::Modifiers) {
    begin(app, p, mods);
}

/// Start snapping for a drag beginning at `p` with `mods` held (called on pointer down).
fn begin(app: &mut PhotocraftApp, p: [f64; 2], mods: egui::Modifiers) {
    app.prefs_rt.snap = None;
    app.prefs_rt.snap_lines.clear();
    if app.session.active().is_none() {
        return;
    }
    // The tool the press acts as: ⌘ makes a painting tool the Move tool, and a selection tool
    // outside the selection moves the layer as the Move tool does (#2681).
    let tool = crate::canvas::event_tool(app, mods);
    let moves_layer = crate::canvas::command_moves_layer(app, tool, p, mods);
    let crop_tol = crate::crop_ui::tolerance(app);
    let gesture = if let Some(t) = &app.ui.transform {
        let corner = match crate::transform_tool::hit(t, p, crate::transform_tool::handle_tolerance(app)) {
            crate::transform_tool::Hit::Corner(index) => t.quad.get(index).map(|&point| (Gesture::TransformCorner { index, point }, vec![LayerId(t.layer)])),
            _ => None,
        };
        if t.mode == crate::state::TransformMode::Distort {
            corner
        } else if near_handle(&t.quad, p, crate::transform_tool::handle_tolerance(app)) {
            corner.or_else(|| Some((Gesture::Point, vec![LayerId(t.layer)])))
        } else if in_quad(&t.quad, p) {
            Some((Gesture::TransformMove { rect: quad_rect(&t.quad) }, vec![LayerId(t.layer)]))
        } else {
            None
        }
    } else if tool == Tool::Move && crate::rulers::guide_at(app, p[0], p[1]).is_some() {
        Some((Gesture::Guide, Vec::new()))
    } else if tool == Tool::Move || moves_layer {
        let exclude = app.session.active().map(|s| s.selected_layers()).unwrap_or_default();
        moving_rect(app, tool).map(|rect| (Gesture::Move { rect }, exclude))
    } else if tool == Tool::Crop
        && (crate::crop_mode::classic(app) || app.crop.default_frame)
        && let Some(rect) =
            app.ui.crop_rect.filter(|r| crate::crop_ui::angle(app) == 0.0 && crate::crop_ui::hit(*r, p, crop_tol) == crate::crop_ui::Hit::Inside)
    {
        // Inside the untouched default frame draws a new crop (in both modes): snap its corner, not
        // the old canvas-sized frame's bounds. An edited Classic Mode frame still moves and snaps
        // its edges.
        let gesture = if app.crop.default_frame { Gesture::Point } else { Gesture::Move { rect } };
        Some((gesture, Vec::new()))
    } else if tool == Tool::Crop
        && (crate::crop_ui::turns_at(app, p)
            || app.ui.crop_rect.is_some_and(|r| crate::crop_ui::hit_turned(r, crate::crop_ui::angle(app), p, crop_tol) == crate::crop_ui::Hit::Inside))
    {
        // Turning the frame, moving a turned one (its edges don't line up with anything), or in
        // the default mode moving or turning the image under the box: no snapping.
        None
    } else if let Some(cut) = crate::canvas::selection_drag_kind(app, tool, p, mods) {
        // A marquee or lasso inside the selection moves the outline (⌘: cuts its pixels into a
        // floating piece): its bounds snap, as the Move tool's do, not the pointer. A cut piece
        // doesn't snap to its own layer.
        let exclude = if cut { app.session.active().map(|s| s.selected_layers()).unwrap_or_default() } else { Vec::new() };
        selection_rect(app).map(|rect| (Gesture::SelectionMove { rect }, exclude))
    } else if tool.is_type()
        && let Some(layer) = app.ui.text_edit.as_ref().map(|e| LayerId(e.layer))
    {
        // Editing type: a paragraph-box handle drag snaps to guides and other layers, not to the
        // text being resized; a press in the text places the caret where it is (#6819).
        crate::type_tool::box_handle_at(app, layer, p[0], p[1]).map(|_| (Gesture::Point, vec![layer]))
    } else if is_point_tool(tool) {
        Some((Gesture::Point, Vec::new()))
    } else {
        None
    };
    let Some((gesture, exclude)) = gesture else { return };
    let mut targets = if snap_on(app) { build(app, &exclude, false) } else { SnapTargets::default() };
    if app.ui.transform.as_ref().is_some_and(|t| t.mode == crate::state::TransformMode::Distort) {
        // Explicit Distort fits corners to guides; preserve free placement near layer edges.
        targets = targets.filtered(|k| k == SnapKind::Guide);
    }
    if gesture == Gesture::Guide {
        // A guide never snaps to itself (or other guides).
        targets = targets.filtered(|k| k != SnapKind::Guide);
    }
    if matches!(gesture, Gesture::SelectionMove { .. }) || (matches!(gesture, Gesture::Move { .. }) && crate::move_ui::moves_selected_pixels_with(app, tool)) {
        // A moving selection never snaps back to where it started.
        targets = targets.filtered(|k| k != SnapKind::Selection);
    }
    let smart = if snap_on(app) && matches!(gesture, Gesture::Move { .. }) && smart_on(app) { build(app, &exclude, true) } else { SnapTargets::default() };
    app.prefs_rt.snap = Some(ActiveSnap { gesture, start: p, targets, smart, disabled: false });
}

/// Snap a guide drag that ⌘ with a selection tool starts at `p` (`canvas::command_guide_at`,
/// #2690) as the Move tool's guide drag: never to guides, so not back onto itself.
pub fn begin_guide(app: &mut PhotocraftApp, p: [f64; 2]) {
    app.prefs_rt.snap_lines.clear();
    let targets = if snap_on(app) { build(app, &[], false).filtered(|k| k != SnapKind::Guide) } else { SnapTargets::default() };
    app.prefs_rt.snap = Some(ActiveSnap { gesture: Gesture::Guide, start: p, targets, smart: SnapTargets::default(), disabled: false });
}

/// Round to whole pixels when Preferences › Tools asks vector tools and transforms to snap to
/// the pixel grid.
fn pixel_round(app: &PhotocraftApp, p: [f64; 2]) -> [f64; 2] {
    if app.session.prefs().tools.snap_vector_tools_and_transforms_to_pixel_grid
        && matches!(app.ui.tool, Tool::Pen | Tool::CurvaturePen | Tool::Rectangle | Tool::EllipseShape | Tool::Triangle | Tool::Polygon | Tool::Line)
    {
        [p[0].round(), p[1].round()]
    } else {
        p
    }
}

/// Snap one pointer position for the active gesture; records the alignment lines.
fn apply(app: &mut PhotocraftApp, p: [f64; 2], mods: egui::Modifiers) -> [f64; 2] {
    let tol = tolerance(app);
    let Some(snap) = app.prefs_rt.snap.as_ref() else { return p };
    if !snap_on(app) || snap.disabled {
        app.prefs_rt.snap_lines.clear();
        return p;
    }
    let (out, lines) = match &snap.gesture {
        Gesture::Point => crop_handle(app, p, mods, &snap.targets, tol).unwrap_or_else(|| snap.targets.snap_point(p, tol)),
        Gesture::TransformCorner { point, .. } if corner_distorts(app, &snap.gesture, mods) => {
            let proposed = [point[0] + p[0] - snap.start[0], point[1] + p[1] - snap.start[1]];
            let (corner, lines) = snap.targets.snap_point(proposed, tol);
            ([p[0] + corner[0] - proposed[0], p[1] + corner[1] - proposed[1]], lines)
        }
        Gesture::TransformCorner { .. } => snap.targets.snap_point(p, tol),
        Gesture::Guide => snap.targets.snap_point(p, tol),
        Gesture::Move { rect } | Gesture::TransformMove { rect } | Gesture::SelectionMove { rect } | Gesture::DrawMove { rect } => {
            let d = [p[0] - snap.start[0], p[1] - snap.start[1]];
            let moved = [rect[0] + d[0], rect[1] + d[1], rect[2] + d[0], rect[3] + d[1]];
            let set = if snap.targets.is_empty() { &snap.smart } else { &snap.targets };
            let (adj, lines) = set.snap_rect(moved, tol);
            ([p[0] + adj[0], p[1] + adj[1]], lines)
        }
    };
    app.prefs_rt.snap_lines = lines;
    out
}

/// A corner stopped by the transform's convexity constraint is not aligned to the proposed
/// target. Keep only the lines the final corner actually reaches, after the transform applies.
pub(crate) fn retain_transform_lines(app: &mut PhotocraftApp, mods: egui::Modifiers) {
    let Some(snap) = app.prefs_rt.snap.as_ref() else { return };
    let Gesture::TransformCorner { index, .. } = snap.gesture else { return };
    if !corner_distorts(app, &snap.gesture, mods) {
        return;
    }
    let Some(point) = app.ui.transform.as_ref().and_then(|t| t.quad.get(index)).copied() else { return };
    app.prefs_rt.snap_lines.retain(|line| (if line.vertical { point[0] } else { point[1] } - line.pos).abs() < 1e-6);
}

/// Snap a crop handle being dragged: the handle itself (the frame's edge or corner), not the
/// pointer, which grabbed it up to the grab radius away (#1919: snapping the pointer left a strip
/// as wide as that offset between the frame and the edge it showed as snapped to). With Auto
/// Center Preview the handle is found on the image, calculated from the original camera: snapping
/// the pointer in the moving camera feeds the previous frame's pan back into the resize and makes
/// a stationary pointer oscillate near the canvas edges.
fn crop_handle(app: &PhotocraftApp, p: [f64; 2], mods: egui::Modifiers, targets: &SnapTargets, tol: f64) -> Option<([f64; 2], Vec<SnapLine>)> {
    if app.ui.tool != Tool::Crop {
        return None;
    }
    let crate::crop_ui::CropDrag::Resize { hx, hy, start, rect, view } = app.crop.drag? else { return None };
    let deg = crate::crop_ui::angle(app);
    let ratio = crate::crop_ui::resize_ratio(app, rect, mods);
    let q = view.map_or(p, |view| crate::crop_mode::to_press(app, Some(view), p));
    // The frame for the pointer at `q`, as `crop_ui` resizes it.
    let frame_at = |q: [f64; 2]| match view {
        Some(_) => crate::crop_mode::resized_frame(crate::crop_mode::LiveResize { rect, deg, hx, hy, start, q, ratio, alt: mods.alt }).map(|(f, _)| f),
        None => Some(crate::crop_ui::resized_turned(rect, deg, hx, hy, [q[0] - start[0], q[1] - start[1]], ratio, mods.alt)),
    };
    let frame = frame_at(q)?;
    let at = |r: [f64; 4]| {
        let c = crate::crop_ui::center(r);
        crate::crop_ui::turn([c[0] + f64::from(hx) * (r[2] - r[0]) / 2.0, c[1] + f64::from(hy) * (r[3] - r[1]) / 2.0], c, deg)
    };
    let handle = at(frame);
    let (snapped, mut lines) = targets.snap_point(handle, tol);
    let mut d = crate::crop_ui::turn([snapped[0] - handle[0], snapped[1] - handle[1]], [0.0, 0.0], -deg);
    // An edge handle controls only its own axis, not the midpoint along the other one.
    if hx == 0 {
        d[0] = 0.0;
    }
    if hy == 0 {
        d[1] = 0.0;
    }
    if deg == 0.0 {
        lines.retain(|line| if line.vertical { hx != 0 } else { hy != 0 });
    }
    if lines.is_empty() {
        return Some((p, lines));
    }
    // Auto Center Preview moves both screen edges. On the image the opposite edge stays put,
    // so the grabbed edge travels twice as far as the pointer (once with Alt). Without it the
    // handle follows the pointer.
    let scale = if view.is_some() && !mods.alt { 2.0 } else { 1.0 };
    // How much the frame's extent grows per unit of pointer travel: both sides with Auto Center
    // Preview or Alt, else the grabbed one.
    let grow = if view.is_some() || mods.alt { 2.0 } else { 1.0 };
    let original = at(rect);
    let travel = crate::crop_ui::turn([handle[0] - original[0], handle[1] - original[1]], [0.0, 0.0], -deg);
    let mut local = [(travel[0] + d[0]) / scale, (travel[1] + d[1]) / scale];
    if deg == 0.0
        && hx != 0
        && hy != 0
        && let Some(k) = ratio
    {
        // A corner with a ratio has one degree of freedom. Put the other axis on the same
        // ratio, rather than letting the raw pointer's unused axis override the snapped edge.
        if lines.iter().any(|line| line.vertical) {
            local[1] = f64::from(hy) * ((rect[2] - rect[0] + grow * f64::from(hx) * local[0]) / k - (rect[3] - rect[1])) / grow;
        } else {
            local[0] = f64::from(hx) * ((rect[3] - rect[1] + grow * f64::from(hy) * local[1]) * k - (rect[2] - rect[0])) / grow;
        }
    }
    let d = crate::crop_ui::turn(local, [0.0, 0.0], deg);
    let q = [start[0] + d[0], start[1] + d[1]];
    if let Some(snapped_frame) = frame_at(q) {
        let h = at(snapped_frame);
        lines.retain(|line| (if line.vertical { h[0] } else { h[1] } - line.pos).abs() < 1e-3);
    }
    Some((view.map_or(q, |view| crate::crop_mode::from_press(app, view, q)), lines))
}

/// The bounds `[x0, y0, x1, y1]` of what drag `d` draws so far, for the tools whose drag Space
/// repositions and that snap (`None` for the rest, or a shape still too small to draw).
fn drawn_rect(app: &PhotocraftApp, d: &crate::canvas::Drag) -> Option<[f64; 4]> {
    let last = d.points.last().map_or(d.start, |p| [p[0], p[1]]);
    let (a, b) = match d.tool {
        Tool::RectMarquee | Tool::EllipseMarquee => crate::canvas::marquee_corners(&app.ui.tool_options, d, last),
        Tool::ObjectSelection => (d.start, last),
        t if crate::vector_ui::is_shape_tool(t) => return crate::vector_ui::shape_bounds(&app.ui.tool_options, t, d.start, last, d.live),
        _ => return None,
    };
    Some([a[0].min(b[0]), a[1].min(b[1]), a[0].max(b[0]), a[1].max(b[1])])
}

/// Space held while a marquee, Object Selection box or shape is drawn moves the whole of it
/// (`hold_keys`): its edges and centre snap, as a moved selection's do, not just the corner under
/// the pointer. Releasing Space goes back to snapping that corner.
fn follow_reposition(app: &mut PhotocraftApp) {
    let Some(d) = app.drag.as_ref() else { return };
    let Some(snap) = app.prefs_rt.snap.as_ref() else { return };
    let gesture = match (&snap.gesture, d.reposition) {
        (Gesture::Point, true) => match drawn_rect(app, d) {
            Some(rect) => Gesture::DrawMove { rect },
            None => return,
        },
        (Gesture::DrawMove { .. }, false) => Gesture::Point,
        _ => return,
    };
    let last = d.points.last().map_or(d.start, |p| [p[0], p[1]]);
    if let Some(snap) = app.prefs_rt.snap.as_mut() {
        snap.gesture = gesture;
        snap.start = last;
    }
}

/// Snap a tool event's coordinates (called by `canvas::tool_event` before the tools see it).
pub fn filter_event(app: &mut PhotocraftApp, ev: ToolEvent, mods: egui::Modifiers) -> ToolEvent {
    match ev {
        ToolEvent::Down { x, y, pressure } => {
            begin(app, [x, y], mods);
            // Moves snap their delta, which is zero at the start: keep the press point.
            let p = match app.prefs_rt.snap.as_ref().map(|s| &s.gesture) {
                Some(Gesture::Point)
                    if !(app.ui.tool == Tool::Crop
                        && app.ui.crop_rect.is_some_and(|r| {
                            matches!(
                                crate::crop_ui::hit_turned(r, crate::crop_ui::angle(app), [x, y], crate::crop_ui::tolerance(app)),
                                crate::crop_ui::Hit::Handle(..)
                            )
                        })) =>
                {
                    let q = apply(app, [x, y], mods);
                    pixel_round(app, q)
                }
                _ => [x, y],
            };
            ToolEvent::Down { x: p[0], y: p[1], pressure }
        }
        ToolEvent::Move { x, y, pressure } => {
            if app.prefs_rt.snap.is_none() {
                return ev;
            }
            follow_reposition(app);
            let disabled = override_for_gesture(app, mods);
            if let Some(s) = app.prefs_rt.snap.as_mut() {
                s.disabled = disabled;
            }
            let q = apply(app, [x, y], mods);
            let p = pixel_round(app, q);
            ToolEvent::Move { x: p[0], y: p[1], pressure }
        }
        ToolEvent::Up { x, y } => {
            if app.prefs_rt.snap.is_none() {
                return ev;
            }
            follow_reposition(app);
            let disabled = override_for_gesture(app, mods);
            if let Some(s) = app.prefs_rt.snap.as_mut() {
                s.disabled = disabled;
            }
            let q = apply(app, [x, y], mods);
            let p = pixel_round(app, q);
            app.prefs_rt.snap = None;
            app.prefs_rt.snap_lines.clear();
            ToolEvent::Up { x: p[0], y: p[1] }
        }
    }
}

/// Snap a guide being dragged out of a ruler (`vertical` guides snap their x).
pub fn snap_guide(app: &mut PhotocraftApp, vertical: bool, pos: f64) -> f64 {
    if !snap_on(app) {
        return pos;
    }
    let Some(st) = app.session.active() else { return pos };
    // The Snap To options (grid, layers, document bounds, grid step) change the targets too.
    let key = (st.doc.id, st.revision, options(app));
    if app.prefs_rt.guide_targets.as_ref().is_none_or(|(k, _)| *k != key) {
        let t = build(app, &[], false).filtered(|k| k != SnapKind::Guide);
        app.prefs_rt.guide_targets = Some((key, t));
    }
    let tol = tolerance(app);
    let t = app.prefs_rt.guide_targets.as_ref().map(|(_, t)| t);
    t.and_then(|t| t.nearest(vertical, pos, tol)).map_or(pos, |t| t.pos)
}

fn hex_color(s: &str, fallback: Color32) -> Color32 {
    photocraft_engine::prefs::parse_hex(s).map_or(fallback, |c| Color32::from_rgb(c[0], c[1], c[2]))
}

/// The Smart Guides colour from Preferences › Guides, Grid & Slices (magenta by default).
pub(crate) fn smart_color(app: &PhotocraftApp) -> Color32 {
    hex_color(&app.session.prefs().guides_grid_and_slices.smart_guide_color, Color32::from_rgb(255, 0, 255))
}

/// Draw smart guides (and the moving bounds) for the drag in progress.
pub fn draw(app: &PhotocraftApp, painter: &egui::Painter, xf: &ViewXform) {
    let Some(snap) = &app.prefs_rt.snap else { return };
    let smart_color = smart_color(app);
    // The moved layers' bounds follow the Move tool (Free Transform draws its own box).
    if let (Gesture::Move { rect }, Some(d)) =
        (&snap.gesture, app.drag.as_ref().and_then(|d| d.points.last()).map(|p| [p[0] - snap.start[0], p[1] - snap.start[1]]))
    {
        let r = egui::Rect::from_two_pos(
            xf.to_screen((rect[0] + d[0]) as f32, (rect[1] + d[1]) as f32),
            xf.to_screen((rect[2] + d[0]) as f32, (rect[3] + d[1]) as f32),
        );
        painter.rect_stroke(r, 0.0, Stroke::new(1.0, crate::theme::Tokens::get(painter.ctx()).accent), egui::StrokeKind::Middle);
    }
    if !smart_on(app) {
        return;
    }
    for l in &app.prefs_rt.snap_lines {
        if !l.kind.is_smart() {
            continue;
        }
        draw_line(painter, xf, l, smart_color);
    }
}

fn draw_line(painter: &egui::Painter, xf: &ViewXform, l: &SnapLine, color: Color32) {
    let (a, b) = if l.vertical {
        (xf.to_screen(l.pos as f32, l.from as f32), xf.to_screen(l.pos as f32, l.to as f32))
    } else {
        (xf.to_screen(l.from as f32, l.pos as f32), xf.to_screen(l.to as f32, l.pos as f32))
    };
    painter.line_segment([pos2(a.x.round() + 0.5, a.y.round() + 0.5), pos2(b.x.round() + 0.5, b.y.round() + 0.5)], Stroke::new(1.0, color));
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn app_with_box() -> PhotocraftApp {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
        app.sync_views();
        app.ui.views[0].zoom = 1.0;
        app.ui.views[0].fit_pending = false;
        // Two boxes: a target at (300..350, 200..260) and the moving one at (40..90, 40..80).
        app.run("layer.new.layer", json!({"name": "target"})).unwrap();
        app.run("select.rect", json!({"x": 300, "y": 200, "width": 50, "height": 60})).unwrap();
        app.run("edit.fill", json!({"color": "#ff0000"})).unwrap();
        app.run("layer.new.layer", json!({"name": "mover"})).unwrap();
        app.run("select.rect", json!({"x": 40, "y": 40, "width": 50, "height": 40})).unwrap();
        app.run("edit.fill", json!({"color": "#0000ff"})).unwrap();
        app.run("select.deselect", json!({})).unwrap();
        app
    }

    /// #7538: moving selected pixels snaps (and frames) the pixels, not a loose marquee round them.
    /// Snapping starts before the press cuts them, so the cut retargets it to their outline.
    #[test]
    fn moving_selected_pixels_snaps_their_bounds_not_the_marquee() {
        for alt in [false, true] {
            let mut app = app_with_box();
            app.ui.tool = Tool::Move;
            app.run("select.rect", json!({"x": 20, "y": 20, "width": 100, "height": 90})).unwrap();
            let m = egui::Modifiers { alt, ..Default::default() };
            crate::canvas::tool_event(&mut app, ToolEvent::Down { x: 60.0, y: 60.0, pressure: 1.0 }, m);
            let gesture = app.prefs_rt.snap.as_ref().map(|s| s.gesture.clone());
            assert_eq!(gesture, Some(Gesture::Move { rect: [40.0, 40.0, 90.0, 80.0] }), "alt {alt}: the blue box, not the marquee");
            // 263 px right puts the box's left edge 3 px from the target's (the marquee's 20 px off).
            crate::canvas::tool_event(&mut app, ToolEvent::Move { x: 323.0, y: 100.0, pressure: 1.0 }, m);
            crate::canvas::tool_event(&mut app, ToolEvent::Up { x: 323.0, y: 100.0 }, m);
            app.run("select.drop", json!({})).unwrap();
            let sel = app.session.active().unwrap().doc.selection.as_ref().map(|s| s.content_bounds().x0);
            assert_eq!(sel, Some(300), "alt {alt}: snapped to the target's edge");
        }
        // A marquee's ⌘-drag cuts them the same way.
        let mut app = app_with_box();
        app.ui.tool = Tool::RectMarquee;
        app.run("select.rect", json!({"x": 20, "y": 20, "width": 100, "height": 90})).unwrap();
        let cmd = egui::Modifiers { mac_cmd: true, command: true, ..Default::default() };
        crate::canvas::tool_event(&mut app, ToolEvent::Down { x: 60.0, y: 60.0, pressure: 1.0 }, cmd);
        let gesture = app.prefs_rt.snap.as_ref().map(|s| s.gesture.clone());
        assert_eq!(gesture, Some(Gesture::SelectionMove { rect: [40.0, 40.0, 90.0, 80.0] }), "⌘-drag with the marquee");
        // The Background has no transparency to shrink to.
        let mut app = app_with_box();
        app.ui.tool = Tool::Move;
        app.run("layer.select", json!({"name": "Background"})).unwrap();
        app.run("select.rect", json!({"x": 20, "y": 20, "width": 100, "height": 90})).unwrap();
        crate::canvas::tool_event(&mut app, ToolEvent::Down { x: 60.0, y: 60.0, pressure: 1.0 }, egui::Modifiers::NONE);
        let gesture = app.prefs_rt.snap.as_ref().map(|s| s.gesture.clone());
        assert_eq!(gesture, Some(Gesture::Move { rect: [20.0, 20.0, 120.0, 110.0] }), "the Background");
    }

    fn mover_bounds(app: &PhotocraftApp) -> photocraft_geom::Rect {
        let st = app.session.active().unwrap();
        st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap().content_bounds()
    }

    #[test]
    fn drawing_a_new_crop_snaps_its_corner_not_the_default_frame() {
        // In both crop modes: the default mode draws a new box inside the untouched frame too.
        for (classic, snap, end) in [(true, false, [300.0, 220.0]), (true, true, [294.0, 217.0]), (false, false, [300.0, 220.0]), (false, true, [294.0, 217.0])]
        {
            let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
            app.run("file.new", json!({"width": 640, "height": 480, "background": "white"})).unwrap();
            app.run("shape.create", json!({"kind": "rect", "rect": [100, 80, 200, 140], "fill": "#d59b40"})).unwrap();
            app.sync_views();
            app.ppp = 2.0;
            app.ui.views[0].zoom = 1.0;
            app.ui.views[0].fit_pending = false;
            app.ui.tool = Tool::Crop;
            app.ui.tool_options.crop_shield.classic_mode = classic;
            app.ui.extras.snap = snap;
            app.ui.view.show.smart_guides = true;
            crate::crop_ui::ensure_frame(&mut app);
            let view = app.ui.views[0].clone();
            let history = app.session.active().unwrap().history.past_len();
            let m = egui::Modifiers::NONE;
            crate::canvas::tool_event(&mut app, ToolEvent::Down { x: 100.0, y: 80.0, pressure: 1.0 }, m);
            crate::canvas::tool_event(&mut app, ToolEvent::Move { x: end[0], y: end[1], pressure: 1.0 }, m);
            crate::canvas::tool_event(&mut app, ToolEvent::Up { x: end[0], y: end[1] }, m);
            // Smart Guides must not snap the old canvas-sized frame as if it were moving.
            // With View Snap on, the new corner still snaps to the nearby shape's edges.
            assert_eq!(app.ui.crop_rect, Some([100.0, 80.0, 300.0, 220.0]), "Classic Mode: {classic}, View Snap: {snap}");
            // The default mode's Auto Center Preview re-centres the drawn box; Classic Mode keeps the view.
            if classic {
                assert_eq!(app.ui.views[0], view);
            }
            assert_eq!(app.session.active().unwrap().history.past_len(), history);
        }
    }

    #[test]
    fn move_tool_snaps_to_other_layer_edges_with_smart_guides() {
        let mut app = app_with_box();
        app.ui.tool = Tool::Move;
        let m = egui::Modifiers::NONE;
        crate::canvas::tool_event(&mut app, ToolEvent::Down { x: 60.0, y: 60.0, pressure: 1.0 }, m);
        // Drag so the mover's left edge lands 3 px right of the target's (x = 303).
        crate::canvas::tool_event(&mut app, ToolEvent::Move { x: 323.0, y: 100.0, pressure: 1.0 }, m);
        assert!(app.prefs_rt.snap_lines.iter().any(|l| l.vertical && l.pos == 300.0 && l.kind.is_smart()), "{:?}", app.prefs_rt.snap_lines);
        crate::canvas::tool_event(&mut app, ToolEvent::Up { x: 323.0, y: 100.0 }, m);
        assert_eq!(mover_bounds(&app).x0, 300);
        assert!(app.prefs_rt.snap.is_none());
    }

    /// ⌘ turns a painting tool, or a marquee outside the selection, into the Move tool for the
    /// drag: it gets the Move tool's box and smart-guide snapping, not the selected tool's.
    #[test]
    fn command_drag_temporary_move_snaps_like_the_move_tool() {
        for tool in [Tool::Brush, Tool::RectMarquee] {
            let mut app = app_with_box();
            app.ui.tool = tool;
            let cmd = egui::Modifiers { mac_cmd: true, command: true, ..Default::default() };
            crate::canvas::tool_event(&mut app, ToolEvent::Down { x: 60.0, y: 60.0, pressure: 1.0 }, cmd);
            let gesture = app.prefs_rt.snap.as_ref().map(|s| s.gesture.clone());
            assert_eq!(gesture, Some(Gesture::Move { rect: [40.0, 40.0, 90.0, 80.0] }), "{tool:?}: the mover's box");
            crate::canvas::tool_event(&mut app, ToolEvent::Up { x: 323.0, y: 100.0 }, cmd);
            assert_eq!(mover_bounds(&app).x0, 300, "{tool:?}: snapped to the target's edge");
        }
    }

    /// The drag's box and snapping belong to the layer Auto-Select picks on the press, not to the
    /// one selected before: another layer (its box would move instead), or an empty layer (no box,
    /// no snapping at all).
    #[test]
    fn move_tool_box_and_snapping_follow_the_auto_selected_layer() {
        for before in ["target", "empty"] {
            let mut app = app_with_box();
            let mover = app.session.active().unwrap().active_layer.unwrap();
            if before == "empty" {
                app.run("layer.new.layer", json!({"name": "empty"})).unwrap();
            } else {
                let st = app.session.active().unwrap();
                let target = st.doc.walk().into_iter().find(|(_, _, l)| l.name == "target").unwrap().2.id;
                app.run("layer.select", json!({"layer": target.0})).unwrap();
            }
            app.ui.tool = Tool::Move;
            app.ui.tool_options.move_auto_select = true;
            let m = egui::Modifiers::NONE;
            crate::canvas::tool_event(&mut app, ToolEvent::Down { x: 60.0, y: 60.0, pressure: 1.0 }, m);
            assert_eq!(app.session.active().unwrap().active_layer, Some(mover), "{before}: Auto-Select picked the mover");
            let gesture = app.prefs_rt.snap.as_ref().map(|s| s.gesture.clone());
            assert_eq!(gesture, Some(Gesture::Move { rect: [40.0, 40.0, 90.0, 80.0] }), "{before}: the box is the mover's");
            // The mover's left edge lands 3 px right of the target's and snaps to it.
            crate::canvas::tool_event(&mut app, ToolEvent::Move { x: 323.0, y: 100.0, pressure: 1.0 }, m);
            crate::canvas::tool_event(&mut app, ToolEvent::Up { x: 323.0, y: 100.0 }, m);
            assert_eq!(mover_bounds(&app).x0, 300, "{before}: snapped to the target's edge");
        }
    }

    #[test]
    fn view_snap_off_blocks_smart_guide_snapping() {
        let mut app = app_with_box();
        app.ui.tool = Tool::Move;
        app.ui.view.show.smart_guides = true;
        app.ui.extras.snap = false;
        app.ui.tool_options.move_auto_select = false;
        let m = egui::Modifiers::NONE;
        crate::canvas::tool_event(&mut app, ToolEvent::Down { x: 60.0, y: 60.0, pressure: 1.0 }, m);
        crate::canvas::tool_event(&mut app, ToolEvent::Up { x: 323.0, y: 100.0 }, m);
        assert_eq!(mover_bounds(&app).x0, 303, "View › Snap off must not snap via smart guides");
    }

    #[test]
    fn ctrl_or_snap_off_disables_snapping() {
        let mut app = app_with_box();
        app.ui.tool = Tool::Move;
        app.ui.view.show.smart_guides = false;
        app.ui.extras.snap = false;
        // The presses land beside the moved layer: keep moving it (no Auto-Select pick).
        app.ui.tool_options.move_auto_select = false;
        let m = egui::Modifiers::NONE;
        crate::canvas::tool_event(&mut app, ToolEvent::Down { x: 60.0, y: 60.0, pressure: 1.0 }, m);
        crate::canvas::tool_event(&mut app, ToolEvent::Up { x: 323.0, y: 100.0 }, m);
        assert_eq!(mover_bounds(&app).x0, 303);
        app.ui.extras.snap = true;
        let ctrl = egui::Modifiers { ctrl: true, ..Default::default() };
        crate::canvas::tool_event(&mut app, ToolEvent::Down { x: 60.0, y: 60.0, pressure: 1.0 }, m);
        crate::canvas::tool_event(&mut app, ToolEvent::Up { x: 62.0, y: 60.0 }, ctrl);
        assert_eq!(mover_bounds(&app).x0, 305, "no snap to the target's right edge at 350? (moved freely)");
        // Ctrl only at the press (Ctrl-click auto-select) doesn't turn snapping off for the drag.
        crate::canvas::tool_event(&mut app, ToolEvent::Down { x: 60.0, y: 60.0, pressure: 1.0 }, ctrl);
        crate::canvas::tool_event(&mut app, ToolEvent::Move { x: 61.0, y: 60.0, pressure: 1.0 }, m);
        assert!(!app.prefs_rt.snap.as_ref().is_some_and(|s| s.disabled));
        crate::canvas::tool_event(&mut app, ToolEvent::Up { x: 61.0, y: 60.0 }, m);
        // ⌘ on the Mac is not the override.
        let cmd = egui::Modifiers { mac_cmd: true, command: true, ..Default::default() };
        assert!(!override_held(cmd) && override_held(ctrl));
    }

    #[test]
    fn snap_off_disables_all_snapping_even_with_smart_guides() {
        let mut app = app_with_box();
        app.ui.tool = Tool::Move;
        assert!(app.ui.view.show.smart_guides);
        app.ui.extras.snap = false;
        app.ui.tool_options.move_auto_select = false;
        let m = egui::Modifiers::NONE;
        crate::canvas::tool_event(&mut app, ToolEvent::Down { x: 60.0, y: 60.0, pressure: 1.0 }, m);
        crate::canvas::tool_event(&mut app, ToolEvent::Up { x: 323.0, y: 100.0 }, m);
        assert_eq!(mover_bounds(&app).x0, 303);
        assert!(app.prefs_rt.snap_lines.is_empty());
    }

    #[test]
    fn marquee_corner_snaps_to_guides_and_document_edges() {
        let mut app = app_with_box();
        app.run("view.newGuide", json!({"orientation": "vertical", "position": 120})).unwrap();
        app.ui.tool = Tool::RectMarquee;
        let m = egui::Modifiers::NONE;
        crate::canvas::tool_event(&mut app, ToolEvent::Down { x: 3.0, y: 2.0, pressure: 1.0 }, m);
        crate::canvas::tool_event(&mut app, ToolEvent::Up { x: 116.0, y: 50.0 }, m);
        let sel = app.session.active().unwrap().doc.selection.as_ref().unwrap().content_bounds();
        assert_eq!((sel.x0, sel.y0, sel.x1), (0, 0, 120));
        // Zoomed in to 800%, 4 px is beyond the 1 px threshold: no snap.
        app.ui.views[0].zoom = 8.0;
        // (Deselect first: a drag starting inside the selection would move it.)
        app.run("select.deselect", json!({})).unwrap();
        crate::canvas::tool_event(&mut app, ToolEvent::Down { x: 10.0, y: 10.0, pressure: 1.0 }, m);
        crate::canvas::tool_event(&mut app, ToolEvent::Up { x: 116.0, y: 50.0 }, m);
        let sel = app.session.active().unwrap().doc.selection.as_ref().unwrap().content_bounds();
        assert_eq!(sel.x1, 116);
    }

    /// Space while drawing a marquee, Object Selection box or shape moves it: its edges snap, not
    /// only the corner under the pointer; with Space released the corner snaps again.
    #[test]
    fn drawing_repositioned_with_space_snaps_its_edges() {
        for tool in [Tool::RectMarquee, Tool::EllipseMarquee, Tool::ObjectSelection, Tool::Rectangle, Tool::Line] {
            let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
            app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
            app.sync_views();
            app.ui.views[0].zoom = 1.0;
            app.ui.views[0].fit_pending = false;
            app.run("view.newGuide", json!({"orientation": "vertical", "position": 220})).unwrap();
            app.ui.tool = tool;
            let m = egui::Modifiers::NONE;
            crate::canvas::tool_event(&mut app, ToolEvent::Down { x: 100.0, y: 100.0, pressure: 1.0 }, m);
            crate::canvas::tool_event(&mut app, ToolEvent::Move { x: 150.0, y: 140.0, pressure: 1.0 }, m);
            // Space down: the 50 × 40 box moves so its left edge lands 4 px short of the guide;
            // the pointer corner (x = 266) is near nothing.
            app.drag.as_mut().unwrap().reposition = true;
            crate::canvas::tool_event(&mut app, ToolEvent::Move { x: 266.0, y: 140.0, pressure: 1.0 }, m);
            assert_eq!(app.drag.as_ref().unwrap().start, [220.0, 100.0], "{tool:?}");
            if tool != Tool::RectMarquee {
                continue;
            }
            // Space up: sizing resumes and the corner snaps to the document's right edge.
            app.drag.as_mut().unwrap().reposition = false;
            crate::canvas::tool_event(&mut app, ToolEvent::Up { x: 397.0, y: 140.0 }, m);
            let sel = app.session.active().unwrap().doc.selection.as_ref().unwrap().content_bounds();
            assert_eq!((sel.x0, sel.y0, sel.x1, sel.y1), (220, 100, 400, 140));
        }
    }

    /// Dragging inside a selection with a marquee or lasso moves the outline (⌘ cuts the pixels
    /// into a floating piece): its bounds snap, not the pointer.
    #[test]
    fn selection_drag_snaps_the_selection_bounds() {
        for (tool, cut) in [(Tool::RectMarquee, false), (Tool::Lasso, false), (Tool::RectMarquee, true)] {
            let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
            app.run("file.new", json!({"width": 400, "height": 300, "background": "white"})).unwrap();
            app.sync_views();
            app.ui.views[0].zoom = 1.0;
            app.ui.views[0].fit_pending = false;
            app.run("view.newGuide", json!({"orientation": "vertical", "position": 220})).unwrap();
            app.run("select.rect", json!({"x": 100, "y": 100, "width": 50, "height": 40})).unwrap();
            app.ui.tool = tool;
            let m = if cut { egui::Modifiers { mac_cmd: true, command: true, ..Default::default() } } else { egui::Modifiers::NONE };
            // The outline's right edge lands 2 px short of the guide (x = 218); the pointer ends
            // 7 px from the document centre (x = 200), which it must not snap to.
            crate::canvas::tool_event(&mut app, ToolEvent::Down { x: 125.0, y: 117.0, pressure: 1.0 }, m);
            crate::canvas::tool_event(&mut app, ToolEvent::Move { x: 193.0, y: 117.0, pressure: 1.0 }, m);
            crate::canvas::tool_event(&mut app, ToolEvent::Up { x: 193.0, y: 117.0 }, m);
            let st = app.session.active().unwrap();
            let off = photocraft_engine::float_cmds::floating(st).map_or((0, 0), |f| f.offset);
            let sel = st.doc.selection.as_ref().unwrap().content_bounds().translate(off.0, off.1);
            assert_eq!((sel.x0, sel.y0, sel.x1, sel.y1), (170, 100, 220, 140), "{tool:?} cut: {cut}");
        }
    }

    #[test]
    fn distort_corner_snaps_from_positive_grab_offset() {
        distort_corner_snaps_from_grab_offset([4.0, 3.0]);
    }

    #[test]
    fn distort_corner_snaps_from_negative_grab_offset() {
        distort_corner_snaps_from_grab_offset([-4.0, -3.0]);
    }

    fn distort_corner_snaps_from_grab_offset(offset: [f64; 2]) {
        let mut app = app_with_box();
        app.run("view.newGuide", json!({"orientation": "vertical", "position": 110})).unwrap();
        app.run("view.newGuide", json!({"orientation": "horizontal", "position": 30})).unwrap();
        let ctx = egui::Context::default();
        crate::menus::invoke(&mut app, &ctx, "edit.transform.distort", json!({})).unwrap();
        let before = app.ui.transform.as_ref().unwrap().quad;
        let press = [before[1][0] + offset[0], before[1][1] + offset[1]];
        // The proposed corner is (108, 32); the pointer keeps its original grab offset.
        let release = [108.0 + offset[0], 32.0 + offset[1]];
        let m = egui::Modifiers::NONE;
        crate::canvas::tool_event(&mut app, ToolEvent::Down { x: press[0], y: press[1], pressure: 1.0 }, m);
        crate::canvas::tool_event(&mut app, ToolEvent::Move { x: release[0], y: release[1], pressure: 1.0 }, m);
        let moved = app.ui.transform.as_ref().unwrap().quad;
        assert_eq!(moved[1], [110.0, 30.0], "grab offset {offset:?}");
        assert_eq!([moved[0], moved[2], moved[3]], [before[0], before[2], before[3]]);
        assert!(app.prefs_rt.snap_lines.iter().any(|l| l.vertical && l.pos == 110.0 && l.kind == SnapKind::Guide));
        assert!(app.prefs_rt.snap_lines.iter().any(|l| !l.vertical && l.pos == 30.0 && l.kind == SnapKind::Guide));
        crate::canvas::tool_event(&mut app, ToolEvent::Up { x: release[0], y: release[1] }, m);
        assert_eq!(app.ui.transform.as_ref().unwrap().quad, moved);
        assert!(app.prefs_rt.snap.is_none() && app.prefs_rt.snap_lines.is_empty());
    }

    fn app_with_corner_guides(menu: &str) -> PhotocraftApp {
        let mut app = app_with_box();
        app.run("view.newGuide", json!({"orientation": "vertical", "position": 110})).unwrap();
        app.run("view.newGuide", json!({"orientation": "horizontal", "position": 30})).unwrap();
        crate::menus::invoke(&mut app, &egui::Context::default(), menu, json!({})).unwrap();
        app
    }

    fn move_corner(app: &mut PhotocraftApp, index: usize, offset: [f64; 2], end: [f64; 2], down: egui::Modifiers, moving: egui::Modifiers) {
        let corner = app.ui.transform.as_ref().unwrap().quad[index];
        crate::canvas::tool_event(app, ToolEvent::Down { x: corner[0] + offset[0], y: corner[1] + offset[1], pressure: 1.0 }, down);
        crate::canvas::tool_event(app, ToolEvent::Move { x: end[0] + offset[0], y: end[1] + offset[1], pressure: 1.0 }, moving);
    }

    #[test]
    fn distort_corner_respects_guide_axes_tolerance_and_toggles() {
        for (snap, shown, guides, end, expected) in [
            (true, true, true, [108.0, 20.0], [110.0, 20.0]),
            (true, true, true, [100.0, 32.0], [100.0, 30.0]),
            (true, true, true, [100.0, 20.0], [100.0, 20.0]),
            (false, true, true, [108.0, 32.0], [108.0, 32.0]),
            (true, false, true, [108.0, 32.0], [108.0, 32.0]),
            (true, true, false, [108.0, 32.0], [108.0, 32.0]),
        ] {
            let mut app = app_with_corner_guides("edit.transform.distort");
            app.ui.extras.snap = snap;
            app.ui.extras.guides = shown;
            app.ui.view.snap_to.guides = guides;
            move_corner(&mut app, 1, [4.0, 3.0], end, egui::Modifiers::NONE, egui::Modifiers::NONE);
            assert_eq!(app.ui.transform.as_ref().unwrap().quad[1], expected, "Snap: {snap}, shown: {shown}, guides: {guides}, proposed: {end:?}");
            assert!(app.prefs_rt.snap_lines.iter().all(|line| (if line.vertical { expected[0] } else { expected[1] } - line.pos).abs() < 1e-6));
        }
    }

    #[test]
    fn free_transform_command_corner_snaps_with_the_real_grab_offset() {
        let windows = egui::Modifiers { command: true, ctrl: true, ..egui::Modifiers::NONE };
        let mac = egui::Modifiers { command: true, mac_cmd: true, ..egui::Modifiers::NONE };
        for m in [windows, mac] {
            for offset in [[4.0, 3.0], [-4.0, -3.0], [0.0, 0.0]] {
                for down in [m, egui::Modifiers::NONE] {
                    let mut app = app_with_corner_guides("edit.freeTransform");
                    let before = app.ui.transform.as_ref().unwrap().quad;
                    move_corner(&mut app, 1, offset, [108.0, 32.0], down, m);
                    let moved = app.ui.transform.as_ref().unwrap().quad;
                    assert_eq!(moved[1], [110.0, 30.0], "{m:?}, press {down:?}, offset {offset:?}");
                    assert_eq!([moved[0], moved[2], moved[3]], [before[0], before[2], before[3]]);
                    assert_eq!(app.prefs_rt.snap_lines.len(), 2);
                    assert!(!app.prefs_rt.snap.as_ref().unwrap().disabled);
                }
            }
        }
    }

    #[test]
    fn control_alone_still_disables_snapping_in_menu_distort() {
        let mut app = app_with_corner_guides("edit.transform.distort");
        let ctrl = egui::Modifiers { ctrl: true, ..egui::Modifiers::NONE };
        move_corner(&mut app, 1, [4.0, 3.0], [108.0, 32.0], ctrl, ctrl);
        assert_eq!(app.ui.transform.as_ref().unwrap().quad[1], [108.0, 32.0]);
        assert!(app.prefs_rt.snap.as_ref().unwrap().disabled);
        assert!(app.prefs_rt.snap_lines.is_empty());
    }

    #[test]
    fn skew_control_corner_keeps_its_axis_instead_of_snapping_as_distort() {
        let ctrl = egui::Modifiers { command: true, ctrl: true, ..egui::Modifiers::NONE };
        for legacy in [false, true] {
            for (menu, expected, snapping) in [
                ("edit.transform.skew", [[40.0, 40.0], [108.0, 40.0], [90.0, 80.0], [40.0, 80.0]], false),
                ("edit.transform.distort", [[40.0, 40.0], [110.0, 30.0], [90.0, 80.0], [40.0, 80.0]], true),
                ("edit.transform.perspective", [[22.0, 40.0], [108.0, 40.0], [90.0, 80.0], [40.0, 80.0]], false),
            ] {
                let mut app = app_with_corner_guides(menu);
                app.run("prefs.set", json!({"path": "general.useLegacyFreeTransform", "value": legacy})).unwrap();
                let doc = app.session.active().unwrap().doc.clone();
                let past = app.session.active().unwrap().history.past_len();
                let prefs = serde_json::to_value(app.session.prefs()).unwrap();
                // Ctrl arrives after the raw, off-centre press, as on the actual canvas path.
                move_corner(&mut app, 1, [4.0, 3.0], [108.0, 32.0], egui::Modifiers::NONE, ctrl);
                assert_eq!(app.ui.transform.as_ref().unwrap().quad, expected, "{menu}, legacy {legacy}");
                assert_eq!(!app.prefs_rt.snap.as_ref().unwrap().disabled, snapping, "{menu}");
                assert_eq!(app.prefs_rt.snap_lines.len(), if snapping { 2 } else { 0 }, "{menu}");
                crate::canvas::tool_event(&mut app, ToolEvent::Up { x: 112.0, y: 35.0 }, ctrl);
                assert_eq!(app.ui.transform.as_ref().unwrap().quad, expected, "release preserves {menu}");
                crate::transform_tool::cancel(&mut app);
                assert!(app.ui.transform.is_none() && app.prefs_rt.snap.is_none() && app.prefs_rt.snap_lines.is_empty());
                assert!(std::sync::Arc::ptr_eq(&app.session.active().unwrap().doc, &doc), "cancel preserves {menu}'s document");
                assert_eq!(app.session.active().unwrap().history.past_len(), past, "cancel preserves {menu}'s history");
                assert_eq!(serde_json::to_value(app.session.prefs()).unwrap(), prefs, "{menu} does not change preferences");
            }
        }
    }

    #[test]
    fn skew_control_corner_on_editable_type_slides_the_whole_edge() {
        let ctrl = egui::Modifiers { command: true, ctrl: true, ..egui::Modifiers::NONE };
        for legacy in [false, true] {
            let mut app = app_with_box();
            app.run("type.create", json!({"x": 40, "y": 60, "text": "Hello"})).unwrap();
            app.run("prefs.set", json!({"path": "general.useLegacyFreeTransform", "value": legacy})).unwrap();
            crate::menus::invoke(&mut app, &egui::Context::default(), "edit.transform.skew", json!({})).unwrap();
            let before = app.ui.transform.as_ref().unwrap().quad;
            let end = [before[1][0] + 18.0, before[1][1] - 8.0];
            let doc = app.session.active().unwrap().doc.clone();
            let past = app.session.active().unwrap().history.past_len();
            move_corner(&mut app, 1, [2.0, 1.0], end, egui::Modifiers::NONE, ctrl);
            let mut expected = before;
            expected[0][0] += 18.0;
            expected[1][0] += 18.0;
            assert_eq!(app.ui.transform.as_ref().unwrap().quad, expected, "type remains affine, legacy {legacy}");
            assert!(app.prefs_rt.snap.as_ref().unwrap().disabled && app.prefs_rt.snap_lines.is_empty());
            crate::canvas::tool_event(&mut app, ToolEvent::Up { x: end[0] + 2.0, y: end[1] + 1.0 }, ctrl);
            assert_eq!(app.ui.transform.as_ref().unwrap().quad, expected, "release preserves the type's edge");
            crate::transform_tool::cancel(&mut app);
            assert!(std::sync::Arc::ptr_eq(&app.session.active().unwrap().doc, &doc));
            assert_eq!(app.session.active().unwrap().history.past_len(), past);
        }
    }

    #[test]
    fn transform_non_distortion_gestures_keep_control_snap_override() {
        let ctrl = egui::Modifiers { command: true, ctrl: true, ..egui::Modifiers::NONE };
        let perspective = egui::Modifiers { alt: true, shift: true, ..ctrl };
        for (menu, press, m) in [
            ("edit.freeTransform", [65.0, 40.0], ctrl), // edge
            ("edit.freeTransform", [65.0, 60.0], ctrl), // pivot
            ("edit.freeTransform", [55.0, 52.0], ctrl), // inside
            ("edit.freeTransform", [90.0, 40.0], perspective),
            ("edit.transform.perspective", [90.0, 40.0], ctrl),
            ("edit.transform.warp", [90.0, 40.0], ctrl),
            ("edit.contentAwareScale", [90.0, 40.0], ctrl),
        ] {
            let mut app = app_with_corner_guides(menu);
            crate::canvas::tool_event(&mut app, ToolEvent::Down { x: press[0], y: press[1], pressure: 1.0 }, m);
            crate::canvas::tool_event(&mut app, ToolEvent::Move { x: 108.0, y: 32.0, pressure: 1.0 }, m);
            assert!(app.prefs_rt.snap.as_ref().unwrap().disabled, "{menu}, {press:?}, {m:?}");
            assert!(app.prefs_rt.snap_lines.is_empty(), "{menu}, {press:?}");
        }
    }

    #[test]
    fn editable_type_refusal_keeps_control_snap_override() {
        let mut app = app_with_box();
        app.run("type.create", json!({"x": 40, "y": 60, "text": "Hello"})).unwrap();
        crate::menus::invoke(&mut app, &egui::Context::default(), "edit.freeTransform", json!({})).unwrap();
        let corner = app.ui.transform.as_ref().unwrap().quad[1];
        app.run("view.newGuide", json!({"orientation": "vertical", "position": corner[0] + 20.0})).unwrap();
        app.run("view.newGuide", json!({"orientation": "horizontal", "position": corner[1] - 10.0})).unwrap();
        let ctrl = egui::Modifiers { command: true, ctrl: true, ..egui::Modifiers::NONE };
        move_corner(&mut app, 1, [0.0, 0.0], [corner[0] + 18.0, corner[1] - 8.0], ctrl, ctrl);
        assert!(app.prefs_rt.snap.as_ref().unwrap().disabled);
        assert!(app.prefs_rt.snap_lines.is_empty());
        let q = app.ui.transform.as_ref().unwrap().quad;
        assert_eq!(q[0][1], q[1][1], "editable type still scales, rather than distorting");
    }

    #[test]
    fn a_foldover_constraint_never_claims_a_guide_alignment() {
        let mut app = app_with_corner_guides("edit.transform.distort");
        app.run("view.newGuide", json!({"orientation": "horizontal", "position": 100})).unwrap();
        let before = app.ui.transform.as_ref().unwrap().quad;
        move_corner(&mut app, 0, [4.0, 3.0], [108.0, 98.0], egui::Modifiers::NONE, egui::Modifiers::NONE);
        let q = app.ui.transform.as_ref().unwrap().quad;
        assert_ne!(q[0], [110.0, 100.0], "the guide intersection would fold the quad");
        assert_eq!([q[1], q[2], q[3]], [before[1], before[2], before[3]]);
        for i in 0..4 {
            let (a, b, c) = (q[i], q[(i + 1) % 4], q[(i + 2) % 4]);
            assert!((b[0] - a[0]) * (c[1] - b[1]) - (b[1] - a[1]) * (c[0] - b[0]) > 0.0, "still convex: {q:?}");
        }
        assert!(app.prefs_rt.snap_lines.is_empty(), "{:?}", app.prefs_rt.snap_lines);
    }

    #[test]
    fn a_snapped_distort_commits_one_undoable_transform() {
        let mut app = app_with_corner_guides("edit.transform.distort");
        let before_doc = app.session.active().unwrap().doc.clone();
        let before_bounds = mover_bounds(&app);
        let past = app.session.active().unwrap().history.past_len();
        move_corner(&mut app, 1, [4.0, 3.0], [108.0, 32.0], egui::Modifiers::NONE, egui::Modifiers::NONE);
        assert!(std::sync::Arc::ptr_eq(&app.session.active().unwrap().doc, &before_doc), "preview does not edit the document");
        crate::canvas::tool_event(&mut app, ToolEvent::Up { x: 112.0, y: 35.0 }, egui::Modifiers::NONE);
        crate::transform_tool::commit(&mut app);
        assert!(!app.ui.status_error, "{}", app.ui.status);
        let after_bounds = mover_bounds(&app);
        assert_ne!(after_bounds, before_bounds);
        assert_eq!(app.session.active().unwrap().history.past_len(), past + 1);
        app.run("edit.undo", json!({})).unwrap();
        assert_eq!(mover_bounds(&app), before_bounds);
        app.run("edit.redo", json!({})).unwrap();
        assert_eq!(mover_bounds(&app), after_bounds);
    }

    #[test]
    fn transform_corner_press_is_not_adjusted_near_an_original_guide() {
        for menu in ["edit.transform.distort", "edit.freeTransform"] {
            let mut app = app_with_corner_guides(menu);
            app.run("view.newGuide", json!({"orientation": "vertical", "position": 95})).unwrap();
            app.run("view.newGuide", json!({"orientation": "horizontal", "position": 45})).unwrap();
            let raw = ToolEvent::Down { x: 94.0, y: 43.0, pressure: 1.0 };
            let ToolEvent::Down { x, y, .. } = filter_event(&mut app, raw, egui::Modifiers::NONE) else { panic!("still a press") };
            assert_eq!([x, y], [94.0, 43.0], "{menu}: filtering must preserve the handle press");
        }
    }

    #[test]
    fn corner_near_original_guides_keeps_the_raw_press_in_the_canvas_path() {
        for (menu, m) in
            [("edit.transform.distort", egui::Modifiers::NONE), ("edit.freeTransform", egui::Modifiers { command: true, ctrl: true, ..egui::Modifiers::NONE })]
        {
            let mut app = app_with_corner_guides(menu);
            app.run("view.newGuide", json!({"orientation": "vertical", "position": 95})).unwrap();
            app.run("view.newGuide", json!({"orientation": "horizontal", "position": 45})).unwrap();
            let before = app.ui.transform.as_ref().unwrap().quad;
            // The command modifier is added after Down, including in Free Transform.
            move_corner(&mut app, 1, [4.0, 3.0], [93.0, 43.0], egui::Modifiers::NONE, m);
            let q = app.ui.transform.as_ref().unwrap().quad;
            assert_eq!(q[1], [95.0, 45.0], "{menu}");
            assert_eq!([q[0], q[2], q[3]], [before[0], before[2], before[3]]);
        }
    }

    #[test]
    fn corner_distortion_does_not_snap_to_guides_hidden_by_view_flags() {
        for menu in ["edit.transform.distort", "edit.freeTransform"] {
            for hide_extras in [false, true] {
                let mut app = app_with_corner_guides(menu);
                if hide_extras {
                    app.ui.view.extras = false;
                } else {
                    app.ui.view.show.canvas_guides = false;
                }
                let m = egui::Modifiers { command: true, ctrl: true, ..egui::Modifiers::NONE };
                move_corner(&mut app, 1, [4.0, 3.0], [108.0, 32.0], m, m);
                assert_eq!(app.ui.transform.as_ref().unwrap().quad[1], [108.0, 32.0], "{menu}, hide Extras: {hide_extras}");
                assert!(app.prefs_rt.snap_lines.is_empty());
            }
        }
    }

    #[test]
    fn marquee_and_corner_snapping_share_the_canvas_guide_visibility() {
        for (extras, canvas_guides, shown) in [(true, true, true), (false, true, false), (true, false, false)] {
            let mut app = app_with_box();
            app.run("view.newGuide", json!({"orientation": "vertical", "position": 110})).unwrap();
            app.run("view.newGuide", json!({"orientation": "horizontal", "position": 30})).unwrap();
            app.ui.tool = Tool::RectMarquee;
            // Isolate guides: the mover's y=40 edge would otherwise be a valid fallback target.
            app.ui.view.snap_to.layers = false;
            app.ui.view.snap_to.document_bounds = false;
            app.ui.view.snap_to.grid = false;
            app.ui.view.extras = extras;
            app.ui.view.show.canvas_guides = canvas_guides;
            let m = egui::Modifiers::NONE;
            crate::canvas::tool_event(&mut app, ToolEvent::Down { x: 12.0, y: 12.0, pressure: 1.0 }, m);
            crate::canvas::tool_event(&mut app, ToolEvent::Up { x: 108.0, y: 32.0 }, m);
            let rect = app.session.active().unwrap().doc.selection.as_ref().unwrap().content_bounds();
            assert_eq!((rect.x0, rect.y0), (12, 12));
            assert_eq!((rect.x1, rect.y1), if shown { (110, 30) } else { (108, 32) }, "Extras: {extras}, Canvas Guides: {canvas_guides}");
        }
    }

    #[test]
    fn distort_corner_does_not_snap_to_layer_edges() {
        let mut app = app_with_box();
        let ctx = egui::Context::default();
        crate::menus::invoke(&mut app, &ctx, "edit.transform.distort", json!({})).unwrap();
        let q = app.ui.transform.as_ref().unwrap().quad;
        let m = egui::Modifiers::NONE;
        // Drag the top-right corner to 2 px short of the target's right edge (x = 350).
        crate::canvas::tool_event(&mut app, ToolEvent::Down { x: q[1][0], y: q[1][1], pressure: 1.0 }, m);
        crate::canvas::tool_event(&mut app, ToolEvent::Up { x: 348.0, y: q[1][1] }, m);
        let q = app.ui.transform.as_ref().unwrap().quad;
        assert_eq!(q[1][0], 348.0, "the corner stays where it was dropped");
        assert!(app.prefs_rt.snap_lines.is_empty());
    }

    #[test]
    fn free_transform_drags_and_handles_snap() {
        let mut app = app_with_box();
        let ctx = egui::Context::default();
        crate::menus::invoke(&mut app, &ctx, "edit.freeTransform", json!({})).unwrap();
        assert!(app.ui.transform.is_some());
        let m = egui::Modifiers::NONE;
        // Drag from inside the box (40..90 × 40..80) so its left edge lands 2 px from x = 300.
        // (Away from the centre, which holds the reference point.)
        crate::canvas::tool_event(&mut app, ToolEvent::Down { x: 50.0, y: 50.0, pressure: 1.0 }, m);
        crate::canvas::tool_event(&mut app, ToolEvent::Up { x: 312.0, y: 50.0 }, m);
        let q = app.ui.transform.as_ref().unwrap().quad;
        assert!((q[0][0] - 300.0).abs() < 1e-6, "{q:?}");
        // The right-edge handle snaps to the target's right edge (x = 350).
        crate::canvas::tool_event(&mut app, ToolEvent::Down { x: q[1][0], y: (q[1][1] + q[2][1]) / 2.0, pressure: 1.0 }, m);
        crate::canvas::tool_event(&mut app, ToolEvent::Up { x: 353.0, y: 60.0 }, m);
        let q = app.ui.transform.as_ref().unwrap().quad;
        assert!((q[1][0] - 350.0).abs() < 1e-6, "{q:?}");
        crate::transform_tool::commit(&mut app);
        assert_eq!(mover_bounds(&app).x0, 300);
    }

    /// #5751: a Rectangle drawn on a 1080 px canvas, then resized with Free Transform under the
    /// Move tool. With Snap Vector Tools and Transforms to Pixel Grid on (the default) the box's
    /// edges stay on whole pixels: the grab offset and a proportional scale used to leave them at
    /// 20.6 and 1040.4. Off, the box follows the pointer exactly.
    #[test]
    fn free_transform_of_a_shape_keeps_whole_pixels() {
        for grid in [true, false] {
            let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
            app.run("file.new", json!({"width": 1080, "height": 1080, "resolution": 72})).unwrap();
            app.run("prefs.set", json!({"path": "tools.snapVectorToolsAndTransformsToPixelGrid", "value": grid})).unwrap();
            app.sync_views();
            app.ui.views[0].zoom = 0.6;
            app.ui.views[0].fit_pending = false;
            app.ui.tool = Tool::Rectangle;
            let m = egui::Modifiers::NONE;
            crate::canvas::tool_event(&mut app, ToolEvent::Down { x: 40.0, y: 31.0, pressure: 1.0 }, m);
            crate::canvas::tool_event(&mut app, ToolEvent::Move { x: 300.0, y: 700.0, pressure: 1.0 }, m);
            crate::canvas::tool_event(&mut app, ToolEvent::Up { x: 481.0, y: 1000.0 }, m);
            app.ui.tool = Tool::Move;
            crate::menus::invoke(&mut app, &egui::Context::default(), "edit.freeTransform", json!({})).unwrap();
            let whole = |q: [[f64; 2]; 4]| q.iter().flatten().all(|v| v.fract() == 0.0);
            // The left edge handle, grabbed 1.3 px inside, dropped away from any snap target.
            let q = app.ui.transform.as_ref().unwrap().quad;
            let mid_y = (q[0][1] + q[3][1]) / 2.0;
            crate::canvas::tool_event(&mut app, ToolEvent::Down { x: q[0][0] + 1.3, y: mid_y, pressure: 1.0 }, m);
            crate::canvas::tool_event(&mut app, ToolEvent::Move { x: 60.0, y: mid_y, pressure: 1.0 }, m);
            crate::canvas::tool_event(&mut app, ToolEvent::Up { x: 80.6, y: mid_y }, m);
            let q = app.ui.transform.as_ref().unwrap().quad;
            assert_eq!(whole(q), grid, "grid {grid}: left edge {q:?}");
            // The bottom-right corner scaled proportionally (the default), off any snap target.
            crate::canvas::tool_event(&mut app, ToolEvent::Down { x: q[2][0], y: q[2][1], pressure: 1.0 }, m);
            crate::canvas::tool_event(&mut app, ToolEvent::Up { x: 450.4, y: 900.1 }, m);
            let q = app.ui.transform.as_ref().unwrap().quad;
            assert_eq!(whole(q), grid, "grid {grid}: corner {q:?}");
            crate::transform_tool::commit(&mut app);
            let st = app.session.active().unwrap();
            let photocraft_doc::LayerContent::Shape(sh) = &st.doc.layer(st.active_layer.unwrap()).unwrap().content else { panic!("a shape layer") };
            let (x0, y0, x1, y1) = sh.path.control_bounds().unwrap();
            if grid {
                for v in [x0, y0, x1, y1] {
                    assert!((v - v.round()).abs() < 1e-6, "committed shape {:?}", (x0, y0, x1, y1));
                }
            }
        }
    }

    #[test]
    fn guides_snap_to_layer_edges() {
        let mut app = app_with_box();
        assert_eq!(snap_guide(&mut app, true, 297.0), 300.0);
        assert_eq!(snap_guide(&mut app, false, 150.0), 150.0, "document centre");
        assert_eq!(snap_guide(&mut app, false, 170.0), 170.0);
        // Dragging an existing guide with the Move tool while a selection is active still snaps to the selection.
        app.run("view.newGuide", json!({"orientation": "vertical", "position": 10})).unwrap();
        app.run("select.rect", json!({"x": 100, "y": 100, "width": 50, "height": 40})).unwrap();
        app.ui.tool = Tool::Move;
        begin(&mut app, [10.0, 50.0], egui::Modifiers::NONE);
        assert_eq!(apply(&mut app, [98.0, 50.0], egui::Modifiers::NONE)[0], 100.0);
        app.ui.extras.snap = false;
        assert_eq!(snap_guide(&mut app, true, 297.0), 297.0);
    }

    /// The cached ruler-guide targets follow the Snap To options, not just document edits.
    #[test]
    fn guide_targets_follow_snap_to_options() {
        let mut app = app_with_box();
        app.run(
            "prefs.set",
            json!({"values": {"guidesGridAndSlices.gridlineEvery": 100, "guidesGridAndSlices.gridUnit": "pixels", "guidesGridAndSlices.subdivisions": 4}}),
        )
        .unwrap();
        app.ui.extras.grid = false;
        assert_eq!(snap_guide(&mut app, true, 23.0), 23.0, "grid hidden");
        app.ui.extras.grid = true;
        app.ui.view.snap_to.grid = true;
        assert_eq!(snap_guide(&mut app, true, 23.0), 25.0, "grid shown");
        app.ui.extras.grid = false;
        app.ui.view.snap_to.layers = false;
        assert_eq!(snap_guide(&mut app, true, 297.0), 297.0, "Snap To Layers off");
    }

    #[test]
    fn percentage_grid_snaps_at_separate_axis_intervals() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 1280, "height": 720})).unwrap();
        app.ui.extras.grid = true;
        app.ui.view.snap_to.grid = true;
        app.run(
            "prefs.set",
            json!({"values": {"guidesGridAndSlices.gridlineEvery": 20, "guidesGridAndSlices.gridUnit": "percent", "guidesGridAndSlices.subdivisions": 2}}),
        )
        .unwrap();
        let opts = options(&app);
        assert_eq!(opts.grid_step, [128.0, 72.0]);
        let targets = SnapTargets::from_document(&app.session.active().unwrap().doc, &opts, &[]);
        assert_eq!(targets.snap_point([127.0, 71.0], 2.0).0, [128.0, 72.0]);
    }

    #[test]
    fn grid_snapping_uses_preferences() {
        let mut app = app_with_box();
        app.ui.extras.grid = true;
        app.run(
            "prefs.set",
            json!({"values": {"guidesGridAndSlices.gridlineEvery": 100, "guidesGridAndSlices.gridUnit": "pixels", "guidesGridAndSlices.subdivisions": 4}}),
        )
        .unwrap();
        assert_eq!(options(&app).grid_step, [25.0; 2]);
        app.ui.tool = Tool::RectMarquee;
        let m = egui::Modifiers::NONE;
        crate::canvas::tool_event(&mut app, ToolEvent::Down { x: 152.0, y: 127.0, pressure: 1.0 }, m);
        crate::canvas::tool_event(&mut app, ToolEvent::Up { x: 199.0, y: 176.0 }, m);
        let sel = app.session.active().unwrap().doc.selection.as_ref().unwrap().content_bounds();
        assert_eq!((sel.x0, sel.y0, sel.x1, sel.y1), (150, 125, 200, 175));
    }
}
