//! ⌥ (Alt) with the Background Eraser is the Eyedropper, as with the Brush (#11906): a click or
//! drag sets the foreground colour, adds no history state and changes no pixel, and the cursor
//! is the pipette while ⌥ is held. Control+⌥ stays the brush-resize gesture.

use egui::{Modifiers, PointerButton, Pos2, vec2};
use egui_kittest::Harness;
use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::ViewXform;
use crate::state::Tool;

const RED: [f32; 4] = [1.0, 0.0, 0.0, 1.0];
const GREEN: [f32; 4] = [0.0, 1.0, 0.0, 1.0];
const BLUE: [f32; 4] = [0.0, 0.0, 1.0, 1.0];

fn harness(app: PhotocraftApp) -> Harness<'static, PhotocraftApp> {
    let mut h = Harness::builder().with_size(vec2(900.0, 600.0)).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            let ctx = ui.ctx().clone();
            if !ctx.fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            crate::shortcuts::handle(app, &ctx);
            egui::CentralPanel::default().show(ui, |ui| crate::canvas::document_area(app, ui));
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    h.run_steps(4);
    let v = &mut h.state_mut().ui.views[0];
    v.zoom = 1.0;
    v.center = [100.0, 50.0];
    v.fit_pending = false;
    h.run_steps(2);
    h
}

fn screen(h: &Harness<'static, PhotocraftApp>, x: f32, y: f32) -> Pos2 {
    let app = h.state();
    let v = &app.ui.views[0];
    let xf = ViewXform {
        rect: crate::rulers::content_rect(app, app.last_canvas_rect),
        zoom: v.zoom,
        center: v.center,
        flip: app.ui.view.flip_horizontal,
        rotation: v.rotation,
        aspect: app.ui.view.display_aspect(),
    };
    xf.to_screen(x, y)
}

fn mods(h: &mut Harness<'static, PhotocraftApp>, m: Modifiers) {
    h.event(egui::Event::ModifiersChanged(m));
    h.run_steps(1);
}

fn button(h: &mut Harness<'static, PhotocraftApp>, b: PointerButton, p: Pos2, down: bool, m: Modifiers) {
    h.event(egui::Event::PointerButton { pos: p, button: b, pressed: down, modifiers: m });
    h.run_steps(1);
}

/// Press `b` at document point `from` with `m` held from before the press, and move to `to`;
/// the button stays down.
fn press_and_move(h: &mut Harness<'static, PhotocraftApp>, b: PointerButton, from: [f32; 2], to: [f32; 2], m: Modifiers) {
    let p = screen(h, from[0], from[1]);
    mods(h, m);
    h.event(egui::Event::PointerMoved(p));
    h.run_steps(1);
    button(h, b, p, true, m);
    if from != to {
        for i in 1..=8 {
            let t = i as f32 / 8.0;
            let q = screen(h, from[0] + (to[0] - from[0]) * t, from[1] + (to[1] - from[1]) * t);
            h.event(egui::Event::PointerMoved(q));
            h.run_steps(1);
        }
    }
}

/// Release `b` at document point `at`, then let go of the modifiers.
fn release(h: &mut Harness<'static, PhotocraftApp>, b: PointerButton, at: [f32; 2], m: Modifiers) {
    button(h, b, screen(h, at[0], at[1]), false, m);
    h.run_steps(2);
    mods(h, Modifiers::NONE);
}

/// A drag between document points with `m` held from before the press until after the
/// release; `from == to` is a click.
fn drag(h: &mut Harness<'static, PhotocraftApp>, from: [f32; 2], to: [f32; 2], m: Modifiers) {
    press_and_move(h, PointerButton::Primary, from, to, m);
    release(h, PointerButton::Primary, to, m);
}

/// Red on the left, green on the right of an unlocked layer, Background Eraser active, blue
/// foreground and white background colour.
fn background_eraser() -> Harness<'static, PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 200, "height": 100, "background": "white"})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("select.rect", json!({"x": 0, "y": 0, "width": 100, "height": 100})).unwrap();
    app.run("edit.fill", json!({"color": "#ff0000"})).unwrap();
    app.run("select.rect", json!({"x": 100, "y": 0, "width": 100, "height": 100})).unwrap();
    app.run("edit.fill", json!({"color": "#00ff00"})).unwrap();
    app.run("select.deselect", json!({})).unwrap();
    app.run("tools.setColors", json!({"foreground": BLUE, "background": [1.0, 1.0, 1.0, 1.0]})).unwrap();
    app.sync_views();
    let mut h = harness(app);
    h.state_mut().ui.tool = Tool::BackgroundEraser;
    h.run_steps(1);
    h
}

/// The document and the number of history states, to show nothing changed.
fn snapshot(h: &Harness<'static, PhotocraftApp>) -> (std::sync::Arc<photocraft_doc::Document>, usize) {
    let st = h.state().session.active().unwrap();
    (st.doc.clone(), st.history.past_len())
}

fn unchanged(h: &Harness<'static, PhotocraftApp>, before: &(std::sync::Arc<photocraft_doc::Document>, usize), what: &str) {
    let (doc, steps) = snapshot(h);
    assert!(std::sync::Arc::ptr_eq(&doc, &before.0), "{what}: no pixel changed");
    assert_eq!(steps, before.1, "{what}: no history state");
}

