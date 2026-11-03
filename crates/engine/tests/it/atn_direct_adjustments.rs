//! Recorded direct Gradient Map and Replace Color forms from #10603 / #10607.
use photocraft_engine::{Session, atn_cmds};
use photocraft_psd::{
    atn::{self, AtnAction, AtnEvent, AtnFile, AtnItem},
    descriptor::{Descriptor, Id, UnicodeString, Value as D},
};
use serde_json::json;

fn enumeration(kind: &str, value: &str) -> D {
    D::Enumerated { type_id: Id::new(kind), value: Id::new(value) }
}

fn color(red: f64, green: f64, blue: f64) -> D {
    D::Descriptor(Descriptor::new("RGBC").with("Rd  ", D::Double(red)).with("Grn ", D::Double(green)).with("Bl  ", D::Double(blue)))
}

fn gradient() -> Descriptor {
    let stop = |location, rgb| {
        D::Descriptor(
            Descriptor::new("Clrt").with("Clr ", rgb).with("Type", enumeration("Clry", "UsrS")).with("Lctn", D::Integer(location)).with("Mdpn", D::Integer(50)),
        )
    };
    let opacity = |location| {
        D::Descriptor(
            Descriptor::new("TrnS").with("Opct", D::UnitFloat { unit: *b"#Prc", value: 100.0 }).with("Lctn", D::Integer(location)).with("Mdpn", D::Integer(50)),
        )
    };
    Descriptor::new("null").with("gradientsInterpolationMethod", enumeration("gradientInterpolationMethodType", "Gcls")).with(
        "Grad",
        D::Descriptor(
            Descriptor::new("Grdn")
                .with("Nm  ", D::Text(UnicodeString::new_nul("Recorded gradient")))
                .with("GrdF", enumeration("GrdF", "CstS"))
                .with("Intr", D::Double(4096.0))
                .with("Clrs", D::List(vec![stop(0, color(0.0, 0.0, 255.0)), stop(4096, color(255.0, 0.0, 0.0))]))
                .with("Trns", D::List(vec![opacity(0), opacity(4096)])),
        ),
    )
}

fn replace() -> Descriptor {
    let lab = || D::Descriptor(Descriptor::new("LbCl").with("Lmnc", D::Double(40.0)).with("A   ", D::Double(30.0)).with("B   ", D::Double(40.0)));
    Descriptor::new("null")
        .with("Fzns", D::Integer(40))
        .with("Mnm ", lab())
        .with("Mxm ", lab())
        .with("H   ", D::Integer(30))
        .with("Strt", D::Integer(0))
        .with("Lght", D::Integer(0))
        .with("colorModel", D::Integer(0))
}

fn bytes(event: [u8; 4], descriptor: Descriptor) -> Vec<u8> {
    atn::write(&AtnFile {
        name: UnicodeString::new_nul("Direct adjustments"),
        expanded: false,
        actions: vec![AtnAction {
            name: UnicodeString::new_nul("Recorded"),
            items: vec![AtnItem { event: AtnEvent::Code(event), descriptor: Some(descriptor), ..Default::default() }],
            ..Default::default()
        }],
        warnings: vec![],
        ..Default::default()
    })
    .unwrap()
}

fn load(event: [u8; 4], descriptor: Descriptor, command: &str) -> Session {
    let mut s = Session::new();
    let bytes = bytes(event, descriptor);
    let report = s.execute("actions.load", json!({"data": photocraft_paint::tile::b64_encode(&bytes)})).unwrap();
    assert_eq!(report["unsupported"], json!([]), "recorded {} must map to {command}", String::from_utf8_lossy(&event));
    assert_eq!(s.actions.list[0].steps[0].0, command);
    s
}

#[test]
fn recorded_grmp_imports_as_direct_gradient_map() {
    let s = load(*b"GrMp", gradient(), "image.adjustments.gradientMap");
    assert_eq!(s.actions.list[0].steps[0].1["stops"], json!([[0, [0, 0, 1]], [1, [1, 0, 0]]]));
}

#[test]
fn recorded_rplc_imports_as_direct_replace_color() {
    let s = load(*b"RplC", replace(), "image.adjustments.replaceColor");
    assert_eq!(s.actions.list[0].steps[0].1["hue"], json!(30));
}

fn saved(s: &Session) -> AtnItem {
    let refs = s.actions.list.iter().collect::<Vec<_>>();
    let (bytes, skipped) = atn_cmds::export("Direct adjustments", &refs, &[], false).unwrap();
    assert!(skipped.is_empty());
    atn::parse(&bytes).unwrap().actions.remove(0).items.remove(0)
}

