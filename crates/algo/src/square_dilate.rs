//! Grow a boolean mask by a clipped square: shared by the Remove Tool and the non-local completion.

/// `mask` grown by `r` cells with a square structuring element, clipped to the `w × h` grid: a
/// cell is set when any mask cell lies within chessboard distance `r`. A mask of the wrong size
/// comes back unchanged.
///
/// Work stays proportional to the mask's extent, not the grid's: a mask cell costs `(2r+1)²`
/// stores directly, which is the cheapest for a small or sparse mask; a bulky one is grown by two
/// running-count passes over its own expanded bounding box instead.
pub(crate) fn dilate_square(w: usize, h: usize, mask: &[bool], r: usize) -> Vec<bool> {
    if r == 0 || mask.len() != w.saturating_mul(h) {
        return mask.to_vec();
    }
    let Some(b) = bounds(w, mask) else {
        return vec![false; mask.len()];
    };
    let mut out = vec![false; mask.len()];
    match choose(w, h, &b, r) {
        Path::Direct => direct(w, h, mask, r, &b, &mut out),
        Path::Separable => {
            separable(w, h, mask, r, &b, &mut out);
        }
    }
    out
}

/// The tight box around the set cells (inclusive corners) and how many there are. A mask of at
/// most `KEEP` set cells also keeps their indices (`cells.len() == count`), so the direct path
/// needn't scan the grid a second time.
struct Bounds {
    x0: usize,
    y0: usize,
    x1: usize,
    y1: usize,
    count: usize,
    cells: Vec<usize>,
}

/// Most set cells whose indices `bounds` keeps.
const KEEP: usize = 1 << 12;

#[derive(Debug, PartialEq, Eq)]
enum Path {
    Direct,
    Separable,
}

/// `None` when no cell is set. `mask` has `w` columns.
fn bounds(w: usize, mask: &[bool]) -> Option<Bounds> {
    let mut b = Bounds { x0: usize::MAX, y0: usize::MAX, x1: 0, y1: 0, count: 0, cells: Vec::new() };
    // Most cells are unset: skip 64 at a time with a vectorised test before looking at any.
    for (c, chunk) in mask.chunks(64).enumerate().filter(|(_, chunk)| chunk.iter().any(|m| *m)) {
        for (j, _) in chunk.iter().enumerate().filter(|(_, m)| **m) {
            let i = c * 64 + j;
            let (x, y) = (i % w, i / w);
            // Row-major order: `y` never decreases.
            (b.x0, b.y0, b.x1, b.y1, b.count) = (b.x0.min(x), b.y0.min(y), b.x1.max(x), y, b.count + 1);
            if b.count <= KEEP {
                b.cells.push(i);
            } else if !b.cells.is_empty() {
                b.cells = Vec::new();
            }
        }
    }
    (b.count > 0).then_some(b)
}

/// The box around the set cells grown by `r` and clipped to the grid: `(x0, x1, y0, y1)`, inclusive.
fn expanded(w: usize, h: usize, b: &Bounds, r: usize) -> (usize, usize, usize, usize) {
    (b.x0.saturating_sub(r), b.x1.saturating_add(r).min(w - 1), b.y0.saturating_sub(r), b.y1.saturating_add(r).min(h - 1))
}

/// Cells of scratch the separable path allocates: the rows that can hold set cells, across the
/// expanded box. Never more than the expanded box's area.
#[cfg(test)]
fn scratch_cells(w: usize, h: usize, b: &Bounds, r: usize) -> usize {
    let (ex0, ex1, _, _) = expanded(w, h, b, r);
    (b.y1 - b.y0 + 1).saturating_mul(ex1 - ex0 + 1)
}

