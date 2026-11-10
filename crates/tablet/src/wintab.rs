//! WinTab 1.4 backend for Windows.
//!
//! The app opens a WinTab context on its real top-level window and polls queued packets once per
//! frame. This module only translates Wintab packets into [`Sample`]s; it does not synthesise
//! pointer events. With Windows Ink disabled for the tablet, the vendor driver provides the
//! mouse cursor while this module provides pressure, tilt, rotation and the eraser flag.
//!
//! Devices without orientation axes (pressure-only tablets) are opened with a packet that asks
//! only for the fields the driver reports; their tilt is neutral rather than a made-up value.
//!
//! The context is thread-local because Wintab contexts belong to the thread and window that
//! opened them. [`install`] and [`poll`] must therefore run on the UI thread.

use std::cell::RefCell;
use std::ffi::c_void;
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};

use crate::{Error, Sample};

wintab::packet! {
    struct PenPacket { status, buttons, x, y, normal_pressure, orientation }
}

wintab::packet! {
    struct PressurePacket { status, buttons, x, y, normal_pressure }
}

/// The open context. A device that reports orientation gets tilt and rotation; a pressure-only
/// device gets a packet without the orientation field, so a zeroed field never becomes a tilt.
enum Backend {
    Oriented { context: wintab::Context<PenPacket>, axes: [wintab::Axis; 3] },
    PressureOnly { context: wintab::Context<PressurePacket> },
}

struct State {
    backend: Backend,
    pressure_axis: wintab::Axis,
    last_packet: Instant,
    emitted: bool,
    focused: bool,
    /// The pen is inside the tablet's sensing range (`WT_PROXIMITY`, hardware transitions).
    proximity_in: bool,
    /// Requests a frame when a WinTab message arrives, so pressure-only strokes update without
    /// cursor movement.
    wake: Box<dyn Fn()>,
}

thread_local! {
    static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
}

/// Opens a WinTab system context on `hwnd`.
///
/// Call on the UI thread. A previous context on this thread is replaced, so the caller keeps the
/// app's normal single-window lifetime. The context is polled with [`poll`]. `wake` is called from
/// the thread's message hook for every WinTab message: a frame is then requested even when the
/// pen only changes pressure (no cursor movement, no other input event).
pub fn install(hwnd: isize, wake: Box<dyn Fn()>) -> Result<(), Error> {
    if hwnd == 0 {
        return Err(Error::Unsupported("no window handle".into()));
    }
    let wintab = wintab::Wintab::load().map_err(|e| Error::Platform(e.to_string()))?;
    let device = wintab.device(0).ok_or_else(|| Error::Unsupported("no Wintab digitizer".into()))?;
    let pressure_axis = device.pressure.ok_or_else(|| Error::Unsupported("Wintab device has no pressure axis".into()))?;
    let hwnd = HWND(hwnd as *mut c_void);
    let options = wintab::ContextOptions::new().system(true).name("PhotoCraft WinTab");
    // Ask only for the fields the device reports: a pressure-only tablet never fills
    // orientation, and reading a zeroed orientation would invent a maximum tilt.
    let backend = match device.orientation {
        Some(axes) => {
            let context = wintab.open::<PenPacket>(hwnd, &options).map_err(|e| Error::Platform(e.to_string()))?;
            if let Err(error) = context.overlap(true) {
                log::warn!("WinTab: could not take overlap priority: {error}");
            }
            Backend::Oriented { context, axes }
        }
        None => {
            let context = wintab.open::<PressurePacket>(hwnd, &options).map_err(|e| Error::Platform(e.to_string()))?;
            if let Err(error) = context.overlap(true) {
                log::warn!("WinTab: could not take overlap priority: {error}");
            }
            Backend::PressureOnly { context }
        }
    };
    STATE.with_borrow_mut(|slot| {
        *slot = Some(State { backend, pressure_axis, last_packet: Instant::now(), emitted: false, focused: false, proximity_in: true, wake });
    });
    Ok(())
}

