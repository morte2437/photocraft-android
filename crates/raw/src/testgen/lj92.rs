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
    assert!(table_of.iter().all(|&t| t < 4));
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
                let mut d = (i32::from(samples[i]) - px) & 0xFFFF;
                if d > 32768 {
                    d -= 65536;
                }
                diffs.push(d);
            }
        }
    }
    // Tables: categories ordered by frequency get increasing code lengths.
    let mut ids: Vec<u8> = table_of.to_vec();
    ids.sort_unstable();
    ids.dedup();
    let mut code_of = [[(0u32, 0u32); 17]; 4];
    let mut dhts = Vec::new();
    for &t in &ids {
        let mut freq = [0usize; 17];
        for (i, &d) in diffs.iter().enumerate() {
            if table_of[i % components] == t {
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
    out.push(precision);
    out.extend_from_slice(&(height as u16).to_be_bytes());
    out.extend_from_slice(&(width as u16).to_be_bytes());
    out.push(components as u8);
    for c in 0..components {
        out.extend_from_slice(&[c as u8 + 1, 0x11, 0]);
    }
    // DHT, one per table id
    for (t, counts, vals) in &dhts {
        out.extend_from_slice(&[0xFF, 0xC4]);
        out.extend_from_slice(&((2 + 17 + vals.len()) as u16).to_be_bytes());
        out.push(*t);
        out.extend_from_slice(counts);
        out.extend_from_slice(vals);
    }
    if let Some(r) = restart_rows {
        out.extend_from_slice(&[0xFF, 0xDD, 0, 4]);
        out.extend_from_slice(&((r * width) as u16).to_be_bytes());
    }
    // SOS
    out.extend_from_slice(&[0xFF, 0xDA]);
    out.extend_from_slice(&((6 + 2 * components) as u16).to_be_bytes());
    out.push(components as u8);
    for (c, &t) in table_of.iter().enumerate() {
        out.extend_from_slice(&[c as u8 + 1, t << 4]);
    }
    out.extend_from_slice(&[predictor, 0, 0]);
    let mut w = BitWriter { out, acc: 0, n: 0 };
    let mut rst = 0u8;
    for (i, &d) in diffs.iter().enumerate() {
        if i > 0 && i % (interval.saturating_mul(row)) == 0 {
            w.flush();
            w.out.extend_from_slice(&[0xFF, 0xD0 + rst]);
            rst = (rst + 1) % 8;
        }
        let s = category(d);
        let (c, l) = code_of[usize::from(table_of[i % components])][s as usize];
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
