use super::sampling::{AllLayers, sample_all_layers};
use super::*;

// ---------------------------------------------------------------------------------------------
// Toning (Dodge / Burn / Sponge) and focus (Blur / Sharpen), Smudge
// ---------------------------------------------------------------------------------------------

/// Per-dab strength such that one pass over a point totals roughly `e`, whatever the spacing
/// (about `1/spacing` dabs overlap each point on the stroke's centre line).
#[inline]
fn per_dab(e: f32, spacing: f32) -> f32 {
    1.0 - (1.0 - e.clamp(0.0, 1.0)).powf(spacing.clamp(0.01, 1.0))
}

/// Apply an RGB colour transform `f(rgb, strength)` to each covered pixel of a dab (through straight
/// RGBA, so it works for Gray/CMYK/Lab too; untouched pixels are never round-tripped).
fn color_dab(fmt: &PixelFormat, work: &mut Region, fp: &Footprint, strength: f32, spacing: f32, f: impl Fn([f32; 3], f32) -> [f32; 3]) {
    let r = fp.rect.intersect(&work.rect);
    let n = fmt.channels();
    let mut enc = [0.0f32; 8];
    for y in r.y0..r.y1 {
        for x in r.x0..r.x1 {
            let k = per_dab(fp.at(x, y) * strength, spacing);
            if k <= 0.0 {
                continue;
            }
            let px = work.px_mut(x, y);
            let rgba = to_rgba(fmt, px);
            let o = f([rgba[0], rgba[1], rgba[2]], k);
            from_rgba_into(fmt, [o[0], o[1], o[2], rgba[3]], &mut enc);
            let a = alpha_index(fmt);
            for c in 0..n {
                if Some(c) != a {
                    px[c] = enc[c];
                }
            }
        }
    }
}

/// Dodge and Burn: a stroke tones each pixel once, from its original colour, mixed in by the
/// stroke's coverage there (`toned` is the full-exposure colour). Dabs build up coverage as in a
/// brush stroke, capped at full coverage, so Exposure caps a stroke the way Opacity caps a paint
/// stroke: overlapping dabs and passes within one stroke don't compound the tone curve, and a
/// new stroke builds on the last.
fn tone_stroke(fmt: PixelFormat, toned: impl Fn([f32; 3]) -> [f32; 3] + 'static) -> DabEffect {
    /// Side of the cells that remember, per touched pixel, the coverage applied so far and the
    /// original pixel (copied from the working pixels the first time a dab reaches it). Dense
    /// cells rather than a map per pixel: a map lookup per covered pixel made long strokes with
    /// large brushes over ten times slower.
    const CELL: i32 = 64;
    struct Cell {
        cov: Vec<f32>,
        orig: Vec<f32>,
    }
    let mut cells: std::collections::HashMap<(i32, i32), Cell> = std::collections::HashMap::new();
    let a = alpha_index(&fmt);
    let n = fmt.channels();
    Box::new(move |work, fp| {
        let r = fp.rect.intersect(&work.rect);
        if r.is_empty() {
            return;
        }
        let mut enc = [0.0f32; 8];
        for cy in r.y0.div_euclid(CELL)..=(r.y1 - 1).div_euclid(CELL) {
            for cx in r.x0.div_euclid(CELL)..=(r.x1 - 1).div_euclid(CELL) {
                let cr = Rect::new(cx * CELL, cy * CELL, cx * CELL + CELL, cy * CELL + CELL);
                let part = r.intersect(&cr);
                if part.is_empty() {
                    continue;
                }
                let len = (CELL * CELL) as usize;
                // Coverage -1: not touched yet, so `orig` isn't filled in for that pixel.
                let cell = cells.entry((cx, cy)).or_insert_with(|| Cell { cov: vec![-1.0; len], orig: vec![0.0; len * n] });
                for y in part.y0..part.y1 {
                    for x in part.x0..part.x1 {
                        let k = fp.at(x, y).clamp(0.0, 1.0);
                        if k <= 0.0 {
                            continue;
                        }
                        let i = ((y - cr.y0) * CELL + (x - cr.x0)) as usize;
                        let (Some(prev), Some(src)) = (cell.cov.get_mut(i), cell.orig.get_mut(i * n..i * n + n)) else { continue };
                        if *prev < 0.0 {
                            // First touch: the working pixel is still the original.
                            src.copy_from_slice(work.px(x, y));
                            *prev = 0.0;
                        }
                        // Dabs build up the stroke's coverage as a brush stroke's do (flow), up
                        // to full coverage; the tone is applied once, from the original.
                        let k = *prev + k * (1.0 - *prev);
                        if k <= *prev {
                            continue;
                        }
                        *prev = k;
                        let o = to_rgba(&fmt, src);
                        let t = toned([o[0], o[1], o[2]]);
                        let m = [o[0] + (t[0] - o[0]) * k, o[1] + (t[1] - o[1]) * k, o[2] + (t[2] - o[2]) * k];
                        from_rgba_into(&fmt, [m[0], m[1], m[2], o[3]], &mut enc);
                        let px = work.px_mut(x, y);
                        for c in 0..n {
                            if Some(c) != a {
                                px[c] = enc[c];
                            }
                        }
                    }
                }
            }
        }
    })
}

