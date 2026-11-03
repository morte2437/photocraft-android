use super::*;

#[test]
fn float_rgb_colors_parse() {
    let d = Descriptor::new("RGBC").with("redFloat", Value::Double(1.0)).with("greenFloat", Value::Double(0.5)).with("blueFloat", Value::Double(0.25));
    let c = color_from_desc(&d).unwrap();
    assert_eq!(c.to_rgb(), [1.0, 0.5, 0.25]);
}

#[test]
fn smart_transform_from_sold() {
    let pts = [10.0, 20.0, 110.0, 20.0, 110.0, 70.0, 10.0, 70.0].map(Value::Double).to_vec();
    let d = Descriptor::new("null")
        .with("Idnt", Value::Text(UnicodeString::new_nul("uuid-1")))
        .with("Trnf", Value::List(pts))
        .with("Sz  ", Value::Descriptor(Descriptor::new("Pnt ").with("Wdth", Value::Double(50.0)).with("Hght", Value::Double(25.0))));
    let mut data = b"soLD".to_vec();
    data.extend_from_slice(&4u32.to_be_bytes());
    data.extend(VersionedDescriptor::new(d).to_bytes());
    data.extend_from_slice(&[0, 0]);
    let (id, a) = parse_smart(b"SoLd", &data);
    assert_eq!(id, "uuid-1");
    assert_eq!(a.m, [2.0, 0.0, 0.0, 2.0, 10.0, 20.0]);
    assert_eq!(parse_smart(b"PlLd", &data).0, "");
    assert_eq!(parse_plld(&data), None, "soLD data is not a plcL record");
}

/// A `PlLd` record (as Photoshop CS2 writes it, with no `SoLd`, #2465) gives its source id and
/// placed corners; with the source size those are the same transform `SoLd` gives.
#[test]
fn plld_id_and_corners() {
    use crate::smart_map::{PlacedSpec, plld_bytes};
    let t = Affine { m: [1.5, 0.25, -0.5, 2.0, 10.0, 20.0] };
    let spec = PlacedSpec { idnt: "uuid-cs2", placed: "", transform: t, perspective: None, size: (40.0, 30.0), dpi: 72.0, warp: None, filter_fx: None };
    let data = plld_bytes(&spec, &mut Vec::new());
    let (id, p) = parse_plld(&data).expect("a plcL record");
    assert_eq!(id, "uuid-cs2");
    assert_eq!(p, [10.0, 20.0, 70.0, 30.0, 55.0, 90.0, -5.0, 80.0]);
    let back = corners_affine(&p, [0.0, 0.0, 40.0, 30.0]).unwrap();
    for (a, b) in back.m.iter().zip(t.m) {
        assert!((a - b).abs() < 1e-12, "{back:?} vs {t:?}");
    }
    assert_eq!(corners_perspective([[p[0], p[1]], [p[2], p[3]], [p[4], p[5]], [p[6], p[7]]], (40.0, 30.0)), None, "affine corners");
    assert_eq!(corners_affine(&p, [0.0, 0.0, 0.0, 30.0]), None);
    assert_eq!(corners_affine(&p, [0.0, 0.0, f64::NAN, 30.0]), None);
    assert_eq!(corners_affine(&p, [5.0, 0.0, 5.0, 30.0]), None, "an empty frame");
    let mut nan = p;
    nan[3] = f64::INFINITY;
    assert_eq!(corners_affine(&nan, [0.0, 0.0, 40.0, 30.0]), None);
    // The placed-layer record is the one block that can be the principal one: PlLd-only
    // layers have no smart transform from `parse_smart`.
    assert_eq!(parse_smart(b"PlLd", &data), (String::new(), Affine::IDENTITY));
}

