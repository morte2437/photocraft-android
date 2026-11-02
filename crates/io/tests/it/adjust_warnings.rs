//! Opening a file whose adjustment layer can't be read warns, naming the layer and the reason,
//! instead of opening it silently without its effect (#1763).

use photocraft_color::{ColorMode, SampleType};
use photocraft_doc::{Adjustment, Document, Layer, LayerContent};
use photocraft_geom::Size;
use photocraft_io::*;
use photocraft_psd::descriptor::{Descriptor, Id, UnicodeString, Value, VersionedDescriptor};
use photocraft_psd::{Compression, LayerFlags, LayerRecord, PsdFile, TaggedBlock, Version, testgen};
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
    let profile = Adjustment::Unsupported { psd_key: "clrL".into(), raw: vec![0, 2, 0, 0].into() };
    for (path, opts) in [("x.psd", ExportOptions::default()), ("x.psb", ExportOptions { force_psb: true, ..Default::default() })] {
        let (doc, warnings) = reopen(profile.clone(), path, &opts);
        assert!(matches!(&doc.layers[0].content, LayerContent::Adjustment(Adjustment::Unsupported { .. })), "{path}");
        let w: Vec<&String> = warnings.iter().filter(|w| w.contains("Teal Look")).collect();
        assert_eq!(w.len(), 1, "{path}: {warnings:?}");
        assert!(w[0].contains("Color Lookup") && w[0].contains("version 2") && w[0].contains("no effect"), "{path}: {}", w[0]);
    }
    // Another kind without a specific reason still names the layer and the adjustment.
    let (_, warnings) = reopen(Adjustment::Unsupported { psd_key: "levl".into(), raw: vec![0, 9].into() }, "x.psd", &ExportOptions::default());
    assert!(warnings.iter().any(|w| w.contains("Teal Look") && w.contains("Levels settings could not be read;")), "{warnings:?}");
}

#[test]
fn readable_adjustments_open_without_warnings() {
    let lut: Vec<f32> = (0..8).flat_map(|i| [(i & 1) as f32, ((i >> 1) & 1) as f32, ((i >> 2) & 1) as f32]).collect();
    let lookup =
        Adjustment::ColorLookup { name: "id.cube".into(), size: 2, lut: Some(std::sync::Arc::new(lut)), tetrahedral: false, dither: false, source: None };
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
    photoshop_clr_l_block_from_cube(cube)
}

