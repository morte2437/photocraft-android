//! Mesh warps (Edit › Transform › Warp, smart-object warps): resampling a surface through any
//! forward map, such as a [`photocraft_geom::warp::Warp`] (bicubic Bezier patches or a preset).
//!
//! The source rectangle is subdivided into a fine grid whose vertices are mapped forward; each
//! cell's two triangles are rasterized in destination space with the source position
//! interpolated barycentrically, then the source is sampled there (premultiplied, nearest /
//! bilinear / Photoshop's Keys bicubic, like Free Transform). Cells are a few pixels across in
//! both the source and the destination (a thin layer stretched far gets as many cells as its
//! warped size needs), so the piecewise-affine inverse is within a small fraction of a pixel of
//! the true one; affine
//! maps (the identity included) are reproduced exactly. Where the warp shrinks the source, each
//! pixel instead averages a grid of bilinear subsamples over its source footprint (about one per
//! source pixel), so thin lines don't alias. Where the warp folds over itself, [`Fold`] decides
//! what shows: for Edit › Transform › Warp and smart objects the part dragged furthest is on top and
//! the layers composite; for other mesh maps (lens corrections, distortions) the later
//! (lower-right) cell wins. Work is split per destination tile (parallel on native).

use photocraft_color::PixelFormat;
use photocraft_geom::{Rect, TileCoord};
use photocraft_raster::{Interrupt, Surface, Tile};
use std::sync::Arc;

use crate::transform::{Interp, bicubic};

/// Grid cell size (source px) used to approximate the map.
const CELL: f64 = 4.0;
/// Most cells along one axis (bounds memory and time on huge layers).
const MAX_CELLS: usize = 512;
/// How a mesh warp stacks the picture where it folds over itself.
#[derive(Clone, Copy)]
pub enum Fold<'a> {
    /// Cells in row order, the last one drawn wins (lens corrections, distortions).
    Rows,
    /// A warp of the picture in place (Edit › Transform › Warp, a smart object's warp): cells in
    /// [`cell_order`] (a part the fold turns over on top, else the part dragged furthest), and a fold's layers composite, so transparent
    /// parts show what is under them, as Warp mode's preview draws them. The map is where a
    /// source point lands without the warp: the identity for a layer, a smart object's placement.
    Stack(&'a (dyn Fn(f64, f64) -> (f64, f64) + Sync)),
}

impl Fold<'_> {
    /// [`Fold::Stack`] about the identity: a warp of a layer where it is.
    pub const IN_PLACE: Fold<'static> = Fold::Stack(&|x: f64, y: f64| (x, y));
}

/// Most subsamples per axis when a warp shrinks the source; also the longest source step per
/// destination pixel the filter follows (bounds the footprint each tile reads).
const MAX_SS: f64 = 8.0;
/// Bounding-box pixels scanned per destination pixel of a tile, across all its triangles.
const MAX_SCAN_PER_PIXEL: usize = 256;

/// Places a smart object's source image (`src`, its pixels in `src_rect`) in the document:
/// through `warp` (source space) and then the affine `t` (source → document) in one resampling
/// pass, an exact shift for whole-pixel translations (so conversions and re-renders are
/// lossless), bicubic otherwise. Shared by the engine's re-render and PSD export's filter cache.
pub fn place_source(
    src: &Surface,
    src_rect: Rect,
    t: &photocraft_geom::Affine,
    warp: Option<&photocraft_geom::warp::Warp>,
) -> Result<Surface, photocraft_raster::AllocationError> {
    let [a, b, c, d, e, f] = t.m;
    if let Some(w) = warp.filter(|w| !w.is_identity()) {
        let map = |x: f64, y: f64| {
            let (u, v) = w.map(x, y);
            (a * u + c * v + e, b * u + d * v + f)
        };
        // A fold's layers are ordered by how far the warp moved them, not the placement.
        let rest = |x: f64, y: f64| (a * x + c * y + e, b * x + d * y + f);
        return Ok(mesh(src, src_rect, &map, Fold::Stack(&rest), Interp::Bicubic, &Interrupt::NONE));
    }
    let near = |x: f64, y: f64| (x - y).abs() < 1e-9;
    if near(a, 1.0) && near(b, 0.0) && near(c, 0.0) && near(d, 1.0) && near(e, e.round()) && near(f, f.round()) {
        return Ok(crate::resample::translate_surface(src, e.round() as i32, f.round() as i32));
    }
    crate::transform::warp_surface(src, src_rect, &crate::transform::Homography([a, c, e, b, d, f, 0.0, 0.0, 1.0]), Interp::Bicubic)
}

/// [`place_source`] through a projective map `h` (source → document; Distort, Perspective): the
/// warp first (in source space), then `h`.
pub fn place_source_projective(
    src: &Surface,
    src_rect: Rect,
    h: &crate::transform::Homography,
    warp: Option<&photocraft_geom::warp::Warp>,
) -> Result<Surface, photocraft_raster::AllocationError> {
    if let Some(w) = warp.filter(|w| !w.is_identity()) {
        let map = |x: f64, y: f64| {
            let (u, v) = w.map(x, y);
            h.apply(u, v)
        };
        let rest = |x: f64, y: f64| h.apply(x, y);
        return Ok(mesh(src, src_rect, &map, Fold::Stack(&rest), Interp::Bicubic, &Interrupt::NONE));
    }
    crate::transform::warp_surface(src, src_rect, h, Interp::Bicubic)
}

/// Warps the content of `src` inside `src_rect` through the forward map `f` (source document
/// coordinates → destination document coordinates). Output has `src`'s format with alpha
/// added; pixels outside the warped area are transparent.
pub fn warp_mesh_surface(src: &Surface, src_rect: Rect, f: &(dyn Fn(f64, f64) -> (f64, f64) + Sync), interp: Interp) -> Surface {
    mesh(src, src_rect, f, Fold::Rows, interp, &Interrupt::NONE)
}

/// [`warp_mesh_surface`] with folds stacked as `fold` says, reporting progress per destination
/// tile; `None` once `ctl` is cancelled.
pub fn warp_mesh_surface_with(
    src: &Surface,
    src_rect: Rect,
    f: &(dyn Fn(f64, f64) -> (f64, f64) + Sync),
    fold: Fold,
    interp: Interp,
    ctl: &Interrupt,
) -> Option<Surface> {
    let out = mesh(src, src_rect, f, fold, interp, ctl);
    (!ctl.cancelled()).then_some(out)
}

/// The order Warp draws the cells of a vertex grid in (`row` vertices a row; `pos` where the warp
/// put each vertex, `rest` where it would be without it). Where a warp folds the picture over
/// itself, the part turned over lies on top, like a folded-back flap: a corner or edge dragged
/// across the picture, and also the edge row that folds back when the middle is pulled out past
/// it, which leaves the pulled part under that row. Within each, the cells moved least come first,
/// so the part dragged furthest is on top. Each cell is named by its top-left vertex. Shared by
/// the render and Warp mode's preview, so they stack folds alike.
pub fn cell_order(pos: &[[f64; 2]], rest: &[[f64; 2]], row: usize) -> Vec<usize> {
    if row < 2 || pos.len() != rest.len() {
        return Vec::new();
    }
    let rows = pos.len() / row;
    // A vertex with no finite movement (sent to infinity, as near a Distort's horizon) goes under,
    // not on top.
    let moved = |k: usize| match (pos.get(k), rest.get(k)) {
        (Some(p), Some(r)) => Some((p[0] - r[0]).hypot(p[1] - r[1])).filter(|m| m.is_finite()).unwrap_or(f64::NEG_INFINITY),
        _ => f64::NEG_INFINITY,
    };
    // Twice the signed area of a cell's quad: its sign is the cell's orientation (NaN when a corner
    // isn't finite, which counts as not turned over).
    let area = |pts: &[[f64; 2]], a: usize| {
        let q = [a, a + 1, a + row + 1, a + row].map(|k| pts.get(k).copied().unwrap_or([f64::NAN; 2]));
        (0..4).map(|i| q[i][0] * q[(i + 1) % 4][1] - q[(i + 1) % 4][0] * q[i][1]).sum::<f64>()
    };
    let mut cells: Vec<(bool, f64, usize)> = Vec::with_capacity(rows.saturating_sub(1) * (row - 1));
    for j in 0..rows.saturating_sub(1) {
        for i in 0..row - 1 {
            let a = j * row + i;
            let turned = area(pos, a) * area(rest, a) < 0.0;
            cells.push((turned, moved(a) + moved(a + 1) + moved(a + row) + moved(a + row + 1), a));
        }
    }
    let by = |x: &(bool, f64, usize), y: &(bool, f64, usize)| x.0.cmp(&y.0).then(x.1.total_cmp(&y.1));
    // A stable sort, so equal cells keep row order. On this thread alone: a render asks for the
    // order from a pool thread that other tiles wait on, and a sort shared with the pool could
    // hand that thread one of those tiles, which would wait for the order for ever.
    cells.sort_by(by);
    cells.into_iter().map(|(_, _, a)| a).collect()
}

