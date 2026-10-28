use super::*;

fn document(width: u32, height: u32) -> Document {
    Document::new("size estimate", photocraft_geom::Size::new(width, height), ColorMode::Rgb, SampleType::U8)
}

/// Photoshop opens a PSD's bottom layer as its Background only when that layer has no
/// transparency channel (measured on Photoshop 27.11: with the channel written, re-exported
/// files reopened with the Background as an ordinary layer).
#[test]
fn a_background_is_written_without_a_transparency_channel() {
    use photocraft_color::Color;
    let size = photocraft_geom::Size::new(8, 6);
    let mut doc = Document::with_background("bg", size, ColorMode::Rgb, SampleType::U8, Color::rgb(0.2, 0.4, 0.6));
    doc.layers.push(Layer::raster("Layer 1", doc.pixel_format()));
    let file = document_to_psd(&doc);
    let layers = &file.layer_info.as_ref().unwrap().layers;
    let ids = |i: usize| layers[i].channels.iter().map(|c| c.id).collect::<Vec<_>>();
    assert_eq!(ids(0), vec![0, 1, 2], "the Background has no -1 channel");
    assert_eq!(layers[0].flags.0, LayerFlags::BIT4_USEFUL | LayerFlags::TRANSPARENCY_PROTECTED, "as Photoshop writes it");
    assert!(ids(1).contains(&-1), "other layers keep their transparency");
    // It comes back as the same locked, opaque Background.
    let back = crate::import("bg.psd", &file.to_bytes().unwrap()).unwrap().document;
    let bg = &back.layers[0];
    assert!(bg.locks.transparency && bg.locks.position, "{:?}", bg.locks);
    assert_eq!(bg.surface().unwrap().read_region(back.bounds()), doc.layers[0].surface().unwrap().read_region(doc.bounds()));
    // A bottom layer with transparency (or unlocked) is not a Background.
    let mut holey = doc.clone();
    holey.layers[0].surface_mut().unwrap().write_pixel(2, 2, &[0.0, 0.0, 0.0, 0.0]);
    assert!(document_to_psd(&holey).layer_info.unwrap().layers[0].channels.iter().any(|c| c.id == -1));
    let mut unlocked = doc.clone();
    unlocked.layers[0].locks.position = false;
    assert!(document_to_psd(&unlocked).layer_info.unwrap().layers[0].channels.iter().any(|c| c.id == -1));
}

/// A flat 1-bit Bitmap file (no layers) opens in Photoshop as its Background; so does it in
/// PhotoCraft, and it is written back as one (no transparency channel).
#[test]
fn a_flat_bitmap_file_opens_and_saves_as_a_background() {
    let file = photocraft_psd::testgen::merged_only(Version::Psd, PsdMode::Bitmap, 1, Compression::Raw, 8, 4);
    let doc = crate::import("flat.psd", &file.to_bytes().unwrap()).unwrap().document;
    let bg = &doc.layers[0];
    assert!(bg.name == "Background" && bg.locks.transparency && bg.locks.position, "{:?}", bg.locks);
    // Bitmap has no layers in Photoshop (#4814): the file is saved flat in Bitmap mode, without
    // a layer section, and opens as a Background again.
    let out = document_to_psd(&doc);
    assert_eq!(out.header.color_mode, PsdMode::Bitmap);
    assert!(out.layer_info.as_ref().is_none_or(|li| li.layers.is_empty()));
    let back = crate::import("flat.psd", &out.to_bytes().unwrap()).unwrap().document;
    let bg = &back.layers[0];
    assert!(back.layers.len() == 1 && bg.name == "Background" && bg.locks.transparency && bg.locks.position, "{:?}", bg.locks);
}

/// A layered document with a spot and an alpha channel. Each channel holds 100 %, 25 %, 75 %
/// and 0 % in four columns, the layout of the Photoshop spot oracles.
fn spot_and_alpha_document() -> Document {
    let mut doc = document(4, 1);
    doc.layers.push(Layer::group("Group", vec![Layer::raster("Layer 1", doc.pixel_format())]));
    let channel = |name: &str| {
        let mut s = Surface::new(PixelFormat::GRAY8);
        s.write_interleaved(doc.bounds(), &[255, 64, 191, 0]);
        photocraft_doc::AlphaChannel::new(name, s)
    };
    let mut spot = channel("Spot Color 1");
    spot.spot = Some((photocraft_doc::Color::rgb(0.0, 0.6, 1.0), 0.0));
    let alpha = channel("Alpha 1");
    doc.channels.extend([spot, alpha]);
    doc
}

/// The extra planes after the colour channels (and the merged transparency) of `file`.
fn extra_planes(file: &PsdFile, count: usize) -> Vec<u8> {
    let planes = file.decode_merged().unwrap();
    let plane = (file.header.width * file.header.height) as usize * usize::from(file.header.depth / 8);
    planes[planes.len() - count * plane..].to_vec()
}

/// Resource 1053's identifiers.
fn alpha_ids(file: &PsdFile) -> Option<Vec<u32>> {
    file.resource(crate::channel_map::ALPHA_IDENTIFIERS).map(|r| r.data.as_chunks::<4>().0.iter().map(|c| u32::from_be_bytes(*c)).collect())
}

