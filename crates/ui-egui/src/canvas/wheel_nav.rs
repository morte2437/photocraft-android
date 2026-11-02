//! Mouse wheel and trackpad navigation over the canvas (#293, #635), as in Photoshop:
//!
//! - The wheel scrolls up and down; ⇧ or ⌘/Ctrl + wheel scrolls sideways.
//! - ⌥/Alt + wheel zooms around the pointer, 10% per wheel notch (eased or instant, see below).
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
//! Either way a wheel notch zooms by exactly ×1.1 in total. How that step is spread over frames
//! is set by Preferences › General › Animated Zoom (`general.animatedZoom`, on by default):
//!
//! - On: the zoom is eased over several frames (the curve of egui's wheel smoothing, fed by the zoom
//!   notches only), so the factors of the frames of one notch multiply to ×1.1. An unfinished notch
//!   is superseded, not carried, when ⇧ snap zoom or a pan takes over or the pointer leaves.
//! - Off: one instant ×1.1 step per notch, applied the frame it arrives with no easing in between
//!   (the Photoshop behaviour, measured frame by frame on a screen recording). Zooming reads the
//!   wheel events themselves and ignores egui's smoothed tail.
//!
//! In both modes panning keeps the smoothed delta, and ⇧/Shift snap zoom reads the wheel events
//! and stays instant. The Alt state is taken from the wheel events (and the Alt key's own press
//! and release, `wheel_events`) and kept until the next wheel event: releasing Alt while the tail
//! is still settling must not turn it into a pan.

use egui::{Context, Event, Id, InputState, Key, Modifiers, MouseWheelUnit, RawInput, TouchPhase, Vec2};

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
    /// Smoothed scroll delta (points, content direction); what panning uses. In the animated zoom
    /// modes it is the eased zoom delta of that mode's own wheel events only.
    pub scroll: Vec2,
    /// This frame's wheel events for the gesture in progress, unsmoothed (points, content
    /// direction); what the instant zoom and the snap zoom use.
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
    /// Preferences › General › Animated Zoom: the wheel zoom follows the smoothed delta (`scroll`)
    /// across frames; off, it is one instant step from this frame's wheel events (`raw`).
    pub animated_zoom: bool,
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
    let scroll_finite = i.scroll.x.is_finite() && i.scroll.y.is_finite();
    match (i.alt, i.zoom_with_wheel) {
        // ⇧ + ⌥ + scroll: snap to the zoom ladder instead of continuous ×1.1.
        (true, false) if i.shift => snap(i.raw.y + i.raw.x),
        // ⌥ + scroll: continuous ×1.1 zoom. Animated Zoom on: follow the smoothed delta across frames.
        (true, false) if i.animated_zoom && scroll_finite => step(i.scroll.y + i.scroll.x),
        // Animated Zoom off: one instant step from the wheel events. The smoothed tail is ignored.
        (true, false) if !i.animated_zoom => step(i.raw.y + i.raw.x),
        // ⇧ + scroll wheel with the preference on: snap to the zoom ladder.
        // Some systems deliver Shift+wheel as horizontal (delta in x, y = 0).
        (false, true) if i.shift => snap(i.raw.y + i.raw.x),
        // ⌘/Ctrl folds the wheel into x: those still scroll sideways.
        (false, true) if i.animated_zoom && scroll_finite && i.scroll.y != 0.0 => step(i.scroll.y),
        (false, true) if !i.animated_zoom && (i.raw.y != 0.0 || i.scroll.y != 0.0) => step(i.raw.y),
        _ if !scroll_finite || i.scroll == Vec2::ZERO => None,
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
    Id::unique("pc-wheel-legacy-pinch")
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
    Id::unique("pc-wheel-alt")
}

fn shift_id() -> Id {
    Id::unique("pc-wheel-shift")
}

fn snap_residual_id(owner: Id) -> Id {
    Id::unique("pc-wheel-snap-residual").with(owner)
}

