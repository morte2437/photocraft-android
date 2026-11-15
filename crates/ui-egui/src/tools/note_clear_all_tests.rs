//! #11791: the Note tool's Clear All asks "Are you sure you want to delete all notes?" first. OK
//! deletes every note in one "Delete all Notes" step that one undo restores; Cancel changes
//! nothing; `notes.delete {"all": true}` itself never asks.

use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;

use super::*;

/// A 200 × 100 document with three notes, the second one selected, and the Note tool.
fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
    for (x, author) in [(10, "A"), (60, "B"), (110, "C")] {
        app.run("notes.add", json!({"x": x, "y": 20, "author": author})).unwrap();
    }
    app.ui.tool = Tool::Note;
    app.ui.analysis.note_selected = Some(1);
    app
}

fn harness(app: PhotocraftApp) -> Harness<'static, PhotocraftApp> {
    let mut h = Harness::builder().with_size(vec2(1200.0, 80.0)).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            if !ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            crate::panels::options_bar(app, ui);
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::Studio);
    h.run_steps(4);
    h
}

fn notes(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().doc.notes.len()
}

fn steps(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().history.past_len()
}

/// The open Clear All alert's id.
fn alert(app: &PhotocraftApp) -> Option<u64> {
    app.ui.dialogs.iter().find(|d| owns_clear_notes(&d.fields)).map(|d| d.id)
}

/// Click the options bar's Clear All and return the alert it opened.
fn clear_all(h: &mut Harness<'static, PhotocraftApp>) -> u64 {
    h.get_by_label("Clear All").click();
    h.run_steps(3);
    alert(h.state()).expect("Clear All opens the alert")
}

#[test]
fn clear_all_asks_first_and_cancel_changes_nothing() {
    let mut h = harness(app());
    let before = steps(h.state());
    let id = clear_all(&mut h);
    assert_eq!(notes(h.state()), 3, "nothing is deleted before the answer");
    let ctx = egui::Context::default();
    let mut texts = Vec::new();
    let mut o = ctx.run_ui(egui::RawInput::default(), clear_notes_body);
    o.textures_delta.clear();
    for c in o.shapes {
        if let egui::Shape::Text(t) = c.shape {
            texts.push(t.galley.text().to_owned());
        }
    }
    assert!(texts.iter().any(|t| t == "Are you sure you want to delete all notes?"), "{texts:?}");
    crate::dialogs::cancel(h.state_mut(), id).unwrap();
    assert_eq!(notes(h.state()), 3);
    assert_eq!(steps(h.state()), before, "no history step");
    assert_eq!(h.state().ui.analysis.note_selected, Some(1), "the selection stays");
}

#[test]
fn ok_deletes_every_note_in_one_step_that_undo_restores() {
    let mut h = harness(app());
    let before = steps(h.state());
    let id = clear_all(&mut h);
    crate::dialogs::confirm(h.state_mut(), id).unwrap();
    h.run_steps(3);
    let app = h.state();
    assert_eq!(notes(app), 0);
    assert_eq!(steps(app), before + 1, "one history step");
    assert_eq!(app.session.active().unwrap().history.undo_label(), Some("Delete all Notes"));
    assert_eq!(app.ui.analysis.note_selected, None, "nothing selected after OK");
    assert!(h.get_by_label("Clear All").accesskit_node().is_disabled(), "Clear All greys without notes");
    h.state_mut().run("edit.undo", json!({})).unwrap();
    assert_eq!(notes(h.state()), 3, "one undo brings every note back");
}

#[test]
fn the_command_itself_never_asks_and_names_its_step_too() {
    let mut app = app();
    app.run("notes.delete", json!({"all": true})).unwrap();
    assert_eq!(notes(&app), 0);
    assert!(alert(&app).is_none(), "no alert from the command");
    assert_eq!(app.session.active().unwrap().history.undo_label(), Some("Delete all Notes"));
}

#[test]
fn clear_all_stays_grey_without_notes() {
    let mut app = app();
    app.run("notes.delete", json!({"all": true})).unwrap();
    ask_clear_notes(&mut app);
    assert!(alert(&app).is_none(), "nothing to ask about");
    let h = harness(app);
    assert!(h.get_by_label("Clear All").accesskit_node().is_disabled());
}

