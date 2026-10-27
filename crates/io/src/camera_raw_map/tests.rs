use super::*;
use crate::smart_map::{UNSUPPORTED_FILTER, filter_from_item, item_for_filter};
use photocraft_color::BlendMode;

// Parameter values observed in the supplied 18.4 PSD. No source pixels or private
// document metadata are included in this synthetic regression fixture.
fn source_item() -> Descriptor {
    let mut d = Descriptor::new(CLASS)
        .with("CrVe", Value::Text(UnicodeString::new_nul("18.4")))
        .with("PrVN", Value::Integer(6))
        .with("PrVe", Value::Integer(251920384))
        .with("Ex12", Value::Double(1.15));
    for (key, value) in [
        ("Cr12", -38),
        ("Hi12", 35),
        ("Sh12", -50),
        ("Wh12", 49),
        ("Bk12", -34),
        ("PC_S", 0),
        ("PC_D", -21),
        ("PC_L", 38),
        ("PC_H", 0),
        ("PC_1", 25),
        ("PC_2", 50),
        ("PC_3", 75),
    ] {
        d = d.with(key, Value::Integer(value));
    }
    for (_, keys) in ARRAYS.iter().skip(1) {
        for key in *keys {
            let value = match *key {
                "HA_R" => -50,
                "HA_Y" => 29,
                "HA_G" => 1,
                _ => 0,
            };
            d = d.with(key, Value::Integer(value));
        }
    }
    let f = SmartFilter { command: COMMAND.into(), params: J::Null, blend: BlendMode::Normal, opacity: 1.0, visible: true };
    Descriptor::new("filterFX")
        .with("Nm  ", Value::Text(UnicodeString::new_nul("Camera Raw Filter")))
        .with("blendOptions", blend_options(&f))
        .with("enab", Value::Boolean(true))
        .with("hasoptions", Value::Boolean(true))
        .with("FrgC", rgbc(0.0))
        .with("BckC", rgbc(255.0))
        .with("Fltr", Value::Descriptor(d))
        .with("filterID", Value::Integer(FILTER_ID))
}

fn all_controls_item() -> Descriptor {
    let mut item = source_item();
    let mut d = get_desc(&item, "Fltr").unwrap().clone();
    set(&mut d, "WBal", Value::Enumerated { type_id: photocraft_psd::descriptor::Id::new("WBal"), value: photocraft_psd::descriptor::Id::new("Cst ") });
    set(&mut d, "ShpR", Value::Double(1.2));
    for (key, value) in [
        ("Temp", -26),
        ("Tint", -25),
        ("CrTx", -31),
        ("Cl12", 30),
        ("Dhze", -34),
        ("Vibr", 33),
        ("Strt", 33),
        ("Shrp", 43),
        ("ShpD", 36),
        ("ShpM", 30),
        ("LNR ", 47),
        ("LNRD", 50),
        ("CNR ", 25),
        ("CNRD", 50),
        ("STSH", 117),
        ("STSS", 52),
        ("STHH", 316),
        ("STHS", 70),
        ("STB ", 35),
        ("CgMH", 51),
        ("CgMS", 51),
        ("CgSL", -24),
        ("CgML", 41),
        ("CgHL", 28),
        ("CgBl", 36),
        ("CgGH", 20),
        ("CgGS", 61),
        ("CgGL", 45),
        ("GRNA", 45),
        ("GRNS", 25),
        ("GRNF", 50),
        ("PCVA", 31),
        ("PCVM", 50),
        ("PCVF", 50),
        ("PCVR", 0),
    ] {
        set(&mut d, key, Value::Integer(value));
    }
    for (key, neutral) in NEUTRAL_ONLY {
        set(&mut d, key, Value::Integer(*neutral));
    }
    for (key, x, y) in [("Crv ", 146, 102), ("CrvR", 90, 174), ("CrvG", 162, 122), ("CrvB", 129, 202)] {
        set(&mut d, key, Value::List([0, 0, x, y, 255, 255].into_iter().map(Value::Integer).collect()));
    }
    set(&mut item, "Fltr", Value::Descriptor(d));
    item
}

