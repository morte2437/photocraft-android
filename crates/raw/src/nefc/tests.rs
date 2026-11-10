use super::*;

/// MSB-first bit writer for the test encoder.
#[derive(Default)]
struct Writer {
    out: Vec<u8>,
    acc: u64,
    n: u32,
}

impl Writer {
    fn put(&mut self, v: u32, len: u32) {
        for i in (0..len).rev() {
            self.acc = (self.acc << 1) | u64::from((v >> i) & 1);
            self.n += 1;
            if self.n == 8 {
                self.out.push(self.acc as u8);
                self.acc = 0;
                self.n = 0;
            }
        }
    }
    fn finish(mut self) -> Vec<u8> {
        if self.n > 0 {
            self.out.push((self.acc << (8 - self.n)) as u8);
        }
        self.out.extend_from_slice(&[0; 4]);
        self.out
    }
}

/// `(code, length)` per category, from the canonical table.
fn codes(t: &Table) -> Vec<(u32, u32)> {
    let mut out = vec![(0, 0); 17];
    let mut code = 0u32;
    let mut symbols = t.symbols.iter();
    for (i, &n) in t.counts.iter().enumerate() {
        for _ in 0..n {
            let s = *symbols.next().unwrap();
            if s != UNSEEN {
                out[s as usize] = (code, i as u32 + 1);
            }
            code += 1;
        }
        code <<= 1;
    }
    out
}

/// Encodes `values` (samples or curve indices) the way the camera does.
fn encode(values: &[i32], w: usize, t: &Table, seeds: [[i32; 2]; 2]) -> Vec<u8> {
    let codes = codes(t);
    let mut wr = Writer::default();
    let mut vpred = seeds;
    for (y, row) in values.chunks(w).enumerate() {
        for (x, &v) in row.iter().enumerate() {
            let pred = if x < 2 { vpred[y & 1][x] } else { row[x - 2] };
            if x < 2 {
                vpred[y & 1][x] = v;
            }
            let d = v - pred;
            let k = 32 - d.unsigned_abs().leading_zeros();
            let (code, len) = codes[k as usize];
            assert!(len > 0, "no code for category {k}");
            wr.put(code, len);
            wr.put(if d < 0 { (d + (1 << k) - 1) as u32 } else { d as u32 }, k);
        }
    }
    wr.finish()
}

/// Smooth gradient + texture + hard edges, within `0..=max`.
fn image(w: usize, h: usize, max: i32, seed: u32) -> Vec<i32> {
    let mut s = seed;
    (0..w * h)
        .map(|i| {
            let (x, y) = ((i % w) as i32, (i / w) as i32);
            s = s.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let noise = (s >> 24) as i32 % 9 - 4;
            let base = (x * 37 + y * 11) % (max / 2) + if (x / 7 + y / 5) % 3 == 0 { max / 3 } else { 0 };
            (base + noise).clamp(0, max)
        })
        .collect()
}

fn lossless(bits: u32) -> DecodeTable {
    let s = 1 << (bits - 3);
    DecodeTable { encoding: Encoding::Lossless, seeds: [[s, s], [s, s]], split: 0 }
}

fn lossy_table(bits: u32, le: bool) -> Vec<u8> {
    let mut t = vec![0x44, 0x20];
    let put = |t: &mut Vec<u8>, v: u16| t.extend_from_slice(&if le { v.to_le_bytes() } else { v.to_be_bytes() });
    for _ in 0..4 {
        put(&mut t, 300);
    }
    put(&mut t, 257);
    let max = f64::from((1u32 << bits) - 1);
    for i in 0..257u32 {
        put(&mut t, (max * (f64::from(i) / 64.0).min(1.0).sqrt()) as u16);
    }
    t.resize(624, 0);
    t
}

