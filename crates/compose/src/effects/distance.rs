use super::*;

/// 1D squared distance transform (Felzenszwalb & Huttenlocher).
fn dt1(f: &[f32], out: &mut [f32], v: &mut [usize], z: &mut [f32], near: &mut [usize]) {
    let n = f.len();
    let mut k = 0usize;
    v[0] = 0;
    z[0] = f32::NEG_INFINITY;
    z[1] = f32::INFINITY;
    for q in 1..n {
        loop {
            let p = v[k];
            let s = ((f[q] + (q * q) as f32) - (f[p] + (p * p) as f32)) / (2.0 * q as f32 - 2.0 * p as f32);
            if s <= z[k] && k > 0 {
                k -= 1;
                continue;
            }
            if s <= z[k] {
                // k == 0 and parabola dominates: replace.
                v[0] = q;
                z[0] = f32::NEG_INFINITY;
                z[1] = f32::INFINITY;
                break;
            }
            k += 1;
            v[k] = q;
            z[k] = s;
            z[k + 1] = f32::INFINITY;
            break;
        }
    }
    k = 0;
    for (q, o) in out.iter_mut().enumerate() {
        while z[k + 1] < q as f32 {
            k += 1;
        }
        let p = v[k];
        let d = q as f32 - p as f32;
        *o = d * d + f[p];
        near[q] = p;
    }
}

/// Exact Euclidean distance from every pixel to the nearest pixel where
/// `inside` is true (0 on those pixels).
#[cfg(test)]
pub(super) fn edt(inside: &[bool], w: usize, h: usize) -> Vec<f32> {
    edt_nearest(inside, w, h).0
}

/// Maps with fewer pixels (or other units of work) than this are not worth spawning threads for.
const PAR_MIN: usize = 1 << 16;

/// Most threads one pass uses (as for the blurs, [`rows_par`]).
const PAR_THREADS: usize = 16;

#[cfg(test)]
thread_local! {
    /// Test hook: the thread count [`par_threads`] gives on this thread, whatever the work.
    pub(super) static THREADS: std::cell::Cell<Option<usize>> = const { std::cell::Cell::new(None) };
}

/// How many threads a pass over `work` pixels (or similar units) uses.
fn par_threads(work: usize) -> usize {
    #[cfg(test)]
    if let Some(n) = THREADS.with(std::cell::Cell::get) {
        return n;
    }
    if cfg!(target_arch = "wasm32") || work < PAR_MIN { 1 } else { std::thread::available_parallelism().map_or(1, |n| n.get()).clamp(1, PAR_THREADS) }
}

/// Runs `f(scratch, u, a_u, b_u)` for every unit `u` of two slices: unit `u` of `a` is
/// `a[u * unit_a..][..unit_a]` (the last may be shorter), the same of `b` with `unit_b`, which is 0
/// when there is no second slice. `work` says how big the whole pass is ([`par_threads`]). Groups of
/// units go to scoped OS threads, each with a scratch value from `init`; not to rayon, as for
/// [`rows_par`]: maps are built from inside rayon tiles, where a waiting worker steals other tiles.
/// A unit is computed from its own arguments alone, so the result does not depend on the threads.
pub(super) fn par_units<S, A: Send, B: Send>(
    work: usize,
    a: &mut [A],
    unit_a: usize,
    b: &mut [B],
    unit_b: usize,
    init: impl Fn() -> S + Sync,
    f: impl Fn(&mut S, usize, &mut [A], &mut [B]) + Sync,
) {
    if unit_a == 0 {
        return;
    }
    let units = a.len().div_ceil(unit_a);
    let threads = par_threads(work).min(units);
    if threads <= 1 {
        run_units(&mut init(), 0, a, unit_a, b, unit_b, &f);
        return;
    }
    // Many more groups than threads, so that a heavy one (an edge-rich stretch) does not hold the rest up.
    let group = units.div_ceil(threads * 8).max(1);
    let mut groups = Vec::new();
    let (mut rest_a, mut rest_b, mut first) = (a, b, 0usize);
    while !rest_a.is_empty() {
        let (ga, na) = rest_a.split_at_mut((group * unit_a).min(rest_a.len()));
        let (gb, nb) = rest_b.split_at_mut((group * unit_b).min(rest_b.len()));
        groups.push((first, ga, gb));
        (rest_a, rest_b, first) = (na, nb, first + group);
    }
    let queue = std::sync::Mutex::new(groups.into_iter());
    let worker = || {
        let mut scratch = init();
        loop {
            let next = queue.lock().unwrap_or_else(std::sync::PoisonError::into_inner).next();
            let Some((first, ga, gb)) = next else { break };
            run_units(&mut scratch, first, ga, unit_a, gb, unit_b, &f);
        }
    };
    std::thread::scope(|sc| {
        // A thread that cannot be spawned leaves its share to the others.
        for _ in 1..threads {
            let _ = std::thread::Builder::new().spawn_scoped(sc, worker);
        }
        worker();
    });
}

