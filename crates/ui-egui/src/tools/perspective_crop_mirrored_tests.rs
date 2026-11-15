//! #12489: a Perspective Crop box flipped on one axis (⌥ dragging a corner past the opposite one
//! along x only) is wound counter-clockwise. It is drawn valid and commits mirrored, each corner
//! mapped as it lies, as in the reference application; `image.perspectiveCrop` takes the same
//! corners. Clicked corners still commit upright, a box turned 180° still commits upside down, and
//! self-crossing and degenerate quads stay invalid.

use std::sync::Arc;

use egui::{Modifiers, Pos2, Rect, vec2};
use serde_json::json;

use super::*;

const RED: [f32; 4] = [1.0, 0.0, 0.0, 1.0];
const GREEN: [f32; 4] = [0.0, 1.0, 0.0, 1.0];
const BLUE: [f32; 4] = [0.0, 0.0, 1.0, 1.0];
const YELLOW: [f32; 4] = [1.0, 1.0, 0.0, 1.0];

/// The measured test document: 800 x 500, red (top left), green (top right), blue (bottom left)
/// and yellow (bottom right) quadrants with a black square in the middle; the Perspective Crop tool.
fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 800, "height": 500})).unwrap();
    app.sync_views();
    app.ui.tool = Tool::PerspectiveCrop;
    let surf = Arc::make_mut(&mut app.session.active_mut().unwrap().doc).layers[0].surface_mut().unwrap();
    for (x, y, c) in [(0, 0, RED), (400, 0, GREEN), (0, 250, BLUE), (400, 250, YELLOW)] {
        surf.fill_rect(photocraft_geom::Rect::from_xywh(x, y, 400, 250), &c);
    }
    surf.fill_rect(photocraft_geom::Rect::from_xywh(395, 245, 10, 10), &[0.0, 0.0, 0.0, 1.0]);
    app
}

fn drag(app: &mut PhotocraftApp, from: [f64; 2], to: [f64; 2], mods: Modifiers) {
    assert!(pointer(app, ToolEvent::Down { x: from[0], y: from[1], pressure: 1.0 }, mods));
    let mid = [(from[0] + to[0]) / 2.0, (from[1] + to[1]) / 2.0];
    assert!(pointer(app, ToolEvent::Move { x: mid[0], y: mid[1], pressure: 1.0 }, mods));
    assert!(pointer(app, ToolEvent::Move { x: to[0], y: to[1], pressure: 1.0 }, mods));
    assert!(pointer(app, ToolEvent::Up { x: to[0], y: to[1] }, mods));
}

fn click(app: &mut PhotocraftApp, p: [f64; 2]) {
    drag(app, p, p, Modifiers::NONE);
}

/// The overlay `draw_overlay` paints: (shield polygons, whether the frame is red).
fn overlay(app: &PhotocraftApp) -> (usize, bool) {
    let ctx = egui::Context::default();
    let xf =
        ViewXform { rect: Rect::from_min_size(Pos2::ZERO, vec2(800.0, 500.0)), zoom: 1.0, center: [400.0, 250.0], flip: false, rotation: 0.0, aspect: 1.0 };
    let mut o = ctx.run_ui(Default::default(), |ui| draw_overlay(app, ui.painter(), &xf));
    o.textures_delta.clear();
    let shield = o.shapes.iter().filter(|c| matches!(&c.shape, egui::Shape::Path(p) if p.fill == SHIELD)).count();
    let red = Color32::from_rgb(0xe5, 0x3e, 0x3e);
    let frame_red = o.shapes.iter().any(|c| matches!(&c.shape, egui::Shape::Path(p) if p.closed && p.stroke.color == egui::epaint::ColorMode::Solid(red)));
    (shield, frame_red)
}

/// The committed canvas's size and the colours at its four corners (TL, TR, BL, BR).
fn corners(app: &PhotocraftApp) -> ((u32, u32), [[f32; 4]; 4]) {
    let doc = &app.session.active().unwrap().doc;
    let (w, h) = (doc.size.width as i32, doc.size.height as i32);
    let surf = doc.layers[0].surface().unwrap();
    ((doc.size.width, doc.size.height), [surf.rgba(3, 3), surf.rgba(w - 4, 3), surf.rgba(3, h - 4), surf.rgba(w - 4, h - 4)])
}

