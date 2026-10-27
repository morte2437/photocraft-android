use super::params::{color, f64p, layer_id, op_from, parse_path, path_json};
use super::shape::{map_path, map_shape, transform_shape, with_shape};
use super::*;

// ---------------------------------------------------------------------------
// Document paths
// ---------------------------------------------------------------------------

pub(crate) fn is_work(name: Option<&str>) -> bool {
    name.is_none_or(|n| n.is_empty() || n.eq_ignore_ascii_case("work") || n == "Work Path")
}

/// The path a command targets: `"work"`/absent → the work path (falling back to the active
/// layer's shape path or vector mask when there is none), `"layer"` → the active layer's,
/// otherwise a saved path by name.
pub(super) fn resolve_path(s: &Session, name: Option<&str>) -> Result<Path> {
    let d = s.active().ok_or(EngineError::NoDocument)?;
    let layer_path = || -> Option<Path> {
        let l = d.doc.layer(d.active_layer?)?;
        match &l.content {
            LayerContent::Shape(sh) => Some(sh.path.clone()),
            _ => l.vector_mask.as_ref().map(|v| v.path.clone()),
        }
    };
    if name == Some("layer") {
        return layer_path().ok_or_else(|| EngineError::Other("active layer has no shape path or vector mask".into()));
    }
    if is_work(name) {
        return d.doc.work_path.clone().or_else(layer_path).ok_or_else(|| EngineError::Other("no work path".into()));
    }
    let n = name.unwrap_or_default();
    d.doc.paths.iter().find(|p| p.name == n).map(|p| p.path.clone()).ok_or_else(|| EngineError::Other(format!("no path named \"{n}\"")))
}

/// The path an edit targets, except a shape layer's (edit those with [`with_shape`], which
/// re-renders them): `layer`'s vector mask when given, else the work path or a saved path.
pub(crate) fn path_mut<'a>(doc: &'a mut Document, name: &str, layer: Option<LayerId>) -> Result<&'a mut Path> {
    if let Some(id) = layer {
        let l = doc.layer_mut(id).ok_or(EngineError::NoLayer(id))?;
        return l.vector_mask.as_mut().map(|vm| &mut vm.path).ok_or_else(|| EngineError::Other("active layer has no shape path or vector mask".into()));
    }
    if is_work(Some(name)) {
        return doc.work_path.as_mut().ok_or_else(|| EngineError::Other("no work path".into()));
    }
    doc.paths.iter_mut().find(|q| q.name == name).map(|q| &mut q.path).ok_or_else(|| EngineError::Other(format!("no path named \"{name}\"")))
}

pub(super) fn paths_list(s: &Session) -> Result<Value> {
    let d = s.active().ok_or(EngineError::NoDocument)?;
    let doc = &d.doc;
    let knots = |p: &Path| p.subpaths.iter().map(|s| s.knots.len()).sum::<usize>();
    let layer = d.active_layer.and_then(|id| doc.layer(id)).and_then(|l| match &l.content {
        LayerContent::Shape(sh) => Some(json!({ "kind": "shape", "layer": l.id.0, "name": format!("{} Shape Path", l.name), "knots": knots(&sh.path) })),
        _ => {
            l.vector_mask.as_ref().map(|v| json!({ "kind": "vectorMask", "layer": l.id.0, "name": format!("{} Vector Mask", l.name), "knots": knots(&v.path) }))
        }
    });
    Ok(json!({
        "paths": doc.paths.iter().map(|p| json!({ "name": p.name, "knots": knots(&p.path), "subpaths": p.path.subpaths.len() })).collect::<Vec<_>>(),
        "workPath": doc.work_path.as_ref().map(|p| json!({ "knots": knots(p), "subpaths": p.subpaths.len() })),
        "clippingPath": doc.clipping_path.as_ref().map(|c| json!({ "name": c.name, "flatness": c.flatness })),
        "layerPath": layer,
    }))
}

