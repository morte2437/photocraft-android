//! Fill: exact-area scan conversion of the traced shapes, so edges are anti-aliased and shapes
//! that share an edge sum to full coverage.

use super::Pt;

/// Accumulates the signed area of segment `a`–`b` into `acc` (rows of `stride` cells, `h` rows
/// visible, columns `0..stride - 2` visible): the part left of the tile is drawn down its left
/// edge (it still winds the pixels to its right), the part right of it is dropped.
pub(super) fn line_clipped(acc: &mut [f32], stride: usize, h: usize, a: Pt, b: Pt) {
    if !(a[0].is_finite() && a[1].is_finite() && b[0].is_finite() && b[1].is_finite()) || a[1] == b[1] {
        return;
    }
    let wf = (stride - 2) as f32;
    // At most the two ends and two crossings; on the stack, as this runs for every segment drawn.
    let mut buf = [0.0f32, 1.0, 0.0, 0.0];
    let mut n = 2;
    for x in [0.0, wf] {
        let t = (x - a[0]) / (b[0] - a[0]);
        if t > 0.0 && t < 1.0 {
            buf[n] = t;
            n += 1;
        }
    }
    let cuts = &mut buf[..n];
    cuts.sort_by(f32::total_cmp);
    for t in cuts.windows(2) {
        let p = [a[0] + (b[0] - a[0]) * t[0], a[1] + (b[1] - a[1]) * t[0]];
        let q = [a[0] + (b[0] - a[0]) * t[1], a[1] + (b[1] - a[1]) * t[1]];
        let mx = (p[0] + q[0]) / 2.0;
        if mx >= wf {
            continue;
        }
        if mx < 0.0 {
            line(acc, stride, h, [0.0, p[1]], [0.0, q[1]]);
        } else {
            line(acc, stride, h, [p[0].clamp(0.0, wf), p[1]], [q[0].clamp(0.0, wf), q[1]]);
        }
    }
}

/// Signed-area accumulation of one segment inside the tile (the scanline method of font
/// rasterisers): after a running sum along each row, a cell holds the coverage winding.
fn line(acc: &mut [f32], stride: usize, h: usize, p0: Pt, p1: Pt) {
    if p0[1] == p1[1] {
        return;
    }
    let (dir, p0, p1) = if p0[1] < p1[1] { (1.0f32, p0, p1) } else { (-1.0, p1, p0) };
    let dxdy = (p1[0] - p0[0]) / (p1[1] - p0[1]);
    let mut x = p0[0];
    let ystart = p0[1].max(0.0);
    if p0[1] < 0.0 {
        x -= p0[1] * dxdy;
    }
    let yend = p1[1].min(h as f32);
    if ystart >= yend {
        return;
    }
    let max_x = (stride - 1) as f32;
    for y in (ystart as usize)..(yend.ceil() as usize).min(h) {
        let ls = y * stride;
        let dy = ((y + 1) as f32).min(p1[1]) - (y as f32).max(p0[1]);
        let xnext = x + dxdy * dy;
        let d = dy * dir;
        let (x0, x1) = if x < xnext { (x, xnext) } else { (xnext, x) };
        let (x0, x1) = (x0.clamp(0.0, max_x - 1.0), x1.clamp(0.0, max_x - 1.0));
        let x0f = x0.floor();
        let x0i = x0f as usize;
        let x1c = x1.ceil();
        let x1i = x1c as usize;
        if x1i <= x0i + 1 {
            let xmf = 0.5 * (x0 + x1) - x0f;
            acc[ls + x0i] += d - d * xmf;
            acc[ls + x0i + 1] += d * xmf;
        } else {
            let sx = (x1 - x0).recip();
            let x0r = x0 - x0f;
            let a0 = 0.5 * sx * (1.0 - x0r) * (1.0 - x0r);
            let x1r = x1 - x1c + 1.0;
            let am = 0.5 * sx * x1r * x1r;
            acc[ls + x0i] += d * a0;
            if x1i == x0i + 2 {
                acc[ls + x0i + 1] += d * (1.0 - a0 - am);
            } else {
                let a1 = sx * (1.5 - x0r);
                acc[ls + x0i + 1] += d * (a1 - a0);
                for xi in x0i + 2..x1i - 1 {
                    acc[ls + xi] += d * sx;
                }
                let a2 = a1 + (x1i - x0i - 3) as f32 * sx;
                acc[ls + x1i - 1] += d * (1.0 - a2 - am);
            }
            acc[ls + x1i] += d * am;
        }
        x = xnext;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_square_covers_its_area_exactly() {
        let mut acc = vec![0.0f32; 12 * 10];
        let sq = [[2.25, 2.5], [7.75, 2.5], [7.75, 6.5], [2.25, 6.5], [2.25, 2.5]];
        for s in sq.windows(2) {
            line_clipped(&mut acc, 12, 10, s[0], s[1]);
        }
        let mut total = 0.0;
        for y in 0..10 {
            let mut sum = 0.0;
            for x in 0..10 {
                sum += acc[y * 12 + x];
                total += sum.abs().min(1.0);
            }
        }
        assert!((total - 5.5 * 4.0).abs() < 1e-4, "{total}");
    }
}
