//! Magnetic Lasso: a right-click while a border is being drawn opens no menu (#11689).
//!
//! Measured in Photoshop: with a Magnetic Lasso border in progress, a right-click on the canvas
//! opens nothing and changes nothing; the border stays live, and ↩ still closes it. With no border
//! in progress the right-click opens the selection tools' menu as before. PhotoCraft opened that
//! menu over the live border, from the canvas and from the control channel alike.
//!
//! The border is drawn with ⌥-clicks (straight segments), so its points don't depend on edges.
//! No GPU needed.

use egui::{Event, Modifiers, PointerButton};
use photocraft_ui_egui::PhotocraftApp;
use photocraft_ui_egui::canvas::ViewXform;
use photocraft_ui_egui::control::{ControlRequest, Outcome, handle};
use serde_json::{Value, json};

type Harness = egui_kittest::Harness<'static, PhotocraftApp>;

/// The border's corners: a plain click starts it, ⌥-clicks add the others.
const CORNERS: [[f64; 2]; 3] = [[50.0, 50.0], [250.0, 50.0], [150.0, 200.0]];

/// Where the right-click goes: inside the border, on the image.
const INSIDE: [f64; 2] = [150.0, 100.0];

fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 300})).expect("new document");
    app.sync_views();
    app
}

fn call(app: &mut PhotocraftApp, method: &str, params: Value) -> Value {
    let ctx = egui::Context::default();
    let (req, _rx) = ControlRequest::new(method, params);
    let Outcome::Done(v) = handle(app, &ctx, &req) else { panic!("{method} didn't finish") };
    v
}

fn pointer(app: &mut PhotocraftApp, params: Value) {
    let v = call(app, "ui.pointer", params);
    assert_eq!(v["ok"], true, "{v}");
}

/// Clicks at `p` through `ui.pointer` with the Magnetic Lasso.
fn agent_click(app: &mut PhotocraftApp, p: [f64; 2], button: &str, alt: bool) {
    let events = json!([{"kind": "down", "x": p[0], "y": p[1]}, {"kind": "up", "x": p[0], "y": p[1]}]);
    pointer(app, json!({"tool": "MagneticLasso", "button": button, "alt": alt, "events": events}));
}

/// `ui.inspect`'s Magnetic Lasso border and canvas tool menu.
fn inspect(app: &mut PhotocraftApp) -> (Value, Value) {
    let v = call(app, "ui.inspect", json!({}));
    let r = &v["result"];
    (r["magnetic"].clone(), r["canvasToolMenu"].clone())
}

#[test]
fn agent_right_click_while_drawing_opens_no_menu_and_keeps_the_border() {
    let mut app = app();
    agent_click(&mut app, CORNERS[0], "left", false);
    agent_click(&mut app, CORNERS[1], "left", true);
    agent_click(&mut app, CORNERS[2], "left", true);
    let (before, menu) = inspect(&mut app);
    assert!(menu.is_null(), "no menu yet: {menu}");
    assert_eq!(before["anchors"].as_array().map(Vec::len), Some(3), "a border is in progress: {before}");

    agent_click(&mut app, INSIDE, "right", false);
    let (after, menu) = inspect(&mut app);
    assert!(menu.is_null(), "a right-click over a live border opens no menu: {menu}");
    assert_eq!(after["path"], before["path"], "the border is unchanged");
    assert_eq!(after["anchors"], before["anchors"], "its fastening points are unchanged");
    assert!(app.session.active().expect("a document").doc.selection.is_none(), "nothing is selected yet");

    // The border is still live: closing it selects it.
    photocraft_ui_egui::magnetic_lasso_ui::close(&mut app, true);
    let sel = app.session.active().expect("a document").doc.selection.as_ref().expect("the border closed into a selection");
    assert!(sel.sample_channel(150, 100, 0) > 0.9, "the inside is selected");
}

#[test]
fn agent_right_click_on_another_document_still_opens_the_selection_menu() {
    // A border left on the first document doesn't hold the menu back on the second: the agent's
    // events run no frame, so nothing has dropped that border yet.
    let mut app = app();
    agent_click(&mut app, CORNERS[0], "left", false);
    agent_click(&mut app, CORNERS[1], "left", true);
    app.run("file.new", json!({"width": 400, "height": 300})).expect("second document");
    app.sync_views();
    assert!(app.ui.magnetic.active(), "the first document's border is still there");

    agent_click(&mut app, INSIDE, "right", false);
    let (_, menu) = inspect(&mut app);
    let entries = menu["entries"].as_array().expect("the selection menu opened");
    assert!(entries.iter().any(|e| e["id"] == "select.all"), "{menu}");
}