fn tone_range(p: &Value, cmd: &str) -> Result<ToneRange> {
    match string(p, "range", "midtones") {
        "shadows" => Ok(ToneRange::Shadows),
        "midtones" => Ok(ToneRange::Midtones),
        "highlights" => Ok(ToneRange::Highlights),
        o => Err(bad(cmd, format!("unknown range `{o}` (shadows|midtones|highlights)"))),
    }
}

/// A sequential-dab tool's settings (Dodge, Burn, Sponge, Blur, Sharpen, Smudge), parsed once and
/// turned into its per-dab effect, for the commit and the live preview alike.
#[derive(Clone, Debug, PartialEq)]
enum DabTool {
    Tone { burn: bool, range: ToneRange, exposure: f32, protect: bool },
    Sponge { saturate: bool, vibrance: bool },
    Focus { sharpen: bool, strength: f32, protect: bool, sigma: f32 },
    Smudge { strength: f32, finger: Option<[f32; 4]>, max_size: f32 },
}

/// One dab's edit of the working copy.
pub(crate) type DabEffect = Box<dyn FnMut(&mut Region, &Footprint)>;

/// Pixels an effect reads beyond the dab it writes. Smudge's pickup is a fixed-size square,
/// independent of pressure, roundness and the dab's current angle.
#[derive(Clone, Copy)]
enum DabNeighborhood {
    Halo(i32),
    Smudge(i32),
}

impl DabNeighborhood {
    fn halo(self) -> i32 {
        match self {
            Self::Halo(radius) | Self::Smudge(radius) => radius,
        }
    }

    fn read_rect(self, rect: Rect, dab: &photocraft_paint::Dab) -> Rect {
        match self {
            Self::Halo(radius) => rect.inflate(radius),
            Self::Smudge(radius) => {
                let (x, y) = (dab.center.x.floor() as i32, dab.center.y.floor() as i32);
                rect.union(&Rect::new(
                    x.saturating_sub(radius),
                    y.saturating_sub(radius),
                    x.saturating_add(radius).saturating_add(1),
                    y.saturating_add(radius).saturating_add(1),
                ))
            }
        }
    }
}