fn run_units<S, A, B>(scratch: &mut S, first: usize, a: &mut [A], unit_a: usize, b: &mut [B], unit_b: usize, f: &impl Fn(&mut S, usize, &mut [A], &mut [B])) {
    for (k, ua) in a.chunks_mut(unit_a).enumerate() {
        let ub = b.get_mut(k * unit_b..((k + 1) * unit_b).min(b.len())).unwrap_or(&mut []);
        f(scratch, first + k, ua, ub);
    }
}

/// A `w × h` buffer of `fill`, then filled row by row through `f(y, row)`.
pub(super) fn par_fill<T: Clone + Send>(w: usize, h: usize, fill: T, f: impl Fn(usize, &mut [T]) + Sync) -> Vec<T> {
    let mut out = vec![fill; w.saturating_mul(h)];
    par_units(out.len(), &mut out, w.max(1), &mut [] as &mut [()], 0, || (), |_, y, row, _| f(y, row));
    out
}

/// Working buffers of the 1D distance transform, for lines of up to `n` pixels.
struct Dt1Scratch {
    f: Vec<f32>,
    o: Vec<f32>,
    v: Vec<usize>,
    z: Vec<f32>,
    near: Vec<usize>,
}

impl Dt1Scratch {
    fn new(n: usize) -> Self {
        Dt1Scratch { f: vec![0.0; n], o: vec![0.0; n], v: vec![0; n], z: vec![0.0; n + 1], near: vec![0; n] }
    }
}

/// [`edt`] plus the index of the nearest `inside` pixel (as `u32`: a map past 2³² pixels has no
/// nearest pixel to give, and every distance is then "far").
///
/// Both passes run line by line in parallel: the columns, then the rows. The columns' results are
/// kept column by column (`gc[x * h + y]`) so that each thread writes its own stretch of them.
pub(super) fn edt_nearest(inside: &[bool], w: usize, h: usize) -> (Vec<f32>, Vec<u32>) {
    const INF: f32 = 1e20;
    let total = w.saturating_mul(h);
    if total != inside.len() || total > u32::MAX as usize {
        return (vec![INF; inside.len()], vec![0; inside.len()]);
    }
    let n = w.max(h);
    let mut gc = vec![0.0f32; total];
    let mut nc = vec![0u32; total];
    par_units(
        total,
        &mut gc,
        h.max(1),
        &mut nc,
        h.max(1),
        || Dt1Scratch::new(n),
        |t, x, g, near| {
            for (y, f) in t.f[..h].iter_mut().enumerate() {
                *f = if inside[y * w + x] { 0.0 } else { INF };
            }
            dt1(&t.f[..h], &mut t.o[..h], &mut t.v, &mut t.z, &mut t.near[..h]);
            g.copy_from_slice(&t.o[..h]);
            for (d, &p) in near.iter_mut().zip(&t.near[..h]) {
                *d = p as u32;
            }
        },
    );
    let mut g = vec![0.0f32; total];
    let mut nearest = vec![0u32; total];
    par_units(
        total,
        &mut g,
        w.max(1),
        &mut nearest,
        w.max(1),
        || Dt1Scratch::new(n),
        |t, y, out, near| {
            for (x, f) in t.f[..w].iter_mut().enumerate() {
                *f = gc[x * h + y];
            }
            dt1(&t.f[..w], &mut t.o[..w], &mut t.v, &mut t.z, &mut t.near[..w]);
            for (x, (d, ne)) in out.iter_mut().zip(near.iter_mut()).enumerate() {
                *d = t.o[x].sqrt();
                let px = t.near[x];
                *ne = (nc[px * h + y] as usize * w + px) as u32;
            }
        },
    );
    (g, nearest)
}

/// Chamfer distance transform with a 5 × 5 mask of exact Euclidean step lengths (1, √2, √5),
/// plus the index of the nearest `inside` pixel (0 on inside pixels). Photoshop's effect
/// distances follow this metric: its stroke corners measure 3.236 = √5 + 1 at offset (1, 3)
/// and 3.650 = √5 + √2 at (2, 3), where the exact distances are 3.162 and 3.606.
#[cfg(test)]
pub(super) fn chamfer_nearest(inside: &[bool], w: usize, h: usize) -> (Vec<f32>, Vec<usize>) {
    chamfer_from(inside.iter().map(|&b| if b { 0.0 } else { CHAMFER_INF }).collect(), w, h)
}

const CHAMFER_INF: f32 = 1e20;

