//! Web slices (Slice tool, Layer › New Layer Based Slice, Save for Web): pure data plus the
//! auto-slice partition and Photoshop-style numbering.
//!
//! A document keeps its *user* slices (drawn with the Slice tool) and *layer-based* slices (they
//! follow a layer's bounds, effects included). *Auto* slices are never stored: they are the
//! rectangles that fill the rest of the canvas, recomputed whenever they are needed, exactly like
//! Photoshop regenerates them after every slice edit.

use photocraft_geom::Rect;
use serde::{Deserialize, Serialize};

use crate::{Document, LayerId};

/// Where a slice comes from (PSD `ESliceOrigin`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SliceOrigin {
    /// Fills the canvas around user and layer slices; regenerated, never edited.
    Auto,
    /// Follows a layer's content bounds (Layer › New Layer Based Slice).
    Layer,
    /// Drawn with the Slice tool (or promoted from an auto slice).
    #[default]
    User,
}

impl SliceOrigin {
    /// PSD resource 1050 code: 0 auto, 1 layer, 2 user.
    pub fn code(self) -> u32 {
        match self {
            SliceOrigin::Auto => 0,
            SliceOrigin::Layer => 1,
            SliceOrigin::User => 2,
        }
    }
    pub fn from_code(c: u32) -> Self {
        match c {
            0 => SliceOrigin::Auto,
            1 => SliceOrigin::Layer,
            _ => SliceOrigin::User,
        }
    }
    pub fn id(self) -> &'static str {
        match self {
            SliceOrigin::Auto => "auto",
            SliceOrigin::Layer => "layer",
            SliceOrigin::User => "user",
        }
    }
}

/// Slice Options › Slice Type.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SliceKind {
    /// Exported as an image file.
    #[default]
    Image,
    /// An HTML cell with text (or nothing); no image is written.
    NoImage,
    /// A nested table (ImageReady legacy; exported like Image).
    Table,
}

impl SliceKind {
    /// PSD resource 1050 code: 0 no image, 1 image, 2 table.
    pub fn code(self) -> u32 {
        match self {
            SliceKind::NoImage => 0,
            SliceKind::Image => 1,
            SliceKind::Table => 2,
        }
    }
    pub fn from_code(c: u32) -> Self {
        match c {
            0 => SliceKind::NoImage,
            2 => SliceKind::Table,
            _ => SliceKind::Image,
        }
    }
    pub fn id(self) -> &'static str {
        match self {
            SliceKind::Image => "image",
            SliceKind::NoImage => "noImage",
            SliceKind::Table => "table",
        }
    }
    pub fn from_id(s: &str) -> Option<Self> {
        Some(match s {
            "image" | "img" => SliceKind::Image,
            "noImage" | "none" | "no-image" => SliceKind::NoImage,
            "table" => SliceKind::Table,
            _ => return None,
        })
    }
}

/// One slice and its Slice Options.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Slice {
    /// Stable id (PSD `sliceID`), unique within the document.
    pub id: u32,
    pub group_id: u32,
    pub origin: SliceOrigin,
    /// The layer a layer-based slice follows.
    pub layer: Option<LayerId>,
    /// Slice Options › Name (empty = the generated `<document>_<NN>`).
    pub name: String,
    /// Document pixels. Layer-based slices: the layer's bounds (with effects) plus `outsets`.
    pub rect: Rect,
    pub kind: SliceKind,
    pub url: String,
    pub target: String,
    pub message: String,
    pub alt: String,
    pub cell_text_is_html: bool,
    pub cell_text: String,
    /// HTML alignment of no-image cells (0 default, 1 left, 2 centre, 3 right / top, middle, …).
    pub horizontal_align: u32,
    pub vertical_align: u32,
    /// Slice background (ARGB, 8-bit); None = none.
    pub background: Option<[u8; 4]>,
    /// Layer-based slices: extra margin [top, left, bottom, right].
    pub outsets: [i32; 4],
}

/// The document's slices (PSD resource 1050).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Slices {
    /// Group name (PSD: the base name, usually the document name).
    pub group_name: String,
    /// User and layer-based slices, in creation order (later slices are on top).
    pub list: Vec<Slice>,
}

