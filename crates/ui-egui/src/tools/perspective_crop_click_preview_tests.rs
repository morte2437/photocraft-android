//! #11804: while corners are clicked, the quad follows the pointer with its grid. After two clicks
//! the third and fourth corner are the pointer (a triangle), after three the fourth is.

use super::*;

/// A 200 × 150 document with the Perspective Crop tool.
fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 200, "height": 150})).unwrap();
    app.sync_views();
    app.ui.tool = Tool::PerspectiveCrop;
    app
}

fn click(app: &mut PhotocraftApp, p: [f64; 2]) {
    assert!(pointer(app, ToolEvent::Down { x: p[0], y: p[1], pressure: 1.0 }, egui::Modifiers::NONE));
    assert!(pointer(app, ToolEvent::Up { x: p[0], y: p[1] }, egui::Modifiers::NONE));
}

/// The pointer moved with the button up.
fn hover(app: &mut PhotocraftApp, p: [f64; 2]) {
    assert!(pointer(app, ToolEvent::Move { x: p[0], y: p[1], pressure: 1.0 }, egui::Modifiers::NONE));
}

const A: [f64; 2] = [30.0, 20.0];
const B: [f64; 2] = [170.0, 25.0];
const C: [f64; 2] = [160.0, 130.0];
const D: [f64; 2] = [40.0, 120.0];

#[test]
fn after_two_clicks_the_pointer_is_the_third_and_fourth_corner() {
    let mut app = app();
    click(&mut app, A);
    click(&mut app, B);
    hover(&mut app, [120.0, 100.0]);
    assert_eq!(click_preview(&app), Some([A, B, [120.0, 100.0], [120.0, 100.0]]));
    // Every frame's move updates it.
    hover(&mut app, [90.0, 140.0]);
    assert_eq!(click_preview(&app), Some([A, B, [90.0, 140.0], [90.0, 140.0]]));
}

#[test]
fn after_three_clicks_the_pointer_is_the_fourth_corner() {
    let mut app = app();
    for p in [A, B, C] {
        click(&mut app, p);
    }
    hover(&mut app, [45.0, 110.0]);
    assert_eq!(click_preview(&app), Some([A, B, C, [45.0, 110.0]]));
    hover(&mut app, D);
    assert_eq!(click_preview(&app), Some([A, B, C, D]));
}

#[test]
fn with_one_click_or_none_nothing_follows_the_pointer() {
    let mut app = app();
    hover(&mut app, [100.0, 70.0]);
    assert_eq!(click_preview(&app), None);
    assert_eq!(app.pcrop.hover, None, "no clicks: the pointer isn't kept");
    click(&mut app, A);
    hover(&mut app, [100.0, 70.0]);
    assert_eq!(click_preview(&app), None);
}

#[test]
fn esc_and_the_fourth_click_work_as_before() {
    let mut app = app();
    click(&mut app, A);
    click(&mut app, B);
    hover(&mut app, [100.0, 70.0]);
    cancel(&mut app);
    assert!(app.pcrop.clicks.is_empty());
    assert_eq!(app.pcrop.hover, None);
    assert_eq!(click_preview(&app), None);
    // The fourth click makes the same ordered quad as without the preview, clicked in any order.
    for p in [C, A, D] {
        click(&mut app, p);
        hover(&mut app, [100.0, 70.0]);
    }
    click(&mut app, B);
    assert_eq!(app.ui.perspective_crop, Some(ordered([C, A, D, B])));
    assert_eq!(app.ui.perspective_crop, Some([A, B, C, D]));
    assert_eq!(click_preview(&app), None);
}

#[test]
fn a_non_finite_pointer_or_a_document_switch_leaves_no_preview() {
    let mut app = app();
    click(&mut app, A);
    click(&mut app, B);
    hover(&mut app, [100.0, 70.0]);
    hover(&mut app, [f64::NAN, 70.0]);
    assert_eq!(click_preview(&app), None, "a non-finite pointer");
    hover(&mut app, [100.0, f64::INFINITY]);
    assert_eq!(click_preview(&app), None);
    hover(&mut app, [100.0, 70.0]);
    assert!(click_preview(&app).is_some());
    app.run("file.new", json!({"width": 64, "height": 48})).unwrap();
    app.sync_views();
    assert_eq!(click_preview(&app), None, "another document: the clicks and preview are dropped");
    assert!(app.pcrop.clicks.is_empty());
}

