//! Regions: connected areas of one palette colour, the merge of small ones and slivers, and the
//! paper colour of each region.

use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap};
use std::hash::BuildHasherDefault;

use super::colour::{in_gamut, oklab, srgb};
use super::util::{FxHasher, find, transpose, transpose_u32};
use super::{Px, Settings};
use crate::par;

/// 4-connected components of equal labels, numbered in raster order of their first pixel.
pub(super) fn components(labels: &[u8], w: usize, h: usize) -> (Vec<u32>, usize) {
    let mut comp = vec![0u32; w * h];
    let mut parent: Vec<u32> = Vec::new();
    for y in 0..h {
        for x in 0..w {
            let i = y * w + x;
            let up = (y > 0 && labels[i - w] == labels[i]).then(|| comp[i - w]);
            let left = (x > 0 && labels[i - 1] == labels[i]).then(|| comp[i - 1]);
            comp[i] = match (up, left) {
                (Some(a), Some(b)) => {
                    let (ra, rb) = (find(&mut parent, a), find(&mut parent, b));
                    let (lo, hi) = (ra.min(rb), ra.max(rb));
                    parent[hi as usize] = lo;
                    lo
                }
                (Some(a), None) | (None, Some(a)) => a,
                (None, None) => {
                    parent.push(parent.len() as u32);
                    parent.len() as u32 - 1
                }
            };
        }
    }
    let mut index = vec![u32::MAX; parent.len()];
    let mut n = 0u32;
    for c in comp.iter_mut() {
        let root = find(&mut parent, *c) as usize;
        if index[root] == u32::MAX {
            index[root] = n;
            n += 1;
        }
        *c = index[root];
    }
    (comp, n as usize)
}

/// Merges small and sliver components into their neighbours. Returns each pixel's region and
/// each region's mean colour.
pub(super) fn merge_regions(comp: &[u32], n: usize, c: &[Px], w: usize, h: usize, s: &Settings) -> (Vec<u32>, Vec<Px>) {
    let mut area = vec![0u32; n];
    let mut sum = vec![[0.0f64; 3]; n];
    for (&k, p) in comp.iter().zip(c) {
        let k = k as usize;
        area[k] += 1;
        sum[k] = [sum[k][0] + f64::from(p[0]), sum[k][1] + f64::from(p[1]), sum[k][2] + f64::from(p[2])];
    }
    // Shared border lengths (in cracks) and perimeters, the canvas edge included.
    let mut shared: HashMap<u64, u32, BuildHasherDefault<FxHasher>> = HashMap::default();
    let mut perim = vec![0u32; n];
    for y in 0..h {
        for x in 0..w {
            let a = comp[y * w + x];
            for (nx, ny) in [(x + 1, y), (x, y + 1)] {
                if nx < w && ny < h {
                    let b = comp[ny * w + nx];
                    if a != b {
                        *shared.entry((u64::from(a.min(b)) << 32) | u64::from(a.max(b))).or_insert(0) += 1;
                    }
                }
            }
            perim[a as usize] += u32::from(x == 0) + u32::from(x + 1 == w) + u32::from(y == 0) + u32::from(y + 1 == h);
        }
    }
    let mut adj: Vec<Vec<(u32, u32)>> = vec![Vec::new(); n];
    let mut pairs: Vec<(u64, u32)> = shared.into_iter().collect();
    pairs.sort_unstable();
    for (key, len) in pairs {
        let (a, b) = ((key >> 32) as u32, key as u32);
        adj[a as usize].push((b, len));
        adj[b as usize].push((a, len));
        perim[a as usize] += len;
        perim[b as usize] += len;
    }
    let (min_area, sliver) = (s.min_area(), s.sliver());
    let mergeable = |a: u32, p: u32| a < min_area || 2.0 * a as f32 / (p.max(1) as f32) < sliver;
    let mut parent: Vec<u32> = (0..n as u32).collect();
    let mut heap: BinaryHeap<Reverse<(u32, u32)>> = (0..n).filter(|&i| mergeable(area[i], perim[i])).map(|i| Reverse((area[i], i as u32))).collect();
    while let Some(Reverse((a, id))) = heap.pop() {
        let i = id as usize;
        if find(&mut parent, id) != id || area[i] != a || !mergeable(a, perim[i]) {
            continue;
        }
        // The neighbours as they are now, with their total shared border.
        let mut nb: Vec<(u32, u32)> = std::mem::take(&mut adj[i]).into_iter().map(|(k, len)| (find(&mut parent, k), len)).filter(|&(k, _)| k != id).collect();
        nb.sort_unstable();
        nb.dedup_by(|b, a| {
            if a.0 == b.0 {
                a.1 += b.1;
                true
            } else {
                false
            }
        });
        let Some(&(t, len)) = nb.iter().max_by(|a, b| a.1.cmp(&b.1).then(b.0.cmp(&a.0))) else {
            adj[i] = nb;
            continue;
        };
        let ti = t as usize;
        parent[i] = t;
        area[ti] += area[i];
        sum[ti] = [sum[ti][0] + sum[i][0], sum[ti][1] + sum[i][1], sum[ti][2] + sum[i][2]];
        perim[ti] = (perim[ti] + perim[i]).saturating_sub(2 * len);
        // Appended as they are; resolved and summed when `t` itself merges.
        adj[ti].extend(nb.into_iter().filter(|&(k, _)| k != t));
        if mergeable(area[ti], perim[ti]) {
            heap.push(Reverse((area[ti], t)));
        }
    }
    let mut index = vec![u32::MAX; n];
    let mut colours = Vec::new();
    let mut reg = vec![0u32; comp.len()];
    for (r, &k) in reg.iter_mut().zip(comp) {
        let root = find(&mut parent, k) as usize;
        if index[root] == u32::MAX {
            index[root] = colours.len() as u32;
            let a = f64::from(area[root].max(1));
            colours.push([(sum[root][0] / a) as f32, (sum[root][1] / a) as f32, (sum[root][2] / a) as f32]);
        }
        *r = index[root];
    }
    (reg, colours)
}

