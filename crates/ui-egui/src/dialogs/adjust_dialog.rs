//! Modal Image › Adjustments dialogs (Levels, Curves, Hue/Saturation, …) built from the same
//! editors as the adjustment-layer Properties panel (`adjust_editors`).
//!
//! The dialog's fields are the command's complete parameter set (so automation can read and set
//! them with `ui.dialog.set`) plus private `__` keys. While it is open the canvas previews the
//! settings as a temporary adjustment layer clipped to the target (`adjust_preview`, composited on
//! the GPU); where that can't match the command, the *real command* runs on the proxy document
//! through the filter-preview machinery (`__filter` + `__preview`). OK runs the command once (one
//! history step) and Cancel drops the preview, leaving the document untouched.

use serde_json::{Map, Value, json};

use crate::PhotocraftApp;
use crate::adjust_editors::{self, EditorCx};
use crate::tone::{self, HistSource};

const PREFIX: &str = "image.adjustments.";
const CURVE_PICKER: &str = "__curvePicker";
const CURVE_PICKER_BASE: &str = "__curvePickerBase";
/// Set once a Photo Filter dialog has drawn: its editor memory was refreshed for this opening.
const PHOTO_FILTER_FRESH: &str = "__photoFilterFresh";

fn picker(value: Option<&Value>) -> Option<photocraft_engine::adjust_params::CurvesEyedropper> {
    use photocraft_engine::adjust_params::CurvesEyedropper;
    match value.and_then(Value::as_str) {
        Some("black") => Some(CurvesEyedropper::Black),
        Some("gray") => Some(CurvesEyedropper::NeutralGray),
        Some("white") => Some(CurvesEyedropper::White),
        _ => None,
    }
}

pub(crate) fn picker_supported(mode: photocraft_doc::ColorMode, eyedropper: photocraft_engine::adjust_params::CurvesEyedropper) -> bool {
    use photocraft_engine::adjust_params::CurvesEyedropper;
    mode == photocraft_doc::ColorMode::Rgb || (mode == photocraft_doc::ColorMode::Grayscale && eyedropper != CurvesEyedropper::NeutralGray)
}

/// The kind of an adjustment dialog with Set Black/Neutral Gray/White Point eyedroppers (Levels,
/// Curves), if `fields` are one.
fn picker_kind(fields: &Map<String, Value>) -> Option<&str> {
    fields.get("__adjust").and_then(Value::as_str).filter(|k| matches!(*k, "levels" | "curves"))
}

/// Whether the top modal has a supported Levels or Curves eyedropper armed.
pub fn picker_armed(app: &PhotocraftApp) -> bool {
    let Some(dialog) = app.ui.dialogs.last().filter(|d| picker_kind(&d.fields).is_some()) else { return false };
    let Some(eyedropper) = picker(dialog.fields.get(CURVE_PICKER)) else { return false };
    let kind = picker_kind(&dialog.fields).unwrap_or_default();
    if target_context(app, &dialog.fields).is_err() {
        return false;
    }
    app.session.active().is_some() && picker_supported(curve_mode(app, kind, &dialog.fields), eyedropper)
}

