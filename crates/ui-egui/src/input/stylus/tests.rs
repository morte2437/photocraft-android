use super::*;

/// Hold a touch contact still for 1.2 s over a click-sensing area, with the press-and-hold
/// handling the app does in `raw_input_hook`. Did the area see a secondary click?
fn held_contact_right_clicks(hover_first: bool, force: Option<f32>) -> bool {
    let ctx = egui::Context::default();
    let mut contacts = PenContacts::default();
    let mut right_clicked = false;
    let ev = |phase, force| egui::Event::Touch { device_id: egui::TouchDeviceId(1), id: egui::TouchId(3), phase, pos: egui::pos2(50.0, 50.0), force };
    // A first frame lays the area out, so the press hits it.
    let mut frames = vec![(0.0, vec![])];
    if hover_first {
        frames.push((0.05, vec![ev(egui::TouchPhase::Move, None), egui::Event::PointerMoved(egui::pos2(50.0, 50.0))]));
    }
    frames.push((
        0.1,
        vec![
            ev(egui::TouchPhase::Start, force),
            egui::Event::PointerMoved(egui::pos2(50.0, 50.0)),
            egui::Event::PointerButton { pos: egui::pos2(50.0, 50.0), button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE },
        ],
    ));
    for t in [0.4, 0.7, 1.0, 1.3] {
        frames.push((t, vec![]));
    }
    for (time, mut events) in frames {
        // egui-winit emits the touch and its emulated pointer events together.
        let mut raw =
            egui::RawInput { time: Some(time), screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(200.0, 200.0))), ..Default::default() };
        raw.events.append(&mut events);
        let pen = contacts.observe(&raw.events);
        pen_hold_is_no_right_click(&ctx, pen);
        let mut out = ctx.run_ui(raw, |ui| {
            let r = ui.allocate_rect(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(200.0, 200.0)), egui::Sense::click_and_drag());
            right_clicked |= r.secondary_clicked();
        });
        out.textures_delta.clear();
    }
    right_clicked
}

#[test]
fn a_held_pen_is_not_a_right_click_but_a_held_finger_still_is() {
    assert!(held_contact_right_clicks(false, None), "a finger's press-and-hold is egui's context menu");
    assert!(!held_contact_right_clicks(true, None), "a pen that hovered first");
    assert!(!held_contact_right_clicks(false, Some(0.4)), "a contact that reports pressure");
}

#[test]
fn pen_contacts_follow_hover_force_and_lift() {
    use egui::TouchPhase::{Cancel, End, Move, Start};
    let t = |id, phase, force| egui::Event::Touch { device_id: egui::TouchDeviceId(1), id: egui::TouchId(id), phase, pos: egui::pos2(1.0, 1.0), force };
    let mut c = PenContacts::default();
    assert!(!c.observe(&[t(1, Move, None)]), "hovering is no contact");
    assert!(c.observe(&[t(1, Start, None)]));
    assert!(c.observe(&[]), "still down");
    assert!(c.observe(&[t(1, End, None)]), "the lift frame still counts");
    assert!(!c.observe(&[]));
    // A finger, until it reports a force.
    assert!(!c.observe(&[t(9, Start, None), t(9, Move, None)]));
    assert!(c.observe(&[t(9, Move, Some(0.5))]));
    assert!(c.observe(&[t(9, Cancel, None)]));
    assert!(!c.observe(&[]));
}

fn touch(phase: egui::TouchPhase, force: Option<f32>) -> egui::Event {
    egui::Event::Touch { device_id: egui::TouchDeviceId(1), id: egui::TouchId(0), phase, pos: egui::pos2(1.0, 1.0), force }
}

fn pen(pressure: f32) -> PenSample {
    PenSample { pressure, ..Default::default() }
}

#[test]
fn the_feed_queues_every_sample_in_order_and_keeps_the_latest() {
    let feed = StylusFeed::default();
    for p in [0.1, 0.2, 0.3] {
        feed.set(Some(pen(p)));
    }
    assert_eq!(feed.get().map(|s| s.pressure), Some(0.3));
    assert_eq!(feed.drain().iter().map(|s| s.pressure).collect::<Vec<_>>(), vec![0.1, 0.2, 0.3]);
    assert!(feed.drain().is_empty(), "drained once");
    feed.set(None);
    assert_eq!(feed.get(), None, "a lifted pen is not queued");
    assert!(feed.drain().is_empty());
    // A UI that stops draining keeps only the newest samples.
    for i in 0..FEED_QUEUE + 10 {
        feed.set(Some(pen((i % 100) as f32 / 100.0)));
    }
    assert_eq!(feed.drain().len(), FEED_QUEUE);
}

