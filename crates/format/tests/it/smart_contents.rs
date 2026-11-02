//! Native bundles preserve smart-object sharing without inferring it from equal source bytes.

use std::sync::Arc;

use photocraft_color::{ColorMode, SampleType};
use photocraft_doc::{Document, Layer, LayerContent, LayerId, Size, SmartContentsId, SmartObject, SmartSource};
use photocraft_format::{LoadOptions, SaveOptions, load_from_bytes, load_from_bytes_with, save_to_bytes, zip};
use photocraft_geom::Affine;

fn embedded(bytes: &Arc<Vec<u8>>) -> Layer {
    Layer::new(
        "embedded",
        LayerContent::Smart(SmartObject::new(SmartSource::Embedded { file_name: "source.png".into(), bytes: bytes.clone() }, Affine::IDENTITY, None)),
    )
}

fn linked(path: &str) -> Layer {
    Layer::new("linked", LayerContent::Smart(SmartObject::new(SmartSource::Linked { path: path.into() }, Affine::IDENTITY, None)))
}

fn sample() -> Document {
    let mut doc = Document::new("contents identities", Size::new(1, 1), ColorMode::Rgb, SampleType::U8);
    let bytes = Arc::new(vec![1, 2, 3]);
    let first = embedded(&bytes);
    let duplicate = first.duplicate();
    doc.layers.push(Layer::group("instances", vec![first, duplicate]));
    doc.layers.push(embedded(&bytes));
    doc
}

fn contents_ids(doc: &Document) -> Vec<SmartContentsId> {
    doc.walk()
        .iter()
        .filter_map(|(_, _, layer)| match &layer.content {
            LayerContent::Smart(s) => Some(s.contents_id),
            _ => None,
        })
        .collect()
}

fn rewrite_manifest(bytes: &[u8], edit: impl FnOnce(&mut serde_json::Value)) -> Vec<u8> {
    let r = zip::ZipReader::new(bytes).unwrap();
    let mut manifest: serde_json::Value = serde_json::from_slice(&r.read_by_name("manifest.json", usize::MAX).unwrap()).unwrap();
    edit(&mut manifest);
    let mut z = zip::ZipWriter::new();
    z.add("manifest.json", &serde_json::to_vec(&manifest).unwrap()).unwrap();
    for entry in r.entries.iter().filter(|e| e.name != "manifest.json") {
        z.add(&entry.name, &r.read(entry, usize::MAX).unwrap()).unwrap();
    }
    z.finish().unwrap()
}

fn remove_contents_ids(layers: &mut serde_json::Value) {
    for layer in layers.as_array_mut().unwrap() {
        let content = layer["content"].as_object_mut().unwrap();
        content.remove("contents_id");
        if let Some(children) = content.get_mut("children") {
            remove_contents_ids(children);
        }
    }
}

#[test]
fn smart_contents_roundtrip_preserves_shared_instances_and_independent_equal_blobs() {
    let doc = sample();
    let ids = contents_ids(&doc);
    assert_eq!(ids[0], ids[1], "ordinary layer duplicates share their contents");
    assert_ne!(ids[0], ids[2], "equal bytes, even the same Arc, do not imply shared contents");
    let bytes = save_to_bytes(&doc, &SaveOptions::default()).unwrap();
    assert_eq!(load_from_bytes(&bytes).unwrap(), doc);
}

#[test]
fn smart_contents_remapping_preserves_groups() {
    let doc = sample();
    let original = contents_ids(&doc);
    let bytes = save_to_bytes(&doc, &SaveOptions::default()).unwrap();
    let back = load_from_bytes_with(&bytes, &LoadOptions { preserve_ids: false, ..Default::default() }).unwrap();
    let ids = contents_ids(&back);
    assert_eq!(ids[0], ids[1]);
    assert_ne!(ids[0], ids[2]);
    assert!(ids.iter().all(|id| !original.contains(id)));
    assert_ne!(back.id, doc.id);
    assert_ne!(back.layers[0].id, doc.layers[0].id);
}

#[test]
fn legacy_smart_contents_are_independent_except_for_nonempty_linked_paths() {
    let mut doc = sample();
    doc.layers.extend([linked("/tmp/shared.psd"), linked("/tmp/shared.psd"), linked("/tmp/other.psd"), linked(""), linked("")]);
    let bytes = save_to_bytes(&doc, &SaveOptions::default()).unwrap();
    let legacy = rewrite_manifest(&bytes, |m| remove_contents_ids(&mut m["document"]["layers"]));
    let back = load_from_bytes(&legacy).unwrap();
    let ids = contents_ids(&back);
    assert_ne!(ids[0], ids[1], "legacy embedded duplicates have no recorded shared identity");
    assert_ne!(ids[0], ids[2]);
    assert_ne!(ids[1], ids[2]);
    assert_eq!(ids[3], ids[4]);
    assert_ne!(ids[3], ids[5]);
    assert_ne!(ids[6], ids[7], "an empty path identifies no shared source");
    // Allocated legacy identities become explicit on the next save.
    assert_eq!(load_from_bytes(&save_to_bytes(&back, &SaveOptions::default()).unwrap()).unwrap(), back);
}

