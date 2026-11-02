//! Select and Mask's own selection tools (#7069, #8148): the edits the workspace makes to the
//! selection before it refines the edge, carried as `select.refineEdge`'s `edits` param.
//!
//! Photoshop's Select and Mask has a Quick Selection tool, a Brush, Object Selection, Lasso and
//! Polygonal Lasso, a Select Subject button and Clear Selection / Invert. Here each use is one
//! [`SelectionEdit`], applied in order to the selection (or layer mask) Select and Mask starts
//! from. Keeping them as params rather than live edits of the document means Cancel discards
//! them, OK is one history step, and the UI, CLI, MCP and the control channel all drive them the
//! same way. [`apply`] is pure, so the workspace's preview runs exactly what OK commits.

use std::collections::BTreeSet;

use photocraft_algo::matting;
use photocraft_algo::segment::{grabcut, quick, subject};
use photocraft_algo::selection::{self as sel, Region, SelectionMode, combine_region};
use photocraft_doc::{Document, LayerId};
use photocraft_geom::Rect;
use photocraft_raster::Surface;
use serde_json::Value;

use crate::smartselect_cmds::with_doc_sampler;
use crate::{EngineError, Result};

const CMD: &str = "select.refineEdge";

/// Most edits one Select and Mask takes, and points across them.
pub const MAX_EDITS: usize = 4096;
pub const MAX_EDIT_POINTS: usize = 100_000;

/// One use of a Select and Mask tool. `add: false` is the tool's subtract mode (Alt).
#[derive(Clone, Debug, PartialEq)]
pub enum SelectionEdit {
    /// Quick Selection: grows a region from the stroke out to the image's edges.
    Quick { points: Vec<(f32, f32)>, size: f32, add: bool, sample_all_layers: bool },
    /// Brush: paints the selection directly with a round brush.
    Brush { points: Vec<(f32, f32)>, size: f32, hardness: f32, add: bool },
    /// Lasso and Polygonal Lasso: the polygon through `points`.
    Lasso { points: Vec<(f32, f32)>, add: bool },
    /// Object Selection: the object in `rect` [x, y, w, h], or the one under `point`.
    Object { area: ObjectArea, add: bool, sample_all_layers: bool },
    /// The Select Subject button.
    Subject { mode: SelectionMode, sample_all_layers: bool },
    /// Global Refinements › Clear Selection.
    Clear,
    /// Global Refinements › Invert.
    Invert,
}

/// Where Object Selection looks.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ObjectArea {
    Rect(Rect),
    Point(f32, f32),
}

fn bad(msg: &str) -> EngineError {
    EngineError::BadParams { cmd: CMD.into(), msg: msg.into() }
}

/// A finite coordinate within the range strokes may use.
fn coord(v: &Value) -> Option<f32> {
    v.as_f64().filter(|v| v.is_finite() && v.abs() <= 1e7).map(|v| v as f32)
}

fn point(v: &Value) -> Option<(f32, f32)> {
    let xy = v.as_array().filter(|a| a.len() == 2)?;
    Some((coord(xy.first()?)?, coord(xy.get(1)?)?))
}

fn points(o: &serde_json::Map<String, Value>, min: usize, total: &mut usize) -> Result<Vec<(f32, f32)>> {
    let list = o.get("points").and_then(Value::as_array).ok_or_else(|| bad("this edit needs a points array of [x,y] pairs"))?;
    if list.len() < min {
        return Err(bad(&format!("this edit needs at least {min} point{}", if min == 1 { "" } else { "s" })));
    }
    *total = total.saturating_add(list.len());
    if *total > MAX_EDIT_POINTS {
        return Err(bad(&format!("edits hold more than {MAX_EDIT_POINTS} points")));
    }
    list.iter().map(point).collect::<Option<Vec<_>>>().ok_or_else(|| bad("edit points must be [x,y] pairs of finite numbers"))
}

