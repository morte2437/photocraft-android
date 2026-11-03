use super::export::{set, set_before, unit};
use super::{Affine, BezierMesh, Descriptor, Id, ObjectArray, UnicodeString, Value, VersionedDescriptor, Warp, WarpStyle, get_desc, num, text_of};

/// Everything needed to write the placed-layer blocks of a smart object.
#[derive(Clone, Debug)]
pub struct PlacedSpec<'a> {
    pub idnt: &'a str,
    pub placed: &'a str,
    /// Source pixels → document pixels.
    pub transform: Affine,
    /// Distort / Perspective: the full projective map (row-major 3×3), overriding `transform`.
    pub perspective: Option<[f64; 9]>,
    /// Source size in pixels.
    pub size: (f64, f64),
    /// Source resolution.
    pub dpi: f64,
    pub warp: Option<&'a Warp>,
    /// `filterFX`, when the smart object has filters.
    pub filter_fx: Option<Descriptor>,
}

/// Where the frame corners land (top-left, top-right, bottom-right, bottom-left), as x, y pairs.
/// The frame is [`crate::blocks::placed_frame`] of `warp`, the warp the block stores, for an
/// affine placement (the inverse of `parse_smart`), [`crate::blocks::perspective_frame`] of it
/// for a projective one (as `parse_smart_perspective` reads it).
fn quad_points(s: &PlacedSpec<'_>, warp: Option<&Warp>) -> [f64; 8] {
    let (w, h) = s.size;
    let [a, b, c, d, e, f] = s.transform.m;
    let map = |x: f64, y: f64| match &s.perspective {
        Some(p) => photocraft_algo::transform::Homography(*p).apply(x, y),
        None => (a * x + c * y + e, b * x + d * y + f),
    };
    let [x0, y0, x1, y1] = if s.perspective.is_some() { crate::blocks::perspective_frame(warp, w, h) } else { crate::blocks::placed_frame(warp, w, h) };
    let mut out = [0.0; 8];
    for (i, (x, y)) in [(x0, y0), (x1, y0), (x1, y1), (x0, y1)].into_iter().enumerate() {
        (out[2 * i], out[2 * i + 1]) = map(x, y);
    }
    out
}

/// The transform quad: the source corners (top-left, top-right, bottom-right, bottom-left).
fn quad(s: &PlacedSpec<'_>, warp: Option<&Warp>) -> Value {
    Value::List(quad_points(s, warp).into_iter().map(Value::Double).collect())
}

/// Has the placement moved from the quad a template descriptor stores (`Trnf`)?
pub(super) fn quad_moved(template: &Descriptor, s: &PlacedSpec<'_>, warp: Option<&Warp>) -> bool {
    let Some(Value::List(pts)) = template.get("Trnf") else { return true };
    let old: Vec<f64> = pts.iter().filter_map(|v| crate::blocks::num(Some(v))).collect();
    let new = quad_points(s, warp);
    old.len() != 8 || old.iter().zip(new).any(|(a, b)| (a - b).abs() > 1e-6 * (1.0 + a.abs()))
}

pub(crate) fn enumv(t: &str, v: &str) -> Value {
    Value::Enumerated { type_id: Id::new(t), value: Id::new(v) }
}

/// The mesh of a split warp: a custom warp of more than one patch, which `SoLd` stores as
/// `quiltWarp` and `PlLd` can't store.
fn split_mesh(w: Option<&Warp>) -> Option<&BezierMesh> {
    w.filter(|w| w.style == WarpStyle::Custom).and_then(|w| w.mesh.as_ref()).filter(|m| m.points.len() != 16 && m.is_valid())
}

/// The warp `SoLd` stores: none, with a message, for a split warp of more patches along a side
/// than a `quiltWarp` is read with, so its corners are framed by the source rect as on reopen.
fn sold_warp<'a>(w: Option<&'a Warp>, warnings: &mut Vec<String>) -> Option<&'a Warp> {
    let too_big = split_mesh(w).is_some_and(|m| m.us.len().max(m.vs.len()) > crate::blocks::QUILT_MAX_PATCHES + 1);
    if too_big {
        warnings.push(format!("a split warp of more than {} patches along a side was written to PSD without its warp", crate::blocks::QUILT_MAX_PATCHES));
        return None;
    }
    w
}

/// Style, bend, distortions, axis, bounds and order: the keys a `warp` and a `quiltWarp`
/// descriptor share, in Photoshop's order.
fn warp_head(class: &str, w: &Warp) -> Descriptor {
    let [x0, y0, x1, y1] = w.bounds;
    let bounds = Descriptor::new("classFloatRect")
        .with("Top ", Value::Double(y0))
        .with("Left", Value::Double(x0))
        .with("Btom", Value::Double(y1))
        .with("Rght", Value::Double(x1));
    let d = Descriptor::new(class).with("warpStyle", enumv("warpStyle", w.style.psd_name())).with("warpValue", Value::Double(w.bend));
    // A cylinder is stored as its seven `warpValues` (#4112).
    let d = if w.style == WarpStyle::Cylinder { d.with("warpValues", Value::List(w.values.iter().copied().map(Value::Double).collect())) } else { d };
    d.with("warpPerspective", Value::Double(w.h_distort))
        .with("warpPerspectiveOther", Value::Double(w.v_distort))
        .with("warpRotate", enumv("Ornt", if w.vertical { "Vrtc" } else { "Hrzn" }))
        .with("bounds", Value::Descriptor(bounds))
        .with("uOrder", Value::Integer(4))
        .with("vOrder", Value::Integer(4))
}

