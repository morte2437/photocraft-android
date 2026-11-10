//! Lossless JPEG decoder (ITU-T T.81 / ISO 10918-1, process 14 "LJ92":
//! Huffman-coded, predictive, sequential, 2–16 bit samples).
//!
//! Written from the T.81 specification: Annex C (Huffman table
//! specification), Annex F.2.2.1-3 (decoding of DC-style difference
//! categories, here per Annex H) and Annex H (lossless mode: predictors 1–7,
//! the point transform, first-line / first-column rules, restart intervals).
//!
//! Output is the sample sequence in raster order: `height` rows of `width ×
//! components` interleaved samples. Only one interleaved scan holding every
//! component is supported, which is how DNG, CR2 and the TIFF-based raws use
//! it; [`decode`] takes sampling factors 1×1 only. Canon's sRAW / mRAW frames
//! (three components, the first sampled 2×1 or 2×2) go through
//! [`decode_subsampled`], which follows Canon's prediction rather than T.81's.

use crate::error::{RawError, Result};

/// Frame header (SOF3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Frame {
    pub precision: u8,
    pub width: usize,
    pub height: usize,
    pub components: usize,
    /// Horizontal and vertical sampling factors of the first component (1 and 1 unless the
    /// frame is subsampled; the other components are then sampled 1×1).
    pub h: usize,
    pub v: usize,
}

impl Frame {
    /// Samples in the frame: per MCU, `h × v` of the first component and one of each other.
    pub fn samples(&self) -> Option<usize> {
        let mcus = (self.width / self.h.max(1)).checked_mul(self.height / self.v.max(1))?;
        mcus.checked_mul(self.h.checked_mul(self.v)?.checked_add(self.components)?.checked_sub(1)?)
    }

    pub fn is_subsampled(&self) -> bool {
        self.h != 1 || self.v != 1
    }
}

/// Bits resolved by one table lookup.
const LOOKUP_BITS: u32 = 13;

#[derive(Clone)]
struct Huffman {
    /// `(length << 8) | symbol` for codes up to `LOOKUP_BITS` long, 0 if longer.
    lookup: Vec<u16>,
    /// `(difference << 8) | bits` when a code and its extra bits fit in
    /// `LOOKUP_BITS`, else 0: one lookup per sample on the common path.
    fast: Vec<i32>,
    /// Largest code of each length (-1 = none), lengths 1..=16.
    maxcode: [i32; 17],
    mincode: [i32; 17],
    valptr: [usize; 17],
    vals: Vec<u8>,
}

impl Huffman {
    /// Builds the decoder from BITS (counts per length 1..16) and HUFFVAL (T.81 Annex C).
    fn new(counts: &[u8; 16], vals: &[u8]) -> Result<Self> {
        let total: usize = counts.iter().map(|&c| usize::from(c)).sum();
        if total == 0 || total > 256 || vals.len() < total {
            return Err(RawError::malformed("bad Huffman table"));
        }
        let mut h = Huffman {
            lookup: vec![0; 1 << LOOKUP_BITS],
            fast: vec![0; 1 << LOOKUP_BITS],
            maxcode: [-1; 17],
            mincode: [0; 17],
            valptr: [0; 17],
            vals: vals[..total].to_vec(),
        };
        let mut code: u32 = 0;
        let mut k = 0usize;
        for len in 1..=16u32 {
            let n = usize::from(counts[len as usize - 1]);
            if n > 0 {
                h.valptr[len as usize] = k;
                h.mincode[len as usize] = code as i32;
                for _ in 0..n {
                    if code >= (1 << len) {
                        return Err(RawError::malformed("Huffman code overflow"));
                    }
                    if len <= LOOKUP_BITS {
                        let shift = LOOKUP_BITS - len;
                        let base = (code << shift) as usize;
                        let entry = ((len as u16) << 8) | u16::from(h.vals[k]);
                        for slot in &mut h.lookup[base..base + (1 << shift)] {
                            *slot = entry;
                        }
                    }
                    code += 1;
                    k += 1;
                }
                h.maxcode[len as usize] = code as i32 - 1;
            }
            code <<= 1;
        }
        for (idx, f) in h.fast.iter_mut().enumerate() {
            let e = h.lookup[idx];
            let (len, s) = (u32::from(e >> 8), u32::from(e & 0xFF));
            if len == 0 {
                continue;
            }
            let d: i32 = match s {
                0 => 0,
                16 => 32768,
                1..=15 if len + s <= LOOKUP_BITS => {
                    let v = ((idx as u32 >> (LOOKUP_BITS - len - s)) & ((1 << s) - 1)) as i32;
                    if v < (1 << (s - 1)) { v - (1 << s) + 1 } else { v }
                }
                _ => continue,
            };
            *f = (d << 8) | (len + if (1..=15).contains(&s) { s } else { 0 }) as i32;
        }
        Ok(h)
    }
}

