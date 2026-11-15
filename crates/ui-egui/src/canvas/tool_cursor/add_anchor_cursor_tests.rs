//! #12729: Add Anchor Point's cursor says what a click would do. Over a segment of the targeted path
//! it is the pen with "+"; over an anchor, a handle end, empty canvas, with no path targeted and
//! over a locked shape layer's segments it is the black arrow. ⌘ keeps Direct Selection's arrow,
//! ⌥ and the Precise "Other Cursors" crosshair stay as they were. The tests drive the real canvas
//! (`document_area`) and compare the OS cursor bitmap it hands the platform with the arrow's and
//! the pen's.

use egui::{CursorIcon, Modifiers, Pos2, vec2};
use egui_kittest::Harness;
use serde_json::json;

use super::{PenVariant, Shape, rasterize};
use crate::PhotocraftApp;
use crate::anchor_tools::delete_anchor_targeted_path_tests::{select_layer, select_saved_path, setup};
use crate::state::Tool;

/// Path 1 is the square (50,450)-(150,550); the shape layer is the rectangle (450,100)-(650,250).
const SQUARE_TOP: [f32; 2] = [100.0, 450.0];
const SQUARE_CORNER: [f32; 2] = [50.0, 450.0];
const SQUARE_MIDDLE: [f32; 2] = [100.0, 500.0];
const SHAPE_TOP: [f32; 2] = [550.0, 100.0];

/// The canvas at 100 % over `app`, the view centred on `view` (a document point).
fn harness(app: PhotocraftApp, view: [f32; 2]) -> Harness<'static, PhotocraftApp> {
    let mut app = app;
    app.sync_views();
    app.ui.extras.rulers = false;
    let mut h = Harness::builder().with_size(vec2(900.0, 600.0)).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            if ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                egui::CentralPanel::default().show(ui, |ui| crate::canvas::document_area(app, ui));
            }
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    h.run_steps(4);
    let v = &mut h.state_mut().ui.views[0];
    v.zoom = 1.0;
    v.center = view;
    v.fit_pending = false;
    h.run_steps(2);
    h
}

/// Hover the document point `at` with `mods` held.
fn hover(h: &mut Harness<'static, PhotocraftApp>, at: [f32; 2], view: [f32; 2], mods: Modifiers) {
    h.event(egui::Event::ModifiersChanged(mods));
    let p: Pos2 = h.state().last_canvas_rect.center() + vec2(at[0] - view[0], at[1] - view[1]);
    h.hover_at(p);
    h.run_steps(3);
}

#[derive(Debug, PartialEq)]
enum Shown {
    Arrow,
    Pen,
    Crosshair,
    Other,
}

/// Which cursor the last frame asked the platform for.
fn shown(h: &Harness<'static, PhotocraftApp>) -> Shown {
    let out = &h.output().platform_output;
    let scale = super::native_scale(&h.ctx);
    let same =
        |a: &egui::CustomCursorImage, b: Option<egui::CustomCursorImage>| b.is_some_and(|b| a.size == b.size && a.hotspot == b.hotspot && a.rgba == b.rgba);
    match &out.cursor_image {
        Some(i) if same(i, rasterize(&Shape::Arrow, scale)) => Shown::Arrow,
        Some(i) if same(i, PenVariant::from_tool(Tool::AddAnchorPoint).and_then(|v| super::pen::rasterize(v, scale))) => Shown::Pen,
        Some(i) if same(i, rasterize(&super::STANDARD_CROSSHAIR, scale)) => Shown::Crosshair,
        _ => Shown::Other,
    }
}

/// The square's Path 1 targeted, the pointer at `at` on it.
fn on_path_one(at: [f32; 2], mods: Modifiers) -> Shown {
    let (mut app, _) = setup(Tool::AddAnchorPoint);
    select_saved_path(&mut app);
    let mut h = harness(app, SQUARE_MIDDLE);
    hover(&mut h, at, SQUARE_MIDDLE, mods);
    shown(&h)
}

#[test]
fn over_a_segment_of_the_targeted_path_it_is_the_pen_with_plus() {
    assert_eq!(on_path_one(SQUARE_TOP, Modifiers::NONE), Shown::Pen);
}

