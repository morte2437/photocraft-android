//! #10700: recorded Link and Unlink must not replay as a state-dependent toggle.
use photocraft_doc::{Layer, LayerContent, LayerId, SmartObject, SmartSource};
use photocraft_engine::{Session, actions_cmds::Action, atn_cmds};
use photocraft_psd::{
    atn::{self, AtnAction, AtnEvent, AtnFile, AtnItem},
    descriptor::{Class, Descriptor, Id, ReferenceItem, UnicodeString, Value as D},
};
use serde_json::json;
use std::sync::Arc;

fn fixture(groups: &[Option<u64>], selected: &[usize]) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 2, "height": 2})).unwrap();
    let st = s.active_mut().unwrap();
    let doc = Arc::make_mut(&mut st.doc);
    doc.layers = groups
        .iter()
        .enumerate()
        .map(|(i, group)| {
            let mut layer = Layer::raster(format!("Layer {i}"), doc.pixel_format());
            layer.link_group = *group;
            layer
        })
        .collect();
    let ids: Vec<_> = selected.iter().map(|i| doc.layers[*i].id).collect();
    s.restore_layer_selection(ids.last().copied(), &ids, ids.first().copied()).unwrap();
    s
}

fn groups(s: &Session) -> Vec<Option<u64>> {
    s.active().unwrap().doc.layers.iter().map(|layer| layer.link_group).collect()
}

fn target() -> D {
    target_class("Lyr ")
}

fn target_class(class_id: &str) -> D {
    ordinal_target(class_id, "Trgt")
}

fn ordinal_target(class_id: &str, ordinal: &str) -> D {
    D::Reference(vec![ReferenceItem::Enumerated {
        class: Class { name: UnicodeString::new_nul(""), class_id: Id::new(class_id) },
        type_id: Id::new("Ordn"),
        value: Id::new(ordinal),
    }])
}

fn file(event: &str, descriptor: Descriptor) -> Vec<u8> {
    atn::write(&AtnFile {
        name: UnicodeString::new_nul("Links"),
        expanded: false,
        actions: vec![AtnAction {
            name: UnicodeString::new_nul("A"),
            items: vec![AtnItem {
                event: match event.as_bytes() {
                    [a, b, c, d] => AtnEvent::Code([*a, *b, *c, *d]),
                    _ => AtnEvent::name(event),
                },
                descriptor: Some(descriptor),
                ..Default::default()
            }],
            ..Default::default()
        }],
        warnings: vec![],
        ..Default::default()
    })
    .unwrap()
}

fn load(s: &mut Session, bytes: &[u8]) -> serde_json::Value {
    s.execute("actions.load", json!({"data": photocraft_engine::paint::tile::b64_encode(bytes)})).unwrap()
}

fn play(s: &mut Session) {
    let report = s.execute("actions.play", json!({"action": "A", "set": "Links"})).unwrap();
    assert!(report.get("failed").is_none(), "{report}");
    assert_eq!(report["ran"], 1, "{report}");
}

fn saved_item(s: &mut Session) -> AtnItem {
    let saved = s.execute("actions.save", json!({"set": "Links"})).unwrap();
    assert_eq!(saved["skipped"], json!([]));
    let bytes = photocraft_engine::paint::tile::b64_decode(saved["data"].as_str().unwrap()).unwrap();
    atn::parse(&bytes).unwrap().actions[0].items[0].clone()
}

#[test]
fn explicit_links_are_idempotent_and_omitted_state_still_toggles() {
    let mut s = fixture(&[None, None], &[0, 1]);
    let before = s.active().unwrap().doc.clone();
    let history = s.active().unwrap().history.past_len();
    s.execute("layer.linkLayers", json!({"linked": false})).unwrap();
    assert_eq!(s.active().unwrap().doc, before);
    assert_eq!(s.active().unwrap().history.past_len(), history);
    s.execute("layer.linkLayers", json!({"linked": true})).unwrap();
    let linked = s.active().unwrap().doc.clone();
    assert!(groups(&s)[0].is_some());
    assert_eq!(groups(&s)[0], groups(&s)[1]);
    let history = s.active().unwrap().history.past_len();
    s.execute("layer.linkLayers", json!({"linked": true})).unwrap();
    assert_eq!(s.active().unwrap().doc, linked);
    assert_eq!(s.active().unwrap().history.past_len(), history);
    s.execute("layer.linkLayers", json!({})).unwrap();
    assert_eq!(groups(&s), [None, None]);
    s.execute("layer.linkLayers", json!({})).unwrap();
    assert!(groups(&s).iter().all(Option::is_some));
}

#[test]
fn unlink_cleans_singletons_preserves_other_groups_and_undoes() {
    for initial in [vec![Some(77), Some(77), Some(77), Some(93), Some(93)], vec![Some(77), None, Some(77), Some(93), Some(93)]] {
        let mut s = fixture(&initial, &[0, 1]);
        let before = s.active().unwrap().doc.clone();
        s.execute("layer.linkLayers", json!({"linked": false})).unwrap();
        assert_eq!(groups(&s), [None, None, None, Some(93), Some(93)]);
        let history = s.active().unwrap().history.past_len();
        s.execute("layer.linkLayers", json!({"linked": false})).unwrap();
        assert_eq!(s.active().unwrap().history.past_len(), history);
        assert!(s.undo());
        assert_eq!(s.active().unwrap().doc, before);
    }
    let mut one = fixture(&[Some(77), Some(77)], &[0]);
    one.execute("layer.linkLayers", json!({"linked": false})).unwrap();
    assert_eq!(groups(&one), [None, None]);
    let history = one.active().unwrap().history.past_len();
    one.execute("layer.linkLayers", json!({"linked": false})).unwrap();
    assert_eq!(one.active().unwrap().history.past_len(), history);
    assert!(!one.is_enabled("layer.linkLayers"), "the UI toggle still requires two unlinked layers");
}

/// Explicit Link cleans an orphan without touching a real group's unselected member.
#[test]
fn explicit_link_cleans_a_singleton_but_preserves_a_linked_pair() {
    for initial in [vec![Some(77)], vec![Some(77), None], vec![Some(77), Some(77)]] {
        let mut s = fixture(&initial, &[0]);
        let before = s.active().unwrap().doc.clone();
        let history = s.active().unwrap().history.past_len();
        let linked = initial == [Some(77), Some(77)];
        assert_eq!(s.execute("layer.linkLayers", json!({"linked": true})).unwrap()["linked"], linked);
        if linked {
            assert_eq!(s.active().unwrap().doc, before);
            assert_eq!(s.active().unwrap().history.past_len(), history);
        } else {
            assert!(groups(&s).iter().all(Option::is_none));
            assert!(s.undo());
            assert_eq!(s.active().unwrap().doc, before);
        }
    }
}

#[test]
fn invalid_link_state_fails_without_editing() {
    let mut s = fixture(&[None, None], &[0, 1]);
    let before = s.active().unwrap().doc.clone();
    let history = s.active().unwrap().history.past_len();
    for invalid in [json!(null), json!("false"), json!(0), json!([]), json!({})] {
        assert!(s.execute("layer.linkLayers", json!({"linked": invalid})).is_err());
        assert_eq!(s.active().unwrap().doc, before);
        assert_eq!(s.active().unwrap().history.past_len(), history);
    }
    for active in [None, Some(LayerId(u64::MAX))] {
        let st = s.active_mut().unwrap();
        st.active_layer = active;
        st.selected_layers = active.into_iter().collect();
        assert!(s.execute("layer.linkLayers", json!({"linked": false})).is_err());
        assert_eq!(s.active().unwrap().doc, before);
        assert_eq!(s.active().unwrap().history.past_len(), history);
    }
    assert!(Session::new().execute("layer.linkLayers", json!({"linked": false})).is_err());
}

