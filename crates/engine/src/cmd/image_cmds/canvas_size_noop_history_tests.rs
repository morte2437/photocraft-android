//! #14537: a Canvas Size whose result equals the current size changes nothing and adds no history
//! step, as the reference application does; any real change still adds exactly one "Canvas Size"
//! step. The dialog's OK and a recorded action step dispatch `image.canvasSize`, so they share
//! this behaviour.

use photocraft_geom::Rect;
use serde_json::{Value, json};

use crate::Session;

/// The issue's document: 400 x 300 px with a Background and a layer of pixels.
fn doc() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 400, "height": 300, "background": "white"})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.edit("paint", |doc, active| {
        let l = doc.layer_mut(active.unwrap()).unwrap();
        l.surface_mut().unwrap().fill_rect(Rect::new(100, 50, 200, 150), &[1.0, 0.0, 0.0, 1.0]);
        Ok(())
    })
    .unwrap();
    s
}

fn size(s: &Session) -> (u32, u32) {
    let d = &s.active().unwrap().doc;
    (d.size.width, d.size.height)
}

fn steps(s: &Session) -> usize {
    s.active().unwrap().history.past_len()
}

fn sample(s: &Session) -> Vec<f32> {
    s.active().unwrap().doc.layers.last().unwrap().surface().unwrap().pixel(150, 100)
}

#[test]
fn a_canvas_size_that_keeps_the_size_adds_no_history_step() {
    let cases = [
        json!({}),
        json!({"width": 400, "height": 300}),
        json!({"relative": true, "width": 0, "height": 0}),
        json!({"width": 100, "height": 100, "units": "percent"}),
    ];
    for params in cases {
        let mut s = doc();
        let n = steps(&s);
        let pixel = sample(&s);
        assert_eq!(pixel, vec![1.0, 0.0, 0.0, 1.0], "the test document has its pixels");
        let r = s.execute("image.canvasSize", params.clone()).unwrap();
        assert_eq!(r, json!({"width": 400, "height": 300, "offset": [0, 0]}), "response for {params}");
        assert_eq!(steps(&s), n, "no history step for {params}");
        assert_eq!(size(&s), (400, 300), "size for {params}");
        assert_eq!(sample(&s), pixel, "pixel for {params}");
    }
}

#[test]
fn a_bad_anchor_still_errors_when_the_size_is_unchanged() {
    let mut s = doc();
    let n = steps(&s);
    assert!(s.execute("image.canvasSize", json!({"anchor": "nowhere"})).is_err());
    assert_eq!(steps(&s), n);
}

#[test]
fn a_real_canvas_size_change_still_adds_exactly_one_step() {
    let mut s = doc();
    let n = steps(&s);
    let r = s.execute("image.canvasSize", json!({"relative": true, "width": 10, "height": 0})).unwrap();
    assert_eq!(r["width"], Value::from(410));
    assert_eq!(size(&s), (410, 300));
    assert_eq!(steps(&s), n + 1, "exactly one step");
    assert_eq!(s.active().unwrap().history.undo_label(), Some("Canvas Size"));
    s.undo();
    assert_eq!(size(&s), (400, 300));
}