/// A PSD stores spot ink as Photoshop's own PSDs do (0 = solid ink: the 100, 25, 75 and 0 %
/// columns are 0, 191, 64, 255) and an alpha channel straight (255 = selected). The global
/// layer mask stays empty and flag bit 3 stays on the layers only, not on the group's closing
/// divider: Photoshop opens those as they are, and 509 of the corpus's 721 Photoshop PSDs have
/// an empty mask too. Opening the file gives the same ink back.
#[test]
fn psd_stores_spot_ink_as_photoshop_does() {
    let doc = spot_and_alpha_document();
    let file = PsdFile::from_bytes(&document_to_psd(&doc).to_bytes().unwrap()).unwrap();
    assert_eq!(extra_planes(&file, 2), [0, 191, 64, 255, 255, 64, 191, 0], "spot, then alpha");
    assert_eq!(file.global_layer_mask.as_ref().map(|m| m.data.len()), Some(0));
    let flags: Vec<u8> = file.layers().iter().map(|l| l.flags.0 & LayerFlags::BIT4_USEFUL).collect();
    assert_eq!(flags, [0, LayerFlags::BIT4_USEFUL, LayerFlags::BIT4_USEFUL], "closing divider, layer, group");
    let back = crate::import("spot.psd", &file.to_bytes().unwrap()).unwrap().document;
    assert!(back.channels[0].spot.is_some() && back.channels[1].spot.is_none());
    for c in &back.channels {
        assert_eq!(c.surface.to_interleaved(back.bounds()), [255, 64, 191, 0], "{}", c.name);
    }
}

/// Resource 1053 numbers the extra channels as Photoshop 27.11 does in a new document: 3, 4, 5
/// in every colour mode, after 0 for the merged transparency when there is one (its
/// transparent PSD lists `0, 3, 4, 5`). A Quick Mask is an extra channel too. Without extra
/// channels there is none.
#[test]
fn psd_alpha_identifiers_number_the_channels_as_photoshop_does() {
    use photocraft_color::Color;
    let size = photocraft_geom::Size::new(4, 1);
    for mode in [ColorMode::Rgb, ColorMode::Cmyk, ColorMode::Grayscale, ColorMode::Lab] {
        let mut doc = Document::with_background("bg", size, mode, SampleType::U8, Color::rgb(1.0, 1.0, 1.0));
        assert_eq!(alpha_ids(&document_to_psd(&doc)), None, "{mode:?} without channels");
        doc.channels = spot_and_alpha_document().channels;
        let file = document_to_psd(&doc);
        assert!(!file.merged_has_alpha(), "{mode:?}");
        assert_eq!(alpha_ids(&file), Some(vec![3, 4]), "{mode:?}");
        doc.quick_mask = Some(photocraft_doc::AlphaChannel::new("Quick Mask", Surface::new(PixelFormat::GRAY8)));
        assert_eq!(alpha_ids(&document_to_psd(&doc)), Some(vec![3, 4, 5]), "{mode:?} with a Quick Mask");
    }
    // The empty layer leaves the composite transparent.
    let file = document_to_psd(&spot_and_alpha_document());
    assert!(file.merged_has_alpha());
    assert_eq!(alpha_ids(&file), Some(vec![0, 3, 4]));
}

/// A save writes one fresh 1053 for the channels the document has. A Photoshop file's own is
/// not kept on open (it would be new metadata after every round trip), and one a document
/// carries anyway (kept by an earlier version) is never written: next to the fresh one it
/// would be a second, and without channels it would list channels that are gone. The
/// identifiers here, 3 and 2, are those of psd-tools' `cmyk-alpha-spot.psd` (Photoshop 24.7).
#[test]
fn a_kept_alpha_identifiers_resource_is_not_written() {
    let kept = || (crate::channel_map::ALPHA_IDENTIFIERS, String::new(), std::sync::Arc::new([0, 0, 0, 3, 0, 0, 0, 2].to_vec()));
    let mut file = document_to_psd(&spot_and_alpha_document());
    file.resources.retain(|r| r.id != crate::channel_map::ALPHA_IDENTIFIERS);
    file.resources.push(ImageResource::new(kept().0, kept().2.to_vec()));
    let mut opened = crate::import("ps.psd", &file.to_bytes().unwrap()).unwrap().document;
    assert!(!opened.metadata.psd_resources.iter().any(|(id, ..)| *id == crate::channel_map::ALPHA_IDENTIFIERS));
    opened.metadata.psd_resources.push(kept());
    let again = PsdFile::from_bytes(&crate::export(&opened, "x.psd", &crate::ExportOptions::default()).unwrap().bytes).unwrap();
    assert_eq!(again.resources.iter().filter(|r| r.id == crate::channel_map::ALPHA_IDENTIFIERS).count(), 1);
    assert_eq!(alpha_ids(&again), Some(vec![0, 3, 4]));
    opened.channels.clear();
    assert_eq!(alpha_ids(&document_to_psd(&opened)), None, "no channels left");
}

