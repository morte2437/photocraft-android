//! Recorded Perspective Warp layout and warp planes (#10710).

use std::collections::BTreeSet;

use photocraft_algo::perspective::{Plane, linked_corners};
use photocraft_algo::transform::{WARP_REACH, convex_quad};
use photocraft_io::smart_map::{MAX_PERSPECTIVE_WARP_PLANES as MAX_PLANES, perspective_warp_geometry, perspective_warp_indexed_geometry};

use super::convert::normalise;
use super::descriptors::{desc, enm, get_obj, get_ref, layer_ref, layer_target, px, ref_class};
use super::events::pixel_target;
use super::fields::{Mapped, step};
use super::{D, Descriptor, ReferenceItem, Value, json};
use crate::distort_cmds::PERSPECTIVE;

const EVENT: &str = "perspectiveWarpTransform";
const SOURCE: &str = "__atnPerspectiveWarp";
const RECT: &str = "__atnPerspectiveWarpRect";
const MAX_DESCRIPTOR_BYTES: usize = 256 * 1024;

/// Allows exactly the measured keys, once each. The preserved descriptor retains their order.
fn exact_keys(d: &Descriptor, keys: &[&str]) -> bool {
    d.name.0.len() <= 1024 && d.items.len() == keys.len() && keys.iter().all(|key| d.items.iter().filter(|(id, _)| id.is(key)).count() == 1)
}

fn coordinate(value: f64) -> bool {
    value.abs() <= f64::from(WARP_REACH)
}

fn rect_valid([left, top, right, bottom]: [f64; 4]) -> bool {
    [left, top, right, bottom].into_iter().all(coordinate) && left < right && top < bottom
}

fn reference_rect(d: &Descriptor) -> Option<[f64; 4]> {
    let r = get_obj(d, "referenceRect")?;
    if !r.class_id.is("Rctn") || !exact_keys(r, &["Top ", "Left", "Btom", "Rght"]) {
        return None;
    }
    let number = |key| match r.get(key)? {
        D::UnitFloat { unit: [b'#', b'P', b'x', b'l'], value } => Some(*value),
        _ => None,
    };
    let r = [number("Left")?, number("Top ")?, number("Rght")?, number("Btom")?];
    rect_valid(r).then_some(r)
}

fn objects<'a>(d: &'a Descriptor, key: &str, class: &str, keys: &[&str], max: usize) -> Option<&'a [D]> {
    let D::List(items) = d.get(key)? else { return None };
    (!items.is_empty() && items.len() <= max && items.iter().all(|item| matches!(item, D::Descriptor(d) if d.class_id.is(class) && exact_keys(d, keys))))
        .then_some(items)
}

/// Each recorded index must link exactly the corners the native command would link. Edges
/// shared by two faces must have opposite winding, since the native map has one neighbour per edge.
fn same_topology(planes: &[Plane], indices: &[[usize; 4]], vertices: usize) -> Option<()> {
    let at = |plane: usize, corner: usize| indices.get(plane)?.get(corner);
    let mut linked = BTreeSet::new();
    for group in linked_corners(planes) {
        let &(plane, corner) = group.first()?;
        let index = at(plane, corner)?;
        if !linked.insert(*index) || !group.iter().all(|&(p, c)| at(p, c) == Some(index)) {
            return None;
        }
    }
    let mut edges = BTreeSet::new();
    let distinct = indices.iter().all(|&[a, b, c, d]| [(a, b), (b, c), (c, d), (d, a)].into_iter().all(|edge| edges.insert(edge)));
    (linked.len() == vertices && distinct).then_some(())
}

