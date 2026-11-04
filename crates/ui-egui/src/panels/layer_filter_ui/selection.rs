//! #10879: applying the Layers panel filter, or changing its criteria while it filters, deselects
//! the selected layers it no longer lists, as in Photoshop. The listed ones stay selected (one of
//! them becomes the active layer when the active one is hidden), and turning the filter off changes
//! nothing. Like any layer selection this is no history step. A layer selected later, while the
//! filter hides it, stays selected: the filter only acts when it changes.

use photocraft_doc::LayerId;
use serde_json::json;

use super::Filter;
use crate::PhotocraftApp;

/// Deselect the layers the filter hides when its criteria changed since the last call. The
/// criteria are kept in `ctx`'s memory, so a change from the filter row or from `ui.set
/// {layerFilter}` is seen the next time the row is drawn; with nothing kept yet (the row's first
/// draw, e.g. after `ui.set` while the panel was closed) a filter that is on counts as applied.
pub fn follow(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let id = egui::Id::unique("layer-filter-applied");
    let now = Filter::of(&app.ui);
    let before = ctx.data_mut(|d| {
        let before = d.get_temp::<Filter>(id);
        d.insert_temp(id, now.clone());
        before
    });
    if before.is_none_or(|b| b != now) && now.active() {
        deselect_hidden(app);
    }
}

/// Deselect the active document's selected layers that [`Filter::of_app`] doesn't list: through
/// `select.deselectLayers` when none stays listed, else by keeping only the listed ones.
pub fn deselect_hidden(app: &mut PhotocraftApp) {
    let f = Filter::of_app(app);
    if !f.active() {
        return;
    }
    let Some(st) = app.session.active() else { return };
    let selected = st.selected_layers();
    let listed: Vec<LayerId> = selected.iter().copied().filter(|id| st.doc.layer(*id).is_some_and(|l| f.shows(l))).collect();
    if listed.len() == selected.len() && st.active_layer.is_none_or(|a| listed.contains(&a)) {
        return;
    }
    if listed.is_empty() {
        let _ = app.run("select.deselectLayers", json!({}));
        return;
    }
    let active = st.active_layer.filter(|a| listed.contains(a)).or(listed.last().copied());
    let anchor = st.layer_anchor.filter(|a| listed.contains(a)).or(active);
    let _ = photocraft_engine::layer_multi_cmds::set_selection(&mut app.session, listed, active, anchor);
}
