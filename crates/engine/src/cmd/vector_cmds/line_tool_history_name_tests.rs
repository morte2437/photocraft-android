//! #13244: a Line tool drag is one History step named "Line Tool" in Shape and Path mode; the
//! commands without the tool keep "New Shape Layer" and "Work Path".

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

fn line(history_name: Option<&str>) -> Value {
    json!({"kind": "line", "from": [400, 300], "to": [600, 400], "weight": 6, "historyName": history_name})
}

/// The closed outline a Path-mode Line drag adds to the Work Path.
fn outline(x: f64) -> Value {
    json!({"subpaths": [{"knots": [[x, 300.0], [x + 200.0, 300.0], [x + 200.0, 306.0], [x, 306.0]]}]})
}

#[test]
fn a_shape_mode_line_is_one_step_named_line_tool() {
    let mut s = session();
    let n = steps(&s);
    s.execute("shape.create", line(Some("Line Tool"))).unwrap();
    assert_eq!(steps(&s), n + 1);
    assert_eq!(last_step(&s), Some("Line Tool"));
}

#[test]
fn each_path_mode_line_is_one_step_named_line_tool() {
    let mut s = session();
    for x in [100.0, 400.0] {
        let n = steps(&s);
        s.execute("path.set", json!({"name": "work", "path": outline(x), "historyName": "Line Tool"})).unwrap();
        assert_eq!(steps(&s), n + 1);
        assert_eq!(last_step(&s), Some("Line Tool"));
    }
}

#[test]
fn without_the_tool_the_commands_keep_their_own_names() {
    let mut s = session();
    s.execute("shape.create", line(None)).unwrap();
    assert_eq!(last_step(&s), Some("New Shape Layer"));
    s.execute("path.set", json!({"name": "work", "path": outline(100.0)})).unwrap();
    assert_eq!(last_step(&s), Some("Work Path"));
}

#[test]
fn an_unknown_history_name_is_refused_without_a_step() {
    let mut s = session();
    let n = steps(&s);
    assert!(s.execute("shape.create", line(Some("Ellipse Tool"))).is_err());
    assert!(s.execute("path.set", json!({"name": "work", "path": outline(100.0), "historyName": 7})).is_err());
    assert_eq!(steps(&s), n);
}
