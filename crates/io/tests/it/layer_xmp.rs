//! A layer's own XMP packet (`Layer::xmp`, PSD `shmd` › `cust` › `layerXMP`) survives every
//! layered format and keeps the rest of a Photoshop `shmd` block as it was.

use std::sync::Arc;

use photocraft_color::{ColorMode, SampleType};
use photocraft_doc::{Document, Layer, Size};
use photocraft_io::{ExportOptions, export, import};
use photocraft_psd::PsdFile;
use photocraft_psd::metadata::{layer_xmp, put_layer_xmp};

const A: &str = r#"<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"><rdf:Description rdf:about="" xmlns:app="http://example.com/app/1.0/"><app:arc>120</app:arc></rdf:Description></rdf:RDF></x:xmpmeta>"#;
const G: &str = r#"<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"><rdf:Description rdf:about="" xmlns:app="http://example.com/app/1.0/"><app:name>Café 🎨</app:name></rdf:Description></rdf:RDF></x:xmpmeta>"#;

/// `shmd` of a Photoshop 2026 layer without XMP: a `cust` item holding only `layerTime`.
const PS_WITHOUT_XMP: &str =
    "000000013842494d63757374000000000000003400000010000000010000000000086d6574616461746100000001000000096c6179657254696d65646f756241dab643ffd86bec00";

fn unhex(h: &str) -> Vec<u8> {
    (0..h.len()).step_by(2).map(|i| u8::from_str_radix(&h[i..i + 2], 16).unwrap()).collect()
}

fn xmps(d: &Document) -> Vec<(String, Option<String>)> {
    d.walk().into_iter().map(|(_, _, l)| (l.name.clone(), l.xmp.as_deref().map(str::to_string))).collect()
}

fn layer_shmd<'a>(f: &'a PsdFile, name: &str) -> Option<&'a [u8]> {
    f.layers().iter().find(|r| r.name() == name).and_then(|r| r.block(b"shmd")).map(|b| b.data.as_slice())
}

fn sample(depth: SampleType) -> Document {
    let mut doc = Document::new("Layer XMP", Size::new(4, 4), ColorMode::Rgb, depth);
    let mut a = Layer::raster("A", doc.pixel_format());
    a.xmp = Some(Arc::from(A));
    let plain = Layer::raster("Plain", doc.pixel_format());
    let mut g = Layer::group("G", vec![plain]);
    g.xmp = Some(Arc::from(G));
    doc.layers.extend([a, g]);
    doc
}

#[test]
fn layer_xmp_round_trips_through_every_layered_format_and_depth() {
    let psb = ExportOptions { force_psb: true, ..Default::default() };
    let tiff = ExportOptions { tiff_layers: true, ..Default::default() };
    for depth in SampleType::ALL {
        let doc = sample(depth);
        for (path, opts) in [("x.psd", &ExportOptions::default()), ("x.psb", &psb), ("x.tif", &tiff), ("x.pcraft", &ExportOptions::default())] {
            let bytes = export(&doc, path, opts).unwrap().bytes;
            let back = import(path, &bytes).unwrap().document;
            assert_eq!(xmps(&back), xmps(&doc), "{path} {depth:?}");
        }
        // Stored as Photoshop stores it; a layer without XMP gets no block.
        let bytes = export(&doc, "x.psd", &ExportOptions::default()).unwrap().bytes;
        let f = PsdFile::from_bytes(&bytes).unwrap();
        assert_eq!(layer_shmd(&f, "A").and_then(layer_xmp).as_deref(), Some(A));
        assert_eq!(layer_shmd(&f, "G").and_then(layer_xmp).as_deref(), Some(G));
        assert!(layer_shmd(&f, "Plain").is_none());
    }
}

#[test]
fn photoshop_shmd_keeps_its_other_data() {
    let without = unhex(PS_WITHOUT_XMP);
    let with = put_layer_xmp(Some(&without), A).unwrap();
    // A layer as an older PhotoCraft kept it: Photoshop's block verbatim, no field.
    let mut doc = Document::new("PS", Size::new(2, 2), ColorMode::Rgb, SampleType::U8);
    let mut l = Layer::raster("L", doc.pixel_format());
    l.psd_blocks.push((*b"shmd", Arc::new(with.clone())));
    doc.layers.push(l);
    let bytes = export(&doc, "x.psd", &ExportOptions::default()).unwrap().bytes;

    // Opening it gives the packet to the field and keeps the rest of the block.
    let mut back = import("x.psd", &bytes).unwrap().document;
    let l = &back.layers[0];
    assert_eq!(l.xmp.as_deref(), Some(A));
    assert_eq!(l.psd_blocks.iter().find(|(k, _)| k == b"shmd").map(|(_, d)| d.as_slice()), Some(without.as_slice()));

    // Unchanged, the block is written back byte for byte.
    let again = export(&back, "x.psd", &ExportOptions::default()).unwrap().bytes;
    assert_eq!(layer_shmd(&PsdFile::from_bytes(&again).unwrap(), "L"), Some(with.as_slice()));

    // Edited, `layerTime` stays; removed, the block is Photoshop's block without XMP.
    back.layers[0].xmp = Some(Arc::from(G));
    let edited = export(&back, "x.psd", &ExportOptions::default()).unwrap().bytes;
    assert_eq!(layer_shmd(&PsdFile::from_bytes(&edited).unwrap(), "L"), Some(put_layer_xmp(Some(&without), G).unwrap().as_slice()));
    back.layers[0].xmp = None;
    let removed = export(&back, "x.psd", &ExportOptions::default()).unwrap().bytes;
    assert_eq!(layer_shmd(&PsdFile::from_bytes(&removed).unwrap(), "L"), Some(without.as_slice()));
}

#[test]
fn an_unreadable_shmd_is_kept_and_reported() {
    let mut doc = Document::new("Bad", Size::new(2, 2), ColorMode::Rgb, SampleType::U8);
    let mut l = Layer::raster("L", doc.pixel_format());
    l.psd_blocks.push((*b"shmd", Arc::new(vec![0, 0, 0, 9])));
    l.xmp = Some(Arc::from(A));
    doc.layers.push(l);
    let out = export(&doc, "x.psd", &ExportOptions::default()).unwrap();
    assert!(out.warnings.iter().any(|w| w.contains("XMP metadata was not saved")), "{:?}", out.warnings);
    assert_eq!(layer_shmd(&PsdFile::from_bytes(&out.bytes).unwrap(), "L"), Some([0, 0, 0, 9].as_slice()));
}
