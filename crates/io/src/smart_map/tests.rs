use super::*;

fn count(hay: &[u8], needle: &[u8]) -> usize {
    hay.windows(needle.len()).filter(|w| *w == needle).count()
}

/// A freshly built placed layer writes `warp` (key and class) as a length-4 string, as
/// Photoshop does; the code form is a different key to Photoshop.
#[test]
fn fresh_placed_warp_is_a_string_id() {
    let spec =
        PlacedSpec { idnt: "id", placed: "id", transform: Affine::IDENTITY, perspective: None, size: (10.0, 10.0), dpi: 72.0, warp: None, filter_fx: None };
    let mut warnings = Vec::new();
    let sold = sold_bytes(None, &spec, &mut warnings);
    let plld = plld_bytes(&spec, &mut warnings);
    // `SoLd`: the `warp` key and the warp descriptor's class; `PlLd`: the descriptor's class.
    for (bytes, n) in [(&sold, 2), (&plld, 1)] {
        assert_eq!(count(bytes, b"\0\0\0\x04warp"), n);
        assert_eq!(count(bytes, b"\0\0\0\0warp"), 0);
    }
    let d = sold_descriptor(&sold).unwrap();
    assert!(matches!(d.get("warp"), Some(Value::Descriptor(w)) if w.class_id == Id::Str(b"warp".to_vec())));
}

fn sf(command: &str, params: J) -> SmartFilter {
    SmartFilter { command: command.into(), params, blend: BlendMode::Normal, opacity: 1.0, visible: true }
}

#[test]
fn modelled_filters_round_trip() {
    let filters = vec![
        sf("filter.blur.gaussianBlur", json!({"radius": 4.0})),
        sf("filter.blur.boxBlur", json!({"radius": 5.0})),
        sf("filter.blur.motionBlur", json!({"angle": -20, "distance": 10.0})),
        sf("filter.blur.surfaceBlur", json!({"radius": 12.0, "threshold": 40})),
        sf("filter.blur.smartBlur", json!({"radius": 5.0, "threshold": 25.0, "quality": "high", "mode": "normal"})),
        sf("filter.blur.smartBlur", json!({"radius": 3.0, "threshold": 30.0, "quality": "medium", "mode": "overlayEdge"})),
        sf("filter.blur.smartBlur", json!({"radius": 0.5, "threshold": 0.1, "quality": "low", "mode": "edgeOnly"})),
        sf("filter.blur.radialBlur", json!({"amount": 10, "method": "spin", "quality": "draft"})),
        sf("filter.blur.radialBlur", json!({"amount": 20, "method": "zoom", "quality": "good"})),
        sf("filter.blur.radialBlur", json!({"amount": 100, "method": "spin", "quality": "best"})),
        sf("filter.sharpen.unsharpMask", json!({"amount": 150.0, "radius": 2.0, "threshold": 0})),
        sf("filter.noise.addNoise", json!({"amount": 12.0, "distribution": "uniform", "monochromatic": false, "seed": 1234})),
        sf("filter.noise.median", json!({"radius": 3.0})),
        sf("filter.other.highPass", json!({"radius": 3.0})),
        sf("filter.other.maximum", json!({"radius": 2.0})),
        sf("filter.other.minimum", json!({"radius": 2.0})),
        sf("filter.pixelate.mosaic", json!({"cellSize": 8.0})),
        sf("filter.pixelate.mezzotint", json!({"type": "mediumStrokes"})),
        sf("filter.pixelate.colorHalftone", json!({"maxRadius": 6, "channel1": 10, "channel2": 20, "channel3": 30, "channel4": 40})),
        sf("filter.stylize.emboss", json!({"angle": 135, "height": 3, "amount": 100})),
        sf("filter.sharpen.smartSharpen", json!({"amount": 120.0, "radius": 2.5, "reduceNoise": 5.0})),
        sf("filter.noise.dustAndScratches", json!({"radius": 4, "threshold": 20})),
        sf("filter.stylize.diffuse", json!({"mode": "normal"})),
        sf("filter.stylize.diffuse", json!({"mode": "darkenOnly"})),
        sf("filter.stylize.diffuse", json!({"mode": "lightenOnly"})),
        sf("filter.stylize.diffuse", json!({"mode": "anisotropic"})),
        sf("filter.stylize.traceContour", json!({"level": 60, "edge": "lower"})),
        sf("filter.stylize.traceContour", json!({"level": 128, "edge": "upper"})),
        sf("filter.stylize.traceContour", json!({"level": 0, "edge": "upper"})),
        sf("filter.stylize.traceContour", json!({"level": 255, "edge": "lower"})),
        sf("filter.pixelate.crystallize", json!({"cellSize": 10})),
        sf("filter.pixelate.crystallize", json!({"cellSize": 3})),
        sf("filter.pixelate.crystallize", json!({"cellSize": 300})),
        sf("filter.pixelate.pointillize", json!({"cellSize": 7, "background": [1.0, 1.0, 1.0, 1.0]})),
        sf("filter.pixelate.pointillize", json!({"cellSize": 300, "background": [0.0, 0.5, 1.0, 1.0]})),
        sf("filter.other.offset", json!({"horizontal": 12, "vertical": -7, "undefinedAreas": "transparent"})),
        sf("filter.other.custom", json!({"kernel": SHARPEN, "scale": 1, "offset": 0})),
        sf("image.adjustments.levels", json!({"inBlack": 20, "inWhite": 220, "gamma": 1.4, "red": {"inBlack": 0, "inWhite": 255, "gamma": 0.8}})),
        sf("image.adjustments.curves", json!({"points": [[0.0, 0.0], [64.0, 40.0], [255.0, 255.0]], "blue": [[0.0, 20.0], [255.0, 235.0]]})),
        sf("image.adjustments.shadowsHighlights", json!({})),
        sf("filter.sharpen.sharpen", json!({})),
        // Photoshop stores Dust & Scratches' radius in whole pixels (a `long`).
        sf("filter.noise.dustAndScratches", json!({"radius": 4, "threshold": 12})),
        sf(
            "image.adjustments.hueSaturation",
            json!({"colorize": false, "hue": 10, "saturation": -20, "lightness": 5, "reds": {"hue": 5, "saturation": 0, "lightness": -3, "range": [315, 345, 15, 45]}}),
        ),
        // A pencil-drawn curve: 256 output levels (`Mpng`).
        sf("image.adjustments.curves", json!({"pencil": true, "points": (0..256).rev().collect::<Vec<i32>>()})),
        sf("filter.distort.pinch", json!({"amount": -28})),
        sf("filter.distort.polarCoordinates", json!({"mode": "rectangularToPolar"})),
        sf("filter.distort.ripple", json!({"amount": 250, "size": "small"})),
        sf("filter.distort.spherize", json!({"amount": 60, "mode": "verticalOnly"})),
        sf("filter.distort.twirl", json!({"angle": -355})),
        sf(
            "filter.distort.wave",
            json!({"type": "square", "generators": 3, "wavelengthMin": 20, "wavelengthMax": 90, "amplitudeMin": 4, "amplitudeMax": 30,
                   "undefinedAreas": "repeat", "seed": 4_000_000_000u32}),
        ),
        sf("filter.distort.zigZag", json!({"amount": 30, "ridges": 7, "style": "aroundCenter"})),
        sf("image.adjustments.invert", json!({})),
    ];
    for mut f in filters.into_iter().chain(wind_extrude_tiles_settings()) {
        f.blend = BlendMode::Overlay;
        f.opacity = 0.75;
        f.visible = false;
        let d = item_for_filter(&f).unwrap();
        let bytes = d.to_bytes();
        let back = filter_from_item(&Descriptor::from_bytes(&bytes).unwrap());
        assert_eq!(back, f, "{}", f.command);
    }
}

/// The filters without options, their measured `filterID`s (as Photoshop wrote them).
const MEASURED_NO_OPTIONS: [(&str, i32); 11] = [
    ("filter.blur.blur", 1114403360),
    ("filter.blur.blurMore", 1114403405),
    ("filter.blur.average", 1098281575),
    ("filter.noise.despeckle", 1148416099),
    ("filter.stylize.findEdges", 1181639749),
    ("filter.pixelate.facet", 1180922912),
    ("filter.pixelate.fragment", 1181902701),
    ("filter.sharpen.sharpen", 1399353968),
    ("filter.sharpen.sharpenMore", 1399353933),
    ("filter.sharpen.sharpenEdges", 1399353925),
    ("filter.stylize.solarize", 1399616122),
];

/// A filter without options as Photoshop saves it: `hasoptions` false, no `Fltr`, a localized
/// name and the current colours.
fn photoshop_no_options_item(name: &str, filter_id: Value) -> Descriptor {
    let rgb = |r: f64, g: f64, b: f64| {
        Value::Descriptor(Descriptor::new("RGBC").with("Rd  ", Value::Double(r)).with("Grn ", Value::Double(g)).with("Bl  ", Value::Double(b)))
    };
    Descriptor::new("filterFX")
        .with("Nm  ", Value::Text(UnicodeString::new_nul(name)))
        .with("blendOptions", Value::Descriptor(Descriptor::new("blendOptions").with("Opct", unit(b"#Prc", 60.0)).with("Md  ", enumv("BlnM", "Mltp"))))
        .with("enab", Value::Boolean(false))
        .with("hasoptions", Value::Boolean(false))
        .with("FrgC", rgb(167.0, 157.0, 147.0))
        .with("BckC", rgb(255.0, 255.0, 255.0))
        .with("filterID", filter_id)
}

#[test]
fn filters_without_options_round_trip_in_the_measured_shape() {
    let keys = ["Nm  ", "blendOptions", "enab", "hasoptions", "FrgC", "BckC", "filterID"];
    for (command, id) in MEASURED_NO_OPTIONS {
        let mut f = sf(command, json!({}));
        f.blend = BlendMode::Overlay;
        f.opacity = 0.75;
        f.visible = false;
        let d = item_for_filter(&f).unwrap();
        let got: Vec<String> = d.items.iter().map(|(k, _)| String::from_utf8_lossy(k.as_bytes()).into_owned()).collect();
        assert_eq!(got, keys, "{command}");
        assert_eq!(d.get("hasoptions"), Some(&Value::Boolean(false)), "{command}");
        assert_eq!(d.get("filterID"), Some(&Value::Integer(id)), "{command}");
        let back = filter_from_item(&Descriptor::from_bytes(&d.to_bytes()).unwrap());
        assert_eq!(back, f, "{command}");
        // Keys an old record may carry have no effect on these filters: still written.
        let extra = item_for_filter(&sf(command, json!({"radius": 3.0, "layer": "L"}))).unwrap();
        assert_eq!(extra.get("filterID"), Some(&Value::Integer(id)), "{command}");
        for p in [json!(null), json!("x"), json!([1, 2])] {
            assert!(item_for_filter(&sf(command, p)).is_ok(), "{command}");
        }
    }
}

#[test]
fn photoshop_filters_without_options_open_editable() {
    for (command, id) in MEASURED_NO_OPTIONS {
        let item = photoshop_no_options_item("Weichzeichnen", Value::Integer(id));
        let f = filter_from_item(&Descriptor::from_bytes(&item.to_bytes()).unwrap());
        assert_eq!(f.command, command);
        assert_eq!(f.params, json!({}));
        assert_eq!((f.visible, f.blend), (false, BlendMode::Multiply));
        assert!((f.opacity - 0.6).abs() < 1e-6);
    }
}

#[test]
fn malformed_filters_without_options_are_kept_verbatim() {
    let blur = code(b"Blr ");
    let cases = [
        // An id no filter has, a missing id, ids of the wrong type.
        photoshop_no_options_item("X", Value::Integer(code(b"Zzzz"))),
        Descriptor::new("filterFX").with("hasoptions", Value::Boolean(false)),
        photoshop_no_options_item("X", Value::Double(f64::from(blur))),
        photoshop_no_options_item("X", Value::Double(f64::NAN)),
        photoshop_no_options_item("X", Value::Text(UnicodeString::new_nul("Blr "))),
        photoshop_no_options_item("X", Value::Integer(-1)),
        // Settings we don't know of on a filter that has none.
        photoshop_no_options_item("X", Value::Integer(blur)).with("Fltr", Value::Descriptor(Descriptor::new("Blr ").with("Amnt", Value::Integer(5)))),
        // Settings that aren't a descriptor, on a filter that has none.
        photoshop_no_options_item("X", Value::Integer(blur)).with("Fltr", Value::Integer(5)),
        photoshop_no_options_item("X", Value::Integer(blur)).with("Fltr", Value::Text(UnicodeString::new_nul("Blr "))),
        photoshop_no_options_item("X", Value::Integer(781)).with("Fltr", Value::Integer(5)),
    ];
    for item in cases {
        let f = filter_from_item(&item);
        assert_eq!(f.command, UNSUPPORTED_FILTER, "{item:?}");
        // Written back (a bare item gains blending and visibility) and still not editable.
        let written = item_for_filter(&f).unwrap();
        assert_eq!(written.get("Fltr"), item.get("Fltr"), "{item:?}");
        let back = filter_from_item(&written);
        assert_eq!((back.command.as_str(), &back.params["filterID"]), (UNSUPPORTED_FILTER, &f.params["filterID"]));
    }
}

/// Smart Sharpen's `Fltr` in the measured shape (no `presetKind`, no `Angl`).
fn smart_sharpen(amount: f64, radius: f64, noise: f64, blur: &str) -> Descriptor {
    Descriptor::new("smartSharpen")
        .with("useLegacy", Value::Boolean(false))
        .with("Amnt", unit(b"#Prc", amount))
        .with("Rds ", unit(b"#Pxl", radius))
        .with("noiseReduction", unit(b"#Prc", noise))
        .with("blur", enumv("blurType", blur))
}

fn dust(radius: i32, threshold: i32) -> Descriptor {
    Descriptor::new("DstS").with("Rds ", Value::Integer(radius)).with("Thsh", Value::Integer(threshold))
}

/// The `Fltr` descriptor PhotoCraft writes for `f`, after a byte round trip.
fn written_fltr(f: &SmartFilter) -> Descriptor {
    let d = Descriptor::from_bytes(&item_for_filter(f).unwrap().to_bytes()).unwrap();
    get_desc(&d, "Fltr").unwrap().clone()
}

/// Measured in Photoshop: Smart Sharpen 120 %, 2.5 px, noise 5 %, Gaussian Blur
/// (filterID 1131); the default entry adds `presetKind` first, which is not kept.
#[test]
fn smart_sharpen_photoshop_entry_opens_and_writes_back() {
    let fltr = smart_sharpen(120.0, 2.5, 5.0, "GsnB");
    let item = ps_item("Smart Sharpen...", fltr.clone(), 1131);
    let f = filter_from_item(&Descriptor::from_bytes(&item.to_bytes()).unwrap());
    assert_eq!(f.command, "filter.sharpen.smartSharpen");
    assert_eq!(f.params, json!({"amount": 120.0, "radius": 2.5, "reduceNoise": 5.0}));
    assert_eq!(written_fltr(&f), fltr);
    let d = item_for_filter(&f).unwrap();
    assert_eq!(int(&d, "filterID"), Some(1131));
    assert_eq!(get_desc(&d, "Fltr").unwrap().class_id, Id::Str(b"smartSharpen".to_vec()));
    // The four-character keys and enum values are codes, as Photoshop writes them.
    let bytes = d.to_bytes();
    assert_eq!(count(&bytes, b"\0\0\0\0blurenum\0\0\0\x08blurType\0\0\0\0GsnB"), 1);
    assert_eq!(count(&bytes, b"\0\0\0\0Rds UntF#Pxl"), 1);

    // With `presetKind` (any value): opens, but the params hold only the engine's own keys
    // (it rejects unknown ones), and a re-save leaves it out as Photoshop does for explicit
    // values.
    for kind in ["presetKindDefault", "presetKindCustom", "Xxxx"] {
        let mut with_preset = Descriptor::new("smartSharpen").with("presetKind", enumv("presetKindType", kind));
        with_preset.items.extend(smart_sharpen(200.0, 1.0, 10.0, "GsnB").items);
        let f = filter_from_item(&Descriptor::from_bytes(&ps_item("Smart Sharpen...", with_preset, 1131).to_bytes()).unwrap());
        assert_eq!(f.command, "filter.sharpen.smartSharpen");
        let keys: Vec<&str> = f.params.as_object().unwrap().keys().map(String::as_str).collect();
        assert_eq!(keys.len(), 3, "{keys:?}");
        assert!(keys.iter().all(|k| ["amount", "radius", "reduceNoise"].contains(k)), "{keys:?}");
        assert_eq!(f.params, json!({"amount": 200.0, "radius": 1.0, "reduceNoise": 10.0}));
        assert_eq!(written_fltr(&f), smart_sharpen(200.0, 1.0, 10.0, "GsnB"));
    }

    // Without options PhotoCraft writes the engine's defaults, Gaussian Blur.
    assert_eq!(written_fltr(&sf("filter.sharpen.smartSharpen", json!({}))), smart_sharpen(100.0, 1.0, 10.0, "GsnB"));
}

/// Lens Blur (Photoshop's default), Motion Blur and legacy mode render differently from
/// PhotoCraft's Smart Sharpen, so those entries stay verbatim.
#[test]
fn smart_sharpen_settings_we_dont_render_stay_verbatim() {
    let mut default_entry = Descriptor::new("smartSharpen").with("presetKind", enumv("presetKindType", "presetKindDefault"));
    default_entry.items.extend(smart_sharpen(200.0, 1.0, 10.0, "lensBlur").items);
    let motion = smart_sharpen(80.0, 1.5, 0.0, "MtnB").with("Angl", Value::Integer(30));
    let mut legacy = smart_sharpen(100.0, 1.0, 10.0, "GsnB");
    set(&mut legacy, "useLegacy", Value::Boolean(true));
    for fltr in [default_entry, motion, legacy] {
        let item = ps_item("Smart Sharpen...", fltr, 1131);
        let f = filter_from_item(&item);
        assert_eq!(f.command, UNSUPPORTED_FILTER);
        assert_eq!(f.params["filterID"], 1131);
        assert_eq!(item_for_filter(&f).unwrap(), item);
    }
}

