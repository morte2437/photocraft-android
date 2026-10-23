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
    app.session.active().is_some_and(|s| picker_supported(s.doc.mode, eyedropper))
}

/// Sample the committed merged document through the regular canvas sampler and update only the
/// open Levels or Curves dialog. Returns true when an armed picker consumed the pointer event,
/// even if the point was outside the image or transparent.
pub fn sample_at(app: &mut PhotocraftApp, x: f64, y: f64) -> bool {
    let Some((id, kind, eyedropper, base)) = app.ui.dialogs.last().and_then(|dialog| {
        let kind = picker_kind(&dialog.fields)?.to_string();
        let eyedropper = picker(dialog.fields.get(CURVE_PICKER))?;
        let base = dialog.fields.get(CURVE_PICKER_BASE).cloned().unwrap_or_else(|| crate::filter_dialog::params_of(&dialog.fields));
        Some((dialog.id, kind, eyedropper, base))
    }) else {
        return false;
    };
    let Some(mode) = app.session.active().map(|s| s.doc.mode) else { return true };
    if !picker_supported(mode, eyedropper) {
        return true;
    }
    let Some(sample) = crate::canvas::composite_color(app, x, y) else { return true };
    let params = match photocraft_engine::adjust_params::adjustment_eyedropper(&kind, &base, eyedropper, sample, mode) {
        Ok(adjustment) => photocraft_engine::adjust_params::to_params(&adjustment),
        Err(error) => {
            app.ui.status = error.to_string();
            return true;
        }
    };
    let Some(dialog) = app.ui.dialog_mut(id) else { return true };
    dialog.fields.insert(CURVE_PICKER_BASE.into(), base);
    if let Value::Object(params) = params {
        dialog.fields.retain(|key, _| key.starts_with("__"));
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
    let Some(mode) = app.session.active().map(|s| s.doc.mode) else { return true };
    if !matches!(mode, photocraft_doc::ColorMode::Rgb | photocraft_doc::ColorMode::Grayscale) {
        app.ui.status = tl!("Unavailable in this color mode").into();
        return true;
    }
    // A mask, saved channel or Quick Mask holds its own gray levels, not the composite's.
    let isolated = isolated(app);
    if isolated.as_ref().is_some_and(|c| c.gray.is_some()) {
        return true;
    }
    let Some(sample) = crate::canvas::composite_color(app, x, y) else { return true };
    let gray = adjust_editors::is_gray(mode);
    let (shown, shown_key) = match &isolated {
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
        dialog.fields.retain(|k, _| k.starts_with("__"));
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
    response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, enabled, selected, tooltip));
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
fn dialog_mem(kind: &str) -> egui::Id {
    egui::Id::new(("adjust-dialog", kind))
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
    let mode = curve_mode(app, &kind);
    // Color Balance: Photoshop keeps the user's Preserve Luminosity across Reset.
    let preserve = if kind == "colorBalance" { fields.get("preserveLuminosity").cloned() } else { None };
    let Ok(defaults) = photocraft_engine::adjust_params::default_for(&kind, mode) else { return };
    let values = photocraft_engine::adjust_params::to_params(&defaults);
    // Replace parameter keys; keep private `__` chrome (preview, command, …).
    fields.retain(|k, _| k.starts_with("__"));
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
}

/// Opens the dialog for `command` with the kind's neutral settings.
pub fn open(app: &mut PhotocraftApp, command: &str) -> Option<u64> {
    let kind = command.strip_prefix(PREFIX).filter(|k| adjust_editors::has_editor(k))?;
    let spec = photocraft_engine::commands::find(command)?;
    let mode = curve_mode(app, kind);
    let defaults = photocraft_engine::adjust_params::default_for(kind, mode).ok()?;
    let mut fields = Map::new();
    fields.insert("__command".into(), json!(command));
    fields.insert("__label".into(), json!(spec.label));
    fields.insert("__adjust".into(), json!(kind));
    fields.insert("__filter".into(), json!(true));
    fields.insert("__preview".into(), json!(true));
    if let Value::Object(p) = photocraft_engine::adjust_params::to_params(&defaults) {
        fields.extend(p);
    }
    Some(app.ui.open_dialog(crate::state::DialogKind::Command, fields))
}

/// The Curves dialog's single-channel target: the Channels panel's, plus the Layers panel's mask
/// selection — both ways of selecting a mask must give the same dropdown (#5335).
fn isolated(app: &PhotocraftApp) -> Option<photocraft_engine::channel_cmds::IsolatedCurveChannel> {
    let mask = app.layer_mask_targeted();
    app.session.active().and_then(|st| photocraft_engine::channel_cmds::isolated_curve_channel_for(st, mask))
}

/// The colour mode the Levels/Curves dialog seeds and parses in: a mask, saved channel or
/// Quick Mask holds gray levels whatever the document's mode, so its curve is a neutral one
/// and the fields carry no Lab or CMYK records (#5335).
fn curve_mode(app: &PhotocraftApp, kind: &str) -> photocraft_doc::ColorMode {
    let mode = app.session.active().map_or(photocraft_doc::ColorMode::Rgb, |s| s.doc.mode);
    if kind == "curves" && isolated(app).is_some_and(|c| c.gray.is_some()) { photocraft_doc::ColorMode::Grayscale } else { mode }
}

/// Dialog body: the kind's editor over the fields, then the Preview checkbox.
pub fn body(app: &mut PhotocraftApp, ui: &mut egui::Ui, fields: &mut Map<String, Value>) {
    let kind = fields.get("__adjust").and_then(Value::as_str).unwrap_or_default().to_string();
    let mode = curve_mode(app, &kind);
    if matches!(kind.as_str(), "levels" | "curves") {
        picker_controls(ui, fields, mode);
        ui.add_space(4.0);
    }
    let mut values = crate::filter_dialog::params_of(fields);
    let active = fields
        .get("__smartFilter")
        .and_then(|f| f.get("layer"))
        .and_then(Value::as_u64)
        .map(photocraft_doc::LayerId)
        .or_else(|| app.session.active().and_then(|s| s.active_layer));
    let isolated_curve = isolated(app);
    // Only Curves gets an isolated single-channel target here; Levels keeps its usual fields.
    let gray_plane = (kind == "curves").then(|| isolated_curve.as_ref().and_then(|c| c.gray)).flatten();
    let gray = gray_plane.is_some() || app.session.active().is_some_and(|s| adjust_editors::is_gray(s.doc.mode));
    let hist = match active {
        Some(id) if adjust_editors::needs_histogram(&kind) => {
            // A mask or saved channel's curve edits that plane's own levels: the histogram
            // describes it, not the colour layers beneath (#5335).
            let source = match gray_plane {
                Some(surface) => HistSource::Channel { layer: id, surface },
                None => HistSource::Layer(id),
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
    let e = adjust_editors::editor(ui, &kind, &mut values, &cx);
    if e.import_gradients {
        crate::preset_files_ui::import_gradients_dialog(app);
    }
    if e.changed
        && let Value::Object(v) = values
    {
        // Replace the parameter keys (an editor may remove one), keep the private ones.
        fields.retain(|k, _| k.starts_with("__"));
        fields.extend(v);
        if picker(fields.get(CURVE_PICKER)).is_some() {
            let base = crate::filter_dialog::params_of(fields);
            fields.insert(CURVE_PICKER_BASE.into(), base);
        }
    }
    let mut preview = fields.get("__preview").and_then(Value::as_bool).unwrap_or(true);
    ui.add_space(6.0);
    if crate::widgets::checkbox(ui, &mut preview, tl!("Preview")).changed() {
        fields.insert("__preview".into(), json!(preview));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::Harness;
    use egui_kittest::kittest::{NodeT, Queryable};

    fn app_with_image() -> PhotocraftApp {
        let mut s = photocraft_engine::Session::new();
        s.execute("file.new", json!({"width": 64, "height": 48})).unwrap();
        s.execute("select.rect", json!({"x": 0, "y": 0, "width": 32, "height": 48})).unwrap();
        s.execute("edit.fill", json!({"color": "#b04020"})).unwrap();
        s.execute("select.deselect", json!({})).unwrap();
        PhotocraftApp::new(s, crate::Services::default())
    }

    fn app_in_mode(mode: &str) -> PhotocraftApp {
        let mut session = photocraft_engine::Session::new();
        session.execute("file.new", json!({"width": 32, "height": 24, "mode": mode})).unwrap();
        PhotocraftApp::new(session, crate::Services::default())
    }

    fn arm(app: &mut PhotocraftApp, id: u64, picker: &str) {
        let base = app.ui.dialog_mut(id).map(|dialog| crate::filter_dialog::params_of(&dialog.fields)).unwrap();
        let dialog = app.ui.dialog_mut(id).unwrap();
        dialog.fields.insert(CURVE_PICKER.into(), json!(picker));
        dialog.fields.insert(CURVE_PICKER_BASE.into(), base);
    }

    fn channel_points(app: &PhotocraftApp, key: &str) -> Vec<[f64; 2]> {
        app.ui
            .dialogs
            .last()
            .and_then(|dialog| dialog.fields.get(key))
            .and_then(Value::as_array)
            .map(|points| {
                points
                    .iter()
                    .filter_map(|point| {
                        let point = point.as_array()?;
                        Some([point.first()?.as_f64()?, point.get(1)?.as_f64()?])
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    #[test]
    fn every_adjustment_with_an_editor_has_a_dialog() {
        for kind in adjust_editors::KINDS {
            assert!(has_dialog(&format!("image.adjustments.{kind}")), "{kind}");
        }
        for id in ["image.adjustments.invert", "image.adjustments.selectiveColor", "image.adjustments.desaturate", "filter.blur.gaussianBlur"] {
            assert!(!has_dialog(id), "{id}");
        }
    }

    /// Opens each dialog, renders it, changes a value, then cancels (document untouched) or
    /// confirms (one history step).
    #[test]
    fn dialogs_preview_cancel_and_commit() {
        for kind in adjust_editors::KINDS {
            let cmd = format!("image.adjustments.{kind}");
            let mut harness =
                Harness::builder().with_size(egui::vec2(1200.0, 900.0)).build_ui_state(|ui, app| crate::dialogs::show(app, ui.ctx()), app_with_image());
            PhotocraftApp::setup_context(&harness.ctx, crate::theme::ThemeKind::ALL[0]);
            let id = open(harness.state_mut(), &cmd).unwrap();
            harness.run_steps(3);
            let before = harness.state().session.active().unwrap().revision;
            let steps = harness.state().session.active().unwrap().history.past_len();
            // Change the settings the way automation does.
            let sample = photocraft_engine::adjust_params::to_params(
                &photocraft_engine::adjust_params::from_params(kind, &sample_params(kind), None, photocraft_doc::ColorMode::Rgb).unwrap(),
            );
            let d = harness.state_mut().ui.dialog_mut(id).unwrap();
            if let Value::Object(p) = sample {
                d.fields.extend(p);
            }
            harness.run_steps(2);
            // The preview runs the real command on the proxy.
            let fields = harness.state().ui.dialogs[0].fields.clone();
            let doc = harness.state().session.active().unwrap().doc.clone();
            let prev = crate::filter_dialog::preview_document(
                &doc,
                harness.state().session.active().unwrap().active_layer,
                &cmd,
                &crate::filter_dialog::params_of(&fields),
                1,
            );
            assert!(prev.is_some(), "{kind}: preview");
            assert_eq!(harness.state().session.active().unwrap().revision, before, "{kind}: previewing leaves the document alone");
            let r = crate::dialogs::confirm(harness.state_mut(), id);
            assert!(r.is_ok(), "{kind}: {r:?}");
            assert_eq!(harness.state().session.active().unwrap().history.past_len(), steps + 1, "{kind}: one history step");
        }
    }

    fn color_balance_dialog() -> (Harness<'static, PhotocraftApp>, u64) {
        // Real frame times: a double-click's two clicks must land within the double-click time.
        let mut h = Harness::builder()
            .with_size(egui::vec2(1200.0, 900.0))
            .with_step_dt(1.0 / 60.0)
            .build_ui_state(|ui, app| crate::dialogs::show(app, ui.ctx()), app_with_image());
        PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
        let id = open(h.state_mut(), "image.adjustments.colorBalance").unwrap();
        let d = h.state_mut().ui.dialog_mut(id).unwrap();
        for (k, v) in
            [("shadows", json!([10, 0, 0])), ("midtones", json!([40, -30, 20])), ("highlights", json!([0, 0, -15])), ("preserveLuminosity", json!(false))]
        {
            d.fields.insert(k.into(), v);
        }
        h.run_steps(3);
        (h, id)
    }

    fn field(h: &Harness<'static, PhotocraftApp>, id: u64, key: &str) -> Value {
        h.state().ui.dialogs.iter().find(|d| d.id == id).map(|d| d.fields.get(key).cloned().unwrap_or(Value::Null)).unwrap_or(Value::Null)
    }

    fn input(h: &mut Harness<'static, PhotocraftApp>, mods: egui::Modifiers, events: &[egui::Event]) {
        h.event(egui::Event::ModifiersChanged(mods));
        h.run_steps(1);
        for e in events {
            h.event(e.clone());
            h.run_steps(1);
        }
        h.event(egui::Event::ModifiersChanged(egui::Modifiers::NONE));
        h.run_steps(3);
    }

    /// Photoshop 25.4's Color Balance dialog (measured with the real application): a double-click
    /// on a slider zeroes it whatever the modifiers; the wheel over a slider steps it by 1, with
    /// Shift by 10 (Ctrl/Alt: 1); holding Alt turns Cancel into Reset, which zeroes every tone,
    /// shows Midtones, keeps Preserve Luminosity and leaves the dialog open.
    #[test]
    fn color_balance_dialog_gestures_match_photoshop() {
        use egui_kittest::kittest::Queryable;
        let (mut h, id) = color_balance_dialog();
        let label = h.query_all_by_label("Cyan  ·  Red").last().map(|n| n.rect()).expect("Cyan · Red row");
        let at = egui::pos2(label.left() + 60.0, label.bottom() + 14.0);
        let press = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::ALT };
        input(&mut h, egui::Modifiers::ALT, &[egui::Event::PointerMoved(at), press(true), press(false), press(true), press(false)]);
        assert_eq!(field(&h, id, "midtones"), json!([0.0, -30.0, 20.0]), "Alt+double-click zeroes the slider");
        assert_eq!(field(&h, id, "shadows"), json!([10, 0, 0]), "other tones untouched");
        let wheel =
            |n: f32, m| egui::Event::MouseWheel { unit: egui::MouseWheelUnit::Line, delta: egui::vec2(0.0, n), phase: egui::TouchPhase::Move, modifiers: m };
        input(&mut h, egui::Modifiers::NONE, &[egui::Event::PointerMoved(at), wheel(3.0, egui::Modifiers::NONE)]);
        assert_eq!(field(&h, id, "midtones")[0], json!(3.0), "3 notches up");
        input(&mut h, egui::Modifiers::SHIFT, &[egui::Event::PointerMoved(at), wheel(2.0, egui::Modifiers::SHIFT)]);
        assert_eq!(field(&h, id, "midtones")[0], json!(23.0), "Shift: 10 per notch");
        input(&mut h, egui::Modifiers::CTRL, &[egui::Event::PointerMoved(at), wheel(-1.0, egui::Modifiers::CTRL)]);
        assert_eq!(field(&h, id, "midtones")[0], json!(22.0), "Ctrl: 1 per notch, down");
        // Alt turns Cancel into Reset.
        assert!(h.query_by_label("Cancel").is_some() && h.query_by_label("Reset").is_none());
        h.state_mut().ui.dialog_mut(id).unwrap().fields.insert("highlights".into(), json!([5, 5, 5]));
        let tone = egui::Id::new(("adjust-dialog", "colorBalance")).with("cb-tone");
        h.ctx.data_mut(|d| d.insert_temp(tone, 2usize));
        h.event(egui::Event::ModifiersChanged(egui::Modifiers::ALT));
        h.run_steps(2);
        let reset = h.get_by_label("Reset").rect();
        assert!(h.query_by_label("Cancel").is_none());
        let click =
            |pressed| egui::Event::PointerButton { pos: reset.center(), button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::ALT };
        input(&mut h, egui::Modifiers::ALT, &[egui::Event::PointerMoved(reset.center()), click(true), click(false)]);
        assert!(h.state().ui.dialogs.iter().any(|d| d.id == id), "Reset keeps the dialog open");
        for k in ["shadows", "midtones", "highlights"] {
            assert_eq!(field(&h, id, k), json!([0.0, 0.0, 0.0]), "{k}");
        }
        assert_eq!(field(&h, id, "preserveLuminosity"), json!(false), "Reset keeps Preserve Luminosity");
        assert_eq!(h.ctx.data(|d| d.get_temp::<usize>(tone)), Some(1), "Reset shows Midtones");
        // Without Alt it is Cancel again, and it closes.
        let cancel = h.get_by_label("Cancel").rect();
        let click =
            |pressed| egui::Event::PointerButton { pos: cancel.center(), button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
        input(&mut h, egui::Modifiers::NONE, &[egui::Event::PointerMoved(cancel.center()), click(true), click(false)]);
        assert!(h.state().ui.dialogs.is_empty(), "Cancel closes");
    }

    /// Every adjustment kind opts into Alt+Reset; Levels restores engine defaults and stays open.
    #[test]
    fn levels_alt_reset_restores_defaults() {
        use egui_kittest::kittest::Queryable;
        let mut h = Harness::builder().with_size(egui::vec2(1200.0, 900.0)).build_ui_state(|ui, app| crate::dialogs::show(app, ui.ctx()), app_with_image());
        PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
        let id = open(h.state_mut(), "image.adjustments.levels").unwrap();
        h.state_mut().ui.dialog_mut(id).unwrap().fields.insert("inBlack".into(), json!(30));
        h.state_mut().ui.dialog_mut(id).unwrap().fields.insert("gamma".into(), json!(1.3));
        h.run_steps(2);
        assert!(h.query_by_label("Cancel").is_some() && h.query_by_label("Reset").is_none());
        h.event(egui::Event::ModifiersChanged(egui::Modifiers::ALT));
        h.run_steps(2);
        let reset = h.get_by_label("Reset").rect();
        assert!(h.query_by_label("Cancel").is_none());
        let click =
            |pressed| egui::Event::PointerButton { pos: reset.center(), button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::ALT };
        input(&mut h, egui::Modifiers::ALT, &[egui::Event::PointerMoved(reset.center()), click(true), click(false)]);
        assert!(h.state().ui.dialogs.iter().any(|d| d.id == id), "Reset keeps the dialog open");
        let defaults =
            photocraft_engine::adjust_params::to_params(&photocraft_engine::adjust_params::default_for("levels", photocraft_doc::ColorMode::Rgb).unwrap());
        assert_eq!(field(&h, id, "inBlack"), defaults["inBlack"]);
        assert_eq!(field(&h, id, "gamma"), defaults["gamma"]);
    }

    fn sample_params(kind: &str) -> Value {
        match kind {
            "brightnessContrast" => json!({"brightness": 40}),
            "levels" => json!({"inBlack": 30, "gamma": 1.3}),
            "curves" => json!({"points": [[0, 0], [100, 150], [255, 255]]}),
            "exposure" => json!({"exposure": 1}),
            "vibrance" => json!({"vibrance": 50}),
            "hueSaturation" => json!({"reds": {"hue": 40}}),
            "colorBalance" => json!({"midtones": [40, 0, -20]}),
            "blackWhite" => json!({"reds": 150, "tint": true}),
            "photoFilter" => json!({"filter": "cooling80", "density": 60}),
            "channelMixer" => json!({"red": [50, 50, 0, 0]}),
            "posterize" => json!({"levels": 3}),
            "threshold" => json!({"level": 90}),
            "gradientMap" => json!({"stops": [[0, "#200040"], [1, "#ffd080"]]}),
            _ => json!({}),
        }
    }

    /// #5113: the dialog should fit its curve graph instead of reserving a blank right column.
    #[test]
    fn curves_dialog_fits_its_graph_and_controls() {
        for language in ["en", "ru"] {
            let _language = crate::i18n::language_scope(crate::i18n::Lang::from_code(language).unwrap());
            for theme in crate::theme::ThemeKind::ALL {
                for size in [egui::vec2(1280.0, 900.0), egui::vec2(900.0, 720.0)] {
                    let mut h = Harness::builder().with_size(size).build_ui_state(|ui, app| crate::dialogs::show(app, ui.ctx()), app_with_image());
                    PhotocraftApp::setup_context(&h.ctx, theme);
                    let id = open(h.state_mut(), "image.adjustments.curves").unwrap();
                    h.run_steps(4);
                    let window = h.ctx.memory(|m| m.area_rect(egui::Id::new(("dialog", id)))).unwrap();
                    let graph = h.ctx.data(|d| d.get_temp::<egui::Rect>(dialog_mem("curves").with("curves-graph"))).unwrap();
                    assert!(window.right() - graph.right() < 40.0, "blank right column: window {window:?}, graph {graph:?}");
                    assert!(graph.width() > 270.0, "keep the graph usable: {graph:?}");
                    assert!(window.contains_rect(graph));
                    for label in ["Channel:", "Input:", "Output:", "Preview", "OK", "Cancel"] {
                        assert!(window.contains_rect(h.get_by_label(crate::i18n::t(label)).rect()), "{label} inside the window");
                    }
                }
            }
        }
    }

    #[test]
    fn cancel_leaves_the_document_untouched() {
        let mut harness =
            Harness::builder().with_size(egui::vec2(1200.0, 900.0)).build_ui_state(|ui, app| crate::dialogs::show(app, ui.ctx()), app_with_image());
        PhotocraftApp::setup_context(&harness.ctx, crate::theme::ThemeKind::ALL[0]);
        let id = open(harness.state_mut(), "image.adjustments.curves").unwrap();
        harness.run_steps(2);
        harness.state_mut().ui.dialog_mut(id).unwrap().fields.insert("points".into(), json!([[0, 255], [255, 0]]));
        harness.run_steps(2);
        let before = harness.state().session.active().unwrap().doc.clone();
        harness.state_mut().ui.close_dialog(id);
        harness.run_steps(2);
        let after = harness.state().session.active().unwrap().doc.clone();
        assert!(std::sync::Arc::ptr_eq(&before, &after));
        assert!(harness.state().ui.dialogs.is_empty());
    }

    #[test]
    fn gradient_map_dialog_handles_preview_without_committing_until_ok() {
        use egui_kittest::kittest::Queryable;
        let mut h = Harness::builder().with_size(egui::vec2(1200.0, 900.0)).build_ui_state(|ui, app| crate::dialogs::show(app, ui.ctx()), app_with_image());
        PhotocraftApp::setup_context(&h.ctx, Default::default());
        let id = open(h.state_mut(), "image.adjustments.gradientMap").unwrap();
        h.run_steps(4);
        let doc = h.state().session.active().unwrap().doc.clone();
        let steps = h.state().session.active().unwrap().history.past_len();
        let strip = h.get_by_label("Gradient stops").rect();
        let start = egui::pos2(strip.left() + 8.0, strip.top() + 36.0);
        let to = start + egui::vec2((strip.width() - 16.0) * 0.3, 0.0);
        h.hover_at(start);
        h.run_steps(1);
        h.drag_at(start);
        h.run_steps(1);
        h.hover_at(to);
        h.run_steps(1);
        h.drop_at(to);
        h.run_steps(3);
        let stops = h.state().ui.dialogs[0].fields["stops"].as_array().unwrap();
        assert!((stops[0][0].as_f64().unwrap() - 0.3).abs() < 0.001);
        assert!(std::sync::Arc::ptr_eq(&doc, &h.state().session.active().unwrap().doc), "dialog edits are previews");
        assert_eq!(h.state().session.active().unwrap().history.past_len(), steps);
        crate::dialogs::confirm(h.state_mut(), id).unwrap();
        h.run_steps(2);
        assert_eq!(h.state().session.active().unwrap().history.past_len(), steps + 1);
        h.state_mut().run("edit.undo", json!({})).unwrap();
        assert_eq!(*h.state().session.active().unwrap().doc, *doc);
    }
    #[test]
    fn curves_pencil_drag_draws_the_map_and_confirm_applies_it() {
        let mut h = Harness::builder().with_size(egui::vec2(1200.0, 900.0)).build_ui_state(|ui, app| crate::dialogs::show(app, ui.ctx()), app_with_image());
        PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
        let steps = h.state().session.active().unwrap().history.past_len();
        let id = open(h.state_mut(), "image.adjustments.curves").unwrap();
        let fields = &mut h.state_mut().ui.dialog_mut(id).unwrap().fields;
        fields.insert("pencil".into(), json!(true));
        fields.insert("points".into(), json!((0..256).collect::<Vec<_>>()));
        h.run_steps(4);
        let graph = h.ctx.data(|d| d.get_temp::<egui::Rect>(egui::Id::new(("adjust-dialog", "curves")).with("curves-graph"))).unwrap();
        let at = |i: f32, o: f32| egui::pos2(graph.left() + i / 255.0 * graph.width(), graph.bottom() - o / 255.0 * graph.height());
        h.hover_at(at(64.0, 200.0));
        h.run_steps(1);
        h.drag_at(at(64.0, 200.0));
        h.run_steps(1);
        for i in [100.0, 150.0, 192.0] {
            h.hover_at(at(i, 200.0));
            h.run_steps(1);
        }
        h.drop_at(at(192.0, 200.0));
        h.run_steps(3);
        let map = h.state().ui.dialogs[0].fields["points"].as_array().unwrap().clone();
        assert_eq!(map.len(), 256);
        assert!((70..185).all(|k| (map[k].as_f64().unwrap() - 200.0).abs() < 3.0), "{map:?}");
        assert_eq!(map[20].as_f64(), Some(20.0), "outside the stroke the map is untouched");
        crate::dialogs::confirm(h.state_mut(), id).unwrap();
        assert_eq!(h.state().session.active().unwrap().history.past_len(), steps + 1);
    }

    #[test]
    fn curves_pointer_drag_previews_until_confirm_and_cancel_preserves_document() {
        for confirm in [false, true] {
            let mut h = Harness::builder().with_size(egui::vec2(1200.0, 900.0)).build_ui_state(|ui, app| crate::dialogs::show(app, ui.ctx()), app_with_image());
            PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
            let original = h.state().session.active().unwrap().doc.clone();
            let steps = h.state().session.active().unwrap().history.past_len();
            let id = open(h.state_mut(), "image.adjustments.curves").unwrap();
            h.state_mut().ui.dialog_mut(id).unwrap().fields.insert("points".into(), json!([[0, 0], [128, 128], [255, 255]]));
            h.run_steps(4);
            let graph = h.ctx.data(|d| d.get_temp::<egui::Rect>(egui::Id::new(("adjust-dialog", "curves")).with("curves-graph"))).unwrap();
            let a = egui::pos2(graph.left() + 128.0 / 255.0 * graph.width(), graph.bottom() - 128.0 / 255.0 * graph.height());
            h.hover_at(a);
            h.run_steps(1);
            h.drag_at(a);
            h.run_steps(1);
            h.hover_at(a + egui::vec2(40.0, -32.0));
            h.run_steps(1);
            h.drop_at(a + egui::vec2(40.0, -32.0));
            h.run_steps(3);
            let points = &h.state().ui.dialogs[0].fields["points"];
            assert_eq!(points.as_array().unwrap().len(), 3);
            assert!(points[1][0].as_f64().unwrap() > 150.0 && points[1][1].as_f64().unwrap() > 150.0, "{points}");
            assert!(std::sync::Arc::ptr_eq(&original, &h.state().session.active().unwrap().doc));
            assert_eq!(h.state().session.active().unwrap().history.past_len(), steps);
            if confirm {
                crate::dialogs::confirm(h.state_mut(), id).unwrap();
                assert_eq!(h.state().session.active().unwrap().history.past_len(), steps + 1);
                h.state_mut().run("edit.undo", json!({})).unwrap();
                assert!(std::sync::Arc::ptr_eq(&original, &h.state().session.active().unwrap().doc));
            } else {
                h.state_mut().ui.close_dialog(id);
                h.run_steps(2);
                assert!(std::sync::Arc::ptr_eq(&original, &h.state().session.active().unwrap().doc));
            }
        }
    }

    #[test]
    fn curves_eyedroppers_sample_repeatedly_without_editing_the_document() {
        let mut app = app_with_image();
        let original = app.session.active().unwrap().doc.clone();
        let revision = app.session.active().unwrap().revision;
        let steps = app.session.active().unwrap().history.past_len();
        let id = open(&mut app, "image.adjustments.curves").unwrap();
        arm(&mut app, id, "black");
        assert!(picker_armed(&app));
        assert!(sample_at(&mut app, 8.0, 8.0));
        let first = channel_points(&app, "red");
        assert!((first[0][0] - 176.0).abs() < 0.6 && first[0][1] == 0.0, "{first:?}");
        assert!(sample_at(&mut app, 8.0, 8.0));
        assert_eq!(channel_points(&app, "red"), first, "the same repeated sample is deterministic");
        assert!(sample_at(&mut app, 48.0, 8.0));
        assert_ne!(channel_points(&app, "red"), first);
        assert!(sample_at(&mut app, 8.0, 8.0));
        assert_eq!(channel_points(&app, "red"), first, "repeated samples resolve from the armed baseline");
        let st = app.session.active().unwrap();
        assert!(std::sync::Arc::ptr_eq(&st.doc, &original));
        assert_eq!(st.revision, revision);
        assert_eq!(st.history.past_len(), steps);
    }

    /// #3116: the Levels dialog's Set White Point moves each channel's input white to the sample
    /// (#b04020) and leaves the document alone until OK.
    #[test]
    fn levels_white_point_eyedropper_sets_the_channel_input_whites() {
        let mut app = app_with_image();
        let original = app.session.active().unwrap().doc.clone();
        let id = open(&mut app, "image.adjustments.levels").unwrap();
        arm(&mut app, id, "white");
        assert!(picker_armed(&app));
        assert!(sample_at(&mut app, 8.0, 8.0));
        let in_white = |key: &str| app.ui.dialogs.last().and_then(|d| d.fields.get(key)).and_then(|c| c.get("inWhite")).and_then(Value::as_f64);
        assert_eq!([in_white("red"), in_white("green"), in_white("blue")], [Some(176.0), Some(64.0), Some(32.0)]);
        assert!(std::sync::Arc::ptr_eq(&app.session.active().unwrap().doc, &original));
    }

    #[test]
    fn curves_picker_controls_arm_and_disable_by_document_mode() {
        let harness = |app| {
            let mut h = Harness::builder().with_size(egui::vec2(1200.0, 900.0)).build_ui_state(|ui, app| crate::dialogs::show(app, ui.ctx()), app);
            PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
            open(h.state_mut(), "image.adjustments.curves").unwrap();
            h.run_steps(3);
            h
        };
        let mut rgb = harness(app_in_mode("rgb"));
        for label in ["Set Black Point", "Set Neutral Gray Point", "Set White Point"] {
            assert!(!rgb.get_by_label(label).accesskit_node().is_disabled(), "{label}");
        }
        rgb.get_by_label("Set Black Point").click();
        rgb.run_steps(2);
        assert!(picker_armed(rgb.state()));
        assert_eq!(rgb.state().ui.dialogs.last().and_then(|d| d.fields.get(CURVE_PICKER)).and_then(Value::as_str), Some("black"));

        let gray = harness(app_in_mode("gray"));
        assert!(!gray.get_by_label("Set Black Point").accesskit_node().is_disabled());
        assert!(gray.get_by_label("Set Neutral Gray Point").accesskit_node().is_disabled());
        assert!(!gray.get_by_label("Set White Point").accesskit_node().is_disabled());

        for mode in ["cmyk", "lab"] {
            let h = harness(app_in_mode(mode));
            for label in ["Set Black Point", "Set Neutral Gray Point", "Set White Point"] {
                assert!(h.get_by_label(label).accesskit_node().is_disabled(), "{mode}: {label}");
            }
        }
    }

    #[test]
    fn curves_picker_rejects_outside_and_transparent_samples() {
        let mut s = photocraft_engine::Session::new();
        s.execute("file.new", json!({"width": 32, "height": 24, "background": "transparent"})).unwrap();
        let mut app = PhotocraftApp::new(s, crate::Services::default());
        let id = open(&mut app, "image.adjustments.curves").unwrap();
        arm(&mut app, id, "white");
        let before = crate::filter_dialog::params_of(&app.ui.dialog_mut(id).unwrap().fields);
        assert!(sample_at(&mut app, -1.0, 4.0));
        assert_eq!(crate::filter_dialog::params_of(&app.ui.dialog_mut(id).unwrap().fields), before);
        assert!(sample_at(&mut app, 4.0, 4.0));
        assert_eq!(crate::filter_dialog::params_of(&app.ui.dialog_mut(id).unwrap().fields), before);
        assert!(picker_armed(&app), "an invalid sample does not disarm the picker");
    }

    #[test]
    fn curves_picker_cancel_and_confirm_keep_one_transaction() {
        for confirm in [false, true] {
            let mut app = app_with_image();
            let original = app.session.active().unwrap().doc.clone();
            let steps = app.session.active().unwrap().history.past_len();
            let id = open(&mut app, "image.adjustments.curves").unwrap();
            arm(&mut app, id, "white");
            assert!(sample_at(&mut app, 8.0, 8.0));
            assert!(std::sync::Arc::ptr_eq(&app.session.active().unwrap().doc, &original));
            assert_eq!(app.session.active().unwrap().history.past_len(), steps);
            if confirm {
                crate::dialogs::confirm(&mut app, id).unwrap();
                let committed = app.session.active().unwrap().doc.clone();
                assert!(!std::sync::Arc::ptr_eq(&committed, &original));
                assert_eq!(app.session.active().unwrap().history.past_len(), steps + 1);
                app.run("edit.undo", json!({})).unwrap();
                assert!(std::sync::Arc::ptr_eq(&app.session.active().unwrap().doc, &original));
                app.run("edit.redo", json!({})).unwrap();
                assert!(std::sync::Arc::ptr_eq(&app.session.active().unwrap().doc, &committed));
            } else {
                app.ui.close_dialog(id);
                assert!(std::sync::Arc::ptr_eq(&app.session.active().unwrap().doc, &original));
                assert_eq!(app.session.active().unwrap().history.past_len(), steps);
            }
        }
    }

    #[test]
    fn curves_picker_modes_and_depths_are_explicit() {
        use photocraft_engine::adjust_params::CurvesEyedropper;
        assert!(picker_supported(photocraft_doc::ColorMode::Rgb, CurvesEyedropper::NeutralGray));
        assert!(picker_supported(photocraft_doc::ColorMode::Grayscale, CurvesEyedropper::Black));
        assert!(picker_supported(photocraft_doc::ColorMode::Grayscale, CurvesEyedropper::White));
        assert!(!picker_supported(photocraft_doc::ColorMode::Grayscale, CurvesEyedropper::NeutralGray));
        assert!(!picker_supported(photocraft_doc::ColorMode::Cmyk, CurvesEyedropper::Black));
        assert!(!picker_supported(photocraft_doc::ColorMode::Lab, CurvesEyedropper::White));

        for depth in [8, 16, 32] {
            let mut s = photocraft_engine::Session::new();
            s.execute("file.new", json!({"width": 16, "height": 16, "depth": depth})).unwrap();
            s.execute("edit.fill", json!({"color": [0.3142, 0.4821, 0.7137]})).unwrap();
            let mut app = PhotocraftApp::new(s, crate::Services::default());
            let sampled = crate::canvas::composite_color(&mut app, 4.0, 4.0).unwrap();
            let id = open(&mut app, "image.adjustments.curves").unwrap();
            arm(&mut app, id, "gray");
            assert!(sample_at(&mut app, 4.0, 4.0), "RGB @{depth}");
            let red = channel_points(&app, "red");
            let target_level = (f64::from(sampled[0]) * 0.2126 + f64::from(sampled[1]) * 0.7152 + f64::from(sampled[2]) * 0.0722) * 255.0;
            let sample_level = f64::from(sampled[0]) * 255.0;
            let serialized_sample_level = (sample_level * 100.0).round() / 100.0;
            let neutral_anchor = red.iter().find(|point| (point[0] - serialized_sample_level).abs() < 1e-5).unwrap();
            let serialized_target_level = (target_level * 100.0).round() / 100.0;
            assert!((neutral_anchor[1] - serialized_target_level).abs() < 1e-5, "depth={depth} anchor={neutral_anchor:?} sampled={sampled:?}");
            assert!((neutral_anchor[0] - sample_level).abs() <= 0.0051, "depth={depth} anchor={neutral_anchor:?} sampled={}", sampled[0]);
            if depth == 16 || depth == 32 {
                assert!(
                    (sampled[0] * 255.0 - (sampled[0] * 255.0).round()).abs() > 0.01,
                    "depth={depth} sample was quantized to an 8-bit level: {}",
                    sampled[0]
                );
            }
        }
    }

    fn pointer_down(app: &mut PhotocraftApp, ctx: &egui::Context, x: f64, y: f64, modifiers: Value) {
        let (request, _receiver) =
            crate::control::ControlRequest::new("ui.pointer", json!({"events": [{"kind": "down", "x": x, "y": y}], "modifiers": modifiers}));
        let _ = crate::control::handle(app, ctx, &request);
    }

    /// The Curves curve `pts` (0-255 levels) as a 256-entry 0-255 map.
    fn curve_levels(pts: &[[f64; 2]]) -> Vec<f32> {
        let cp: Vec<_> = pts.iter().map(|q| photocraft_doc::adjust::CurvePoint { input: q[0] as f32 / 255.0, output: q[1] as f32 / 255.0 }).collect();
        let lut = photocraft_compose::adjust::curve_lut(&cp);
        (0..256).map(|k| lut[(k as f32 / 255.0 * (lut.len() - 1) as f32).round() as usize] * 255.0).collect()
    }

    /// #7325: ⌘/Ctrl-click on the image puts a point on the shown curve at the pixel's tone
    /// (#b04020: Rec. 709 luma 85.5 → level 86) and at the curve's own output there, so the
    /// image does not change; the new point is the selected one, and the document waits for OK.
    #[test]
    fn command_click_on_the_image_adds_a_point_on_the_composite_curve() {
        let mut app = app_with_image();
        let original = app.session.active().unwrap().doc.clone();
        let steps = app.session.active().unwrap().history.past_len();
        let id = open(&mut app, "image.adjustments.curves").unwrap();
        let base = [[0.0, 0.0], [128.0, 200.0], [255.0, 255.0]];
        app.ui.dialog_mut(id).unwrap().fields.insert("points".into(), json!(base));
        let ctx = egui::Context::default();
        let sample = crate::canvas::composite_color(&mut app, 8.0, 8.0).unwrap();
        let level = ((sample[0] * 0.2126 + sample[1] * 0.7152 + sample[2] * 0.0722) * 255.0).round();
        assert_eq!(level, 86.0);
        let output = f64::from(curve_levels(&base)[86].round());
        assert_ne!(output, 86.0, "the base curve is not the identity at the sampled level");

        pointer_down(&mut app, &ctx, 8.0, 8.0, json!({"command": true}));
        let points = channel_points(&app, "points");
        assert_eq!(points, vec![base[0], [86.0, output], base[1], base[2]]);
        let (before, after) = (curve_levels(&base), curve_levels(&points));
        assert!(before.iter().zip(&after).all(|(a, b)| (a - b).abs() <= 0.5), "the curve keeps its shape:\n{before:?}\n{after:?}");
        let st: crate::state::CurvesEditorState = ctx.data(|d| d.get_temp(dialog_mem("curves").with("curves"))).unwrap();
        assert_eq!((st.channel, st.gesture.selected), (0, Some(1)), "the new point is selected");
        for key in ["red", "green", "blue"] {
            assert_eq!(channel_points(&app, key), vec![[0.0, 0.0], [255.0, 255.0]], "{key} untouched");
        }
        assert!(std::sync::Arc::ptr_eq(&app.session.active().unwrap().doc, &original));
        assert_eq!(app.session.active().unwrap().history.past_len(), steps);
        crate::dialogs::confirm(&mut app, id).unwrap();
        assert_eq!(app.session.active().unwrap().history.past_len(), steps + 1);
    }

    /// #7325: Shift+⌘-click puts a point on each colour channel at that channel's own value
    /// (176, 64, 32), and leaves the composite curve alone.
    #[test]
    fn shift_command_click_adds_a_point_on_each_colour_channel() {
        let mut app = app_with_image();
        open(&mut app, "image.adjustments.curves").unwrap();
        let ctx = egui::Context::default();
        let composite = channel_points(&app, "points");
        pointer_down(&mut app, &ctx, 8.0, 8.0, json!({"command": true, "shift": true}));
        for (key, level) in [("red", 176.0), ("green", 64.0), ("blue", 32.0)] {
            assert_eq!(channel_points(&app, key), vec![[0.0, 0.0], [level, level], [255.0, 255.0]], "{key}");
        }
        assert_eq!(channel_points(&app, "points"), composite);
    }

    /// #7325: with Red shown in the Channel popup the point goes on the red curve, at red's value.
    #[test]
    fn command_click_adds_the_point_on_the_channel_shown_in_the_dialog() {
        let mut app = app_with_image();
        open(&mut app, "image.adjustments.curves").unwrap();
        let ctx = egui::Context::default();
        ctx.data_mut(|d| d.insert_temp(dialog_mem("curves").with("curves-ch"), 1usize));
        let composite = channel_points(&app, "points");
        pointer_down(&mut app, &ctx, 8.0, 8.0, json!({"command": true}));
        assert_eq!(channel_points(&app, "red"), vec![[0.0, 0.0], [176.0, 176.0], [255.0, 255.0]]);
        for key in ["green", "blue"] {
            assert_eq!(channel_points(&app, key), vec![[0.0, 0.0], [255.0, 255.0]], "{key} untouched");
        }
        assert_eq!(channel_points(&app, "points"), composite);
        let st: crate::state::CurvesEditorState = ctx.data(|d| d.get_temp(dialog_mem("curves").with("curves"))).unwrap();
        assert_eq!((st.channel, st.gesture.selected), (1, Some(1)));
    }

    /// #7325: a plain click, a full curve and an armed eyedropper add nothing.
    #[test]
    fn command_click_adds_nothing_without_the_modifier_on_a_full_curve_or_with_an_eyedropper() {
        let mut app = app_with_image();
        let id = open(&mut app, "image.adjustments.curves").unwrap();
        let ctx = egui::Context::default();
        let before = crate::filter_dialog::params_of(&app.ui.dialog_mut(id).unwrap().fields);
        pointer_down(&mut app, &ctx, 8.0, 8.0, json!({}));
        assert_eq!(crate::filter_dialog::params_of(&app.ui.dialog_mut(id).unwrap().fields), before, "no modifier, no point");
        // Space and the middle button pan, as with the mouse.
        for extra in [json!({"modifiers": {"command": true, "space": true}}), json!({"command": true, "button": "middle"})] {
            let mut params = json!({"events": [{"kind": "down", "x": 8, "y": 8}]});
            params.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
            let (request, _receiver) = crate::control::ControlRequest::new("ui.pointer", params);
            let _ = crate::control::handle(&mut app, &ctx, &request);
            assert_eq!(crate::filter_dialog::params_of(&app.ui.dialog_mut(id).unwrap().fields), before, "{extra}");
        }

        // Sixteen points (the most a curve holds), none within reach of level 86.
        let full: Vec<[f64; 2]> =
            [0, 10, 20, 30, 40, 50, 60, 70, 80, 100, 120, 140, 160, 180, 200, 255].iter().map(|&i| [f64::from(i), f64::from(i)]).collect();
        app.ui.dialog_mut(id).unwrap().fields.insert("points".into(), json!(full));
        pointer_down(&mut app, &ctx, 8.0, 8.0, json!({"command": true}));
        assert_eq!(channel_points(&app, "points"), full, "a full curve takes no more points and moves none");

        let mut app = app_with_image();
        let id = open(&mut app, "image.adjustments.curves").unwrap();
        arm(&mut app, id, "black");
        pointer_down(&mut app, &ctx, 8.0, 8.0, json!({"command": true}));
        assert_eq!(channel_points(&app, "points").len(), 2, "the armed eyedropper samples instead");
        assert_eq!(channel_points(&app, "red")[0], [176.0, 0.0]);
    }

    /// #7325: Lab and CMYK curves and a targeted mask are left to a later change: nothing is added.
    #[test]
    fn command_click_adds_nothing_in_lab_or_on_a_targeted_mask() {
        let ctx = egui::Context::default();
        let mut lab = app_in_mode("lab");
        lab.run("edit.fill", json!({"color": "#b04020"})).unwrap();
        let id = open(&mut lab, "image.adjustments.curves").unwrap();
        let before = crate::filter_dialog::params_of(&lab.ui.dialog_mut(id).unwrap().fields);
        assert!(add_point_at(&mut lab, &ctx, 4.0, 4.0, false));
        assert_eq!(crate::filter_dialog::params_of(&lab.ui.dialog_mut(id).unwrap().fields), before);

        let mut masked = app_with_image();
        masked.run("layer.layerMask.revealAll", json!({})).unwrap();
        masked.run("channel.target", json!({"channel": "mask"})).unwrap();
        let id = open(&mut masked, "image.adjustments.curves").unwrap();
        let before = crate::filter_dialog::params_of(&masked.ui.dialog_mut(id).unwrap().fields);
        assert!(add_point_at(&mut masked, &ctx, 8.0, 8.0, false));
        assert_eq!(crate::filter_dialog::params_of(&masked.ui.dialog_mut(id).unwrap().fields), before);
    }

    #[test]
    fn ui_pointer_drives_an_armed_curves_picker() {
        let mut app = app_with_image();
        let id = open(&mut app, "image.adjustments.curves").unwrap();
        arm(&mut app, id, "black");
        let context = egui::Context::default();
        let before = channel_points(&app, "red");
        let (request, _receiver) = crate::control::ControlRequest::new("ui.pointer", json!({"events": [{"kind": "move", "x": 48, "y": 8}]}));
        let _ = crate::control::handle(&mut app, &context, &request);
        assert_eq!(channel_points(&app, "red"), before, "pointer move does not sample");

        let (request, _receiver) = crate::control::ControlRequest::new("ui.pointer", json!({"events": [{"kind": "down", "x": 8, "y": 8}]}));
        let _ = crate::control::handle(&mut app, &context, &request);
        let red = channel_points(&app, "red");
        assert!((red[0][0] - 176.0).abs() < 0.6 && red[0][1] == 0.0, "{red:?}");
        assert_ne!(red, before, "pointer down samples");

        let (request, _receiver) = crate::control::ControlRequest::new("ui.pointer", json!({"events": [{"kind": "up", "x": 48, "y": 8}]}));
        let _ = crate::control::handle(&mut app, &context, &request);
        assert_eq!(channel_points(&app, "red"), red, "pointer up does not sample again");
        assert!(picker_armed(&app));
    }

    /// #5335: a targeted mask, saved channel or colour component is the only Channel popup entry.
    #[test]
    fn curves_channel_popup_lists_only_the_targeted_channel() {
        let mut app = app_with_image();
        app.run("layer.layerMask.revealAll", json!({})).unwrap();
        app.run("channel.target", json!({"channel": "mask"})).unwrap();
        let mut h = curves_harness(app);
        let names = popup_channels(&mut h);
        assert!(names.iter().any(|n| n.contains("Mask")), "{names:?}");
        assert!(!names.iter().any(|n| n == "RGB" || n == "Red" || n == "Green" || n == "Blue"), "{names:?}");
    }

    /// #5335: selecting the mask in the Layers panel gives the same Curves dropdown as selecting
    /// it in the Channels panel.
    #[test]
    fn curves_channel_popup_lists_only_the_mask_selected_in_the_layers_panel() {
        let mut app = app_with_image();
        app.run("layer.layerMask.revealAll", json!({})).unwrap();
        app.ui.mask_target = true; // clicking the mask thumbnail in the Layers panel
        assert!(app.layer_mask_targeted(), "the Layers panel mask thumbnail is targeted");
        let mut h = curves_harness(app);
        let names = popup_channels(&mut h);
        assert_eq!(names.len(), 1, "only the mask: {names:?}");
        assert!(names[0].contains("Mask"), "{names:?}");
    }

    /// #5335: in a Lab document, Curves over a gray target keeps neutral fields: no Lab records
    /// to overwrite the mask's own curve or pick the wrong tone space.
    #[test]
    fn curves_on_a_gray_target_seeds_neutral_fields_in_a_lab_document() {
        for via_layers_panel in [false, true] {
            let mut app = app_in_mode("lab");
            app.run("layer.layerMask.revealAll", json!({})).unwrap();
            if via_layers_panel {
                app.ui.mask_target = true;
            } else {
                app.run("channel.target", json!({"channel": "mask"})).unwrap();
            }
            let id = open(&mut app, "image.adjustments.curves").expect("opens");
            let values = crate::filter_dialog::params_of(&app.ui.dialog_mut(id).unwrap().fields.clone());
            assert!(values.get("points").is_some(), "{values:?}");
            for key in ["lightness", "a", "b", "cyan", "magenta", "yellow", "black"] {
                assert!(values.get(key).is_none(), "{key} in a gray target's curves: {values:?}");
            }
        }
    }

    /// #5335: in Italian the built-in labels translate, but a channel the user named "Green"
    /// stays exactly "Green".
    #[test]
    fn curves_channel_popup_keeps_custom_names_verbatim_in_italian() {
        let it = crate::i18n::Lang::from_code("it").expect("it registered");
        let green = crate::i18n::tr(it, "Green");
        let _lang = crate::i18n::language_scope(it);
        let mut app = app_with_image();
        app.run("channel.new", json!({"name": "Green", "fill": "white"})).unwrap();
        app.run("channel.target", json!({"channel": 0})).unwrap();
        let mut h = curves_harness(app);
        let names = popup_channels(&mut h);
        assert!(names.iter().any(|n| n == "Green"), "custom names stay as entered: {names:?}");
        assert!(green == "Green" || !names.iter().any(|n| n == green), "not translated: {names:?}");
        // A targeted colour component keeps its translated built-in label.
        let mut colour = app_with_image();
        colour.run("channel.target", json!({"channel": {"color": 1}})).unwrap();
        let mut h = curves_harness(colour);
        let names = popup_channels(&mut h);
        assert!(names.iter().any(|n| n == green), "built-in labels follow the language: {names:?}");
    }

    /// Curves open in a harness over `app`.
    fn curves_harness(app: PhotocraftApp) -> Harness<'static, PhotocraftApp> {
        let mut h = Harness::builder().with_size(egui::vec2(1200.0, 900.0)).build_ui_state(|ui, app| crate::dialogs::show(app, ui.ctx()), app);
        PhotocraftApp::setup_context(&h.ctx, Default::default());
        open(h.state_mut(), "image.adjustments.curves").unwrap();
        h.run_steps(4);
        h
    }

    /// The Channel popup's selected value (its option labels are what it shows).
    fn popup_channels(h: &mut Harness<'static, PhotocraftApp>) -> Vec<String> {
        h.query_all_by_role(egui::accesskit::Role::ComboBox).filter_map(|n| n.value().map(|s| s.to_string())).collect()
    }
}
