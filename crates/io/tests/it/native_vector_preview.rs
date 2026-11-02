//! Native embedded files stay native while PhotoCraft retains a source-bound display preview.

use std::sync::Arc;

use photocraft_color::{ColorMode, SampleType};
use photocraft_doc::{Document, Layer, LayerContent, SmartObject, SmartSource, VectorPreview};
use photocraft_geom::{Affine, Size};
use photocraft_io::{ExportOptions, export, import, linked, smart_map, vector_preview};
use photocraft_psd::PsdFile;
use photocraft_raster::Surface;

const PDF: &[u8] = b"%PDF-1.7\nopaque native source bytes\n%%EOF\n";
const EPS: &[u8] = b"%!PS-Adobe-3.0 EPSF-3.0\n%%BoundingBox: 0 0 8 6\n1 0 0 setrgbcolor 0 0 8 6 rectfill\nshowpage\n%%EOF\n";
const SVG: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" width="8" height="6"><rect width="8" height="6" fill="red"/></svg>"#;

fn fixture(name: &str, source: &[u8]) -> Document {
    let mut doc = Document::new("native", Size::new(20, 20), ColorMode::Rgb, SampleType::U8);
    let mut cache = Surface::new(doc.pixel_format());
    cache.fill_rect(photocraft_geom::Rect::new(2, 3, 18, 15), &[1.0, 0.0, 0.0, 1.0]);
    let mut sm = SmartObject::new(
        SmartSource::Embedded { file_name: name.into(), bytes: Arc::new(source.to_vec()) },
        Affine { m: [2.0, 0.0, 0.0, 2.0, 2.0, 3.0] },
        Some(cache),
    );
    sm.vector_preview = Some(VectorPreview { source_hash: *blake3::hash(source).as_bytes(), svg: Arc::new(SVG.to_vec()) });
    doc.layers.push(Layer::new("native", LayerContent::Smart(sm)));
    doc
}

fn smart(doc: &Document) -> &SmartObject {
    let LayerContent::Smart(sm) = &doc.layers[0].content else { panic!("native vector was rasterized") };
    sm
}

#[test]
fn ai_pdf_eps_psd_and_psb_roundtrips_keep_source_bytes_preview_and_pixel_frame() {
    for (name, bytes) in [("Artwork.ai", PDF), ("Artwork.pdf", PDF), ("Artwork.eps", EPS)] {
        for extension in ["psd", "psb"] {
            let original = fixture(name, bytes);
            let output = export(&original, extension, &ExportOptions::default()).unwrap();
            assert!(!output.warnings.iter().any(|w| w.contains("smart object written as pixels")), "{:?}", output.warnings);
            let file = PsdFile::from_bytes(&output.bytes).unwrap();
            let placed = smart_map::parse_sold(&file.layers()[0].block(b"SoLd").unwrap().data).unwrap();
            assert_eq!(smart_map::stored_size(&placed.descriptor), Some((8.0, 6.0)));
            assert!(file.layers()[0].block(&vector_preview::PSD_KEY).is_some());
            let back = import(extension, &output.bytes).unwrap().document;
            let sm = smart(&back);
            let SmartSource::Linked { path } = &sm.source else { panic!("expected PSD embedded-table source") };
            let (actual_name, actual_bytes) = linked::embedded_payload(&back.metadata, path).unwrap();
            assert_eq!(actual_name, name);
            assert_eq!(actual_bytes, bytes);
            assert_eq!(sm.vector_preview, smart(&original).vector_preview);
            assert_eq!(sm.transform, smart(&original).transform);
            let again = export(&back, extension, &Default::default()).unwrap();
            let again = import(extension, &again.bytes).unwrap().document;
            assert_eq!(smart(&again).vector_preview, sm.vector_preview);
            assert_eq!(smart(&again).transform, sm.transform);
        }
    }
}

#[test]
fn stale_or_corrupt_psd_preview_is_discarded_and_native_source_survives() {
    let output = export(&fixture("Artwork.ai", PDF), "psd", &Default::default()).unwrap();
    for hash_offset in [8, 44] {
        let mut file = PsdFile::from_bytes(&output.bytes).unwrap();
        let block = file.layers_mut()[0].blocks.iter_mut().find(|b| b.key == vector_preview::PSD_KEY).unwrap();
        block.data[hash_offset] ^= 0xff;
        let back = import("test.psd", &file.to_bytes().unwrap()).unwrap().document;
        let sm = smart(&back);
        assert!(sm.vector_preview.is_none());
        let SmartSource::Linked { path } = &sm.source else { panic!("source disappeared") };
        assert_eq!(linked::embedded_payload(&back.metadata, path).unwrap().1, PDF);
    }
}

