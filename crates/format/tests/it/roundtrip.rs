//! doc → .pcraft → doc equality, incremental saves, ids, previews.

use crate::common;
use common::*;
use photocraft_color::{ColorMode, SampleType};
use photocraft_doc::text::{CharStyle, ParagraphStyle, TextRun};
use photocraft_doc::text_styles::{CharacterStyleDef, ParagraphStyleDef};
use photocraft_doc::{Document, Layer, LayerContent, LayerId, Slice, TextLayer, VarKind, VariableDef};
use photocraft_format::*;
use photocraft_raster::Rgba8Image;

const MODES: [ColorMode; 4] = [ColorMode::Rgb, ColorMode::Grayscale, ColorMode::Cmyk, ColorMode::Lab];

/// Create inert historical conversion metadata with one native target and a separately stored original attachment.
fn conversion_report() -> photocraft_doc::import_conversion::ImportConversion {
    use photocraft_doc::import_conversion::*;
    let owner = SourceItemRef { kind: "layer".into(), ordinal: 3, name: "Duplicate".into(), ..Default::default() };
    ImportConversion::Report(ConversionReport {
        source_format: "xcf".into(),
        source_version: Some(26),
        policy_version: 1,
        entries: vec![ConversionEntry {
            feature_code: "blend.composite".into(),
            source: owner.clone(),
            targets: vec![NativeTargetRef { layer_id: Some(LayerId(800_000)), label: "Historical target".into(), ..Default::default() }],
            disposition: ConversionDisposition::NativeApproximation,
            reason: ConversionReason::NativePolicy,
            source_description: "Source intersection".into(),
            native_description: "Native source-over".into(),
            attachments: vec![0],
        }],
        attachments: vec![RetainedImportData {
            owner,
            kind: "configuration".into(),
            encoding: "utf-8".into(),
            bytes: std::sync::Arc::new(b"inert-config".to_vec()),
        }],
    })
}

#[test]
/// Round-trip converted native structures and off-canvas samples at every depth without losing their report.
fn converted_native_groups_masks_off_canvas_pixels_and_report_survive_at_every_depth() {
    for depth in SampleType::ALL {
        let mut doc = Document::new("Native converted components", photocraft_geom::Size::new(4, 3), ColorMode::Rgb, depth);
        let mut raster = Layer::raster("Pixels", doc.pixel_format());
        raster.surface_mut().unwrap().write_pixel(-2, 1, &[if depth == SampleType::F32 { 1.5 } else { 0.75 }, 0.25, 0.5, 0.8]);
        let mut coverage = photocraft_raster::Surface::new(photocraft_color::PixelFormat::new(ColorMode::Grayscale, depth, false));
        coverage.write_pixel(-2, 1, &[0.5]);
        raster.mask = Some(photocraft_doc::LayerMask { surface: coverage, enabled: true, linked: true, density: 0.8, feather: 1.25 });
        let child = raster.id;
        let group = Layer::group("Native group", vec![raster]);
        let group_id = group.id;
        doc.layers.push(group);
        let mut report = conversion_report();
        if let photocraft_doc::import_conversion::ImportConversion::Report(report) = &mut report {
            report.entries[0].targets = vec![
                photocraft_doc::import_conversion::NativeTargetRef { layer_id: Some(group_id), ..Default::default() },
                photocraft_doc::import_conversion::NativeTargetRef { layer_id: Some(child), ..Default::default() },
            ];
        }
        doc.metadata.import_conversion = Some(report.into());
        let bytes = save_to_bytes(&doc, &SaveOptions::default()).unwrap();
        let reopened = load_from_bytes(&bytes).unwrap();
        assert!(reopened == doc, "native components or inert report changed at {depth:?}");
    }
}

#[test]
/// Verify report attachments use native hashed blobs and never acquire executable document semantics.
fn import_conversion_roundtrip_is_inert_and_uses_native_blobs() {
    for depth in SampleType::ALL {
        let mut doc = Document::new("Converted", photocraft_geom::Size::new(4, 3), ColorMode::Rgb, depth);
        doc.metadata.import_conversion = Some(conversion_report().into());
        let bytes = save_to_bytes(&doc, &SaveOptions::default()).unwrap();
        assert_eq!(load_from_bytes(&bytes).unwrap(), doc);
        let fresh = load_from_bytes_with(&bytes, &LoadOptions { preserve_ids: false, ..Default::default() }).unwrap();
        assert_eq!(fresh.metadata.import_conversion, doc.metadata.import_conversion, "historical targets do not become live identity references");
        let zip = zip::ZipReader::new(&bytes).unwrap();
        let manifest: serde_json::Value = serde_json::from_slice(&zip.read(zip.find("manifest.json").unwrap(), 1 << 20).unwrap()).unwrap();
        let report = &manifest["document"]["metadata"]["import_conversion"];
        assert_eq!(report["schema_version"], 1);
        assert!(report["record"].is_string());
        assert!(report["attachments"][0].is_string());
        assert!(!String::from_utf8_lossy(&zip.read(zip.find("manifest.json").unwrap(), 1 << 20).unwrap()).contains("inert-config"));
        assert_eq!(zip.entries.iter().filter(|e| e.name.starts_with("blobs/")).count(), 2);
    }
}

