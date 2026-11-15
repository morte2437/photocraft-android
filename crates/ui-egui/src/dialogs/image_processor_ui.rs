//! File Type section of the Image Processor dialog.

use std::cell::Cell;

use serde_json::{Value, json};

use super::Fields;
use crate::{panels::options_bar::range_alert, state::UiState};

const FORMATS: [&str; 3] = ["jpeg", "psd", "tiff"];
const NO_FORMAT: &str = "You must save to at least one file type.";

pub(super) fn owns(fields: &Fields) -> bool {
    fields.contains_key("__form") && fields.get("__command").and_then(Value::as_str) == Some("file.scripts.imageProcessor") && !fields.contains_key("format")
}

pub(super) fn preflight(ui: &mut UiState, id: u64) -> Result<(), String> {
    if let Some(dialog) = ui.dialog_mut(id).filter(|dialog| owns(&dialog.fields))
        && !FORMATS.iter().any(|key| dialog.fields.get(*key).is_some_and(|options| options.is_object() && options.get("enabled") != Some(&Value::Bool(false))))
    {
        range_alert(ui, NO_FORMAT);
        return Err(tl!(NO_FORMAT).into());
    }
    Ok(())
}

pub(super) fn finish(ui: &mut UiState, interactive: bool, alert: Option<&str>, outcome: Option<bool>) -> Option<bool> {
    if interactive
        && outcome != Some(false)
        && let Some(alert) = alert
    {
        range_alert(ui, alert);
        return None;
    }
    outcome
}

pub(super) fn body(ui: &mut egui::Ui, fields: &mut Fields) -> Option<&'static str> {
    let height = (ui.ctx().content_rect().height() - super::DIALOG_CHROME).max(120.0);
    egui::ScrollArea::vertical()
        .id_salt("image-processor")
        .max_height(height)
        .show(ui, |ui| {
            crate::view_cmds::form_body(ui, fields);
            formats(ui, fields)
        })
        .inner
}

fn formats(ui: &mut egui::Ui, fields: &mut Fields) -> Option<&'static str> {
    ui.add_space(8.0);
    crate::widgets::hairline(ui);
    ui.label(egui::RichText::new(tl!("File Type")).strong());
    let mut alert = None;
    for (key, label) in FORMATS.into_iter().zip([tl!("Save as JPEG"), tl!("Save as PSD"), tl!("Save as TIFF")]) {
        let Some(options) = fields.get_mut(key).and_then(Value::as_object_mut) else { continue };
        ui.push_id(key, |ui| {
            let enabled = checkbox(ui, options, "enabled", label);
            ui.indent("format-options", |ui| {
                ui.add_enabled_ui(enabled, |ui| {
                    match key {
                        "jpeg" => {
                            ui.horizontal(|ui| number(ui, options, "quality", tl!("Quality"), 5.0, 12.0, &mut alert));
                            checkbox(ui, options, "convertToSrgb", tl!("Convert Profile to sRGB"));
                        }
                        "psd" => {
                            checkbox(ui, options, "maximizeCompatibility", tl!("Maximize Compatibility"));
                        }
                        _ => {
                            checkbox(ui, options, "lzw", tl!("LZW Compression"));
                        }
                    }
                    let resize = checkbox(ui, options, "resize", tl!("Resize to Fit"));
                    ui.add_enabled_ui(resize, |ui| {
                        ui.horizontal(|ui| {
                            number(ui, options, "width", tl!("W"), 1.0, 30_000.0, &mut alert);
                            number(ui, options, "height", tl!("H"), 1.0, 30_000.0, &mut alert);
                            ui.label(tl!("px"));
                        });
                    });
                });
            });
        });
        ui.add_space(4.0);
    }
    alert
}

fn checkbox(ui: &mut egui::Ui, options: &mut Fields, key: &str, label: &str) -> bool {
    let mut enabled = options.get(key).and_then(Value::as_bool).unwrap_or(false);
    if ui.checkbox(&mut enabled, label).changed() {
        options.insert(key.into(), json!(enabled));
    }
    enabled
}

#[cfg(test)]
mod tests;

fn number(ui: &mut egui::Ui, options: &mut Fields, key: &str, label: &str, default: f64, max: f64, alert: &mut Option<&'static str>) {
    let mut value = options.get(key).and_then(Value::as_f64).unwrap_or(default);
    let warning = Cell::new(None);
    ui.label(label);
    if ui.is_enabled() {
        crate::field_tab::register(ui.ctx(), ui.next_auto_id());
    }
    ui.add(egui::DragValue::new(&mut value).range(1.0..=max).max_decimals(if key == "quality" { 8 } else { 0 }).update_while_editing(false).custom_parser(
        |text| {
            let parsed = crate::widgets::parse_num(text);
            warning.set(match parsed {
                None => Some(invalid_entry(key)),
                Some(value) if !(1.0..=max).contains(&value) => Some(closest_value(key)),
                Some(_) => None,
            });
            parsed.map(|n| if key == "quality" { n } else { n.trunc() })
        },
    ))
    .accessible_name(label);
    options.insert(key.into(), json!(value));
    *alert = warning.get().or(*alert);
}

fn invalid_entry(key: &str) -> &'static str {
    if key == "quality" {
        "Invalid numeric entry. A value between 1 and 12 is required. Previous value inserted."
    } else {
        "Invalid numeric entry. A value between 1 and 30000 is required. Previous value inserted."
    }
}

fn closest_value(key: &str) -> &'static str {
    if key == "quality" {
        "A value between 1 and 12 is required. Closest value inserted."
    } else {
        "A value between 1 and 30000 is required. Closest value inserted."
    }
}
