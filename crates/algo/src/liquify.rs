//! Filter › Liquify: a dense displacement field edited by brush strokes, then used to resample
//! the layer once.
//!
//! The field stores, for every node of a regular grid (one node per `cell` document pixels), the
//! offset `d` such that output pixel `p` shows the source at `p + d(p)` (an inverse map, so
//! applying it never leaves holes). Strokes are plain data ([`LiquifyStroke`]: tool, brush and
//! points), applied in order by [`LiquifyField::apply_stroke`]; replaying the same strokes on the
//! same bounds gives the same field bit for bit, which is what makes the filter a replayable
//! command and a smart filter.
//!
//! Every tool is written as "where does the new output at `p` come from in the current output":
//! `q = M(p)` (Forward Warp: `p - kΔ`, Twirl: a rotation about the brush centre, Pucker/Bloat: a
//! scaling about it), so the new field is `d'(p) = q + d(q) - p`, sampled bilinearly from the
//! field as it was before the dab. Reconstruct scales `d` toward zero and Smooth relaxes it toward
//! its local mean. Freeze/Thaw paint a mask that scales every tool's strength down.
//!
//! [`apply_liquify`] upsamples the field with Catmull-Rom (bicubic) per pixel and samples the
//! source (premultiplied, bicubic), tile by tile in parallel; tiles where the field is zero are
//! copied untouched. [`ProxyImage::render`] is the cheap preview path: bilinear field and
//! bilinear source on a downsampled RGBA copy, re-rendered only where a dab changed things.

use crate::par;
use photocraft_geom::Rect;
use photocraft_raster::Surface;
use serde::{Deserialize, Serialize};

/// The Liquify tools (Photoshop's left tool strip, minus Hand/Zoom/Face).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LiquifyTool {
    #[default]
    ForwardWarp,
    Reconstruct,
    Smooth,
    TwirlCw,
    /// Twirl Clockwise with Alt (Option) held: a stroke's direction, not a tool of its own. The
    /// reference application's Liquify has Twirl Clockwise only; strokes may still name
    /// `twirlCcw` (Alt-strokes record it, and older scripts and saved strokes use it).
    TwirlCcw,
    Pucker,
    Bloat,
    PushLeft,
    Freeze,
    Thaw,
    LassoMask,
    /// The Reconstruct… button: scales the whole (unfrozen) field toward zero by `amount` %.
    ReconstructAll,
    /// Mask Options › Mask All / None / Invert All (whole-field freeze edits).
    FreezeAll,
    ThawAll,
    InvertFreeze,
}

impl LiquifyTool {
    /// The tool strip, in order. Twirl Counterclockwise is not on it: it is Twirl Clockwise with
    /// Alt (Option) held.
    pub const ALL: [LiquifyTool; 10] = [
        LiquifyTool::ForwardWarp,
        LiquifyTool::Reconstruct,
        LiquifyTool::Smooth,
        LiquifyTool::TwirlCw,
        LiquifyTool::Pucker,
        LiquifyTool::Bloat,
        LiquifyTool::PushLeft,
        LiquifyTool::Freeze,
        LiquifyTool::Thaw,
        LiquifyTool::LassoMask,
    ];

    pub fn label(self) -> &'static str {
        match self {
            LiquifyTool::ForwardWarp => "Forward Warp",
            LiquifyTool::Reconstruct => "Reconstruct",
            LiquifyTool::Smooth => "Smooth",
            LiquifyTool::TwirlCw => "Twirl Clockwise",
            LiquifyTool::TwirlCcw => "Twirl Counterclockwise",
            LiquifyTool::Pucker => "Pucker",
            LiquifyTool::Bloat => "Bloat",
            LiquifyTool::PushLeft => "Push Left",
            LiquifyTool::Freeze => "Freeze Mask",
            LiquifyTool::Thaw => "Thaw Mask",
            LiquifyTool::LassoMask => "Freeze Lasso",
            LiquifyTool::ReconstructAll => "Reconstruct All",
            LiquifyTool::FreezeAll => "Mask All",
            LiquifyTool::ThawAll => "Mask None",
            LiquifyTool::InvertFreeze => "Invert Mask",
        }
    }

    /// Whole-field operations (no brush, no points).
    pub fn is_global(self) -> bool {
        matches!(self, LiquifyTool::ReconstructAll | LiquifyTool::FreezeAll | LiquifyTool::ThawAll | LiquifyTool::InvertFreeze)
    }

    /// Tools that act while the brush is held still (each recorded point is a dab).
    pub fn is_stationary(self) -> bool {
        !matches!(self, LiquifyTool::ForwardWarp | LiquifyTool::PushLeft | LiquifyTool::LassoMask)
    }
}

fn d_size() -> f64 {
    100.0
}
fn d_density() -> f64 {
    50.0
}
fn d_pressure() -> f64 {
    100.0
}
fn d_rate() -> f64 {
    80.0
}

/// One brush stroke: the tool, its brush options and the points it passed through
/// (`[x, y]` or `[x, y, pressure 0..1]`, document pixels).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LiquifyStroke {
    #[serde(default)]
    pub tool: LiquifyTool,
    /// Brush diameter (px).
    #[serde(default = "d_size")]
    pub size: f64,
    /// How far the full effect reaches toward the brush edge, 0 (only the centre; feathered)
    /// ..100 (most of the brush). It always fades to nothing at the edge ([`falloff`]).
    #[serde(default = "d_density")]
    pub density: f64,
    /// Overall strength, 0..100.
    #[serde(default = "d_pressure")]
    pub pressure: f64,
    /// Speed of the stationary tools, 0..100.
    #[serde(default = "d_rate")]
    pub rate: f64,
    #[serde(default)]
    pub points: Vec<Vec<f64>>,
    /// `reconstructAll` only: how much to restore, 0..100. `lassoMask`: 1.0/none freezes the
    /// polygon, 0.0 thaws it.
    #[serde(default)]
    pub amount: Option<f64>,
}

impl LiquifyStroke {
    pub fn new(tool: LiquifyTool, size: f64) -> Self {
        LiquifyStroke { tool, size, density: d_density(), pressure: d_pressure(), rate: d_rate(), points: Vec::new(), amount: None }
    }

    /// Validates the stroke (finite numbers, 2 or 3 per point).
    pub fn validate(&self) -> Result<(), String> {
        if !(self.size.is_finite() && self.size >= 1.0 && self.size <= 15000.0) {
            return Err(format!("brush size {} out of range 1..15000", self.size));
        }
        for p in &self.points {
            if !(2..=3).contains(&p.len()) || p.iter().any(|v| !v.is_finite()) {
                return Err("stroke points are [x, y] or [x, y, pressure]".into());
            }
        }
        Ok(())
    }
}

