//! #6152: intact layers can be recovered when only the merged preview is damaged.

use photocraft_color::{ColorMode, SampleType};
use photocraft_doc::{Document, Layer};
use photocraft_geom::Size;
use photocraft_io::{ExportOptions, export, import};
use photocraft_psd::{Compression, GroupSpec, PsdBuilder, PsdFile};

fn layered(depth: SampleType, extension: &str) -> PsdFile {
    layered_in_mode(ColorMode::Rgb, depth, extension)
}

fn layered_in_mode(mode: ColorMode, depth: SampleType, extension: &str) -> PsdFile {
    let mut doc = Document::new("recovery", Size::new(3, 2), mode, depth);
    let format = doc.pixel_format();
    let mut layer = Layer::raster("red pixels", doc.pixel_format());
    let mut pixel = vec![0.0; format.channels()];
    photocraft_raster::from_rgba_into(&format, [1.0, 0.0, 0.0, 1.0], &mut pixel);
    layer.surface_mut().unwrap().fill_rect(doc.bounds(), &pixel);
    doc.layers.push(layer);
    PsdFile::from_bytes(&export(&doc, extension, &ExportOptions::default()).unwrap().bytes).unwrap()
}

#[test]
fn truncated_merged_image_keeps_layer_pixels_at_every_depth_and_compression() {
    use ColorMode::{Cmyk, Grayscale, Lab, Rgb};
    use SampleType::{F32, U8, U16};
    for extension in ["x.psd", "x.psb"] {
        for (mode, depth) in
            [(Rgb, U8), (Rgb, U16), (Rgb, F32), (Grayscale, U8), (Grayscale, U16), (Grayscale, F32), (Cmyk, U8), (Cmyk, U16), (Lab, U8), (Lab, U16)]
        {
            for compression in Compression::ALL {
                let mut file = layered_in_mode(mode, depth, extension);
                let intact = import(extension, &file.to_bytes().unwrap()).unwrap();
                let expected = photocraft_compose::flatten(&intact.document).px;
                let pixels = file.decode_merged().unwrap();
                file.image_data = photocraft_psd::ImageData::encode(compression, &pixels, &file.header).unwrap();
                file.image_data.data.truncate(file.image_data.data.len() / 2);
                let bytes = file.to_bytes().unwrap();
                assert!(PsdFile::from_bytes(&bytes).is_err(), "strict parsing stays strict");
                let result = import(extension, &bytes).expect("recover the intact layers");
                assert_eq!(result.document.layer_count(), 1);
                assert_eq!(result.document.depth, depth);
                assert!(result.warnings.iter().any(|w| w.contains("merged image")), "{:?}", result.warnings);
                let rendered = photocraft_compose::flatten(&result.document);
                assert_eq!(rendered.px, expected, "{extension} {mode:?} {depth:?} {compression:?}");
                // Saving the recovered document replaces the bad preview with a valid one.
                let saved = export(&result.document, extension, &ExportOptions::default()).unwrap();
                assert!(PsdFile::from_bytes(&saved.bytes).unwrap().decode_merged().is_ok());
            }
        }
    }
}

#[test]
fn corrupt_zip_merged_image_keeps_the_layers() {
    let mut file = layered(SampleType::U8, "x.psd");
    file.image_data = photocraft_psd::ImageData::encode(Compression::Zip, &file.decode_merged().unwrap(), &file.header).unwrap();
    *file.image_data.data.last_mut().unwrap() ^= 0xff;
    let result = import("x.psd", &file.to_bytes().unwrap()).unwrap();
    assert_eq!(result.document.layer_count(), 1);
    assert!(result.warnings.iter().any(|w| w.contains("merged image")));
}

#[test]
fn an_empty_group_survives_a_truncated_preview() {
    // The reported psd-tools fixture has only the two records of an empty group.
    let mut builder = PsdBuilder::new(100, 100);
    builder.begin_group(GroupSpec::new("empty group"));
    builder.end_group().unwrap();
    let mut file = builder.build().unwrap();
    file.image_data.data.truncate(file.image_data.data.len() / 2);
    let result = import("empty.psd", &file.to_bytes().unwrap()).unwrap();
    assert_eq!(result.document.layer_count(), 1);
    assert_eq!(result.document.layers[0].name, "empty group");
    assert!(result.warnings.iter().any(|w| w.contains("merged image")));
}

#[test]
fn a_flattened_file_with_no_recoverable_layers_still_fails() {
    let mut file = layered(SampleType::U8, "x.psd");
    file.layer_info = None;
    file.image_data.data.truncate(file.image_data.data.len() / 2);
    assert!(import("flat.psd", &file.to_bytes().unwrap()).is_err());
}

#[test]
fn modes_imported_flattened_do_not_recover_from_unusable_layer_records() {
    for mode in [photocraft_psd::ColorMode::Indexed, photocraft_psd::ColorMode::Multichannel, photocraft_psd::ColorMode::Duotone] {
        let mut file = layered(SampleType::U8, "x.psd");
        file.header.color_mode = mode;
        file.image_data = photocraft_psd::ImageData::new(Compression::Raw, Vec::new());
        assert!(import("flat-mode.psd", &file.to_bytes().unwrap()).is_err(), "{mode:?}");
    }
}

#[test]
fn lost_extra_channels_are_reported_while_layers_remain_usable() {
    let mut file = layered(SampleType::U8, "x.psd");
    file.header.channels = 5;
    file.image_data = photocraft_psd::ImageData::new(Compression::Raw, Vec::new());
    let result = import("extra-channel.psd", &file.to_bytes().unwrap()).unwrap();
    assert_eq!(result.document.layer_count(), 1);
    assert!(result.document.channels.is_empty());
    assert!(result.warnings.iter().any(|w| w.contains("alpha/spot channels")), "{:?}", result.warnings);
}

#[test]
fn truncated_layer_sections_are_not_recovered() {
    let file = layered(SampleType::U8, "x.psd");
    let mut bytes = file.to_bytes().unwrap();
    bytes.truncate(bytes.len() - file.image_data.data.len() - 3);
    assert!(import("broken-layers.psd", &bytes).is_err());
}

#[cfg(feature = "corpus")]
#[test]
fn reported_group_divider_fixture_opens_with_its_group() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus/psd-tools/blend-modes/group-divider-blend-mode.psd");
    let bytes = std::fs::read(path).expect("run cargo xtask corpus --psd-tools");
    let result = import("group-divider-blend-mode.psd", &bytes).expect("#6152 opens");
    assert_eq!(result.document.layer_count(), 1);
    assert!(matches!(result.document.layers[0].content, photocraft_doc::LayerContent::Group(_)));
    assert!(result.warnings.iter().any(|w| w.contains("merged image")));
}
