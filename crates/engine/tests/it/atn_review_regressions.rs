//! Public-API regressions for the dialog-event and Batch review findings.
use craft_psd::atn::{self, AtnAction, AtnEvent, AtnFile, AtnItem};
use craft_psd::descriptor::{Class, Descriptor, Id, ReferenceItem, UnicodeString, Value as D};
use photocraft_doc::LayerContent;
use photocraft_engine::{Session, actions_cmds::Action, atn_cmds};
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
        let lut = craft_cms::lutfile::parse_3dl(&text).unwrap();
        let mid = ((lut.size / 2 * lut.size + lut.size / 2) * lut.size + lut.size / 2) * 3;
        assert!((lut.data[mid] - expected).abs() < 0.002, "{shaper}: {}", lut.data[mid]);
    }
}

/// An untouched one-name Batch keeps all original bytes; an edit keeps its unqualified lookup.
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
    assert_eq!(*names, vec![named("Actn", "Edited")]);
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

/// Accepted main encodings still map and play, while an unchanged save retains their exact descriptor.
#[test]
fn custom_supported_representations_map_and_save_losslessly() {
    let kernel = D::List((0..25).map(|i| D::Integer(i32::from(i == 12))).collect());
    let mut metadata = Descriptor::new("null").with("Mtrx", kernel.clone()).with("Scl ", D::Integer(2));
    metadata.name = UnicodeString(vec![0xd800, 0]);
    metadata.class_id = Id::Str(b"null".to_vec());
    for d in [
        Descriptor::new("null").with("Scl", D::Integer(3)).with("Mtrx", kernel.clone()),
        Descriptor::new("null").with("Mtrx", kernel.clone()).with("Scl ", D::Double(2.0)),
        Descriptor::new("Cstm").with("Ofst", D::Double(0.0)).with("Mtrx", kernel).with("Scl ", D::Integer(1)),
        metadata,
    ] {
        let imported = atn_cmds::import(&encoded(*b"Cstm", d.clone(), true)).unwrap();
        assert!(imported.unsupported.is_empty());
        let mut action = imported.actions[0].clone();
        assert_eq!(action.steps[0].0, "filter.other.custom");
        assert_eq!(exported(&action), d);
        let mut native = session("rgb");
        native.execute("filter.other.custom", action.steps[0].1.clone()).unwrap();
        let mut played = session("rgb");
        played.actions.list.push(action.clone());
        played.execute("actions.play", json!({"action":action.name})).unwrap();
        let commands: Vec<_> = played.journal.iter().filter(|(id, _)| id == "filter.other.custom").collect();
        assert!(!commands.is_empty());
        assert!(commands.iter().all(|(_, params)| params.get("__atnCustom").is_none()));
        let (id, last) = photocraft_engine::filters::last_filter(&played).unwrap();
        assert_eq!(id, "filter.other.custom");
        assert!(last.get("__atnCustom").is_none());
        assert!(played.actions.list[0].steps[0].1.get("__atnCustom").is_some());
        assert_eq!(exported(&played.actions.list[0]), d, "Save Actions still owns the original descriptor");
        assert_eq!(photocraft_compose::flatten(&played.active().unwrap().doc).px, photocraft_compose::flatten(&native.active().unwrap().doc).px);
        action.steps[0].1["scale"] = json!(4);
        assert_eq!(exported(&action).get("Scl "), Some(&D::Integer(4)), "editing must not reuse stale original metadata");
    }
}

