use super::common::{Bounds, Level, PAR_MIN, Px};
use super::{Cow, par};

/// Half-size level: a cell is a hole if any of its pixels is; else it averages its non-void
/// pixels, and may be copied from only if all of them may.
pub(super) fn downsample(l: &Level, ch: usize) -> Level<'static> {
    let (w, h) = (l.w.div_ceil(2), l.h.div_ceil(2));
    let mut img = vec![0.0f32; w * h * ch];
    let mut px = vec![Px::Void; w * h];
    let (lw, lh, limg, lpx) = (l.w, l.h, &*l.img, &*l.px);
    let row = |y: usize, irow: &mut [f32], prow: &mut [Px]| {
        for (x, p) in prow.iter_mut().enumerate() {
            let mut n = 0.0f32;
            let o = x * ch;
            let (mut hole, mut all_source, mut any) = (false, true, false);
            for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                let (sx, sy) = (x * 2 + dx, y * 2 + dy);
                if sx >= lw || sy >= lh {
                    continue;
                }
                let si = sy * lw + sx;
                match lpx[si] {
                    Px::Hole => hole = true,
                    Px::Void => all_source = false,
                    k => {
                        any = true;
                        all_source &= k == Px::Source;
                        for c in 0..ch {
                            irow[o + c] += limg[si * ch + c];
                        }
                        n += 1.0;
                    }
                }
            }
            if n > 0.0 {
                for c in 0..ch {
                    irow[o + c] /= n;
                }
            }
            *p = if hole {
                Px::Hole
            } else if !any {
                Px::Void
            } else if all_source {
                Px::Source
            } else {
                Px::Known
            };
        }
    };
    par::chunks2_mut(&mut img, w * ch, &mut px, w, w * h >= PAR_MIN, row);
    Level { w, h, img: Cow::Owned(img), px: Cow::Owned(px) }
}

/// Summed-area table of a boolean grid (`(w+1) × (h+1)`). Not inlined: in [`synthesize`] it made
/// Create Texture's candidate loop slower (measured).
#[inline(never)]
pub(super) fn integral(w: usize, h: usize, m: &[bool]) -> Vec<u32> {
    let mut s = vec![0u32; (w + 1) * (h + 1)];
    for y in 0..h {
        let mut row = 0u32;
        for x in 0..w {
            row += u32::from(m[y * w + x]);
            s[(y + 1) * (w + 1) + x + 1] = s[y * (w + 1) + x + 1] + row;
        }
    }
    s
}

/// Count of set cells in the clipped window `[x0, x1) × [y0, y1)`.
pub(super) fn window_count(s: &[u32], gw: usize, gh: usize, x0: i32, y0: i32, x1: i32, y1: i32) -> u32 {
    let (x0, y0) = (x0.max(0) as usize, y0.max(0) as usize);
    let (x1, y1) = ((x1.max(0) as usize).min(gw), (y1.max(0) as usize).min(gh));
    if x1 <= x0 || y1 <= y0 {
        return 0;
    }
    let w1 = gw + 1;
    s[y1 * w1 + x1] + s[y0 * w1 + x0] - s[y0 * w1 + x1] - s[y1 * w1 + x0]
}

pub(super) fn hole_bbox(w: usize, h: usize, hole: &[bool]) -> Option<(usize, usize, usize, usize)> {
    let mut b: Option<(usize, usize, usize, usize)> = None;
    for y in 0..h {
        for x in 0..w {
            if hole[y * w + x] {
                b = Some(match b {
                    None => (x, y, x + 1, y + 1),
                    Some((x0, y0, x1, y1)) => (x0.min(x), y0.min(y), x1.max(x + 1), y1.max(y + 1)),
                });
            }
        }
    }
    b
}

/// A hole at most `1 / THIN_RATIO` as thick as its bounds are long counts as thin (a wire, a
/// string, a hair, a long brush stroke): see the module docs.
pub(super) const THIN_RATIO: usize = 4;

/// How thick the hole within bounds `hb` of a `w`-wide role grid is: twice the largest distance
/// from a hole pixel to the nearest pixel that is not one (a long, thin hole is thin however long
/// its bounds).
pub(super) fn hole_thickness(w: usize, hb: Bounds, px: &[Px]) -> usize {
    // The bounds and a ring of non-hole cells around them (also past the grid's edge).
    let (gw, gh) = (hb.width() + 2, hb.height() + 2);
    let outside: Vec<bool> = par::map(gw * gh, gw * gh >= PAR_MIN, |i| {
        let (gx, gy) = (i % gw, i / gw);
        if gx == 0 || gy == 0 || gx > hb.width() || gy > hb.height() {
            return true;
        }
        px.get((hb.y0 + gy - 1) * w + hb.x0 + gx - 1) != Some(&Px::Hole)
    });
    let deepest = crate::selection::edt(&outside, gw, gh).into_iter().fold(0.0f32, f32::max);
    (2.0 * deepest).ceil() as usize
}

