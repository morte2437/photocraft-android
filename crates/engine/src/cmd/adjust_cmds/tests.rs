use super::*;

const DEPTHS: [u64; 3] = [8, 16, 32];
const MODES: [&str; 4] = ["rgb", "gray", "cmyk", "lab"];

fn session(depth: u64, mode: &str) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 48, "height": 32, "depth": depth, "mode": mode})).unwrap();
    paint(&mut s, |x, y| [0.1 + x as f32 / 60.0, 0.2 + y as f32 / 50.0, 0.6 - x as f32 / 120.0, 1.0]);
    s
}

fn paint(s: &mut Session, f: impl Fn(i32, i32) -> [f32; 4]) {
    s.edit("setup", |doc, active| {
        let b = doc.bounds();
        let surf = doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap();
        let fmt = surf.format();
        let mut data = Vec::new();
        for y in b.y0..b.y1 {
            for x in b.x0..b.x1 {
                data.extend(photocraft_raster::from_rgba(&fmt, f(x, y)));
            }
        }
        surf.write_region(b, &data);
        Ok(())
    })
    .unwrap();
}

fn rgba(s: &Session, x: i32, y: i32) -> [f32; 4] {
    let d = s.active().unwrap();
    d.doc.layer(d.active_layer.unwrap()).unwrap().surface().unwrap().rgba(x, y)
}

fn changed(a: [f32; 4], b: [f32; 4]) -> f32 {
    (0..3).map(|c| (a[c] - b[c]).abs()).fold(0.0, f32::max)
}

/// Every pixel of the active layer, row by row.
fn pixels(s: &Session) -> Vec<[f32; 4]> {
    let b = doc(s).bounds();
    (b.y0..b.y1).flat_map(|y| (b.x0..b.x1).map(move |x| (x, y))).map(|(x, y)| rgba(s, x, y)).collect()
}

fn doc(s: &Session) -> &Document {
    &s.active().unwrap().doc
}

#[test]
fn destructive_adjustments_all_depths_and_modes_one_undo_step() {
    let cases: [(&str, Value); 4] = [
        ("image.adjustments.shadowsHighlights", json!({"shadowAmount": 80, "blackClip": 0, "whiteClip": 0})),
        ("image.adjustments.replaceColor", json!({"color": "#4d66cc", "fuzziness": 200, "hue": 90, "saturation": 30})),
        ("image.adjustments.matchColor", json!({"neutralize": true, "intensity": 50})),
        ("image.adjustments.hdrToning", json!({"strength": 2, "exposure": 0.5})),
    ];
    for (id, params) in &cases {
        for depth in DEPTHS {
            for mode in MODES {
                let mut s = session(depth, mode);
                let before = rgba(&s, 3, 3);
                let steps = s.active().unwrap().history.past_len();
                s.execute(id, params.clone()).unwrap_or_else(|e| panic!("{id} {depth} {mode}: {e}"));
                let after = rgba(&s, 3, 3);
                // Gray documents can't show hue changes from Replace Color at a gray pixel.
                if !(mode == "gray" && (id.ends_with("replaceColor") || id.ends_with("matchColor"))) {
                    assert!(changed(before, after) > 1e-3, "{id} {depth} {mode}: {before:?} → {after:?}");
                }
                assert_eq!(s.active().unwrap().history.past_len(), steps + 1, "{id}: one history step");
                s.execute("edit.undo", json!({})).unwrap();
                assert!(changed(before, rgba(&s, 3, 3)) < 1e-6, "{id} undo");
            }
        }
    }
}

#[test]
fn destructive_adjustments_respect_selection() {
    let mut s = session(8, "rgb");
    s.execute("select.all", json!({})).ok();
    s.edit("sel", |doc, _| {
        let mut sel = Surface::new(photocraft_color::PixelFormat::GRAY8);
        sel.write_region(Rect::new(0, 0, 10, 32), &vec![1.0; 320]);
        doc.selection = Some(sel);
        Ok(())
    })
    .unwrap();
    let outside = rgba(&s, 30, 5);
    let inside = rgba(&s, 5, 5);
    s.execute("image.adjustments.shadowsHighlights", json!({"shadowAmount": 100, "blackClip": 0, "whiteClip": 0})).unwrap();
    assert!(changed(inside, rgba(&s, 5, 5)) > 1e-3);
    assert_eq!(outside, rgba(&s, 30, 5));
}

#[test]
fn match_color_from_another_document() {
    let mut s = session(8, "rgb");
    s.execute("file.new", json!({"width": 16, "height": 16, "background": "#e05010"})).unwrap();
    // The target is document 0.
    s.execute("document.activate", json!({"document": 0})).unwrap();
    let before = rgba(&s, 10, 10);
    s.execute("image.adjustments.matchColor", json!({"source": 1})).unwrap();
    let after = rgba(&s, 10, 10);
    assert!(after[0] > before[0] && after[2] < before[2], "{before:?} → {after:?}");
    // Fade 100 is a no-op; a bad source index is an error.
    s.execute("edit.undo", json!({})).unwrap();
    s.execute("image.adjustments.matchColor", json!({"source": 1, "fade": 100})).unwrap();
    assert!(changed(before, rgba(&s, 10, 10)) < 2.0 / 255.0);
    assert!(s.execute("image.adjustments.matchColor", json!({"source": 7})).is_err());
    // Source "None" with default options is a no-op.
    let now = rgba(&s, 10, 10);
    s.execute("image.adjustments.matchColor", json!({})).unwrap();
    assert!(changed(now, rgba(&s, 10, 10)) < 2.0 / 255.0);
}

#[test]
fn hdr_toning_flattens() {
    let mut s = session(16, "rgb");
    s.execute("layer.new.layer", json!({})).unwrap();
    assert_eq!(doc(&s).layers.len(), 2);
    s.execute("image.adjustments.hdrToning", json!({})).unwrap();
    assert_eq!(doc(&s).layers.len(), 1);
    assert_eq!(doc(&s).layers[0].name, "Background");
}

#[test]
fn disabled_without_pixels() {
    let mut s = session(8, "rgb");
    s.execute("layer.newAdjustmentLayer.invert", json!({})).unwrap();
    for id in ["image.adjustments.shadowsHighlights", "image.adjustments.replaceColor", "image.adjustments.matchColor"] {
        assert!(!s.is_enabled(id), "{id}");
    }
    assert!(s.is_enabled("image.adjustments.hdrToning"));
    assert!(!Session::new().is_enabled("image.adjustments.hdrToning"));
}

#[test]
fn selective_color_layer_and_destructive() {
    for depth in DEPTHS {
        for mode in ["rgb", "cmyk", "lab"] {
            let mut s = session(depth, mode);
            paint(&mut s, |_, _| [0.9, 0.1, 0.1, 1.0]);
            let before = rgba(&s, 1, 1);
            // Reds: −100 % cyan (absolute) makes red redder; +100 % black darkens.
            s.execute("image.adjustments.selectiveColor", json!({"method": "absolute", "reds": [0, 0, 0, 60]})).unwrap();
            let after = rgba(&s, 1, 1);
            assert!(after[0] < before[0] - 0.05, "{depth} {mode}: {before:?} → {after:?}");
        }
    }
    let mut s = session(8, "rgb");
    let r = s.execute("layer.newAdjustmentLayer.selectiveColor", json!({"colors": "blues", "yellow": 40})).unwrap();
    let id = photocraft_doc::LayerId(r["layer"].as_u64().unwrap());
    let get = |s: &Session| match &doc(s).layer(id).unwrap().content {
        LayerContent::Adjustment(Adjustment::SelectiveColor { relative, adjustments }) => (*relative, *adjustments),
        other => panic!("{other:?}"),
    };
    assert_eq!(get(&s).1[4], [0.0, 0.0, 40.0, 0.0]);
    // Properties edits merge one range at a time.
    s.execute("layer.setAdjustment", json!({"layer": id.0, "colors": "neutrals", "black": -20, "method": "absolute"})).unwrap();
    let (rel, a) = get(&s);
    assert!(!rel);
    assert_eq!(a[4][2], 40.0);
    assert_eq!(a[7][3], -20.0);
    // No parameters: reset.
    s.execute("layer.setAdjustment", json!({"layer": id.0})).unwrap();
    assert_eq!(get(&s), (true, [[0.0; 4]; 9]));
}

#[test]
fn color_lookup_builtin_data_and_errors() {
    let mut s = session(8, "rgb");
    let r = s.execute("layer.newAdjustmentLayer.colorLookup", json!({"lut": "dayForNight"})).unwrap();
    let id = photocraft_doc::LayerId(r["layer"].as_u64().unwrap());
    let px = photocraft_compose::render(doc(&s), Rect::from_xywh(40, 30, 1, 1)).px[0];
    assert!(px[2] > px[0], "night is blue: {px:?}");
    // Switching interpolation keeps the table.
    s.execute("layer.setAdjustment", json!({"layer": id.0, "interpolation": "tetrahedral", "dither": true})).unwrap();
    match &doc(&s).layer(id).unwrap().content {
        LayerContent::Adjustment(Adjustment::ColorLookup { lut: Some(_), size: 33, tetrahedral: true, dither: true, name, .. }) => {
            assert_eq!(name, "Day for Night")
        }
        other => panic!("{other:?}"),
    }
    // Embedded .cube text (an inverting LUT) applied destructively, at every depth.
    let mut cube = String::from("LUT_3D_SIZE 2\n");
    for b in 0..2 {
        for g in 0..2 {
            for r in 0..2 {
                cube.push_str(&format!("{} {} {}\n", 1 - r, 1 - g, 1 - b));
            }
        }
    }
    for depth in DEPTHS {
        let mut s = session(depth, "rgb");
        let before = rgba(&s, 5, 5);
        s.execute("image.adjustments.colorLookup", json!({"data": cube, "fileName": "invert.cube"})).unwrap();
        let after = rgba(&s, 5, 5);
        for c in 0..3 {
            assert!((after[c] - (1.0 - before[c])).abs() < 2.0 / 255.0, "{depth}: {before:?} {after:?}");
        }
    }
    assert!(s.execute("image.adjustments.colorLookup", json!({"lut": "nope"})).is_err());
    // A custom input domain is rejected with the reason instead of silently shifting the look.
    // (A fresh pixel layer: this `s` still has the adjustment layer active, and the command is
    // disabled there - the domain rejection itself needs a pixel layer to reach.)
    s.execute("layer.new.layer", json!({})).unwrap();
    let domained = "TITLE \"d\"
DOMAIN_MIN 0.1 0.1 0.1
DOMAIN_MAX 0.9 0.9 0.9
LUT_3D_SIZE 2
0 0 0
0 0 1
0 1 0
0 1 1
1 0 0
1 0 1
1 1 0
1 1 1
";
    let e = s.execute("image.adjustments.colorLookup", json!({"data": domained, "fileName": "log.cube"})).unwrap_err();
    assert!(e.to_string().contains("DOMAIN_MIN"), "{e}");
    assert!(s.execute("image.adjustments.colorLookup", json!({"data": "LUT_3D_SIZE 3\n0 0 0\n"})).is_err());
    assert!(s.execute("layer.newAdjustmentLayer.colorLookup", json!({"file": "/nonexistent/x.cube"})).is_err());
    let looks = s.execute("image.adjustments.colorLookup.list", json!({})).unwrap();
    assert_eq!(looks.as_array().unwrap().len(), craft_cms::lutfile::BUILTIN.len());
}

