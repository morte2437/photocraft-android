//! Clip groups through Release Clipping Mask, Merge Down, Merge Layers and Delete Hidden Layers,
//! as in Photoshop (layers audit L2). The stack: Background, B, C1*, C2* (C1 and C2 clipped to B).

use photocraft_engine::Session;
use serde_json::json;

fn stack() -> (Session, [u64; 3]) {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 32, "height": 32, "background": "white"})).unwrap();
    let mut ids = [0u64; 3];
    for (i, color) in ["#ff0000", "#00ff00", "#0000ff"].iter().enumerate() {
        ids[i] = s.execute("layer.new.layer", json!({})).unwrap()["layer"].as_u64().unwrap();
        s.execute("select.rect", json!({"x": 4 * i, "y": 4, "width": 12, "height": 12})).unwrap();
        s.execute("edit.fill", json!({"color": color})).unwrap();
        s.execute("select.deselect", json!({})).unwrap();
    }
    for c in &ids[1..] {
        s.execute("layer.setProps", json!({"layer": c, "clipped": true})).unwrap();
    }
    (s, ids)
}

fn clipped(s: &Session, id: u64) -> bool {
    s.active().unwrap().doc.layer(photocraft_doc::LayerId(id)).unwrap().clipped
}

#[test]
fn releasing_a_middle_layer_releases_the_ones_above_it() {
    let (mut s, [_, c1, c2]) = stack();
    let r = s.execute("layer.releaseClippingMask", json!({"layer": c1})).unwrap();
    assert!(!clipped(&s, c1) && !clipped(&s, c2), "C2 isn't left clipped to the released C1: {r}");
    // Releasing the top one leaves the one below it alone.
    let (mut s, [_, c1, c2]) = stack();
    s.execute("layer.releaseClippingMask", json!({"layer": c2})).unwrap();
    assert!(clipped(&s, c1) && !clipped(&s, c2));
}

#[test]
fn merging_clipped_layers_keeps_the_result_clipped() {
    let (mut s, [_, c1, c2]) = stack();
    s.execute("layer.select", json!({"layer": c2})).unwrap();
    let r = s.execute("layer.mergeDown", json!({})).unwrap();
    let merged = r["layer"].as_u64().unwrap_or(c1);
    assert!(clipped(&s, merged), "C2 merged down into C1 stays clipped to B");
    let (mut s, [_, c1, c2]) = stack();
    s.execute("layer.select", json!({"layer": c1})).unwrap();
    s.execute("layer.select", json!({"layer": c2, "mode": "add"})).unwrap();
    let r = s.execute("layer.mergeLayers", json!({})).unwrap();
    assert!(clipped(&s, r["layer"].as_u64().unwrap()), "C1 + C2 merged stay clipped");
    // Clipped layers of two chains (Background, B, C1*, E, D*: C1 clipped to B, D to E) merge
    // into an ordinary layer: kept clipped at D's slot, C1's pixels would clip to E.
    let (mut s, [_, c1, e]) = stack();
    s.execute("layer.setProps", json!({"layer": e, "clipped": false})).unwrap();
    let d = s.execute("layer.new.layer", json!({})).unwrap()["layer"].as_u64().unwrap();
    s.execute("edit.fill", json!({"color": "#808080"})).unwrap();
    s.execute("layer.setProps", json!({"layer": d, "clipped": true})).unwrap();
    s.execute("layer.select", json!({"layer": c1})).unwrap();
    s.execute("layer.select", json!({"layer": d, "mode": "add"})).unwrap();
    let r = s.execute("layer.mergeLayers", json!({})).unwrap();
    assert!(!clipped(&s, r["layer"].as_u64().unwrap()), "C1 + D come from two clip chains");
    // Merging the base with its clipped layers gives an ordinary layer.
    let (mut s, [b, _, c2]) = stack();
    s.execute("layer.select", json!({"layer": b})).unwrap();
    s.execute("layer.select", json!({"layer": c2, "mode": "add"})).unwrap();
    let r = s.execute("layer.mergeLayers", json!({})).unwrap();
    assert!(!clipped(&s, r["layer"].as_u64().unwrap()));
}

#[test]
fn deleting_a_hidden_base_releases_its_clipped_layers() {
    let (mut s, [b, c1, c2]) = stack();
    s.execute("layer.setProps", json!({"layer": b, "visible": false})).unwrap();
    s.execute("layer.delete.hiddenLayers", json!({})).unwrap();
    assert!(!clipped(&s, c1) && !clipped(&s, c2), "they don't clip to the layer below the deleted base");
}
