use super::*;
use crate::canvas::tool_event;
use photocraft_doc::{Color, ColorMode, Document, SampleType, Size};
use serde_json::json;

const NONE: Modifiers = Modifiers::NONE;
const SHIFT: Modifiers = Modifiers::SHIFT;

fn app(depth: SampleType) -> PhotocraftApp {
    let doc = Document::with_background("crop", Size::new(200, 100), ColorMode::Rgb, depth, Color::WHITE);
    let mut s = photocraft_engine::Session::new();
    s.add_document(doc, None);
    let mut app = PhotocraftApp::new(s, crate::Services::default());
    app.ui.tool = Tool::Crop;
    // These tests pin Classic Mode (the frame moves and turns over the image); the default
    // mode's are in `crop_mode`.
    app.ui.tool_options.crop_shield.classic_mode = true;
    app.ui.extras.snap = false;
    app.ui.view.show.smart_guides = false;
    app
}

fn drag(app: &mut PhotocraftApp, pts: &[[f64; 2]], mods: Modifiers) {
    let (first, last) = (pts[0], pts[pts.len() - 1]);
    tool_event(app, ToolEvent::Down { x: first[0], y: first[1], pressure: 1.0 }, mods);
    for p in &pts[1..] {
        tool_event(app, ToolEvent::Move { x: p[0], y: p[1], pressure: 1.0 }, mods);
    }
    tool_event(app, ToolEvent::Up { x: last[0], y: last[1] }, mods);
}

#[test]
fn fitting_a_ratio_keeps_the_centre_angle_and_area() {
    let mut app = app(SampleType::U8);
    app.ui.crop_rect = Some([20.0, 10.0, 180.0, 90.0]);
    app.ui.crop_angle = 25.0;
    for key in ["1:1", "16:9", "9:16", "", "0:0"] {
        let before = app.ui.crop_rect.unwrap();
        app.ui.tool_options.crop_ratio = key.into();
        crate::crop_size::chosen(&mut app);
        let after = app.ui.crop_rect.unwrap();
        assert_eq!(center(after), center(before));
        assert_eq!(app.ui.crop_angle, 25.0);
        assert!(after[0] >= before[0] && after[1] >= before[1] && after[2] <= before[2] && after[3] <= before[3]);
        if let Some(ratio) = preset_ratio(&app) {
            assert!(((after[2] - after[0]) / (after[3] - after[1]) - ratio).abs() < 1e-10);
        } else {
            assert_eq!(after, before);
        }
    }
    for ratio in [0.0, -1.0, f64::NAN, f64::INFINITY] {
        assert!(fitted_ratio([0.0, 0.0, 200.0, 100.0], ratio).is_none());
    }
    assert!(fitted_ratio([-f64::MAX, 0.0, f64::MAX, 100.0], 1.0).is_none());
}

#[test]
fn choosing_a_ratio_fits_and_commits_without_a_resize() {
    for classic in [false, true] {
        let mut app = app(SampleType::U8);
        app.ui.tool_options.crop_shield.classic_mode = classic;
        ensure_frame(&mut app);
        app.ui.tool_options.crop_ratio = "1:1".into();
        crate::crop_size::chosen(&mut app);
        assert_eq!(app.ui.crop_rect, Some([50.0, 0.0, 150.0, 100.0]));
        crate::canvas::commit_crop(&mut app);
        assert_eq!(app.session.active().unwrap().doc.size, Size::new(100, 100));
    }
}

#[test]
fn entering_crop_with_a_saved_ratio_fits_the_selection_or_canvas() {
    let mut app = app(SampleType::U8);
    app.ui.tool_options.crop_ratio = "1:1".into();
    ensure_frame(&mut app);
    assert_eq!(app.ui.crop_rect, Some([50.0, 0.0, 150.0, 100.0]));
    cancel(&mut app);
    app.run("select.rect", json!({"x": 20, "y": 10, "width": 80, "height": 40})).unwrap();
    ensure_frame(&mut app);
    assert_eq!(app.ui.crop_rect, Some([40.0, 10.0, 80.0, 50.0]));
}

#[test]
fn picking_the_tool_frames_the_canvas_or_the_selection() {
    // #668: the Crop tool started with no frame.
    let mut app = app(SampleType::U8);
    ensure_frame(&mut app);
    assert_eq!(app.ui.crop_rect, Some([0.0, 0.0, 200.0, 100.0]));
    // Committing the untouched frame crops nothing.
    let steps = app.session.active().unwrap().history.past_len();
    crate::canvas::commit_crop(&mut app);
    assert_eq!(app.session.active().unwrap().history.past_len(), steps);
    assert_eq!(app.session.active().unwrap().doc.size, Size::new(200, 100));
    // A drag inside the untouched frame draws a new one instead of moving it.
    ensure_frame(&mut app);
    drag(&mut app, &[[50.0, 20.0], [80.0, 40.0], [120.0, 70.0]], NONE);
    assert_eq!(app.ui.crop_rect, Some([50.0, 20.0, 120.0, 70.0]));
    // Now it's a real frame: a drag inside moves it.
    drag(&mut app, &[[60.0, 30.0], [70.0, 30.0]], NONE);
    assert_eq!(app.ui.crop_rect, Some([60.0, 20.0, 130.0, 70.0]));
    // Cancelling frames the selection's bounds when there is one.
    app.run("select.rect", json!({"x": 10, "y": 10, "width": 30, "height": 20})).unwrap();
    app.ui.crop_rect = None;
    ensure_frame(&mut app);
    assert_eq!(app.ui.crop_rect, Some([10.0, 10.0, 40.0, 30.0]));
    // Leaving the tool drops the untouched frame.
    app.ui.tool = Tool::Brush;
    ensure_frame(&mut app);
    assert_eq!(app.ui.crop_rect, None);
}

#[test]
fn enter_on_the_selection_frame_crops_to_it_at_8_and_16_bit() {
    // #1789: selection, C, ↵ did nothing, as the untouched frame was always treated as a no-op.
    for depth in [SampleType::U8, SampleType::U16] {
        let mut app = app(depth);
        app.run("select.rect", json!({"x": 10, "y": 20, "width": 30, "height": 40})).unwrap();
        ensure_frame(&mut app);
        assert_eq!(app.ui.crop_rect, Some([10.0, 20.0, 40.0, 60.0]));
        crate::canvas::commit_crop(&mut app);
        let doc = &app.session.active().unwrap().doc;
        assert_eq!(doc.size, Size::new(30, 40), "{depth:?}");
        assert_eq!(doc.depth, depth);
        assert!(doc.selection.is_none(), "cropping to the selection deselects, as in Photoshop");
        // The new default frame is the whole (cropped) canvas: ↵ on it does nothing.
        ensure_frame(&mut app);
        assert_eq!(app.ui.crop_rect, Some([0.0, 0.0, 30.0, 40.0]));
        let steps = app.session.active().unwrap().history.past_len();
        crate::canvas::commit_crop(&mut app);
        let st = app.session.active().unwrap();
        assert_eq!((st.doc.size, st.history.past_len()), (Size::new(30, 40), steps), "{depth:?}");
    }
}

#[test]
fn the_frame_is_edited_from_a_press_until_commit_cancel_or_another_tool() {
    let mut app = app(SampleType::U8);
    ensure_frame(&mut app);
    assert!(!shows_beyond_canvas(&app), "a new frame is not being edited");
    assert!(press(&mut app));
    assert!(shows_beyond_canvas(&app));
    assert!(!press(&mut app), "only the first press is news");
    // A click on the frame keeps it edited.
    drag(&mut app, &[[50.0, 50.0]], NONE);
    assert!(shows_beyond_canvas(&app));
    // Esc drops the frame: the new default one isn't edited.
    app.ui.crop_rect = None;
    ensure_frame(&mut app);
    assert!(!shows_beyond_canvas(&app));
    // A drawn frame is edited until it is committed.
    drag(&mut app, &[[10.0, 10.0], [60.0, 40.0]], NONE);
    assert!(shows_beyond_canvas(&app));
    crate::canvas::commit_crop(&mut app);
    ensure_frame(&mut app);
    assert_eq!(app.ui.crop_rect, Some([0.0, 0.0, 50.0, 30.0]));
    assert!(!shows_beyond_canvas(&app));
    // Or until another tool is picked.
    drag(&mut app, &[[5.0, 5.0], [20.0, 20.0]], NONE);
    app.ui.tool = Tool::Brush;
    ensure_frame(&mut app);
    assert!(!app.crop.editing && !shows_beyond_canvas(&app));
    assert!(!press(&mut app), "no frame without the Crop tool");
}

