use super::common::{Bounds, Px, Rng};
use super::solver::{Best, LANES, known_sse, known_sse_any, known_sse_lanes, least_sum};
use super::sources::{hole_bbox, integral, window_count};
use super::*;

/// Fill the hole from its edge inwards, one ring at a time: each pixel takes the centre of the
/// source patch that best matches what is already known around it (the first in `sources` of
/// those that match best). `hb` bounds the hole.
///
/// The search is exhaustive, but most sources are given up early (#2771, #2805): the target's
/// known pixels are gathered once, the search starts from where the pixel's filled neighbours
/// took their sources (or every 16th source at the hole's edge), and a source's error stops being
/// added up once it can only lose (it only grows). So it finds the same source as a plain scan in
/// order, sooner.
#[allow(clippy::too_many_arguments)]
pub(super) fn onion_peel(
    w: usize,
    h: usize,
    ch: usize,
    r: i32,
    img: &mut [f32],
    px: &[Px],
    hb: Bounds,
    sources: &[(i32, i32)],
    ctl: &photocraft_raster::Interrupt,
) -> Result<(), photocraft_raster::Cancelled> {
    const NONE: u32 = u32::MAX;
    let mut known: Vec<bool> = px.iter().map(|k| matches!(k, Px::Known | Px::Source)).collect();
    // Each source centre's place in `sources`, and the source each filled pixel took.
    let mut index = vec![NONE; w * h];
    for (j, &(x, y)) in sources.iter().enumerate() {
        index[y as usize * w + x as usize] = j as u32;
    }
    let mut took = vec![NONE; w * h];
    let neighbours = [(-1, -1), (0, -1), (1, -1), (-1, 0), (1, 0), (-1, 1), (0, 1), (1, 1)];
    loop {
        ctl.check()?;
        let mut front: Vec<usize> = Vec::new();
        for y in hb.y0..hb.y1 {
            for x in hb.x0..hb.x1 {
                let i = y * w + x;
                if known[i] || px[i] != Px::Hole {
                    continue;
                }
                let (x, y) = (x as i32, y as i32);
                let edge = (-1..=1).any(|dy: i32| {
                    (-1..=1).any(|dx: i32| {
                        let (nx, ny) = (x + dx, y + dy);
                        nx >= 0 && ny >= 0 && (nx as usize) < w && (ny as usize) < h && known[ny as usize * w + nx as usize]
                    })
                });
                if edge {
                    front.push(i);
                }
            }
        }
        if front.is_empty() {
            break;
        }
        let (known_now, took_now) = (&known, &took);
        let image = &*img;
        let best = |&i: &usize| -> (Vec<f32>, u32) {
            let (x, y) = ((i % w) as i32, (i / w) as i32);
            // The known pixels of the target patch, in the plain scan's order: each one's offset
            // from the patch centre (in pixels) and its samples (gathered once, #2805).
            let side = (2 * r + 1) as usize;
            let mut offs: Vec<isize> = Vec::with_capacity(side * side);
            let mut tv: Vec<f32> = Vec::with_capacity(side * side * ch);
            for dy in -r..=r {
                for dx in -r..=r {
                    let (tx, ty) = (x + dx, y + dy);
                    if tx >= 0 && ty >= 0 && (tx as usize) < w && (ty as usize) < h && known_now[ty as usize * w + tx as usize] {
                        let ti = (ty as usize * w + tx as usize) * ch;
                        offs.push(dy as isize * w as isize + dx as isize);
                        tv.extend_from_slice(&image[ti..ti + ch]);
                    }
                }
            }
            // The best so far: its error per sample, its place in `sources` (the first of equals
            // wins, as in a scan in order), and the sums of squares at which a source surely loses
            // to it (`e / n` above the best) or ties it (at least the best).
            let mut best = Best { v: f32::INFINITY, j: usize::MAX, worse: f32::INFINITY, equal: f32::INFINITY };
            if !offs.is_empty() {
                let nf = offs.len() as f32;
                let base_of = |j: usize| sources.get(j).map_or(0, |&(sx, sy)| sy as isize * w as isize + sx as isize);
                // The sum at which source `j` can't win: a tie loses to an earlier source.
                let limit_of = |j: usize, best: &Best| if j > best.j { best.equal } else { best.worse };
                // Source `j`'s whole sum `e` (it did not reach its limit): does it win?
                let adopt = |j: usize, e: f32, best: &mut Best| {
                    let v = e / nf;
                    if v < f32::INFINITY && (v < best.v || (v == best.v && j < best.j)) {
                        *best = Best { v, j, worse: least_sum(v, nf, true), equal: least_sum(v, nf, false) };
                    }
                };
                let consider = |j: usize, best: &mut Best| {
                    if j >= sources.len() {
                        return;
                    }
                    let (base, limit) = (base_of(j), limit_of(j, best));
                    let e = match ch {
                        1 => known_sse::<1>(image, &offs, &tv, base, limit),
                        3 => known_sse::<3>(image, &offs, &tv, base, limit),
                        4 => known_sse::<4>(image, &offs, &tv, base, limit),
                        _ => known_sse_any(image, ch, &offs, &tv, base, limit),
                    };
                    if let Some(e) = e {
                        adopt(j, e, best);
                    }
                };
                // A close bound early: where the filled neighbours' sources point (shifted as in
                // PatchMatch's propagation), or at the hole's edge, where none is filled yet,
                // every 16th source.
                for (dx, dy) in neighbours {
                    let (qx, qy) = (x + dx, y + dy);
                    if qx < 0 || qy < 0 || qx as usize >= w || qy as usize >= h {
                        continue;
                    }
                    let Some(&(sx, sy)) = sources.get(took_now[qy as usize * w + qx as usize] as usize) else { continue };
                    let (cx, cy) = (sx - dx, sy - dy);
                    if cx >= 0 && cy >= 0 && (cx as usize) < w && (cy as usize) < h && index[cy as usize * w + cx as usize] != NONE {
                        consider(index[cy as usize * w + cx as usize] as usize, &mut best);
                    }
                }
                if best.j == usize::MAX {
                    for j in (0..sources.len()).step_by(16) {
                        consider(j, &mut best);
                    }
                }
                // The scan in order, `LANES` sources at a time (their sums are independent, so they
                // overlap), each against its limit from the best before them: the best only gets
                // better, so a source given up would have lost to it anyway.
                let mut j = 0;
                while j + LANES <= sources.len() {
                    if j.is_multiple_of(4096) && j > 0 && ctl.cancelled() {
                        break;
                    }
                    let bases: [isize; LANES] = std::array::from_fn(|l| base_of(j + l));
                    let limits: [f32; LANES] = std::array::from_fn(|l| limit_of(j + l, &best));
                    let sums = match ch {
                        1 => known_sse_lanes::<1>(image, &offs, &tv, bases, limits),
                        3 => known_sse_lanes::<3>(image, &offs, &tv, bases, limits),
                        4 => known_sse_lanes::<4>(image, &offs, &tv, bases, limits),
                        _ => std::array::from_fn(|l| known_sse_any(image, ch, &offs, &tv, bases[l], limits[l])),
                    };
                    for (l, e) in sums.into_iter().enumerate() {
                        if let Some(e) = e {
                            adopt(j + l, e, &mut best);
                        }
                    }
                    j += LANES;
                }
                for j in j..sources.len() {
                    consider(j, &mut best);
                }
            }
            let (sx, sy) = sources.get(best.j).copied().unwrap_or((0, 0));
            let si = (sy as usize * w + sx as usize) * ch;
            (image[si..si + ch].to_vec(), if best.j == usize::MAX { NONE } else { best.j as u32 })
        };
        let vals: Vec<(Vec<f32>, u32)> = par::map(front.len(), front.len() * sources.len() >= 4096, |k| best(&front[k]));
        ctl.check()?;
        for (&i, (v, j)) in front.iter().zip(vals) {
            img[i * ch..(i + 1) * ch].copy_from_slice(&v);
            known[i] = true;
            took[i] = j;
        }
    }
    Ok(())
}

