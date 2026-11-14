//! #12609: a Clone Stamp press before any Alt-click opens the reference application's alert with
//! OK at the press, instead of a status-bar line, and paints nothing. The Healing Brush keeps its
//! own behaviour.

use egui::Modifiers;
use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::{ToolEvent, tool_event};
use crate::control::{ControlRequest, Outcome, handle};
use crate::state::{DialogKind, Tool};
use crate::tools::brush_refusal::brush_refusal_alert_tests::{alerts, app, refused, steps, stroke};

const MESSAGE: &str = if cfg!(target_os = "macos") {
    "Could not use the clone stamp tool because the area to clone has not been defined (Option-click to define a source point)."
} else {
    "Could not use the clone stamp tool because the area to clone has not been defined (Alt-click to define a source point)."
};

/// The active layer's pixels.
fn pixels(app: &PhotocraftApp) -> Vec<f32> {
    let st = app.session.active().unwrap();
    st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap().read_region(st.doc.bounds())
}

#[test]
fn a_stroke_without_a_source_alerts_once_and_paints_nothing() {
    let mut app = app(Tool::CloneStamp);
    let before = pixels(&app);
    refused(&mut app, MESSAGE);
    assert!(!app.ui.status_error, "no status-bar error as well: {}", app.ui.status);
    assert!(pixels(&app) == before, "after OK the document is unchanged");
}

#[test]
fn the_alert_opens_at_the_press_and_the_drag_and_release_add_none() {
    let mut app = app(Tool::CloneStamp);
    tool_event(&mut app, ToolEvent::Down { x: 20.0, y: 32.0, pressure: 1.0 }, Modifiers::NONE);
    assert_eq!(alerts(&app), [MESSAGE], "at the press, before any move");
    assert!(app.drag.is_none() && app.live_stroke.is_none(), "the press starts no stroke");
    // Closed before the drag, so an alert from the move or the release would show.
    let id = app.ui.dialogs.iter().find(|d| d.kind == DialogKind::Error).map(|d| d.id).unwrap();
    crate::dialogs::confirm(&mut app, id).unwrap();
    tool_event(&mut app, ToolEvent::Move { x: 44.0, y: 32.0, pressure: 1.0 }, Modifiers::NONE);
    tool_event(&mut app, ToolEvent::Up { x: 44.0, y: 32.0 }, Modifiers::NONE);
    assert!(alerts(&app).is_empty(), "neither the drag nor the release alerts");
}

#[test]
fn a_second_press_while_the_alert_is_up_adds_none() {
    let mut app = app(Tool::CloneStamp);
    stroke(&mut app);
    stroke(&mut app);
    assert_eq!(alerts(&app), [MESSAGE]);
}

#[test]
fn the_alert_is_listed_and_confirmed_over_the_control_channel() {
    let ctx = egui::Context::default();
    let mut app = app(Tool::CloneStamp);
    stroke(&mut app);
    let (inspect, _rx) = ControlRequest::new("ui.inspect", json!({}));
    let Outcome::Done(inspect) = handle(&mut app, &ctx, &inspect) else { panic!("ui.inspect replies at once") };
    let dialog = &inspect["result"]["dialogs"][0];
    assert_eq!(dialog["fields"]["message"], MESSAGE, "{inspect}");
    let (confirm, _rx) = ControlRequest::new("ui.dialog.confirm", json!({"dialog": dialog["id"]}));
    assert!(matches!(handle(&mut app, &ctx, &confirm), Outcome::Done(v) if v["ok"] == true));
    assert!(alerts(&app).is_empty());
}

#[test]
fn after_an_alt_click_the_clone_stamp_paints_without_an_alert() {
    let mut app = app(Tool::CloneStamp);
    // A white band around the source on the red layer, so the clone has something to copy.
    app.run("select.rect", json!({"x": 0, "y": 0, "width": 64, "height": 20})).unwrap();
    app.run("edit.fill", json!({"color": "#ffffff"})).unwrap();
    app.run("select.deselect", json!({})).unwrap();
    tool_event(&mut app, ToolEvent::Down { x: 10.0, y: 10.0, pressure: 1.0 }, Modifiers::ALT);
    tool_event(&mut app, ToolEvent::Up { x: 10.0, y: 10.0 }, Modifiers::ALT);
    let before = steps(&app);
    stroke(&mut app);
    assert!(alerts(&app).is_empty());
    assert_eq!(steps(&app), before + 1, "one Clone Stamp state");
    let st = app.session.active().unwrap();
    let on_stroke = st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap().rgba(32, 32);
    assert!(on_stroke[1] > 0.5, "the stroke copies the white source onto the red: {on_stroke:?}");
}

#[test]
fn the_healing_brush_keeps_its_status_line() {
    let mut app = app(Tool::Healing);
    stroke(&mut app);
    assert!(alerts(&app).is_empty());
    assert!(app.ui.status_error && app.ui.status.contains("source point to clone from"), "{}", app.ui.status);
}
