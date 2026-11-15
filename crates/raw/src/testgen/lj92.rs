/// Huffman code lengths assigned to categories in decreasing frequency
/// order. Kraft sum < 1 and no code is all ones.
const LENGTHS: [u8; 17] = [2, 3, 3, 3, 4, 4, 5, 5, 6, 6, 7, 8, 9, 10, 11, 12, 13];

fn category(d: i32) -> u32 {
    if d == 0 {
        0
    } else if d == 32768 {
        16
    } else {
        32 - d.unsigned_abs().leading_zeros()
    }
}

struct BitWriter {
    out: Vec<u8>,
    acc: u32,
    n: u32,
}

impl BitWriter {
    fn put(&mut self, v: u32, bits: u32) {
        for i in (0..bits).rev() {
            self.acc = (self.acc << 1) | ((v >> i) & 1);
            self.n += 1;
            if self.n == 8 {
                let b = self.acc as u8;
                self.out.push(b);
                if b == 0xFF {
                    self.out.push(0);
                }
                self.acc = 0;
                self.n = 0;
            }
        }
    }
    fn flush(&mut self) {
        if self.n > 0 {
            let pad = 8 - self.n;
            self.put((1 << pad) - 1, pad);
        }
    }
}

/// Encodes `samples` (raster order, `components` interleaved, values below
/// `2^precision`) as a lossless JPEG with the given predictor (1–7) and an
/// optional restart interval in rows. All components share one Huffman table.
pub fn lj92_encode(samples: &[u16], width: usize, height: usize, components: usize, precision: u8, predictor: u8, restart_rows: Option<usize>) -> Vec<u8> {
    lj92_encode_tables(samples, width, height, &vec![0; components], precision, predictor, restart_rows)
}

/// Like [`lj92_encode`], but component `c` uses Huffman table `table_of[c]`
/// (0–3; the component count is `table_of.len()`), one DHT per table id as
/// CR2 writes them. Table `t` assigns its code lengths rotated by `t`, so the
/// tables differ even when their statistics match.
pub fn lj92_encode_tables(samples: &[u16], width: usize, height: usize, table_of: &[u8], precision: u8, predictor: u8, restart_rows: Option<usize>) -> Vec<u8> {
    let components = table_of.len();
    assert_eq!(samples.len(), width * height * components);
    let row = width * components;
    let interval = restart_rows.unwrap_or(usize::MAX);
    // Differences per sample, following T.81 H.1.2.1 prediction rules.
    let mut diffs = Vec::with_capacity(samples.len());
    let initial = 1i32 << (precision - 1);
    for y in 0..height {
        let first_line = y % interval == 0;
        for x in 0..width {
            for c in 0..components {
                let i = y * row + x * components + c;
                let px = if x == 0 {
                    if first_line { initial } else { i32::from(samples[i - row]) }
                } else if first_line {
                    i32::from(samples[i - components])
                } else {
                    let ra = i32::from(samples[i - components]);
                    let rb = i32::from(samples[i - row]);
                    let rc = i32::from(samples[i - row - components]);
                    match predictor {
                        1 => ra,
                        2 => rb,
                        3 => rc,
                        4 => ra + rb - rc,
                        5 => ra + ((rb - rc) >> 1),
                        6 => rb + ((ra - rc) >> 1),
                        _ => (ra + rb) >> 1,
                    }
                };
                diffs.push(wrap_diff(i32::from(samples[i]) - px));
            }
        }
    }
    let frame = Frame {
        width,
        height,
        factors: &vec![0x11; components],
        table_of,
        pattern: &(0..components as u8).collect::<Vec<_>>(),
        precision,
        predictor,
        restart: restart_rows.map(|r| (r * width, interval.saturating_mul(row))),
    };
    lj92_stream(&diffs, &frame)
}

/// A difference modulo 2^16, in −32767..=32768 (T.81 H.1.2.2).
fn wrap_diff(d: i32) -> i32 {
    let d = d & 0xFFFF;
    if d > 32768 { d - 65536 } else { d }
}