#[test]
fn agent_right_click_during_an_alt_drag_still_opens_the_selection_menu() {
    // Not measured in Photoshop: a right-click while an ⌥-drag draws a freehand segment keeps
    // today's behaviour, the selection menu.
    let mut app = app();
    agent_click(&mut app, CORNERS[0], "left", false);
    let events = json!([{"kind": "down", "x": CORNERS[1][0], "y": CORNERS[1][1]}, {"kind": "move", "x": 200.0, "y": 120.0}]);
    pointer(&mut app, json!({"tool": "MagneticLasso", "alt": true, "events": events}));
    assert!(app.ui.magnetic.freehand, "an ⌥-drag is drawing a freehand segment");

    agent_click(&mut app, INSIDE, "right", false);
    let (_, menu) = inspect(&mut app);
    let entries = menu["entries"].as_array().expect("the selection menu opened");
    assert!(entries.iter().any(|e| e["id"] == "select.all"), "{menu}");
}

#[test]
fn agent_right_click_without_a_border_still_opens_the_selection_menu() {
    let mut app = app();
    agent_click(&mut app, INSIDE, "right", false);
    let (magnetic, menu) = inspect(&mut app);
    assert_eq!(magnetic["path"], json!([]), "a right-click starts no border");
    let entries = menu["entries"].as_array().expect("the selection menu opened");
    assert!(entries.iter().any(|e| e["id"] == "select.all"), "{menu}");
}

/// The app's document area alone, at 100 %, the Magnetic Lasso selected.
fn harness() -> Harness {
    let mut app = app();
    app.ui.tool = photocraft_ui_egui::state::Tool::MagneticLasso;
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1000.0, 700.0)).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            let ctx = ui.ctx().clone();
            if !ctx.fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            photocraft_ui_egui::shortcuts::handle(app, &ctx);
            egui::CentralPanel::default().show(ui, |ui| photocraft_ui_egui::canvas::document_area(app, ui));
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, Default::default());
    h.run_steps(4);
    let v = &mut h.state_mut().ui.views[0];
    (v.zoom, v.center, v.fit_pending) = (1.0, [200.0, 150.0], false);
    h.run_steps(2);
    h
}

/// The screen point of document point `p`.
fn screen(h: &Harness, p: [f64; 2]) -> egui::Pos2 {
    let app = h.state();
    let v = &app.ui.views[0];
    let xf = ViewXform {
        rect: photocraft_ui_egui::rulers::content_rect(app, app.last_canvas_rect),
        zoom: v.zoom,
        center: v.center,
        flip: false,
        rotation: v.rotation,
        aspect: 1.0,
    };
    xf.to_screen(p[0] as f32, p[1] as f32)
}

/// Moves the pointer to `p`.
fn hover(h: &mut Harness, p: [f64; 2], modifiers: Modifiers) {
    let pos = screen(h, p);
    h.event(Event::ModifiersChanged(modifiers));
    h.event(Event::PointerMoved(pos));
    h.run_steps(2);
}

fn mouse_click(h: &mut Harness, p: [f64; 2], button: PointerButton, modifiers: Modifiers) {
    hover(h, p, modifiers);
    let pos = screen(h, p);
    h.event(Event::PointerButton { pos, button, pressed: true, modifiers });
    h.run_steps(1);
    h.event(Event::PointerButton { pos, button, pressed: false, modifiers });
    h.run_steps(2);
}

#[test]
fn canvas_right_click_while_drawing_opens_no_menu_and_enter_still_closes_the_border() {
    let mut h = harness();
    mouse_click(&mut h, CORNERS[0], PointerButton::Primary, Modifiers::NONE);
    mouse_click(&mut h, CORNERS[1], PointerButton::Primary, Modifiers::ALT);
    mouse_click(&mut h, CORNERS[2], PointerButton::Primary, Modifiers::ALT);
    // The pointer goes where the right-click will be first: moves with the button up fasten points
    // along the way, as they should.
    hover(&mut h, INSIDE, Modifiers::NONE);
    let before = h.state().ui.magnetic.clone();
    assert!(before.anchors.len() >= 3, "a border is in progress: {before:?}");

    mouse_click(&mut h, INSIDE, PointerButton::Secondary, Modifiers::NONE);
    assert!(h.state().ui.canvas_tool_menu.is_none(), "a right-click over a live border opens no menu");
    let after = &h.state().ui.magnetic;
    assert_eq!((&after.path, &after.anchors), (&before.path, &before.anchors), "the border is unchanged");

    h.event(Event::Key { key: egui::Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE });
    h.event(Event::Key { key: egui::Key::Enter, physical_key: None, pressed: false, repeat: false, modifiers: Modifiers::NONE });
    h.run_steps(2);
    assert!(!h.state().ui.magnetic.active(), "↩ closed the border");
    let sel = h.state().session.active().expect("a document").doc.selection.as_ref().expect("the border closed into a selection");
    assert!(sel.sample_channel(150, 100, 0) > 0.9, "the inside is selected");
}

#[test]
fn canvas_right_click_without_a_border_still_opens_the_selection_menu() {
    let mut h = harness();
    mouse_click(&mut h, INSIDE, PointerButton::Secondary, Modifiers::NONE);
    assert!(!h.state().ui.magnetic.active(), "a right-click starts no border");
    assert!(h.state().ui.canvas_tool_menu.is_some(), "the selection menu opened");
}
