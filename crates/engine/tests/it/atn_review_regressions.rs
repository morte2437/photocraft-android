//! Public-API regressions for the dialog-event and Batch review findings.
use photocraft_doc::LayerContent;
use photocraft_engine::{Session, actions_cmds::Action, atn_cmds};
use photocraft_psd::atn::{self, AtnAction, AtnEvent, AtnFile, AtnItem};
use photocraft_psd::descriptor::{Class, Descriptor, Id, ReferenceItem, UnicodeString, Value as D};
use serde_json::{Value, json};

/// A bounded synthetic document for command and channel comparisons.
fn session(mode: &str) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width":5,"height":5,"mode":mode,"background":"#804020"})).unwrap();
    s
}

/// Encode one real ATN event, preserving its enabled state.
fn encoded(code: [u8; 4], d: Descriptor, enabled: bool) -> Vec<u8> {
    atn::write(&AtnFile {
        actions: vec![AtnAction {
            name: UnicodeString::new_nul("Review"),
            items: vec![AtnItem { event: AtnEvent::Code(code), descriptor: Some(d), enabled, ..Default::default() }],
            ..Default::default()
        }],
        ..Default::default()
    })
    .unwrap()
}

/// The supported UTF-16 path representation, without platform-specific Alias bytes.
fn path(value: &str) -> D {
    let units: Vec<_> = value.encode_utf16().chain(Some(0)).collect();
    let mut bytes = b"txtu".to_vec();
    bytes.extend_from_slice(&((12 + 2 * units.len()) as u32).to_le_bytes());
    bytes.extend_from_slice(&(units.len() as u32).to_le_bytes());
    for unit in units {
        bytes.extend_from_slice(&unit.to_le_bytes());
    }
    D::Path(bytes)
}

/// A named descriptor reference with an empty display-class name.
fn named(class: &str, name: &str) -> ReferenceItem {
    ReferenceItem::Name { class: Class { name: UnicodeString::new_nul(""), class_id: Id::new(class) }, name: UnicodeString::new_nul(name) }
}

/// Recover the first saved descriptor using the public exporter and parser.
fn exported(action: &Action) -> Descriptor {
    let (bytes, skipped) = atn_cmds::export("Parent", &[action], &[], false).unwrap();
    assert!(skipped.is_empty());
    atn::parse(&bytes).unwrap().actions.remove(0).items.remove(0).descriptor.unwrap()
}

/// A three-entry Mesh shaper must affect the input, while an even ramp preserves identity.
#[test]
fn mesh_three_value_shaper_is_applied() {
    for (shaper, expected) in [("0 0 1023", 0.0), ("0 511.5 1023", 0.5)] {
        let mut text = format!("3DMESH\nMesh 1 12\n{shaper}\n");
        for r in [0, 2048, 4095] {
            for g in [0, 2048, 4095] {
                for b in [0, 2048, 4095] {
                    text.push_str(&format!("{r} {g} {b}\n"));
                }
            }
        }
        let lut = photocraft_cms::lutfile::parse_3dl(&text).unwrap();
        let mid = ((lut.size / 2 * lut.size + lut.size / 2) * lut.size + lut.size / 2) * 3;
        assert!((lut.data[mid] - expected).abs() < 0.002, "{shaper}: {}", lut.data[mid]);
    }
}

/// An untouched one-name Batch keeps all original bytes; an edit still receives its enclosing set.
#[test]
fn batch_untouched_reference_and_edited_set_are_distinct() {
    let d = Descriptor::new("Btch").with("null", path("input")).with("T   ", path("output")).with("Usng", D::Reference(vec![named("Actn", "Child")]));
    let imported = atn_cmds::import(&encoded(*b"Btch", d.clone(), true)).unwrap();
    assert!(imported.unsupported.is_empty());
    let mut action = imported.actions[0].clone();
    assert_eq!(exported(&action), d);
    action.steps[0].1["action"] = json!("Edited");
    let edited = exported(&action);
    let D::Reference(names) = edited.get("Usng").unwrap() else { panic!("action reference") };
    assert_eq!(*names, vec![named("Actn", "Edited"), named("ASet", "Parent")]);
}

