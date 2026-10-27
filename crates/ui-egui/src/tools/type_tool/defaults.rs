use super::editing::edit_home;
use super::*;

/// Representative for the displayed properties: the edit layer, else a selected type layer.
pub(crate) fn target(app: &PhotocraftApp) -> Option<(u64, Option<[usize; 2]>)> {
    if let Some(ed) = &app.ui.text_edit {
        let (a, b) = (ed.caret.min(ed.anchor), ed.caret.max(ed.anchor));
        return Some((ed.layer, (a < b).then_some([a, b])));
    }
    let st = app.session.active()?;
    let id = st
        .active_layer
        .filter(|id| text_layer(&st.doc, *id).is_some())
        .or_else(|| st.selected_layers().into_iter().find(|id| text_layer(&st.doc, *id).is_some()))?;
    Some((id.0, None))
}

/// The type layer whose properties the Type controls and dialogs show (see [`target`]).
pub(crate) fn target_text(app: &PhotocraftApp) -> Option<&TextLayer> {
    let (id, _) = target(app)?;
    text_layer(&target_doc(app)?.doc, LayerId(id))
}

/// The document [`target`]'s layer is in: a text edit's own, which can stay in a background
/// document where the same layer id may name another layer, else the active one.
pub(super) fn target_doc(app: &PhotocraftApp) -> Option<&photocraft_engine::DocState> {
    match &app.ui.text_edit {
        Some(ed) => app.session.documents().get(edit_home(app, ed)?),
        None => app.session.active(),
    }
}

/// Snapshot the mutation scope separately from the representative used to display properties.
/// An inline edit remains on its layer/range; layer selection formats every selected type layer.
pub(crate) fn formatting_params(app: &PhotocraftApp) -> Option<serde_json::Value> {
    if let Some(ed) = &app.ui.text_edit {
        let mut params = json!({"layer": ed.layer, "coalesce": ed.session});
        let (a, b) = (ed.caret.min(ed.anchor), ed.caret.max(ed.anchor));
        if a < b {
            params["range"] = json!([a, b]);
        }
        return Some(params);
    }
    let st = app.session.active()?;
    let layers: Vec<u64> = st.selected_layers().into_iter().filter(|id| text_layer(&st.doc, *id).is_some()).map(|id| id.0).collect();
    (!layers.is_empty()).then(|| json!({"layers": layers}))
}

/// Scale of the target layer's transform. Like Photoshop, sizes show and edit as the layer
/// appears: a 12 pt layer scaled 200% (common in PSDs) reads 24 pt.
pub(super) fn shown_scale(app: &PhotocraftApp) -> f32 {
    let Some((id, _)) = target(app) else { return 1.0 };
    let Some(t) = app.session.active().and_then(|st| text_layer(&st.doc, LayerId(id))) else { return 1.0 };
    let m = t.transform.m;
    let k = (m[0] * m[3] - m[1] * m[2]).abs().sqrt() as f32;
    if k.is_finite() && k > 1e-3 { k } else { 1.0 }
}

/// The display unit for type metrics (Preferences ▸ Units & Rulers ▸ Type): the multiplier from
/// the model's points to the unit, and its label. Pixels follow the document's resolution.
pub(crate) fn type_unit(app: &PhotocraftApp) -> (f32, &'static str) {
    let dpi = app.session.active().map_or(72.0, |d| d.doc.resolution_dpi.max(1.0));
    match app.session.prefs().units_and_rulers.type_units {
        photocraft_engine::prefs::TypeUnit::Points => (1.0, "pt"),
        photocraft_engine::prefs::TypeUnit::Pixels => (dpi / 72.0, "px"),
        photocraft_engine::prefs::TypeUnit::Millimeters => (25.4 / 72.0, "mm"),
    }
}

