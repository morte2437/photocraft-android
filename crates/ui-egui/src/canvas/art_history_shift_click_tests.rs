//! #12627: Shift-click with the Art History Brush paints a separate cluster at the new click
//! without connecting from the previous stroke, while Brush, Pencil, History Brush and the other
//! brush-like tools still connect on Shift-click.

use crate::PhotocraftApp;
use crate::canvas::{ToolEvent, tool_event};
use crate::state::Tool;
use serde_json::json;

/// Create a session whose initial history state is white and whose current canvas is black.
fn app_with_history_source() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    // State 0 ("New"): white 160x160 RGB canvas, which is the default history source.
    app.run("file.new", json!({"width": 160, "height": 160, "background": "white"})).unwrap();
    // Fill the canvas with black so any pixel restored from the white history source is > 0.
    app.run("tools.setColors", json!({"foreground": [0.0, 0.0, 0.0, 1.0]})).unwrap();
    app.run("edit.fill", json!({"contents": "foreground"})).unwrap();
    app.run("tools.setBrush", json!({"brush": {"size": 8, "hardness": 1.0}})).unwrap();
    app.session.tools.brush.smoothing.amount = 0.0;
    app.ui.tool_options.art_history_area = 10.0;
    app
}

/// Read the red channel of the active background layer at `(x, y)`.
fn red(app: &PhotocraftApp, x: i32, y: i32) -> f32 {
    app.session.documents()[0].doc.layers[0].surface().unwrap().rgba(x, y)[0]
}

/// Extract the `[x, y]` stroke points recorded by the latest journaled `cmd`.
fn last_points(app: &PhotocraftApp, cmd: &str) -> Vec<[f64; 2]> {
    let p = app.session.journal.iter().rev().find(|(id, _)| id == cmd).map(|(_, p)| p.clone()).unwrap();
    p["points"].as_array().unwrap().iter().map(|q| [q[0].as_f64().unwrap(), q[1].as_f64().unwrap()]).collect()
}

/// Clicking and then Shift-clicking with the Art History Brush paints only around the new click.
#[test]
fn art_history_brush_shift_click_paints_only_at_new_point_without_connecting_line() {
    let mut app = app_with_history_source();
    app.ui.tool = Tool::ArtHistoryBrush;
    let none = egui::Modifiers::NONE;
    let shift = egui::Modifiers::SHIFT;

    let before = app.session.documents()[0].history.past_len();
    // First click at (25, 25).
    tool_event(&mut app, ToolEvent::Down { x: 25.0, y: 25.0, pressure: 1.0 }, none);
    tool_event(&mut app, ToolEvent::Up { x: 25.0, y: 25.0 }, none);
    assert_eq!(app.session.documents()[0].history.past_len(), before + 1);
    assert_eq!(last_points(&app, "paint.artHistoryBrush"), vec![[25.0, 25.0]]);
    assert!(red(&app, 25, 25) > 0.5, "first click paints around (25, 25)");
    assert_eq!(crate::stroke_constraint::line_preview_start(&app, Tool::ArtHistoryBrush, true), None);

    // Shift-click at (135, 135): paints a separate cluster at (135, 135) with no line from (25, 25).
    tool_event(&mut app, ToolEvent::Down { x: 135.0, y: 135.0, pressure: 1.0 }, shift);
    tool_event(&mut app, ToolEvent::Up { x: 135.0, y: 135.0 }, shift);
    assert_eq!(app.session.documents()[0].history.past_len(), before + 2, "Shift-click adds one history state");
    assert_eq!(app.session.documents()[0].history.undo_label(), Some("Art History Brush"));
    assert_eq!(last_points(&app, "paint.artHistoryBrush"), vec![[135.0, 135.0]]);
    assert!(red(&app, 135, 135) > 0.5, "Shift-click paints around (135, 135)");
    for t in [50, 65, 80, 95, 110] {
        assert_eq!(red(&app, t, t), 0.0, "no connecting line painted at ({t}, {t})");
    }

    // Even when a previous Brush stroke recorded `last_stroke_end`, switching to Art History Brush
    // and Shift-clicking does not connect from that endpoint.
    let doc_id = app.session.active().unwrap().doc.id;
    app.last_stroke_end = Some((doc_id, [25.0, 135.0, 1.0]));
    tool_event(&mut app, ToolEvent::Down { x: 135.0, y: 25.0, pressure: 1.0 }, shift);
    tool_event(&mut app, ToolEvent::Up { x: 135.0, y: 25.0 }, shift);
    assert_eq!(last_points(&app, "paint.artHistoryBrush"), vec![[135.0, 25.0]]);
    assert_eq!(red(&app, 80, 80), 0.0);
}

/// Brush, Pencil, History Brush and the other brush-like tools still connect on Shift-click.
#[test]
fn shift_click_still_connects_brush_pencil_history_brush_and_other_painting_tools() {
    assert!(!crate::stroke_constraint::connects(Tool::ArtHistoryBrush));
    for tool in [
        Tool::Brush,
        Tool::Pencil,
        Tool::HistoryBrush,
        Tool::Eraser,
        Tool::CloneStamp,
        Tool::Healing,
        Tool::SpotHealing,
        Tool::PatternStamp,
        Tool::Blur,
        Tool::Sharpen,
        Tool::Smudge,
        Tool::Dodge,
        Tool::Burn,
        Tool::Sponge,
        Tool::MixerBrush,
        Tool::ColorReplacement,
    ] {
        assert!(crate::stroke_constraint::connects(tool), "{tool:?} should connect on Shift-click");
    }

    let none = egui::Modifiers::NONE;
    let shift = egui::Modifiers::SHIFT;
    for (tool, cmd) in [(Tool::Brush, "paint.stroke"), (Tool::Pencil, "paint.pencil"), (Tool::HistoryBrush, "paint.historyBrush")] {
        let mut app = app_with_history_source();
        app.run("tools.setColors", json!({"foreground": [1.0, 1.0, 1.0, 1.0]})).unwrap();
        app.ui.tool = tool;
        tool_event(&mut app, ToolEvent::Down { x: 20.0, y: 40.0, pressure: 1.0 }, none);
        tool_event(&mut app, ToolEvent::Up { x: 20.0, y: 40.0 }, none);
        tool_event(&mut app, ToolEvent::Down { x: 120.0, y: 40.0, pressure: 1.0 }, shift);
        tool_event(&mut app, ToolEvent::Up { x: 120.0, y: 40.0 }, shift);
        assert_eq!(last_points(&app, cmd), vec![[20.0, 40.0], [120.0, 40.0]], "{tool:?}");
        assert!(red(&app, 70, 40) > 0.9, "{tool:?} painted the connecting line midpoint");
    }
}