/// [`chamfer_nearest`] from per-pixel start values (`CHAMFER_INF` = not a seed): the result is
/// the minimum over seeds of start value + path length, so partly covered seeds can start at
/// their sub-pixel edge offset.
fn chamfer_from(mut d: Vec<f32>, w: usize, h: usize) -> (Vec<f32>, Vec<usize>) {
    const S2: f32 = std::f32::consts::SQRT_2;
    const S5: f32 = 2.236_068;
    // Forward-pass neighbours (already visited in raster order); the backward pass mirrors them.
    const FWD: [(i64, i64, f32); 8] = [(-1, 0, 1.0), (-1, -1, S2), (0, -1, 1.0), (1, -1, S2), (-1, -2, S5), (1, -2, S5), (-2, -1, S5), (2, -1, S5)];
    let mut near: Vec<usize> = (0..w * h).collect();
    let (wi, hi) = (w as i64, h as i64);
    let relax = |x: i64, y: i64, sign: i64, d: &mut Vec<f32>, near: &mut Vec<usize>| {
        let i = (y * wi + x) as usize;
        let mut best = d[i];
        let mut bn = near[i];
        for &(ox, oy, wt) in &FWD {
            let (nx, ny) = (x + ox * sign, y + oy * sign);
            if nx < 0 || ny < 0 || nx >= wi || ny >= hi {
                continue;
            }
            let j = (ny * wi + nx) as usize;
            let c = d[j] + wt;
            if c < best {
                best = c;
                bn = near[j];
            }
        }
        d[i] = best;
        near[i] = bn;
    };
    for y in 0..hi {
        for x in 0..wi {
            relax(x, y, 1, &mut d, &mut near);
        }
    }
    for y in (0..hi).rev() {
        for x in (0..wi).rev() {
            relax(x, y, -1, &mut d, &mut near);
        }
    }
    (d, near)
}

/// Distance from each pixel centre outside the shape to the shape's edge.
/// Pixels with non-zero alpha are inside; the edge inside the nearest such
/// pixel is placed by its alpha (coverage), as Photoshop does. Inside pixels
/// get ≤ 0.
pub(super) fn dist_outside(s: &Map) -> Vec<f32> {
    dist_outside_by(s, Metric::Euclidean)
}

/// Distance metric for effect distance fields.
#[derive(Clone, Copy, PartialEq)]
pub(super) enum Metric {
    /// Exact Euclidean (effect edges).
    Euclidean,
    /// 5 × 5 chamfer (inside strokes, spread / choke; see [`chamfer_nearest`]).
    Chamfer,
}

/// Outside width from which a stroke's outer edge uses the exact Euclidean transform instead of
/// the fitted 5 × 5 chamfer.
///
/// The chamfer's level sets are octagons: measured against a point source, its distance exceeds
/// the Euclidean radius by up to `0.027 · r` px — 0.48 px at r = 18 (the widest outside stroke any
/// Photoshop oracle matches with the chamfer) and 1.6 px at r = 60, where a wide stroke's boundary
/// visibly facets (#3646). Half a pixel of deviation is reached at r ≈ 19, so strokes narrower
/// than 20 px keep the chamfer, whose sub-pixel edge seeding Photoshop matches at small offsets,
/// and wider ones stay round.
pub const STROKE_CHAMFER_MAX: f32 = 20.0;

/// Whether a stroke whose outside edge is `out_w` px wide uses the exact Euclidean transform (round)
/// rather than the 5 × 5 chamfer's octagon ([`STROKE_CHAMFER_MAX`]). The CPU compositor and the GPU
/// compositor's `program_with` both call this, so a stroke picks the same field in each.
pub fn stroke_outside_exact(out_w: f32) -> bool {
    out_w >= STROKE_CHAMFER_MAX
}

pub(super) fn dist_outside_by(s: &Map, m: Metric) -> Vec<f32> {
    if m == Metric::Chamfer {
        // Seeds start at their sub-pixel edge offset (1 − coverage).
        let start = s.v.iter().map(|&a| if a > INSIDE_EPS { 1.0 - a.min(1.0) } else { CHAMFER_INF }).collect();
        let (d, _) = chamfer_from(start, s.w, s.h);
        return d.iter().zip(&s.v).map(|(d, &a)| if a > INSIDE_EPS { -0.5 } else { d - 0.5 }).collect();
    }
    let inside = covered(s, |a| a > INSIDE_EPS);
    let (d, near) = edt_nearest(&inside, s.w, s.h);
    par_fill(s.w, s.h, 0.0f32, |y, row| {
        for (x, out) in row.iter_mut().enumerate() {
            let i = y * s.w + x;
            *out = if inside[i] { -0.5 } else { d[i] - 0.5 + (1.0 - s.v[near[i] as usize].min(1.0)) };
        }
    })
}

/// Which pixels of `s` satisfy `test`, row by row in parallel.
fn covered(s: &Map, test: impl Fn(f32) -> bool + Sync) -> Vec<bool> {
    par_fill(s.w, s.h, false, |y, row| {
        for (e, &a) in row.iter_mut().zip(s.v.get(y * s.w..).unwrap_or_default()) {
            *e = test(a);
        }
    })
}

/// Distance from each pixel centre inside the shape to the shape's edge
/// (≤ 0 outside).
pub(super) fn dist_inside(s: &Map) -> Vec<f32> {
    dist_inside_by(s, Metric::Euclidean)
}

