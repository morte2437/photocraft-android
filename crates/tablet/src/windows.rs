//! Windows pointer hook for tablet data dropped by winit (the pen barrel button and eraser end).
//!
//! winit 0.30 drops `POINTER_PEN_INFO::penFlags` (`PEN_FLAG_BARREL`, `PEN_FLAG_INVERTED`,
//! `PEN_FLAG_ERASER`) on Windows and turns pen inputs into `WindowEvent::Touch`, mapping them to
//! `PointerButton::Primary`. Windows Ink also does not update `GetAsyncKeyState(VK_RBUTTON)` for
//! stylus barrel buttons. Thread-local hooks inspect `WM_POINTER*` messages on the UI thread to
//! track the physical barrel button state and which end of the pen is in use: `WH_GETMESSAGE` for
//! the posted ones (`WM_POINTERDOWN`, `WM_POINTERUPDATE`, `WM_POINTERUP`) and `WH_CALLWNDPROC` for
//! `WM_POINTERENTER` and `WM_POINTERLEAVE`, which Windows documents as sent to the window, so the
//! queue hook alone may never see the pen arrive or leave.

#![allow(unsafe_code, clippy::upper_case_acronyms)]

use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU32, Ordering};

static BARREL_DOWN: AtomicBool = AtomicBool::new(false);

/// The pointer id of the pen last seen, or [`NO_PEN`]. Ids come from a 16-bit field, so
/// `u32::MAX` is never one.
static PEN_ID: AtomicU32 = AtomicU32::new(NO_PEN);
const NO_PEN: u32 = u32::MAX;

/// The pen end last reported, as [`encode_end`] stores it.
static PEN_END: AtomicU8 = AtomicU8::new(0);

/// The pen end of the last contact [`take_pen_end`] has not read yet, as [`encode_end`] stores it.
static CONTACT_END: AtomicU8 = AtomicU8::new(0);

fn encode_end(end: Option<bool>) -> u8 {
    match end {
        None => 0,
        Some(false) => 1,
        Some(true) => 2,
    }
}

fn decode_end(v: u8) -> Option<bool> {
    match v {
        1 => Some(false),
        2 => Some(true),
        _ => None,
    }
}

#[repr(C)]
struct CWPSTRUCT {
    l_param: isize,
    w_param: usize,
    message: u32,
    hwnd: *mut std::ffi::c_void,
}

#[repr(C)]
struct POINT {
    x: i32,
    y: i32,
}

#[repr(C)]
struct MSG {
    hwnd: *mut std::ffi::c_void,
    message: u32,
    w_param: usize,
    l_param: isize,
    time: u32,
    pt: POINT,
}

#[repr(C)]
struct POINTER_INFO {
    pointer_type: u32,
    pointer_id: u32,
    frame_id: u32,
    pointer_flags: u32,
    source_device: *mut std::ffi::c_void,
    hwnd_target: *mut std::ffi::c_void,
    pt_pixel_location: POINT,
    pt_himetric_location: POINT,
    pt_pixel_location_raw: POINT,
    pt_himetric_location_raw: POINT,
    dw_time: u32,
    history_count: u32,
    input_data: i32,
    dw_key_states: u32,
    performance_count: u64,
    button_change_type: i32,
}

#[repr(C)]
struct POINTER_PEN_INFO {
    pointer_info: POINTER_INFO,
    pen_flags: u32,
    pen_mask: u32,
    pressure: u32,
    rotation: u32,
    tilt_x: i32,
    tilt_y: i32,
}

#[link(name = "user32")]
unsafe extern "system" {
    fn SetWindowsHookExW(
        id_hook: i32,
        lpfn: unsafe extern "system" fn(i32, usize, isize) -> isize,
        h_mod: *mut std::ffi::c_void,
        dw_thread_id: u32,
    ) -> *mut std::ffi::c_void;
    fn UnhookWindowsHookEx(hhk: *mut std::ffi::c_void) -> i32;
    fn CallNextHookEx(hhk: *mut std::ffi::c_void, n_code: i32, w_param: usize, l_param: isize) -> isize;
    fn GetCurrentThreadId() -> u32;
    fn GetPointerPenInfo(pointer_id: u32, pen_info: *mut POINTER_PEN_INFO) -> i32;
}

