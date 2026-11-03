//! #3373: Field Blur's pins are placed, dragged and tuned on the image under the dialog, the
//! preview follows them, and OK applies them.

use egui::{Modifiers, PointerButton, Pos2, pos2, vec2};
use egui_kittest::Harness;
use serde_json::json;

use crate::PhotocraftApp;
use crate::blur_pins::{COMMAND, IRIS, Iris, Pin, SPIN, TILT, pins, selected};
use crate::canvas::ViewXform;

/// A 400×300 document at 400 %, so it fills the window round the dialog, with Field Blur open.
fn harness(selection: bool) -> Harness<'static, PhotocraftApp> {
    harness_for(COMMAND, selection)
}

/// [`harness`] with the Blur Gallery `command` open.
fn harness_for(command: &str, selection: bool) -> Harness<'static, PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    if selection {
        app.run("select.rect", json!({"x": 100, "y": 50, "width": 200, "height": 100})).unwrap();
    }
    app.sync_views();
    app.ui.extras.rulers = false;
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
    (v.zoom, v.center, v.fit_pending) = (4.0, [200.0, 150.0], false);
    crate::filter_dialog::open(h.state_mut(), command).unwrap();
    h.run_steps(4);
    h
}

fn fields(h: &Harness<'static, PhotocraftApp>) -> serde_json::Map<String, serde_json::Value> {
    h.state().ui.dialogs.last().unwrap().fields.clone()
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

fn button(h: &mut Harness<'static, PhotocraftApp>, p: Pos2, down: bool) {
    h.event(egui::Event::PointerButton { pos: p, button: PointerButton::Primary, pressed: down, modifiers: Modifiers::NONE });
    h.run_steps(1);
}

fn move_to(h: &mut Harness<'static, PhotocraftApp>, p: Pos2) {
    h.event(egui::Event::PointerMoved(p));
    h.run_steps(1);
}

fn drag(h: &mut Harness<'static, PhotocraftApp>, path: &[Pos2]) {
    move_to(h, path[0]);
    button(h, path[0], true);
    for p in &path[1..] {
        move_to(h, *p);
    }
    button(h, *path.last().unwrap(), false);
    h.run_steps(2);
}

/// Where pin `p` is on screen.
fn on_screen(h: &Harness<'static, PhotocraftApp>, p: Pin, bounds: [f64; 4]) -> Pos2 {
    let (x, y) = (bounds[0] + p.x * (bounds[2] - bounds[0]), bounds[1] + p.y * (bounds[3] - bounds[1]));
    xf(h).to_screen(x as f32, y as f32)
}

#[test]
fn it_opens_with_one_selected_pin_in_the_middle_of_the_bounds() {
    let h = harness(false);
    let f = fields(&h);
    assert_eq!(pins(&f), vec![Pin { x: 0.5, y: 0.5, blur: 15.0, iris: None, tilt: None }]);
    assert_eq!(selected(&f), Some(0));
    assert_eq!(f["__bounds"], json!([0, 0, 400, 300]));
    assert!(!f.contains_key("centerX") && !f.contains_key("centerY"), "the pins replace the single centre");
    // With a selection the pins are fractions of its bounds, as the engine reads them.
    let h = harness(true);
    assert_eq!(fields(&h)["__bounds"], json!([100, 50, 300, 150]));
}

#[test]
fn pins_are_added_moved_tuned_and_removed_on_the_image() {
    let mut h = harness(false);
    let r = h.state().last_canvas_rect;
    // A click on the free image (left of the dialog) adds a pin there, selected.
    let a = pos2(r.left() + 80.0, r.bottom() - 80.0);
    move_to(&mut h, a);
    button(&mut h, a, true);
    button(&mut h, a, false);
    h.run_steps(2);
    let f = fields(&h);
    let list = pins(&f);
    assert_eq!(list.len(), 2, "{list:?}");
    assert_eq!(selected(&f), Some(1));
    let at = xf(&h).to_doc(a);
    assert!((list[1].x - at[0] / 400.0).abs() < 1e-3 && (list[1].y - at[1] / 300.0).abs() < 1e-3, "{:?} vs {at:?}", list[1]);
    assert_eq!(list[1].blur, 15.0, "a new pin takes the selected pin's blur");
    // Dragging the pin moves it by the pointer's movement (4 screen points per pixel at 400 %).
    let b = a + vec2(40.0, -20.0);
    drag(&mut h, &[a, a + vec2(20.0, -10.0), b]);
    let moved = pins(&fields(&h))[1];
    assert!((moved.x - (list[1].x + 10.0 / 400.0)).abs() < 1e-3 && (moved.y - (list[1].y - 5.0 / 300.0)).abs() < 1e-3, "{moved:?}");
    assert!(fields(&h).get("__drag").is_none(), "the drag ends on release");
    // A quarter turn clockwise on its ring adds a quarter of the 0–500 px range.
    let c = on_screen(&h, moved, [0.0, 0.0, 400.0, 300.0]);
    let ring: Vec<Pos2> = (0..=6).map(|k| c + vec2(24.0 * (k as f32 * 0.2618).cos(), 24.0 * (k as f32 * 0.2618).sin())).collect();
    drag(&mut h, &ring);
    let tuned = pins(&fields(&h))[1];
    assert!((tuned.blur - 140.0).abs() < 2.0, "15 + 125 px: {}", tuned.blur);
    assert!((tuned.x - moved.x).abs() < 1e-9, "the ring doesn't move the pin");
    assert_eq!(fields(&h)["blur"], json!(tuned.blur), "the dialog's Blur follows the selected pin");
    // The preview runs the filter with the pins (computed by the canvas's GPU path in the app;
    // asked for directly here, as the headless harness has no GPU).
    assert!(crate::canvas::ensure_filter_preview(h.state_mut(), 0, &egui::Context::default()).is_some());
    let preview = h.state().filter_preview.as_ref().expect("a live preview");
    assert_eq!(preview.key.params["pins"].as_array().map(Vec::len), Some(2));
    // Delete removes the selected pin; OK applies the rest in one step.
    h.key_press(egui::Key::Delete);
    h.run_steps(2);
    assert_eq!(pins(&fields(&h)).len(), 1);
    assert_eq!(selected(&fields(&h)), Some(0));
    let id = h.state().ui.dialogs.last().unwrap().id;
    let steps = h.state().session.active().unwrap().history.past_len();
    crate::dialogs::confirm(h.state_mut(), id).unwrap();
    let (cmd, p) = h.state().session.journal.last().cloned().unwrap();
    assert_eq!(cmd, COMMAND);
    assert_eq!(p["pins"], json!([{"x": 0.5, "y": 0.5, "blur": 15.0}]), "{p}");
    assert_eq!(h.state().session.active().unwrap().history.past_len(), steps + 1);
}

/// A click outside the bounds (beside a selection) adds no pin; pins are capped.
#[test]
fn pins_stay_inside_the_bounds_and_are_capped() {
    let mut h = harness(true);
    let r = h.state().last_canvas_rect;
    let outside = pos2(r.left() + 30.0, r.top() + 30.0);
    assert!(xf(&h).to_doc(outside)[0] < 100.0, "left of the selection");
    move_to(&mut h, outside);
    button(&mut h, outside, true);
    button(&mut h, outside, false);
    h.run_steps(2);
    assert_eq!(pins(&fields(&h)).len(), 1);
    let many: Vec<_> = (0..200).map(|i| json!({"x": f64::from(i) / 200.0, "y": 0.5, "blur": 1e9})).collect();
    h.state_mut().ui.dialogs.last_mut().unwrap().fields.insert("pins".into(), json!(many));
    let list = pins(&fields(&h));
    assert_eq!(list.len(), crate::blur_pins::MAX_PINS);
    assert!(list.iter().all(|p| p.blur == 500.0));
}

#[test]
fn proxy_previews_scale_the_pins_blur() {
    let mut p = json!({"pins": [{"x": 0.2, "y": 0.3, "blur": 40.0}, {"x": 0.9, "y": 0.9, "blur": 0.0}]});
    crate::blur_pins::scale_for_proxy(&mut p, 4);
    assert_eq!(p, json!({"pins": [{"x": 0.2, "y": 0.3, "blur": 10.0}, {"x": 0.9, "y": 0.9, "blur": 0.0}]}));
    crate::blur_pins::scale_for_proxy(&mut p, 1);
    assert_eq!(p["pins"][0]["blur"], json!(10.0));
}

/// Screen point of an Iris pin's local point `(u, v)` (1 = on the outline along that axis), on the
/// 400×300 canvas (shorter side 300), as the engine places it.
fn iris_at(h: &Harness<'static, PhotocraftApp>, p: Pin, u: f64, v: f64) -> Pos2 {
    let s = p.iris.unwrap();
    let (cx, cy) = (p.x * 400.0, p.y * 300.0);
    let (x, y) = (u * s.rx * 300.0, v * s.ry * 300.0);
    let (sin, cos) = s.angle.to_radians().sin_cos();
    xf(h).to_screen((cx + x * cos - y * sin) as f32, (cy + x * sin + y * cos) as f32)
}

fn only_pin(h: &Harness<'static, PhotocraftApp>) -> Pin {
    pins(&fields(h))[0]
}

#[test]
fn iris_opens_with_its_shape_as_a_pin() {
    let h = harness_for(IRIS, false);
    let f = fields(&h);
    let list = pins(&f);
    assert_eq!(list.len(), 1);
    let (p, s, d) = (list[0], list[0].iris.unwrap(), Iris::default());
    assert_eq!((p.x, p.y, p.blur), (0.5, 0.5, 15.0));
    // The dialog's defaults come from the command spec (f32), so compare loosely.
    for (got, want) in [(s.rx, d.rx), (s.ry, d.ry), (s.angle, d.angle), (s.roundness, d.roundness), (s.feather, d.feather)] {
        assert!((got - want).abs() < 1e-6, "{s:?}");
    }
    for k in ["centerX", "radiusX", "radiusY", "angle", "roundness", "feather"] {
        assert!(!f.contains_key(k), "{k} is in the pin now");
    }
    assert!(f["pins"][0]["radiusX"].as_f64().is_some_and(|r| (r - 0.35).abs() < 1e-6), "the command's own keys: {}", f["pins"]);
}

/// The selected Iris pin's handles: an outline handle resizes its axis and turns the shape, the
/// square rounds it, the inner dots set where the blur starts; a new pin copies the shape; OK
/// applies them all.
#[test]
fn iris_handles_resize_turn_round_and_feather() {
    let mut h = harness_for(IRIS, false);
    // A small shape, so its handles sit in the free middle of the window, left of the dialog.
    let small = json!([{"x": 0.5, "y": 0.5, "blur": 15.0, "radiusX": 0.1, "radiusY": 0.08, "angle": 0.0, "roundness": 0.0, "feather": 0.5}]);
    h.state_mut().ui.dialogs.last_mut().unwrap().fields.insert("pins".into(), small);
    h.run_steps(2);
    // Drag the +u handle (30 px right of the centre) to 40 px below it: radius 40/300, a quarter turn.
    let p = only_pin(&h);
    let below = xf(&h).to_screen(200.0, 190.0);
    let from = iris_at(&h, p, 1.0, 0.0);
    drag(&mut h, &[from, from.lerp(below, 0.5), below]);
    let s = only_pin(&h).iris.unwrap();
    assert!((s.rx - 40.0 / 300.0).abs() < 2e-3 && (s.angle - 90.0).abs() < 0.5, "{s:?}");
    assert_eq!(s.ry, 0.08, "the other axis keeps its radius");
    // Drag the corner square out along the diagonal to 0.9: the exponent 6.58, roundness 76.
    let p = only_pin(&h);
    let t = 0.5f64.sqrt();
    let path = [iris_at(&h, p, t, -t), iris_at(&h, p, 0.8, -0.8), iris_at(&h, p, 0.9, -0.9)];
    drag(&mut h, &path);
    let s = only_pin(&h).iris.unwrap();
    let m = -std::f64::consts::LN_2 / 0.9f64.ln();
    assert!((s.roundness - (m - 2.0) / 6.0 * 100.0).abs() < 1.0, "{s:?}");
    // Drag a feather dot in to a fifth of the way along the diagonal.
    let p = only_pin(&h);
    let k = 0.5 * 0.9;
    let path = [iris_at(&h, p, k, k), iris_at(&h, p, 0.3, 0.3), iris_at(&h, p, 0.2, 0.2)];
    drag(&mut h, &path);
    let s = only_pin(&h).iris.unwrap();
    let want = 0.2 * 2f64.powf(1.0 / m);
    assert!((s.feather - want).abs() < 0.01, "{} vs {want}", s.feather);
    // A click away from the shape adds a pin with the same shape and blur.
    let r = h.state().last_canvas_rect;
    let at = pos2(r.left() + 80.0, r.bottom() - 80.0);
    move_to(&mut h, at);
    button(&mut h, at, true);
    button(&mut h, at, false);
    h.run_steps(2);
    let list = pins(&fields(&h));
    assert_eq!(list.len(), 2);
    assert_eq!(list[1].iris, list[0].iris);
    assert_eq!(selected(&fields(&h)), Some(1));
    let id = h.state().ui.dialogs.last().unwrap().id;
    crate::dialogs::confirm(h.state_mut(), id).unwrap();
    let (cmd, params) = h.state().session.journal.last().cloned().unwrap();
    assert_eq!(cmd, IRIS);
    assert_eq!(params["pins"].as_array().map(Vec::len), Some(2));
    assert_eq!(params["pins"][0]["angle"], json!(s.angle), "{params}");
}

/// Hostile pin values from automation are cleaned before anything uses them.
#[test]
fn iris_pins_are_cleaned() {
    let mut h = harness_for(IRIS, false);
    let bad = json!([{"x": 7, "y": -3, "blur": -1, "radiusX": 0, "radiusY": 1e9, "angle": 540.0, "roundness": 300, "feather": 2}]);
    h.state_mut().ui.dialogs.last_mut().unwrap().fields.insert("pins".into(), bad);
    let p = only_pin(&h);
    assert_eq!((p.x, p.y, p.blur), (1.0, 0.0, 0.0));
    let s = p.iris.unwrap();
    assert_eq!((s.rx, s.ry, s.angle, s.roundness, s.feather), (0.01, 1.0, -180.0, 100.0, 0.99));
    h.run_steps(2);
}

fn at_doc(h: &Harness<'static, PhotocraftApp>, x: f64, y: f64) -> Pos2 {
    xf(h).to_screen(x as f32, y as f32)
}

/// Tilt-Shift's one band on the 400×300 canvas (shorter side 300): the solid lines set the sharp
/// band, the dashed ones the transition, the dots turn it, the pin moves it; a click adds nothing;
/// OK sends the command's own params.
#[test]
fn tilt_shift_lines_and_dots_shape_the_band() {
    let mut h = harness_for(TILT, false);
    let f = fields(&h);
    assert!(!f.contains_key("pins"), "Tilt-Shift keeps its own params");
    let t = only_pin(&h).tilt.unwrap();
    assert!((t.focus - 0.1).abs() < 1e-6 && (t.transition - 0.15).abs() < 1e-6 && t.angle == 0.0, "{t:?}");
    // The band is horizontal: the solid lines at y = 150 ± 30, the dashed ones 45 px further.
    // Drag the upper dashed line, far left of the pin, out to 80 px from the centre: the transition
    // is 50 px.
    let path = [at_doc(&h, 60.0, 75.0), at_doc(&h, 60.0, 72.0), at_doc(&h, 60.0, 70.0)];
    drag(&mut h, &path);
    let t = only_pin(&h).tilt.unwrap();
    assert!((t.transition - 50.0 / 300.0).abs() < 1e-3, "{t:?}");
    // Drag the lower solid line 20 px down: the sharp band is 50 px each side.
    let path = [at_doc(&h, 60.0, 180.0), at_doc(&h, 60.0, 190.0), at_doc(&h, 60.0, 200.0)];
    drag(&mut h, &path);
    let t = only_pin(&h).tilt.unwrap();
    assert!((t.focus - 50.0 / 300.0).abs() < 1e-3 && (t.transition - 50.0 / 300.0).abs() < 1e-3, "{t:?}");
    // Turn: pull the lower line's dot round to 30° (the band's normal is (sin a, cos a)).
    let p = only_pin(&h);
    let dot = crate::blur_pins::tilt_turn_handle([0.0, 0.0, 400.0, 300.0], p, t, &xf(&h), 1.0);
    let (s30, c30) = 30f64.to_radians().sin_cos();
    let path = [at_doc(&h, dot[0], dot[1]), at_doc(&h, 200.0 + 40.0 * s30, 150.0 + 40.0), at_doc(&h, 200.0 + 60.0 * s30, 150.0 + 60.0 * c30)];
    drag(&mut h, &path);
    let t = only_pin(&h).tilt.unwrap();
    assert!((t.angle - 30.0).abs() < 0.5, "{t:?}");
    assert!((t.focus - 50.0 / 300.0).abs() < 1e-3, "turning keeps the widths");
    // The pin moves the band.
    let path = [at_doc(&h, 200.0, 150.0), at_doc(&h, 210.0, 145.0), at_doc(&h, 220.0, 140.0)];
    drag(&mut h, &path);
    let p = only_pin(&h);
    assert!((p.x - 0.55).abs() < 1e-3 && (p.y - 140.0 / 300.0).abs() < 1e-3, "{p:?}");
    // A click away from the band's lines adds nothing: there is one band.
    let r = h.state().last_canvas_rect;
    let away = pos2(r.left() + 30.0, r.top() + 30.0);
    move_to(&mut h, away);
    button(&mut h, away, true);
    button(&mut h, away, false);
    h.run_steps(2);
    assert_eq!(pins(&fields(&h)).len(), 1);
    h.key_press(egui::Key::Delete);
    h.run_steps(2);
    assert_eq!(pins(&fields(&h)).len(), 1, "Delete keeps the one band");
    let id = h.state().ui.dialogs.last().unwrap().id;
    crate::dialogs::confirm(h.state_mut(), id).unwrap();
    let (cmd, params) = h.state().session.journal.last().cloned().unwrap();
    assert_eq!(cmd, TILT);
    for (k, want) in [("centerX", 0.55), ("angle", t.angle), ("focus", t.focus), ("transition", t.transition), ("blur", 15.0)] {
        assert!(params[k].as_f64().is_some_and(|v| (v - want).abs() < 1e-3), "{k}: {params}");
    }
    assert!(params.get("pins").is_none(), "{params}");
}

/// Spin Blur: outline handles resize and turn the ellipse, the ring is the blur angle (a quarter
/// turn = 90°), a click adds a pin with the same shape, OK sends `blurAngle` per pin.
#[test]
fn spin_blur_handles_and_its_angle_ring() {
    let mut h = harness_for(SPIN, false);
    let p = only_pin(&h);
    assert_eq!(p.blur, 15.0, "the blur angle");
    assert!(p.iris.is_some_and(|s| (s.rx - 0.3).abs() < 1e-6 && (s.ry - 0.3).abs() < 1e-6));
    let small = json!([{"x": 0.5, "y": 0.5, "blurAngle": 15.0, "radiusX": 0.1, "radiusY": 0.1, "angle": 0.0}]);
    h.state_mut().ui.dialogs.last_mut().unwrap().fields.insert("pins".into(), small);
    h.run_steps(2);
    // The +v handle (30 px below the centre) dragged 45 px out to the right: radius 45/300, and
    // the shape turned so +v points right (−90°).
    let p = only_pin(&h);
    let path = [iris_at(&h, p, 0.0, 1.0), at_doc(&h, 230.0, 175.0), at_doc(&h, 245.0, 150.0)];
    drag(&mut h, &path);
    let s = only_pin(&h).iris.unwrap();
    assert!((s.ry - 45.0 / 300.0).abs() < 2e-3 && (s.angle + 90.0).abs() < 0.5 && s.rx == 0.1, "{s:?}");
    // A quarter turn clockwise on the ring adds 90° of blur angle.
    let c = at_doc(&h, 200.0, 150.0);
    let ring: Vec<Pos2> = (0..=6).map(|k| c + vec2(24.0 * (k as f32 * 0.2618).cos(), 24.0 * (k as f32 * 0.2618).sin())).collect();
    drag(&mut h, &ring);
    let p = only_pin(&h);
    assert!((p.blur - 105.0).abs() < 1.0, "{}", p.blur);
    assert_eq!(fields(&h)["blurAngle"], json!(p.blur), "the dialog's Blur Angle follows the pin");
    let r = h.state().last_canvas_rect;
    let at = pos2(r.left() + 80.0, r.bottom() - 80.0);
    move_to(&mut h, at);
    button(&mut h, at, true);
    button(&mut h, at, false);
    h.run_steps(2);
    let list = pins(&fields(&h));
    assert_eq!(list.len(), 2);
    assert_eq!((list[1].iris, list[1].blur), (list[0].iris, list[0].blur));
    let id = h.state().ui.dialogs.last().unwrap().id;
    crate::dialogs::confirm(h.state_mut(), id).unwrap();
    let (cmd, params) = h.state().session.journal.last().cloned().unwrap();
    assert_eq!(cmd, SPIN);
    assert!(params["pins"][0]["blurAngle"].as_f64().is_some_and(|a| (a - 105.0).abs() < 1.0), "{params}");
    assert!(params["pins"][0].get("blur").is_none() && params["pins"][0].get("roundness").is_none(), "{params}");
}

/// Alt+Cancel → Reset (#3871) starts the workspace again from one pin in the middle, as when it
/// opened, instead of leaving it with no pins; the bounds captured on opening stay. An Iris pin
/// gets the command's default shape back, and Spin Blur and Tilt-Shift their defaults too.
#[test]
fn alt_reset_starts_again_from_one_pin() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    let id = crate::filter_dialog::open(&mut app, COMMAND).unwrap();
    let mut fields = app.ui.dialog_mut(id).unwrap().fields.clone();
    let bounds = fields["__bounds"].clone();
    fields.insert("pins".into(), json!([{"x": 0.1, "y": 0.2, "blur": 40.0}, {"x": 0.8, "y": 0.7, "blur": 90.0}]));
    fields.insert("__selected".into(), json!(1));
    assert!(crate::dialog_alt_reset::supported(&fields));
    crate::dialog_alt_reset::apply(&mut app, &egui::Context::default(), &mut fields);
    assert_eq!(pins(&fields), [Pin { x: 0.5, y: 0.5, blur: 15.0, iris: None, tilt: None }]);
    assert_eq!(selected(&fields), Some(0));
    assert_eq!(fields["__bounds"], bounds);
    assert!(!fields.contains_key("centerX") && !fields.contains_key("centerY"), "{fields:?}");
    let id = crate::filter_dialog::open(&mut app, IRIS).unwrap();
    let mut fields = app.ui.dialog_mut(id).unwrap().fields.clone();
    let bent = json!([{"x": 0.2, "y": 0.2, "blur": 40.0, "radiusX": 0.1, "radiusY": 0.1, "angle": 30.0, "roundness": 50.0, "feather": 0.2}]);
    fields.insert("pins".into(), bent);
    crate::dialog_alt_reset::apply(&mut app, &egui::Context::default(), &mut fields);
    let list = pins(&fields);
    assert_eq!(list.len(), 1);
    assert_eq!((list[0].x, list[0].y, list[0].blur), (0.5, 0.5, 15.0));
    // The dialog's defaults come from the command spec (f32), so compare loosely.
    let (s, d) = (list[0].iris.unwrap(), Iris::default());
    for (got, want) in [(s.rx, d.rx), (s.ry, d.ry), (s.angle, d.angle), (s.roundness, d.roundness), (s.feather, d.feather)] {
        assert!((got - want).abs() < 1e-6, "{s:?}");
    }
    let id = crate::filter_dialog::open(&mut app, SPIN).unwrap();
    let mut fields = app.ui.dialog_mut(id).unwrap().fields.clone();
    fields.insert("pins".into(), json!([{"x": 0.2, "y": 0.2, "blurAngle": 90.0}, {"x": 0.7, "y": 0.6, "blurAngle": 30.0}]));
    crate::dialog_alt_reset::apply(&mut app, &egui::Context::default(), &mut fields);
    let list = pins(&fields);
    assert_eq!(list.len(), 1);
    assert_eq!((list[0].x, list[0].y, list[0].blur), (0.5, 0.5, 15.0));
    let id = crate::filter_dialog::open(&mut app, TILT).unwrap();
    let mut fields = app.ui.dialog_mut(id).unwrap().fields.clone();
    fields.insert("centerX".into(), json!(0.2));
    fields.insert("angle".into(), json!(30.0));
    crate::dialog_alt_reset::apply(&mut app, &egui::Context::default(), &mut fields);
    let p = pins(&fields)[0];
    assert_eq!((p.x, p.y, p.tilt.map(|t| t.angle)), (0.5, 0.5, Some(0.0)));
}
