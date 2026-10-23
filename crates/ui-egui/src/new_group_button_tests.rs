//! The Layers panel New Group button groups multiple selected layers on a plain click.
//! Shift-click also groups a single selected layer; otherwise it creates an empty group.

use egui::Modifiers;
use egui_kittest::kittest::Queryable;
use photocraft_doc::{Layer, LayerId};
use serde_json::json;

use crate::PhotocraftApp;

/// Background, then `A` and `B` on top of it with both selected (`B` active).
fn two_selected() -> (egui_kittest::Harness<'static, PhotocraftApp>, LayerId, LayerId) {
    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
    let a = LayerId(s.execute("layer.new.layer", json!({"name": "A"})).unwrap()["layer"].as_u64().unwrap());
    let b = LayerId(s.execute("layer.new.layer", json!({"name": "B"})).unwrap()["layer"].as_u64().unwrap());
    s.execute("layer.select", json!({"layer": a.0})).unwrap();
    s.execute("layer.select", json!({"layer": b.0, "mode": "add"})).unwrap();
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1200.0, 800.0)).with_max_steps(64).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        PhotocraftApp::new(s, crate::Services::default())
    });
    h.run_steps(4);
    (h, a, b)
}

fn groups(h: &egui_kittest::Harness<'static, PhotocraftApp>) -> Vec<Layer> {
    h.state().session.active().unwrap().doc.walk().into_iter().map(|(_, _, l)| l).filter(|l| l.is_group()).cloned().collect()
}

#[test]
fn shift_click_groups_the_selected_layers_in_order() {
    let (mut h, a, b) = two_selected();
    h.get_by_label("Create a new group").click_modifiers(Modifiers::SHIFT);
    h.run_steps(4);
    let groups = groups(&h);
    assert_eq!(groups.len(), 1);
    let children: Vec<LayerId> = groups[0].children().unwrap().iter().map(|l| l.id).collect();
    assert_eq!(children, vec![a, b]);
    let doc = &h.state().session.active().unwrap().doc;
    assert_eq!(doc.layers.len(), 2, "background and the new group stay at the top level");
}

#[test]
fn plain_click_groups_all_selected_layers_in_order_and_undoes_in_one_step() {
    for active in ["A", "B"] {
        let (mut h, a, b) = two_selected();
        let id = if active == "A" { a } else { b };
        h.state_mut().session.execute("layer.select", json!({"layer": id.0, "mode": "add"})).unwrap();
        h.run_steps(4);
        h.get_by_label("Create a new group").click();
        h.run_steps(4);
        let groups = groups(&h);
        assert_eq!(groups.len(), 1);
        let group = &groups[0];
        let children: Vec<LayerId> = group.children().unwrap().iter().map(|l| l.id).collect();
        assert_eq!(children, vec![a, b]);
        let st = h.state().session.active().unwrap();
        assert_eq!(st.doc.layers.len(), 2, "only the background and group remain at the top level");
        assert_eq!(st.selected_layers(), vec![group.id]);

        h.state_mut().session.execute("edit.undo", json!({})).unwrap();
        let st = h.state().session.active().unwrap();
        assert_eq!(st.doc.layers.len(), 3);
        assert_eq!(st.doc.layers[1].id, a);
        assert_eq!(st.doc.layers[2].id, b);
        assert_eq!(st.active_layer, Some(id));

        h.state_mut().session.execute("edit.redo", json!({})).unwrap();
        let st = h.state().session.active().unwrap();
        let children: Vec<LayerId> = st.doc.layer(group.id).unwrap().children().unwrap().iter().map(|l| l.id).collect();
        assert_eq!(children, vec![a, b]);
    }
}

#[test]
fn plain_click_with_one_selected_layer_still_adds_an_empty_group() {
    let (mut h, a, b) = two_selected();
    h.state_mut().session.execute("layer.select", json!({"layer": a.0})).unwrap();
    h.run_steps(4);
    h.get_by_label("Create a new group").click();
    h.run_steps(4);
    let groups = groups(&h);
    assert_eq!(groups.len(), 1);
    assert!(groups[0].children().unwrap().is_empty());
    let doc = &h.state().session.active().unwrap().doc;
    let top: Vec<LayerId> = doc.layers.iter().map(|l| l.id).collect();
    assert_eq!(top[1], a, "the selected layer stays where it was");
    assert_eq!(top[3], b, "the unselected layer stays above the new group");
}
