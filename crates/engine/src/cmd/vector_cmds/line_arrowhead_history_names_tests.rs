//! #13267: an edit of one arrowhead setting in the Properties panel is named after it in History
//! ("Change Arrowhead End", "Change Arrowhead Width", ...); other `shape.edit`s keep "Edit Shape".

use serde_json::{Value, json};

use super::*;

/// A document whose active layer is a line with arrowhead settings (a live `ArrowLine`).
fn arrow_line() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 800, "height": 600})).unwrap();
    s.execute(
        "shape.create",
        json!({"kind": "line", "from": [100, 300], "to": [600, 300], "weight": 4, "arrowStart": false, "arrowEnd": false, "arrowWidth": 5, "arrowLength": 10, "arrowConcavity": 0}),
    )
    .unwrap();
    s
}

fn steps(s: &Session) -> usize {
    s.active().unwrap().history.past_len()
}

fn last_step(s: &Session) -> Option<&str> {
    s.active().unwrap().history.undo_label()
}

fn edit(s: &mut Session, p: Value) {
    s.execute("shape.edit", p).unwrap();
}

#[test]
fn each_arrowhead_setting_names_its_step() {
    let cases = [
        (json!({"arrowStart": true}), "Change Arrowhead Start"),
        (json!({"arrowEnd": true}), "Change Arrowhead End"),
        (json!({"arrowWidth": 15}), "Change Arrowhead Width"),
        (json!({"arrowLength": 20}), "Change Arrowhead Length"),
        (json!({"arrowConcavity": 30}), "Change Arrowhead Concavity"),
    ];
    for (p, name) in cases {
        let mut s = arrow_line();
        let n = steps(&s);
        edit(&mut s, p);
        assert_eq!(steps(&s), n + 1, "{name}");
        assert_eq!(last_step(&s), Some(name));
    }
}

#[test]
fn turning_end_off_is_also_change_arrowhead_end() {
    let mut s = arrow_line();
    edit(&mut s, json!({"arrowEnd": true}));
    edit(&mut s, json!({"arrowEnd": false}));
    assert_eq!(last_step(&s), Some("Change Arrowhead End"));
}

#[test]
fn the_panel_keys_layer_and_coalesce_do_not_change_the_name() {
    let mut s = arrow_line();
    let id = s.active().unwrap().active_layer.unwrap().0;
    edit(&mut s, json!({"arrowWidth": 15, "layer": id, "coalesce": "w"}));
    assert_eq!(last_step(&s), Some("Change Arrowhead Width"));
}

#[test]
fn one_drag_of_a_field_is_still_one_step() {
    let mut s = arrow_line();
    let n = steps(&s);
    for w in [6, 9, 12, 15] {
        edit(&mut s, json!({"arrowWidth": w, "coalesce": "arrow-width:drag-1"}));
    }
    assert_eq!(steps(&s), n + 1);
    assert_eq!(last_step(&s), Some("Change Arrowhead Width"));
}

#[test]
fn other_edits_keep_edit_shape() {
    for p in [
        json!({"weight": 8}),
        json!({"from": [50, 300]}),
        json!({"fill": "#ff0000"}),
        json!({"name": "Arrow"}),
        json!({"arrowWidth": 15, "arrowLength": 20}),
        json!({"arrowEnd": true, "weight": 8}),
    ] {
        let mut s = arrow_line();
        edit(&mut s, p.clone());
        assert_eq!(last_step(&s), Some("Edit Shape"), "{p}");
    }
}

#[test]
fn a_refused_arrowhead_edit_adds_no_step() {
    let mut s = arrow_line();
    let n = steps(&s);
    assert!(s.execute("shape.edit", json!({"arrowWidth": "wide"})).is_err());
    assert!(s.execute("shape.edit", json!({"arrowEnd": 1})).is_err());
    assert_eq!(steps(&s), n);
}

#[test]
fn a_plain_line_turning_an_arrowhead_on_is_change_arrowhead_end() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 800, "height": 600})).unwrap();
    s.execute("shape.create", json!({"kind": "line", "from": [100, 300], "to": [600, 300], "weight": 4})).unwrap();
    edit(&mut s, json!({"arrowEnd": true}));
    assert_eq!(last_step(&s), Some("Change Arrowhead End"));
}

#[test]
fn arrowhead_keys_on_a_shape_that_is_not_a_line_stay_edit_shape() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 800, "height": 600})).unwrap();
    s.execute("shape.create", json!({"kind": "rect", "rect": [10, 10, 100, 80]})).unwrap();
    edit(&mut s, json!({"arrowWidth": 15}));
    assert_eq!(last_step(&s), Some("Edit Shape"));
}
