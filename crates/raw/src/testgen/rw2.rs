use super::tiff::{TiffBuilder, Val};

// ---------------------------------------------------------------- Panasonic RW2

/// Page size and split point of the RW2 stream (see `rw2.rs`).
const RW2_PAGE: usize = 0x4000;

const RW2_SPLIT: usize = 0x1FF8;

/// Packs `bits`-bit samples (12: 10 per block, 14: 9 per block) into the RW2
/// RawFormat 5 layout: 16-byte little-endian blocks, stored in 0x4000-byte
/// pages split at 0x1FF8.
pub fn rw2_pack(data: &[u16], width: usize, bits: u32) -> Vec<u8> {
    let ppb = if bits == 14 { 9 } else { 10 };
    assert_eq!(width % ppb, 0);
    let mut logical = Vec::new();
    for px in data.chunks_exact(ppb) {
        let mut v: u128 = 0;
        for (i, &p) in px.iter().enumerate() {
            v |= u128::from(p & ((1 << bits) - 1)) << (bits as usize * i);
        }
        logical.extend_from_slice(&v.to_le_bytes());
    }
    logical.resize(logical.len().div_ceil(RW2_PAGE) * RW2_PAGE, 0);
    let mut out = vec![0u8; logical.len()];
    for (i, &b) in logical.iter().enumerate() {
        let page = i - i % RW2_PAGE;
        out[page + (i % RW2_PAGE + RW2_SPLIT) % RW2_PAGE] = b;
    }
    out
}

/// A synthetic Panasonic RW2 (RawFormat 5) with an RGGB sensor, a 2-pixel
/// border on every side, black levels 128 / 129 / 130 and WB levels
/// 512 / 256 / 384.
pub fn rw2(width: usize, height: usize, data: &[u16], bits: u32) -> Vec<u8> {
    let mut t = TiffBuilder::default();
    let raw = t.blob(rw2_pack(data, width, bits));
    let white = ((1u32 << bits) - 1) as u16;
    let ifd0 = t.ifd(vec![
        (0x0002, Val::Short(vec![width as u16])),
        (0x0003, Val::Short(vec![height as u16])),
        (0x0004, Val::Short(vec![2])),
        (0x0005, Val::Short(vec![2])),
        (0x0006, Val::Short(vec![height as u16 - 2])),
        (0x0007, Val::Short(vec![width as u16 - 2])),
        (0x0009, Val::Short(vec![1])),
        (0x000A, Val::Short(vec![bits as u16])),
        (0x000E, Val::Short(vec![white])),
        (0x000F, Val::Short(vec![white])),
        (0x0010, Val::Short(vec![white])),
        (0x001C, Val::Short(vec![128])),
        (0x001D, Val::Short(vec![129])),
        (0x001E, Val::Short(vec![130])),
        (0x0024, Val::Short(vec![512])),
        (0x0025, Val::Short(vec![256])),
        (0x0026, Val::Short(vec![384])),
        (0x002D, Val::Short(vec![5])),
        (0x010F, Val::Ascii("Panasonic".into())),
        (0x0110, Val::Ascii("DC-Synthetic".into())),
        (0x0112, Val::Short(vec![1])),
        (0x0118, Val::Blobs(vec![raw])),
    ]);
    t.chain = vec![ifd0];
    let mut b = t.build();
    b[2..4].copy_from_slice(b"U\0");
    b
}

/// Byte split of the vendor-compressed (RawFormat 3/4) stream (see `rw2.rs`).
const RW2_COMPRESSED_SPLIT: usize = 0x2008;

/// Writer for the vendor-compressed RW2 stream: mirrors the decoder's cursor
/// (page addressing, downward field order) and deposits every field where the
/// reader will pick it up.
struct V4Writer<'a> {
    buf: &'a mut [u8],
    base: usize,
    pos: usize,
    nbits: u32,
}

