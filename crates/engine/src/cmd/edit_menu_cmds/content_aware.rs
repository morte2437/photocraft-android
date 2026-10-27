use super::*;

// ------------------------------------------------------------------ Content-Aware Fill

pub(super) fn can_caf(s: &Session) -> std::result::Result<(), String> {
    pixel_layer(s)?;
    let d = s.active().ok_or("no document open")?;
    if d.doc.selection.as_ref().is_none_or(|m| m.content_bounds().is_empty()) {
        return Err("make a selection around what to remove".into());
    }
    Ok(())
}

/// Mask surface (gray) of an alpha channel by index or name.
pub(crate) fn channel_mask<'a>(doc: &'a Document, v: &Value) -> Option<&'a Surface> {
    match v {
        Value::Number(n) => doc.channels.get(n.as_u64()? as usize).map(|c| &c.surface),
        Value::String(name) => doc.channels.iter().find(|c| c.name == *name).map(|c| &c.surface),
        _ => None,
    }
}

/// `key` as an `[x, y, w, h]` rectangle. The saturating float→int casts bound each component;
/// the additions saturate too — `area: [1e30, 0, 1e30, 10]` is a whole-canvas window, not an
/// overflow (it panicked in debug builds before the `saturating_add`).
fn rect_param(p: &Value, key: &str, cmd: &str) -> Result<Rect> {
    let a = p.get(key).and_then(Value::as_array).ok_or_else(|| bad(cmd, format!("`{key}` must be [x, y, w, h]")))?;
    let n = |i: usize| {
        a.get(i)
            .and_then(Value::as_f64)
            .filter(|f| f.is_finite())
            .map(|v| v.round() as i32)
            .ok_or_else(|| bad(cmd, format!("`{key}` must be four finite numbers")))
    };
    let (x, y, w, h) = (n(0)?, n(1)?, n(2)?, n(3)?);
    Ok(Rect::new(x, y, x.saturating_add(w.max(0)), y.saturating_add(h.max(0))))
}

pub(super) fn content_aware_fill(s: &mut Session, p: &Value) -> Result<Value> {
    content_aware_fill_as(s, p, "edit.contentAwareFill", "Content-Aware Fill")
}

/// Delete and Fill Selection (#1286): Photoshop's one-click removal from the selection-tool
/// context menu. Content-Aware Fill into the layer, no dialog, sampling as Edit › Fill does (the
/// whole window, measured on Photoshop 25.4) with the default colour adaptation.
pub(super) fn delete_and_fill(s: &mut Session, _: &Value) -> Result<Value> {
    content_aware_fill_as(s, &json!({"sampling": "rectangular"}), "edit.deleteAndFillSelection", "Delete and Fill Selection")
}

/// Everything one Content-Aware Fill needs, worked out from the document and the command's
/// params. The Content-Aware Fill workspace runs exactly this for its preview, so what it shows
/// is what OK and Apply write.
pub struct CafPlan {
    pub layer: LayerId,
    /// The rectangle worked on: the sampling window, and any sampling-brush strokes or custom
    /// area beyond it.
    pub rect: Rect,
    /// Photoshop's sampling window around the fill area (`content_aware::sampling_window`).
    pub window: Rect,
    pub fmt: PixelFormat,
    /// What is sampled over `rect`: the target's pixels, or with Sample All Layers the visible
    /// composite in the target's encoding.
    pub img: Vec<f32>,
    /// The target's own pixels over `rect` when they differ from `img` (Sample All Layers); see
    /// [`CafPlan::layer_pixels`].
    pub layer_px: Option<Vec<f32>>,
    /// The fill area (selection coverage, after the workspace's lasso edits) over `rect`.
    pub cover: Vec<f32>,
    pub hole: Vec<bool>,
    /// What may be copied from: the sampling option's area before the brush strokes, and after.
    pub base: Vec<bool>,
    pub source: Vec<bool>,
    pub opts: photocraft_algo::content_aware::FillOptions,
    pub output: String,
}

