use super::paths_panel::{PathEntry, PathRow, path_rows};
use super::pen::pen_commit;
use super::*;

/// The path that path commands act on without a name: the one selected in the Paths panel, else
/// the work path, else the selected layer's shape path or vector mask; `None` when there's none.
pub fn active_path_name(app: &PhotocraftApp) -> Option<String> {
    let st = app.session.active()?;
    let rows = path_rows(&st.doc, st.active_layer);
    let key = |r: &PathEntry| match r.kind {
        PathRow::Work => "work".to_string(),
        PathRow::Layer => "layer".to_string(),
        PathRow::Saved => r.name.clone(),
    };
    let selected = app.ui.selected_path.as_deref().filter(|s| rows.iter().any(|r| key(r) == *s)).map(str::to_string);
    selected.or_else(|| rows.iter().find(|r| r.kind == PathRow::Work).map(key)).or_else(|| rows.iter().find(|r| r.kind == PathRow::Layer).map(key))
}

/// The commands that create a fill or adjustment layer and take a `"path"` as its vector mask.
pub fn takes_path_mask(id: &str) -> bool {
    id.starts_with("layer.newFillLayer.") || id.starts_with("layer.newAdjustmentLayer.")
}

/// The work or saved path explicitly selected in the Paths panel (not a layer's own path).
pub fn selected_document_path(app: &PhotocraftApp) -> Option<String> {
    let st = app.session.active()?;
    let sel = app.ui.selected_path.as_deref()?;
    let listed = path_rows(&st.doc, st.active_layer).iter().any(|r| match r.kind {
        PathRow::Work => sel == "work",
        PathRow::Saved => r.name == sel,
        PathRow::Layer => false,
    });
    listed.then(|| sel.to_string())
}

/// New fill and adjustment layers take the path selected in the Paths panel as their vector mask,
/// as in Photoshop (#1419): the command gets it as `"path"` unless the caller set one (`null`
/// opts out). Only an explicitly selected work or saved path counts; deselect it by clicking the
/// panel's empty area.
pub fn with_active_path(app: &PhotocraftApp, id: &str, params: Value) -> Value {
    if !takes_path_mask(id) || params.get("path").is_some() {
        return params;
    }
    let Some(sel) = selected_document_path(app) else { return params };
    match params {
        Value::Object(mut m) => {
            m.insert("path".into(), json!(sel));
            Value::Object(m)
        }
        _ => json!({ "path": sel }),
    }
}

/// ⌘↩ / Ctrl+Enter (#306): load a path as a selection, like Photoshop with a Pen or Path
/// Selection tool or a path selected in the Paths panel. A path being drawn with the Pen is
/// finished first and loads; otherwise [`active_path_name`]. `params` may set feather, mode…
pub fn path_to_selection(app: &mut PhotocraftApp, params: Value) -> Result<Value, String> {
    if app.ui.pen.as_ref().is_some_and(|p| p.knots.len() >= 2) {
        // The finished path becomes the work path, or a shape layer's path or a vector mask.
        let on_layer = app.ui.tool_options.vector_mode == "shape" || targeted_vector_mask(app).is_some();
        pen_commit(app, false);
        app.ui.selected_path = Some(if on_layer { "layer" } else { "work" }.into());
    }
    let name = active_path_name(app)
        .ok_or_else(|| tl!("No path to make a selection from: draw one with the Pen tool or select one in the Paths panel.").to_string())?;
    let mut p = params.as_object().cloned().unwrap_or_default();
    p.insert("name".into(), json!(name));
    app.run("path.toSelection", Value::Object(p))
}

// ---------------------------------------------------------------------------------------------
// Path Selection

/// What Path Selection edits.
enum PathTarget {
    Shape(u64),
    /// The active layer's vector mask, when its Layers thumbnail is targeted (#196).
    VectorMask(u64),
    Work,
}