#[test]
fn malformed_smart_sharpen_is_kept_verbatim() {
    let g = || smart_sharpen(100.0, 1.0, 10.0, "GsnB");
    let without = |key: &str| {
        let mut d = g();
        d.items.retain(|(k, _)| !k.is(key));
        d
    };
    let with = |key: &str, v: Value| {
        let mut d = g();
        set(&mut d, key, v);
        d
    };
    let cases = vec![
        without("useLegacy"),
        without("Amnt"),
        without("Rds "),
        without("noiseReduction"),
        without("blur"),
        with("Amnt", Value::Double(100.0)),
        with("Amnt", unit(b"#Pxl", 100.0)),
        with("Rds ", Value::Integer(1)),
        with("useLegacy", Value::Integer(0)),
        with("Amnt", unit(b"#Prc", 0.0)),
        with("Amnt", unit(b"#Prc", 501.0)),
        with("Rds ", unit(b"#Pxl", 0.05)),
        with("Rds ", unit(b"#Pxl", 65.0)),
        with("Rds ", unit(b"#Pxl", f64::NAN)),
        with("noiseReduction", unit(b"#Prc", -1.0)),
        with("noiseReduction", unit(b"#Prc", f64::INFINITY)),
        with("blur", enumv("blurType", "Xxxx")),
        with("blur", enumv("BlrM", "GsnB")),
        with("blur", Value::Integer(0)),
        with("presetKind", Value::Integer(1)),
        g().with("Angl", Value::Integer(30)),
        g().with("shadowMode", Value::Descriptor(Descriptor::new("adaptCorrectTones"))),
    ];
    for fltr in cases {
        let item = ps_item("Smart Sharpen...", fltr, 1131);
        let f = filter_from_item(&Descriptor::from_bytes(&item.to_bytes()).unwrap());
        assert_eq!(f.command, UNSUPPORTED_FILTER, "{:?}", get_desc(&item, "Fltr"));
        assert_eq!(item_for_filter(&f).unwrap().to_bytes(), item.to_bytes());
    }
    // No `Fltr` at all (hasoptions false).
    let bare = Descriptor::new("filterFX").with("filterID", Value::Integer(1131));
    assert_eq!(filter_from_item(&bare).command, UNSUPPORTED_FILTER);
}

/// Measured in Photoshop: Dust & Scratches at its defaults (1, 0) and at 4, 20 (filterID
/// `DstS`, 1148417107; both settings `long`).
#[test]
fn dust_and_scratches_photoshop_entry_opens_and_writes_back() {
    for (radius, threshold) in [(1, 0), (4, 20), (500, 255)] {
        let fltr = dust(radius, threshold);
        let item = ps_item("Dust & Scratches...", fltr.clone(), 1_148_417_107);
        let f = filter_from_item(&Descriptor::from_bytes(&item.to_bytes()).unwrap());
        assert_eq!(f.command, "filter.noise.dustAndScratches");
        assert_eq!(f.params, json!({"radius": radius, "threshold": threshold}));
        assert_eq!(written_fltr(&f), fltr);
        assert_eq!(int(&item_for_filter(&f).unwrap(), "filterID"), Some(1_148_417_107));
    }
    // No `Fltr` (hasoptions false): kept verbatim.
    let bare = Descriptor::new("filterFX").with("filterID", Value::Integer(code(b"DstS")));
    assert_eq!(filter_from_item(&bare).command, UNSUPPORTED_FILTER);
}

#[test]
fn malformed_dust_and_scratches_is_kept_verbatim() {
    let cases = vec![
        Descriptor::new("DstS").with("Thsh", Value::Integer(0)),
        Descriptor::new("DstS").with("Rds ", Value::Integer(1)),
        Descriptor::new("DstS").with("Rds ", Value::Double(1.0)).with("Thsh", Value::Integer(0)),
        Descriptor::new("DstS").with("Rds ", unit(b"#Pxl", 1.0)).with("Thsh", Value::Integer(0)),
        Descriptor::new("DstS").with("Rds ", Value::Integer(1)).with("Thsh", Value::Boolean(false)),
        dust(0, 0),
        dust(501, 0),
        dust(1, -1),
        dust(1, 256),
        dust(i32::MIN, i32::MAX),
        dust(1, 0).with("Xtra", Value::Integer(1)),
    ];
    for fltr in cases {
        let item = ps_item("Dust & Scratches...", fltr, code(b"DstS"));
        let f = filter_from_item(&item);
        assert_eq!(f.command, UNSUPPORTED_FILTER, "{:?}", get_desc(&item, "Fltr"));
        assert_eq!(item_for_filter(&f).unwrap(), item);
    }
}

/// Every option's range ends round trip; hostile params are written in range (the engine
/// clamps the same way), never a panic.
#[test]
fn smart_sharpen_and_dust_option_values_round_trip() {
    let mut cases = Vec::new();
    for amount in [1.0, 100.0, 500.0] {
        for radius in [0.1, 1.0, 64.0] {
            for noise in [0.0, 10.0, 100.0] {
                cases.push(sf("filter.sharpen.smartSharpen", json!({"amount": amount, "radius": radius, "reduceNoise": noise})));
            }
        }
    }
    for radius in [1, 2, 100, 500] {
        for threshold in [0, 1, 128, 255] {
            cases.push(sf("filter.noise.dustAndScratches", json!({"radius": radius, "threshold": threshold})));
        }
    }
    for f in cases {
        let back = filter_from_item(&Descriptor::from_bytes(&item_for_filter(&f).unwrap().to_bytes()).unwrap());
        assert_eq!(back, f);
    }
    let defaults = json!({"amount": 100.0, "radius": 1.0, "reduceNoise": 10.0});
    for (command, p, want) in [
        (
            "filter.sharpen.smartSharpen",
            json!({"amount": 1e9, "radius": -5.0, "reduceNoise": "x"}),
            json!({"amount": 500.0, "radius": 0.1, "reduceNoise": 10.0}),
        ),
        ("filter.sharpen.smartSharpen", json!({"presetKind": "presetKindDefault", "amount": null}), defaults.clone()),
        ("filter.sharpen.smartSharpen", json!(null), defaults.clone()),
        ("filter.noise.dustAndScratches", json!({"radius": 2.4, "threshold": 1e12}), json!({"radius": 2, "threshold": 255})),
        ("filter.noise.dustAndScratches", json!({"radius": -3, "threshold": -3}), json!({"radius": 1, "threshold": 0})),
        ("filter.noise.dustAndScratches", json!([]), json!({"radius": 1, "threshold": 0})),
    ] {
        let back = filter_from_item(&item_for_filter(&sf(command, p.clone())).unwrap());
        assert_eq!((back.command.as_str(), &back.params), (command, &want), "{p}");
    }
}

/// Every option value of Wind, Extrude and Tiles (and the ends of their ranges).
fn wind_extrude_tiles_settings() -> Vec<SmartFilter> {
    let mut v = Vec::new();
    for method in ["wind", "blast", "stagger"] {
        for direction in ["fromRight", "fromLeft"] {
            v.push(sf("filter.stylize.wind", json!({"method": method, "direction": direction})));
        }
    }
    for t in ["blocks", "pyramids"] {
        for mode in ["random", "levelBased"] {
            for (solid, mask) in [(false, false), (true, false), (false, true), (true, true)] {
                v.push(sf(
                    "filter.stylize.extrude",
                    json!({"type": t, "size": 7, "depth": 9, "depthMode": mode, "solidFrontFaces": solid, "maskIncompleteBlocks": mask}),
                ));
            }
        }
    }
    v.push(sf(
        "filter.stylize.extrude",
        json!({"type": "blocks", "size": 2, "depth": 1, "depthMode": "random", "solidFrontFaces": false, "maskIncompleteBlocks": false}),
    ));
    v.push(sf(
        "filter.stylize.extrude",
        json!({"type": "pyramids", "size": 255, "depth": 255, "depthMode": "levelBased", "solidFrontFaces": true, "maskIncompleteBlocks": true}),
    ));
    for fill in ["background", "foreground", "inverse", "unaltered"] {
        v.push(sf(
            "filter.stylize.tiles",
            json!({"count": 6, "maxOffset": 20, "fill": fill, "foreground": [1.0, 0.0, 0.5, 1.0], "background": [0.0, 0.5, 1.0, 1.0]}),
        ));
    }
    for (count, offset) in [(1, 1), (99, 99)] {
        v.push(sf(
            "filter.stylize.tiles",
            json!({"count": count, "maxOffset": offset, "fill": "background", "foreground": [0.0, 0.0, 0.0, 1.0], "background": [1.0, 1.0, 1.0, 1.0]}),
        ));
    }
    v
}

/// A `filterFX` entry shaped as Photoshop saves Wind, Extrude and Tiles (measured in Photoshop:
/// applied through Action Manager to a smart object, saved, `filterFX` read back): the item's
/// `FrgC`/`BckC` are the document colours, `Fltr` holds the settings, `filterID` the class code.
fn measured_item(fltr: Descriptor, fg: f64, bg: f64) -> Descriptor {
    let id = code(fltr.class_id.as_bytes().try_into().unwrap());
    Descriptor::new("filterFX")
        .with("Nm  ", Value::Text(UnicodeString::new_nul("Filter...")))
        .with("blendOptions", blend_options(&sf("", J::Null)))
        .with("enab", Value::Boolean(true))
        .with("hasoptions", Value::Boolean(true))
        .with("FrgC", rgbc(fg))
        .with("BckC", rgbc(bg))
        .with("Fltr", Value::Descriptor(fltr))
        .with("filterID", Value::Integer(id))
}