/// MSB-first bit reader over entropy-coded data: removes `FF 00` stuffing,
/// stops at markers (then feeds zero bits).
struct Bits<'a> {
    data: &'a [u8],
    pos: usize,
    acc: u64,
    n: u32,
    marker: bool,
    /// Zero bytes fed past the end of the data or a marker.
    fake: usize,
}

impl<'a> Bits<'a> {
    fn new(data: &'a [u8]) -> Self {
        Bits { data, pos: 0, acc: 0, n: 0, marker: false, fake: 0 }
    }

    #[inline]
    fn refill(&mut self) {
        // Fast path: the next 8 bytes hold no 0xFF (no stuffing, no marker).
        if !self.marker
            && let Some(s) = self.data.get(self.pos..self.pos + 8)
        {
            let chunk = u64::from_be_bytes([s[0], s[1], s[2], s[3], s[4], s[5], s[6], s[7]]);
            let inv = !chunk;
            if inv.wrapping_sub(0x0101_0101_0101_0101) & !inv & 0x8080_8080_8080_8080 == 0 {
                let k = (64 - self.n) / 8; // whole bytes that fit (1..=8)
                if k == 0 {
                    return;
                }
                if k == 8 {
                    self.acc = chunk;
                } else {
                    self.acc |= (chunk >> (64 - 8 * k)) << (64 - self.n - 8 * k);
                }
                self.n += 8 * k;
                self.pos += k as usize;
                return;
            }
        }
        while self.n <= 56 {
            let byte = if self.marker {
                self.fake += 1;
                0
            } else {
                match self.data.get(self.pos) {
                    Some(&0xFF) => match self.data.get(self.pos + 1) {
                        Some(0) => {
                            self.pos += 2;
                            0xFF
                        }
                        _ => {
                            self.marker = true;
                            self.fake += 1;
                            0
                        }
                    },
                    Some(&b) => {
                        self.pos += 1;
                        b
                    }
                    None => {
                        self.marker = true;
                        self.fake += 1;
                        0
                    }
                }
            };
            self.acc |= u64::from(byte) << (56 - self.n);
            self.n += 8;
        }
    }

    #[inline]
    fn peek(&self, k: u32) -> u32 {
        (self.acc >> (64 - k)) as u32
    }

    #[inline]
    fn consume(&mut self, k: u32) {
        self.acc <<= k;
        self.n -= k;
    }

    /// Reads `k` (1..=16) bits.
    #[inline]
    fn get(&mut self, k: u32) -> u32 {
        if self.n < k {
            self.refill();
        }
        let v = self.peek(k);
        self.consume(k);
        v
    }

    /// Skips to the next RSTn marker and past it; resets the bit buffer.
    fn restart(&mut self) -> Result<()> {
        self.acc = 0;
        self.n = 0;
        self.marker = false;
        self.fake = 0;
        // Tolerate fill bytes before the marker.
        while let Some(&b) = self.data.get(self.pos) {
            if b == 0xFF {
                match self.data.get(self.pos + 1) {
                    Some(0xD0..=0xD7) => {
                        self.pos += 2;
                        return Ok(());
                    }
                    Some(0xFF) => self.pos += 1,
                    _ => return Err(RawError::malformed("expected a restart marker")),
                }
            } else {
                self.pos += 1;
            }
        }
        Err(RawError::malformed("missing restart marker"))
    }

    #[inline]
    fn decode(&mut self, h: &Huffman) -> Result<u32> {
        if self.n < 32 {
            self.refill();
        }
        let e = h.lookup[self.peek(LOOKUP_BITS) as usize];
        if e != 0 {
            self.consume(u32::from(e >> 8));
            return Ok(u32::from(e & 0xFF));
        }
        for len in LOOKUP_BITS + 1..=16 {
            let code = self.peek(len) as i32;
            if code <= h.maxcode[len as usize] {
                let i = h.valptr[len as usize] + (code - h.mincode[len as usize]) as usize;
                let v = *h.vals.get(i).ok_or_else(|| RawError::malformed("bad Huffman code"))?;
                self.consume(len);
                return Ok(u32::from(v));
            }
        }
        Err(RawError::malformed("bad Huffman code"))
    }

