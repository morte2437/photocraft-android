//! Channels: alpha and spot channels, Select › Save / Load Selection, Quick Mask, channel
//! targeting and visibility (the Channels panel), Image › Apply Image / Calculations, and the
//! Channels panel menu (Split / Merge Channels, Merge Spot Channel).
//!
//! Alpha channels live in [`Document::channels`] (white = selected). Quick Mask is a temporary
//! channel in [`Document::quick_mask`], so entering and leaving it are history steps like in
//! Photoshop. Which channel is targeted and which are visible is view state on
//! [`DocState::channel_view`] (not history), like the active layer.
//!
//! Pixel commands (`paint.*`, `filter.*`, `image.adjustments.*`, `edit.fill`) take
//! `"target": {"channel": i}` or `"target": "quickMask"`; [`Session::execute`] fills that in from the
//! targeted channel when the caller didn't pass a target. Targeting a single colour channel keeps
//! those commands from touching the other colour channels of the layer ([`restrict_to_color`]).

use photocraft_algo::selection::{self as sel, SelectionMode};
use photocraft_color::{BlendMode, Color, ColorMode, PixelFormat, blend::blend_channel};
use photocraft_doc::{AlphaChannel, ColorIndicates, Document, Layer, LayerContent, LayerId};
use photocraft_geom::Rect;
use photocraft_raster::{Surface, from_rgba, to_rgba};
use serde::Serialize;
use serde_json::{Value, json};

use crate::commands::CommandSpec;
use crate::{DocState, EngineError, Result, Session};

#[cfg(test)]
mod tests;

// ---------------------------------------------------------------------------------------------
// Enabled predicates

fn has_doc(s: &Session) -> std::result::Result<(), String> {
    s.active().map(|_| ()).ok_or_else(|| "no document open".into())
}
fn has_channels(s: &Session) -> std::result::Result<(), String> {
    has_doc(s)?;
    if s.active().is_some_and(|d| !d.doc.channels.is_empty()) { Ok(()) } else { Err("the document has no alpha channels".into()) }
}
fn has_selection(s: &Session) -> std::result::Result<(), String> {
    has_doc(s)?;
    if s.active().is_some_and(|d| d.doc.selection.is_some()) { Ok(()) } else { Err("no selection".into()) }
}
fn has_spot(s: &Session) -> std::result::Result<(), String> {
    has_doc(s)?;
    if s.active().is_some_and(|d| d.doc.channels.iter().any(|c| c.spot.is_some())) { Ok(()) } else { Err("the document has no spot channels".into()) }
}
fn can_split(s: &Session) -> std::result::Result<(), String> {
    let d = s.active().ok_or("no document open")?;
    if color_count(&d.doc) + d.doc.channels.len() < 2 { Err("only one channel to split".into()) } else { Ok(()) }
}
fn can_merge(s: &Session) -> std::result::Result<(), String> {
    has_doc(s)?;
    if mergeable(s).len() >= 2 { Ok(()) } else { Err("Merge Channels needs at least two open grayscale documents of the same size".into()) }
}

// ---------------------------------------------------------------------------------------------
// Save / Load Selection

fn operation<'a>(p: &'a Value, key: &str) -> &'a str {
    p.get(key).and_then(Value::as_str).unwrap_or("new")
}

// ---------------------------------------------------------------------------------------------
// State as JSON

/// The Channels panel rows and view state of a document (also part of `inspect`).
pub fn channels_json(st: &DocState) -> Value {
    let doc = &st.doc;
    let mode = edit_mode(doc);
    let v = &st.channel_view;
    let colors = color_count(doc);
    json!({
        "composite": composite_name(mode),
        "colors": color_names(mode).iter().enumerate().map(|(k, n)| json!({ "name": n, "visible": v.color_visible(k) })).collect::<Vec<_>>(),
        "alpha": doc.channels.iter().enumerate().map(|(i, c)| json!({
            "index": i,
            "name": c.name,
            "visible": v.alpha_shown(i),
            "spot": c.spot.map(|(col, sol)| json!({ "color": col.to_rgb(), "solidity": sol })),
            "color": c.color.to_rgb(),
            "opacity": c.opacity,
            "indicates": c.indicates,
        })).collect::<Vec<_>>(),
        "quickMask": doc.quick_mask.as_ref().map(|q| json!({ "visible": !v.quick_mask_hidden, "color": q.color.to_rgb(), "opacity": q.opacity, "indicates": q.indicates })),
        "target": v.target,
        "selectedColors": v.color_selection(colors).map(|c| c.indices().collect::<Vec<_>>()),
        "compositeVisible": v.visible_colors(colors) == colors,
        "layerMaskView": crate::mask_view_cmds::view_json(st),
    })
}

fn list(s: &Session) -> Value {
    s.active().map(channels_json).unwrap_or(Value::Null)
}

// ---------------------------------------------------------------------------------------------
// Registry

macro_rules! spec {
    ($id:literal, $label:literal, [$($m:literal),*], $sc:expr, $params:literal, $en:expr, $run:expr) => {
        CommandSpec { id: $id, label: $label, menu: &[$($m),*], shortcut: $sc, params: $params, enabled: $en, run: $run, journal: true }
    };
}

