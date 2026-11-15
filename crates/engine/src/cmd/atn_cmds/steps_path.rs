//! Measured Insert Path: per-component operations, with even-odd subpaths in each component.
use super::descriptors::{PX, class, desc, enm, get_list, get_ref, jstr, px};
use super::fields::{Mapped, step};
use super::{D, Descriptor, Id, ReferenceItem, Value, json};
use photocraft_doc::{FillRule, Knot, Path, PathOp, Subpath};
use photocraft_geom::Point;

pub(super) const SOURCE: &str = "_atnInsertPathDescriptor";
// The native path validator admits at most 4096 knots/subpaths. Larger wire
// records remain complete kept steps rather than secondary geometry allocations.
const MAX_PARTS: usize = 4096;
const MAX_SOURCE_BYTES: usize = 4 * 1024 * 1024;

#[cfg(test)]
#[path = "path_tests.rs"]
mod tests;

/// Bound the retained wire form before serializing it into a second allocation.
/// Names use UTF-16; 64 bytes per measured field conservatively covers its fixed
/// key/type/scalar or list header. Nested descriptors charge their own fields.
fn keys(d: &Descriptor, required: &[&str], optional: &[&str], budget: &mut usize) -> bool {
    let size = d.name.0.len().checked_mul(2).and_then(|names| d.items.len().checked_mul(64)?.checked_add(names)?.checked_add(32));
    let Some(remaining) = size.and_then(|size| budget.checked_sub(size)) else { return false };
    *budget = remaining;
    d.name.0.len() <= 4096
        && d.items.iter().all(|(k, _)| required.iter().chain(optional).any(|key| k.is(key)))
        && required.iter().all(|key| d.items.iter().filter(|(k, _)| k.is(key)).count() == 1)
        && optional.iter().all(|key| d.items.iter().filter(|(k, _)| k.is(key)).count() <= 1)
}

/// Require the recorded object class before interpreting its fields as geometry.
fn object<'a>(value: &'a D, class_id: &str) -> Option<&'a Descriptor> {
    match value {
        D::Descriptor(d) if d.class_id.is(class_id) => Some(d),
        _ => None,
    }
}

/// Read a pixel point within the native coordinate range and charge its fields to the wire budget.
fn point(value: &D, budget: &mut usize) -> Option<Point> {
    let d = object(value, "Pnt ")?;
    if !keys(d, &["Hrzn", "Vrtc"], &[], budget) {
        return None;
    }
    let coordinate = |key| match d.get(key)? {
        D::UnitFloat { unit, value } if *unit == PX && value.is_finite() && value.abs() <= f64::from(f32::MAX) => Some(*value),
        _ => None,
    };
    Some(Point::new(coordinate("Hrzn")?, coordinate("Vrtc")?))
}

/// A corner stores only its anchor. A cubic knot must supply both handles and its smooth flag.
fn knot(value: &D, budget: &mut usize) -> Option<Knot> {
    let d = object(value, "Pthp")?;
    if !keys(d, &["Anch"], &["Fwd ", "Bwd ", "Smoo"], budget) {
        return None;
    }
    let anchor = point(d.get("Anch")?, budget)?;
    match (d.get("Fwd "), d.get("Bwd "), d.get("Smoo")) {
        (None, None, None) => Some(Knot::corner(anchor.x, anchor.y)),
        (Some(out), Some(input), Some(D::Boolean(smooth))) => {
            Some(Knot { anchor, out_ctrl: point(out, budget)?, in_ctrl: point(input, budget)?, smooth: *smooth })
        }
        _ => None,
    }
}

/// Decode a Work Path property assignment, preserving the grouping and operation of each component.
fn path_from(d: &Descriptor) -> Option<Path> {
    let mut budget = MAX_SOURCE_BYTES;
    if !(d.class_id.is("null") || d.class_id.is("setd")) || !keys(d, &["null", "T   "], &[], &mut budget) {
        return None;
    }
    let Some([ReferenceItem::Property { class, key }]) = get_ref(d, "null") else { return None };
    if !class.class_id.is("Path") || class.name.0.len() > 4096 || !key.is("WrPt") {
        return None;
    }
    budget = budget.checked_sub(class.name.0.len().checked_mul(2)?.checked_add(64)?)?;
    let components = get_list(d, "T   ")?;
    if components.is_empty() || components.len() > MAX_PARTS {
        return None;
    }
    let mut path = Path { fill_rule: FillRule::EvenOdd, ..Default::default() };
    let mut count = 0usize;
    for value in components {
        let component = object(value, "PaCm")?;
        if !keys(component, &["shapeOperation", "SbpL"], &[], &mut budget) {
            return None;
        }
        let op = match component.get("shapeOperation")? {
            D::Enumerated { type_id, value } if type_id.is("shapeOperation") => {
                if value.is("Add ") {
                    PathOp::Combine
                } else if value.is("Sbtr") {
                    PathOp::Subtract
                } else if value.is("xor") {
                    PathOp::Exclude
                } else {
                    return None;
                }
            }
            _ => return None,
        };
        if path.subpaths.is_empty() && op != PathOp::Combine {
            return None;
        }
        let subpaths = get_list(component, "SbpL")?;
        if subpaths.is_empty() || path.subpaths.len().checked_add(subpaths.len())? > MAX_PARTS {
            return None;
        }
        for (i, value) in subpaths.iter().enumerate() {
            let sub = object(value, "Sbpl")?;
            if !keys(sub, &["Clsp", "Pts "], &[], &mut budget) {
                return None;
            }
            let D::Boolean(closed) = sub.get("Clsp")? else { return None };
            let points = get_list(sub, "Pts ")?;
            count = count.checked_add(points.len())?;
            if count > MAX_PARTS {
                return None;
            }
            let knots = points.iter().map(|value| knot(value, &mut budget)).collect::<Option<Vec<_>>>()?;
            path.subpaths.push(Subpath { closed: *closed, knots, op: if i == 0 { op } else { PathOp::Join } });
        }
    }
    photocraft_vector::flatten::validate_path(&path).ok()?;
    Some(path)
}