pub(super) fn dist_inside_by(s: &Map, m: Metric) -> Vec<f32> {
    if m == Metric::Chamfer {
        // Partly covered edge pixels (alpha below their neighbourhood's) are seeds starting at
        // their coverage, mirroring `dist_outside`: a 25 % edge column puts the edge 0.25 px in.
        let cov = local_coverage(s);
        let start =
            s.v.iter()
                .zip(&cov.v)
                .map(|(&a, &c)| {
                    if a <= INSIDE_EPS {
                        0.0
                    } else if c < 1.0 - INSIDE_EPS {
                        c
                    } else {
                        CHAMFER_INF
                    }
                })
                .collect();
        let (d, _) = chamfer_from(start, s.w, s.h);
        return d.iter().zip(&s.v).map(|(d, &a)| if a <= INSIDE_EPS { -0.5 } else { d - 0.5 }).collect();
    }
    let outside = covered(s, |a| a <= INSIDE_EPS);
    let (d, _) = edt_nearest(&outside, s.w, s.h);
    par_fill(s.w, s.h, 0.0f32, |y, row| {
        for (x, out) in row.iter_mut().enumerate() {
            let i = y * s.w + x;
            *out = if outside[i] { -0.5 } else { d[i] - 0.5 };
        }
    })
}

/// Alpha relative to the 3×3 neighbourhood maximum: separates edge coverage
/// from the fill's own opacity on shape layers.
pub(super) fn local_coverage(s: &Map) -> Map {
    let mut out = s.clone();
    for y in 0..s.h as i64 {
        for x in 0..s.w as i64 {
            let a = s.get(x, y);
            if a <= INSIDE_EPS {
                continue;
            }
            let mut mx = a;
            for dy in -1..=1 {
                for dx in -1..=1 {
                    mx = mx.max(s.get(x + dx, y + dy));
                }
            }
            out.v[y as usize * s.w + x as usize] = (a / mx).min(1.0);
        }
    }
    out
}

/// Signed distance from the centre of a pixel of coverage `a` to the straight edge crossing it,
/// positive when the centre is inside the shape. `n` is the edge's unit normal (its signs don't
/// matter).
///
/// A unit pixel cut by a straight edge at distance `r` from its centre (on the side that holds at
/// most half of the pixel) is covered `1/2 - r/u` while the edge crosses two opposite sides, and
/// `((u + v)/2 - r)² / (2uv)` once it only cuts off a corner, with `u ≥ v` the larger and smaller
/// component of the normal. Inverting that places the edge by its coverage at any angle. For an
/// axis-aligned edge (`v = 0`) it is the `a - 1/2` that [`dist_outside`] assumes.
pub(super) fn edge_offset(n: [f32; 2], a: f32) -> f32 {
    if !a.is_finite() {
        return 0.0;
    }
    let (u, v) = (n[0].abs().max(n[1].abs()), n[0].abs().min(n[1].abs()));
    let (c, inside) = if a > 0.5 { (1.0 - a, true) } else { (a, false) };
    let c = c.clamp(0.0, 0.5);
    // The coverage below which the edge only cuts off a corner.
    let corner = if u > 0.0 { v / (2.0 * u) } else { 0.0 };
    let r = if c >= corner { (0.5 - c) * u } else { 0.5 * (u + v) - (2.0 * u * v * c).sqrt() };
    if inside { r } else { -r }
}

/// A sharp, anti-aliased edge pixel: where it puts the straight edge crossing it.
#[derive(Clone, Copy)]
pub(super) struct SharpEdge {
    /// Signed distance from the pixel's centre to the edge, positive inside ([`edge_offset`]).
    pub(super) offset: f32,
    /// The edge's unit normal pointing into the shape (`[0, 0]` when the coverage gives no direction).
    pub(super) normal: [f32; 2],
}

/// Half the spacing of the edge pixels along an edge (at most √2 / 2, along a diagonal): a pixel
/// within it of the foot of the perpendicular sees the edge as a line, one further out as its end.
const EDGE_REACH: f32 = 0.75;

impl SharpEdge {
    /// Distance to the edge from the pixel `(dx, dy)` away from this one, positive on the shape's
    /// side (`inside`) or on the empty side. It is the distance to the edge's line where the
    /// perpendicular lands within [`EDGE_REACH`] of this pixel, and to the end of that stretch
    /// beyond, so the distance to a straight edge is exact and does not follow the pixel staircase.
    /// `None` when the edge has no direction or `(dx, dy)` is on the wrong side of it.
    fn distance(&self, dx: f32, dy: f32, inside: bool) -> Option<f32> {
        let [nx, ny] = self.normal;
        if nx == 0.0 && ny == 0.0 {
            return None;
        }
        let signed = self.offset + dx * nx + dy * ny;
        let across = if inside { signed } else { -signed };
        let along = (dx * -ny + dy * nx).abs();
        (across > 0.0).then(|| across.hypot((along - EDGE_REACH).max(0.0)))
    }
}

/// A sharp edge pixel and where it sits.
#[derive(Clone, Copy)]
pub(super) struct EdgePixel {
    /// Index of the pixel in the map.
    pub(super) at: u32,
    /// The pixel's coverage ([`local_coverage`] of it).
    pub(super) cov: f32,
    pub(super) edge: SharpEdge,
}