#[test]
fn group_divider_id_wraps_without_colliding_with_imported_layer_ids() {
    let mut doc = document(8, 8);
    let mut group = Layer::group("Imported group", vec![Layer::raster("Child", doc.pixel_format())]);
    group.psd_id = Some(u32::MAX);
    doc.layers.push(group);

    let file = document_to_psd(&doc);
    let ids: Vec<u32> = file.layers().iter().filter_map(|layer| layer.layer_id()).collect();
    let unique: std::collections::HashSet<u32> = ids.iter().copied().collect();
    assert_eq!(ids.len(), 3, "group, child and closing divider");
    assert_eq!(unique.len(), ids.len(), "no generated ID may collide");
    assert!(unique.contains(&u32::MAX), "preserve the imported group ID");
    assert!(unique.contains(&1), "reuse the first unused positive ID on wrap");

    let encoded = file.to_bytes().unwrap();
    let parsed = PsdFile::from_bytes(&encoded).unwrap();
    let reloaded_ids: std::collections::HashSet<u32> = parsed.layers().iter().filter_map(|layer| layer.layer_id()).collect();
    assert_eq!(unique, reloaded_ids);
}

/// #5068: Photoshop won't open a file whose `Lr32` comes before the `Mt32` marker it keeps for
/// a 32-bit document's merged transparency (verified in Photoshop 2026 on psd-tools'
/// `transparentbg.psd` re-saved both ways). The layer block follows a kept marker.
#[test]
fn layer_block_follows_a_kept_merged_transparency_marker() {
    for (depth, key, marker) in [
        (SampleType::F32, *b"Lr32", *b"Mt32"),
        (SampleType::U16, *b"Lr16", *b"Mt16"),
        (SampleType::F32, *b"Lr32", *b"Mtrn"),
        (SampleType::F32, *b"Lr32", *b"MTrn"),
    ] {
        let mut doc = Document::new("t", photocraft_geom::Size::new(8, 6), ColorMode::Rgb, depth);
        let mut s = Surface::new(doc.pixel_format());
        s.fill_rect(photocraft_geom::Rect::new(1, 1, 5, 4), &[0.5, 0.2, 0.9, 1.0]);
        doc.layers.push(Layer::new("Layer 1", LayerContent::Raster(s)));
        doc.metadata.psd_global_blocks = vec![(*b"8BIM", *b"Patt", std::sync::Arc::new(Vec::new())), (*b"8BIM", marker, std::sync::Arc::new(Vec::new()))];
        let saved = PsdFile::from_bytes(&document_to_psd(&doc).to_bytes().unwrap()).unwrap();
        let LayerInfoPlacement::GlobalBlock { index, key: written, .. } = saved.layer_info_placement else { panic!("{depth:?}: a global layer block") };
        assert_eq!(written, key);
        let at = saved.global_blocks.iter().position(|b| b.key == marker).expect("the marker is kept");
        assert!(at < index, "{depth:?}: {marker:?} at {at}, the layer block at {index}");
    }
    // Without a marker the layer block stays first.
    let mut doc = Document::new("t", photocraft_geom::Size::new(8, 6), ColorMode::Rgb, SampleType::F32);
    doc.layers.push(Layer::new("Layer 1", LayerContent::Raster(Surface::new(doc.pixel_format()))));
    doc.metadata.psd_global_blocks = vec![(*b"8BIM", *b"Patt", std::sync::Arc::new(Vec::new()))];
    let saved = PsdFile::from_bytes(&document_to_psd(&doc).to_bytes().unwrap()).unwrap();
    assert!(matches!(saved.layer_info_placement, LayerInfoPlacement::GlobalBlock { index: 0, .. }));
}

/// A merged-transparency marker kept from the opened file describes that file's composite.
/// Saved with an opaque composite, it would make the first alpha channel read back as the
/// merged transparency, and the channel would be gone: the save drops it.
#[test]
fn an_opaque_save_drops_a_kept_merged_transparency_marker() {
    for (depth, marker) in [(SampleType::U8, *b"MTrn"), (SampleType::U8, *b"Mtrn"), (SampleType::U16, *b"Mt16"), (SampleType::F32, *b"Mt32")] {
        let case = format!("{depth:?} {}", String::from_utf8_lossy(&marker));
        let mut doc = Document::new("t", photocraft_geom::Size::new(8, 6), ColorMode::Rgb, depth);
        let mut s = Surface::new(doc.pixel_format());
        s.fill_rect(photocraft_geom::Rect::new(0, 0, 8, 6), &[0.5, 0.2, 0.9, 1.0]);
        doc.layers.push(Layer::new("Layer 1", LayerContent::Raster(s)));
        doc.channels.push(photocraft_doc::AlphaChannel::new("Alpha 1", Surface::new(PixelFormat::GRAY8)));
        doc.metadata.psd_global_blocks = vec![(*b"8BIM", marker, std::sync::Arc::new(Vec::new()))];
        let file = document_to_psd(&doc);
        assert!(!file.global_blocks.iter().any(TaggedBlock::is_merged_transparency_marker), "{case}");
        assert!(!file.merged_has_alpha(), "{case}");
        let back = crate::import("a.psd", &file.to_bytes().unwrap()).unwrap().document;
        let names: Vec<&str> = back.channels.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["Alpha 1"], "{case}");
    }
}

