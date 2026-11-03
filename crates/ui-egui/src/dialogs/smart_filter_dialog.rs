//! Per-filter blending and shared target validation for Smart Filter editors.
//! Preview and commit dispatch the existing engine commands against the captured filter.

use std::sync::{Arc, Weak};

use photocraft_color::BlendMode;
use photocraft_doc::{Document, LayerContent, LayerId};
use serde_json::{Map, Value, json};

use crate::{PhotocraftApp, state::DialogKind, theme::Tokens, widgets};

pub const COMMAND: &str = "layer.smartFilter.blendingOptions";
const TARGET: &str = "__smartFilter";

/// Private document snapshots; public dialog fields carry an opaque counter, never an address.
#[derive(Default)]
pub(crate) struct Targets {
    next: u64,
    snapshots: Vec<(u64, Weak<Document>)>,
}

impl Targets {
    /// Remember a weak handle to `doc` and return the opaque token that stands for it in a dialog's target.
    fn capture(&mut self, doc: &Arc<Document>) -> Result<u64, String> {
        self.next = self.next.checked_add(1).ok_or("Smart Filter target counter exhausted")?;
        self.snapshots.retain(|(_, doc)| doc.strong_count() > 0);
        // Dialogs normally are modal. Bound retained snapshots even if automation opens many.
        if self.snapshots.len() >= 64 {
            self.snapshots.remove(0);
        }
        self.snapshots.push((self.next, Arc::downgrade(doc)));
        Ok(self.next)
    }

    /// Whether `token` was captured from this very document allocation, not just one with the same id.
    fn matches(&self, token: u64, doc: &Arc<Document>) -> bool {
        self.snapshots.iter().find(|(id, _)| *id == token).and_then(|(_, doc)| doc.upgrade()).is_some_and(|old| Arc::ptr_eq(&old, doc))
    }
}

/// Identify the Smart Filter at `layer`/`index` (document, revision, filter) so a stale dialog can be told apart from a current one.
fn snapshot(app: &PhotocraftApp, layer: LayerId, index: usize) -> Result<Value, String> {
    let st = app.session.active().ok_or("no document open")?;
    if app.session.job_on(st.doc.id).is_some() {
        return Err(tl!("Wait for the current job to finish.").into());
    }
    let sm = match &st.doc.layer(layer).ok_or("Smart Object no longer exists")?.content {
        LayerContent::Smart(sm) => sm,
        _ => return Err("the layer is not a Smart Object".into()),
    };
    let filter = sm.smart_filters.get(index).ok_or("Smart Filter no longer exists")?;
    Ok(json!({"doc": st.doc.id.0, "revision": st.revision, "layer": layer.0, "index": index, "filter": filter}))
}

/// The `__smartFilter` dialog field: the filter's snapshot plus a guard token for the document it was taken from.
pub(crate) fn target(app: &mut PhotocraftApp, layer: LayerId, index: usize) -> Result<Value, String> {
    let mut target = snapshot(app, layer, index)?;
    let doc = &app.session.active().ok_or("no document open")?.doc;
    let token = app.smart_filter_targets.capture(doc)?;
    if let Some(fields) = target.as_object_mut() {
        fields.insert("guard".into(), json!(token));
    }
    Ok(target)
}

/// Revision/filter identity catches edits; the weak snapshot also rejects a same-id reopen.
pub(crate) fn validate_target(app: &PhotocraftApp, target: &Value) -> Result<(LayerId, usize), String> {
    let layer = LayerId(target.get("layer").and_then(Value::as_u64).ok_or("invalid Smart Filter target")?);
    let index = target.get("index").and_then(Value::as_u64).and_then(|i| usize::try_from(i).ok()).ok_or("invalid Smart Filter index")?;
    let token = target.get("guard").and_then(Value::as_u64).ok_or("invalid Smart Filter snapshot")?;
    let doc = &app.session.active().ok_or("no document open")?.doc;
    let current = snapshot(app, layer, index)?;
    let mut captured = target.clone();
    if let Some(fields) = captured.as_object_mut() {
        fields.remove("guard");
    }
    if current != captured || !app.smart_filter_targets.matches(token, doc) {
        return Err(tl!("The document or Smart Filter changed. Reopen its dialog.").into());
    }
    Ok((layer, index))
}