impl DabTool {
    /// The tool for `cmd` and its history label.
    fn parse(s: &Session, cmd: &str, p: &Value, stroke: &Stroke) -> Result<(Self, &'static str)> {
        Ok(match cmd {
            "paint.dodge" | "paint.burn" => {
                let burn = cmd == "paint.burn";
                let tool = Self::Tone {
                    burn,
                    range: tone_range(p, cmd)?,
                    exposure: num(p, "exposure", 50.0).clamp(1.0, 100.0) / 100.0,
                    protect: flag(p, "protectTones", true),
                };
                (tool, if burn { "Burn Tool" } else { "Dodge Tool" })
            }
            "paint.sponge" => {
                let saturate = match string(p, "mode", "desaturate") {
                    "desaturate" => false,
                    "saturate" => true,
                    o => return Err(bad(cmd, format!("unknown mode `{o}` (desaturate|saturate)"))),
                };
                (Self::Sponge { saturate, vibrance: flag(p, "vibrance", true) }, "Sponge Tool")
            }
            "paint.blur" | "paint.sharpen" => {
                let sharpen = cmd == "paint.sharpen";
                let tool = Self::Focus {
                    sharpen,
                    strength: num(p, "strength", 50.0).clamp(1.0, 100.0) / 100.0,
                    protect: flag(p, "protectDetail", true),
                    sigma: (stroke.brush.size * 0.03).clamp(1.0, 4.0),
                };
                (tool, if sharpen { "Sharpen Tool" } else { "Blur Tool" })
            }
            "paint.smudge" => {
                let finger = flag(p, "fingerPainting", false).then_some(s.tools.foreground);
                (Self::Smudge { strength: num(p, "strength", 50.0).clamp(1.0, 100.0) / 100.0, finger, max_size: stroke.brush.size }, "Smudge Tool")
            }
            o => return Err(bad(o, "not a sequential-dab tool")),
        })
    }

    /// How far around each dab the effect reads (Blur and Sharpen read a neighbourhood).
    fn neighborhood(&self) -> DabNeighborhood {
        match self {
            Self::Focus { sigma, .. } => DabNeighborhood::Halo((sigma * 3.0).ceil() as i32 + 1),
            Self::Smudge { max_size, .. } => DabNeighborhood::Smudge(Smudge::sampling_radius(*max_size)),
            _ => DabNeighborhood::Halo(0),
        }
    }

    /// The per-dab effect on a working copy in `fmt`, for a brush of `spacing`.
    fn effect(&self, fmt: PixelFormat, spacing: f32) -> DabEffect {
        let sp = spacing;
        match self.clone() {
            Self::Tone { burn, range, exposure, protect } => tone_stroke(fmt, move |c| dodge_burn(c, exposure, range, burn, protect)),
            // Flow is already in each dab's coverage; a full-flow pass moves colours half-way.
            Self::Sponge { saturate, vibrance } => Box::new(move |work, fp| color_dab(&fmt, work, fp, 0.5, sp, |c, k| sponge(c, k, saturate, vibrance))),
            Self::Focus { sharpen, strength, protect, sigma } => {
                let a = alpha_index(&fmt);
                Box::new(move |work, fp| {
                    let r = fp.rect.intersect(&work.rect);
                    if r.is_empty() {
                        return;
                    }
                    let (w, h, n) = (work.width(), work.height(), work.ch);
                    let win = ((r.x0 - work.rect.x0) as usize, (r.y0 - work.rect.y0) as usize, (r.x1 - work.rect.x0) as usize, (r.y1 - work.rect.y0) as usize);
                    let out = if sharpen { local_sharpen(&work.data, w, h, n, a, win, 1.0, protect) } else { local_blur(&work.data, w, h, n, a, win, sigma) };
                    let rw = r.width() as usize;
                    // Row-parallel, like the blur itself (#5426).
                    let (wx0, wy0) = (work.rect.x0, work.rect.y0);
                    work.data.par_chunks_mut(w * n).enumerate().for_each(|(row, line)| {
                        let y = wy0 + row as i32;
                        if y < r.y0 || y >= r.y1 {
                            return;
                        }
                        for x in r.x0..r.x1 {
                            let k = per_dab(fp.at(x, y) * strength, sp);
                            if k <= 0.0 {
                                continue;
                            }
                            let o = ((y - r.y0) as usize * rw + (x - r.x0) as usize) * n;
                            let i = (x - wx0) as usize * n;
                            let px = &mut line[i..i + n];
                            for c in 0..n {
                                px[c] += (out[o + c] - px[c]) * k;
                            }
                        }
                    });
                })
            }
            Self::Smudge { strength, finger, max_size } => {
                let finger_px = finger.map(|fg| photocraft_raster::from_rgba(&fmt, fg));
                // Premultiplied mixing: transparent pixels carry no colour, so no dark fringes.
                let mut sm = Smudge::new(strength, finger_px, max_size).with_alpha(alpha_index(&fmt));
                Box::new(move |work, fp| sm.dab(work, fp))
            }
        }
    }
}