impl CafPlan {
    /// The target layer's pixels over `rect`.
    pub fn layer_pixels(&self) -> &[f32] {
        self.layer_px.as_deref().unwrap_or(&self.img)
    }
}

/// The workspace's lasso edits of the fill area: `{"op": "new|add|subtract|intersect", "points":
/// [[x, y], ...]}` or `{"op": "expand|contract", "by": px}`, applied in order.
#[derive(Clone, Debug, PartialEq)]
pub enum FillEdit {
    Lasso { op: SelOp, points: Vec<[f64; 2]> },
    Grow(i32),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelOp {
    New,
    Add,
    Subtract,
    Intersect,
}

/// Most `fillEdits` and `samplingStrokes` one fill takes, and most points in all of them
/// together: the strokes and lassos are painted pixel by pixel, segment by segment.
pub const MAX_CAF_EDITS: usize = 1_000;

pub const MAX_CAF_POINTS: usize = 200_000;

fn points_param(v: &Value, cmd: &str, what: &str, budget: &mut usize) -> Result<Vec<[f64; 2]>> {
    let a = v.get("points").and_then(Value::as_array).ok_or_else(|| bad(cmd, format!("{what} needs `points` [[x, y], ...]")))?;
    *budget = budget.checked_sub(a.len()).ok_or_else(|| bad(cmd, format!("at most {MAX_CAF_POINTS} points in all fill edits and sampling strokes")))?;
    a.iter()
        .map(|p| {
            let x = p.get(0).and_then(Value::as_f64).filter(|v| v.is_finite() && v.abs() < 1e7);
            let y = p.get(1).and_then(Value::as_f64).filter(|v| v.is_finite() && v.abs() < 1e7);
            match (x, y) {
                (Some(x), Some(y)) => Ok([x, y]),
                _ => Err(bad(cmd, format!("{what} points must be pairs of finite numbers"))),
            }
        })
        .collect()
}

/// `p[key]` as an array of at most [`MAX_CAF_EDITS`] entries (empty when absent).
fn edits_param<'a>(p: &'a Value, key: &str, cmd: &str) -> Result<&'a [Value]> {
    let Some(a) = p.get(key) else { return Ok(&[]) };
    let a = a.as_array().ok_or_else(|| bad(cmd, format!("`{key}` must be an array")))?;
    if a.len() > MAX_CAF_EDITS {
        return Err(bad(cmd, format!("`{key}` takes at most {MAX_CAF_EDITS} entries")));
    }
    Ok(a)
}

fn fill_edits(p: &Value, cmd: &str, budget: &mut usize) -> Result<Vec<FillEdit>> {
    edits_param(p, "fillEdits", cmd)?
        .iter()
        .map(|e| {
            let op = e.get("op").and_then(Value::as_str).unwrap_or("");
            Ok(match op {
                "expand" | "contract" => {
                    let by = e
                        .get("by")
                        .and_then(Value::as_f64)
                        .filter(|v| v.is_finite() && (0.0..=1000.0).contains(v))
                        .ok_or_else(|| bad(cmd, "`by` must be 0..1000 px"))?;
                    FillEdit::Grow(if op == "expand" { by.round() as i32 } else { -(by.round() as i32) })
                }
                "new" | "add" | "subtract" | "intersect" => FillEdit::Lasso {
                    op: match op {
                        "new" => SelOp::New,
                        "add" => SelOp::Add,
                        "subtract" => SelOp::Subtract,
                        _ => SelOp::Intersect,
                    },
                    points: points_param(e, cmd, "a lasso edit", budget)?,
                },
                o => return Err(bad(cmd, format!("unknown fill edit `{o}` (new|add|subtract|intersect|expand|contract)"))),
            })
        })
        .collect()
}

