//! A type edit ends when its layer stops being the one worked on (#6483): another layer or
//! another document became active. Photoshop commits the text then; left open, the session kept
//! taking single-key shortcuts as typing and could type into a layer of the wrong document.

use egui_kittest::Harness;
use photocraft_doc::{LayerContent, LayerId};
use serde_json::json;

use crate::PhotocraftApp;

fn new_app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 400, "height": 200})).unwrap();
    app.sync_views();
    app.ui.tool = crate::state::Tool::Type;
    app
}

fn text_of(app: &PhotocraftApp, doc: usize, id: LayerId) -> Option<String> {
    match &app.session.documents().get(doc)?.doc.layer(id)?.content {
        LayerContent::Text(t) => Some(t.text.clone()),
        _ => None,
    }
}

/// Start a new type layer with a click and type `s` into it, as the canvas does.
fn type_new(app: &mut PhotocraftApp, s: &str) -> LayerId {
    super::pointer_up(app, [50.0, 100.0], [50.0, 100.0]);
    super::insert(app, s);
    LayerId(app.ui.text_edit.as_ref().unwrap().layer)
}

/// Feed one frame of key handling with `events`.
fn keys(app: PhotocraftApp, events: Vec<egui::Event>) -> PhotocraftApp {
    let mut h = Harness::builder().build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            super::handle_keys(app, ui.ctx());
        },
        app,
    );
    h.run_steps(1);
    for e in events {
        h.event(e);
    }
    h.run_steps(1);
    h.into_state()
}

#[test]
fn typing_after_a_document_switch_never_edits_the_other_document() {
    let mut app = new_app();
    let id = app.run("type.create", json!({"text": "Bee", "size": 24, "x": 20, "y": 80})).unwrap()["layer"].as_u64().map(LayerId).unwrap();
    // A second document with the same layer ids, as when one .pcraft is opened twice (its saved
    // ids are kept; only the document id is made unique).
    let copy = (*app.session.active().unwrap().doc).clone();
    app.session.add_document(copy, None);
    app.sync_views();
    // Edit the copy's type layer (index 1), then switch to the first document's tab and type.
    crate::type_tool::edit_active(&mut app).unwrap();
    super::insert(&mut app, "Ay");
    assert!(app.session.set_active(0));
    super::end_if_left(&mut app);
    let app = keys(app, vec![egui::Event::Text("Z".into())]);
    assert_eq!(text_of(&app, 0, id).as_deref(), Some("Bee"), "the other document is untouched");
    assert_eq!(text_of(&app, 1, id).as_deref(), Some("Ay"), "the edited copy keeps its text");
    assert!(app.ui.text_edit.is_none(), "the copy's edit was committed");
    assert_eq!(app.session.active_index(), Some(0), "and the document switched to stays active");
}

#[test]
fn selecting_another_layer_commits_the_edit() {
    let mut app = new_app();
    let other = LayerId(app.run("layer.new.layer", json!({"name": "Other pixels"})).unwrap()["layer"].as_u64().unwrap());
    let text = type_new(&mut app, "Hello");
    super::end_if_left(&mut app);
    assert!(app.ui.text_edit.is_some(), "a session's own layer is active: it goes on");
    // The Layers panel menus, the canvas layer picker, ⌥[ / ⌥] and automation all select this way.
    app.run("layer.select", json!({"layer": other.0})).unwrap();
    super::end_if_left(&mut app);
    assert!(app.ui.text_edit.is_none());
    let st = app.session.active().unwrap();
    assert_eq!(st.active_layer, Some(other), "the selection stands");
    assert_eq!(st.doc.layer(text).unwrap().name, "Hello", "committed: the new layer is named after its text");
    assert_eq!(text_of(&app, 0, text).as_deref(), Some("Hello"));
}

#[test]
fn an_empty_new_layer_goes_when_another_layer_is_selected() {
    let mut app = new_app();
    let other = LayerId(app.run("layer.new.layer", json!({"name": "Other pixels"})).unwrap()["layer"].as_u64().unwrap());
    let text = type_new(&mut app, "");
    app.run("layer.select", json!({"layer": other.0})).unwrap();
    super::end_if_left(&mut app);
    assert!(app.ui.text_edit.is_none());
    assert!(app.session.active().unwrap().doc.layer(text).is_none(), "an empty new type layer is deleted on commit");
}