/// A `PlLd` record's corners frame the same box as `SoLd`'s `Trnf` (#3677): with a custom
/// warp whose control points spill past the source, the control-point box, not the source
/// rect. The CS2 record then places the layer exactly as the equivalent `SoLd` does, with or
/// without the source size (the box needs none).
#[test]
fn plld_corners_frame_a_spilled_custom_warp_like_sold() {
    use crate::smart_map::{PlacedSpec, parse_sold, plld_bytes, sold_bytes};
    use photocraft_geom::warp::{BezierMesh, Warp};
    let bounds = [0.0, 0.0, 50.0, 25.0];
    let mut mesh = BezierMesh::identity(bounds, 1, 1);
    mesh.points[0] = [-10.0, -5.0];
    mesh.points[15] = [60.0, 25.0];
    let warp = Warp::custom(mesh, bounds);
    let t = Affine { m: [2.0, 0.5, -0.25, 2.0, 120.0, 210.0] };
    let spec = PlacedSpec { idnt: "cs2", placed: "p", transform: t, perspective: None, size: (50.0, 25.0), dpi: 72.0, warp: Some(&warp), filter_fx: None };
    let plld = plld_bytes(&spec, &mut Vec::new());
    let sold = sold_bytes(None, &spec, &mut Vec::new());
    let (_, p) = parse_plld(&plld).expect("a plcL record");
    assert_eq!(parse_plld_warp(&plld), PlldWarp::Warp(warp.clone()));
    let from_sold = parse_smart(b"SoLd", &sold).1;
    assert_eq!(parse_sold(&sold).map(|s| s.transform), Some(from_sold));
    let close = |a: Affine, b: Affine| a.m.iter().zip(b.m).all(|(x, y)| (x - y).abs() < 1e-9);
    assert!(close(from_sold, t), "{from_sold:?} vs {t:?}");
    for size in [Some((50.0, 25.0)), None] {
        let (got, perspective) = plld_transform(&p, size, Some(&warp));
        assert!(close(got, from_sold), "size {size:?}: {got:?} vs SoLd {from_sold:?}");
        assert_eq!(perspective, None);
    }
    // Framing the source rect instead would stretch it (the bug #3677 fixed for SoLd).
    let rect = corners_affine(&p, [0.0, 0.0, 50.0, 25.0]).unwrap();
    assert!(!close(rect, t), "the test needs a mesh that spills: {rect:?}");
    // With no warp, or a preset one, the frame is the source rect, as before.
    let plain = PlacedSpec { warp: None, ..spec };
    let (_, q) = parse_plld(&plld_bytes(&plain, &mut Vec::new())).unwrap();
    assert!(close(plld_transform(&q, Some((50.0, 25.0)), None).0, t));
}

/// Malformed `PlLd` data (wrong magic, truncated anywhere, a Pascal length past the end,
/// non-finite corners) reads as `None`, never a panic.
#[test]
fn plld_malformed_is_none() {
    use crate::smart_map::{PlacedSpec, plld_bytes};
    let spec = PlacedSpec { idnt: "id", placed: "", transform: Affine::IDENTITY, perspective: None, size: (4.0, 4.0), dpi: 72.0, warp: None, filter_fx: None };
    let data = plld_bytes(&spec, &mut Vec::new());
    // Magic + version + Pascal "id" + 4 × u32 + 8 × f64: everything after is the warp.
    let end = 4 + 4 + 1 + 2 + 16 + 64;
    assert!(parse_plld(&data[..end]).is_some());
    for n in 0..end {
        assert_eq!(parse_plld(&data[..n]), None, "truncated to {n} bytes");
    }
    let mut bad = data.clone();
    bad[..4].copy_from_slice(b"soLD");
    assert_eq!(parse_plld(&bad), None);
    let mut long = data.clone();
    long[8] = 255; // a Pascal id longer than the record
    assert_eq!(parse_plld(&long[..200.min(long.len())]), None);
    let mut nan = data.clone();
    nan[end - 8..end].copy_from_slice(&f64::NAN.to_be_bytes());
    assert_eq!(parse_plld(&nan), None);
    assert_eq!(parse_plld(&[]), None);
}

