//! Canon sRAW / mRAW (CR2 with subsampled YCbCr).
//!
//! The camera demosaics the sensor data and stores it as Y, Cb, Cr in one lossless JPEG frame
//! whose Y component is sampled 2×2 (sRAW1, which Canon calls mRAW on the later bodies) or 2×1
//! (sRAW2), sliced like the Bayer CR2 data (tag 0xC640, widths in samples). The decoder
//! ([`crate::ljpeg::decode_subsampled`]) gives the sample sequence; [`planes`] lays it out,
//! [`camera_rgb`] interpolates the chroma and converts to camera RGB, and the result is
//! developed like a three-sample LinearRaw: no CFA, the as-shot white balance, the look fitted
//! to the embedded JPEG (`photocraft-io`), tone and exposure as for every other raw.
//!
//! Observed on EOS 6D Mark II mRAW files (ColorData version 15, 4680 × 3132 data):
//! * Y is already black-subtracted (the darkest samples are 0) and on the sensor's scale; Cb and
//!   Cr are colour differences offset by `2^(P−1)`.
//! * The stored RGB is camera RGB divided by per-shot gains, an RGGB entry of ColorData (word
//!   0x80 in version 15; its green is 1170, not the 1024 of the white-balance presets) that
//!   roughly neutralises the scene. Multiplying them back gives camera RGB that matches an
//!   independent converter's linear camera-RGB output of the same files to a diagonal of
//!   (1, 1, 1) ± 1.5 % and 0.6 % RMS; the as-shot white balance then applies as for Bayer data.
//! * The inverse with the BT.601 luma weights (ITU-R BT.601: Y = 0.299 R + 0.587 G + 0.114 B)
//!   and unscaled differences (Cb = B − Y, Cr = R − Y) leaves no cross-channel terms above 3 %
//!   in that comparison.
//! * Chroma edges follow the luma edges slightly better half an MCU in than at its first
//!   sample, so the chroma is treated as centred on its MCU and interpolated bilinearly.
//!
//! Approximate: the chroma siting and the bilinear interpolation (colour edges are softer than
//! in a Bayer development; luma keeps full resolution). The gains are read only for ColorData
//! versions where their offset was observed; for other bodies they are left at 1, which is a
//! diagonal colour error that the matrix fitted to the camera's JPEG absorbs (a warning says
//! so). Older bodies (sRAW before about 2009) are documented elsewhere to use other offsets
//! and chroma scalings; no such sample was available, so they decode with the same rule.
//! Shadow noise below black is clipped at 0 by the conversion.

use crate::error::{RawError, Result};
use crate::ljpeg::Frame;
use crate::par;

/// The de-sliced YCbCr planes: Y at full resolution, Cb and Cr one sample per MCU, stored
/// values (chroma still carries its offset).
pub(crate) struct Planes {
    pub width: usize,
    pub height: usize,
    pub h: usize,
    pub v: usize,
    pub y: Vec<u16>,
    pub cb: Vec<u16>,
    pub cr: Vec<u16>,
}

/// Lays the decoded sample sequence out as planes. `slices` are the slice widths in samples
/// (CR2 tag 0xC640; one slice of the whole MCU row when absent): slice 0 is filled MCU row by
/// MCU row, then slice 1, and so on, as for the Bayer CR2 data.
pub(crate) fn planes(f: &Frame, samples: &[u16], slices: &[usize]) -> Result<Planes> {
    let (h, v) = (f.h, f.v);
    let per_mcu = h * v + 2;
    let total: usize = slices.iter().sum();
    if total == 0 || !total.is_multiple_of(per_mcu) || slices.iter().any(|w| !w.is_multiple_of(per_mcu)) || !samples.len().is_multiple_of(total) {
        return Err(RawError::malformed("CR2 sRAW slices do not match the JPEG frame"));
    }
    let mcus = total / per_mcu;
    let rows = samples.len() / total;
    let (width, height) = (mcus * h, rows * v);
    let mut p = Planes { width, height, h, v, y: vec![0; width * height], cb: vec![0; mcus * rows], cr: vec![0; mcus * rows] };
    let mut runs = samples.chunks_exact(per_mcu);
    let mut x0 = 0usize;
    for &sw in slices {
        let n = sw / per_mcu;
        for r in 0..rows {
            for mx in x0..x0 + n {
                let mcu = runs.next().ok_or_else(|| RawError::malformed("CR2 sRAW slice data is short"))?;
                let (ys, c) = mcu.split_at(h * v);
                for (k, &s) in ys.iter().enumerate() {
                    let (x, y) = (mx * h + k % h, r * v + k / h);
                    if let Some(d) = p.y.get_mut(y * width + x) {
                        *d = s;
                    }
                }
                if let ([cb, cr], Some(dcb), Some(dcr)) = (c, p.cb.get_mut(r * mcus + mx), p.cr.get_mut(r * mcus + mx)) {
                    (*dcb, *dcr) = (*cb, *cr);
                }
            }
        }
        x0 += n;
    }
    Ok(p)
}