/// Duplicate kernels are ambiguous even when both lists are valid or use different wire ID kinds.
#[test]
fn custom_duplicate_kernel_fields_are_kept_with_their_enabled_state() {
    let kernel = D::List((0..25).map(|i| D::Integer(i32::from(i == 12))).collect());
    for key in [Id::new("Mtrx"), Id::Str(b"Mtrx".to_vec())] {
        for enabled in [false, true] {
            for second in [kernel.clone(), D::List(vec![D::Integer(0); 25])] {
                let mut d = Descriptor::new("null").with("Mtrx", kernel.clone());
                d.items.push((key.clone(), second));
                let imported = atn_cmds::import(&encoded(*b"Cstm", d.clone(), enabled)).unwrap();
                assert_eq!(imported.unsupported.len(), 1);
                let action = &imported.actions[0];
                assert_eq!(action.steps[0].0, atn_cmds::PHOTOSHOP_STEP);
                assert_eq!(action.disabled.contains(&0), !enabled);
                assert_eq!(exported(action), d);
                let (bytes, _) = atn_cmds::export("Parent", &[action], &[], false).unwrap();
                assert_eq!(atn::parse(&bytes).unwrap().actions[0].items[0].enabled, enabled);
            }
        }
    }
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

/// In-place EXR writes retain canonical windows or refuse both active and closed results.
#[test]
fn batch_save_close_refuses_changed_exr_windows() {
    use exr::prelude::*;
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("window.exr");
    for (origin, display, accepted) in [
        ((-3, 5), IntegerBounds::new((-3, 5), (2, 2)), false),
        ((0, 0), IntegerBounds::new((-1, -1), (4, 4)), false),
        ((0, 0), IntegerBounds::new((0, 0), (2, 2)), true),
    ] {
        let layer = Layer::new(
            (2, 2),
            LayerAttributes { layer_position: Vec2(origin.0, origin.1), ..Default::default() },
            Encoding::FAST_LOSSLESS,
            AnyChannels::sort(
                [("R", 0.25), ("G", 0.5), ("B", 2.0)].into_iter().map(|(name, value)| AnyChannel::new(name, FlatSamples::F32(vec![value; 4]))).collect(),
            ),
        );
        Image::from_layers(ImageAttributes::new(display), vec![layer]).write().to_file(&input).unwrap();
        let before = std::fs::read(&input).unwrap();
        for snapshot in [false, true] {
            std::fs::write(&input, &before).unwrap();
            let steps = if snapshot { json!([["file.openAs", {"path":"recorded.exr"}], ["file.saveAs", {}], ["file.close", {}]]) } else { json!([]) };
            let result = Session::new()
                .execute("file.automate.batch", json!({"input":[input],"destination":"saveAndClose","overrideOpen":snapshot,"steps":steps}))
                .unwrap();
            assert_eq!(result["files"].as_array().unwrap().len(), usize::from(accepted), "{result}");
            if accepted {
                let after = std::fs::read(&input).unwrap();
                let info = photocraft_codecs::exr_info(&after, &Default::default()).unwrap();
                assert_eq!(info[0].data_window, (0, 0, 2, 2));
                assert_eq!(info[0].display_window, info[0].data_window);
                let rgba = |bytes: &[u8]| {
                    photocraft_codecs::decode(bytes)
                        .unwrap()
                        .convert(photocraft_codecs::ChannelLayout::Rgba, photocraft_codecs::SampleType::F32)
                        .to_f32_samples()
                        .unwrap()
                };
                assert_eq!(rgba(&after), rgba(&before));
            } else {
                assert!(result["errors"][0]["error"].as_str().unwrap().contains("data/display windows"), "{result}");
                assert_eq!(std::fs::read(&input).unwrap(), before);
            }
        }
        std::fs::write(&input, &before).unwrap();
        let separate =
            Session::new().execute("file.automate.batch", json!({"input":[input],"output":dir.path().join("copies"),"format":"exr","steps":[]})).unwrap();
        assert_eq!(separate["files"].as_array().unwrap().len(), 1, "{separate}");
        assert_eq!(std::fs::read(&input).unwrap(), before);
    }
}

/// Targa alpha remains source data even when its descriptor omits the alpha flag.
#[test]
fn batch_save_close_refuses_targa_24_alpha_loss() {
    use photocraft_codecs::{ChannelLayout, EncodeOptions, Format, Image};
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("alpha.tga");
    for (alpha, descriptor_alpha) in [([0, 173], true), ([0, 0], true), ([0, 173], false)] {
        let image = Image::from_u8(2, 1, ChannelLayout::Rgba, vec![11, 22, 33, alpha[0], 77, 88, 99, alpha[1]]).unwrap();
        let mut before = photocraft_codecs::encode(&image, Format::Tga, &EncodeOptions::default()).unwrap();
        if !descriptor_alpha {
            before[17] &= 0xf0;
        }
        let opened = photocraft_io::import("alpha.tga", &before).unwrap().document;
        assert_eq!(opened.channels.len(), 1, "the fixture must retain its imported alpha plane");
        for snapshot in [false, true] {
            std::fs::write(&input, &before).unwrap();
            let steps = if snapshot { json!([["file.openAs", {"path":"recorded.tga"}], ["file.saveAs", {}], ["file.close", {}]]) } else { json!([]) };
            let result = Session::new()
                .execute("file.automate.batch", json!({"input":[input],"destination":"saveAndClose","tgaBits":24,"overrideOpen":snapshot,"steps":steps}))
                .unwrap();
            assert_eq!(result["files"], json!([]), "{result}");
            assert_eq!(result["errors"].as_array().unwrap().len(), 1);
            assert!(result["errors"][0]["error"].as_str().unwrap().contains("24-bit Targa"), "{result}");
            assert_eq!(std::fs::read(&input).unwrap(), before);
        }
    }
}

/// Automatic/32-bit Targa keeps hidden RGB and alpha; separate 24-bit output reports the loss.
#[test]
fn batch_targa_preservation_and_export_warnings() {
    use photocraft_codecs::{ChannelLayout, EncodeOptions, Format, Image};
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("alpha.tga");
    let pixels = vec![11, 22, 33, 0, 77, 88, 99, 173];
    let image = Image::from_u8(2, 1, ChannelLayout::Rgba, pixels.clone()).unwrap();
    let before = photocraft_codecs::encode(&image, Format::Tga, &EncodeOptions::default()).unwrap();
    for bits in [Value::Null, json!(32)] {
        std::fs::write(&input, &before).unwrap();
        let result = Session::new().execute("file.automate.batch", json!({"input":[input],"destination":"saveAndClose","tgaBits":bits,"steps":[]})).unwrap();
        assert_eq!(result["files"], json!([input]), "{result}");
        assert_eq!(photocraft_codecs::decode(&std::fs::read(&input).unwrap()).unwrap().data(), pixels);
    }
    for snapshot in [false, true] {
        std::fs::write(&input, &before).unwrap();
        let steps = if snapshot { json!([["file.saveAs", {}], ["file.close", {}]]) } else { json!([]) };
        let result = Session::new()
            .execute("file.automate.batch", json!({"input":[input],"output":dir.path().join("copies"),"format":"tga","tgaBits":24,"steps":steps}))
            .unwrap();
        assert_eq!(result["files"].as_array().unwrap().len(), 1, "{result}");
        let output = result["files"][0].as_str().unwrap();
        assert_eq!(photocraft_codecs::decode(&std::fs::read(output).unwrap()).unwrap().data(), &[11, 22, 33, 77, 88, 99]);
        assert!(
            result["notices"]
                .as_array()
                .unwrap()
                .iter()
                .any(|n| n["warnings"].as_array().is_some_and(|w| w.iter().any(|v| v.as_str().is_some_and(|s| s.contains("24-bit") && s.contains("Alpha 1"))))),
            "{result}"
        );
        assert_eq!(std::fs::read(&input).unwrap(), before);
    }
}

/// Explicit 24-bit options do not reject opaque RGB or the grayscale Targa option bypass.
#[test]
fn batch_targa_24_keeps_opaque_and_gray_alpha_results() {
    use photocraft_codecs::{ChannelLayout, EncodeOptions, Format, Image};
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("allowed.tga");
    for (layout, pixels) in [(ChannelLayout::Rgb, vec![11, 22, 33, 77, 88, 99]), (ChannelLayout::GrayA, vec![0, 0, 96, 255])] {
        let image = Image::from_u8(2, 1, layout, pixels.clone()).unwrap();
        std::fs::write(&input, photocraft_codecs::encode(&image, Format::Tga, &EncodeOptions::default()).unwrap()).unwrap();
        let result = Session::new().execute("file.automate.batch", json!({"input":[input],"destination":"saveAndClose","tgaBits":24,"steps":[]})).unwrap();
        assert_eq!(result["files"], json!([input]), "{result}");
        assert_eq!(photocraft_codecs::decode(&std::fs::read(&input).unwrap()).unwrap().data(), pixels);
    }
}

/// A tiny flat TIFF with real composite transparency and an optional retained alpha plane.
fn batch_tiff_alpha_fixture(extra: bool, opaque: bool) -> Vec<u8> {
    use photocraft_color::{ColorMode, PixelFormat, SampleType};
    use photocraft_doc::{AlphaChannel, Document, Layer};
    use photocraft_geom::{Rect, Size};
    use photocraft_raster::Surface;
    let mut doc = Document::new("alpha", Size::new(2, 2), ColorMode::Rgb, SampleType::U8);
    let mut pixels = Surface::new(doc.pixel_format());
    pixels.fill_rect(if opaque { Rect::new(0, 0, 2, 2) } else { Rect::new(1, 1, 2, 2) }, &[1.0, 0.0, 0.0, 1.0]);
    doc.layers.push(Layer::new("Art", LayerContent::Raster(pixels)));
    if extra {
        let mut mask = Surface::new(PixelFormat::new(ColorMode::Grayscale, SampleType::U8, false));
        mask.fill_rect(Rect::new(0, 0, 1, 2), &[1.0]);
        doc.channels.push(AlphaChannel::new("Mask", mask));
    }
    photocraft_io::export(&doc, "alpha.tif", &photocraft_io::ExportOptions { tiff_transparency: Some(true), ..Default::default() }).unwrap().bytes
}

/// The actual flat path refuses explicit and auxiliary-plane matting before replacing the source.
#[test]
fn batch_save_close_refuses_flat_tiff_transparency_loss() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("alpha.tif");
    for (extra, transparency) in [(false, json!(false)), (true, Value::Null)] {
        let before = batch_tiff_alpha_fixture(extra, false);
        for snapshot in [false, true] {
            std::fs::write(&input, &before).unwrap();
            let steps = if snapshot {
                json!([["file.openAs", {"path":"recorded.tif"}], ["layer.toggleLockTransparency", {}], ["file.saveAs", {}], ["file.close", {}]])
            } else {
                json!([["layer.toggleLockTransparency", {}]])
            };
            let result = Session::new()
                .execute(
                    "file.automate.batch",
                    json!({"input":[input],"destination":"saveAndClose","overrideOpen":snapshot,"tiffTransparency":transparency,"steps":steps}),
                )
                .unwrap();
            assert_eq!(result["files"], json!([]), "{result}");
            assert_eq!(result["errors"].as_array().unwrap().len(), 1);
            assert!(result["errors"][0]["error"].as_str().unwrap().contains("TIFF composite transparency"), "{result}");
            assert_eq!(std::fs::read(&input).unwrap(), before);
        }
    }
}