/// The displacement field plus the freeze mask, over `bounds` (document pixels).
#[derive(Clone, Debug)]
pub struct LiquifyField {
    pub bounds: Rect,
    /// Document pixels per field node.
    pub cell: f64,
    pub w: usize,
    pub h: usize,
    /// Inverse displacement per node, row-major.
    pub d: Vec<[f32; 2]>,
    /// Freeze mask per node (0 = free, 1 = frozen).
    pub freeze: Vec<f32>,
    /// Where the brush stroke in progress stands, so dab spacing runs on across its segments.
    cursor: Option<StrokeCursor>,
}

/// Two fields are equal when their displacement and freeze mask are: the stroke cursor is
/// transient brush state, not part of the result.
impl PartialEq for LiquifyField {
    fn eq(&self, o: &Self) -> bool {
        self.bounds == o.bounds && self.cell == o.cell && self.w == o.w && self.h == o.h && self.d == o.d && self.freeze == o.freeze
    }
}

/// The brush stroke in progress: the point its last segment ended at, its last dab and how far
/// the brush has travelled along the path since that dab.
#[derive(Clone, Copy, Debug, PartialEq)]
struct StrokeCursor {
    end: [f64; 2],
    last_dab: [f64; 2],
    carry: f64,
}

/// Dabs over this many field nodes compute their rows in parallel (an 800 px brush has about
/// 125,000); smaller ones are not worth the hand-off.
const PARALLEL_DAB_NODES: usize = 16_384;

/// Bound completed output buffers independently of the document's size.
const RENDER_BATCH_TILES: usize = 64;

/// Distance between dabs along a stroke for brush radius `r`: a twentieth of the radius, so a
/// drag is many small pushes and its edge stays smooth.
fn dab_spacing(r: f64) -> f64 {
    (r * 0.05).max(0.5)
}

/// Per-dab strength of a stationary tool (Twirl, Pucker, Bloat, Reconstruct, Smooth, Freeze,
/// Thaw) while it moves: scaled to the spacing so its effect per distance travelled stays what it
/// was with dabs a fifth of the radius apart.
fn moving_weight(r: f64) -> f64 {
    dab_spacing(r) / (r * 0.2).max(0.5)
}

/// The default field resolution for `bounds`: 2 px per node up to 4 MP, 4 px beyond.
pub fn auto_cell(bounds: Rect) -> f64 {
    if bounds.size().area() <= 4_200_000 { 2.0 } else { 4.0 }
}

/// Most nodes of a field's displacement grid (12 bytes each with the freeze mask): 192 MB.
const MAX_FIELD_NODES: usize = 1 << 24;

impl LiquifyField {
    pub fn new(bounds: Rect, cell: f64) -> Self {
        let mut cell = cell.clamp(1.0, 64.0);
        let nodes = |cell: f64| ((f64::from(bounds.width()) / cell).ceil() as usize + 2, (f64::from(bounds.height()) / cell).ceil() as usize + 2);
        let (mut w, mut h) = nodes(cell);
        // The extent is the document canvas, which a file declares: past the node budget the
        // field gets coarser (it is interpolated anyway) instead of asking for gigabytes.
        while w.saturating_mul(h) > MAX_FIELD_NODES {
            cell *= 2.0;
            (w, h) = nodes(cell);
        }
        LiquifyField { bounds, cell, w, h, d: vec![[0.0; 2]; w * h], freeze: vec![0.0; w * h], cursor: None }
    }

    /// Document position of node (i, j) (node 0 sits on the first pixel centre).
    #[inline]
    pub fn node_pos(&self, i: usize, j: usize) -> [f64; 2] {
        [f64::from(self.bounds.x0) + 0.5 + i as f64 * self.cell, f64::from(self.bounds.y0) + 0.5 + j as f64 * self.cell]
    }

    /// Field coordinates of a document point.
    #[inline]
    fn to_grid(&self, x: f64, y: f64) -> (f64, f64) {
        ((x - f64::from(self.bounds.x0) - 0.5) / self.cell, (y - f64::from(self.bounds.y0) - 0.5) / self.cell)
    }

    pub fn is_identity(&self) -> bool {
        self.d.iter().all(|v| v[0] == 0.0 && v[1] == 0.0)
    }

    /// Largest displacement magnitude (px).
    pub fn max_displacement(&self) -> f32 {
        self.d.iter().map(|v| v[0].hypot(v[1])).fold(0.0, f32::max)
    }

    /// Bilinear displacement at a document point (zero outside the field).
    pub fn sample(&self, x: f64, y: f64) -> [f64; 2] {
        let (gx, gy) = self.to_grid(x, y);
        bilinear(&self.d, self.w, self.h, gx, gy)
    }

    /// Freeze value at a document point (bilinear).
    pub fn freeze_at(&self, x: f64, y: f64) -> f32 {
        let (gx, gy) = self.to_grid(x, y);
        let gx = gx.clamp(0.0, (self.w - 1) as f64);
        let gy = gy.clamp(0.0, (self.h - 1) as f64);
        let (i, j) = ((gx as usize).min(self.w - 2), (gy as usize).min(self.h - 2));
        let (fx, fy) = ((gx - i as f64) as f32, (gy - j as f64) as f32);
        let at = |i: usize, j: usize| self.freeze[j * self.w + i];
        (at(i, j) * (1.0 - fx) + at(i + 1, j) * fx) * (1.0 - fy) + (at(i, j + 1) * (1.0 - fx) + at(i + 1, j + 1) * fx) * fy
    }

    /// Applies one stroke. Returns the document rectangle whose output changed.
    pub fn apply_stroke(&mut self, s: &LiquifyStroke) -> Rect {
        self.cursor = None;
        match s.tool {
            LiquifyTool::LassoMask => return self.apply_lasso(s),
            LiquifyTool::ReconstructAll => {
                let k = (s.amount.unwrap_or(100.0) / 100.0).clamp(0.0, 1.0) as f32;
                for (v, f) in self.d.iter_mut().zip(&self.freeze) {
                    let m = 1.0 - k * (1.0 - f);
                    v[0] *= m;
                    v[1] *= m;
                }
                return self.bounds;
            }
            LiquifyTool::FreezeAll => self.freeze.fill(1.0),
            LiquifyTool::ThawAll => self.freeze.fill(0.0),
            LiquifyTool::InvertFreeze => self.freeze.iter_mut().for_each(|f| *f = 1.0 - *f),
            _ => {}
        }
        if s.tool.is_global() {
            return self.bounds;
        }
        let pts: Vec<[f64; 3]> = s.points.iter().filter(|p| p.len() >= 2).map(|p| [p[0], p[1], p.get(2).copied().unwrap_or(1.0)]).collect();
        let Some(&first) = pts.first() else { return Rect::EMPTY };
        let mut dirty = self.stroke_begin(s, first);
        for w in pts.windows(2) {
            dirty = dirty.union(&self.stroke_segment(s, w[0], w[1]));
        }
        dirty
    }

