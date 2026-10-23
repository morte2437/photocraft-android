//! Indexed Color and Bitmap documents -> PSD colour modes 2 and 0.
//!
//! Neither mode has layers in Photoshop, so the file holds only the merged image (the document
//! flattened): for Indexed Color one plane of palette indices, with the Color Table as Color Mode
//! Data (768 bytes: 256 reds, 256 greens, 256 blues), its length in resource 1046 and the
//! transparent entry in resource 1047; for Bitmap one plane of 1 bit per pixel, 1 = black.

use photocraft_color::ColorMode;
use photocraft_doc::{ColorTable, Document};
use photocraft_psd::resources::{ImageResource, ResolutionInfo, ids};
use photocraft_psd::{ColorMode as PsdMode, Compression, Header, ImageData, PsdFile};

/// Indexed Color Table Count: the number of colours in the table (u16).
pub(crate) const COLOR_TABLE_COUNT: u16 = 1046;
/// Transparency Index: the colour table entry shown as transparent (u16).
pub(crate) const TRANSPARENCY_INDEX: u16 = 1047;
/// Channel resources (names, display info, alpha identifiers): these modes have no extra channels.
const CHANNEL_RESOURCES: [u16; 4] = [1006, 1045, 1053, crate::channel_map::DISPLAY_INFO];

/// The PSD for an Indexed Color document with a colour table of at most 256 entries, one of them
/// opaque, or a Bitmap document; None for any other document (it is written like the others). A
/// document with alpha channels or a Quick Mask is written like the others too: the importer does
/// not read the extra channels of these modes back, so they would be lost.
pub(crate) fn document_to_psd(doc: &Document, force_psb: bool) -> Option<(PsdFile, Vec<String>)> {
    if !doc.channels.is_empty() || doc.quick_mask.is_some() {
        return None;
    }
    let table = doc.color_table.as_ref().filter(|t| t.colors.len() <= 256 && (0..t.colors.len()).any(|i| Some(i) != t.transparent.map(usize::from)));
    let (mode, plane, color_mode_data) = match doc.mode {
        ColorMode::Indexed => {
            let table = table?;
            (PsdMode::Indexed, indices(doc, table), palette(table))
        }
        ColorMode::Bitmap => (PsdMode::Bitmap, bits(doc), Vec::new()),
        _ => return None,
    };
    let mut warnings = Vec::new();
    if !flattens_losslessly(doc) {
        warnings.push(format!("{:?} documents have no layers in Photoshop: written flattened", doc.mode));
    }
    let big = doc.size.width > 30_000 || doc.size.height > 30_000;
    let size_exceeds_limit = !force_psb && !big && crate::psd_export::psd_size_exceeds_limit(doc);
    let version = crate::psd_export::psd_version(doc, force_psb, size_exceeds_limit);
    let depth = if mode == PsdMode::Bitmap { 1 } else { 8 };
    let header = Header::new(version, doc.size.width, doc.size.height, 1, depth, mode);
    let image_data = ImageData::encode(Compression::Rle, &plane, &header)
        .or_else(|_| ImageData::encode(Compression::Raw, &plane, &header))
        .unwrap_or(ImageData { compression: Compression::Raw, data: plane });
    let mut resources = vec![
        ImageResource::new(ids::RESOLUTION_INFO, ResolutionInfo::from_dpi(f64::from(doc.resolution_dpi)).to_bytes()),
        ImageResource::new(ids::GLOBAL_ANGLE, (doc.global_light.angle.round() as i32).to_be_bytes().to_vec()),
        ImageResource::new(ids::GLOBAL_ALTITUDE, (doc.global_light.altitude.round() as i32).to_be_bytes().to_vec()),
        ImageResource::new(1032, crate::psd_export::guides_resource(doc)),
    ];
    if let (PsdMode::Indexed, Some(t)) = (mode, table) {
        resources.push(ImageResource::new(COLOR_TABLE_COUNT, (t.colors.len() as u16).to_be_bytes().to_vec()));
        if let Some(i) = t.transparent {
            resources.push(ImageResource::new(TRANSPARENCY_INDEX, u16::from(i).to_be_bytes().to_vec()));
        }
        // Photoshop keeps the RGB profile of an Indexed document (Bitmap has none). A profile of
        // another colour space (kept from a Grayscale or CMYK source) does not describe the table.
        if let Some(icc) = doc.icc_profile.as_ref().filter(|icc| icc.get(16..20) == Some(b"RGB ".as_slice())) {
            resources.push(ImageResource::new(ids::ICC_PROFILE, icc.to_vec()));
        }
    }
    // An imported transparency index that no longer applies is not copied either.
    let keep = |id: u16, _: &[u8]| !CHANNEL_RESOURCES.contains(&id) && id != TRANSPARENCY_INDEX && id != COLOR_TABLE_COUNT;
    let file = crate::multichannel_map::layerless_file(doc, header, image_data, color_mode_data, resources, keep, &mut warnings);
    Some((file, warnings))
}

