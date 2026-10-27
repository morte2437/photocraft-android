use super::params::{
    f64p, fill_json, layer_id, live_from_params, live_json, nums, op_from, parse_fill, parse_path, parse_stroke, path_json, radii, stroke_json,
};
use super::vector_mask::with;
use super::*;

fn active_shape(s: &Session) -> std::result::Result<&ShapeLayer, String> {
    let d = s.active().ok_or("no document open")?;
    let id = d.active_layer.ok_or("no active layer")?;
    let layer = d.doc.layer(id).ok_or("no active layer")?;
    match &layer.content {
        LayerContent::Shape(shape) => Ok(shape),
        _ => Err("active layer is not a shape layer".into()),
    }
}

pub(super) fn has_shape_fill(s: &Session) -> std::result::Result<(), String> {
    active_shape(s)?.fill.as_ref().map(|_| ()).ok_or_else(|| "shape has no fill".into())
}

pub(super) fn has_shape_stroke(s: &Session) -> std::result::Result<(), String> {
    active_shape(s)?.stroke.as_ref().map(|_| ()).ok_or_else(|| "shape has no stroke".into())
}

pub(super) fn can_paste_shape_fill(s: &Session) -> std::result::Result<(), String> {
    active_shape(s)?;
    s.path_fill_clipboard.as_ref().map(|_| ()).ok_or_else(|| "no shape fill copied".into())
}

pub(super) fn can_paste_shape_stroke(s: &Session) -> std::result::Result<(), String> {
    active_shape(s)?;
    s.path_stroke_clipboard.as_ref().map(|_| ()).ok_or_else(|| "no shape stroke copied".into())
}

pub(super) fn copy_shape_fill(s: &mut Session, _p: &Value) -> Result<Value> {
    let fill = active_shape(s).map_err(EngineError::Other)?.fill.clone().ok_or_else(|| EngineError::Other("shape has no fill".into()))?;
    s.path_fill_clipboard = Some(fill);
    Ok(Value::Null)
}

pub(super) fn copy_shape_stroke(s: &mut Session, _p: &Value) -> Result<Value> {
    let stroke = active_shape(s).map_err(EngineError::Other)?.stroke.clone().ok_or_else(|| EngineError::Other("shape has no stroke".into()))?;
    s.path_stroke_clipboard = Some(stroke);
    Ok(Value::Null)
}

pub(super) fn paste_shape_fill(s: &mut Session, p: &Value) -> Result<Value> {
    let fill = s.path_fill_clipboard.clone().ok_or_else(|| EngineError::Other("no shape fill copied".into()))?;
    let id = layer_id(s, p)?;
    with_shape(s, id, "Paste Fill", |shape, _| {
        shape.fill = Some(fill);
        Ok(())
    })?;
    shape_info(s, id)
}

pub(super) fn paste_shape_stroke(s: &mut Session, p: &Value) -> Result<Value> {
    let stroke = s.path_stroke_clipboard.clone().ok_or_else(|| EngineError::Other("no shape stroke copied".into()))?;
    let id = layer_id(s, p)?;
    with_shape(s, id, "Paste Complete Stroke", |shape, _| {
        shape.stroke = Some(stroke);
        Ok(())
    })?;
    shape_info(s, id)
}

/// The path `shape.create` draws for live-shape params `p` (the Shape tools preview it while dragging).
pub fn shape_path(p: &Value) -> Result<Path> {
    let kind = p.get("kind").and_then(Value::as_str).unwrap_or("rect");
    Ok(vector::shapes::live_path(&live_from_params("shape.create", kind, p, None)?))
}

fn live_kind(l: &LiveShape) -> &'static str {
    match l {
        LiveShape::Rect { radii, .. } if radii.iter().any(|r| *r > 0.0) => "roundedRect",
        LiveShape::Rect { .. } => "rect",
        LiveShape::Ellipse { .. } => "ellipse",
        LiveShape::Polygon { star_ratio, .. } if *star_ratio < 1.0 => "star",
        LiveShape::Polygon { .. } => "polygon",
        LiveShape::Line { .. } => "line",
    }
}

/// Re-renders a shape layer's cache for `doc` (canvas-clipped, document pixel format).
pub fn refresh_shape(doc: &Document, sh: &mut ShapeLayer) {
    sh.cache = Some(vector::render_shape(sh, doc.pixel_format(), doc.bounds()));
}

