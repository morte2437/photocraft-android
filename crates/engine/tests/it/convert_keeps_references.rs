use photocraft_doc::{LayerId, SliceOrigin};
use photocraft_engine::Session;
use serde_json::json;

fn art() -> (Session, LayerId) {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 40, "height": 30})).unwrap();
    s.execute("layer.new.layer", json!({"name": "Art"})).unwrap();
    let id = s.active().unwrap().active_layer.unwrap();
    s.execute("paint.stroke", json!({"points": [[5, 10], [20, 10]], "size": 8, "hardness": 1.0, "color": "#ff0000"})).unwrap();
    s.execute("layer.newLayerBasedSlice", json!({})).unwrap();
    s.execute("image.variables.define", json!({"defs": [{"name": "v", "layer": id.0, "type": "visibility"}]})).unwrap();
    (s, id)
}

fn references_point_at(s: &Session, id: LayerId) -> bool {
    let doc = &s.active().unwrap().doc;
    let slice = doc.slices.list.iter().any(|sl| sl.origin == SliceOrigin::Layer && sl.layer == Some(id));
    let comps = doc.layer_comps.iter().all(|c| c.states.iter().any(|st| st.layer == id));
    let vars = doc.variables.defs.iter().all(|d| d.layer == id);
    slice && comps && vars
}

#[test]
fn converting_a_layer_keeps_its_slice_comps_and_variables() {
    for cmd in ["layer.smartObjects.convertToSmartObject", "filter.convertForSmartFilters"] {
        let (mut s, id) = art();
        s.execute("layerComp.new", json!({"name": "A", "visibility": true, "position": true, "appearance": true})).unwrap();
        s.execute(cmd, json!({})).unwrap();
        let smart = s.active().unwrap().active_layer.unwrap();
        assert!(references_point_at(&s, smart), "{cmd} left references on the old layer");
        s.execute("edit.undo", json!({})).unwrap();
        assert!(references_point_at(&s, id), "{cmd}: undo restores the original references");
    }
}

#[test]
fn a_comp_does_not_put_the_converted_layers_style_on_the_smart_object() {
    let (mut s, _) = art();
    s.execute("layer.layerStyle.dropShadow", json!({})).unwrap();
    let r = s.execute("layerComp.new", json!({"name": "A", "visibility": true, "position": true, "appearance": true})).unwrap();
    let comp = r["comp"].as_u64().unwrap();
    s.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
    let smart = s.active().unwrap().active_layer.unwrap();
    s.execute("layerComp.apply", json!({"comp": comp})).unwrap();
    assert!(s.active().unwrap().doc.layer(smart).unwrap().effects.items.is_empty(), "the style stays inside the smart object");
}