fn snap_time_id(owner: Id) -> Id {
    Id::unique("pc-wheel-snap-time").with(owner)
}

fn snap_frame_result_id(owner: Id) -> Id {
    Id::unique("pc-wheel-snap-frame-result").with(owner)
}

fn active_snap_owner_id() -> Id {
    Id::unique("pc-wheel-active-snap-owner")
}

const SNAP_IDLE_TIMEOUT: f64 = 0.5;

/// egui's own wheel smoothing (`InputState::begin_pass`): 90 % of a notch is released in 0.1 s.
/// The eased zoom uses the same curve, so it paces like the eased pan.
const EASE_REACH: f32 = 0.90;
const EASE_REACH_SECS: f32 = 0.1;

/// An unfinished eased notch belongs to the frames that follow it. It is dropped, never carried
/// into a later notch, once its owner was not driven for more than this many passes (the pointer
/// left the viewport, or another gesture took over there)...
const ZOOM_EASE_MAX_GAP_PASSES: u64 = 3;
/// ...or, when at least one pass was skipped, for this long (seconds). A single slow frame is
/// not a gap: the owner was driven in the pass right before it, so the notch still finishes.
const ZOOM_EASE_STALE_SECS: f64 = 0.5;

#[derive(Clone, Copy)]
struct ZoomEase {
    residual: f32,
    time: f64,
    frame: u64,
    released: f32,
}

fn zoom_ease_id(owner: Id) -> Id {
    Id::unique(("wheel-zoom-ease", owner))
}

/// Eases `delta` (points of this viewport's zoom-mode wheel events) over the next frames and
/// returns what is released this frame. Same curve as egui's wheel smoothing, but fed by the
/// zoom notches only, so the tail of a pan notch never leaks in. What is released adds up to
/// exactly what was fed. A second call in the same frame returns the cached result.
fn ease_zoom_delta(ctx: &Context, owner: Id, delta: f32) -> f32 {
    let id = zoom_ease_id(owner);
    let frame = ctx.cumulative_pass_nr();
    let (now, dt) = ctx.input(|i| (i.time, i.stable_dt.min(0.1)));
    let prev = ctx.data(|d| d.get_temp::<ZoomEase>(id));
    if let Some(e) = prev.filter(|e| e.frame == frame) {
        return e.released;
    }
    // Fresh: driven in the previous pass (a slow frame, however long, keeps its notch), or within a
    // few passes and a short time (a one-pass hover flicker). Anything older is stale.
    let fresh = |e: &ZoomEase| {
        let gap = frame.saturating_sub(e.frame);
        e.residual.is_finite() && (gap <= 1 || (gap <= ZOOM_EASE_MAX_GAP_PASSES && now - e.time < ZOOM_EASE_STALE_SECS))
    };
    let carried = prev.filter(fresh).map_or(0.0, |e| e.residual);
    let mut residual = carried + if delta.is_finite() { delta } else { 0.0 };
    let t = egui::emath::exponential_smooth_factor(EASE_REACH, EASE_REACH_SECS, dt);
    let released = if residual.abs() < 1.0 { residual } else { t * residual };
    residual -= released;
    ctx.data_mut(|d| d.insert_temp(id, ZoomEase { residual, time: now, frame, released }));
    // Callers do not repaint on their own after consuming the zoom: keep frames coming until the
    // notch has been fully released, or a reactive UI would stop below ×1.1.
    if residual != 0.0 {
        ctx.request_repaint();
    }
    released
}

/// A zoom limit stopped the eased zoom of `owner`: drop what is left of the notch. It would no
/// longer zoom, only cancel the first notch back (flick to 12800 %, and the notch back did
/// nothing). This frame's release stays cached, so a second read in the same frame is unchanged.
pub fn settle(ctx: &Context, owner: Id) {
    let id = zoom_ease_id(owner);
    ctx.data_mut(|d| {
        if let Some(e) = d.get_temp::<ZoomEase>(id) {
            d.insert_temp(id, ZoomEase { residual: 0.0, ..e });
        }
    });
}