/// The sharp edge pixels of a map: a short list, and for every pixel a 4 byte slot (0 = no edge, k =
/// the k-th of the list). Maps are big and edge pixels few, so a per-pixel `Option<SharpEdge>` (16
/// bytes) made every pass over it, and the copies the refinement needs, memory-bound.
pub(super) struct SharpEdges {
    w: usize,
    h: usize,
    pub(super) slot: Vec<u32>,
    pub(super) pixels: Vec<EdgePixel>,
}

impl SharpEdges {
    fn empty(w: usize, h: usize) -> Self {
        SharpEdges { w, h, slot: Vec::new(), pixels: Vec::new() }
    }

    /// The edge of pixel `i`, if it is a sharp edge pixel.
    fn get(&self, i: usize) -> Option<SharpEdge> {
        let k = (*self.slot.get(i)? as usize).checked_sub(1)?;
        self.pixels.get(k).map(|p| p.edge)
    }

    /// The edge of pixel `(x, y)`, if it is a sharp edge pixel (`None` off the map).
    fn at(&self, x: usize, y: usize) -> Option<SharpEdge> {
        if x >= self.w || y >= self.h { None } else { self.get(y * self.w + x) }
    }

    fn has(&self, x: usize, y: usize) -> bool {
        self.at(x, y).is_some()
    }

    pub(super) fn is_empty(&self) -> bool {
        self.pixels.is_empty()
    }

    /// Every sharp edge pixel, as (index in the map, its edge), in pixel order.
    #[cfg(test)]
    pub(super) fn iter(&self) -> impl Iterator<Item = (usize, SharpEdge)> + '_ {
        self.pixels.iter().map(|p| (p.at as usize, p.edge))
    }
}

/// Where the sharp, anti-aliased edge pixels of `s` put the edge ([`edge_offset`], along the normal
/// the coverage gradient gives). Sharp pixels are partly covered and touch both an empty and a full
/// pixel; hard edges and soft ramps have none and keep the `alpha - 1/2` of [`dist_outside`].
///
/// An edge pixel is covered and has an empty neighbour, so a pass over which pixels are covered finds
/// the few candidates without computing [`local_coverage`] of the whole map; the coverage is then
/// worked out where it is needed, by the same formula. Rows are independent and go in parallel.
pub(super) fn sharp_edges(s: &Map) -> SharpEdges {
    let (w, h) = (s.w, s.h);
    if w == 0 || h == 0 || s.v.len() != w * h || s.v.len() >= u32::MAX as usize {
        return SharpEdges::empty(w, h);
    }
    let (wi, hi) = (w as i64, h as i64);
    // `local_coverage` of the pixel at (x, y), on the map.
    let cov_at = |x: i64, y: i64| -> f32 {
        let a = s.v[y as usize * w + x as usize];
        if a <= INSIDE_EPS {
            return a;
        }
        let mut mx = a;
        for dy in -1..=1 {
            for dx in -1..=1 {
                mx = mx.max(s.get(x + dx, y + dy));
            }
        }
        (a / mx).min(1.0)
    };
    // The coverage with the map's border repeated, for the gradient.
    let cov_clamped = |x: i64, y: i64| cov_at(x.max(0).min(wi - 1), y.max(0).min(hi - 1));
    // Whether a pixel is not empty. Off the map counts as covered: only a neighbour on the map can be empty.
    let solid = |x: i64, y: i64| -> bool {
        if x < 0 || y < 0 || x >= wi || y >= hi {
            return true;
        }
        let a = s.v[y as usize * w + x as usize];
        a.is_nan() || a > INSIDE_EPS
    };
    let mut rows: Vec<Vec<EdgePixel>> = vec![Vec::new(); h];
    par_units(
        w * h,
        &mut rows,
        1,
        &mut [] as &mut [()],
        0,
        || (),
        |_, y, row, _| {
            let Some(row) = row.first_mut() else { return };
            let y = y as i64;
            for x in 0..wi {
                if !solid(x, y) || (-1..=1).all(|dy| (-1..=1).all(|dx| solid(x + dx, y + dy))) {
                    continue;
                }
                let i = (y * wi + x) as usize;
                let c = cov_at(x, y);
                if s.v[i] <= INSIDE_EPS || c >= 1.0 - INSIDE_EPS {
                    continue;
                }
                let (mut empty, mut full) = (false, false);
                for (dx, dy) in (-1..=1).flat_map(|dy| (-1..=1).map(move |dx| (dx, dy))) {
                    let (nx, ny) = (x + dx, y + dy);
                    if (dx, dy) == (0, 0) || nx < 0 || ny < 0 || nx >= wi || ny >= hi {
                        continue;
                    }
                    empty |= s.get(nx, ny) <= INSIDE_EPS;
                    full |= cov_at(nx, ny) >= 1.0 - INSIDE_EPS;
                }
                if !(empty && full) {
                    continue;
                }
                let at = &cov_clamped;
                let gx = (at(x + 1, y - 1) + 2.0 * at(x + 1, y) + at(x + 1, y + 1)) - (at(x - 1, y - 1) + 2.0 * at(x - 1, y) + at(x - 1, y + 1));
                let gy = (at(x - 1, y + 1) + 2.0 * at(x, y + 1) + at(x + 1, y + 1)) - (at(x - 1, y - 1) + 2.0 * at(x, y - 1) + at(x + 1, y - 1));
                let len = gx.hypot(gy);
                let (normal, n) = if len.is_finite() && len > 1e-6 { ([gx / len, gy / len], [gx / len, gy / len]) } else { ([0.0, 0.0], [1.0, 0.0]) };
                row.push(EdgePixel { at: i as u32, cov: c, edge: SharpEdge { offset: edge_offset(n, c), normal } });
            }
        },
    );
    let mut pixels: Vec<EdgePixel> = Vec::with_capacity(rows.iter().map(Vec::len).sum());
    for row in rows {
        pixels.extend(row);
    }
    let mut slot = vec![0u32; w * h];
    for (k, p) in pixels.iter().enumerate() {
        if let Some(s) = slot.get_mut(p.at as usize) {
            *s = k as u32 + 1;
        }
    }
    refine_edge_normals(&mut pixels, &slot, w, h);
    SharpEdges { w, h, slot, pixels }
}

