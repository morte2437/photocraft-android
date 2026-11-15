//! #13205: the History names of Direct Selection's path edits, as in the reference: "Drag Anchor
//! Point" for one anchor, "Drag Anchor Points" for two or more, "Drag Control Point" for a
//! handle and "Drag Segment" for a segment. The Convert Point and Curvature Pen names stay.

use serde_json::{Value, json};

use crate::Session;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 200, "height": 200, "background": "white"})).unwrap();
    // Knot 1 at (120, 40) is smooth, with handles at (100, 40) and (140, 40).
    let path = json!({"subpaths": [{"closed": true, "knots": [
        [40, 40],
        {"anchor": [120, 40], "in": [100, 40], "out": [140, 40], "smooth": true},
        [120, 120],
        [40, 120],
    ]}]});
    s.execute("path.set", json!({"path": path})).unwrap();
    s
}

/// The History step `id` with `params` adds.
fn step(s: &mut Session, id: &str, params: Value) -> String {
    let n = s.active().unwrap().history.entries().len();
    s.execute(id, params).unwrap();
    let names = s.active().unwrap().history.entries();
    assert_eq!(names.len(), n + 1, "{id} adds one step: {names:?}");
    names[n].clone()
}

#[test]
fn one_anchor_is_drag_anchor_point_and_more_are_drag_anchor_points() {
    let mut s = session();
    let one = json!({"anchors": [[0, 2]], "move": [5, 0], "stepName": "Drag Anchor Point"});
    assert_eq!(step(&mut s, "path.moveAnchors", one), "Drag Anchor Point");
    assert_eq!(step(&mut s, "path.moveAnchors", json!({"anchors": [[0, 2], [0, 3]], "move": [5, 0]})), "Drag Anchor Points");
}

#[test]
fn a_handle_drag_is_drag_control_point() {
    let mut s = session();
    for independent in [false, true] {
        let p = json!({"subpath": 0, "knot": 1, "handle": "out", "to": [150, 60], "independent": independent});
        assert_eq!(step(&mut s, "path.moveHandle", p), "Drag Control Point", "independent: {independent}");
    }
}

#[test]
fn a_segment_drag_stays_drag_segment() {
    let mut s = session();
    assert_eq!(step(&mut s, "path.bendSegment", json!({"subpath": 0, "knot": 2, "move": [0, 10]})), "Drag Segment");
}

#[test]
fn other_tools_keep_their_names() {
    let mut s = session();
    let corner = json!({"subpath": 0, "knot": 1, "handle": "out", "to": [150, 60], "independent": true, "stepName": "Corner Drag"});
    assert_eq!(step(&mut s, "path.moveHandle", corner), "Corner Drag");
    assert_eq!(step(&mut s, "path.convertPoint", json!({"subpath": 0, "knot": 2, "stepName": "Change Anchor Point"})), "Change Anchor Point");
    let whole = json!({"anchors": [[0, 0], [0, 1], [0, 2], [0, 3]], "move": [1, 1], "stepName": "Drag Path"});
    assert_eq!(step(&mut s, "path.moveAnchors", whole), "Drag Path");
    // The Curvature Pen's drag, and a one-anchor move that names no step (an arrow-key nudge).
    assert_eq!(step(&mut s, "path.moveAnchors", json!({"anchors": [[0, 1]], "move": [0, 5], "curvature": true})), "Drag Anchor Points");
    assert_eq!(step(&mut s, "path.moveAnchors", json!({"anchors": [[0, 1]], "move": [1, 0]})), "Drag Anchor Points");
}

#[test]
fn an_unknown_step_name_is_refused_and_changes_nothing() {
    let mut s = session();
    let before = (s.active().unwrap().doc.work_path.clone(), s.active().unwrap().history.entries());
    for bad in [json!("Drag Anchor Points"), json!("Drag Control Point"), json!("drag anchor point"), json!(""), json!(1), json!(["Drag Path"])] {
        let e = s.execute("path.moveAnchors", json!({"anchors": [[0, 1]], "move": [1, 0], "stepName": bad})).unwrap_err().to_string();
        assert!(e.contains("\"Drag Path\" or \"Drag Anchor Point\""), "{bad}: {e}");
        let e = s.execute("path.moveHandle", json!({"subpath": 0, "knot": 1, "handle": "out", "to": [150, 60], "stepName": bad})).unwrap_err().to_string();
        assert!(e.contains("stepName") && e.contains("\"Corner Drag\""), "{bad}: {e}");
    }
    assert_eq!((s.active().unwrap().doc.work_path.clone(), s.active().unwrap().history.entries()), before);
}

#[test]
fn the_command_list_says_the_new_names() {
    let specs = crate::command_specs();
    let spec = |id: &str| specs.iter().find(|c| c.id == id).unwrap();
    assert_eq!(spec("path.moveHandle").label, "Drag Control Point");
    assert!(spec("path.moveHandle").params.contains("default \"Drag Control Point\""));
    assert!(spec("path.moveAnchors").params.contains("\"Drag Anchor Point\" when it drags one anchor"));
}