const WH_GETMESSAGE: i32 = 3;
const WH_CALLWNDPROC: i32 = 4;
/// `WM_POINTERENTER` (winuser.h).
pub const WM_POINTERENTER: u32 = 0x0249;
/// `WM_POINTERUPDATE` (winuser.h).
pub const WM_POINTERUPDATE: u32 = 0x0245;
/// `WM_POINTERDOWN` (winuser.h).
pub const WM_POINTERDOWN: u32 = 0x0246;
/// `WM_POINTERUP` (winuser.h).
pub const WM_POINTERUP: u32 = 0x0247;
/// `WM_POINTERLEAVE` (winuser.h).
pub const WM_POINTERLEAVE: u32 = 0x024A;

/// `PEN_FLAG_BARREL` (winuser.h): the barrel button of the pen is pressed.
pub const PEN_FLAG_BARREL: u32 = 0x00000001;

/// `PEN_FLAG_INVERTED` (winuser.h): the pen is inverted (its eraser end is in use).
pub const PEN_FLAG_INVERTED: u32 = 0x00000002;

/// `PEN_FLAG_ERASER` (winuser.h): the eraser button of the pen is pressed.
pub const PEN_FLAG_ERASER: u32 = 0x00000004;

/// `POINTER_MESSAGE_FLAG_SECONDBUTTON` (winuser.h): the secondary action button is pressed.
pub const POINTER_FLAG_SECONDBUTTON: u32 = 0x00000020;

/// Pure decode of which end of the pen is in use: `Some(true)` for the eraser end, `Some(false)`
/// for the tip, `None` when the pointer is not a pen (`GetPointerPenInfo` failed: mouse, touch).
///
/// Only `PEN_FLAG_INVERTED` means the pen is flipped: it is set the whole time the eraser end is
/// in range, and `PEN_FLAG_ERASER` joins it on contact. `PEN_FLAG_ERASER` alone is an eraser
/// *button*, which some pens put on the barrel; pressing that must not switch tools.
pub fn is_eraser_end(pen_flags: Option<u32>) -> Option<bool> {
    pen_flags.map(|pen| (pen & PEN_FLAG_INVERTED) != 0)
}

/// Pure decode of whether the stylus barrel button is pressed.
///
/// Plain tip contact sets `POINTER_FLAG_INCONTACT` (`0x0004`) and optionally `POINTER_FLAG_FIRSTBUTTON`
/// (`0x0010`), which must never be confused with the barrel button (`POINTER_FLAG_SECONDBUTTON`, `0x0020`).
pub fn is_barrel_pressed(pen_flags: Option<u32>, pointer_message_flags: u32) -> bool {
    let second_button = (pointer_message_flags & POINTER_FLAG_SECONDBUTTON) != 0;
    match pen_flags {
        Some(pen) => (pen & PEN_FLAG_BARREL) != 0 || second_button,
        None => second_button,
    }
}

/// What the hooks track between pointer messages.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PenState {
    /// The barrel button is pressed.
    pub barrel: bool,
    /// Which end of the pen is in use (`Some(true)` = eraser), `None` = no pen in range.
    pub end: Option<bool>,
    /// The pointer id of the pen last seen, so another pointer leaving does not forget it.
    pub pen_id: Option<u32>,
    /// The end of the last pen contact, kept when the pen leaves: the contact and its release
    /// can still be queued for the UI after the leave is handled, and must get that end's tool.
    pub contact: Option<bool>,
}

/// Pure transition of [`PenState`] for one pointer message. `w_param` is the message's (pointer
/// id in the low word, pointer flags in the high word); `pen_flags` reads `GetPointerPenInfo`'s
/// `penFlags` for a pointer id (`None`: not a pen), and only down and update read it.
pub fn next_state(prev: PenState, message: u32, w_param: usize, pen_flags: impl FnOnce(u32) -> Option<u32>) -> PenState {
    let pointer_id = (w_param & 0xFFFF) as u32;
    match message {
        // A pen entering range while hovering may stay still: its first update can come late.
        WM_POINTERDOWN | WM_POINTERUPDATE | WM_POINTERENTER => {
            let pointer_message_flags = ((w_param >> 16) & 0xFFFF) as u32;
            let pen_flags = pen_flags(pointer_id);
            let end = is_eraser_end(pen_flags);
            // A mouse or a finger in between leaves the pen's end as it was.
            PenState {
                barrel: is_barrel_pressed(pen_flags, pointer_message_flags),
                end: end.or(prev.end),
                pen_id: if pen_flags.is_some() { Some(pointer_id) } else { prev.pen_id },
                contact: if message == WM_POINTERDOWN { end.or(prev.contact) } else { prev.contact },
            }
        }
        WM_POINTERUP => PenState { barrel: false, ..prev },
        // A finger or a palm leaving while the pen stays in range (pen and touch screens).
        WM_POINTERLEAVE if prev.pen_id.is_some_and(|id| id != pointer_id) => prev,
        WM_POINTERLEAVE => PenState { contact: prev.contact, ..PenState::default() },
        _ => prev,
    }
}