/// The paper colour of every region. A plain average of a region's pixels greys it (a face of
/// pink and cream patches averages to beige, hues cancel), and a whole image of averages sits in
/// the middle tones. So in Oklab each region keeps its average hue at the average *strength* of
/// its pixels' colour, and lightness is stretched over the image's regions so the darkest paper is
/// near black and the lightest near white. Out-of-gamut colours lose chroma, not hue.
pub(super) fn paper_colours(reg: &[u32], n: usize, lab: &[Px]) -> Vec<Px> {
    let mut acc = vec![[0.0f64; 5]; n];
    for (&r, p) in reg.iter().zip(lab) {
        if let Some(a) = acc.get_mut(r as usize) {
            let chroma = (p[1] * p[1] + p[2] * p[2]).sqrt();
            *a = [a[0] + f64::from(p[0]), a[1] + f64::from(p[1]), a[2] + f64::from(p[2]), a[3] + f64::from(chroma), a[4] + 1.0];
        }
    }
    let mean: Vec<(Px, f32, f64)> = acc
        .iter()
        .map(|a| {
            let k = a[4].max(1.0);
            ([(a[0] / k) as f32, (a[1] / k) as f32, (a[2] / k) as f32], (a[3] / k) as f32, a[4])
        })
        .collect();
    // The lightness range of the image's paper, by area (ignoring the extreme 1% at each end).
    let mut by_l: Vec<(f32, f64)> = mean.iter().map(|m| (m.0[0], m.2)).collect();
    by_l.sort_by(|a, b| a.0.total_cmp(&b.0));
    let total: f64 = by_l.iter().map(|m| m.1).sum();
    let at = |q: f64| {
        let mut run = 0.0;
        for &(l, a) in &by_l {
            run += a;
            if run >= q * total {
                return l;
            }
        }
        by_l.last().map_or(0.5, |m| m.0)
    };
    let (lo, hi) = (at(0.01), at(0.99));
    const STRETCH: f32 = 0.75;
    mean.iter()
        .map(|(m, chroma, _)| {
            let ab = (m[1] * m[1] + m[2] * m[2]).sqrt();
            let k = if ab > 1e-5 { chroma / ab } else { 1.0 };
            let stretched = if hi - lo > 0.05 { m[0] + (0.04 + (m[0] - lo) / (hi - lo) * 0.88 - m[0]) * STRETCH } else { m[0] };
            let stretched = stretched.clamp(0.0, 1.0);
            let colour = |l: f32, t: f32| srgb([l, m[1] * k * t, m[2] * k * t]);
            if let Some(c) = colour(stretched, 1.0) {
                return c;
            }
            // Colour comes before contrast: a strong colour the stretch took out of gamut (a
            // bright blue pushed towards white) moves back towards its own lightness first.
            let own = m[0].clamp(0.0, 1.0);
            if colour(own, 1.0).is_some() {
                let (mut ok, mut bad) = (own, stretched);
                for _ in 0..12 {
                    let l = (ok + bad) / 2.0;
                    if colour(l, 1.0).is_some() { ok = l } else { bad = l }
                }
                return colour(ok, 1.0).unwrap_or([ok, ok, ok]);
            }
            // Even its own lightness can't hold it: keep the stretch, lose chroma (not hue).
            let (mut ok, mut bad) = (0.0f32, 1.0f32);
            for _ in 0..12 {
                let t = (ok + bad) / 2.0;
                if colour(stretched, t).is_some() { ok = t } else { bad = t }
            }
            colour(stretched, ok).unwrap_or([stretched, stretched, stretched])
        })
        .collect()
}