impl Slices {
    pub fn is_empty(&self) -> bool {
        self.list.is_empty()
    }
    /// A fresh slice id (Photoshop counts from 1; auto slices use ids too, so leave room).
    /// Returns `None` when the stored ids have exhausted the `u32` id space.
    pub fn next_id(&self) -> Option<u32> {
        self.list.iter().map(|s| s.id).max().unwrap_or(0).checked_add(1)
    }
    pub fn get(&self, id: u32) -> Option<&Slice> {
        self.list.iter().find(|s| s.id == id)
    }
    pub fn get_mut(&mut self, id: u32) -> Option<&mut Slice> {
        self.list.iter_mut().find(|s| s.id == id)
    }
}

/// A slice as shown and exported: user/layer slices plus generated auto slices, numbered the way
/// Photoshop numbers them (top-to-bottom, left-to-right, from 1).
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedSlice {
    /// 1-based display number.
    pub number: usize,
    /// The stored slice (None for auto slices).
    pub id: Option<u32>,
    pub origin: SliceOrigin,
    pub rect: Rect,
    /// The slice's name, or the generated `<base>_<NN>`.
    pub name: String,
    pub kind: SliceKind,
}

/// The most stored slices [`resolve`] considers; later ones in the list are left out. A Divide
/// Slice makes at most 10 000 parts, and Photoshop documents hold a few hundred at most, but a
/// file's slice resource is only bounded by its size. The partition below is quadratic in the
/// slice count for nested rectangles, so this keeps a hostile list to well under a second of work.
pub const MAX_RESOLVED_SLICES: usize = 10_000;

/// A warning naming how many stored slices [`resolve`] leaves out (`None` when it considers all).
pub fn overflow_warning(doc: &Document) -> Option<String> {
    let n = doc.slices.list.len().checked_sub(MAX_RESOLVED_SLICES).filter(|n| *n > 0)?;
    Some(format!("{n} stored slices past the first {MAX_RESOLVED_SLICES} were ignored"))
}

/// Rectangles that tile `canvas` minus `taken`. The canvas is cut into a grid along every slice
/// edge; free cells merge into horizontal runs per row band, then runs with identical columns
/// merge down, which yields the few large auto slices Photoshop shows.
pub fn auto_slices(canvas: Rect, taken: &[Rect]) -> Vec<Rect> {
    partition(canvas, taken, false).1
}