/// Edit a known nested fixture object without replacing its surrounding descriptor.
fn object<'a>(d: &'a mut Descriptor, key: &str) -> &'a mut Descriptor {
    match &mut d.items.iter_mut().find(|(k, _)| k.is(key)).unwrap().1 {
        D::Descriptor(d) => d,
        _ => panic!("not an object"),
    }
}

/// Expose a fixture list so negative cases change only the intended wire setting.
fn list<'a>(d: &'a mut Descriptor, key: &str) -> &'a mut Vec<D> {
    match &mut d.items.iter_mut().find(|(k, _)| k.is(key)).unwrap().1 {
        D::List(l) => l,
        _ => panic!("not a list"),
    }
}

/// Locate the first color stop for fractional-color and unsupported-setting controls.
fn first_stop(d: &mut Descriptor) -> &mut Descriptor {
    match &mut list(object(d, "Grad"), "Clrs")[0] {
        D::Descriptor(d) => d,
        _ => panic!("not a stop"),
    }
}

/// Saving retains the measured wire forms, including source colors that hex would quantize.
#[test]
fn direct_forms_save_exact_descriptors_including_fractional_rgb() {
    let mut fractional = gradient();
    object(first_stop(&mut fractional), "Clr ").items[0].1 = D::Double(73.1234567);
    let mut defaults = gradient();
    object(first_stop(&mut defaults), "Clr ").items[2].1 = D::Double(0.0);
    let g = object(&mut defaults, "Grad");
    let D::Descriptor(stop) = &mut list(g, "Clrs")[1] else { panic!("stop") };
    object(stop, "Clr ").items[1].1 = D::Double(255.0);
    object(stop, "Clr ").items[2].1 = D::Double(255.0);
    let mut implicit_classic = gradient();
    implicit_classic.items.remove(0);
    for descriptor in [defaults, gradient(), fractional, implicit_classic] {
        let s = load(*b"GrMp", descriptor.clone(), "image.adjustments.gradientMap");
        let item = saved(&s);
        assert_eq!(item.event, AtnEvent::Code(*b"GrMp"));
        assert_eq!(item.descriptor, Some(descriptor));
    }
    let mut changed = replace();
    object(&mut changed, "Mnm ").items[0].1 = D::Double(20.125);
    changed.items[0].1 = D::Integer(130);
    changed.items[3].1 = D::Integer(-120);
    changed.items[4].1 = D::Integer(45);
    changed.items[5].1 = D::Integer(-20);
    for descriptor in [replace(), changed] {
        let s = load(*b"RplC", descriptor.clone(), "image.adjustments.replaceColor");
        let item = saved(&s);
        assert_eq!(item.event, AtnEvent::Code(*b"RplC"));
        assert_eq!(item.descriptor, Some(descriptor));
    }
}

fn fixture(mode: &str, depth: u32) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width":4,"height":1,"mode":mode,"depth":depth})).unwrap();
    s.edit("original pixels", |doc, active| {
        let surface = doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap();
        let format = surface.format();
        let data = [[0.0, 0.0, 0.0, 1.0], [0.3, 0.8, 0.2, 1.0], [0.7, 0.2, 0.6, 0.4], [1.0, 1.0, 1.0, 1.0]]
            .into_iter()
            .flat_map(|p| photocraft_raster::from_rgba(&format, p))
            .collect::<Vec<_>>();
        surface.write_region(photocraft_doc::Rect::from_xywh(0, 0, 4, 1), &data);
        Ok(())
    })
    .unwrap();
    s
}

fn raw(s: &Session) -> Vec<f32> {
    let st = s.active().unwrap();
    st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap().read_region(st.doc.bounds())
}

fn play(s: &mut Session) {
    let report = s.execute("actions.play", json!({"action":"Recorded","set":"Direct adjustments"})).unwrap();
    assert_eq!(report["ran"], 1, "{report}");
    assert!(report.get("failed").is_none(), "{report}");
}