#[test]
fn each_move_takes_its_share_of_the_frames_pen_samples() {
    let mut s = Stylus::default();
    s.feed.set(Some(pen(0.2)));
    s.update(&[]);
    s.begin_stroke();
    // Next frame: the pen reported 0.4, 0.6 and 0.8 while six pointer moves arrived.
    for p in [0.4, 0.6, 0.8] {
        s.feed.set(Some(pen(p)));
    }
    s.update(&[]);
    let got: Vec<f32> = (0..6)
        .map(|k| {
            s.select(k, 6);
            s.pressure()
        })
        .collect();
    let want = [0.3, 0.4, 0.5, 0.6, 0.7, 0.8];
    assert!(got.iter().zip(want).all(|(g, w)| (g - w).abs() < 1e-6), "{got:?}");
    s.clear_selection();
    assert_eq!(s.pressure(), 0.8, "back to the current sample");
}

#[test]
fn the_pressure_curve_reshapes_pen_pressure_but_not_the_mouse() {
    let mut s = Stylus::default();
    s.set_pressure_curve(&[[0.0, 0.0], [0.3, 0.6], [1.0, 1.0]]);
    s.feed.set(Some(pen(0.3)));
    s.update(&[]);
    assert!((s.pressure() - 0.6).abs() < 1e-5, "{}", s.pressure());
    s.set_pressure_curve(&[[0.0, 0.0], [1.0, 1.0]]);
    assert!((s.pressure() - 0.3).abs() < 1e-6, "linear again");
    // A mouse is full pressure whatever the curve.
    let mut m = Stylus::default();
    m.set_pressure_curve(&[[0.0, 0.0], [1.0, 0.2]]);
    m.update(&[egui::Event::PointerMoved(egui::pos2(1.0, 1.0))]);
    assert_eq!(m.pressure(), 1.0);
}

#[test]
fn a_second_update_in_the_same_frame_keeps_its_samples() {
    // The canvas runs once per view, and egui may run a frame in several passes.
    let mut s = Stylus::default();
    s.feed.set(Some(pen(0.4)));
    s.feed.set(Some(pen(0.8)));
    s.update_for_frame(7, &[]);
    s.update_for_frame(7, &[]);
    s.select(0, 2);
    assert!((s.pressure() - 0.4).abs() < 1e-6, "{}", s.pressure());
}

#[test]
fn a_new_stroke_never_starts_from_an_old_strokes_pressure() {
    let mut s = Stylus::default();
    s.feed.set(Some(pen(0.9)));
    s.update(&[]);
    s.feed.set(Some(pen(0.1)));
    s.update(&[]);
    s.begin_stroke();
    s.select(0, 2);
    assert!((s.pressure() - 0.1).abs() < 1e-6, "{}", s.pressure());
}

#[test]
fn a_mouse_or_use_pressure_off_paints_at_full_pressure() {
    let mut s = Stylus::default();
    s.update(&[egui::Event::PointerMoved(egui::pos2(1.0, 1.0))]);
    s.select(0, 1);
    assert_eq!(s.pressure(), 1.0);
    s.feed.set(Some(pen(0.3)));
    s.use_pressure = false;
    s.update(&[]);
    s.select(0, 1);
    assert_eq!(s.pressure(), 1.0);
}

#[test]
fn rotation_between_samples_takes_the_short_way_round() {
    let mut s = Stylus::default();
    s.feed.set(Some(PenSample { rotation: 350.0, ..pen(1.0) }));
    s.update(&[]);
    s.feed.set(Some(PenSample { rotation: 10.0, ..pen(1.0) }));
    s.update(&[]);
    s.select(0, 2);
    let r = s.sample().map_or(-1.0, |p| p.rotation);
    assert!(r.abs() < 1e-3 || (r - 360.0).abs() < 1e-3, "halfway from 350° to 10° is 0°, got {r}");
}

#[test]
fn mouse_is_full_pressure() {
    let mut s = Stylus::default();
    s.update(&[egui::Event::PointerMoved(egui::pos2(3.0, 4.0))]);
    assert_eq!(s.pressure(), 1.0);
    assert_eq!(s.sample(), None);
}

