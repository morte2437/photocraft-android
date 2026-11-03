use super::*;
use photocraft_doc::{DocId, Locks, SmartFilter};
use photocraft_geom::Affine;

const RED: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" width="8" height="6"><rect width="8" height="6" fill="red"/></svg>"#;
const BLUE: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" width="8" height="6"><rect width="8" height="6" fill="blue"/></svg>"#;
const GREEN: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" width="8" height="6"><rect width="8" height="6" fill="lime"/></svg>"#;

fn session(depth: u32) -> (Session, LayerId) {
    let mut s = Session::new();
    s.execute("file.new", json!({"width":32,"height":24,"depth":depth})).unwrap();
    let result = crate::file_cmds::place_bytes(&mut s, "Artwork.svg", RED.as_bytes().to_vec(), None, &json!({"smartObject":true})).unwrap();
    let id = LayerId(result["layer"].as_u64().unwrap());
    (s, id)
}

fn request(s: &mut Session, id: LayerId, svg: &str) -> Value {
    let handoff = s.execute(EXPORT, json!({"layer":id.0})).unwrap();
    json!({"document":handoff["document"],"layer":id.0,"svg":svg,"fingerprint":handoff["fingerprint"],"contentsId":handoff["contentsId"]})
}

fn source(s: &Session, id: LayerId) -> Vec<u8> {
    embedded(&s.active().unwrap().doc, smart(&s.active().unwrap().doc, id).unwrap()).unwrap().1.to_vec()
}

fn fails_without_edit(s: &mut Session, params: Value) {
    let before = s.active().unwrap().doc.clone();
    let revision = s.active().unwrap().revision;
    assert!(s.execute(UPDATE, params).is_err());
    assert_eq!(s.active().unwrap().doc, before);
    assert_eq!(s.active().unwrap().revision, revision);
}

#[test]
fn export_and_unchanged_save_have_no_history_step() {
    let (mut s, id) = session(8);
    let revision = s.active().unwrap().revision;
    let params = request(&mut s, id, RED);
    let result = s.execute(UPDATE, params).unwrap();
    assert_eq!(result["updated"], false);
    assert_eq!(s.active().unwrap().revision, revision);
    assert!(s.undo());
    assert!(s.active().unwrap().doc.layer(id).is_none());
}

#[test]
fn deleting_last_svg_object_saves_transparency_and_undo_restores_source() {
    let (mut s, id) = session(8);
    let before = s.active().unwrap().doc.clone();
    let empty = r#"<svg xmlns="http://www.w3.org/2000/svg" width="8" height="6"/>"#;
    let params = request(&mut s, id, empty);
    s.execute(UPDATE, params).unwrap();
    assert_eq!(source(&s, id), empty.as_bytes());
    assert_eq!(smart(&s.active().unwrap().doc, id).unwrap().cache.as_ref().unwrap().pixel(15, 11), vec![0.0; 4]);
    assert!(s.undo());
    assert_eq!(s.active().unwrap().doc, before);
}

#[test]
fn valid_updates_repeat_at_all_depths_and_roundtrip_original_svg() {
    for depth in [8, 16, 32] {
        let (mut s, id) = session(depth);
        let original = s.active().unwrap().doc.clone();
        let journal_len = s.journal.len();
        let params = request(&mut s, id, BLUE);
        let result = s.execute(UPDATE, params).unwrap();
        assert_eq!(result["updated"], true);
        let sm = smart(&s.active().unwrap().doc, id).unwrap();
        assert_eq!(sm.cache.as_ref().unwrap().format(), s.active().unwrap().doc.pixel_format());
        assert_eq!(sm.cache.as_ref().unwrap().pixel(15, 11), vec![0.0, 0.0, 1.0, 1.0]);
        assert_eq!(source(&s, id), BLUE.as_bytes());
        let after = s.active().unwrap().doc.clone();
        assert!(s.undo());
        assert_eq!(s.active().unwrap().doc, original);
        assert!(s.redo());
        assert_eq!(s.active().unwrap().doc, after);
        let params = request(&mut s, id, GREEN);
        s.execute(UPDATE, params).unwrap();
        assert_eq!(source(&s, id), GREEN.as_bytes());
        assert_eq!(s.journal.len(), journal_len, "vector saves must not accumulate full artwork in the replay journal");
        let bytes = photocraft_format::save_to_bytes(&s.active().unwrap().doc, &Default::default()).unwrap();
        let loaded = photocraft_format::load_from_bytes(&bytes).unwrap();
        let loaded_source = &smart(&loaded, id).unwrap().source;
        assert!(matches!(loaded_source, SmartSource::Embedded {file_name,bytes} if file_name == "Artwork.svg" && bytes.as_slice() == GREEN.as_bytes()));
    }
}

