//! #11937: with the Eraser in Block mode a plain right-click on the canvas opens the Brush · Pencil
//! · Block menu at the pointer instead of the Brush Preset picker; picking an entry sets the mode,
//! adds no history and erases nothing. Brush and Pencil mode keep the picker; Shift+right-click
//! opens the menu in every mode; with the Erase preference a right-click still erases.

use egui_kittest::Harness;
use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::control::{ControlRequest, Outcome, handle};
use crate::state::Tool;

pub(super) fn app(mode: &str) -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 64, "height": 64, "background": "white"})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("edit.fill", json!({"color": "#dc1e1e"})).unwrap();
    app.ui.tool = Tool::Eraser;
    app.ui.tool_options.eraser_mode = mode.into();
    app.sync_views();
    app
}

pub(super) fn call(app: &mut PhotocraftApp, ctx: &egui::Context, method: &str, params: Value) -> Value {
    let (req, _rx) = ControlRequest::new(method, params);
    match handle(app, ctx, &req) {
        Outcome::Done(v) => v,
        _ => panic!("{method}: expected an immediate reply"),
    }
}

pub(super) fn right_click(app: &mut PhotocraftApp, ctx: &egui::Context, modifiers: Value) {
    call(
        app,
        ctx,
        "ui.pointer",
        json!({"button": "right", "modifiers": modifiers, "events": [{"kind": "down", "x": 20, "y": 20}, {"kind": "up", "x": 20, "y": 20}]}),
    );
}

pub(super) fn steps(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().history.past_len()
}

#[test]
fn block_mode_right_click_opens_the_mode_menu() {
    let ctx = egui::Context::default();
    let mut app = app("block");
    let n = steps(&app);
    right_click(&mut app, &ctx, json!({}));
    assert!(app.ui.brush_mode_menu.is_some(), "the Brush · Pencil · Block menu");
    assert!(app.ui.brush_picker.is_none(), "not the preset picker");
    let inspect = call(&mut app, &ctx, "ui.inspect", json!({}));
    assert_eq!(inspect["result"]["brushModeMenu"]["choices"], json!(["Brush", "Pencil", "Block"]), "{inspect}");
    // Picking Pencil sets the mode; nothing is erased and no history is added.
    assert_eq!(call(&mut app, &ctx, "ui.context.choose", json!({"id": "Pencil"}))["ok"], true);
    assert_eq!(app.ui.tool_options.eraser_mode, "pencil");
    assert_eq!(steps(&app), n);
    assert!(app.ui.brush_mode_menu.is_none());
}

#[test]
fn brush_and_pencil_mode_keep_the_picker() {
    let ctx = egui::Context::default();
    for mode in ["brush", "pencil"] {
        let mut app = app(mode);
        right_click(&mut app, &ctx, json!({}));
        assert!(app.ui.brush_picker.is_some(), "{mode}: the Brush Preset picker");
        assert!(app.ui.brush_mode_menu.is_none(), "{mode}");
    }
}

#[test]
fn shift_right_click_opens_the_menu_in_every_mode() {
    let ctx = egui::Context::default();
    for mode in ["brush", "pencil", "block"] {
        let mut app = app(mode);
        right_click(&mut app, &ctx, json!({"shift": true}));
        assert!(app.ui.brush_mode_menu.is_some(), "{mode}");
    }
}

#[test]
fn with_the_erase_preference_block_mode_still_erases() {
    let ctx = egui::Context::default();
    let mut app = app("block");
    app.run("prefs.set", json!({"path": "tools.rightClickWithPaintingTools", "value": "erase"})).unwrap();
    call(&mut app, &ctx, "ui.pointer", json!({"button": "right", "events": [{"kind": "down", "x": 20, "y": 20}]}));
    assert!(app.ui.brush_mode_menu.is_none(), "the preference's erase wins");
    assert!(app.secondary_erase || app.drag.is_some(), "a right-erase started");
}

/// The real mouse: a right-click on the canvas in Block mode opens the menu; Escape closes it and
/// nothing is erased.
#[test]
fn a_real_right_click_opens_the_menu_and_escape_closes_it() {
    let mut h = harness(app("block"));
    let n = steps(h.state());
    let c = h.state().last_canvas_rect.center();
    h.event(egui::Event::PointerMoved(c));
    h.run_steps(1);
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton { pos: c, button: egui::PointerButton::Secondary, pressed, modifiers: egui::Modifiers::NONE });
        h.run_steps(2);
    }
    assert!(h.state().ui.brush_mode_menu.is_some(), "the menu at the pointer");
    assert!(h.state().ui.brush_picker.is_none());
    h.key_press(egui::Key::Escape);
    h.run_steps(2);
    assert!(h.state().ui.brush_mode_menu.is_none(), "Escape closes it");
    assert_eq!(steps(h.state()), n, "nothing erased");
}

/// The canvas alone, drawn by the real mouse path.
pub(super) fn harness(app: PhotocraftApp) -> Harness<'static, PhotocraftApp> {
    let mut h = Harness::builder().with_size(egui::vec2(1200.0, 800.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            let ctx = ui.ctx().clone();
            if !ctx.fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            egui::CentralPanel::default().show(ui, |ui| crate::canvas::document_area(app, ui));
        },
        app,
    );
    crate::PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    h.run_steps(4);
    h
}
