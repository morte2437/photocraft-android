use super::heal::history_source;
use super::sampling::{SampleLayers, sample, sample_layers};
use super::{
    BlendMode, BrushSettings, Document, EngineError, LayerId, PixelFormat, Rect, Region, Result, SampleType, Session, Stroke, StrokePoint, Surface, Value,
    alpha_index, apply_coverage, bad, blend_from_str, coverage, damage_json, flag, has_pixel_layer, json, parse_brush, pattern_stamp, periodic, point,
    reject_huge_stroke, run_stroke, to_rgba,
};

/// Where a clone stroke samples: `offset` / `source` params, else the active Clone Source slot,
/// with the slot's (or the call's) scale, rotation and flips (see `presets::clone_source`).
pub(super) fn clone_mapping(s: &mut Session, p: &Value, stroke: &Stroke, cmd: &str) -> Result<crate::presets::clone_source::Mapping> {
    let f = stroke.points[0];
    crate::presets::clone_source::mapping(s, p, (f.x, f.y), cmd)
}

/// Source pixels for destination `rect`: a translated read, or a bilinear resample when the
/// clone source is scaled, rotated or flipped.
#[allow(clippy::too_many_arguments)]
pub(super) fn clone_sample(
    pre: &Document,
    id: Option<LayerId>,
    surf: &Surface,
    which: SampleLayers,
    rect: Rect,
    map: &crate::presets::clone_source::Mapping,
    wrap: bool,
    cmd: &str,
) -> Result<Region> {
    let read = |rect| {
        if wrap { periodic::sample(pre, id, surf, which, rect, cmd) } else { Ok(sample(pre, id, surf, which, rect)) }
    };
    if map.is_translation() {
        let off = map.offset();
        let src = rect.translate(off.0, off.1);
        // `translate` saturates at the i32 edges, so a far source (issue #1111) yields a window
        // smaller than `rect`; relabelling that as `rect` would index past the sampled pixels.
        if src.width() != rect.width() || src.height() != rect.height() {
            return Err(bad(cmd, format!("source offset [{}, {}] is too far: the sample window leaves the coordinate range", off.0, off.1)));
        }
        let mut r = read(src)?;
        r.rect = rect;
        return Ok(r);
    }
    let src = read(map.source_rect(rect))?;
    Ok(crate::presets::clone_source::resample(&src, rect, map, alpha_index(&surf.format())))
}

/// Read-only clone overlay. Shares the stroke sampler without anchoring a source or editing history.
/// Both destination and transformed source allocations are bounded for interactive use.
pub fn clone_preview(s: &Session, rect: Rect, map: &crate::presets::clone_source::Mapping, sample_layer: &str) -> Result<photocraft_compose::Buffer> {
    clone_preview_periodic(s, rect, map, sample_layer, false)
}

/// Clone-source overlay with optional source repetition for View > Pattern Preview.
pub fn clone_preview_periodic(
    s: &Session,
    rect: Rect,
    map: &crate::presets::clone_source::Mapping,
    sample_layer: &str,
    wrap: bool,
) -> Result<photocraft_compose::Buffer> {
    if ![map.source.0, map.source.1, map.anchor.0, map.anchor.1].into_iter().chain(map.m).all(|v| v.is_finite() && v.abs() < 1_000_000.0) {
        return Err(bad("clone.preview", "invalid preview mapping"));
    }
    if ![rect.x0, rect.y0, rect.x1, rect.y1].into_iter().all(|v| v.abs_diff(0) < 1_000_000)
        || ![(rect.x0, rect.y0), (rect.x1, rect.y1), (rect.x0, rect.y1), (rect.x1, rect.y0)].into_iter().all(|(x, y)| {
            let (x, y) = map.map(f64::from(x), f64::from(y));
            x.is_finite() && y.is_finite() && x.abs() < 1_000_000.0 && y.abs() < 1_000_000.0
        })
    {
        return Err(bad("clone.preview", "preview coordinates exceed their budget"));
    }
    let bounded = |r: Rect| r.width() > 0 && r.height() > 0 && r.width() <= 1024 && r.height() <= 1024;
    if !bounded(rect) || !bounded(map.source_rect(rect)) {
        return Err(bad("clone.preview", "preview region exceeds its budget"));
    }
    let d = s.active().ok_or(EngineError::NoDocument)?;
    let id = d.active_layer.ok_or_else(|| bad("clone.preview", "no active layer"))?;
    let surf = d.doc.layer(id).and_then(|l| l.surface()).ok_or_else(|| bad("clone.preview", "no pixel surface"))?;
    let which = sample_layers(&json!({"sampleLayer": sample_layer}), "clone.preview")?;
    let region = clone_sample(&d.doc, Some(id), surf, which, rect, map, wrap, "clone.preview")?;
    let px = region.data.chunks_exact(surf.format().channels()).map(|p| to_rgba(&surf.format(), p)).collect();
    Ok(photocraft_compose::Buffer { rect, px })
}