/// Decode only the measured Work Path form; unsupported data stays a complete kept step.
pub(super) fn from(d: &Descriptor) -> Mapped {
    let Some(path) = path_from(d) else { return Mapped::Unsupported { harmless: false } };
    let raw = d.to_bytes();
    if raw.len() > MAX_SOURCE_BYTES {
        return Mapped::Unsupported { harmless: false };
    }
    step("path.set", json!({"name":"work", "path":crate::vector_cmds::path_json(&path), SOURCE:photocraft_paint::tile::b64_encode(&raw)}))
}

/// Write a point using pixel units for both coordinates.
fn point_to(p: Point) -> D {
    D::Descriptor(desc("Pnt ").with("Hrzn", px(p.x)).with("Vrtc", px(p.y)))
}

/// Emit the compact corner form only when the handles coincide and the knot has no smooth flag.
fn knot_to(k: &Knot) -> D {
    let mut d = desc("Pthp").with("Anch", point_to(k.anchor));
    if k.in_ctrl != k.anchor || k.out_ctrl != k.anchor || k.smooth {
        d = d.with("Fwd ", point_to(k.out_ctrl)).with("Bwd ", point_to(k.in_ctrl)).with("Smoo", D::Boolean(k.smooth));
    }
    D::Descriptor(d)
}

/// Export representable native geometry or an unchanged validated source descriptor.
/// Nonzero winding, inversion and unmeasured operations cannot silently become even-odd.
pub(super) fn to(p: &Value) -> Option<Descriptor> {
    if jstr(p, "name") != Some("work") || p.as_object()?.keys().any(|k| !matches!(k.as_str(), "name" | "path" | SOURCE)) {
        return None;
    }
    let value = p.get("path")?;
    let subpaths = value.get("subpaths")?.as_array()?;
    if subpaths.is_empty()
        || subpaths.len() > MAX_PARTS
        || subpaths.iter().try_fold(0usize, |n, s| n.checked_add(s.get("knots")?.as_array()?.len()))? > MAX_PARTS
    {
        return None;
    }
    let path = crate::vector_cmds::parse_path(value).ok()?;
    photocraft_vector::flatten::validate_path(&path).ok()?;
    let mut canonical = crate::vector_cmds::path_json(&path);
    let mut supplied = value.clone();
    super::convert::normalise(&mut canonical);
    super::convert::normalise(&mut supplied);
    if path.inverted || path.fill_rule != FillRule::EvenOdd || canonical != supplied {
        return None;
    }
    if let Some(source) = p.get(SOURCE) {
        let text = source.as_str()?;
        if text.len() > MAX_SOURCE_BYTES.div_ceil(3) * 4 {
            return None;
        }
        let raw = photocraft_paint::tile::b64_decode(text)?;
        if raw.len() > MAX_SOURCE_BYTES {
            return None;
        }
        let original = Descriptor::from_bytes(&raw).ok()?;
        return (path_from(&original)? == path).then_some(original);
    }
    let mut components = Vec::new();
    for range in path.components() {
        let first = path.subpaths.get(range.start)?;
        let op = match first.op {
            PathOp::Combine => "Add ",
            PathOp::Subtract => "Sbtr",
            PathOp::Exclude => "xor",
            _ => return None,
        };
        if components.is_empty() && first.op != PathOp::Combine {
            return None;
        }
        let subs = path
            .subpaths
            .get(range)?
            .iter()
            .map(|s| D::Descriptor(desc("Sbpl").with("Clsp", D::Boolean(s.closed)).with("Pts ", D::List(s.knots.iter().map(knot_to).collect()))))
            .collect();
        components.push(D::Descriptor(desc("PaCm").with("shapeOperation", enm("shapeOperation", op)).with("SbpL", D::List(subs))));
    }
    Some(
        desc("setd").with("null", D::Reference(vec![ReferenceItem::Property { class: class("Path"), key: Id::new("WrPt") }])).with("T   ", D::List(components)),
    )
}
