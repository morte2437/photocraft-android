use super::*;

/// The parameterless events use an empty descriptor and accept both event identifiers.
#[test]
fn parameterless_stylize_and_pixelate_events_use_both_event_id_forms() {
    for (code, name, id) in [
        (*b"FndE", "findEdges", "filter.stylize.findEdges"),
        (*b"Slrz", "solarize", "filter.stylize.solarize"),
        (*b"Fct ", "facet", "filter.pixelate.facet"),
        (*b"Frgm", "fragment", "filter.pixelate.fragment"),
    ] {
        for event in [AtnEvent::Code(code), AtnEvent::name(name)] {
            let mut s = load_items(vec![AtnItem { event, descriptor: Some(desc("null")), ..Default::default() }]);
            assert_eq!(s.actions.list[0].steps, vec![(id.into(), json!({}))]);
            assert_eq!(saved_items(&mut s)[0].descriptor, Some(desc("null")));
            assert!(export_item(id, &json!({"layer": 17}), "Group D").is_none());
            assert!(export_item(id, &json!({"target": "mask"}), "Group D").is_none());
        }
    }
}

struct Fixture {
    code: &'static str,
    name: &'static str,
    id: &'static str,
    default: Descriptor,
    defaults: Value,
    changed: Descriptor,
    changes: Value,
}

/// Independent descriptor fixtures spell out measured key order, types, values and defaults.
fn fixtures() -> Vec<Fixture> {
    vec![
        Fixture {
            code: "Extr",
            name: "extrude",
            id: "filter.stylize.extrude",
            default: desc("null")
                .with("ExtS", D::Integer(30))
                .with("ExtD", D::Integer(30))
                .with("ExtF", D::Boolean(false))
                .with("ExtM", D::Boolean(false))
                .with("ExtT", enm("ExtT", "Blks"))
                .with("ExtR", enm("ExtR", "Rndm")),
            defaults: json!({"size":30,"depth":30,"solidFrontFaces":false,"maskIncompleteBlocks":false,"type":"blocks","depthMode":"random"}),
            changed: desc("null")
                .with("ExtS", D::Integer(42))
                .with("ExtD", D::Integer(91))
                .with("ExtF", D::Boolean(true))
                .with("ExtM", D::Boolean(true))
                .with("ExtT", enm("ExtT", "Pyrm"))
                .with("ExtR", enm("ExtR", "LvlB")),
            changes: json!({"size":42,"depth":91,"solidFrontFaces":true,"maskIncompleteBlocks":true,"type":"pyramids","depthMode":"levelBased"}),
        },
        Fixture {
            code: "TrcC",
            name: "traceContour",
            id: "filter.stylize.traceContour",
            default: desc("null").with("Lvl ", D::Integer(128)).with("Edg ", enm("CntE", "Upr ")),
            defaults: json!({"level":128,"edge":"upper"}),
            changed: desc("null").with("Lvl ", D::Integer(43)).with("Edg ", enm("CntE", "Lwr ")),
            changes: json!({"level":43,"edge":"lower"}),
        },
        Fixture {
            code: "Wnd ",
            name: "wind",
            id: "filter.stylize.wind",
            default: desc("null").with("WndM", enm("WndM", "Wnd ")).with("Drct", enm("Drct", "Left")),
            defaults: json!({"method":"wind","direction":"fromRight"}),
            changed: desc("null").with("WndM", enm("WndM", "Stgr")).with("Drct", enm("Drct", "Rght")),
            changes: json!({"method":"stagger","direction":"fromLeft"}),
        },
        Fixture {
            code: "Tls ",
            name: "tiles",
            id: "filter.stylize.tiles",
            default: desc("null").with("TlNm", D::Integer(10)).with("TlOf", D::Integer(10)).with("FlCl", enm("FlCl", "FlBc")),
            defaults: json!({"count":10,"maxOffset":10,"fill":"background"}),
            changed: desc("null").with("TlNm", D::Integer(17)).with("TlOf", D::Integer(23)).with("FlCl", enm("FlCl", "FlFr")),
            changes: json!({"count":17,"maxOffset":23,"fill":"foreground"}),
        },
        Fixture {
            code: "ClrH",
            name: "colorHalftone",
            id: "filter.pixelate.colorHalftone",
            default: desc("null")
                .with("Rds ", D::Integer(8))
                .with("Ang1", D::Integer(108))
                .with("Ang2", D::Integer(162))
                .with("Ang3", D::Integer(90))
                .with("Ang4", D::Integer(45)),
            defaults: json!({"maxRadius":8,"channel1":108,"channel2":162,"channel3":90,"channel4":45}),
            changed: desc("null")
                .with("Rds ", D::Integer(12))
                .with("Ang1", D::Integer(-10000))
                .with("Ang2", D::Integer(10000))
                .with("Ang3", D::Integer(-721))
                .with("Ang4", D::Integer(1001)),
            changes: json!({"maxRadius":12,"channel1":-10000,"channel2":10000,"channel3":-721,"channel4":1001}),
        },
        Fixture {
            code: "Mztn",
            name: "mezzotint",
            id: "filter.pixelate.mezzotint",
            default: desc("null").with("MztT", enm("MztT", "FnDt")),
            defaults: json!({"type":"fineDots"}),
            changed: desc("null").with("MztT", enm("MztT", "LngS")),
            changes: json!({"type":"longStrokes"}),
        },
        Fixture {
            code: "Msc ",
            name: "mosaic",
            id: "filter.pixelate.mosaic",
            default: desc("null").with("ClSz", px(2.0)),
            defaults: json!({"cellSize":2}),
            changed: desc("null").with("ClSz", px(31.5)),
            changes: json!({"cellSize":31.5}),
        },
        Fixture {
            code: "Pntl",
            name: "pointillize",
            id: "filter.pixelate.pointillize",
            default: desc("null").with("ClSz", D::Integer(5)),
            defaults: json!({"cellSize":5}),
            changed: desc("null").with("ClSz", D::Integer(19)),
            changes: json!({"cellSize":19}),
        },
    ]
}