/// Dodge, Burn, Sponge, Blur, Sharpen and Smudge: each dab edits a working copy in order
/// (`apply_dab_stroke`). Blur, Sharpen and Smudge can sample all layers.
pub(super) fn dab_cmd(s: &mut Session, p: &Value, cmd: &str) -> Result<Value> {
    let (stroke, id) = parse_brush(s, p, cmd)?;
    let (tool, label) = DabTool::parse(s, cmd, p, &stroke)?;
    let all = matches!(tool, DabTool::Focus { .. } | DabTool::Smudge { .. }) && sample_all_layers(p);
    // The live stroke the canvas already drew for exactly this commit (`LiveDab::into_commit`):
    // take its pixels instead of running every dab again (the hitch on release).
    let live = s.live_commit.take().filter(|l| l.matches(s, cmd, p, &stroke.points));
    let from_live = live.is_some();
    let dmg = run_stroke(s, label, id, p, |pre, surf, sel, lock| {
        if let Some(l) = live {
            *surf = l.surface;
            return Ok(l.bounds);
        }
        let fmt = surf.format();
        let mut effect = tool.effect(fmt, stroke.brush.spacing);
        let mut sampler = all.then(|| AllLayers::new(pre, fmt));
        Ok(apply_dab_stroke(surf, &stroke, sel, lock, tool.neighborhood().halo(), |work, fp| match &mut sampler {
            Some(all) => all.dab(work, fp, |w, f| effect(w, f)),
            None => effect(work, fp),
        }))
    })?;
    let mut r = json!({ "damage": damage_json(dmg) });
    if from_live {
        r["fromLive"] = json!(true);
    }
    Ok(r)
}

/// Side of the tiles a live sequential-dab stroke keeps its working pixels in ([`WorkTiles`]).
const WORK_TILE: i32 = 128;

/// The working copy of a live sequential-dab stroke: the target's native channels as `f32`, in
/// tiles copied from the pre-stroke surface the first time a dab writes to them. Unlike a
/// `Surface`, it isn't rounded to the document's depth between dabs, so each dab sees what the
/// commit's working copy (`apply_dab_stroke`) holds and the preview is the commit's pixels.
struct WorkTiles {
    ch: usize,
    tiles: std::collections::HashMap<(i32, i32), Vec<f32>>,
}

impl WorkTiles {
    fn new(ch: usize) -> Self {
        Self { ch, tiles: std::collections::HashMap::new() }
    }

    fn tile_rect((tx, ty): (i32, i32)) -> Rect {
        Rect::new(tx * WORK_TILE, ty * WORK_TILE, tx * WORK_TILE + WORK_TILE, ty * WORK_TILE + WORK_TILE)
    }

    /// The tiles `r` overlaps.
    fn coords(r: Rect) -> impl Iterator<Item = (i32, i32)> {
        let (x0, x1) = (r.x0.div_euclid(WORK_TILE), (r.x1 - 1).div_euclid(WORK_TILE));
        let (y0, y1) = (r.y0.div_euclid(WORK_TILE), (r.y1 - 1).div_euclid(WORK_TILE));
        let empty = r.is_empty();
        (y0..=y1).filter(move |_| !empty).flat_map(move |ty| (x0..=x1).map(move |tx| (tx, ty)))
    }

    /// The working pixels of `r`: the pre-stroke pixels `pre` where no dab has written yet.
    fn read(&self, pre: &Surface, r: Rect) -> Region {
        let mut out = Region::new(r, self.ch);
        let n = self.ch;
        let mut buf = Vec::new();
        for c in Self::coords(r) {
            let tr = Self::tile_rect(c);
            let part = r.intersect(&tr);
            let w = part.width() as usize * n;
            let tile = self.tiles.get(&c);
            if tile.is_none() {
                pre.read_region_into(part, &mut buf);
            }
            for y in part.y0..part.y1 {
                let src = match tile {
                    Some(t) => t.get(((y - tr.y0) * WORK_TILE + (part.x0 - tr.x0)) as usize * n..).and_then(|s| s.get(..w)),
                    None => buf.get((y - part.y0) as usize * w..).and_then(|s| s.get(..w)),
                };
                let dst = out.index(part.x0, y) * n;
                if let (Some(s), Some(d)) = (src, out.data.get_mut(dst..dst + w)) {
                    d.copy_from_slice(s);
                }
            }
        }
        out
    }

