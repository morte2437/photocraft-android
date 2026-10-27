use super::tiff::{TiffBuilder, Val};

// ---------------------------------------------------------------- Olympus ORF

/// A little-endian IFD whose value offsets are relative to `base` (the IFD
/// itself sits at `base_offset` from that base). Entries: (tag, type, count,
/// value bytes).
fn relative_ifd(entries: &[(u16, u16, u32, Vec<u8>)], base_offset: usize) -> Vec<u8> {
    let head = 2 + 12 * entries.len() + 4;
    let mut out = Vec::new();
    let mut extra = Vec::new();
    out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    for (tag, typ, count, bytes) in entries {
        out.extend_from_slice(&tag.to_le_bytes());
        out.extend_from_slice(&typ.to_le_bytes());
        out.extend_from_slice(&count.to_le_bytes());
        if bytes.len() <= 4 {
            let mut v = bytes.clone();
            v.resize(4, 0);
            out.extend_from_slice(&v);
        } else {
            out.extend_from_slice(&((base_offset + head + extra.len()) as u32).to_le_bytes());
            extra.extend_from_slice(bytes);
        }
    }
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&extra);
    out
}

fn shorts(v: &[u16]) -> Vec<u8> {
    v.iter().flat_map(|x| x.to_le_bytes()).collect()
}

/// A synthetic uncompressed Olympus ORF (`IIRO`): 12-bit `data` stored
/// left-justified in 16-bit samples, a GRBG EXIF CFAPattern and a new-style
/// maker note with ImageProcessing levels (black 64, WB 2.0 / 1.5, crop of a
/// 2-pixel border) and CameraSettings pointing at a tiny preview JPEG.
pub fn orf(width: usize, height: usize, data: &[u16]) -> Vec<u8> {
    let ip = relative_ifd(
        &[
            (0x0100, 3, 2, shorts(&[512, 384])),
            (0x011F, 3, 1, shorts(&[256])),
            (0x0600, 3, 4, shorts(&[64, 64, 64, 64])),
            (0x0611, 3, 2, shorts(&[12, 0])),
            (0x0612, 3, 2, shorts(&[2, 0])),
            (0x0613, 3, 2, shorts(&[2, 0])),
            (0x0614, 4, 1, ((width - 4) as u32).to_le_bytes().to_vec()),
            (0x0615, 4, 1, ((height - 4) as u32).to_le_bytes().to_vec()),
        ],
        64,
    );
    // CameraSettings with a preview JPEG (SOI, SOF0 16×8, EOI) right after it.
    let jpeg = [0xFF, 0xD8, 0xFF, 0xC0, 0x00, 0x0B, 8, 0x00, 0x08, 0x00, 0x10, 1, 1, 0x11, 0, 0xFF, 0xD9];
    let cs_at = 64 + ip.len().next_multiple_of(4);
    let cs_len = 2 + 12 * 3 + 4;
    let cs = relative_ifd(
        &[
            (0x0100, 4, 1, 1u32.to_le_bytes().to_vec()),
            (0x0101, 4, 1, ((cs_at + cs_len) as u32).to_le_bytes().to_vec()),
            (0x0102, 4, 1, (jpeg.len() as u32).to_le_bytes().to_vec()),
        ],
        cs_at,
    );
    let mut note = b"OLYMPUS\0II\x03\0".to_vec();
    note.extend_from_slice(&relative_ifd(&[(0x2020, 13, 1, (cs_at as u32).to_le_bytes().to_vec()), (0x2040, 13, 1, 64u32.to_le_bytes().to_vec())], 12));
    note.resize(64, 0);
    note.extend_from_slice(&ip);
    note.resize(cs_at, 0);
    note.extend_from_slice(&cs);
    note.extend_from_slice(&jpeg);
    let mut t = TiffBuilder::default();
    let strip = t.blob(data.iter().flat_map(|v| (v << 4).to_le_bytes()).collect());
    let exif = t.ifd(vec![(37500, Val::Undefined(note)), (41730, Val::Undefined(vec![2, 0, 2, 0, 1, 0, 2, 1]))]);
    let ifd0 = t.ifd(vec![
        (256, Val::Long(vec![width as u32])),
        (257, Val::Long(vec![height as u32])),
        (258, Val::Short(vec![16])),
        (259, Val::Short(vec![1])),
        (262, Val::Short(vec![1])),
        (271, Val::Ascii("OLYMPUS IMAGING CORP.".into())),
        (272, Val::Ascii("E-Synthetic".into())),
        (273, Val::Blobs(vec![strip])),
        (277, Val::Short(vec![1])),
        (278, Val::Long(vec![height as u32])),
        (279, Val::Long(vec![(data.len() * 2) as u32])),
        (34665, Val::Ifds(vec![exif])),
    ]);
    t.chain = vec![ifd0];
    let mut b = t.build();
    b[2..4].copy_from_slice(b"RO");
    b
}