#[test]
fn atn_link_events_keep_descriptors_and_replay_the_requested_operation() {
    // Link Layers keeps the measured toggle (main's Group F mapping); Unlink is explicit.
    {
        let (event, linked) = ("unlinkSelectedLayers", false);
        for initial in [vec![None, None, None, Some(93), Some(93)], vec![Some(77), Some(77), Some(77), Some(93), Some(93)]] {
            let descriptor = Descriptor::new("null").with("null", target());
            let mut s = fixture(&initial, &[0, 1]);
            assert_eq!(load(&mut s, &file(event, descriptor.clone()))["unsupported"], json!([]));
            assert_eq!(s.actions.list[0].steps, [("layer.linkLayers".into(), json!({"linked": linked}))]);
            let saved = s.execute("actions.save", json!({"set": "Links"})).unwrap();
            assert_eq!(saved["skipped"], json!([]));
            let bytes = photocraft_engine::paint::tile::b64_decode(saved["data"].as_str().unwrap()).unwrap();
            let parsed = atn::parse(&bytes).unwrap();
            assert_eq!(parsed.actions[0].items[0].event, AtnEvent::name(event));
            assert_eq!(parsed.actions[0].items[0].descriptor, Some(descriptor));
            let mut replay = fixture(&initial, &[0, 1]);
            assert_eq!(load(&mut replay, &bytes)["unsupported"], json!([]));
            play(&mut replay);
            if linked {
                let result = groups(&replay);
                assert!(result[0].is_some());
                assert_eq!(result[0], result[1]);
                assert_eq!(&result[2..], &[initial[2], Some(93), Some(93)]);
            } else {
                assert_eq!(groups(&replay), [None, None, None, Some(93), Some(93)]);
            }
            let before = replay.active().unwrap().doc.clone();
            let history = replay.active().unwrap().history.past_len();
            play(&mut replay);
            assert_eq!(replay.active().unwrap().doc, before);
            assert_eq!(replay.active().unwrap().history.past_len(), history);
        }
    }
}

#[test]
fn unsupported_link_descriptors_are_kept_and_ambiguous_exports_are_refused() {
    for event in ["linkSelectedLayers", "unlinkSelectedLayers"] {
        for descriptor in [
            Descriptor::new("null").with("null", D::Reference(vec![])),
            Descriptor::new("null").with("null", D::Integer(1)),
            Descriptor::new("null").with("null", target_class("Dcmn")),
            Descriptor::new("null").with("null", target()).with("extra", D::Boolean(true)),
            Descriptor::new("null").with("null", target()).with("null", target()),
        ] {
            let mut s = Session::new();
            assert_eq!(load(&mut s, &file(event, descriptor.clone()))["unsupported"].as_array().unwrap().len(), 1);
            let saved = s.execute("actions.save", json!({"set": "Links"})).unwrap();
            let bytes = photocraft_engine::paint::tile::b64_decode(saved["data"].as_str().unwrap()).unwrap();
            let parsed = atn::parse(&bytes).unwrap();
            assert_eq!(parsed.actions[0].items[0].event, AtnEvent::name(event));
            assert_eq!(parsed.actions[0].items[0].descriptor, Some(descriptor));
        }
    }
    // `{}` is the measured Link Layers toggle and exports (main's Group F mapping).
    for params in [json!({"linked": null}), json!({"linked": true, "layer": "Layer 0"}), json!({"linked": false, "layers": [1, 2]})] {
        let action = Action::new("A", vec![("layer.linkLayers".into(), params)]);
        assert!(atn_cmds::export("Links", &[&action], &[], false).is_err());
    }
}

const TARGET_EVENTS: &[(&str, &str)] = &[
    ("reverse", "layer.arrange.reverse"),
    ("selectLinkedLayers", "layer.selectLinkedLayers"),
    ("disableLayerStyle", "layer.layerStyle.clear"),
    ("placedLayerMakeCopy", "layer.smartObjects.newSmartObjectViaCopy"),
    ("placedLayerConvertToLayers", "layer.smartObjects.convertToLayers"),
    ("placedLayerConvertToEmbedded", "layer.smartObjects.convertToEmbedded"),
    ("placedLayerUpdateAllModified", "layer.smartObjects.updateAllModifiedContent"),
    ("rasterizeAll", "layer.rasterize.allLayers"),
    ("addBlankVideoLayer", "layer.videoLayers.newBlankVideoLayer"),
    ("isolateLayers", "select.isolateLayers"),
    ("textLoadDefaultTypeStyles", "type.loadDefaultTypeStyles"),
    ("textSaveDefaultTypeStyles", "type.saveDefaultTypeStyles"),
    ("recordMeasurements", "image.analysis.recordMeasurements"),
    ("CropPhotosAuto0001", "file.automate.cropAndStraightenPhotos"),
];

fn targeted(event: &str, explicit: bool) -> Descriptor {
    let d = Descriptor::new(event);
    if explicit { d.with("null", target()) } else { d }
}

fn load_targeted(s: &mut Session, event: &str, id: &str, explicit: bool) {
    assert_eq!(load(s, &file(event, targeted(event, explicit)))["unsupported"], json!([]), "{event}");
    let p = if explicit { json!({}) } else { json!({"_atnNoTarget": true}) };
    assert_eq!(s.actions.list[0].steps, [(id.into(), p)]);
}

#[test]
fn target_events_keep_empty_and_explicit_target_descriptors_distinct() {
    for &(event, id) in TARGET_EVENTS {
        for explicit in [false, true] {
            let mut s = Session::new();
            load_targeted(&mut s, event, id, explicit);
            let saved = s.execute("actions.save", json!({"set": "Links"})).unwrap();
            let bytes = photocraft_engine::paint::tile::b64_decode(saved["data"].as_str().unwrap()).unwrap();
            let item = &atn::parse(&bytes).unwrap().actions[0].items[0];
            assert_eq!(item.event, AtnEvent::name(event));
            assert_eq!(item.descriptor, Some(targeted("null", explicit)), "{event}");
            assert_eq!(load(&mut s, &bytes)["unsupported"], json!([]));
        }
    }
    // Reference lines197–199 permits writing an empty descriptor for an absent one.
    let mut f = atn::parse(&file("reverse", Descriptor::new("null"))).unwrap();
    f.actions[0].items[0].descriptor = None;
    let mut s = Session::new();
    assert_eq!(load(&mut s, &atn::write(&f).unwrap())["unsupported"], json!([]));
    let saved = s.execute("actions.save", json!({"set": "Links"})).unwrap();
    let bytes = photocraft_engine::paint::tile::b64_decode(saved["data"].as_str().unwrap()).unwrap();
    assert_eq!(atn::parse(&bytes).unwrap().actions[0].items[0].descriptor, Some(Descriptor::new("null")));
}

#[test]
fn bare_link_events_keep_their_operation_and_empty_keys() {
    // Link Layers keeps the measured toggle (main's Group F mapping); Unlink is explicit.
    {
        let (event, linked) = ("unlinkSelectedLayers", false);
        let initial = if linked { None } else { Some(77) };
        let mut s = fixture(&[initial, initial], &[0, 1]);
        assert_eq!(load(&mut s, &file(event, Descriptor::new(event)))["unsupported"], json!([]));
        assert_eq!(s.actions.list[0].steps[0].1, json!({"linked": linked, "_atnNoTarget": true}));
        let saved = s.execute("actions.save", json!({"set": "Links"})).unwrap();
        let bytes = photocraft_engine::paint::tile::b64_decode(saved["data"].as_str().unwrap()).unwrap();
        assert_eq!(atn::parse(&bytes).unwrap().actions[0].items[0].descriptor, Some(Descriptor::new("null")));
        let before = s.active().unwrap().doc.clone();
        play(&mut s);
        assert_ne!(s.active().unwrap().doc, before);
        let after = s.active().unwrap().doc.clone();
        assert!(groups(&s).iter().all(|group| group.is_some() == linked));
        assert_eq!(groups(&s)[0], groups(&s)[1]);
        play(&mut s);
        assert_eq!(s.active().unwrap().doc, after);
    }
}