/// Splits `canvas` by `rects` (later rects on top) in one sweep: for each rect, the pieces of it
/// that no later rect covers, and the pieces that no rect covers.
///
/// The canvas is cut into a grid along every rect edge. Each row band is painted from the topmost
/// rect down, skipping cells already painted (a next-free-cell index), so every cell is written
/// once per band. Consecutive cells with the same owner make a horizontal run, and runs with the
/// same owner and columns merge down. Resolving each rect against the rects above it separately
/// rebuilt that grid once per rect and re-marked shared overlaps every time, which was quartic in
/// the slice count for nested rectangles and let a crafted document stall canvas drawing.
fn partition(canvas: Rect, rects: &[Rect], want_owned: bool) -> (Vec<Vec<Rect>>, Vec<Rect>) {
    let mut owned_runs: Vec<Vec<Rect>> = vec![Vec::new(); rects.len()];
    let mut free_runs: Vec<Rect> = Vec::new();
    if canvas.is_empty() {
        return (owned_runs, free_runs);
    }
    let clipped: Vec<Rect> = rects.iter().map(|r| r.intersect(&canvas)).collect();
    let mut xs = vec![canvas.x0, canvas.x1];
    let mut ys = vec![canvas.y0, canvas.y1];
    for r in clipped.iter().filter(|r| !r.is_empty()) {
        xs.extend([r.x0, r.x1]);
        ys.extend([r.y0, r.y1]);
    }
    xs.sort_unstable();
    xs.dedup();
    ys.sort_unstable();
    ys.dedup();
    // Every clipped edge is in `xs`/`ys`, so these are exact grid indices.
    let x_edge = |x: i32| xs.partition_point(|&e| e < x);
    let y_edge = |y: i32| ys.partition_point(|&e| e < y);
    let nbands = ys.len().saturating_sub(1);
    let ncells = xs.len().saturating_sub(1);
    // Rects entering and leaving the sweep at each band edge.
    let mut enter: Vec<Vec<usize>> = vec![Vec::new(); ys.len()];
    let mut leave: Vec<Vec<usize>> = vec![Vec::new(); ys.len()];
    let mut cols: Vec<(usize, usize)> = vec![(0, 0); rects.len()];
    for (i, r) in clipped.iter().enumerate().filter(|(_, r)| !r.is_empty()) {
        if let (Some(e), Some(l), Some(c)) = (enter.get_mut(y_edge(r.y0)), leave.get_mut(y_edge(r.y1)), cols.get_mut(i)) {
            e.push(i);
            l.push(i);
            *c = (x_edge(r.x0), x_edge(r.x1));
        }
    }
    let mut active: std::collections::BTreeSet<usize> = std::collections::BTreeSet::new();
    let mut owner: Vec<usize> = vec![usize::MAX; ncells];
    // next[c]: the first unpainted cell at or after c (ncells when none).
    let mut next: Vec<usize> = (0..=ncells).collect();
    for b in 0..nbands {
        for i in leave.get(b).into_iter().flatten() {
            active.remove(i);
        }
        active.extend(enter.get(b).into_iter().flatten().copied());
        let (Some(&band_y0), Some(&band_y1)) = (ys.get(b), ys.get(b + 1)) else { break };
        owner.fill(usize::MAX);
        for (c, n) in next.iter_mut().enumerate() {
            *n = c;
        }
        for &i in active.iter().rev() {
            let (c0, c1) = cols.get(i).copied().unwrap_or((0, 0));
            let mut c = find_free(&mut next, c0);
            while c < c1 {
                if let Some(o) = owner.get_mut(c) {
                    *o = i;
                }
                if let Some(n) = next.get_mut(c) {
                    *n = c + 1;
                }
                c = find_free(&mut next, c + 1);
            }
        }
        let mut c = 0;
        while c < ncells {
            let who = owner.get(c).copied().unwrap_or(usize::MAX);
            let mut end = c + 1;
            while end < ncells && owner.get(end).copied() == Some(who) {
                end += 1;
            }
            if let (Some(&x0), Some(&x1)) = (xs.get(c), xs.get(end)) {
                let run = Rect::new(x0, band_y0, x1, band_y1);
                match owned_runs.get_mut(who) {
                    Some(runs) => {
                        if want_owned {
                            runs.push(run);
                        }
                    }
                    None => free_runs.push(run),
                }
            }
            c = end;
        }
    }
    (owned_runs.into_iter().map(merge_down).collect(), merge_down(free_runs))
}

/// The first unpainted cell at or after `c`, compressing the path it walks.
fn find_free(next: &mut [usize], c: usize) -> usize {
    let mut root = c;
    while let Some(&n) = next.get(root) {
        if n == root {
            break;
        }
        root = n;
    }
    let mut c = c;
    while c != root {
        let Some(slot) = next.get_mut(c) else { break };
        c = std::mem::replace(slot, root);
    }
    root
}

/// Merges vertically adjacent runs (given band by band, top to bottom) with the same columns.
/// `open` finds the slice a run continues by its columns and bottom edge: searching the output
/// for each run is quadratic in the runs, and bars crossing in a grid leave a million of them.
fn merge_down(runs: Vec<Rect>) -> Vec<Rect> {
    let mut out: Vec<Rect> = Vec::new();
    let mut open: std::collections::HashMap<(i32, i32, i32), usize> = std::collections::HashMap::new();
    for r in runs {
        match open.remove(&(r.x0, r.x1, r.y0)) {
            Some(i) => {
                if let Some(prev) = out.get_mut(i) {
                    prev.y1 = r.y1;
                }
                open.insert((r.x0, r.x1, r.y1), i);
            }
            None => {
                open.insert((r.x0, r.x1, r.y1), out.len());
                out.push(r);
            }
        }
    }
    out
}