    /// A difference value: Huffman-coded category SSSS then SSSS extra bits (T.81 H.1.2.2).
    #[inline(always)]
    fn diff(&mut self, h: &Huffman) -> Result<i32> {
        if self.n < 32 {
            self.refill();
        }
        let f = h.fast[self.peek(LOOKUP_BITS) as usize];
        if f != 0 {
            self.consume((f & 0xFF) as u32);
            return Ok(f >> 8);
        }
        let s = self.decode(h)?;
        Ok(match s {
            0 => 0,
            1..=15 => {
                let v = self.get(s) as i32;
                if v < (1 << (s - 1)) { v - (1 << s) + 1 } else { v }
            }
            16 => 32768,
            _ => return Err(RawError::malformed("bad difference category")),
        })
    }
}

fn be16(d: &[u8], at: usize) -> Result<usize> {
    let s = d.get(at..at + 2).ok_or_else(|| RawError::malformed("truncated lossless JPEG"))?;
    Ok(usize::from(u16::from_be_bytes([s[0], s[1]])))
}

/// Reads the frame header without decoding.
pub(crate) fn frame(data: &[u8]) -> Result<Frame> {
    parse(data).map(|p| p.frame)
}

struct Parsed<'a> {
    frame: Frame,
    /// The scan's distinct Huffman tables...
    tables: Vec<Huffman>,
    /// ...and per component of the scan (in frame order) the index of its table: components
    /// sharing a table share one copy (one 48 KiB lookup pair in cache instead of several).
    comp_table: Vec<usize>,
    predictor: u8,
    point_transform: u8,
    restart: usize,
    entropy: &'a [u8],
}