#[test]
fn target_events_keep_unknown_targets_and_refuse_native_options() {
    for &(event, id) in TARGET_EVENTS {
        for descriptor in [
            Descriptor::new(event).with("null", D::Integer(1)),
            Descriptor::new(event).with("null", target_class("Dcmn")),
            targeted(event, true).with("null", target()),
            targeted(event, true).with("extra", D::Boolean(true)),
        ] {
            let mut s = Session::new();
            assert_eq!(load(&mut s, &file(event, descriptor.clone()))["unsupported"].as_array().unwrap().len(), 1);
            let saved = s.execute("actions.save", json!({"set": "Links"})).unwrap();
            let bytes = photocraft_engine::paint::tile::b64_decode(saved["data"].as_str().unwrap()).unwrap();
            assert_eq!(atn::parse(&bytes).unwrap().actions[0].items[0].descriptor, Some(descriptor));
        }
        for p in [json!({"layer": 1}), json!({"on": true}), json!({"_atnNoTarget": false}), json!({"_atnNoTarget": "true"})] {
            let action = Action::new("A", vec![(id.into(), p)]);
            assert!(atn_cmds::export("Links", &[&action], &[], false).is_err(), "{event}");
        }
    }
    for linked in [false, true] {
        let action = Action::new("A", vec![("layer.linkLayers".into(), json!({"linked": linked, "_atnNoTarget": false}))]);
        assert!(atn_cmds::export("Links", &[&action], &[], false).is_err());
    }
}

fn scan_fixture() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 480, "height": 340, "background": "#fafafa"})).unwrap();
    let st = s.active_mut().unwrap();
    let id = st.active_layer.unwrap();
    let surface = Arc::make_mut(&mut st.doc).layer_mut(id).unwrap().surface_mut().unwrap();
    for (x, y, w, h, color) in [(40, 60, 160, 110, [0.6, 0.1, 0.1, 1.0]), (290, 180, 150, 120, [0.1, 0.2, 0.6, 1.0])] {
        surface.fill_rect(photocraft_doc::Rect::new(x, y, x + w, y + h), &color);
    }
    s.execute("select.rect", json!({"x": 270, "y": 160, "width": 190, "height": 160})).unwrap();
    s
}

fn assert_cropped_scan(s: &Session) {
    assert_eq!(s.documents().len(), 2);
    let doc = &s.documents()[1].doc;
    assert!((i64::from(doc.size.width) - 150).abs() <= 4);
    assert!((i64::from(doc.size.height) - 120).abs() <= 4);
    let color = doc.layers[0].surface().unwrap().rgba(doc.size.width as i32 / 2, doc.size.height as i32 / 2);
    assert!(color[2] > 0.5 && color[0] < 0.2, "{color:?}");
}

#[test]
fn crop_parent_creates_the_selected_photo_without_changing_the_scan() {
    for explicit in [false, true] {
        let mut s = scan_fixture();
        let source = s.active().unwrap().doc.clone();
        let history = s.active().unwrap().history.past_len();
        load_targeted(&mut s, "CropPhotosAuto0001", "file.automate.cropAndStraightenPhotos", explicit);
        assert!(s.is_enabled("file.automate.cropAndStraightenPhotos"));
        play(&mut s);
        assert_cropped_scan(&s);
        assert_eq!(s.documents()[0].doc, source);
        assert_eq!(s.documents()[0].history.past_len(), history);
    }
}

#[test]
fn crop_parent_refuses_ineligible_layers_without_document_or_history_changes() {
    for case in ["missing", "stale", "hidden", "hidden-parent", "pixels", "locked-parent", "indexed"] {
        let mut s = scan_fixture();
        let st = s.active_mut().unwrap();
        let id = st.active_layer.unwrap();
        let doc = Arc::make_mut(&mut st.doc);
        match case {
            "missing" => st.active_layer = None,
            "stale" => st.active_layer = Some(LayerId::fresh()),
            "hidden" => doc.layer_mut(id).unwrap().visible = false,
            "pixels" => doc.layer_mut(id).unwrap().locks.pixels = true,
            "indexed" => doc.mode = photocraft_doc::ColorMode::Indexed,
            _ => {
                let mut group = Layer::group("Parent", std::mem::take(&mut doc.layers));
                group.visible = case != "hidden-parent";
                group.locks.all = case == "locked-parent";
                doc.layers.push(group);
            }
        }
        let before = s.active().unwrap().doc.clone();
        let history = s.active().unwrap().history.past_len();
        assert!(!s.is_enabled("file.automate.cropAndStraightenPhotos"), "{case}");
        load_targeted(&mut s, "CropPhotosAuto0001", "file.automate.cropAndStraightenPhotos", false);
        let report = s.execute("actions.play", json!({"action": "A", "set": "Links"})).unwrap();
        assert_eq!(report["ran"], 0, "{case}: {report}");
        assert_eq!(report["failed"]["id"], "file.automate.cropAndStraightenPhotos", "{case}: {report}");
        assert_eq!(s.documents().len(), 1);
        assert_eq!(s.active().unwrap().doc, before);
        assert_eq!(s.active().unwrap().history.past_len(), history);
    }
    assert!(!Session::new().is_enabled("file.automate.cropAndStraightenPhotos"));
}

#[test]
fn crop_parent_without_photos_fails_without_creating_documents() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 80, "height": 80, "background": "white"})).unwrap();
    let before = s.active().unwrap().doc.clone();
    let history = s.active().unwrap().history.past_len();
    load_targeted(&mut s, "CropPhotosAuto0001", "file.automate.cropAndStraightenPhotos", false);
    let report = s.execute("actions.play", json!({"action": "A", "set": "Links"})).unwrap();
    assert_eq!(report["ran"], 0, "{report}");
    assert!(report["failed"]["error"].as_str().unwrap().contains("no photos found"), "{report}");
    assert_eq!(s.documents().len(), 1);
    assert_eq!(s.active().unwrap().doc, before);
    assert_eq!(s.active().unwrap().history.past_len(), history);
}

#[test]
fn crop_parent_runs_once_and_the_kept_helper_stops_the_duplicate_crop_tail() {
    let mut f = atn::parse(&file("CropPhotosAuto0001", Descriptor::new("cjsp"))).unwrap();
    let helper = Descriptor::new("null").with("Vl  ", D::List([0.0, 0.0, 150.0, 0.0, 150.0, 120.0, 0.0, 120.0].into_iter().map(D::Double).collect()));
    f.actions[0].items.push(AtnItem { event: AtnEvent::name("CropPhotos0001"), descriptor: Some(helper.clone()), ..Default::default() });
    let tail = Action::new("Tail", vec![("image.duplicate".into(), json!({})), ("image.crop".into(), json!({"x": 1, "y": 1, "width": 10, "height": 10}))]);
    let (bytes, skipped) = atn_cmds::export("Tail", &[&tail], &[], false).unwrap();
    assert!(skipped.is_empty());
    f.actions[0].items.extend(atn::parse(&bytes).unwrap().actions[0].items.clone());
    let mut s = scan_fixture();
    let source = s.active().unwrap().doc.clone();
    let report = load(&mut s, &atn::write(&f).unwrap());
    assert_eq!(report["unsupported"].as_array().unwrap().len(), 1);
    assert_eq!(
        s.actions.list[0].steps.iter().map(|step| step.0.as_str()).collect::<Vec<_>>(),
        ["file.automate.cropAndStraightenPhotos", "actions.photoshopStep", "image.duplicate", "image.crop"]
    );
    let saved = s.execute("actions.save", json!({"set": "Links"})).unwrap();
    let bytes = photocraft_engine::paint::tile::b64_decode(saved["data"].as_str().unwrap()).unwrap();
    assert_eq!(atn::parse(&bytes).unwrap().actions[0].items[1].descriptor, Some(helper));
    let report = s.execute("actions.play", json!({"action": "A", "set": "Links"})).unwrap();
    assert_eq!(report["ran"], 1, "{report}");
    assert_eq!(report["failed"]["step"], 1);
    assert_eq!(report["failed"]["id"], "actions.photoshopStep");
    assert_cropped_scan(&s);
    assert_eq!(s.documents()[0].doc, source);
}

/// Two sparse marks leave a transparent gap where the converted board's background is observable.
fn artboard_fixture() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 12, "height": 10, "background": "transparent"})).unwrap();
    let st = s.active_mut().unwrap();
    let doc = Arc::make_mut(&mut st.doc);
    let mut child = Layer::raster("Pixels", doc.pixel_format());
    let surface = child.surface_mut().unwrap();
    for rect in [photocraft_doc::Rect::new(2, 2, 3, 3), photocraft_doc::Rect::new(5, 5, 6, 6)] {
        surface.fill_rect(rect, &[1.0, 0.0, 0.0, 1.0]);
    }
    let group = Layer::group("Group", vec![child]);
    let id = group.id;
    doc.layers = vec![group];
    s.restore_layer_selection(Some(id), &[id], Some(id)).unwrap();
    s
}