    /// Lasso: sets the freeze mask to `amount` (1.0 when none; 0.0 thaws) at every field node
    /// inside the closed polygon in `s.points` (even-odd rule; non-finite points are skipped).
    /// A scanline fill: each node row costs one pass over the edges, then only the nodes inside
    /// are written. Returns the document rectangle the mask changed in.
    fn apply_lasso(&mut self, s: &LiquifyStroke) -> Rect {
        let poly: Vec<[f64; 2]> = s.points.iter().filter_map(|p| Some([*p.first()?, *p.get(1)?])).filter(|p| p[0].is_finite() && p[1].is_finite()).collect();
        if poly.len() < 3 || self.w == 0 || self.h == 0 {
            return Rect::EMPTY;
        }
        let (mut px0, mut py0, mut px1, mut py1) = (f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY);
        for p in &poly {
            px0 = px0.min(p[0]);
            py0 = py0.min(p[1]);
            px1 = px1.max(p[0]);
            py1 = py1.max(p[1]);
        }
        let (_, gy0) = self.to_grid(px0, py0);
        let (_, gy1) = self.to_grid(px1, py1);
        let last_row = self.h.saturating_sub(1) as f64;
        let last_col = self.w.saturating_sub(1) as f64;
        let (j0, j1) = (gy0.ceil().max(0.0), gy1.floor().min(last_row));
        if j1 < j0 {
            return Rect::EMPTY;
        }
        let v = if s.amount.unwrap_or(1.0) > 0.5 { 1.0f32 } else { 0.0 };
        let mut xs: Vec<f64> = Vec::new();
        for j in (j0 as usize)..=(j1 as usize) {
            let y = self.node_pos(0, j)[1];
            xs.clear();
            let Some(mut prev) = poly.last().copied() else { return Rect::EMPTY };
            for &a in &poly {
                if (a[1] > y) != (prev[1] > y) {
                    xs.push(a[0] + (y - a[1]) * (prev[0] - a[0]) / (prev[1] - a[1]));
                }
                prev = a;
            }
            xs.sort_by(f64::total_cmp);
            // Even-odd: nodes with xs[2k] <= x < xs[2k + 1] are inside.
            for &[xa, xb] in xs.as_chunks::<2>().0 {
                let (ga, _) = self.to_grid(xa, y);
                let (gb, _) = self.to_grid(xb, y);
                let (i0, i1) = (ga.ceil().max(0.0), (gb.ceil() - 1.0).min(last_col));
                if i1 < i0 {
                    continue;
                }
                let row = j * self.w;
                if let Some(cells) = self.freeze.get_mut(row + i0 as usize..=row + i1 as usize) {
                    cells.fill(v);
                }
            }
        }
        Rect::new(px0.floor() as i32, py0.floor() as i32, px1.ceil() as i32, py1.ceil() as i32).intersect(&self.bounds)
    }

    /// Starts a stroke at `p` (`[x, y, pressure]`): stationary tools dab once there. Interactive
    /// callers use this plus [`Self::stroke_segment`] per new point, which is exactly what
    /// [`Self::apply_stroke`] does with the whole point list.
    pub fn stroke_begin(&mut self, s: &LiquifyStroke, p: [f64; 3]) -> Rect {
        self.cursor = Some(StrokeCursor { end: [p[0], p[1]], last_dab: [p[0], p[1]], carry: 0.0 });
        if s.tool.is_stationary() && !s.tool.is_global() { self.dab(s, [p[0], p[1]], [0.0, 0.0], p[2].clamp(0.0, 1.0), 1.0) } else { Rect::EMPTY }
    }

    /// Continues a stroke from `a` to `b`. Dabs fall every [`dab_spacing`] of distance travelled
    /// along the whole stroke, carried from one segment to the next, so the result is the same
    /// however finely the pointer path was sampled (a fast and a slow mouse give one warp). A
    /// stationary tool held still dabs once at `b`. When `a` is not where the previous segment
    /// ended, the stroke starts afresh at `a`.
    pub fn stroke_segment(&mut self, s: &LiquifyStroke, a: [f64; 3], b: [f64; 3]) -> Rect {
        self.walk_segment(s, a, b, true)
    }

    /// [`Self::stroke_segment`]; `skip` leaves out the dabs that cannot reach a field node (they
    /// would change nothing), which tests compare with the full walk.
    fn walk_segment(&mut self, s: &LiquifyStroke, a: [f64; 3], b: [f64; 3], skip: bool) -> Rect {
        let mut dirty = Rect::EMPTY;
        let (pa, pb) = (a[2].clamp(0.0, 1.0), b[2].clamp(0.0, 1.0));
        let stationary = s.tool.is_stationary();
        let cur = match self.cursor {
            Some(c) if c.end == [a[0], a[1]] => c,
            _ => StrokeCursor { end: [a[0], a[1]], last_dab: [a[0], a[1]], carry: 0.0 },
        };
        let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
        let len = dx.hypot(dy);
        if len == 0.0 {
            self.cursor = Some(cur);
            return if stationary { self.dab(s, [b[0], b[1]], [0.0, 0.0], pb, 1.0) } else { dirty };
        }
        // NaN or a length that overflowed: nothing sensible to walk; the next segment restarts.
        if !len.is_finite() {
            self.cursor = None;
            return dirty;
        }
        let r = (s.size / 2.0).max(0.5);
        let spacing = dab_spacing(r);
        let weight = if stationary { moving_weight(r) } else { 1.0 };
        // Distance along this segment of its first dab, then one every `spacing`.
        let first = (spacing - cur.carry).clamp(0.0, spacing);
        // Distances are compared with a little slack: a dab that rounding puts a hair past the
        // segment's end still falls on it, so cutting one segment in two never adds or drops a dab.
        let slack = spacing * 1e-6;
        if first > len + slack {
            self.cursor = Some(StrokeCursor { end: [b[0], b[1]], last_dab: cur.last_dab, carry: cur.carry + len });
            return dirty;
        }
        let last_k = ((len - first + slack) / spacing).floor().max(0.0);
        let at = |dist: f64| [a[0] + dx * dist / len, a[1] + dy * dist / len];
        let (mut k0, mut k1) = (0.0f64, last_k);
        if skip {
            match self.reachable_span(a, [dx, dy], len, r) {
                // A dragging dab is centred one spacing behind its position: pad by that and a step.
                Some((d0, d1)) => {
                    k0 = k0.max(((d0 - spacing - first) / spacing).floor() - 1.0);
                    k1 = k1.min(((d1 + spacing - first) / spacing).ceil() + 1.0);
                }
                None => k1 = -1.0,
            }
        }
        if k0 <= k1 {
            let mut prev = if k0 > 0.0 { at(first + (k0 - 1.0) * spacing) } else { cur.last_dab };
            for k in (k0 as u64)..=(k1 as u64) {
                let dist = (first + k as f64 * spacing).min(len);
                let c = at(dist);
                let delta = [c[0] - prev[0], c[1] - prev[1]];
                // Forward Warp centres the dab where the brush was (it drags what is under it).
                let centre = if stationary { c } else { prev };
                dirty = dirty.union(&self.dab(s, centre, delta, pa + (pb - pa) * dist / len, weight));
                prev = c;
            }
        }
        let last = (first + last_k * spacing).min(len);
        self.cursor = Some(StrokeCursor { end: [b[0], b[1]], last_dab: at(last), carry: (len - last).clamp(0.0, spacing) });
        dirty
    }

