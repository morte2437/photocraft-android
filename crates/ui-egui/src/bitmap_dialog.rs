//! Image › Mode › Bitmap… (#5559), laid out like Photoshop's dialog: Resolution (Input, read-only,
//! and Output, in pixels/inch or pixels/centimeter), Method (Use: 50% Threshold, Pattern Dither,
//! Diffusion Dither, Halftone Screen, Custom Pattern) with the Custom Pattern picker, enabled only
//! for Custom Pattern. The Halftone Screen settings (Frequency in lines/inch or lines/centimeter,
//! Angle, Shape), which Photoshop asks for in a second dialog after OK, show here only while
//! Halftone Screen is the method.
//!
//! It is the schema dialog of `image.mode.bitmap` with its own body: the fields are the command's
//! params (`resolution` in pixels/inch, `frequency` in lines/inch), so OK, the live preview,
//! `ui.dialog.set` and `ui.dialog.confirm` work as for every schema dialog. The units are view
//! state (`__resUnit`, `__freqUnit`).

use serde_json::{Map, Value, json};

use crate::PhotocraftApp;
use crate::theme::Tokens;

pub(crate) const COMMAND: &str = "image.mode.bitmap";

/// Centimetres per inch.
const CM: f32 = 2.54;
/// The command's Output resolution range (pixels/inch).
const RESOLUTION: (f32, f32) = (1.0, 30_000.0);
/// The command's Halftone Screen frequency range (lines/inch).
const FREQUENCY: (f32, f32) = (1.0, 999.0);

const METHODS: [(&str, &str); 5] = [
    ("threshold", "50% Threshold"),
    ("pattern", "Pattern Dither"),
    ("diffusion", "Diffusion Dither"),
    ("halftone", "Halftone Screen"),
    ("customPattern", "Custom Pattern"),
];

/// Photoshop's order of the Halftone Screen shapes.
const SHAPES: [(&str, &str); 6] =
    [("round", "Round"), ("diamond", "Diamond"), ("ellipse", "Ellipse"), ("line", "Line"), ("square", "Square"), ("cross", "Cross")];

/// Is this dialog the Bitmap dialog? (A plug-in's own `__spec` dialog never is.)
pub(crate) fn owns(f: &Map<String, Value>) -> bool {
    f.get("__filter").and_then(Value::as_bool) == Some(true) && f.get("__command").and_then(Value::as_str) == Some(COMMAND) && !f.contains_key("__spec")
}

/// Open defaults: Output starts at the document's resolution (Input, as in Photoshop), Custom
/// Pattern at the Patterns panel's pattern.
pub(crate) fn defaults(app: &PhotocraftApp, f: &mut Map<String, Value>) {
    let input = app.session.active().map_or(72.0, |d| d.doc.resolution_dpi);
    f.insert("__inputResolution".into(), json!(input));
    f.insert("resolution".into(), json!(input));
    match photocraft_engine::pattern_cmds::resolve(&app.session, "") {
        Some(p) => f.insert("pattern".into(), json!(p.id)),
        None => f.remove("pattern"),
    };
    f.entry("__resUnit").or_insert_with(|| json!("ppi"));
    f.entry("__freqUnit").or_insert_with(|| json!("lpi"));
}

fn num(f: &Map<String, Value>, key: &str, default: f32) -> f32 {
    f.get(key).and_then(Value::as_f64).map_or(default, |v| v as f32)
}

fn text(f: &Map<String, Value>, key: &str, default: &str) -> String {
    f.get(key).and_then(Value::as_str).unwrap_or(default).to_string()
}

fn label(ui: &mut egui::Ui, text: &str) {
    let t = Tokens::get(ui.ctx());
    ui.add_sized([96.0, 20.0], egui::Label::new(egui::RichText::new(text).color(t.text_dim)).truncate());
}

/// A value stored in per-inch units, edited in per-inch or per-centimeter ones: `field` holds the
/// per-inch value, `unit` the view's unit (`per_cm` when its value is per centimeter).
fn per_unit_row(ui: &mut egui::Ui, f: &mut Map<String, Value>, field: &str, range: (f32, f32), default: f32, unit: &str, units: [(&str, &str); 2]) {
    let per_inch = num(f, field, default);
    let mut cur = text(f, unit, units[0].0);
    let per_cm = cur == units[1].0;
    let k = if per_cm { CM } else { 1.0 };
    let mut shown = per_inch / k;
    if crate::widgets::value_field(ui, &mut shown, range.0 / k..=range.1 / k, "", 70.0).changed() {
        f.insert(field.into(), json!(((shown * k).clamp(range.0, range.1) * 1000.0).round() / 1000.0));
    }
    let opts: Vec<(String, &str)> = units.iter().map(|(id, l)| (id.to_string(), *l)).collect();
    if crate::widgets::dropdown(ui, &format!("bitmap-{unit}"), &mut cur, &opts, 150.0) {
        f.insert(unit.into(), json!(cur));
    }
}