mod calc;
mod manage;
mod planes;
mod refs;
mod selection;
mod spot;
mod targeting;
mod view;

use calc::{apply_image, calculations};
use manage::{delete_channel, duplicate_channel, move_channel, new_channel, new_spot, options, rename_channel, set_visible};
use planes::edit_mode;
pub(crate) use planes::tone;
pub use planes::{
    GraySurface, IsolatedCurveChannel, channel_format, color_count, curve_channel_for_colors, filter_mask_curve_channel, isolated_curve_channel,
    isolated_curve_channel_for, layer_mask_curve_channel, next_alpha_name,
};
pub(crate) use refs::source_spec;
pub use refs::{Blending, ChanRef, parse_ref};
use selection::{load_selection, quick_mask, save_selection};
use spot::{merge, merge_spot, mergeable, split};
use targeting::{SLOT_KEY, target_cmd, target_slot};
pub(crate) use targeting::{
    Target, channel_surface_for_filter, clear_channel, clear_color_channel, color_restriction, edits_channel, filter_mask_layer, inject_target, invert_region,
    is_channel_target, is_spot_target, mask_target_enabled, restrict_to_color, routed, target_of, target_surface, targeted_channel,
};
pub use targeting::{follows_filter_mask, follows_target, read_target_surface};
pub(crate) use view::fix_view;
pub use view::{ChannelTarget, ChannelView, QuickMaskOptions, color_names, composite_name};

