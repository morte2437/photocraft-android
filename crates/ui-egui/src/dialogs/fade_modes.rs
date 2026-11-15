//! Edit › Fade's Mode list (#13806): the 27 layer blend modes in the reference application's order
//! and groups, a line between the groups, with the modes the document's colour mode and depth can't
//! fade with greyed (listed, never chosen).

use photocraft_color::{BlendMode, ColorMode, SampleType};
use serde_json::{Map, Value, json};

use super::dialog_blend_ui::mode_key;
use crate::PhotocraftApp;

pub(crate) const COMMAND: &str = "edit.fade";
/// The `mode` keys greyed for the document the dialog opened on.
const GREYED: &str = "__fadeGreyed";

/// The modes the reference application greys in its Fade dialog for a document of `mode` at
/// `depth`. Combinations it wasn't measured in (Grayscale 16/32, Lab 16, CMYK 16, Multichannel,
/// Duotone…) grey none, as before.
pub(crate) fn greyed(mode: ColorMode, depth: SampleType) -> &'static [BlendMode] {
    use BlendMode::*;
    match (mode, depth) {
        (ColorMode::Grayscale, SampleType::U8) => &[DarkerColor, LighterColor, Hue, Saturation, Color, Luminosity],
        (ColorMode::Lab, SampleType::U8) => &[Darken, ColorBurn, Lighten, ColorDodge, Difference, Exclusion, Subtract, Divide],
        (ColorMode::Rgb, SampleType::F32) => {
            &[ColorBurn, LinearBurn, Screen, ColorDodge, Overlay, SoftLight, HardLight, VividLight, LinearLight, PinLight, HardMix, Exclusion]
        }
        _ => &[],
    }
}

/// Opening (and Alt+Reset): note the active document's greyed modes for the list. A remembered
/// mode this document greys starts the dialog at Normal instead, since it can't be chosen.
pub(crate) fn defaults(app: &PhotocraftApp, fields: &mut Map<String, Value>) {
    let greyed = app.session.active().map_or(&[][..], |st| greyed(st.doc.mode, st.doc.depth));
    if mode_of(fields).is_some_and(|m| greyed.contains(&m)) {
        fields.insert("mode".into(), json!(mode_key(BlendMode::Normal)));
    }
    fields.insert(GREYED.into(), json!(greyed.iter().map(|m| mode_key(*m)).collect::<Vec<_>>()));
}

fn mode_of(fields: &Map<String, Value>) -> Option<BlendMode> {
    fields.get("mode").and_then(Value::as_str).and_then(photocraft_engine::commands::blend_from_str)
}

fn greyed_of(fields: &Map<String, Value>) -> Vec<BlendMode> {
    fields
        .get(GREYED)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .filter_map(photocraft_engine::commands::blend_from_str)
        .collect()
}

/// The mode to keep after the list's keys or type-ahead moved from `from` to `to`: `to` unless it
/// is greyed, else the next mode past it the same way that isn't, else `from`. A greyed mode the
/// list didn't move to (written by an agent) stays: the command decides about it.
pub(crate) fn step_past(from: BlendMode, to: BlendMode, greyed: &[BlendMode]) -> BlendMode {
    if to == from || !greyed.contains(&to) {
        return to;
    }
    let list = BlendMode::LAYER_MODES;
    let at = |m: BlendMode| list.iter().position(|l| *l == m);
    let (Some(a), Some(b)) = (at(from), at(to)) else { return from };
    let open = |m: &&BlendMode| !greyed.contains(m);
    let next = if b < a { list.get(..b).and_then(|l| l.iter().rev().find(open)) } else { list.get(b + 1..).and_then(|l| l.iter().find(open)) };
    next.copied().unwrap_or(from)
}

/// The Mode row of the Fade dialog, its label `label` in the schema dialog's style.
pub(crate) fn row(ui: &mut egui::Ui, f: &mut Map<String, Value>, label: &str) {
    let t = crate::theme::Tokens::get(ui.ctx());
    let greyed = greyed_of(f);
    let known = mode_of(f);
    let before = known.unwrap_or(BlendMode::Normal);
    let mut mode = before;
    let opts: Vec<(BlendMode, &str)> = BlendMode::LAYER_MODES.iter().map(|m| (*m, m.label())).collect();
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(label).color(t.text_dim));
        crate::widgets::dropdown_wheel_grouped_enabled(
            ui,
            "flt-edit.fade-mode",
            label,
            &mut mode,
            &opts,
            170.0,
            |m| crate::panels::starts_blend_group(*m),
            |m| !greyed.contains(m),
        );
    });
    let mode = step_past(before, mode, &greyed);
    // A mode no list names (an agent's typo) stays until one is picked: the command reports it.
    if known.is_some() || mode != before {
        f.insert("mode".into(), json!(mode_key(mode)));
    }
}

#[cfg(test)]
#[path = "fade_mode_list_tests.rs"]
mod tests;
