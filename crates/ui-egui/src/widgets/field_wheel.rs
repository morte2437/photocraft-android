//! The mouse wheel on value fields (#5473, #4061): one step a notch, as the Up/Down keys (×10
//! with Shift).
//!
//! - While a field has keyboard focus, the wheel steps it wherever the pointer is. [`begin_frame`]
//!   takes the wheel out of the frame's input before anything is laid out, so neither the canvas
//!   nor a panel scrolls or zooms with it. ⌥/Alt and ⌘/Ctrl + wheel stay the view's.
//! - Over a field without focus, the wheel steps it too, but only a gesture that starts over it:
//!   a panel being scrolled keeps scrolling when a field passes under the pointer. A gesture that
//!   leaves the field is the view's from then on.

use egui::{Context, Event, Id, MouseWheelUnit, Rect, Ui, Vec2};

/// Wheel events this close together (seconds) are one gesture.
const GESTURE_GAP: f64 = 0.3;

/// Who a wheel gesture belongs to.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Owner {
    /// It started this frame and no field has taken it yet.
    New,
    Field(Id),
    /// It started where no field took it (a panel, the canvas).
    Elsewhere,
}

#[derive(Clone, Copy, Debug)]
struct Gesture {
    last: f64,
    owner: Owner,
}

fn gesture_id() -> Id {
    Id::unique("pc-field-wheel-gesture")
}

fn focused_id() -> Id {
    Id::unique("pc-field-wheel-focused")
}

fn taken_id() -> Id {
    Id::unique("pc-field-wheel-taken")
}

/// A wheel event a field takes: plain or Shift, not the view's Alt or Ctrl/Cmd zoom, nor a plain
/// sideways scroll (a trackpad swipe pans the view; Shift+wheel can arrive sideways). An event
/// with no delta at all counts: macOS starts a trackpad gesture with one, and it is what lets a
/// field under the pointer claim the gesture.
fn for_field(e: &Event) -> bool {
    matches!(e, Event::MouseWheel { delta, modifiers, .. }
        if (delta.y != 0.0 || delta.x == 0.0 || modifiers.shift) && !modifiers.alt && !modifiers.ctrl && !modifiers.command && !modifiers.mac_cmd)
}

/// Before this frame's UI is laid out: hand the wheel to the focused value field, if any, and
/// follow which gesture the wheel is in.
pub fn begin_frame(ctx: &Context) {
    ctx.data_mut(|d| d.remove::<Vec<Event>>(taken_id()));
    // (the field's widget id, its slot id): focus is the widget's, a gesture the slot's.
    let focused = ctx.data(|d| d.get_temp::<(Id, Id)>(focused_id())).filter(|(id, _)| ctx.memory(|m| m.has_focus(*id)));
    let (now, wheel) = ctx.input(|i| (i.time, i.events.iter().any(|e| matches!(e, Event::MouseWheel { .. }))));
    if let Some((_, field)) = focused {
        let (taken, view_wheel): (Vec<Event>, bool) = ctx.input_mut(|i| {
            let (taken, rest) = std::mem::take(&mut i.events).into_iter().partition(for_field);
            i.events = rest;
            (taken, i.events.iter().any(|e| matches!(e, Event::MouseWheel { .. })))
        });
        if wheel {
            // The latest wheel is the view's when any of it was left to the view (Alt, Ctrl/Cmd).
            let owner = if view_wheel { Owner::Elsewhere } else { Owner::Field(field) };
            ctx.data_mut(|d| d.insert_temp(gesture_id(), Gesture { last: now, owner }));
        }
        if !taken.is_empty() {
            ctx.data_mut(|d| d.insert_temp(taken_id(), taken));
        }
        // egui spreads each notch over a few frames: the tail of the field's own notches scrolls
        // nothing, while the view's keeps all of its own.
        if ctx.data(|d| d.get_temp::<Gesture>(gesture_id())).is_some_and(|g| g.owner == Owner::Field(field)) {
            ctx.input_mut(|i| i.smooth_scroll_delta = Vec2::ZERO);
        }
        return;
    }
    if !wheel {
        return;
    }
    let gesture = ctx.data(|d| d.get_temp::<Gesture>(gesture_id()));
    let owner = match gesture {
        Some(g) if now - g.last <= GESTURE_GAP => match g.owner {
            // No field took it when it started.
            Owner::New => Owner::Elsewhere,
            owner => owner,
        },
        _ => Owner::New,
    };
    ctx.data_mut(|d| d.insert_temp(gesture_id(), Gesture { last: now, owner }));
}

/// Record that value field `id` (whose slot is `field`, as given to [`notches`]) has keyboard
/// focus this frame, so the wheel is its next frame.
pub fn set_focused(ctx: &Context, id: Id, field: Id) {
    ctx.data_mut(|d| d.insert_temp(focused_id(), (id, field)));
}

