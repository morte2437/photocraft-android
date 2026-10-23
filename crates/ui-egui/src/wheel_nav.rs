//! Mouse wheel and trackpad navigation over the canvas (#293, #635), as in Photoshop:
//!
//! - The wheel scrolls up and down; ⇧ or ⌘/Ctrl + wheel scrolls sideways.
//! - ⌥/Alt + wheel zooms around the pointer, 10% per wheel notch, applied the frame it arrives.
//!   ⇧/Shift + ⌥/Alt + wheel snaps to the next zoom ladder level instead.
//! - Preferences › General › Zoom with Scroll Wheel swaps the two: the wheel zooms and
//!   ⌥/Alt + wheel scrolls; ⇧/Shift + wheel snaps to the next zoom ladder level.
//! - A trackpad pinch zooms around the pointer, unless Preferences › Enhanced Controls ›
//!   Zoom with Trackpad Pinch is off (then a pinch does nothing, and Windows' Ctrl + wheel
//!   fractions scroll sideways like the wheel, because the fold below is skipped).
//! - A two-finger pinch on a touch screen zooms the same way, unless Preferences › Enhanced
//!   Controls › Enable Gestures is off (#204): then the fingers leave the view alone, and a
//!   trackpad pinch still zooms ([`pinch_factor`]).
//!
//! [`configure`] makes egui fold ⌘/Ctrl + wheel into a sideways scroll as it does ⇧ + wheel, so
//! only a real pinch (egui's `Event::Zoom`, or a touch screen) reaches `zoom_delta`. macOS and
//! browsers deliver a pinch as its own event, but Windows hands a precision touchpad pinch to apps
//! as Ctrl + wheel in fractions of a notch (also on some Linux Wayland systems):
//! [`fold_legacy_pinch`] turns those back into a pinch
//! before egui sees them, so Ctrl + a notched mouse wheel scrolls sideways and the pinch still
//! zooms.
//!
//! The target behaviour is exact ×1.1 steps, one per notch, with no easing in between
//! (measured frame by frame on a screen recording). egui smooths a wheel notch over several
//! frames, so zooming reads the wheel events themselves and ignores that smoothed tail; panning
//! keeps the smoothed delta. The Alt state is taken from the wheel events (and the Alt key's own
//! press and release, `wheel_events`) and kept until the next wheel event: releasing Alt while
//! the tail is still settling must not turn it into a pan.

use egui::{Context, Event, Id, InputState, Key, Modifiers, MouseWheelUnit, RawInput, Vec2};

/// Zoom factor of one wheel notch: every step is exactly ×1.1.
pub const NOTCH: f32 = 1.1;

/// A Ctrl + wheel event this soon (seconds) after a pinch step still belongs to the pinch, even
/// when it happens to be a whole notch.
const PINCH_GAP: f64 = 0.25;

/// What the wheel does to the view this frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Wheel {
    /// Multiply the zoom by this factor around the pointer.
    Zoom(f32),
    /// Step N rungs on the zoom ladder (positive = in, negative = out).
    SnapZoom(i32),
    /// Move the image by this many screen points (egui's content direction).
    Pan(Vec2),
}

/// Inputs of one frame's wheel gesture.
#[derive(Clone, Copy, Debug)]
pub struct Input {
    /// Smoothed scroll delta (points, content direction); what panning uses.
    pub scroll: Vec2,
    /// This frame's wheel events for the gesture in progress, unsmoothed (points, content
    /// direction); what zooming uses.
    pub raw: Vec2,
    /// egui's pinch zoom factor (1 = none).
    pub zoom_delta: f32,
    /// ⌥/Alt was held for the wheel gesture in progress.
    pub alt: bool,
    /// ⇧/Shift was held for the wheel gesture in progress (snap zoom).
    pub shift: bool,
    /// Preferences › General › Zoom with Scroll Wheel.
    pub zoom_with_wheel: bool,
    /// Preferences › Enhanced Controls › Zoom with Trackpad Pinch.
    pub pinch_zooms: bool,
    /// Points per wheel notch (egui's `line_scroll_speed`).
    pub notch: f32,
}

/// The view change for one frame of wheel input, if any.
pub fn classify(i: Input) -> Option<Wheel> {
    if i.pinch_zooms && i.zoom_delta.is_finite() && i.zoom_delta > 0.0 && i.zoom_delta != 1.0 {
        return Some(Wheel::Zoom(i.zoom_delta));
    }
    // One notch (`notch` points) is one ×1.1 step.
    let step = |dy: f32| {
        let notch = if i.notch.is_finite() && i.notch > 0.0 { i.notch } else { 40.0 };
        let f = NOTCH.powf(dy / notch);
        (f.is_finite() && f > 0.0 && f != 1.0).then_some(Wheel::Zoom(f))
    };
    let snap = |dy: f32| {
        if dy == 0.0 {
            return None;
        }
        let notch = if i.notch.is_finite() && i.notch > 0.0 { i.notch } else { 40.0 };
        let n = dy / notch;
        if !n.is_finite() {
            return None;
        }
        let steps = n.clamp(-10.0, 10.0).round() as i32;
        if steps == 0 { None } else { Some(Wheel::SnapZoom(steps)) }
    };
    match (i.alt, i.zoom_with_wheel) {
        // ⇧ + ⌥ + scroll: snap to the zoom ladder instead of continuous ×1.1.
        (true, false) if i.shift => snap(i.raw.y + i.raw.x),
        // ⌥ + scroll: continuous ×1.1 zoom. The smoothed tail of a notch is ignored.
        (true, false) => step(i.raw.y + i.raw.x),
        // ⇧ + scroll wheel with the preference on: snap to the zoom ladder.
        // Some systems deliver Shift+wheel as horizontal (delta in x, y = 0).
        (false, true) if i.shift => snap(i.raw.y + i.raw.x),
        // ⌘/Ctrl folds the wheel into x: those still scroll sideways.
        (false, true) if i.raw.y != 0.0 || i.scroll.y != 0.0 => step(i.raw.y),
        _ if !(i.scroll.x.is_finite() && i.scroll.y.is_finite()) || i.scroll == Vec2::ZERO => None,
        _ => Some(Wheel::Pan(i.scroll)),
    }
}

/// Set egui's wheel modifiers (once, at setup): ⌘/Ctrl + wheel scrolls sideways like ⇧ + wheel
/// instead of zooming, everywhere in the app.
pub fn configure(ctx: &Context) {
    ctx.options_mut(|o| {
        o.input_options.zoom_modifier = Modifiers::NONE;
        o.input_options.horizontal_scroll_modifier = Modifiers::SHIFT | Modifiers::COMMAND;
    });
}