fn sampling_strokes(p: &Value, cmd: &str, budget: &mut usize) -> Result<Vec<photocraft_algo::content_aware::SamplingStroke>> {
    edits_param(p, "samplingStrokes", cmd)?
        .iter()
        .map(|e| {
            let add = match e.get("mode").and_then(Value::as_str).unwrap_or("add") {
                "add" => true,
                "subtract" => false,
                o => return Err(bad(cmd, format!("unknown stroke mode `{o}` (add|subtract)"))),
            };
            let size = e
                .get("size")
                .and_then(Value::as_f64)
                .filter(|v| v.is_finite() && (1.0..=5000.0).contains(v))
                .ok_or_else(|| bad(cmd, "a stroke's `size` must be 1..5000 px"))?;
            Ok(photocraft_algo::content_aware::SamplingStroke { add, size, points: points_param(e, cmd, "a sampling stroke", budget)? })
        })
        .collect()
}

/// A Content-Aware Fill's params, checked against the document before any pixel is read.
struct CafParams<'a> {
    sel: &'a Surface,
    /// The selection's bounds on the canvas.
    sb: Rect,
    edits: Vec<FillEdit>,
    strokes: Vec<photocraft_algo::content_aware::SamplingStroke>,
    sampling: String,
    custom_mask: Option<&'a Surface>,
    custom_rect: Option<Rect>,
    opts: photocraft_algo::content_aware::FillOptions,
    output: String,
    surf: &'a Surface,
}

fn caf_params<'a>(doc: &'a Document, id: LayerId, p: &Value, cmd: &str) -> Result<CafParams<'a>> {
    use photocraft_algo::content_aware::{FillOptions, color_level, rotation_level};
    let sel = doc.selection.as_ref().ok_or_else(|| bad(cmd, "no selection"))?;
    let sb = sel.content_bounds().intersect(&doc.bounds());
    if sb.is_empty() {
        return Err(bad(cmd, "the selection is outside the canvas"));
    }
    let mut budget = MAX_CAF_POINTS;
    let edits = fill_edits(p, cmd, &mut budget)?;
    let strokes = sampling_strokes(p, cmd, &mut budget)?;
    let sampling = str_or(p, "sampling", "auto").to_string();
    let custom_mask: Option<&Surface> = p.get("channel").and_then(|v| channel_mask(doc, v));
    let custom_rect = if p.get("area").is_some() { Some(rect_param(p, "area", cmd)?) } else { None };
    match sampling.as_str() {
        "auto" | "rectangular" => {}
        // The workspace's Custom: only what the Sampling Brush adds.
        "custom" if custom_rect.is_some() || custom_mask.is_some() || strokes.iter().any(|s| s.add) => {}
        "custom" => return Err(bad(cmd, "custom sampling needs `area` [x,y,w,h], `channel` (alpha channel index or name) or `samplingStrokes`")),
        other => return Err(bad(cmd, format!("unknown sampling `{other}` (auto|rectangular|custom)"))),
    }
    let opts = FillOptions {
        color_adaptation: color_level(str_or(p, "colorAdaptation", "default")),
        rotations: rotation_level(str_or(p, "rotationAdaptation", "none")),
        scale: bool_or(p, "scale", false),
        mirror: bool_or(p, "mirror", false),
        seed: p.get("seed").and_then(Value::as_u64).unwrap_or(1),
    };
    let output = str_or(p, "output", "current").to_string();
    if !matches!(output.as_str(), "current" | "new" | "duplicate") {
        return Err(bad(cmd, "`output` must be current|new|duplicate"));
    }
    let surf = doc.layer(id).and_then(|l| l.surface()).ok_or(EngineError::NoLayer(id))?;
    Ok(CafParams { sel, sb, edits, strokes, sampling, custom_mask, custom_rect, opts, output, surf })
}

/// Work out a Content-Aware Fill (see [`CafPlan`]) on layer `id` of `doc`.
pub fn content_aware_plan(doc: &Document, id: LayerId, p: &Value, cmd: &str) -> Result<CafPlan> {
    content_aware_plan_with(doc, id, p, cmd, &photocraft_raster::Interrupt::NONE)
}

