//! Duplicated smart objects share their editable contents, but not instance settings (#2163).

use photocraft_doc::{Layer, LayerContent, LayerId, SmartObject};
use photocraft_engine::Session;
use photocraft_engine::smart_cmds::{decode_source, source_bytes};
use photocraft_geom::Rect;
use serde_json::{Value, json};

fn session(depth: u64) -> (Session, u64, u64) {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 8, "height": 6, "depth": depth})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    paint(&mut s, [0.0, 0.0, 1.0, 1.0]);
    let original = s.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap()["layer"].as_u64().unwrap();
    let duplicate = s.execute("layer.duplicate", json!({"layer": original})).unwrap()["layer"].as_u64().unwrap();
    (s, original, duplicate)
}

fn paint(s: &mut Session, rgba: [f32; 4]) {
    s.edit("paint", |doc, active| {
        doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap().fill_rect(Rect::new(0, 0, 8, 6), &rgba);
        Ok(())
    })
    .unwrap();
}

fn smart(s: &Session, id: u64) -> &SmartObject {
    let LayerContent::Smart(sm) = &s.active().unwrap().doc.layer(LayerId(id)).unwrap().content else { panic!("smart object") };
    sm
}

fn contents_pixel(s: &Session, id: u64) -> [f32; 4] {
    let (name, bytes) = source_bytes(&s.active().unwrap().doc.metadata, &smart(s, id).source).unwrap();
    photocraft_compose::flatten(&decode_source(&name, &bytes).unwrap()).px[0]
}

#[test]
fn editing_original_updates_duplicate_contents() {
    for depth in [8, 16, 32] {
        let (mut s, original, duplicate) = session(depth);
        // The sibling may be nested and have its own placement and smart filters.
        s.execute("filter.blur.gaussianBlur", json!({"radius": 1})).unwrap();
        s.execute("edit.transform", json!({"layer": duplicate, "matrix": [1, 0, 0, 1, 2, 1]})).unwrap();
        let instance = smart(&s, duplicate).clone();
        s.edit("group", |doc, _| {
            let copy = doc.remove(LayerId(duplicate)).unwrap();
            doc.layers.push(Layer::group("Copies", vec![copy]));
            Ok(())
        })
        .unwrap();
        let before = s.active().unwrap().doc.clone();
        let child = s.execute("layer.smartObjects.editContents", json!({"layer": original})).unwrap()["document"].as_u64().unwrap();
        paint(&mut s, [1.0, 0.0, 0.0, 1.0]);
        s.execute("layer.smartObjects.saveContents", json!({})).unwrap();
        assert_eq!(s.active_index(), Some(child as usize));
        assert!(!s.active().unwrap().is_dirty());
        s.set_active(0);
        for id in [original, duplicate] {
            assert_eq!(contents_pixel(&s, id), [1.0, 0.0, 0.0, 1.0]);
            let cache = smart(&s, id).cache.as_ref().unwrap();
            let alpha = if id == duplicate { instance.cache.as_ref().unwrap().rgba(4, 3)[3] } else { 1.0 };
            assert_eq!(cache.rgba(4, 3), [1.0, 0.0, 0.0, alpha], "re-rendered contents retain the instance's blurred edge");
        }
        assert_eq!(smart(&s, duplicate).transform, instance.transform);
        assert_eq!(smart(&s, duplicate).smart_filters, instance.smart_filters);
        assert!(smart(&s, original).smart_filters.is_empty());
        let after = s.active().unwrap().doc.clone();
        assert!(s.undo());
        assert_eq!(s.active().unwrap().doc, before, "all instances revert in one undo step");
        assert!(s.redo());
        assert_eq!(s.active().unwrap().doc, after);
        // Entering from the duplicate reuses the same editor, and close commits back to both.
        let open = s.documents().len();
        assert_eq!(s.execute("layer.smartObjects.editContents", json!({"layer": duplicate})).unwrap()["document"], child);
        assert_eq!(s.documents().len(), open);
        paint(&mut s, [0.0, 1.0, 0.0, 1.0]);
        s.execute("file.close", json!({})).unwrap();
        assert_eq!(contents_pixel(&s, original), [0.0, 1.0, 0.0, 1.0]);
        assert_eq!(contents_pixel(&s, duplicate), [0.0, 1.0, 0.0, 1.0]);
    }
}

