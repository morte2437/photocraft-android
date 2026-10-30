//! Android: the Activity's pen hook.
//!
//! winit's Android backend forwards a pen's pressure as touch force but drops its tilt,
//! orientation and tool type (tip or eraser). The app's Activity (`PhotoCraftActivity.java`)
//! reads them from every `MotionEvent` before the event goes on to GameActivity and winit, and
//! calls the `native` method below for each pointer and historical sample. Only primitives
//! cross JNI: no `JNIEnv` call is made, so nothing here dereferences a pointer.
//!
//! The one `unsafe` in this module is the `#[unsafe(no_mangle)]` attribute itself, which JNI's
//! name-based lookup (`Java_<package>_<class>_<method>`) needs.

#![allow(unsafe_code)]

use crate::input_age::{InputAge, InputAges};
use crate::motion::{self, PenControl, Pointers, RawEvent, handle_with, tool};
use crate::{Error, Sample, Update, deliver};
use std::ffi::c_void;
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

type Sink = Box<dyn Fn(Option<Sample>) + Send + Sync>;

/// Where pen samples go; set by [`install`].
static SINK: Mutex<Option<Sink>> = Mutex::new(None);

/// The tool of each pointer on the screen (`nativePointer`).
static POINTERS: Mutex<Pointers> = Mutex::new(Pointers::new());
/// When a pen (tip or eraser) last touched or hovered.
static PEN_SEEN: Mutex<Option<Instant>> = Mutex::new(None);
/// The controls that erase while in use, by [`PenControl::index`] ([`set_erasing_controls`]).
static ERASES: Mutex<[bool; 4]> = Mutex::new([false, false, false, true]);
/// Finger pressure from the contact size (Touch Input): on, and the light and firm sizes.
static FINGER_PRESSURE: Mutex<Option<(f32, f32)>> = Mutex::new(None);
/// The last finger contact the screen reported, and the pressure given to the stroke (smoothed).
static CONTACT: Mutex<Option<Contact>> = Mutex::new(None);

/// What the touch screen says about a finger: Android's pressure, its size (`getSize`, 0..1)
/// and the long axis of the contact (`getTouchMajor`, pixels). Devices fill these in very
/// differently: some report real values, some a constant.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Contact {
    pub pressure: f32,
    pub size: f32,
    pub major: f32,
    /// The pressure a finger stroke gets from it (when finger pressure is on).
    pub mapped: f32,
}

/// A finger contact's size for [`motion::finger_pressure`]: the long axis where the device
/// reports one, else the normalised size (scaled to the same rough range).
pub fn contact_size(c: &Contact) -> f32 {
    if c.major.is_finite() && c.major > 0.0 { c.major } else { c.size * 1000.0 }
}

/// Turn finger pressure from the contact size on (`Some((light, firm))`) or off.
pub fn set_finger_pressure(calibration: Option<(f32, f32)>) {
    *FINGER_PRESSURE.lock().unwrap_or_else(PoisonError::into_inner) = calibration;
}

/// The last finger contact (for the Touch Input readout and calibration).
pub fn last_contact() -> Option<Contact> {
    *CONTACT.lock().unwrap_or_else(PoisonError::into_inner)
}

/// A finger's sample, with pressure from its contact size, when finger pressure is on.
fn finger_sample(action: i32, pressure: f32, size: f32, major: f32) -> Option<Update> {
    let calibration = *FINGER_PRESSURE.lock().unwrap_or_else(PoisonError::into_inner);
    let mut last = CONTACT.lock().unwrap_or_else(PoisonError::into_inner);
    let mut c = Contact { pressure, size, major, mapped: 1.0 };
    let (light, firm) = calibration.unwrap_or((0.0, 0.0));
    let target = motion::finger_pressure(contact_size(&c), light, firm);
    // Contact sizes jump in steps and wobble: ease towards each new reading.
    c.mapped = match (action, *last) {
        (motion::action::MOVE, Some(prev)) => prev.mapped + (target - prev.mapped) * 0.35,
        _ => target,
    };
    *last = Some(c);
    calibration?;
    Some(match action {
        motion::action::DOWN | motion::action::MOVE | motion::action::POINTER_DOWN => Update::Set(Some(Sample { pressure: c.mapped, ..Sample::default() })),
        motion::action::UP | motion::action::POINTER_UP => Update::Keep,
        _ => Update::Set(None),
    })
}
/// The controls in use in the pen's last event, and the presses since [`take_presses`].
static HELD: Mutex<[bool; 4]> = Mutex::new([false; 4]);
static PRESSES: Mutex<Vec<Press>> = Mutex::new(Vec::new());
/// Where the pen touched in each move sample (historical ones included), window pixels, since
/// [`take_pen_path`].
static PATH: Mutex<Vec<(f32, f32)>> = Mutex::new(Vec::new());
/// The most [`PATH`] holds (a second of a fast digitizer, if nothing takes it).
const PATH_MAX: usize = 480;
/// Whether the hovering pen's last sample erased, so hover moves send only changes.
static HOVER_END: Mutex<Option<bool>> = Mutex::new(None);
/// How old the pen's samples were on arrival, since [`take_input_age`].
static AGES: Mutex<InputAges> = Mutex::new(InputAges::new());
/// The button state of the pen's last event, as Android reported it ([`last_buttons`]).
static LAST_BUTTONS: Mutex<i32> = Mutex::new(0);

