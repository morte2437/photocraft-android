use super::begin::warps_type;
use super::{BezierMesh, CursorIcon, PhotocraftApp, SplitTool, ToolEvent, ViewXform, Warp, WarpDrag, WarpStyle, json};

/// An anchor or a handle on a section line. The inner control points of a patch are not shown.
pub(super) fn on_section_edge(i: usize, j: usize) -> bool {
    i.is_multiple_of(3) || j.is_multiple_of(3)
}

/// Index of the warp control point within `tol` of `p` (nearest anchor or handle).
pub(super) fn warp_hit(w: &Warp, p: [f64; 2], tol: f64) -> Option<usize> {
    let m = w.mesh.as_ref()?;
    let nx = m.nx();
    m.points
        .iter()
        .enumerate()
        .filter(|(i, _)| on_section_edge(i % nx, i / nx))
        .map(|(i, q)| (i, ((q[0] - p[0]).powi(2) + (q[1] - p[1]).powi(2)).sqrt()))
        .filter(|(_, d)| *d < tol)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(i, _)| i)
}

/// A 1×1 mesh (Grid › Default) draws rule-of-thirds guides. Denser presets are real patch
/// boundaries only — a 3×3 is three cells, not a third-line lattice inside every cell.
pub(super) fn single_patch(mesh: &BezierMesh) -> bool {
    mesh.us.len() == 2 && mesh.vs.len() == 2
}

/// The surface parameters under `p` when it is inside the warped mesh, else None.
pub(super) fn surface_at(mesh: &BezierMesh, p: [f64; 2]) -> Option<(f64, f64)> {
    let (s, t) = mesh.param_at(p);
    let q = mesh.eval(s, t);
    ((q[0] - p[0]).hypot(q[1] - p[1]) < 1.0).then_some((s, t))
}

/// Ctrl-click (or Option-click) picks the split: crosswise in the open, and the perpendicular
/// split when the pointer's grid position `(s, t)` is close to an existing grid line.
pub(super) fn quick_split_kind(mesh: &BezierMesh, bounds: [f64; 4], (s, t): (f64, f64)) -> SplitTool {
    let w = (bounds[2] - bounds[0]).abs().max(1.0);
    let h = (bounds[3] - bounds[1]).abs().max(1.0);
    let su = (8.0 / w).clamp(0.02, 0.12);
    let sv = (8.0 / h).clamp(0.02, 0.12);
    let thirds = single_patch(mesh);
    let near = |knots: &[f64], v: f64, tol: f64| {
        knots.iter().any(|k| *k > 0.0 && *k < 1.0 && (k - v).abs() < tol) || (thirds && [1.0 / 3.0, 2.0 / 3.0].iter().any(|k| (k - v).abs() < tol))
    };
    match (near(&mesh.us, s, su), near(&mesh.vs, t, sv)) {
        (true, false) => SplitTool::Horizontal,
        (false, true) => SplitTool::Vertical,
        _ => SplitTool::Cross,
    }
}

/// The cursor for a split armed at grid position `(s, t)` of warp grid `m`: the arrow across the
/// split line as it looks on screen. A vertical split line runs along the grid's columns and is
/// placed by moving across them, in the grid's own direction there, so the arrow turns with a
/// turned or bent grid and with the view.
pub(super) fn split_cursor(m: &BezierMesh, tool: SplitTool, (s, t): (f64, f64), xf: &ViewXform) -> CursorIcon {
    let e = 1e-3;
    let (a, b) = match tool {
        SplitTool::Vertical => (m.eval((s - e).max(0.0), t), m.eval((s + e).min(1.0), t)),
        SplitTool::Horizontal => (m.eval(s, (t - e).max(0.0)), m.eval(s, (t + e).min(1.0))),
        SplitTool::Cross => return CursorIcon::Crosshair,
    };
    crate::tool_cursor::resize([b[0] - a[0], b[1] - a[1]], xf)
}

/// Armed split tool, or Option held: the guide tracks the pointer and the split lands on release.
/// Returns false when this event is an ordinary control-point drag.
fn split_gesture(app: &mut PhotocraftApp, ev: ToolEvent, mods: egui::Modifiers) -> bool {
    let armed = app.transform_preview.as_ref().and_then(|p| p.split_tool);
    let placing = app.transform_preview.as_ref().is_some_and(|p| p.split_placing);
    match ev {
        ToolEvent::Down { x, y, .. } if armed.is_some() || mods.alt || mods.command => {
            if let Some(pv) = app.transform_preview.as_mut() {
                pv.split_placing = true;
                pv.split_pointer = Some([x, y]);
                pv.split_quick = armed.is_none();
            }
            true
        }
        ToolEvent::Move { x, y, .. } if placing => {
            if let Some(pv) = app.transform_preview.as_mut() {
                pv.split_pointer = Some([x, y]);
            }
            true
        }
        ToolEvent::Up { x, y } if placing => {
            let quick = app.transform_preview.as_ref().is_some_and(|p| p.split_quick);
            if let Some(pv) = app.transform_preview.as_mut() {
                pv.split_placing = false;
                pv.split_pointer = None;
                pv.split_quick = false;
            }
            let kind = if let Some(tool) = armed {
                Some(tool)
            } else if quick {
                app.ui.transform.as_ref().and_then(|t| t.warp.as_ref()).map(|w| {
                    let m = w.to_mesh(1, 1);
                    quick_split_kind(&m, w.bounds, m.param_at([x, y]))
                })
            } else {
                None
            };
            if let Some(kind) = kind {
                // On an existing line, or a click the drag path also delivers, this is a no-op.
                let _ = split(app, kind.command(), Some([x, y]));
            }
            true
        }
        _ => false,
    }
}

