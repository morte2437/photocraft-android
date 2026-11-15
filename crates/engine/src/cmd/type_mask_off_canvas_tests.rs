//! #12074: a Type Mask that runs past the canvas edge keeps the off-canvas part of its selection,
//! as the reference application does; masks fully on the canvas select exactly as before.

use photocraft_doc::{LayerContent, LayerId};
use photocraft_geom::Rect;
use photocraft_raster::Surface;
use serde_json::{Value, json};

use crate::Session;

fn session(width: u32, height: u32) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": width, "height": height})).unwrap();
    s
}

fn selection(s: &Session) -> Option<Surface> {
    s.active().unwrap().doc.selection.clone()
}

fn bounds(s: &Session) -> Rect {
    selection(s).map_or(Rect::EMPTY, |m| m.content_bounds())
}

fn mask(s: &mut Session, params: Value) -> Rect {
    s.execute(super::CMD, params).unwrap();
    bounds(s)
}

/// The rendered bounds of the same text as a type layer, which the canvas doesn't clip.
fn layer_bounds(width: u32, height: u32, params: Value) -> Rect {
    let mut s = session(width, height);
    let r = s.execute("type.create", params).unwrap();
    let b: Vec<i32> = r["bounds"].as_array().unwrap().iter().map(|v| i32::try_from(v.as_i64().unwrap()).unwrap()).collect();
    Rect::new(b[0], b[1], b[2], b[3])
}

#[test]
fn a_mask_started_on_the_pasteboard_selects_the_glyphs_left_of_the_canvas() {
    let text = json!({"text": "MASK", "x": -129, "y": 300, "size": 48});
    let mut s = session(600, 400);
    let b = mask(&mut s, text.clone());
    assert!(b.x0 < 0, "{b:?}");
    assert!(b.x1 > 0, "the text reaches onto the canvas: {b:?}");
    assert_eq!(b, layer_bounds(600, 400, text), "the whole outline, as the type layer renders it");
}

#[test]
fn a_vertical_mask_keeps_the_part_below_the_canvas() {
    let text = json!({"text": "MASKMASKMASK", "x": 500, "y": 450, "size": 48, "orientation": "vertical"});
    let mut s = session(800, 600);
    let b = mask(&mut s, text.clone());
    assert!(b.y0 < 600 && b.y1 > 600, "{b:?}");
    assert_eq!(b, layer_bounds(800, 600, text));

    // Moved up, the rows that were below the canvas are there: it equals the text masked there.
    s.execute("select.transformSelection", json!({"dy": -300, "stepName": "Move Selection"})).unwrap();
    let moved = selection(&s).unwrap();
    let mut s = session(800, 600);
    mask(&mut s, json!({"text": "MASKMASKMASK", "x": 500, "y": 150, "size": 48, "orientation": "vertical"}));
    let there = selection(&s).unwrap();
    let b = there.content_bounds();
    assert_eq!(moved.content_bounds(), b);
    assert_eq!(moved.read_region(b), there.read_region(b), "pixel for pixel");
}

/// Moving the selection onto the canvas shows the whole outline: it equals a mask of the same text
/// started there.
#[test]
fn moving_the_selection_onto_the_canvas_shows_the_whole_outline() {
    let mut s = session(600, 400);
    mask(&mut s, json!({"text": "MASK", "x": -129, "y": 300, "size": 48}));
    s.execute("select.transformSelection", json!({"dx": 200, "stepName": "Move Selection"})).unwrap();
    let moved = selection(&s).unwrap();
    let mut s = session(600, 400);
    mask(&mut s, json!({"text": "MASK", "x": 71, "y": 300, "size": 48}));
    let there = selection(&s).unwrap();
    assert_eq!(moved.content_bounds(), there.content_bounds());
    let b = there.content_bounds();
    assert!(b.x0 >= 0, "now fully on the canvas: {b:?}");
    assert_eq!(moved.read_region(b), there.read_region(b), "pixel for pixel");
}

