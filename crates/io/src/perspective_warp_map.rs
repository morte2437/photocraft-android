//! Perspective Warp smart filters (`Fltr` class `perspectiveWarpTransform`).
//!
//! Layout as Photoshop writes it (checked by applying Edit › Perspective Warp to a smart object
//! through the Action Manager and saving the PSD):
//!
//! ```text
//! filterFX: Nm, blendOptions, enab, hasoptions (true), FrgC, BckC, filterID (1023 observed;
//!   other versions write other ids), Fltr = perspectiveWarpTransform {
//!     null = reference to the target layer,
//!     referenceRect = Rctn { Top, Left, Btom, Rght } (UntF #Pxl, the layer bounds),
//!     vertices = [ Pnt { Hrzn, Vrtc } (UntF #Pxl, document pixels) ],      (Layout mode)
//!     warpedVertices = [ Pnt { Hrzn, Vrtc } ] (same count),                 (Warp mode)
//!     quads = [ perspectiveWarpQuad { indices = [4 × long] } ],
//!     PuX0..PuX3, PuY0..PuY3 (doubles: the placement corners when the filter was applied) }
//! ```
//!
//! Each quad is one plane of `layer.smartObjects.perspectiveWarp`: `src` = the quad's
//! `vertices`, `dst` = its `warpedVertices`, in the stored corner order. Quads that share a vertex
//! share the corner, which the engine links. The item is kept as a template ([`TEMPLATE_KEY`]) so
//! the keys PhotoCraft does not model go back unchanged, with only the vertices and quads
//! rewritten from the edited planes. Anything inconsistent (an index out of range, mismatched
//! vertex counts, a non-finite or degenerate quad) stays an opaque [`super::UNSUPPORTED_FILTER`].

use photocraft_algo::perspective::{PerspectiveMap, Plane, Straighten, straighten};
use photocraft_doc::SmartFilter;
use photocraft_psd::descriptor::{Class, Descriptor, Id, ReferenceItem, UnicodeString, Value};
use serde_json::{Value as J, json};

use super::{blend_options, from_hex, get_desc, rgbc, set, to_hex, unit};

pub(super) const CLASS: &str = "perspectiveWarpTransform";
/// The command this filter maps to (`photocraft_engine::distort_cmds::PERSPECTIVE_SMART`).
pub(super) const COMMAND: &str = "layer.smartObjects.perspectiveWarp";
/// Edit › Perspective Warp on a smart object stores its own id; it renders the same.
const EDIT_COMMAND: &str = "edit.perspectiveWarp";
/// The `filterID` Photoshop wrote for a new Perspective Warp smart filter.
const FILTER_ID: i32 = 1023;
const NAME: &str = "Perspective Warp...";
pub(super) const TEMPLATE_KEY: &str = "__perspectiveWarpPsd";
/// Far more planes than anyone lays out by hand; bounds the corner linking (quadratic).
const MAX_QUADS: usize = 256;
const MAX_VERTICES: usize = 4 * MAX_QUADS;
const MAX_TEMPLATE_BYTES: usize = 1 << 20;

/// Whether `command` is a Perspective Warp smart filter this module writes.
pub(super) fn handles(command: &str) -> bool {
    command == COMMAND || command == EDIT_COMMAND
}

fn point(v: &Value) -> Option<[f64; 2]> {
    let Value::Descriptor(p) = v else { return None };
    let coord = |k: &str| match p.get(k)? {
        Value::Double(d) => Some(*d),
        Value::UnitFloat { unit: [b'#', b'P', b'x', b'l'], value } => Some(*value),
        _ => None,
    };
    Some([coord("Hrzn")?, coord("Vrtc")?]).filter(|q| q.iter().all(|v| v.is_finite()))
}

fn points(f: &Descriptor, key: &str) -> Option<Vec<[f64; 2]>> {
    let Some(Value::List(list)) = f.get(key) else { return None };
    if list.is_empty() || list.len() > MAX_VERTICES {
        return None;
    }
    list.iter().map(point).collect()
}

