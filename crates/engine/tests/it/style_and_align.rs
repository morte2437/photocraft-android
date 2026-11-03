//! Copy / Paste Layer Style carries the whole style, Blending Options included (Blend If and the
//! channel restrictions); align with an active selection aligns to it however many layers are
//! selected (layers audit L2).

use photocraft_engine::Session;
use serde_json::json;

fn layer(s: &Session, id: u64) -> photocraft_doc::Layer {
    s.active().unwrap().doc.layer(photocraft_doc::LayerId(id)).unwrap().clone()
}

fn new_doc(s: &mut Session, mode: &str) -> (u64, u64) {
    s.execute("file.new", json!({"width": 64, "height": 64, "mode": mode, "background": "white"})).unwrap();
    let a = s.execute("layer.new.layer", json!({})).unwrap()["layer"].as_u64().unwrap();
    let b = s.execute("layer.new.layer", json!({})).unwrap()["layer"].as_u64().unwrap();
    (a, b)
}

#[test]
fn paste_layer_style_carries_blend_if_and_channels() {
    let mut s = Session::new();
    let (a, b) = new_doc(&mut s, "rgb");
    s.execute(
        "layer.layerStyle.blendingOptions",
        json!({"layer": a, "blendIf": [{"channel": "gray", "underlying": [0, 128]}, {"channel": "red", "thisLayer": [10, 200]}]}),
    )
    .unwrap();
    s.execute("layer.setProps", json!({"layer": a, "channels": [true, false, true]})).unwrap();
    s.execute("layer.layerStyle.copyLayerStyle", json!({"layer": a})).unwrap();
    s.execute("layer.layerStyle.pasteLayerStyle", json!({"layer": b})).unwrap();
    let (la, lb) = (layer(&s, a), layer(&s, b));
    assert_eq!(lb.blend_if, la.blend_if, "Blend If, Gray and channel ranges");
    assert_eq!(lb.excluded_channels, la.excluded_channels, "the unchecked G channel");
    // Into a CMYK document: only the Gray range carries over; channels reset to all.
    let (c, _) = new_doc(&mut s, "cmyk");
    s.execute("layer.layerStyle.pasteLayerStyle", json!({"layer": c})).unwrap();
    let lc = layer(&s, c);
    assert_eq!(lc.blend_if.get(0), la.blend_if.get(0));
    assert_eq!(lc.blend_if.ranges.len(), 1, "no RGB channel range on CMYK");
    assert_eq!(lc.excluded_channels, 0);
}

#[test]
fn paste_layer_style_into_another_mode_keeps_blend_if_normalised() {
    // A full Gray range and only a Red range: in CMYK nothing is left, so it is the default
    // (which compares equal, so the paste isn't seen as a Blend If change).
    let mut s = Session::new();
    let (a, _) = new_doc(&mut s, "rgb");
    s.execute("layer.layerStyle.blendingOptions", json!({"layer": a, "blendIf": [{"channel": "red", "thisLayer": [10, 200]}]})).unwrap();
    s.execute("layer.layerStyle.copyLayerStyle", json!({"layer": a})).unwrap();
    let (c, _) = new_doc(&mut s, "cmyk");
    s.execute("layer.layerStyle.pasteLayerStyle", json!({"layer": c})).unwrap();
    assert_eq!(layer(&s, c).blend_if, photocraft_doc::BlendIf::default());
}

#[test]
fn paste_layer_style_on_several_layers_is_one_undo_step() {
    // 16-bit, onto a pixel layer and a group: one History step, and one Undo restores both.
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 64, "height": 64, "depth": 16, "background": "white"})).unwrap();
    let a = s.execute("layer.new.layer", json!({})).unwrap()["layer"].as_u64().unwrap();
    s.execute("layer.layerStyle.blendingOptions", json!({"layer": a, "blendIf": [{"channel": "gray", "underlying": [0, 128]}]})).unwrap();
    s.execute("layer.setProps", json!({"layer": a, "fill": 0.5, "blend": "multiply"})).unwrap();
    s.execute("layer.layerStyle.copyLayerStyle", json!({"layer": a})).unwrap();
    let b = s.execute("layer.new.layer", json!({})).unwrap()["layer"].as_u64().unwrap();
    let g = s.execute("layer.new.group", json!({})).unwrap()["layer"].as_u64().unwrap();
    let before = (layer(&s, b), layer(&s, g));
    s.execute("layer.select", json!({"layer": b})).unwrap();
    s.execute("layer.select", json!({"layer": g, "mode": "add"})).unwrap();
    s.execute("layer.layerStyle.pasteLayerStyle", json!({})).unwrap();
    let la = layer(&s, a);
    for id in [b, g] {
        let l = layer(&s, id);
        assert_eq!((l.blend_if.clone(), l.fill_opacity, l.blend), (la.blend_if.clone(), la.fill_opacity, la.blend), "layer {id}");
    }
    s.execute("edit.undo", json!({})).unwrap();
    let after = (layer(&s, b), layer(&s, g));
    assert_eq!((after.0.blend_if, after.0.blend, after.1.blend_if, after.1.blend), (before.0.blend_if, before.0.blend, before.1.blend_if, before.1.blend));
    assert_eq!((after.0.fill_opacity, after.1.fill_opacity), (before.0.fill_opacity, before.1.fill_opacity));
}

#[test]
fn several_layers_align_to_an_active_selection() {
    let mut s = Session::new();
    let (a, b) = new_doc(&mut s, "rgb");
    for (id, x) in [(a, 4), (b, 30)] {
        s.execute("layer.select", json!({"layer": id})).unwrap();
        s.execute("select.rect", json!({"x": x, "y": 10, "width": 8, "height": 8})).unwrap();
        s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
    }
    s.execute("select.rect", json!({"x": 40, "y": 40, "width": 20, "height": 20})).unwrap();
    s.execute("layer.select", json!({"layer": a})).unwrap();
    s.execute("layer.select", json!({"layer": b, "mode": "add"})).unwrap();
    s.execute("layer.align.leftEdges", json!({})).unwrap();
    for id in [a, b] {
        let x0 = layer(&s, id).surface().unwrap().content_bounds().x0;
        assert_eq!(x0, 40, "layer {id} aligned to the selection's left edge");
    }
}

#[test]
fn a_groups_pass_through_pastes_onto_a_pixel_layer_as_normal() {
    let mut s = Session::new();
    let (a, _) = new_doc(&mut s, "rgb");
    let g = s.execute("layer.new.group", json!({})).unwrap()["layer"].as_u64().unwrap();
    assert_eq!(layer(&s, g).blend, photocraft_color::BlendMode::PassThrough);
    s.execute("layer.layerStyle.copyLayerStyle", json!({"layer": g})).unwrap();
    s.execute("layer.layerStyle.pasteLayerStyle", json!({"layer": a})).unwrap();
    assert_eq!(layer(&s, a).blend, photocraft_color::BlendMode::Normal, "Pass Through isn't a pixel layer's mode");
    // Onto another group it stays Pass Through.
    let g2 = s.execute("layer.new.group", json!({})).unwrap()["layer"].as_u64().unwrap();
    s.execute("layer.setProps", json!({"layer": g2, "blend": "Multiply"})).unwrap();
    s.execute("layer.layerStyle.pasteLayerStyle", json!({"layer": g2})).unwrap();
    assert_eq!(layer(&s, g2).blend, photocraft_color::BlendMode::PassThrough);
}