#[test]
fn tables_are_prefix_codes() {
    for t in [LOSSLESS_12, LOSSLESS_14, LOSSY_12, LOSSY_14] {
        assert_eq!(t.counts.iter().map(|&c| c as usize).sum::<usize>(), t.symbols.len());
        let kraft: f64 = t.counts.iter().enumerate().map(|(i, &c)| f64::from(c) / f64::from(1u32 << (i + 1))).sum();
        assert!(kraft <= 1.0);
        assert!(Huffman::new(&t).is_ok());
    }
}

#[test]
fn lossless_round_trip_12_and_14() {
    for (bits, t) in [(12, LOSSLESS_12), (14, LOSSLESS_14)] {
        let (w, h) = (64, 9);
        let max = (1 << bits) - 1;
        let mut img = image(w, h, max, bits);
        img[5] = 0;
        img[7] = max;
        img[w + 9] = max;
        let table = lossless(bits);
        let out = decode(&encode(&img, w, &t, table.seeds), w, h, bits, &table).unwrap();
        assert_eq!(out, img.iter().map(|&v| v as u16).collect::<Vec<_>>(), "{bits}-bit");
    }
}

#[test]
fn lossy_type2_curve_and_round_trip() {
    for (bits, t) in [(12u32, LOSSY_12), (14, LOSSY_14)] {
        for le in [false, true] {
            let table = parse_table(&lossy_table(bits, le), le, bits).unwrap();
            assert_eq!(table.seeds, [[300, 300], [300, 300]]);
            let Encoding::Lossy(curve) = &table.encoding else { panic!() };
            let step = (1usize << bits) / 256;
            assert_eq!(curve.len(), (1 << bits) + 1);
            assert_eq!(curve[64 * step], (1 << bits) - 1);
            assert!(curve.windows(2).all(|p| p[0] <= p[1]));
            let (w, h) = (48, 7);
            let img = image(w, h, 64 * step as i32 - 1, 7);
            let out = decode(&encode(&img, w, &t, table.seeds), w, h, bits, &table).unwrap();
            assert_eq!(out, img.iter().map(|&v| curve[v as usize]).collect::<Vec<_>>());
        }
    }
}

#[test]
fn lossy_type1_full_curve() {
    let mut t = vec![0x44, 0x10, 0, 10, 0, 10, 0, 10, 0, 10, 0, 5];
    for v in [0u16, 100, 400, 900, 1600] {
        t.extend_from_slice(&v.to_be_bytes());
    }
    let table = parse_table(&t, false, 12).unwrap();
    assert_eq!(table.encoding, Encoding::Lossy(vec![0, 100, 400, 900, 1600]));
    let img = vec![0, 4, 1, 3, 2, 2, 4, 0];
    assert_eq!(decode(&encode(&img, 4, &LOSSY_12, table.seeds), 4, 2, 12, &table).unwrap(), vec![0, 1600, 100, 900, 400, 400, 1600, 0]);
}

#[test]
fn unsupported_and_malformed_tables() {
    let unsupported = |r: Result<DecodeTable>| matches!(r, Err(RawError::Unsupported(_)));
    let malformed = |r: Result<DecodeTable>| matches!(r, Err(RawError::Malformed(_)));
    let mut split = lossy_table(12, false);
    split[562..564].copy_from_slice(&345u16.to_be_bytes());
    assert_eq!(parse_table(&split, false, 12).unwrap().split, 345);
    assert!(unsupported(parse_table(&[0x49, 0x30, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0], false, 12)));
    assert!(unsupported(parse_table(&lossy_table(12, false), false, 16)));
    assert!(malformed(parse_table(&[], false, 12)));
    assert!(malformed(parse_table(&[0x46, 0x30, 8], false, 14)));
    assert!(malformed(parse_table(&lossy_table(12, false)[..100], false, 12)));
    let mut odd = lossy_table(12, false);
    odd[10..12].copy_from_slice(&300u16.to_be_bytes());
    assert!(parse_table(&odd, false, 12).is_err());
    assert!(parse_table(&[0x44, 0x20, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0], false, 12).is_err());
}