/// The targeted vector mask of the active layer, if the Layers panel targets it.
pub(crate) fn targeted_vector_mask(app: &PhotocraftApp) -> Option<(u64, Path)> {
    let st = app.session.active()?;
    let l = st.active_layer.and_then(|id| st.doc.layer(id))?;
    let targeted = app.ui.vector_mask_target || st.channel_view.target == photocraft_engine::channel_cmds::ChannelTarget::VectorMask;
    (targeted && !matches!(l.content, LayerContent::Shape(_))).then_some(())?;
    l.vector_mask.as_ref().map(|m| (l.id.0, m.path.clone()))
}

/// The active shape layer's path, if the active layer is a shape layer.
pub(crate) fn active_shape_path(app: &PhotocraftApp) -> Option<(u64, Path)> {
    let st = app.session.active()?;
    let l = st.active_layer.and_then(|id| st.doc.layer(id))?;
    match &l.content {
        LayerContent::Shape(sh) => Some((l.id.0, sh.path.clone())),
        _ => None,
    }
}

/// Does ⌫ / Delete delete the work path? With the Pen, Freeform Pen or Path Selection tool, no
/// path being drawn and the work path the one these tools show and edit (no shape layer or
/// targeted vector mask in front of it), as in Photoshop (#5702). It was drawn whenever one of
/// these tools was active, but the key reached Edit › Clear, so it looked impossible to delete.
/// Direct Selection and the Curvature Pen delete selected anchors in Photoshop, not the path.
pub(crate) fn delete_key_deletes_work_path(app: &PhotocraftApp) -> bool {
    matches!(app.ui.tool, Tool::Pen | Tool::FreeformPen | Tool::PathSelection)
        && app.ui.pen.is_none()
        && app.ui.transform.is_none()
        && app.ui.view.shows(app.ui.view.show.target_path)
        && matches!(target_path(app), Some((PathTarget::Work, _)))
}

/// The path Path Selection edits: the targeted vector mask, the active shape layer's path, else
/// the work path.
fn target_path(app: &PhotocraftApp) -> Option<(PathTarget, Path)> {
    if let Some((id, p)) = targeted_vector_mask(app) {
        return Some((PathTarget::VectorMask(id), p));
    }
    if let Some((id, p)) = active_shape_path(app) {
        return Some((PathTarget::Shape(id), p));
    }
    app.session.active()?.doc.work_path.clone().map(|p| (PathTarget::Work, p))
}

/// Edit › Free Transform Path (⌘T with Path Selection or Direct Selection): the path the
/// box transforms (the one Path Selection edits) and `path.transform`'s params that name it.
pub(crate) fn free_transform_path(app: &PhotocraftApp) -> Option<(Value, Path)> {
    if !matches!(app.ui.tool, Tool::PathSelection | Tool::DirectSelection) {
        return None;
    }
    let (target, path) = target_path(app)?;
    path.control_bounds()?;
    let params = match target {
        PathTarget::Shape(id) | PathTarget::VectorMask(id) => json!({"name": "layer", "layer": id}),
        PathTarget::Work => json!({"name": "work"}),
    };
    Some((params, path))
}

fn point_in_subpath(sp: &photocraft_doc::vector::Subpath, pt: [f64; 2]) -> bool {
    if !sp.closed || sp.knots.len() < 3 {
        return false;
    }
    let mut inside = false;
    let n = sp.knots.len();
    for i in 0..n {
        let j = (i + 1) % n;
        let xi = sp.knots[i].anchor.x;
        let yi = sp.knots[i].anchor.y;
        let xj = sp.knots[j].anchor.x;
        let yj = sp.knots[j].anchor.y;
        let intersect = ((yi > pt[1]) != (yj > pt[1])) && (pt[0] < (xj - xi) * (pt[1] - yi) / (yj - yi) + xi);
        if intersect {
            inside = !inside;
        }
    }
    inside
}