#[test]
fn surviving_duplicate_receives_edits_after_originating_layer_is_deleted() {
    let (mut s, original, duplicate) = session(16);
    let child = s.execute("layer.smartObjects.editContents", json!({"layer": original})).unwrap()["document"].as_u64().unwrap() as usize;
    s.set_active(0);
    s.execute("layer.delete", json!({"layer": original})).unwrap();
    s.set_active(child);
    paint(&mut s, [1.0, 0.0, 0.0, 1.0]);
    s.execute("layer.smartObjects.saveContents", json!({})).unwrap();
    s.set_active(0);
    assert_eq!(contents_pixel(&s, duplicate), [1.0, 0.0, 0.0, 1.0]);
}

#[test]
fn shared_and_independent_contents_survive_native_psd_and_psb() {
    for format in ["pcraft", "psd", "psb"] {
        let (mut s, original, duplicate) = session(16);
        let independent = s.execute("layer.smartObjects.newSmartObjectViaCopy", json!({"layer": original})).unwrap()["layer"].as_u64().unwrap();
        s.edit("name", |doc, _| {
            for (id, name) in [(original, "original"), (duplicate, "duplicate"), (independent, "independent")] {
                doc.layer_mut(LayerId(id)).unwrap().name = name.into();
            }
            Ok(())
        })
        .unwrap();
        // Initially byte-identical independent copies must not be merged by blob deduplication.
        let doc = &s.active().unwrap().doc;
        let back = if format == "pcraft" {
            let bytes = photocraft_format::save_to_bytes(doc, &Default::default()).unwrap();
            photocraft_format::load_from_bytes(&bytes).unwrap()
        } else {
            let out = photocraft_io::export(doc, format, &Default::default()).unwrap();
            assert!(out.warnings.is_empty(), "{format}: {:?}", out.warnings);
            photocraft_io::import(&format!("duplicates.{format}"), &out.bytes).unwrap().document
        };
        let named = |name: &str| back.walk().into_iter().find(|(_, _, l)| l.name == name).unwrap().2.id.0;
        let (original, duplicate, independent) = (named("original"), named("duplicate"), named("independent"));
        let mut s = Session::new();
        s.add_document(back, None);
        s.execute("layer.smartObjects.editContents", json!({"layer": duplicate})).unwrap();
        paint(&mut s, [1.0, 0.0, 0.0, 1.0]);
        s.execute("layer.smartObjects.saveContents", json!({})).unwrap();
        s.set_active(0);
        assert_eq!(contents_pixel(&s, original), [1.0, 0.0, 0.0, 1.0], "{format}");
        assert_eq!(contents_pixel(&s, duplicate), [1.0, 0.0, 0.0, 1.0], "{format}");
        assert_eq!(contents_pixel(&s, independent), [0.0, 0.0, 1.0, 1.0], "{format}");
        s.execute("layer.smartObjects.editContents", json!({"layer": independent})).unwrap();
        assert_eq!(s.documents().len(), 3, "independent contents need their own editor");
        paint(&mut s, [0.0, 1.0, 0.0, 1.0]);
        s.execute("layer.smartObjects.saveContents", json!({})).unwrap();
        s.set_active(0);
        assert_eq!(contents_pixel(&s, original), [1.0, 0.0, 0.0, 1.0]);
        assert_eq!(contents_pixel(&s, independent), [0.0, 1.0, 0.0, 1.0]);
    }
}

#[test]
fn copying_a_batch_between_documents_preserves_only_its_internal_sharing() {
    let (mut s, original, duplicate) = session(16);
    let independent = s.execute("layer.smartObjects.newSmartObjectViaCopy", json!({"layer": original})).unwrap()["layer"].as_u64().unwrap();
    let source_names = [original, duplicate, independent].map(|id| s.active().unwrap().doc.layer(LayerId(id)).unwrap().name.clone());
    // Another opening of the same document has the same persisted contents IDs.
    let target = (*s.active().unwrap().doc).clone();
    s.add_document(target, None);
    s.set_active(0);
    let result = s.execute("layer.copyToDocument", json!({"document": 1, "layers": [original, duplicate, independent]})).unwrap();
    // The command returns stacking order, which differs from the requested order after via-copy.
    let copied = source_names.map(|name| {
        result["layers"]
            .as_array()
            .unwrap()
            .iter()
            .map(|id| id.as_u64().unwrap())
            .find(|id| s.active().unwrap().doc.layer(LayerId(*id)).unwrap().name == name)
            .unwrap()
    });
    assert_eq!(smart(&s, copied[0]).contents_id, smart(&s, copied[1]).contents_id);
    assert_ne!(smart(&s, copied[0]).contents_id, smart(&s, copied[2]).contents_id);
    assert_ne!(smart(&s, copied[0]).contents_id, smart(&s, original).contents_id);
    s.execute("layer.smartObjects.editContents", json!({"layer": copied[0]})).unwrap();
    paint(&mut s, [1.0, 0.0, 0.0, 1.0]);
    s.execute("layer.smartObjects.saveContents", json!({})).unwrap();
    s.set_active(1);
    for id in &copied[..2] {
        assert_eq!(contents_pixel(&s, *id), [1.0, 0.0, 0.0, 1.0]);
    }
    for id in [original, duplicate, independent, copied[2]] {
        assert_eq!(contents_pixel(&s, id), [0.0, 0.0, 1.0, 1.0]);
    }
    s.set_active(0);
    assert_eq!(contents_pixel(&s, original), [0.0, 0.0, 1.0, 1.0]);
}

