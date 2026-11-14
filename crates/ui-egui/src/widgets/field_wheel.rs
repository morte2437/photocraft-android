//! The mouse wheel on value fields (#5473, #4061): one step a notch, as the Up/Down keys (×10
//! with Shift).
//!
//! - While a field has keyboard focus, the wheel steps it wherever the pointer is. [`begin_frame`]
//!   takes the wheel out of the frame's input before anything is laid out, so neither the canvas
//!   nor a panel scrolls or zooms with it. ⌥/Alt and ⌘/Ctrl + wheel stay the view's.
//! - Over a field without focus, the wheel steps it too, but only a gesture that starts over it:
//!   a panel being scrolled keeps scrolling when a field passes under the pointer. A gesture that
//!   leaves the field is the view's from then on.
//! - Dropdowns, sliders and the font list that the wheel steps follow the same rule
//!   ([`hover_notches`]).
//! - A trackpad (macOS says when it is one; other systems don't) steps only a field with focus:
//!   over anything else its gesture scrolls the panel, wherever it starts (#11353).
//! - Each window keeps its own record (a panel floated into a window of its own has input of its
//!   own), and its own fractions of a notch; [`FieldWheel`] starts every window's pass with
//!   [`begin_frame`]. The app installs it; a widget drawn without the app's setup installs it the
//!   first time it asks for the wheel, so it steps there as it always has.

use egui::{Context, Event, Id, MouseWheelSource, MouseWheelUnit, Rect, Ui, Vec2};

/// Wheel events this close together (seconds) are one gesture.
pub(crate) const GESTURE_GAP: f64 = 0.3;

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

// Each window (egui viewport) has its own input, so its own records: a panel floated into a
// window of its own follows the gestures made there, not the main window's.

fn gesture_id(ctx: &Context) -> Id {
    Id::unique("pc-field-wheel-gesture").with(ctx.viewport_id())
}

fn focused_id(ctx: &Context) -> Id {
    Id::unique("pc-field-wheel-focused").with(ctx.viewport_id())
}

fn taken_id(ctx: &Context) -> Id {
    Id::unique("pc-field-wheel-taken").with(ctx.viewport_id())
}

fn pass_id(ctx: &Context) -> Id {
    Id::unique("pc-field-wheel-pass").with(ctx.viewport_id())
}

/// A wheel event a field takes: plain or Shift, not the view's Alt or Ctrl/Cmd zoom, nor a plain
/// sideways scroll (a trackpad swipe pans the view; Shift+wheel can arrive sideways). An event
/// with no delta at all counts: a gesture can start with one, and it is what lets a field under
/// the pointer claim the gesture.
fn for_field(e: &Event) -> bool {
    matches!(e, Event::MouseWheel { delta, modifiers, .. }
        if (delta.y != 0.0 || delta.x == 0.0 || modifiers.shift) && !modifiers.alt && !modifiers.ctrl && !modifiers.command && !modifiers.mac_cmd)
}

/// Fingers on a trackpad or the glide they leave, where the system says so (macOS): they scroll
/// what is under the pointer, and step only a field with focus (#11353). A system that doesn't
/// say counts as a wheel.
fn from_fingers(e: &Event) -> bool {
    matches!(e, Event::MouseWheel { source: MouseWheelSource::Trackpad | MouseWheelSource::Momentum, .. })
}