/// Load an actual action file through the command boundary, including flag and name handling.
fn load_items(items: Vec<AtnItem>) -> Session {
    let file = AtnFile {
        name: UnicodeString::new_nul("Group D"),
        actions: vec![AtnAction { name: UnicodeString::new_nul("A"), items, ..Default::default() }],
        ..Default::default()
    };
    let bytes = atn::write(&file).unwrap();
    let mut s = Session::new();
    s.execute("actions.load", json!({"data":photocraft_paint::tile::b64_encode(&bytes)})).unwrap();
    s
}

/// Save through the real command and parse its bytes so descriptor order and types stay visible.
fn saved_items(s: &mut Session) -> Vec<AtnItem> {
    let result = s.execute("actions.save", json!({"action":"A"})).unwrap();
    assert_eq!(result["skipped"], json!([]));
    let bytes = photocraft_paint::tile::b64_decode(result["data"].as_str().unwrap()).unwrap();
    atn::parse(&bytes).unwrap().actions.remove(0).items
}

/// A measured four-character event with the supplied independent descriptor.
fn code_item(code: &str, descriptor: Descriptor) -> AtnItem {
    AtnItem { event: AtnEvent::Code(code.as_bytes().try_into().unwrap()), descriptor: Some(descriptor), ..Default::default() }
}

/// Check exact independent descriptors through real load/save commands, not a codec against itself.
#[test]
fn measured_defaults_and_every_changed_value_round_trip_in_order() {
    for fixture in fixtures() {
        for (descriptor, params) in [(&fixture.default, &fixture.defaults), (&fixture.changed, &fixture.changes)] {
            for named in [false, true] {
                let mut item = code_item(fixture.code, descriptor.clone());
                if named {
                    item.event = AtnEvent::name(fixture.name);
                }
                let mut s = load_items(vec![item]);
                assert_eq!(s.actions.list[0].steps, vec![(fixture.id.into(), params.clone())], "{} named={named}", fixture.code);
                let saved = saved_items(&mut s);
                assert_eq!(saved[0].event, AtnEvent::Code(fixture.code.as_bytes().try_into().unwrap()));
                assert_eq!(saved[0].descriptor.as_ref(), Some(descriptor), "{} named={named}", fixture.code);
            }
        }
        let mut s = load_items(vec![code_item(fixture.code, desc("null"))]);
        assert_eq!(s.actions.list[0].steps[0], (fixture.id.into(), fixture.defaults), "{} missing keys", fixture.code);
        assert_eq!(saved_items(&mut s)[0].descriptor, Some(fixture.default));
    }
}

