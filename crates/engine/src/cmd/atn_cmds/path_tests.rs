use super::super::{Action, AtnAction, AtnEvent, AtnFile, AtnItem, UnicodeString, atn};
use super::*;

/// Construct a pixel point directly from the measured #10706 wire format.
fn wire_point(x: f64, y: f64) -> D {
    D::Descriptor(Descriptor::new("Pnt ").with("Hrzn", D::UnitFloat { unit: *b"#Pxl", value: x }).with("Vrtc", D::UnitFloat { unit: *b"#Pxl", value: y }))
}

/// Encode a fixture corner without cubic handles.
fn corner(x: f64, y: f64) -> D {
    D::Descriptor(Descriptor::new("Pthp").with("Anch", wire_point(x, y)))
}

/// Build the measured outer and inner contours independently of the exporter.
fn fixture(operation: &str, joined: bool, reversed: bool) -> Descriptor {
    let mut outer = vec![corner(50.0, 250.0), corner(50.0, 100.0), corner(350.0, 100.0), corner(350.0, 250.0)];
    if !joined {
        outer[1] = D::Descriptor(
            Descriptor::new("Pthp")
                .with("Anch", wire_point(50.0, 100.0))
                .with("Fwd ", wire_point(50.0, 100.0))
                .with("Bwd ", wire_point(100.0, 20.0))
                .with("Smoo", D::Boolean(false)),
        );
        outer[2] = D::Descriptor(
            Descriptor::new("Pthp")
                .with("Anch", wire_point(350.0, 100.0))
                .with("Fwd ", wire_point(300.0, 20.0))
                .with("Bwd ", wire_point(350.0, 100.0))
                .with("Smoo", D::Boolean(false)),
        );
    }
    let mut inner = vec![corner(150.0, 150.0), corner(250.0, 150.0), corner(250.0, 220.0), corner(150.0, 220.0)];
    if reversed {
        inner.reverse();
    }
    let sub = |points| D::Descriptor(Descriptor::new("Sbpl").with("Clsp", D::Boolean(true)).with("Pts ", D::List(points)));
    let component = |op, subs| {
        D::Descriptor(
            Descriptor::new("PaCm")
                .with("shapeOperation", D::Enumerated { type_id: Id::new("shapeOperation"), value: Id::new(op) })
                .with("SbpL", D::List(subs)),
        )
    };
    let components = if joined {
        vec![component("Add ", vec![sub(outer), sub(inner)])]
    } else {
        vec![component("Add ", vec![sub(outer)]), component(operation, vec![sub(inner)])]
    };
    Descriptor::new(if joined { "null" } else { "setd" })
        .with("null", D::Reference(vec![ReferenceItem::Property { class: class("Path"), key: Id::new("WrPt") }]))
        .with("T   ", D::List(components))
}

/// Extract the single executable Work Path assignment from a measured descriptor.
fn params(d: &Descriptor) -> Value {
    let Mapped::Steps(steps) = from(d) else { panic!("measured path must map") };
    assert_eq!(steps.len(), 1);
    assert_eq!(steps[0].0, "path.set");
    steps[0].1.clone()
}

/// Sample the filled contour, its hole and the untouched exterior independently.
fn check_area(path: &Path) {
    let pixels = photocraft_vector::fill_rasterizer(path, photocraft_vector::DEFAULT_TOLERANCE).render(photocraft_geom::Rect::new(0, 0, 400, 300));
    for (x, y, expected) in [(100, 185, 1.0), (200, 185, 0.0), (10, 20, 0.0)] {
        assert_eq!(pixels[y * 400 + x], expected, "sample {x},{y}");
    }
}

