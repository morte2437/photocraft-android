//! Outlines of curves (`PCvD` node lists), parametric shapes (`Shpe` objects) and compounds, in
//! node space.

use crate::model::{Affine, Path, Point, Reader, SubPath};
use crate::stream::{ObjId, Tag, Value};

/// Nodes per curve object.
const MAX_NODES: usize = 4_000_000;
const MAX_COMPOUND_DEPTH: usize = 64;
const MAX_OUTLINE_VISITS: usize = 500_000;
const MAX_OUTLINE_POINTS: usize = 4_000_000;
/// Curve records decoded across the document. A shared curve is decoded again for each
/// occurrence, and records that only move handles add no points.
const MAX_OUTLINE_RECORDS: usize = 8_000_000;

/// Geometry is expanded per occurrence, since parametric outlines can depend on their world
/// transform. Bound work across the entire document instead of memoizing by object id alone.
pub(crate) struct Budget {
    visits_left: usize,
    points_left: usize,
    records_left: usize,
}

impl Default for Budget {
    fn default() -> Self {
        Self { visits_left: MAX_OUTLINE_VISITS, points_left: MAX_OUTLINE_POINTS, records_left: MAX_OUTLINE_RECORDS }
    }
}

impl Budget {
    /// Charge an outline call or a raw compound operand slot before inspecting it.
    fn visit(&mut self) -> Result<(), &'static str> {
        self.visits_left = self.visits_left.checked_sub(1).ok_or("compound geometry budget exhausted")?;
        Ok(())
    }

    /// Charge curve records before they are decoded.
    fn records(&mut self, n: usize) -> Result<(), &'static str> {
        self.records_left = self.records_left.checked_sub(n).ok_or("curve geometry budget exhausted")?;
        Ok(())
    }

    /// Charge both leaf geometry and each copy into a compound before allocating that copy.
    fn path(&mut self, path: &Path) -> Result<(), &'static str> {
        let points = path.subpaths.iter().fold(0usize, |n, s| n.saturating_add(1).saturating_add(s.segments.len().saturating_mul(3)));
        let Some(left) = self.points_left.checked_sub(points) else {
            // Do not rebuild an expensive leaf for every subsequent root after exhaustion.
            self.visits_left = 0;
            return Err("compound geometry budget exhausted");
        };
        self.points_left = left;
        Ok(())
    }
}

/// The node-space outline of a curve, shape or compound node, and whether it fills even-odd.
/// Instances (master-page or symbol copies) take the geometry of their master.
pub(crate) fn outline(r: &mut Reader, id: ObjId, class: Tag, world: Affine) -> Option<(Path, bool)> {
    match outline_inner(r, id, class, world, &mut Vec::new()) {
        Ok(path) => path,
        Err(reason) => {
            r.warn(reason);
            None
        }
    }
}

fn outline_inner(r: &mut Reader, id: ObjId, class: Tag, world: Affine, active: &mut Vec<ObjId>) -> Result<Option<(Path, bool)>, &'static str> {
    r.geometry_budget.visit()?;
    if active.contains(&id) {
        return Err("cyclic compound shape");
    }
    // Independent of the layer-tree limit: Comp operands do not pass through Reader::node.
    if active.len() >= MAX_COMPOUND_DEPTH {
        return Err("compound depth exceeds 64 levels");
    }
    if class != Tag::of(b"Comp") {
        let path = leaf_outline(r, id, class, world);
        if let Some((path, _)) = &path {
            r.geometry_budget.path(path)?;
        }
        return Ok(path);
    }
    active.push(id);
    let result = compound(r, id, world, active);
    active.pop();
    result
}