#[test]
fn all_observed_develop_controls_roundtrip_and_patch_only_the_edited_field() {
    let source = all_controls_item();
    let f = filter_from_item(&source);
    assert_eq!(f.command, COMMAND);
    assert_eq!(f.params["temperature"], -26.0);
    assert_eq!(f.params["texture"], -31.0);
    assert!((f.params["sharpenRadius"].as_f64().unwrap() - 1.2).abs() < 1e-6);
    assert_eq!(f.params["gradeHighlights"], json!({"hue": 316.0, "sat": 70.0, "lum": 28.0}));
    assert_eq!(f.params["pointCurveBlue"], json!([[0.0, 0.0], [129.0, 202.0], [255.0, 255.0]]));
    assert_eq!(item_for_filter(&f).unwrap(), source);
    let defaults = default_params().unwrap();
    for (param, key) in SCALARS {
        let mut edited = f.clone();
        let mut number = get_param(&defaults, param).unwrap().as_f64().unwrap();
        if get_param(&f.params, param).unwrap().as_f64() == Some(number) {
            number += 1.0;
        }
        *get_param_mut(&mut edited.params, param).unwrap() = json!(number);
        let mut expected = get_desc(&source, "Fltr").unwrap().clone();
        set(&mut expected, key, if matches!(*key, "Ex12" | "ShpR") { Value::Double(number) } else { Value::Integer(number as i32) });
        assert_eq!(get_desc(&item_for_filter(&edited).unwrap(), "Fltr"), Some(&expected), "{param}");
    }
    for (param, key) in CURVES {
        let mut edited = f.clone();
        edited.params[*param] = json!([]);
        let item = item_for_filter(&edited).unwrap();
        let mut expected = get_desc(&source, "Fltr").unwrap().clone();
        set(&mut expected, key, Value::List([0, 0, 255, 255].into_iter().map(Value::Integer).collect()));
        assert_eq!(get_desc(&item, "Fltr"), Some(&expected), "{param}");
        assert_eq!(filter_from_item(&item).params[*param], json!([]), "{param}");
    }
}

#[test]
fn active_unimplemented_controls_and_malformed_curves_stay_opaque() {
    for (key, neutral) in NEUTRAL_ONLY {
        let mut item = all_controls_item();
        let mut d = get_desc(&item, "Fltr").unwrap().clone();
        set(&mut d, key, Value::Integer(neutral + 1));
        set(&mut item, "Fltr", Value::Descriptor(d));
        let f = filter_from_item(&item);
        assert_eq!(f.command, UNSUPPORTED_FILTER, "{key}");
        assert_eq!(item_for_filter(&f).unwrap(), item, "{key}");
    }
    for value in [
        Value::List(vec![Value::Integer(0)]),
        Value::List([0, 0, 0, 255].into_iter().map(Value::Integer).collect()),
        Value::List([0, 0, 255, 256].into_iter().map(Value::Integer).collect()),
        Value::List(vec![Value::Double(0.0); 4]),
        Value::List(vec![Value::Integer(0); 34]),
    ] {
        let mut item = all_controls_item();
        let mut d = get_desc(&item, "Fltr").unwrap().clone();
        set(&mut d, "Crv ", value);
        set(&mut item, "Fltr", Value::Descriptor(d));
        let f = filter_from_item(&item);
        assert_eq!(f.command, UNSUPPORTED_FILTER);
        assert_eq!(item_for_filter(&f).unwrap(), item);
    }
}

