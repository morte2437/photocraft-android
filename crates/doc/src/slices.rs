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

/// The visible part of every stored slice (later slices cover earlier ones, as in Photoshop),
/// clipped to the canvas: a covered slice is cut into the rectangles that stay visible, each
/// paired with the slice's index in the list.
fn visible(doc: &Document) -> Vec<(usize, Rect)> {
    let canvas = doc.bounds();
    let rects: Vec<Rect> = doc.slices.list.iter().map(|s| s.rect.intersect(&canvas)).collect();
    let mut out = Vec::new();
    for (i, r) in rects.iter().enumerate() {
        let above = rects.get(i + 1..).unwrap_or_default();
        out.extend(auto_slices(*r, above).into_iter().map(|p| (i, p)));
    }
    out
}

/// Rectangles that tile `canvas` minus `taken`. The canvas is cut into a grid along every slice
/// edge; free cells merge into horizontal runs per row band, then runs with identical columns
/// merge down, which yields the few large auto slices Photoshop shows.
pub fn auto_slices(canvas: Rect, taken: &[Rect]) -> Vec<Rect> {
    if canvas.is_empty() {
        return Vec::new();
    }
    let mut xs = vec![canvas.x0, canvas.x1];
    let mut ys = vec![canvas.y0, canvas.y1];
    for r in taken {
        let r = r.intersect(&canvas);
        if r.is_empty() {
            continue;
        }
        xs.extend([r.x0, r.x1]);
        ys.extend([r.y0, r.y1]);
    }
    xs.sort_unstable();
    xs.dedup();
    ys.sort_unstable();
    ys.dedup();
    // Clip the taken rectangles to the canvas once, so band coverage only considers rects that
    // can actually cover a cell.
    let clipped: Vec<Rect> = taken.iter().map(|r| r.intersect(&canvas)).filter(|r| !r.is_empty()).collect();
    // Map an x edge to its cell index in `xs` (xs is sorted/deduped).
    let x_cell = |x: i32| xs.partition_point(|&e| e <= x).saturating_sub(1);
    // Horizontal runs per row band. For each band, mark covered x-cells in a single pass over
    // the rects intersecting that band, instead of scanning every rect for every cell (which
    // was cubic in the slice count and let a crafted document stall canvas drawing).
    let ncells = xs.len().saturating_sub(1);
    let mut runs: Vec<Rect> = Vec::new();
    for yw in ys.windows(2) {
        let (band_y0, band_y1) = (yw[0], yw[1]);
        let mut covered_cell = vec![false; ncells];
        for r in &clipped {
            if r.y0 <= band_y0 && band_y1 <= r.y1 {
                // Rect spans this whole band vertically; mark the x-cells it covers.
                let (c0, c1) = (x_cell(r.x0), x_cell(r.x1));
                for c in covered_cell.iter_mut().take(c1).skip(c0) {
                    *c = true;
                }
            }
        }
        let mut start: Option<i32> = None;
        for (ci, xw) in xs.windows(2).enumerate() {
            let free = !covered_cell.get(ci).copied().unwrap_or(false);
            match (free, start) {
                (true, None) => start = Some(xw[0]),
                (false, Some(s)) => {
                    runs.push(Rect::new(s, band_y0, xw[0], band_y1));
                    start = None;
                }
                _ => {}
            }
        }
        if let Some(s) = start {
            runs.push(Rect::new(s, band_y0, *xs.last().unwrap_or(&canvas.x1), band_y1));
        }
    }
    // Merge vertically adjacent runs with the same columns.
    // `open` finds the slice a run continues by its columns and bottom edge: searching the output
    // for each run is quadratic in the runs, and bars crossing in a grid leave a million of them.
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
    let stored = visible(doc);
    let rects: Vec<Rect> = stored.iter().map(|(_, r)| *r).collect();
    let mut all: Vec<(Option<usize>, Rect)> = stored.iter().map(|(i, r)| (Some(*i), *r)).collect();
    all.extend(auto_slices(doc.bounds(), &rects).into_iter().map(|r| (None, r)));
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

    #[test]
    fn exhausted_slice_id_survives_serialization() {
        let slices = Slices { list: vec![Slice { id: u32::MAX, ..Default::default() }], ..Default::default() };
        let value = serde_json::to_value(&slices).unwrap();
        let loaded: Slices = serde_json::from_value(value).unwrap();
        assert_eq!(loaded.next_id(), None);
    }
}