/// Native Batch's optional false flag must export, but malformed values must not.
#[test]
fn batch_missing_override_save_as_exports_false() {
    let p = json!({"input":"in","output":"out","action":"Child","stopOnError":true,"includeNonImages":true,"namingComponents":[{"component":"name"},{"component":"extension"}]});
    let action = Action::new("Review", vec![("file.automate.batch".into(), p.clone())]);
    assert_eq!(exported(&action).get("Ovrd"), Some(&D::Boolean(false)));
    let mut invalid = p;
    invalid["overrideSaveAs"] = json!(17);
    assert!(atn_cmds::export("Parent", &[&Action::new("Bad", vec![("file.automate.batch".into(), invalid)])], &[], false).is_err());
}

/// Unknown outer Calculations metadata stays a whole disabled step instead of changing on save.
#[test]
fn calculations_outer_descriptor_metadata_is_preserved() {
    let action = Action::new(
        "Calculate",
        vec![("image.calculations".into(), json!({"source1":{"channel":{"color":"red"}},"source2":{"channel":{"color":"red"}},"blending":"add"}))],
    );
    let supported = exported(&action);
    for change_name in [false, true] {
        let mut d = supported.clone();
        if change_name {
            d.name = UnicodeString::new_nul("Outer name");
        } else {
            d.class_id = Id::new("Unrepresented");
        }
        let imported = atn_cmds::import(&encoded(*b"Mk  ", d.clone(), false)).unwrap();
        assert!(!imported.unsupported.is_empty(), "outer metadata was silently admitted");
        let (bytes, _) = atn_cmds::export("Parent", &[&imported.actions[0]], &[], false).unwrap();
        let item = &atn::parse(&bytes).unwrap().actions[0].items[0];
        assert_eq!(item.descriptor, Some(d));
        assert!(!item.enabled);
    }
}

/// Custom export refuses non-object values instead of inventing an identity filter.
#[test]
fn custom_non_object_params_refuse_export() {
    for params in [Value::Null, json!(3), json!(true), json!([])] {
        assert!(atn_cmds::export("Parent", &[&Action::new("Bad", vec![("filter.other.custom".into(), params)])], &[], false).is_err());
    }
    exported(&Action::new("Identity", vec![("filter.other.custom".into(), json!({}))]));
}

/// Cached and cache-less shapes expose identical component and composite Calculations planes.
#[test]
fn calculations_uncached_shape_matches_cached_pixels() {
    for channel in [json!({"color":0}), json!("composite")] {
        let result = |drop_cache| {
            let mut s = session("rgb");
            let id = s.execute("shape.create", json!({"kind":"rect","rect":[1,1,3,3],"fill":"#cc3333"})).unwrap()["layer"].as_u64().unwrap();
            s.edit("Source shape", |doc, _| {
                let fmt = doc.pixel_format();
                let area = doc.bounds();
                let layer = doc.layer_mut(photocraft_doc::LayerId(id)).unwrap();
                layer.name.clear();
                if let LayerContent::Shape(shape) = &mut layer.content {
                    shape.cache = if drop_cache { None } else { Some(photocraft_vector::render_shape(shape, fmt, area)) };
                }
                Ok(())
            })
            .unwrap();
            let source = json!({"layer":{"name":""},"channel":channel});
            s.execute("image.calculations", json!({"source1":source,"source2":source,"blending":"multiply"})).unwrap();
            let doc = &s.active().unwrap().doc;
            doc.channels[0].surface.read_region(doc.bounds())
        };
        assert_eq!(result(false), result(true));
    }
}

/// Targeted Gray and an explicit Gray selector resolve the same single source plane.
#[test]
fn calculations_targeted_gray_matches_explicit_gray() {
    let result = |channel| {
        let mut s = session("gray");
        s.execute("image.calculations", json!({"source1":{"channel":channel},"source2":{"channel":"gray"},"blending":"multiply"})).unwrap();
        let doc = &s.active().unwrap().doc;
        doc.channels[0].surface.read_region(doc.bounds())
    };
    assert_eq!(result(json!("gray")), result(json!({"targeted":true})));
}