/// Turn a Windows or Linux touchpad pinch, which arrives as Ctrl + wheel in fractions of a notch or points, into
/// pinch events (`Event::Zoom`) at egui's Ctrl-scroll zoom speed (`notch` points per line,
/// `speed` per point). Ctrl + wheel in whole notches (a mouse wheel) is left to scroll sideways,
/// unless it comes within [`PINCH_GAP`] of a pinch step. `last` is the time of the previous pinch
/// step; returns the time of the latest one.
pub fn legacy_pinch_to_zoom(events: &mut [Event], now: f64, last: Option<f64>, notch: f32, speed: f32) -> Option<f64> {
    let ctrl_wheel = |e: &Event| match e {
        Event::MouseWheel { unit: MouseWheelUnit::Line, delta, modifiers, .. } if modifiers.ctrl => Some((*delta, false)),
        // Wayland precision touchpads can use point units. These are already actual
        // screen points, not line notches; normalize before the exponential below.
        Event::MouseWheel { unit: MouseWheelUnit::Point, delta, modifiers, .. } if modifiers.ctrl => Some((*delta / notch.max(1.0), true)),
        _ => None,
    };
    let fraction = |d: Vec2| d.x.fract() != 0.0 || d.y.fract() != 0.0;
    let pinching =
        last.is_some_and(|t| (0.0..PINCH_GAP).contains(&(now - t))) || events.iter().filter_map(ctrl_wheel).any(|(d, high_res)| high_res || fraction(d));
    if !pinching {
        return last;
    }
    let mut latest = last;
    for e in events.iter_mut() {
        if let Some((d, _)) = ctrl_wheel(e) {
            let f = (speed * notch * (d.x + d.y)).exp();
            *e = Event::Zoom(if f.is_finite() && f > 0.0 { f } else { 1.0 });
            latest = Some(now);
        }
    }
    latest
}

fn pinch_id() -> Id {
    Id::new("pc-wheel-legacy-pinch")
}

/// [`legacy_pinch_to_zoom`] for one frame's raw input (`raw_input_hook`, Windows and Linux).
pub fn fold_legacy_pinch(ctx: &Context, raw: &mut RawInput) {
    // Native gesture events already carry their own magnification. Some backends
    // also emit a Ctrl-wheel event in the same frame: folding that wheel event
    // would apply the same pinch twice. End any legacy continuation as well,
    // so a later real Ctrl+wheel notch isn't treated as part of that pinch.
    if raw.events.iter().any(|event| matches!(event, Event::Zoom(_))) {
        ctx.data_mut(|d| d.remove::<f64>(pinch_id()));
        return;
    }
    let (notch, speed) = ctx.options(|o| (o.input_options.line_scroll_speed, o.input_options.scroll_zoom_speed));
    let now = raw.time.unwrap_or_else(|| ctx.input(|i| i.time));
    let last = ctx.data(|d| d.get_temp::<f64>(pinch_id()));
    let latest = legacy_pinch_to_zoom(&mut raw.events, now, last, notch, speed);
    if latest != last
        && let Some(t) = latest
    {
        ctx.data_mut(|d| d.insert_temp(pinch_id(), t));
    }
}

fn alt_id() -> Id {
    Id::new("pc-wheel-alt")
}

fn shift_id() -> Id {
    Id::new("pc-wheel-shift")
}

fn snap_residual_id(owner: Id) -> Id {
    Id::new("pc-wheel-snap-residual").with(owner)
}

fn snap_time_id(owner: Id) -> Id {
    Id::new("pc-wheel-snap-time").with(owner)
}

fn snap_frame_result_id(owner: Id) -> Id {
    Id::new("pc-wheel-snap-frame-result").with(owner)
}

fn active_snap_owner_id() -> Id {
    Id::new("pc-wheel-active-snap-owner")
}

const SNAP_IDLE_TIMEOUT: f64 = 0.5;

/// Read this frame's wheel input. Call it every frame (hovered or not) so the Alt and Shift
/// states of the gesture follow the latest wheel event. `owner` scopes the snap-zoom
/// accumulator per viewport. `hovered` gates accumulation: only the viewport receiving
/// navigation may add raw input to the snap residual.
pub fn read(ctx: &Context, zoom_with_wheel: bool, pinch_zooms: bool, owner: Id, hovered: bool) -> Option<Wheel> {
    read_with(ctx, zoom_with_wheel, pinch_zooms, owner, hovered, true, true)
}

/// [`read`], choosing how a plain wheel scroll pans: `smooth` glides it over a few frames (egui's
/// smoothed delta); off, as with Interface › Enable Animations off, the view moves the whole notch
/// in the frame the wheel turns and stops there (#5721), as scroll areas already do.
/// `touch_gestures` is Preferences › Enhanced Controls › Enable Gestures: off, a touch screen's
/// two-finger pinch doesn't zoom ([`pinch_factor`]).
pub fn read_with(ctx: &Context, zoom_with_wheel: bool, pinch_zooms: bool, owner: Id, hovered: bool, smooth: bool, touch_gestures: bool) -> Option<Wheel> {
    let notch = ctx.options(|o| o.input_options.line_scroll_speed);

    // --- Modifier tracking (always runs for all callers) ---
    let (scroll, zoom_delta, latest_alt, latest_shift, wheel, page) = ctx.input(|i| {
        let wheel = wheel_events(i);
        let latest_alt = wheel.last().map(|(_, _, m)| m.alt && !m.command && !m.ctrl);
        let latest_shift = wheel.last().map(|(_, _, m)| m.shift && !m.command && !m.ctrl);
        (i.smooth_scroll_delta, pinch_factor(i, touch_gestures), latest_alt, latest_shift, wheel, i.content_rect().height())
    });
    let alt = match latest_alt {
        Some(a) => {
            ctx.data_mut(|d| d.insert_temp(alt_id(), a));
            a
        }
        None => ctx.data(|d| d.get_temp::<bool>(alt_id())).unwrap_or(false),
    };
    let shift = match latest_shift {
        Some(s) => {
            ctx.data_mut(|d| d.insert_temp(shift_id(), s));
            s
        }
        None => ctx.data(|d| d.get_temp::<bool>(shift_id())).unwrap_or(false),
    };

    if !hovered {
        return None;
    }

    // --- Input classification (hovered viewport only) ---
    // The wheel events that belong to the gesture in progress: Shift+Alt (snap zoom), Alt-only
    // (continuous zoom), Shift-only (snap zoom with zoom_with_wheel), or plain (scroll/zoom).
    let raw = wheel
        .iter()
        .filter(|(_, _, m)| {
            if alt && shift {
                m.alt && m.shift && !m.command && !m.ctrl
            } else if alt {
                m.alt && !m.shift && !m.command && !m.ctrl
            } else if shift {
                m.shift && !m.alt && !m.command && !m.ctrl
            } else {
                !m.alt && !m.shift && !m.command && !m.ctrl
            }
        })
        .fold(Vec2::ZERO, |sum, (unit, delta, _)| {
            sum + *delta
                * match unit {
                    MouseWheelUnit::Point => 1.0,
                    MouseWheelUnit::Line => notch,
                    MouseWheelUnit::Page => page,
                }
        });
    let snap_mode = shift && ((alt && !zoom_with_wheel) || (!alt && zoom_with_wheel));
    let has_pinch = pinch_zooms && zoom_delta.is_finite() && zoom_delta > 0.0 && zoom_delta != 1.0;

    if snap_mode && !has_pinch {
        let dy = raw.y + raw.x;
        if dy == 0.0 {
            return None;
        }

        // Per-frame idempotency: a second read in the same frame returns the cached result.
        let frame_nr = ctx.cumulative_pass_nr();
        if let Some((cached_frame, cached_steps)) = ctx.data(|d| d.get_temp::<(u64, i32)>(snap_frame_result_id(owner)))
            && cached_frame == frame_nr
        {
            return if cached_steps == 0 { None } else { Some(Wheel::SnapZoom(cached_steps)) };
        }

        // When the receiving viewport changes, clear the old viewport's residual so a
        // partial gesture doesn't reappear when returning to it.
        let prev_active = ctx.data(|d| d.get_temp::<Id>(active_snap_owner_id()));
        if let Some(old) = prev_active.filter(|&p| p != owner) {
            ctx.data_mut(|d| {
                d.remove::<f32>(snap_residual_id(old));
                d.remove::<f64>(snap_time_id(old));
            });
        }
        ctx.data_mut(|d| d.insert_temp(active_snap_owner_id(), owner));

        let now = ctx.input(|i| i.time);
        let last_time = ctx.data(|d| d.get_temp::<f64>(snap_time_id(owner)));
        let prev = if last_time.is_some_and(|t| now - t < SNAP_IDLE_TIMEOUT && (now - t).is_finite()) {
            ctx.data(|d| d.get_temp::<f32>(snap_residual_id(owner))).unwrap_or(0.0)
        } else {
            0.0
        };
        let notch_val = if notch.is_finite() && notch > 0.0 { notch } else { 40.0 };
        let total = prev + dy;
        let n = if total.is_finite() { (total / notch_val).trunc() } else { 0.0 };
        let remainder = total - n * notch_val;
        let steps = n.clamp(-10.0, 10.0) as i32;
        ctx.data_mut(|d| {
            d.insert_temp(snap_residual_id(owner), if remainder.is_finite() { remainder } else { 0.0 });
            d.insert_temp(snap_time_id(owner), now);
            d.insert_temp(snap_frame_result_id(owner), (frame_nr, steps));
        });
        if steps == 0 { None } else { Some(Wheel::SnapZoom(steps)) }
    } else {
        if !snap_mode {
            ctx.data_mut(|d| {
                d.remove::<f32>(snap_residual_id(owner));
                d.remove::<f64>(snap_time_id(owner));
            });
        }
        // Unsmoothed, a plain scroll pans by this frame's own wheel events (none: no tail).
        let scroll = if smooth || alt || shift { scroll } else { raw };
        classify(Input { scroll, raw, zoom_delta, alt, shift, zoom_with_wheel, pinch_zooms, notch })
    }
}