    /// The distances along a segment from `a` (direction `dir`, length `len`) whose points lie
    /// within a dab radius `r` plus two nodes of the node grid, or `None` when none do. Only dabs
    /// near that span can touch a node; a segment to a far-off point has trillions of others
    /// (#937).
    fn reachable_span(&self, a: [f64; 3], dir: [f64; 2], len: f64, r: f64) -> Option<(f64, f64)> {
        if self.w == 0 || self.h == 0 {
            return None;
        }
        let pad = r + 2.0 * self.cell;
        let lo = [f64::from(self.bounds.x0) + 0.5 - pad, f64::from(self.bounds.y0) + 0.5 - pad];
        let hi = [lo[0] + (self.w - 1) as f64 * self.cell + 2.0 * pad, lo[1] + (self.h - 1) as f64 * self.cell + 2.0 * pad];
        let (mut u0, mut u1) = (0.0f64, 1.0f64);
        for ax in 0..2 {
            let d = dir[ax];
            if d == 0.0 {
                if a[ax].is_nan() || a[ax] < lo[ax] || a[ax] > hi[ax] {
                    return None;
                }
                continue;
            }
            let (v0, v1) = ((lo[ax] - a[ax]) / d, (hi[ax] - a[ax]) / d);
            u0 = u0.max(v0.min(v1));
            u1 = u1.min(v0.max(v1));
        }
        if u0.is_nan() || u1.is_nan() || u0 > u1 {
            return None;
        }
        Some((u0 * len, u1 * len))
    }

    /// One dab at `c` with brush motion `delta`, its strength scaled by `weight`. Returns the
    /// document rect it affected.
    fn dab(&mut self, s: &LiquifyStroke, c: [f64; 2], delta: [f64; 2], point_pressure: f64, weight: f64) -> Rect {
        if s.tool == LiquifyTool::LassoMask {
            return Rect::EMPTY; // the lasso is polygon-only; never a brush dab
        }
        let r = (s.size / 2.0).max(0.5);
        let strength = (s.pressure / 100.0).clamp(0.0, 1.0) * point_pressure * weight;
        let rate = (s.rate / 100.0).clamp(0.0, 1.0);
        let density = (s.density / 100.0).clamp(0.0, 1.0);
        if strength <= 0.0 {
            return Rect::EMPTY;
        }
        let (gx0, gy0) = self.to_grid(c[0] - r, c[1] - r);
        let (gx1, gy1) = self.to_grid(c[0] + r, c[1] + r);
        let i0 = gx0.floor().max(0.0) as usize;
        let j0 = gy0.floor().max(0.0) as usize;
        let i1 = (gx1.ceil().max(-1.0) as isize).min(self.w as isize - 1);
        let j1 = (gy1.ceil().max(-1.0) as isize).min(self.h as isize - 1);
        if i1 < i0 as isize || j1 < j0 as isize {
            return Rect::EMPTY;
        }
        let (i1, j1) = (i1 as usize, j1 as usize);
        // Snapshot of the affected nodes plus a margin (sampling reads around them).
        let m = ((delta[0].abs().max(delta[1].abs()) + r * 0.2) / self.cell).ceil() as usize + 2;
        let (si0, sj0) = (i0.saturating_sub(m), j0.saturating_sub(m));
        let (si1, sj1) = ((i1 + m).min(self.w - 1), (j1 + m).min(self.h - 1));
        let sw = si1 - si0 + 1;
        let sh = sj1 - sj0 + 1;
        let mut snap = Vec::with_capacity(sw * sh);
        for j in sj0..=sj1 {
            snap.extend_from_slice(&self.d[j * self.w + si0..=j * self.w + si1]);
        }
        let old = |gx: f64, gy: f64| -> [f64; 2] {
            let (lx, ly) = (gx - si0 as f64, gy - sj0 as f64);
            if lx >= 0.0 && ly >= 0.0 && lx <= (sw - 1) as f64 && ly <= (sh - 1) as f64 {
                bilinear(&snap, sw, sh, lx, ly)
            } else {
                // Outside the snapshot nothing changed: read the live field.
                bilinear(&self.d, self.w, self.h, gx, gy)
            }
        };
        // Every node reads only the snapshot and the field as it was before this dab (the writes
        // land after), so rows are independent: a large brush computes them in parallel.
        let inv_cell = 1.0 / self.cell;
        let node_row = |j: usize| -> Vec<(usize, [f32; 2], f32)> {
            let mut row = Vec::with_capacity(i1 - i0 + 1);
            for i in i0..=i1 {
                let p = self.node_pos(i, j);
                let (dx, dy) = (p[0] - c[0], p[1] - c[1]);
                let t2 = (dx * dx + dy * dy) / (r * r);
                if t2 >= 1.0 {
                    continue;
                }
                let fall = falloff(t2, density);
                let idx = j * self.w + i;
                let k_raw = strength * fall;
                if s.tool == LiquifyTool::Freeze || s.tool == LiquifyTool::Thaw {
                    let f = self.freeze[idx];
                    let v = if s.tool == LiquifyTool::Freeze { (f as f64 + k_raw).min(1.0) } else { (f as f64 - k_raw).max(0.0) };
                    row.push((idx, self.d[idx], v as f32));
                    continue;
                }
                let k = k_raw * (1.0 - f64::from(self.freeze[idx]));
                if k <= 0.0 {
                    continue;
                }
                let cur = self.d[idx];
                let new = match s.tool {
                    LiquifyTool::Reconstruct => {
                        let a = (1.0 - k * (0.1 + 0.4 * rate)) as f32;
                        [cur[0] * a, cur[1] * a]
                    }
                    LiquifyTool::Smooth => {
                        let (gx, gy) = ((i - si0) as isize, (j - sj0) as isize);
                        let at = |x: isize, y: isize| snap[(y.clamp(0, sh as isize - 1) as usize) * sw + x.clamp(0, sw as isize - 1) as usize];
                        let mut mean = [0.0f32; 2];
                        for (ox, oy) in [(-1, 0), (1, 0), (0, -1), (0, 1), (-1, -1), (1, -1), (-1, 1), (1, 1)] {
                            let v = at(gx + ox, gy + oy);
                            mean[0] += v[0] / 8.0;
                            mean[1] += v[1] / 8.0;
                        }
                        let a = (k * (0.1 + 0.4 * rate)) as f32;
                        [cur[0] + (mean[0] - cur[0]) * a, cur[1] + (mean[1] - cur[1]) * a]
                    }
                    _ => {
                        let q = match s.tool {
                            LiquifyTool::ForwardWarp => [p[0] - k * delta[0], p[1] - k * delta[1]],
                            // Left of the stroke direction (y down): (dx, dy) → (dy, -dx).
                            LiquifyTool::PushLeft => [p[0] - k * delta[1], p[1] + k * delta[0]],
                            LiquifyTool::TwirlCw | LiquifyTool::TwirlCcw => {
                                let th = k * rate * 0.12 * if s.tool == LiquifyTool::TwirlCw { 1.0 } else { -1.0 };
                                // Content turns by +th, so sample at the point turned by -th.
                                let (sn, cs) = (-th).sin_cos();
                                [c[0] + dx * cs - dy * sn, c[1] + dx * sn + dy * cs]
                            }
                            LiquifyTool::Pucker => {
                                let a = 1.0 + k * rate * 0.06;
                                [c[0] + dx * a, c[1] + dy * a]
                            }
                            _ => {
                                let a = 1.0 - k * rate * 0.06;
                                [c[0] + dx * a, c[1] + dy * a]
                            }
                        };
                        let (qx, qy) = ((q[0] - p[0]) * inv_cell + i as f64, (q[1] - p[1]) * inv_cell + j as f64);
                        let dq = old(qx, qy);
                        [(q[0] + dq[0] - p[0]) as f32, (q[1] + dq[1] - p[1]) as f32]
                    }
                };
                row.push((idx, new, self.freeze[idx]));
            }
            row
        };
        let rows = j1 - j0 + 1;
        let writes = par::map(rows, rows * (i1 - i0 + 1) > PARALLEL_DAB_NODES, |row| node_row(j0 + row));
        for (idx, d, f) in writes.into_iter().flatten() {
            self.d[idx] = d;
            self.freeze[idx] = f;
        }
        let pad = r.ceil() as i32 + 2;
        Rect::new(c[0].floor() as i32 - pad, c[1].floor() as i32 - pad, c[0].ceil() as i32 + pad, c[1].ceil() as i32 + pad).intersect(&self.bounds)
    }