#[test]
fn color_lookup_mesh_3dl_honours_the_header() {
    // #9078 / #9073: a Lustre .3dl whose `Mesh <inBits> <outBits>` line declares the cube edge
    // (2^inBits + 1) and the output range (up to 2^outBits - 1). The three-value shaper row used
    // to be counted as a 28th node ("28 rows is not a cube"), and 10-bit codes under a 12-bit
    // declaration divided by the inferred 1023 instead of the declared 4095.
    let mut lut = String::from("3DMESH\nMesh 1 12\n0 512 1023\n");
    for r in 0..3 {
        for g in 0..3 {
            for b in 0..3 {
                let v = [r, g, b].map(|c| (4095.0 - 4095.0 * c as f64 / 2.0).round() as i32);
                lut.push_str(&format!("{} {} {}\n", v[0], v[1], v[2]));
            }
        }
    }
    let mut s = session(8, "rgb");
    paint(&mut s, |_, _| [64.0 / 255.0, 128.0 / 255.0, 192.0 / 255.0, 1.0]);
    s.execute("image.adjustments.colorLookup", json!({"data": lut, "fileName": "inverse.3dl", "dither": false})).unwrap();
    // Photoshop 27.11 renders (64,128,192) as (191,127,63) through this file.
    let after = rgba(&s, 5, 5);
    for (c, want) in [(0, 191.0), (1, 127.0), (2, 63.0)] {
        assert!((after[c] * 255.0 - want).abs() <= 1.0, "channel {c}: {after:?}");
    }
}

/// The kind's parameters, read back from the model (Color Lookup with its table name and size).
#[test]
fn color_lookup_file_round_trips_psd_in_its_own_format() {
    use photocraft_doc::adjust::{LutFormat, LutSource};
    // #6544: a .3dl / .look file is kept as loaded and embedded unchanged in a PSD.
    let mut threedl = String::from("0 1023\n");
    for r in 0..2 {
        for g in 0..2 {
            for b in 0..2 {
                threedl.push_str(&format!("{} {} {}\n", (1 - r) * 4095, (1 - g) * 4095, (1 - b) * 4095));
            }
        }
    }
    let hex: String = craft_cms::lutfile::LutFile::identity(2).unwrap().data.iter().flat_map(|v| v.to_le_bytes()).map(|b| format!("{b:02X}")).collect();
    let look = format!("<look><LUT><size>2</size><data>{hex}</data></LUT></look>");
    for (text, file_name, format, psd_format) in
        [(threedl.as_str(), "Invert.3dl", LutFormat::ThreeDl, "LUTFormat3DL"), (look.as_str(), "Grade.look", LutFormat::Look, "LUTFormatLOOK")]
    {
        let mut s = session(8, "rgb");
        let r = s.execute("layer.newAdjustmentLayer.colorLookup", json!({"data": text, "fileName": file_name})).unwrap();
        let id = photocraft_doc::LayerId(r["layer"].as_u64().unwrap());
        let want = LutSource { format, bytes: std::sync::Arc::new(text.as_bytes().to_vec()) };
        let source_of = |s: &Session| match adjustment_of(s, id) {
            Adjustment::ColorLookup { source, .. } => source,
            other => panic!("{other:?}"),
        };
        assert_eq!(source_of(&s), Some(want.clone()), "{file_name}");
        // Settings that don't touch the table keep the file.
        s.execute("layer.setAdjustment", json!({"layer": id.0, "interpolation": "tetrahedral", "dither": true})).unwrap();
        assert_eq!(source_of(&s), Some(want.clone()), "{file_name}");
        // Saved as PSD and reopened: same format tag, same bytes, same name.
        let psd = photocraft_io::export(doc(&s), "psd", &photocraft_io::ExportOptions::default()).unwrap().bytes;
        let back = photocraft_io::import("x.psd", &psd).unwrap().document;
        let layer = back.walk().into_iter().map(|(_, _, l)| l).find(|l| matches!(l.content, LayerContent::Adjustment(Adjustment::ColorLookup { .. }))).unwrap();
        let LayerContent::Adjustment(Adjustment::ColorLookup { name, source, .. }) = &layer.content else { panic!("not a Color Lookup") };
        assert_eq!((name.as_str(), source.as_ref()), (file_name, Some(&want)), "{psd_format}");
        // Choosing a built-in look replaces the table, so the file goes too.
        s.execute("layer.setAdjustment", json!({"layer": id.0, "lut": "dayForNight"})).unwrap();
        assert_eq!(source_of(&s), None);
    }
    // "none" drops it as well.
    let mut s = session(8, "rgb");
    let r = s.execute("layer.newAdjustmentLayer.colorLookup", json!({"data": threedl, "fileName": "Invert.3dl"})).unwrap();
    let id = photocraft_doc::LayerId(r["layer"].as_u64().unwrap());
    s.execute("layer.setAdjustment", json!({"layer": id.0, "lut": "none"})).unwrap();
    assert!(matches!(adjustment_of(&s, id), Adjustment::ColorLookup { source: None, lut: None, .. }));
}

fn adjustment_of(s: &Session, id: photocraft_doc::LayerId) -> Adjustment {
    match &doc(s).layer(id).unwrap().content {
        LayerContent::Adjustment(a) => a.clone(),
        other => panic!("{other:?}"),
    }
}

#[test]
fn selective_color_and_color_lookup_reject_wrong_typed_params() {
    let cube = "LUT_3D_SIZE 2\n0 0 0\n1 0 0\n0 1 0\n1 1 0\n0 0 1\n1 0 1\n0 1 1\n1 1 1\n";
    // (kind, params, the key the error must name)
    let cases: Vec<(&str, Value, &str)> = vec![
        ("selectiveColor", json!({"method": 5}), "method"),
        ("selectiveColor", json!({"method": "sideways"}), "method"),
        ("selectiveColor", json!({"relative": "yes"}), "relative"),
        ("selectiveColor", json!({"reds": "not-an-array"}), "reds"),
        ("selectiveColor", json!({"blues": [10, 20]}), "blues"),
        ("selectiveColor", json!({"neutrals": [10, "20", 0, 0]}), "neutrals"),
        ("selectiveColor", json!({"colors": 3, "cyan": 50}), "colors"),
        ("selectiveColor", json!({"colors": "purples", "cyan": 50}), "purples"),
        ("selectiveColor", json!({"colors": "reds", "cyan": "50"}), "cyan"),
        ("selectiveColor", json!({"colors": "blacks", "black": [1]}), "black"),
        ("colorLookup", json!({"lut": 42}), "lut"),
        ("colorLookup", json!({"interpolation": 5}), "interpolation"),
        ("colorLookup", json!({"interpolation": "bicubic"}), "interpolation"),
        ("colorLookup", json!({"tetrahedral": 1}), "tetrahedral"),
        ("colorLookup", json!({"dither": "yes"}), "dither"),
        ("colorLookup", json!({"data": 42}), "data"),
        ("colorLookup", json!({"data": cube, "fileName": 7}), "fileName"),
        ("colorLookup", json!({"file": 42}), "file"),
    ];
    let named = |r: Result<Value>, key: &str, what: &str| match r {
        Err(e @ EngineError::BadParams { .. }) => assert!(e.to_string().contains(key), "{what}: {e}"),
        other => panic!("{what}: expected BadParams naming `{key}`, got {other:?}"),
    };
    for depth in DEPTHS {
        for mode in ["rgb", "cmyk", "lab"] {
            for (kind, p, key) in &cases {
                let what = format!("{kind} {p} {depth} {mode}");
                let mut s = session(depth, mode);
                let (layers, steps, px) = (doc(&s).layers.len(), s.active().unwrap().history.past_len(), rgba(&s, 3, 3));
                named(s.execute(&format!("image.adjustments.{kind}"), p.clone()), key, &what);
                named(s.execute(&format!("layer.newAdjustmentLayer.{kind}"), p.clone()), key, &what);
                assert_eq!(doc(&s).layers.len(), layers, "{what}: no layer added");
                assert_eq!(s.active().unwrap().history.past_len(), steps, "{what}: no history step");
                assert_eq!(rgba(&s, 3, 3), px, "{what}: pixels unchanged");
            }
        }
    }
    // An update with a wrong-typed key fails and keeps every existing value.
    let mut s = session(8, "rgb");
    let made = [
        ("selectiveColor", json!({"method": "absolute", "reds": [10, -20, 30, -40], "colors": "blues", "yellow": 40})),
        ("colorLookup", json!({"lut": "warm", "interpolation": "tetrahedral", "dither": true})),
    ];
    for (kind, p) in made {
        let id = photocraft_doc::LayerId(s.execute(&format!("layer.newAdjustmentLayer.{kind}"), p).unwrap()["layer"].as_u64().unwrap());
        let before = adjustment_of(&s, id);
        for (k, p, key) in cases.iter().filter(|c| c.0 == kind) {
            let mut p = p.clone();
            p["layer"] = json!(id.0);
            named(s.execute("layer.setAdjustment", p.clone()), key, &format!("setAdjustment {k} {p}"));
            assert_eq!(adjustment_of(&s, id), before, "{p}");
        }
        // Null still means "not given": the update keeps everything.
        let nulls = json!({"layer": id.0, "method": null, "reds": null, "colors": null, "lut": null, "interpolation": null, "dither": null});
        s.execute("layer.setAdjustment", nulls).unwrap();
        assert_eq!(adjustment_of(&s, id), before, "{kind}: nulls");
    }
    // Well-typed values reach the model.
    match adjustment_of(&s, doc(&s).layers[1].id) {
        Adjustment::SelectiveColor { relative: false, adjustments } => {
            assert_eq!(adjustments[0], [10.0, -20.0, 30.0, -40.0]);
            assert_eq!(adjustments[4], [0.0, 0.0, 40.0, 0.0]);
        }
        other => panic!("{other:?}"),
    }
    assert!(matches!(adjustment_of(&s, doc(&s).layers[2].id), Adjustment::ColorLookup { lut: Some(_), tetrahedral: true, dither: true, .. }));
}