/// Wheel notches for the value field `field` in `rect` this frame (up positive, Shift ×10):
/// all of the wheel while it is `focused`, else a gesture that started with the pointer over it.
pub fn notches(ui: &Ui, field: Id, focused: bool, rect: Rect) -> f32 {
    let events: Vec<Event> = if focused {
        ui.data_mut(|d| d.remove_temp::<Vec<Event>>(taken_id())).unwrap_or_default()
    } else {
        if !ui.is_enabled() {
            return 0.0;
        }
        let Some(mut g) = ui.data(|d| d.get_temp::<Gesture>(gesture_id())) else { return 0.0 };
        let (events, wheel): (Vec<Event>, bool) =
            ui.input(|i| (i.events.iter().filter(|e| for_field(e)).cloned().collect(), i.events.iter().any(|e| matches!(e, Event::MouseWheel { .. }))));
        // Wheel the field doesn't take (a sideways swipe after a zero-delta start), or any wheel
        // with the pointer off the field, hands the rest of the gesture to the view: coming back
        // over the field in the same gesture doesn't step it.
        if g.owner == Owner::Field(field) && wheel && (events.is_empty() || !ui.rect_contains_pointer(rect)) {
            g.owner = Owner::Elsewhere;
            ui.data_mut(|d| d.insert_temp(gesture_id(), g));
            return 0.0;
        }
        if !ui.rect_contains_pointer(rect) {
            return 0.0;
        }
        match g.owner {
            Owner::New if !events.is_empty() => {
                g.owner = Owner::Field(field);
                ui.data_mut(|d| d.insert_temp(gesture_id(), g));
            }
            Owner::Field(id) if id == field => {}
            _ => return 0.0,
        }
        // Nothing under the field scrolls with the wheel that steps it, smoothed tail included.
        ui.ctx().input_mut(|i| i.smooth_scroll_delta = Vec2::ZERO);
        events
    };
    let wheel: Vec<(MouseWheelUnit, Vec2, bool)> = events
        .iter()
        .filter_map(|e| match e {
            Event::MouseWheel { unit, delta, modifiers, .. } if for_field(e) => Some((*unit, *delta, modifiers.shift)),
            _ => None,
        })
        .collect();
    crate::widgets::count_notches(ui, field.with("wheel-acc"), &wheel, 10.0)
}

#[cfg(test)]
mod tests {
    use egui::accesskit::Role;
    use egui::{Modifiers, Pos2, vec2};
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;

    /// The value, the scroll area's vertical offset, whether the field is enabled, and the
    /// horizontal offset.
    type State = (f32, f32, bool, f32);

