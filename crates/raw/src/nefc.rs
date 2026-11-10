//! Nikon Huffman-compressed NEF sensor data (TIFF compression 34713):
//! "lossless compressed" and "lossy compressed" (type 1 and type 2), 12 and
//! 14 bits.
//!
//! **Clean-room.** No raw-decoder source code (dcraw, LibRaw, rawspeed,
//! rawloader / rawler, darktable, RawTherapee, libopenraw, ExifTool's code)
//! was read. Sources:
//!
//! * Prose format descriptions: Laurent Clévy, "Nikon Electronic File (NEF)
//!   format" (maker-note layout and the `0x0096` linearization-table layout:
//!   version bytes, four `u16` predictor seeds at offset 2, curve size at 10,
//!   curve at 12, split row at 562); Bill Claff, "NEF Compression" (a lossy
//!   encoding curve followed by a fixed-table Huffman stage); ExifTool's Nikon
//!   tag-name documentation (`0x0093` NEFCompression, `0x0096`
//!   NEFLinearizationTable); ITU-T T.81 Annex F / H for the "difference
//!   category + additional bits" coding and canonical Huffman codes.
//! * The Huffman tables, which Nikon files do not carry, come from the
//!   black-box analysis documented in LightCraft (storytold/lightcraft#86,
//!   `crates/raw/src/vendor/nefc.rs`): recovered from CC0 raw.pixls.us samples
//!   shot in both compressed and uncompressed mode, and verified on ~45 files
//!   from ~35 bodies (D40 … D850, Df, Z 6, Z 50, 1 J1). Only the format facts
//!   and the tables were taken over (and, for "lossy after split" below, the
//!   dequantisation function); this module is written against this
//!   crate's own TIFF reader.
//!
//! Format:
//!
//! * One strip; an MSB-first bit stream without byte stuffing, continuous
//!   across rows. Per pixel a Huffman code gives the category `n`, followed by
//!   `n` additional bits (T.81: a leading 0 means a negative difference).
//! * Each pixel is predicted from the same-colour pixel two to the left; the
//!   first two pixels of a row from the first two pixels of the previous row
//!   of the same parity, starting at the seeds of maker note `0x0096`.
//! * Lossless (`0x0096` version `0x46`): the decoded values are the samples.
//!   Lossy (`0x44`): they index a curve. Type 1 (`0x44 0x10`) stores the whole
//!   curve, type 2 (`0x44 0x20` / `0x44 0x40`) stores `n` points spaced
//!   `2^bits / (n - 1)` codes apart, linearly interpolated. Version `0x44 0x40`
//!   uses a two-bit-smaller encoded domain (1025 / 4097 levels for 12 / 14-bit
//!   samples), not the sensor bit depth; see Bill Claff's NEF Compression tables.
//! * Values in the maker note are in the maker note's byte order.
//!
//! * "Lossy after split" (type 2 only, a non-zero split row at `0x0096` offset
//!   562): rows above the split row are Huffman coded as above. Row `split`
//!   starts at the very next bit; from there every pixel is one fixed-rate
//!   word of `bits - 4` bits (8 at 12 bit, 10 at 14 bit), MSB first, with no
//!   row padding: a prefix code (every word but all-ones) for the category `n`,
//!   followed by the top bits of the T.81 additional bits. The difference is
//!   the centre of the magnitude bin that was kept, the predictor is the same
//!   as above, and the running value is not clamped. The section fills the
//!   strip to its last byte: whole rows (up to 4 more than the image), then
//!   0–7 one-bits of padding. These facts, the prefix
//!   tables and the dequantisation (`fixed_words`, ported from LightCraft's
//!   `fixed_word_table`) come from LightCraft's black-box analysis of 11 CC0
//!   raw.pixls.us files (storytold/lightcraft#633), like the Huffman tables above.
//!
//! Not decoded (reported as unsupported; the embedded preview is used
//! instead): "lossy after split" strips that fail to decode, other
//! `0x0096` versions and bit depths other than 12 / 14.

use crate::Limits;
use crate::error::{RawError, Result};
use crate::sensor::Plane;
use crate::tiff::{Ifd, Tiff, tag};

/// TIFF Compression value of Nikon's Huffman-coded NEF data.
pub(crate) const NIKON_COMPRESSION: u32 = 34713;
/// Maker note NEFLinearizationTable.
pub(crate) const NIKON_LINEARIZATION_TABLE: u16 = 0x0096;

