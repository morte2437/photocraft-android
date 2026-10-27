use super::*;
use photocraft_psd::descriptor::UnicodeString;
use photocraft_psd::resources::VersionInfo;
use photocraft_psd::{Compression, ImageData, ImageResource, PsdError, Version, testgen};

/// #5200: the Multichannel fallback checks its RGBA output against the budget before
/// allocating it (here a 2x2 image, 16 bytes, against caps of 15 and 16 bytes).
#[test]
fn flat_fallback_refuses_an_output_over_the_cap() {
    let file = testgen::merged_only(Version::Psd, PsdMode::Multichannel, 8, Compression::Raw, 2, 2);
    assert!(file.composite_rgba8().is_err(), "Multichannel goes through the fallback");
    let merged = file.decode_merged();
    assert!(matches!(flat_composite(&file, Some(&merged), 15), Err(PsdError::LimitExceeded(_))));
    let img = flat_composite(&file, Some(&merged), 16).unwrap();
    assert_eq!(img.data.len(), 16);
}

/// #5200: a composite over the budget is not retried by the fallback (which allocated the
/// refused buffer: 14.4 GB for this 60000x60000 Bitmap header).
#[test]
fn flat_fallback_does_not_retry_an_over_budget_composite() {
    let mut file = testgen::small(Version::Psb, Compression::Raw);
    file.header.color_mode = PsdMode::Bitmap;
    file.header.depth = 1;
    file.header.channels = 1;
    file.header.width = 60_000;
    file.header.height = 60_000;
    let merged = Ok(vec![0u8; 16]);
    let cap = photocraft_psd::compression::MAX_DECODED_BYTES;
    assert!(matches!(flat_composite(&file, Some(&merged), cap), Err(PsdError::LimitExceeded(_))));
}

/// #5202: a hostile name length ends the list instead of overflowing.
#[test]
fn unicode_names_stop_at_a_hostile_length() {
    let mut data = 1u32.to_be_bytes().to_vec();
    data.extend([0, b'A']);
    data.extend(u32::MAX.to_be_bytes());
    data.extend([0, b'B']);
    assert_eq!(unicode_names(&data), ["A"]);
}

/// Verifies an unreadable layer channel sets the structured damage indicator.
#[test]
fn unreadable_layer_channel_marks_conversion_damaged() {
    let mut file = testgen::small(Version::Psd, Compression::Raw);
    let channel = file.layers_mut()[0].channels.iter_mut().find(|c| c.id == 0).unwrap();
    channel.data.clear();
    assert!(file.layers()[0].decode_channel(0, file.header.depth, file.header.version).is_err());

    let (_, warnings, damaged) = psd_to_document_decoded_integrity(&file, &photocraft_raster::Interrupt::NONE, None).unwrap();
    assert!(damaged);
    assert!(warnings.iter().any(|w| w.contains("could not be decoded")));
}

/// Verifies an unreadable user-mask channel sets the structured damage indicator.
#[test]
fn unreadable_mask_channel_marks_conversion_damaged() {
    let mut file = testgen::small(Version::Psd, Compression::Raw);
    let channel = file.layers_mut()[0].channels.iter_mut().find(|c| c.id == CHANNEL_USER_MASK).unwrap();
    channel.data.clear();

    let (_, warnings, damaged) = psd_to_document_decoded_integrity(&file, &photocraft_raster::Interrupt::NONE, None).unwrap();
    assert!(damaged);
    assert!(warnings.iter().any(|w| w.contains("could not be decoded")));
}

/// A layer whose bounds hold pixels but whose colour and transparency channels are missing
/// lost its pixel data: the open asks before continuing.
#[test]
fn layer_bounds_without_channel_data_mark_conversion_damaged() {
    let mut file = testgen::small(Version::Psd, Compression::Raw);
    file.layers_mut()[0].channels.retain(|c| c.id == CHANNEL_USER_MASK);

    let (_, warnings, damaged) = psd_to_document_decoded_integrity(&file, &photocraft_raster::Interrupt::NONE, None).unwrap();
    assert!(damaged);
    assert!(warnings.iter().any(|w| w.contains("no channel data")));
}

/// A layer without pixels (an empty layer, or a text or shape layer written without a
/// pixel cache) has empty bounds: nothing is missing, so the file is not damaged even when
/// it lists no colour or transparency channel.
#[test]
fn empty_layer_without_channel_data_is_not_damaged() {
    let mut file = testgen::small(Version::Psd, Compression::Raw);
    file.layers_mut()[0].channels.retain(|c| c.id == CHANNEL_USER_MASK);
    file.layers_mut()[0].rect = Default::default();

    let (_, warnings, damaged) = psd_to_document_decoded_integrity(&file, &photocraft_raster::Interrupt::NONE, None).unwrap();
    assert!(!damaged, "an empty layer marked the PSD damaged: {warnings:?}");
}