/// Sample the committed colour composite or targeted grayscale plane and update only the
/// open Levels or Curves dialog. Returns true when an armed picker consumed the pointer event,
/// even if the point was outside the image or transparent.
pub fn sample_at(app: &mut PhotocraftApp, x: f64, y: f64) -> bool {
    let Some((id, kind, eyedropper, mut base)) = app.ui.dialogs.last().and_then(|dialog| {
        let kind = picker_kind(&dialog.fields)?.to_string();
        let eyedropper = picker(dialog.fields.get(CURVE_PICKER))?;
        let base = dialog.fields.get(CURVE_PICKER_BASE).cloned().unwrap_or_else(|| crate::filter_dialog::params_of(&dialog.fields));
        Some((dialog.id, kind, eyedropper, base))
    }) else {
        return false;
    };
    let Some(dialog) = app.ui.dialogs.last() else { return true };
    if let Err(error) = target_context(app, &dialog.fields) {
        app.ui.status = error.to_string();
        return true;
    }
    // Automation can repair or retarget a dialog after arming the picker. Keep the cached
    // adjustment baseline, but sample the address that the current controls and OK use.
    if let Some(object) = base.as_object_mut() {
        for key in ["target", "layer", "document"] {
            if let Some(value) = dialog.fields.get(key) {
                object.insert(key.into(), value.clone());
            } else {
                object.remove(key);
            }
        }
    }
    let mode = curve_mode(app, &kind, &dialog.fields);
    let gray = isolated(app, &dialog.fields).is_some_and(|c| c.gray.is_some());
    if !picker_supported(mode, eyedropper) {
        return true;
    }
    let adjustment = if gray {
        let mut params = app.with_mask_target(&format!("{PREFIX}{kind}"), base.clone());
        let point = match eyedropper {
            photocraft_engine::adjust_params::CurvesEyedropper::Black => "black",
            photocraft_engine::adjust_params::CurvesEyedropper::NeutralGray => "gray",
            photocraft_engine::adjust_params::CurvesEyedropper::White => "white",
        };
        let Some(object) = params.as_object_mut() else {
            app.ui.status = "Invalid eyedropper parameters".into();
            return true;
        };
        object.insert("eyedropper".into(), json!({"point": point, "at": [x,y], "size": app.ui.tool_options.eyedropper_size}));
        photocraft_engine::adjust_params::destructive_eyedropper(&app.session, &kind, &params)
            .and_then(|a| a.ok_or_else(|| photocraft_engine::EngineError::Other("missing eyedropper adjustment".into())))
    } else {
        let Some(sample) = crate::canvas::composite_color(app, x, y) else { return true };
        photocraft_engine::adjust_params::adjustment_eyedropper(&kind, &base, eyedropper, sample, mode)
    };
    let params = match adjustment {
        // Scalar samples retain their native precision. The colour picker keeps its existing
        // hundredth-level curve-point convention, independently of source sample depth.
        Ok(adjustment) => {
            if gray {
                photocraft_engine::adjust_params::to_dialog_params(&adjustment)
            } else {
                photocraft_engine::adjust_params::to_params(&adjustment)
            }
        }
        Err(error) => {
            app.ui.status = error.to_string();
            return true;
        }
    };
    let Some(dialog) = app.ui.dialog_mut(id) else { return true };
    dialog.fields.insert(CURVE_PICKER_BASE.into(), base);
    if let Value::Object(params) = params {
        keep_context(&mut dialog.fields);
        dialog.fields.extend(params);
    }
    true
}

/// Whether the top modal is the Curves dialog.
pub fn curves_on_top(app: &PhotocraftApp) -> bool {
    app.ui.dialogs.last().is_some_and(|d| d.fields.get("__adjust").and_then(Value::as_str) == Some("curves"))
}