/// The `Fltr` descriptors Photoshop 27 wrote for these filters (one smart object each, saved as
/// PSD and read back), with the settings PhotoCraft reads from them.
const PHOTOSHOP_FLTR: [(&str, &str, &str); 6] = [
    ("filter.distort.pinch", r#"{"amount":-28}"#, "0000000600500069006e00630068000000000000506e63680000000100000000416d6e746c6f6e67ffffffe4"),
    ("filter.distort.twirl", r#"{"angle":-355}"#, "000000060054007700690072006c0000000000005477726c0000000100000000416e676c6c6f6e67fffffe9d"),
    (
        "filter.distort.ripple",
        r#"{"amount":-273,"size":"large"}"#,
        "000000070052006900700070006c006500000000000052706c650000000200000000416d6e746c6f6e67fffffeef0000000052706c53656e756d0000000052706c53000000004c726720",
    ),
    (
        "filter.distort.spherize",
        r#"{"amount":-1,"mode":"horizontalOnly"}"#,
        "00000009005300700068006500720069007a0065000000000000537068720000000200000000416d6e746c6f6e67ffffffff000000005370684d656e756d000000005370684d0000000048727a4f",
    ),
    (
        "filter.distort.polarCoordinates",
        r#"{"mode":"polarToRectangular"}"#,
        "000000120050006f006c0061007200200043006f006f007200640069006e0061007400650073000000000000506c72200000000100000000436e7672656e756d00000000436e767200000000506c7252",
    ),
    (
        "filter.distort.zigZag",
        r#"{"amount":-42,"ridges":11,"style":"pondRipples"}"#,
        "00000007005a00690067005a006100670000000000005a675a670000000300000000416d6e746c6f6e67ffffffd6000000004e6d62526c6f6e670000000b000000005a5a5479656e756d000000005a5a547900000000506e6452",
    ),
];

/// Photoshop's Wave with Scale 100% / 28%: PhotoCraft's Wave has no Scale.
const PHOTOSHOP_WAVE: &str = "000000050057006100760065000000000000576176650000000a0000000057767470656e756d00000000577674700000000057765472000000004e6d62476c6f6e670000014c00000000574c4d6e6c6f6e67000000d500000000574c4d786c6f6e67000000d600000000416d4d6e6c6f6e67000000f000000000416d4d786c6f6e67000000f10000000053636c486c6f6e67000000640000000053636c566c6f6e670000001c00000000556e6441656e756d00000000556e6441000000005772704100000000526e64536c6f6e670013624a";

/// A `filterFX` list item as Photoshop writes it (enabled, Normal at 100%, with options), wrapping the `Fltr` descriptor `fltr`.
fn photoshop_fltr_item(fltr: Descriptor) -> Descriptor {
    let id = code(fltr.class_id.as_bytes().try_into().unwrap());
    Descriptor::new("filterFX")
        .with("Nm  ", Value::Text(UnicodeString::new_nul("from Photoshop")))
        .with("blendOptions", Value::Descriptor(Descriptor::new("blendOptions").with("Opct", unit(b"#Prc", 100.0)).with("Md  ", enumv("BlnM", "normal"))))
        .with("enab", Value::Boolean(true))
        .with("hasoptions", Value::Boolean(true))
        .with("Fltr", Value::Descriptor(fltr))
        .with("filterID", Value::Integer(id))
}

fn wind(method: &str, direction: &str) -> Descriptor {
    Descriptor::new("Wnd ").with("WndM", enumv("WndM", method)).with("Drct", enumv("Drct", direction))
}

fn extrude(t: &str, size: Value, depth: Value, mode: &str, solid: Value, mask: Value) -> Descriptor {
    Descriptor::new("Extr")
        .with("ExtT", enumv("ExtT", t))
        .with("ExtS", size)
        .with("ExtD", depth)
        .with("ExtR", enumv("ExtR", mode))
        .with("ExtF", solid)
        .with("ExtM", mask)
}

fn tiles(count: Value, offset: Value, fill: &str) -> Descriptor {
    Descriptor::new("Tls ").with("TlNm", count).with("TlOf", offset).with("FlCl", enumv("FlCl", fill))
}

#[test]
fn wind_extrude_tiles_open_from_photoshop_shapes() {
    let (long, yes, no) = (Value::Integer, Value::Boolean(true), Value::Boolean(false));
    let cases = [
        // The measured files.
        (wind("Blst", "Left"), "filter.stylize.wind", json!({"method": "blast", "direction": "fromRight"})),
        (wind("Stgr", "Rght"), "filter.stylize.wind", json!({"method": "stagger", "direction": "fromLeft"})),
        (wind("Wnd ", "Rght"), "filter.stylize.wind", json!({"method": "wind", "direction": "fromLeft"})),
        (
            extrude("Blks", long(7), long(9), "Rndm", yes.clone(), no.clone()),
            "filter.stylize.extrude",
            json!({"type": "blocks", "size": 7, "depth": 9, "depthMode": "random", "solidFrontFaces": true, "maskIncompleteBlocks": false}),
        ),
        (
            extrude("Pyrm", long(12), long(20), "LvlB", no, yes),
            "filter.stylize.extrude",
            json!({"type": "pyramids", "size": 12, "depth": 20, "depthMode": "levelBased", "solidFrontFaces": false, "maskIncompleteBlocks": true}),
        ),
        (
            tiles(long(6), long(20), "FlFr"),
            "filter.stylize.tiles",
            json!({"count": 6, "maxOffset": 20, "fill": "foreground", "foreground": [0.2, 0.2, 0.2, 1.0], "background": [1.0, 1.0, 1.0, 1.0]}),
        ),
        (
            tiles(long(4), long(5), "FlIn"),
            "filter.stylize.tiles",
            json!({"count": 4, "maxOffset": 5, "fill": "inverse", "foreground": [0.2, 0.2, 0.2, 1.0], "background": [1.0, 1.0, 1.0, 1.0]}),
        ),
        (
            tiles(long(10), long(10), "FlBc"),
            "filter.stylize.tiles",
            json!({"count": 10, "maxOffset": 10, "fill": "background", "foreground": [0.2, 0.2, 0.2, 1.0], "background": [1.0, 1.0, 1.0, 1.0]}),
        ),
        (
            tiles(long(99), long(1), "FlSm"),
            "filter.stylize.tiles",
            json!({"count": 99, "maxOffset": 1, "fill": "unaltered", "foreground": [0.2, 0.2, 0.2, 1.0], "background": [1.0, 1.0, 1.0, 1.0]}),
        ),
    ];
    for (fltr, command, params) in cases {
        let item = measured_item(fltr, 51.0, 255.0);
        // Through bytes, as read from a file.
        let f = filter_from_item(&Descriptor::from_bytes(&item.to_bytes()).unwrap());
        assert_eq!((f.command.as_str(), &f.params), (command, &params));
        // Written back in the measured shape: same class, keys and their order, value types
        // (long, bool, enum with its type), filterID; Tiles also its colours.
        let back = item_for_filter(&f).unwrap();
        assert_eq!(get_desc(&back, "Fltr").map(Descriptor::to_bytes), get_desc(&item, "Fltr").map(Descriptor::to_bytes), "{command}");
        assert_eq!(back.get("filterID"), item.get("filterID"), "{command}");
        if command == "filter.stylize.tiles" {
            assert_eq!((back.get("FrgC"), back.get("BckC")), (item.get("FrgC"), item.get("BckC")));
        }
    }
    // The class codes: numeric filterIDs of the four-char classes.
    for (c, id) in [("filter.stylize.wind", 1466852384), ("filter.stylize.extrude", 1165522034), ("filter.stylize.tiles", 1416393504)] {
        assert_eq!(item_for_filter(&sf(c, json!({}))).unwrap().get("filterID"), Some(&Value::Integer(id)));
    }
    // A filter found by its filterID alone (no `Fltr` class to go by) still needs its settings.
    let bare = Descriptor::new("filterFX").with("hasoptions", Value::Boolean(false)).with("filterID", Value::Integer(1466852384));
    assert_eq!(filter_from_item(&bare).command, UNSUPPORTED_FILTER);
    // Only Tiles reads the entry's colours; Wind and Extrude write the defaults.
    let f = filter_from_item(&measured_item(wind("Wnd ", "Left"), 51.0, 0.0));
    assert_eq!(f.params, json!({"method": "wind", "direction": "fromRight"}));
    let back = item_for_filter(&f).unwrap();
    assert_eq!((back.get("FrgC"), back.get("BckC")), (Some(&rgbc(0.0)), Some(&rgbc(255.0))));
    // Tiles without colours in its params (or the entry) writes the engine's black and white.
    let f = filter_from_item(&Descriptor::new("filterFX").with("Fltr", Value::Descriptor(tiles(long(4), long(5), "FlBc"))));
    assert_eq!(f.params, json!({"count": 4, "maxOffset": 5, "fill": "background"}));
    let back = item_for_filter(&f).unwrap();
    assert_eq!((back.get("FrgC"), back.get("BckC")), (Some(&rgbc(0.0)), Some(&rgbc(255.0))));
}

#[test]
fn wind_extrude_tiles_write_defaults_and_photoshop_ranges() {
    let fltr = |c: &str, p: J| match item_for_filter(&sf(c, p)).unwrap().get("Fltr") {
        Some(Value::Descriptor(d)) => d.clone(),
        other => panic!("no Fltr: {other:?}"),
    };
    let long = Value::Integer;
    let (yes, no) = (Value::Boolean(true), Value::Boolean(false));
    // Defaults are the engine's; our random seed has no PSD key and is not written.
    assert_eq!(fltr("filter.stylize.wind", json!({"seed": 9})), wind("Wnd ", "Left"));
    assert_eq!(fltr("filter.stylize.extrude", json!({"seed": 9})), extrude("Blks", long(30), long(30), "Rndm", no.clone(), no.clone()));
    assert_eq!(fltr("filter.stylize.tiles", json!({"seed": 9})), tiles(long(10), long(10), "FlBc"));
    // Out of range, fractional, non-numeric and non-bool values as the engine reads them.
    assert_eq!(
        fltr("filter.stylize.extrude", json!({"size": 1e12, "depth": -5, "solidFrontFaces": "yes", "maskIncompleteBlocks": true})),
        extrude("Blks", long(255), long(1), "Rndm", no.clone(), yes)
    );
    assert_eq!(fltr("filter.stylize.tiles", json!({"count": 0, "maxOffset": 12.6})), tiles(long(1), long(13), "FlBc"));
    assert_eq!(fltr("filter.stylize.tiles", json!({"count": "many", "maxOffset": 1000})), tiles(long(10), long(99), "FlBc"));
    assert_eq!(fltr("filter.stylize.extrude", json!({"size": f64::NAN})), extrude("Blks", long(30), long(30), "Rndm", no.clone(), no));
}

#[test]
fn malformed_wind_extrude_tiles_are_kept_verbatim() {
    let long = Value::Integer;
    let (yes, no) = (Value::Boolean(true), Value::Boolean(false));
    let cases = [
        // Missing keys.
        Descriptor::new("Wnd "),
        Descriptor::new("Wnd ").with("WndM", enumv("WndM", "Blst")),
        Descriptor::new("Wnd ").with("Drct", enumv("Drct", "Left")),
        Descriptor::new("Extr").with("ExtT", enumv("ExtT", "Blks")).with("ExtS", long(7)).with("ExtD", long(9)).with("ExtR", enumv("ExtR", "Rndm")),
        Descriptor::new("Tls ").with("TlNm", long(6)).with("TlOf", long(20)),
        Descriptor::new("Tls ").with("TlNm", long(6)).with("FlCl", enumv("FlCl", "FlFr")),
        // Unknown enum values and enum types (Photoshop falls back to a default for these;
        // we keep them as they are rather than guess).
        wind("Wild", "Left"),
        wind("Blst", "Up  "),
        Descriptor::new("Wnd ").with("WndM", enumv("Drct", "Blst")).with("Drct", enumv("Drct", "Left")),
        extrude("Cube", long(7), long(9), "Rndm", yes.clone(), no.clone()),
        extrude("Blks", long(7), long(9), "Sine", yes.clone(), no.clone()),
        tiles(long(6), long(20), "FlXx"),
        Descriptor::new("Tls ").with("TlNm", long(6)).with("TlOf", long(20)).with("FlCl", enumv("BlnM", "FlFr")),
        // Wrong types (also NaN).
        Descriptor::new("Wnd ").with("WndM", long(1)).with("Drct", enumv("Drct", "Left")),
        extrude("Blks", Value::Double(7.0), long(9), "Rndm", yes.clone(), no.clone()),
        extrude("Blks", long(7), unit(b"#Pxl", 9.0), "Rndm", yes.clone(), no.clone()),
        extrude("Blks", long(7), long(9), "Rndm", long(1), no.clone()),
        extrude("Blks", long(7), long(9), "Rndm", yes.clone(), Value::Double(f64::NAN)),
        tiles(Value::Double(f64::NAN), long(20), "FlFr"),
        tiles(long(6), Value::Text(UnicodeString::new_nul("20")), "FlFr"),
        // Out of range.
        extrude("Blks", long(1), long(9), "Rndm", yes.clone(), no.clone()),
        extrude("Blks", long(256), long(9), "Rndm", yes.clone(), no.clone()),
        extrude("Blks", long(7), long(0), "Rndm", yes.clone(), no.clone()),
        extrude("Blks", long(7), long(i32::MAX), "Rndm", yes, no),
        tiles(long(0), long(20), "FlFr"),
        tiles(long(100), long(20), "FlFr"),
        tiles(long(6), long(0), "FlFr"),
        tiles(long(6), long(i32::MIN), "FlFr"),
    ];
    for fltr in cases {
        let item = measured_item(fltr, 0.0, 255.0);
        let f = filter_from_item(&Descriptor::from_bytes(&item.to_bytes()).unwrap());
        assert_eq!(f.command, UNSUPPORTED_FILTER, "{item:?}");
        assert_eq!(item_for_filter(&f).unwrap().to_bytes(), item.to_bytes());
    }
    // A Tiles entry whose colour is unreadable is kept verbatim too.
    for (key, bad) in [
        ("FrgC", Value::Integer(3)),
        ("BckC", Value::Descriptor(Descriptor::new("RGBC").with("Rd  ", Value::Double(0.0)))),
        (
            "BckC",
            Value::Descriptor(Descriptor::new("RGBC").with("Rd  ", Value::Double(f64::NAN)).with("Grn ", Value::Double(0.0)).with("Bl  ", Value::Double(0.0))),
        ),
    ] {
        let mut item = measured_item(tiles(long(6), long(20), "FlFr"), 0.0, 255.0);
        set(&mut item, key, bad);
        assert_eq!(filter_from_item(&item).command, UNSUPPORTED_FILTER);
    }
    // Values with no PSD equivalent are reported, not guessed.
    for (c, p) in [
        ("filter.stylize.wind", json!({"method": "gale"})),
        ("filter.stylize.wind", json!({"direction": "fromTop"})),
        ("filter.stylize.wind", json!({"direction": 1})),
        ("filter.stylize.extrude", json!({"type": "cubes"})),
        ("filter.stylize.extrude", json!({"depthMode": null})),
        ("filter.stylize.tiles", json!({"fill": "transparent"})),
        ("filter.stylize.tiles", json!({"foreground": "teal"})),
        ("filter.stylize.tiles", json!({"background": [1.0]})),
        ("filter.stylize.tiles", json!({"background": [f64::NAN, 0.0, 0.0]})),
    ] {
        assert!(item_for_filter(&sf(c, p.clone())).is_err(), "{c} {p}");
    }
}

/// Photoshop's default Custom kernel: a 3 × 3 sharpen centred in the 5 × 5.
const SHARPEN: [i32; 25] = [0, 0, 0, 0, 0, 0, 0, -1, 0, 0, 0, -1, 5, -1, 0, 0, 0, -1, 0, 0, 0, 0, 0, 0, 0];

/// A `filterFXList` item as measured in Photoshop (applied through Action Manager to a smart
/// object, saved, `filterFX` read back).
fn measured_named_item(id: &[u8; 4], name: &str, fltr: Descriptor) -> Descriptor {
    Descriptor::new("filterFX")
        .with("Nm  ", Value::Text(UnicodeString::new_nul(name)))
        .with("blendOptions", blend_options(&sf("", J::Null)))
        .with("enab", Value::Boolean(true))
        .with("hasoptions", Value::Boolean(true))
        .with("FrgC", rgbc(0.0))
        .with("BckC", rgbc(255.0))
        .with("Fltr", Value::Descriptor(fltr))
        .with("filterID", Value::Integer(code(id)))
}

fn offset_fltr(h: Value, v: Value, fill: Value) -> Descriptor {
    Descriptor::new("Ofst").with("Hrzn", h).with("Vrtc", v).with("Fl  ", fill)
}

fn custom_fltr(scale: Value, offset: Value, m: Vec<Value>) -> Descriptor {
    Descriptor::new("Cstm").with("Scl ", scale).with("Ofst", offset).with("Mtrx", Value::List(m))
}

fn longs(m: &[i32]) -> Vec<Value> {
    m.iter().map(|v| Value::Integer(*v)).collect()
}

#[test]
fn offset_round_trips_every_fill_mode() {
    for (fill, ours) in [("Wrp ", "wrap"), ("Rpt ", "repeat"), ("Bckg", "transparent")] {
        for (h, v) in [(0, 0), (12, -7), (5, 3), (i32::MIN, i32::MAX)] {
            let f = sf("filter.other.offset", json!({"horizontal": h, "vertical": v, "undefinedAreas": ours}));
            let d = item_for_filter(&f).unwrap();
            let fltr = get_desc(&d, "Fltr").unwrap();
            assert_eq!(*fltr, offset_fltr(Value::Integer(h), Value::Integer(v), enumv("FlMd", fill)));
            assert_eq!(int(&d, "filterID"), Some(1_332_114_292));
            assert_eq!(filter_from_item(&Descriptor::from_bytes(&d.to_bytes()).unwrap()), f);
        }
    }
}

/// The measured descriptors (Wrap 0/0, Set to Background 12/−7, Repeat 5/3) open as Offset.
#[test]
fn offset_measured_descriptors_open_as_offset() {
    for (h, v, fill, ours) in [(0, 0, "Wrp ", "wrap"), (12, -7, "Bckg", "transparent"), (5, 3, "Rpt ", "repeat")] {
        let item = measured_named_item(b"Ofst", "Offset...", offset_fltr(Value::Integer(h), Value::Integer(v), enumv("FlMd", fill)));
        let f = filter_from_item(&item);
        assert_eq!(f.command, "filter.other.offset");
        assert_eq!(f.params, json!({"horizontal": h, "vertical": v, "undefinedAreas": ours}));
        // Written back exactly as measured.
        assert_eq!(item_for_filter(&f).unwrap(), item);
    }
}

#[test]
fn offset_written_with_engine_defaults() {
    // No params: 0, 0, Wrap Around (the engine's defaults); fractional pixels round.
    let d = item_for_filter(&sf("filter.other.offset", json!({}))).unwrap();
    assert_eq!(*get_desc(&d, "Fltr").unwrap(), offset_fltr(Value::Integer(0), Value::Integer(0), enumv("FlMd", "Wrp ")));
    let d = item_for_filter(&sf("filter.other.offset", json!({"horizontal": 2.6, "vertical": 3e12, "undefinedAreas": "bogus"}))).unwrap();
    assert_eq!(*get_desc(&d, "Fltr").unwrap(), offset_fltr(Value::Integer(3), Value::Integer(i32::MAX), enumv("FlMd", "Wrp ")));
}

#[test]
fn malformed_offset_is_kept_verbatim() {
    let i = Value::Integer;
    for fltr in [
        offset_fltr(i(1), i(2), enumv("FlMd", "Xxxx")),
        offset_fltr(i(1), i(2), Value::Integer(1)),
        offset_fltr(Value::Double(1.5), i(2), enumv("FlMd", "Wrp ")),
        offset_fltr(i(1), unit(b"#Pxl", f64::NAN), enumv("FlMd", "Wrp ")),
        Descriptor::new("Ofst").with("Hrzn", i(1)).with("Fl  ", enumv("FlMd", "Wrp ")),
        Descriptor::new("Ofst"),
        // A mistyped enum, a setting we don't write back and a repeated one.
        offset_fltr(i(1), i(2), enumv("Xxxx", "Wrp ")),
        offset_fltr(i(1), i(2), enumv("FlMd", "Wrp ")).with("Xtra", i(1)),
        offset_fltr(i(1), i(2), enumv("FlMd", "Wrp ")).with("Hrzn", i(3)),
    ] {
        let item = measured_named_item(b"Ofst", "Offset...", fltr);
        let f = filter_from_item(&item);
        assert_eq!(f.command, UNSUPPORTED_FILTER, "{item:?}");
        assert_eq!(item_for_filter(&f).unwrap().to_bytes(), item.to_bytes(), "kept byte for byte");
    }
    let bare = Descriptor::new("filterFX").with("filterID", Value::Integer(code(b"Ofst")));
    assert_eq!(filter_from_item(&bare).command, UNSUPPORTED_FILTER);
}

#[test]
fn custom_round_trips() {
    let edge: Vec<i32> = (0..25)
        .map(|i| {
            if i == 12 {
                8
            } else if (6..=18).contains(&i) && i % 5 != 0 && i % 5 != 4 {
                -1
            } else {
                0
            }
        })
        .collect();
    let full: Vec<i32> = (0..25).map(|i| i * 83 - 999).collect();
    for (kernel, scale, offset) in [(SHARPEN.to_vec(), 1, 0), (edge, 2, 10), (full, 9999, -9999), (vec![999; 25], 1, 9999)] {
        let f = sf("filter.other.custom", json!({"kernel": kernel, "scale": scale, "offset": offset}));
        let d = item_for_filter(&f).unwrap();
        assert_eq!(*get_desc(&d, "Fltr").unwrap(), custom_fltr(Value::Integer(scale), Value::Integer(offset), longs(&kernel)));
        assert_eq!(int(&d, "filterID"), Some(1_131_639_917));
        assert_eq!(filter_from_item(&Descriptor::from_bytes(&d.to_bytes()).unwrap()), f);
    }
}

/// The measured descriptors (the default sharpen; a blur at Scale 2, Offset 10) open as Custom.
#[test]
fn custom_measured_descriptors_open_as_custom() {
    let blur = [0, 0, 0, 0, 0, 0, 1, 2, 1, 0, 0, 2, 4, 2, 0, 0, 1, 2, 1, 0, 0, 0, 0, 0, 0];
    for (kernel, scale, offset) in [(SHARPEN, 1, 0), (blur, 2, 10)] {
        let item = measured_named_item(b"Cstm", "Custom...", custom_fltr(Value::Integer(scale), Value::Integer(offset), longs(&kernel)));
        let f = filter_from_item(&item);
        assert_eq!(f.command, "filter.other.custom");
        assert_eq!(f.params, json!({"kernel": kernel, "scale": scale, "offset": offset}));
        assert_eq!(item_for_filter(&f).unwrap(), item);
    }
}

#[test]
fn custom_written_like_the_engine_reads_it() {
    let fltr = |p: J| get_desc(&item_for_filter(&sf("filter.other.custom", p)).unwrap(), "Fltr").cloned().unwrap();
    // No kernel: the engine's identity; short kernels pad with 0; out-of-range values clamp.
    let mut id = [0; 25];
    id[12] = 1;
    assert_eq!(fltr(json!({})), custom_fltr(Value::Integer(1), Value::Integer(0), longs(&id)));
    let mut short = [0; 25];
    short[0] = 3;
    short[1] = -999;
    assert_eq!(fltr(json!({"kernel": [3, -5000], "scale": 0, "offset": 20000})), custom_fltr(Value::Integer(1), Value::Integer(9999), longs(&short)));
    // Fractions have no PSD equivalent.
    for p in [json!({"kernel": [0.5]}), json!({"scale": 1.5}), json!({"offset": -0.25})] {
        assert!(item_for_filter(&sf("filter.other.custom", p.clone())).is_err(), "{p}");
    }
}

#[test]
fn malformed_custom_is_kept_verbatim() {
    let i = Value::Integer;
    let mut wrong_type = longs(&SHARPEN);
    wrong_type[3] = Value::Double(1.0);
    for fltr in [
        custom_fltr(i(1), i(0), longs(&SHARPEN[..24])),
        custom_fltr(i(1), i(0), longs(&[0; 26])),
        custom_fltr(i(1), i(0), wrong_type),
        custom_fltr(i(1), i(0), longs(&[1000; 25])),
        custom_fltr(i(0), i(0), longs(&SHARPEN)),
        custom_fltr(i(10_000), i(0), longs(&SHARPEN)),
        custom_fltr(i(1), i(-10_000), longs(&SHARPEN)),
        custom_fltr(Value::Double(f64::NAN), i(0), longs(&SHARPEN)),
        custom_fltr(i(1), unit(b"#Pxl", 1.0), longs(&SHARPEN)),
        Descriptor::new("Cstm").with("Scl ", i(1)).with("Ofst", i(0)).with("Mtrx", i(5)),
        Descriptor::new("Cstm").with("Scl ", i(1)).with("Mtrx", Value::List(longs(&SHARPEN))),
        Descriptor::new("Cstm"),
        // A setting we don't write back.
        custom_fltr(i(1), i(0), longs(&SHARPEN)).with("Xtra", i(1)),
    ] {
        let item = measured_named_item(b"Cstm", "Custom...", fltr);
        let f = filter_from_item(&item);
        assert_eq!(f.command, UNSUPPORTED_FILTER, "{item:?}");
        assert_eq!(item_for_filter(&f).unwrap().to_bytes(), item.to_bytes(), "kept byte for byte");
    }
}

/// A `filterFX` item holding `fltr`, as Photoshop writes a filter with options.
fn pixelate_item(name: &str, fltr: Descriptor, filter_id: i32) -> Descriptor {
    Descriptor::new("filterFX")
        .with("Nm  ", Value::Text(UnicodeString::new_nul(name)))
        .with("blendOptions", Value::Descriptor(Descriptor::new("blendOptions").with("Opct", unit(b"#Prc", 100.0)).with("Md  ", enumv("BlnM", "Nrml"))))
        .with("enab", Value::Boolean(true))
        .with("hasoptions", Value::Boolean(true))
        .with("FrgC", rgbc(0.0))
        .with("BckC", rgbc(255.0))
        .with("Fltr", Value::Descriptor(fltr))
        .with("filterID", Value::Integer(filter_id))
}

#[test]
fn mezzotint_round_trips_every_type_and_reads_photoshops_shape() {
    for (code, ty) in MEZZOTINT_TYPES {
        let f = sf("filter.pixelate.mezzotint", json!({"type": ty}));
        let d = item_for_filter(&f).unwrap();
        assert_eq!(int(&d, "filterID"), Some(1_299_870_830));
        let fltr = get_desc(&d, "Fltr").unwrap();
        assert!(fltr.class_id.is("Mztn"));
        assert_eq!(enum_of(fltr, "MztT"), Some(&code[..]));
        assert!(matches!(fltr.get("MztT"), Some(Value::Enumerated { type_id, .. }) if type_id.is("MztT")));
        assert_eq!(filter_from_item(&Descriptor::from_bytes(&d.to_bytes()).unwrap()), f, "{ty}");
    }
    // Measured in Photoshop: `{'_class': 'Mztn', 'MztT': 'MztT.CrsD'}` and `LngS`, filterID 1299870830.
    for (code, ty) in [("CrsD", "coarseDots"), ("LngS", "longStrokes")] {
        let item = pixelate_item("Mezzotint...", Descriptor::new("Mztn").with("MztT", enumv("MztT", code)), 1_299_870_830);
        let f = filter_from_item(&item);
        assert_eq!((f.command.as_str(), &f.params), ("filter.pixelate.mezzotint", &json!({"type": ty})));
    }
    // No type or an unknown type is Fine Dots, like the engine; the seed
    // (PhotoCraft only) is not saved.
    for p in [json!({}), json!({"type": "sparkles"}), json!({"type": "fineDots", "seed": 77})] {
        let d = item_for_filter(&sf("filter.pixelate.mezzotint", p)).unwrap();
        let fltr = get_desc(&d, "Fltr").unwrap();
        assert_eq!(enum_of(fltr, "MztT"), Some(&b"FnDt"[..]));
        assert_eq!(fltr.items.len(), 1);
    }
}

#[test]
fn color_halftone_round_trips_and_reads_photoshops_shape() {
    for p in [
        json!({"maxRadius": 8, "channel1": 108, "channel2": 162, "channel3": 90, "channel4": 45}),
        json!({"maxRadius": 4, "channel1": -360, "channel2": 0, "channel3": 360, "channel4": -45}),
        json!({"maxRadius": 127, "channel1": 1, "channel2": 2, "channel3": 3, "channel4": 4}),
    ] {
        let f = sf("filter.pixelate.colorHalftone", p);
        let d = item_for_filter(&f).unwrap();
        assert_eq!(int(&d, "filterID"), Some(1_131_180_616));
        let fltr = get_desc(&d, "Fltr").unwrap();
        assert!(fltr.class_id.is("ClrH"));
        for k in ["Rds ", "Ang1", "Ang2", "Ang3", "Ang4"] {
            assert!(matches!(fltr.get(k), Some(Value::Integer(_))), "{k} is a long");
        }
        assert_eq!(filter_from_item(&Descriptor::from_bytes(&d.to_bytes()).unwrap()), f);
    }
    // Measured in Photoshop: `{'_class': 'ClrH', 'Rds ': 6, 'Ang1': 10, 'Ang2': 20, 'Ang3': 30, 'Ang4': 40}`.
    let measured = Descriptor::new("ClrH")
        .with("Rds ", Value::Integer(6))
        .with("Ang1", Value::Integer(10))
        .with("Ang2", Value::Integer(20))
        .with("Ang3", Value::Integer(30))
        .with("Ang4", Value::Integer(40));
    let item = pixelate_item("Color Halftone...", measured.clone(), 1_131_180_616);
    let f = filter_from_item(&item);
    assert_eq!(f.command, "filter.pixelate.colorHalftone");
    assert_eq!(f.params, json!({"maxRadius": 6, "channel1": 10, "channel2": 20, "channel3": 30, "channel4": 40}));
    assert_eq!(get_desc(&item_for_filter(&f).unwrap(), "Fltr"), Some(&measured));
    // Defaults and hostile values write longs within the dialog's ranges.
    let d =
        item_for_filter(&sf("filter.pixelate.colorHalftone", json!({"maxRadius": f64::NAN, "channel1": 1e12, "channel2": -1e12, "channel3": "x"}))).unwrap();
    let fltr = get_desc(&d, "Fltr").unwrap();
    let got: Vec<_> = ["Rds ", "Ang1", "Ang2", "Ang3", "Ang4"].iter().map(|k| int(fltr, k)).collect();
    assert_eq!(got, [Some(8), Some(360), Some(-360), Some(90), Some(45)]);
}

#[test]
fn malformed_pixelate_filters_stay_verbatim() {
    let mz = |v: Value| Descriptor::new("Mztn").with("MztT", v);
    let ch = |rds: Value, ang1: Value| {
        Descriptor::new("ClrH")
            .with("Rds ", rds)
            .with("Ang1", ang1)
            .with("Ang2", Value::Integer(20))
            .with("Ang3", Value::Integer(30))
            .with("Ang4", Value::Integer(40))
    };
    let cases = [
        pixelate_item("Mezzotint...", Descriptor::new("Mztn"), code(b"Mztn")),
        pixelate_item("Mezzotint...", mz(enumv("MztT", "Sprk")), code(b"Mztn")),
        pixelate_item("Mezzotint...", mz(Value::Integer(1)), code(b"Mztn")),
        pixelate_item("Color Halftone...", Descriptor::new("ClrH"), code(b"ClrH")),
        pixelate_item("Color Halftone...", ch(Value::Double(6.0), Value::Integer(10)), code(b"ClrH")),
        pixelate_item("Color Halftone...", ch(unit(b"#Pxl", 6.0), Value::Integer(10)), code(b"ClrH")),
        pixelate_item("Color Halftone...", ch(Value::Integer(3), Value::Integer(10)), code(b"ClrH")),
        pixelate_item("Color Halftone...", ch(Value::Integer(128), Value::Integer(10)), code(b"ClrH")),
        pixelate_item("Color Halftone...", ch(Value::Integer(6), Value::Integer(361)), code(b"ClrH")),
        pixelate_item("Color Halftone...", ch(Value::Integer(6), Value::Integer(i32::MIN)), code(b"ClrH")),
        pixelate_item("Color Halftone...", ch(Value::Integer(6), Value::Double(f64::NAN)), code(b"ClrH")),
        // A mistyped enum and settings we don't write back.
        pixelate_item("Mezzotint...", mz(enumv("Xxxx", "FnDt")), code(b"Mztn")),
        pixelate_item("Mezzotint...", mz(enumv("MztT", "FnDt")).with("Xtra", Value::Integer(1)), code(b"Mztn")),
        pixelate_item("Color Halftone...", ch(Value::Integer(6), Value::Integer(10)).with("Xtra", Value::Integer(1)), code(b"ClrH")),
        pixelate_item("Color Halftone...", ch(Value::Integer(6), Value::Integer(10)).with("Ang1", Value::Integer(11)), code(b"ClrH")),
    ];
    for item in cases {
        let f = filter_from_item(&item);
        assert_eq!(f.command, UNSUPPORTED_FILTER, "{:?}", get_desc(&item, "Fltr"));
        // Written back with its own settings (blending is rewritten from the model).
        let back = item_for_filter(&f).unwrap();
        // Compared as bytes: a NaN setting is not equal to itself.
        let fltr = |d: &Descriptor| get_desc(d, "Fltr").map(Descriptor::to_bytes);
        assert_eq!((fltr(&back), int(&back, "filterID")), (fltr(&item), int(&item, "filterID")));
    }
    // Without a `Fltr` the filter has no settings to read: kept verbatim too.
    let bare = Descriptor::new("filterFX").with("filterID", Value::Integer(code(b"ClrH")));
    assert_eq!(filter_from_item(&bare).command, UNSUPPORTED_FILTER);
}

/// Distort smart filters from a Photoshop file open as their PhotoCraft filters, and the ones
/// PhotoCraft writes carry the same settings, keys and order as Photoshop's.
#[test]
fn photoshop_distort_filters_read_and_write_like_photoshop() {
    for (command, params, hex) in PHOTOSHOP_FLTR {
        let ps = Descriptor::from_bytes(&from_hex(hex).unwrap()).unwrap();
        let f = filter_from_item(&photoshop_fltr_item(ps.clone()));
        let want: J = serde_json::from_str(params).unwrap();
        assert_eq!((f.command.as_str(), &f.params), (command, &want), "{command}");
        let ours = match item_for_filter(&f).unwrap().get("Fltr") {
            Some(Value::Descriptor(d)) => d.clone(),
            other => panic!("{command}: no Fltr ({other:?})"),
        };
        assert_eq!((&ours.class_id, &ours.items), (&ps.class_id, &ps.items), "{command}: same keys, values and order");
    }
    // A scaled wave can't be modelled, so it stays Photoshop's, unchanged.
    let wave = Descriptor::from_bytes(&from_hex(PHOTOSHOP_WAVE).unwrap()).unwrap();
    let item = photoshop_fltr_item(wave);
    let f = filter_from_item(&item);
    assert_eq!(f.command, UNSUPPORTED_FILTER);
    assert_eq!(item_for_filter(&f).unwrap().get("Fltr"), item.get("Fltr"));
    // Unscaled, but with 332 generators (the renderer draws 64 at most), it stays Photoshop's too.
    let mut unscaled = match item.get("Fltr") {
        Some(Value::Descriptor(d)) => d.clone(),
        other => panic!("wave: no Fltr ({other:?})"),
    };
    set(&mut unscaled, "SclV", Value::Integer(100));
    let item = photoshop_fltr_item(unscaled);
    let f = filter_from_item(&item);
    assert_eq!(f.command, UNSUPPORTED_FILTER);
    assert_eq!(item_for_filter(&f).unwrap().get("Fltr"), item.get("Fltr"));

    // An extra or repeated setting PhotoCraft doesn't model keeps the filter Photoshop's, unchanged.
    let mut fltrs: Vec<Descriptor> = PHOTOSHOP_FLTR.iter().map(|(_, _, hex)| Descriptor::from_bytes(&from_hex(hex).unwrap()).unwrap()).collect();
    let mut wave = unscaled_wave();
    set(&mut wave, "NmbG", Value::Integer(5));
    assert_eq!(filter_from_item(&photoshop_fltr_item(wave.clone())).command, "filter.distort.wave", "the plain wave maps");
    fltrs.push(wave);
    for ps in fltrs {
        let name = String::from_utf8_lossy(ps.class_id.as_bytes()).into_owned();
        let mut repeated = ps.clone();
        repeated.items.push(ps.items[0].clone());
        for odd in [ps.clone().with("Xtra", Value::Integer(7)), repeated] {
            let item = photoshop_fltr_item(odd);
            let f = filter_from_item(&item);
            assert_eq!(f.command, UNSUPPORTED_FILTER, "{name}");
            assert_eq!(item_for_filter(&f).unwrap().get("Fltr"), item.get("Fltr"), "{name}: written back unchanged");
        }
    }
}

/// Photoshop's Wave ([`PHOTOSHOP_WAVE`]) at 100% scale both ways.
fn unscaled_wave() -> Descriptor {
    let mut wave = Descriptor::from_bytes(&from_hex(PHOTOSHOP_WAVE).unwrap()).unwrap();
    set(&mut wave, "SclV", Value::Integer(100));
    wave
}

/// An unscaled Wave opens as PhotoCraft's Wave (Triangle, the other settings as stored) up to
/// the generators the renderer models; one more and it stays Photoshop's, unchanged.
#[test]
fn wave_maps_up_to_the_generators_the_renderer_models() {
    let max = i32::try_from(photocraft_algo::MAX_WAVE_GENERATORS).unwrap();
    assert_eq!(max, 64);
    let mut wave = Descriptor::from_bytes(&from_hex(PHOTOSHOP_WAVE).unwrap()).unwrap();
    set(&mut wave, "SclV", Value::Integer(100));
    for (generators, mapped) in [(1, true), (max, true), (max + 1, false), (999, false), (0, false), (-1, false)] {
        set(&mut wave, "NmbG", Value::Integer(generators));
        let item = photoshop_fltr_item(wave.clone());
        let f = filter_from_item(&item);
        if mapped {
            assert_eq!(
                (f.command.as_str(), &f.params),
                (
                    "filter.distort.wave",
                    &json!({"type": "triangle", "generators": generators, "wavelengthMin": 213, "wavelengthMax": 214, "amplitudeMin": 240,
                            "amplitudeMax": 241, "undefinedAreas": "wrap", "seed": 1270346})
                ),
                "{generators} generators"
            );
            match item_for_filter(&f).unwrap().get("Fltr") {
                Some(Value::Descriptor(ours)) => assert_eq!((&ours.class_id, &ours.items), (&wave.class_id, &wave.items), "{generators} generators"),
                other => panic!("wave: no Fltr ({other:?})"),
            }
        } else {
            assert_eq!(f.command, UNSUPPORTED_FILTER, "{generators} generators");
            assert_eq!(item_for_filter(&f).unwrap().get("Fltr"), item.get("Fltr"), "{generators} generators");
        }
    }
}

/// PhotoCraft draws at most `MAX_WAVE_GENERATORS` generators, so a Wave edited past that exports
/// the 64 it rendered, and a count below 1 exports 1, instead of Photoshop drawing more.
#[test]
fn wave_export_writes_only_the_generators_the_renderer_draws() {
    let max = i32::try_from(photocraft_algo::MAX_WAVE_GENERATORS).unwrap();
    for (stored, written) in [(100, max), (999, max), (max + 1, max), (max, max), (5, 5), (1, 1), (0, 1), (-3, 1)] {
        let wave = sf(
            "filter.distort.wave",
            json!({"type": "sine", "generators": stored, "wavelengthMin": 20, "wavelengthMax": 90, "amplitudeMin": 4, "amplitudeMax": 30}),
        );
        let fltr = match item_for_filter(&wave).unwrap().get("Fltr") {
            Some(Value::Descriptor(d)) => d.clone(),
            other => panic!("wave: no Fltr ({other:?})"),
        };
        assert_eq!(int(&fltr, "NmbG"), Some(written), "{stored} generators");
    }
}

/// `RndS` is the seed the renderer draws with, also for a legacy signed or fractional seed
/// (stored as is by `layer.smartFilter.setParams`), not 0.
#[test]
fn wave_export_writes_the_seed_the_renderer_draws() {
    for (stored, written) in [
        (json!(77), 77),
        (json!(4_000_000_000u32), -294_967_296),
        (json!(-1), -1),
        (json!(7.4), 7),
        (json!(-2.6), -3),
        (json!(1e12), i32::MAX),
        (json!("x"), 0),
    ] {
        let wave = sf("filter.distort.wave", json!({"generators": 3, "seed": stored}));
        let fltr = match item_for_filter(&wave).unwrap().get("Fltr") {
            Some(Value::Descriptor(d)) => d.clone(),
            other => panic!("wave: no Fltr ({other:?})"),
        };
        assert_eq!(int(&fltr, "RndS"), Some(written), "seed {stored}");
        // And it reads back as that bit pattern.
        assert_eq!(known_params("Wave", Some(&fltr), &Descriptor::new("filterFX")).unwrap().1["seed"], json!(written as u32), "seed {stored}");
    }
}

/// Invert on a smart object: Photoshop stores no `Fltr`, `hasoptions` false and its filter id.
#[test]
fn invert_smart_filter_reads_and_writes_like_photoshop() {
    let item = Descriptor::new("filterFX")
        .with("Nm  ", Value::Text(UnicodeString::new_nul("Invert")))
        .with("blendOptions", Value::Descriptor(Descriptor::new("blendOptions").with("Opct", unit(b"#Prc", 100.0)).with("Md  ", enumv("BlnM", "normal"))))
        .with("enab", Value::Boolean(true))
        .with("hasoptions", Value::Boolean(false))
        .with("FrgC", rgbc(255.0))
        .with("BckC", rgbc(0.0))
        .with("filterID", Value::Integer(code(b"Invr")));
    let f = filter_from_item(&item);
    assert_eq!((f.command.as_str(), &f.params), ("image.adjustments.invert", &json!({})));
    let back = item_for_filter(&f).unwrap();
    assert_eq!(back.get("Fltr"), None);
    assert_eq!(back.get("hasoptions"), Some(&Value::Boolean(false)));
    assert_eq!(back.get("filterID"), Some(&Value::Integer(code(b"Invr"))));
    assert_eq!(back.get("Nm  "), Some(&Value::Text(UnicodeString::new_nul("Invert"))));
}

/// A filter PhotoCraft doesn't model, or a known one with settings it doesn't, is kept verbatim; only its visibility and blend change.
#[test]
fn photoshop_sharpen_without_fltr_imports_as_sharpen() {
    // #6267: Photoshop's Sharpen item holds only its name, blending and `filterID`.
    let item = Descriptor::new("filterFX")
        .with("Nm  ", Value::Text(UnicodeString::new_nul("Sharpen")))
        .with("blendOptions", Value::Descriptor(Descriptor::new("blendOptions").with("Opct", unit(b"#Prc", 100.0)).with("Md  ", enumv("BlnM", "normal"))))
        .with("enab", Value::Boolean(true))
        .with("hasoptions", Value::Boolean(false))
        .with("filterID", Value::Integer(1_399_353_968));
    let f = filter_from_item(&item);
    assert_eq!((f.command.as_str(), &f.params), ("filter.sharpen.sharpen", &json!({})));
    // Hue/Saturation with an unnumbered range and Dust & Scratches with a missing setting fail
    // cleanly instead of panicking.
    let bad = SmartFilter {
        command: "image.adjustments.hueSaturation".into(),
        params: json!({"reds": {"range": [1, "x", 3, 4]}}),
        blend: BlendMode::Normal,
        opacity: 1.0,
        visible: true,
    };
    assert!(item_for_filter(&bad).is_err());
    let dust = Descriptor::new("filterFX")
        .with("Fltr", Value::Descriptor(Descriptor::new("DstS").with("Rds ", Value::Integer(3))))
        .with("filterID", Value::Integer(code(b"DstS")));
    assert_eq!(filter_from_item(&dust).command, UNSUPPORTED_FILTER);
}

#[test]
fn hue_saturation_ranges_are_not_lost() {
    let hue_sat = |params: J| item_for_filter(&sf("image.adjustments.hueSaturation", params));
    // A range that is not an object is an error, not a silently dropped setting.
    assert!(hue_sat(json!({"reds": 5})).is_err());
    assert!(hue_sat(json!({"reds": null})).is_ok());
    // Colorize's 25% default is the master saturation's only: a range's stays 0.
    let d = hue_sat(json!({"colorize": true, "reds": {"hue": 5}})).unwrap();
    let adjs = adjustment_list(get_desc(&d, "Fltr").unwrap()).unwrap();
    let saturations: Vec<_> = adjs.iter().map(|a| int(a, "Strt")).collect();
    assert_eq!(saturations, [Some(25), Some(0)]);
    // A repeated range (or master) would overwrite the first: the filter is kept verbatim.
    let entry = |range: Option<i32>| {
        let e = Descriptor::new("Hst2").with("H   ", Value::Integer(0)).with("Strt", Value::Integer(0)).with("Lght", Value::Integer(0));
        let Some(r) = range else { return Value::Descriptor(e) };
        let e = ["BgnR", "BgnS", "EndS", "EndR"].iter().fold(e.with("LclR", Value::Integer(r)), |e, k| e.with(k, Value::Integer(10)));
        Value::Descriptor(e)
    };
    for adjs in [vec![entry(None), entry(Some(1)), entry(Some(1))], vec![entry(None), entry(None)]] {
        let fltr = Descriptor::new("HStr").with("Clrz", Value::Boolean(false)).with("Adjs", Value::List(adjs));
        let item = Descriptor::new("filterFX")
            .with("Nm  ", Value::Text(UnicodeString::new_nul("Hue/Saturation...")))
            .with("blendOptions", Value::Descriptor(Descriptor::new("blendOptions").with("Opct", unit(b"#Prc", 100.0)).with("Md  ", enumv("BlnM", "normal"))))
            .with("enab", Value::Boolean(true))
            .with("Fltr", Value::Descriptor(fltr))
            .with("filterID", Value::Integer(code(b"HStr")));
        let f = filter_from_item(&item);
        assert_eq!(f.command, UNSUPPORTED_FILTER);
        assert_eq!(item_for_filter(&f).unwrap(), item);
    }
}

#[test]
fn unknown_filters_keep_their_descriptor() {
    let item = Descriptor::new("filterFX")
        .with("Nm  ", Value::Text(UnicodeString::new_nul("Shear...")))
        .with("blendOptions", Value::Descriptor(Descriptor::new("blendOptions").with("Opct", unit(b"#Prc", 100.0)).with("Md  ", enumv("BlnM", "normal"))))
        .with("enab", Value::Boolean(true))
        .with("Fltr", Value::Descriptor(Descriptor::new("Shr ").with("UndA", enumv("UndA", "WrpA"))))
        .with("filterID", Value::Integer(code(b"Shr ")));
    let f = filter_from_item(&item);
    assert_eq!(f.command, UNSUPPORTED_FILTER);
    assert_eq!(f.params["name"], "Shear...");
    assert_eq!(item_for_filter(&f).unwrap(), item);
    // Hidden and re-blended: only those keys change.
    let mut g = f.clone();
    g.visible = false;
    g.blend = BlendMode::Screen;
    let back = filter_from_item(&item_for_filter(&g).unwrap());
    assert_eq!((back.visible, back.blend, &back.params), (false, BlendMode::Screen, &f.params));
    // A known filter with a setting we don't model is kept verbatim too.
    let odd = Descriptor::new("filterFX").with("Fltr", Value::Descriptor(Descriptor::new("GsnB"))).with("filterID", Value::Integer(code(b"GsnB")));
    assert_eq!(filter_from_item(&odd).command, UNSUPPORTED_FILTER);
}

/// A `filterFXList` item with filter `fltr` and `filterID`, as Photoshop saves it.
fn ps_item(name: &str, fltr: Descriptor, filter_id: i32) -> Descriptor {
    Descriptor::new("filterFX")
        .with("Nm  ", Value::Text(UnicodeString::new_nul(name)))
        .with("blendOptions", Value::Descriptor(Descriptor::new("blendOptions").with("Opct", unit(b"#Prc", 100.0)).with("Md  ", enumv("BlnM", "normal"))))
        .with("enab", Value::Boolean(true))
        .with("hasoptions", Value::Boolean(true))
        .with("FrgC", rgbc(0.0))
        .with("BckC", rgbc(255.0))
        .with("Fltr", Value::Descriptor(fltr))
        .with("filterID", Value::Integer(filter_id))
}

fn surface_blur(rds: f64, thsh: i32) -> Descriptor {
    Descriptor::new("surfaceBlur").with("Rds ", unit(b"#Pxl", rds)).with("Thsh", Value::Integer(thsh))
}

fn smart_blur(rds: f64, thsh: f64, q: &str, m: &str) -> Descriptor {
    Descriptor::new("SmrB").with("Rds ", Value::Double(rds)).with("Thsh", Value::Double(thsh)).with("SmBQ", enumv("SmBQ", q)).with("SmBM", enumv("SmBM", m))
}

fn radial_blur(amnt: i32, m: &str, q: &str) -> Descriptor {
    Descriptor::new("RdlB").with("Amnt", Value::Integer(amnt)).with("BlrM", enumv("BlrM", m)).with("BlrQ", enumv("BlrQ", q))
}

/// The `Fltr` shapes measured in Photoshop (each filter applied to a smart object through
/// Action Manager, saved, and `filterFX` read back) open as our filters and are written back
/// with the same keys, value types, units, enums and filter ids.
#[test]
fn blur_family_matches_photoshop_shapes() {
    let smart = code(b"SmrB");
    let radial = code(b"RdlB");
    assert_eq!((smart, radial), (1_399_681_602, 1_382_313_026));
    let cases = [
        (surface_blur(5.0, 15), 1135, "filter.blur.surfaceBlur", json!({"radius": 5.0, "threshold": 15})),
        (surface_blur(12.0, 40), 1135, "filter.blur.surfaceBlur", json!({"radius": 12.0, "threshold": 40})),
        (smart_blur(5.0, 25.0, "SBQH", "SBMN"), smart, "filter.blur.smartBlur", json!({"radius": 5.0, "threshold": 25.0, "quality": "high", "mode": "normal"})),
        (
            smart_blur(3.0, 30.0, "SBQL", "SBME"),
            smart,
            "filter.blur.smartBlur",
            json!({"radius": 3.0, "threshold": 30.0, "quality": "low", "mode": "edgeOnly"}),
        ),
        (
            smart_blur(3.0, 30.0, "SBQM", "SBMO"),
            smart,
            "filter.blur.smartBlur",
            json!({"radius": 3.0, "threshold": 30.0, "quality": "medium", "mode": "overlayEdge"}),
        ),
        (radial_blur(10, "Spn ", "Drft"), radial, "filter.blur.radialBlur", json!({"amount": 10, "method": "spin", "quality": "draft"})),
        (radial_blur(20, "Spn ", "Bst "), radial, "filter.blur.radialBlur", json!({"amount": 20, "method": "spin", "quality": "best"})),
        (radial_blur(20, "Zm  ", "Gd  "), radial, "filter.blur.radialBlur", json!({"amount": 20, "method": "zoom", "quality": "good"})),
    ];
    for (fltr, id, command, params) in cases {
        let item = ps_item("x", fltr.clone(), id);
        let f = filter_from_item(&Descriptor::from_bytes(&item.to_bytes()).unwrap());
        assert_eq!((f.command.as_str(), &f.params), (command, &params), "{fltr:?}");
        let out = item_for_filter(&f).unwrap();
        assert_eq!(out.get("filterID"), Some(&Value::Integer(id)));
        assert_eq!(out.get("hasoptions"), Some(&Value::Boolean(true)));
        // Byte-identical filter descriptor: class id, four-char keys, types, units and enums.
        let Some(Value::Descriptor(w)) = out.get("Fltr") else { panic!("no Fltr for {command}") };
        assert_eq!(w.to_bytes(), fltr.to_bytes(), "{command}");
    }
    // Surface Blur's class is a string id; Smart Blur's and Radial Blur's are four-char codes.
    assert_eq!(surface_blur(1.0, 2).class_id, Id::Str(b"surfaceBlur".to_vec()));
    assert_eq!(radial_blur(1, "Spn ", "Gd  ").class_id, Id::Code(*b"RdlB"));
    // Unset options are written with the dialog defaults.
    let fltr = |c: &str, p: J| item_for_filter(&sf(c, p)).unwrap().get("Fltr").cloned();
    assert_eq!(fltr("filter.blur.surfaceBlur", json!({})), Some(Value::Descriptor(surface_blur(5.0, 15))));
    assert_eq!(fltr("filter.blur.smartBlur", json!({})), Some(Value::Descriptor(smart_blur(3.0, 25.0, "SBQH", "SBMN"))));
    assert_eq!(fltr("filter.blur.radialBlur", json!({"centerX": 0.5, "centerY": 0.5})), Some(Value::Descriptor(radial_blur(10, "Spn ", "Gd  "))));
}

/// `d` with `key` set to `v` (replacing the measured value).
fn replaced(mut d: Descriptor, key: &str, v: Value) -> Descriptor {
    set(&mut d, key, v);
    d
}

/// Malformed or unmodelled blur descriptors stay verbatim (and are written back unchanged);
/// settings PSD can't hold are reported, never a panic.
#[test]
fn blur_family_malformed_is_kept_verbatim() {
    let smart = code(b"SmrB");
    let radial = code(b"RdlB");
    let text = |s: &str| Value::Text(UnicodeString::new_nul(s));
    let bad = [
        // Missing keys.
        (Descriptor::new("surfaceBlur").with("Rds ", unit(b"#Pxl", 5.0)), 1135),
        (Descriptor::new("SmrB").with("Rds ", Value::Double(3.0)).with("Thsh", Value::Double(25.0)), smart),
        (Descriptor::new("RdlB").with("Amnt", Value::Integer(10)).with("BlrM", enumv("BlrM", "Spn ")), radial),
        // Wrong types.
        (Descriptor::new("surfaceBlur").with("Rds ", unit(b"#Pxl", 5.0)).with("Thsh", Value::Double(15.0)), 1135),
        (
            Descriptor::new("SmrB")
                .with("Rds ", text("3"))
                .with("Thsh", Value::Double(25.0))
                .with("SmBQ", enumv("SmBQ", "SBQH"))
                .with("SmBM", enumv("SmBM", "SBMN")),
            smart,
        ),
        (Descriptor::new("RdlB").with("Amnt", text("10")).with("BlrM", enumv("BlrM", "Spn ")).with("BlrQ", enumv("BlrQ", "Gd  ")), radial),
        (Descriptor::new("RdlB").with("Amnt", Value::Integer(10)).with("BlrM", Value::Integer(1)).with("BlrQ", enumv("BlrQ", "Gd  ")), radial),
        // Out of range, NaN and infinity.
        (surface_blur(0.5, 15), 1135),
        (surface_blur(5.0, 300), 1135),
        (surface_blur(f64::NAN, 15), 1135),
        (smart_blur(f64::INFINITY, 25.0, "SBQH", "SBMN"), smart),
        (smart_blur(3.0, 0.0, "SBQH", "SBMN"), smart),
        (radial_blur(0, "Spn ", "Gd  "), radial),
        (radial_blur(101, "Zm  ", "Gd  "), radial),
        // Unknown enum values.
        (smart_blur(3.0, 25.0, "SBQX", "SBMN"), smart),
        (smart_blur(3.0, 25.0, "Hgh ", "SBMN"), smart),
        (radial_blur(10, "Twrl", "Gd  "), radial),
        (radial_blur(10, "Spn ", "Hgh "), radial),
        // A key we don't model (an off-centre Radial Blur would carry one).
        (radial_blur(10, "Zm  ", "Gd  ").with("Cntr", Value::Descriptor(Descriptor::new("Pnt ").with("Hrzn", Value::Double(10.0)))), radial),
        (surface_blur(5.0, 15).with("Edg ", Value::Integer(1)), 1135),
        // Enums of another type, units and value types we would not write back.
        (replaced(smart_blur(3.0, 25.0, "SBQH", "SBMN"), "SmBQ", enumv("Othr", "SBQH")), smart),
        (replaced(smart_blur(3.0, 25.0, "SBQH", "SBMN"), "SmBM", enumv("SmBQ", "SBMN")), smart),
        (replaced(radial_blur(10, "Spn ", "Gd  "), "BlrM", enumv("Othr", "Spn ")), radial),
        (replaced(radial_blur(10, "Spn ", "Gd  "), "BlrQ", enumv("BlrM", "Gd  ")), radial),
        (Descriptor::new("surfaceBlur").with("Rds ", unit(b"#Prc", 5.0)).with("Thsh", Value::Integer(15)), 1135),
        (Descriptor::new("surfaceBlur").with("Rds ", Value::Double(5.0)).with("Thsh", Value::Integer(15)), 1135),
        (replaced(smart_blur(3.0, 25.0, "SBQH", "SBMN"), "Rds ", unit(b"#Pxl", 3.0)), smart),
        (replaced(smart_blur(3.0, 25.0, "SBQH", "SBMN"), "Thsh", unit(b"#Prc", 25.0)), smart),
        (replaced(smart_blur(3.0, 25.0, "SBQH", "SBMN"), "Thsh", Value::Integer(25)), smart),
    ];
    for (fltr, id) in bad {
        let item = ps_item("x", fltr.clone(), id);
        let f = filter_from_item(&item);
        assert_eq!(f.command, UNSUPPORTED_FILTER, "{fltr:?}");
        // Compared as bytes: a NaN setting is not equal to itself.
        assert_eq!(item_for_filter(&f).unwrap().to_bytes(), item.to_bytes(), "{fltr:?}");
    }
    // Radial Blur at its defaults has no `Fltr` (measured): its values are not stored, so the
    // entry stays verbatim instead of guessing them.
    let no_opts = Descriptor::new("filterFX").with("hasoptions", Value::Boolean(false)).with("filterID", Value::Integer(radial));
    assert_eq!(filter_from_item(&no_opts).command, UNSUPPORTED_FILTER);
    // Settings with no PSD equivalent are errors when writing.
    for (command, p) in [
        ("filter.blur.smartBlur", json!({"quality": "ultra"})),
        ("filter.blur.smartBlur", json!({"mode": 3})),
        ("filter.blur.radialBlur", json!({"method": "twirl"})),
        ("filter.blur.radialBlur", json!({"quality": ["best"]})),
        ("filter.blur.radialBlur", json!({"centerX": 0.25})),
        ("filter.blur.radialBlur", json!({"centerY": 1.0})),
        // A centre, or a value, that is there but can't be written as Photoshop would open it.
        ("filter.blur.radialBlur", json!({"centerX": "middle"})),
        ("filter.blur.radialBlur", json!({"centerY": true})),
        ("filter.blur.radialBlur", json!({"amount": 0})),
        ("filter.blur.radialBlur", json!({"amount": 101})),
        ("filter.blur.radialBlur", json!({"amount": "10"})),
        ("filter.blur.surfaceBlur", json!({"radius": 0.5})),
        ("filter.blur.surfaceBlur", json!({"radius": 100.5})),
        ("filter.blur.surfaceBlur", json!({"threshold": 1})),
        ("filter.blur.surfaceBlur", json!({"threshold": 256})),
        ("filter.blur.surfaceBlur", json!({"radius": "5"})),
        ("filter.blur.smartBlur", json!({"radius": 0.05})),
        ("filter.blur.smartBlur", json!({"threshold": 100.5})),
        ("filter.blur.smartBlur", json!({"threshold": [25]})),
    ] {
        assert!(item_for_filter(&sf(command, p.clone())).is_err(), "{command} {p}");
    }
    // Every value that is written opens again as the same editable filter (range ends included).
    for (command, p) in [
        ("filter.blur.surfaceBlur", json!({"radius": 1.0, "threshold": 2})),
        ("filter.blur.surfaceBlur", json!({"radius": 100.0, "threshold": 255})),
        ("filter.blur.smartBlur", json!({"radius": 0.1, "threshold": 100.0, "quality": "low", "mode": "edgeOnly"})),
        ("filter.blur.radialBlur", json!({"amount": 1, "method": "zoom", "quality": "best", "centerX": 0.5, "centerY": null})),
        ("filter.blur.radialBlur", json!({"amount": 100, "method": "spin", "quality": "draft"})),
    ] {
        let back = filter_from_item(&item_for_filter(&sf(command, p.clone())).unwrap());
        assert_eq!(back.command, command, "{p}");
    }
    // Hostile numbers write without panicking.
    for p in [json!({"radius": f64::MAX, "threshold": -1e300, "amount": 1e300}), json!({"radius": "x", "threshold": null}), json!(null), json!([1, 2])] {
        for command in ["filter.blur.surfaceBlur", "filter.blur.smartBlur", "filter.blur.radialBlur"] {
            let _ = item_for_filter(&sf(command, p.clone()));
        }
    }
}

/// A `filterFX` entry shaped as Photoshop saves it (measured in Photoshop: the filter applied
/// through Action Manager to a smart object, the PSD saved, `filterFX` read back).
fn photoshop_item(filter_id: &[u8; 4], fltr: Descriptor) -> Descriptor {
    Descriptor::new("filterFX")
        .with("Nm  ", Value::Text(UnicodeString::new_nul("Filter...")))
        .with("blendOptions", blend_options(&sf("", J::Null)))
        .with("enab", Value::Boolean(true))
        .with("hasoptions", Value::Boolean(true))
        .with("FrgC", rgbc(0.0))
        .with("BckC", rgbc(255.0))
        .with("Fltr", Value::Descriptor(fltr))
        .with("filterID", Value::Integer(code(filter_id)))
}

#[test]
fn stylize_and_pixelate_filters_open_from_photoshop_shapes() {
    let cases = [
        (b"Dfs ", Descriptor::new("Dfs ").with("Md  ", enumv("DfsM", "Nrml")), "filter.stylize.diffuse", json!({"mode": "normal"})),
        (b"Dfs ", Descriptor::new("Dfs ").with("Md  ", enumv("DfsM", "DrkO")), "filter.stylize.diffuse", json!({"mode": "darkenOnly"})),
        (b"Dfs ", Descriptor::new("Dfs ").with("Md  ", enumv("DfsM", "LghO")), "filter.stylize.diffuse", json!({"mode": "lightenOnly"})),
        (b"Dfs ", Descriptor::new("Dfs ").with("Md  ", enumv("DfsM", "anisotropic")), "filter.stylize.diffuse", json!({"mode": "anisotropic"})),
        (
            b"TrcC",
            Descriptor::new("TrcC").with("Lvl ", Value::Integer(128)).with("Edg ", enumv("CntE", "Upr ")),
            "filter.stylize.traceContour",
            json!({"level": 128, "edge": "upper"}),
        ),
        (
            b"TrcC",
            Descriptor::new("TrcC").with("Lvl ", Value::Integer(60)).with("Edg ", enumv("CntE", "Lwr ")),
            "filter.stylize.traceContour",
            json!({"level": 60, "edge": "lower"}),
        ),
        (b"Crst", Descriptor::new("Crst").with("ClSz", Value::Integer(10)), "filter.pixelate.crystallize", json!({"cellSize": 10})),
        (
            b"Pntl",
            Descriptor::new("Pntl").with("ClSz", Value::Integer(7)),
            "filter.pixelate.pointillize",
            json!({"cellSize": 7, "background": [1.0, 1.0, 1.0, 1.0]}),
        ),
    ];
    for (id, fltr, command, params) in cases {
        let item = photoshop_item(id, fltr);
        // Through bytes, as read from a file.
        let f = filter_from_item(&Descriptor::from_bytes(&item.to_bytes()).unwrap());
        assert_eq!((f.command.as_str(), &f.params), (command, &params));
        // Written back in the measured shape: same class, keys, value types and filterID.
        let back = item_for_filter(&f).unwrap();
        for key in ["Fltr", "filterID", "BckC"] {
            assert_eq!(back.get(key), item.get(key), "{command} {key}");
        }
    }
    // The `anisotropic` mode is a long id, as Photoshop writes it.
    let d = item_for_filter(&sf("filter.stylize.diffuse", json!({"mode": "anisotropic"}))).unwrap();
    let Some(Value::Descriptor(fltr)) = d.get("Fltr") else { panic!("no Fltr") };
    assert!(matches!(fltr.get("Md  "), Some(Value::Enumerated { value: Id::Str(v), .. }) if v == b"anisotropic"));
    // Pointillize takes the entry's background colour; the others ignore it.
    let mut item = photoshop_item(b"Pntl", Descriptor::new("Pntl").with("ClSz", Value::Integer(7)));
    set(&mut item, "BckC", rgbc(51.0));
    let f = filter_from_item(&item);
    assert_eq!(f.params["background"], json!([0.2, 0.2, 0.2, 1.0]));
    assert_eq!(item_for_filter(&f).unwrap().get("BckC"), Some(&rgbc(51.0)));
    let mut item = photoshop_item(b"Crst", Descriptor::new("Crst").with("ClSz", Value::Integer(7)));
    set(&mut item, "BckC", rgbc(51.0));
    assert_eq!(filter_from_item(&item).params, json!({"cellSize": 7}));
    // Our random seed has no PSD key: it is not written.
    let fltr = |c: &str, p: J| item_for_filter(&sf(c, p)).unwrap().get("Fltr").cloned();
    assert_eq!(
        fltr("filter.stylize.diffuse", json!({"mode": "normal", "seed": 7})),
        Some(Value::Descriptor(Descriptor::new("Dfs ").with("Md  ", enumv("DfsM", "Nrml"))))
    );
    // Defaults and out-of-range values are written as the engine renders them: clamped to
    // Photoshop's ranges, and a non-number as the default.
    assert_eq!(
        fltr("filter.stylize.traceContour", json!({})),
        Some(Value::Descriptor(Descriptor::new("TrcC").with("Lvl ", Value::Integer(128)).with("Edg ", enumv("CntE", "Lwr "))))
    );
    assert_eq!(fltr("filter.pixelate.crystallize", json!({})), Some(Value::Descriptor(Descriptor::new("Crst").with("ClSz", Value::Integer(10)))));
    assert_eq!(fltr("filter.pixelate.pointillize", json!({})), Some(Value::Descriptor(Descriptor::new("Pntl").with("ClSz", Value::Integer(5)))));
    assert_eq!(
        fltr("filter.pixelate.crystallize", json!({"cellSize": 300})),
        Some(Value::Descriptor(Descriptor::new("Crst").with("ClSz", Value::Integer(300))))
    );
    assert_eq!(fltr("filter.pixelate.pointillize", json!({"cellSize": 2.6})), Some(Value::Descriptor(Descriptor::new("Pntl").with("ClSz", Value::Integer(3)))));
    assert_eq!(
        fltr("filter.stylize.traceContour", json!({"level": 0, "edge": "upper"})),
        Some(Value::Descriptor(Descriptor::new("TrcC").with("Lvl ", Value::Integer(0)).with("Edg ", enumv("CntE", "Upr "))))
    );
    assert_eq!(
        fltr("filter.stylize.traceContour", json!({"level": 255.4})),
        Some(Value::Descriptor(Descriptor::new("TrcC").with("Lvl ", Value::Integer(255)).with("Edg ", enumv("CntE", "Lwr "))))
    );
    let crst = |v: i32| Some(Value::Descriptor(Descriptor::new("Crst").with("ClSz", Value::Integer(v))));
    let pntl = |v: i32| Some(Value::Descriptor(Descriptor::new("Pntl").with("ClSz", Value::Integer(v))));
    let trcc = |v: i32, e: &str| Some(Value::Descriptor(Descriptor::new("TrcC").with("Lvl ", Value::Integer(v)).with("Edg ", enumv("CntE", e))));
    assert_eq!(fltr("filter.pixelate.crystallize", json!({"cellSize": 1e9})), crst(300));
    assert_eq!(fltr("filter.pixelate.crystallize", json!({"cellSize": 2.4})), crst(3));
    assert_eq!(fltr("filter.pixelate.pointillize", json!({"cellSize": "big"})), pntl(5));
    assert_eq!(fltr("filter.pixelate.pointillize", json!({"cellSize": null})), pntl(5));
    assert_eq!(fltr("filter.stylize.traceContour", json!({"level": -40, "edge": "upper"})), trcc(0, "Upr "));
    assert_eq!(fltr("filter.stylize.traceContour", json!({"level": 255.5})), trcc(255, "Lwr "));
    assert_eq!(fltr("filter.stylize.traceContour", json!({"level": [128]})), trcc(128, "Lwr "));
}

#[test]
fn malformed_stylize_and_pixelate_filters_are_kept_verbatim() {
    let long = Value::Integer;
    let cases = [
        // Missing keys.
        (b"Dfs ", Descriptor::new("Dfs ")),
        (b"TrcC", Descriptor::new("TrcC").with("Lvl ", long(60))),
        (b"TrcC", Descriptor::new("TrcC").with("Edg ", enumv("CntE", "Upr "))),
        (b"Crst", Descriptor::new("Crst")),
        (b"Pntl", Descriptor::new("Pntl")),
        // Unknown enum values or enum types.
        (b"Dfs ", Descriptor::new("Dfs ").with("Md  ", enumv("DfsM", "Wild"))),
        (b"Dfs ", Descriptor::new("Dfs ").with("Md  ", enumv("BlnM", "Nrml"))),
        (b"TrcC", Descriptor::new("TrcC").with("Lvl ", long(60)).with("Edg ", enumv("CntE", "Mddl"))),
        // Wrong types (also NaN).
        (b"Dfs ", Descriptor::new("Dfs ").with("Md  ", long(1))),
        (b"TrcC", Descriptor::new("TrcC").with("Lvl ", Value::Double(60.0)).with("Edg ", enumv("CntE", "Upr "))),
        (b"Crst", Descriptor::new("Crst").with("ClSz", unit(b"#Pxl", 10.0))),
        (b"Pntl", Descriptor::new("Pntl").with("ClSz", Value::Double(f64::NAN))),
        // Out of range.
        (b"TrcC", Descriptor::new("TrcC").with("Lvl ", long(256)).with("Edg ", enumv("CntE", "Upr "))),
        (b"TrcC", Descriptor::new("TrcC").with("Lvl ", long(-1)).with("Edg ", enumv("CntE", "Upr "))),
        (b"Crst", Descriptor::new("Crst").with("ClSz", long(2))),
        (b"Pntl", Descriptor::new("Pntl").with("ClSz", long(i32::MAX))),
    ];
    for (id, fltr) in cases {
        let item = photoshop_item(id, fltr);
        let f = filter_from_item(&Descriptor::from_bytes(&item.to_bytes()).unwrap());
        assert_eq!(f.command, UNSUPPORTED_FILTER, "{item:?}");
        assert_eq!(item_for_filter(&f).unwrap().to_bytes(), item.to_bytes());
    }
    // Saved without options (`hasoptions` false, no `Fltr`): kept verbatim.
    let bare = Descriptor::new("filterFX").with("hasoptions", Value::Boolean(false)).with("filterID", Value::Integer(code(b"Crst")));
    assert_eq!(filter_from_item(&bare).command, UNSUPPORTED_FILTER);
    // A Pointillize entry whose background colour is unreadable is kept verbatim too.
    for bad in [Value::Integer(3), Value::Descriptor(Descriptor::new("RGBC").with("Rd  ", Value::Double(0.0)))] {
        let mut item = photoshop_item(b"Pntl", Descriptor::new("Pntl").with("ClSz", Value::Integer(7)));
        set(&mut item, "BckC", bad);
        assert_eq!(filter_from_item(&item).command, UNSUPPORTED_FILTER);
    }
    // Values with no PSD equivalent are reported, not guessed.
    for (c, p) in [
        ("filter.stylize.diffuse", json!({"mode": "sideways"})),
        ("filter.stylize.diffuse", json!({"mode": 3})),
        ("filter.stylize.traceContour", json!({"edge": "middle"})),
        ("filter.pixelate.pointillize", json!({"background": "teal"})),
        ("filter.pixelate.pointillize", json!({"background": [1.0]})),
    ] {
        assert!(item_for_filter(&sf(c, p.clone())).is_err(), "{c} {p}");
    }
}

/// A Filter Gallery item as Photopea writes it (Chrome, Detail 12, Smoothness 4).
fn gallery_item_with(effects: Vec<Descriptor>) -> Descriptor {
    let rgb = |v: f64| Value::Descriptor(Descriptor::new("RGBC").with("Rd  ", Value::Double(v)).with("Grn ", Value::Double(v)).with("Bl  ", Value::Double(v)));
    Descriptor::new("filterFX")
        .with("Nm  ", Value::Text(UnicodeString::new_nul("Filtergalerie")))
        .with("blendOptions", Value::Descriptor(Descriptor::new("blendOptions").with("Opct", unit(b"#Prc", 100.0)).with("Md  ", enumv("BlnM", "normal"))))
        .with("enab", Value::Boolean(true))
        .with("hasoptions", Value::Boolean(true))
        .with("FrgC", rgb(0.0))
        .with("BckC", rgb(255.0))
        .with("Fltr", Value::Descriptor(Descriptor::new("GEfc").with("GEfs", Value::List(effects.into_iter().map(Value::Descriptor).collect()))))
        .with("filterID", Value::Integer(code(b"GEfc")))
}

fn chrome(detail: i32, smoothness: i32, visible: bool) -> Descriptor {
    Descriptor::new("GEfc")
        .with("Dtl ", Value::Integer(detail))
        .with("Smth", Value::Integer(smoothness))
        .with("GEfk", enumv("GEft", "Chrm"))
        .with("GELv", Value::Boolean(visible))
}

#[test]
fn filter_gallery_chrome_imports_as_editable_gallery() {
    let f = filter_from_item(&gallery_item_with(vec![chrome(12, 4, true), chrome(3, 9, false)]));
    assert_eq!(f.command, "filter.filterGallery");
    assert_eq!(
        f.params,
        json!({
            "effects": [
                {"filter": "chrome", "params": {"detail": 12, "smoothness": 4}, "visible": true},
                {"filter": "chrome", "params": {"detail": 3, "smoothness": 9}, "visible": false},
            ],
            "foreground": [0.0, 0.0, 0.0, 1.0],
            "background": [1.0, 1.0, 1.0, 1.0],
        })
    );
    // Written back and read again: unchanged, also when re-blended and hidden.
    let mut g = f.clone();
    g.blend = BlendMode::Multiply;
    g.opacity = 0.5;
    g.visible = false;
    let d = item_for_filter(&g).unwrap();
    assert_eq!(filter_from_item(&Descriptor::from_bytes(&d.to_bytes()).unwrap()), g);
    let fltr = get_desc(&d, "Fltr").unwrap();
    assert!(fltr.class_id.is("GEfc"));
    assert_eq!(int(&d, "filterID"), Some(code(b"GEfc")));
}

/// A single effect as Photoshop 27 writes it: inline in `Fltr`, no `GEfs` list or `GELv`.
fn photoshop_chrome(detail: i32, smoothness: i32) -> Descriptor {
    let rgb = |v: f64| Value::Descriptor(Descriptor::new("RGBC").with("Rd  ", Value::Double(v)).with("Grn ", Value::Double(v)).with("Bl  ", Value::Double(v)));
    let mut fltr = Descriptor::new("GEfc").with("GEfk", enumv("GEft", "Chrm")).with("Dtl ", Value::Integer(detail)).with("Smth", Value::Integer(smoothness));
    fltr.name = UnicodeString::new_nul("Filter Gallery");
    Descriptor::new("filterFX")
        .with("Nm  ", Value::Text(UnicodeString::new_nul("Filter Gallery...")))
        .with("blendOptions", Value::Descriptor(Descriptor::new("blendOptions").with("Opct", unit(b"#Prc", 100.0)).with("Md  ", enumv("BlnM", "Nrml"))))
        .with("enab", Value::Boolean(true))
        .with("hasoptions", Value::Boolean(true))
        .with("FrgC", rgb(255.0))
        .with("BckC", rgb(0.0))
        .with("Fltr", Value::Descriptor(fltr))
        .with("filterID", Value::Integer(code(b"GEfc")))
}

#[test]
fn filter_gallery_photoshop_inline_effect_imports_and_writes_back_inline() {
    let item = photoshop_chrome(10, 4);
    let f = filter_from_item(&item);
    assert_eq!(f.command, "filter.filterGallery");
    assert_eq!(
        f.params,
        json!({
            "effects": [{"filter": "chrome", "params": {"detail": 10, "smoothness": 4}, "visible": true}],
            "foreground": [1.0, 1.0, 1.0, 1.0],
            "background": [0.0, 0.0, 0.0, 1.0],
        })
    );
    // Written back in Photoshop's own layout.
    let d = item_for_filter(&f).unwrap();
    assert_eq!(get_desc(&d, "Fltr"), get_desc(&item, "Fltr"));
    assert_eq!((get_desc(&d, "FrgC"), get_desc(&d, "BckC")), (get_desc(&item, "FrgC"), get_desc(&item, "BckC")));
    assert_eq!(filter_from_item(&Descriptor::from_bytes(&d.to_bytes()).unwrap()), f);
    // A hidden single effect needs the list (only it has a visibility flag).
    let mut hidden = f.clone();
    hidden.params["effects"][0]["visible"] = json!(false);
    let d = item_for_filter(&hidden).unwrap();
    assert!(get_desc(&d, "Fltr").unwrap().get("GEfs").is_some());
    assert_eq!(filter_from_item(&d), hidden);
    // Inline effects we don't model stay verbatim.
    let mut odd = photoshop_chrome(10, 4);
    set(&mut odd, "Fltr", Value::Descriptor(Descriptor::new("GEfc").with("GEfk", enumv("GEft", "Ctot")).with("Dtl ", Value::Integer(1))));
    assert_eq!(filter_from_item(&odd).command, UNSUPPORTED_FILTER);
    let mut extra = photoshop_chrome(10, 4);
    set(&mut extra, "Fltr", Value::Descriptor(Descriptor::new("GEfc").with("GEfk", enumv("GEft", "Chrm")).with("Xtra", Value::Integer(1))));
    assert_eq!(filter_from_item(&extra).command, UNSUPPORTED_FILTER);
}

#[test]
fn gallery_table_matches_the_gallery_filters() {
    for g in &GALLERY_EFFECTS {
        let f = photocraft_algo::GalleryFilter::from_key(g.key).unwrap();
        let keys: Vec<&str> = f.params().iter().map(|p| p.key).collect();
        for (_, ours) in g.settings {
            assert!(keys.contains(ours), "{}.{ours}", g.key);
        }
    }
}

#[test]
fn filter_gallery_written_with_defaults_and_colours() {
    let f = sf("filter.filterGallery", json!({"effects": [{"filter": "chrome"}], "foreground": "#ff0000", "background": [0.0, 0.0, 1.0, 1.0]}));
    let back = filter_from_item(&item_for_filter(&f).unwrap());
    assert_eq!(back.params["effects"][0]["params"], json!({"detail": 4, "smoothness": 7}));
    assert_eq!(back.params["foreground"], json!([1.0, 0.0, 0.0, 1.0]));
    assert_eq!(back.params["background"], json!([0.0, 0.0, 1.0, 1.0]));
}

#[test]
fn filter_gallery_we_dont_model_is_kept_verbatim() {
    // Another effect, an extra setting, a wrong-typed setting, no effects, a non-list.
    let other = Descriptor::new("GEfc").with("GEfk", enumv("GEft", "Ctot")).with("GELv", Value::Boolean(true));
    let extra = chrome(4, 7, true).with("Xtra", Value::Integer(1));
    let typed = Descriptor::new("GEfc").with("Dtl ", Value::Double(4.0)).with("GEfk", enumv("GEft", "Chrm"));
    for effects in [vec![chrome(4, 7, true), other], vec![extra], vec![typed], vec![]] {
        let item = gallery_item_with(effects);
        let f = filter_from_item(&item);
        assert_eq!(f.command, UNSUPPORTED_FILTER);
        assert_eq!(item_for_filter(&f).unwrap(), item);
    }
    let odd = Descriptor::new("filterFX").with("Fltr", Value::Descriptor(Descriptor::new("GEfc").with("GEfs", Value::Integer(3))));
    assert_eq!(filter_from_item(&odd).command, UNSUPPORTED_FILTER);
    let bare = Descriptor::new("filterFX").with("filterID", Value::Integer(code(b"GEfc")));
    assert_eq!(filter_from_item(&bare).command, UNSUPPORTED_FILTER);
}

#[test]
fn filter_gallery_without_psd_equivalent_is_reported() {
    for p in [
        json!({"effects": [{"filter": "cutout"}]}),
        json!({"effects": [{"filter": "chrome", "params": {"sparkle": 1}}]}),
        json!({"effects": [{"params": {}}]}),
        json!({"effects": []}),
        json!({}),
        json!({"effects": [{"filter": "chrome"}], "list": true}),
        json!({"effects": [{"filter": "chrome"}], "foreground": "red"}),
        json!(null),
    ] {
        assert!(item_for_filter(&sf("filter.filterGallery", p.clone())).is_err(), "{p}");
    }
}

/// Filters with no PSD equivalent are skipped with a warning each, and the writable ones around them still export.
#[test]
fn unwritable_filters_are_reported_not_fatal() {
    let stack = FilterStack {
        filters: vec![
            // Shear's curve isn't mapped to Photoshop's grid yet.
            sf("filter.distort.shear", json!({"amount": 40})),
            sf("filter.blur.gaussianBlur", json!({"radius": 2.0})),
            sf(UNSUPPORTED_FILTER, json!({"psd": "zz"})),
            sf("image.adjustments.levels", json!({"cyan": {"gamma": 2.0}})),
            sf("image.adjustments.shadowsHighlights", json!({"shadowAmount": 50})),
        ],
        ..Default::default()
    };
    let mut w = Vec::new();
    let d = filter_fx(&stack, "L", &mut w);
    assert_eq!(w.len(), 4, "{w:?}");
    let back = filter_stack(&d);
    assert_eq!(back.filters, vec![stack.filters[1].clone()]);
}

#[test]
fn placed_blocks_parse_back() {
    let t = Affine { m: [0.5, 0.1, -0.1, 0.5, 10.0, 20.0] };
    let stack = FilterStack { enabled: false, filters: vec![sf("filter.blur.gaussianBlur", json!({"radius": 3.0}))], mask_enabled: false, mask_linked: true };
    let mut w = Vec::new();
    let spec = PlacedSpec {
        idnt: "id-1",
        placed: "pl-2",
        transform: t,
        perspective: None,
        size: (64.0, 32.0),
        dpi: 72.0,
        warp: None,
        filter_fx: Some(filter_fx(&stack, "L", &mut w)),
    };
    let sold = sold_bytes(None, &spec, &mut w);
    assert!(w.is_empty());
    assert_eq!(sold.len() % 4, 0);
    let p = parse_sold(&sold).unwrap();
    assert_eq!((p.idnt.as_str(), p.placed.as_str()), ("id-1", "pl-2"));
    for (a, b) in p.transform.m.iter().zip(t.m) {
        assert!((a - b).abs() < 1e-9);
    }
    assert_eq!(p.stack, Some(stack));
    let tb = photocraft_psd::TaggedBlock::new(*b"SoLd", sold.clone());
    tb.check_structure().unwrap();
    let pl = photocraft_psd::TaggedBlock::new(*b"PlLd", plld_bytes(&spec, &mut w));
    pl.check_structure().unwrap();
    // A template keeps its other keys; an unmoved transform keeps its exact quad.
    let mut tmpl = sold_descriptor(&sold).unwrap();
    tmpl.items.push((Id::new("extraKey"), Value::Integer(7)));
    let again = sold_bytes(Some(&tmpl), &PlacedSpec { filter_fx: None, ..spec }, &mut w);
    let d = sold_descriptor(&again).unwrap();
    assert_eq!(d.get("extraKey"), Some(&Value::Integer(7)));
    assert!(d.get("filterFX").is_none());
    assert_eq!(d.get("Trnf"), tmpl.get("Trnf"));
}

/// A Distort / Perspective placement goes out as its four corners (`Trnf`, `nonAffineTransform`)
/// and comes back as the same projective map; an affine one reads back as no perspective.
#[test]
fn perspective_placement_round_trips_through_sold() {
    let h = photocraft_algo::transform::Homography::rect_to_quad([0.0, 0.0, 64.0, 32.0], [[10.0, 5.0], [80.0, 8.0], [70.0, 50.0], [12.0, 40.0]]).unwrap();
    let mut w = Vec::new();
    let mut spec = PlacedSpec {
        idnt: "id",
        placed: "pl",
        transform: Affine::IDENTITY,
        perspective: Some(h.0),
        size: (64.0, 32.0),
        dpi: 72.0,
        warp: None,
        filter_fx: None,
    };
    let sold = sold_bytes(None, &spec, &mut w);
    let back = crate::blocks::parse_smart_perspective(b"SoLd", &sold).expect("a projective placement");
    let n = |m: [f64; 9]| m.map(|v| v / m[8]);
    for (a, b) in n(back).iter().zip(n(h.0)) {
        assert!((a - b).abs() < 1e-9 * (1.0 + b.abs()), "{back:?} vs {:?}", h.0);
    }
    // Affine placements carry no perspective.
    spec.perspective = None;
    spec.transform = Affine { m: [2.0, 0.0, 0.0, 2.0, 5.0, 7.0] };
    let sold = sold_bytes(None, &spec, &mut w);
    assert_eq!(crate::blocks::parse_smart_perspective(b"SoLd", &sold), None);
    assert_eq!(crate::blocks::parse_smart_perspective(b"PlLd", &sold), None);
}

/// `quad_moved` compares the corners a template stores with the placement's: unchanged keeps
/// the template's quad, a moved corner (affine or projective) or a missing quad rewrites it.
#[test]
fn quad_moved_detects_corner_changes() {
    let t = Affine { m: [1.0, 0.0, 0.0, 1.0, 3.0, 4.0] };
    let mut w = Vec::new();
    let mut spec = PlacedSpec { idnt: "id", placed: "pl", transform: t, perspective: None, size: (20.0, 10.0), dpi: 72.0, warp: None, filter_fx: None };
    let template = parse_sold(&sold_bytes(None, &spec, &mut w)).expect("parses").descriptor;
    assert!(!quad_moved(&template, &spec, spec.warp), "same placement");
    spec.transform = Affine { m: [1.0, 0.0, 0.0, 1.0, 3.5, 4.0] };
    assert!(quad_moved(&template, &spec, spec.warp), "moved half a pixel");
    spec.transform = t;
    spec.perspective =
        photocraft_algo::transform::Homography::rect_to_quad([0.0, 0.0, 20.0, 10.0], [[3.0, 4.0], [23.0, 4.0], [20.0, 12.0], [3.0, 14.0]]).map(|h| h.0);
    assert!(quad_moved(&template, &spec, spec.warp), "one corner moved (Distort)");
    assert!(quad_moved(&Descriptor::new("null"), &spec, spec.warp), "no stored quad");
}

/// Corrupted placed-layer data and filter descriptors never panic: they parse to something
/// (unknown filters stay verbatim) or to nothing.
#[test]
fn mutated_placed_data_never_panics() {
    let stack = FilterStack {
        filters: vec![
            sf("filter.blur.gaussianBlur", json!({"radius": 3.0})),
            sf("image.adjustments.curves", json!({"points": [[0.0, 0.0], [255.0, 255.0]]})),
            sf("image.adjustments.levels", json!({"inBlack": 3, "inWhite": 250, "gamma": 1.1})),
        ],
        ..Default::default()
    };
    let mut w = Vec::new();
    let spec = PlacedSpec {
        idnt: "id",
        placed: "pl",
        transform: Affine::IDENTITY,
        perspective: None,
        size: (8.0, 8.0),
        dpi: 72.0,
        warp: None,
        filter_fx: Some(filter_fx(&stack, "L", &mut w)),
    };
    let sold = sold_bytes(None, &spec, &mut w);
    let mut state = 0x2545_F491_4F6C_DD1Du64;
    let mut next = || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    for _ in 0..3000 {
        let mut b = sold.clone();
        for _ in 0..1 + next() % 3 {
            let i = (next() as usize) % b.len();
            b[i] = next() as u8;
        }
        if let Some(p) = parse_sold(&b) {
            for f in p.stack.iter().flat_map(|s| &s.filters) {
                let _ = item_for_filter(f);
            }
            let _ = sold_bytes(Some(&p.descriptor), &spec, &mut w);
        }
    }
}

#[test]
fn warps_are_written_and_read_back() {
    use photocraft_geom::warp::BezierMesh;
    let bounds = [0.0, 0.0, 60.0, 30.0];
    let mut custom = BezierMesh::identity(bounds, 1, 1);
    custom.points[15] = [67.0, 30.0];
    let mut multi = Warp::preset(WarpStyle::Custom, 0.0, bounds);
    let mut split = BezierMesh::identity(bounds, 2, 2);
    split.points[8][1] += 4.0;
    multi.mesh = Some(split);
    let mut arc = Warp::preset(WarpStyle::Arc, 40.0, bounds);
    arc.vertical = true;
    arc.h_distort = 10.0;
    let mut c = Warp::preset(WarpStyle::Custom, 0.0, bounds);
    c.mesh = Some(custom);
    for (w, back) in [(Some(arc.clone()), Some(arc)), (Some(c.clone()), Some(c)), (Some(multi.clone()), Some(multi)), (None, None)] {
        let mut warnings = Vec::new();
        let spec = PlacedSpec {
            idnt: "i",
            placed: "p",
            transform: Affine::IDENTITY,
            perspective: None,
            size: (60.0, 30.0),
            dpi: 72.0,
            warp: w.as_ref(),
            filter_fx: None,
        };
        let sold = sold_bytes(None, &spec, &mut warnings);
        assert_eq!(crate::blocks::parse_placed_warp(b"SoLd", &sold), back);
        assert!(warnings.is_empty(), "{warnings:?}");
    }
}

/// A placed layer over a 50 × 25 source, at `transform`, with `warp`.
fn placed<'a>(transform: Affine, warp: Option<&'a Warp>) -> PlacedSpec<'a> {
    PlacedSpec { idnt: "i", placed: "p", transform, perspective: None, size: (50.0, 25.0), dpi: 72.0, warp, filter_fx: None }
}