/// Coalesce key while a field is being scrubbed: every step of one drag shares one history step
/// (Photoshop's single step per scrub, #124). Unique per press, so two drags are two steps.
fn drag_key(ctx: &egui::Context) -> Option<String> {
    let id = ctx.dragged_id()?;
    let t = ctx.input(|i| i.pointer.press_start_time())?;
    Some(format!("type-drag-{}-{}", id.value(), t.to_bits()))
}

/// The styles a new Type-tool layer will receive. Use the same command parameters for the
/// panels and creation so defaults also survive action recording/replay. The older options-bar
/// fields remain authoritative, and text colour continues to follow the foreground colour.
pub(super) fn default_style_params(app: &PhotocraftApp) -> serde_json::Value {
    let o = &app.ui.tool_options;
    let mut p = json!({});
    // Settings are editable JSON: only recognized formatting keys may reach type.create.
    // In particular a stray `text`, `box`, or `coalesce` key must not become a create parameter.
    let (mut c, mut para) = (photocraft_doc::text::CharStyle::default(), photocraft_doc::text::ParagraphStyle::default());
    for (key, value) in &o.type_style_defaults {
        // These belong to the existing options-bar fields. Apart from being redundant,
        // parsing a saved font key here would wait for the background font scan.
        if matches!(key.as_str(), "font" | "family" | "fontStyle" | "weight" | "italic" | "postscriptName") {
            continue;
        }
        let one = json!({ (key): value });
        if !finite_style_value(value)
            || photocraft_engine::type_cmds::check_size_tracking(&one).is_err()
            || photocraft_engine::type_cmds::check_kerning(&one).is_err()
        {
            continue;
        }
        if photocraft_engine::type_cmds::apply_char_props(&mut c, &one) | photocraft_engine::type_cmds::apply_para_props(&mut para, &one) {
            p[key] = value.clone();
        }
    }
    p["font"] = json!(o.type_font);
    p["fontStyle"] = json!(o.type_style);
    p["size"] = json!(o.type_size);
    p["align"] = json!(o.type_align);
    p["direction"] = json!(o.type_direction);
    p["color"] = json!(hex(app.session.tools.foreground));
    p
}

/// Style numbers become f32 in the engine. JSON itself permits finite f64 values too large
/// for that conversion; a hand-edited setting must not inject infinity into a new layer.
/// Style collections (features, axes, colour) contain only scalar values, so no recursion.
fn finite_style_value(value: &serde_json::Value) -> bool {
    let scalar = |v: &serde_json::Value| match v {
        serde_json::Value::Number(n) => n.as_f64().is_some_and(|n| (n as f32).is_finite()),
        serde_json::Value::Array(_) | serde_json::Value::Object(_) => false,
        _ => true,
    };
    match value {
        serde_json::Value::Array(v) => v.iter().all(scalar),
        serde_json::Value::Object(v) => v.values().all(scalar),
        v => scalar(v),
    }
}

pub(super) fn default_styles(app: &PhotocraftApp) -> (photocraft_doc::text::CharStyle, photocraft_doc::text::ParagraphStyle) {
    let (mut c, mut para) = app.session.type_defaults.clone().unwrap_or_default();
    let mut p = default_style_params(app);
    // The saved panels can appear on the very first frame. Style parsing resolves the named
    // font through shared(), which waits for the font scan; display the saved names meanwhile,
    // and let fonts_ready/axis_sliders expose the font controls when try_shared is ready.
    if photocraft_text::try_shared().is_none() {
        c.font_family = app.ui.tool_options.type_font.clone();
        c.font_style = app.ui.tool_options.type_style.clone();
        c.postscript_name = None;
        if let Some(p) = p.as_object_mut() {
            p.remove("font");
            p.remove("fontStyle");
        }
    }
    photocraft_engine::type_cmds::apply_char_props(&mut c, &p);
    photocraft_engine::type_cmds::apply_para_props(&mut para, &p);
    (c, para)
}