/// [`content_aware_plan`], asking `ctl` row by row whether to stop ([`EngineError::Cancelled`]).
pub fn content_aware_plan_with(doc: &Document, id: LayerId, p: &Value, cmd: &str, ctl: &photocraft_raster::Interrupt) -> Result<CafPlan> {
    use photocraft_algo::content_aware::{auto_sampling_with, grow_mask, paint_strokes, polygon_mask};
    use rayon::prelude::*;

    use crate::fill_cmds::{read_window, read_window_mask};
    let CafParams { sel, sb, edits, strokes, sampling, custom_mask, custom_rect, opts, output, surf } = caf_params(doc, id, p, cmd)?;
    let canvas = doc.bounds();
    let cancel = || ctl.cancelled();
    let stop = || if ctl.cancelled() { Err(EngineError::Cancelled) } else { Ok(()) };
    // The fill area: the selection, then the lasso edits, over a rectangle that holds them all.
    let grow: i32 = edits.iter().map(|e| if let FillEdit::Grow(b) = e { b.max(&0) } else { &0 }).sum();
    let reach = edits.iter().fold(sb, |r, e| match e {
        FillEdit::Lasso { points, .. } => {
            points.iter().fold(r, |r, q| r.union(&Rect::new(q[0].floor() as i32, q[1].floor() as i32, q[0].floor() as i32 + 1, q[1].floor() as i32 + 1)))
        }
        FillEdit::Grow(_) => r,
    });
    let er = reach.inflate(grow.saturating_add(1)).intersect(&canvas);
    let (ew, eh) = crate::fill_cmds::window_size(er)?;
    let ecover = read_window_mask(sel, er, &cancel).ok_or(EngineError::Cancelled)?;
    let mut ehole: Vec<bool> = ecover.par_iter().map(|c| *c > 0.0).collect();
    for e in &edits {
        stop()?;
        match e {
            FillEdit::Lasso { op, points } => {
                let m = polygon_mask(er, points);
                for (h, v) in ehole.iter_mut().zip(&m) {
                    *h = match op {
                        SelOp::New => *v,
                        SelOp::Add => *h || *v,
                        SelOp::Subtract => *h && !*v,
                        SelOp::Intersect => *h && *v,
                    };
                }
            }
            FillEdit::Grow(by) => ehole = grow_mask(&ehole, ew, eh, *by),
        }
    }
    let (bx0, by0, bx1, by1) = mask_bounds(ew, eh, &ehole);
    if bx1 <= bx0 {
        return Err(bad(cmd, "the fill area is empty"));
    }
    let hb = Rect::new(er.x0 + bx0 as i32, er.y0 + by0 as i32, er.x0 + bx1 as i32, er.y0 + by1 as i32);
    // Photoshop's window (`content_aware::sampling_window`) for Auto and Rectangular; `margin`
    // asks for a rectangle that far around the fill area instead.
    let window = match sampling.as_str() {
        "rectangular" => match int(p, "margin") {
            Some(m) => hb.inflate(m.clamp(0, 100_000) as i32),
            None => crate::fill_cmds::sampling_window(hb, canvas),
        },
        "custom" => match (custom_rect, custom_mask) {
            (Some(r), _) => r,
            (None, Some(m)) => m.content_bounds(),
            (None, None) => hb,
        }
        .union(&hb)
        .inflate(4),
        _ => crate::fill_cmds::sampling_window(hb, canvas),
    }
    .intersect(&canvas);
    // Strokes that add to the sampling area may reach past the window.
    let rect = strokes.iter().filter(|s| s.add).fold(window, |r, s| r.union(&s.bounds())).intersect(&canvas);
    let fmt = surf.format();
    // Photoshop's Sample All Layers checkbox is off unless the caller asks. When the active
    // layer has no pixels in the window there is nothing to sample, so that case uses the
    // visible composite (#4026). An explicit false still reads only this layer.
    let sample_all = match p.get("sampleAllLayers").and_then(Value::as_bool) {
        Some(v) => v,
        None => surf.content_bounds().intersect(&window).is_empty(),
    };
    let n = fmt.channels();
    // Refuse a window too large to read before allocating for it, whatever the sampling (#963).
    let (w, h) = crate::fill_cmds::window_size(rect)?;
    let pixels = read_window(surf, rect, &cancel).ok_or(EngineError::Cancelled)?;
    let (img, layer_px) = if sample_all {
        stop()?;
        (crate::retouch_cmds::composite_region(doc, Some(id), crate::retouch_cmds::SampleLayers::All, rect, fmt).data, Some(pixels))
    } else {
        (pixels, None)
    };
    stop()?;
    let mut cover = vec![0.0f32; w * h];
    let mut hole = vec![false; w * h];
    cover.par_chunks_mut(w.max(1)).zip(hole.par_chunks_mut(w.max(1))).enumerate().try_for_each(|(y, (crow, hrow))| {
        stop()?;
        let dy = rect.y0 + y as i32;
        if dy < er.y0 || dy >= er.y1 {
            return Ok(());
        }
        for (x, (c, hl)) in crow.iter_mut().zip(hrow.iter_mut()).enumerate() {
            let dx = rect.x0 + x as i32;
            if dx >= er.x0 && dx < er.x1 {
                let j = (dy - er.y0) as usize * ew + (dx - er.x0) as usize;
                let (eh, ec) = (ehole.get(j).copied().unwrap_or(false), ecover.get(j).copied().unwrap_or(0.0));
                *hl = eh;
                // Pixels the lasso added are filled outright; selected ones by their coverage.
                *c = if eh { if ec > 0.0 { ec } else { 1.0 } } else { 0.0 };
            }
        }
        Ok::<(), EngineError>(())
    })?;
    drop((ecover, ehole));
    // The sampling option's area: Auto keeps what looks like the surroundings within the window,
    // Rectangular the whole window, Custom the given area (or nothing, for the brush to add to).
    let mut base = vec![false; w * h];
    match sampling.as_str() {
        "auto" | "rectangular" => {
            let (ww, wh) = (window.width() as usize, window.height() as usize);
            let (ox, oy) = ((window.x0 - rect.x0) as usize, (window.y0 - rect.y0) as usize);
            let area: Vec<bool> = match sampling.as_str() {
                "auto" if window == rect => auto_sampling_with(w, h, n, &img, &hole, ctl).map_err(|_| EngineError::Cancelled)?,
                "auto" => {
                    let mut wimg = vec![0.0f32; ww * wh * n];
                    let mut whole = vec![false; ww * wh];
                    for y in 0..wh {
                        let s = ((oy + y) * w + ox) * n;
                        wimg[y * ww * n..(y + 1) * ww * n].copy_from_slice(&img[s..s + ww * n]);
                        whole[y * ww..(y + 1) * ww].copy_from_slice(&hole[(oy + y) * w + ox..(oy + y) * w + ox + ww]);
                    }
                    auto_sampling_with(ww, wh, n, &wimg, &whole, ctl).map_err(|_| EngineError::Cancelled)?
                }
                _ => vec![true; ww * wh],
            };
            for y in 0..wh {
                base[(oy + y) * w + ox..(oy + y) * w + ox + ww].copy_from_slice(&area[y * ww..(y + 1) * ww]);
            }
        }
        _ => match (custom_rect, custom_mask) {
            (Some(r), _) => base.par_iter_mut().enumerate().for_each(|(i, b)| *b = r.contains(rect.x0 + (i % w) as i32, rect.y0 + (i / w) as i32)),
            (None, Some(m)) => {
                let v = read_window_mask(m, rect, &cancel).ok_or(EngineError::Cancelled)?;
                base.par_iter_mut().zip(&v).for_each(|(b, v)| *b = *v > 0.5);
            }
            (None, None) => {}
        },
    }
    stop()?;
    base.par_iter_mut().zip(&hole).for_each(|(b, h)| *b &= !*h);
    let mut source = base.clone();
    paint_strokes(&mut source, rect, &strokes);
    source.par_iter_mut().zip(&hole).for_each(|(s, h)| *s &= !*h);
    Ok(CafPlan { layer: id, rect, window, fmt, img, layer_px, cover, hole, base, source, opts, output })
}

