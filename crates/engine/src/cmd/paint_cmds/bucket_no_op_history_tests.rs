//! #12535: a Paint Bucket click that fills nothing (on the pasteboard, or on a pixel outside the
//! selection) adds no history state; a fill inside the selection adds one.

use super::*;
use photocraft_geom::Rect;

/// A white 80x60 document whose Background has a black wall at x = 30, so the regions left and
/// right of it are separate; with a 20x20 selection in the top-left corner.
fn walled() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 80, "height": 60, "background": "white"})).unwrap();
    s.edit("wall", |doc, active| {
        doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap().fill_rect(Rect::new(30, 0, 31, 60), &[0.0, 0.0, 0.0, 1.0]);
        Ok(())
    })
    .unwrap();
    s.execute("select.rect", json!({"x": 0, "y": 0, "width": 20, "height": 20, "antiAlias": false})).unwrap();
    s
}

fn last(s: &Session) -> (usize, Option<String>) {
    let h = &s.active().unwrap().history;
    (h.past_len(), h.entries().last().map(|e| e.to_string()))
}

fn pixels(s: &Session) -> Vec<f32> {
    let d = s.active().unwrap();
    d.doc.layer(d.active_layer.unwrap()).unwrap().surface().unwrap().read_region(d.doc.bounds())
}

fn px(s: &Session, x: i32, y: i32) -> Vec<f32> {
    let d = s.active().unwrap();
    d.doc.layer(d.active_layer.unwrap()).unwrap().surface().unwrap().pixel(x, y)
}

/// Runs a click that must fill nothing: `filled: false`, no error, no state, no pixel change.
fn fills_nothing(s: &mut Session, params: Value, case: &str) {
    let (before, state) = (pixels(s), last(s));
    let r = s.execute("paint.bucket", params).unwrap_or_else(|e| panic!("{case}: {e}"));
    assert_eq!(r["filled"], false, "{case}: {r}");
    assert_eq!(last(s), state, "{case}: no history state");
    assert!(pixels(s) == before, "{case}: no pixel changes");
}

#[test]
fn a_click_on_the_pasteboard_adds_no_state() {
    for selection in [false, true] {
        let mut s = walled();
        if !selection {
            s.execute("select.deselect", json!({})).unwrap();
        }
        for (x, y) in [(-5.0, 3.0), (85.0, 10.0), (40.0, -1.0), (40.0, 60.0), (-1e12, 1e12)] {
            for all in [false, true] {
                let case = format!("selection {selection}, ({x}, {y}), sampleAllLayers {all}");
                fills_nothing(&mut s, json!({"x": x, "y": y, "color": "#ff0000", "sampleAllLayers": all}), &case);
            }
        }
    }
}

#[test]
fn a_region_wholly_outside_the_selection_adds_no_state() {
    let mut s = walled();
    for all in [false, true] {
        for contiguous in [true, false] {
            // Right of the wall: white pixels that never reach the selection. Non-contiguous,
            // the black wall's colour matches only the wall itself.
            let (x, y) = if contiguous { (50, 5) } else { (30, 5) };
            let case = format!("({x}, {y}), sampleAllLayers {all}, contiguous {contiguous}");
            fills_nothing(&mut s, json!({"x": x, "y": y, "color": "#ff0000", "sampleAllLayers": all, "contiguous": contiguous}), &case);
            let pattern = json!({"x": x, "y": y, "contents": "pattern", "pattern": "Checkerboard", "sampleAllLayers": all, "contiguous": contiguous});
            fills_nothing(&mut s, pattern, &format!("{case}: a pattern fill"));
        }
    }
}

#[test]
fn a_click_outside_the_selection_adds_no_state_even_when_its_region_reaches_it() {
    // Left of the wall: the region from (25, 5) takes in the whole selection.
    let mut s = walled();
    for all in [false, true] {
        fills_nothing(&mut s, json!({"x": 25, "y": 5, "color": "#ff0000", "sampleAllLayers": all}), &format!("sampleAllLayers {all}"));
    }
    // The issue's measured case: a plain 800x600 document, a click outside the selection.
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 800, "height": 600, "background": "white"})).unwrap();
    s.execute("select.rect", json!({"x": 0, "y": 0, "width": 200, "height": 200})).unwrap();
    fills_nothing(&mut s, json!({"x": 400, "y": 50, "color": "#ff0000"}), "800x600, click at (400, 50)");
}

/// Not measured, today's behaviour: a click on a partly selected pixel fills, at that coverage.
#[test]
fn a_click_on_a_partly_selected_pixel_still_fills() {
    let mut s = walled();
    s.edit("feathered", |doc, _| {
        let mut m = photocraft_raster::Surface::new(photocraft_color::PixelFormat::GRAY8);
        m.fill_rect(Rect::new(0, 0, 20, 20), &[0.5]);
        doc.selection = Some(m);
        Ok(())
    })
    .unwrap();
    let (n, _) = last(&s);
    let r = s.execute("paint.bucket", json!({"x": 5, "y": 5, "color": "#ff0000", "antiAlias": false})).unwrap();
    assert_eq!(r["filled"], true, "{r}");
    assert_eq!(last(&s), (n + 1, Some("Paint Bucket".to_string())));
    let p = px(&s, 5, 5);
    assert!(p != vec![1.0; 4] && p != vec![1.0, 0.0, 0.0, 1.0], "half the red: {p:?}");
}

#[test]
fn a_click_inside_the_selection_fills_only_the_selected_part_as_one_state() {
    let mut s = walled();
    let (n, _) = last(&s);
    let before = pixels(&s);
    let r = s.execute("paint.bucket", json!({"x": 5, "y": 5, "color": "#ff0000", "antiAlias": false})).unwrap();
    assert_eq!(r["filled"], true, "{r}");
    assert_eq!(last(&s), (n + 1, Some("Paint Bucket".to_string())));
    assert_eq!(px(&s, 5, 5), vec![1.0, 0.0, 0.0, 1.0]);
    assert_eq!(px(&s, 19, 19), vec![1.0, 0.0, 0.0, 1.0]);
    assert_eq!(px(&s, 25, 5), vec![1.0; 4], "in the region but not selected");
    assert_eq!(px(&s, 5, 40), vec![1.0; 4], "in the region but not selected");
    assert!(s.undo());
    assert!(pixels(&s) == before, "undo restores the pixels");
}

/// Refusals keep their error off the canvas too, and add no state.
fn refused(s: &mut Session, params: Value, message: &str) {
    let (n, _) = last(s);
    let e = s.execute("paint.bucket", params).unwrap_err();
    assert_eq!(e.to_string(), message);
    assert_eq!(last(s).0, n, "{message}: no state");
}

#[test]
fn refusals_stay_errors_off_the_canvas() {
    let mut s = walled();
    s.execute("layer.new.layer", json!({})).unwrap();
    refused(&mut s, json!({"x": -5, "y": 3, "target": "mask"}), "layer has no mask");
    s.execute("layer.lockLayers", json!({"pixels": true})).unwrap();
    refused(&mut s, json!({"x": -5, "y": 3}), "Could not complete your request because the layer \"Layer 1\" is locked");
}