/// ⌘/Ctrl-click on the image with Curves on top (#7325): a point on the curve shown in the dialog
/// at the clicked pixel's tone, at the output the curve gives there rounded to a whole level, so the
/// image barely changes (where the curve is clipped at 0 or 255 the point re-shapes it, as a click on
/// the graph there does); with `every_channel` (Shift) one on each colour channel instead, at that
/// channel's value.
/// The pixel comes from the same merged composite as the eyedroppers. Returns true when Curves is
/// on top: the click is the dialog's even when nothing is added (pencil curve, Lab or CMYK, a mask
/// or saved channel target, outside the image, a full curve).
pub fn add_point_at(app: &mut PhotocraftApp, ctx: &egui::Context, x: f64, y: f64, every_channel: bool) -> bool {
    if !curves_on_top(app) {
        return false;
    }
    let Some((id, mut values)) = app.ui.dialogs.last().map(|d| (d.id, crate::filter_dialog::params_of(&d.fields))) else { return false };
    if values.get("pencil").and_then(Value::as_bool) == Some(true) {
        return true;
    }
    if values.get("target").is_some_and(|target| matches!(target.as_str(), Some("mask" | "filterMask" | "quickMask")) || target.get("channel").is_some()) {
        return true;
    }
    let Some(mode) = app.session.active().map(|s| s.doc.mode) else { return true };
    if !matches!(mode, photocraft_doc::ColorMode::Rgb | photocraft_doc::ColorMode::Grayscale) {
        app.ui.status = tl!("Unavailable in this color mode").into();
        return true;
    }
    // A mask, saved channel or Quick Mask holds its own gray levels, not the composite's.
    let isolated = app.ui.dialogs.last().and_then(|dialog| isolated(app, &dialog.fields));
    if isolated.as_ref().is_some_and(|c| c.gray.is_some()) {
        return true;
    }
    let Some(sample) = crate::canvas::composite_color(app, x, y) else { return true };
    let gray = adjust_editors::is_gray(mode);
    let (shown, shown_key) = match &isolated {
        Some(c) if c.components.is_some() => {
            let ch: usize = ctx.data(|d| d.get_temp(dialog_mem("curves").with("curves-ch"))).unwrap_or(0);
            let keys = photocraft_engine::adjust_params::channel_keys(adjust_editors::space_of(&values));
            let key = ch.checked_sub(1).and_then(|i| c.components.and_then(|colors| colors.indices().nth(i))).and_then(|i| keys.get(i)).copied().unwrap_or("");
            (ch, key)
        }
        Some(c) => (0, c.key),
        None => {
            let ch: usize = ctx.data(|d| d.get_temp(dialog_mem("curves").with("curves-ch"))).unwrap_or(0);
            let chans = adjust_editors::tone_channels(adjust_editors::space_of(&values), gray);
            (ch, chans.get(ch).map_or("", |c| c.key))
        }
    };
    let keys: &[&str] = if every_channel && !gray && isolated.is_none() { &["red", "green", "blue"] } else { &[shown_key] };
    let mut selected = None;
    let mut changed = false;
    for &key in keys {
        let level = match key {
            "red" => sample[0],
            "green" => sample[1],
            "blue" => sample[2],
            // The composite's tone: Gray's mean, as the Grayscale eyedroppers read it, and RGB's
            // Rec. 709 luma, as the Neutral Gray eyedropper does (`adjust_params::rgb_luma`).
            _ if gray => (sample[0] + sample[1] + sample[2]) / 3.0,
            _ => sample[0] * 0.2126 + sample[1] * 0.7152 + sample[2] * 0.0722,
        };
        let input = (level.clamp(0.0, 1.0) * 255.0).round();
        let field = if key.is_empty() { "points" } else { key };
        let mut pts = adjust_editors::read_curve(&values, field);
        let curve: Vec<_> = pts.iter().map(|q| photocraft_doc::adjust::CurvePoint { input: q[0] / 255.0, output: q[1] / 255.0 }).collect();
        let lut = photocraft_compose::adjust::curve_lut(&curve);
        let at = (input / 255.0 * lut.len().saturating_sub(1) as f32).round() as usize;
        let Some(output) = lut.get(at).map(|o| (o * 255.0).round()) else { continue };
        let before = pts.len();
        let Some(i) = crate::point_curve::insert(&mut pts, [input, output]) else { continue };
        if pts.len() > before {
            // Whole levels, as the editor writes a point clicked on the graph.
            values[field] = json!(pts.iter().map(|q| [q[0].round(), q[1].round()]).collect::<Vec<_>>());
            changed = true;
        }
        if key == shown_key {
            selected = Some(i);
        }
    }
    if let Some(i) = selected {
        let state = dialog_mem("curves").with("curves");
        ctx.data_mut(|d| {
            let mut st: crate::state::CurvesEditorState = d.get_temp(state).unwrap_or_default();
            st.channel = shown;
            st.gesture = crate::state::PointCurveState { selected: Some(i), drag: None };
            d.insert_temp(state, st);
        });
    }
    if changed
        && let Some(dialog) = app.ui.dialog_mut(id)
        && let Value::Object(v) = values
    {
        keep_context(&mut dialog.fields);
        dialog.fields.extend(v);
        if picker(dialog.fields.get(CURVE_PICKER)).is_some() {
            let base = crate::filter_dialog::params_of(&dialog.fields);
            dialog.fields.insert(CURVE_PICKER_BASE.into(), base);
        }
    }
    true
}

pub(crate) fn picker_button(ui: &mut egui::Ui, selected: bool, enabled: bool, tooltip: &str, color: egui::Color32) -> egui::Response {
    let response = ui.add_enabled_ui(enabled, |ui| crate::icons::button(ui, "pipette", 28.0, selected, tooltip)).inner;
    let tokens = crate::theme::Tokens::get(ui.ctx());
    ui.painter().circle_filled(response.rect.right_bottom() - egui::vec2(5.5, 5.5), 3.5, color);
    ui.painter().circle_stroke(response.rect.right_bottom() - egui::vec2(5.5, 5.5), 3.5, egui::Stroke::new(1.0, tokens.field_border));
    response.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, enabled, selected, tooltip));
    response
}

fn picker_controls(ui: &mut egui::Ui, fields: &mut Map<String, Value>, mode: photocraft_doc::ColorMode) {
    use photocraft_engine::adjust_params::CurvesEyedropper;
    let current = picker(fields.get(CURVE_PICKER));
    let mut chosen = None;
    ui.horizontal(|ui| {
        let tokens = crate::theme::Tokens::get(ui.ctx());
        ui.label(egui::RichText::new(tl!("Eyedroppers:")).color(tokens.text_dim).size(12.0));
        // The swatch is the tone the picker maps the sample to (image data, like the Curves
        // gradient bars), not chrome, so it is the same in every theme; its ring is a token.
        for (id, eyedropper, label, tone) in [
            ("black", CurvesEyedropper::Black, tl!("Set Black Point"), 0),
            ("gray", CurvesEyedropper::NeutralGray, tl!("Set Neutral Gray Point"), 128),
            ("white", CurvesEyedropper::White, tl!("Set White Point"), 255),
        ] {
            let enabled = picker_supported(mode, eyedropper);
            if picker_button(ui, current == Some(eyedropper), enabled, label, egui::Color32::from_gray(tone)).clicked() {
                chosen = Some((id, eyedropper));
            }
        }
        if !matches!(mode, photocraft_doc::ColorMode::Rgb | photocraft_doc::ColorMode::Grayscale) {
            ui.label(egui::RichText::new(tl!("Unavailable in this color mode")).color(tokens.text_faint).size(11.0));
        }
    });
    if let Some((id, selected)) = chosen
        && current != Some(selected)
    {
        let base = crate::filter_dialog::params_of(fields);
        fields.insert(CURVE_PICKER.into(), json!(id));
        fields.insert(CURVE_PICKER_BASE.into(), base);
    }
}

