use serde_json::json;

use super::*;
use crate::menus::invoke as menu;

/// One 4 × 2 inch document at 100 ppi, with the rulers in `unit`.
fn app(unit: &str) -> (PhotocraftApp, egui::Context) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 400, "height": 200, "resolution": 100})).unwrap();
    app.run("prefs.set", json!({"path": "unitsAndRulers.rulers", "value": unit})).unwrap();
    app.sync_views();
    (app, egui::Context::default())
}

/// Open New Guide through the same menu entry used by the shell.
fn open(app: &mut PhotocraftApp, ctx: &egui::Context) -> u64 {
    menu(app, ctx, "view.newGuide", json!({})).unwrap()["dialog"].as_u64().unwrap()
}

/// Change the typed orientation and position without confirming the dialog.
fn set(app: &mut PhotocraftApp, d: u64, orientation: &str, position: &str) {
    let f = &mut app.ui.dialog_mut(d).unwrap().fields;
    f.insert("orientation".into(), json!(orientation));
    f.insert("position".into(), json!(position));
}

/// Read committed guide positions, excluding any local draft preview.
fn guides(app: &PhotocraftApp) -> (Vec<f64>, Vec<f64>) {
    let g = &app.session.active().unwrap().doc.guides;
    (g.horizontal.clone(), g.vertical.clone())
}

/// Guide positions accept explicit units and reopen in the current ruler unit.
#[test]
fn opens_in_the_ruler_unit_and_takes_photoshop_units() {
    let (mut app, ctx) = app("cm");
    let d = open(&mut app, &ctx);
    let f = &app.ui.dialog_mut(d).unwrap().fields;
    assert!(owns(f), "New Guide has its own dialog, not the generic one");
    assert_eq!((f["orientation"].as_str(), f["position"].as_str()), (Some("horizontal"), Some("0 cm")));
    // Percent is of the width for a vertical guide.
    set(&mut app, d, "vertical", "50%");
    crate::dialogs::confirm(&mut app, d).unwrap();
    assert_eq!(guides(&app), (vec![], vec![200.0]));
    // The last guide opens next time, in the ruler unit (200 px at 100 ppi is 5.08 cm); a bare
    // number is in the ruler unit, as in Photoshop.
    let d = open(&mut app, &ctx);
    let f = &app.ui.dialog_mut(d).unwrap().fields;
    assert_eq!((f["orientation"].as_str(), f["position"].as_str()), (Some("vertical"), Some("5.08 cm")));
    set(&mut app, d, "horizontal", "2.54");
    crate::dialogs::confirm(&mut app, d).unwrap();
    assert_eq!(guides(&app), (vec![100.0], vec![200.0]));
    // Any unit overrides the ruler unit.
    let d = open(&mut app, &ctx);
    set(&mut app, d, "horizontal", "36 pt");
    crate::dialogs::confirm(&mut app, d).unwrap();
    assert_eq!(guides(&app).0, vec![100.0, 50.0]);
}

/// Bare positions use percent when the ruler unit is percent.
#[test]
fn percent_rulers_open_at_zero_percent() {
    let (mut app, ctx) = app("percent");
    let d = open(&mut app, &ctx);
    assert_eq!(app.ui.dialog_mut(d).unwrap().fields["position"], "0 %");
    set(&mut app, d, "horizontal", "25");
    crate::dialogs::confirm(&mut app, d).unwrap();
    assert_eq!(guides(&app).0, vec![50.0], "25% of the 200 px height");
}

/// Invalid New Guide input is retained without recording a guide or preferences.
#[test]
fn a_refused_position_reopens_the_dialog_without_a_guide() {
    let (mut app, ctx) = app("pixels");
    let d = open(&mut app, &ctx);
    set(&mut app, d, "vertical", "12 furlongs");
    let err = crate::dialogs::confirm(&mut app, d).unwrap_err();
    assert!(err.contains("not a length"), "{err}");
    assert_eq!(guides(&app), (vec![], vec![]));
    let again = app.ui.dialogs.iter().find(|d| owns(&d.fields)).expect("the dialog is open again");
    assert_eq!(again.fields["position"], "12 furlongs", "what was typed is kept");
    assert_eq!(again.fields["orientation"], "vertical");
    assert_eq!(again.fields["__error"].as_str(), Some(err.as_str()));
    // A refused position is not remembered.
    assert!(!app.session.prefs().dialogs.contains_key("view.newGuide"));
}