#[test]
fn modern_lossy_curve_preserves_sensor_adus_at_both_depths() {
    for bits in [12u32, 14] {
        for le in [false, true] {
            let put = |v: u16| if le { v.to_le_bytes() } else { v.to_be_bytes() };
            let mut t = vec![0x44, 0x40];
            for _ in 0..4 {
                t.extend(put(256));
            }
            t.extend(put(257));
            let white = (1u16 << bits) - 1;
            for i in 0..=256u32 {
                t.extend(put(((i * 64).min(u32::from(white))) as u16));
            }
            t.resize(624, 0);
            let parsed = parse_table(&t, le, bits).unwrap();
            let Encoding::Lossy(curve) = &parsed.encoding else { panic!() };
            let range = 1usize << (bits - 2);
            assert_eq!(curve.len(), range + 1);
            assert_eq!(curve[range], white);
            // Stored point 16 is 1024 ADU. It belongs to encoded code 256
            // at 14 bits, code 64 at 12 bits, independent of end-point clipping.
            assert_eq!(curve[range / 16], 1024.min(white));
            let indices = vec![0, (range / 16) as i32, (range / 8) as i32, (range / 32) as i32];
            let huffman = if bits == 12 { &LOSSY_12 } else { &LOSSY_14 };
            let src = encode(&indices, 2, huffman, parsed.seeds);
            assert_eq!(decode(&src, 2, 2, bits, &parsed).unwrap(), vec![0, 1024, 2048, 512]);
        }
    }
}

/// A 14-bit lossless table (version 0x46) with the given seeds.
fn lossless_table(seed: u16, le: bool) -> Vec<u8> {
    let mut t = vec![0x46, 0x30];
    for _ in 0..4 {
        t.extend_from_slice(&if le { seed.to_le_bytes() } else { seed.to_be_bytes() });
    }
    t.resize(46, 0);
    t
}

#[test]
fn decodes_through_the_nef_container() {
    use crate::testgen::{Val, nef_compressed};
    use crate::{Limits, RawFormat, identify};
    let (w, h) = (40usize, 6usize);
    // lossless 14-bit, big-endian maker note, with a BlackLevel tag
    let img = image(w, h, 16383, 11);
    let src = encode(&img, w, &LOSSLESS_14, [[2048; 2]; 2]);
    let note = vec![(0x0096, Val::Undefined(lossless_table(2048, false))), (0x003d, Val::Short(vec![600, 600, 600, 600]))];
    let b = nef_compressed(w, h, 14, src, [0, 1, 1, 2], true, note);
    assert_eq!(identify(&b), Some(RawFormat::Nef));
    let s = crate::decode(&b, &Limits::default()).unwrap();
    assert_eq!(s.data, img.iter().map(|&v| v as u16).collect::<Vec<_>>());
    assert_eq!(s.black.values, vec![600.0; 4]);
    // lossy 12-bit type 2, little-endian maker note (newer bodies); the
    // 12-bit BlackLevel is scaled from 14-bit units
    let table = lossy_table(12, true);
    let t = parse_table(&table, true, 12).unwrap();
    let Encoding::Lossy(curve) = &t.encoding else { panic!() };
    let img = image(w, h, 1023, 5);
    let src = encode(&img, w, &LOSSY_12, t.seeds);
    let note = vec![(0x0096, Val::Undefined(table.clone())), (0x003d, Val::Short(vec![600, 600, 600, 600]))];
    let s = crate::decode(&nef_compressed(w, h, 12, src.clone(), [2, 1, 1, 0], false, note), &Limits::default()).unwrap();
    assert_eq!(s.data, img.iter().map(|&v| curve[v as usize]).collect::<Vec<_>>());
    assert_eq!(s.black.values, vec![150.0; 4]);
    assert!(crate::develop_sensor(&s, &crate::DevelopOptions::default()).is_ok());
    // a split row over a stream that does not follow the fixed-rate rule, and a missing
    // table, stay unsupported (preview fallback)
    let mut split = table;
    split[562..564].copy_from_slice(&3u16.to_le_bytes());
    let e = crate::decode(&nef_compressed(w, h, 12, src.clone(), [2, 1, 1, 0], false, vec![(0x0096, Val::Undefined(split))]), &Limits::default());
    assert!(matches!(e, Err(RawError::Unsupported(_))), "{e:?}");
    let e = crate::decode(&nef_compressed(w, h, 12, src, [2, 1, 1, 0], false, vec![(0x0001, Val::Short(vec![0]))]), &Limits::default());
    assert!(matches!(&e, Err(RawError::Unsupported(m)) if m.contains("Nikon compressed NEF")), "{e:?}");
}