/// Execute fractional stops at every depth while preserving unselected samples and Undo.
#[test]
fn fractional_gradient_changes_pixels_and_preserves_selection_and_undo() {
    let mut descriptor = gradient();
    object(first_stop(&mut descriptor), "Clr ").items[0].1 = D::Double(73.125);
    for depth in [8, 16, 32] {
        let imported = atn_cmds::import(&bytes(*b"GrMp", descriptor.clone())).unwrap();
        assert!(imported.unsupported.is_empty());
        let mut direct = fixture("rgb", depth);
        direct.execute("select.rect", json!({"x":0,"y":0,"width":3,"height":1})).unwrap();
        let original = raw(&direct);
        direct.execute("image.adjustments.gradientMap", imported.actions[0].steps[0].1.clone()).unwrap();
        let expected = raw(&direct);
        assert_ne!(expected, original);
        assert_eq!(&expected[12..], &original[12..], "outside the selection");
        let mut replay = fixture("rgb", depth);
        replay.execute("select.rect", json!({"x":0,"y":0,"width":3,"height":1})).unwrap();
        replay.actions.list = imported.actions;
        play(&mut replay);
        assert_eq!(raw(&replay), expected);
        assert!(replay.undo());
        assert_eq!(raw(&replay), original);
        if depth == 32 {
            assert!((expected[0] - 73.125 / 255.0).abs() < 1e-6, "fractional color must not be rounded to hex");
        }
    }
}

fn set_lab_pixels(s: &mut Session) {
    s.edit("known Lab pixels", |doc, active| {
        let surface = doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap();
        surface.write_region(
            photocraft_doc::Rect::from_xywh(0, 0, 4, 1),
            &[
                0.4,
                128.0 / 255.0,
                128.0 / 255.0,
                1.0,
                0.5,
                128.0 / 255.0,
                128.0 / 255.0,
                1.0,
                0.6,
                128.0 / 255.0,
                128.0 / 255.0,
                1.0,
                0.95,
                128.0 / 255.0,
                128.0 / 255.0,
                1.0,
            ],
        );
        Ok(())
    })
    .unwrap();
}

/// Different box widths must select different pixels even when their midpoint is identical.
#[test]
fn lab_boxes_with_the_same_midpoint_have_different_real_effects() {
    // Independent mask anchors: at zero fuzziness a neutral Lab50 pixel is
    // inside both boxes, Lab40/60 only the wide one, and Lab95 is outside both.
    for depth in [8, 16, 32] {
        let mut results = Vec::new();
        for (minimum, maximum) in [(49, 51), (39, 61)] {
            let mut s = fixture("lab", depth);
            set_lab_pixels(&mut s);
            let before = raw(&s);
            s.execute("image.adjustments.replaceColor", json!({"labMinimum":[minimum,0,0],"labMaximum":[maximum,0,0],"fuzziness":0,"lightness":100})).unwrap();
            let after = raw(&s);
            assert!(after[4] > 0.99, "Lab50 must become white: {after:?}");
            assert!((after[12] - before[12]).abs() < 0.005, "Lab95 stays outside");
            if minimum == 49 {
                assert!((after[0] - before[0]).abs() < 0.005);
            } else {
                assert!(after[0] > 0.99 && after[8] > 0.99, "wide box includes Lab40/60");
            }
            results.push(after);
            assert!(s.undo());
            assert_eq!(raw(&s), before);
        }
        assert_ne!(results[0], results[1], "collapsing a box to its midpoint is incorrect");
    }
}

/// Conflicting or invalid Lab settings leave both pixels and history untouched.
#[test]
fn invalid_lab_inputs_are_rejected_before_any_edit() {
    let mut s = fixture("rgb", 32);
    for patch in [
        json!({"labMaximum":null}),
        json!({"labMinimum":[0,0]}),
        json!({"labMinimum":[1e100,0,0]}),
        json!({"labMinimum":[50.0000001,0,0],"labMaximum":[50,0,0]}),
        json!({"color":"#ff0000"}),
        json!({"points":[]}),
        json!({"localized":false}),
        json!({"target":"mask"}),
        json!({"hue":"30"}),
        json!({"layer":999999}),
    ] {
        let mut p = json!({"labMinimum":[40,0,0],"labMaximum":[60,0,0],"lightness":100});
        p.as_object_mut().unwrap().extend(patch.as_object().unwrap().clone());
        let before = s.active().unwrap().doc.clone();
        let history = s.active().unwrap().history.past_len();
        assert!(s.execute("image.adjustments.replaceColor", p.clone()).is_err(), "{p}");
        assert_eq!(s.active().unwrap().doc, before, "{p}");
        assert_eq!(s.active().unwrap().history.past_len(), history, "{p}");
    }
}

fn kept(event: [u8; 4], d: Descriptor) {
    let imported = atn_cmds::import(&bytes(event, d.clone())).unwrap();
    assert_eq!(imported.unsupported.len(), 1, "{d:?}");
    let mut s = Session::new();
    s.actions.list = imported.actions;
    assert_eq!(saved(&s).descriptor, Some(d));
}

