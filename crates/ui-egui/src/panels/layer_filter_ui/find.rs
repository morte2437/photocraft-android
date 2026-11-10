//! #10884: Select › Find Layers (⌥⇧⌘F) filters the Layers panel by Name, as in Photoshop: it shows
//! the Layers panel, switches the filter to Name and puts the caret in its field with the text
//! selected, so typing replaces it. The layer selection is not changed. With a `name` param
//! (scripts and agents) it sets the Name text to it instead of focusing the field.

use serde_json::{Value, json};

use super::FilterBy;
use crate::{PhotocraftApp, widgets};

/// The egui memory flag: focus the Name field the next time it is drawn.
fn focus_flag() -> egui::Id {
    egui::Id::unique("layer-filter-find-focus")
}

/// Run `select.findLayers`: the engine checks `params` and lists the layers the Name text matches;
/// then the Name filter is applied. Without a `name` the text is kept, and the Layers panel is
/// shown with the field focused.
pub fn invoke(app: &mut PhotocraftApp, ctx: &egui::Context, params: Value) -> Result<Value, String> {
    let given = params.get("name").is_some_and(|n| !n.is_null());
    // `run` keeps the Name text when no `name` is given ([`params_for_run`]) and applies the
    // reply's Name filter ([`apply`]).
    let reply = app.run("select.findLayers", params)?;
    if !given {
        // The panel docking layout (when on) shows the Layers panel itself, else the dock group.
        if app.ui.docking.is_some() {
            if let Some(shown) = crate::panel_docking::command(app, "window.panel.activate", &json!({"panel": "layers"})) {
                shown?;
            }
        } else {
            *crate::dock::Group::Layers.tab_mut(&mut app.ui.dock_tabs) = 0;
            crate::dock::reveal(app, crate::dock::Group::Layers);
        }
        // A hidden filter row (Filter Options › Hide) stays hidden, as in Photoshop, and showing it
        // later doesn't put the caret in the field: only a drawn row takes the focus.
        if app.ui.layers_panel_options.show_filters {
            ctx.data_mut(|d| d.insert_temp(focus_flag(), true));
        }
    }
    Ok(reply)
}

/// The params `select.findLayers` runs with: without a `name` the panel's Name text is kept, so the
/// reply lists the layers the panel lists however the command runs (the menu, the shortcut,
/// `engine.execute` or an action step).
pub(crate) fn params_for_run(app: &PhotocraftApp, params: Value) -> Value {
    if params.get("name").is_some_and(|n| !n.is_null()) {
        return params;
    }
    let mut params = if params.is_object() { params } else { json!({}) };
    params["name"] = json!(app.ui.layer_filter_name);
    params
}

/// Apply the Name filter in a `select.findLayers` reply to the Layers panel: switch to Name and set
/// the text when the reply has one. [`PhotocraftApp::run`] calls it after every Find Layers.
pub(crate) fn apply(app: &mut PhotocraftApp, reply: &Value) {
    app.ui.layer_filter_by = FilterBy::Name;
    if let Some(name) = reply["layerFilter"]["name"].as_str() {
        app.ui.layer_filter_name = name.to_string();
    }
}

/// The Name filter's text field, taking the focus with its text selected after [`invoke`]. The
/// focus waits for a frame without a pointer press or release: egui drops the focus at the end of
/// a frame in which the pointer was clicked elsewhere, as it is when the menu item is clicked.
pub fn name_field(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let id = ui.scope_id().with("layer-filter-name");
    let pending = ui.ctx().data(|d| d.get_temp::<bool>(focus_flag())).unwrap_or(false);
    let clicking = ui.input(|i| i.pointer.any_pressed() || i.pointer.any_released() || i.pointer.any_down());
    if pending && clicking {
        ui.ctx().request_repaint();
    } else if pending {
        ui.ctx().data_mut(|d| d.remove_temp::<bool>(focus_flag()));
        ui.memory_mut(|m| m.request_focus(id));
        widgets::field_tab::select_all(ui.ctx(), id, &app.ui.layer_filter_name);
    }
    let width = ui.available_width().max(48.0);
    widgets::text_field(ui, id, &mut app.ui.layer_filter_name, tl!("Search"), width, false);
}