/// A recorded visibility target resolves in each input, not against its active layer.
#[test]
fn named_batch_resolves_recorded_layer_targets() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("layers.pcraft");
    let mut source = session("rgb");
    source.execute("layer.new.layer", json!({"name":"Target"})).unwrap();
    source.execute("layer.new.layer", json!({"name":"Active"})).unwrap();
    std::fs::write(&input, photocraft_format::save_to_bytes(&source.active().unwrap().doc, &Default::default()).unwrap()).unwrap();
    let mut s = Session::new();
    s.actions.list.push(Action::new("Hide", vec![("layer.setProps".into(), json!({"visible":false,"_actionLayer":{"name":"Target","ordinal":0}}))]));
    let result = s.execute("file.automate.batch", json!({"input":[input],"output":dir.path().join("out"),"format":"pcraft","action":"Hide"})).unwrap();
    assert_eq!(result["files"].as_array().unwrap().len(), 1, "{result}");
    let saved = photocraft_io::import("saved.pcraft", &std::fs::read(dir.path().join("out/layers.pcraft")).unwrap()).unwrap().document;
    assert!(!saved.walk().iter().find(|(_, _, l)| l.name == "Target").unwrap().2.visible);
    assert!(saved.walk().iter().find(|(_, _, l)| l.name == "Active").unwrap().2.visible);
}

/// A selected multipart EXR import may save elsewhere but must not destroy its original parts.
#[test]
fn batch_save_close_refuses_lossy_exr_parts() {
    use exr::prelude::*;
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("parts.exr");
    let mk = |name: &str, channels: &[(&str, f32)]| {
        Layer::new(
            (2, 2),
            LayerAttributes::named(name),
            Encoding::FAST_LOSSLESS,
            AnyChannels::sort(channels.iter().map(|(name, v)| AnyChannel::new(*name, FlatSamples::F32(vec![*v; 4]))).collect()),
        )
    };
    Image::from_layers(
        ImageAttributes::new(IntegerBounds::new((0, 0), (2, 2))),
        vec![mk("Z", &[("Z", 2.0)]), mk("beauty", &[("R", 0.8), ("G", 0.4), ("B", 0.2), ("A", 1.0)])],
    )
    .write()
    .to_file(&input)
    .unwrap();
    let before = std::fs::read(&input).unwrap();
    let steps = json!([{"command":"file.openExrParts","params":{"path":"recorded.exr","part/beauty":true}}, {"command":"file.saveAs","params":{}}]);
    let mut s = Session::new();
    for override_open in [false, true] {
        let steps = if override_open { steps.clone() } else { json!([]) };
        let result =
            s.execute("file.automate.batch", json!({"input":[input],"destination":"saveAndClose","overrideOpen":override_open,"steps":steps})).unwrap();
        assert_eq!(result["files"].as_array().unwrap().len(), 0, "{result}");
        assert_eq!(result["errors"].as_array().unwrap().len(), 1, "{result}");
        assert!(result["errors"][0]["error"].as_str().unwrap().contains("cannot preserve this EXR"), "{result}");
        assert_eq!(std::fs::read(&input).unwrap(), before, "all original parts must remain byte-identical");
    }
    let result =
        s.execute("file.automate.batch", json!({"input":[input],"output":dir.path().join("out"),"format":"exr","overrideOpen":true,"steps":steps})).unwrap();
    assert_eq!(result["files"].as_array().unwrap().len(), 1, "Save As still permits the selected part: {result}");
    let flat = dir.path().join("flat.exr");
    std::fs::copy(result["files"][0].as_str().unwrap(), &flat).unwrap();
    let result = s.execute("file.automate.batch", json!({"input":[flat],"destination":"saveAndClose","steps":[]})).unwrap();
    assert_eq!(result["files"].as_array().unwrap().len(), 1, "ordinary flat EXR still saves: {result}");
}