#[test]
fn nikon_without_black_level_tag_is_black_subtracted() {
    use crate::testgen::{Val, nef_compressed};
    let (w, h) = (32usize, 8usize);
    let img = image(w, h, 16383, 4);
    let src = encode(&img, w, &LOSSLESS_14, [[2048; 2]; 2]);
    let b = nef_compressed(w, h, 14, src, [0, 1, 1, 2], true, vec![(0x0096, Val::Undefined(lossless_table(2048, false)))]);
    let s = crate::decode(&b, &crate::Limits::default()).unwrap();
    assert_eq!(s.black.values, vec![0.0]);
    assert!(!s.warnings.iter().any(|w| w.contains("black level")), "{:?}", s.warnings);
}

#[test]
fn truncated_container_never_panics() {
    use crate::testgen::{Val, nef_compressed};
    let (w, h) = (32usize, 8usize);
    let img = image(w, h, 16383, 2);
    let src = encode(&img, w, &LOSSLESS_14, [[2048; 2]; 2]);
    let b = nef_compressed(w, h, 14, src, [0, 1, 1, 2], true, vec![(0x0096, Val::Undefined(lossless_table(2048, false)))]);
    for n in 0..b.len() {
        let _ = crate::decode(&b[..n], &crate::Limits::default());
    }
}

#[test]
fn truncated_and_corrupt_streams_error_never_panic() {
    let (w, h) = (64, 16);
    let table = lossless(14);
    let img = image(w, h, 16383, 3);
    let src = encode(&img, w, &LOSSLESS_14, table.seeds);
    for cut in [0, 1, 10, src.len() / 2, src.len() - 40] {
        assert!(decode(&src[..cut], w, h, 14, &table).is_err(), "cut {cut}");
    }
    let mut s = 12345u32;
    for i in 0..200 {
        let mut bad = src.clone();
        for _ in 0..1 + i % 8 {
            s = s.wrapping_mul(1_103_515_245).wrapping_add(12345);
            let at = (s >> 8) as usize % bad.len();
            bad[at] ^= 1 << (s % 8);
        }
        let _ = decode(&bad, w, h, 14, &table);
        let noise: Vec<u8> = (0..src.len()).map(|k| ((k as u32).wrapping_mul(2_654_435_761) >> (i % 24)) as u8).collect();
        let _ = decode(&noise, w, h, 14, &table);
        let _ = decode(&noise, w, h, 12, &lossless(12));
    }
    let lossy = parse_table(&lossy_table(12, false), false, 12).unwrap();
    assert!(decode(&[0xff; 64], 8, 8, 12, &lossy).is_err());
    assert!(decode(&src, usize::MAX, 2, 14, &table).is_err());
    assert!(decode(&src, 0, 2, 14, &table).is_err());
}