impl V4Writer<'_> {
    /// Consumes one byte of the row seek (the decoder reads 8-bit fields).
    fn skip_byte(&mut self) {
        if 8 > self.nbits {
            self.nbits += (RW2_PAGE as u32) * 8;
            self.pos += RW2_PAGE;
        }
        self.nbits -= 8;
    }

    fn put(&mut self, v: u32, num: u32) {
        if num > self.nbits {
            self.nbits += (RW2_PAGE as u32) * 8;
            self.pos += RW2_PAGE;
        }
        let mut at = ((self.nbits - num) >> 3) as usize ^ 0x3FF0;
        at = (at + RW2_PAGE - RW2_COMPRESSED_SPLIT) % RW2_PAGE;
        let at = self.base + at + self.pos - RW2_PAGE;
        self.nbits -= num;
        let cur = u32::from(self.buf[at]) | (u32::from(self.buf[at + 1]) << 8);
        let cur = cur | ((v & ((1 << num) - 1)) << (self.nbits & 7));
        self.buf[at] = cur as u8;
        self.buf[at + 1] = (cur >> 8) as u8;
    }

    /// Encodes one row as 14-pixel groups: the scale selector is always 0 and
    /// the two first pixels of each group are absolute (8 + 4 bits); later
    /// pixels use the 8-bit delta path. Callers keep values inside 16..4096
    /// and stepping by at most ±127 between pixels of the same parity.
    fn row(&mut self, pixels: &[u16]) {
        for group in pixels.as_chunks::<14>().0 {
            let mut pred = [0i32; 2];
            let mut nonz = [0i32; 2];
            for (i, &px) in group.iter().enumerate() {
                if i % 3 == 2 {
                    self.put(0, 2); // sh = 0
                }
                let p = i & 1;
                let t = i32::from(px);
                if nonz[p] == 0 {
                    let nz = (t >> 4) & 0xFF;
                    self.put(nz as u32, 8);
                    if nz != 0 || i > 11 {
                        self.put((t & 0xF) as u32, 4);
                        pred[p] = t;
                    } else {
                        pred[p] = 0;
                    }
                    nonz[p] = nz;
                } else {
                    let q = (pred[p] - 0x80).max(0);
                    let j = t - q;
                    assert!((1..=0xFF).contains(&j), "{t} is not encodable from {}", pred[p]);
                    self.put(j as u32, 8);
                    pred[p] = q + j;
                }
                assert_eq!(pred[p], t);
            }
        }
    }
}

/// The vendor-compressed (RawFormat 3/4) stream of a whole image; see
/// [`V4Writer::row`] for the value constraints.
pub fn rw2_v4_pack(data: &[u16], width: usize) -> Vec<u8> {
    assert_eq!(width % 14, 0);
    let height = data.len() / width;
    assert_eq!(data.len(), width * height);
    let row_bytes = width / 14 * 16;
    let mut buf = vec![0u8; row_bytes * height + 2 * RW2_PAGE];
    for (row, pixels) in data.chunks_exact(width).enumerate() {
        let skip = row_bytes * row;
        let mut w = V4Writer { buf: &mut buf, base: skip / RW2_PAGE * RW2_PAGE, pos: 0, nbits: 0 };
        for _ in 0..(skip % RW2_PAGE) {
            w.skip_byte();
        }
        w.row(pixels);
    }
    buf
}

/// A synthetic Panasonic RW2 (RawFormat 4, vendor-compressed) with an RGGB
/// sensor, a 2-pixel border on every side, black levels 128 / 129 / 130 and
/// WB levels 512 / 256 / 384; see [`V4Writer::row`] for the pixel constraints.
pub fn rw2_format4(width: usize, height: usize, data: &[u16]) -> Vec<u8> {
    let mut t = TiffBuilder::default();
    let raw = t.blob(rw2_v4_pack(data, width));
    let ifd0 = t.ifd(vec![
        (0x0002, Val::Short(vec![width as u16])),
        (0x0003, Val::Short(vec![height as u16])),
        (0x0004, Val::Short(vec![2])),
        (0x0005, Val::Short(vec![2])),
        (0x0006, Val::Short(vec![height as u16 - 2])),
        (0x0007, Val::Short(vec![width as u16 - 2])),
        (0x0009, Val::Short(vec![1])),
        (0x000A, Val::Short(vec![12])),
        (0x000E, Val::Short(vec![4095])),
        (0x000F, Val::Short(vec![4095])),
        (0x0010, Val::Short(vec![4095])),
        (0x001C, Val::Short(vec![128])),
        (0x001D, Val::Short(vec![129])),
        (0x001E, Val::Short(vec![130])),
        (0x0024, Val::Short(vec![512])),
        (0x0025, Val::Short(vec![256])),
        (0x0026, Val::Short(vec![384])),
        (0x002D, Val::Short(vec![4])),
        (0x010F, Val::Ascii("Panasonic".into())),
        (0x0110, Val::Ascii("DC-Synthetic".into())),
        (0x0112, Val::Short(vec![1])),
        (0x0118, Val::Blobs(vec![raw])),
    ]);
    t.chain = vec![ifd0];
    let mut b = t.build();
    b[2..4].copy_from_slice(b"U\0");
    b
}
