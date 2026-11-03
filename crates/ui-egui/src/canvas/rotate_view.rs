//! Rotate View tool: turn the canvas camera around its centre without rewriting pixels.
//!
//! Drag around the view centre: the image follows the pointer (clockwise increases the angle, or
//! decreases it in a flipped view). Shift snaps to 15°. Reset View,
//! Escape, a double-click, or the on-canvas compass put the angle back to 0. History is not
//! recorded. Trackpad rotate is gated by Preferences › Enhanced Controls › Rotate View with
//! Trackpad; a two-finger twist on a touch screen also needs Enable Gestures (#204).

use egui::{Align2, Pos2, Rect, Stroke, pos2, vec2};
use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::canvas::ViewXform;
use crate::paint_mouse::Buttons;
use crate::state::View;
use crate::theme::Tokens;

/// Snap increment, in degrees, while Shift is held.
pub const SNAP_DEG: f32 = 15.0;

/// A Rotate View drag in progress.
#[derive(Clone, Copy, Debug, PartialEq)]
struct RotateDrag {
    /// Screen point of the view centre when the drag began.
    pivot: Pos2,
    /// Pointer angle, in degrees, at the press (atan2, Y-down: clockwise is positive).
    start_pointer: f32,
    /// View rotation, in degrees, at the press.
    start_rotation: f32,
}

fn id() -> egui::Id {
    egui::Id::unique("pc-rotate-view-drag")
}

fn compass_id() -> egui::Id {
    egui::Id::unique("pc-rotate-view-compass")
}

/// Wrap `a` degrees into (−180, 180]. Non-finite values become 0 so a hostile angle never
/// propagates into the camera.
pub fn wrap_deg(a: f32) -> f32 {
    if !a.is_finite() {
        return 0.0;
    }
    let mut x = f64::from(a) % 360.0;
    if x > 180.0 {
        x -= 360.0;
    } else if x <= -180.0 {
        x += 360.0;
    }
    x as f32
}

/// Nearest multiple of `step` degrees, wrapped. A non-positive or non-finite step is ignored.
pub fn snap_deg(a: f32, step: f32) -> f32 {
    if !(step.is_finite() && step > 0.0) {
        return wrap_deg(a);
    }
    wrap_deg((a / step).round() * step)
}

/// Turn a pointer or trackpad angle change (screen degrees, clockwise positive) into a change of
/// `View::rotation`. [`ViewXform::to_screen`] mirrors x after rotating, so in a flipped view a
/// larger angle turns the image the other way on screen; negating keeps it under the pointer.
pub fn screen_delta(delta: f32, flip: bool) -> f32 {
    let d = wrap_deg(delta);
    if flip { wrap_deg(-d) } else { d }
}

/// Pointer angle in degrees around `pivot` (Y-down, so clockwise is positive).
pub fn pointer_deg(p: Pos2, pivot: Pos2) -> f32 {
    let d = p - pivot;
    if !(d.x.is_finite() && d.y.is_finite()) || d.length_sq() < 1e-8 {
        return 0.0;
    }
    d.y.atan2(d.x).to_degrees()
}

fn finite_number(p: &Value, key: &str) -> Result<Option<f32>, String> {
    match p.get(key) {
        None => Ok(None),
        Some(v) => v
            .as_f64()
            .filter(|n| n.is_finite() && n.abs() <= f64::from(f32::MAX))
            .map(|n| Some(n as f32))
            .ok_or_else(|| format!("{key} must be a finite number representable as f32")),
    }
}

