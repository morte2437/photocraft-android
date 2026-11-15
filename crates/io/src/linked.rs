//! Embedded smart-object files from the PSD global `lnk2`/`lnk3`/`lnkD` blocks.
//!
//! PSD import keeps a placed layer's source as `SmartSource::Linked { path: <Idnt uuid> }` and the
//! file bytes stay inside the preserved global block (so PSD export stays byte-stable). This module
//! finds the bytes for a uuid when the engine needs to re-render the smart object from its source.
//!
//! Layout (Adobe PSD spec, "Linked Layer"): a sequence of items, each `u64 length` + item data,
//! padded to 4 bytes. Item: type (`liFD` data / `liFE` external / `liFA` alias), version (1–7),
//! Pascal uuid, Unicode file name, file type, creator, `u64` data length, open-descriptor flag (+
//! versioned descriptor), then for `liFD` the raw file bytes.

use photocraft_doc::Metadata;

/// An embedded file found in a linked-layer block.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinkedFile {
    pub uuid: String,
    pub file_name: String,
    pub bytes: Vec<u8>,
}

/// A selected embedded item's decoded header and borrowed bytes.
fn item_data(item: &[u8]) -> Option<(String, String, &[u8])> {
    let header = craft_psd::smart::embedded_file_header(item).ok()?;
    let bytes = header.payload(item)?;
    Some((header.uuid, header.name, bytes))
}

/// Embedded filename without requiring the declared payload to be resident.
pub fn embedded_file_name(meta: &Metadata, uuid: &str) -> Option<String> {
    if uuid.is_empty() {
        return None;
    }
    meta.psd_global_blocks
        .iter()
        .filter(|(_, k, _)| matches!(k, b"lnk2" | b"lnk3" | b"lnkD"))
        .flat_map(|(_, _, data)| craft_psd::smart::iter_linked_records(data))
        .filter_map(|record| craft_psd::smart::embedded_file_header(record.item()?).ok())
        .find(|header| header.uuid == uuid)
        .map(|header| header.name)
}

/// Every complete embedded file; malformed inner headers do not hide later records.
pub fn parse_linked_files(data: &[u8]) -> Vec<LinkedFile> {
    craft_psd::smart::iter_linked_records(data)
        .filter_map(|record| {
            let (uuid, file_name, bytes) = item_data(record.item()?)?;
            Some(LinkedFile { uuid, file_name, bytes: bytes.to_vec() })
        })
        .collect()
}

/// The embedded file for smart-object `uuid` among the document's preserved PSD global blocks.
pub fn find_linked_file(meta: &Metadata, uuid: &str) -> Option<LinkedFile> {
    find_in_blocks(meta.psd_global_blocks.iter().map(|(_, k, d)| (k, d.as_slice())), uuid)
}

/// Borrow only the selected embedded payload, without copying other smart objects or reading disk.
pub fn embedded_payload<'a>(meta: &'a Metadata, uuid: &str) -> Option<(String, &'a [u8])> {
    embedded_payload_in_blocks(meta.psd_global_blocks.iter().map(|(_, key, data)| (key, data.as_slice())), uuid)
}

/// Borrow one selected payload from PSD global blocks without copying unrelated files.
pub fn embedded_payload_in_blocks<'a>(blocks: impl IntoIterator<Item = (&'a [u8; 4], &'a [u8])>, uuid: &str) -> Option<(String, &'a [u8])> {
    let (_, name, bytes) = find_slice_in_blocks(blocks, uuid)?;
    Some((name, bytes))
}

/// The embedded file for smart-object `uuid` among PSD global blocks given as (key, data).
pub fn find_in_blocks<'a>(blocks: impl IntoIterator<Item = (&'a [u8; 4], &'a [u8])>, uuid: &str) -> Option<LinkedFile> {
    if uuid.is_empty() {
        return None;
    }
    let (uuid, file_name, bytes) = find_slice_in_blocks(blocks, uuid)?;
    Some(LinkedFile { uuid, file_name, bytes: bytes.to_vec() })
}