#[test]
fn over_an_anchor_a_handle_end_or_empty_canvas_it_is_the_black_arrow() {
    assert_eq!(on_path_one(SQUARE_CORNER, Modifiers::NONE), Shown::Arrow);
    assert_eq!(on_path_one(SQUARE_MIDDLE, Modifiers::NONE), Shown::Arrow);
    // A handle end: a curved top edge's out-handle ends in empty canvas, off every segment.
    let (mut app, _) = setup(Tool::AddAnchorPoint);
    let path = json!({"subpaths": [{"closed": true, "knots": [{"anchor": [50, 450], "in": [50, 450], "out": [50, 400]}, [150, 450], [150, 550], [50, 550]]}]});
    app.run("path.set", json!({"name": "Path 1", "path": path})).unwrap();
    app.ui.selected_path = Some("Path 1".into());
    let mut h = harness(app, SQUARE_MIDDLE);
    hover(&mut h, [50.0, 400.0], SQUARE_MIDDLE, Modifiers::NONE);
    assert_eq!(shown(&h), Shown::Arrow);
}

#[test]
fn with_no_path_targeted_it_is_the_black_arrow_everywhere() {
    // The active layer is the pixel layer "Red": no path is targeted, whatever lies under the pointer.
    for at in [SHAPE_TOP, [450.0, 100.0], [550.0, 175.0], [300.0, 150.0]] {
        let (app, _) = setup(Tool::AddAnchorPoint);
        let mut h = harness(app, SHAPE_TOP);
        hover(&mut h, at, SHAPE_TOP, Modifiers::NONE);
        assert_eq!(shown(&h), Shown::Arrow, "{at:?}");
    }
}

#[test]
fn over_a_segment_of_a_locked_shape_layers_mask_it_is_the_black_arrow() {
    let (mut app, shape) = setup(Tool::AddAnchorPoint);
    select_layer(&mut app, shape);
    let mut h = harness(app, SHAPE_TOP);
    hover(&mut h, SHAPE_TOP, SHAPE_TOP, Modifiers::NONE);
    assert_eq!(shown(&h), Shown::Pen, "unlocked, the same point is a segment");
    h.state_mut().run("layer.lockLayers", json!({"position": true})).unwrap();
    hover(&mut h, SHAPE_TOP, SHAPE_TOP, Modifiers::NONE);
    assert_eq!(shown(&h), Shown::Arrow);
}

#[test]
fn with_ctrl_held_it_is_the_black_arrow_even_over_a_segment() {
    assert_eq!(on_path_one(SQUARE_TOP, Modifiers::COMMAND), Shown::Arrow);
}

#[test]
fn with_alt_held_it_stays_the_pen_over_empty_canvas() {
    assert_eq!(on_path_one(SQUARE_MIDDLE, Modifiers::ALT), Shown::Pen);
}

#[test]
fn the_precise_other_cursors_crosshair_keeps_working() {
    for at in [SQUARE_TOP, SQUARE_MIDDLE] {
        let (mut app, _) = setup(Tool::AddAnchorPoint);
        select_saved_path(&mut app);
        app.run("prefs.set", json!({"path": "cursors.other", "value": "precise"})).unwrap();
        let mut h = harness(app, SQUARE_MIDDLE);
        hover(&mut h, at, SQUARE_MIDDLE, Modifiers::NONE);
        assert_eq!(shown(&h), Shown::Crosshair, "{at:?}");
    }
}

#[test]
fn the_arrow_bitmap_is_not_empty_and_its_hotspot_is_the_tip_at_1_1() {
    let image = rasterize(&Shape::Arrow, 1.0).unwrap();
    assert_eq!(image.hotspot, [1, 1]);
    let alpha = |x: usize, y: usize| image.rgba[(y * usize::from(image.size[0]) + x) * 4 + 3];
    assert!(image.rgba.chunks(4).any(|p| p[3] == 255 && p[0] == 0), "a solid black body");
    assert!(image.rgba.chunks(4).any(|p| p[3] > 0 && p[0] == 255), "a white edge");
    assert!(alpha(1, 1) > 0, "the tip pixel is covered");
    assert_eq!(super::arrow_with(&egui::Context::default(), false), CursorIcon::Default);
}