/// Encodes `values` the way a "lossy after split" camera does: Huffman (`t`) for rows above
/// `split` (exact differences), then fixed-rate words of `bits - 4` bits, then one-bits up to the
/// byte boundary. The fixed-rate words keep only the top bits of large differences, so the
/// encoder runs the decoder's own predictor on what it reconstructs (closed loop) and returns the
/// strip together with the values the decoder must produce.
fn encode_split(values: &[i32], w: usize, t: &Table, seeds: [[i32; 2]; 2], bits: u32, split: usize) -> (Vec<u8>, Vec<i32>) {
    let codes = codes(t);
    let (word, fixed) = if bits == 12 { (8, FIXED_12) } else { (10, FIXED_14) };
    let lut = fixed_words(bits);
    let mut wr = Writer::default();
    let mut vpred = seeds;
    let mut recon = Vec::with_capacity(values.len());
    for (y, row) in values.chunks(w).enumerate() {
        let mut hpred = [0i32; 2];
        for (x, &v) in row.iter().enumerate() {
            let pred = if x < 2 { vpred[y & 1][x] } else { hpred[x & 1] };
            let d = v - pred;
            let k = 32 - d.unsigned_abs().leading_zeros();
            let extra = if d < 0 { (d + (1 << k) - 1) as u32 } else { d as u32 };
            let got = if y < split {
                let (code, len) = codes[k as usize];
                assert!(len > 0, "no Huffman code for category {k}");
                wr.put(code, len);
                wr.put(extra, k);
                d
            } else {
                let &(prefix, len, _) = fixed.iter().find(|c| c.2 == k).unwrap_or_else(|| panic!("no fixed-rate code for category {k}"));
                let kept = word - len;
                let field = if k <= kept { extra << (kept - k) } else { extra >> (k - kept) };
                wr.put(prefix, len);
                wr.put(field, kept);
                lut[((prefix << kept) | field) as usize].unwrap()
            };
            let r = pred + got;
            if x < 2 {
                vpred[y & 1][x] = r;
            }
            hpred[x & 1] = r;
            recon.push(r);
        }
    }
    while wr.n > 0 {
        wr.put(1, 1);
    }
    (wr.out, recon)
}

/// Samples with large jumps (differences up to about ±1000 codes, the largest category both
/// lossy Huffman tables and the 12-bit fixed-rate words code) as well as small steps.
fn rough(w: usize, h: usize, seed: u32) -> Vec<i32> {
    let mut s = seed;
    (0..w * h)
        .map(|i| {
            s = s.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let jump = if (s >> 28) < 3 { ((s >> 8) % 900) as i32 - 450 } else { ((s >> 8) % 23) as i32 - 11 };
            700 + jump + ((i % w) as i32 * 7) % 40
        })
        .collect()
}

fn split_table(bits: u32, split: usize) -> DecodeTable {
    let mut table = lossy_table(bits, false);
    table[562..564].copy_from_slice(&(split as u16).to_be_bytes());
    let t = parse_table(&table, false, bits).unwrap();
    assert_eq!(t.split, split);
    t
}

#[test]
fn lossy_after_split_known_answers() {
    for bits in [12u32, 14] {
        let huff = if bits == 12 { &LOSSY_12 } else { &LOSSY_14 };
        // Split right after the first row, in the middle and before the last row.
        for (h, split) in [(7usize, 1usize), (7, 3), (7, 6)] {
            let w = 24usize;
            let t = split_table(bits, split);
            let Encoding::Lossy(curve) = &t.encoding else { panic!() };
            let max = curve.len() as i32 - 1;
            // Large jumps: the fixed-rate words drop low bits and decode to bin centres, which
            // the closed-loop encoder reproduces; the running value may leave 0..=max.
            let img = rough(w, h, 7 + split as u32);
            let (src, recon) = encode_split(&img, w, huff, t.seeds, bits, split);
            let want: Vec<u16> = recon.iter().map(|&r| curve[r.clamp(0, max) as usize]).collect();
            assert_eq!(decode(&src, w, h, bits, &t).unwrap(), want, "{bits}-bit split {split}");
            // Some differences really were quantised.
            assert!(recon.iter().zip(&img).any(|(r, v)| r != v), "{bits}-bit split {split}: nothing quantised");
        }
    }
}

