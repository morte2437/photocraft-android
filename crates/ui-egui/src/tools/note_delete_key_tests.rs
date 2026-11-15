//! #11788: with the Note tool, ⌫/Delete on a selected note asks "Delete this note?" and deletes
//! the note, never the layer; the Notes panel's trash asks the same.

use super::*;

/// A document with a second layer and one note, the Note tool on and the note selected.
fn app() -> PhotocraftApp {
    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width": 64, "height": 48})).unwrap();
    s.execute("layer.new.layer", json!({"name": "Ink"})).unwrap();
    s.execute("notes.add", json!({"x": 10, "y": 12, "text": "fix this"})).unwrap();
    let mut app = PhotocraftApp::new(s, crate::Services::default());
    app.ui.tool = Tool::Note;
    app.ui.analysis.note_selected = Some(0);
    app
}

/// A key through the app's shortcut handling, as the canvas gets it.
fn press(app: &mut PhotocraftApp, key: egui::Key) {
    let ctx = egui::Context::default();
    let m = egui::Modifiers::NONE;
    let raw = egui::RawInput {
        events: vec![egui::Event::ModifiersChanged(m), egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: m }],
        ..Default::default()
    };
    ctx.begin_pass(raw);
    crate::shortcuts::handle(app, &ctx);
    ctx.end_pass().textures_delta.clear();
}

fn layers(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().doc.layers.len()
}

fn notes(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().doc.notes.len()
}

fn alert(app: &PhotocraftApp) -> Option<u64> {
    app.ui.dialogs.iter().find(|d| owns_delete_note(&d.fields)).map(|d| d.id)
}

#[test]
fn delete_with_a_note_selected_asks_and_deletes_the_note_not_the_layer() {
    for key in [egui::Key::Delete, egui::Key::Backspace] {
        let mut app = app();
        let steps = app.session.active().unwrap().history.past_len();
        press(&mut app, key);
        let id = alert(&app).unwrap_or_else(|| panic!("{key:?}: the alert"));
        assert_eq!((layers(&app), notes(&app)), (2, 1), "{key:?}: nothing changes before the answer");
        crate::dialogs::confirm(&mut app, id).unwrap();
        assert_eq!((layers(&app), notes(&app)), (2, 0), "{key:?}: the note goes, the layers stay");
        let st = app.session.active().unwrap();
        assert_eq!(st.history.past_len(), steps + 1);
        assert_eq!(st.history.undo_label(), Some("Delete Note"));
        assert_eq!(app.ui.analysis.note_selected, None, "nothing selected after");
    }
}

#[test]
fn no_keeps_the_document_and_history() {
    let mut app = app();
    let steps = app.session.active().unwrap().history.past_len();
    press(&mut app, egui::Key::Delete);
    let id = alert(&app).expect("the alert");
    crate::dialogs::cancel(&mut app, id).unwrap();
    assert_eq!((layers(&app), notes(&app)), (2, 1));
    assert_eq!(app.session.active().unwrap().history.past_len(), steps);
}

#[test]
fn dont_show_again_skips_the_alert_next_time() {
    let mut app = app();
    app.run("notes.add", json!({"x": 30, "y": 30})).unwrap();
    press(&mut app, egui::Key::Delete);
    let id = alert(&app).expect("the alert");
    app.ui.dialog_mut(id).unwrap().fields.insert("skip".into(), json!(true));
    crate::dialogs::confirm(&mut app, id).unwrap();
    assert_eq!(notes(&app), 1);
    // The panel's trash goes the same way: no alert, the note is deleted at once.
    ask_delete_note(&mut app, 0);
    assert!(alert(&app).is_none());
    assert_eq!((layers(&app), notes(&app)), (2, 0));
}

#[test]
fn the_notes_panel_trash_asks_too() {
    let mut app = app();
    ask_delete_note(&mut app, 0);
    assert!(alert(&app).is_some());
    assert_eq!(notes(&app), 1, "nothing goes before Yes");
}

#[test]
fn without_a_selected_note_delete_keeps_its_usual_route() {
    let mut app = app();
    app.ui.analysis.note_selected = None;
    press(&mut app, egui::Key::Delete);
    assert!(alert(&app).is_none());
    assert_eq!(notes(&app), 1, "the note stays");
    // Another tool with a note still selected: the note isn't asked about.
    let mut other = self::app();
    other.ui.tool = Tool::Move;
    press(&mut other, egui::Key::Delete);
    assert!(alert(&other).is_none());
    assert_eq!(notes(&other), 1);
}
