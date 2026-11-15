//! Polygonal Lasso History step and Shift snap (#11393): the step is named "Polygonal Lasso"
//! however the polygon closes, and a Shift diagonal keeps the shorter of the two offsets on both
//! axes, as Photoshop does.

use egui::{Modifiers, PointerButton, Pos2, vec2};
use egui_kittest::Harness;
use photocraft_geom::Rect;
use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::{ToolEvent, ViewXform, constrained_polygon_point, tool_event};
use crate::state::Tool;

fn app_with(tool: Tool) -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 800, "height": 600, "background": "transparent"})).unwrap();
    app.sync_views();
    app.ui.extras.snap = false;
    app.ui.tool = tool;
    app
}

fn ev(app: &mut PhotocraftApp, e: ToolEvent, m: Modifiers) {
    tool_event(app, e, m);
}

fn click(app: &mut PhotocraftApp, x: f64, y: f64, m: Modifiers) {
    ev(app, ToolEvent::Down { x, y, pressure: 1.0 }, m);
    ev(app, ToolEvent::Up { x, y }, m);
}

fn last_step(app: &PhotocraftApp) -> Option<String> {
    app.session.active().unwrap().history.entries().last().cloned()
}

fn bounds(app: &PhotocraftApp) -> Option<Rect> {
    app.session.active().unwrap().doc.selection.as_ref().map(|s| s.content_bounds())
}