/// A staged stroke edit rendered with the same validation and cache refresh as `shape.edit`.
/// The caller owns this snapshot; the session and its history remain untouched.
pub fn preview_shape_stroke(doc: &Document, id: LayerId, params: &Value) -> Result<Document> {
    let mut result = doc.clone();
    let layer = result.layer_mut(id).ok_or(EngineError::NoLayer(id))?;
    let LayerContent::Shape(shape) = &mut layer.content else {
        return Err(EngineError::Other("preview target is not a shape layer".into()));
    };
    shape.stroke = parse_stroke(params, shape.stroke.clone(), Color::BLACK).map_err(|e| bad("shape.edit", e))?;
    vector::flatten::validate_shape(&shape.path, shape.stroke.as_ref()).map_err(|e| bad("shape.edit", e))?;
    refresh_shape(doc, shape);
    Ok(result)
}

/// Moves a layer's vector content by `(dx, dy)`: shape paths (re-rendered) and linked vector
/// masks, recursing into groups. For the Move tool / `layer.move`.
pub fn translate_vectors(doc: &Document, l: &mut Layer, dx: f64, dy: f64) {
    let a = Affine::translate(dx, dy);
    if let Some(vm) = l.vector_mask.as_mut()
        && vm.linked
    {
        vm.path = vm.path.transform(&a);
    }
    match &mut l.content {
        LayerContent::Shape(sh) => shift_shape(doc, sh, dx, dy),
        LayerContent::Group(g) => {
            for c in &mut g.children {
                translate_vectors(doc, c, dx, dy);
            }
        }
        _ => {}
    }
}

/// Moves a shape by `(dx, dy)`. A whole-pixel move shifts the rendered pixels as they are, so a
/// shape from a PSD keeps Photoshop's anti-aliasing (as moved type does). The render is cut at
/// the canvas, so a shape reaching past it renders again, as does a fractional move.
pub(crate) fn shift_shape(doc: &Document, sh: &mut ShapeLayer, dx: f64, dy: f64) {
    transform_shape(sh, &Affine::translate(dx, dy));
    let whole = |v: f64| (v.fract() == 0.0 && v.abs() <= f64::from(i32::MAX)).then_some(v as i32);
    let canvas = doc.bounds();
    let inside = sh.cache.as_ref().is_some_and(|c| {
        let b = photocraft_compose::bounds::content_bounds(c);
        // An empty cache can be an off-canvas shape moving back into view.
        !b.is_empty() && b.x0 > canvas.x0 && b.y0 > canvas.y0 && b.x1 < canvas.x1 && b.y1 < canvas.y1
    });
    let stroke_pad = sh.stroke.as_ref().filter(|s| s.opacity > 0.0).map_or(0.0, |s| f64::from(s.width.ceil()));
    let stays_inside = sh.path.control_bounds().is_some_and(|(x0, y0, x1, y1)| {
        (x0 - stroke_pad) > canvas.x0 as f64
            && (y0 - stroke_pad) > canvas.y0 as f64
            && (x1 + stroke_pad) < canvas.x1 as f64
            && (y1 + stroke_pad) < canvas.y1 as f64
    });
    match (&mut sh.cache, whole(dx), whole(dy)) {
        (Some(c), Some(dx), Some(dy)) if inside && stays_inside => *c = crate::layer_multi_cmds::shift_surface(c, dx, dy),
        _ => refresh_shape(doc, sh),
    }
}