/// [`find_in_blocks`] without copying: the uuid, file name and bytes of the embedded file, as a
/// slice of the block. Only the matching item's header is parsed beyond the uuid, and no other
/// embedded file is copied (a `lnk2` block can hold gigabytes of them).
fn find_slice_in_blocks<'a>(blocks: impl IntoIterator<Item = (&'a [u8; 4], &'a [u8])>, uuid: &str) -> Option<(String, String, &'a [u8])> {
    if uuid.is_empty() {
        return None;
    }
    blocks
        .into_iter()
        .filter(|(key, _)| matches!(*key, b"lnk2" | b"lnk3" | b"lnkD"))
        .flat_map(|(_, data)| craft_psd::smart::iter_linked_records(data))
        .filter_map(|record| item_data(record.item()?))
        .find(|(id, _, _)| id == uuid)
}

/// The size of embedded file `uuid` among PSD global blocks given as (key, data), when it is there
/// and readable with a non-empty size (the source size a `PlLd` record leaves out).
pub(crate) fn source_size_in_blocks<'a>(
    blocks: impl IntoIterator<Item = (&'a [u8; 4], &'a [u8])>,
    uuid: &str,
    ctl: &photocraft_raster::Interrupt,
) -> Option<(f64, f64)> {
    let (_, _, bytes) = find_slice_in_blocks(blocks, uuid)?;
    if ctl.cancelled() {
        return None;
    }
    source_size_probe(bytes).filter(|(w, h)| *w > 0.0 && *h > 0.0)
}

/// Pixel size of an embedded source file from its header: PSD/PSB, TIFF (the default page's
/// directory), PNG (`IHDR`) and JPEG (the frame header); WebP, GIF, BMP, EXR and the rest of
/// `photocraft_codecs::probe_size`'s formats from their decoder's header; anything else (or a
/// header that probe cannot read) is decoded flat by `photocraft-codecs`. It never imports a
/// document: a layered TIFF can carry PSD layer data whose `lnkD` holds another file, so a full
/// import here would nest without bound (#2465).
pub(crate) fn source_size_probe(bytes: &[u8]) -> Option<(f64, f64)> {
    let be32 = |at: usize| bytes.get(at..at + 4).map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]));
    let size = |w: u32, h: u32| (f64::from(w), f64::from(h));
    match bytes {
        _ if crate::is_psd(bytes) => be32(18).zip(be32(14)).map(|(w, h)| size(w, h)),
        [b'I', b'I', 42, 0, ..] | [b'M', b'M', 0, 42, ..] | [b'I', b'I', 43, 0, ..] | [b'M', b'M', 0, 43, ..] => {
            let info = photocraft_codecs::tiff_info(bytes).ok()?;
            let page = info.pages.get(info.default_page()?)?;
            Some(size(page.width, page.height))
        }
        [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, ..] => be32(16).zip(be32(20)).map(|(w, h)| size(w, h)),
        [0xFF, 0xD8, ..] => jpeg_size(bytes),
        _ => photocraft_codecs::probe_size(bytes).or_else(|_| photocraft_codecs::decode(bytes).map(|img| img.dimensions())).ok().map(|(w, h)| size(w, h)),
    }
}