/// Drains queued packets and reports the newest sample.
///
/// Reports `Some` while a pen is in use, `None` once the pen has left the tablet. Leaving is the
/// packet's proximity status, not a quiet period: a pen held still may send no packets for a
/// while, and treating that as left painted a held airbrush build-up at mouse pressure. A long
/// timeout remains only as a fallback for drivers that never report the leave.
///
/// Coordinates in the packet are not used: with Windows Ink off, the vendor driver moves the
/// normal cursor, and PhotoCraft's existing pointer path supplies the position.
///
/// `focused` is whether the app's window has focus. Another tablet application can take the top
/// WinTab overlap priority; when PhotoCraft is activated again it has to ask for it back. The
/// window procedure belongs to winit, so focus from the app's input is the activation signal.
pub fn poll(focused: bool, callback: &dyn Fn(Option<Sample>)) {
    STATE.with_borrow_mut(|slot| {
        let Some(state) = slot.as_mut() else { return };
        if focused && !state.focused {
            let result = match &state.backend {
                Backend::Oriented { context, .. } => context.overlap(true),
                Backend::PressureOnly { context } => context.overlap(true),
            };
            if let Err(error) = result {
                log::warn!("WinTab: could not take overlap priority: {error}");
            }
        } else if !focused && state.focused {
            let result = match &state.backend {
                Backend::Oriented { context, .. } => context.overlap(false),
                Backend::PressureOnly { context } => context.overlap(false),
            };
            if let Err(error) = result {
                log::warn!("WinTab: could not release overlap priority: {error}");
            }
        }
        state.focused = focused;
        let State { backend, pressure_axis, last_packet, emitted, proximity_in, .. } = state;
        match backend {
            Backend::Oriented { context, axes } => {
                let mut packets = Vec::new();
                context.drain(&mut packets);
                for packet in packets {
                    if packet.status.out_of_context() {
                        if *emitted {
                            *emitted = false;
                            callback(None);
                        }
                        continue;
                    }
                    let sample = sample(&packet, pressure_axis, axes);
                    *last_packet = Instant::now();
                    *emitted = true;
                    callback(Some(sample));
                }
            }
            Backend::PressureOnly { context } => {
                let mut packets = Vec::new();
                context.drain(&mut packets);
                for packet in packets {
                    if packet.status.out_of_context() {
                        if *emitted {
                            *emitted = false;
                            callback(None);
                        }
                        continue;
                    }
                    let sample = sample_pressure(&packet, pressure_axis);
                    *last_packet = Instant::now();
                    *emitted = true;
                    callback(Some(sample));
                }
            }
        }
        if !*proximity_in && *emitted {
            *emitted = false;
            callback(None);
        }
        if *emitted && last_packet.elapsed() >= Duration::from_secs(60) {
            *emitted = false;
            callback(None);
        }
    });
}

/// Handles a WinTab window message from the thread's message hook (`windows.rs`).
///
/// A hardware proximity leave is a `WT_PROXIMITY` message, not a packet status: the context's
/// `TPS_PROXIMITY` status only says the cursor left the context. Clearing the feed here covers
/// drivers that send the hardware leave without a following out-of-context packet.
pub(crate) fn on_message(message: u32, wparam: usize, lparam: isize) {
    // This runs inside the Win32 hook, where a panic aborts the app: a driver call under `poll`
    // that pumps this thread's messages would find `STATE` borrowed. Such a message is skipped.
    let _ = STATE.try_with(|cell| {
        let Ok(mut slot) = cell.try_borrow_mut() else { return };
        let Some(state) = slot.as_mut() else { return };
        let decoded = match &state.backend {
            Backend::Oriented { context, .. } => context.message(message, WPARAM(wparam), LPARAM(lparam)),
            Backend::PressureOnly { context } => context.message(message, WPARAM(wparam), LPARAM(lparam)),
        };
        let win_message = decoded.is_some();
        if let Some(wintab::Message::Proximity { entering, hardware }) = decoded
            && hardware
        {
            state.proximity_in = entering;
        }
        if win_message {
            // Packets can arrive with no cursor movement; the frame then polls them.
            (state.wake)();
        }
    });
}

/// Closes the context on this thread.
pub fn uninstall() {
    STATE.with_borrow_mut(|slot| *slot = None);
}

/// One packet from a device with orientation axes as the UI's pen sample.
///
/// Angles come from the device's own axes (`TU_CIRCLE`), falling back to Wintab's usual tenths
/// of a degree when an axis does not measure angles.
fn sample(packet: &PenPacket, pressure: &wintab::Axis, orientation: &[wintab::Axis; 3]) -> Sample {
    let raw = packet.orientation;
    let degrees = |index: usize, value: i32| orientation[index].degrees(value).map_or(value as f32 / 10.0, |degrees| degrees as f32);
    let azimuth = degrees(0, raw.azimuth).to_radians();
    // Wintab altitude is the angle from the tablet plane (0 = flat, 90 = upright); W3C tilt is
    // the angle from the perpendicular, per axis: atan2 of the in-plane projection over height.
    // Wintab azimuth 0 points at the tablet's top and grows clockwise, while W3C +x is right and
    // +y towards the user, so the in-plane components are (sin(azimuth), -cos(azimuth)).
    let elevation = degrees(1, raw.altitude).abs().to_radians();
    let in_plane_x = elevation.cos() * azimuth.sin();
    let in_plane_y = -(elevation.cos() * azimuth.cos());
    let height = elevation.sin();
    Sample {
        pressure: pressure.normalize(packet.normal_pressure as i32),
        tilt_x: in_plane_x.atan2(height).to_degrees(),
        tilt_y: in_plane_y.atan2(height).to_degrees(),
        rotation: degrees(2, raw.twist),
        eraser: packet.status.inverted(),
    }
    .sanitized()
}

/// One packet from a device without orientation axes: pressure and the eraser end, neutral tilt.
fn sample_pressure(packet: &PressurePacket, pressure: &wintab::Axis) -> Sample {
    Sample { pressure: pressure.normalize(packet.normal_pressure as i32), tilt_x: 0.0, tilt_y: 0.0, rotation: 0.0, eraser: packet.status.inverted() }
        .sanitized()
}