fn parse(data: &[u8]) -> Result<Parsed<'_>> {
    if data.get(0..2) != Some(&[0xFF, 0xD8]) {
        return Err(RawError::malformed("lossless JPEG: missing SOI"));
    }
    let mut pos = 2;
    let mut frame: Option<Frame> = None;
    let mut comp_ids: Vec<u8> = Vec::new();
    // A lossless JPEG uses at most a handful of Huffman tables. Each Huffman::new allocates a
    // ~48 KiB lookup pair and fills it, so a stream that repeats DHT definitions could rebuild
    // tables thousands of times. Cap the cumulative builds to bound that header-processing work.
    const MAX_HUFFMAN_BUILDS: usize = 64;
    let mut huffman_builds = 0usize;
    let mut huff: [Option<Huffman>; 4] = [None, None, None, None];
    let mut restart = 0usize;
    loop {
        // Skip fill bytes up to the marker.
        while data.get(pos) == Some(&0xFF) && data.get(pos + 1) == Some(&0xFF) {
            pos += 1;
        }
        if data.get(pos) != Some(&0xFF) {
            return Err(RawError::malformed("lossless JPEG: expected a marker"));
        }
        let marker = *data.get(pos + 1).ok_or_else(|| RawError::malformed("truncated lossless JPEG"))?;
        pos += 2;
        if marker == 0xD9 {
            return Err(RawError::malformed("lossless JPEG: no scan"));
        }
        if (0xD0..=0xD7).contains(&marker) || marker == 0x01 {
            continue;
        }
        let len = be16(data, pos)?;
        if len < 2 {
            return Err(RawError::malformed("lossless JPEG: bad segment length"));
        }
        let seg = data.get(pos + 2..pos + len).ok_or_else(|| RawError::malformed("truncated lossless JPEG segment"))?;
        match marker {
            0xC3 => {
                if seg.len() < 6 {
                    return Err(RawError::malformed("short SOF3"));
                }
                let precision = seg[0];
                let height = usize::from(u16::from_be_bytes([seg[1], seg[2]]));
                let width = usize::from(u16::from_be_bytes([seg[3], seg[4]]));
                let nf = usize::from(seg[5]);
                if !(2..=16).contains(&precision) || width == 0 || height == 0 || nf == 0 || nf > 4 || seg.len() < 6 + 3 * nf {
                    return Err(RawError::malformed(format!("unsupported SOF3 ({precision}-bit, {width}x{height}, {nf} components)")));
                }
                comp_ids.clear();
                let mut factors = Vec::with_capacity(nf);
                for c in 0..nf {
                    let s = &seg[6 + 3 * c..9 + 3 * c];
                    factors.push(s[1]);
                    comp_ids.push(s[0]);
                }
                // Canon sRAW / mRAW: Y sampled 2×1 or 2×2, Cb and Cr 1×1, whole MCUs only.
                let (h, v) = match factors.as_slice() {
                    f if f.iter().all(|&s| s == 0x11) => (1, 1),
                    [y @ (0x21 | 0x22), 0x11, 0x11] if width % 2 == 0 && height % usize::from(y & 0x0F) == 0 => (2, usize::from(y & 0x0F)),
                    _ => return Err(RawError::unsupported(format!("lossless JPEG with subsampled components {factors:02x?} ({width}x{height})"))),
                };
                frame = Some(Frame { precision, width, height, components: nf, h, v });
            }
            0xC0..=0xC2 => return Err(RawError::unsupported("lossy (DCT) JPEG compressed raw data")),
            0xC5..=0xC7 | 0xC9..=0xCB | 0xCD..=0xCF => return Err(RawError::unsupported("hierarchical or arithmetic-coded JPEG")),
            0xC4 => {
                let mut p = 0;
                while p < seg.len() {
                    let tc_th = seg[p];
                    let id = usize::from(tc_th & 0x0F);
                    let counts: [u8; 16] = seg.get(p + 1..p + 17).and_then(|s| s.try_into().ok()).ok_or_else(|| RawError::malformed("short DHT"))?;
                    let total: usize = counts.iter().map(|&c| usize::from(c)).sum();
                    let vals = seg.get(p + 17..p + 17 + total).ok_or_else(|| RawError::malformed("short DHT"))?;
                    if id < 4 {
                        huffman_builds += 1;
                        if huffman_builds > MAX_HUFFMAN_BUILDS {
                            return Err(RawError::malformed("lossless JPEG defines too many Huffman tables"));
                        }
                        huff[id] = Some(Huffman::new(&counts, vals)?);
                    }
                    p += 17 + total;
                }
            }
            0xDD => {
                restart = be16(seg, 0)?;
            }
            0xDA => {
                let f = frame.ok_or_else(|| RawError::malformed("lossless JPEG: SOS before SOF3"))?;
                let ns = usize::from(*seg.first().ok_or_else(|| RawError::malformed("short SOS"))?);
                if ns != f.components || seg.len() < 1 + 2 * ns + 3 {
                    return Err(RawError::unsupported("lossless JPEG with non-interleaved scans"));
                }
                let mut tables: Vec<Huffman> = Vec::with_capacity(ns);
                let mut ids: Vec<usize> = Vec::with_capacity(ns);
                let mut comp_table = Vec::with_capacity(ns);
                for c in 0..ns {
                    let id = seg[1 + 2 * c];
                    if comp_ids.get(c) != Some(&id) {
                        return Err(RawError::unsupported("lossless JPEG scan component order"));
                    }
                    let td = usize::from(seg[2 + 2 * c] >> 4);
                    let h = huff.get(td).and_then(Option::as_ref).ok_or_else(|| RawError::malformed("missing Huffman table"))?;
                    match ids.iter().position(|&i| i == td) {
                        Some(k) => comp_table.push(k),
                        None => {
                            ids.push(td);
                            comp_table.push(tables.len());
                            tables.push(h.clone());
                        }
                    }
                }
                let predictor = seg[1 + 2 * ns];
                let point_transform = seg[3 + 2 * ns] & 0x0F;
                if !(1..=7).contains(&predictor) {
                    return Err(RawError::malformed(format!("lossless JPEG predictor {predictor}")));
                }
                if point_transform >= f.precision {
                    return Err(RawError::malformed("lossless JPEG point transform"));
                }
                let entropy = data.get(pos + len..).unwrap_or(&[]);
                return Ok(Parsed { frame: f, tables, comp_table, predictor, point_transform, restart, entropy });
            }
            _ => {} // APPn, COM, DQT, DNL… ignored
        }
        pos += len;
    }
}