#[test]
fn new_kinds_round_trip_through_psd_and_pcraft() {
    let mut s = session(8, "rgb");
    s.execute("layer.newAdjustmentLayer.selectiveColor", json!({"reds": [10, -20, 30, -40], "method": "absolute"})).unwrap();
    s.execute("layer.newAdjustmentLayer.colorLookup", json!({"lut": "warm", "dither": true})).unwrap();
    let d = doc(&s).clone();
    let bytes = photocraft_io::export(&d, "x.psd", &Default::default()).unwrap().bytes;
    let back = photocraft_io::import("x.psd", &bytes).unwrap().document;
    let kinds: Vec<&str> = back
        .layers
        .iter()
        .filter_map(|l| match &l.content {
            LayerContent::Adjustment(a) => Some(crate::commands::adjustment_kind(a)),
            _ => None,
        })
        .collect();
    assert_eq!(kinds, ["selectiveColor", "colorLookup"]);
    let pc = photocraft_format::save_to_bytes(&d, &Default::default()).unwrap();
    let again = photocraft_format::load_from_bytes(&pc).unwrap();
    for (a, b) in d.layers.iter().zip(&again.layers) {
        assert_eq!(a.content, b.content);
    }
}

fn mask_at(s: &Session, x: i32, y: i32) -> f32 {
    let d = s.active().unwrap();
    d.doc.layer(d.active_layer.unwrap()).unwrap().mask.as_ref().unwrap().surface.sample_channel(x, y, 0)
}

/// #2456: Edit > Fill (and the fill keys) fill the targeted mask of an adjustment layer.
#[test]
fn fill_works_on_an_adjustment_layers_mask() {
    let mut s = session(8, "rgb");
    s.execute("layer.newAdjustmentLayer.curves", json!({})).unwrap();
    s.execute("layer.layerMask.revealAll", json!({})).unwrap();
    let mask = json!({"target": "mask"});
    assert!(!s.is_enabled("edit.fill"), "no pixels to fill without the mask target");
    assert!(s.is_enabled_with("edit.fill", &mask));
    assert!(s.is_enabled_with("edit.fillForeground", &mask));
    s.execute("edit.fill", json!({"contents": "black", "target": "mask"})).unwrap();
    assert_eq!(mask_at(&s, 3, 3), 0.0, "the mask is filled black");
    s.execute("edit.fill", json!({"contents": "white", "target": "mask"})).unwrap();
    assert_eq!(mask_at(&s, 3, 3), 1.0);
}

/// #780: with the layer mask targeted, Invert (⌘I) inverts the mask, past the canvas too, and
/// leaves the layer's pixels alone; one history step.
#[test]
fn invert_with_the_mask_targeted_inverts_the_mask() {
    for depth in DEPTHS {
        let mut s = session(depth, "rgb");
        s.execute("select.rect", json!({"x": 24, "y": 0, "width": 24, "height": 32})).unwrap();
        s.execute("layer.layerMask.revealSelection", json!({})).unwrap();
        let (pixels, off_canvas) = (rgba(&s, 3, 3), mask_at(&s, -500, 3));
        let steps = s.active().unwrap().history.past_len();
        s.execute("image.adjustments.invert", json!({"target": "mask"})).unwrap();
        assert_eq!((mask_at(&s, 3, 3), mask_at(&s, 30, 3)), (1.0, 0.0), "depth {depth}");
        assert_eq!(mask_at(&s, -500, 3), 1.0 - off_canvas, "the mask's untouched area inverts too");
        assert_eq!(rgba(&s, 3, 3), pixels, "the layer is untouched");
        assert_eq!(s.active().unwrap().history.past_len(), steps + 1);
        // Through a selection, only the selected part.
        s.execute("select.rect", json!({"x": 0, "y": 0, "width": 10, "height": 32})).unwrap();
        s.execute("image.adjustments.invert", json!({"target": "mask"})).unwrap();
        assert_eq!((mask_at(&s, 3, 3), mask_at(&s, 12, 3), mask_at(&s, 30, 3)), (0.0, 1.0, 0.0));
        for _ in 0..3 {
            s.undo(); // invert, select, invert
        }
        assert_eq!((mask_at(&s, 3, 3), mask_at(&s, 30, 3), mask_at(&s, -500, 3)), (0.0, 1.0, off_canvas));
    }
}

/// Photoshop: a mask made from the selection (Reveal Selection, the Layers panel's mask button)
/// takes the selection with it, as Hide Selection does. So ⌘I on the new mask right away inverts
/// all of it; with the selection kept it inverted only the inside, leaving the mask all black.
/// Undo brings the selection back with the mask gone, in one step.
#[test]
fn reveal_selection_drops_the_selection_so_invert_flips_the_whole_mask() {
    for depth in DEPTHS {
        let mut s = session(depth, "rgb");
        s.execute("select.rect", json!({"x": 24, "y": 0, "width": 24, "height": 32})).unwrap();
        let selection = s.active().unwrap().doc.selection.clone();
        let steps = s.active().unwrap().history.past_len();
        s.execute("layer.layerMask.revealSelection", json!({})).unwrap();
        assert!(s.active().unwrap().doc.selection.is_none(), "depth {depth}: the selection became the mask");
        assert_eq!(s.active().unwrap().history.past_len(), steps + 1, "one step");
        assert_eq!((mask_at(&s, 3, 3), mask_at(&s, 30, 3)), (0.0, 1.0));
        s.execute("image.adjustments.invert", json!({"target": "mask"})).unwrap();
        assert_eq!((mask_at(&s, 3, 3), mask_at(&s, 30, 3)), (1.0, 0.0), "depth {depth}: inside and outside both flip");
        s.undo();
        s.undo();
        let st = s.active().unwrap();
        assert_eq!(st.doc.selection, selection, "undo restores the selection");
        assert!(st.doc.layer(st.active_layer.unwrap()).unwrap().mask.is_none(), "and removes the mask");
    }
}

// ---------- Image › Adjustments as smart filters (FILE-215-4) ----------

/// A painted pixel layer converted to a smart object, over the painted Background.
fn smart_session() -> Session {
    let mut s = session(8, "rgb");
    s.execute("layer.new.layer", json!({})).unwrap();
    paint(&mut s, |x, y| [0.1 + x as f32 / 60.0, 0.2 + y as f32 / 50.0, 0.6 - x as f32 / 120.0, 1.0]);
    s.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
    s
}

fn flatten(s: &Session) -> Vec<[f32; 4]> {
    photocraft_compose::flatten(doc(s)).into_px()
}

fn max_diff(a: &[[f32; 4]], b: &[[f32; 4]]) -> f32 {
    a.iter().zip(b).flat_map(|(p, q)| (0..4).map(move |c| (p[c] - q[c]).abs())).fold(0.0, f32::max)
}

fn smart_filters_of(s: &Session) -> Vec<photocraft_doc::SmartFilter> {
    let d = s.active().unwrap();
    match &d.doc.layer(d.active_layer.unwrap()).unwrap().content {
        LayerContent::Smart(sm) => sm.smart_filters.clone(),
        other => panic!("not a smart object: {}", other.kind_name()),
    }
}

/// FILE-215-4: Image › Adjustments on a smart object record a smart filter (Photoshop) instead
/// of baking into a copy of the pixels: it renders exactly what the destructive command would
/// bake in, and can be hidden, re-edited and undone, each in one history step.
#[test]
fn adjustments_on_a_smart_object_record_a_smart_filter() {
    let mut s = smart_session();
    let original = flatten(&s);
    // The destructive result on the same pixels: the smart filter must match it exactly.
    let mut plain = session(8, "rgb");
    plain.execute("image.adjustments.invert", json!({})).unwrap();
    let baked = flatten(&plain);
    assert!(max_diff(&original, &baked) > 0.5, "the control changes the pixels");
    assert!(s.is_enabled("image.adjustments.invert"), "an adjustment runs on a smart object");
    let steps = s.active().unwrap().history.past_len();
    s.execute("image.adjustments.invert", json!({})).unwrap();
    assert_eq!(s.active().unwrap().history.past_len(), steps + 1, "one history step");
    let sf = smart_filters_of(&s);
    assert_eq!(sf.len(), 1);
    assert_eq!(sf[0].command, "image.adjustments.invert");
    assert!(sf[0].visible && sf[0].opacity == 1.0 && sf[0].blend == photocraft_color::BlendMode::Normal);
    assert!(sf[0].params.get("__kind").is_none(), "the engine's private key is not recorded");
    assert_eq!(max_diff(&flatten(&s), &baked), 0.0, "the smart filter renders the destructive result");
    // Hiding the filter shows the original exactly.
    s.execute("layer.smartFilter.setVisible", json!({"index": 0})).unwrap();
    assert_eq!(flatten(&s), original);
    s.undo();
    assert_eq!(max_diff(&flatten(&s), &baked), 0.0, "undo restores the filtered pixels");
    // Re-editing the recorded params re-renders; the same params give the same pixels.
    s.execute("layer.smartFilter.setParams", json!({"index": 0, "params": {}})).unwrap();
    assert_eq!(max_diff(&flatten(&s), &baked), 0.0, "the same params render the same pixels");
    // The setParams step, then the filter itself, undo away.
    s.undo();
    s.undo();
    assert!(smart_filters_of(&s).is_empty(), "the filter is gone");
    assert_eq!(flatten(&s), original);
}