#[test]
/// Preserve a future report schema and its bytes as opaque optional metadata.
fn unknown_optional_import_report_survives_without_interpretation() {
    use photocraft_doc::import_conversion::ImportConversion;
    use std::sync::Arc;
    let mut doc = Document::new("Future report", photocraft_geom::Size::new(1, 1), ColorMode::Grayscale, SampleType::U16);
    doc.metadata.import_conversion = Some(Arc::new(ImportConversion::Opaque {
        schema_version: 99,
        record: Arc::new(br#"{"future-fields":["never execute"]}"#.to_vec()),
        attachments: vec![Arc::new(vec![0, 255, 8])],
    }));
    let bytes = save_to_bytes(&doc, &SaveOptions::default()).unwrap();
    assert_eq!(load_from_bytes(&bytes).unwrap(), doc);
}

/// Native loading preserves known and future report values while subsequent document clones share their entire report allocation.
#[test]
fn native_report_roundtrip_restores_shared_known_and_opaque_metadata() {
    use photocraft_doc::import_conversion::ImportConversion;
    use std::sync::Arc;
    let future = ImportConversion::Opaque {
        schema_version: 77,
        record: Arc::new(br#"{"unknown":"preserve verbatim"}"#.to_vec()),
        attachments: vec![Arc::new(vec![0, 17, 255, 42])],
    };
    for report in [conversion_report(), future] {
        let mut doc = Document::new("Shared native provenance", photocraft_geom::Size::new(1, 1), ColorMode::Rgb, SampleType::F32);
        doc.metadata.import_conversion = Some(Arc::new(report));
        let bytes = save_to_bytes(&doc, &SaveOptions::default()).unwrap();
        let reopened = load_from_bytes(&bytes).unwrap();
        assert_eq!(reopened.metadata.import_conversion, doc.metadata.import_conversion);
        let clone = reopened.clone();
        assert!(Arc::ptr_eq(reopened.metadata.import_conversion.as_ref().unwrap(), clone.metadata.import_conversion.as_ref().unwrap()));
        let Some(ImportConversion::Opaque { record, attachments, schema_version }) = reopened.metadata.import_conversion.as_deref() else { continue };
        assert_eq!(*schema_version, 77);
        assert_eq!(record.as_slice(), br#"{"unknown":"preserve verbatim"}"#);
        assert_eq!(attachments[0].as_slice(), &[0, 17, 255, 42]);
    }
}

#[test]
/// Reject invalid attachment references before writing a native bundle.
fn import_report_invalid_references_are_rejected_before_save() {
    use photocraft_doc::import_conversion::ImportConversion;
    let mut doc = Document::new("Invalid report", photocraft_geom::Size::new(1, 1), ColorMode::Rgb, SampleType::U8);
    let mut report = conversion_report();
    if let ImportConversion::Report(report) = &mut report {
        report.entries[0].attachments = vec![1];
    }
    doc.metadata.import_conversion = Some(report.into());
    assert!(save_to_bytes(&doc, &SaveOptions::default()).is_err());
}

#[test]
/// Preserve ordinal/path identity and multiple historical targets even when source display names repeat.
fn import_report_distinguishes_duplicate_names_and_preserves_one_to_many_targets() {
    use photocraft_doc::import_conversion::*;
    let mut doc = Document::new("Duplicate items", photocraft_geom::Size::new(1, 1), ColorMode::Rgb, SampleType::U8);
    let mut data = conversion_report();
    if let ImportConversion::Report(report) = &mut data {
        let mut other = report.entries[0].clone();
        other.source.ordinal = 4;
        other.targets.push(NativeTargetRef { channel_index: Some(2), ..Default::default() });
        report.entries.push(other);
        assert_eq!(report.entries[0].source.name, report.entries[1].source.name);
        assert_ne!(report.entries[0].source, report.entries[1].source);
    }
    doc.metadata.import_conversion = Some(data.into());
    assert_eq!(load_from_bytes(&save_to_bytes(&doc, &SaveOptions::default()).unwrap()).unwrap(), doc);
}

#[test]
/// Check backward compatibility when the optional native report field is absent.
fn older_bundles_without_import_conversion_still_load() {
    let doc = Document::new("Older bundle", photocraft_geom::Size::new(1, 1), ColorMode::Rgb, SampleType::U8);
    let bytes = save_to_bytes(&doc, &SaveOptions::default()).unwrap();
    let zip = zip::ZipReader::new(&bytes).unwrap();
    let manifest: serde_json::Value = serde_json::from_slice(&zip.read(zip.find("manifest.json").unwrap(), 1 << 20).unwrap()).unwrap();
    assert!(manifest["document"]["metadata"].get("import_conversion").is_none());
    assert_eq!(load_from_bytes(&bytes).unwrap(), doc);
}

/// Rewrite only the fixture's report manifest to exercise corrupt framing and missing-blob validation.
fn edit_import_manifest(bytes: &[u8], edit: impl Fn(&mut serde_json::Value)) -> Vec<u8> {
    edit_import_bundle(bytes, None, edit)
}

/// Preserve the report manifest while optionally omitting one named bundle entry.
fn edit_import_bundle(bytes: &[u8], omit: Option<&str>, edit: impl Fn(&mut serde_json::Value)) -> Vec<u8> {
    let zip = zip::ZipReader::new(bytes).unwrap();
    let mut out = zip::ZipWriter::new();
    for entry in &zip.entries {
        if Some(entry.name.as_str()) == omit {
            continue;
        }
        let mut data = zip.read(entry, usize::MAX).unwrap();
        if entry.name == "manifest.json" {
            let mut value = serde_json::from_slice(&data).unwrap();
            edit(&mut value);
            data = serde_json::to_vec(&value).unwrap();
        }
        out.add(&entry.name, &data).unwrap();
    }
    out.finish().unwrap()
}

#[test]
/// Require controlled native-load errors for malformed report records and unavailable attachment blobs.
fn import_report_corrupt_framing_and_missing_blobs_fail_without_panic() {
    let mut doc = Document::new("Report framing", photocraft_geom::Size::new(1, 1), ColorMode::Rgb, SampleType::U8);
    doc.metadata.import_conversion = Some(conversion_report().into());
    let bytes = save_to_bytes(&doc, &SaveOptions::default()).unwrap();
    let zip = zip::ZipReader::new(&bytes).unwrap();
    let manifest: serde_json::Value = serde_json::from_slice(&zip.read(zip.find("manifest.json").unwrap(), 1 << 20).unwrap()).unwrap();
    let attachment = manifest["document"]["metadata"]["import_conversion"]["attachments"][0].as_str().unwrap();
    let attachment_path = craft_pcraft_format::blob_path(attachment).unwrap();
    assert!(zip.find(&attachment_path).is_some());
    let missing_attachment = edit_import_bundle(&bytes, Some(&attachment_path), |_| {});
    let missing_zip = zip::ZipReader::new(&missing_attachment).unwrap();
    assert!(missing_zip.find(&attachment_path).is_none());
    let unchanged: serde_json::Value = serde_json::from_slice(&missing_zip.read(missing_zip.find("manifest.json").unwrap(), 1 << 20).unwrap()).unwrap();
    assert_eq!(unchanged, manifest);
    for bad in [
        missing_attachment,
        edit_import_manifest(&bytes, |v| v["document"]["metadata"]["import_conversion"]["schema_version"] = 0.into()),
        edit_import_manifest(&bytes, |v| v["document"]["metadata"]["import_conversion"]["attachments"] = serde_json::json!([])),
        edit_import_manifest(&bytes, |v| v["document"]["metadata"]["import_conversion"]["record"] = "0".repeat(64).into()),
    ] {
        assert!(load_from_bytes(&bad).is_err());
    }
}

#[test]
/// Count escaped JSON bytes so serialization expansion cannot bypass the independent report ceiling.
fn import_report_encoded_size_is_bounded_even_when_strings_expand_in_json() {
    use photocraft_doc::import_conversion::ImportConversion;
    let mut doc = Document::new("Escaped report", photocraft_geom::Size::new(1, 1), ColorMode::Rgb, SampleType::U8);
    let mut report = conversion_report();
    if let ImportConversion::Report(report) = &mut report {
        let mut entry = report.entries[0].clone();
        entry.native_description = "\0".repeat(600_000);
        report.entries = vec![entry; 10];
    }
    assert!(report.validate().is_ok(), "unescaped model fits, encoded record does not");
    doc.metadata.import_conversion = Some(report.into());
    assert!(matches!(save_to_bytes(&doc, &SaveOptions::default()), Err(FormatError::LimitExceeded(_))));
}

#[test]
/// Ensure blob-cache reuse still enforces report payload limits rather than treating cached data as free.
fn import_report_limits_also_apply_to_already_cached_blobs() {
    use photocraft_doc::import_conversion::MAX_CONVERSION_BYTES;
    let mut doc = Document::new("Cached blob", photocraft_geom::Size::new(1, 1), ColorMode::Rgb, SampleType::U8);
    doc.icc_profile = Some(std::sync::Arc::new(vec![0; MAX_CONVERSION_BYTES + 1]));
    let mut layer = Layer::raster("Cached", doc.pixel_format());
    layer.psd_blocks.push((*b"test", doc.icc_profile.as_ref().unwrap().clone()));
    doc.layers.push(layer);
    doc.metadata.import_conversion = Some(conversion_report().into());
    let bytes = save_to_bytes(&doc, &SaveOptions::default()).unwrap();
    let bad = edit_import_manifest(&bytes, |v| {
        v["document"]["metadata"]["import_conversion"]["attachments"][0] = v["document"]["icc_profile"].clone();
    });
    assert!(matches!(load_from_bytes(&bad), Err(FormatError::LimitExceeded(ref message)) if message == "optional preview blob exceeds its limit"));
}

#[test]
/// Verify measured loading reports unique decoded blob work without double-charging shared references.
fn measured_native_load_reports_existing_unique_blob_accounting() {
    let mut doc = Document::new("Measured contents", photocraft_geom::Size::new(1, 1), ColorMode::Rgb, SampleType::U8);
    let shared = std::sync::Arc::new(vec![7; 123]);
    doc.metadata.exif = Some(shared.clone());
    doc.metadata.psd_resources.push((1000, "Shared".into(), shared));
    let bytes = save_to_bytes(&doc, &SaveOptions::default()).unwrap();
    let (loaded, decoded) = load_from_bytes_measured(&bytes, &LoadOptions::default()).unwrap();
    assert_eq!(loaded, doc);
    assert_eq!(decoded, 123, "shared native blobs are decoded and charged once");
    assert!(load_from_bytes_measured(&bytes, &LoadOptions { max_total_bytes: 122, ..Default::default() }).is_err());
}

#[test]
fn gradient_overlay_interpolation_roundtrips_in_native_documents() {
    use photocraft_doc::effects::GradientInterpolation;
    use photocraft_doc::{Color, Effect, FxCommon, Gradient};

    for mode in MODES {
        for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
            let mut doc = rich_doc(mode, depth);
            let source = GradientInterpolation {
                method: "smooth".into(),
                smoothness: 73.0,
                stops: vec![(0.0, Color::rgb(0.123456, 0.4, 0.9)), (0.4, Color::WHITE), (1.0, Color::BLACK)],
            };
            let effect = Effect::GradientOverlay {
                common: FxCommon::new(photocraft_doc::BlendMode::Normal, 1.0),
                gradient: Gradient::default(),
                dither: true,
                interpolation: Some(source),
            }
            .in_mode(mode);
            doc.layers.first_mut().unwrap().effects.items.push(effect);
            let bytes = save_to_bytes(&doc, &SaveOptions::default()).unwrap();
            assert_eq!(load_from_bytes(&bytes).unwrap(), doc);
        }
    }
}

/// #8351: a Gradient Map layer keeps Photoshop's own gradient (stops, midpoints, Smoothness,
/// method) through a native save, and a file without one still loads.
#[test]
fn gradient_map_source_roundtrips_in_native_documents() {
    use photocraft_doc::Adjustment;
    use photocraft_doc::adjust::GradientSource;
    let mut doc = rich_doc(ColorMode::Rgb, SampleType::U8);
    let source =
        GradientSource { stops: vec![(0.0, [0.1, 0.2, 0.6]), (1.0, [1.0, 0.8, 0.2])], midpoints: vec![0.3], smoothness: 0.8, method: "perceptual".into() };
    let map = Adjustment::GradientMap {
        stops: vec![(0.0, [0.1, 0.2, 0.6]), (0.5, [0.7, 0.4, 0.4]), (1.0, [1.0, 0.8, 0.2])],
        reverse: false,
        dither: true,
        source: Some(source),
    };
    doc.insert_above(None, Layer::new("map", LayerContent::Adjustment(map)));
    let bytes = save_to_bytes(&doc, &SaveOptions::default()).unwrap();
    assert_eq!(load_from_bytes(&bytes).unwrap(), doc);
}

#[test]
fn old_effect_snapshots_need_no_interpolation_record() {
    let effect = photocraft_doc::Effect::GradientOverlay {
        common: photocraft_doc::FxCommon::new(photocraft_doc::BlendMode::Normal, 1.0),
        gradient: photocraft_doc::Gradient::default(),
        dither: false,
        interpolation: None,
    };
    let json = serde_json::to_value(&effect).unwrap();
    assert!(json["GradientOverlay"].get("interpolation").is_none());
    assert_eq!(serde_json::from_value::<photocraft_doc::Effect>(json).unwrap(), effect);
}

#[test]
fn custom_shape_stroke_roundtrip_keeps_every_style_field() {
    for mode in MODES {
        for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
            let mut d = rich_doc(mode, depth);
            let stroke = photocraft_doc::ShapeStroke {
                width: 5.5,
                opacity: 0.35,
                align: photocraft_doc::StrokeAlign::Outside,
                cap: photocraft_doc::LineCap::Square,
                join: photocraft_doc::LineJoin::Bevel,
                miter_limit: 2.0,
                dashes: vec![0.0, 2.0, 4.0, 1.0, 1.5, 3.0],
                dash_offset: -0.5,
                ..Default::default()
            };
            d.layers.push(Layer::new(
                "Custom stroke",
                LayerContent::Shape(photocraft_doc::ShapeLayer {
                    path: photocraft_doc::Path::new(vec![photocraft_doc::Subpath::polygon(&[(10.0, 10.0), (40.0, 10.0), (25.0, 40.0)])]),
                    stroke: Some(stroke),
                    ..Default::default()
                }),
            ));
            let bytes = save_to_bytes(&d, &SaveOptions::default()).unwrap();
            assert_eq!(load_from_bytes(&bytes).unwrap(), d);
        }
    }
}

fn check_zip(mode: ColorMode, depth: SampleType) {
    let doc = rich_doc(mode, depth);
    let bytes = save_to_bytes(&doc, &SaveOptions::default()).unwrap();
    assert!(is_pcraft(&bytes));
    let back = load_from_bytes(&bytes).unwrap();
    assert_eq!(back, doc, "{mode:?} {depth:?}");
}

macro_rules! zip_cases {
    ($($name:ident: $m:ident, $d:ident;)*) => {$(
        #[test]
        fn $name() {
            check_zip(ColorMode::$m, SampleType::$d);
        }
    )*};
}

zip_cases! {
    rgb_u8: Rgb, U8; rgb_u16: Rgb, U16; rgb_f32: Rgb, F32;
    gray_u8: Grayscale, U8; gray_u16: Grayscale, U16; gray_f32: Grayscale, F32;
    cmyk_u8: Cmyk, U8; cmyk_u16: Cmyk, U16; cmyk_f32: Cmyk, F32;
    lab_u8: Lab, U8; lab_u16: Lab, U16; lab_f32: Lab, F32;
}

#[test]
fn directory_bundle_roundtrip_all_modes() {
    for mode in MODES {
        for depth in SampleType::ALL {
            let doc = rich_doc(mode, depth);
            let dir = temp_dir("dir");
            PcraftWriter::new().save_dir(&doc, &dir, &SaveOptions::default()).unwrap();
            assert!(dir.join("manifest.json").is_file());
            assert_eq!(load_path(&dir).unwrap(), doc, "{mode:?} {depth:?}");
            std::fs::remove_dir_all(dir).unwrap();
        }
    }
}

#[test]
fn save_path_zip_file() {
    let doc = rich_doc(ColorMode::Rgb, SampleType::U8);
    let dir = temp_dir("file");
    let path = dir.join("a.pcraft");
    PcraftWriter::new().save_path(&doc, &path, &SaveOptions::default()).unwrap();
    assert!(path.is_file());
    assert_eq!(load_path(&path).unwrap(), doc);
    std::fs::remove_dir_all(dir).unwrap();
}

/// The public [`ByteSource`] entry loads a saved directory like `load_path` does.
#[test]
fn load_bytes_source_matches_the_path_load() {
    let doc = rich_doc(ColorMode::Rgb, SampleType::U8);
    let dir = temp_dir("bytes-source");
    let bundle = dir.join("a.pcraft");
    PcraftWriter::new().save_dir(&doc, &bundle, &SaveOptions::default()).unwrap();
    let loaded = load_bytes_source(&FsDir { root: bundle.clone() }, &LoadOptions::default()).unwrap();
    assert_eq!(loaded, doc);
    assert!(load_bytes_source(&FsDir { root: bundle.join("no-such-dir") }, &LoadOptions::default()).is_err(), "a missing bundle errors");
    std::fs::remove_dir_all(dir).unwrap();
}

/// `read_entry` bounds the read itself, not the stat hint: a stream that ignores the hint
/// and yields more than `max` errors instead of filling memory.
#[test]
fn read_entry_bounds_the_read_not_the_hint() {
    struct Endless;
    impl std::io::Read for Endless {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            buf.fill(b'x');
            Ok(buf.len())
        }
    }
    let mut endless = Endless;
    let error = read_entry(&mut endless, 0, 100, "manifest.json").unwrap_err().to_string();
    assert!(error.contains("exceeds"), "{error}");
    let mut small = &b"hi"[..];
    assert_eq!(read_entry(&mut small, 2, 100, "manifest.json").unwrap(), b"hi");
}

