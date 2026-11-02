//! Recorded identifiers must match Photoshop, even when a local round trip would accept a typo.

use photocraft_engine::{Session, actions_cmds::Action, atn_cmds};
use photocraft_psd::{
    atn::{self, AtnAction, AtnEvent, AtnFile, AtnItem},
    descriptor::{Class, Descriptor, Id, ReferenceItem, UnicodeString, Value as D},
};
use serde_json::json;

/// A small marked document makes canvas translation and image resampling distinguishable.
fn sized_document(width: u32, height: u32, dpi: u32) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width":width,"height":height,"background":"white"})).unwrap();
    s.execute("image.imageSize", json!({"resolution":dpi,"resample":"none"})).unwrap();
    s.edit("Mark", |doc, _| {
        doc.layers[0].surface_mut().unwrap().write_region(photocraft_geom::Rect::from_xywh(0, 0, 1, 1), &[1.0, 0.0, 0.0, 1.0]);
        Ok(())
    })
    .unwrap();
    s
}

/// Replay saved bytes, so the geometry witness covers import, export and execution together.
fn replay_saved(s: &mut Session, action: &Action) {
    let (bytes, _) = atn_cmds::export("Identifiers", &[action], &[], false).unwrap();
    let imported = atn_cmds::import(&bytes).unwrap();
    assert!(imported.unsupported.is_empty());
    assert_eq!(imported.actions[0].steps, action.steps);
    s.actions.list = imported.actions;
    let result = s.execute("actions.play", json!({"action":"Recorded"})).unwrap();
    assert_eq!(result["ran"], 1);
    assert!(result.get("failed").is_none(), "{result}");
}

/// Percent canvas dimensions use each playback document and leave its pixels unscaled.
#[test]
fn recorded_canvas_percent_resolves_at_playback_and_keeps_an_omitted_side() {
    for both_sides in [false, true] {
        let mut d = Descriptor::new("null")
            .with("Wdth", D::UnitFloat { unit: *b"#Prc", value: 200.0 })
            .with("Hrzn", enm("HrzL", "Left"))
            .with("Vrtc", enm("VrtL", "Top "));
        if both_sides {
            d = d.with("Hght", D::UnitFloat { unit: *b"#Prc", value: 200.0 });
        }
        let action = recorded(AtnEvent::Code(*b"CnvS"), d);
        assert_eq!(action.steps[0].0, "image.canvasSize", "recorded canvas percentages must map");
        assert_eq!(action.steps[0].1["units"], "percent");
        assert!(matches!(saved(&action).descriptor.unwrap().get("Wdth"), Some(D::UnitFloat {unit,value}) if unit==b"#Prc" && *value==200.0));
        for (width, height) in [(40, 30), (10, 15)] {
            let mut s = sized_document(width, height, 72);
            replay_saved(&mut s, &action);
            assert_eq!(s.active().unwrap().doc.size, photocraft_geom::Size::new(width * 2, if both_sides { height * 2 } else { height }));
            assert_eq!(s.execute("document.pixel", json!({"x":0,"y":0})).unwrap(), json!([1.0, 0.0, 0.0, 1.0]));
            assert_eq!(s.execute("document.pixel", json!({"x":1,"y":0})).unwrap(), json!([1.0, 1.0, 1.0, 1.0]), "canvas expansion must not stretch the mark");
            s.execute("edit.undo", json!({})).unwrap();
            assert_eq!(s.active().unwrap().doc.size, photocraft_geom::Size::new(width, height));
        }
    }
}

/// A missing resampler means print-size editing, which the native point-size resize cannot express.
#[test]
fn review_point_resize_without_interpolation_stays_kept() {
    for key in ["Wdth", "Hght"] {
        for interpolation in [None, Some(D::Boolean(false)), Some(enm("wrong", "Nrst"))] {
            let mut d = Descriptor::new("null").with(key, D::UnitFloat { unit: *b"#Rlt", value: 72.0 });
            if let Some(value) = interpolation {
                d = d.with("Intr", value);
            }
            let action = recorded(AtnEvent::Code(*b"ImgS"), d.clone());
            assert_eq!(action.steps[0].0, atn_cmds::PHOTOSHOP_STEP, "point sizes without a valid Intr must stay unsupported");
            assert_eq!(saved(&action).descriptor, Some(d));
        }
    }
}

/// Null optional values have the same export semantics as omitted values.
#[test]
fn review_point_resize_exports_with_null_optional_values() {
    for extras in [json!({"resolution":null}), json!({"percent":null}), json!({"resolution":null,"percent":null})] {
        let mut params = json!({"width":72,"units":"pt","resample":"nearest","scaleStyles":false});
        params.as_object_mut().unwrap().extend(extras.as_object().unwrap().clone());
        let action = Action::new("Recorded", vec![("image.imageSize".into(), params)]);
        let (bytes, skipped) = atn_cmds::export("Points", &[&action], &[], false).expect("null values must not prevent point-size export");
        assert!(skipped.is_empty());
        let imported = atn_cmds::import(&bytes).unwrap();
        assert!(imported.unsupported.is_empty());
        assert_eq!(imported.actions[0].steps[0].1, json!({"width":72,"units":"pt","resample":"nearest","scaleStyles":false}));
    }
}

/// The native default resampler must be explicit in a physical-size recording.
#[test]
fn review_point_resize_exports_the_default_resampler() {
    for extras in [json!({}), json!({"resample":null})] {
        let mut params = json!({"width":72,"units":"pt","scaleStyles":false});
        params.as_object_mut().unwrap().extend(extras.as_object().unwrap().clone());
        let action = Action::new("Recorded", vec![("image.imageSize".into(), params.clone())]);
        let (bytes, skipped) = atn_cmds::export("Points", &[&action], &[], false).unwrap();
        assert!(skipped.is_empty());
        let imported = atn_cmds::import(&bytes).unwrap();
        assert!(imported.unsupported.is_empty(), "a point-size export must include its native default resampler");
        assert_eq!(imported.actions[0].steps[0].1["resample"], "bicubic");
        let mut expected = sized_document(40, 20, 144);
        expected.execute("image.imageSize", params).unwrap();
        let mut actual = sized_document(40, 20, 144);
        actual.actions.list = imported.actions;
        let result = actual.execute("actions.play", json!({"action":"Recorded"})).unwrap();
        assert!(result.get("failed").is_none(), "{result}");
        let (a, b) = (&actual.active().unwrap().doc, &expected.active().unwrap().doc);
        assert_eq!(a.size, b.size);
        assert_eq!(a.layers[0].surface().unwrap().read_region(a.bounds()), b.layers[0].surface().unwrap().read_region(b.bounds()));
    }
}