/// A 3 × 2 patch split warp over the 50 × 25 source with one interior control point pulled.
fn split_warp() -> Warp {
    use photocraft_geom::warp::BezierMesh;
    let mut mesh = BezierMesh::identity([0.0, 0.0, 50.0, 25.0], 3, 2);
    mesh.points[10 * 3 + 4][1] += 3.0;
    Warp::custom(mesh, [0.0, 0.0, 50.0, 25.0])
}

/// An imported `soLD` descriptor holding `warp` as Photoshop writes a split warp: a
/// `quiltWarp` whose outer slices sit 0.6 px past the source, before a `warpNone` `warp`.
fn photoshop_template(warp: &Warp) -> Descriptor {
    let mesh = warp.mesh.as_ref().expect("a mesh");
    let slices = |k: &str, knots: &[f64], len: f64| {
        let n = knots.len() - 1;
        let values = knots
            .iter()
            .enumerate()
            .map(|(i, u)| {
                if i == 0 {
                    -0.6
                } else if i == n {
                    len + 0.6
                } else {
                    u * len
                }
            })
            .collect::<Vec<_>>();
        let prefix = values.len() as u32;
        Value::ObjectArray(ObjectArray { prefix, body: Descriptor::new("UntF").with(k, Value::UnitFloats { unit: *b"#Pxl", values }) })
    };
    let Value::Descriptor(plain) = warp_desc(None, (50.0, 25.0), &mut Vec::new()) else { panic!("a descriptor") };
    let mut quilt = plain.clone();
    quilt.class_id = Id::new("quiltWarp");
    set(&mut quilt, "warpStyle", enumv("warpStyle", "warpCustom"));
    let points = Descriptor::new("rationalPoint")
        .with("Hrzn", Value::UnitFloats { unit: *b"#Pxl", values: mesh.points.iter().map(|p| p[0]).collect() })
        .with("Vrtc", Value::UnitFloats { unit: *b"#Pxl", values: mesh.points.iter().map(|p| p[1]).collect() });
    let envelope = Descriptor::new("customEnvelopeWarp")
        .with("quiltSliceX", slices("quiltSliceX", &mesh.us, 50.0))
        .with("quiltSliceY", slices("quiltSliceY", &mesh.vs, 25.0))
        .with("meshPoints", Value::ObjectArray(ObjectArray { prefix: mesh.points.len() as u32, body: points }));
    quilt = quilt
        .with("deformNumRows", Value::Integer(mesh.ny() as i32))
        .with("deformNumCols", Value::Integer(mesh.nx() as i32))
        .with("customEnvelopeWarp", Value::Descriptor(envelope));
    let mut d = sold_descriptor(&sold_bytes(None, &placed(Affine::IDENTITY, None), &mut Vec::new())).expect("a descriptor");
    let at = d.items.iter().position(|(k, _)| k.is("warp")).expect("a warp key");
    d.items.insert(at, (Id::new("quiltWarp"), Value::Descriptor(quilt)));
    d
}