fn is_alt_key(key: Key) -> bool {
    matches!(key, Key::AltLeft | Key::AltRight)
}

/// This frame's wheel events, in order, with ⌥/Alt added to their modifiers while an Alt key is
/// down (#8049). The modifiers a wheel event carries are the windowing layer's modifier state,
/// which on Windows can miss a held Alt (the wheel then scrolled until another modifier was
/// tapped), while the Alt key's own press and release still arrive; the menu bar follows those
/// too (`menu_keys`). Alt is down from its press until its release, starting from whether it
/// was down before this frame's first Alt key event (egui's `keys_down` when there is none).
fn wheel_events(i: &InputState) -> Vec<(MouseWheelUnit, Vec2, Modifiers)> {
    let first_alt_key = i.events.iter().find_map(|e| match e {
        // A repeat means the key was already down; a release means it was down until then.
        Event::Key { key, pressed, repeat, .. } if is_alt_key(*key) => Some(!*pressed || *repeat),
        _ => None,
    });
    let mut alt_key = first_alt_key.unwrap_or_else(|| i.key_down(Key::AltLeft) || i.key_down(Key::AltRight));
    let mut wheel = Vec::new();
    for e in &i.events {
        match e {
            Event::Key { key, pressed, .. } if is_alt_key(*key) => alt_key = *pressed,
            Event::MouseWheel { unit, delta, modifiers, .. } => {
                let modifiers = Modifiers { alt: modifiers.alt || alt_key, ..*modifiers };
                wheel.push((*unit, *delta, modifiers));
            }
            _ => {}
        }
    }
    wheel
}

/// This frame's pinch zoom factor (1 = none). While two fingers are down on a touch screen, egui's
/// `zoom_delta` reports their pinch in place of the trackpad's `Event::Zoom`. With Preferences ›
/// Enhanced Controls › Enable Gestures off (`touch_gestures` false) the fingers are ignored and only
/// the trackpad's own pinch events count (Windows' Ctrl + wheel pinch included: [`fold_legacy_pinch`]
/// has already turned it into `Event::Zoom`).
pub fn pinch_factor(i: &InputState, touch_gestures: bool) -> f32 {
    if touch_gestures || i.multi_touch().is_none() {
        return i.zoom_delta();
    }
    i.events
        .iter()
        .filter_map(|e| match e {
            Event::Zoom(f) if f.is_finite() && *f > 0.0 => Some(*f),
            _ => None,
        })
        .product()
}

#[cfg(test)]
mod tests {
    use egui::TouchPhase;

    use super::*;

    fn input(scroll: Vec2, alt: bool) -> Input {
        Input { scroll, raw: scroll, zoom_delta: 1.0, alt, shift: false, zoom_with_wheel: false, pinch_zooms: true, notch: 40.0 }
    }

    fn wheel(delta: Vec2, modifiers: Modifiers) -> Event {
        Event::MouseWheel { unit: MouseWheelUnit::Line, delta, phase: TouchPhase::Move, modifiers }
    }

    const CTRL: Modifiers = Modifiers { ctrl: true, command: true, ..Modifiers::NONE };
    const CMD: Modifiers = Modifiers { mac_cmd: true, command: true, ..Modifiers::NONE };

    /// Feed `events` through an egui context set up like the app's (`legacy`: the Windows pinch
    /// fold on) and add up what the wheel did to the view over the following second.
    fn gesture(frames: Vec<Vec<Event>>, zoom_with_wheel: bool, legacy: bool) -> (Vec2, f32, i32) {
        let owner = Id::new("test-gesture");
        let ctx = Context::default();
        configure(&ctx);
        let (mut pan, mut zoom, mut snaps) = (Vec2::ZERO, 1.0, 0i32);
        let mut frames = frames.into_iter();
        for n in 0..60 {
            let mut raw =
                RawInput { time: Some(f64::from(n) / 60.0), predicted_dt: 1.0 / 60.0, events: frames.next().unwrap_or_default(), ..Default::default() };
            if legacy {
                fold_legacy_pinch(&ctx, &mut raw);
            }
            let mut out = ctx.run_ui(raw, |ui| match read(ui.ctx(), zoom_with_wheel, true, owner, true) {
                Some(Wheel::Pan(d)) => pan += d,
                Some(Wheel::Zoom(f)) => zoom *= f,
                Some(Wheel::SnapZoom(n)) => snaps += n,
                None => {}
            });
            out.textures_delta.clear();
        }
        (pan, zoom, snaps)
    }

