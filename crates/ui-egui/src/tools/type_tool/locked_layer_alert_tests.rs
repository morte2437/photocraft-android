//! #12040: a Type tool click on a text layer under Lock All, its own or an enclosing group's,
//! makes the layer active and shows the reference application's alert instead of starting an edit
//! the engine would refuse at the first keystroke. A position lock alone, and a click on empty
//! canvas, keep today's behaviour.

use super::*;
use crate::state::DialogKind;

const MESSAGE: &str = "Could not use the type tool because the layer is locked.";
/// A point on the glyphs of the text layer `locked_text` makes.
const ON_TEXT: (f64, f64) = (60.0, 95.0);
/// A point on the canvas well away from any text.
const OFF_TEXT: (f64, f64) = (300.0, 40.0);

/// A committed type layer reading "Locked text" at `ON_TEXT`, and an empty layer above it that
/// is active, so the click's activation shows.
fn locked_text(lock: serde_json::Value) -> (PhotocraftApp, LayerId) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 400, "height": 200})).unwrap();
    app.sync_views();
    app.ui.tool = crate::state::Tool::Type;
    pointer_up(&mut app, [50.0, 100.0], [50.0, 100.0]);
    insert(&mut app, "Locked text");
    commit(&mut app);
    let id = active_layer(&app).unwrap();
    app.run("layer.lockLayers", lock).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    assert_ne!(active_layer(&app), Some(id));
    assert_eq!(hit_layer(&mut app, ON_TEXT.0, ON_TEXT.1), Some(id), "the click lands on the text");
    (app, id)
}

fn active_layer(app: &PhotocraftApp) -> Option<LayerId> {
    app.session.active().and_then(|st| st.active_layer)
}

fn history_len(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().history.past_len()
}

fn error_dialogs(app: &PhotocraftApp) -> Vec<String> {
    app.ui.dialogs.iter().filter(|d| d.kind == DialogKind::Error).filter_map(|d| d.fields.get("message")?.as_str().map(String::from)).collect()
}

fn click(app: &mut PhotocraftApp, (x, y): (f64, f64)) -> bool {
    pointer_down(app, x, y, egui::Modifiers::NONE)
}

#[test]
fn a_click_on_a_text_layer_under_lock_all_alerts_instead_of_editing() {
    let (mut app, id) = locked_text(json!({"all": true}));
    let steps = history_len(&app);
    assert!(click(&mut app, ON_TEXT), "the click is the Type tool's");
    assert_eq!(active_layer(&app), Some(id), "the clicked layer becomes active");
    assert_eq!(error_dialogs(&app), [MESSAGE]);
    assert!(app.ui.text_edit.is_none(), "no edit starts");
    assert_eq!(history_len(&app), steps, "no history state");
    // OK closes the alert and changes nothing else (an alert's OK, and ↩, take the cancel path).
    let dialog = app.ui.dialogs[0].id;
    crate::dialogs::cancel(&mut app, dialog).unwrap();
    assert!(app.ui.dialogs.is_empty());
    assert!(app.ui.text_edit.is_none());
    assert_eq!(history_len(&app), steps);
    assert_eq!(current_text(&app, id).as_deref(), Some("Locked text"));
}

#[test]
fn lock_all_on_an_enclosing_group_alerts_too() {
    // No lock of the layer's own (`{}` would toggle Lock All on it).
    let (mut app, id) = locked_text(json!({"position": false}));
    app.run("layer.select", json!({"layer": id.0, "mode": "replace"})).unwrap();
    app.run("layer.groupLayers", json!({})).unwrap();
    // The new group is active; lock it, then make another layer active.
    app.run("layer.lockLayers", json!({"all": true})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    let steps = history_len(&app);
    assert!(click(&mut app, ON_TEXT));
    assert_eq!(active_layer(&app), Some(id));
    assert_eq!(error_dialogs(&app), [MESSAGE]);
    assert!(app.ui.text_edit.is_none());
    assert_eq!(history_len(&app), steps);
}

#[test]
fn a_position_lock_alone_still_edits() {
    let (mut app, id) = locked_text(json!({"position": true}));
    assert!(click(&mut app, ON_TEXT));
    assert!(error_dialogs(&app).is_empty());
    assert_eq!(app.ui.text_edit.as_ref().map(|e| e.layer), Some(id.0), "the click edits the text");
}

#[test]
fn a_click_on_empty_canvas_with_a_locked_text_layer_active_starts_new_text() {
    let (mut app, id) = locked_text(json!({"all": true}));
    app.run("layer.select", json!({"layer": id.0, "mode": "replace"})).unwrap();
    assert!(!click(&mut app, OFF_TEXT), "no text there: the press is left to start new text");
    pointer_up(&mut app, [OFF_TEXT.0, OFF_TEXT.1], [OFF_TEXT.0, OFF_TEXT.1]);
    assert!(error_dialogs(&app).is_empty());
    let edit = app.ui.text_edit.as_ref().expect("new text is being edited");
    assert!(edit.created);
    assert_ne!(edit.layer, id.0, "a new type layer");
}
