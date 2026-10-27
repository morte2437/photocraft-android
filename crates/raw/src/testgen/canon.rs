use super::lj92::lj92_encode;
use super::tiff::{TiffBuilder, Val};

// ---------------------------------------------------------------- CR2

/// A synthetic CR2: one lossless JPEG with `components` components, cut into
/// vertical slices, plus a Canon maker note with SensorInfo and ColorData.
#[derive(Debug, Clone)]
pub struct Cr2Spec {
    pub width: usize,
    pub height: usize,
    /// Sensor samples, row-major.
    pub data: Vec<u16>,
    pub precision: u8,
    pub components: usize,
    /// Slice widths (all but the last equal); empty = no slice tag.
    pub slices: Vec<usize>,
    /// Image borders: left, top, right, bottom (inclusive), as in SensorInfo.
    pub borders: Option<[u16; 4]>,
    /// As-shot RGGB levels written to ColorData at word offset 0x47 (the PowerShot version).
    pub wb_rggb: Option<[u16; 4]>,
    pub orientation: u16,
    /// Canon model ID written to MakerNote tag 0x0010.
    pub model_id: Option<u32>,
}

impl Cr2Spec {
    pub fn build(&self) -> Vec<u8> {
        let (w, h) = (self.width, self.height);
        // Slice the sensor data into the JPEG sample order.
        let widths: Vec<usize> = if self.slices.is_empty() { vec![w] } else { self.slices.clone() };
        assert_eq!(widths.iter().sum::<usize>(), w);
        let mut seq = Vec::with_capacity(w * h);
        let mut x0 = 0;
        for &sw in &widths {
            for y in 0..h {
                seq.extend_from_slice(&self.data[y * w + x0..y * w + x0 + sw]);
            }
            x0 += sw;
        }
        let c = self.components;
        assert_eq!(w % c, 0);
        let jpeg = lj92_encode(&seq, w / c, h, c, self.precision, 1, None);
        let mut t = TiffBuilder::default();
        let jb = t.blob(jpeg.clone());
        let mut raw = vec![(259, Val::Short(vec![6])), (273, Val::Blobs(vec![jb])), (279, Val::Long(vec![jpeg.len() as u32]))];
        if !self.slices.is_empty() {
            let n = widths.len() - 1;
            raw.push((50752, Val::Short(vec![n as u16, widths[0] as u16, widths[n] as u16])));
        }
        let raw_ifd = t.ifd(raw);
        let mut mn: Vec<(u16, Val)> = Vec::new();
        if let Some(id) = self.model_id {
            mn.push((0x0010, Val::Long(vec![id])));
        }
        if let Some([l, tp, r, b]) = self.borders {
            mn.push((0x00E0, Val::Short(vec![34, w as u16, h as u16, 0, 0, l, tp, r, b, 0, 0, 0, 0, 0, 0, 0, 0])));
        }
        if let Some(wb) = self.wb_rggb {
            // Canon writes ColorData (0x4001) as UNDEFINED bytes that are 16-bit words; the
            // WB_RGGBLevelsAsShot of the PowerShot version sits at word offset 0x47 (the EOS
            // versions use 0x3F — see as_shot_wb).
            let mut cd = vec![0u8; 2 * 1273];
            for (i, v) in wb.iter().enumerate() {
                cd[2 * (0x47 + i)..2 * (0x47 + i) + 2].copy_from_slice(&v.to_le_bytes());
            }
            mn.push((0x4001, Val::Undefined(cd)));
        }
        let mut exif: Vec<(u16, Val)> = vec![(33434, Val::Rational(vec![(1, 100)]))];
        if !mn.is_empty() {
            let mi = t.ifd(mn);
            exif.push((37500, Val::IfdBytes(mi)));
        }
        let exif_ifd = t.ifd(exif);
        let thumb = t.blob(vec![128; 12]);
        let ifd0 = t.ifd(vec![
            (256, Val::Long(vec![2])),
            (257, Val::Long(vec![2])),
            (259, Val::Short(vec![1])),
            (262, Val::Short(vec![2])),
            (271, Val::Ascii("Canon".into())),
            (272, Val::Ascii("Canon EOS Synthetic".into())),
            (273, Val::Blobs(vec![thumb])),
            (274, Val::Short(vec![self.orientation])),
            (277, Val::Short(vec![3])),
            (279, Val::Long(vec![12])),
            (34665, Val::Ifds(vec![exif_ifd])),
        ]);
        let ifd1 = t.ifd(vec![(256, Val::Long(vec![1]))]);
        let ifd2 = t.ifd(vec![(256, Val::Long(vec![1]))]);
        t.chain = vec![ifd0, ifd1, ifd2, raw_ifd];
        t.cr2_raw_ifd = Some(raw_ifd);
        t.build()
    }
}

