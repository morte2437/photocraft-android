//! Layers panel right-click menu in Photoshop's order. Items are command ids from the menu catalog;
//! unavailable ones are greyed using the same enablement as the main menus. Items marked as menu
//! invocations (`Value::Null` params) go through `menus::invoke`, so dialogs open like in the menu bar.

use photocraft_doc::{LabelColor, Layer, LayerContent};
use serde_json::{Value, json};

/// One entry: label, command id. `None` = separator.
pub type Entry = Option<(&'static str, &'static str)>;

/// English action key; each menu translates it using its existing display context.
pub(crate) fn mask_toggle_label(enabled: bool) -> &'static str {
    if enabled { "Disable Layer Mask" } else { "Enable Layer Mask" }
}

/// The command behind "Add Layer Mask" (the panel button and this menu). Like Photoshop, an active
/// selection becomes the mask; ⌥ inverts it (Hide Selection, or Hide All without a selection).
pub fn add_mask_command(has_selection: bool, alt: bool) -> &'static str {
    match (has_selection, alt) {
        (true, false) => "layer.layerMask.revealSelection",
        (true, true) => "layer.layerMask.hideSelection",
        (false, false) => "layer.layerMask.revealAll",
        (false, true) => "layer.layerMask.hideAll",
    }
}

/// What the Layers panel "Add a mask" button does for `layer` (#2075). Like Photoshop it never
/// replaces a mask: a layer without a layer mask gets one ([`add_mask_command`]); a layer that
/// already has one gets a vector mask instead (⌥: Hide All); a layer with both (or a shape layer,
/// whose path is already its vector mask, with a layer mask) gets nothing (`None`).
pub fn mask_button_command(layer: Option<&Layer>, has_selection: bool, alt: bool) -> Option<&'static str> {
    let l = layer?;
    if l.mask.is_none() {
        Some(add_mask_command(has_selection, alt))
    } else if l.vector_mask.is_none() && !matches!(l.content, LayerContent::Shape(_)) {
        Some(if alt { "layer.vectorMask.hideAll" } else { "layer.vectorMask.revealAll" })
    } else {
        None
    }
}

/// Commands that act on the active layer only and have no sensible meaning for a selection: Blending
/// Options (one dialog), Copy Layer Style (copy from which layer?) and the per-layer mask items.
/// With several layers selected they would hit whichever layer is active, not the one that was
/// right-clicked, so the menu greys them. Adding a mask, Paste/Clear Layer Style, Rasterize and
/// Export As (one file per layer, #3629) run on every selected layer instead.
pub fn single_layer_only(id: &str) -> bool {
    let adds_mask = matches!(id, "layer.layerMask.revealAll" | "layer.layerMask.hideAll" | "layer.layerMask.revealSelection" | "layer.layerMask.hideSelection");
    matches!(id, "layer.layerStyle.blendingOptions" | "layer.layerStyle.copyLayerStyle") || (id.starts_with("layer.layerMask.") && !adds_mask)
}