/// Before this frame's UI is laid out: hand the wheel to the focused value field, if any, and
/// follow which gesture the wheel is in. The app runs it through [`FieldWheel`]. Once a pass:
/// a second call in the same pass does nothing, since running again would drop the wheel the
/// first kept for a focused field and count a new gesture as one that no field took.
pub fn begin_frame(ctx: &Context) {
    // (outside the closures below: the viewport id is read under the context's lock)
    let (gesture_key, taken_key, pass_key, pass) = (gesture_id(ctx), taken_id(ctx), pass_id(ctx), ctx.cumulative_pass_nr());
    if ctx.data(|d| d.get_temp::<u64>(pass_key)) == Some(pass) {
        return;
    }
    ctx.data_mut(|d| d.insert_temp(pass_key, pass));
    ctx.data_mut(|d| d.remove::<Vec<Event>>(taken_key));
    // (the field's widget id, its slot id): focus is the widget's, a gesture the slot's.
    let focused_key = focused_id(ctx);
    let focused = ctx.data(|d| d.get_temp::<(Id, Id)>(focused_key)).filter(|(id, _)| ctx.memory(|m| m.has_focus(*id)));
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
            ctx.data_mut(|d| d.insert_temp(gesture_key, Gesture { last: now, owner }));
        }
        if !taken.is_empty() {
            ctx.data_mut(|d| d.insert_temp(taken_key, taken));
        }
        // egui spreads each notch over a few frames: the tail of the field's own notches scrolls
        // nothing, while the view's keeps all of its own.
        if ctx.data(|d| d.get_temp::<Gesture>(gesture_key)).is_some_and(|g| g.owner == Owner::Field(field)) {
            ctx.input_mut(|i| i.smooth_scroll_delta = Vec2::ZERO);
        }
        return;
    }
    if !wheel {
        return;
    }
    let gesture = ctx.data(|d| d.get_temp::<Gesture>(gesture_key));
    let owner = match gesture {
        Some(g) if now - g.last <= GESTURE_GAP => match g.owner {
            // No field took it when it started.
            Owner::New => Owner::Elsewhere,
            owner => owner,
        },
        _ => Owner::New,
    };
    ctx.data_mut(|d| d.insert_temp(gesture_key, Gesture { last: now, owner }));
}

/// Runs [`begin_frame`] at the start of every pass of every window (egui viewport), the main
/// window's and each floated panel's, before anything in it is laid out.
pub(crate) struct FieldWheel;

impl egui::plugin::Plugin for FieldWheel {
    fn debug_name(&self) -> &'static str {
        "PhotoCraft field wheel"
    }

    fn on_begin_pass(&mut self, ui: &mut Ui) {
        begin_frame(ui.ctx());
    }
}

/// Install [`FieldWheel`] where it isn't yet: a widget drawn without the app's setup (another
/// app, a test) steps with the wheel as it did before the wheel followed gestures. Its hook
/// starts the next pass; this one, already under way, gets [`begin_frame`] now.
fn install(ctx: &Context) {
    if ctx.plugin_opt::<FieldWheel>().is_none() {
        ctx.add_plugin(FieldWheel);
        begin_frame(ctx);
    }
}

/// Whether a wheel gesture stepping a value field is still running: its last notch came less than
/// [`GESTURE_GAP`] ago. A field that applies each value as a new edit holds the wheeled value until
/// this turns false, so a burst of notches is one edit (#9898).
pub fn field_gesture_running(ctx: &Context) -> bool {
    // This window's gesture; the key is read outside the closure, as in `begin_frame`.
    let (now, gesture_key) = (ctx.input(|i| i.time), gesture_id(ctx));
    ctx.data(|d| d.get_temp::<Gesture>(gesture_key)).is_some_and(|g| matches!(g.owner, Owner::Field(_)) && now - g.last <= GESTURE_GAP)
}

/// Whether [`begin_frame`] handed wheel notches to the focused value field this frame: they step
/// it as the arrow keys do, though the frame's scroll delta no longer shows them.
pub(crate) fn focused_field_wheeled(ctx: &Context) -> bool {
    let taken_key = taken_id(ctx);
    ctx.data(|d| d.get_temp::<Vec<Event>>(taken_key)).is_some_and(|e| !e.is_empty())
}

/// Record that value field `id` (whose slot is `field`, as given to [`notches`]) has keyboard
/// focus this frame, so the wheel is its next frame.
pub fn set_focused(ctx: &Context, id: Id, field: Id) {
    let key = focused_id(ctx);
    ctx.data_mut(|d| d.insert_temp(key, (id, field)));
}

/// Wheel notches for the value field `field` in `rect` this frame (up positive, Shift ×10):
/// all of the wheel while it is `focused`, else a gesture that started with the pointer over it.
pub fn notches(ui: &Ui, field: Id, focused: bool, rect: Rect) -> f32 {
    install(ui.ctx());
    let events: Vec<Event> = if focused {
        let taken_key = taken_id(ui.ctx());
        ui.data_mut(|d| d.remove_temp::<Vec<Event>>(taken_key)).unwrap_or_default()
    } else {
        hovered_events(ui, field, rect, for_field)
    };
    crate::widgets::count_notches(ui, field.with("wheel-acc"), &wheel_of(&events), 10.0)
}