/// Most target pixels × sources the onion peel of a thin hole may compare (its search is
/// exhaustive, though most comparisons stop early: about a second at this many).
const PEEL_BUDGET: usize = 1 << 28;

/// Whether the onion peel of the hole cells within `hb` of a `w`-wide role grid from `sources`
/// source centres stays within [`PEEL_BUDGET`].
pub(super) fn peel_affordable(w: usize, px: &[Px], hb: Bounds, sources: usize) -> bool {
    let holes: usize = (hb.y0..hb.y1).map(|y| px.get(y * w + hb.x0..y * w + hb.x1).map_or(0, |row| row.iter().filter(|k| **k == Px::Hole).count())).sum();
    holes.saturating_mul(sources) <= PEEL_BUDGET
}

/// Bounds of the hole pixels of a role grid.
pub(super) fn hole_bounds(w: usize, h: usize, px: &[Px]) -> Option<Bounds> {
    // A zero-width grid holds no pixels whatever its height; don't size a row table by it.
    if w == 0 || h == 0 {
        return None;
    }
    // Per row the first and last hole column; the union of the rows' spans.
    let rows: Vec<Option<(usize, usize)>> = par::map(h, w * h >= PAR_MIN, |y| {
        let row = px.get(y * w..(y + 1) * w)?;
        let x0 = row.iter().position(|k| *k == Px::Hole)?;
        let x1 = row.iter().rposition(|k| *k == Px::Hole).unwrap_or(x0) + 1;
        Some((x0, x1))
    });
    let mut b: Option<Bounds> = None;
    for (y, span) in rows.into_iter().enumerate() {
        if let Some((x0, x1)) = span {
            b = Some(match b {
                None => Bounds { x0, y0: y, x1, y1: y + 1 },
                Some(b) => Bounds { x0: b.x0.min(x0), y0: b.y0, x1: b.x1.max(x1), y1: y + 1 },
            });
        }
    }
    b
}

/// Valid source centres: the whole `2r+1` patch inside the grid and copyable. Found per row
/// (each pixel's horizontal run), then per column of runs. `None` when cancelled.
pub(super) fn valid_sources(w: usize, h: usize, r: usize, px: &[Px], ctl: &photocraft_raster::Interrupt) -> Option<Vec<bool>> {
    const BAND: usize = 64;
    let side = 2 * r + 1;
    let par = w * h >= PAR_MIN;
    let mut run = vec![false; w * h];
    if w < side || h < side {
        return Some(run);
    }
    // Per row: is the run of `side` pixels centred on each one all copyable (a sliding count)?
    par::chunks_mut(&mut run, w, par, |y, out| {
        if y % BAND == 0 && ctl.cancelled() {
            return;
        }
        let row = &px[y * w..(y + 1) * w];
        let copyable = |x: usize| usize::from(row[x] == Px::Source);
        let mut count: usize = (0..side).map(copyable).sum();
        out[r] = count == side;
        for (x, o) in out.iter_mut().enumerate().take(w - r).skip(r + 1) {
            count = count + copyable(x + r) - copyable(x - r - 1);
            *o = count == side;
        }
    });
    if ctl.cancelled() {
        return None;
    }
    // Per column, in bands of rows: how many such runs end each row in a row; a centre is valid
    // where `side` of them stack up to `r` rows below it.
    let mut valid = vec![false; w * h];
    par::chunks_mut(&mut valid, BAND * w, par, |band, out| {
        if ctl.cancelled() {
            return;
        }
        let (y0, rows) = (band * BAND, out.len() / w);
        let mut stack = vec![0usize; w];
        for yy in y0.saturating_sub(r)..(y0 + rows + r).min(h) {
            for (x, n) in stack.iter_mut().enumerate() {
                *n = if run[yy * w + x] { *n + 1 } else { 0 };
            }
            // `yy` is the bottom row of centre `y`'s patch.
            if yy < y0 + r || yy < 2 * r {
                continue;
            }
            let y = yy - r;
            for (v, n) in out[(y - y0) * w..(y - y0 + 1) * w].iter_mut().zip(&stack).take(w - r).skip(r) {
                *v = *n >= side;
            }
        }
    });
    if ctl.cancelled() {
        return None;
    }
    Some(valid)
}

/// Columns per block of [`Sources`]' index.
const SOURCE_BLOCK: usize = 64;

/// The valid source centres in row-major order, indexed by counts per row and per block of
/// [`SOURCE_BLOCK`] columns (so the `k`-th is found without listing them all).
pub(super) struct Sources {
    /// Valid centres before each row (`h + 1` entries).
    row_start: Vec<usize>,
    /// Per row, the valid centres in it before each block.
    block_start: Vec<u32>,
    blocks: usize,
}

