use super::*;
use photocraft_psd::{ColorMode, Compression, Version, testgen};

#[test]
fn retained_storage_is_summed_but_workspace_is_reused() {
    let mut budget = ImportBudget::default();
    budget.retain(40).unwrap();
    budget.retain(20).unwrap();
    budget.scratch(30);
    budget.scratch(10);
    assert!(budget.check(90).is_ok());
    assert!(matches!(budget.check(89), Err(IoError::PsdImportBudget { required: 90, limit: 89 })));
    budget.retain(u64::MAX).unwrap_err();
    budget.workspace = u64::MAX;
    assert!(budget.required().is_err());
}

#[test]
fn tiny_negative_offset_rectangles_pay_for_every_touched_tile() {
    let mut aligned = ImportBudget::default();
    aligned.surface(Rect::from_xywh(0, 0, 2, 2), 4, 2).unwrap();
    let mut offset = ImportBudget::default();
    offset.surface(Rect::from_xywh(-1, -1, 2, 2), 4, 2).unwrap();
    assert_eq!(offset.required().unwrap(), aligned.required().unwrap() * 4);
    offset.surface(Rect::default(), 4, 2).unwrap();
    assert_eq!(offset.required().unwrap(), aligned.required().unwrap() * 4);
}

fn estimate(file: &PsdFile) -> u64 {
    match check(file, 0).unwrap_err() {
        IoError::PsdImportBudget { required, .. } => required,
        error => panic!("unexpected error: {error}"),
    }
}

#[test]
fn individually_fitting_layers_cannot_each_spend_the_import_allowance() {
    let mut file = testgen::small(Version::Psd, Compression::Zip);
    let record = file.layers()[0].clone();
    file.layer_info.as_mut().unwrap().layers = vec![record.clone()];
    let one = estimate(&file);
    assert!(check(&file, one).is_ok());
    file.layer_info.as_mut().unwrap().layers = vec![record; 8];
    assert!(matches!(check(&file, one), Err(IoError::PsdImportBudget { .. })));
}

#[test]
fn large_off_canvas_layer_is_rejected_without_decoding_its_tiny_data() {
    let mut file = testgen::small(Version::Psd, Compression::Zip);
    file.header.width = 1;
    file.header.height = 1;
    file.layers_mut()[0].rect = Rect::from_xywh(-10_000, -10_000, 20_000, 20_000);
    assert!(matches!(check(&file, DEFAULT_PSD_IMPORT_PIXEL_BYTES), Err(IoError::PsdImportBudget { .. })));
    // Both ordinary import routes must fail before reading a deliberately missing merged
    // section. No enormous decoded plane is ever constructed by this test.
    let mut bytes = file.to_bytes().unwrap();
    bytes.truncate(bytes.len() - file.image_data.data.len() - 2);
    assert!(matches!(crate::import("huge.psd", &bytes), Err(IoError::PsdImportBudget { .. })));
    let source = photocraft_psd::Source::new(bytes);
    assert!(matches!(crate::import_source_for_open_with("huge.psd", source, &photocraft_raster::Interrupt::NONE), Err(IoError::PsdImportBudget { .. })));
}

#[test]
fn user_mask_bounds_count_even_when_the_layer_is_small() {
    let mut file = testgen::small(Version::Psd, Compression::Raw);
    let before = estimate(&file);
    // The generator's first record has a user mask. Modify its parsed mask in place.
    if let photocraft_psd::layer::MaskData::Mask(mask) = &mut file.layers_mut()[0].mask {
        mask.rect = Rect::from_xywh(0, 0, 20_000, 20_000);
    } else {
        panic!("fixture needs a parsed mask");
    }
    assert!(estimate(&file) > before);
    assert!(matches!(check(&file, before), Err(IoError::PsdImportBudget { .. })));
}

#[test]
fn merged_bitmap_and_multichannel_expansion_are_budgeted() {
    for (mode, depth) in [(ColorMode::Bitmap, 1), (ColorMode::Multichannel, 8), (ColorMode::Rgb, 32)] {
        let mut file = testgen::merged_only(Version::Psb, mode, depth, Compression::Zip, 2, 2);
        let before = estimate(&file);
        assert!(check(&file, before).is_ok());
        file.header.width = 60_000;
        file.header.height = 60_000;
        assert!(matches!(check(&file, DEFAULT_PSD_IMPORT_PIXEL_BYTES), Err(IoError::PsdImportBudget { .. })));
    }
}

#[test]
fn a_custom_limit_is_fatal_for_interactive_and_headless_imports() {
    let file = testgen::small(Version::Psd, Compression::Zip);
    let bytes = file.to_bytes().unwrap();
    let limit = estimate(&file);
    let opts = crate::ImportOptions { psd_max_pixel_bytes: limit, ..Default::default() };
    assert!(crate::import_with_options("small.psd", &bytes, &opts).is_ok());
    let opts = crate::ImportOptions { psd_max_pixel_bytes: limit - 1, ..opts };
    for result in [crate::import_with_options("small.psd", &bytes, &opts), crate::import_for_open("small.psd", &bytes, &opts).map(|(result, _)| result)] {
        assert!(matches!(result, Err(IoError::PsdImportBudget { .. })));
    }
}