/// A JPEG's size from its first start-of-frame marker (ITU-T T.81 B.2.2), walking the marker
/// segments without decoding. Each step moves forward, so a malformed file ends the walk.
fn jpeg_size(bytes: &[u8]) -> Option<(f64, f64)> {
    let mut at = 2usize;
    loop {
        // Markers are 0xFF then a code; extra 0xFF fill bytes may come first.
        while bytes.get(at) == Some(&0xFF) && bytes.get(at + 1) == Some(&0xFF) {
            at += 1;
        }
        if *bytes.get(at)? != 0xFF {
            return None;
        }
        let code = *bytes.get(at + 1)?;
        match code {
            // SOF0..SOF15, except DHT (C4), JPG (C8) and DAC (CC).
            0xC0..=0xCF if !matches!(code, 0xC4 | 0xC8 | 0xCC) => {
                let h = bytes.get(at + 5..at + 7)?;
                let w = bytes.get(at + 7..at + 9)?;
                return Some((f64::from(u16::from_be_bytes([w[0], w[1]])), f64::from(u16::from_be_bytes([h[0], h[1]]))));
            }
            // Standalone markers have no length.
            0x01 | 0xD0..=0xD7 => at = at.checked_add(2)?,
            // Start of scan or end of image before any frame header.
            0xDA | 0xD9 => return None,
            _ => {
                let len = bytes.get(at + 2..at + 4).map(|b| usize::from(u16::from_be_bytes([b[0], b[1]])))?;
                if len < 2 {
                    return None;
                }
                at = at.checked_add(2 + len)?;
            }
        }
    }
}

/// Size and resolution of a smart-object source file (PSD/PSB header, `.pcraft`, or any image).
/// Other formats are decoded in full, which `ctl` can cancel (its progress is left alone: this is
/// a step of a larger operation).
pub(crate) fn source_geometry(name: &str, bytes: &[u8], ctl: &photocraft_raster::Interrupt) -> Result<((f64, f64), f64), String> {
    if crate::is_psd(bytes) {
        let rd = |at: usize| bytes.get(at..at + 4).map(|b| f64::from(u32::from_be_bytes([b[0], b[1], b[2], b[3]])));
        let (h, w) = rd(14).zip(rd(18)).ok_or("truncated PSD header")?;
        return Ok(((w, h), 72.0));
    }
    let cancel = || ctl.cancelled();
    let doc = crate::import_with(name, bytes, &photocraft_raster::Interrupt::cancel_only(&cancel)).map_err(|e| e.to_string())?.document;
    Ok(((f64::from(doc.size.width), f64::from(doc.size.height)), f64::from(doc.resolution_dpi)))
}

/// The four-character file type Photoshop records for an embedded file, from its contents.
pub fn file_type(bytes: &[u8]) -> [u8; 4] {
    craft_psd::smart::file_type(bytes)
}

/// Encode using the shared PSD record writer.
pub fn encode_linked_file(f: &LinkedFile) -> Vec<u8> {
    craft_psd::smart::encode_embedded_file(&f.uuid, &f.file_name, &f.bytes)
}

/// Preserve unreadable records, retaining or appending the selected embedded files.
pub fn rebuild_block(data: &[u8], keep: &dyn Fn(&str) -> bool, add: &[LinkedFile]) -> Option<Vec<u8>> {
    let kept = craft_psd::smart::rebuild_linked_block(data, keep, &[]);
    if add.is_empty() {
        return kept;
    }
    // Encode one new payload at a time, preserving the app's peak-memory behavior.
    let mut out = kept.unwrap_or_else(|| data.to_vec());
    for file in add {
        out.extend(encode_linked_file(file));
    }
    Some(out)
}

/// IDs retain the shared reader's lossy metadata decoding.
pub fn block_uuids(data: &[u8]) -> Vec<String> {
    craft_psd::smart::linked_uuids(data)
}

/// Clipboard selection requires exact identity; ambiguous selected IDs are refused.
pub fn select_items(data: &[u8], keep: &dyn Fn(&str, &[u8]) -> bool) -> Result<Option<Vec<u8>>, crate::IoError> {
    craft_psd::smart::select_linked_records(data, keep).map_err(|error| {
        let message = match error {
            craft_psd::PsdError::OutOfMemory(_) => "not enough memory to copy PSD linked-layer records",
            _ if error.to_string().contains("not valid UTF-8") => "can't verify a PSD linked-layer source ID with invalid UTF-8",
            _ => "can't copy a truncated selected PSD linked-layer record",
        };
        crate::IoError::Unsupported(message.into())
    })
}

pub fn contains_item(data: &[u8], record: &[u8]) -> bool {
    craft_psd::smart::contains_linked_record(data, record)
}