/// Invalid metadata must keep the whole Copy/Cut event, before a layer or source pixels can change.
#[test]
fn review_copy_cut_opacity_is_validated_before_any_step_runs() {
    for copy in [true, false] {
        for opacity in [D::Double(-1.0), D::Double(101.0), D::Double(f64::NAN), D::Boolean(true), D::UnitFloat { unit: *b"#Pxl", value: 50.0 }] {
            let d = Descriptor::new("null")
                .with("Nw  ", D::Descriptor(Descriptor::new("Lyr ").with("Opct", opacity)))
                .with("Usng", enm("UsrM", "Slct"))
                .with("Cpy ", D::Boolean(copy));
            let action = recorded(AtnEvent::Code(*b"Mk  "), d);
            assert_eq!(action.steps[0].0, atn_cmds::PHOTOSHOP_STEP, "invalid opacity must keep Copy/Cut before mutation");
            let mut s = sized_document(8, 6, 72);
            s.execute("select.rect", json!({"x":0,"y":0,"width":2,"height":2})).unwrap();
            let before = s.active().unwrap().doc.clone();
            s.actions.list = vec![action];
            let result = s.execute("actions.play", json!({"action":"Recorded"})).unwrap();
            assert!(result.get("failed").is_some());
            assert_eq!(s.active().unwrap().doc, before);
        }
        for opacity in [0.0, 50.0, 100.0] {
            let d = Descriptor::new("null")
                .with("Nw  ", D::Descriptor(Descriptor::new("Lyr ").with("Opct", D::Double(opacity))))
                .with("Usng", enm("UsrM", "Slct"))
                .with("Cpy ", D::Boolean(copy));
            let action = recorded(AtnEvent::Code(*b"Mk  "), d);
            assert!(action.steps.iter().all(|(id, _)| id != atn_cmds::PHOTOSHOP_STEP));
            assert_eq!(action.steps.iter().find(|(id, _)| id == "layer.setProps").unwrap().1["opacity"], opacity / 100.0);
        }
    }
}

/// Every named layer anchor must affect actual playback geometry, not just the saved parameters.
#[test]
fn review_layer_transform_uses_the_recorded_anchor() {
    for depth in [8, 16, 32] {
        for (code, fx, fy) in
            [("Qcs0", 0, 0), ("Qcs4", 1, 0), ("Qcs1", 2, 0), ("Qcs7", 0, 1), ("Qcsa", 1, 1), ("Qcs5", 2, 1), ("Qcs3", 0, 2), ("Qcs6", 1, 2), ("Qcs2", 2, 2)]
        {
            let d = Descriptor::new("null")
                .with("FTcs", enm("QCSt", code))
                .with(
                    "Ofst",
                    D::Descriptor(
                        Descriptor::new("Ofst")
                            .with("Hrzn", D::UnitFloat { unit: *b"#Pxl", value: 0.0 })
                            .with("Vrtc", D::UnitFloat { unit: *b"#Pxl", value: 0.0 }),
                    ),
                )
                .with("Wdth", D::UnitFloat { unit: *b"#Prc", value: 50.0 })
                .with("Hght", D::UnitFloat { unit: *b"#Prc", value: 50.0 })
                .with("Intr", enm("Intp", "Nrst"));
            let action = recorded(AtnEvent::Code(*b"Trnf"), d);
            let mut s = Session::new();
            s.execute("file.new", json!({"width":64,"height":64,"depth":depth})).unwrap();
            s.execute("layer.new.layer", json!({})).unwrap();
            s.edit("Mark", |doc, active| {
                doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap().fill_rect(photocraft_geom::Rect::new(20, 20, 40, 40), &[1.0, 0.0, 0.0, 1.0]);
                Ok(())
            })
            .unwrap();
            replay_saved(&mut s, &action);
            let state = s.active().unwrap();
            let bounds = state.doc.layer(state.active_layer.unwrap()).unwrap().surface().unwrap().content_bounds();
            assert_eq!(bounds, photocraft_geom::Rect::new(20 + 5 * fx, 20 + 5 * fy, 30 + 5 * fx, 30 + 5 * fy), "{code}, depth {depth}");
            // Measured in the reference application: Qcs6 keeps the bottom centre fixed, Qcs1 the
            // top-right corner.
            match code {
                "Qcs6" => assert_eq!(bounds, photocraft_geom::Rect::new(25, 30, 35, 40), "depth {depth}"),
                "Qcs1" => assert_eq!(bounds, photocraft_geom::Rect::new(30, 20, 40, 30), "depth {depth}"),
                _ => {}
            }
            s.execute("edit.undo", json!({})).unwrap();
            if code == "Qcs0" {
                s.edit("Mark rotation", |doc, active| {
                    doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap().write_pixel(22, 30, &[0.0, 1.0, 0.0, 1.0]);
                    Ok(())
                })
                .unwrap();
                s.execute("edit.transform", json!({"rotate":90,"reference":"topLeft","interpolation":"nearest"})).unwrap();
                let state = s.active().unwrap();
                assert_eq!(
                    state.doc.layer(state.active_layer.unwrap()).unwrap().surface().unwrap().content_bounds(),
                    photocraft_geom::Rect::new(0, 20, 20, 40)
                );
                assert_eq!(state.doc.layer(state.active_layer.unwrap()).unwrap().surface().unwrap().rgba(9, 22), [0.0, 1.0, 0.0, 1.0]);
                s.execute("edit.undo", json!({})).unwrap();
            }
            let before = s.active().unwrap().doc.clone();
            assert!(s.execute("edit.transform", json!({"scaleX":50,"reference":"unknown"})).is_err());
            assert_eq!(s.active().unwrap().doc, before);
        }
    }
}