impl Sources {
    pub(super) fn new(valid: &[bool], w: usize) -> Sources {
        let h = valid.len().checked_div(w).unwrap_or(0);
        let blocks = w.div_ceil(SOURCE_BLOCK);
        let mut block_start = vec![0u32; h * blocks];
        // Per row: the block prefix, and the row's count.
        let counts: Vec<usize> = par::map(h, valid.len() >= PAR_MIN, |y| valid[y * w..(y + 1) * w].iter().filter(|v| **v).count());
        par::chunks_mut(&mut block_start, blocks, valid.len() >= PAR_MIN, |y, out| {
            let mut n = 0u32;
            for (b, o) in out.iter_mut().enumerate() {
                *o = n;
                n += valid[y * w + b * SOURCE_BLOCK..y * w + ((b + 1) * SOURCE_BLOCK).min(w)].iter().filter(|v| **v).count() as u32;
            }
        });
        let mut row_start = Vec::with_capacity(counts.len() + 1);
        let mut total = 0usize;
        row_start.push(0);
        for c in counts {
            total += c;
            row_start.push(total);
        }
        Sources { row_start, block_start, blocks }
    }

    pub(super) fn len(&self) -> usize {
        self.row_start.last().copied().unwrap_or(0)
    }

    /// The `k`-th valid centre in row-major order (`k < len`).
    pub(super) fn nth(&self, k: usize, valid: &[bool], w: usize) -> (i32, i32) {
        let y = self.row_start.partition_point(|&s| s <= k).saturating_sub(1);
        let left = k.saturating_sub(self.row_start.get(y).copied().unwrap_or(0));
        let row_blocks = self.block_start.get(y * self.blocks..(y + 1) * self.blocks).unwrap_or(&[]);
        let b = row_blocks.partition_point(|&s| s as usize <= left).saturating_sub(1);
        let mut left = left.saturating_sub(row_blocks.get(b).copied().unwrap_or(0) as usize);
        let x0 = b * SOURCE_BLOCK;
        for (x, v) in valid.iter().skip(y * w + x0).take(w.saturating_sub(x0)).enumerate() {
            if *v {
                if left == 0 {
                    return ((x0 + x) as i32, y as i32);
                }
                left -= 1;
            }
        }
        (0, 0)
    }

    /// All of them, in row-major order.
    pub(super) fn all(&self, valid: &[bool], w: usize) -> Vec<(i32, i32)> {
        let mut out = Vec::with_capacity(self.len());
        for (i, v) in valid.iter().enumerate() {
            if *v {
                out.push(((i % w) as i32, (i / w) as i32));
            }
        }
        out
    }
}

/// Per cell of `tb` (the hole's bounds `hb` grown by the patch radius, where all of them lie): is
/// it a target centre (its patch overlaps the hole)?
pub(super) fn targets_around(w: usize, r: usize, px: &[Px], hb: Bounds, tb: Bounds) -> Vec<bool> {
    // Hole counts summed over the hole's bounds: every hole pixel lies inside them.
    let (bw, bh) = (hb.width(), hb.height());
    let mut s = vec![0u32; (bw + 1) * (bh + 1)];
    for y in 0..bh {
        let mut row = 0u32;
        for x in 0..bw {
            row += u32::from(px[(hb.y0 + y) * w + hb.x0 + x] == Px::Hole);
            s[(y + 1) * (bw + 1) + x + 1] = s[y * (bw + 1) + x + 1] + row;
        }
    }
    let mut targets = vec![false; tb.area()];
    let ri = r as i32;
    let (ox, oy) = (hb.x0 as i32, hb.y0 as i32);
    for y in tb.y0..tb.y1 {
        for x in tb.x0..tb.x1 {
            let k = (y - tb.y0) * tb.width() + (x - tb.x0);
            let (x, y) = (x as i32, y as i32);
            targets[k] = window_count(&s, bw, bh, x - ri - ox, y - ri - oy, x + ri + 1 - ox, y + ri + 1 - oy) > 0;
        }
    }
    targets
}

/// A finished coarser level: its size, pixels and match field (the source centre per cell of
/// its target box, `i32::MIN` where there is none; empty when it had nothing to copy).
pub(super) struct Coarser {
    pub(super) w: usize,
    pub(super) h: usize,
    pub(super) img: Vec<f32>,
    pub(super) field: Vec<(i32, i32)>,
    pub(super) tbox: Bounds,
}

impl Coarser {
    /// The match of cell `(x, y)` (`i32::MIN` outside the target box).
    pub(super) fn field_at(&self, x: usize, y: usize) -> (i32, i32) {
        let t = self.tbox;
        if x < t.x0 || x >= t.x1 || y < t.y0 || y >= t.y1 {
            return (i32::MIN, i32::MIN);
        }
        self.field.get((y - t.y0) * t.width() + (x - t.x0)).copied().unwrap_or((i32::MIN, i32::MIN))
    }
}
