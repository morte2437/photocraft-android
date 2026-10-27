use photocraft_doc::LayerMask;
use photocraft_engine::Session;
use serde_json::json;

const COMMANDS: [&str; 7] = [
    "layer.layerMask.revealAll",
    "layer.layerMask.hideAll",
    "layer.layerMask.revealSelection",
    "layer.layerMask.hideSelection",
    "layer.layerMask.delete",
    "layer.maskAllObjects",
    "image.adjustments.invert",
];

fn masked(all: bool) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 40, "height": 30})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.execute("paint.stroke", json!({"points": [[5, 10], [20, 10]], "size": 8, "hardness": 1.0, "color": "#ff0000"})).unwrap();
    s.execute("layer.layerMask.revealAll", json!({})).unwrap();
    s.execute("paint.stroke", json!({"points": [[25, 20], [35, 20]], "size": 6, "hardness": 1.0, "color": "#000000", "target": "mask"})).unwrap();
    s.execute("select.rect", json!({"x": 2, "y": 2, "width": 20, "height": 15})).unwrap();
    s.execute("layer.setProps", json!({"locks": {"all": all}})).unwrap();
    s
}

fn mask(s: &Session) -> Option<LayerMask> {
    let st = s.active().unwrap();
    st.doc.layer(st.active_layer.unwrap()).unwrap().mask.clone()
}

fn undo_depth(s: &Session) -> usize {
    s.active().unwrap().history.past_len()
}

#[test]
fn lock_all_refuses_every_mask_command() {
    for cmd in COMMANDS {
        let mut s = masked(true);
        let before = mask(&s);
        let undo = undo_depth(&s);
        let err = s.execute(cmd, json!({"target": "mask"})).unwrap_err();
        assert_eq!(err.to_string(), "Could not complete your request because the layer \"Layer 1\" is locked", "{cmd}");
        assert_eq!(mask(&s), before, "{cmd} left the mask unchanged");
        assert_eq!(undo_depth(&s), undo, "{cmd} added no history step");
    }
}

#[test]
fn unlocked_layers_still_take_every_mask_command() {
    for cmd in COMMANDS {
        let mut s = masked(false);
        let before = mask(&s);
        s.execute(cmd, json!({"target": "mask"})).unwrap_or_else(|e| panic!("{cmd}: {e}"));
        assert_ne!(mask(&s), before, "{cmd} changed the mask");
    }
}

#[test]
fn a_locked_layer_among_the_selected_layers_refuses_the_whole_mask() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 40, "height": 30})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    let first = s.active().unwrap().active_layer.unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.execute("layer.setProps", json!({"locks": {"all": true}})).unwrap();
    s.execute("layer.select", json!({"layer": first.0, "mode": "add"})).unwrap();
    assert_eq!(s.active().unwrap().selected_layers.len(), 2);
    assert!(s.execute("layer.layerMask.hideAll", json!({})).is_err());
    assert!(s.active().unwrap().doc.walk().iter().all(|(_, _, l)| l.mask.is_none()), "no layer got a mask");
}

#[test]
fn paste_into_a_lock_all_group_refuses_before_pasting() {
    for cmd in ["edit.pasteSpecial.pasteInto", "edit.pasteSpecial.pasteOutside"] {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 40, "height": 30})).unwrap();
        let child = s.execute("layer.new.layer", json!({"name": "Child"})).unwrap()["layer"].as_u64().unwrap();
        s.execute("paint.stroke", json!({"points": [[5, 10], [20, 10]], "size": 8, "hardness": 1.0, "color": "#ff0000"})).unwrap();
        s.execute("select.all", json!({})).unwrap();
        s.execute("edit.copy", json!({})).unwrap();
        let group = s.execute("layer.groupLayers", json!({})).unwrap()["layer"].as_u64().unwrap();
        s.execute("layer.setProps", json!({"layer": group, "locks": {"all": true}})).unwrap();
        s.execute("layer.select", json!({"layer": child, "mode": "replace"})).unwrap();
        s.execute("select.rect", json!({"x": 2, "y": 2, "width": 20, "height": 15})).unwrap();
        let layers = s.active().unwrap().doc.walk().len();
        let undo = undo_depth(&s);
        assert!(s.execute(cmd, json!({})).is_err(), "{cmd}");
        assert_eq!(s.active().unwrap().doc.walk().len(), layers, "{cmd} pasted no layer");
        assert_eq!(undo_depth(&s), undo, "{cmd} added no history step");
    }
}