pub(super) fn path_set(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "path.set";
    let path = parse_path(p.get("path").ok_or_else(|| bad(CMD, "missing `path`"))?).map_err(|e| bad(CMD, e))?;
    let name = p.get("name").and_then(Value::as_str).map(str::to_string);
    let append = p.get("op").and_then(Value::as_str).map(|o| op_from(o).ok_or_else(|| bad(CMD, format!("unknown op `{o}`")))).transpose()?;
    s.edit(if is_work(name.as_deref()) { "Work Path" } else { "Save Path" }, |doc, _| {
        let merge = |old: Option<&Path>, new: Path| match (old, append) {
            (Some(o), Some(op)) => {
                let mut o = o.clone();
                o.subpaths.extend(new.subpaths.into_iter().map(|mut sp| {
                    sp.op = op;
                    sp
                }));
                o
            }
            _ => new,
        };
        if is_work(name.as_deref()) {
            doc.work_path = Some(merge(doc.work_path.as_ref(), path));
        } else {
            let n = name.clone().unwrap_or_default();
            match doc.paths.iter_mut().find(|q| q.name == n) {
                Some(q) => {
                    q.path = merge(Some(&q.path), path);
                }
                None => doc.paths.push(NamedPath { name: n, path, psd_raw: None }),
            }
        }
        Ok(())
    })?;
    let path = resolve_path(s, name.as_deref())?;
    Ok(json!({ "name": name.unwrap_or_else(|| "work".into()), "path": path_json(&path) }))
}

pub(super) fn path_delete(s: &mut Session, p: &Value) -> Result<Value> {
    let name = p.get("name").and_then(Value::as_str).map(str::to_string);
    s.edit("Delete Path", |doc, _| {
        if is_work(name.as_deref()) {
            return doc.work_path.take().map(|_| ()).ok_or_else(|| EngineError::Other("no work path".into()));
        }
        let n = name.clone().unwrap_or_default();
        let before = doc.paths.len();
        doc.paths.retain(|q| q.name != n);
        if doc.paths.len() == before {
            return Err(EngineError::Other(format!("no path named \"{n}\"")));
        }
        if doc.clipping_path.as_ref().is_some_and(|c| c.name == n) {
            doc.clipping_path = None;
        }
        Ok(())
    })?;
    Ok(Value::Null)
}

pub(super) fn clipping_path_set(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "path.clippingPath.set";
    let name = p.get("name").and_then(Value::as_str).filter(|name| !name.is_empty()).ok_or_else(|| bad(CMD, "missing saved path `name`"))?.to_owned();
    let flatness = match p.get("flatness") {
        Some(value) => {
            value.as_f64().filter(|value| value.is_finite() && (0.0..=100.0).contains(value)).ok_or_else(|| bad(CMD, "`flatness` must be between 0 and 100"))?
                as f32
        }
        None => 0.0,
    };
    s.edit("Clipping Path", |doc, _| {
        if !doc.paths.iter().any(|path| path.name == name) {
            return Err(EngineError::Other(format!("no saved path named \"{name}\"")));
        }
        doc.clipping_path = Some(ClippingPath { name: name.clone(), flatness });
        Ok(())
    })?;
    Ok(json!({ "name": name, "flatness": flatness }))
}

pub(super) fn clipping_path_clear(s: &mut Session, _p: &Value) -> Result<Value> {
    s.edit("Clear Clipping Path", |doc, _| {
        doc.clipping_path.take().ok_or_else(|| EngineError::Other("no clipping path".into()))?;
        Ok(())
    })?;
    Ok(Value::Null)
}

/// The path `path.transform` edits: `layer`'s shape path or vector mask, else the work path or a
/// saved path.
fn transform_target(s: &Session, name: &str, layer: Option<LayerId>) -> Result<Path> {
    let Some(id) = layer else { return resolve_path(s, Some(name)) };
    let d = s.active().ok_or(EngineError::NoDocument)?;
    let l = d.doc.layer(id).ok_or(EngineError::NoLayer(id))?;
    match &l.content {
        LayerContent::Shape(sh) => Ok(sh.path.clone()),
        _ => l.vector_mask.as_ref().map(|v| v.path.clone()).ok_or_else(|| EngineError::Other("target layer has no shape path or vector mask".into())),
    }
}

