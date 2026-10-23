//! #5473 in the app: a focused value field takes the wheel over the canvas, which neither pans
//! nor zooms with it.

use egui::accesskit::Role;
use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;

use crate::PhotocraftApp;
use crate::state::Tool;

fn harness() -> Harness<'static, PhotocraftApp> {
    let mut h = Harness::builder().with_size(vec2(1440.0, 900.0)).with_max_steps(64).build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 64, "height": 48})).unwrap();
        app.ui.tool = Tool::Brush;
        app
    });
    h.run_steps(8);
    h
}

/// The value field showing `value`: the options bar's Smoothing is the brush's only one at 10.
fn field<'a>(h: &'a Harness<'_, PhotocraftApp>, value: &str) -> egui_kittest::Node<'a> {
    h.query_all_by_role(Role::SpinButton).find(|n| n.value().as_deref() == Some(value)).unwrap()
}

fn wheel(h: &mut Harness<'_, PhotocraftApp>, at: egui::Pos2, lines: f32, modifiers: egui::Modifiers) {
    h.hover_at(at);
    h.run_steps(1);
    h.event(egui::Event::MouseWheel { unit: egui::MouseWheelUnit::Line, delta: vec2(0.0, lines), phase: egui::TouchPhase::Move, modifiers });
    h.run_steps(6);
}

/// (zoom, centre) of the document's view.
fn view(h: &Harness<'_, PhotocraftApp>) -> (f32, [f32; 2]) {
    let v = &h.state().ui.views[0];
    (v.zoom, v.center)
}

#[test]
fn a_focused_field_takes_the_wheel_over_the_canvas() {
    let mut h = harness();
    let canvas = egui::pos2(640.0, 500.0);
    let before = view(&h);
    wheel(&mut h, canvas, -1.0, egui::Modifiers::NONE);
    assert_ne!(view(&h), before, "without a focused field the wheel pans the canvas");
    let before = view(&h);
    field(&h, "10").click();
    h.run();
    wheel(&mut h, canvas, -2.0, egui::Modifiers::NONE);
    assert_eq!(view(&h), before, "the canvas neither pans nor zooms");
    assert!(h.query_all_by_role(Role::SpinButton).any(|n| n.value().as_deref() == Some("8")), "Smoothing stepped from 10 to 8");
    // ⌥/Alt + wheel is still the view's zoom.
    wheel(&mut h, canvas, 1.0, egui::Modifiers::ALT);
    assert_ne!(view(&h).0, before.0, "Alt + wheel zooms");
    assert!(h.query_all_by_role(Role::SpinButton).any(|n| n.value().as_deref() == Some("8")));
}

#[test]
fn the_wheel_over_a_field_steps_it_without_a_click() {
    let mut h = harness();
    let before = view(&h);
    let at = field(&h, "10").rect().center();
    wheel(&mut h, at, 1.0, egui::Modifiers::NONE);
    assert!(h.query_all_by_role(Role::SpinButton).any(|n| n.value().as_deref() == Some("11")), "Smoothing stepped from 10 to 11");
    wheel(&mut h, at, 1.0, egui::Modifiers::SHIFT);
    assert!(h.query_all_by_role(Role::SpinButton).any(|n| n.value().as_deref() == Some("21")), "Shift: ten a notch");
    assert!(!field(&h, "21").is_focused(), "hovering doesn't put the cursor in the field");
    assert_eq!(view(&h), before, "the canvas didn't move");
}

/// How far a wheel of `delta` lines with `modifiers` over the canvas moves the view, a Smoothing
/// field focused or not, once egui's smoothed scroll has run out. The field isn't stepped.
fn canvas_pan(focus: bool, delta: egui::Vec2, modifiers: egui::Modifiers) -> (f32, [f32; 2]) {
    let mut h = harness();
    if focus {
        field(&h, "10").click();
        h.run();
    }
    let canvas = egui::pos2(640.0, 500.0);
    h.hover_at(canvas);
    h.run_steps(1);
    let before = view(&h);
    h.event(egui::Event::MouseWheel { unit: egui::MouseWheelUnit::Line, delta, phase: egui::TouchPhase::Move, modifiers });
    h.run_steps(40);
    assert!(h.query_all_by_role(Role::SpinButton).any(|n| n.value().as_deref() == Some("10")), "focus {focus}: the field wasn't stepped");
    let after = view(&h);
    (after.0 / before.0, [after.1[0] - before.1[0], after.1[1] - before.1[1]])
}

/// The wheel a focused field leaves to the view moves it as far as with no field focused: its
/// smoothed tail isn't cut off (⌘/Ctrl + wheel, and a plain sideways trackpad swipe).
#[test]
fn the_view_keeps_its_whole_wheel_while_a_field_is_focused() {
    for (delta, m) in [(vec2(0.0, -1.0), egui::Modifiers::COMMAND), (vec2(0.0, -1.0), egui::Modifiers::CTRL), (vec2(-1.0, 0.0), egui::Modifiers::NONE)] {
        let free = canvas_pan(false, delta, m);
        assert!(free != (1.0, [0.0, 0.0]), "{delta:?} {m:?}: the wheel moves the view");
        let focused = canvas_pan(true, delta, m);
        let close = (focused.0 - free.0).abs() < 1e-3 && (0..2).all(|k| (focused.1[k] - free.1[k]).abs() < 0.5);
        assert!(close, "{delta:?} {m:?}: {focused:?} with a field focused, {free:?} without");
    }
}
