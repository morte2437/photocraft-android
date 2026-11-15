use craft_psd::atn::{self, AtnAction, AtnEvent, AtnFile, AtnItem};
use craft_psd::descriptor::{Descriptor, Id, UnicodeString, Value as D};
use photocraft_doc::{Adjustment, LayerContent};
use photocraft_engine::{Session, atn_cmds};
use serde_json::json;

const CUBE: &[u8] = b"LUT_3D_SIZE 2\n1 1 1\n0 1 1\n1 0 1\n0 0 1\n1 1 0\n0 1 0\n1 0 0\n0 0 0\n";

/// An unreadable recorded filename ensures playback depends on embedded data alone.
fn descriptor(format: &str, bytes: &[u8]) -> Descriptor {
    let text = |s: &str| D::Text(UnicodeString::new_nul(s));
    let en = |ty: &str, value: &str| D::Enumerated { type_id: Id::new(ty), value: Id::new(value) };
    Descriptor::new("null")
        .with("lookupType", en("colorLookupType", "3DLUT"))
        .with("Nm  ", text("/missing/original/source"))
        .with("Dthr", D::Boolean(false))
        .with("profile", D::RawData(Vec::new()))
        .with("LUTFormat", en("LUTFormatType", format))
        .with("LUT3DFileData", D::RawData(bytes.to_vec()))
        .with("LUT3DFileName", text("embedded.bin"))
}

fn encoded(d: Descriptor) -> Vec<u8> {
    atn::write(&AtnFile {
        actions: vec![AtnAction {
            name: UnicodeString::new_nul("Lookup"),
            items: vec![AtnItem { event: AtnEvent::name("colorLookup"), descriptor: Some(d), ..Default::default() }],
            ..Default::default()
        }],
        ..Default::default()
    })
    .unwrap()
}

fn fixture(depth: u32) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width":3,"height":2,"depth":depth,"background":"#4080c0"})).unwrap();
    s
}

fn pixels(s: &Session) -> Vec<[f32; 4]> {
    photocraft_compose::flatten(&s.active().unwrap().doc).into_px()
}

fn saved(s: &Session) -> Vec<u8> {
    photocraft_format::save_to_bytes(&s.active().unwrap().doc, &Default::default()).unwrap()
}

/// Original invert tables exercise text CUBE/3DL and binary UTF-16 LOOK decoding.
fn samples() -> Vec<(&'static str, Vec<u8>)> {
    let mut three_dl = String::from("0 1023\n");
    for r in 0..2 {
        for g in 0..2 {
            for b in 0..2 {
                three_dl.push_str(&format!("{} {} {}\n", (1 - r) * 4095, (1 - g) * 4095, (1 - b) * 4095));
            }
        }
    }
    let hex: String =
        craft_cms::lutfile::LutFile::identity(2).unwrap().data.into_iter().flat_map(|v| (1.0 - v).to_le_bytes()).map(|b| format!("{b:02X}")).collect();
    let look = format!("<look><LUT><size>2</size><data>{hex}</data></LUT></look>");
    let le = [vec![0xff, 0xfe], look.encode_utf16().flat_map(u16::to_le_bytes).collect()].concat();
    let be = [vec![0xfe, 0xff], look.encode_utf16().flat_map(u16::to_be_bytes).collect()].concat();
    vec![("LUTFormatCUBE", CUBE.to_vec()), ("LUTFormat3DL", three_dl.into_bytes()), ("LUTFormatLOOK", le), ("LUTFormatLOOK", be)]
}

/// Compare every supported encoding and document depth with native adjustment playback and Undo.
#[test]
fn embedded_lookup_bytes_replay_invert_and_save_the_exact_descriptor() {
    for (format, bytes) in samples() {
        let d = descriptor(format, &bytes);
        let imported = atn_cmds::import(&encoded(d.clone())).unwrap();
        assert!(imported.unsupported.is_empty());
        let a = &imported.actions[0];
        assert_eq!(a.steps[0].0, "image.adjustments.colorLookup");
        let (out, skipped) = atn_cmds::export("Lookup", &[a], &[], false).unwrap();
        assert!(skipped.is_empty());
        assert_eq!(atn::parse(&out).unwrap().actions[0].items[0].descriptor, Some(d));
        for depth in [8, 16, 32] {
            let mut s = fixture(depth);
            let before = saved(&s);
            let original = pixels(&s);
            s.actions.list = imported.actions.clone();
            assert_eq!(s.execute("actions.play", json!({"action":"Lookup"})).unwrap()["ran"], 1);
            for (got, old) in pixels(&s).iter().zip(&original) {
                for c in 0..3 {
                    assert!((got[c] - (1.0 - old[c])).abs() < 2.0 / 255.0, "{format} {depth}: {got:?} {old:?}");
                }
            }
            s.execute("edit.undo", json!({})).unwrap();
            assert_eq!(saved(&s), before);
        }
    }
}