/// Verifies healthy layer channels do not mark the PSD as damaged.
#[test]
fn valid_layer_channels_keep_conversion_editable() {
    let file = testgen::small(Version::Psd, Compression::Raw);
    let (_, warnings, damaged) = psd_to_document_decoded_integrity(&file, &photocraft_raster::Interrupt::NONE, None).unwrap();
    assert!(!damaged, "unexpectedly marked good PSD damaged: {warnings:?}");
}

#[test]
fn merged_plane_checks_offsets() {
    let all = [1u8, 2, 3, 4];
    assert_eq!(merged_plane(&all, 1, 2), Some(&[3u8, 4][..]));
    assert_eq!(merged_plane(&all, 2, 2), None);
    assert_eq!(merged_plane(&all, usize::MAX, 2), None);
}

/// Resource 1057's data.
fn version_info(writer: UnicodeString, reader: UnicodeString, has_real_merged_data: bool) -> Vec<u8> {
    VersionInfo { version: 1, has_real_merged_data, writer, reader, file_version: 1 }.to_bytes()
}

/// The 1057 PhotoCraft 0.1.0 to 0.7.0 wrote: Photoshop CS6's names, NUL-terminated.
fn version_info_cs6(has_real_merged_data: bool) -> Vec<u8> {
    version_info(UnicodeString::new_nul("Adobe Photoshop"), UnicodeString::new_nul("Adobe Photoshop CS6"), has_real_merged_data)
}

/// The 1057 PhotoCraft writes since 0.8.2: Photoshop 2026's bytes, no NUL.
fn version_info_2026(has_real_merged_data: bool) -> Vec<u8> {
    photocraft_psd::resources::version_info_resource(has_real_merged_data).data
}

/// An opaque 4 × 1 RGB document with the Photoshop spot oracles' channels in their order:
/// "Spot A", "Alpha 1" and "Spot B", each holding 100, 25, 75 and 0 % in four columns.
fn channels_document(depth: SampleType) -> Document {
    let mut doc = Document::with_background("old", Size::new(4, 1), ColorMode::Rgb, depth, photocraft_color::Color::rgb(1.0, 1.0, 1.0));
    let gray = PixelFormat::new(ColorMode::Grayscale, depth, false);
    let canvas = doc.bounds();
    let channel = |name: &str, spot: Option<(photocraft_color::Color, f32)>| {
        let mut s = Surface::new(gray);
        s.write_region(canvas, &[1.0, 0.25, 0.75, 0.0]);
        AlphaChannel { spot, ..AlphaChannel::new(name, s) }
    };
    doc.channels.push(channel("Spot A", Some((photocraft_color::Color::rgb(0.0, 0.627, 1.0), 0.0))));
    doc.channels.push(channel("Alpha 1", None));
    doc.channels.push(channel("Spot B", Some((photocraft_color::Color::rgb(1.0, 0.0, 0.627), 0.5))));
    doc
}

/// Each channel's values on an 8-bit scale.
fn ink(doc: &Document) -> Vec<Vec<u8>> {
    doc.channels.iter().map(|c| c.surface.read_region(doc.bounds()).iter().map(|v| (v * 255.0).round() as u8).collect()).collect()
}

fn spots(doc: &Document) -> Vec<bool> {
    doc.channels.iter().map(|c| c.spot.is_some()).collect()
}

/// `doc` (opaque, 8 or 16-bit) saved the way PhotoCraft 0.1.0 to 0.16.0 saved it: spot planes
/// straight (the maximum = solid ink), no 1053, and `version_info` as resource 1057. `padded`
/// lays DisplayInfo out in the 14-byte records of 0.1.0 to 0.7.0.
fn as_earlier_photocraft_saved(doc: &Document, version_info: Vec<u8>, padded: bool) -> Vec<u8> {
    let mut file = crate::psd_export::document_to_psd(doc);
    assert!(!file.merged_has_alpha());
    file.resources.retain(|r| r.id != crate::channel_map::ALPHA_IDENTIFIERS);
    for r in &mut file.resources {
        match r.id {
            ids::VERSION_INFO => r.data = version_info.clone(),
            crate::channel_map::DISPLAY_INFO if padded => {
                let (version, records) = r.data.split_at(4);
                r.data = version.iter().copied().chain(records.chunks(13).flat_map(|rec| rec.iter().copied().chain([0]))).collect();
            }
            _ => {}
        }
    }
    let mut planes = file.decode_merged().unwrap();
    let plane = planes.len() / usize::from(file.header.channels);
    for (k, c) in doc.channels.iter().enumerate() {
        if c.spot.is_some() {
            // At 8 and 16 bits the complement of a sample is that of each of its bytes.
            planes[(3 + k) * plane..(4 + k) * plane].iter_mut().for_each(|b| *b = !*b);
        }
    }
    file.image_data = ImageData::encode(Compression::Raw, &planes, &file.header).unwrap();
    file.to_bytes().unwrap()
}