fn quads(f: &Descriptor) -> Option<Vec<[usize; 4]>> {
    let Some(Value::List(list)) = f.get("quads") else { return None };
    if list.is_empty() || list.len() > MAX_QUADS {
        return None;
    }
    list.iter()
        .map(|q| {
            let Value::Descriptor(q) = q else { return None };
            let Some(Value::List(ix)) = q.get("indices") else { return None };
            let ix: Vec<usize> = ix
                .iter()
                .map(|i| match i {
                    Value::Integer(i) => usize::try_from(*i).ok(),
                    _ => None,
                })
                .collect::<Option<_>>()?;
            ix.try_into().ok()
        })
        .collect()
}

/// The planes a `perspectiveWarpTransform` descriptor describes; `None` when it is inconsistent.
fn planes_of(f: &Descriptor) -> Option<Vec<Plane>> {
    let (src, dst) = (points(f, "vertices")?, points(f, "warpedVertices")?);
    if src.len() != dst.len() {
        return None;
    }
    let corners =
        |list: &[[f64; 2]], q: &[usize; 4]| -> Option<[[f64; 2]; 4]> { Some([*list.get(q[0])?, *list.get(q[1])?, *list.get(q[2])?, *list.get(q[3])?]) };
    let planes: Vec<Plane> = quads(f)?.iter().map(|q| Some(Plane { src: corners(&src, q)?, dst: corners(&dst, q)? })).collect::<Option<_>>()?;
    PerspectiveMap::new(&planes)?;
    Some(planes)
}

/// `layer.smartObjects.perspectiveWarp` params for a Perspective Warp item.
pub(super) fn import_params(item: &Descriptor) -> Option<J> {
    let f = get_desc(item, "Fltr")?;
    if !f.class_id.is(CLASS) {
        return None;
    }
    let planes = planes_of(f)?;
    // Neutral blending in the template: the model's own fields replace it on export.
    let mut template = item.clone();
    let neutral = SmartFilter { command: String::new(), params: J::Null, blend: photocraft_color::BlendMode::Normal, opacity: 1.0, visible: true };
    set(&mut template, "blendOptions", blend_options(&neutral));
    set(&mut template, "enab", Value::Boolean(true));
    // Export refuses a larger template; such an item stays opaque, so it is still written back.
    let bytes = template.to_bytes();
    if bytes.len() > MAX_TEMPLATE_BYTES {
        return None;
    }
    Some(json!({"planes": planes, TEMPLATE_KEY: to_hex(&bytes)}))
}

/// The template item of `params` (`None` when there is none), checked to be a Perspective Warp.
fn template(params: &J) -> Result<Option<Descriptor>, String> {
    let Some(raw) = params.get(TEMPLATE_KEY) else { return Ok(None) };
    let raw = raw.as_str().ok_or("the Perspective Warp PSD data must be hex text")?;
    if raw.len() > MAX_TEMPLATE_BYTES * 2 {
        return Err("the Perspective Warp PSD data is too large".into());
    }
    let bytes = from_hex(raw).ok_or("the Perspective Warp PSD data is not valid hex")?;
    let item = Descriptor::from_bytes(&bytes).map_err(|e| format!("the Perspective Warp PSD data is unreadable ({e})"))?;
    if !item.class_id.is("filterFX") || !get_desc(&item, "Fltr").is_some_and(|f| f.class_id.is(CLASS)) {
        return Err("the Perspective Warp PSD data is not a Perspective Warp filter".into());
    }
    Ok(Some(item))
}