#[test]
fn lossy_after_split_dequantises_to_symmetric_bin_centres() {
    for bits in [12u32, 14] {
        let lut = fixed_words(bits);
        let values: Vec<i32> = lut.iter().flatten().copied().collect();
        // Every word but the all-ones one codes a difference, and the codes are symmetric.
        assert_eq!(values.len(), lut.len() - 1, "{bits}-bit");
        assert!(lut.last().unwrap().is_none());
        for v in &values {
            assert!(values.contains(&-v), "{bits}-bit: {v} has no negative twin");
        }
        // Small differences are exact; the largest category (10 at 12 bit, 12 at 14 bit) reaches
        // past half its range.
        for d in -31..=31 {
            assert!(values.contains(&d), "{bits}-bit: {d}");
        }
        let top = *values.iter().max().unwrap();
        assert!(top >= if bits == 12 { 1 << 9 } else { 1 << 11 }, "{bits}-bit: {top}");
    }
}

#[test]
fn split_files_that_break_the_rule_fall_back_to_the_preview() {
    let unsupported = |r: Result<Vec<u16>>| matches!(r, Err(RawError::Unsupported(_)));
    for bits in [12u32, 14] {
        let huff = if bits == 12 { &LOSSY_12 } else { &LOSSY_14 };
        let (w, h, split) = (24usize, 7usize, 3usize);
        let t = split_table(bits, split);
        let (src, _) = encode_split(&gentle(w, h, 600), w, huff, t.seeds, bits, split);
        assert!(decode(&src, w, h, bits, &t).is_ok());
        // Trailing data, a truncated strip, a split row past the image, zero padding and the
        // all-ones word: all unsupported (the embedded preview opens), none a panic.
        let mut longer = src.clone();
        longer.extend_from_slice(&[0x12; 9]);
        assert!(unsupported(decode(&longer, w, h, bits, &t)), "{bits}-bit: trailing data");
        assert!(unsupported(decode(&src[..src.len() - 12], w, h, bits, &t)), "{bits}-bit: truncated");
        assert!(unsupported(decode(&src, w, split, bits, &t)), "{bits}-bit: split past the image");
        let mut zeros = src.clone();
        let last = zeros.len() - 1;
        zeros[last] &= 0xf0;
        assert_ne!(zeros, src, "{bits}-bit: the fixture must end in padding bits");
        assert!(unsupported(decode(&zeros, w, h, bits, &t)), "{bits}-bit: zero padding");
        let mut all_ones = src.clone();
        let fixed_start = all_ones.len() - (w * (h - split) * (bits as usize - 4)).div_ceil(8);
        all_ones[fixed_start + 2..fixed_start + 6].fill(0xff);
        assert!(unsupported(decode(&all_ones, w, h, bits, &t)), "{bits}-bit: all-ones word");
    }
    // A split row on a lossless table is malformed (no parsed table makes one).
    let lossless_split = DecodeTable { split: 3, ..lossless(12) };
    assert!(unsupported(decode(&[0; 64], 24, 7, 12, &lossless_split)));
    // A curve long enough to cover byte 562 has no split field: refused as before.
    let mut long = vec![0x44, 0x20];
    long.extend(std::iter::repeat_n([0u8, 9], 4).flatten());
    long.extend_from_slice(&513u16.to_be_bytes());
    long.extend((0..513u32).flat_map(|i| ((i * 7) as u16).to_be_bytes()));
    assert!(matches!(parse_table(&long, false, 12), Err(RawError::Unsupported(_))));
}

