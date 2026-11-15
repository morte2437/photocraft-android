//! A Shift or Alt held from the start of a Rectangular or Elliptical Marquee drag picks the
//! selection mode only when the document already has a selection (#11389). Without one the same
//! keys constrain the shape from the first move, as in the reference application: Shift squares to
//! the drag's shorter side, Alt draws from the centre.

use egui::{Modifiers, PointerButton, Pos2, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use photocraft_geom::Rect;
use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::ViewXform;
use crate::state::Tool;

fn harness(tool: Tool) -> Harness<'static, PhotocraftApp> {
    harness_with_step_dt(tool, 0.25)
}

fn harness_with_step_dt(tool: Tool, step_dt: f32) -> Harness<'static, PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 800, "height": 600})).unwrap();
    app.sync_views();
    app.ui.extras.rulers = false;
    app.ui.tool = tool;
    let mut h = Harness::builder().with_size(vec2(1100.0, 800.0)).with_step_dt(step_dt).build_ui_state(
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
    // 100 %: one document pixel per point, so drags land on whole pixels.
    let v = &mut h.state_mut().ui.views[0];
    v.zoom = 1.0;
    v.center = [400.0, 300.0];
    v.fit_pending = false;
    h.run_steps(2);
    h
}

fn view_xform(h: &Harness<'static, PhotocraftApp>) -> ViewXform {
    let app = h.state();
    let v = &app.ui.views[0];
    ViewXform {
        rect: crate::rulers::content_rect(app, app.last_canvas_rect),
        zoom: v.zoom,
        center: v.center,
        flip: app.ui.view.flip_horizontal,
        rotation: v.rotation,
        aspect: app.ui.view.display_aspect(),
    }
}

fn screen(h: &Harness<'static, PhotocraftApp>, x: f32, y: f32) -> Pos2 {
    view_xform(h).to_screen(x, y)
}

fn mods(h: &mut Harness<'static, PhotocraftApp>, m: Modifiers) {
    h.event(egui::Event::ModifiersChanged(m));
    h.run_steps(1);
}

fn button(h: &mut Harness<'static, PhotocraftApp>, p: Pos2, down: bool, m: Modifiers) {
    h.event(egui::Event::PointerButton { pos: p, button: PointerButton::Primary, pressed: down, modifiers: m });
    h.run_steps(1);
}

/// Move the pointer to document point `(x, y)` in a few steps.
fn move_to(h: &mut Harness<'static, PhotocraftApp>, x: f32, y: f32) {
    let p = screen(h, x, y);
    h.event(egui::Event::PointerMoved(p));
    h.run_steps(2);
}

fn press_at(h: &mut Harness<'static, PhotocraftApp>, x: f32, y: f32, m: Modifiers) {
    let p = screen(h, x, y);
    h.event(egui::Event::PointerMoved(p));
    h.run_steps(1);
    button(h, p, true, m);
    // Past egui's click distance, so the drag starts at the press point.
    move_to(h, x + 8.0, y + 6.0);
}

fn release_at(h: &mut Harness<'static, PhotocraftApp>, x: f32, y: f32, m: Modifiers) {
    move_to(h, x, y);
    button(h, screen(h, x, y), false, m);
    h.run_steps(2);
}

fn selection(h: &Harness<'static, PhotocraftApp>) -> Rect {
    h.state().session.active().unwrap().doc.selection.as_ref().map_or(Rect::EMPTY, |s| s.content_bounds())
}

/// The two-row readout shows `W: {w} px` and `H: {ht} px` (Photoshop's format).
fn readout(h: &Harness<'static, PhotocraftApp>, w: i32, ht: i32) -> bool {
    let labels = h.query_by_label("W:").is_some() && h.query_by_label("H:").is_some();
    let count = |v: i32| h.query_all_by_label(&format!("{v} px")).count();
    let values = if w == ht { count(w) == 2 } else { count(w) == 1 && count(ht) == 1 };
    labels && values && h.query_all_by_label_contains(" px").count() == 2
}

/// Drag a marquee from `from` to `to` with `m` held from before the press to after the release.
fn drag_held(h: &mut Harness<'static, PhotocraftApp>, m: Modifiers, from: [f32; 2], to: [f32; 2]) {
    mods(h, m);
    press_at(h, from[0], from[1], m);
    move_to(h, to[0], to[1]);
    release_at(h, to[0], to[1], m);
    mods(h, Modifiers::NONE);
}

#[test]
fn without_a_selection_keys_held_from_the_press_constrain_the_shape() {
    let shift_alt = Modifiers { shift: true, alt: true, ..Modifiers::NONE };
    let cases = [
        // ⇧ from the press: a 200 × 100 drag selects a 100 × 100 square (the shorter side).
        (Modifiers::SHIFT, [100.0, 100.0], [300.0, 200.0], Rect::new(100, 100, 200, 200)),
        // ⌥ from the press: drawn from the centre.
        (Modifiers::ALT, [400.0, 300.0], [500.0, 350.0], Rect::new(300, 250, 500, 350)),
        // Both: a square from the centre, half extent the shorter drag.
        (shift_alt, [400.0, 300.0], [500.0, 350.0], Rect::new(350, 250, 450, 350)),
    ];
    for tool in [Tool::RectMarquee, Tool::EllipseMarquee] {
        for (m, from, to, want) in cases {
            let mut h = harness(tool);
            h.state_mut().ui.extras.snap = false;
            drag_held(&mut h, m, from, to);
            assert_eq!(selection(&h), want, "{tool:?} with {m:?} held from the press");
        }
    }
}

#[test]
fn the_live_readout_shows_the_constrained_shape_from_the_first_move() {
    let mut h = harness(Tool::RectMarquee);
    h.state_mut().ui.extras.snap = false;
    mods(&mut h, Modifiers::SHIFT);
    press_at(&mut h, 100.0, 100.0, Modifiers::SHIFT);
    move_to(&mut h, 300.0, 200.0);
    assert!(readout(&h, 100, 100), "the readout agrees with what the release selects");
    release_at(&mut h, 300.0, 200.0, Modifiers::SHIFT);
    mods(&mut h, Modifiers::NONE);
    assert_eq!(selection(&h), Rect::new(100, 100, 200, 200));
}

#[test]
fn with_a_selection_keys_held_from_the_press_still_pick_the_mode() {
    let mut h = harness(Tool::RectMarquee);
    h.state_mut().ui.extras.snap = false;
    h.state_mut().run("select.rect", json!({"x": 100, "y": 100, "width": 200, "height": 200})).unwrap();
    // ⇧ adds, and the added rectangle is as drawn, not squared: the union reaches 600.
    drag_held(&mut h, Modifiers::SHIFT, [400.0, 100.0], [600.0, 150.0]);
    assert_eq!(selection(&h), Rect::new(100, 100, 600, 300));
    let st = h.state().session.active().unwrap();
    let sel = st.doc.selection.as_ref().unwrap();
    assert!(sel.sample_channel(150, 150, 0) > 0.99 && sel.sample_channel(550, 125, 0) > 0.99 && sel.sample_channel(550, 250, 0) < 0.01);
}
