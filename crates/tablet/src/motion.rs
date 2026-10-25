//! Android `MotionEvent` values → pen samples (pure; tested on every platform).
//!
//! The Activity passes each event's raw fields through [`crate::android`]: the masked action,
//! the pointer's tool type, the button state, and the `AXIS_PRESSURE`, `AXIS_TILT` and
//! `AXIS_ORIENTATION` values. A stylus (Wacom EMR pens, S Pen, USI pens) reports
//! `TOOL_TYPE_STYLUS` for its tip and `TOOL_TYPE_ERASER` for its eraser end.
//!
//! Android describes the pen's pose in polar form: `AXIS_TILT` is the angle from the screen's
//! normal (0 upright, π/2 lying flat) and `AXIS_ORIENTATION` the direction the pen points in
//! the screen plane (0 up, π/2 right, ±π down). Samples use the W3C Pointer Events tilt pair
//! instead (degrees, +x leaning right, +y leaning towards the user); the conversion projects the
//! pen onto the XZ and YZ planes, as browsers do for Android pens.
//!
//! Semantics match the web runner's Pointer Events listener: a sample while the pen touches,
//! kept through the lift (the stroke's last points keep their pressure), cleared when it hovers,
//! when the event is cancelled, and for fingers and mice.

use crate::{Sample, Update};

/// `MotionEvent.getActionMasked()` values.
pub mod action {
    pub const DOWN: i32 = 0;
    pub const UP: i32 = 1;
    pub const MOVE: i32 = 2;
    pub const CANCEL: i32 = 3;
    pub const POINTER_DOWN: i32 = 5;
    pub const POINTER_UP: i32 = 6;
    pub const HOVER_MOVE: i32 = 7;
    pub const HOVER_ENTER: i32 = 9;
    pub const HOVER_EXIT: i32 = 10;
    pub const BUTTON_PRESS: i32 = 11;
}

/// `MotionEvent.getToolType()` values.
pub mod tool {
    pub const UNKNOWN: i32 = 0;
    pub const FINGER: i32 = 1;
    pub const STYLUS: i32 = 2;
    pub const MOUSE: i32 = 3;
    pub const ERASER: i32 = 4;
}

/// `MotionEvent.getButtonState()` bits.
pub mod button {
    pub const PRIMARY: i32 = 1;
    pub const SECONDARY: i32 = 2;
    pub const TERTIARY: i32 = 4;
    pub const STYLUS_PRIMARY: i32 = 0x20;
    pub const STYLUS_SECONDARY: i32 = 0x40;
}

/// A pen's controls besides its tip.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PenControl {
    /// The side button nearest the tip (an S Pen's only one).
    Lower,
    /// The middle side button (a two-button pen's upper one).
    Upper,
    /// A third side button, above the other two (Wacom Pro Pen 3).
    Top,
    /// The eraser end (the pen turned over).
    EraserEnd,
}

impl PenControl {
    pub const ALL: [PenControl; 4] = [PenControl::Lower, PenControl::Upper, PenControl::Top, PenControl::EraserEnd];

    pub fn index(self) -> usize {
        self as usize
    }
}

/// Which of a pen's controls an event shows in use, by [`PenControl::index`]. The lower side
/// button is `BUTTON_STYLUS_PRIMARY` (older Samsung S Pens: `BUTTON_SECONDARY`), the upper
/// `BUTTON_STYLUS_SECONDARY` and a third `BUTTON_TERTIARY`; the eraser end is its own tool type.
pub fn controls(tool_type: i32, buttons: i32) -> [bool; 4] {
    match tool_type {
        tool::STYLUS => {
            [buttons & (button::STYLUS_PRIMARY | button::SECONDARY) != 0, buttons & button::STYLUS_SECONDARY != 0, buttons & button::TERTIARY != 0, false]
        }
        tool::ERASER => [false, false, false, true],
        _ => [false; 4],
    }
}

/// Which tool each pointer on the screen is, by Android pointer id (winit's touch id), so the
/// app can tell a finger from a pen or a mouse. Android reuses a pointer's id once it lifts: the
/// latest touch-down wins. A pointer never reported counts as a finger (the old behaviour).
#[derive(Clone, Debug, Default)]
pub struct Pointers {
    /// Each pointer's id, tool and the buttons held when it went down.
    tools: Vec<(u64, i32, i32)>,
}

impl Pointers {
    /// Pointers remembered at most (ten fingers and a pen, with room to spare).
    const MAX: usize = 32;