/// Keeping flat alpha, preserving actual layers and writing an opaque composite remain allowed.
#[test]
fn batch_tiff_alpha_guard_keeps_legitimate_results() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("allowed.tif");
    for (layered, opaque, transparency) in [(false, false, json!(true)), (true, false, json!(false)), (false, true, Value::Null)] {
        let before = batch_tiff_alpha_fixture(true, opaque);
        std::fs::write(&input, &before).unwrap();
        let steps = if !layered && !opaque { json!([["layer.toggleLockTransparency", {}]]) } else { json!([]) };
        let result = Session::new()
            .execute("file.automate.batch", json!({"input":[input],"destination":"saveAndClose","tiffTransparency":transparency,"steps":steps}))
            .unwrap();
        assert_eq!(result["files"], json!([input]), "{result}");
        let bytes = std::fs::read(&input).unwrap();
        assert_eq!(photocraft_codecs::tiff_photoshop_tags(&bytes).1.is_some(), layered);
        let back = photocraft_io::import("allowed.tif", &bytes).unwrap().document;
        assert_eq!(back.channels.len(), 1);
        assert_eq!(back.channels[0].surface.pixel(0, 0)[0], 1.0);
        assert_eq!(back.channels[0].surface.pixel(1, 1)[0], 0.0);
        if layered {
            let LayerContent::Raster(surface) = &back.layers[0].content else { panic!("the saved layer must remain raster") };
            assert_eq!(surface.pixel(0, 0).last().copied(), Some(0.0));
        } else {
            let actual = photocraft_codecs::decode(&bytes).unwrap();
            let original = photocraft_codecs::decode(&before).unwrap();
            assert_eq!(actual.data(), original.data());
        }
    }
}