/// Folds stack as `fold` says.
fn mesh(src: &Surface, src_rect: Rect, f: &(dyn Fn(f64, f64) -> (f64, f64) + Sync), fold: Fold, interp: Interp, ctl: &Interrupt) -> Surface {
    if src_rect.is_empty() {
        let fmt = src.format();
        return Surface::new(PixelFormat::new(fmt.mode, fmt.sample, true));
    }
    let (sw, sh) = (f64::from(src_rect.width()), f64::from(src_rect.height()));
    // Cells are a few pixels across on both sides of the map: a thin source stretched far (a
    // 3 px column scaled into a wide band, then bent) needs many cells, or it comes out straight.
    let (lu, lv) = mapped_lengths(src_rect, f);
    let cells = |s: f64, d: f64| ((s.max(d) / CELL).ceil() as usize).clamp(1, MAX_CELLS);
    let (nx, ny) = (cells(sw, lu), cells(sh, lv));
    // Forward-mapped vertices (and their source positions).
    let mut verts = Vec::with_capacity((nx + 1) * (ny + 1));
    for j in 0..=ny {
        for i in 0..=nx {
            let sx = f64::from(src_rect.x0) + sw * i as f64 / nx as f64;
            let sy = f64::from(src_rect.y0) + sh * j as f64 / ny as f64;
            let (dx, dy) = f(sx, sy);
            verts.push(([dx, dy], [sx, sy]));
        }
    }
    let w1 = nx + 1;
    // Drawn in row order; for a fold, a tile where two parts of the picture meet is drawn again in
    // [`cell_order`], each cell's two triangles ranked together.
    let mut tris = Vec::with_capacity(nx * ny * 2);
    for j in 0..ny {
        for i in 0..nx {
            let a = j * w1 + i;
            tris.push([a, a + 1, a + w1 + 1]);
            tris.push([a, a + w1 + 1, a + w1]);
        }
    }
    let rank = || -> Vec<u32> {
        let Fold::Stack(rest) = fold else { return Vec::new() };
        let pos: Vec<[f64; 2]> = verts.iter().map(|(d, _)| *d).collect();
        let at_rest: Vec<[f64; 2]> = verts
            .iter()
            .map(|(_, s)| {
                let r = rest(s[0], s[1]);
                [r.0, r.1]
            })
            .collect();
        let mut rank = vec![0u32; tris.len()];
        for (r, a) in cell_order(&pos, &at_rest, w1).into_iter().enumerate() {
            let c = (a / w1) * nx + a % w1;
            for (t, k) in [(2 * c, 2 * r), (2 * c + 1, 2 * r + 1)] {
                if let Some(slot) = rank.get_mut(t) {
                    *slot = u32::try_from(k).unwrap_or(u32::MAX);
                }
            }
        }
        rank
    };
    let stack: Option<&(dyn Fn() -> Vec<u32> + Sync)> = matches!(fold, Fold::Stack(_)).then_some(&rank);
    triangles(src, src_rect, &verts, &tris, interp, ctl, stack)
}

/// Probe lines per axis [`mapped_lengths`] follows through the map.
const PROBE: usize = 16;

/// The longest destination length of a source row and of a source column under `f`, measured
/// along polylines through a [`PROBE`]-step grid. Non-finite steps count as zero.
fn mapped_lengths(src_rect: Rect, f: &(dyn Fn(f64, f64) -> (f64, f64) + Sync)) -> (f64, f64) {
    let (x0, y0) = (f64::from(src_rect.x0), f64::from(src_rect.y0));
    let (sw, sh) = (f64::from(src_rect.width()), f64::from(src_rect.height()));
    let n = PROBE + 1;
    let mut pts = Vec::with_capacity(n * n);
    for j in 0..n {
        for i in 0..n {
            let (dx, dy) = f(x0 + sw * i as f64 / PROBE as f64, y0 + sh * j as f64 / PROBE as f64);
            pts.push([dx, dy]);
        }
    }
    let step = |a: [f64; 2], b: [f64; 2]| {
        let d = (b[0] - a[0]).hypot(b[1] - a[1]);
        if d.is_finite() { d } else { 0.0 }
    };
    let (mut lu, mut lv) = (0.0f64, 0.0f64);
    for k in 0..n {
        let (mut row, mut col) = (0.0, 0.0);
        for m in 1..n {
            if let (Some(&a), Some(&b)) = (pts.get(k * n + m - 1), pts.get(k * n + m)) {
                row += step(a, b);
            }
            if let (Some(&a), Some(&b)) = (pts.get((m - 1) * n + k), pts.get(m * n + k)) {
                col += step(a, b);
            }
        }
        lu = lu.max(row);
        lv = lv.max(col);
    }
    (lu, lv)
}

/// Rasterizes textured triangles: each vertex is `(destination, source)` in document pixels and
/// each triangle samples `src` (restricted to `src_rect`) through the barycentric
/// interpolation of its source positions. Later triangles in `tris` draw over earlier ones (the
/// order is the depth order). Output has `src`'s format with alpha added.
pub fn warp_triangles(src: &Surface, src_rect: Rect, verts: &[([f64; 2], [f64; 2])], tris: &[[usize; 3]], interp: Interp) -> Surface {
    triangles(src, src_rect, verts, tris, interp, &Interrupt::NONE, None)
}

/// [`warp_triangles`], reporting progress per destination tile to `ctl`. A cancelled `ctl` skips
/// the remaining tiles; the caller discards the result.
/// Destination tiles one triangle render may keep bookkeeping for (a 1M-tile render is 69 Gpx).
const MAX_DST_TILES: u64 = 1 << 20;
/// How close (source px) two triangles' source points under one pixel are when they are the
/// same layer of the picture meeting on a shared edge, rather than two layers of a fold.
const SAME_LAYER: f64 = 1e-3;
/// Source pixels one destination tile may read densely.
const MAX_FOOTPRINT_PIXELS: u64 = 1 << 25;

/// Source points a destination tile samples, grouped into connected regions of the source, so
/// each region is read as a window of its own: a fold joins distant parts of the source in one
/// tile, and one window over both read everything between them (a corner dragged over the
/// middle of a 24 MP layer read seven times its pixels). Points are joined through the cells of a
/// coarse grid over their bounding box that their footprints touch.
struct Regions {
    /// The grid's origin, cell size, columns and rows.
    origin: [f64; 2],
    cell: f64,
    cols: usize,
    rows: usize,
    /// Each cell's region (`u32::MAX`: none). Empty for a single region, which every point is in.
    of_cell: Vec<u32>,
    /// Each region's source rectangle (the points' footprints, a 3 px border, inside `src_rect`).
    rects: Vec<Rect>,
}

/// Visits points for [`Regions::new`]: each with the half extent of its footprint along x and y.
type VisitPoints<'a> = &'a dyn Fn(&mut dyn FnMut([f64; 2], [f64; 2]));

/// Most cells per axis of a [`Regions`] grid.
const REGION_GRID: f64 = 256.0;
/// Smallest [`Regions`] cell (source px).
const REGION_CELL: f64 = 32.0;
/// A [`Regions`] bounding box this small (source px) stays one region.
const SINGLE_REGION_PIXELS: f64 = 1024.0 * 1024.0;