/// How far (in pixels) around an edge pixel [`refine_edge_normals`] looks for its neighbours.
const NORMAL_RADIUS: i64 = 3;

/// The 3×3 gradient of the coverage gives an edge's direction to 1-2 degrees, and its error is
/// systematic, so it tilts the distance of every pixel the same way and shows as a faint ripple.
/// The edge points of a straight edge's pixels (each pixel's centre moved to its edge) lie on one
/// line, and the line through the points of the pixels around one gives the direction to a fraction
/// of a degree. Where those points are not on a line (corners, tight curves) the gradient's normal
/// stays.
fn refine_edge_normals(pixels: &mut [EdgePixel], slot: &[u32], w: usize, h: usize) {
    let (wi, hi) = (w as i64, h as i64);
    let points: Vec<[f32; 2]> = pixels
        .iter()
        .map(|p| [(p.at as i64 % wi) as f32 - p.edge.offset * p.edge.normal[0], (p.at as i64 / wi) as f32 - p.edge.offset * p.edge.normal[1]])
        .collect();
    let before: Vec<SharpEdge> = pixels.iter().map(|p| p.edge).collect();
    // Every pixel reads the unrefined edges around it and writes its own, so they go in parallel.
    let work = pixels.len().saturating_mul(32);
    par_units(work, pixels, 1, &mut [] as &mut [()], 0, Vec::<[f32; 2]>::new, |near, k, pixel, _| {
        let (Some(pixel), Some(&e)) = (pixel.first_mut(), before.get(k)) else { return };
        let (x, y) = (pixel.at as i64 % wi, pixel.at as i64 / wi);
        let (mut n, mut sum) = (0.0f32, [0.0f32; 2]);
        near.clear();
        for (dx, dy) in (-NORMAL_RADIUS..=NORMAL_RADIUS).flat_map(|dy| (-NORMAL_RADIUS..=NORMAL_RADIUS).map(move |dx| (dx, dy))) {
            let (nx, ny) = (x + dx, y + dy);
            if dx * dx + dy * dy > NORMAL_RADIUS * NORMAL_RADIUS || nx < 0 || ny < 0 || nx >= wi || ny >= hi {
                continue;
            }
            let Some(j) = slot.get((ny * wi + nx) as usize).and_then(|&s| (s as usize).checked_sub(1)) else { continue };
            let (Some(o), Some(&p)) = (before.get(j), points.get(j)) else { continue };
            if o.normal[0] * e.normal[0] + o.normal[1] * e.normal[1] > 0.9 {
                near.push(p);
                sum = [sum[0] + p[0], sum[1] + p[1]];
                n += 1.0;
            }
        }
        if near.len() < 3 {
            return;
        }
        let mean = [sum[0] / n, sum[1] / n];
        let (mut sxx, mut sxy, mut syy) = (0.0f32, 0.0f32, 0.0f32);
        for p in near.iter() {
            let (dx, dy) = (p[0] - mean[0], p[1] - mean[1]);
            sxx += dx * dx;
            sxy += dx * dy;
            syy += dy * dy;
        }
        // Spread along and across the best line through the points.
        let half = 0.5 * (sxx + syy);
        let root = (0.25 * (sxx - syy) * (sxx - syy) + sxy * sxy).sqrt();
        let (along, across) = (((half + root) / n).max(0.0).sqrt(), ((half - root) / n).max(0.0).sqrt());
        if !(along > EDGE_LINE_MIN_SPREAD && across < EDGE_LINE_MAX_OFF) {
            return;
        }
        let angle = 0.5 * (2.0 * sxy).atan2(sxx - syy);
        let mut normal = [-angle.sin(), angle.cos()];
        if normal[0] * e.normal[0] + normal[1] * e.normal[1] < 0.0 {
            normal = [-normal[0], -normal[1]];
        }
        pixel.edge.normal = normal;
        pixel.edge.offset = edge_offset(normal, pixel.cov);
    });
}