fn sold_of(d: Descriptor) -> Vec<u8> {
    let mut out = b"soLD".to_vec();
    out.extend_from_slice(&4u32.to_be_bytes());
    out.extend(VersionedDescriptor::new(d).to_bytes());
    out
}

fn quilt_of(d: &Descriptor) -> Option<&Descriptor> {
    get_desc(d, "quiltWarp")
}

/// A split warp of more patches along a side than a `quiltWarp` is read with (Warp Grid 64,
/// then one more split) is written as no warp, with a message, framed by the source rect, so
/// the layer reopens where it was. Before, its `quiltWarp` was refused on reopen while `Trnf`
/// still framed its control box: the layer came back stretched and shifted.
#[test]
fn a_split_warp_past_the_quilt_limit_keeps_its_placement() {
    use photocraft_geom::warp::BezierMesh;
    let mut mesh = BezierMesh::identity([0.0, 0.0, 50.0, 25.0], crate::blocks::QUILT_MAX_PATCHES, 1);
    assert!(mesh.split_u(0.005));
    mesh.points[0] = [-10.0, -5.0];
    let warp = Warp::custom(mesh, [0.0, 0.0, 50.0, 25.0]);
    let transform = Affine { m: [2.0, 0.0, 0.0, 2.0, 120.0, 210.0] };
    let mut warnings = Vec::new();
    let sold = sold_bytes(None, &placed(transform, Some(&warp)), &mut warnings);
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert_eq!(crate::blocks::parse_smart(b"SoLd", &sold).1, transform);
    assert_eq!(crate::blocks::parse_placed_warp(b"SoLd", &sold), None);
    assert!(quilt_of(&sold_descriptor(&sold).expect("a descriptor")).is_none());
    // Over an imported split warp, the stale `quiltWarp` goes and the placement holds.
    let template = photoshop_template(&split_warp());
    let mut warnings = Vec::new();
    let sold = sold_bytes(Some(&template), &placed(transform, Some(&warp)), &mut warnings);
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert_eq!(crate::blocks::parse_smart(b"SoLd", &sold).1, transform);
    assert!(quilt_of(&sold_descriptor(&sold).expect("a descriptor")).is_none());
    // So does one this reader can't read, which Photoshop would still apply.
    let mut template = photoshop_template(&split_warp());
    let Some((_, Value::Descriptor(q))) = template.items.iter_mut().find(|(k, _)| k.is("quiltWarp")) else { panic!("a quiltWarp") };
    set(q, "deformNumRows", Value::Integer(0));
    let sold = sold_bytes(Some(&template), &placed(transform, Some(&warp)), &mut Vec::new());
    assert!(quilt_of(&sold_descriptor(&sold).expect("a descriptor")).is_none());
    // At the limit the split warp is still written.
    let mut mesh = BezierMesh::identity([0.0, 0.0, 50.0, 25.0], crate::blocks::QUILT_MAX_PATCHES, 1);
    mesh.points[0] = [-10.0, -5.0];
    let warp = Warp::custom(mesh, [0.0, 0.0, 50.0, 25.0]);
    let mut warnings = Vec::new();
    let sold = sold_bytes(None, &placed(transform, Some(&warp)), &mut warnings);
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(crate::blocks::parse_smart(b"SoLd", &sold).1, transform);
    assert_eq!(crate::blocks::parse_placed_warp(b"SoLd", &sold), Some(warp));
}