/// The dialog body.
pub(crate) fn body(app: &PhotocraftApp, ui: &mut egui::Ui, f: &mut Map<String, Value>) {
    let t = Tokens::get(ui.ctx());
    ui.spacing_mut().item_spacing.y = 6.0;
    let res_units = [("ppi", tl!("Pixels/Inch")), ("ppcm", tl!("Pixels/Centimeter"))];

    crate::widgets::section_label(ui, tl!("Resolution"));
    crate::widgets::hairline(ui);
    ui.horizontal(|ui| {
        label(ui, tl!("Input:"));
        let input = num(f, "__inputResolution", 72.0);
        let (v, unit) = if text(f, "__resUnit", "ppi") == "ppcm" { (input / CM, res_units[1].1) } else { (input, res_units[0].1) };
        ui.label(egui::RichText::new(format!("{} {unit}", crate::widgets::fmt_num_prec(f64::from(v), 3))).color(t.text));
    });
    ui.horizontal(|ui| {
        label(ui, tl!("Output:"));
        let input = num(f, "__inputResolution", 72.0);
        per_unit_row(ui, f, "resolution", RESOLUTION, input, "__resUnit", res_units);
    });

    ui.add_space(4.0);
    crate::widgets::section_label(ui, tl!("Method"));
    crate::widgets::hairline(ui);
    let mut method = text(f, "method", "diffusion");
    ui.horizontal(|ui| {
        label(ui, tl!("Use:"));
        let opts: Vec<(String, &str)> = METHODS.iter().map(|(id, l)| (id.to_string(), tl!(l))).collect();
        if crate::widgets::dropdown(ui, "bitmap-method", &mut method, &opts, 190.0) {
            f.insert("method".into(), json!(method));
        }
    });
    // Photoshop greys the pattern out unless the method is Custom Pattern.
    ui.horizontal(|ui| {
        ui.add_enabled_ui(method == "customPattern", |ui| {
            label(ui, tl!("Custom Pattern:"));
            let pat = text(f, "pattern", "");
            if app.session.patterns.items.is_empty() {
                ui.label(egui::RichText::new(tl!("No patterns")).color(t.text_faint));
            } else if let Some(id) = crate::preset_panels::pattern_picker(app, ui, &pat) {
                f.insert("pattern".into(), json!(id));
            }
        });
    });

    if method == "halftone" {
        ui.add_space(4.0);
        crate::widgets::section_label(ui, tl!("Halftone Screen"));
        crate::widgets::hairline(ui);
        ui.horizontal(|ui| {
            label(ui, tl!("Frequency:"));
            per_unit_row(ui, f, "frequency", FREQUENCY, 53.0, "__freqUnit", [("lpi", tl!("Lines/Inch")), ("lpcm", tl!("Lines/Centimeter"))]);
        });
        ui.horizontal(|ui| {
            label(ui, tl!("Angle:"));
            let mut angle = num(f, "angle", 45.0);
            if crate::widgets::value_field(ui, &mut angle, -180.0..=180.0, "°", 70.0).changed() {
                f.insert("angle".into(), json!((angle.clamp(-180.0, 180.0) * 10.0).round() / 10.0));
            }
        });
        ui.horizontal(|ui| {
            label(ui, tl!("Shape:"));
            let mut shape = text(f, "shape", "round");
            let opts: Vec<(String, &str)> = SHAPES.iter().map(|(id, l)| (id.to_string(), tl!(l))).collect();
            if crate::widgets::dropdown(ui, "bitmap-shape", &mut shape, &opts, 150.0) {
                f.insert("shape".into(), json!(shape));
            }
        });
    }

    if let Some(mut preview) = f.get("__preview").and_then(Value::as_bool) {
        ui.add_space(4.0);
        crate::widgets::checkbox(ui, &mut preview, tl!("Preview"));
        f.insert("__preview".into(), json!(preview));
    }
}

#[cfg(test)]
#[path = "bitmap_dialog_tests.rs"]
mod tests;