fn photoshop_clr_l_block_from_cube(cube: &[u8]) -> Vec<u8> {
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

fn identity_cube(domain: &str) -> Vec<u8> {
    format!("{domain}LUT_3D_SIZE 2\n0 0 0\n1 0 0\n0 1 0\n1 1 0\n0 0 1\n1 0 1\n0 1 1\n1 1 1\n").into_bytes()
}

fn synthetic_lookup_psd(version: Version, depth: u16, cube: &[u8]) -> Vec<u8> {
    const LAYER_NAME: &str = "Custom-domain LUT";
    let mut file = testgen::layered(version, photocraft_psd::ColorMode::Rgb, depth, Compression::Rle);
    let mut flags = LayerFlags::default();
    flags.set_pixel_data_irrelevant(true);
    // Photoshop stores layer-block alignment inside the block length. Make the source payload
    // canonical so the exporter can preserve it exactly rather than adding alignment on save.
    let mut clrl = photoshop_clr_l_block_from_cube(cube);
    clrl.resize(clrl.len().next_multiple_of(2), 0);
    let record = LayerRecord {
        name: LAYER_NAME.as_bytes().to_vec(),
        flags,
        blocks: vec![TaggedBlock::new(*b"clrL", clrl), TaggedBlock::unicode_name(LAYER_NAME), TaggedBlock::layer_id(9876)],
        ..Default::default()
    };
    file.layers_mut().insert(0, record);
    file.to_bytes().expect("serialize synthetic PSD/PSB")
}

fn unsupported_lookup_raw(doc: &Document) -> &[u8] {
    let layer = doc.layers.iter().find(|layer| layer.name == "Custom-domain LUT").expect("lookup layer");
    let LayerContent::Adjustment(Adjustment::Unsupported { psd_key, raw }) = &layer.content else {
        panic!("expected preserved unsupported Color Lookup, got {:?}", layer.content)
    };
    assert_eq!(psd_key, "clrL");
    raw
}

fn parsed_lookup_block(bytes: &[u8]) -> (String, Vec<u8>) {
    let file = PsdFile::from_bytes(bytes).expect("parse PSD/PSB");
    let layer = file.layers().iter().find(|layer| layer.name() == "Custom-domain LUT").expect("lookup layer record");
    let blocks: Vec<_> = layer.blocks.iter().filter(|block| block.key == *b"clrL").collect();
    assert_eq!(blocks.len(), 1, "lookup blocks on layer: {blocks:?}");
    (layer.name(), blocks[0].data.clone())
}

#[test]
fn photoshop_layout_color_lookup_round_trips_through_psb() {
    // A kept-raw `clrL` block in Photoshop's layout survives a PSB round trip and reopens as a
    // working Color Lookup, with no warning naming the layer (#1763).
    let raw = photoshop_clr_l_block();
    let opts = ExportOptions { force_psb: true, ..Default::default() };
    let (doc, warnings) = reopen(Adjustment::Unsupported { psd_key: "clrL".into(), raw: raw.as_slice().into() }, "x.psb", &opts);
    assert!(warnings.iter().all(|w| !w.contains("Teal Look")), "{warnings:?}");
    let LayerContent::Adjustment(Adjustment::ColorLookup { name, size, lut: Some(table), tetrahedral: false, dither: false, source: Some(source) }) =
        &doc.layers[0].content
    else {
        panic!("{:?}", doc.layers[0].content)
    };
    assert_eq!(name, "frost.cube");
    // The loaded `.cube` file is kept as its source, so a later save can embed it unchanged.
    assert_eq!(source.format, photocraft_doc::adjust::LutFormat::Cube);
    assert_eq!(source.bytes.as_slice(), b"LUT_3D_SIZE 2\n0 0 0\n0 1 1\n0 1 0\n1 1 0\n0 0 1\n1 0 1\n0 1 1\n1 1 1\n");
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
    let short = Adjustment::ColorLookup {
        name: "t.cube".into(),
        size: 3,
        lut: Some(std::sync::Arc::new(vec![0.5; 7])),
        tetrahedral: false,
        dither: false,
        source: None,
    };
    let (doc, warnings) = reopen(short, "x.pcraft", &ExportOptions::default());
    assert!(matches!(&doc.layers[0].content, LayerContent::Adjustment(Adjustment::ColorLookup { .. })), "{:?}", doc.layers[0].content);
    let w: Vec<&String> = warnings.iter().filter(|w| w.contains("Teal Look")).collect();
    assert_eq!(w.len(), 1, "{warnings:?}");
    assert!(w[0].contains("Color Lookup") && w[0].contains("7 values") && w[0].contains("81"), "{}", w[0]);
    // A table long enough for its size opens without the note.
    let ok = Adjustment::ColorLookup {
        name: "t.cube".into(),
        size: 2,
        lut: Some(std::sync::Arc::new(vec![0.5; 24])),
        tetrahedral: false,
        dither: false,
        source: None,
    };
    let (_, warnings) = reopen(ok, "x.pcraft", &ExportOptions::default());
    assert!(warnings.iter().all(|w| !w.contains("Teal Look")), "{warnings:?}");
    // A missing table renders as the identity too.
    let none = Adjustment::ColorLookup { name: "t.cube".into(), size: 2, lut: None, tetrahedral: false, dither: false, source: None };
    let (_, warnings) = reopen(none, "x.pcraft", &ExportOptions::default());
    assert!(warnings.iter().any(|w| w.contains("Teal Look") && w.contains("no usable table")), "{warnings:?}");
}

#[test]
fn custom_domain_lookup_warns_and_round_trips_verbatim() {
    let domains = ["DOMAIN_MIN 0 0 0\nDOMAIN_MAX 0.5 0.5 0.5\n", "DOMAIN_MIN 0 0.25 0\nDOMAIN_MAX 1 0.75 1\n", "LUT_3D_INPUT_RANGE 0 0.5\n"];
    for domain in domains {
        let cube = identity_cube(domain);
        for (version, depth, extension) in [
            (Version::Psd, 8, "psd"),
            (Version::Psd, 16, "psd"),
            (Version::Psd, 32, "psd"),
            (Version::Psb, 8, "psb"),
            (Version::Psb, 16, "psb"),
            (Version::Psb, 32, "psb"),
        ] {
            let input = synthetic_lookup_psd(version, depth, &cube);
            let (_, source_block) = parsed_lookup_block(&input);
            let name = format!("input.{extension}");
            let opened = import(&name, &input).expect("open synthetic PSD/PSB");
            assert_eq!(unsupported_lookup_raw(&opened.document), source_block, "{extension}, {depth}-bit, {domain:?}");
            let warnings: Vec<_> = opened.warnings.iter().filter(|warning| warning.contains("Custom-domain LUT")).collect();
            assert_eq!(warnings.len(), 1, "{extension}, {depth}-bit: {:?}", opened.warnings);
            assert!(
                warnings[0].contains("Color Lookup")
                    && warnings[0].contains("DOMAIN_MIN")
                    && warnings[0].contains("DOMAIN_MAX")
                    && warnings[0].to_ascii_lowercase().contains("not supported"),
                "{}",
                warnings[0]
            );

            let native = export(&opened.document, "preserved.pcraft", &ExportOptions::default()).expect("save native document");
            let native_open = import("preserved.pcraft", &native.bytes).expect("reopen native document");
            assert_eq!(unsupported_lookup_raw(&native_open.document), source_block, "native preservation, {extension}, {depth}-bit");

            let output_name = format!("roundtrip.{extension}");
            let options = ExportOptions { force_psb: version.is_psb(), ..Default::default() };
            let saved = export(&native_open.document, &output_name, &options).expect("re-export PSD/PSB");
            let (_, output_block) = parsed_lookup_block(&saved.bytes);
            assert_eq!(output_block, source_block, "saved block, {extension}, {depth}-bit, {domain:?}");
            let reopened = import(&output_name, &saved.bytes).expect("reopen re-exported PSD/PSB");
            assert_eq!(unsupported_lookup_raw(&reopened.document), source_block, "reopened block, {extension}, {depth}-bit");
        }
    }
}

#[test]
fn default_domain_lookup_stays_readable() {
    for (version, domain, extension) in [(Version::Psd, "", "psd"), (Version::Psb, "DOMAIN_MIN 0 0 0\nDOMAIN_MAX 1 1 1\n", "psb")] {
        let cube = identity_cube(domain);
        let input = synthetic_lookup_psd(version, 8, &cube);
        let opened = import(&format!("default.{extension}"), &input).expect("open default-domain LUT");
        let layer = opened.document.layers.iter().find(|layer| layer.name == "Custom-domain LUT").expect("lookup layer");
        assert!(matches!(layer.content, LayerContent::Adjustment(Adjustment::ColorLookup { size: 2, .. })), "{:?}", layer.content);
        assert!(opened.warnings.iter().all(|warning| !warning.contains("Custom-domain LUT")), "{:?}", opened.warnings);
    }
}
