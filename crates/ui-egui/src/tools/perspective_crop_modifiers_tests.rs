//! #11802: ⇧ and ⌥ constrain the Perspective Crop's new drag (a square, from the centre) and its
//! corner drags (one axis, stays a rectangle). Ctrl, and no modifier, keep today's behaviour.

use std::sync::Arc;

use egui::Modifiers;

use super::*;

const SHIFT: Modifiers = Modifiers::SHIFT;
const ALT: Modifiers = Modifiers::ALT;

/// An 800 × 500 document with the Perspective Crop tool.
fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 800, "height": 500})).unwrap();
    app.sync_views();
    app.ui.tool = Tool::PerspectiveCrop;
    app
}

/// A press at `from`, a move and the release at `to`, all with `mods`.
fn drag(app: &mut PhotocraftApp, from: [f64; 2], to: [f64; 2], mods: Modifiers) {
    assert!(pointer(app, ToolEvent::Down { x: from[0], y: from[1], pressure: 1.0 }, mods));
    let mid = [(from[0] + to[0]) / 2.0, (from[1] + to[1]) / 2.0];
    assert!(pointer(app, ToolEvent::Move { x: mid[0], y: mid[1], pressure: 1.0 }, mods));
    assert!(pointer(app, ToolEvent::Move { x: to[0], y: to[1], pressure: 1.0 }, mods));
    assert!(pointer(app, ToolEvent::Up { x: to[0], y: to[1] }, mods));
}

fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> Option<[[f64; 2]; 4]> {
    Some([[x0, y0], [x1, y0], [x1, y1], [x0, y1]])
}

#[test]
fn shift_drag_draws_a_square_of_the_shorter_side() {
    let mut app = app();
    drag(&mut app, [100.0, 100.0], [522.0, 320.0], SHIFT);
    assert_eq!(app.ui.perspective_crop, rect(100.0, 100.0, 320.0, 320.0), "422 x 220 drag: 220 x 220");
    // Up and to the left: anchored at the press, extending the drag's way.
    app.ui.perspective_crop = None;
    drag(&mut app, [500.0, 400.0], [450.0, 100.0], SHIFT);
    assert_eq!(app.ui.perspective_crop, rect(450.0, 350.0, 500.0, 400.0));
}

#[test]
fn alt_drag_draws_from_the_centre() {
    let mut app = app();
    drag(&mut app, [400.0, 250.0], [600.0, 350.0], ALT);
    assert_eq!(app.ui.perspective_crop, rect(200.0, 150.0, 600.0, 350.0));
}

#[test]
fn shift_alt_drag_draws_a_centred_square() {
    let mut app = app();
    drag(&mut app, [400.0, 250.0], [600.0, 350.0], SHIFT | ALT);
    assert_eq!(app.ui.perspective_crop, rect(300.0, 150.0, 500.0, 350.0), "side twice the shorter drag side");
}

#[test]
fn the_live_box_follows_the_modifiers() {
    let mut app = app();
    assert!(pointer(&mut app, ToolEvent::Down { x: 400.0, y: 250.0, pressure: 1.0 }, ALT));
    assert!(pointer(&mut app, ToolEvent::Move { x: 600.0, y: 350.0, pressure: 1.0 }, ALT));
    let Some(Drag::New { start, cur, square, centred }) = app.pcrop.drag else { panic!("drawing a box") };
    assert_eq!(new_box(start, cur, square, centred), rect(200.0, 150.0, 600.0, 350.0), "the overlay draws the centred box");
}

