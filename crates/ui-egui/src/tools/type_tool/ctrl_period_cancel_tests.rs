//! #12076: ⌘. (Ctrl+. off the Mac) ends a type editing session the way the options bar's Cancel
//! does, with any of the four type tools: the document, the selection and the history are as
//! before the session. ⇧⌘. stays Increase Type Size; outside a session, and while an input
//! method is composing (not measured), ⌘. does nothing.
//! Keys go through the app's shortcut handling, as on the canvas.

use egui_kittest::Harness;
use photocraft_doc::{LayerContent, LayerId};
use photocraft_raster::Surface;
use serde_json::json;

use crate::PhotocraftApp;
use crate::state::Tool;

fn new_app(tool: Tool) -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 400, "height": 200})).unwrap();
    // A selection away from the text, which a Type Mask session must leave as it is.
    app.run("select.rect", json!({"x": 300, "y": 150, "width": 60, "height": 30})).unwrap();
    app.sync_views();
    app.ui.tool = tool;
    app
}

/// Start new text with a click and type `s`, as the canvas does.
fn type_new(app: &mut PhotocraftApp, s: &str) {
    super::pointer_up(app, [50.0, 100.0], [50.0, 100.0]);
    super::insert(app, s);
}

/// One frame of the app's shortcut handling for each of `events`.
fn send(app: PhotocraftApp, events: Vec<egui::Event>) -> PhotocraftApp {
    let mut h = Harness::builder().build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            let ctx = ui.ctx().clone();
            // As `canvas_tests`' harness: the shortcuts wait for the app's fonts.
            if ctx.fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                crate::shortcuts::handle(app, &ctx);
            }
        },
        app,
    );
    crate::PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    h.run_steps(4);
    for event in events {
        h.event(event);
        h.run_steps(1);
    }
    h.into_state()
}

fn key(key: egui::Key, modifiers: egui::Modifiers) -> egui::Event {
    egui::Event::Key { key, physical_key: Some(key), pressed: true, repeat: false, modifiers }
}

fn press(app: PhotocraftApp, k: egui::Key, modifiers: egui::Modifiers) -> PhotocraftApp {
    send(app, vec![key(k, modifiers)])
}

fn command_period(app: PhotocraftApp) -> PhotocraftApp {
    press(app, egui::Key::Period, egui::Modifiers::COMMAND)
}

/// The selection (compared pixel for pixel, shown by its bounds), the history (oldest first) and
/// the layers' ids.
#[derive(PartialEq)]
struct Snapshot {
    selection: Option<Surface>,
    history: Vec<String>,
    layers: Vec<LayerId>,
}

impl std::fmt::Debug for Snapshot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let bounds = self.selection.as_ref().map(Surface::content_bounds);
        write!(f, "selection {bounds:?}, history {:?}, layers {:?}", self.history, self.layers)
    }
}

fn snapshot(app: &PhotocraftApp) -> Snapshot {
    let st = app.session.active().unwrap();
    Snapshot { selection: st.doc.selection.clone(), history: st.history.entries(), layers: st.doc.layers.iter().map(|l| l.id).collect() }
}

fn text_of(app: &PhotocraftApp, id: LayerId) -> String {
    match &app.session.active().unwrap().doc.layer(id).unwrap().content {
        LayerContent::Text(t) => t.text.clone(),
        other => panic!("{}", other.kind_name()),
    }
}

fn size_of(app: &PhotocraftApp, id: LayerId) -> f32 {
    match &app.session.active().unwrap().doc.layer(id).unwrap().content {
        LayerContent::Text(t) => t.runs[0].style.size_pt,
        other => panic!("{}", other.kind_name()),
    }
}

#[test]
fn command_period_cancels_new_text_with_every_type_tool() {
    for tool in [Tool::Type, Tool::VerticalType, Tool::HorizontalTypeMask, Tool::VerticalTypeMask] {
        let mut app = new_app(tool);
        let before = snapshot(&app);
        type_new(&mut app, "AB");
        assert!(app.ui.text_edit.is_some(), "{tool:?}: editing");
        let app = command_period(app);
        assert!(app.ui.text_edit.is_none(), "{tool:?}: the session ends");
        assert_eq!(snapshot(&app), before, "{tool:?}: as before the session");
    }
}