/// Packed uncompressed NEF modes 6 (12-bit) and 10 (14-bit). Measured on CC0
/// Z6/Z7/D850 samples: little-endian bit packing, rows optionally padded to 16
/// bytes. Selection comes from the maker note, never from the strip's length.
pub(crate) fn read_packed(t: &Tiff, ifd: &Ifd, bits: u32, limits: &Limits) -> Result<Plane> {
    let w = t.tag_uint(ifd, tag::IMAGE_WIDTH).ok_or_else(|| RawError::malformed("packed NEF has no width"))? as usize;
    let h = t.tag_uint(ifd, tag::IMAGE_LENGTH).ok_or_else(|| RawError::malformed("packed NEF has no height"))? as usize;
    if !t.le || t.tag_uint(ifd, tag::BITS_PER_SAMPLE) != Some(bits) || t.tag_uint(ifd, tag::SAMPLES_PER_PIXEL).unwrap_or(1) != 1 {
        return Err(RawError::unsupported("packed NEF byte order, bit depth or sample layout"));
    }
    limits.check(w as u64, h as u64, 2)?;
    let offsets = t.tag_uints(ifd, tag::STRIP_OFFSETS);
    let counts = t.tag_uints(ifd, tag::STRIP_BYTE_COUNTS);
    let ([off], [len]) = (offsets.as_slice(), counts.as_slice()) else {
        return Err(RawError::unsupported("packed NEF with multiple strips"));
    };
    let src = t.bytes(*off as usize, *len as usize).ok_or_else(|| RawError::malformed("packed NEF strip lies outside the file"))?;
    let data = unpack_little_rows(src, w, h, bits)?;
    Ok(Plane { width: w, height: h, samples: 1, bits, data, float_scale: None })
}

fn unpack_little_rows(src: &[u8], w: usize, h: usize, bits: u32) -> Result<Vec<u16>> {
    let bad = || RawError::malformed("packed NEF row layout is invalid or truncated");
    if w == 0 || h == 0 || !matches!(bits, 12 | 14) {
        return Err(bad());
    }
    let row = w.checked_mul(bits as usize).ok_or_else(bad)?.div_ceil(8);
    let padded = row.checked_add(15).ok_or_else(bad)? / 16 * 16;
    let stride = if row.checked_mul(h) == Some(src.len()) {
        row
    } else if padded.checked_mul(h) == Some(src.len()) {
        padded
    } else {
        return Err(bad());
    };
    let count = w.checked_mul(h).ok_or_else(bad)?;
    let mut out = Vec::with_capacity(count);
    let mask = (1u32 << bits) - 1;
    for row in src.chunks_exact(stride) {
        let mut bytes = row.iter();
        let (mut acc, mut have) = (0u32, 0u32);
        for _ in 0..w {
            while have < bits {
                acc |= u32::from(*bytes.next().ok_or_else(bad)?) << have;
                have += 8;
            }
            out.push((acc & mask) as u16);
            acc >>= bits;
            have -= bits;
        }
    }
    Ok(out)
}

/// A code word that never occurred in any analysed sample.
const UNSEEN: u8 = u8::MAX;

/// Canonical Huffman table: the number of codes of each length 1..=16 and the
/// symbols (difference categories) in code order.
struct Table {
    counts: &'static [u8],
    symbols: &'static [u8],
}

const LOSSLESS_12: Table = Table { counts: &[0, 1, 4, 2, 3, 1, 2], symbols: &[5, 4, 6, 3, 7, 2, 8, 1, 9, 0, 10, 11, 12] };
const LOSSLESS_14: Table = Table { counts: &[0, 1, 4, 2, 2, 3, 1, 2], symbols: &[7, 6, 8, 5, 9, 4, 10, 3, 11, 12, 2, 0, 1, 13, 14] };
const LOSSY_12: Table = Table { counts: &[0, 1, 5, 1, 1, 1, 1, 1, 1], symbols: &[5, 4, 3, 6, 2, 7, 1, 0, 8, 9, UNSEEN, 10] };
const LOSSY_14: Table = Table { counts: &[0, 1, 4, 3, 1, 1, 1, 1, 1], symbols: &[5, 6, 4, 7, 8, 3, 9, 2, 1, 0, 10, 11, 12] };

/// Longest code word in the tables above.
const MAX_LEN: u32 = 9;