/// Apply one pointer message to the state [`is_barrel_button_down`], [`pen_end`] and
/// [`take_pen_end`] read. The hooks call this; tests call it with a fake `pen_flags` reader.
pub fn handle_pointer_message(message: u32, w_param: usize, pen_flags: impl FnOnce(u32) -> Option<u32>) {
    let prev = PenState {
        barrel: BARREL_DOWN.load(Ordering::Relaxed),
        end: decode_end(PEN_END.load(Ordering::Relaxed)),
        pen_id: Some(PEN_ID.load(Ordering::Relaxed)).filter(|&id| id != NO_PEN),
        contact: decode_end(CONTACT_END.load(Ordering::Relaxed)),
    };
    let next = next_state(prev, message, w_param, pen_flags);
    BARREL_DOWN.store(next.barrel, Ordering::Relaxed);
    PEN_END.store(encode_end(next.end), Ordering::Relaxed);
    PEN_ID.store(next.pen_id.unwrap_or(NO_PEN), Ordering::Relaxed);
    CONTACT_END.store(encode_end(next.contact), Ordering::Relaxed);
}

fn read_pen_flags(pointer_id: u32) -> Option<u32> {
    // SAFETY: `info` is a stack-allocated buffer initialized to zero that outlives the call, and
    // `pointer_id` comes from a WM_POINTER message; an unknown id makes the call fail, not crash.
    unsafe {
        let mut info: POINTER_PEN_INFO = std::mem::zeroed();
        (GetPointerPenInfo(pointer_id, &mut info) != 0).then_some(info.pen_flags)
    }
}

/// `WH_GETMESSAGE`: the posted pointer messages, as the UI thread takes them from its queue.
unsafe extern "system" fn get_msg_proc(code: i32, w_param: usize, l_param: isize) -> isize {
    if code >= 0 && l_param != 0 {
        // SAFETY: Win32 guarantees that for code >= 0, l_param points to a valid MSG struct
        // dispatched to this thread's message queue.
        let msg = unsafe { &*(l_param as *const MSG) };
        // The WinTab backend shares this thread hook: it decodes the driver's proximity
        // messages (and ignores everything else) to clear a pen that left the tablet.
        crate::wintab::on_message(msg.message, msg.w_param, msg.l_param);
        handle_pointer_message(msg.message, msg.w_param, read_pen_flags);
    }
    // SAFETY: CallNextHookEx passes the parameters down the hook chain. MSDN notes the first
    // parameter is ignored and can be null.
    unsafe { CallNextHookEx(std::ptr::null_mut(), code, w_param, l_param) }
}

/// `WH_CALLWNDPROC`: messages sent to the thread's windows. Only `WM_POINTERENTER` and
/// `WM_POINTERLEAVE` matter here; down, update and up are posted and reach [`get_msg_proc`].
unsafe extern "system" fn call_wnd_proc(code: i32, w_param: usize, l_param: isize) -> isize {
    if code >= 0 && l_param != 0 {
        // SAFETY: Win32 guarantees that for code >= 0 (HC_ACTION), l_param points to a valid
        // CWPSTRUCT describing the message being sent.
        let msg = unsafe { &*(l_param as *const CWPSTRUCT) };
        match msg.message {
            WM_POINTERENTER => handle_pointer_message(msg.message, msg.w_param, read_pen_flags),
            WM_POINTERLEAVE => handle_pointer_message(msg.message, msg.w_param, |_| None),
            _ => {}
        }
    }
    // SAFETY: as in `get_msg_proc`.
    unsafe { CallNextHookEx(std::ptr::null_mut(), code, w_param, l_param) }
}

type HookProc = unsafe extern "system" fn(i32, usize, isize) -> isize;