#[test]
fn unreadable_lr16_warns_on_import_and_is_not_saved_twice() {
    // #2700: a 16-bit file whose Lr16 block claims five layers but holds none.
    let mut file = photocraft_psd::testgen::merged_only(Version::Psd, PsdMode::Rgb, 16, Compression::Raw, 8, 6);
    file.global_layer_mask = Some(GlobalLayerMask::default());
    file.global_blocks.push(TaggedBlock::new(*b"Lr16", vec![0, 5, 0, 0]));
    let imported = crate::import("broken.psd", &file.to_bytes().unwrap()).unwrap();
    assert!(imported.warnings.iter().any(|w| w.contains("Lr16") && w.contains("could not be read")), "{:?}", imported.warnings);

    let saved = PsdFile::from_bytes(&document_to_psd(&imported.document).to_bytes().unwrap()).unwrap();
    let ours = matches!(saved.layer_info_placement, LayerInfoPlacement::GlobalBlock { key, .. } if key == *b"Lr16");
    let kept = saved.global_blocks.iter().filter(|b| b.key == *b"Lr16").count();
    assert_eq!((ours, kept), (true, 0), "exactly one Lr16 block: the document's own layers");
    assert_eq!(saved.layers().len(), 1);
}

#[test]
fn photo_filter_psd_is_four_byte_aligned_and_round_trips() {
    let color = [60000u16, 30000, 0].map(|x| x as f32 / 65535.0);
    let adjustment = photocraft_doc::Adjustment::PhotoFilter { color, density: 0.14, preserve_luminosity: true };
    let mut doc =
        Document::with_background("Photo Filter PSD", photocraft_geom::Size::new(32, 24), ColorMode::Rgb, SampleType::U8, photocraft_doc::Color::WHITE);
    let mut layer = Layer::new("Warming Filter", LayerContent::Adjustment(adjustment.clone()));
    doc.layers.push(layer.clone());

    let assert_phfl = |file: &PsdFile| {
        let records = &file.layer_info.as_ref().unwrap().layers;
        let data = records.iter().flat_map(|r| r.blocks.iter()).find(|b| b.key == *b"phfl").map(|b| b.data.as_slice()).expect("Photo Filter adjustment data");
        assert_eq!(data.len(), 20, "PSD layer block length includes four-byte padding");
        assert_eq!(&data[0..2], &[0, 2], "version 2 RGB Photo Filter");
        assert_eq!(&data[17..], &[0, 0, 0], "three padding bytes after the 17-byte payload");
    };

    let exported = document_to_psd(&doc);
    assert_phfl(&exported);
    let encoded = exported.to_bytes().unwrap();
    let parsed = PsdFile::from_bytes(&encoded).unwrap();
    assert_phfl(&parsed);
    let (restored, _) = crate::psd_import::psd_to_document(&parsed);
    assert!(restored.layers.iter().any(|l| l.content == LayerContent::Adjustment(adjustment.clone())));

    // Importing a legacy 18-byte block must still work; saving it again should repair it
    // even when the adjustment values have not changed (normally raw blocks are reused).
    let mut legacy = crate::adjust_map::write(&adjustment)[0].1.clone();
    legacy.truncate(18);
    layer.psd_blocks = vec![(*b"phfl", std::sync::Arc::new(legacy))];
    doc.layers.pop();
    doc.layers.push(layer);
    let repaired = document_to_psd(&doc);
    assert_phfl(&repaired);
    let repaired_bytes = repaired.to_bytes().unwrap();
    assert_phfl(&PsdFile::from_bytes(&repaired_bytes).unwrap());
}

#[test]
fn photo_filter_version_3_is_kept_until_edited() {
    // The #3777 block (Photoshop CS2, XYZ colour): saved back byte-exact while unchanged.
    let v3 = vec![0, 3, 0, 0, 0x5e, 0x7f, 0, 0, 0x61, 0xaa, 0, 0, 0x0e, 0xf3, 0, 0, 0, 0x3f, 1, 0];
    let adjustment = crate::adjust_map::parse(b"phfl", &v3, None, crate::adjust_map::Channels::Rgb);
    let photocraft_doc::Adjustment::PhotoFilter { color, .. } = adjustment else { panic!("{adjustment:?}") };
    let mut doc = Document::with_background("Photo Filter v3", photocraft_geom::Size::new(8, 8), ColorMode::Rgb, SampleType::U8, photocraft_doc::Color::WHITE);
    let mut layer = Layer::new("Photo Filter 1", LayerContent::Adjustment(adjustment.clone()));
    layer.psd_blocks = vec![(*b"phfl", std::sync::Arc::new(v3.clone()))];
    doc.layers.push(layer);
    let phfl = |doc: &Document| {
        let file = PsdFile::from_bytes(&document_to_psd(doc).to_bytes().unwrap()).unwrap();
        let records = &file.layer_info.as_ref().unwrap().layers;
        records.iter().flat_map(|r| r.blocks.iter()).find(|b| b.key == *b"phfl").map(|b| b.data.clone()).expect("Photo Filter adjustment data")
    };
    assert_eq!(phfl(&doc), v3);
    // Once edited it is written as version 2 (RGB), and the colour survives.
    let edited = photocraft_doc::Adjustment::PhotoFilter { color, density: 0.5, preserve_luminosity: true };
    if let Some(l) = doc.layers.last_mut() {
        l.content = LayerContent::Adjustment(edited.clone());
    }
    let saved = phfl(&doc);
    assert_eq!(&saved[..2], &[0, 2]);
    let reread = crate::adjust_map::parse(b"phfl", &saved, None, crate::adjust_map::Channels::Rgb);
    let photocraft_doc::Adjustment::PhotoFilter { color: back, density, .. } = reread else { panic!("{reread:?}") };
    assert!(back.iter().zip(color).all(|(b, c)| (b - c).abs() <= 1.0 / 65535.0), "{back:?} vs {color:?}");
    assert_eq!(density, 0.5);
}

