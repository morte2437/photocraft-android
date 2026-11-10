//! #11729: Color Sampler drag gestures, as in Photoshop. A new point follows the pointer and lands
//! where the drag ends, a press on a point drags it (a click on it moves it to the clicked pixel),
//! ⌥-click off a point adds one, and Escape cancels either drag.

use super::*;
use egui::Modifiers;

const NONE: Modifiers = Modifiers::NONE;
const ALT: Modifiers = Modifiers { alt: true, ctrl: false, shift: false, mac_cmd: false, command: false };

/// A 200 × 100 document with the Color Sampler tool.
fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
    app.ui.tool = Tool::ColorSampler;
    app
}

fn down(app: &mut PhotocraftApp, x: f64, y: f64, m: Modifiers) {
    crate::canvas::tool_event(app, ToolEvent::Down { x, y, pressure: 1.0 }, m);
}

fn mv(app: &mut PhotocraftApp, x: f64, y: f64) {
    crate::canvas::tool_event(app, ToolEvent::Move { x, y, pressure: 1.0 }, NONE);
}

fn up(app: &mut PhotocraftApp, x: f64, y: f64) {
    crate::canvas::tool_event(app, ToolEvent::Up { x, y }, NONE);
}

fn drag(app: &mut PhotocraftApp, from: [f64; 2], to: [f64; 2]) {
    down(app, from[0], from[1], NONE);
    mv(app, (from[0] + to[0]) / 2.0, (from[1] + to[1]) / 2.0);
    mv(app, to[0], to[1]);
    up(app, to[0], to[1]);
}

fn samplers(app: &PhotocraftApp) -> Vec<[f64; 2]> {
    app.session.active().unwrap().color_samplers.clone()
}

/// The pixel each point sits on (the stored position floors to it).
fn pixels(app: &PhotocraftApp) -> Vec<[i64; 2]> {
    samplers(app).iter().map(|p| [p[0].floor() as i64, p[1].floor() as i64]).collect()
}

/// Escape through the app's shortcut handling, as the canvas gets it.
fn escape(app: &mut PhotocraftApp) {
    let ctx = egui::Context::default();
    let raw = egui::RawInput {
        events: vec![
            egui::Event::ModifiersChanged(NONE),
            egui::Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: NONE },
        ],
        ..Default::default()
    };
    ctx.begin_pass(raw);
    crate::shortcuts::handle(app, &ctx);
    ctx.end_pass().textures_delta.clear();
}

#[test]
fn a_new_point_follows_the_pointer_and_lands_where_the_drag_ends() {
    let mut app = app();
    down(&mut app, 120.0, 40.0, NONE);
    assert!(samplers(&app).is_empty(), "nothing is placed on the press");
    mv(&mut app, 135.0, 50.0);
    assert_eq!(app.ui.analysis.drag, Some(Drag::SamplerNew([135.0, 50.0])), "the marker follows the pointer");
    mv(&mut app, 150.0, 60.0);
    up(&mut app, 150.0, 60.0);
    assert_eq!(pixels(&app), vec![[150, 60]]);
    assert_eq!(app.ui.analysis.drag, None);
}

#[test]
fn a_drag_ending_off_the_canvas_places_nothing_and_one_from_the_pasteboard_places_one() {
    let mut app = app();
    drag(&mut app, [150.0, 50.0], [230.0, 50.0]);
    assert!(samplers(&app).is_empty(), "ended on the pasteboard");
    drag(&mut app, [-20.0, 50.0], [30.0, 40.0]);
    assert_eq!(pixels(&app), vec![[30, 40]], "started on the pasteboard, ended on the canvas");
}

#[test]
fn a_plain_click_off_a_point_still_places_one_at_that_pixel() {
    let mut app = app();
    down(&mut app, 12.4, 7.9, NONE);
    up(&mut app, 12.4, 7.9);
    assert_eq!(pixels(&app), vec![[12, 7]]);
    // A click on the pasteboard places nothing.
    down(&mut app, -5.0, 7.0, NONE);
    up(&mut app, -5.0, 7.0);
    assert_eq!(samplers(&app).len(), 1);
}

#[test]
fn dragging_a_point_moves_its_marker_live_and_the_point_on_release() {
    let mut app = app();
    app.run("view.colorSamplers.add", json!({"x": 50, "y": 50})).unwrap();
    app.run("view.colorSamplers.add", json!({"x": 100, "y": 20})).unwrap();
    let before = samplers(&app);
    down(&mut app, before[0][0], before[0][1], NONE);
    mv(&mut app, 70.0, 30.0);
    assert_eq!(app.ui.analysis.drag, Some(Drag::Sampler(0, [70.0, 30.0])), "the marker follows the pointer");
    assert_eq!(samplers(&app), before, "the point itself moves on release");
    mv(&mut app, 80.0, 20.0);
    up(&mut app, 80.0, 20.0);
    assert_eq!(pixels(&app), vec![[80, 20], [100, 20]], "#1 moved, #2 untouched, numbering kept");
}

#[test]
fn a_click_on_a_point_two_pixels_off_moves_it_to_the_clicked_pixel() {
    let mut app = app();
    app.run("view.colorSamplers.add", json!({"x": 50, "y": 50})).unwrap();
    let p = samplers(&app)[0];
    down(&mut app, p[0] + 2.0, p[1] + 1.0, NONE);
    up(&mut app, p[0] + 2.0, p[1] + 1.0);
    assert_eq!(pixels(&app), vec![[52, 51]]);
}

