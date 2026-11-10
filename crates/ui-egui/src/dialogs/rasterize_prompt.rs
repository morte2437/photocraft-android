//! "Rasterize?" prompt: painting the pixels of a type, shape, Smart Object or fill layer with a
//! pixel tool asks first, as Photoshop does ("This type layer must be rasterized before
//! proceeding. Its text will no longer be editable. Rasterize the type?"), instead of failing
//! with a status-bar message.
//!
//! The prompt is an ordinary dialog in the UI state (`ui.inspect` shows it, `ui.dialog.confirm`
//! and `ui.dialog.cancel` answer it). OK runs the matching `layer.rasterize.*` command and nothing
//! else: the click that opened the prompt is not replayed, as in Photoshop (#8806), so the next
//! click paints. Cancel does nothing.
//!
//! A filter on a type layer asks the same question (#4303), with Photoshop's Convert to Smart
//! Object / Rasterize / Cancel: the layer is converted or rasterized, then the filter is chosen
//! again from the menu (its dialog, window or mode opens, or it runs), a smart filter on the Smart
//! Object. `ui.dialog.set` with `"choice": "smartObject"` picks the conversion.

use serde_json::{Map, Value, json};

use crate::PhotocraftApp;
use crate::state::{DialogKind, Tool};

const MARK: &str = "__rasterize";

/// A layer kind that must be rasterized before its pixels can be painted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Type,
    Shape,
    SmartObject,
    Fill,
}

impl Kind {
    pub const ALL: [Kind; 4] = [Kind::Type, Kind::Shape, Kind::SmartObject, Kind::Fill];

    fn key(self) -> &'static str {
        match self {
            Kind::Type => "type",
            Kind::Shape => "shape",
            Kind::SmartObject => "smartObject",
            Kind::Fill => "fill",
        }
    }

    /// The command that rasterizes it.
    pub fn command(self) -> &'static str {
        match self {
            Kind::Type => "layer.rasterize.type",
            Kind::Shape => "layer.rasterize.shape",
            Kind::SmartObject => "layer.rasterize.smartObject",
            Kind::Fill => "layer.rasterize.fillContent",
        }
    }

    /// Photoshop's question.
    pub fn message(self) -> &'static str {
        match self {
            Kind::Type => "This type layer must be rasterized before proceeding. Its text will no longer be editable. Rasterize the type?",
            Kind::Shape => "This shape layer must be rasterized before proceeding. It will no longer be editable as a shape. Rasterize the shape?",
            // As measured in the reference application (#11899): lower-case "smart object".
            Kind::SmartObject => {
                "This smart object must be rasterized before proceeding. Edit Contents will no longer be available. Rasterize the smart object?"
            }
            Kind::Fill => "This fill layer must be rasterized before proceeding. Its fill content will no longer be editable. Rasterize the layer?",
        }
    }

    fn from_key(k: &str) -> Option<Kind> {
        Kind::ALL.into_iter().find(|x| x.key() == k)
    }
}

/// Does `tool` paint the layer's pixels (so a vector or Smart Object layer must be rasterized)?
pub fn paints_pixels(app: &PhotocraftApp, tool: Tool) -> bool {
    tool.is_brushlike()
        || matches!(tool, Tool::PaintBucket | Tool::MagicEraser | Tool::RedEye)
        || (tool == Tool::Gradient && app.ui.tool_options.gradient_classic)
}

/// The active layer's kind when painting its pixels needs rasterizing first: the tool targets
/// the layer's pixels (not its mask, an alpha channel or the Quick Mask) and the layer is a
/// type, shape, Smart Object or fill layer that isn't locked.
pub fn needed(app: &PhotocraftApp) -> Option<(Kind, photocraft_doc::LayerId)> {
    if crate::canvas::paint_target(app) != json!("pixels") {
        return None;
    }
    let st = app.session.active()?;
    let id = st.active_layer?;
    let l = st.doc.layer(id)?;
    let locks = st.doc.effective_locks(id);
    if locks.all || locks.pixels {
        return None;
    }
    use photocraft_doc::LayerContent as C;
    let kind = match &l.content {
        C::Text(_) => Kind::Type,
        C::Shape(_) => Kind::Shape,
        C::Smart(_) => Kind::SmartObject,
        C::Fill(_) => Kind::Fill,
        _ => return None,
    };
    Some((kind, id))
}