#[test]
fn shift_corner_drag_keeps_one_axis() {
    let mut app = app();
    let q = [[100.0, 100.0], [500.0, 100.0], [500.0, 400.0], [100.0, 400.0]];
    // Mostly horizontal: only x moves.
    app.ui.perspective_crop = Some(q);
    drag(&mut app, [500.0, 400.0], [600.0, 430.0], SHIFT);
    assert_eq!(app.ui.perspective_crop.unwrap()[2], [600.0, 400.0]);
    // Mostly vertical: only y moves.
    app.ui.perspective_crop = Some(q);
    drag(&mut app, [500.0, 400.0], [520.0, 470.0], SHIFT);
    assert_eq!(app.ui.perspective_crop.unwrap()[2], [500.0, 470.0]);
    assert_eq!(app.ui.perspective_crop.unwrap()[..2], q[..2], "the other corners stay");
}

#[test]
fn alt_corner_drag_keeps_a_rectangle() {
    let mut app = app();
    app.ui.perspective_crop = Some([[100.0, 100.0], [500.0, 100.0], [500.0, 400.0], [100.0, 400.0]]);
    drag(&mut app, [500.0, 400.0], [740.0, 430.0], ALT);
    assert_eq!(app.ui.perspective_crop, rect(100.0, 100.0, 740.0, 430.0), "opposite corner fixed, neighbours follow");
    // From the top-left corner too.
    drag(&mut app, [100.0, 100.0], [50.0, 160.0], ALT);
    assert_eq!(app.ui.perspective_crop, rect(50.0, 160.0, 740.0, 430.0));
}

/// #11802: ⌥ dragging a corner past the opposite one on both axes keeps today's result: the
/// rectangle turned 180° (the corners keep their order, so it commits upside down). Crossing on
/// one axis only is a separate item.
#[test]
fn alt_corner_drag_past_the_opposite_corner_on_both_axes_turns_the_rectangle_180_degrees() {
    let mut app = app();
    app.ui.perspective_crop = Some([[100.0, 100.0], [500.0, 100.0], [500.0, 400.0], [100.0, 400.0]]);
    // Where the turned rectangle will lie (40,60 to 100,100): a red patch in its bottom right and
    // a green one in its top left.
    let patch = |app: &mut PhotocraftApp, x: i32, y: i32, c: [f32; 4]| {
        Arc::make_mut(&mut app.session.active_mut().unwrap().doc).layers[0]
            .surface_mut()
            .unwrap()
            .fill_rect(photocraft_geom::Rect::from_xywh(x, y, 10, 10), &c);
    };
    patch(&mut app, 90, 90, [1.0, 0.0, 0.0, 1.0]);
    patch(&mut app, 40, 60, [0.0, 1.0, 0.0, 1.0]);
    // The bottom right corner goes above and to the left of the fixed top left one.
    drag(&mut app, [500.0, 400.0], [40.0, 60.0], ALT);
    assert_eq!(
        app.ui.perspective_crop,
        Some([[100.0, 100.0], [40.0, 100.0], [40.0, 60.0], [100.0, 60.0]]),
        "the opposite corner stays, the neighbours follow, the corners keep their order: the rectangle turned 180 degrees"
    );
    // Committing keeps that: a 60 x 40 canvas upside down, the red patch (the old bottom right)
    // at the top left and the green one (the old top left) at the bottom right.
    commit(&mut app);
    assert!(app.ui.perspective_crop.is_none(), "committed");
    let doc = &app.session.active().unwrap().doc;
    assert_eq!((doc.size.width, doc.size.height), (60, 40));
    let surf = doc.layers[0].surface().unwrap();
    assert_eq!(surf.rgba(3, 3), [1.0, 0.0, 0.0, 1.0], "upside down: the red patch is at the top left");
    assert_eq!(surf.rgba(56, 36), [0.0, 1.0, 0.0, 1.0], "upside down: the green patch is at the bottom right");
}

#[test]
fn alt_corner_drag_on_a_skewed_quad_moves_the_corner_alone() {
    // Not measured: today's single-corner drag.
    let mut app = app();
    let q = [[100.0, 100.0], [500.0, 130.0], [480.0, 400.0], [120.0, 380.0]];
    app.ui.perspective_crop = Some(q);
    drag(&mut app, q[2], [700.0, 450.0], ALT);
    assert_eq!(app.ui.perspective_crop, Some([q[0], q[1], [700.0, 450.0], q[3]]));
}

