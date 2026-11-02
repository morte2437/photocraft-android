//! Windows pointer hook for tablet data dropped by winit (such as the pen barrel button).
//!
//! winit 0.30 drops `POINTER_PEN_INFO::penFlags` (`PEN_FLAG_BARREL`) on Windows and turns pen
//! inputs into `WindowEvent::Touch`, mapping them to `PointerButton::Primary`. Windows Ink also
//! does not update `GetAsyncKeyState(VK_RBUTTON)` for stylus barrel buttons. This thread-local
//! `WH_GETMESSAGE` hook inspects `WM_POINTER*` messages on the UI thread's message queue to
//! track the physical barrel button state.

#![allow(unsafe_code, clippy::upper_case_acronyms)]

use std::sync::atomic::{AtomicBool, Ordering};

static BARREL_DOWN: AtomicBool = AtomicBool::new(false);

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
const WM_POINTERUPDATE: u32 = 0x0245;
const WM_POINTERDOWN: u32 = 0x0246;
const WM_POINTERUP: u32 = 0x0247;
const WM_POINTERLEAVE: u32 = 0x024A;

/// `PEN_FLAG_BARREL` (winuser.h): the barrel button of the pen is pressed.
pub const PEN_FLAG_BARREL: u32 = 0x00000001;

/// `POINTER_MESSAGE_FLAG_SECONDBUTTON` (winuser.h): the secondary action button is pressed.
pub const POINTER_FLAG_SECONDBUTTON: u32 = 0x00000020;

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

unsafe extern "system" fn get_msg_proc(code: i32, w_param: usize, l_param: isize) -> isize {
    if code >= 0 && l_param != 0 {
        // SAFETY: Win32 guarantees that for code >= 0, l_param points to a valid MSG struct
        // dispatched to this thread's message queue.
        let msg = unsafe { &*(l_param as *const MSG) };
        match msg.message {
            WM_POINTERDOWN | WM_POINTERUPDATE => {
                let pointer_id = (msg.w_param & 0xFFFF) as u32;
                let pointer_message_flags = (msg.w_param >> 16) as u32;
                // SAFETY: `pen_info` is a stack-allocated buffer initialized to zero, and `pointer_id`
                // is the pointer identifier from the valid WM_POINTER message.
                let pen_flags = unsafe {
                    let mut info: POINTER_PEN_INFO = std::mem::zeroed();
                    (GetPointerPenInfo(pointer_id, &mut info) != 0).then_some(info.pen_flags)
                };
                let is_barrel = is_barrel_pressed(pen_flags, pointer_message_flags);
                BARREL_DOWN.store(is_barrel, Ordering::Relaxed);
            }
            WM_POINTERUP | WM_POINTERLEAVE => {
                BARREL_DOWN.store(false, Ordering::Relaxed);
            }
            _ => {}
        }
    }
    // SAFETY: CallNextHookEx passes the parameters down the hook chain. MSDN notes the first
    // parameter is ignored and can be null.
    unsafe { CallNextHookEx(std::ptr::null_mut(), code, w_param, l_param) }
}

/// Keeps the thread-local pointer message hook installed; dropping it removes the hook.
/// Main-thread only (not `Send`).
pub struct Monitor {
    hook: *mut std::ffi::c_void,
}

impl Monitor {
    /// Installs the thread-local pointer message hook on the UI thread.
    pub fn install() -> Result<Self, crate::Error> {
        // SAFETY: GetCurrentThreadId has no preconditions. SetWindowsHookExW installs
        // a thread-specific WH_GETMESSAGE hook on the calling thread's message queue
        // using our extern "system" callback.
        let hook = unsafe {
            let thread_id = GetCurrentThreadId();
            SetWindowsHookExW(WH_GETMESSAGE, get_msg_proc, std::ptr::null_mut(), thread_id)
        };
        if hook.is_null() {
            return Err(crate::Error::Platform("SetWindowsHookExW returned null".into()));
        }
        Ok(Self { hook })
    }
}

impl Drop for Monitor {
    fn drop(&mut self) {
        // SAFETY: `self.hook` is the non-null handle returned by `SetWindowsHookExW`,
        // unhooked exactly once here on the thread that created it.
        unsafe {
            UnhookWindowsHookEx(self.hook);
        }
        BARREL_DOWN.store(false, Ordering::Relaxed);
    }
}

/// Returns true if the stylus barrel button is physically pressed.
pub fn is_barrel_button_down() -> bool {
    BARREL_DOWN.load(Ordering::Relaxed)
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
}