/// Encodes a Canon sRAW / mRAW frame: `seq` is the sample sequence in decode order (per MCU the
/// `h × v` Y samples, then Cb and Cr; `width / h` MCUs a row), predicted as
/// `ljpeg::decode_subsampled` describes: each Y from the previous Y, Cb and Cr from the previous
/// MCU's, the first MCU of a row from the first of the row above. `restart_rows` counts MCU rows.
pub fn lj92_encode_sraw(seq: &[u16], width: usize, height: usize, h: usize, v: usize, precision: u8, restart_rows: Option<usize>) -> Vec<u8> {
    lj92_encode_sraw_tables(seq, width, height, (h, v), [0, 0, 0], precision, restart_rows)
}

/// Like [`lj92_encode_sraw`], but Y, Cb and Cr use Huffman tables `table_of` (0–3).
pub fn lj92_encode_sraw_tables(
    seq: &[u16],
    width: usize,
    height: usize,
    (h, v): (usize, usize),
    table_of: [u8; 3],
    precision: u8,
    restart_rows: Option<usize>,
) -> Vec<u8> {
    let (luma, mcus) = (h * v, width / h);
    let per_mcu = luma + 2;
    let row_len = mcus * per_mcu;
    assert_eq!(seq.len(), row_len * (height / v));
    let interval = restart_rows.unwrap_or(usize::MAX);
    let initial = 1i32 << (precision - 1);
    let mut diffs = Vec::with_capacity(seq.len());
    let mut top = [initial; 3];
    for (r, row) in seq.chunks_exact(row_len).enumerate() {
        if r % interval == 0 {
            top = [initial; 3];
        }
        let (mut y, mut cb, mut cr) = (top[0], top[1], top[2]);
        for (m, mcu) in row.chunks_exact(per_mcu).enumerate() {
            for (k, &s) in mcu[..luma].iter().enumerate() {
                let pred = if m == 0 && k == 0 { top[0] } else { y };
                diffs.push(wrap_diff(i32::from(s) - pred));
                y = i32::from(s);
                if m == 0 && k == 0 {
                    top[0] = y;
                }
            }
            diffs.push(wrap_diff(i32::from(mcu[luma]) - cb));
            diffs.push(wrap_diff(i32::from(mcu[luma + 1]) - cr));
            (cb, cr) = (i32::from(mcu[luma]), i32::from(mcu[luma + 1]));
            if m == 0 {
                (top[1], top[2]) = (cb, cr);
            }
        }
    }
    let mut pattern = vec![0u8; luma];
    pattern.extend_from_slice(&[1, 2]);
    let frame = Frame {
        width,
        height,
        factors: &[(h as u8) << 4 | v as u8, 0x11, 0x11],
        table_of: &table_of,
        pattern: &pattern,
        precision,
        predictor: 1,
        restart: restart_rows.map(|r| (r * mcus, interval.saturating_mul(row_len))),
    };
    lj92_stream(&diffs, &frame)
}

/// The frame header [`lj92_stream`] writes around the differences.
struct Frame<'a> {
    width: usize,
    height: usize,
    /// Each component's sampling factors (`h << 4 | v`).
    factors: &'a [u8],
    /// Each component's Huffman table id (0–3).
    table_of: &'a [u8],
    /// The component of each difference, repeating: `0, 1, 2` interleaved, or an MCU's Y samples
    /// then Cb and Cr.
    pattern: &'a [u8],
    precision: u8,
    predictor: u8,
    /// The restart interval `(DRI value, differences per interval)`.
    restart: Option<(usize, usize)>,
}

