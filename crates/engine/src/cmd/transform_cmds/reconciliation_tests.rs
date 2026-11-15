use super::*;

fn session() -> Session {
    session_depth(8)
}

fn session_depth(depth: u32) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 100, "height": 100, "depth": depth})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.edit("paint", |doc, active| {
        let l = doc.layer_mut(active.unwrap()).unwrap();
        l.surface_mut().unwrap().fill_rect(Rect::new(10, 10, 30, 20), &[1.0, 0.0, 0.0, 1.0]);
        Ok(())
    })
    .unwrap();
    s
}

/// Two layers: the session's red rect at (10,10)-(30,20) and a blue one at (60,50)-(80,70),
/// both selected; returns (bottom, top).
fn two_layers() -> (Session, LayerId, LayerId) {
    let mut s = session();
    let a = s.active().unwrap().active_layer.unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.edit("paint", |doc, active| {
        doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap().fill_rect(Rect::new(60, 50, 80, 70), &[0.0, 0.0, 1.0, 1.0]);
        Ok(())
    })
    .unwrap();
    let b = s.active().unwrap().active_layer.unwrap();
    (s, a, b)
}

fn bounds_of(s: &Session, id: LayerId) -> Rect {
    s.active().unwrap().doc.layer(id).unwrap().surface().unwrap().content_bounds()
}

#[test]
fn several_layers_scale_around_one_shared_frame_in_one_step() {
    let (mut s, a, b) = two_layers();
    assert_eq!(transform_bounds_of(&s.active().unwrap().doc, &[a, b]), Rect::new(10, 10, 80, 70));
    let steps = s.active().unwrap().history.entries().len();
    // Halve the shared frame about its top-left corner.
    let r = s.execute("edit.transform", json!({"layers": [a.0, b.0], "quad": [[10, 10], [45, 10], [45, 40], [10, 40]]})).unwrap();
    assert_eq!(r["rect"], json!([10.0, 10.0, 80.0, 70.0]));
    assert_eq!(bounds_of(&s, a), Rect::new(10, 10, 20, 15));
    assert_eq!(bounds_of(&s, b), Rect::new(35, 30, 45, 40));
    assert_eq!(s.active().unwrap().history.entries().len(), steps + 1, "one history step");
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(bounds_of(&s, a), Rect::new(10, 10, 30, 20));
    assert_eq!(bounds_of(&s, b), Rect::new(60, 50, 80, 70));
}

#[test]
fn several_layers_move_and_rotate_consistently() {
    let (mut s, a, b) = two_layers();
    s.execute("edit.transform", json!({"layers": [a.0, b.0], "matrix": [1, 0, 0, 1, 5, 7]})).unwrap();
    assert_eq!(bounds_of(&s, a), Rect::new(15, 17, 35, 27));
    assert_eq!(bounds_of(&s, b), Rect::new(65, 57, 85, 77));
    s.execute("edit.undo", json!({})).unwrap();
    // 180° about the shared centre (45, 40): each rect lands where the other's mirror is.
    s.execute("edit.transform", json!({"layers": [a.0, b.0], "quad": [[80, 70], [10, 70], [10, 10], [80, 10]], "interpolation": "nearest"})).unwrap();
    assert_eq!(bounds_of(&s, a), Rect::new(60, 60, 80, 70));
    assert_eq!(bounds_of(&s, b), Rect::new(10, 10, 30, 30));
}

#[test]
fn several_layers_accept_relative_transform_params_after_main_merge() {
    let (mut s, a, b) = two_layers();
    let steps = s.active().unwrap().history.entries().len();
    s.execute("edit.transform", json!({"layers": [a.0, b.0], "dx": 5, "dy": 7})).unwrap();
    assert_eq!(bounds_of(&s, a), Rect::new(15, 17, 35, 27));
    assert_eq!(bounds_of(&s, b), Rect::new(65, 57, 85, 77));
    assert_eq!(s.active().unwrap().history.entries().len(), steps + 1);
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(bounds_of(&s, a), Rect::new(10, 10, 30, 20));
    assert_eq!(bounds_of(&s, b), Rect::new(60, 50, 80, 70));
}