/// The cheaper way to grow the mask: the direct squares when they cost no more than two passes
/// over the expanded box.
fn choose(w: usize, h: usize, b: &Bounds, r: usize) -> Path {
    let side = r.saturating_mul(2).saturating_add(1);
    let direct = b.count.saturating_mul(side.saturating_mul(side));
    let (ex0, ex1, ey0, ey1) = expanded(w, h, b, r);
    let separable = (ex1 - ex0 + 1).saturating_mul(ey1 - ey0 + 1).saturating_mul(2);
    if direct <= separable { Path::Direct } else { Path::Separable }
}

/// Set the clipped `(2r+1)²` square around every set cell.
fn direct(w: usize, h: usize, mask: &[bool], r: usize, b: &Bounds, out: &mut [bool]) {
    let mut stamp = |x: usize, y: usize| {
        let (sx, ex) = (x.saturating_sub(r), x.saturating_add(r).min(w - 1));
        for ty in y.saturating_sub(r)..=y.saturating_add(r).min(h - 1) {
            out[ty * w + sx..=ty * w + ex].fill(true);
        }
    };
    if b.cells.len() == b.count {
        b.cells.iter().for_each(|i| stamp(i % w, i / w));
        return;
    }
    for y in b.y0..=b.y1 {
        for (dx, _) in mask[y * w + b.x0..=y * w + b.x1].iter().enumerate().filter(|(_, m)| **m) {
            stamp(b.x0 + dx, y);
        }
    }
}