/// Photoshop CS2 can warp a smart object; `PlLd` keeps that warp after the corners (a u32
/// warp version, then the versioned `warp` descriptor). It reads back as written, and
/// `warpNone`, an identity warp or a record that stops at its corners mean no warp.
#[test]
fn plld_warps_round_trip() {
    use crate::smart_map::{PlacedSpec, plld_bytes};
    use photocraft_geom::warp::{BezierMesh, Warp, WarpStyle};
    let bounds = [0.0, 0.0, 60.0, 30.0];
    let mut arc = Warp::preset(WarpStyle::Arc, 40.0, bounds);
    arc.vertical = true;
    arc.h_distort = 10.0;
    let mut mesh = BezierMesh::identity(bounds, 1, 1);
    mesh.points[15] = [67.0, 30.0];
    let custom = Warp::custom(mesh, bounds);
    let flat = Warp::preset(WarpStyle::Flag, 0.0, bounds);
    for (w, back) in
        [(Some(&arc), PlldWarp::Warp(arc.clone())), (Some(&custom), PlldWarp::Warp(custom.clone())), (Some(&flat), PlldWarp::None), (None, PlldWarp::None)]
    {
        let t = Affine { m: [0.5, 0.0, 0.0, 0.5, 3.0, 4.0] };
        let spec = PlacedSpec { idnt: "w", placed: "", transform: t, perspective: None, size: (60.0, 30.0), dpi: 72.0, warp: w, filter_fx: None };
        let data = plld_bytes(&spec, &mut Vec::new());
        assert_eq!(parse_plld_warp(&data), back, "{w:?}");
        // The corners are unchanged by the warp that follows them.
        assert_eq!(parse_plld(&data).map(|(_, p)| p[..2].to_vec()), Some(vec![3.0, 4.0]));
    }
    let spec = PlacedSpec { idnt: "id", placed: "", transform: Affine::IDENTITY, perspective: None, size: (4.0, 4.0), dpi: 72.0, warp: None, filter_fx: None };
    let end = 4 + 4 + 1 + 2 + 16 + 64;
    let data = plld_bytes(&spec, &mut Vec::new());
    assert_eq!(parse_plld_warp(&data[..end]), PlldWarp::None, "no warp stored");
    assert_eq!(parse_plld_warp(&[data[..end].to_vec(), vec![0; 3]].concat()), PlldWarp::None, "padding");
}

/// A `PlLd` warp that is stored but can't be read (truncated, an unknown style, a missing
/// bounds rectangle or mesh) is `Unreadable`, so the import keeps the warped pixels.
#[test]
fn plld_unreadable_warps() {
    use crate::smart_map::{PlacedSpec, plld_bytes};
    use photocraft_geom::warp::{Warp, WarpStyle};
    let arc = Warp::preset(WarpStyle::Arc, 40.0, [0.0, 0.0, 4.0, 4.0]);
    let spec =
        PlacedSpec { idnt: "id", placed: "", transform: Affine::IDENTITY, perspective: None, size: (4.0, 4.0), dpi: 72.0, warp: Some(&arc), filter_fx: None };
    let data = plld_bytes(&spec, &mut Vec::new());
    let end = 4 + 4 + 1 + 2 + 16 + 64;
    assert!(matches!(parse_plld_warp(&data), PlldWarp::Warp(_)));
    for n in end + 1..data.len() - 4 {
        if data[end..n].iter().any(|b| *b != 0) {
            assert_eq!(parse_plld_warp(&data[..n]), PlldWarp::Unreadable, "truncated to {n} bytes");
        }
    }
    let with = |d: Descriptor| [data[..end].to_vec(), vec![0; 4], VersionedDescriptor::new(d).to_bytes()].concat();
    let style = |s: &str| Value::Enumerated { type_id: Id::new("warpStyle"), value: Id::new(s) };
    assert_eq!(parse_plld_warp(&with(Descriptor::new("warp").with("warpStyle", style("warpSomethingNew")))), PlldWarp::Unreadable);
    assert_eq!(parse_plld_warp(&with(Descriptor::new("warp").with("warpStyle", style("warpArc")))), PlldWarp::Unreadable, "no bounds");
    assert_eq!(parse_plld_warp(&with(Descriptor::new("warp").with("warpStyle", style("warpNone")))), PlldWarp::None);
    let bounds = Descriptor::new("classFloatRect")
        .with("Top ", Value::Double(0.0))
        .with("Left", Value::Double(0.0))
        .with("Btom", Value::Double(4.0))
        .with("Rght", Value::Double(4.0));
    let custom = Descriptor::new("warp").with("warpStyle", style("warpCustom")).with("bounds", Value::Descriptor(bounds));
    assert_eq!(parse_plld_warp(&with(custom)), PlldWarp::Unreadable, "custom with no mesh");
    assert_eq!(parse_plld_warp(&[data[..end].to_vec(), vec![0, 0, 0, 0, 0, 0, 0, 16, 1, 2]].concat()), PlldWarp::Unreadable);
    assert_eq!(parse_plld_warp(&data[..end - 1]), PlldWarp::Unreadable, "the record itself");
    assert_eq!(parse_plld_warp(&[]), PlldWarp::Unreadable);
}