/// Proximity match: the displacement `(dx, dy)` (within `max_radius`) whose surroundings best match the
/// ring of `ring` pixels around the hole, with the displaced hole fully inside the grid and outside the
/// hole. `None` if no displacement fits.
pub fn best_offset(w: usize, h: usize, ch: usize, img: &[f32], hole: &[bool], ring: usize, max_radius: i32) -> Option<(i32, i32)> {
    let (bx0, by0, bx1, by1) = hole_bbox(w, h, hole)?;
    // No valid translation can move farther than the source image's extent.
    // Bound arbitrary public API inputs before constructing i32 search ranges:
    // usize -> i32 truncation and signed increments otherwise overflow or hang.
    let extent = w.max(h).min((i32::MAX / 4) as usize) as i32;
    let max_radius = max_radius.max(0).min(extent);
    if max_radius == 0 {
        return None;
    }
    let ring = ring.max(1).min(extent as usize) as i32;
    // Every set cell lies in the hole bounds. Counts outside that rectangle are zero,
    // so source rejection needs a table for the hole, not for the whole photo.
    let hs = HoleIntegral::new(w, hole, (bx0, by0, bx1, by1))?;
    // Sample points: the hole's ring (dilated minus hole), subsampled for speed.
    let mut pts = Vec::new();
    let (x0, y0) = (bx0 as i32 - ring, by0 as i32 - ring);
    let (x1, y1) = (bx1 as i32 + ring, by1 as i32 + ring);
    for y in y0.max(0)..y1.min(h as i32) {
        for x in x0.max(0)..x1.min(w as i32) {
            if hole[y as usize * w + x as usize] {
                continue;
            }
            if hs.count(x - ring, y - ring, x + ring + 1, y + ring + 1) > 0 {
                pts.push((x, y));
            }
        }
    }
    let stride = (pts.len() / 2000).max(1);
    let pts: Vec<(i32, i32)> = pts.into_iter().step_by(stride).collect();
    let eval = |dx: i32, dy: i32, cutoff: f32| -> Option<f32> {
        let (hx0, hy0, hx1, hy1) = (x0 + dx, y0 + dy, x1 + dx, y1 + dy);
        let inside = hx0 >= 0 && hy0 >= 0 && hx1 <= w as i32 && hy1 <= h as i32;
        if (dx, dy) == (0, 0) || !inside || hs.count(hx0, hy0, hx1, hy1) != 0 {
            return None;
        }
        let mut e = 0.0f32;
        let n = pts.len().max(1) as f32;
        let penalty = 1.0 + 1e-3 * ((dx as f32).hypot(dy as f32) / max_radius as f32);
        for (i, &(x, y)) in pts.iter().enumerate() {
            let (a, b) = ((y as usize * w + x as usize) * ch, ((y + dy) as usize * w + (x + dx) as usize) * ch);
            for c in 0..ch {
                let d = img[a + c] - img[b + c];
                e += d * d;
            }
            // SSD terms are nonnegative. The partial score is a lower bound on the
            // final score; use the same division and penalty to keep float rounding
            // and strict tie ordering unchanged. Check in batches to keep this cheap.
            if i % 32 == 31 && cutoff.is_finite() && e / n * penalty >= cutoff {
                return None;
            }
        }
        // Mild preference for nearer sources on ties.
        Some(e / n * penalty)
    };
    // Coarse grid over the search window, then a full-resolution refinement around the winner.
    let step = (max_radius / 24).max(1);
    let mut best: Option<((i32, i32), f32)> = None;
    let consider = |dx: i32, dy: i32, best: &mut Option<((i32, i32), f32)>| {
        if let Some(e) = eval(dx, dy, best.map_or(f32::INFINITY, |(_, error)| error))
            && best.is_none_or(|(_, be)| e < be)
        {
            *best = Some(((dx, dy), e));
        }
    };
    let mut dy = -max_radius;
    while dy <= max_radius {
        let mut dx = -max_radius;
        while dx <= max_radius {
            consider(dx, dy, &mut best);
            dx += step;
        }
        dy += step;
    }
    if step > 1
        && let Some(((bx, by), _)) = best
    {
        for dy in (by - step)..=(by + step) {
            for dx in (bx - step)..=(bx + step) {
                consider(dx, dy, &mut best);
            }
        }
    }
    best.map(|b| b.0)
}