/// Applies an affine transform to a shape's path, keeping the live shape when the transform
/// is a translation plus axis-aligned scale (otherwise the shape stops being parametric).
pub fn transform_shape(sh: &mut ShapeLayer, a: &Affine) {
    sh.path = sh.path.transform(a);
    let [m0, m1, m2, m3, tx, ty] = a.m;
    let axis_aligned = m1.abs() < 1e-12 && m2.abs() < 1e-12 && m0 > 0.0 && m3 > 0.0;
    sh.live = match (sh.live.take(), axis_aligned) {
        (Some(l), true) => {
            let r = |r: [f64; 4]| [r[0] * m0 + tx, r[1] * m3 + ty, r[2] * m0, r[3] * m3];
            let pt = |p: [f64; 2]| [p[0] * m0 + tx, p[1] * m3 + ty];
            Some(match l {
                LiveShape::Rect { rect, radii } => {
                    let k = m0.min(m3);
                    LiveShape::Rect { rect: r(rect), radii: radii.map(|v| v * k) }
                }
                LiveShape::Ellipse { rect } => LiveShape::Ellipse { rect: r(rect) },
                LiveShape::Polygon { rect, sides, star_ratio, corner_radius } => {
                    LiveShape::Polygon { rect: r(rect), sides, star_ratio, corner_radius: corner_radius * m0.min(m3) }
                }
                LiveShape::Line { from, to, weight } => {
                    // Live-shape edits regenerate the line's filled bar from its parameters.
                    // Keep the weight in sync with a positive scale of the path, or an
                    // unchanged endpoint edit unexpectedly restores the old thickness (#3063).
                    // A line's width scales perpendicular to its direction.
                    let (dx, dy) = (to[0] - from[0], to[1] - from[1]);
                    let old_length = dx.hypot(dy);
                    let new_length = (dx * m0).hypot(dy * m3);
                    let width_scale = if old_length > 0.0 && new_length > 0.0 { m0 * m3 * old_length / new_length } else { m0.min(m3) };
                    LiveShape::Line { from: pt(from), to: pt(to), weight: weight * width_scale }
                }
            })
        }
        // Flips and 90° turns keep rectangles, ellipses and lines live (Image Rotation): the new
        // live box is the mapped box. Polygons/stars would change orientation, so they don't.
        (Some(l), false) if (m1.abs() < 1e-12 && m2.abs() < 1e-12) || (m0.abs() < 1e-12 && m3.abs() < 1e-12) => {
            let pt = |p: [f64; 2]| [m0 * p[0] + m2 * p[1] + tx, m1 * p[0] + m3 * p[1] + ty];
            let r = |r: [f64; 4]| {
                let (p, q) = (pt([r[0], r[1]]), pt([r[0] + r[2], r[1] + r[3]]));
                [p[0].min(q[0]), p[1].min(q[1]), (q[0] - p[0]).abs(), (q[1] - p[1]).abs()]
            };
            let k = (m0.abs() + m1.abs()).min(m2.abs() + m3.abs());
            match l {
                LiveShape::Rect { rect, radii } if radii.iter().all(|v| (v - radii[0]).abs() < 1e-9) => {
                    Some(LiveShape::Rect { rect: r(rect), radii: radii.map(|v| v * k) })
                }
                LiveShape::Ellipse { rect } => Some(LiveShape::Ellipse { rect: r(rect) }),
                LiveShape::Line { from, to, weight } => Some(LiveShape::Line { from: pt(from), to: pt(to), weight: weight * k }),
                _ => None,
            }
        }
        _ => None,
    };
    sh.psd_raw = None;
}

/// Is every anchor and control point of `path` a finite point?
fn finite_path(path: &Path) -> bool {
    path.subpaths.iter().flat_map(|s| &s.knots).all(|k| [k.anchor, k.in_ctrl, k.out_ctrl].iter().all(|p| p.x.is_finite() && p.y.is_finite()))
}

/// `path` through Distort / Perspective (`h`, a projective map of its points: exact for straight
/// segments, the end tangents of curves exact) or, with `warp`, Warp (split where the warp bends
/// it). Refused when the map sends a point to infinity (past a perspective's horizon).
pub(crate) fn map_path(path: &Path, h: Option<&photocraft_algo::transform::Homography>, warp: Option<&photocraft_geom::warp::Warp>) -> Result<Path> {
    let out = match (h, warp) {
        (_, Some(w)) => vector::map::map_path(
            path,
            |p| {
                let (x, y) = w.map(p.x, p.y);
                Point::new(x, y)
            },
            0.1,
        ),
        (Some(h), None) => vector::map::map_points(path, |p| {
            let (x, y) = h.apply(p.x, p.y);
            Point::new(x, y)
        }),
        (None, None) => path.clone(),
    };
    if !finite_path(&out) {
        return Err(EngineError::Other("the distortion sends part of the path out of bounds".into()));
    }
    Ok(out)
}