/// A Huffman decoder: a lookup on the next [`MAX_LEN`] bits.
struct Huffman {
    /// `(category, code length)`; length 0 = no such code.
    lut: Vec<(u8, u8)>,
}

impl Huffman {
    fn new(t: &Table) -> Result<Huffman> {
        let mut lut = vec![(0u8, 0u8); 1 << MAX_LEN];
        let mut code = 0u32;
        let mut symbols = t.symbols.iter();
        for (i, &n) in t.counts.iter().enumerate() {
            let len = i as u32 + 1;
            if len > MAX_LEN && n > 0 {
                return Err(RawError::malformed("NEF Huffman code longer than the lookup"));
            }
            for _ in 0..n {
                let &s = symbols.next().ok_or_else(|| RawError::malformed("NEF Huffman table is short of symbols"))?;
                let first = (code << (MAX_LEN - len)) as usize;
                let span = 1usize << (MAX_LEN - len);
                if s != UNSEEN {
                    for e in lut.get_mut(first..first + span).ok_or_else(|| RawError::malformed("NEF Huffman table overflows"))? {
                        *e = (s, len as u8);
                    }
                }
                code += 1;
            }
            code <<= 1;
        }
        Ok(Huffman { lut })
    }
}

/// MSB-first bit reader without byte stuffing. Reads past the end return
/// zero bits; [`Bits::consumed`] tells how far it got.
struct Bits<'a> {
    src: &'a [u8],
    pos: usize,
    acc: u64,
    n: u32,
    consumed: usize,
}

impl<'a> Bits<'a> {
    fn new(src: &'a [u8]) -> Self {
        Bits { src, pos: 0, acc: 0, n: 0, consumed: 0 }
    }

    #[inline]
    fn fill(&mut self) {
        while self.n <= 56 {
            let b = self.src.get(self.pos).copied().unwrap_or(0);
            self.pos += 1;
            self.acc |= u64::from(b) << (56 - self.n);
            self.n += 8;
        }
    }

    #[inline]
    fn peek(&mut self, k: u32) -> u32 {
        self.fill();
        (self.acc >> (64 - k)) as u32
    }

    #[inline]
    fn skip(&mut self, k: u32) {
        self.acc <<= k;
        self.n -= k;
        self.consumed += k as usize;
    }

    /// `k` bits (k ≤ 32) as a number.
    #[inline]
    fn take(&mut self, k: u32) -> u32 {
        if k == 0 {
            return 0;
        }
        let v = self.peek(k);
        self.skip(k);
        v
    }
}

/// One coded difference, or `None` for a code word not in the table.
#[inline]
fn diff(b: &mut Bits, h: &Huffman) -> Option<i32> {
    let &(cat, len) = h.lut.get(b.peek(MAX_LEN) as usize)?;
    if len == 0 {
        return None;
    }
    b.skip(u32::from(len));
    let cat = u32::from(cat);
    if cat > 16 {
        return None;
    }
    let v = b.take(cat) as i32;
    // T.81 F.1.2.1: a leading 0 bit means a negative difference.
    Some(if cat > 0 && v < 1 << (cat - 1) { v - (1 << cat) + 1 } else { v })
}

/// How the decoded values map to samples.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Encoding {
    /// The decoded values are the samples.
    Lossless,
    /// The decoded values index this curve.
    Lossy(Vec<u16>),
}

/// Maker note `0x0096`, parsed.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct DecodeTable {
    pub encoding: Encoding,
    /// Predictor seeds `[row parity][column]`.
    pub seeds: [[i32; 2]; 2],
    /// First row of the fixed-rate section ("lossy after split"); 0 when the whole strip is
    /// Huffman coded.
    pub split: usize,
}