/// Whether `command` opens an adjustment dialog (when run without params).
pub fn has_dialog(command: &str) -> bool {
    command.strip_prefix(PREFIX).is_some_and(adjust_editors::has_editor)
}

pub fn owns(fields: &Map<String, Value>) -> bool {
    fields.contains_key("__adjust")
}

/// Root of the dialog editor's view state in egui memory.
pub(crate) fn dialog_mem(kind: &str) -> egui::Id {
    egui::Id::unique(("adjust-dialog", kind))
}

/// Whether holding Alt turns Cancel into Reset for this adjustment dialog (every kind).
pub fn resets(fields: &Map<String, Value>) -> bool {
    owns(fields)
}

/// Alt+Cancel (Reset): engine defaults for the kind; Color Balance keeps Preserve Luminosity and
/// shows Midtones (Photoshop 25.4). Dialog stays open.
pub fn reset(app: &PhotocraftApp, ctx: &egui::Context, fields: &mut Map<String, Value>) {
    if !owns(fields) {
        return;
    }
    let kind = fields.get("__adjust").and_then(Value::as_str).unwrap_or_default().to_string();
    if matches!(kind.as_str(), "levels" | "curves") && target_context(app, fields).is_err() {
        return;
    }
    let mode = curve_mode(app, &kind, fields);
    // Color Balance: Photoshop keeps the user's Preserve Luminosity across Reset.
    let preserve = if kind == "colorBalance" { fields.get("preserveLuminosity").cloned() } else { None };
    let Ok(defaults) = photocraft_engine::adjust_params::default_for(&kind, mode) else { return };
    let values = photocraft_engine::adjust_params::to_params(&defaults);
    // Replace parameter keys; keep private `__` chrome (preview, command, …).
    keep_context(fields);
    if let Value::Object(v) = values {
        fields.extend(v);
    }
    if let Some(p) = preserve {
        fields.insert("preserveLuminosity".into(), p);
    }
    let mut values = crate::filter_dialog::params_of(fields);
    adjust_editors::reset_adjustment_editor(ctx, &kind, dialog_mem(&kind), &mut values);
    if let Value::Object(v) = values {
        fields.extend(v);
    }
    fields.remove(CURVE_PICKER_BASE);
}

/// Settings an adjustment dialog keeps from its last OK although it opens at its neutral ones:
/// the reference application's Photo Filter reopens at Warming Filter (85) and 25 % every time,
/// but with Preserve Luminosity as last confirmed (measured 2026-11-08, #9879). ⌥ still reopens
/// at all the last settings ([`open_last`]).
const KEEPS_LAST: &[(&str, &str)] = &[("photoFilter", "preserveLuminosity")];

/// Opens the dialog for `command` at the kind's neutral settings, except the [`KEEPS_LAST`] ones,
/// which keep their last confirmed value.
pub fn open(app: &mut PhotocraftApp, command: &str) -> Option<u64> {
    let id = open_neutral(app, command)?;
    let kind = command.strip_prefix(PREFIX).unwrap_or_default();
    let kept: Vec<(String, Value)> = match app.session.prefs().dialogs.get(command) {
        Some(last) => KEEPS_LAST
            .iter()
            .filter(|(k, _)| *k == kind)
            .filter_map(|(_, key)| last.get(*key).filter(|v| v.is_boolean()).map(|v| ((*key).to_owned(), v.clone())))
            .collect(),
        None => Vec::new(),
    };
    if let Some(d) = app.ui.dialog_mut(id) {
        d.fields.extend(kept);
    }
    Some(id)
}

/// Opens a neutral adjustment dialog using the current edit target.
pub fn open_neutral(app: &mut PhotocraftApp, command: &str) -> Option<u64> {
    open_with_params(app, command, &json!({}))
}