#[test]
fn touch_force_drives_pressure_until_lift() {
    let mut s = Stylus::default();
    s.update(&[touch(egui::TouchPhase::Start, Some(0.25))]);
    assert_eq!(s.pressure(), 0.25);
    s.update(&[touch(egui::TouchPhase::Move, Some(0.8)), touch(egui::TouchPhase::Move, Some(1.7))]);
    assert_eq!(s.pressure(), 1.0, "clamped");
    s.update(&[touch(egui::TouchPhase::Move, Some(f32::NAN))]);
    assert_eq!(s.pressure(), 1.0);
    s.update(&[touch(egui::TouchPhase::Move, Some(0.4))]);
    assert_eq!(s.pressure(), 0.4);
    // The lift frame still paints at the last force; the next frame is back to the mouse.
    s.update(&[touch(egui::TouchPhase::End, None)]);
    assert_eq!(s.pressure(), 0.4);
    s.update(&[]);
    assert_eq!(s.pressure(), 1.0);
}

fn touch_id(id: u64, phase: egui::TouchPhase, force: Option<f32>) -> egui::Event {
    egui::Event::Touch { device_id: egui::TouchDeviceId(1), id: egui::TouchId(id), phase, pos: egui::pos2(1.0, 1.0), force }
}

#[test]
fn a_light_windows_pen_tap_is_not_full_pressure() {
    use egui::TouchPhase::{End, Move, Start};
    // winit on Windows reports a pen pressed below its first pressure level with no force
    // (pressure 0 maps to `None`). The pen hovered first, so the tap is pressure 0, not a
    // mouse click's 1.
    let mut s = Stylus::default();
    s.update(&[touch_id(5, Move, None)]);
    assert_eq!(s.pressure(), 1.0, "hovering is no contact");
    s.update(&[touch_id(5, Start, None), touch_id(5, End, None)]);
    assert_eq!(s.pressure(), 0.0);
    assert_eq!(s.tap_pressure(), 0.0);
    s.update(&[]);
    assert_eq!(s.pressure(), 1.0, "lifted");
    // A pen known from an earlier force: a stroke pressed before its first level starts at 0
    // and then follows the pen.
    let mut s = Stylus::default();
    s.update(&[touch_id(7, Start, Some(0.5)), touch_id(7, End, None)]);
    s.update(&[]);
    s.update(&[touch_id(8, Start, None)]);
    assert_eq!(s.pressure(), 0.0);
    s.update(&[touch_id(8, Move, Some(0.3))]);
    assert_eq!(s.pressure(), 0.3);
    // A force-less move mid-stroke keeps the last force.
    s.update(&[touch_id(8, Move, None)]);
    assert_eq!(s.pressure(), 0.3);
}

#[test]
fn a_finger_without_force_still_paints_at_full_pressure() {
    use egui::TouchPhase::{End, Move, Start};
    let mut s = Stylus::default();
    s.update(&[touch_id(1, Start, None), touch_id(1, Move, None)]);
    assert_eq!(s.pressure(), 1.0);
    s.update(&[touch_id(1, End, None)]);
    s.update(&[]);
    assert_eq!(s.pressure(), 1.0);
}

#[test]
fn a_new_tap_in_the_frame_an_old_one_lifted_keeps_its_force() {
    use egui::TouchPhase::{End, Start};
    let mut s = Stylus::default();
    s.update(&[touch_id(1, Start, Some(0.6)), touch_id(1, End, None), touch_id(2, Start, Some(0.2))]);
    assert_eq!(s.pressure(), 0.2);
    s.update(&[]);
    assert_eq!(s.pressure(), 0.2, "the second contact is still down");
}

#[test]
fn feed_wins_and_is_sanitized() {
    let mut s = Stylus::default();
    s.update(&[touch(egui::TouchPhase::Start, Some(0.3))]);
    s.feed.set(Some(PenSample { pressure: 0.6, tilt_x: 120.0, tilt_y: -30.0, rotation: -90.0, eraser: false }));
    assert_eq!(s.sample(), Some(PenSample { pressure: 0.6, tilt_x: 90.0, tilt_y: -30.0, rotation: 270.0, eraser: false }));
    s.feed.set(None);
    assert_eq!(s.pressure(), 0.3);
}