/// A pen control pressed (a side button) or brought to the screen (the eraser end).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Press {
    pub control: PenControl,
    /// Where the pen hovered (window pixels), or `None` when it was touching.
    pub hover: Option<(f32, f32)>,
}

/// The buttons held when touch `id` went down (`MotionEvent.getButtonState()`): a mouse's right
/// button, a pen's side button.
pub fn pointer_buttons(id: u64) -> i32 {
    POINTERS.lock().unwrap_or_else(PoisonError::into_inner).buttons(id)
}

/// Called when the window's layout changes (the keyboard, a camera cutout, rounded corners), to
/// draw a frame: the app is idle, and nothing else wakes it for these.
static ON_LAYOUT: Mutex<Option<Box<dyn Fn() + Send>>> = Mutex::new(None);

/// Runs `wake` (from the Activity's thread) whenever the keyboard, cutouts or corners change.
pub fn on_layout_change(wake: impl Fn() + Send + 'static) {
    *ON_LAYOUT.lock().unwrap_or_else(PoisonError::into_inner) = Some(Box::new(wake));
}

fn layout_changed() {
    if let Some(wake) = ON_LAYOUT.lock().unwrap_or_else(PoisonError::into_inner).as_ref() {
        wake();
    }
}

/// The on-screen keyboard's height in window pixels (0 when it's hidden).
static IME_HEIGHT: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);

/// The on-screen keyboard's height in window pixels, 0 when it's hidden: the app lays out above
/// it (the Activity reports it on every layout change, `PhotoCraftActivity.nativeImeHeight`).
pub fn ime_height_px() -> f32 {
    IME_HEIGHT.load(std::sync::atomic::Ordering::Relaxed).max(0) as f32
}

/// The keyboard's height changed, from `PhotoCraftActivity.nativeImeHeight` (Java).
#[unsafe(no_mangle)]
pub extern "system" fn Java_ai_storyteller_photocraft_PhotoCraftActivity_nativeImeHeight(_env: *mut c_void, _this: *mut c_void, px: i32) {
    IME_HEIGHT.store(px, std::sync::atomic::Ordering::Relaxed);
    layout_changed();
}

/// The camera cutouts' rects (left, top, right, bottom, window pixels): up to two.
static CUTOUTS: std::sync::Mutex<Vec<[f32; 4]>> = std::sync::Mutex::new(Vec::new());
static CORNERS: std::sync::Mutex<[[f32; 3]; 4]> = std::sync::Mutex::new([[0.0; 3]; 4]);

/// Where a front camera's cutouts (a punch hole or notch) are on screen, as `[left, top, right,
/// bottom]` in window pixels: none on most tablets (the Activity reports them on layout changes,
/// `PhotoCraftActivity.nativeCutout`).
pub fn cutouts_px() -> Vec<[f32; 4]> {
    CUTOUTS.lock().unwrap_or_else(PoisonError::into_inner).clone()
}

/// The cutouts changed (or were first measured), from `PhotoCraftActivity.nativeCutout` (Java):
/// `n` of the two rects are real.
#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub extern "system" fn Java_ai_storyteller_photocraft_PhotoCraftActivity_nativeCutout(
    _env: *mut c_void,
    _this: *mut c_void,
    n: i32,
    l0: i32,
    t0: i32,
    r0: i32,
    b0: i32,
    l1: i32,
    t1: i32,
    r1: i32,
    b1: i32,
) {
    let rects = [[l0, t0, r0, b0], [l1, t1, r1, b1]];
    let n = usize::try_from(n).unwrap_or(0).min(rects.len());
    let rects: Vec<[f32; 4]> = rects.iter().take(n).filter(|r| r[2] > r[0] && r[3] > r[1]).map(|r| r.map(|v| v as f32)).collect();
    *CUTOUTS.lock().unwrap_or_else(PoisonError::into_inner) = rects;
    layout_changed();
}