/// Each pixel's paper colour: its region's lightness (flat across the shape) with the average
/// hue, at the average strength, of the region's own pixels within about `reach` pixels. Colour
/// can drift gently across a large shape (pink cheeks on a cream face) but never crosses a cut.
/// `paper` is each region's colour (from [`paper_colours`]).
pub(super) fn local_paper(reg: &[u32], paper: &[Px], lab: &[Px], w: usize, h: usize, reach: f32) -> Vec<Px> {
    let mut v: Vec<Px> = lab.iter().map(|p| [p[1], p[2], (p[1] * p[1] + p[2] * p[2]).sqrt()]).collect();
    let regt = transpose_u32(reg, w, h);
    let a = (-1.0 / reach.max(0.5)).exp();
    for _ in 0..2 {
        same_region_rows(&mut v, reg, w, a);
        let mut t = transpose(&v, w, h);
        same_region_rows(&mut t, &regt, h, a);
        v = transpose(&t, h, w);
    }
    let light: Vec<f32> = paper.iter().map(|c| oklab(*c)[0]).collect();
    let mut out = vec![[0.0f32; 3]; w * h];
    par::chunks_mut(&mut out, w, true, |y, row| {
        for (x, o) in row.iter_mut().enumerate() {
            let i = y * w + x;
            let (q, l) = (v[i], light.get(reg[i] as usize).copied().unwrap_or(0.5));
            let ab = (q[0] * q[0] + q[1] * q[1]).sqrt();
            let k = if ab > 1e-5 { q[2] / ab } else { 1.0 };
            *o = in_gamut(l, q[0] * k, q[1] * k);
        }
    });
    out
}

