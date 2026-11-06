//! Incremental Mixer Brush transaction. Source pixels are composed before each dab;
//! only the selected target changes. Dropping the transaction cancels it.
use super::{
    EngineError, LiveCommit, MixerSettings, Rect, Result, Session, Stroke, StrokePoint, Surface, Value, bad, check_coords, color, damage_json, doc_cmyk_space,
    flag, has_paintable, json, layer_id, num, parse_points, pct, resolve_brush,
};
use photocraft_color::{PixelFormat, SampleType};
use photocraft_doc::{Document, LayerId};
use photocraft_paint::dynamics::DabGenerator;
use photocraft_paint::mixer::{MixerState, apply_mixer_dab_region};
use photocraft_paint::retouch::{Region, mix_back};
use photocraft_paint::{BrushContext, Dab};
use std::collections::BTreeSet;
use std::sync::Arc;

const CMD: &str = "paint.mixerBrush";
const MAX_TILES: usize = 256;
/// The largest tip the live preview renders; larger valid tips render on release.
const PREVIEW_SIZE: f32 = 1024.0;
const TILE: i32 = 256;

/// Accept bounded finite channels, including HDR values, without admitting unsafe arithmetic inputs.
fn valid_color(color: &[f32; 4]) -> bool {
    color.iter().all(|v| v.is_finite() && v.abs() <= 1_000_000.0)
}

/// Validate finite paint-load values and the exact 64 by 64 shape of a spatial reservoir.
fn valid_state(state: &MixerState) -> bool {
    state.amount.is_finite()
        && state.reservoir.as_ref().is_none_or(valid_color)
        && state.pickup.as_ref().is_none_or(valid_color)
        && state.color.as_ref().is_none_or(valid_color)
        && state.sampled_over.as_ref().is_none_or(valid_color)
        && state.spatial.as_ref().is_none_or(|colors| colors.len() == 4096 && colors.iter().all(valid_color))
}

#[derive(Clone)]
struct Core {
    doc: Arc<Document>,
    generator: DabGenerator,
    ctx: BrushContext,
    pre: Surface,
    work: Surface,
    tiles: BTreeSet<(i32, i32)>,
    preview_limits: bool,
    state: MixerState,
    settings: MixerSettings,
    layer: Option<LayerId>,
    params: Value,
    selection: Option<Surface>,
    lock: bool,
    color_restrict: Option<crate::color_channels::Restriction>,
    count: usize,
    last: Option<StrokePoint>,
    bounds: Rect,
}

impl Core {
    /// Apply a batch against evolving working pixels and return its damage bounds.
    /// Dab, footprint and tile budgets apply only to live previews; the release replays without them.
    fn apply(&mut self, dabs: &[Dab]) -> Result<Rect> {
        if self.preview_limits && self.count.saturating_add(dabs.len()) > 250_000 {
            return Err(bad(CMD, "stroke exceeds 250000 dabs; start another stroke"));
        }
        let mut damage = Rect::EMPTY;
        for dab in dabs {
            let index = self.count;
            self.count += 1;
            let r = self.ctx.dab_rect(dab, false);
            if r.is_empty() {
                continue;
            }
            let full = self.ctx.dab_rect(dab, false);
            if self.preview_limits && u64::from(full.width()).saturating_mul(u64::from(full.height())) > 4_194_304 {
                return Err(bad(CMD, "Mixer Brush footprint exceeds 4194304 pixels"));
            }
            // Float working tiles avoid repeated integer-depth quantization. Initialize only
            // touched tiles, keeping unchanged source tiles shared by the document snapshots.
            for ty in r.y0.div_euclid(TILE)..=(r.y1 - 1).div_euclid(TILE) {
                for tx in r.x0.div_euclid(TILE)..=(r.x1 - 1).div_euclid(TILE) {
                    if !self.tiles.contains(&(tx, ty)) {
                        if self.preview_limits && self.tiles.len() >= MAX_TILES {
                            return Err(bad(CMD, "Mixer Brush stroke working set exceeds 256 tiles"));
                        }
                        let tile = Rect::new(tx * TILE, ty * TILE, (tx + 1) * TILE, (ty + 1) * TILE);
                        self.work.write_region(tile, &self.pre.read_region(tile));
                        self.tiles.insert((tx, ty));
                    }
                }
            }
            // Render the visible document including the active layer exactly once, before
            // deposition. Masks, transforms, blend modes and adjustments use the compositor.
            let sample = (self.settings.sample_all_layers && self.settings.samples_canvas(dab)).then(|| {
                let buf = photocraft_compose::render(&self.doc, r);
                let fmt = self.pre.format();
                let mut sample = Region::new(r, fmt.channels());
                for (src, dst) in buf.px.iter().zip(sample.data.chunks_exact_mut(sample.ch)) {
                    photocraft_raster::from_rgba_into(&fmt, *src, dst);
                }
                sample
            });
            let fp = self.ctx.footprint(dab, index);
            let mut region = Region::read(&self.work, r);
            apply_mixer_dab_region(&mut region, &fp, self.pre.format(), sample.as_ref(), &self.settings, &mut self.state, self.lock);
            region.write(&mut self.work);
            let original = Region::read(&self.pre, r);
            let (target, _) = crate::channel_cmds::target_surface(Arc::make_mut(&mut self.doc), self.layer, &self.params)?;
            mix_back(target, &original, &region, r, 1.0, self.selection.as_ref(), self.lock);
            if let Some(restrict) = self.color_restrict {
                crate::color_channels::restrict_region(&self.pre, target, restrict.colors, r)?;
            }
            damage = damage.union(&r);
        }
        self.bounds = self.bounds.union(&damage);
        Ok(damage)
    }
}