/// The context menu entries for a layer (Photoshop 2026 order, trimmed to the layer kind).
pub fn entries(l: &Layer, multi: bool, has_selection: bool) -> Vec<Entry> {
    let mut v: Vec<Entry> = vec![Some((tl!("Blending Options…"), "layer.layerStyle.blendingOptions"))];
    v.push(None);
    v.push(Some((if multi { tl!("Duplicate Layers…") } else { tl!("Duplicate Layer…") }, "layer.duplicate")));
    v.push(Some((if multi { tl!("Delete Layers") } else { tl!("Delete Layer") }, "layer.delete")));
    // Groups the clicked layer, or the whole selection when several layers are selected, after
    // Photoshop's New Group dialog.
    v.push(Some((tl!("Group from Layers…"), "layer.new.groupFromLayers")));
    v.push(None);
    v.push(Some((tl!("Quick Export As PNG"), "layer.quickExportAsPng")));
    v.push(Some((tl!("Export As…"), "layer.exportAs")));
    v.push(None);
    v.push(Some((tl!("Artboard from Layers…"), "layer.new.artboardFromLayers")));
    v.push(Some((tl!("Frame from Layers…"), "layer.new.frameFromLayers")));
    if !multi && matches!(l.content, LayerContent::Shape(_) | LayerContent::Text(_)) {
        v.push(Some((tl!("Convert to Frame…"), "layer.frame.convert")));
    }
    v.push(None);
    v.push(Some((tl!("Convert to Smart Object"), "layer.smartObjects.convertToSmartObject")));
    if multi {
        // One item for the whole selection, whatever the clicked layer is: Rasterize acts on every
        // selected layer. Edit Contents and Convert to Layers work on one layer, so they are not
        // offered here.
        v.push(Some(("Rasterize Layers", "layer.rasterize.layer")));
    } else {
        match &l.content {
            LayerContent::Text(_) => v.push(Some(("Rasterize Type", "layer.rasterize.type"))),
            LayerContent::Shape(_) => v.push(Some(("Rasterize Layer", "layer.rasterize.shape"))),
            LayerContent::Smart(_) => {
                v.push(Some((tl!("Edit Contents"), "layer.smartObjects.editContents")));
                v.push(Some((tl!("Replace Contents…"), "layer.smartObjects.replaceContents")));
                v.push(Some((tl!("Relink to File…"), "layer.smartObjects.relinkToFile")));
                v.push(Some((tl!("Convert to Layers"), "layer.smartObjects.convertToLayers")));
                v.push(Some(("Rasterize Layer", "layer.rasterize.smartObject")));
            }
            LayerContent::Fill(_) => v.push(Some(("Rasterize Layer", "layer.rasterize.fillContent"))),
            _ => v.push(Some(("Rasterize Layer", "layer.rasterize.layer"))),
        }
    }
    // Photoshop offers it under Rasterize Layer for a layer with a style (it bakes the effects).
    if multi || !l.effects.items.is_empty() {
        v.push(Some(("Rasterize Layer Style", "layer.rasterize.layerStyle")));
    }
    v.push(None);
    // A selection has no single mask to toggle, apply or delete, so its menu offers Add Layer Mask only.
    if let Some(mask) = l.mask.as_ref().filter(|_| !multi) {
        v.push(Some((tl!(mask_toggle_label(mask.enabled)), "layer.layerMask.enabled")));
        v.push(Some((tl!("Apply Layer Mask"), "layer.layerMask.apply")));
        v.push(Some((tl!("Delete Layer Mask"), "layer.layerMask.delete")));
    } else {
        v.push(Some((tl!("Add Layer Mask"), add_mask_command(has_selection, false))));
    }
    v.push(Some((
        if l.clipped { tl!("Release Clipping Mask") } else { tl!("Create Clipping Mask") },
        if l.clipped { "layer.releaseClippingMask" } else { "layer.createClippingMask" },
    )));
    v.push(None);
    v.push(Some((tl!("Link Layers"), "layer.linkLayers")));
    v.push(Some((tl!("Select Linked Layers"), "layer.selectLinkedLayers")));
    v.push(None);
    v.push(Some((tl!("Copy Layer Style"), "layer.layerStyle.copyLayerStyle")));
    v.push(Some((tl!("Paste Layer Style"), "layer.layerStyle.pasteLayerStyle")));
    v.push(Some((tl!("Clear Layer Style"), "layer.layerStyle.clear")));
    v.push(None);
    if multi {
        v.push(Some((tl!("Merge Layers"), "layer.mergeLayers")));
    } else if matches!(l.content, LayerContent::Group(_)) {
        // Photoshop's Merge Group: a group merges itself, not into the layer below (#3832).
        v.push(Some((tl!("Merge Group"), "layer.mergeGroup")));
    } else {
        v.push(Some((tl!("Merge Down"), "layer.mergeDown")));
    }
    v.push(Some((tl!("Merge Visible"), "layer.mergeVisible")));
    v.push(Some((tl!("Flatten Image"), "layer.flattenImage")));
    v
}

/// The layer style menu of an effect row ("Effects" or one effect) under a layer (#2846), in
/// Photoshop's order. It acts on that layer, like Layer ▸ Layer Style.
pub fn effects_entries() -> Vec<Entry> {
    vec![
        Some((tl!("Blending Options…"), "layer.layerStyle.blendingOptions")),
        None,
        Some((tl!("Copy Layer Style"), "layer.layerStyle.copyLayerStyle")),
        Some((tl!("Paste Layer Style"), "layer.layerStyle.pasteLayerStyle")),
        Some((tl!("Clear Layer Style"), "layer.layerStyle.clear")),
        None,
        Some((tl!("Global Light…"), "layer.layerStyle.globalLight")),
        Some((tl!("Create Layer"), "layer.layerStyle.createLayer")),
        Some((tl!("Hide All Effects"), "layer.layerStyle.hideAllEffects")),
        Some((tl!("Scale Effects…"), "layer.layerStyle.scaleEffects")),
    ]
}