/// Create texture: fill the hole with blocks of surrounding texture (see module docs).
pub fn synthesize(w: usize, h: usize, ch: usize, img: &[f32], hole: &[bool], block: usize, seed: u64) -> Vec<f32> {
    let mut out = img.to_vec();
    let Some((bx0, by0, bx1, by1)) = hole_bbox(w, h, hole) else { return out };
    let b = block.max(4) as i32;
    let ov = (b / 3).max(1);
    let hs = integral(w, h, hole);
    // Candidate block origins: whole block (with overlap) inside the grid and outside the hole.
    let mut cands = Vec::new();
    for y in ov..(h as i32 - b - ov) {
        for x in ov..(w as i32 - b - ov) {
            if window_count(&hs, w, h, x - ov, y - ov, x + b + ov, y + b + ov) == 0 {
                cands.push((x, y));
            }
        }
    }
    if cands.is_empty() {
        return membrane_fill(w, h, ch, img, hole);
    }
    let mut filled: Vec<bool> = hole.iter().map(|x| !x).collect();
    let mut rng = Rng::new(seed);
    let mut by = by0 as i32;
    while by < by1 as i32 {
        let mut bx = bx0 as i32;
        while bx < bx1 as i32 {
            if window_count(&hs, w, h, bx, by, bx + b, by + b) > 0 {
                // Pick the candidate whose block best agrees with already-known pixels around/inside.
                let mut best = (cands[0], f32::INFINITY);
                for _ in 0..48 {
                    let c = cands[(rng.next() % cands.len() as u64) as usize];
                    let (mut e, mut n) = (0.0f32, 0u32);
                    for y in (by - ov)..(by + b + ov) {
                        for x in (bx - ov)..(bx + b + ov) {
                            if x < 0 || y < 0 || x >= w as i32 || y >= h as i32 || !filled[y as usize * w + x as usize] {
                                continue;
                            }
                            let (a, s) = ((y as usize * w + x as usize) * ch, ((y - by + c.1) as usize * w + (x - bx + c.0) as usize) * ch);
                            for k in 0..ch {
                                let d = out[a + k] - img[s + k];
                                e += d * d;
                            }
                            n += 1;
                        }
                    }
                    let e = if n == 0 { 0.0 } else { e / n as f32 };
                    if e < best.1 {
                        best = (c, e);
                    }
                }
                let c = best.0;
                let (x0, y0) = ((bx - ov).max(0), (by - ov).max(0));
                let (x1, y1) = ((bx + b + ov).min(w as i32), (by + b + ov).min(h as i32));
                let src = |x: i32, y: i32| ((y - by + c.1) as usize * w + (x - bx + c.0) as usize) * ch;
                // Squared difference between what is there and the block, where both exist.
                let err = |x: i32, y: i32| {
                    let i = y as usize * w + x as usize;
                    if !(hole[i] && filled[i]) {
                        return 0.0;
                    }
                    let (a, s) = (i * ch, src(x, y));
                    (0..ch).map(|q| (out[a + q] - img[s + q]).powi(2)).sum::<f32>()
                };
                // Minimum-error boundary cuts through the overlaps with the blocks to the left and
                // above: the block is copied right of and below them, untouched pixels are kept
                // on the other side. Copying (rather than averaging) keeps the texture's grain.
                let left = min_cut((x0..(bx + ov).min(x1)).len(), (y0..y1).len(), |u, v| err(x0 + u as i32, y0 + v as i32));
                let top = min_cut((y0..(by + ov).min(y1)).len(), (x0..x1).len(), |u, v| err(x0 + v as i32, y0 + u as i32));
                for y in y0..y1 {
                    for x in x0..x1 {
                        let i = y as usize * w + x as usize;
                        if !hole[i] {
                            continue;
                        }
                        let (u, v) = ((x - x0) as usize, (y - y0) as usize);
                        let keep = filled[i] && (left.get(v).is_some_and(|s| u < *s) || top.get(u).is_some_and(|s| v < *s));
                        if !keep {
                            let (a, s) = (i * ch, src(x, y));
                            out[a..a + ch].copy_from_slice(&img[s..s + ch]);
                        }
                        filled[i] = true;
                    }
                }
            }
            bx += b;
        }
        by += b;
    }
    out
}