/// Distort, Perspective or Warp of a shape layer (#3009): the shape stays a shape, its path
/// mapped ([`map_path`]), as in Photoshop. A live rectangle, ellipse, polygon or line becomes a
/// plain path, as its parameters can't describe the distorted outline.
pub(crate) fn map_shape(sh: &mut ShapeLayer, h: Option<&photocraft_algo::transform::Homography>, warp: Option<&photocraft_geom::warp::Warp>) -> Result<()> {
    sh.path = map_path(&sh.path, h, warp)?;
    sh.live = None;
    sh.psd_raw = None;
    Ok(())
}

pub(super) fn shape_info(s: &Session, id: LayerId) -> Result<Value> {
    let d = s.active().ok_or(EngineError::NoDocument)?;
    let l = d.doc.layer(id).ok_or(EngineError::NoLayer(id))?;
    let LayerContent::Shape(sh) = &l.content else {
        return Err(EngineError::Other(format!("layer {} is {} {} layer, not a shape layer", id.0, l.content.article(), l.content.kind_name())));
    };
    let bounds = sh.cache.as_ref().map(|c| c.content_bounds()).filter(|r| !r.is_empty()).map(|r| json!([r.x0, r.y0, r.width(), r.height()]));
    Ok(json!({
        "layer": id.0,
        "name": l.name,
        "path": path_json(&sh.path),
        "fill": sh.fill.as_ref().map(fill_json),
        "stroke": sh.stroke.as_ref().map(stroke_json),
        "live": sh.live.as_ref().map(live_json),
        "kind": sh.live.as_ref().map_or("path", live_kind),
        "bounds": bounds,
    }))
}

pub(crate) fn with_shape<R>(s: &mut Session, id: LayerId, label: &str, f: impl FnOnce(&mut ShapeLayer, &mut Layer) -> Result<R>) -> Result<R> {
    if let Some(st) = s.active() {
        let locks = st.doc.effective_locks(id);
        if locks.all || locks.pixels {
            return Err(EngineError::Other("shape layer is locked".into()));
        }
    }
    s.edit(label, |doc, _| {
        let snapshot = doc.clone();
        let l = doc.layer_mut(id).ok_or(EngineError::NoLayer(id))?;
        if !matches!(l.content, LayerContent::Shape(_)) {
            return Err(EngineError::Other(format!("layer {} is {} {} layer, not a shape layer", id.0, l.content.article(), l.content.kind_name())));
        }
        let LayerContent::Shape(mut sh) = std::mem::replace(&mut l.content, LayerContent::Fill(Fill::Solid(Color::BLACK))) else {
            return Err(EngineError::Other(format!("layer {} is not a shape layer", id.0)));
        };
        let r = f(&mut sh, l);
        if r.is_ok() {
            refresh_shape(&snapshot, &mut sh);
        }
        l.content = LayerContent::Shape(sh);
        r
    })
}

fn default_name(kind: &str) -> &'static str {
    match kind {
        "rect" | "rectangle" => "Rectangle",
        "roundedRect" | "roundRect" => "Rounded Rectangle",
        "ellipse" => "Ellipse",
        "polygon" => "Polygon",
        "star" => "Star",
        "line" => "Line",
        _ => "Shape",
    }
}