/// Decodes a lossless JPEG stream. `max_samples` bounds the output size and
/// is checked before allocating.
pub(crate) fn decode(data: &[u8], max_samples: usize) -> Result<(Frame, Vec<u16>)> {
    let p = parse(data)?;
    let f = p.frame;
    if f.is_subsampled() {
        return Err(RawError::unsupported(format!("lossless JPEG with subsampled components ({}x{} luma)", f.h, f.v)));
    }
    let total = f
        .samples()
        .filter(|&n| n <= max_samples)
        .ok_or_else(|| RawError::LimitExceeded(format!("lossless JPEG frame {}x{}x{}", f.width, f.height, f.components)))?;
    // An entropy-coded sample needs at least one bit: a stream far shorter than
    // the frame cannot be valid (cheap decompression-bomb guard).
    if p.entropy.len().saturating_mul(8) < total / 2 {
        return Err(RawError::malformed("lossless JPEG data is truncated"));
    }
    let row_len = f.width * f.components;
    if p.restart != 0 && p.restart % f.width != 0 {
        return Err(RawError::unsupported("lossless JPEG restart interval that is not whole rows"));
    }
    let rows_per_interval = if p.restart == 0 { usize::MAX } else { p.restart / f.width };
    let mut out = vec![0u16; total];
    let mut bits = Bits::new(p.entropy);
    let nc = f.components;
    let tables: Vec<&Huffman> = p.comp_table.iter().map(|&k| &p.tables[k]).collect();
    let initial = 1i32 << (f.precision - p.point_transform - 1);
    let pred = p.predictor;
    let mut interval_row = 0usize;
    for y in 0..f.height {
        if interval_row == rows_per_interval {
            bits.restart()?;
            interval_row = 0;
        }
        let first_line = interval_row == 0;
        let (prev, cur) = if y == 0 {
            let (_, cur) = out.split_at_mut(0);
            (&[][..], &mut cur[..row_len])
        } else {
            let (a, b) = out.split_at_mut(y * row_len);
            (&a[(y - 1) * row_len..], &mut b[..row_len])
        };
        if pred == 1 || first_line {
            // Predictor 1 (or the first line of an interval): Ra after the first column.
            for c in 0..nc {
                let px = if first_line { initial } else { i32::from(prev[c]) };
                cur[c] = (px + bits.diff(tables[c])?) as u16;
            }
            // Ra: the previous pixel's sample of the same component, carried in registers.
            if nc == 1 {
                let t = tables[0];
                let mut ra = cur[0];
                for v in &mut cur[1..] {
                    ra = (i32::from(ra) + bits.diff(t)?) as u16;
                    *v = ra;
                }
            } else if nc == 2 {
                let (t0, t1) = (tables[0], tables[1]);
                let mut ra = [cur[0], cur[1]];
                for px in cur[2..].as_chunks_mut::<2>().0 {
                    ra[0] = (i32::from(ra[0]) + bits.diff(t0)?) as u16;
                    ra[1] = (i32::from(ra[1]) + bits.diff(t1)?) as u16;
                    *px = ra;
                }
            } else {
                for i in (nc..row_len).step_by(nc) {
                    for c in 0..nc {
                        cur[i + c] = (i32::from(cur[i + c - nc]) + bits.diff(tables[c])?) as u16;
                    }
                }
            }
            interval_row += 1;
            if bits.fake > 64 {
                return Err(RawError::malformed("lossless JPEG data is truncated"));
            }
            continue;
        }
        for x in 0..f.width {
            for (c, &table) in tables.iter().enumerate() {
                let i = x * nc + c;
                let px = if x == 0 {
                    if first_line { initial } else { i32::from(prev[i]) }
                } else {
                    let ra = i32::from(cur[i - nc]);
                    if first_line {
                        ra
                    } else {
                        let rb = i32::from(prev[i]);
                        let rc = i32::from(prev[i - nc]);
                        match pred {
                            1 => ra,
                            2 => rb,
                            3 => rc,
                            4 => ra + rb - rc,
                            5 => ra + ((rb - rc) >> 1),
                            6 => rb + ((ra - rc) >> 1),
                            _ => (ra + rb) >> 1,
                        }
                    }
                };
                let d = bits.diff(table)?;
                cur[i] = (px + d) as u16;
            }
        }
        interval_row += 1;
        if bits.fake > 64 {
            return Err(RawError::malformed("lossless JPEG data is truncated"));
        }
    }
    if p.point_transform > 0 {
        for v in &mut out {
            *v <<= p.point_transform;
        }
    }
    Ok((f, out))
}