    fn wheel_point(delta: Vec2, modifiers: Modifiers) -> Event {
        Event::MouseWheel { unit: MouseWheelUnit::Point, delta, phase: TouchPhase::Move, modifiers }
    }

    fn near(a: Vec2, b: Vec2) -> bool {
        (a - b).length() < 0.01
    }

    /// #5721: with smoothing off a wheel notch pans the whole notch in the frame it arrives and
    /// nothing after; smoothed (the default) it glides, reaching the same total over later frames.
    #[test]
    fn unsmoothed_wheel_pans_at_once_without_a_glide() {
        let pans = |smooth: bool| -> Vec<Vec2> {
            let owner = Id::new("test-unsmoothed");
            let ctx = Context::default();
            configure(&ctx);
            (0..30)
                .map(|n| {
                    let events = if n == 0 { vec![wheel(Vec2::new(0.0, -1.0), Modifiers::NONE)] } else { Vec::new() };
                    let raw = RawInput { time: Some(f64::from(n) / 60.0), predicted_dt: 1.0 / 60.0, events, ..Default::default() };
                    let mut pan = Vec2::ZERO;
                    let mut out = ctx.run_ui(raw, |ui| {
                        if let Some(Wheel::Pan(d)) = read_with(ui.ctx(), false, true, owner, true, smooth, true) {
                            pan = d;
                        }
                    });
                    out.textures_delta.clear();
                    pan
                })
                .collect()
        };
        let notch = {
            let ctx = Context::default();
            configure(&ctx);
            ctx.options(|o| o.input_options.line_scroll_speed)
        };
        let direct = pans(false);
        assert!(near(direct[0], Vec2::new(0.0, -notch)), "the whole notch at once: {:?}", direct[0]);
        assert!(direct[1..].iter().all(|p| *p == Vec2::ZERO), "no glide after it");
        let smooth = pans(true);
        assert!(smooth[0].y.abs() < notch - 0.5, "smoothed, the first frame moves only part of it: {:?}", smooth[0]);
        assert!(smooth[1..].iter().any(|p| p.y != 0.0), "and the rest follows");
    }

    #[test]
    fn plain_scroll_pans() {
        assert_eq!(classify(input(Vec2::new(3.0, -40.0), false)), Some(Wheel::Pan(Vec2::new(3.0, -40.0))));
        assert_eq!(classify(input(Vec2::ZERO, false)), None);
    }

    #[test]
    fn alt_scroll_zooms_ten_percent_per_notch() {
        let Some(Wheel::Zoom(f)) = classify(input(Vec2::new(0.0, 40.0), true)) else { panic!("zoom") };
        assert!((f - 1.1).abs() < 1e-5, "{f}");
        let Some(Wheel::Zoom(f)) = classify(input(Vec2::new(0.0, -40.0), true)) else { panic!("zoom") };
        assert!((f - 1.0 / 1.1).abs() < 1e-5, "{f}");
        // Two notches in one frame are two steps, x1.1 each.
        let Some(Wheel::Zoom(f)) = classify(input(Vec2::new(0.0, 80.0), true)) else { panic!("zoom") };
        assert!((f - 1.21).abs() < 1e-4, "{f}");
    }

    #[test]
    fn a_notch_zooms_the_frame_it_arrives_and_its_smoothed_tail_does_nothing() {
        // egui keeps feeding a smoothed delta for several frames after the wheel event. Zooming
        // must not follow it (that would ease the zoom), nor turn it into a pan.
        let tail = Input { raw: Vec2::ZERO, ..input(Vec2::new(0.0, 12.0), true) };
        assert_eq!(classify(tail), None);
        let tail = Input { raw: Vec2::ZERO, zoom_with_wheel: true, ..input(Vec2::new(0.0, 12.0), false) };
        assert_eq!(classify(tail), None);
        // Plain scrolling without the preference still pans from the smoothed delta.
        let pan = Input { raw: Vec2::ZERO, ..input(Vec2::new(0.0, 12.0), false) };
        assert_eq!(classify(pan), Some(Wheel::Pan(Vec2::new(0.0, 12.0))));
    }

    #[test]
    fn pinch_zooms() {
        let i = Input { zoom_delta: 1.2, ..input(Vec2::ZERO, false) };
        assert_eq!(classify(i), Some(Wheel::Zoom(1.2)));
        // The pinch wins over Alt.
        let i = Input { zoom_delta: 0.8, ..input(Vec2::new(0.0, 40.0), true) };
        assert_eq!(classify(i), Some(Wheel::Zoom(0.8)));
    }

    /// Preferences › Enhanced Controls › Zoom with Trackpad Pinch off: the pinch is ignored, and
    /// a scroll in the same frame pans as it would without a pinch.
    #[test]
    fn the_pinch_preference_switches_pinch_zooming() {
        let i = Input { zoom_delta: 1.2, pinch_zooms: false, ..input(Vec2::ZERO, false) };
        assert_eq!(classify(i), None);
        let i = Input { zoom_delta: 1.2, pinch_zooms: false, ..input(Vec2::new(0.0, 12.0), false) };
        assert_eq!(classify(i), Some(Wheel::Pan(Vec2::new(0.0, 12.0))));
        // On (the default) the pinch still zooms.
        let i = Input { zoom_delta: 1.2, pinch_zooms: true, ..input(Vec2::ZERO, false) };
        assert_eq!(classify(i), Some(Wheel::Zoom(1.2)));
    }

    /// The Windows pinch fold follows the preference too: off, the Ctrl + wheel fractions stay
    /// wheel events (which egui folds into a sideways scroll).
    #[test]
    fn the_windows_pinch_fold_follows_the_preference() {
        use eframe::App as _;
        if !cfg!(target_os = "windows") {
            return;
        }
        let mut app = crate::PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        let ctx = Context::default();
        let event = || Event::MouseWheel {
            unit: MouseWheelUnit::Line,
            delta: egui::vec2(0.0, 0.25),
            phase: egui::TouchPhase::Move,
            modifiers: Modifiers { ctrl: true, command: true, ..Modifiers::NONE },
        };
        let mut raw = RawInput { time: Some(1.0), events: vec![event()], ..Default::default() };
        app.raw_input_hook(&ctx, &mut raw);
        assert!(matches!(raw.events[0], Event::Zoom(_)), "the fraction is a pinch: {:?}", raw.events[0]);
        app.run("prefs.set", serde_json::json!({"path": "enhancedControls.zoomWithTrackpadPinch", "value": false})).unwrap();
        let mut raw = RawInput { time: Some(2.0), events: vec![event()], ..Default::default() };
        app.raw_input_hook(&ctx, &mut raw);
        assert!(matches!(raw.events[0], Event::MouseWheel { .. }), "off: it stays a wheel event: {:?}", raw.events[0]);
    }