/// Reads the on-disk bundle shape for [`load_bytes_source_matches_the_path_load`].
struct FsDir {
    root: std::path::PathBuf,
}

impl ByteSource for FsDir {
    fn get(&self, path: &str, max: usize) -> Result<Vec<u8>> {
        let p = self.root.join(path);
        let metadata = std::fs::metadata(&p).map_err(|_| FormatError::Corrupt(format!("missing `{path}`")))?;
        if !metadata.is_file() {
            return Err(FormatError::Corrupt(format!("`{path}` is not a regular file")));
        }
        let mut file = std::fs::File::open(&p).map_err(|_| FormatError::Corrupt(format!("missing `{path}`")))?;
        read_entry(&mut file, metadata.len(), max, path)
    }
}

/// Deepest group nesting in `layers` (0 when there are no groups).
fn group_depth(layers: &[Layer]) -> usize {
    layers.iter().map(|l| if let photocraft_doc::LayerContent::Group(g) = &l.content { 1 + group_depth(&g.children) } else { 0 }).max().unwrap_or(0)
}

/// [`rich_doc`] with its layers wrapped in groups until they are nested `levels` deep.
fn nested_doc(levels: usize, depth: SampleType) -> Document {
    let mut doc = rich_doc(ColorMode::Rgb, depth);
    let mut layers = std::mem::take(&mut doc.layers);
    for i in group_depth(&layers)..levels {
        layers = vec![Layer::group(format!("Level {i}"), layers)];
    }
    doc.layers = layers;
    assert_eq!(group_depth(&doc.layers), levels);
    doc
}

/// serde_json's default limit (128 levels) used to make bundles with 40+ nested groups unreadable.
/// Runs on a 1 MiB stack, the smallest main-thread stack we ship on (Windows, wasm).
#[test]
fn deeply_nested_groups_roundtrip() {
    let run = || {
        for (levels, depth) in [(40, SampleType::U8), (MAX_GROUP_DEPTH, SampleType::U16), (MAX_GROUP_DEPTH, SampleType::F32)] {
            let doc = nested_doc(levels, depth);
            let back = load_from_bytes(&save_to_bytes(&doc, &SaveOptions::default()).unwrap()).unwrap();
            assert_eq!(back, doc, "{levels} levels at {depth:?}");
        }
    };
    std::thread::Builder::new().stack_size(1 << 20).spawn(run).unwrap().join().unwrap();
}

/// A save never writes a bundle the loader would reject for its nesting.
#[test]
fn nesting_beyond_the_load_limit_is_refused_at_save() {
    let e = save_to_bytes(&nested_doc(MAX_GROUP_DEPTH + 1, SampleType::U8), &SaveOptions::default()).unwrap_err();
    assert!(matches!(e, FormatError::LimitExceeded(_)), "{e}");
}

/// craft-pcraft-format's default manifest nesting limit is computed from its own copy of the
/// group depth, so the two must stay equal.
#[test]
fn group_depth_matches_craft_pcraft_format() {
    assert_eq!(MAX_GROUP_DEPTH, craft_pcraft_format::MAX_GROUP_DEPTH);
}

#[test]
fn empty_document_roundtrips() {
    let doc = Document::new("empty", photocraft_doc::Size::new(1, 1), ColorMode::Rgb, SampleType::U8);
    assert_eq!(load_from_bytes(&save_to_bytes(&doc, &SaveOptions::default()).unwrap()).unwrap(), doc);
}

#[test]
fn exhausted_document_ids_and_maximal_text_runs_roundtrip() {
    let mut doc = Document::new("exhausted", photocraft_doc::Size::new(2, 1), ColorMode::Rgb, SampleType::U8);
    doc.slices.list.push(Slice { id: u32::MAX, ..Default::default() });
    doc.text_styles.character.push(CharacterStyleDef { id: u32::MAX, ..Default::default() });
    doc.text_styles.paragraph.push(ParagraphStyleDef { id: u32::MAX, ..Default::default() });
    doc.layers.push(Layer::new(
        "Text",
        LayerContent::Text(TextLayer {
            text: "ab".into(),
            runs: vec![TextRun { len: 1, style: CharStyle::default() }, TextRun { len: usize::MAX, style: CharStyle { size_pt: 24.0, ..Default::default() } }],
            paragraphs: vec![
                photocraft_doc::text::ParagraphRun { len: 1, style: ParagraphStyle::default() },
                photocraft_doc::text::ParagraphRun { len: usize::MAX, style: ParagraphStyle::default() },
            ],
            ..Default::default()
        }),
    ));

    let bytes = save_to_bytes(&doc, &SaveOptions::default()).unwrap();
    let loaded = load_from_bytes(&bytes).unwrap();
    assert_eq!(loaded.slices.next_id(), None);
    assert_eq!(loaded.text_styles.next_char_id(), None);
    assert_eq!(loaded.text_styles.next_para_id(), None);
    let LayerContent::Text(text) = &loaded.layers.last().unwrap().content else { panic!("text layer must roundtrip") };
    assert_eq!(text.char_runs().iter().map(|r| r.len).sum::<usize>(), text.text.len());
    assert_eq!(text.char_runs()[1].len, 1);
    assert_eq!(text.char_runs()[1].style.size_pt, 24.0);
    assert_eq!(text.paragraph_runs().iter().map(|r| r.len).sum::<usize>(), text.text.len());
}