/// A selection becomes the smart filter's mask, and the adjustment with its own command
/// (Shadows/Highlights) records a filter too.
#[test]
fn adjustment_smart_filters_take_a_selection_as_their_mask() {
    let mut s = smart_session();
    let original = flatten(&s);
    s.execute("select.rect", json!({"x": 0, "y": 0, "width": 24, "height": 32})).unwrap();
    s.execute("image.adjustments.shadowsHighlights", json!({"shadowAmount": 100, "blackClip": 0, "whiteClip": 0})).unwrap();
    let d = s.active().unwrap();
    let LayerContent::Smart(sm) = &d.doc.layer(d.active_layer.unwrap()).unwrap().content else { panic!("not smart") };
    assert_eq!(sm.smart_filters.len(), 1);
    assert_eq!(sm.smart_filters[0].command, "image.adjustments.shadowsHighlights");
    assert!(sm.filter_mask.is_some(), "the selection became the filter mask");
    let now = flatten(&s);
    let at = |px: &[[f32; 4]], x: i32, y: i32| px[(y * 48 + x) as usize];
    assert!(changed(at(&original, 5, 16), at(&now, 5, 16)) > 1e-3, "inside the selection the shadows lifted");
    assert_eq!(at(&now, 40, 16), at(&original, 40, 16), "outside the mask the composite is unchanged");
}

/// #5655: as for `filter.*` smart filters, a pixel or full lock refuses an adjustment smart
/// filter, leaving the layer and the history as they were.
#[test]
fn locked_smart_objects_refuse_adjustment_smart_filters() {
    let adjustments =
        [("image.adjustments.invert", json!({})), ("image.adjustments.shadowsHighlights", json!({"shadowAmount": 100, "blackClip": 0, "whiteClip": 0}))];
    for lock in [json!({"all": true}), json!({"pixels": true})] {
        for (cmd, p) in &adjustments {
            let mut s = smart_session();
            s.execute("layer.lockLayers", lock.clone()).unwrap();
            let (steps, flat) = (s.active().unwrap().history.past_len(), flatten(&s));
            let r = s.execute(cmd, p.clone());
            assert!(r.is_err(), "{cmd} added a smart filter to a smart object locked with {lock}: {r:?}");
            assert!(smart_filters_of(&s).is_empty(), "{cmd} {lock}: nothing recorded");
            assert_eq!(s.active().unwrap().history.past_len(), steps, "{cmd} {lock}: no history step");
            assert_eq!(flatten(&s), flat, "{cmd} {lock}: pixels untouched");
        }
    }
}

/// A wrong-typed value errors before anything is recorded (never a panic), and the adjustments
/// that can't re-render from a source (Match Color) stays off a smart object.
#[test]
fn a_bad_adjustment_param_on_a_smart_object_is_an_error() {
    let mut s = smart_session();
    let (filters, steps, flat) = (smart_filters_of(&s).len(), s.active().unwrap().history.past_len(), flatten(&s));
    let e = s.execute("image.adjustments.colorLookup", json!({"lut": 42})).unwrap_err();
    assert!(e.to_string().contains("lut"), "{e}");
    assert_eq!(smart_filters_of(&s).len(), filters, "nothing recorded");
    assert_eq!(s.active().unwrap().history.past_len(), steps, "no history step");
    assert_eq!(flatten(&s), flat, "pixels untouched");
    assert!(!s.is_enabled("image.adjustments.matchColor"), "only adjustments that re-render become smart filters");
}

/// Add/Subtract from Sample edit through Color Range's matcher, which the one-colour smart filter
/// cannot replay: refused on a smart object instead of recording a filter that drops the samples.
#[test]
fn multi_sample_replace_color_is_refused_on_a_smart_object() {
    let mut s = smart_session();
    let (filters, steps) = (smart_filters_of(&s).len(), s.active().unwrap().history.past_len());
    for p in [json!({"points": [[1, 1], [2, 2]]}), json!({"subtractPoints": [[1, 1]]})] {
        let e = s.execute("image.adjustments.replaceColor", p).unwrap_err();
        assert!(e.to_string().contains("smart filter"), "{e}");
    }
    assert_eq!(smart_filters_of(&s).len(), filters, "nothing recorded");
    assert_eq!(s.active().unwrap().history.past_len(), steps, "no history step");
}

/// Localized Replace Color is refused only for the smart object's own pixels: an alpha channel,
/// or an explicit `layer` that is not a smart object, edits as usual while a smart object is active.
#[test]
fn localized_replace_color_is_refused_only_for_the_smart_objects_pixels() {
    let mut s = smart_session();
    let (filters, steps) = (smart_filters_of(&s).len(), s.active().unwrap().history.past_len());
    let own = json!({"localized": true, "points": [[3, 3]]});
    let e = s.execute("image.adjustments.replaceColor", own).unwrap_err();
    assert!(e.to_string().contains("smart filter"), "{e}");
    assert_eq!(smart_filters_of(&s).len(), filters);
    s.execute("channel.new", json!({"name": "Alpha", "fill": "white"})).unwrap();
    let channel = json!({"localized": true, "points": [[3, 3]], "target": {"channel": 0}});
    s.execute("image.adjustments.replaceColor", channel).expect("a channel under a smart object is not refused");
    assert_eq!(smart_filters_of(&s).len(), filters, "no smart filter for a channel edit");
    assert!(s.active().unwrap().history.past_len() > steps);
    // The smart object is active, but the command names the Background.
    let background = doc(&s).layers.iter().find(|l| !matches!(l.content, LayerContent::Smart(_))).unwrap().id;
    let other = json!({"localized": true, "points": [[3, 3]], "layer": background.0});
    s.execute("image.adjustments.replaceColor", other).expect("an explicit pixel layer is not refused");
    assert_eq!(smart_filters_of(&s).len(), filters);
}

/// Equalize Entire Image Based On Selected Area needs the selection for the histogram only, which
/// a smart filter's output mask cannot express: refused rather than recorded as "selected area only".
#[test]
fn equalize_entire_image_is_refused_on_a_smart_object_with_a_selection() {
    let mut s = smart_session();
    s.execute("select.rect", json!({"x": 0, "y": 0, "width": 24, "height": 32})).unwrap();
    let steps = s.active().unwrap().history.past_len();
    let e = s.execute("image.adjustments.equalize", json!({"selectedAreaOnly": false})).unwrap_err();
    assert!(e.to_string().contains("smart object"), "{e}");
    assert!(smart_filters_of(&s).is_empty(), "nothing recorded");
    assert_eq!(s.active().unwrap().history.past_len(), steps);
    s.execute("image.adjustments.equalize", json!({"selectedAreaOnly": true})).unwrap();
    assert_eq!(smart_filters_of(&s).len(), 1, "the selected area itself still records a filter");
    s.execute("select.deselect", json!({})).unwrap();
    s.execute("image.adjustments.equalize", json!({"selectedAreaOnly": false})).unwrap();
    assert_eq!(smart_filters_of(&s).len(), 2, "without a selection it is plain Equalize");
}

/// #780: a targeted mask enables Invert (and filters) on layers without pixels, such as an
/// adjustment layer; viewing the mask (⌥-click) targets it too.
#[test]
fn a_targeted_mask_enables_editing_it_on_any_layer() {
    let mut s = session(8, "rgb");
    s.execute("layer.newAdjustmentLayer.levels", json!({})).unwrap();
    s.execute("layer.layerMask.revealAll", json!({})).unwrap();
    let mask = json!({"target": "mask"});
    assert!(!s.is_enabled("image.adjustments.invert"), "no pixels to invert");
    assert!(s.is_enabled_with("image.adjustments.invert", &mask));
    assert!(s.is_enabled_with("filter.blur.gaussianBlur", &mask));
    assert!(!s.is_enabled_with("image.adjustments.desaturate", &mask), "only what edits the mask");
    s.execute("image.adjustments.invert", mask.clone()).unwrap();
    assert_eq!((mask_at(&s, 3, 3), mask_at(&s, -500, -500)), (0.0, 0.0), "reveal all → hide all");
    s.execute("view.layerMask", json!({"mode": "gray"})).unwrap();
    assert!(s.is_enabled("image.adjustments.invert"), "the mask view targets the mask");
    s.execute("image.adjustments.invert", json!({})).unwrap();
    assert_eq!(mask_at(&s, 3, 3), 1.0);
    // A filter on a pixel layer's targeted mask leaves the pixels alone.
    s.execute("layer.delete", json!({})).unwrap();
    s.execute("select.rect", json!({"x": 24, "y": 0, "width": 24, "height": 32})).unwrap();
    s.execute("layer.layerMask.revealSelection", json!({})).unwrap();
    let pixels = rgba(&s, 23, 3);
    s.execute("filter.blur.gaussianBlur", json!({"radius": 3, "target": "mask"})).unwrap();
    assert!(mask_at(&s, 23, 3) > 0.0 && mask_at(&s, 23, 3) < 1.0, "the mask's edge is blurred");
    assert_eq!(rgba(&s, 23, 3), pixels);
    // No mask: an error, not the layer's pixels.
    s.execute("layer.layerMask.delete", json!({})).unwrap();
    assert!(!s.is_enabled_with("image.adjustments.invert", &mask));
    assert!(s.execute("image.adjustments.invert", mask).is_err());
    assert_eq!(rgba(&s, 23, 3), pixels);
}