/// Result JSON shared by Clone Stamp and Healing Brush: the offset used and where the next stroke
/// should sample from if the UI keeps Photoshop's "Aligned" semantics.
pub(super) fn clone_result(dmg: Rect, off: (i32, i32), aligned: bool, p: &Value, stroke: &Stroke) -> Value {
    let f = stroke.points[0];
    let source = point(p, "source").unwrap_or((f.x + off.0 as f64, f.y + off.1 as f64));
    // Aligned: the offset persists (next stroke samples at its own first point + offset).
    // Non-aligned: every stroke restarts at the original source point.
    json!({ "damage": damage_json(dmg), "offset": [off.0, off.1], "aligned": aligned, "nextSource": if aligned { Value::Null } else { json!([source.0, source.1]) } })
}

pub(super) fn blend_param(p: &Value, cmd: &str) -> Result<BlendMode> {
    match p.get("mode").and_then(Value::as_str) {
        None => Ok(BlendMode::Normal),
        Some(m) => blend_from_str(m).ok_or_else(|| bad(cmd, format!("unknown blend mode `{m}`"))),
    }
}

/// Clamp results into the storable range: integer depths to 0..1, float depths only below at 0;
/// alpha always 0..1.
pub(super) fn clamp_samples(fmt: &PixelFormat, data: &mut [f32]) {
    let n = fmt.channels();
    let a = alpha_index(fmt);
    let float = fmt.sample == SampleType::F32;
    for px in data.chunks_exact_mut(n) {
        for (c, v) in px.iter_mut().enumerate() {
            *v = if Some(c) == a || !float { v.clamp(0.0, 1.0) } else { v.max(0.0) };
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Clone Stamp / Healing Brush / History Brush
// ---------------------------------------------------------------------------------------------

pub(super) fn clone_stamp(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "paint.cloneStamp";
    let (stroke, id) = parse_brush(s, p, CMD)?;
    reject_huge_stroke(CMD, &stroke)?;
    let which = sample_layers(p, CMD)?;
    let mode = blend_param(p, CMD)?;
    let aligned = flag(p, "aligned", true);
    // The live stroke the canvas already drew for exactly this commit: take its pixels instead of
    // rendering the stroke again (the hitch on release).
    let live = s.live_commit.take().filter(|l| l.matches(s, CMD, p, &stroke.points));
    let from_live = live.is_some();
    let map = clone_mapping(s, p, &stroke, CMD)?;
    let off = map.offset();
    let dmg =
        run_stroke(s, "Clone Stamp", id, p, |pre, surf, sel, lock| {
            if let Some(l) = live {
                *surf = l.surface;
                return Ok(l.bounds);
            }
            if flag(p, "wrap", false) {
                let mut renderer = photocraft_paint::StrokeRenderer::new(&stroke.brush, None, 1.0);
                renderer.push(&stroke.points);
                renderer.finish();
                let before = surf.clone();
                let bounds = renderer.bounds();
                return periodic::Composite { target: surf, before: &before, tile: pre.bounds(), opacity: stroke.brush.opacity, selection: sel, lock, mode }
                    .draw(bounds, &stroke.brush, &stroke.points, |r| clone_sample(pre, id, &before, which, r, &map, true, CMD));
            }
            let (bounds, cov) = coverage(CMD, &stroke)?;
            // Samples come from the pre-stroke state, so pixels painted earlier in the stroke are never re-cloned.
            let paint = clone_sample(pre, id, surf, which, bounds, &map, flag(p, "wrap", false), CMD)?;
            Ok(apply_coverage(surf, bounds, &cov, stroke.brush.opacity, sel, lock, &paint, mode))
        })?;
    let mut r = clone_result(dmg, off, aligned, p, &stroke);
    if from_live {
        r["fromLive"] = json!(true);
    }
    Ok(r)
}

/// What a live retouching stroke paints through its coverage.
enum LivePaint {
    /// The clone source (Clone Stamp; the Healing Brush shows its texture while drawing and
    /// blends it on release).
    Clone { which: SampleLayers, map: crate::presets::clone_source::Mapping, cmd: &'static str },
    /// The History Brush's history state.
    History(Surface),
    /// The Pattern Stamp's pattern, tiled from its aligned placement (Impressionist strokes are
    /// laid dab by dab: `pattern_stamp::live_impressionist`).
    Pattern { tile: std::sync::Arc<photocraft_compose::pattern::Tile>, place: photocraft_compose::pattern::Placement },
}

/// A retouching stroke shown while it is drawn: the dabs so far composited onto a copy of the
/// active document as the command with the same params commits them (the same coverage,
/// sampling and blend): Clone Stamp, History Brush, and the Healing Brush's texture.
pub struct LiveRetouch {
    /// The active document with the stroke so far.
    pub doc: std::sync::Arc<Document>,
    cmd: String,
    renderer: photocraft_paint::StrokeRenderer,
    pre: std::sync::Arc<Document>,
    pre_surf: Surface,
    id: Option<LayerId>,
    params: Value,
    paint: LivePaint,
    mode: BlendMode,
    opacity: f32,
    sel: Option<Surface>,
    lock: bool,
    /// Where the doc shows what finishing the stroke now would add (a lone first dab, the
    /// smoothing catch-up), redrawn on every step as the Brush's live stroke does.
    tail: Rect,
    /// The document and revision the stroke started on.
    base: (photocraft_doc::DocId, u64),
    brush: Box<BrushSettings>,
    /// Every point rendered so far, in order.
    points: Vec<StrokePoint>,
}

impl LiveRetouch {
    /// Start a live stroke of `cmd` (`paint.cloneStamp`, `paint.healingBrush`,
    /// `paint.historyBrush` or an aligned, non-Impressionist `paint.patternStamp`) from its
    /// params; their `points` are rendered.
    pub fn begin(s: &Session, cmd: &str, p: &Value) -> Result<Self> {
        has_pixel_layer(s).map_err(EngineError::Other)?;
        if cmd == "paint.patternStamp" {
            let st = pattern_stamp::Setup::parse(s, p)?;
            if st.impressionist {
                return Err(bad(cmd, "Impressionist strokes are laid dab by dab (pattern_stamp::live_impressionist)"));
            }
            let paint = LivePaint::Pattern { place: st.placement(), tile: st.tile };
            return Self::with_paint(s, cmd, p, st.stroke, st.id, paint, st.mode);
        }
        let (stroke, id) = parse_brush(s, p, cmd)?;
        let mode = blend_param(p, cmd)?;
        let paint = match cmd {
            "paint.cloneStamp" | "paint.healingBrush" => {
                let f = *stroke.points.first().ok_or_else(|| bad(cmd, "`points` is empty"))?;
                LivePaint::Clone {
                    which: sample_layers(p, cmd)?,
                    map: crate::presets::clone_source::resolve(s, p, (f.x, f.y), cmd)?.0,
                    cmd: if cmd == "paint.healingBrush" { "paint.healingBrush" } else { "paint.cloneStamp" },
                }
            }
            "paint.historyBrush" => LivePaint::History(history_source(s, id, p, cmd)?),
            other => return Err(bad(other, "live retouching strokes are the Clone Stamp, Healing Brush, History Brush and Pattern Stamp")),
        };
        Self::with_paint(s, cmd, p, stroke, id, paint, mode)
    }

    /// Start a live stroke of `cmd` painting `paint` through `stroke`'s coverage.
    fn with_paint(s: &Session, cmd: &str, p: &Value, stroke: Stroke, id: Option<LayerId>, paint: LivePaint, mode: BlendMode) -> Result<Self> {
        let st = s.active().ok_or(EngineError::NoDocument)?;
        let (pre, base) = (st.doc.clone(), (st.doc.id, st.revision));
        let mut doc = (*pre).clone();
        let sel = doc.selection.clone();
        let (surf, lock) = crate::channel_cmds::target_surface(&mut doc, id, p)?;
        let pre_surf = surf.clone();
        // As `stroke_coverage`, which the commit uses.
        let renderer = photocraft_paint::StrokeRenderer::new(&stroke.brush, None, 1.0);
        let opacity = stroke.brush.opacity;
        let brush = Box::new(stroke.brush.clone());
        let paint = match paint {
            LivePaint::History(src) if src.format() != pre_surf.format() => LivePaint::History(src.convert(pre_surf.format())),
            other => other,
        };
        let mut live = Self {
            doc: std::sync::Arc::new(doc),
            cmd: cmd.into(),
            renderer,
            pre,
            pre_surf,
            id,
            params: p.clone(),
            paint,
            mode,
            opacity,
            sel,
            lock,
            tail: Rect::EMPTY,
            base,
            brush,
            points: Vec::new(),
        };
        live.push(&stroke.points)?;
        Ok(live)
    }

    /// Everything the stroke has touched so far.
    pub fn bounds(&self) -> Rect {
        let bounds = self.renderer.bounds().union(&self.tail);
        if self.cmd == "paint.cloneStamp" && flag(&self.params, "wrap", false) {
            periodic::project(bounds, self.pre.bounds()).into_iter().fold(Rect::EMPTY, |a, b| a.union(&b))
        } else {
            bounds
        }
    }

    /// Every point rendered so far, in order.
    pub fn points(&self) -> &[StrokePoint] {
        &self.points
    }

    /// The finished stroke as its commit would take it over ([`Session::offer_live_retouch`]).
    /// Only the Clone Stamp and History Brush draw what they commit (the Healing Brush blends on
    /// release), and only while the brush, layer and clone source are still the ones the stroke
    /// started with: the document revision doesn't cover those.
    pub(crate) fn into_commit(mut self, s: &Session) -> Result<crate::brush_cmds::LiveCommit> {
        let (stroke, id) = parse_brush(s, &self.params, &self.cmd)?;
        let source_unchanged = match &self.paint {
            LivePaint::Clone { map, .. } if self.cmd == "paint.cloneStamp" => {
                let f = stroke.points.first().ok_or_else(|| bad(&self.cmd, "`points` is empty"))?;
                crate::presets::clone_source::resolve(s, &self.params, (f.x, f.y), &self.cmd)?.0 == *map
            }
            LivePaint::History(_) if self.cmd == "paint.historyBrush" => true,
            _ => return Err(bad(&self.cmd, "only Clone Stamp and History Brush strokes commit their live pixels")),
        };
        if !source_unchanged || *self.brush != stroke.brush || id != self.id {
            return Err(bad(&self.cmd, "the brush, layer or clone source changed during the stroke"));
        }
        let bounds = self.bounds();
        let (surf, _) = crate::channel_cmds::target_surface(std::sync::Arc::make_mut(&mut self.doc), self.id, &self.params)?;
        let surface = surf.clone();
        Ok(crate::brush_cmds::LiveCommit { mixer: None, cmd: self.cmd, base: self.base, params: self.params, seed: None, points: self.points, surface, bounds })
    }

    /// Render more points; returns the rectangle that changed. Each changed area is redrawn from
    /// the pre-stroke pixels with the stroke's coverage so far, so overlapping dabs build up as in
    /// the commit, and pixels painted earlier in the stroke are never re-cloned. What finishing
    /// the stroke now would add is drawn too (a lone first dab shows on the press), and redrawn on
    /// every step.
    /// Invalid coordinates reject the whole batch without changing the preview.
    /// Only the position and pressure of `pts` are used: the commands take no tilt or times.
    pub fn push(&mut self, pts: &[StrokePoint]) -> Result<Rect> {
        crate::brush_cmds::check_coords(pts, &self.cmd)?;
        let pts: Vec<_> = pts.iter().map(|q| StrokePoint::new(q.x, q.y, q.pressure)).collect();
        self.points.extend_from_slice(&pts);
        self.renderer.push(&pts);
        if self.cmd == "paint.cloneStamp" && flag(&self.params, "wrap", false) {
            return self.push_periodic();
        }
        let old = std::mem::replace(&mut self.tail, Rect::EMPTY);
        let mut dmg = Rect::EMPTY;
        if !old.is_empty() {
            // Back to the stroke without the previous tail.
            let (surf, _) = crate::channel_cmds::target_surface(std::sync::Arc::make_mut(&mut self.doc), self.id, &self.params)?;
            surf.write_region(old, &self.pre_surf.read_region(old));
            self.renderer.mark_dirty(old);
            dmg = old;
        }
        let r = self.renderer.take_dirty_rect();
        let cov = self.renderer.coverage_in(r);
        dmg = dmg.union(&self.composite(r, &cov)?);
        if let Some(mut tail) = self.renderer.tail_preview() {
            let r = tail.take_dirty_rect();
            let cov = tail.coverage_in(r);
            self.tail = self.composite(r, &cov)?;
            dmg = dmg.union(&self.tail);
        }
        Ok(dmg)
    }

    fn push_periodic(&mut self) -> Result<Rect> {
        let mut dirty = self.renderer.take_dirty_rect().union(&self.tail);
        let mut tail = self.renderer.tail_preview();
        self.tail = tail.as_mut().map_or(Rect::EMPTY, |t| t.take_dirty_rect());
        dirty = dirty.union(&self.tail);
        let LivePaint::Clone { which, map, cmd } = &self.paint else {
            return Err(bad(&self.cmd, "periodic painting requires a clone source"));
        };
        let (surf, _) = crate::channel_cmds::target_surface(std::sync::Arc::make_mut(&mut self.doc), self.id, &self.params)?;
        periodic::Composite {
            target: surf,
            before: &self.pre_surf,
            tile: self.pre.bounds(),
            opacity: self.opacity,
            selection: self.sel.as_ref(),
            lock: self.lock,
            mode: self.mode,
        }
        .draw(dirty, &self.brush, &self.points, |r| clone_sample(&self.pre, self.id, &self.pre_surf, *which, r, map, true, cmd))
    }

    /// Redraw `r` from the pre-stroke pixels with the clone source through coverage `cov`.
    fn composite(&mut self, r: Rect, cov: &[f32]) -> Result<Rect> {
        if r.is_empty() {
            return Ok(r);
        }
        let paint = match &self.paint {
            LivePaint::Clone { which, map, cmd } => {
                clone_sample(&self.pre, self.id, &self.pre_surf, *which, r, map, *cmd == "paint.cloneStamp" && flag(&self.params, "wrap", false), cmd)?
            }
            LivePaint::History(src) => Region::read(src, r),
            LivePaint::Pattern { tile, place } => pattern_stamp::paint_region(tile, place, &self.pre_surf.format(), r),
        };
        let (surf, _) = crate::channel_cmds::target_surface(std::sync::Arc::make_mut(&mut self.doc), self.id, &self.params)?;
        surf.write_region(r, &self.pre_surf.read_region(r));
        Ok(apply_coverage(surf, r, cov, self.opacity, self.sel.as_ref(), self.lock, &paint, self.mode).union(&r))
    }
}