/// A number in `range`, `default` when absent; anything else is an error naming `key`.
fn number(o: &serde_json::Map<String, Value>, key: &str, default: f32, range: std::ops::RangeInclusive<f32>) -> Result<f32> {
    match o.get(key) {
        None => Ok(default),
        Some(v) => v
            .as_f64()
            .map(|v| v as f32)
            .filter(|v| v.is_finite() && range.contains(v))
            .ok_or_else(|| bad(&format!("edit {key} must be a number in {}..{}", range.start(), range.end()))),
    }
}

fn flag(o: &serde_json::Map<String, Value>, key: &str, default: bool) -> Result<bool> {
    match o.get(key) {
        None => Ok(default),
        Some(v) => v.as_bool().ok_or_else(|| bad(&format!("edit {key} must be true or false"))),
    }
}

/// The tools' `mode`: `add` (the default) or `subtract`.
fn adds(o: &serde_json::Map<String, Value>) -> Result<bool> {
    match o.get("mode").map(Value::as_str) {
        None | Some(Some("add")) => Ok(true),
        Some(Some("subtract")) => Ok(false),
        Some(_) => Err(bad("edit mode must be add|subtract")),
    }
}

/// The `edits` of `p`, in order (none when absent or null).
pub fn parse(p: &Value) -> Result<Vec<SelectionEdit>> {
    let Some(v) = p.get("edits").filter(|v| !v.is_null()) else { return Ok(Vec::new()) };
    let list = v.as_array().ok_or_else(|| bad("edits must be an array of {\"tool\":…} objects"))?;
    if list.len() > MAX_EDITS {
        return Err(bad(&format!("edits holds more than {MAX_EDITS} edits")));
    }
    let size_range = matting::BRUSH_SIZE_RANGE;
    let mut total = 0usize;
    // The area the Brush edits sweep in all, bounded as `refineStrokes`' is.
    let mut swept = 0.0f64;
    let mut edits = Vec::with_capacity(list.len());
    for e in list {
        let o = e.as_object().ok_or_else(|| bad("each edit must be an object with a tool"))?;
        let tool = o.get("tool").and_then(Value::as_str).ok_or_else(|| bad("each edit needs a tool: quick|brush|lasso|object|subject|clear|invert"))?;
        let edit = match tool {
            "quick" => SelectionEdit::Quick {
                points: points(o, 1, &mut total)?,
                size: number(o, "size", 30.0, size_range.clone())?,
                add: adds(o)?,
                sample_all_layers: flag(o, "sampleAllLayers", false)?,
            },
            "brush" => {
                let points = points(o, 1, &mut total)?;
                let size = number(o, "size", 30.0, size_range.clone())?;
                swept += matting::stroke_area(&points, size);
                if swept > matting::MAX_SWEPT_AREA {
                    return Err(bad("brush edits paint too large an area"));
                }
                SelectionEdit::Brush { points, size, hardness: number(o, "hardness", 100.0, 0.0..=100.0)?, add: adds(o)? }
            }
            "lasso" => SelectionEdit::Lasso { points: points(o, 3, &mut total)?, add: adds(o)? },
            "object" => {
                let area = match (o.get("rect"), o.get("point")) {
                    (Some(r), _) => {
                        let v = r.as_array().filter(|a| a.len() == 4).ok_or_else(|| bad("object rect must be [x,y,w,h]"))?;
                        let n = v.iter().map(coord).collect::<Option<Vec<_>>>().ok_or_else(|| bad("object rect must be four finite numbers"))?;
                        let (x, y, w, h) = (n.first().copied(), n.get(1).copied(), n.get(2).copied(), n.get(3).copied());
                        let (Some(x), Some(y), Some(w), Some(h)) = (x, y, w, h) else { return Err(bad("object rect must be [x,y,w,h]")) };
                        // Negative sizes (dragging up or left) are normalised.
                        let (x0, x1, y0, y1) = (x.min(x + w), x.max(x + w), y.min(y + h), y.max(y + h));
                        ObjectArea::Rect(Rect::new(x0.floor() as i32, y0.floor() as i32, x1.ceil() as i32, y1.ceil() as i32))
                    }
                    (None, Some(pt)) => {
                        let (x, y) = point(pt).ok_or_else(|| bad("object point must be [x,y]"))?;
                        ObjectArea::Point(x, y)
                    }
                    (None, None) => return Err(bad("object needs \"rect\": [x,y,w,h] or \"point\": [x,y]")),
                };
                SelectionEdit::Object { area, add: adds(o)?, sample_all_layers: flag(o, "sampleAllLayers", false)? }
            }
            "subject" => {
                let mode = match o.get("mode").map(Value::as_str) {
                    None | Some(Some("replace")) => SelectionMode::Replace,
                    Some(Some("add")) => SelectionMode::Add,
                    Some(Some("subtract")) => SelectionMode::Subtract,
                    Some(_) => return Err(bad("subject mode must be replace|add|subtract")),
                };
                SelectionEdit::Subject { mode, sample_all_layers: flag(o, "sampleAllLayers", true)? }
            }
            "clear" => SelectionEdit::Clear,
            "invert" => SelectionEdit::Invert,
            other => return Err(bad(&format!("unknown edit tool `{other}`: quick|brush|lasso|object|subject|clear|invert"))),
        };
        edits.push(edit);
    }
    Ok(edits)
}