/// `n` numbers at `key` (`[x0, y0, x1, y1]` or `[[x, y] × 4]`, flattened): finite, magnitude at
/// most 1000000. `None` when `key` is absent.
fn coords(p: &Value, key: &str, n: usize) -> Result<Option<Vec<f64>>> {
    const CMD: &str = "path.transform";
    let Some(v) = p.get(key) else { return Ok(None) };
    let flat: Vec<&Value> = match v.as_array() {
        Some(a) if a.iter().all(Value::is_array) => a.iter().filter_map(Value::as_array).flatten().collect(),
        Some(a) => a.iter().collect(),
        None => Vec::new(),
    };
    let nums: Vec<f64> = flat.iter().filter_map(|v| v.as_f64()).filter(|x| x.is_finite() && x.abs() <= 1_000_000.0).collect();
    if flat.len() != n || nums.len() != n {
        return Err(bad(CMD, format!("`{key}` must hold {n} finite numbers with magnitude at most 1000000")));
    }
    Ok(Some(nums))
}

/// How `path.transform` moves the path.
enum PathMap {
    Affine(Affine),
    /// Distort / Perspective: the box's corners go to a quad.
    Projective(photocraft_algo::transform::Homography),
    Warp(Box<photocraft_geom::warp::Warp>),
}

