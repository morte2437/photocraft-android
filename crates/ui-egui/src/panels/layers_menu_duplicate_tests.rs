//! The Layers panel menu's Duplicate Layer… (#14615): right after New Group…, opening the same
//! dialog as Layer › Duplicate Layer…, plural with several layers selected, greyed without a document.
use egui::vec2;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;

use crate::PhotocraftApp;

fn doc_app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 32, "height": 24})).unwrap();
    app.run("layer.new.layer", json!({"name": "Ink"})).unwrap();
    app
}

fn harness(app: PhotocraftApp) -> egui_kittest::Harness<'static, PhotocraftApp> {
    let mut h = egui_kittest::Harness::builder().with_size(vec2(260.0, 520.0)).build_ui_state(|ui, app: &mut PhotocraftApp| super::panel_menu(app, ui), app);
    h.run_steps(2);
    h
}

fn disabled(h: &egui_kittest::Harness<'static, PhotocraftApp>, label: &str) -> bool {
    h.get_by_label(label).accesskit_node().is_disabled()
}

fn top(h: &egui_kittest::Harness<'static, PhotocraftApp>, label: &str) -> f32 {
    h.get_by_label(label).rect().top()
}

fn layer_count(app: &PhotocraftApp) -> usize {
    app.session.active().map_or(0, |st| st.doc.walk().len())
}

#[test]
fn duplicate_layer_comes_right_after_new_group() {
    let h = harness(doc_app());
    let rows = ["New Layer…", "New Group…", "Duplicate Layer…", "New Group from Layers…", "Flatten Image"];
    let tops: Vec<f32> = rows.iter().map(|l| top(&h, l)).collect();
    assert!(tops.windows(2).all(|w| w[0] < w[1]), "{rows:?} in this order: {tops:?}");
    // Nothing sits between New Group… and Duplicate Layer…, nor between it and New Group from Layers….
    let between = |a: f32, b: f32| h.query_all_by_role(egui::accesskit::Role::Button).filter(|n| n.rect().top() > a && n.rect().top() < b).count();
    assert_eq!(between(tops[1], tops[2]), 0);
    assert_eq!(between(tops[2], tops[3]), 0);
}

#[test]
fn duplicate_layer_opens_the_same_dialog_as_the_layer_menu() {
    let mut h = harness(doc_app());
    let before = layer_count(h.state());
    h.get_by_label("Duplicate Layer…").click();
    h.run_steps(2);
    let app = h.state();
    assert!(!app.ui.status_error, "{}", app.ui.status);
    assert_eq!(layer_count(app), before, "nothing is duplicated before OK");
    let d = app.ui.dialogs.last().expect("the Duplicate Layer dialog");
    assert!(crate::duplicate_layer_ui::owns(&d.fields));
    assert_eq!(d.fields["__label"], json!("Duplicate Layer…"));
    assert_eq!(d.fields["name"], json!("Ink copy"));
    let fields = d.fields.clone();

    // Layer › Duplicate Layer… on the same document opens a dialog with the same fields.
    let ctx = egui::Context::default();
    let id = crate::menus::invoke(h.state_mut(), &ctx, "layer.duplicate", json!({})).unwrap()["dialog"].as_u64().unwrap();
    let from_menu = h.state().ui.dialogs.iter().find(|m| m.id == id).expect("the menu bar's dialog");
    assert_eq!(from_menu.fields, fields);
}

#[test]
fn several_selected_layers_read_duplicate_layers() {
    let mut app = doc_app();
    let st = app.session.active().unwrap();
    let background = st.doc.walk().into_iter().map(|(_, _, l)| l.id.0).find(|id| Some(*id) != st.active_layer.map(|a| a.0)).unwrap();
    app.run("layer.select", json!({"layer": background, "mode": "add"})).unwrap();
    assert_eq!(app.session.active().unwrap().selected_layers().len(), 2);
    let mut h = harness(app);
    assert!(h.query_by_label("Duplicate Layer…").is_none(), "the singular label is gone");
    assert!(!disabled(&h, "Duplicate Layers…"));
    assert!(top(&h, "New Group…") < top(&h, "Duplicate Layers…") && top(&h, "Duplicate Layers…") < top(&h, "New Group from Layers…"));
    h.get_by_label("Duplicate Layers…").click();
    h.run_steps(2);
    let d = h.state().ui.dialogs.last().expect("the Duplicate Layers dialog");
    assert_eq!(d.fields["__label"], json!("Duplicate Layers…"));
}

#[test]
fn duplicate_layer_is_enabled_exactly_when_the_layer_menu_item_is() {
    let no_doc = || PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    for make in [doc_app as fn() -> PhotocraftApp, no_doc] {
        let app = make();
        let on = crate::menus::is_enabled(&app, "layer.duplicate");
        let h = harness(app);
        assert_eq!(!disabled(&h, "Duplicate Layer…"), on);
    }
    assert!(disabled(&harness(no_doc()), "Duplicate Layer…"), "greyed without a document");
}