#[test]
fn light_adjustment_is_kept_verbatim_and_warned() {
    // #6542: Photoshop 2026's Light layer is a `brit` block whose `CgEd` descriptor carries
    // more than Brightness/Contrast. Synthetic block: the extra key names are placeholders.
    use photocraft_psd::descriptor::{Descriptor, Value, VersionedDescriptor};
    let brit = vec![0, 0, 0xff, 0xfe, 0, 127, 0, 0];
    let d = Descriptor::new("null")
        .with("Vrsn", Value::Integer(1))
        .with("Brgh", Value::Integer(0))
        .with("Cntr", Value::Integer(-2))
        .with("Exps", Value::Double(-2.3))
        .with("Hghl", Value::Integer(-64))
        .with("useLegacy", Value::Boolean(false));
    let cged = VersionedDescriptor::new(d).to_bytes();
    let adjustment = crate::adjust_map::parse(b"brit", &brit, Some(&cged), crate::adjust_map::Channels::Rgb);
    assert_eq!(adjustment, photocraft_doc::Adjustment::Unsupported { psd_key: "brit".into(), raw: brit.clone().into() });
    let mut doc = Document::with_background("Light", photocraft_geom::Size::new(8, 8), ColorMode::Rgb, SampleType::U8, photocraft_doc::Color::WHITE);
    let mut layer = Layer::new("Light 1", LayerContent::Adjustment(adjustment));
    layer.psd_blocks = vec![(*b"brit", std::sync::Arc::new(brit.clone())), (*b"CgEd", std::sync::Arc::new(cged.clone()))];
    doc.layers.push(layer);
    let block = |file: &PsdFile, key: &[u8; 4]| {
        let records = &file.layer_info.as_ref().unwrap().layers;
        records.iter().flat_map(|r| r.blocks.iter()).find(|b| b.key == *key).map(|b| b.data.clone())
    };
    let file = PsdFile::from_bytes(&document_to_psd(&doc).to_bytes().unwrap()).unwrap();
    let (restored, warnings) = crate::psd_import::psd_to_document(&file);
    assert!(warnings.iter().any(|w| w.contains("\"Light 1\"") && w.contains("Light settings")), "{warnings:?}");
    // Saved again unchanged, both blocks are written back byte for byte.
    let again = PsdFile::from_bytes(&document_to_psd(&restored).to_bytes().unwrap()).unwrap();
    assert_eq!(block(&again, b"brit").as_deref(), Some(&brit[..]));
    assert_eq!(block(&again, b"CgEd").as_deref(), Some(&cged[..]));
    // A plain Brightness/Contrast descriptor still reads as Brightness/Contrast.
    let plain = crate::adjust_map::write(&photocraft_doc::Adjustment::BrightnessContrast { brightness: 5.0, contrast: -2.0, legacy: false });
    assert!(!crate::adjust_map::is_extended_brit(Some(&plain[1].1)));
}

/// A 1024² RGB document with one full-canvas layer: flat colour (packs to a few KB) or noise
/// (RLE can't pack it). Its raw planes (layer RGBA + merged RGB) are 7 MiB either way.
fn sized_document(noise: bool, depth: SampleType) -> Document {
    const SIDE: u32 = 1024;
    let mut doc = Document::new("size limit", photocraft_geom::Size::new(SIDE, SIDE), ColorMode::Rgb, depth);
    let mut layer = Layer::raster("Layer 1", doc.pixel_format());
    let rect = photocraft_geom::Rect::new(0, 0, SIDE as i32, SIDE as i32);
    let surface = layer.surface_mut().unwrap();
    if noise {
        let mut state = 0x2545_f491_u32;
        let data: Vec<f32> = (0..SIDE * SIDE * 4)
            .map(|i| {
                state ^= state << 13;
                state ^= state >> 17;
                state ^= state << 5;
                if i % 4 == 3 { 1.0 } else { (state >> 24) as f32 / 255.0 }
            })
            .collect();
        surface.write_region(rect, &data);
    } else {
        surface.fill_rect(rect, &[0.2, 0.4, 0.6, 1.0]);
    }
    doc.layers.push(layer);
    doc
}