    #[test]
    fn zoom_with_scroll_wheel_preference_swaps_wheel_and_alt_wheel() {
        let pref = |scroll, alt| Input { zoom_with_wheel: true, ..input(scroll, alt) };
        let Some(Wheel::Zoom(f)) = classify(pref(Vec2::new(0.0, 40.0), false)) else { panic!("zoom") };
        assert!((f - 1.1).abs() < 1e-5, "{f}");
        assert_eq!(classify(pref(Vec2::new(0.0, 40.0), true)), Some(Wheel::Pan(Vec2::new(0.0, 40.0))), "⌥ + wheel scrolls");
        assert_eq!(classify(pref(Vec2::new(40.0, 0.0), false)), Some(Wheel::Pan(Vec2::new(40.0, 0.0))), "sideways still scrolls");
    }

    #[test]
    fn shift_scroll_snaps_to_zoom_ladder_with_preference() {
        let snap = |dy: f32| Input { shift: true, zoom_with_wheel: true, ..input(Vec2::new(0.0, dy), false) };
        assert_eq!(classify(snap(40.0)), Some(Wheel::SnapZoom(1)), "one notch up → +1 step");
        assert_eq!(classify(snap(-40.0)), Some(Wheel::SnapZoom(-1)), "one notch down → −1 step");
        assert_eq!(classify(snap(80.0)), Some(Wheel::SnapZoom(2)), "two notches → +2 steps");
        assert_eq!(classify(snap(0.0)), None, "no scroll → nothing");
        // Some systems deliver Shift+wheel as horizontal (delta in x, y = 0).
        let snap_h = |dx: f32| Input { shift: true, zoom_with_wheel: true, raw: Vec2::new(dx, 0.0), ..input(Vec2::new(dx, 0.0), false) };
        assert_eq!(classify(snap_h(40.0)), Some(Wheel::SnapZoom(1)), "horizontal notch → +1 step");
        assert_eq!(classify(snap_h(-40.0)), Some(Wheel::SnapZoom(-1)), "horizontal notch down → −1 step");
    }

    #[test]
    fn shift_scroll_tail_does_not_snap_or_pan() {
        let tail = Input { shift: true, zoom_with_wheel: true, raw: Vec2::ZERO, ..input(Vec2::new(12.0, 0.0), false) };
        assert_eq!(classify(tail), None, "smoothed tail of ⇧ + scroll produces nothing");
    }

    #[test]
    fn shift_alt_scroll_snaps_without_preference() {
        let snap = |dy: f32| Input { shift: true, ..input(Vec2::new(0.0, dy), true) };
        assert_eq!(classify(snap(40.0)), Some(Wheel::SnapZoom(1)), "⇧ + ⌥ + notch up → +1 step");
        assert_eq!(classify(snap(-40.0)), Some(Wheel::SnapZoom(-1)), "⇧ + ⌥ + notch down → −1 step");
        assert_eq!(classify(snap(0.0)), None, "no scroll → nothing");
    }

    #[test]
    fn shift_only_without_zoom_with_wheel_does_not_snap() {
        let i = Input { shift: true, zoom_with_wheel: false, ..input(Vec2::new(0.0, 40.0), false) };
        assert_eq!(classify(i), Some(Wheel::Pan(Vec2::new(0.0, 40.0))), "⇧ alone without the preference: no zoom");
    }

    #[test]
    fn shift_scroll_snaps_through_egui_context() {
        let notch = Vec2::new(0.0, 1.0);
        // Shift + wheel with zoom_with_wheel on.
        let (pan, zoom, snaps) = gesture(vec![vec![wheel(notch, Modifiers::SHIFT)]], true, false);
        assert_eq!(snaps, 1, "⇧ + wheel with zoom_with_wheel → snap");
        assert!(pan == Vec2::ZERO, "no pan: {pan:?}");
        assert_eq!(zoom, 1.0, "no continuous zoom: {zoom}");
        let (_, _, snaps) = gesture(vec![vec![wheel(-notch, Modifiers::SHIFT)]], true, false);
        assert_eq!(snaps, -1, "⇧ + wheel down → snap out");
        // Some systems deliver Shift+wheel as horizontal (delta in x, y = 0).
        let h_notch = Vec2::new(1.0, 0.0);
        let (pan, zoom, snaps) = gesture(vec![vec![wheel(h_notch, Modifiers::SHIFT)]], true, false);
        assert_eq!(snaps, 1, "⇧ + horizontal wheel with zoom_with_wheel → snap");
        assert!(pan == Vec2::ZERO, "no pan: {pan:?}");
        assert_eq!(zoom, 1.0, "no continuous zoom: {zoom}");
        // Shift + Alt + wheel without the preference.
        let shift_alt = Modifiers { shift: true, alt: true, ..Modifiers::NONE };
        let (pan, zoom, snaps) = gesture(vec![vec![wheel(notch, shift_alt)]], false, false);
        assert_eq!(snaps, 1, "⇧ + ⌥ + wheel → snap");
        assert!(pan == Vec2::ZERO, "no pan: {pan:?}");
        assert_eq!(zoom, 1.0, "no continuous zoom: {zoom}");
    }

    #[test]
    fn each_modifier_maps_like_photoshop() {
        let notch = Vec2::new(0.0, 1.0);
        let one = |m: Modifiers| gesture(vec![vec![wheel(notch, m)]], false, false);
        let (pan, zoom, _) = one(Modifiers::NONE);
        assert!(near(pan, Vec2::new(0.0, 40.0)) && zoom == 1.0, "wheel scrolls vertically: {pan:?} {zoom}");
        for m in [Modifiers::SHIFT, CTRL, CMD, Modifiers::CTRL] {
            let (pan, zoom, _) = one(m);
            assert!(near(pan, Vec2::new(40.0, 0.0)) && zoom == 1.0, "{m:?} + wheel scrolls sideways: {pan:?} {zoom}");
        }
        let (pan, zoom, _) = one(Modifiers::ALT);
        assert!(pan == Vec2::ZERO && (zoom - 1.1).abs() < 1e-4, "⌥ + wheel zooms 10%, at once: {pan:?} {zoom}");
        // Zoom with Scroll Wheel on: swapped.
        let (pan, zoom, _) = gesture(vec![vec![wheel(notch, Modifiers::ALT)]], true, false);
        assert!(near(pan, Vec2::new(0.0, 40.0)) && zoom == 1.0, "{pan:?} {zoom}");
        let (pan, zoom, _) = gesture(vec![vec![wheel(notch, Modifiers::NONE)]], true, false);
        assert!(pan == Vec2::ZERO && (zoom - 1.1).abs() < 1e-4, "{pan:?} {zoom}");
        let (pan, zoom, _) = gesture(vec![vec![wheel(notch, CTRL)]], true, false);
        assert!(near(pan, Vec2::new(40.0, 0.0)) && zoom == 1.0, "⌘/Ctrl + wheel still scrolls sideways: {pan:?} {zoom}");
    }