/// Unsupported interpolation, transparency and wire types survive as complete kept steps.
#[test]
fn unsupported_gradient_settings_and_wire_forms_remain_kept() {
    let mut midpoint = gradient();
    first_stop(&mut midpoint).items[3].1 = D::Integer(30);
    let mut transparency = gradient();
    let D::Descriptor(opacity) = &mut list(object(&mut transparency, "Grad"), "Trns")[0] else { panic!("opacity") };
    opacity.items[0].1 = D::UnitFloat { unit: *b"#Prc", value: 50.0 };
    let mut interpolation = gradient();
    interpolation.items[0].1 = enumeration("gradientInterpolationMethodType", "Lnr ");
    let mut order = gradient();
    list(object(&mut order, "Grad"), "Clrs").reverse();
    let mut duplicate = gradient();
    duplicate.items.push(duplicate.items[0].clone());
    let mut unknown = gradient();
    unknown.items.push((Id::new("extra"), D::Boolean(true)));
    for d in [midpoint, transparency, interpolation, order, duplicate, unknown] {
        kept(*b"GrMp", d);
    }
    let mut f = atn::parse(&bytes(*b"GrMp", gradient())).unwrap();
    f.actions[0].items[0].event = AtnEvent::name("GrMp");
    assert_eq!(atn_cmds::import(&atn::write(&f).unwrap()).unwrap().unsupported.len(), 1);
}

/// Invalid recorded boxes stay preserved, and edited or unrepresentable native settings cannot save lossily.
#[test]
fn invalid_replace_descriptors_and_lossy_exports_are_refused() {
    let mut model = replace();
    model.items[6].1 = D::Integer(1);
    let mut inverted = replace();
    object(&mut inverted, "Mnm ").items[0].1 = D::Double(80.0);
    let mut missing = replace();
    missing.items.remove(0);
    let mut unknown = replace();
    unknown.items.push((Id::new("extra"), D::Boolean(true)));
    let mut duplicate = replace();
    duplicate.items.push(duplicate.items[0].clone());
    for d in [model, inverted, missing, unknown, duplicate] {
        kept(*b"RplC", d);
    }
    let mut s = load(*b"GrMp", gradient(), "image.adjustments.gradientMap");
    s.actions.list[0].steps[0].1["stops"][0][1][0] = json!(0.2);
    assert!(atn_cmds::export("Direct adjustments", &[&s.actions.list[0]], &[], false).is_err(), "stale color metadata must not fall through to GdMp");
    for p in [json!({"color":"#ff0000"}), json!({"labMinimum":[40,0,0],"labMaximum":[60,0,0],"hue":0.5})] {
        let a = photocraft_engine::actions_cmds::Action::new("Native", vec![("image.adjustments.replaceColor".into(), p)]);
        assert!(atn_cmds::export("Direct adjustments", &[&a], &[], false).is_err());
    }
}

/// Playback respects bounded and implicitly selected masks at every depth, including an offset selection and Undo.
#[test]
fn recorded_replace_color_clamps_shifts_and_plays_the_lab_box_at_every_depth() {
    for (depth, inverse) in [8, 16, 32].into_iter().flat_map(|depth| [(depth, false), (depth, true)]) {
        let mut d = replace();
        for key in ["Mnm ", "Mxm "] {
            let o = object(&mut d, key);
            o.items[0].1 = D::Double(if key == "Mnm " { 39.0 } else { 61.0 });
            o.items[1].1 = D::Double(0.0);
            o.items[2].1 = D::Double(0.0);
        }
        d.items[0].1 = D::Integer(-10);
        d.items[3].1 = D::Integer(0);
        d.items[5].1 = D::Integer(250);
        let imported = atn_cmds::import(&bytes(*b"RplC", d)).unwrap();
        assert!(imported.unsupported.is_empty());
        assert_eq!(imported.actions[0].steps[0].1["fuzziness"], 0);
        assert_eq!(imported.actions[0].steps[0].1["lightness"], 100);
        let mut s = fixture("lab", depth);
        set_lab_pixels(&mut s);
        s.execute("select.rect", json!({"x":1,"y":0,"width":1,"height":1})).unwrap();
        if inverse {
            let mut mask = photocraft_raster::Surface::with_default(photocraft_doc::PixelFormat::GRAY8, &[1.0]);
            mask.fill_rect(photocraft_doc::Rect::new(1, 0, 2, 1), &[0.0]);
            std::sync::Arc::make_mut(&mut s.active_mut().unwrap().doc).selection = Some(mask);
        }
        let before = raw(&s);
        s.actions.list = imported.actions;
        play(&mut s);
        let after = raw(&s);
        if inverse {
            assert!(after[0] > 0.99 && after[8] > 0.99);
            assert_eq!(&after[4..8], &before[4..8], "the selection hole must stay unchanged");
        } else {
            assert!(after[4] > 0.99);
            assert_eq!(&after[..4], &before[..4], "pixels before the selection must stay unchanged");
            assert_eq!(&after[8..], &before[8..], "pixels after the selection must stay unchanged");
        }
        assert!(s.undo());
        assert_eq!(raw(&s), before);
        let descriptor = saved(&s).descriptor.unwrap();
        assert_eq!(descriptor.get("Fzns"), Some(&D::Integer(0)));
        assert_eq!(descriptor.get("Lght"), Some(&D::Integer(100)));
    }
}