/// The reference's ordered settings keys, with fractional RGB that must not be rounded to hex.
fn artboard_descriptor(background: i32, guides: Vec<D>, flag: D) -> Descriptor {
    Descriptor::new("null").with("null", target()).with("artboard", D::Descriptor(artboard_settings(Some(background), guides))).with("changeBackground", flag)
}

/// Background types 1..3 still record the last custom RGB value; absent type means white.
fn artboard_settings(background: Option<i32>, guides: Vec<D>) -> Descriptor {
    let color = Descriptor::new("RGBC").with("Rd  ", D::Double(63.75)).with("Grn ", D::Double(127.5)).with("Bl  ", D::Double(191.25));
    let board = Descriptor::new("artboard")
        .with("guideIDs", D::List(guides))
        .with("artboardPresetName", D::Text(UnicodeString::new_nul("iPhone 14")))
        .with("Clr ", D::Descriptor(color));
    match background {
        Some(n) => board.with("artboardBackgroundType", D::Integer(n)),
        None => board,
    }
}

/// Conversion preserves the group's geometry/content while applying every admitted setting and Undo.
#[test]
fn artboard_recorded_flags_and_guides_roundtrip_without_resizing() {
    for (background, expected) in [(1, [1.0, 1.0, 1.0, 1.0]), (2, [0.0, 0.0, 0.0, 1.0]), (3, [0.0; 4]), (4, [0.25, 0.5, 0.75, 1.0])] {
        for flag in [None, Some(0), Some(1), Some(2), Some(3), Some(-1), Some(7)] {
            let mut s = artboard_fixture();
            Arc::make_mut(&mut s.active_mut().unwrap().doc).guides.horizontal.push(3.0);
            Arc::make_mut(&mut s.active_mut().unwrap().doc).guides.vertical.push(4.0);
            let before = s.active().unwrap().doc.clone();
            let id = s.active().unwrap().active_layer.unwrap();
            let mut d = artboard_descriptor(background, vec![D::Integer(2), D::Integer(99999)], D::Integer(flag.unwrap_or(1)));
            if flag.is_none() {
                d.items.pop();
            }
            assert_eq!(load(&mut s, &file("artboardFromLayerGroupEvent", d.clone()))["unsupported"], json!([]));
            assert_eq!(s.actions.list[0].steps[0].0, "layer.new.artboardFromGroup");
            assert_eq!(s.actions.list[0].steps[0].1["color"], json!([0.25, 0.5, 0.75]));
            assert_eq!(saved_item(&mut s).descriptor, Some(d));
            play(&mut s);
            let doc = &s.active().unwrap().doc;
            let board = doc.layer(id).unwrap().artboard().unwrap();
            assert_eq!(board.rect, photocraft_doc::Rect::new(2, 2, 6, 6), "preset is a tag, not a request to resize the group");
            assert_eq!(board.preset, "iPhone 14");
            assert_eq!(doc.size, before.size);
            assert_eq!(doc.layer(id).unwrap().children(), before.layer(id).unwrap().children());
            assert_eq!(doc.guides, before.guides);
            let pixel = photocraft_compose::render(doc, photocraft_doc::Rect::new(3, 3, 4, 4)).px[0];
            let applied = flag.is_none_or(|n| n == 1);
            for (actual, expected) in pixel.into_iter().zip(if applied { expected } else { [1.0; 4] }) {
                assert!((actual - expected).abs() < 0.005, "background {background}, flag {flag:?}: {pixel:?}");
            }
            assert!(s.undo());
            assert_eq!(s.active().unwrap().doc, before);
        }
    }
}

/// The target-only call uses the existing menu behavior without synthesizing settings.
#[test]
fn artboard_target_only_form_keeps_existing_native_group_behavior() {
    let mut s = artboard_fixture();
    let before = s.active().unwrap().doc.clone();
    let d = Descriptor::new("null").with("null", target());
    assert_eq!(load(&mut s, &file("artboardFromLayerGroupEvent", d.clone()))["unsupported"], json!([]));
    assert_eq!(s.actions.list[0].steps, [("layer.new.artboardFromGroup".into(), json!({}))]);
    assert_eq!(saved_item(&mut s).descriptor, Some(d));
    play(&mut s);
    assert!(s.active().unwrap().doc.layers[0].artboard().is_some());
    assert!(s.undo());
    assert_eq!(s.active().unwrap().doc, before);
}

/// A conversion selects its result, so a following Delete cannot remove another selected layer.
#[test]
fn artboard_from_group_replaces_the_previous_layer_selection() {
    let mut s = artboard_fixture();
    let group = s.active().unwrap().active_layer.unwrap();
    let doc = Arc::make_mut(&mut s.active_mut().unwrap().doc);
    let other = Layer::raster("Other", doc.pixel_format());
    let id = other.id;
    doc.layers.push(other);
    s.restore_layer_selection(Some(group), &[id, group], Some(id)).unwrap();
    load(&mut s, &file("artboardFromLayerGroupEvent", Descriptor::new("null").with("null", target())));
    play(&mut s);
    assert_eq!(s.active().unwrap().selected_layers(), vec![group]);
    s.execute("layer.delete", json!({})).unwrap();
    assert!(s.active().unwrap().doc.layer(id).is_some());
    assert!(s.active().unwrap().doc.layer(group).is_none());
}

/// A missing custom color means White; an explicit white color remains Custom through save/load.
#[test]
fn artboard_partial_custom_background_preserves_the_document_variant() {
    for params in [json!({"background":"custom"}), json!({"background":"custom","color":[1,1,1]})] {
        let mut s = artboard_fixture();
        s.execute("layer.new.artboardFromGroup", params.clone()).unwrap();
        let expected = s.active().unwrap().doc.clone();
        assert!(s.undo());
        let action = Action::new("A", vec![("layer.new.artboardFromGroup".into(), params)]);
        let (bytes, skipped) = atn_cmds::export("Links", &[&action], &[], false).unwrap();
        assert!(skipped.is_empty());
        assert_eq!(load(&mut s, &bytes)["unsupported"], json!([]));
        play(&mut s);
        assert_eq!(s.active().unwrap().doc, expected);
    }
}

/// Preserve fractional wire values exactly while playback uses normalized native RGB.
#[test]
fn artboard_fractional_colors_survive_import_save_and_playback() {
    let rgb = [7.97, 31.88, 127.6];
    for event in ["artboardFromLayerGroupEvent", "editArtboardEvent"] {
        let mut s = artboard_fixture();
        let mut d = if event == "editArtboardEvent" {
            s.execute("layer.new.artboardFromGroup", json!({})).unwrap();
            artboard_edit_descriptor(4, 8)
        } else {
            artboard_descriptor(4, vec![], D::Integer(1))
        };
        let D::Descriptor(board) = &mut d.items.iter_mut().find(|(key, _)| key.is("artboard")).unwrap().1 else { unreachable!() };
        board.items.iter_mut().find(|(key, _)| key.is("Clr ")).unwrap().1 =
            D::Descriptor(["Rd  ", "Grn ", "Bl  "].into_iter().zip(rgb).fold(Descriptor::new("RGBC"), |color, (key, n)| color.with(key, D::Double(n))));
        assert_eq!(load(&mut s, &file(event, d.clone()))["unsupported"], json!([]));
        assert_eq!(saved_item(&mut s).descriptor, Some(d));
        let (id, mut changed) = s.actions.list[0].steps[0].clone();
        changed["color"][0] = 0.5.into();
        assert!(atn_cmds::export("Links", &[&Action::new("Changed", vec![(id, changed)])], &[], false).is_err());
        let before = s.active().unwrap().doc.clone();
        play(&mut s);
        let pixel = photocraft_compose::render(&s.active().unwrap().doc, photocraft_doc::Rect::new(3, 3, 4, 4)).px[0];
        for (actual, expected) in pixel.into_iter().zip(rgb.map(|n| n as f32 / 255.0)) {
            assert!((actual - expected).abs() < 0.005, "{event}: {pixel:?}");
        }
        assert!(s.undo());
        assert_eq!(s.active().unwrap().doc, before);
    }
}