#[test]
fn measured_curves_and_component_operations_round_trip_and_cut_holes() {
    for (operation, expected) in [("Sbtr", PathOp::Subtract), ("xor", PathOp::Exclude)] {
        let d = fixture(operation, false, false);
        let p = params(&d);
        let path = crate::vector_cmds::parse_path(&p["path"]).unwrap();
        assert_eq!(path.fill_rule, FillRule::EvenOdd);
        assert_eq!(path.subpaths[1].op, expected);
        let knots = &path.subpaths[0].knots;
        assert_eq!(knots[1].out_ctrl, Point::new(50.0, 100.0));
        assert_eq!(knots[1].in_ctrl, Point::new(100.0, 20.0));
        assert_eq!(knots[2].out_ctrl, Point::new(300.0, 20.0));
        assert_eq!(knots[2].in_ctrl, Point::new(350.0, 100.0));
        assert_eq!(to(&p), Some(d));
        check_area(&path);
    }
}

#[test]
fn one_component_has_even_odd_holes_with_either_inner_winding() {
    for reversed in [false, true] {
        let d = fixture("Add ", true, reversed);
        let p = params(&d);
        let path = crate::vector_cmds::parse_path(&p["path"]).unwrap();
        assert_eq!(path.components(), vec![0..2]);
        assert_eq!(path.subpaths[1].op, PathOp::Join);
        check_area(&path);
        assert_eq!(to(&p), Some(d));
    }
}

#[test]
fn whole_atn_and_native_insert_path_replay_keep_geometry() {
    let original = fixture("Sbtr", false, false);
    let file = AtnFile {
        actions: vec![AtnAction {
            items: vec![AtnItem { event: AtnEvent::name("set"), descriptor: Some(original.clone()), ..Default::default() }],
            ..Default::default()
        }],
        ..Default::default()
    };
    let imported = super::super::import(&atn::write(&file).unwrap()).unwrap();
    assert!(imported.unsupported.is_empty());
    let (bytes, skipped) = super::super::export("S", &[&imported.actions[0]], &[], false).unwrap();
    assert!(skipped.is_empty());
    assert_eq!(atn::parse(&bytes).unwrap().actions[0].items[0].descriptor, Some(original.clone()));
    let expected = path_from(&original).unwrap();
    let mut s = crate::Session::new();
    s.execute("file.new", json!({"width":400,"height":300})).unwrap();
    s.execute("path.set", json!({"name":"Source","path":crate::vector_cmds::path_json(&expected)})).unwrap();
    s.actions.list.push(Action::new("Captured", Vec::new()));
    let revision = s.active().unwrap().revision;
    s.execute("actions.insertPath", json!({"action":0,"name":"Source"})).unwrap();
    assert_eq!(s.active().unwrap().revision, revision);
    let (bytes, skipped) = super::super::export("S", &[&s.actions.list[0]], &[], false).unwrap();
    assert!(skipped.is_empty());
    let imported = super::super::import(&bytes).unwrap();
    assert!(imported.unsupported.is_empty());
    s.actions.list = imported.actions;
    s.execute("path.set", json!({"name":"Source","path":{"subpaths":[]}})).unwrap();
    let played = s.execute("actions.play", json!({"action":0})).unwrap();
    assert!(played.get("failed").is_none(), "{played}");
    assert_eq!(s.active().unwrap().doc.work_path.as_ref(), Some(&expected));
    check_area(s.active().unwrap().doc.work_path.as_ref().unwrap());
}

#[test]
fn source_wire_identity_is_retained_only_for_unchanged_geometry() {
    let mut d = fixture("xor", false, false);
    d.name = UnicodeString::new_nul("Set");
    d.items.reverse();
    let mut p = params(&d);
    assert_eq!(to(&p).unwrap().to_bytes(), d.to_bytes());
    p["path"]["subpaths"][0]["knots"][0]["anchor"][0] = json!(51.0);
    assert!(to(&p).is_none());
}

