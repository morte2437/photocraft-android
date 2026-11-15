//! Fit Image's bounds belong to its dialog. The engine still accepts an unconstrained side.

use std::cell::Cell;

use serde_json::{Value, json};

use super::Fields;
use crate::panels::options_bar::range_alert;
use crate::state::UiState;

const COMMAND: &str = "file.automate.fitImage";
const RANGE: std::ops::RangeInclusive<f64> = 1.0..=300_000.0;
const WIDTH: (&str, &str) = ("width", "__fitImageWidth");
const HEIGHT: (&str, &str) = ("height", "__fitImageHeight");
const SIDES: [(&str, &str); 2] = [WIDTH, HEIGHT];

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Alert {
    Closest,
    Invalid,
}

impl Alert {
    pub(super) fn message(self) -> &'static str {
        match self {
            Self::Closest => "A value between 1 and 300000 is required. Closest value inserted.",
            Self::Invalid => "Invalid numeric entry. A value between 1 and 300000 is required. Previous value inserted.",
        }
    }
}

pub(super) fn owns(fields: &Fields) -> bool {
    fields.contains_key("__form") && fields.get("__command").and_then(Value::as_str) == Some(COMMAND)
}

fn bounded(number: Option<f64>, alert: &Cell<Option<Alert>>) -> Option<f64> {
    let Some(number) = number.filter(|value| value.is_finite()) else {
        alert.set(Some(Alert::Invalid));
        return None;
    };
    if !RANGE.contains(&number) {
        alert.set(Some(Alert::Closest));
    }
    Some(number.clamp(*RANGE.start(), *RANGE.end()))
}

/// Covers dialog confirmation when automation supplies values without rendering the fields.
pub(super) fn preflight(ui: &mut UiState, id: u64) -> Result<(), String> {
    if let Some(dialog) = ui.dialog_mut(id).filter(|dialog| owns(&dialog.fields))
        && let Some(alert) = validate(&mut dialog.fields)
    {
        range_alert(ui, alert.message());
        if alert == Alert::Invalid {
            return Err(tl!(alert.message()).into());
        }
    }
    Ok(())
}

fn validate(fields: &mut Fields) -> Option<Alert> {
    let mut result = None;
    for (key, accepted) in SIDES {
        let alert = Cell::new(None);
        let number = fields.get(key).and_then(|value| value.as_f64().or_else(|| value.as_str().and_then(crate::widgets::parse_num)));
        let value = bounded(number, &alert)
            .or_else(|| fields.get(accepted).and_then(Value::as_f64))
            .or_else(|| fields.get("__formOrig")?.get(key)?.as_f64())
            .unwrap_or(*RANGE.start())
            .clamp(*RANGE.start(), *RANGE.end()) as i64;
        fields.insert(key.into(), json!(value));
        fields.insert(accepted.into(), json!(value));
        // An invalid entry needs acknowledgment before the dialog can run.
        if result != Some(Alert::Invalid) {
            result = alert.get().or(result);
        }
    }
    result
}

pub(super) fn finish(ui: &mut UiState, interactive: bool, alert: Option<Alert>, outcome: Option<bool>) -> Option<bool> {
    if interactive
        && outcome != Some(false)
        && let Some(alert) = alert
    {
        range_alert(ui, alert.message());
        if alert == Alert::Invalid {
            return None;
        }
    }
    outcome
}

pub(super) fn body(ui: &mut egui::Ui, fields: &mut Fields) -> Option<Alert> {
    let mut alert = None;
    egui::Grid::new("form-dialog").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
        for key in crate::view_cmds::form_keys(fields) {
            match key.as_str() {
                "width" => alert = side(ui, fields, WIDTH, tl!("Width")).or(alert),
                "height" => alert = side(ui, fields, HEIGHT, tl!("Height")).or(alert),
                "dontEnlarge" => {
                    ui.label("");
                    let mut enabled = fields.get(&key).and_then(Value::as_bool).unwrap_or(false);
                    ui.checkbox(&mut enabled, tl!("Dont enlarge"));
                    fields.insert(key, json!(enabled));
                }
                _ => continue,
            }
            ui.end_row();
        }
    });
    alert
}

fn side(ui: &mut egui::Ui, fields: &mut Fields, (key, accepted): (&str, &str), label: &str) -> Option<Alert> {
    let input = fields.get(key).and_then(Value::as_i64);
    let mut value = input.or_else(|| fields.get(accepted).and_then(Value::as_i64)).or_else(|| fields.get("__formOrig")?.get(key)?.as_i64()).unwrap_or(1);
    let alert = Cell::new(None);
    let parsed = Cell::new(false);
    ui.label(label);
    crate::field_tab::register(ui.ctx(), ui.next_auto_id());
    let response = ui
        .add(egui::DragValue::new(&mut value).range(RANGE).clamp_existing_to_range(false).update_while_editing(false).custom_parser(|text| {
            parsed.set(true);
            bounded(crate::widgets::parse_num(text), &alert)
        }))
        .accessible_name(label);
    if response.changed() || parsed.get() || input.is_some_and(|input| RANGE.contains(&(input as f64))) {
        let value = value.clamp(*RANGE.start() as i64, *RANGE.end() as i64);
        fields.insert(key.into(), json!(value));
        fields.insert(accepted.into(), json!(value));
    }
    alert.get()
}