/// A real, cancellable paint result, displayed by the existing damage-aware canvas.
pub struct LiveMixer {
    pub doc: Arc<Document>,
    pub seed: u64,
    core: Core,
    state: MixerState,
    tail: Rect,
    cmyk: Option<Arc<photocraft_color::convert::CmykSpace>>,
    original: Arc<Document>,
    points: Vec<StrokePoint>,
    original_state: MixerState,
}

impl LiveMixer {
    /// Capture the current document and paint load, then render the initial samples with live-preview limits.
    pub fn begin(s: &Session, p: &Value) -> Result<Self> {
        Self::begin_with_limits(s, p, true)
    }

    /// Share initialization between live previews and command replay while keeping their tile budgets distinct.
    fn begin_with_limits(s: &Session, p: &Value, preview_limits: bool) -> Result<Self> {
        let (mut live, points) = Self::prepare(s, p, preview_limits)?;
        live.push(&points)?;
        Ok(live)
    }

    /// Whether a stroke the live preview refuses only for its tip size still paints on release.
    /// `Err` when the stroke can't be painted at all (locked layer, bad params...).
    pub fn defers_to_release(s: &Session, p: &Value) -> Result<bool> {
        let (live, _) = Self::prepare(s, p, false)?;
        Ok(live.core.ctx.brush.size > PREVIEW_SIZE)
    }

