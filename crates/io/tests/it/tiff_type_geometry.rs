//! Photoshop's mixed-order Intel TIFF type blocks and unsafe/unresolved type fallback (#4758).

use photocraft_color::{ColorMode, SampleType};
use photocraft_doc::text::{CharStyle, TextRun};
use photocraft_doc::{Document, Layer, LayerContent, Size, TextLayer};
use photocraft_geom::Affine;
use photocraft_io::{ExportOptions, PsdExportOptions, document_to_psd_with, export, import, psd_to_document};
use photocraft_psd::descriptor::{Value, VersionedDescriptor};

fn document() -> Document {
    document_in(ColorMode::Rgb, SampleType::U8)
}

fn document_in(mode: ColorMode, depth: SampleType) -> Document {
    let mut doc = Document::new("Synthetic type", Size::new(640, 400), mode, depth);
    doc.resolution_dpi = 300.0;
    let mut t = TextLayer {
        text: "Synthetic".into(),
        transform: Affine { m: [1.8054202887117468, 0.0, -0.143, 1.8063853899082987, 40.0, 120.0] },
        ..Default::default()
    };
    t.runs = vec![TextRun { len: t.text.len(), style: CharStyle { size_pt: 24.0, ..Default::default() } }];
    t.sync_summary();
    t.cache = Some(photocraft_text::TextEngine::default().render(&t, doc.resolution_dpi, doc.pixel_format()).1.surface);
    doc.layers.push(Layer::new("Synthetic", LayerContent::Text(t)));
    doc
}

#[test]
fn photoshop_mixed_order_tiff_import_keeps_editable_geometry_and_pixels() {
    let doc = document();
    let (psd, _) = document_to_psd_with(&doc, &PsdExportOptions::default());
    let canonical = &psd.layers()[0].block(b"TySh").unwrap().data;
    let encoded = export(&doc, "test.tif", &ExportOptions { tiff_layers: true, ..Default::default() }).unwrap();
    let mut image = photocraft_codecs::decode(&encoded.bytes).unwrap();
    let blocks = image.meta.photoshop_layers.as_mut().unwrap();
    let at = blocks.windows(8).position(|b| b == b"MIB8hSyT").unwrap() + 12;
    // Only the text version/descriptor switch order; warp and bounds remain little-endian.
    let (_, used) = photocraft_psd::descriptor::VersionedDescriptor::parse_prefix(&canonical[52..]).unwrap();
    blocks[at + 50..at + 52 + used].copy_from_slice(&canonical[50..52 + used]);
    let bytes = photocraft_codecs::encode(&image, photocraft_codecs::Format::Tiff, &Default::default()).unwrap();
    let opened = import("photoshop.tif", &bytes).unwrap();
    assert!(!opened.warnings.iter().any(|w| w.contains("TySh")), "{:?}", opened.warnings);
    let LayerContent::Text(before) = &doc.layers[0].content else { panic!("source type") };
    let LayerContent::Text(after) = &opened.document.layers[0].content else { panic!("imported type") };
    assert_eq!(after.transform, before.transform);
    assert_eq!(after.size_pt, 24.0);
    assert_eq!(after.text, before.text);
    assert_eq!(after.cache.as_ref().unwrap().content_bounds(), before.cache.as_ref().unwrap().content_bounds());
    let rerendered = photocraft_text::TextEngine::default().render(after, opened.document.resolution_dpi, opened.document.pixel_format()).1.surface;
    assert_eq!(rerendered.content_bounds(), after.cache.as_ref().unwrap().content_bounds());
}