/// The largest supported custom kernel is prepared on the worker and can be cancelled without an edit.
#[test]
fn custom_shape_preparation_yields_a_cancellable_job() {
    for smart in [false, true] {
        let mut s = session("rgb");
        if smart {
            s.execute("filter.convertForSmartFilters", json!({})).unwrap();
        }
        let before = photocraft_format::save_to_bytes(&s.active().unwrap().doc, &Default::default()).unwrap();
        assert!(s.execute("filter.blur.shapeBlur", json!({"radius":5,"shapePath":{"subpaths":[]}})).is_err());
        assert_eq!(photocraft_format::save_to_bytes(&s.active().unwrap().doc, &Default::default()).unwrap(), before);
        let knots: Vec<_> = (0..2048).map(|i| json!([if i % 2 == 0 { 0 } else { 1000 }, (i * 617) % 1001])).collect();
        let params = json!({"radius":1000,"shapePath":{"subpaths":[{"closed":true,"knots":knots}]}});
        let start = std::time::Instant::now();
        let photocraft_engine::jobs::Started::Job(job) = s.start("filter.blur.shapeBlur", params).unwrap() else { panic!("background filter job") };
        let admitted = start.elapsed();
        s.cancel_job(job);
        s.join_cancelled_jobs();
        eprintln!("custom shape smart={smart}: admission={admitted:?}, cancelled worker drained in {:?}", start.elapsed());
        assert!(admitted < std::time::Duration::from_secs(1), "command thread prepared the kernel for {admitted:?}");
        assert!(start.elapsed() < std::time::Duration::from_secs(3), "kernel preparation did not observe cancellation");
        assert_eq!(photocraft_format::save_to_bytes(&s.active().unwrap().doc, &Default::default()).unwrap(), before);
    }
}

/// Source-preservation inspection must follow the recorded Open's authorization gate.
#[test]
fn batch_exr_preservation_does_not_preempt_open_authorization() {
    fn deny_open(id: &str, _: &Value) -> photocraft_engine::Result<()> {
        if id == "file.openExrParts" { Err(photocraft_engine::EngineError::Other("recorded Open denied".into())) } else { Ok(()) }
    }
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("unreadable.exr");
    std::fs::write(&input, b"not an EXR header").unwrap();
    let mut s = Session::new();
    s.authorize = Some(deny_open);
    let result = s
        .execute(
            "file.automate.batch",
            json!({
                "input":[input],"destination":"saveAndClose","overrideOpen":true,
                "steps":[{"command":"file.openExrParts","params":{"path":"recorded.exr"}}]
            }),
        )
        .unwrap();
    assert_eq!(result["files"].as_array().unwrap().len(), 0);
    assert!(result["errors"][0]["error"].as_str().unwrap().contains("recorded Open denied"), "{result}");
    assert_eq!(std::fs::read(input).unwrap(), b"not an EXR header");
}

/// Removing the original descriptor must exercise every native naming-component wire mapping.
#[test]
fn batch_naming_components_rebuild_without_original_descriptor() {
    for (wire, native) in [
        ("Nm  ", "name"),
        ("lowerCase", "nameLower"),
        ("upperCase", "nameUpper"),
        ("oneDigit", "serial1"),
        ("twoDigit", "serial2"),
        ("threeDigit", "serial3"),
        ("fourDigit", "serial4"),
        ("lowerCaseSerial", "letter"),
        ("upperCaseSerial", "LETTER"),
        ("mmddyy", "mmddyy"),
        ("mmdd", "mmdd"),
        ("yyyymmdd", "yyyymmdd"),
        ("yymmdd", "yymmdd"),
        ("yyddmm", "yyddmm"),
        ("ddmmyy", "ddmmyy"),
        ("ddmm", "ddmm"),
        ("lowerCaseExtension", "extension"),
        ("upperCaseExtension", "EXTENSION"),
    ] {
        let component =
            Descriptor::new("fileNamingComponent").with("component", D::Enumerated { type_id: Id::new("fileNamingComponent"), value: Id::new(wire) });
        let rules = Descriptor::new("fileNamingRules").with("fileNamingComponents", D::List(vec![D::Descriptor(component)]));
        let d = Descriptor::new("Btch")
            .with("null", path("input"))
            .with("T   ", path("output"))
            .with("Usng", D::Reference(vec![named("Actn", "Child"), named("ASet", "Parent")]))
            .with("fileNamingRules", D::Descriptor(rules.clone()));
        let imported = atn_cmds::import(&encoded(*b"Btch", d, true)).unwrap();
        assert!(imported.unsupported.is_empty(), "{wire}");
        let mut action = imported.actions[0].clone();
        let params = &mut action.steps[0].1;
        assert_eq!(params["namingComponents"], json!([{"component":native}]));
        assert!(params.as_object_mut().unwrap().remove("__atnBatch").is_some());
        assert_eq!(exported(&action).get("fileNamingRules"), Some(&D::Descriptor(rules)), "{wire}");
    }
}