pub(super) fn shape_create(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "shape.create";
    let kind = p.get("kind").and_then(Value::as_str).unwrap_or("rect");
    let (path, live) = if kind == "path" {
        let v = p.get("path").ok_or_else(|| bad(CMD, "kind \"path\" needs `path`"))?;
        (parse_path(v).map_err(|e| bad(CMD, e))?, None)
    } else {
        let l = live_from_params(CMD, kind, p, None)?;
        (vector::shapes::live_path(&l), Some(l))
    };
    let fg = s.tools.foreground;
    let fgc = Color::rgba(fg[0], fg[1], fg[2], fg[3]);
    let fill = match p.get("fill") {
        Some(v) => parse_fill(v).map_err(|e| bad(CMD, e))?,
        None => Some(Fill::Solid(fgc)),
    };
    let stroke = match p.get("stroke") {
        Some(v) => parse_stroke(v, None, fgc).map_err(|e| bad(CMD, e))?,
        None => None,
    };
    vector::flatten::validate_shape(&path, stroke.as_ref()).map_err(|e| bad(CMD, e))?;
    // Add to an existing shape layer with a path operation (Shape tool in combine/subtract… mode).
    if let Some(target) = p.get("addTo").and_then(Value::as_u64) {
        let op = p.get("op").and_then(Value::as_str).map_or(Some(PathOp::Combine), op_from).ok_or_else(|| bad(CMD, "unknown `op`"))?;
        let id = LayerId(target);
        with_shape(s, id, "Add Shape", |sh, _| {
            for mut sp in path.subpaths {
                sp.op = op;
                sh.path.subpaths.push(sp);
            }
            sh.live = None;
            sh.psd_raw = None;
            vector::flatten::validate_shape(&sh.path, sh.stroke.as_ref()).map_err(|e| bad(CMD, e))?;
            Ok(())
        })?;
        return shape_info(s, id);
    }
    let name = p.get("name").and_then(Value::as_str).map(str::to_string);
    let mut sh = ShapeLayer { path, fill, stroke, live, ..Default::default() };
    let id = s.edit("New Shape Layer", |doc, active| {
        refresh_shape(doc, &mut sh);
        let name = name.unwrap_or_else(|| doc.next_layer_name(default_name(kind)));
        let id = doc.insert_above(*active, Layer::new(name, LayerContent::Shape(sh)));
        *active = Some(id);
        Ok(id)
    })?;
    shape_info(s, id)
}

pub(super) fn shape_edit(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "shape.edit";
    let id = layer_id(s, p)?;
    let fg = s.tools.foreground;
    let fgc = Color::rgba(fg[0], fg[1], fg[2], fg[3]);
    let name = p.get("name").and_then(Value::as_str).map(str::to_string);
    with_shape(s, id, "Edit Shape", |sh, layer| {
        if let Some(v) = p.get("path") {
            sh.path = parse_path(v).map_err(|e| bad(CMD, e))?;
            sh.live = None;
            sh.psd_raw = None;
        }
        let geom_keys = ["kind", "rect", "radii", "radius", "sides", "starRatio", "cornerRadius", "from", "to", "weight"];
        if geom_keys.iter().any(|k| p.get(*k).is_some()) {
            let kind = p.get("kind").and_then(Value::as_str).map(str::to_string).or_else(|| sh.live.as_ref().map(|l| live_kind(l).to_string()));
            let kind = kind.ok_or_else(|| bad(CMD, "this shape has no live-shape parameters; pass `kind` or `path`"))?;
            // Explicit radii on a plain rectangle keep it a (now rounded) rectangle.
            let kind = if kind == "rect" && radii(p).is_some_and(|r| r.iter().any(|v| *v > 0.0)) { "roundedRect".to_string() } else { kind };
            let l = live_from_params(CMD, &kind, p, sh.live.as_ref())?;
            // A transformed live shape's path may carry nonuniform corner scaling
            // that its scalar radii cannot represent. Re-entering unchanged values
            // must not regenerate the outline and silently discard that geometry.
            if sh.live.as_ref() != Some(&l) {
                sh.path = vector::shapes::live_path(&l);
                sh.live = Some(l);
                sh.psd_raw = None;
            }
        }
        if let Some(m) = nums::<6>(p, "transform") {
            transform_shape(sh, &Affine { m });
        }
        if let Some([dx, dy]) = nums::<2>(p, "move") {
            transform_shape(sh, &Affine::translate(dx, dy));
        }
        if let Some(op) = p.get("op").and_then(Value::as_str) {
            // Operation of the selected subpath(s) (`subpath`: index, default all but the first).
            let op = op_from(op).ok_or_else(|| bad(CMD, format!("unknown op `{op}`")))?;
            match p.get("subpath").and_then(Value::as_u64) {
                Some(i) => sh.path.subpaths.get_mut(i as usize).ok_or_else(|| bad(CMD, "no such subpath"))?.op = op,
                None => sh.path.subpaths.iter_mut().skip(1).for_each(|sp| sp.op = op),
            }
            sh.live = None;
        }
        if let Some(r) = p.get("fillRule").and_then(Value::as_str) {
            sh.path.fill_rule = if r.eq_ignore_ascii_case("evenodd") { FillRule::EvenOdd } else { FillRule::NonZero };
        }
        if let Some(v) = p.get("fill") {
            sh.fill = parse_fill(v).map_err(|e| bad(CMD, e))?;
        }
        if let Some(v) = p.get("stroke") {
            sh.stroke = parse_stroke(v, sh.stroke.clone(), fgc).map_err(|e| bad(CMD, e))?;
        }
        if let Some(n) = name {
            layer.name = n;
        }
        vector::flatten::validate_shape(&sh.path, sh.stroke.as_ref()).map_err(|e| bad(CMD, e))?;
        Ok(())
    })?;
    shape_info(s, id)
}