/// Wheel notches for another control the wheel steps while the pointer is over it (a dropdown
/// and its open list, a slider, the font field), `id` naming it and `rect` its area, a notch with
/// Shift counting `shift_notches`: the same rule as a value field without focus, so a panel being
/// scrolled keeps scrolling when the control passes under the pointer (#11353). Ask every frame
/// the control is shown, the pointer over it or not, as [`notches`] is: a gesture that wheels on
/// off the control is the view's from then on. Unlike a value field, it takes the wheel with any
/// modifier (Photoshop's dialog sliders step one a notch with Ctrl or Alt).
pub fn hover_notches(ui: &Ui, id: Id, rect: Rect, shift_notches: f32) -> f32 {
    install(ui.ctx());
    let events = hovered_events(ui, id, rect, |e| matches!(e, Event::MouseWheel { .. }));
    crate::widgets::count_notches(ui, id.with("wheel-acc"), &wheel_of(&events), shift_notches)
}

/// This frame's wheel events that `takes` for control `field` in `rect` without keyboard focus:
/// those of a gesture that started with the pointer over it, none otherwise, and never a
/// trackpad's.
fn hovered_events(ui: &Ui, field: Id, rect: Rect, takes: fn(&Event) -> bool) -> Vec<Event> {
    if !ui.is_enabled() {
        return Vec::new();
    }
    let gesture_key = gesture_id(ui.ctx());
    let Some(mut g) = ui.data(|d| d.get_temp::<Gesture>(gesture_key)) else { return Vec::new() };
    let (events, wheel): (Vec<Event>, bool) = ui.input(|i| {
        (i.events.iter().filter(|e| takes(e) && !from_fingers(e)).cloned().collect(), i.events.iter().any(|e| matches!(e, Event::MouseWheel { .. })))
    });
    // Wheel the field doesn't take (a sideways swipe after a zero-delta start), or any wheel
    // with the pointer off the field, hands the rest of the gesture to the view: coming back
    // over the field in the same gesture doesn't step it.
    if g.owner == Owner::Field(field) && wheel && (events.is_empty() || !ui.rect_contains_pointer(rect)) {
        g.owner = Owner::Elsewhere;
        ui.data_mut(|d| d.insert_temp(gesture_key, g));
        return Vec::new();
    }
    if !ui.rect_contains_pointer(rect) {
        return Vec::new();
    }
    match g.owner {
        Owner::New if !events.is_empty() => {
            g.owner = Owner::Field(field);
            ui.data_mut(|d| d.insert_temp(gesture_key, g));
        }
        Owner::Field(id) if id == field => {}
        _ => return Vec::new(),
    }
    // Nothing under the field scrolls with the wheel that steps it, smoothed tail included.
    ui.ctx().input_mut(|i| i.smooth_scroll_delta = Vec2::ZERO);
    events
}

