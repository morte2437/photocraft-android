//! #9648: with a filter dialog that has a preview window on top (Smart Sharpen, Unsharp Mask…),
//! the image under it is the preview's locator, as in Photoshop: the pointer is a small square
//! there, and a click centres the dialog's preview window on that document point, without reaching
//! the tool or making a history step. Only the press counts: a drag after it leaves the window
//! where the press put it, as in the reference application.

use egui::{Modifiers, PointerButton, Pos2, pos2, vec2};
use egui_kittest::Harness;
use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::canvas::ViewXform;
use crate::state::Tool;

/// A 400×300 document at 400 % (so it fills the window round the dialog) with the Brush selected,
/// and the `command` filter dialog open over it when there is one.
fn harness(command: Option<&str>, rotation: f32) -> Harness<'static, PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app.sync_views();
    app.ui.extras.rulers = false;
    app.ui.tool = Tool::Brush;
    let mut h = Harness::builder().with_size(vec2(1200.0, 800.0)).build_ui_state(
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
    let v = &mut h.state_mut().ui.views[0];
    (v.zoom, v.center, v.rotation, v.fit_pending) = (4.0, [200.0, 150.0], rotation, false);
    if let Some(command) = command {
        crate::filter_dialog::open(h.state_mut(), command).unwrap();
    }
    h.run_steps(4);
    h
}

fn xf(h: &Harness<'static, PhotocraftApp>) -> ViewXform {
    let app = h.state();
    let v = &app.ui.views[0];
    ViewXform {
        rect: crate::rulers::content_rect(app, app.last_canvas_rect),
        zoom: v.zoom,
        center: v.center,
        flip: false,
        rotation: v.rotation,
        aspect: app.ui.view.display_aspect(),
    }
}

/// The preview window's centre, in document pixels.
fn centre(h: &Harness<'static, PhotocraftApp>) -> [f64; 2] {
    let f = &h.state().ui.dialogs.last().unwrap().fields;
    let at = |k: &str| f.get(k).and_then(Value::as_f64).unwrap();
    [at("__pvCx"), at("__pvCy")]
}

fn near(a: [f64; 2], b: [f64; 2]) -> bool {
    (a[0] - b[0]).abs() < 0.5 && (a[1] - b[1]).abs() < 0.5
}

/// The document's revision and undo depth: a click that reached a tool or ran a command moves them.
fn edits(h: &Harness<'static, PhotocraftApp>) -> (u64, usize) {
    let st = h.state().session.active().unwrap();
    (st.revision, st.history.past_len())
}

fn button(h: &mut Harness<'static, PhotocraftApp>, p: Pos2, down: bool) {
    h.event(egui::Event::PointerButton { pos: p, button: PointerButton::Primary, pressed: down, modifiers: Modifiers::NONE });
    h.run_steps(1);
}

fn move_to(h: &mut Harness<'static, PhotocraftApp>, p: Pos2) {
    h.event(egui::Event::PointerMoved(p));
    h.run_steps(1);
}

fn click(h: &mut Harness<'static, PhotocraftApp>, p: Pos2) {
    move_to(h, p);
    button(h, p, true);
    button(h, p, false);
    h.run_steps(2);
}