/// Unchanged text keeps its UTF-16 representation; editing it still writes the requested name.
#[test]
fn lookup_filename_and_outer_metadata_survive_unrelated_edits() {
    for name in [UnicodeString::new("embedded.bin"), UnicodeString(vec![65, 0, 0]), UnicodeString(vec![0xd800, 0])] {
        let mut d = descriptor("LUTFormatCUBE", CUBE);
        d.name = UnicodeString(vec![0xd800, 0]);
        d.class_id = Id::Str(b"null".to_vec());
        d.items.iter_mut().find(|(key, _)| key.is("LUT3DFileName")).unwrap().1 = D::Text(name);
        let imported = atn_cmds::import(&encoded(d.clone())).unwrap();
        assert!(imported.unsupported.is_empty());
        let mut action = imported.actions[0].clone();
        assert_eq!(action.steps[0].0, "image.adjustments.colorLookup");
        for edited in [false, true] {
            action.steps[0].1["dither"] = json!(edited);
            let (out, skipped) = atn_cmds::export("Lookup", &[&action], &[], false).unwrap();
            assert!(skipped.is_empty());
            let mut expected = d.clone();
            expected.items.iter_mut().find(|(key, _)| key.is("Dthr")).unwrap().1 = D::Boolean(edited);
            assert_eq!(atn::parse(&out).unwrap().actions[0].items[0].descriptor, Some(expected));
        }
        action.steps[0].1["fileName"] = json!("renamed.cube");
        let (out, _) = atn_cmds::export("Lookup", &[&action], &[], false).unwrap();
        let result = atn::parse(&out).unwrap().actions[0].items[0].descriptor.clone().unwrap();
        assert_eq!(result.name, d.name);
        assert_eq!(result.class_id, d.class_id);
        assert_eq!(result.get("LUT3DFileName"), Some(&D::Text(UnicodeString::new_nul("renamed.cube"))));
    }
}

/// Saving an adjustment must preserve the original payload rather than a decoded substitute.
#[test]
fn encoded_lookup_keeps_its_bytes_in_a_saved_adjustment() {
    for (format, bytes) in samples() {
        let imported = atn_cmds::import(&encoded(descriptor(format, &bytes))).unwrap();
        let mut p = imported.actions[0].steps[0].1.clone();
        p.as_object_mut().unwrap().remove("__atnColorLookup");
        let mut s = fixture(8);
        s.execute("layer.newAdjustmentLayer.colorLookup", p).unwrap();
        let psd = photocraft_io::export(&s.active().unwrap().doc, "psd", &Default::default()).unwrap().bytes;
        let doc = photocraft_io::import("lookup.psd", &psd).unwrap().document;
        let source = doc
            .walk()
            .into_iter()
            .find_map(|(_, _, layer)| match &layer.content {
                LayerContent::Adjustment(Adjustment::ColorLookup { source, .. }) => source.as_ref(),
                _ => None,
            })
            .unwrap();
        assert_eq!(source.bytes.as_slice(), bytes);
    }
}

/// An identity table isolates the nonlinear shaper through import, replay and saved adjustment bytes.
#[test]
fn embedded_three_dl_retains_its_nonlinear_shaper_through_action_playback() {
    // A two-node identity table with a four-knot shaper. Ignoring the first row
    // would leave the original pixels untouched; #9170 retains these midtones.
    let mut table = String::from("0 0 512 1023\n");
    for r in 0..2 {
        for g in 0..2 {
            for b in 0..2 {
                table.push_str(&format!("{} {} {}\n", r * 4095, g * 4095, b * 4095));
            }
        }
    }
    let d = descriptor("LUTFormat3DL", table.as_bytes());
    let imported = atn_cmds::import(&encoded(d.clone())).unwrap();
    assert!(imported.unsupported.is_empty());
    assert_eq!(imported.actions[0].steps[0].0, "image.adjustments.colorLookup");
    let (out, skipped) = atn_cmds::export("Lookup", &[&imported.actions[0]], &[], false).unwrap();
    assert!(skipped.is_empty());
    assert_eq!(atn::parse(&out).unwrap().actions[0].items[0].descriptor, Some(d));
    for depth in [8, 16, 32] {
        let mut s = fixture(depth);
        let original = pixels(&s);
        s.actions.list = imported.actions.clone();
        assert_eq!(s.execute("actions.play", json!({"action":"Lookup"})).unwrap()["ran"], 1);
        for (got, old) in pixels(&s).iter().zip(&original) {
            for c in 0..3 {
                let x = old[c].clamp(0.0, 1.0) * 3.0;
                let knot = (x.floor() as usize).min(2);
                let knots = [0.0, 0.0, 512.0 / 1023.0, 1.0];
                let expected = knots[knot] + (knots[knot + 1] - knots[knot]) * (x - knot as f32);
                assert!((got[c] - expected).abs() < 2.0 / 255.0, "{depth}-bit channel {c}: {got:?} from {old:?}");
            }
        }
    }
    // A shaper the parser refuses remains a complete step, rather than applying
    // just its identity table and reporting a successful Color Lookup.
    let bad = descriptor("LUTFormat3DL", table.replacen("0 0 512 1023", "0 NaN 512 1023", 1).as_bytes());
    let imported = atn_cmds::import(&encoded(bad.clone())).unwrap();
    assert_eq!(imported.actions[0].steps[0].0, atn_cmds::PHOTOSHOP_STEP);
    let (out, _) = atn_cmds::export("Lookup", &[&imported.actions[0]], &[], false).unwrap();
    assert_eq!(atn::parse(&out).unwrap().actions[0].items[0].descriptor, Some(bad));
}