    /// Replays strokes on a fresh field.
    pub fn from_strokes(bounds: Rect, cell: f64, strokes: &[LiquifyStroke]) -> Self {
        let mut f = LiquifyField::new(bounds, cell);
        for s in strokes {
            f.apply_stroke(s);
        }
        f
    }

    /// Catmull-Rom (bicubic) displacement at grid coordinates.
    fn sample_cubic(&self, xs: &[(usize, f64); 4], ys: &[(usize, f64); 4]) -> [f64; 2] {
        let mut acc = [0.0f64; 2];
        for &(y, wyj) in ys {
            let row = &self.d[y * self.w..(y + 1) * self.w];
            for &(x, wxi) in xs {
                let k = wxi * wyj;
                acc[0] += f64::from(row[x][0]) * k;
                acc[1] += f64::from(row[x][1]) * k;
            }
        }
        acc
    }

    /// Whether any node influencing document rect `r` is displaced.
    fn touches(&self, r: Rect) -> bool {
        let (gx0, gy0) = self.to_grid(f64::from(r.x0), f64::from(r.y0));
        let (gx1, gy1) = self.to_grid(f64::from(r.x1), f64::from(r.y1));
        let i0 = (gx0.floor() as isize - 2).clamp(0, self.w as isize - 1) as usize;
        let j0 = (gy0.floor() as isize - 2).clamp(0, self.h as isize - 1) as usize;
        let i1 = (gx1.ceil() as isize + 2).clamp(0, self.w as isize - 1) as usize;
        let j1 = (gy1.ceil() as isize + 2).clamp(0, self.h as isize - 1) as usize;
        (j0..=j1).any(|j| self.d[j * self.w + i0..=j * self.w + i1].iter().any(|v| v[0] != 0.0 || v[1] != 0.0))
    }
}

/// A dab's strength at squared distance `t2` (in brush radii, 0..1) for Density `density` (0..1):
/// 1 at the centre, and down to 0 at the rim with a flat slope at every density, as in
/// Photoshop, where Density sets how far the full effect reaches toward the edge (0: only the
/// centre, the rest feathered; 1: most of the brush) but the effect always fades out at it.
/// A brush that stopped short of 0 at its rim moved the pixels inside it and left those just
/// outside, tearing the image along every dab's circle (#2606).
pub fn falloff(t2: f64, density: f64) -> f64 {
    let t2 = t2.clamp(0.0, 1.0);
    let density = if density.is_nan() { 0.5 } else { density.clamp(0.0, 1.0) };
    // Feathered: (1 - t²)³. Dense: (1 - t⁴)², flat in the middle. Both are 0 with zero slope at
    // t = 1, and their slopes stay small enough that a dab every fifth of the radius never folds
    // the image over itself.
    let feathered = (1.0 - t2).powi(3);
    let dense = (1.0 - t2 * t2).powi(2);
    feathered + (dense - feathered) * density
}

fn bilinear(d: &[[f32; 2]], w: usize, h: usize, gx: f64, gy: f64) -> [f64; 2] {
    if !(gx > -1.0 && gy > -1.0 && gx < w as f64 && gy < h as f64) {
        return [0.0, 0.0];
    }
    let gx = gx.clamp(0.0, (w - 1) as f64);
    let gy = gy.clamp(0.0, (h - 1) as f64);
    let (i, j) = ((gx as usize).min(w.saturating_sub(2)), (gy as usize).min(h.saturating_sub(2)));
    let (fx, fy) = (gx - i as f64, gy - j as f64);
    let i1 = (i + 1).min(w - 1);
    let j1 = (j + 1).min(h - 1);
    let at = |x: usize, y: usize| d[y * w + x];
    let (a, b, c, e) = (at(i, j), at(i1, j), at(i, j1), at(i1, j1));
    let l = |k: usize| (f64::from(a[k]) * (1.0 - fx) + f64::from(b[k]) * fx) * (1.0 - fy) + (f64::from(c[k]) * (1.0 - fx) + f64::from(e[k]) * fx) * fy;
    [l(0), l(1)]
}

