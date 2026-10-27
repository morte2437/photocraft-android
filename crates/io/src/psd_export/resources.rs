use super::*;

pub(crate) fn legacy_name(s: &str) -> Vec<u8> {
    s.chars().map(|c| if c.is_ascii() && !c.is_ascii_control() { c as u8 } else { b'?' }).take(255).collect()
}

pub(crate) fn unicode_names_resource(names: &[&str]) -> Vec<u8> {
    let mut v = Vec::new();
    for n in names {
        let units: Vec<u16> = n.encode_utf16().chain(std::iter::once(0)).collect();
        v.extend_from_slice(&(units.len() as u32).to_be_bytes());
        for u in units {
            v.extend_from_slice(&u.to_be_bytes());
        }
    }
    v
}

pub(crate) fn pascal_names_resource(names: &[&str]) -> Vec<u8> {
    let mut v = Vec::new();
    for n in names {
        let b = legacy_name(n);
        let n = b.len().min(255);
        v.push(n as u8);
        v.extend_from_slice(&b[..n]);
    }
    // No per-string pad: Photoshop's resource 1006 for "Spot Color 1" is 13 bytes
    // (length byte + 12 characters). The 8BIM container pads an odd resource separately.
    v
}

pub(crate) fn guides_resource(doc: &Document) -> Vec<u8> {
    let mut v = Vec::new();
    v.extend_from_slice(&1u32.to_be_bytes());
    v.extend_from_slice(&576u32.to_be_bytes());
    v.extend_from_slice(&576u32.to_be_bytes());
    let n = doc.guides.horizontal.len() + doc.guides.vertical.len();
    v.extend_from_slice(&(n as u32).to_be_bytes());
    for (list, dir) in [(&doc.guides.vertical, 0u8), (&doc.guides.horizontal, 1u8)] {
        for p in list {
            v.extend_from_slice(&((p * 32.0).round() as i32).to_be_bytes());
            v.push(dir);
        }
    }
    v
}

/// Path resources: saved paths (2000+), the work path (1025) and the clipping path name (2999),
/// the last only when the imported one doesn't already name the document's clipping path.
pub(crate) fn path_resources(doc: &Document, warnings: &mut Vec<String>) -> Vec<ImageResource> {
    use crate::vector_map::{CLIPPING_PATH, WORK_PATH, path_from_resource, path_to_records};
    let mut out = Vec::new();
    let (w, h) = (doc.size.width, doc.size.height);
    let max = (*crate::vector_map::SAVED_PATHS.end() - *crate::vector_map::SAVED_PATHS.start() + 1) as usize;
    if doc.paths.len() > max {
        warnings.push(format!("only {max} saved paths fit in PSD; the rest were dropped"));
    }
    for (i, p) in doc.paths.iter().take(max).enumerate() {
        let data = match &p.psd_raw {
            Some(r) if path_from_resource(r, w, h).as_ref() == Some(&p.path) => r.to_vec(),
            _ => path_to_records(&p.path, w, h).to_bytes(),
        };
        let mut r = ImageResource::new(crate::vector_map::SAVED_PATHS.start() + i as u16, data);
        r.name = legacy_name(&p.name);
        out.push(r);
    }
    if let Some(wp) = &doc.work_path {
        out.push(ImageResource::new(WORK_PATH, path_to_records(wp, w, h).to_bytes()));
    }
    let raw_clip = doc.metadata.psd_resources.iter().find(|(id, _, _)| *id == CLIPPING_PATH);
    let raw_name = raw_clip.map(|(_, _, d)| {
        let n = usize::from(d.first().copied().unwrap_or(0));
        d.get(1..1 + n).map(|b| String::from_utf8_lossy(b).into_owned()).unwrap_or_default()
    });
    if let Some(c) = &doc.clipping_path
        && raw_name.as_deref() != Some(c.name.as_str())
    {
        // Pascal name, then flatness as 16.16 fixed (best effort; see vector_map docs).
        let mut data = vec![0u8];
        let name = legacy_name(&c.name);
        data[0] = name.len() as u8;
        data.extend_from_slice(&name);
        data.extend_from_slice(&((c.flatness.max(0.0) * 65536.0) as u32).to_be_bytes());
        out.push(ImageResource::new(CLIPPING_PATH, data));
    }
    out
}

/// Whether an imported (unmodelled) resource still describes the document and is written back.
pub(crate) fn keep_imported_resource(doc: &Document, id: u16, data: &[u8]) -> bool {
    // Measurement scale / count resources: only while they still describe the document.
    if !crate::annotations_map::keep_resource(doc, id) {
        return false;
    }
    // Guides v2 and guide GUIDs: only while they still describe the document's guides.
    if !crate::guides_map::keep_resource(doc, id, data) {
        return false;
    }
    // A preserved clipping-path resource is only valid while it names the current one.
    if id == crate::vector_map::CLIPPING_PATH {
        let n = usize::from(data.first().copied().unwrap_or(0));
        let raw_name = data.get(1..1 + n).map(|b| String::from_utf8_lossy(b).into_owned()).unwrap_or_default();
        return doc.clipping_path.as_ref().is_some_and(|c| c.name == raw_name);
    }
    true
}

/// A document-level (global) tagged block. Photoshop pads these to a multiple of 4, and readers
/// such as psd-tools step to the next block that way: an even pad after `CAI ` (77 bytes)
/// misaligned every block after it (#200).
pub(crate) fn global_block(signature: [u8; 4], key: [u8; 4], data: &[u8]) -> TaggedBlock {
    let mut tb = TaggedBlock::new(key, data.to_vec());
    tb.signature = signature;
    tb.padding = Some(vec![0; (4 - data.len() % 4) % 4]);
    tb
}