/// Keeps the thread-local pointer message hooks installed; dropping it removes them.
/// Main-thread only (not `Send`).
pub struct Monitor {
    /// `WH_GETMESSAGE`, then `WH_CALLWNDPROC` (null when it failed to install).
    hooks: [*mut std::ffi::c_void; 2],
}

impl Monitor {
    /// Installs the thread-local pointer message hooks on the UI thread. Only a failed
    /// `WH_GETMESSAGE` hook is an error: it carries the barrel button and the pen end. Without
    /// `WH_CALLWNDPROC` the pen may not be seen leaving (see [`Monitor::tracks_leave`]).
    pub fn install() -> Result<Self, crate::Error> {
        // SAFETY: GetCurrentThreadId has no preconditions.
        let thread_id = unsafe { GetCurrentThreadId() };
        Self::install_with(|id_hook, proc_| {
            // SAFETY: installs a thread-specific hook on the calling thread (no module handle
            // needed) with one of our extern "system" callbacks.
            unsafe { SetWindowsHookExW(id_hook, proc_, std::ptr::null_mut(), thread_id) }
        })
    }

    /// [`Monitor::install`] with the hook installer passed in, so tests can fail one hook.
    fn install_with(mut set_hook: impl FnMut(i32, HookProc) -> *mut std::ffi::c_void) -> Result<Self, crate::Error> {
        let get_msg = set_hook(WH_GETMESSAGE, get_msg_proc);
        if get_msg.is_null() {
            return Err(crate::Error::Platform("SetWindowsHookExW(WH_GETMESSAGE) returned null".into()));
        }
        Ok(Self { hooks: [get_msg, set_hook(WH_CALLWNDPROC, call_wnd_proc)] })
    }

    /// Whether the `WH_CALLWNDPROC` hook installed. Without it the pen leaving range may be
    /// missed: the last pen end and barrel state stay until the next pointer message.
    pub fn tracks_leave(&self) -> bool {
        !self.hooks[1].is_null()
    }
}

impl Drop for Monitor {
    fn drop(&mut self) {
        for hook in self.hooks {
            if !hook.is_null() {
                // SAFETY: `hook` is a non-null handle returned by `SetWindowsHookExW`, unhooked
                // exactly once here on the thread that created it.
                unsafe {
                    UnhookWindowsHookEx(hook);
                }
            }
        }
        BARREL_DOWN.store(false, Ordering::Relaxed);
        PEN_END.store(encode_end(None), Ordering::Relaxed);
        PEN_ID.store(NO_PEN, Ordering::Relaxed);
        CONTACT_END.store(encode_end(None), Ordering::Relaxed);
    }
}

/// Returns true if the stylus barrel button is physically pressed.
pub fn is_barrel_button_down() -> bool {
    BARREL_DOWN.load(Ordering::Relaxed)
}

/// Which end of the pen is in use: `Some(true)` for the eraser end, `Some(false)` for the tip,
/// `None` when no pen is in range of the window.
pub fn pen_end() -> Option<bool> {
    decode_end(PEN_END.load(Ordering::Relaxed))
}

/// [`pen_end`] for the UI's once-per-frame read: the end of a contact not read yet comes first,
/// once, even when the pen has left since, so a click still queued for the UI gets that end's
/// tool.
pub fn take_pen_end() -> Option<bool> {
    decode_end(CONTACT_END.swap(encode_end(None), Ordering::Relaxed)).or_else(pen_end)
}

#[cfg(test)]
mod tests {
    use super::*;

    const POINTER_FLAG_INCONTACT: u32 = 0x0004;
    const POINTER_FLAG_FIRSTBUTTON: u32 = 0x0010;

    #[test]
    fn plain_pen_contact_is_not_treated_as_barrel() {
        // Plain tip touch on digitizer: INCONTACT is set, first button may be set, no barrel flag.
        assert!(!is_barrel_pressed(Some(0), POINTER_FLAG_INCONTACT));
        assert!(!is_barrel_pressed(Some(0), POINTER_FLAG_INCONTACT | POINTER_FLAG_FIRSTBUTTON));
        assert!(!is_barrel_pressed(None, POINTER_FLAG_INCONTACT));
    }

    #[test]
    fn pen_barrel_flag_is_detected() {
        assert!(is_barrel_pressed(Some(PEN_FLAG_BARREL), 0));
        assert!(is_barrel_pressed(Some(PEN_FLAG_BARREL), POINTER_FLAG_INCONTACT));
    }