/// Unproved settings remain byte-preserving kept steps and cannot partially edit the document.
#[test]
fn artboard_unmeasured_types_and_extra_settings_stay_whole() {
    for d in [
        artboard_descriptor(1, vec![D::Double(7.0)], D::Integer(1)),
        artboard_descriptor(1, vec![], D::Boolean(true)),
        artboard_descriptor(5, vec![], D::Integer(1)),
        artboard_descriptor(1, vec![], D::Integer(1)).with("extra", D::Boolean(true)),
        Descriptor::new("null"),
    ] {
        let mut s = artboard_fixture();
        assert_eq!(load(&mut s, &file("artboardFromLayerGroupEvent", d.clone()))["unsupported"].as_array().unwrap().len(), 1);
        assert_eq!(saved_item(&mut s).descriptor, Some(d));
        let before = s.active().unwrap().doc.clone();
        let history = s.active().unwrap().history.past_len();
        let report = s.execute("actions.play", json!({"action":"A","set":"Links"})).unwrap();
        assert_eq!(report["ran"], 0, "{report}");
        assert_eq!(report["failed"]["id"], "actions.photoshopStep");
        assert_eq!(s.active().unwrap().doc, before);
        assert_eq!(s.active().unwrap().history.past_len(), history);
    }
    for p in [json!({"layer":1}), json!({"preset":7}), json!({"guideIDs":[7.5]}), json!({"changeBackground":false})] {
        let action = Action::new("A", vec![("layer.new.artboardFromGroup".into(), p)]);
        assert!(atn_cmds::export("Links", &[&action], &[], false).is_err());
    }
}

/// Omitted type means white rather than the color-only native call's custom background.
#[test]
fn artboard_omitted_type_defaults_to_white_and_invalid_native_settings_are_atomic() {
    let mut s = artboard_fixture();
    let before = s.active().unwrap().doc.clone();
    let d =
        Descriptor::new("null").with("null", target()).with("artboard", D::Descriptor(artboard_settings(None, vec![]))).with("changeBackground", D::Integer(1));
    assert_eq!(load(&mut s, &file("artboardFromLayerGroupEvent", d.clone()))["unsupported"], json!([]));
    assert_eq!(saved_item(&mut s).descriptor, Some(d));
    let mut changed = s.actions.list[0].steps[0].1.clone();
    changed["background"] = "black".into();
    assert!(atn_cmds::export("Links", &[&Action::new("A", vec![("layer.new.artboardFromGroup".into(), changed)])], &[], false).is_err());
    play(&mut s);
    assert_eq!(photocraft_compose::render(&s.active().unwrap().doc, photocraft_doc::Rect::new(3, 3, 4, 4)).px[0], [1.0; 4]);
    assert!(s.undo());
    assert_eq!(s.active().unwrap().doc, before);
    let history = s.active().unwrap().history.past_len();
    for p in [
        json!({"guideIDs":[7.5]}),
        json!({"guideIDs":false}),
        json!({"changeBackground":false}),
        json!({"changeBackground":2147483648i64}),
        json!({"preset":7}),
    ] {
        assert!(s.execute("layer.new.artboardFromGroup", p).is_err());
        assert_eq!(s.active().unwrap().doc, before);
        assert_eq!(s.active().unwrap().history.past_len(), history);
    }
}

/// Recorded geometry deliberately differs from the target: flag 8 changes only its background.
fn artboard_edit_descriptor(background: i32, change_sizes: i32) -> Descriptor {
    let rect = Descriptor::new("classFloatRect")
        .with("Top ", D::Double(40.5))
        .with("Left", D::Double(60.25))
        .with("Btom", D::Double(340.5))
        .with("Rght", D::Double(260.25));
    let mut board = Descriptor::new("artboard").with("artboardRect", D::Descriptor(rect));
    board.items.extend(artboard_settings(Some(background), vec![D::Integer(99999)]).items);
    Descriptor::new("null")
        .with("null", target())
        .with("artboard", D::Descriptor(board))
        .with("changeSizes", D::Integer(change_sizes))
        .with("changeBackground", D::Integer(1))
}

/// Background edits preserve all wire fields, target geometry, children and guides, with one Undo.
#[test]
fn artboard_background_edit_roundtrips_without_replaying_recorded_geometry() {
    for (kind, expected) in [(1, [1.0; 4]), (2, [0.0, 0.0, 0.0, 1.0]), (3, [0.0; 4]), (4, [0.25, 0.5, 0.75, 1.0])] {
        let mut s = artboard_fixture();
        s.execute("layer.new.artboardFromGroup", json!({})).unwrap();
        Arc::make_mut(&mut s.active_mut().unwrap().doc).guides.horizontal.push(3.0);
        let before = s.active().unwrap().doc.clone();
        let id = s.active().unwrap().active_layer.unwrap();
        let history = s.active().unwrap().history.past_len();
        let d = artboard_edit_descriptor(kind, 8);
        assert_eq!(load(&mut s, &file("editArtboardEvent", d.clone()))["unsupported"], json!([]));
        assert_eq!(s.actions.list[0].steps[0].0, "layer.artboard.set");
        assert_eq!(saved_item(&mut s).descriptor, Some(d));
        play(&mut s);
        let doc = &s.active().unwrap().doc;
        let board = doc.layer(id).unwrap().artboard().unwrap();
        assert_eq!(board.rect, before.layer(id).unwrap().artboard().unwrap().rect);
        assert_eq!(board.preset, before.layer(id).unwrap().artboard().unwrap().preset);
        assert_eq!(doc.layer(id).unwrap().children(), before.layer(id).unwrap().children());
        assert_eq!(doc.size, before.size);
        assert_eq!(doc.guides, before.guides);
        let pixel = photocraft_compose::render(doc, photocraft_doc::Rect::new(3, 3, 4, 4)).px[0];
        for (actual, expected) in pixel.into_iter().zip(expected) {
            assert!((actual - expected).abs() < 0.005, "{kind}: {pixel:?}");
        }
        assert_eq!(s.active().unwrap().history.past_len(), history + 1);
        assert!(s.undo());
        assert_eq!(s.active().unwrap().doc, before);
    }
}

/// Unmeasured geometry modes remain whole; edited native resize options cannot be silently omitted.
#[test]
fn artboard_edit_refuses_unmeasured_modes_and_lossy_native_exports() {
    let mut s = artboard_fixture();
    for d in [artboard_edit_descriptor(1, 4), artboard_edit_descriptor(5, 8), artboard_edit_descriptor(1, 8).with("extra", D::Integer(1))] {
        let before = s.active().unwrap().doc.clone();
        let history = s.active().unwrap().history.past_len();
        assert_eq!(load(&mut s, &file("editArtboardEvent", d.clone()))["unsupported"].as_array().unwrap().len(), 1);
        assert_eq!(saved_item(&mut s).descriptor, Some(d));
        let result = s.execute("actions.play", json!({"set":"Links","action":"A"})).unwrap();
        assert_eq!(result["ran"], 0);
        assert_eq!(s.active().unwrap().doc, before);
        assert_eq!(s.active().unwrap().history.past_len(), history);
    }
    load(&mut s, &file("editArtboardEvent", artboard_edit_descriptor(4, 8)));
    let params = s.actions.list[0].steps[0].1.clone();
    for (key, value) in [("rect", json!([0, 0, 100, 100])), ("preset", json!("iPhone 14")), ("changeSizes", json!(4)), ("changeBackground", json!(0))] {
        let mut edited = params.clone();
        edited[key] = value;
        assert!(atn_cmds::export("Links", &[&Action::new("A", vec![("layer.artboard.set".into(), edited)])], &[], false).is_err());
    }
}