/// Render the menu. Pushes `(command, params)` actions; `Value::Null` params mean "invoke like the
/// menu item" (opens the command's dialog when it has one).
pub fn show(app: &crate::PhotocraftApp, ui: &mut egui::Ui, l: &Layer, on_set: bool, actions: &mut Vec<(String, Value)>) -> bool {
    crate::widgets::menu_scroll(ui, |ui| {
        ui.set_min_width(220.0);
        let mut rename = false;
        let has_selection = app.session.active().is_some_and(|s| s.doc.selection.is_some());
        items(app, ui, l, on_set, entries(l, on_set, has_selection), actions);
        ui.separator();
        if ui.button(tl!("Rename Layer…")).clicked() {
            rename = true;
            ui.close();
        }
        color_menu(app, ui, l, on_set, actions);
        rename
    })
}

/// The menu of `l`'s layer-mask thumbnail (#4575), in Photoshop's order. It acts on `l`, which it
/// selects first like the row menu.
pub fn show_mask(app: &crate::PhotocraftApp, ui: &mut egui::Ui, l: &Layer, actions: &mut Vec<(String, Value)>) {
    let Some(mask) = &l.mask else { return };
    crate::widgets::menu_scroll(ui, |ui| {
        ui.set_min_width(220.0);
        let edits = vec![
            Some((tl!(mask_toggle_label(mask.enabled)), "layer.layerMask.enabled")),
            Some((tl!("Delete Layer Mask"), "layer.layerMask.delete")),
            Some((tl!("Apply Layer Mask"), "layer.layerMask.apply")),
        ];
        items(app, ui, l, false, edits, actions);
        ui.separator();
        // ⌘⇧-, ⌘⌥- and ⌘⇧⌥-clicking the thumbnail, as menu items.
        let loads =
            [(tl!("Add Mask to Selection"), "add"), (tl!("Subtract Mask from Selection"), "subtract"), (tl!("Intersect Mask with Selection"), "intersect")];
        for (label, operation) in loads {
            if ui.button(label).clicked() {
                actions.push(("layer.select".into(), json!({"layer": l.id.0})));
                actions.push(("select.loadSelection".into(), json!({"channel": "mask", "layer": l.id.0, "operation": operation})));
                ui.close();
            }
        }
        ui.separator();
        if ui.add_enabled(crate::menus::is_enabled(app, "select.selectAndMask"), egui::Button::new(tl!("Select and Mask…"))).clicked() {
            // Like double-clicking the thumbnail with that preference on: it refines the mask.
            actions.push(("layer.select".into(), json!({"layer": l.id.0})));
            actions.push(("ui.maskTarget".into(), json!(true)));
            actions.push(("select.selectAndMask".into(), Value::Null));
            ui.close();
        }
        // Layer Mask Display Options: the rubylith's colour and opacity (`channel.options`).
        if ui.button(tl!("Mask Options…")).clicked() {
            actions.push(("layer.select".into(), json!({"layer": l.id.0})));
            actions.push(("ui.maskTarget".into(), json!(true)));
            actions.push(("channel.options".into(), Value::Null));
            ui.close();
        }
    });
}

/// Render the layer style menu of `l`'s effect rows ([`effects_entries`]); the actions select `l`
/// first when it isn't the active layer.
pub fn show_effects(app: &crate::PhotocraftApp, ui: &mut egui::Ui, l: &Layer, actions: &mut Vec<(String, Value)>) {
    crate::widgets::menu_scroll(ui, |ui| {
        ui.set_min_width(200.0);
        items(app, ui, l, false, effects_entries(), actions);
    });
}

