//! `files::save_bytes` is the capability-scoped encoder: a linked path inside the document must
//! not be read through ambient filesystem authority and embedded in the returned bytes.

use photocraft_automation::files;
use photocraft_doc::{Document, Layer, LayerContent, SmartContentsId, SmartObject, SmartSource};

#[test]
fn a_linked_smart_object_is_not_read_into_the_bytes() {
    let mut doc = Document::new("linked", photocraft_doc::Size::new(8, 6), photocraft_doc::ColorMode::Rgb, photocraft_doc::SampleType::U8);
    let linked = std::env::current_exe().unwrap();
    let smart = SmartObject {
        contents_id: SmartContentsId::fresh(),
        source: SmartSource::Linked { path: linked.to_string_lossy().into_owned() },
        transform: photocraft_geom::Affine::IDENTITY,
        smart_filters: vec![],
        cache: Some(photocraft_raster::Surface::new(doc.pixel_format())),
        psd_raw: None,
        filters_enabled: true,
        filter_mask: None,
        warp: None,
        stack_mode: None,
        perspective: None,
        vector_preview: None,
    };
    doc.layers.push(Layer::new("Linked", LayerContent::Smart(smart)));
    let (bytes, warnings) = files::save_bytes(&doc, "out.psd", None, &Default::default()).unwrap();
    assert!(warnings.iter().any(|w| w.contains("not read for this export")), "{warnings:?}");
    assert!(bytes.len() < 1 << 20, "the test executable was not embedded");
}