fn content_aware_fill_as(s: &mut Session, p: &Value, cmd: &'static str, label: &'static str) -> Result<Value> {
    use photocraft_algo::content_aware::fill_owned;
    // The workspace names the layer it opened on: after an Apply to a new layer that one is
    // active, but the fill still samples the original.
    let id = match p.get("layer").and_then(Value::as_u64) {
        Some(l) => pixel_layer_id(s, LayerId(l)),
        None => pixel_layer(s),
    }
    .map_err(EngineError::Other)?;
    let st = s.active().ok_or(EngineError::NoDocument)?;
    let doc = st.doc.clone();
    // Check the params before the job starts.
    caf_params(&doc, id, p, cmd)?;
    let p = p.clone();
    // A background job when started with `Session::start` (#210): working out the fill area and
    // the sampling area, reading the pixels and the PatchMatch fill run on a worker against the
    // document snapshot, cancellable per row band.
    crate::jobs::run(
        s,
        label,
        true,
        move |ctx| {
            use rayon::prelude::*;
            ctx.progress(0.0, label);
            let cancel = || ctx.cancelled();
            let plan = content_aware_plan_with(&doc, id, &p, cmd, &photocraft_raster::Interrupt::cancel_only(&cancel))?;
            let CafPlan { layer, rect, fmt, img, layer_px, cover, hole, source, opts, output, .. } = plan;
            // Patches come only from the sampling area outside the fill area. With none (a
            // Rectangular `margin` of 0, a Custom area inside the selection or off the canvas)
            // the fill would be black (#6532), so refuse instead. The workspace's preview builds
            // the same plan and may show an empty area while the Sampling Brush adds to it.
            if !source.iter().any(|s| *s) {
                return Err(bad(cmd, "nothing to sample from: the sampling area has no pixels outside the fill area"));
            }
            let (w, h) = (rect.width() as usize, rect.height() as usize);
            let n = fmt.channels();
            // The fill changes only the fill area: keep the layer's pixels and the coverage over
            // its bounds for the apply step, and fill the sampled pixels themselves.
            let (bx0, by0, bx1, by1) = mask_bounds(w, h, &hole);
            let own = layer_px.as_deref().unwrap_or(&img);
            let original: Vec<f32> = (by0..by1).flat_map(|y| own[(y * w + bx0) * n..(y * w + bx1) * n].iter().copied()).collect();
            let cover: Vec<f32> = (by0..by1).flat_map(|y| cover[y * w + bx0..y * w + bx1].iter().copied()).collect();
            let n_hole = cover.iter().filter(|c| **c > 0.0).count();
            ctx.check()?;
            let mut filled = ctx.stage(0.05, 1.0, label, |ctl| fill_owned(w, h, n, img, &hole, &source, &opts, ctl)).map_err(|_| EngineError::Cancelled)?;
            // Sampled from all layers: outside the fill area the window keeps the layer's pixels.
            if let Some(own) = layer_px {
                filled.par_chunks_mut(n.max(1)).zip(own.par_chunks(n.max(1))).zip(hole.par_iter()).for_each(|((f, o), h)| {
                    if !*h {
                        f.copy_from_slice(o);
                    }
                });
            }
            Ok((layer, rect, fmt, output, FilledWindow { filled, bounds: (bx0, by0, bx1, by1), original, cover, n_hole }))
        },
        move |s, (layer, rect, fmt, output, done)| apply_content_aware_fill(s, label, layer, &output, fmt, rect, done),
    )
}

