//! #11762: count points keep their document coordinates through Image Size, Canvas Size and Image
//! Rotation (right angles, flips and arbitrary), also off the new canvas; other marks still follow
//! the image, and Crop keeps today's behaviour.

use serde_json::{Value, json};

use crate::Session;

const POINT: [f64; 2] = [99.3, 99.7];

fn doc() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 600, "height": 400})).unwrap();
    s.execute("count.add", json!({"x": POINT[0], "y": POINT[1]})).unwrap();
    s.execute("notes.add", json!({"x": 100, "y": 100})).unwrap();
    s
}

fn count_point(s: &Session) -> [f64; 2] {
    s.active().unwrap().doc.measurement.count_groups[0].points[0]
}

fn note(s: &Session) -> [f64; 2] {
    s.active().unwrap().doc.notes[0].position
}

#[test]
fn count_points_keep_their_coordinates() {
    // The note at [100, 100] on the 600×400 canvas, where each command moves it.
    let steps: [(&str, Value, [f64; 2]); 8] = [
        ("image.imageSize", json!({"width": 300, "height": 200}), [50.0, 50.0]),
        ("image.canvasSize", json!({"width": 400, "height": 300, "anchor": "center"}), [0.0, 50.0]),
        ("image.imageRotation.90cw", json!({}), [300.0, 100.0]),
        ("image.imageRotation.90ccw", json!({}), [100.0, 500.0]),
        ("image.imageRotation.180", json!({}), [500.0, 300.0]),
        ("image.imageRotation.flipCanvasHorizontal", json!({}), [500.0, 100.0]),
        ("image.imageRotation.flipCanvasVertical", json!({}), [100.0, 300.0]),
        ("image.rotation.arbitrary", json!({"angle": 30}), [236.794_919_243_112_3, 136.897_459_621_556_2]),
    ];
    for (cmd, p, moved) in steps {
        let mut s = doc();
        let before = note(&s);
        s.execute(cmd, p).unwrap();
        assert_eq!(count_point(&s), POINT, "{cmd}");
        let n = note(&s);
        assert!((n[0] - moved[0]).abs() < 1e-6 && (n[1] - moved[1]).abs() < 1e-6, "{cmd}: note at {n:?}, expected {moved:?}");
        s.undo();
        assert_eq!((count_point(&s), note(&s)), (POINT, before), "{cmd}: undo");
    }
}

#[test]
fn all_three_in_a_row_leave_the_point_where_it_was() {
    let mut s = doc();
    s.execute("image.imageSize", json!({"width": 300, "height": 200})).unwrap();
    s.execute("image.canvasSize", json!({"width": 400, "height": 300, "anchor": "center"})).unwrap();
    s.execute("image.imageRotation.90cw", json!({})).unwrap();
    assert_eq!(count_point(&s), POINT);
}

#[test]
fn crop_still_moves_count_points() {
    let mut s = doc();
    s.execute("image.crop", json!({"x": 50, "y": 50, "width": 300, "height": 200})).unwrap();
    assert_ne!(count_point(&s), POINT, "not measured: Crop keeps today's behaviour");
}

#[test]
fn ruler_straighten_with_crop_moves_count_points_with_the_image() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 600, "height": 400})).unwrap();
    s.execute("count.add", json!({"x": 250, "y": 150})).unwrap();
    s.execute("notes.add", json!({"x": 250, "y": 150})).unwrap();
    // Straighten Layer is off for the Background (#12033).
    s.execute("layer.new.layer", json!({})).unwrap();
    s.execute("image.analysis.rulerTool", json!({"start": [0, 0], "end": [100, 10]})).unwrap();
    s.execute("image.analysis.straightenLayer", json!({"crop": true})).unwrap();
    let (c, n) = (count_point(&s), note(&s));
    assert!((c[0] - n[0]).abs() < 1e-6 && (c[1] - n[1]).abs() < 1e-6, "count {c:?} vs note {n:?}");
}

#[test]
fn ruler_straighten_with_crop_asks_the_gate_for_its_rotation() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 600, "height": 400})).unwrap();
    // Straighten Layer is off for the Background (#12033).
    s.execute("layer.new.layer", json!({})).unwrap();
    s.execute("image.analysis.rulerTool", json!({"start": [0, 0], "end": [100, 10]})).unwrap();
    let size = s.active().unwrap().doc.size;
    s.authorize = Some(|id, _| if id == "image.rotation.arbitrary" { Err(crate::EngineError::Other("denied".into())) } else { Ok(()) });
    assert!(s.execute("image.analysis.straightenLayer", json!({"crop": true})).is_err(), "the nested rotation is denied");
    s.authorize = None;
    assert_eq!(s.active().unwrap().doc.size, size, "nothing turned");
}
