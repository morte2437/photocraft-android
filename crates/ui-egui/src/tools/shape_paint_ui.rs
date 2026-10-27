//! Shape fill and stroke swatches shared by the options bar and the Properties panel.

use egui::{Color32, Sense, Stroke, vec2};

use serde_json::{Value, json};

use crate::shape_gradient_ui::{self as shape_gradient, GradientChoices};
use crate::theme::Tokens;

pub(crate) fn color_of(f: &photocraft_doc::Fill) -> Option<[f32; 4]> {
    match f {
        photocraft_doc::Fill::Solid(c) => Some(rgba_of(c)),
        _ => None,
    }
}

pub(crate) fn rgba_of(c: &photocraft_doc::Color) -> [f32; 4] {
    let [r, g, b] = c.to_rgb();
    [r, g, b, c.alpha]
}

/// A solid or gradient paint exactly as `shape.create` / `shape.edit` take it: float RGBA, the
/// gradient's stops as stored colours (a CMYK or Lab stop keeps its model) and every gradient
/// field, so a swatch edit (angle, style…) keeps the rest of the gradient. The swatches never
/// hand it a pattern (they don't offer one).
pub(crate) fn paint_param(f: &photocraft_doc::Fill) -> Value {
    match f {
        photocraft_doc::Fill::Gradient { stops, angle, scale, style, reverse, opacity_stops, midpoints, offset, dither, align } => json!({"gradient": {
            "stops": stops.iter().map(|(t, c)| json!([t, c])).collect::<Vec<_>>(),
            "angle": angle, "scale": scale * 100.0, "style": format!("{style:?}").to_ascii_lowercase(), "reverse": reverse,
            "opacityStops": opacity_stops, "midpoints": midpoints, "offset": [offset.0, offset.1], "dither": dither, "align": align,
        }}),
        other => json!(color_of(other)),
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum PaintChange {
    Color([f32; 4]),
    /// A `Fill::Gradient` (the swatch's Gradient tab).
    Gradient(photocraft_doc::Fill),
    None,
}

pub(crate) fn rgba32(c: [f32; 4]) -> Color32 {
    let b = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    Color32::from_rgba_unmultiplied(b(c[0]), b(c[1]), b(c[2]), b(c[3]))
}

pub(crate) fn picker_color(c: [f32; 4]) -> egui::ecolor::Hsva {
    let linear = egui::ecolor::linear_from_gamma;
    let display = c.map(|v| if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 });
    egui::ecolor::Hsva::from_rgba_unmultiplied(linear(display[0]), linear(display[1]), linear(display[2]), display[3])
}

pub(crate) fn picker_rgba(hsva: egui::ecolor::Hsva) -> [f32; 4] {
    let [r, g, b, a] = hsva.to_rgba_unmultiplied();
    let gamma = egui::ecolor::gamma_from_linear;
    [gamma(r), gamma(g), gamma(b), a]
}

/// The shared compact picker keeps the float RGB paint and its alpha. Color32 is only display
/// data: opening the picker must never quantize an existing high-depth colour or enable paint.
pub(crate) fn swatch(
    ui: &mut egui::Ui,
    fill: Option<&photocraft_doc::Fill>,
    fallback: [f32; 4],
    tip: &str,
    gradients: &mut GradientChoices,
) -> Option<PaintChange> {
    let t = Tokens::get(ui.ctx());
    let (r, resp) = ui.allocate_exact_size(vec2(30.0, 22.0), Sense::click());
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::ColorWell, ui.is_enabled(), tip));
    let current = fill.and_then(color_of);
    crate::widgets::checker(ui.painter(), r.shrink(1.0), 4.0);
    match (current, fill) {
        (Some(c), _) => {
            ui.painter().rect_filled(r.shrink(1.0), t.radius_sm, rgba32(c));
        }
        (None, Some(photocraft_doc::Fill::Gradient { stops, .. })) if !stops.is_empty() => {
            // Gradient fills preview as a left-to-right ramp through their stops.
            let rgb = |c: &photocraft_doc::Color| {
                let v = c.to_rgba8();
                Color32::from_rgba_unmultiplied(v[0], v[1], v[2], v[3])
            };
            let mut mesh = egui::Mesh::default();
            let mut ramp: Vec<(f32, Color32)> = stops.iter().map(|(p, c)| (p.clamp(0.0, 1.0), rgb(c))).collect();
            ramp.insert(0, (0.0, ramp[0].1));
            ramp.push((1.0, ramp[ramp.len() - 1].1));
            for (i, (p, c)) in ramp.iter().enumerate() {
                let x = r.left() + r.width() * p;
                mesh.colored_vertex(egui::pos2(x, r.top()), *c);
                mesh.colored_vertex(egui::pos2(x, r.bottom()), *c);
                if i > 0 {
                    let k = (i as u32) * 2;
                    mesh.add_triangle(k - 2, k - 1, k);
                    mesh.add_triangle(k - 1, k, k + 1);
                }
            }
            ui.painter().add(mesh);
        }
        (None, Some(_)) => {}
        (None, None) => {
            ui.painter().rect_filled(r.shrink(1.0), t.radius_sm, t.field);
            ui.painter().line_segment([r.shrink(3.0).left_bottom(), r.shrink(3.0).right_top()], Stroke::new(1.5, t.danger));
        }
    };
    let border = if resp.has_focus() {
        t.accent
    } else if resp.hovered() {
        t.text_dim
    } else {
        t.field_border
    };
    ui.painter().rect_stroke(r, t.radius_sm, Stroke::new(1.0, border), egui::StrokeKind::Inside);
    let resp = resp.on_hover_text(tip);
    let screen_id = resp.id.with("screen-color");
    let original = current.unwrap_or(fallback);
    let sampled = |rgb: [f32; 3]| PaintChange::Color([rgb[0], rgb[1], rgb[2], original[3]]);
    let mut out = crate::screen_picker::take(ui.ctx(), screen_id).map(sampled);
    let cache = resp.id.with("shape-paint-hsva");
    let mut hsva = ui
        .data(|d| d.get_temp::<([f32; 4], egui::ecolor::Hsva)>(cache))
        .filter(|(previous, _)| *previous == original)
        .map_or_else(|| picker_color(original), |(_, hsva)| hsva);
    if let Some(PaintChange::Color(color)) = out {
        hsva = picker_color(color);
    }
    let gradient = fill.filter(|f| matches!(f, photocraft_doc::Fill::Gradient { .. }));
    let popup_id = resp.id;
    let shown = crate::widgets::swatch_popup(&resp).show(|ui| {
        ui.horizontal(|ui| {
            if ui.selectable_label(fill.is_none(), tl!("No Color")).clicked() && fill.is_some() {
                out = Some(PaintChange::None);
            }
            if ui.selectable_label(current.is_some(), tl!("Solid")).clicked() && current.is_none() {
                out = Some(PaintChange::Color(original));
            }
            if ui.selectable_label(gradient.is_some(), tl!("Gradient")).clicked() && gradient.is_none() {
                out = Some(PaintChange::Gradient(gradients.default_fill()));
            }
        });
        ui.separator();
        if let Some(g) = gradient {
            if let Some(edited) = shape_gradient::gradient_tab(ui, g, gradients, popup_id) {
                out = Some(PaintChange::Gradient(edited));
            }
            return;
        }
        let before = picker_rgba(hsva);
        if egui::color_picker::color_picker_hsva_2d(ui, &mut hsva, egui::color_picker::Alpha::OnlyBlend) {
            let color = picker_rgba(hsva);
            // egui's HSV gamma/linear round trip can move a float by an ulp without input.
            if color.iter().zip(before).any(|(a, b)| (a - b).abs() > 0.000001) {
                out = Some(PaintChange::Color(color));
            }
        }
        ui.horizontal(|ui| {
            let label = ui.label(egui::RichText::new("#").color(t.text_dim));
            let id = resp.id.with("shape-paint-hex");
            let selected = match out {
                Some(PaintChange::Color(color)) => color,
                _ => original,
            };
            // Keep incomplete typing across frames; deriving the field from the colour each
            // frame would discard every keystroke until all six digits were present.
            let mut text = ui.data(|d| d.get_temp::<([f32; 4], String)>(id)).filter(|(previous, _)| *previous == selected).map_or_else(
                || crate::color_picker_ui::hex([selected[0], selected[1], selected[2]]).trim_start_matches('#').to_ascii_uppercase(),
                |(_, text)| text,
            );
            let response = crate::widgets::text_field(ui, id, &mut text, "RRGGBB", 90.0, true).labelled_by(label.id);
            if response.changed()
                && let Some(rgb) = crate::color_picker_ui::parse_hex(&text)
            {
                let color = [rgb[0], rgb[1], rgb[2], selected[3]];
                out = Some(PaintChange::Color(color));
                hsva = picker_color(color);
            }
            if response.has_focus() {
                let color = match out {
                    Some(PaintChange::Color(color)) => color,
                    _ => selected,
                };
                ui.data_mut(|d| d.insert_temp(id, (color, text)));
            } else {
                ui.data_mut(|d| d.remove::<([f32; 4], String)>(id));
            }
        });
        if let Some(rgb) = crate::screen_picker::button(ui, screen_id) {
            out = Some(sampled(rgb));
            hsva = picker_color([rgb[0], rgb[1], rgb[2], original[3]]);
        }
    });
    // The gradient a swatch's popup closes on joins the Gradient tab's Recent row.
    let open_key = popup_id.with("open");
    let was_open = ui.data(|d| d.get_temp::<bool>(open_key)).unwrap_or(false);
    let open = shown.is_some();
    if was_open
        && !open
        && let Some(g) = gradient
    {
        gradients.remember(g);
    }
    ui.data_mut(|d| d.insert_temp(open_key, open));
    let cached_color = match out {
        Some(PaintChange::Color(color)) => color,
        _ => original,
    };
    ui.data_mut(|d| d.insert_temp(cache, (cached_color, hsva)));
    out
}