/// White level of unclipped frames: 14-bit full scale less Canon's usual black of about 2048.
pub(crate) const FULL_SCALE: f32 = 14335.0;

/// BT.601 luma weights of R and B (G's is the rest).
const KR: f32 = 0.299;
const KB: f32 = 0.114;

/// Interpolates the chroma to every pixel and converts to camera RGB, interleaved:
/// `R = Y + Cr`, `B = Y + Cb`, `G = (Y − KR·R − KB·B) / KG`, then each channel multiplied by the
/// camera's sRAW `gains`. `offset` is the chroma's zero (`2^(P−1)`).
pub(crate) fn camera_rgb(p: &Planes, offset: f32, gains: [f64; 3]) -> Vec<u16> {
    let (w, h) = (p.width, p.height);
    let cw = w / p.h.max(1);
    let ch = p.cb.len().checked_div(cw).unwrap_or(0);
    let mut out = vec![0u16; w * h * 3];
    if w == 0 || cw == 0 || ch == 0 {
        return out;
    }
    let gains = gains.map(|m| if m.is_finite() && m > 0.0 { m as f32 } else { 1.0 });
    let kg = 1.0 - KR - KB;
    // Chroma sample positions: centred on the MCU (see the module docs). Per output column (row):
    // the two chroma columns (rows) around it and the weight of the second.
    let taps = |n: usize, f: usize, len: usize| -> Vec<(usize, usize, f32)> {
        (0..n)
            .map(|i| {
                let c = ((i as f32 + 0.5) / f as f32 - 0.5).clamp(0.0, (len - 1) as f32);
                let a = (c as usize).min(len - 1);
                let b = (a + 1).min(len - 1);
                (a, b, c - a as f32)
            })
            .collect()
    };
    let xs = taps(w, p.h.max(1), cw);
    let ys = taps(h, p.v.max(1), ch);
    let band = par::band_rows(w);
    par::chunks_mut(&mut out, band * w * 3, |b, chunk| {
        for (r, row) in chunk.chunks_exact_mut(w * 3).enumerate() {
            let y = b * band + r;
            let (Some(luma), Some(&(ya, yb, fy))) = (p.y.get(y * w..(y + 1) * w), ys.get(y)) else { continue };
            let chroma = |plane: &[u16], xa: usize, xb: usize, fx: f32| {
                let at = |cy: usize, cx: usize| plane.get(cy * cw + cx).map_or(offset, |&v| f32::from(v));
                let top = at(ya, xa) * (1.0 - fx) + at(ya, xb) * fx;
                let bottom = at(yb, xa) * (1.0 - fx) + at(yb, xb) * fx;
                top * (1.0 - fy) + bottom * fy - offset
            };
            for ((o, &l), &(xa, xb, fx)) in row.as_chunks_mut::<3>().0.iter_mut().zip(luma).zip(&xs) {
                let l = f32::from(l);
                let (cb, cr) = (chroma(&p.cb, xa, xb, fx), chroma(&p.cr, xa, xb, fx));
                let (red, blue) = (l + cr, l + cb);
                let green = (l - KR * red - KB * blue) / kg;
                for (k, v) in [red, green, blue].into_iter().enumerate() {
                    o[k] = (v * gains[k]).round().clamp(0.0, 65535.0) as u16;
                }
            }
        }
    });
    out
}