fn compound(r: &mut Reader, id: ObjId, world: Affine, active: &mut Vec<ObjId>) -> Result<Option<(Path, bool)>, &'static str> {
    let s = r.s;
    // One path of all operands' contours, filled even-odd, as before. A safety failure rejects
    // the whole outline: a partial add/subtract shape could silently change its meaning.
    let mut path = Path::default();
    // Charge every slot before filtering nulls or malformed values. Stream::objs would scan
    // and allocate the whole list first, repeating uncharged work for each shared occurrence.
    let children = if let Some(Value::Array(children)) = s.field(id, b"Chld") { children.as_slice() } else { &[] };
    for child in children {
        r.geometry_budget.visit()?;
        let Value::Obj(Some(c)) = child else { continue };
        let c = *c;
        let local = s.floats::<6>(c, b"Xfrm").map(Affine::from_xfrm).unwrap_or(Affine::IDENTITY);
        let Some(class) = s.class(c) else { return Ok(None) };
        if s.enumeration(c, b"ComO").is_some_and(|(op, _)| op != 0 && op != 2) {
            r.warn("compound shapes that intersect or exclude (imported as add)");
        }
        if let Some((p, _)) = outline_inner(r, c, class, local.then(world), active)? {
            r.geometry_budget.path(&p)?;
            path.subpaths.extend(p.transformed(local).subpaths);
        }
    }
    Ok(Some((path, true)))
}

fn leaf_outline(r: &mut Reader, id: ObjId, class: Tag, world: Affine) -> Option<(Path, bool)> {
    let s = r.s;
    match &class.0.to_be_bytes() {
        b"PCrv" | b"TxtC" => {
            let crvs = s.obj(id, b"Crvs").or_else(|| s.obj(id, b"MCrM").and_then(|m| s.obj(m, b"Crvs")))?;
            let path = curves(r, crvs)?;
            let multi = path.subpaths.len() > 1;
            Some((path, multi))
        }
        _ => {
            let (shape, box_owner) = match s.obj(id, b"Shpe") {
                Some(sh) => (sh, id),
                None => {
                    let master = s.obj(id, b"ShpM")?;
                    (s.obj(master, b"Shpe")?, master)
                }
            };
            let b = s.floats::<4>(box_owner, b"ShpB")?;
            crate::shapes::shape(r, shape, b, world)
        }
    }
}

/// `PCvD.Data`: positional `[u8, u32 count, then per subpath: bool closed, 18-byte records]`.
/// A record is (f64 x, f64 y, u8 kind, u8 role): role 0 is an anchor, 1 the out-handle of the
/// anchor before it, 2 the in-handle of the anchor after it. A closed subpath repeats its first
/// anchor at the end. `CnrD` live corners are rounded on the stored polyline.
pub(crate) fn curves(r: &mut Reader, crvs: ObjId) -> Option<Path> {
    let s = r.s;
    let data = s.obj(crvs, b"Data")?;
    let mut fields = s.positional(data);
    fields.next()?;
    let count = match fields.next()? {
        Value::UInt(n) => usize::try_from(*n).ok()?,
        _ => return None,
    };
    let corners = s.obj(crvs, b"CnrD").and_then(|c| corner_radii(s.positional(c).collect()));
    let mut path = Path::default();
    let mut seen = 0usize;
    for index in 0..count {
        let closed = match fields.next()? {
            Value::Bool(b) => *b,
            _ => return None,
        };
        let Value::Array(records) = fields.next()? else { return None };
        seen = seen.checked_add(records.len())?;
        if seen > MAX_NODES {
            r.warn("curves with more than four million nodes");
            return None;
        }
        if let Err(reason) = r.geometry_budget.records(records.len()) {
            r.warn(reason);
            return None;
        }
        let anchors = anchors(records)?;
        let radii = if index == 0 { corners.as_ref() } else { None };
        if let Some(sp) = subpath(&anchors, closed, radii) {
            path.subpaths.push(sp);
        }
    }
    if corners.is_some() && count > 1 {
        r.warn("rounded corners on curves with several subpaths (only the first is rounded)");
    }
    Some(path)
}

struct Anchor {
    p: Point,
    inh: Option<Point>,
    out: Option<Point>,
    /// Index of the anchor's record (live corners refer to records).
    record: usize,
}

