//! Puppet Warp smart filters (`Fltr` class `rigidTransform`) as observed in Photoshop-authored
//! mockup PSDs.
//!
//! The filter holds a `puppetShapeList` with one `puppetShape` per mesh. Each shape stores its
//! pins (`PinP` where they were placed, `posFinalPins` where they are now, x, y pairs in document
//! pixels) and the mesh Photoshop solved for them, as raw data:
//!
//! ```text
//! originalVertexArray, deformedVertexArray: f32 LE x, y per vertex (document pixels)
//! indexArray: i32 LE, three per triangle
//! ```
//!
//! and the corners of the layer the mesh was built on (`PuX0..3`, `PuY0..3` on the filter).
//! Replace Contents keeps the mesh: Photoshop redraws the new contents through the stored
//! triangles (measured on a dot grid: 0.1 px mean). So the mesh is imported as the
//! `layer.smartObjects.puppetWarp` filter's `mesh`, which is drawn as it is while the pins are
//! unchanged, and the whole item is kept as a template ([`TEMPLATE_KEY`]) to write back. Pins
//! alone feed PhotoCraft's own solver once they are edited; the mode, density and rotation
//! settings aren't mapped (their stored values aren't identified), so those stay at defaults.

use photocraft_doc::SmartFilter;
use photocraft_psd::descriptor::{Descriptor, Value};
use serde_json::{Value as J, json};

use super::{blend_options, from_hex, get_desc, set, to_hex};
use crate::blocks::num;

pub(super) const CLASS: &str = "rigidTransform";
pub(super) const COMMAND: &str = "layer.smartObjects.puppetWarp";
pub(super) const TEMPLATE_KEY: &str = "__puppetPsd";
/// Far beyond the few hundred vertices Photoshop's meshes have, yet small enough that the
/// copies a hostile mesh makes (arrays, JSON params, cloned filters) stay bounded.
const MAX_VERTS: usize = 1 << 16;

fn raw<'a>(d: &'a Descriptor, key: &str) -> Option<&'a [u8]> {
    match d.get(key)? {
        Value::RawData(b) => Some(b),
        _ => None,
    }
}

fn floats(d: &Descriptor, key: &str) -> Option<Vec<f64>> {
    let Value::List(v) = d.get(key)? else { return None };
    v.iter().map(|x| num(Some(x)).filter(|f| f.is_finite())).collect()
}

/// `f32` LE x, y pairs; `None` unless whole, finite and at most [`MAX_VERTS`].
fn vertices(b: &[u8]) -> Option<Vec<[f64; 2]>> {
    if !b.len().is_multiple_of(8) || b.len() / 8 > MAX_VERTS {
        return None;
    }
    let f = |c: &[u8]| c.try_into().ok().map(f32::from_le_bytes).filter(|v| v.is_finite()).map(f64::from);
    b.as_chunks::<8>().0.iter().map(|c| Some([f(&c[..4])?, f(&c[4..])?])).collect()
}

/// `i32` LE triples, each index below `n`.
fn triangles(b: &[u8], n: usize) -> Option<Vec<[usize; 3]>> {
    if !b.len().is_multiple_of(12) || b.len() / 12 > 2 * MAX_VERTS {
        return None;
    }
    let i = |c: &[u8]| c.try_into().ok().map(i32::from_le_bytes).and_then(|v| usize::try_from(v).ok()).filter(|&v| v < n);
    b.as_chunks::<12>().0.iter().map(|c| Some([i(&c[..4])?, i(&c[4..8])?, i(&c[8..])?])).collect()
}

/// One shape's pins (as params), vertices, deformed vertices and triangles.
type Shape = (Vec<J>, Vec<[f64; 2]>, Vec<[f64; 2]>, Vec<[usize; 3]>);

