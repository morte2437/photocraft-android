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
    DecodeTable { encoding: Encoding::Lossless, seeds: [[s, s], [s, s]] }
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
    assert!(unsupported(parse_table(&split, false, 12)));
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
    // "lossy after split" and a missing table stay unsupported (preview fallback)
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
