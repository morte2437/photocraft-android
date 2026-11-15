//! The menu a right-click on the empty area below the last layer row opens (#11404): the
//! thumbnail size and what a thumbnail frames. Both are the settings Panel Options... edits.
//! A right-click on a layer's thumbnail opens the same menu under Select Pixels (#12324).

use serde_json::json;

use crate::PhotocraftApp;
use crate::state::LayerThumbnailSize;

/// The four sizes, in menu order.
pub(super) fn sizes() -> [(LayerThumbnailSize, &'static str); 4] {
    [
        (LayerThumbnailSize::None, tl!("No Thumbnails")),
        (LayerThumbnailSize::Small, tl!("Small Thumbnails")),
        (LayerThumbnailSize::Medium, tl!("Medium Thumbnails")),
        (LayerThumbnailSize::Large, tl!("Large Thumbnails")),
    ]
}

/// The layer thumbnail's menu: Select Pixels loads the layer's transparency as the selection,
/// as a ⌘-click on the thumbnail does, then the thumbnail settings.
pub(super) fn show_thumbnail(app: &mut PhotocraftApp, ui: &mut egui::Ui, layer: u64, actions: &mut Vec<(String, serde_json::Value)>) {
    if ui.button(tl!("Select Pixels")).clicked() {
        actions.push(("select.loadSelection".into(), json!({"channel": "transparency", "layer": layer, "operation": "new"})));
        ui.close();
    }
    ui.separator();
    show(app, ui);
}

pub(super) fn show(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let current = app.ui.layers_panel_options.thumbnail_size;
    for (size, label) in sizes() {
        let mut on = current == size;
        if crate::widgets::checkbox(ui, &mut on, label).clicked() {
            app.ui.layers_panel_options.thumbnail_size = size;
            super::layers_panel_ui::persist_options(app);
            ui.close();
        }
    }
    ui.separator();
    let clip = app.session.prefs().interface.clip_thumbnails_to_layer_bounds;
    for (to_layer, label) in [(true, tl!("Clip Thumbnails to Layer Bounds")), (false, tl!("Clip Thumbnails to Document Bounds"))] {
        let mut on = clip == to_layer;
        if crate::widgets::checkbox(ui, &mut on, label).clicked() {
            if clip != to_layer
                && let Err(e) = app.run("prefs.set", json!({"path": "interface.clipThumbnailsToLayerBounds", "value": to_layer}))
            {
                app.ui.status = e;
                app.ui.status_error = true;
            }
            ui.close();
        }
    }
}