#[test]
fn pen_samples_reach_the_brush_stroke() {
    use crate::canvas::{ToolEvent, tool_event};
    use serde_json::json;
    let mut app = crate::PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.session.execute("file.new", json!({"width": 120, "height": 80})).unwrap();
    app.session.execute("layer.new.layer", json!({})).unwrap();
    app.ui.tool = crate::state::Tool::Brush;
    let m = egui::Modifiers::NONE;
    app.stylus.feed.set(Some(PenSample { pressure: 0.2, tilt_x: 40.0, tilt_y: 0.0, rotation: 30.0, eraser: false }));
    let pr = app.stylus.pressure();
    tool_event(&mut app, ToolEvent::Down { x: 10.0, y: 40.0, pressure: pr }, m);
    app.stylus.feed.set(Some(PenSample { pressure: 0.9, tilt_x: 10.0, tilt_y: -5.0, rotation: 60.0, eraser: false }));
    let pr = app.stylus.pressure();
    tool_event(&mut app, ToolEvent::Move { x: 100.0, y: 40.0, pressure: pr }, m);
    tool_event(&mut app, ToolEvent::Up { x: 100.0, y: 40.0 }, m);
    let (id, p) = app.session.journal.iter().rev().find(|(id, _)| id == "paint.stroke").cloned().unwrap();
    assert_eq!(id, "paint.stroke");
    assert_eq!(p["points"][0], json!([10.0, 40.0, 0.2f32 as f64, 40.0, 0.0, 30.0]));
    assert_eq!(p["points"][1], json!([100.0, 40.0, 0.9f32 as f64, 10.0, -5.0, 60.0]));
    // A mouse stroke stays at full pressure with plain [x, y, p] points.
    app.stylus.feed.set(None);
    let pr = app.stylus.pressure();
    tool_event(&mut app, ToolEvent::Down { x: 10.0, y: 60.0, pressure: pr }, m);
    tool_event(&mut app, ToolEvent::Up { x: 90.0, y: 60.0 }, m);
    let (_, p) = app.session.journal.iter().rev().find(|(id, _)| id == "paint.stroke").cloned().unwrap();
    assert_eq!(p["points"][0], json!([10.0, 60.0, 1.0]));
}

#[test]
fn stroke_points_carry_tilt_only_when_present() {
    let mut s = Stylus::default();
    let pts = [[0.0, 0.0, 0.5], [10.0, 0.0, 0.7], [20.0, 0.0, 0.7]];
    s.begin_stroke();
    s.record_point();
    assert_eq!(s.stroke_points(&pts)[1], vec![10.0, 0.0, 0.7]);
    s.feed.set(Some(PenSample { pressure: 0.7, tilt_x: 30.0, tilt_y: 10.0, rotation: 45.0, eraser: false }));
    s.record_point();
    let out = s.stroke_points(&pts);
    assert_eq!(out[0], vec![0.0, 0.0, 0.5, 0.0, 0.0, 0.0]);
    assert_eq!(out[2], vec![20.0, 0.0, 0.7, 30.0, 10.0, 45.0]);
}

#[test]
fn use_pressure_off_ignores_pen_and_touch() {
    let mut s = Stylus { use_pressure: false, ..Default::default() };
    s.update(&[touch(egui::TouchPhase::Start, Some(0.3))]);
    s.feed.set(Some(PenSample { pressure: 0.2, tilt_x: 40.0, ..Default::default() }));
    assert_eq!((s.sample(), s.pressure()), (None, 1.0));
    s.begin_stroke();
    assert_eq!(s.stroke_points(&[[1.0, 2.0, 1.0]]), vec![vec![1.0, 2.0, 1.0]], "no tilt either");
}

#[test]
fn eraser_end_switches_tools_once_and_never_mid_drag() {
    use crate::canvas::{ToolEvent, tool_event};
    use crate::state::Tool;
    let mut app = crate::PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.session.execute("file.new", serde_json::json!({"width": 60, "height": 60})).unwrap();
    app.ui.tool = Tool::Brush;
    let pen = |eraser| Some(PenSample { pressure: 0.5, eraser, ..Default::default() });
    // A mouse never switches.
    assert!(!Stylus::sync_eraser_tool(&mut app));
    app.stylus.feed.set(pen(true));
    assert!(Stylus::sync_eraser_tool(&mut app));
    assert_eq!(app.ui.tool, Tool::Eraser);
    assert!(!Stylus::sync_eraser_tool(&mut app), "reported once");
    // Flipping back during a drag waits for the drag to end.
    tool_event(&mut app, ToolEvent::Down { x: 10.0, y: 10.0, pressure: 0.5 }, egui::Modifiers::NONE);
    app.stylus.feed.set(pen(false));
    assert!(!Stylus::sync_eraser_tool(&mut app));
    assert_eq!(app.ui.tool, Tool::Eraser);
    tool_event(&mut app, ToolEvent::Up { x: 20.0, y: 10.0 }, egui::Modifiers::NONE);
    assert!(Stylus::sync_eraser_tool(&mut app));
    assert_eq!(app.ui.tool, Tool::Brush, "the tip restores the previous tool");
    // The eraser end while the Eraser is already chosen remembers nothing to restore.
    app.ui.tool = Tool::Eraser;
    app.stylus.feed.set(pen(true));
    assert!(!Stylus::sync_eraser_tool(&mut app));
    app.stylus.feed.set(pen(false));
    assert!(!Stylus::sync_eraser_tool(&mut app));
    assert_eq!(app.ui.tool, Tool::Eraser);
}