#[test]
fn several_layers_refuse_when_one_is_locked_and_skip_the_empty() {
    let (mut s, a, b) = two_layers();
    s.execute("layer.new.layer", json!({})).unwrap();
    let empty = s.active().unwrap().active_layer.unwrap();
    s.edit("lock", |doc, _| {
        doc.layer_mut(b).unwrap().locks.position = true;
        Ok(())
    })
    .unwrap();
    let before = s.active().unwrap().history.entries().len();
    assert!(s.execute("edit.transform", json!({"layers": [a.0, b.0], "matrix": [1, 0, 0, 1, 5, 0]})).is_err());
    assert_eq!(bounds_of(&s, a), Rect::new(10, 10, 30, 20), "nothing moved");
    assert_eq!(s.active().unwrap().history.entries().len(), before);
    // One layer left: the single-layer path, with its own frame.
    s.execute("edit.transform", json!({"layers": [a.0, empty.0], "matrix": [1, 0, 0, 1, 5, 0]})).unwrap();
    assert_eq!(bounds_of(&s, a), Rect::new(15, 10, 35, 20));
}

#[test]
fn flip_and_rotate_turn_every_selected_layer_around_the_shared_box() {
    let (mut s, a, b) = two_layers();
    s.execute("layer.select", json!({"layer": a.0, "mode": "add"})).unwrap();
    let steps = s.active().unwrap().history.entries().len();
    s.execute("edit.transform.flipHorizontal", json!({})).unwrap();
    assert_eq!(bounds_of(&s, a), Rect::new(60, 10, 80, 20));
    assert_eq!(bounds_of(&s, b), Rect::new(10, 50, 30, 70));
    assert_eq!(s.active().unwrap().history.entries().len(), steps + 1, "one step");
    s.execute("edit.undo", json!({})).unwrap();
    s.execute("edit.transform.rotate180", json!({})).unwrap();
    assert_eq!(bounds_of(&s, a), Rect::new(60, 60, 80, 70));
    assert_eq!(bounds_of(&s, b), Rect::new(10, 10, 30, 30));
    s.execute("edit.undo", json!({})).unwrap();
    // A named layer still turns alone, around its own box.
    s.execute("edit.transform.flipVertical", json!({"layer": a.0})).unwrap();
    assert_eq!((bounds_of(&s, a), bounds_of(&s, b)), (Rect::new(10, 10, 30, 20), Rect::new(60, 50, 80, 70)));
}

#[test]
fn several_layers_convert_the_background_for_transform() {
    let mut s = session();
    let bg = s.active().unwrap().doc.layers[0].id;
    let layer = s.active().unwrap().active_layer.unwrap();
    assert!(s.active().unwrap().doc.is_background(bg));
    let before = s.active().unwrap().doc.clone();
    let steps = s.active().unwrap().history.past_len();
    s.execute("edit.transform", json!({"layers": [bg.0, layer.0], "dx": 5})).unwrap();
    let st = s.active().unwrap();
    assert!(!st.doc.is_background(bg));
    assert!(st.doc.layer(bg).unwrap().surface().unwrap().format().alpha);
    assert_eq!(st.history.past_len(), steps + 1);
    s.execute("edit.undo", json!({})).unwrap();
    assert!(s.active().unwrap().doc == before);
}

#[test]
fn several_layers_preflight_the_selection_pixel_budget() {
    let (mut s, a, b) = two_layers();
    s.execute("select.rect", json!({"x": 0, "y": 0, "width": 100, "height": 100})).unwrap();
    let before = s.active().unwrap().doc.clone();
    let steps = s.active().unwrap().history.past_len();
    let err = s.execute("edit.transform", json!({"layers": [a.0, b.0], "matrix": [50, 0, 0, 50, 0, 0]})).unwrap_err().to_string();
    assert!(err.contains("selection"), "{err}");
    assert!(s.active().unwrap().doc == before);
    assert_eq!(s.active().unwrap().history.past_len(), steps);
}