/// A Content-Aware Fill's result for its apply step: the filled window (the same as the
/// original outside the selected pixels) and, over the bounds of the selected pixels (in window
/// coordinates), the original pixels and the selection's coverage.
struct FilledWindow {
    filled: Vec<f32>,
    bounds: (usize, usize, usize, usize),
    original: Vec<f32>,
    cover: Vec<f32>,
    n_hole: usize,
}

/// `f(window index, index over the bounds, coverage)` for each selected pixel of a
/// [`FilledWindow`] (`w` wide).
fn each_selected(w: usize, (bx0, by0, bx1, by1): (usize, usize, usize, usize), cover: &[f32], mut f: impl FnMut(usize, usize, f32)) {
    let bw = bx1.saturating_sub(bx0);
    for y in by0..by1 {
        for x in bx0..bx1 {
            let k = (y - by0) * bw + (x - bx0);
            if let Some(&c) = cover.get(k)
                && c > 0.0
            {
                f(y * w + x, k, c);
            }
        }
    }
}

/// Bounds `(x0, y0, x1, y1)` of the set cells of a `w × h` mask (all zero when there are none).
fn mask_bounds(w: usize, h: usize, m: &[bool]) -> (usize, usize, usize, usize) {
    use rayon::prelude::*;
    if w == 0 {
        return (0, 0, 0, 0);
    }
    let rows: Vec<Option<(usize, usize)>> =
        m.par_chunks(w).take(h).map(|row| Some((row.iter().position(|v| *v)?, row.iter().rposition(|v| *v)? + 1))).collect();
    let mut b: Option<(usize, usize, usize, usize)> = None;
    for (y, span) in rows.into_iter().enumerate() {
        if let Some((x0, x1)) = span {
            b = Some(b.map_or((x0, y, x1, y + 1), |(bx0, by0, bx1, _)| (bx0.min(x0), by0, bx1.max(x1), y + 1)));
        }
    }
    b.unwrap_or((0, 0, 0, 0))
}

