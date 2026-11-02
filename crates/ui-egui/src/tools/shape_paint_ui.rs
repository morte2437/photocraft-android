//! Shape fill and stroke swatches shared by the options bar and the Properties panel.

use egui::{Color32, Sense, Stroke, vec2};

use serde_json::{Value, json};

use crate::shape_gradient_ui::GradientChoices;
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
    /// The popover's Color Picker button: open the full Color Picker dialog on this paint
    /// (`shape_fill_ui::open_paint`).
    OpenPicker,
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

/// A shape paint swatch (Fill or Stroke) opening Photoshop's paint popover
/// ([`crate::paint_picker`]). The swatch shows the float paint as it is: opening the popover never
/// quantizes a high-depth colour or enables paint.
pub(crate) fn swatch(
    ui: &mut egui::Ui,
    lib: &crate::paint_picker::Library,
    fill: Option<&photocraft_doc::Fill>,
    fallback: [f32; 4],
    tip: &str,
    gradients: &mut GradientChoices,
) -> Option<PaintChange> {
    let t = Tokens::get(ui.ctx());
    let (r, resp) = ui.allocate_exact_size(vec2(30.0, 22.0), Sense::click());
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::ColorWell, ui.is_enabled(), tip));
    let current = fill.and_then(color_of);
    // Clipped to the rounded corners: square checker cells showed white past them (#8741, #3208).
    crate::widgets::checker_rounded(ui.painter(), r.shrink(1.0), 4.0, t.radius_sm);
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
    let original = current.unwrap_or(fallback);
    let gradient = fill.filter(|f| matches!(f, photocraft_doc::Fill::Gradient { .. }));
    let popup_id = resp.id;
    let mut out = None;
    let shown = crate::widgets::swatch_popup(&resp).show(|ui| {
        out = crate::paint_picker::body(ui, lib, fill, original, gradients, popup_id);
        if out == Some(PaintChange::OpenPicker) {
            // The Color Picker dialog takes over, as Photoshop's does.
            ui.close();
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
    out
}
