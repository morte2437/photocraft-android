//! #12112: in an Indexed Color document an ⌥-click and a stroke with the Healing Brush open the
//! reference application's alert with OK and change neither the clone source, the pixels nor the
//! History. RGB documents keep healing, and the Clone Stamp (not measured) keeps setting its source.

use egui::Modifiers;
use serde_json::{Value, json};

use super::tests::{app, stripes};
use super::*;
use crate::canvas::{ToolEvent, tool_event};
use crate::state::DialogKind;

const MESSAGE: &str = "Could not set the clone source because it does not work with Indexed Color images (convert image to RGB Color mode to edit).";

fn to_indexed(app: &mut PhotocraftApp) {
    app.run("image.mode.indexedColor", json!({"palette": "exact", "forced": "none", "dither": "none"})).unwrap();
}

fn click(app: &mut PhotocraftApp, x: f64, mods: Modifiers) {
    tool_event(app, ToolEvent::Down { x, y: 30.0, pressure: 1.0 }, mods);
    tool_event(app, ToolEvent::Up { x, y: 30.0 }, mods);
}

fn alerts(app: &PhotocraftApp) -> Vec<String> {
    app.ui.dialogs.iter().filter(|d| d.kind == DialogKind::Error).filter_map(|d| d.fields.get("message").and_then(Value::as_str).map(str::to_string)).collect()
}

/// The clone source (the options bar's and the engine's), the layer's pixels and the History length.
type State = (Option<[f64; 2]>, Option<[f64; 2]>, Vec<f32>, usize);

fn state(app: &PhotocraftApp) -> State {
    let st = app.session.active().unwrap();
    let pixels = st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap().read_region(st.doc.bounds());
    (app.ui.clone_source, app.session.presets.clone.active().source, pixels, st.history.past_len())
}

/// The alert is up once; OK closes it and leaves `before` as it was.
fn alerted_once(app: &mut PhotocraftApp, before: &State) {
    assert_eq!(alerts(app), [MESSAGE]);
    assert!(state(app) == *before, "the source, the pixels and the History are unchanged");
    let id = app.ui.dialogs.iter().find(|d| d.kind == DialogKind::Error).map(|d| d.id).unwrap();
    crate::dialogs::confirm(app, id).unwrap();
    assert!(alerts(app).is_empty());
    assert!(state(app) == *before, "OK changes nothing");
}

/// An Indexed Color document with stripes to heal, the Healing Brush active.
fn indexed() -> PhotocraftApp {
    let mut app = app();
    stripes(&mut app, 4, "pixels");
    to_indexed(&mut app);
    app.ui.tool = Tool::Healing;
    app
}

#[test]
fn an_alt_click_alerts_and_sets_no_source() {
    let mut app = indexed();
    let before = state(&app);
    click(&mut app, 70.0, Modifiers::ALT);
    alerted_once(&mut app, &before);
}

#[test]
fn a_click_alerts_and_heals_nothing() {
    let mut app = app();
    stripes(&mut app, 4, "pixels");
    app.ui.tool = Tool::Healing;
    // The source set while the document was RGB: the stroke must still be refused.
    click(&mut app, 70.0, Modifiers::ALT);
    to_indexed(&mut app);
    let before = state(&app);
    assert!(before.0.is_some(), "the source is set");
    tool_event(&mut app, ToolEvent::Down { x: 20.0, y: 30.0, pressure: 1.0 }, Modifiers::NONE);
    assert!(crate::canvas::live_retouch_points(&app).is_none(), "no live preview");
    tool_event(&mut app, ToolEvent::Up { x: 20.0, y: 30.0 }, Modifiers::NONE);
    alerted_once(&mut app, &before);
    // Without a source too: the same alert, not the status-bar hint.
    let mut app = indexed();
    let before = state(&app);
    click(&mut app, 20.0, Modifiers::NONE);
    assert!(!app.ui.status_error, "{}", app.ui.status);
    alerted_once(&mut app, &before);
}

#[test]
fn an_rgb_document_keeps_healing_and_the_clone_stamp_its_source() {
    let mut app = app();
    stripes(&mut app, 4, "pixels");
    app.ui.tool = Tool::Healing;
    click(&mut app, 70.0, Modifiers::ALT);
    let before = state(&app);
    tool_event(&mut app, ToolEvent::Down { x: 20.0, y: 30.0, pressure: 1.0 }, Modifiers::NONE);
    assert!(crate::canvas::live_retouch_points(&app).is_some(), "the live preview");
    tool_event(&mut app, ToolEvent::Up { x: 20.0, y: 30.0 }, Modifiers::NONE);
    assert!(alerts(&app).is_empty());
    assert_eq!(state(&app).3, before.3 + 1, "one Healing Brush state");
    // Not measured in the reference: the Clone Stamp's ⌥-click in an Indexed document is kept.
    let mut app = indexed();
    app.ui.tool = Tool::CloneStamp;
    click(&mut app, 70.0, Modifiers::ALT);
    assert!(alerts(&app).is_empty());
    assert_eq!(app.ui.clone_source, Some([70.0, 30.0]));
}