/// Adding a mask to, or subtracting it from, a selection that runs past the canvas keeps that part.
#[test]
fn add_and_subtract_keep_an_off_canvas_previous_selection() {
    for operation in ["add", "subtract"] {
        let mut s = session(600, 400);
        // A rectangle dragged half off the left edge.
        s.execute("select.rect", json!({"x": 10, "y": 50, "width": 100, "height": 70})).unwrap();
        s.execute("select.transformSelection", json!({"dx": -60, "stepName": "Move Selection"})).unwrap();
        let first = bounds(&s);
        assert_eq!(first, Rect::new(-50, 50, 50, 120), "precondition");
        let off = Rect::new(-50, 50, 0, 120);
        let before = selection(&s).unwrap().read_region(off);
        mask(&mut s, json!({"text": "I", "x": 400, "y": 300, "size": 48, "operation": operation}));
        assert_eq!(bounds(&s).x0, -50, "{operation}: the off-canvas part stays");
        assert_eq!(selection(&s).unwrap().read_region(off), before, "{operation}: unchanged left of the canvas");
    }
}

/// The glyph buffers are no longer bounded by the canvas, so running out of memory for either of
/// them is an error that changes nothing, not an abort.
#[test]
fn a_failed_coverage_allocation_is_an_error_that_changes_nothing() {
    for skip in [0, 1] {
        let mut s = session(600, 400);
        s.execute("select.rect", json!({"x": 400, "y": 50, "width": 100, "height": 70})).unwrap();
        let (before, steps) = (selection(&s), s.active().unwrap().history.entries().len());
        crate::allocation::fail_after_for_test(skip);
        let err = s.execute(super::CMD, json!({"text": "MASK", "x": -129, "y": 300, "size": 48})).unwrap_err().to_string();
        assert!(err.contains("not enough memory for the Type Mask selection"), "allocation {skip}: {err}");
        assert_eq!(selection(&s), before, "allocation {skip}");
        assert_eq!(s.active().unwrap().history.entries().len(), steps, "allocation {skip}: no history step");
    }
}

/// The expected selection at one pixel from the rectangle's and the glyphs' coverage.
type Combine = fn(f32, f32) -> f32;

/// A mask fully on the canvas selects what it did before: the glyphs' coverage, which is the alpha
/// of the same text rasterized, and new / add / subtract / intersect with a rectangle over part of
/// the text combine the two pixel by pixel.
#[test]
fn masks_on_the_canvas_select_as_before() {
    let text = json!({"text": "MASK", "x": 100, "y": 200, "size": 48, "color": "#000000"});
    let outline = layer_bounds(600, 400, text.clone());
    assert!(outline.x0 >= 0 && outline.y0 >= 0 && outline.x1 <= 600 && outline.y1 <= 400, "{outline:?}");
    let rect = Rect::new(outline.x0 + 20, outline.y0 - 10, outline.x0 + 80, outline.y1 + 10);
    let area = outline.union(&rect);
    let glyphs: Vec<f32> = {
        let mut s = session(600, 400);
        let id = s.execute("type.create", text.clone()).unwrap()["layer"].as_u64().unwrap();
        s.execute("type.rasterize", json!({"layer": id})).unwrap();
        match &s.active().unwrap().doc.layer(LayerId(id)).unwrap().content {
            LayerContent::Raster(px) => px.read_region(area).as_chunks::<4>().0.iter().map(|p| p[3]).collect(),
            other => panic!("{}", other.kind_name()),
        }
    };
    assert!(glyphs.iter().any(|&a| a > 0.0 && a < 1.0), "anti-aliased edges are compared too");
    let inside = |i: usize| {
        let (x, y) = (area.x0 + (i % area.width() as usize) as i32, area.y0 + (i / area.width() as usize) as i32);
        rect.contains(x, y)
    };
    let cases: [(&str, Combine); 4] = [("new", |_, g| g), ("add", |r, g| r.max(g)), ("subtract", |r, g| (r - g).max(0.0)), ("intersect", |r, g| r.min(g))];
    for (operation, expected) in cases {
        let mut s = session(600, 400);
        s.execute("select.rect", json!({"x": rect.x0, "y": rect.y0, "width": rect.width(), "height": rect.height()})).unwrap();
        let mut p = text.clone();
        p["operation"] = json!(operation);
        s.execute(super::CMD, p).unwrap();
        let got = selection(&s).map_or_else(|| vec![0.0; glyphs.len()], |m| m.read_region(area));
        for (i, (&v, &g)) in got.iter().zip(&glyphs).enumerate() {
            let want = expected(if inside(i) { 1.0 } else { 0.0 }, g);
            assert!((v - want).abs() <= 1.5 / 255.0, "{operation}: pixel {i} is {v}, expected {want}");
        }
    }
}