const SIZED_RAW_BYTES: u64 = 1024 * 1024 * (4 + 3);
/// Under the raw size, so only the encoded size can keep a document PSD.
const SIZED_LIMIT: u64 = 4 * 1024 * 1024;
const _: () = assert!(SIZED_RAW_BYTES > SIZED_LIMIT);

fn export_sized(doc: &Document, limit: u64) -> (PsdFile, Vec<String>) {
    document_to_psd_sized(doc, &PsdExportOptions::default(), 0, &Interrupt::NONE, limit)
}

/// #5285: a document whose raw pixels are over the PSD limit but whose encoded file fits is
/// saved as PSD (the raw-size estimate used to make it a PSB).
#[test]
fn a_file_that_fits_after_compression_stays_psd() {
    let doc = sized_document(false, SampleType::U8);
    let (file, warnings) = export_sized(&doc, SIZED_LIMIT);
    assert_eq!(file.written_version(), Version::Psd);
    assert!(warnings.iter().all(|w| !w.contains("PSB")), "{warnings:?}");
    let bytes = file.to_bytes().unwrap();
    assert!(bytes.len() as u64 + PSD_SAVE_HEADROOM <= SIZED_LIMIT, "{} bytes", bytes.len());
    let (back, _) = crate::psd_to_document(&PsdFile::from_bytes(&bytes).unwrap());
    assert_eq!(back.layers[0].surface().unwrap().pixel(700, 300), doc.layers[0].surface().unwrap().pixel(700, 300));
}

/// #5285: a document whose encoded file is over the limit becomes a PSB, byte for byte the
/// one a forced PSB save writes, and reads back. 8 bits keep the layers in the layer section,
/// 16 bits in a global `Lr16` block.
#[test]
fn a_file_over_the_limit_after_compression_is_psb() {
    for depth in [SampleType::U8, SampleType::U16] {
        let doc = sized_document(true, depth);
        let (file, warnings) = export_sized(&doc, SIZED_LIMIT);
        assert_eq!(file.written_version(), Version::Psb, "{depth:?}");
        assert!(warnings.iter().any(|w| w == "encoded size exceeds 2 GB; written as PSB"), "{warnings:?}");
        let bytes = file.to_bytes().unwrap();
        assert!(bytes.len() as u64 > SIZED_LIMIT);
        let forced = document_to_psd_with(&doc, &PsdExportOptions { force_psb: true, ..Default::default() }).0;
        let forced = forced.to_bytes().unwrap();
        assert!(bytes == forced, "{depth:?}: the promoted PSB differs from a PSB written directly");
        assert_eq!(PsdFile::from_bytes(&bytes).unwrap(), PsdFile::from_bytes(&forced).unwrap(), "{depth:?}");
        // The promoted model itself stays a consistent PSD for every reader.
        assert_eq!(file.header.version, Version::Psd);
        assert_eq!(crate::psd_to_document(&file).0.layers[0].surface().unwrap().pixel(511, 777), doc.layers[0].surface().unwrap().pixel(511, 777));
        let (back, _) = crate::psd_to_document(&PsdFile::from_bytes(&bytes).unwrap());
        for (x, y) in [(0, 0), (1023, 0), (511, 777), (1023, 1023)] {
            assert_eq!(back.layers[0].surface().unwrap().pixel(x, y), doc.layers[0].surface().unwrap().pixel(x, y), "{depth:?} ({x}, {y})");
        }
    }
}

/// The limit is the exact written length plus the save's headroom: one byte less is a PSB.
#[test]
fn the_size_limit_is_the_exact_encoded_length() {
    let doc = sized_document(false, SampleType::U8);
    let len = export_sized(&doc, u64::MAX).0.to_bytes().unwrap().len() as u64;
    assert_eq!(export_sized(&doc, len + PSD_SAVE_HEADROOM).0.written_version(), Version::Psd);
    assert_eq!(export_sized(&doc, len + PSD_SAVE_HEADROOM - 1).0.written_version(), Version::Psb);
    // The real limit: a small document stays PSD, a forced one is PSB.
    let small = document(1, 1);
    assert_eq!(document_to_psd(&small).written_version(), Version::Psd);
    assert_eq!(document_to_psd_with(&small, &PsdExportOptions { force_psb: true, ..Default::default() }).0.written_version(), Version::Psb);
    assert_eq!(psd_version(&document(30_001, 1), false), Version::Psb);
}

/// A Multichannel document follows the same rule.
#[test]
fn a_multichannel_file_over_the_limit_is_psb() {
    let mut doc = document(64, 64);
    doc.mode = ColorMode::Multichannel;
    doc.channels.push(photocraft_doc::AlphaChannel::new("Cyan", photocraft_raster::Surface::new(PixelFormat::GRAY8)));
    let (file, _) = export_sized(&doc, u64::MAX);
    assert_eq!(file.written_version(), Version::Psd);
    let len = file.to_bytes().unwrap().len() as u64;
    let (file, warnings) = export_sized(&doc, len + PSD_SAVE_HEADROOM - 1);
    assert_eq!(file.written_version(), Version::Psb);
    assert!(warnings.iter().any(|w| w.contains("written as PSB")), "{warnings:?}");
    let back = PsdFile::from_bytes(&file.to_bytes().unwrap()).unwrap();
    assert_eq!(back.header.version, Version::Psb);
    assert_eq!(back.decode_merged().unwrap(), PsdFile::from_bytes(&export_sized(&doc, u64::MAX).0.to_bytes().unwrap()).unwrap().decode_merged().unwrap());
}

