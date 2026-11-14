//! `type.createSelection`: the Horizontal and Vertical Type Mask tools' result. Typed text
//! becomes a selection in the shape of its glyphs (their anti-aliased coverage) instead of a
//! type layer, as Photoshop's Type Mask tools do.
//!
//! The tools edit a temporary type layer in one coalesced history step and commit with
//! `{"layer": id, "removeLayer": true}`, so the whole session is one undo step that leaves no
//! layer behind. Scripts can skip the layer and pass `type.create`'s keys directly.

use photocraft_algo::selection::{self as sel, SelectionMode};
use photocraft_doc::text::AntiAlias;
use photocraft_doc::{Document, LayerContent, TextLayer};
use photocraft_geom::Rect;
use serde_json::{Value, json};

use crate::commands::CommandSpec;
use crate::{EngineError, Result, Session};

const CMD: &str = "type.createSelection";
/// What an allocation failure names: the buffers are no longer bounded by the canvas.
const PURPOSE: &str = "the Type Mask selection";

fn bad(msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: CMD.into(), msg: msg.into() }
}

fn has_doc(s: &Session) -> std::result::Result<(), String> {
    s.active().map(|_| ()).ok_or_else(|| "no document open".into())
}

fn antialias(p: &Value) -> Result<Option<AntiAlias>> {
    Ok(match p.get("antialias") {
        None => None,
        Some(v) => Some(match v.as_str() {
            Some("none") => AntiAlias::None,
            Some("sharp") => AntiAlias::Sharp,
            Some("crisp") => AntiAlias::Crisp,
            Some("strong") => AntiAlias::Strong,
            Some("smooth") => AntiAlias::Smooth,
            _ => return Err(bad("antialias must be none, sharp, crisp, strong or smooth")),
        }),
    })
}

/// Glyph coverage of `t` (0..=1 per pixel) over its rendered bounds, past the canvas edge too: the
/// reference application keeps a Type Mask's off-canvas part (#12074). The text colour's own alpha
/// doesn't thin the selection: every run renders opaque.
fn coverage(doc: &Document, t: &TextLayer) -> Result<(Vec<f32>, Rect)> {
    let mut t = t.clone();
    for r in &mut t.runs {
        r.style.color.alpha = 1.0;
    }
    let mut eng = photocraft_text::shared().lock().unwrap_or_else(|e| e.into_inner());
    let surface = eng.render(&t, doc.resolution_dpi, doc.pixel_format()).1.surface;
    let area = surface.content_bounds();
    if area.is_empty() {
        return Ok((Vec::new(), Rect::EMPTY));
    }
    // No longer bounded by the canvas: a very large text must fail as an error, not abort.
    let len = (area.width() as usize).checked_mul(area.height() as usize).ok_or_else(|| crate::allocation::error(PURPOSE, "requested size overflowed"))?;
    let mut px = crate::allocation::filled(len, [0.0f32; 4], PURPOSE)?;
    surface.read_rgba_into(area, &mut px);
    let mut alpha = crate::allocation::capacity(len, PURPOSE)?;
    alpha.extend(px.iter().map(|p| p[3]));
    Ok((alpha, area))
}

fn create_selection(s: &mut Session, p: &Value) -> Result<Value> {
    let mode = match p.get("operation").map(|v| v.as_str()) {
        None | Some(Some("new" | "replace")) => SelectionMode::Replace,
        Some(Some("add")) => SelectionMode::Add,
        Some(Some("subtract")) => SelectionMode::Subtract,
        Some(Some("intersect")) => SelectionMode::Intersect,
        _ => return Err(bad("operation must be new, add, subtract or intersect")),
    };
    let aa = antialias(p)?;
    let remove = match p.get("removeLayer") {
        None => false,
        Some(v) => v.as_bool().ok_or_else(|| bad("removeLayer must be true or false"))?,
    };
    let doc = s.active().ok_or(EngineError::NoDocument)?.doc.clone();
    let layer = crate::commands::explicit_layer(s, p)?;
    if remove && layer.is_none() {
        return Err(bad("removeLayer needs a layer"));
    }
    let mut t = match layer {
        Some(id) => match &doc.layer(id).ok_or(EngineError::NoLayer(id))?.content {
            LayerContent::Text(t) => t.clone(),
            _ => return Err(bad(format!("layer {} is not a type layer", id.0))),
        },
        None => crate::type_cmds::new_text_layer(s, p, CMD)?,
    };
    if let Some(aa) = aa {
        t.antialias = aa;
    }
    let (mask, text_area) = coverage(&doc, &t)?;
    // `combine_band` builds the new selection over the text's band where running out of memory
    // aborts, and the band is no longer bounded by the canvas: check first that it fits, so a text
    // too big for it is an error that changes nothing. Per pixel that is the new 8-bit grey tiles.
    // With an unusual earlier selection (not 8-bit grey) that the mode reads, it keeps its format
    // through more: the combined grey tiles and their float copy (1 + 4), the copy widened to every
    // channel and its growth while collected (8 per channel), and its own new tiles.
    let unusual = doc.selection.as_ref().filter(|o| mode != SelectionMode::Replace && o.format() != photocraft_color::PixelFormat::GRAY8);
    let per_pixel = unusual.map_or(1, |old| 5 + 8 * old.channels() + old.format().bytes_per_pixel());
    let bytes = (text_area.width() as usize).checked_mul(text_area.height() as usize).and_then(|n| n.checked_mul(per_pixel));
    let bytes = bytes.ok_or_else(|| crate::allocation::error(PURPOSE, "requested size overflowed"))?;
    drop(crate::allocation::capacity::<u8>(bytes, PURPOSE)?);
    let selected = s.edit("Type Mask", |doc, active| {
        // Replace and Intersect leave nothing outside the text; Add and Subtract keep the rest of
        // the selection as it is, its part past the canvas too, without a buffer spanning both.
        doc.selection = sel::combine_band(doc.selection.as_ref(), &mask, text_area, mode);
        if let (true, Some(id)) = (remove, layer) {
            let neighbours = crate::layer_multi_cmds::deletion_neighbours(doc, id);
            doc.remove(id).ok_or(EngineError::NoLayer(id))?;
            if doc.layers.is_empty() {
                return Err(EngineError::Other("a document must keep at least one layer".into()));
            }
            if active.is_some_and(|current| doc.layer(current).is_none()) {
                *active = neighbours.into_iter().find(|candidate| doc.layer(*candidate).is_some());
            }
        }
        Ok(doc.selection.as_ref().map(|m| m.content_bounds()))
    })?;
    // The Type Mask tools' whole typing session is this one step.
    if let Some(st) = s.active_mut() {
        st.history.set_current_label("Type Mask");
    }
    Ok(json!({ "selected": selected.is_some(), "bounds": selected.map(|r| [r.x0, r.y0, r.x1, r.y1]) }))
}