/// Parses maker note `0x0096` for a `bits`-bit image; `le` is the maker
/// note's byte order.
pub(crate) fn parse_table(t: &[u8], le: bool, bits: u32) -> Result<DecodeTable> {
    let u16_at = |o: usize| {
        let s: [u8; 2] = t.get(o..o.checked_add(2)?)?.try_into().ok()?;
        Some(if le { u16::from_le_bytes(s) } else { u16::from_be_bytes(s) })
    };
    let short = || RawError::malformed(format!("NEF linearization table too short ({} bytes)", t.len()));
    if bits != 12 && bits != 14 {
        return Err(RawError::unsupported(format!("Nikon compressed NEF with {bits}-bit samples")));
    }
    let (Some(&v0), Some(&v1)) = (t.first(), t.get(1)) else { return Err(short()) };
    let mut seeds = [[0i32; 2]; 2];
    let mut split = 0;
    for (i, s) in seeds.iter_mut().flatten().enumerate() {
        *s = i32::from(u16_at(2 + 2 * i).ok_or_else(short)?);
    }
    let encoding = match (v0, v1) {
        (0x46, _) => Encoding::Lossless,
        (0x44, 0x10 | 0x20 | 0x40) => {
            let n = usize::from(u16_at(10).ok_or_else(short)?);
            if n < 2 {
                return Err(RawError::malformed(format!("NEF linearization curve of {n} points")));
            }
            let points: Vec<u16> = (0..n).map(|i| u16_at(12 + 2 * i)).collect::<Option<_>>().ok_or_else(short)?;
            if v1 == 0x10 {
                Encoding::Lossy(points)
            } else {
                split = usize::from(u16_at(562).unwrap_or(0));
                // Bytes 12.. hold the curve; with more than 275 points it covers byte 562, so a
                // non-zero value there is not a split row: refused as before (preview fallback).
                if split != 0 && 12 + 2 * n > 562 {
                    return Err(RawError::unsupported(format!("Nikon compressed NEF: {n}-point curve overlapping the split row field")));
                }
                // The modern lossy curve has a smaller *encoded* domain while
                // its output remains in sensor ADUs. Using 2^bits stretches the
                // curve fourfold and puts Z6 image values below its black level.
                let range = 1usize << if v1 == 0x40 { bits - 2 } else { bits };
                if n - 1 > range || !range.is_multiple_of(n - 1) {
                    return Err(RawError::unsupported(format!("Nikon compressed NEF: {n}-point curve for {bits}-bit data")));
                }
                let step = (range / (n - 1)) as u32;
                let mut curve = Vec::with_capacity(range + 1);
                for p in points.windows(2) {
                    let (a, b) = (u32::from(p.first().copied().unwrap_or(0)), u32::from(p.get(1).copied().unwrap_or(0)));
                    for k in 0..step {
                        curve.push(((a * (step - k) + b * k) / step) as u16);
                    }
                }
                curve.extend(points.last().copied());
                Encoding::Lossy(curve)
            }
        }
        _ => return Err(RawError::unsupported(format!("Nikon compressed NEF version {v0:#04x} {v1:#04x}"))),
    };
    Ok(DecodeTable { encoding, seeds, split })
}

/// Difference-category prefix codes of the fixed-rate ("lossy after split") section:
/// `(prefix bits, prefix length, category n)`. A word of `bits - 4` bits is a prefix followed by
/// the top `bits - 4 - length` additional bits of category `n` (storytold/lightcraft#633; the
/// 14-bit `010` prefix never occurred in the analysed files, its category 12 is deduced).
const FIXED_12: &[(u32, u32, u32)] = &[
    (0b00, 2, 9),
    (0b010, 3, 10),
    (0b011, 3, 8),
    (0b100, 3, 7),
    (0b101, 3, 6),
    (0b110, 3, 5),
    (0b1110, 4, 4),
    (0b11110, 5, 3),
    (0b111110, 6, 2),
    (0b1111110, 7, 1),
    (0b11111110, 8, 0),
];
const FIXED_14: &[(u32, u32, u32)] = &[
    (0b00, 2, 8),
    (0b010, 3, 12),
    (0b011, 3, 11),
    (0b100, 3, 10),
    (0b101, 3, 9),
    (0b110, 3, 7),
    (0b1110, 4, 6),
    (0b11110, 5, 5),
    (0b111110, 6, 4),
    (0b1111110, 7, 3),
    (0b11111110, 8, 2),
    (0b111111110, 9, 1),
    (0b1111111110, 10, 0),
];

/// Fixed-rate word → dequantised difference, for `bits`-bit samples; `None` for the all-ones
/// word, which no prefix covers.
fn fixed_words(bits: u32) -> Vec<Option<i32>> {
    let (w, codes) = if bits == 12 { (8, FIXED_12) } else { (10, FIXED_14) };
    let mut lut = vec![None; 1 << w];
    for &(prefix, len, n) in codes {
        let kept = w - len;
        // `dropped` low bits of the T.81 additional bits are not stored: the word gives the
        // bin, the difference is its centre.
        let dropped = n.saturating_sub(kept);
        let half = (1i32 << dropped) >> 1;
        for m in 0..1u32 << kept {
            let extra = (m << dropped) as i32;
            let d = if n == 0 {
                0
            } else if extra >= 1 << (n - 1) {
                extra + half
            } else if dropped > 0 {
                extra + half - (1 << n)
            } else {
                extra - ((1 << n) - 1)
            };
            if let Some(slot) = lut.get_mut(((prefix << kept) | m) as usize) {
                *slot = Some(d);
            }
        }
    }
    lut
}