/// Drags the warp grid. A press on an anchor or handle moves that point (an anchor carries its
/// handles along); a press anywhere else inside the grid pulls the surface under the pointer.
/// Preset warps turn into a custom mesh when the press grabs something. An armed split tool, or
/// Ctrl / Option held, places a split where the pointer is released instead.
pub(super) fn warp_pointer(app: &mut PhotocraftApp, ev: ToolEvent, tol: f64, mods: egui::Modifiers) {
    // Type takes only the warp styles: nothing on the canvas to drag or split (#3004).
    if warps_type(app) {
        return;
    }
    if split_gesture(app, ev, mods) {
        return;
    }
    let (Some(t), Some(pv)) = (app.ui.transform.as_mut(), app.transform_preview.as_mut()) else { return };
    let Some(w) = t.warp.as_mut() else { return };
    match ev {
        ToolEvent::Down { x, y, .. } => {
            // A preset warp becomes a custom mesh only when the press grabs one of its points.
            let custom = if w.mesh.is_none() || w.style != WarpStyle::Custom { Warp::custom(w.to_mesh(1, 1), w.bounds) } else { w.clone() };
            let pts0 = custom.mesh.as_ref().map(|m| m.points.clone()).unwrap_or_default();
            pv.warp_drag = warp_hit(&custom, [x, y], tol).map(|index| WarpDrag::Point { index, start: [x, y], pts0: pts0.clone() }).or_else(|| {
                let (s, t) = surface_at(custom.mesh.as_ref()?, [x, y])?;
                Some(WarpDrag::Surface { s, t, start: [x, y], pts0 })
            });
            if pv.warp_drag.is_some() {
                *w = custom;
            }
        }
        ToolEvent::Move { x, y, .. } | ToolEvent::Up { x, y } => {
            match (&pv.warp_drag, w.mesh.as_mut()) {
                (Some(WarpDrag::Point { index, start, pts0 }), Some(m)) => m.drag_point(pts0, *index, [x - start[0], y - start[1]]),
                (Some(WarpDrag::Surface { s, t, start, pts0 }), Some(m)) => m.pull(pts0, *s, *t, [x - start[0], y - start[1]]),
                _ => {}
            }
            if matches!(ev, ToolEvent::Up { .. }) {
                pv.warp_drag = None;
            }
        }
    }
}

/// Runs a warp-edit command against the mesh in the open Warp session and writes the mesh back.
pub fn edit_session_warp(app: &mut PhotocraftApp, id: &str, params: &serde_json::Value) -> Result<(), String> {
    let Some(w) = app.ui.transform.as_ref().and_then(|t| t.warp.clone()) else { return Err("not warping".into()) };
    if warps_type(app) {
        return Err("Warp on type takes only the warp styles".into());
    }
    let mut p = if params.is_object() { params.clone() } else { json!({}) };
    if p.get("warp").is_none() {
        p["warp"] = serde_json::to_value(&w).map_err(|e| e.to_string())?;
    }
    let r = app.session.execute(id, p).map_err(|e| e.to_string())?;
    let raw = r.get("warp").cloned().ok_or_else(|| "warp command returned no mesh".to_string())?;
    let nw: Warp = serde_json::from_value(raw).map_err(|e| e.to_string())?;
    // A warp-edit command can replace the mesh while pointer input is split across control
    // requests. Any drag index captured from the old mesh is invalid once that happens.
    if let Some(pv) = app.transform_preview.as_mut() {
        pv.warp_drag = None;
    }
    if let Some(t) = app.ui.transform.as_mut() {
        t.warp = Some(nw);
    }
    Ok(())
}

/// Applies a split-warp command to the mesh being edited (at the box centre unless `at`).
pub fn split(app: &mut PhotocraftApp, id: &str, at: Option<[f64; 2]>) -> Result<(), String> {
    let mut p = json!({});
    if let Some(a) = at {
        p["at"] = json!(a);
    }
    edit_session_warp(app, id, &p)
}