    pub const fn new() -> Self {
        Self { tools: Vec::new() }
    }

    /// A pointer went down with this tool and these buttons held.
    pub fn down(&mut self, id: u64, tool: i32, buttons: i32) {
        self.tools.retain(|(i, _, _)| *i != id);
        if self.tools.len() >= Self::MAX {
            self.tools.remove(0);
        }
        self.tools.push((id, tool, buttons));
    }

    pub fn tool(&self, id: u64) -> Option<i32> {
        self.tools.iter().find(|(i, _, _)| *i == id).map(|(_, t, _)| *t)
    }

    /// The buttons held when the pointer went down (0 if never reported).
    pub fn buttons(&self, id: u64) -> i32 {
        self.tools.iter().find(|(i, _, _)| *i == id).map_or(0, |(_, _, b)| *b)
    }

    /// A finger (or a pointer of unknown kind), as opposed to a pen, eraser or mouse.
    pub fn is_finger(&self, id: u64) -> bool {
        matches!(self.tool(id), None | Some(tool::FINGER) | Some(tool::UNKNOWN))
    }
}

/// One pointer of one `MotionEvent`, as the Activity read it.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RawEvent {
    pub action: i32,
    pub tool: i32,
    pub buttons: i32,
    /// `AXIS_PRESSURE`, nominally 0..1 (some digitizers overshoot; samples clamp).
    pub pressure: f32,
    /// `AXIS_TILT` in radians, 0..π/2.
    pub tilt: f32,
    /// `AXIS_ORIENTATION` in radians, -π..π.
    pub orientation: f32,
}

/// What `e` means for the current pen sample (the eraser end erases, the side buttons don't).
pub fn handle(e: &RawEvent) -> Update {
    handle_with(e, [false, false, false, true])
}

/// [`handle`], where the controls set in `erases` (by [`PenControl::index`]) erase while in use.
pub fn handle_with(e: &RawEvent, erases: [bool; 4]) -> Update {
    let pen = matches!(e.tool, tool::STYLUS | tool::ERASER);
    if !pen {
        return Update::Set(None);
    }
    match e.action {
        action::DOWN | action::MOVE | action::POINTER_DOWN => {
            let mut s = sample(e);
            s.eraser = controls(e.tool, e.buttons).iter().zip(erases).any(|(used, erases)| *used && erases);
            Update::Set(Some(s))
        }
        // Keep the last sample through the lift: the stroke's final points still need it.
        action::UP | action::POINTER_UP => Update::Keep,
        // Hovering: no pressure, but the end (or eraser button) in use is known, so the eraser
        // switch happens before the stroke, as on the desktop (AppKit proximity).
        action::HOVER_ENTER | action::HOVER_MOVE => {
            let mut s = sample(e);
            s.pressure = 0.0;
            s.eraser = controls(e.tool, e.buttons).iter().zip(erases).any(|(used, erases)| *used && erases);
            Update::Set(Some(s))
        }
        action::CANCEL | action::HOVER_EXIT => Update::Set(None),
        _ => Update::Keep,
    }
}

/// A finger's pressure from the size of its contact with the screen: a finger pressed harder
/// flattens and touches more glass. `light` and `firm` are the contact sizes of a light touch and
/// a firm press (calibrated on the device); between them the pressure rises from a little to
/// full. Without a usable calibration (or a size), full pressure, as before.
pub fn finger_pressure(size: f32, light: f32, firm: f32) -> f32 {
    if !(size.is_finite() && light.is_finite() && firm.is_finite()) || size <= 0.0 || firm <= light {
        return 1.0;
    }
    ((size - light) / (firm - light)).clamp(MIN_FINGER_PRESSURE, 1.0)
}

/// The lightest a finger paints: a touch always leaves a mark.
pub const MIN_FINGER_PRESSURE: f32 = 0.05;

/// The sample of a pen contact.
fn sample(e: &RawEvent) -> Sample {
    let (tilt_x, tilt_y) = tilt_xy(e.tilt, e.orientation);
    Sample { pressure: e.pressure, tilt_x, tilt_y, rotation: 0.0, eraser: e.tool == tool::ERASER }.sanitized()
}