#[test]
fn split_nef_decodes_through_the_container() {
    use crate::testgen::{Val, nef_compressed};
    use crate::{Limits, decode as decode_raw};
    let (w, h, split) = (40usize, 6usize, 2usize);
    let mut table = lossy_table(12, true);
    table[562..564].copy_from_slice(&(split as u16).to_le_bytes());
    let t = parse_table(&table, true, 12).unwrap();
    let Encoding::Lossy(curve) = &t.encoding else { panic!() };
    let (src, recon) = encode_split(&rough(w, h, 3), w, &LOSSY_12, t.seeds, 12, split);
    let note = vec![(0x0096, Val::Undefined(table))];
    let s = decode_raw(&nef_compressed(w, h, 12, src, [2, 1, 1, 0], false, note), &Limits::default()).unwrap();
    let max = curve.len() as i32 - 1;
    assert_eq!(s.data, recon.iter().map(|&r| curve[r.clamp(0, max) as usize]).collect::<Vec<_>>());
    assert!(crate::develop_sensor(&s, &crate::DevelopOptions::default()).is_ok());
}

/// Small steps around a level (every difference below 32), so fixed-rate words keep them whole.
fn gentle(w: usize, h: usize, base: i32) -> Vec<i32> {
    (0..w * h).map(|i| base + ((i % w) as i32 * 3 + (i / w) as i32 * 5) % 13 + if i % 7 == 0 { 7 } else { 0 }).collect()
}

#[test]
fn fixed_rate_running_value_is_not_clamped() {
    // A jump from 900 down to 0 is category 10: its 12-bit word keeps 5 of 10 additional bits
    // and decodes to -912, so the running value sits at -12. The next same-colour pixel codes +12
    // exactly and must land on 0 again; clamping the running value would give 12.
    let (w, h, split) = (12usize, 4usize, 1usize);
    let t = split_table(12, split);
    let Encoding::Lossy(curve) = &t.encoding else { panic!() };
    let img: Vec<i32> = (0..w * h).map(|i| if i % w < 2 { 900 } else { 0 }).collect();
    let (src, recon) = encode_split(&img, w, &LOSSY_12, t.seeds, 12, split);
    assert!(recon.iter().any(|&r| r < 0), "{recon:?}");
    let max = curve.len() as i32 - 1;
    let want: Vec<u16> = recon.iter().map(|&r| curve[r.clamp(0, max) as usize]).collect();
    assert_eq!(decode(&src, w, h, 12, &t).unwrap(), want);
}

#[test]
fn fixed_rate_words_known_answers() {
    // Worked by hand from the rule: prefix -> category n; the word keeps the top `kept` of n
    // additional bits; T.81: a leading 0 bit is a negative difference; the difference is the
    // centre of the kept bin (low dropped bits = half the bin).
    let (w12, w14) = (fixed_words(12), fixed_words(14));
    for (word, want) in [
        (0b1111_1110, Some(0)),    // category 0
        (0b1110_0000, Some(-15)),  // category 4, all 4 bits kept: 0000 = -15
        (0b1110_1111, Some(15)),   // 1111 = +15
        (0b101_00000, Some(-63)),  // category 6, 5 of 6 bits kept: bin -63..-62, centre -63
        (0b101_10000, Some(33)),   // bin 32..33, centre 33
        (0b010_00011, Some(-912)), // category 10, 5 of 10 bits: 00011xxxxx = 96..127 -> -927..-896
        (0b010_11111, Some(1008)), // 11111xxxxx = 992..1023, centre 1008
        (0b1111_1111, None),       // the all-ones word codes nothing
    ] {
        assert_eq!(w12[word], want, "12-bit word {word:#010b}");
    }
    for (word, want) in [
        (0b11_1111_1110, Some(0)),    // category 0
        (0b010_0000000, Some(-4080)), // category 12, 7 of 12 bits: -4095..-4064, centre -4080
        (0b010_1111111, Some(4080)),  // 4064..4095, centre 4080
        (0b011_1000000, Some(1032)),  // category 11, 7 of 11 bits: 1024..1039, centre 1032
        (0b00_00000000, Some(-255)),  // category 8, all 8 bits kept: 00000000 = -255
        (0b11_1111_1111, None),       // the all-ones word
    ] {
        assert_eq!(w14[word], want, "14-bit word {word:#012b}");
    }
}