#[test]
fn hit_test_finds_handles_inside_and_outside() {
    let r = [10.0, 10.0, 110.0, 60.0];
    assert_eq!(hit(r, [10.0, 10.0], 4.0), Hit::Handle(-1, -1));
    assert_eq!(hit(r, [112.0, 61.0], 4.0), Hit::Handle(1, 1));
    assert_eq!(hit(r, [60.0, 8.0], 4.0), Hit::Handle(0, -1));
    assert_eq!(hit(r, [108.0, 30.0], 4.0), Hit::Handle(1, 0));
    assert_eq!(hit(r, [60.0, 30.0], 4.0), Hit::Inside);
    assert_eq!(hit(r, [150.0, 30.0], 4.0), Hit::Outside);
    // A tiny frame keeps a movable middle.
    assert_eq!(hit([0.0, 0.0, 4.0, 4.0], [2.0, 2.0], 8.0), Hit::Inside);
}

#[test]
fn dragging_inside_moves_the_frame() {
    let mut app = app(SampleType::U8);
    drag(&mut app, &[[20.0, 20.0], [80.0, 60.0]], NONE);
    assert_eq!(app.ui.crop_rect, Some([20.0, 20.0, 80.0, 60.0]));
    drag(&mut app, &[[50.0, 40.0], [60.0, 45.0], [70.0, 50.0]], NONE);
    assert_eq!(app.ui.crop_rect, Some([40.0, 30.0, 100.0, 70.0]), "moved, not redrawn");
    // A click outside keeps the frame (and leaves it upright).
    drag(&mut app, &[[5.0, 5.0]], NONE);
    assert_eq!(app.ui.crop_rect, Some([40.0, 30.0, 100.0, 70.0]));
    assert_eq!(app.ui.crop_angle, 0.0);
    // Outside, a drag turns the frame rather than drawing a new one (#1792).
    drag(&mut app, &[[150.0, 50.0], [190.0, 90.0]], NONE);
    assert_eq!(app.ui.crop_rect, Some([40.0, 30.0, 100.0, 70.0]));
    assert!(app.ui.crop_angle > 10.0, "{}", app.ui.crop_angle);
}

#[test]
fn dragging_outside_turns_the_frame_about_its_centre() {
    let mut app = app(SampleType::U8);
    app.ui.crop_rect = Some([40.0, 30.0, 100.0, 70.0]);
    // Centre (70, 50): from straight right to straight below is a quarter turn clockwise.
    drag(&mut app, &[[150.0, 50.0], [140.0, 70.0], [70.0, 95.0]], NONE);
    assert!((app.ui.crop_angle - 90.0).abs() < 1e-9, "{}", app.ui.crop_angle);
    assert_eq!(app.ui.crop_rect, Some([40.0, 30.0, 100.0, 70.0]), "turning keeps the frame");
    // Back the other way, past -180 (normalised).
    drag(&mut app, &[[70.0, 120.0], [-10.0, 50.0], [70.0, -20.0]], NONE);
    assert!((app.ui.crop_angle - -90.0).abs() < 1e-9, "{}", app.ui.crop_angle);
    // ⇧ snaps the angle to 15° steps.
    app.ui.crop_angle = 0.0;
    let a = 22.0f64.to_radians();
    drag(&mut app, &[[150.0, 50.0], [70.0 + 80.0 * a.cos(), 50.0 + 80.0 * a.sin()]], SHIFT);
    assert_eq!(app.ui.crop_angle, 15.0);
    assert_eq!(cursor(&app, [150.0, 50.0]), Some(CursorIcon::None), "outside: the drawn turn cursor");
    // Turning the untouched default frame makes it a real frame; a click outside it doesn't.
    let mut app = super::tests::app(SampleType::U8);
    ensure_frame(&mut app);
    drag(&mut app, &[[250.0, 50.0]], NONE);
    assert!(app.crop.default_frame, "a click outside keeps the default frame");
    drag(&mut app, &[[250.0, 50.0], [250.0, 80.0]], NONE);
    assert!(!app.crop.default_frame && app.ui.crop_angle > 0.0);
    // Esc (no frame) starts the next one upright.
    app.ui.crop_rect = None;
    ensure_frame(&mut app);
    assert_eq!((app.ui.crop_rect, app.ui.crop_angle), (Some([0.0, 0.0, 200.0, 100.0]), 0.0));
}

#[test]
fn hit_testing_a_turned_frame_uses_its_own_axes() {
    let r = [40.0, 30.0, 100.0, 70.0];
    // A quarter turn: the frame covers x 50..90, y 20..80 on the page.
    assert_eq!(hit_turned(r, 90.0, [70.0, 75.0], 2.0), Hit::Inside, "inside the turned frame only");
    assert_eq!(hit(r, [70.0, 75.0], 2.0), Hit::Outside);
    assert_eq!(hit_turned(r, 90.0, [45.0, 50.0], 2.0), Hit::Outside, "inside the upright frame only: the rotate zone");
    // The upright frame's top-left corner turns to the top-right of the page.
    assert_eq!(hit_turned(r, 90.0, [90.0, 20.0], 2.0), Hit::Handle(-1, -1));
    // Its right edge's middle is now at the bottom.
    assert_eq!(hit_turned(r, 90.0, [70.0, 80.0], 2.0), Hit::Handle(1, 0));
    let c = corners(r, 90.0);
    for (a, b) in c.iter().zip([[90.0, 20.0], [90.0, 80.0], [50.0, 80.0], [50.0, 20.0]]) {
        assert!((a[0] - b[0]).abs() < 1e-9 && (a[1] - b[1]).abs() < 1e-9, "{c:?}");
    }
    let b = turned_bounds(r, 90.0);
    assert!((b[0] - 50.0).abs() < 1e-9 && (b[3] - 80.0).abs() < 1e-9, "{b:?}");
    // Handle arrows turn with the frame.
    assert_eq!(resize_icon(1, 0, 0.0, false), CursorIcon::ResizeHorizontal);
    assert_eq!(resize_icon(1, 0, 90.0, false), CursorIcon::ResizeVertical);
    assert_eq!(resize_icon(1, 0, 45.0, false), CursorIcon::ResizeNwSe);
    assert_eq!(resize_icon(-1, -1, 90.0, false), CursorIcon::ResizeNeSw);
    assert_eq!(normalized_angle(540.0), 180.0);
    assert_eq!(normalized_angle(-190.0), 170.0);
    assert_eq!(normalized_angle(f64::NAN), 0.0);
}

#[test]
fn resizing_a_turned_frame_moves_along_its_axes_and_keeps_the_opposite_edge() {
    let r = [40.0, 30.0, 100.0, 70.0];
    // At 90°, the right edge (local +x) faces down the page: dragging down 10 grows the width.
    let n = resized_turned(r, 90.0, 1, 0, [0.0, 10.0], None, false);
    assert!(((n[2] - n[0]) - 70.0).abs() < 1e-9 && ((n[3] - n[1]) - 40.0).abs() < 1e-9, "{n:?}");
    // The opposite (left) edge stays put on the page: at y = 20 with x 50..90.
    let (old, new) = (corners(r, 90.0), corners(n, 90.0));
    for (i, (a, b)) in [(0, 0), (3, 3)].map(|(i, j)| (old[i], new[j])).iter().enumerate() {
        assert!((a[0] - b[0]).abs() < 1e-9 && (a[1] - b[1]).abs() < 1e-9, "corner {i}: {a:?} vs {b:?}");
    }
    // A sideways drag does nothing to that handle.
    let same = resized_turned(r, 90.0, 1, 0, [25.0, 0.0], None, false);
    assert!(same.iter().zip(r).all(|(a, b)| (a - b).abs() < 1e-9), "{same:?}");
    // At 0° it is the plain resize.
    assert_eq!(resized_turned(r, 0.0, 1, 1, [5.0, 6.0], None, false), resized(r, 1, 1, [5.0, 6.0], None, false));
    // Through the pointer: grabbing the turned right-edge handle and dragging down 10.
    let mut app = app(SampleType::U8);
    app.ui.crop_rect = Some(r);
    app.ui.crop_angle = 90.0;
    drag(&mut app, &[[70.0, 80.0], [70.0, 90.0]], NONE);
    let got = app.ui.crop_rect.unwrap();
    assert!(got.iter().zip(n).all(|(a, b)| (a - b).abs() < 1e-9), "{got:?} vs {n:?}");
    // Moving a turned frame is a plain translation, angle kept.
    drag(&mut app, &[[70.0, 50.0], [80.0, 55.0]], NONE);
    let moved = app.ui.crop_rect.unwrap();
    assert!(moved.iter().zip([n[0] + 10.0, n[1] + 5.0, n[2] + 10.0, n[3] + 5.0]).all(|(a, b)| (a - b).abs() < 1e-9), "{moved:?}");
    assert_eq!(app.ui.crop_angle, 90.0);
}