/// The planes to write: as rendered, so a `straighten` is baked in.
fn export_planes(params: &J) -> Result<Vec<Plane>, String> {
    let object = params.as_object().ok_or("Perspective Warp parameters must be an object")?;
    if let Some(k) = object.keys().find(|k| !super::is_meta(k) && !["planes", "straighten", "interpolation"].contains(&k.as_str())) {
        return Err(format!("Perspective Warp setting `{k}` has no PSD equivalent"));
    }
    let mut planes: Vec<Plane> =
        serde_json::from_value(params.get("planes").cloned().ok_or("it has no planes")?).map_err(|e| format!("its planes are unreadable ({e})"))?;
    if planes.is_empty() || planes.len() > MAX_QUADS {
        return Err(format!("it must have 1 to {MAX_QUADS} planes"));
    }
    if planes.iter().any(|q| q.src.iter().chain(&q.dst).flatten().any(|v| !v.is_finite())) {
        return Err("a plane corner is not finite".into());
    }
    if let Some(s) = params.get("straighten").and_then(J::as_str) {
        straighten(&mut planes, Straighten::parse(s).ok_or_else(|| format!("unknown straighten `{s}`"))?);
    }
    PerspectiveMap::new(&planes).ok_or("a plane is degenerate")?;
    Ok(planes)
}

fn pnt(p: [f64; 2]) -> Value {
    Value::Descriptor(Descriptor::new("Pnt ").with("Hrzn", unit(b"#Pxl", p[0])).with("Vrtc", unit(b"#Pxl", p[1])))
}

/// `vertices`, `warpedVertices` and `quads` for `planes`; a corner shared by several planes (same
/// layout and warped position) is written once.
fn geometry(planes: &[Plane]) -> (Value, Value, Value) {
    let mut corners: Vec<([f64; 2], [f64; 2])> = Vec::new();
    let mut quads = Vec::with_capacity(planes.len());
    for p in planes {
        let mut ix = Vec::with_capacity(4);
        for (s, d) in p.src.iter().zip(&p.dst) {
            let i = corners.iter().position(|c| c == &(*s, *d)).unwrap_or_else(|| {
                corners.push((*s, *d));
                corners.len() - 1
            });
            ix.push(Value::Integer(i32::try_from(i).unwrap_or(i32::MAX)));
        }
        quads.push(Value::Descriptor(Descriptor::new("perspectiveWarpQuad").with("indices", Value::List(ix))));
    }
    let src = corners.iter().map(|c| pnt(c.0)).collect();
    let dst = corners.iter().map(|c| pnt(c.1)).collect();
    (Value::List(src), Value::List(dst), Value::List(quads))
}

/// A new `perspectiveWarpTransform` descriptor (no Photoshop template to start from).
fn fresh_fltr(planes: &[Plane]) -> Descriptor {
    let (mut x0, mut y0, mut x1, mut y1) = (f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY);
    for c in planes.iter().flat_map(|p| p.src) {
        (x0, y0, x1, y1) = (x0.min(c[0]), y0.min(c[1]), x1.max(c[0]), y1.max(c[1]));
    }
    let target = Value::Reference(vec![ReferenceItem::Enumerated {
        class: Class { name: UnicodeString::new_nul(""), class_id: Id::new("Lyr ") },
        type_id: Id::new("Ordn"),
        value: Id::new("Trgt"),
    }]);
    let rect =
        Descriptor::new("Rctn").with("Top ", unit(b"#Pxl", y0)).with("Left", unit(b"#Pxl", x0)).with("Btom", unit(b"#Pxl", y1)).with("Rght", unit(b"#Pxl", x1));
    Descriptor::new(CLASS).with("null", target).with("referenceRect", Value::Descriptor(rect))
}

/// The `filterFXList` item for a Perspective Warp smart filter.
pub(super) fn export_item(sf: &SmartFilter) -> Result<Descriptor, String> {
    let params = &sf.params;
    let planes = export_planes(params)?;
    let template = template(params)?;
    let unchanged = template.as_ref().and_then(|item| get_desc(item, "Fltr")).and_then(planes_of).is_some_and(|p| p == planes);
    let mut item = match template {
        // Unchanged planes: the original item, vertex order and sharing included.
        Some(item) if unchanged => item,
        Some(mut item) => {
            let mut f = get_desc(&item, "Fltr").cloned().ok_or("the Perspective Warp PSD data has no filter")?;
            write_geometry(&mut f, &planes);
            set(&mut item, "Fltr", Value::Descriptor(f));
            item
        }
        None => {
            let mut f = fresh_fltr(&planes);
            write_geometry(&mut f, &planes);
            Descriptor::new("filterFX")
                .with("Nm  ", Value::Text(UnicodeString::new_nul(NAME)))
                .with("blendOptions", blend_options(sf))
                .with("enab", Value::Boolean(sf.visible))
                .with("hasoptions", Value::Boolean(true))
                .with("FrgC", rgbc(0.0))
                .with("BckC", rgbc(255.0))
                .with("Fltr", Value::Descriptor(f))
                .with("filterID", Value::Integer(FILTER_ID))
        }
    };
    set(&mut item, "blendOptions", blend_options(sf));
    set(&mut item, "enab", Value::Boolean(sf.visible));
    Ok(item)
}