/// Indexed Color and Bitmap (written without layers, #4816), Multichannel and RGB follow the
/// encoded-size rule, and a file turned into PSB by its size writes its global blocks with
/// 8-byte lengths as `8B64` (#7937), whatever signature they were kept with; a file that fits
/// stays a PSD with `8BIM` throughout.
#[test]
fn every_mode_promoted_by_size_signs_long_blocks_8b64() {
    let white = photocraft_color::Color::rgba(1.0, 1.0, 1.0, 1.0);
    let mut docs = Vec::new();
    for (mode, psd_mode) in [(ColorMode::Indexed, PsdMode::Indexed), (ColorMode::Bitmap, PsdMode::Bitmap), (ColorMode::Rgb, PsdMode::Rgb)] {
        let mut doc = Document::with_background("modes", photocraft_geom::Size::new(64, 64), ColorMode::Rgb, SampleType::U8, white);
        doc.mode = mode;
        doc.color_table = Some(photocraft_doc::ColorTable { colors: vec![[255, 255, 255], [0, 0, 0]], transparent: None });
        docs.push((doc, psd_mode));
    }
    let mut multichannel = document(64, 64);
    multichannel.mode = ColorMode::Multichannel;
    multichannel.channels.push(photocraft_doc::AlphaChannel::new("Cyan", photocraft_raster::Surface::new(PixelFormat::GRAY8)));
    docs.push((multichannel, PsdMode::Multichannel));
    for (mut doc, psd_mode) in docs {
        doc.metadata.psd_global_blocks = vec![
            (*b"8BIM", *b"cinf", std::sync::Arc::new(vec![0, 0, 0, 16, 0, 0, 0, 0])),
            (*b"8BIM", *b"pths", std::sync::Arc::new(vec![0, 0, 0, 16, 0, 0, 0, 0])),
            (*b"8B64", *b"FMsk", std::sync::Arc::new(vec![0xff, 0xff, 0, 0, 0, 0, 0, 0, 0, 0, 0, 50])),
            (*b"8BIM", *b"Patt", std::sync::Arc::new(Vec::new())),
        ];
        let written = |limit| {
            let (file, warnings) = export_sized(&doc, limit);
            let bytes = file.to_bytes().unwrap();
            let back = PsdFile::from_bytes(&bytes).unwrap();
            let blocks = back.global_blocks.iter().map(|b| (b.signature, b.key)).collect::<Vec<_>>();
            (bytes.len() as u64, back, blocks, warnings)
        };
        let (len, small, blocks, _) = written(u64::MAX);
        assert_eq!((small.header.version, small.header.color_mode), (Version::Psd, psd_mode), "{psd_mode:?}");
        assert_eq!(blocks, vec![(*b"8BIM", *b"cinf"), (*b"8BIM", *b"pths"), (*b"8BIM", *b"FMsk"), (*b"8BIM", *b"Patt")], "{psd_mode:?}: a PSD keeps 8BIM");
        // One byte short of the encoded length: the same document is written as PSB.
        let (_, big, blocks, warnings) = written(len + PSD_SAVE_HEADROOM - 1);
        assert_eq!((big.header.version, big.header.color_mode), (Version::Psb, psd_mode), "{psd_mode:?}: {warnings:?}");
        assert!(warnings.iter().any(|w| w == "encoded size exceeds 2 GB; written as PSB"), "{psd_mode:?}: {warnings:?}");
        assert_eq!(blocks, vec![(*b"8B64", *b"cinf"), (*b"8B64", *b"pths"), (*b"8B64", *b"FMsk"), (*b"8BIM", *b"Patt")], "{psd_mode:?}: a PSB signs 8B64");
        assert_eq!(big.decode_merged().unwrap(), small.decode_merged().unwrap(), "{psd_mode:?}");
        // Without `size_limit_psb` no mode is promoted by its size, the layerless ones included.
        let opts = PsdExportOptions { size_limit_psb: false, ..PsdExportOptions::default() };
        let (kept, warnings) = document_to_psd_sized(&doc, &opts, 0, &Interrupt::NONE, len + PSD_SAVE_HEADROOM - 1);
        assert_eq!((kept.written_version(), kept.header.color_mode), (Version::Psd, psd_mode), "{psd_mode:?}: {warnings:?}");
        assert!(!warnings.iter().any(|w| w.contains("written as PSB")), "{psd_mode:?}: {warnings:?}");
    }
}