/// Seed an Action's dialog only after resolving its recorded target, so document-space
/// defaults cannot overwrite the target's neutral grayscale values.
pub(crate) fn open_with_params(app: &mut PhotocraftApp, command: &str, params: &Value) -> Option<u64> {
    let params = if params.is_null() { json!({}) } else { Value::Object(params.as_object()?.clone()) };
    let params = complete_params(app, command, &params).unwrap_or(params);
    let mut params = app.with_mask_target(command, params);
    if params.get("target").is_none()
        && let Some(colors) = photocraft_engine::color_channels::selected(&app.session)
        && let Some(object) = params.as_object_mut()
    {
        object.insert("target".into(), colors.params());
    }
    open_fields(app, command, params.as_object()?.clone())
}

/// Reopen a captured Smart Filter with document-colour defaults, independent of channel targeting.
/// Only the caller's validated snapshot may supply the private Smart Filter marker.
pub(crate) fn open_smart_filter(app: &mut PhotocraftApp, command: &str, params: &Value, target: Value) -> Option<u64> {
    let completed = complete_params(app, command, params);
    let mut fields = completed.as_ref().unwrap_or(params).as_object().cloned().unwrap_or_default();
    fields.retain(|key, _| !key.starts_with("__"));
    fields.insert("__smartFilter".into(), target);
    let mut fields = seeded_fields(app, command, fields)?;
    let mode = app.session.active()?.doc.mode;
    seed_from_recorded(&mut fields, params, mode);
    Some(app.ui.open_dialog(crate::state::DialogKind::Command, fields))
}

/// Merge target-aware defaults with supplied parameters while preserving trusted modal metadata.
fn open_fields(app: &mut PhotocraftApp, command: &str, fields: Map<String, Value>) -> Option<u64> {
    let fields = seeded_fields(app, command, fields)?;
    Some(app.ui.open_dialog(crate::state::DialogKind::Command, fields))
}

/// Normalize recorded settings once their target resolves, retaining modal addressing/chrome.
fn seeded_fields(app: &PhotocraftApp, command: &str, mut fields: Map<String, Value>) -> Option<Map<String, Value>> {
    let kind = command.strip_prefix(PREFIX).filter(|k| adjust_editors::has_editor(k))?;
    let spec = photocraft_engine::commands::find(command)?;
    let mode = curve_mode(app, kind, &fields);
    let valid_target = !matches!(kind, "levels" | "curves") || target_context(app, &fields).is_ok();
    let defaults = if valid_target { Some(photocraft_engine::adjust_params::default_for(kind, mode).ok()?) } else { None };
    let isolated = matches!(kind, "levels" | "curves").then(|| isolated(app, &fields)).flatten();
    let gray_target = isolated.as_ref().is_some_and(|c| c.gray.is_some());
    let color_set = isolated.and_then(|c| c.components);
    let mut supplied = if gray_target {
        photocraft_engine::adjust_params::gray_tone_params(&Value::Object(std::mem::take(&mut fields))).as_object().cloned().unwrap_or_default()
    } else {
        std::mem::take(&mut fields)
    };
    // Resolve a recorded gray alias before adding defaults: Levels' alias would otherwise
    // override visible control edits, while Curves' default points would hide the alias entirely.
    if valid_target && matches!(kind, "levels" | "curves") && supplied.contains_key("gray") {
        let params = Value::Object(supplied.clone());
        let canonical = if let Some(colors) = color_set {
            photocraft_engine::color_channels::tone_dialog_params(kind, &params, mode, colors)
        } else {
            photocraft_engine::adjust_params::from_params(kind, &params, None, mode).map(|a| photocraft_engine::adjust_params::to_dialog_params(&a))
        };
        if let Ok(Value::Object(canonical)) = canonical {
            supplied = photocraft_engine::adjust_params::gray_tone_params(&params).as_object().cloned().unwrap_or_default();
            supplied.remove("gray");
            supplied.remove("smooth"); // Already applied by parsing; never smooth a second time.
            supplied.extend(canonical);
        }
    }
    fields.insert("__command".into(), json!(command));
    fields.insert("__label".into(), json!(spec.label));
    fields.insert("__adjust".into(), json!(kind));
    fields.insert("__filter".into(), json!(true));
    fields.entry("__preview").or_insert(json!(true));
    if let Some(defaults) = defaults
        && let Value::Object(mut p) = photocraft_engine::adjust_params::to_params(&defaults)
    {
        // An invalid gray alias must reach engine validation: seeded identity points would
        // otherwise override it. Valid aliases were already canonicalized above.
        if kind == "curves" && supplied.get("gray").is_some_and(|v| !v.is_null()) && supplied.get("points").is_none_or(Value::is_null) {
            p.remove("points");
        }
        fields.extend(p);
    }
    fields.extend(supplied);
    fields.insert("__unresolvedTarget".into(), json!(!valid_target));
    Some(fields)
}