/// Write a Content-Aware Fill result (the job's apply step): into the layer, a new layer or a
/// duplicate, as one undo step.
fn apply_content_aware_fill(s: &mut Session, label: &str, id: LayerId, output: &str, fmt: PixelFormat, window: Rect, done: FilledWindow) -> Result<Value> {
    let n = fmt.channels();
    let (w, h) = (window.width() as usize, window.height() as usize);
    let FilledWindow { mut filled, bounds, original, cover, n_hole } = done;
    let layer = s.edit(label, |doc, active| {
        let target = match output {
            "new" => {
                let mut l = Layer::raster(doc.next_layer_name("Content-Aware Fill"), PixelFormat { alpha: true, ..fmt });
                let mut px = vec![0.0f32; w * h * n.max(1)];
                let lf = l.surface().map(|s| s.format()).unwrap_or(fmt);
                let ln = lf.channels();
                px.resize(w * h * ln, 0.0);
                each_selected(w, bounds, &cover, |i, _, c| {
                    let rgba = to_rgba(&fmt, &filled[i * n..(i + 1) * n]);
                    let mut enc = [0.0f32; 8];
                    from_rgba_into(&lf, [rgba[0], rgba[1], rgba[2], rgba[3] * c], &mut enc);
                    px[i * ln..(i + 1) * ln].copy_from_slice(&enc[..ln]);
                });
                if let Some(s) = l.surface_mut() {
                    s.write_region(window, &px);
                    s.prune();
                }
                let nid = doc.insert_above(Some(id), l);
                *active = Some(nid);
                return Ok(nid);
            }
            "duplicate" => {
                let mut dup = doc.layer(id).ok_or(EngineError::NoLayer(id))?.duplicate();
                dup.name = doc.copy_name(&dup.name);
                let nid = doc.insert_above(Some(id), dup);
                *active = Some(nid);
                nid
            }
            _ => id,
        };
        let surf = writable_surface(doc, target)?;
        // The fill blended in by the coverage (elsewhere it is the original).
        each_selected(w, bounds, &cover, |i, k, c| {
            for ch in 0..n {
                let (j, o) = (i * n + ch, k * n + ch);
                filled[j] = original[o] + (filled[j] - original[o]) * c;
            }
        });
        surf.write_region(window, &filled);
        Ok(target)
    })?;
    Ok(json!({"layer": layer.0, "filled": n_hole, "window": [window.x0, window.y0, window.width(), window.height()]}))
}