/// Android's polar pose (tilt from the normal, orientation in the screen plane, radians) as the
/// W3C tilt pair in degrees. Non-finite input reads as upright.
pub fn tilt_xy(tilt: f32, orientation: f32) -> (f32, f32) {
    if !tilt.is_finite() || !orientation.is_finite() {
        return (0.0, 0.0);
    }
    let tilt = tilt.clamp(0.0, std::f32::consts::FRAC_PI_2);
    // f32 cos(π/2) is a hair below 0, which would flip atan2 to ±180°.
    let (r, z) = (tilt.sin(), tilt.cos().max(0.0));
    let x = (-orientation).sin() * r;
    let y = (-orientation).cos() * r;
    (x.atan2(z).to_degrees(), y.atan2(z).to_degrees())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_finger_pressed_harder_touches_more_and_paints_heavier() {
        // Calibrated: a light touch 40 px across, a firm press 90.
        assert_eq!(finger_pressure(90.0, 40.0, 90.0), 1.0);
        assert!((finger_pressure(65.0, 40.0, 90.0) - 0.5).abs() < 1e-6);
        assert_eq!(finger_pressure(30.0, 40.0, 90.0), MIN_FINGER_PRESSURE, "a touch always leaves a mark");
        assert_eq!(finger_pressure(500.0, 40.0, 90.0), 1.0);
        // Not calibrated, no size, or nonsense: full pressure, as without the option.
        assert_eq!(finger_pressure(65.0, 0.0, 0.0), 1.0);
        assert_eq!(finger_pressure(0.0, 40.0, 90.0), 1.0);
        assert_eq!(finger_pressure(f32::NAN, 40.0, 90.0), 1.0);
        assert_eq!(finger_pressure(65.0, 90.0, 40.0), 1.0);
    }

    #[test]
    fn pointers_remember_their_tool() {
        let mut p = Pointers::default();
        assert!(p.is_finger(7), "unknown pointers count as fingers");
        p.down(0, tool::FINGER, 0);
        p.down(1, tool::STYLUS, 0);
        p.down(2, tool::MOUSE, button::SECONDARY);
        assert!(p.is_finger(0) && !p.is_finger(1) && !p.is_finger(2));
        assert_eq!((p.buttons(2), p.buttons(0), p.buttons(9)), (button::SECONDARY, 0, 0));
        p.down(1, tool::FINGER, 0);
        assert!(p.is_finger(1), "a reused id takes the new tool");
        for i in 0..100 {
            p.down(i, tool::ERASER, 0);
        }
        assert!(!p.is_finger(99) && p.tools.len() <= Pointers::MAX, "bounded");
    }
    use std::f32::consts::{FRAC_PI_2, FRAC_PI_4, PI};

    fn pen(action: i32, pressure: f32) -> RawEvent {
        RawEvent { action, tool: tool::STYLUS, pressure, ..Default::default() }
    }

    fn close(a: (f32, f32), b: (f32, f32)) -> bool {
        (a.0 - b.0).abs() < 1e-3 && (a.1 - b.1).abs() < 1e-3
    }

    #[test]
    fn a_pen_contact_reports_pressure_until_it_hovers() {
        assert!(matches!(handle(&pen(action::HOVER_ENTER, 0.0)), Update::Set(Some(Sample { pressure: 0.0, eraser: false, .. }))));
        let Update::Set(Some(s)) = handle(&pen(action::DOWN, 0.25)) else { panic!("down is a sample") };
        assert_eq!((s.pressure, s.eraser), (0.25, false));
        let Update::Set(Some(s)) = handle(&pen(action::MOVE, 0.75)) else { panic!("move is a sample") };
        assert_eq!(s.pressure, 0.75);
        assert_eq!(handle(&pen(action::UP, 0.0)), Update::Keep, "the lift keeps the last pressure");
        assert!(matches!(handle(&pen(action::HOVER_MOVE, 0.4)), Update::Set(Some(Sample { pressure: 0.0, .. }))), "hovering has no pressure");
        assert_eq!(handle(&pen(action::HOVER_EXIT, 0.0)), Update::Set(None));
        assert_eq!(handle(&pen(action::CANCEL, 0.5)), Update::Set(None));
        assert_eq!(handle(&pen(42, 0.5)), Update::Keep, "unknown actions change nothing");
    }

    #[test]
    fn fingers_and_mice_are_not_pens() {
        for t in [tool::FINGER, tool::MOUSE, tool::UNKNOWN, 99] {
            assert_eq!(handle(&RawEvent { action: action::MOVE, tool: t, pressure: 0.3, ..Default::default() }), Update::Set(None), "tool {t}");
        }
    }

    #[test]
    fn the_eraser_end_is_flagged() {
        let e = RawEvent { action: action::DOWN, tool: tool::ERASER, pressure: 0.5, ..Default::default() };
        assert!(matches!(handle(&e), Update::Set(Some(Sample { eraser: true, .. }))));
    }

    #[test]
    fn each_control_erases_only_when_set_to() {
        let erases = |tool, buttons, set: [bool; 4]| {
            let e = RawEvent { action: action::MOVE, tool, buttons, pressure: 0.5, ..Default::default() };
            matches!(handle_with(&e, set), Update::Set(Some(Sample { eraser: true, .. })))
        };
        for b in [button::STYLUS_PRIMARY, button::SECONDARY] {
            assert!(erases(tool::STYLUS, b, [true, false, false, false]), "lower {b}");
            assert!(!erases(tool::STYLUS, b, [false, true, true, true]), "lower {b}");
        }
        assert!(erases(tool::STYLUS, button::STYLUS_SECONDARY, [false, true, false, false]), "upper");
        assert!(!erases(tool::STYLUS, button::STYLUS_SECONDARY, [true, false, true, true]), "upper");
        assert!(erases(tool::STYLUS, button::TERTIARY, [false, false, true, false]), "top");
        assert!(!erases(tool::STYLUS, button::TERTIARY, [true, true, false, true]), "top");
        assert!(!erases(tool::STYLUS, 0, [true; 4]), "the plain tip");
        assert!(erases(tool::ERASER, 0, [false, false, false, true]) && !erases(tool::ERASER, 0, [true, true, true, false]), "the eraser end");
        assert_eq!(controls(tool::MOUSE, button::SECONDARY), [false; 4], "a mouse has no pen controls");
    }

    #[test]
    fn hovering_tells_which_end_erases_before_the_stroke() {
        let hover = |tool, buttons| {
            let e = RawEvent { action: action::HOVER_MOVE, tool, buttons, ..Default::default() };
            matches!(handle_with(&e, [true, false, false, true]), Update::Set(Some(Sample { eraser: true, pressure: 0.0, .. })))
        };
        assert!(hover(tool::STYLUS, button::STYLUS_PRIMARY), "the erasing button, hovering");
        assert!(hover(tool::ERASER, 0), "the eraser end, hovering");
        assert!(!hover(tool::STYLUS, 0) && !hover(tool::STYLUS, button::STYLUS_SECONDARY), "the tip, or a button that doesn't erase");
    }

    #[test]
    fn tilt_converts_to_the_w3c_pair() {
        assert!(close(tilt_xy(0.0, 1.0), (0.0, 0.0)), "upright has no tilt whatever the orientation");
        // 45° from upright, pointing up the screen: the pen leans towards the user (+y).
        assert!(close(tilt_xy(FRAC_PI_4, 0.0), (0.0, 45.0)));
        // Pointing right: the barrel leans left (-x); pointing left: right (+x); down: away (-y).
        assert!(close(tilt_xy(FRAC_PI_4, FRAC_PI_2), (-45.0, 0.0)));
        assert!(close(tilt_xy(FRAC_PI_4, -FRAC_PI_2), (45.0, 0.0)));
        assert!(close(tilt_xy(FRAC_PI_4, PI), (0.0, -45.0)));
        // Diagonal: both components, each below the full tilt.
        let (x, y) = tilt_xy(FRAC_PI_4, -FRAC_PI_4);
        assert!(x > 0.0 && y > 0.0 && x < 45.0 && y < 45.0, "{x} {y}");
        // Lying flat reaches ±90 on the axis it points along.
        assert!(close(tilt_xy(FRAC_PI_2, 0.0), (0.0, 90.0)));
    }

    #[test]
    fn hostile_values_never_escape_the_documented_ranges() {
        for v in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, -10.0, 1e9] {
            let e = RawEvent { action: action::MOVE, tool: tool::STYLUS, buttons: -1, pressure: v, tilt: v, orientation: v };
            let Update::Set(Some(s)) = handle(&e) else { panic!("a pen move is a sample") };
            assert!((0.0..=1.0).contains(&s.pressure), "{v}: {s:?}");
            assert!((-90.0..=90.0).contains(&s.tilt_x) && (-90.0..=90.0).contains(&s.tilt_y), "{v}: {s:?}");
            assert_eq!(s.rotation, 0.0);
        }
    }
}
