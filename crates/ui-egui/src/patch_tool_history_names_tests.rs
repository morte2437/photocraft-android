//! The Patch tool's lasso is a History step named "Patch Tool" (#12117); the Lasso tool keeps "Lasso".

use egui::Modifiers;
use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::{ToolEvent, tool_event};
use crate::state::Tool;

fn app_with(tool: Tool) -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 800, "height": 600, "background": "transparent"})).unwrap();
    app.sync_views();
    app.ui.tool = tool;
    app
}

/// Drag a triangle with the active tool and return the last History step.
fn drag_lasso(app: &mut PhotocraftApp) -> Option<String> {
    tool_event(app, ToolEvent::Down { x: 100.0, y: 100.0, pressure: 1.0 }, Modifiers::NONE);
    for (x, y) in [(200.0, 100.0), (300.0, 100.0), (300.0, 300.0)] {
        tool_event(app, ToolEvent::Move { x, y, pressure: 1.0 }, Modifiers::NONE);
    }
    tool_event(app, ToolEvent::Up { x: 200.0, y: 300.0 }, Modifiers::NONE);
    assert!(app.session.active().unwrap().doc.selection.is_some());
    app.session.active().unwrap().history.entries().last().cloned()
}

#[test]
fn the_patch_tool_lasso_is_named_for_the_tool() {
    assert_eq!(drag_lasso(&mut app_with(Tool::Patch)).as_deref(), Some("Patch Tool"));
    assert_eq!(drag_lasso(&mut app_with(Tool::Lasso)).as_deref(), Some("Lasso"));
}

/// A Patch tool lasso keeps the selection icon in the History panel.
#[test]
fn the_patch_tool_step_has_the_selection_icon() {
    assert_eq!(crate::panels::history_step_icon("Patch Tool"), crate::panels::history_step_icon("Lasso"));
    assert_eq!(crate::panels::history_step_icon("Patch Tool"), "square-dashed");
    // Applying the patch edits pixels, so it does not take the selection icon from "Selection".
    assert_ne!(crate::panels::history_step_icon("Patch Selection"), "square-dashed");
}
