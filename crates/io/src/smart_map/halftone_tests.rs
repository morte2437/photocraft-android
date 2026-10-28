//! Halftone Pattern descriptors with the keys from the reference application's Filter Gallery plug-in terminology (#8940):
//! `HlSz` size, `Cntr` contrast and a `ScrT` enum (`ScrD` dot, `ScrC` circle, `ScrL` line).
use super::import::{GALLERY_CLASS, GALLERY_COMMAND, colour_rgbc};
use super::*;

/// Builds an editable Halftone Pattern filter with explicit foreground/background colours.
fn filter(pattern: &str, size: i32, contrast: i32) -> SmartFilter {
    SmartFilter {
        command: GALLERY_COMMAND.into(),
        params: json!({
            "effects": [{"filter": "halftonePattern", "params": {"size": size, "contrast": contrast, "patternType": pattern}, "visible": true}],
            "foreground": [0.2, 0.4, 0.6, 1.0], "background": [0.8, 0.6, 0.4, 1.0],
        }),
        blend: BlendMode::Normal,
        opacity: 1.0,
        visible: true,
    }
}

/// Builds the inline Halftone descriptor as the reference application writes it.
fn native_effect(pattern: &str, size: i32, contrast: i32) -> Descriptor {
    let mut d = Descriptor::new("GEfc")
        .with("GEfk", enumv("GEft", "HlfS"))
        .with("HlSz", Value::Integer(size))
        .with("Cntr", Value::Integer(contrast))
        .with("ScrT", enumv("ScrT", pattern));
    d.name = UnicodeString::new_nul("Filter Gallery");
    d
}

/// Wraps a Gallery descriptor in a Photoshop smart-filter item.
fn native_item(effect: Descriptor) -> Descriptor {
    let f = filter("dot", 2, 5);
    Descriptor::new("filterFX")
        .with("Nm  ", Value::Text(UnicodeString::new_nul("Filter Gallery...")))
        .with("blendOptions", blend_options(&f))
        .with("enab", Value::Boolean(true))
        .with("hasoptions", Value::Boolean(true))
        .with("FrgC", colour_rgbc(f.params.get("foreground"), 0.0).unwrap())
        .with("BckC", colour_rgbc(f.params.get("background"), 1.0).unwrap())
        .with("Fltr", Value::Descriptor(effect))
        .with("filterID", Value::Integer(code(b"GEfc")))
}

/// Checks the pattern enums and editable settings in both mapping directions.
#[test]
fn halftone_pattern_reads_and_writes_photoshop_inline_descriptors() {
    for (pattern, native) in [("dot", "ScrD"), ("circle", "ScrC"), ("line", "ScrL")] {
        for (size, contrast) in [(1, 0), (2, 5), (7, 31), (12, 50)] {
            let expected = filter(pattern, size, contrast);
            let native = native_item(native_effect(native, size, contrast));
            assert_eq!(filter_from_item(&native), expected);
            let written = item_for_filter(&expected).unwrap();
            assert_eq!(get_desc(&written, "Fltr"), get_desc(&native, "Fltr"));
            assert_eq!(filter_from_item(&Descriptor::from_bytes(&written.to_bytes()).unwrap()), expected);
        }
    }
}

/// Keeps defaults, hidden effects, stack order, and blending intact on round trip.
#[test]
fn halftone_pattern_defaults_and_mixed_hidden_stack_round_trip() {
    let defaults = SmartFilter { params: json!({"effects": [{"filter": "halftonePattern"}]}), ..filter("dot", 1, 5) };
    assert_eq!(filter_from_item(&item_for_filter(&defaults).unwrap()).params["effects"][0]["params"], json!({"size": 1, "contrast": 5, "patternType": "dot"}));
    let mut f = filter("circle", 8, 22);
    f.params["effects"][0]["visible"] = json!(false);
    f.params["effects"].as_array_mut().unwrap().push(json!({"filter": "chrome", "params": {"detail": 4, "smoothness": 7}, "visible": true}));
    f.opacity = 0.5;
    f.blend = BlendMode::Multiply;
    f.visible = false;
    let item = item_for_filter(&f).unwrap();
    assert!(get_desc(&item, "Fltr").unwrap().get("GEfs").is_some());
    assert_eq!(filter_from_item(&Descriptor::from_bytes(&item.to_bytes()).unwrap()), f);
}

/// A default Halftone Pattern as the reference application saves it (size 1, contrast 5, Dot) opens editable.
#[test]
fn halftone_pattern_photoshop_default_imports_editable() {
    let f = filter_from_item(&native_item(native_effect("ScrD", 1, 5)));
    assert_eq!(f.command, GALLERY_COMMAND);
    assert_eq!(f.params["effects"][0]["filter"], json!("halftonePattern"));
    assert_eq!(f.params["effects"][0]["params"], json!({"size": 1, "contrast": 5, "patternType": "dot"}));
}

/// Keys the reference application's Filter Gallery doesn't use (`Sz  `, `Ptrn` of type `HlfT`) are not taken for
/// Halftone settings: the item stays opaque and is written back as it was read.
#[test]
fn halftone_pattern_foreign_keys_stay_verbatim() {
    for (pattern, shape) in [("Dt  ", "dot"), ("Crc ", "circle"), ("Ln  ", "line")] {
        let mut d = Descriptor::new("GEfc")
            .with("GEfk", enumv("GEft", "HlfS"))
            .with("Sz  ", Value::Integer(2))
            .with("Cntr", Value::Integer(5))
            .with("Ptrn", enumv("HlfT", pattern));
        d.name = UnicodeString::new_nul("Filter Gallery");
        let item = native_item(d);
        let f = filter_from_item(&item);
        assert_eq!(f.command, UNSUPPORTED_FILTER, "{shape}");
        assert_eq!(item_for_filter(&f).unwrap(), item, "{shape}");
    }
}