    /// A value field (0–1000) at the top left of a scroll area 100 pt tall, beside and above a
    /// large filler.
    fn harness(value: f32) -> Harness<'static, State> {
        harness_stepping(value, 0.25)
    }

    /// [`harness`] with each step `step_dt` seconds long.
    fn harness_stepping(value: f32, step_dt: f32) -> Harness<'static, State> {
        let mut h = Harness::builder().with_size(vec2(300.0, 200.0)).with_step_dt(step_dt).build_ui_state(
            |ui, s: &mut State| {
                super::begin_frame(ui.ctx());
                let out = egui::ScrollArea::both().max_height(100.0).max_width(200.0).show(ui, |ui| {
                    ui.add_enabled_ui(s.2, |ui| crate::widgets::value_field(ui, &mut s.0, 0.0..=1000.0, "px", 90.0).accessible_name("Size"));
                    ui.allocate_space(vec2(1000.0, 1000.0));
                });
                let offset = out.state.clamped_offset();
                s.1 = offset.y;
                s.3 = offset.x;
            },
            (value, 0.0, true, 0.0),
        );
        h.run();
        h
    }

    fn field(h: &Harness<'static, State>) -> egui::Rect {
        h.get_by_role(Role::SpinButton).rect()
    }

    /// `lines` wheel lines (up positive) at `at`, then let the gesture end.
    fn wheel(h: &mut Harness<'static, State>, at: Pos2, lines: f32, modifiers: Modifiers) {
        h.hover_at(at);
        h.run_steps(1);
        h.event(egui::Event::MouseWheel {
            source: egui::MouseWheelSource::Unknown,
            unit: egui::MouseWheelUnit::Line,
            delta: vec2(0.0, lines),
            phase: egui::TouchPhase::Move,
            modifiers,
        });
        h.run_steps(4);
    }

    #[test]
    fn the_wheel_over_a_field_steps_it_and_scrolls_nothing() {
        let mut h = harness(100.0);
        let at = field(&h).center();
        wheel(&mut h, at, 1.0, Modifiers::NONE);
        assert_eq!(h.state().0, 101.0);
        wheel(&mut h, at, -3.0, Modifiers::NONE);
        assert_eq!(h.state().0, 98.0);
        wheel(&mut h, at, 1.0, Modifiers::SHIFT);
        assert_eq!(h.state().0, 108.0, "Shift: ten a notch");
        assert_eq!(h.state().1, 0.0, "the panel under the field didn't scroll");
    }

    #[test]
    fn a_focused_field_takes_the_wheel_wherever_the_pointer_is() {
        let mut h = harness(100.0);
        h.get_by_role(Role::SpinButton).click();
        h.run();
        let away = field(&h).center() + vec2(0.0, 60.0);
        wheel(&mut h, away, -2.0, Modifiers::NONE);
        assert_eq!(h.state().0, 98.0);
        assert_eq!(h.state().1, 0.0, "the panel under the pointer didn't scroll");
        assert!(h.get_by_role(Role::SpinButton).is_focused(), "the field keeps focus");
        // Alt + wheel stays the view's (zoom), not the field's.
        wheel(&mut h, away, 1.0, Modifiers::ALT);
        assert_eq!(h.state().0, 98.0);
    }

    #[test]
    fn a_gesture_that_starts_over_the_panel_keeps_scrolling_it_over_a_field() {
        let mut h = harness(100.0);
        let below = field(&h).center() + vec2(0.0, 40.0);
        h.hover_at(below);
        h.run_steps(1);
        let down = || egui::Event::MouseWheel {
            source: egui::MouseWheelSource::Unknown,
            unit: egui::MouseWheelUnit::Line,
            delta: vec2(0.0, -1.0),
            phase: egui::TouchPhase::Move,
            modifiers: Modifiers::NONE,
        };
        h.event(down());
        h.run_steps(1);
        // Still the same gesture as the pointer reaches a field.
        h.hover_at(field(&h).center());
        h.event(down());
        h.run_steps(1);
        assert_eq!(h.state().0, 100.0, "the field passing under the pointer is not stepped");
        assert!(h.state().1 > 0.0, "the panel scrolled");
    }

    /// A plain sideways scroll (a trackpad swipe) is the view's, focused or hovered; Shift+wheel
    /// that the system turned sideways still steps by ten.
    #[test]
    fn a_sideways_scroll_steps_a_field_only_with_shift() {
        for focus in [false, true] {
            for (m, want) in [(Modifiers::NONE, 100.0), (Modifiers::SHIFT, 110.0)] {
                let mut h = harness(100.0);
                if focus {
                    h.get_by_role(Role::SpinButton).click();
                    h.run();
                }
                let at = field(&h).center();
                h.hover_at(at);
                h.run_steps(1);
                h.event(egui::Event::MouseWheel {
                    source: egui::MouseWheelSource::Unknown,
                    unit: egui::MouseWheelUnit::Line,
                    delta: vec2(1.0, 0.0),
                    phase: egui::TouchPhase::Move,
                    modifiers: m,
                });
                h.run_steps(4);
                assert_eq!(h.state().0, want, "focus {focus}, {m:?}");
            }
        }
    }

    /// A trackpad gesture that macOS starts with a zero-delta event, then moves a frame later,
    /// steps the field under the pointer.
    #[test]
    fn a_trackpad_gesture_that_starts_with_no_delta_steps_a_hovered_field() {
        let mut h = harness(100.0);
        h.hover_at(field(&h).center());
        h.run_steps(1);
        let ev = |phase, y| egui::Event::MouseWheel {
            source: egui::MouseWheelSource::Unknown,
            unit: egui::MouseWheelUnit::Line,
            delta: vec2(0.0, y),
            phase,
            modifiers: Modifiers::NONE,
        };
        h.event(ev(egui::TouchPhase::Start, 0.0));
        h.run_steps(1);
        h.event(ev(egui::TouchPhase::Move, 2.0));
        h.run_steps(4);
        assert_eq!(h.state().0, 102.0);
        assert_eq!(h.state().1, 0.0, "the panel didn't scroll");
    }

    /// A trackpad gesture that starts with a zero-delta event over a field, then swipes sideways,
    /// pans the view and leaves the field alone.
    #[test]
    fn a_sideways_swipe_after_a_zero_delta_start_pans_the_view() {
        let mut h = harness(100.0);
        h.hover_at(field(&h).center());
        h.run_steps(1);
        let ev = |phase, x| egui::Event::MouseWheel {
            source: egui::MouseWheelSource::Unknown,
            unit: egui::MouseWheelUnit::Line,
            delta: vec2(x, 0.0),
            phase,
            modifiers: Modifiers::NONE,
        };
        h.event(ev(egui::TouchPhase::Start, 0.0));
        h.run_steps(1);
        h.event(ev(egui::TouchPhase::Move, -2.0));
        h.run_steps(4);
        assert_eq!(h.state().0, 100.0, "the field was not stepped");
        assert!(h.state().3 > 0.0, "the panel panned sideways");
    }

    /// The same swipe with the pointer off the field hands the gesture to the view too: coming
    /// back over the field in the same gesture doesn't step it.
    #[test]
    fn a_sideways_swipe_off_the_field_keeps_the_rest_of_the_gesture_the_views() {
        // Short frames: moving the pointer takes a frame of its own, and the gesture must not end.
        let mut h = harness_stepping(100.0, 1.0 / 30.0);
        h.hover_at(field(&h).center());
        h.run_steps(1);
        let ev = |phase, unit, d| egui::Event::MouseWheel { source: egui::MouseWheelSource::Unknown, unit, delta: d, phase, modifiers: Modifiers::NONE };
        h.event(ev(egui::TouchPhase::Start, egui::MouseWheelUnit::Point, vec2(0.0, 0.0)));
        h.run_steps(1);
        // A short swipe, so the field stays in view.
        h.hover_at(field(&h).center() + vec2(0.0, 40.0));
        h.event(ev(egui::TouchPhase::Move, egui::MouseWheelUnit::Point, vec2(-10.0, 0.0)));
        h.run_steps(1);
        h.hover_at(field(&h).center());
        h.event(ev(egui::TouchPhase::Move, egui::MouseWheelUnit::Line, vec2(0.0, -1.0)));
        h.run_steps(4);
        assert_eq!(h.state().0, 100.0, "the field was not stepped");
        assert!(h.state().1 > 0.0, "the panel scrolled");
    }

    /// A gesture that steps a field, then wheels on with the pointer off it, is the view's from
    /// then on: coming back over the field in the same gesture doesn't step it again.
    #[test]
    fn a_gesture_that_wheels_off_the_field_does_not_step_it_on_coming_back() {
        // Short frames: moving the pointer takes a frame of its own, and the gesture must not end.
        let mut h = harness_stepping(100.0, 1.0 / 30.0);
        h.hover_at(field(&h).center());
        h.run_steps(1);
        // Up, so the panel (already at the top) doesn't move the field.
        let up = || egui::Event::MouseWheel {
            source: egui::MouseWheelSource::Unknown,
            unit: egui::MouseWheelUnit::Line,
            delta: vec2(0.0, 1.0),
            phase: egui::TouchPhase::Move,
            modifiers: Modifiers::NONE,
        };
        h.event(up());
        h.run_steps(1);
        assert_eq!(h.state().0, 101.0);
        h.hover_at(field(&h).center() + vec2(0.0, 40.0));
        h.event(up());
        h.run_steps(1);
        h.hover_at(field(&h).center());
        h.event(up());
        h.run_steps(4);
        assert_eq!(h.state().0, 101.0, "the field was stepped once, by the start of the gesture");
    }

    #[test]
    fn a_disabled_field_ignores_the_wheel() {
        let mut h = harness(100.0);
        h.state_mut().2 = false;
        h.run();
        let at = field(&h).center();
        wheel(&mut h, at, 1.0, Modifiers::NONE);
        assert_eq!(h.state().0, 100.0);
    }

    #[test]
    fn wild_wheel_deltas_leave_a_finite_value_in_range() {
        // egui itself asserts on a non-finite wheel event in debug builds: `count_notches` is checked
        // with those below.
        for lines in [1e30, -1e30, 1e6, -1e6] {
            for focus in [false, true] {
                let mut h = harness(100.0);
                if focus {
                    h.get_by_role(Role::SpinButton).click();
                    h.run();
                }
                let at = field(&h).center();
                wheel(&mut h, at, lines, Modifiers::SHIFT);
                let v = h.state().0;
                assert!(v.is_finite() && (0.0..=1000.0).contains(&v), "{lines} (focus {focus}): {v}");
            }
        }
    }

    #[test]
    fn notches_of_nan_or_infinite_deltas_are_none() {
        use egui::MouseWheelUnit::{Line, Point};
        let ctx = egui::Context::default();
        let mut out = ctx.run_ui(Default::default(), |ui| {
            let id = egui::Id::unique("acc");
            for d in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
                for unit in [Line, Point] {
                    assert_eq!(crate::widgets::count_notches(ui, id, &[(unit, vec2(0.0, d), false)], 10.0), 0.0, "{d} {unit:?}");
                    // Nothing poisoned is carried over to the next notch.
                    assert_eq!(crate::widgets::count_notches(ui, id, &[(Line, vec2(0.0, 1.0), true)], 10.0), 10.0);
                }
            }
        });
        out.textures_delta.clear();
    }
}