/// `SoLd` data with a 50 × 25 source, the quad `trnf` and, when given, a custom warp whose
/// 16 control points are `pts`.
fn sold_with_warp(trnf: [f64; 8], pts: Option<&[[f64; 2]]>) -> Vec<u8> {
    use photocraft_psd::descriptor::ObjectArray;
    let e = |t: &str, v: &str| Value::Enumerated { type_id: Id::new(t), value: Id::new(v) };
    let mut d = Descriptor::new("null")
        .with("Idnt", Value::Text(UnicodeString::new_nul("uuid-1")))
        .with("Trnf", Value::List(trnf.map(Value::Double).to_vec()))
        .with("Sz  ", Value::Descriptor(Descriptor::new("Pnt ").with("Wdth", Value::Double(50.0)).with("Hght", Value::Double(25.0))));
    if let Some(pts) = pts {
        let bounds = Descriptor::new("Rctn")
            .with("Top ", Value::Double(0.0))
            .with("Left", Value::Double(0.0))
            .with("Btom", Value::Double(25.0))
            .with("Rght", Value::Double(50.0));
        let mesh = Descriptor::new("null")
            .with("Hrzn", Value::UnitFloats { unit: *b"#Pxl", values: pts.iter().map(|p| p[0]).collect() })
            .with("Vrtc", Value::UnitFloats { unit: *b"#Pxl", values: pts.iter().map(|p| p[1]).collect() });
        let warp = Descriptor::new("warp").with("warpStyle", e("warpStyle", "warpCustom")).with("bounds", Value::Descriptor(bounds)).with(
            "customEnvelopeWarp",
            Value::Descriptor(Descriptor::new("customEnvelopeWarp").with("meshPoints", Value::ObjectArray(ObjectArray { prefix: 16, body: mesh }))),
        );
        d = d.with("warp", Value::Descriptor(warp));
    }
    let mut data = b"soLD".to_vec();
    data.extend_from_slice(&4u32.to_be_bytes());
    data.extend(VersionedDescriptor::new(d).to_bytes());
    data
}

/// `SoLd` data with a split (quilt) custom warp, as Photoshop writes it: a `quiltWarp`
/// descriptor with `deformNumCols` × `deformNumRows` control points and the patch boundaries
/// in `quiltSliceX` / `quiltSliceY`. With `plain_warp`, a `warpNone` `warp` key sits next to it.
fn sold_with_quilt(trnf: [f64; 8], cols: f64, rows: f64, sx: &[f64], sy: &[f64], pts: &[[f64; 2]], plain_warp: bool) -> Vec<u8> {
    use photocraft_psd::descriptor::ObjectArray;
    let e = |t: &str, v: &str| Value::Enumerated { type_id: Id::new(t), value: Id::new(v) };
    let bounds = || {
        Descriptor::new("Rctn")
            .with("Top ", Value::Double(0.0))
            .with("Left", Value::Double(0.0))
            .with("Btom", Value::Double(25.0))
            .with("Rght", Value::Double(50.0))
    };
    let slices = |k: &str, v: &[f64]| {
        let body = Descriptor::new("null").with(k, Value::UnitFloats { unit: *b"#Pxl", values: v.to_vec() });
        Value::ObjectArray(ObjectArray { prefix: v.len() as u32, body })
    };
    let mesh = Descriptor::new("null")
        .with("Hrzn", Value::UnitFloats { unit: *b"#Pxl", values: pts.iter().map(|p| p[0]).collect() })
        .with("Vrtc", Value::UnitFloats { unit: *b"#Pxl", values: pts.iter().map(|p| p[1]).collect() });
    let envelope = Descriptor::new("customEnvelopeWarp")
        .with("quiltSliceX", slices("quiltSliceX", sx))
        .with("quiltSliceY", slices("quiltSliceY", sy))
        .with("meshPoints", Value::ObjectArray(ObjectArray { prefix: 16, body: mesh }));
    let quilt = Descriptor::new("quiltWarp")
        .with("warpStyle", e("warpStyle", "warpCustom"))
        .with("bounds", Value::Descriptor(bounds()))
        .with("deformNumRows", Value::Integer(rows as i32))
        .with("deformNumCols", Value::Double(cols))
        .with("customEnvelopeWarp", Value::Descriptor(envelope));
    let mut d = Descriptor::new("null")
        .with("Idnt", Value::Text(UnicodeString::new_nul("uuid-1")))
        .with("Trnf", Value::List(trnf.map(Value::Double).to_vec()))
        .with("Sz  ", Value::Descriptor(Descriptor::new("Pnt ").with("Wdth", Value::Double(50.0)).with("Hght", Value::Double(25.0))));
    if plain_warp {
        d = d
            .with("warp", Value::Descriptor(Descriptor::new("warp").with("warpStyle", e("warpStyle", "warpNone")).with("bounds", Value::Descriptor(bounds()))));
    }
    d = d.with("quiltWarp", Value::Descriptor(quilt));
    let mut data = b"soLD".to_vec();
    data.extend_from_slice(&4u32.to_be_bytes());
    data.extend(VersionedDescriptor::new(d).to_bytes());
    data
}