pub(super) fn path_transform(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "path.transform";
    let name = p.get("name").and_then(Value::as_str).unwrap_or("work").to_owned();
    let layer_id = if name == "layer" { Some(layer_id(s, p)?) } else { None };
    // The box Distort, Perspective and Warp frame: `rect`, else the path's control bounds.
    let frame = |s: &Session| -> Result<[f64; 4]> {
        let r = match coords(p, "rect", 4)? {
            Some(r) => [r[0], r[1], r[2], r[3]],
            None => {
                let (x0, y0, x1, y1) = transform_target(s, &name, layer_id)?.control_bounds().ok_or_else(|| bad(CMD, "path has no points to transform"))?;
                [x0, y0, x1, y1]
            }
        };
        if r[2] - r[0] <= 0.0 || r[3] - r[1] <= 0.0 {
            return Err(bad(CMD, "`rect` must have a positive width and height (a straight path needs a `rect`)"));
        }
        Ok(r)
    };
    let map = if let Some(values) = p.get("matrix") {
        let values = values.as_array().filter(|values| values.len() == 6).ok_or_else(|| bad(CMD, "`matrix` must contain six numbers"))?;
        let mut m = [0.0; 6];
        for (slot, value) in m.iter_mut().zip(values) {
            *slot = value
                .as_f64()
                .filter(|number| number.is_finite() && number.abs() <= 1_000_000.0)
                .ok_or_else(|| bad(CMD, "matrix entries must be finite numbers with magnitude at most 1000000"))?;
        }
        PathMap::Affine(Affine { m })
    } else if let Some(q) = coords(p, "quad", 8)? {
        // Distort and Perspective (#3027): where the frame's corners go, clockwise from top-left.
        let quad = [[q[0], q[1]], [q[2], q[3]], [q[4], q[5]], [q[6], q[7]]];
        let h = photocraft_algo::transform::Homography::rect_to_quad(frame(s)?, quad)
            .filter(|h| h.0.iter().all(|v| v.is_finite()) && h.inverse().is_some())
            .ok_or_else(|| bad(CMD, "degenerate quad"))?;
        let [a, b, c, d, e, f, g, hh, i] = h.0;
        if g.abs() < 1e-12 && hh.abs() < 1e-12 && i.abs() > 1e-12 {
            // A parallelogram is affine: a live shape stays live.
            PathMap::Affine(Affine { m: [a / i, d / i, b / i, e / i, c / i, f / i] })
        } else {
            PathMap::Projective(h)
        }
    } else if ["warp", "style", "mesh"].iter().any(|key| p.get(*key).is_some()) {
        let w = crate::warp_cmds::warp_from_params(CMD, p, frame(s)?)?;
        if [w.bend, w.h_distort, w.v_distort].iter().chain(w.bounds.iter()).any(|v| !v.is_finite() || v.abs() > 1_000_000.0) {
            return Err(bad(CMD, "warp values must be finite numbers with magnitude at most 1000000"));
        }
        PathMap::Warp(Box::new(w))
    } else {
        let number = |key: &str, default: f64| -> Result<f64> {
            match p.get(key) {
                Some(value) => value
                    .as_f64()
                    .filter(|n| n.is_finite() && n.abs() <= 1_000_000.0)
                    .ok_or_else(|| bad(CMD, format!("`{key}` must be a finite number with magnitude at most 1000000"))),
                None => Ok(default),
            }
        };
        if !["translateX", "translateY", "scaleX", "scaleY", "angle"].iter().any(|key| p.get(*key).is_some()) {
            return Err(bad(CMD, "pass `matrix`, `quad`, a warp (`style`, `mesh` or `warp`) or translate/scale/angle parameters"));
        }
        let (tx, ty) = (number("translateX", 0.0)?, number("translateY", 0.0)?);
        let (sx, sy) = (number("scaleX", 1.0)?, number("scaleY", 1.0)?);
        let angle = number("angle", 0.0)?;
        // An explicit `layer` parameter determines both the transformed path and its pivot (#3062).
        let path = transform_target(s, &name, layer_id)?;
        let (x0, y0, x1, y1) = path.control_bounds().ok_or_else(|| bad(CMD, "path has no points to transform"))?;
        let (cx, cy) = ((x0 + x1) * 0.5, (y0 + y1) * 0.5);
        PathMap::Affine(
            Affine::translate(-cx, -cy)
                .then(&Affine { m: [sx, 0.0, 0.0, sy, 0.0, 0.0] })
                .then(&Affine::rotate(angle.to_radians()))
                .then(&Affine::translate(cx + tx, cy + ty)),
        )
    };
    let out = match &map {
        PathMap::Affine(a) => json!({ "name": name, "matrix": a.m }),
        PathMap::Projective(h) => json!({ "name": name, "homography": h.0 }),
        PathMap::Warp(_) => json!({ "name": name, "warp": true }),
    };
    if let Some(id) = layer_id
        && s.active().and_then(|d| d.doc.layer(id)).is_some_and(|layer| matches!(layer.content, LayerContent::Shape(_)))
    {
        with_shape(s, id, "Transform Path", |shape, _| {
            match &map {
                PathMap::Affine(a) => transform_shape(shape, a),
                PathMap::Projective(h) => map_shape(shape, Some(h), None)?,
                PathMap::Warp(w) => map_shape(shape, None, Some(w))?,
            }
            shape.psd_raw = None;
            Ok(())
        })?;
        return Ok(out);
    }
    s.edit("Transform Path", |doc, _| {
        let path = path_mut(doc, &name, layer_id)?;
        *path = match &map {
            PathMap::Affine(a) => path.transform(a),
            PathMap::Projective(h) => map_path(path, Some(h), None)?,
            PathMap::Warp(w) => map_path(path, None, Some(w))?,
        };
        Ok(())
    })?;
    Ok(out)
}