/// Starts the largest supported custom kernel as a background job, cancels it at once and checks
/// it ends cancelled without an edit. Returns how long admission and the whole run took.
fn cancel_largest_custom_shape(smart: bool) -> (std::time::Duration, std::time::Duration) {
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
    assert!(s.cancel_job(job));
    s.join_cancelled_jobs();
    let drained = start.elapsed();
    let outcome = loop {
        if let Some(event) = s.poll_jobs().into_iter().find(|e| e.id == job) {
            break event.outcome;
        }
        assert!(start.elapsed() < std::time::Duration::from_secs(60), "the cancelled job never reported");
        std::thread::sleep(std::time::Duration::from_millis(2));
    };
    assert_eq!(outcome, photocraft_engine::jobs::JobOutcome::Cancelled, "smart={smart}");
    assert_eq!(photocraft_format::save_to_bytes(&s.active().unwrap().doc, &Default::default()).unwrap(), before);
    (admitted, drained)
}

/// The largest supported custom kernel is prepared on the worker and can be cancelled without an edit.
#[test]
fn custom_shape_preparation_yields_a_cancellable_job() {
    for smart in [false, true] {
        cancel_largest_custom_shape(smart);
    }
}

/// How fast that job is admitted and drained after a cancel.
#[test]
// Wall-clock timing depends on machine load (the release corpus job runs every test), so it is
// opt-in, like `cancel_takes_effect_within_200_ms_on_24_mp`.
#[ignore = "timing: cargo test --release -p photocraft-engine --test it custom_shape_preparation_cancels_quickly -- --ignored"]
fn custom_shape_preparation_cancels_quickly() {
    for smart in [false, true] {
        let (admitted, drained) = cancel_largest_custom_shape(smart);
        eprintln!("custom shape smart={smart}: admission={admitted:?}, cancelled worker drained in {drained:?}");
        assert!(admitted < std::time::Duration::from_secs(1), "command thread prepared the kernel for {admitted:?}");
        assert!(drained < std::time::Duration::from_secs(3), "kernel preparation did not observe cancellation");
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

/// Tiny encoded sources keep Batch tests independent of external image assets.
fn small_image(format: photocraft_codecs::Format) -> Vec<u8> {
    let image = photocraft_codecs::Image::from_u8(2, 2, photocraft_codecs::ChannelLayout::Rgba, [32, 64, 128, 255].repeat(4)).unwrap();
    photocraft_codecs::encode(&image, format, &Default::default()).unwrap()
}

/// A real one-name Batch import follows its own renamed action, never a same-name existing action.
#[test]
fn imported_batch_references_follow_collision_renames_once_and_keep_other_sets() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("input.png");
    let output = dir.path().join("out");
    std::fs::write(&input, small_image(photocraft_codecs::Format::Png)).unwrap();
    let d = Descriptor::new("Btch")
        .with("null", path(dir.path().to_str().unwrap()))
        .with("T   ", path(output.to_str().unwrap()))
        .with("Usng", D::Reference(vec![named("Actn", "Child")]));
    let mut file = atn::parse(&encoded(*b"Btch", d, true)).unwrap();
    file.name = UnicodeString::new_nul("Imported");
    for name in ["Child", "Child 2"] {
        let child = Action::new(name, vec![("image.imageSize".into(), json!({"width":3,"height":1,"resample":"nearest"}))]);
        file.actions.push(atn::parse(&atn_cmds::export("Imported", &[&child], &[], false).unwrap().0).unwrap().actions.remove(0));
    }
    let mut external = file.actions[0].items[0].clone();
    external.descriptor.as_mut().unwrap().items.iter_mut().find(|(k, _)| k.is("Usng")).unwrap().1 =
        D::Reference(vec![named("Actn", "Child"), named("ASet", "Other")]);
    file.actions[0].items.push(external);
    let mut s = Session::new();
    let mut old = Action::new("Child", vec![("select.deselect".into(), json!({}))]);
    old.set = Some("Other".into());
    s.actions.list.push(old);
    let loaded = s.execute("actions.load", json!({"data":photocraft_paint::tile::b64_encode(&atn::write(&file).unwrap()),"replace":false})).unwrap();
    let own_name = loaded["actions"][1].as_str().unwrap();
    let imported = &s.actions.list[1];
    assert_eq!(imported.steps[0].1["action"], own_name);
    assert_eq!(imported.steps[1].1["action"], "Child");
    assert_eq!(imported.steps[1].1["set"], "Other");
    let batch = imported.steps[0].1.clone();
    let result = s.execute("file.automate.batch", batch).unwrap();
    assert_eq!(result["files"].as_array().unwrap().len(), 1, "{result}");
    let doc = photocraft_io::import("output.png", &std::fs::read(result["files"][0].as_str().unwrap()).unwrap()).unwrap().document;
    assert_eq!(doc.size, photocraft_doc::Size::new(3, 1), "the renamed imported child ran");
}

/// Null is the optional default set, while invalid typed and explicit set values remain distinct.
#[test]
fn batch_null_set_exports_like_absent_and_preserves_explicit_sets() {
    let p = json!({"input":"in","output":"out","action":"Child","stopOnError":true,"includeNonImages":true,"namingComponents":[{"component":"name"},{"component":"extension"}]});
    let absent = exported(&Action::new("Review", vec![("file.automate.batch".into(), p.clone())]));
    for (set, allowed) in [(Value::Null, true), (json!("Other"), true), (json!(4), false)] {
        let mut p = p.clone();
        p["set"] = set.clone();
        let action = Action::new("Review", vec![("file.automate.batch".into(), p)]);
        let result = atn_cmds::export("Parent", &[&action], &[], false);
        assert_eq!(result.is_ok(), allowed);
        if allowed {
            let bytes = result.unwrap().0;
            let imported = atn_cmds::import(&bytes).unwrap();
            assert_eq!(imported.actions[0].steps[0].1.get("set"), if set.is_null() { None } else { Some(&set) });
            if set.is_null() {
                assert_eq!(atn::parse(&bytes).unwrap().actions[0].items[0].descriptor, Some(absent.clone()));
            }
        }
    }
}

/// Both native and edited imported Batch steps keep global lookup when their reference is unqualified.
#[test]
fn batch_unqualified_export_keeps_cross_set_resolution() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("input.png"), small_image(photocraft_codecs::Format::Png)).unwrap();
    for edited in [false, true] {
        let p = json!({"input":dir.path(),"output":dir.path().join(format!("out-{edited}")),"action":"Child","stopOnError":true,"includeNonImages":true,"namingComponents":[{"component":"name"},{"component":"extension"}]});
        let mut parent = Action::new("Parent", vec![("file.automate.batch".into(), p)]);
        if edited {
            parent = atn_cmds::import(&atn_cmds::export("Parent set", &[&parent], &[], false).unwrap().0).unwrap().actions.remove(0);
            parent.steps[0].1["suppressWarnings"] = json!(true);
        }
        let imported = atn_cmds::import(&atn_cmds::export("Parent set", &[&parent], &[], false).unwrap().0).unwrap();
        let batch = imported.actions[0].steps[0].1.clone();
        assert!(batch.get("set").is_none());
        let mut child = Action::new("Child", vec![("image.imageSize".into(), json!({"width":3,"height":1,"resample":"nearest"}))]);
        child.set = Some("Other set".into());
        let mut s = Session::new();
        s.actions.list.push(child);
        let result = s.execute("file.automate.batch", batch).unwrap();
        assert_eq!(result["files"].as_array().unwrap().len(), 1, "{result}");
        let doc = photocraft_io::import("out.png", &std::fs::read(result["files"][0].as_str().unwrap()).unwrap()).unwrap().document;
        assert_eq!(doc.size, photocraft_doc::Size::new(3, 1));
    }
}