/// Whether `p` carries Select and Mask tool edits.
pub fn has_edits(p: &Value) -> bool {
    p.get("edits").is_some_and(|v| !v.is_null() && v.as_array().is_none_or(|a| !a.is_empty()))
}

fn mode(add: bool) -> SelectionMode {
    if add { SelectionMode::Add } else { SelectionMode::Subtract }
}

/// Most tiles the Brush and Lasso edits may paint in all: four times what one may
/// ([`sel::MAX_PAINTED_TILES`]), 1 GiB of GRAY8 tiles. Each is rasterised a tile at a time, so
/// this, not the canvas size, bounds what they allocate (the #8783 review).
pub const MAX_EDIT_TILES: usize = 4 * sel::MAX_PAINTED_TILES;

/// Refuses Brush and Lasso edits that would paint too many tiles of `canvas`, one by one or in
/// all, before anything is painted.
pub fn check_size(edits: &[SelectionEdit], canvas: Rect) -> Result<()> {
    let mut all = BTreeSet::new();
    for e in edits {
        let tiles = match e {
            SelectionEdit::Brush { points, size, .. } => matting::painted_tiles(points, *size, canvas, sel::MAX_PAINTED_TILES),
            SelectionEdit::Lasso { points, .. } => sel::polygon_edge_tiles(points, canvas, sel::MAX_PAINTED_TILES),
            _ => continue,
        };
        all.extend(tiles.ok_or_else(|| bad("a brush or lasso edit crosses too many tiles of the canvas"))?);
        if all.len() > MAX_EDIT_TILES {
            return Err(bad("brush and lasso edits cross too many tiles of the canvas"));
        }
    }
    Ok(())
}

/// Refuses Refine Edge Brush strokes that would paint more tiles of `canvas` than
/// [`matting::EdgeBrush`] holds.
pub fn check_strokes(strokes: &[matting::EdgeStroke], canvas: Rect) -> Result<()> {
    let refused = || EngineError::BadParams { cmd: CMD.into(), msg: "refineStrokes cross too many tiles of the canvas".into() };
    let mut all = BTreeSet::new();
    for s in strokes {
        all.extend(matting::painted_tiles(&s.points, s.size, canvas, sel::MAX_PAINTED_TILES).ok_or_else(refused)?);
        if all.len() > sel::MAX_PAINTED_TILES {
            return Err(refused());
        }
    }
    Ok(())
}

