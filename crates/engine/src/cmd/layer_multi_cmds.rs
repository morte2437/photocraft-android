//! Multi-layer selection and the commands that act on it: ⌘/⇧-click selection, Select › All /
//! Deselect / Find Layers, Layer › Align and Distribute, Link Layers, Merge Layers, Group from
//! Layers, Arrange › Reverse, Lock Layers and Rename Layer.
//!
//! The selection itself lives in [`DocState::selected_layers`] (UI state, like the active layer):
//! changing it never adds a history step. Every structural change below is one history step.

use photocraft_color::PixelFormat;
use photocraft_doc::{Document, Layer, LayerContent, LayerId, VideoData};
use photocraft_geom::Rect;
use serde_json::{Value, json};

use crate::commands::{CommandSpec, int_i32};
use crate::{DocState, EngineError, Result, Session};

mod align;
mod damage;
mod moves;
mod selection;
mod structure;

use align::{align, distribute};
pub use damage::{layer_bounds, layers_damage, same_pixels, selection_only_change, step_damage};
pub(crate) use damage::{note_damage, note_inert_insert, note_insert};
use moves::link_groups;
pub(crate) use moves::shift_surface;
#[cfg(test)]
pub(crate) use moves::with_links;
pub use moves::{move_targets, move_to, moved, moved_target, moves_mask_alone, preview_move, translate};
use selection::{doc_state, find_layers, select_all_layers, select_linked};
pub(crate) use selection::{mask_targets, recorded_step, reselect, targets, top_level};
pub use selection::{select, selected, set_selection};
pub(crate) use structure::{check_group_depth, deletion_neighbours};
pub use structure::{delete_selected, duplicate_selected, group_layers, set_visible_selected};
use structure::{link_layers, lock_layers, merge_layers, rename_layer, reverse};

// ---------- enabled predicates ----------

fn has_layer(s: &Session) -> std::result::Result<(), String> {
    if selected(s).is_empty() { Err("no layer selected".into()) } else { Ok(()) }
}
fn has_doc(s: &Session) -> std::result::Result<(), String> {
    doc_state(s).map(|_| ())
}
fn two_plus(s: &Session) -> std::result::Result<(), String> {
    if selected(s).len() >= 2 { Ok(()) } else { Err("select two or more layers".into()) }
}
fn can_align(s: &Session) -> std::result::Result<(), String> {
    let d = doc_state(s)?;
    match d.selected_layers().len() {
        0 => Err("no layer selected".into()),
        // Photoshop aligns to an active selection however many layers are selected; without one,
        // a single layer aligns to the canvas and 2+ align to each other.
        _ => Ok(()),
    }
}
fn can_distribute(s: &Session) -> std::result::Result<(), String> {
    if selected(s).len() >= 3 { Ok(()) } else { Err("select three or more layers to distribute".into()) }
}
fn can_link(s: &Session) -> std::result::Result<(), String> {
    let d = doc_state(s)?;
    let sel = d.selected_layers();
    match sel.len() {
        0 => Err("no layer selected".into()),
        1 if d.doc.layer(sel[0]).is_none_or(|l| l.link_group.is_none()) => Err("select two or more layers to link".into()),
        _ => Ok(()),
    }
}
fn has_linked(s: &Session) -> std::result::Result<(), String> {
    let d = doc_state(s)?;
    if link_groups(&d.doc, &d.selected_layers()).is_empty() { Err("the selected layers are not linked".into()) } else { Ok(()) }
}

fn bad(cmd: &str, msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: cmd.into(), msg: msg.into() }
}

/// Is more than one layer selected (so single-layer commands should act on the whole set)?
pub fn multi(s: &Session, p: &Value) -> bool {
    p.get("layer").is_none() && selected(s).len() > 1
}

// ---------- registry ----------