pub(super) fn path_rename(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "path.rename";
    let name = p.get("name").and_then(Value::as_str).map(str::to_string);
    let to = p.get("to").and_then(Value::as_str).filter(|t| !t.is_empty()).ok_or_else(|| bad(CMD, "missing `to`"))?.to_string();
    if is_work(Some(&to)) {
        return Err(bad(CMD, "`to` cannot name the work path"));
    }
    s.edit(if is_work(name.as_deref()) { "Save Path" } else { "Rename Path" }, |doc, _| {
        if doc.paths.iter().any(|q| q.name == to) {
            return Err(EngineError::Other(format!("a path named \"{to}\" already exists")));
        }
        if is_work(name.as_deref()) {
            // Photoshop: saving the work path turns it into a named path.
            let path = doc.work_path.take().ok_or_else(|| EngineError::Other("no work path".into()))?;
            doc.paths.push(NamedPath { name: to.clone(), path, psd_raw: None });
            return Ok(());
        }
        let n = name.clone().unwrap_or_default();
        let q = doc.paths.iter_mut().find(|q| q.name == n).ok_or_else(|| EngineError::Other(format!("no path named \"{n}\"")))?;
        q.name = to.clone();
        if let Some(c) = doc.clipping_path.as_mut()
            && c.name == n
        {
            c.name = to.clone();
        }
        Ok(())
    })?;
    Ok(json!({ "name": to }))
}

pub(super) fn path_to_selection(s: &mut Session, p: &Value) -> Result<Value> {
    let path = resolve_path(s, p.get("name").and_then(Value::as_str))?;
    // Validate before rasterizing or touching selection history. An unchecked feather value
    // could silently become zero (negative/string), or expand into an excessive blur.
    let feather = match p.get("feather") {
        None => 0.0,
        Some(v) => v
            .as_f64()
            .filter(|n| n.is_finite() && (0.0..=1000.0).contains(n))
            .ok_or_else(|| bad("path.toSelection", "`feather` must be a finite number within 0..1000"))? as f32,
    };
    let aa = match p.get("antiAlias") {
        None => true,
        Some(Value::Bool(v)) => *v,
        _ => return Err(bad("path.toSelection", "`antiAlias` must be a boolean")),
    };
    let mode = match p.get("mode") {
        None => SelectionMode::Replace,
        Some(Value::String(v)) if matches!(v.as_str(), "replace" | "add" | "subtract" | "intersect") => SelectionMode::parse(v),
        _ => return Err(bad("path.toSelection", "`mode` must be replace, add, subtract or intersect")),
    };
    let area = s.active().ok_or(EngineError::NoDocument)?.doc.bounds();
    let mut mask = vector::fill_rasterizer(&path, vector::DEFAULT_TOLERANCE).render(area);
    if !aa {
        for v in &mut mask {
            *v = if *v >= 0.5 { 1.0 } else { 0.0 };
        }
    }
    if feather > 0.0 {
        mask = sel::feather(&mask, area.width() as usize, area.height() as usize, feather);
    }
    let selected = s.edit("Make Selection", |doc, _| {
        doc.selection = sel::combine(doc.selection.as_ref(), &mask, area, mode);
        Ok(doc.selection.is_some())
    })?;
    Ok(json!({ "selected": selected }))
}

pub(super) fn select_to_work_path(s: &mut Session, p: &Value) -> Result<Value> {
    let tol = f64p(p, "tolerance").unwrap_or(2.0).clamp(0.5, 10.0);
    let d = s.active().ok_or(EngineError::NoDocument)?;
    let area = d.doc.bounds();
    let sel_surface = d.doc.selection.as_ref().ok_or_else(|| EngineError::Other("no selection".into()))?;
    let mask = sel::mask_from_surface(Some(sel_surface), area);
    let path = vector::trace::trace_mask(&mask, area, 0.0, tol);
    let knots: usize = path.subpaths.iter().map(|s| s.knots.len()).sum();
    let subpaths = path.subpaths.len();
    s.edit("Make Work Path", |doc, _| {
        doc.work_path = Some(path);
        Ok(())
    })?;
    Ok(json!({ "subpaths": subpaths, "knots": knots }))
}