#[test]
fn accepted_off_canvas_pixels_and_depths_are_preserved() {
    for depth in [8, 16, 32] {
        for compression in Compression::ALL {
            let file = testgen::layered(Version::Psd, ColorMode::Rgb, depth, compression);
            assert!(check(&file, DEFAULT_PSD_IMPORT_PIXEL_BYTES).is_ok());
            let (doc, _) = crate::psd_to_document(&file);
            assert_eq!(doc.depth, crate::pixels::sample_for_depth(depth));
            let bytes = file.to_bytes().unwrap();
            let opened = crate::import("small.psd", &bytes).unwrap().document;
            let rect = photocraft_geom::Rect::new(-8, -8, 32, 24);
            assert_eq!(photocraft_compose::render(&opened, rect), photocraft_compose::render(&doc, rect));
        }
    }
}

#[test]
fn legacy_conversion_reports_a_rejected_import_instead_of_decoding_it() {
    let mut file = testgen::small(Version::Psd, Compression::Zip);
    file.layers_mut()[0].rect = Rect::from_xywh(0, 0, 20000, 20000);
    let (doc, warnings) = crate::psd_to_document(&file);
    assert!(doc.layers.is_empty());
    assert!(warnings.iter().any(|warning| warning.contains("pixel memory")));
}

fn pattern_block(mode: u32, depth: u16, width: u32, height: u32) -> photocraft_psd::TaggedBlock {
    use photocraft_psd::patterns::{PsdPattern, mode_channels, write_pattern_block};
    let len = (width as usize * depth as usize).div_ceil(8) * height as usize;
    let p = PsdPattern {
        mode,
        depth,
        width,
        height,
        name: "Budget pattern".into(),
        id: "budget".into(),
        palette: (mode == 2).then(|| vec![0; 768]),
        channels: vec![vec![0; len]; mode_channels(mode)],
        alpha: None,
    };
    photocraft_psd::TaggedBlock::new(*b"Patt", write_pattern_block(&[p]).unwrap())
}

#[test]
fn pattern_tiles_and_expansion_share_the_import_allowance() {
    for (mode, depth) in [(1, 1), (2, 8), (3, 8), (4, 16), (9, 32)] {
        let mut file = testgen::small(Version::Psd, Compression::Raw);
        let baseline = estimate(&file);
        let block = pattern_block(mode, depth, 9, 3);
        file.global_blocks.push(block.clone());
        let one = estimate(&file);
        assert!(one > baseline);
        assert!(check(&file, one).is_ok());
        assert!(matches!(check(&file, one - 1), Err(IoError::PsdImportBudget { .. })));
        let bytes = file.to_bytes().unwrap();
        let opts = crate::ImportOptions { psd_max_pixel_bytes: one - 1, ..Default::default() };
        assert!(matches!(crate::import_with_options("pattern.psd", &bytes, &opts), Err(IoError::PsdImportBudget { .. })));
        assert_eq!(crate::import("pattern.psd", &bytes).unwrap().document.patterns.len(), 1);
        file.global_blocks.push(block);
        assert!(estimate(&file) > one, "blocks cannot each reuse the allowance");
    }
}

#[test]
fn malformed_later_pattern_still_counts_earlier_decodes() {
    let mut file = testgen::small(Version::Psd, Compression::Raw);
    let baseline = estimate(&file);
    let mut block = pattern_block(1, 1, 9, 3);
    // An invalid second pattern makes the real parser discard the block, after decoding
    // its first record. The preflight must still account for that transient allocation.
    block.data.extend_from_slice(&4u32.to_be_bytes());
    block.data.extend_from_slice(&99u32.to_be_bytes());
    file.global_blocks.push(block);
    assert!(estimate(&file) > baseline);
}

/// An unknown colour mode imports as RGB (`import_format`), layers included, so it must pay
/// for them: a layered file can't dodge the allowance by carrying mode 5.
#[test]
fn an_unknown_color_mode_pays_for_the_layers_it_imports_as_rgb() {
    let rgb = testgen::small(Version::Psd, Compression::Raw);
    assert!(!rgb.layers().is_empty() && !super::super::psd_import::merged_needed(&rgb), "the fixture must import its layers alone");
    let mut unknown = rgb.clone();
    unknown.header.color_mode = photocraft_psd::ColorMode::Unknown(5);
    assert!(unknown.header.validate().is_ok());
    let required = estimate(&unknown);
    assert!(required >= estimate(&rgb), "the layers import as RGB and are counted");
    let bytes = unknown.to_bytes().unwrap();
    let opts = crate::ImportOptions { psd_max_pixel_bytes: required - 1, ..Default::default() };
    assert!(matches!(crate::import_with_options("mode5.psd", &bytes, &opts), Err(IoError::PsdImportBudget { .. })));
    let source = photocraft_psd::Source::new(bytes);
    let refused = crate::import_source_with_limit("mode5.psd", source, &photocraft_raster::Interrupt::NONE, required - 1);
    assert!(matches!(refused, Err(IoError::PsdImportBudget { .. })), "{:?}", refused.map(|(r, _)| r.warnings));
}