/// A split (multi-patch) custom warp is written as Photoshop writes it, a `quiltWarp` before a
/// `warpNone` `warp`, and reads back with its mesh and its transform. Before, the warp was
/// written as none while `Trnf` was still the image of its control-point box, so the layer
/// came back stretched and shifted.
#[test]
fn split_warp_round_trips_through_sold() {
    use photocraft_geom::warp::BezierMesh;
    let mut mesh = BezierMesh::identity([0.0, 0.0, 50.0, 25.0], 2, 1);
    mesh.points[0] = [-10.0, -5.0];
    let warp = Warp::custom(mesh, [0.0, 0.0, 50.0, 25.0]);
    let transform = Affine { m: [2.0, 0.0, 0.0, 2.0, 120.0, 210.0] };
    let mut warnings = Vec::new();
    let sold = sold_bytes(None, &placed(transform, Some(&warp)), &mut warnings);
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(crate::blocks::parse_smart(b"SoLd", &sold).1, transform);
    assert_eq!(crate::blocks::parse_placed_warp(b"SoLd", &sold), Some(warp));
    let d = sold_descriptor(&sold).expect("a descriptor");
    let at = |key: &str| d.items.iter().position(|(k, _)| k.is(key));
    assert!(at("quiltWarp").expect("a quiltWarp") < at("warp").expect("a warp"));
    let plain = get_desc(&d, "warp").expect("a warp");
    assert_eq!(enum_of(plain, "warpStyle"), Some(&b"warpNone"[..]));
    assert!(plain.get("customEnvelopeWarp").is_none());
    let quilt = quilt_of(&d).expect("a quiltWarp");
    assert!(quilt.class_id.is("quiltWarp"));
    assert_eq!((quilt.get("deformNumCols"), quilt.get("deformNumRows")), (Some(&Value::Integer(7)), Some(&Value::Integer(4))));
    let env = get_desc(quilt, "customEnvelopeWarp").expect("an envelope");
    let prefix = |k: &str| match env.get(k) {
        Some(Value::ObjectArray(a)) => Some(a.prefix),
        _ => None,
    };
    assert_eq!((prefix("quiltSliceX"), prefix("quiltSliceY"), prefix("meshPoints")), (Some(3), Some(2), Some(28)));
}