    /// Validate the params and capture the document and paint load, without rendering a dab.
    fn prepare(s: &Session, p: &Value, preview_limits: bool) -> Result<(Self, Vec<StrokePoint>)> {
        let captured = crate::channel_cmds::inject_target(s, CMD, p.clone());
        let p = &captured;
        has_paintable(s).map_err(EngineError::Other)?;
        if preview_limits && p.get("points").and_then(Value::as_array).is_some_and(|v| v.len() > 65_536) {
            return Err(bad(CMD, "stroke exceeds 65536 recorded input samples"));
        }
        let points = parse_points(p, CMD)?;
        let brush = resolve_brush(s, p, CMD)?;
        let bm = &brush.mixer;
        let settings = MixerSettings {
            wet: pct(p, "wet", bm.wet * 100.0),
            load: pct(p, "load", bm.load * 100.0),
            mix: pct(p, "mix", bm.mix * 100.0),
            flow: pct(p, "flow", bm.flow * 100.0),
            sample_all_layers: flag(p, "sampleAllLayers", bm.sample_all_layers) && crate::channel_cmds::target_of(p) == crate::channel_cmds::Target::Pixels,
            clean_after_stroke: flag(p, "cleanAfterStroke", bm.clean_after_stroke),
            load_after_stroke: flag(p, "loadAfterStroke", bm.load_after_stroke),
        };
        let initial_state = if let Some(value) = p.get("mixerState") {
            if value.get("spatial").and_then(Value::as_array).is_some_and(|v| v.len() != 4096) {
                return Err(bad(CMD, "spatial reservoir must contain exactly 4096 colors"));
            }
            serde_json::from_value::<MixerState>(value.clone()).map_err(|e| bad(CMD, e.to_string()))?
        } else {
            s.tools.mixer.clone()
        };
        let mut state = initial_state;
        if !valid_state(&state) {
            return Err(bad(CMD, "loaded paint must be finite and color magnitudes at most 1000000"));
        }
        let paint = color(p.get("color"), s.tools.foreground);
        if !valid_color(&paint) {
            return Err(bad(CMD, "loaded color must be finite and magnitudes at most 1000000"));
        }
        // Earlier captured replay states had no foreground marker. Keep their loaded paint.
        if p.get("mixerState").is_some_and(|v| v.get("color").is_none()) && (state.reservoir.is_some() || state.spatial.is_some()) {
            state.color = Some(paint);
        }
        state.begin_stroke(paint);
        let layer = if crate::channel_cmds::is_channel_target(p) { None } else { Some(layer_id(s, p)?) };
        let mut doc = (*s.active().ok_or(EngineError::NoDocument)?.doc).clone();
        let selection = doc.selection.clone();
        let (target, lock) = crate::channel_cmds::target_surface(&mut doc, layer, p)?;
        let pre = target.clone();
        let work = Surface::new(PixelFormat { sample: SampleType::F32, ..pre.format() });
        let core = Core {
            doc: Arc::new(doc),
            generator: DabGenerator::new(&brush, num(p, "zoom").unwrap_or(1.0)),
            ctx: BrushContext::new(&brush),
            pre,
            work,
            tiles: BTreeSet::new(),
            preview_limits,
            state: state.clone(),
            settings,
            layer,
            params: p.clone(),
            selection,
            lock,
            color_restrict: crate::channel_cmds::color_restriction(s, CMD, p),
            count: 0,
            last: None,
            bounds: Rect::EMPTY,
        };
        // Checked last, so a tip that is too big is the only reason left (`defers_to_release`),
        // and before `push` allocates a dense brush footprint.
        if preview_limits && core.ctx.brush.size > PREVIEW_SIZE {
            return Err(bad(CMD, "the live Mixer Brush preview is limited to 1024 px tips"));
        }
        let live = Self {
            doc: core.doc.clone(),
            seed: brush.seed,
            core,
            state,
            tail: Rect::EMPTY,
            cmyk: doc_cmyk_space(s),
            original: s.active().ok_or(EngineError::NoDocument)?.doc.clone(),
            points: Vec::new(),
            original_state: s.tools.mixer.clone(),
        };
        Ok((live, points))
    }

    /// Return all damaged pixels, including the provisional smoothing tail currently shown by the preview.
    pub fn bounds(&self) -> Rect {
        self.core.bounds.union(&self.tail)
    }
    /// Whether the dab generator needs stationary time samples, as for a continuing airbrush stroke.
    pub fn wants_time(&self) -> bool {
        self.core.generator.wants_time()
    }

    /// Batches and individual input samples produce identical results. The speculative end
    /// dab / smoothing tail is displayed but never fed into the next accepted input.
    pub fn push(&mut self, points: &[StrokePoint]) -> Result<Rect> {
        check_coords(points, CMD)?;
        if self.core.preview_limits && self.points.len().saturating_add(points.len()) > 65_536 {
            return Err(bad(CMD, "stroke exceeds 65536 recorded input samples"));
        }
        let cmyk = self.cmyk.clone();
        photocraft_color::convert::with_cmyk_space(cmyk.as_ref(), || {
            let old_tail = self.tail;
            let mut damage = Rect::EMPTY;
            for point in points {
                // Bound each temporary dab batch without rejecting sparse mouse events or
                // long Shift-click lines. Live and recorded replay use identical interpolation.
                let start = self.core.last.unwrap_or(*point);
                let steps = ((point.x - start.x).hypot(point.y - start.y) / 2048.0).ceil().max(1.0) as usize;
                for step in 1..=steps {
                    let f = step as f64 / steps as f64;
                    let ff = f as f32;
                    let q = if step == steps {
                        *point
                    } else {
                        StrokePoint {
                            x: start.x + (point.x - start.x) * f,
                            y: start.y + (point.y - start.y) * f,
                            pressure: start.pressure + (point.pressure - start.pressure) * ff,
                            tilt_x: start.tilt_x + (point.tilt_x - start.tilt_x) * ff,
                            tilt_y: start.tilt_y + (point.tilt_y - start.tilt_y) * ff,
                            rotation: start.rotation + (point.rotation - start.rotation) * ff,
                            wheel: start.wheel + (point.wheel - start.wheel) * ff,
                            time: start.time + (point.time - start.time) * f,
                        }
                    };
                    let (mut dabs, mut dual) = (Vec::new(), Vec::new());
                    self.core.generator.push(&[q], &mut dabs, &mut dual);
                    damage = damage.union(&self.core.apply(&dabs)?);
                }
                self.core.last = Some(*point);
            }
            let mut finished = self.core.clone();
            let (mut dabs, mut dual) = (Vec::new(), Vec::new());
            finished.generator.finish(&mut dabs, &mut dual);
            self.tail = finished.apply(&dabs)?;
            self.doc = finished.doc;
            self.state = finished.state;
            self.points.extend_from_slice(points);
            Ok(damage.union(&old_tail).union(&self.tail))
        })
    }