/// Every slice of `doc` in display order (see [`ResolvedSlice`]). Without user or layer slices
/// the whole canvas is one auto slice named `<base>` alone (as Photoshop's Save for Web names
/// it); with several, generated names are numbered `<base>_NN`. A slice covered by later ones
/// appears once per visible piece: the first piece keeps its name, the others are `<name>-NN`.
pub fn resolve(doc: &Document) -> Vec<ResolvedSlice> {
    let base = base_name(doc);
    let rects: Vec<Rect> = doc.slices.list.iter().take(MAX_RESOLVED_SLICES).map(|s| s.rect).collect();
    let (owned, free) = partition(doc.bounds(), &rects, true);
    let mut all: Vec<(Option<usize>, Rect)> = owned.into_iter().enumerate().flat_map(|(i, pieces)| pieces.into_iter().map(move |p| (Some(i), p))).collect();
    all.extend(free.into_iter().map(|r| (None, r)));
    all.sort_by_key(|(_, r)| (r.y0, r.x0));
    let single = all.len() == 1;
    let mut named = vec![false; doc.slices.list.len()];
    all.into_iter()
        .enumerate()
        .map(|(n, (i, rect))| {
            let s = i.and_then(|i| doc.slices.list.get(i));
            let number = n + 1;
            let first = i.and_then(|i| named.get_mut(i)).is_none_or(|seen| !std::mem::replace(seen, true));
            let name = match s.map(|s| s.name.as_str()).filter(|n| !n.is_empty()) {
                Some(n) if first => n.to_string(),
                Some(n) => format!("{n}-{number:02}"),
                None if single => base.clone(),
                None => format!("{base}_{number:02}"),
            };
            let (id, origin, kind) = s.map_or((None, SliceOrigin::Auto, SliceKind::Image), |s| (Some(s.id), s.origin, s.kind));
            ResolvedSlice { number, id, origin, rect, name, kind }
        })
        .collect()
}