/// The edge points around a pixel must spread at least this far (px, standard deviation) along
/// their line to give it a direction, and stay within [`EDGE_LINE_MAX_OFF`] of it.
const EDGE_LINE_MIN_SPREAD: f32 = 0.7;

const EDGE_LINE_MAX_OFF: f32 = 0.1;

/// How far from the foot of the perpendicular (in pixels) [`edge_distance`] looks for edge pixels.
const FOOT_RADIUS: i64 = 2;

/// The distance from pixel `p` to the sharp edge whose pixel `q` is nearest to it, or `None` when
/// `q` gives no usable distance. The nearest pixel by its centre is not the best one: it can sit a
/// few pixels along the edge from the foot of the perpendicular and deeper than the edge, and the
/// further `p` is from the edge the further, so its distance ripples along the edge. So this takes
/// the smallest distance of the edge pixels around that foot, which is exact for a straight edge.
fn edge_distance(sharp: &SharpEdges, p: [usize; 2], q: [usize; 2], inside: bool) -> Option<f32> {
    let (px, py) = (p[0] as f32, p[1] as f32);
    let e = sharp.at(q[0], q[1])?;
    let mut best = e.distance(px - q[0] as f32, py - q[1] as f32, inside)?;
    // Where p's perpendicular meets q's line.
    let signed = e.offset + (px - q[0] as f32) * e.normal[0] + (py - q[1] as f32) * e.normal[1];
    let (fx, fy) = ((px - signed * e.normal[0]).round() as i64, (py - signed * e.normal[1]).round() as i64);
    for sy in (fy - FOOT_RADIUS).max(0)..=(fy + FOOT_RADIUS).min(sharp.h as i64 - 1) {
        for sx in (fx - FOOT_RADIUS).max(0)..=(fx + FOOT_RADIUS).min(sharp.w as i64 - 1) {
            if let Some(near) = sharp.at(sx as usize, sy as usize)
                && let Some(d) = near.distance(px - sx as f32, py - sy as f32, inside)
            {
                best = best.min(d);
            }
        }
    }
    Some(best)
}

/// [`dist_inside`] for chiselled bevels. A sharp edge pixel puts the edge where its coverage says,
/// at any angle, and the pixels near it measure to that edge ([`edge_distance`]), so the distance
/// ramps stay planar along an oblique edge instead of following the pixel staircase. The plain
/// distance (to the nearest empty pixel, less half a pixel) never falls short of the true one, so
/// it caps the result where an edge lies exactly on a pixel boundary and has no sharp pixel; a mask
/// without sharp pixels gives exactly [`dist_inside`]. Values past `reach` pixels are not refined.
pub(super) fn bevel_dist_inside(s: &Map, sharp: &SharpEdges, reach: f32) -> Vec<f32> {
    let empty = covered(s, |a| a <= INSIDE_EPS);
    let (d, near) = edt_nearest(&empty, s.w, s.h);
    let at = |x: usize, y: usize| -> f32 {
        let i = y * s.w + x;
        if empty[i] {
            return -0.5;
        }
        if let Some(e) = sharp.get(i) {
            return e.offset;
        }
        let plain = d[i] - 0.5;
        if d[i] > reach + EDGE_MARGIN || sharp.is_empty() {
            return plain;
        }
        // The sharp pixel next to the nearest empty one that is nearest to this pixel.
        let (px, py) = (x as i64, y as i64);
        let (ox, oy) = ((near[i] as usize % s.w) as i64, (near[i] as usize / s.w) as i64);
        let q = (-1..=1)
            .flat_map(|dy| (-1..=1).map(move |dx| (ox + dx, oy + dy)))
            .filter(|&(x, y)| x >= 0 && y >= 0 && sharp.has(x as usize, y as usize))
            .min_by_key(|&(x, y)| (x - px) * (x - px) + (y - py) * (y - py));
        match q.and_then(|(x, y)| edge_distance(sharp, [px as usize, py as usize], [x as usize, y as usize], true)) {
            Some(refined) => refined.min(plain),
            None => plain,
        }
    };
    par_fill(s.w, s.h, 0.0f32, |y, row| {
        for (x, out) in row.iter_mut().enumerate() {
            *out = at(x, y);
        }
    })
}