/// Every measured numeric bound is checked independently, including angles beyond 360 degrees.
#[test]
fn numeric_imports_clamp_measured_ranges() {
    for (code, key, param, min, max) in [
        ("Extr", "ExtS", "size", 2, 255),
        ("Extr", "ExtD", "depth", 1, 255),
        ("TrcC", "Lvl ", "level", 0, 255),
        ("Tls ", "TlNm", "count", 1, 99),
        ("Tls ", "TlOf", "maxOffset", 1, 99),
        ("ClrH", "Rds ", "maxRadius", 4, 127),
        ("ClrH", "Ang1", "channel1", -10000, 10000),
        ("ClrH", "Ang2", "channel2", -10000, 10000),
        ("ClrH", "Ang3", "channel3", -10000, 10000),
        ("ClrH", "Ang4", "channel4", -10000, 10000),
        ("Msc ", "ClSz", "cellSize", 2, 200),
        ("Pntl", "ClSz", "cellSize", 3, 300),
    ] {
        for (input, expected) in [(i32::MIN, min), (min - 1, min), (min, min), (max, max), (max + 1, max), (i32::MAX, max)] {
            let value = if code == "Msc " { px(f64::from(input)) } else { D::Integer(input) };
            let mut s = load_items(vec![code_item(code, desc("null").with(key, value))]);
            assert_ne!(s.actions.list[0].steps[0].0, PHOTOSHOP_STEP, "{code}/{key}");
            assert_eq!(s.actions.list[0].steps[0].1[param], expected, "{code}/{key}/{input}");
            assert!(saved_items(&mut s)[0].descriptor.is_some());
        }
    }
    for unitless in [D::Integer(2), D::Double(2.0)] {
        let mut s = load_items(vec![code_item("Msc ", desc("null").with("ClSz", unitless))]);
        assert_eq!(saved_items(&mut s)[0].descriptor, Some(desc("null").with("ClSz", px(2.0))));
    }
}

/// Trace levels are whole numbers 0..255: imports clamp to that range and play like the clamped direct command.
#[test]
fn trace_levels_clamp_on_import_and_play_at_the_boundary() {
    for level in [i32::MIN, -1, 0, 128, 255, 256, 300, i32::MAX] {
        let clamped = level.clamp(0, 255);
        for (edge, name) in [("Lwr ", "lower"), ("Upr ", "upper")] {
            let descriptor = desc("null").with("Lvl ", D::Integer(level)).with("Edg ", enm("CntE", edge));
            let mut item = code_item("TrcC", descriptor);
            item.with_dialog = true;
            let mut imported = load_items(vec![item]);
            assert_eq!(imported.actions.list[0].steps[0].0, "filter.stylize.traceContour", "{level}/{name}");
            assert_eq!(imported.actions.list[0].steps[0].1["level"], clamped);
            let saved = saved_items(&mut imported);
            let expected = desc("null").with("Lvl ", D::Integer(clamped)).with("Edg ", enm("CntE", edge));
            assert_eq!(saved[0].descriptor.as_ref().unwrap().to_bytes(), expected.to_bytes());
            assert!(saved[0].with_dialog);
            for depth in [8, 16, 32] {
                let mut direct = Session::new();
                prepare_pattern(&mut imported, depth, "rgb");
                prepare_pattern(&mut direct, depth, "rgb");
                let played = imported.execute("actions.play", json!({"action":"A"})).unwrap();
                assert!(played.get("failed").is_none(), "{level}/{name}/{depth}: {played}");
                direct.execute("filter.stylize.traceContour", json!({"level":level.clamp(0,255),"edge":name})).unwrap();
                assert_eq!(pixels(&imported), pixels(&direct), "{level}/{name}/{depth}");
            }
        }
    }
    for level in [0, 128, 255] {
        assert!(export_item("filter.stylize.traceContour", &json!({"level":level}), "Group D").is_some(), "{level}");
    }
    for level in [json!(-1), json!(256), json!(255.5), json!(2147483648.0)] {
        assert!(export_item("filter.stylize.traceContour", &json!({"level":level}), "Group D").is_none(), "{level}");
    }
}

