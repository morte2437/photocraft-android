//! #12058: new vertical text is the history state "New Vertical Type Layer", point and paragraph
//! text alike; horizontal text stays "New Type Layer", and a committed Type Mask session, vertical
//! or not, still leaves one "Type Mask" state.

use serde_json::{Value, json};

use crate::Session;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 300, "height": 300})).unwrap();
    s
}

fn entries(s: &Session) -> Vec<String> {
    s.active().unwrap().history.entries()
}

/// The history states `type.create` adds with `params`, and the layer it makes.
fn create(params: Value) -> (Vec<String>, u64) {
    let mut s = session();
    let before = entries(&s).len();
    let layer = s.execute("type.create", params).unwrap()["layer"].as_u64().unwrap();
    (entries(&s)[before..].to_vec(), layer)
}

#[test]
fn vertical_text_is_a_new_vertical_type_layer() {
    for (what, params) in [
        ("point", json!({"text": "AB", "x": 150, "y": 40, "orientation": "vertical"})),
        ("paragraph", json!({"text": "AB", "box": [100, 20, 60, 200], "orientation": "vertical"})),
    ] {
        assert_eq!(create(params).0, ["New Vertical Type Layer"], "{what}");
    }
}

#[test]
fn horizontal_text_is_still_a_new_type_layer() {
    for (what, params) in [
        ("point", json!({"text": "AB", "x": 40, "y": 120})),
        ("point, horizontal", json!({"text": "AB", "x": 40, "y": 120, "orientation": "horizontal"})),
        ("paragraph", json!({"text": "AB", "box": [20, 20, 200, 60]})),
        ("paragraph, horizontal", json!({"text": "AB", "box": [20, 20, 200, 60], "orientation": "horizontal"})),
    ] {
        assert_eq!(create(params).0, ["New Type Layer"], "{what}");
    }
}

/// The Type Mask tools' commands: a hidden working layer from `type.create`, then
/// `type.createSelection` removing it, all coalesced into the session's one step.
#[test]
fn a_type_mask_session_is_still_one_type_mask_state() {
    for orientation in ["horizontal", "vertical"] {
        let mut s = session();
        let before = entries(&s).len();
        let key = json!("type-mask-session");
        let created = json!({"text": "", "x": 150, "y": 40, "orientation": orientation, "coalesce": key});
        let layer = s.execute("type.create", created).unwrap()["layer"].as_u64().unwrap();
        s.execute("layer.setProps", json!({"layer": layer, "visible": false, "coalesce": key})).unwrap();
        s.execute("type.edit", json!({"layer": layer, "text": "AB", "coalesce": key})).unwrap();
        s.execute("type.createSelection", json!({"layer": layer, "removeLayer": true, "coalesce": key})).unwrap();
        assert_eq!(entries(&s)[before..], ["Type Mask"], "{orientation}");
    }
    // Without a layer, `type.createSelection` makes its text itself: the same one state.
    for orientation in ["horizontal", "vertical"] {
        let mut s = session();
        let before = entries(&s).len();
        s.execute("type.createSelection", json!({"text": "AB", "x": 150, "y": 40, "size": 48, "orientation": orientation})).unwrap();
        assert_eq!(entries(&s)[before..], ["Type Mask"], "{orientation}, no layer");
    }
}
