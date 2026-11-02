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
    /// Default crop mode, a drag inside an upright box: the image moves under it, so the frame
    /// moves the other way over the document and its edges and centre snap ([`crop_image`]).
    CropImage,
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
    // Puppet Warp's pins and Perspective Warp's quads go where they are dragged, as in Photoshop:
    // the press is theirs, not the tool's (with the Move tool it took a layer move's Smart Guides).
    if app.session.active().is_none() || app.distort.active() {
        return;
    }
    // The tool the press acts as: ⌘ makes a painting tool the Move tool, and a selection tool
    // outside the selection moves the layer as the Move tool does (#2681).
    let tool = crate::canvas::event_tool(app, mods);
    let moves_layer = crate::canvas::command_moves_layer(app, tool, p, mods);
    let crop_tol = crate::crop_ui::tolerance(app);
    let gesture = if let Some(t) = &app.ui.transform {
        let corner = match crate::transform_tool::hit(t, p, crate::transform_tool::handle_tolerance(app)) {
            crate::transform_tool::Hit::Corner(index) => {
                t.quad.get(index).map(|&point| (Gesture::TransformCorner { index, point }, crate::transform_tool::moving_layers(t)))
            }
            _ => None,
        };
        if t.mode == crate::state::TransformMode::Distort {
            corner
        } else if near_handle(&t.quad, p, crate::transform_tool::handle_tolerance(app)) {
            corner.or_else(|| Some((Gesture::Point, crate::transform_tool::moving_layers(t))))
        } else if in_quad(&t.quad, p) {
            Some((Gesture::TransformMove { rect: quad_rect(&t.quad) }, crate::transform_tool::moving_layers(t)))
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
        && crate::crop_mode::moves_image(app)
        && !app.crop.default_frame
        && crate::crop_ui::angle(app) == 0.0
        && app.ui.crop_rect.is_some_and(|r| crate::crop_ui::hit(r, p, crop_tol) == crate::crop_ui::Hit::Inside)
    {
        // The default mode moves the image under an upright box: the frame's edges and centre
        // snap, as an edited Classic Mode frame's do, so content can be centred in it (#8580).
        Some((Gesture::CropImage, Vec::new()))
    } else if tool == Tool::Crop
        && (crate::crop_ui::turns_at(app, p)
            || app.ui.crop_rect.is_some_and(|r| crate::crop_ui::hit_turned(r, crate::crop_ui::angle(app), p, crop_tol) == crate::crop_ui::Hit::Inside))
    {
        // Turning the frame, moving a turned one (its edges don't line up with anything), or in
        // the default mode turning the image or moving it under a turned box: no snapping.
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
    let smart = if snap_on(app) && matches!(gesture, Gesture::Move { .. } | Gesture::CropImage) && smart_on(app) {
        build(app, &exclude, true)
    } else {
        SnapTargets::default()
    };
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
        Gesture::CropImage => {
            // Smart guides still snap when nothing in View › Snap To is within reach: the
            // document's bounds alone keep `targets` from ever being empty.
            let snapped = |set| crop_image(app, p, set, tol).filter(|(_, lines)| !lines.is_empty());
            snapped(&snap.targets).or_else(|| snapped(&snap.smart)).unwrap_or((p, Vec::new()))
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

/// Snap the image being moved under the crop box (default mode): the frame moves by the image's
/// travel the other way over the document, so its edges and centre snap and the pointer moves by
/// the frame's correction the other way too (#8580). As `crop_ui` reads this drag, the pointer is
/// read in the camera at the press, since the view pans with the frame meanwhile.
fn crop_image(app: &PhotocraftApp, p: [f64; 2], targets: &SnapTargets, tol: f64) -> Option<([f64; 2], Vec<SnapLine>)> {
    let crate::crop_ui::CropDrag::MoveImage { start, rect, view } = app.crop.drag? else { return None };
    let q = crate::crop_mode::to_press(app, view, p);
    let d = [q[0] - start[0], q[1] - start[1]];
    let (adj, lines) = targets.snap_rect([rect[0] - d[0], rect[1] - d[1], rect[2] - d[0], rect[3] - d[1]], tol);
    if adj == [0.0, 0.0] {
        return Some((p, lines));
    }
    let q = [q[0] - adj[0], q[1] - adj[1]];
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
mod tests;
