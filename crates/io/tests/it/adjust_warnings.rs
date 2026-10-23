//! Opening a file whose adjustment layer can't be read warns, naming the layer and the reason,
//! instead of opening it silently without its effect (#1763).

use photocraft_color::{ColorMode, SampleType};
use photocraft_doc::{Adjustment, Document, Layer, LayerContent};
use photocraft_geom::Size;
use photocraft_io::*;
use photocraft_psd::descriptor::{Descriptor, Id, UnicodeString, Value, VersionedDescriptor};
use photocraft_raster::Surface;

fn reopen(adj: Adjustment, path: &str, opts: &ExportOptions) -> (Document, Vec<String>) {
    let mut d = Document::new("a", Size::new(16, 16), ColorMode::Rgb, SampleType::U8);
    d.layers.push(Layer::new("Teal Look", LayerContent::Adjustment(adj)));
    let r = export(&d, path, opts).expect("export");
    let imp = import(path, &r.bytes).expect("import");
    (imp.document, imp.warnings)
}

#[test]
fn unreadable_adjustments_warn_on_open() {
    // A Color Lookup that uses a profile, not a LUT file: kept verbatim, not applied.
    let profile = Adjustment::Unsupported { psd_key: "clrL".into(), raw: vec![0, 2, 0, 0] };
    for (path, opts) in [("x.psd", ExportOptions::default()), ("x.psb", ExportOptions { force_psb: true, ..Default::default() })] {
        let (doc, warnings) = reopen(profile.clone(), path, &opts);
        assert!(matches!(&doc.layers[0].content, LayerContent::Adjustment(Adjustment::Unsupported { .. })), "{path}");
        let w: Vec<&String> = warnings.iter().filter(|w| w.contains("Teal Look")).collect();
        assert_eq!(w.len(), 1, "{path}: {warnings:?}");
        assert!(w[0].contains("Color Lookup") && w[0].contains("version 2") && w[0].contains("no effect"), "{path}: {}", w[0]);
    }
    // Another kind without a specific reason still names the layer and the adjustment.
    let (_, warnings) = reopen(Adjustment::Unsupported { psd_key: "levl".into(), raw: vec![0, 9] }, "x.psd", &ExportOptions::default());
    assert!(warnings.iter().any(|w| w.contains("Teal Look") && w.contains("Levels settings could not be read;")), "{warnings:?}");
}

#[test]
fn readable_adjustments_open_without_warnings() {
    let lut: Vec<f32> = (0..8).flat_map(|i| [(i & 1) as f32, ((i >> 1) & 1) as f32, ((i >> 2) & 1) as f32]).collect();
    let lookup = Adjustment::ColorLookup { name: "id.cube".into(), size: 2, lut: Some(std::sync::Arc::new(lut)), tetrahedral: false, dither: false };
    for adj in [lookup, Adjustment::Invert] {
        let (doc, warnings) = reopen(adj, "x.psd", &ExportOptions::default());
        assert!(!matches!(&doc.layers[0].content, LayerContent::Adjustment(Adjustment::Unsupported { .. })));
        assert!(!warnings.iter().any(|w| w.contains("Teal Look")), "{warnings:?}");
    }
}

/// The raw bytes of a `clrL` block laid out the way Photoshop writes it (Adobe spec): version 1,
/// then a version 16 descriptor carrying a small `.cube` LUT file (its entry for red maps to
/// cyan, so the table is not the identity).
fn photoshop_clr_l_block() -> Vec<u8> {
    let cube = b"LUT_3D_SIZE 2\n0 0 0\n0 1 1\n0 1 0\n1 1 0\n0 0 1\n1 0 1\n0 1 1\n1 1 1\n";
    let en = |t: &str, val: &str| Value::Enumerated { type_id: Id::new(t), value: Id::new(val) };
    let text = |t: &str| Value::Text(UnicodeString::new_nul(t));
    let desc = Descriptor::new("null")
        .with("NM  ", text("Frost"))
        .with("LUTFormat", en("LUTFormatType", "LUTFormatCUBE"))
        .with("LUT3DFileName", text("frost.cube"))
        .with("LUT3DFileData", Value::RawData(cube.to_vec()));
    let mut raw = vec![0, 1];
    raw.extend(VersionedDescriptor::new(desc).to_bytes());
    assert_eq!(&raw[..6], &[0, 1, 0, 0, 0, 16], "version 1, then the descriptor's version 16");
    raw
}