/// Open the Blending Options dialog for one Smart Filter, starting from its recorded mode and opacity (shown as a percentage).
pub fn open(app: &mut PhotocraftApp, layer: LayerId, index: usize) -> Result<u64, String> {
    let target = self::target(app, layer, index)?;
    let st = app.session.active().ok_or("no document open")?;
    let Some(LayerContent::Smart(sm)) = st.doc.layer(layer).map(|l| &l.content) else { return Err("the layer is not a Smart Object".into()) };
    let filter = sm.smart_filters.get(index).ok_or("Smart Filter no longer exists")?;
    let mut fields = Map::new();
    fields.insert("__command".into(), json!(COMMAND));
    fields.insert("__label".into(), json!("Blending Options"));
    fields.insert("__filterName".into(), json!(crate::smart_ui::filter_label(filter)));
    fields.insert("__filter".into(), json!(true));
    fields.insert("__preview".into(), json!(true));
    fields.insert(TARGET.into(), target);
    fields.insert("blend".into(), json!(filter.blend.label()));
    fields.insert("opacity".into(), json!(filter.opacity * 100.0));
    let fit = (220.0 / st.doc.size.width.max(st.doc.size.height).max(1) as f64 * 100.0).min(100.0);
    fields.insert("__zoom".into(), json!(fit));
    fields.insert("__imageSize".into(), json!([st.doc.size.width, st.doc.size.height]));
    app.filter_preview = None;
    Ok(app.ui.open_dialog(DialogKind::Command, fields))
}

/// Whether `fields` belong to this Blending Options dialog rather than another Smart Filter editor.
pub fn owns(fields: &Map<String, Value>) -> bool {
    fields.get("__command").and_then(Value::as_str) == Some(COMMAND) && fields.contains_key(TARGET)
}

/// The dialog title, naming the filter being edited.
pub(crate) fn title(fields: &Map<String, Value>) -> String {
    crate::i18n::fmt(tl!("Blending Options ({filter})"), &[("filter", fields.get("__filterName").and_then(Value::as_str).unwrap_or_default())])
}

/// UI opacity is always percent; the engine gets a fraction, including 1% = 0.01.
fn blend_params(fields: &Map<String, Value>) -> Result<Value, String> {
    let opacity = fields
        .get("opacity")
        .and_then(Value::as_f64)
        .filter(|v| v.is_finite() && (0.0..=100.0).contains(v))
        .ok_or(tl!("Opacity must be between 0% and 100%."))?;
    let blend = fields.get("blend").and_then(Value::as_str).ok_or("choose a blend mode")?;
    Ok(json!({"blend": blend, "opacity": opacity / 100.0}))
}

/// The command a confirmed Smart Filter dialog runs on the validated target: its blending, or `layer.smartFilter.setParams` for a parameter editor.
pub(crate) fn edit_command(app: &PhotocraftApp, fields: &Map<String, Value>) -> Result<(String, Value), String> {
    let (layer, index) = validate_target(app, fields.get(TARGET).ok_or("no Smart Filter target")?)?;
    if owns(fields) {
        let mut params = blend_params(fields)?;
        if let Some(p) = params.as_object_mut() {
            p.insert("layer".into(), json!(layer.0));
            p.insert("index".into(), json!(index));
        }
        Ok((COMMAND.into(), params))
    } else {
        Ok(("layer.smartFilter.setParams".into(), json!({"layer": layer.0, "index": index, "params": crate::filter_dialog::params_of(fields)})))
    }
}

/// Dissolve has no per-pixel coverage sampling in Smart Filter blending yet (docs/smart-filters.md).
fn mode_note(blend: Option<&str>) -> Option<&'static str> {
    (blend == Some(BlendMode::Dissolve.label())).then(|| tl!("Dissolve is approximated: it blends without Photoshop's dither."))
}