#[test]
fn alt_click_sets_the_foreground_without_erasing() {
    let mut h = background_eraser();
    let before = snapshot(&h);
    drag(&mut h, [150.0, 50.0], [150.0, 50.0], Modifiers::ALT);
    assert_eq!(h.state().session.tools.foreground, GREEN, "⌥-click picked green");
    assert_eq!(h.state().session.tools.background, [1.0, 1.0, 1.0, 1.0], "the background colour stays");
    unchanged(&h, &before, "⌥-click");
    assert!(!h.state().alt_sampling, "the release ends the sampling");
}

#[test]
fn alt_drag_follows_the_pointer_without_erasing() {
    let mut h = background_eraser();
    let before = snapshot(&h);
    press_and_move(&mut h, PointerButton::Primary, [150.0, 50.0], [40.0, 50.0], Modifiers::ALT);
    assert_eq!(h.state().session.tools.foreground, RED, "⌥-drag samples while the button is held");
    release(&mut h, PointerButton::Primary, [40.0, 50.0], Modifiers::ALT);
    assert_eq!(h.state().session.tools.foreground, RED, "⌥-drag ends on red");
    unchanged(&h, &before, "⌥-drag");
}

/// The same press without ⌥ still erases: the harness drives the real tool.
#[test]
fn a_click_without_alt_still_erases() {
    let mut h = background_eraser();
    let (_, steps) = snapshot(&h);
    drag(&mut h, [150.0, 50.0], [150.0, 50.0], Modifiers::NONE);
    assert_eq!(h.state().session.tools.foreground, BLUE);
    assert_eq!(snapshot(&h).1, steps + 1, "the stroke added a history state");
    let st = h.state().session.active().unwrap();
    let layer = st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap();
    assert_eq!(layer.rgba(150, 50)[3], 0.0, "the clicked pixel was erased");
}

/// While ⌥ is held the cursor is the Eyedropper's pipette (`pipette_cursor`), where the
/// Background Eraser otherwise shows its own tip; Control+⌥ keeps the tip.
#[test]
fn alt_shows_the_eyedropper_cursor() {
    let mut h = background_eraser();
    let p = screen(&h, 150.0, 50.0);
    let pipette = |h: &mut Harness<'static, PhotocraftApp>, m: Modifiers| {
        mods(h, m);
        h.hover_at(p);
        h.run_steps(2);
        let layer = egui::LayerId::new(egui::Order::Tooltip, egui::Id::unique("pc-icon-cursor"));
        let shown = h.ctx.memory(|memory| memory.areas().visible_layer_ids().contains(&layer));
        (shown, h.output().platform_output.cursor_icon)
    };
    assert_eq!(pipette(&mut h, Modifiers::ALT), (true, egui::CursorIcon::None), "⌥: the pipette replaces the pointer");
    assert!(!pipette(&mut h, Modifiers::NONE).0, "without ⌥: the eraser's own tip");
    assert!(!pipette(&mut h, Modifiers::ALT | Modifiers::CTRL).0, "Control+⌥ is not the Eyedropper");
    assert!(pipette(&mut h, Modifiers::ALT).0, "⌥ again: the pipette again");
}

/// Control+⌥-drag stays the brush-resize gesture: it samples nothing and erases nothing.
#[test]
fn ctrl_alt_drag_resizes_instead_of_sampling() {
    assert!(crate::canvas::alt_samples(Tool::BackgroundEraser, Modifiers::ALT));
    assert!(!crate::canvas::alt_samples(Tool::BackgroundEraser, Modifiers::ALT | Modifiers::CTRL));
    let mut h = background_eraser();
    h.state_mut().run("tools.setBrush", json!({"brush": {"size": 40}})).unwrap();
    let before = snapshot(&h);
    drag(&mut h, [150.0, 50.0], [40.0, 50.0], Modifiers::ALT | Modifiers::CTRL);
    assert_eq!(h.state().session.tools.foreground, BLUE, "Control+⌥ sampled nothing");
    unchanged(&h, &before, "Control+⌥-drag");
    assert_ne!(h.state().session.tools.brush.size, 40.0, "Control+⌥-drag resized the brush");
}

/// ⌥+right-drag (not measured in the reference application) keeps today's meaning, the brush
/// resize: it samples nothing and erases nothing.
#[test]
fn alt_right_drag_resizes_instead_of_sampling() {
    let mut h = background_eraser();
    h.state_mut().run("tools.setBrush", json!({"brush": {"size": 40}})).unwrap();
    let before = snapshot(&h);
    press_and_move(&mut h, PointerButton::Secondary, [150.0, 50.0], [40.0, 50.0], Modifiers::ALT);
    release(&mut h, PointerButton::Secondary, [40.0, 50.0], Modifiers::ALT);
    assert_eq!(h.state().session.tools.foreground, BLUE, "⌥+right-drag sampled nothing");
    unchanged(&h, &before, "⌥+right-drag");
    assert_ne!(h.state().session.tools.brush.size, 40.0, "⌥+right-drag resized the brush");
}