// ---------------------------------------------------------------- TIFF/EP

/// A synthetic uncompressed TIFF/EP raw (NEF / ARW-like): IFD0 with Make, the
/// CFA image in a SubIFD with 16-bit samples.
pub fn tiff_ep(make: &str, width: usize, height: usize, data: &[u16], cfa: [u8; 4], bits: u16, extra_raw_tags: Vec<(u16, Val)>) -> Vec<u8> {
    let mut t = TiffBuilder::default();
    let strip = t.blob(data.iter().flat_map(|v| v.to_le_bytes()).collect());
    let mut raw = vec![
        (254, Val::Long(vec![0])),
        (256, Val::Long(vec![width as u32])),
        (257, Val::Long(vec![height as u32])),
        (258, Val::Short(vec![bits])),
        (259, Val::Short(vec![1])),
        (262, Val::Short(vec![32803])),
        (273, Val::Blobs(vec![strip])),
        (277, Val::Short(vec![1])),
        (278, Val::Long(vec![height as u32])),
        (279, Val::Long(vec![(data.len() * 2) as u32])),
        (33421, Val::Short(vec![2, 2])),
        (33422, Val::Byte(cfa.to_vec())),
    ];
    raw.extend(extra_raw_tags);
    let raw_ifd = t.ifd(raw);
    let thumb = t.blob(vec![128; 12]);
    let ifd0 = t.ifd(vec![
        (254, Val::Long(vec![1])),
        (256, Val::Long(vec![2])),
        (257, Val::Long(vec![2])),
        (259, Val::Short(vec![1])),
        (262, Val::Short(vec![2])),
        (271, Val::Ascii(make.into())),
        (273, Val::Blobs(vec![thumb])),
        (277, Val::Short(vec![3])),
        (279, Val::Long(vec![12])),
        (330, Val::Ifds(vec![raw_ifd])),
    ]);
    t.chain = vec![ifd0];
    t.build()
}

/// A synthetic Nikon NEF with a Huffman-compressed (34713) strip: IFD0 with
/// Make and an EXIF IFD whose maker note (`Nikon\0`, version 2.10, then an
/// embedded TIFF in the given byte order) holds `note_tags` (for example
/// `0x0096`, the decode table, and `0x003d`, BlackLevel).
pub fn nef_compressed(width: usize, height: usize, bits: u16, strip: Vec<u8>, cfa: [u8; 4], note_big_endian: bool, note_tags: Vec<(u16, Val)>) -> Vec<u8> {
    let mut note = TiffBuilder { big_endian: note_big_endian, ..Default::default() };
    let n = note.ifd(note_tags);
    note.chain = vec![n];
    let mut maker = b"Nikon\0\x02\x10\0\0".to_vec();
    maker.extend(note.build());
    let mut t = TiffBuilder::default();
    let len = strip.len();
    let strip = t.blob(strip);
    let raw_ifd = t.ifd(vec![
        (254, Val::Long(vec![0])),
        (256, Val::Long(vec![width as u32])),
        (257, Val::Long(vec![height as u32])),
        (258, Val::Short(vec![bits])),
        (259, Val::Short(vec![34713])),
        (262, Val::Short(vec![32803])),
        (273, Val::Blobs(vec![strip])),
        (277, Val::Short(vec![1])),
        (278, Val::Long(vec![height as u32])),
        (279, Val::Long(vec![len as u32])),
        (33421, Val::Short(vec![2, 2])),
        (33422, Val::Byte(cfa.to_vec())),
    ]);
    let exif = t.ifd(vec![(37500, Val::Undefined(maker))]);
    let ifd0 = t.ifd(vec![(271, Val::Ascii("NIKON CORPORATION".into())), (330, Val::Ifds(vec![raw_ifd])), (34665, Val::Ifds(vec![exif]))]);
    t.chain = vec![ifd0];
    t.build()
}