/// A 3 × 2 patch identity grid over the 50 × 25 source with one interior control point pulled.
fn quilt_points() -> Vec<[f64; 2]> {
    let mut pts = photocraft_geom::warp::BezierMesh::identity([0.0, 0.0, 50.0, 25.0], 3, 2).points;
    if let Some(p) = pts.get_mut(10 * 3 + 4) {
        p[1] += 3.0;
    }
    pts
}

/// Photoshop's split warp (`quiltWarp`, mockups' usual warp) reads as a multi-patch mesh: the
/// patch boundaries become the knots, the control points stay in order, and the quilt wins over
/// the `warpNone` `warp` key written next to it. Before, any mesh but 4 × 4 was dropped and the
/// layer re-rendered flat.
#[test]
fn quilt_warp_is_read_as_a_multi_patch_mesh() {
    let quad = [100.0, 200.0, 240.0, 200.0, 240.0, 260.0, 100.0, 260.0];
    let pts = quilt_points();
    // Photoshop pads the outer boundaries a little past the grid's edges.
    let data = sold_with_quilt(quad, 10.0, 7.0, &[-0.5, 12.5, 37.5, 50.5], &[-0.5, 12.5, 25.5], &pts, true);
    let warp = parse_placed_warp(b"SoLd", &data).expect("a quilt warp");
    let mesh = warp.mesh.as_ref().expect("its mesh");
    assert_eq!(mesh.points, pts);
    assert_eq!(mesh.us.len(), 4);
    assert_eq!(mesh.vs.len(), 3);
    assert_eq!((mesh.us[0], mesh.us[3], mesh.vs[0], mesh.vs[2]), (0.0, 1.0, 0.0, 1.0));
    assert!((mesh.us[1] - 13.0 / 51.0).abs() < 1e-12 && (mesh.vs[1] - 13.0 / 26.0).abs() < 1e-12);
    assert!(mesh.is_valid());
}

/// A quilt's control points frame the placed quad as a single patch's do (#3677): a grid
/// spilling to x = -10, y = -5 and x = 60 frames a 70 × 30 box.
#[test]
fn quilt_warp_control_box_frames_the_quad() {
    let mut pts = quilt_points();
    pts[0] = [-10.0, -5.0];
    if let Some(last) = pts.last_mut() {
        *last = [60.0, 25.0];
    }
    let quad = [100.0, 200.0, 240.0, 200.0, 240.0, 260.0, 100.0, 260.0];
    let data = sold_with_quilt(quad, 10.0, 7.0, &[0.0, 12.5, 37.5, 50.0], &[0.0, 12.5, 25.0], &pts, false);
    assert_eq!(parse_smart(b"SoLd", &data).1.m, [2.0, 0.0, 0.0, 2.0, 120.0, 210.0]);
}

/// An inconsistent quilt reads as no warp rather than a wrong one, and never panics.
#[test]
fn inconsistent_quilt_warps_are_not_read() {
    let quad = [100.0, 200.0, 240.0, 200.0, 240.0, 260.0, 100.0, 260.0];
    let pts = quilt_points();
    let (sx, sy) = ([0.0, 12.5, 37.5, 50.0], [0.0, 12.5, 25.0]);
    let read =
        |cols: f64, rows: f64, sx: &[f64], sy: &[f64], pts: &[[f64; 2]]| parse_placed_warp(b"SoLd", &sold_with_quilt(quad, cols, rows, sx, sy, pts, false));
    assert!(read(10.0, 7.0, &sx, &sy, &pts).is_some());
    assert!(read(9.0, 7.0, &sx, &sy, &pts).is_none(), "not three points per patch side plus one");
    assert!(read(13.0, 7.0, &sx, &sy, &pts).is_none(), "more points claimed than stored");
    assert!(read(10.0, 7.0, &[0.0, 12.5, 50.0], &sy, &pts).is_none(), "one slice per patch boundary");
    assert!(read(10.0, 7.0, &[0.0, 37.5, 12.5, 50.0], &sy, &pts).is_none(), "slices must increase");
    assert!(read(10.0, 7.0, &[0.0, f64::NAN, 37.5, 50.0], &sy, &pts).is_none());
    assert!(read(1e9, 7.0, &sx, &sy, &pts).is_none(), "an absurd grid is refused before any arithmetic");
    assert!(read(10.0, 7.0, &sx, &sy, &pts[..pts.len() - 1]).is_none());
    let mut nan = pts.clone();
    nan[3] = [f64::NAN, 0.0];
    assert!(read(10.0, 7.0, &sx, &sy, &nan).is_none());
}