#[test]
fn out_of_range_fixed_rate_values_are_not_counted_as_clipped() {
    // Many jumps down to 0 leave the running value just below 0 (see the test above), far more
    // than the 16 + n/1000 out-of-range values tolerated in Huffman rows. They are legitimate in
    // the fixed-rate section and must not fail the decode.
    let (w, h, split) = (40usize, 5usize, 1usize);
    let t = split_table(12, split);
    let Encoding::Lossy(curve) = &t.encoding else { panic!() };
    let img: Vec<i32> = (0..w * h).map(|i| if (i % w) % 4 < 2 { 900 } else { 0 }).collect();
    let (src, recon) = encode_split(&img, w, &LOSSY_12, t.seeds, 12, split);
    assert!(recon.iter().filter(|&&r| r < 0).count() > 16 + w * h / 1000, "{recon:?}");
    let max = curve.len() as i32 - 1;
    assert_eq!(decode(&src, w, h, 12, &t).unwrap(), recon.iter().map(|&r| curve[r.clamp(0, max) as usize]).collect::<Vec<_>>());
}

#[test]
fn split_files_failing_after_the_table_still_fall_back_to_the_preview() {
    use crate::testgen::{Val, nef_compressed};
    use crate::{Limits, decode as decode_raw};
    let (w, h, split) = (40usize, 6usize, 2usize);
    let mut table = lossy_table(12, true);
    table[562..564].copy_from_slice(&(split as u16).to_le_bytes());
    let t = parse_table(&table, true, 12).unwrap();
    let (src, _) = encode_split(&gentle(w, h, 600), w, &LOSSY_12, t.seeds, 12, split);
    let note = || vec![(0x0096, Val::Undefined(table.clone()))];
    // The refusal must come from the split path, not from some other part of the file.
    let unsupported = |r: Result<crate::Sensor>| matches!(&r, Err(RawError::Unsupported(m)) if m.contains("lossy after split"));
    // Over the size limits, and with the strip cut off by the end of the file.
    let small = Limits { max_pixels: 100, ..Limits::default() };
    assert!(unsupported(decode_raw(&nef_compressed(w, h, 12, src.clone(), [2, 1, 1, 0], false, note()), &small)));
    let file = nef_compressed(w, h, 12, src, [2, 1, 1, 0], false, note());
    let cut = file.len() - 40;
    assert!(unsupported(decode_raw(&file[..cut], &Limits::default())));
}

#[test]
fn split_strips_with_up_to_four_extra_rows_are_accepted() {
    // A strip may hold whole fixed-rate rows past the image (one body stores 4022 rows for
    // 4020): up to 4 such rows decode, the image rows unchanged; a fifth is refused.
    for bits in [12u32, 14] {
        let huff = if bits == 12 { &LOSSY_12 } else { &LOSSY_14 };
        let (w, h, split) = (24usize, 7usize, 3usize);
        let t = split_table(bits, split);
        let Encoding::Lossy(curve) = &t.encoding else { panic!() };
        let max = curve.len() as i32 - 1;
        for extra in 1..=5 {
            let (src, recon) = encode_split(&gentle(w, h + extra, 600), w, huff, t.seeds, bits, split);
            let got = decode(&src, w, h, bits, &t);
            if extra <= 4 {
                let want: Vec<u16> = recon[..w * h].iter().map(|&r| curve[r.clamp(0, max) as usize]).collect();
                assert_eq!(got.unwrap(), want, "{bits}-bit, {extra} extra rows");
            } else {
                assert!(matches!(got, Err(RawError::Unsupported(_))), "{bits}-bit, {extra} extra rows: {got:?}");
            }
        }
    }
}