#[test]
fn shared_instances_keep_placement_filters_masks_and_one_undo_transaction() {
    let (mut s, id) = session(8);
    let duplicate = {
        let doc = Arc::make_mut(&mut s.active_mut().unwrap().doc);
        let mut layer = doc.layer(id).unwrap().duplicate();
        let LayerContent::Smart(sm) = &mut layer.content else { panic!("smart object") };
        sm.transform = Affine::translate(2.0, 3.0);
        sm.smart_filters.push(SmartFilter {
            command: "filter.invert".into(),
            params: json!({}),
            blend: photocraft_color::BlendMode::Normal,
            opacity: 0.5,
            visible: true,
        });
        sm.filters_enabled = false;
        sm.filter_mask = Some(photocraft_doc::LayerMask {
            surface: photocraft_raster::Surface::new(photocraft_color::PixelFormat::GRAY8),
            enabled: true,
            linked: true,
            density: 0.5,
            feather: 1.0,
        });
        let duplicate = layer.id;
        doc.layers.push(layer);
        duplicate
    };
    let before = s.active().unwrap().doc.clone();
    let preserved = smart(&before, duplicate).unwrap().clone();
    let revision = s.active().unwrap().revision;
    let params = request(&mut s, id, BLUE);
    s.execute(UPDATE, params).unwrap();
    assert_eq!(s.active().unwrap().revision, revision + 1);
    assert_eq!(source(&s, id), source(&s, duplicate));
    let edited = smart(&s.active().unwrap().doc, duplicate).unwrap();
    assert_eq!(edited.transform, preserved.transform);
    assert_eq!(edited.smart_filters, preserved.smart_filters);
    assert_eq!(edited.filter_mask, preserved.filter_mask);
    assert_eq!(edited.filters_enabled, preserved.filters_enabled);
    assert_eq!(edited.contents_id, preserved.contents_id);
    assert_eq!(edited.cache.as_ref().unwrap().pixel(4, 4), vec![0.0, 0.0, 1.0, 1.0]);
    assert!(s.undo());
    assert_eq!(s.active().unwrap().doc, before);
    assert!(s.redo());
    assert_eq!(source(&s, id), BLUE.as_bytes());
}

#[test]
fn invalid_oversized_and_external_resources_are_bounded_atomic() {
    let (mut s, id) = session(8);
    for svg in ["garbage", "<svg>", "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"99999\" height=\"99999\"><rect width=\"99999\" height=\"99999\"/></svg>"]
    {
        let params = request(&mut s, id, svg);
        fails_without_edit(&mut s, params);
    }
    let huge = " ".repeat(super::MAX_VECTOR_SOURCE_BYTES + 1);
    let params = request(&mut s, id, &huge);
    fails_without_edit(&mut s, params);
    // External images cannot trigger ambient filesystem or network access.
    let external = r#"<svg xmlns="http://www.w3.org/2000/svg" width="8" height="6"><image href="file:///secret.png" width="8" height="6"/></svg>"#;
    let params = request(&mut s, id, external);
    let before = s.active().unwrap().doc.clone();
    match s.execute(UPDATE, params) {
        Err(_) => assert_eq!(s.active().unwrap().doc, before),
        Ok(_) => assert_eq!(smart(&s.active().unwrap().doc, id).unwrap().cache.as_ref().unwrap().pixel(15, 11), vec![0.0; 4]),
    }
}

#[test]
fn stale_hash_content_identity_wrong_parent_deleted_and_locked_reject() {
    let (mut s, id) = session(8);
    let params = request(&mut s, id, BLUE);
    for (key, value) in [("fingerprint", json!("0".repeat(64))), ("contentsId", json!(0)), ("document", json!(DocId::fresh().0)), ("layer", json!("wrong"))] {
        let mut bad = params.clone();
        bad[key] = value;
        fails_without_edit(&mut s, bad);
    }
    let mut missing = params.clone();
    missing.as_object_mut().unwrap().remove("document");
    fails_without_edit(&mut s, missing);
    Arc::make_mut(&mut s.active_mut().unwrap().doc).layer_mut(id).unwrap().locks = Locks { all: true, ..Default::default() };
    fails_without_edit(&mut s, params.clone());
    Arc::make_mut(&mut s.active_mut().unwrap().doc).layer_mut(id).unwrap().locks = Default::default();
    s.execute(UPDATE, params.clone()).unwrap();
    fails_without_edit(&mut s, params.clone());
    Arc::make_mut(&mut s.active_mut().unwrap().doc).remove(id).unwrap();
    fails_without_edit(&mut s, params);
}

#[test]
fn linked_sources_and_internal_editor_conflicts_reject_without_reads() {
    let (mut s, id) = session(8);
    let params = request(&mut s, id, BLUE);
    let parent = s.active().unwrap().doc.id;
    let contents_id = smart(&s.active().unwrap().doc, id).unwrap().contents_id;
    s.smart_links.push(crate::smart_cmds::SmartLink { child: DocId::fresh(), parent, layer: id, contents_id });
    fails_without_edit(&mut s, params.clone());
    assert!(s.execute(EXPORT, json!({"layer":id.0})).is_err());
    s.smart_links.clear();
    let doc = Arc::make_mut(&mut s.active_mut().unwrap().doc);
    let LayerContent::Smart(sm) = &mut doc.layer_mut(id).unwrap().content else { panic!("smart object") };
    sm.source = SmartSource::Linked { path: "C:/not-authorized/source.svg".into() };
    fails_without_edit(&mut s, params);
    assert!(s.execute(EXPORT, json!({"layer":id.0})).is_err());
}

