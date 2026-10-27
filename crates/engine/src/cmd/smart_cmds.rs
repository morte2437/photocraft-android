//! Smart Objects and Smart Filters: Layer › Smart Objects, Layer › Smart Filter and
//! Filter › Convert for Smart Filters.
//!
//! A smart object keeps its **source** (embedded file bytes, or a linked path) plus a
//! `transform` and a stack of smart filters; its `cache` is the rendered appearance the
//! compositors draw. The engine owns re-rendering (compose stays a pure function of the
//! document):
//!
//! ```text
//! source bytes ──decode + composite (cached by content hash)──► source image (source pixels)
//!              ──transform (exact shift, or bicubic warp)──────► placed (document pixels)
//!              ──each visible smart filter, blended (blend/opacity)──► filtered
//!              ──filter mask mix (unfiltered ↔ filtered)───────► cache
//! ```
//!
//! The placed stage and the result after each filter are cached too (`stack_cache`), so a
//! re-render starts from the deepest stage it shares with an earlier one.
//!
//! Converting a layer embeds it as an in-memory `.pcraft` bundle of a nested document (exactly
//! lossless, layered, any depth/model). PSD placed layers keep their source in the preserved
//! global `lnk2` block (found by uuid), and keep Photoshop's rendering until something changes,
//! so an unedited PSD still round-trips byte for byte. PSD export (`photocraft_io::smart_map`)
//! writes every smart object back as one: its source embedded in `lnk2` (a `.pcraft` source as a
//! PSB), its smart filters in `filterFX` and its filter mask in `FEid`.

use std::sync::{Arc, Mutex};

use photocraft_algo::resample::translate_surface;
use photocraft_algo::transform::Homography;
use photocraft_color::{BlendMode, ColorMode, PixelFormat, SampleType};
use photocraft_doc::{DocId, Document, Layer, LayerContent, LayerId, LayerMask, Metadata, SmartContentsId, SmartFilter, SmartObject, SmartSource};
use photocraft_geom::{Affine, Rect, Size};
use photocraft_raster::Surface;
use serde_json::{Value, json};
use stack_cache::{Stage, StageKey};

use crate::commands::{CommandSpec, blend_from_str, layer_param};
use crate::jobs::JobCtx;
use crate::{EngineError, Result, Session};

/// An open Edit Contents document and the shared contents it updates when saved or closed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SmartLink {
    pub child: DocId,
    pub parent: DocId,
    pub layer: LayerId,
    pub contents_id: SmartContentsId,
}

/// PSD placed-layer keys that describe the smart object's source; stale once we change it.
const PLACED_KEYS: [&[u8; 4]; 3] = [b"SoLd", b"PlLd", b"SoLE"];

/// Command id of a Photoshop smart filter PhotoCraft does not implement (kept verbatim from PSD).
pub use photocraft_io::smart_map::UNSUPPORTED_FILTER;

fn other(msg: impl Into<String>) -> EngineError {
    EngineError::Other(msg.into())
}

fn bad(cmd: &str, msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: cmd.into(), msg: msg.into() }
}

// ---------- source encoding / decoding ----------

/// Encodes a document as embedded smart-object contents: an in-memory `.pcraft` bundle.
pub fn encode_source(doc: &Document) -> Result<Vec<u8>> {
    photocraft_format::save_to_bytes(doc, &Default::default()).map_err(|e| other(format!("can't encode smart object contents: {e}")))
}

/// Decodes smart-object contents (a `.pcraft` bundle, PSD/PSB, or any flat image format).
pub fn decode_source(file_name: &str, bytes: &[u8]) -> Result<Document> {
    crate::file_cmds::import(file_name, bytes).map_err(|e| other(format!("can't read smart object contents \"{file_name}\": {e}")))
}

fn base_name(path: &str) -> String {
    path.rsplit(['/', '\\']).next().unwrap_or(path).to_string()
}

// ---------- enablement ----------