    #[test]
    fn pointer_message_second_button_is_detected() {
        assert!(is_barrel_pressed(None, POINTER_FLAG_SECONDBUTTON));
        assert!(is_barrel_pressed(Some(0), POINTER_FLAG_SECONDBUTTON));
        assert!(is_barrel_pressed(Some(0), POINTER_FLAG_INCONTACT | POINTER_FLAG_SECONDBUTTON));
    }

    #[test]
    fn eraser_end_is_the_inverted_pen() {
        assert_eq!(is_eraser_end(Some(PEN_FLAG_INVERTED)), Some(true), "hovering eraser end");
        assert_eq!(is_eraser_end(Some(PEN_FLAG_INVERTED | PEN_FLAG_ERASER)), Some(true), "eraser end on the tablet");
        assert_eq!(is_eraser_end(Some(PEN_FLAG_ERASER)), Some(false), "an eraser button on the barrel is not the eraser end");
        assert_eq!(is_eraser_end(Some(0)), Some(false), "the tip");
        assert_eq!(is_eraser_end(Some(PEN_FLAG_BARREL)), Some(false), "the barrel button is not the eraser");
        assert_eq!(is_eraser_end(None), None, "not a pen");
    }

    /// `w_param` of a pointer message: pointer id 7 with these pointer flags.
    fn wp(flags: u32) -> usize {
        7 | ((flags as usize) << 16)
    }

    #[test]
    fn pen_state_follows_pointer_messages() {
        let pen = |flags: u32| move |id: u32| (id == 7).then_some(flags);
        let s = next_state(PenState::default(), WM_POINTERUPDATE, wp(0), pen(0));
        assert_eq!(s, PenState { barrel: false, end: Some(false), pen_id: Some(7), contact: None }, "the tip hovers");
        let s = next_state(s, WM_POINTERUPDATE, wp(0), pen(PEN_FLAG_INVERTED));
        assert_eq!(s.end, Some(true), "the eraser end hovers");
        let s = next_state(s, WM_POINTERDOWN, wp(POINTER_FLAG_INCONTACT), pen(PEN_FLAG_INVERTED | PEN_FLAG_ERASER));
        assert_eq!(s, PenState { barrel: false, end: Some(true), pen_id: Some(7), contact: Some(true) });
        let s = next_state(s, WM_POINTERUPDATE, wp(0), |_| None);
        assert_eq!((s.end, s.pen_id), (Some(true), Some(7)), "a mouse in between keeps the pen");
        let s = next_state(s, WM_POINTERUPDATE, wp(0), pen(PEN_FLAG_BARREL));
        assert_eq!(s, PenState { barrel: true, end: Some(false), pen_id: Some(7), contact: Some(true) }, "only a contact sets it");
        let s = next_state(s, WM_POINTERUP, wp(0), |_| panic!("up does not read the pen"));
        assert_eq!(s, PenState { barrel: false, end: Some(false), pen_id: Some(7), contact: Some(true) }, "lifting keeps the end");
        let s = next_state(s, WM_POINTERLEAVE, wp(0), |_| panic!("leave does not read the pen"));
        assert_eq!(s, PenState { contact: Some(true), ..PenState::default() }, "leaving range forgets the pen, not its contact");
        let mouse_move = 0x0200;
        assert_eq!(next_state(s, mouse_move, wp(0), pen(PEN_FLAG_INVERTED)), s, "other messages change nothing");
    }

    #[test]
    fn another_pointer_leaving_keeps_the_pen() {
        let held = PenState { barrel: true, end: Some(true), pen_id: Some(7), contact: None };
        let finger_leaves = 2 | ((POINTER_FLAG_INCONTACT as usize) << 16);
        assert_eq!(next_state(held, WM_POINTERLEAVE, finger_leaves, |_| panic!("leave does not read the pen")), held);
        assert_eq!(next_state(held, WM_POINTERLEAVE, wp(0), |_| None), PenState::default(), "the pen itself leaving");
        let unknown = PenState { barrel: true, end: None, pen_id: None, contact: None };
        assert_eq!(next_state(unknown, WM_POINTERLEAVE, finger_leaves, |_| None), PenState::default(), "no pen seen yet");
    }

