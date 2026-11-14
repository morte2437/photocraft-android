//! #12553: a press and drag with the Remove tool on a type layer does nothing, as in the
//! reference: no rasterize prompt, no stroke overlay, no message and no history state; the layer
//! stays type. The other pixel tools, and the Remove tool on shape, Smart Object and fill
//! layers, still ask to rasterize first.

use egui::Modifiers;

use super::Kind;
use super::tests::{active_content, app_with, click, past, prompt};
use crate::canvas::{ToolEvent, tool_event};
use crate::state::Tool;

/// Both Remove modes: filled at each release, and collected for ↩ (no stroke is collected).
#[test]
fn a_remove_drag_on_a_type_layer_does_nothing() {
    for after_each in [true, false] {
        let mut app = app_with(Kind::Type);
        app.ui.tool = Tool::Remove;
        app.ui.tool_options.remove_after_each_stroke = after_each;
        let before = (past(&app), app.session.active().unwrap().doc.clone());
        tool_event(&mut app, ToolEvent::Down { x: 20.0, y: 40.0, pressure: 1.0 }, Modifiers::NONE);
        for i in 1..20 {
            tool_event(&mut app, ToolEvent::Move { x: 20.0 + f64::from(i) * 3.0, y: 40.0, pressure: 1.0 }, Modifiers::NONE);
            assert!(app.drag.is_none() && app.trail.is_none() && app.live_stroke.is_none(), "{after_each}: no stroke overlay while dragging");
        }
        tool_event(&mut app, ToolEvent::Up { x: 77.0, y: 40.0 }, Modifiers::NONE);
        assert!(app.ui.dialogs.is_empty(), "{after_each}: no prompt: {:?}", app.ui.dialogs.iter().map(|d| &d.fields).collect::<Vec<_>>());
        assert!(app.remove_pending.is_empty(), "{after_each}: no stroke collected");
        assert!(app.ui.status.is_empty(), "{after_each}: no message: {}", app.ui.status);
        assert_eq!(past(&app), before.0, "{after_each}: no history state");
        assert_eq!(*app.session.active().unwrap().doc, *before.1, "{after_each}: nothing changes");
        assert_eq!(active_content(&app), "Type");
    }
}

#[test]
fn brush_and_eraser_still_ask_on_a_type_layer() {
    for tool in [Tool::Brush, Tool::Eraser] {
        let mut app = app_with(Kind::Type);
        app.ui.tool = tool;
        click(&mut app, 30.0, 20.0);
        let d = prompt(&app).unwrap_or_else(|| panic!("{tool:?}: no prompt"));
        assert_eq!(d.fields["message"], serde_json::json!(Kind::Type.message()));
    }
}

#[test]
fn remove_still_asks_on_shape_smart_object_and_fill_layers() {
    for kind in [Kind::Shape, Kind::SmartObject, Kind::Fill] {
        let mut app = app_with(kind);
        app.ui.tool = Tool::Remove;
        let before = past(&app);
        click(&mut app, 30.0, 20.0);
        let d = prompt(&app).unwrap_or_else(|| panic!("{kind:?}: no prompt"));
        assert_eq!(d.fields["message"], serde_json::json!(kind.message()));
        assert_eq!(past(&app), before, "{kind:?}: nothing happens until the answer");
    }
}