/// Photoshop's `Trnf` is the image of the custom warp's control-point bounding box, not of the
/// source rect: a mesh spilling to x = -10, y = -5 and x = 60 frames a 70 × 30 box.
#[test]
fn smart_transform_maps_the_warp_control_box_onto_the_quad() {
    let mut pts = photocraft_geom::warp::BezierMesh::identity([0.0, 0.0, 50.0, 25.0], 1, 1).points;
    pts[0] = [-10.0, -5.0];
    pts[15] = [60.0, 25.0];
    let quad = [100.0, 200.0, 240.0, 200.0, 240.0, 260.0, 100.0, 260.0];
    let (_, a) = parse_smart(b"SoLd", &sold_with_warp(quad, Some(&pts)));
    assert_eq!(a.m, [2.0, 0.0, 0.0, 2.0, 120.0, 210.0]);
    // Without a warp the source rect is the frame, as before.
    assert_eq!(parse_smart(b"SoLd", &sold_with_warp(quad, None)).1.m, [2.8, 0.0, 0.0, 2.4, 100.0, 200.0]);
}

/// A custom mesh with no area (every control point on one spot) can't frame the quad: the
/// source rect does, with no division by zero.
#[test]
fn smart_transform_with_a_degenerate_warp_box_uses_the_source_rect() {
    let quad = [100.0, 200.0, 240.0, 200.0, 240.0, 260.0, 100.0, 260.0];
    let (_, a) = parse_smart(b"SoLd", &sold_with_warp(quad, Some(&[[7.0, 7.0]; 16])));
    assert_eq!(a.m, [2.8, 0.0, 0.0, 2.4, 100.0, 200.0]);
    let mut warp = photocraft_geom::warp::Warp::preset(photocraft_geom::warp::WarpStyle::Custom, 0.0, [0.0, 0.0, 50.0, 25.0]);
    warp.mesh = Some(photocraft_geom::warp::BezierMesh { us: vec![0.0, 1.0], vs: vec![0.0, 1.0], points: vec![[f64::NAN, 1.0]; 16] });
    assert_eq!(placed_frame(Some(&warp), 50.0, 25.0), [0.0, 0.0, 50.0, 25.0]);
    assert_eq!(placed_frame(None, 50.0, 25.0), [0.0, 0.0, 50.0, 25.0]);
}

/// A `SoLd` placed on `quad` (`Trnf` and `nonAffineTransform`) for a 1000 × 1000 source,
/// warped by `warp`.
fn sold_placed(quad: [f64; 8], warp: Descriptor) -> Vec<u8> {
    let list = Value::List(quad.iter().map(|v| Value::Double(*v)).collect());
    let d = Descriptor::new("null")
        .with("Trnf", list.clone())
        .with("nonAffineTransform", list)
        .with("Sz  ", Value::Descriptor(Descriptor::new("Pnt ").with("Wdth", Value::Double(1000.0)).with("Hght", Value::Double(1000.0))))
        .with("warp", Value::Descriptor(warp));
    let mut data = b"soLD".to_vec();
    data.extend_from_slice(&4u32.to_be_bytes());
    data.extend(VersionedDescriptor::new(d).to_bytes());
    data
}