/// The command items of a layer menu, separators collapsed.
fn items(app: &crate::PhotocraftApp, ui: &mut egui::Ui, l: &Layer, on_set: bool, entries: Vec<Entry>, actions: &mut Vec<(String, Value)>) {
    let mut last_sep = true;
    for e in entries {
        match e {
            None => {
                if !last_sep {
                    ui.separator();
                }
                last_sep = true;
            }
            Some((label, id)) => {
                // Skip commands this build doesn't have rather than showing dead items.
                if photocraft_engine::commands::find(id).is_none() && !crate::menu_catalog::CATALOG.iter().any(|m| m.3 == id) {
                    continue;
                }
                last_sep = false;
                // Enablement is exact for the active layer (or the selection); another row is
                // selected first when clicked, so its items stay available.
                let is_active = app.session.active().is_some_and(|s| s.active_layer == Some(l.id));
                let enabled = !(on_set && single_layer_only(id)) && if on_set || is_active { crate::menus::is_enabled(app, id) } else { true };
                if ui.add_enabled(enabled, egui::Button::new(tl!(&label))).clicked() {
                    if !on_set {
                        actions.push(("layer.select".into(), json!({"layer": l.id.0})));
                    }
                    actions.push((id.into(), Value::Null));
                    ui.close();
                }
            }
        }
    }
}

pub(crate) fn color_name(color: LabelColor) -> &'static str {
    crate::i18n::tr_ctx(crate::i18n::current(), "layerLabel", color.label())
}

fn common_color(app: &crate::PhotocraftApp, l: &Layer, on_set: bool) -> Option<LabelColor> {
    if !on_set {
        return Some(l.label);
    }
    let st = app.session.active()?;
    let ids = st.selected_layers();
    let mut labels = ids.iter().filter_map(|id| st.doc.layer(*id).map(|l| l.label));
    let first = labels.next()?;
    labels.all(|c| c == first).then_some(first)
}

fn color_menu(app: &crate::PhotocraftApp, ui: &mut egui::Ui, l: &Layer, on_set: bool, actions: &mut Vec<(String, Value)>) {
    ui.menu_button(tl!("Color"), |ui| {
        color_choices(app, ui, l, on_set, actions);
    });
}

/// The eye column opens the same color choices directly, without the row's full context menu.
pub(crate) fn color_choices(app: &crate::PhotocraftApp, ui: &mut egui::Ui, l: &Layer, on_set: bool, actions: &mut Vec<(String, Value)>) {
    let current = common_color(app, l, on_set);
    crate::widgets::menu_scroll(ui, |ui| {
        ui.set_min_width(170.0);
        for color in LabelColor::ALL {
            if color == LabelColor::Red {
                ui.separator();
            }
            if color_button(ui, color, current == Some(color)).clicked() {
                let params = if on_set {
                    json!({"color": color.id()})
                } else {
                    actions.push(("layer.select".into(), json!({"layer": l.id.0})));
                    json!({"layer": l.id.0, "color": color.id()})
                };
                actions.push(("layer.setLabelColor".into(), params));
                ui.close();
            }
        }
    });
}

fn color_button(ui: &mut egui::Ui, color: LabelColor, checked: bool) -> egui::Response {
    use egui::{Atom, Rect, Stroke, StrokeKind, vec2};
    let t = crate::theme::Tokens::get(ui.ctx());
    let check_id = egui::IdSalt::new(("label-check", color.id()));
    let swatch_id = egui::IdSalt::new(("label-swatch", color.id()));
    let name = color_name(color);
    let button = egui::Button::new((Atom::custom(check_id, vec2(14.0, 16.0)), Atom::custom(swatch_id, vec2(16.0, 16.0)), name, Atom::grow())).atom_ui(ui);
    if checked && let Some(rect) = button.rect(check_id) {
        crate::icons::paint(ui, rect, "check", 14.0, t.icon);
    }
    if let Some(rect) = button.rect(swatch_id) {
        let rect = Rect::from_center_size(rect.center(), vec2(16.0, 16.0));
        let p = ui.painter();
        if let Some((swatch, _)) = t.layer_label_colors(color) {
            p.rect_filled(rect, t.radius_sm, swatch);
        } else {
            let stroke = Stroke::new(1.0, t.text_dim);
            p.rect_stroke(rect, t.radius_sm, stroke, StrokeKind::Inside);
            let r = rect.shrink(4.0);
            p.line_segment([r.left_top(), r.right_bottom()], stroke);
            p.line_segment([r.right_top(), r.left_bottom()], stroke);
        }
    }
    button.response.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, ui.is_enabled(), checked, name));
    button.response
}

#[cfg(test)]
mod color_tests;

#[cfg(test)]
mod tests;
