use photocraft_doc::SliceOrigin;
use photocraft_engine::Session;
use serde_json::{Value, json};

fn sliced() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 40, "height": 30})).unwrap();
    s.execute("layer.new.layer", json!({"name": "logo"})).unwrap();
    s.execute("paint.stroke", json!({"points": [[5, 10], [20, 10]], "size": 8, "hardness": 1.0, "color": "#ff0000"})).unwrap();
    s.execute("layer.newLayerBasedSlice", json!({"url": "https://example.com", "alt": "logo"})).unwrap();
    s
}

fn layer_slice(s: &mut Session) -> Option<Value> {
    let list = s.execute("slice.list", json!({})).unwrap();
    list["slices"].as_array().unwrap().iter().find(|sl| sl["origin"] == "layer").cloned()
}

#[test]
fn duplicating_a_document_keeps_its_layer_based_slices() {
    for cmd in ["image.duplicate", "history.newDocument"] {
        let mut s = sliced();
        s.execute(cmd, json!({})).unwrap();
        let st = s.active().unwrap();
        let logo = st.doc.walk().into_iter().find(|(_, _, l)| l.name == "logo").unwrap().2.id;
        let slices: Vec<_> = st.doc.slices.list.iter().filter(|sl| sl.origin == SliceOrigin::Layer).collect();
        assert_eq!(slices.len(), 1, "{cmd}");
        assert_eq!(slices[0].layer, Some(logo), "{cmd}");
        assert_eq!((slices[0].url.as_str(), slices[0].alt.as_str()), ("https://example.com", "logo"), "{cmd}");
        let before = layer_slice(&mut s).unwrap()["rect"].clone();
        s.execute("layer.translate", json!({"dx": 5, "dy": 3})).unwrap();
        let after = layer_slice(&mut s).unwrap_or_else(|| panic!("{cmd} lost the slice after a move"))["rect"].clone();
        assert_ne!(before, after, "{cmd}: the slice no longer follows its layer");
    }
}