/// The base of generated slice names: the document name without extension, file-name safe.
pub fn base_name(doc: &Document) -> String {
    let n = doc.name.rsplit_once('.').map_or(doc.name.as_str(), |(a, _)| a);
    let s: String = n.chars().map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect();
    if s.is_empty() { "Untitled".into() } else { s }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ColorMode, SampleType, Size};

    fn area(rs: &[Rect]) -> i64 {
        rs.iter().map(|r| i64::from(r.width()) * i64::from(r.height())).sum()
    }

    #[test]
    fn auto_slices_tile_the_rest_of_the_canvas() {
        let canvas = Rect::new(0, 0, 100, 80);
        let user = [Rect::new(20, 10, 60, 30)];
        let auto = auto_slices(canvas, &user);
        assert_eq!(area(&auto) + area(&user), 100 * 80);
        for (i, a) in auto.iter().enumerate() {
            assert!(a.intersect(&user[0]).is_empty());
            for b in &auto[i + 1..] {
                assert!(a.intersect(b).is_empty(), "{a:?} overlaps {b:?}");
            }
        }
        // Above band, left, right, below band.
        assert_eq!(auto.len(), 4);
        assert_eq!(auto[0], Rect::new(0, 0, 100, 10));
        assert_eq!(auto_slices(canvas, &[]), vec![canvas]);
    }

    #[test]
    fn auto_slices_cover_exactly_with_many_overlapping_slices() {
        // Several overlapping slices form a non-trivial grid. The per-band coverage pass must
        // still tile the complement exactly: auto area + taken-union area == canvas, with no
        // auto slice overlapping a taken rect or another auto slice.
        let canvas = Rect::new(0, 0, 200, 150);
        let taken = [Rect::new(10, 10, 90, 70), Rect::new(50, 40, 160, 120), Rect::new(120, 0, 200, 50), Rect::new(0, 100, 60, 150)];
        let auto = auto_slices(canvas, &taken);
        // No auto slice intersects any taken rect.
        for a in &auto {
            for t in &taken {
                assert!(a.intersect(t).is_empty(), "{a:?} overlaps taken {t:?}");
            }
        }
        // Auto slices do not overlap each other.
        for (i, a) in auto.iter().enumerate() {
            for b in &auto[i + 1..] {
                assert!(a.intersect(b).is_empty(), "{a:?} overlaps {b:?}");
            }
        }
        // Auto area equals the canvas minus the union of taken rects (computed on the same grid).
        let union_cells: i64 = {
            // Sum cell areas covered by at least one taken rect, over the fine grid.
            let mut xs = vec![canvas.x0, canvas.x1];
            let mut ys = vec![canvas.y0, canvas.y1];
            for t in &taken {
                xs.extend([t.x0, t.x1]);
                ys.extend([t.y0, t.y1]);
            }
            xs.sort_unstable();
            xs.dedup();
            ys.sort_unstable();
            ys.dedup();
            let mut sum = 0i64;
            for yw in ys.windows(2) {
                for xw in xs.windows(2) {
                    let covered = taken.iter().any(|r| xw[0] >= r.x0 && xw[0] < r.x1 && yw[0] >= r.y0 && yw[0] < r.y1);
                    if covered {
                        sum += i64::from(xw[1] - xw[0]) * i64::from(yw[1] - yw[0]);
                    }
                }
            }
            sum
        };
        assert_eq!(area(&auto) + union_cells, i64::from(canvas.width()) * i64::from(canvas.height()));
    }

    #[test]
    fn resolve_numbers_top_to_bottom() {
        let mut d = Document::new("site.psd", Size::new(100, 80), ColorMode::Rgb, SampleType::U8);
        assert_eq!(resolve(&d).len(), 1);
        assert_eq!(resolve(&d)[0].name, "site");
        d.slices.list.push(Slice { id: 1, rect: Rect::new(20, 10, 60, 30), ..Default::default() });
        let r = resolve(&d);
        assert_eq!(r.len(), 5);
        assert_eq!(r[0].name, "site_01");
        let user = r.iter().find(|s| s.id == Some(1)).unwrap();
        assert_eq!(user.number, 3, "after the top band and the left cell");
        assert_eq!(user.origin, SliceOrigin::User);
        d.slices.list[0].name = "logo".into();
        assert!(resolve(&d).iter().any(|s| s.name == "logo"));
    }

    #[test]
    fn later_slices_cover_earlier_ones() {
        // Photoshop's Save for Web on this document writes sliceA [0,0,60,40], o82_02, sliceA-03
        // [0,40,40,60], sliceB and o82_05: the covered slice is cut into its visible pieces.
        let mut d = Document::new("o82.psd", Size::new(100, 100), ColorMode::Rgb, SampleType::U8);
        d.slices.list.push(Slice { id: 1, name: "sliceA".into(), rect: Rect::new(0, 0, 60, 60), ..Default::default() });
        d.slices.list.push(Slice { id: 2, name: "sliceB".into(), rect: Rect::new(40, 40, 100, 100), ..Default::default() });
        let r = resolve(&d);
        let got: Vec<(&str, Option<u32>, Rect)> = r.iter().map(|s| (s.name.as_str(), s.id, s.rect)).collect();
        assert_eq!(
            got,
            [
                ("sliceA", Some(1), Rect::new(0, 0, 60, 40)),
                ("o82_02", None, Rect::new(60, 0, 100, 40)),
                ("sliceA-03", Some(1), Rect::new(0, 40, 40, 60)),
                ("sliceB", Some(2), Rect::new(40, 40, 100, 100)),
                ("o82_05", None, Rect::new(0, 60, 40, 100)),
            ]
        );
        d.slices.list[0].name.clear();
        let names: Vec<String> = resolve(&d).into_iter().filter(|s| s.id == Some(1)).map(|s| s.name).collect();
        assert_eq!(names, ["o82_01", "o82_03"]);
    }

    /// The earlier resolver: each stored slice cut against the slices above it on its own grid,
    /// then the auto slices cut against the visible pieces.
    fn reference_resolve(canvas: Rect, rects: &[Rect]) -> (Vec<(usize, Rect)>, Vec<Rect>) {
        fn cut(canvas: Rect, taken: &[Rect]) -> Vec<Rect> {
            if canvas.is_empty() {
                return Vec::new();
            }
            let clipped: Vec<Rect> = taken.iter().map(|r| r.intersect(&canvas)).filter(|r| !r.is_empty()).collect();
            let mut xs = vec![canvas.x0, canvas.x1];
            let mut ys = vec![canvas.y0, canvas.y1];
            for r in &clipped {
                xs.extend([r.x0, r.x1]);
                ys.extend([r.y0, r.y1]);
            }
            xs.sort_unstable();
            xs.dedup();
            ys.sort_unstable();
            ys.dedup();
            let mut runs = Vec::new();
            for yw in ys.windows(2) {
                let mut start = None;
                for xw in xs.windows(2) {
                    let free = !clipped.iter().any(|r| r.x0 <= xw[0] && xw[1] <= r.x1 && r.y0 <= yw[0] && yw[1] <= r.y1);
                    match (free, start) {
                        (true, None) => start = Some(xw[0]),
                        (false, Some(s)) => {
                            runs.push(Rect::new(s, yw[0], xw[0], yw[1]));
                            start = None;
                        }
                        _ => {}
                    }
                }
                if let Some(s) = start {
                    runs.push(Rect::new(s, yw[0], canvas.x1, yw[1]));
                }
            }
            merge_down(runs)
        }
        let clipped: Vec<Rect> = rects.iter().map(|r| r.intersect(&canvas)).collect();
        let mut stored = Vec::new();
        for (i, r) in clipped.iter().enumerate() {
            stored.extend(cut(*r, &clipped[i + 1..]).into_iter().map(|p| (i, p)));
        }
        let pieces: Vec<Rect> = stored.iter().map(|(_, r)| *r).collect();
        let free = cut(canvas, &pieces);
        (stored, free)
    }

    #[test]
    fn sweep_partition_matches_per_slice_resolution() {
        // A small deterministic generator: overlapping, nested, touching and off-canvas rects.
        let mut seed = 0x2545_f491_u32;
        let mut next = |m: i32| {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            (seed % (m as u32)) as i32
        };
        let canvas = Rect::new(0, 0, 120, 90);
        for round in 0..200 {
            let n = 1 + round % 9;
            let rects: Vec<Rect> = (0..n)
                .map(|_| {
                    let (x, y) = (next(140) - 10, next(110) - 10);
                    Rect::new(x, y, x + next(70), y + next(60))
                })
                .collect();
            let (owned, free) = partition(canvas, &rects, true);
            let got: Vec<(usize, Rect)> = owned.into_iter().enumerate().flat_map(|(i, ps)| ps.into_iter().map(move |p| (i, p))).collect();
            let (want, want_free) = reference_resolve(canvas, &rects);
            assert_eq!(got, want, "pieces for {rects:?}");
            assert_eq!(free, want_free, "auto slices for {rects:?}");
        }
    }

    #[test]
    fn nested_slices_resolve_in_bounded_work() {
        // Nested rectangles were the quartic case: every slice re-marked every later overlap on
        // its own grid. Each ring here stays visible as four bars around the next slice in.
        let n = 2_000;
        let mut d = Document::new("nest.psd", Size::new(10_000, 10_000), ColorMode::Rgb, SampleType::U8);
        d.slices.list = (0..n).map(|i| Slice { id: i as u32 + 1, rect: Rect::new(i, i, 10_000 - i, 10_000 - i), ..Default::default() }).collect();
        let r = resolve(&d);
        let stored = r.iter().filter(|s| s.id.is_some()).count();
        assert_eq!(stored, 4 * (n as usize - 1) + 1);
        assert_eq!(r.iter().map(|s| i64::from(s.rect.width()) * i64::from(s.rect.height())).sum::<i64>(), 10_000 * 10_000);
    }

    #[test]
    fn slices_past_the_limit_are_left_out() {
        let mut d = Document::new("many.psd", Size::new(100, 100), ColorMode::Rgb, SampleType::U8);
        d.slices.list = (0..MAX_RESOLVED_SLICES + 5).map(|i| Slice { id: i as u32 + 1, rect: Rect::new(0, 0, 10, 10), ..Default::default() }).collect();
        let r = resolve(&d);
        let ids: Vec<u32> = r.iter().filter_map(|s| s.id).collect();
        assert_eq!(ids, [MAX_RESOLVED_SLICES as u32], "the topmost considered slice covers the others");
        assert!(overflow_warning(&d).unwrap().starts_with("5 stored slices"));
        d.slices.list.truncate(MAX_RESOLVED_SLICES);
        assert_eq!(overflow_warning(&d), None);
    }

    #[test]
    fn exhausted_slice_id_survives_serialization() {
        let slices = Slices { list: vec![Slice { id: u32::MAX, ..Default::default() }], ..Default::default() };
        let value = serde_json::to_value(&slices).unwrap();
        let loaded: Slices = serde_json::from_value(value).unwrap();
        assert_eq!(loaded.next_id(), None);
    }
}
