//! Edit › Stroke…: a Photoshop-style options dialog for the existing `edit.stroke` command.
//! The engine remains directly callable with parameters by CLI, MCP and Actions.

use serde_json::{Map, Value, json};

use crate::PhotocraftApp;

pub const COMMAND: &str = "edit.stroke";
const MARK: &str = "__stroke";
const KEYS: [&str; 6] = ["width", "color", "location", "mode", "opacity", "preserveTransparency"];
const LOCATIONS: [(&str, &str); 3] = [("inside", "Inside"), ("center", "Center"), ("outside", "Outside")];

pub fn owns(fields: &Map<String, Value>) -> bool {
    fields.contains_key(MARK)
}

fn hex(c: [f32; 4]) -> String {
    let to_byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    format!("#{:02x}{:02x}{:02x}", to_byte(c[0]), to_byte(c[1]), to_byte(c[2]))
}

fn valid(key: &str, v: &Value) -> bool {
    match key {
        "width" => v.as_f64().is_some_and(|x| x.is_finite() && (1.0..=250.0).contains(&x)),
        "color" => v.as_str().is_some_and(|h| h.len() == 7 && h.starts_with('#') && h[1..].bytes().all(|b| b.is_ascii_hexdigit())),
        "location" => v.as_str().is_some_and(|s| LOCATIONS.iter().any(|(id, _)| *id == s)),
        _ => crate::dialog_blend_ui::valid(key, v),
    }
}

/// Construct a dialog from defaults and only valid, persisted preferences.
pub fn fields(app: &PhotocraftApp) -> Map<String, Value> {
    let mut f = Map::new();
    f.insert(MARK.into(), json!(true));
    f.insert("__command".into(), json!(COMMAND));
    f.insert("__label".into(), json!("Stroke…"));
    f.insert("width".into(), json!(1.0));
    f.insert("color".into(), json!(hex(app.session.tools.foreground)));
    f.insert("location".into(), json!("center"));
    f.insert("mode".into(), json!("normal"));
    f.insert("opacity".into(), json!(100.0));
    f.insert("preserveTransparency".into(), json!(false));
    if let Some(Value::Object(saved)) = app.session.prefs().dialogs.get(COMMAND) {
        for key in KEYS {
            if let Some(value) = saved.get(key).filter(|v| valid(key, v)) {
                f.insert(key.into(), value.clone());
            }
        }
    }
    f
}

/// Open Stroke with the current editing target, retaining the command's channel-view behavior.
pub fn open(app: &mut PhotocraftApp) -> u64 {
    let mut f = fields(app);
    let target = app.with_mask_target(COMMAND, json!({}));
    f.insert("__target".into(), target.get("target").cloned().unwrap_or_else(|| crate::canvas::paint_target(app)));
    if let Some(st) = app.session.active() {
        f.insert("__document".into(), json!(st.doc.id.0));
        f.insert("__layer".into(), json!(st.active_layer));
    }
    app.ui.open_dialog(crate::state::DialogKind::Command, f)
}

/// Pass Stroke choices and the captured editing target, leaving other private markers out.
pub fn params(f: &Map<String, Value>) -> Value {
    let mut p: Map<String, Value> = KEYS.iter().filter_map(|k| f.get(*k).map(|v| (k.to_string(), v.clone()))).collect();
    if let Some(target) = f.get("__target") {
        p.insert("target".into(), target.clone());
    }
    Value::Object(p)
}

/// OK: runs `edit.stroke` with the dialog's settings and, when it succeeds, remembers them for
/// the next time the dialog opens.
pub fn confirm(app: &mut PhotocraftApp, f: &Map<String, Value>) -> Result<Value, String> {
    let st = app.session.active().ok_or(tl!("The document is closed"))?;
    if f.get("__document").and_then(Value::as_u64) != Some(st.doc.id.0) || f.get("__layer") != Some(&json!(st.active_layer)) {
        return Err(tl!("The document or active layer changed. Open Stroke again.").into());
    }
    let result = app.run(COMMAND, params(f));
    if result.is_ok() {
        let remembered = KEYS.iter().filter_map(|k| f.get(*k).map(|v| (k.to_string(), v.clone()))).collect();
        app.session.prefs.edit(|p| p.dialogs.insert(COMMAND.into(), Value::Object(remembered)));
    }
    result
}

/// Label column bounds for [`crate::widgets::label_column_width`]: the English labels' width, and a cap.
const LABEL_MIN_W: f32 = 112.0;
const LABEL_MAX_W: f32 = 240.0;
/// The labels above the Blending section, measured with its own in the current language.
const LABELS: [&str; 3] = ["Width:", "Color:", "Location:"];

/// The Stroke (Width, Color) and Location sections and the shared Blending section, all their
/// labels in one column measured over [`LABELS`] and the Blending labels together.
pub fn body(ui: &mut egui::Ui, f: &mut Map<String, Value>) {
    ui.spacing_mut().item_spacing.y = 6.0;
    let labels: Vec<&str> = LABELS.iter().chain(&crate::dialog_blend_ui::LABELS).copied().map(crate::i18n::t).collect();
    let label_w = crate::widgets::label_column_width(ui, &labels, LABEL_MIN_W, LABEL_MAX_W);
    let label = |ui: &mut egui::Ui, text: &str| {
        crate::widgets::column_label(ui, text, label_w);
    };
    crate::widgets::section_label(ui, tl!("Stroke"));
    crate::widgets::hairline(ui);
    ui.horizontal(|ui| {
        label(ui, tl!("Width:"));
        let mut width = f.get("width").and_then(Value::as_f64).unwrap_or(1.0) as f32;
        if crate::widgets::value_field(ui, &mut width, 1.0..=250.0, "px", 84.0).accessible_name(tl!("Width:")).changed() {
            f.insert("width".into(), json!(width.round()));
        }
    });
    ui.horizontal(|ui| {
        label(ui, tl!("Color:"));
        let current = f.get("color").and_then(Value::as_str).unwrap_or("#000000");
        let rgb = [1usize, 3, 5].map(|i| current.get(i..i + 2).and_then(|b| u8::from_str_radix(b, 16).ok()).unwrap_or(0));
        // As in Photoshop, the swatch opens the Color Picker on the stroke colour.
        if crate::widgets::color_swatch_button(ui, egui::Color32::from_rgb(rgb[0], rgb[1], rgb[2]), tl!("Stroke color")).clicked() {
            crate::color_picker_ui::request(f, tl!("Color Picker (Stroke Color)"));
        }
    });

    ui.add_space(4.0);
    crate::widgets::section_label(ui, tl!("Location"));
    crate::widgets::hairline(ui);
    ui.horizontal(|ui| {
        label(ui, tl!("Location:"));
        let mut location = f.get("location").and_then(Value::as_str).unwrap_or("center").to_string();
        let opts: Vec<(String, &str)> = LOCATIONS.iter().map(|(key, label)| (key.to_string(), tl!(label))).collect();
        if crate::widgets::dropdown(ui, "stroke-location", tl!("Location"), &mut location, &opts, 170.0) {
            f.insert("location".into(), json!(location));
        }
    });
    crate::dialog_blend_ui::body(ui, f, "stroke", label_w);
}

#[cfg(test)]
mod tests;