#[test]
fn text_run_baseline_roundtrips() {
    use photocraft_doc::text::FontBaseline;
    let mut doc = Document::new("scripts", photocraft_doc::Size::new(2, 1), ColorMode::Rgb, SampleType::U8);
    doc.layers.push(Layer::new(
        "Text",
        LayerContent::Text(TextLayer {
            text: "H2O2".into(),
            runs: vec![
                TextRun { len: 1, style: CharStyle::default() },
                TextRun { len: 1, style: CharStyle { baseline: FontBaseline::Subscript, ..Default::default() } },
                TextRun { len: 1, style: CharStyle::default() },
                TextRun { len: 1, style: CharStyle { baseline: FontBaseline::Superscript, ..Default::default() } },
            ],
            ..Default::default()
        }),
    ));
    let loaded = load_from_bytes(&save_to_bytes(&doc, &SaveOptions::default()).unwrap()).unwrap();
    let LayerContent::Text(text) = &loaded.layers.last().unwrap().content else { panic!("text layer must roundtrip") };
    let baselines: Vec<FontBaseline> = text.char_runs().iter().map(|r| r.style.baseline).collect();
    assert_eq!(baselines, [FontBaseline::Normal, FontBaseline::Subscript, FontBaseline::Normal, FontBaseline::Superscript]);
}

#[test]
fn zip_incremental_only_new_tiles() {
    let mut doc = rich_doc(ColorMode::Rgb, SampleType::U16);
    let mut w = PcraftWriter::new();
    let (first, s1) = w.save_zip(&doc, &SaveOptions::default()).unwrap();
    assert!(s1.tiles_total > 5);
    assert_eq!(s1.tiles_written, s1.tiles_total);
    let (cached, s2) = w.save_zip(&doc, &SaveOptions::default()).unwrap();
    assert_eq!(first, cached);
    assert_eq!(s2.tiles_written, 0);
    assert_eq!(s2.tiles_reused, s2.tiles_total);
    assert_eq!(s2.blobs_written, 0);
    // Touch one pixel of one layer: exactly one tile is new.
    let id = doc.layers[1].id;
    doc.layer_mut(id).unwrap().surface_mut().unwrap().write_pixel(3, 3, &[0.5, 0.5, 0.5, 1.0]);
    let (bytes, s3) = w.save_zip(&doc, &SaveOptions::default()).unwrap();
    assert_eq!(s3.tiles_written, 1);
    assert_eq!(load_from_bytes(&bytes).unwrap(), doc);
    assert_eq!(bytes, PcraftWriter::new().save_zip(&doc, &SaveOptions::default()).unwrap().0);
}

#[test]
fn directory_incremental_and_gc() {
    let mut doc = rich_doc(ColorMode::Rgb, SampleType::U8);
    let dir = temp_dir("gc");
    let mut w = PcraftWriter::new();
    let s1 = w.save_dir(&doc, &dir, &SaveOptions::default()).unwrap();
    assert_eq!(s1.tiles_written, s1.tiles_total);
    let s2_same_writer = w.save_dir(&doc, &dir, &SaveOptions::default()).unwrap();
    assert_eq!(s2_same_writer.tiles_written, 0);
    assert_eq!(s2_same_writer.tiles_reused, s2_same_writer.tiles_total);
    // A fresh writer still skips files already on disk.
    let s2 = PcraftWriter::new().save_dir(&doc, &dir, &SaveOptions::default()).unwrap();
    assert_eq!(s2.tiles_written, 0);
    assert_eq!(s2.objects_removed, 0);
    // Delete a layer: its unique tiles get garbage-collected.
    let paint = doc.layers[1].id;
    doc.remove(paint);
    let s3 = w.save_dir(&doc, &dir, &SaveOptions::default()).unwrap();
    assert!(s3.objects_removed > 0, "{s3:?}");
    assert_eq!(load_path(&dir).unwrap(), doc);
    let files = std::fs::read_dir(dir.join("tiles")).unwrap().count();
    assert_eq!(files, s3.tiles_total);
    std::fs::remove_dir_all(dir).unwrap();
}

#[cfg(unix)]
#[test]
fn directory_save_rejects_object_directory_symlink_without_deleting_external_objects() {
    use std::os::unix::fs::symlink;

    let doc = rich_doc(ColorMode::Rgb, SampleType::U8);
    let dir = temp_dir("symlink-object-dir");
    let outside = temp_dir("symlink-object-dir-outside");
    let external_object = outside.join("unrelated.zst");
    std::fs::write(&external_object, b"keep this object").unwrap();
    symlink(&outside, dir.join("tiles")).unwrap();

    assert!(PcraftWriter::new().save_dir(&doc, &dir, &SaveOptions::default()).is_err());
    assert_eq!(std::fs::read(&external_object).unwrap(), b"keep this object");
    assert!(!dir.join("manifest.json").exists());

    std::fs::remove_file(dir.join("tiles")).unwrap();
    std::fs::remove_dir_all(dir).unwrap();
    std::fs::remove_dir_all(outside).unwrap();
}

#[cfg(unix)]
#[test]
fn directory_save_rejects_object_symlink_without_overwriting_target() {
    use std::os::unix::fs::symlink;

    let doc = rich_doc(ColorMode::Rgb, SampleType::U8);
    let dir = temp_dir("symlink-object");
    let outside = temp_dir("symlink-object-outside");
    let mut writer = PcraftWriter::new();
    writer.save_dir(&doc, &dir, &SaveOptions::default()).unwrap();
    let object = std::fs::read_dir(dir.join("tiles")).unwrap().next().unwrap().unwrap().path();
    std::fs::remove_file(&object).unwrap();
    let external_object = outside.join("external.zst");
    std::fs::write(&external_object, b"do not overwrite").unwrap();
    symlink(&external_object, &object).unwrap();

    assert!(writer.save_dir(&doc, &dir, &SaveOptions::default()).is_err());
    assert_eq!(std::fs::read(&external_object).unwrap(), b"do not overwrite");
    assert!(std::fs::symlink_metadata(&object).unwrap().file_type().is_symlink());

    std::fs::remove_dir_all(dir).unwrap();
    std::fs::remove_dir_all(outside).unwrap();
}

#[cfg(unix)]
#[test]
fn directory_save_accepts_chosen_root_symlink_but_rejects_preview_symlink() {
    use std::os::unix::fs::symlink;

    let doc = rich_doc(ColorMode::Rgb, SampleType::U8);
    let outside = temp_dir("symlink-preview-outside");
    let root_link_parent = temp_dir("symlink-root-parent");
    let root_link = root_link_parent.join("linked.pcraft");
    symlink(&outside, &root_link).unwrap();
    PcraftWriter::new().save_dir(&doc, &root_link, &SaveOptions::default()).unwrap();
    assert_eq!(load_path(&root_link).unwrap(), doc);
    let trailing_separator = std::path::PathBuf::from(format!("{}/", root_link.display()));
    PcraftWriter::new().save_dir(&doc, &trailing_separator, &SaveOptions::default()).unwrap();
    assert!(outside.join("manifest.json").is_file());

    let dir = temp_dir("symlink-preview");
    let external_preview = outside.join("preview.png");
    std::fs::write(&external_preview, b"keep preview").unwrap();
    std::fs::create_dir(dir.join("composite")).unwrap();
    symlink(&external_preview, dir.join("composite/preview.png")).unwrap();
    let opts = SaveOptions { composite: Some(Rgba8Image::new(2, 2)), ..Default::default() };
    assert!(PcraftWriter::new().save_dir(&doc, &dir, &opts).is_err());
    assert_eq!(std::fs::read(&external_preview).unwrap(), b"keep preview");
    assert!(!dir.join("manifest.json").exists());

    std::fs::remove_file(root_link).unwrap();
    std::fs::remove_dir_all(root_link_parent).unwrap();
    std::fs::remove_dir_all(dir).unwrap();
    std::fs::remove_dir_all(outside).unwrap();
}

#[cfg(unix)]
#[test]
fn directory_save_rejects_manifest_symlink_without_overwriting_target() {
    use std::os::unix::fs::symlink;

    let doc = rich_doc(ColorMode::Rgb, SampleType::U8);
    let dir = temp_dir("symlink-manifest");
    let outside = temp_dir("symlink-manifest-outside");
    let external_manifest = outside.join("external.json");
    std::fs::write(&external_manifest, b"keep manifest").unwrap();
    symlink(&external_manifest, dir.join("manifest.json")).unwrap();

    assert!(PcraftWriter::new().save_dir(&doc, &dir, &SaveOptions::default()).is_err());
    assert_eq!(std::fs::read(&external_manifest).unwrap(), b"keep manifest");
    assert_eq!(std::fs::read_dir(dir.join("tiles")).unwrap().count(), 0);

    std::fs::remove_dir_all(dir).unwrap();
    std::fs::remove_dir_all(outside).unwrap();
}

#[test]
fn identical_tiles_are_stored_once() {
    let mut doc = Document::new("d", photocraft_doc::Size::new(10, 10), ColorMode::Rgb, SampleType::U8);
    let mut a = Layer::raster("a", doc.pixel_format());
    scribble(a.surface_mut().unwrap(), 5, false);
    let b = a.duplicate();
    doc.layers.push(a);
    doc.layers.push(b);
    let (_, s) = PcraftWriter::new().save_zip(&doc, &SaveOptions::default()).unwrap();
    let per_layer = doc.layers[0].surface().unwrap().tile_count();
    assert_eq!(s.tiles_total, per_layer);
}