#[cfg(test)]
mod tests {
    use super::*;
    use wintab::Units;

    fn pressure_axis() -> wintab::Axis {
        wintab::Axis { min: 0, max: 32767, units: Units::None, resolution: 0.0 }
    }

    fn angle_axis() -> wintab::Axis {
        wintab::Axis { min: 0, max: 3599, units: Units::Circle, resolution: 3600.0 }
    }

    fn axes() -> [wintab::Axis; 3] {
        [angle_axis(), angle_axis(), angle_axis()]
    }

    fn packet(normal_pressure: u32) -> PenPacket {
        PenPacket { status: wintab::Status(0), buttons: wintab::Buttons(0), x: 0, y: 0, normal_pressure, orientation: wintab::Orientation::default() }
    }

    fn pressure_packet(normal_pressure: u32) -> PressurePacket {
        PressurePacket { status: wintab::Status(0), buttons: wintab::Buttons(0), x: 0, y: 0, normal_pressure }
    }

    /// Pressure is normalized through the device's own axis.
    #[test]
    fn pressure_maps_through_the_device_axis() {
        let axes = axes();
        let s = sample(&packet(16384), &pressure_axis(), &axes);
        assert!((s.pressure - 0.5).abs() < 0.01, "{}", s.pressure);
        let s = sample(&packet(0), &pressure_axis(), &axes);
        assert_eq!(s.pressure, 0.0);
    }

    /// The inverted status bit is the eraser end.
    #[test]
    fn eraser_comes_from_the_inverted_status() {
        let axes = axes();
        let inverted = PenPacket { status: wintab::Status(wintab::sys::TPS_INVERT), ..packet(0) };
        assert!(sample(&inverted, &pressure_axis(), &axes).eraser);
        assert!(!sample(&packet(0), &pressure_axis(), &axes).eraser);
    }

    /// Orientation axes are converted to degrees for the W3C tilt projection.
    #[test]
    fn orientation_axes_report_degrees() {
        let axes = axes();
        let tilted = PenPacket { orientation: wintab::Orientation { azimuth: 450, altitude: 300, twist: 900 }, ..packet(0) };
        let sample = sample(&tilted, &pressure_axis(), &axes);
        assert!((sample.tilt_x - 50.8).abs() < 0.1, "{}", sample.tilt_x);
        assert!((sample.tilt_y + 50.8).abs() < 0.1, "{}", sample.tilt_y);
        assert!((sample.rotation - 90.0).abs() < 0.1, "{}", sample.rotation);
    }

    /// A pressure-only device keeps pressure and the eraser end, with neutral tilt.
    #[test]
    fn pressure_only_devices_get_pressure_and_neutral_tilt() {
        let s = sample_pressure(&pressure_packet(16384), &pressure_axis());
        assert!((s.pressure - 0.5).abs() < 0.01, "{}", s.pressure);
        assert_eq!((s.tilt_x, s.tilt_y, s.rotation), (0.0, 0.0, 0.0));
        let inverted = PressurePacket { status: wintab::Status(wintab::sys::TPS_INVERT), ..pressure_packet(0) };
        assert!(sample_pressure(&inverted, &pressure_axis()).eraser);
    }

    /// Wintab azimuth 0 is the tablet top; W3C axes are right and toward the user.
    #[test]
    fn cardinal_azimuths_match_w3c_tilt() {
        let axes = axes();
        let tilt = |azimuth: i32| {
            let p = PenPacket { orientation: wintab::Orientation { azimuth, altitude: 450, twist: 0 }, ..packet(0) };
            sample(&p, &pressure_axis(), &axes)
        };
        let top = tilt(0);
        assert!(top.tilt_x.abs() < 0.1 && (top.tilt_y + 45.0).abs() < 0.1, "{top:?}");
        let right = tilt(900);
        assert!((right.tilt_x - 45.0).abs() < 0.1 && right.tilt_y.abs() < 0.1, "{right:?}");
        let bottom = tilt(1800);
        assert!(bottom.tilt_x.abs() < 0.1 && (bottom.tilt_y - 45.0).abs() < 0.1, "{bottom:?}");
        let left = tilt(2700);
        assert!((left.tilt_x + 45.0).abs() < 0.1 && left.tilt_y.abs() < 0.1, "{left:?}");
    }

    /// An upright pen has no tilt; a flat one has the most.
    #[test]
    fn an_upright_pen_has_no_tilt_and_a_flat_one_has_the_most() {
        let axes = axes();
        let upright = PenPacket { orientation: wintab::Orientation { azimuth: 0, altitude: 900, twist: 0 }, ..packet(0) };
        let s = sample(&upright, &pressure_axis(), &axes);
        assert!(s.tilt_x.abs() < 0.1 && s.tilt_y.abs() < 0.1, "{s:?}");
        let flat = PenPacket { orientation: wintab::Orientation { azimuth: 0, altitude: 0, twist: 0 }, ..packet(0) };
        let s = sample(&flat, &pressure_axis(), &axes);
        assert!(s.tilt_x.abs() < 0.1 && (s.tilt_y + 90.0).abs() < 0.1, "{s:?}");
    }
}