/// `base` inverted over `canvas`; `None` when that leaves nothing selected. Tile by tile: the
/// canvas takes the inverse of `base`'s untouched value (one tile shared by every fully covered
/// tile, as `fill_rect` stores it), then only the tiles `base` holds are inverted, so neither a
/// sparse selection nor a huge canvas costs a canvas-sized buffer (the #8783 review).
fn invert(base: Option<&Surface>, canvas: Rect) -> Option<Surface> {
    let outside = base.map_or(0.0, |b| b.default_pixel().first().copied().unwrap_or(0.0).clamp(0.0, 1.0));
    let mut s = Surface::new(photocraft_color::PixelFormat::GRAY8);
    if outside < 1.0 {
        s.fill_rect(canvas, &[1.0 - outside]);
    }
    if let Some(b) = base {
        let ch = b.channels().max(1);
        for (tc, _) in b.tiles() {
            let r = tc.rect().intersect(&canvas);
            if r.is_empty() {
                continue;
            }
            let inverted: Vec<f32> = b.read_region(r).chunks_exact(ch).map(|px| 1.0 - px.first().copied().unwrap_or(0.0).clamp(0.0, 1.0)).collect();
            s.write_region(r, &inverted);
        }
    }
    s.prune();
    (!s.content_bounds().is_empty()).then_some(s)
}

/// One edit applied to `base` (the selection so far; `None` = nothing selected) in `doc`, the
/// sampling tools reading `layer` (or the composite with Sample All Layers). A tool that finds
/// nothing leaves the selection as it was, as the same tools do outside Select and Mask.
pub fn apply_one(doc: &Document, layer: Option<LayerId>, base: Option<Surface>, edit: &SelectionEdit) -> Option<Surface> {
    let canvas = doc.bounds();
    let combine = |region: Option<Region>, m: SelectionMode| match region {
        Some(r) => combine_region(base.as_ref(), Some(&r), m),
        None => base.clone(),
    };
    // A painted shape (tile-sparse) added or subtracted, a tile at a time.
    let paint = |shape: Option<Surface>, add: bool| match (shape.filter(|s| s.tile_count() > 0), base.as_ref()) {
        (None, _) => base.clone(),
        (Some(s), None) => add.then_some(s),
        (Some(s), Some(o)) => {
            let out = if add { sel::combine_sparse(o, &s, canvas, f32::max) } else { sel::combine_sparse(o, &s, canvas, |a, b| (a - b).max(0.0)) };
            (!out.content_bounds().is_empty()).then_some(out)
        }
    };
    match edit {
        SelectionEdit::Quick { points, size, add, sample_all_layers } => {
            let region = with_doc_sampler(doc, layer, *sample_all_layers, |smp, doc| quick::quick_select(smp, doc.bounds(), points, *size));
            combine(region, mode(*add))
        }
        SelectionEdit::Brush { points, size, hardness, add } => paint(matting::brush_surface(points, *size, *hardness, canvas), *add),
        SelectionEdit::Lasso { points, add } => paint(sel::polygon_surface(points, canvas), *add),
        SelectionEdit::Object { area, add, sample_all_layers } => {
            let region = with_doc_sampler(doc, layer, *sample_all_layers, |smp, doc| match *area {
                ObjectArea::Rect(r) => grabcut::object_select(smp, doc.bounds(), r, 160_000),
                ObjectArea::Point(x, y) => grabcut::object_at(smp, doc.bounds(), x, y, grabcut::OBJECT_AT_WORK_PX),
            });
            combine(region, mode(*add))
        }
        SelectionEdit::Subject { mode, sample_all_layers } => {
            let region = with_doc_sampler(doc, layer, *sample_all_layers, |smp, doc| subject::select_subject(smp, doc.bounds()));
            combine(region, *mode)
        }
        SelectionEdit::Clear => None,
        SelectionEdit::Invert => invert(base.as_ref(), canvas),
    }
}