fn write_geometry(f: &mut Descriptor, planes: &[Plane]) {
    let (src, dst, quads) = geometry(planes);
    set(f, "vertices", src);
    set(f, "warpedVertices", dst);
    set(f, "quads", quads);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::smart_map::{UNSUPPORTED_FILTER, filter_from_item, int, item_for_filter};

    fn pts(list: &[[f64; 2]]) -> Value {
        Value::List(list.iter().map(|&p| pnt(p)).collect())
    }

    fn quad_list<Q: AsRef<[i32]>>(q: &[Q]) -> Value {
        Value::List(
            q.iter()
                .map(|q| {
                    Value::Descriptor(
                        Descriptor::new("perspectiveWarpQuad").with("indices", Value::List(q.as_ref().iter().map(|&i| Value::Integer(i)).collect())),
                    )
                })
                .collect(),
        )
    }

    /// A `filterFXList` item as Photoshop wrote it for a one-plane Perspective Warp of a smart
    /// object placed at (40, 30)–(160, 120).
    fn item_with<Q: AsRef<[i32]>>(vertices: &[[f64; 2]], warped: &[[f64; 2]], q: &[Q], filter_id: i32) -> Descriptor {
        let mut f = fresh_fltr(&[Plane::identity([[40.0, 30.0], [160.0, 30.0], [160.0, 120.0], [40.0, 120.0]])])
            .with("vertices", pts(vertices))
            .with("warpedVertices", pts(warped))
            .with("quads", quad_list(q));
        for (i, (x, y)) in [(40.0, 30.0), (160.0, 30.0), (160.0, 120.0), (40.0, 120.0)].into_iter().enumerate() {
            f = f.with(&format!("PuX{i}"), Value::Double(x));
            f = f.with(&format!("PuY{i}"), Value::Double(y));
        }
        Descriptor::new("filterFX")
            .with("Nm  ", Value::Text(UnicodeString::new_nul("Perspektivische Verformung...")))
            .with(
                "blendOptions",
                blend_options(&SmartFilter {
                    command: String::new(),
                    params: J::Null,
                    blend: photocraft_color::BlendMode::Normal,
                    opacity: 1.0,
                    visible: true,
                }),
            )
            .with("enab", Value::Boolean(true))
            .with("hasoptions", Value::Boolean(true))
            .with("FrgC", rgbc(0.0))
            .with("BckC", rgbc(255.0))
            .with("Fltr", Value::Descriptor(f))
            .with("filterID", Value::Integer(filter_id))
    }

    const V: [[f64; 2]; 4] = [[40.0, 30.0], [160.0, 30.0], [160.0, 120.0], [40.0, 120.0]];
    const W: [[f64; 2]; 4] = [[60.0, 50.0], [140.0, 40.0], [180.0, 130.0], [20.0, 110.0]];

    fn one_plane() -> Descriptor {
        item_with(&V, &W, &[[0, 1, 2, 3]], FILTER_ID)
    }

    /// Two planes sharing an edge (vertices 1 and 4).
    fn two_planes(filter_id: i32) -> Descriptor {
        let v = [[40.0, 30.0], [100.0, 30.0], [160.0, 30.0], [160.0, 120.0], [100.0, 120.0], [40.0, 120.0]];
        let w = [[40.0, 40.0], [100.0, 30.0], [170.0, 45.0], [150.0, 115.0], [100.0, 125.0], [30.0, 110.0]];
        item_with(&v, &w, &[[0, 1, 4, 5], [1, 2, 3, 4]], filter_id)
    }

    #[test]
    fn a_perspective_warp_opens_as_the_command_with_its_planes() {
        let f = filter_from_item(&one_plane());
        assert_eq!(f.command, COMMAND);
        let planes: Vec<Plane> = serde_json::from_value(f.params["planes"].clone()).unwrap();
        assert_eq!(planes, vec![Plane { src: V, dst: W }]);

        // Several quads become several planes; any filterID (708 in other versions) is read.
        let f = filter_from_item(&two_planes(708));
        assert_eq!(f.command, COMMAND);
        let planes: Vec<Plane> = serde_json::from_value(f.params["planes"].clone()).unwrap();
        assert_eq!(planes.len(), 2);
        assert_eq!(planes[0].src[1], planes[1].src[0], "the shared vertex is the same corner in both planes");
        assert_eq!(planes[1].dst, [[100.0, 30.0], [170.0, 45.0], [150.0, 115.0], [100.0, 125.0]]);
    }

    #[test]
    fn unchanged_filters_write_the_original_item_back() {
        for src in [one_plane(), two_planes(708)] {
            let f = filter_from_item(&src);
            assert_eq!(item_for_filter(&f).unwrap(), src);
        }
    }

    #[test]
    fn edited_planes_round_trip_and_keep_the_rest_of_the_template() {
        let src = two_planes(708);
        let mut f = filter_from_item(&src);
        f.params["planes"][1]["dst"][2] = json!([155.5, 118.0]);
        f.opacity = 0.5;
        f.visible = false;
        let item = item_for_filter(&f).unwrap();
        assert_eq!(int(&item, "filterID"), Some(708), "the original filter id stays");
        let fltr = get_desc(&item, "Fltr").unwrap();
        assert_eq!(fltr.get("PuX1"), Some(&Value::Double(160.0)), "unmodelled keys stay");
        let Some(Value::List(v)) = fltr.get("vertices") else { panic!() };
        assert_eq!(v.len(), 6, "shared corners stay shared");
        let back = filter_from_item(&item);
        assert_eq!(back.command, COMMAND);
        assert_eq!(back.params["planes"], f.params["planes"]);
        assert_eq!((back.opacity, back.visible), (0.5, false));
    }

    #[test]
    fn a_new_perspective_warp_writes_photoshops_layout() {
        let planes = vec![Plane { src: V, dst: W }];
        let f = SmartFilter {
            command: COMMAND.into(),
            params: json!({"planes": planes}),
            blend: photocraft_color::BlendMode::Multiply,
            opacity: 0.25,
            visible: true,
        };
        let item = item_for_filter(&f).unwrap();
        assert_eq!(int(&item, "filterID"), Some(FILTER_ID));
        assert_eq!(item.get("hasoptions"), Some(&Value::Boolean(true)));
        let fltr = get_desc(&item, "Fltr").unwrap();
        assert!(fltr.class_id.is(CLASS));
        assert_eq!(fltr.get("vertices"), Some(&pts(&V)));
        assert_eq!(fltr.get("warpedVertices"), Some(&pts(&W)));
        assert_eq!(fltr.get("quads"), Some(&quad_list(&[[0, 1, 2, 3]])));
        let rect = get_desc(fltr, "referenceRect").unwrap();
        assert_eq!(rect.get("Rght"), Some(&unit(b"#Pxl", 160.0)));
        let back = filter_from_item(&Descriptor::from_bytes(&item.to_bytes()).unwrap());
        assert_eq!((back.command.as_str(), &back.params["planes"]), (COMMAND, &json!(planes)));
        assert_eq!((back.blend, back.opacity), (photocraft_color::BlendMode::Multiply, 0.25));

        // Edit › Perspective Warp on a smart object stores its own id; it is written too, with a
        // straighten baked in.
        let f = SmartFilter { command: EDIT_COMMAND.into(), params: json!({"planes": planes, "straighten": "auto", "interpolation": "bilinear"}), ..f };
        assert!(item_for_filter(&f).is_ok());
    }

    #[test]
    fn inconsistent_perspective_warps_stay_opaque() {
        let nan = [[f64::NAN, 30.0], V[1], V[2], V[3]];
        let flat = [[0.0, 0.0]; 4];
        let without = |key: &str| {
            let mut i = one_plane();
            let mut f = get_desc(&i, "Fltr").unwrap().clone();
            f.items.retain(|(k, _)| !k.is(key));
            set(&mut i, "Fltr", Value::Descriptor(f));
            i
        };
        let mut bad_unit = one_plane();
        let mut f = get_desc(&bad_unit, "Fltr").unwrap().clone();
        let Some(Value::List(l)) = f.get("vertices").cloned() else { panic!() };
        let mut l = l;
        l[0] = Value::Descriptor(Descriptor::new("Pnt ").with("Hrzn", unit(b"#Prc", 40.0)).with("Vrtc", unit(b"#Prc", 30.0)));
        set(&mut f, "vertices", Value::List(l));
        set(&mut bad_unit, "Fltr", Value::Descriptor(f));
        let too_many: Vec<[i32; 4]> = vec![[0, 1, 2, 3]; MAX_QUADS + 1];
        // Valid planes, but a template export would refuse as too large.
        let mut huge = one_plane();
        huge.items.push((Id::new("bigData"), Value::RawData(vec![0; MAX_TEMPLATE_BYTES])));
        for (i, src) in [
            item_with(&V, &W, &[[0, 1, 2, 4]], FILTER_ID),
            item_with(&V, &W, &[[0, 1, 2, -1]], FILTER_ID),
            item_with(&V, &W, &[[0, 1, 2]], FILTER_ID),
            item_with(&V, &W, &[[0, 1, 2, 3, 0]], FILTER_ID),
            item_with(&V, &W, &[] as &[[i32; 4]], FILTER_ID),
            item_with(&V, &W, &too_many, FILTER_ID),
            item_with(&V, &W[..3], &[[0, 1, 2, 3]], FILTER_ID),
            item_with(&nan, &W, &[[0, 1, 2, 3]], FILTER_ID),
            item_with(&flat, &W, &[[0, 1, 2, 3]], FILTER_ID),
            item_with(&[], &[], &[[0, 1, 2, 3]], FILTER_ID),
            without("vertices"),
            without("warpedVertices"),
            without("quads"),
            bad_unit,
            huge,
        ]
        .into_iter()
        .enumerate()
        {
            let f = filter_from_item(&src);
            assert_eq!(f.command, UNSUPPORTED_FILTER, "case {i}");
            // Bytes, as NaN is not equal to itself.
            assert_eq!(item_for_filter(&f).unwrap().to_bytes(), src.to_bytes());
        }
    }

    #[test]
    fn bad_exports_report_errors() {
        let ok = filter_from_item(&one_plane()).params;
        let with = |k: &str, v: J| {
            let mut p = ok.clone();
            p[k] = v;
            p
        };
        let plane = json!([{"src": V, "dst": W}]);
        for p in [
            J::Null,
            json!({}),
            json!({"planes": []}),
            json!({"planes": "x"}),
            json!({"planes": [{"src": [[0, 0], [0, 0], [0, 0], [0, 0]], "dst": W}]}),
            json!({"planes": plane, "straighten": "sideways"}),
            json!({"planes": plane, "pins": []}),
            json!({"planes": vec![json!({"src": V, "dst": W}); MAX_QUADS + 1]}),
            with(TEMPLATE_KEY, json!("zz")),
            with(TEMPLATE_KEY, json!(7)),
            with(TEMPLATE_KEY, json!(to_hex(&Descriptor::new("filterFX").to_bytes()))),
            with(TEMPLATE_KEY, json!("0".repeat(MAX_TEMPLATE_BYTES * 2 + 2))),
        ] {
            let f = SmartFilter { command: COMMAND.into(), params: p.clone(), blend: photocraft_color::BlendMode::Normal, opacity: 1.0, visible: true };
            assert!(export_item(&f).is_err(), "{p}");
        }
    }
}