fn harness() -> Harness<'static, PhotocraftApp> {
    let mut app = app_with(Tool::PolygonLasso);
    app.ui.extras.rulers = false;
    // 60 fps, so a double-click's two clicks fall inside egui's 0.3 s window.
    let mut h = Harness::builder().with_size(vec2(1000.0, 700.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            let ctx = ui.ctx().clone();
            if !ctx.fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            crate::shortcuts::handle(app, &ctx);
            egui::CentralPanel::default().show(ui, |ui| crate::canvas::document_area(app, ui));
            crate::dialogs::show(app, &ctx);
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    h.run_steps(4);
    // 100 %: one document pixel per point, the area around (200, 200) in view.
    let v = &mut h.state_mut().ui.views[0];
    v.zoom = 1.0;
    v.center = [200.0, 200.0];
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

fn screen_click(h: &mut Harness<'static, PhotocraftApp>, x: f32, y: f32) {
    let p = screen(h, x, y);
    h.event(egui::Event::PointerMoved(p));
    h.run_steps(1);
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton { pos: p, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
        h.run_steps(1);
    }
}

/// Three corners, well apart in time so they aren't a double-click.
fn three_corners(h: &mut Harness<'static, PhotocraftApp>) {
    for (x, y) in [(100.0, 100.0), (300.0, 100.0), (300.0, 300.0)] {
        screen_click(h, x, y);
        h.run_steps(30);
    }
}

#[test]
fn closing_on_the_start_point_records_a_polygonal_lasso_step() {
    let mut app = app_with(Tool::PolygonLasso);
    for (x, y) in [(100.0, 100.0), (300.0, 100.0), (300.0, 300.0), (100.0, 100.0)] {
        click(&mut app, x, y, Modifiers::NONE);
    }
    assert!(app.ui.polygon.is_empty(), "the start point closes the polygon");
    assert_eq!(bounds(&app), Some(Rect::new(100, 100, 300, 300)));
    assert_eq!(last_step(&app).as_deref(), Some("Polygonal Lasso"));
}

#[test]
fn closing_with_enter_records_a_polygonal_lasso_step() {
    let mut h = harness();
    three_corners(&mut h);
    let before = h.state().session.active().unwrap().history.entries().len();
    h.key_press(egui::Key::Enter);
    h.run_steps(2);
    assert!(h.state().ui.polygon.is_empty(), "Enter closes the polygon");
    assert!(bounds(h.state()).is_some());
    assert_eq!(h.state().session.active().unwrap().history.entries().len(), before + 1, "one step");
    assert_eq!(last_step(h.state()).as_deref(), Some("Polygonal Lasso"));
}

#[test]
fn closing_with_a_double_click_records_a_polygonal_lasso_step() {
    let mut h = harness();
    three_corners(&mut h);
    screen_click(&mut h, 100.0, 300.0);
    screen_click(&mut h, 100.0, 300.0);
    h.run_steps(1);
    assert!(h.state().ui.polygon.is_empty(), "the double-click closes the polygon");
    assert!(bounds(h.state()).is_some());
    assert_eq!(last_step(h.state()).as_deref(), Some("Polygonal Lasso"));
}

#[test]
fn the_freehand_lasso_still_records_a_lasso_step() {
    let mut app = app_with(Tool::Lasso);
    ev(&mut app, ToolEvent::Down { x: 100.0, y: 100.0, pressure: 1.0 }, Modifiers::NONE);
    for (x, y) in [(200.0, 100.0), (300.0, 100.0), (300.0, 200.0), (300.0, 300.0), (200.0, 300.0)] {
        ev(&mut app, ToolEvent::Move { x, y, pressure: 1.0 }, Modifiers::NONE);
    }
    ev(&mut app, ToolEvent::Up { x: 200.0, y: 300.0 }, Modifiers::NONE);
    assert!(bounds(&app).is_some());
    assert_eq!(last_step(&app).as_deref(), Some("Lasso"));
    // And the engine's own default, for scripts that leave `polygonal` out.
    app.run("select.lasso", json!({"points": [[10, 10], [60, 10], [10, 60]]})).unwrap();
    assert_eq!(last_step(&app).as_deref(), Some("Lasso"));
}

/// The reference measurement: from 100,100 a Shift-click at 200,170 lands on (170,170), and the
/// closed selection's right edge is at 170.
#[test]
fn a_shift_click_at_200_170_from_100_100_places_the_vertex_at_170_170() {
    let mut app = app_with(Tool::PolygonLasso);
    click(&mut app, 100.0, 100.0, Modifiers::NONE);
    click(&mut app, 200.0, 170.0, Modifiers::SHIFT);
    assert_eq!(app.ui.polygon, vec![[100.0, 100.0], [170.0, 170.0]]);
    click(&mut app, 100.0, 300.0, Modifiers::NONE);
    crate::canvas::commit_polygon(&mut app);
    assert_eq!(bounds(&app).map(|r| (r.x0, r.x1)), Some((100, 170)), "{:?}", bounds(&app));
    assert_eq!(last_step(&app).as_deref(), Some("Polygonal Lasso"));
}

#[test]
fn each_diagonal_keeps_the_shorter_offset_with_its_signs() {
    let from = [100.0, 100.0];
    // One case per diagonal, the shorter offset on either axis.
    for (to, want) in [
        ([200.0, 170.0], [170.0, 170.0]), // down-right, dy shorter
        ([30.0, 200.0], [30.0, 170.0]),   // down-left, dx shorter
        ([0.0, 30.0], [30.0, 30.0]),      // up-left, dy shorter
        ([170.0, 0.0], [170.0, 30.0]),    // up-right, dx shorter
    ] {
        assert_eq!(constrained_polygon_point(from, to), want, "{to:?}");
    }
}

/// Axis snaps aren't measured in the reference, so they keep today's rule: the segment's length.
#[test]
fn horizontal_and_vertical_snaps_keep_the_segment_length() {
    let from = [100.0, 100.0];
    let length = 100.0_f64.hypot(20.0);
    for (to, want) in [
        ([200.0, 120.0], [100.0 + length, 100.0]),
        ([80.0, 200.0], [100.0, 100.0 + length]),
        ([0.0, 80.0], [100.0 - length, 100.0]),
        ([120.0, 0.0], [100.0, 100.0 - length]),
    ] {
        let got = constrained_polygon_point(from, to);
        assert!((got[0] - want[0]).abs() < 1e-9 && (got[1] - want[1]).abs() < 1e-9, "{to:?} → {got:?}");
    }
}