/// The wheel in `events`: (unit, delta, Shift held).
fn wheel_of(events: &[Event]) -> Vec<(MouseWheelUnit, Vec2, bool)> {
    events
        .iter()
        .filter_map(|e| match e {
            Event::MouseWheel { unit, delta, modifiers, .. } => Some((*unit, *delta, modifiers.shift)),
            _ => None,
        })
        .collect()
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

    /// A wheel dropdown (0–3) and a wheel slider row (0–100) below a gap, at the top of a scroll
    /// area 200 pt tall over a large filler: (the dropdown's index, the slider's value, the
    /// scroll area's vertical offset).
    fn controls_harness() -> Harness<'static, (usize, f32, f32)> {
        const MODES: [(usize, &str); 4] = [(0, "Normal"), (1, "Dissolve"), (2, "Darken"), (3, "Multiply")];
        let mut h = Harness::builder().with_size(vec2(300.0, 300.0)).with_step_dt(1.0 / 30.0).build_ui_state(
            |ui, s: &mut (usize, f32, f32)| {
                super::begin_frame(ui.ctx());
                let out = egui::ScrollArea::vertical().max_height(200.0).show(ui, |ui| {
                    ui.add_space(100.0);
                    crate::widgets::dropdown_wheel_hovered(ui, "blend-mode", "Blend Mode", &mut s.0, &MODES, 120.0);
                    let g = crate::widgets::RowGestures { wheel_step: Some(1.0), ..Default::default() };
                    crate::widgets::slider_row_with(ui, "Amount", &mut s.1, 0.0..=100.0, "", None, g);
                    ui.allocate_space(vec2(200.0, 1000.0));
                });
                s.2 = out.state.clamped_offset().y;
            },
            (1, 50.0, 0.0),
        );
        h.run();
        h
    }

    /// #11353: the same for a dropdown and a slider the wheel steps (a trackpad scrolling the
    /// Properties or Character panel past them).
    #[test]
    fn a_gesture_that_starts_over_the_panel_keeps_scrolling_it_over_a_dropdown_or_slider() {
        for role in [Role::ComboBox, Role::Slider] {
            let mut h = controls_harness();
            let below = h.get_by_role(Role::Slider).rect().center() + vec2(0.0, 40.0);
            h.hover_at(below);
            h.run_steps(1);
            let down = || egui::Event::MouseWheel {
                source: egui::MouseWheelSource::Unknown,
                unit: egui::MouseWheelUnit::Point,
                delta: vec2(0.0, -12.0),
                phase: egui::TouchPhase::Move,
                modifiers: Modifiers::NONE,
            };
            h.event(down());
            h.run_steps(1);
            // Still the same gesture as the pointer reaches the control, a few notches' worth.
            for _ in 0..6 {
                let over = h.get_by_role(role).rect();
                assert!(over.top() > 0.0, "{role:?} still in view");
                h.hover_at(over.center());
                h.event(down());
                h.run_steps(1);
            }
            assert_eq!((h.state().0, h.state().1), (1, 50.0), "{role:?} passing under the pointer is not stepped");
            assert!(h.state().2 > 12.0, "{role:?}: the panel kept scrolling");
        }
    }

    /// A gesture that starts over a dropdown or slider still steps it, and the panel stays put.
    #[test]
    fn a_gesture_that_starts_over_a_dropdown_or_slider_steps_it() {
        for role in [Role::ComboBox, Role::Slider] {
            let mut h = controls_harness();
            h.hover_at(h.get_by_role(role).rect().center());
            h.run_steps(1);
            h.event(egui::Event::MouseWheel {
                source: egui::MouseWheelSource::Unknown,
                unit: egui::MouseWheelUnit::Line,
                delta: vec2(0.0, -1.0),
                phase: egui::TouchPhase::Move,
                modifiers: Modifiers::NONE,
            });
            h.run_steps(4);
            let want = if role == Role::ComboBox { (2, 50.0) } else { (1, 49.0) };
            assert_eq!((h.state().0, h.state().1), want, "{role:?}");
            assert_eq!(h.state().2, 0.0, "{role:?}: the panel didn't scroll");
        }
    }

    /// A slider drawn without the app's setup, neither [`super::FieldWheel`] nor
    /// [`super::begin_frame`], still steps with the wheel over it, as it did before the wheel
    /// followed gestures: the widget installs what it needs the first time it asks.
    #[test]
    fn a_slider_drawn_without_the_apps_setup_steps_with_the_wheel() {
        let mut h = Harness::builder().with_size(vec2(300.0, 100.0)).build_ui_state(
            |ui, v: &mut f32| {
                let g = crate::widgets::RowGestures { wheel_step: Some(1.0), ..Default::default() };
                crate::widgets::slider_row_with(ui, "Amount", v, 0.0..=100.0, "", None, g);
            },
            50.0,
        );
        h.run();
        h.hover_at(h.get_by_role(Role::Slider).rect().center());
        h.run_steps(1);
        h.event(egui::Event::MouseWheel {
            source: egui::MouseWheelSource::Unknown,
            unit: egui::MouseWheelUnit::Line,
            delta: vec2(0.0, -1.0),
            phase: egui::TouchPhase::Move,
            modifiers: Modifiers::NONE,
        });
        h.run_steps(4);
        assert_eq!(*h.state(), 49.0);
    }

    /// #11353: fingers on a trackpad, and the glide they leave, scroll the panel even when the
    /// gesture starts over a control without focus (macOS says which input it is). A physical
    /// wheel still steps it.
    #[test]
    fn a_trackpad_gesture_that_starts_over_a_control_scrolls_the_panel() {
        use egui::{MouseWheelSource as Source, TouchPhase as Phase};
        let ev =
            |source, phase, y| egui::Event::MouseWheel { source, unit: egui::MouseWheelUnit::Point, delta: vec2(0.0, y), phase, modifiers: Modifiers::NONE };
        for role in [Role::ComboBox, Role::Slider] {
            let mut h = controls_harness();
            h.hover_at(h.get_by_role(role).rect().center());
            h.run_steps(1);
            // As macOS sends it: a start with no movement, then the fingers, then the glide.
            h.event(ev(Source::Trackpad, Phase::Start, 0.0));
            h.run_steps(1);
            for _ in 0..4 {
                h.event(ev(Source::Trackpad, Phase::Move, -12.0));
                h.run_steps(1);
            }
            h.event(ev(Source::Momentum, Phase::Move, -12.0));
            h.run_steps(4);
            assert_eq!((h.state().0, h.state().1), (1, 50.0), "{role:?}: no step");
            assert!(h.state().2 > 0.0, "{role:?}: the panel scrolled");
        }
        let mut h = harness(100.0);
        h.hover_at(field(&h).center());
        h.run_steps(1);
        h.event(ev(Source::Trackpad, Phase::Start, 0.0));
        h.run_steps(1);
        for _ in 0..4 {
            h.event(ev(Source::Trackpad, Phase::Move, -12.0));
            h.run_steps(1);
        }
        h.run_steps(4);
        assert_eq!(h.state().0, 100.0, "the value field isn't stepped");
        assert!(h.state().1 > 0.0, "the panel scrolled");

        let mut h = controls_harness();
        h.hover_at(h.get_by_role(Role::Slider).rect().center());
        h.run_steps(1);
        h.event(egui::Event::MouseWheel {
            source: Source::Wheel,
            unit: egui::MouseWheelUnit::Line,
            delta: vec2(0.0, -1.0),
            phase: Phase::Move,
            modifiers: Modifiers::NONE,
        });
        h.run_steps(4);
        assert_eq!(h.state().1, 49.0, "a physical wheel steps it");
    }

    /// A field with keyboard focus keeps a trackpad's wheel, as it keeps a mouse's.
    #[test]
    fn a_focused_field_takes_a_trackpad_gesture() {
        let mut h = harness(100.0);
        h.get_by_role(Role::SpinButton).click();
        h.run_steps(2);
        let at = field(&h).center();
        h.hover_at(at);
        h.run_steps(1);
        h.event(egui::Event::MouseWheel {
            source: egui::MouseWheelSource::Trackpad,
            unit: egui::MouseWheelUnit::Line,
            delta: vec2(0.0, 2.0),
            phase: egui::TouchPhase::Move,
            modifiers: Modifiers::NONE,
        });
        h.run_steps(4);
        assert_eq!(h.state().0, 102.0);
    }

    /// A gesture that steps a dropdown or slider, then wheels on with the pointer off it, is the
    /// panel's from then on: coming back over the control in the same gesture doesn't step it
    /// again, as for a value field.
    #[test]
    fn a_gesture_that_wheels_off_a_dropdown_or_slider_does_not_step_it_on_coming_back() {
        for role in [Role::ComboBox, Role::Slider] {
            let mut h = controls_harness();
            // Room to step up from.
            h.state_mut().0 = 3;
            h.run();
            let over = h.get_by_role(role).rect().center();
            h.hover_at(over);
            h.run_steps(1);
            // Up, so the panel (already at the top) doesn't move the controls.
            let up = || egui::Event::MouseWheel {
                source: egui::MouseWheelSource::Unknown,
                unit: egui::MouseWheelUnit::Line,
                delta: vec2(0.0, 1.0),
                phase: egui::TouchPhase::Move,
                modifiers: Modifiers::NONE,
            };
            h.event(up());
            h.run_steps(1);
            let once = if role == Role::ComboBox { (2, 50.0) } else { (3, 51.0) };
            assert_eq!((h.state().0, h.state().1), once, "{role:?}: the start of the gesture steps it");
            h.hover_at(h.get_by_role(Role::Slider).rect().center() + vec2(0.0, 40.0));
            h.event(up());
            h.run_steps(1);
            h.hover_at(over);
            h.event(up());
            h.run_steps(4);
            assert_eq!((h.state().0, h.state().1), once, "{role:?} was stepped once, by the start of the gesture");
        }
    }

    /// The list of `harness`'s open dropdown (`None` while closed).
    fn open_list<S>(h: &Harness<'static, S>) -> Option<egui::Rect> {
        h.ctx.memory(|m| m.layer_ids().filter(|l| l.order == egui::Order::Foreground).find_map(|l| m.area_rect(l.id)))
    }

    /// An open dropdown list stays open as the panel scrolls, carried along with its button, so a
    /// gesture scrolling the panel can bring the pointer over it: that doesn't step it either.
    #[test]
    fn a_gesture_that_starts_over_the_panel_does_not_step_an_open_dropdown_list() {
        let mut h = controls_harness();
        h.get_by_role(Role::ComboBox).click();
        h.run_steps(4);
        let Some(list) = open_list(&h) else { panic!("the list is open") };
        // Beside the list, over the panel: below the slider's row, which would take the gesture.
        let slider = h.get_by_role(Role::Slider).rect();
        h.hover_at(egui::pos2(list.right() + 20.0, slider.bottom() + 10.0));
        h.run_steps(1);
        let down = || egui::Event::MouseWheel {
            source: egui::MouseWheelSource::Unknown,
            unit: egui::MouseWheelUnit::Point,
            delta: vec2(0.0, -12.0),
            phase: egui::TouchPhase::Move,
            modifiers: Modifiers::NONE,
        };
        h.event(down());
        h.run_steps(1);
        assert!(h.state().2 > 0.0, "the panel scrolled");
        // Still the same gesture as the pointer moves onto the list, a few notches' worth.
        for _ in 0..6 {
            let Some(list) = open_list(&h) else { panic!("the list stays open as the panel scrolls") };
            h.hover_at(list.center());
            h.event(down());
            h.run_steps(1);
        }
        assert_eq!(h.state().0, 1, "the open list under the pointer is not stepped");
        // A gesture that starts over the list still steps it.
        h.run_steps(20);
        let Some(list) = open_list(&h) else { panic!("the list is open") };
        h.hover_at(list.center());
        h.run_steps(1);
        h.event(egui::Event::MouseWheel {
            source: egui::MouseWheelSource::Unknown,
            unit: egui::MouseWheelUnit::Line,
            delta: vec2(0.0, -1.0),
            phase: egui::TouchPhase::Move,
            modifiers: Modifiers::NONE,
        });
        h.run_steps(4);
        assert_eq!(h.state().0, 2, "a gesture over the list steps it");
    }

    /// One pass of window `viewport` (an egui viewport: the main window, or a panel floated into
    /// a window of its own) at `time`, with [`super::FieldWheel`] installed as the app does: a
    /// wheel dropdown over a large filler. The dropdown keeps its id from one window to another,
    /// as a panel's widgets do when it is floated. The dropdown's rect.
    fn window_pass(ctx: &egui::Context, viewport: egui::ViewportId, time: f64, events: Vec<egui::Event>, selected: &mut usize) -> egui::Rect {
        const MODES: [(usize, &str); 4] = [(0, "Normal"), (1, "Dissolve"), (2, "Darken"), (3, "Multiply")];
        let input = egui::RawInput {
            viewport_id: viewport,
            viewports: std::iter::once((viewport, egui::ViewportInfo::default())).collect(),
            time: Some(time),
            screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(300.0, 200.0))),
            events,
            ..Default::default()
        };
        let mut rect = egui::Rect::NOTHING;
        let mut out = ctx.run_ui(input, |ui| {
            ui.scope_builder(egui::UiBuilder::new().scope_id(egui::Id::unique("panel-body")), |ui| {
                crate::widgets::dropdown_wheel_hovered(ui, "blend-mode", "Blend Mode", selected, &MODES, 120.0);
                rect = ui.min_rect();
            });
            ui.allocate_space(vec2(200.0, 1000.0));
        });
        out.textures_delta.clear();
        rect
    }

    /// Half a notch left over in one window doesn't add up with half a notch in another: a panel
    /// floated into a window of its own takes no step from the wheel of the window it left.
    #[test]
    fn a_half_notch_stays_in_its_window() {
        let ctx = egui::Context::default();
        ctx.add_plugin(super::FieldWheel);
        let floated = egui::ViewportId::from_hash_of("floated-panel");
        let mut selected = 1;
        let half = || egui::Event::MouseWheel {
            source: egui::MouseWheelSource::Unknown,
            unit: egui::MouseWheelUnit::Line,
            delta: vec2(0.0, -0.5),
            phase: egui::TouchPhase::Move,
            modifiers: Modifiers::NONE,
        };
        // Half a notch over the dropdown in the main window.
        let over = window_pass(&ctx, egui::ViewportId::ROOT, 0.0, Vec::new(), &mut selected).center();
        window_pass(&ctx, egui::ViewportId::ROOT, 0.01, vec![egui::Event::PointerMoved(over), half()], &mut selected);
        window_pass(&ctx, egui::ViewportId::ROOT, 0.02, Vec::new(), &mut selected);
        assert_eq!(selected, 1, "half a notch");
        // The panel, floated, gets half a notch in its own window.
        let over = window_pass(&ctx, floated, 1.0, Vec::new(), &mut selected).center();
        window_pass(&ctx, floated, 1.01, vec![egui::Event::PointerMoved(over), half()], &mut selected);
        window_pass(&ctx, floated, 1.02, Vec::new(), &mut selected);
        assert_eq!(selected, 1, "two halves in two windows are no notch");
        // Another half in the same window makes a whole one.
        window_pass(&ctx, floated, 2.0, vec![half()], &mut selected);
        window_pass(&ctx, floated, 2.01, Vec::new(), &mut selected);
        assert_eq!(selected, 2);
    }

    /// #11353: a panel floated into a window of its own has input of its own. A gesture there
    /// that starts over its dropdown steps it, though the main window is in the middle of
    /// scrolling a panel, and the main window's record isn't the floated window's.
    #[test]
    fn a_floated_panels_window_follows_its_own_wheel_gestures() {
        let ctx = egui::Context::default();
        ctx.add_plugin(super::FieldWheel);
        let floated = egui::ViewportId::from_hash_of("floated-panel");
        let (mut main_sel, mut floated_sel) = (1, 1);
        let wheel = || egui::Event::MouseWheel {
            source: egui::MouseWheelSource::Unknown,
            unit: egui::MouseWheelUnit::Line,
            delta: vec2(0.0, -1.0),
            phase: egui::TouchPhase::Move,
            modifiers: Modifiers::NONE,
        };
        // The main window scrolls a panel, its pointer below its dropdown.
        let below = window_pass(&ctx, egui::ViewportId::ROOT, 0.0, Vec::new(), &mut main_sel).center() + vec2(0.0, 130.0);
        window_pass(&ctx, egui::ViewportId::ROOT, 0.01, vec![egui::Event::PointerMoved(below), wheel()], &mut main_sel);
        // Meanwhile a gesture in the floated window starts with its pointer on its dropdown. (Its
        // passes come after the main window's: egui forgets a window the main window's pass
        // didn't show, and the dock shows its floated panels in every pass.)
        let over = window_pass(&ctx, floated, 0.02, Vec::new(), &mut floated_sel).center();
        window_pass(&ctx, floated, 0.03, vec![egui::Event::PointerMoved(over)], &mut floated_sel);
        window_pass(&ctx, floated, 0.04, vec![wheel()], &mut floated_sel);
        window_pass(&ctx, floated, 0.06, Vec::new(), &mut floated_sel);
        assert_eq!(floated_sel, 2, "the floated panel's dropdown is stepped");
        assert_eq!(main_sel, 1, "the main window's dropdown isn't");
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

    /// A gesture from a system that doesn't say what drives it, starting with a zero-delta event
    /// and moving a frame later, steps the field under the pointer.
    #[test]
    fn a_gesture_that_starts_with_no_delta_steps_a_hovered_field() {
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
