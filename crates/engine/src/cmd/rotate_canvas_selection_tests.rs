//! Rotate Canvas (Image Rotation > Arbitrary) drops the selection, as the reference does (#14507).
//! One undo brings it back. The Crop tool's straighten turn and the Ruler's Straighten still warp
//! the selection (`rotate_arbitrary_with(.., false)`, not measured).

use photocraft_geom::Rect;

use super::*;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 400, "height": 300, "background": "white"})).unwrap();
    s.execute("select.rect", json!({"x": 50, "y": 50, "width": 100, "height": 100})).unwrap();
    s
}

fn size(s: &Session) -> (u32, u32) {
    let d = &s.active().unwrap().doc;
    (d.size.width, d.size.height)
}

fn selection_bounds(s: &Session) -> Option<Rect> {
    s.active().unwrap().doc.selection.as_ref().map(|m| m.content_bounds())
}

fn original() -> Rect {
    selection_bounds(&session()).unwrap()
}

#[test]
fn the_original_selection_is_the_100_square_at_50_50() {
    let b = original();
    assert_eq!((b.x0, b.y0, b.x1, b.y1), (50, 50, 150, 150));
}

#[test]
fn a_rotation_drops_the_selection_and_one_undo_brings_it_back() {
    let cases = [json!({"angle": 30}), json!({"angle": 90}), json!({"angle": -45}), json!({"angle": 30, "direction": "ccw"})];
    for params in cases {
        let mut s = session();
        let before = original();
        s.execute("image.rotation.arbitrary", params.clone()).unwrap();
        assert!(selection_bounds(&s).is_none(), "{params}: selection kept: {:?}", selection_bounds(&s));
        assert_ne!(size(&s), (400, 300), "{params}: the canvas did not turn");
        assert!(s.undo());
        assert_eq!(size(&s), (400, 300), "{params}");
        assert_eq!(selection_bounds(&s), Some(before), "{params}: undo");
        assert!(s.redo());
        assert!(selection_bounds(&s).is_none(), "{params}: redo");
    }
}

#[test]
fn a_no_op_angle_keeps_the_selection_and_adds_no_step() {
    for angle in [0, 360] {
        let mut s = session();
        let past = s.active().unwrap().history.past_len();
        s.execute("image.rotation.arbitrary", json!({"angle": angle})).unwrap();
        assert_eq!(selection_bounds(&s), Some(original()), "angle {angle}");
        assert_eq!(s.active().unwrap().history.past_len(), past, "angle {angle}");
        assert_eq!(size(&s), (400, 300));
    }
}

#[test]
fn the_flag_false_path_of_crop_and_straighten_still_warps_the_selection() {
    let mut s = session();
    crate::mode_cmds::rotate_arbitrary_with(&mut s, &json!({"angle": 30, "direction": "cw"}), false).unwrap();
    let b = selection_bounds(&s).expect("selection warped, not dropped");
    assert!(b.x1 > b.x0 && b.y1 > b.y0, "{b:?}");
    assert_ne!(b, original());
}
