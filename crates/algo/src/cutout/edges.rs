//! Edges: the cracks between regions, linked into chains from junction to junction, each crack
//! placed where the anti-aliasing says the edge is.

use super::{OUT, Pt, Px};
use crate::par;

/// A chain's points, and whether it is closed.
pub(super) type Chain = (Vec<Pt>, bool);
/// A pixel, or a lattice point (the corner at a pixel's top left).
type Cell = (i64, i64);

/// A step along the lattice, y down.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Dir {
    E,
    W,
    S,
    N,
}

/// Links the cracks between regions into chains. Each chain is its points (the junctions it runs
/// between, if any, and one sub-pixel point per crack) and whether it is closed; its sides are
/// the regions on its left and right (y down).
pub(super) fn follow_edges(reg: &[u32], colours: &[Px], c: &[Px], w: usize, h: usize) -> (Vec<Chain>, Vec<(u32, u32)>) {
    let (wi, hi) = (w as i64, h as i64);
    let at = |x: i64, y: i64| -> u32 { if x < 0 || y < 0 || x >= wi || y >= hi { OUT } else { reg[y as usize * w + x as usize] } };
    let hcount = w * (h + 1);
    // Horizontal crack (x, y)–(x+1, y) is `y * w + x`; vertical (x, y)–(x, y+1) follows them.
    let hc = |x: i64, y: i64| (y as usize) * w + x as usize;
    let vc = |x: i64, y: i64| hcount + (y as usize) * (w + 1) + x as usize;
    let mut visited = vec![false; hcount + (w + 1) * h];
    // The crack leaving lattice point (x, y) in `d`: its id, the pixels on its left and right,
    // and the point it reaches.
    let step = |x: i64, y: i64, d: Dir| -> Option<(usize, Cell, Cell, Cell)> {
        match d {
            Dir::E if x < wi => Some((hc(x, y), (x, y - 1), (x, y), (x + 1, y))),
            Dir::W if x > 0 => Some((hc(x - 1, y), (x - 1, y), (x - 1, y - 1), (x - 1, y))),
            Dir::S if y < hi => Some((vc(x, y), (x, y), (x - 1, y), (x, y + 1))),
            Dir::N if y > 0 => Some((vc(x, y - 1), (x - 1, y - 1), (x, y - 1), (x, y - 1))),
            _ => None,
        }
    };
    // Which cracks separate two regions, and which lattice points are junctions, worked out
    // once (in parallel) before the walk.
    let mut is_edge = vec![false; visited.len()];
    let (hb, vb) = is_edge.split_at_mut(hcount);
    par::chunks_mut(hb, w, true, |y, row| {
        for (x, b) in row.iter_mut().enumerate() {
            *b = at(x as i64, y as i64 - 1) != at(x as i64, y as i64);
        }
    });
    par::chunks_mut(vb, w + 1, true, |y, row| {
        for (x, b) in row.iter_mut().enumerate() {
            *b = at(x as i64 - 1, y as i64) != at(x as i64, y as i64);
        }
    });
    let boundary = |x: i64, y: i64, d: Dir| step(x, y, d).filter(|s| is_edge[s.0]);
    let mut is_junction = vec![false; (w + 1) * (h + 1)];
    par::chunks_mut(&mut is_junction, w + 1, true, |y, row| {
        let y = y as i64;
        for (x, j) in row.iter_mut().enumerate() {
            let x = x as i64;
            let deg = [Dir::E, Dir::W, Dir::S, Dir::N].iter().filter(|&&d| boundary(x, y, d).is_some()).count();
            *j = deg >= 3 || (deg > 0 && (x == 0 || x == wi) && (y == 0 || y == hi));
        }
    });
    let junction = |x: i64, y: i64| is_junction[y as usize * (w + 1) + x as usize];
    // One point per crack: its middle, moved along the normal from left to right pixel by what
    // their colours say about where the edge really is.
    let crack_point = |l: (i64, i64), r: (i64, i64)| -> Pt {
        let mid = [(l.0 + r.0 + 1) as f32 / 2.0, (l.1 + r.1 + 1) as f32 / 2.0];
        let (n0, n1) = ((r.0 - l.0) as f32, (r.1 - l.1) as f32);
        let (ra, rb) = (at(l.0, l.1), at(r.0, r.1));
        let (Some(ca), Some(cb)) = (colours.get(ra as usize), colours.get(rb as usize)) else { return mid };
        let d = [cb[0] - ca[0], cb[1] - ca[1], cb[2] - ca[2]];
        let dd = d[0] * d[0] + d[1] * d[1] + d[2] * d[2];
        if dd < 1e-4 {
            return mid;
        }
        // How far each pixel's colour is from its own region's towards the other's (0–1).
        let frac = |p: (i64, i64)| {
            let q = c[p.1 as usize * w + p.0 as usize];
            ((q[0] - ca[0]) * d[0] + (q[1] - ca[1]) * d[1] + (q[2] - ca[2]) * d[2]) / dd
        };
        let (fl, fr) = (frac(l).clamp(0.0, 1.0), frac(r).clamp(0.0, 1.0));
        let off = ((1.0 - fr) - fl).clamp(-0.5, 0.5);
        [mid[0] + n0 * off, mid[1] + n1 * off]
    };
    let mut chains = Vec::new();
    let mut sides = Vec::new();
    let mut walk = |x: i64, y: i64, first: Dir, visited: &mut Vec<bool>| {
        let open = junction(x, y);
        let mut pts = if open { vec![[x as f32, y as f32]] } else { Vec::new() };
        let (mut cx, mut cy, mut d) = (x, y, first);
        let mut side = (OUT, OUT);
        let mut closed = !open;
        while let Some((id, l, r, to)) = boundary(cx, cy, d) {
            if visited[id] {
                break;
            }
            visited[id] = true;
            if pts.len() <= usize::from(open) {
                side = (at(l.0, l.1), at(r.0, r.1));
            }
            pts.push(crack_point(l, r));
            (cx, cy) = to;
            if junction(cx, cy) {
                pts.push([cx as f32, cy as f32]);
                closed = false;
                break;
            }
            let back = match d {
                Dir::E => Dir::W,
                Dir::W => Dir::E,
                Dir::S => Dir::N,
                Dir::N => Dir::S,
            };
            let Some(next) = [Dir::E, Dir::S, Dir::W, Dir::N].into_iter().find(|&nd| nd != back && boundary(cx, cy, nd).is_some_and(|b| !visited[b.0])) else {
                break;
            };
            d = next;
        }
        if pts.len() >= 2 {
            chains.push((pts, closed));
            sides.push(side);
        }
    };
    for y in 0..=hi {
        for x in 0..=wi {
            if junction(x, y) {
                for d in [Dir::E, Dir::S, Dir::W, Dir::N] {
                    if boundary(x, y, d).is_some_and(|b| !visited[b.0]) {
                        walk(x, y, d, &mut visited);
                    }
                }
            }
        }
    }
    // What is left are closed loops with no junction (an island inside one other region).
    for y in 0..=hi {
        for x in 0..=wi {
            for d in [Dir::E, Dir::S] {
                if boundary(x, y, d).is_some_and(|b| !visited[b.0]) {
                    walk(x, y, d, &mut visited);
                }
            }
        }
    }
    (chains, sides)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Regions drawn as digits, one per pixel, with grey `colours` (region `k` is `k / 4`).
    fn regions(rows: &[&str]) -> (Vec<u32>, Vec<Px>, usize, usize) {
        let reg: Vec<u32> = rows.iter().flat_map(|r| r.bytes()).map(|b| u32::from(b - b'0')).collect();
        let n = reg.iter().max().map_or(0, |m| m + 1);
        (reg, (0..n).map(|k| [k as f32 / 4.0; 3]).collect(), rows[0].len(), rows.len())
    }

    /// The chain whose sides are regions `a` and `b`, either way round.
    fn between(chains: &[Chain], sides: &[(u32, u32)], a: u32, b: u32) -> Vec<Chain> {
        chains.iter().zip(sides).filter(|(_, s)| **s == (a, b) || **s == (b, a)).map(|(c, _)| c.clone()).collect()
    }

    #[test]
    fn the_edge_between_two_regions_is_one_chain_between_junctions() {
        let (reg, colours, w, h) = regions(&["000111", "000111", "000111", "000111"]);
        let c: Vec<Px> = reg.iter().map(|&r| colours[r as usize]).collect();
        let (chains, sides) = follow_edges(&reg, &colours, &c, w, h);
        let inner = between(&chains, &sides, 0, 1);
        assert_eq!(inner.len(), 1, "{chains:?}");
        let (pts, closed) = &inner[0];
        assert!(!closed);
        // From junction to junction (where it meets the canvas edge), one point per crack.
        let ends = [pts[0], pts[pts.len() - 1]];
        assert!(ends.contains(&[3.0, 0.0]) && ends.contains(&[3.0, 4.0]), "{pts:?}");
        assert_eq!(pts.len(), 2 + 4);
        assert!(pts.iter().all(|p| p[0] == 3.0), "flat colours: the edge is on the crack: {pts:?}");
        // Every crack between two regions, the canvas edge included, is in exactly one chain.
        let cracks: usize = chains.iter().map(|(p, closed)| p.len() - if *closed { 0 } else { 2 }).sum();
        assert_eq!(cracks, 4 + 2 * (w + h));
    }

    #[test]
    fn an_island_is_one_closed_chain() {
        let (reg, colours, w, h) = regions(&["00000", "01100", "01100", "00000"]);
        let c: Vec<Px> = reg.iter().map(|&r| colours[r as usize]).collect();
        let (chains, sides) = follow_edges(&reg, &colours, &c, w, h);
        let island = between(&chains, &sides, 0, 1);
        assert_eq!(island.len(), 1);
        assert!(island[0].1, "closed");
        assert_eq!(island[0].0.len(), 8, "one point per crack around the 2x2 island");
    }

    #[test]
    fn an_anti_aliased_edge_is_placed_inside_its_pixel() {
        // Region 0 is black and region 1 white; the last column of region 0 is half white, so
        // the edge runs down the middle of that column.
        let (reg, _, w, h) = regions(&["000111", "000111", "000111"]);
        let colours = vec![[0.0; 3], [1.0; 3]];
        let c: Vec<Px> = reg.iter().enumerate().map(|(i, &r)| if i % w == 2 { [0.5; 3] } else { colours[r as usize] }).collect();
        let (chains, sides) = follow_edges(&reg, &colours, &c, w, h);
        let inner = between(&chains, &sides, 0, 1);
        let pts = &inner[0].0;
        assert!(pts[1..pts.len() - 1].iter().all(|p| (p[0] - 2.5).abs() < 1e-5), "{pts:?}");
    }
}