    #[test]
    fn pinch_zooms_and_ctrl_wheel_pans() {
        // macOS trackpad and browsers: the pinch is its own event.
        let (pan, zoom, _) = gesture(vec![vec![Event::Zoom(1.1)], vec![Event::Zoom(1.1)]], false, false);
        assert!(pan == Vec2::ZERO && (zoom - 1.21).abs() < 1e-4, "{pan:?} {zoom}");
        // Windows: a touchpad pinch is Ctrl + wheel in fractions of a notch, a mouse wheel whole notches.
        let (pan, zoom, _) = gesture(vec![vec![wheel(Vec2::new(0.0, 0.25), CTRL)], vec![wheel(Vec2::new(0.0, 0.5), CTRL)]], false, true);
        let speed = egui::InputOptions::default().scroll_zoom_speed;
        assert!(pan == Vec2::ZERO && (zoom - (speed * 40.0 * 0.75).exp()).abs() < 1e-4, "the pinch zooms: {pan:?} {zoom}");
        let (pan, zoom, _) = gesture(vec![vec![wheel(Vec2::new(0.0, 1.0), CTRL)]], false, true);
        assert!(near(pan, Vec2::new(40.0, 0.0)) && zoom == 1.0, "Ctrl + mouse wheel scrolls sideways: {pan:?} {zoom}");
    }

    #[test]
    fn linux_wayland_point_unit_pinch_keeps_mouse_notches_for_sideways_scroll() {
        // Some Wayland compositors report fractional Ctrl-wheel pinch steps in points
        // rather than line units. The delta must not be multiplied by notch twice.
        let mut point = [Event::MouseWheel { unit: MouseWheelUnit::Point, delta: Vec2::new(0.0, 8.0), phase: TouchPhase::Move, modifiers: CTRL }];
        assert_eq!(legacy_pinch_to_zoom(&mut point, 1.0, None, 40.0, 0.005), Some(1.0));
        assert!(matches!(point[0], Event::Zoom(f) if (f - (8.0f32 * 0.005).exp()).abs() < 1e-5));
        let mut wheel_only = [wheel(Vec2::new(0.0, 1.0), CTRL)];
        assert_eq!(legacy_pinch_to_zoom(&mut wheel_only, 2.0, Some(1.0), 40.0, 0.005), Some(1.0));
        assert!(matches!(wheel_only[0], Event::MouseWheel { .. }), "whole mouse notch is not a pinch");
    }

    #[test]
    fn native_pinch_is_not_zoomed_twice_when_ctrl_wheel_arrives_in_the_same_frame() {
        let (_, zoom, _) = gesture(vec![vec![Event::Zoom(1.2), wheel(Vec2::new(0.0, 0.25), CTRL)]], false, true);
        assert!((zoom - 1.2).abs() < 1e-4, "native zoom must win over fallback: {zoom}");
    }

    #[test]
    fn native_pinch_resets_legacy_gesture_continuation() {
        let ctx = Context::default();
        let mut legacy = RawInput { time: Some(1.0), events: vec![wheel(Vec2::new(0.0, 0.25), CTRL)], ..Default::default() };
        fold_legacy_pinch(&ctx, &mut legacy);
        assert!(matches!(legacy.events[0], Event::Zoom(_)));

        // Native magnification takes over before the synthetic gesture times out.
        let mut native = RawInput { time: Some(1.1), events: vec![Event::Zoom(1.1), wheel(Vec2::new(0.0, 0.25), CTRL)], ..Default::default() };
        fold_legacy_pinch(&ctx, &mut native);
        assert!(matches!(native.events[0], Event::Zoom(f) if (f - 1.1).abs() < 1e-5));
        assert!(matches!(native.events[1], Event::MouseWheel { .. }));

        // A notched mouse scroll must no longer inherit the previous pinch.
        let mut notch = RawInput { time: Some(1.12), events: vec![wheel(Vec2::new(0.0, 1.0), CTRL)], ..Default::default() };
        fold_legacy_pinch(&ctx, &mut notch);
        assert!(matches!(notch.events[0], Event::MouseWheel { .. }));
    }

    #[test]
    fn a_whole_notch_inside_a_pinch_stays_a_pinch() {
        let mut ev = [wheel(Vec2::new(0.0, 0.5), CTRL)];
        let t = legacy_pinch_to_zoom(&mut ev, 1.0, None, 40.0, 0.005);
        assert_eq!(t, Some(1.0));
        assert!(matches!(ev[0], Event::Zoom(f) if (f - 0.1f32.exp()).abs() < 1e-5), "{ev:?}");
        // The next step lands on a whole notch: still the pinch.
        let mut ev = [wheel(Vec2::new(0.0, -1.0), CTRL)];
        assert_eq!(legacy_pinch_to_zoom(&mut ev, 1.1, t, 40.0, 0.005), Some(1.1));
        assert!(matches!(ev[0], Event::Zoom(f) if f < 1.0), "{ev:?}");
        // Long after: a mouse wheel notch, untouched.
        let mut ev = [wheel(Vec2::new(0.0, -1.0), CTRL)];
        assert_eq!(legacy_pinch_to_zoom(&mut ev, 2.0, Some(1.1), 40.0, 0.005), Some(1.1));
        assert!(matches!(ev[0], Event::MouseWheel { .. }));
        // Without Ctrl nothing is a pinch.
        let mut ev = [wheel(Vec2::new(0.0, 0.5), Modifiers::NONE)];
        assert_eq!(legacy_pinch_to_zoom(&mut ev, 2.0, None, 40.0, 0.005), None);
        assert!(matches!(ev[0], Event::MouseWheel { .. }));
    }

    #[test]
    fn hostile_numbers_never_produce_a_bad_zoom() {
        for s in [f32::NAN, f32::INFINITY, -f32::INFINITY, 1e30, -1e30] {
            for alt in [false, true] {
                for notch in [0.0, -1.0, f32::NAN, 40.0] {
                    for zd in [f32::NAN, 0.0, -1.0, f32::INFINITY, 1.0] {
                        let i = Input {
                            scroll: Vec2::new(0.0, s),
                            raw: Vec2::new(0.0, s),
                            zoom_delta: zd,
                            alt,
                            shift: false,
                            zoom_with_wheel: !alt,
                            pinch_zooms: true,
                            notch,
                        };
                        if let Some(Wheel::Zoom(f)) = classify(i) {
                            assert!(f.is_finite() && f > 0.0, "{f} from {i:?}");
                        }
                        let si = Input {
                            shift: true,
                            zoom_with_wheel: true,
                            ..Input {
                                scroll: Vec2::new(0.0, s),
                                raw: Vec2::new(0.0, s),
                                zoom_delta: zd,
                                alt: false,
                                shift: false,
                                zoom_with_wheel: true,
                                pinch_zooms: true,
                                notch,
                            }
                        };
                        if let Some(Wheel::SnapZoom(n)) = classify(si) {
                            assert!(n.abs() <= 10, "snap steps clamped: {n} from {si:?}");
                        }
                    }
                    let mut ev = [wheel(Vec2::new(s, 0.5), CTRL)];
                    let _ = legacy_pinch_to_zoom(&mut ev, f64::NAN, Some(f64::NAN), notch, 0.005);
                    let _ = legacy_pinch_to_zoom(&mut ev, 0.0, None, notch, f32::NAN);
                    if let Event::Zoom(f) = ev[0] {
                        assert!(f.is_finite() && f > 0.0, "{f}");
                    }
                }
            }
        }
    }