/// Invalid saved orientation and position fall back to a usable new-guide dialog.
#[test]
fn remembered_values_that_no_longer_parse_are_ignored() {
    let (mut app, ctx) = app("mm");
    app.session.prefs.edit(|p| p.dialogs.insert("view.newGuide".into(), json!({"orientation": "diagonal", "positionPx": "far away"})));
    let d = open(&mut app, &ctx);
    let f = &app.ui.dialog_mut(d).unwrap().fields;
    assert_eq!((f["orientation"].as_str(), f["position"].as_str()), (Some("horizontal"), Some("0 mm")));
}

/// Read the colour stored on a committed guide.
fn color_of(app: &PhotocraftApp, vertical: bool, position: f64) -> Option<[u8; 3]> {
    app.session.active().unwrap().doc.guides.color(vertical, position)
}

/// Named and custom guide colours persist on the guide and in dialog preferences.
#[test]
fn guides_store_the_chosen_colour_cyan_by_default() {
    let (mut app, ctx) = app("pixels");
    let d = open(&mut app, &ctx);
    assert_eq!(app.ui.dialog_mut(d).unwrap().fields["color"], "cyan");
    set(&mut app, d, "vertical", "10");
    crate::dialogs::confirm(&mut app, d).unwrap();
    // Stored on the guide itself, so a later change of the preference colour leaves it alone.
    assert_eq!(color_of(&app, true, 10.0), Some([0x4a, 0xff, 0xff]));
    let d = open(&mut app, &ctx);
    set(&mut app, d, "vertical", "20");
    app.ui.dialog_mut(d).unwrap().fields.insert("color".into(), json!("magenta"));
    crate::dialogs::confirm(&mut app, d).unwrap();
    assert_eq!(color_of(&app, true, 20.0), Some([0xff, 0x00, 0xff]));
    // Custom… sends the picked colour; the list's choice and the custom colour open next time.
    let d = open(&mut app, &ctx);
    assert_eq!(app.ui.dialog_mut(d).unwrap().fields["color"], "magenta");
    set(&mut app, d, "horizontal", "30");
    let f = &mut app.ui.dialog_mut(d).unwrap().fields;
    f.insert("color".into(), json!("custom"));
    f.insert("customColor".into(), json!("#abcdef"));
    crate::dialogs::confirm(&mut app, d).unwrap();
    assert_eq!(color_of(&app, false, 30.0), Some([0xab, 0xcd, 0xef]));
    let d = open(&mut app, &ctx);
    let f = &app.ui.dialog_mut(d).unwrap().fields;
    assert_eq!((f["color"].as_str(), f["customColor"].as_str()), (Some("custom"), Some("#abcdef")));
}

/// Invalid saved colours fall back to the default cyan choice.
#[test]
fn remembered_colours_that_are_not_on_the_list_are_ignored() {
    let (mut app, ctx) = app("pixels");
    app.session.prefs.edit(|p| p.dialogs.insert("view.newGuide".into(), json!({"color": "puce", "customColor": "#zzzzzz"})));
    let d = open(&mut app, &ctx);
    let f = &app.ui.dialog_mut(d).unwrap().fields;
    assert_eq!((f["color"].as_str(), f["customColor"].as_str()), (Some("cyan"), Some("#4affff")));
}

/// A remembered pixel position is displayed in each current ruler unit.
#[test]
fn the_last_position_reopens_in_whatever_the_ruler_unit_is_now() {
    let (mut app, ctx) = app("pixels");
    let d = open(&mut app, &ctx);
    set(&mut app, d, "horizontal", "1in");
    crate::dialogs::confirm(&mut app, d).unwrap();
    for (unit, shown) in [("pixels", "100 px"), ("inches", "1 in"), ("mm", "25.4 mm"), ("points", "72 pt"), ("picas", "6 pica"), ("percent", "50 %")] {
        app.run("prefs.set", json!({"path": "unitsAndRulers.rulers", "value": unit})).unwrap();
        let d = open(&mut app, &ctx);
        assert_eq!(app.ui.dialog_mut(d).unwrap().fields["position"], shown, "{unit}");
        crate::dialogs::cancel(&mut app, d).unwrap();
    }
}

// These regressions exercise the ordinary dialog handlers, not a separate model.
/// Open a draft on the first of two guides and propose moving it to 30 px.
fn edit_fixture() -> (PhotocraftApp, u64) {
    let (mut app, _) = app("pixels");
    app.run("view.newGuide", json!({"orientation": "vertical", "position": 10})).unwrap();
    app.run("view.newGuide", json!({"orientation": "vertical", "position": 20})).unwrap();
    crate::guide_select::select(&mut app, true, 0);
    let id = open_edit(&mut app).unwrap();
    set(&mut app, id, "vertical", "30 px");
    (app, id)
}

