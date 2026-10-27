use super::fields::{FieldOpts, value_field_in};
use super::panel::surface;
use super::scrub::scrub_label_within;
use super::*;

/// Thin-track slider with a round knob. `gradient` paints the track (e.g. hue spectrum).
pub fn slider(ui: &mut Ui, value: &mut f32, range: std::ops::RangeInclusive<f32>, gradient: Option<&[Color32]>) -> Response {
    slider_with(ui, value, range, gradient, RowGestures::default())
}

/// Photoshop's gestures on a slider row beyond click and drag, as measured on Color Balance in
/// Photoshop 25.4 (Image › Adjustments dialog and the adjustment layer's Properties). Shift, Ctrl
/// and Alt change nothing about clicks, double-clicks or drags; the wheel takes ten steps with
/// Shift. (Up/Down in the field are every value field's, see [`value_field`].)
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RowGestures {
    /// A double-click on the slider (knob or track; not the label or field) sets this value.
    pub reset: Option<f32>,
    /// Each wheel notch over the slider moves the value by this step (up = higher; ×10 with
    /// Shift). Photoshop does this in dialogs, not in the Properties panel.
    pub wheel_step: Option<f32>,
    /// A curved track: the knob at fraction f of the track is `lo + (hi - lo) * f^k`, so small
    /// values get fine steps (Photoshop's mask Feather, #6545). Typed, scrubbed and stepped
    /// values stay linear; only the track maps through the curve.
    pub curve: Option<f32>,
}

/// The value at fraction `f` of a track over `lo..=hi`, through `curve` (see [`RowGestures`]).
pub(crate) fn curve_value(f: f32, lo: f32, hi: f32, curve: Option<f32>) -> f32 {
    match curve {
        Some(k) if k.is_finite() && k > 0.0 => lo + f.clamp(0.0, 1.0).powf(k) * (hi - lo),
        _ => lo + f * (hi - lo),
    }
}

/// Where `v` sits on a track over `lo..=hi`, as a fraction: the inverse of [`curve_value`].
pub(crate) fn curve_fraction(v: f32, lo: f32, hi: f32, curve: Option<f32>) -> f32 {
    let f = ((v - lo) / (hi - lo)).clamp(0.0, 1.0);
    match curve {
        // An unset (NaN) value draws the knob at the start rather than nowhere.
        Some(k) if k.is_finite() && k > 0.0 => {
            if f.is_finite() {
                f.powf(1.0 / k)
            } else {
                0.0
            }
        }
        _ => f,
    }
}

/// Wheel notches this frame while the pointer is over `resp` (Shift ×10). Wheel lines add up in
/// egui memory until they make whole notches (high-resolution wheels report fractions of one);
/// smooth (point) deltas count `line_scroll_speed` points per notch, like the canvas wheel.
fn wheel_notches(ui: &Ui, resp: &Response) -> f32 {
    wheel_notches_of(ui, resp.id, resp.hovered(), 10.0)
}

/// [`wheel_notches`] of widget `id` while `hovered`, a notch with Shift counting `shift_notches`.
pub(crate) fn wheel_notches_of(ui: &Ui, id: egui::Id, hovered: bool, shift_notches: f32) -> f32 {
    if !hovered {
        return 0.0;
    }
    let wheel: Vec<(egui::MouseWheelUnit, Vec2, bool)> = ui.input(|i| {
        i.events
            .iter()
            .filter_map(|e| match e {
                egui::Event::MouseWheel { unit, delta, modifiers, .. } => Some((*unit, *delta, modifiers.shift)),
                _ => None,
            })
            .collect()
    });
    count_notches(ui, id.with("wheel-acc"), &wheel, shift_notches)
}

/// Whole notches in `wheel` (unit, delta, Shift held), a notch with Shift counting
/// `shift_notches`; fractions carry over in egui memory under `acc_id`.
pub(crate) fn count_notches(ui: &Ui, acc_id: egui::Id, wheel: &[(egui::MouseWheelUnit, Vec2, bool)], shift_notches: f32) -> f32 {
    let mut acc: f32 = ui.data(|d| d.get_temp(acc_id)).unwrap_or(0.0);
    let per_line = ui.ctx().options(|o| o.input_options.line_scroll_speed);
    let per_line = if per_line.is_finite() && per_line > 0.0 { per_line } else { 40.0 };
    let mut notches = 0.0;
    for &(unit, delta, shift) in wheel {
        // Some systems turn Shift+wheel into a horizontal scroll.
        let d = if delta.y != 0.0 { delta.y } else { delta.x };
        acc += match unit {
            egui::MouseWheelUnit::Point => d / per_line,
            _ => d,
        };
        let whole = acc.trunc();
        acc -= whole;
        notches += if shift { whole * shift_notches } else { whole };
    }
    if !acc.is_finite() {
        acc = 0.0;
    }
    ui.data_mut(|d| d.insert_temp(acc_id, acc));
    if notches.is_finite() { notches } else { 0.0 }
}