#[test]
fn reverse_linked_selection_and_clear_style_events_do_their_edits() {
    for explicit in [false, true] {
        let mut s = fixture(&[None, None, None], &[0, 1, 2]);
        let before = s.active().unwrap().doc.clone();
        let ids: Vec<_> = before.layers.iter().map(|layer| layer.id).rev().collect();
        load_targeted(&mut s, "reverse", "layer.arrange.reverse", explicit);
        play(&mut s);
        assert_eq!(s.active().unwrap().doc.layers.iter().map(|layer| layer.id).collect::<Vec<_>>(), ids);
        assert!(s.undo());
        assert_eq!(s.active().unwrap().doc, before);

        let mut s = fixture(&[Some(77), Some(77), None], &[0]);
        let ids: Vec<_> = s.active().unwrap().doc.layers[..2].iter().map(|layer| layer.id).collect();
        load_targeted(&mut s, "selectLinkedLayers", "layer.selectLinkedLayers", explicit);
        play(&mut s);
        assert_eq!(s.active().unwrap().selected_layers(), ids);

        let mut s = fixture(&[None], &[0]);
        s.execute("layer.layerStyle.colorOverlay", json!({"color": "#ff0000"})).unwrap();
        let before = s.active().unwrap().doc.clone();
        assert_eq!(before.layers[0].effects.items.len(), 1);
        load_targeted(&mut s, "disableLayerStyle", "layer.layerStyle.clear", explicit);
        play(&mut s);
        assert!(s.active().unwrap().doc.layers[0].effects.items.is_empty());
        assert!(s.undo());
        assert_eq!(s.active().unwrap().doc, before);
    }
}

fn smart_target_scene(event: &str, bytes: &[u8], replacement: &[u8], path: &str) -> Session {
    let mut s = smart_fixture(bytes);
    if matches!(event, "placedLayerConvertToEmbedded" | "placedLayerUpdateAllModified") {
        s.execute("layer.smartObjects.convertToLinked", json!({"path": path})).unwrap();
    }
    if event == "placedLayerUpdateAllModified" {
        std::fs::write(path, replacement).unwrap();
    }
    s
}

fn assert_smart_target_edit(event: &str, s: &Session, original_id: photocraft_doc::SmartContentsId, pixels: &[[f32; 4]], bytes: &[u8]) {
    match event {
        "placedLayerMakeCopy" => {
            assert_ne!(active_smart(s).contents_id, original_id);
            assert_eq!(s.active().unwrap().doc.walk().iter().filter(|(_, _, layer)| matches!(layer.content, LayerContent::Smart(_))).count(), 2);
        }
        "placedLayerConvertToLayers" => assert!(s.active().unwrap().doc.walk().iter().all(|(_, _, layer)| !matches!(layer.content, LayerContent::Smart(_)))),
        "placedLayerConvertToEmbedded" => assert!(matches!(&active_smart(s).source, SmartSource::Embedded { bytes: actual, .. } if actual.as_slice() == bytes)),
        "placedLayerUpdateAllModified" => assert!(photocraft_compose::flatten(&s.active().unwrap().doc).px.iter().all(|px| *px == [1.0, 0.0, 0.0, 1.0])),
        _ => unreachable!(),
    }
    if event != "placedLayerUpdateAllModified" {
        assert_eq!(photocraft_compose::flatten(&s.active().unwrap().doc).px, pixels);
    }
}

#[test]
fn smart_target_events_copy_unpack_embed_and_refresh_real_contents() {
    let original = png("#0000ff");
    let replacement = png("#ff0000");
    for &(event, id) in &TARGET_EVENTS[3..7] {
        for explicit in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("linked.png");
            let mut s = smart_target_scene(event, &original, &replacement, path.to_str().unwrap());
            let before = s.active().unwrap().doc.clone();
            let pixels = photocraft_compose::flatten(&before).into_px();
            let contents_id = active_smart(&s).contents_id;
            load_targeted(&mut s, event, id, explicit);
            play(&mut s);
            assert_smart_target_edit(event, &s, contents_id, &pixels, &original);
            assert!(s.undo());
            assert_eq!(s.active().unwrap().doc, before);
        }
    }
}

#[test]
fn rasterize_all_event_visits_nested_and_top_level_smart_layers() {
    for explicit in [false, true] {
        let mut s = smart_fixture(&png("#0000ff"));
        let first = s.active().unwrap().active_layer.unwrap();
        s.execute("layer.smartObjects.newSmartObjectViaCopy", json!({})).unwrap();
        let doc = Arc::make_mut(&mut s.active_mut().unwrap().doc);
        let nested = doc.remove(first).unwrap();
        doc.layers.push(Layer::group("Nested", vec![nested]));
        let before = s.active().unwrap().doc.clone();
        let pixels = photocraft_compose::flatten(&before).into_px();
        load_targeted(&mut s, "rasterizeAll", "layer.rasterize.allLayers", explicit);
        play(&mut s);
        assert!(s.active().unwrap().doc.walk().iter().all(|(_, _, layer)| !matches!(layer.content, LayerContent::Smart(_))));
        assert_eq!(photocraft_compose::flatten(&s.active().unwrap().doc).px, pixels);
        assert!(s.undo());
        assert_eq!(s.active().unwrap().doc, before);
    }
}

#[test]
fn video_and_isolate_events_use_current_document_state() {
    for explicit in [false, true] {
        let mut s = fixture(&[None], &[0]);
        let before = s.active().unwrap().doc.clone();
        load_targeted(&mut s, "addBlankVideoLayer", "layer.videoLayers.newBlankVideoLayer", explicit);
        play(&mut s);
        let st = s.active().unwrap();
        let video = st.doc.layer(st.active_layer.unwrap()).unwrap().video.as_ref().unwrap();
        assert_eq!(video.frames.len(), 1);
        assert_eq!(video.fps, 30.0);
        assert!(video.frames[0].content_bounds().is_empty());
        assert!(s.undo());
        assert_eq!(s.active().unwrap().doc, before);

        let mut s = fixture(&[None, None, None], &[0, 2]);
        let before = s.active().unwrap().doc.clone();
        let ids = s.active().unwrap().selected_layers();
        let history = s.active().unwrap().history.past_len();
        load_targeted(&mut s, "isolateLayers", "select.isolateLayers", explicit);
        play(&mut s);
        assert_eq!(s.active().unwrap().isolated_layers, ids);
        play(&mut s);
        assert!(s.active().unwrap().isolated_layers.is_empty());
        assert_eq!(s.active().unwrap().doc, before);
        assert_eq!(s.active().unwrap().history.past_len(), history);
    }
}

fn active_text(s: &Session) -> &photocraft_doc::TextLayer {
    let st = s.active().unwrap();
    let LayerContent::Text(text) = &st.doc.layer(st.active_layer.unwrap()).unwrap().content else { panic!("expected Type") };
    text
}

#[test]
fn type_default_events_save_then_restore_changed_styles() {
    for explicit in [false, true] {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 32, "height": 32})).unwrap();
        s.execute("type.create", json!({"text": "Hi", "size": 14})).unwrap();
        assert!(s.type_defaults.is_none());
        load_targeted(&mut s, "textSaveDefaultTypeStyles", "type.saveDefaultTypeStyles", explicit);
        play(&mut s);
        assert_eq!(s.type_defaults.as_ref().unwrap().0.size_pt, 14.0);
        s.execute("type.setStyle", json!({"size": 28})).unwrap();
        assert_eq!(active_text(&s).char_runs()[0].style.size_pt, 28.0);
        let before = s.active().unwrap().doc.clone();
        load_targeted(&mut s, "textLoadDefaultTypeStyles", "type.loadDefaultTypeStyles", explicit);
        play(&mut s);
        assert_eq!(active_text(&s).char_runs()[0].style.size_pt, 14.0);
        assert!(s.undo());
        assert_eq!(s.active().unwrap().doc, before);
    }
}

#[test]
fn measurement_event_records_actual_selected_area() {
    for explicit in [false, true] {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 4, "height": 4, "background": "white"})).unwrap();
        s.execute("select.rect", json!({"x": 0, "y": 0, "width": 2, "height": 2})).unwrap();
        let before = s.active().unwrap().doc.clone();
        load_targeted(&mut s, "recordMeasurements", "image.analysis.recordMeasurements", explicit);
        play(&mut s);
        assert_eq!(s.analysis.log.len(), 1);
        assert_eq!(s.analysis.log[0].values["source"], "Selection");
        assert_eq!(s.analysis.log[0].values["area"].as_f64(), Some(4.0));
        assert_eq!(s.active().unwrap().doc, before);
    }
}

fn class_reference(class_id: &str) -> D {
    D::Reference(vec![ReferenceItem::Class(Class { name: UnicodeString::new_nul(""), class_id: Id::new(class_id) })])
}