#[test]
fn the_two_click_grid_meets_at_the_pointer() {
    let h = [120.0, 100.0];
    let lines = grid_lines([A, B, h, h], 4);
    assert_eq!(lines.len(), 6);
    let meets = lines.iter().filter(|[_, b]| (b[0] - h[0]).hypot(b[1] - h[1]) < 1e-9).count();
    assert_eq!(meets, 3, "one family of lines runs from side A–B to the pointer");
    // With three clicks and the pointer making a convex quad, the finished quad's perspective grid.
    let q = [A, B, C, D];
    let hom = Homography::rect_to_quad([0.0, 0.0, 1.0, 1.0], q).unwrap();
    let (x, y) = hom.apply(0.5, 0.0);
    assert!(grid_lines(q, 2).iter().any(|[a, _]| (a[0] - x).abs() < 1e-9 && (a[1] - y).abs() < 1e-9));
}

/// Shapes `draw_overlay` paints for the app as it is.
fn painted(app: &PhotocraftApp) -> usize {
    let ctx = egui::Context::default();
    let rect = egui::Rect::from_min_size(Pos2::ZERO, egui::vec2(200.0, 150.0));
    let xf = ViewXform { rect, zoom: 1.0, center: [100.0, 75.0], flip: false, rotation: 0.0, aspect: 1.0 };
    let mut o = ctx.run_ui(Default::default(), |ui| draw_overlay(app, ui.painter(), &xf));
    o.textures_delta.clear();
    o.shapes.len()
}

#[test]
fn the_preview_is_drawn_with_its_grid() {
    let mut app = app();
    click(&mut app, A);
    click(&mut app, B);
    let without = painted(&app);
    hover(&mut app, [120.0, 100.0]);
    app.ui.tool_options.perspective_crop_grid = true;
    let with_grid = painted(&app);
    app.ui.tool_options.perspective_crop_grid = false;
    let without_grid = painted(&app);
    assert_eq!(without_grid, without, "with Show Grid off the outline replaces the line between the clicks");
    assert!(with_grid > without_grid, "Show Grid adds the grid: {with_grid} vs {without_grid}");
}

/// Real egui input: the pointer moving over the canvas with the button up reaches the tool, so
/// the preview follows it.
#[test]
fn hover_over_the_canvas_moves_the_preview() {
    use egui::{Event, Modifiers, PointerButton};
    let mut a = app();
    a.ui.extras.snap = false;
    a.ui.extras.rulers = false;
    a.ui.views = vec![crate::state::View {
        zoom: 2.0,
        center: [100.0, 75.0],
        fit_pending: false,
        fill_pending: false,
        doc_size: [200, 150],
        rotation: 0.0,
        zoom_animating: false,
        zoom_anchor: None,
        display_camera: None,
        zoom_transition: None,
    }];
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(600.0, 400.0)).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            let v = app.ui.views[0].clone();
            crate::canvas::canvas_view(app, ui, 0, ui.max_rect(), v, true);
        },
        a,
    );
    h.run_steps(2);
    let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(600.0, 400.0));
    let xf = ViewXform { rect, zoom: 2.0, center: [100.0, 75.0], flip: false, rotation: 0.0, aspect: 1.0 };
    let button = |pos, pressed| Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
    for p in [[60.0, 40.0], [140.0, 40.0]] {
        let at = xf.to_screen(p[0], p[1]);
        h.event(Event::PointerMoved(at));
        h.step();
        h.event(button(at, true));
        h.step();
        h.event(button(at, false));
        h.step();
    }
    assert_eq!(h.state().pcrop.clicks.len(), 2, "{:?}", h.state().pcrop.clicks);
    let to = xf.to_screen(110.0, 100.0);
    h.event(Event::PointerMoved(to));
    h.step();
    let q = click_preview(h.state()).expect("a preview after two clicks and a move");
    assert!((q[2][0] - 110.0).abs() < 0.5 && (q[2][1] - 100.0).abs() < 0.5, "{q:?}");
    assert_eq!(q[2], q[3]);
}

/// #13065: a drag that builds no box, which is what a start the snap moved onto its release
/// point gives, keeps the corners clicked so far, so the next click still finishes the quad.
#[test]
fn a_drag_that_makes_no_box_keeps_the_clicked_corners() {
    let mut app = app();
    for p in [A, B, C] {
        click(&mut app, p);
    }
    // A straight drag has no area, so `new_box` returns None and no quad replaces the clicks.
    pointer(&mut app, ToolEvent::Down { x: 60.0, y: 70.0, pressure: 1.0 }, egui::Modifiers::NONE);
    pointer(&mut app, ToolEvent::Move { x: 120.0, y: 70.0, pressure: 1.0 }, egui::Modifiers::NONE);
    pointer(&mut app, ToolEvent::Up { x: 120.0, y: 70.0 }, egui::Modifiers::NONE);
    assert_eq!(app.ui.perspective_crop, None, "a drag with no area builds no box");
    assert_eq!(app.pcrop.clicks, vec![A, B, C], "the three clicks stay");
    click(&mut app, D);
    assert!(app.pcrop.clicks.is_empty(), "the fourth click still finishes the quad");
    assert_eq!(app.ui.perspective_crop, Some([A, B, C, D]));
}