/// Two running-count passes (a cell is set when any cell within `r` along one axis is set),
/// confined to the expanded box. Returns the scratch cells it allocated.
fn separable(w: usize, h: usize, mask: &[bool], r: usize, b: &Bounds, out: &mut [bool]) -> usize {
    let (ex0, ex1, ey0, ey1) = expanded(w, h, b, r);
    let (bw, eh, rows) = (ex1 - ex0 + 1, ey1 - ey0 + 1, b.y1 - b.y0 + 1);
    // Horizontal: every set cell lies in `x0..=x1`, so clipping a row's window at the box instead
    // of the grid only drops cells that are unset anyway. Only the rows that hold set cells.
    let mut horiz = vec![false; rows * bw];
    for ry in 0..rows {
        let src = &mask[(b.y0 + ry) * w + ex0..=(b.y0 + ry) * w + ex1];
        let dst = &mut horiz[ry * bw..(ry + 1) * bw];
        let mut count = 0usize;
        for &s in &src[..bw.min(r)] {
            count += usize::from(s);
        }
        for (k, d) in dst.iter_mut().enumerate() {
            count += src.get(k.saturating_add(r)).map_or(0, |s| usize::from(*s));
            count -= k.checked_sub(r).and_then(|j| j.checked_sub(1)).and_then(|j| src.get(j)).map_or(0, |s| usize::from(*s));
            *d = count > 0;
        }
    }
    // Vertical, row-major: the expanded box's rows form a sequence of length `eh` whose element
    // `k` is the scratch row of output row `ey0 + k`, or all unset above and below the set rows.
    let row = |k: usize| if k < eh { (ey0 + k).checked_sub(b.y0).filter(|j| *j < rows).map(|j| &horiz[j * bw..(j + 1) * bw]) } else { None };
    let mut counts = vec![0usize; bw];
    for k in 0..eh.min(r) {
        if let Some(src) = row(k) {
            counts.iter_mut().zip(src).for_each(|(c, s)| *c += usize::from(*s));
        }
    }
    for k in 0..eh {
        if let Some(src) = row(k.saturating_add(r)) {
            counts.iter_mut().zip(src).for_each(|(c, s)| *c += usize::from(*s));
        }
        if let Some(src) = k.checked_sub(r).and_then(|j| j.checked_sub(1)).and_then(row) {
            counts.iter_mut().zip(src).for_each(|(c, s)| *c -= usize::from(*s));
        }
        let dst = &mut out[(ey0 + k) * w + ex0..(ey0 + k) * w + ex1 + 1];
        dst.iter_mut().zip(&counts).for_each(|(d, c)| *d = *c > 0);
    }
    horiz.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The loop the non-local completion used before this module: the oracle.
    fn reference(w: usize, h: usize, mask: &[bool], r: usize) -> Vec<bool> {
        let ri = r as i32;
        let mut targets = vec![false; w * h];
        for (i, _) in mask.iter().enumerate().filter(|(_, m)| **m) {
            let (x, y) = ((i % w) as i32, (i / w) as i32);
            for ty in (y - ri).max(0)..=(y + ri).min(h as i32 - 1) {
                for tx in (x - ri).max(0)..=(x + ri).min(w as i32 - 1) {
                    targets[ty as usize * w + tx as usize] = true;
                }
            }
        }
        targets
    }

    fn forced(w: usize, h: usize, mask: &[bool], r: usize, path: &Path) -> Vec<bool> {
        let mut out = vec![false; w * h];
        if let Some(b) = bounds(w, mask) {
            match path {
                Path::Direct => direct(w, h, mask, r, &b, &mut out),
                Path::Separable => {
                    separable(w, h, mask, r, &b, &mut out);
                }
            }
        }
        out
    }

    /// `dilate_square` and both paths, forced, against the oracle.
    fn check(w: usize, h: usize, mask: &[bool], r: usize, what: &str) {
        let want = reference(w, h, mask, r);
        assert!(dilate_square(w, h, mask, r) == want, "{what}: dilate_square {w}x{h} r={r}");
        assert!(forced(w, h, mask, r, &Path::Direct) == want, "{what}: direct {w}x{h} r={r}");
        assert!(forced(w, h, mask, r, &Path::Separable) == want, "{what}: separable {w}x{h} r={r}");
    }

    struct Rng(u64);

    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }

        fn mask(&mut self, n: usize, per_mille: u64) -> Vec<bool> {
            (0..n).map(|_| self.next() % 1000 < per_mille).collect()
        }
    }

    #[test]
    fn every_mask_on_small_grids_matches_the_oracle() {
        for w in 1..=3usize {
            for h in 1..=3usize {
                for bits in 0..1u32 << (w * h) {
                    let mask: Vec<bool> = (0..w * h).map(|i| bits >> i & 1 == 1).collect();
                    for r in [1, 2, 3, 5] {
                        check(w, h, &mask, r, "exhaustive");
                    }
                }
            }
        }
    }

    fn cases(w: usize, h: usize) -> Vec<(&'static str, Vec<bool>)> {
        let mut rng = Rng(0x9e37_79b9_7f4a_7c15 ^ (w * 31 + h) as u64);
        let at = |x: usize, y: usize| y * w + x;
        let single = |x: usize, y: usize| {
            let mut m = vec![false; w * h];
            m[at(x, y)] = true;
            m
        };
        let (cx, cy, mx, my) = (w / 2, h / 2, w - 1, h - 1);
        let mut two = single(0, 0);
        two[at(mx, my)] = true;
        let blob = |x0: usize, y0: usize| -> Vec<bool> {
            (0..w * h).map(|i| (x0..x0 + 3.min(w - x0)).contains(&(i % w)) && (y0..y0 + 3.min(h - y0)).contains(&(i / w))).collect()
        };
        let mut corners = blob(0, 0);
        corners.iter_mut().zip(blob(w.saturating_sub(3), h.saturating_sub(3))).for_each(|(a, b)| *a |= b);
        vec![
            ("empty", vec![false; w * h]),
            ("centre", single(cx, cy)),
            ("top-left", single(0, 0)),
            ("top-right", single(mx, 0)),
            ("bottom-left", single(0, my)),
            ("bottom-right", single(mx, my)),
            ("top edge", single(cx, 0)),
            ("bottom edge", single(cx, my)),
            ("left edge", single(0, cy)),
            ("right edge", single(mx, cy)),
            ("sparse", rng.mask(w * h, 5)),
            ("dense", rng.mask(w * h, 700)),
            ("full", vec![true; w * h]),
            ("two corners", two),
            ("two blobs", corners),
            ("edge blob", blob(0, cy.saturating_sub(1))),
            ("horizontal line", (0..w * h).map(|i| i / w == cy).collect()),
            ("vertical line", (0..w * h).map(|i| i % w == cx).collect()),
        ]
    }

    #[test]
    fn named_cases_match_the_oracle() {
        for (w, h) in [(64, 48), (1, 97), (97, 1), (7, 3)] {
            for (what, mask) in cases(w, h) {
                for r in [1, 2, 3, 5, 16, 200] {
                    check(w, h, &mask, r, what);
                }
            }
        }
    }

    /// More set cells than `bounds` keeps (`KEEP`): it drops its cell list, and the direct path
    /// rescans the box instead.
    #[test]
    fn a_mask_with_more_cells_than_are_kept_matches_the_oracle() {
        let (w, h) = (200, 200);
        let mask = Rng(0x2545_f491_4f6c_dd1d).mask(w * h, 130);
        let Some(b) = bounds(w, &mask) else { panic!("cells are set") };
        assert!(b.count > KEEP && b.cells.is_empty(), "{} cells, {} kept", b.count, b.cells.len());
        check(w, h, &mask, 1, "more than KEEP");
    }

    #[test]
    fn the_remove_tool_dilate_is_unchanged() {
        for (w, h) in [(64, 48), (1, 97), (7, 3)] {
            for (what, mask) in cases(w, h) {
                for r in [1, 3, 16] {
                    assert!(crate::remove::dilate(w, h, &mask, r) == reference(w, h, &mask, r), "{what} {w}x{h} r={r}");
                }
                assert!(crate::remove::dilate(w, h, &mask, 0) == mask, "{what}: r = 0");
            }
        }
        assert_eq!(dilate_square(4, 4, &[true; 5], 2), vec![true; 5], "a mask of the wrong size");
        assert_eq!(dilate_square(0, 0, &[], 2), Vec::<bool>::new());
    }

    #[test]
    fn a_small_or_sparse_mask_takes_the_direct_path_however_big_the_grid() {
        let (w, h) = (600, 400);
        let mut one = vec![false; w * h];
        one[200 * w + 300] = true;
        let Some(b) = bounds(w, &one) else { panic!("one cell is set") };
        assert_eq!(choose(w, h, &b, 3), Path::Direct);
        let mut two = vec![false; w * h];
        two[0] = true;
        two[w * h - 1] = true;
        let Some(b) = bounds(w, &two) else { panic!("two cells are set") };
        assert_eq!((b.x0, b.y0, b.x1, b.y1), (0, 0, w - 1, h - 1), "the box is the whole grid");
        assert_eq!(choose(w, h, &b, 3), Path::Direct);
    }

    #[test]
    fn a_bulky_mask_takes_the_separable_path_with_bounded_scratch() {
        let (w, h) = (240, 240);
        let mut rng = Rng(7);
        let mut mask = vec![false; w * h];
        for y in 20..220 {
            for x in 20..220 {
                mask[y * w + x] = rng.next().is_multiple_of(2);
            }
        }
        let Some(b) = bounds(w, &mask) else { panic!("the patch has set cells") };
        assert_eq!(choose(w, h, &b, 3), Path::Separable);
        let (ex0, ex1, ey0, ey1) = expanded(w, h, &b, 3);
        let scratch = scratch_cells(w, h, &b, 3);
        assert!(scratch <= (ex1 - ex0 + 1) * (ey1 - ey0 + 1), "scratch stays inside the expanded box");
        assert!(scratch < w * h, "and is smaller than the grid when the mask is inside it");
        let mut out = vec![false; w * h];
        assert_eq!(separable(w, h, &mask, 3, &b, &mut out), scratch);
    }

    #[test]
    fn huge_radii_do_not_overflow() {
        let mask = [false, true, false, false];
        assert_eq!(dilate_square(2, 2, &mask, usize::MAX), vec![true; 4]);
        assert_eq!(forced(2, 2, &mask, usize::MAX, &Path::Separable), vec![true; 4]);
    }
}