/// Pins and mesh of one `puppetShape`, with vertex indices offset by `base`.
fn shape(s: &Descriptor, base: usize) -> Option<Shape> {
    let (from, to) = (floats(s, "PinP")?, floats(s, "posFinalPins")?);
    if from.len() != to.len() || !from.len().is_multiple_of(2) {
        return None;
    }
    let pins = from.as_chunks::<2>().0.iter().zip(to.as_chunks::<2>().0).map(|(a, b)| json!({"src": a, "dst": b})).collect();
    let verts = vertices(raw(s, "originalVertexArray")?)?;
    let deformed = vertices(raw(s, "deformedVertexArray")?)?;
    if verts.len() != deformed.len() {
        return None;
    }
    let tris = triangles(raw(s, "indexArray")?, verts.len())?;
    Some((pins, verts, deformed, tris.into_iter().map(|t| t.map(|i| i + base)).collect()))
}

/// Pins and mesh of a `rigidTransform` descriptor (shapes joined into one mesh), as
/// `{"pins", "mesh"}`.
fn settings(f: &Descriptor) -> Option<J> {
    let Some(Value::List(shapes)) = f.get("puppetShapeList") else { return None };
    let (mut pins, mut verts, mut deformed, mut tris) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for s in shapes {
        let Value::Descriptor(s) = s else { return None };
        let (p, v, d, t) = shape(s, verts.len())?;
        pins.extend(p);
        verts.extend(v);
        deformed.extend(d);
        tris.extend(t);
    }
    if tris.is_empty() || verts.len() > MAX_VERTS {
        return None;
    }
    let pins = J::Array(pins);
    Some(json!({"pins": pins, "mesh": {"pins": pins, "verts": verts, "deformed": deformed, "tris": tris}}))
}

/// The corners of the layer the mesh was built on (`PuX0`, `PuY0` … `PuX3`, `PuY3`), as x, y
/// pairs.
pub(super) fn built_on(params: &J) -> Option<[f64; 8]> {
    let item = template(params).ok()?;
    let f = get_desc(&item, "Fltr")?;
    let mut q = [0.0; 8];
    for i in 0..4 {
        q[2 * i] = num(f.get(&format!("PuX{i}"))).filter(|v| v.is_finite())?;
        q[2 * i + 1] = num(f.get(&format!("PuY{i}"))).filter(|v| v.is_finite())?;
    }
    Some(q)
}

/// `layer.smartObjects.puppetWarp` params for a Puppet Warp item with a readable mesh.
pub(super) fn import_params(item: &Descriptor) -> Option<J> {
    let f = get_desc(item, "Fltr")?;
    if !f.class_id.is(CLASS) {
        return None;
    }
    let mut params = settings(f)?;
    // Neutral blending in the template: the model's own fields replace it on export.
    let mut template = item.clone();
    let neutral = SmartFilter { command: String::new(), params: J::Null, blend: photocraft_color::BlendMode::Normal, opacity: 1.0, visible: true };
    set(&mut template, "blendOptions", blend_options(&neutral));
    set(&mut template, "enab", Value::Boolean(true));
    params.as_object_mut()?.insert(TEMPLATE_KEY.into(), json!(to_hex(&template.to_bytes())));
    Some(params)
}

/// The template item of `params`, checked to be a Puppet Warp filter.
fn template(params: &J) -> Result<Descriptor, String> {
    let raw = params.get(TEMPLATE_KEY).and_then(J::as_str).ok_or("a Puppet Warp made in PhotoCraft is not written to PSD")?;
    let bytes = from_hex(raw).ok_or("the Puppet Warp PSD data is not valid hex")?;
    let item = Descriptor::from_bytes(&bytes).map_err(|e| format!("the Puppet Warp PSD data is unreadable ({e})"))?;
    if !item.class_id.is("filterFX") || !get_desc(&item, "Fltr").is_some_and(|f| f.class_id.is(CLASS)) {
        return Err("the Puppet Warp PSD data is not a Puppet Warp filter".into());
    }
    Ok(item)
}