/// Desaturate, Equalize and Replace Color on a smart object record a smart filter that renders
/// what the destructive command bakes into the same pixels (they used to be refused).
#[test]
fn desaturate_equalize_and_replace_color_record_smart_filters() {
    let replace = json!({"color": "#ff0000", "fuzziness": 120, "hue": 90});
    for (id, params) in [("image.adjustments.desaturate", json!({})), ("image.adjustments.equalize", json!({})), ("image.adjustments.replaceColor", replace)] {
        let mut plain = session(8, "rgb");
        let original = flatten(&plain);
        plain.execute(id, params.clone()).unwrap();
        let baked = flatten(&plain);
        assert!(max_diff(&original, &baked) > 0.02, "{id}: the control changes the pixels");

        let mut s = smart_session();
        assert!(s.is_enabled(id), "{id} runs on a smart object");
        s.execute(id, params).unwrap();
        let sf = smart_filters_of(&s);
        assert_eq!(sf.len(), 1, "{id}");
        assert_eq!(sf[0].command, id);
        assert_eq!(max_diff(&flatten(&s), &baked), 0.0, "{id}: the smart filter renders the destructive result");
        s.undo();
        assert!(smart_filters_of(&s).is_empty(), "{id}: undo removes the filter");
    }
}

/// Replace Color without `color` samples the foreground: the smart filter records that colour, so
/// changing the foreground afterwards and re-rendering the filter leaves its result unchanged.
#[test]
fn replace_color_smart_filter_keeps_the_foreground_it_was_run_with() {
    let picked = [0.27, 0.6, 0.52, 1.0];
    let params = json!({"fuzziness": 60, "hue": 90});
    let mut plain = session(8, "rgb");
    plain.tools.foreground = picked;
    plain.execute("image.adjustments.replaceColor", params.clone()).unwrap();
    let baked = flatten(&plain);

    let mut s = smart_session();
    let original = flatten(&s);
    s.tools.foreground = picked;
    s.execute("image.adjustments.replaceColor", params).unwrap();
    assert!(smart_filters_of(&s)[0].params.get("color").is_some(), "the foreground colour is recorded");
    assert_eq!(max_diff(&flatten(&s), &baked), 0.0, "the filter replaces the foreground colour");
    assert!(max_diff(&original, &baked) > 0.02, "the control changes the pixels");
    // A new foreground, then a re-render (hide and show the filter): the result stays put.
    s.tools.foreground = [1.0, 0.0, 0.0, 1.0];
    s.execute("layer.smartFilter.setVisible", json!({"index": 0})).unwrap();
    s.execute("layer.smartFilter.setVisible", json!({"index": 0})).unwrap();
    assert!(smart_filters_of(&s)[0].visible);
    assert_eq!(max_diff(&flatten(&s), &baked), 0.0, "changing the foreground leaves the filter unchanged");
}

/// Lab bounds edit pixels in place; a smart object keeps its pixels, so they are refused there.
#[test]
fn replace_color_lab_bounds_are_refused_on_a_smart_object() {
    let mut s = smart_session();
    let before = flatten(&s);
    let r = s.execute("image.adjustments.replaceColor", json!({"labMinimum": [0, -128, -128], "labMaximum": [100, 127, 127], "hue": 90}));
    assert!(r.is_err(), "Lab bounds on a smart object must be refused");
    assert!(smart_filters_of(&s).is_empty());
    assert_eq!(max_diff(&flatten(&s), &before), 0.0);
}

/// #5335: Curves (⌘M) edits the targeted layer mask, saved channel or Quick Mask at every depth.
#[test]
fn curves_apply_to_non_colour_targets_at_every_depth() {
    let area = photocraft_geom::Rect::from_xywh(0, 0, 48, 32);
    for depth in DEPTHS {
        for target in ["mask", "alpha", "quickMask"] {
            let mut s = session(depth, "rgb");
            s.execute("edit.fill", json!({"contents": "color", "color": "#336699"})).unwrap();
            let p = match target {
                "mask" => {
                    s.execute("layer.layerMask.revealAll", json!({})).unwrap();
                    json!({"points": [[0, 0], [128, 200], [255, 255]], "target": "mask"})
                }
                "alpha" => {
                    s.execute("channel.new", json!({"name": "Sky sel", "fill": "white"})).unwrap();
                    json!({"points": [[0, 0], [128, 200], [255, 255]], "target": {"channel": 0}})
                }
                _ => {
                    s.execute("select.editInQuickMaskMode", json!({"on": true})).unwrap();
                    json!({"points": [[0, 0], [128, 200], [255, 255]], "target": "quickMask"})
                }
            };
            let edit_target = p.get("target").cloned().unwrap_or(json!(null));
            s.edit("levels", |doc, active| {
                let (surf, _) = crate::channel_cmds::target_surface(doc, *active, &json!({ "target": edit_target })).unwrap();
                surf.fill_rect(area, &[0.5]);
                Ok(())
            })
            .unwrap();
            let read = |s: &Session| {
                let d = s.active().unwrap();
                let layer = d.doc.layer(d.active_layer.unwrap()).unwrap();
                let t = match target {
                    "quickMask" => d.doc.quick_mask.as_ref().unwrap().surface.sample_channel(8, 8, 0),
                    "alpha" => d.doc.channels[0].surface.sample_channel(8, 8, 0),
                    _ => layer.mask.as_ref().unwrap().surface.sample_channel(8, 8, 0),
                };
                (rgba(s, 8, 8), t)
            };
            let ((pixels, before), past) = (read(&s), s.active().unwrap().history.past_len());
            s.execute("image.adjustments.curves", p).unwrap();
            let (pixels_after, after) = read(&s);
            let case = format!("{target} at {depth}");
            assert_eq!(pixels_after, pixels, "{case}: colour pixels stay put");
            assert!(after > before + 0.1, "{case}: the curve lifted the midtone ({before} → {after})");
            assert_eq!(s.active().unwrap().history.past_len(), past + 1, "{case}");
            s.undo();
            assert!((read(&s).1 - before).abs() < 1e-5, "{case}: undo restores the target");
        }
    }
}

/// #5335: a layer mask, saved channel or Quick Mask holds gray levels whatever the document's
/// colour mode: inverting 25% grey gives 75% grey in Lab and CMYK, not an untouched plane (Lab
/// ignores the composite) or an ink-separated result (CMYK curves the inks).
#[test]
fn curves_on_grey_targets_ignore_the_document_colour_mode() {
    let area = photocraft_geom::Rect::from_xywh(0, 0, 48, 32);
    for mode in ["lab", "cmyk"] {
        for depth in [8u64, 16] {
            for target in ["mask", "alpha", "quickMask"] {
                let mut s = session(depth, mode);
                let p = match target {
                    "mask" => {
                        s.execute("layer.layerMask.revealAll", json!({})).unwrap();
                        json!("mask")
                    }
                    "alpha" => {
                        s.execute("channel.new", json!({"name": "Sky sel", "fill": "white"})).unwrap();
                        json!({"channel": 0})
                    }
                    _ => {
                        s.execute("select.editInQuickMaskMode", json!({"on": true})).unwrap();
                        json!("quickMask")
                    }
                };
                s.edit("levels", |doc, active| {
                    let (surf, _) = crate::channel_cmds::target_surface(doc, *active, &json!({ "target": p })).unwrap();
                    surf.fill_rect(area, &[0.25]);
                    Ok(())
                })
                .unwrap();
                let read = |s: &Session| match target {
                    "quickMask" => doc(s).quick_mask.as_ref().unwrap().surface.sample_channel(8, 8, 0),
                    "alpha" => doc(s).channels[0].surface.sample_channel(8, 8, 0),
                    _ => mask_at(s, 8, 8),
                };
                let before = read(&s);
                let pixels = rgba(&s, 8, 8);
                s.execute("image.adjustments.curves", json!({"points": [[0, 255], [255, 0]], "target": p})).unwrap();
                let after = read(&s);
                let case = format!("{mode} {depth}-bit {target}");
                assert!((before - 0.25).abs() < 0.01, "{case}: filled with 25% grey ({before})");
                assert!((after - 0.75).abs() < 0.02, "{case}: inverting 25% grey gives 75% grey ({after})");
                assert_eq!(rgba(&s, 8, 8), pixels, "{case}: colour pixels stay put");
            }
        }
    }
}

#[test]
fn replace_color_localized_clusters_only_change_the_colour_near_the_sample() {
    // The same cream on the left (x < 8) and the right (x ≥ 40), grey between.
    let cream = [0.9, 0.88, 0.79, 1.0];
    let mut s = session(8, "rgb");
    paint(&mut s, |x, _| if !(8..40).contains(&x) { cream } else { [0.5, 0.5, 0.5, 1.0] });
    let base = |s: &Session| (rgba(s, 3, 16), rgba(s, 44, 16), rgba(s, 20, 16));
    let (left, right, grey) = base(&s);
    let shift = json!({"color": "#e6e0c9", "fuzziness": 20, "hue": 180, "saturation": 60});
    // Without clusters the colour is replaced everywhere (unchanged behaviour).
    s.execute("image.adjustments.replaceColor", shift.clone()).unwrap();
    assert!(changed(rgba(&s, 3, 16), left) > 0.05 && changed(rgba(&s, 44, 16), right) > 0.05);
    s.execute("edit.undo", json!({})).unwrap();
    // Picked on the left with Range 50 % (24 px): only the left cream changes.
    let mut local = shift.clone();
    local["points"] = json!([[2, 16]]);
    local["localized"] = json!(true);
    local["range"] = json!(50);
    s.execute("image.adjustments.replaceColor", local.clone()).unwrap();
    assert!(changed(rgba(&s, 3, 16), left) > 0.05, "near the sample");
    assert_eq!(rgba(&s, 44, 16), right, "the same colour far from the sample");
    assert_eq!(rgba(&s, 20, 16), grey);
    s.execute("edit.undo", json!({})).unwrap();
    // Inside a selection that doesn't start at the origin the sample stays where it was picked.
    s.execute("select.rect", json!({"x": 1, "y": 4, "width": 46, "height": 24})).unwrap();
    s.execute("image.adjustments.replaceColor", local).unwrap();
    assert!(changed(rgba(&s, 3, 16), left) > 0.05);
    assert_eq!(rgba(&s, 44, 16), right);
    // Clusters need a place to be near.
    let err = s.execute("image.adjustments.replaceColor", json!({"localized": true})).unwrap_err();
    assert!(err.to_string().contains("points"), "{err}");
    let err = s.execute("image.adjustments.replaceColor", json!({"points": [[1]]})).unwrap_err();
    assert!(err.to_string().contains("bad point"), "{err}");
}