#[test]
fn no_modifier_and_ctrl_keep_todays_behaviour() {
    for mods in [Modifiers::NONE, Modifiers::CTRL, Modifiers::COMMAND] {
        let mut app = app();
        drag(&mut app, [100.0, 100.0], [522.0, 320.0], mods);
        assert_eq!(app.ui.perspective_crop, rect(100.0, 100.0, 522.0, 320.0), "{mods:?}");
        drag(&mut app, [522.0, 320.0], [740.0, 430.0], mods);
        assert_eq!(app.ui.perspective_crop, Some([[100.0, 100.0], [522.0, 100.0], [740.0, 430.0], [100.0, 320.0]]), "a single-corner drag");
        drag(&mut app, [300.0, 200.0], [310.0, 220.0], mods);
        assert_eq!(app.ui.perspective_crop.unwrap()[0], [110.0, 120.0], "a press inside moves the quad");
        // Four clicks.
        let mut app = self::app();
        for p in [[10.0, 10.0], [90.0, 12.0], [88.0, 70.0], [12.0, 72.0]] {
            assert!(pointer(&mut app, ToolEvent::Down { x: p[0], y: p[1], pressure: 1.0 }, mods));
            assert!(pointer(&mut app, ToolEvent::Up { x: p[0], y: p[1] }, mods));
        }
        assert_eq!(app.ui.perspective_crop, Some([[10.0, 10.0], [90.0, 12.0], [88.0, 70.0], [12.0, 72.0]]));
    }
}

#[test]
fn zero_size_drags_and_degenerate_quads_never_panic() {
    for mods in [SHIFT, ALT, SHIFT | ALT, Modifiers::CTRL] {
        let mut app = app();
        // A straight drag: no box.
        drag(&mut app, [100.0, 100.0], [300.0, 100.0], mods);
        assert_eq!(app.ui.perspective_crop, None, "{mods:?}");
        // All four corners on one point.
        app.ui.perspective_crop = Some([[200.0, 200.0]; 4]);
        drag(&mut app, [200.0, 200.0], [260.0, 230.0], mods);
        // A quad with a line for a side.
        app.ui.perspective_crop = Some([[100.0, 100.0], [100.0, 100.0], [400.0, 300.0], [100.0, 300.0]]);
        drag(&mut app, [400.0, 300.0], [420.0, 360.0], mods);
        assert!(new_box([1.0, 1.0], [1.0, 1.0], true, true).is_none());
    }
}

#[test]
fn releasing_alt_mid_corner_drag_puts_the_neighbours_back() {
    // ⌥ released during the drag: the corner then moves alone, from the quad at the press.
    let mut app = app();
    let q = [[100.0, 100.0], [500.0, 100.0], [500.0, 400.0], [100.0, 400.0]];
    app.ui.perspective_crop = Some(q);
    assert!(pointer(&mut app, ToolEvent::Down { x: 500.0, y: 400.0, pressure: 1.0 }, ALT));
    assert!(pointer(&mut app, ToolEvent::Move { x: 740.0, y: 430.0, pressure: 1.0 }, ALT));
    assert_eq!(app.ui.perspective_crop, rect(100.0, 100.0, 740.0, 430.0));
    assert!(pointer(&mut app, ToolEvent::Move { x: 700.0, y: 450.0, pressure: 1.0 }, Modifiers::NONE));
    assert_eq!(app.ui.perspective_crop, Some([q[0], q[1], [700.0, 450.0], q[3]]), "the preview during the drag");
    assert!(pointer(&mut app, ToolEvent::Up { x: 700.0, y: 450.0 }, Modifiers::NONE));
    assert_eq!(app.ui.perspective_crop, Some([q[0], q[1], [700.0, 450.0], q[3]]));
}