/// Physical dimensions retain points and resolve current dpi before rounding or style scaling.
#[test]
fn recorded_image_points_use_current_resolution_and_keep_style_scaling() {
    use photocraft_color::{BlendMode, Color};
    use photocraft_doc::{Effect, Effects, FxCommon, FxPaint, StrokeFx, StrokePosition};
    for dpi in [72, 144] {
        for scale_styles in [None, Some(false), Some(true)] {
            let mut d = Descriptor::new("null")
                .with("Wdth", D::UnitFloat { unit: *b"#Rlt", value: 72.0 })
                .with("CnsP", D::Boolean(true))
                .with("Intr", enm("Intp", "Nrst"));
            if let Some(value) = scale_styles {
                d = d.with("scaleStyles", D::Boolean(value));
            }
            let action = recorded(AtnEvent::Code(*b"ImgS"), d);
            assert_eq!(action.steps[0].0, "image.imageSize", "recorded point dimensions must map");
            assert_eq!(action.steps[0].1["units"], "pt");
            assert_eq!(action.steps[0].1["width"], 72);
            assert!(matches!(saved(&action).descriptor.unwrap().get("Wdth"), Some(D::UnitFloat {unit,value}) if unit==b"#Rlt" && *value==72.0));
            let mut s = sized_document(40, 20, dpi);
            s.edit("Style", |doc, _| {
                doc.layers[0].effects = Effects {
                    enabled: true,
                    items: vec![Effect::Stroke(StrokeFx {
                        common: FxCommon::new(BlendMode::Normal, 1.0),
                        size: 20.0,
                        position: StrokePosition::Outside,
                        paint: FxPaint::Color(Color::rgb(0.0, 0.0, 1.0)),
                        overprint: false,
                    })],
                    ..Default::default()
                };
                Ok(())
            })
            .unwrap();
            replay_saved(&mut s, &action);
            let doc = &s.active().unwrap().doc;
            assert_eq!(doc.size, photocraft_geom::Size::new(dpi, dpi / 2));
            let Effect::Stroke(stroke) = &doc.layers[0].effects.items[0] else { panic!("stroke lost") };
            assert_eq!(stroke.size, if scale_styles.unwrap_or(false) { 20.0 * dpi as f32 / 40.0 } else { 20.0 });
            s.execute("edit.undo", json!({})).unwrap();
            assert_eq!(s.active().unwrap().doc.size, photocraft_geom::Size::new(40, 20));
        }
    }
    let d = Descriptor::new("null").with("Wdth", D::UnitFloat { unit: *b"#Rlt", value: 0.36 }).with("Intr", enm("Intp", "Nrst"));
    let action = recorded(AtnEvent::Code(*b"ImgS"), d);
    let mut s = sized_document(40, 20, 300);
    replay_saved(&mut s, &action);
    assert_eq!(s.active().unwrap().doc.size, photocraft_geom::Size::new(2, 1), "round pixels after point conversion");
}

/// The layered false/false recording writes a class and explicit layer-retaining choices.
#[test]
fn recorded_mode_uses_a_class_and_retains_layers_with_false_flags() {
    let d = Descriptor::new("null").with("T   ", D::Class(class("Grys"))).with("Fltt", D::Boolean(false)).with("Rstr", D::Boolean(false));
    let action = recorded(AtnEvent::Code(*b"CnvM"), d);
    assert_eq!(action.steps, vec![("image.mode.grayscale".into(), json!({}))]);
    let written = saved(&action).descriptor.unwrap();
    assert!(matches!(written.get("T   "),Some(D::Class(c)) if c.class_id.is("Grys")), "mode target must save as a class");
    assert_eq!(written.get("Fltt"), Some(&D::Boolean(false)));
    assert_eq!(written.get("Rstr"), Some(&D::Boolean(false)));
    let mut s = sized_document(4, 2, 72);
    s.execute("layer.new.layer", json!({"name":"Second"})).unwrap();
    replay_saved(&mut s, &action);
    assert_eq!(s.active().unwrap().doc.mode, photocraft_doc::ColorMode::Grayscale);
    assert_eq!(s.active().unwrap().doc.layers.len(), 2);
}

/// Unsupported dimension combinations and destructive mode choices must not become different edits.
#[test]
fn unrepresented_units_and_mode_choices_stay_verbatim() {
    let point = || Descriptor::new("null").with("Wdth", D::UnitFloat { unit: *b"#Rlt", value: 72.0 });
    let mode = || Descriptor::new("null").with("T   ", D::Class(class("Grys")));
    let canvas = || Descriptor::new("null").with("Wdth", D::UnitFloat { unit: *b"#Prc", value: 200.0 });
    for (event, d) in [
        (*b"CnvM", mode().with("Fltt", D::Boolean(true))),
        (*b"CnvM", mode().with("Rstr", D::Boolean(true))),
        (*b"CnvM", mode().with("Rstr", D::Integer(0))),
        (*b"CnvS", canvas().with("Hght", D::UnitFloat { unit: *b"#Pxl", value: 40.0 })),
        (*b"CnvS", canvas().with("Rltv", D::Boolean(true))),
        (*b"ImgS", point().with("Hght", D::UnitFloat { unit: *b"#Pxl", value: 40.0 })),
        (*b"ImgS", point().with("Rslt", D::UnitFloat { unit: *b"#Rsl", value: 144.0 })),
        (*b"ImgS", point().with("CnsP", D::Boolean(false))),
    ] {
        let action = recorded(AtnEvent::Code(event), d.clone());
        assert_eq!(action.steps[0].0, atn_cmds::PHOTOSHOP_STEP);
        assert_eq!(saved(&action).descriptor, Some(d));
    }
}