    /// A gesture helper that runs each frame at 1/60 s spacing with Point-unit events,
    /// collecting snaps, and using a specific owner ID.
    fn snap_gesture(frames: Vec<Vec<Event>>, zoom_with_wheel: bool, owner: Id) -> i32 {
        let ctx = Context::default();
        configure(&ctx);
        let mut snaps = 0i32;
        let mut frames = frames.into_iter();
        for n in 0..60 {
            let raw = RawInput { time: Some(f64::from(n) / 60.0), predicted_dt: 1.0 / 60.0, events: frames.next().unwrap_or_default(), ..Default::default() };
            let mut out = ctx.run_ui(raw, |ui| {
                if let Some(Wheel::SnapZoom(s)) = read(ui.ctx(), zoom_with_wheel, true, owner, true) {
                    snaps += s;
                }
            });
            out.textures_delta.clear();
        }
        snaps
    }

    #[test]
    fn fractional_point_events_accumulate_across_frames() {
        let quarter = Vec2::new(0.0, 10.0);
        let batched = snap_gesture(vec![vec![wheel_point(Vec2::new(0.0, 40.0), Modifiers::SHIFT)]], true, Id::new("accum-batched"));
        let split = snap_gesture(
            vec![
                vec![wheel_point(quarter, Modifiers::SHIFT)],
                vec![wheel_point(quarter, Modifiers::SHIFT)],
                vec![wheel_point(quarter, Modifiers::SHIFT)],
                vec![wheel_point(quarter, Modifiers::SHIFT)],
            ],
            true,
            Id::new("accum-split"),
        );
        assert_eq!(batched, 1, "40 points batched → 1 step");
        assert_eq!(split, 1, "4 × 10 points split → 1 step");
    }

    #[test]
    fn fractional_accumulation_negative_and_horizontal() {
        let neg_quarter = Vec2::new(0.0, -10.0);
        let neg_split = snap_gesture(
            vec![
                vec![wheel_point(neg_quarter, Modifiers::SHIFT)],
                vec![wheel_point(neg_quarter, Modifiers::SHIFT)],
                vec![wheel_point(neg_quarter, Modifiers::SHIFT)],
                vec![wheel_point(neg_quarter, Modifiers::SHIFT)],
            ],
            true,
            Id::new("accum-neg"),
        );
        assert_eq!(neg_split, -1, "4 × −10 points → −1 step");
        let h_quarter = Vec2::new(10.0, 0.0);
        let h_split = snap_gesture(
            vec![
                vec![wheel_point(h_quarter, Modifiers::SHIFT)],
                vec![wheel_point(h_quarter, Modifiers::SHIFT)],
                vec![wheel_point(h_quarter, Modifiers::SHIFT)],
                vec![wheel_point(h_quarter, Modifiers::SHIFT)],
            ],
            true,
            Id::new("accum-horiz"),
        );
        assert_eq!(h_split, 1, "horizontal 4 × 10 → 1 step");
    }

    #[test]
    fn accumulation_handles_empty_frames_and_direction_reversal() {
        let snaps = snap_gesture(
            vec![
                vec![wheel_point(Vec2::new(0.0, 10.0), Modifiers::SHIFT)],
                vec![], // empty frame
                vec![wheel_point(Vec2::new(0.0, 10.0), Modifiers::SHIFT)],
                vec![wheel_point(Vec2::new(0.0, 10.0), Modifiers::SHIFT)],
                vec![wheel_point(Vec2::new(0.0, 10.0), Modifiers::SHIFT)],
            ],
            true,
            Id::new("accum-empty"),
        );
        assert_eq!(snaps, 1, "empty frame preserves residual");
        let reversal = snap_gesture(
            vec![vec![wheel_point(Vec2::new(0.0, 30.0), Modifiers::SHIFT)], vec![wheel_point(Vec2::new(0.0, -15.0), Modifiers::SHIFT)]],
            true,
            Id::new("accum-reversal"),
        );
        assert_eq!(reversal, 0, "30 − 15 = 15 points, below 1 notch → 0 steps");
    }

    #[test]
    fn accumulation_shift_alt_without_preference() {
        let shift_alt = Modifiers { shift: true, alt: true, ..Modifiers::NONE };
        let split = snap_gesture(
            vec![
                vec![wheel_point(Vec2::new(0.0, 10.0), shift_alt)],
                vec![wheel_point(Vec2::new(0.0, 10.0), shift_alt)],
                vec![wheel_point(Vec2::new(0.0, 10.0), shift_alt)],
                vec![wheel_point(Vec2::new(0.0, 10.0), shift_alt)],
            ],
            false,
            Id::new("accum-shift-alt"),
        );
        assert_eq!(split, 1, "Shift+Alt 4 × 10 → 1 step without preference");
    }

    #[test]
    fn cross_viewport_does_not_leak_residual() {
        let owner_a = Id::new("owner-a");
        let owner_b = Id::new("owner-b");
        let ctx = Context::default();
        configure(&ctx);
        let mut snaps_a = 0i32;
        let mut snaps_b = 0i32;
        // Frame 0: 30 points over A (hovered), B not hovered.
        let raw =
            RawInput { time: Some(0.0), predicted_dt: 1.0 / 60.0, events: vec![wheel_point(Vec2::new(0.0, 30.0), Modifiers::SHIFT)], ..Default::default() };
        let mut out = ctx.run_ui(raw, |ui| {
            if let Some(Wheel::SnapZoom(s)) = read(ui.ctx(), true, true, owner_a, true) {
                snaps_a += s;
            }
            if let Some(Wheel::SnapZoom(s)) = read(ui.ctx(), true, true, owner_b, false) {
                snaps_b += s;
            }
        });
        out.textures_delta.clear();
        assert_eq!(snaps_a, 0, "A: 30 points, below 1 notch");
        assert_eq!(snaps_b, 0, "B: not hovered, nothing");
        // Frame 1: 10 points, pointer moved to B (B hovered, A not).
        let raw = RawInput {
            time: Some(1.0 / 60.0),
            predicted_dt: 1.0 / 60.0,
            events: vec![wheel_point(Vec2::new(0.0, 10.0), Modifiers::SHIFT)],
            ..Default::default()
        };
        let mut out = ctx.run_ui(raw, |ui| {
            if let Some(Wheel::SnapZoom(s)) = read(ui.ctx(), true, true, owner_a, false) {
                snaps_a += s;
            }
            if let Some(Wheel::SnapZoom(s)) = read(ui.ctx(), true, true, owner_b, true) {
                snaps_b += s;
            }
        });
        out.textures_delta.clear();
        assert_eq!(snaps_b, 0, "B: only 10 points of its own, no snap");
        // Frame 2–4: 30 more points over B.
        for n in 2..5 {
            let raw = RawInput {
                time: Some(f64::from(n) / 60.0),
                predicted_dt: 1.0 / 60.0,
                events: vec![wheel_point(Vec2::new(0.0, 10.0), Modifiers::SHIFT)],
                ..Default::default()
            };
            let mut out = ctx.run_ui(raw, |ui| {
                let _ = read(ui.ctx(), true, true, owner_a, false);
                if let Some(Wheel::SnapZoom(s)) = read(ui.ctx(), true, true, owner_b, true) {
                    snaps_b += s;
                }
            });
            out.textures_delta.clear();
        }
        assert_eq!(snaps_b, 1, "B: 10+10+10+10 = 40 → 1 step");
        // Frame 5: switch back to A — A's residual was cleared on the viewport switch.
        let raw = RawInput {
            time: Some(5.0 / 60.0),
            predicted_dt: 1.0 / 60.0,
            events: vec![wheel_point(Vec2::new(0.0, 30.0), Modifiers::SHIFT)],
            ..Default::default()
        };
        let mut out = ctx.run_ui(raw, |ui| {
            if let Some(Wheel::SnapZoom(s)) = read(ui.ctx(), true, true, owner_a, true) {
                snaps_a += s;
            }
            let _ = read(ui.ctx(), true, true, owner_b, false);
        });
        out.textures_delta.clear();
        assert_eq!(snaps_a, 0, "A: fresh start after switch, 30 points, no snap yet");
    }