#[test]
fn a_click_on_the_image_centres_the_smart_sharpen_preview() {
    let mut h = harness(Some("filter.sharpen.smartSharpen"), 0.0);
    assert!(crate::filter_preview_ui::locator(h.state()).is_some(), "Smart Sharpen has a preview window");
    let r = h.state().last_canvas_rect;
    assert!(near(centre(&h), [200.0, 150.0]), "it opens centred: {:?}", centre(&h));
    let before = edits(&h);

    // Over the image (left of the dialog) the pointer is the locator square; over the dialog it is
    // the normal pointer.
    let a = pos2(r.left() + 80.0, r.bottom() - 80.0);
    move_to(&mut h, a);
    assert_eq!(h.output().platform_output.cursor_icon, egui::CursorIcon::None, "the locator replaces the pointer");
    move_to(&mut h, r.center());
    assert_ne!(h.output().platform_output.cursor_icon, egui::CursorIcon::None, "over the dialog it is the normal pointer");

    // A click centres the preview on the document point under it, through the canvas transform.
    click(&mut h, a);
    let at = xf(&h).to_doc(a);
    assert!(near(centre(&h), at), "{:?} vs {at:?}", centre(&h));
    assert!(at[0] < 150.0 && at[1] > 200.0, "the point is off the default centre: {at:?}");

    // A drag centres it where it was pressed: neither its moves nor its release move it on.
    let b = pos2(r.left() + 140.0, r.top() + 100.0);
    move_to(&mut h, b);
    button(&mut h, b, true);
    let pressed = xf(&h).to_doc(b);
    assert!(near(centre(&h), pressed), "the press centres it: {:?} vs {pressed:?}", centre(&h));
    for i in 1..=4 {
        move_to(&mut h, b + (a - b) * (i as f32 / 4.0));
        assert!(near(centre(&h), pressed), "a held move leaves it: {:?} vs {pressed:?}", centre(&h));
    }
    button(&mut h, a, false);
    h.run_steps(2);
    assert!(near(centre(&h), pressed), "the release leaves it: {:?} vs {pressed:?}", centre(&h));

    // The Brush never painted, no command ran, and the dialog stays open.
    assert_eq!(edits(&h), before, "no stroke and no history step");
    assert!(h.state().drag.is_none(), "no tool drag started");
    assert_eq!(h.state().ui.dialogs.len(), 1);
    // The view fields stay out of the filter's params.
    let params = crate::filter_dialog::params_of(&h.state().ui.dialogs[0].fields);
    assert!(params.get("__pvCx").is_none(), "{params}");

    // With the Hand tool the click centres the preview too (the tool doesn't matter).
    h.state_mut().ui.tool = Tool::Hand;
    click(&mut h, a);
    assert!(near(centre(&h), xf(&h).to_doc(a)), "{:?}", centre(&h));
    assert_eq!(edits(&h), before);
}

#[test]
fn a_click_off_the_image_centres_the_preview_on_its_nearest_edge() {
    let mut h = harness(Some("filter.sharpen.unsharpMask"), 0.0);
    // At 100 % the 400×300 image sits in the middle; the pasteboard's left edge is far off it.
    h.state_mut().ui.views[0].zoom = 1.0;
    h.run_steps(2);
    let r = h.state().last_canvas_rect;
    let p = pos2(r.left() + 20.0, r.center().y);
    assert!(xf(&h).to_doc(p)[0] < 0.0, "the point is off the image");
    click(&mut h, p);
    let [x, y] = centre(&h);
    assert_eq!(x, 0.0, "clamped to the image's left edge");
    assert!((y - xf(&h).to_doc(p)[1]).abs() < 0.5, "{y}");
}

#[test]
fn a_rotated_view_centres_the_preview_on_the_point_under_the_pointer() {
    let mut h = harness(Some("filter.sharpen.smartSharpen"), 90.0);
    let r = h.state().last_canvas_rect;
    let a = pos2(r.left() + 80.0, r.bottom() - 80.0);
    let before = edits(&h);
    click(&mut h, a);
    let at = xf(&h).to_doc(a);
    assert!((0.0..=400.0).contains(&at[0]) && (0.0..=300.0).contains(&at[1]), "on the image: {at:?}");
    assert!(near(centre(&h), at), "{:?} vs {at:?}", centre(&h));
    assert_eq!(edits(&h), before);
}

#[test]
fn without_a_preview_dialog_the_click_still_reaches_the_tool() {
    let mut h = harness(None, 0.0);
    assert!(crate::filter_preview_ui::locator(h.state()).is_none());
    let r = h.state().last_canvas_rect;
    let before = edits(&h);
    click(&mut h, pos2(r.left() + 80.0, r.bottom() - 80.0));
    let after = edits(&h);
    assert!(after.0 != before.0 && after.1 == before.1 + 1, "the Brush paints one stroke: {before:?} -> {after:?}");
}