/// Whether the merged image holds everything the layers do: no layer, or a single visible pixel
/// layer at full opacity, Normal blend, unclipped, without masks or layer effects. Anything else
/// (more layers, a type, shape or smart object layer, a mask, a hidden layer) is lost when the
/// document is written flattened.
fn flattens_losslessly(doc: &Document) -> bool {
    match doc.layers.as_slice() {
        [] => true,
        [l] => {
            l.visible
                && matches!(l.content, photocraft_doc::LayerContent::Raster(_))
                && l.mask.is_none()
                && l.vector_mask.is_none()
                && !l.clipped
                && l.opacity >= 1.0
                && l.fill_opacity >= 1.0
                && l.blend == photocraft_color::BlendMode::Normal
                && !(l.effects.enabled && l.effects.items.iter().any(photocraft_doc::Effect::enabled))
        }
        _ => false,
    }
}

/// The Color Table as Color Mode Data: 256 reds, 256 greens, 256 blues (unused entries black).
fn palette(t: &ColorTable) -> Vec<u8> {
    let mut m = vec![0u8; 768];
    for (i, c) in t.colors.iter().take(256).enumerate() {
        m[i] = c[0];
        m[256 + i] = c[1];
        m[512 + i] = c[2];
    }
    m
}

/// The flattened document as palette indices: the transparent entry where the document is less
/// than half covered, else the nearest other entry (the transparent one may duplicate an opaque
/// colour, as in many PNG-8 files). Without a transparent entry the document is flattened over
/// white, as Photoshop does.
fn indices(doc: &Document, t: &ColorTable) -> Vec<u8> {
    let (opaque_indices, colors): (Vec<u8>, Vec<_>) =
        t.colors.iter().copied().enumerate().filter(|(i, _)| Some(*i) != t.transparent.map(usize::from)).map(|(i, c)| (i as u8, c)).unzip();
    let opaque = ColorTable { colors, transparent: None };
    let mut idx = Vec::with_capacity(doc.size.area() as usize);
    let _ = photocraft_compose::render_bands(doc, doc.bounds(), 0, |band| -> Result<(), ()> {
        idx.extend(band.px.iter().map(|p| match t.transparent {
            Some(i) if p[3] < 0.5 => i,
            Some(_) => opaque_indices.get(opaque.nearest([p[0], p[1], p[2]])).copied().unwrap_or(0),
            None => {
                let over_white = |c: f32| c * p[3] + (1.0 - p[3]);
                t.nearest([over_white(p[0]), over_white(p[1]), over_white(p[2])]) as u8
            }
        }));
        Ok(())
    });
    idx
}