/// `edits` applied in order to `base`: the selection Select and Mask refines.
/// A run of Inverts is one Invert or none (pairs cancel exactly), so thousands of them cost at
/// most one pass each between other edits (the #8783 review).
pub fn apply(doc: &Document, layer: Option<LayerId>, base: Option<Surface>, edits: &[SelectionEdit]) -> Option<Surface> {
    let mut sel = base;
    let mut invert = false;
    for e in edits {
        if *e == SelectionEdit::Invert {
            invert = !invert;
            continue;
        }
        if std::mem::take(&mut invert) {
            sel = apply_one(doc, layer, sel, &SelectionEdit::Invert);
        }
        sel = apply_one(doc, layer, sel, e);
    }
    if invert {
        sel = apply_one(doc, layer, sel, &SelectionEdit::Invert);
    }
    sel
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_every_tool() {
        let p = json!({"edits": [
            {"tool": "quick", "points": [[1, 2]], "size": 12, "mode": "subtract", "sampleAllLayers": true},
            {"tool": "brush", "points": [[1, 2], [3, 4]], "hardness": 50},
            {"tool": "lasso", "points": [[0, 0], [10, 0], [0, 10]]},
            {"tool": "object", "rect": [10, 10, -5, -5]},
            {"tool": "object", "point": [3, 4], "mode": "subtract"},
            {"tool": "subject", "mode": "add"},
            {"tool": "clear"},
            {"tool": "invert"},
        ]});
        let edits = parse(&p).expect("valid edits");
        assert_eq!(edits.len(), 8);
        assert_eq!(edits[0], SelectionEdit::Quick { points: vec![(1.0, 2.0)], size: 12.0, add: false, sample_all_layers: true });
        assert_eq!(edits[1], SelectionEdit::Brush { points: vec![(1.0, 2.0), (3.0, 4.0)], size: 30.0, hardness: 50.0, add: true });
        assert_eq!(edits[3], SelectionEdit::Object { area: ObjectArea::Rect(Rect::new(5, 5, 10, 10)), add: true, sample_all_layers: false });
        assert_eq!(edits[5], SelectionEdit::Subject { mode: SelectionMode::Add, sample_all_layers: true });
        assert!(parse(&json!({})).expect("none").is_empty());
        assert!(parse(&json!({"edits": null})).expect("none").is_empty());
        assert!(has_edits(&p) && !has_edits(&json!({"edits": []})) && !has_edits(&json!({})));
        // One long brush edit within the area limit is fine; two of them (rejects_hostile_edits) are not.
        assert!(parse(&json!({"edits": [{"tool": "brush", "points": [[0, 0], [10_000_000, 0]], "size": 20}]})).is_ok());
    }

    #[test]
    fn rejects_hostile_edits() {
        let many_points = vec![json!([1, 1]); MAX_EDIT_POINTS + 1];
        let many_edits = vec![json!({"tool": "clear"}); MAX_EDITS + 1];
        for bad in [
            json!("x"),
            json!([1]),
            json!([{}]),
            json!([{"tool": 7}]),
            json!([{"tool": "wand"}]),
            json!([{"tool": "quick"}]),
            json!([{"tool": "quick", "points": []}]),
            json!([{"tool": "quick", "points": [[1]]}]),
            json!([{"tool": "quick", "points": [["a", 1]]}]),
            json!([{"tool": "quick", "points": [[1e300, 1]]}]),
            json!([{"tool": "quick", "points": [[1, 1]], "size": 0}]),
            json!([{"tool": "quick", "points": [[1, 1]], "size": "big"}]),
            json!([{"tool": "quick", "points": [[1, 1]], "mode": "intersect"}]),
            json!([{"tool": "quick", "points": [[1, 1]], "sampleAllLayers": 1}]),
            json!([{"tool": "brush", "points": [[1, 1]], "hardness": 101}]),
            json!([{"tool": "lasso", "points": [[1, 1], [2, 2]]}]),
            json!([{"tool": "object"}]),
            json!([{"tool": "object", "rect": [1, 2, 3]}]),
            json!([{"tool": "object", "point": [1, "y"]}]),
            json!([{"tool": "subject", "mode": "intersect"}]),
            json!([{"tool": "brush", "points": many_points}]),
            json!(many_edits),
            // Each within the limit, together past it.
            json!(vec![json!({"tool": "brush", "points": [[0, 0], [10_000_000, 0]], "size": 20}); 2]),
        ] {
            assert!(parse(&json!({"edits": bad})).is_err(), "{bad}");
        }
    }

    /// The dense inversion this replaced: the whole canvas read, inverted and written back.
    fn dense_invert(base: Option<&Surface>, canvas: Rect) -> Vec<u8> {
        let m: Vec<f32> = sel::mask_from_surface(base, canvas).into_iter().map(|v| 1.0 - v).collect();
        bytes(Some(&sel::mask_to_surface(&m, canvas)), canvas)
    }

    fn bytes(s: Option<&Surface>, canvas: Rect) -> Vec<u8> {
        sel::mask_from_surface(s, canvas).into_iter().map(|v| (v * 255.0).round() as u8).collect()
    }

    /// Tile by tile, Invert gives exactly what the dense inversion gave: partial edge tiles, soft
    /// values, a selection reaching past the canvas, a mask whose untouched pixels are white, and
    /// nothing selected (everything, inverted).
    #[test]
    fn tiled_invert_matches_the_dense_one() {
        let canvas = Rect::new(0, 0, 600, 333);
        let mut soft = Surface::new(photocraft_color::PixelFormat::GRAY8);
        soft.fill_rect(Rect::new(250, 20, 900, 300), &[1.0]);
        soft.fill_rect(Rect::new(10, 10, 40, 30), &[0.4]);
        let mut white = Surface::with_default(photocraft_color::PixelFormat::GRAY8, &[1.0]);
        white.fill_rect(Rect::new(300, 100, 520, 140), &[0.0]);
        for base in [None, Some(soft), Some(white)] {
            let got = bytes(invert(base.as_ref(), canvas).as_ref(), canvas);
            assert_eq!(got, dense_invert(base.as_ref(), canvas));
        }
        // Inverting everything selects nothing.
        let mut all = Surface::new(photocraft_color::PixelFormat::GRAY8);
        all.fill_rect(canvas, &[1.0]);
        assert!(invert(Some(&all), canvas).is_none());
    }

    /// A huge canvas inverts without a canvas-sized buffer, and a run of Inverts costs one.
    #[test]
    fn invert_on_a_huge_canvas_and_runs_of_inverts() {
        let doc = Document::new("huge", photocraft_doc::Size::new(100_000, 100_000), photocraft_doc::ColorMode::Rgb, photocraft_color::SampleType::U8);
        let mut small = Surface::new(photocraft_color::PixelFormat::GRAY8);
        small.fill_rect(Rect::new(10, 10, 20, 20), &[1.0]);
        let t = std::time::Instant::now();
        let one = apply(&doc, None, Some(small.clone()), &[SelectionEdit::Invert]).expect("almost everything selected");
        assert!(one.sample_channel(15, 15, 0) < 0.01 && one.sample_channel(99_999, 99_999, 0) > 0.99 && one.sample_channel(50_000, 5, 0) > 0.99);
        let odd = apply(&doc, None, Some(small.clone()), &vec![SelectionEdit::Invert; 4095]).expect("inverted");
        assert!(odd.sample_channel(15, 15, 0) < 0.01 && odd.sample_channel(99_999, 99_999, 0) > 0.99);
        let even = apply(&doc, None, Some(small.clone()), &vec![SelectionEdit::Invert; 4096]).expect("the selection itself");
        assert!(even.sample_channel(15, 15, 0) > 0.99 && even.sample_channel(50_000, 50_000, 0) < 0.01);
        // Runs split by another edit invert on each side of it.
        let split = [SelectionEdit::Invert, SelectionEdit::Invert, SelectionEdit::Invert, SelectionEdit::Clear, SelectionEdit::Invert];
        let all = apply(&doc, None, Some(small), &split).expect("cleared, then everything");
        assert!(all.sample_channel(15, 15, 0) > 0.99 && all.sample_channel(70_000, 3, 0) > 0.99);
        assert!(t.elapsed() < std::time::Duration::from_secs(20), "{:?}", t.elapsed());
    }

    /// The #8783 review: a size-1 Brush edit from corner to corner of a 100,000 px canvas sweeps
    /// about 141,000 px but its bounding box is 10 billion; it, a Lasso sliver along it and their
    /// subtraction paint only the tiles they cross, and keep the selection there was.
    #[test]
    fn thin_edits_across_a_huge_canvas_paint_only_their_tiles() {
        let doc = Document::new("huge", photocraft_doc::Size::new(100_000, 100_000), photocraft_doc::ColorMode::Rgb, photocraft_color::SampleType::U8);
        let canvas = doc.bounds();
        let mut small = Surface::new(photocraft_color::PixelFormat::GRAY8);
        small.fill_rect(Rect::new(10, 60, 20, 70), &[1.0]);
        let diagonal = vec![(0.0, 0.0), (99_999.0, 99_999.0)];
        let brush = SelectionEdit::Brush { points: diagonal.clone(), size: 1.0, hardness: 100.0, add: true };
        let lasso = SelectionEdit::Lasso { points: vec![(0.0, 0.0), (99_999.0, 99_999.0), (0.0, 1.5)], add: true };
        assert!(check_size(&[brush.clone(), lasso.clone()], canvas).is_ok());
        let t = std::time::Instant::now();
        for edit in [&brush, &lasso] {
            let s = apply(&doc, None, Some(small.clone()), std::slice::from_ref(edit)).expect("painted");
            assert!(s.tile_count() < 1500, "{}", s.tile_count());
            assert!(s.sample_channel(15, 65, 0) > 0.99 && s.sample_channel(50_000, 50_000, 0) > 0.0 && s.sample_channel(50_000, 40_000, 0) == 0.0);
        }
        let erased =
            apply(&doc, None, Some(small.clone()), &[brush, SelectionEdit::Brush { points: diagonal, size: 9.0, hardness: 100.0, add: false }]).expect("kept");
        assert!(erased.sample_channel(15, 65, 0) > 0.99 && erased.sample_channel(50_000, 50_000, 0) == 0.0);
        assert!(t.elapsed() < std::time::Duration::from_secs(20), "{:?}", t.elapsed());
        // A zigzag across more tiles than one edit may paint is refused before painting.
        let zigzag: Vec<(f32, f32)> = (0..20).map(|i| (if i % 2 == 0 { 0.0 } else { 99_999.0 }, i as f32 * 5000.0)).collect();
        let over = [SelectionEdit::Brush { points: zigzag.clone(), size: 1.0, hardness: 100.0, add: true }];
        assert!(check_size(&over, canvas).is_err());
        assert!(check_size(&[SelectionEdit::Lasso { points: zigzag, add: true }], canvas).is_err());
        // So are many edits that each fit but cross too many tiles in all.
        let rows: Vec<SelectionEdit> = (0..48)
            .map(|i| SelectionEdit::Brush { points: vec![(0.0, i as f32 * 2000.0), (99_999.0, i as f32 * 2000.0)], size: 1.0, hardness: 100.0, add: true })
            .collect();
        assert!(rows.iter().all(|e| check_size(std::slice::from_ref(e), canvas).is_ok()));
        assert!(check_size(&rows, canvas).is_err());
    }
}
