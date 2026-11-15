//! Export As starts only from its Export button, as in Photoshop (#13740): Enter, with no text field
//! taking it, leaves the dialog open and asks for no file; Enter in a size field commits the value
//! typed there; Escape still cancels. Other dialogs keep Enter = OK.
use super::*;
use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use std::{cell::RefCell, rc::Rc};

/// Every save dialog the export asked for.
type Asked = Rc<RefCell<Vec<crate::file_dialog::FileDialogRequest>>>;

/// A 200 × 150 document with Export As open, in a harness that draws the dialogs and answers the
/// save dialog with Cancel. Returns the dialog's id and the log of save dialogs asked for, which
/// grows by one when the export starts.
fn export_as_open() -> (Harness<'static, PhotocraftApp>, u64, Asked) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 200, "height": 150})).unwrap();
    let (show, asked) = crate::file_dialog::fake(vec![None]);
    app.services.file_dialog = Some(show);
    let mut h = Harness::builder().with_size(egui::vec2(1000.0, 800.0)).build_ui_state(
        |ui, app| {
            crate::dialogs::show(app, ui.ctx());
            app.poll_file_dialog(ui.ctx(), None);
        },
        app,
    );
    // The dialog opens once the app's fonts are in: the harness draws a first frame as it is built.
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    let id = open(h.state_mut()).unwrap();
    h.run_steps(4);
    (h, id, asked)
}

/// Whether the dialog `id` is still open.
fn is_open(h: &Harness<'_, PhotocraftApp>, id: u64) -> bool {
    h.state().ui.dialogs.iter().any(|d| d.id == id)
}

/// The dialog's stored export size: scale (percent), width and height (pixels).
fn size(h: &Harness<'_, PhotocraftApp>, id: u64) -> (f64, f64, f64) {
    let f = &h.state().ui.dialogs.iter().find(|d| d.id == id).expect("Export As is open").fields;
    (n(f, "scale", -1.0), n(f, "width", -1.0), n(f, "height", -1.0))
}

/// Enter with no field being edited does nothing: the dialog stays open with its size, no save
/// dialog is asked for, and a second Enter changes nothing either. A click on Export then starts
/// the export (the save dialog is asked for once) and closes the dialog, so the harness would
/// have seen an export that Enter started.
#[test]
fn enter_leaves_export_as_open_and_exports_nothing() {
    let (mut h, id, asked) = export_as_open();
    assert!(h.query_by_role_and_label(Role::Button, "Export").is_some(), "the default button is Export");
    assert_eq!(size(&h, id), (100.0, 200.0, 150.0));
    for _ in 0..2 {
        h.key_press(egui::Key::Enter);
        h.run_steps(4);
        assert!(is_open(&h, id), "Enter keeps Export As open");
        assert!(asked.borrow().is_empty(), "Enter starts no export");
        assert_eq!(size(&h, id), (100.0, 200.0, 150.0));
    }
    h.get_by_role_and_label(Role::Button, "Export").click();
    h.run_steps(4);
    assert!(!is_open(&h, id), "Export closes the dialog");
    assert_eq!(asked.borrow().len(), 1, "Export asks where to save, once");
}

/// Enter in the Scale field commits the typed percent there (50 % of 200 × 150 is 100 × 75) and
/// leaves the dialog open with nothing exported.
#[test]
fn enter_in_the_scale_field_commits_it_and_keeps_the_dialog() {
    let (mut h, id, asked) = export_as_open();
    h.get_by_role_and_label(Role::SpinButton, "Scale").click();
    h.run_steps(1);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.run_steps(1);
    for ch in "50".chars() {
        h.event(egui::Event::Text(ch.to_string()));
        h.run_steps(1);
    }
    h.key_press(egui::Key::Enter);
    h.run_steps(4);
    assert!(is_open(&h, id), "Enter in a field keeps Export As open");
    assert!(asked.borrow().is_empty(), "Enter in a field starts no export");
    assert_eq!(size(&h, id), (50.0, 100.0, 75.0));
    assert_eq!(h.get_by_role_and_label(Role::SpinButton, "Scale").accesskit_node().numeric_value(), Some(50.0));
}

/// Escape still cancels Export As: the dialog closes and nothing is exported.
#[test]
fn escape_cancels_export_as() {
    let (mut h, id, asked) = export_as_open();
    h.key_press(egui::Key::Escape);
    h.run_steps(4);
    assert!(!is_open(&h, id), "Escape closes Export As");
    assert!(asked.borrow().is_empty(), "Escape starts no export");
}

/// Only Export As opts out of Enter = OK.
#[test]
fn only_export_as_opts_out_of_enter() {
    let mut f = Map::new();
    assert!(enter_confirms(&f), "a dialog without the Export As marker keeps Enter = OK");
    f.insert("__export".into(), json!(true));
    assert!(!enter_confirms(&f));
}
