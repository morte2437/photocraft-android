//! The Patch tool's Content-Aware Sample All Layers starts on (#12125): a fresh start and Reset
//! Tool give the Patch tool Sample All Layers on while other tools stay off, and saved settings
//! keep their stored value.

use serde_json::json;

use crate::PhotocraftApp;
use crate::state::{SampleAllLayers, Tool, ToolOptions};

fn app() -> PhotocraftApp {
    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width": 16, "height": 16})).unwrap();
    PhotocraftApp::new(s, crate::Services::default())
}

#[test]
fn patch_sample_all_layers_starts_on_and_other_tools_start_off() {
    let defaults = SampleAllLayers::default();
    assert!(defaults.patch);
    assert!(defaults.get(Tool::Patch));
    for tool in Tool::ALL {
        if tool != Tool::Patch {
            assert!(!defaults.get(tool), "{tool:?} should default to Sample All Layers off");
        }
    }

    let app = app();
    assert!(app.ui.tool_options.sample_all.patch);
}

#[test]
fn reset_tool_restores_patch_sample_all_layers_to_on() {
    let mut app = app();
    let ctx = egui::Context::default();
    app.ui.tool = Tool::Patch;
    app.ui.tool_options.sample_all.patch = false;
    app.ui.tool_options.sample_all.magic_wand = true;

    crate::menus::invoke(&mut app, &ctx, super::RESET, json!({})).unwrap();
    assert!(app.ui.tool_options.sample_all.patch, "Reset Tool restores Patch Sample All Layers to on");
    assert!(app.ui.tool_options.sample_all.magic_wand, "other tools keep their Sample All Layers setting");
}

#[test]
fn saved_settings_keep_stored_patch_sample_all_layers_value() {
    let saved_off: ToolOptions = serde_json::from_value(json!({
        "sample_all": {
            "patch": false,
            "magic_wand": true
        }
    }))
    .unwrap();
    assert!(!saved_off.sample_all.patch);
    assert!(saved_off.sample_all.magic_wand);

    let missing_patch: ToolOptions = serde_json::from_value(json!({
        "sample_all": {
            "magic_wand": true
        }
    }))
    .unwrap();
    assert!(missing_patch.sample_all.patch);
    assert!(missing_patch.sample_all.magic_wand);
}