/// ↵ on a turned frame crops the turned document (one step), at 8, 16 and 32 bits.
#[test]
fn commit_crops_to_the_turned_frame() {
    for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
        let mut app = app(depth);
        app.run("select.rect", json!({"x": 60, "y": 40, "width": 1, "height": 1})).unwrap();
        app.run("edit.fill", json!({"color": "#ff0000"})).unwrap();
        app.run("select.deselect", json!({})).unwrap();
        let past = app.session.active().unwrap().history.past_len();
        // A 40 × 20 frame centred on (60, 40), turned 90° by a drag outside it.
        app.ui.crop_rect = Some([40.0, 30.0, 80.0, 50.0]);
        app.crop.default_frame = false;
        drag(&mut app, &[[120.0, 40.0], [60.0, 100.0]], NONE);
        assert!((app.ui.crop_angle - 90.0).abs() < 1e-6, "{depth:?}: {}", app.ui.crop_angle);
        crate::canvas::commit_crop(&mut app);
        assert!(app.ui.crop_rect.is_none());
        assert_eq!(app.ui.crop_angle, 0.0, "the next frame starts upright");
        let st = app.session.active().unwrap();
        assert_eq!(st.history.past_len(), past + 1, "{depth:?}: one undo step");
        assert_eq!((st.doc.size.width, st.doc.size.height), (40, 20), "{depth:?}");
        assert_eq!(st.doc.depth, depth);
        // The red pixel was at the frame's centre: it is at the cropped image's centre.
        let surf = st.doc.layers[0].surface().unwrap();
        let red = (19..=21).flat_map(|x| (9..=11).map(move |y| (x, y))).map(|(x, y)| surf.rgba(x, y)).find(|px| px[0] > 0.6 && px[1] < 0.4);
        assert!(red.is_some(), "{depth:?}: no red at the centre");
        app.run("edit.undo", json!({})).unwrap();
        assert_eq!(app.session.active().unwrap().doc.size, Size::new(200, 100), "{depth:?}");
    }
}

/// Fill › Content-Aware Fill (#3823): ↵ on an extending frame fills the added canvas from the
/// image instead of the background colour; Background (Default) keeps the colour.
#[test]
fn commit_passes_the_fill_choice() {
    for content_aware in [false, true] {
        let mut app = app(SampleType::U16);
        app.run("tools.setColors", json!({"foreground": "#808080", "background": "#000000"})).unwrap();
        app.run("edit.fill", json!({"contents": "foreground"})).unwrap();
        app.ui.tool_options.crop_content_aware = content_aware;
        app.ui.crop_rect = Some([-10.0, -10.0, 210.0, 110.0]);
        app.crop.default_frame = false;
        crate::canvas::commit_crop(&mut app);
        let st = app.session.active().unwrap();
        assert_eq!((st.doc.size.width, st.doc.size.height), (220, 120));
        let corner = st.doc.layers[0].surface().unwrap().rgba(0, 0);
        if content_aware {
            assert!((corner[0] - 0.5).abs() < 0.05, "filled from the grey image: {corner:?}");
        } else {
            assert!(corner[0] < 0.05, "the background colour: {corner:?}");
        }
    }
}

#[test]
fn moving_the_frame_snaps_its_edges() {
    let mut app = app(SampleType::U8);
    app.ui.extras.snap = true;
    app.ui.crop_rect = Some([10.0, 10.0, 50.0, 40.0]);
    drag(&mut app, &[[30.0, 25.0], [23.0, 25.0]], NONE);
    assert_eq!(app.ui.crop_rect, Some([0.0, 10.0, 40.0, 40.0]), "left edge snaps to the document edge");
}

/// #3145: Edit › Undo while a crop is pending steps the *box*, not the document (Photoshop keeps
/// Undo live during a crop), so a moved frame comes back.
#[test]
fn edit_undo_steps_the_crop_box_not_the_document() {
    let mut app = app(SampleType::U8);
    app.ui.crop_rect = Some([40.0, 30.0, 100.0, 70.0]);
    app.crop.default_frame = false;
    let before = app.ui.crop_rect.unwrap();
    let doc_steps = app.session.active().unwrap().history.past_len();
    drag(&mut app, &[[70.0, 50.0], [90.0, 50.0]], NONE); // move the frame right 20
    let moved = app.ui.crop_rect.unwrap();
    assert!((moved[0] - 60.0).abs() < 1e-3, "{moved:?}");
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(app.ui.crop_rect, Some(before), "undo restores the box");
    assert_eq!(app.session.active().unwrap().history.past_len(), doc_steps, "the document is untouched");
    app.run("edit.redo", json!({})).unwrap();
    assert!((app.ui.crop_rect.unwrap()[0] - 60.0).abs() < 1e-3, "redo re-applies the move");
}

/// Once the box has no step left in one direction, that command greys out rather than staying
/// live over the document's history and doing nothing (as the Free Transform box does).
#[test]
fn box_undo_and_redo_grey_out_when_their_stack_is_empty() {
    let mut app = app(SampleType::U8);
    app.run("edit.fill", json!({"color": "#ff0000"})).unwrap();
    assert!(app.session.active().unwrap().history.can_undo());
    app.ui.crop_rect = Some([40.0, 30.0, 100.0, 70.0]);
    app.crop.default_frame = false;
    drag(&mut app, &[[70.0, 50.0], [90.0, 50.0]], NONE);
    assert!(crate::menus::is_enabled(&app, "edit.undo"));
    assert!(!crate::menus::is_enabled(&app, "edit.redo"), "no box step to redo");
    app.run("edit.undo", json!({})).unwrap();
    assert!(!crate::menus::is_enabled(&app, "edit.undo"), "no box step left to undo");
    assert!(crate::menus::is_enabled(&app, "edit.redo"));
}

#[test]
fn handles_resize_with_shift_and_alt() {
    let mut app = app(SampleType::U8);
    app.ui.crop_rect = Some([20.0, 20.0, 100.0, 60.0]);
    // Right edge.
    drag(&mut app, &[[100.0, 40.0], [120.0, 50.0]], NONE);
    assert_eq!(app.ui.crop_rect, Some([20.0, 20.0, 120.0, 60.0]));
    // Bottom-right corner, free.
    drag(&mut app, &[[120.0, 60.0], [140.0, 70.0]], NONE);
    assert_eq!(app.ui.crop_rect, Some([20.0, 20.0, 140.0, 70.0]));
    // ⇧ keeps 120:50 from the top-left corner: the larger relative extent wins.
    app.ui.crop_rect = Some([20.0, 20.0, 140.0, 70.0]);
    drag(&mut app, &[[20.0, 20.0], [10.0, 0.0]], SHIFT);
    let r = app.ui.crop_rect.unwrap();
    assert_eq!((r[2], r[3]), (140.0, 70.0), "opposite corner fixed");
    assert!(((r[2] - r[0]) / (r[3] - r[1]) - 120.0 / 50.0).abs() < 1e-9, "{r:?}");
    assert_eq!(r[3] - r[1], 70.0);
    // ⌥ resizes about the centre.
    app.ui.crop_rect = Some([40.0, 20.0, 80.0, 60.0]);
    drag(&mut app, &[[80.0, 40.0], [90.0, 40.0]], Modifiers::ALT);
    assert_eq!(app.ui.crop_rect, Some([30.0, 20.0, 90.0, 60.0]));
    // An options-bar ratio holds on an edge drag, centred on the other axis.
    app.ui.tool_options.crop_ratio = "1:1".into();
    app.ui.crop_rect = Some([40.0, 20.0, 80.0, 60.0]);
    drag(&mut app, &[[80.0, 40.0], [100.0, 40.0]], NONE);
    assert_eq!(app.ui.crop_rect, Some([40.0, 10.0, 100.0, 70.0]));
}