    /// Store `region` as the working pixels of its rectangle.
    fn write(&mut self, pre: &Surface, region: &Region) {
        let (r, n) = (region.rect, self.ch);
        for c in Self::coords(r) {
            let tr = Self::tile_rect(c);
            let tile = self.tiles.entry(c).or_insert_with(|| pre.read_region(tr));
            let part = r.intersect(&tr);
            let w = part.width() as usize * n;
            for y in part.y0..part.y1 {
                let dst = ((y - tr.y0) * WORK_TILE + (part.x0 - tr.x0)) as usize * n;
                let src = region.index(part.x0, y) * n;
                if let (Some(s), Some(d)) = (region.data.get(src..src + w), tile.get_mut(dst..dst + w)) {
                    d.copy_from_slice(s);
                }
            }
        }
    }
}

/// A Dodge, Burn, Sponge, Blur, Sharpen or Smudge stroke shown while it is drawn: each new dab
/// runs the tool's effect on a working copy of the target, in order, and the changed area is
/// mixed back over the original at the brush opacity, as `apply_dab_stroke` does on release.
/// The working copy keeps full precision ([`WorkTiles`]), so the pixels are the commit's, and the
/// commit takes them over ([`Session::offer_live_dab`]) instead of running every dab again.
/// Sample All Layers isn't previewed (its composite spans the whole stroke).
pub struct LiveDab {
    /// The active document with the stroke so far.
    pub doc: std::sync::Arc<Document>,
    cmd: String,
    renderer: photocraft_paint::StrokeRenderer,
    done: usize,
    effect: DabEffect,
    neighborhood: DabNeighborhood,
    pre_surf: Surface,
    work: WorkTiles,
    id: Option<LayerId>,
    params: Value,
    opacity: f32,
    sel: Option<Surface>,
    lock: bool,
    bounds: Rect,
    /// Show the dabs finishing the stroke would add, so the preview at the last point is the
    /// commit. Only for effects that a repeated dab doesn't change (Color Replacement): the
    /// tail runs on a scratch copy at every push, so an effect that carries paint from dab to
    /// dab (Smudge) would drift.
    preview_tail: bool,
    /// Where the shown tail is.
    tail: Rect,
    /// The tool and brush the stroke started with, which its commit must still have: the
    /// document revision doesn't cover the session's brush or foreground colour. `None` for
    /// strokes whose commit renders itself (Color Replacement).
    tool: Option<(DabTool, Box<BrushSettings>)>,
    /// The document and revision the stroke started on, for strokes started from the session's
    /// active document ([`Self::with_effect`]); `None` for strokes on a prepared copy.
    base: Option<(photocraft_doc::DocId, u64)>,
    /// Every point rendered so far, in order.
    points: Vec<StrokePoint>,
}

impl LiveDab {
    /// Start a live stroke of `cmd` from its params; their `points` are rendered.
    pub fn begin(s: &Session, cmd: &str, p: &Value) -> Result<Self> {
        has_pixel_layer(s).map_err(EngineError::Other)?;
        if sample_all_layers(p) {
            return Err(bad(cmd, "Sample All Layers strokes aren't previewed live"));
        }
        let (stroke, id) = parse_brush(s, p, cmd)?;
        let (tool, _) = DabTool::parse(s, cmd, p, &stroke)?;
        let (neighborhood, spacing, brush) = (tool.neighborhood(), stroke.brush.spacing, Box::new(stroke.brush.clone()));
        let effect = tool.clone();
        let mut live = Self::with_neighborhood(s, cmd, p, stroke, id, neighborhood, |pre| effect.effect(pre.format(), spacing))?;
        live.tool = Some((tool, brush));
        Ok(live)
    }