/// Validate tone edits before the Smart Filter renderer uses them for preview or commit.
pub(crate) fn smart_filter_params(app: &PhotocraftApp, fields: &Map<String, Value>) -> Result<Value, String> {
    let params = crate::filter_dialog::params_of(fields);
    let kind = fields.get("__adjust").and_then(Value::as_str).unwrap_or_default();
    if matches!(kind, "levels" | "curves") {
        photocraft_engine::adjust_params::from_params(kind, &params, None, curve_mode(app, kind, fields)).map_err(|error| error.to_string())?;
    }
    Ok(params)
}

/// Sets an adjustment dialog's parameters to the complete set `recorded` renders in a `mode`
/// document, so the controls show it and OK keeps it. Laid over the dialog's defaults instead, a
/// default can be read over a recorded key that means the same setting (a Lab `lightness` over
/// `points`). Settings the engine can't read are left as they are, for OK to report on.
fn seed_from_recorded(fields: &mut Map<String, Value>, recorded: &Value, mode: photocraft_doc::ColorMode) {
    let Some(kind) = fields.get("__adjust").and_then(Value::as_str) else { return };
    let Ok(adjustment) = photocraft_engine::adjust_params::from_params(kind, recorded, None, mode) else { return };
    if let Value::Object(mut values) = photocraft_engine::adjust_params::to_dialog_params(&adjustment) {
        // Selective Color's chosen row is editor state, absent from the normalized adjustment.
        // Keep it so an older single-colour record reopens on the colour the user edited.
        if kind == "selectiveColor"
            && let Some(colors) = recorded.get("colors")
        {
            values.insert("colors".into(), colors.clone());
        }
        // The normalized pencil maps already include any recorded smoothing. Replacing the
        // filter parameters drops the old count.
        keep_context(fields);
        fields.extend(values);
    }
}

/// Whether the dialog's captured target is a set of several Lab components, whose Levels and
/// Curves settings are kept as a separate combined record.
fn combined_lab_target(fields: &Map<String, Value>) -> bool {
    photocraft_engine::color_channels::explicit(&crate::filter_dialog::params_of(fields), 3).ok().flatten().is_some_and(|c| c.len() > 1)
}

/// Opens the dialog for `command` at the settings last confirmed in it (Photoshop's ⌥ plus the
/// adjustment's shortcut, ⌥⌘L / ⌥⌘M, #8900), or at the neutral ones when there are none or they
/// don't fit the current colour mode.
pub fn open_last(app: &mut PhotocraftApp, command: &str) -> Option<u64> {
    let id = open_neutral(app, command)?;
    let kind = command.strip_prefix(PREFIX).unwrap_or_default();
    let fields = app.ui.dialog_mut(id)?.fields.clone();
    if matches!(kind, "levels" | "curves") && target_context(app, &fields).is_err() {
        return Some(id);
    }
    let mode = curve_mode(app, kind, &fields);
    let Some(mut last) = app.session.prefs().dialogs.get(command).cloned() else { return Some(id) };
    // Recall settings in the current plane's space, independently of the target they came from.
    if matches!(kind, "levels" | "curves") && mode == photocraft_doc::ColorMode::Grayscale {
        last = photocraft_engine::adjust_params::gray_tone_params(&last);
    }
    let Ok(adjustment) = photocraft_engine::adjust_params::from_params(kind, &last, None, mode) else { return Some(id) };
    if matches!(
        &adjustment,
        photocraft_doc::Adjustment::Levels { space, .. } | photocraft_doc::Adjustment::Curves { space, .. }
            if *space != photocraft_engine::adjust_params::default_space(mode)
    ) {
        return Some(id);
    }
    let mut restored = photocraft_engine::adjust_params::to_dialog_params(&adjustment);
    // The target `open` captured, as `remember` reads it: a viewed layer mask wins over the
    // Channels panel's colour set, so its settings are restored as saved.
    let captured_set = app.ui.dialog_mut(id).is_some_and(|d| combined_lab_target(&d.fields));
    if mode == photocraft_doc::ColorMode::Lab && matches!(kind, "levels" | "curves") && captured_set {
        // The parser validates all records, but ordinary Lab serialization aliases the master
        // to Lightness. A partial set keeps the separately saved combined record.
        let neutral = photocraft_engine::adjust_params::default_for(kind, photocraft_doc::ColorMode::Grayscale)
            .ok()
            .map(|a| photocraft_engine::adjust_params::to_params(&a))
            .unwrap_or(Value::Null);
        let combined = if last.get("__combinedLab").and_then(Value::as_bool) == Some(true) { &last } else { &neutral };
        for key in ["points", "inBlack", "inWhite", "gamma", "outBlack", "outWhite"] {
            if let Some(value) = combined.get(key) {
                restored[key] = value.clone();
            }
        }
    }
    if let (Some(dialog), Value::Object(values)) = (app.ui.dialog_mut(id), restored) {
        keep_context(&mut dialog.fields);
        dialog.fields.extend(values);
    }
    Some(id)
}