#[test]
fn agents_centre_the_preview_with_ui_pointer_without_painting() {
    let mut h = harness(Some("filter.sharpen.smartSharpen"), 0.0);
    let before = edits(&h);
    let ctx = egui::Context::default();
    let events = json!([{"kind": "down", "x": 60.5, "y": 250}, {"kind": "move", "x": 40, "y": 30}, {"kind": "up", "x": 40, "y": 30}]);
    let (req, _rx) = crate::control::ControlRequest::new("ui.pointer", json!({"events": events}));
    let _ = crate::control::handle(h.state_mut(), &ctx, &req);
    assert!(near(centre(&h), [60.5, 250.0]), "the press decides, not the drag: {:?}", centre(&h));
    assert_eq!(edits(&h), before, "the Brush must not paint under the dialog");
    // Off the image the centre is clamped to it.
    let events = json!([{"kind": "down", "x": 900, "y": -40}, {"kind": "up", "x": 900, "y": -40}]);
    let (req, _rx) = crate::control::ControlRequest::new("ui.pointer", json!({"events": events}));
    let _ = crate::control::handle(h.state_mut(), &ctx, &req);
    assert!(near(centre(&h), [400.0, 0.0]), "{:?}", centre(&h));

    // With the dialog cancelled the same gesture paints with the Brush again.
    h.state_mut().ui.dialogs.clear();
    let events = json!([{"kind": "down", "x": 40, "y": 30}, {"kind": "move", "x": 60, "y": 30}, {"kind": "up", "x": 60, "y": 30}]);
    let (req, _rx) = crate::control::ControlRequest::new("ui.pointer", json!({"events": events}));
    let _ = crate::control::handle(h.state_mut(), &ctx, &req);
    assert_ne!(edits(&h).0, before.0, "the Brush paints once no preview dialog is open");
}

/// Sends `events` as one `ui.pointer` request, then lets the frames run as between requests.
fn pointer(h: &mut Harness<'static, PhotocraftApp>, events: Value) {
    let (req, _rx) = crate::control::ControlRequest::new("ui.pointer", json!({"events": events}));
    let _ = crate::control::handle(h.state_mut(), &egui::Context::default(), &req);
    h.run_steps(2);
}

/// Agents send a press, its moves and its release as separate requests: only the press centres
/// the preview; a hover, a move with the press held and the release leave it.
#[test]
fn ui_pointer_centres_only_on_the_press() {
    let mut h = harness(Some("filter.sharpen.smartSharpen"), 0.0);
    pointer(&mut h, json!([{"kind": "move", "x": 40, "y": 30}]));
    // An event without a kind is a move, so a hover too.
    pointer(&mut h, json!([{"x": 50, "y": 40}]));
    assert!(near(centre(&h), [200.0, 150.0]), "a hover leaves the preview where it was: {:?}", centre(&h));
    pointer(&mut h, json!([{"kind": "down", "x": 40, "y": 30}]));
    assert!(near(centre(&h), [40.0, 30.0]), "{:?}", centre(&h));
    pointer(&mut h, json!([{"kind": "move", "x": 80, "y": 60}]));
    assert!(near(centre(&h), [40.0, 30.0]), "a move while the press is held leaves it: {:?}", centre(&h));
    pointer(&mut h, json!([{"kind": "up", "x": 80, "y": 60}]));
    pointer(&mut h, json!([{"kind": "move", "x": 120, "y": 90}]));
    assert!(near(centre(&h), [40.0, 30.0]), "so do the release and a later hover: {:?}", centre(&h));
}

