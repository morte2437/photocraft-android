//! #12552: a Remove press on a layer with Lock All opens the reference application's alert with
//! OK at once and changes nothing: no stroke overlay, no job, no History state. Lock transparent
//! pixels still removes, and the Brush keeps its own wording.

use egui::Modifiers;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;

use super::brush_refusal_alert_tests::{active, alerts, app, refused, steps, stroke};
use super::*;
use crate::canvas::{ToolEvent, tool_event};

const WANT: &str = "Can't use the tool because the layer is locked.";

/// Both Remove modes: filled at each release, and collected for ↩.
#[test]
fn lock_all_opens_the_alert_at_the_press() {
    for after_each in [true, false] {
        let mut app = app(Tool::Remove);
        app.ui.tool_options.remove_after_each_stroke = after_each;
        app.run("layer.lockLayers", json!({"all": true})).unwrap();
        let journal = app.session.journal.len();
        tool_event(&mut app, ToolEvent::Down { x: 20.0, y: 32.0, pressure: 1.0 }, Modifiers::NONE);
        assert_eq!(alerts(&app), [WANT], "{after_each}: the alert at the press");
        tool_event(&mut app, ToolEvent::Move { x: 44.0, y: 32.0, pressure: 1.0 }, Modifiers::NONE);
        assert!(app.drag.is_none() && app.trail.is_none() && app.live_stroke.is_none(), "{after_each}: no stroke overlay");
        tool_event(&mut app, ToolEvent::Up { x: 44.0, y: 32.0 }, Modifiers::NONE);
        assert!(app.remove_pending.is_empty(), "{after_each}: no stroke collected");
        assert_eq!(app.session.journal.len(), journal, "{after_each}: no job ran");
        assert!(!app.ui.status_error, "{after_each}: no error notice as well: {}", app.ui.status);
        let id = app.ui.dialogs.iter().find(|d| d.kind == DialogKind::Error).map(|d| d.id).unwrap();
        crate::dialogs::confirm(&mut app, id).unwrap();
        // `refused` repeats the whole press: the alert once, no History state.
        refused(&mut app, WANT);
    }
}

#[test]
fn the_alert_has_one_button_ok() {
    let mut app = app(Tool::Remove);
    app.run("layer.lockLayers", json!({"all": true})).unwrap();
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(900.0, 600.0)).build_ui_state(|ui, app| crate::dialogs::show(app, ui.ctx()), app);
    // The press after the fonts are set up: the harness draws its first frame at build.
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    stroke(h.state_mut());
    h.run_steps(2);
    assert!(h.query_by_label(WANT).is_some(), "the message is shown");
    let buttons: Vec<String> = h.query_all_by_role(egui::accesskit::Role::Button).filter_map(|n| n.accesskit_node().label()).collect();
    // Off macOS the title bar also has its close box.
    let want: &[&str] = if crate::dialogs::CLOSE_BOX { &["Close", "OK"] } else { &["OK"] };
    assert_eq!(buttons, want);
    h.get_by_label("OK").click();
    h.run_steps(2);
    assert!(h.state().ui.dialogs.is_empty(), "OK closes it");
}

#[test]
fn lock_transparent_pixels_still_removes() {
    let mut app = app(Tool::Remove);
    app.run("layer.lockLayers", json!({"transparency": true})).unwrap();
    let before = steps(&app);
    stroke(&mut app);
    assert!(alerts(&app).is_empty());
    assert_eq!(app.session.journal.last().map(|(id, _)| id.as_str()), Some("paint.remove"));
    assert_eq!(steps(&app), before + 1, "the stroke was removed");
}

#[test]
fn the_brush_keeps_its_wording() {
    let mut app = app(Tool::Brush);
    app.run("layer.lockLayers", json!({"all": true})).unwrap();
    refused(&mut app, "Could not use the brush tool because the layer is locked.");
}

/// Not this alert: a hidden target is quiet (#12554), a group keeps its own alert (#10074) and
/// Lock Image Pixels alone doesn't refuse the press.
#[test]
fn unmeasured_cases_keep_todays_behaviour() {
    let mut app = app(Tool::Remove);
    let id = active(&app);
    app.run("layer.lockLayers", json!({"all": true})).unwrap();
    app.run("layer.setProps", json!({"layer": id, "visible": false})).unwrap();
    stroke(&mut app);
    assert!(alerts(&app).is_empty(), "hidden: {:?}", alerts(&app));
    assert!(!app.ui.status_error, "hidden: {}", app.ui.status);
    let mut app = self::app(Tool::Remove);
    app.run("layer.new.group", json!({})).unwrap();
    app.run("layer.lockLayers", json!({"all": true})).unwrap();
    stroke(&mut app);
    assert_eq!(alerts(&app), ["Could not use the Remove Tool because the target layer is a group."]);
    let mut app = self::app(Tool::Remove);
    app.run("layer.lockLayers", json!({"pixels": true})).unwrap();
    stroke(&mut app);
    assert!(alerts(&app).is_empty(), "Lock Image Pixels: {:?}", alerts(&app));
}

/// A type layer with Lock All: no alert, and today's status-bar error at the release (#12553 drops
/// only an unlocked type layer's press).
#[test]
fn a_locked_type_layer_keeps_todays_error() {
    let mut app = app(Tool::Remove);
    app.run("type.create", json!({"x": 10, "y": 32, "text": "Hello"})).unwrap();
    app.run("layer.lockLayers", json!({"all": true})).unwrap();
    let before = steps(&app);
    stroke(&mut app);
    assert!(alerts(&app).is_empty(), "{:?}", alerts(&app));
    assert!(app.ui.status_error && app.ui.status.contains("not a pixel layer"), "{}", app.ui.status);
    assert_eq!(steps(&app), before);
}
