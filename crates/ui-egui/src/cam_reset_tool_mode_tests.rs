//! Reset Tool on the Content-Aware Move tool keeps the Mode (#12183), as the reference app does;
//! Reset All Tools still resets it, and a tool preset still stores it.

use serde_json::json;

use crate::PhotocraftApp;
use crate::state::{Tool, ToolOptions};

fn app() -> PhotocraftApp {
    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width": 16, "height": 16})).unwrap();
    let mut app = PhotocraftApp::new(s, crate::Services::default());
    app.ui.tool = Tool::ContentAwareMove;
    app
}

/// Mode Extend, Structure 7, Color 5, Transform On Drop flipped.
fn customize(app: &mut PhotocraftApp) {
    let o = &mut app.ui.tool_options;
    o.cam_mode = "extend".into();
    o.cam_structure = 7.0;
    o.cam_color = 5.0;
    o.cam_transform_on_drop = !ToolOptions::default().cam_transform_on_drop;
}

#[test]
fn reset_tool_keeps_the_mode_and_resets_the_rest() {
    let mut app = app();
    customize(&mut app);
    app.ui.selection_mode = 2;
    crate::menus::invoke(&mut app, &egui::Context::default(), crate::tool_reset::RESET, json!({})).unwrap();
    let (o, d) = (&app.ui.tool_options, ToolOptions::default());
    assert_eq!(o.cam_mode, "extend");
    assert_eq!((o.cam_structure, o.cam_color), (4.0, 0.0));
    assert_eq!(o.cam_transform_on_drop, d.cam_transform_on_drop);
    assert_eq!(app.ui.selection_mode, 0);
}

#[test]
fn reset_all_tools_still_resets_the_mode_to_move() {
    let mut app = app();
    customize(&mut app);
    crate::menus::invoke(&mut app, &egui::Context::default(), crate::tool_reset::RESET_ALL, json!({"confirmed": true})).unwrap();
    let o = &app.ui.tool_options;
    assert_eq!(o.cam_mode, "move");
    assert_eq!((o.cam_structure, o.cam_color), (4.0, 0.0));
}

#[test]
fn a_content_aware_move_preset_still_stores_the_mode() {
    let keys = crate::tool_reset::option_keys(Tool::ContentAwareMove);
    for k in ["cam_mode", "cam_structure", "cam_color", "cam_transform_on_drop"] {
        assert!(keys.contains(&k), "{k}");
    }
}