/// [`dist_outside`] for chiselled bevels, with sharp edge pixels placed as in [`bevel_dist_inside`].
pub(super) fn bevel_dist_outside(s: &Map, sharp: &SharpEdges, reach: f32) -> Vec<f32> {
    let inside = covered(s, |a| a > INSIDE_EPS);
    let (d, near) = edt_nearest(&inside, s.w, s.h);
    let at = |x: usize, y: usize| -> f32 {
        let i = y * s.w + x;
        if let Some(e) = sharp.get(i) {
            return -e.offset;
        }
        if inside[i] {
            return -0.5;
        }
        let q = near[i] as usize;
        let Some(e) = sharp.get(q) else { return d[i] - (s.v[q].min(1.0) - 0.5) };
        let (p, qp) = ([x, y], [q % s.w, q / s.w]);
        let measured = if d[i] <= reach + EDGE_MARGIN {
            edge_distance(sharp, p, qp, false)
        } else {
            e.distance(p[0] as f32 - qp[0] as f32, p[1] as f32 - qp[1] as f32, false)
        };
        measured.unwrap_or(d[i] - e.offset)
    };
    par_fill(s.w, s.h, 0.0f32, |y, row| {
        for (x, out) in row.iter_mut().enumerate() {
            *out = at(x, y);
        }
    })
}

/// Pixels past the reach that are still refined, so the value is right where the ramp ends.
const EDGE_MARGIN: f32 = 2.0;

/// Alpha above which a pixel belongs to the layer's shape (half an 8-bit step).
pub(super) const INSIDE_EPS: f32 = 0.5 / 255.0;

/// A distance field of an effect shape, as the map builders above compute it (for the GPU
/// compositor, which builds the rest of the maps itself; keep in sync with `shadow_map`,
/// `glow_map`, `bevel_maps` and `build_maps`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FieldKind {
    /// `dist_outside` of the shape: precise outer glows.
    Outside,
    /// `dist_inside` of the shape: precise inner glows.
    Inside,
    /// `bevel_dist_outside` of the shape: the outside distance of chiselled bevels.
    BevelOutside,
    /// `bevel_dist_inside` of the shape: the inside distance of chiselled bevels.
    BevelInside,
    /// Chamfer distance outside the shape: a narrow raster stroke's outer edge (see
    /// [`stroke_outside_exact`]), and the spread of drop shadows and softer outer glows
    /// ([`dilate`]); their metric is fitted to Photoshop (#224) and keeps the sub-pixel seeding.
    StrokeOutside,
    /// Chamfer distance outside `1 - alpha`: the choke of inner shadows and softer inner glows.
    ChokeInside,
    /// Stroke distance inside the shape.
    StrokeInside,
    /// Exact Euclidean distance outside the shape: a wide raster stroke's outer edge (see
    /// [`stroke_outside_exact`]). The chamfer's level sets are polygons (an ~8-fold octagon), so a
    /// wide stroke's outer boundary facets; the shadow / glow spread above keeps the chamfer (#3646).
    StrokeOutsideExact,
    /// Stroke distance outside a shape layer's outline (from its local coverage), exact Euclidean.
    StrokeOutsideVector,
    /// Chamfer distance outside a shape layer's outline (from its local coverage): a narrow vector
    /// stroke's outer edge, where the chamfer matches Photoshop (see [`STROKE_CHAMFER_MAX`]).
    StrokeOutsideVectorChamfer,
}

/// The `kind` distance field of the shape `alpha` (row-major `w × h`). Values past `reach` pixels
/// from the shape's edge are only meant to be large (the bevel fields skip refining them).
pub fn distance_field(kind: FieldKind, alpha: Vec<f32>, w: usize, h: usize, reach: f32) -> Vec<f32> {
    let s = Map { w, h, v: alpha };
    match kind {
        FieldKind::Outside => dist_outside(&s),
        FieldKind::Inside => dist_inside(&s),
        FieldKind::BevelOutside => bevel_dist_outside(&s, &sharp_edges(&s), reach),
        FieldKind::BevelInside => bevel_dist_inside(&s, &sharp_edges(&s), reach),
        FieldKind::StrokeOutside => dist_outside_by(&s, Metric::Chamfer),
        FieldKind::ChokeInside => dist_outside_by(&s.map(|a| 1.0 - a), Metric::Chamfer),
        FieldKind::StrokeInside => dist_inside_by(&s, Metric::Chamfer),
        FieldKind::StrokeOutsideExact => dist_outside_by(&s, Metric::Euclidean),
        FieldKind::StrokeOutsideVector => dist_outside_by(&local_coverage(&s), Metric::Euclidean),
        FieldKind::StrokeOutsideVectorChamfer => dist_outside_by(&local_coverage(&s), Metric::Chamfer),
    }
}

/// Both chiselled bevel fields of `alpha`, `[BevelInside, BevelOutside]`: each is its
/// [`distance_field`], with the sharp edges they both start from found once.
pub fn bevel_distance_fields(alpha: Vec<f32>, w: usize, h: usize, reach: f32) -> [Vec<f32>; 2] {
    bevel_dists(&Map { w, h, v: alpha }, reach)
}

/// The inside and outside bevel distances of `s`, from one set of sharp edges.
pub(super) fn bevel_dists(s: &Map, reach: f32) -> [Vec<f32>; 2] {
    let sharp = sharp_edges(s);
    [bevel_dist_inside(s, &sharp, reach), bevel_dist_outside(s, &sharp, reach)]
}
