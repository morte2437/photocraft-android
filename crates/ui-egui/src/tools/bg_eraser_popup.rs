//! The Background Eraser's compact brush popup (#11910): what its options-bar brush picker and a
//! canvas right-click open instead of the Brush Preset picker, as in Photoshop. No preset list and
//! no search field; top to bottom Size, Hardness and Spacing (fields with sliders), the
//! angle/roundness widget, Angle and Roundness fields, then the "Size:" and "Tolerance:" pen
//! dropdowns (Off · Pen Pressure · Stylus Wheel), the latter with the reference's tablet warning.
//!
//! It edits the caller's copy of the brush like the picker does (`paint_mouse` commits it through
//! `tools.setBrush`); the Tolerance control is a tool option (`ToolOptions::bg_tolerance_control`).

use egui::{RichText, vec2};
use photocraft_engine::BrushSettings;
use photocraft_engine::paint::{Control, MAX_BRUSH_SIZE, TipShape};

use crate::theme::Tokens;
use crate::widgets;

/// The pen dropdowns' entries, as stored.
pub const OFF: &str = "off";
pub const PEN_PRESSURE: &str = "penPressure";
pub const STYLUS_WHEEL: &str = "stylusWheel";
/// The tooltip of the warning triangle next to "Tolerance:".
pub const PRESSURE_WARNING: &str = "Control by pen pressure requires the use of a pressure sensitive tablet";

/// The popup's width.
pub const WIDTH: f32 = 240.0;

fn pen_options() -> [(String, &'static str); 3] {
    [(OFF.into(), tl!("Off")), (PEN_PRESSURE.into(), tl!("Pen Pressure")), (STYLUS_WHEEL.into(), tl!("Stylus Wheel"))]
}

/// What the brush's "Size:" control is: Pen Pressure is the brush's pressure-for-size switch,
/// Stylus Wheel its Shape Dynamics size control.
pub fn size_control(b: &BrushSettings) -> &'static str {
    if b.pressure_size {
        PEN_PRESSURE
    } else if b.shape_dynamics.enabled && b.shape_dynamics.size.control == Control::StylusWheel {
        STYLUS_WHEEL
    } else {
        OFF
    }
}

/// Set the brush's "Size:" control.
pub fn set_size_control(b: &mut BrushSettings, control: &str) {
    b.pressure_size = control == PEN_PRESSURE;
    if control == STYLUS_WHEEL {
        b.shape_dynamics.enabled = true;
        b.shape_dynamics.size.control = Control::StylusWheel;
    } else if b.shape_dynamics.size.control == Control::StylusWheel {
        b.shape_dynamics.size.control = Control::Off;
    }
}

/// The popup's controls, editing `b` and the tool's Tolerance control.
pub fn body(ui: &mut egui::Ui, b: &mut BrushSettings, tolerance_control: &mut String) {
    let t = Tokens::get(ui.ctx());
    ui.set_width(WIDTH);
    ui.horizontal(|ui| {
        ui.label(RichText::new(tl!("Size")).color(t.text_dim));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let mut size = b.size;
            if widgets::value_field(ui, &mut size, 1.0..=MAX_BRUSH_SIZE, "px", 72.0).accessible_name(tl!("Size")).changed() {
                b.size = size.round().clamp(1.0, MAX_BRUSH_SIZE);
            }
        });
    });
    // Logarithmic, as the Brush Preset picker's: small sizes get most of the travel.
    let mut lv = b.size.max(1.0).ln();
    if widgets::slider(ui, &mut lv, 0.0..=MAX_BRUSH_SIZE.ln(), None).changed() {
        b.size = lv.exp().round().clamp(1.0, MAX_BRUSH_SIZE);
    }
    if b.tip == TipShape::Round {
        let mut hard = b.hardness * 100.0;
        if widgets::slider_row(ui, "Hardness", &mut hard, 0.0..=100.0, "%", None).changed() {
            b.hardness = (hard / 100.0).clamp(0.0, 1.0);
        }
    }
    let mut spacing = b.spacing * 100.0;
    if widgets::slider_row(ui, "Spacing", &mut spacing, 1.0..=1000.0, "%", None).changed() {
        b.spacing = (spacing / 100.0).clamp(0.01, 10.0);
    }
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        crate::brush_sections::ellipse_widget(ui, &mut b.angle, &mut b.roundness);
        ui.vertical(|ui| {
            let mut angle = b.angle;
            ui.horizontal(|ui| {
                ui.label(RichText::new(tl!("Angle:")).color(t.text_dim));
                if widgets::value_field(ui, &mut angle, -180.0..=180.0, "°", 64.0).accessible_name(tl!("Angle")).changed() && angle.is_finite() {
                    b.angle = angle.round().clamp(-180.0, 180.0);
                }
            });
            let mut round = b.roundness * 100.0;
            ui.horizontal(|ui| {
                ui.label(RichText::new(tl!("Roundness:")).color(t.text_dim));
                if widgets::value_field(ui, &mut round, 0.0..=100.0, "%", 64.0).accessible_name(tl!("Roundness")).changed() && round.is_finite() {
                    b.roundness = (round / 100.0).clamp(0.0, 1.0);
                }
            });
        });
    });
    ui.add_space(4.0);
    let options = pen_options();
    ui.horizontal(|ui| {
        ui.label(RichText::new(tl!("Size:")).color(t.text_dim));
        let mut size = size_control(b).to_string();
        if widgets::dropdown(ui, "bg-eraser-size-control", tl!("Size:"), &mut size, &options, 120.0) {
            set_size_control(b, &size);
        }
    });
    ui.horizontal(|ui| {
        ui.label(RichText::new(tl!("Tolerance:")).color(t.text_dim));
        widgets::dropdown(ui, "bg-eraser-tolerance-control", tl!("Tolerance:"), tolerance_control, &options, 120.0);
        let (r, resp) = ui.allocate_exact_size(vec2(16.0, 16.0), egui::Sense::hover());
        crate::icons::paint(ui, r, "triangle-alert", 14.0, t.warning);
        resp.on_hover_text(tl!(PRESSURE_WARNING)).widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, true, tl!(PRESSURE_WARNING)));
    });
}

#[cfg(test)]
#[path = "../background_eraser_brush_popup_tests.rs"]
mod background_eraser_brush_popup_tests;