/// A tool's pointer press: when it would paint a layer that must be rasterized first, park it
/// behind the prompt and return true (the tool must not see the press).
pub fn intercept(app: &mut PhotocraftApp, tool: Tool, x: f64, y: f64, pressure: f32) -> bool {
    if !paints_pixels(app, tool) {
        return false;
    }
    let Some((kind, layer)) = needed(app) else { return false };
    if !app.ui.dialogs.iter().any(|d| owns(&d.fields)) {
        open(app, kind, layer, tool, [x, y, f64::from(pressure)]);
    }
    true
}

/// Open the prompt for `layer`; OK rasterizes it. `tool` and `at` (x, y, pressure) only record the press that opened it.
pub fn open(app: &mut PhotocraftApp, kind: Kind, layer: photocraft_doc::LayerId, tool: Tool, at: [f64; 3]) -> u64 {
    let mut f = Map::new();
    f.insert(MARK.into(), json!(kind.key()));
    f.insert("__command".into(), json!(kind.command()));
    f.insert("__label".into(), json!("PhotoCraft"));
    f.insert("message".into(), json!(kind.message()));
    f.insert("layer".into(), json!(layer.0));
    f.insert("tool".into(), json!(format!("{tool:?}")));
    f.insert("at".into(), json!(at));
    app.ui.open_dialog(DialogKind::Command, f)
}

/// The active type layer when `id` is a filter that only needs it rasterized: the engine greys it
/// for not being a pixel layer, and the layer is not locked. The flag says whether the filter also
/// runs on a Smart Object, as a smart filter (Vanishing Point doesn't).
pub fn filter_needs(app: &PhotocraftApp, id: &str) -> Option<(photocraft_doc::LayerId, bool)> {
    if !id.starts_with("filter.") {
        return None;
    }
    let st = app.session.active()?;
    let layer = st.active_layer?;
    if !matches!(st.doc.layer(layer)?.content, photocraft_doc::LayerContent::Text(_)) {
        return None;
    }
    let locks = st.doc.effective_locks(layer);
    if locks.all || locks.pixels {
        return None;
    }
    // Last Filter checks the layer before the journal: with no filter to repeat it stays grey.
    if id == "filter.lastFilter" && photocraft_engine::filters::last_filter(&app.session).is_none() {
        return None;
    }
    // The engine's reason for a type layer: "filters need a pixel layer (active layer is a Type
    // layer)" from filters::has_filterable_layer, which also takes Smart Objects, or a filter's own
    // wording ("Vanishing Point needs a pixel layer (…)", "active layer is a Type layer, not a pixel
    // layer"). Any other reason (a job running) keeps the item grey.
    let reason = app.session.disabled_reason(id)?;
    (reason.contains("active layer is a Type layer") && reason.contains("pixel layer")).then(|| (layer, reason.starts_with("filters need a pixel layer")))
}

/// A filter chosen on a type layer: open the prompt instead and return its answer to the caller.
pub fn intercept_filter(app: &mut PhotocraftApp, id: &str, params: &Value) -> Option<Result<Value, String>> {
    let (layer, smart) = filter_needs(app, id)?;
    if let Some(d) = app.ui.dialogs.iter().find(|d| owns(&d.fields)) {
        return Some(Ok(json!({"dialog": d.id})));
    }
    let mut f = Map::new();
    f.insert(MARK.into(), json!(Kind::Type.key()));
    f.insert("__label".into(), json!("PhotoCraft"));
    f.insert("message".into(), json!(Kind::Type.message()));
    f.insert("layer".into(), json!(layer.0));
    f.insert("filter".into(), json!(id));
    f.insert("params".into(), params.clone());
    f.insert("choice".into(), json!("rasterize"));
    f.insert("smartObject".into(), json!(smart));
    Some(Ok(json!({"dialog": app.ui.open_dialog(DialogKind::Command, f)})))
}