// ---------------------------------------------------------------- Sony cRAW

/// Encodes 16 same-colour 11-bit codes as one cRAW block (see the decoder in
/// `sony.rs`): maximum, minimum, their positions, then 7-bit deltas at the
/// smallest power-of-two step that covers the range (rounded down).
pub fn craw_block(codes: &[u16; 16]) -> [u8; 16] {
    let max = *codes.iter().max().unwrap();
    let min = *codes.iter().min().unwrap();
    let imax = codes.iter().position(|&c| c == max).unwrap();
    let mut imin = codes.iter().position(|&c| c == min).unwrap();
    if imin == imax {
        imin = (imax + 1) % 16;
    }
    let mut sh = 0;
    while (128u16 << sh) <= max - min {
        sh += 1;
    }
    let mut v: u128 = u128::from(max) | u128::from(min) << 11 | (imax as u128) << 22 | (imin as u128) << 26;
    let mut at = 30;
    for (i, &c) in codes.iter().enumerate() {
        if i != imax && i != imin {
            v |= u128::from(((c - min) >> sh).min(127)) << at;
            at += 7;
        }
    }
    v.to_le_bytes()
}

/// Encodes a mosaic of 11-bit codes (`width` a multiple of 32) as cRAW rows.
pub fn craw_encode(codes: &[u16], width: usize, height: usize) -> Vec<u8> {
    assert_eq!(width % 32, 0);
    let mut out = Vec::with_capacity(width * height);
    for row in codes.chunks_exact(width).take(height) {
        for g in row.as_chunks::<32>().0 {
            for parity in 0..2 {
                let seq: [u16; 16] = std::array::from_fn(|i| g[2 * i + parity].min(2047));
                out.extend_from_slice(&craw_block(&seq));
            }
        }
    }
    out
}

/// A synthetic Sony compressed ARW: IFD0 with Make, the cRAW image in a
/// SubIFD with SonyRawFileType 2, the tone curve (0x7010), black (0x7310),
/// white (50717) and WB_RGGBLevels (0x7313).
pub fn sony_craw(width: usize, height: usize, codes: &[u16], curve: [u16; 4]) -> Vec<u8> {
    let bytes = craw_encode(codes, width, height);
    let mut t = TiffBuilder::default();
    let n = bytes.len();
    let strip = t.blob(bytes);
    let raw_ifd = t.ifd(vec![
        (254, Val::Long(vec![0])),
        (256, Val::Long(vec![width as u32])),
        (257, Val::Long(vec![height as u32])),
        (258, Val::Short(vec![12])),
        (259, Val::Short(vec![32767])),
        (262, Val::Short(vec![32803])),
        (273, Val::Blobs(vec![strip])),
        (277, Val::Short(vec![1])),
        (278, Val::Long(vec![height as u32])),
        (279, Val::Long(vec![n as u32])),
        (33421, Val::Short(vec![2, 2])),
        (33422, Val::Byte(vec![0, 1, 1, 2])),
        (0x7000, Val::Short(vec![2])),
        (0x7010, Val::Short(curve.to_vec())),
        (0x7310, Val::Short(vec![512; 4])),
        (0x7313, Val::Short(vec![2048, 1024, 1024, 1536])),
        (50717, Val::Short(vec![16383])),
    ]);
    let ifd0 = t.ifd(vec![(271, Val::Ascii("SONY".into())), (272, Val::Ascii("ILCE-Synthetic".into())), (330, Val::Ifds(vec![raw_ifd]))]);
    t.chain = vec![ifd0];
    t.build()
}