/// Matching uses original Lab/CMYK channels and tagged RGB rather than display-converted samples.
#[test]
fn lab_matching_uses_native_channels_and_the_embedded_rgb_profile() {
    use photocraft_cms::{Builtin, Transform, TransformOptions};
    for depth in [8, 16, 32] {
        for (mode, profile) in [("rgb", Builtin::DisplayP3), ("gray", Builtin::SGray), ("cmyk", Builtin::CoatedCmyk), ("lab", Builtin::LabD50)] {
            let mut s = fixture(mode, depth);
            s.edit("explicit profile", |doc, _| {
                doc.icc_profile = Some(profile.profile().clone().with_encoded_bytes().to_bytes());
                Ok(())
            })
            .unwrap();
            let st = s.active().unwrap();
            let surface = st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap();
            let format = surface.format();
            let before = raw(&s);
            // Independent direct transform from this explicitly chosen source profile,
            // not the command's profile resolver or its new matching helper.
            let t = Transform::with_options(profile.profile(), Builtin::LabD50.profile(), TransformOptions::default()).unwrap();
            let mut lab = [[0.0, 0.0, 0.0, 1.0]; 4];
            t.convert_f32(&before, format.channels(), lab.as_flattened_mut(), 4, true);
            let center = [lab[1][0] as f64 * 100.0, lab[1][1] as f64 * 255.0 - 128.0, lab[1][2] as f64 * 255.0 - 128.0];
            let lo = center.map(|v| v - 0.2);
            let hi = center.map(|v| v + 0.2);
            s.execute("image.adjustments.replaceColor", json!({"labMinimum":lo,"labMaximum":hi,"fuzziness":0,"lightness":100})).unwrap();
            let after = raw(&s);
            let changed = after.chunks_exact(format.channels()).nth(1).unwrap();
            let changed = photocraft_raster::to_rgba(&format, changed);
            assert!(changed[..3].iter().all(|v| *v > 0.97), "{mode}/{depth}: {changed:?}");
            assert_eq!(&after[..format.channels()], &before[..format.channels()], "black is outside {mode}/{depth}");
            assert!(s.undo());
            assert_eq!(raw(&s), before);
            if mode == "rgb" {
                let mut srgb = fixture("rgb", depth);
                let before_srgb = raw(&srgb);
                srgb.execute("image.adjustments.replaceColor", json!({"labMinimum":lo,"labMaximum":hi,"fuzziness":0,"lightness":100})).unwrap();
                assert_eq!(raw(&srgb), before_srgb, "the P3 sample must not be mistaken for an sRGB sample");
            }
        }
    }
}

