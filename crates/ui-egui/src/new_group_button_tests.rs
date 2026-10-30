//! #2561: the Layers panel New Group button. Like Photoshop CC 2014 and later, a click with two or
//! more layers selected puts them in a new, collapsed group, and with one layer it adds an empty
//! group. Shift-click (the older way) groups the selection too, ⌥-click asks in the New Group
//! dialog first and ⌘-click adds the empty group below the active layer.

use egui::Modifiers;
use egui_kittest::kittest::Queryable;
use photocraft_doc::{Layer, LayerContent, LayerId};
use serde_json::json;

use crate::PhotocraftApp;

/// Background, then `A` and `B` on top of it; `A` and `B` are selected when `both`, otherwise
/// only `B` is (`B` active either way).
fn selected(both: bool) -> (egui_kittest::Harness<'static, PhotocraftApp>, LayerId, LayerId) {
    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
    let a = LayerId(s.execute("layer.new.layer", json!({"name": "A"})).unwrap()["layer"].as_u64().unwrap());
    let b = LayerId(s.execute("layer.new.layer", json!({"name": "B"})).unwrap()["layer"].as_u64().unwrap());
    if both {
        s.execute("layer.select", json!({"layer": a.0})).unwrap();
        s.execute("layer.select", json!({"layer": b.0, "mode": "add"})).unwrap();
    }
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

/// The one group holds `[a, b]` in order, sits next to the background, is the only selected layer
/// and, as in Photoshop, is collapsed: its layers' rows are not drawn.
fn assert_grouped(h: &egui_kittest::Harness<'static, PhotocraftApp>, a: LayerId, b: LayerId) {
    let groups = groups(h);
    assert_eq!(groups.len(), 1);
    let children: Vec<LayerId> = groups[0].children().unwrap().iter().map(|l| l.id).collect();
    assert_eq!(children, vec![a, b]);
    let st = h.state().session.active().unwrap();
    assert_eq!(st.doc.layers.len(), 2, "background and the new group stay at the top level");
    assert_eq!(st.selected_layers(), vec![groups[0].id], "the new group is selected");
    assert!(matches!(&groups[0].content, LayerContent::Group(g) if !g.expanded), "the new group is collapsed");
    let rows: Vec<u64> = crate::layer_row_ui::recorded(&h.ctx).into_iter().map(|r| r.layer).collect();
    assert!(rows.contains(&groups[0].id.0) && !rows.contains(&a.0) && !rows.contains(&b.0), "only the group's row shows: {rows:?}");
}

#[test]
fn plain_click_groups_the_selected_layers_in_order() {
    let (mut h, a, b) = selected(true);
    h.get_by_label("Create a new group").click();
    h.run_steps(4);
    assert_grouped(&h, a, b);
    // One step: a single undo puts both layers back where they were.
    h.state_mut().run("edit.undo", json!({})).unwrap();
    h.run_steps(2);
    assert!(groups(&h).is_empty());
    let top: Vec<LayerId> = h.state().session.active().unwrap().doc.layers.iter().map(|l| l.id).collect();
    assert_eq!(top[1..], [a, b]);
}

#[test]
fn shift_click_groups_the_selected_layers_in_order() {
    let (mut h, a, b) = selected(true);
    h.get_by_label("Create a new group").click_modifiers(Modifiers::SHIFT);
    h.run_steps(4);
    assert_grouped(&h, a, b);
}

#[test]
fn plain_click_with_one_selected_layer_adds_an_empty_group() {
    let (mut h, a, b) = selected(false);
    h.get_by_label("Create a new group").click();
    h.run_steps(4);
    let groups = groups(&h);
    assert_eq!(groups.len(), 1);
    assert!(groups[0].children().unwrap().is_empty());
    let top: Vec<LayerId> = h.state().session.active().unwrap().doc.layers.iter().map(|l| l.id).collect();
    assert_eq!(top[1..], [a, b, groups[0].id], "the layers stay where they were; the group goes above the active one");
}

/// Selecting a layer inside the collapsed group opens it, as in Photoshop.
#[test]
fn selecting_a_grouped_layer_opens_the_group() {
    let (mut h, a, b) = selected(true);
    h.get_by_label("Create a new group").click();
    h.run_steps(4);
    assert_grouped(&h, a, b);
    h.state_mut().run("layer.select", json!({"layer": a.0})).unwrap();
    h.run_steps(4);
    let rows: Vec<u64> = crate::layer_row_ui::recorded(&h.ctx).into_iter().map(|r| r.layer).collect();
    assert!(rows.contains(&a.0) && rows.contains(&b.0), "{rows:?}");
}

/// ⌥-click asks in the New Group dialog first: New Group from Layers with several layers
/// selected, New Group with one. Nothing changes until OK.
#[test]
fn alt_click_opens_the_new_group_dialog() {
    for (both, command) in [(true, "layer.new.groupFromLayers"), (false, "layer.new.group")] {
        let (mut h, a, b) = selected(both);
        h.get_by_label("Create a new group").click_modifiers(Modifiers::ALT);
        h.run_steps(4);
        assert!(groups(&h).is_empty(), "nothing happens before OK");
        let (id, fields) = h.state().ui.dialogs.last().map(|d| (d.id, d.fields.clone())).expect("the New Group dialog opened");
        assert!(crate::new_group_ui::owns(&fields));
        assert_eq!(fields["__command"], command);
        crate::dialogs::confirm(h.state_mut(), id).unwrap();
        h.run_steps(4);
        if both {
            assert_grouped(&h, a, b);
        } else {
            let groups = groups(&h);
            assert_eq!(groups.len(), 1);
            assert!(groups[0].children().unwrap().is_empty());
        }
    }
}

/// ⌘-click adds an empty group below the active layer, even with several layers selected.
#[test]
fn command_click_adds_an_empty_group_below_the_active_layer() {
    for both in [true, false] {
        let (mut h, a, b) = selected(both);
        h.get_by_label("Create a new group").click_modifiers(Modifiers::COMMAND);
        h.run_steps(4);
        let groups = groups(&h);
        assert_eq!(groups.len(), 1);
        assert!(groups[0].children().unwrap().is_empty());
        let top: Vec<LayerId> = h.state().session.active().unwrap().doc.layers.iter().map(|l| l.id).collect();
        assert_eq!(top[1..], [a, groups[0].id, b]);
    }
}

/// Its neighbour, the New Layer button: ⌘-click adds the layer below the active one, a plain
/// click above it, as in Photoshop.
#[test]
fn command_click_on_new_layer_adds_it_below_the_active_layer() {
    for (modifiers, below) in [(Modifiers::COMMAND, true), (Modifiers::NONE, false)] {
        let (mut h, a, b) = selected(false);
        h.get_by_label("Create a new layer").click_modifiers(modifiers);
        h.run_steps(4);
        let top: Vec<LayerId> = h.state().session.active().unwrap().doc.layers.iter().map(|l| l.id).collect();
        assert_eq!(top.len(), 4);
        let new = *top.iter().find(|id| ![a, b].contains(id) && **id != top[0]).unwrap();
        assert_eq!(top[1..], if below { [a, new, b] } else { [a, b, new] });
    }
}

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
