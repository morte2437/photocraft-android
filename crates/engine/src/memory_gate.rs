//! Refuses a big operation that cannot fit in memory, with an error, before it starts (#10024).
//!
//! On the web build a failed allocation aborts the whole module and the page freezes without a
//! message. [`photocraft_raster::headroom`] tells whether a given number of bytes can still be
//! allocated; an operation that is about to allocate a lot asks here first, with a rough upper
//! bound of what it will need. A refusal changes nothing (the document, its history and the
//! selection stay as they were) and says what the user can do. Off the web every check passes.
//!
//! The estimates are deliberately rough and err high: they count the big buffers an operation
//! holds at once, not every temporary. A layered PSD/PSB adds a canvas-sized surface per layer
//! to the flat-image estimate; one whose header or sections can't be read, or that isn't 8-bit
//! RGB or grayscale, has no safe bound and is refused by the web gate before decoding.

use photocraft_doc::{Document, LayerContent, SmartSource};
use photocraft_geom::TILE_SIZE;

use crate::{EngineError, Result};

/// Peak bytes per source pixel while a smart object is rendered from its source: the decoded
/// source, its composite, the converted surface and the transformed result, measured on a large
/// embedded file. Lower it when the decoded originals stop being kept alongside (#10030).
pub const SMART_RENDER_BYTES_PER_PX: u64 = 41;

/// Bytes per pixel of the compositor's working buffer (`RGBA`, `f32`) that flattening a placed
/// file fills.
const COMPOSITE_BYTES_PER_PX: u64 = 16;

/// Pixels assumed per byte of a file whose size can't be read from its header (a rough bound: a
/// compressed image holds a few pixels per byte).
const FALLBACK_PX_PER_FILE_BYTE: u64 = 4;

/// Bytes per canvas pixel each layer of a layered PSD/PSB keeps once decoded, on top of the flat
/// estimate: an 8-bit RGBA surface and its mask, assumed to cover the whole canvas.
const PSD_LAYER_BYTES_PER_PX: u64 = 5;

/// `Ok` when `need` bytes can be allocated; otherwise an error naming `what` (for example
/// `place “x.png”`), the need, and what the user can do. The document is not touched.
pub fn ensure(what: &str, need: u64) -> Result<()> {
    check(what, need, "Save your work, close other documents or use Edit › Purge › All, then try again.")
}

/// [`ensure`] for saving and exporting: the user must be able to keep their work, so the advice
/// leads with freeing memory. The document stays open and unsaved.
pub fn ensure_for_save(what: &str, need: u64) -> Result<()> {
    check(what, need, "Use Edit › Purge › All or close other documents to free memory, then save again. The document is still open and unchanged.")
}

fn check(what: &str, need: u64, advice: &str) -> Result<()> {
    photocraft_raster::headroom::ensure(need).map_err(|_| {
        if need == u64::MAX {
            // Freeing memory cannot satisfy an unbounded need, so the retry advice would mislead.
            EngineError::Other(format!(
                "Not enough memory to safely {what} in the browser: the file has a format whose memory use cannot be bounded in advance. Open it in the desktop app, or flatten it to a plain 8-bit RGB or grayscale image first."
            ))
        } else {
            EngineError::Other(format!("Not enough memory to {what} (it needs about {} MB). {advice}", mb(need)))
        }
    })
}