/// Explicit false layer flags mean the same thing as omitted flags for a bit-depth conversion.
#[test]
fn review_10702_depth_conversion_accepts_false_layer_flags() {
    for (depth, command) in [(8, "image.mode.bits8"), (16, "image.mode.bits16"), (32, "image.mode.bits32")] {
        for flags in [vec![], vec!["Fltt"], vec!["Rstr"], vec!["Fltt", "Rstr"]] {
            let mut d = Descriptor::new("null").with("Dpth", D::Integer(depth));
            for key in flags {
                d = d.with(key, D::Boolean(false));
            }
            let action = recorded(AtnEvent::Code(*b"CnvM"), d);
            assert_eq!(action.steps, vec![(command.into(), json!({}))]);
            let mut s = sized_document(2, 1, 72);
            s.execute("layer.new.layer", json!({"name":"Retained"})).unwrap();
            replay_saved(&mut s, &action);
            assert_eq!(s.active().unwrap().doc.layers.len(), 2);
            assert_eq!(s.active().unwrap().doc.depth.bits(), depth as u32);
        }
    }
    for key in ["Fltt", "Rstr"] {
        for value in [D::Boolean(true), D::Integer(0), D::Text(UnicodeString::new_nul("false"))] {
            let d = Descriptor::new("null").with("Dpth", D::Integer(16)).with(key, value);
            let action = recorded(AtnEvent::Code(*b"CnvM"), d.clone());
            assert_eq!(action.steps[0].0, atn_cmds::PHOTOSHOP_STEP);
            assert_eq!(saved(&action).descriptor, Some(d));
        }
    }
}

/// A recorded name is resolved at playback, including names that collide with native sentinels.
#[test]
fn review_10702_transparency_names_resolve_at_playback() {
    use photocraft_doc::LayerId;
    use photocraft_geom::Rect;

    for name in ["Subject", "merged", ""] {
        let d = Descriptor::new("null").with("null", D::Reference(vec![ReferenceItem::Property { class: class("Chnl"), key: Id::new("fsel") }])).with(
            "T   ",
            D::Reference(vec![
                ReferenceItem::Enumerated { class: class("Chnl"), type_id: Id::new("Chnl"), value: Id::new("Trsp") },
                ReferenceItem::Name { class: class("Lyr "), name: UnicodeString::new_nul(name) },
            ]),
        );
        let action = recorded(AtnEvent::Code(*b"setd"), d.clone());
        assert_eq!(action.steps[0].0, "select.loadSelection");
        let mut s = sized_document(2, 1, 72);
        for alpha in [[1.0, 0.0], [0.0, 1.0]] {
            let id = LayerId(s.execute("layer.new.layer", json!({"name":"Placeholder"})).unwrap()["layer"].as_u64().unwrap());
            s.edit("Fixture", |doc, _| {
                let layer = doc.layer_mut(id).unwrap();
                layer.name = name.into();
                layer.surface_mut().unwrap().write_region(Rect::from_xywh(0, 0, 2, 1), &[1.0, 0.0, 0.0, alpha[0], 1.0, 0.0, 0.0, alpha[1]]);
                Ok(())
            })
            .unwrap();
        }
        s.execute("layer.new.layer", json!({"name":"Active"})).unwrap();
        s.execute("edit.fill", json!({"contents":"color","color":"#ffffff"})).unwrap();
        s.actions.list = vec![action.clone()];
        let played = s.execute("actions.play", json!({"action":"Recorded"})).unwrap();
        assert_eq!(played["ran"], 1, "{played}");
        let doc = &s.active().unwrap().doc;
        let selected = doc.selection.as_ref().map(|mask| mask.read_region(doc.bounds())).unwrap_or(vec![1.0; 2]);
        assert_eq!(selected, vec![0.0, 1.0], "{name:?}: use the topmost named layer, not the active layer or merged image");
        assert!(s.undo());
        assert!(s.active().unwrap().doc.selection.is_none());
        assert_eq!(saved(&action).descriptor, Some(d));
        replay_saved(&mut s, &action);
        assert_eq!(s.active().unwrap().doc.selection.as_ref().unwrap().read_region(Rect::from_xywh(0, 0, 2, 1)), vec![0.0, 1.0]);
        for layer in [json!({"name":"Missing"}), json!({"name":1}), json!({"name":name,"extra":true})] {
            assert!(s.execute("select.loadSelection", json!({"channel":"transparency","layer":layer})).is_err());
            assert_eq!(s.active().unwrap().doc.selection.as_ref().unwrap().read_region(Rect::from_xywh(0, 0, 2, 1)), vec![0.0, 1.0]);
        }
        for sentinel in ["merged", ""] {
            let native = Action::new("Recorded", vec![("select.loadSelection".into(), json!({"channel":"transparency","layer":sentinel}))]);
            let reloaded = recorded(AtnEvent::Code(*b"setd"), saved(&native).descriptor.unwrap());
            let (id, params) = &reloaded.steps[0];
            s.execute(id, params.clone()).unwrap();
            let doc = &s.active().unwrap().doc;
            assert_eq!(doc.selection.as_ref().map(|mask| mask.read_region(doc.bounds())).unwrap_or(vec![1.0; 2]), vec![1.0, 1.0]);
        }
    }
}