    /// Keep the same compact journal params as other paint tools: UI settings, points and seed.
    fn commit_params(&self) -> Result<Value> {
        let mut params = self.core.params.clone();
        params["points"] = serde_json::to_value(&self.points).map_err(|e| bad(CMD, e.to_string()))?;
        params["seed"] = json!(self.seed);
        Ok(params)
    }

    /// Stage the already rendered pixels for the ordinary journaled command. Refuse stale
    /// document/tool state. The command consumes this single-use cache only for identical params.
    pub fn prepare_commit(self, session: &mut Session) -> Result<Value> {
        if !session.active().is_some_and(|st| Arc::ptr_eq(&st.doc, &self.original) && (self.core.layer.is_none() || st.active_layer == self.core.layer))
            || session.tools.mixer != self.original_state
        {
            return Err(bad(CMD, "document or loaded paint changed during the stroke"));
        }
        let params = self.commit_params()?;
        let base = session.active().map(|st| (st.doc.id, st.revision)).ok_or(EngineError::NoDocument)?;
        let bounds = self.bounds();
        let mut doc = (*self.doc).clone();
        let surface = crate::channel_cmds::target_surface(&mut doc, self.core.layer, &params)?.0.clone();
        let mut after = self.state;
        after.end_stroke(self.core.ctx.brush.color, self.core.settings.clean_after_stroke, self.core.settings.load_after_stroke);
        session.live_commit = Some(LiveCommit {
            cmd: CMD.into(),
            base,
            params: params.clone(),
            seed: Some(self.seed),
            points: self.points,
            surface,
            bounds,
            mixer: Some((self.original_state, after)),
        });
        Ok(params)
    }
}

/// Consume a matching prepared result or replay without the preview tile cap, then commit one history step.
/// The completed stroke also applies the automatic clean/load settings to the stored paint load.
pub(super) fn run(s: &mut Session, p: &Value) -> Result<Value> {
    let points = parse_points(p, CMD)?;
    let cached =
        s.live_commit.take().filter(|live| live.matches(s, CMD, p, &points) && live.mixer.as_ref().is_some_and(|(before, _)| *before == s.tools.mixer));
    if let Some(live) = cached {
        let id = if crate::channel_cmds::is_channel_target(p) { None } else { Some(layer_id(s, p)?) };
        s.edit("Mixer Brush", |doc, _| {
            *crate::channel_cmds::target_surface(doc, id, p)?.0 = live.surface;
            Ok(())
        })?;
        if let Some((_, after)) = live.mixer {
            s.tools.mixer = after;
        }
        let mut result = damage_json(s, live.bounds);
        result["fromLive"] = json!(true);
        return Ok(result);
    }
    let live = LiveMixer::begin_with_limits(s, p, false)?;
    let damage = live.bounds();
    s.edit("Mixer Brush", |doc, _| {
        *doc = (*live.doc).clone();
        Ok(())
    })?;
    let mut state = live.state;
    let paint = live.core.ctx.brush.color;
    state.end_stroke(paint, live.core.settings.clean_after_stroke, live.core.settings.load_after_stroke);
    s.tools.mixer = state;
    Ok(damage_json(s, damage))
}