pub fn specs() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: CMD,
        label: "Type Mask",
        menu: &[],
        shortcut: None,
        params: r##"{"layer":id|name? (a type layer whose glyphs become the selection),"removeLayer":bool=false (delete that layer in the same step: the Type Mask tools' temporary text),"operation":"new|add|subtract|intersect"="new","antialias":"none|sharp|crisp|strong|smooth"?, …without "layer", type.create's keys: "text","x","y" or "box","orientation","font","size","align",…}"##,
        enabled: has_doc,
        journal: true,
        run: create_selection,
    }]
}

#[cfg(test)]
#[path = "type_mask_off_canvas_tests.rs"]
mod off_canvas_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_doc::LayerId;

    fn session() -> Session {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 300, "height": 300})).unwrap();
        s
    }

    fn bounds(s: &Session) -> Option<Rect> {
        s.active().unwrap().doc.selection.as_ref().map(|m| m.content_bounds())
    }

    #[test]
    fn a_named_type_layer_can_make_a_selection_for_action_replay() {
        let mut s = session();
        let id = s.execute("type.create", json!({"text": "I", "x": 40, "y": 120, "size": 48})).unwrap()["layer"].as_u64().unwrap();
        let name = s.active().unwrap().doc.layer(LayerId(id)).unwrap().name.clone();
        s.execute("layer.new.layer", json!({})).unwrap();
        s.execute(CMD, json!({"layer": name})).unwrap();
        assert!(bounds(&s).is_some());
    }

    #[test]
    fn typed_text_selects_its_glyphs_without_a_layer() {
        let mut s = session();
        let layers = s.active().unwrap().doc.layers.len();
        let text = json!({"text": "I", "x": 40, "y": 120, "size": 48});
        s.execute(CMD, text.clone()).unwrap();
        let sel = bounds(&s).expect("the glyph is selected");
        assert_eq!(s.active().unwrap().doc.layers.len(), layers, "no layer added");
        // The same text as a type layer covers the same pixels.
        s.execute("edit.undo", json!({})).unwrap();
        assert!(bounds(&s).is_none(), "one undo step");
        let r = s.execute("type.create", text).unwrap();
        let b: Vec<i32> = r["bounds"].as_array().unwrap().iter().map(|v| v.as_i64().unwrap() as i32).collect();
        assert_eq!([sel.x0, sel.y0, sel.x1, sel.y1], [b[0], b[1], b[2], b[3]]);

        // Vertical: the word runs down the column, so its selection is taller than wide.
        let mut s = session();
        s.execute(CMD, json!({"text": "HELLO", "x": 150, "y": 40, "size": 48, "orientation": "vertical"})).unwrap();
        let v = bounds(&s).unwrap();
        assert!(v.height() > 2 * v.width(), "{v:?}");
    }

    #[test]
    fn bad_params_are_errors() {
        let mut s = session();
        for p in [json!({"layer": "x"}), json!({"removeLayer": true}), json!({"operation": 3}), json!({"antialias": "blurry"}), json!({"layer": 999})] {
            assert!(s.execute(CMD, p.clone()).is_err(), "{p}");
        }
        assert!(Session::new().execute(CMD, json!({"text": "I"})).is_err());
    }
}
