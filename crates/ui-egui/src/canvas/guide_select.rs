//! Selecting a guide on the canvas, as in Photoshop: a click on a guide with the Move tool selects
//! it (drawn in Photoshop's selected-guide blue), a press anywhere else deselects it, Delete
//! removes it, and right-click opens the guide menu (`canvas_tool_menu`), whose Edit Selected
//! Guides reopens New Guide on it.
//!
//! One guide at a time. The selection is view state, not a history step: it names the guide by
//! document, orientation and position, as guide colours do, so it lapses by itself when the guide
//! moves away, is deleted or undone, and the index is looked up when it is used.

use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::PhotocraftApp;

/// Photoshop draws a selected guide in this blue, whatever the guide's own colour.
pub const SELECTED_COLOR: egui::Color32 = egui::Color32::from_rgb(0x00, 0x00, 0xff);

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct SelectedGuide {
    /// The document's index among the open documents.
    pub doc: usize,
    pub vertical: bool,
    pub position: f64,
}

/// The selected guide's orientation and index in the active document, while guides can be
/// interacted with (shown, unlocked).
pub fn selected(app: &PhotocraftApp) -> Option<(bool, usize)> {
    let sel = app.ui.selected_guide?;
    if !interactive(app) || app.session.active_index() != Some(sel.doc) {
        return None;
    }
    let guides = &app.session.active()?.doc.guides;
    let list = if sel.vertical { &guides.vertical } else { &guides.horizontal };
    list.iter().position(|p| *p == sel.position).map(|i| (sel.vertical, i))
}

/// Is the guide at `position` the selected one (drawn blue)?
pub fn is_selected(app: &PhotocraftApp, vertical: bool, position: f64) -> bool {
    selected(app).is_some() && app.ui.selected_guide.is_some_and(|s| s.vertical == vertical && s.position == position)
}

/// Guides take clicks only while they're shown and unlocked (as `rulers::guide_at` requires).
fn interactive(app: &PhotocraftApp) -> bool {
    app.ui.extras.guides && app.ui.view.shows(app.ui.view.show.canvas_guides) && !app.ui.extras.lock_guides
}

/// Select guide `index` of the active document.
pub fn select(app: &mut PhotocraftApp, vertical: bool, index: usize) {
    let Some(doc) = app.session.active_index() else { return };
    let Some(guides) = app.session.active().map(|st| &st.doc.guides) else { return };
    let list = if vertical { &guides.vertical } else { &guides.horizontal };
    app.ui.selected_guide = list.get(index).map(|position| SelectedGuide { doc, vertical, position: *position });
}

pub fn clear(app: &mut PhotocraftApp) {
    app.ui.selected_guide = None;
}

/// Delete the selected guide (Delete, Clear Selected Guides). False when no guide is selected.
pub fn delete_selected(app: &mut PhotocraftApp) -> Result<bool, String> {
    let Some((vertical, index)) = selected(app) else { return Ok(false) };
    app.run("view.deleteGuide", delete_params(vertical, index))?;
    clear(app);
    Ok(true)
}

pub fn delete_params(vertical: bool, index: usize) -> serde_json::Value {
    json!({"orientation": if vertical { "vertical" } else { "horizontal" }, "index": index})
}

/// ⌥-click on guide `index`: turn it, so a vertical guide becomes horizontal through the point
/// clicked (`at`, document pixels) and the other way round, as one undoable step (`view.editGuide`);
/// it stays selected.
pub fn flip(app: &mut PhotocraftApp, vertical: bool, index: usize, at: [f64; 2]) {
    let (from, to) = if vertical { ("vertical", "horizontal") } else { ("horizontal", "vertical") };
    let position = if vertical { at[1] } else { at[0] };
    let params = json!({"orientation": from, "index": index, "newOrientation": to, "position": (position * 1000.0).round() / 1000.0});
    match app.run("view.editGuide", params) {
        Ok(_) => {
            let last = app.session.active().map(|st| if vertical { st.doc.guides.horizontal.len() } else { st.doc.guides.vertical.len() });
            if let Some(n) = last.and_then(|n| n.checked_sub(1)) {
                select(app, !vertical, n);
            }
        }
        Err(e) => app.ui.status = e,
    }
}

/// Delete or Backspace with no modifiers while a guide is selected deletes the guide, before Edit ›
/// Clear can reach the pixels, as in Photoshop.
pub fn keys(app: &mut PhotocraftApp, ctx: &egui::Context) -> bool {
    use crate::shortcut_dispatch::Focus;
    // Text fields and widgets that delete with these keys themselves keep them (#1534).
    if selected(app).is_none() || app.ui.text_edit.is_some() || matches!(Focus::of(ctx), Focus::Text | Focus::DeletingWidget) {
        return false;
    }
    let mods = ctx.input(|i| i.modifiers);
    if !mods.is_none() || !ctx.input_mut(|i| i.consume_key(mods, egui::Key::Delete) || i.consume_key(mods, egui::Key::Backspace)) {
        return false;
    }
    if let Err(e) = delete_selected(app) {
        app.ui.status = e;
        app.ui.status_error = true;
    }
    true
}

#[cfg(test)]
mod tests;
