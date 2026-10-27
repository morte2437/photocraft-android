use super::*;

/// How much of a PSD conversion the merged composite weighs, in layer records: rendering it
/// (all layers composited) and encoding its planes cost about two full-canvas layers.
pub(super) const MERGED_WEIGHT: f32 = 2.0;

/// The part of the merged composite's share filled once its bands are rendered and converted; the
/// rest is compressing its planes.
pub(super) const MERGED_RENDERED: f32 = 0.8;

/// Adds headroom for compressed channel streams and their per-row length tables.
pub(super) fn estimate_encoded_size(raw_bytes: u64, rows: u64, structural_bytes: u64) -> Option<u64> {
    raw_bytes.checked_add(raw_bytes.checked_add(63)?.checked_div(64)?)?.checked_add(rows.checked_mul(8)?)?.checked_add(structural_bytes)
}

fn add_plane_estimate(raw_bytes: &mut u64, rows: &mut u64, width: u64, height: u64, channels: u64, bytes_per_sample: u64) -> Option<()> {
    let plane_bytes = width.checked_mul(height)?.checked_mul(channels)?.checked_mul(bytes_per_sample)?;
    let plane_rows = height.checked_mul(channels)?;
    *raw_bytes = raw_bytes.checked_add(plane_bytes)?;
    *rows = rows.checked_add(plane_rows)?;
    Some(())
}

pub(super) fn exceeds_psd_size_limit(estimated_size: Option<u64>) -> bool {
    estimated_size.is_none_or(|size| size > PSD_MAX_FILE_SIZE)
}

fn surface_dimensions(surface: &Surface) -> Option<(u64, u64)> {
    let bounds = surface.content_bounds();
    Some((u64::from(bounds.width()), u64::from(bounds.height())))
}

pub(crate) fn estimate_psd_size(doc: &Document) -> Option<u64> {
    let mut raw_bytes = 0;
    let mut rows = 0;
    let mut structural_bytes = PSD_ESTIMATE_FIXED_OVERHEAD;

    let width = u64::from(doc.size.width);
    let height = u64::from(doc.size.height);

    if doc.mode == ColorMode::Multichannel {
        let channels = u64::try_from(doc.channels.len().clamp(1, 56)).ok()?;
        let sample_bytes = u64::try_from(if doc.depth == SampleType::F32 { SampleType::U16.bytes() } else { doc.depth.bytes() }).ok()?;
        add_plane_estimate(&mut raw_bytes, &mut rows, width, height, channels, sample_bytes)?;
    } else {
        let format = doc.pixel_format();
        let color_channels = u64::try_from(format.mode.color_channels()).ok()?;
        let bytes_per_sample = u64::try_from(format.sample.bytes()).ok()?;
        let layer_channels = color_channels.checked_add(1)?;

        for (_, _, layer) in doc.walk() {
            structural_bytes = structural_bytes.checked_add(PSD_ESTIMATE_LAYER_OVERHEAD)?.checked_add(u64::try_from(layer.name.len()).ok()?)?;
            for (_, data) in &layer.psd_blocks {
                structural_bytes = structural_bytes.checked_add(u64::try_from(data.len()).ok()?)?;
            }

            let surface = match &layer.content {
                LayerContent::Fill(fill) => {
                    let mut fill_width = width;
                    let mut fill_height = height;
                    if let Some(cache) = &layer.fill_cache
                        && cache.fill == *fill
                    {
                        let (cache_width, cache_height) = surface_dimensions(&cache.surface)?;
                        fill_width = fill_width.max(cache_width);
                        fill_height = fill_height.max(cache_height);
                    }
                    add_plane_estimate(&mut raw_bytes, &mut rows, fill_width, fill_height, layer_channels, bytes_per_sample)?;
                    None
                }
                _ => layer.surface(),
            };
            if let Some(surface) = surface {
                let (surface_width, surface_height) = surface_dimensions(surface)?;
                add_plane_estimate(&mut raw_bytes, &mut rows, surface_width, surface_height, layer_channels, bytes_per_sample)?;
            }
            if let Some(mask) = &layer.mask {
                let (mask_width, mask_height) = surface_dimensions(&mask.surface)?;
                add_plane_estimate(&mut raw_bytes, &mut rows, mask_width, mask_height, 1, bytes_per_sample)?;
            }
            if let LayerContent::Smart(smart) = &layer.content
                && let photocraft_doc::SmartSource::Embedded { bytes, .. } = &smart.source
            {
                structural_bytes = structural_bytes.checked_add(u64::try_from(bytes.len()).ok()?)?;
            }
        }

        let max_extra_channels = 56usize.saturating_sub(format.mode.color_channels() + 1);
        let mut extra_channels = doc.channels.len().min(max_extra_channels);
        if doc.quick_mask.is_some() && extra_channels < max_extra_channels {
            extra_channels += 1;
        }
        let merged_channels = color_channels.checked_add(1)?.checked_add(u64::try_from(extra_channels).ok()?)?;
        add_plane_estimate(&mut raw_bytes, &mut rows, width, height, merged_channels, bytes_per_sample)?;
        for channel in doc.channels.iter().take(extra_channels) {
            structural_bytes = structural_bytes.checked_add(u64::try_from(channel.name.len()).ok()?)?;
        }
    }

    structural_bytes = structural_bytes
        .checked_add(u64::try_from(doc.icc_profile.as_ref().map_or(0, |profile| profile.len())).ok()?)?
        .checked_add(u64::try_from(doc.metadata.xmp.as_deref().map_or(0, str::len)).ok()?)?
        .checked_add(u64::try_from(doc.metadata.exif.as_ref().map_or(0, |data| data.len())).ok()?)?;
    for (_, name, data) in &doc.metadata.psd_resources {
        structural_bytes = structural_bytes.checked_add(u64::try_from(name.len()).ok()?)?.checked_add(u64::try_from(data.len()).ok()?)?;
    }
    for (_, _, data) in &doc.metadata.psd_global_blocks {
        structural_bytes = structural_bytes.checked_add(u64::try_from(data.len()).ok()?)?;
    }

    estimate_encoded_size(raw_bytes, rows, structural_bytes)
}

/// The signature a kept tagged block is written with. `8B64` exists only in PSB files: a PSD
/// with an `8B64` block (kept from a PSB source, e.g. its global `FMsk` or `cinf`) is refused by
/// Photoshop as "not compatible with this version" (measured in Photoshop 27.11). The other way
/// round, a PSB block with an 8-byte length is `8B64`, as Photoshop writes it: an `8BIM` `lnk2`,
/// `pths` or `cinf` (kept from a PSD source, whose lengths the PSB widens) makes Photoshop read
/// the length as 4 bytes and refuse the whole file (#7937).
pub(crate) fn block_signature(sig: [u8; 4], key: [u8; 4], version: Version) -> [u8; 4] {
    match version {
        Version::Psd if sig == *b"8B64" => *b"8BIM",
        Version::Psb if sig == *b"8BIM" && photocraft_psd::tagged::uses_long_length(version, &key) => *b"8B64",
        _ => sig,
    }
}

pub(crate) fn psd_version(doc: &Document, force_psb: bool, size_exceeds_limit: bool) -> Version {
    let exceeds_dimensions = doc.size.width > 30_000 || doc.size.height > 30_000;
    if force_psb || exceeds_dimensions || size_exceeds_limit { Version::Psb } else { Version::Psd }
}

pub(crate) fn psd_size_exceeds_limit(doc: &Document) -> bool {
    exceeds_psd_size_limit(estimate_psd_size(doc))
}