/// Full and selected edits agree across tile edges, including the final partial tiles and Undo.
#[test]
fn lab_replace_crosses_tile_edges_without_changing_unselected_native_pixels() {
    let tile = photocraft_geom::TILE_SIZE;
    let area = photocraft_doc::Rect::new(0, 0, tile + 4, tile + 3);
    let window = photocraft_doc::Rect::new(tile - 1, tile - 1, tile + 2, tile + 2);
    let width = area.width() as usize;
    for depth in [8, 16, 32] {
        for selected in ["none", "window", "outside"] {
            let mut s = fixture("lab", depth);
            s.edit("Known Lab tiles", |doc, active| {
                doc.size = area.size();
                doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap().fill_rect(area, &[0.5, 128.0 / 255.0, 128.0 / 255.0, 1.0]);
                if selected != "none" {
                    let default = if selected == "outside" { 1.0 } else { 0.0 };
                    let mut mask = photocraft_raster::Surface::with_default(photocraft_doc::PixelFormat::GRAY8, &[default]);
                    mask.fill_rect(window, &[1.0 - default]);
                    doc.selection = Some(mask);
                }
                Ok(())
            })
            .unwrap();
            let before = s.active().unwrap().doc.clone();
            let original = raw(&s);
            s.execute("image.adjustments.replaceColor", json!({"labMinimum":[49,0,0],"labMaximum":[51,0,0],"fuzziness":0,"lightness":100})).unwrap();
            let after = raw(&s);
            for (i, (pixel, original)) in after.as_chunks::<4>().0.iter().zip(original.as_chunks::<4>().0).enumerate() {
                let inside = window.contains((i % width) as i32, (i / width) as i32);
                let changed = selected == "none" || (selected == "window") == inside;
                if changed {
                    assert!(pixel[0] > 0.99, "{depth}/{selected} pixel {i}: {pixel:?}");
                } else {
                    assert_eq!(pixel, original, "{depth}/{selected} pixel {i}");
                }
                assert_eq!(pixel[3], original[3]);
            }
            assert!(s.undo());
            assert_eq!(s.active().unwrap().doc, before);
        }
    }
}

/// Matching stored pixels change without materializing gaps whose default does not change.
#[test]
fn lab_replace_sparse_pixels_preserve_unstored_gaps() {
    let tile = photocraft_geom::TILE_SIZE;
    let far = photocraft_doc::Rect::new(4 * tile, 4 * tile, 4 * tile + 1, 4 * tile + 1);
    let second = far.translate(2 * tile, 0);
    let gap = far.translate(tile, 0);
    for (alpha, opacity) in [(true, 0.0), (true, 1.0), (false, 1.0)] {
        let mut s = fixture("lab", 32);
        s.edit("Two distant Lab pixels", |doc, active| {
            let surface = doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap();
            let format = photocraft_doc::PixelFormat { alpha, ..surface.format() };
            let channels = format.channels();
            *surface = photocraft_raster::Surface::with_default(format, &[0.25, 128.0 / 255.0, 128.0 / 255.0, opacity][..channels]);
            for rect in [far, second] {
                surface.fill_rect(rect, &[0.5, 128.0 / 255.0, 128.0 / 255.0, 1.0][..channels]);
            }
            Ok(())
        })
        .unwrap();
        let before = s.active().unwrap().doc.clone();
        let layer = s.active().unwrap().active_layer.unwrap();
        let original = before.layer(layer).unwrap().surface().unwrap();
        assert_eq!(original.tile_count(), 2);
        s.execute("image.adjustments.replaceColor", json!({"labMinimum":[49,0,0],"labMaximum":[51,0,0],"fuzziness":0,"lightness":100})).unwrap();
        let surface = s.active().unwrap().doc.layer(layer).unwrap().surface().unwrap();
        for rect in [far, second] {
            assert!(surface.read_region(rect)[0] > 0.99);
        }
        assert_eq!(surface.read_region(gap), original.read_region(gap));
        assert_eq!(surface.default_pixel(), original.default_pixel());
        assert_eq!(surface.tile_count(), 2);
        assert!(s.undo());
        assert_eq!(s.active().unwrap().doc, before);
    }
}

