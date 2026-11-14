//! A Window › New Window document window under an open dialog: its canvas is as inert as the main
//! window's, so a tool drag there adds no History step (the dialog's Cancel would not undo it).

use egui::{Event, Modifiers, PointerButton, Pos2, RawInput, Rect, vec2};
use serde_json::json;

use crate::PhotocraftApp;
use crate::state::{DocWindow, Tool, View};

use super::tabs::document_window_canvas;

fn app_with_a_layer() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.background_jobs = false;
    app.run("file.new", json!({"width": 200, "height": 150})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    // Filled where the drag starts; Auto-Select off, so the press moves the active layer whatever
    // lies under it.
    app.run("select.rect", json!({"x": 20, "y": 20, "width": 160, "height": 110})).unwrap();
    app.run("edit.fill", json!({"color": "#ff0000"})).unwrap();
    app.run("select.deselect", json!({})).unwrap();
    app.sync_views();
    app.ui.tool = Tool::Move;
    app.ui.tool_options.move_auto_select = false;
    app.ui.extras.rulers = false;
    app.ui.extras.snap = false;
    app.ui.view.show.smart_guides = false;
    app
}

fn history(app: &PhotocraftApp) -> Vec<String> {
    app.session.active().unwrap().history.entries().to_vec()
}

/// A document window's canvas, drawn in its own context as its own viewport is: no dialog is
/// drawn there, only in the main window. 1:1 around the document's centre, at the screen's centre.
struct DocumentWindow {
    ctx: egui::Context,
    screen: Rect,
    window: DocWindow,
    view: View,
}

impl DocumentWindow {
    fn new(app: &PhotocraftApp) -> Self {
        let window = DocWindow { id: 7, document: 0, view: app.ui.views[0].clone(), open: true, position: None, floating: false, focused: true };
        let mut view = window.view.clone();
        let size = app.session.active().unwrap().doc.size;
        view.zoom = 1.0;
        view.center = [size.width as f32 / 2.0, size.height as f32 / 2.0];
        view.fit_pending = false;
        view.fill_pending = false;
        Self { ctx: egui::Context::default(), screen: Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0)), window, view }
    }

    fn frame(&mut self, app: &mut PhotocraftApp, events: Vec<Event>) {
        let input = RawInput { screen_rect: Some(self.screen), events, ..Default::default() };
        let mut out = self.ctx.run_ui(input, |ui| {
            self.view = document_window_canvas(app, ui, &self.window, self.view.clone(), true).0;
        });
        out.textures_delta.clear();
    }
}

fn button(pos: Pos2, pressed: bool, modifiers: Modifiers) -> Event {
    Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers }
}

/// Drags across the middle of a document window's canvas.
fn drag_in_document_window(app: &mut PhotocraftApp) {
    let mut w = DocumentWindow::new(app);
    let start = w.screen.center();
    w.frame(app, vec![]);
    w.frame(app, vec![Event::PointerMoved(start)]);
    w.frame(app, vec![button(start, true, Modifiers::NONE)]);
    let mut at = start;
    for _ in 0..6 {
        at += vec2(15.0, 10.0);
        w.frame(app, vec![Event::PointerMoved(at)]);
    }
    w.frame(app, vec![button(at, false, Modifiers::NONE)]);
    w.frame(app, vec![]);
}

#[test]
fn a_move_drag_in_a_document_window_adds_a_history_step_without_a_dialog() {
    // The control: the same drag does reach the Move tool when no dialog is open.
    let mut app = app_with_a_layer();
    let before = history(&app);
    drag_in_document_window(&mut app);
    let after = history(&app);
    assert_eq!(after.len(), before.len() + 1, "{after:?}");
    assert_eq!(after.last().map(String::as_str), Some("Move"));
}

#[test]
fn a_document_window_canvas_takes_no_tool_drag_under_a_dialog() {
    let mut app = app_with_a_layer();
    crate::filter_dialog::open(&mut app, "image.rotation.arbitrary").unwrap();
    assert!(!app.ui.dialogs.is_empty());
    let before = history(&app);
    let revision = app.session.active().unwrap().revision;
    drag_in_document_window(&mut app);
    assert_eq!(history(&app), before, "no History step under the dialog");
    assert_eq!(app.session.active().unwrap().revision, revision, "the document untouched");
    assert!(app.drag.is_none(), "no tool gesture left open");
}

/// Text being typed in a paragraph box over the document's centre, with the Type tool.
fn app_typing(dialog: bool) -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.background_jobs = false;
    app.run("file.new", json!({"width": 200, "height": 150})).unwrap();
    app.run("type.create", json!({"text": "Hello", "size": 24, "x": 20, "y": 40, "box": [20, 20, 160, 110]})).unwrap();
    app.sync_views();
    app.ui.extras.rulers = false;
    app.ui.tool = Tool::Type;
    if dialog {
        crate::filter_dialog::open(&mut app, "image.rotation.arbitrary").unwrap();
        assert!(!app.ui.dialogs.is_empty());
    }
    crate::type_tool::edit_active(&mut app).unwrap();
    assert!(app.ui.text_edit.is_some());
    app
}

/// A ⌘-press on the text: the type transform reads it from the raw events, which a modal layer
/// doesn't block. Whether a transform started.
fn command_press_on_the_text(app: &mut PhotocraftApp) -> bool {
    let command = Modifiers { command: true, ctrl: true, ..Modifiers::NONE };
    let mut w = DocumentWindow::new(app);
    let at = w.screen.center();
    w.frame(app, vec![]);
    w.frame(app, vec![Event::ModifiersChanged(command), Event::PointerMoved(at)]);
    w.frame(app, vec![button(at, true, command)]);
    app.ui.type_transform.is_some()
}

#[test]
fn a_command_press_on_text_in_a_document_window_starts_a_type_transform_without_a_dialog() {
    // The control: the press does reach the type transform when no dialog is open.
    let mut app = app_typing(false);
    assert!(command_press_on_the_text(&mut app));
}

#[test]
fn a_command_press_on_text_in_a_document_window_starts_no_type_transform_under_a_dialog() {
    let mut app = app_typing(true);
    assert!(!command_press_on_the_text(&mut app), "no type transform under the dialog");
}