#[test]
fn kept_psb_blocks_are_written_as_8bim_in_a_psd() {
    // Photoshop 27.11 refuses a PSD holding `8B64` blocks ("not compatible with this
    // version"); they came from re-saving a PSB as PSD with its global FMsk and cinf kept.
    let mut doc = document(4, 4);
    doc.metadata.psd_global_blocks =
        vec![(*b"8B64", *b"FMsk", std::sync::Arc::new(vec![0xff, 0xff, 0, 0, 0, 0, 0, 0, 0, 0, 0, 50])), (*b"8BIM", *b"Patt", std::sync::Arc::new(Vec::new()))];
    let signatures = |force_psb| {
        let opts = PsdExportOptions { force_psb, ..PsdExportOptions::default() };
        let bytes = document_to_psd_with(&doc, &opts).0.to_bytes().unwrap();
        let file = PsdFile::from_bytes(&bytes).unwrap();
        file.global_blocks.iter().map(|b| (b.signature, b.key)).collect::<Vec<_>>()
    };
    assert_eq!(signatures(false), vec![(*b"8BIM", *b"FMsk"), (*b"8BIM", *b"Patt")]);
    assert_eq!(signatures(true), vec![(*b"8B64", *b"FMsk"), (*b"8BIM", *b"Patt")], "a PSB keeps 8B64");
}

/// A PSB writes every global block with an 8-byte length as `8B64`, the way Photoshop does:
/// kept from a PSD source, `lnk2`, `pths` and `cinf` arrive as `8BIM`, and with that signature
/// and the PSB's 8-byte length Photoshop 27.11 refused the whole file (#7937). Blocks with a
/// 4-byte length keep `8BIM`, and a PSD keeps `8BIM` throughout.
#[test]
fn psb_writes_long_length_blocks_as_8b64() {
    let mut doc = document(4, 4);
    doc.metadata.psd_global_blocks = vec![
        (*b"8BIM", *b"cinf", std::sync::Arc::new(vec![0, 0, 0, 16, 0, 0, 0, 0])),
        (*b"8BIM", *b"pths", std::sync::Arc::new(vec![0, 0, 0, 16, 0, 0, 0, 0])),
        (*b"8BIM", *b"Patt", std::sync::Arc::new(Vec::new())),
    ];
    let written = |force_psb| {
        let opts = PsdExportOptions { force_psb, ..PsdExportOptions::default() };
        let bytes = document_to_psd_with(&doc, &opts).0.to_bytes().unwrap();
        let file = PsdFile::from_bytes(&bytes).unwrap();
        let blocks = file.global_blocks.iter().map(|b| (b.signature, b.key, b.data.len())).collect::<Vec<_>>();
        (bytes, blocks)
    };
    let (bytes, blocks) = written(true);
    assert_eq!(blocks, vec![(*b"8B64", *b"cinf", 8), (*b"8B64", *b"pths", 8), (*b"8BIM", *b"Patt", 0)]);
    // On disk: the signature, the key, then the 8-byte length.
    let at = bytes.windows(8).position(|w| w == b"8B64cinf").expect("8B64cinf in the file");
    assert_eq!(bytes.get(at + 8..at + 16), Some(&[0u8, 0, 0, 0, 0, 0, 0, 8][..]));
    assert!(!bytes.windows(8).any(|w| w == b"8BIMcinf" || w == b"8BIMpths"));
    let (_, blocks) = written(false);
    assert_eq!(blocks, vec![(*b"8BIM", *b"cinf", 8), (*b"8BIM", *b"pths", 8), (*b"8BIM", *b"Patt", 0)], "a PSD keeps 8BIM");
}

/// A layered document with transparency in the merged image and an alpha channel: the name
/// and DisplayInfo resources hold one entry per extra channel, the merged transparency first,
/// as in every file Photoshop opens (one short, Photoshop 27.11 refused the file with "Could
/// not import the clipboard because an unexpected end-of-file was encountered"). The alpha
/// channel keeps its own name and options on the way back.
#[test]
fn alpha_names_count_the_merged_transparency() {
    let mut doc = document(8, 6);
    let mut layer = Layer::raster("Layer 1", doc.pixel_format());
    layer.surface_mut().unwrap().write_pixel(1, 1, &[1.0, 0.0, 0.0, 1.0]);
    doc.layers.push(layer);
    let mut a = photocraft_doc::AlphaChannel::new("Alpha 1", photocraft_raster::Surface::new(PixelFormat::GRAY8));
    a.opacity = 0.5;
    doc.channels.push(a);
    let file = document_to_psd(&doc);
    let header_extra = usize::from(file.header.channels) - 3;
    let names = |id| file.resources.iter().find(|r| r.id == id).map(|r| r.data.clone()).unwrap();
    let pascal = names(1006);
    let (mut count, mut i) = (0, 0);
    while i < pascal.len() {
        count += 1;
        i += 1 + usize::from(pascal[i]);
    }
    assert_eq!((header_extra, count), (2, 2), "transparency + Alpha 1");
    assert_eq!(names(crate::channel_map::DISPLAY_INFO).len(), 4 + 2 * 13);
    let back = crate::import("a.psd", &file.to_bytes().unwrap()).unwrap().document;
    assert_eq!(back.channels.len(), 1);
    assert_eq!(back.channels[0].name, "Alpha 1");
    assert!((back.channels[0].opacity - 0.5).abs() < 1e-6);
}