pub(super) fn shape_rasterize(s: &mut Session, p: &Value) -> Result<Value> {
    let id = layer_id(s, p)?;
    s.edit("Rasterize Shape", |doc, _| {
        let snapshot = doc.clone();
        let l = doc.layer_mut(id).ok_or(EngineError::NoLayer(id))?;
        let LayerContent::Shape(sh) = &mut l.content else {
            return Err(EngineError::Other(format!("layer {} is not a shape layer", id.0)));
        };
        if sh.cache.is_none() {
            refresh_shape(&snapshot, sh);
        }
        let fmt = snapshot.pixel_format();
        let surface = sh.cache.take().unwrap_or_else(|| photocraft_raster::Surface::new(fmt));
        let surface = if surface.format() == fmt { surface } else { surface.convert(fmt) };
        l.content = LayerContent::Raster(surface);
        l.psd_blocks.retain(|(k, _)| !SHAPE_BLOCKS.contains(&k));
        Ok(())
    })?;
    Ok(json!({ "layer": id.0 }))
}

/// Traces the exact coverage of the shape's path back into plain outlines.
pub(super) fn merge_components(s: &mut Session, p: &Value) -> Result<Value> {
    let tol = f64p(p, "tolerance").unwrap_or(0.1).clamp(0.05, 10.0);
    if let Some(ids) = combine_targets(s, p, &["layer"])? {
        return merge_shape_layers(s, &ids, None, "Merge Shape Components", Some(tol));
    }
    let id = layer_id(s, p)?;
    with_shape(s, id, "Merge Shape Components", |sh, _| bake_components(sh, tol))?;
    shape_info(s, id)
}

/// Bakes `sh`'s path operations into plain combined outlines, traced from the exact coverage.
fn bake_components(sh: &mut ShapeLayer, tol: f64) -> Result<()> {
    // Trace at 4× resolution so the fitted outline follows the anti-aliased edge closely.
    const K: f64 = 4.0;
    let up = sh.path.transform(&Affine::scale(K));
    let r = vector::fill_rasterizer(&up, vector::DEFAULT_TOLERANCE);
    let Some(b) = r.pixel_bounds() else {
        sh.path.subpaths.clear();
        return Ok(());
    };
    let b = b.inflate(2);
    if u64::from(b.width()) * u64::from(b.height()) > 400_000_000 {
        return Err(EngineError::Other("shape too large to merge".into()));
    }
    let cov = r.render(b);
    let traced = vector::trace::trace_mask(&cov, b, 0.0, tol * K);
    sh.path = traced.transform(&Affine::scale(1.0 / K));
    sh.live = None;
    sh.psd_raw = None;
    Ok(())
}

/// The shape layers Layer › Combine Shapes merges into one: with none of the `narrow` params
/// given and two or more shape layers selected, those layers bottom-to-top (other selected layers
/// are left alone). `None` keeps the single-layer behaviour (acting on one layer's components).
fn combine_targets(s: &Session, p: &Value, narrow: &[&str]) -> Result<Option<Vec<LayerId>>> {
    if narrow.iter().any(|k| p.get(*k).is_some()) {
        return Ok(None);
    }
    let d = s.active().ok_or(EngineError::NoDocument)?;
    let sel = d.selected_layers();
    let ids = crate::layer_multi_cmds::top_level(&d.doc, &sel);
    let ids: Vec<LayerId> = ids.into_iter().filter(|id| d.doc.layer(*id).is_some_and(|l| matches!(l.content, LayerContent::Shape(_)))).collect();
    Ok((ids.len() > 1).then_some(ids))
}