/// A solid fill saves one tile however many coordinates share it; reopening must decode it once,
/// charge the byte budget once and keep the sharing (#3315). The budget here holds one tile,
/// standing in for a canvas too large to expand tile by tile.
#[test]
fn solid_fill_reopens_with_one_shared_tile() {
    for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
        let size = photocraft_doc::Size::new(2048, 2048);
        let mut doc = Document::new("d", size, ColorMode::Rgb, depth);
        let mut l = Layer::raster("solid", doc.pixel_format());
        l.surface_mut().unwrap().fill_rect(photocraft_geom::Rect::from_size(size), &[1.0, 0.5, 0.25, 1.0]);
        doc.layers.push(l);
        let bytes = save_to_bytes(&doc, &SaveOptions::default()).unwrap();
        let one_tile = (256 * 256 * doc.pixel_format().bytes_per_pixel()) as u64;
        let back = load_from_bytes_with(&bytes, &LoadOptions { max_total_bytes: one_tile, ..Default::default() }).unwrap();
        assert_eq!(back, doc, "{depth:?}");
        let s = back.layers[0].surface().unwrap();
        assert_eq!(s.tile_count(), 64, "{depth:?}");
        let first = s.tiles().next().unwrap().1;
        assert!(s.tiles().all(|(_, t)| std::sync::Arc::ptr_eq(t, first)), "{depth:?}: tiles not shared after reopening");
    }
}

#[test]
fn ids_preserved_and_counter_advanced() {
    let doc = rich_doc(ColorMode::Rgb, SampleType::U8);
    let bytes = save_to_bytes(&doc, &SaveOptions::default()).unwrap();
    // Simulate a file written with much larger ids.
    let mut m = read_manifest(&bytes).unwrap();
    m.document.layers[0].id = LayerId::fresh().0 + 5000;
    let big = m.document.layers[0].id;
    let text = serde_json::to_vec(&m).unwrap();
    let mut z = zip::ZipWriter::new();
    z.add("manifest.json", &text).unwrap();
    let r = zip::ZipReader::new(&bytes).unwrap();
    for e in r.entries.iter().filter(|e| e.name != "manifest.json") {
        z.add(&e.name, &r.read(e, usize::MAX).unwrap()).unwrap();
    }
    let back = load_from_bytes(&z.finish().unwrap()).unwrap();
    assert_eq!(back.layers[0].id.0, big);
    assert!(LayerId::fresh().0 > big, "fresh ids must not collide with loaded ids");
}

#[test]
fn fresh_ids_option() {
    let doc = rich_doc(ColorMode::Rgb, SampleType::U8);
    let bytes = save_to_bytes(&doc, &SaveOptions::default()).unwrap();
    let back = load_from_bytes_with(&bytes, &LoadOptions { preserve_ids: false, ..Default::default() }).unwrap();
    assert_ne!(back.id, doc.id);
    assert_ne!(back.layers[0].id, doc.layers[0].id);
    assert_eq!(back.layers.len(), doc.layers.len());
    assert_eq!(back.layers[1].surface(), doc.layers[1].surface());
}

#[test]
fn previews_embedded_and_readable() {
    let doc = rich_doc(ColorMode::Rgb, SampleType::U8);
    let mut thumb = Rgba8Image::new(4, 3);
    thumb.pixels.iter_mut().enumerate().for_each(|(i, p)| *p = i as u8);
    let opts = SaveOptions { thumbnail: Some(thumb.clone()), composite: Some(thumb.clone()), ..Default::default() };
    let bytes = save_to_bytes(&doc, &opts).unwrap();
    let png = read_thumbnail(&bytes).unwrap().unwrap();
    let img = photocraft_codecs::decode(&png).unwrap();
    assert_eq!(img.dimensions(), (4, 3));
    assert_eq!(img.to_rgba8(), thumb.pixels);
    let m = read_manifest(&bytes).unwrap();
    assert_eq!(m.composite.as_deref(), Some("composite/preview.png"));
    assert_eq!(m.format_version, FORMAT_VERSION);
    assert!(read_thumbnail(&save_to_bytes(&doc, &SaveOptions::default()).unwrap()).unwrap().is_none());
}

/// The full-size composite is an optional manifest member: a bundle without it (every file saved
/// before it was written) opens as before, at the same format version.
#[test]
fn bundles_without_the_full_composite_still_open() {
    let doc = rich_doc(ColorMode::Rgb, SampleType::U8);
    let full = Rgba8Image::new(doc.size.width, doc.size.height);
    let opts = SaveOptions { thumbnail: Some(Rgba8Image::new(2, 2)), composite: Some(Rgba8Image::new(4, 3)), full_composite: Some(full), ..Default::default() };
    let bytes = save_to_bytes(&doc, &opts).unwrap();
    assert!(read_full_composite(&bytes).unwrap().is_some());
    assert_eq!(read_manifest(&bytes).unwrap().format_version, 1);
    // Rewrite the bundle as an older writer left it: no member in the manifest, no entry.
    let r = zip::ZipReader::new(&bytes).unwrap();
    let mut manifest: serde_json::Value = serde_json::from_slice(&r.read_by_name("manifest.json", usize::MAX).unwrap()).unwrap();
    assert!(manifest.as_object_mut().unwrap().remove("full_composite").is_some());
    let mut z = zip::ZipWriter::new();
    z.add("manifest.json", &serde_json::to_vec_pretty(&manifest).unwrap()).unwrap();
    for e in r.entries.iter().filter(|e| e.name != "manifest.json" && e.name != "composite/full.png") {
        z.add(&e.name, &r.read(e, usize::MAX).unwrap()).unwrap();
    }
    let older = z.finish().unwrap();
    assert_eq!(read_full_composite(&older).unwrap(), None);
    // A member that names any other entry is refused, and the document still opens.
    manifest["full_composite"] = serde_json::Value::from("thumb.png");
    let mut z = zip::ZipWriter::new();
    z.add("manifest.json", &serde_json::to_vec_pretty(&manifest).unwrap()).unwrap();
    for e in r.entries.iter().filter(|e| e.name != "manifest.json") {
        z.add(&e.name, &r.read(e, usize::MAX).unwrap()).unwrap();
    }
    let other = z.finish().unwrap();
    assert!(read_full_composite(&other).is_err());
    assert_eq!(load_from_bytes(&other).unwrap(), doc);
    assert_eq!(load_from_bytes(&older).unwrap(), load_from_bytes(&bytes).unwrap());
    assert_eq!(load_from_bytes(&older).unwrap(), doc);
    // A save without it writes no member at all, as older writers did.
    let plain = save_to_bytes(&doc, &SaveOptions::default()).unwrap();
    let text = zip::ZipReader::new(&plain).unwrap().read_by_name("manifest.json", usize::MAX).unwrap();
    assert!(!String::from_utf8(text).unwrap().contains("full_composite"));
}

#[test]
fn manifest_is_human_readable_json() {
    let doc = rich_doc(ColorMode::Rgb, SampleType::U8);
    let bytes = save_to_bytes(&doc, &SaveOptions::default()).unwrap();
    let r = zip::ZipReader::new(&bytes).unwrap();
    assert_eq!(r.entries[0].name, "manifest.json");
    let text = String::from_utf8(r.read_by_name("manifest.json", usize::MAX).unwrap()).unwrap();
    assert!(text.contains("\"format_version\": 1"));
    assert!(text.contains("\"Hue/Sat\""));
    assert!(text.contains("filter.blur.gaussian"));
    assert!(r.entries.iter().any(|e| e.name.starts_with("tiles/") && e.name.ends_with(".zst")));
    assert!(r.entries.iter().any(|e| e.name.starts_with("blobs/")));
}

#[test]
fn save_is_deterministic() {
    let doc = rich_doc(ColorMode::Cmyk, SampleType::U16);
    let a = save_to_bytes(&doc, &SaveOptions::default()).unwrap();
    let b = save_to_bytes(&doc, &SaveOptions::default()).unwrap();
    assert_eq!(a, b);
}

/// A bundle that is read and saved again gives the same bytes, previews included.
#[test]
fn save_load_save_gives_the_same_bytes() {
    let mut preview = Rgba8Image::new(4, 3);
    preview.pixels.iter_mut().enumerate().for_each(|(i, p)| *p = i as u8);
    for opts in [SaveOptions::default(), SaveOptions { thumbnail: Some(preview.clone()), composite: Some(preview.clone()), ..Default::default() }] {
        for mode in MODES {
            for depth in SampleType::ALL {
                let first = save_to_bytes(&rich_doc(mode, depth), &opts).unwrap();
                let again = save_to_bytes(&load_from_bytes(&first).unwrap(), &opts).unwrap();
                assert!(first == again, "{mode:?} {depth:?}: the second save differs");
            }
        }
    }
}

#[test]
fn not_pcraft() {
    assert!(!is_pcraft(b""));
    assert!(!is_pcraft(b"8BPS...."));
    let mut z = zip::ZipWriter::new();
    z.add("other.txt", b"x").unwrap();
    assert!(!is_pcraft(&z.finish().unwrap()));
}