    /// Start a live stroke of `cmd` that runs `effect` (made from the target before the stroke)
    /// at each dab of `stroke`, reading `halo` pixels around it. `p` gives the target.
    pub(crate) fn with_effect(
        s: &Session,
        cmd: &str,
        p: &Value,
        stroke: Stroke,
        id: Option<LayerId>,
        halo: i32,
        effect: impl FnOnce(&Surface) -> DabEffect,
    ) -> Result<Self> {
        Self::with_neighborhood(s, cmd, p, stroke, id, DabNeighborhood::Halo(halo), effect)
    }

    fn with_neighborhood(
        s: &Session,
        cmd: &str,
        p: &Value,
        stroke: Stroke,
        id: Option<LayerId>,
        neighborhood: DabNeighborhood,
        effect: impl FnOnce(&Surface) -> DabEffect,
    ) -> Result<Self> {
        let st = s.active().ok_or(EngineError::NoDocument)?;
        let base = (st.doc.id, st.revision);
        let doc = (*st.doc).clone();
        let mut live = Self::with_neighborhood_on(doc, cmd, p, stroke, id, neighborhood, effect)?;
        live.base = Some(base);
        Ok(live)
    }

    /// [`Self::with_effect`] on `doc`, a copy of the active document with what the commit's own
    /// edit does first (the Background Eraser's Background turned into a normal layer, say).
    pub(crate) fn with_effect_on(
        doc: Document,
        cmd: &str,
        p: &Value,
        stroke: Stroke,
        id: Option<LayerId>,
        halo: i32,
        effect: impl FnOnce(&Surface) -> DabEffect,
    ) -> Result<Self> {
        Self::with_neighborhood_on(doc, cmd, p, stroke, id, DabNeighborhood::Halo(halo), effect)
    }

    fn with_neighborhood_on(
        mut doc: Document,
        cmd: &str,
        p: &Value,
        stroke: Stroke,
        id: Option<LayerId>,
        neighborhood: DabNeighborhood,
        effect: impl FnOnce(&Surface) -> DabEffect,
    ) -> Result<Self> {
        let sel = doc.selection.clone();
        let (surf, lock) = crate::channel_cmds::target_surface(&mut doc, id, p)?;
        let pre_surf = surf.clone();
        let effect = effect(&pre_surf);
        // `apply_dab_stroke` generates the same dabs (`dabs`: zoom 1, no smoothing scale).
        let renderer = photocraft_paint::StrokeRenderer::new(&stroke.brush, None, 1.0).record_dabs();
        let mut live = Self {
            doc: std::sync::Arc::new(doc),
            cmd: cmd.into(),
            renderer,
            done: 0,
            effect,
            neighborhood,
            work: WorkTiles::new(pre_surf.channels()),
            pre_surf,
            id,
            params: p.clone(),
            opacity: stroke.brush.opacity,
            sel,
            lock,
            bounds: Rect::EMPTY,
            preview_tail: false,
            tail: Rect::EMPTY,
            tool: None,
            base: None,
            points: Vec::new(),
        };
        live.push(&stroke.points)?;
        Ok(live)
    }

    /// Also show the stroke's finishing dabs (see `preview_tail`).
    pub(crate) fn with_tail_preview(mut self) -> Result<Self> {
        self.preview_tail = true;
        self.push(&[])?;
        Ok(self)
    }

    /// Everything the stroke has touched so far.
    pub fn bounds(&self) -> Rect {
        self.bounds
    }

    /// Run the dabs the renderer has generated since the last call on the working copy; returns
    /// the rectangle they changed.
    fn run_new_dabs(&mut self) -> Rect {
        let ctx = self.renderer.ctx.clone();
        let new: Vec<_> = self.renderer.dabs().get(self.done..).unwrap_or_default().to_vec();
        let mut dmg = Rect::EMPTY;
        for (k, d) in new.iter().enumerate() {
            let r = ctx.dab_rect(d, false);
            if r.is_empty() {
                continue;
            }
            let mut region = self.work.read(&self.pre_surf, self.neighborhood.read_rect(r, d));
            (self.effect)(&mut region, &ctx.footprint(d, self.done + k));
            self.work.write(&self.pre_surf, &region.crop(r));
            dmg = dmg.union(&r);
        }
        self.done += new.len();
        dmg
    }