/// Preserves unfamiliar and malformed settings instead of substituting editable defaults.
#[test]
fn halftone_pattern_unknown_or_malformed_descriptors_stay_verbatim() {
    let valid = native_effect("ScrD", 2, 5);
    let mut invalid = vec![
        valid.clone().with("futureSetting", Value::Integer(1)),
        valid.clone().with("HlSz", Value::Integer(3)), // repeated key
    ];
    for (key, value) in [
        ("GEfk", enumv("wrong", "HlfS")),
        ("ScrT", enumv("ScrT", "????")),
        ("ScrT", enumv("wrong", "ScrD")),
        ("ScrT", Value::Integer(0)),
        ("HlSz", Value::Integer(0)),
        ("HlSz", Value::Integer(13)),
        ("HlSz", Value::Double(2.0)),
        ("Cntr", Value::Integer(-1)),
        ("Cntr", Value::Integer(51)),
        ("GELv", Value::Integer(1)),
    ] {
        let mut effect = valid.clone();
        effect.items.retain(|(id, _)| !id.is(key));
        effect.items.push((Id::new(key), value));
        invalid.push(effect);
    }
    for missing in ["HlSz", "Cntr", "ScrT"] {
        let mut effect = valid.clone();
        effect.items.retain(|(id, _)| !id.is(missing));
        invalid.push(effect);
    }
    for effect in invalid {
        let item = native_item(effect);
        let f = filter_from_item(&item);
        assert_eq!(f.command, UNSUPPORTED_FILTER);
        assert_eq!(item_for_filter(&f).unwrap(), item);
    }
}

/// Keeps a whole Gallery stack opaque when a Halftone entry has an unfamiliar class ID.
#[test]
fn halftone_pattern_unknown_nested_class_stays_verbatim() {
    let valid = native_effect("ScrD", 2, 5);
    let mut unknown = valid.clone();
    unknown.class_id = Id::new("Xxxx");
    for effects in [vec![unknown.clone()], vec![valid.clone(), unknown.clone()], vec![unknown, valid]] {
        let gallery = Descriptor::new(GALLERY_CLASS).with("GEfs", Value::List(effects.into_iter().map(Value::Descriptor).collect()));
        let item = native_item(gallery);
        let parsed = Descriptor::from_bytes(&item.to_bytes()).unwrap();
        let filter = filter_from_item(&parsed);
        assert_eq!(filter.command, UNSUPPORTED_FILTER);
        assert_eq!(item_for_filter(&filter).unwrap().to_bytes(), item.to_bytes());
    }
}

/// Rejects unsupported export values instead of silently changing their meaning.
#[test]
fn halftone_pattern_invalid_export_settings_are_reported() {
    for (key, value) in [
        ("patternType", json!("triangle")),
        ("patternType", json!(2)),
        ("size", json!(0)),
        ("size", json!(13)),
        ("size", json!("2")),
        ("size", json!(2.5)),
        ("contrast", json!(-1)),
        ("contrast", json!(51)),
        ("contrast", json!(1e100)),
        ("contrast", json!(5.5)),
        ("futureSetting", json!(1)),
    ] {
        let mut f = filter("dot", 2, 5);
        f.params["effects"][0]["params"][key] = value;
        assert!(item_for_filter(&f).is_err(), "{key}: {}", f.params);
    }
}

/// Checks complete PSD/PSB round trips for every pattern at 8-, 16-, and 32-bit depth.
#[test]
fn halftone_pattern_survives_complete_psd_and_psb_export() {
    use photocraft_doc::{Document, Layer, LayerContent, SmartObject, SmartSource};
    use photocraft_geom::Size;
    use std::sync::Arc;

    for sample in [SampleType::U8, SampleType::U16, SampleType::F32] {
        for pattern in ["dot", "circle", "line"] {
            let mut source = Document::new("synthetic contents", Size::new(2, 2), ColorMode::Rgb, sample);
            let mut pixels = Surface::new(source.pixel_format());
            pixels.fill_rect(source.bounds(), &[0.5, 0.5, 0.5, 1.0]);
            source.layers.push(Layer::new("pixels", LayerContent::Raster(pixels.clone())));
            let bytes = photocraft_format::save_to_bytes(&source, &Default::default()).unwrap();
            let mut smart =
                SmartObject::new(SmartSource::Embedded { file_name: "contents.pcraft".into(), bytes: Arc::new(bytes) }, Affine::IDENTITY, Some(pixels));
            smart.smart_filters.push(filter(pattern, 2, 5));
            let mut doc = Document::new("halftone", Size::new(2, 2), ColorMode::Rgb, sample);
            doc.layers.push(Layer::new("halftone", LayerContent::Smart(smart)));
            for ext in ["psd", "psb"] {
                let out = crate::export(&doc, ext, &crate::ExportOptions::default()).unwrap();
                assert!(!out.warnings.iter().any(|w| w.contains("smart filter")), "{ext}: {:?}", out.warnings);
                let back = crate::import(ext, &out.bytes).unwrap().document;
                let LayerContent::Smart(smart) = &back.layers[0].content else { panic!("smart object lost") };
                assert_eq!(smart.smart_filters, vec![filter(pattern, 2, 5)], "{ext} {sample:?} {pattern}");
            }
        }
    }
}