/// Every enumerator is explicit, and unknown values or enum types retain the original flagged step.
#[test]
fn enum_values_map_exactly_and_unknown_descriptors_stay_kept() {
    for (code, key, param, ty, values) in [
        ("Extr", "ExtT", "type", "ExtT", vec![("Blks", "blocks"), ("Pyrm", "pyramids")]),
        ("Extr", "ExtR", "depthMode", "ExtR", vec![("Rndm", "random"), ("LvlB", "levelBased")]),
        ("TrcC", "Edg ", "edge", "CntE", vec![("Lwr ", "lower"), ("Upr ", "upper")]),
        ("Wnd ", "WndM", "method", "WndM", vec![("Wnd ", "wind"), ("Blst", "blast"), ("Stgr", "stagger")]),
        ("Wnd ", "Drct", "direction", "Drct", vec![("Left", "fromRight"), ("Rght", "fromLeft")]),
        ("Tls ", "FlCl", "fill", "FlCl", vec![("FlBc", "background"), ("FlFr", "foreground"), ("FlIn", "inverse"), ("FlSm", "unaltered")]),
        (
            "Mztn",
            "MztT",
            "type",
            "MztT",
            vec![
                ("FnDt", "fineDots"),
                ("CrsD", "coarseDots"),
                ("GrnD", "grainyDots"),
                ("LngL", "longLines"),
                ("LngS", "longStrokes"),
                ("MdmD", "mediumDots"),
                ("MdmL", "mediumLines"),
                ("MdmS", "mediumStrokes"),
                ("ShSt", "shortStrokes"),
                ("ShrL", "shortLines"),
            ],
        ),
    ] {
        for (value, expected) in &values {
            let mut s = load_items(vec![code_item(code, desc("null").with(key, enm(ty, value)))]);
            assert_eq!(s.actions.list[0].steps[0].1[param], *expected, "{code}/{value}");
            assert_eq!(saved_items(&mut s)[0].descriptor.as_ref().unwrap().get(key), Some(&enm(ty, value)));
        }
        for value in [enm(ty, "????"), enm("Bad!", values[0].0), D::Boolean(true)] {
            let descriptor = desc("null").with(key, value);
            let mut item = code_item(code, descriptor.clone());
            item.enabled = false;
            item.with_dialog = true;
            item.dialog_options = 2;
            let mut s = load_items(vec![item]);
            assert_eq!(s.actions.list[0].steps[0].0, PHOTOSHOP_STEP, "{code}/{key}");
            assert!(s.actions.list[0].disabled.contains(&0));
            assert!(s.actions.list[0].dialogs.contains(&0));
            let saved = saved_items(&mut s);
            assert_eq!(saved[0].descriptor, Some(descriptor));
            assert!(!saved[0].enabled);
            assert!(saved[0].with_dialog);
        }
    }
    for (code, descriptor) in [
        ("Msc ", desc("null").with("ClSz", D::UnitFloat { unit: *b"#Prc", value: 3.0 })),
        ("Msc ", desc("null").with("ClSz", D::Double(f64::INFINITY))),
        ("Extr", desc("null").with("ExtS", D::Double(30.5))),
        ("Extr", desc("null").with("ExtS", D::Integer(30)).with("ExtS", D::Integer(31))),
        ("FndE", desc("null").with("unknown", D::Boolean(true))),
    ] {
        let mut s = load_items(vec![code_item(code, descriptor.clone())]);
        assert_eq!(s.actions.list[0].steps[0].0, PHOTOSHOP_STEP);
        assert_eq!(saved_items(&mut s)[0].descriptor, Some(descriptor));
    }
}

/// Native defaults stay native; seeds, frozen colors, targets and lossy values are not discarded.
#[test]
fn exports_materialize_native_defaults_and_reject_unrepresentable_settings() {
    for fixture in fixtures() {
        let item = export_item(fixture.id, &json!({}), "Group D").unwrap();
        let mut expected = fixture.default.clone();
        if fixture.code == "Msc " {
            expected = desc("null").with("ClSz", px(10.0));
        }
        if fixture.code == "TrcC" {
            expected = desc("null").with("Lvl ", D::Integer(128)).with("Edg ", enm("CntE", "Lwr "));
        }
        assert_eq!(item.descriptor, Some(expected), "{}", fixture.id);
        for (key, value) in [
            ("seed", json!(1)),
            ("foreground", json!("#000000")),
            ("background", json!([1, 1, 1, 1])),
            ("layer", json!(17)),
            ("target", json!("mask")),
            ("target", json!("channel:red")),
            ("unknown", json!(true)),
        ] {
            // Tiles and Pointillize drop the dialog's frozen colours (tested below).
            if matches!(key, "foreground" | "background") && matches!(fixture.code, "Tls " | "Pntl") {
                continue;
            }
            let mut params = fixture.defaults.clone();
            params[key] = value;
            let mut s = Session::new();
            s.actions.list.push(Action::new("A", vec![(fixture.id.into(), params)]));
            assert!(s.execute("actions.save", json!({"action":"A"})).is_err(), "{}/{key}", fixture.id);
        }
        let mut params = fixture.defaults.clone();
        params["target"] = json!("pixels");
        assert!(export_item(fixture.id, &params, "Group D").is_some());
    }
    for id in ["filter.stylize.extrude", "filter.stylize.wind", "filter.stylize.tiles", "filter.pixelate.mezzotint", "filter.pixelate.pointillize"] {
        assert!(export_item(id, &json!({"seed":0}), "Group D").is_some(), "{id}");
    }
    for (id, params) in [
        ("filter.stylize.extrude", json!({"size":30.5})),
        ("filter.stylize.wind", json!({"direction":"unknown"})),
        ("filter.stylize.tiles", json!({"fill":"unknown"})),
        ("filter.stylize.traceContour", json!({"edge":"unknown"})),
        ("filter.pixelate.colorHalftone", json!({"channel1":10001})),
        ("filter.pixelate.pointillize", json!({"cellSize":5.5})),
        ("filter.pixelate.mosaic", json!({"cellSize":201})),
        ("filter.pixelate.mezzotint", json!({"type":"unknown"})),
    ] {
        assert!(export_item(id, &params, "Group D").is_none(), "{id}/{params}");
    }
}