/// A group's transparency loads the whole canvas, as ⌘-clicking its thumbnail always did (the
/// group's combined transparency is not composited yet), rather than failing.
#[test]
fn group_transparency_sources_load_the_canvas() {
    use photocraft_doc::LayerId;
    use photocraft_geom::Rect;

    for name in ["Group", "merged", ""] {
        let mut s = sized_document(2, 1, 72);
        let child = LayerId(s.execute("layer.new.layer", json!({"name":"Child"})).unwrap()["layer"].as_u64().unwrap());
        s.edit("Child pixels", |doc, _| {
            doc.layer_mut(child).unwrap().surface_mut().unwrap().write_region(Rect::from_xywh(0, 0, 2, 1), &[1.0, 0.0, 0.0, 1.0, 1.0, 0.0, 0.0, 0.0]);
            Ok(())
        })
        .unwrap();
        let group = LayerId(s.execute("layer.groupLayers", json!({"name":"Group"})).unwrap()["layer"].as_u64().unwrap());
        s.edit("Group name", |doc, _| {
            doc.layer_mut(group).unwrap().name = name.into();
            Ok(())
        })
        .unwrap();
        s.execute("layer.new.layer", json!({"name":"Active"})).unwrap();
        s.execute("edit.fill", json!({"color":"#ffffff"})).unwrap();
        s.execute("select.loadSelection", json!({"channel":"transparency","layer":"Child"})).unwrap();
        assert_eq!(s.active().unwrap().doc.selection.as_ref().unwrap().read_region(Rect::from_xywh(0, 0, 2, 1)), vec![1.0, 0.0]);
        let descriptor = Descriptor::new("null").with("null", D::Reference(vec![ReferenceItem::Property { class: class("Chnl"), key: Id::new("fsel") }])).with(
            "T   ",
            D::Reference(vec![
                ReferenceItem::Enumerated { class: class("Chnl"), type_id: Id::new("Chnl"), value: Id::new("Trsp") },
                ReferenceItem::Name { class: class("Lyr "), name: UnicodeString::new_nul(name) },
            ]),
        );
        let action = recorded(AtnEvent::Code(*b"setd"), descriptor.clone());
        let (id, params) = &action.steps[0];
        let result = s.execute(id, params.clone());
        let doc = &s.active().unwrap().doc;
        let alpha = doc.selection.as_ref().map(|mask| mask.read_region(doc.bounds())).unwrap_or(vec![1.0; 2]);
        assert!(result.is_ok(), "group {name:?} returned {result:?}");
        assert_eq!(alpha, vec![1.0, 1.0], "group {name:?}");
        // ⌘-click on the group's thumbnail (by id) loads the same.
        s.execute("select.deselect", json!({})).unwrap();
        s.execute("select.loadSelection", json!({"channel":"transparency","layer":group.0})).unwrap();
        let doc = &s.active().unwrap().doc;
        assert_eq!(doc.selection.as_ref().map(|mask| mask.read_region(doc.bounds())).unwrap_or(vec![1.0; 2]), vec![1.0, 1.0]);
        assert_eq!(saved(&action).descriptor, Some(descriptor));

        // The group's explicit mask remains a valid source.
        s.execute("select.loadSelection", json!({"channel":"transparency","layer":"Child"})).unwrap();
        s.execute("layer.layerMask.revealSelection", json!({"layer":group.0})).unwrap();
        assert!(s.active().unwrap().doc.selection.is_none());
        s.execute("select.loadSelection", json!({"channel":"mask","layer":params["layer"]})).unwrap();
        assert_eq!(s.active().unwrap().doc.selection.as_ref().unwrap().read_region(Rect::from_xywh(0, 0, 2, 1)), vec![1.0, 0.0]);
    }
}

/// New unit validation must fail before editing, while no-resample still ignores size fields.
#[test]
fn native_units_validate_before_edit_and_no_resample_remains_resolution_only() {
    let mut s = sized_document(40, 20, 300);
    for (id, params, refuse_export) in [
        ("image.canvasSize", json!({"width":200,"units":"unknown"}), true),
        ("image.canvasSize", json!({"width":"200","units":"percent"}), true),
        ("image.canvasSize", json!({"width":200,"units":"percent","relative":true}), true),
        ("image.imageSize", json!({"width":72,"units":1}), true),
        ("image.imageSize", json!({"width":"72","units":"pt"}), true),
        // A finite recorded point value is representable; overflow depends on playback dpi.
        ("image.imageSize", json!({"width":1e308,"units":"pt"}), false),
    ] {
        let before = s.execute("document.inspect", json!({})).unwrap();
        assert!(s.execute(id, params.clone()).is_err(), "{id} {params}");
        assert_eq!(s.execute("document.inspect", json!({})).unwrap(), before);
        let action = Action::new("Invalid", vec![(id.into(), params)]);
        assert_eq!(atn_cmds::export("Identifiers", &[&action], &[], false).is_err(), refuse_export);
    }
    let params = json!({"width":1e308,"units":"pt","resolution":144,"resample":"none"});
    s.execute("image.imageSize", params.clone()).unwrap();
    assert_eq!(s.active().unwrap().doc.size, photocraft_geom::Size::new(40, 20));
    assert_eq!(s.active().unwrap().doc.resolution_dpi, 144.0);
    let action = Action::new("Resolution", vec![("image.imageSize".into(), params)]);
    let d = saved(&action).descriptor.unwrap();
    assert!(d.get("Wdth").is_none() && d.get("Hght").is_none());
}

fn class(id: &str) -> Class {
    Class { name: UnicodeString::new_nul(""), class_id: Id::new(id) }
}

fn enm(ty: &str, value: &str) -> D {
    D::Enumerated { type_id: Id::new(ty), value: Id::new(value) }
}

fn blue() -> Descriptor {
    Descriptor::new("RGBC").with("Rd  ", D::Double(0.0)).with("Grn ", D::Double(0.0)).with("Bl  ", D::Double(255.0))
}

/// Load a reference-shaped descriptor through the public ATN parser.
fn recorded(event: AtnEvent, d: Descriptor) -> Action {
    let bytes = atn::write(&AtnFile {
        name: UnicodeString::new_nul("Identifiers"),
        expanded: false,
        actions: vec![AtnAction {
            name: UnicodeString::new_nul("Recorded"),
            items: vec![AtnItem { event, descriptor: Some(d), ..Default::default() }],
            ..Default::default()
        }],
        warnings: vec![],
        encoding: None,
    })
    .unwrap();
    atn_cmds::import(&bytes).unwrap().actions.remove(0)
}

/// Parse the actual saved bytes, so matching import/export mistakes cannot cancel out.
fn saved(action: &Action) -> AtnItem {
    let (bytes, skipped) = atn_cmds::export("Identifiers", &[action], &[], false).unwrap();
    assert!(skipped.is_empty());
    let file = atn::parse(&bytes).unwrap();
    assert_eq!(file.actions[0].items.len(), 1);
    file.actions[0].items[0].clone()
}

