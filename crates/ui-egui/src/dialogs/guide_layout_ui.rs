//! Photoshop-style guide layout dialog. Its preview is a pure guide overlay, never an edit.

use egui::{Color32, Stroke};
use photocraft_doc::{Document, Guides};
use photocraft_engine::prefs::Unit;
use serde_json::{Map, Value, json};

use crate::{PhotocraftApp, theme::Tokens, widgets};

const COMMAND: &str = "view.newGuideLayout";
const MARK: &str = "__guideLayout";

/// A `"#rrggbb"` colour as drawn.
fn rgb(hex: &str) -> Option<Color32> {
    photocraft_engine::prefs::parse_hex(hex).map(|[r, g, b]| Color32::from_rgb(r, g, b))
}

/// The Color list: New Guide's colours (`GUIDE_COLORS`) by their `"#rrggbb"`, then `current` as
/// Custom when it's none of them.
fn color_options(current: &str) -> Vec<(String, &'static str)> {
    let mut colors: Vec<(String, &'static str)> = photocraft_engine::edit_cmds::GUIDE_COLORS
        .iter()
        .map(|(name, [r, g, b])| (format!("#{r:02x}{g:02x}{b:02x}"), crate::new_guide_ui::color_label(name)))
        .collect();
    if !colors.iter().any(|(v, _)| v == current) {
        colors.push((current.to_string(), "Custom"));
    }
    colors
}

pub fn owns(f: &Map<String, Value>) -> bool {
    f.contains_key(MARK)
}

fn ruler_unit(app: &PhotocraftApp) -> Unit {
    app.session.prefs().units_and_rulers.rulers
}

fn with_unit(value: &str, unit: Unit) -> String {
    format!("{value} {}", unit.suffix())
}

/// Decimals the reference application's guide layout fields show in every unit, trailing zeros
/// dropped: 20 px at 72 ppi opens as "0.706 cm", a typed "13.5 px" stays "13.5 px".
const FIELD_DECIMALS: usize = 3;

/// Document pixels in the active document's ruler unit: "0.706 cm", "0.278 in", "20 px".
pub(crate) fn in_ruler_unit(app: &PhotocraftApp, px: f64, along_x: bool) -> String {
    let units = &app.session.prefs().units_and_rulers;
    let (dpi, extent) = app.session.active().map_or((72.0, 1.0), |st| {
        let d = &st.doc;
        (f64::from(d.resolution_dpi.max(1.0)), f64::from(if along_x { d.size.width } else { d.size.height }))
    });
    let v = units.rulers.from_px(px, dpi, extent, units.point_size.per_inch());
    with_unit(&photocraft_engine::prefs::fmt_decimals(v, FIELD_DECIMALS), units.rulers)
}

/// The unit a field's text names, or `fallback` when it names none.
fn text_unit(text: &str, fallback: Unit) -> Unit {
    Unit::parse_length(text).and_then(|(_, unit)| unit).unwrap_or(fallback)
}

/// The text a field settles on when it loses focus. A bare number takes `unit`, the unit the
/// field showed when it got focus (after "2 mm", a typed "5" becomes "5 mm").
fn normalize_field_text(text: &str, app: &PhotocraftApp, along_x: bool, optional: bool, unit: Unit) -> Option<String> {
    let t = text.trim();
    if t.is_empty() {
        if optional {
            return Some(String::new());
        } else {
            return Some(in_ruler_unit(app, 0.0, along_x));
        }
    }
    let (v, unit) = match Unit::parse_length(t) {
        Some((v, typed)) => (v, typed.unwrap_or(unit)),
        None => (widgets::parse_num(t)?, unit),
    };
    Some(with_unit(&photocraft_engine::prefs::fmt_decimals(v, FIELD_DECIMALS), unit))
}

fn parse_length_str(text: &str, app: &PhotocraftApp, along_x: bool) -> Option<f64> {
    let t = text.trim();
    if t.is_empty() {
        return None;
    }
    let units = &app.session.prefs().units_and_rulers;
    let (dpi, extent) = app.session.active().map_or((72.0, 1.0), |st| {
        let d = &st.doc;
        (f64::from(d.resolution_dpi.max(1.0)), f64::from(if along_x { d.size.width } else { d.size.height }))
    });
    let ppi = units.point_size.per_inch();
    let ruler = units.rulers;

    if let Some((n, unit)) = Unit::parse_length(t) {
        Some(unit.unwrap_or(ruler).to_px(n, dpi, extent, ppi))
    } else {
        widgets::parse_num(t).map(|n| ruler.to_px(n, dpi, extent, ppi))
    }
}

/// The percent a field's text gives ("10%", or a bare number while the rulers are in percent).
/// Percent stays percent all the way to the engine, which takes it of each target, so a selected
/// artboard smaller than the document gets its own share, not the document's.
fn percent_of(text: &str, ruler: Unit) -> Option<f64> {
    match Unit::parse_length(text) {
        Some((n, Some(Unit::Percent))) => Some(n),
        Some((n, None)) if ruler == Unit::Percent => Some(n),
        Some(_) => None,
        None if ruler == Unit::Percent => widgets::parse_num(text),
        None => None,
    }
}

fn format_stored_length(app: &PhotocraftApp, val: &Value, along_x: bool, optional: bool) -> Value {
    match val {
        Value::Number(n) => {
            let px = n.as_f64().unwrap_or(0.0);
            if optional && px <= 0.0 { Value::Null } else { json!(in_ruler_unit(app, px, along_x)) }
        }
        Value::String(s) => {
            let t = s.trim();
            if t.is_empty() {
                if optional { Value::Null } else { json!(in_ruler_unit(app, 0.0, along_x)) }
            } else if let Some(pct) = percent_of(t, ruler_unit(app)) {
                if optional && pct <= 0.0 { Value::Null } else { json!(with_unit(&photocraft_engine::prefs::fmt_decimals(pct, FIELD_DECIMALS), Unit::Percent)) }
            } else if let Some(px) = parse_length_str(t, app, along_x) {
                if optional && px <= 0.0 { Value::Null } else { json!(in_ruler_unit(app, px, along_x)) }
            } else {
                json!(s)
            }
        }
        _ => {
            if optional {
                Value::Null
            } else {
                json!(in_ruler_unit(app, 0.0, along_x))
            }
        }
    }
}

/// Alt+Cancel (Reset): factory New Guide Layout values, not the last-used remembered layout.
pub fn reset(app: &PhotocraftApp, f: &mut Map<String, Value>) {
    if !owns(f) {
        return;
    }
    let Value::Object(defaults) = crate::view_cmds::default_guide_layout() else { return };
    // Drop layout params and interactive toggles; keep dialog chrome (`__command`, …).
    f.retain(|k, _| {
        k.starts_with("__")
            && !matches!(
                k.as_str(),
                "__preset" | "__columnsEnabled" | "__rowsEnabled" | "__marginEnabled" | "__previewGuides" | "__previewParams" | "__previewSource"
            )
    });
    f.extend(defaults);
    f.insert("color".into(), json!(app.session.prefs().guides_grid_and_slices.guide_color));
    f.insert("target".into(), json!("document"));
    f.insert("__preset".into(), json!("custom"));
    f.insert("width".into(), Value::Null);
    f.insert("height".into(), Value::Null);
    f.insert("gutter".into(), json!(in_ruler_unit(app, 20.0, true)));
    f.insert("rowGutter".into(), json!(in_ruler_unit(app, 0.0, false)));
    f.insert(
        "margin".into(),
        json!([in_ruler_unit(app, 0.0, false), in_ruler_unit(app, 0.0, true), in_ruler_unit(app, 0.0, false), in_ruler_unit(app, 0.0, true)]),
    );
    f.insert("__preview".into(), json!(true));
}

fn on(f: &Map<String, Value>, key: &str, fallback: bool) -> bool {
    f.get(key).and_then(Value::as_bool).unwrap_or(fallback)
}
fn positive(f: &Map<String, Value>, key: &str) -> bool {
    f.get(key).and_then(Value::as_f64).is_some_and(|n| n > 0.0)
}

pub fn open(app: &mut PhotocraftApp) -> u64 {
    let mut f = app.ui.view.guide_layout.as_object().cloned().unwrap_or_default();
    f.insert(MARK.into(), json!(true));
    f.insert("__command".into(), json!(COMMAND));
    f.insert("__label".into(), json!("New Guide Layout…"));
    f.insert("__document".into(), app.session.active().map(|s| json!(s.doc.id)).unwrap_or(Value::Null));
    f.insert("__preview".into(), json!(true));
    f.insert("color".into(), json!(app.session.prefs().guides_grid_and_slices.guide_color));
    f.entry("target").or_insert_with(|| json!("document"));
    if f.get("target").and_then(Value::as_str) == Some("selectedArtboards") && photocraft_engine::guide_layout::selected_artboards(&app.session).is_empty() {
        f.insert("target".into(), json!("document"));
    }
    f.entry("__preset").or_insert_with(|| json!("custom"));
    f.entry("width").or_insert(Value::Null);
    f.entry("height").or_insert(Value::Null);

    // Format fields in active ruler unit when opening the dialog
    if let Some(gutter) = f.get("gutter").cloned() {
        f.insert("gutter".into(), format_stored_length(app, &gutter, true, false));
    } else {
        f.insert("gutter".into(), json!(in_ruler_unit(app, 20.0, true)));
    }
    if let Some(row_gutter) = f.get("rowGutter").cloned() {
        f.insert("rowGutter".into(), format_stored_length(app, &row_gutter, false, false));
    } else {
        f.insert("rowGutter".into(), json!(in_ruler_unit(app, 0.0, false)));
    }
    if let Some(width) = f.get("width").cloned() {
        f.insert("width".into(), format_stored_length(app, &width, true, true));
    }
    if let Some(height) = f.get("height").cloned() {
        f.insert("height".into(), format_stored_length(app, &height, false, true));
    }

    let margin = f.get("margin").cloned().unwrap_or(json!(0));
    let mut margins = if let Value::Array(a) = margin { if a.len() == 4 { a } else { vec![json!(0); 4] } } else { vec![margin.clone(); 4] };
    // Top, Left, Bottom, Right: Left and Right measure along x.
    for (m, along_x) in margins.iter_mut().zip([false, true, false, true]) {
        *m = format_stored_length(app, m, along_x, false);
    }
    f.insert("margin".into(), Value::Array(margins));

    for key in ["__previewGuides", "__previewParams", "__previewSource"] {
        f.remove(key);
    }
    app.ui.open_dialog(crate::state::DialogKind::Command, f)
}

fn parse_length_value(val: Option<&Value>, app: Option<&PhotocraftApp>, along_x: bool, optional: bool) -> Value {
    match val {
        Some(Value::Number(n)) => {
            if let Some(num) = n.as_f64() {
                if optional && num <= 0.0 { Value::Null } else { json!(num) }
            } else {
                Value::Null
            }
        }
        Some(Value::String(s)) => {
            let t = s.trim();
            if t.is_empty() {
                if optional { Value::Null } else { json!(0.0) }
            } else if let Some(pct) = percent_of(t, app.map_or(Unit::Pixels, ruler_unit)) {
                if optional && pct <= 0.0 { Value::Null } else { json!(format!("{pct}%")) }
            } else {
                let (dpi, extent, ppi, ruler) = match app {
                    Some(app) => {
                        let units = &app.session.prefs().units_and_rulers;
                        let (dpi, extent) = app.session.active().map_or((72.0, 1.0), |st| {
                            let d = &st.doc;
                            (f64::from(d.resolution_dpi.max(1.0)), f64::from(if along_x { d.size.width } else { d.size.height }))
                        });
                        (dpi, extent, units.point_size.per_inch(), units.rulers)
                    }
                    None => (72.0, 1.0, 72.0, Unit::Pixels),
                };

                if let Some((n, unit)) = Unit::parse_length(t) {
                    let px = unit.unwrap_or(ruler).to_px(n, dpi, extent, ppi);
                    if optional && px <= 0.0 { Value::Null } else { serde_json::Number::from_f64(px).map(Value::Number).unwrap_or(Value::Null) }
                } else if let Some(n) = widgets::parse_num(t) {
                    let px = ruler.to_px(n, dpi, extent, ppi);
                    if optional && px <= 0.0 { Value::Null } else { serde_json::Number::from_f64(px).map(Value::Number).unwrap_or(Value::Null) }
                } else {
                    json!(s)
                }
            }
        }
        _ => {
            if optional {
                Value::Null
            } else {
                json!(0.0)
            }
        }
    }
}

pub fn params(f: &Map<String, Value>) -> Value {
    params_with(None, f)
}

pub fn params_with(app: Option<&PhotocraftApp>, f: &Map<String, Value>) -> Value {
    let mut p: Map<String, Value> = ["columns", "rows", "width", "height", "gutter", "rowGutter", "margin", "centerColumns", "clearExisting", "target"]
        .into_iter()
        .filter_map(|k| f.get(k).cloned().map(|v| (k.to_string(), v)))
        .collect();
    for (toggle, key) in [("__columnsEnabled", "columns"), ("__rowsEnabled", "rows")] {
        if !on(f, toggle, positive(f, key)) {
            p.insert(key.into(), json!(0));
        }
    }
    if !on(f, "__marginEnabled", true) {
        p.insert("margin".into(), json!(0));
    } else if let Some(margin_val) = f.get("margin") {
        let parsed = match margin_val {
            Value::Array(arr) if arr.len() == 4 => {
                let top = parse_length_value(Some(&arr[0]), app, false, false);
                let left = parse_length_value(Some(&arr[1]), app, true, false);
                let bottom = parse_length_value(Some(&arr[2]), app, false, false);
                let right = parse_length_value(Some(&arr[3]), app, true, false);
                json!([top, left, bottom, right])
            }
            Value::Array(_) => json!(0),
            other => {
                let m = parse_length_value(Some(other), app, false, false);
                json!([m.clone(), m.clone(), m.clone(), m])
            }
        };
        p.insert("margin".into(), parsed);
    }
    p.insert("width".into(), parse_length_value(f.get("width"), app, true, true));
    p.insert("height".into(), parse_length_value(f.get("height"), app, false, true));
    p.insert("gutter".into(), parse_length_value(f.get("gutter"), app, true, false));
    p.insert("rowGutter".into(), parse_length_value(f.get("rowGutter"), app, false, false));

    // A disabled axis must not be rejected for a stale width or gap typed before disabling it.
    for (key, size, gap) in [("columns", "width", "gutter"), ("rows", "height", "rowGutter")] {
        if p.get(key).and_then(Value::as_u64) == Some(0) {
            p.insert(size.into(), Value::Null);
            p.insert(gap.into(), json!(0));
        }
    }
    Value::Object(p)
}

fn source(app: &PhotocraftApp) -> Value {
    app.session.active().map(|st| json!([st.doc.id, st.revision, st.active_layer, st.selected_layers])).unwrap_or(Value::Null)
}

fn calculate(app: &PhotocraftApp, f: &Map<String, Value>) -> Result<Guides, String> {
    let st = app.session.active().ok_or("no document")?;
    if f.get("__document") != Some(&json!(st.doc.id)) {
        return Err("the guide layout belongs to another document".into());
    }
    let params = params_with(Some(app), f);
    // Painting an unchanged dialog must not repeat the duplicate scans every frame.
    if f.get("__previewParams") == Some(&params)
        && f.get("__previewSource") == Some(&source(app))
        && let Some(guides) = f.get("__previewGuides").and_then(|v| serde_json::from_value(v.clone()).ok())
    {
        return Ok(guides);
    }
    photocraft_engine::guide_layout::calculate(&app.session, &params).map(|l| l.guides).map_err(|e| e.to_string())
}

pub fn validate(app: &PhotocraftApp, f: &Map<String, Value>) -> Result<(), String> {
    let color = f.get("color").and_then(Value::as_str).unwrap_or("#4affff");
    rgb(color).ok_or("invalid guide color")?;
    calculate(app, f).map(|_| ())
}

pub fn confirm(app: &mut PhotocraftApp, f: &Map<String, Value>) -> Result<Value, String> {
    validate(app, f)?;
    let color = f.get("color").and_then(Value::as_str).unwrap_or("#4affff");
    // Colour is the existing global guide display preference, not document data.
    let color = rgb(color).ok_or("invalid guide color")?;
    let hex = format!("#{:02x}{:02x}{:02x}", color.r(), color.g(), color.b());
    let params = params_with(Some(app), f);
    // Lengths are remembered as the pixels they applied, so a bare "5" typed in px still means
    // 5 px after the ruler unit changes; reopening shows them in the new unit. Percent stays
    // percent, of whatever the next target is.
    let mut remembered = f.clone();
    for (key, along_x, optional) in [("width", true, true), ("height", false, true), ("gutter", true, false), ("rowGutter", false, false)] {
        if let Some(v) = f.get(key) {
            remembered.insert(key.into(), parse_length_value(Some(v), Some(app), along_x, optional));
        }
    }
    if let Some(Value::Array(m)) = f.get("margin")
        && m.len() == 4
    {
        let px = m.iter().zip([false, true, false, true]).map(|(v, along_x)| parse_length_value(Some(v), Some(app), along_x, false)).collect();
        remembered.insert("margin".into(), Value::Array(px));
    }
    let result = app.run(COMMAND, params)?;
    // The preview is visible even when guides were hidden. Keep a committed layout visible too.
    app.ui.view.extras = true;
    app.ui.view.show.canvas_guides = true;
    app.ui.extras.guides = true;
    app.session.prefs.edit(|prefs| prefs.guides_grid_and_slices.guide_color = hex);
    for key in [MARK, "__command", "__label", "__document", "__previewGuides", "__previewParams", "__previewSource"] {
        remembered.remove(key);
    }
    app.ui.view.guide_layout = Value::Object(remembered);
    Ok(result)
}

/// Full replacement guide set for the dialog's own document. Cancel/removal drops it immediately.
pub fn preview(app: &PhotocraftApp, doc: &Document) -> Option<(Guides, Color32)> {
    let dialog = app.ui.dialogs.iter().rev().find(|d| owns(&d.fields))?;
    let f = &dialog.fields;
    if !on(f, "__preview", true) || f.get("__document") != Some(&json!(doc.id)) {
        return None;
    }
    let guides = calculate(app, f).ok().or_else(|| f.get("__previewGuides").and_then(|v| serde_json::from_value(v.clone()).ok()))?;
    let color = f.get("color").and_then(Value::as_str).and_then(rgb).unwrap_or(Color32::from_rgb(74, 255, 255));
    Some((guides, color))
}

fn check(ui: &mut egui::Ui, f: &mut Map<String, Value>, key: &str, label: &str, fallback: bool) -> bool {
    let mut value = on(f, key, fallback);
    if widgets::checkbox(ui, &mut value, label).changed() {
        f.insert(key.into(), json!(value));
    }
    value
}

fn count_field(ui: &mut egui::Ui, f: &mut Map<String, Value>, key: &str, name: &str) {
    let mut value = f.get(key).and_then(Value::as_f64).unwrap_or(0.0) as f32;
    if widgets::value_field(ui, &mut value, 1.0..=1000.0, "", 68.0)
        .accessible_name(tl!(name))
        .changed()
        // A disabled field still clamps its value into range (a zero count to 1); writing that
        // back would switch the axis on through its `positive` fallback (#5625).
        && ui.is_enabled()
    {
        f.insert(key.into(), json!(value.round() as u64));
    }
}

/// Tab selects the text even while a mouse button is held; a click places the caret.
fn should_select_all(response: &egui::Response, ui: &egui::Ui) -> bool {
    response.gained_focus()
        && (crate::field_tab::reached(ui.ctx(), response.id)
            || (!response.clicked() && !response.dragged() && !ui.input(|i| i.pointer.any_click() || i.pointer.any_down())))
}

fn length_field(app: &PhotocraftApp, ui: &mut egui::Ui, f: &mut Map<String, Value>, key: &str, label: egui::Id, optional: bool, along_x: bool) {
    let mut value = f.get(key).cloned().unwrap_or(Value::Null);
    let response = unit_edit(app, ui, key, &mut value, optional, along_x, 68.0).labelled_by(label);
    if f.get(key).unwrap_or(&Value::Null) != &value {
        f.insert(key.into(), value);
    }
    if optional {
        response.on_hover_text(tl!("Leave empty for automatic size"));
    }
}

/// A length field that takes any unit. Its text is stored as typed and settles on lost focus.
fn unit_edit(app: &PhotocraftApp, ui: &mut egui::Ui, id_salt: &str, value: &mut Value, optional: bool, along_x: bool, width: f32) -> egui::Response {
    let mut text = match value {
        Value::String(s) => s.clone(),
        Value::Number(n) => {
            let px = n.as_f64().unwrap_or(0.0);
            if optional && px <= 0.0 { String::new() } else { in_ruler_unit(app, px, along_x) }
        }
        _ if optional => String::new(),
        _ => in_ruler_unit(app, 0.0, along_x),
    };
    let t = Tokens::get(ui.ctx());
    let response = ui.add(egui::TextEdit::singleline(&mut text).id_salt(id_salt).desired_width(width).font(crate::theme::mono(12.0)).background_color(t.field));
    if ui.is_enabled() {
        crate::field_tab::register(ui.ctx(), response.id);
    }
    let ruler = ruler_unit(app);
    if response.gained_focus() {
        let unit = text_unit(&text, ruler);
        ui.ctx().data_mut(|d| d.insert_temp(response.id, unit));
    }
    if should_select_all(&response, ui) {
        crate::field_tab::select_all(ui.ctx(), response.id, &text);
    }
    if response.changed() {
        *value = json!(text);
    }
    if response.lost_focus() {
        let unit = ui.ctx().data_mut(|d| d.remove_temp::<Unit>(response.id)).unwrap_or(ruler);
        // Leaving a field untouched keeps its stored value (an automatic size stays empty), so
        // tabbing past it does not turn a named preset into Custom.
        if let Some(normalized) = normalize_field_text(&text, app, along_x, optional, unit)
            && normalized != text
        {
            *value = json!(normalized);
        }
    }
    response
}

fn group<R>(ui: &mut egui::Ui, content: impl FnOnce(&mut egui::Ui) -> R) -> R {
    let t = Tokens::get(ui.ctx());
    egui::Frame::new().stroke(Stroke::new(1.0, t.field_border)).inner_margin(8).show(ui, |ui| ui.vertical(content).inner).inner
}

fn axis(app: &PhotocraftApp, ui: &mut egui::Ui, f: &mut Map<String, Value>, rows: bool) {
    let (toggle, count, size, gap, title, size_label) = if rows {
        ("__rowsEnabled", "rows", "height", "rowGutter", tl!("Rows"), tl!("Height"))
    } else {
        ("__columnsEnabled", "columns", "width", "gutter", tl!("Columns"), tl!("Width"))
    };
    group(ui, |ui| {
        let enabled = check(ui, f, toggle, title, positive(f, count));
        if enabled && f.get(count).and_then(Value::as_f64) == Some(0.0) {
            f.insert(count.into(), json!(1));
        }
        ui.add_space(6.0);
        ui.add_enabled_ui(enabled, |ui| {
            egui::Grid::new(count).spacing([8.0, 6.0]).show(ui, |ui| {
                ui.label(tl!("Number"));
                count_field(ui, f, count, "Number");
                ui.end_row();
                let size_label_resp = ui.label(size_label);
                length_field(app, ui, f, size, size_label_resp.id, true, !rows);
                ui.end_row();
                let gap_label_resp = ui.label(tl!("Spacing"));
                length_field(app, ui, f, gap, gap_label_resp.id, false, !rows);
                ui.end_row();
            });
        });
    });
}

fn main_fields(app: &PhotocraftApp, ui: &mut egui::Ui, f: &mut Map<String, Value>) {
    let before: Map<String, Value> = f.iter().filter(|(key, _)| !key.starts_with("__preview")).map(|(k, v)| (k.clone(), v.clone())).collect();
    egui::Grid::new("guide-header").spacing([8.0, 6.0]).show(ui, |ui| {
        ui.label(tl!("Preset:"));
        let mut preset = f.get("__preset").and_then(Value::as_str).unwrap_or("custom").to_string();
        if widgets::dropdown(
            ui,
            "guide-preset",
            tl!("Preset:"),
            &mut preset,
            &[("custom".into(), "Custom"), ("five".into(), "5 Columns, 3 Rows"), ("eight".into(), "8 Columns"), ("twelve".into(), "12 Columns")],
            222.0,
        ) {
            f.insert("__preset".into(), json!(preset));
            match preset.as_str() {
                "five" | "eight" | "twelve" => {
                    let (cols, rows, gap) = match preset.as_str() {
                        "five" => (5, 3, 0.0),
                        "eight" => (8, 0, 20.0),
                        _ => (12, 0, 24.0),
                    };
                    for (k, v) in [
                        ("columns", json!(cols)),
                        ("rows", json!(rows)),
                        ("gutter", json!(in_ruler_unit(app, gap, true))),
                        ("rowGutter", json!(in_ruler_unit(app, 0.0, false))),
                        ("width", Value::Null),
                        ("height", Value::Null),
                        (
                            "margin",
                            json!([
                                in_ruler_unit(app, 0.0, false),
                                in_ruler_unit(app, 0.0, true),
                                in_ruler_unit(app, 0.0, false),
                                in_ruler_unit(app, 0.0, true)
                            ]),
                        ),
                        ("__columnsEnabled", json!(true)),
                        ("__rowsEnabled", json!(rows > 0)),
                        ("__marginEnabled", json!(true)),
                        ("centerColumns", json!(false)),
                    ] {
                        f.insert(k.into(), v);
                    }
                }
                _ => {}
            }
        }
        ui.end_row();
        ui.label(tl!("Target:"));
        let mut target = f.get("target").and_then(Value::as_str).unwrap_or("document").to_string();
        let mut options = vec![("document".to_string(), "Document")];
        if !photocraft_engine::guide_layout::selected_artboards(&app.session).is_empty() {
            options.push(("selectedArtboards".into(), "Selected Artboards"));
        }
        if widgets::dropdown(ui, "guide-target", tl!("Target"), &mut target, &options, 222.0) {
            f.insert("target".into(), json!(target));
        }
        ui.end_row();
    });
    ui.add_space(10.0);
    group(ui, |ui| {
        ui.label(tl!("Color"));
        ui.horizontal(|ui| {
            ui.label(tl!("Color:"));
            let mut color = f.get("color").and_then(Value::as_str).unwrap_or("#4affff").to_string();
            let colors = color_options(&color);
            if widgets::dropdown(ui, "guide-color", tl!("Color"), &mut color, &colors, 174.0) {
                f.insert("color".into(), json!(color));
            }
            let shown = rgb(&color).unwrap_or(Tokens::get(ui.ctx()).accent);
            if widgets::color_swatch_button(ui, shown, tl!("Guide color")).clicked() {
                crate::color_picker_ui::request(f, tl!("Guide color"));
            }
        });
    });
    ui.add_space(10.0);
    ui.horizontal_top(|ui| {
        axis(app, ui, f, false);
        ui.add_space(4.0);
        axis(app, ui, f, true);
    });
    ui.add_space(10.0);
    group(ui, |ui| {
        let enabled = check(ui, f, "__marginEnabled", tl!("Margin"), true);
        let mut margins = f.get("margin").and_then(Value::as_array).cloned().unwrap_or_else(|| vec![json!(0); 4]);
        if margins.len() != 4 {
            margins = vec![json!(0); 4];
        }
        ui.add_space(5.0);
        ui.add_enabled_ui(enabled, |ui| {
            ui.horizontal(|ui| {
                let items = [
                    (tl!("Top:"), "margin-top", false),
                    (tl!("Left:"), "margin-left", true),
                    (tl!("Bottom:"), "margin-bottom", false),
                    (tl!("Right:"), "margin-right", true),
                ];
                for (value, (label, id_salt, along_x)) in margins.iter_mut().zip(items) {
                    ui.vertical(|ui| {
                        ui.label(label);
                        unit_edit(app, ui, id_salt, value, false, along_x, 64.0).accessible_name(label);
                    });
                }
            });
        });
        f.insert("margin".into(), Value::Array(margins));
    });
    ui.add_space(8.0);
    check(ui, f, "centerColumns", tl!("Center Columns"), false);
    check(ui, f, "clearExisting", tl!("Clear Existing Guides"), false);
    // A typed edit makes a named preset custom; changing target/colour/preview does not.
    if ["columns", "rows", "width", "height", "gutter", "rowGutter", "margin", "centerColumns", "__columnsEnabled", "__rowsEnabled", "__marginEnabled"]
        .iter()
        .any(|k| before.get(*k) != f.get(*k))
        && before.get("__preset") == f.get("__preset")
    {
        f.insert("__preset".into(), json!("custom"));
    }
}

/// Own the action rail as well as the body, so OK/Cancel/Preview follow Photoshop's arrangement.
pub fn body(app: &PhotocraftApp, ui: &mut egui::Ui, f: &mut Map<String, Value>, interactive: bool) -> Option<bool> {
    let before = (params_with(Some(app), f), on(f, "__preview", true), f.get("color").cloned());
    let mut outcome = None;
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            main_fields(app, ui, f);
        });
        ui.add_space(12.0);
        ui.vertical(|ui| {
            let valid = validate(app, f).is_ok();
            if ui.add_enabled_ui(valid, |ui| widgets::primary_button(ui, tl!("OK"), 96.0)).inner.clicked() {
                outcome = Some(true);
            }
            ui.add_space(4.0);
            if widgets::secondary_button(ui, tl!("Cancel"), 96.0).clicked() {
                outcome = Some(false);
            }
            ui.add_space(6.0);
            check(ui, f, "__preview", tl!("Preview"), true);
        });
    });
    match calculate(app, f) {
        Ok(guides) => {
            f.insert("__previewGuides".into(), json!(guides));
            f.insert("__previewParams".into(), params_with(Some(app), f));
            f.insert("__previewSource".into(), source(app));
        }
        Err(error) => {
            ui.colored_label(Tokens::get(ui.ctx()).text_dim, tl!("Invalid guide layout")).on_hover_text(error);
        }
    }
    if outcome.is_none() && interactive && validate(app, f).is_ok() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
        outcome = Some(true);
    }
    // The canvas paints before the dialog. Schedule the frame that displays this draft edit.
    if before != (params_with(Some(app), f), on(f, "__preview", true), f.get("color").cloned()) {
        ui.ctx().request_repaint();
    }
    outcome
}

#[cfg(test)]
mod tests;