#[test]
fn deselecting_all_layers_commits_the_edit() {
    let mut app = new_app();
    type_new(&mut app, "Hello");
    app.run("select.deselectLayers", json!({})).unwrap();
    super::end_if_left(&mut app);
    assert!(app.ui.text_edit.is_none());
}

#[test]
fn a_type_mask_session_goes_on_while_its_layer_is_active() {
    let mut app = new_app();
    app.ui.tool = crate::state::Tool::HorizontalTypeMask;
    type_new(&mut app, "Mask");
    super::end_if_left(&mut app);
    let ed = app.ui.text_edit.as_ref().expect("the Type Mask session is still open");
    assert!(ed.mask);
}

#[test]
fn closing_the_edited_document_drops_the_edit_without_touching_another() {
    let mut app = new_app();
    let id = app.run("type.create", json!({"text": "Bee", "size": 24, "x": 20, "y": 80})).unwrap()["layer"].as_u64().map(LayerId).unwrap();
    let copy = (*app.session.active().unwrap().doc).clone();
    app.session.add_document(copy, None);
    app.sync_views();
    crate::type_tool::edit_active(&mut app).unwrap();
    super::insert(&mut app, "");
    assert!(app.session.close(1).is_some());
    let before = app.session.active().unwrap().revision;
    super::end_if_left(&mut app);
    assert!(app.ui.text_edit.is_none());
    assert_eq!(app.session.active().unwrap().revision, before, "nothing ran in the remaining document");
    assert_eq!(text_of(&app, 0, id).as_deref(), Some("Bee"), "its same-id type layer keeps its text");
}

/// The issue's second symptom: after a layer is selected, single-key tool shortcuts work again
/// instead of typing into the type layer.
#[test]
fn tool_shortcuts_work_after_another_layer_is_selected() {
    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width": 400, "height": 300})).unwrap();
    let mut h = Harness::builder().with_size(egui::vec2(1440.0, 900.0)).with_max_steps(64).build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        PhotocraftApp::new(s, crate::Services::default())
    });
    h.run_steps(8);
    let app = h.state_mut();
    app.ui.tool = crate::state::Tool::Type;
    let other = app.run("layer.new.layer", json!({"name": "Other pixels"})).unwrap()["layer"].as_u64().unwrap();
    let text = type_new(app, "Hello");
    app.run("layer.select", json!({"layer": other})).unwrap();
    h.run_steps(2);
    assert!(h.state().ui.text_edit.is_none(), "the frame commits the edit");
    h.key_press(egui::Key::V);
    h.event(egui::Event::Text("v".into()));
    h.run_steps(2);
    assert_eq!(h.state().ui.tool, crate::state::Tool::Move, "V picks the Move tool");
    assert_eq!(text_of(h.state(), 0, text).as_deref(), Some("Hello"), "and types nothing");
}

/// Ending an empty Type Mask undoes its session, and undo restores the layer selection from
/// before it: the layer just selected (or Deselect Layers) must stand. So must it when a Type
/// Mask with text becomes the selection and its working layer goes.
#[test]
fn ending_a_type_mask_keeps_the_new_layer_selection() {
    for (deselect, text) in [(false, ""), (true, ""), (false, "Mask"), (true, "Mask")] {
        let mut app = new_app();
        let first = LayerId(app.run("layer.new.layer", json!({"name": "First"})).unwrap()["layer"].as_u64().unwrap());
        let second = LayerId(app.run("layer.new.layer", json!({"name": "Second"})).unwrap()["layer"].as_u64().unwrap());
        app.run("layer.select", json!({"layer": first.0})).unwrap();
        app.ui.tool = crate::state::Tool::HorizontalTypeMask;
        let mask = type_new(&mut app, text);
        if deselect {
            app.run("select.deselectLayers", json!({})).unwrap();
        } else {
            app.run("layer.select", json!({"layer": second.0})).unwrap();
        }
        super::end_if_left(&mut app);
        assert!(app.ui.text_edit.is_none());
        let st = app.session.active().unwrap();
        assert!(st.doc.layer(mask).is_none(), "{text:?}: the mask's working layer is gone");
        assert_eq!(st.doc.selection.is_some(), !text.is_empty(), "{text:?}: text becomes the selection");
        let want = if deselect { None } else { Some(second) };
        assert_eq!(st.active_layer, want, "{text:?}, deselect={deselect}: the new selection stands");
        assert_eq!(st.selected_layers(), want.into_iter().collect::<Vec<_>>(), "{text:?}, deselect={deselect}");
    }
}