pub(super) fn path_fill(s: &mut Session, p: &Value) -> Result<Value> {
    let path = resolve_path(s, p.get("name").and_then(Value::as_str))?;
    let fg = s.tools.foreground;
    let c = p.get("color").and_then(color).unwrap_or(Color::rgba(fg[0], fg[1], fg[2], fg[3]));
    let rgb = c.to_rgb();
    let src = [rgb[0], rgb[1], rgb[2], c.alpha];
    let opacity = (f64p(p, "opacity").unwrap_or(100.0) / 100.0).clamp(0.0, 1.0) as f32;
    let mode = p.get("mode").and_then(Value::as_str).and_then(blend_from_str).unwrap_or(BlendMode::Normal);
    let feather = f64p(p, "feather").unwrap_or(0.0) as f32;
    let aa = p.get("antiAlias").and_then(Value::as_bool).unwrap_or(true);
    let id = layer_id(s, p)?;
    s.edit("Fill Path", |doc, _| {
        let area = doc.bounds();
        let r = vector::fill_rasterizer(&path, vector::DEFAULT_TOLERANCE);
        // The feather reaches 3σ = 3 × radius (σ = radius, as in Photoshop), plus a pixel.
        let feather_pad = (feather.ceil() as i32).saturating_mul(3).saturating_add(1);
        let area = r.pixel_bounds().map_or(area, |b| b.inflate(feather_pad).intersect(&area));
        if area.is_empty() {
            return Ok(());
        }
        let mut cov = r.render(area);
        if !aa {
            cov.iter_mut().for_each(|v| *v = if *v >= 0.5 { 1.0 } else { 0.0 });
        }
        if feather > 0.0 {
            cov = sel::feather(&cov, area.width() as usize, area.height() as usize, feather);
        }
        // An active selection limits the fill, soft edges included, as every other fill does (#2387).
        if let Some(selection) = doc.selection.as_ref() {
            let mask = sel::mask_from_surface(Some(selection), area);
            cov.iter_mut().zip(&mask).for_each(|(c, m)| *c *= m);
        }
        let locks = doc.effective_locks(id);
        let l = doc.layer_mut(id).ok_or(EngineError::NoLayer(id))?;
        if locks.pixels || locks.all {
            return Err(EngineError::Other(format!("Could not complete your request because the layer \"{}\" is locked", l.name)));
        }
        let lock = locks.transparency;
        let surf = l.surface_mut().ok_or_else(|| EngineError::Other("Fill Path needs a pixel layer".into()))?;
        paint_coverage(surf, area, &cov, src, opacity, mode, lock);
        Ok(())
    })?;
    Ok(Value::Null)
}

/// Composites `src` × coverage onto a surface over `area` (any pixel format).
fn paint_coverage(surf: &mut photocraft_raster::Surface, area: Rect, cov: &[f32], src: [f32; 4], opacity: f32, mode: BlendMode, lock_alpha: bool) {
    let fmt = surf.format();
    let ch = fmt.channels();
    let mut vals = surf.read_region(area);
    for (px, c) in vals.chunks_exact_mut(ch).zip(cov) {
        if *c <= 0.0 {
            continue;
        }
        let dst = photocraft_raster::to_rgba(&fmt, px);
        let mut out = photocraft_compose::psblend::composite(mode, dst, [src[0], src[1], src[2], src[3] * c], opacity);
        if lock_alpha {
            out[3] = dst[3];
        }
        photocraft_raster::from_rgba_into(&fmt, out, px);
    }
    surf.write_region(area, &vals);
    surf.prune();
}

/// Stroke Path's brush-based tools besides the Brush, Pencil and Eraser it renders itself: each
/// paints the path with its own `paint.*` command (Photoshop's Stroke Path dialog Tool list).
pub const STROKE_PATH_TOOLS: &[(&str, &str)] = &[
    ("backgroundEraser", "paint.backgroundEraser"),
    ("cloneStamp", "paint.cloneStamp"),
    ("patternStamp", "paint.patternStamp"),
    ("healingBrush", "paint.healingBrush"),
    ("spotHealingBrush", "paint.spotHealing"),
    ("historyBrush", "paint.historyBrush"),
    ("blur", "paint.blur"),
    ("sharpen", "paint.sharpen"),
    ("smudge", "paint.smudge"),
    ("dodge", "paint.dodge"),
    ("burn", "paint.burn"),
    ("sponge", "paint.sponge"),
    ("colorReplacement", "paint.colorReplacement"),
    ("mixerBrush", "paint.mixerBrush"),
];

