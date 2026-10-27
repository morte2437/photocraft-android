//! Color Sampler points (#1046): numbered colour readings pinned to canvas pixels and listed in
//! the Info panel, so a fixed position can be compared instead of chasing the live pointer.
//!
//! Points are per-document view state ([`crate::DocState::color_samplers`]): they belong to their
//! document, are discarded when it closes, and are not saved to `.pcraft` or PSD. Placing, moving
//! and removing them changes no pixels and adds no History step, so they are not journaled. Each
//! point reads one composite pixel (no averaging); its values match the Info panel's pointer
//! readout (RGB 0–255 and colour-managed CMYK percentages, as in the Color Picker).

use photocraft_doc::Document;
use serde_json::{Value, json};

use crate::commands::CommandSpec;
use crate::{DocState, EngineError, Result, Session};

/// Photoshop's Color Sampler limit: at most 10 points per document.
pub const MAX_SAMPLERS: usize = 10;

fn bad(cmd: &str, msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: cmd.into(), msg: msg.into() }
}

fn has_doc(s: &Session) -> std::result::Result<(), String> {
    s.active().map(|_| ()).ok_or_else(|| "no document open".into())
}

/// Colour-managed CMYK, in percent, as the Info panel shows it: the same conversion the Color
/// Picker uses (`photocraft_color::convert::rgb_to_cmyk`: the CMYK profile active on this thread,
/// else the built-in coated CMYK), so both panels agree (#8101). 32-bit samples can lie outside
/// 0..1, so each channel is clamped first (NaN reads as 0), like the RGB readout.
pub fn device_cmyk(r: f32, g: f32, b: f32) -> [i32; 4] {
    let unit = |v: f32| if v.is_nan() { 0.0 } else { v.clamp(0.0, 1.0) };
    photocraft_color::convert::rgb_to_cmyk([unit(r), unit(g), unit(b)]).map(|v| (unit(v) * 100.0).round() as i32)
}

/// The Info-panel values for a sample point at document pixel `pos`: composite RGB 0–255 and
/// colour-managed CMYK percentages (the document's CMYK profile when it has one). One composite
/// pixel, no averaging.
pub fn readout(doc: &Document, pos: [f64; 2]) -> ([i32; 3], [i32; 4]) {
    let rgba = crate::sample_cmds::sample_color(doc, None, pos[0], pos[1], 1, crate::sample_cmds::SampleLayers::All);
    let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as i32;
    let cmyk = photocraft_color::convert::with_cmyk_space(photocraft_compose::cmyk_space(doc).as_ref(), || device_cmyk(rgba[0], rgba[1], rgba[2]));
    ([q(rgba[0]), q(rgba[1]), q(rgba[2])], cmyk)
}

/// One point as the list command reports it.
pub fn point_json(doc: &Document, index: usize, pos: [f64; 2]) -> Value {
    let (rgb, cmyk) = readout(doc, pos);
    json!({"index": index, "number": index + 1, "position": [pos[0], pos[1]], "rgb": rgb, "cmyk": cmyk})
}

fn list_json(doc: &Document, points: &[[f64; 2]]) -> Value {
    json!({
        "max": MAX_SAMPLERS,
        "samplers": points.iter().enumerate().map(|(i, p)| point_json(doc, i, *p)).collect::<Vec<_>>(),
    })
}

fn coord(cmd: &str, p: &Value, key: &str) -> Result<f64> {
    p.get(key).and_then(Value::as_f64).filter(|v| v.is_finite()).ok_or_else(|| bad(cmd, format!("`{key}` must be a finite number (document pixels)")))
}

/// The pixel a point at document coordinate `(x, y)` sits on, if it is inside the canvas. Whole
/// pixels only: the click's fractional position floors to the pixel it lands on.
fn pixel(doc: &Document, x: f64, y: f64) -> Option<[f64; 2]> {
    let (px, py) = (x.floor(), y.floor());
    if px < f64::from(i32::MIN) || px > f64::from(i32::MAX) || py < f64::from(i32::MIN) || py > f64::from(i32::MAX) {
        return None;
    }
    doc.bounds().contains(px as i32, py as i32).then_some([px, py])
}

fn index_param(cmd: &str, p: &Value) -> Result<usize> {
    p.get("index").and_then(Value::as_u64).and_then(|n| usize::try_from(n).ok()).ok_or_else(|| bad(cmd, "pass `index` (0-based)"))
}