/// Decodes a `w × h` Nikon Huffman-compressed strip with `bits`-bit samples.
///
/// A "lossy after split" strip that fails to decode for any reason is reported as unsupported,
/// not malformed: before these files were decoded, every one was refused as unsupported and
/// opened as the camera's embedded preview, and that fallback stays for files the rule does not fit.
pub(crate) fn decode(src: &[u8], w: usize, h: usize, bits: u32, table: &DecodeTable) -> Result<Vec<u16>> {
    match decode_strip(src, w, h, bits, table) {
        Err(RawError::Malformed(m)) if table.split != 0 => Err(RawError::unsupported(format!("Nikon \"lossy after split\" NEF that does not decode: {m}"))),
        r => r,
    }
}

fn decode_strip(src: &[u8], w: usize, h: usize, bits: u32, table: &DecodeTable) -> Result<Vec<u16>> {
    if bits != 12 && bits != 14 {
        return Err(RawError::unsupported(format!("Nikon compressed NEF with {bits}-bit samples")));
    }
    let n = w.checked_mul(h).filter(|&n| n > 0).ok_or_else(|| RawError::malformed("NEF image is empty"))?;
    // Every code word is at least 2 bits: a strip that can't hold the image is
    // truncated (this also bounds the allocation by the file size).
    if n / 4 > src.len() {
        return Err(RawError::malformed(format!("NEF: {} bytes of compressed data for {w}x{h} pixels", src.len())));
    }
    let huff = Huffman::new(match (&table.encoding, bits) {
        (Encoding::Lossless, 12) => &LOSSLESS_12,
        (Encoding::Lossless, _) => &LOSSLESS_14,
        (Encoding::Lossy(_), 12) => &LOSSY_12,
        (Encoding::Lossy(_), _) => &LOSSY_14,
    })?;
    let (lut, max): (&[u16], i32) = match &table.encoding {
        Encoding::Lossless => (&[], (1i32 << bits) - 1),
        Encoding::Lossy(curve) => (curve, i32::try_from(curve.len()).unwrap_or(i32::MAX) - 1),
    };
    if table.split != 0 && table.split >= h {
        return Err(RawError::malformed(format!("NEF: split row {} for a {h}-row image", table.split)));
    }
    if table.split != 0 && !matches!(table.encoding, Encoding::Lossy(_)) {
        return Err(RawError::malformed("NEF: split row with a lossless table"));
    }
    let fixed = if table.split != 0 { fixed_words(bits) } else { Vec::new() };
    let word = bits - 4;
    let mut out = vec![0u16; n];
    let mut stream = Bits::new(src);
    let mut vpred = table.seeds;
    let mut clipped = 0usize;
    for (y, row) in out.chunks_exact_mut(w).enumerate() {
        let seeds = &mut vpred[y & 1];
        let mut hpred = [0i32; 2];
        let fixed_rate = table.split != 0 && y >= table.split;
        for (x, o) in row.iter_mut().enumerate() {
            let d = if fixed_rate {
                let code = stream.take(word) as usize;
                fixed.get(code).copied().flatten().ok_or_else(|| RawError::malformed(format!("NEF: unknown fixed-rate word in row {y}")))?
            } else {
                diff(&mut stream, &huff).ok_or_else(|| RawError::malformed(format!("NEF: invalid Huffman code in row {y}")))?
            };
            let p = if x < 2 {
                seeds[x] = seeds[x].saturating_add(d);
                hpred[x] = seeds[x];
                seeds[x]
            } else {
                hpred[x & 1] = hpred[x & 1].saturating_add(d);
                hpred[x & 1]
            };
            let v = p.clamp(0, max);
            // The fixed-rate section's running value is not clamped and may leave the range.
            clipped += usize::from(v != p && !fixed_rate);
            *o = if lut.is_empty() { v as u16 } else { lut.get(v as usize).copied().unwrap_or(0) };
        }
        // The camera pads the strip; reading well past its end means the data
        // is truncated or corrupt.
        if stream.consumed > src.len().saturating_mul(8).saturating_add(64) {
            return Err(RawError::malformed(format!("NEF: compressed data ends in row {y} of {h}")));
        }
    }
    // The fixed-rate section fills the rest of the strip: whole rows of `w` words, then 0–7
    // one-bits up to the byte boundary (a strip may hold up to 4 rows more than the image). This
    // checks the section's length, not its content: a strip that ends any other way does not
    // follow the rule and is refused, but a wrong word inside a well-formed one is not detected.
    if table.split != 0 {
        let left = src.len().saturating_mul(8).checked_sub(stream.consumed);
        let row_bits = w.saturating_mul(word as usize).max(1);
        let pad = left.map(|l| l % row_bits);
        let ones = |k: usize| src.last().is_some_and(|b| u32::from(*b) & ((1 << k) - 1) == (1 << k) - 1);
        if !left.is_some_and(|l| l / row_bits <= 4) || !pad.is_some_and(|k| k < 8 && ones(k)) {
            return Err(RawError::malformed(format!("NEF: the fixed-rate section from row {} does not end the strip", table.split)));
        }
    }
    // A wrong table or a corrupt stream drifts out of range quickly; real
    // files stay inside (a handful of edge pixels).
    if clipped > n / 1000 + 16 {
        return Err(RawError::malformed(format!("NEF: {clipped} decoded values out of range")));
    }
    Ok(out)
}