/// Windows' default double-click time (Photoshop's; egui's own default is 0.3 s).
pub(super) const DOUBLE_CLICK_S: f64 = 0.5;

/// Finger jitter between taps easily exceeds egui's 6 px `max_click_dist`; 64 px is about two-thirds of an inch at 96 DPI (roughly a fingertip pad). 32 px still rejected casual Framework double-taps; Windows' mouse SM_CXDOUBLECLK is only 4 px and is not used for touch.
const TOUCH_MAX_CLICK_DIST: f32 = 64.0;

/// True when this frame's pointer work came from a touch contact (still down, or a Touch event including End on the click frame).
fn pointer_from_touch(ui: &Ui) -> bool {
    ui.input(|i| i.any_touches() || i.events.iter().any(|e| matches!(e, egui::Event::Touch { .. })))
}

/// Whether this click on `resp` completes a double-click / double-tap, counted the Windows way (Photoshop): a click soon after a lone click near the same spot; the click after a double starts over.
///
/// Time window is 0.5 s (Windows / Photoshop; egui's default is 0.3 s). Mouse uses egui's `max_click_dist` (6 px); touch uses [`TOUCH_MAX_CLICK_DIST`] so finger jitter between taps still counts. Call on every click for that widget so the first tap is remembered.
pub fn second_click(ui: &Ui, resp: &Response) -> bool {
    if !resp.clicked() {
        return false;
    }
    let key = resp.id.with("pc-last-click");
    // Prefer the response's hit pos: raw `pointer.interact_pos` can be unset on a release-only frame.
    let pos = resp.interact_pointer_pos().or_else(|| ui.input(|i| i.pointer.interact_pos()));
    let time = ui.input(|i| i.time);
    let touch = pointer_from_touch(ui);
    let mouse_dist = ui.ctx().options(|o| o.input_options.max_click_dist);
    let last: Option<(f64, Pos2, bool)> = ui.data(|d| d.get_temp(key));
    // Looser distance if either tap in the pair was touch (End clears `any_touches` but leaves Touch in `events`).
    let dist = if touch || last.is_some_and(|(_, _, t)| t) { TOUCH_MAX_CLICK_DIST.max(mouse_dist) } else { mouse_dist };
    let double = match (last, pos) {
        (Some((t, p, _)), Some(q)) => time - t < DOUBLE_CLICK_S && p.distance(q) < dist,
        _ => false,
    };
    ui.data_mut(|d| {
        if let (false, Some(q)) = (double, pos) {
            d.insert_temp(key, (time, q, touch));
        } else {
            d.remove::<(f64, Pos2, bool)>(key);
        }
    });
    double
}