/// PSDs that PhotoCraft 0.1.0 to 0.16.0 saved with spot channels store them straight. They
/// open with the ink they had in those versions (which read every plane straight), whichever
/// 1057 they carry. Saved again, they get Photoshop's polarity and a 1053, and reopen with the
/// same ink. (A 0.1.0 to 0.7.0 file's padded records misread every channel's options after
/// the first, here Spot B's kind, as those versions' successors always have; its ink is kept.)
#[test]
fn spot_planes_from_earlier_photocraft_keep_their_ink() {
    for depth in [SampleType::U8, SampleType::U16] {
        let doc = channels_document(depth);
        let cases = [
            ("0.1.0 to 0.7.0", version_info_cs6(true), true),
            ("0.8.2 on", version_info_2026(true), false),
            ("0.8.2 on, Maximize Compatibility off", version_info_2026(false), false),
            ("0.1.0 to 0.7.0, 13-byte records", version_info_cs6(true), false),
        ];
        for (what, version_info, padded) in cases {
            let bytes = as_earlier_photocraft_saved(&doc, version_info, padded);
            assert!(earlier_photocraft_spot_planes(&PsdFile::from_bytes(&bytes).unwrap()), "{what}");
            let opened = crate::import("old.psd", &bytes).unwrap().document;
            assert_eq!(ink(&opened), ink(&doc), "{depth:?} {what}: the ink earlier versions opened");
            assert!(spots(&opened)[0], "{depth:?} {what}");
            if !padded {
                assert_eq!(spots(&opened), spots(&doc), "{depth:?} {what}");
            }
            let saved = crate::export(&opened, "old.psd", &crate::ExportOptions::default()).unwrap();
            let file = PsdFile::from_bytes(&saved.bytes).unwrap();
            assert!(file.resource(crate::channel_map::ALPHA_IDENTIFIERS).is_some() && !earlier_photocraft_spot_planes(&file), "{what}");
            let again = crate::import("again.psd", &saved.bytes).unwrap().document;
            assert_eq!(ink(&again), ink(&doc), "{depth:?} {what}: saved again");
            assert_eq!(spots(&again), spots(&opened), "{depth:?} {what}: saved again");
        }
    }
}

/// A 0.1.0 to 0.7.0 file whose first extra channel is an alpha channel, spots after it. In its
/// 14-byte DisplayInfo records the spots' kind bytes sit where a 13-byte reader (main's, and
/// this one's) doesn't look, so only the padded reading finds a spot for the rule. Every plane
/// is then read straight, as those versions read it, and keeps its ink. The 13-byte reader
/// opens each channel's options from shifted bytes, as on main: here every channel opens as
/// an alpha channel. A spot of 2 % solidity puts a 2 where that reader looks for the kind, so
/// that channel opens as a spot: without the rule its plane would be inverted. Saved again and
/// reopened, every channel keeps its ink and kind.
#[test]
fn an_early_file_with_an_alpha_channel_first_keeps_its_ink() {
    for solidity in [0.0, 0.02] {
        let mut doc = channels_document(SampleType::U8);
        doc.channels.swap(0, 1);
        doc.channels[1].spot = Some((photocraft_color::Color::rgb(0.0, 0.627, 1.0), solidity));
        let names: Vec<&str> = doc.channels.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["Alpha 1", "Spot A", "Spot B"]);
        let bytes = as_earlier_photocraft_saved(&doc, version_info_cs6(true), true);
        let file = PsdFile::from_bytes(&bytes).unwrap();
        let info = &file.resource(crate::channel_map::DISPLAY_INFO).unwrap().data;
        let misread = solidity > 0.0;
        assert_eq!(crate::channel_map::spot_flags(info, true, 0), [false, misread, false], "{solidity}: 13-byte records");
        assert!(earlier_photocraft_spot_planes(&file), "{solidity}: found in the 14-byte records");
        let opened = crate::import("old.psd", &bytes).unwrap().document;
        assert_eq!(ink(&opened), ink(&doc), "{solidity}: the ink 0.7.0 opened");
        assert_eq!(spots(&opened), [false, misread, false], "{solidity}: kinds as main reads them");
        let saved = crate::export(&opened, "old.psd", &crate::ExportOptions::default()).unwrap();
        let again = crate::import("again.psd", &saved.bytes).unwrap().document;
        assert_eq!(ink(&again), ink(&doc), "{solidity}: saved again");
        assert_eq!(spots(&again), spots(&opened), "{solidity}: saved again");
    }
}

