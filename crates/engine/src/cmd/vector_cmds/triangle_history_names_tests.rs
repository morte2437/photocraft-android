//! #13354: a Triangle tool drag is one History step named "Triangle Tool" in Shape and Path mode
//! and the Create Triangle dialog's OK; a corner-radius change on a live triangle is "Change
//! Rounded Triangle". The commands without the tool, and other shapes, keep their own names.

use serde_json::{Value, json};

use super::*;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 800, "height": 600})).unwrap();
    s
}

fn steps(s: &Session) -> usize {
    s.active().unwrap().history.past_len()
}

fn last_step(s: &Session) -> Option<&str> {
    s.active().unwrap().history.undo_label()
}

/// The geometry a Shape-mode Triangle drag (or the Create Triangle dialog) sends.
fn triangle(history_name: Option<&str>) -> Value {
    json!({"kind": "triangle", "rect": [400, 300, 200, 100], "historyName": history_name})
}

/// The closed outline a Path-mode Triangle drag adds to the Work Path.
fn outline(x: f64) -> Value {
    json!({"subpaths": [{"closed": true, "knots": [[x, 300.0], [x + 200.0, 300.0], [x + 100.0, 200.0]]}]})
}

#[test]
fn a_shape_mode_triangle_is_one_step_named_triangle_tool() {
    let mut s = session();
    let n = steps(&s);
    s.execute("shape.create", triangle(Some("Triangle Tool"))).unwrap();
    assert_eq!(steps(&s), n + 1);
    assert_eq!(last_step(&s), Some("Triangle Tool"));
}

#[test]
fn each_path_mode_triangle_is_one_step_named_triangle_tool() {
    let mut s = session();
    for x in [100.0, 400.0] {
        let n = steps(&s);
        s.execute("path.set", json!({"name": "work", "path": outline(x), "historyName": "Triangle Tool"})).unwrap();
        assert_eq!(steps(&s), n + 1);
        assert_eq!(last_step(&s), Some("Triangle Tool"));
    }
}

#[test]
fn rounding_a_live_triangle_is_change_rounded_triangle() {
    let mut s = session();
    let id = s.execute("shape.create", triangle(None)).unwrap()["layer"].as_u64().unwrap();
    s.execute("shape.edit", json!({"layer": id, "cornerRadius": 15.0})).unwrap();
    assert_eq!(last_step(&s), Some("Change Rounded Triangle"));
    // The coalesce key the Properties field and canvas widget send does not widen the match.
    s.execute("shape.edit", json!({"layer": id, "cornerRadius": 20.0, "coalesce": "corner-radius-1"})).unwrap();
    assert_eq!(last_step(&s), Some("Change Rounded Triangle"));
}

#[test]
fn other_edits_and_other_shapes_keep_edit_shape() {
    let mut s = session();
    let tri = s.execute("shape.create", triangle(None)).unwrap()["layer"].as_u64().unwrap();
    s.execute("shape.edit", json!({"layer": tri, "cornerRadius": 5.0, "fill": "#ff0000"})).unwrap();
    assert_eq!(last_step(&s), Some("Edit Shape"), "radius plus another field");
    s.execute("shape.edit", json!({"layer": tri, "sides": 4})).unwrap();
    assert_eq!(last_step(&s), Some("Edit Shape"), "a non-radius change on the triangle");
    let star = s.execute("shape.create", json!({"kind": "star", "rect": [10, 10, 50, 50], "sides": 3, "starRatio": 0.5})).unwrap()["layer"].as_u64().unwrap();
    s.execute("shape.edit", json!({"layer": star, "cornerRadius": 5.0})).unwrap();
    assert_eq!(last_step(&s), Some("Edit Shape"), "rounding a three-point star");
    let tri_poly = s.execute("shape.create", json!({"kind": "polygon", "rect": [10, 10, 50, 50], "sides": 3})).unwrap()["layer"].as_u64().unwrap();
    s.execute("shape.edit", json!({"layer": tri_poly, "cornerRadius": 5.0})).unwrap();
    assert_eq!(
        last_step(&s),
        Some("Edit Shape"),
        "rounding a three-sided Polygon: Photoshop says \"Change Rounded Polygon\", which is #13300's scope, not this issue"
    );
    let pent = s.execute("shape.create", json!({"kind": "polygon", "rect": [10, 10, 50, 50], "sides": 5})).unwrap()["layer"].as_u64().unwrap();
    s.execute("shape.edit", json!({"layer": pent, "cornerRadius": 5.0})).unwrap();
    assert_eq!(last_step(&s), Some("Edit Shape"), "rounding a five-sided polygon");
}

#[test]
fn without_the_tool_the_commands_keep_their_own_names() {
    let mut s = session();
    s.execute("shape.create", triangle(None)).unwrap();
    assert_eq!(last_step(&s), Some("New Shape Layer"));
    s.execute("path.set", json!({"name": "work", "path": outline(100.0)})).unwrap();
    assert_eq!(last_step(&s), Some("Work Path"));
}

#[test]
fn an_unknown_history_name_is_refused_without_a_step() {
    let mut s = session();
    let n = steps(&s);
    assert!(s.execute("shape.create", triangle(Some("Ellipse Tool"))).is_err());
    assert!(s.execute("path.set", json!({"name": "work", "path": outline(100.0), "historyName": 7})).is_err());
    assert_eq!(steps(&s), n);
}