#[test]
fn command_period_restores_edited_text() {
    for tool in [Tool::Type, Tool::VerticalType] {
        let mut app = new_app(tool);
        let orientation = if tool == Tool::VerticalType { "vertical" } else { "horizontal" };
        let made = app.run("type.create", json!({"text": "Hello", "x": 50, "y": 100, "orientation": orientation})).unwrap();
        let id = LayerId(made["layer"].as_u64().unwrap());
        let before = snapshot(&app);
        super::edit_active(&mut app).unwrap();
        super::insert(&mut app, "CD");
        assert_ne!(text_of(&app, id), "Hello", "{tool:?}: typed");
        let app = command_period(app);
        assert!(app.ui.text_edit.is_none(), "{tool:?}: the session ends");
        assert_eq!(text_of(&app, id), "Hello", "{tool:?}: the text is as before");
        assert_eq!(snapshot(&app), before, "{tool:?}: as before the session");
    }
}

#[test]
fn shift_command_period_still_increases_the_type_size() {
    let mut app = new_app(Tool::Type);
    type_new(&mut app, "AB");
    let id = LayerId(app.ui.text_edit.as_ref().unwrap().layer);
    let before = size_of(&app, id);
    let app = press(app, egui::Key::Period, egui::Modifiers::COMMAND | egui::Modifiers::SHIFT);
    assert!(app.ui.text_edit.is_some(), "still editing");
    assert!(size_of(&app, id) > before, "Increase Type Size ran");
}

/// Not measured, kept as before: while an input method is composing, ⌘. leaves the session.
#[test]
fn command_period_while_an_input_method_composes_keeps_the_session() {
    let mut app = new_app(Tool::Type);
    type_new(&mut app, "AB");
    let preedit = egui::Event::Ime(egui::ImeEvent::Preedit { text: "にほ".into(), active_range_chars: None });
    let app = send(app, vec![preedit, key(egui::Key::Period, egui::Modifiers::COMMAND)]);
    assert!(app.ui.text_edit.is_some(), "still editing");
}

#[test]
fn command_period_outside_a_type_session_changes_nothing() {
    for tool in [Tool::Type, Tool::HorizontalTypeMask] {
        let app = new_app(tool);
        let before = snapshot(&app);
        let app = command_period(app);
        assert!(app.ui.text_edit.is_none(), "{tool:?}");
        assert_eq!(snapshot(&app), before, "{tool:?}");
    }
}

/// Not measured, kept as before: once the composition text has gone but its committed text has
/// not arrived yet, ⌘. still leaves the session, as it does while composing.
#[test]
fn command_period_before_a_composition_commits_keeps_the_session() {
    let mut app = new_app(Tool::Type);
    type_new(&mut app, "AB");
    let preedit = |text: &str| egui::Event::Ime(egui::ImeEvent::Preedit { text: text.into(), active_range_chars: None });
    let app = send(app, vec![preedit("にほ"), preedit(""), key(egui::Key::Period, egui::Modifiers::COMMAND)]);
    assert!(app.ui.text_edit.is_some(), "still editing");
}

#[test]
fn command_period_after_a_composition_commits_cancels() {
    let mut app = new_app(Tool::Type);
    let before = snapshot(&app);
    type_new(&mut app, "AB");
    let preedit = egui::Event::Ime(egui::ImeEvent::Preedit { text: "にほ".into(), active_range_chars: None });
    let commit = egui::Event::Ime(egui::ImeEvent::Commit("日本".into()));
    let app = send(app, vec![preedit, commit, key(egui::Key::Period, egui::Modifiers::COMMAND)]);
    assert!(app.ui.text_edit.is_none(), "the session ends");
    assert_eq!(snapshot(&app), before, "as before the session");
}