/// The flattened document over white as 1-bit rows (padded to a byte), 1 = black: pixels darker
/// than middle grey.
fn bits(doc: &Document) -> Vec<u8> {
    let w = doc.size.width as usize;
    let row = w.div_ceil(8);
    let mut out = Vec::with_capacity(row * doc.size.height as usize);
    let _ = photocraft_compose::render_bands(doc, doc.bounds(), 0, |band| -> Result<(), ()> {
        for line in band.px.chunks(w.max(1)) {
            let mut bytes = vec![0u8; row];
            for (x, p) in line.iter().enumerate() {
                let grey = (p[0] + p[1] + p[2]) / 3.0 * p[3] + (1.0 - p[3]);
                if grey < 0.5 {
                    bytes[x / 8] |= 0x80 >> (x % 8);
                }
            }
            out.extend(bytes);
        }
        Ok(())
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_color::{Color, SampleType};
    use photocraft_doc::Size;

    fn document(width: u32, height: u32, depth: SampleType, mode: ColorMode, pixels: &[f32]) -> Document {
        let mut doc = Document::with_background("palette test", Size::new(width, height), ColorMode::Rgb, depth, Color::rgba(0.0, 0.0, 0.0, 1.0));
        let bounds = doc.bounds();
        doc.layers[0].surface_mut().unwrap().write_region(bounds, pixels);
        doc.mode = mode;
        doc
    }

    fn round_trip(doc: &Document) -> (PsdFile, Document) {
        let (file, _) = document_to_psd(doc, false).unwrap();
        let bytes = file.to_bytes().unwrap();
        let file = PsdFile::from_bytes(&bytes).unwrap();
        let back = crate::import("test.psd", &bytes).unwrap().document;
        (file, back)
    }

    #[test]
    fn full_palette_endpoints_round_trip_with_either_transparent_endpoint() {
        let colors: Vec<_> = (0..=255).map(|v| [v, 255 - v, v / 2]).collect();
        for transparent in [0, 255] {
            let opaque = 255 - transparent;
            let rgb = colors[usize::from(opaque)].map(|v| f32::from(v) / 255.0);
            let mut doc = document(2, 1, SampleType::U8, ColorMode::Indexed, &[rgb[0], rgb[1], rgb[2], 1.0, 0.0, 0.0, 0.0, 0.0]);
            doc.color_table = Some(ColorTable { colors: colors.clone(), transparent: Some(transparent) });
            let (file, back) = round_trip(&doc);
            assert_eq!((file.header.color_mode, file.header.depth, file.header.channels), (PsdMode::Indexed, 8, 1));
            assert_eq!(file.decode_merged().unwrap(), [opaque, transparent]);
            assert_eq!(file.resource(COLOR_TABLE_COUNT).unwrap().data, [1, 0]);
            assert_eq!(file.color_mode_data.len(), 768);
            for (i, c) in colors.iter().enumerate() {
                assert_eq!([file.color_mode_data[i], file.color_mode_data[256 + i], file.color_mode_data[512 + i]], *c);
            }
            assert_eq!(back.color_table, doc.color_table);
            let surface = back.layers[0].surface().unwrap();
            assert_eq!(surface.pixel(0, 0), vec![rgb[0], rgb[1], rgb[2], 1.0]);
            assert_eq!(surface.pixel(1, 0)[3], 0.0);
        }
    }

    #[test]
    fn indexed_alpha_threshold_uses_stored_samples_at_all_depths() {
        for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
            let mut doc = document(3, 1, depth, ColorMode::Indexed, &[0.0, 0.0, 0.0, 0.49, 0.0, 0.0, 0.0, 0.5, 0.0, 0.0, 0.0, 0.51]);
            doc.color_table = Some(ColorTable { colors: vec![[0, 0, 0], [255, 255, 255]], transparent: Some(1) });
            let (file, back) = round_trip(&doc);
            assert_eq!(file.decode_merged().unwrap(), [1, 0, 0], "{depth:?}");
            let surface = back.layers[0].surface().unwrap();
            assert_eq!((0..3).map(|x| surface.pixel(x, 0)[3]).collect::<Vec<_>>(), [0.0, 1.0, 1.0]);
        }
    }

    #[test]
    fn small_palette_zero_pads_unused_entries_and_rejects_invalid_lengths() {
        let mut doc = document(1, 1, SampleType::U8, ColorMode::Indexed, &[1.0, 0.0, 0.0, 1.0]);
        doc.color_table = Some(ColorTable { colors: vec![[255, 0, 0]], transparent: None });
        let (file, back) = round_trip(&doc);
        assert_eq!(file.color_mode_data[0], 255);
        assert!(file.color_mode_data[1..].iter().all(|&v| v == 0));
        assert_eq!(back.color_table, doc.color_table);
        for count in [0, 257] {
            doc.color_table = Some(ColorTable { colors: vec![[0, 0, 0]; count], transparent: None });
            assert!(document_to_psd(&doc, false).is_none(), "{count}");
        }
        doc.color_table = None;
        assert!(document_to_psd(&doc, false).is_none());
    }

    #[test]
    fn indexed_import_bounds_malformed_count_and_transparency_resources() {
        let mut doc = document(1, 1, SampleType::U8, ColorMode::Indexed, &[0.0, 0.0, 0.0, 1.0]);
        doc.color_table = Some(ColorTable { colors: vec![[0, 0, 0], [255, 255, 255]], transparent: None });
        let (original, _) = document_to_psd(&doc, false).unwrap();
        for (count, expected) in [(vec![0, 0], 1), (vec![1, 1], 256), (vec![255, 255], 256), (vec![0], 256)] {
            let mut file = original.clone();
            file.resources.retain(|r| r.id != COLOR_TABLE_COUNT);
            file.resources.push(ImageResource::new(COLOR_TABLE_COUNT, count));
            file.resources.push(ImageResource::new(TRANSPARENCY_INDEX, vec![255, 255]));
            let bytes = file.to_bytes().unwrap();
            let back = crate::import("malformed.psd", &bytes).unwrap().document;
            let table = back.color_table.unwrap();
            assert_eq!(table.colors.len(), expected);
            assert_eq!(table.transparent, None);
            assert_eq!(back.layers[0].surface().unwrap().pixel(0, 0)[3], 1.0);
        }
    }

    #[test]
    fn bitmap_rows_are_msb_first_and_independently_byte_padded() {
        for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
            for width in [1u32, 7, 8, 9, 15, 17] {
                // Opposite rows expose padding carried into the next row or reversed bit order.
                let pixels: Vec<_> = (0..2)
                    .flat_map(|y| {
                        (0..width).flat_map(move |x| {
                            let v = if (x + y) % 2 == 0 { 0.0 } else { 1.0 };
                            [v, v, v, 1.0]
                        })
                    })
                    .collect();
                let doc = document(width, 2, depth, ColorMode::Bitmap, &pixels);
                let (file, back) = round_trip(&doc);
                assert_eq!((file.header.color_mode, file.header.depth, file.header.channels), (PsdMode::Bitmap, 1, 1));
                let mut expected = Vec::new();
                for y in 0..2 {
                    for byte in 0..width.div_ceil(8) {
                        let mut packed = 0;
                        for bit in 0..8 {
                            let x = byte * 8 + bit;
                            if x < width && (x + y) % 2 == 0 {
                                packed |= 0x80 >> bit;
                            }
                        }
                        expected.push(packed);
                    }
                }
                assert_eq!(file.decode_merged().unwrap(), expected, "{depth:?}, width={width}");
                assert!(file.color_mode_data.is_empty());
                assert_eq!(back.mode, ColorMode::Bitmap);
                let gray_alpha: Vec<_> = pixels.as_chunks::<4>().0.iter().flat_map(|p| [p[0], p[3]]).collect();
                assert_eq!(back.layers[0].surface().unwrap().read_region(back.bounds()), gray_alpha);
            }
        }
    }

    #[test]
    fn bitmap_alpha_is_flattened_over_white_before_thresholding() {
        for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
            let doc = document(3, 1, depth, ColorMode::Bitmap, &[0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.49, 0.0, 0.0, 0.0, 0.51]);
            let (file, back) = round_trip(&doc);
            assert_eq!(file.decode_merged().unwrap(), [0x20], "{depth:?}");
            assert_eq!(back.layers[0].surface().unwrap().read_region(back.bounds()), [1.0, 1.0, 1.0, 1.0, 0.0, 1.0]);
        }
    }
}