/// The screen's rounded corners as `[radius, centre x, centre y]` in window pixels, top-left,
/// top-right, bottom-right and bottom-left; a radius of 0 is a square corner (Android 12+ reports
/// them, `PhotoCraftActivity.nativeCorners`).
pub fn corners_px() -> [[f32; 3]; 4] {
    *CORNERS.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The rounded corners changed (or were first measured), from `PhotoCraftActivity.nativeCorners`.
#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub extern "system" fn Java_ai_storyteller_photocraft_PhotoCraftActivity_nativeCorners(
    _env: *mut c_void,
    _this: *mut c_void,
    r0: i32,
    x0: i32,
    y0: i32,
    r1: i32,
    x1: i32,
    y1: i32,
    r2: i32,
    x2: i32,
    y2: i32,
    r3: i32,
    x3: i32,
    y3: i32,
) {
    let c = [[r0, x0, y0], [r1, x1, y1], [r2, x2, y2], [r3, x3, y3]];
    *CORNERS.lock().unwrap_or_else(PoisonError::into_inner) = c.map(|c| if c[0] > 0 { c.map(|v| v as f32) } else { [0.0; 3] });
    layout_changed();
}

/// The tool of touch `id` (`MotionEvent.getToolType()`), if it was reported.
pub fn pointer_tool(id: u64) -> Option<i32> {
    POINTERS.lock().unwrap_or_else(PoisonError::into_inner).tool(id)
}

/// Which pen controls erase while in use, by [`PenControl::index`].
pub fn set_erasing_controls(erases: [bool; 4]) {
    *ERASES.lock().unwrap_or_else(PoisonError::into_inner) = erases;
}