#[test]
fn unknown_and_unrepresented_settings_are_not_silently_lost() {
    let base = fixture("Sbtr", false, false);
    let mut empty_components = base.clone();
    let (_, components) = empty_components.items.iter_mut().find(|(key, _)| key.is("T   ")).unwrap();
    *components = D::List(Vec::new());
    for d in [base.clone().with("extra", D::Boolean(true)), empty_components, fixture("Intr", false, false)] {
        assert!(matches!(from(&d), Mapped::Unsupported { harmless: false }));
    }
    let p = params(&base);
    let mut empty = p.clone();
    empty["path"]["subpaths"] = json!([]);
    empty.as_object_mut().unwrap().remove(SOURCE);
    assert!(to(&empty).is_none());
    for key in ["inverted", "fillRule"] {
        let mut changed = p.clone();
        changed["path"][key] = if key == "inverted" { json!(true) } else { json!("nonzero") };
        changed.as_object_mut().unwrap().remove(SOURCE);
        assert!(to(&changed).is_none());
    }
    let mut changed = p.clone();
    changed["path"]["unrepresented"] = json!(1);
    assert!(to(&changed).is_none());
    let mut changed = p;
    changed[SOURCE] = json!("not base64");
    assert!(to(&changed).is_none());
}

#[test]
fn malformed_coordinates_and_oversized_paths_stay_unsupported() {
    fn list(value: &mut D) -> &mut Vec<D> {
        match value {
            D::List(v) => v,
            _ => panic!("fixture list"),
        }
    }
    fn object(value: &mut D) -> &mut Descriptor {
        match value {
            D::Descriptor(v) => v,
            _ => panic!("fixture object"),
        }
    }
    for bad in [D::UnitFloat { unit: *b"#Prc", value: 50.0 }, D::UnitFloat { unit: *b"#Pxl", value: f64::NAN }, D::Double(50.0)] {
        let mut d = fixture("Sbtr", false, false);
        let components = list(&mut d.items[1].1);
        let subs = list(&mut object(&mut components[0]).items[1].1);
        let points = list(&mut object(&mut subs[0]).items[1].1);
        let anchor = object(&mut points[0]);
        let coordinates = object(&mut anchor.items[0].1);
        coordinates.items[0].1 = bad;
        assert!(matches!(from(&d), Mapped::Unsupported { harmless: false }));
    }
    let mut p = params(&fixture("Sbtr", false, false));
    p.as_object_mut().unwrap().remove(SOURCE);
    let knot = p["path"]["subpaths"][0]["knots"][0].clone();
    p["path"]["subpaths"][0]["knots"] = json!(vec![knot.clone(); MAX_PARTS - 4]);
    assert!(to(&p).is_some(), "the existing 4096-knot ceiling remains representable");
    p["path"]["subpaths"][0]["knots"] = json!(vec![knot; MAX_PARTS + 1]);
    assert!(to(&p).is_none());
}

#[test]
fn oversized_descriptor_names_are_rejected_before_retained_wire_serialization() {
    // Each name meets the local limit, but their aggregate exceeds the carrier.
    // Keep this typed input independent of the serializer under test.
    let mut point =
        Descriptor::new("Pnt ").with("Hrzn", D::UnitFloat { unit: *b"#Pxl", value: 50.0 }).with("Vrtc", D::UnitFloat { unit: *b"#Pxl", value: 50.0 });
    point.name = UnicodeString(vec![b'x' as u16; 4096]);
    let mut knot = Descriptor::new("Pthp").with("Anch", D::Descriptor(point));
    knot.name = UnicodeString(vec![b'x' as u16; 4096]);
    let sub = Descriptor::new("Sbpl").with("Clsp", D::Boolean(true)).with("Pts ", D::List(vec![D::Descriptor(knot); 300]));
    let component = Descriptor::new("PaCm")
        .with("shapeOperation", D::Enumerated { type_id: Id::new("shapeOperation"), value: Id::new("Add ") })
        .with("SbpL", D::List(vec![D::Descriptor(sub)]));
    let d = Descriptor::new("setd")
        .with("null", D::Reference(vec![ReferenceItem::Property { class: class("Path"), key: Id::new("WrPt") }]))
        .with("T   ", D::List(vec![D::Descriptor(component)]));
    assert!(path_from(&d).is_none(), "recognition must reject before from() serializes the retained descriptor");
    assert!(matches!(from(&d), Mapped::Unsupported { harmless: false }));
}