#[test]
fn replacing_shared_contents_does_not_reuse_or_commit_a_stale_editor() {
    let (mut s, original, duplicate) = session(16);
    let child = s.execute("layer.smartObjects.editContents", json!({"layer": original})).unwrap()["document"].as_u64().unwrap() as usize;
    paint(&mut s, [0.0, 1.0, 0.0, 1.0]);
    let path = std::env::temp_dir().join(format!("photocraft-shared-contents-{}-{original}.pcraft", std::process::id()));
    std::fs::write(&path, photocraft_format::save_to_bytes(&s.active().unwrap().doc, &Default::default()).unwrap()).unwrap();
    paint(&mut s, [1.0, 0.0, 0.0, 1.0]);
    s.set_active(0);
    let replaced = s.execute("layer.smartObjects.replaceContents", json!({"layer": duplicate, "path": path}));
    std::fs::remove_file(path).unwrap();
    replaced.unwrap();
    for id in [original, duplicate] {
        assert_eq!(contents_pixel(&s, id), [0.0, 1.0, 0.0, 1.0]);
    }
    s.set_active(child);
    assert!(!s.is_enabled("layer.smartObjects.saveContents"));
    s.execute("file.close", json!({})).unwrap();
    assert_eq!(contents_pixel(&s, original), [0.0, 1.0, 0.0, 1.0]);
    s.execute("layer.smartObjects.editContents", json!({"layer": original})).unwrap();
    assert_eq!(photocraft_compose::flatten(&s.active().unwrap().doc).px[0], [0.0, 1.0, 0.0, 1.0]);
}

/// A `.pcraft` file whose contents are one solid colour, for Replace Contents.
fn contents_file(rgba: [f32; 4], tag: &str) -> std::path::PathBuf {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 8, "height": 6})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    paint(&mut s, rgba);
    let path = std::env::temp_dir().join(format!("photocraft-{tag}-{}.pcraft", std::process::id()));
    std::fs::write(&path, photocraft_format::save_to_bytes(&s.active().unwrap().doc, &Default::default()).unwrap()).unwrap();
    path
}

/// The node named `name` in `document.inspect`'s layer tree, searched through groups.
fn inspected<'a>(layers: &'a Value, name: &str) -> Option<&'a Value> {
    layers.as_array()?.iter().find_map(|l| if l["name"] == name { Some(l) } else { inspected(&l["children"], name) })
}