#[test]
fn alt_corner_drag_past_the_opposite_corner_on_one_axis_commits_mirrored() {
    let mut app = app();
    // A fresh 100 x 100 box, its bottom right corner ⌥ dragged past the fixed top left one on x only.
    drag(&mut app, [450.0, 220.0], [550.0, 320.0], Modifiers::NONE);
    drag(&mut app, [550.0, 320.0], [350.0, 300.0], Modifiers::ALT);
    assert_eq!(app.ui.perspective_crop, Some([[450.0, 220.0], [350.0, 220.0], [350.0, 300.0], [450.0, 300.0]]), "flipped left to right");
    let (shield, red) = overlay(&app);
    assert!(shield > 0 && !red, "drawn as a valid quad: accent frame and shield, not red");
    commit(&mut app);
    assert!(app.ui.perspective_crop.is_none(), "committed");
    let (size, [tl, tr, bl, br]) = corners(&app);
    assert_eq!(size, (100, 80));
    assert_eq!([tl, tr, bl, br], [GREEN, RED, YELLOW, BLUE], "mirrored left to right, top stays on top");
}

#[test]
fn the_command_takes_counter_clockwise_corners_and_mirrors() {
    let mut app = app();
    let r = app.run("image.perspectiveCrop", json!({"corners": [[450, 220], [350, 220], [350, 300], [450, 300]]})).unwrap();
    assert_eq!((r["width"].as_u64(), r["height"].as_u64()), (Some(100), Some(80)));
    assert_eq!(corners(&app).1, [GREEN, RED, YELLOW, BLUE]);
    // Mirrored top to bottom the other way round.
    let mut app = self::app();
    app.run("image.perspectiveCrop", json!({"corners": [[350, 300], [450, 300], [450, 220], [350, 220]]})).unwrap();
    assert_eq!(corners(&app).1, [BLUE, YELLOW, RED, GREEN]);
}

#[test]
fn clicked_corners_commit_upright_in_any_order() {
    let mut app = app();
    // Counter-clockwise clicks from the top left.
    for p in [[350.0, 220.0], [350.0, 300.0], [450.0, 300.0], [450.0, 220.0]] {
        click(&mut app, p);
    }
    assert_eq!(app.ui.perspective_crop, Some([[350.0, 220.0], [450.0, 220.0], [450.0, 300.0], [350.0, 300.0]]), "sorted clockwise");
    commit(&mut app);
    assert_eq!(corners(&app), ((100, 80), [RED, GREEN, BLUE, YELLOW]));
}

#[test]
fn a_box_turned_180_degrees_commits_upside_down() {
    let mut app = app();
    drag(&mut app, [450.0, 300.0], [550.0, 400.0], Modifiers::NONE);
    drag(&mut app, [550.0, 400.0], [350.0, 200.0], Modifiers::ALT);
    assert_eq!(app.ui.perspective_crop, Some([[450.0, 300.0], [350.0, 300.0], [350.0, 200.0], [450.0, 200.0]]));
    commit(&mut app);
    assert_eq!(corners(&app), ((100, 100), [YELLOW, BLUE, GREEN, RED]));
}

#[test]
fn self_crossing_and_degenerate_quads_stay_invalid() {
    for q in [
        // Self-crossing (a bow tie), each way round.
        [[350.0, 220.0], [450.0, 220.0], [350.0, 300.0], [450.0, 300.0]],
        [[450.0, 220.0], [350.0, 220.0], [450.0, 300.0], [350.0, 300.0]],
        // Three corners on a line, and no area.
        [[350.0, 220.0], [400.0, 220.0], [450.0, 220.0], [350.0, 300.0]],
        [[400.0, 250.0]; 4],
    ] {
        let mut app = app();
        app.ui.perspective_crop = Some(q);
        let (shield, red) = overlay(&app);
        assert!(shield == 0 && red, "{q:?} is drawn red without the shield");
        assert!(app.run("image.perspectiveCrop", json!({ "corners": q })).is_err(), "{q:?}");
        commit(&mut app);
        assert_eq!(app.ui.perspective_crop, Some(q), "{q:?} stays for fixing");
        assert_eq!(app.session.active().unwrap().doc.size.width, 800);
    }
    let mut app = app();
    assert!(app.run("image.perspectiveCrop", json!({"corners": [[350, 220], [f64::NAN, 220], [450, 300], [350, 300]]})).is_err());
}