/// #6280: Add to Sample and Subtract from Sample, as in Photoshop: several picked colours are
/// all replaced, and a subtracted one is left alone, matching as Select › Color Range does for
/// the same samples.
#[test]
fn replace_color_adds_and_subtracts_samples_as_color_range_does() {
    // Red on the left (x < 16), blue in the middle, green on the right (32 ≤ x < 44), and a
    // lighter red at the edge that the samples only partly cover.
    let (red, blue, green, pink) = ([0.85, 0.1, 0.1, 1.0], [0.1, 0.15, 0.85, 1.0], [0.1, 0.8, 0.15, 1.0], [0.95, 0.25, 0.25, 1.0]);
    let mut s = session(8, "rgb");
    paint(&mut s, |x, _| {
        if x < 16 {
            red
        } else if x < 32 {
            blue
        } else if x < 44 {
            green
        } else {
            pink
        }
    });
    let before = (rgba(&s, 4, 16), rgba(&s, 24, 16), rgba(&s, 40, 16), rgba(&s, 46, 16));
    let original = pixels(&s);
    let shift = |extra: Value| {
        let mut p = json!({"fuzziness": 40, "hue": 120, "saturation": 20});
        p.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        p
    };
    // Red and blue sampled (one added): both change, green doesn't.
    s.execute("image.adjustments.replaceColor", shift(json!({"color": "#d91a1a", "points": [[4, 16], [24, 16]]}))).unwrap();
    assert!(changed(rgba(&s, 4, 16), before.0) > 0.1, "red is replaced");
    assert!(changed(rgba(&s, 24, 16), before.1) > 0.1, "the added blue is replaced");
    assert_eq!(rgba(&s, 40, 16), before.2, "green wasn't sampled");
    let partial = rgba(&s, 46, 16);
    // One undo step brings every pixel back exactly.
    s.execute("edit.undo", json!({})).unwrap();
    assert!(pixels(&s) == original, "undo restores the pixels");
    // The same coverage as Select › Color Range for the same clicks.
    s.execute("select.colorRange", json!({"points": [[4, 16], [24, 16]], "fuzziness": 40})).unwrap();
    let sel = doc(&s).selection.clone().unwrap();
    for x in [4, 24, 40] {
        let want = sel.read_region(Rect::from_xywh(x, 16, 1, 1))[0];
        assert!((want > 0.5) == (x < 32), "Color Range at {x}: {want}");
    }
    // The lighter red is partly covered, and replaced in proportion: the blend of its colour and
    // its fully replaced colour at that coverage, not either one (as a 0.5 threshold would give).
    let k = sel.read_region(Rect::from_xywh(46, 16, 1, 1))[0];
    assert!(k > 0.1 && k < 0.9, "partial coverage: {k}");
    let mut full = [before.3];
    tone::replace_color_masked(&mut full, &[1.0], 120.0, 20.0, 0.0);
    let blend: [f32; 4] = std::array::from_fn(|c| before.3[c] + (full[0][c] - before.3[c]) * k);
    assert!(changed(partial, blend) < 1.5 / 255.0, "{partial:?} vs the blend {blend:?} at coverage {k}");
    assert!(changed(blend, before.3).min(changed(blend, full[0])) > 0.1, "the blend is far from both ends a threshold gives");
    s.execute("select.deselect", json!({})).unwrap();
    // A subtraction undoes exactly too. (Blue is far from the red sample here, so subtracting
    // it changes nothing; `replace_color_subtracting_inside_the_added_range_takes_a_colour_out`
    // has a subtraction that does.)
    let selected = pixels(&s);
    s.execute("image.adjustments.replaceColor", shift(json!({"points": [[4, 16]], "subtractPoints": [[24, 16]], "order": ["+", "-"]}))).unwrap();
    assert!(changed(rgba(&s, 4, 16), before.0) > 0.1);
    assert_eq!(rgba(&s, 40, 16), before.2);
    s.execute("edit.undo", json!({})).unwrap();
    assert!(pixels(&s) == selected, "undo restores the pixels");
    // Bad samples are refused, naming this command.
    let err = s.execute("image.adjustments.replaceColor", shift(json!({"points": [[4, 16]], "subtractPoints": [[999, 16]]}))).unwrap_err();
    assert!(err.to_string().contains("replaceColor") && err.to_string().contains("outside"), "{err}");
}

/// FredApps review on #10443: a subtracted sample inside the added range. Photoshop's Subtract
/// from Sample reshapes the samples' Lab box (`LabRange::subtract`): a subtracted colour inside
/// it pulls the nearer edge onto itself, so it stays matched and the added colour beyond that
/// edge drops out. Two added greys span dark to light; subtracting a grey near the light end
/// leaves the light one unreplaced.
#[test]
fn replace_color_subtracting_inside_the_added_range_takes_a_colour_out() {
    let grey = |v: f32| [v, v, v, 1.0];
    // Dark grey (x < 12), mid grey (subtracted), light grey, black (never sampled).
    let mut s = session(8, "rgb");
    paint(&mut s, |x, _| match x {
        ..12 => grey(0.25),
        12..24 => grey(0.65),
        24..36 => grey(0.75),
        _ => grey(0.0),
    });
    let at = [(4, 16), (16, 16), (28, 16), (40, 16)];
    let before: Vec<_> = at.iter().map(|&(x, y)| rgba(&s, x, y)).collect();
    let added = json!({"fuzziness": 20, "lightness": -50, "points": [[4, 16], [28, 16]]});
    // Added only: the box spans dark to light, so all three greys are replaced.
    s.execute("image.adjustments.replaceColor", added.clone()).unwrap();
    for (i, &(x, y)) in at[..3].iter().enumerate() {
        assert!(changed(rgba(&s, x, y), before[i]) > 0.1, "grey {i} is in the added range");
    }
    assert_eq!(rgba(&s, 40, 16), before[3]);
    s.execute("edit.undo", json!({})).unwrap();
    // Mid grey subtracted: the light end moves onto it, so the light grey is left alone.
    let mut subtracted = added.clone();
    subtracted["subtractPoints"] = json!([[16, 16]]);
    subtracted["order"] = json!(["+", "+", "-"]);
    s.execute("image.adjustments.replaceColor", subtracted.clone()).unwrap();
    assert!(changed(rgba(&s, 4, 16), before[0]) > 0.1, "the dark grey is still replaced");
    assert!(changed(rgba(&s, 16, 16), before[1]) > 0.1, "the subtracted grey stays matched, at the new edge");
    assert_eq!(rgba(&s, 28, 16), before[2], "the light grey beyond it is taken out");
    assert_eq!(rgba(&s, 40, 16), before[3]);
    // Select › Color Range agrees for the same clicks.
    s.execute("edit.undo", json!({})).unwrap();
    let mut range = subtracted;
    range.as_object_mut().unwrap().remove("lightness");
    s.execute("select.colorRange", range).unwrap();
    let sel = doc(&s).selection.clone().unwrap();
    let covered: Vec<bool> = at.iter().map(|&(x, y)| sel.read_region(Rect::from_xywh(x, y, 1, 1))[0] > 0.5).collect();
    assert_eq!(covered, [true, true, false, false]);
}

/// CodeRabbit review on #10443: the click order (`"order"`) reaches the matcher, as in Select ›
/// Color Range. Added dark grey, subtracted mid grey, added light grey (`+ - +`): the subtraction
/// comes while the box is the dark grey alone, far from the mid grey, so it changes nothing and the
/// light grey is replaced; all additions first (`+ + -`) would leave it out.
#[test]
fn replace_color_follows_the_click_order_as_color_range_does() {
    let grey = |v: f32| [v, v, v, 1.0];
    // Dark grey (x < 12), mid grey (subtracted), light grey, black (never sampled).
    let mut s = session(8, "rgb");
    paint(&mut s, |x, _| match x {
        ..12 => grey(0.25),
        12..24 => grey(0.65),
        24..36 => grey(0.75),
        _ => grey(0.0),
    });
    let at = [(4, 16), (16, 16), (28, 16), (40, 16)];
    let before: Vec<_> = at.iter().map(|&(x, y)| rgba(&s, x, y)).collect();
    let clicks = json!({"fuzziness": 20, "points": [[4, 16], [28, 16]], "subtractPoints": [[16, 16]], "order": ["+", "-", "+"]});
    let mut p = clicks.clone();
    p["lightness"] = json!(-50);
    s.execute("image.adjustments.replaceColor", p).unwrap();
    let replaced: Vec<bool> = at.iter().enumerate().map(|(i, &(x, y))| changed(rgba(&s, x, y), before[i]) > 0.1).collect();
    s.execute("edit.undo", json!({})).unwrap();
    // Darkening leaves black black whether or not it's matched: lighten to see whether it is.
    p = clicks.clone();
    p["lightness"] = json!(50);
    s.execute("image.adjustments.replaceColor", p).unwrap();
    let lifted = changed(rgba(&s, 40, 16), before[3]) > 0.1;
    s.execute("edit.undo", json!({})).unwrap();
    s.execute("select.colorRange", clicks).unwrap();
    let sel = doc(&s).selection.clone().unwrap();
    let covered: Vec<bool> = at.iter().map(|&(x, y)| sel.read_region(Rect::from_xywh(x, y, 1, 1))[0] > 0.5).collect();
    assert_eq!(covered, [true, true, true, false], "Color Range: the light grey is added back after the subtraction");
    assert_eq!(replaced, covered, "Replace Color matches what Color Range covers for the same clicks");
    assert_eq!(lifted, covered[3], "Replace Color matches black as Color Range does");
}