/// Rebinding the imported set follows local children, but a foreign destination-set reference stays foreign.
#[test]
fn batch_load_override_preserves_original_set_identity_and_foreign_references() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("input.png"), small_image(photocraft_codecs::Format::Png)).unwrap();
    for (foreign, collision) in [(false, false), (false, true), (true, true)] {
        let params = json!({"input":dir.path(),"output":dir.path().join(format!("out-{foreign}-{collision}")),"action":"Child","set":if foreign {"Destination"} else {"Original"},"stopOnError":true,"includeNonImages":true,"namingComponents":[{"component":"name"},{"component":"extension"}]});
        let parent = Action::new("Parent", vec![("file.automate.batch".into(), params)]);
        let child = Action::new("Child", vec![("image.imageSize".into(), json!({"width":3,"height":1,"resample":"nearest"}))]);
        let bytes = atn_cmds::export("Original", &[&parent, &child], &[], false).unwrap().0;
        let mut s = Session::new();
        if collision {
            let mut old = Action::new("Child", vec![("image.imageSize".into(), json!({"width":5,"height":1,"resample":"nearest"}))]);
            old.set = Some("Destination".into());
            s.actions.list.push(old);
        }
        let loaded = s.execute("actions.load", json!({"data":photocraft_paint::tile::b64_encode(&bytes),"set":"Destination","replace":false})).unwrap();
        let parent = s.actions.list.iter().find(|a| a.name == loaded["actions"][0].as_str().unwrap()).unwrap();
        let batch = parent.steps[0].1.clone();
        assert_eq!(batch["set"], "Destination");
        assert_eq!(batch["action"], if foreign { json!("Child") } else { loaded["actions"][1].clone() });
        let saved = atn_cmds::import(&atn_cmds::export("Destination", &[parent], &[], false).unwrap().0).unwrap();
        assert_eq!(saved.actions[0].steps[0].1["set"], batch["set"]);
        assert_eq!(saved.actions[0].steps[0].1["action"], batch["action"]);
        let result = s.execute("file.automate.batch", batch).unwrap();
        assert_eq!(result["files"].as_array().unwrap().len(), 1, "{result}");
        let doc = photocraft_io::import("out.png", &std::fs::read(result["files"][0].as_str().unwrap()).unwrap()).unwrap().document;
        assert_eq!(doc.size, photocraft_doc::Size::new(if foreign { 5 } else { 3 }, 1));
    }
}