#[test]
fn dragging_a_handle_past_the_other_side_flips_and_collapse_reverts() {
    let mut app = app(SampleType::U8);
    app.ui.crop_rect = Some([20.0, 20.0, 60.0, 60.0]);
    drag(&mut app, &[[60.0, 40.0], [10.0, 40.0]], NONE);
    assert_eq!(app.ui.crop_rect, Some([10.0, 20.0, 20.0, 60.0]));
    // Collapsing to nothing restores the frame the gesture started from.
    drag(&mut app, &[[20.0, 40.0], [10.0, 40.0]], NONE);
    assert_eq!(app.ui.crop_rect, Some([10.0, 20.0, 20.0, 60.0]));
}

#[test]
fn space_repositions_the_frame_being_drawn() {
    let mut app = app(SampleType::U8);
    tool_event(&mut app, ToolEvent::Down { x: 10.0, y: 10.0, pressure: 1.0 }, NONE);
    tool_event(&mut app, ToolEvent::Move { x: 50.0, y: 40.0, pressure: 1.0 }, NONE);
    assert!(active(&app));
    set_space(&mut app, true);
    tool_event(&mut app, ToolEvent::Move { x: 70.0, y: 50.0, pressure: 1.0 }, NONE);
    assert_eq!(app.ui.crop_rect, Some([30.0, 20.0, 70.0, 50.0]), "same size, moved");
    set_space(&mut app, false);
    tool_event(&mut app, ToolEvent::Move { x: 90.0, y: 60.0, pressure: 1.0 }, NONE);
    tool_event(&mut app, ToolEvent::Up { x: 90.0, y: 60.0 }, NONE);
    assert_eq!(app.ui.crop_rect, Some([30.0, 20.0, 90.0, 60.0]), "sizing resumes from the moved anchor");
    assert!(!active(&app));
}

/// Real egui input on the canvas: holding Space mid-draw moves the frame instead of panning.
#[test]
fn space_on_the_canvas_moves_the_frame_not_the_view() {
    use egui::{Event, Key, PointerButton, pos2};
    let mut a = app(SampleType::U8);
    let view = crate::state::View {
        zoom: 2.0,
        center: [100.0, 50.0],
        fit_pending: false,
        fill_pending: false,
        doc_size: [200, 100],
        rotation: 0.0,
        zoom_animating: false,
        zoom_anchor: None,
        display_camera: None,
        zoom_transition: None,
    };
    a.ui.views = vec![view.clone()];
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(600.0, 400.0)).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            let v = app.ui.views[0].clone();
            crate::canvas::canvas_view(app, ui, 0, ui.max_rect(), v, true);
        },
        a,
    );
    h.run_steps(2);
    let button = |pos, pressed| Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
    let space = |pressed| Event::Key { key: Key::Space, physical_key: None, pressed, repeat: false, modifiers: Modifiers::NONE };
    let p0 = pos2(200.0, 150.0);
    h.event(Event::PointerMoved(p0));
    h.event(button(p0, true));
    h.step();
    for p in [pos2(220.0, 170.0), pos2(260.0, 190.0)] {
        h.event(Event::PointerMoved(p));
        h.step();
    }
    let drawn = h.state().ui.crop_rect.unwrap();
    // The frame starts where the button went down (200, 150), not at the first move past
    // egui's drag threshold (#123).
    assert_eq!((drawn[2] - drawn[0], drawn[3] - drawn[1]), (30.0, 20.0), "60x40 screen px at 200%");
    h.event(space(true));
    h.step();
    for p in [pos2(280.0, 200.0), pos2(300.0, 210.0)] {
        h.event(Event::PointerMoved(p));
        h.step();
    }
    let moved = h.state().ui.crop_rect.unwrap();
    assert_eq!(moved, [drawn[0] + 20.0, drawn[1] + 10.0, drawn[2] + 20.0, drawn[3] + 10.0], "moved by 40x20 screen px");
    h.event(space(false));
    h.event(button(pos2(300.0, 210.0), false));
    h.step();
    let st = h.state();
    assert_eq!(st.ui.crop_rect, Some(moved));
    assert_eq!(st.ui.views[0].center, view.center, "the view did not pan");
    assert!(st.crop.drag.is_none());
}

/// #1918: a pending crop belongs to the document it was drawn on. File › New, opening a file
/// or switching tabs cancels it; it never shows on, or crops, another document.
/// A turned pending frame is dropped with its angle when another document becomes active, and
/// Esc drops both too, so the next frame starts upright.
#[test]
fn cancelling_a_turned_crop_resets_its_angle() {
    let mut app = app(SampleType::U8);
    drag(&mut app, &[[10.0, 10.0], [50.0, 40.0]], NONE);
    app.ui.crop_angle = 30.0;
    app.run("file.new", json!({"width": 300, "height": 150})).unwrap();
    app.sync_views();
    assert_eq!(app.ui.crop_angle, 0.0);
    drag(&mut app, &[[10.0, 10.0], [50.0, 40.0]], NONE);
    app.ui.crop_angle = -12.5;
    cancel(&mut app);
    assert_eq!((app.ui.crop_rect, app.ui.crop_angle, app.crop.drag), (None, 0.0, None));
}

#[test]
fn a_pending_crop_stays_with_its_document() {
    let size = |app: &PhotocraftApp| app.session.active().map(|st| st.doc.size);
    // File › New.
    let mut app = app(SampleType::U8);
    drag(&mut app, &[[10.0, 10.0], [50.0, 40.0]], NONE);
    assert_eq!(app.ui.crop_rect, Some([10.0, 10.0, 50.0, 40.0]));
    app.run("file.new", json!({"width": 300, "height": 150})).unwrap();
    assert_eq!(app.ui.crop_rect, None, "the first document's frame is gone");
    assert!(app.crop.drag.is_none() && !app.crop.editing && !app.crop.default_frame);
    crate::canvas::commit_crop(&mut app);
    assert_eq!(size(&app), Some(Size::new(300, 150)), "↵ crops nothing");
    // The new document gets its own default frame.
    ensure_frame(&mut app);
    assert_eq!(app.ui.crop_rect, Some([0.0, 0.0, 300.0, 150.0]));
    assert!(app.crop.default_frame);
    // Switching tabs (outside a command), even before the next frame's sync: ↵ never crops
    // the other document.
    drag(&mut app, &[[20.0, 20.0], [60.0, 60.0]], NONE);
    assert!(app.session.set_active(0));
    crate::canvas::commit_crop(&mut app);
    assert_eq!(size(&app), Some(Size::new(200, 100)));
    assert_eq!(app.session.documents()[1].doc.size, Size::new(300, 150));
    assert_eq!(app.ui.crop_rect, None);
    // The next frame's sync also drops a frame left on a tab switched away from.
    drag(&mut app, &[[20.0, 20.0], [60.0, 60.0]], NONE);
    assert!(app.session.set_active(1));
    app.sync_views();
    assert_eq!(app.ui.crop_rect, None);
    // Closing another document keeps the pending crop; closing its own cancels it.
    drag(&mut app, &[[20.0, 20.0], [60.0, 60.0]], NONE);
    app.session.close(0);
    app.sync_views();
    assert_eq!(app.ui.crop_rect, Some([20.0, 20.0, 60.0, 60.0]), "still its document");
    crate::canvas::commit_crop(&mut app);
    assert_eq!(size(&app), Some(Size::new(40, 40)));
    drag(&mut app, &[[5.0, 5.0], [25.0, 25.0]], NONE);
    app.session.close(0);
    app.sync_views();
    assert!(app.session.active().is_none());
    assert_eq!(app.ui.crop_rect, None);
    assert!(app.crop.drag.is_none());
    crate::canvas::commit_crop(&mut app);
    ensure_frame(&mut app);
    assert_eq!(app.ui.crop_rect, None, "no document, no frame");
}

/// A drag in progress when its document goes away is dropped too, not finished elsewhere.
#[test]
fn a_crop_drag_ends_with_its_document() {
    let mut app = app(SampleType::U8);
    tool_event(&mut app, ToolEvent::Down { x: 10.0, y: 10.0, pressure: 1.0 }, NONE);
    tool_event(&mut app, ToolEvent::Move { x: 50.0, y: 40.0, pressure: 1.0 }, NONE);
    app.run("file.new", json!({"width": 300, "height": 150})).unwrap();
    tool_event(&mut app, ToolEvent::Up { x: 60.0, y: 50.0 }, NONE);
    assert_eq!(app.ui.crop_rect, None);
    crate::canvas::commit_crop(&mut app);
    assert_eq!(app.session.active().map(|st| st.doc.size), Some(Size::new(300, 150)));
}

