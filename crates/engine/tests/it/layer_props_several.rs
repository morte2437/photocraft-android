//! `layer.setProps` on several layers at once (the Layers panel with several selected, as in
//! Photoshop), and Photoshop's History names for each kind of change.

use photocraft_engine::Session;
use serde_json::json;

fn doc() -> (Session, Vec<u64>) {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 32, "height": 32, "background": "white"})).unwrap();
    let ids = (0..3).map(|_| s.execute("layer.new.layer", json!({})).unwrap()["layer"].as_u64().unwrap()).collect();
    (s, ids)
}

fn layer(s: &Session, id: u64) -> photocraft_doc::Layer {
    s.active().unwrap().doc.layer(photocraft_doc::LayerId(id)).unwrap().clone()
}

fn last_step(s: &Session) -> String {
    s.active().unwrap().history.entries().last().cloned().unwrap_or_default()
}

#[test]
fn one_change_reaches_every_listed_layer_in_one_step() {
    let (mut s, ids) = doc();
    let steps = s.active().unwrap().history.entries().len();
    s.execute("layer.setProps", json!({"layers": [ids[0], ids[2]], "opacity": 0.4})).unwrap();
    assert_eq!(s.active().unwrap().history.entries().len(), steps + 1, "one step");
    assert_eq!(last_step(&s), "Opacity Change");
    assert_eq!((layer(&s, ids[0]).opacity, layer(&s, ids[1]).opacity, layer(&s, ids[2]).opacity), (0.4, 1.0, 0.4));
    s.execute("layer.setProps", json!({"layers": ids, "blend": "Multiply"})).unwrap();
    assert_eq!(last_step(&s), "Blending Change");
    assert!(ids.iter().all(|id| layer(&s, *id).blend == photocraft_color::BlendMode::Multiply));
    s.execute("layer.setProps", json!({"layers": [ids[1], ids[1], ids[2]], "locks": {"all": true}})).unwrap();
    assert!(layer(&s, ids[1]).locks.all && layer(&s, ids[2]).locks.all && !layer(&s, ids[0]).locks.all, "duplicates are harmless");
    s.execute("layer.setProps", json!({"layers": [ids[0]], "fill": 0.5})).unwrap();
    assert_eq!(last_step(&s), "Fill Opacity Change");
    s.execute("layer.setProps", json!({"layer": ids[0], "name": "Sky"})).unwrap();
    assert_eq!(last_step(&s), "Rename Layer");
    s.execute("layer.setProps", json!({"layer": ids[0], "opacity": 0.3, "fill": 0.3})).unwrap();
    assert_eq!(last_step(&s), "Layer Properties", "a mix keeps the general name");
}

#[test]
fn bad_layer_lists_change_nothing() {
    let (mut s, ids) = doc();
    let before = s.active().unwrap().doc.clone();
    for p in [
        json!({"layers": [], "opacity": 0.5}),
        json!({"layers": "all", "opacity": 0.5}),
        json!({"layers": ["x"], "opacity": 0.5}),
        json!({"layers": [ids[0], 999_999], "opacity": 0.5}),
        json!({"layers": [ids[0], ids[1]], "name": "Both"}),
        json!({"layers": [ids[0], ids[1]], "opacity": 2.0}),
    ] {
        assert!(s.execute("layer.setProps", p.clone()).is_err(), "{p}");
        assert_eq!(s.active().unwrap().doc.layers, before.layers, "{p}");
    }
}

#[test]
fn pass_through_is_for_groups_only() {
    let (mut s, ids) = doc();
    let g = s.execute("layer.new.group", json!({})).unwrap()["layer"].as_u64().unwrap();
    // A pixel layer, alone or with a group, can't take Pass Through; nothing changes.
    assert!(s.execute("layer.setProps", json!({"layer": ids[0], "blend": "Pass Through"})).is_err());
    let before = (layer(&s, ids[0]).blend, layer(&s, g).blend);
    assert!(s.execute("layer.setProps", json!({"layers": [g, ids[0]], "blend": "Pass Through"})).is_err());
    assert_eq!((layer(&s, ids[0]).blend, layer(&s, g).blend), before);
    s.execute("layer.setProps", json!({"layers": [g], "blend": "Normal"})).unwrap();
    s.execute("layer.setProps", json!({"layer": g, "blend": "Pass Through"})).unwrap();
    assert_eq!(layer(&s, g).blend, photocraft_color::BlendMode::PassThrough);
}
