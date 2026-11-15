//! #14539: Canvas Size drops the active selection when it changes the canvas size, as the
//! reference application does; undo brings back the old size and selection in the same step.
//! A Canvas Size that keeps the size is not measured and keeps today's behaviour; `translate_doc`
//! is unchanged, so its other callers keep theirs (Crop still leaves no selection).

use photocraft_geom::Rect;
use serde_json::json;

use crate::Session;

/// 400 x 300 px with a 100 x 100 selection at (50,50).
fn doc() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 400, "height": 300})).unwrap();
    s.execute("select.rect", json!({"x": 50, "y": 50, "width": 100, "height": 100})).unwrap();
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
fn grow_shrink_and_anchored_canvas_sizes_drop_the_selection_in_one_step() {
    let cases = [
        ("grow centre", json!({"relative": true, "width": 100, "height": 100}), (500, 400)),
        ("shrink", json!({"width": 300, "height": 200}), (300, 200)),
        ("grow top left", json!({"relative": true, "width": 100, "height": 100, "anchor": "topLeft"}), (500, 400)),
    ];
    for (name, p, new_size) in cases {
        let mut s = doc();
        let before = selection(&s);
        assert_eq!(before, Some(Rect::from_xywh(50, 50, 100, 100)));
        let n = steps(&s);
        s.execute("image.canvasSize", p).unwrap();
        assert_eq!(size(&s), new_size, "{name}");
        assert_eq!(selection(&s), None, "{name}: no selection after Canvas Size");
        assert_eq!(steps(&s), n + 1, "{name}: exactly one history step");
        s.undo();
        assert_eq!(size(&s), (400, 300), "{name}: undo restores the size");
        assert_eq!(selection(&s), before, "{name}: and the selection, in the same step");
        s.redo();
        assert_eq!(size(&s), new_size, "{name}: redo");
        assert_eq!(selection(&s), None, "{name}: redo drops the selection again");
    }
}

#[test]
fn guides_still_move_with_the_offset() {
    let mut s = doc();
    {
        let d = std::sync::Arc::make_mut(&mut s.active_mut().unwrap().doc);
        d.guides.vertical.push(100.0);
        d.guides.horizontal.push(50.0);
    }
    s.execute("image.canvasSize", json!({"relative": true, "width": 100, "height": 100})).unwrap();
    let d = &s.active().unwrap().doc;
    assert_eq!(d.guides.vertical, vec![150.0]);
    assert_eq!(d.guides.horizontal, vec![100.0]);
}

#[test]
fn a_canvas_size_that_keeps_the_size_keeps_the_selection() {
    let mut s = doc();
    let before = selection(&s);
    s.execute("image.canvasSize", json!({"width": 400, "height": 300})).unwrap();
    assert_eq!(size(&s), (400, 300));
    assert_eq!(selection(&s), before, "not measured: today's behaviour");
}

#[test]
fn crop_behaves_as_before() {
    // Crop consumes its selection on main; Canvas Size's change must not alter that.
    let mut s = doc();
    s.execute("image.crop", json!({"x": 10, "y": 20, "width": 300, "height": 200})).unwrap();
    assert_eq!(size(&s), (300, 200));
    assert_eq!(selection(&s), None, "Crop leaves no selection, as on main");
}