/// Preserve the app's historical lossy-ID lookup; raw payload is never rewritten.
pub(crate) fn copy_item(data: &[u8], uuid: &str, new_uuid: &str) -> Option<Vec<u8>> {
    craft_psd::smart::iter_linked_records(data).find(|record| record.uuid.is_some_and(|id| String::from_utf8_lossy(id) == uuid))?.copy_with_uuid(new_uuid)
}

#[cfg(test)]
#[path = "linked_legacy_tests.rs"]
mod legacy;

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    /// #2465: a placed source's size comes from its header, for every format a CS2 file embeds,
    /// and never by importing it as a document.
    #[test]
    fn source_size_probe_reads_headers() {
        use photocraft_codecs::{ChannelLayout, EncodeOptions, Format, Image, SampleType};
        let img = Image::new(7, 5, ChannelLayout::Rgb, SampleType::U8).unwrap();
        for format in [Format::Png, Format::Jpeg, Format::Tiff, Format::Bmp, Format::WebP, Format::Gif, Format::OpenExr, Format::Tga] {
            let bytes = photocraft_codecs::encode(&img, format, &EncodeOptions::default()).unwrap();
            assert_eq!(source_size_probe(&bytes), Some((7.0, 5.0)), "{format:?}");
            // A header cut short neither panics nor invents a size (the decode fallback fails too).
            assert_eq!(source_size_probe(&bytes[..10]), None, "{format:?} truncated");
        }
        let mut psd = b"8BPS\0\x01".to_vec();
        psd.extend([0; 6]);
        psd.extend(3u16.to_be_bytes());
        psd.extend(40u32.to_be_bytes()); // height
        psd.extend(30u32.to_be_bytes()); // width
        assert_eq!(source_size_probe(&psd), Some((30.0, 40.0)));
        assert_eq!(source_size_probe(b"not an image"), None);
        assert_eq!(source_size_probe(&[]), None);
    }

    /// Malformed JPEG marker walks end without a size and without panicking or looping.
    #[test]
    fn jpeg_size_rejects_malformed_marker_walks() {
        let sof = [0xFF, 0xC0, 0, 17, 8, 0, 5, 0, 7];
        let ok: Vec<u8> = [&[0xFF, 0xD8][..], &[0xFF, 0xE0, 0, 4, 1, 2], &[0xFF, 0xFF, 0xD0], &sof].concat();
        assert_eq!(jpeg_size(&ok), Some((7.0, 5.0)), "APP0, a fill byte and a standalone marker are skipped");
        for bad in [
            vec![0xFF, 0xD8],
            vec![0xFF, 0xD8, 0xFF, 0xE0, 0, 1],       // length below 2
            vec![0xFF, 0xD8, 0xFF, 0xE0, 0xFF, 0xFF], // length past the end
            vec![0xFF, 0xD8, 0x00, 0x00],             // not a marker
            vec![0xFF, 0xD8, 0xFF, 0xDA, 0, 2],       // scan before any frame header
            vec![0xFF, 0xD8, 0xFF, 0xC0, 0, 17, 8],   // truncated frame header
            [&[0xFF, 0xD8][..], &[0xFF; 64]].concat(),
        ] {
            assert_eq!(jpeg_size(&bad), None, "{bad:?}");
        }
    }

    #[test]
    fn round_trips_and_finds_by_uuid() {
        let a = LinkedFile { uuid: "abc-1".into(), file_name: "a.png".into(), bytes: vec![1, 2, 3] };
        let b = LinkedFile { uuid: "def-2".into(), file_name: "b.psd".into(), bytes: vec![9; 10] };
        let mut data = encode_linked_file(&a);
        data.extend(encode_linked_file(&b));
        assert_eq!(parse_linked_files(&data), vec![a.clone(), b.clone()]);
        let mut meta = Metadata::default();
        meta.psd_global_blocks.push((*b"8BIM", *b"lnk2", Arc::new(data)));
        assert_eq!(find_linked_file(&meta, "def-2"), Some(b));
        assert_eq!(find_linked_file(&meta, "zzz"), None);
        assert_eq!(find_linked_file(&meta, ""), None);
    }

    #[test]
    fn blocks_are_pruned_and_extended() {
        let a = LinkedFile { uuid: "a".into(), file_name: "a.png".into(), bytes: vec![0x89, b'P', b'N', b'G', 1] };
        let b = LinkedFile { uuid: "b".into(), file_name: "b.psb".into(), bytes: b"8BPS\0\x02rest".to_vec() };
        let c = LinkedFile { uuid: "c".into(), file_name: "c".into(), bytes: vec![7; 3] };
        let mut data = encode_linked_file(&a);
        data.extend(encode_linked_file(&b));
        assert_eq!(block_uuids(&data), vec!["a", "b"]);
        assert_eq!(rebuild_block(&data, &|_| true, &[]), None);
        let out = rebuild_block(&data, &|u| u != "a", std::slice::from_ref(&c)).unwrap();
        assert_eq!(parse_linked_files(&out), vec![b.clone(), c.clone()]);
        craft_psd::TaggedBlock::new(*b"lnk2", out).check_structure().unwrap();
        assert_eq!(file_type(&b.bytes), *b"8BPB");
        assert_eq!(file_type(&a.bytes), *b"PNGf");
        // A malformed tail is kept as is.
        let mut bad = encode_linked_file(&a);
        bad.extend([0, 0, 0, 0, 0, 0, 1, 0, 9]);
        let out = rebuild_block(&bad, &|_| true, std::slice::from_ref(&c)).unwrap();
        assert!(out.starts_with(&bad));
    }

    #[test]
    fn malformed_data_is_ignored() {
        assert!(parse_linked_files(&[0, 0, 0, 0, 0, 0, 0, 200, 1, 2]).is_empty());
        assert!(parse_linked_files(&[]).is_empty());
        let mut d = encode_linked_file(&LinkedFile { uuid: "x".into(), file_name: "f".into(), bytes: vec![5; 4] });
        d.truncate(d.len() - 6);
        assert!(parse_linked_files(&d).is_empty());
    }

    #[test]
    fn copying_linked_item_preserves_unknown_external_payload() {
        let mut raw = encode_linked_file(&LinkedFile { uuid: "old".into(), file_name: "missing.psb".into(), bytes: vec![7; 5] });
        raw[8..12].copy_from_slice(b"liFE");
        let copy = copy_item(&raw, "old", "independent-copy").unwrap();
        assert_eq!(block_uuids(&copy), vec!["independent-copy"]);
        let original_len = u64::from_be_bytes(raw[..8].try_into().unwrap()) as usize;
        let copied_len = u64::from_be_bytes(copy[..8].try_into().unwrap()) as usize;
        assert_eq!(&copy[8..16], &raw[8..16]);
        assert_eq!(&copy[17 + "independent-copy".len()..8 + copied_len], &raw[20..8 + original_len]);
        assert!(copy_item(&raw, "missing", "new").is_none());
    }
    #[test]
    fn selected_records_exclude_other_files_and_unidentified_pieces() {
        let a = encode_linked_file(&LinkedFile { uuid: "a".into(), file_name: "a.psb".into(), bytes: vec![1; 3] });
        let b = encode_linked_file(&LinkedFile { uuid: "b".into(), file_name: "b.psb".into(), bytes: vec![2; 4] });
        assert!(select_items(&a, &|uuid, _| uuid == "a").unwrap().is_none(), "an exclusive block can keep its original Arc");
        let mut data = a.clone();
        data.extend(&b);
        data.extend([0, 0, 0, 0, 0, 0, 1, 0, 9]);
        assert_eq!(select_items(&data, &|uuid, _| uuid == "a").unwrap(), Some(a.clone()));
        assert_eq!(select_items(&data, &|uuid, _| uuid == "b").unwrap(), Some(b));
        assert_eq!(select_items(&data, &|_, _| false).unwrap(), Some(Vec::new()));
        // A complete but unidentified item can be skipped without losing a later selected item.
        let mut unreadable = 1u64.to_be_bytes().to_vec();
        unreadable.extend([7, 0, 0, 0]);
        unreadable.extend(&a);
        assert_eq!(select_items(&unreadable, &|uuid, _| uuid == "a").unwrap(), Some(a));
    }

    #[test]
    fn selected_external_and_alias_records_keep_unknown_payloads_verbatim() {
        for kind in [*b"liFE", *b"liFA", *b"next"] {
            let mut raw = encode_linked_file(&LinkedFile { uuid: "selected".into(), file_name: "missing.psb".into(), bytes: vec![7; 5] });
            raw[8..12].copy_from_slice(&kind);
            raw[12..16].copy_from_slice(&42u32.to_be_bytes()); // an unknown future version
            let mut data = encode_linked_file(&LinkedFile { uuid: "unselected".into(), file_name: "other.psb".into(), bytes: vec![9; 3] });
            data.extend(&raw);
            assert_eq!(select_items(&data, &|uuid, _| uuid == "selected").unwrap(), Some(raw));
        }
    }

    #[test]
    fn truncated_selected_records_fail_instead_of_becoming_unavailable() {
        let mut raw = encode_linked_file(&LinkedFile { uuid: "selected".into(), file_name: "missing.psb".into(), bytes: vec![7; 5] });
        raw[..8].copy_from_slice(&u64::MAX.to_be_bytes());
        assert!(select_items(&raw, &|uuid, _| uuid == "selected").unwrap_err().to_string().contains("truncated selected"));
        assert_eq!(select_items(&raw, &|uuid, _| uuid == "other").unwrap(), Some(Vec::new()));
    }
    #[test]
    fn existing_records_are_matched_individually_without_matching_other_payloads() {
        let a = encode_linked_file(&LinkedFile { uuid: "a".into(), file_name: "a.psb".into(), bytes: vec![1; 3] });
        let b = encode_linked_file(&LinkedFile { uuid: "b".into(), file_name: "b.psb".into(), bytes: vec![2; 4] });
        let different = encode_linked_file(&LinkedFile { uuid: "a".into(), file_name: "a.psb".into(), bytes: vec![9; 3] });
        let mut combined = a.clone();
        combined.extend(&b);
        assert!(contains_item(&combined, &a));
        assert!(contains_item(&combined, &b));
        assert!(!contains_item(&combined, &different));
        assert_eq!(select_items(&a, &|_, raw| !contains_item(&combined, raw)).unwrap(), Some(Vec::new()));
        assert!(select_items(&different, &|_, raw| !contains_item(&combined, raw)).unwrap().is_none());
    }
    #[test]
    fn empty_uuids_are_unidentified_even_when_requested() {
        let raw = encode_linked_file(&LinkedFile { uuid: String::new(), file_name: "unidentified.psb".into(), bytes: vec![7; 5] });
        assert_eq!(select_items(&raw, &|_, _| true).unwrap(), Some(Vec::new()));
        let mut truncated = raw;
        truncated[..8].copy_from_slice(&u64::MAX.to_be_bytes());
        assert_eq!(select_items(&truncated, &|_, _| true).unwrap(), Some(Vec::new()));
    }
    #[test]
    fn selection_never_uses_lossy_uuid_aliases_as_record_identity() {
        let mut a = encode_linked_file(&LinkedFile { uuid: "x".into(), file_name: "a.psb".into(), bytes: vec![1; 3] });
        let mut b = encode_linked_file(&LinkedFile { uuid: "x".into(), file_name: "b.psb".into(), bytes: vec![2; 4] });
        a[17] = 0xff;
        b[17] = 0xfe;
        let mut data = a;
        data.extend(b);
        assert_eq!(block_uuids(&data), vec!["\u{fffd}", "\u{fffd}"], "legacy lossy parsing remains unchanged");
        assert!(select_items(&data, &|uuid, _| uuid == "\u{fffd}").unwrap_err().to_string().contains("invalid UTF-8"));
        let valid = encode_linked_file(&LinkedFile { uuid: "valid".into(), file_name: "valid.psb".into(), bytes: vec![3; 5] });
        data.extend(&valid);
        assert_eq!(select_items(&data, &|uuid, _| uuid == "valid").unwrap(), Some(valid));
        // A literal replacement character can itself be a valid UTF-8 identity.
        let valid_unicode = encode_linked_file(&LinkedFile { uuid: "\u{fffd}".into(), file_name: "unicode.psb".into(), bytes: vec![4; 2] });
        assert!(select_items(&valid_unicode, &|uuid, _| uuid == "\u{fffd}").unwrap().is_none());
    }
}