/// FredApps review on #10443: Localized Color Clusters with several samples. A matching colour
/// is replaced only near one of the samples, as in Select › Color Range.
#[test]
fn replace_color_localized_clusters_with_several_samples() {
    // Two creams, sampled at x = 2 and x = 24; the first cream again far right (x ≥ 40).
    let (cream, cream2, grey) = ([0.9, 0.88, 0.79, 1.0], [0.88, 0.86, 0.76, 1.0], [0.5, 0.5, 0.5, 1.0]);
    let mut s = session(8, "rgb");
    paint(&mut s, |x, _| match x {
        ..8 => cream,
        20..28 => cream2,
        40.. => cream,
        _ => grey,
    });
    let at = [(3, 16), (24, 16), (44, 16), (14, 16)];
    let before: Vec<_> = at.iter().map(|&(x, y)| rgba(&s, x, y)).collect();
    let mut p = json!({"fuzziness": 20, "hue": 180, "saturation": 60, "points": [[2, 16], [24, 16]]});
    // Without clusters both creams change everywhere, the far one too.
    s.execute("image.adjustments.replaceColor", p.clone()).unwrap();
    assert!(changed(rgba(&s, 44, 16), before[2]) > 0.05, "far cream without clusters");
    s.execute("edit.undo", json!({})).unwrap();
    // Range 20 % of 48 px: within about 10 px of either sample.
    p["localized"] = json!(true);
    p["range"] = json!(20);
    s.execute("image.adjustments.replaceColor", p.clone()).unwrap();
    assert!(changed(rgba(&s, 3, 16), before[0]) > 0.05, "near the first sample");
    assert!(changed(rgba(&s, 24, 16), before[1]) > 0.05, "near the second sample");
    assert_eq!(rgba(&s, 44, 16), before[2], "the same cream far from both samples");
    assert_eq!(rgba(&s, 14, 16), before[3], "grey isn't sampled");
    // Select › Color Range agrees for the same clicks.
    s.execute("edit.undo", json!({})).unwrap();
    for k in ["hue", "saturation"] {
        p.as_object_mut().unwrap().remove(k);
    }
    s.execute("select.colorRange", p).unwrap();
    let sel = doc(&s).selection.clone().unwrap();
    let covered: Vec<bool> = at.iter().map(|&(x, y)| sel.read_region(Rect::from_xywh(x, y, 1, 1))[0] > 0.5).collect();
    assert_eq!(covered, [true, true, false, false]);
}

/// #10443 review: with only Subtract from Sample clicks, the dialog's From colour is what the
/// subtraction starts from, not the foreground (black here), so the red is still replaced.
#[test]
fn replace_color_subtract_only_starts_from_the_from_colour() {
    let (red, blue) = ([0.85, 0.1, 0.1, 1.0], [0.1, 0.15, 0.85, 1.0]);
    let mut s = session(8, "rgb");
    paint(&mut s, |x, _| if x < 16 { red } else { blue });
    let (r0, b0) = (rgba(&s, 4, 16), rgba(&s, 24, 16));
    let p = json!({"color": "#d91a1a", "hue": 120, "fuzziness": 40, "points": [], "subtractPoints": [[24, 16]]});
    s.execute("image.adjustments.replaceColor", p).unwrap();
    assert_ne!(rgba(&s, 4, 16), r0, "the From colour (red) is replaced");
    // Blue is outside the From colour's range anyway: subtracting from a single colour doesn't
    // reshape its range, as in Photoshop (see
    // `replace_color_subtracting_inside_the_added_range_takes_a_colour_out`).
    assert_eq!(rgba(&s, 24, 16), b0, "blue isn't matched");
}

/// #10443 review: a `null` From colour means the foreground, for the subtract-only matcher too.
#[test]
fn replace_color_subtract_only_reads_a_null_colour_as_the_foreground() {
    let (red, blue) = ([0.85, 0.1, 0.1, 1.0], [0.1, 0.15, 0.85, 1.0]);
    let mut s = session(8, "rgb");
    paint(&mut s, |x, _| if x < 16 { red } else { blue });
    s.tools.foreground = red;
    let r0 = rgba(&s, 4, 16);
    let p = json!({"color": null, "hue": 120, "fuzziness": 40, "points": [], "subtractPoints": [[24, 16]]});
    s.execute("image.adjustments.replaceColor", p).unwrap();
    assert_ne!(rgba(&s, 4, 16), r0, "the foreground (red) is replaced");
}

/// A malformed `subtractPoints` is refused even with one sample, where the single-colour path
/// would otherwise ignore it (CodeRabbit review on #10443).
#[test]
fn replace_color_refuses_malformed_subtract_points() {
    let mut s = session(8, "rgb");
    paint(&mut s, |_, _| [0.85, 0.1, 0.1, 1.0]);
    for points in [json!([]), json!([[4, 16]])] {
        let p = json!({"color": "#d91a1a", "hue": 120, "points": points, "subtractPoints": "invalid"});
        let err = s.execute("image.adjustments.replaceColor", p).unwrap_err();
        assert!(err.to_string().contains("replaceColor") && err.to_string().contains("subtractPoints"), "{err}");
    }
    // An empty list is no subtraction: the single-colour path.
    s.execute("image.adjustments.replaceColor", json!({"color": "#d91a1a", "hue": 120, "points": [[4, 16]], "subtractPoints": []})).unwrap();
}

/// #6095: ⌘I in Quick Mask inverts the whole mask, not only where it was white, so leaving Quick
/// Mask selects everything but the original rectangle, whichever way the colour reads. With
/// Color Indicates Selected Areas and nothing selected, it selects everything. A black alpha
/// channel inverts to white.
#[test]
fn invert_turns_the_black_of_quick_mask_and_alpha_channels_white() {
    for depth in DEPTHS {
        let mut s = session(depth, "rgb");
        let pixels = rgba(&s, 2, 2);
        s.execute("select.rect", json!({"x": 10, "y": 8, "width": 20, "height": 10})).unwrap();
        s.execute("select.editInQuickMaskMode", json!({"on": true})).unwrap();
        s.execute("image.adjustments.invert", json!({"target": "quickMask"})).unwrap();
        let q = &doc(&s).quick_mask.as_ref().unwrap().surface;
        let (inside, outside) = (q.sample_channel(15, 12, 0), q.sample_channel(2, 2, 0));
        assert!(inside < 0.01 && outside > 0.99, "{depth}-bit Quick Mask inverted: inside {inside}, outside {outside}");
        s.execute("select.editInQuickMaskMode", json!({"on": false})).unwrap();
        let sel = doc(&s).selection.as_ref().expect("an inverse selection after Quick Mask");
        let (inside, outside) = (sel.sample_channel(15, 12, 0), sel.sample_channel(2, 2, 0));
        assert!(inside < 0.01 && outside > 0.99, "{depth}-bit selection: inside {inside}, outside {outside}");
        assert_eq!(rgba(&s, 2, 2), pixels, "{depth}-bit: the layer is untouched");

        s.execute("select.deselect", json!({})).unwrap();
        s.execute("channel.new", json!({"name": "Black", "fill": "black"})).unwrap();
        s.execute("image.adjustments.invert", json!({"target": {"channel": 0}})).unwrap();
        let a = doc(&s).channels[0].surface.sample_channel(40, 30, 0);
        assert!(a > 0.99, "{depth}-bit black alpha channel inverts to white ({a})");
        s.undo();
        assert!(doc(&s).channels[0].surface.sample_channel(40, 30, 0) < 0.01, "{depth}-bit: undo restores the black");

        // Color Indicates Selected Areas: a rectangle comes back inverted, and nothing selected
        // comes back as everything selected.
        for (start, inside, outside) in [("rectangle", 0.0, 1.0), ("nothing", 1.0, 1.0)] {
            let mut s = session(depth, "rgb");
            s.execute("channel.options", json!({"channel": "quickMask", "indicates": "selected"})).unwrap();
            if start == "rectangle" {
                s.execute("select.rect", json!({"x": 10, "y": 8, "width": 20, "height": 10})).unwrap();
            }
            s.execute("select.editInQuickMaskMode", json!({"on": true})).unwrap();
            s.execute("image.adjustments.invert", json!({"target": "quickMask"})).unwrap();
            s.execute("select.editInQuickMaskMode", json!({"on": false})).unwrap();
            let sel = doc(&s).selection.as_ref().unwrap_or_else(|| panic!("{depth}-bit Selected Areas, {start}: a selection after Quick Mask"));
            let got = (sel.sample_channel(15, 12, 0), sel.sample_channel(2, 2, 0));
            assert!(
                (got.0 - inside).abs() < 0.01 && (got.1 - outside).abs() < 0.01,
                "{depth}-bit Selected Areas, {start}: selection (inside, outside) {got:?}"
            );
        }
    }
}

#[test]
fn equalize_smart_filter_uses_selected_pixels_for_its_histogram() {
    for depth in DEPTHS {
        let mut plain = session(depth, "rgb");
        let mut smart = session(depth, "rgb");
        smart.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
        for s in [&mut plain, &mut smart] {
            s.execute("select.rect", json!({"x": 0, "y": 0, "width": 12, "height": 32})).unwrap();
            s.execute("image.adjustments.equalize", json!({})).unwrap();
        }
        assert!(max_diff(&flatten(&plain), &flatten(&smart)) < 2.0 / 255.0, "depth={depth}");
        // Disabling the filter mask changes the histogram too: no stale cached stage.
        let mut full = session(depth, "rgb");
        full.execute("image.adjustments.equalize", json!({})).unwrap();
        smart.execute("select.deselect", json!({})).unwrap();
        smart.execute("layer.smartFilter.disableFilterMask", json!({"enabled": false})).unwrap();
        assert!(max_diff(&flatten(&full), &flatten(&smart)) < 2.0 / 255.0, "disabled mask, depth={depth}");
    }
}

#[test]
fn replace_color_null_records_foreground_and_bad_color_does_not_edit() {
    for depth in DEPTHS {
        let mut plain = session(depth, "rgb");
        let mut smart = session(depth, "rgb");
        smart.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
        let params = json!({"color": null, "fuzziness": 200, "hue": 90});
        for s in [&mut plain, &mut smart] {
            s.tools.foreground = [0.27, 0.6, 0.52, 1.0];
            s.execute("image.adjustments.replaceColor", params.clone()).unwrap();
        }
        assert_eq!(max_diff(&flatten(&plain), &flatten(&smart)), 0.0);
        assert_eq!(smart_filters_of(&smart)[0].params["color"], json!([0.27_f32, 0.6_f32, 0.52_f32, 1.0_f32]));
        let before = smart.active().unwrap().doc.clone();
        let steps = smart.active().unwrap().history.past_len();
        for color in [json!(3), json!("wrong"), json!([1, 2]), json!([1e300, 0, 0]), json!(["red", 0, 0])] {
            assert!(smart.execute("image.adjustments.replaceColor", json!({"color": color})).is_err());
            assert!(std::sync::Arc::ptr_eq(&smart.active().unwrap().doc, &before));
            assert_eq!(smart.active().unwrap().history.past_len(), steps);
        }
    }
}

