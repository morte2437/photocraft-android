//! Type tool: click to add point text, drag to add paragraph text, edit inline with a caret.
//!
//! Every edit is an engine `type.*` command carrying the session's `coalesce` key, so a whole
//! typing session is one "Edit Type" history step and automation sees exactly what the user does.
//! Offsets in [`TextEdit`] are character indices (the engine's unit); the layout works in bytes.

use std::collections::HashMap;
use std::ops::Range;
use std::sync::{Arc, PoisonError};

use egui::ime_context::{CommittedText, ImeContextTracker, ImeMutationStamp, ImeObservation, ImeTarget};
use egui::{Color32, Pos2, Stroke};
use photocraft_doc::{Document, LayerContent, LayerId, TextLayer};
use photocraft_geom::{Affine, Point};
use photocraft_text::TextLayout;
use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::ViewXform;
use crate::state::{BoxResize, TextEdit};

mod defaults;
mod editing;
mod fonts;
mod ime;
mod ime_edit;
mod keys;
mod layout;
mod options_bar;
mod overlay;
mod panel;
mod pointer;

#[cfg(test)]
use defaults::{apply, default_style_params, default_styles, shown_scale};
pub use defaults::{foreground_changed, open_color_picker};
pub(crate) use defaults::{formatting_params, target, target_text, type_unit};
#[cfg(test)]
pub(crate) use editing::{CharToggle, step_char_metric, toggle_char_style, toggle_hyphenate};
pub use editing::{cancel, commit, commit_with, edit_active, edit_at, end_if_left};
pub(crate) use editing::{copy_selection, cut_selection, request_paste, select_all};
#[cfg(test)]
use editing::{insert, insert_typed, smart_quotes};
#[cfg(test)]
pub(crate) use fonts::{Axis, axis_value, slider_axes};
#[cfg(test)]
use fonts::{FONT_MENU_HEIGHT, font_labels_id, font_menu_rows, font_picker_in, preview_text, style_names, wheel_family_index};
pub(crate) use fonts::{FontLibrary, family_picker_in, fonts_ready, selected_style};
pub use fonts::{families, style_label, styles, sync_font_names, warm_fonts};
pub(crate) use ime::{ImeState, controls_only, finish_reset_logic, finish_reset_pass, in_ui_pass, install_reset_hooks};
#[cfg(test)]
use ime::{IncomingComposition, ResetPhase};
#[cfg(test)]
use ime_edit::ime_update;
pub(crate) use ime_edit::sync_ime;
#[cfg(test)]
pub(crate) use keys::flow_key;
pub use keys::handle_keys;
pub use layout::layout;
pub(crate) use layout::{hit_layer, offset_at, shows_own_layout};
#[cfg(test)]
use options_bar::num_field;
pub use options_bar::options_bar;
pub(crate) use options_bar::styles_at;
#[cfg(test)]
pub(crate) use options_bar::{kerning_label, parse_kerning};
#[cfg(test)]
use overlay::CARET_STROKES;
pub use overlay::draw_overlay;
#[cfg(test)]
pub(crate) use panel::{OPENTYPE_BUTTONS, toggle_opentype};
pub use panel::{character_panel, type_properties};
#[cfg(test)]
use pointer::{MIN_BOX, handle_sides, word_boundary};
pub(crate) use pointer::{PathClick, box_handle_at, hover_path_target, inside_box};
#[cfg(test)]
pub(crate) use pointer::{box_shape, click_selection, next_click_count, path_target};
pub use pointer::{multi_click, pointer_down, pointer_move, pointer_up};

pub const PLACEHOLDER: &str = "Lorem Ipsum";

fn text_layer(doc: &Document, id: LayerId) -> Option<&TextLayer> {
    match &doc.layer(id)?.content {
        LayerContent::Text(t) => Some(t),
        _ => None,
    }
}

fn byte_of(text: &str, ci: usize) -> usize {
    photocraft_text::byte_index(text, ci)
}

fn hex(c: [f32; 4]) -> String {
    let b = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    format!("#{:02x}{:02x}{:02x}", b(c[0]), b(c[1]), b(c[2]))
}

fn session_key(app: &mut PhotocraftApp) -> String {
    format!("type-{}", app.ui.alloc_id())
}

fn active_doc_id(app: &PhotocraftApp) -> Option<u64> {
    app.session.active().map(|st| st.doc.id.0)
}

fn current_text(app: &PhotocraftApp, id: LayerId) -> Option<String> {
    Some(text_layer(&app.session.active()?.doc, id)?.text.clone())
}

/// Character count without cloning the layer's text.
fn text_char_count(app: &PhotocraftApp, id: LayerId) -> Option<usize> {
    Some(text_layer(&app.session.active()?.doc, id)?.text.chars().count())
}

/// The selected characters, clamped after undo/redo may have shortened the text.
pub(crate) fn selected_range(app: &PhotocraftApp) -> Option<[usize; 2]> {
    let ed = app.ui.text_edit.as_ref()?;
    let n = text_layer(&app.session.active()?.doc, LayerId(ed.layer))?.text.chars().count();
    let (a, b) = (ed.caret.min(ed.anchor).min(n), ed.caret.max(ed.anchor).min(n));
    (a < b).then_some([a, b])
}

/// The byte range of characters `a..b` of `text`.
fn byte_range(text: &str, a: usize, b: usize) -> (usize, usize) {
    let at = |c: usize| text.char_indices().nth(c).map_or(text.len(), |(i, _)| i);
    (at(a.min(b)), at(a.max(b)))
}

#[cfg(test)]
mod canvas_tests;

#[cfg(test)]
mod font_picker_tests;

#[cfg(test)]
mod tests;

#[cfg(test)]
mod commit_tests;

#[cfg(test)]
mod ime_tests;