#[test]
fn inherited_shared_instance_lock_and_unbounded_placement_reject() {
    let (mut s, id) = session(8);
    let params = request(&mut s, id, BLUE);
    {
        let doc = Arc::make_mut(&mut s.active_mut().unwrap().doc);
        let mut duplicate = doc.layer(id).unwrap().duplicate();
        duplicate.locks.pixels = true;
        doc.layers.push(duplicate);
    }
    fails_without_edit(&mut s, params.clone());
    {
        let doc = Arc::make_mut(&mut s.active_mut().unwrap().doc);
        doc.layers.pop();
        let LayerContent::Smart(sm) = &mut doc.layer_mut(id).unwrap().content else { panic!("smart object") };
        sm.transform = Affine { m: [10000.0, 0.0, 0.0, 10000.0, 0.0, 0.0] };
    }
    fails_without_edit(&mut s, params);
}

#[test]
fn diverged_shared_source_is_never_overwritten() {
    let (mut s, id) = session(8);
    let params = request(&mut s, id, BLUE);
    let doc = Arc::make_mut(&mut s.active_mut().unwrap().doc);
    let mut duplicate = doc.layer(id).unwrap().duplicate();
    let LayerContent::Smart(sm) = &mut duplicate.content else { panic!("smart object") };
    sm.source = SmartSource::Embedded { file_name: "Other.svg".into(), bytes: Arc::new(GREEN.as_bytes().to_vec()) };
    doc.layers.push(duplicate);
    fails_without_edit(&mut s, params);
}

#[test]
fn failed_second_instance_render_does_not_commit_first_instance() {
    let (mut s, id) = session(8);
    {
        let doc = Arc::make_mut(&mut s.active_mut().unwrap().doc);
        let mut duplicate = doc.layer(id).unwrap().duplicate();
        let LayerContent::Smart(sm) = &mut duplicate.content else { panic!("smart object") };
        sm.transform = Affine { m: [100.0, 0.0, 0.0, 0.001, 0.0, 0.0] };
        doc.layers.push(duplicate);
        let LayerContent::Smart(sm) = &mut doc.layer_mut(id).unwrap().content else { panic!("smart object") };
        sm.transform = Affine { m: [0.001, 0.0, 0.0, 0.001, 0.0, 0.0] };
    }
    let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="2000" height="2000"><rect width="2000" height="2000" fill="blue"/></svg>"#;
    let params = request(&mut s, id, svg);
    fails_without_edit(&mut s, params);
}

#[test]
fn inherited_parent_lock_refuses_content_edits() {
    let (mut s, id) = session(8);
    let params = request(&mut s, id, BLUE);
    let doc = Arc::make_mut(&mut s.active_mut().unwrap().doc);
    let layer = doc.remove(id).unwrap();
    let mut group =
        Layer::new("Locked parent", LayerContent::Group(photocraft_doc::Group { children: vec![layer], expanded: true, artboard: None, frame_stroke: None }));
    group.locks.pixels = true;
    doc.layers.push(group);
    fails_without_edit(&mut s, params);
}

#[test]
fn perspective_and_warp_geometry_survive_an_update() {
    for warp in [false, true] {
        let (mut s, id) = session(8);
        let doc = Arc::make_mut(&mut s.active_mut().unwrap().doc);
        let LayerContent::Smart(sm) = &mut doc.layer_mut(id).unwrap().content else { panic!("smart object") };
        sm.perspective = Some([1.0, 0.0, 12.0, 0.0, 1.0, 9.0, 0.01, 0.0, 1.0]);
        if warp {
            sm.warp = Some(photocraft_geom::warp::Warp::preset(photocraft_geom::warp::WarpStyle::Arc, 10.0, [0.0, 0.0, 8.0, 6.0]));
        }
        let before = sm.clone();
        let params = request(&mut s, id, BLUE);
        s.execute(UPDATE, params).unwrap();
        let after = smart(&s.active().unwrap().doc, id).unwrap();
        assert_eq!(after.warp, before.warp);
        assert_eq!(after.perspective, before.perspective);
        assert_eq!(after.transform, before.transform);
        assert_eq!(source(&s, id), BLUE.as_bytes());
    }
}

#[test]
fn perspective_crossing_zero_is_rejected_before_rendering() {
    let (mut s, id) = session(8);
    let params = request(&mut s, id, BLUE);
    let doc = Arc::make_mut(&mut s.active_mut().unwrap().doc);
    let LayerContent::Smart(sm) = &mut doc.layer_mut(id).unwrap().content else { panic!("smart object") };
    sm.perspective = Some([1.0, 0.0, 0.0, 0.0, 1.0, 0.0, -0.5, 0.0, 1.0]);
    fails_without_edit(&mut s, params);
}