impl Regions {
    /// `points` visits each point with the half extent of its footprint along x and y (it is
    /// called again only when the points form more than one region).
    fn new(points: VisitPoints, src_rect: Rect) -> Regions {
        let mut b = [f64::MAX, f64::MAX, f64::MIN, f64::MIN];
        let mut count = 0usize;
        points(&mut |q, e| {
            b = [b[0].min(q[0] - e[0]), b[1].min(q[1] - e[1]), b[2].max(q[0] + e[0]), b[3].max(q[1] + e[1])];
            count += 1;
        });
        let empty = Regions { origin: [0.0; 2], cell: 1.0, cols: 0, rows: 0, of_cell: Vec::new(), rects: Vec::new() };
        if !(b[0] <= b[2] && b.iter().all(|v| v.is_finite())) {
            return empty;
        }
        let rect =
            |r: [f64; 4]| Rect::new(r[0].floor() as i32 - 3, r[1].floor() as i32 - 3, r[2].ceil() as i32 + 3, r[3].ceil() as i32 + 3).intersect(&src_rect);
        // Most tiles sample one stretch of the source: no more than a few times the source
        // pixels there are points (the footprints' own spread), or a small window, is one region.
        let area = (b[2] - b[0] + 7.0) * (b[3] - b[1] + 7.0);
        if area <= SINGLE_REGION_PIXELS.max(4.0 * count as f64) {
            return Regions { rects: vec![rect(b)], ..empty };
        }
        let cell = ((b[2] - b[0]).max(b[3] - b[1]) / REGION_GRID).max(REGION_CELL);
        let (cols, rows) = (((b[2] - b[0]) / cell).floor() as usize + 1, ((b[3] - b[1]) / cell).floor() as usize + 1);
        let to = |v: f64, o: f64, n: usize| (((v - o) / cell).floor().max(0.0) as usize).min(n - 1);
        // Cells a footprint touches, marked (0) then labelled by region.
        let mut of_cell = vec![u32::MAX; cols * rows];
        points(&mut |q, e| {
            for y in to(q[1] - e[1], b[1], rows)..=to(q[1] + e[1], b[1], rows) {
                for x in to(q[0] - e[0], b[0], cols)..=to(q[0] + e[0], b[0], cols) {
                    if let Some(c) = of_cell.get_mut(y * cols + x) {
                        *c = 0;
                    }
                }
            }
        });
        let mut regions = 0u32;
        let mut stack = Vec::new();
        for start in 0..of_cell.len() {
            if of_cell.get(start) != Some(&0) {
                continue;
            }
            regions += 1;
            stack.push(start);
            while let Some(c) = stack.pop() {
                if of_cell.get(c) != Some(&0) {
                    continue;
                }
                of_cell[c] = regions;
                let (x, y) = ((c % cols) as i64, (c / cols) as i64);
                for (dx, dy) in [(-1, -1), (0, -1), (1, -1), (-1, 0), (1, 0), (-1, 1), (0, 1), (1, 1)] {
                    let (nx, ny) = (x + dx, y + dy);
                    if nx >= 0 && ny >= 0 && (nx as usize) < cols && (ny as usize) < rows {
                        stack.push(ny as usize * cols + nx as usize);
                    }
                }
            }
        }
        // Labels 1.. become indices 0..; each region's rectangle covers its points' footprints.
        for c in of_cell.iter_mut().filter(|c| **c != u32::MAX) {
            *c -= 1;
        }
        let mut boxes = vec![[f64::MAX, f64::MAX, f64::MIN, f64::MIN]; regions as usize];
        let mut out = Regions { origin: [b[0], b[1]], cell, cols, rows, of_cell, rects: Vec::new() };
        points(&mut |q, e| {
            if let Some(r) = out.of(q).and_then(|r| boxes.get_mut(r)) {
                *r = [r[0].min(q[0] - e[0]), r[1].min(q[1] - e[1]), r[2].max(q[0] + e[0]), r[3].max(q[1] + e[1])];
            }
        });
        out.rects = boxes.into_iter().map(rect).collect();
        out
    }

    /// The region of a point given to [`Regions::new`].
    fn of(&self, q: [f64; 2]) -> Option<usize> {
        if self.of_cell.is_empty() {
            return (!self.rects.is_empty()).then_some(0);
        }
        if !(q[0].is_finite() && q[1].is_finite()) {
            return None;
        }
        let x = (((q[0] - self.origin[0]) / self.cell).floor().max(0.0) as usize).min(self.cols - 1);
        let y = (((q[1] - self.origin[1]) / self.cell).floor().max(0.0) as usize).min(self.rows - 1);
        self.of_cell.get(y * self.cols + x).filter(|&&r| r != u32::MAX).map(|&r| r as usize)
    }

    /// Source pixels all the regions' rectangles hold.
    fn area(&self) -> u64 {
        self.rects.iter().map(|r| r.size().area()).sum()
    }
}

/// A premultiplied window of the source that a destination tile samples.
struct Win {
    px: Vec<f32>,
    foot: Rect,
    fw: usize,
}