/// Check the enum type and value in saved bytes independently of the command parameters.
fn enum_value<'a>(d: &'a Descriptor, key: &str, ty: &str) -> &'a Id {
    let Some(D::Enumerated { type_id, value }) = d.get(key) else { panic!("missing {key} enumeration") };
    assert_eq!(*type_id, Id::new(ty));
    value
}

/// Make's opacity belongs to the layer around the adjustment descriptor.
fn adjustment_make(t: Descriptor, opacity: Option<D>) -> Descriptor {
    let mut using = Descriptor::new("AdjL").with("Type", D::Descriptor(t));
    if let Some(opacity) = opacity {
        using = using.with("Opct", opacity);
    }
    Descriptor::new("null").with("null", D::Reference(vec![ReferenceItem::Class(class("AdjL"))])).with("Usng", D::Descriptor(using))
}

/// Ported from #8659: the saved event changes, while legacy imports and pixel results agree.
#[test]
fn recorded_vibrance_uses_the_string_event_and_keeps_legacy_playback() {
    let d = Descriptor::new("Vbrn").with("vibrance", D::Integer(47)).with("Strt", D::Integer(-19));
    for depth in [8, 16, 32] {
        for event in [AtnEvent::name("vibrance"), AtnEvent::Code(*b"Vbrn")] {
            let action = recorded(event, d.clone());
            assert_eq!(action.steps, vec![("image.adjustments.vibrance".into(), json!({"vibrance":47,"saturation":-19}))]);
            let written = saved(&action);
            assert_eq!(written.event, AtnEvent::name("vibrance"), "Vibrance must save by its string event");
            // Saved with the `vibrance` class the reference application records (#8534); the
            // top-level class does not change what it plays.
            let mut want = d.clone();
            want.class_id = Descriptor::new("vibrance").class_id;
            assert_eq!(written.descriptor, Some(want));
            let create = || {
                let mut s = Session::new();
                s.execute("file.new", json!({"width":4,"height":3,"depth":depth})).unwrap();
                s.execute("edit.fill", json!({"contents":"color","color":"#6594c2"})).unwrap();
                s
            };
            let mut actual = create();
            replay_saved(&mut actual, &action);
            let mut expected = create();
            expected.execute("image.adjustments.vibrance", action.steps[0].1.clone()).unwrap();
            for channel in 0..4 {
                assert_eq!(
                    actual.active().unwrap().doc.layers[0].surface().unwrap().sample_channel(1, 1, channel),
                    expected.active().unwrap().doc.layers[0].surface().unwrap().sample_channel(1, 1, channel)
                );
            }
        }
    }
}

/// The reported Gradient Map Make and the metadata-only default both round-trip atomically.
#[test]
fn recorded_adjustment_opacity_keeps_gradient_settings_and_defaults() {
    let full =
        saved(&Action::new("Recorded", vec![("image.adjustments.gradientMap".into(), json!({"stops":[[0,"#0000ff"],[1,"#ff0000"]]}))])).descriptor.unwrap();
    let default = Descriptor::new("GdMp").with("presetKind", enm("presetKindType", "presetKindDefault"));
    for t in [full, default] {
        let action = recorded(AtnEvent::Code(*b"Mk  "), adjustment_make(t.clone(), Some(D::UnitFloat { unit: *b"#Prc", value: 70.0 })));
        assert_eq!(action.steps.len(), 1);
        assert_eq!(action.steps[0].0, "layer.newAdjustmentLayer.gradientMap");
        assert_eq!(action.steps[0].1["opacity"], json!(179.0 / 255.0), "Make must retain surrounding opacity");
        if t.get("Grad").is_some() {
            assert_eq!(action.steps[0].1["stops"], json!([[0, "#0000ff"], [1, "#ff0000"]]));
        }
        let written = saved(&action).descriptor.unwrap();
        let Some(D::Descriptor(using)) = written.get("Usng") else { panic!("missing adjustment metadata") };
        // The recorded 70 % saves back as 70 %, not as the 8-bit level 179/255 (70.196 %).
        assert!(matches!(using.get("Opct"), Some(D::UnitFloat { unit, value }) if unit == b"#Prc" && *value == 70.0), "{:?}", using.get("Opct"));
        for depth in [8, 16, 32] {
            let mut s = Session::new();
            s.execute("file.new", json!({"width":4,"height":3,"depth":depth})).unwrap();
            let base = s.active().unwrap().active_layer.unwrap();
            replay_saved(&mut s, &action);
            let state = s.active().unwrap();
            assert_eq!(state.doc.layers.len(), 2);
            assert_eq!(state.doc.layer(state.active_layer.unwrap()).unwrap().opacity, 179.0_f32 / 255.0);
            assert_eq!(state.doc.layer(base).unwrap().opacity, 1.0);
            s.execute("edit.undo", json!({})).unwrap();
            assert_eq!(s.active().unwrap().doc.layers.len(), 1, "one undo removes the complete Make");
            s.execute("edit.redo", json!({})).unwrap();
            assert_eq!(s.active().unwrap().doc.layers.len(), 2);
        }
    }
}