#[test]
fn photoshop_layout_color_lookup_round_trips_through_psb() {
    // A kept-raw `clrL` block in Photoshop's layout survives a PSB round trip and reopens as a
    // working Color Lookup, with no warning naming the layer (#1763).
    let raw = photoshop_clr_l_block();
    let opts = ExportOptions { force_psb: true, ..Default::default() };
    let (doc, warnings) = reopen(Adjustment::Unsupported { psd_key: "clrL".into(), raw: raw.clone() }, "x.psb", &opts);
    assert!(warnings.iter().all(|w| !w.contains("Teal Look")), "{warnings:?}");
    let LayerContent::Adjustment(Adjustment::ColorLookup { name, size, lut: Some(table), tetrahedral: false, dither: false }) = &doc.layers[0].content else {
        panic!("{:?}", doc.layers[0].content)
    };
    assert_eq!(name, "frost.cube");
    assert_eq!(*size, 2);
    let want: Vec<f32> = [0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 0.0, 1.0, 0.0, 1.0, 1.0, 0.0, 0.0, 0.0, 1.0, 1.0, 0.0, 1.0, 0.0, 1.0, 1.0, 1.0, 1.0, 1.0].into();
    assert_eq!(table.as_slice(), want.as_slice());
    // The exporter wrote the block verbatim (the importer keeps it for saving; the block writer
    // pads to Photoshop's four-byte alignment, which the spec requires).
    let kept = doc.layers[0].psd_blocks.iter().find(|(k, _)| k == b"clrL").map(|(_, d)| d.as_slice());
    let Some(kept) = kept else { panic!("{:?}", doc.layers[0].psd_blocks) };
    assert!(kept.starts_with(&raw) && kept[raw.len()..].iter().all(|&b| b == 0), "{kept:?}");
    // The reopened lookup really applies: a red backdrop renders as the table's cyan.
    let mut d = doc;
    let mut bg = Surface::new(d.pixel_format());
    bg.fill_rect(d.bounds(), &[1.0, 0.0, 0.0, 1.0]);
    d.layers.insert(0, Layer::new("bg", LayerContent::Raster(bg)));
    let px = photocraft_compose::flatten(&d).px[0];
    assert!(px[0] < 0.05 && (px[1] - 1.0).abs() < 0.05 && (px[2] - 1.0).abs() < 0.05, "{px:?}");
}

#[test]
fn short_color_lookup_table_warns_on_open() {
    // A stored table shorter than size³×3 renders as the identity; opening must say so, naming
    // the layer (#1763).
    let short = Adjustment::ColorLookup { name: "t.cube".into(), size: 3, lut: Some(std::sync::Arc::new(vec![0.5; 7])), tetrahedral: false, dither: false };
    let (doc, warnings) = reopen(short, "x.pcraft", &ExportOptions::default());
    assert!(matches!(&doc.layers[0].content, LayerContent::Adjustment(Adjustment::ColorLookup { .. })), "{:?}", doc.layers[0].content);
    let w: Vec<&String> = warnings.iter().filter(|w| w.contains("Teal Look")).collect();
    assert_eq!(w.len(), 1, "{warnings:?}");
    assert!(w[0].contains("Color Lookup") && w[0].contains("7 values") && w[0].contains("81"), "{}", w[0]);
    // A table long enough for its size opens without the note.
    let ok = Adjustment::ColorLookup { name: "t.cube".into(), size: 2, lut: Some(std::sync::Arc::new(vec![0.5; 24])), tetrahedral: false, dither: false };
    let (_, warnings) = reopen(ok, "x.pcraft", &ExportOptions::default());
    assert!(warnings.iter().all(|w| !w.contains("Teal Look")), "{warnings:?}");
    // A missing table renders as the identity too.
    let none = Adjustment::ColorLookup { name: "t.cube".into(), size: 2, lut: None, tetrahedral: false, dither: false };
    let (_, warnings) = reopen(none, "x.pcraft", &ExportOptions::default());
    assert!(warnings.iter().any(|w| w.contains("Teal Look") && w.contains("no usable table")), "{warnings:?}");
}