/// `stack`: the picture can fold over itself, and gives each triangle's rank in the order a fold
/// stacks them (asked once, when a tile first needs it). A tile is drawn in `tris`' order; where
/// two parts of the source meet at a pixel (a fold), the tile is drawn again in rank order, and
/// triangles drawn later composite over what they cover instead of replacing it. Without a fold
/// the order makes no difference, so a warp that doesn't fold costs no more than one that can't.
fn triangles(
    src: &Surface,
    src_rect: Rect,
    verts: &[([f64; 2], [f64; 2])],
    tris: &[[usize; 3]],
    interp: Interp,
    ctl: &Interrupt,
    stack: Option<&(dyn Fn() -> Vec<u32> + Sync)>,
) -> Surface {
    let folds = stack.is_some();
    let ranks = std::sync::OnceLock::new();
    let rank_of = |t: u32| stack.map(|f| ranks.get_or_init(f)).and_then(|r| r.get(t as usize)).copied().unwrap_or(u32::MAX);
    let mut fmt = src.format();
    let converted;
    let src = if fmt.alpha {
        src
    } else {
        fmt = PixelFormat::new(fmt.mode, fmt.sample, true);
        converted = src.convert(fmt);
        &converted
    };
    let mut out = Surface::new(fmt);
    if src_rect.is_empty() || tris.is_empty() {
        return out;
    }
    if verts.iter().any(|v| !v.0[0].is_finite() || !v.0[1].is_finite()) {
        return out;
    }
    let lim = f64::from(1 << 20);
    let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
    for v in tris.iter().flatten().map(|&i| &verts[i]) {
        x0 = x0.min(v.0[0]);
        y0 = y0.min(v.0[1]);
        x1 = x1.max(v.0[0]);
        y1 = y1.max(v.0[1]);
    }
    let dst = Rect::new(x0.max(-lim).floor() as i32 - 1, y0.max(-lim).floor() as i32 - 1, x1.min(lim).ceil() as i32 + 1, y1.min(lim).ceil() as i32 + 1);
    // Bookkeeping is kept per destination tile, so a mesh flung across the whole coordinate range
    // would ask for millions of entries before drawing anything: draw nothing instead.
    let ts_px = photocraft_geom::TILE_SIZE as u64;
    if u64::from(dst.width()).div_ceil(ts_px).saturating_add(1).saturating_mul(u64::from(dst.height()).div_ceil(ts_px).saturating_add(1)) > MAX_DST_TILES {
        return out;
    }
    // Triangles: (vertex indices), binned per destination tile.
    let tiles: Vec<Rect> = dst.tiles().map(|tc| tc.rect().intersect(&dst)).filter(|r| !r.is_empty()).collect();
    // Triangles by index into `tris`, in its order.
    let mut bins: Vec<Vec<u32>> = vec![Vec::new(); tiles.len()];
    let ts = photocraft_geom::TILE_SIZE;
    // Bin origin on the tile grid (the first tile is clipped to `dst`, so its x0/y0 may not be).
    let tx0 = tiles.iter().map(|t| t.x0).min().unwrap_or(0).div_euclid(ts) * ts;
    let ty0 = tiles.iter().map(|t| t.y0).min().unwrap_or(0).div_euclid(ts) * ts;
    let cols = tiles.iter().map(|t| (t.x0 - tx0) / ts).max().unwrap_or(0) as usize + 1;
    let grid_of = |t: &Rect| ((t.y0 - ty0) / ts) as usize * cols + ((t.x0 - tx0) / ts) as usize;
    let mut lookup = vec![usize::MAX; cols * (tiles.iter().map(|t| (t.y0 - ty0) / ts).max().unwrap_or(0) as usize + 1)];
    for (k, t) in tiles.iter().enumerate() {
        lookup[grid_of(t)] = k;
    }
    for (index, &tri) in tris.iter().enumerate() {
        let Ok(index) = u32::try_from(index) else { break };
        let ps = tri.map(|v| verts[v].0);
        let bx0 = ps.iter().map(|p| p[0]).fold(f64::MAX, f64::min).floor() as i32;
        let by0 = ps.iter().map(|p| p[1]).fold(f64::MAX, f64::min).floor() as i32;
        let bx1 = ps.iter().map(|p| p[0]).fold(f64::MIN, f64::max).ceil() as i32;
        let by1 = ps.iter().map(|p| p[1]).fold(f64::MIN, f64::max).ceil() as i32;
        // Degenerate triangles draw nothing (`work` skips them the same way): do not bin them.
        let det = (ps[1][0] - ps[0][0]) * (ps[2][1] - ps[0][1]) - (ps[2][0] - ps[0][0]) * (ps[1][1] - ps[0][1]);
        if det.is_nan() || det.abs() < 1e-12 {
            continue;
        }
        let (cx0, cy0) = ((bx0.max(dst.x0) - tx0).div_euclid(ts), (by0.max(dst.y0) - ty0).div_euclid(ts));
        let (cx1, cy1) = ((bx1.min(dst.x1 - 1) - tx0).div_euclid(ts), (by1.min(dst.y1 - 1) - ty0).div_euclid(ts));
        for cy in cy0.max(0)..=cy1 {
            for cx in cx0.max(0)..=cx1 {
                let g = cy as usize * cols + cx as usize;
                if let Some(&k) = lookup.get(g)
                    && k != usize::MAX
                {
                    bins[k].push(index);
                }
            }
        }
    }
    let n = fmt.channels();
    let a = n - 1;
    // Tiles finished so far, for progress reported per tile (from any worker thread).
    let finished = std::sync::atomic::AtomicUsize::new(0);
    let total = tiles.len().max(1);
    let work = |(t, bin): (&Rect, &Vec<u32>)| -> Option<(TileCoord, Arc<Tile>)> {
        if ctl.cancelled() {
            return None;
        }
        let k = finished.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
        // The last few percent are putting the tiles in place.
        ctl.progress(0.98 * k as f32 / total as f32);
        if bin.is_empty() {
            return None;
        }
        let w = t.width() as usize;
        let h = t.height() as usize;
        // Subsamples along one destination axis, from the source step `j` it makes: one where
        // the warp keeps or enlarges the source, else about one per source pixel the output pixel
        // spans (an area average, as the transform path gets from its pre-reduction; a single
        // bicubic sample aliases thin lines when shrinking). The subsamples are bilinear: the
        // box does the filtering, and it is ~4x cheaper than bicubic at no loss.
        let count = |j: [f64; 2]| if interp == Interp::Nearest { 1 } else { j[0].hypot(j[1]).round().clamp(1.0, MAX_SS) as usize };
        // Source position per destination pixel (NaN = not covered) and the triangle it came
        // from. Per triangle: its source step per destination pixel along x and y (`[dx_u, dx_v,
        // dy_u, dy_v]`, the inverse map's Jacobian, constant over a triangle; zero when
        // point-sampled) and the subsamples along each.
        // The tile's triangles, first in `tris`' order, then, if a fold shows here, by rank.
        let mut order: Vec<[usize; 3]> = bin.iter().filter_map(|&k| tris.get(k as usize).copied()).collect();
        let mut stacked = false;
        let mut uv = vec![[f64::NAN; 2]; w * h];
        let mut owner = vec![0u32; w * h];
        // The layers of a fold under a pixel's top one, bottom first, as a list linked through
        // `under` (source position, triangle, next node or `u32::MAX`) from the node `under_at`
        // holds for the pixel (`u32::MAX`: none): one arena for the tile rather than a list per
        // folded pixel. A layer taken out again keeps its node, its position NaN.
        let mut under: Vec<([f64; 2], u32, u32)> = Vec::new();
        let mut under_at: Vec<u32> = Vec::new();
        let mut jac: Vec<([f64; 4], usize, usize)> = Vec::with_capacity(order.len());
        'pass: loop {
            // Folded warps pile thousands of triangles over the same pixels: the bounding boxes
            // scanned per tile are capped (a sound warp scans a small multiple of the tile's area).
            let mut scan_left = w * h * MAX_SCAN_PER_PIXEL + (1 << 16);
            for (k, tri) in order.iter().enumerate() {
                let [(p0, s0), (p1, s1), (p2, s2)] = tri.map(|v| verts[v]);
                let det = (p1[0] - p0[0]) * (p2[1] - p0[1]) - (p2[0] - p0[0]) * (p1[1] - p0[1]);
                if det.abs() < 1e-12 {
                    jac.push(([0.0; 4], 1, 1));
                    continue;
                }
                let (d1, d2) = ([s1[0] - s0[0], s1[1] - s0[1]], [s2[0] - s0[0], s2[1] - s0[1]]);
                let (l1x, l1y, l2x, l2y) = ((p2[1] - p0[1]) / det, (p0[0] - p2[0]) / det, (p0[1] - p1[1]) / det, (p1[0] - p0[0]) / det);
                // Capped, so a sliver triangle can't ask for an unbounded footprint.
                let cap = |j: [f64; 2]| {
                    let l = j[0].hypot(j[1]);
                    if l > MAX_SS { [j[0] * MAX_SS / l, j[1] * MAX_SS / l] } else { j }
                };
                let jx = cap([l1x * d1[0] + l2x * d2[0], l1x * d1[1] + l2x * d2[1]]);
                let jy = cap([l1y * d1[0] + l2y * d2[0], l1y * d1[1] + l2y * d2[1]]);
                let (nx, ny) = (count(jx), count(jy));
                jac.push(if nx * ny == 1 { ([0.0; 4], 1, 1) } else { ([jx[0], jx[1], jy[0], jy[1]], nx, ny) });
                let bx0 = (p0[0].min(p1[0]).min(p2[0]) - 0.5).floor().max(f64::from(t.x0)) as i32;
                let by0 = (p0[1].min(p1[1]).min(p2[1]) - 0.5).floor().max(f64::from(t.y0)) as i32;
                let bx1 = (p0[0].max(p1[0]).max(p2[0]) + 0.5).ceil().min(f64::from(t.x1)) as i32;
                let by1 = (p0[1].max(p1[1]).max(p2[1]) + 0.5).ceil().min(f64::from(t.y1)) as i32;
                let eps = -1e-9;
                let scan = (bx1 - bx0).max(0) as usize * (by1 - by0).max(0) as usize;
                // A triangle that no longer fits is skipped, not the end of the tile: later (smaller) ones still draw.
                if scan > scan_left {
                    continue;
                }
                scan_left -= scan;
                for y in by0..by1 {
                    let py = f64::from(y) + 0.5;
                    for x in bx0..bx1 {
                        let px = f64::from(x) + 0.5;
                        let l1 = ((px - p0[0]) * (p2[1] - p0[1]) - (p2[0] - p0[0]) * (py - p0[1])) / det;
                        let l2 = ((p1[0] - p0[0]) * (py - p0[1]) - (px - p0[0]) * (p1[1] - p0[1])) / det;
                        let l0 = 1.0 - l1 - l2;
                        if l0 < eps || l1 < eps || l2 < eps {
                            continue;
                        }
                        let o = (y - t.y0) as usize * w + (x - t.x0) as usize;
                        let q = [l0 * s0[0] + l1 * s1[0] + l2 * s2[0], l0 * s0[1] + l1 * s1[1] + l2 * s2[1]];
                        // Another part of the source: a fold. Triangles of one layer meeting at a pixel
                        // (on their shared edge) interpolate the same source point, to float noise,
                        // and replace each other; one drawn again after another layer comes back on
                        // top of it, as painting would put it. Layers of a tight fold can be well under
                        // a source pixel apart and still be layers.
                        let near = |a: [f64; 2]| (a[0] - q[0]).hypot(a[1] - q[1]) <= SAME_LAYER;
                        if folds && !stacked && !uv[o][0].is_nan() && !near(uv[o]) {
                            // A fold shows in this tile: draw it again in the fold's order.
                            let mut ranked: Vec<(u32, [usize; 3])> = bin.iter().filter_map(|&k| tris.get(k as usize).map(|&t| (rank_of(k), t))).collect();
                            ranked.sort_by_key(|e| e.0);
                            order = ranked.into_iter().map(|e| e.1).collect();
                            stacked = true;
                            uv.fill([f64::NAN; 2]);
                            under_at = vec![u32::MAX; w * h];
                            jac.clear();
                            continue 'pass;
                        }
                        if stacked
                            && !uv[o][0].is_nan()
                            && !near(uv[o])
                            && let Some(head) = under_at.get_mut(o)
                            && let Ok(node) = u32::try_from(under.len())
                        {
                            // This layer replaces a layer of itself already in the list; the old top
                            // goes on the end.
                            let (mut at, mut last) = (*head, u32::MAX);
                            *head = u32::MAX;
                            while let Some(&(p, _, next)) = under.get(at as usize) {
                                if near(p) {
                                    if let Some(e) = under.get_mut(at as usize) {
                                        e.0 = [f64::NAN; 2];
                                    }
                                } else {
                                    match under.get_mut(last as usize) {
                                        Some(prev) => prev.2 = at,
                                        None => *head = at,
                                    }
                                    last = at;
                                }
                                at = next;
                            }
                            under.push((uv[o], owner[o], u32::MAX));
                            match under.get_mut(last as usize) {
                                Some(prev) => prev.2 = node,
                                None => *head = node,
                            }
                        }
                        uv[o] = q;
                        owner[o] = k as u32;
                    }
                }
            }
            break;
        }
        // Where the covered pixels sample the source, with the spread of their subsamples, in
        // regions read apart ([`Regions`]): the top layer's and, apart, those of the fold layers
        // under it.
        let spread = |(q, k): ([f64; 2], u32)| {
            let j = jac[k as usize].0;
            (q, [0.5 * (j[0].abs() + j[2].abs()), 0.5 * (j[1].abs() + j[3].abs())])
        };
        // Sparse components far apart in the source can land in one region: the dense window
        // between them is not read (it stays transparent). The pixel-area cap is checked first:
        // `has_tiles_in` walks the source tile grid, so an oversized footprint must return
        // without paying for that scan.
        let window = |foot: Rect| -> Option<Win> {
            if foot.is_empty() || foot.size().area() > MAX_FOOTPRINT_PIXELS || !src.has_tiles_in(foot) {
                return None;
            }
            let mut px = src.read_region(foot);
            for p in px.chunks_exact_mut(n) {
                let al = p[a];
                for v in &mut p[..a] {
                    *v *= al;
                }
            }
            Some(Win { px, foot, fw: foot.width() as usize })
        };
        // A top layer over an empty part of the source has no window: it is transparent there,
        // and the fold layers under it still show. Too much to read leaves the tile empty, as
        // without folds: what is under it can't show through where it may be opaque.
        let top_regions = Regions::new(
            &|f| {
                for (q, &k) in uv.iter().zip(&owner).filter(|(q, _)| !q[0].is_nan()) {
                    let (q, e) = spread((*q, k));
                    f(q, e);
                }
            },
            src_rect,
        );
        if top_regions.area() > MAX_FOOTPRINT_PIXELS {
            return None;
        }
        let top: Vec<Option<Win>> = top_regions.rects.iter().map(|&r| window(r)).collect();
        // Fold layers whose source can't be read in windows of their own, within the one cap for
        // both, are left out (the top one stays).
        let below_regions = Regions::new(
            &|f| {
                for &(q, k, _) in under.iter().filter(|e| !e.0[0].is_nan()) {
                    let (q, e) = spread((q, k));
                    f(q, e);
                }
            },
            src_rect,
        );
        let room = MAX_FOOTPRINT_PIXELS.saturating_sub(top_regions.area());
        let below: Vec<Option<Win>> =
            if !under.is_empty() && below_regions.area() <= room { below_regions.rects.iter().map(|&r| window(r)).collect() } else { Vec::new() };
        if below.iter().all(Option::is_none) {
            under.clear();
        }
        if top.iter().all(Option::is_none) && under.is_empty() {
            return None;
        }
        // The window a point of the top layer, or of a fold layer under it, samples.
        let top_win = |q: [f64; 2]| top_regions.of(q).and_then(|r| top.get(r)).and_then(Option::as_ref);
        let below_win = |q: [f64; 2]| below_regions.of(q).and_then(|r| below.get(r)).and_then(Option::as_ref);
        let at = |win: &Win, x: i32, y: i32, c: usize| -> f64 {
            let foot = win.foot;
            if x < foot.x0 || y < foot.y0 || x >= foot.x1 || y >= foot.y1 {
                0.0
            } else {
                f64::from(win.px[((y - foot.y0) as usize * win.fw + (x - foot.x0) as usize) * n + c])
            }
        };
        // Offset of pixel (x, y) when the `taps` x `taps` block from it lies inside `foot`, so
        // the samplers can index `px` directly instead of bounds-checking every tap.
        let base = |win: &Win, x: i32, y: i32, taps: i32| -> Option<usize> {
            let foot = win.foot;
            (x >= foot.x0 && y >= foot.y0 && x + taps <= foot.x1 && y + taps <= foot.y1).then(|| ((y - foot.y0) as usize * win.fw + (x - foot.x0) as usize) * n)
        };
        let mut outp = vec![0.0f32; w * h * n];
        let mut any = false;
        let mut acc = [0.0f64; 8];
        let mut one = [0.0f64; 8];
        // One premultiplied sample at source position (u, v) (pixel centres at integers).
        let point = |win: &Win, interp: Interp, u: f64, v: f64, acc: &mut [f64; 8]| {
            acc[..n].fill(0.0);
            match interp {
                Interp::Nearest => {
                    let (ix, iy) = ((u + 0.5).floor() as i32, (v + 0.5).floor() as i32);
                    for (c, s) in acc.iter_mut().enumerate().take(n) {
                        *s = at(win, ix, iy, c);
                    }
                }
                Interp::Bilinear => {
                    let (ix, iy) = (u.floor() as i32, v.floor() as i32);
                    let (fx, fy) = (u - f64::from(ix), v - f64::from(iy));
                    if let Some(o) = base(win, ix, iy, 2) {
                        for (dy, wy) in [(0, 1.0 - fy), (1, fy)] {
                            for (dx, wx) in [(0, 1.0 - fx), (1, fx)] {
                                let p = &win.px[o + (dy * win.fw + dx) * n..][..n];
                                for (s, v) in acc.iter_mut().zip(p) {
                                    *s += f64::from(*v) * wx * wy;
                                }
                            }
                        }
                        return;
                    }
                    for (dy, wy) in [(0, 1.0 - fy), (1, fy)] {
                        for (dx, wx) in [(0, 1.0 - fx), (1, fx)] {
                            for (c, s) in acc.iter_mut().enumerate().take(n) {
                                *s += at(win, ix + dx, iy + dy, c) * wx * wy;
                            }
                        }
                    }
                }
                Interp::Bicubic => {
                    // Snap to the pixel grid when within float noise, so identity is exact.
                    let snap = |t: f64| if (t - t.round()).abs() < 1e-7 { t.round() } else { t };
                    let (u, v) = (snap(u), snap(v));
                    let (ix, iy) = (u.floor() as i32, v.floor() as i32);
                    let (wx, wy) = (bicubic(u - f64::from(ix)), bicubic(v - f64::from(iy)));
                    if let Some(o) = base(win, ix - 1, iy - 1, 4) {
                        for (j, wyj) in wy.iter().enumerate() {
                            for (i, wxi) in wx.iter().enumerate() {
                                let k = wxi * wyj;
                                if k == 0.0 {
                                    continue;
                                }
                                let p = &win.px[o + (j * win.fw + i) * n..][..n];
                                for (s, v) in acc.iter_mut().zip(p) {
                                    *s += f64::from(*v) * k;
                                }
                            }
                        }
                        return;
                    }
                    for (j, wyj) in wy.iter().enumerate() {
                        for (i, wxi) in wx.iter().enumerate() {
                            let k = wxi * wyj;
                            if k == 0.0 {
                                continue;
                            }
                            for (c, s) in acc.iter_mut().enumerate().take(n) {
                                *s += at(win, ix - 1 + i as i32, iy - 1 + j as i32, c) * k;
                            }
                        }
                    }
                }
            }
        };
        // A premultiplied sample of triangle `k`'s source at (u, v): one point, or the average of
        // its subsamples where the warp shrinks the source.
        let sample = |win: &Win, q: [f64; 2], k: u32, acc: &mut [f64; 8], one: &mut [f64; 8]| {
            let (j, nx, ny) = jac[k as usize];
            if nx * ny == 1 {
                point(win, interp, q[0] - 0.5, q[1] - 0.5, acc);
                return;
            }
            acc[..n].fill(0.0);
            for sy in 0..ny {
                let t = (sy as f64 + 0.5) / ny as f64 - 0.5;
                for sx in 0..nx {
                    let s = (sx as f64 + 0.5) / nx as f64 - 0.5;
                    point(win, Interp::Bilinear, q[0] + s * j[0] + t * j[2] - 0.5, q[1] + s * j[1] + t * j[3] - 0.5, one);
                    for (c, v) in acc.iter_mut().zip(one.iter()).take(n) {
                        *c += v;
                    }
                }
            }
            let k = 1.0 / (nx * ny) as f64;
            for c in acc.iter_mut().take(n) {
                *c *= k;
            }
        };
        let mut layer = [0.0f64; 8];
        for (o, (q, &k)) in uv.iter().zip(&owner).enumerate() {
            if q[0].is_nan() {
                continue;
            }
            match top_win(*q) {
                Some(tw) => sample(tw, *q, k, &mut acc, &mut one),
                None => acc[..n].fill(0.0),
            }
            // A fold: its layers under this one, bottom first, composite under the top (unless
            // the top covers them).
            let first = under_at.get(o).copied().unwrap_or(u32::MAX);
            if first != u32::MAX && !under.is_empty() && acc[a] < 1.0 {
                let mut under_top = [0.0f64; 8];
                let mut at = first;
                while let Some(&(q, k, next)) = under.get(at as usize) {
                    at = next;
                    match below_win(q) {
                        Some(bw) => sample(bw, q, k, &mut layer, &mut one),
                        None => layer[..n].fill(0.0),
                    }
                    let keep = 1.0 - layer[a].clamp(0.0, 1.0);
                    for c in 0..n {
                        under_top[c] = layer[c] + under_top[c] * keep;
                    }
                }
                let keep = 1.0 - acc[a].clamp(0.0, 1.0);
                for c in 0..n {
                    acc[c] += under_top[c] * keep;
                }
            }
            let al = acc[a].clamp(0.0, 1.0);
            // Float noise (a vertex a hair off the pixel grid) must not leave invisible
            // colour behind in transparent pixels.
            if al <= 1e-6 {
                continue;
            }
            any = true;
            let b = o * n;
            for c in 0..a {
                // The unclamped alpha, so a bicubic overshoot past 1 doesn't lighten the colour (#6203).
                outp[b + c] = (acc[c] / acc[a]).clamp(0.0, 1.0) as f32;
            }
            outp[b + a] = al as f32;
        }
        if !any {
            return None;
        }
        // Encoded (and dropped if fully transparent) here, in parallel, rather than one tile at
        // a time once all are done. `t` lies in one tile.
        let mut one = Surface::new(fmt);
        one.write_region(*t, &outp);
        one.prune();
        if one.tile_count() == 0 {
            return None;
        }
        one.take_tiles(*t).pop()
    };
    #[cfg(not(target_arch = "wasm32"))]
    let done: Vec<(TileCoord, Arc<Tile>)> = {
        use rayon::prelude::*;
        tiles.par_iter().zip(bins.par_iter()).filter_map(work).collect()
    };
    #[cfg(target_arch = "wasm32")]
    let done: Vec<(TileCoord, Arc<Tile>)> = tiles.iter().zip(bins.iter()).filter_map(work).collect();
    if ctl.cancelled() {
        return out;
    }
    out.put_tiles(done);
    out
}