#[test]
fn replacing_native_source_cannot_export_a_stale_preview() {
    let mut doc = fixture("Artwork.ai", PDF);
    let LayerContent::Smart(sm) = &mut doc.layers[0].content else { unreachable!() };
    sm.source = SmartSource::Embedded { file_name: "Artwork.ai".into(), bytes: Arc::new(b"%PDF-1.7\nother".to_vec()) };
    let output = export(&doc, "psd", &Default::default()).unwrap();
    let file = PsdFile::from_bytes(&output.bytes).unwrap();
    assert!(file.layers()[0].block(&vector_preview::PSD_KEY).is_none());
    assert!(smart(&import("test.psd", &output.bytes).unwrap().document).vector_preview.is_none());
}

#[test]
fn native_preview_supplies_unfiltered_pixels_for_psd_smart_filter_masks() {
    let mut doc = fixture("Artwork.ai", PDF);
    let LayerContent::Smart(sm) = &mut doc.layers[0].content else { unreachable!() };
    sm.smart_filters.push(photocraft_doc::SmartFilter {
        command: "filter.blur.gaussianBlur".into(),
        params: serde_json::json!({"radius": 1.0}),
        blend: photocraft_color::BlendMode::Normal,
        opacity: 1.0,
        visible: true,
    });
    let mut mask = photocraft_doc::LayerMask::reveal_all();
    // All-white masks intentionally reopen as None. Use an actual masked region to prove
    // native-preview source pixels keep a nontrivial mask through PSD serialization.
    mask.surface.fill_rect(photocraft_geom::Rect::new(4, 4, 8, 8), &[0.0]);
    sm.filter_mask = Some(mask);
    let out = export(&doc, "psd", &Default::default()).unwrap();
    assert!(!out.warnings.iter().any(|warning| warning.contains("smart filter mask was not written")), "{:?}", out.warnings);
    let file = PsdFile::from_bytes(&out.bytes).unwrap();
    assert!(file.global_blocks.iter().any(|block| &block.key == b"FEid"));
    let back = import("test.psd", &out.bytes).unwrap().document;
    let mask = smart(&back).filter_mask.as_ref().expect("nontrivial filter mask was lost");
    assert_eq!(mask.value(5, 5), 0.0);
    assert_eq!(mask.value(0, 0), 1.0);
    assert!(smart(&back).vector_preview.is_some());
    assert_eq!(smart(&back).smart_filters.len(), 1);
}

#[test]
fn one_hundred_shared_native_instances_keep_source_bytes_and_preview_through_psd_resaves() {
    let mut doc = fixture("Artwork.ai", PDF);
    let layer = doc.layers[0].clone();
    for _ in 1..100 {
        doc.layers.push(Layer::new("native", layer.content.clone()));
    }
    let check = |bytes: &[u8]| {
        let back = import("shared.psd", bytes).unwrap().document;
        assert_eq!(back.layers.len(), 100);
        let file = PsdFile::from_bytes(bytes).unwrap();
        assert!(file.layers().iter().all(|layer| layer.block(&vector_preview::PSD_KEY).is_some()));
        for layer in &back.layers {
            let LayerContent::Smart(sm) = &layer.content else { panic!("native vector was rasterized") };
            assert_eq!(sm.vector_preview, smart(&doc).vector_preview);
            assert_eq!(sm.transform, smart(&doc).transform);
            let SmartSource::Linked { path } = &sm.source else { panic!("expected PSD embedded-table source") };
            assert_eq!(linked::embedded_payload(&back.metadata, path).unwrap(), ("Artwork.ai".to_string(), PDF));
        }
        back
    };
    let back = check(&export(&doc, "psd", &Default::default()).unwrap().bytes);
    check(&export(&back, "psd", &Default::default()).unwrap().bytes);
}

#[test]
fn native_preview_preserves_fractional_source_pixel_frame_in_psd() {
    let mut doc = fixture("Artwork.ai", PDF);
    let LayerContent::Smart(sm) = &mut doc.layers[0].content else { unreachable!() };
    let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" width="8.3" height="6.2"><rect width="8" height="6" fill="red"/></svg>"#;
    sm.vector_preview.as_mut().unwrap().svg = Arc::new(svg.to_vec());
    assert_eq!(vector_preview::frame(sm.vector_preview.as_ref().unwrap(), PDF), Some((8.3, 6.2)));
    let out = export(&doc, "psd", &Default::default()).unwrap();
    let file = PsdFile::from_bytes(&out.bytes).unwrap();
    let placed = smart_map::parse_sold(&file.layers()[0].block(b"SoLd").unwrap().data).unwrap();
    assert_eq!(smart_map::stored_size(&placed.descriptor), Some((8.3, 6.2)));
    let back = import("test.psd", &out.bytes).unwrap().document;
    assert_eq!(smart(&back).transform, smart(&doc).transform);
    let again = export(&back, "psd", &Default::default()).unwrap();
    let file = PsdFile::from_bytes(&again.bytes).unwrap();
    let placed = smart_map::parse_sold(&file.layers()[0].block(b"SoLd").unwrap().data).unwrap();
    assert_eq!(smart_map::stored_size(&placed.descriptor), Some((8.3, 6.2)));
}