#[test]
fn bad_params_are_errors_for_both_commands() {
    let (mut s, _) = session(8);
    for p in [Value::Null, json!([]), json!({"layer":-1}), json!({"layer":"invalid"})] {
        assert!(s.execute(EXPORT, p.clone()).is_err());
        fails_without_edit(&mut s, p);
    }
    fails_without_edit(&mut s, json!({}));
}

/// Embedded PSD link-table vectors are exported in memory, then atomically replaced by SVG.
/// The original payload and shared identities survive undo and native-format save/reload.
#[test]
fn embedded_pdf_ai_and_eps_update_shared_instances_without_reading_linked_paths() {
    for depth in [8, 16, 32] {
        for (name, bytes) in [("Logo.ai", b"%PDF-1.6\nowned synthetic payload".as_slice()), ("Logo.eps", b"%!PS-Adobe-3.0 EPSF-3.0\n".as_slice())] {
            let (mut s, id) = session(depth);
            let duplicate;
            {
                let doc = Arc::make_mut(&mut s.active_mut().unwrap().doc);
                let payload = photocraft_io::linked::encode_linked_file(&photocraft_io::linked::LinkedFile {
                    uuid: "owned-vector".into(),
                    file_name: name.into(),
                    bytes: bytes.to_vec(),
                });
                doc.metadata.psd_global_blocks.push((*b"8BIM", *b"lnk2", Arc::new(payload)));
                let LayerContent::Smart(sm) = &mut doc.layer_mut(id).unwrap().content else { panic!("smart") };
                sm.source = SmartSource::Linked { path: "owned-vector".into() };
                let layer = doc.layer(id).unwrap().duplicate();
                duplicate = layer.id;
                doc.layers.push(layer);
            }
            let before = s.active().unwrap().doc.clone();
            let exported = s.execute(EXPORT, json!({"layer":id.0})).unwrap();
            assert_eq!(base64::engine::general_purpose::STANDARD.decode(exported["sourceBase64"].as_str().unwrap()).unwrap(), bytes);
            assert_eq!(exported["extension"], if name.ends_with("ai") { "ai" } else { "eps" });
            assert_eq!(s.active().unwrap().doc, before);
            // updateSvg refuses native sources; they save back in their own format.
            let svg_params =
                json!({"document":exported["document"],"layer":id.0,"svg":BLUE,"fingerprint":exported["fingerprint"],"contentsId":exported["contentsId"]});
            fails_without_edit(&mut s, svg_params);
            let changed = [bytes, b" edited"].concat();
            let params = json!({"document":exported["document"],"layer":id.0,"fingerprint":exported["fingerprint"],"contentsId":exported["contentsId"],
                "sourceBase64":base64::engine::general_purpose::STANDARD.encode(&changed),"extension":exported["extension"],"previewSvg":BLUE});
            assert_eq!(s.execute(UPDATE_VECTOR, params.clone()).unwrap()["updated"], true);
            for id in [id, duplicate] {
                assert_eq!(source(&s, id), changed);
            }
            let after = s.active().unwrap().doc.clone();
            assert!(s.undo());
            assert_eq!(s.active().unwrap().doc, before);
            assert!(s.redo());
            assert_eq!(s.active().unwrap().doc, after);
            let before_stale = s.active().unwrap().doc.clone();
            assert!(s.execute(UPDATE_VECTOR, params).is_err(), "stale first-save fingerprint");
            assert_eq!(s.active().unwrap().doc, before_stale);
            let serialized = photocraft_format::save_to_bytes(&after, &Default::default()).unwrap();
            assert_eq!(photocraft_format::load_from_bytes(&serialized).unwrap(), *after);
        }
    }
}

#[test]
fn vector_handoff_never_reads_a_linked_disk_path_or_accepts_disguised_raster() {
    let (mut s, id) = session(8);
    for source in
        [SmartSource::Linked { path: "/dev/zero".into() }, SmartSource::Embedded { file_name: "Logo.ai".into(), bytes: Arc::new(b"not a vector".to_vec()) }]
    {
        let doc = Arc::make_mut(&mut s.active_mut().unwrap().doc);
        let LayerContent::Smart(sm) = &mut doc.layer_mut(id).unwrap().content else { panic!("smart") };
        sm.source = source;
        let before = s.active().unwrap().doc.clone();
        assert!(s.execute(EXPORT, json!({"layer":id.0})).is_err());
        assert_eq!(s.active().unwrap().doc, before);
    }
}

#[test]
fn quick_mask_blocks_source_updates_without_mutating_the_document() {
    let (mut s, id) = session(8);
    let params = request(&mut s, id, BLUE);
    s.execute("select.editInQuickMaskMode", json!({})).unwrap();
    fails_without_edit(&mut s, params);
}

