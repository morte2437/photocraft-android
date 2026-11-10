//! #11763: each Count tool edit gets the reference application's History name. A `count.setGroup`
//! that changes exactly one field takes that field's name; several fields at once, or a switch of
//! the current group, keep "Count Group Options".

use serde_json::{Value, json};

use crate::Session;

/// A 100 × 100 document with one count group holding one marker at 10,10 (its first state is
/// "Add Count Object").
fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 100, "height": 100})).unwrap();
    s.execute("count.add", json!({"x": 10, "y": 10})).unwrap();
    s
}

fn steps(s: &Session) -> usize {
    s.active().unwrap().history.past_len()
}

/// Run `id` and return the History name of the one step it added.
fn named(s: &mut Session, id: &str, p: Value) -> String {
    let before = steps(s);
    s.execute(id, p.clone()).unwrap_or_else(|e| panic!("{id} {p}: {e}"));
    assert_eq!(steps(s), before + 1, "{id} {p} adds exactly one step");
    s.active().unwrap().history.undo_label().unwrap_or_default().to_string()
}

#[test]
fn marker_edits_are_add_drag_and_delete_count_object() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 100, "height": 100})).unwrap();
    // The first click makes "Count Group 1" too, still one "Add Count Object" step.
    assert_eq!(named(&mut s, "count.add", json!({"x": 10, "y": 10})), "Add Count Object");
    assert_eq!(named(&mut s, "count.add", json!({"x": 40, "y": 40})), "Add Count Object");
    assert_eq!(named(&mut s, "count.move", json!({"x": 10, "y": 10, "to": [20, 20]})), "Drag Count Object");
    assert_eq!(named(&mut s, "count.remove", json!({"x": 40, "y": 40})), "Delete Count Object");
}

#[test]
fn group_edits_keep_or_take_their_names() {
    let mut s = session();
    assert_eq!(named(&mut s, "count.newGroup", json!({"name": "Leaves"})), "Add Count Group");
    assert_eq!(named(&mut s, "count.clear", json!({})), "Clear Count");
    assert_eq!(named(&mut s, "count.deleteGroup", json!({})), "Delete Count Group");
}

#[test]
fn a_single_field_change_uses_that_fields_name() {
    for (p, want) in [
        (json!({"name": "Seeds"}), "Set Count Group Name"),
        (json!({"visible": false}), "Set Count Group Visibility"),
        (json!({"color": "#204080"}), "Set Count Draw Color"),
        (json!({"markerSize": 5}), "Set Count Marker Size"),
        (json!({"labelSize": 20}), "Set Count Font Size"),
    ] {
        let mut s = session();
        assert_eq!(named(&mut s, "count.setGroup", p.clone()), want, "{p}");
    }
}

#[test]
fn several_fields_at_once_keep_count_group_options() {
    let mut s = session();
    assert_eq!(named(&mut s, "count.setGroup", json!({"markerSize": 5, "labelSize": 20})), "Count Group Options");
    // A field sent with its current value doesn't count as a change.
    let mut s = session();
    let name = s.execute("image.analysis.countTool", json!({})).unwrap()["groups"][0]["name"].clone();
    assert_eq!(named(&mut s, "count.setGroup", json!({"name": name, "markerSize": 6})), "Set Count Marker Size");
}

#[test]
fn switching_the_current_group_keeps_todays_name() {
    let mut s = session();
    s.execute("count.newGroup", json!({"name": "Two"})).unwrap();
    assert_eq!(named(&mut s, "count.setGroup", json!({"group": 0})), "Count Group Options", "only `group`");
    assert_eq!(named(&mut s, "count.setGroup", json!({"group": 1, "active": true})), "Count Group Options", "`active: true`, no field change");
    // A field change together with a switch is two changes: today's name too.
    assert_eq!(named(&mut s, "count.setGroup", json!({"group": 0, "active": true, "visible": false})), "Count Group Options");
}

#[test]
fn no_change_adds_no_step_and_bad_params_stay_errors() {
    let mut s = session();
    let before = steps(&s);
    // The current group's own values: nothing changes, no step.
    s.execute("count.setGroup", json!({"group": 0})).unwrap();
    assert_eq!(steps(&s), before);
    for (id, p) in [
        ("count.setGroup", json!({"group": 9, "name": "x"})),
        ("count.setGroup", json!({"markerSize": 2.5})),
        ("count.move", json!({"x": 90, "y": 90, "to": [1, 1]})),
        ("count.remove", json!({"x": 90, "y": 90})),
        ("count.add", json!({"x": 1, "y": 1, "group": 7})),
    ] {
        assert!(s.execute(id, p.clone()).is_err(), "{id} {p}");
        assert_eq!(steps(&s), before, "{id} {p} adds no step");
    }
}