fn make_type_descriptor() -> Descriptor {
    Descriptor::new("Mk  ").with("null", class_reference("contentLayer")).with("Usng", target_class("textLayer"))
}

#[test]
fn make_type_shape_roundtrips_and_converts_actual_glyphs_with_undo() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 128, "height": 80, "background": "white"})).unwrap();
    let id = LayerId(s.execute("type.create", json!({"text": "oil", "size": 24, "x": 8, "y": 48})).unwrap()["layer"].as_u64().unwrap());
    Arc::make_mut(&mut s.active_mut().unwrap().doc).layer_mut(id).unwrap().psd_blocks.push((*b"TySh", Arc::new(vec![1, 2, 3])));
    let before = s.active().unwrap().doc.clone();
    assert_eq!(load(&mut s, &file("Mk  ", make_type_descriptor()))["unsupported"], json!([]));
    assert_eq!(s.actions.list[0].steps, [("type.convertToShape".into(), json!({}))]);
    let saved = saved_item(&mut s);
    assert_eq!(saved.event, AtnEvent::Code(*b"Mk  "));
    let mut canonical = make_type_descriptor();
    canonical.class_id = Id::new("null");
    assert_eq!(saved.descriptor, Some(canonical.clone()));
    assert_eq!(load(&mut s, &file("Mk  ", canonical))["unsupported"], json!([]));
    play(&mut s);
    let layer = s.active().unwrap().doc.layer(id).unwrap();
    let LayerContent::Shape(shape) = &layer.content else { panic!("Type was not converted to Shape") };
    assert!(!shape.path.subpaths.is_empty());
    assert!(!layer.psd_blocks.iter().any(|(key, _)| key == b"TySh"));
    assert!(s.undo());
    assert_eq!(s.active().unwrap().doc, before);
}

#[test]
fn make_type_shape_keeps_wrong_references_and_preserves_fill_mappings() {
    for descriptor in [
        Descriptor::new("null").with("null", class_reference("contentLayer")),
        Descriptor::new("null").with("null", class_reference("contentLayer")).with("Usng", target_class("Lyr ")),
        Descriptor::new("null").with("null", class_reference("contentLayer")).with("Usng", ordinal_target("textLayer", "Frnt")),
        make_type_descriptor().with("extra", D::Boolean(true)),
        make_type_descriptor().with("Usng", target_class("textLayer")),
    ] {
        let mut s = Session::new();
        assert_eq!(load(&mut s, &file("Mk  ", descriptor.clone()))["unsupported"].as_array().unwrap().len(), 1);
        assert_eq!(saved_item(&mut s).descriptor, Some(descriptor));
    }
    for (id, p) in [("layer.newFillLayer.solidColor", json!({"color": "#ff0000"})), ("layer.newFillLayer.pattern", json!({"pattern": "Grid"}))] {
        let action = Action::new("A", vec![(id.into(), p)]);
        let (bytes, skipped) = atn_cmds::export("Links", &[&action], &[], false).unwrap();
        assert!(skipped.is_empty());
        let mut s = Session::new();
        assert_eq!(load(&mut s, &bytes)["unsupported"], json!([]));
        assert_eq!(s.actions.list[0].steps[0].0, id);
    }
    let action = Action::new("A", vec![("type.convertToShape".into(), json!({"layer": 1}))]);
    assert!(atn_cmds::export("Links", &[&action], &[], false).is_err());
}

fn radius_descriptor(radius: f64) -> Descriptor {
    Descriptor::new("shapeBlur").with("Rds ", D::UnitFloat { unit: *b"#Pxl", value: radius })
}

fn impulse_fixture() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 9, "height": 9, "background": "transparent"})).unwrap();
    s.edit("impulse fixture", |doc, active| {
        doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap().fill_rect(photocraft_doc::Rect::new(4, 4, 5, 5), &[1.0, 1.0, 1.0, 1.0]);
        Ok(())
    })
    .unwrap();
    s
}

#[test]
fn radius_only_shape_blur_roundtrips_and_changes_pixels_including_default() {
    for radius in [None, Some(5.0)] {
        let mut s = impulse_fixture();
        let d = radius.map_or_else(|| Descriptor::new("shapeBlur"), radius_descriptor);
        assert_eq!(load(&mut s, &file("shapeBlur", d.clone()))["unsupported"], json!([]));
        let saved = saved_item(&mut s);
        assert_eq!(saved.event, AtnEvent::name("shapeBlur"));
        let mut canonical = d;
        canonical.class_id = Id::new("null");
        assert_eq!(saved.descriptor, Some(canonical.clone()));
        assert_eq!(load(&mut s, &file("shapeBlur", canonical))["unsupported"], json!([]));
        let before = s.active().unwrap().doc.clone();
        let pixels = photocraft_compose::flatten(&before).into_px();
        play(&mut s);
        let blurred = photocraft_compose::flatten(&s.active().unwrap().doc).into_px();
        assert_ne!(blurred, pixels);
        let mut direct = impulse_fixture();
        direct.execute("filter.blur.shapeBlur", json!({"radius": radius.unwrap_or(10.0)})).unwrap();
        assert_eq!(blurred, photocraft_compose::flatten(&direct.active().unwrap().doc).px);
        assert!(s.undo());
        assert_eq!(s.active().unwrap().doc, before);
    }
}

#[test]
fn shape_blur_radius_boundaries_keep_their_value_without_running_large_kernels() {
    for radius in [5.0, 10.0, 1000.0] {
        let mut s = Session::new();
        let mut canonical = radius_descriptor(radius);
        assert_eq!(load(&mut s, &file("shapeBlur", canonical.clone()))["unsupported"], json!([]));
        assert_eq!(s.actions.list[0].steps[0].1["radius"].as_f64(), Some(radius));
        canonical.class_id = Id::new("null");
        assert_eq!(saved_item(&mut s).descriptor, Some(canonical));
    }
}

#[test]
fn shape_blur_keeps_custom_assets_and_invalid_radius_forms_whole() {
    let custom = Descriptor::new("customShape").with("Nm  ", D::Text(UnicodeString::new_nul("Star"))).with("Idnt", D::Text(UnicodeString::new_nul("asset-id")));
    for descriptor in [
        radius_descriptor(4.0),
        radius_descriptor(1001.0),
        radius_descriptor(f64::NAN),
        radius_descriptor(f64::INFINITY),
        Descriptor::new("shapeBlur").with("Rds ", D::UnitFloat { unit: *b"#Prc", value: 10.0 }),
        radius_descriptor(10.0).with("customShape", D::Descriptor(custom)).with("keyOriginType", D::Integer(9)),
        radius_descriptor(10.0).with("extra", D::Boolean(true)),
    ] {
        let mut s = Session::new();
        assert_eq!(load(&mut s, &file("shapeBlur", descriptor.clone()))["unsupported"].as_array().unwrap().len(), 1);
        assert_eq!(saved_item(&mut s).descriptor.unwrap().to_bytes(), descriptor.to_bytes());
    }
    for p in [json!({"radius": 0}), json!({"radius": 1001}), json!({"radius": null}), json!({"radius": 10, "shape": "circle"}), json!({"target": "mask"})] {
        let action = Action::new("A", vec![("filter.blur.shapeBlur".into(), p)]);
        assert!(atn_cmds::export("Links", &[&action], &[], false).is_err());
    }
}

#[test]
fn link_descriptor_headers_normalize_without_changing_meaningful_keys() {
    for event in ["linkSelectedLayers", "unlinkSelectedLayers"] {
        let mut s = Session::new();
        let descriptor = Descriptor::new(event).with("null", target());
        assert_eq!(load(&mut s, &file(event, descriptor))["unsupported"], json!([]));
        let saved = s.execute("actions.save", json!({"set": "Links"})).unwrap();
        let bytes = photocraft_engine::paint::tile::b64_decode(saved["data"].as_str().unwrap()).unwrap();
        let parsed = atn::parse(&bytes).unwrap();
        assert_eq!(parsed.actions[0].items[0].event, AtnEvent::name(event));
        assert_eq!(parsed.actions[0].items[0].descriptor, Some(Descriptor::new("null").with("null", target())));
    }
}