/// #1918: as in Photoshop, a pending crop (not the untouched default frame) greys almost every
/// menu command; the ones Photoshop keeps stay available, and commit or cancel frees the rest.
/// Walks the whole menu catalog.
#[test]
fn a_pending_crop_greys_the_menus_photoshop_greys() {
    let mut app = app(SampleType::U8);
    ensure_frame(&mut app);
    assert!(app.crop.default_frame);
    let ids: Vec<&str> = crate::menu_catalog::CATALOG.iter().map(|e| e.3).filter(|id| *id != "---").collect();
    for id in &ids {
        assert!(!blocks(&app, id), "default frame: {id} stays as it was");
    }
    drag(&mut app, &[[10.0, 10.0], [50.0, 40.0]], NONE);
    assert!(pending(&app));
    let kept = [
        "file.close",
        "file.save",
        "file.saveAs",
        "file.saveACopy",
        "edit.undo",
        "edit.redo",
        "edit.toggleLastState",
        "edit.search",
        "image.crop",
        "view.proofColors",
        "view.gamutWarning",
        "view.zoomIn",
        "view.zoomOut",
        "view.fitOnScreen",
        "view.actualPixels",
        "view.twoHundredPercent",
        "view.printSize",
        "view.extras",
        "view.show.grid",
        "view.rulers",
        "view.snap",
        "view.snapTo.guides",
        "view.lockSlices",
        "view.newGuide",
        "window.arrange.tile",
        "window.panel.layers",
        "help.about",
    ];
    let greyed = [
        "file.new",
        "file.open",
        "file.openAs",
        "file.openExrParts",
        "file.closeAll",
        "file.revert",
        "file.export.exportAs",
        "file.placeEmbedded",
        "file.automate.batch",
        "file.scripts.browse",
        "file.import.notes",
        "file.fileInfo",
        "file.print",
        "edit.cut",
        "edit.fill",
        "edit.freeTransform",
        "edit.colorSettings",
        "edit.preferences.general",
        "image.imageSize",
        "image.mode.grayscale",
        "layer.new.layer",
        "select.all",
        "filter.blur.gaussianBlur",
        "view.proofSetup.workingCmyk",
        "view.pixelAspectRatio.square",
        "view.pixelAspectRatioCorrection",
        "view.thirtyTwoBitPreviewOptions",
        "view.flipHorizontal",
        "view.screenMode.fullScreen",
        "window.workspace.essentials",
        "window.panel.adjustments",
        "help.systemInfo",
    ];
    for id in kept {
        assert!(!blocks(&app, id), "{id} stays available");
    }
    for id in greyed {
        assert!(blocks(&app, id), "{id} is greyed");
        assert!(!crate::menus::is_enabled(&app, id), "{id} greyed in the menus");
        assert!(!crate::menus::modal_allows(&app, id), "{id} refused over the menu gate");
    }
    assert!(
        ids.iter()
            .filter(|id| id.starts_with("layer.") || id.starts_with("type.") || id.starts_with("select.") || id.starts_with("filter."))
            .all(|id| blocks(&app, id))
    );
    // Committing still runs `image.crop` itself, and frees the menus again.
    crate::canvas::commit_crop(&mut app);
    assert_eq!(app.session.active().unwrap().doc.size, Size::new(40, 30));
    assert!(!pending(&app) && crate::menus::is_enabled(&app, "file.new"));
    // Cancelling does too.
    drag(&mut app, &[[5.0, 5.0], [20.0, 20.0]], NONE);
    assert!(blocks(&app, "file.new"));
    cancel(&mut app);
    assert!(!blocks(&app, "file.new"));
    // Another tool: nothing is pending.
    drag(&mut app, &[[5.0, 5.0], [20.0, 20.0]], NONE);
    app.ui.tool = Tool::Brush;
    assert!(!blocks(&app, "file.new"));
}

/// #3368: as in Photoshop, picking another tool (toolbar or its key) with a drawn frame
/// commits the crop silently, and the new tool is current.
#[test]
fn another_tool_commits_a_drawn_frame_at_8_and_16_bit() {
    for depth in [SampleType::U8, SampleType::U16] {
        let mut app = app(depth);
        drag(&mut app, &[[10.0, 10.0], [50.0, 40.0]], NONE);
        assert!(pending(&app));
        app.ui.tool = Tool::Move;
        ensure_frame(&mut app);
        let st = app.session.active().unwrap();
        assert_eq!((st.doc.size, st.doc.depth), (Size::new(40, 30), depth));
        assert_eq!(st.history.undo_label(), Some("Crop"));
        assert_eq!(app.ui.tool, Tool::Move);
        assert_eq!((app.ui.crop_rect, app.ui.crop_angle), (None, 0.0));
        // Nothing is left to commit: back on the Crop tool, the new canvas gets a default frame.
        ensure_frame(&mut app);
        assert_eq!(app.session.active().unwrap().doc.size, Size::new(40, 30));
        app.ui.tool = Tool::Crop;
        ensure_frame(&mut app);
        assert!(app.crop.default_frame && app.ui.crop_rect == Some([0.0, 0.0, 40.0, 30.0]));
    }
}

/// #3368, default mode: a turned frame crops turned, and the view it turned is left upright.
#[test]
fn another_tool_commits_a_turned_frame_and_leaves_the_view_upright() {
    let mut app = app(SampleType::U8);
    app.sync_views();
    app.ui.tool_options.crop_shield.classic_mode = false;
    app.ui.crop_rect = Some([40.0, 30.0, 100.0, 70.0]);
    app.ui.crop_angle = 10.0;
    ensure_frame(&mut app);
    assert!(pending(&app) && crate::crop_mode::view_rotation(&app) != 0.0);
    app.ui.tool = Tool::Brush;
    ensure_frame(&mut app);
    assert_eq!(app.session.active().unwrap().doc.size, Size::new(60, 40));
    assert_eq!(crate::crop_mode::view_rotation(&app), 0.0);
    assert_eq!((app.ui.tool, app.ui.crop_rect), (Tool::Brush, None));
}

/// #3368: the untouched default frame (the canvas, or the selection's bounds) is just dropped
/// on leaving the tool: nothing is cropped.
#[test]
fn another_tool_drops_an_untouched_default_frame() {
    for select in [false, true] {
        let mut app = app(SampleType::U8);
        if select {
            app.run("select.rect", json!({"x": 10, "y": 20, "width": 30, "height": 40})).unwrap();
        }
        ensure_frame(&mut app);
        assert!(app.crop.default_frame && app.ui.crop_rect.is_some());
        let steps = app.session.active().unwrap().history.past_len();
        app.ui.tool = Tool::Move;
        ensure_frame(&mut app);
        let st = app.session.active().unwrap();
        assert_eq!((st.doc.size, st.history.past_len()), (Size::new(200, 100), steps), "selection: {select}");
        assert_eq!(app.ui.crop_rect, None);
    }
}

