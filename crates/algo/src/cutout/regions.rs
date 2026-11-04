//! Regions: connected areas of one tone band, and the merge of small ones into their neighbours.

use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap};
use std::hash::BuildHasherDefault;

use super::util::{FxHasher, find};
use super::{Px, Settings};

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
    // Shared border lengths, in cracks.
    let mut shared: HashMap<u64, u32, BuildHasherDefault<FxHasher>> = HashMap::default();
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
        }
    }
    let mut adj: Vec<Vec<(u32, u32)>> = vec![Vec::new(); n];
    let mut pairs: Vec<(u64, u32)> = shared.into_iter().collect();
    pairs.sort_unstable();
    for (key, len) in pairs {
        let (a, b) = ((key >> 32) as u32, key as u32);
        adj[a as usize].push((b, len));
        adj[b as usize].push((a, len));
    }
    let min_area = s.min_area();
    let mergeable = |a: u32| a < min_area;
    let mut parent: Vec<u32> = (0..n as u32).collect();
    let mut heap: BinaryHeap<Reverse<(u32, u32)>> = (0..n).filter(|&i| mergeable(area[i])).map(|i| Reverse((area[i], i as u32))).collect();
    while let Some(Reverse((a, id))) = heap.pop() {
        let i = id as usize;
        if find(&mut parent, id) != id || area[i] != a || !mergeable(a) {
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
        let Some(&(t, _)) = nb.iter().max_by(|a, b| a.1.cmp(&b.1).then(b.0.cmp(&a.0))) else {
            adj[i] = nb;
            continue;
        };
        let ti = t as usize;
        parent[i] = t;
        area[ti] += area[i];
        sum[ti] = [sum[ti][0] + sum[i][0], sum[ti][1] + sum[i][1], sum[ti][2] + sum[i][2]];
        // Appended as they are; resolved and summed when `t` itself merges.
        adj[ti].extend(nb.into_iter().filter(|&(k, _)| k != t));
        if mergeable(area[ti]) {
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
        let rows = ["aaaaaaaaaa", "aaaaaaaaaa", "aacaaaaaaa", "aaccbbbbbb", "aaaabbbbbb", "aaaabbbbbb", "aaaabbbbbb", "aaaabbbbbb"];
        // At Edge Simplicity 0 a region needs 25 pixels: the three `c`s go into the `a`s, which
        // they share six cracks with (one with the `b`s); the 30 `b`s stay.
        let (reg, colours) = merged(&rows, 0.0);
        assert_eq!(colours.len(), 2);
        assert_eq!(reg[2 * 10 + 2], reg[0]);
        assert_ne!(reg[3 * 10 + 4], reg[0]);
        // A region's colour is the mean of its pixels: 47 `a`s (0.0) and 3 `c`s (0.5).
        let a = colours[reg[0] as usize][0];
        assert!((a - 1.5 / 50.0).abs() < 1e-6, "{a}");
        // At 4 a region needs 100 pixels, so the `b`s merge too.
        assert_eq!(merged(&rows, 4.0).1.len(), 1);
    }
}
