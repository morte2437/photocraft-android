//! `shape.presets.place` names its layer after the shape with a number, each shape counting on its
//! own (#13459).

use serde_json::{Value, json};

use crate::Session;
use photocraft_doc::LayerId;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 640, "height": 480})).unwrap();
    s
}

fn place(s: &mut Session, p: Value) -> String {
    let r = s.execute("shape.presets.place", p).unwrap();
    let id = LayerId(r["layer"].as_u64().unwrap());
    s.active().unwrap().doc.layer(id).unwrap().name.clone()
}

fn drag(s: &mut Session, preset: &str) -> String {
    place(s, json!({"preset": preset, "rect": [400, 300, 200, 100], "keepAspect": false}))
}

#[test]
fn each_shape_counts_its_own_layers() {
    let mut s = session();
    assert_eq!(drag(&mut s, "Heart"), "Heart 1");
    assert_eq!(drag(&mut s, "Heart"), "Heart 2");
    assert_eq!(drag(&mut s, "Star"), "Star 1");
    assert_eq!(drag(&mut s, "Heart"), "Heart 3");
}

#[test]
fn numbering_skips_names_already_taken() {
    let mut s = session();
    s.execute("layer.new.layer", json!({"name": "Heart 1"})).unwrap();
    assert_eq!(drag(&mut s, "Heart"), "Heart 2");
}

#[test]
fn an_explicit_name_is_used_as_given() {
    let mut s = session();
    assert_eq!(place(&mut s, json!({"preset": "Heart", "name": "Valentine"})), "Valentine");
    assert_eq!(place(&mut s, json!({"preset": "Heart", "name": "Heart"})), "Heart");
    assert_eq!(drag(&mut s, "Heart"), "Heart 1");
}

#[test]
fn a_preset_name_ending_in_a_number_keeps_todays_name() {
    let mut s = session();
    s.execute("shape.create", json!({"kind": "star", "rect": [0, 0, 30, 30]})).unwrap();
    s.execute("edit.defineCustomShape", json!({"name": "Badge 2"})).unwrap();
    assert_eq!(drag(&mut s, "Badge 2"), "Badge 2");
}
