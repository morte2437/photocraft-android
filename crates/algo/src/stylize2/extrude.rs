//! Rasterize each swept block only where it can hit, in the original cell order.

use super::{ExtrudeSpec, extrude_reach, read_px};
use crate::fxutil::{MAXC, luma, ncol, subtractive, unpremul_px};
use crate::image::Image;
use crate::noise::hash01;
use crate::{Ctx, ExtrudeType};
use photocraft_geom::Rect;

const MAX_PIXELS: usize = 4_194_304;
const MAX_CELLS: i64 = 262_144;
// Beyond this, f32 pixel centres can lose enough precision to widen the sweep bounds.
const MAX_COORD: i32 = 1 << 22;

pub(super) fn blocks(src: &Image, out: Rect, ctx: &Ctx, spec: &ExtrudeSpec) -> Option<Vec<f32>> {
    if spec.kind != ExtrudeType::Blocks || src.ch == 0 || src.ch > MAXC || !spec.size.is_finite() || !spec.depth.is_finite() {
        return None;
    }
    let b = ctx.bounds;
    if [b.x0, b.y0, b.x1, b.y1, out.x0, out.y0, out.x1, out.y1].iter().any(|v| v.unsigned_abs() > MAX_COORD as u32) {
        return None;
    }
    let w = out.width() as usize;
    let count = w.checked_mul(out.height() as usize)?;
    if count > MAX_PIXELS {
        return None;
    }
    let mut heights = Vec::new();
    heights.try_reserve_exact(count).ok()?;
    heights.resize(count, f32::NEG_INFINITY);
    let mut res = src.crop(out);
    let area = out.intersect(&b);
    if area.is_empty() {
        return Some(res);
    }
    let cs = spec.size.clamp(2.0, 255.0).round() as i32;
    let reach = extrude_reach(spec.depth);
    let k = (reach / cs as f32).ceil() as i32 + 1;
    let (gx0, gy0) = ((area.x0 - b.x0).div_euclid(cs), (area.y0 - b.y0).div_euclid(cs));
    let (gx1, gy1) = ((area.x1 - 1 - b.x0).div_euclid(cs), (area.y1 - 1 - b.y0).div_euclid(cs));
    let (cx0, cy0) = ((gx0 - k).max(0), (gy0 - k).max(0));
    let (cx1, cy1) = ((gx1 + k).min((b.width() as i32 - 1) / cs), (gy1 + k).min((b.height() as i32 - 1) / cs));
    if i64::from(cx1 - cx0 + 1).checked_mul(i64::from(cy1 - cy0 + 1))? > MAX_CELLS {
        return None;
    }
    let pass = BlockPass {
        src,
        out,
        ctx,
        spec,
        b,
        area,
        cs,
        k,
        reach,
        centre: ((b.x0 + b.x1) as f32 / 2.0, (b.y0 + b.y1) as f32 / 2.0),
        half_diag: ((b.width() as f32).hypot(b.height() as f32) / 2.0).max(1.0),
        half: cs as f32 / 2.0,
    };
    // Ascending gy, then gx, is the tie order of every pixel's old candidate search.
    for cy in cy0..=cy1 {
        for cx in cx0..=cx1 {
            pass.paint_cell(cx, cy, &mut heights, &mut res)?;
        }
    }
    Some(res)
}

/// The constants shared by every swept block of one `blocks` call.
struct BlockPass<'a> {
    src: &'a Image,
    out: Rect,
    ctx: &'a Ctx,
    spec: &'a ExtrudeSpec,
    b: Rect,
    area: Rect,
    cs: i32,
    k: i32,
    reach: f32,
    centre: (f32, f32),
    half_diag: f32,
    half: f32,
}

/// One cell's flat colour and where its extruded block sweeps to.
struct Block {
    r: Rect,
    cx: i32,
    m: (f32, f32),
    o: (f32, f32),
    off: f32,
    h: f32,
    col: [f32; MAXC],
}

