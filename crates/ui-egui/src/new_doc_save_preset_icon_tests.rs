//! #14043: File › New saves a document preset from an icon right of Name, as Photoshop does. The
//! icon turns the pane into SAVE DOCUMENT PRESET, proposing the next free `Saved N` with its text
//! selected, with Save Preset / Cancel; the old "Save Preset…" button under the pane is gone.

use egui::accesskit::Role;
use egui_kittest::{Harness, kittest::Queryable};
use serde_json::json;

use crate::PhotocraftApp;
use crate::dialogs::new_doc_ui::next_saved_name;
use crate::state::{DialogKind, UiState};

/// The default New Document settings, as the dialog saves them.
fn settings() -> serde_json::Value {
    crate::dialogs::new_doc_ui::document_settings(&UiState::new_document_fields())
}

fn harness(saved: &[&str]) -> Harness<'static, PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    for name in saved {
        app.run("document.presets.save", json!({ "name": name, "settings": settings() })).unwrap();
    }
    let mut h =
        Harness::builder().with_size(egui::vec2(1400.0, 900.0)).with_step_dt(1.0 / 60.0).build_ui_state(|ui, app| crate::dialogs::show(app, ui.ctx()), app);
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    h.state_mut().ui.open_dialog(DialogKind::NewDocument, UiState::new_document_fields());
    h.run_steps(3);
    h
}

fn field(h: &Harness<'static, PhotocraftApp>, key: &str) -> Option<serde_json::Value> {
    h.state().ui.dialogs.first().and_then(|d| d.fields.get(key).cloned())
}

/// Click the icon beside Name and let the pane settle (the name field takes focus a frame later).
fn open_save(h: &mut Harness<'static, PhotocraftApp>) {
    h.get_by_label("Save Document Preset").click();
    h.run_steps(3);
}

#[test]
fn the_save_icon_sits_right_of_name_and_the_old_button_is_gone() {
    let h = harness(&[]);
    assert!(h.query_by_label("Save Preset…").is_none(), "no Save Preset… button under the pane");
    let name = h.get_by_role_and_label(Role::TextInput, "Name").rect();
    let icon = h.get_by_label("Save Document Preset").rect();
    assert!(icon.left() >= name.right(), "icon {icon:?} right of Name {name:?}");
    assert!(icon.top() < name.bottom() && name.top() < icon.bottom(), "icon {icon:?} on the Name row {name:?}");
}

#[test]
fn the_icon_opens_save_document_preset_proposing_saved_1_selected() {
    let mut h = harness(&[]);
    assert!(h.query_by_label("PRESET DETAILS").is_some());
    let create = h.get_by_label("Create").rect();
    open_save(&mut h);
    assert!(h.query_by_label("SAVE DOCUMENT PRESET").is_some());
    assert_eq!(h.get_by_label("Create").rect(), create, "the dialog keeps its size while saving");
    assert!(h.query_by_label("PRESET DETAILS").is_none(), "the header turns into SAVE DOCUMENT PRESET");
    assert!(h.query_by_label("Save Preset").is_some());
    assert!(h.query_by_label("Cancel").is_some());
    assert_eq!(field(&h, "__presetName"), Some(json!("Saved 1")));
    // The proposed name is selected: typing replaces it.
    h.event(egui::Event::Text("Banner".into()));
    h.run_steps(1);
    assert_eq!(field(&h, "__presetName"), Some(json!("Banner")));
    h.get_by_label("Save Preset").click();
    h.run_steps(3);
    let names: Vec<_> = h.state().session.presets.documents.iter().map(|p| p.name.clone()).collect();
    assert_eq!(names, ["Banner"]);
    assert!(h.query_by_label("PRESET DETAILS").is_some(), "saving goes back to Preset Details");
}

#[test]
fn the_proposal_is_the_next_free_saved_number() {
    let mut h = harness(&["Saved 1", "saved 2", "Saved 4"]);
    open_save(&mut h);
    assert_eq!(field(&h, "__presetName"), Some(json!("Saved 3")));
    h.get_by_label("Save Preset").click();
    h.run_steps(3);
    assert!(h.state().session.presets.documents.iter().any(|p| p.name == "Saved 3"));
    // The next time proposes the next free number again.
    open_save(&mut h);
    assert_eq!(field(&h, "__presetName"), Some(json!("Saved 5")));
}

#[test]
fn cancel_returns_to_preset_details_without_saving() {
    let mut h = harness(&[]);
    open_save(&mut h);
    h.event(egui::Event::Text("Draft".into()));
    h.run_steps(1);
    h.get_by_label("Cancel").click();
    h.run_steps(3);
    assert!(h.state().session.presets.documents.is_empty(), "Cancel saves nothing");
    assert!(h.query_by_label("PRESET DETAILS").is_some());
    assert!(h.query_by_label("SAVE DOCUMENT PRESET").is_none());
    assert_eq!(h.state().ui.dialogs.len(), 1, "Cancel keeps the New Document dialog open");
    // Opening it again proposes a fresh name, not the cancelled one.
    open_save(&mut h);
    assert_eq!(field(&h, "__presetName"), Some(json!("Saved 1")));
}

#[test]
fn next_saved_name_skips_taken_numbers_ignoring_case() {
    let mut s = photocraft_engine::Session::new();
    assert_eq!(next_saved_name(&s.presets.documents), "Saved 1");
    for name in ["SAVED 1", "Saved 3", "Poster"] {
        s.execute("document.presets.save", json!({ "name": name, "settings": settings() })).unwrap();
    }
    assert_eq!(next_saved_name(&s.presets.documents), "Saved 2");
}