/// A template's split warp is replaced along with its `warp`: a warp changed to Arc, to none,
/// to one patch or to another mesh reads back as changed, not as the imported split warp.
#[test]
fn a_replaced_split_warp_does_not_come_back() {
    use photocraft_geom::warp::BezierMesh;
    let bounds = [0.0, 0.0, 50.0, 25.0];
    let split = split_warp();
    let template = photoshop_template(&split);
    let arc = Warp::preset(WarpStyle::Arc, 30.0, bounds);
    let mut one = BezierMesh::identity(bounds, 1, 1);
    one.points[5][0] += 2.0;
    let one = Warp::custom(one, bounds);
    let mut resplit = BezierMesh::identity(bounds, 2, 2);
    resplit.points[8][1] += 4.0;
    let resplit = Warp::custom(resplit, bounds);
    for w in [Some(arc), None, Some(one), Some(resplit)] {
        let mut warnings = Vec::new();
        let sold = sold_bytes(Some(&template), &placed(Affine::IDENTITY, w.as_ref()), &mut warnings);
        assert_eq!(crate::blocks::parse_placed_warp(b"SoLd", &sold), w);
        let d = sold_descriptor(&sold).expect("a descriptor");
        assert_eq!(quilt_of(&d).is_some(), w.as_ref().is_some_and(|w| w.mesh.as_ref().is_some_and(|m| m.points.len() > 16)), "{w:?}");
        assert!(warnings.is_empty(), "{warnings:?}");
    }
}

/// An imported split warp that nothing changed keeps Photoshop's `quiltWarp` as stored when
/// the layer is rewritten for something else (here a move); a mesh edited on the same grid
/// keeps Photoshop's slices.
#[test]
fn an_imported_split_warp_keeps_photoshop_s_quilt() {
    let template = photoshop_template(&split_warp());
    let read = crate::blocks::parse_placed_warp(b"SoLd", &sold_of(template.clone())).expect("the split warp");
    let moved = Affine { m: [1.0, 0.0, 0.0, 1.0, 4.0, 0.0] };
    let sold = sold_bytes(Some(&template), &placed(moved, Some(&read)), &mut Vec::new());
    let d = sold_descriptor(&sold).expect("a descriptor");
    assert_eq!(d.get("quiltWarp"), template.get("quiltWarp"));
    assert_eq!(crate::blocks::parse_smart(b"SoLd", &sold).1, moved);
    let mut edited = read.clone();
    if let Some(m) = edited.mesh.as_mut() {
        m.points[12][0] += 1.5;
    }
    let sold = sold_bytes(Some(&template), &placed(Affine::IDENTITY, Some(&edited)), &mut Vec::new());
    assert_eq!(crate::blocks::parse_placed_warp(b"SoLd", &sold), Some(edited));
    let slices = |d: &Descriptor| {
        let env = get_desc(quilt_of(d)?, "customEnvelopeWarp")?;
        Some((env.get("quiltSliceX").cloned(), env.get("quiltSliceY").cloned()))
    };
    assert_eq!(slices(&sold_descriptor(&sold).expect("a descriptor")), slices(&template));
}

/// The legacy `PlLd` record has no split warp: it stores none, with its corners framed by the
/// source rect, so a reader of `PlLd` alone places the layer where its transform puts it.
#[test]
fn plld_of_a_split_warp_is_framed_by_the_source() {
    let warp = split_warp();
    let transform = Affine { m: [2.0, 0.0, 0.0, 2.0, 120.0, 210.0] };
    let mut warnings = Vec::new();
    let plld = plld_bytes(&placed(transform, Some(&warp)), &mut warnings);
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(crate::blocks::parse_plld_warp(&plld), crate::blocks::PlldWarp::None);
    let (_, corners) = crate::blocks::parse_plld(&plld).expect("a record");
    assert_eq!(crate::blocks::plld_transform(&corners, Some((50.0, 25.0)), None).0, transform);
}