/// `params` of the adjustment `command` with the keys the dialog would otherwise fill in wrongly:
/// a recorded Photo Filter that names only its `filter` gets that preset's `color`, not the
/// dialog's neutral one. Every recorded value is kept as it was (a round trip through the engine
/// would rewrite them: float colours as hex, a pencil curve smoothed again). `None` when nothing
/// needs adding, or for params the engine rejects.
pub(crate) fn complete_params(app: &PhotocraftApp, command: &str, params: &Value) -> Option<Value> {
    let kind = command.strip_prefix(PREFIX).filter(|k| *k == "photoFilter")?;
    // A `null` colour reads as none, as the engine reads it.
    let recorded = params.as_object().filter(|p| p.contains_key("filter") && p.get("color").is_none_or(Value::is_null))?;
    let adjustment = photocraft_engine::adjust_params::from_params(kind, params, None, curve_mode(app, kind, recorded)).ok()?;
    let color = photocraft_engine::adjust_params::to_params(&adjustment).get("color").cloned()?;
    let mut done = recorded.clone();
    done.insert("color".into(), color);
    Some(Value::Object(done))
}

/// Remember an adjustment dialog's settings after a successful OK, for [`open_last`]. Editing a
/// smart filter's settings is not a use of the dialog's own (as `filter_dialog::remember`).
pub(crate) fn remember(app: &mut PhotocraftApp, fields: &Map<String, Value>) {
    if fields.contains_key("__smartFilter") {
        return;
    }
    let Some(command) = fields.get("__command").and_then(Value::as_str).filter(|c| has_dialog(c)) else { return };
    let mut values = crate::filter_dialog::params_of(fields);
    if let Some(settings) = values.as_object_mut() {
        for key in ["target", "layer", "document"] {
            settings.remove(key);
        }
    }
    if app.session.active().is_some_and(|st| st.doc.mode == photocraft_doc::ColorMode::Lab)
        && combined_lab_target(fields)
        && matches!(command.strip_prefix(PREFIX), Some("levels" | "curves"))
    {
        values["__combinedLab"] = json!(true);
    }
    app.session.prefs.edit(|prefs| prefs.dialogs.insert(command.into(), values));
}

/// The Levels/Curves dialog's target from panel state or explicit parameters, including filter masks.
/// Re-editing a captured Smart Filter always exposes its document-colour controls.
fn isolated(app: &PhotocraftApp, fields: &Map<String, Value>) -> Option<photocraft_engine::channel_cmds::IsolatedCurveChannel> {
    target_context(app, fields).ok()?.channel
}

/// Resolve layer selectors and target validity in the engine without mutating panel selection.
/// A captured Smart Filter supplies its own layer and keeps document-colour controls.
fn target_context(app: &PhotocraftApp, fields: &Map<String, Value>) -> photocraft_engine::Result<photocraft_engine::channel_cmds::ToneTarget> {
    if let Some(filter) = fields.get("__smartFilter") {
        let (layer, _) = crate::smart_filter_dialog::validate_target(app, filter).map_err(photocraft_engine::EngineError::Other)?;
        return Ok(photocraft_engine::channel_cmds::ToneTarget { layer: Some(layer), channel: None });
    }
    let mut params = crate::filter_dialog::params_of(fields);
    if params.get("target").is_none()
        && app.filter_mask_targeted()
        && let Some(object) = params.as_object_mut()
    {
        object.insert("target".into(), json!("filterMask"));
    }
    photocraft_engine::channel_cmds::resolve_tone_target(&app.session, app.layer_mask_targeted(), &params)
}

/// Retain modal metadata and explicit command addressing when replacing adjustment parameters.
fn keep_context(fields: &mut Map<String, Value>) {
    fields.retain(|key, _| key.starts_with("__") || matches!(key.as_str(), "target" | "layer" | "document"));
}

/// The colour mode the Levels/Curves dialog seeds and parses in: a mask, saved channel or
/// Quick Mask holds gray levels whatever the document's mode, so its curve is a neutral one
/// and the fields carry no Lab or CMYK records (#5335).
fn curve_mode(app: &PhotocraftApp, kind: &str, fields: &Map<String, Value>) -> photocraft_doc::ColorMode {
    let mode = app.session.active().map_or(photocraft_doc::ColorMode::Rgb, |s| s.doc.mode);
    if matches!(kind, "levels" | "curves") && isolated(app, fields).is_some_and(|c| c.gray.is_some()) { photocraft_doc::ColorMode::Grayscale } else { mode }
}