/// The control points of a custom warp: an object array of their x and y.
fn mesh_points(m: &BezierMesh) -> Value {
    let body = Descriptor::new("rationalPoint")
        .with("Hrzn", Value::UnitFloats { unit: *b"#Pxl", values: m.points.iter().map(|p| p[0]).collect() })
        .with("Vrtc", Value::UnitFloats { unit: *b"#Pxl", values: m.points.iter().map(|p| p[1]).collect() });
    Value::ObjectArray(ObjectArray { prefix: m.points.len() as u32, body })
}

/// The `warp` descriptor (also the `PlLd` warp). A split warp is written as no warp: `SoLd`
/// keeps it in `quiltWarp` ([`quilt_desc`]), next to this `warpNone`, as Photoshop does. A
/// custom warp without a usable mesh is written as no warp, with a message. A cylinder is
/// written as its seven `warpValues`.
pub(super) fn warp_desc(w: Option<&Warp>, size: (f64, f64), warnings: &mut Vec<String>) -> Value {
    let none = Warp::none([0.0, 0.0, size.0, size.1]);
    let mut w = w.unwrap_or(&none).clone();
    let mesh = match (&w.style, &w.mesh) {
        (WarpStyle::Custom, Some(m)) if m.points.len() == 16 => Some(m.clone()),
        (WarpStyle::Custom, Some(m)) if m.is_valid() => {
            w = none.clone();
            None
        }
        (WarpStyle::Custom, _) => {
            warnings.push("a custom warp without a valid mesh was written to PSD without its warp".into());
            w = none.clone();
            None
        }
        _ => None,
    };
    let mut d = warp_head("warp", &w);
    if let Some(m) = mesh {
        d = d.with("customEnvelopeWarp", Value::Descriptor(Descriptor::new("customEnvelopeWarp").with("meshPoints", mesh_points(&m))));
    }
    Value::Descriptor(d)
}

/// The `quiltWarp` descriptor of a split warp, in Photoshop's layout: `deformNumCols` ×
/// `deformNumRows` control points, and the patch boundaries (`quiltSliceX` / `quiltSliceY`) at
/// the knots across the warp's box. The imported `quiltWarp` (`old`) gives its own slices where
/// they still bound the mesh's patches, so their units and padding stay Photoshop's.
fn quilt_desc(w: &Warp, mesh: &BezierMesh, old: Option<&Descriptor>) -> Value {
    let old_env = old.and_then(|q| get_desc(q, "customEnvelopeWarp"));
    let [x0, y0, x1, y1] = w.bounds;
    let slices = |key: &str, knots: &[f64], lo: f64, hi: f64| {
        if let Some(env) = old_env
            && crate::blocks::quilt_knots(env, key, knots.len() - 1).as_deref() == Some(knots)
            && let Some(v) = env.get(key)
        {
            return v.clone();
        }
        let (lo, hi) = if (hi - lo).is_finite() && hi > lo { (lo, hi) } else { (0.0, 1.0) };
        let values: Vec<f64> = knots.iter().map(|u| lo + u * (hi - lo)).collect();
        let prefix = values.len() as u32;
        Value::ObjectArray(ObjectArray { prefix, body: Descriptor::new("UntF").with(key, Value::UnitFloats { unit: *b"#Pxl", values }) })
    };
    let envelope = Descriptor::new("customEnvelopeWarp")
        .with("quiltSliceX", slices("quiltSliceX", &mesh.us, x0, x1))
        .with("quiltSliceY", slices("quiltSliceY", &mesh.vs, y0, y1))
        .with("meshPoints", mesh_points(mesh));
    Value::Descriptor(
        warp_head("quiltWarp", w)
            .with("deformNumRows", Value::Integer(mesh.ny() as i32))
            .with("deformNumCols", Value::Integer(mesh.nx() as i32))
            .with("customEnvelopeWarp", Value::Descriptor(envelope)),
    )
}

fn ratio(n: i32, d: i32) -> Value {
    Value::Descriptor(Descriptor::new("null").with("numerator", Value::Integer(n)).with("denominator", Value::Integer(d)))
}

fn pad4(mut v: Vec<u8>) -> Vec<u8> {
    while !v.len().is_multiple_of(4) {
        v.push(0);
    }
    v
}