/// Decodes the recorded target and its document-space planes, retaining the reference rectangle
/// for export. Unmodelled references and geometry stay as kept steps.
fn parameters(d: &Descriptor) -> Option<Value> {
    if !(d.class_id.is(EVENT) || d.class_id.is("null")) || !exact_keys(d, &["null", "Md  ", "referenceRect", "vertices", "warpedVertices", "quads"]) {
        return None;
    }
    if !matches!(d.get("Md  ")?, D::Enumerated { type_id, value } if type_id.is("perspectiveWarpMode") && value.is("warp")) {
        return None;
    }
    let target = get_ref(d, "null")?;
    if target.len() != 1 || ref_class(target).as_deref() != Some("Lyr") {
        return None;
    }
    match target.first()? {
        ReferenceItem::Enumerated { type_id, value, .. } if type_id.is("Ordn") && value.is("Trgt") => {}
        ReferenceItem::Name { name, .. } if name.0.len() <= 1024 && String::from_utf16(&name.0).is_ok() && !name.to_string_lossy().is_empty() => {}
        ReferenceItem::Property { key, .. } if key.is("Bckg") => {}
        _ => return None,
    }
    let mut p = layer_target(target)?;
    let rect = reference_rect(d)?;
    let src = objects(d, "vertices", "Pnt ", &["Hrzn", "Vrtc"], 4 * MAX_PLANES)?;
    objects(d, "warpedVertices", "Pnt ", &["Hrzn", "Vrtc"], 4 * MAX_PLANES)?;
    objects(d, "quads", "perspectiveWarpQuad", &["indices"], MAX_PLANES)?;
    let (planes, indices) = perspective_warp_indexed_geometry(d)?;
    if !planes.iter().all(|p| p.src.iter().chain(&p.dst).flatten().copied().all(coordinate) && convex_quad(&p.src) && convex_quad(&p.dst)) {
        return None;
    }
    same_topology(&planes, &indices, src.len())?;
    p.insert("planes".into(), json!(planes));
    p.insert("target".into(), json!("pixels"));
    p.insert(RECT.into(), json!(rect));
    Some(Value::Object(p))
}

/// Keep the exact recorded descriptor after decoding its executable settings.
pub(super) fn from(d: &Descriptor) -> Mapped {
    let mapped = || {
        let mut p = parameters(d)?;
        let bytes = d.to_bytes();
        if bytes.len() > MAX_DESCRIPTOR_BYTES {
            return None;
        }
        p[SOURCE] = json!(crate::paint::tile::b64_encode(&bytes));
        Some(p)
    };
    mapped().map_or(Mapped::Unsupported { harmless: false }, |p| step(PERSPECTIVE, p))
}

/// Emits the measured field order when the caller supplies the original reference rectangle.
fn canonical(p: &Value) -> Option<Descriptor> {
    let [left, top, right, bottom] = serde_json::from_value(p.get(RECT)?.clone()).ok()?;
    if !rect_valid([left, top, right, bottom]) {
        return None;
    }
    let planes = p.get("planes")?.as_array().filter(|planes| (1..=MAX_PLANES).contains(&planes.len()))?;
    let planes: Vec<Plane> = serde_json::from_value(Value::Array(planes.clone())).ok()?;
    let rect = desc("Rctn").with("Top ", px(top)).with("Left", px(left)).with("Btom", px(bottom)).with("Rght", px(right));
    let (src, dst, quads) = perspective_warp_geometry(&planes);
    Some(
        desc(EVENT)
            .with("null", layer_ref(p)?)
            .with("Md  ", enm("perspectiveWarpMode", "warp"))
            .with("referenceRect", D::Descriptor(rect))
            .with("vertices", src)
            .with("warpedVertices", dst)
            .with("quads", quads),
    )
}

/// Reuses source bytes only while they still describe the same target, rectangle and planes.
/// A changed recorded mesh cannot silently replace its indexed topology.
pub(super) fn to(_: &str, p: &Value) -> Option<Descriptor> {
    if !pixel_target(p) || !p.as_object()?.keys().all(|k| ["layer", "planes", "target", "interpolation", RECT, SOURCE].contains(&k.as_str())) {
        return None;
    }
    if p.get("interpolation").is_some_and(|v| v.as_str() != Some("bicubic")) {
        return None;
    }
    let descriptor = if let Some(source) = p.get(SOURCE) {
        let text = source.as_str().filter(|v| v.len() <= MAX_DESCRIPTOR_BYTES * 2)?;
        let bytes = crate::paint::tile::b64_decode(text)?;
        if bytes.len() > MAX_DESCRIPTOR_BYTES {
            return None;
        }
        Descriptor::from_bytes(&bytes).ok()?
    } else {
        canonical(p)?
    };
    let mut expected = parameters(&descriptor)?;
    let mut actual = p.clone();
    let m = actual.as_object_mut()?;
    m.remove(SOURCE);
    m.remove("interpolation");
    if m.get("layer").is_some_and(Value::is_null) {
        m.remove("layer");
    }
    m.insert("target".into(), json!("pixels"));
    normalise(&mut actual);
    normalise(&mut expected);
    (actual == expected).then_some(descriptor)
}