/// Imported actions match native pixels with either grouped or per-step Undo and Redo.
#[test]
fn imported_group_d_playback_matches_direct_commands_across_modes_and_depths() {
    let items: Vec<_> = fixtures()
        .into_iter()
        .map(|f| code_item(f.code, f.changed))
        .chain(["FndE", "Slrz", "Fct ", "Frgm"].map(|code| code_item(code, desc("null"))))
        .collect();
    for single_state in [true, false] {
        for (depth, mode) in [(8, "rgb"), (16, "rgb"), (32, "rgb"), (8, "gray"), (8, "cmyk"), (16, "lab")] {
            let mut imported = load_items(items.clone());
            imported.execute("actions.setHistory", json!({"singleState": single_state})).unwrap();
            imported.actions.list[0].disabled.insert(10);
            imported.actions.list[0].dialogs.insert(0);
            let steps = imported.actions.list[0].steps.clone();
            let mut direct = Session::new();
            for s in [&mut imported, &mut direct] {
                prepare_pattern(s, depth, mode);
            }
            let before = pixels(&imported);
            let before_doc = imported.active().unwrap().doc.clone();
            let history = imported.active().unwrap().history.past_len();
            let result = imported.execute("actions.play", json!({"action":"A"})).unwrap();
            assert!(result.get("failed").is_none(), "{depth}/{mode}: {result}");
            assert_eq!(result["ran"], 11);
            for (index, (id, params)) in steps.into_iter().enumerate() {
                if index != 10 {
                    direct.execute(&id, params).unwrap();
                }
            }
            assert_eq!(pixels(&imported), pixels(&direct), "{depth}/{mode}");
            let after_doc = imported.active().unwrap().doc.clone();
            if single_state {
                assert_eq!(imported.active().unwrap().history.past_len(), history + 1);
            } else {
                assert_eq!(imported.active().unwrap().history.past_len(), direct.active().unwrap().history.past_len());
            }
            let added = imported.active().unwrap().history.past_len() - history;
            assert!(added > 0);
            for _ in 0..added {
                imported.execute("edit.undo", json!({})).unwrap();
            }
            assert_eq!(pixels(&imported), before, "{depth}/{mode} undo");
            assert_eq!(imported.active().unwrap().doc.as_ref(), before_doc.as_ref(), "{depth}/{mode} undo");
            for _ in 0..added {
                imported.execute("edit.redo", json!({})).unwrap();
            }
            assert_eq!(imported.active().unwrap().doc.as_ref(), after_doc.as_ref(), "{depth}/{mode} redo");
            let saved = saved_items(&mut imported);
            assert!(!saved[10].enabled);
            assert!(saved[0].with_dialog);
        }
    }
}

/// A nonuniform image and nondefault tool colors expose differences hidden by flat inputs.
fn prepare_pattern(s: &mut Session, depth: u32, mode: &str) {
    s.execute("file.new", json!({"width":32,"height":24,"depth":depth,"mode":mode,"background":"#102030"})).unwrap();
    s.execute("select.rect", json!({"x":3,"y":2,"width":14,"height":11})).unwrap();
    s.execute("edit.fill", json!({"color":"#ee9922"})).unwrap();
    s.execute("select.deselect", json!({})).unwrap();
    s.execute("tools.setColors", json!({"foreground":"#e83a91","background":"#148fca"})).unwrap();
}