/// #6750: a ratio preset that shrinks the untouched default frame does not make it a drawn
/// one: leaving the tool drops the frame and crops nothing, as with the plain default frame.
/// Picking the ratio before entering the tool (the issue's repro) and while it is picked both
/// stay untouched; editing the fitted frame makes it a drawn one that leaving commits (#3368).
#[test]
fn another_tool_drops_an_untouched_ratio_fitted_frame() {
    // Entering the Crop tool with a saved ratio: the frame is fitted but untouched.
    let mut app = app(SampleType::U8);
    app.ui.tool_options.crop_ratio = "1:1".into();
    ensure_frame(&mut app);
    assert_eq!(app.ui.crop_rect, Some([50.0, 0.0, 150.0, 100.0]), "the default frame is fitted to the ratio");
    assert!(app.crop.default_frame, "fitting it is not an edit");
    let steps = app.session.active().unwrap().history.past_len();
    app.ui.tool = Tool::Move;
    ensure_frame(&mut app);
    let st = app.session.active().unwrap();
    assert_eq!((st.doc.size, st.history.past_len()), (Size::new(200, 100), steps), "switching tools must not crop");
    assert_eq!(app.ui.crop_rect, None);
    // Choosing the ratio with the Crop tool already picked: the same untouched fit.
    let mut app2 = self::app(SampleType::U8);
    ensure_frame(&mut app2);
    app2.ui.tool_options.crop_ratio = "1:1".into();
    crate::crop_size::chosen(&mut app2);
    assert_eq!(app2.ui.crop_rect, Some([50.0, 0.0, 150.0, 100.0]));
    assert!(app2.crop.default_frame);
    let steps = app2.session.active().unwrap().history.past_len();
    app2.ui.tool = Tool::Move;
    ensure_frame(&mut app2);
    let st = app2.session.active().unwrap();
    assert_eq!((st.doc.size, st.history.past_len()), (Size::new(200, 100), steps));
    // Resizing a handle of the fitted frame is an edit: leaving commits it (#3368).
    let mut app3 = self::app(SampleType::U8);
    app3.ui.tool_options.crop_ratio = "1:1".into();
    ensure_frame(&mut app3);
    drag(&mut app3, &[[50.0, 0.0], [70.0, 20.0]], NONE);
    assert!(!app3.crop.default_frame, "the resize made it a drawn frame");
    app3.ui.tool = Tool::Move;
    ensure_frame(&mut app3);
    assert_eq!(app3.session.active().unwrap().doc.size, Size::new(80, 80), "the edited frame crops");
    // Default (non-classic) mode, the issue's exact scenario: the fitted preview stays a
    // preview; pressing V neither crops nor applies anything.
    let mut app4 = self::app(SampleType::U8);
    app4.ui.tool_options.crop_shield.classic_mode = false;
    app4.sync_views();
    app4.ui.tool_options.crop_ratio = "1:1".into();
    ensure_frame(&mut app4);
    assert!(app4.crop.default_frame, "the fitted preview is still the untouched frame");
    let steps = app4.session.active().unwrap().history.past_len();
    app4.ui.tool = Tool::Move;
    ensure_frame(&mut app4);
    let st = app4.session.active().unwrap();
    assert_eq!((st.doc.size, st.history.past_len()), (Size::new(200, 100), steps));
}

/// The pen's eraser end stands in for the Crop tool only until the tip comes back (`stylus`),
/// and a gesture still in progress isn't cut short: neither commits the frame.
#[test]
fn the_stylus_eraser_and_a_gesture_in_progress_keep_the_frame_pending() {
    let mut app = app(SampleType::U8);
    drag(&mut app, &[[10.0, 10.0], [50.0, 40.0]], NONE);
    app.stylus.tool_before_eraser = Some(Tool::Crop);
    app.ui.tool = Tool::Eraser;
    ensure_frame(&mut app);
    assert_eq!(app.session.active().unwrap().doc.size, Size::new(200, 100));
    app.ui.tool = app.stylus.tool_before_eraser.take().unwrap();
    ensure_frame(&mut app);
    assert!(pending(&app) && app.ui.crop_rect == Some([10.0, 10.0, 50.0, 40.0]));

    tool_event(&mut app, ToolEvent::Down { x: 20.0, y: 20.0, pressure: 1.0 }, NONE);
    assert!(app.crop.drag.is_some());
    app.ui.tool = Tool::Move;
    ensure_frame(&mut app);
    assert_eq!(app.session.active().unwrap().doc.size, Size::new(200, 100));
}

/// Image › Crop with a pending crop commits the frame (the one Image item Photoshop keeps).
#[test]
fn image_crop_menu_commits_a_pending_crop() {
    let mut app = app(SampleType::U8);
    drag(&mut app, &[[10.0, 10.0], [50.0, 40.0]], NONE);
    let ctx = egui::Context::default();
    crate::menus::invoke(&mut app, &ctx, "image.crop", json!({})).unwrap();
    assert_eq!(app.session.active().unwrap().doc.size, Size::new(40, 30));
    assert!(app.ui.crop_rect.is_none() || app.crop.default_frame);
}

#[test]
fn commit_crops_to_the_moved_frame_at_8_and_16_bit() {
    for depth in [SampleType::U8, SampleType::U16] {
        let mut app = app(depth);
        app.run("select.rect", json!({"x": 60, "y": 40, "width": 1, "height": 1})).unwrap();
        app.run("edit.fill", json!({"color": "#ff0000"})).unwrap();
        app.run("select.deselect", json!({})).unwrap();
        drag(&mut app, &[[10.0, 10.0], [50.0, 40.0]], NONE);
        drag(&mut app, &[[30.0, 25.0], [70.0, 45.0]], NONE);
        assert_eq!(app.ui.crop_rect, Some([50.0, 30.0, 90.0, 60.0]));
        crate::canvas::commit_crop(&mut app);
        assert!(app.ui.crop_rect.is_none());
        let doc = &app.session.active().unwrap().doc;
        assert_eq!((doc.size.width, doc.size.height), (40, 30), "{depth:?}");
        assert_eq!(doc.depth, depth);
        let px = doc.layers[0].surface().unwrap().rgba(10, 10);
        assert!(px[0] > 0.99 && px[1] < 0.01, "{depth:?} {px:?}");
    }
}

#[test]
fn enter_during_a_drag_ends_the_drag() {
    // ↵ while the frame is still being dragged commits it; the release after must not bring
    // back a frame in the old document's coordinates as a pending crop.
    let mut app = app(SampleType::U8);
    app.ui.crop_rect = Some([20.0, 20.0, 120.0, 70.0]);
    tool_event(&mut app, ToolEvent::Down { x: 50.0, y: 40.0, pressure: 1.0 }, NONE);
    tool_event(&mut app, ToolEvent::Move { x: 60.0, y: 40.0, pressure: 1.0 }, NONE);
    crate::canvas::commit_crop(&mut app);
    tool_event(&mut app, ToolEvent::Up { x: 60.0, y: 40.0 }, NONE);
    ensure_frame(&mut app);
    assert_eq!(app.session.active().unwrap().doc.size, Size::new(100, 50));
    assert_eq!(app.ui.crop_rect, Some([0.0, 0.0, 100.0, 50.0]));
    assert!(!pending(&app));
}

#[test]
fn degenerate_and_bad_input_never_panics() {
    let mut app = app(SampleType::U8);
    for r in [[0.0; 4], [5.0, 5.0, 5.0, 5.0], [10.0, 10.0, 0.0, 0.0], [f64::NAN, 0.0, 10.0, 10.0], [f64::INFINITY, 0.0, f64::MAX, 1e300]] {
        app.ui.crop_rect = Some(r);
        for m in [NONE, SHIFT, Modifiers::ALT, SHIFT | Modifiers::ALT] {
            drag(&mut app, &[[5.0, 5.0], [f64::NAN, 3.0], [1e300, -1e300], [6.0, 7.0]], m);
            let _ = cursor(&app, [5.0, 5.0]);
            app.ui.crop_rect = Some(r);
        }
    }
    // Odd angles (the control channel can set any) turn, resize and commit without panicking.
    for a in [f64::NAN, f64::INFINITY, -1e300, 1e-300, 720.0] {
        app.ui.crop_rect = Some([10.0, 10.0, 50.0, 40.0]);
        app.ui.crop_angle = a;
        for pts in [[[30.0, 25.0], [f64::NAN, 1.0]], [[30.0, 25.0], [1e300, 2.0]], [[50.0, 40.0], [60.0, 45.0]], [[90.0, 90.0], [-1e300, 1e300]]] {
            drag(&mut app, &pts, SHIFT);
            let _ = cursor(&app, [5.0, 5.0]);
        }
        assert!(app.ui.crop_angle.is_finite() || !a.is_finite());
        crate::canvas::commit_crop(&mut app);
        app.run("edit.undo", json!({})).ok();
    }
    app.ui.tool_options.crop_ratio = "0:0".into();
    app.ui.crop_rect = Some([0.0, 0.0, 10.0, 10.0]);
    drag(&mut app, &[[10.0, 10.0], [30.0, 30.0]], SHIFT);
    crate::canvas::commit_crop(&mut app);
    // No document: events are swallowed quietly.
    let mut empty = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    empty.ui.tool = Tool::Crop;
    drag(&mut empty, &[[1.0, 1.0], [9.0, 9.0]], NONE);
    assert!(empty.ui.crop_rect.is_none());
}