    /// Mix the working pixels of `r` back over the original into the shown document.
    fn show(&mut self, r: Rect, work: &Region) -> Result<()> {
        let (surf, _) = crate::channel_cmds::target_surface(std::sync::Arc::make_mut(&mut self.doc), self.id, &self.params)?;
        photocraft_paint::retouch::mix_back(surf, &Region::read(&self.pre_surf, r), work, r, self.opacity, self.sel.as_ref(), self.lock);
        Ok(())
    }

    /// Render more points; returns the rectangle that changed.
    /// Invalid coordinates reject the whole batch without changing the preview.
    /// Only the position and pressure of `pts` are used: the commands take no tilt or times.
    pub fn push(&mut self, pts: &[StrokePoint]) -> Result<Rect> {
        crate::brush_cmds::check_coords(pts, &self.cmd)?;
        let pts: Vec<_> = pts.iter().map(|q| StrokePoint::new(q.x, q.y, q.pressure)).collect();
        self.points.extend_from_slice(&pts);
        self.renderer.push(&pts);
        let dmg = self.run_new_dabs();
        let ctx = self.renderer.ctx.clone();
        // The finishing dabs, run on a scratch copy of the working pixels.
        let mut tail = None;
        if self.preview_tail {
            let ds = self.renderer.tail_dabs();
            let r = ds.iter().fold(Rect::EMPTY, |a, d| a.union(&ctx.dab_rect(d, false)));
            if !r.is_empty() {
                let mut region = self.work.read(&self.pre_surf, r.inflate(self.neighborhood.halo()));
                for (k, d) in ds.iter().enumerate() {
                    (self.effect)(&mut region, &ctx.footprint(d, self.done + k));
                }
                tail = Some((r, region.crop(r)));
            }
        }
        // Back to the stroke without the previous tail.
        let old = std::mem::replace(&mut self.tail, Rect::EMPTY);
        let shown = dmg.union(&old);
        let tail_rect = tail.as_ref().map_or(Rect::EMPTY, |(r, _)| *r);
        if shown.is_empty() && tail_rect.is_empty() {
            return Ok(Rect::EMPTY);
        }
        self.bounds = self.bounds.union(&dmg).union(&tail_rect);
        if !shown.is_empty() {
            let work = self.work.read(&self.pre_surf, shown);
            self.show(shown, &work)?;
        }
        if let Some((r, work)) = tail {
            self.show(r, &work)?;
            self.tail = r;
        }
        Ok(shown.union(&tail_rect))
    }

    /// The finished stroke as its commit would take it over ([`Session::offer_live_dab`]): the
    /// finishing dabs are run, and the pixels are the ones the command with the same params and
    /// points paints. Only while the brush, tool settings and layer are still the ones the stroke
    /// started with: the document revision doesn't cover those.
    pub(crate) fn into_commit(mut self, s: &Session) -> Result<crate::brush_cmds::LiveCommit> {
        let (Some((tool, brush)), Some(base)) = (self.tool.take(), self.base) else {
            return Err(bad(&self.cmd, "this stroke's commit renders itself"));
        };
        let (stroke, id) = parse_brush(s, &self.params, &self.cmd)?;
        let (now, _) = DabTool::parse(s, &self.cmd, &self.params, &stroke)?;
        if now != tool || stroke.brush != *brush || id != self.id {
            return Err(bad(&self.cmd, "the brush, tool options or layer changed during the stroke"));
        }
        self.renderer.finish();
        let dmg = self.run_new_dabs();
        let shown = dmg.union(&self.tail);
        if !shown.is_empty() {
            let work = self.work.read(&self.pre_surf, shown);
            self.show(shown, &work)?;
        }
        self.bounds = self.bounds.union(&dmg);
        let (surf, _) = crate::channel_cmds::target_surface(std::sync::Arc::make_mut(&mut self.doc), self.id, &self.params)?;
        let surface = surf.clone();
        Ok(crate::brush_cmds::LiveCommit {
            mixer: None,
            cmd: self.cmd,
            base,
            params: self.params,
            seed: None,
            points: self.points,
            surface,
            bounds: self.bounds,
        })
    }
}