/// White level of channel `k` of interleaved RGB: the largest value when a meaningful share of
/// samples piles up there (clipped highlights), else `full`.
pub(crate) fn clip_level(rgb: &[u16], k: usize, full: f32) -> f32 {
    let step = 3 * (rgb.len() / 3 / 2_000_000).max(1);
    let samples = || rgb.iter().skip(k).step_by(step);
    let max = samples().copied().max().unwrap_or(0);
    if max == 0 {
        return full;
    }
    let near = samples().filter(|&&v| u32::from(v) + u32::from(max / 256) >= u32::from(max)).count();
    if near * 2000 >= samples().count() { f32::from(max) * 0.995 } else { full.max(f32::from(max)) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testgen::Cr2SrawSpec;
    use crate::{DevelopOptions, Limits, decode, develop_sensor};

    const OFFSET: f32 = 16384.0;

    /// A linear ramp of camera RGB (bilinear chroma interpolation reproduces it exactly away
    /// from the borders).
    fn camera(x: usize, y: usize) -> [f32; 3] {
        let (x, y) = (x as f32, y as f32);
        [1500.0 + 40.0 * x + 10.0 * y, 2500.0 + 15.0 * x + 30.0 * y, 1200.0 + 5.0 * x + 45.0 * y]
    }

    /// The camera's stored YCbCr for `camera` divided by `gains`: Y per pixel, Cb / Cr at the
    /// centre of each MCU (the siting the decoder assumes).
    fn spec(h: usize, v: usize, gains: [f32; 3], slices: Vec<usize>, restart_rows: Option<usize>) -> Cr2SrawSpec {
        let (w, ht) = (48, 24);
        let stored = |x: f32, y: f32| {
            let c = camera(0, 0);
            let (dx, dy) = (camera(1, 0), camera(0, 1));
            [0, 1, 2].map(|k| (c[k] + (dx[k] - c[k]) * x + (dy[k] - c[k]) * y) / gains[k])
        };
        let ycc = |p: [f32; 3]| {
            let y = KR * p[0] + (1.0 - KR - KB) * p[1] + KB * p[2];
            (y, p[2] - y + OFFSET, p[0] - y + OFFSET)
        };
        let y = (0..w * ht).map(|i| ycc(stored((i % w) as f32, (i / w) as f32)).0.round() as u16).collect();
        let (mw, mh) = (w / h, ht / v);
        let centre = |m: usize, f: usize| (m * f) as f32 + (f as f32 - 1.0) / 2.0;
        let chroma: Vec<(f32, f32, f32)> = (0..mw * mh).map(|i| ycc(stored(centre(i % mw, h), centre(i / mw, v)))).collect();
        Cr2SrawSpec {
            width: w,
            height: ht,
            h,
            v,
            precision: 15,
            y,
            cb: chroma.iter().map(|c| c.1.round() as u16).collect(),
            cr: chroma.iter().map(|c| c.2.round() as u16).collect(),
            slices,
            restart_rows,
            wb_rggb: Some([2048, 1024, 1024, 1536]),
            gains: Some([(1170.0 * gains[0]) as u16, 1170, 1170, (1170.0 * gains[2]) as u16]),
            borders: Some([0, 2, 47, 23]),
        }
    }

    #[test]
    fn sraw_decodes_to_camera_rgb() {
        let gains = [0.75, 1.0, 0.4];
        // sRAW1 (2×2: 6 samples an MCU) and sRAW2 (2×1: 4 samples an MCU), with and without
        // slices and restart intervals.
        for (h, v, slices, restart) in
            [(2, 2, vec![], None), (2, 2, vec![48, 48, 48], Some(3)), (2, 2, vec![60, 60, 24], None), (2, 1, vec![], Some(1)), (2, 1, vec![32, 32, 32], None)]
        {
            let s = spec(h, v, gains, slices.clone(), restart);
            let sensor = decode(&s.build(), &Limits::default()).expect("decode");
            assert_eq!((sensor.width, sensor.height, sensor.samples), (48, 24, 3), "{h}x{v} {slices:?}");
            assert!(sensor.cfa.is_none());
            assert_eq!(sensor.crop, crate::Rect::new(0, 2, 48, 22));
            assert_eq!(sensor.camera_wb, Some([2.0, 1.0, 1.5]));
            assert!(sensor.warnings.is_empty(), "{:?}", sensor.warnings);
            for y in 2..22 {
                for x in 2..46 {
                    let at = (y * 48 + x) * 3;
                    for (k, (&got, want)) in sensor.data[at..at + 3].iter().zip(camera(x, y)).enumerate() {
                        let got = f32::from(got);
                        // Rounding of Y, Cb, Cr and the stored gains (whole 1/1170 steps).
                        assert!((got - want).abs() <= 4.0 + want * 0.002, "{h}x{v} {slices:?} ({x}, {y}) ch {k}: {got} vs {want}");
                    }
                }
            }
        }
    }

    #[test]
    fn unknown_gains_keep_the_stored_colour_and_say_so() {
        let mut s = spec(2, 2, [1.0; 3], vec![], None);
        s.gains = None;
        let sensor = decode(&s.build(), &Limits::default()).expect("decode");
        assert!(sensor.warnings.iter().any(|w| w.contains("sRAW colour gains")), "{:?}", sensor.warnings);
        let want = camera(10, 10);
        let got = &sensor.data[(10 * 48 + 10) * 3..(10 * 48 + 10) * 3 + 3];
        assert!(got.iter().zip(want).all(|(&g, w)| (f32::from(g) - w).abs() <= 4.0), "{got:?} vs {want:?}");
    }

    #[test]
    fn sraw_develops_as_linear_rgb() {
        let s = spec(2, 1, [0.75, 1.0, 0.4], vec![], None);
        let sensor = decode(&s.build(), &Limits::default()).expect("decode");
        let d = develop_sensor(&sensor, &DevelopOptions::default()).expect("develop");
        assert_eq!((d.width, d.height), (48, 22));
        assert_eq!(d.info.cfa, None);
        assert_eq!(d.info.wb_multipliers, [2.0, 1.0, 1.5]);
    }

    #[test]
    fn corrupt_sraw_errors_without_panicking() {
        let b = spec(2, 2, [0.75, 1.0, 0.4], vec![48, 48, 48], Some(2)).build();
        for n in (0..b.len()).step_by(7) {
            let _ = decode(&b[..n], &Limits::default());
        }
        let mut s = 0x9E37_79B9u64;
        for _ in 0..1500 {
            let mut c = b.clone();
            for _ in 0..3 {
                s = s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                let at = (s >> 33) as usize % c.len();
                c[at] = (s >> 20) as u8;
            }
            if let Ok(sensor) = decode(&c, &Limits::default()) {
                let _ = develop_sensor(&sensor, &DevelopOptions::default());
            }
        }
        // Slices that do not split whole MCUs are refused.
        let mut bad = spec(2, 2, [1.0; 3], vec![48, 48, 48], None).build();
        let tag = bad.windows(4).position(|w| w == [0x40, 0xC6, 3, 0]).expect("slice tag");
        let at = u32::from_le_bytes([bad[tag + 8], bad[tag + 9], bad[tag + 10], bad[tag + 11]]) as usize;
        bad[at + 2..at + 4].copy_from_slice(&47u16.to_le_bytes());
        assert!(matches!(decode(&bad, &Limits::default()), Err(RawError::Malformed(_))));
        // The planes refuse a sample count that is not whole slice rows.
        let f = Frame { precision: 15, width: 4, height: 2, components: 3, h: 2, v: 2 };
        assert!(planes(&f, &[0; 11], &[12]).is_err());
        assert!(planes(&f, &[0; 12], &[0]).is_err());
    }

    #[test]
    fn white_level_follows_clipping() {
        // A lone bright sample is not clipping.
        let mut rgb: Vec<u16> = (0..3 * 10_000).map(|i| (i % 5000) as u16).collect();
        rgb[0] = 9000;
        assert_eq!(clip_level(&rgb, 0, 14335.0), 14335.0);
        for p in rgb.as_chunks_mut::<3>().0.iter_mut().take(100) {
            p[1] = 12000;
        }
        assert!((clip_level(&rgb, 1, 14335.0) - 12000.0 * 0.995).abs() < 1e-3);
        assert_eq!(clip_level(&[], 2, 14335.0), 14335.0);
    }

    /// Real files: `PHOTOCRAFT_SRAW_CR2` lists CR2 paths (separated by `:`), e.g. EOS 6D Mark II
    /// mRAW. Checks the size, the gains and that the development runs.
    #[test]
    #[ignore = "needs real sRAW CR2 files in PHOTOCRAFT_SRAW_CR2"]
    fn real_sraw_files() {
        let paths = std::env::var("PHOTOCRAFT_SRAW_CR2").expect("PHOTOCRAFT_SRAW_CR2");
        for path in paths.split(':').filter(|p| !p.is_empty()) {
            let bytes = std::fs::read(path).expect("read");
            let s = decode(&bytes, &Limits::default()).expect("decode");
            assert_eq!(s.samples, 3, "{path}");
            assert!(s.cfa.is_none() && s.camera_wb.is_some(), "{path}");
            assert!(s.warnings.is_empty(), "{path}: {:?}", s.warnings);
            let d = develop_sensor(&s, &DevelopOptions::default()).expect("develop");
            eprintln!("{path}: {}x{} from {}x{}, white {:?}", d.width, d.height, s.width, s.height, s.white);
        }
    }
}