#[test]
fn vector_fields_roundtrip_and_default_when_absent() {
    let doc = rich_doc(ColorMode::Rgb, SampleType::U8);
    let back = load_from_bytes(&save_to_bytes(&doc, &SaveOptions::default()).unwrap()).unwrap();
    assert_eq!(back.paths, doc.paths);
    assert_eq!(back.work_path, doc.work_path);
    assert_eq!(back.clipping_path, doc.clipping_path);
    let vm = |d: &Document| d.walk().into_iter().find_map(|(_, _, l)| l.vector_mask.clone());
    assert!(vm(&back).is_some());
    assert_eq!(vm(&back), vm(&doc));
    // Manifests written before the vector fields existed still load (serde defaults).
    let m = read_manifest(&save_to_bytes(&doc, &SaveOptions::default()).unwrap()).unwrap();
    let mut v = serde_json::to_value(&m.document).unwrap();
    for k in ["paths", "work_path", "clipping_path"] {
        v.as_object_mut().unwrap().remove(k);
    }
    fn strip(layers: &mut serde_json::Value) {
        for l in layers.as_array_mut().unwrap() {
            l.as_object_mut().unwrap().remove("vector_mask");
            let c = l["content"].as_object_mut().unwrap();
            // A shape's density and feather (#5846) came later still.
            if c.get("kind").and_then(|k| k.as_str()) == Some("shape") {
                c.remove("density");
                c.remove("feather");
            }
            for k in ["path", "stroke", "live"] {
                c.remove(k);
            }
            if let Some(ch) = c.get_mut("children") {
                strip(ch);
            }
        }
    }
    strip(&mut v["layers"]);
    let old: photocraft_format::manifest::DocM = serde_json::from_value(v).unwrap();
    assert!(old.paths.is_empty() && old.work_path.is_none());
    assert!(old.layers.iter().all(|l| l.vector_mask.is_none()));
    let shapes: Vec<_> = old
        .layers
        .iter()
        .filter_map(|l| match &l.content {
            photocraft_format::manifest::ContentM::Shape { density, feather, .. } => Some((*density, *feather)),
            _ => None,
        })
        .collect();
    assert!(!shapes.is_empty() && shapes.iter().all(|s| *s == (1.0, 0.0)), "{shapes:?}");
}

/// Type kerning (#206): the auto mode and manual kerning survive .pcraft; manifests written
/// before `kern` existed load with no manual kerning.
#[test]
fn text_kerning_roundtrips_and_defaults() {
    use photocraft_doc::LayerContent;
    use photocraft_doc::text::Kerning;
    let doc = rich_doc(ColorMode::Rgb, SampleType::U8);
    let bytes = save_to_bytes(&doc, &SaveOptions::default()).unwrap();
    let back = load_from_bytes(&bytes).unwrap();
    let runs = |d: &Document| {
        d.walk()
            .into_iter()
            .find_map(|(_, _, l)| match &l.content {
                LayerContent::Text(t) => Some(t.runs.iter().map(|r| (r.len, r.style.kerning, r.style.kern)).collect::<Vec<_>>()),
                _ => None,
            })
            .unwrap()
    };
    assert_eq!(runs(&back), vec![(1, Kerning::Off, 120.0), (5, Kerning::Metrics, 0.0), (9, Kerning::Optical, 0.0)]);
    assert_eq!(runs(&back), runs(&doc));
    let m = read_manifest(&bytes).unwrap();
    let mut v = serde_json::to_value(&m.document).unwrap();
    fn strip(v: &mut serde_json::Value) {
        match v {
            serde_json::Value::Object(o) => {
                o.remove("kern");
                o.values_mut().for_each(strip);
            }
            serde_json::Value::Array(a) => a.iter_mut().for_each(strip),
            _ => {}
        }
    }
    strip(&mut v);
    assert!(!v.to_string().contains("\"kern\""));
    let old: photocraft_format::manifest::DocM = serde_json::from_value(v).unwrap();
    let s = serde_json::to_string(&old).unwrap();
    assert!(s.contains("\"kern\":0.0"), "kern defaults to 0");
}

/// Type on a path (#218): the path, start and end offsets, and flip survive .pcraft.
#[test]
fn path_text_shape_roundtrips() {
    use photocraft_doc::text::TextShape;
    use photocraft_doc::vector::Subpath;
    let mut doc = rich_doc(ColorMode::Rgb, SampleType::U8);
    let path = Subpath::polyline(&[(0.0, 0.0), (50.0, 20.0), (120.0, 5.5)]);
    let ids: Vec<LayerId> = doc.walk().into_iter().filter(|(_, _, l)| matches!(l.content, LayerContent::Text(_))).map(|(_, _, l)| l.id).collect();
    for id in ids {
        if let Some(LayerContent::Text(t)) = doc.layer_mut(id).map(|l| &mut l.content) {
            t.shape = TextShape::Path { path: path.clone(), start: 12.5, end: Some(83.0), flip: true };
        }
    }
    let back = load_from_bytes(&save_to_bytes(&doc, &SaveOptions::default()).unwrap()).unwrap();
    let shape = back.walk().into_iter().find_map(|(_, _, l)| match &l.content {
        LayerContent::Text(t) => Some(t.shape.clone()),
        _ => None,
    });
    assert_eq!(shape, Some(TextShape::Path { path, start: 12.5, end: Some(83.0), flip: true }));
}

/// Area type (#218): the whole path, ops and fill rule included, survives .pcraft.
#[test]
fn area_text_shape_roundtrips() {
    use photocraft_doc::text::TextShape;
    use photocraft_doc::vector::{FillRule, Path, PathOp, Subpath};
    let mut doc = rich_doc(ColorMode::Rgb, SampleType::U8);
    let mut path = Path::new(vec![
        Subpath::polygon(&[(0.0, 0.0), (200.0, 0.0), (200.0, 120.0), (0.0, 120.0)]),
        Subpath::polygon(&[(80.0, 40.0), (120.0, 40.0), (120.0, 80.0)]).with_op(PathOp::Subtract),
    ]);
    path.fill_rule = FillRule::EvenOdd;
    let ids: Vec<LayerId> = doc.walk().into_iter().filter(|(_, _, l)| matches!(l.content, LayerContent::Text(_))).map(|(_, _, l)| l.id).collect();
    assert!(!ids.is_empty());
    for id in ids {
        if let Some(LayerContent::Text(t)) = doc.layer_mut(id).map(|l| &mut l.content) {
            t.shape = TextShape::Area { path: path.clone() };
        }
    }
    let back = load_from_bytes(&save_to_bytes(&doc, &SaveOptions::default()).unwrap()).unwrap();
    let shape = back.walk().into_iter().find_map(|(_, _, l)| match &l.content {
        LayerContent::Text(t) => Some(t.shape.clone()),
        _ => None,
    });
    assert_eq!(shape, Some(TextShape::Area { path }));
}

#[test]
fn channel_restrictions_and_bevel_elements_roundtrip() {
    use photocraft_doc::{Bevel, BevelContour, BevelTexture, Contour, Effect};
    let mut doc = rich_doc(ColorMode::Rgb, SampleType::U8);
    doc.layers[0].excluded_channels = 0b101;
    let mut b = Bevel {
        enabled: true,
        style: photocraft_doc::BevelStyle::Emboss,
        technique: photocraft_doc::BevelTechnique::ChiselHard,
        depth: 1.2,
        up: false,
        size: 9.0,
        soften: 1.0,
        angle: 30.0,
        altitude: 40.0,
        use_global_light: false,
        gloss_contour: Contour::Linear,
        gloss_anti_alias: false,
        highlight: photocraft_doc::FxCommon::new(photocraft_color::BlendMode::Screen, 0.7),
        highlight_color: photocraft_color::Color::WHITE,
        shadow: photocraft_doc::FxCommon::new(photocraft_color::BlendMode::Multiply, 0.6),
        shadow_color: photocraft_color::Color::BLACK,
        contour: None,
        texture: None,
    };
    b.contour = Some(BevelContour { contour: Contour::Linear, range: 0.3, anti_alias: true });
    b.texture = Some(BevelTexture { name: "p".into(), id: "i".into(), scale: 0.5, depth: -1.5, invert: true, link: false, phase: (1.0, 2.0) });
    doc.layers[0].effects.items.push(Effect::BevelEmboss(Bevel { ..b }));
    let back = load_from_bytes(&save_to_bytes(&doc, &SaveOptions::default()).unwrap()).unwrap();
    assert_eq!(back, doc);
    assert_eq!(back.layers[0].excluded_channels, 0b101);
}

#[test]
fn blend_if_roundtrips() {
    use photocraft_doc::{BlendIf, BlendRange};
    let mut doc = rich_doc(ColorMode::Rgb, SampleType::U16);
    let mut bi = BlendIf::default();
    bi.set(0, [BlendRange { black: [20, 60], white: [255, 255] }, BlendRange::FULL]);
    bi.set(3, [BlendRange::FULL, BlendRange { black: [0, 0], white: [180, 220] }]);
    doc.layers[0].blend_if = bi.clone();
    let back = load_from_bytes(&save_to_bytes(&doc, &SaveOptions::default()).unwrap()).unwrap();
    assert_eq!(back, doc);
    assert_eq!(back.layers[0].blend_if, bi);
    // Layers without Blend If keep the default (the field is omitted from the manifest).
    assert!(back.layers.iter().skip(1).all(|l| l.blend_if.is_default()));
}

