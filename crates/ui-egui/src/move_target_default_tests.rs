//! The Move tool's Auto-Select target (#11380): fresh options and Tool › Reset Tool give Layer,
//! as the reference application's Reset Tool does; the dropdown lists Group, then Layer; a saved
//! choice is kept.

use egui::accesskit::Role;
use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;

use crate::PhotocraftApp;
use crate::state::{Tool, ToolOptions};

fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 64, "height": 64})).unwrap();
    app.sync_views();
    app.ui.tool = Tool::Move;
    app
}

#[test]
fn fresh_options_and_reset_tool_give_auto_select_on_with_the_layer_target() {
    let o = ToolOptions::default();
    assert!(o.move_auto_select);
    assert_eq!(o.move_target, "layer");
    let mut app = app();
    app.ui.tool_options.move_target = "group".into();
    app.ui.tool_options.move_auto_select = false;
    crate::menus::invoke(&mut app, &egui::Context::default(), crate::tools::tool_reset::RESET, json!({})).unwrap();
    assert!(app.ui.tool_options.move_auto_select);
    assert_eq!(app.ui.tool_options.move_target, "layer", "Reset Tool gives Layer");
}

#[test]
fn a_saved_choice_is_kept_and_only_a_missing_key_gets_layer() {
    let mut v = serde_json::to_value(ToolOptions::default()).unwrap();
    v["move_target"] = json!("group");
    let back: ToolOptions = serde_json::from_value(v.clone()).unwrap();
    assert_eq!(back.move_target, "group", "a saved Group stays Group");
    v.as_object_mut().unwrap().remove("move_target");
    let back: ToolOptions = serde_json::from_value(v).unwrap();
    assert_eq!(back.move_target, "layer");
}

#[test]
fn the_target_dropdown_lists_group_then_layer() {
    let mut h = Harness::builder().with_size(vec2(1600.0, 400.0)).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            if !ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            crate::panels::options_bar(app, ui);
        },
        app(),
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::Studio);
    h.run_steps(4);
    let combo = h
        .get_all_by_role(Role::ComboBox)
        .find(|n| n.accesskit_node().value().as_deref() == Some("Layer") || n.accesskit_node().label().as_deref() == Some("Layer"))
        .expect("the Auto-Select target dropdown shows Layer");
    let below = combo.rect().max.y;
    combo.click();
    h.run_steps(4);
    let item_y = |label: &str| {
        h.get_all_by_label(label)
            .map(|n| n.rect())
            .filter(|r| r.min.y >= below - 0.5)
            .map(|r| r.min.y)
            .next()
            .unwrap_or_else(|| panic!("a {label} entry in the open list"))
    };
    assert!(item_y("Group") < item_y("Layer"), "Group is listed above Layer");
}