/// Opaque defaults remain visible when the surface has no tiles or only a small exception.
#[test]
fn lab_replace_includes_visible_default_pixels_with_and_without_alpha() {
    let tile = photocraft_geom::TILE_SIZE;
    let far = photocraft_doc::Rect::new(2 * tile + 1, 2 * tile + 1, 2 * tile + 2, 2 * tile + 2);
    for depth in [8, 16, 32] {
        for alpha in [false, true] {
            for exception in [None, Some(photocraft_doc::Rect::new(1, 0, 2, 1)), Some(far)] {
                let mut s = fixture("lab", depth);
                s.edit("Opaque default Lab", |doc, active| {
                    let surface = doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap();
                    let format = photocraft_doc::PixelFormat { alpha, ..surface.format() };
                    let channels = format.channels();
                    *surface = photocraft_raster::Surface::with_default(format, &[0.5, 128.0 / 255.0, 128.0 / 255.0, 1.0][..channels]);
                    if let Some(rect) = exception {
                        surface.fill_rect(rect, &[0.25, 128.0 / 255.0, 128.0 / 255.0, 1.0][..channels]);
                    } else {
                        assert!(surface.content_bounds().is_empty());
                    }
                    Ok(())
                })
                .unwrap();
                let before = s.active().unwrap().doc.clone();
                let original = raw(&s);
                s.execute("image.adjustments.replaceColor", json!({"labMinimum":[49,0,0],"labMaximum":[51,0,0],"fuzziness":0,"lightness":100})).unwrap();
                for (i, (pixel, original)) in raw(&s).chunks_exact(3 + usize::from(alpha)).zip(original.chunks_exact(3 + usize::from(alpha))).enumerate() {
                    if exception == Some(photocraft_doc::Rect::new(1, 0, 2, 1)) && i == 1 {
                        assert_eq!(pixel, original);
                    } else {
                        assert!(pixel[0] > 0.99, "{depth}/{alpha}/{exception:?} pixel {i}: {pixel:?}");
                    }
                }
                if exception == Some(far) {
                    let layer = s.active().unwrap().active_layer.unwrap();
                    let surface = s.active().unwrap().doc.layer(layer).unwrap().surface().unwrap();
                    assert!(surface.read_region(far.translate(-tile, 0))[0] > 0.99, "off-canvas defaults inside the edit extent change too");
                    assert_eq!(surface.read_region(far), before.layer(layer).unwrap().surface().unwrap().read_region(far));
                    assert_eq!(surface.read_region(far.translate(2, 2)), surface.default_pixel(), "pixels outside the extent stay unchanged");
                }
                assert!(s.undo());
                assert_eq!(s.active().unwrap().doc, before);
            }
        }
    }
}

/// Selection islands and holes retain their coverage over unstored opaque pixels.
#[test]
fn lab_replace_selection_controls_opaque_gap_pixels() {
    let tile = photocraft_geom::TILE_SIZE;
    let far = photocraft_doc::Rect::new(2 * tile, 2 * tile, 2 * tile + 1, 2 * tile + 1);
    let gap = far.translate(-tile, -tile);
    for implicit in [false, true] {
        let mut s = fixture("lab", 32);
        s.edit("Selected opaque gap", |doc, active| {
            let surface = doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap();
            *surface = photocraft_raster::Surface::with_default(surface.format(), &[0.5, 128.0 / 255.0, 128.0 / 255.0, 1.0]);
            surface.fill_rect(far, &[0.25, 128.0 / 255.0, 128.0 / 255.0, 1.0]);
            let coverage = if implicit { 1.0 } else { 0.0 };
            let mut mask = photocraft_raster::Surface::with_default(photocraft_doc::PixelFormat::GRAY8, &[coverage]);
            mask.fill_rect(gap, &[1.0 - coverage]);
            doc.selection = Some(mask);
            Ok(())
        })
        .unwrap();
        let before = s.active().unwrap().doc.clone();
        let layer = s.active().unwrap().active_layer.unwrap();
        let original = before.layer(layer).unwrap().surface().unwrap();
        s.execute("image.adjustments.replaceColor", json!({"labMinimum":[49,0,0],"labMaximum":[51,0,0],"fuzziness":0,"lightness":100})).unwrap();
        let surface = s.active().unwrap().doc.layer(layer).unwrap().surface().unwrap();
        let canvas_pixel = photocraft_doc::Rect::new(0, 0, 1, 1);
        for (rect, changed) in [(gap, !implicit), (canvas_pixel, implicit)] {
            if changed {
                assert!(surface.read_region(rect)[0] > 0.99);
            } else {
                assert_eq!(surface.read_region(rect), original.read_region(rect));
            }
        }
        assert_eq!(surface.read_region(far), original.read_region(far));
        assert_eq!(surface.default_pixel(), original.default_pixel());
        if !implicit {
            assert_eq!(surface.tile_count(), 2, "only the selected gap tile is added");
        }
        assert!(s.undo());
        assert_eq!(s.active().unwrap().doc, before);
    }
}