#[test]
fn dragging_a_point_off_the_document_still_deletes_it() {
    let mut app = app();
    app.run("view.colorSamplers.add", json!({"x": 50, "y": 50})).unwrap();
    app.run("view.colorSamplers.add", json!({"x": 100, "y": 20})).unwrap();
    let p = samplers(&app)[0];
    drag(&mut app, p, [-30.0, 50.0]);
    assert_eq!(pixels(&app), vec![[100, 20]]);
}

#[test]
fn alt_click_off_a_point_adds_one_and_on_a_point_deletes_it() {
    let mut app = app();
    down(&mut app, 60.0, 30.0, ALT);
    up(&mut app, 60.0, 30.0);
    assert_eq!(pixels(&app), vec![[60, 30]], "⌥-click off a point adds one, like a plain click");
    let p = samplers(&app)[0];
    down(&mut app, p[0], p[1], ALT);
    up(&mut app, p[0], p[1]);
    assert!(samplers(&app).is_empty(), "⌥-click on a point deletes it");
}

#[test]
fn escape_cancels_either_drag_and_leaves_the_list_unchanged() {
    // A new point: Escape mid-drag, then the release places nothing.
    let mut app = app();
    down(&mut app, 40.0, 40.0, NONE);
    mv(&mut app, 60.0, 50.0);
    escape(&mut app);
    assert_eq!(app.ui.analysis.drag, None);
    up(&mut app, 60.0, 50.0);
    assert!(samplers(&app).is_empty(), "no new point");
    // An existing point stays where it was.
    app.run("view.colorSamplers.add", json!({"x": 50, "y": 50})).unwrap();
    let before = samplers(&app);
    down(&mut app, before[0][0], before[0][1], NONE);
    mv(&mut app, 90.0, 10.0);
    escape(&mut app);
    up(&mut app, 90.0, 10.0);
    assert_eq!(samplers(&app), before, "the dragged point stays put");
}

#[test]
fn the_dragged_and_the_new_marker_are_drawn_at_the_pointer() {
    let mut app = app();
    app.run("view.colorSamplers.add", json!({"x": 50, "y": 50})).unwrap();
    let ctx = egui::Context::default();
    let xf = ViewXform {
        rect: egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(200.0, 100.0)),
        zoom: 1.0,
        center: [100.0, 50.0],
        flip: false,
        rotation: 0.0,
        aspect: 1.0,
    };
    // The marker numbers the overlay paints, with where they are.
    let numbers_at = |app: &PhotocraftApp| -> Vec<(String, egui::Pos2)> {
        let mut o = ctx.run_ui(Default::default(), |ui| draw_overlay(app, ui.painter(), &xf));
        o.textures_delta.clear();
        o.shapes
            .into_iter()
            .filter_map(|c| match c.shape {
                egui::Shape::Text(t) => Some((t.galley.text().to_owned(), t.pos)),
                _ => None,
            })
            .collect()
    };
    let p = samplers(&app)[0];
    down(&mut app, p[0], p[1], NONE);
    mv(&mut app, 120.0, 30.0);
    let at = xf.to_screen(120.0, 30.0);
    let shown = numbers_at(&app);
    assert!(shown.iter().any(|(t, q)| t == "1" && (q.x - at.x).abs() < 20.0 && (q.y - at.y).abs() < 20.0), "#1 follows the pointer: {shown:?}");
    up(&mut app, 120.0, 30.0);
    down(&mut app, 20.0, 80.0, NONE);
    mv(&mut app, 30.0, 70.0);
    let at = xf.to_screen(30.0, 70.0);
    let shown = numbers_at(&app);
    assert!(shown.iter().any(|(t, q)| t == "2" && (q.x - at.x).abs() < 20.0 && (q.y - at.y).abs() < 20.0), "the new #2 follows the pointer: {shown:?}");
}

#[test]
fn at_the_ten_point_limit_a_drag_off_a_point_previews_no_new_marker() {
    let mut app = app();
    for i in 0..10 {
        app.run("view.colorSamplers.add", json!({"x": 10 + 15 * i, "y": 10})).unwrap();
    }
    let ctx = egui::Context::default();
    let xf = ViewXform {
        rect: egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(200.0, 100.0)),
        zoom: 1.0,
        center: [100.0, 50.0],
        flip: false,
        rotation: 0.0,
        aspect: 1.0,
    };
    down(&mut app, 50.0, 70.0, NONE);
    mv(&mut app, 60.0, 80.0);
    let mut o = ctx.run_ui(Default::default(), |ui| draw_overlay(&app, ui.painter(), &xf));
    o.textures_delta.clear();
    let numbers: Vec<String> = o
        .shapes
        .into_iter()
        .filter_map(|c| match c.shape {
            egui::Shape::Text(t) => Some(t.galley.text().to_owned()),
            _ => None,
        })
        .collect();
    assert!(!numbers.iter().any(|t| t == "11"), "no 11th marker that the release cannot place: {numbers:?}");
    up(&mut app, 60.0, 80.0);
    assert_eq!(samplers(&app).len(), 10);
}

#[test]
fn a_sampler_drag_whose_release_went_to_another_tool_leaves_escape_alone() {
    let mut app = app();
    app.run("select.all", json!({})).unwrap();
    down(&mut app, 40.0, 40.0, NONE);
    mv(&mut app, 60.0, 50.0);
    // A tool letter mid-drag: the release goes to the new tool.
    crate::tool_memory::select(&mut app, Tool::Hand);
    up(&mut app, 60.0, 50.0);
    escape(&mut app);
    assert_eq!(app.ui.analysis.drag, None, "the stale drag is dropped");
    assert!(app.session.active().unwrap().doc.selection.is_none(), "Escape reached Deselect, not the stale sampler drag");
}