/// The pen controls in use right now (held buttons, the eraser end near the screen).
pub fn controls_held() -> [bool; 4] {
    *HELD.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The pen's button state as Android last reported it (`MotionEvent.getButtonState()`), so the
/// settings can show what an unusual pen sends.
pub fn last_buttons() -> i32 {
    *LAST_BUTTONS.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The pen's positions (window pixels, oldest first) for every move sample since the last call,
/// the ones Android batched between events included. winit passes on only each event's
/// latest position, so a quick curve would be drawn from a few straight pieces.
pub fn take_pen_path() -> Vec<(f32, f32)> {
    std::mem::take(&mut *PATH.lock().unwrap_or_else(PoisonError::into_inner))
}

/// How old the pen's samples have become now, since the last call (see [`InputAges`]): the
/// input queue, batching and waiting for a frame. `None` when the pen sent nothing since.
pub fn take_input_age() -> Option<InputAge> {
    AGES.lock().unwrap_or_else(PoisonError::into_inner).take(Instant::now())
}

fn track_path(action: i32, tool_type: i32, x: f32, y: f32) {
    if tool_type != tool::STYLUS && tool_type != tool::ERASER {
        return;
    }
    let mut path = PATH.lock().unwrap_or_else(PoisonError::into_inner);
    match action {
        // A new contact: earlier samples belong to the last stroke.
        motion::action::DOWN => path.clear(),
        motion::action::MOVE if x.is_finite() && y.is_finite() => {
            if path.len() >= PATH_MAX {
                path.remove(0);
            }
            path.push((x, y));
        }
        _ => {}
    }
}

fn track_age(action: i32, tool_type: i32, age_ms: i32) {
    if (tool_type == tool::STYLUS || tool_type == tool::ERASER) && matches!(action, motion::action::DOWN | motion::action::MOVE) {
        let age = Duration::from_millis(u64::try_from(age_ms).unwrap_or(0));
        AGES.lock().unwrap_or_else(PoisonError::into_inner).record(Instant::now(), age);
    }
}

/// The presses since the last call, oldest first.
pub fn take_presses() -> Vec<Press> {
    std::mem::take(&mut *PRESSES.lock().unwrap_or_else(PoisonError::into_inner))
}

fn track_controls(action: i32, tool_type: i32, buttons: i32, x: f32, y: f32) {
    if tool_type != tool::STYLUS && tool_type != tool::ERASER {
        return;
    }
    let gone = matches!(action, motion::action::HOVER_EXIT | motion::action::CANCEL);
    let now = if gone { [false; 4] } else { motion::controls(tool_type, buttons) };
    *LAST_BUTTONS.lock().unwrap_or_else(PoisonError::into_inner) = if gone { 0 } else { buttons };
    let hovering = matches!(action, motion::action::HOVER_ENTER | motion::action::HOVER_MOVE | motion::action::BUTTON_PRESS);
    let mut held = HELD.lock().unwrap_or_else(PoisonError::into_inner);
    for control in PenControl::ALL {
        let i = control.index();
        if now[i] && !held[i] {
            let hover = (hovering && x.is_finite() && y.is_finite()).then_some((x, y));
            let mut presses = PRESSES.lock().unwrap_or_else(PoisonError::into_inner);
            if presses.len() < 16 {
                presses.push(Press { control, hover });
            }
        }
    }
    *held = now;
}

/// Whether touch `id` (winit's touch id: Android's pointer id) is a finger rather than a pen,
/// eraser or mouse.
pub fn is_finger(id: u64) -> bool {
    POINTERS.lock().unwrap_or_else(PoisonError::into_inner).is_finger(id)
}

/// Whether a pen touched or hovered within `d` (palm rejection: a hand resting while the pen is
/// near).
pub fn pen_seen_within(d: Duration) -> bool {
    PEN_SEEN.lock().unwrap_or_else(PoisonError::into_inner).is_some_and(|t| t.elapsed() <= d)
}

fn saw_pen(tool_type: i32) {
    if tool_type == tool::STYLUS || tool_type == tool::ERASER {
        *PEN_SEEN.lock().unwrap_or_else(PoisonError::into_inner) = Some(Instant::now());
    }
}

/// A pointer went down, from `PhotoCraftActivity.nativePointer` (Java), before the event goes on
/// to winit: its id, `MotionEvent` tool type and button state.
#[unsafe(no_mangle)]
pub extern "system" fn Java_ai_storyteller_photocraft_PhotoCraftActivity_nativePointer(
    _env: *mut c_void,
    _this: *mut c_void,
    id: i32,
    tool_type: i32,
    buttons: i32,
) {
    if let Ok(id) = u64::try_from(id) {
        POINTERS.lock().unwrap_or_else(PoisonError::into_inner).down(id, tool_type, buttons);
    }
    saw_pen(tool_type);
}

/// Route the Activity's pen samples to `callback` (replacing an earlier one).
pub fn install(callback: impl Fn(Option<Sample>) + Send + Sync + 'static) -> Result<(), Error> {
    *SINK.lock().unwrap_or_else(PoisonError::into_inner) = Some(Box::new(callback));
    Ok(())
}

/// One pointer of a `MotionEvent`, from `PhotoCraftActivity.nativePen` (Java). Called on the
/// Activity's UI thread. JNI types: `jint` is `i32`, `jfloat` is `f32`. `age_ms` is the time
/// since the digitizer event (`SystemClock.uptimeMillis() - MotionEvent.getEventTime()`).
///
/// Never unwinds: the mapping doesn't panic and [`deliver`] contains the callback.
#[unsafe(no_mangle)]
pub extern "system" fn Java_ai_storyteller_photocraft_PhotoCraftActivity_nativePen(
    _env: *mut c_void,
    _this: *mut c_void,
    action: i32,
    tool: i32,
    buttons: i32,
    pressure: f32,
    tilt: f32,
    orientation: f32,
    x: f32,
    y: f32,
    age_ms: i32,
    size: f32,
    major: f32,
) {
    saw_pen(tool);
    track_age(action, tool, age_ms);
    track_controls(action, tool, buttons, x, y);
    track_path(action, tool, x, y);
    let e = RawEvent { action, tool, buttons, pressure, tilt, orientation };
    let finger = (tool == motion::tool::FINGER).then(|| finger_sample(action, pressure, size, major)).flatten();
    let update = finger.unwrap_or_else(|| handle_with(&e, *ERASES.lock().unwrap_or_else(PoisonError::into_inner)));
    if let Update::Set(sample) = update {
        // Hovering, only a change of end (or of an erasing button) is news: a hover move every
        // few milliseconds would crowd the frame's samples.
        let hovering = matches!(action, motion::action::HOVER_ENTER | motion::action::HOVER_MOVE);
        let mut last = HOVER_END.lock().unwrap_or_else(PoisonError::into_inner);
        if hovering {
            let end = sample.map(|s| s.eraser);
            if action == motion::action::HOVER_MOVE && *last == end {
                return;
            }
            *last = end;
        } else {
            *last = None;
        }
        drop(last);
        let guard = SINK.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(sink) = guard.as_ref() {
            deliver(sink.as_ref(), sample);
        }
    }
}
