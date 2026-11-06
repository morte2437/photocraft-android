//! #10879: applying a Layers panel filter deselects the selected layers it hides, against the
//! cases measured in Photoshop.

use egui::vec2;
use egui_kittest::Harness;
use photocraft_doc::LayerId;
use serde_json::json;

use super::FilterBy;
use crate::PhotocraftApp;

/// "Red" and "Deep Pixel" (pixel layers) and "Hello" (type) over the Background, in the panel.
fn harness() -> Harness<'static, PhotocraftApp> {
    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width": 64, "height": 48})).unwrap();
    s.execute("layer.new.layer", json!({"name": "Red"})).unwrap();
    s.execute("layer.new.layer", json!({"name": "Deep Pixel"})).unwrap();
    let text = s.execute("type.create", json!({"text": "Hello", "x": 4, "y": 20, "size": 12})).unwrap()["layer"].as_u64().unwrap();
    s.execute("layer.setProps", json!({"layer": text, "name": "Hello"})).unwrap();
    let mut h = Harness::builder().with_size(vec2(1440.0, 900.0)).with_max_steps(64).build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        PhotocraftApp::new(s, crate::Services::default())
    });
    h.run_steps(8);
    h
}

fn id(h: &Harness<'_, PhotocraftApp>, name: &str) -> LayerId {
    h.state()
        .session
        .active()
        .unwrap()
        .doc
        .walk()
        .into_iter()
        .find(|(_, _, l)| l.name == name)
        .map(|(_, _, l)| l.id)
        .unwrap_or_else(|| panic!("no layer {name}"))
}

fn select(h: &mut Harness<'_, PhotocraftApp>, names: &[&str]) {
    for (i, name) in names.iter().enumerate() {
        let layer = id(h, name).0;
        h.state_mut().session.execute("layer.select", json!({"layer": layer, "mode": if i == 0 { "replace" } else { "add" }})).unwrap();
    }
    h.run_steps(4);
}

/// The selected layers' names, bottom to top, and the active layer's.
fn selection(h: &Harness<'_, PhotocraftApp>) -> (Vec<String>, Option<String>) {
    let st = h.state().session.active().unwrap();
    let name = |id: LayerId| st.doc.layer(id).unwrap().name.clone();
    (st.selected_layers().into_iter().map(name).collect(), st.active_layer.map(name))
}

fn none() -> (Vec<String>, Option<String>) {
    (Vec::new(), None)
}

#[test]
fn a_filter_that_hides_the_selected_layer_deselects_it_and_turning_it_off_keeps_that() {
    let mut h = harness();
    select(&mut h, &["Red"]);
    h.state_mut().ui.layer_filter = vec!["type".into()];
    h.run_steps(4);
    assert_eq!(selection(&h), none(), "Kind › Type hides Red: no layer is selected");
    h.state_mut().ui.layer_filter.clear();
    h.run_steps(4);
    assert_eq!(selection(&h), none(), "turning the filter off does not bring the selection back");
}

/// Batch 16 review: a filter already on when the row is first drawn (set before the panel drew,
/// as `ui.set {layerFilter}` can) deselects the layers it hides too.
#[test]
fn a_filter_on_before_the_rows_first_draw_deselects_the_hidden_layers() {
    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width": 64, "height": 48})).unwrap();
    s.execute("layer.new.layer", json!({"name": "Red"})).unwrap();
    s.execute("type.create", json!({"text": "Hello", "x": 4, "y": 20, "size": 12})).unwrap();
    let red = s.active().unwrap().doc.walk().into_iter().find(|(_, _, l)| l.name == "Red").map(|(_, _, l)| l.id).unwrap();
    s.execute("layer.select", json!({"layer": red.0})).unwrap();
    let mut h = Harness::builder().with_size(vec2(1440.0, 900.0)).with_max_steps(64).build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        let mut app = PhotocraftApp::new(s, crate::Services::default());
        app.ui.layer_filter = vec!["type".into()];
        app
    });
    h.run_steps(8);
    assert_eq!(selection(&h), none(), "Kind › Type hides Red from the first draw on");
}

#[test]
fn a_filter_that_lists_none_of_the_selected_layers_deselects_them_all() {
    let mut h = harness();
    select(&mut h, &["Hello", "Deep Pixel"]);
    h.state_mut().ui.layer_filter_by = FilterBy::SmartObject;
    h.state_mut().ui.layer_filter_smart = vec!["embedded".into()];
    h.run_steps(4);
    assert_eq!(selection(&h), none(), "Smart Object › Embedded matches neither");
}

#[test]
fn the_listed_layers_stay_selected_and_one_of_them_becomes_active() {
    let mut h = harness();
    select(&mut h, &["Red", "Hello", "Deep Pixel"]);
    assert_eq!(selection(&h).1.as_deref(), Some("Deep Pixel"));
    h.state_mut().ui.layer_filter = vec!["type".into()];
    h.run_steps(4);
    assert_eq!(selection(&h), (vec!["Hello".to_string()], Some("Hello".to_string())));
    let history = h.state().session.active().unwrap().history.entries().len();
    // Changing the criteria while the filter is on acts again: Name "zzzz" lists nothing.
    h.state_mut().ui.layer_filter_by = FilterBy::Name;
    h.state_mut().ui.layer_filter_name = "zzzz".into();
    h.run_steps(4);
    assert_eq!(selection(&h), none());
    assert_eq!(h.state().session.active().unwrap().history.entries().len(), history, "deselecting is no history step");
}

#[test]
fn a_layer_selected_while_the_filter_hides_it_stays_selected() {
    let mut h = harness();
    select(&mut h, &["Hello"]);
    h.state_mut().ui.layer_filter = vec!["type".into()];
    h.run_steps(4);
    assert_eq!(selection(&h).0, ["Hello"]);
    select(&mut h, &["Red"]);
    assert_eq!(selection(&h).0, ["Red"], "the filter only acts when it is applied");
}

#[test]
fn a_filter_set_through_the_control_channel_deselects_too() {
    let mut h = harness();
    select(&mut h, &["Red"]);
    let f = super::patched(&h.state().ui, &json!({"by": "name", "name": "hello"})).unwrap();
    super::apply(&mut h.state_mut().ui, f);
    h.run_steps(4);
    assert_eq!(selection(&h), none());
}