/// Dialog body: the kind's editor over the fields, then the Preview checkbox.
pub fn body(app: &mut PhotocraftApp, ui: &mut egui::Ui, fields: &mut Map<String, Value>) {
    let kind = fields.get("__adjust").and_then(Value::as_str).unwrap_or_default().to_string();
    let context = if matches!(kind.as_str(), "levels" | "curves") {
        match target_context(app, fields) {
            Ok(context) => context,
            Err(error) => {
                ui.label(tl!("The adjustment target is missing or unavailable."));
                ui.label(error.to_string());
                return;
            }
        }
    } else {
        target_context(app, fields)
            .unwrap_or_else(|_| photocraft_engine::channel_cmds::ToneTarget { layer: app.session.active().and_then(|s| s.active_layer), channel: None })
    };
    if fields.get("__unresolvedTarget").and_then(Value::as_bool) == Some(true)
        && let Some(command) = fields.get("__command").and_then(Value::as_str)
        && let Some(resolved) = seeded_fields(app, command, fields.clone())
    {
        *fields = resolved;
    }
    let mode = curve_mode(app, &kind, fields);
    // Each opening starts from the settings it opened at: Photo Filter's Filter / Color choice
    // follows its colour, whatever a cancelled opening left in the editor memory.
    if kind == "photoFilter" && !fields.contains_key(PHOTO_FILTER_FRESH) {
        adjust_editors::forget_photo_filter_mode(ui.ctx(), dialog_mem(&kind));
        fields.insert(PHOTO_FILTER_FRESH.into(), json!(false));
    }
    if matches!(kind.as_str(), "levels" | "curves") {
        picker_controls(ui, fields, mode);
        ui.add_space(4.0);
    }
    let mut values = crate::filter_dialog::params_of(fields);
    let active = context.layer;
    let isolated_curve = context.channel;
    let gray_plane = matches!(kind.as_str(), "levels" | "curves").then(|| isolated_curve.as_ref().and_then(|c| c.gray)).flatten();
    let gray = gray_plane.is_some() || app.session.active().is_some_and(|s| adjust_editors::is_gray(s.doc.mode));
    let hist = match active {
        Some(id) if adjust_editors::needs_histogram(&kind) => {
            // A mask or saved channel's curve edits that plane's own levels: the histogram
            // describes it, not the colour layers beneath (#5335).
            let source = match gray_plane {
                Some(surface) => HistSource::Channel { layer: id, surface },
                None => {
                    if let Some(colors) = isolated_curve.as_ref().and_then(|c| c.components) {
                        HistSource::Colors { layer: id, colors }
                    } else {
                        HistSource::Layer(id)
                    }
                }
            };
            Some(tone::histograms(app, source, adjust_editors::space_of(&values)))
        }
        _ => None,
    };
    let cx = EditorCx {
        mem: dialog_mem(&kind),
        hist,
        gray,
        swatches: adjust_editors::swatches(app),
        gradients: adjust_editors::gradient_presets(app, &kind),
        dialog: true,
        dynamic_color_sliders: app.session.prefs().interface.dynamic_color_sliders,
        isolated_curve,
    };
    let mut e = adjust_editors::editor(ui, &kind, &mut values, &cx);
    if e.import_gradients {
        crate::preset_files_ui::import_gradients_dialog(app);
    }
    if e.auto {
        // A Smart Filter's Auto reads what that filter edits, not the filtered result.
        let filter = fields.get("__smartFilter").and_then(|f| f.get("index")).and_then(Value::as_u64);
        adjust_editors::apply_auto(app, &mut values, active, filter, &mut e);
    }
    if e.changed
        && let Value::Object(v) = values
    {
        // Replace the parameter keys (an editor may remove one), keep the private ones.
        keep_context(fields);
        fields.extend(v);
        if picker(fields.get(CURVE_PICKER)).is_some() {
            let base = crate::filter_dialog::params_of(fields);
            fields.insert(CURVE_PICKER_BASE.into(), base);
        }
    }
    crate::levels_clipping::sync(ui.ctx(), dialog_mem(&kind), fields);
    let mut preview = fields.get("__preview").and_then(Value::as_bool).unwrap_or(true);
    ui.add_space(6.0);
    if crate::widgets::checkbox(ui, &mut preview, tl!("Preview")).changed() {
        fields.insert("__preview".into(), json!(preview));
    }
}

#[cfg(test)]
mod tests;