/// [`warp_mesh_surface`] for single-channel surfaces whose outside reads as their default value
/// (layer masks, selections): the content is warped with alpha and flattened back onto the
/// default.
pub fn warp_mesh_gray(s: &Surface, f: &(dyn Fn(f64, f64) -> (f64, f64) + Sync), interp: Interp) -> Surface {
    warp_mesh_gray_with(s, f, Fold::Rows, interp)
}

/// [`warp_mesh_gray`] with folds stacked as `fold` says: a layer's mask and its selection follow
/// the layer's pixels, the part on top of a fold included.
pub fn warp_mesh_gray_with(s: &Surface, f: &(dyn Fn(f64, f64) -> (f64, f64) + Sync), fold: Fold, interp: Interp) -> Surface {
    let default = s.default_pixel().first().copied().unwrap_or(0.0);
    let fmt = s.format();
    let src = s.content_bounds();
    let mut out = Surface::with_default(fmt, &[default]);
    if src.is_empty() {
        return out;
    }
    let with_alpha = PixelFormat::new(fmt.mode, fmt.sample, true);
    let mut tmp = Surface::new(with_alpha);
    let v = s.read_region(src);
    tmp.write_region(src, &v.iter().flat_map(|g| [*g, 1.0]).collect::<Vec<f32>>());
    let w = mesh(&tmp, src, f, fold, interp, &Interrupt::NONE);
    // Clear the old region and write the warped one separately, as `warp_gray` does: one dense
    // `src ∪ warped` buffer grows with the square of how far a mesh moves the content (#995).
    // `Surface` is tile-sparse, so a distant region costs only its own tiles.
    out.write_region(src, &vec![default; src.width() as usize * src.height() as usize]);
    let b = w.content_bounds();
    if !b.is_empty() {
        let px = w.read_region(b);
        let flat: Vec<f32> = px.as_chunks::<2>().0.iter().map(|p| p[0] * p[1] + default * (1.0 - p[1])).collect();
        out.write_region(b, &flat);
    }
    out.prune();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Edit › Transform › Warp folding the picture over itself (#8927): the layers stack as the
    /// preview draws them, the part dragged furthest on top, and composite, so a transparent part on top
    /// shows the content folded under it, also where the top part is a stretch of the source with
    /// no pixels at all (no tiles). Layers aren't composited twice, and other mesh maps keep row
    /// order.
    #[test]
    fn a_fold_shows_what_is_under_its_transparent_parts() {
        let fmt = PixelFormat::new(ColorMode::Rgb, SampleType::U8, true);
        let ts = photocraft_geom::TILE_SIZE;
        for width in [100, 4 * ts] {
            let half = width / 2;
            // The red left half stays; the right half, transparent (no pixels), folds back over it.
            let mut src = Surface::new(fmt);
            src.fill_rect(Rect::new(0, 0, half, 10), &[1.0, 0.0, 0.0, 1.0]);
            let w = f64::from(width);
            let fold = move |x: f64, y: f64| (if x <= w / 2.0 { x } else { w - x }, y);
            let out = warp_mesh_surface_with(&src, Rect::new(0, 0, width, 10), &fold, Fold::IN_PLACE, Interp::Bilinear, &Interrupt::NONE).unwrap();
            let px = out.read_region(Rect::new(0, 5, half, 6));
            let alpha: Vec<f32> = px.as_chunks::<4>().0.iter().map(|p| p[3]).collect();
            let lit = alpha.iter().filter(|a| **a > 0.99).count();
            assert!(lit as i32 >= half - 4, "width {width}: the red under the transparent top shows ({lit} of {half} px)");
            // The generic mesh warp keeps row order: the last drawn (the right half, empty) wins.
            let plain = warp_mesh_surface(&src, Rect::new(0, 0, width, 10), &fold, Interp::Bilinear);
            assert!(plain.read_region(Rect::new(half / 2, 5, half / 2 + 1, 6))[3] < 0.01);
        }
        // Two half-transparent layers composite: 0.75.
        let mut half = Surface::new(fmt);
        half.fill_rect(Rect::new(0, 0, 100, 10), &[0.0, 0.0, 1.0, 0.5]);
        let fold = |x: f64, y: f64| (if x <= 50.0 { x } else { 100.0 - x }, y);
        let out = warp_mesh_surface_with(&half, Rect::new(0, 0, 100, 10), &fold, Fold::IN_PLACE, Interp::Bilinear, &Interrupt::NONE).unwrap();
        let a = out.read_region(Rect::new(20, 5, 21, 6))[3];
        assert!((a - 0.75).abs() < 0.02, "two half-transparent layers: alpha {a}");
        // No fold: every pixel of a half-transparent picture stays at 0.5 (no layer counted twice
        // along the cells' edges).
        let mut big = Surface::new(fmt);
        big.fill_rect(Rect::new(0, 0, 200, 200), &[0.0, 0.0, 1.0, 0.5]);
        let bend = |x: f64, y: f64| (x + 6.0 * (y / 30.0).sin(), y + 6.0 * (x / 40.0).cos());
        let out = warp_mesh_surface_with(&big, Rect::new(0, 0, 200, 200), &bend, Fold::IN_PLACE, Interp::Bilinear, &Interrupt::NONE).unwrap();
        let px = out.read_region(Rect::new(20, 20, 180, 180));
        let worst = px.as_chunks::<4>().0.iter().map(|p| (p[3] - 0.5).abs()).fold(0.0f32, f32::max);
        assert!(worst < 0.02, "a warp without a fold keeps the alpha: off by {worst}");
        // A tight fold, magnified: the layers are under a source pixel apart and still composite.
        let mut thin = Surface::new(fmt);
        thin.fill_rect(Rect::new(0, 0, 20, 2), &[0.0, 0.0, 1.0, 0.5]);
        let tight = |x: f64, y: f64| (50.0 * if x <= 10.0 { x } else { 20.0 - x }, y);
        let out = warp_mesh_surface_with(&thin, Rect::new(0, 0, 20, 2), &tight, Fold::IN_PLACE, Interp::Bilinear, &Interrupt::NONE).unwrap();
        // 20 px from the crease: 0.4 source px into each layer, 0.8 apart.
        let a = out.read_region(Rect::new(480, 1, 481, 2))[3];
        assert!((a - 0.75).abs() < 0.02, "layers 0.8 source px apart: alpha {a}");
    }

    /// A mask or selection warped with the layer follows its pixels: where the warp folds, the
    /// part on top is the part the layer shows on top.
    #[test]
    fn a_fold_stacks_a_mask_as_it_stacks_the_layer() {
        let fmt = PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false);
        let mut m = Surface::with_default(fmt, &[0.0]);
        m.fill_rect(Rect::new(0, 0, 100, 10), &[0.25]);
        m.fill_rect(Rect::new(50, 0, 100, 10), &[1.0]);
        // The left half folds over the right.
        let fold = |x: f64, y: f64| (if x >= 50.0 { x } else { 100.0 - x }, y);
        let top = warp_mesh_gray_with(&m, &fold, Fold::IN_PLACE, Interp::Bilinear).read_region(Rect::new(80, 5, 81, 6))[0];
        assert!((top - 0.25).abs() < 1e-3, "the left part, dragged over, is on top: {top}");
        let rows = warp_mesh_gray(&m, &fold, Interp::Bilinear).read_region(Rect::new(80, 5, 81, 6))[0];
        assert!((rows - 1.0).abs() < 1e-3, "other mesh maps: the last cell wins: {rows}");
    }

    /// A vertex with no finite movement (past a Distort's horizon) goes under, not on top.
    #[test]
    fn cells_moved_without_bound_go_first() {
        // Two cells in a row: the second's right vertices moved to infinity.
        let rest = [[0.0, 0.0], [1.0, 0.0], [2.0, 0.0], [0.0, 1.0], [1.0, 1.0], [2.0, 1.0]];
        let mut pos = rest.map(|[x, y]| [x + 1.0, y]);
        pos[2] = [f64::INFINITY, 0.0];
        pos[5] = [f64::NAN, 1.0];
        assert_eq!(cell_order(&pos, &rest, 3), vec![1, 0]);
    }

    /// A cell the warp turns over (mirrored, as a flap folded back) goes on top, even when it moved
    /// less than the cells around it; within each kind, the cell moved least goes first.
    #[test]
    fn a_turned_over_cell_goes_on_top() {
        let rest = [[0.0, 0.0], [1.0, 0.0], [2.0, 0.0], [3.0, 0.0], [0.0, 1.0], [1.0, 1.0], [2.0, 1.0], [3.0, 1.0]];
        // Cell 0 moved far and cell 1 a little, both facing as before; cell 2 is mirrored in x and
        // moved less than cell 0.
        let pos = [[-9.0, 0.0], [1.0, 0.0], [2.2, 0.0], [1.6, 0.0], [-9.0, 1.0], [1.0, 1.0], [2.2, 1.0], [1.6, 1.0]];
        assert_eq!(cell_order(&pos, &rest, 4), vec![1, 0, 2]);
        // A mirrored placement turns nothing over: only a change of facing counts.
        let mirrored = rest.map(|[x, y]| [-x, y]);
        assert_eq!(cell_order(&mirrored, &mirrored, 4), vec![0, 1, 2]);
    }

    /// The middle of the picture pulled out past its right edge: the edge strip folds back over it
    /// and lies on top, so the pulled middle is under that strip (by distance moved alone, the middle
    /// would cover it).
    #[test]
    fn the_middle_pulled_past_an_edge_goes_under_the_folded_edge() {
        let fmt = PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false);
        let mut m = Surface::with_default(fmt, &[0.0]);
        m.fill_rect(Rect::new(0, 0, 75, 10), &[0.25]);
        m.fill_rect(Rect::new(75, 0, 100, 10), &[1.0]);
        // 0..75 stretches to 0..90; the edge strip 75..100 folds back onto 90..80; beyond, unchanged.
        let pull = |x: f64, y: f64| {
            (
                if x <= 75.0 {
                    x * 1.2
                } else if x <= 100.0 {
                    90.0 - (x - 75.0) * 0.4
                } else {
                    x - 20.0
                },
                y,
            )
        };
        let at = warp_mesh_gray_with(&m, &pull, Fold::IN_PLACE, Interp::Bilinear).read_region(Rect::new(85, 5, 86, 6))[0];
        assert!((at - 1.0).abs() < 1e-3, "the folded-back edge strip is on top: {at}");
    }

    use photocraft_color::{ColorMode, SampleType};
    use photocraft_geom::warp::{BezierMesh, Warp, WarpStyle};

    fn sample(s: SampleType) -> Surface {
        let mut surf = Surface::new(PixelFormat::new(ColorMode::Rgb, s, true));
        for y in 0..24 {
            for x in 0..40 {
                let v = ((x * 7 + y * 13) % 17) as f32 / 16.0;
                surf.write_pixel(x + 5, y + 3, &[v, 1.0 - v, (x as f32) / 40.0, 1.0]);
            }
        }
        surf
    }

    #[test]
    fn a_mesh_flung_across_the_coordinate_range_gets_no_per_tile_bookkeeping() {
        // About 8000 × 8000 destination tiles from three vertices.
        let src = sample(SampleType::U8);
        let far = 1_000_000.0;
        let verts = [([-far, -far], [0.0, 0.0]), ([far, -far], [40.0, 0.0]), ([0.0, far], [0.0, 24.0])];
        let t = std::time::Instant::now();
        let out = warp_triangles(&src, Rect::new(0, 0, 40, 24), &verts, &[[0, 1, 2]], Interp::Nearest);
        assert!(out.content_bounds().is_empty());
        assert!(t.elapsed().as_secs() < 5);
    }

    #[test]
    fn a_thin_layer_stretched_into_a_curve_keeps_the_curve() {
        // #5793: a 3 px wide column stretched 100x and bent into an arch. The grid was sized in
        // source pixels (one cell across), so the arch came out as a straight band.
        let mut src = Surface::new(PixelFormat::new(ColorMode::Rgb, SampleType::U8, true));
        src.fill_rect(Rect::new(0, 0, 3, 40), &[1.0, 0.0, 0.0, 1.0]);
        let arch = |x: f64, y: f64| {
            let dx = x * 100.0;
            let t = (dx - 150.0) / 150.0;
            (dx, y + 40.0 * (1.0 - t * t))
        };
        let out = warp_mesh_surface(&src, Rect::new(0, 0, 3, 40), &arch, Interp::Bilinear);
        let alpha = |x: i32, y: i32| {
            let mut p = [0.0f32; 4];
            out.read_pixel(x, y, &mut p);
            p[3]
        };
        // The band follows the arch: 40 px lower in the middle, about 36 px a third of the way in.
        assert!(alpha(150, 60) > 0.99 && alpha(150, 20) < 0.01, "mid {} {}", alpha(150, 60), alpha(150, 20));
        assert!(alpha(100, 55) > 0.99 && alpha(100, 20) < 0.01, "third {} {}", alpha(100, 55), alpha(100, 20));
    }

    #[test]
    fn the_interruptible_mesh_warp_reports_progress_and_stops_when_cancelled() {
        let mut s = Surface::new(PixelFormat::RGBA8);
        s.fill_rect(Rect::new(0, 0, 600, 400), &[0.2, 0.4, 0.6, 1.0]);
        let r = s.content_bounds();
        let w = Warp::preset(WarpStyle::Arc, 30.0, [0.0, 0.0, 600.0, 400.0]);
        let map = |x: f64, y: f64| w.map(x, y);
        let reported = std::sync::Mutex::new(0.0f32);
        let progress = |p: f32| {
            let mut m = reported.lock().unwrap();
            *m = m.max(p);
        };
        let never = || false;
        let out = warp_mesh_surface_with(&s, r, &map, Fold::Rows, Interp::Bicubic, &Interrupt::new(&never, &progress)).expect("not cancelled");
        let want = warp_mesh_surface(&s, r, &map, Interp::Bicubic);
        assert_eq!(out.content_bounds(), want.content_bounds());
        assert_eq!(out.read_region(want.content_bounds()), want.read_region(want.content_bounds()));
        assert!(*reported.lock().unwrap() > 0.9);
        let always = || true;
        assert!(warp_mesh_surface_with(&s, r, &map, Fold::Rows, Interp::Bicubic, &Interrupt::cancel_only(&always)).is_none());
    }

    #[test]
    fn a_mesh_warp_far_off_canvas_writes_only_the_two_regions() {
        // #995: the old and warped regions were cleared as one dense `src ∪ warped` buffer, about
        // 37 GiB of floats for a 48 px selection moved 100000 px. Same fix as `warp_gray` (b70fbd4).
        for sample in [SampleType::U8, SampleType::F32] {
            let mut sel = Surface::new(PixelFormat::new(ColorMode::Grayscale, sample, false));
            sel.fill_rect(Rect::new(8, 8, 56, 56), &[1.0]);
            for d in [100_000.0, -100_000.0, 300.0] {
                let out = warp_mesh_gray(&sel, &|x, y| (x + d, y + d), Interp::Bilinear);
                let k = d as i32;
                assert_eq!(out.content_bounds(), Rect::new(8 + k, 8 + k, 56 + k, 56 + k), "{sample:?} {d}");
                assert_eq!(out.sample_channel(30 + k, 30 + k, 0), 1.0, "{sample:?} {d}: moved");
                assert_eq!(out.sample_channel(30, 30, 0), 0.0, "{sample:?} {d}: old region cleared");
            }
        }
    }

    #[test]
    fn identity_warp_is_a_no_op_at_every_depth() {
        for s in [SampleType::U8, SampleType::U16, SampleType::F32] {
            let src = sample(s);
            let r = src.content_bounds();
            let b = [f64::from(r.x0), f64::from(r.y0), f64::from(r.x1), f64::from(r.y1)];
            for w in [Warp::none(b), Warp::custom(BezierMesh::identity(b, 1, 1), b), Warp::preset(WarpStyle::Arc, 0.0, b)] {
                for interp in [Interp::Nearest, Interp::Bilinear, Interp::Bicubic] {
                    let out = warp_mesh_surface(&src, r, &|x, y| w.map(x, y), interp);
                    assert_eq!(out.content_bounds(), r, "{s:?} {interp:?}");
                    let (a, o) = (src.read_region(r), out.read_region(r));
                    let worst = a.iter().zip(&o).map(|(x, y)| (x - y).abs()).fold(0.0f32, f32::max);
                    assert!(worst <= 1.0 / 255.0, "{s:?} {interp:?}: {worst}");
                }
            }
        }
    }

    #[test]
    fn a_rotated_soft_edged_sprite_keeps_its_colour() {
        // #6203, as in Free Transform: the bicubic alpha overshoots past 1 next to the half
        // transparent edge, and must not lighten the colour.
        let mut src = Surface::new(PixelFormat::new(ColorMode::Rgb, SampleType::U8, true));
        src.fill_rect(Rect::new(10, 10, 30, 30), &[0.8, 0.6, 0.4, 0.5]);
        src.fill_rect(Rect::new(11, 11, 29, 29), &[0.8, 0.6, 0.4, 1.0]);
        let k = std::f64::consts::FRAC_1_SQRT_2;
        let rotate = |x: f64, y: f64| (20.0 + (x - 20.0) * k - (y - 20.0) * k, 20.0 + (x - 20.0) * k + (y - 20.0) * k);
        let out = warp_mesh_surface(&src, src.content_bounds(), &rotate, Interp::Bicubic);
        let b = out.content_bounds();
        for y in b.y0..b.y1 {
            for x in b.x0..b.x1 {
                let p = out.pixel(x, y);
                if p[3] > 0.0 {
                    let off = (0..3).map(|i| (p[i] - [0.8, 0.6, 0.4][i]).abs()).fold(0.0f32, f32::max);
                    assert!(off <= 1.0 / 255.0, "({x}, {y}): {p:?}");
                }
            }
        }
    }

    #[test]
    fn translation_and_known_arc_points() {
        let src = sample(SampleType::U8);
        let r = src.content_bounds();
        let out = warp_mesh_surface(&src, r, &|x, y| (x + 10.0, y - 2.0), Interp::Bilinear);
        assert_eq!(out.content_bounds(), r.translate(10, -2));
        assert_eq!(out.pixel(20, 10), src.pixel(10, 12));
        // Arc 50 % (Photoshop): the bottom corners keep their pixels, the top rises and spreads.
        let b = [f64::from(r.x0), f64::from(r.y0), f64::from(r.x1), f64::from(r.y1)];
        let w = Warp::preset(WarpStyle::Arc, 50.0, b);
        let arc = warp_mesh_surface(&src, r, &|x, y| w.map(x, y), Interp::Bilinear);
        let corner = w.map(f64::from(r.x0), f64::from(r.y1));
        assert!((corner.0 - f64::from(r.x0)).abs() < 1e-9 && (corner.1 - f64::from(r.y1)).abs() < 1e-9, "{corner:?}");
        let (ex, ey) = w.map(f64::from(r.x0) + 2.5, f64::from(r.y0) + 2.5);
        let e = arc.pixel(ex.floor() as i32, ey.floor() as i32);
        assert!(e[3] > 0.2, "a point near the top-left corner lands at its mapped position: {e:?}");
        let c = arc.content_bounds();
        assert!(c.y0 < r.y0 - 2 && c.x0 < r.x0 && c.x1 > r.x1, "the top rises and spreads past the box: {c:?}");
    }

    #[test]
    fn a_shrinking_mesh_warp_is_prefiltered() {
        // A warped smart object placed at ~1/5 size: a grid of 1 px lines every 4 px must come
        // out its area average (9/16 white) everywhere, not point-sampled moiré (crinkled,
        // doubled or missing lines), like the affine path's `large_downscale_is_prefiltered`.
        for s in [SampleType::U8, SampleType::U16, SampleType::F32] {
            let (w, h) = (400, 320);
            let mut src = Surface::new(PixelFormat::new(ColorMode::Rgb, s, true));
            let px: Vec<f32> =
                (0..h).flat_map(|y| (0..w).map(move |x| if x % 4 == 0 || y % 4 == 0 { 0.0 } else { 1.0 })).flat_map(|v| [v, v, v, 1.0]).collect();
            let r = Rect::new(0, 0, w, h);
            src.write_region(r, &px);
            let warp = Warp::preset(WarpStyle::Arc, 15.0, [0.0, 0.0, f64::from(w), f64::from(h)]);
            let t = photocraft_geom::Affine { m: [0.2, 0.0, 0.0, 0.2, 7.0, 5.0] };
            let out = place_source(&src, r, &t, Some(&warp)).unwrap();
            let b = out.content_bounds();
            let o = out.read_region(b);
            let opaque: Vec<f32> = o.as_chunks::<4>().0.iter().filter(|p| p[3] >= 0.999).map(|p| p[0]).collect();
            assert!(opaque.len() > 2000, "{s:?}: {} opaque pixels", opaque.len());
            let worst = opaque.iter().map(|v| (v - 0.5625).abs()).fold(0.0f32, f32::max);
            // An area average over the ~5 px footprint still covers one or two lines per axis
            // (up to 0.2 off); point sampling lands wholly on or off a line (up to 0.56 off).
            assert!(worst < 0.25, "{s:?}: worst deviation from the area average {worst}");
        }
    }

    /// A warp that doesn't fold is drawn in row order whether or not folds stack: the result is
    /// the same to the bit, and only a tile where the picture overlaps itself is drawn again.
    #[test]
    fn a_warp_without_a_fold_is_the_same_either_way() {
        let (w, h) = (300, 200);
        let mut src = Surface::new(PixelFormat::RGBA8);
        let px: Vec<f32> = (0..w * h).flat_map(|i| [(i % 7) as f32 / 6.0, (i % 11) as f32 / 10.0, 0.5, 1.0]).collect();
        let r = Rect::new(0, 0, w, h);
        src.write_region(r, &px);
        let bulge = |x: f64, y: f64| {
            let (u, v) = (x / 300.0 - 0.5, y / 200.0 - 0.5);
            let k = 1.0 - 0.6 * (u * u + v * v);
            (300.0 * (0.5 + u * k) + 10.0, 200.0 * (0.5 + v * k) + 10.0)
        };
        let rows = warp_mesh_surface_with(&src, r, &bulge, Fold::Rows, Interp::Bicubic, &Interrupt::NONE).unwrap();
        let stacked = warp_mesh_surface_with(&src, r, &bulge, Fold::IN_PLACE, Interp::Bicubic, &Interrupt::NONE).unwrap();
        let b = rows.content_bounds();
        assert_eq!(b, stacked.content_bounds());
        assert!(rows.read_region(b) == stacked.read_region(b), "the same pixels");
    }

    /// Points in two parts of the source far apart are two regions, each read on its own; points
    /// close together are one, and a point not given has none.
    #[test]
    fn regions_split_distant_parts_of_the_source() {
        let src = Rect::new(0, 0, 6000, 4000);
        let pts: Vec<([f64; 2], [f64; 2])> =
            (0..64).flat_map(|i| [([100.0 + f64::from(i), 100.0], [0.5, 0.5]), ([5000.0 + f64::from(i), 3500.0], [0.5, 0.5])]).collect();
        let r = Regions::new(&|f| pts.iter().for_each(|&(q, e)| f(q, e)), src);
        assert_eq!(r.rects.len(), 2, "{:?}", r.rects);
        assert!(r.area() < 20_000, "small windows: {:?}", r.rects);
        assert_ne!(r.of([100.0, 100.0]), r.of([5000.0, 3500.0]));
        assert_eq!(r.of([120.0, 100.0]), r.of([100.0, 100.0]));
        assert_eq!(r.of([3000.0, 2000.0]), None, "nothing samples between them");
        // A compact patch stays one region; nothing at all is none.
        let near: Vec<([f64; 2], [f64; 2])> = (0..400).map(|i| ([f64::from(i % 20), f64::from(i / 20)], [0.5, 0.5])).collect();
        let one = Regions::new(&|f| near.iter().for_each(|&(q, e)| f(q, e)), src);
        assert_eq!(one.rects.len(), 1);
        assert_eq!(one.of([5.0, 5.0]), Some(0));
        let none = Regions::new(&|_| {}, src);
        assert!(none.rects.is_empty() && none.of([0.0, 0.0]).is_none());
        let bad = Regions::new(&|f| f([f64::NAN, 1.0], [0.5, 0.5]), src);
        assert!(bad.rects.is_empty());
    }

    #[test]
    fn gray_masks_keep_their_default() {
        let mut m = Surface::with_default(PixelFormat::GRAY8, &[1.0]);
        m.fill_rect(Rect::new(4, 4, 10, 10), &[0.0]);
        let o = warp_mesh_gray(&m, &|x, y| (x + 5.0, y), Interp::Bilinear);
        assert_eq!(o.pixel(0, 0), vec![1.0]);
        assert_eq!(o.pixel(12, 6), vec![0.0]);
        assert_eq!(o.pixel(5, 6), vec![1.0]);
    }

    /// Tiles are drawn on the thread pool, and the first one that meets a fold works out the
    /// fold's order while the others that need it wait. That must never hand the working thread
    /// another folded tile (it would wait on itself, and every tile after it for ever): a warp
    /// that folds some of its tiles, with a mesh big enough to be worth sorting on several
    /// threads, finishes every time.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn a_fold_never_stalls_the_thread_pool() {
        let mut src = Surface::new(PixelFormat::RGBA8);
        src.fill_rect(Rect::new(0, 0, 64, 64), &[1.0, 0.0, 0.0, 1.0]);
        let r = Rect::new(0, 0, 2048, 2048);
        // The left quarter folded over the next: a quarter of the tiles hold two layers.
        let fold = |x: f64, y: f64| (if x < 512.0 { 1024.0 - x } else { x }, y);
        let (tx, rx) = std::sync::mpsc::channel();
        // Pools of its own, so a stall can't take the other tests' pool with it.
        std::thread::spawn(move || {
            for threads in [8, 16] {
                let Ok(pool) = rayon::ThreadPoolBuilder::new().num_threads(threads).build() else { return };
                for _ in 0..12 {
                    pool.install(|| warp_mesh_surface_with(&src, r, &fold, Fold::IN_PLACE, Interp::Nearest, &Interrupt::NONE));
                }
            }
            let _ = tx.send(());
        });
        assert!(rx.recv_timeout(std::time::Duration::from_secs(120)).is_ok(), "the warp never finished");
    }
}
