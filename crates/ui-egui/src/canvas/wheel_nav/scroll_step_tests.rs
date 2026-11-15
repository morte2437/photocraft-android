//! #12028: one wheel notch over the canvas pans 16 screen points, as in Photoshop (egui's notch is
//! 40): down, and sideways with Ctrl, at any zoom. Trackpad point deltas, the Alt and Shift+Alt
//! zooms, Zoom with Scroll Wheel and panels keep their notch.

use egui::{Context, Event, Id, Modifiers, MouseWheelUnit, RawInput, TouchPhase, Vec2};

use super::{CANVAS_PAN_PER_LINE, Wheel, configure, read_with};

fn event(unit: MouseWheelUnit, delta: Vec2, modifiers: Modifiers) -> Event {
    Event::MouseWheel { source: egui::MouseWheelSource::Unknown, unit, delta, phase: TouchPhase::Move, modifiers }
}

/// What the canvas reads (unsmoothed) for one frame holding `events`; `zoom_with_wheel` and
/// `animated` are the preferences.
fn canvas_reads(events: Vec<Event>, zoom_with_wheel: bool, animated: bool) -> Option<Wheel> {
    let ctx = Context::default();
    configure(&ctx);
    let owner = Id::unique("scroll-step-test");
    let modifiers = events.iter().find_map(|e| if let Event::MouseWheel { modifiers, .. } = e { Some(*modifiers) } else { None }).unwrap_or_default();
    let events = std::iter::once(Event::ModifiersChanged(modifiers)).chain(events).collect();
    let raw = RawInput { time: Some(0.0), predicted_dt: 1.0 / 60.0, events, ..Default::default() };
    let mut got = None;
    let mut out = ctx.run_ui(raw, |ui| got = read_with(ui.ctx(), zoom_with_wheel, true, animated, owner, true, false, true));
    out.textures_delta.clear();
    got
}

fn pan(w: Option<Wheel>) -> Vec2 {
    match w {
        Some(Wheel::Pan(d)) => d,
        other => panic!("not a pan: {other:?}"),
    }
}

#[test]
fn one_notch_pans_sixteen_points() {
    assert_eq!(pan(canvas_reads(vec![event(MouseWheelUnit::Line, Vec2::new(0.0, -1.0), Modifiers::NONE)], false, false)), Vec2::new(0.0, -16.0));
    assert_eq!(pan(canvas_reads(vec![event(MouseWheelUnit::Line, Vec2::new(0.0, 3.0), Modifiers::NONE)], false, true)), Vec2::new(0.0, 48.0));
    assert_eq!(CANVAS_PAN_PER_LINE, 16.0);
}

#[test]
fn ctrl_wheel_pans_sixteen_points_sideways() {
    let ctrl = Modifiers { ctrl: true, command: true, ..Modifiers::NONE };
    assert_eq!(pan(canvas_reads(vec![event(MouseWheelUnit::Line, Vec2::new(0.0, -1.0), ctrl)], false, false)), Vec2::new(-16.0, 0.0));
}

#[test]
fn trackpad_points_pan_as_before() {
    assert_eq!(pan(canvas_reads(vec![event(MouseWheelUnit::Point, Vec2::new(0.0, -30.0), Modifiers::NONE)], false, false)), Vec2::new(0.0, -30.0));
}

#[test]
fn the_wheel_zooms_keep_their_notch() {
    let notch = |ctx: &Context| ctx.options(|o| o.input_options.line_scroll_speed);
    let egui_notch = {
        let ctx = Context::default();
        configure(&ctx);
        notch(&ctx)
    };
    assert!(egui_notch > CANVAS_PAN_PER_LINE, "egui's notch: {egui_notch}");
    // Alt + one notch up: one ×1.1 step, instant with Animated Zoom off.
    match canvas_reads(vec![event(MouseWheelUnit::Line, Vec2::new(0.0, 1.0), Modifiers::ALT)], false, false) {
        Some(Wheel::Zoom(f)) => assert!((f - 1.1).abs() < 1e-4, "{f}"),
        other => panic!("{other:?}"),
    }
    // Zoom with Scroll Wheel: the plain notch is a ×1.1 step too.
    match canvas_reads(vec![event(MouseWheelUnit::Line, Vec2::new(0.0, 1.0), Modifiers::NONE)], true, false) {
        Some(Wheel::Zoom(f)) => assert!((f - 1.1).abs() < 1e-4, "{f}"),
        other => panic!("{other:?}"),
    }
    // Shift + Alt: one snap step.
    assert_eq!(
        canvas_reads(vec![event(MouseWheelUnit::Line, Vec2::new(0.0, 1.0), Modifiers { alt: true, shift: true, ..Modifiers::NONE })], false, false),
        Some(Wheel::SnapZoom(1))
    );
}

/// Line and point deltas that cancel out at the canvas notch pan nothing, even where they would
/// not cancel at egui's notch (Alt with Zoom with Scroll Wheel pans).
#[test]
fn a_pan_that_cancels_at_the_canvas_notch_stays_still() {
    let events = vec![event(MouseWheelUnit::Line, Vec2::new(0.0, 1.0), Modifiers::ALT), event(MouseWheelUnit::Point, Vec2::new(0.0, -16.0), Modifiers::ALT)];
    assert_eq!(canvas_reads(events, true, false), None);
}

/// A sideways Ctrl pan in the same frame as plain zoom deltas that cancel at egui's notch (Zoom
/// with Scroll Wheel, Animated Zoom) stays sideways: the zoom's deltas never reach the pan.
#[test]
fn zoom_deltas_stay_out_of_a_sideways_pan() {
    let ctrl = Modifiers { ctrl: true, command: true, ..Modifiers::NONE };
    let events = vec![
        event(MouseWheelUnit::Line, Vec2::new(0.0, -1.0), ctrl),
        event(MouseWheelUnit::Line, Vec2::new(0.0, 1.0), ctrl),
        event(MouseWheelUnit::Point, Vec2::new(0.0, -40.0), ctrl),
        event(MouseWheelUnit::Line, Vec2::new(0.0, -1.0), ctrl),
    ];
    let p = pan(canvas_reads(events, true, true));
    assert_eq!(p.y, 0.0, "{p:?}");
}