/// A Brush stroke an agent began before the dialog opened still ends along its own path: its moves
/// and release reach the tool, not the locator.
#[test]
fn a_tool_gesture_begun_before_the_dialog_still_ends_under_it() {
    let mut h = harness(None, 0.0);
    let before = edits(&h);
    pointer(&mut h, json!([{"kind": "down", "x": 40, "y": 30}, {"kind": "move", "x": 80, "y": 30}]));
    assert!(h.state().drag.is_some(), "the Brush stroke is under way");
    crate::filter_dialog::open(h.state_mut(), "filter.sharpen.smartSharpen").unwrap();
    h.run_steps(2);
    assert!(h.state().drag.is_some(), "the stroke is still open under the dialog");
    pointer(&mut h, json!([{"kind": "move", "x": 80, "y": 120}, {"kind": "move", "x": 120, "y": 120}]));
    let points = h.state().drag.as_ref().map(|d| d.points.clone()).unwrap_or_default();
    assert!(points.iter().any(|p| p[1] > 100.0), "the moves under the dialog reached the stroke: {points:?}");
    pointer(&mut h, json!([{"kind": "up", "x": 120, "y": 120}]));
    assert!(h.state().drag.is_none(), "the release reached the Brush");
    assert_eq!(edits(&h).1, before.1 + 1, "the stroke is one history step");
    assert!(near(centre(&h), [200.0, 150.0]), "the stroke's moves and release are not locator presses: {:?}", centre(&h));
}

/// The Crop tool keeps its frame drag apart from `app.drag`: one an agent began before the dialog
/// opened still ends along its path, so no stale drag reshapes the frame on later hovers.
#[test]
fn a_crop_drag_begun_before_the_dialog_still_ends_under_it() {
    let mut h = harness(None, 0.0);
    h.state_mut().ui.tool = Tool::Crop;
    h.run_steps(2);
    pointer(&mut h, json!([{"kind": "down", "x": 40, "y": 30}, {"kind": "move", "x": 80, "y": 60}]));
    assert!(h.state().crop.drag.is_some(), "the Crop frame drag is under way");
    crate::filter_dialog::open(h.state_mut(), "filter.sharpen.smartSharpen").unwrap();
    h.run_steps(2);
    pointer(&mut h, json!([{"kind": "move", "x": 160, "y": 120}]));
    let r = h.state().ui.crop_rect.expect("a crop frame");
    assert!(r[2] > 150.0 && r[3] > 110.0, "the move under the dialog reached the frame before the release: {r:?}");
    pointer(&mut h, json!([{"kind": "up", "x": 160, "y": 120}]));
    assert!(h.state().crop.drag.is_none(), "the release reached the Crop tool");
    let r = h.state().ui.crop_rect.expect("a crop frame");
    assert!(r[2] > 150.0 && r[3] > 110.0, "the moves under the dialog reached the frame: {r:?}");
    assert!(near(centre(&h), [200.0, 150.0]), "the drag's moves and release are not locator presses: {:?}", centre(&h));
}

/// A second press while an agent's Brush stroke is still open (no mouse can make one) is dropped:
/// it neither centres the preview nor reaches the Brush, and the stroke's own moves and release
/// still finish it.
#[test]
fn a_press_during_an_open_tool_gesture_is_dropped() {
    let mut h = harness(None, 0.0);
    let before = edits(&h);
    pointer(&mut h, json!([{"kind": "down", "x": 40, "y": 30}, {"kind": "move", "x": 80, "y": 30}]));
    crate::filter_dialog::open(h.state_mut(), "filter.sharpen.smartSharpen").unwrap();
    h.run_steps(2);
    let points = h.state().drag.as_ref().map(|d| d.points.len()).unwrap_or_default();
    pointer(&mut h, json!([{"kind": "down", "x": 300, "y": 200}]));
    assert!(near(centre(&h), [200.0, 150.0]), "the extra press doesn't centre the preview: {:?}", centre(&h));
    assert_eq!(h.state().drag.as_ref().map(|d| d.points.len()), Some(points), "nor does it reach the open stroke");
    pointer(&mut h, json!([{"kind": "move", "x": 80, "y": 120}]));
    assert!(near(centre(&h), [200.0, 150.0]), "the stroke's moves are not a locator drag: {:?}", centre(&h));
    let pts = h.state().drag.as_ref().map(|d| d.points.clone()).unwrap_or_default();
    assert!(pts.iter().any(|p| p[1] > 100.0), "the move reached the stroke: {pts:?}");
    pointer(&mut h, json!([{"kind": "up", "x": 80, "y": 120}]));
    assert!(h.state().drag.is_none(), "the release ended the stroke");
    assert_eq!(edits(&h).1, before.1 + 1, "the stroke is one history step");
}
