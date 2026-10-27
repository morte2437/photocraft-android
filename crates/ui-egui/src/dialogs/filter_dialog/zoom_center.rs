//! Radial Blur's Zoom centre: a schematic of the streaks beside the two coordinate sliders.
//! The diagram and sliders edit the same dialog fields used by preview, OK and automation.

use egui::{Rect, Response, Sense, Stroke, Ui, Vec2, pos2, vec2};
use serde_json::{Map, Value, json};

use crate::theme::Tokens;

pub(super) fn owns(command: &str, fields: &Map<String, Value>) -> bool {
    command == "filter.blur.radialBlur" && fields.get("method").and_then(Value::as_str) == Some("zoom")
}

fn number(fields: &Map<String, Value>, key: &str, default: f64, min: f64, max: f64) -> f32 {
    fields.get(key).and_then(Value::as_f64).filter(|n| n.is_finite()).unwrap_or(default).clamp(min, max) as f32
}

fn rounded(value: f32) -> f32 {
    (value.clamp(0.0, 1.0) * 100.0).round() / 100.0
}

pub(super) fn row(ui: &mut Ui, fields: &mut Map<String, Value>) {
    let mut center = vec2(number(fields, "centerX", 0.5, 0.0, 1.0), number(fields, "centerY", 0.5, 0.0, 1.0));
    let amount = number(fields, "amount", 10.0, 1.0, 100.0);
    ui.horizontal(|ui| {
        let (rect, mut response) = ui.allocate_exact_size(vec2(112.0, 112.0), Sense::click_and_drag());
        // Leave room for the centre marker at 0 and 1, including all four corners.
        let plot = rect.shrink(4.0);
        interact(ui, &mut response, plot, &mut center);
        ui.add_space(4.0);
        ui.vertical(|ui| {
            ui.set_width(ui.available_width());
            for (key, value) in [("centerX", &mut center.x), ("centerY", &mut center.y)] {
                let label = super::label(key);
                let gestures = crate::widgets::RowGestures { reset: Some(0.5), wheel_step: None, ..Default::default() };
                let slider = crate::widgets::slider_row_with(ui, &label, value, 0.0..=1.0, "", None, gestures);
                *value = rounded(*value);
                slider.widget_info(|| egui::WidgetInfo::slider(ui.is_enabled(), f64::from(*value), &label));
            }
        });
        // Paint after both sliders so their edits move the marker in this very frame.
        paint(ui, rect, plot, center, amount, response.has_focus());
        response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Slider, ui.is_enabled(), tl!("Center")));
        response.on_hover_cursor(egui::CursorIcon::Crosshair);
    });
    for (key, value) in [("centerX", center.x), ("centerY", center.y)] {
        fields.insert(key.into(), json!((f64::from(value) * 100.0).round() / 100.0));
    }
}

fn interact(ui: &Ui, response: &mut Response, plot: Rect, center: &mut Vec2) {
    ui.memory_mut(|m| m.set_focus_lock_filter(response.id, egui::EventFilter { horizontal_arrows: true, vertical_arrows: true, ..Default::default() }));
    let primary = egui::PointerButton::Primary;
    if (response.clicked_by(primary) || response.dragged_by(primary) || (response.is_pointer_button_down_on() && ui.input(|i| i.pointer.primary_down())))
        && let Some(p) = response.interact_pointer_pos().filter(|p| p.x.is_finite() && p.y.is_finite())
    {
        *center = vec2(rounded((p.x - plot.left()) / plot.width()), rounded((p.y - plot.top()) / plot.height()));
        response.request_focus();
        response.mark_changed();
    }
    if response.has_focus() {
        let step = ui.input(|i| {
            vec2(
                i.num_presses(egui::Key::ArrowRight) as f32 - i.num_presses(egui::Key::ArrowLeft) as f32,
                i.num_presses(egui::Key::ArrowDown) as f32 - i.num_presses(egui::Key::ArrowUp) as f32,
            )
        });
        if step != Vec2::ZERO {
            *center = vec2(rounded(center.x + step.x * 0.01), rounded(center.y + step.y * 0.01));
            response.mark_changed();
        }
    }
}

fn paint(ui: &Ui, rect: Rect, plot: Rect, center: Vec2, amount: f32, focused: bool) {
    if !ui.is_rect_visible(rect) {
        return;
    }
    let t = Tokens::get(ui.ctx());
    ui.painter().rect_filled(rect, t.radius_sm, t.field);
    let painter = ui.painter_at(rect.shrink(1.0));
    let origin = plot.min + center * plot.size();
    // A regular grid of points stretches away from the origin. The Zoom kernel samples
    // inward by amount/200, so a source point's outermost image is at 1/(1 - amount/200).
    // This small fixed diagram never filters or reads the document's pixels.
    let stretch = 1.0 / (1.0 - amount / 200.0);
    for y in 0..9 {
        for x in 0..9 {
            let point = plot.min + vec2((x as f32 + 0.5) / 9.0, (y as f32 + 0.5) / 9.0) * plot.size();
            let end = origin + (point - origin) * stretch;
            for step in 0..4 {
                let from = point.lerp(end, step as f32 / 4.0);
                let to = point.lerp(end, (step + 1) as f32 / 4.0);
                painter.line_segment([from, to], Stroke::new(1.0, t.text_dim.gamma_multiply(1.0 - step as f32 / 5.0)));
            }
        }
    }
    painter.line_segment([pos2(plot.left(), origin.y), pos2(plot.right(), origin.y)], Stroke::new(1.0, t.field_border));
    painter.line_segment([pos2(origin.x, plot.top()), pos2(origin.x, plot.bottom())], Stroke::new(1.0, t.field_border));
    painter.circle(origin, 3.0, t.field, Stroke::new(1.5, t.accent));
    ui.painter().rect_stroke(rect, t.radius_sm, Stroke::new(1.0, if focused { t.accent } else { t.field_border }), egui::StrokeKind::Inside);
}
