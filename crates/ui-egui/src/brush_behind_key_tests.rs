//! Shift+Alt+Q selects Behind for Brush and Pencil (#11894).

use egui::{Context, Event, Key, Modifiers, RawInput};
use photocraft_color::BlendMode;
use photocraft_engine::Session;
use serde_json::json;

use crate::PhotocraftApp;
use crate::brush_mode_menu::{self, Item};
use crate::state::Tool;

/// Create a test document for the selected tool, optionally with a transparent layer.
fn app(tool: Tool, transparent_layer: bool) -> PhotocraftApp {
    let mut app = PhotocraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 32, "height": 32})).unwrap();
    if transparent_layer {
        app.run("layer.new.layer", json!({})).unwrap();
    }
    app.ui.tool = tool;
    app
}

/// Send Shift+Alt+Q through the shortcut handler and assert that it consumes the key.
fn press_behind(app: &mut PhotocraftApp, ctx: &Context) {
    let modifiers = Modifiers::SHIFT | Modifiers::ALT;
    let events = vec![Event::ModifiersChanged(modifiers), Event::Key { key: Key::Q, physical_key: Some(Key::Q), pressed: true, repeat: false, modifiers }];
    ctx.begin_pass(RawInput { events, ..Default::default() });
    crate::shortcuts::handle(app, ctx);
    assert!(!ctx.input(|i| i.events.iter().any(|e| matches!(e, Event::Key { key: Key::Q, .. }))));
    ctx.end_pass().textures_delta.clear();
}

/// Check whether the rendered options bar displays the Behind label.
fn options_show_behind(app: &mut PhotocraftApp, ctx: &Context) -> bool {
    /// Find the Behind text label in a shape or its descendants.
    fn has_behind(shape: &egui::Shape) -> bool {
        match shape {
            egui::Shape::Text(text) => text.galley.text() == "Behind",
            egui::Shape::Vec(shapes) => shapes.iter().any(has_behind),
            _ => false,
        }
    }
    let screen_rect = Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1400.0, 300.0)));
    let mut out = ctx.run_ui(RawInput { screen_rect, ..Default::default() }, |ui| crate::panels::options_bar(app, ui));
    out.textures_delta.clear();
    out.shapes.iter().any(|shape| has_behind(&shape.shape))
}

/// The key leaves Clear, uses one menu-equivalent command, and changes the displayed Mode.
#[test]
fn brush_and_pencil_select_behind_in_one_command() {
    for tool in [Tool::Brush, Tool::Pencil] {
        let mut app = app(tool, true);
        app.ui.tool_options.brush_clear = true;
        app.ui.tool_options.pencil_clear = true;
        app.session.tools.brush.mode = BlendMode::Multiply;
        let ctx = Context::default();
        PhotocraftApp::setup_context(&ctx, Default::default());
        assert!(brush_mode_menu::enabled(&app, Item::Behind));
        let (id, params) = brush_mode_menu::engine_call(&app, tool, Item::Behind).unwrap();
        assert_eq!((id, &params), ("tools.setBrush", &json!({"brush": {"mode": "Normal", "behind": true}})));
        let before = app.session.journal.len();
        press_behind(&mut app, &ctx);
        assert_eq!(&app.session.journal[before..], &[(id.to_owned(), params)]);
        assert_eq!(app.ui.brush_tool, tool);
        assert_eq!(app.ui.tool_options.brush_clear, tool != Tool::Brush);
        assert_eq!(app.ui.tool_options.pencil_clear, tool != Tool::Pencil);
        let brush = app.session.execute("brush.get", json!({})).unwrap();
        assert_eq!(brush["behind"], true);
        assert_eq!(brush["mode"], "Normal");
        assert!(options_show_behind(&mut app, &ctx), "{tool:?}");
    }
}

/// A shortcut on the start screen must leave both the live brush and its owning tool alone.
#[test]
fn brush_and_pencil_do_not_select_behind_without_a_document() {
    for (tool, brush_tool) in [(Tool::Brush, Tool::Pencil), (Tool::Pencil, Tool::Brush)] {
        let mut app = PhotocraftApp::new(Session::new(), Default::default());
        assert!(app.session.active().is_none());
        app.ui.tool = tool;
        app.ui.brush_tool = brush_tool;
        app.ui.tool_options.brush_clear = true;
        app.ui.tool_options.pencil_clear = true;
        app.session.tools.brush.mode = BlendMode::Multiply;
        let brush = app.session.execute("brush.get", json!({})).unwrap();
        let before = app.session.journal.len();
        press_behind(&mut app, &Context::default());
        assert_eq!(app.session.journal.len(), before);
        assert_eq!(app.session.execute("brush.get", json!({})).unwrap(), brush);
        assert_eq!(app.ui.brush_tool, brush_tool);
        assert!(app.ui.tool_options.brush_clear && app.ui.tool_options.pencil_clear);
    }
}

/// The same Background restriction used by the menu applies before the shortcut changes settings.
#[test]
fn background_does_not_accept_behind() {
    for tool in [Tool::Brush, Tool::Pencil] {
        let mut app = app(tool, false);
        app.ui.tool_options.brush_clear = true;
        app.ui.tool_options.pencil_clear = true;
        let ctx = Context::default();
        assert!(!brush_mode_menu::enabled(&app, Item::Behind));
        let before = app.session.journal.len();
        let brush_tool = app.ui.brush_tool;
        let brush = app.session.execute("brush.get", json!({})).unwrap();
        press_behind(&mut app, &ctx);
        assert_eq!(app.session.journal.len(), before);
        assert_eq!(app.session.execute("brush.get", json!({})).unwrap(), brush);
        assert_eq!(app.ui.brush_tool, brush_tool);
        assert!(app.ui.tool_options.brush_clear && app.ui.tool_options.pencil_clear);
    }
}

/// The consumed key stays a no-op for layers and for tools outside the measured scope.
#[test]
fn other_tools_keep_the_behind_key_as_a_no_op() {
    for tool in [Tool::Move, Tool::CloneStamp, Tool::HistoryBrush, Tool::Gradient, Tool::PaintBucket] {
        let mut app = app(tool, true);
        let ctx = Context::default();
        let before = app.session.journal.len();
        let brush_tool = app.ui.brush_tool;
        let active = app.session.active().unwrap();
        let layer = active.active_layer.unwrap();
        let blend = active.doc.layer(layer).unwrap().blend;
        press_behind(&mut app, &ctx);
        assert_eq!(app.session.journal.len(), before, "{tool:?}");
        assert_eq!(app.ui.tool, tool);
        assert_eq!(app.ui.brush_tool, brush_tool);
        assert_eq!(app.session.active().unwrap().doc.layer(layer).unwrap().blend, blend);
        assert!(!app.session.tools.brush.behind);
    }
}