/// Is this the filter prompt (Convert to Smart Object / Rasterize / Cancel)?
pub fn is_filter(fields: &Map<String, Value>) -> bool {
    owns(fields) && fields.contains_key("filter")
}

/// Does the filter prompt offer Convert to Smart Object?
pub fn offers_smart_object(fields: &Map<String, Value>) -> bool {
    fields.get("smartObject").and_then(Value::as_bool) == Some(true)
}

/// Is this dialog the prompt?
pub fn owns(fields: &Map<String, Value>) -> bool {
    fields.contains_key(MARK)
}

/// The prompt's body: the question, with a caution mark.
pub fn body(ui: &mut egui::Ui, f: &Map<String, Value>) {
    let t = crate::theme::Tokens::get(ui.ctx());
    let text = f.get("message").and_then(Value::as_str).unwrap_or("This layer must be rasterized before proceeding.");
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 12.0;
        let (r, _) = ui.allocate_exact_size(egui::vec2(32.0, 32.0), egui::Sense::hover());
        crate::icons::paint(ui, r, "triangle-alert", 30.0, t.warning);
        ui.add(egui::Label::new(egui::RichText::new(tl!(text)).color(t.text).size(13.0)).wrap());
    });
}

/// OK: rasterize the layer. The click that opened the prompt does not paint.
pub fn confirm(app: &mut PhotocraftApp, f: &Map<String, Value>) -> Result<Value, String> {
    let kind = f.get(MARK).and_then(Value::as_str).and_then(Kind::from_key).ok_or("not a rasterize prompt")?;
    let layer = f.get("layer").and_then(Value::as_u64).ok_or("the prompt has no layer")?;
    if let Some(filter) = f.get("filter").and_then(Value::as_str) {
        return confirm_filter(app, f, filter, layer);
    }
    let r = app.run(kind.command(), json!({ "layer": layer }))?;
    // The press that opened the prompt is not replayed: Photoshop only converts the layer, the
    // next click paints (#8806).
    Ok(json!({ "rasterized": r }))
}

/// The filter prompt's answer: convert or rasterize the layer, then queue the filter for
/// [`run_pending`]. Liquify, Camera Raw and the other filters with their own window need the
/// egui context to open, which a dialog's answer doesn't have.
fn confirm_filter(app: &mut PhotocraftApp, f: &Map<String, Value>, filter: &str, layer: u64) -> Result<Value, String> {
    let smart = f.get("choice").and_then(Value::as_str) == Some("smartObject");
    if smart && !offers_smart_object(f) {
        return Err(format!("{filter} can't run as a smart filter"));
    }
    let command = if smart { "layer.smartObjects.convertToSmartObject" } else { Kind::Type.command() };
    let r = app.run(command, json!({ "layer": layer }))?;
    let params = f.get("params").cloned().unwrap_or(Value::Null);
    app.pending_after_prompt = Some((filter.to_string(), params));
    Ok(json!({ "converted": r, "filter": filter }))
}

/// Runs the filter a prompt was answered for, as the menu would, now that the layer can take it.
pub fn run_pending(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let Some((filter, params)) = app.pending_after_prompt.take() else { return };
    if let Err(e) = crate::menus::invoke(app, ctx, &filter, params) {
        app.ui.status = e;
        app.ui.status_error = true;
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
#[path = "../rasterize_prompt_text_tests.rs"]
mod rasterize_prompt_text_tests;