#[test]
#[ignore = "requires a user-supplied Camera Raw PSD"]
fn all_sliders_fixture() {
    let path = std::env::var("PHOTOCRAFT_CAMERA_RAW_PSD").expect("set PHOTOCRAFT_CAMERA_RAW_PSD");
    let doc = crate::import("fixture.psd", &std::fs::read(path).unwrap()).unwrap().document;
    let original = doc
        .walk()
        .into_iter()
        .find_map(|(_, _, layer)| {
            let photocraft_doc::LayerContent::Smart(sm) = &layer.content else { return None };
            sm.smart_filters.iter().find_map(|f| {
                let item = item_for_filter(f).ok()?;
                get_desc(&item, "Fltr").is_some_and(|d| d.class_id.is(CLASS)).then_some(item)
            })
        })
        .expect("a Camera Raw descriptor");
    assert_eq!(item_for_filter(&filter_from_item(&original)).unwrap(), original);
    let mut projected = original.clone();
    let mut d = get_desc(&original, "Fltr").unwrap().clone();
    // This is a codec projection, not an editable replacement for the source filter:
    // active Optics settings have no rendering counterpart, and PCVS is unverified.
    d.items.retain(|(key, _)| !["MDis", "VigA", "PCVS"].iter().any(|k| key.is(k)));
    set(&mut projected, "Fltr", Value::Descriptor(d));
    let mut f = filter_from_item(&projected);
    assert_eq!(f.command, COMMAND);
    assert_eq!(f.params["temperature"], -26.0);
    assert_eq!(f.params["sharpenAmount"], 43.0);
    assert_eq!(f.params["gradeGlobal"], json!({"hue": 20.0, "sat": 61.0, "lum": 45.0}));
    assert_eq!(f.params["pointCurveRed"], json!([[0.0, 0.0], [90.0, 174.0], [255.0, 255.0]]));
    assert_eq!(item_for_filter(&f).unwrap(), projected);
    f.params["exposure"] = json!(0.5);
    let mut expected = get_desc(&projected, "Fltr").unwrap().clone();
    set(&mut expected, "Ex12", Value::Double(0.5));
    assert_eq!(get_desc(&item_for_filter(&f).unwrap(), "Fltr"), Some(&expected));
}

#[test]
fn light_curve_and_hsl_are_editable_without_rewriting_unchanged_settings() {
    let source = source_item().with("futureOuterData", Value::RawData(vec![1, 2, 3, 4]));
    let mut f = filter_from_item(&source);
    assert_eq!(f.command, COMMAND);
    assert!((f.params["exposure"].as_f64().unwrap() - 1.15).abs() < 1e-6);
    assert_eq!(f.params["contrast"], -38.0);
    assert_eq!(f.params["curveDarks"], -21.0);
    assert_eq!(f.params["curveLights"], 38.0);
    assert_eq!(f.params["curveSplits"], json!([25.0, 50.0, 75.0]));
    assert_eq!(f.params["hslHue"], json!([-50.0, 0.0, 29.0, 1.0, 0.0, 0.0, 0.0, 0.0]));
    assert_eq!(item_for_filter(&f).unwrap(), source);
    f.params["exposure"] = json!(0.0);
    f.params["hslHue"][2] = json!(-20.0);
    f.visible = false;
    f.opacity = 0.4;
    f.blend = BlendMode::Multiply;
    let edited = item_for_filter(&f).unwrap();
    let mut expected = get_desc(&source, "Fltr").unwrap().clone();
    set(&mut expected, "Ex12", Value::Double(0.0));
    set(&mut expected, "HA_Y", Value::Integer(-20));
    assert_eq!(get_desc(&edited, "Fltr"), Some(&expected));
    assert_eq!(edited.get("futureOuterData"), source.get("futureOuterData"));
    let back = filter_from_item(&edited);
    assert_eq!((back.visible, back.blend, back.opacity), (false, BlendMode::Multiply, 0.4));
    assert_eq!(back.params["exposure"], 0.0);
    assert_eq!(back.params["hslHue"][2], -20.0);
}

