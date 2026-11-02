//! A layer's own XMP packet, as Photoshop scripts reach it through `layer.xmpMetadata.rawData`:
//! scripts and embedding apps keep data about a layer there under their own namespaces. It is
//! saved in PSD, PSB, layered TIFF and `.pcraft`, and copied when the layer is duplicated.

use std::sync::Arc;

use photocraft_doc::LayerId;
use serde_json::{Map, Value, json};

use crate::commands::CommandSpec;
use crate::{EngineError, Result, Session};

const ID: &str = "layer.xmpMetadata";

/// Largest packet accepted, in UTF-8 bytes. Every history state shares the packet, but a PSD
/// stores it as UTF-16 in each layer's record.
pub const MAX_LAYER_XMP: usize = 4 << 20;

fn bad(message: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: ID.into(), msg: message.into() }
}

fn enabled(s: &Session) -> std::result::Result<(), String> {
    crate::active_layer_of(s).map(|_| ())
}

/// `raw` if it is an XMP packet: well-formed XML (no DTD) whose root is `x:xmpmeta` or
/// `rdf:RDF`, without NUL characters (Photoshop stores the packet NUL-terminated).
fn check_packet(raw: &str) -> Result<()> {
    if raw.len() > MAX_LAYER_XMP {
        return Err(bad(format!("`rawData` is {} bytes; the most a layer holds is {MAX_LAYER_XMP}", raw.len())));
    }
    if raw.contains('\0') {
        return Err(bad("`rawData` can't contain NUL characters"));
    }
    let xml = photocraft_io::xml::parse(raw, roxmltree::ParsingOptions::default()).map_err(|e| bad(format!("`rawData` isn't well-formed XML: {e}")))?;
    let root = xml.root_element().tag_name();
    let xmp = matches!((root.namespace(), root.name()), (Some("adobe:ns:meta/"), "xmpmeta") | (Some("http://www.w3.org/1999/02/22-rdf-syntax-ns#"), "RDF"));
    if !xmp {
        return Err(bad("`rawData` must be an XMP packet: its root element is x:xmpmeta or rdf:RDF"));
    }
    Ok(())
}

fn xmp_metadata(s: &mut Session, p: &Value) -> Result<Value> {
    let empty = Map::new();
    let p = match p {
        Value::Null => &empty,
        Value::Object(m) => m,
        _ => return Err(bad("expected an object")),
    };
    let st = s.active().ok_or(EngineError::NoDocument)?;
    let id = match p.get("layer") {
        Some(v) => LayerId(v.as_u64().ok_or_else(|| bad("`layer` must be an unsigned integer"))?),
        None => st.active_layer.ok_or_else(|| bad("no active layer"))?,
    };
    let current = st.doc.layer(id).ok_or(EngineError::NoLayer(id))?.xmp.clone();
    let new = match p.get("rawData") {
        None => return Ok(json!({"layer": id.0, "rawData": current.as_deref(), "changed": false})),
        Some(Value::Null) => None,
        Some(Value::String(raw)) => {
            check_packet(raw)?;
            Some(raw.as_str())
        }
        Some(_) => return Err(bad("`rawData` must be a string (an XMP packet) or null (remove it)")),
    };
    let changed = current.as_deref() != new;
    if changed {
        let stored: Option<Arc<str>> = new.map(Into::into);
        s.edit("Layer Metadata", move |doc, _| {
            doc.layer_mut(id).ok_or(EngineError::NoLayer(id))?.xmp = stored;
            Ok(())
        })?;
        if let Some(st) = s.active_mut() {
            // Saved metadata only; no canvas pixels need recompositing.
            st.last_damage = Some(photocraft_geom::Rect::EMPTY);
        }
    }
    Ok(json!({"layer": id.0, "rawData": new, "changed": changed}))
}

pub fn specs() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: ID,
        label: "Layer XMP Metadata",
        menu: &[],
        shortcut: None,
        params: r#"{"layer":id?,"rawData":string|null?} → {"layer","rawData","changed"} (no layer: the active layer; no rawData: read it; a string sets the layer's XMP packet, null removes it; as Photoshop's layer.xmpMetadata.rawData)"#,
        enabled,
        run: xmp_metadata,
        journal: true,
    }]
}

#[cfg(test)]
mod tests;