/// Whether a trackpad touch is in progress (egui's `InTouch`), at the start and at the end of a
/// frame: the start is kept, so every read in one frame sees the same states.
#[derive(Clone, Copy)]
struct Touch {
    frame: u64,
    at_start: bool,
    at_end: bool,
}

/// Per viewport, as egui keeps its wheel state per viewport.
fn touch_id(ctx: &Context) -> Id {
    Id::unique(("wheel-nav-touch", ctx.viewport_id()))
}

/// This frame's wheel events (in order) with, for each, whether egui leaves it unsmoothed: during a
/// trackpad touch, or a point delta under 8 points (`WheelState::on_wheel_event`). Those are
/// already smooth, and easing them again would only add latency; the rest (notches, and the
/// big pixel steps browsers report for a mouse wheel) is what Animated Zoom eases.
fn mark_unsmoothed(ctx: &Context, events: Vec<(MouseWheelUnit, Vec2, Modifiers, TouchPhase)>) -> Vec<(MouseWheelUnit, Vec2, Modifiers, bool)> {
    let frame = ctx.cumulative_pass_nr();
    let (horizontal, vertical) = ctx.options(|o| (o.input_options.horizontal_scroll_modifier, o.input_options.vertical_scroll_modifier));
    // Not read in the previous pass (no canvas then): a touch could have ended unseen, and a
    // stuck one would leave every later notch un-eased.
    let id = touch_id(ctx);
    let mut touch = match ctx.data(|d| d.get_temp::<Touch>(id)) {
        Some(t) if t.frame == frame => t.at_start,
        Some(t) if t.frame.saturating_add(1) == frame => t.at_end,
        _ => false,
    };
    let at_start = touch;
    let mut wheel = Vec::with_capacity(events.len());
    for (unit, delta, modifiers, phase) in events {
        match phase {
            TouchPhase::Start => touch = true,
            TouchPhase::End | TouchPhase::Cancel => touch = false,
            TouchPhase::Move => {}
        }
        // egui folds a scroll onto one axis by its modifiers before it measures the step.
        let step = match (modifiers.matches_any(horizontal), modifiers.matches_any(vertical)) {
            (true, false) | (false, true) => (delta.x + delta.y).abs(),
            _ => delta.length(),
        };
        let instant = touch || (unit == MouseWheelUnit::Point && step < 8.0);
        wheel.push((unit, delta, modifiers, instant));
    }
    ctx.data_mut(|d| d.insert_temp(id, Touch { frame, at_start, at_end: touch }));
    wheel
}

/// Read this frame's wheel input. Call it every frame (hovered or not) so the Alt and Shift
/// states of the gesture follow the latest wheel event. `owner` scopes the snap-zoom
/// accumulator per viewport. `hovered` gates accumulation: only the viewport receiving
/// navigation may add raw input to the snap residual.
pub fn read(ctx: &Context, zoom_with_wheel: bool, pinch_zooms: bool, animated_zoom: bool, owner: Id, hovered: bool) -> Option<Wheel> {
    read_with(ctx, zoom_with_wheel, pinch_zooms, animated_zoom, owner, hovered, true, true)
}