/// Layer › Combine Shapes with several shape layers selected: Photoshop merges them into the
/// top-most one. The bottom layer's components keep their operations; every component of the
/// layers above takes `op` (front shapes subtract, intersect… in stacking order), or, with no
/// `op` (Merge Shape Components), keeps its own with each layer's first component combined, and
/// the result is then baked with tolerance `bake`. The other layers are removed; one history step.
fn merge_shape_layers(s: &mut Session, ids: &[LayerId], op: Option<PathOp>, label: &str, bake: Option<f64>) -> Result<Value> {
    let (&keep, _) = ids.split_last().ok_or_else(|| EngineError::Other("Combine Shapes needs two or more shape layers".into()))?;
    if let Some(st) = s.active() {
        for id in ids {
            let locks = st.doc.effective_locks(*id);
            if locks.all || locks.pixels {
                return Err(EngineError::Other("a selected shape layer is locked".into()));
            }
        }
    }
    s.edit(label, |doc, active| {
        let mut path: Option<Path> = None;
        for id in ids {
            let l = doc.layer(*id).ok_or(EngineError::NoLayer(*id))?;
            let LayerContent::Shape(sh) = &l.content else {
                return Err(EngineError::Other(format!("layer {} is not a shape layer", id.0)));
            };
            let Some(acc) = path.as_mut() else {
                path = Some(sh.path.clone());
                continue;
            };
            for (j, sp) in sh.path.subpaths.iter().enumerate() {
                let mut sp = sp.clone();
                sp.op = match op {
                    Some(op) => op,
                    None if j == 0 => PathOp::Combine,
                    None => sp.op,
                };
                acc.subpaths.push(sp);
            }
        }
        let path = path.ok_or_else(|| EngineError::Other("Combine Shapes needs two or more shape layers".into()))?;
        let snapshot = doc.clone();
        let l = doc.layer_mut(keep).ok_or(EngineError::NoLayer(keep))?;
        let LayerContent::Shape(sh) = &mut l.content else {
            return Err(EngineError::Other(format!("layer {} is not a shape layer", keep.0)));
        };
        let mut merged = ShapeLayer { path, live: None, psd_raw: None, cache: None, ..sh.clone() };
        if let Some(tol) = bake {
            bake_components(&mut merged, tol)?;
        }
        vector::flatten::validate_shape(&merged.path, merged.stroke.as_ref()).map_err(|e| bad(label, e))?;
        refresh_shape(&snapshot, &mut merged);
        *sh = merged;
        for id in ids.iter().filter(|id| **id != keep) {
            doc.remove(*id);
        }
        *active = Some(keep);
        Ok(())
    })?;
    crate::layer_multi_cmds::reselect(s, vec![keep], Some(keep));
    shape_info(s, keep)
}

/// Layer › Combine Shapes › Unite / Subtract Front Shape / Intersect / Exclude: merges the
/// selected shape layers with `op` ([`merge_shape_layers`]), or with one layer (or an explicit
/// `layer`/`subpath`) sets the operation of that layer's components (`shape.edit`'s `op`).
pub(super) fn combine_shapes(s: &mut Session, p: &Value, op: &str, label: &str) -> Result<Value> {
    if let Some(ids) = combine_targets(s, p, &["layer", "subpath"])? {
        let op = op_from(op).ok_or_else(|| EngineError::Other(format!("unknown op `{op}`")))?;
        return merge_shape_layers(s, &ids, Some(op), label, None);
    }
    shape_edit(s, &with(p, "op", json!(op)))
}

pub(super) fn selection_to_shape(s: &mut Session, p: &Value) -> Result<Value> {
    let tol = f64p(p, "tolerance").or_else(|| f64p(p, "simplifyTolerance")).unwrap_or(2.0).clamp(0.5, 10.0);
    let d = s.active().ok_or(EngineError::NoDocument)?;
    let area = d.doc.bounds();
    let mask = sel::mask_from_surface(d.doc.selection.as_ref(), area);
    let path = vector::trace::trace_mask(&mask, area, 0.0, tol);
    let mut q = json!({ "kind": "path", "path": path_json(&path) });
    for k in ["fill", "name", "stroke"] {
        if let Some(v) = p.get(k) {
            q[k] = v.clone();
        }
    }
    shape_create(s, &q)
}