/// A document switch rejects the old draft without touching the new document.
#[test]
fn draft_cannot_edit_a_guide_in_another_document() {
    let (mut app, id) = edit_fixture();
    app.run("file.new", json!({"width": 400, "height": 200})).unwrap();
    app.run("view.newGuide", json!({"orientation": "vertical", "position": 10})).unwrap();
    let history = app.session.active().unwrap().history.past_len();
    assert!(crate::dialogs::confirm(&mut app, id).is_err());
    assert_eq!(guides(&app).1, vec![10.0]);
    assert_eq!(app.session.active().unwrap().history.past_len(), history);
}

/// Deleting the target invalidates its draft even when another guide reuses the index.
#[test]
fn draft_cannot_edit_the_guide_that_reused_a_deleted_index() {
    let (mut app, id) = edit_fixture();
    app.run("view.deleteGuide", json!({"orientation": "vertical", "index": 0})).unwrap();
    let history = app.session.active().unwrap().history.past_len();
    assert!(crate::dialogs::confirm(&mut app, id).is_err());
    assert_eq!(guides(&app).1, vec![20.0]);
    assert_eq!(app.session.active().unwrap().history.past_len(), history);
}

/// Confirming unchanged values records no command and retains the redo stack.
#[test]
fn draft_unchanged_confirmation_preserves_redo() {
    let (mut app, id) = edit_fixture();
    crate::dialogs::cancel(&mut app, id).unwrap();
    app.run("view.moveGuide", json!({"orientation": "vertical", "index": 0, "position": 15})).unwrap();
    assert!(app.session.undo());
    crate::guide_select::select(&mut app, true, 0);
    let id = open_edit(&mut app).unwrap();
    let history = app.session.active().unwrap().history.past_len();
    crate::dialogs::confirm(&mut app, id).unwrap();
    assert_eq!(app.session.active().unwrap().history.past_len(), history);
    assert!(app.session.active().unwrap().history.can_redo());
}

/// Preview changes stay local; invalid input can be corrected into exactly one commit.
#[test]
fn draft_preview_is_isolated_and_invalid_text_keeps_the_last_good_preview() {
    let (mut app, id) = edit_fixture();
    crate::guide_edit::sync(&mut app);
    let shown = |app: &PhotocraftApp| edit_preview(app, &app.session.active().unwrap().doc).unwrap().vertical.clone();
    assert_eq!(shown(&app), vec![30.0, 20.0]);
    assert_eq!(guides(&app).1, vec![10.0, 20.0]);
    assert_eq!(app.session.active().unwrap().history.past_len(), 2);
    set(&mut app, id, "vertical", "far");
    crate::guide_edit::sync(&mut app);
    assert_eq!(shown(&app), vec![30.0, 20.0]);
    assert!(crate::dialogs::confirm(&mut app, id).is_err());
    let id = app.ui.dialogs.last().unwrap().id;
    assert!(app.ui.dialog_mut(id).unwrap().fields.contains_key("__error"));
    set(&mut app, id, "vertical", "40 px");
    crate::dialogs::confirm(&mut app, id).unwrap();
    assert_eq!(guides(&app).1, vec![40.0, 20.0]);
    assert_eq!(app.session.active().unwrap().history.past_len(), 3);
    assert_eq!(app.session.journal.iter().filter(|(cmd, _)| cmd == "view.editGuide").count(), 1);
    assert!(crate::dialogs::confirm(&mut app, id).is_err());
}

/// Opening another edit invalidates the earlier dialog; cancellation removes the draft.
#[test]
fn draft_reopening_retires_the_previous_dialog() {
    let (mut app, first) = edit_fixture();
    let second = open_edit(&mut app).unwrap();
    assert!(crate::dialogs::confirm(&mut app, first).is_err());
    crate::dialogs::cancel(&mut app, second).unwrap();
    assert!(app.guide_edit.is_none());
    assert_eq!(guides(&app).1, vec![10.0, 20.0]);
}

/// Locking guides invalidates an open edit before it can record a command.
#[test]
fn draft_cannot_commit_after_guides_are_locked() {
    let (mut app, id) = edit_fixture();
    app.ui.extras.lock_guides = true;
    assert!(crate::dialogs::confirm(&mut app, id).is_err());
    assert_eq!(guides(&app).1, vec![10.0, 20.0]);
    assert!(app.guide_edit.is_none());
}