fn native_session(extension: &str, depth: u32) -> (Session, LayerId, Vec<u8>) {
    let (mut s, id) = session(depth);
    let bytes = if extension == "eps" { b"%!PS-Adobe-3.0 EPSF-3.0\noriginal".to_vec() } else { b"%PDF-1.7\noriginal".to_vec() };
    let doc = Arc::make_mut(&mut s.active_mut().unwrap().doc);
    let LayerContent::Smart(sm) = &mut doc.layer_mut(id).unwrap().content else { panic!("smart") };
    sm.source = SmartSource::Embedded { file_name: format!("Logo.{extension}"), bytes: Arc::new(bytes.clone()) };
    (s, id, bytes)
}

fn native_request(s: &mut Session, id: LayerId, ext: &str, bytes: &[u8]) -> Value {
    let handoff = s.execute(EXPORT, json!({"layer":id.0})).unwrap();
    json!({"document":handoff["document"],"layer":id.0,"contentsId":handoff["contentsId"],"fingerprint":handoff["fingerprint"],"sourceBase64":base64::engine::general_purpose::STANDARD.encode(bytes),"extension":ext,"previewSvg":BLUE})
}

#[test]
fn original_vector_formats_save_atomically_and_survive_undo_and_persistence() {
    for ext in ["ai", "pdf", "eps"] {
        for depth in [8, 16, 32] {
            let (mut s, id, original) = native_session(ext, depth);
            let before = s.active().unwrap().doc.clone();
            let changed = [original.as_slice(), b" edited"].concat();
            let params = native_request(&mut s, id, ext, &changed);
            assert_eq!(s.execute(UPDATE_VECTOR, params.clone()).unwrap()["updated"], true);
            let after = s.active().unwrap().doc.clone();
            let sm = smart(&after, id).unwrap();
            assert!(matches!(&sm.source,SmartSource::Embedded {file_name,bytes} if file_name == &format!("Logo.{ext}") && bytes.as_slice() == changed));
            assert_eq!(sm.vector_preview.as_ref().unwrap().source_hash, *blake3::hash(&changed).as_bytes());
            assert_eq!(sm.cache.as_ref().unwrap().pixel(15, 11), vec![0.0, 0.0, 1.0, 1.0]);
            assert!(s.undo());
            assert_eq!(s.active().unwrap().doc, before);
            assert!(s.redo());
            assert_eq!(s.active().unwrap().doc, after);
            let bundle = photocraft_format::save_to_bytes(&after, &Default::default()).unwrap();
            let reopened = photocraft_format::load_from_bytes(&bundle).unwrap();
            assert_eq!(smart(&reopened, id).unwrap(), sm);
            let mut repeated = native_request(&mut s, id, ext, &changed);
            let revision = s.active().unwrap().revision;
            assert_eq!(s.execute(UPDATE_VECTOR, repeated.clone()).unwrap()["updated"], false);
            assert_eq!(s.active().unwrap().revision, revision);
            repeated["sourceBase64"] = json!(base64::engine::general_purpose::STANDARD.encode([changed.as_slice(), b" twice"].concat()));
            s.execute(UPDATE_VECTOR, repeated).unwrap();
            assert!(s.execute(UPDATE_VECTOR, params).is_err(), "stale session must not overwrite later changes");
        }
    }
}

/// A save from an external editor whose SVG embeds an image PhotoCraft can't read (here a BMP)
/// is accepted with that image left out, as placing the same SVG is.
#[test]
fn saved_svg_with_an_unreadable_embedded_image_still_updates() {
    let (mut s, id) = session(8);
    let edited = r#"<svg xmlns="http://www.w3.org/2000/svg" width="8" height="6"><image width="4" height="6" href="data:image/bmp,BM"/><rect x="4" width="4" height="6" fill="blue"/></svg>"#;
    let params = request(&mut s, id, edited);
    assert_eq!(s.execute(UPDATE, params).unwrap()["updated"], true);
    assert_eq!(source(&s, id), edited.as_bytes());
}

/// updateSvg replaces SVG contents only: it never turns an AI, PDF or EPS source into SVG.
#[test]
fn update_svg_refuses_native_sources() {
    for ext in ["ai", "pdf", "eps"] {
        let (mut s, id, original) = native_session(ext, 8);
        let params = request(&mut s, id, BLUE);
        fails_without_edit(&mut s, params);
        assert_eq!(source(&s, id), original, "{ext}");
    }
}