#[cfg(test)]
mod bounded_payload_tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn selected_payload_is_borrowed_and_other_sources_are_not_copied() {
        let a = LinkedFile { uuid: "a".into(), file_name: "Other.psb".into(), bytes: vec![1; 1024 * 1024] };
        let b = LinkedFile { uuid: "b".into(), file_name: "Logo.ai".into(), bytes: b"%PDF-1.6".to_vec() };
        let mut data = encode_linked_file(&a);
        data.extend(encode_linked_file(&b));
        let mut meta = Metadata::default();
        meta.psd_global_blocks.push((*b"8BIM", *b"lnk2", Arc::new(data)));
        let (name, bytes) = embedded_payload(&meta, "b").unwrap();
        assert_eq!(name, "Logo.ai");
        assert_eq!(bytes, b.bytes);
        let original = meta.psd_global_blocks[0].2.as_slice();
        assert!((original.as_ptr_range()).contains(&bytes.as_ptr()));
        assert!(embedded_payload(&meta, "").is_none());
        assert!(embedded_payload(&meta, "/dev/zero").is_none());
    }
}

#[cfg(test)]
mod shared_parity {
    use super::*;
    fn check(data: &[u8]) {
        let old: Vec<_> = legacy::parse_linked_files(data).into_iter().map(|f| (f.uuid, f.file_name, f.bytes)).collect();
        let new: Vec<_> = parse_linked_files(data).into_iter().map(|f| (f.uuid, f.file_name, f.bytes)).collect();
        assert_eq!(new, old);
        assert_eq!(block_uuids(data), legacy::block_uuids(data));
        for id in ["wanted", "other", "\u{fffd}", ""] {
            let meta = Metadata { psd_global_blocks: vec![(*b"8BIM", *b"lnk2", std::sync::Arc::new(data.to_vec()))], ..Metadata::default() };
            assert_eq!(embedded_file_name(&meta, id), legacy::embedded_file_name(&meta, id));
            assert_eq!(embedded_payload(&meta, id), legacy::embedded_payload(&meta, id));
            assert_eq!(copy_item(data, id, "new"), legacy::copy_item(data, id, "new"));
            assert_eq!(rebuild_block(data, &|x| x == id, &[]), legacy::rebuild_block(data, &|x| x == id, &[]));
            assert_eq!(select_items(data, &|x, _| x == id).map_err(|e| e.to_string()), legacy::select_items(data, &|x, _| x == id).map_err(|e| e.to_string()));
        }
    }
    #[test]
    fn shared_linked_views_match_original_on_truncation_and_mutation() {
        let file = LinkedFile { uuid: "wanted".into(), file_name: "source.psd".into(), bytes: b"8BPS payload".to_vec() };
        let encoded = encode_linked_file(&file);
        assert_eq!(
            encoded,
            legacy::encode_linked_file(&legacy::LinkedFile { uuid: file.uuid.clone(), file_name: file.file_name.clone(), bytes: file.bytes.clone() })
        );
        let data = [vec![0; 8], encoded.clone(), encoded, vec![1, 2, 3]].concat();
        for n in 0..=data.len() {
            check(&data[..n]);
        }
        for at in 0..data.len() {
            for byte in [0, 0xff, 0x80] {
                let mut changed = data.clone();
                changed[at] = byte;
                check(&changed);
            }
        }
    }
}