/// The Blending Options form: Mode (with the Dissolve note), Opacity, Preview and the result viewport.
pub fn body(app: &mut PhotocraftApp, ui: &mut egui::Ui, fields: &mut Map<String, Value>) {
    ui.horizontal(|ui| {
        ui.label(tl!("Mode"));
        let mut blend = fields.get("blend").and_then(Value::as_str).unwrap_or("Normal").to_string();
        let options: Vec<_> = BlendMode::LAYER_MODES.iter().map(|m| (m.label().to_string(), m.label())).collect();
        if widgets::dropdown(ui, "smart-filter-blend", tl!("Blend Mode"), &mut blend, &options, 200.0) {
            fields.insert("blend".into(), json!(blend));
        }
    });
    if let Some(note) = mode_note(fields.get("blend").and_then(Value::as_str)) {
        ui.label(egui::RichText::new(note).color(Tokens::get(ui.ctx()).text_faint).size(11.0));
    }
    let mut opacity = fields.get("opacity").and_then(Value::as_f64).unwrap_or(100.0) as f32;
    if widgets::slider_row(ui, tl!("Opacity"), &mut opacity, 0.0..=100.0, "%", None).changed() {
        fields.insert("opacity".into(), json!(opacity));
    }
    let mut preview = fields.get("__preview").and_then(Value::as_bool).unwrap_or(true);
    if widgets::checkbox(ui, &mut preview, tl!("Preview")).changed() {
        fields.insert("__preview".into(), json!(preview));
    }
    if let Err(e) = fields.get(TARGET).ok_or("no Smart Filter target").map_err(str::to_string).and_then(|t| validate_target(app, t)) {
        ui.colored_label(Tokens::get(ui.ctx()).danger, e);
        return;
    }
    viewport(app, ui, fields, preview);
}

#[derive(Clone)]
struct ViewImage {
    doc: Arc<Document>,
    texture: egui::TextureHandle,
}

/// Show the document as the dialog will leave it (the live preview, or the unchanged document while Preview is off) as a fixed-size thumbnail.
fn viewport(app: &mut PhotocraftApp, ui: &mut egui::Ui, fields: &mut Map<String, Value>, preview: bool) {
    let Some(st) = app.session.active() else { return };
    let doc = if preview {
        app.filter_preview.as_ref().filter(|p| p.key.doc == st.doc.id && p.key.revision == st.revision && !p.committing).and_then(|p| p.result.clone())
    } else {
        Some(st.doc.clone())
    };
    let Some(doc) = doc else {
        ui.allocate_ui(egui::vec2(360.0, 220.0), |ui| {
            ui.centered_and_justified(|ui| {
                ui.spinner();
            });
        });
        return;
    };
    let key = ui.scope_id().with("smart-filter-view-image");
    let mut image = ui.ctx().data(|d| d.get_temp::<ViewImage>(key));
    if image.as_ref().is_none_or(|i| !Arc::ptr_eq(&i.doc, &doc)) {
        // Fixed thumbnail size and content identity: never composite a full surface per frame.
        let buffer = photocraft_compose::thumbnail_buffer(&doc, 256);
        match app.session.color.canvas_display(&doc) {
            Ok(display) => {
                let thumb = display.to_rgba8(&buffer);
                let texture = ui.ctx().load_texture(
                    "smart-filter-view-image",
                    egui::ColorImage::from_rgba_unmultiplied([thumb.width as usize, thumb.height as usize], &thumb.pixels),
                    egui::TextureOptions::LINEAR,
                );
                image = Some(ViewImage { doc, texture });
                if let Some(i) = &image {
                    ui.ctx().data_mut(|d| d.insert_temp(key, i.clone()));
                }
            }
            Err(e) => {
                ui.colored_label(Tokens::get(ui.ctx()).danger, e.to_string());
                return;
            }
        }
    }
    let Some(image) = image else { return };
    let zoom = fields.get("__zoom").and_then(Value::as_f64).unwrap_or(100.0).clamp(0.1, 800.0) as f32;
    let size = fields
        .get("__imageSize")
        .and_then(Value::as_array)
        .and_then(|a| Some(egui::vec2(a.first()?.as_u64()? as f32, a.get(1)?.as_u64()? as f32)))
        .unwrap_or(image.texture.size_vec2());
    egui::Frame::new().fill(Tokens::get(ui.ctx()).canvas).stroke(egui::Stroke::new(1.0, Tokens::get(ui.ctx()).field_border)).show(ui, |ui| {
        ui.set_width(360.0);
        egui::ScrollArea::both().id_salt("smart-filter-image").auto_shrink([false, false]).min_scrolled_height(220.0).max_height(220.0).show(ui, |ui| {
            ui.add(egui::Image::new(&image.texture).fit_to_exact_size(size * zoom / 100.0));
        });
    });
    ui.horizontal(|ui| {
        if widgets::secondary_button(ui, "−", 32.0).clicked() {
            fields.insert("__zoom".into(), json!((zoom / 2.0).max(0.1)));
        }
        ui.label(format!("{zoom:.1}%"));
        if widgets::secondary_button(ui, "+", 32.0).clicked() {
            fields.insert("__zoom".into(), json!((zoom * 2.0).min(800.0)));
        }
    });
}

#[cfg(test)]
pub(crate) mod tests;
