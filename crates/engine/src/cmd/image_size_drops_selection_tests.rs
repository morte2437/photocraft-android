//! #13856: Fit Image and Image Size drop the active selection when they resample to a new pixel
//! size, as the reference application does; undo brings back the old size and selection in one
//! step. A Fit Image that changes nothing and a resolution-only Image Size keep the selection.

use photocraft_geom::Rect;
use serde_json::{Value, json};

use crate::Session;

/// The issue's document: 400 x 300 px with a 10,10 50 x 50 selection.
fn doc() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 400, "height": 300})).unwrap();
    s.execute("select.rect", json!({"x": 10, "y": 10, "width": 50, "height": 50})).unwrap();
    s
}

fn size(s: &Session) -> (u32, u32) {
    let d = &s.active().unwrap().doc;
    (d.size.width, d.size.height)
}

fn selection(s: &Session) -> Option<Rect> {
    s.active().unwrap().doc.selection.as_ref().map(|sel| sel.content_bounds())
}

fn steps(s: &Session) -> usize {
    s.active().unwrap().history.past_len()
}

#[test]
fn fit_image_that_resizes_drops_the_selection_and_undo_restores_it() {
    let mut s = doc();
    let before = selection(&s);
    assert_eq!(before, Some(Rect::from_xywh(10, 10, 50, 50)));
    let n = steps(&s);
    let r = s.execute("file.automate.fitImage", json!({"width": 200, "height": 200})).unwrap();
    assert_eq!(r["changed"], Value::Bool(true));
    assert_eq!(size(&s), (200, 150));
    assert_eq!(selection(&s), None, "no selection after Fit Image");
    assert_eq!(steps(&s), n + 1, "one history step");
    s.undo();
    assert_eq!(size(&s), (400, 300), "undo restores the size");
    assert_eq!(selection(&s), before, "and the selection, in the same step");
}

#[test]
fn image_size_with_a_new_pixel_size_drops_the_selection_and_undo_restores_it() {
    let mut s = doc();
    let before = selection(&s);
    let n = steps(&s);
    s.execute("image.imageSize", json!({"width": 200, "height": 150, "resample": "bicubic"})).unwrap();
    assert_eq!(size(&s), (200, 150));
    assert_eq!(selection(&s), None, "no selection after Image Size");
    assert_eq!(steps(&s), n + 1, "one history step");
    s.undo();
    assert_eq!(size(&s), (400, 300));
    assert_eq!(selection(&s), before);
}

#[test]
fn fit_image_that_changes_nothing_keeps_the_selection_and_adds_no_step() {
    for p in [json!({"width": 400, "height": 300}), json!({"width": 800, "height": 800, "dontEnlarge": true})] {
        let mut s = doc();
        let before = selection(&s);
        let n = steps(&s);
        let r = s.execute("file.automate.fitImage", p.clone()).unwrap();
        assert_eq!(r["changed"], Value::Bool(false), "{p}");
        assert_eq!(size(&s), (400, 300), "{p}");
        assert_eq!(selection(&s), before, "{p}: selection kept");
        assert_eq!(steps(&s), n, "{p}: no history step");
    }
}

#[test]
fn image_size_with_only_a_new_resolution_keeps_the_selection() {
    let mut s = doc();
    let before = selection(&s);
    s.execute("image.imageSize", json!({"resolution": 300, "resample": "none"})).unwrap();
    assert_eq!(size(&s), (400, 300));
    assert_eq!(s.active().unwrap().doc.resolution_dpi, 300.0);
    assert_eq!(selection(&s), before, "a resolution-only change keeps the selection");
}