/// Apply `view.rotateView` params to `current`. Empty params, a non-object, or a missing
/// `angle`/`delta`/`reset` are errors (never a silent no-op).
pub fn apply_params(current: f32, p: &Value) -> Result<f32, String> {
    let obj = p.as_object().ok_or("params must be an object")?;
    for k in obj.keys() {
        if !matches!(k.as_str(), "angle" | "delta" | "reset") {
            return Err(format!("unknown field `{k}`"));
        }
    }
    let reset = match obj.get("reset") {
        None => false,
        Some(v) => v.as_bool().ok_or_else(|| "reset must be a boolean".to_string())?,
    };
    let angle = finite_number(p, "angle")?;
    let delta = finite_number(p, "delta")?;
    if !reset && angle.is_none() && delta.is_none() {
        return Err("expected angle, delta, or reset".into());
    }
    if reset {
        return Ok(0.0);
    }
    let mut r = angle.unwrap_or(current);
    if let Some(d) = delta {
        r += d;
        if !r.is_finite() {
            return Err("delta overflowed the angle".into());
        }
    }
    Ok(wrap_deg(r))
}

/// Trackpad rotate: `delta_rad` is egui's rotation delta (radians, same sign as [`wrap_deg`]).
/// Returns the new angle when the preference is on and the delta is a finite non-zero value.
pub fn apply_trackpad(current: f32, enabled: bool, delta_rad: f32) -> Option<f32> {
    if !enabled || !delta_rad.is_finite() || delta_rad == 0.0 {
        return None;
    }
    let deg = delta_rad.to_degrees();
    if !deg.is_finite() {
        return None;
    }
    let next = current + deg;
    if !next.is_finite() {
        return None;
    }
    Some(wrap_deg(next))
}

/// This frame's trackpad rotation in degrees, if the preference is on and egui reported a delta.
/// The trackpad's own `Rotate` events always count; a touch screen's two-finger twist (egui's
/// multi-touch rotation) only while `touch_gestures` (Enhanced Controls › Enable Gestures) is on.
pub fn trackpad_delta(ctx: &egui::Context, enabled: bool, touch_gestures: bool) -> Option<f32> {
    if !enabled {
        return None;
    }
    let rad = ctx.input(|i| {
        let from_events: f32 = i
            .events
            .iter()
            .filter_map(|e| match e {
                egui::Event::Rotate(r) if r.is_finite() => Some(*r),
                _ => None,
            })
            .sum();
        if from_events != 0.0 {
            from_events
        } else if touch_gestures {
            i.rotation_delta()
        } else {
            0.0
        }
    });
    apply_trackpad(0.0, true, rad).filter(|_| rad != 0.0)
}

/// `view.rotateView`: set or add degrees on the active document's camera.
pub fn command(app: &mut PhotocraftApp, p: &Value) -> Result<Value, String> {
    let i = app.session.active_index().ok_or("no document open")?;
    let current = app.ui.views.get(i).map(|v| v.rotation).ok_or("no document open")?;
    let rotation = apply_params(current, p)?;
    let v = app.ui.views.get_mut(i).ok_or("no document open")?;
    v.rotation = rotation;
    Ok(json!({"rotation": v.rotation}))
}

/// `view.resetView`: angle 0, zoom and centre unchanged.
pub fn reset(app: &mut PhotocraftApp) -> Result<Value, String> {
    let i = app.session.active_index().ok_or("no document open")?;
    let v = app.ui.views.get_mut(i).ok_or("no document open")?;
    v.rotation = 0.0;
    Ok(json!({"rotation": 0.0, "zoom": v.zoom, "center": v.center}))
}

/// Route canvas buttons for Rotate View. Returns true when the drag is owned (the canvas then
/// ignores the tool underneath).
pub fn drag(ctx: &egui::Context, view: &mut View, xf: &ViewXform, b: &Buttons, pointer: Option<Pos2>) -> bool {
    if b.started {
        let pivot = xf.rect.center();
        let origin = ctx.input(|i| i.pointer.press_origin()).filter(|p| xf.rect.contains(*p)).or(pointer);
        if let Some(p) = origin {
            ctx.data_mut(|d| {
                d.insert_temp(id(), RotateDrag { pivot, start_pointer: pointer_deg(p, pivot), start_rotation: view.rotation });
            });
        }
    }
    let Some(z) = ctx.data(|d| d.get_temp::<RotateDrag>(id())) else {
        return b.started || b.dragged || b.stopped;
    };
    if let Some(p) = pointer.filter(|_| b.dragged || b.stopped) {
        let delta = screen_delta(pointer_deg(p, z.pivot) - z.start_pointer, xf.flip);
        let mut angle = z.start_rotation + delta;
        if !angle.is_finite() {
            angle = z.start_rotation;
        }
        if ctx.input(|i| i.modifiers.shift) {
            angle = snap_deg(angle, SNAP_DEG);
        }
        view.rotation = wrap_deg(angle);
    }
    if b.stopped {
        ctx.data_mut(|d| d.remove::<RotateDrag>(id()));
    }
    true
}