/// Unsupported recording fields and native edits must not silently become partial lookup steps.
#[test]
fn unknown_lookup_forms_stay_whole_and_changed_native_steps_refuse_export() {
    let d = descriptor("LUTFormatCUBE", CUBE);
    let imported = atn_cmds::import(&encoded(d.clone())).unwrap();
    let mut edited = imported.actions[0].clone();
    edited.steps[0].1["dither"] = json!(true);
    let (out, _) = atn_cmds::export("Lookup", &[&edited], &[], false).unwrap();
    assert_eq!(atn::parse(&out).unwrap().actions[0].items[0].descriptor.as_ref().unwrap().get("Dthr"), Some(&D::Boolean(true)));
    for (key, value) in [("target", json!("mask")), ("lutFormat", json!("look")), ("interpolation", json!("tetrahedral"))] {
        let mut a = imported.actions[0].clone();
        a.steps[0].1[key] = value;
        assert!(atn_cmds::export("Lookup", &[&a], &[], false).is_err());
    }
    for (key, value) in [
        ("profile", D::RawData(vec![1, 2, 3])),
        ("lookupType", D::Enumerated { type_id: Id::new("colorLookupType"), value: Id::new("abstractProfile") }),
        ("LUTFormat", D::Enumerated { type_id: Id::new("OtherType"), value: Id::new("LUTFormatCUBE") }),
        ("Dthr", D::Integer(0)),
        ("extra", D::Boolean(true)),
    ] {
        let mut odd = d.clone();
        odd.items.retain(|(k, _)| !k.is(key));
        odd = odd.with(key, value);
        let a = atn_cmds::import(&encoded(odd.clone())).unwrap().actions.remove(0);
        assert_eq!(a.steps[0].0, atn_cmds::PHOTOSHOP_STEP);
        let (out, _) = atn_cmds::export("Lookup", &[&a], &[], false).unwrap();
        assert_eq!(atn::parse(&out).unwrap().actions[0].items[0].descriptor, Some(odd));
    }
    let mut s = fixture(8);
    let before = saved(&s);
    let history = s.active().unwrap().history.past_len();
    for p in [json!({"data":"!","dataEncoding":"base64"}), json!({"data":"x","dataEncoding":"unknown"}), json!({"data":"x","lutFormat":"unknown"})] {
        assert!(s.execute("image.adjustments.colorLookup", p).is_err());
        assert_eq!(saved(&s), before);
        assert_eq!(s.active().unwrap().history.past_len(), history);
    }
    s.execute("image.adjustments.colorLookup", json!({"data":std::str::from_utf8(CUBE).unwrap(),"fileName":"legacy.cube"})).unwrap();
    assert_ne!(saved(&s), before, "the existing text input remains supported");
}

/// Float playback must apply the recorded dither without hiding it in integer quantization.
#[test]
fn recorded_dither_changes_float_pixels_within_its_documented_bound() {
    let render = |dither| {
        let mut d = descriptor("LUTFormatCUBE", CUBE);
        d.items.iter_mut().find(|(key, _)| key.is("Dthr")).unwrap().1 = D::Boolean(dither);
        let imported = atn_cmds::import(&encoded(d)).unwrap();
        assert!(imported.unsupported.is_empty());
        let mut s = fixture(32);
        s.actions.list = imported.actions;
        assert_eq!(s.execute("actions.play", json!({"action":"Lookup"})).unwrap()["ran"], 1);
        pixels(&s)
    };
    let plain = render(false);
    // Opaque, unsaturated fixture pixels include nonzero ordered offsets.
    assert!(plain.iter().all(|p| p[3] == 1.0 && p[..3].iter().all(|v| (0.01..0.99).contains(v))));
    assert!((0..2).any(|y| (0..3).any(|x| photocraft_compose::adjust::bayer4(x, y) != 0.0)));
    let dithered = render(true);
    let mut changed = false;
    for (a, b) in dithered.iter().zip(&plain) {
        assert_eq!(a[3], b[3]);
        for c in 0..3 {
            let difference = (a[c] - b[c]).abs();
            assert!(difference < 0.5 / 255.0, "channel {c}: {difference}");
            changed |= difference > 0.0;
        }
    }
    assert!(changed, "Dthr=true must change at least one float channel");
}