/// Export writes `Trnf` as the image of the custom warp's control-point box, the inverse of
/// import: a spilled mesh comes back with the same transform and the quad Photoshop expects.
#[test]
fn warp_control_box_quad_round_trips() {
    use photocraft_geom::warp::BezierMesh;
    let mut mesh = BezierMesh::identity([0.0, 0.0, 50.0, 25.0], 1, 1);
    mesh.points[0] = [-10.0, -5.0];
    mesh.points[15] = [60.0, 25.0];
    let mut warp = Warp::preset(WarpStyle::Custom, 0.0, [0.0, 0.0, 50.0, 25.0]);
    warp.mesh = Some(mesh);
    let transform = Affine { m: [2.0, 0.0, 0.0, 2.0, 120.0, 210.0] };
    let spec = PlacedSpec { idnt: "i", placed: "p", transform, perspective: None, size: (50.0, 25.0), dpi: 72.0, warp: Some(&warp), filter_fx: None };
    let sold = sold_bytes(None, &spec, &mut Vec::new());
    let p = parse_sold(&sold).expect("parses");
    let Some(Value::List(q)) = p.descriptor.get("Trnf") else { panic!("no Trnf") };
    let q: Vec<f64> = q.iter().filter_map(|v| num(Some(v))).collect();
    assert_eq!(q, [100.0, 200.0, 240.0, 200.0, 240.0, 260.0, 100.0, 260.0]);
    assert_eq!(p.transform, transform);
    // The imported quad reads as unmoved, so the template's `Trnf` is kept as stored.
    assert!(!quad_moved(&p.descriptor, &spec, spec.warp));
}

/// #4112: a regenerated `SoLd` (after Replace Contents, a move…) writes a cylinder back as
/// `warpCylinder` and its seven values, not as no warp, and frames its control-point box with
/// the quad, affine or projective, so import places it where it was.
#[test]
fn cylinder_warp_round_trips_through_sold() {
    let v = [100.0, 800.0, 900.0, 200.0, 0.3, 0.3, 0.5];
    let warp = Warp::cylinder(&v, [0.0, 0.0, 1000.0, 1000.0]).unwrap();
    let transform = Affine { m: [0.5, 0.0, 0.0, 0.5, 40.0, 30.0] };
    let mut spec = PlacedSpec { idnt: "i", placed: "p", transform, perspective: None, size: (1000.0, 1000.0), dpi: 72.0, warp: Some(&warp), filter_fx: None };
    let mut warnings = Vec::new();
    let sold = sold_bytes(None, &spec, &mut warnings);
    assert!(warnings.is_empty(), "{warnings:?}");
    let p = parse_sold(&sold).expect("parses");
    let Some(Value::Descriptor(w)) = p.descriptor.get("warp") else { panic!("no warp") };
    assert_eq!(crate::blocks::enum_of(w, "warpStyle"), Some(b"warpCylinder".as_slice()));
    let Some(Value::List(values)) = w.get("warpValues") else { panic!("no warpValues") };
    assert_eq!(values.iter().filter_map(|x| num(Some(x))).collect::<Vec<_>>(), v);
    assert!(w.get("customEnvelopeWarp").is_none());
    assert_eq!(crate::blocks::parse_placed_warp(b"SoLd", &sold), Some(warp.clone()));
    for (a, b) in p.transform.m.iter().zip(transform.m) {
        assert!((a - b).abs() < 1e-9, "{:?}", p.transform);
    }
    // The quad is the image of the control-point box, not of the source rect.
    let cb = warp.mesh.as_ref().unwrap().control_bounds();
    let Some(Value::List(q)) = p.descriptor.get("Trnf") else { panic!("no Trnf") };
    let q: Vec<f64> = q.iter().filter_map(|x| num(Some(x))).collect();
    let want = [cb[0], cb[1], cb[2], cb[1], cb[2], cb[3], cb[0], cb[3]].map(|c| 0.5 * c);
    for (i, (a, b)) in q.iter().zip(want).enumerate() {
        let shift = if i % 2 == 0 { 40.0 } else { 30.0 };
        assert!((a - (b + shift)).abs() < 1e-9, "corner {i}: {q:?}");
    }
    assert!(!quad_moved(&p.descriptor, &spec, spec.warp));
    // A projective placement comes back as the same map.
    let h = photocraft_algo::transform::Homography::rect_to_quad(cb, [[10.0, 20.0], [410.0, 60.0], [400.0, 480.0], [30.0, 520.0]]).unwrap();
    spec.perspective = Some(h.0);
    let sold = sold_bytes(None, &spec, &mut warnings);
    let back = crate::blocks::parse_smart_perspective(b"SoLd", &sold).expect("projective");
    let n = |m: [f64; 9]| m.map(|x| x / m[8]);
    for (a, b) in n(back).iter().zip(n(h.0)) {
        assert!((a - b).abs() < 1e-9 * (1.0 + b.abs()), "{back:?} vs {:?}", h.0);
    }
    let plld = plld_bytes(&spec, &mut warnings);
    assert_eq!(crate::blocks::parse_plld_warp(&plld), crate::blocks::PlldWarp::Warp(warp));
    assert!(warnings.is_empty(), "{warnings:?}");
}

#[test]
fn hex_and_uuid() {
    assert_eq!(from_hex(&to_hex(&[0, 1, 0xab, 0xff])), Some(vec![0, 1, 0xab, 0xff]));
    assert_eq!(from_hex("abc"), None);
    assert_eq!(from_hex("zz"), None);
    let u = uuid_from(b"x");
    assert_eq!(u.len(), 36);
    assert_eq!(u, uuid_from(b"x"));
    assert_ne!(u, uuid_from(b"y"));
}

// Render filters as Photoshop 27 saves them on a smart object (`filterFXList` items, with
// neutral blending): Clouds and Difference Clouds in white on black, Fibers in black on white,
// Lens Flare with the 50-300mm Zoom at 159%.
const PHOTOSHOP_CLOUDS: &str = "0000000100000000000866696c746572465800000008000000004e6d202054455854000000070043006c006f00750064007300000000000c626c656e644f7074696f6e734f626a63000000000000000c626c656e644f7074696f6e7300000002000000004f706374556e7446235072634059000000000000000000004d642020656e756d00000000426c6e4d000000066e6f726d616c00000000656e6162626f6f6c010000000a6861736f7074696f6e73626f6f6c0100000000467267434f626a630000000100000000000052474243000000030000000052642020646f7562406fe000000000000000000047726e20646f7562406fe0000000000000000000426c2020646f7562406fe000000000000000000042636b434f626a630000000100000000000052474243000000030000000052642020646f756200000000000000000000000047726e20646f7562000000000000000000000000426c2020646f7562000000000000000000000000466c74724f626a63000000070043006c006f007500640073000000000000436c64730000000100000000466c52736c6f6e67000049e40000000866696c74657249446c6f6e67436c6473";
const PHOTOSHOP_DIFFERENCE_CLOUDS: &str = "0000000100000000000866696c746572465800000008000000004e6d202054455854000000120044006900660066006500720065006e0063006500200043006c006f00750064007300000000000c626c656e644f7074696f6e734f626a63000000000000000c626c656e644f7074696f6e7300000002000000004f706374556e7446235072634059000000000000000000004d642020656e756d00000000426c6e4d000000066e6f726d616c00000000656e6162626f6f6c010000000a6861736f7074696f6e73626f6f6c0100000000467267434f626a630000000100000000000052474243000000030000000052642020646f7562406fe000000000000000000047726e20646f7562406fe0000000000000000000426c2020646f7562406fe000000000000000000042636b434f626a630000000100000000000052474243000000030000000052642020646f756200000000000000000000000047726e20646f7562000000000000000000000000426c2020646f7562000000000000000000000000466c74724f626a63000000120044006900660066006500720065006e0063006500200043006c006f007500640073000000000000446672430000000100000000466c52736c6f6e67000057d20000000866696c74657249446c6f6e6744667243";
const PHOTOSHOP_FIBERS: &str = "0000000100000000000866696c746572465800000008000000004e6d2020544558540000000a004600690062006500720073002e002e002e00000000000c626c656e644f7074696f6e734f626a63000000000000000c626c656e644f7074696f6e7300000002000000004f706374556e7446235072634059000000000000000000004d642020656e756d00000000426c6e4d000000066e6f726d616c00000000656e6162626f6f6c010000000a6861736f7074696f6e73626f6f6c0100000000467267434f626a630000000100000000000052474243000000030000000052642020646f756200000000000000000000000047726e20646f7562000000000000000000000000426c2020646f756200000000000000000000000042636b434f626a630000000100000000000052474243000000030000000052642020646f7562406fe000000000000000000047726e20646f7562406fe0000000000000000000426c2020646f7562406fe0000000000000000000466c74724f626a630000000700460069006200650072007300000000000046627273000000030000000056726e636c6f6e670000001000000000537472676c6f6e670000000400000000526e64536c6f6e67000062f10000000866696c74657249446c6f6e6746627273";
const PHOTOSHOP_LENS_FLARE: &str = "0000000100000000000866696c746572465800000008000000004e6d2020544558540000000e004c0065006e007300200046006c006100720065002e002e002e00000000000c626c656e644f7074696f6e734f626a63000000000000000c626c656e644f7074696f6e7300000002000000004f706374556e7446235072634059000000000000000000004d642020656e756d00000000426c6e4d000000066e6f726d616c00000000656e6162626f6f6c010000000a6861736f7074696f6e73626f6f6c0100000000467267434f626a630000000100000000000052474243000000030000000052642020646f756200000000000000000000000047726e20646f7562000000000000000000000000426c2020646f756200000000000000000000000042636b434f626a630000000100000000000052474243000000030000000052642020646f7562406fe000000000000000000047726e20646f7562406fe0000000000000000000426c2020646f7562406fe0000000000000000000466c74724f626a630000000b004c0065006e007300200046006c0061007200650000000000004c6e73460000000300000000427267686c6f6e670000009f00000000466c72434f626a6300000001000000000000506e7420000000020000000048727a6e646f75623fe86666600000000000000056727463646f75623fe60b60c0000000000000004c6e7320656e756d000000004c6e7320000000005a6d20200000000866696c74657249446c6f6e674c6e7346";

fn photoshop(hex: &str) -> Descriptor {
    Descriptor::from_bytes(&from_hex(hex).unwrap()).unwrap()
}

fn set_lens(item: &mut Descriptor, code: [u8; 4]) {
    let Some(Value::Descriptor(fltr)) = item.items.iter_mut().find(|(k, _)| k.is("Fltr")).map(|(_, v)| v) else { panic!("no Fltr") };
    set(fltr, "Lns ", Value::Enumerated { type_id: Id::new("Lns "), value: Id::Code(code) });
}

#[test]
fn photoshop_render_filters_read_and_write_like_photoshop() {
    let colours = |fg: f64, bg: f64| (json!([fg, fg, fg, 1.0]), json!([bg, bg, bg, 1.0]));
    let (white, black) = colours(1.0, 0.0);
    let cases = [
        (PHOTOSHOP_CLOUDS, "filter.render.clouds", json!({"seed": 18916, "foreground": white, "background": black})),
        (PHOTOSHOP_DIFFERENCE_CLOUDS, "filter.render.differenceClouds", json!({"seed": 22482, "foreground": white, "background": black})),
        (PHOTOSHOP_FIBERS, "filter.render.fibers", json!({"variance": 16, "strength": 4, "seed": 25329, "foreground": black, "background": white})),
        (
            PHOTOSHOP_LENS_FLARE,
            "filter.render.lensFlare",
            json!({"brightness": 159, "centerX": 0.762499988079071, "centerY": 0.6888889074325562, "lens": "zoom"}),
        ),
    ];
    for (hex, command, params) in cases {
        let item = photoshop(hex);
        let f = filter_from_item(&item);
        assert_eq!((f.command.as_str(), &f.params), (command, &params));
        // Written back, it is Photoshop's item again: name, colours, settings and key order.
        assert_eq!(item_for_filter(&f).unwrap().items, item.items, "{command}");
    }
}

#[test]
fn every_lens_flare_lens_round_trips_with_photoshops_code() {
    for (code, lens) in LENSES {
        let mut item = photoshop(PHOTOSHOP_LENS_FLARE);
        set_lens(&mut item, *code);
        let f = filter_from_item(&item);
        assert_eq!(f.params["lens"], lens);
        assert_eq!(item_for_filter(&f).unwrap().items, item.items, "{lens}");
    }
    // A lens we don't know keeps the filter verbatim.
    let mut item = photoshop(PHOTOSHOP_LENS_FLARE);
    set_lens(&mut item, *b"Xxxx");
    assert_eq!(filter_from_item(&item).command, UNSUPPORTED_FILTER);
}

/// The descriptor at `path` (nested keys) inside `item`, for editing.
fn desc_at<'a>(item: &'a mut Descriptor, path: &[&str]) -> &'a mut Descriptor {
    path.iter().fold(item, |d, key| {
        let Some(Value::Descriptor(x)) = d.items.iter_mut().find(|(k, _)| k.is(key)).map(|(_, v)| v) else { panic!("no {key}") };
        x
    })
}

#[test]
fn render_filters_with_settings_we_dont_model_stay_verbatim() {
    // Each case changes one thing in a Photoshop item: an extra setting, a repeated one, or a
    // value of another type than Photoshop writes. Opened as editable, a re-save would drop the
    // extra setting or rewrite the value; kept verbatim, the item is written back byte for byte.
    type Edit = Box<dyn Fn(&mut Descriptor)>;
    /// A descriptor path in the item (nested keys) and the edit made there.
    type Change = (&'static [&'static str], Edit);
    let extra: fn(&mut Descriptor) = |d| d.items.push((Id::new("Xtra"), Value::Integer(1)));
    let as_double = |key: &'static str| -> Edit { Box::new(move |d| set(d, key, Value::Double(1.0))) };
    let as_long = |key: &'static str| -> Edit { Box::new(move |d| set(d, key, Value::Integer(1))) };
    let repeat = |key: &'static str| -> Edit {
        Box::new(move |d| {
            let v = d.get(key).cloned().unwrap();
            d.items.push((Id::new(key), v));
        })
    };
    let colour_edits = || -> Vec<Change> {
        vec![
            (&["FrgC"], Box::new(|d: &mut Descriptor| d.items.push((Id::new("redFloat"), Value::Double(0.5))))),
            (&["BckC"], as_long("Grn ")),
            (&["FrgC"], Box::new(|d: &mut Descriptor| set(d, "Rd  ", Value::Double(300.0)))),
            (&["BckC"], Box::new(|d: &mut Descriptor| d.class_id = Id::new("HSBC"))),
        ]
    };
    let mut cases: Vec<(&str, &'static [&'static str], Edit)> = Vec::new();
    for hex in [PHOTOSHOP_CLOUDS, PHOTOSHOP_DIFFERENCE_CLOUDS] {
        cases.push((hex, &["Fltr"], Box::new(extra)));
        cases.push((hex, &["Fltr"], repeat("FlRs")));
        cases.push((hex, &["Fltr"], as_double("FlRs")));
        cases.extend(colour_edits().into_iter().map(|(p, e)| (hex, p, e)));
    }
    cases.push((PHOTOSHOP_FIBERS, &["Fltr"], Box::new(extra)));
    cases.push((PHOTOSHOP_FIBERS, &["Fltr"], repeat("Strg")));
    for key in ["Vrnc", "Strg", "RndS"] {
        cases.push((PHOTOSHOP_FIBERS, &["Fltr"], as_double(key)));
    }
    cases.extend(colour_edits().into_iter().map(|(p, e)| (PHOTOSHOP_FIBERS, p, e)));
    cases.push((PHOTOSHOP_LENS_FLARE, &["Fltr"], Box::new(extra)));
    cases.push((PHOTOSHOP_LENS_FLARE, &["Fltr"], repeat("Brgh")));
    cases.push((PHOTOSHOP_LENS_FLARE, &["Fltr"], as_double("Brgh")));
    cases.push((PHOTOSHOP_LENS_FLARE, &["Fltr"], Box::new(|d| set(d, "Lns ", Value::Enumerated { type_id: Id::new("Xxxx"), value: Id::Code(*b"Zm  ") }))));
    cases.push((PHOTOSHOP_LENS_FLARE, &["Fltr", "FlrC"], Box::new(extra)));
    cases.push((PHOTOSHOP_LENS_FLARE, &["Fltr", "FlrC"], as_long("Hrzn")));
    cases.push((PHOTOSHOP_LENS_FLARE, &["Fltr", "FlrC"], Box::new(|d| set(d, "Vrtc", Value::UnitFloat { unit: *b"#Pxl", value: 0.5 }))));
    for (i, (hex, path, edit)) in cases.iter().enumerate() {
        let mut item = photoshop(hex);
        edit(desc_at(&mut item, path));
        let f = filter_from_item(&item);
        assert_eq!(f.command, UNSUPPORTED_FILTER, "case {i} ({path:?}): {:?}", f.params);
        assert_eq!(item_for_filter(&f).unwrap().to_bytes(), item.to_bytes(), "case {i} ({path:?})");
    }
}

#[test]
fn render_filters_made_in_photocraft_are_written() {
    // Without colours (an older document or a script), Photoshop's default black and white.
    let back = filter_from_item(&item_for_filter(&sf("filter.render.clouds", json!({"seed": 7}))).unwrap());
    assert_eq!(back.params, json!({"seed": 7, "foreground": [0.0, 0.0, 0.0, 1.0], "background": [1.0, 1.0, 1.0, 1.0]}));
    // A seed above i32::MAX keeps its bit pattern, and colours may be hex.
    let p = json!({"variance": 30, "strength": 10, "seed": 4_000_000_000u32, "foreground": "#ff0000", "background": [0.0, 0.0, 1.0, 1.0]});
    let back = filter_from_item(&item_for_filter(&sf("filter.render.fibers", p)).unwrap());
    assert_eq!(
        back.params,
        json!({"variance": 30, "strength": 10, "seed": 4_000_000_000u32, "foreground": [1.0, 0.0, 0.0, 1.0], "background": [0.0, 0.0, 1.0, 1.0]})
    );
    let flare = sf("filter.render.lensFlare", json!({"brightness": 120, "centerX": 0.25, "centerY": 0.75, "lens": "moviePrime"}));
    assert_eq!(filter_from_item(&item_for_filter(&flare).unwrap()).params, flare.params);
    // Lenses and colours we can't write are reported, not guessed.
    assert!(item_for_filter(&sf("filter.render.lensFlare", json!({"lens": "fisheye"}))).unwrap_err().contains("fisheye"));
    assert!(item_for_filter(&sf("filter.render.clouds", json!({"foreground": "red"}))).is_err());
}