/// `SoLd` block data. With `template` (the imported descriptor) every key it has is kept and only
/// the placed id (for a duplicate), transform (when it moved), warp and filter stack are rewritten;
/// otherwise a fresh descriptor in Photoshop's key order. A split warp goes in `quiltWarp`.
pub fn sold_bytes(template: Option<&Descriptor>, s: &PlacedSpec<'_>, warnings: &mut Vec<String>) -> Vec<u8> {
    let stored = sold_warp(s.warp, warnings);
    let warp = warp_desc(stored, s.size, warnings);
    let split = stored.zip(split_mesh(stored));
    let d = match template {
        Some(t) => {
            let mut d = t.clone();
            if text_of(t, "placed").as_deref() != Some(s.placed) {
                set(&mut d, "placed", Value::Text(UnicodeString::new_nul(s.placed)));
            }
            if quad_moved(t, s, stored) {
                set(&mut d, "Trnf", quad(s, stored));
                set(&mut d, "nonAffineTransform", quad(s, stored));
            }
            set(&mut d, "warp", warp);
            // `quiltWarp` is read before `warp`, so the imported one stays only while it still
            // says what the layer's warp is (unchanged, or unreadable while the layer has none);
            // otherwise it is rewritten, or dropped for a warp that isn't split.
            let old = get_desc(t, "quiltWarp");
            let kept = old.is_some_and(|q| match crate::blocks::warp_of(q) {
                Some(w) => w.as_ref() == stored,
                None => s.warp.is_none(),
            });
            if !kept {
                match split {
                    Some((w, m)) => set_before(&mut d, "quiltWarp", quilt_desc(w, m, old), "warp"),
                    None => d.items.retain(|(k, _)| !k.is("quiltWarp")),
                }
            }
            match &s.filter_fx {
                Some(fx) => set(&mut d, "filterFX", Value::Descriptor(fx.clone())),
                None => d.items.retain(|(k, _)| !k.is("filterFX")),
            }
            d
        }
        None => {
            let mut d = Descriptor::new("null")
                .with("Idnt", Value::Text(UnicodeString::new_nul(s.idnt)))
                .with("placed", Value::Text(UnicodeString::new_nul(s.placed)))
                .with("PgNm", Value::Integer(1))
                .with("totalPages", Value::Integer(1))
                .with("Crop", Value::Integer(1))
                .with("frameStep", ratio(0, 600))
                .with("duration", ratio(0, 600))
                .with("frameCount", Value::Integer(1))
                .with("Annt", Value::Integer(16))
                .with("Type", Value::Integer(2))
                .with("Trnf", quad(s, stored))
                .with("nonAffineTransform", quad(s, stored));
            if let Some((w, m)) = split {
                d = d.with("quiltWarp", quilt_desc(w, m, None));
            }
            let mut d = d
                .with("warp", warp)
                .with("Sz  ", Value::Descriptor(Descriptor::new("Pnt ").with("Wdth", Value::Double(s.size.0)).with("Hght", Value::Double(s.size.1))))
                .with("Rslt", unit(b"#Rsl", s.dpi));
            if let Some(fx) = &s.filter_fx {
                d = d.with("filterFX", Value::Descriptor(fx.clone()));
            }
            d.with("comp", Value::Integer(-1))
                .with("compInfo", Value::Descriptor(Descriptor::new("null").with("compID", Value::Integer(-1)).with("originalCompID", Value::Integer(-1))))
        }
    };
    let mut out = b"soLD".to_vec();
    out.extend_from_slice(&4u32.to_be_bytes());
    out.extend(VersionedDescriptor::new(d).to_bytes());
    pad4(out)
}

/// `PlLd` block data (`plcL` version 3): the legacy placed-layer record Photoshop writes next to
/// `SoLd`. It has no split warp: it stores none, its corners framed by the source rect.
pub fn plld_bytes(s: &PlacedSpec<'_>, warnings: &mut Vec<String>) -> Vec<u8> {
    let warp = s.warp.filter(|_| split_mesh(s.warp).is_none());
    let mut out = b"plcL".to_vec();
    out.extend_from_slice(&3u32.to_be_bytes());
    let id = s.idnt.as_bytes();
    let n = id.len().min(255);
    out.push(n as u8);
    out.extend_from_slice(&id[..n]);
    for v in [1i32, 1, 16, 2] {
        out.extend_from_slice(&v.to_be_bytes());
    }
    if let Value::List(q) = quad(s, warp) {
        for v in q {
            if let Value::Double(x) = v {
                out.extend_from_slice(&x.to_be_bytes());
            }
        }
    }
    out.extend_from_slice(&0u32.to_be_bytes());
    let Value::Descriptor(w) = warp_desc(warp, s.size, warnings) else { return pad4(out) };
    out.extend(VersionedDescriptor::new(w).to_bytes());
    pad4(out)
}

/// The source size a `soLD` descriptor stores (`Sz  `).
pub fn stored_size(d: &Descriptor) -> Option<(f64, f64)> {
    let sz = get_desc(d, "Sz  ")?;
    Some((num(sz.get("Wdth"))?, num(sz.get("Hght"))?)).filter(|(w, h)| *w > 0.0 && *h > 0.0)
}