const SMART_PATHS: &[(&str, &str)] = &[
    ("placedLayerExportContents", "layer.smartObjects.exportContents"),
    ("placedLayerConvertToLinked", "layer.smartObjects.convertToLinked"),
    ("placedLayerRelinkToFile", "layer.smartObjects.relinkToFile"),
    ("placedLayerReplaceContents", "layer.smartObjects.replaceContents"),
];

fn path_value(path: &str) -> D {
    let units: Vec<_> = path.encode_utf16().chain(std::iter::once(0)).collect();
    let mut bytes = b"txtu".to_vec();
    bytes.extend_from_slice(&u32::try_from(12 + units.len() * 2).unwrap().to_le_bytes());
    bytes.extend_from_slice(&u32::try_from(units.len()).unwrap().to_le_bytes());
    bytes.extend(units.iter().flat_map(|unit| unit.to_le_bytes()));
    D::Path(bytes)
}

fn smart_path_descriptor(event: &str, path: &str) -> Descriptor {
    if event == "placedLayerConvertToLinked" {
        Descriptor::new("null").with("null", target()).with("Usng", path_value(path))
    } else {
        Descriptor::new("null").with("null", path_value(path))
    }
}

fn png(color: &str) -> Vec<u8> {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 4, "height": 4, "background": color})).unwrap();
    photocraft_io::export(&s.active().unwrap().doc, "png", &Default::default()).unwrap().bytes
}

fn smart_fixture(bytes: &[u8]) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 4, "height": 4})).unwrap();
    photocraft_engine::file_cmds::place_bytes(&mut s, "original.png", bytes.to_vec(), None, &json!({})).unwrap();
    s
}

fn active_smart(s: &Session) -> &SmartObject {
    let st = s.active().unwrap();
    let LayerContent::Smart(smart) = &st.doc.layer(st.active_layer.unwrap()).unwrap().content else { panic!("expected a Smart Object") };
    smart
}

fn assert_smart_file_output(event: &str, s: &Session, path: &str, original: &[u8], replacement: &[u8]) {
    match event {
        "placedLayerExportContents" => assert_eq!(std::fs::read(path).unwrap().as_slice(), original),
        "placedLayerConvertToLinked" => {
            assert_eq!(std::fs::read(path).unwrap().as_slice(), original);
            assert!(matches!(&active_smart(s).source, SmartSource::Linked { path: actual } if actual == path));
        }
        "placedLayerRelinkToFile" => assert!(matches!(&active_smart(s).source, SmartSource::Linked { path: actual } if actual == path)),
        "placedLayerReplaceContents" => {
            assert!(matches!(&active_smart(s).source, SmartSource::Embedded { bytes, .. } if bytes.as_slice() == replacement));
        }
        _ => unreachable!(),
    }
}

#[test]
fn smart_paths_roundtrip_and_play_real_files_on_the_current_target() {
    let original = png("#0000ff");
    let replacement = png("#ff0000");
    for &(event, id) in SMART_PATHS {
        for header in ["null", event] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("contents Ω.png");
            let path = path.to_str().unwrap();
            let replacing = matches!(event, "placedLayerRelinkToFile" | "placedLayerReplaceContents");
            if replacing {
                std::fs::write(path, &replacement).unwrap();
            }
            let canonical = smart_path_descriptor(event, path);
            let mut recorded = canonical.clone();
            recorded.class_id = Id::new(header);
            let mut s = smart_fixture(&original);
            if replacing {
                let hint = i32::MAX;
                assert!(s.active().unwrap().doc.walk().iter().all(|(_, _, layer)| layer.id.0 != hint as u64));
                recorded = recorded.with("LyrI", D::Integer(hint));
            }
            assert_eq!(load(&mut s, &file(event, recorded))["unsupported"], json!([]), "{event}");
            assert_eq!(s.actions.list[0].steps, [(id.to_string(), json!({"path": path}))]);
            let saved = s.execute("actions.save", json!({"set": "Links"})).unwrap();
            assert_eq!(saved["skipped"], json!([]));
            let bytes = photocraft_engine::paint::tile::b64_decode(saved["data"].as_str().unwrap()).unwrap();
            let parsed = atn::parse(&bytes).unwrap();
            assert_eq!(parsed.actions[0].items[0].event, AtnEvent::name(event));
            // Reference #10603, docs/atn.md:197–201: normalize the descriptor class,
            // and omit session LyrI. All meaningful event/path keys remain identical.
            assert_eq!(parsed.actions[0].items[0].descriptor, Some(canonical));
            assert_eq!(load(&mut s, &bytes)["unsupported"], json!([]));
            let before = s.active().unwrap().doc.clone();
            let before_smart = active_smart(&s).clone();
            play(&mut s);
            assert_eq!(active_smart(&s).transform, before_smart.transform);
            assert_smart_file_output(event, &s, path, &original, &replacement);
            if replacing {
                assert!(photocraft_compose::flatten(&s.active().unwrap().doc).px.iter().all(|px| *px == [1.0, 0.0, 0.0, 1.0]));
            }
            if event == "placedLayerExportContents" {
                assert_eq!(s.active().unwrap().doc, before);
            } else {
                assert!(s.undo());
                assert_eq!(s.active().unwrap().doc, before);
            }
        }
    }
}

#[test]
fn smart_paths_keep_missing_unusable_and_unrepresented_descriptors_whole() {
    for &(event, _) in SMART_PATHS {
        let key = if event == "placedLayerConvertToLinked" { "Usng" } else { "null" };
        for descriptor in [
            Descriptor::new("null"),
            Descriptor::new("null").with(key, path_value("")),
            Descriptor::new("null").with(key, path_value("a\0b")),
            Descriptor::new("null").with(key, D::Text(UnicodeString::new("/tmp/file.png"))),
            Descriptor::new("null").with(key, D::Path(b"txtu".to_vec())),
            Descriptor::new("null").with("Usng", path_value("/tmp/file.png")),
            smart_path_descriptor(event, "/tmp/file.png").with("extra", D::Integer(1)),
            smart_path_descriptor(event, "/tmp/file.png").with("LyrI", D::Boolean(true)),
            smart_path_descriptor(event, "/tmp/file.png").with("LyrI", D::Integer(-1)),
            smart_path_descriptor(event, "/tmp/file.png").with("LyrI", D::LargeInteger(i64::from(i32::MAX) + 1)),
        ] {
            let mut s = Session::new();
            assert_eq!(load(&mut s, &file(event, descriptor.clone()))["unsupported"].as_array().unwrap().len(), 1);
            let saved = s.execute("actions.save", json!({"set": "Links"})).unwrap();
            let bytes = photocraft_engine::paint::tile::b64_decode(saved["data"].as_str().unwrap()).unwrap();
            assert_eq!(atn::parse(&bytes).unwrap().actions[0].items[0].descriptor, Some(descriptor));
        }
    }
}

#[test]
fn smart_path_exports_refuse_unrepresentable_native_params() {
    for &(_, id) in SMART_PATHS {
        for params in [json!({}), json!({"path": ""}), json!({"path": "a\0b"}), json!({"path": 3}), json!({"path": "/tmp/a.png", "layer": 1})] {
            let action = Action::new("A", vec![(id.into(), params)]);
            assert!(atn_cmds::export("Links", &[&action], &[], false).is_err());
        }
    }
}

#[test]
fn unusable_smart_file_at_playback_fails_without_changing_the_document() {
    let original = png("#0000ff");
    let dir = tempfile::tempdir().unwrap();
    for &(event, id) in SMART_PATHS {
        let mut s = smart_fixture(&original);
        assert_eq!(load(&mut s, &file(event, smart_path_descriptor(event, dir.path().to_str().unwrap())))["unsupported"], json!([]));
        let before = s.active().unwrap().doc.clone();
        let history = s.active().unwrap().history.past_len();
        let report = s.execute("actions.play", json!({"action": "A", "set": "Links"})).unwrap();
        assert_eq!(report["ran"], 0, "{event}: {report}");
        assert_eq!(report["failed"]["id"], id, "{event}: {report}");
        assert!(report["failed"]["error"].as_str().is_some_and(|error| !error.is_empty()), "{event}: {report}");
        assert_eq!(s.active().unwrap().doc, before);
        assert_eq!(s.active().unwrap().history.past_len(), history);
    }
}