/// A real one-bit transparency-mask IFD, skipped by default TIFF decoding, retains separate data.
fn tiff_with_separate_mask() -> Vec<u8> {
    let mut bytes = small_image(photocraft_codecs::Format::Tiff);
    assert_eq!(&bytes[..2], b"II");
    let first = u32::from_le_bytes(bytes[4..8].try_into().unwrap()) as usize;
    let count = u16::from_le_bytes(bytes[first..first + 2].try_into().unwrap()) as usize;
    if !bytes.len().is_multiple_of(2) {
        bytes.push(0);
    }
    let mask = bytes.len() as u32;
    bytes[first + 2 + count * 12..first + 6 + count * 12].copy_from_slice(&mask.to_le_bytes());
    let data = mask + 2 + 11 * 12 + 4;
    bytes.extend_from_slice(&11u16.to_le_bytes());
    for (tag, ty, value) in [
        (254u16, 4u16, 4u32),
        (256, 4, 2),
        (257, 4, 2),
        (258, 3, 1),
        (259, 3, 1),
        (262, 3, 4),
        (273, 4, data),
        (277, 3, 1),
        (278, 4, 2),
        (279, 4, 2),
        (284, 3, 1),
    ] {
        bytes.extend_from_slice(&tag.to_le_bytes());
        bytes.extend_from_slice(&ty.to_le_bytes());
        bytes.extend_from_slice(&1u32.to_le_bytes());
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    bytes.extend_from_slice(&0u32.to_le_bytes());
    bytes.extend_from_slice(&[0x80, 0x40]);
    assert!(photocraft_codecs::tiff_info(&bytes).unwrap().pages.iter().any(|p| p.kind == photocraft_codecs::TiffPageKind::Mask));
    bytes
}

/// A flat RGB(A) TIFF with individually identifiable auxiliary channels.
fn tiff_with_aux_channels(extras: usize, alpha: bool) -> Vec<u8> {
    let samples = 3 + usize::from(alpha) + extras;
    let kinds = extras + usize::from(alpha);
    let bits = 8 + 2 + 12 * 12 + 4;
    let kind_at = bits + samples * 2;
    let data = kind_at + kinds * 2;
    let mut bytes = b"II\x2a\0\x08\0\0\0".to_vec();
    bytes.extend_from_slice(&12u16.to_le_bytes());
    for (tag, ty, count, value) in [
        (256u16, 4u16, 1u32, 2u32),
        (257, 4, 1, 2),
        (258, 3, samples as u32, bits as u32),
        (259, 3, 1, 1),
        (262, 3, 1, 2),
        (273, 4, 1, data as u32),
        (277, 3, 1, samples as u32),
        (278, 4, 1, 2),
        (279, 4, 1, (samples * 4) as u32),
        (284, 3, 1, 1),
        (338, 3, kinds as u32, kind_at as u32),
        (339, 3, 1, 1),
    ] {
        bytes.extend_from_slice(&tag.to_le_bytes());
        bytes.extend_from_slice(&ty.to_le_bytes());
        bytes.extend_from_slice(&count.to_le_bytes());
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    bytes.extend_from_slice(&0u32.to_le_bytes());
    for _ in 0..samples {
        bytes.extend_from_slice(&8u16.to_le_bytes());
    }
    for i in 0..kinds {
        bytes.extend_from_slice(&(if alpha && i == 0 { 2u16 } else { 0 }).to_le_bytes());
    }
    for _ in 0..4 {
        bytes.extend_from_slice(&[32, 64, 128]);
        if alpha {
            bytes.push(200);
        }
        bytes.extend((0..extras).map(|i| i as u8 + 1));
    }
    assert_eq!(bytes.len(), data + samples * 4);
    bytes
}

/// Extra source payloads must survive both ordinary and saved/closed Batch refusal paths.
#[test]
fn batch_save_close_preserves_additional_images_and_allows_single_images() {
    let dir = tempfile::tempdir().unwrap();
    let still = small_image(photocraft_codecs::Format::Gif);
    let flags = still[10];
    let blocks = 13 + if flags & 0x80 != 0 { 3usize << ((flags & 7) + 1) } else { 0 };
    let mut animated = still[..still.len() - 1].to_vec();
    animated.extend_from_slice(&still[blocks..]);
    assert_eq!(photocraft_codecs::decode_gif_frames(&animated, &Default::default()).unwrap().len(), 2);
    let dpx = small_image(photocraft_codecs::Format::Dpx);
    let mut elements = dpx.clone();
    elements[770..772].copy_from_slice(&2u16.to_be_bytes());
    elements.extend_from_slice(b"distinct second DPX element payload");
    let dds = small_image(photocraft_codecs::Format::Dds);
    let mut slices = dds.clone();
    slices[24..28].copy_from_slice(&2u32.to_le_bytes());
    let mut second = dds[128..].to_vec();
    second[0] ^= 0xff;
    slices.extend(second);
    let mut faces = slices.clone();
    faces[24..28].copy_from_slice(&0u32.to_le_bytes());
    faces[112..116].copy_from_slice(&0xc00u32.to_le_bytes());
    let rgb = photocraft_codecs::Image::from_u8(2, 2, photocraft_codecs::ChannelLayout::Rgb, [32, 64, 128].repeat(4)).unwrap();
    let mut options = photocraft_codecs::EncodeOptions::default();
    options.dds.mipmaps = true;
    let mut mipmaps = photocraft_codecs::encode(&rgb, photocraft_codecs::Format::Dds, &options).unwrap();
    let flags = u32::from_le_bytes(mipmaps[8..12].try_into().unwrap());
    mipmaps[8..12].copy_from_slice(&(flags & !0x0002_0000).to_le_bytes());
    let fits = photocraft_codecs::encode(&rgb, photocraft_codecs::Format::Fits, &Default::default()).unwrap();
    let mut hdu = fits.clone();
    hdu[..80].fill(b' ');
    hdu[..17].copy_from_slice(b"XTENSION= 'IMAGE'");
    hdu[2880] ^= 0xff;
    let mut hdus = fits.clone();
    hdus.extend(hdu);
    let mut cubes = fits.clone();
    let axis = cubes.as_chunks_mut::<80>().0.iter_mut().find(|card| card.starts_with(b"NAXIS3  =")).unwrap();
    axis.fill(b' ');
    axis[..11].copy_from_slice(b"NAXIS3  = 4");
    let mut table = fits.clone();
    for card in [
        "XTENSION= 'BINTABLE'",
        "BITPIX  = 8",
        "NAXIS   = 2",
        "NAXIS1  = 4",
        "NAXIS2  = 1",
        "PCOUNT  = 0",
        "GCOUNT  = 1",
        "TFIELDS = 1",
        "TTYPE1  = 'DATA'",
        "TFORM1  = '4B'",
        "END",
    ] {
        table.extend_from_slice(format!("{card:<80}").as_bytes());
    }
    table.resize(table.len().div_ceil(2880) * 2880, b' ');
    table.extend_from_slice(&[0xde, 0xad, 0xbe, 0xef]);
    table.resize(table.len().div_ceil(2880) * 2880, 0);
    let mut cases = vec![
        ("gif", animated),
        ("dpx", elements),
        ("dds", slices),
        ("dds", faces),
        ("dds", mipmaps),
        ("fits", hdus),
        ("fits", cubes),
        ("fits", table),
        ("tiff", tiff_with_separate_mask()),
    ];
    for alpha in [false, true] {
        cases.push(("tiff", tiff_with_aux_channels(photocraft_codecs::MAX_EXTRA_SAMPLES + 1, alpha)));
    }
    for (index, (extension, multiple)) in cases.into_iter().enumerate() {
        photocraft_codecs::decode(&multiple).unwrap();
        let input = dir.path().join(format!("source.{extension}"));
        for steps in [
            json!([]),
            json!([{"command":"file.openAs","params":{"path":"recorded.png"}},{"command":"file.saveAs","params":{}},{"command":"file.close","params":{"force":true}}]),
        ] {
            std::fs::write(&input, &multiple).unwrap();
            let override_open = !steps.as_array().unwrap().is_empty();
            let mut s = Session::new();
            let result =
                s.execute("file.automate.batch", json!({"input":[input],"destination":"saveAndClose","overrideOpen":override_open,"steps":steps})).unwrap();
            assert!(result["files"].as_array().unwrap().is_empty(), "{extension}: {result}");
            assert!(result["errors"][0]["error"].as_str().unwrap().contains("additional source data"), "{extension}: {result}");
            assert_eq!(std::fs::read(&input).unwrap(), multiple);
        }
        let result =
            Session::new().execute("file.automate.batch", json!({"input":[input],"output":dir.path().join(format!("separate-{index}")),"steps":[]})).unwrap();
        assert_eq!(result["files"].as_array().unwrap().len(), 1, "{extension}: {result}");
        assert!(std::path::Path::new(result["files"][0].as_str().unwrap()).is_file());
        assert_eq!(std::fs::read(&input).unwrap(), multiple);
    }
    for (extension, bytes) in [("gif", still), ("dpx", dpx), ("dds", dds), ("fits", fits), ("tiff", small_image(photocraft_codecs::Format::Tiff))] {
        let input = dir.path().join(format!("still.{extension}"));
        std::fs::write(&input, bytes).unwrap();
        let result = Session::new().execute("file.automate.batch", json!({"input":[input],"destination":"saveAndClose","steps":[]})).unwrap();
        assert_eq!(result["files"].as_array().unwrap().len(), 1, "{extension}: {result}");
        assert!(!photocraft_codecs::has_unretained_source_data(&std::fs::read(input).unwrap()).unwrap());
    }
}

/// Auxiliary channels below and at the cap remain writable, including beside a retained alpha.
#[test]
fn batch_save_close_preserves_aux_channels_at_the_supported_cap() {
    let dir = tempfile::tempdir().unwrap();
    for alpha in [false, true] {
        for extras in [photocraft_codecs::MAX_EXTRA_SAMPLES - 1, photocraft_codecs::MAX_EXTRA_SAMPLES] {
            let bytes = tiff_with_aux_channels(extras, alpha);
            let expected = photocraft_codecs::decode(&bytes).unwrap().meta.extra_samples;
            assert_eq!(expected.len(), extras);
            let input = dir.path().join(format!("{alpha}-{extras}.tiff"));
            std::fs::write(&input, bytes).unwrap();
            let result = Session::new().execute("file.automate.batch", json!({"input":[input],"destination":"saveAndClose","steps":[]})).unwrap();
            assert_eq!(result["files"].as_array().unwrap().len(), 1, "{result}");
            let saved = photocraft_codecs::decode(&std::fs::read(input).unwrap()).unwrap();
            assert_eq!(saved.meta.extra_samples, expected, "alpha={alpha} extras={extras}");
        }
    }
}

/// Droplet dependencies retain explicit Batch calls and rewrite their indices before packaging.
#[test]
fn called_actions_packages_explicit_batch_children_and_replays_them() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("input.png");
    std::fs::write(&input, small_image(photocraft_codecs::Format::Png)).unwrap();
    let list = vec![Action::new("Decoy", vec![]), Action::new("Child", vec![("image.imageSize".into(), json!({"width":3,"height":1,"resample":"nearest"}))])];
    for field in ["steps", "action"] {
        for shape in 0..3 {
            let step = match shape {
                0 => json!(["actions.play", {"action":1}]),
                1 => json!({"command":"actions.play","params":{"action":1},"kept":17}),
                _ => json!({"id":"actions.play","params":{"action":1},"kept":17}),
            };
            let mut params = json!({"input":[input],"output":dir.path().join(format!("{field}-{shape}"))});
            params[field] = json!([step]);
            if field == "steps" {
                params["action"] = json!(0);
            }
            let mut steps = vec![("file.automate.batch".into(), params)];
            let captured = photocraft_engine::actions_cmds::called_actions(&list, &mut steps);
            assert_eq!(captured.iter().map(|a| a.name.as_str()).collect::<Vec<_>>(), ["Child"]);
            let rewritten = &steps[0].1[field][0];
            let params = if shape == 0 { &rewritten[1] } else { &rewritten["params"] };
            assert_eq!(params["action"], "Child");
            if shape != 0 {
                assert_eq!(rewritten["kept"], 17);
            }
            let mut s = Session::new();
            s.actions.list = captured;
            let result = s.execute(&steps[0].0, steps[0].1.clone()).unwrap();
            assert_eq!(result["files"].as_array().unwrap().len(), 1, "{result}");
            let doc = photocraft_io::import("saved.png", &std::fs::read(result["files"][0].as_str().unwrap()).unwrap()).unwrap().document;
            assert_eq!(doc.size, photocraft_doc::Size::new(3, 1));
        }
    }
}

/// A deep route to a shared action must not hide children reached by a shallower playable branch.
#[test]
fn called_actions_packages_shared_descendants_at_the_shallowest_depth() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("input.png");
    std::fs::write(&input, small_image(photocraft_codecs::Format::Png)).unwrap();
    let mut list = vec![
        Action::new("Shared", vec![("actions.play".into(), json!({"action":1}))]),
        Action::new("Child", vec![("image.imageSize".into(), json!({"width":3,"height":1,"resample":"nearest"}))]),
    ];
    // The long branch reaches Shared at depth16 and its child beyond the playback limit.
    for i in 0..15 {
        list.push(Action::new(format!("A{i}"), vec![("actions.play".into(), json!({"action":if i == 14 { 0 } else { i + 3 }}))]));
    }
    for (condition, then, otherwise) in [("always", "Shared", "A0"), ("never", "A0", "Shared")] {
        let mut steps = vec![(
            "file.automate.batch".into(),
            json!({"input":[input],"output":dir.path().join(condition),"steps":[["actions.conditional",{"if":condition,"then":then,"else":otherwise}]]}),
        )];
        let captured = photocraft_engine::actions_cmds::called_actions(&list, &mut steps);
        assert_eq!(captured.iter().map(|a| &a.name).collect::<Vec<_>>(), list.iter().map(|a| &a.name).collect::<Vec<_>>());
        assert_eq!(captured[0].steps[0].1["action"], "Child");
        let mut s = Session::new();
        s.actions.list = captured;
        let result = s.execute(&steps[0].0, steps[0].1.clone()).unwrap();
        assert!(result["errors"].as_array().unwrap().is_empty(), "{result}");
        assert_eq!(result["files"].as_array().unwrap().len(), 1, "{result}");
        let saved = std::fs::read(result["files"][0].as_str().unwrap()).unwrap();
        let doc = photocraft_io::import("saved.png", &saved).unwrap().document;
        assert_eq!(doc.size, photocraft_doc::Size::new(3, 1));
    }
}