/// [`read`], choosing how a plain wheel scroll pans: `smooth` glides it over a few frames (egui's
/// smoothed delta); off, as with Interface › Enable Animations off, the view moves the whole notch
/// in the frame the wheel turns and stops there (#5721), as scroll areas already do.
/// `touch_gestures` is Preferences › Enhanced Controls › Enable Gestures: off, a touch screen's
/// two-finger pinch doesn't zoom ([`pinch_factor`]).
/// `animated_zoom` is Preferences › General › Animated Zoom: on, the wheel zoom is eased over the
/// frames of a notch; off, a notch is one instant ×1.1 step.
#[allow(clippy::too_many_arguments)]
pub fn read_with(
    ctx: &Context,
    zoom_with_wheel: bool,
    pinch_zooms: bool,
    animated_zoom: bool,
    owner: Id,
    hovered: bool,
    smooth: bool,
    touch_gestures: bool,
) -> Option<Wheel> {
    let notch = ctx.options(|o| o.input_options.line_scroll_speed);

    // --- Modifier tracking (always runs for all callers) ---
    let (scroll, zoom_delta, latest_alt, latest_shift, wheel, page) = ctx.input(|i| {
        let wheel = wheel_events(i);
        let latest_alt = wheel.last().map(|(_, _, m, _)| m.alt && !m.command && !m.ctrl);
        let latest_shift = wheel.last().map(|(_, _, m, _)| m.shift && !m.command && !m.ctrl);
        (i.smooth_scroll_delta, pinch_factor(i, touch_gestures), latest_alt, latest_shift, wheel, i.content_rect().height())
    });
    let wheel = mark_unsmoothed(ctx, wheel);
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
    // `instant`: the part of `raw` egui leaves unsmoothed (a trackpad; see `wheel_events`).
    let (raw, instant) = wheel
        .iter()
        .filter(|(_, _, m, _)| {
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
        .fold((Vec2::ZERO, Vec2::ZERO), |(sum, instant), (unit, delta, _, unsmoothed)| {
            let d = *delta
                * match unit {
                    MouseWheelUnit::Point => 1.0,
                    MouseWheelUnit::Line => notch,
                    MouseWheelUnit::Page => page,
                };
            (sum + d, if *unsmoothed { instant + d } else { instant })
        });
    let snap_mode = shift && ((alt && !zoom_with_wheel) || (!alt && zoom_with_wheel));
    let has_pinch = pinch_zooms && zoom_delta.is_finite() && zoom_delta > 0.0 && zoom_delta != 1.0;

    if snap_mode && !has_pinch {
        // Shift snap zoom takes over: drop this owner's unfinished eased notch, so a continuous
        // zoom started shortly after Shift is released cannot carry its residual.
        ctx.data_mut(|d| d.remove::<ZoomEase>(zoom_ease_id(owner)));
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
        // Zoom-mode input: ⌥ + wheel, or the plain wheel with Zoom with Scroll Wheel. With Animated
        // Zoom on it eases from this mode's own wheel events: egui's smoothed delta has no notion of
        // modifiers, so it would also carry the unfinished tail of a pan notch into the zoom.
        let wheel_zooms = !shift && (alt != zoom_with_wheel);
        let scroll = if animated_zoom && wheel_zooms {
            // Eased as egui eases a wheel: what it leaves unsmoothed (a trackpad) zooms at once.
            let along = |v: Vec2| if alt { v.y + v.x } else { v.y };
            let eased = ease_zoom_delta(ctx, owner, along(raw - instant)) + along(instant);
            // The plain wheel zooms through `y`; its sideways part (⌘/Ctrl + wheel, a trackpad's
            // horizontal scroll) is not a zoom event, so it pans like any scroll: smoothed, or
            // with Enable Animations off at once (#5721).
            if alt { Vec2::new(0.0, eased) } else { Vec2::new(if smooth { scroll.x } else { raw.x }, eased) }
        } else {
            ctx.data_mut(|d| d.remove::<ZoomEase>(zoom_ease_id(owner)));
            // Unsmoothed, a plain scroll pans by this frame's own wheel events (none: no tail).
            if smooth || alt || shift { scroll } else { raw }
        };
        classify(Input { scroll, raw, zoom_delta, alt, shift, zoom_with_wheel, pinch_zooms, animated_zoom, notch })
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
fn wheel_events(i: &InputState) -> Vec<(MouseWheelUnit, Vec2, Modifiers, TouchPhase)> {
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
            Event::MouseWheel { unit, delta, phase, modifiers, .. } => {
                let modifiers = Modifiers { alt: modifiers.alt || alt_key, ..*modifiers };
                wheel.push((*unit, *delta, modifiers, *phase));
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
mod tests;