fn active_smart(s: &Session) -> std::result::Result<&SmartObject, String> {
    let d = s.active().ok_or("no document open")?;
    let id = d.active_layer.ok_or("no active layer")?;
    match &d.doc.layer(id).ok_or("no active layer")?.content {
        LayerContent::Smart(sm) => Ok(sm),
        other => Err(format!("the active layer is {} {} layer, not a smart object", other.article(), other.kind_name())),
    }
}
fn has_smart(s: &Session) -> std::result::Result<(), String> {
    active_smart(s).map(|_| ())
}
fn has_linked(s: &Session) -> std::result::Result<(), String> {
    match active_smart(s)?.source {
        SmartSource::Linked { .. } => Ok(()),
        _ => Err("the smart object is embedded".into()),
    }
}
fn has_doc(s: &Session) -> std::result::Result<(), String> {
    s.active().map(|_| ()).ok_or_else(|| "no document open".into())
}
fn has_smart_filters(s: &Session) -> std::result::Result<(), String> {
    if active_smart(s)?.smart_filters.is_empty() { Err("the smart object has no smart filters".into()) } else { Ok(()) }
}
fn has_filter_mask(s: &Session) -> std::result::Result<(), String> {
    active_smart(s)?.filter_mask.as_ref().map(|_| ()).ok_or_else(|| "the smart object has no filter mask".into())
}
fn convertible(s: &Session) -> std::result::Result<(), String> {
    let d = s.active().ok_or("no document open")?;
    let id = d.active_layer.ok_or("no active layer")?;
    d.doc.layer(id).map(|_| ()).ok_or_else(|| "no active layer".into())
}
fn not_smart(s: &Session) -> std::result::Result<(), String> {
    convertible(s)?;
    if active_smart(s).is_ok() { Err("the layer is already a smart object".into()) } else { Ok(()) }
}

/// Convert to Smart Object: refused only when the single targeted layer is already a smart
/// object. A multi-layer selection can still be combined (nesting any smart object in it).
fn convertible_to_smart(s: &Session) -> std::result::Result<(), String> {
    convertible(s)?;
    if crate::layer_multi_cmds::selected(s).len() <= 1 { not_smart(s) } else { Ok(()) }
}

macro_rules! spec {
    ($id:literal, $label:literal, $menu:expr, $params:literal, $en:expr, $run:expr) => {
        CommandSpec { id: $id, label: $label, menu: $menu, shortcut: None, params: $params, enabled: $en, run: $run, journal: true }
    };
}

mod contents;
mod filters;
mod placing;
mod rendering;
mod source;

pub(crate) use contents::on_close;
pub use contents::{commit_child, contents_of, replace_contents_bytes, replace_contents_from_bytes};
use contents::{edit_contents, export_contents, is_smart_child, path_param, replace_contents, set_source, update_all};
#[cfg(test)]
use filters::smart_mut;
pub(crate) use filters::{add_smart_filter, filter_mask_paintable, filter_mask_surface, refresh_after_filter_mask_edit};
use filters::{blending_options, delete_filter, edit_filters, move_filter, set_filter_params, set_filter_visible, smart, transfer_filters};
use placing::{convert, detach_psd, shift_layer, via_copy};
pub use placing::{layer_to_smart, layers_to_smart, placement};
pub(crate) use placing::{set_placement, shift_smart, transform_placement};
use rendering::refresh_or_fail;
pub(crate) use rendering::source_box;
pub use rendering::{apply_smart_filters, purge_stack_cache, refresh, refresh_layer, render, render_below_filter, stack_cache_bytes, stack_image};
#[cfg(test)]
use rendering::{blend_surfaces, place, render_in};
pub use source::{SourceImage, source_bytes, source_decode_count, source_image, svg_source_image};
use source::{authorize_source, authorized_source_bytes};
pub(crate) use source::{restore_unrendered, take_caches};

const SO: &[&str] = &["Layer", "Smart Objects"];
const SF: &[&str] = &["Layer", "Smart Filter"];