/// The minimum-error cut through a band `across` wide and `along` long (Efros & Freeman's
/// boundary cut, by dynamic programming): for each step along the band, the offset across it
/// where the cut runs (`err(across, along)` is the cost of a pixel). Pixels before the cut keep
/// what is there. Empty when there is no cost anywhere (nothing to cut through).
fn min_cut(across: usize, along: usize, err: impl Fn(usize, usize) -> f32) -> Vec<usize> {
    if across == 0 || along == 0 {
        return Vec::new();
    }
    // Cumulative cost, step by step along the band; the cut may move by one pixel per step.
    let mut acc: Vec<f32> = (0..along).flat_map(|v| (0..across).map(move |u| (u, v))).map(|(u, v)| err(u, v)).collect();
    if acc.iter().all(|x| *x == 0.0) {
        return Vec::new();
    }
    for v in 1..along {
        for u in 0..across {
            let lo = u.saturating_sub(1);
            let hi = (u + 1).min(across - 1);
            let m = (lo..=hi).map(|k| acc[(v - 1) * across + k]).fold(f32::INFINITY, f32::min);
            acc[v * across + u] += m;
        }
    }
    let argmin = |v: usize, lo: usize, hi: usize| (lo..=hi).min_by(|a, b| acc[v * across + a].total_cmp(&acc[v * across + b])).unwrap_or(lo);
    let mut cut = vec![0; along];
    let mut u = argmin(along - 1, 0, across - 1);
    for v in (0..along).rev() {
        if v + 1 < along {
            u = argmin(v, u.saturating_sub(1), (u + 1).min(across - 1));
        }
        cut[v] = u;
    }
    cut
}