/// #1792: outside the frame the turn cursor bends towards the corner of the pointer's quadrant
/// (four variants, as in Photoshop) and turns with the frame; no turn cursor inside or on a handle.
#[test]
fn turn_cursor_points_at_the_quadrant_corner() {
    let mut app = app(SampleType::U8);
    app.ui.crop_rect = Some([20.0, 20.0, 100.0, 60.0]);
    let h = std::f64::consts::FRAC_1_SQRT_2;
    let near = |d: Option<[f64; 2]>, want: [f64; 2]| d.is_some_and(|d| (d[0] - want[0]).abs() < 1e-9 && (d[1] - want[1]).abs() < 1e-9);
    assert!(near(turn_cursor_dir(&app, [0.0, 0.0]), [h, h]), "top-left: towards the top-left corner");
    assert!(near(turn_cursor_dir(&app, [130.0, 5.0]), [-h, h]), "top-right");
    assert!(near(turn_cursor_dir(&app, [130.0, 90.0]), [-h, -h]), "bottom-right");
    assert!(near(turn_cursor_dir(&app, [5.0, 90.0]), [h, -h]), "bottom-left");
    assert!(near(turn_cursor_dir(&app, [150.0, 30.0]), [-h, h]), "beside the right edge, upper half");
    assert_eq!(turn_cursor_dir(&app, [60.0, 40.0]), None, "inside");
    assert_eq!(turn_cursor_dir(&app, [20.0, 20.0]), None, "on a handle");
    // A frame turned 45° turns the cursor with it: the diagonal becomes horizontal.
    app.ui.crop_angle = 45.0;
    assert!(near(turn_cursor_dir(&app, [0.0, 0.0]), [1.0, 0.0]), "{:?}", turn_cursor_dir(&app, [0.0, 0.0]));
    app.ui.crop_angle = f64::NAN;
    assert!(turn_cursor_dir(&app, [f64::NAN, 0.0]).is_none());
    app.ui.tool = Tool::Brush;
    assert_eq!(turn_cursor_dir(&app, [0.0, 0.0]), None);
}

/// A double-click inside the frame commits it, as ↵ does; on a handle or outside it doesn't.
#[test]
fn double_click_inside_the_frame_commits() {
    let mut app = app(SampleType::U8);
    app.ui.crop_rect = Some([20.0, 20.0, 100.0, 60.0]);
    assert!(commits_at(&app, [60.0, 40.0]));
    assert!(!commits_at(&app, [20.0, 20.0]), "on a handle");
    assert!(!commits_at(&app, [150.0, 40.0]), "outside: that turns the frame");
    app.ui.crop_angle = 30.0;
    assert!(commits_at(&app, [60.0, 40.0]), "inside a turned frame");
    assert!(!commits_at(&app, [25.0, 22.0]), "inside the upright frame but outside the turned one");
    app.ui.crop_angle = 0.0;
    app.ui.crop_rect = None;
    assert!(!commits_at(&app, [60.0, 40.0]), "no frame");
    app.ui.crop_rect = Some([f64::NAN, 0.0, 1.0, 1.0]);
    assert!(!commits_at(&app, [0.5, 0.5]));
    app.ui.crop_rect = Some([20.0, 20.0, 100.0, 60.0]);
    app.ui.tool = Tool::Brush;
    assert!(!commits_at(&app, [60.0, 40.0]));
}

#[test]
fn cursor_follows_the_frame() {
    let mut app = app(SampleType::U8);
    assert_eq!(cursor(&app, [5.0, 5.0]), None, "no frame yet: the default crosshair");
    app.ui.crop_rect = Some([20.0, 20.0, 100.0, 60.0]);
    assert_eq!(cursor(&app, [20.0, 20.0]), Some(CursorIcon::ResizeNwSe));
    assert_eq!(cursor(&app, [100.0, 20.0]), Some(CursorIcon::ResizeNeSw));
    assert_eq!(cursor(&app, [60.0, 60.0]), Some(CursorIcon::ResizeVertical));
    assert_eq!(cursor(&app, [60.0, 40.0]), Some(CursorIcon::Move));
    assert_eq!(cursor(&app, [150.0, 40.0]), Some(CursorIcon::None), "outside: the drawn turn cursor");
    // View › Flip Horizontal shows the top-left corner at the top right: its arrow mirrors too.
    app.ui.view.flip_horizontal = true;
    assert_eq!(cursor(&app, [20.0, 20.0]), Some(CursorIcon::ResizeNeSw), "flipped view");
    assert_eq!(cursor(&app, [100.0, 20.0]), Some(CursorIcon::ResizeNwSe), "flipped view");
    assert_eq!(cursor(&app, [60.0, 60.0]), Some(CursorIcon::ResizeVertical), "flipped view");
    app.ui.view.flip_horizontal = false;
    app.ui.tool = Tool::Brush;
    assert_eq!(cursor(&app, [60.0, 40.0]), None);
}

/// #4461: inside the untouched default frame a drag draws a new frame, so the pointer is the
/// drawing crosshair there, not the move cursor; once the frame is changed a drag moves it.
#[test]
fn cursor_inside_the_default_frame_draws() {
    let mut app = app(SampleType::U8);
    ensure_frame(&mut app);
    assert!(app.crop.default_frame && app.ui.crop_rect == Some([0.0, 0.0, 200.0, 100.0]));
    assert_eq!(cursor(&app, [100.0, 50.0]), Some(CursorIcon::Crosshair));
    assert_eq!(cursor(&app, [200.0, 50.0]), Some(CursorIcon::ResizeHorizontal), "its handles still resize");
    drag(&mut app, &[[200.0, 50.0], [150.0, 50.0]], NONE);
    assert!(!app.crop.default_frame && app.ui.crop_rect == Some([0.0, 0.0, 150.0, 100.0]));
    assert_eq!(cursor(&app, [100.0, 50.0]), Some(CursorIcon::Move));
}

/// One key press through the app's keyboard handling (`shortcuts::handle`).
fn key(app: &mut PhotocraftApp, key: egui::Key, modifiers: Modifiers) {
    let ctx = egui::Context::default();
    let raw = egui::RawInput {
        events: vec![egui::Event::ModifiersChanged(modifiers), egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers }],
        ..Default::default()
    };
    let mut out = ctx.run_ui(raw, |ui| crate::shortcuts::handle(app, ui.ctx()));
    out.textures_delta.clear();
}

fn layer_offset(app: &PhotocraftApp) -> (i32, i32) {
    let st = app.session.active().unwrap();
    let l = st.doc.layer(st.active_layer.unwrap()).unwrap();
    l.surface().map_or((0, 0), |s| (s.content_bounds().x0, s.content_bounds().y0))
}