/// Placed layers sharing a source (a PSD's copies of one smart object) share their contents, so
/// one Replace Contents updates every copy. Nothing showed it: `linkGroup` is Link Layers, which
/// stays unset (#4075). Inspect gives each smart object its contents identity and instance count,
/// and Replace Contents lists every layer it changed.
#[test]
fn inspect_and_replace_contents_report_shared_instances() {
    let (mut s, original, duplicate) = session(8);
    let independent = s.execute("layer.smartObjects.newSmartObjectViaCopy", json!({"layer": original})).unwrap()["layer"].as_u64().unwrap();
    s.edit("name and group", |doc, _| {
        for (id, name) in [(original, "original"), (duplicate, "duplicate"), (independent, "independent")] {
            doc.layer_mut(LayerId(id)).unwrap().name = name.into();
        }
        // An instance inside a group counts too.
        let copy = doc.remove(LayerId(duplicate)).unwrap();
        doc.layers.push(Layer::group("Copies", vec![copy]));
        Ok(())
    })
    .unwrap();
    let out = photocraft_io::export(&s.active().unwrap().doc, "psd", &Default::default()).unwrap();
    let mut s = Session::new();
    s.add_document(photocraft_io::import("copies.psd", &out.bytes).unwrap().document, None);

    let tree = s.execute("document.inspect", json!({})).unwrap();
    let [o, d, i] = ["original", "duplicate", "independent"].map(|name| inspected(&tree["layers"], name).unwrap().clone());
    assert_eq!(o["smartContents"]["id"], d["smartContents"]["id"]);
    assert_ne!(o["smartContents"]["id"], i["smartContents"]["id"]);
    assert_eq!([&o, &d, &i].map(|l| l["smartContents"]["instances"].as_u64()), [Some(2), Some(2), Some(1)]);
    assert!([&o, &d, &i].iter().all(|l| l["linkGroup"].is_null()), "shared contents don't link the layers");

    let [o, d, i] = [&o, &d, &i].map(|l| l["id"].as_u64().unwrap());
    let path = contents_file([0.0, 1.0, 0.0, 1.0], "replace-instances");
    let replaced = s.execute("layer.smartObjects.replaceContents", json!({"layer": d, "path": path}));
    std::fs::remove_file(path).unwrap();
    let mut changed: Vec<u64> = replaced.unwrap()["instances"].as_array().unwrap().iter().map(|v| v.as_u64().unwrap()).collect();
    changed.sort_unstable();
    assert_eq!(changed, if o < d { [o, d] } else { [d, o] });
    assert_eq!([o, d, i].map(|id| contents_pixel(&s, id)), [[0.0, 1.0, 0.0, 1.0], [0.0, 1.0, 0.0, 1.0], [0.0, 0.0, 1.0, 1.0]]);
    for (id, expected) in [(o, [0.0, 1.0, 0.0, 1.0]), (d, [0.0, 1.0, 0.0, 1.0]), (i, [0.0, 0.0, 1.0, 1.0])] {
        assert_eq!(smart(&s, id).cache.as_ref().unwrap().rgba(4, 3), expected, "instance {id} must display its current contents");
    }
}

/// The same with the placed layer Photoshop wrote, twice: Photoshop's copies of a smart object
/// each carry a `SoLd` naming the one embedded file (`Idnt`), so they import as one contents.
#[cfg(feature = "corpus")]
#[test]
fn photoshop_placed_copies_import_as_shared_instances() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus/photoshop/smart-filters/sf-none-plain.psd");
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e} (run `cargo xtask corpus --all`)", path.display()));
    let mut file = photocraft_psd::PsdFile::from_bytes(&bytes).unwrap();
    let placed = file.layers().iter().find(|r| r.block(b"SoLd").is_some()).expect("a placed layer").clone();
    file.layers_mut().push(placed);
    let mut s = Session::new();
    s.add_document(photocraft_io::psd_to_document(&file).0, None);

    let tree = s.execute("document.inspect", json!({})).unwrap();
    let copies: Vec<&Value> = tree["layers"].as_array().unwrap().iter().filter(|l| l["kind"] == "Smart Object").collect();
    assert_eq!(copies.len(), 2);
    assert_eq!(copies[0]["smartContents"], copies[1]["smartContents"]);
    assert_eq!(copies[0]["smartContents"]["instances"], 2);
    for copy in &copies {
        assert_eq!(copy["smartSource"]["kind"], "embedded");
        assert!(copy["smartSource"]["uuid"].as_str().is_some());
    }
    let mut ids: Vec<u64> = copies.iter().map(|l| l["id"].as_u64().unwrap()).collect();
    let path = contents_file([0.0, 1.0, 0.0, 1.0], "photoshop-copies");
    let replaced = s.execute("layer.smartObjects.replaceContents", json!({"layer": ids[0], "path": path}));
    std::fs::remove_file(path).unwrap();
    let mut changed: Vec<u64> = replaced.unwrap()["instances"].as_array().unwrap().iter().map(|v| v.as_u64().unwrap()).collect();
    changed.sort_unstable();
    ids.sort_unstable();
    assert_eq!(changed, ids);
    for id in ids {
        assert_eq!(contents_pixel(&s, id), [0.0, 1.0, 0.0, 1.0]);
        let cache = smart(&s, id).cache.as_ref().unwrap();
        let bounds = cache.content_bounds();
        assert!(!bounds.is_empty(), "instance {id} must have rendered pixels");
        assert_eq!(
            cache.rgba(bounds.x0 + bounds.width() as i32 / 2, bounds.y0 + bounds.height() as i32 / 2),
            [0.0, 1.0, 0.0, 1.0],
            "instance {id} must refresh its imported cache"
        );
    }
}