impl BlockPass<'_> {
    /// Average the (premultiplied) source over `clipped`, sampling every `step` pixels.
    fn cell_colour(&self, clipped: Rect) -> [f32; MAXC] {
        let (src, ctx, n) = (self.src, self.ctx, self.src.ch);
        let mut col = [0.0f32; MAXC];
        let mut cnt = 0.0;
        let step = (self.cs / 8).max(1) as usize;
        for yy in (clipped.y0..clipped.y1).step_by(step) {
            for xx in (clipped.x0..clipped.x1).step_by(step) {
                let a = if ctx.alpha { src.get(xx, yy, n - 1) } else { 1.0 };
                for (c, v) in col.iter_mut().enumerate().take(n) {
                    let s = src.get(xx, yy, c);
                    *v += if ctx.alpha && c < n - 1 { s * a } else { s };
                }
                cnt += 1.0;
            }
        }
        if cnt > 0.0 {
            for v in col.iter_mut().take(n) {
                *v /= cnt;
            }
        }
        unpremul_px(&mut col[..n], ctx.alpha);
        col
    }

    /// Rasterize the block of grid cell (`cx`, `cy`) over the pixels it can hit.
    fn paint_cell(&self, cx: i32, cy: i32, heights: &mut [f32], res: &mut [f32]) -> Option<()> {
        let (b, cs, half) = (self.b, self.cs, self.half);
        let r = Rect::new(b.x0 + cx * cs, b.y0 + cy * cs, b.x0 + (cx + 1) * cs, b.y0 + (cy + 1) * cs);
        let clipped = r.intersect(&b);
        if self.spec.mask_incomplete && clipped != r {
            return Some(());
        }
        let col = self.cell_colour(clipped);
        let n = self.src.ch;
        let h = if self.spec.level_based { luma(self.ctx, &col[..n]).clamp(0.0, 1.0) } else { hash01(cx, cy, 21, self.spec.seed) };
        if !h.is_finite() {
            return None;
        }
        let (mx, my) = (r.x0 as f32 + half, r.y0 as f32 + half);
        let off = self.reach * h / self.half_diag;
        let o = ((mx - self.centre.0) * off, (my - self.centre.1) * off);
        // Two extra pixels conservatively cover rounding in the unchanged interval test.
        let sweep = Rect::new(
            (mx.min(mx + o.0) - half).floor() as i32 - 2,
            (my.min(my + o.1) - half).floor() as i32 - 2,
            (mx.max(mx + o.0) + half).ceil() as i32 + 2,
            (my.max(my + o.1) + half).ceil() as i32 + 2,
        )
        .intersect(&self.area);
        let block = Block { r, cx, m: (mx, my), o, off, h, col };
        for y in sweep.y0..sweep.y1 {
            let gy = (y - b.y0).div_euclid(cs);
            if (cy - gy).abs() > self.k {
                continue;
            }
            for x in sweep.x0..sweep.x1 {
                self.paint_pixel(&block, x, y, heights, res)?;
            }
        }
        Some(())
    }

    /// Paint pixel (`x`, `y`) if `block` covers it and is taller than what is there.
    fn paint_pixel(&self, block: &Block, x: i32, y: i32, heights: &mut [f32], res: &mut [f32]) -> Option<()> {
        let (b, cs, half, n) = (self.b, self.cs, self.half, self.src.ch);
        let (mx, my) = block.m;
        let (ox, oy) = block.o;
        if (block.cx - (x - b.x0).div_euclid(cs)).abs() > self.k {
            return Some(());
        }
        let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
        let i = (y - self.out.y0) as usize * self.out.width() as usize + (x - self.out.x0) as usize;
        let height = heights.get_mut(i)?;
        if *height >= block.h {
            return Some(());
        }
        let Some(iy) = interval(fy, my, oy, half) else { return Some(()) };
        let Some(ix) = interval(fx, mx, ox, half) else { return Some(()) };
        let (lo, hi) = (ix.0.max(iy.0), ix.1.min(iy.1));
        if lo > hi {
            return Some(());
        }
        *height = block.h;
        let face = block_face(hi, ix.1 < iy.1, ox, oy);
        let px = res.get_mut(i * n..(i + 1) * n)?;
        self.write_face(block, face, (fx, fy), px)
    }

    /// Write `face` of `block` into pixel `px` centred at `(fx, fy)`: the front
    /// face shows the shifted source, every face can instead be flat-shaded.
    fn write_face(&self, block: &Block, face: u8, (fx, fy): (f32, f32), px: &mut [f32]) -> Option<()> {
        let n = self.src.ch;
        if face == 0 && !self.spec.solid_front {
            let (sx, sy) = ((fx - (block.m.0 - self.centre.0) * block.off).floor() as i32, (fy - (block.m.1 - self.centre.1) * block.off).floor() as i32);
            read_px(self.src, sx.clamp(block.r.x0, block.r.x1 - 1), sy.clamp(block.r.y0, block.r.y1 - 1), px);
        } else {
            let shade = face_shade(face);
            let sub = subtractive(self.ctx);
            for (v, c) in px.iter_mut().zip(&block.col).take(ncol(self.ctx, n)) {
                *v = if sub { 1.0 - (1.0 - *c) * shade } else { *c * shade }.clamp(0.0, 1.0);
            }
            if self.ctx.alpha {
                *px.last_mut()? = block.col.get(n - 1).copied()?;
            }
        }
        Some(())
    }
}

/// Which face of the swept block a pixel shows: 0 front, 1/2 right/left, 3/4 bottom/top.
fn block_face(hi: f32, x_exits_first: bool, ox: f32, oy: f32) -> u8 {
    if hi >= 1.0 {
        0
    } else if x_exits_first {
        if ox > 0.0 { 1 } else { 2 }
    } else if oy > 0.0 {
        3
    } else {
        4
    }
}

fn face_shade(face: u8) -> f32 {
    match face {
        0 => 1.0,
        1 => 0.85,
        2 => 0.55,
        3 => 0.95,
        _ => 0.45,
    }
}

fn interval(p: f32, m: f32, o: f32, half: f32) -> Option<(f32, f32)> {
    if o.abs() < 1e-6 {
        return ((p - m).abs() <= half).then_some((0.0, 1.0));
    }
    let (a, b) = ((p - m - half) / o, (p - m + half) / o);
    let (lo, hi) = (a.min(b).max(0.0), a.max(b).min(1.0));
    (lo <= hi).then_some((lo, hi))
}