/// A late policy refusal retains the typed proposal and history until a successful retry.
#[test]
fn refused_edit_command_preserves_the_draft_for_retry() {
    let (mut app, id) = edit_fixture();
    crate::dialogs::cancel(&mut app, id).unwrap();
    app.run("view.moveGuide", json!({"orientation": "vertical", "index": 0, "position": 15})).unwrap();
    assert!(app.session.undo());
    crate::guide_select::select(&mut app, true, 0);
    let id = open_edit(&mut app).unwrap();
    set(&mut app, id, "vertical", "30 px");
    let history = app.session.active().unwrap().history.past_len();
    let revision = app.session.active().unwrap().revision;
    app.automation_input = true;
    app.services.automation_command = Some(Box::new(|id, _| if id == "view.editGuide" { Err("guide edits are denied".into()) } else { Ok(()) }));
    let mut id = id;
    for _ in 0..2 {
        assert_eq!(crate::dialogs::confirm(&mut app, id).unwrap_err(), "guide edits are denied");
        assert_eq!(guides(&app).1, vec![10.0, 20.0]);
        assert_eq!(app.session.active().unwrap().history.past_len(), history);
        assert_eq!(app.session.active().unwrap().revision, revision);
        assert!(app.session.active().unwrap().history.can_redo());
        assert_eq!(app.session.journal.iter().filter(|(cmd, _)| cmd == "view.editGuide").count(), 0);
        crate::guide_edit::sync(&mut app);
        let again = app.ui.dialogs.iter().find(|d| d.fields.contains_key(EDIT)).expect("retry dialog remains open");
        id = again.id;
        assert_eq!(again.fields["position"], "30 px");
        assert_eq!(again.fields["orientation"], "vertical");
        assert_eq!(again.fields["__error"], "guide edits are denied");
        assert_eq!(edit_preview(&app, &app.session.active().unwrap().doc).unwrap().vertical, vec![30.0, 20.0]);
    }
    app.services.automation_command = None;
    crate::dialogs::confirm(&mut app, id).unwrap();
    assert_eq!(guides(&app).1, vec![30.0, 20.0]);
    assert_eq!(app.session.active().unwrap().history.past_len(), history + 1);
    assert_eq!(app.session.journal.iter().filter(|(cmd, _)| cmd == "view.editGuide").count(), 1);
    assert!(app.guide_edit.is_none());
    assert!(crate::dialogs::confirm(&mut app, id).is_err());
    assert!(app.session.undo());
    assert_eq!(guides(&app).1, vec![10.0, 20.0]);
    assert!(app.session.redo());
    assert_eq!(guides(&app).1, vec![30.0, 20.0]);
}

/// Restoring a refused proposal must not bind it to a different document at retry time.
#[test]
fn refused_edit_draft_cannot_follow_a_document_switch() {
    let (mut app, id) = edit_fixture();
    app.automation_input = true;
    app.services.automation_command = Some(Box::new(|_, _| Err("denied".into())));
    assert_eq!(crate::dialogs::confirm(&mut app, id).unwrap_err(), "denied");
    let id = app.ui.dialogs.last().unwrap().id;
    app.services.automation_command = None;
    app.run("file.new", json!({"width": 400, "height": 200})).unwrap();
    app.run("view.newGuide", json!({"orientation": "vertical", "position": 10})).unwrap();
    let history = app.session.active().unwrap().history.past_len();
    assert!(crate::dialogs::confirm(&mut app, id).is_err());
    assert_eq!(guides(&app).1, vec![10.0]);
    assert_eq!(app.session.active().unwrap().history.past_len(), history);
    assert!(app.guide_edit.is_none());
    assert!(!app.ui.dialogs.iter().any(|d| d.fields.contains_key(EDIT)));
}

/// Normal UI frames retain the command refusal; editing the position clears it.
#[test]
fn refused_edit_shows_the_error_and_typed_fields_in_the_dialog() {
    use egui_kittest::kittest::Queryable;
    let (mut app, id) = edit_fixture();
    app.automation_input = true;
    app.services.automation_command = Some(Box::new(|_, _| Err("guide edits are denied".into())));
    assert!(crate::dialogs::confirm(&mut app, id).is_err());
    let evidence = std::env::var_os("PHOTOCRAFT_GUIDE_RETRY_EVIDENCE");
    let builder = egui_kittest::Harness::builder().with_size(egui::vec2(1000.0, 700.0));
    let builder = if evidence.is_some() { builder.wgpu() } else { builder };
    let mut h = builder.build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        app
    });
    h.run_steps(3);
    h.get_by_label("guide edits are denied");
    assert_eq!(h.get_by_role_and_label(egui::accesskit::Role::TextInput, "Position").value().as_deref(), Some("30 px"));
    if let Some(path) = evidence {
        h.render().unwrap().save(std::path::Path::new(&path)).unwrap();
    }
    let id = h.state().ui.dialogs.last().unwrap().id;
    set(h.state_mut(), id, "vertical", "40 px");
    h.run_steps(3);
    assert!(h.query_by_label("guide edits are denied").is_none());
    assert_eq!(guides(h.state()).1, vec![10.0, 20.0]);
}