/// `bytes` in whole megabytes, rounded up, with thousands separators.
fn mb(bytes: u64) -> String {
    let n = bytes.div_ceil(1 << 20).to_string();
    let mut out = String::with_capacity(n.len() + n.len() / 3);
    for (i, c) in n.chars().enumerate() {
        if i > 0 && (n.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// Pixel count of an image file read from its header, without decoding it. Formats without a
/// header reader here fall back to a size-based guess, so the answer is always an upper-bound
/// style estimate rather than an error.
pub(crate) fn source_pixels(bytes: &[u8]) -> u64 {
    header_pixels(bytes).unwrap_or_else(|| (bytes.len() as u64).saturating_mul(FALLBACK_PX_PER_FILE_BYTE))
}

fn header_pixels(bytes: &[u8]) -> Option<u64> {
    if let Ok((w, h)) = photocraft_codecs::probe_size(bytes) {
        return Some(u64::from(w) * u64::from(h));
    }
    let be32 = |at: usize| -> Option<u64> { Some(u64::from(u32::from_be_bytes(bytes.get(at..at.checked_add(4)?)?.try_into().ok()?))) };
    let be16 = |at: usize| -> Option<u64> { Some(u64::from(u16::from_be_bytes(bytes.get(at..at.checked_add(2)?)?.try_into().ok()?))) };
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        // IHDR: width, height.
        return Some(be32(16)?.saturating_mul(be32(20)?));
    }
    if bytes.starts_with(b"8BPS") {
        // PSD / PSB header: height then width.
        return Some(be32(14)?.saturating_mul(be32(18)?));
    }
    if bytes.starts_with(&[0xff, 0xd8]) {
        // JPEG: the first start-of-frame marker holds height then width.
        let mut at = 2usize;
        while at.checked_add(4)? <= bytes.len() {
            if *bytes.get(at)? != 0xff {
                at += 1;
                continue;
            }
            let marker = *bytes.get(at + 1)?;
            match marker {
                0xff => at += 1,
                0xc0..=0xcf if !matches!(marker, 0xc4 | 0xc8 | 0xcc) => return Some(be16(at + 5)?.saturating_mul(be16(at + 7)?)),
                0xd0..=0xd9 | 0x01 | 0x00 => at += 2,
                _ => at = at.checked_add(2)?.checked_add(usize::try_from(be16(at + 2)?).ok()?)?,
            }
        }
    }
    None
}

/// The number of layers a PSD/PSB file decodes to, read from its sections without decoding: 0
/// for a flat file. The color-mode data and image resources (resolution, ICC profile, thumbnail
/// and the like) hold no layer pixels and are only skipped. `None` when the header or a section
/// can't be read, or the file isn't 8-bit RGB or grayscale with at most 4 channels: those have
/// no safe bound.
fn psd_layer_count(bytes: &[u8]) -> Option<u64> {
    let header = bytes.get(..26)?;
    let version = u16::from_be_bytes([header[4], header[5]]);
    let channels = u16::from_be_bytes([header[12], header[13]]);
    let depth = u16::from_be_bytes([header[22], header[23]]);
    let mode = u16::from_be_bytes([header[24], header[25]]);
    if !matches!(version, 1 | 2) || !(1..=4).contains(&channels) || depth != 8 || !matches!(mode, 1 | 3) {
        return None;
    }
    // A big-endian length field of `width` bytes at `at`.
    let length = |at: usize, width: usize| -> Option<usize> {
        let field = bytes.get(at..at.checked_add(width)?)?;
        usize::try_from(field.iter().fold(0u64, |n, &b| (n << 8) | u64::from(b))).ok()
    };
    let mut at = 26usize;
    for _ in 0..2 {
        // Color-mode data, then image resources.
        at = at.checked_add(4)?.checked_add(length(at, 4)?)?;
    }
    let width = if version == 1 { 4 } else { 8 };
    let layers_and_masks = length(at, width)?;
    let start = at.checked_add(width)?;
    // The merged image's compression field follows the section.
    if bytes.len().saturating_sub(start.checked_add(layers_and_masks)?) < 2 {
        return None;
    }
    if layers_and_masks == 0 {
        return Some(0);
    }
    // The layer info opens with its own length, then the layer count (negative when the first
    // alpha channel holds the merged image's transparency).
    let layer_info = length(start, width)?;
    if layer_info == 0 {
        // Some writers keep the layers in a `Layr` tagged block instead, which isn't counted here.
        let section = bytes.get(start..start.checked_add(layers_and_masks)?)?;
        return if section.windows(8).any(|w| w == b"8BIMLayr" || w == b"8B64Layr") { None } else { Some(0) };
    }
    if layer_info.checked_add(width)? > layers_and_masks {
        return None;
    }
    let count_at = start.checked_add(width)?;
    let count = <[u8; 2]>::try_from(bytes.get(count_at..count_at.checked_add(2)?)?).ok()?;
    Some(u64::from(i16::from_be_bytes(count).unsigned_abs()))
}

/// Bytes per source pixel a file keeps beyond the flat estimate: a canvas-sized surface per layer
/// of a layered PSD/PSB, nothing for other files. `None` for a PSD/PSB with no safe bound, which
/// the web gate then refuses before decoding; native behavior is unchanged.
fn layer_bytes_per_px(bytes: &[u8]) -> Option<u64> {
    if !bytes.starts_with(b"8BPS") {
        return Some(0);
    }
    Some(psd_layer_count(bytes)?.saturating_mul(PSD_LAYER_BYTES_PER_PX))
}

/// Place: the file's bytes, plus the decoded file, its composite and the placed pixels, which
/// are alive together (`bytes_per_px` is the document's pixel size, at least 4), plus the layers
/// of a layered PSD/PSB.
pub(crate) fn place_need(bytes: &[u8], bytes_per_px: usize) -> u64 {
    let Some(layers) = layer_bytes_per_px(bytes) else { return u64::MAX };
    let px = source_pixels(bytes);
    let per_px = (bytes_per_px.max(4) as u64).saturating_mul(3).saturating_add(COMPOSITE_BYTES_PER_PX).saturating_add(layers);
    (bytes.len() as u64).saturating_add(px.saturating_mul(per_px))
}

/// A smart object's re-render from `bytes`: its source pixels at the measured peak, plus the
/// layers of a layered PSD/PSB.
pub(crate) fn smart_render_need(bytes: &[u8]) -> u64 {
    let Some(layers) = layer_bytes_per_px(bytes) else { return u64::MAX };
    source_pixels(bytes).saturating_mul(SMART_RENDER_BYTES_PER_PX.saturating_add(layers))
}

/// Image Size to `nw` x `nh`: the resampled pixel surfaces and masks (each layer's tiles scaled
/// by the change of area, capped at a full canvas) in the document's pixel size, plus every
/// distinct embedded smart object re-rendered from its source at once.
pub(crate) fn image_size_need(doc: &Document, nw: u32, nh: u32) -> u64 {
    let bpp = doc.pixel_format().bytes_per_pixel().max(1) as u64;
    let old_px = (u64::from(doc.size.width) * u64::from(doc.size.height)).max(1);
    let new_px = u64::from(nw) * u64::from(nh);
    let canvas_bytes = new_px.saturating_mul(bpp);
    let tile_bytes = (TILE_SIZE as u64) * (TILE_SIZE as u64) * bpp;
    let resized = |tiles: usize| -> u64 {
        let old = (tiles as u64).saturating_mul(tile_bytes);
        let scaled = (old as u128 * u128::from(new_px) / u128::from(old_px)).min(u128::from(u64::MAX)) as u64;
        scaled.max(tile_bytes.min(canvas_bytes)).min(canvas_bytes)
    };
    let mut need = 0u64;
    let mut seen = std::collections::HashSet::new();
    for (_, _, layer) in doc.walk() {
        if let Some(s) = layer.surface() {
            need = need.saturating_add(resized(s.tile_count()));
        }
        if let Some(m) = &layer.mask {
            need = need.saturating_add(resized(m.surface.tile_count()));
        }
        if let LayerContent::Smart(so) = &layer.content {
            let embedded = match &so.source {
                SmartSource::Embedded { bytes, .. } => Some(bytes.as_slice()),
                SmartSource::Linked { .. } => None,
            };
            if let Some(bytes) = embedded
                && seen.insert(so.contents_id)
            {
                need = need.saturating_add(smart_render_need(bytes));
            }
        }
    }
    need
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn megabytes_are_rounded_up_and_grouped() {
        assert_eq!(mb(0), "0");
        assert_eq!(mb(1), "1");
        assert_eq!(mb(1240 << 20), "1,240");
        assert_eq!(mb(1_000 << 20), "1,000");
        assert_eq!(mb(12_345_678 << 20), "12,345,678");
    }

    #[test]
    fn headers_give_pixel_counts() {
        let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
        png.extend_from_slice(&300u32.to_be_bytes());
        png.extend_from_slice(&200u32.to_be_bytes());
        assert_eq!(source_pixels(&png), 60_000);
        let mut psd = b"8BPS\0\x01\0\0\0\0\0\0\0\x03".to_vec();
        psd.extend_from_slice(&50u32.to_be_bytes());
        psd.extend_from_slice(&40u32.to_be_bytes());
        assert_eq!(source_pixels(&psd), 2_000);
        // SOI, an APP0 segment of length 4, then SOF0: precision, height 20, width 30.
        let jpeg = [0xff, 0xd8, 0xff, 0xe0, 0, 4, 0, 0, 0xff, 0xc0, 0, 11, 8, 0, 20, 0, 30, 1];
        assert_eq!(source_pixels(&jpeg), 600);
    }

    #[test]
    fn malformed_psd_sections_are_refused_without_panicking() {
        let mut flat = b"8BPS\0\x01\0\0\0\0\0\0\0\x03\0\0\0\x02\0\0\0\x02\0\x08\0\x03".to_vec();
        flat.extend_from_slice(&[0; 14]);
        assert_eq!(psd_layer_count(&flat), Some(0));
        for end in 4..flat.len() {
            assert_eq!(place_need(&flat[..end], 4), u64::MAX);
            assert_eq!(smart_render_need(&flat[..end]), u64::MAX);
        }
        flat[26..30].copy_from_slice(&u32::MAX.to_be_bytes());
        assert_eq!(place_need(&flat, 4), u64::MAX);
    }

    /// A 2 x 2 RGB PSD with a resolution resource (as every Photoshop file has) and, when
    /// `layers` is set, a layer info section holding only that layer count.
    fn psd_with_resources(layers: Option<i16>) -> Vec<u8> {
        let mut psd = b"8BPS\0\x01\0\0\0\0\0\0\0\x03\0\0\0\x02\0\0\0\x02\0\x08\0\x03".to_vec();
        psd.extend_from_slice(&[0; 4]);
        // ResolutionInfo (0x03ED): signature, id, empty name padded to even, 16 bytes of data.
        let mut resource = b"8BIM\x03\xed\0\0".to_vec();
        resource.extend_from_slice(&16u32.to_be_bytes());
        resource.extend_from_slice(&[0, 0x48, 0, 0, 0, 1, 0, 1, 0, 0x48, 0, 0, 0, 1, 0, 1]);
        psd.extend_from_slice(&(resource.len() as u32).to_be_bytes());
        psd.extend_from_slice(&resource);
        match layers {
            None => psd.extend_from_slice(&[0; 4]),
            Some(n) => {
                psd.extend_from_slice(&6u32.to_be_bytes());
                psd.extend_from_slice(&2u32.to_be_bytes());
                psd.extend_from_slice(&n.to_be_bytes());
            }
        }
        psd.extend_from_slice(&[0; 2]);
        psd.extend_from_slice(&[128; 12]);
        psd
    }

    #[test]
    fn image_resources_do_not_make_a_psd_unbounded() {
        let psd = psd_with_resources(None);
        assert_eq!(psd_layer_count(&psd), Some(0));
        assert_eq!(place_need(&psd, 4), psd.len() as u64 + 4 * (4 * 3 + COMPOSITE_BYTES_PER_PX));
        assert_eq!(smart_render_need(&psd), 4 * SMART_RENDER_BYTES_PER_PX);
    }

    #[test]
    fn a_layered_psd_counts_a_canvas_sized_surface_per_layer() {
        // A negative count (merged transparency in the first alpha channel) is still 3 layers.
        for n in [3i16, -3] {
            let psd = psd_with_resources(Some(n));
            assert_eq!(psd_layer_count(&psd), Some(3), "{n}");
            assert_eq!(place_need(&psd, 4), psd.len() as u64 + 4 * (4 * 3 + COMPOSITE_BYTES_PER_PX + 3 * PSD_LAYER_BYTES_PER_PX));
            assert_eq!(smart_render_need(&psd), 4 * (SMART_RENDER_BYTES_PER_PX + 3 * PSD_LAYER_BYTES_PER_PX));
        }
        // A layer info length that runs past its section can't be trusted.
        let mut psd = psd_with_resources(Some(3));
        let info_at = psd.len() - 14 - 6;
        psd[info_at..info_at + 4].copy_from_slice(&100u32.to_be_bytes());
        assert_eq!(place_need(&psd, 4), u64::MAX);
        // Layers kept in a tagged block instead of the layer info aren't counted, so refused.
        let mut tagged = b"8BPS\0\x01\0\0\0\0\0\0\0\x03\0\0\0\x02\0\0\0\x02\0\x08\0\x03".to_vec();
        tagged.extend_from_slice(&[0; 8]);
        tagged.extend_from_slice(&20u32.to_be_bytes());
        tagged.extend_from_slice(&[0; 8]);
        tagged.extend_from_slice(b"8BIMLayr");
        tagged.extend_from_slice(&[0; 4 + 2 + 12]);
        assert_eq!(place_need(&tagged, 4), u64::MAX);
        // Cut anywhere before the merged image's compression field, it's refused.
        let psd = psd_with_resources(Some(3));
        for end in 4..psd.len() - 12 {
            assert_eq!(place_need(&psd[..end], 4), u64::MAX, "{end}");
        }
    }

    #[test]
    fn unreadable_headers_fall_back_to_a_size_guess_without_panicking() {
        assert_eq!(source_pixels(b""), 0);
        assert_eq!(source_pixels(&[0u8; 100]), 400);
        for cut in 0..40 {
            let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR\0\0\0\x10\0\0\0\x10".to_vec();
            png.truncate(cut);
            let _ = source_pixels(&png);
            let jpeg = [0xff, 0xd8, 0xff, 0xe0, 0, 4, 0, 0, 0xff, 0xc0, 0, 11, 8, 0, 20, 0, 30, 1];
            let _ = source_pixels(&jpeg[..cut.min(jpeg.len())]);
        }
        // A segment length that runs off the end of the file ends the scan.
        let _ = source_pixels(&[0xff, 0xd8, 0xff, 0xe0, 0xff, 0xff]);
    }
}