    /// A pen entering range with its eraser end is known before it moves (`WM_POINTERENTER` can
    /// come well before the first hover update).
    #[test]
    fn entering_range_reads_the_pen_end() {
        let s = next_state(PenState::default(), WM_POINTERENTER, wp(0), |id| (id == 7).then_some(PEN_FLAG_INVERTED));
        assert_eq!(s, PenState { barrel: false, end: Some(true), pen_id: Some(7), contact: None });
        let s = next_state(s, WM_POINTERENTER, 9, |_| None);
        assert_eq!((s.end, s.pen_id), (Some(true), Some(7)), "a mouse or finger entering keeps the pen");
    }

    /// Serialises the tests that touch the global state the hooks write (tests run in parallel).
    static GLOBALS: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn handled_messages_reach_the_readers() {
        let _globals = GLOBALS.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        handle_pointer_message(WM_POINTERUPDATE, wp(0), |_| Some(PEN_FLAG_INVERTED | PEN_FLAG_BARREL));
        assert_eq!((pen_end(), is_barrel_button_down()), (Some(true), true));
        handle_pointer_message(WM_POINTERUP, wp(0), |_| None);
        assert_eq!((pen_end(), is_barrel_button_down()), (Some(true), false));
        handle_pointer_message(WM_POINTERUPDATE, wp(0), |_| Some(0));
        assert_eq!(pen_end(), Some(false));
        handle_pointer_message(WM_POINTERLEAVE, 2, |_| None);
        assert_eq!(pen_end(), Some(false), "a finger leaving keeps the pen");
        handle_pointer_message(WM_POINTERLEAVE, wp(0), |_| None);
        assert_eq!((pen_end(), is_barrel_button_down()), (None, false));
        assert_eq!(take_pen_end(), None, "no contact to report");
    }

    /// An eraser-end tap and the pen leaving range, all handled before the UI's next frame: the
    /// frame still reads the eraser end once, so the queued click erases.
    #[test]
    fn a_contact_queued_past_the_leave_keeps_its_end() {
        let _globals = GLOBALS.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        handle_pointer_message(WM_POINTERLEAVE, wp(0), |_| None);
        let _ = take_pen_end();
        handle_pointer_message(WM_POINTERDOWN, wp(POINTER_FLAG_INCONTACT), |_| Some(PEN_FLAG_INVERTED | PEN_FLAG_ERASER));
        handle_pointer_message(WM_POINTERUP, wp(0), |_| None);
        handle_pointer_message(WM_POINTERLEAVE, wp(0), |_| None);
        assert_eq!(pen_end(), None, "the pen is out of range");
        assert_eq!(take_pen_end(), Some(true), "the queued contact was the eraser end");
        assert_eq!(take_pen_end(), None, "reported once");
        handle_pointer_message(WM_POINTERDOWN, wp(POINTER_FLAG_INCONTACT), |_| Some(PEN_FLAG_INVERTED));
        handle_pointer_message(WM_POINTERUP, wp(0), |_| None);
        handle_pointer_message(WM_POINTERUPDATE, wp(0), |_| Some(0));
        assert_eq!(take_pen_end(), Some(true), "the unread contact comes before the tip hovering since");
        assert_eq!(take_pen_end(), Some(false), "then the end in range");
        handle_pointer_message(WM_POINTERLEAVE, wp(0), |_| None);
    }

    #[test]
    fn a_failed_leave_hook_keeps_the_message_hook() {
        let _globals = GLOBALS.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut installed = Vec::new();
        let monitor = Monitor::install_with(|id_hook, proc_| {
            installed.push(id_hook);
            if id_hook == WH_CALLWNDPROC {
                return std::ptr::null_mut();
            }
            // SAFETY: a real thread-specific hook on this test thread; the monitor unhooks it.
            unsafe { SetWindowsHookExW(id_hook, proc_, std::ptr::null_mut(), GetCurrentThreadId()) }
        })
        .expect("the message hook alone is enough");
        assert_eq!(installed, [WH_GETMESSAGE, WH_CALLWNDPROC]);
        assert!(!monitor.hooks[0].is_null());
        assert!(!monitor.tracks_leave());
    }

    #[test]
    fn a_failed_message_hook_is_an_error() {
        let mut installed = Vec::new();
        let r = Monitor::install_with(|id_hook, _| {
            installed.push(id_hook);
            std::ptr::null_mut()
        });
        assert!(matches!(r, Err(crate::Error::Platform(why)) if why.contains("WH_GETMESSAGE")));
        assert_eq!(installed, [WH_GETMESSAGE], "no leave hook without the message hook");
    }
}