/// A `warpCylinder` descriptor as #4112 describes it: horizontal, no perspective, seven
/// `warpValues` in the space of `bounds`.
fn cylinder_desc(values: Value) -> Descriptor {
    let e = |t: &str, v: &str| Value::Enumerated { type_id: Id::new(t), value: Id::new(v) };
    let bounds = Descriptor::new("classFloatRect")
        .with("Top ", Value::Double(0.0))
        .with("Left", Value::Double(0.0))
        .with("Btom", Value::Double(1000.0))
        .with("Rght", Value::Double(1000.0));
    Descriptor::new("warp")
        .with("warpStyle", e("warpStyle", "warpCylinder"))
        .with("warpValue", Value::Double(0.0))
        .with("warpValues", values)
        .with("warpPerspective", Value::Double(0.0))
        .with("warpPerspectiveOther", Value::Double(0.0))
        .with("warpRotate", e("Ornt", "Hrzn"))
        .with("bounds", Value::Descriptor(bounds))
}

const CYL: [f64; 7] = [100.0, 800.0, 900.0, 200.0, 0.3, 0.3, 0.5];

fn doubles(v: &[f64]) -> Value {
    Value::List(v.iter().map(|x| Value::Double(*x)).collect())
}

/// A projective placement is framed by the warp `parse_placed_warp` imports: an unreadable
/// `quiltWarp` next to a cylinder under `warp` imports no warp, so the source rect is framed.
#[test]
fn perspective_frame_follows_the_warp_parse_placed_warp_reads() {
    let quad = [10.0, 20.0, 410.0, 60.0, 400.0, 480.0, 30.0, 520.0];
    let list = Value::List(quad.iter().map(|v| Value::Double(*v)).collect());
    let unreadable = Descriptor::new("quiltWarp").with("warpStyle", Value::Enumerated { type_id: Id::new("warpStyle"), value: Id::new("warpSomethingNew") });
    let d = Descriptor::new("null")
        .with("Trnf", list.clone())
        .with("nonAffineTransform", list)
        .with("Sz  ", Value::Descriptor(Descriptor::new("Pnt ").with("Wdth", Value::Double(1000.0)).with("Hght", Value::Double(1000.0))))
        .with("quiltWarp", Value::Descriptor(unreadable))
        .with("warp", Value::Descriptor(cylinder_desc(doubles(&CYL))));
    let mut data = b"soLD".to_vec();
    data.extend_from_slice(&4u32.to_be_bytes());
    data.extend(VersionedDescriptor::new(d).to_bytes());
    assert_eq!(parse_placed_warp(b"SoLd", &data), None);
    let h = photocraft_algo::transform::Homography(parse_smart_perspective(b"SoLd", &data).expect("projective"));
    let (px, py) = h.apply(1000.0, 1000.0);
    assert!((px - 400.0).abs() < 1e-6 && (py - 480.0).abs() < 1e-6, "{px},{py}");
}
/// #4112: a `warpCylinder` placed layer comes in as the cylinder its seven values describe,
/// instead of no warp (which re-rendered the contents flat).
#[test]
fn placed_cylinder_warp_is_read() {
    use photocraft_geom::warp::{Warp, WarpStyle};
    let quad = [0.0, 0.0, 800.0, 0.0, 800.0, 800.0, 0.0, 800.0];
    let want = Warp::cylinder(&CYL, [0.0, 0.0, 1000.0, 1000.0]).unwrap();
    let w = parse_placed_warp(b"SoLd", &sold_placed(quad, cylinder_desc(doubles(&CYL)))).expect("a cylinder");
    assert_eq!(w, want);
    assert_eq!(w.style, WarpStyle::Cylinder);
    // Values written as one unit-float list read the same.
    let floats = Value::UnitFloats { unit: *b"#Pxl", values: CYL.to_vec() };
    assert_eq!(parse_placed_warp(b"SoLE", &sold_placed(quad, cylinder_desc(floats))), Some(want));
}