fn record(v: &Value) -> Option<(Point, u8)> {
    let Value::Bytes(b) = v else { return None };
    if b.len() != 18 {
        return None;
    }
    let x = f64::from_le_bytes(b.get(0..8)?.try_into().ok()?);
    let y = f64::from_le_bytes(b.get(8..16)?.try_into().ok()?);
    if !x.is_finite() || !y.is_finite() {
        return None;
    }
    Some((Point { x, y }, *b.get(17)?))
}

fn anchors(records: &[Value]) -> Option<Vec<Anchor>> {
    let mut out: Vec<Anchor> = Vec::with_capacity(records.len() / 3 + 1);
    let mut pending_in = None;
    for (i, v) in records.iter().enumerate() {
        let (p, role) = record(v)?;
        match role {
            0 => {
                out.push(Anchor { p, inh: pending_in.take(), out: None, record: i });
            }
            1 => {
                // A leading out-handle without an anchor is edit state only.
                if let Some(a) = out.last_mut() {
                    a.out = Some(p);
                }
            }
            2 => pending_in = Some(p),
            _ => return None,
        }
    }
    Some(out)
}

fn subpath(anchors: &[Anchor], closed: bool, radii: Option<&Vec<(usize, f64)>>) -> Option<SubPath> {
    let first = anchors.first()?;
    if let Some(radii) = radii.filter(|r| !r.is_empty()) {
        // Index each record's maximum radius once. Scanning the whole radius list per anchor
        // was O(anchors * radii), which a file with large anchor and radius arrays could drive
        // into quadratic work. Duplicate record indices keep the maximum, matching the prior
        // fold(0.0, f64::max) behavior.
        let mut radius_by_record: std::collections::HashMap<usize, f64> = std::collections::HashMap::with_capacity(radii.len());
        for &(rec, r) in radii {
            let slot = radius_by_record.entry(rec).or_insert(0.0);
            *slot = slot.max(r);
        }
        let corner = |a: &Anchor| radius_by_record.get(&a.record).copied().unwrap_or(0.0);
        let polyline = anchors.iter().all(|a| a.inh.is_none_or(|h| h == a.p) && a.out.is_none_or(|h| h == a.p));
        if polyline {
            let mut pts: Vec<(Point, f64)> = anchors.iter().map(|a| (a.p, corner(a))).collect();
            // A closed subpath repeats its first anchor; its radius may be on either copy.
            if closed
                && pts.len() > 1
                && pts.first().map(|p| p.0) == pts.last().map(|p| p.0)
                && let Some((_, r)) = pts.pop()
                && let Some(f) = pts.first_mut()
            {
                f.1 = f.1.max(r);
            }
            return Some(crate::shapes::fillet(&pts, closed));
        }
    }
    let segments = anchors
        .windows(2)
        .filter_map(|w| {
            let [a, b] = w else { return None };
            Some([a.out.unwrap_or(a.p), b.inh.unwrap_or(b.p), b.p])
        })
        .collect();
    Some(SubPath { start: first.p, segments, closed })
}

/// `CnrD`: positional `[u8, i32[] record indices, f64[] radii, u8[] types]`.
fn corner_radii(fields: Vec<&Value>) -> Option<Vec<(usize, f64)>> {
    let (Some(Value::Array(idx)), Some(Value::Array(rad))) = (fields.get(1), fields.get(2)) else { return None };
    Some(
        idx.iter()
            .zip(rad)
            .filter_map(|(i, r)| match (i, r) {
                (Value::Int(i), Value::Float(r)) if r.is_finite() && *r > 0.0 => Some((usize::try_from(*i).ok()?, *r)),
                _ => None,
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn curve_records_are_charged_across_the_document() {
        let mut b = Budget::default();
        assert!(b.records(MAX_OUTLINE_RECORDS).is_ok());
        assert_eq!(b.records(1), Err("curve geometry budget exhausted"));
    }
}