pub fn specs() -> Vec<CommandSpec> {
    vec![
        spec!(
            "layer.smartObjects.convertToSmartObject",
            "Convert to Smart Object",
            SO,
            r##"{"layer":id?} (default: combine selected layers)"##,
            convertible_to_smart,
            convert
        ),
        spec!(
            "filter.convertForSmartFilters",
            "Convert for Smart Filters",
            &["Filter"],
            r##"{"layer":id?} (default: combine selected layers)"##,
            not_smart,
            convert
        ),
        spec!("layer.smartObjects.newSmartObjectViaCopy", "New Smart Object via Copy", SO, r##"{"layer":id?}"##, has_smart, via_copy),
        spec!("layer.smartObjects.rasterize", "Rasterize", SO, r##"{"layer":id?}"##, has_smart, |s, p| s.execute("layer.rasterize.smartObject", p.clone())),
        spec!(
            "layer.smartObjects.editContents",
            "Edit Contents",
            SO,
            r##"{"layer":id?} → opens the contents as a new document; saving (layer.smartObjects.saveContents) or closing it updates the smart object"##,
            has_smart,
            edit_contents
        ),
        spec!("layer.smartObjects.saveContents", "Save Contents", &[], "{} (in an Edit Contents document)", is_smart_child, |s, _| {
            let i = s.active_index().ok_or(EngineError::NoDocument)?;
            Ok(json!({"updated": commit_child(s, i)?}))
        }),
        spec!("layer.smartObjects.replaceContents", "Replace Contents…", SO, r##"{"layer":id?,"path":str}"##, has_smart, replace_contents),
        spec!(
            "layer.smartObjects.exportContents",
            "Export Contents…",
            SO,
            r##"{"layer":id?,"path":str (another extension than the contents' own exports them in that format)}"##,
            has_smart,
            export_contents
        ),
        spec!("layer.smartObjects.relinkToFile", "Relink to File…", SO, r##"{"layer":id?,"path":str}"##, has_smart, |s, p| {
            let path = path_param("layer.smartObjects.relinkToFile", p)?.to_string();
            set_source(s, p, "Relink to File", false, |_, _| Ok(SmartSource::Linked { path }))
        }),
        spec!("layer.smartObjects.updateModifiedContent", "Update Modified Content", SO, r##"{"layer":id?}"##, has_linked, |s, p| {
            let id = layer_param(s, p)?;
            s.edit("Update Modified Content", |doc, _| refresh_or_fail(doc, id))?;
            Ok(json!({"layer": id.0}))
        }),
        spec!(
            "layer.smartObjects.updateChanged",
            "Update Changed Linked Content",
            &[],
            "{} — re-render the active document's linked smart objects whose files changed on disk since they were last seen (size or modification time); the desktop app does this by itself every two seconds, as Photoshop does while a document is open. Files seen for the first time are only remembered. One Update Modified Content undo step per file → {updated: [layer ids]}",
            |s| s.active().map(|_| ()).ok_or_else(|| "no document open".into()),
            crate::link_watch::update_changed
        ),
        spec!(
            "layer.smartObjects.updateAllModifiedContent",
            "Update All Modified Content",
            SO,
            "{}",
            |s| s.active().map(|_| ()).ok_or_else(|| "no document open".into()),
            |s, _| update_all(s)
        ),
        spec!(
            "layer.smartObjects.convertToLayers",
            "Convert to Layers",
            SO,
            r##"{"layer":id?} (contents unpacked at the placement: one layer, or a group named after the smart object; smart filters are discarded)"##,
            has_smart,
            unpack::convert_to_layers
        ),
        spec!("layer.smartObjects.convertToEmbedded", "Convert to Embedded", SO, r##"{"layer":id?}"##, has_linked, |s, p| {
            set_source(s, p, "Convert to Embedded", true, |meta, src| {
                let (file_name, bytes) = source_bytes(meta, src).ok_or_else(|| other("the linked file can't be read"))?;
                Ok(SmartSource::Embedded { file_name, bytes })
            })
        }),
        spec!(
            "layer.smartObjects.convertToLinked",
            "Convert to Linked…",
            SO,
            r##"{"layer":id?,"path":str} (writes the contents there)"##,
            has_smart,
            |s, p| {
                let path = path_param("layer.smartObjects.convertToLinked", p)?.to_string();
                set_source(s, p, "Convert to Linked", false, |meta, src| {
                    let (_, bytes) = source_bytes(meta, src).ok_or_else(|| other("the smart object's contents are unavailable"))?;
                    crate::file_cmds::write_file(&path, &bytes)?;
                    Ok(SmartSource::Linked { path })
                })
            }
        ),
        // Smart filters
        spec!(
            "layer.smartFilter.disableSmartFilters",
            "Disable Smart Filters",
            SF,
            r##"{"layer":id?,"enabled":bool? (default: toggle)}"##,
            has_smart_filters,
            |s, p| {
                edit_filters(s, p, "Disable Smart Filters", |sm| {
                    sm.filters_enabled = p.get("enabled").and_then(Value::as_bool).unwrap_or(!sm.filters_enabled);
                    Ok(())
                })
            }
        ),
        spec!("layer.smartFilter.deleteFilterMask", "Delete Filter Mask", SF, r##"{"layer":id?}"##, has_filter_mask, |s, p| {
            edit_filters(s, p, "Delete Filter Mask", |sm| {
                sm.filter_mask = None;
                Ok(())
            })
        }),
        spec!(
            "layer.smartFilter.disableFilterMask",
            "Disable Filter Mask",
            SF,
            r##"{"layer":id?,"enabled":bool? (default: toggle)}"##,
            has_filter_mask,
            |s, p| {
                edit_filters(s, p, "Disable Filter Mask", |sm| {
                    if let Some(m) = &mut sm.filter_mask {
                        m.enabled = p.get("enabled").and_then(Value::as_bool).unwrap_or(!m.enabled);
                    }
                    Ok(())
                })
            }
        ),
        spec!(
            "layer.smartFilter.blendingOptions",
            "Blending Options…",
            SF,
            r##"{"layer":id?,"index":u32? (0 = bottom; default top),"blend":str?,"opacity":0..1?}"##,
            has_smart_filters,
            blending_options
        ),
        spec!("layer.smartFilter.clearSmartFilters", "Clear Smart Filters", SF, r##"{"layer":id?}"##, has_smart_filters, |s, p| {
            edit_filters(s, p, "Clear Smart Filters", |sm| {
                sm.smart_filters.clear();
                sm.filter_mask = None;
                Ok(())
            })
        }),
        spec!(
            "layer.smartFilter.setVisible",
            "Show/Hide Smart Filter",
            &[],
            r##"{"layer":id?,"index":u32?,"visible":bool? (default: toggle)}"##,
            has_smart_filters,
            set_filter_visible
        ),
        spec!(
            "layer.smartFilter.setParams",
            "Edit Smart Filter",
            &[],
            r##"{"layer":id?,"index":u32?,"params":{…} (merged)}"##,
            has_smart_filters,
            set_filter_params
        ),
        spec!("layer.smartFilter.delete", "Delete Smart Filter", &[], r##"{"layer":id?,"index":u32?}"##, has_smart_filters, delete_filter),
        spec!("layer.smartFilter.move", "Move Smart Filter", &[], r##"{"layer":id?,"index":u32?,"to":u32}"##, has_smart_filters, move_filter),
        spec!(
            "layer.smartFilter.transfer",
            "Move or Copy Smart Filters",
            &[],
            r##"{"from":id,"to":id,"filter":u32? (default: all, with the filter mask),"copy":bool=false}"##,
            has_doc,
            transfer_filters
        ),
    ]
}

mod stack_cache;
mod unpack;

#[cfg(test)]
mod tests;