fn pixels(s: &Session) -> Vec<[f32; 4]> {
    photocraft_compose::render(&s.active().unwrap().doc, photocraft_geom::Rect::new(0, 0, 32, 24)).into_px()
}

/// Saving omitted native params preserves their actual command behavior, not Photoshop's defaults.
#[test]
fn native_and_measured_defaults_stay_distinct_through_playback() {
    use photocraft_algo::FilterParams;
    let mosaic = load_items(vec![code_item("Msc ", desc("null"))]);
    let measured = &mosaic.actions.list[0].steps[0].1;
    assert!(matches!(crate::filters::params_for("filter.pixelate.mosaic",&json!({})),Some(FilterParams::Mosaic{cell_size}) if cell_size==10.0));
    assert!(matches!(crate::filters::params_for("filter.pixelate.mosaic",measured),Some(FilterParams::Mosaic{cell_size}) if cell_size==2.0));
    let trace = load_items(vec![code_item("TrcC", desc("null"))]);
    let measured = &trace.actions.list[0].steps[0].1;
    assert!(matches!(crate::filters_ext::params_for("filter.stylize.traceContour", &json!({})), Some(FilterParams::TraceContour { upper: false, .. })));
    assert!(matches!(crate::filters_ext::params_for("filter.stylize.traceContour", measured), Some(FilterParams::TraceContour { upper: true, .. })));
    for id in ["filter.pixelate.mosaic", "filter.stylize.traceContour"] {
        let item = export_item(id, &json!({}), "Group D").unwrap();
        let mut imported = load_items(vec![item]);
        let mut native = Session::new();
        prepare_pattern(&mut imported, 8, "rgb");
        prepare_pattern(&mut native, 8, "rgb");
        native.execute(id, json!({})).unwrap();
        let result = imported.execute("actions.play", json!({"action":"A"})).unwrap();
        assert!(result.get("failed").is_none(), "{id}: {result}");
        assert_eq!(pixels(&imported), pixels(&native), "{id}");
    }
}

/// Dialog recordings of Tiles and Pointillize carry the current colours; the recorded event has none.
#[test]
fn tiles_and_pointillize_exports_drop_dialog_colours() {
    for (id, params) in
        [("filter.stylize.tiles", json!({"count":17,"maxOffset":23,"fill":"foreground"})), ("filter.pixelate.pointillize", json!({"cellSize":19}))]
    {
        let plain = export_item(id, &params, "Group D").unwrap().descriptor;
        let mut coloured = params.clone();
        coloured["foreground"] = json!([0.2, 0.4, 0.6, 1.0]);
        coloured["background"] = json!("#ffffff");
        assert_eq!(export_item(id, &coloured, "Group D").unwrap().descriptor, plain, "{id}");
        let mut s = Session::new();
        s.actions.list.push(Action::new("A", vec![(id.into(), coloured)]));
        assert!(s.execute("actions.save", json!({"action":"A"})).is_ok(), "{id}");
    }
}

/// The native Tiles command rounds its count, so a fractional dialog value exports as the whole count it plays.
#[test]
fn tiles_count_rounds_like_the_native_command() {
    for (count, expected) in [(17.4, 17), (17.5, 18), (1.2, 1), (98.6, 99)] {
        let d = export_item("filter.stylize.tiles", &json!({"count":count}), "Group D").unwrap().descriptor.unwrap();
        assert_eq!(d.get("TlNm"), Some(&D::Integer(expected)), "{count}");
    }
    for count in [0.4, 99.5] {
        assert!(export_item("filter.stylize.tiles", &json!({"count":count}), "Group D").is_none(), "{count}");
    }
    assert!(export_item("filter.stylize.tiles", &json!({"maxOffset":10.5}), "Group D").is_none());
}

/// Null params mean the native defaults, as for the other mapped events.
#[test]
fn null_params_export_native_defaults() {
    for id in ["filter.stylize.findEdges", "filter.stylize.tiles", "filter.stylize.traceContour"] {
        assert_eq!(
            export_item(id, &Value::Null, "Group D").map(|item| item.descriptor),
            export_item(id, &json!({}), "Group D").map(|item| item.descriptor),
            "{id}"
        );
        assert!(export_item(id, &Value::Null, "Group D").is_some(), "{id}");
    }
}