/// Invalid supplied opacity cannot become a silent default or create a layer.
#[test]
fn adjustment_opacity_validation_and_endpoints_are_atomic() {
    let t = Descriptor::new("GdMp").with("presetKind", enm("presetKindType", "presetKindDefault"));
    for bad in [
        D::UnitFloat { unit: *b"#Pxl", value: 70.0 },
        D::UnitFloat { unit: *b"#Prc", value: -1.0 },
        D::UnitFloat { unit: *b"#Prc", value: 101.0 },
        D::Text(UnicodeString::new_nul("70")),
        D::Boolean(false),
        D::Double(f64::NAN),
    ] {
        let action = recorded(AtnEvent::Code(*b"Mk  "), adjustment_make(t.clone(), Some(bad)));
        assert_eq!(action.steps[0].0, atn_cmds::PHOTOSHOP_STEP);
        saved(&action);
    }
    let mut s = Session::new();
    s.execute("file.new", json!({"width":4,"height":3})).unwrap();
    for bad in [json!(null), json!("70"), json!(-0.1), json!(1.1)] {
        let before = s.execute("document.inspect", json!({})).unwrap();
        assert!(s.execute("layer.newAdjustmentLayer.gradientMap", json!({"opacity":bad})).is_err());
        assert_eq!(s.execute("document.inspect", json!({})).unwrap(), before);
        assert!(
            atn_cmds::export("Identifiers", &[&Action::new("Bad", vec![("layer.newAdjustmentLayer.gradientMap".into(), json!({"opacity":bad}))])], &[], false)
                .is_err()
        );
    }
    for opacity in [None, Some(0.0), Some(1.0)] {
        let p = opacity.map_or_else(|| json!({}), |opacity| json!({"opacity":opacity}));
        s.execute("layer.newAdjustmentLayer.gradientMap", p.clone()).unwrap();
        let state = s.active().unwrap();
        assert_eq!(state.doc.layer(state.active_layer.unwrap()).unwrap().opacity, opacity.unwrap_or(1.0));
        let action = Action::new("Recorded", vec![("layer.newAdjustmentLayer.gradientMap".into(), p)]);
        let reloaded = recorded(AtnEvent::Code(*b"Mk  "), saved(&action).descriptor.unwrap());
        assert_eq!(reloaded.steps.len(), 1);
        assert_eq!(reloaded.steps[0].0, action.steps[0].0);
        match opacity {
            None => assert_eq!(reloaded.steps[0].1, json!({})),
            Some(opacity) => assert_eq!(reloaded.steps[0].1["opacity"].as_f64(), Some(f64::from(opacity))),
        }
        s.execute("edit.undo", json!({})).unwrap();
    }
}

/// Canonical event names and old code imports share the existing gradient parameters.
#[test]
fn gradient_map_writes_the_named_event_and_keeps_legacy_code_imports() {
    let stop = |location, color| {
        D::Descriptor(Descriptor::new("Clrt").with("Lctn", D::Integer(location)).with("Mdpn", D::Integer(50)).with("Clr ", D::Descriptor(color)))
    };
    let red = Descriptor::new("RGBC").with("Rd  ", D::Double(255.0)).with("Grn ", D::Double(0.0)).with("Bl  ", D::Double(0.0));
    let d = Descriptor::new("GdMp")
        .with("Grad", D::Descriptor(Descriptor::new("Grdn").with("Clrs", D::List(vec![stop(0, blue()), stop(4096, red)]))))
        .with("Rvrs", D::Boolean(true))
        .with("Dthr", D::Boolean(false));
    for event in [AtnEvent::name("gradientMapEvent"), AtnEvent::Code(*b"GdMp")] {
        let action = recorded(event, d.clone());
        assert_eq!(action.steps, vec![("image.adjustments.gradientMap".into(), json!({"stops":[[0,"#0000ff"],[1,"#ff0000"]],"reverse":true,"dither":false}))]);
        let written = saved(&action);
        assert_eq!(written.event, AtnEvent::name("gradientMapEvent"));
        assert!(written.descriptor.unwrap().class_id.is("GdMp"));
    }
    assert_eq!(recorded(AtnEvent::name("GdMp"), d).steps[0].0, atn_cmds::PHOTOSHOP_STEP);
}

/// The measured outside identifier changes while inside, centre and unknown values stay safe.
#[test]
fn stroke_outside_writes_otsd_without_changing_inside_or_center() {
    for (input, location, canonical) in [("Otsd", "outside", "Otsd"), ("OtsF", "outside", "Otsd"), ("Insd", "inside", "Insd"), ("Cntr", "center", "Cntr")] {
        let d = Descriptor::new("null")
            .with("Wdth", D::UnitFloat { unit: *b"#Pxl", value: 2.0 })
            .with("Lctn", enm("StrL", input))
            .with("Clr ", D::Descriptor(blue()));
        let action = recorded(AtnEvent::Code(*b"Strk"), d);
        assert_eq!(action.steps[0].0, "edit.stroke");
        assert_eq!(action.steps[0].1["location"], location);
        assert_eq!(*enum_value(saved(&action).descriptor.as_ref().unwrap(), "Lctn", "StrL"), Id::new(canonical));
    }
    let unknown = Descriptor::new("null").with("Lctn", enm("StrL", "futureLocation"));
    let action = recorded(AtnEvent::Code(*b"Strk"), unknown.clone());
    assert_eq!(action.steps[0].0, atn_cmds::PHOTOSHOP_STEP);
    assert_eq!(saved(&action).descriptor, Some(unknown));
}

/// A group blend uses Photoshop's actual identifier and reaches the native group command.
#[test]
fn pass_through_uses_the_string_identifier_and_replays_on_a_group() {
    for input in ["passThrough", "PsTh"] {
        let target = D::Reference(vec![ReferenceItem::Name { class: class("Lyr "), name: UnicodeString::new_nul("Group") }]);
        let d = Descriptor::new("null").with("null", target).with("T   ", D::Descriptor(Descriptor::new("Lyr ").with("Md  ", enm("BlnM", input))));
        let action = recorded(AtnEvent::Code(*b"setd"), d);
        assert_eq!(action.steps, vec![("layer.setProps".into(), json!({"layer":"Group","blend":"Pass Through"}))]);
        let written = saved(&action).descriptor.unwrap();
        let Some(D::Descriptor(props)) = written.get("T   ") else { panic!("layer properties lost") };
        assert_eq!(*enum_value(props, "Md  ", "BlnM"), Id::new("passThrough"));
        let mut s = Session::new();
        s.execute("file.new", json!({"width":4,"height":4})).unwrap();
        s.execute("layer.new.group", json!({"name":"Group"})).unwrap();
        s.execute("layer.setProps", json!({"blend":"Normal"})).unwrap();
        let (id, p) = &action.steps[0];
        s.execute(id, p.clone()).unwrap();
        let st = s.active().unwrap();
        assert_eq!(st.doc.layer(st.active_layer.unwrap()).unwrap().blend, photocraft_color::BlendMode::PassThrough);
    }
}