#[test]
fn a_second_click_while_the_alert_is_open_opens_no_second_alert() {
    let mut h = harness(app());
    clear_all(&mut h);
    // The options bar alone, so nothing covers Clear All: the second click reaches the button.
    h.get_by_label("Clear All").click();
    h.run_steps(3);
    assert_eq!(h.state().ui.dialogs.iter().filter(|d| owns_clear_notes(&d.fields)).count(), 1);
}

#[test]
fn ok_after_the_document_changed_deletes_nothing() {
    let mut h = harness(app());
    let id = clear_all(&mut h);
    let app = h.state_mut();
    app.run("file.new", json!({"width": 50, "height": 50})).unwrap();
    app.run("notes.add", json!({"x": 5, "y": 5})).unwrap();
    let past: Vec<usize> = app.session.documents().iter().map(|d| d.history.past_len()).collect();
    assert!(crate::dialogs::confirm(app, id).is_err(), "the alert was for the first document");
    let docs = app.session.documents();
    assert_eq!(docs.iter().map(|d| d.doc.notes.len()).collect::<Vec<_>>(), [3, 1], "no note deleted in either document");
    assert_eq!(docs.iter().map(|d| d.history.past_len()).collect::<Vec<_>>(), past, "no history step in either");
}

/// The options bar with the open dialogs drawn over it, as the window shows them.
fn harness_with_dialogs(app: PhotocraftApp) -> Harness<'static, PhotocraftApp> {
    let mut h = Harness::builder().with_size(vec2(1200.0, 600.0)).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            if !ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            crate::panels::options_bar(app, ui);
            crate::dialogs::show(app, ui.ctx());
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::Studio);
    h.run_steps(4);
    h
}

#[test]
fn the_alert_shows_its_question_and_its_buttons_answer() {
    for (button, left) in [("Cancel", 3), ("OK", 0)] {
        let mut h = harness_with_dialogs(app());
        let before = steps(h.state());
        let id = clear_all(&mut h);
        let d = h.state().ui.dialogs.iter().find(|d| d.id == id).unwrap();
        assert_eq!(crate::dialogs::title(d), "Clear All");
        let question = egui_kittest::kittest::by().label("Are you sure you want to delete all notes?").include_labels();
        assert!(h.query_all(question).next().is_some(), "the question is shown");
        assert!(h.query_by_label("OK").is_some() && h.query_by_label("Cancel").is_some(), "OK and Cancel");
        h.get_by_label(button).click();
        h.run_steps(3);
        assert!(alert(h.state()).is_none(), "{button} closes the alert");
        assert_eq!(notes(h.state()), left, "{button}: {left} notes left");
        assert_eq!(steps(h.state()), before + usize::from(left == 0), "{button}: history");
    }
}

#[test]
fn enter_is_ok_and_escape_is_cancel() {
    for (key, left) in [(egui::Key::Escape, 3), (egui::Key::Enter, 0)] {
        let mut h = harness_with_dialogs(app());
        let before = steps(h.state());
        clear_all(&mut h);
        h.key_press(key);
        h.run_steps(3);
        assert!(alert(h.state()).is_none(), "{key:?} closes the alert");
        assert_eq!(notes(h.state()), left, "{key:?}: {left} notes left");
        assert_eq!(steps(h.state()), before + usize::from(left == 0), "{key:?}: history");
    }
}

#[test]
fn ok_brings_back_the_tool_defaults_in_the_options_bar() {
    let mut app = app();
    app.run("notes.set", json!({"index": 1, "color": "#3366cc"})).unwrap();
    // Clicking the note shows its author and colour in the options bar (#7940).
    show_note_options(&mut app, 1);
    assert_eq!(app.ui.analysis.note_author, "B");
    let mut h = harness(app);
    let id = clear_all(&mut h);
    crate::dialogs::confirm(h.state_mut(), id).unwrap();
    let (defaults, a) = (AnalysisUi::default(), &h.state().ui.analysis);
    assert_eq!((a.note_author.as_str(), a.note_color), (defaults.note_author.as_str(), defaults.note_color), "the tool defaults after OK");
}