/// JSON containers do not consume action-call depth; the existing cycle and sixteen-call limits remain.
#[test]
fn called_actions_nested_batch_shapes_keep_cycle_and_action_depth_bounds() {
    let cycle = vec![Action::new("Child", vec![("actions.play".into(), json!({"action":0}))])];
    let mut step = json!(["actions.play", {"action":0}]);
    for level in 0..64 {
        let key = if level % 2 == 0 { "steps" } else { "action" };
        let mut params = json!({"kept":level});
        params[key] = json!([step]);
        step = json!({"id":"file.automate.batch","params":params});
    }
    let mut steps = vec![("file.automate.batch".into(), json!({"steps":[step]}))];
    let captured = photocraft_engine::actions_cmds::called_actions(&cycle, &mut steps);
    assert_eq!(captured.len(), 1);
    assert_eq!(captured[0].steps[0].1["action"], "Child");
    let mut node = &steps[0].1["steps"][0];
    for level in (0..64).rev() {
        let params = &node["params"];
        assert_eq!(params["kept"], level);
        node = &params[if level % 2 == 0 { "steps" } else { "action" }][0];
    }
    assert_eq!(node[1]["action"], "Child");
    let chain: Vec<_> =
        (0..17).map(|i| Action::new(format!("A{i}"), if i == 16 { vec![] } else { vec![("actions.play".into(), json!({"action":i+1}))] })).collect();
    let mut steps = vec![("file.automate.batch".into(), json!({"steps":[["actions.play",{"action":0}]]}))];
    assert_eq!(photocraft_engine::actions_cmds::called_actions(&chain, &mut steps).len(), 16);
    let mut invalid = vec![("file.automate.batch".into(), json!({"steps":null,"action":0}))];
    let before = invalid.clone();
    assert!(photocraft_engine::actions_cmds::called_actions(&chain, &mut invalid).is_empty());
    assert_eq!(invalid, before, "malformed explicit steps must not fall back to a named action");
}

