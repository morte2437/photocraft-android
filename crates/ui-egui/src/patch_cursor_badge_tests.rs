//! The Patch tool's cursor shows a right-pointing arrow over the selection (#12130).

use egui::Modifiers;
use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::Drag;
use crate::state::Tool;
use crate::tool_feedback::{Badge, badge, badge_glyph};

fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
    app.run("select.rect", json!({"x": 20, "y": 20, "width": 30, "height": 30})).unwrap();
    app
}

const INSIDE: [f64; 2] = [30.0, 30.0];
const OUTSIDE: [f64; 2] = [80.0, 80.0];

#[test]
fn arrow_over_the_selection_without_modifiers() {
    let mut app = app();
    app.hover_doc = Some(INSIDE);
    assert_eq!(badge(&app, Tool::Patch, Modifiers::NONE), Some(Badge::Arrow));
    assert_ne!(Badge::Arrow, Badge::Move, "a new single-arrow glyph");
    // Size 4 is the half-length h of the shaft; the barbs fall back by a = 0.6 h.
    let (h, a, eps) = (4.0_f32, 2.4_f32, 1e-5_f32);
    let glyph = badge_glyph(Badge::Arrow, egui::pos2(0.0, 0.0), h);
    assert_eq!(glyph.len(), 3, "one shaft and two barbs");
    let near = |p: egui::Pos2, x: f32, y: f32| (p.x - x).abs() < eps && (p.y - y).abs() < eps;
    assert!(near(glyph[0][0], -h, 0.0) && near(glyph[0][1], h, 0.0), "shaft through the centre: {:?}", glyph[0]);
    for barb in &glyph[1..] {
        assert!(near(barb[0], h, 0.0), "a barb starts at the tip: {barb:?}");
    }
    let (up, down) = (glyph[1][1], glyph[2][1]);
    assert!((up.x - (h - a)).abs() < eps && (down.x - (h - a)).abs() < eps, "barbs end behind the tip");
    assert!((up.y + down.y).abs() < eps && (up.y.abs() - a).abs() < eps, "barb ends mirror about the shaft: {up:?} {down:?}");
}

#[test]
fn the_arrow_stays_during_a_drag_started_inside() {
    let mut app = app();
    app.drag = Some(Drag::new(Tool::Patch, INSIDE, vec![[INSIDE[0], INSIDE[1], 1.0]], Modifiers::NONE, false));
    app.hover_doc = Some(OUTSIDE);
    assert_eq!(badge(&app, Tool::Patch, Modifiers::NONE), Some(Badge::Arrow));
}

#[test]
fn outside_and_with_shift_or_alt_the_badges_are_as_before() {
    let mut app = app();
    app.hover_doc = Some(OUTSIDE);
    assert_eq!(badge(&app, Tool::Patch, Modifiers::NONE), None);
    app.hover_doc = Some(INSIDE);
    assert_eq!(badge(&app, Tool::Patch, Modifiers::SHIFT), Some(Badge::Add));
    assert_eq!(badge(&app, Tool::Patch, Modifiers::ALT), Some(Badge::Subtract));
}

#[test]
fn a_patch_drag_started_outside_does_not_show_the_arrow_when_it_enters_the_selection() {
    let mut app = app();
    app.hover_doc = Some(OUTSIDE);
    let plain_outside = badge(&app, Tool::Patch, Modifiers::NONE);
    app.drag = Some(Drag::new(Tool::Patch, OUTSIDE, vec![[OUTSIDE[0], OUTSIDE[1], 1.0]], Modifiers::NONE, false));
    app.hover_doc = Some(INSIDE);
    assert_eq!(badge(&app, Tool::Patch, Modifiers::NONE), plain_outside);
    assert_ne!(badge(&app, Tool::Patch, Modifiers::NONE), Some(Badge::Arrow));
}

#[test]
fn content_aware_move_has_no_arrow_even_in_a_drag_started_inside() {
    let mut app = app();
    app.drag = Some(Drag::new(Tool::ContentAwareMove, INSIDE, vec![[INSIDE[0], INSIDE[1], 1.0]], Modifiers::NONE, false));
    app.hover_doc = Some(INSIDE);
    assert_eq!(badge(&app, Tool::ContentAwareMove, Modifiers::NONE), None);
}

/// The release reads the press's modifiers, so ⇧ or ⌥ pressed mid-drag changes nothing.
#[test]
fn modifiers_pressed_mid_drag_do_not_change_the_badge() {
    let mut app = app();
    app.drag = Some(Drag::new(Tool::Patch, INSIDE, vec![[INSIDE[0], INSIDE[1], 1.0]], Modifiers::NONE, false));
    app.hover_doc = Some(OUTSIDE);
    for m in [Modifiers::SHIFT, Modifiers::ALT, Modifiers::SHIFT | Modifiers::ALT] {
        assert_eq!(badge(&app, Tool::Patch, m), Some(Badge::Arrow), "{m:?}");
    }
}

#[test]
fn a_drag_pressed_with_shift_or_alt_keeps_that_badge() {
    let mut app = app();
    app.hover_doc = Some(INSIDE);
    for (pressed, want) in [(Modifiers::SHIFT, Badge::Add), (Modifiers::ALT, Badge::Subtract)] {
        app.drag = Some(Drag::new(Tool::Patch, INSIDE, vec![[INSIDE[0], INSIDE[1], 1.0]], pressed, false));
        assert_eq!(badge(&app, Tool::Patch, pressed), Some(want));
        assert_eq!(badge(&app, Tool::Patch, Modifiers::NONE), Some(want), "released keys mid-drag");
    }
}

#[test]
fn shift_and_alt_together_over_the_selection_intersect() {
    let mut app = app();
    app.hover_doc = Some(INSIDE);
    assert_eq!(badge(&app, Tool::Patch, Modifiers::SHIFT | Modifiers::ALT), Some(Badge::Intersect));
}

#[test]
fn content_aware_move_and_the_curvature_pen_keep_their_cursors() {
    let mut app = app();
    app.hover_doc = Some(INSIDE);
    assert_eq!(badge(&app, Tool::ContentAwareMove, Modifiers::NONE), None);
    // The Curvature Pen over an anchor of the active path: four arrows.
    let path = json!({"subpaths": [{"closed": true, "knots": [[40, 40], [160, 40], [160, 160], [40, 160]]}]});
    app.run("path.set", json!({"name": "work", "path": path})).unwrap();
    app.ui.tool = Tool::CurvaturePen;
    app.hover_doc = Some([160.0, 40.0]);
    assert_eq!(badge(&app, Tool::CurvaturePen, Modifiers::NONE), Some(Badge::Move));
    assert_eq!(badge_glyph(Badge::Move, egui::pos2(0.0, 0.0), 4.0).len(), 10, "four-arrow glyph unchanged");
}