/// Advanced Blending (knockout, as-group switches, transparency shapes, masks hide effects)
/// survives .pcraft at every depth; manifests written before the field existed load with
/// Photoshop's defaults, and default layers don't write it.
#[test]
fn advanced_blending_roundtrips_and_defaults_when_absent() {
    use photocraft_doc::{AdvancedBlending, Knockout};
    for depth in SampleType::ALL {
        let mut doc = rich_doc(ColorMode::Rgb, depth);
        let a = AdvancedBlending {
            knockout: Knockout::Deep,
            blend_interior: true,
            blend_clipped: false,
            transparency_shapes: false,
            layer_mask_hides_effects: true,
            vector_mask_hides_effects: true,
        };
        doc.layers[0].advanced = a;
        if let Some(l) = doc.layers.get_mut(1) {
            l.advanced.knockout = Knockout::Shallow;
        }
        let bytes = save_to_bytes(&doc, &SaveOptions::default()).unwrap();
        let back = load_from_bytes(&bytes).unwrap();
        assert_eq!(back, doc, "{depth:?}");
        assert_eq!(back.layers[0].advanced, a);
        // Only the non-default layers carry the field.
        let m = read_manifest(&bytes).unwrap();
        let v = serde_json::to_value(&m.document).unwrap();
        let with = v["layers"].as_array().unwrap().iter().filter(|l| l.get("advanced").is_some()).count();
        assert_eq!(with, if doc.layers.len() > 1 { 2 } else { 1 });
        // An older manifest (no `advanced`) loads with the defaults.
        let mut old = v.clone();
        for l in old["layers"].as_array_mut().unwrap() {
            l.as_object_mut().unwrap().remove("advanced");
        }
        let old: photocraft_format::manifest::DocM = serde_json::from_value(old).unwrap();
        assert!(old.layers.iter().all(|l| l.advanced.is_default()));
    }
}

#[test]
fn video_layer_frames_survive_roundtrip() {
    use photocraft_doc::{Timeline, VideoData, VideoSource};
    use photocraft_geom::Rect;
    let mut doc = Document::new("Vid", photocraft_geom::Size { width: 8, height: 6 }, ColorMode::Rgb, SampleType::U8);
    doc.timeline = Some(Timeline::new(3, 24.0));
    let fmt = doc.pixel_format();
    let mut l = Layer::raster("Clip", fmt);
    let mut frames = Vec::new();
    for i in 0..3u8 {
        let mut s = photocraft_raster::Surface::new(fmt);
        let v = i as f32 / 2.0;
        s.fill_rect(Rect::new(0, 0, 8, 6), &photocraft_raster::from_rgba(&fmt, [v, 0.0, 1.0 - v, 1.0]));
        frames.push(s);
    }
    let mut vd = VideoData::new(frames, 24.0);
    vd.original_frames = Some(vd.frames.clone());
    vd.source = VideoSource::File { path: "/tmp/seq".into() };
    vd.show_altered = false;
    l.video = Some(vd);
    doc.layers.push(l);

    let bytes = save_to_bytes(&doc, &SaveOptions::default()).unwrap();
    let back = load_from_bytes(&bytes).unwrap();
    let v = back.layers.last().unwrap().video.as_ref().expect("video survived");
    assert_eq!(v.frames.len(), 3);
    assert_eq!(v.original_frames.as_ref().map(Vec::len), Some(3));
    assert_eq!(v.original_frames.as_ref().unwrap()[2].rgba(1, 1), v.frames[2].rgba(1, 1));
    assert_eq!(v.fps, 24.0);
    assert!(!v.show_altered);
    assert_eq!(v.source, VideoSource::File { path: "/tmp/seq".into() });
    assert_eq!(back.timeline.as_ref().unwrap().duration, 3);
    // A middle frame's pixel survived.
    let mut px = [[0.0f32; 4]; 1];
    v.frames[2].read_rgba_into(Rect::from_xywh(1, 1, 1, 1), &mut px);
    assert!(px[0][0] > 0.9 && px[0][2] < 0.1, "frame 2 is reddish: {:?}", px[0]);
}

/// Issue #1101: serde_json writes NaN and the infinities as `null`, which the manifest's float
/// fields refuse on load, so such a document saved "successfully" and never opened again. The
/// save is refused instead, naming the value; finite documents are unaffected.
#[test]
fn a_non_finite_float_is_refused_at_save_rather_than_breaking_every_later_load() {
    use photocraft_doc::Adjustment;
    type Mutate = Box<dyn Fn(&mut Document)>;
    let base = rich_doc(ColorMode::Rgb, SampleType::U8);
    let cases: Vec<(&str, Mutate)> = vec![
        (
            "exposure",
            Box::new(|d| {
                d.insert_above(None, Layer::new("adj", LayerContent::Adjustment(Adjustment::Exposure { exposure: f32::NAN, offset: 0.0, gamma: 1.0 })));
            }),
        ),
        (
            "gamma",
            Box::new(|d| {
                d.insert_above(
                    None,
                    Layer::new("adj", LayerContent::Adjustment(Adjustment::Exposure { exposure: 0.0, offset: 0.0, gamma: f32::NEG_INFINITY })),
                );
            }),
        ),
        ("resolution_dpi", Box::new(|d| d.resolution_dpi = f32::INFINITY)),
    ];
    for (field, mutate) in cases {
        let mut doc = base.clone();
        mutate(&mut doc);
        let err = save_to_bytes(&doc, &SaveOptions::default()).expect_err(field);
        assert!(matches!(err, FormatError::NonFinite { .. }), "{field}: {err:?}");
        let msg = err.to_string();
        assert!(msg.contains(field) && msg.contains("NaN or infinite"), "{field}: {msg}");
        // The incremental writer shares the check, for autosave and directory bundles.
        let mut w = PcraftWriter::new();
        assert!(matches!(w.save_zip(&doc, &SaveOptions::default()), Err(FormatError::NonFinite { .. })), "{field}: PcraftWriter");
    }
    // The same document with finite values round-trips as before.
    let mut doc = base.clone();
    doc.insert_above(None, Layer::new("adj", LayerContent::Adjustment(Adjustment::Exposure { exposure: 0.5, offset: 0.0, gamma: 1.0 })));
    let back = load_from_bytes(&save_to_bytes(&doc, &SaveOptions::default()).unwrap()).unwrap();
    assert_eq!(back, doc);
}