/// At most this many points per subpath when Simulate Pressure resamples it.
const MAX_PRESSURE_POINTS: f64 = 200_000.0;

/// The stroke points along one flattened subpath. With `simulate` (Stroke Path's Simulate
/// Pressure) the subpath is resampled about every pixel and the pressure rises from 0 at its start
/// to full at the middle and falls back to 0 at its end, so a brush whose size or opacity follows
/// pen pressure tapers at both ends, as in Photoshop. Otherwise every point has full pressure.
fn stroke_points(pts: &[(f64, f64)], closed: bool, simulate: bool) -> Vec<photocraft_paint::StrokePoint> {
    use photocraft_paint::StrokePoint;
    let mut line = pts.to_vec();
    if closed && let Some(&first) = pts.first() {
        line.push(first);
    }
    let full = |line: Vec<(f64, f64)>| line.into_iter().map(|(x, y)| StrokePoint::new(x, y, 1.0)).collect();
    if !simulate {
        return full(line);
    }
    let seg = |a: &(f64, f64), b: &(f64, f64)| (b.0 - a.0).hypot(b.1 - a.1);
    let total: f64 = line.iter().zip(line.iter().skip(1)).map(|(a, b)| seg(a, b)).sum();
    if !(total.is_finite() && total > 0.0) {
        return full(line);
    }
    let step = (total / MAX_PRESSURE_POINTS).max(1.0);
    let pressure = |d: f64| (d / total * std::f64::consts::PI).sin().clamp(0.0, 1.0) as f32;
    let mut out = Vec::new();
    let mut walked = 0.0;
    for (a, b) in line.iter().zip(line.iter().skip(1)) {
        let len = seg(a, b);
        let n = (len / step).ceil().clamp(1.0, MAX_PRESSURE_POINTS) as usize;
        for i in 0..n {
            let t = i as f64 / n as f64;
            out.push(StrokePoint::new(a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t, pressure(walked + len * t)));
        }
        walked += len;
    }
    if let Some(&(x, y)) = line.last() {
        out.push(StrokePoint::new(x, y, 0.0));
    }
    out
}

/// Stroke Path with a tool that has its own `paint.*` command: the command runs once per subpath,
/// with the caller's params (size, opacity, the tool's options) and the path's points, all in one
/// history step. Only `path.stroke` itself is journalled, so a recorded action replays it once.
fn stroke_with_command(s: &mut Session, p: &Value, cmd: &str, lines: &[vector::Polyline], simulate: bool) -> Result<Value> {
    let mut params = p.as_object().cloned().unwrap_or_default();
    for k in ["name", "tool", "simulatePressure", "coalesce"] {
        params.remove(k);
    }
    let journal = s.journal.len();
    let r = crate::analysis_cmds::compound(s, "Stroke Path", |s| {
        // The union of what each subpath's command changed; None when one of them didn't say.
        let mut dmg = Some(Rect::EMPTY);
        for pl in lines {
            let pts = stroke_points(&pl.pts, pl.closed, simulate);
            if pts.is_empty() {
                continue;
            }
            let mut call = params.clone();
            call.insert("points".into(), Value::Array(pts.iter().map(|q| json!([q.x, q.y, q.pressure])).collect()));
            s.execute(cmd, Value::Object(call))?;
            let r = s.active().and_then(|d| d.last_damage);
            dmg = dmg.zip(r).map(|(a, b)| {
                if a.is_empty() {
                    b
                } else if b.is_empty() {
                    a
                } else {
                    a.union(&b)
                }
            });
        }
        Ok(dmg)
    });
    s.journal.truncate(journal);
    let dmg = r?;
    if let Some(st) = s.active_mut() {
        st.last_damage = dmg;
    }
    let dmg = dmg.unwrap_or_else(|| s.active().map_or(Rect::EMPTY, |d| d.doc.bounds()));
    Ok(json!({ "damage": [dmg.x0, dmg.y0, dmg.width(), dmg.height()] }))
}