/// Decodes a subsampled frame (Canon sRAW / mRAW: Y sampled 2×1 or 2×2, Cb and Cr 1×1, see
/// [`Frame::h`]) into its sample sequence in decode order: per MCU the `h × v` Y samples
/// row-major, then Cb, then Cr; `width / h` MCUs make a row and `height / v` rows the frame.
///
/// Only predictor 1 is supported, with Canon's rule instead of T.81's per-component sample
/// grid: each Y sample is predicted from the Y sample decoded just before it (the previous one
/// in the MCU, or the last of the previous MCU), Cb and Cr from those of the previous MCU, and
/// the first MCU of a row from the first MCU of the row above (from `2^(P−Pt−1)` on the first
/// row of a restart interval).
pub(crate) fn decode_subsampled(data: &[u8], max_samples: usize) -> Result<(Frame, Vec<u16>)> {
    let p = parse(data)?;
    let f = p.frame;
    let total = f
        .samples()
        .filter(|&n| n <= max_samples)
        .ok_or_else(|| RawError::LimitExceeded(format!("lossless JPEG frame {}x{}x{}", f.width, f.height, f.components)))?;
    if !f.is_subsampled() || f.components != 3 {
        return Err(RawError::malformed("lossless JPEG frame is not subsampled YCbCr"));
    }
    if p.predictor != 1 {
        return Err(RawError::unsupported(format!("subsampled lossless JPEG with predictor {}", p.predictor)));
    }
    if p.entropy.len().saturating_mul(8) < total / 2 {
        return Err(RawError::malformed("lossless JPEG data is truncated"));
    }
    // Each component's table (components may share one).
    let [ty, tcb, tcr] = match p.comp_table.as_slice() {
        &[a, b, c] => [&p.tables[a], &p.tables[b], &p.tables[c]],
        _ => return Err(RawError::malformed("lossless JPEG scan tables")),
    };
    let luma = f.h * f.v;
    let per_mcu = luma + 2;
    let mcus = f.width / f.h;
    let row_len = mcus * per_mcu;
    if row_len == 0 {
        return Err(RawError::malformed("empty lossless JPEG frame"));
    }
    // The restart interval counts MCUs.
    if p.restart != 0 && p.restart % mcus != 0 {
        return Err(RawError::unsupported("lossless JPEG restart interval that is not whole rows"));
    }
    let rows_per_interval = if p.restart == 0 { usize::MAX } else { p.restart / mcus };
    let mut out = vec![0u16; total];
    let mut bits = Bits::new(p.entropy);
    let initial = 1i32 << (f.precision - p.point_transform - 1);
    // First Y, Cb and Cr of the row above: the first MCU's predictors.
    let mut top = [initial; 3];
    let mut interval_row = 0usize;
    for row in out.chunks_exact_mut(row_len) {
        if interval_row == rows_per_interval {
            bits.restart()?;
            interval_row = 0;
        }
        if interval_row == 0 {
            top = [initial; 3];
        }
        let (mut y, mut cb, mut cr) = (top[0], top[1], top[2]);
        for (m, mcu) in row.chunks_exact_mut(per_mcu).enumerate() {
            let (ys, c) = mcu.split_at_mut(luma);
            for (k, s) in ys.iter_mut().enumerate() {
                // The first Y of the row comes from the row above, every other from the previous Y.
                let pred = if m == 0 && k == 0 { top[0] } else { y };
                *s = (pred + bits.diff(ty)?) as u16;
                y = i32::from(*s);
                if m == 0 && k == 0 {
                    top[0] = y;
                }
            }
            if let [scb, scr] = c {
                *scb = (cb + bits.diff(tcb)?) as u16;
                *scr = (cr + bits.diff(tcr)?) as u16;
                (cb, cr) = (i32::from(*scb), i32::from(*scr));
            }
            if m == 0 {
                (top[1], top[2]) = (cb, cr);
            }
        }
        interval_row += 1;
        if bits.fake > 64 {
            return Err(RawError::malformed("lossless JPEG data is truncated"));
        }
    }
    if p.point_transform > 0 {
        for v in &mut out {
            *v <<= p.point_transform;
        }
    }
    Ok((f, out))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testgen::{lj92_encode, lj92_encode_sraw, lj92_encode_sraw_tables, lj92_encode_tables};

    /// Deterministic pseudo-random samples with some structure (smooth + noise).
    fn samples(n: usize, precision: u8, seed: u64) -> Vec<u16> {
        let mut s = seed;
        let max = (1u32 << precision) - 1;
        (0..n)
            .map(|i| {
                s = s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                let noise = (s >> 33) as u32 % 64;
                let base = (i as u32 * 37) % (max + 1);
                ((base + noise) % (max + 1)) as u16
            })
            .collect()
    }

    #[test]
    fn roundtrip_all_predictors_and_precisions() {
        for precision in [2u8, 8, 12, 14, 16] {
            for predictor in 1..=7u8 {
                for comps in 1..=4usize {
                    let (w, h) = (13, 7);
                    let v = samples(w * h * comps, precision, u64::from(predictor) * 31 + comps as u64);
                    let j = lj92_encode(&v, w, h, comps, precision, predictor, None);
                    let (f, out) = decode(&j, usize::MAX).unwrap();
                    assert_eq!(f, Frame { precision, width: w, height: h, components: comps, h: 1, v: 1 });
                    assert_eq!(out, v, "P={precision} pred={predictor} comps={comps}");
                }
            }
        }
    }

    #[test]
    fn roundtrip_with_restarts_and_extreme_values() {
        let (w, h) = (16, 9);
        // Alternating 0 / 65535 forces the 16-bit and 32768 difference categories.
        let v: Vec<u16> = (0..w * h * 2).map(|i| if (i / 3) % 2 == 0 { 0 } else { 65535 }).collect();
        for rows in [1, 2, 4] {
            let j = lj92_encode(&v, w, h, 2, 16, 1, Some(rows));
            let (_, out) = decode(&j, usize::MAX).unwrap();
            assert_eq!(out, v, "restart every {rows} rows");
        }
    }

    /// Real CR2 frames declare one DHT per component. Each case gives the components
    /// different tables (or a mixed sharing), so decoding with the wrong table for a
    /// component, or one table for all, corrupts the output.
    #[test]
    fn roundtrip_with_one_huffman_table_per_component() {
        let (w, h) = (13, 7);
        for table_of in [&[0u8, 1][..], &[1, 0], &[0, 1, 2, 3], &[0, 1, 0, 1], &[0, 0, 1, 1], &[3, 2, 1, 0]] {
            let comps = table_of.len();
            for predictor in [1u8, 4, 7] {
                for restart_rows in [None, Some(2)] {
                    let v = samples(w * h * comps, 14, u64::from(predictor) * 7 + comps as u64);
                    let j = lj92_encode_tables(&v, w, h, table_of, 14, predictor, restart_rows);
                    let (f, out) = decode(&j, usize::MAX).unwrap();
                    assert_eq!(f, Frame { precision: 14, width: w, height: h, components: comps, h: 1, v: 1 });
                    assert_eq!(out, v, "tables={table_of:?} pred={predictor} restart={restart_rows:?}");
                }
            }
        }
    }

    #[test]
    fn roundtrip_per_component_tables_with_16_bit_extremes() {
        let (w, h) = (16, 9);
        for table_of in [&[0u8, 1][..], &[0, 1, 0, 1]] {
            let comps = table_of.len();
            // Alternating 0 / 65535 forces the 16-bit and 32768 difference categories.
            let v: Vec<u16> = (0..w * h * comps).map(|i| if (i / 3) % 2 == 0 { 0 } else { 65535 }).collect();
            for (predictor, restart_rows) in [(1u8, Some(1)), (1, Some(4)), (6, Some(2)), (6, None)] {
                let j = lj92_encode_tables(&v, w, h, table_of, 16, predictor, restart_rows);
                let (_, out) = decode(&j, usize::MAX).unwrap();
                assert_eq!(out, v, "tables={table_of:?} pred={predictor} restart={restart_rows:?}");
            }
        }
    }

    #[test]
    fn hostile_streams_error_without_panicking() {
        let v = samples(32 * 8, 14, 7);
        let j = lj92_encode(&v, 32, 8, 1, 14, 1, Some(2));
        for n in 0..j.len() {
            let _ = decode(&j[..n], usize::MAX);
        }
        let mut s = 0x1234_5678u64;
        for _ in 0..2000 {
            let mut b = j.clone();
            for _ in 0..4 {
                s = s.wrapping_mul(6364136223846793005).wrapping_add(1);
                let at = (s >> 33) as usize % b.len();
                b[at] = (s >> 20) as u8;
            }
            let _ = decode(&b, 1 << 20);
        }
        // A frame larger than allowed is refused before allocating.
        assert!(matches!(decode(&j, 10), Err(RawError::LimitExceeded(_))));
        // A huge frame with a tiny entropy segment is refused as truncated.
        let mut big = j.clone();
        big[7..9].copy_from_slice(&60000u16.to_be_bytes()); // height
        big[9..11].copy_from_slice(&60000u16.to_be_bytes()); // width
        assert!(decode(&big, usize::MAX).is_err());
        // Lossy JPEG markers are reported as unsupported.
        let lossy = [0xFF, 0xD8, 0xFF, 0xC0, 0x00, 0x0B, 8, 0, 1, 0, 1, 1, 1, 0x11, 0];
        assert!(matches!(decode(&lossy, usize::MAX), Err(RawError::Unsupported(_))));
    }

    #[test]
    fn too_many_huffman_tables_are_rejected() {
        // A valid stream still decodes; the same stream with thousands of repeated DHT
        // definitions injected after the SOI is rejected before rebuilding tables endlessly.
        let (w, h) = (8, 8);
        let v = samples(w * h, 12, 5);
        let j = lj92_encode(&v, w, h, 1, 12, 1, None);
        assert!(decode(&j, usize::MAX).is_ok());

        // Minimal DHT for id 0 (one length-1 code, value 0): FF C4, len=0x0014, then
        // tc_th, 16 count bytes, one value.
        let mut dht = vec![0xFF, 0xC4, 0x00, 0x14, 0x00];
        dht.extend_from_slice(&[1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]); // counts
        dht.push(0x00); // one value
        let mut flooded = j[..2].to_vec(); // SOI
        for _ in 0..5000 {
            flooded.extend_from_slice(&dht);
        }
        flooded.extend_from_slice(&j[2..]); // the rest of the valid stream
        assert!(matches!(decode(&flooded, usize::MAX), Err(RawError::Malformed(_))));
    }

    #[test]
    fn subsampled_roundtrip() {
        for (h, v) in [(2usize, 2usize), (2, 1)] {
            let (w, ht) = (14, 6);
            let n = (w / h) * (ht / v) * (h * v + 2);
            for precision in [12u8, 15, 16] {
                for restart in [None, Some(1), Some(ht / v)] {
                    let seq = samples(n, precision, h as u64 * 7 + v as u64);
                    let j = lj92_encode_sraw(&seq, w, ht, h, v, precision, restart);
                    let f = frame(&j).unwrap();
                    assert_eq!((f.h, f.v, f.samples()), (h, v, Some(n)));
                    let (_, out) = decode_subsampled(&j, usize::MAX).unwrap();
                    assert_eq!(out, seq, "{h}x{v} P={precision} restart {restart:?}");
                    // The plain decoder refuses subsampled frames instead of mislaying them.
                    assert!(matches!(decode(&j, usize::MAX), Err(RawError::Unsupported(_))));
                }
            }
        }
        // Extreme values exercise the 16-bit and 32768 difference categories.
        let seq: Vec<u16> = (0..7 * 3 * 6).map(|i| if (i / 5) % 2 == 0 { 0 } else { 65535 }).collect();
        let j = lj92_encode_sraw(&seq, 14, 6, 2, 2, 16, Some(1));
        assert_eq!(decode_subsampled(&j, usize::MAX).unwrap().1, seq);
        // Y and the chroma on separate tables, or each its own: the scan maps every component
        // to its table even when components share one.
        for table_of in [[0, 1, 1], [0, 1, 2], [1, 0, 0]] {
            let seq = samples(7 * 3 * 6, 15, 5);
            let j = lj92_encode_sraw_tables(&seq, 14, 6, (2, 2), table_of, 15, Some(1));
            assert_eq!(decode_subsampled(&j, usize::MAX).unwrap().1, seq, "tables {table_of:?}");
        }
    }

    #[test]
    fn subsampled_hostile_streams_error_without_panicking() {
        let seq = samples(7 * 3 * 6, 15, 3);
        let j = lj92_encode_sraw(&seq, 14, 6, 2, 2, 15, Some(1));
        for n in 0..j.len() {
            let _ = decode_subsampled(&j[..n], usize::MAX);
        }
        let mut s = 0x0123_4567u64;
        for _ in 0..2000 {
            let mut b = j.clone();
            for _ in 0..4 {
                s = s.wrapping_mul(6364136223846793005).wrapping_add(1);
                let at = (s >> 33) as usize % b.len();
                b[at] = (s >> 20) as u8;
            }
            let _ = decode_subsampled(&b, 1 << 20);
        }
        assert!(matches!(decode_subsampled(&j, 10), Err(RawError::LimitExceeded(_))));
        // A frame that is not subsampled, or another predictor, is not Canon sRAW.
        let plain = lj92_encode(&samples(14 * 6 * 3, 15, 1), 14, 6, 3, 15, 1, None);
        assert!(decode_subsampled(&plain, usize::MAX).is_err());
        let mut p6 = j.clone();
        let sos = p6.windows(2).position(|w| w == [0xFF, 0xDA]).unwrap();
        p6[sos + 11] = 6; // Ss (predictor) after Ns and three component selectors
        assert!(matches!(decode_subsampled(&p6, usize::MAX), Err(RawError::Unsupported(_))));
        // Sampling layouts other than Canon's are refused at the frame header.
        let mut odd = j.clone();
        let sof = odd.windows(2).position(|w| w == [0xFF, 0xC3]).unwrap();
        odd[sof + 14] = 0x21; // Cb subsampled too
        assert!(matches!(frame(&odd), Err(RawError::Unsupported(_))));
    }
}
