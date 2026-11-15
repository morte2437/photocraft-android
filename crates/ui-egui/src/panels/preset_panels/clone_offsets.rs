//! Read-only clone geometry shared by the panel, overlay and source marker.

use photocraft_engine::presets::clone_source::{Mapping, resolve};

use super::{PhotocraftApp, Tool, Value, json};

/// Remember a document-space preview position when the pointer is over the active canvas.
/// Retain it while the pointer enters the panel so editing X/Y uses the readout's reference.
pub(crate) fn remember_clone_cursor(app: &mut PhotocraftApp, at: [f64; 2]) -> bool {
    if !matches!(app.ui.tool, Tool::CloneStamp | Tool::Healing) || !at.into_iter().all(|v| v.is_finite() && v.abs() < 1_000_000.0) {
        return false;
    }
    if let Some(document) = app.session.active().map(|st| st.doc.id) {
        let next = Some((document, at));
        if app.ui.presets_ui.clone_cursor != next {
            app.ui.presets_ui.clone_cursor = next;
            return true;
        }
    }
    false
}

/// Resolve the exact sampling geometry without storing a painting anchor or changing a source.
pub fn clone_mapping_at(app: &PhotocraftApp, at: Option<[f64; 2]>) -> Option<Mapping> {
    let slot = app.session.presets.clone.active();
    let source = slot.source?;
    let start = app.drag.as_ref().filter(|d| matches!(d.tool, Tool::CloneStamp | Tool::Healing)).map(|d| d.start);
    let first = start.or(at).or(slot.last_anchor).or(slot.anchor).unwrap_or(source);
    let params = json!({"aligned": app.ui.tool_options.clone_aligned});
    let (map, _) = resolve(&app.session, &params, (first[0], first[1]), "clone.preview").ok()?;
    [map.source.0, map.source.1, map.anchor.0, map.anchor.1].into_iter().chain(map.m).all(|v| v.is_finite() && v.abs() < 1_000_000.0).then_some(map)
}

/// Pairing currently previewed in this document, or the most recently committed pairing.
fn panel_mapping(app: &PhotocraftApp) -> Option<Mapping> {
    let cursor = app
        .ui
        .presets_ui
        .clone_cursor
        .filter(|(id, _)| app.session.active().is_some_and(|st| st.doc.id == *id))
        .filter(|_| matches!(app.ui.tool, Tool::CloneStamp | Tool::Healing))
        .map(|(_, at)| at);
    clone_mapping_at(app, cursor)
}

/// Source-minus-destination displacement in document pixels, independent of canvas zoom.
pub(crate) fn clone_offset_readout(app: &PhotocraftApp) -> Option<[f64; 2]> {
    let map = panel_mapping(app)?;
    Some([map.source.0 - map.anchor.0, map.source.1 - map.anchor.1])
}

/// Edit exactly the reference shown in X/Y, including before the first paint or with Aligned off.
pub(crate) fn clone_offset_edit(app: &PhotocraftApp, offset: [f64; 2]) -> Option<Value> {
    let map = panel_mapping(app)?;
    Some(json!({"offset": offset, "anchor": [map.anchor.0, map.anchor.1]}))
}

/// Where the source marker samples at a canvas point, using the overlay's mapping.
pub fn clone_sample_point(app: &PhotocraftApp, at: Option<[f64; 2]>) -> Option<[f64; 2]> {
    let map = clone_mapping_at(app, at)?;
    let (x, y) = at.map_or(map.source, |at| map.map(at[0], at[1]));
    Some([x, y])
}

#[cfg(test)]
mod tests;