/// Cylinders nobody has measured (vertical, with perspective) and broken values stay
/// unreadable, as every cylinder was before: the layer keeps the pixels Photoshop stored.
#[test]
fn placed_cylinder_warps_that_cant_be_read() {
    let quad = [0.0, 0.0, 800.0, 0.0, 800.0, 800.0, 0.0, 800.0];
    let read = |d: Descriptor| parse_placed_warp(b"SoLd", &sold_placed(quad, d));
    let e = |t: &str, v: &str| Value::Enumerated { type_id: Id::new(t), value: Id::new(v) };
    let with = |key: &str, v: Value| {
        let mut d = cylinder_desc(doubles(&CYL));
        d.items.iter_mut().filter(|(k, _)| k.is(key)).for_each(|(_, old)| *old = v.clone());
        d
    };
    assert!(read(with("warpRotate", e("Ornt", "Vrtc"))).is_none(), "vertical");
    assert!(read(with("warpPerspective", Value::Double(20.0))).is_none(), "perspective");
    assert!(read(with("warpPerspectiveOther", Value::Double(-5.0))).is_none(), "the other perspective");
    assert!(read(cylinder_desc(doubles(&CYL[..6]))).is_none(), "six values");
    assert!(read(cylinder_desc(Value::List(vec![Value::Text(UnicodeString::new_nul("x")); 7]))).is_none(), "not numbers");
    assert!(read(cylinder_desc(Value::Double(1.0))).is_none(), "one number");
    let mut flipped = CYL;
    flipped.swap(0, 2);
    assert!(read(cylinder_desc(doubles(&flipped))).is_none(), "silhouettes swapped");
    let mut no_values = cylinder_desc(doubles(&CYL));
    no_values.items.retain(|(k, _)| !k.is("warpValues"));
    assert!(read(no_values).is_none(), "no warpValues");
}

/// Like a custom warp's (#3677), a cylinder's control-point box is what the placement quad
/// frames, for affine corners and for projective ones (`nonAffineTransform`).
#[test]
fn placed_cylinder_quad_frames_its_control_box() {
    use photocraft_geom::warp::Warp;
    let cb = Warp::cylinder(&CYL, [0.0, 0.0, 1000.0, 1000.0]).unwrap().mesh.unwrap().control_bounds();
    // Affine: the box lands on the quad.
    let quad = [10.0, 20.0, 410.0, 20.0, 410.0, 520.0, 10.0, 520.0];
    let (_, a) = parse_smart(b"SoLd", &sold_placed(quad, cylinder_desc(doubles(&CYL))));
    let [m0, m1, m2, m3, m4, m5] = a.m;
    let map = |x: f64, y: f64| (m0 * x + m2 * y + m4, m1 * x + m3 * y + m5);
    for ((x, y), (qx, qy)) in
        [(cb[0], cb[1]), (cb[2], cb[1]), (cb[2], cb[3]), (cb[0], cb[3])].into_iter().zip(quad.as_chunks::<2>().0.iter().map(|p| (p[0], p[1])))
    {
        let (px, py) = map(x, y);
        assert!((px - qx).abs() < 1e-9 && (py - qy).abs() < 1e-9, "{x},{y} → {px},{py}, not {qx},{qy}");
    }
    assert!(parse_smart_perspective(b"SoLd", &sold_placed(quad, cylinder_desc(doubles(&CYL)))).is_none(), "affine");
    // Projective: the same box lands on the four corners.
    let quad = [10.0, 20.0, 410.0, 60.0, 400.0, 480.0, 30.0, 520.0];
    let h = photocraft_algo::transform::Homography(parse_smart_perspective(b"SoLd", &sold_placed(quad, cylinder_desc(doubles(&CYL)))).expect("projective"));
    for ((x, y), (qx, qy)) in
        [(cb[0], cb[1]), (cb[2], cb[1]), (cb[2], cb[3]), (cb[0], cb[3])].into_iter().zip(quad.as_chunks::<2>().0.iter().map(|p| (p[0], p[1])))
    {
        let (px, py) = h.apply(x, y);
        assert!((px - qx).abs() < 1e-6 && (py - qy).abs() < 1e-6, "{x},{y} → {px},{py}, not {qx},{qy}");
    }
    // Without a warp a projective quad still frames the source rect.
    let none = Descriptor::new("warp").with("warpStyle", Value::Enumerated { type_id: Id::new("warpStyle"), value: Id::new("warpNone") });
    let h = photocraft_algo::transform::Homography(parse_smart_perspective(b"SoLd", &sold_placed(quad, none)).expect("projective"));
    let (px, py) = h.apply(1000.0, 1000.0);
    assert!((px - 400.0).abs() < 1e-6 && (py - 480.0).abs() < 1e-6);
}

#[test]
fn effects_switch() {
    let mut lfx = vec![0, 0, 0, 0];
    lfx.extend(VersionedDescriptor::new(Descriptor::new("null").with("masterFXSwitch", Value::Boolean(false))).to_bytes());
    assert!(!effects_enabled(&lfx));
    assert!(effects_enabled(&[0, 0, 0, 0]));
}

#[test]
fn tysh_garbage_is_none() {
    assert!(parse_tysh(&[0; 10]).is_none());
}