/// Sampling changes only session tool state, never document pixels or document Undo.
pub(super) fn sample(s: &mut Session, p: &Value) -> Result<Value> {
    if p.get("points").and_then(Value::as_array).is_some_and(|v| v.len() != 1) {
        return Err(bad("brush.mixer.sample", "exactly one sample point is required"));
    }
    let points = parse_points(p, "brush.mixer.sample")?;
    if points.len() != 1 {
        return Err(bad("brush.mixer.sample", "exactly one sample point is required"));
    }
    let mut brush = resolve_brush(s, p, "brush.mixer.sample")?;
    if brush.size > 1024.0 {
        return Err(bad("brush.mixer.sample", "sampling size is limited to 1024 pixels"));
    }
    brush.flow = 1.0;
    let dabs = photocraft_paint::dabs(&Stroke { brush: brush.clone(), points });
    let dab = dabs.first().ok_or_else(|| bad("brush.mixer.sample", "brush has no contact"))?;
    let ctx = BrushContext::new(&brush);
    let rect = ctx.dab_rect(dab, false);
    if u64::from(rect.width()).saturating_mul(u64::from(rect.height())) > 4_194_304 {
        return Err(bad("brush.mixer.sample", "sampling footprint is too large"));
    }
    let fp = ctx.footprint(dab, 0);
    let st = s.active().ok_or(EngineError::NoDocument)?;
    let mut doc = crate::float_cmds::displayed(st, (0, 0)).unwrap_or_else(|| (*st.doc).clone());
    let id = if crate::channel_cmds::is_channel_target(p) { None } else { Some(layer_id(s, p)?) };
    let cmyk = doc_cmyk_space(s);
    let load = photocraft_color::convert::with_cmyk_space(cmyk.as_ref(), || -> Result<Vec<[f32; 4]>> {
        let surface = if flag(p, "sampleAllLayers", brush.mixer.sample_all_layers) && crate::channel_cmds::target_of(p) == crate::channel_cmds::Target::Pixels {
            let buf = photocraft_compose::render(&doc, rect);
            let mut surface = Surface::new(PixelFormat::RGBA32F);
            surface.write_region(rect, &buf.px.iter().flatten().copied().collect::<Vec<_>>());
            surface
        } else if crate::channel_cmds::target_of(p) == crate::channel_cmds::Target::Pixels {
            id.and_then(|id| doc.layer(id))
                .and_then(|layer| layer.surface())
                .cloned()
                .ok_or_else(|| bad("brush.mixer.sample", "current layer has no raster pixels to sample"))?
        } else {
            crate::channel_cmds::target_surface(&mut doc, id, p)?.0.clone()
        };
        let mut load = Vec::with_capacity(4096);
        let (sin, cos) = dab.angle.sin_cos();
        for row in 0..64 {
            for col in 0..64 {
                let mut u = ((col as f32 + 0.5) / 32.0 - 1.0) * dab.radius;
                let mut v = ((row as f32 + 0.5) / 32.0 - 1.0) * (dab.radius * dab.roundness).max(0.5);
                if dab.flip_x {
                    u = -u;
                }
                if dab.flip_y {
                    v = -v;
                }
                let (mut dx, mut up) = (u * cos + v * sin, u * sin - v * cos);
                if dab.proj_scale < 0.999 {
                    let (sin, cos) = dab.proj_angle.sin_cos();
                    let along = (dx * cos + up * sin) * (dab.proj_scale.max(0.05) - 1.0);
                    dx += along * cos;
                    up += along * sin;
                }
                let x = (dab.center.x as f32 + dx).floor() as i32;
                let y = (dab.center.y as f32 - up).floor() as i32;
                let mut color = surface.rgba(x, y);
                let coverage = if x >= fp.rect.x0 && x < fp.rect.x1 && y >= fp.rect.y0 && y < fp.rect.y1 { fp.at(x, y) } else { 0.0 };
                color[3] *= coverage.clamp(0.0, 1.0);
                load.push(color);
            }
        }
        Ok(load)
    })?;
    let mut average = [0.0f32; 4];
    for c in &load {
        for k in 0..3 {
            average[k] += c[k] * c[3];
        }
        average[3] += c[3];
    }
    if average[3] <= 0.0 {
        return Err(bad("brush.mixer.sample", "the sampled area contains no paint; the previous brush load is unchanged"));
    }
    for k in 0..3 {
        average[k] /= average[3];
    }
    average[3] = 1.0;
    s.tools.mixer.load(average);
    s.tools.mixer.spatial = Some(load);
    s.tools.mixer.pickup = None;
    s.tools.mixer.color = Some(s.tools.foreground);
    Ok(json!({"width":64,"height":64,"color":average}))
}