/// Text with no glyphs selects nothing: Add and Subtract leave an earlier selection as it is,
/// Replace and Intersect clear it, as before the combine step changed.
#[test]
fn a_mask_with_no_glyphs_keeps_or_clears_as_before() {
    for operation in ["new", "add", "subtract", "intersect"] {
        for earlier in [false, true] {
            let mut s = session(600, 400);
            if earlier {
                s.execute("select.rect", json!({"x": 400, "y": 50, "width": 100, "height": 70})).unwrap();
            }
            let before = selection(&s);
            let r = s.execute(super::CMD, json!({"text": "   ", "x": 100, "y": 200, "size": 48, "operation": operation})).unwrap();
            let kept = earlier && matches!(operation, "add" | "subtract");
            assert_eq!(selection(&s), if kept { before } else { None }, "{operation}, earlier selection {earlier}");
            assert_eq!(r["selected"], json!(kept), "{operation}, earlier selection {earlier}");
        }
    }
}

/// Replace and Intersect keep nothing of an earlier selection outside the text, its part past the
/// canvas included.
#[test]
fn replace_and_intersect_drop_an_off_canvas_previous_selection() {
    // A new mask keeps nothing of the earlier rectangle (it ends at x 50, the text starts at 400);
    // an intersection keeps only its on-canvas part under the text.
    for (operation, x, left, right) in [("new", 400, 50, i32::MAX), ("intersect", 10, 0, 50)] {
        let mut s = session(600, 400);
        s.execute("select.rect", json!({"x": 10, "y": 50, "width": 100, "height": 70})).unwrap();
        s.execute("select.transformSelection", json!({"dx": -60, "stepName": "Move Selection"})).unwrap();
        assert_eq!(bounds(&s), Rect::new(-50, 50, 50, 120), "precondition");
        let b = mask(&mut s, json!({"text": "I", "x": x, "y": 110, "size": 48, "operation": operation}));
        assert!(!b.is_empty() && b.x0 >= left && b.x1 <= right, "{operation}: only the text's part is left: {b:?}");
    }
}

/// Adding to an unusual earlier selection (not 8-bit grey) keeps its format; when the buffers that
/// combining makes over the text can't be had, for any selection, that's an error that changes
/// nothing, not an abort.
#[test]
fn an_unusual_earlier_selection_keeps_its_format_or_fails_cleanly() {
    use photocraft_color::{PixelFormat, SampleType};
    let format = PixelFormat::GRAY8.with_sample(SampleType::F32);
    let earlier = Rect::new(-50, 50, 50, 120);
    let mut old = Surface::new(format);
    old.write_region(earlier, &vec![1.0; (earlier.width() * earlier.height()) as usize]);
    let mut s = session(600, 400);
    std::sync::Arc::make_mut(&mut s.active_mut().unwrap().doc).selection = Some(old.clone());
    let text = json!({"text": "I", "x": 400, "y": 300, "size": 48, "operation": "add"});

    // The two glyph buffers are had; the third allocation is the room for combining.
    crate::allocation::fail_after_for_test(2);
    let steps = s.active().unwrap().history.entries().len();
    let err = s.execute(super::CMD, text.clone()).unwrap_err().to_string();
    assert!(err.contains("not enough memory for the Type Mask selection"), "{err}");
    assert_eq!(selection(&s), Some(old.clone()), "unchanged");
    assert_eq!(s.active().unwrap().history.entries().len(), steps, "no history step");

    s.execute(super::CMD, text.clone()).unwrap();
    let now = selection(&s).unwrap();
    assert_eq!(now.format(), format);
    assert_eq!(now.read_region(earlier), old.read_region(earlier), "the earlier part stays, past the canvas too");

    // An 8-bit grey earlier selection, in every mode, gets the same check for its new tiles.
    for operation in ["add", "subtract", "new", "intersect"] {
        let mut s = session(600, 400);
        s.execute("select.rect", json!({"x": 10, "y": 50, "width": 100, "height": 70})).unwrap();
        let (before, steps) = (selection(&s), s.active().unwrap().history.entries().len());
        crate::allocation::fail_after_for_test(2);
        let err = s.execute(super::CMD, json!({"text": "I", "x": 400, "y": 300, "size": 48, "operation": operation})).unwrap_err().to_string();
        assert!(err.contains("not enough memory for the Type Mask selection"), "{operation}: {err}");
        assert_eq!(selection(&s), before, "{operation}: unchanged");
        assert_eq!(s.active().unwrap().history.entries().len(), steps, "{operation}: no history step");
    }
}