#[test]
fn stored_smart_contents_ids_are_reserved_before_allocating_legacy_ids() {
    let mut doc = sample();
    doc.layers.push(linked("/tmp/source.psd"));
    let bytes = save_to_bytes(&doc, &SaveOptions::default()).unwrap();
    let stored = LayerId::fresh().0 + 5000;
    let mixed = rewrite_manifest(&bytes, |m| {
        remove_contents_ids(&mut m["document"]["layers"]);
        // This later, nested entry must be reserved before the first missing identity is minted.
        m["document"]["layers"][0]["content"]["children"][1]["content"]["contents_id"] = stored.into();
    });
    let back = load_from_bytes(&mixed).unwrap();
    let ids = contents_ids(&back);
    assert_eq!(ids[1], SmartContentsId(stored));
    assert!(ids[0].0 > stored);
    assert!(ids[2].0 > stored);
    assert!(ids[3].0 > stored);
    assert!(LayerId::fresh().0 > stored);
}

#[test]
fn extreme_smart_contents_ids_remap_without_poisoning_allocator() {
    let doc = sample();
    let bytes = save_to_bytes(&doc, &SaveOptions::default()).unwrap();
    let damaged = rewrite_manifest(&bytes, |m| {
        for child in m["document"]["layers"][0]["content"]["children"].as_array_mut().unwrap() {
            child["content"]["contents_id"] = u64::MAX.into();
        }
    });
    let back = load_from_bytes(&damaged).unwrap();
    let ids = contents_ids(&back);
    assert_eq!(ids[0], ids[1]);
    assert_ne!(ids[0], ids[2]);
    assert_ne!(back.id, doc.id);
    assert_ne!(back.layers[0].id, doc.layers[0].id);
    assert!(ids.iter().all(|id| id.0 < u64::MAX / 2));
    assert!(SmartContentsId::fresh().0 < u64::MAX / 2);
    assert!(LayerId::fresh().0 < u64::MAX / 2);
}

#[test]
fn native_vector_display_preview_roundtrips_separately_from_original_payload() {
    let source = Arc::new(b"%PDF-1.7\noriginal editable source".to_vec());
    let svg = Arc::new(br#"<svg xmlns="http://www.w3.org/2000/svg" width="8" height="6"/>"#.to_vec());
    let mut smart = SmartObject::new(SmartSource::Embedded { file_name: "Artwork.ai".into(), bytes: source.clone() }, Affine::IDENTITY, None);
    smart.vector_preview = Some(photocraft_doc::VectorPreview { source_hash: *blake3::hash(&source).as_bytes(), svg });
    let mut doc = Document::new("native vector", Size::new(8, 6), ColorMode::Rgb, SampleType::U8);
    doc.layers.push(Layer::new("vector", LayerContent::Smart(smart)));
    let bytes = save_to_bytes(&doc, &Default::default()).unwrap();
    assert_eq!(load_from_bytes(&bytes).unwrap(), doc);
    let missing_preview = rewrite_manifest(&bytes, |manifest| {
        manifest["document"]["layers"][0]["content"]["vector_preview"]["svg"] = serde_json::json!("0".repeat(64));
    });
    let recovered = load_from_bytes(&missing_preview).unwrap();
    let LayerContent::Smart(recovered_smart) = &recovered.layers[0].content else { panic!("smart object lost") };
    assert!(recovered_smart.vector_preview.is_none(), "an unavailable display preview must not hide the original vector");
    assert_eq!(recovered_smart.source, SmartSource::Embedded { file_name: "Artwork.ai".into(), bytes: source.clone() });
    let mut oversized = doc.clone();
    let LayerContent::Smart(oversized_smart) = &mut oversized.layers[0].content else { panic!("smart object lost") };
    oversized_smart.vector_preview.as_mut().unwrap().svg = Arc::new(vec![b'x'; (16 << 20) + 1]);
    let oversized = save_to_bytes(&oversized, &Default::default()).unwrap();
    let recovered = load_from_bytes(&oversized).unwrap();
    let LayerContent::Smart(recovered_smart) = &recovered.layers[0].content else { panic!("smart object lost") };
    assert!(recovered_smart.vector_preview.is_none(), "an oversized optional preview is ignored before unbounded decompression");
    assert_eq!(recovered_smart.source, SmartSource::Embedded { file_name: "Artwork.ai".into(), bytes: source.clone() });
    let legacy = rewrite_manifest(&bytes, |manifest| {
        manifest["document"]["layers"][0]["content"].as_object_mut().unwrap().remove("vector_preview");
    });
    let back = load_from_bytes(&legacy).unwrap();
    let LayerContent::Smart(smart) = &back.layers[0].content else { panic!("smart object lost") };
    assert!(smart.vector_preview.is_none());
    assert_eq!(smart.source, SmartSource::Embedded { file_name: "Artwork.ai".into(), bytes: source });
}