/// Existing RGB replacement and ordinary gradient export retain their native behavior.
#[test]
fn legacy_rgb_replace_color_and_ordinary_gradient_exports_are_unchanged() {
    let mut s = fixture("rgb", 32);
    let before = raw(&s);
    let mut expected = before.as_chunks::<4>().0.to_vec();
    photocraft_algo::tone::replace_color(&mut expected, [0.3, 0.8, 0.2], 40.0, 60.0, 20.0, -10.0);
    s.execute("image.adjustments.replaceColor", json!({"color":[0.3,0.8,0.2],"fuzziness":40,"hue":60,"saturation":20,"lightness":-10})).unwrap();
    assert_eq!(raw(&s), expected.as_flattened());
    let action = photocraft_engine::actions_cmds::Action::new(
        "Ordinary",
        vec![("image.adjustments.gradientMap".into(), json!({"stops":[[0,"#000000"],[1,"#ffffff"]]}))],
    );
    let (b, _) = atn_cmds::export("Ordinary", &[&action], &[], false).unwrap();
    assert_eq!(
        atn::parse(&b).unwrap().actions[0].items[0].event,
        AtnEvent::name("gradientMapEvent"),
        "an ordinary Gradient Map saves as the string event (#10702)"
    );
}

/// Replacement preserves alpha and refuses locked or profile-incompatible surfaces before editing.
#[test]
fn lab_replace_preserves_alpha_and_rejects_locked_or_mismatched_surfaces() {
    let p = json!({"labMinimum":[0,-128,-128],"labMaximum":[100,127,127],"lightness":100});
    let mut s = fixture("rgb", 32);
    let before = raw(&s);
    s.execute("image.adjustments.replaceColor", p.clone()).unwrap();
    for (a, b) in raw(&s).as_chunks::<4>().0.iter().zip(before.as_chunks::<4>().0) {
        assert_eq!(a[3], b[3]);
    }
    assert!(s.undo());
    s.edit("pixel lock", |doc, active| {
        doc.layer_mut(active.unwrap()).unwrap().locks.pixels = true;
        Ok(())
    })
    .unwrap();
    let original = s.active().unwrap().doc.clone();
    assert!(s.execute("image.adjustments.replaceColor", p.clone()).is_err());
    assert_eq!(s.active().unwrap().doc, original);
    let mut mismatch = fixture("rgb", 32);
    mismatch
        .edit("inconsistent synthetic mode", |doc, _| {
            doc.mode = photocraft_doc::ColorMode::Cmyk;
            Ok(())
        })
        .unwrap();
    let original = mismatch.active().unwrap().doc.clone();
    assert!(mismatch.execute("image.adjustments.replaceColor", p).is_err());
    assert_eq!(mismatch.active().unwrap().doc, original);
}

/// Fractional selection mixing changes RGB while preserving each F32 alpha bit exactly.
#[test]
fn lab_replace_fractional_selection_preserves_f32_alpha_bits() {
    for coverage in [53.0 / 255.0, 127.0 / 255.0] {
        let mut s = fixture("rgb", 32);
        s.edit("fractional alpha and selection", |doc, active| {
            doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap().write_region(photocraft_doc::Rect::new(2, 0, 3, 1), &[0.7, 0.2, 0.6, 0.7]);
            doc.selection = Some(photocraft_raster::Surface::with_default(photocraft_doc::PixelFormat::GRAY8, &[coverage]));
            Ok(())
        })
        .unwrap();
        let before = s.active().unwrap().doc.clone();
        let original = raw(&s);
        s.execute("image.adjustments.replaceColor", json!({"labMinimum":[0,-128,-128],"labMaximum":[100,127,127],"lightness":100})).unwrap();
        let after = raw(&s);
        assert_ne!(&after[8..11], &original[8..11]);
        for (after, before) in after.as_chunks::<4>().0.iter().zip(original.as_chunks::<4>().0) {
            assert_eq!(after[3].to_bits(), before[3].to_bits(), "coverage {coverage}");
        }
        assert!(s.undo());
        assert_eq!(s.active().unwrap().doc, before);
    }
}

/// A zero shift must not alter native ink values through an unnecessary RGB separation.
#[test]
fn an_unchanged_lab_tone_shift_preserves_native_cmyk_samples() {
    for depth in [8, 16, 32] {
        let mut s = fixture("cmyk", depth);
        s.edit("nontrivial native CMYK", |doc, active| {
            doc.layer_mut(active.unwrap())
                .unwrap()
                .surface_mut()
                .unwrap()
                .write_region(photocraft_doc::Rect::from_xywh(1, 0, 1, 1), &[0.2, 0.3, 0.4, 0.25, 1.0]);
            Ok(())
        })
        .unwrap();
        let before = raw(&s);
        s.execute("image.adjustments.replaceColor", json!({"labMinimum":[0,-128,-128],"labMaximum":[100,127,127],"hue":0,"saturation":0,"lightness":0}))
            .unwrap();
        assert_eq!(raw(&s), before, "an unchanged tone operation must not re-separate CMYK, depth {depth}");
    }
}