pub fn specs() -> Vec<CommandSpec> {
    vec![
        spec!(
            "select.saveSelection",
            "Save Selection…",
            ["Select"],
            None,
            r##"{"name":str?,"channel":"new"|index|name="new","document":index?,"operation":"new|replace|add|subtract|intersect"="new"}"##,
            has_selection,
            save_selection
        ),
        spec!(
            "select.loadSelection",
            "Load Selection…",
            ["Select"],
            None,
            r##"{"channel":index|name|"composite"|"red|green|blue|…"|"transparency"|"mask"|"vectorMask"|"quickMask"|"selection"|"targeted" (the Channels panel's target),"layer":id?,"document":index?,"invert":bool=false,"operation":"new|add|subtract|intersect"="new"}"##,
            has_doc,
            load_selection
        ),
        spec!("select.editInQuickMaskMode", "Edit in Quick Mask Mode", ["Select"], Some("Q"), r##"{"on":bool?=toggle}"##, has_doc, quick_mask),
        spec!(
            "image.applyImage",
            "Apply Image…",
            ["Image"],
            None,
            r##"{"source":{"document":index?,"layer":id|name|"merged"="merged","channel":"composite"|"red|…"|index|"selection"|"transparency"|"mask"="composite","invert":bool=false},"blending":"normal|multiply|screen|overlay|softLight|hardLight|colorDodge|colorBurn|darken|lighten|difference|exclusion|linearBurn|linearDodge|add|subtract|…"="multiply","opacity":0..100=100,"scale":1..2=1,"offset":-255..255=0,"preserveTransparency":bool=false,"mask":{"document","layer","channel","invert"}?,"sourceChannel|sourceDocument|sourceLayer|sourceInvert|maskChannel|…":flat form of source/mask?}"##,
            has_apply_target,
            apply_image
        ),
        spec!(
            "image.calculations",
            "Calculations…",
            ["Image"],
            None,
            r##"{"source1":{"document","layer","channel","invert"},"source2":{…},"blending":"multiply|…"="multiply","opacity":0..100=100,"scale":1..2=1,"offset":-255..255=0,"mask":{…}?,"result":"newChannel|newDocument|selection"="newChannel","name":str?}"##,
            has_doc,
            calculations
        ),
        spec!(
            "channel.new",
            "New Channel…",
            [],
            None,
            r##"{"name":str?,"fill":"black|white|selection"="black","color":"#rrggbb"="#ff0000","opacity":0..100=50,"indicates":"masked|selected"="masked"}"##,
            has_doc,
            new_channel
        ),
        spec!(
            "channel.newSpot",
            "New Spot Channel…",
            [],
            None,
            r##"{"name":str?,"color":"#rrggbb"="#ff0000","solidity":0..100=0,"fromSelection":bool=true}"##,
            has_doc,
            new_spot
        ),
        spec!(
            "channel.duplicate",
            "Duplicate Channel…",
            [],
            None,
            r##"{"channel":index|name|"red|…"|"composite"|"quickMask","name":str?,"document":index|"new"?=active,"invert":bool=false}"##,
            has_doc,
            duplicate_channel
        ),
        spec!("channel.delete", "Delete Channel", [], None, r##"{"channel":index|name?=targeted}"##, has_channels, delete_channel),
        spec!("channel.rename", "Rename Channel", [], None, r##"{"channel":index|name,"name":str}"##, has_channels, rename_channel),
        spec!("channel.move", "Reorder Channel", [], None, r##"{"channel":index|name,"to":index}"##, has_channels, move_channel),
        spec!(
            "channel.target",
            "Target Channel",
            [],
            None,
            r##"{"channel":"composite"|"mask"|"vectorMask"|"red|green|blue|…"|index|name="composite","colors":[index,…]?,"extend":bool=false} (colors selects native colour components only; extend toggles one colour channel, as Shift-click; all components select the composite)"##,
            has_doc,
            target_cmd
        ),
        spec!(
            "channel.setVisible",
            "Channel Visibility",
            [],
            None,
            r##"{"channel":"composite"|"red|…"|index|name|"quickMask","visible":bool?=toggle}"##,
            has_doc,
            set_visible
        ),
        spec!(
            "channel.options",
            "Channel Options…",
            [],
            None,
            r##"{"channel":index|name|"quickMask"|"mask","name":str?,"indicates":"masked|selected|spot"?,"color":"#rrggbb"?,"opacity":0..100?,"solidity":0..100?} ("mask": Layer Mask Display Options, the rubylith's color and opacity for every layer mask, kept in the preferences as layerMaskOverlay; not an undo step; returns them as layerMaskOverlay)"##,
            has_doc,
            options
        ),
        spec!("channel.mergeSpot", "Merge Spot Channel", [], None, r##"{"channel":index|name?=targeted or first spot}"##, has_spot, merge_spot),
        spec!("channel.split", "Split Channels", [], None, r##"{"closeOriginal":bool=true}"##, can_split, split),
        spec!(
            "channel.merge",
            "Merge Channels…",
            [],
            None,
            r##"{"mode":"rgb|cmyk|lab"="rgb","documents":[index,…]?=open grayscale docs of the active size,"name":str?}"##,
            can_merge,
            merge
        ),
        CommandSpec {
            id: "channel.list",
            label: "Channels",
            menu: &[],
            shortcut: None,
            params: "{}",
            enabled: has_doc,
            run: |s, _| Ok(list(s)),
            journal: false,
        },
        spec!("channel.target.composite", "Target Composite Channel", [], Some("Cmd+2"), "{}", has_doc, |s, _| target_slot(s, 2)),
        // ⌘\ targets the active layer's mask, as clicking its thumbnail does.
        spec!("channel.target.layerMask", "Target Layer Mask", [], Some("Cmd+\\"), "{}", has_layer_mask, |s, _| target_cmd(
            s,
            &serde_json::json!({"channel": "mask"})
        )),
        // ⌘⌥2 loads the image's luminosity as a selection (⌘-click on the composite channel).
        spec!("select.loadLuminosity", "Load Luminosity", [], Some("Cmd+Alt+2"), "{}", has_doc, |s, _| load_selection(
            s,
            &serde_json::json!({"channel": "composite"})
        )),
        spec!("channel.target.slot3", "Target Channel 3", [], Some("Cmd+3"), "{}", slot_ok::<3>, |s, _| target_slot(s, 3)),
        spec!("channel.target.slot4", "Target Channel 4", [], Some("Cmd+4"), "{}", slot_ok::<4>, |s, _| target_slot(s, 4)),
        spec!("channel.target.slot5", "Target Channel 5", [], Some("Cmd+5"), "{}", slot_ok::<5>, |s, _| target_slot(s, 5)),
        spec!("channel.target.slot6", "Target Channel 6", [], Some("Cmd+6"), "{}", slot_ok::<6>, |s, _| target_slot(s, 6)),
        spec!("channel.target.slot7", "Target Channel 7", [], Some("Cmd+7"), "{}", slot_ok::<7>, |s, _| target_slot(s, 7)),
        spec!("channel.target.slot8", "Target Channel 8", [], Some("Cmd+8"), "{}", slot_ok::<8>, |s, _| target_slot(s, 8)),
        spec!("channel.target.slot9", "Target Channel 9", [], Some("Cmd+9"), "{}", slot_ok::<9>, |s, _| target_slot(s, 9)),
    ]
}

/// Target Layer Mask: the active layer has a mask.
fn has_layer_mask(s: &Session) -> std::result::Result<(), String> {
    let st = s.active().ok_or("no document open")?;
    let layer = st.active_layer.and_then(|id| st.doc.layer(id)).ok_or("no active layer")?;
    layer.mask.as_ref().map(|_| ()).ok_or_else(|| "the active layer has no layer mask".to_string())
}

fn slot_ok<const N: usize>(s: &Session) -> std::result::Result<(), String> {
    let d = s.active().ok_or("no document open")?;
    let colors = color_count(&d.doc);
    let shown = if colors > 1 { colors } else { 0 };
    if N - 3 < shown + d.doc.channels.len() { Ok(()) } else { Err(format!("no channel for {SLOT_KEY}{N}")) }
}

fn has_apply_target(s: &Session) -> std::result::Result<(), String> {
    if edits_channel(s) {
        return Ok(());
    }
    let d = s.active().ok_or("no document open")?;
    let l = d.active_layer.and_then(|id| d.doc.layer(id)).ok_or("no active layer")?;
    if matches!(l.content, LayerContent::Raster(_)) {
        Ok(())
    } else {
        Err(format!("Apply Image needs a pixel layer (active layer is {} {} layer)", l.content.article(), l.content.kind_name()))
    }
}