#[test]
fn new_filters_write_every_verified_control_and_accept_explicit_defaults() {
    let mut params = default_params().unwrap();
    params["seed"] = json!(0);
    for (param, key) in SCALARS {
        let number = match *key {
            "Ex12" => 0.75,
            "ShpR" => 1.2,
            _ => 12.0,
        };
        *get_param_mut(&mut params, param).unwrap() = json!(number);
    }
    for (param, keys) in ARRAYS {
        params[*param] = if *param == "curveSplits" { json!([20, 45, 80]) } else { json!((0..keys.len()).map(|i| i as f64 + 1.0).collect::<Vec<_>>()) };
    }
    let f = SmartFilter { command: COMMAND.into(), params: params.clone(), blend: BlendMode::Screen, opacity: 0.5, visible: false };
    let item = item_for_filter(&f).unwrap();
    let back = filter_from_item(&Descriptor::from_bytes(&item.to_bytes()).unwrap());
    assert_eq!((&back.command, back.visible, back.blend, back.opacity), (&f.command, false, BlendMode::Screen, 0.5));
    // Compare slider numbers after both sides have been normalized through CameraRaw.
    let typed: CameraRaw = serde_json::from_value(params).unwrap();
    let expected = serde_json::to_value(typed).unwrap();
    for param in SCALARS.iter().map(|(p, _)| *p).chain(ARRAYS.iter().map(|(p, _)| *p)) {
        assert_eq!(get_param(&back.params, param), get_param(&expected, param));
    }
}

#[test]
fn unsupported_processing_settings_remain_opaque_and_lossless() {
    for (key, value) in [
        ("LCs ", Value::Text(UnicodeString::new("unknown masks"))),
        ("PrVN", Value::Integer(7)),
        ("PrVe", Value::Integer(123)),
        ("Ex12", Value::Double(f64::INFINITY)),
        ("Cr12", Value::Text(UnicodeString::new("bad type"))),
    ] {
        let mut item = source_item();
        let mut d = get_desc(&item, "Fltr").unwrap().clone();
        set(&mut d, key, value);
        set(&mut item, "Fltr", Value::Descriptor(d));
        let f = filter_from_item(&item);
        assert_eq!(f.command, UNSUPPORTED_FILTER, "{key}");
        assert_eq!(item_for_filter(&f).unwrap(), item, "{key}");
    }
}

#[test]
fn fractional_slider_levels_round_instead_of_dropping_the_filter() {
    let mut f = filter_from_item(&source_item());
    f.params["contrast"] = json!(-12.4);
    f.params["hslHue"][0] = json!(10.5);
    f.params["gradeShadows"]["hue"] = json!(359.7);
    f.params["pointCurve"] = json!([[0, 0], [100.2, 120.6], [200.6, 180.4], [255, 255]]);
    let item = item_for_filter(&f).unwrap();
    let d = get_desc(&item, "Fltr").unwrap();
    assert_eq!(d.get("Cr12"), Some(&Value::Integer(-12)));
    assert_eq!(d.get("HA_R"), Some(&Value::Integer(11)));
    assert_eq!(d.get("STSH"), Some(&Value::Integer(0)));
    assert_eq!(d.get("Crv "), Some(&Value::List([0, 0, 100, 121, 201, 180, 255, 255].into_iter().map(Value::Integer).collect())));
    let back = filter_from_item(&item);
    assert_eq!(back.command, COMMAND);
    assert_eq!(back.params["contrast"], -12.0);
}

#[test]
fn unsupported_or_malformed_exports_report_errors() {
    for params in [
        J::Null,
        json!({"exposure": "bright"}),
        json!({"exposure": 1e40}),
        json!({"exposure": 6}),
        json!({"contrast": 101}),
        json!({"hslHue": [1, 2]}),
        json!({"curveSplits": [80, 50, 20]}),
        json!({"curveSplits": [25, 25, 75]}),
        json!({"pointCurve": [[0, 0], [0, 255]]}),
        json!({"sharpenRadius": 4}),
        json!({"gradeShadows": {"hue": 361}}),
        json!({"gradeShadows": {"unknown": 1}}),
        json!({"seed": 1}),
        json!({"vignetteHighlights": 10}),
        json!({"futureSetting": 1}),
        json!({TEMPLATE_KEY: "zz"}),
        json!({TEMPLATE_KEY: "00"}),
        json!({TEMPLATE_KEY: 12}),
        json!({TEMPLATE_KEY: "0".repeat(MAX_TEMPLATE_BYTES * 2 + 1)}),
    ] {
        assert!(export_item(&params).is_err());
    }
    let wrong = Descriptor::new("filterFX").with("filterID", Value::Integer(1));
    assert!(export_item(&json!({TEMPLATE_KEY: to_hex(&wrong.to_bytes())})).is_err());
}