/// Apply a Color Sampler change to the active document: view state, so no history step and no
/// pixels. Bumps the revision (keeping a clean document clean) so UIs refresh.
fn change<R>(s: &mut Session, f: impl FnOnce(&mut DocState) -> Result<R>) -> Result<R> {
    let st = s.active_mut().ok_or(EngineError::NoDocument)?;
    let r = f(st)?;
    let clean = st.saved_revision == st.revision;
    st.revision += 1;
    if clean {
        st.saved_revision = st.revision;
    }
    st.last_damage = Some(photocraft_geom::Rect::EMPTY);
    Ok(r)
}

fn add(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "view.colorSamplers.add";
    let x = coord(CMD, p, "x")?;
    let y = coord(CMD, p, "y")?;
    change(s, |st| {
        let pos = pixel(&st.doc, x, y).ok_or_else(|| EngineError::Other("the sample point is outside the canvas".into()))?;
        if st.color_samplers.len() >= MAX_SAMPLERS {
            return Err(EngineError::Other(format!("Color Sampler limit reached: at most {MAX_SAMPLERS} points per document")));
        }
        st.color_samplers.push(pos);
        let i = st.color_samplers.len() - 1;
        Ok(json!({"index": i, "count": st.color_samplers.len(), "point": point_json(&st.doc, i, pos)}))
    })
}

fn move_point(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "view.colorSamplers.move";
    let i = index_param(CMD, p)?;
    let x = coord(CMD, p, "x")?;
    let y = coord(CMD, p, "y")?;
    change(s, |st| {
        let n = st.color_samplers.len();
        if i >= n {
            return Err(bad(CMD, format!("no sample point {i} (the document has {n})")));
        }
        let pos = pixel(&st.doc, x, y).ok_or_else(|| EngineError::Other("the sample point is outside the canvas".into()))?;
        st.color_samplers[i] = pos;
        Ok(json!({"index": i, "point": point_json(&st.doc, i, pos)}))
    })
}

fn delete(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "view.colorSamplers.delete";
    let i = index_param(CMD, p)?;
    change(s, |st| {
        let n = st.color_samplers.len();
        if i >= n {
            return Err(bad(CMD, format!("no sample point {i} (the document has {n})")));
        }
        st.color_samplers.remove(i);
        Ok(json!({"deleted": 1, "remaining": st.color_samplers.len()}))
    })
}

fn clear(s: &mut Session, _p: &Value) -> Result<Value> {
    change(s, |st| {
        let n = st.color_samplers.len();
        st.color_samplers.clear();
        Ok(json!({"deleted": n, "remaining": 0}))
    })
}

fn list(s: &mut Session, _p: &Value) -> Result<Value> {
    let st = s.active().ok_or(EngineError::NoDocument)?;
    Ok(list_json(&st.doc, &st.color_samplers))
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "view.colorSamplers.add",
            label: "Add Color Sampler",
            menu: &[],
            shortcut: None,
            params: r##"{"x":f64,"y":f64} → the new point {index, number, position, rgb, cmyk}; errors at the 10-point limit or off the canvas"##,
            enabled: has_doc,
            run: add,
            journal: false,
        },
        CommandSpec {
            id: "view.colorSamplers.move",
            label: "Move Color Sampler",
            menu: &[],
            shortcut: None,
            params: r##"{"index":n,"x":f64,"y":f64}"##,
            enabled: has_doc,
            run: move_point,
            journal: false,
        },
        CommandSpec {
            id: "view.colorSamplers.delete",
            label: "Delete Color Sampler",
            menu: &[],
            shortcut: None,
            params: r##"{"index":n}"##,
            enabled: has_doc,
            run: delete,
            journal: false,
        },
        CommandSpec {
            id: "view.colorSamplers.clear",
            label: "Clear Color Samplers",
            menu: &[],
            shortcut: None,
            params: "{}",
            enabled: has_doc,
            run: clear,
            journal: false,
        },
        CommandSpec {
            id: "view.colorSamplers.list",
            label: "Color Samplers",
            menu: &[],
            shortcut: None,
            params: r##"{} → {max, samplers:[{index, number, position, rgb, cmyk}]}"##,
            enabled: has_doc,
            run: list,
            journal: false,
        },
    ]
}

#[cfg(test)]
mod tests;