pub fn specs() -> Vec<CommandSpec> {
    macro_rules! spec {
        ($id:expr, $label:expr, $menu:expr, $sc:expr, $params:expr, $enabled:expr, $run:expr) => {
            CommandSpec { id: $id, label: $label, menu: $menu, shortcut: $sc, params: $params, enabled: $enabled, journal: true, run: $run }
        };
    }
    const ALIGN: &str = r##"{"to":"auto|layers|selection|canvas"="auto"} (auto: the active selection's bounds when there is one, however many layers are selected; else one layer aligns to the canvas and several to their combined bounds)"##;
    vec![
        spec!("select.allLayers", "All Layers", &["Select"], Some("Cmd+Alt+A"), "{}", has_doc, |s, _| select_all_layers(s)),
        spec!("select.deselectLayers", "Deselect Layers", &["Select"], None, "{}", has_doc, |s, _| {
            set_selection(s, Vec::new(), None, None)?;
            if let Some(st) = s.active_mut() {
                st.active_layer = None;
                st.selected_layers.clear();
            }
            Ok(Value::Null)
        }),
        spec!("select.findLayers", "Find Layers", &["Select"], Some("Cmd+Alt+Shift+F"), r##"{"name":str (case-insensitive substring)}"##, has_doc, find_layers),
        spec!("layer.selectLinkedLayers", "Select Linked Layers", &["Layer"], None, "{}", has_linked, |s, _| select_linked(s)),
        spec!("layer.align.topEdges", "Top Edges", &["Layer", "Align"], None, ALIGN, can_align, |s, p| align(s, p, "topEdges")),
        spec!("layer.align.verticalCenters", "Vertical Centers", &["Layer", "Align"], None, ALIGN, can_align, |s, p| align(s, p, "verticalCenters")),
        spec!("layer.align.bottomEdges", "Bottom Edges", &["Layer", "Align"], None, ALIGN, can_align, |s, p| align(s, p, "bottomEdges")),
        spec!("layer.align.leftEdges", "Left Edges", &["Layer", "Align"], None, ALIGN, can_align, |s, p| align(s, p, "leftEdges")),
        spec!("layer.align.horizontalCenters", "Horizontal Centers", &["Layer", "Align"], None, ALIGN, can_align, |s, p| align(s, p, "horizontalCenters")),
        spec!("layer.align.rightEdges", "Right Edges", &["Layer", "Align"], None, ALIGN, can_align, |s, p| align(s, p, "rightEdges")),
        spec!("layer.distribute.topEdges", "Top Edges", &["Layer", "Distribute"], None, "{}", can_distribute, |s, _| distribute(s, "topEdges")),
        spec!("layer.distribute.verticalCenters", "Vertical Centers", &["Layer", "Distribute"], None, "{}", can_distribute, |s, _| distribute(
            s,
            "verticalCenters"
        )),
        spec!("layer.distribute.bottomEdges", "Bottom Edges", &["Layer", "Distribute"], None, "{}", can_distribute, |s, _| distribute(s, "bottomEdges")),
        spec!("layer.distribute.leftEdges", "Left Edges", &["Layer", "Distribute"], None, "{}", can_distribute, |s, _| distribute(s, "leftEdges")),
        spec!("layer.distribute.horizontalCenters", "Horizontal Centers", &["Layer", "Distribute"], None, "{}", can_distribute, |s, _| distribute(
            s,
            "horizontalCenters"
        )),
        spec!("layer.distribute.rightEdges", "Right Edges", &["Layer", "Distribute"], None, "{}", can_distribute, |s, _| distribute(s, "rightEdges")),
        spec!(
            "layer.distribute.horizontally",
            "Horizontally",
            &["Layer", "Distribute"],
            None,
            "{} (equal horizontal gaps)",
            can_distribute,
            |s, _| distribute(s, "horizontally")
        ),
        spec!("layer.distribute.vertically", "Vertically", &["Layer", "Distribute"], None, "{} (equal vertical gaps)", can_distribute, |s, _| distribute(
            s,
            "vertically"
        )),
        spec!("layer.linkLayers", "Link Layers", &["Layer"], None, "{} (toggles: unlinks when the selection is already one link group)", can_link, |s, _| {
            link_layers(s)
        }),
        spec!("layer.mergeLayers", "Merge Layers", &["Layer"], Some("Cmd+E"), "{} (one layer selected: Merge Down)", has_layer, |s, _| merge_layers(s)),
        spec!(
            "layer.new.groupFromLayers",
            "Group from Layers…",
            &["Layer", "New"],
            None,
            r##"{"layer":id?,"name":text?,"color":"none|red|orange|yellow|green|seafoam|blue|indigo|magenta|fuchsia|violet|gray"="none","mode":str="passThrough" (blend mode),"opacity":0..100=100 (percent)} (no layer: every selected layer; the new group starts collapsed)"##,
            has_layer,
            group_layers
        ),
        spec!("layer.arrange.reverse", "Reverse", &["Layer", "Arrange"], None, "{}", two_plus, |s, _| reverse(s)),
        spec!(
            "layer.lockLayers",
            "Lock Layers…",
            &["Layer"],
            None,
            r##"{"transparency":bool?,"pixels":bool?,"position":bool?,"artboard":bool?,"all":bool?} (none given: toggle lock all)"##,
            has_layer,
            lock_layers
        ),
        spec!("layer.renameLayer", "Rename Layer", &["Layer"], None, r##"{"layer":id?,"name":str}"##, has_layer, rename_layer),
    ]
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod layers_panel_parity_tests;