/// Writes SOI, SOF3, one DHT per table id, the restart interval, SOS and the entropy-coded
/// `diffs`.
fn lj92_stream(diffs: &[i32], f: &Frame<'_>) -> Vec<u8> {
    let components = f.factors.len();
    assert_eq!(f.table_of.len(), components);
    assert!(f.table_of.iter().all(|&t| t < 4));
    let table = |i: usize| f.table_of[usize::from(f.pattern[i % f.pattern.len()])];
    // Tables: categories ordered by frequency get increasing code lengths.
    let mut ids: Vec<u8> = f.table_of.to_vec();
    ids.sort_unstable();
    ids.dedup();
    let mut code_of = [[(0u32, 0u32); 17]; 4];
    let mut dhts = Vec::new();
    for &t in &ids {
        let mut freq = [0usize; 17];
        for (i, &d) in diffs.iter().enumerate() {
            if table(i) == t {
                freq[category(d) as usize] += 1;
            }
        }
        let mut order: Vec<usize> = (0..17).collect();
        order.sort_by_key(|&c| std::cmp::Reverse(freq[c]));
        order.rotate_left(usize::from(t));
        let mut len_of = [0u8; 17];
        for (rank, &c) in order.iter().enumerate() {
            len_of[c] = LENGTHS[rank];
        }
        let mut counts = [0u8; 16];
        let mut vals: Vec<u8> = Vec::new();
        let mut code = 0u32;
        for len in 1..=16u8 {
            for c in 0..17 {
                if len_of[c] == len {
                    counts[len as usize - 1] += 1;
                    vals.push(c as u8);
                    code_of[usize::from(t)][c] = (code, u32::from(len));
                    code += 1;
                }
            }
            code <<= 1;
        }
        dhts.push((t, counts, vals));
    }
    let mut out = vec![0xFF, 0xD8];
    // SOF3
    out.extend_from_slice(&[0xFF, 0xC3]);
    out.extend_from_slice(&((8 + 3 * components) as u16).to_be_bytes());
    out.push(f.precision);
    out.extend_from_slice(&(f.height as u16).to_be_bytes());
    out.extend_from_slice(&(f.width as u16).to_be_bytes());
    out.push(components as u8);
    for (c, &factor) in f.factors.iter().enumerate() {
        out.extend_from_slice(&[c as u8 + 1, factor, 0]);
    }
    // DHT, one per table id
    for (t, counts, vals) in &dhts {
        out.extend_from_slice(&[0xFF, 0xC4]);
        out.extend_from_slice(&((2 + 17 + vals.len()) as u16).to_be_bytes());
        out.push(*t);
        out.extend_from_slice(counts);
        out.extend_from_slice(vals);
    }
    if let Some((dri, _)) = f.restart {
        out.extend_from_slice(&[0xFF, 0xDD, 0, 4]);
        out.extend_from_slice(&(dri as u16).to_be_bytes());
    }
    let every = f.restart.map_or(usize::MAX, |(_, n)| n);
    // SOS
    out.extend_from_slice(&[0xFF, 0xDA]);
    out.extend_from_slice(&((6 + 2 * components) as u16).to_be_bytes());
    out.push(components as u8);
    for (c, &t) in f.table_of.iter().enumerate() {
        out.extend_from_slice(&[c as u8 + 1, t << 4]);
    }
    out.extend_from_slice(&[f.predictor, 0, 0]);
    let mut w = BitWriter { out, acc: 0, n: 0 };
    let mut rst = 0u8;
    for (i, &d) in diffs.iter().enumerate() {
        if i > 0 && i % every == 0 {
            w.flush();
            w.out.extend_from_slice(&[0xFF, 0xD0 + rst]);
            rst = (rst + 1) % 8;
        }
        let s = category(d);
        let (c, l) = code_of[usize::from(table(i))][s as usize];
        w.put(c, l);
        if (1..16).contains(&s) {
            let extra = if d < 0 { (d - 1) as u32 & ((1 << s) - 1) } else { d as u32 };
            w.put(extra, s);
        }
    }
    w.flush();
    let mut out = w.out;
    out.extend_from_slice(&[0xFF, 0xD9]);
    out
}
