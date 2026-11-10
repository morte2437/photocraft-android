//! BUG-TYPE-TOOL-DEFAULT-FONT-SIZE (#12051): the Type tool starts at 50 pt.
//!
//! Measured in the reference application: after Reset All Tools the Type tool's size is 50 pt,
//! the same in an 800x600 document at 72 ppi, a 3000x2000 document at 300 ppi and a 100x100
//! document at 72 ppi, and the scripted tool options report `size=50` in points. A size saved by
//! an older version (which shipped 48 pt) is a remembered choice, not a default, and stays.

use serde_json::json;

use crate::PhotocraftApp;
use crate::state::{Tool, ToolOptions};

/// The size in points the scripted tool options report: `ui.inspect`'s `toolOptions` is
/// `ToolOptions` serialized (`control/inspect.rs`), and its serde name is `type_size`.
fn scripted_size(o: &ToolOptions) -> f64 {
    let v = serde_json::to_value(o).unwrap_or_default();
    v["type_size"].as_f64().unwrap_or_default()
}

/// An app with one document of the given size and resolution.
fn app_with_document(width: u32, height: u32, resolution: f64) -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": width, "height": height, "resolution": resolution})).unwrap();
    app.sync_views();
    app
}

#[test]
fn a_fresh_install_starts_the_type_tool_at_50_pt() {
    assert_eq!(ToolOptions::default().type_size, 50.0, "the shipped default is 50 pt, not 48 pt");
    assert_eq!(scripted_size(&ToolOptions::default()), 50.0, "the scripted tool options report size=50");
}

/// Reset Tool (the options bar's far-left menu) puts the field back to the default, and the
/// default is 50 pt for every flavour of the Type tool.
#[test]
fn reset_tool_gives_50_pt_for_every_type_tool() {
    for tool in [Tool::Type, Tool::VerticalType, Tool::HorizontalTypeMask, Tool::VerticalTypeMask] {
        let mut o = ToolOptions { type_size: 12.0, ..ToolOptions::default() };
        crate::tool_reset::reset_options(&mut o, tool);
        assert_eq!(o.type_size, 50.0, "{tool:?} › Reset Tool");
    }
}

#[test]
fn reset_tool_and_reset_all_tools_give_50_pt_in_the_running_app() {
    let ctx = egui::Context::default();
    for tool in [Tool::Type, Tool::VerticalType] {
        let mut app = app_with_document(800, 600, 72.0);
        app.ui.tool = tool;
        app.ui.tool_options.type_size = 12.0;
        crate::menus::invoke(&mut app, &ctx, crate::tool_reset::RESET, json!({})).unwrap();
        assert_eq!(app.ui.tool_options.type_size, 50.0, "{tool:?} › Reset Tool");
        assert_eq!(scripted_size(&app.ui.tool_options), 50.0, "{tool:?} › Reset Tool, scripted");

        app.ui.tool_options.type_size = 12.0;
        crate::menus::invoke(&mut app, &ctx, crate::tool_reset::RESET_ALL, json!({"confirmed": true})).unwrap();
        assert_eq!(app.ui.tool_options.type_size, 50.0, "{tool:?} › Reset All Tools");
    }
}

/// The default is a constant of the tool, not a function of the document: the measured reference
/// values are 800x600 at 72 ppi, 3000x2000 at 300 ppi and 100x100 at 72 ppi.
#[test]
fn the_default_size_does_not_depend_on_the_document() {
    for (width, height, dpi) in [(800u32, 600u32, 72.0f64), (3000, 2000, 300.0), (100, 100, 72.0)] {
        let app = app_with_document(width, height, dpi);
        assert_eq!(app.ui.tool_options.type_size, 50.0, "{width}x{height} at {dpi} ppi");
        assert_eq!(scripted_size(&app.ui.tool_options), 50.0, "{width}x{height} at {dpi} ppi: scripted tool options");
    }
}

/// A tool option saved by an older version is a remembered choice: a saved 48 stays 48.
#[test]
fn a_size_saved_by_an_older_version_keeps_its_value() {
    let mut restarted = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    restarted.session.prefs.edit(|p| p.tool_options = json!({ "type_size": 48.0 }));
    crate::tool_memory::restore(&mut restarted);
    assert_eq!(restarted.ui.tool_options.type_size, 48.0, "a saved 48 pt is not overwritten by the new default");
}

/// The default is also the fallback for a damaged remembered value (`tool_memory::sanitize`,
/// the second site #12051 names), so it follows it to 50 pt.
#[test]
fn a_damaged_saved_size_falls_back_to_50_pt() {
    let mut o = ToolOptions { type_size: f32::NAN, ..ToolOptions::default() };
    crate::tool_memory::sanitize(&mut o);
    assert_eq!(o.type_size, 50.0, "a damaged value takes the default");
}