/// Change tool defaults without creating a document edit. The engine owns the style schema;
/// keep its property names rather than maintaining another style parser in the shell.
pub(super) fn apply_defaults(app: &mut PhotocraftApp, props: &serde_json::Value) {
    let Some(props) = props.as_object() else { return };
    let o = &mut app.ui.tool_options;
    for (key, value) in props {
        match key.as_str() {
            "font" => {
                if let Some(v) = value.as_str() {
                    o.type_font = v.into();
                }
            }
            "fontStyle" => {
                if let Some(v) = value.as_str() {
                    o.type_style = v.into();
                    // A newly chosen named style replaces a weight-axis setting, as it does
                    // in apply_char_props. Other axes retain their independently chosen values.
                    if let Some(axes) = o.type_style_defaults.get_mut("variations").and_then(serde_json::Value::as_object_mut) {
                        axes.remove("wght");
                    }
                }
            }
            "size" => {
                if let Some(v) = value.as_f64() {
                    o.type_size = v as f32;
                }
            }
            "align" => {
                if let Some(v) = value.as_str() {
                    o.type_align = v.into();
                }
            }
            "direction" => {
                if let Some(v) = value.as_str() {
                    o.type_direction = v.into();
                }
            }
            "metricsAsShown" => {} // Defaults have no layer transform.
            "features" | "variations" => {
                if let Some(patch) = value.as_object() {
                    let old = o.type_style_defaults.entry(key.clone()).or_insert_with(|| json!({}));
                    if let Some(old) = old.as_object_mut() {
                        old.extend(patch.clone());
                    } else {
                        *old = value.clone();
                    }
                }
            }
            _ => {
                o.type_style_defaults.insert(key.clone(), value.clone());
            }
        }
    }
}

/// Apply only the supplied properties to the text-edit scope or selected type layers; without
/// a type target, prepare the defaults for the next layer instead.
pub(super) fn apply(app: &mut PhotocraftApp, ctx: &egui::Context, props: serde_json::Value) {
    let Some(mut p) = formatting_params(app) else {
        apply_defaults(app, &props);
        return;
    };
    if let Some(props) = props.as_object() {
        for (key, value) in props {
            p[key] = value.clone();
        }
    }
    if let Some(key) = app.ui.text_edit.as_ref().map(|ed| ed.session.clone()).or_else(|| drag_key(ctx)) {
        p["coalesce"] = json!(key);
    }
    let _ = app.run("type.setStyle", p);
}

/// `props` aimed at the type target: its layer, the selected characters while typing, and the
/// typing session's undo step. `None` without a type target.
pub(crate) fn with_target(app: &PhotocraftApp, mut props: serde_json::Value) -> Option<serde_json::Value> {
    let (layer, range) = target(app)?;
    props["layer"] = json!(layer);
    if let Some(r) = range {
        props["range"] = json!(r);
    }
    if let Some(key) = app.ui.text_edit.as_ref().map(|ed| ed.session.clone()) {
        props["coalesce"] = json!(key);
    }
    Some(props)
}

/// Open the shared Color Picker with a snapshot of the text target. Sampling or cancelling
/// the dialog never changes the text selection, tool colours, or document history.
pub fn open_color_picker(app: &mut PhotocraftApp, rgb: [f32; 3]) -> u64 {
    if let Some(params) = formatting_params(app) {
        crate::color_picker_ui::open_for_command(app, "Color Picker (Text Color)", rgb, "type.setStyle", params)
    } else {
        crate::color_picker_ui::open(app, "foreground")
    }
}

/// While characters are selected in a type layer, a new foreground colour (Color and Swatches
/// panels, the Color Picker) recolours them, in the editing session's history step.
pub fn foreground_changed(app: &mut PhotocraftApp) {
    let Some((layer, Some(range))) = target(app) else { return };
    let Some(ed) = app.ui.text_edit.as_ref() else { return };
    let p = json!({"layer": layer, "range": range, "color": hex(app.session.tools.foreground), "coalesce": ed.session});
    let _ = app.run("type.setStyle", p);
}