pub(super) fn path_stroke(s: &mut Session, p: &Value) -> Result<Value> {
    let path = resolve_path(s, p.get("name").and_then(Value::as_str))?;
    let tool = p.get("tool").and_then(Value::as_str).unwrap_or("brush");
    let simulate = match p.get("simulatePressure") {
        None | Some(Value::Null) => false,
        Some(v) => v.as_bool().ok_or_else(|| bad("path.stroke", "`simulatePressure` must be true or false"))?,
    };
    if let Some(&(_, cmd)) = STROKE_PATH_TOOLS.iter().find(|(t, _)| *t == tool) {
        let lines = vector::flatten_path(&path, 0.1);
        return stroke_with_command(s, p, cmd, &lines, simulate);
    }
    let base = s.tools.brush.clone();
    let fg = s.tools.foreground;
    let mut brush = photocraft_paint::BrushSettings {
        size: f64p(p, "size").map_or(base.size, |v| v as f32),
        hardness: f64p(p, "hardness").map_or(base.hardness, |v| v as f32),
        opacity: f64p(p, "opacity").map_or(base.opacity, |v| (v / 100.0) as f32),
        color: p.get("color").and_then(color).map_or(fg, |c| {
            let v = c.to_rgb();
            [v[0], v[1], v[2], c.alpha]
        }),
        erase: tool == "eraser",
        ..base
    };
    // The path is the exact geometry: stroke smoothing (a hand-drawing aid) would cut its corners.
    brush.smoothing.amount = 0.0;
    match tool {
        "pencil" => brush.hardness = 1.0,
        "brush" | "eraser" => {}
        o => {
            let tools: Vec<&str> = ["brush", "pencil", "eraser"].into_iter().chain(STROKE_PATH_TOOLS.iter().map(|(t, _)| *t)).collect();
            return Err(bad("path.stroke", format!("unknown tool `{o}` ({})", tools.join("|"))));
        }
    }
    crate::brush_cmds::validate_brush(&brush, "path.stroke")?;
    let lines = vector::flatten_path(&path, 0.1);
    let id = layer_id(s, p)?;
    let bg = s.tools.background;
    let dmg = s.edit("Stroke Path", |doc, _| {
        let sel = doc.selection.clone();
        let locks = doc.effective_locks(id);
        let l = doc.layer_mut(id).ok_or(EngineError::NoLayer(id))?;
        // Pixel and full locks (the layer's own or a parent group's) refuse the stroke, as Fill Path does (#2388).
        if locks.pixels || locks.all {
            return Err(EngineError::Other(format!("Could not complete your request because the layer \"{}\" is locked", l.name)));
        }
        let lock = locks.transparency;
        let surf = l.surface_mut().ok_or_else(|| EngineError::Other("Stroke Path needs a pixel layer".into()))?;
        let mut brush = brush.clone();
        if brush.erase && lock {
            // Locked transparency (e.g. the Background): the Eraser paints the background colour (#76).
            brush.erase = false;
            brush.color = bg;
        }
        let mut dmg = Rect::EMPTY;
        for pl in &lines {
            let pts = stroke_points(&pl.pts, pl.closed, simulate);
            if pts.is_empty() {
                continue;
            }
            let r = photocraft_paint::apply_stroke(surf, &photocraft_paint::Stroke { brush: brush.clone(), points: pts }, sel.as_ref(), lock);
            dmg = if dmg.is_empty() { r } else { dmg.union(&r) };
        }
        Ok(dmg)
    })?;
    Ok(json!({ "damage": [dmg.x0, dmg.y0, dmg.width(), dmg.height()] }))
}
