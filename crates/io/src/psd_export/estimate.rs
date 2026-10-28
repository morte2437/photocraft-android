use super::{Document, PSD_SAVE_HEADROOM};
use photocraft_psd::{PsdFile, Version, file::LayerInfoPlacement};

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

/// How much of a PSD conversion the merged composite weighs, in layer records.
pub(super) const MERGED_WEIGHT: f32 = 2.0;

/// The part of the merged composite's share filled once its bands are rendered and converted.
pub(super) const MERGED_RENDERED: f32 = 0.8;

/// The version a document is encoded as: PSB when asked for or past PSD's 30000 px; otherwise
/// PSD, which [`fit_psd_size`] turns into PSB once the encoded file proves too large.
pub(crate) fn psd_version(doc: &Document, force_psb: bool) -> Version {
    let exceeds_dimensions = doc.size.width > 30_000 || doc.size.height > 30_000;
    if force_psb || exceeds_dimensions { Version::Psb } else { Version::Psd }
}

/// Promote files that exceed the PSD limit, accounting for the preview appended by the save
/// (#212, #5285). The decision is made on the exact length of the file as it will be written,
/// not on the raw pixel size: RLE usually packs the channels far below it.
pub(crate) fn fit_psd_size(file: &mut PsdFile, limit: u64) -> Option<String> {
    if file.written_version() == Version::Psb {
        return None;
    }
    if file.encoded_len().is_ok_and(|len| len.saturating_add(PSD_SAVE_HEADROOM) <= limit) {
        return None;
    }
    match file.promote_to_psb() {
        Ok(()) => {
            let padded = file.layer_info.as_ref().is_some_and(|info| info.padding.is_some());
            if matches!(file.layer_info_placement, LayerInfoPlacement::Section)
                && padded
                && file.pad_layer_info_to(4).is_err()
                && let Some(info) = &mut file.layer_info
            {
                info.padding = None;
            }
            Some("encoded size exceeds 2 GB; written as PSB".into())
        }
        Err(error) => Some(format!("encoded size exceeds 2 GB, but the file could not be written as PSB: {error}")),
    }
}

/// Signs every global block for the version the file is written as.
pub(crate) fn sign_global_blocks(file: &mut PsdFile) {
    let version = file.written_version();
    for block in &mut file.global_blocks {
        block.signature = block_signature(block.signature, block.key, version);
    }
}
