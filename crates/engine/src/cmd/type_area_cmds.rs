//! Area type (#218): type that wraps inside a closed path, like Photoshop's Type tool clicked
//! inside a closed path or shape. `type.detachFromPath` (type_extra_cmds.rs) releases it.

use photocraft_doc::text::TextShape;
use photocraft_doc::vector::{Knot, Path};
use photocraft_doc::{Document, LayerContent, LayerId};
use photocraft_geom::Point;
use serde_json::{Value, json};

use crate::commands::{CommandSpec, layer_param};
use crate::type_extra_cmds::with_text;
use crate::{EngineError, Result, Session};

const CMD: &str = "type.fillPath";

// Largest total anchor count of a path area type can fill. Within it every path fits the area
// edge cap, so a big curved path gets "too many anchor points" rather than "encloses no area".
use photocraft_text::area::MAX_AREA_KNOTS;

fn bad(m: &str) -> EngineError {
    EngineError::BadParams { cmd: CMD.into(), msg: m.into() }
}

fn has_text(s: &Session) -> std::result::Result<(), String> {
    let d = s.active().ok_or("no document open")?;
    match d.active_layer.and_then(|id| d.doc.layer(id)).map(|l| &l.content) {
        Some(LayerContent::Text(_)) => Ok(()),
        _ => Err("the active layer is not a type layer".into()),
    }
}

/// The path area type fills, in document space: the work path (default) or a shape layer's
/// path (`"shape":id`). Every closed subpath counts, with its path operation.
fn area_source(doc: &Document, p: &Value) -> Result<Path> {
    let path = match p.get("shape") {
        None | Some(Value::Null) => doc.work_path.as_ref().ok_or_else(|| bad("the document has no work path; draw a closed path or pass \"shape\":layerId"))?,
        Some(v) => {
            let id = v.as_u64().ok_or_else(|| bad("\"shape\" must be a layer id"))?;
            match doc.layer(LayerId(id)).map(|l| &l.content) {
                Some(LayerContent::Shape(sh)) => &sh.path,
                Some(_) => return Err(bad("\"shape\" is not a shape layer")),
                None => return Err(EngineError::NoLayer(LayerId(id))),
            }
        }
    };
    let knots = path.subpaths.iter().fold(0usize, |n, s| n.saturating_add(s.knots.len()));
    if knots > MAX_AREA_KNOTS {
        return Err(bad("the path has too many anchor points"));
    }
    if path.inverted {
        return Err(bad("an inverted path has no inside to fill"));
    }
    Ok(path.clone())
}

fn fill_path(s: &mut Session, p: &Value) -> Result<Value> {
    let id = layer_param(s, p)?;
    let doc = &s.active().ok_or(EngineError::NoDocument)?.doc;
    // `with_text` refuses a locked layer.
    let source = area_source(doc, p)?;
    let n = source.subpaths.len();
    with_text(s, p, "Area Type", |t, _| {
        // The path is in document space; the layer keeps it in text space so the type lands
        // inside the path where it is drawn.
        let inv = t.transform.inverse().ok_or_else(|| EngineError::Other("the type layer's transform cannot be inverted".into()))?;
        let map = |q: Point| inv.apply(q);
        let mut path = source.clone();
        for k in path.subpaths.iter_mut().flat_map(|s| s.knots.iter_mut()) {
            *k = Knot { anchor: map(k.anchor), in_ctrl: map(k.in_ctrl), out_ctrl: map(k.out_ctrl), smooth: k.smooth };
        }
        let vertical = t.orientation == photocraft_doc::text::Orientation::Vertical;
        if photocraft_text::area::AreaShape::new(&path, vertical).is_none() {
            return Err(bad("the path encloses no area; draw a closed path"));
        }
        t.shape = TextShape::Area { path };
        Ok(())
    })?;
    Ok(json!({"layer": id.0, "subpaths": n}))
}

pub fn specs() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: CMD,
        label: "Area Type",
        menu: &[],
        shortcut: None,
        params: r##"{"layer":id? (default active Type layer),"shape":layerId? (a shape layer; default the work path)} → the text wraps inside the path's closed subpaths, holes included"##,
        enabled: has_text,
        journal: true,
        run: fill_path,
    }]
}

#[cfg(test)]
#[path = "type_area_cmds/tests.rs"]
mod tests;