/// A file with spots and no 1053 that no earlier PhotoCraft wrote is read in Photoshop's
/// polarity (0 = solid ink). psd-tools' `cmyk-spot.psd` is one: its 1057 names psd-tools as
/// writer and reader. So is a file with no 1057, with Photoshop CS6's names without the NUL,
/// or with bytes after PhotoCraft's.
#[test]
fn other_files_without_alpha_identifiers_use_photoshop_polarity() {
    let doc = channels_document(SampleType::U8);
    let mut trailing = version_info_2026(true);
    trailing.extend([0, 0]);
    let cases = [
        ("psd-tools", Some(version_info(UnicodeString::new("psd-tools 1.10.0"), UnicodeString::new("psd-tools 1.10.0"), true))),
        ("no 1057", None),
        ("CS6 without a NUL", Some(version_info(UnicodeString::new("Adobe Photoshop"), UnicodeString::new("Adobe Photoshop CS6"), true))),
        ("bytes after PhotoCraft's 1057", Some(trailing)),
    ];
    for (what, info) in cases {
        // Photoshop's polarity, as this version writes it, without 1053.
        let mut file = crate::psd_export::document_to_psd(&doc);
        file.resources.retain(|r| r.id != crate::channel_map::ALPHA_IDENTIFIERS && r.id != ids::VERSION_INFO);
        file.resources.extend(info.map(|d| ImageResource::new(ids::VERSION_INFO, d)));
        let bytes = file.to_bytes().unwrap();
        assert!(!earlier_photocraft_spot_planes(&PsdFile::from_bytes(&bytes).unwrap()), "{what}");
        let opened = crate::import("x.psd", &bytes).unwrap().document;
        assert_eq!(ink(&opened), ink(&doc), "{what}");
        assert_eq!(spots(&opened), spots(&doc), "{what}");
    }
}

/// The rule needs a spot in DisplayInfo and no 1053, besides PhotoCraft's 1057.
#[test]
fn the_earlier_photocraft_rule_needs_a_spot_and_no_alpha_identifiers() {
    let mut doc = channels_document(SampleType::U8);
    let old = PsdFile::from_bytes(&as_earlier_photocraft_saved(&doc, version_info_2026(true), false)).unwrap();
    assert!(earlier_photocraft_spot_planes(&old));
    let mut with_ids = old.clone();
    with_ids.resources.push(ImageResource::new(crate::channel_map::ALPHA_IDENTIFIERS, crate::channel_map::alpha_identifiers(false, 3)));
    assert!(!earlier_photocraft_spot_planes(&with_ids), "a 1053");
    let mut no_info = old.clone();
    no_info.resources.retain(|r| r.id != crate::channel_map::DISPLAY_INFO);
    assert!(!earlier_photocraft_spot_planes(&no_info), "no DisplayInfo");
    doc.channels.retain(|c| c.spot.is_none());
    let alpha_only = PsdFile::from_bytes(&as_earlier_photocraft_saved(&doc, version_info_2026(true), false)).unwrap();
    assert!(!earlier_photocraft_spot_planes(&alpha_only), "alpha channels only");
}

/// `doc` as an earlier PhotoCraft saved a Photoshop file it had opened with the first `kept`
/// of `doc`'s channels: it wrote back the Photoshop file's 1053, `stale_ids`, unchanged and
/// those channels' planes as Photoshop stored them (it had read and written them straight),
/// and stored the channels added after them straight.
fn as_earlier_photocraft_resaved(doc: &Document, stale_ids: &[u32], kept: usize) -> Vec<u8> {
    let mut file = crate::psd_export::document_to_psd(doc);
    file.resources.retain(|r| r.id != crate::channel_map::ALPHA_IDENTIFIERS && r.id != ids::VERSION_INFO);
    file.resources.push(ImageResource::new(crate::channel_map::ALPHA_IDENTIFIERS, stale_ids.iter().flat_map(|id| id.to_be_bytes()).collect()));
    file.resources.push(ImageResource::new(ids::VERSION_INFO, version_info_2026(true)));
    let mut planes = file.decode_merged().unwrap();
    let plane = planes.len() / usize::from(file.header.channels);
    let first = usize::from(file.header.channels) - doc.channels.len();
    for (k, c) in doc.channels.iter().enumerate().skip(kept) {
        if c.spot.is_some() {
            planes[(first + k) * plane..(first + k + 1) * plane].iter_mut().for_each(|b| *b = !*b);
        }
    }
    file.image_data = ImageData::encode(Compression::Raw, &planes, &file.header).unwrap();
    file.to_bytes().unwrap()
}