/// Compass in the top-right of the canvas (north = document up). Hidden at 0°.
pub fn compass_rect(canvas: Rect) -> Rect {
    const S: f32 = 36.0;
    const M: f32 = 10.0;
    Rect::from_min_size(pos2(canvas.right() - M - S, canvas.top() + M), vec2(S, S))
}

/// True when the compass is showing (angle is not ~0).
pub fn compass_visible(rotation: f32) -> bool {
    rotation.is_finite() && rotation.abs() > 0.05
}

/// Draw the compass and handle a click / Escape / double-click reset. Returns true when the
/// pointer over the compass should not start a rotate drag.
pub fn overlay(app: &PhotocraftApp, ctx: &egui::Context, painter: &egui::Painter, xf: &ViewXform, view: &mut View, response: &egui::Response) -> bool {
    if app.ui.tool == crate::state::Tool::RotateView && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) && view.rotation != 0.0 {
        view.rotation = 0.0;
        ctx.data_mut(|d| d.remove::<RotateDrag>(id()));
    }
    if app.ui.tool == crate::state::Tool::RotateView && (response.double_clicked() || response.triple_clicked()) {
        view.rotation = 0.0;
        ctx.data_mut(|d| d.remove::<RotateDrag>(id()));
    }
    if !compass_visible(view.rotation) {
        ctx.data_mut(|d| d.remove::<Rect>(compass_id()));
        return false;
    }
    let r = compass_rect(xf.rect);
    ctx.data_mut(|d| d.insert_temp(compass_id(), r));
    let t = Tokens::get(ctx);
    let hover = response.hover_pos().is_some_and(|p| r.contains(p));
    painter.circle_filled(r.center(), r.width() * 0.5, if hover { t.hover } else { t.chrome });
    painter.circle_stroke(r.center(), r.width() * 0.5 - 0.5, Stroke::new(1.0, t.separator));
    // Document up: (0, −1) in document space, rotated then flipped like [`ViewXform::to_screen`].
    let rad = view.rotation.to_radians();
    let (c, s) = if view.rotation.abs() < 1e-6 { (1.0, 0.0) } else { (rad.cos(), rad.sin()) };
    let (rx, ry) = (s, -c);
    let sx = if xf.flip { -1.0 } else { 1.0 };
    let needle = vec2(rx * sx, ry) * (r.width() * 0.32);
    let tip = r.center() + needle;
    painter.line_segment([r.center(), tip], Stroke::new(2.0, t.accent));
    painter.circle_filled(tip, 2.5, t.accent);
    painter.text(r.center() + vec2(0.0, r.width() * 0.18), Align2::CENTER_CENTER, "N", egui::FontId::proportional(9.0), t.text_faint);
    if response.clicked() && response.interact_pointer_pos().is_some_and(|p| r.contains(p)) {
        view.rotation = 0.0;
        ctx.data_mut(|d| d.remove::<RotateDrag>(id()));
        return true;
    }
    hover
}

/// Cursor for the Rotate View tool.
pub fn cursor(dragged: bool) -> egui::CursorIcon {
    if dragged { egui::CursorIcon::Grabbing } else { egui::CursorIcon::Grab }
}

#[cfg(test)]
mod tests;