fn canvas() -> Descriptor {
    Descriptor::new("null")
        .with("Wdth", D::UnitFloat { unit: *b"#Pxl", value: 6.0 })
        .with("Hght", D::UnitFloat { unit: *b"#Pxl", value: 4.0 })
        .with("Hrzn", enm("HrzL", "Left"))
        .with("Vrtc", enm("VrtL", "Top "))
}

/// Foreground and custom extension colours select visibly different new border pixels.
#[test]
fn canvas_extension_identifiers_preserve_foreground_and_custom_color() {
    for (input, custom, canonical, expected) in [
        ("FrgC", false, "FrgC", [1.0, 0.0, 0.0, 1.0]),
        ("Frgd", false, "FrgC", [1.0, 0.0, 0.0, 1.0]),
        ("Clr ", true, "Clr ", [0.0, 0.0, 1.0, 1.0]),
        ("Othr", true, "Clr ", [0.0, 0.0, 1.0, 1.0]),
    ] {
        let mut d = canvas().with("canvasExtensionColorType", enm("canvasExtensionColorType", input));
        if custom {
            d = d.with("canvasExtensionColor", D::Descriptor(blue()));
        }
        let action = recorded(AtnEvent::Code(*b"CnvS"), d);
        assert_eq!(action.steps[0].0, "image.canvasSize");
        assert_eq!(*enum_value(saved(&action).descriptor.as_ref().unwrap(), "canvasExtensionColorType", "canvasExtensionColorType"), Id::new(canonical));
        let mut s = Session::new();
        s.execute("file.new", json!({"width":4,"height":4,"background":"white"})).unwrap();
        s.execute("tools.setColors", json!({"foreground":"#ff0000","background":"#00ff00"})).unwrap();
        let (id, p) = &action.steps[0];
        s.execute(id, p.clone()).unwrap();
        let pixel: Vec<f32> = serde_json::from_value(s.execute("document.pixel", json!({"x":5,"y":1})).unwrap()).unwrap();
        assert_eq!(pixel, expected);
    }
    for d in [
        canvas().with("canvasExtensionColorType", enm("canvasExtensionColorType", "futureColor")).with("canvasExtensionColor", D::Descriptor(blue())),
        canvas().with("canvasExtensionColorType", D::Integer(1)),
        canvas().with("canvasExtensionColorType", enm("canvasExtensionColorType", "Clr ")),
    ] {
        let action = recorded(AtnEvent::Code(*b"CnvS"), d.clone());
        assert_eq!(action.steps[0].0, atn_cmds::PHOTOSHOP_STEP);
        assert_eq!(saved(&action).descriptor, Some(d));
    }
}

/// Absolute intent writes the measured code and unknown intent values remain preserved steps.
#[test]
fn absolute_colorimetric_writes_aclr_and_rejects_unknown_intents() {
    let profile = || Descriptor::new("null").with("T   ", D::Text(UnicodeString::new_nul("sRGB IEC61966-2.1")));
    for input in ["AClr", "AbsC"] {
        let action = recorded(AtnEvent::name("convertToProfile"), profile().with("Inte", enm("Inte", input)));
        assert_eq!(
            action.steps,
            vec![("edit.convertToProfile".into(), json!({"profile":"srgb","intent":"absolute","dither":false,"flatten":false,"rasterizePlaced":false}))]
        );
        assert_eq!(*enum_value(saved(&action).descriptor.as_ref().unwrap(), "Inte", "Inte"), Id::new("AClr"));
    }
    for value in [enm("Inte", "futureIntent"), D::Integer(1)] {
        let d = profile().with("Inte", value);
        let action = recorded(AtnEvent::name("convertToProfile"), d.clone());
        assert_eq!(action.steps[0].0, atn_cmds::PHOTOSHOP_STEP);
        assert_eq!(saved(&action).descriptor, Some(d));
    }
}

/// Wrap uses opening order after tab moves, with overflow-safe offsets and valid ATN references.
#[test]
fn document_offsets_wrap_in_opening_order_and_zero_atn_offsets_stay_kept() {
    let mut s = Session::new();
    for name in ["a", "b", "c"] {
        s.execute("file.new", json!({"width":2,"height":2,"name":name})).unwrap();
    }
    s.execute("document.move", json!({"document":2,"to":0})).unwrap();
    for (offset, expected) in [(-2, "c"), (2, "a"), (5, "a"), (-5, "c"), (i64::MIN, "c"), (i64::MAX, "c"), (0, "b")] {
        s.execute("document.activate", json!({"document":2})).unwrap();
        s.execute("document.activate", json!({"offset":offset})).unwrap();
        assert_eq!(s.active().unwrap().doc.name, expected, "{offset}");
    }
    for offset in [-2, 2] {
        let d = Descriptor::new("null").with("null", D::Reference(vec![ReferenceItem::Offset { class: class("Dcmn"), offset }]));
        let action = recorded(AtnEvent::Code(*b"slct"), d);
        assert_eq!(action.steps[0].0, "document.activate");
        s.execute("document.activate", json!({"document":2})).unwrap();
        let (id, p) = &action.steps[0];
        s.execute(id, p.clone()).unwrap();
        assert_eq!(s.active().unwrap().doc.name, if offset < 0 { "c" } else { "a" });
        let written = saved(&action).descriptor.unwrap();
        assert!(matches!(written.get("null"),Some(D::Reference(r)) if matches!(r.as_slice(),[ReferenceItem::Offset{offset:o,..}] if *o==offset)));
    }
    let zero = Descriptor::new("null").with("null", D::Reference(vec![ReferenceItem::Offset { class: class("Dcmn"), offset: 0 }]));
    let action = recorded(AtnEvent::Code(*b"slct"), zero.clone());
    assert_eq!(action.steps[0].0, atn_cmds::PHOTOSHOP_STEP);
    assert_eq!(saved(&action).descriptor, Some(zero));
    assert!(atn_cmds::export("Identifiers", &[&Action::new("Zero", vec![("document.activate".into(), json!({"offset":0}))])], &[], false).is_err());
}