/// #1919: with a pending frame the arrow keys nudge it 1 px (⇧ 10 px), turned or not, and
/// never the selection or layer; without one they keep their usual job.
#[test]
fn arrow_keys_nudge_the_pending_frame() {
    use egui::Key;
    let mut app = app(SampleType::U8);
    app.run("select.rect", json!({"x": 10, "y": 10, "width": 30, "height": 20})).unwrap();
    app.ui.crop_rect = Some([20.0, 20.0, 100.0, 60.0]);
    let steps = app.session.active().unwrap().history.past_len();
    key(&mut app, Key::ArrowRight, NONE);
    assert_eq!(app.ui.crop_rect, Some([21.0, 20.0, 101.0, 60.0]));
    key(&mut app, Key::ArrowDown, SHIFT);
    assert_eq!(app.ui.crop_rect, Some([21.0, 30.0, 101.0, 70.0]));
    key(&mut app, Key::ArrowLeft, SHIFT);
    key(&mut app, Key::ArrowUp, NONE);
    assert_eq!(app.ui.crop_rect, Some([11.0, 29.0, 91.0, 69.0]));
    assert!(app.crop.editing && !app.crop.default_frame);
    assert_eq!(app.session.active().unwrap().history.past_len(), steps, "nothing in the document moved");
    assert_eq!(app_selection_bounds(&app), Some((10, 10, 40, 30)));
    assert_eq!(layer_offset(&app), (0, 0));
    // A turned frame moves on the page and keeps its angle and shape.
    app.ui.crop_angle = 30.0;
    let before = corners(app.ui.crop_rect.unwrap(), 30.0);
    key(&mut app, Key::ArrowRight, SHIFT);
    let after = corners(app.ui.crop_rect.unwrap(), angle(&app));
    for (a, b) in before.iter().zip(after) {
        assert!((b[0] - a[0] - 10.0).abs() < 1e-9 && (b[1] - a[1]).abs() < 1e-9);
    }
    assert_eq!(app.ui.crop_angle, 30.0);
    // The untouched default frame becomes a real one.
    app.ui.crop_rect = None;
    app.ui.crop_angle = 0.0;
    app.run("select.deselect", json!({})).unwrap();
    ensure_frame(&mut app);
    assert!(app.crop.default_frame);
    assert!(nudge(&mut app, 1.0, 0.0));
    assert_eq!(app.ui.crop_rect, Some([1.0, 0.0, 201.0, 100.0]));
    assert!(!app.crop.default_frame);
    // No frame: not the Crop tool's key. With a selection tool the arrows nudge the selection.
    app.ui.crop_rect = None;
    assert!(!nudge(&mut app, 1.0, 0.0));
    app.ui.tool = Tool::RectMarquee;
    app.run("select.rect", json!({"x": 10, "y": 10, "width": 30, "height": 20})).unwrap();
    key(&mut app, Key::ArrowRight, NONE);
    assert_eq!(app_selection_bounds(&app), Some((11, 10, 41, 30)));
    // Not mid-gesture, nor with another tool, nor for a non-finite frame or step.
    app.ui.tool = Tool::Crop;
    app.ui.crop_rect = Some([20.0, 20.0, 100.0, 60.0]);
    app.crop.drag = Some(CropDrag::Move { start: [0.0, 0.0], rect: [20.0, 20.0, 100.0, 60.0] });
    assert!(!nudge(&mut app, 1.0, 0.0));
    app.crop.drag = None;
    assert!(nudge(&mut app, f64::NAN, f64::INFINITY), "the key is the Crop tool's");
    assert_eq!(app.ui.crop_rect, Some([20.0, 20.0, 100.0, 60.0]), "but the frame stays put");
    app.ui.crop_rect = Some([f64::MAX, 0.0, f64::MAX, 1.0]);
    assert!(nudge(&mut app, f64::MAX, 0.0));
    assert_eq!(app.ui.crop_rect, Some([f64::MAX, 0.0, f64::MAX, 1.0]), "an overflow leaves it");
    app.ui.crop_rect = Some([f64::NAN, 0.0, 1.0, 1.0]);
    assert!(!nudge(&mut app, 1.0, 0.0));
    app.ui.crop_rect = Some([20.0, 20.0, 100.0, 60.0]);
    app.ui.tool = Tool::Brush;
    assert!(!nudge(&mut app, 1.0, 0.0));
}

fn app_selection_bounds(app: &PhotocraftApp) -> Option<(i32, i32, i32, i32)> {
    let b = app.session.active()?.doc.selection.as_ref()?.content_bounds();
    Some((b.x0, b.y0, b.x1, b.y1))
}

/// #1919: X swaps the pending frame between portrait and landscape about its centre (keeping
/// its angle and swapping a ratio preset); without a frame X still swaps the colours.
#[test]
fn x_swaps_the_frame_orientation_not_the_colours() {
    use egui::Key;
    let mut app = app(SampleType::U16);
    app.run("tools.setColors", json!({"foreground": "#ff0000", "background": "#0000ff"})).unwrap();
    let fg = |app: &PhotocraftApp| app.session.tools.foreground;
    let red = fg(&app);
    app.ui.crop_rect = Some([20.0, 20.0, 100.0, 60.0]);
    app.ui.crop_angle = 25.0;
    key(&mut app, Key::X, NONE);
    assert_eq!(app.ui.crop_rect, Some([40.0, 0.0, 80.0, 80.0]), "40 x 80 about the centre (60, 40)");
    assert_eq!(app.ui.crop_angle, 25.0);
    assert_eq!(fg(&app), red, "the colours stay");
    key(&mut app, Key::X, NONE);
    assert_eq!(app.ui.crop_rect, Some([20.0, 20.0, 100.0, 60.0]), "and back");
    // A ratio preset swaps with the frame, as the options bar's ⇄ does.
    app.ui.tool_options.crop_ratio = "16:9".into();
    assert!(swap_orientation(&mut app));
    assert_eq!(app.ui.tool_options.crop_ratio, "9:16");
    app.ui.tool_options.crop_ratio = "original".into();
    assert!(swap_orientation(&mut app));
    assert_eq!(app.ui.tool_options.crop_ratio, "100:200", "the document's size on its side");
    app.ui.tool_options.crop_ratio = String::new();
    assert!(swap_orientation(&mut app));
    assert_eq!(app.ui.tool_options.crop_ratio, "", "no ratio stays none");
    assert_eq!(swapped_ratio("1.414:1", 1.0, 1.0).as_deref(), Some("1:1.414"), "decimals survive the swap (#6487)");
    assert_eq!(swapped_ratio("0:5", 1.0, 1.0), None);
    assert_eq!(swapped_ratio("nonsense", 1.0, 1.0), None);
    // No frame, or another tool: X is Switch Foreground and Background Colors.
    app.ui.crop_rect = None;
    key(&mut app, Key::X, NONE);
    assert_ne!(fg(&app), red, "swapped");
    app.ui.crop_rect = Some([20.0, 20.0, 100.0, 60.0]);
    app.ui.tool = Tool::Brush;
    key(&mut app, Key::X, NONE);
    assert_eq!(fg(&app), red, "swapped back");
    assert_eq!(app.ui.crop_rect, Some([20.0, 20.0, 100.0, 60.0]));
    // Huge or broken frames never panic and are never made non-finite.
    app.ui.tool = Tool::Crop;
    app.ui.crop_rect = Some([-f64::MAX, 0.0, f64::MAX, 1.0]);
    assert!(swap_orientation(&mut app));
    assert_eq!(app.ui.crop_rect, Some([-f64::MAX, 0.0, f64::MAX, 1.0]));
    app.ui.crop_rect = Some([f64::NAN, 0.0, 1.0, 1.0]);
    assert!(!swap_orientation(&mut app));
}

/// #1919: W × H beside the pointer while the frame is drawn or resized (its own size when
/// turned), not while it moves or turns or between gestures.
#[test]
fn readout_shows_the_frame_size_while_drawing_or_resizing() {
    let mut app = app(SampleType::U8);
    let ev = |app: &mut PhotocraftApp, e: ToolEvent| tool_event(app, e, NONE);
    ev(&mut app, ToolEvent::Down { x: 10.0, y: 10.0, pressure: 1.0 });
    assert_eq!(sizing(&app), None, "nothing drawn yet");
    ev(&mut app, ToolEvent::Move { x: 70.4, y: 50.6, pressure: 1.0 });
    assert_eq!(sizing(&app), Some([60.0, 41.0]), "rounded as ↵ crops it");
    ev(&mut app, ToolEvent::Up { x: 70.4, y: 50.6 });
    assert_eq!(sizing(&app), None);
    // Resizing a turned frame by its right edge: the frame's own width grows.
    app.ui.crop_rect = Some([20.0, 20.0, 100.0, 60.0]);
    app.ui.crop_angle = 30.0;
    let grab = turn([100.0, 40.0], [60.0, 40.0], 30.0);
    let to = [grab[0] + 10.0 * 30f64.to_radians().cos(), grab[1] + 10.0 * 30f64.to_radians().sin()];
    ev(&mut app, ToolEvent::Down { x: grab[0], y: grab[1], pressure: 1.0 });
    ev(&mut app, ToolEvent::Move { x: to[0], y: to[1], pressure: 1.0 });
    assert!(matches!(app.crop.drag, Some(CropDrag::Resize { .. })));
    assert_eq!(sizing(&app), Some([90.0, 40.0]));
    ev(&mut app, ToolEvent::Up { x: to[0], y: to[1] });
    // Moving shows no size.
    ev(&mut app, ToolEvent::Down { x: 60.0, y: 40.0, pressure: 1.0 });
    ev(&mut app, ToolEvent::Move { x: 62.0, y: 40.0, pressure: 1.0 });
    assert!(matches!(app.crop.drag, Some(CropDrag::Move { .. })));
    assert_eq!(sizing(&app), None);
    ev(&mut app, ToolEvent::Up { x: 62.0, y: 40.0 });
    assert_eq!(frame_size([0.0, 0.0, 0.2, 0.4]), Some([1.0, 1.0]));
    assert_eq!(frame_size([f64::NAN, 0.0, 1.0, 1.0]), None);
    assert_eq!(frame_size([-f64::MAX, 0.0, f64::MAX, 1.0]), None);
}