    #[test]
    fn two_reads_same_owner_same_frame_do_not_double_count() {
        let owner = Id::new("same-owner");
        let ctx = Context::default();
        configure(&ctx);
        let mut total_snaps = 0i32;
        let raw =
            RawInput { time: Some(0.0), predicted_dt: 1.0 / 60.0, events: vec![wheel_point(Vec2::new(0.0, 20.0), Modifiers::SHIFT)], ..Default::default() };
        let mut out = ctx.run_ui(raw, |ui| {
            if let Some(Wheel::SnapZoom(s)) = read(ui.ctx(), true, true, owner, true) {
                total_snaps += s;
            }
            if let Some(Wheel::SnapZoom(s)) = read(ui.ctx(), true, true, owner, true) {
                total_snaps += s;
            }
        });
        out.textures_delta.clear();
        assert_eq!(total_snaps, 0, "20 points read twice must not become 40");
        // Now add another 20 in the next frame — should snap since accumulated 20+20=40.
        let raw = RawInput {
            time: Some(1.0 / 60.0),
            predicted_dt: 1.0 / 60.0,
            events: vec![wheel_point(Vec2::new(0.0, 20.0), Modifiers::SHIFT)],
            ..Default::default()
        };
        let mut out = ctx.run_ui(raw, |ui| {
            if let Some(Wheel::SnapZoom(s)) = read(ui.ctx(), true, true, owner, true) {
                total_snaps += s;
            }
        });
        out.textures_delta.clear();
        assert_eq!(total_snaps, 1, "20 + 20 across two frames → 1 step");
    }

    #[test]
    fn accumulation_hostile_numbers() {
        for dy in [1e30, -1e30, 1e-10, -1e-10] {
            let snaps = snap_gesture(vec![vec![wheel_point(Vec2::new(0.0, dy), Modifiers::SHIFT)]], true, Id::new("accum-hostile"));
            assert!(snaps.abs() <= 10, "hostile {dy} clamped: {snaps}");
        }
    }

    #[test]
    fn line_unit_snap_still_works_with_accumulation() {
        let (_, _, snaps) = gesture(vec![vec![wheel(Vec2::new(0.0, 1.0), Modifiers::SHIFT)]], true, false);
        assert_eq!(snaps, 1, "Line-unit notch produces 1 step");
        let (_, _, snaps) = gesture(vec![vec![wheel(Vec2::new(0.0, -1.0), Modifiers::SHIFT)]], true, false);
        assert_eq!(snaps, -1, "Line-unit notch down produces −1 step");
    }

    /// Two fingers on a touch screen: down 20 points apart, spread to 40 (a ×2 pinch), lifted.
    fn touch_pinch_frames() -> Vec<Vec<Event>> {
        let touch = |id: u64, phase: TouchPhase, x: f32| Event::Touch {
            device_id: egui::TouchDeviceId(1),
            id: egui::TouchId(id),
            phase,
            pos: egui::pos2(x, 50.0),
            force: None,
        };
        vec![
            // egui only starts a multi-touch gesture once it knows where the pointer is.
            vec![Event::PointerMoved(egui::pos2(40.0, 50.0))],
            vec![touch(1, TouchPhase::Start, 40.0), touch(2, TouchPhase::Start, 60.0)],
            vec![touch(1, TouchPhase::Move, 30.0), touch(2, TouchPhase::Move, 70.0)],
            vec![touch(1, TouchPhase::End, 30.0), touch(2, TouchPhase::End, 70.0)],
        ]
    }

    /// The total zoom [`read_with`] reports over `frames`, Enable Gestures set to `touch_gestures`.
    fn touch_zoom(frames: Vec<Vec<Event>>, touch_gestures: bool) -> f32 {
        let owner = Id::new("test-touch");
        let ctx = Context::default();
        configure(&ctx);
        let mut zoom = 1.0;
        for (n, events) in (0u32..).zip(frames) {
            let raw = RawInput { time: Some(f64::from(n) / 60.0), predicted_dt: 1.0 / 60.0, events, ..Default::default() };
            let mut out = ctx.run_ui(raw, |ui| {
                if let Some(Wheel::Zoom(f)) = read_with(ui.ctx(), false, true, owner, true, true, touch_gestures) {
                    zoom *= f;
                }
            });
            out.textures_delta.clear();
        }
        zoom
    }

    /// Preferences › Enhanced Controls › Enable Gestures (#204): on (the default) a two-finger
    /// pinch on a touch screen zooms; off, it leaves the view alone.
    #[test]
    fn enable_gestures_switches_touch_screen_pinch_zoom() {
        let on = touch_zoom(touch_pinch_frames(), true);
        assert!((on - 2.0).abs() < 1e-4, "on: the fingers zoom ×2: {on}");
        let off = touch_zoom(touch_pinch_frames(), false);
        assert_eq!(off, 1.0, "off: the fingers don't zoom");
    }

    /// Enable Gestures off only silences the touch screen: a trackpad pinch still zooms, also in a
    /// frame where two fingers rest on the screen (egui's `zoom_delta` would report theirs instead).
    #[test]
    fn enable_gestures_off_keeps_the_trackpad_pinch() {
        let zoom = touch_zoom(vec![vec![Event::Zoom(1.25)]], false);
        assert!((zoom - 1.25).abs() < 1e-4, "no fingers down: {zoom}");
        let mut frames = touch_pinch_frames();
        frames[2] = vec![Event::Zoom(1.25)];
        let zoom = touch_zoom(frames, false);
        assert!((zoom - 1.25).abs() < 1e-4, "fingers resting: {zoom}");
    }
}