/// Scripts nested through Batch retain the caller's depth and cumulative spending across failed inputs.
#[test]
fn batch_carries_script_limits_and_restores_balanced_depth() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("source.png");
    std::fs::write(&input, small_image(photocraft_codecs::Format::Png)).unwrap();
    let step = json!({"command":"file.scripts.browse","params":{"steps":[{"command":"select.all"}]}});
    for (depth, spending, allowed) in [(16, 0, false), (1, photocraft_engine::actions_cmds::MAX_PLAYBACK_STEPS - 1, false), (1, 2, true)] {
        let mut s = Session::new();
        s.actions.script_depth = depth;
        s.actions.script_steps = spending;
        let result = s.execute("file.automate.batch",json!({"input":[input,input],"output":dir.path().join("out"),"steps":[step],"namingComponents":[{"component":"serial2"},{"component":"extension"}]})).unwrap();
        assert_eq!(result["errors"].as_array().unwrap().is_empty(), allowed, "{result}");
        assert_eq!(s.actions.script_depth, depth);
        if depth == 1 {
            // Per input: the Batch's direct step (the nested script), then that script's one step
            // unless the playback limit refuses it.
            assert_eq!(s.actions.script_steps, spending + if allowed { 4 } else { 2 });
        }
    }
}

/// Structured naming with Override Open keeps the actual input encoding when format defaults to same.
#[test]
fn structured_batch_override_open_saves_default_same_signature() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("source.png");
    std::fs::write(&input, small_image(photocraft_codecs::Format::Png)).unwrap();
    let result=Session::new().execute("file.automate.batch",json!({"input":[input],"output":dir.path().join("out"),"overrideOpen":true,"namingComponents":[{"component":"name"},{"component":"extension"}],"steps":[{"command":"file.openAs","params":{"path":"recorded.jpg"}}]})).unwrap();
    assert_eq!(result["files"].as_array().unwrap().len(), 1, "{result}");
    let output = result["files"][0].as_str().unwrap();
    assert_eq!(std::path::Path::new(output).extension().unwrap(), "png");
    assert!(std::fs::read(output).unwrap().starts_with(b"\x89PNG\r\n\x1a\n"));
}