#[test]
fn native_saves_reject_wrong_formats_bad_previews_and_locks_without_editing() {
    let (mut s, id, original) = native_session("ai", 8);
    let changed = [original.as_slice(), b" edited"].concat();
    let valid = native_request(&mut s, id, "ai", &changed);
    for (key, value) in [
        ("extension", json!("svg")),
        ("sourceBase64", json!("bad base64!")),
        ("sourceBase64", json!(base64::engine::general_purpose::STANDARD.encode(RED))),
        ("previewSvg", json!("not an SVG")),
        ("previewSvg", json!(r#"<svg xmlns="http://www.w3.org/2000/svg" width="1000000" height="1000000"/>"#)),
        // The stored preview must draw completely, as PSD export and reopening require.
        (
            "previewSvg",
            json!(
                r#"<svg xmlns="http://www.w3.org/2000/svg" width="8" height="6"><image width="4" height="6" href="data:image/bmp,BM"/><rect x="4" width="4" height="6" fill="blue"/></svg>"#
            ),
        ),
        ("document", json!(0)),
    ] {
        let mut params = valid.clone();
        params[key] = value;
        let before = s.active().unwrap().doc.clone();
        assert!(s.execute(UPDATE_VECTOR, params).is_err());
        assert_eq!(s.active().unwrap().doc, before);
    }
    Arc::make_mut(&mut s.active_mut().unwrap().doc).layer_mut(id).unwrap().locks.all = true;
    assert!(s.execute(UPDATE_VECTOR, valid.clone()).is_err());
    Arc::make_mut(&mut s.active_mut().unwrap().doc).layer_mut(id).unwrap().locks.all = false;
    let mut duplicate = s.active().unwrap().doc.layer(id).unwrap().duplicate();
    let LayerContent::Smart(sm) = &mut duplicate.content else { panic!("smart") };
    sm.transform = Affine::translate(2.0, 3.0);
    let duplicate_id = Arc::make_mut(&mut s.active_mut().unwrap().doc).insert_above(Some(id), duplicate);
    s.execute(UPDATE_VECTOR, valid).unwrap();
    assert_eq!(source(&s, duplicate_id), changed);
    assert!(
        Arc::ptr_eq(
            &smart(&s.active().unwrap().doc, id).unwrap().vector_preview.as_ref().unwrap().svg,
            &smart(&s.active().unwrap().doc, duplicate_id).unwrap().vector_preview.as_ref().unwrap().svg
        ),
        "shared instances must share the preview allocation"
    );
    assert_eq!(smart(&s.active().unwrap().doc, duplicate_id).unwrap().transform, Affine::translate(2.0, 3.0));
}

#[test]
fn native_save_keeps_fractional_psd_frame_and_rejects_a_resized_preview() {
    let (mut s, id, original) = native_session("ai", 8);
    let spec = photocraft_io::smart_map::PlacedSpec {
        idnt: "authored",
        placed: "authored",
        transform: Affine::IDENTITY,
        perspective: None,
        size: (8.3, 6.2),
        dpi: 72.0,
        warp: None,
        filter_fx: None,
    };
    let sold = photocraft_io::smart_map::sold_bytes(None, &spec, &mut vec![]);
    let state = s.active_mut().unwrap();
    let LayerContent::Smart(sm) = &mut Arc::make_mut(&mut state.doc).layer_mut(id).unwrap().content else { panic!("smart") };
    sm.psd_raw = Some(Arc::new(sold));
    let changed = [original.as_slice(), b" changed"].concat();
    let mut params = native_request(&mut s, id, "ai", &changed);
    params["previewSvg"] = json!(BLUE.replace("width=\"8\" height=\"6\"", "width=\"8.3\" height=\"6.2\""));
    s.execute(UPDATE_VECTOR, params).unwrap();
    let before = s.active().unwrap().doc.clone();
    let mut params = native_request(&mut s, id, "ai", &[changed.as_slice(), b" next"].concat());
    params["previewSvg"] = json!(BLUE.replace("width=\"8\" height=\"6\"", "width=\"10\" height=\"6\""));
    assert!(s.execute(UPDATE_VECTOR, params).is_err());
    assert_eq!(s.active().unwrap().doc, before);
}

#[test]
fn shared_native_instances_check_one_preview_identity_once_and_keep_per_instance_frames() {
    let (mut s, id, original) = native_session("ai", 8);
    let changed = [original.as_slice(), b" changed"].concat();
    let params = native_request(&mut s, id, "ai", &changed);
    s.execute(UPDATE_VECTOR, params).unwrap();
    // 40 shared instances borrow one source and preview allocation; one more has an equal
    // preview in its own allocation, so it is a separate identity and checked separately.
    let template = s.active().unwrap().doc.layer(id).unwrap().clone();
    let doc = Arc::make_mut(&mut s.active_mut().unwrap().doc);
    for _ in 0..40 {
        doc.insert_above(Some(id), template.duplicate());
    }
    let mut copied = template.duplicate();
    let LayerContent::Smart(sm) = &mut copied.content else { panic!("smart") };
    let preview = sm.vector_preview.as_mut().unwrap();
    preview.svg = Arc::new(preview.svg.as_ref().clone());
    doc.insert_above(Some(id), copied);
    let next = [changed.as_slice(), b" next"].concat();
    let params = native_request(&mut s, id, "ai", &next);
    let before = PREVIEW_FRAME_CHECKS.with(std::cell::Cell::get);
    s.execute(UPDATE_VECTOR, params).unwrap();
    assert_eq!(PREVIEW_FRAME_CHECKS.with(std::cell::Cell::get) - before, 2, "one check per source and preview identity");
    // An instance without a preview still validates its own PSD frame.
    let spec = photocraft_io::smart_map::PlacedSpec {
        idnt: "authored",
        placed: "authored",
        transform: Affine::IDENTITY,
        perspective: None,
        size: (10.0, 6.0),
        dpi: 72.0,
        warp: None,
        filter_fx: None,
    };
    let sold = photocraft_io::smart_map::sold_bytes(None, &spec, &mut vec![]);
    let mut framed = s.active().unwrap().doc.layer(id).unwrap().duplicate();
    let LayerContent::Smart(sm) = &mut framed.content else { panic!("smart") };
    sm.vector_preview = None;
    sm.psd_raw = Some(Arc::new(sold));
    Arc::make_mut(&mut s.active_mut().unwrap().doc).insert_above(Some(id), framed);
    let params = native_request(&mut s, id, "ai", &[next.as_slice(), b" again"].concat());
    let before = s.active().unwrap().doc.clone();
    let error = s.execute(UPDATE_VECTOR, params).unwrap_err().to_string();
    assert!(error.contains("source-pixel frame"), "a differing PSD frame on one shared instance must reject: {error}");
    assert_eq!(s.active().unwrap().doc, before);
}

/// Distinct old previews share a byte budget even when destination surfaces are tiny.
#[test]
fn distinct_native_preview_metadata_exhausts_budget_without_editing_any_instance() {
    let (mut s, id, original) = native_session("ai", 8);
    let first_save = [original.as_slice(), b" first"].concat();
    let params = native_request(&mut s, id, "ai", &first_save);
    s.execute(UPDATE_VECTOR, params).unwrap();
    let template = s.active().unwrap().doc.layer(id).unwrap().clone();
    let doc = Arc::make_mut(&mut s.active_mut().unwrap().doc);
    let mut preview_length = 0;
    for n in 0..3 {
        let mut instance = template.duplicate();
        let LayerContent::Smart(sm) = &mut instance.content else { panic!("smart") };
        let preview = sm.vector_preview.as_mut().unwrap();
        let svg = RED.replace("</svg>", &format!("<!--{n}--></svg>"));
        preview_length = svg.len();
        preview.svg = Arc::new(svg.into_bytes());
        if n == 0 {
            let layer = doc.layer_mut(id).unwrap();
            layer.content = instance.content;
        } else {
            doc.insert_above(Some(id), instance);
        }
    }
    let next = [first_save.as_slice(), b" next"].concat();
    let params = native_request(&mut s, id, "ai", &next);
    let before = s.active().unwrap().doc.clone();
    let revision = s.active().unwrap().revision;
    let checks = PREVIEW_FRAME_CHECKS.with(std::cell::Cell::get);
    let error = update_contents_with_frame_budget(&mut s, &params, true, preview_length * 2).unwrap_err().to_string();
    assert!(error.contains("validation budget"), "{error}");
    assert_eq!(PREVIEW_FRAME_CHECKS.with(std::cell::Cell::get) - checks, 2, "exhausted metadata is never parsed");
    assert_eq!(s.active().unwrap().doc, before);
    assert_eq!(s.active().unwrap().revision, revision);
    assert_eq!(s.execute(UPDATE_VECTOR, params).unwrap()["updated"], true);
    assert!(s.undo());
    assert_eq!(s.active().unwrap().doc, before);
}

/// A large opaque native payload is shared data, not work repeated for every display cache.
#[test]
fn native_save_refreshes_shared_caches_without_rehashing_the_native_payload_per_instance() {
    let (mut s, id, original) = native_session("eps", 8);
    let template = s.active().unwrap().doc.layer(id).unwrap().clone();
    let doc = Arc::make_mut(&mut s.active_mut().unwrap().doc);
    for _ in 1..100 {
        doc.insert_above(Some(id), template.duplicate());
    }
    let before = s.active().unwrap().doc.clone();
    let mut changed = vec![b' '; MAX_VECTOR_SOURCE_BYTES];
    changed[..original.len()].copy_from_slice(&original);
    let params = native_request(&mut s, id, "eps", &changed);
    let hashed = crate::smart_cmds::DISPLAY_NATIVE_HASH_BYTES.with(std::cell::Cell::get);
    assert_eq!(s.execute(UPDATE_VECTOR, params).unwrap()["updated"], true);
    assert_eq!(crate::smart_cmds::DISPLAY_NATIVE_HASH_BYTES.with(std::cell::Cell::get) - hashed, 0, "cache refresh must not hash 16 MiB for every copy");
    let after = s.active().unwrap().doc.clone();
    let target = smart(&after, id).unwrap();
    let SmartSource::Embedded { file_name, bytes } = &target.source else { panic!("native source lost") };
    assert_eq!(file_name, "Logo.eps");
    assert_eq!(bytes.as_slice(), changed.as_slice());
    assert_eq!(target.vector_preview.as_ref().unwrap().source_hash, *blake3::hash(&changed).as_bytes());
    for (_, _, layer) in after.walk() {
        let LayerContent::Smart(sm) = &layer.content else { continue };
        let SmartSource::Embedded { bytes: shared, .. } = &sm.source else { panic!("native source lost") };
        assert!(Arc::ptr_eq(shared, bytes));
        assert_eq!(sm.contents_id, target.contents_id);
        assert_eq!(sm.cache.as_ref().unwrap().pixel(15, 11), vec![0.0, 0.0, 1.0, 1.0]);
        let old = smart(&before, layer.id).unwrap();
        assert_eq!(sm.transform, old.transform);
        assert_eq!(sm.smart_filters, old.smart_filters);
        assert_eq!(sm.filter_mask, old.filter_mask);
    }
    assert!(s.undo());
    assert_eq!(s.active().unwrap().doc, before);
    assert!(s.redo());
    assert_eq!(s.active().unwrap().doc, after);
}

/// Explicit refreshes and warp frames of one shared native source verify its preview hash once,
/// not once per call; a replaced source is verified again and a forged preview still fails.
#[test]
fn native_preview_digest_is_reused_per_source_allocation_and_rechecked_after_replacement() {
    let (mut s, id, original) = native_session("eps", 8);
    let mut changed = vec![b' '; 1024 * 1024];
    changed[..original.len()].copy_from_slice(&original);
    let params = native_request(&mut s, id, "eps", &changed);
    s.execute(UPDATE_VECTOR, params).unwrap();
    let doc = s.active().unwrap().doc.clone();
    let mut sm = smart(&doc, id).unwrap().clone();
    // A fresh allocation of the same bytes: nothing about it can be remembered yet.
    sm.source = SmartSource::Embedded { file_name: "Logo.eps".into(), bytes: Arc::new(changed.clone()) };
    let hashed = || crate::smart_cmds::DISPLAY_NATIVE_HASH_BYTES.with(std::cell::Cell::get);
    let start = hashed();
    let frame = crate::smart_cmds::source_box(&doc, &sm).unwrap();
    for _ in 0..3 {
        assert_eq!(crate::smart_cmds::source_box(&doc, &sm), Some(frame));
        assert_eq!(crate::smart_cmds::render(&doc, &sm).unwrap().unwrap().pixel(15, 11), vec![0.0, 0.0, 1.0, 1.0], "the verified preview is drawn");
    }
    assert_eq!(hashed() - start, changed.len(), "one verification for one source allocation");
    // Different bytes in a new allocation never inherit the verified digest.
    let mut other = changed.clone();
    other[original.len()] = b'%';
    sm.source = SmartSource::Embedded { file_name: "Logo.eps".into(), bytes: Arc::new(other) };
    let before = hashed();
    assert!(crate::smart_cmds::render(&doc, &sm).is_err(), "a stale preview must not draw a different source");
    assert_eq!(hashed() - before, changed.len());
}

#[test]
fn native_source_replacement_never_uses_a_forged_or_stale_preview() {
    let (mut s, id, original) = native_session("ai", 8);
    let changed = [original.as_slice(), b" changed"].concat();
    let params = native_request(&mut s, id, "ai", &changed);
    s.execute(UPDATE_VECTOR, params).unwrap();
    let mut sm = smart(&s.active().unwrap().doc, id).unwrap().clone();
    sm.vector_preview.as_mut().unwrap().svg = Arc::new(b"not SVG".to_vec());
    assert!(crate::smart_cmds::render(&s.active().unwrap().doc, &sm).is_err());
    sm.vector_preview.as_mut().unwrap().svg = Arc::new(BLUE.as_bytes().to_vec());
    sm.source = SmartSource::Embedded { file_name: "Changed.ai".into(), bytes: Arc::new(b"%PDF-1.7\nnot the same source".to_vec()) };
    assert!(crate::smart_cmds::render(&s.active().unwrap().doc, &sm).is_err());
}

#[test]
fn native_update_authorizes_filters_on_every_shared_instance_before_editing() {
    let (mut s, id, original) = native_session("ai", 8);
    let mut duplicate = s.active().unwrap().doc.layer(id).unwrap().duplicate();
    let LayerContent::Smart(sm) = &mut duplicate.content else { panic!("smart") };
    sm.smart_filters.push(SmartFilter {
        command: "image.adjustments.colorLookup".into(),
        params: json!({"file":"/not-authorized/private.cube"}),
        blend: photocraft_color::BlendMode::Normal,
        opacity: 1.0,
        visible: true,
    });
    Arc::make_mut(&mut s.active_mut().unwrap().doc).insert_above(Some(id), duplicate);
    let params = native_request(&mut s, id, "ai", &[original.as_slice(), b" changed"].concat());
    s.authorize =
        Some(|command, _| if command == "image.adjustments.colorLookup" { Err(EngineError::Other("denied stored native filter".into())) } else { Ok(()) });
    let before = s.active().unwrap().doc.clone();
    assert!(s.execute(UPDATE_VECTOR, params).unwrap_err().to_string().contains("denied stored native filter"));
    assert_eq!(s.active().unwrap().doc, before);
}