fn catmull_rom(t: f64) -> [f64; 4] {
    let t2 = t * t;
    let t3 = t2 * t;
    [-0.5 * t3 + t2 - 0.5 * t, 1.5 * t3 - 2.5 * t2 + 1.0, -1.5 * t3 + 2.0 * t2 + 0.5 * t, 0.5 * t3 - 0.5 * t2]
}

/// A field axis's clamped nodes and weights, reusable across a tile's rows or columns.
fn cubic_taps(g: f64, len: usize) -> [(usize, f64); 4] {
    let i = g.floor() as isize;
    let weights = catmull_rom(g - g.floor());
    std::array::from_fn(|k| ((i - 1 + k as isize).clamp(0, len as isize - 1) as usize, weights[k]))
}

/// Resamples `src` through the field: inside `field.bounds` each pixel shows the source at
/// `p + d(p)` (bicubic field, bicubic premultiplied source, edge pixels repeat past the field
/// bounds); everything outside the bounds is kept. Same format as `src`.
pub fn apply_liquify(src: &Surface, field: &LiquifyField) -> Surface {
    let fmt = src.format();
    let n = fmt.channels();
    let alpha = fmt.alpha;
    let b = field.bounds;
    let mut out = src.clone();
    if field.is_identity() || b.is_empty() {
        return out;
    }
    // A 1D Catmull-Rom stencil's absolute weights sum to at most 5/4, so the 2D
    // interpolated displacement can reach 25/16 of the largest nodal displacement.
    let maxd = ((f64::from(field.max_displacement()) * (25.0 / 16.0)).ceil() as i32).saturating_add(3);
    let has_freeze = field.freeze.iter().any(|&f| f > 0.0);
    let tiles: Vec<Rect> = b.tiles().map(|t| t.rect().intersect(&b)).filter(|r| !r.is_empty() && field.touches(*r)).collect();
    let work = |t: &Rect| -> Option<(Rect, Vec<f32>)> {
        let w = t.width() as usize;
        let h = t.height() as usize;
        // Field per pixel (bicubic), with axis taps shared across rows and columns.
        let mut disp = vec![[0.0f64; 2]; w * h];
        let (mut fx0, mut fy0, mut fx1, mut fy1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
        let mut moved = false;
        let columns: Vec<_> = (0..w)
            .map(|x| {
                let px = f64::from(t.x0) + x as f64 + 0.5;
                let gx = (px - f64::from(b.x0) - 0.5) / field.cell;
                (px, cubic_taps(gx, field.w))
            })
            .collect();
        for y in 0..h {
            let py = f64::from(t.y0) + y as f64 + 0.5;
            let gy = (py - f64::from(b.y0) - 0.5) / field.cell;
            let ys = cubic_taps(gy, field.h);
            for (x, (px, xs)) in columns.iter().enumerate() {
                let px = *px;
                let mut d = field.sample_cubic(xs, &ys);
                if has_freeze {
                    let mask = f64::from(field.freeze_at(px, py)).clamp(0.0, 1.0);
                    if mask > 0.0 {
                        let linear = field.sample(px, py);
                        d[0] = d[0] * (1.0 - mask) + linear[0] * mask;
                        d[1] = d[1] * (1.0 - mask) + linear[1] * mask;
                    }
                }
                if d[0] != 0.0 || d[1] != 0.0 {
                    moved = true;
                }
                disp[y * w + x] = d;
                let (sx, sy) = (px + d[0], py + d[1]);
                fx0 = fx0.min(sx);
                fy0 = fy0.min(sy);
                fx1 = fx1.max(sx);
                fy1 = fy1.max(sy);
            }
        }
        if !moved {
            return None;
        }
        let foot =
            Rect::new(fx0.floor() as i32 - 3, fy0.floor() as i32 - 3, fx1.ceil() as i32 + 3, fy1.ceil() as i32 + 3).intersect(&t.inflate(maxd)).intersect(&b);
        if foot.is_empty() {
            return None;
        }
        let mut px = src.read_region(foot);
        if alpha {
            for p in px.chunks_exact_mut(n) {
                let al = p[n - 1];
                for v in &mut p[..n - 1] {
                    *v *= al;
                }
            }
        }
        let fw = foot.width() as usize;
        let at = |x: i32, y: i32| -> usize {
            let x = x.clamp(foot.x0, foot.x1 - 1);
            let y = y.clamp(foot.y0, foot.y1 - 1);
            ((y - foot.y0) as usize * fw + (x - foot.x0) as usize) * n
        };
        let snap = |t: f64| if (t - t.round()).abs() < 1e-7 { t.round() } else { t };
        let mut outp = vec![0.0f32; w * h * n];
        let mut acc = [0.0f64; 8];
        for y in 0..h {
            for x in 0..w {
                let d = disp[y * w + x];
                let u = snap(f64::from(t.x0) + x as f64 + d[0]);
                let v = snap(f64::from(t.y0) + y as f64 + d[1]);
                let (ix, iy) = (u.floor() as i32, v.floor() as i32);
                let (wx, wy) = (catmull_rom(u - f64::from(ix)), catmull_rom(v - f64::from(iy)));
                acc[..n].fill(0.0);
                for (j, wyj) in wy.iter().enumerate() {
                    for (i, wxi) in wx.iter().enumerate() {
                        let k = wxi * wyj;
                        if k == 0.0 {
                            continue;
                        }
                        let o = at(ix - 1 + i as i32, iy - 1 + j as i32);
                        for (c, s) in acc.iter_mut().enumerate().take(n) {
                            *s += f64::from(px[o + c]) * k;
                        }
                    }
                }
                let o = (y * w + x) * n;
                if alpha {
                    let al = acc[n - 1].clamp(0.0, 1.0);
                    outp[o + n - 1] = al as f32;
                    for c in 0..n - 1 {
                        outp[o + c] = if al > 0.0 { (acc[c] / al).clamp(0.0, 1.0) as f32 } else { 0.0 };
                    }
                } else {
                    for c in 0..n {
                        outp[o + c] = acc[c].clamp(0.0, 1.0) as f32;
                    }
                }
            }
        }
        Some((*t, outp))
    };
    for batch in tiles.chunks(RENDER_BATCH_TILES) {
        for (r, v) in par::map(batch.len(), true, |i| batch.get(i).and_then(work)).into_iter().flatten() {
            out.write_region(r, &v);
        }
    }
    if alpha {
        out.prune();
    }
    out
}

/// A downsampled, premultiplied RGBA copy of a layer for the interactive preview.
#[derive(Clone, Debug)]
pub struct ProxyImage {
    /// Document region the proxy covers (the field bounds).
    pub bounds: Rect,
    /// Document pixels per proxy pixel.
    pub scale: f64,
    pub w: usize,
    pub h: usize,
    /// Premultiplied RGBA, row-major.
    pub px: Vec<[f32; 4]>,
}

const MAX_FULL_RES_SOURCE_PIXELS: usize = 8 * 1024 * 1024;

impl ProxyImage {
    /// Box-downsamples `src` over `bounds` so the longer side is at most `max_side`.
    pub fn new(src: &Surface, bounds: Rect, max_side: usize) -> Self {
        let side = bounds.width().max(bounds.height()).max(1) as f64;
        let scale = (side / max_side.max(1) as f64).max(1.0).ceil();
        let w = ((f64::from(bounds.width()) / scale).ceil() as usize).max(1);
        let h = ((f64::from(bounds.height()) / scale).ceil() as usize).max(1);
        let k = scale as i32;
        let mut px = vec![[0.0f32; 4]; w * h];
        let fmt = src.format();
        let fill_rows = |y: usize, row: &mut [[f32; 4]]| {
            let r = Rect::new(bounds.x0, bounds.y0 + y as i32 * k, bounds.x0 + w as i32 * k, (bounds.y0 + (y as i32 + 1) * k).min(bounds.y1));
            let mut buf = vec![[0.0f32; 4]; r.width() as usize * r.height() as usize];
            src.read_rgba_into(r, &mut buf);
            let rw = r.width() as usize;
            for (x, o) in row.iter_mut().enumerate() {
                let mut acc = [0.0f32; 4];
                let mut cnt = 0.0f32;
                for yy in 0..r.height() as usize {
                    for xx in x * k as usize..((x + 1) * k as usize).min(rw) {
                        let p = buf[yy * rw + xx];
                        let a = if fmt.alpha { p[3] } else { 1.0 };
                        acc[0] += p[0] * a;
                        acc[1] += p[1] * a;
                        acc[2] += p[2] * a;
                        acc[3] += a;
                        cnt += 1.0;
                    }
                }
                if cnt > 0.0 {
                    *o = acc.map(|v| v / cnt);
                }
            }
        };
        #[cfg(not(target_arch = "wasm32"))]
        {
            use rayon::prelude::*;
            px.par_chunks_mut(w).enumerate().for_each(|(y, row)| fill_rows(y, row));
        }
        #[cfg(target_arch = "wasm32")]
        px.chunks_mut(w).enumerate().for_each(|(y, row)| fill_rows(y, row));
        ProxyImage { bounds, scale, w, h, px }
    }

    /// Renders proxy pixels `r` (proxy coordinates) through the field into `out` (premultiplied
    /// RGBA8, `w × h`): bilinear field, bilinear source.
    pub fn render(&self, field: &LiquifyField, r: [usize; 4], out: &mut [[u8; 4]]) {
        let [x0, y0, x1, y1] = [r[0].min(self.w), r[1].min(self.h), r[2].min(self.w), r[3].min(self.h)];
        if x0 >= x1 || y0 >= y1 {
            return;
        }
        let s = self.scale;
        let (bx, by) = (f64::from(self.bounds.x0), f64::from(self.bounds.y0));
        let row = |y: usize, line: &mut [[u8; 4]]| {
            let py = by + (y as f64 + 0.5) * s;
            for (x, slot) in line.iter_mut().enumerate().take(x1).skip(x0) {
                let px = bx + (x as f64 + 0.5) * s;
                let d = field.sample(px, py);
                let (u, v) = ((px + d[0] - bx) / s - 0.5, (py + d[1] - by) / s - 0.5);
                let (iu, iv) = (u.floor(), v.floor());
                let (fu, fv) = ((u - iu) as f32, (v - iv) as f32);
                let at = |x: f64, y: f64| self.px[(y.clamp(0.0, (self.h - 1) as f64) as usize) * self.w + x.clamp(0.0, (self.w - 1) as f64) as usize];
                let (a, b, c, e) = (at(iu, iv), at(iu + 1.0, iv), at(iu, iv + 1.0), at(iu + 1.0, iv + 1.0));
                *slot = std::array::from_fn(|k| {
                    let v = (a[k] * (1.0 - fu) + b[k] * fu) * (1.0 - fv) + (c[k] * (1.0 - fu) + e[k] * fu) * fv;
                    (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8
                });
            }
        };
        let rows = &mut out[y0 * self.w..y1 * self.w];
        // A dab's few thousand pixels are faster on this thread than waking the pool.
        #[cfg(not(target_arch = "wasm32"))]
        if (x1 - x0) * (y1 - y0) > 150_000 {
            use rayon::prelude::*;
            rows.par_chunks_mut(self.w).enumerate().for_each(|(i, line)| row(y0 + i, line));
            return;
        }
        rows.chunks_mut(self.w).enumerate().for_each(|(i, line)| row(y0 + i, line));
    }

    /// Renders a document-pixel-resolution preview for `region` into the corresponding part of
    /// `out`, whose dimensions cover `bounds`. Source reads are limited to the dirty region plus
    /// the displacement range that can affect it, so zoomed-in brush updates do not flatten the
    /// whole layer. `source_scratch` is reused by the caller between updates.
    pub fn render_full_resolution(
        src: &Surface,
        field: &LiquifyField,
        bounds: Rect,
        region: Rect,
        out: &mut [[u8; 4]],
        source_scratch: &mut Vec<[f32; 4]>,
    ) -> bool {
        let bounds = bounds.intersect(&field.bounds);
        let region = region.intersect(&bounds);
        let (width, height) = (bounds.width() as usize, bounds.height() as usize);
        let Some(out_len) = width.checked_mul(height) else { return false };
        if region.is_empty() || out.len() != out_len || width == 0 || height == 0 {
            return false;
        }

        // Bilinear field interpolation is a convex combination of its surrounding nodes. The
        // extrema of those nodes therefore bound every source sample in this output rectangle.
        let (mut min_dx, mut min_dy, mut max_dx, mut max_dy) = (f32::INFINITY, f32::INFINITY, f32::NEG_INFINITY, f32::NEG_INFINITY);
        let (gx0, gy0) = field.to_grid(f64::from(region.x0) + 0.5, f64::from(region.y0) + 0.5);
        let (gx1, gy1) = field.to_grid(f64::from(region.x1) - 0.5, f64::from(region.y1) - 0.5);
        let (i0, j0) = (gx0.floor().max(0.0) as usize, gy0.floor().max(0.0) as usize);
        let (i1, j1) = (gx1.ceil().max(0.0) as usize, gy1.ceil().max(0.0) as usize);
        for j in j0..=j1.min(field.h.saturating_sub(1)) {
            for i in i0..=i1.min(field.w.saturating_sub(1)) {
                let Some(v) = j.checked_mul(field.w).and_then(|row| row.checked_add(i)).and_then(|idx| field.d.get(idx)) else { continue };
                min_dx = min_dx.min(v[0]);
                min_dy = min_dy.min(v[1]);
                max_dx = max_dx.max(v[0]);
                max_dy = max_dy.max(v[1]);
            }
        }
        if !min_dx.is_finite() || !min_dy.is_finite() || !max_dx.is_finite() || !max_dy.is_finite() {
            return false;
        }

        // Source coordinates are pixel centres. Add one sample on each side for bilinear
        // interpolation, then clamp to the document just as the regular proxy renderer does.
        let x0 = (f64::from(region.x0) + f64::from(min_dx)).floor() - 1.0;
        let y0 = (f64::from(region.y0) + f64::from(min_dy)).floor() - 1.0;
        let x1 = (f64::from(region.x1 - 1) + f64::from(max_dx)).ceil() + 2.0;
        let y1 = (f64::from(region.y1 - 1) + f64::from(max_dy)).ceil() + 2.0;
        let canvas = field.bounds;
        let src_rect = Rect::new(
            x0.max(f64::from(canvas.x0)).min(f64::from(canvas.x1)) as i32,
            y0.max(f64::from(canvas.y0)).min(f64::from(canvas.y1)) as i32,
            x1.max(f64::from(canvas.x0)).min(f64::from(canvas.x1)) as i32,
            y1.max(f64::from(canvas.y0)).min(f64::from(canvas.y1)) as i32,
        );
        if src_rect.is_empty() {
            return false;
        }
        let (sw, sh) = (src_rect.width() as usize, src_rect.height() as usize);
        let Some(source_len) = sw.checked_mul(sh) else { return false };
        if source_len > MAX_FULL_RES_SOURCE_PIXELS {
            return false;
        }
        if source_scratch.try_reserve(source_len.saturating_sub(source_scratch.len())).is_err() {
            return false;
        }
        source_scratch.resize(source_len, [0.0; 4]);
        // Read and premultiply the source in one pass per band of rows, the bands in parallel: at
        // an 800 px brush it is about a million pixels (P30).
        let opaque = !src.format().alpha;
        const BAND_ROWS: usize = 32;
        par::chunks_mut(source_scratch, sw * BAND_ROWS, true, |band_index, band| {
            let y = src_rect.y0 + (band_index * BAND_ROWS) as i32;
            src.read_rgba_into(Rect::new(src_rect.x0, y, src_rect.x1, y + (band.len() / sw) as i32), band);
            for pixel in band.iter_mut() {
                if opaque {
                    pixel[3] = 1.0;
                }
                let alpha = pixel[3];
                pixel[0] *= alpha;
                pixel[1] *= alpha;
                pixel[2] *= alpha;
            }
        });

        let x0 = (region.x0 - bounds.x0) as usize;
        let x1 = (region.x1 - bounds.x0) as usize;
        let y0 = (region.y0 - bounds.y0) as usize;
        let y1 = (region.y1 - bounds.y0) as usize;
        let source_at = |x: f64, y: f64| -> [f32; 4] {
            let fx = (x - 0.5).clamp(f64::from(canvas.x0), f64::from(canvas.x1 - 1));
            let fy = (y - 0.5).clamp(f64::from(canvas.y0), f64::from(canvas.y1 - 1));
            let ix = fx.floor();
            let iy = fy.floor();
            let (tx, ty) = ((fx - ix) as f32, (fy - iy) as f32);
            let sx0 = (ix as i32).clamp(src_rect.x0, src_rect.x1 - 1);
            let sy0 = (iy as i32).clamp(src_rect.y0, src_rect.y1 - 1);
            let sx1 = (sx0 + 1).min(src_rect.x1 - 1);
            let sy1 = (sy0 + 1).min(src_rect.y1 - 1);
            let at = |sx: i32, sy: i32| -> [f32; 4] {
                let ox = (sx - src_rect.x0) as usize;
                let oy = (sy - src_rect.y0) as usize;
                source_scratch.get(oy * sw + ox).copied().unwrap_or([0.0; 4])
            };
            let (a, b, c, d) = (at(sx0, sy0), at(sx1, sy0), at(sx0, sy1), at(sx1, sy1));
            std::array::from_fn(|channel| (a[channel] * (1.0 - tx) + b[channel] * tx) * (1.0 - ty) + (c[channel] * (1.0 - tx) + d[channel] * tx) * ty)
        };
        let render_rows = |first_y: usize, rows: &mut [[u8; 4]]| {
            for (offset, row) in rows.chunks_mut(width).enumerate() {
                let y = bounds.y0 + (first_y + offset) as i32;
                let Some(dest) = row.get_mut(x0..x1) else { continue };
                for (offset_x, pixel) in dest.iter_mut().enumerate() {
                    let x = bounds.x0 + (x0 + offset_x) as i32;
                    let p = [f64::from(x) + 0.5, f64::from(y) + 0.5];
                    let d = field.sample(p[0], p[1]);
                    let rgba = source_at(p[0] + d[0], p[1] + d[1]);
                    *pixel = rgba.map(|v| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8);
                }
            }
        };
        let Some(rows) = out.get_mut(y0 * width..y1 * width) else { return false };
        #[cfg(not(target_arch = "wasm32"))]
        if (x1 - x0).saturating_mul(y1 - y0) > 150_000 {
            use rayon::prelude::*;
            rows.par_chunks_mut(width).enumerate().for_each(|(offset, line)| render_rows(y0 + offset, line));
            return true;
        }
        rows.chunks_mut(width).enumerate().for_each(|(offset, line)| render_rows(y0 + offset, line));
        true
    }

    /// The proxy pixel rectangle covering document rect `r`.
    pub fn proxy_rect(&self, r: Rect) -> [usize; 4] {
        let s = self.scale;
        let f = |v: i32, o: i32| (f64::from(v - o) / s).max(0.0);
        [
            f(r.x0, self.bounds.x0).floor() as usize,
            f(r.y0, self.bounds.y0).floor() as usize,
            (f(r.x1, self.bounds.x0).ceil() as usize + 1).min(self.w),
            (f(r.y1, self.bounds.y0).ceil() as usize + 1).min(self.h),
        ]
    }
}

#[cfg(test)]
mod tests;