/// The topmost visible shape layer under `at` (document coordinates):
/// hits the shape path or anchor within `tol`, or any non-transparent pixel of the shape's raster surface,
/// or point inside the shape's filled geometry.
pub fn pick_shape_at(doc: &Document, at: [f64; 2], tol: f64) -> Option<photocraft_doc::LayerId> {
    let (x, y) = (at[0].floor() as i32, at[1].floor() as i32);
    for (path, _, l) in doc.walk().iter().rev() {
        if !l.visible || !(1..=path.len()).all(|n| doc.layer_at(&path[..n]).is_some_and(|a| a.visible)) {
            continue;
        }
        let board = path.first().and_then(|i| doc.layers.get(*i)).and_then(|t| t.artboard());
        if board.is_some_and(|a| !a.rect.contains(x, y)) {
            continue;
        }
        if let LayerContent::Shape(sh) = &l.content {
            if photocraft_vector::edit::hit(&sh.path, photocraft_geom::Point::new(at[0], at[1]), tol, &[]).is_some() {
                return Some(l.id);
            }
            if sh.fill.is_some() {
                if let Some(surf) = l.surface() {
                    let mut px = [[0.0f32; 4]; 1];
                    surf.read_rgba_into(photocraft_doc::Rect::from_xywh(x, y, 1, 1), &mut px);
                    if px[0][3] > 0.0 {
                        return Some(l.id);
                    }
                }
                if let Some(photocraft_doc::LiveShape::Rect { rect, .. }) = &sh.live {
                    let x0 = rect[0].min(rect[0] + rect[2]);
                    let x1 = rect[0].max(rect[0] + rect[2]);
                    let y0 = rect[1].min(rect[1] + rect[3]);
                    let y1 = rect[1].max(rect[1] + rect[3]);
                    if at[0] >= x0 && at[0] <= x1 && at[1] >= y0 && at[1] <= y1 {
                        return Some(l.id);
                    }
                }
                if let Some(photocraft_doc::LiveShape::Ellipse { rect }) = &sh.live {
                    let rx = rect[2].abs() / 2.0;
                    let ry = rect[3].abs() / 2.0;
                    if rx > 1e-6 && ry > 1e-6 {
                        let cx = rect[0] + rect[2] / 2.0;
                        let cy = rect[1] + rect[3] / 2.0;
                        let dx = (at[0] - cx) / rx;
                        let dy = (at[1] - cy) / ry;
                        if dx * dx + dy * dy <= 1.0 {
                            return Some(l.id);
                        }
                    }
                }
                if sh.path.subpaths.iter().any(|sp| point_in_subpath(sp, at)) {
                    return Some(l.id);
                }
            }
        }
    }
    None
}

pub fn path_selection_finish(app: &mut PhotocraftApp, start: [f64; 2], end: [f64; 2]) {
    let (dx, dy) = (end[0] - start[0], end[1] - start[1]);
    let tol = 6.0 / f64::from(app.point_zoom().max(0.01));
    if dx.abs() + dy.abs() < 0.5 {
        let Some(st) = app.session.active() else { return };
        if let Some(id) = pick_shape_at(&st.doc, start, tol) {
            let _ = app.run("layer.select", json!({"layer": id.0}));
        }
        return;
    }
    if target_path(app).is_none() {
        let Some(st) = app.session.active() else { return };
        if let Some(id) = pick_shape_at(&st.doc, start, tol) {
            let _ = app.run("layer.select", json!({"layer": id.0}));
        }
    }
    let Some((target, path)) = target_path(app) else { return };
    let _ = match target {
        PathTarget::Shape(id) => app.run("shape.edit", json!({"layer": id, "move": [dx.round(), dy.round()]})),
        PathTarget::VectorMask(id) => {
            let moved = path.transform(&Affine::translate(dx.round(), dy.round()));
            app.run("layer.vectorMask.edit", json!({"layer": id, "path": photocraft_engine::vector_cmds::path_json(&moved)}))
        }
        PathTarget::Work => {
            let moved = path.transform(&Affine::translate(dx.round(), dy.round()));
            app.run("path.set", json!({"name": "work", "path": photocraft_engine::vector_cmds::path_json(&moved)}))
        }
    };
}