/// The `filterFXList` item: the template, while the pins are still the imported ones.
pub(super) fn export_item(params: &J) -> Result<Descriptor, String> {
    let item = template(params)?;
    let imported = get_desc(&item, "Fltr").and_then(settings).ok_or("the Puppet Warp PSD data has no readable mesh")?;
    if params.get("pins") != imported.get("pins") {
        return Err("Puppet Warp pins moved in PhotoCraft are not written to PSD".into());
    }
    // Imported params set no solver settings, so a set one is an edit the template doesn't hold.
    if let Some(k) = ["mode", "density", "expansion"].into_iter().find(|k| params.get(*k).is_some()) {
        return Err(format!("the Puppet Warp `{k}` set in PhotoCraft is not written to PSD"));
    }
    Ok(item)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::smart_map::{UNSUPPORTED_FILTER, filter_from_item, item_for_filter, parse_sold};
    use photocraft_psd::descriptor::{UnicodeString, VersionedDescriptor};

    const QUAD: [f64; 8] = [10.0, 20.0, 110.0, 22.0, 108.0, 220.0, 12.0, 218.0];

    fn list(v: &[f64]) -> Value {
        Value::List(v.iter().map(|x| Value::Double(*x)).collect())
    }

    fn f32s(v: &[[f32; 2]]) -> Value {
        Value::RawData(v.iter().flatten().flat_map(|x| x.to_le_bytes()).collect())
    }

    fn i32s(v: &[i32]) -> Value {
        Value::RawData(v.iter().flat_map(|x| x.to_le_bytes()).collect())
    }

    /// A Puppet Warp shape as Photoshop writes it: a box of two triangles with one pin.
    fn shape(dx: f32) -> Descriptor {
        let verts = [[10.0, 20.0], [110.0, 22.0], [108.0, 220.0], [12.0, 218.0]];
        let moved: Vec<[f32; 2]> = verts.iter().map(|v| [v[0] + dx, v[1]]).collect();
        Descriptor::new("puppetShape")
            .with("rigidType", Value::Boolean(true))
            .with("originalVertexArray", f32s(&verts))
            .with("deformedVertexArray", f32s(&moved))
            .with("indexArray", i32s(&[0, 1, 2, 0, 2, 3]))
            .with("pinOffsets", list(&[0.0, 0.0]))
            .with("posFinalPins", list(&[60.0 + f64::from(dx), 120.0]))
            .with("pinVertexIndices", list(&[0.0]))
            .with("PinP", list(&[60.0, 120.0]))
            .with("meshExpansion", Value::Integer(2))
    }

    fn item(shapes: Vec<Descriptor>) -> Descriptor {
        let mut f = Descriptor::new(CLASS)
            .with("rigidType", Value::Boolean(true))
            .with("puppetShapeList", Value::List(shapes.into_iter().map(Value::Descriptor).collect()));
        for i in 0..4 {
            f = f.with(&format!("PuX{i}"), Value::Double(QUAD[2 * i])).with(&format!("PuY{i}"), Value::Double(QUAD[2 * i + 1]));
        }
        let neutral = SmartFilter { command: String::new(), params: J::Null, blend: photocraft_color::BlendMode::Normal, opacity: 1.0, visible: true };
        Descriptor::new("filterFX")
            .with("Nm  ", Value::Text(UnicodeString::new_nul("Puppet Warp")))
            .with("blendOptions", blend_options(&neutral))
            .with("enab", Value::Boolean(true))
            .with("hasoptions", Value::Boolean(true))
            .with("Fltr", Value::Descriptor(f))
            .with("filterID", Value::Integer(973))
    }

    #[test]
    fn puppet_items_map_to_the_command_with_their_mesh() {
        let src = item(vec![shape(4.0), shape(-2.0)]);
        let f = filter_from_item(&src);
        assert_eq!(f.command, COMMAND);
        assert_eq!(f.params["pins"], json!([{"src": [60.0, 120.0], "dst": [64.0, 120.0]}, {"src": [60.0, 120.0], "dst": [58.0, 120.0]}]));
        let mesh = &f.params["mesh"];
        assert_eq!(mesh["pins"], f.params["pins"]);
        assert_eq!(mesh["verts"].as_array().unwrap().len(), 8);
        assert_eq!(mesh["verts"][2], json!([108.0, 220.0]));
        assert_eq!(mesh["deformed"][2], json!([112.0, 220.0]));
        // The second shape's triangles index its own vertices.
        assert_eq!(mesh["tris"], json!([[0, 1, 2], [0, 2, 3], [4, 5, 6], [4, 6, 7]]));
        assert_eq!(built_on(&f.params), Some(QUAD));
        // Unchanged pins write the original item back; blending is the model's.
        assert_eq!(item_for_filter(&f).unwrap(), src);
        let mut faded = f.clone();
        faded.opacity = 0.5;
        assert_eq!(filter_from_item(&item_for_filter(&faded).unwrap()).opacity, 0.5);
        // Moved pins aren't written (the stored mesh would no longer match them).
        let mut moved = f.clone();
        moved.params["pins"][0]["dst"] = json!([70.0, 120.0]);
        assert!(item_for_filter(&moved).is_err());
        // Nor are solver settings set after import (the template holds the imported ones).
        for (k, v) in [("mode", json!("rigid")), ("density", json!("more")), ("expansion", json!(2.0))] {
            let mut edited = f.clone();
            edited.params[k] = v;
            assert!(item_for_filter(&edited).is_err(), "{k}");
        }
        // The import records the placement the mesh was solved for; the item written back is
        // unchanged by it.
        let mut sm = photocraft_doc::SmartObject::new(
            photocraft_doc::SmartSource::Linked { path: String::new() },
            photocraft_geom::Affine { m: [1.0, 0.0, 0.0, 1.0, 3.0, 4.0] },
            None,
        );
        sm.smart_filters.push(f.clone());
        crate::smart_map::pin_puppet_meshes(&mut sm);
        assert_eq!(sm.smart_filters[0].params["mesh"]["placement"], sm.placement_json());
        assert_eq!(item_for_filter(&sm.smart_filters[0]).unwrap(), src);
        // A Puppet Warp made in PhotoCraft has no template: not written, as before.
        let own = SmartFilter { params: json!({"pins": f.params["pins"]}), ..f };
        assert!(item_for_filter(&own).is_err());
    }

    #[test]
    fn unreadable_puppet_filters_stay_opaque() {
        let with = |key: &str, v: Value| {
            let mut s = shape(4.0);
            set(&mut s, key, v);
            item(vec![s])
        };
        for src in [
            with("originalVertexArray", Value::RawData(vec![0; 31])),
            with("originalVertexArray", f32s(&[[0.0, 0.0], [1.0, 0.0], [1.0, 1.0]])),
            with("deformedVertexArray", f32s(&[[f32::NAN, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]])),
            with("indexArray", i32s(&[0, 1, 4])),
            with("indexArray", i32s(&[0, 1, -1])),
            with("indexArray", Value::RawData(vec![0; 10])),
            with("indexArray", i32s(&[])),
            with("posFinalPins", list(&[60.0])),
            with("PinP", Value::Integer(3)),
            item(vec![]),
        ] {
            let f = filter_from_item(&src);
            assert_eq!(f.command, UNSUPPORTED_FILTER);
            assert_eq!(item_for_filter(&f).unwrap(), src);
        }
    }

    #[test]
    fn a_mesh_built_on_other_corners_is_dropped() {
        // The mesh is in document pixels where the layer was when it was built: a layer whose
        // corners differ from the stored ones keeps the pins but not the mesh.
        let sold = |quad: [f64; 8]| {
            let fx = Descriptor::new("filterFXStyle")
                .with("enab", Value::Boolean(true))
                .with("filterFXList", Value::List(vec![Value::Descriptor(item(vec![shape(4.0)]))]));
            let d = Descriptor::new("null").with("Trnf", list(&quad)).with("nonAffineTransform", list(&quad)).with("filterFX", Value::Descriptor(fx));
            let mut v = b"soLD".to_vec();
            v.extend_from_slice(&4u32.to_be_bytes());
            v.extend(VersionedDescriptor::new(d).to_bytes());
            v
        };
        let filter = |quad| parse_sold(&sold(quad)).and_then(|p| p.stack).map(|s| s.filters[0].clone()).expect("a stack");
        let kept = filter(QUAD);
        assert_eq!(kept.command, COMMAND);
        assert!(kept.params.get("mesh").is_some());
        let mut moved = QUAD;
        moved[0] += 5.0;
        let f = filter(moved);
        assert_eq!(f.command, COMMAND);
        assert!(f.params.get("mesh").is_none());
        assert_eq!(f.params["pins"], kept.params["pins"]);
        // Still written back as it was.
        assert_eq!(item_for_filter(&f).unwrap(), item(vec![shape(4.0)]));
    }
}