/// #4957: a directory save holds the bundle's lock for its whole transaction. While another
/// writer holds it, a save waits, then fails without writing anything; once it is released the
/// save goes ahead.
#[test]
fn directory_save_waits_for_the_bundle_lock() {
    let doc = rich_doc(ColorMode::Rgb, SampleType::U8);
    let dir = temp_dir("lock");
    let other = std::fs::OpenOptions::new().read(true).write(true).create(true).truncate(false).open(dir.join("save.lock")).unwrap();
    // A file system without file locks saves unlocked (`lock_bundle`), so there's nothing to test.
    match other.try_lock() {
        Ok(()) => {}
        Err(std::fs::TryLockError::Error(e)) if e.kind() == std::io::ErrorKind::Unsupported => return,
        Err(std::fs::TryLockError::Error(e)) => panic!("locking save.lock failed: {e}"),
        Err(std::fs::TryLockError::WouldBlock) => panic!("save.lock is already locked"),
    }
    let mut w = PcraftWriter::new();
    w.set_lock_wait(std::time::Duration::from_millis(100));
    let e = w.save_dir(&doc, &dir, &SaveOptions::default()).unwrap_err();
    assert!(e.to_string().contains("in progress"), "{e}");
    assert!(!dir.join("manifest.json").exists(), "nothing written while locked");
    assert!(!dir.join("tiles").exists() || std::fs::read_dir(dir.join("tiles")).unwrap().count() == 0);
    other.unlock().unwrap();
    w.save_dir(&doc, &dir, &SaveOptions::default()).unwrap();
    assert_eq!(load_path(&dir).unwrap(), doc);
    drop(other);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn variables_follow_layer_id_remapping() {
    let mut doc = rich_doc(ColorMode::Rgb, SampleType::U8);
    let id_of = |d: &Document, name: &str| d.walk().into_iter().find(|(_, _, l)| l.name == name).unwrap().2.id;
    doc.variables.defs = vec![
        VariableDef { name: "head".into(), layer: id_of(&doc, "Paint"), kind: VarKind::TextReplacement },
        VariableDef { name: "badge".into(), layer: id_of(&doc, "Inner"), kind: VarKind::Visibility },
        VariableDef { name: "gone".into(), layer: LayerId(u64::MAX), kind: VarKind::Visibility },
    ];
    let bytes = save_to_bytes(&doc, &SaveOptions::default()).unwrap();
    // Preserved ids keep every definition exactly as saved, dangling ones included.
    assert_eq!(load_from_bytes(&bytes).unwrap(), doc);
    // Remapped ids carry the definitions to the copied layers; a dangling target stays
    // dangling and must not collide with a live layer (#4969).
    let back = load_from_bytes_with(&bytes, &LoadOptions { preserve_ids: false, ..Default::default() }).unwrap();
    let defs = &back.variables.defs;
    assert_eq!(defs.iter().find(|d| d.name == "head").unwrap().layer, id_of(&back, "Paint"));
    assert_eq!(defs.iter().find(|d| d.name == "badge").unwrap().layer, id_of(&back, "Inner"));
    let gone = defs.iter().find(|d| d.name == "gone").unwrap().layer;
    assert!(back.walk().into_iter().all(|(_, _, l)| l.id != gone), "dangling target must not address a live layer");
}

#[test]
fn color_lookup_lut_file_round_trips_and_old_files_load() {
    use photocraft_doc::Adjustment;
    use photocraft_doc::adjust::{LutFormat, LutSource};
    let lut = Some(std::sync::Arc::new(vec![0.5f32; 24]));
    let source = Some(LutSource { format: LutFormat::ThreeDl, bytes: std::sync::Arc::new(b"0 1023\n0 0 0\n".to_vec()) });
    let adj = Adjustment::ColorLookup { name: "Film.3dl".into(), lut, size: 2, tetrahedral: false, dither: false, source };
    let mut doc = Document::new("d", photocraft_doc::Size::new(4, 4), ColorMode::Rgb, SampleType::U8);
    doc.insert_above(None, Layer::new("lookup", LayerContent::Adjustment(adj.clone())));
    let back = load_from_bytes(&save_to_bytes(&doc, &SaveOptions::default()).unwrap()).unwrap();
    assert_eq!(back, doc, "the LUT file survives a .pcraft round trip (#6544)");
    // A .pcraft written before the field existed has no `source`: it loads as None.
    let mut v = serde_json::to_value(&adj).unwrap();
    let removed = v.as_object_mut().and_then(|o| o.values_mut().next()).and_then(|f| f.as_object_mut()).and_then(|f| f.remove("source"));
    assert!(removed.is_some(), "{v}");
    let Adjustment::ColorLookup { source: None, name, .. } = serde_json::from_value(v).unwrap() else { panic!("old lookup must load") };
    assert_eq!(name, "Film.3dl");
}

/// Removing previews is post-commit housekeeping, not part of publishing the new document.
#[test]
fn directory_save_without_previews_removes_stale_files_after_commit() {
    let mut doc = Document::new("before", photocraft_doc::Size::new(4, 3), ColorMode::Rgb, SampleType::U8);
    let dir = temp_dir("preview-cleanup");
    let mut writer = PcraftWriter::new();
    let opts = SaveOptions { thumbnail: Some(Rgba8Image::new(2, 2)), composite: Some(Rgba8Image::new(4, 3)), ..Default::default() };
    writer.save_dir(&doc, &dir, &opts).unwrap();
    doc.name = "after".into();
    writer.save_dir(&doc, &dir, &SaveOptions::default()).unwrap();
    assert_eq!(load_path(&dir).unwrap(), doc);
    assert!(!dir.join("thumb.png").exists());
    assert!(!dir.join("composite/preview.png").exists());
    std::fs::remove_dir_all(dir).unwrap();
}

/// A real Windows handle denying delete-sharing blocks manifest replacement after preparation.
#[cfg(windows)]
#[test]
fn failed_manifest_commit_preserves_omitted_previews() {
    use std::os::windows::fs::OpenOptionsExt;

    for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
        let mut doc = Document::with_background("before", photocraft_doc::Size::new(4, 3), ColorMode::Rgb, depth, photocraft_color::Color::WHITE);
        let dir = temp_dir("locked-manifest-previews");
        let mut writer = PcraftWriter::new();
        let opts = SaveOptions { thumbnail: Some(Rgba8Image::new(2, 2)), composite: Some(Rgba8Image::new(4, 3)), ..Default::default() };
        writer.save_dir(&doc, &dir, &opts).unwrap();
        let old_doc = doc.clone();
        let paths = ["manifest.json", "thumb.png", "composite/preview.png"];
        let before: Vec<_> = paths.iter().map(|p| std::fs::read(dir.join(p)).unwrap()).collect();
        // FILE_SHARE_READ | FILE_SHARE_WRITE; deliberately omit FILE_SHARE_DELETE.
        let held = std::fs::OpenOptions::new().read(true).share_mode(3).open(dir.join("manifest.json")).unwrap();
        doc.name = "after".into();
        let error = writer.save_dir(&doc, &dir, &SaveOptions::default()).unwrap_err();
        assert!(matches!(error, FormatError::Io(_)), "expected a real commit I/O failure: {error}");
        for (path, bytes) in paths.iter().zip(&before) {
            assert_eq!(std::fs::read(dir.join(path)).ok().as_ref(), Some(bytes), "saved file lost after failed commit: {path}");
        }
        assert_eq!(load_path(&dir).unwrap(), old_doc);
        drop(held);
        writer.save_dir(&doc, &dir, &SaveOptions::default()).unwrap();
        assert_eq!(load_path(&dir).unwrap(), doc);
        assert!(!dir.join("thumb.png").exists());
        assert!(!dir.join("composite/preview.png").exists());
        std::fs::remove_dir_all(dir).unwrap();
    }
}

#[cfg(windows)]
#[test]
fn locked_stale_preview_does_not_fail_committed_save() {
    use std::os::windows::fs::OpenOptionsExt;

    let mut doc = Document::new("before", photocraft_doc::Size::new(4, 3), ColorMode::Rgb, SampleType::U8);
    let dir = temp_dir("locked-stale-preview");
    let mut writer = PcraftWriter::new();
    let opts = SaveOptions { thumbnail: Some(Rgba8Image::new(2, 2)), composite: Some(Rgba8Image::new(4, 3)), ..Default::default() };
    writer.save_dir(&doc, &dir, &opts).unwrap();
    let held = std::fs::OpenOptions::new().read(true).share_mode(3).open(dir.join("thumb.png")).unwrap();
    doc.name = "after".into();
    writer.save_dir(&doc, &dir, &SaveOptions::default()).unwrap();
    assert_eq!(load_path(&dir).unwrap(), doc);
    assert!(dir.join("thumb.png").exists(), "locked leftover remains until a later cleanup");
    assert!(!dir.join("composite/preview.png").exists());
    drop(held);
    writer.save_dir(&doc, &dir, &SaveOptions::default()).unwrap();
    assert!(!dir.join("thumb.png").exists());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn arrow_document_is_backward_compatible() {
    use photocraft_doc::{Fill, LiveShape, Path, ShapeLayer, Subpath};

    let arrow =
        LiveShape::ArrowLine { from: [12.0, 64.0], to: [140.0, 64.0], weight: 16.0, start: false, end: true, width: 52.0, length: 50.0, concavity: 20.0 };

    let path = Path::new(vec![Subpath::polygon(&[(12.0, 56.0), (100.0, 56.0), (100.0, 44.0), (140.0, 64.0), (100.0, 84.0), (100.0, 72.0), (12.0, 72.0)])]);

    let mut doc = Document::new("Arrow compatibility", photocraft_doc::Size::new(256, 256), ColorMode::Rgb, SampleType::U8);

    doc.layers.push(Layer::new(
        "Editable arrow",
        LayerContent::Shape(ShapeLayer {
            path: path.clone(),
            fill: Some(Fill::Solid(photocraft_doc::Color::BLACK)),
            live: Some(arrow.clone()),
            ..Default::default()
        }),
    ));

    let bytes = save_to_bytes(&doc, &SaveOptions::default()).unwrap();
    let manifest = read_manifest(&bytes).unwrap();
    let json = serde_json::to_value(&manifest).unwrap();
    let saved = &json["document"]["layers"][0]["content"];

    assert_eq!(saved["kind"], "shape");
    assert!(saved["live"].is_null());
    assert_eq!(saved["arrow_live"]["kind"], "arrowLine");
    assert_eq!(saved["path"], serde_json::to_value(&path).unwrap());

    // Simulate an old reader that knows no ArrowLine or arrow_live.
    #[derive(serde::Deserialize)]
    struct LegacyShape {
        live: Option<LegacyLive>,
        path: Path,
    }

    #[derive(serde::Deserialize)]
    #[serde(tag = "kind", rename_all = "camelCase")]
    #[allow(dead_code)]
    enum LegacyLive {
        Line { from: [f64; 2], to: [f64; 2], weight: f64 },
    }

    let legacy: LegacyShape = serde_json::from_value(saved.clone()).unwrap();
    assert!(legacy.live.is_none());
    assert_eq!(legacy.path, path);

    // The new reader restores full parametric editability.
    let loaded = load_from_bytes(&bytes).unwrap();
    assert_eq!(loaded, doc);
}

/// `FileEncoding::source_images` is kept in `.pcraft` with the other encoder settings (#5733):
/// the crash-recovery autosave is a `.pcraft` bundle reopened at the document's original path, so
/// dropping the count would let Save write one frame over a multi-image file. A manifest written
/// before the key existed loads with no count.
#[test]
fn file_encoding_source_images_roundtrips_and_defaults_when_absent() {
    for count in [3, u32::MAX] {
        let mut doc = Document::new("anim", photocraft_doc::Size::new(2, 2), ColorMode::Rgb, SampleType::U8);
        doc.file_encoding = photocraft_doc::FileEncoding { jpeg_quality: Some(80), source_images: Some(count), ..Default::default() };
        let bytes = save_to_bytes(&doc, &SaveOptions::default()).unwrap();
        let loaded = load_from_bytes(&bytes).unwrap();
        assert_eq!(loaded.file_encoding.source_images, Some(count));
        assert_eq!(loaded, doc);

        // The same manifest without the key, as an older build wrote it.
        let mut json = serde_json::to_value(read_manifest(&bytes).unwrap().document).unwrap();
        let encoding = json["file_encoding"].as_object_mut().unwrap();
        assert_eq!(encoding.remove("sourceImages"), Some(serde_json::json!(count)));
        let old: photocraft_format::manifest::DocM = serde_json::from_value(json).unwrap();
        assert_eq!(old.file_encoding, photocraft_doc::FileEncoding { jpeg_quality: Some(80), ..Default::default() });
    }
}