/// Reads the Huffman-compressed CFA image of `raw`. `note` is the Nikon maker
/// note (its own TIFF and IFD 0), which holds the decode table.
pub(crate) fn read_compressed(t: &Tiff, raw: &Ifd, note: Option<&(Tiff, Ifd)>, limits: &Limits) -> Result<Plane> {
    let width = t.tag_uint(raw, tag::IMAGE_WIDTH).ok_or_else(|| RawError::malformed("raw image has no width"))? as usize;
    let height = t.tag_uint(raw, tag::IMAGE_LENGTH).ok_or_else(|| RawError::malformed("raw image has no height"))? as usize;
    let samples = t.tag_uint(raw, tag::SAMPLES_PER_PIXEL).unwrap_or(1);
    if samples != 1 {
        return Err(RawError::unsupported(format!("Nikon compressed NEF with {samples} samples per pixel")));
    }
    let bits = t.tag_uint(raw, tag::BITS_PER_SAMPLE).unwrap_or(0);
    let table = note
        .and_then(|(n, m)| Some((n.raw(m.get(NIKON_LINEARIZATION_TABLE)?)?, n.le)))
        .ok_or_else(|| RawError::unsupported("Nikon compressed NEF without a linearization table (maker note 0x96)"))?;
    let table = parse_table(table.0, table.1, bits)?;
    match read_strip(t, raw, width, height, bits, &table, limits) {
        // Until split files were decoded, each was refused as unsupported before the image was
        // read, and opened as the camera's preview: any failure after that point keeps it so.
        Err(e @ (RawError::Malformed(_) | RawError::LimitExceeded(_))) if table.split != 0 => {
            Err(RawError::unsupported(format!("Nikon \"lossy after split\" NEF that does not decode: {e}")))
        }
        r => r,
    }
}

fn read_strip(t: &Tiff, raw: &Ifd, width: usize, height: usize, bits: u32, table: &DecodeTable, limits: &Limits) -> Result<Plane> {
    limits.check(width as u64, height as u64, 2)?;
    // One strip in every sample; several would be consecutive parts of the
    // same bit stream.
    let offsets = t.tag_uints(raw, tag::STRIP_OFFSETS);
    let counts = t.tag_uints(raw, tag::STRIP_BYTE_COUNTS);
    let (Some(&first), Some(&last), Some(&last_len)) = (offsets.first(), offsets.last(), counts.last()) else {
        return Err(RawError::malformed("NEF: no image data"));
    };
    let start = first as usize;
    let end = (last as usize).saturating_add(last_len as usize).min(t.data.len());
    let src = t.data.get(start..end).filter(|s| !s.is_empty()).ok_or_else(|| RawError::malformed("raw data lies outside the file"))?;
    let data = decode(src, width, height, bits, table)?;
    Ok(Plane { width, height, samples: 1, bits, data, float_scale: None })
}

#[cfg(test)]
mod tests;