/// Exponential smoothing along each row (both ways) that restarts at every change of region.
fn same_region_rows(v: &mut [Px], reg: &[u32], w: usize, a: f32) {
    let mut chunks: Vec<(&mut [Px], &[u32])> = v.chunks_mut(w).zip(reg.chunks(w)).collect();
    let f = |(row, rg): &mut (&mut [Px], &[u32])| {
        let n = row.len();
        let src = row.to_vec();
        let mut fwd = vec![([0.0f32; 3], 0.0f32); n];
        for x in 0..n {
            let (p, d) = if x > 0 && rg[x] == rg[x - 1] { fwd[x - 1] } else { ([0.0; 3], 0.0) };
            fwd[x] = ([src[x][0] + a * p[0], src[x][1] + a * p[1], src[x][2] + a * p[2]], 1.0 + a * d);
        }
        let mut back = ([0.0f32; 3], 0.0f32);
        for x in (0..n).rev() {
            if x + 1 < n && rg[x] != rg[x + 1] {
                back = ([0.0; 3], 0.0);
            }
            back = ([src[x][0] + a * back.0[0], src[x][1] + a * back.0[1], src[x][2] + a * back.0[2]], 1.0 + a * back.1);
            let den = fwd[x].1 + back.1 - 1.0;
            row[x] = std::array::from_fn(|c| (fwd[x].0[c] + back.0[c] - src[x][c]) / den);
        }
    };
    #[cfg(not(target_arch = "wasm32"))]
    {
        use rayon::prelude::*;
        chunks.par_iter_mut().for_each(f);
    }
    #[cfg(target_arch = "wasm32")]
    chunks.iter_mut().for_each(f);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings(simplicity: f32) -> Settings {
        Settings { levels: 4, simplicity, fidelity: 2.0 }
    }

    /// Labels drawn as text, one character per pixel.
    fn labels(rows: &[&str]) -> (Vec<u8>, usize, usize) {
        (rows.iter().flat_map(|r| r.bytes()).collect(), rows[0].len(), rows.len())
    }

    #[test]
    fn components_are_four_connected_and_numbered_in_raster_order() {
        let (l, w, h) = labels(&["aab", "abb", "bba"]);
        let (comp, n) = components(&l, w, h);
        // The top-left `a`s, the `b`s (one area), then the lone `a` (touching the first only
        // diagonally), in the order their first pixels come.
        assert_eq!(n, 3);
        assert_eq!(comp, vec![0, 0, 1, 0, 1, 1, 1, 1, 2]);
        // A U shape: both arms are found as separate runs first and joined further down.
        let (l, w, h) = labels(&["a.a", "a.a", "aaa"]);
        let (comp, n) = components(&l, w, h);
        assert_eq!(n, 2);
        assert_eq!(comp[0], comp[2]);
    }

    /// Regions from labels, with one flat colour per label.
    fn merged(rows: &[&str], simplicity: f32) -> (Vec<u32>, Vec<Px>) {
        let (l, w, h) = labels(rows);
        let c: Vec<Px> = l.iter().map(|&b| [f32::from(b - b'a') / 4.0; 3]).collect();
        let (comp, n) = components(&l, w, h);
        merge_regions(&comp, n, &c, w, h, &settings(simplicity))
    }

    #[test]
    fn small_regions_merge_into_the_neighbour_they_share_most_border_with() {
        let rows = ["aaaaaaaa", "aaaaaaaa", "aacaaaaa", "aaccbbbb", "aaaabbbb", "aaaabbbb", "aaaabbbb"];
        // At Edge Simplicity 0 every region is kept.
        assert_eq!(merged(&rows, 0.0).1.len(), 3);
        // At 1 (regions under four pixels merge) the three `c`s go into the `a`s, which they share seven
        // cracks with (one with the `b`s).
        let (reg, colours) = merged(&rows, 1.0);
        assert_eq!(colours.len(), 2);
        assert_eq!(reg[2 * 8 + 2], reg[0]);
        assert_ne!(reg[3 * 8 + 4], reg[0]);
        // A region's colour is the mean of its pixels: 37 `a`s (0.0) and 3 `c`s (0.5).
        let a = colours[reg[0] as usize][0];
        assert!((a - 1.5 / 40.0).abs() < 1e-6, "{a}");
    }

    #[test]
    fn slivers_merge_though_they_are_large() {
        // A line one pixel wide between two areas: the blend along an edge, not a shape.
        let rows: Vec<String> = (0..40).map(|_| format!("{}c{}", "a".repeat(10), "b".repeat(9))).collect();
        let rows: Vec<&str> = rows.iter().map(String::as_str).collect();
        assert_eq!(merged(&rows, 0.0).1.len(), 3, "kept at Edge Simplicity 0");
        let (_, colours) = merged(&rows, 4.0);
        assert_eq!(colours.len(), 2, "40 pixels is over the 25 that merge at 4, but it is a sliver");
    }

    #[test]
    fn paper_is_stretched_towards_black_and_white_and_keeps_its_hue() {
        // Three regions: a dark and a light grey, and a mid-tone orange.
        let lab = [[0.35, 0.0, 0.0], [0.65, 0.0, 0.0], [0.5, 0.06, 0.05]];
        let reg: Vec<u32> = (0..300).map(|i| (i / 100) as u32).collect();
        let px: Vec<Px> = reg.iter().map(|&r| lab[r as usize]).collect();
        let paper: Vec<Px> = paper_colours(&reg, 3, &px).into_iter().map(oklab).collect();
        assert!(paper[0][0] < 0.25 && paper[1][0] > 0.75, "{paper:?}");
        assert!((paper[2][0] - 0.5).abs() < 0.05, "a middle tone stays in the middle: {paper:?}");
        assert!((paper[2][2] / paper[2][1] - 0.05 / 0.06).abs() < 0.05, "hue kept: {paper:?}");
        assert!(paper[0][1].abs() < 1e-3 && paper[0][2].abs() < 1e-3, "grey stays grey: {paper:?}");
    }

    #[test]
    fn local_colour_drifts_within_a_shape_but_never_crosses_a_cut() {
        // One light region, pink on the left and cream on the right, next to a blue one.
        let (w, h) = (30usize, 4usize);
        let reg: Vec<u32> = (0..w * h).map(|i| u32::from(i % w >= 20)).collect();
        let lab: Vec<Px> = (0..w * h)
            .map(|i| match i % w {
                0..10 => [0.8, 0.08, 0.02],
                10..20 => [0.8, 0.01, 0.04],
                _ => [0.4, -0.02, -0.12],
            })
            .collect();
        let paper = [srgb([0.8, 0.04, 0.03]).unwrap_or_default(), srgb([0.4, -0.02, -0.12]).unwrap_or_default()];
        let out: Vec<Px> = local_paper(&reg, &paper, &lab, w, h, 3.0).into_iter().map(oklab).collect();
        let (left, right, edge) = (out[w + 2], out[w + 17], out[w + 19]);
        assert!(left[1] > right[1] + 0.03, "the pink side is pinker: {left:?} vs {right:?}");
        assert!((left[0] - right[0]).abs() < 0.01, "one lightness across the shape: {left:?} vs {right:?}");
        assert!(edge[2] > 0.0, "no blue from the next shape at the cut: {edge:?}");
    }
}