/// #6558: a LUT folder lists its LUT files by name, case-insensitively, and nothing else.
#[test]
fn folder_luts_lists_lut_files_by_name() {
    let dir = tempfile::tempdir().unwrap();
    for f in ["b.cube", "A.3DL", "c.look", "notes.txt", ".hidden.cube", "noext"] {
        std::fs::write(dir.path().join(f), "LUT_3D_SIZE 2\n").unwrap();
    }
    std::fs::create_dir(dir.path().join("sub.cube")).unwrap();
    // Too large to be a LUT (sparse, so cheap to make).
    std::fs::File::create(dir.path().join("huge.cube")).unwrap().set_len(MAX_FOLDER_LUT_BYTES + 1).unwrap();
    // A symlink to a LUT is listed: shared LUT libraries are often linked in.
    #[cfg(unix)]
    std::os::unix::fs::symlink(dir.path().join("b.cube"), dir.path().join("linked.cube")).unwrap();
    let want: Vec<_> = ["A.3DL", "b.cube", "c.look"].into_iter().chain(cfg!(unix).then_some("linked.cube")).collect();
    let names: Vec<_> = folder_luts(dir.path().to_str().unwrap()).into_iter().map(|(name, _)| name).collect();
    assert_eq!(names, want);
    assert!(folder_luts("").is_empty());
    assert!(folder_luts(dir.path().join("missing").to_str().unwrap()).is_empty());
}

/// #9269: Black & White's Auto spreads an image's greys wider than the default mix, leaves the
/// weights of colours the image lacks at their defaults, and changes nothing in the document.
#[test]
fn black_white_auto_spreads_the_greys() {
    use crate::adjust_params::{BW_DEFAULTS, HUE_RANGES};
    // A red and a blue the default mix turns into the same grey (0.43).
    let (red, blue) = ([0.776, 0.2, 0.2], [0.29, 0.29, 1.0]);
    let gray = |c: [f32; 3], w: &[f32; 6]| photocraft_compose::adjust::black_white_gray(c, w);
    let w = auto_black_white(&[red, blue]);
    let gap = |w: &[f32; 6]| (gray(red, w) - gray(blue, w)).abs();
    assert!(gap(&BW_DEFAULTS) < 0.01);
    assert!(gap(&w) > 1.0 / 32.0, "{w:?}: {}", gap(&w));
    assert!(w.iter().all(|v| (0.0..=150.0).contains(v)), "{w:?}");
    // Yellows, greens, cyans and magentas aren't in the image: they keep their defaults.
    for k in [1, 2, 3, 5] {
        assert_eq!(w[k], BW_DEFAULTS[k], "{}", HUE_RANGES[k]);
    }
    // A grey image (and no pixels at all) keeps the default mix.
    assert_eq!(auto_black_white(&[[0.2; 3], [0.7; 3]]), BW_DEFAULTS);
    assert_eq!(auto_black_white(&[]), BW_DEFAULTS);

    // The command: an adjustment layer reads the image under it; the document is unchanged.
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 20, "height": 10})).unwrap();
    paint(&mut s, |x, _| if x < 10 { [0.776, 0.2, 0.2, 1.0] } else { [0.29, 0.29, 1.0, 1.0] });
    let id = s.execute("layer.newAdjustmentLayer.blackWhite", json!({})).unwrap()["layer"].as_u64().unwrap();
    let rev = s.active().unwrap().revision;
    let r = s.execute("image.adjustments.blackWhite.auto", json!({"layer": id})).unwrap();
    let got: Vec<f32> = HUE_RANGES.iter().map(|k| r[*k].as_f64().unwrap() as f32).collect();
    assert_eq!(got, w.to_vec());
    assert_eq!(s.active().unwrap().revision, rev, "nothing changes");
    assert!(s.execute("image.adjustments.blackWhite.auto", json!({"layer": 999})).is_err());
    assert!(s.execute("image.adjustments.blackWhite.auto", json!({"layer": "x"})).is_err());

    // A layer above the adjustment isn't part of what it adjusts.
    s.execute("layer.new.layer", json!({})).unwrap();
    paint(&mut s, |_, _| [0.5, 0.5, 0.5, 1.0]);
    assert_eq!(s.execute("image.adjustments.blackWhite.auto", json!({"layer": id})).unwrap(), r);

    // Sampling a big canvas still finds a thin painted stripe.
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 512, "height": 512, "background": "transparent"})).unwrap();
    paint(&mut s, |x, y| match (x, y < 256) {
        (1, true) => [0.776, 0.2, 0.2, 1.0],
        (1, false) => [0.29, 0.29, 1.0, 1.0],
        _ => [0.0; 4],
    });
    let r = s.execute("image.adjustments.blackWhite.auto", json!({})).unwrap();
    let got: Vec<f32> = HUE_RANGES.iter().map(|k| r[*k].as_f64().unwrap() as f32).collect();
    assert_eq!(got, w.to_vec());

    // Columns alternating with a period that divides the sampling stride (4 here): both colours
    // are still sampled, not only the even columns.
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 512, "height": 512})).unwrap();
    paint(&mut s, |x, _| if x % 2 == 0 { [0.776, 0.2, 0.2, 1.0] } else { [0.29, 0.29, 1.0, 1.0] });
    let r = s.execute("image.adjustments.blackWhite.auto", json!({})).unwrap();
    let got: Vec<f32> = HUE_RANGES.iter().map(|k| r[*k].as_f64().unwrap() as f32).collect();
    assert_ne!(got, BW_DEFAULTS.to_vec(), "only one colour was sampled");
    assert!(gap(&got.clone().try_into().unwrap()) > 1.0 / 32.0, "{got:?}");
}

/// #9301 review: as in the reference application, Auto reads the whole canvas whatever the
/// selection, the adjustment's mask or its clip base; a smart filter's Auto reads the pixels below
/// that filter, not the filtered result; a canvas too large to analyse is an error, not the
/// default mix.
#[test]
fn black_white_auto_reads_the_whole_input() {
    use crate::adjust_params::{BW_DEFAULTS, HUE_RANGES};
    let (red, blue) = ([0.776, 0.2, 0.2, 1.0], [0.29, 0.29, 1.0, 1.0]);
    let mix = auto_black_white(&[[red[0], red[1], red[2]], [blue[0], blue[1], blue[2]]]);
    assert_ne!(mix, BW_DEFAULTS);
    let auto = |s: &mut Session, p: Value| -> Vec<f32> {
        let r = s.execute("image.adjustments.blackWhite.auto", p).unwrap();
        HUE_RANGES.iter().map(|k| r[*k].as_f64().unwrap() as f32).collect()
    };
    let split = |x: i32, _| if x < 10 { red } else { blue };

    // A selection over the red half doesn't narrow Image › Adjustments' Auto.
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 20, "height": 10})).unwrap();
    paint(&mut s, split);
    s.execute("select.rect", json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap();
    assert_eq!(auto(&mut s, json!({})), mix.to_vec());

    // Nor does an adjustment layer's mask hiding the blue half.
    s.execute("select.deselect", json!({})).unwrap();
    let id = s.execute("layer.newAdjustmentLayer.blackWhite", json!({})).unwrap()["layer"].as_u64().unwrap();
    s.edit("mask", |doc, _| {
        let b = doc.bounds();
        let mut m = photocraft_doc::LayerMask::reveal_all();
        m.surface.write_region(Rect::new(10, 0, b.x1, b.y1), &vec![0.0; 10 * 10]);
        doc.layer_mut(photocraft_doc::LayerId(id)).unwrap().mask = Some(m);
        Ok(())
    })
    .unwrap();
    assert_eq!(auto(&mut s, json!({"layer": id})), mix.to_vec());

    // Nor clipping it to a layer that only covers the red half.
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 20, "height": 10})).unwrap();
    paint(&mut s, |_, _| blue);
    s.execute("layer.new.layer", json!({})).unwrap();
    paint(&mut s, |x, _| if x < 10 { red } else { [0.0; 4] });
    let id = s.execute("layer.newAdjustmentLayer.blackWhite", json!({})).unwrap()["layer"].as_u64().unwrap();
    s.execute("layer.createClippingMask", json!({})).unwrap();
    assert!(s.active().unwrap().doc.layer(photocraft_doc::LayerId(id)).unwrap().clipped);
    assert_eq!(auto(&mut s, json!({"layer": id})), mix.to_vec());

    // A Black & White smart filter: its own result is grey, its input is the colours.
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 20, "height": 10})).unwrap();
    paint(&mut s, split);
    s.execute("filter.convertForSmartFilters", json!({})).unwrap();
    s.execute("image.adjustments.blackWhite", json!({})).unwrap();
    let smart = s.active().unwrap().active_layer.unwrap().0;
    assert_eq!(auto(&mut s, json!({"layer": smart})), BW_DEFAULTS.to_vec(), "the filtered result is grey");
    assert_eq!(auto(&mut s, json!({"layer": smart, "filter": 0})), mix.to_vec());
    for bad in [json!({"layer": smart, "filter": 1}), json!({"layer": smart, "filter": "x"}), json!({"layer": smart, "filter": -1})] {
        assert!(s.execute("image.adjustments.blackWhite.auto", bad).is_err());
    }
    s.execute("file.new", json!({"width": 20, "height": 10})).unwrap();
    assert!(s.execute("image.adjustments.blackWhite.auto", json!({"filter": 0})).is_err(), "not a smart object");

    // Past the render limit there is nothing to sample.
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 40000, "height": 30000, "background": "transparent"})).unwrap();
    let id = s.execute("layer.newAdjustmentLayer.blackWhite", json!({})).unwrap()["layer"].as_u64().unwrap();
    assert!(s.execute("image.adjustments.blackWhite.auto", json!({"layer": id})).is_err());
}