#[test]
fn layered_tiff_export_keeps_tysh_text_descriptor_big_endian() {
    for (mode, depth) in [
        (ColorMode::Rgb, SampleType::U8),
        (ColorMode::Rgb, SampleType::U16),
        (ColorMode::Rgb, SampleType::F32),
        (ColorMode::Grayscale, SampleType::U8),
        (ColorMode::Grayscale, SampleType::U16),
        (ColorMode::Grayscale, SampleType::F32),
        (ColorMode::Cmyk, SampleType::U8),
        (ColorMode::Cmyk, SampleType::U16),
    ] {
        let doc = document_in(mode, depth);
        let (psd, _) = document_to_psd_with(&doc, &PsdExportOptions::default());
        let canonical = &psd.layers()[0].block(b"TySh").unwrap().data;
        let encoded = export(&doc, "test.tif", &ExportOptions { tiff_layers: true, ..Default::default() }).unwrap();
        let (_, layers) = photocraft_codecs::tiff_photoshop_tags(&encoded.bytes);
        let layers = layers.expect("layered TIFF tag 37724");
        let block_at = layers.windows(8).position(|window| window == b"MIB8hSyT").expect("little-endian TySh block");
        let length_at = block_at + 8;
        let length_bytes: [u8; 4] = layers.get(length_at..length_at + 4).expect("TySh length").try_into().expect("four-byte length");
        let length = usize::try_from(u32::from_le_bytes(length_bytes)).expect("test block length fits usize");
        let data_at = length_at + 4;
        let payload = layers.get(data_at..data_at + length).expect("TySh payload");
        let (_, descriptor_len) = VersionedDescriptor::parse_prefix(canonical.get(52..).unwrap()).unwrap();
        let text_section_len = 2 + descriptor_len;
        assert_eq!(payload.get(50..52), Some(&50u16.to_be_bytes()[..]), "{mode:?} {depth:?}");
        assert_eq!(payload.get(52..56), Some(&16u32.to_be_bytes()[..]), "{mode:?} {depth:?}");
        assert_eq!(payload.get(50..50 + text_section_len), canonical.get(50..50 + text_section_len), "{mode:?} {depth:?}");
        let descriptor = VersionedDescriptor::parse_prefix(payload.get(52..).unwrap()).unwrap().0;
        assert!(matches!(descriptor.descriptor.get("Txt "), Some(Value::Text(value)) if value.to_string_lossy() == doc.layers[0].name));

        let opened = import("photoshop.tif", &encoded.bytes).unwrap();
        assert!(!opened.warnings.iter().any(|warning| warning.contains("TySh")), "{:?}", opened.warnings);
        let LayerContent::Text(before) = &doc.layers[0].content else { panic!("source type") };
        let LayerContent::Text(after) = &opened.document.layers[0].content else { panic!("imported type") };
        assert_eq!(after.transform, before.transform, "{mode:?} {depth:?}");
        assert_eq!(after.size_pt, before.size_pt, "{mode:?} {depth:?}");
        assert_eq!(after.text, before.text, "{mode:?} {depth:?}");
        assert_eq!(after.cache.as_ref().unwrap().content_bounds(), before.cache.as_ref().unwrap().content_bounds(), "{mode:?} {depth:?}");
    }
}

#[test]
fn foreign_or_unsafe_type_keeps_cached_pixels_without_editable_geometry() {
    let doc = document();
    for foreign in [true, false] {
        let (mut psd, _) = document_to_psd_with(&doc, &PsdExportOptions::default());
        let block = psd.layer_info.as_mut().unwrap().layers[0].blocks.iter_mut().find(|b| b.key == *b"TySh").unwrap();
        if foreign {
            // A foreign-order marker must be respected even if the descriptor could parse.
            block.signature = *b"MIB8";
        } else {
            block.data[2..10].copy_from_slice(&1.315776645269349e62f64.to_be_bytes());
        }
        let raw = block.data.clone();
        let (opened, warnings) = psd_to_document(&psd);
        let LayerContent::Raster(pixels) = &opened.layers[0].content else { panic!("unsafe type must stay pixels") };
        assert!(!pixels.content_bounds().is_empty());
        assert_eq!(pixels.content_bounds(), doc.layers[0].surface().unwrap().content_bounds());
        assert!(warnings.iter().any(|w| w.contains("unsafe type geometry")), "{warnings:?}");
        assert!(opened.layers[0].psd_blocks.iter().any(|(k, data)| k == b"TySh" && **data == raw));
    }
}

#[test]
fn canonical_type_the_typed_reader_rejects_stays_type_and_saves_its_block() {
    let doc = document();
    let (mut psd, _) = document_to_psd_with(&doc, &PsdExportOptions::default());
    let block = psd.layer_info.as_mut().unwrap().layers[0].blocks.iter_mut().find(|b| b.key == *b"TySh").unwrap();
    // A text descriptor version the typed reader doesn't know; the matrix is ordinary.
    block.data[52..56].copy_from_slice(&17u32.to_be_bytes());
    let raw = block.data.clone();
    let (opened, warnings) = psd_to_document(&psd);
    assert!(!warnings.iter().any(|w| w.contains("unsafe type geometry")), "{warnings:?}");
    let LayerContent::Text(t) = &opened.layers[0].content else { panic!("canonical type must stay a type layer") };
    let LayerContent::Text(before) = &doc.layers[0].content else { panic!("source type") };
    assert_eq!(t.transform, before.transform);
    let (saved, _) = document_to_psd_with(&opened, &PsdExportOptions::default());
    assert_eq!(saved.layers()[0].block(b"TySh").map(|b| &b.data), Some(&raw), "PSD save must keep the type block");
}