/// An earlier PhotoCraft kept the 1053 of a Photoshop file it opened and wrote it back
/// unchanged, so a spot channel added there sits behind a 1053 that doesn't list it: a
/// transparent Photoshop document's `[0]` (the white plate of a DTF print, added before
/// saving), the `[0, 3]` of one with a spot channel already, or a `[0]` kept through a
/// flatten that removed the transparency. The added spot is stored straight and keeps its
/// ink; the Photoshop file's own spot keeps Photoshop's polarity. Saved again, both reopen
/// the same.
#[test]
fn a_spot_added_behind_a_kept_alpha_identifiers_resource_keeps_its_ink() {
    let transparent = |names: &[&str]| {
        let mut doc = channels_document(SampleType::U8);
        doc.layers = vec![Layer::raster("Layer 1", doc.pixel_format())];
        doc.channels.retain(|c| names.contains(&c.name.as_str()));
        doc
    };
    let mut flattened = channels_document(SampleType::U8);
    flattened.channels.retain(|c| c.name == "Spot A");
    let cases = [
        ("transparent, one spot added", transparent(&["Spot A"]), vec![0], 0),
        ("transparent, a Photoshop spot and one added", transparent(&["Spot A", "Spot B"]), vec![0, 3], 1),
        ("flattened, one spot added", flattened, vec![0], 0),
    ];
    for (what, doc, stale_ids, kept) in cases {
        let bytes = as_earlier_photocraft_resaved(&doc, &stale_ids, kept);
        let file = PsdFile::from_bytes(&bytes).unwrap();
        assert_eq!(file.merged_has_alpha(), what.starts_with("transparent"), "{what}");
        assert!(earlier_photocraft_spot_planes(&file), "{what}");
        let opened = crate::import("old.psd", &bytes).unwrap().document;
        assert_eq!(ink(&opened), ink(&doc), "{what}: the ink earlier versions opened");
        assert_eq!(spots(&opened), spots(&doc), "{what}");
        let saved = crate::export(&opened, "old.psd", &crate::ExportOptions::default()).unwrap();
        let again = crate::import("again.psd", &saved.bytes).unwrap().document;
        assert_eq!(ink(&again), ink(&doc), "{what}: saved again");
    }
}

/// A 1053 that lists every extra channel, as Photoshop and this version write it, keeps the
/// file in Photoshop's polarity, with PhotoCraft's (Photoshop 2026's) 1057 too: a transparent
/// file with a spot, and Photoshop's own `[0]` in a file without a transparency plane.
#[test]
fn a_complete_alpha_identifiers_resource_keeps_photoshop_polarity() {
    let mut doc = channels_document(SampleType::U8);
    doc.layers = vec![Layer::raster("Layer 1", doc.pixel_format())];
    let bytes = as_earlier_photocraft_resaved(&doc, &[0, 3, 4, 5], 3);
    let file = PsdFile::from_bytes(&bytes).unwrap();
    assert!(file.merged_has_alpha());
    assert!(!earlier_photocraft_spot_planes(&file));
    assert_eq!(ink(&crate::import("ps.psd", &bytes).unwrap().document), ink(&doc));
    let mut opaque = channels_document(SampleType::U8);
    opaque.channels.clear();
    let mut file = PsdFile::from_bytes(&as_earlier_photocraft_resaved(&opaque, &[0], 0)).unwrap();
    file.resources.retain(|r| r.id != crate::channel_map::DISPLAY_INFO);
    file.resources.push(ImageResource::new(
        crate::channel_map::DISPLAY_INFO,
        crate::channel_map::display_info(&channels_document(SampleType::U8).channels.iter().collect::<Vec<_>>(), false),
    ));
    assert_eq!(photoshop_identified_channels(&file), 0, "0 is the transparency's");
    assert!(!earlier_photocraft_spot_planes(&file), "no channel left unlisted");
}