fn slider_with(ui: &mut Ui, value: &mut f32, range: std::ops::RangeInclusive<f32>, gradient: Option<&[Color32]>, g: RowGestures) -> Response {
    let t = Tokens::get(ui.ctx());
    let width = ui.available_width().max(60.0);
    let (rect, mut resp) = ui.allocate_exact_size(vec2(width, 18.0), Sense::click_and_drag());
    let (lo, hi) = (*range.start(), *range.end());
    let track = Rect::from_center_size(rect.center(), vec2(rect.width() - 14.0, if gradient.is_some() { 5.0 } else { 3.0 }));
    if let Some(p) = resp.interact_pointer_pos()
        && (resp.dragged() || resp.clicked())
    {
        let f = ((p.x - track.left()) / track.width()).clamp(0.0, 1.0);
        let nv = curve_value(f, lo, hi, g.curve);
        if (nv - *value).abs() > f32::EPSILON {
            *value = nv;
            resp.mark_changed();
        }
    }
    // The double-click's first click has moved the knob to the pointer; the second resets.
    if let Some(reset) = g.reset
        && second_click(ui, &resp)
    {
        *value = reset;
        resp.mark_changed();
    }
    if let Some(step) = g.wheel_step {
        let n = wheel_notches(ui, &resp);
        if n != 0.0 {
            let nv = (*value + n * step).clamp(lo, hi);
            if nv != *value {
                *value = nv;
                resp.mark_changed();
            }
        }
    }
    let f = curve_fraction(*value, lo, hi, g.curve);
    let knob_x = track.left() + f * track.width();
    let painter = ui.painter();
    match gradient {
        Some(colors) if colors.len() >= 2 => {
            let n = colors.len() - 1;
            for (i, w) in colors.windows(2).enumerate() {
                let x0 = track.left() + track.width() * i as f32 / n as f32;
                let x1 = track.left() + track.width() * (i + 1) as f32 / n as f32;
                let mut mesh = egui::Mesh::default();
                let r = Rect::from_min_max(pos2(x0, track.top()), pos2(x1, track.bottom()));
                mesh.colored_vertex(r.left_top(), w[0]);
                mesh.colored_vertex(r.right_top(), w[1]);
                mesh.colored_vertex(r.right_bottom(), w[1]);
                mesh.colored_vertex(r.left_bottom(), w[0]);
                mesh.add_triangle(0, 1, 2);
                mesh.add_triangle(0, 2, 3);
                painter.add(mesh);
            }
        }
        _ => {
            painter.rect_filled(track, 2.0, t.field_border);
            let filled = Rect::from_min_max(track.min, pos2(knob_x, track.max.y));
            painter.rect_filled(filled, 2.0, if t.bevel { t.accent } else { t.text_dim });
        }
    }
    let knob = pos2(knob_x, rect.center().y);
    if t.bevel {
        let kr = Rect::from_center_size(knob, vec2(10.0, 16.0));
        surface(ui, kr, t.card, true);
    } else {
        let kr = if t.pro { 6.0 } else { 7.0 };
        painter.circle_filled(knob + vec2(0.0, 1.0), kr + 0.5, Color32::from_black_alpha(90));
        painter.circle_filled(knob, kr, if t.pro { Color32::from_gray(236) } else { Color32::WHITE });
        if let Some(outline) = t.knob_outline() {
            painter.circle_stroke(knob, kr, Stroke::new(1.0, outline));
        }
        if resp.hovered() || resp.dragged() {
            painter.circle_stroke(knob, 9.5, Stroke::new(2.0, t.accent_soft));
        }
    }
    resp
}

/// Labelled slider row: `Label ........ [value field]` above a full-width thin slider.
pub fn slider_row(ui: &mut Ui, label: &str, value: &mut f32, range: std::ops::RangeInclusive<f32>, suffix: &str, gradient: Option<&[Color32]>) -> Response {
    slider_row_with(ui, label, value, range, suffix, gradient, RowGestures::default())
}

/// [`slider_row`] with Photoshop's extra gestures ([`RowGestures`]).
pub fn slider_row_with(
    ui: &mut Ui,
    label: &str,
    value: &mut f32,
    range: std::ops::RangeInclusive<f32>,
    suffix: &str,
    gradient: Option<&[Color32]>,
    g: RowGestures,
) -> Response {
    slider_row_prec(ui, label, value, range, suffix, gradient, g, None)
}

/// [`slider_row_with`] whose field shows `decimals` places instead of the one (or two, for a
/// small range) it picks itself: Exposure's ±20 stops show two, its Offset four (#4164).
#[allow(clippy::too_many_arguments)]
pub fn slider_row_prec(
    ui: &mut Ui,
    label: &str,
    value: &mut f32,
    range: std::ops::RangeInclusive<f32>,
    suffix: &str,
    gradient: Option<&[Color32]>,
    g: RowGestures,
    decimals: Option<u32>,
) -> Response {
    let t = Tokens::get(ui.ctx());
    let mut changed_resp = None;
    // The row's caption names both its number (through the scrub link) and its slider.
    let mut label_id = None;
    ui.horizontal(|ui| {
        // The label scrubs the row's field, across the row (Photoshop's scrubby labels).
        let l = ui.label(egui::RichText::new(tl!(label)).color(t.text_dim));
        label_id = Some(l.id);
        scrub_label_within(ui, &l, f32::INFINITY);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            changed_resp = Some(
                value_field_in(
                    ui,
                    value,
                    range.clone(),
                    suffix,
                    74.0,
                    0.0,
                    FieldOpts { decimals, step_from_decimals: decimals.is_some(), ..Default::default() },
                    None,
                )
                .0,
            );
        });
    });
    let s = slider_with(ui, value, range, gradient, g);
    s.widget_info(|| egui::WidgetInfo::slider(ui.is_enabled(), f64::from(*value), tl!(label)));
    let s = if let Some(id) = label_id { s.labelled_by(id) } else { s };
    let mut r = s.clone();
    if let Some(v) = changed_resp {
        if v.changed() {
            r.mark_changed();
        }
        // A scrub of the field or its label is one drag, like the slider's, for history.
        use egui::response::Flags;
        r.flags |= v.flags & (Flags::DRAG_STARTED | Flags::DRAGGED | Flags::DRAG_STOPPED);
    }
    ui.add_space(4.0);
    r
}
