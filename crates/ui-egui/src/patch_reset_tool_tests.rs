//! Reset Tool on the Patch tool keeps Structure and Color (#12127), as the reference app does;
//! Reset All Tools still resets them, and a tool preset still stores them.

use serde_json::json;

use crate::PhotocraftApp;
use crate::state::{Tool, ToolOptions};

fn app() -> PhotocraftApp {
    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width": 16, "height": 16})).unwrap();
    let mut app = PhotocraftApp::new(s, crate::Services::default());
    app.ui.tool = Tool::Patch;
    app
}

/// Structure 7, Color 5, Destination mode, Content-Aware on.
fn customize(app: &mut PhotocraftApp) {
    let o = &mut app.ui.tool_options;
    o.patch_structure = 7.0;
    o.patch_color = 5.0;
    o.patch_mode = "destination".into();
    o.patch_content_aware = true;
}

#[test]
fn reset_tool_keeps_structure_and_color_and_resets_the_mode() {
    let mut app = app();
    customize(&mut app);
    app.ui.selection_mode = 2;
    crate::menus::invoke(&mut app, &egui::Context::default(), crate::tool_reset::RESET, json!({})).unwrap();
    let o = &app.ui.tool_options;
    assert_eq!((o.patch_structure, o.patch_color), (7.0, 5.0));
    assert_eq!(o.patch_mode, "source");
    assert!(!o.patch_content_aware);
    assert_eq!(app.ui.selection_mode, 0);
}

#[test]
fn reset_all_tools_still_gives_structure_4_and_color_0() {
    let mut app = app();
    customize(&mut app);
    crate::menus::invoke(&mut app, &egui::Context::default(), crate::tool_reset::RESET_ALL, json!({"confirmed": true})).unwrap();
    let o = &app.ui.tool_options;
    assert_eq!((o.patch_structure, o.patch_color), (4.0, 0.0));
    assert_eq!(o.patch_mode, "source");
}

#[test]
fn content_aware_move_reset_tool_resets_its_own_structure_and_color() {
    let mut app = app();
    app.ui.tool = Tool::ContentAwareMove;
    app.ui.tool_options.cam_mode = "extend".into();
    app.ui.tool_options.cam_structure = 7.0;
    app.ui.tool_options.cam_color = 5.0;
    app.ui.tool_options.patch_structure = 7.0;
    crate::menus::invoke(&mut app, &egui::Context::default(), crate::tool_reset::RESET, json!({})).unwrap();
    let d = ToolOptions::default();
    let o = &app.ui.tool_options;
    assert_eq!((o.cam_structure, o.cam_color), (d.cam_structure, d.cam_color));
    assert_eq!(o.patch_structure, 7.0, "another tool's option stays");
    assert_eq!(o.cam_mode, "extend", "Reset Tool keeps the Content-Aware Move Mode (#12183)");
}

#[test]
fn a_patch_tool_preset_still_stores_structure_and_color() {
    let keys = crate::tool_reset::option_keys(Tool::Patch);
    for k in ["patch_mode", "patch_content_aware", "patch_structure", "patch_color"] {
        assert!(keys.contains(&k), "{k}");
    }
}
