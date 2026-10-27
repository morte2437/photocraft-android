//! Synthetic DNG files: every storage layout decodes to the exact sensor
//! samples, and development reproduces known colours.

use photocraft_raw::testgen::{self, DngSpec, DngStorage, Val, baseline_jpeg, mosaic, scene};
use photocraft_raw::*;

fn sensor(bytes: &[u8]) -> Sensor {
    decode(bytes, &Limits::default()).unwrap()
}

/// Deterministic sensor-like samples below `2^bits`.
fn noise(n: usize, bits: u16) -> Vec<u16> {
    let mut s = 0x9E37_79B9_7F4A_7C15u64;
    let max = (1u32 << bits) - 1;
    (0..n)
        .map(|i| {
            s = s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            ((i as u32 * 13 + (s >> 40) as u32 % 200) % (max + 1)) as u16
        })
        .collect()
}

#[test]
fn identifies_dng() {
    let b = DngSpec::cfa(4, 4, vec![0; 16]).build();
    assert_eq!(identify(&b), Some(RawFormat::Dng));
    assert!(is_raw(&b));
    assert!(!is_raw(b"II*\0\x08\0\0\0\0\0"));
    assert!(!is_raw(b"\x89PNG\r\n\x1a\n"));
}

#[test]
fn storage_layouts_decode_exactly() {
    let (w, h) = (37, 23); // odd sizes exercise partial strips and edge tiles
    let cases: Vec<(&str, u16, DngStorage, bool)> = vec![
        ("16-bit strips", 16, DngStorage::Strips { rows: 5 }, false),
        ("16-bit big-endian", 16, DngStorage::Strips { rows: 23 }, true),
        ("12-bit packed", 12, DngStorage::Strips { rows: 4 }, false),
        ("10-bit packed big-endian", 10, DngStorage::Strips { rows: 7 }, true),
        ("8-bit", 8, DngStorage::Strips { rows: 9 }, false),
        ("LJ92 tiles", 14, DngStorage::Lj92Tiles { width: 16, height: 8 }, false),
        ("LJ92 tiles big-endian", 16, DngStorage::Lj92Tiles { width: 32, height: 32 }, true),
        ("LJ92 strips", 12, DngStorage::Lj92Strips { rows: 6 }, false),
    ];
    for (name, bits, storage, be) in cases {
        let data = noise(w * h, bits);
        let mut spec = DngSpec::cfa(w, h, data.clone());
        spec.bits = bits;
        spec.storage = storage;
        spec.big_endian = be;
        spec.white = (1 << bits) - 1;
        let s = sensor(&spec.build());
        assert_eq!((s.width, s.height, s.samples), (w, h, 1), "{name}");
        assert_eq!(s.data, data, "{name}");
        assert_eq!(s.format, RawFormat::Dng);
    }
}

#[test]
fn linear_raw_decodes_and_develops() {
    let (w, h) = (20, 10);
    let data = noise(w * h * 3, 16);
    let mut spec = DngSpec::cfa(w, h, data.clone());
    spec.samples = 3;
    spec.storage = DngStorage::Lj92Tiles { width: 16, height: 16 };
    let s = sensor(&spec.build());
    assert_eq!((s.samples, s.cfa.is_none()), (3, true));
    assert_eq!(s.data, data);
    let d = develop_sensor(&s, &DevelopOptions::default()).unwrap();
    assert_eq!((d.width, d.height, d.rgb.len()), (20, 10, 600));
    assert!(d.info.cfa.is_none());
}

/// A horizontal ramp from `lo` to `hi` (inclusive), the same in every row.
fn ramp(w: usize, h: usize, lo: u16, hi: u16) -> Vec<u16> {
    (0..w * h).map(|i| lo + ((u32::from(hi - lo) * (i % w) as u32) / (w as u32 - 1)) as u16).collect()
}

/// Developed RGB of a monochrome DNG: every pixel has R = G = B, and the first row's grey level
/// rises with the ramp. Returns the first row's grey levels.
fn assert_neutral_ramp(d: &Developed, name: &str) -> Vec<u16> {
    let px = d.rgb.as_chunks::<3>().0;
    assert!(px.iter().all(|p| p[0] == p[1] && p[1] == p[2]), "{name}: channels differ");
    let row: Vec<u16> = px[..d.width as usize].iter().map(|p| p[0]).collect();
    assert!(row.windows(2).all(|w| w[1] >= w[0]), "{name}: not monotonic: {row:?}");
    assert!(row[row.len() - 1] > row[0] + 10_000, "{name}: flat: {row:?}");
    row
}

/// #3497: a one-sample LinearRaw DNG (VueScan film scans, Leica Monochrom) was rejected as
/// "LinearRaw data with 1 samples per pixel". It opens as neutral grey from every storage.
#[test]
fn monochrome_linear_raw_develops_to_neutral_grey() {
    let (w, h) = (37, 23);
    let cases: Vec<(&str, u16, DngStorage)> = vec![
        ("16-bit strips", 16, DngStorage::Strips { rows: 5 }),
        ("12-bit packed", 12, DngStorage::Strips { rows: 4 }),
        ("8-bit", 8, DngStorage::Strips { rows: 23 }),
        ("LJ92 strips", 14, DngStorage::Lj92Strips { rows: 6 }),
        ("LJ92 tiles", 16, DngStorage::Lj92Tiles { width: 16, height: 8 }),
        ("deflate predictor 2", 16, DngStorage::DeflateStrips { rows: 7, predictor: 2 }),
        ("lossy JPEG tiles", 8, DngStorage::LossyJpegTiles { width: 16, height: 8 }),
        ("lossy JPEG strips", 8, DngStorage::LossyJpegStrips { rows: 5 }),
    ];
    for (name, bits, storage) in cases {
        let white = ((1u32 << bits) - 1) as u16;
        let data = ramp(w, h, 0, white);
        let mut spec = DngSpec::monochrome(w, h, data.clone());
        spec.bits = bits;
        spec.white = u32::from(white);
        spec.storage = storage;
        let lossy = matches!(storage, DngStorage::LossyJpegTiles { .. } | DngStorage::LossyJpegStrips { .. });
        let s = sensor(&spec.build());
        assert_eq!((s.samples, s.cfa.is_none(), s.is_monochrome()), (1, true, true), "{name}");
        if !lossy {
            assert_eq!(s.data, data, "{name}");
        }
        let d = develop_sensor(&s, &DevelopOptions::default()).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!((d.width, d.height, d.rgb.len()), (37, 23, 37 * 23 * 3), "{name}");
        assert!(d.info.cfa.is_none() && d.info.wb_multipliers == [1.0; 3], "{name}: {:?}", d.info);
        assert!(d.warnings.iter().all(|w| !w.contains("colour") && !w.contains("white balance")), "{name}: {:?}", d.warnings);
        let row = assert_neutral_ramp(&d, name);
        if !lossy {
            assert_eq!((row[0], row[row.len() - 1]), (0, 65535), "{name}");
        }
    }
}

/// Black and white levels of a monochrome DNG scale its one sample; the white-balance
/// settings do not tint it.
#[test]
fn monochrome_linear_raw_levels_and_white_balance() {
    let (w, h) = (32, 4);
    let mut spec = DngSpec::monochrome(w, h, ramp(w, h, 1000, 60000));
    spec.black = vec![1000];
    spec.white = 60000;
    let s = sensor(&spec.build());
    for wb in [WhiteBalance::AsShot, WhiteBalance::Auto, WhiteBalance::Multipliers([2.0, 1.0, 1.5])] {
        let d = develop_sensor(&s, &DevelopOptions { white_balance: wb, ..Default::default() }).unwrap();
        let row = assert_neutral_ramp(&d, &format!("{wb:?}"));
        assert_eq!((row[0], row[w - 1]), (0, 65535), "{wb:?}");
    }
    // Develop exposure still applies.
    let dark = develop_sensor(&s, &DevelopOptions { exposure: -1.0, ..Default::default() }).unwrap();
    let row = assert_neutral_ramp(&dark, "exposure -1");
    assert!(row[w - 1] < 60000, "{row:?}");
}

/// Colour metadata a monochrome DNG may carry, well-formed or not (a 1×3 ColorMatrix, a
/// one-value ColorMatrix or AsShotNeutral, BlackLevel counts that fit no repeat), is ignored
/// or falls back; it never errors or indexes out of bounds.
#[test]
fn monochrome_linear_raw_odd_metadata_develops() {
    let (w, h) = (16, 6);
    let base = DngSpec::monochrome(w, h, ramp(w, h, 100, 4000));
    let rat = |v: &[f64]| Val::SRational(v.iter().map(|x| ((x * 10000.0) as i32, 10000)).collect());
    let variants: Vec<(&str, DngSpec)> = vec![
        ("1x3 ColorMatrix", DngSpec { ifd0_extra: vec![(50721, rat(&[0.3, 0.6, 0.1])), (50778, Val::Short(vec![21]))], ..base.clone() }),
        ("one-value ColorMatrix", DngSpec { ifd0_extra: vec![(50721, rat(&[1.0]))], ..base.clone() }),
        ("one-value AsShotNeutral", DngSpec { ifd0_extra: vec![(50728, Val::Rational(vec![(1, 1)]))], ..base.clone() }),
        ("two BlackLevels", DngSpec { black: vec![100, 120], ..base.clone() }),
        ("three BlackLevels", DngSpec { black: vec![100, 110, 120], ..base.clone() }),
        ("2x2 BlackLevel repeat", DngSpec { black: vec![100, 101, 102, 103], black_repeat: (2, 2), ..base.clone() }),
        ("2x2 repeat, one value", DngSpec { black: vec![100], black_repeat: (2, 2), ..base.clone() }),
        ("3x3 ColorMatrix", DngSpec { color_matrix1: Some((21, [3.2, -1.5, -0.5, -1.0, 1.9, 0.04, 0.06, -0.2, 1.06])), ..base.clone() }),
        ("3-value AsShotNeutral", DngSpec { as_shot_neutral: Some([0.5, 1.0, 0.7]), ..base.clone() }),
    ];
    for (name, spec) in variants {
        let d = develop(&spec.build(), &DevelopOptions::default()).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_neutral_ramp(&d, name);
    }
}

/// Broken monochrome DNGs fail with an error, not a panic.
#[test]
fn broken_monochrome_linear_raw_errors() {
    let (w, h) = (16, 6);
    let spec = DngSpec::monochrome(w, h, ramp(w, h, 100, 4000));
    // Every truncation decodes to an error or develops; none panics, and a cut into the
    // strip data is an error.
    let b = spec.build();
    for n in 0..b.len() {
        if let Ok(s) = decode(&b[..n], &Limits::default()) {
            let _ = develop_sensor(&s, &DevelopOptions::default());
        }
    }
    let first: Vec<u8> = spec.data[..4].iter().flat_map(|v| v.to_le_bytes()).collect();
    let strip = b.windows(first.len()).position(|win| win == first.as_slice()).expect("strip data");
    assert!(decode(&b[..strip + w], &Limits::default()).is_err(), "truncated strip");
    // Two samples per pixel is no LinearRaw layout.
    let two = DngSpec { samples: 2, data: ramp(2 * w, h, 100, 4000), ..spec.clone() };
    let err = decode(&two.build(), &Limits::default()).unwrap_err().to_string();
    assert!(err.contains("2 samples per pixel"), "{err}");
    // A monochrome sensor holding less data than its size.
    let mut s = sensor(&spec.build());
    s.data.truncate(10);
    assert!(develop_sensor(&s, &DevelopOptions::default()).is_err());
}

/// `spec`'s file with its Orientation entry (a SHORT `from`) rewritten as a LONG `to`.
fn with_long_orientation(spec: &DngSpec, from: u16, to: u32) -> Vec<u8> {
    let mut b = spec.build();
    let short = [&[0x12, 0x01, 3, 0, 1, 0, 0, 0][..], &from.to_le_bytes(), &[0, 0]].concat();
    let at: Vec<usize> = b.windows(short.len()).enumerate().filter(|(_, w)| *w == short.as_slice()).map(|(i, _)| i).collect();
    assert_eq!(at.len(), 1, "one Orientation entry");
    let long = [&[0x12, 0x01, 4, 0, 1, 0, 0, 0][..], &to.to_le_bytes()].concat();
    b[at[0]..at[0] + long.len()].copy_from_slice(&long);
    b
}

#[test]
fn an_orientation_past_u16_is_not_truncated_into_range() {
    // #1817: 65538 truncated to 2 and rotated the image; it is out of range, so 1.
    let mut spec = DngSpec::cfa(8, 6, vec![500; 48]);
    spec.orientation = 2;
    assert_eq!(sensor(&with_long_orientation(&spec, 2, 65_538)).orientation, 1);
    // Control: a LONG orientation in range is read.
    assert_eq!(sensor(&with_long_orientation(&spec, 2, 6)).orientation, 6);
}

#[test]
fn metadata_is_read() {
    let (w, h) = (40, 30);
    let mut spec = DngSpec::cfa(w, h, vec![1000; w * h]);
    spec.bits = 14;
    spec.white = 15000;
    spec.black = vec![100, 101, 102, 103];
    spec.black_repeat = (2, 2);
    spec.active_area = Some([2, 4, 28, 36]); // top, left, bottom, right
    spec.default_crop = Some(([2, 1], [20, 16]));
    spec.cfa = [1, 0, 2, 1]; // GRBG at the active-area origin
    spec.as_shot_neutral = Some([0.5, 1.0, 0.8]);
    spec.baseline_exposure = Some(0.5);
    spec.orientation = 6;
    spec.linearization = Some((0..4096u32).map(|v| (v * 3).min(65535) as u16).collect());
    let s = sensor(&spec.build());
    assert_eq!(s.white, [15000.0; 3]);
    assert_eq!(s.black.values, vec![100.0, 101.0, 102.0, 103.0]);
    assert_eq!(s.active, Rect::new(4, 2, 32, 26));
    assert_eq!(s.crop, Rect::new(6, 3, 20, 16));
    // Pattern anchored at the active area origin (4, 2): GRBG there.
    assert_eq!(s.cfa.as_ref().unwrap().phase(4, 2), [1, 0, 2, 1]);
    assert_eq!(s.color.as_shot_neutral, Some([0.5, 1.0, 0.8]));
    assert!((s.baseline_exposure - 0.5).abs() < 1e-6);
    assert_eq!(s.orientation, 6);
    assert_eq!(s.linearization.as_ref().map(Vec::len), Some(4096));
    let d = develop_sensor(&s, &DevelopOptions::default()).unwrap();
    // Orientation 6 swaps the cropped 20×16 to 16×20.
    assert_eq!((d.width, d.height), (16, 20));
    assert_eq!(d.info.wb_multipliers, [2.0, 1.0, 1.25]);
    let d = develop_sensor(&s, &DevelopOptions { orient: false, ..Default::default() }).unwrap();
    assert_eq!((d.width, d.height), (20, 16));
}

#[test]
fn enormous_double_crop_origin_is_rejected() {
    let (w, h) = (16, 12);
    let mut spec = DngSpec::cfa(w, h, vec![1000; w * h]);
    spec.active_area = Some([2, 3, 10, 14]);
    spec.default_crop = Some(([0, 0], [11, 8]));
    spec.default_crop_origin_double = Some([1e300, 0.0]);

    assert!(matches!(decode(&spec.build(), &Limits::default()), Err(RawError::Malformed(_))));
}

#[test]
fn out_of_range_default_crop_falls_back_to_active_area_with_warning() {
    let (w, h) = (16, 12);
    let mut spec = DngSpec::cfa(w, h, vec![1000; w * h]);
    spec.active_area = Some([2, 3, 10, 14]);
    spec.default_crop = Some(([10, 0], [5, 4]));

    let s = sensor(&spec.build());
    assert_eq!(s.crop, s.active);
    assert!(s.warnings.iter().any(|warning| warning.contains("out-of-range") && warning.contains("default crop")), "{:?}", s.warnings);
}

/// DefaultCropSize defaults to the whole image, which can't fit once the origin moves in.
#[test]
fn default_crop_origin_without_size_falls_back_to_active_area_with_warning() {
    let (w, h) = (16, 12);
    let mut spec = DngSpec::cfa(w, h, vec![1000; w * h]);
    spec.active_area = Some([2, 3, 10, 14]);
    spec.default_crop_origin_double = Some([1.0, 1.0]);

    let s = sensor(&spec.build());
    assert_eq!(s.crop, s.active);
    assert!(s.warnings.iter().any(|warning| warning.contains("out-of-range") && warning.contains("default crop")), "{:?}", s.warnings);
}

/// #950: DefaultCropOrigin defaults to (0, 0), so a size-only crop starts at the active area's
/// top-left corner.
#[test]
fn default_crop_size_without_origin_crops_from_the_active_area_origin() {
    let (w, h) = (8, 8);
    let mut spec = DngSpec::cfa(w, h, (0..w * h).map(|i| (i as u16) * 37).collect());
    spec.active_area = Some([0, 0, 8, 8]);
    spec.default_crop_size_only = Some([4, 4]);
    let bytes = spec.build();

    let s = sensor(&bytes);
    assert_eq!(s.crop, Rect::new(0, 0, 4, 4));
    assert!(s.warnings.iter().all(|warning| !warning.contains("default crop")), "{:?}", s.warnings);
    let d = develop(&bytes, &DevelopOptions::default()).unwrap();
    assert_eq!((d.width, d.height), (4, 4));

    // Inside an offset active area the crop starts at that area's corner.
    let (w, h) = (16, 12);
    let mut spec = DngSpec::cfa(w, h, vec![1000; w * h]);
    spec.active_area = Some([2, 3, 10, 14]);
    spec.default_crop_size_only = Some([6, 4]);
    let s = sensor(&spec.build());
    assert_eq!(s.crop, Rect::new(3, 2, 6, 4));
}

/// Linear sRGB (D65) → XYZ, from the sRGB primaries (IEC 61966-2-1).
const SRGB_TO_XYZ: [[f64; 3]; 3] = [[0.4124564, 0.3575761, 0.1804375], [0.2126729, 0.7151522, 0.0721750], [0.0193339, 0.1191920, 0.9503041]];

fn invert(m: [[f64; 3]; 3]) -> [[f64; 3]; 3] {
    let [[a, b, c], [d, e, f], [g, h, i]] = m;
    let det = a * (e * i - f * h) - b * (d * i - f * g) + c * (d * h - e * g);
    [
        [(e * i - f * h) / det, (c * h - b * i) / det, (b * f - c * e) / det],
        [(f * g - d * i) / det, (a * i - c * g) / det, (c * d - a * f) / det],
        [(d * h - e * g) / det, (b * g - a * h) / det, (a * e - b * d) / det],
    ]
}

fn ap(m: &[[f64; 3]; 3], v: [f64; 3]) -> [f64; 3] {
    [0, 1, 2].map(|i| m[i][0] * v[0] + m[i][1] * v[1] + m[i][2] * v[2])
}

/// The DNG model computed independently: camera XYZ of `camera` under an
/// illuminant whose camera neutral is `neutral`, Bradford-adapted from that
/// white (normalized to Y = 1) to D50, then XYZ D50 → linear ProPhoto.
fn reference_prophoto(camera: [f64; 3], neutral: [f64; 3]) -> [f64; 3] {
    const BRADFORD: [[f64; 3]; 3] = [[0.8951, 0.2664, -0.1614], [-0.7502, 1.7135, 0.0367], [0.0389, -0.0685, 1.0296]];
    const D50: [f64; 3] = [0.9642, 1.0, 0.8249];
    const XYZ_D50_TO_PROPHOTO: [[f64; 3]; 3] = [[1.3459433, -0.2556075, -0.0511118], [-0.5445989, 1.5081673, 0.0205351], [0.0, 0.0, 1.2118128]];
    let white = ap(&SRGB_TO_XYZ, neutral);
    let white = white.map(|v| v / white[1]);
    let xyz = ap(&SRGB_TO_XYZ, camera).map(|v| v / ap(&SRGB_TO_XYZ, neutral)[1]);
    let (s, d) = (ap(&BRADFORD, white), ap(&BRADFORD, D50));
    let cone = ap(&BRADFORD, xyz);
    let adapted = ap(&invert(BRADFORD), [cone[0] * d[0] / s[0], cone[1] * d[1] / s[1], cone[2] * d[2] / s[2]]);
    ap(&XYZ_D50_TO_PROPHOTO, adapted)
}

#[test]
fn colour_matrix_development_matches_reference() {
    // A camera whose native RGB is linear sRGB: ColorMatrix = XYZ → sRGB at D65.
    // The scene is captured with per-channel gains (the "white balance" the camera
    // must undo), recorded as AsShotNeutral.
    let (w, h) = (48, 32);
    let gains = [0.5f32, 1.0, 0.7];
    let patches: [[f32; 3]; 4] = [[0.18, 0.18, 0.18], [0.40, 0.10, 0.05], [0.05, 0.30, 0.10], [0.10, 0.12, 0.45]];
    let rgb: Vec<[f32; 3]> = (0..w * h)
        .map(|i| {
            let p = patches[((i % w) / (w / 2)) + 2 * ((i / w) / (h / 2))];
            [p[0] * gains[0], p[1] * gains[1], p[2] * gains[2]]
        })
        .collect();
    let mut spec = DngSpec::cfa(w, h, mosaic(&rgb, w, [0, 1, 1, 2], 256, 16383));
    spec.bits = 14;
    spec.black = vec![256];
    spec.white = 16383;
    let cm = invert(SRGB_TO_XYZ);
    spec.color_matrix1 = Some((21, [cm[0][0], cm[0][1], cm[0][2], cm[1][0], cm[1][1], cm[1][2], cm[2][0], cm[2][1], cm[2][2]]));
    spec.as_shot_neutral = Some(gains.map(f64::from));
    for storage in [DngStorage::Strips { rows: 32 }, DngStorage::Lj92Tiles { width: 16, height: 16 }] {
        spec.storage = storage;
        for method in [Demosaic::Bilinear, Demosaic::Mhc, Demosaic::Ahd] {
            let d = develop(&spec.build(), &DevelopOptions { demosaic: method, ..Default::default() }).unwrap();
            for (pi, p) in patches.iter().enumerate() {
                // Centre of each patch.
                let (cx, cy) = ((pi % 2) * (w / 2) + w / 4, (pi / 2) * (h / 2) + h / 4);
                let o = &d.rgb[(cy * w + cx) * 3..(cy * w + cx) * 3 + 3];
                let g = gains.map(f64::from);
                let want = reference_prophoto([0, 1, 2].map(|c| f64::from(p[c]) * g[c]), g);
                for c in 0..3 {
                    let got = (f64::from(o[c]) / 65535.0).powf(1.8);
                    assert!((got - want[c]).abs() < 0.006, "{storage:?} {method:?} patch {pi} c {c}: got {got:.4} want {:.4}", want[c]);
                }
            }
        }
    }
}

#[test]
fn forward_matrix_path_maps_neutral_to_white() {
    let (w, h) = (16, 16);
    let rgb = vec![[0.3f32, 0.6, 0.45]; w * h]; // a grey under a green-ish light
    let mut spec = DngSpec::cfa(w, h, mosaic(&rgb, w, [0, 1, 1, 2], 0, 65535));
    let cm = invert(SRGB_TO_XYZ);
    let flat = |m: [[f64; 3]; 3]| [m[0][0], m[0][1], m[0][2], m[1][0], m[1][1], m[1][2], m[2][0], m[2][1], m[2][2]];
    spec.color_matrix1 = Some((17, flat(cm)));
    spec.color_matrix2 = Some((21, flat(cm)));
    // ForwardMatrix: balanced camera → XYZ D50 (sRGB adapted to D50; white → D50).
    let fm = [[0.4360747, 0.3850649, 0.1430804], [0.2225045, 0.7168786, 0.0606169], [0.0139322, 0.0971045, 0.7141733]];
    spec.forward_matrix1 = Some(flat(fm));
    spec.forward_matrix2 = Some(flat(fm));
    spec.as_shot_neutral = Some([0.5, 1.0, 0.75]);
    let d = develop(&spec.build(), &DevelopOptions::default()).unwrap();
    let o = &d.rgb[(8 * w + 8) * 3..(8 * w + 8) * 3 + 3];
    // Balanced grey 0.6 → ProPhoto (0.6, 0.6, 0.6).
    for (c, v) in o.iter().enumerate() {
        let got = (f64::from(*v) / 65535.0).powf(1.8);
        assert!((got - 0.6).abs() < 0.004, "c {c}: {got}");
    }
}

#[test]
fn highlights_clip_to_white_and_exposure_applies() {
    let (w, h) = (8, 8);
    let mut spec = DngSpec::cfa(w, h, vec![65535; w * h]);
    spec.as_shot_neutral = Some([0.5, 1.0, 0.6]);
    let d = develop(&spec.build(), &DevelopOptions::default()).unwrap();
    assert!(d.rgb.iter().all(|&v| v >= 65500), "clipped sensor must render white");
    let mut spec = DngSpec::cfa(w, h, vec![16384; w * h]); // 0.25 linear grey
    spec.as_shot_neutral = Some([1.0, 1.0, 1.0]);
    let b = spec.build();
    let base = develop(&b, &DevelopOptions::default()).unwrap().rgb[27];
    let up = develop(&b, &DevelopOptions { exposure: 1.0, ..Default::default() }).unwrap().rgb[27];
    let lin = |v: u16| (f64::from(v) / 65535.0).powf(1.8);
    assert!((lin(up) / lin(base) - 2.0).abs() < 0.02, "{} vs {}", lin(up), lin(base));
}

/// storytold/photocraft#4348 (as storytold/lightcraft#548): a 12-bit DNG that states WhiteLevel 4095
/// but whose sensor saturates at 3567 in every colour. Under the camera's white balance (red x2.46,
/// blue x1.59) the stuck red and blue samples clip at 1 while green stays at 0.85, so a blown wall
/// rendered magenta. The wall must come out neutral, and the unclipped ramp beside it unchanged.
#[test]
fn highlights_that_clip_below_the_stated_white_level_stay_neutral() {
    let (w, h) = (64, 32);
    let data: Vec<u16> = (0..w * h).map(|i| if i % w < w / 2 { 528 + ((i % w) * 90 + (i / w) * 3) as u16 } else { 3567 }).collect();
    let mut spec = DngSpec::cfa(w, h, data.clone());
    spec.bits = 12;
    spec.black = vec![528];
    spec.white = 4095;
    spec.as_shot_neutral = Some([1.0 / 2.46, 1.0, 1.0 / 1.59]);
    let b = spec.build();
    let d = develop(&b, &DevelopOptions::default()).unwrap();
    let px = &d.rgb[(16 * w + 48) * 3..(16 * w + 48) * 3 + 3];
    let lin = |v: u16| (f64::from(v) / 65535.0).powf(1.8);
    let tint = (lin(px[0]) / lin(px[1]) - 1.0).abs().max((lin(px[2]) / lin(px[1]) - 1.0).abs());
    assert!(tint < 0.01, "the clipped wall is tinted: {px:?}");
    let s = sensor(&b);
    for (i, (&got, &was)) in s.data.iter().zip(&data).enumerate() {
        assert_eq!(got, if was == 3567 { 4095 } else { was }, "sample {i}");
    }
}

#[test]
fn gain_map_opcode_is_applied() {
    // Uniform 0.25 grey; a gain map that doubles everything over the whole image.
    let (w, h) = (16, 12);
    let mut spec = DngSpec::cfa(w, h, vec![16384; w * h]);
    spec.as_shot_neutral = Some([1.0; 3]);
    let lin = |v: u16| (f64::from(v) / 65535.0).powf(1.8);
    let base = develop(&spec.build(), &DevelopOptions::default()).unwrap().rgb[(6 * w + 8) * 3 + 1];
    spec.opcode_list2 = Some(photocraft_raw::testgen::gain_map_opcode_list([0, 0, h as u32, w as u32], 1, [[2.0, 2.0], [2.0, 2.0]]));
    let d = develop(&spec.build(), &DevelopOptions::default()).unwrap();
    assert!(!d.warnings.iter().any(|w| w.contains("Opcode")), "{:?}", d.warnings);
    let gained = d.rgb[(6 * w + 8) * 3 + 1];
    assert!((lin(gained) / lin(base) - 2.0).abs() < 0.02, "{} vs {}", lin(gained), lin(base));
}

#[test]
fn white_balance_options() {
    let (w, h) = (16, 16);
    let rgb = vec![[0.2f32, 0.4, 0.3]; w * h];
    let spec = DngSpec::cfa(w, h, mosaic(&rgb, w, [0, 1, 1, 2], 0, 65535));
    let b = spec.build();
    // No AsShotNeutral: grey world → neutral output.
    let d = develop(&b, &DevelopOptions::default()).unwrap();
    assert!(d.warnings.iter().any(|w| w.contains("grey world")));
    let o = &d.rgb[(8 * w + 8) * 3..(8 * w + 8) * 3 + 3];
    assert!((i32::from(o[0]) - i32::from(o[1])).abs() < 300 && (i32::from(o[2]) - i32::from(o[1])).abs() < 300, "{o:?}");
    let d = develop(&b, &DevelopOptions { white_balance: WhiteBalance::Multipliers([2.0, 1.0, 4.0 / 3.0]), ..Default::default() }).unwrap();
    assert_eq!(d.info.wb_multipliers.map(|v| (v * 1000.0).round()), [2000.0, 1000.0, 1333.0]);
    assert!(develop(&b, &DevelopOptions { white_balance: WhiteBalance::Multipliers([0.0, 1.0, 1.0]), ..Default::default() }).is_err());
}

#[test]
fn smooth_scene_develops_with_every_method() {
    let (w, h) = (64, 48);
    let rgb = scene(w, h);
    let spec = DngSpec::cfa(w, h, mosaic(&rgb, w, [2, 1, 1, 0], 0, 65535));
    let mut spec = spec;
    spec.cfa = [2, 1, 1, 0];
    spec.as_shot_neutral = Some([1.0; 3]);
    let b = spec.build();
    let outs: Vec<Vec<u16>> = [Demosaic::Bilinear, Demosaic::Mhc, Demosaic::Ahd]
        .iter()
        .map(|&m| develop(&b, &DevelopOptions { demosaic: m, ..Default::default() }).unwrap().rgb)
        .collect();
    // Interior pixels agree closely across methods on a smooth scene.
    for y in 4..h - 4 {
        for x in 4..w - 4 {
            for c in 0..3 {
                let i = (y * w + x) * 3 + c;
                let (a, b, d) = (i32::from(outs[0][i]), i32::from(outs[1][i]), i32::from(outs[2][i]));
                assert!((a - b).abs() < 800 && (a - d).abs() < 800, "({x},{y}) c{c}: {a} {b} {d}");
            }
        }
    }
}

#[test]
fn unsupported_variants_say_so() {
    let mut spec = DngSpec::cfa(8, 8, vec![0; 64]);
    spec.cfa = [0, 1, 2, 1]; // not Bayer
    assert!(matches!(decode(&spec.build(), &Limits::default()), Err(RawError::Unsupported(_))));
}

#[test]
fn limits_are_enforced_before_allocating() {
    let spec = DngSpec::cfa(64, 64, vec![0; 64 * 64]);
    let b = spec.build();
    let tiny = Limits { max_width: 32, ..Limits::default() };
    assert!(matches!(decode(&b, &tiny), Err(RawError::LimitExceeded(_))));
    let tiny = Limits { max_alloc: 1000, ..Limits::default() };
    assert!(matches!(decode(&b, &tiny), Err(RawError::LimitExceeded(_))));
}

/// Adobe DNG compression 8 (zlib): with and without the horizontal-differencing predictor,
/// with and without 12-bit packing, and both byte orders — each decodes to the exact samples.
#[test]
fn deflate_strips_round_trip() {
    let (w, h) = (48, 40);
    for (name, bits, storage, be) in [
        ("deflate 16-bit", 16, DngStorage::DeflateStrips { rows: 16, predictor: 1 }, false),
        ("deflate 16-bit predictor 2", 16, DngStorage::DeflateStrips { rows: 16, predictor: 2 }, false),
        ("deflate 12-bit predictor 2", 12, DngStorage::DeflateStrips { rows: 12, predictor: 2 }, false),
        ("deflate 16-bit predictor 2 BE", 16, DngStorage::DeflateStrips { rows: 16, predictor: 2 }, true),
        ("deflate LinearRaw predictor 2", 16, DngStorage::DeflateStrips { rows: 40, predictor: 2 }, false),
    ] {
        let samples = if name.contains("LinearRaw") { 3 } else { 1 };
        let data = noise(w * h * samples, bits);
        let mut spec = DngSpec::cfa(w, h, data.clone());
        spec.samples = samples;
        spec.bits = bits;
        spec.storage = storage;
        spec.big_endian = be;
        spec.white = (1 << bits) - 1;
        let s = sensor(&spec.build());
        assert_eq!((s.width, s.height, s.samples), (w, h, samples), "{name}");
        assert_eq!(s.data, data, "{name}");
    }
}

/// A zlib stream cut short must fail cleanly (decompression comes up short or errors), and a
/// declared-but-absent giant tile must trip the limits instead of allocating.
#[test]
fn deflate_hostile_fails_cleanly() {
    let (w, h) = (32, 32);
    let data = noise(w * h, 16);
    let mut spec = DngSpec::cfa(w, h, data);
    spec.storage = DngStorage::DeflateStrips { rows: 32, predictor: 2 };
    let bytes = spec.build();
    assert_eq!(sensor(&bytes).data.len(), w * h);
    // Locate the zlib stream (78 9c header of the single strip) and cut it mid-stream.
    let at = bytes.windows(2).position(|w| w == [0x78, 0x9c]).expect("zlib header") + 2;
    let cut = &bytes[..at + (bytes.len() - at) / 2];
    assert!(decode(cut, &Limits::default()).is_err(), "a cut zlib stream must not decode");
}

/// Predictor 2 on 8-bit samples wraps at 8 bits, and 12-bit samples bit-packed inside the zlib
/// stream (not in 16-bit containers) decode too.
#[test]
fn deflate_8_bit_predictor_and_packed_12_bit() {
    use std::io::Write as _;
    let (w, h) = (16usize, 4usize);
    let zlib = |bytes: &[u8]| {
        let mut e = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        e.write_all(bytes).unwrap();
        e.finish().unwrap()
    };
    // 8-bit, predictor 2: a row falling from 250 to 10 has deltas that wrap mod 256.
    let data8: Vec<u16> = (0..w * h).map(|i| if i % w < w / 2 { 250 } else { 10 }).collect();
    let mut spec = DngSpec::cfa(w, h, data8.clone());
    spec.bits = 8;
    spec.white = 255;
    spec.storage = DngStorage::DeflateStrips { rows: h, predictor: 2 };
    assert_eq!(sensor(&spec.build()).data, data8, "8-bit predictor 2");
    // 12-bit packed: two samples in three bytes, MSB first. Swap the testgen's 16-bit
    // containers for a packed stream of the same samples.
    let data12: Vec<u16> = (0..w * h).map(|i| (i as u16 * 61) & 0xFFF).collect();
    let mut spec = DngSpec::cfa(w, h, data12.clone());
    spec.bits = 12;
    spec.white = 4095;
    spec.storage = DngStorage::DeflateStrips { rows: h, predictor: 1 };
    let mut bytes = spec.build();
    let containers: Vec<u8> = data12.iter().flat_map(|v| v.to_le_bytes()).collect();
    let old = zlib(&containers);
    let packed: Vec<u8> = data12.chunks(2).flat_map(|p| [(p[0] >> 4) as u8, ((p[0] & 0xF) << 4 | p[1] >> 8) as u8, p[1] as u8]).collect();
    let mut new = zlib(&packed);
    let at = bytes.windows(old.len()).position(|c| c == old.as_slice()).expect("the strip");
    assert!(new.len() <= old.len());
    new.resize(old.len(), 0); // trailing bytes after the zlib stream are ignored
    bytes[at..at + old.len()].copy_from_slice(&new);
    assert_eq!(sensor(&bytes).data, data12, "12-bit packed");
}

// ---------------------------------------------------------------- JPEG XL (DNG 1.7)

/// Losslessly encodes `w × h` samples (1 = grey, 3 = RGB) at 8 or 16 bits as a JPEG XL codestream.
fn jxl(samples: &[u16], w: usize, h: usize, channels: usize, bits: u16) -> Vec<u8> {
    use zune_core::{bit_depth::BitDepth, colorspace::ColorSpace, options::EncoderOptions};
    let (depth, bytes): (BitDepth, Vec<u8>) = match bits {
        8 => (BitDepth::Eight, samples.iter().map(|&v| v as u8).collect()),
        _ => (BitDepth::Sixteen, samples.iter().flat_map(|v| v.to_ne_bytes()).collect()),
    };
    let space = if channels == 1 { ColorSpace::Luma } else { ColorSpace::RGB };
    let mut out = Vec::new();
    zune_jpegxl::JxlSimpleEncoder::new(&bytes, EncoderOptions::new(w, h, space, depth)).encode(&mut out).unwrap();
    out
}

/// `data` (`w × h × spp`) cut into `tw × th` tiles, edge tiles padded by repeating the last row / column.
fn tiles(data: &[u16], w: usize, h: usize, spp: usize, tw: usize, th: usize) -> Vec<Vec<u16>> {
    let mut out = Vec::new();
    for ty in (0..h).step_by(th) {
        for tx in (0..w).step_by(tw) {
            let mut t = Vec::with_capacity(tw * th * spp);
            for y in 0..th {
                for x in 0..tw {
                    let at = ((ty + y).min(h - 1) * w + (tx + x).min(w - 1)) * spp;
                    t.extend_from_slice(&data[at..at + spp]);
                }
            }
            out.push(t);
        }
    }
    out
}

#[test]
fn jpeg_xl_layouts_decode_exactly() {
    let (w, h) = (37, 23); // odd sizes: a short last strip and partial edge tiles
    for (name, spp, bits, tiled) in [
        ("LinearRaw 16-bit strips", 3, 16, false),
        ("LinearRaw 16-bit tiles", 3, 16, true),
        ("CFA 16-bit strips", 1, 16, false),
        ("CFA 16-bit tiles", 1, 16, true),
        ("LinearRaw 8-bit strips", 3, 8, false),
        ("CFA 8-bit tiles", 1, 8, true),
    ] {
        let data = noise(w * h * spp, bits);
        let mut spec = DngSpec::cfa(w, h, data.clone());
        spec.samples = spp;
        spec.bits = bits;
        spec.white = (1u32 << bits) - 1;
        if tiled {
            let (tw, th) = (16, 16);
            spec.storage = DngStorage::JxlTiles { width: tw, height: th };
            spec.jxl_segments = tiles(&data, w, h, spp, tw, th).iter().map(|t| jxl(t, tw, th, spp, bits)).collect();
        } else {
            let rows = 10;
            spec.storage = DngStorage::JxlStrips { rows };
            spec.jxl_segments =
                (0..h).step_by(rows).map(|y| (y, (y + rows).min(h) - y)).map(|(y, r)| jxl(&data[y * w * spp..(y + r) * w * spp], w, r, spp, bits)).collect();
        }
        let s = sensor(&spec.build());
        assert_eq!((s.width, s.height, s.samples), (w, h, spp), "{name}");
        assert_eq!(s.data, data, "{name}");
    }
}

#[test]
fn jpeg_xl_linear_raw_develops() {
    // A Samsung Expert RAW-like file: one 16-bit LinearRaw JPEG XL strip with a baseline exposure.
    let (w, h) = (24, 16);
    let data: Vec<u16> = scene(w, h).iter().flat_map(|p| p.map(|v| (v * 20000.0) as u16)).collect();
    let mut spec = DngSpec::cfa(w, h, data.clone());
    spec.samples = 3;
    spec.storage = DngStorage::JxlStrips { rows: h };
    spec.jxl_segments = vec![jxl(&data, w, h, 3, 16)];
    spec.baseline_exposure = Some(1.0);
    let b = spec.build();
    let d = develop(&b, &DevelopOptions::default()).unwrap();
    assert_eq!((d.width as usize, d.height as usize, d.rgb.len()), (w, h, w * h * 3));
    // Same pixels stored uncompressed develop identically.
    spec.storage = DngStorage::Strips { rows: h };
    assert_eq!(develop(&spec.build(), &DevelopOptions::default()).unwrap().rgb, d.rgb);
}

/// A LinearRaw JPEG XL strip of `data` (16-bit samples) whose raw IFD says the samples are
/// floating-point (SampleFormat 3), as converters write lossy DNGs (#8125). The codestream is
/// an integer one (the test encoder writes no floats): the decoder reads it as floats in 0..=1,
/// the same values a float codestream of `data / 65535` holds.
fn float_jxl_dng(data: &[u16], w: usize, h: usize, white: Option<f64>) -> DngSpec {
    let mut spec = DngSpec::cfa(w, h, data.to_vec());
    spec.samples = 3;
    spec.white = 0;
    spec.storage = DngStorage::JxlStrips { rows: h };
    spec.jxl_segments = vec![jxl(data, w, h, 3, 16)];
    spec.raw_extra = vec![(339, Val::Short(vec![3; 3]))];
    if let Some(white) = white {
        spec.raw_extra.push((50717, Val::Double(vec![white; 3])));
    }
    spec
}

#[test]
fn floating_point_jpeg_xl_linear_raw_decodes() {
    let (w, h) = (24, 16);
    let data = noise(w * h * 3, 16);
    // White level 1.0 (the default) becomes 65535: the samples read back as stored.
    let s = sensor(&float_jxl_dng(&data, w, h, None).build());
    assert_eq!((s.width, s.height, s.samples), (w, h, 3));
    assert_eq!(s.data, data);
    assert_eq!(s.white, [65535.0; 3]);
    // A stated white level of 0.5 becomes 65535 too: samples double, those above it clip.
    let s = sensor(&float_jxl_dng(&data, w, h, Some(0.5)).build());
    assert_eq!(s.white, [65535.0; 3]);
    assert_eq!(s.data, data.iter().map(|&v| (u32::from(v) * 2).min(65535) as u16).collect::<Vec<_>>());
    // It develops like the same samples stored as integers.
    let pixels: Vec<u16> = scene(w, h).iter().flat_map(|p| p.map(|v| (v * 20000.0) as u16)).collect();
    let float = develop(&float_jxl_dng(&pixels, w, h, None).build(), &DevelopOptions::default()).unwrap();
    let mut ints = DngSpec::cfa(w, h, pixels.clone());
    ints.samples = 3;
    ints.white = 65535;
    ints.storage = DngStorage::Strips { rows: h };
    assert_eq!(float.rgb, develop(&ints.build(), &DevelopOptions::default()).unwrap().rgb);
}

#[test]
fn floating_point_raw_data_that_isnt_jpeg_xl_is_unsupported() {
    let (w, h) = (8, 4);
    let mut spec = float_jxl_dng(&noise(w * h * 3, 16), w, h, None);
    spec.storage = DngStorage::Strips { rows: h };
    match decode(&spec.build(), &Limits::default()) {
        Err(RawError::Unsupported(m)) => assert!(m.contains("floating-point"), "{m}"),
        other => panic!("expected unsupported, got {other:?}"),
    }
}

/// WhiteLevel has one value per sample plane: the floats are scaled by the highest, so a plane
/// with a higher white level than the first keeps its highlights.
#[test]
fn floating_point_white_levels_per_plane_scale_from_the_highest() {
    let (w, h) = (24, 16);
    let data = noise(w * h * 3, 16);
    let mut spec = float_jxl_dng(&data, w, h, None);
    spec.raw_extra.push((50717, Val::Double(vec![0.5, 1.0, 0.5])));
    let s = sensor(&spec.build());
    assert_eq!(s.data, data, "scaled by 1.0, the highest: nothing clipped");
    assert_eq!(s.white, [32767.5, 65535.0, 32767.5]);
}

/// `Σ cᵢ · xⁱ` at `x`, both clipped to 0..1 (as MapPolynomial).
fn poly(c: &[f64], x: f64) -> f64 {
    let x = x.clamp(0.0, 1.0);
    c.iter().rev().fold(0.0, |acc, c| acc * x + c).clamp(0.0, 1.0)
}

/// #8125: a lossy DNG's OpcodeList2 carries a MapPolynomial per plane, and its colours depend on
/// it. Each plane's samples are mapped through its polynomial, on the linear reference values.
#[test]
fn map_polynomial_maps_each_plane_of_floating_point_data() {
    let (w, h) = (24, 16);
    let data = noise(w * h * 3, 16);
    let planes = [vec![0.0, 0.5, 0.5], vec![0.0, 1.0], vec![0.1, 0.8, 0.0, 0.3]];
    let mut spec = float_jxl_dng(&data, w, h, None);
    spec.opcode_list2 = Some(testgen::map_polynomial_opcode_list(
        [0, 0, h as u32, w as u32],
        &planes.iter().enumerate().map(|(p, c)| (p as u32, c.clone())).collect::<Vec<_>>(),
    ));
    let bytes = spec.build();
    let s = sensor(&bytes);
    let want: Vec<u16> = data.iter().enumerate().map(|(i, &v)| (poly(&planes[i % 3], f64::from(v) / 65535.0) * 65535.0).round() as u16).collect();
    assert_eq!(s.data, want);
    assert!(s.warnings.iter().all(|w| !w.contains("MapPolynomial")), "{:?}", s.warnings);
    let d = develop(&bytes, &DevelopOptions::default()).unwrap();
    assert!(d.warnings.iter().all(|w| !w.contains("MapPolynomial")), "{:?}", d.warnings);
}

/// With a LinearizationTable, MapPolynomial maps the linearized values: the samples are
/// linearized first and the table is then done with.
#[test]
fn map_polynomial_follows_the_linearization_table() {
    let (w, h) = (16, 8);
    let data = noise(w * h, 12);
    let table: Vec<u16> = (0..4096u16).map(|i| i * 2).collect();
    let mut spec = DngSpec::cfa(w, h, data.clone());
    spec.bits = 12;
    spec.white = 8190;
    spec.linearization = Some(table.clone());
    spec.opcode_list2 = Some(testgen::map_polynomial_opcode_list([0, 0, h as u32, w as u32], &[(0, vec![0.0, 0.0, 1.0])]));
    let s = sensor(&spec.build());
    assert!(s.linearization.is_none(), "applied with the polynomial");
    let want: Vec<u16> = data.iter().map(|&v| (poly(&[0.0, 0.0, 1.0], f64::from(table[usize::from(v)]) / 8190.0) * 8190.0).round() as u16).collect();
    assert_eq!(s.data, want);
}

/// A LinearizationTable maps integer samples: floating-point data ignores one.
#[test]
fn floating_point_raw_data_ignores_a_linearization_table() {
    let (w, h) = (24, 16);
    let data = noise(w * h * 3, 16);
    let mut spec = float_jxl_dng(&data, w, h, None);
    spec.linearization = Some(vec![0; 4]);
    let s = sensor(&spec.build());
    assert!(s.linearization.is_none());
    assert_eq!(s.data, data);
}

/// Floating-point samples are read into four bytes each before they become 16-bit: the limits
/// are charged for that buffer, not for the two-byte result.
#[test]
fn floating_point_jpeg_xl_is_charged_four_bytes_a_sample() {
    let (w, h) = (24, 16);
    let bytes = float_jxl_dng(&noise(w * h * 3, 16), w, h, None).build();
    let limits = Limits { max_alloc: (w * h * 3 * 2) as u64, ..Limits::default() };
    match decode(&bytes, &limits) {
        Err(RawError::LimitExceeded(_)) => {}
        other => panic!("expected the limits to refuse the f32 buffer, got {other:?}"),
    }
}

/// When the largest JPEG XL preview doesn't decode, the next largest is used.
#[test]
fn a_jpeg_xl_preview_that_does_not_decode_gives_way_to_a_smaller_one() {
    use photocraft_raw::testgen::TiffBuilder;
    let mut t = TiffBuilder::default();
    let preview = |t: &mut TiffBuilder, w: usize, h: usize, stream: Vec<u8>| {
        let len = stream.len() as u32;
        let strip = t.blob(stream);
        t.ifd(vec![
            (254, Val::Long(vec![1])),
            (256, Val::Long(vec![w as u32])),
            (257, Val::Long(vec![h as u32])),
            (258, Val::Short(vec![8, 8, 8])),
            (259, Val::Short(vec![52546])),
            (262, Val::Short(vec![2])),
            (273, Val::Blobs(vec![strip])),
            (277, Val::Short(vec![3])),
            (278, Val::Long(vec![h as u32])),
            (279, Val::Long(vec![len])),
        ])
    };
    let small = noise(20 * 15 * 3, 8);
    let damaged = preview(&mut t, 40, 30, vec![0xFF, 0x0A, 0, 0, 0, 0, 0, 0]);
    let valid = preview(&mut t, 20, 15, jxl(&small, 20, 15, 3, 8));
    let ifd0 = t.ifd(vec![(254, Val::Long(vec![1])), (330, Val::Ifds(vec![damaged, valid])), (50706, Val::Byte(vec![1, 7, 0, 0]))]);
    t.chain = vec![ifd0];
    let p = embedded_jxl_preview(&t.build(), &Limits::default()).expect("the 20×15 preview");
    assert_eq!((p.width, p.height, p.bits), (20, 15, 8));
    assert_eq!(p.rgb, small);
}

/// Only a rendered RGB picture is a preview: a reduced-resolution LinearRaw image (NewSubFileType 1,
/// three samples, JPEG XL) holds undeveloped sensor data, so the RGB one is used even when smaller.
#[test]
fn a_reduced_resolution_linear_raw_is_not_a_jpeg_xl_preview() {
    use photocraft_raw::testgen::TiffBuilder;
    let mut t = TiffBuilder::default();
    let mut ifd = |w: usize, h: usize, photometric: u16, data: &[u16]| {
        let stream = jxl(data, w, h, 3, 8);
        let len = stream.len() as u32;
        let strip = t.blob(stream);
        t.ifd(vec![
            (254, Val::Long(vec![1])),
            (256, Val::Long(vec![w as u32])),
            (257, Val::Long(vec![h as u32])),
            (258, Val::Short(vec![8, 8, 8])),
            (259, Val::Short(vec![52546])),
            (262, Val::Short(vec![photometric])),
            (273, Val::Blobs(vec![strip])),
            (277, Val::Short(vec![3])),
            (278, Val::Long(vec![h as u32])),
            (279, Val::Long(vec![len])),
        ])
    };
    let small = noise(20 * 15 * 3, 8);
    let linear_raw = ifd(40, 30, 34892, &noise(40 * 30 * 3, 8));
    let rgb = ifd(20, 15, 2, &small);
    let ifd0 = t.ifd(vec![(254, Val::Long(vec![1])), (330, Val::Ifds(vec![linear_raw, rgb])), (50706, Val::Byte(vec![1, 7, 0, 0]))]);
    t.chain = vec![ifd0];
    let p = embedded_jxl_preview(&t.build(), &Limits::default()).expect("the 20×15 RGB preview");
    assert_eq!((p.width, p.height), (20, 15));
    assert_eq!(p.rgb, small);
}

/// A WhiteLevel too large for the sensor's levels (a DOUBLE past f32::MAX) is ignored, as a zero
/// one is: the samples keep the default white instead of an infinite one that blacks them out.
#[test]
fn an_infinite_white_level_falls_back_to_the_default() {
    let (w, h) = (24, 16);
    let pixels: Vec<u16> = scene(w, h).iter().flat_map(|p| p.map(|v| (v * 20000.0) as u16)).collect();
    let mut ints = DngSpec::cfa(w, h, pixels.clone());
    ints.samples = 3;
    ints.storage = DngStorage::Strips { rows: h };
    ints.white = 0;
    ints.raw_extra.push((50717, Val::Double(vec![1e300; 3])));
    assert_eq!(sensor(&ints.build()).white, [65535.0; 3], "integer data");
    let float = float_jxl_dng(&pixels, w, h, Some(1e300)).build();
    assert_eq!(sensor(&float).white, [65535.0; 3], "floating-point data");
    let default = develop(&float_jxl_dng(&pixels, w, h, None).build(), &DevelopOptions::default()).unwrap();
    assert_eq!(develop(&float, &DevelopOptions::default()).unwrap().rgb, default.rgb);
}

#[test]
fn jpeg_xl_mismatches_are_errors() {
    let (w, h) = (16, 8);
    let data = noise(w * h * 3, 16);
    let mut spec = DngSpec::cfa(w, h, data.clone());
    spec.samples = 3;
    spec.storage = DngStorage::JxlStrips { rows: h };
    // Codestream smaller than the strip.
    spec.jxl_segments = vec![jxl(&data[..w * 4 * 3], w, 4, 3, 16)];
    assert!(matches!(decode(&spec.build(), &Limits::default()), Err(RawError::Malformed(_))));
    // Grey codestream for RGB samples.
    spec.jxl_segments = vec![jxl(&data[..w * h], w, h, 1, 16)];
    assert!(matches!(decode(&spec.build(), &Limits::default()), Err(RawError::Unsupported(_))));
    // Not a codestream at all.
    spec.jxl_segments = vec![vec![0xff, 0x0a, 1, 2, 3, 4, 5]];
    assert!(matches!(decode(&spec.build(), &Limits::default()), Err(RawError::Malformed(_))));
}

#[test]
fn profile_gain_table_map_is_reported() {
    let gain_table = |s: &Sensor| s.warnings.iter().any(|w| w.contains("ProfileGainTableMap"));
    let mut spec = DngSpec::cfa(8, 8, vec![500; 64]);
    assert!(!gain_table(&sensor(&spec.build())));
    spec.profile_gain_table_map = Some(vec![0; 64]);
    assert!(gain_table(&sensor(&spec.build())));
}

// ---------------------------------------------------------------- lossy DNG (compression 34892)

/// The largest difference between decoded and source samples, in stored (8-bit) levels.
fn worst_diff(got: &[u16], want: &[u16]) -> u16 {
    assert_eq!(got.len(), want.len());
    got.iter().zip(want).map(|(g, w)| g.abs_diff(*w)).max().unwrap_or(0)
}

/// An 8-bit lossy DNG around `data` (every value below 256): BitsPerSample 8, WhiteLevel 255.
fn lossy_spec(w: usize, h: usize, data: Vec<u16>, storage: DngStorage) -> DngSpec {
    assert!(data.iter().all(|&v| v <= 255));
    let mut spec = DngSpec::cfa(w, h, data);
    spec.bits = 8;
    spec.white = 255;
    spec.storage = storage;
    spec
}

/// Adobe DNG compression 34892 (DNG 1.4 lossy JPEG): a CFA image whose tiles or strips are
/// baseline JPEGs of 8-bit gray samples. They decode as the stored values (0..255, like any
/// other 8-bit raw: levels and the linearization table apply afterwards), within the JPEG
/// quality band (Q60 artifacts are mild but real), from whole and partial edge tiles and
/// from strips with a short last strip.
#[test]
fn lossy_jpeg_cfa_tiles_and_strips_decode_stored_values() {
    let (w, h) = (37, 23); // odd: partial edge tiles and a short last strip
    // A smooth ramp: baseline JPEG at Q60 keeps it within a few quantisation steps.
    let data: Vec<u16> = (0..w * h).map(|i| ((i % w) * 5 + (i / w) * 2) as u16).collect();
    let cases = [
        ("16x8 tiles", DngStorage::LossyJpegTiles { width: 16, height: 8 }),
        ("one tile", DngStorage::LossyJpegTiles { width: 37, height: 23 }),
        ("strips of 5", DngStorage::LossyJpegStrips { rows: 5 }),
        ("one strip", DngStorage::LossyJpegStrips { rows: 23 }),
    ];
    for (name, storage) in cases {
        let bytes = lossy_spec(w, h, data.clone(), storage).build();
        assert_eq!(identify(&bytes), Some(RawFormat::Dng), "{name}");
        let s = sensor(&bytes);
        assert_eq!((s.width, s.height, s.samples, s.cfa.is_some()), (w, h, 1, true), "{name}");
        assert_eq!(s.white, [255.0; 3], "{name}");
        assert!(s.data.iter().all(|&v| v <= 255), "{name}: a sample above the 8-bit range");
        let worst = worst_diff(&s.data, &data);
        assert!(worst <= 4, "{name}: worst deviation from the source: {worst} levels");
        let d = develop_sensor(&s, &DevelopOptions::default()).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!((d.width as usize, d.height as usize, d.rgb.len()), (w, h, w * h * 3), "{name}");
        assert!(d.info.cfa.is_some(), "{name}");
    }
}

/// A 3-sample LinearRaw lossy DNG (as Adobe writes them: demosaiced, YCbCr-coded JPEG tiles)
/// decodes to the stored R, G, B, each channel at its own level, from tiles and strips.
#[test]
fn lossy_jpeg_linearraw_rgb_tiles_and_strips_decode_stored_values() {
    let (w, h) = (37, 23);
    // Channel offsets of 64 levels survive JPEG chroma quantisation at Q60; the ramp is smooth.
    let data: Vec<u16> = (0..w * h * 3).map(|i| ((i / 3 % w) * 2 + i / 3 / w) as u16 + (i % 3) as u16 * 64).collect();
    let cases = [
        ("16x16 tiles", DngStorage::LossyJpegTiles { width: 16, height: 16 }),
        ("one tile", DngStorage::LossyJpegTiles { width: 37, height: 23 }),
        ("strips of 7", DngStorage::LossyJpegStrips { rows: 7 }),
    ];
    for (name, storage) in cases {
        let mut spec = lossy_spec(w, h, data.clone(), storage);
        spec.samples = 3;
        let s = sensor(&spec.build());
        assert_eq!((s.width, s.height, s.samples, s.cfa.is_none()), (w, h, 3, true), "{name}");
        assert!(s.data.iter().all(|&v| v <= 255), "{name}: a sample above the 8-bit range");
        let worst = worst_diff(&s.data, &data);
        assert!(worst <= 8, "{name}: worst deviation from the source: {worst} levels");
        // Each channel carries its own level: R and B stay apart everywhere.
        for p in 0..w * h {
            let (r, b) = (s.data[p * 3], s.data[p * 3 + 2]);
            assert!(b >= r + 64 - 8, "{name}: channels collapsed at pixel {p}: R {r}, B {b}");
        }
        let d = develop_sensor(&s, &DevelopOptions::default()).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!((d.width as usize, d.height as usize, d.rgb.len()), (w, h, w * h * 3), "{name}");
    }
}

/// The decoded 8-bit values are the stored samples, so WhiteLevel defaults to 255 when absent,
/// BlackLevel / WhiteLevel scale them and a LinearizationTable maps them, exactly as for an
/// uncompressed 8-bit DNG. Flat images keep the comparison inside the JPEG's DC rounding.
#[test]
fn lossy_jpeg_levels_and_linearization_apply_to_stored_values() {
    let (w, h) = (24, 16);
    let tiles = DngStorage::LossyJpegTiles { width: 16, height: 8 };
    let flat = |v: u16| vec![v; w * h];
    let grey = |d: &Developed| d.rgb.chunks(3).map(|p| p[0]).collect::<Vec<u16>>();

    // No WhiteLevel: 255 is white. A flat 255 develops to full white, a flat 0 to black.
    for (v, lo, hi) in [(255u16, 65000u16, 65535u16), (0, 0, 300)] {
        let mut spec = DngSpec::monochrome(w, h, flat(v));
        spec.bits = 8;
        spec.white = 0;
        spec.storage = tiles;
        let s = sensor(&spec.build());
        assert_eq!(s.white, [255.0; 3]);
        let d = develop_sensor(&s, &DevelopOptions::default()).unwrap();
        assert!(grey(&d).iter().all(|&g| (lo..=hi).contains(&g)), "flat {v}: {:?}", &grey(&d)[..4]);
    }

    // BlackLevel 20 / WhiteLevel 220 scale the stored values like the uncompressed path.
    let levels = |storage: DngStorage| {
        let mut spec = DngSpec::monochrome(w, h, flat(200));
        spec.bits = 8;
        spec.black = vec![20];
        spec.white = 220;
        spec.storage = storage;
        let s = sensor(&spec.build());
        assert_eq!((s.white, s.black.at(0, 0, 0, 1)), ([220.0; 3], 20.0));
        grey(&develop_sensor(&s, &DevelopOptions::default()).unwrap())
    };
    let (lossy, plain) = (levels(tiles), levels(DngStorage::Strips { rows: h }));
    assert!(plain[0] > 50_000 && plain[0] < 65_000, "{}", plain[0]);
    let worst = worst_diff(&lossy, &plain);
    assert!(worst <= 600, "levels: lossy and uncompressed differ by {worst}");

    // A LinearizationTable indexed by the stored value. It is flat around 200, so the JPEG's
    // DC rounding (199 or 200) lands on the same output: the two paths develop identically.
    let table: Vec<u16> = (0..256u32).map(|v| if (190..=210).contains(&v) { 40_000 } else { (v * 257) as u16 }).collect();
    let linearized = |storage: DngStorage| {
        let mut spec = DngSpec::monochrome(w, h, flat(200));
        spec.bits = 8;
        spec.white = 0;
        spec.linearization = Some(table.clone());
        spec.storage = storage;
        let s = sensor(&spec.build());
        assert_eq!(s.linearization.as_deref(), Some(table.as_slice()));
        assert_eq!(s.white, [65535.0; 3], "the table's maximum is white");
        grey(&develop_sensor(&s, &DevelopOptions::default()).unwrap())
    };
    let (lossy, plain) = (linearized(tiles), linearized(DngStorage::Strips { rows: h }));
    assert!(plain[0] > 40_000 && plain[0] < 60_000, "{}", plain[0]);
    assert_eq!(lossy, plain, "linearization: the lossy path differs from the uncompressed one");
}

/// A lossy JPEG tile cut mid-stream fails cleanly.
#[test]
fn lossy_jpeg_truncated_tile_errors() {
    let (w, h) = (32, 32);
    let bytes = lossy_spec(w, h, vec![128; w * h], DngStorage::LossyJpegTiles { width: 32, height: 32 }).build();
    let at = bytes.windows(2).position(|w| w == [0xFF, 0xD8]).expect("jpeg SOI") + 2;
    let cut = &bytes[..at + (bytes.len() - at) / 2];
    assert!(matches!(decode(cut, &Limits::default()), Err(RawError::Malformed(_))), "a cut JPEG tile must not decode");
}

/// A tile JPEG whose header disagrees with the tile size fails the header check, before any
/// pixel decode (a hostile SOF must not be able to size the decoder's buffers first).
#[test]
fn lossy_jpeg_tile_dimension_mismatch_errors() {
    let (w, h) = (32, 32);
    let mut spec = lossy_spec(w, h, vec![100; w * h], DngStorage::LossyJpegTiles { width: 32, height: 32 });
    let small: Vec<u8> = (0..16 * 16).map(|i| (i % 16 * 16) as u8).collect();
    spec.lossy_segments = vec![baseline_jpeg(&small, 16, 16, 1)];
    let err = decode(&spec.build(), &Limits::default()).unwrap_err().to_string();
    assert!(err.contains("16x16") && err.contains("32x32"), "{err}");
    // A tile coded at the image's row count instead of the tile's is tolerated (a strip-like
    // encoder); anything else is not.
    let (w, h) = (16, 12);
    let mut spec = lossy_spec(w, h, vec![100; w * h], DngStorage::LossyJpegTiles { width: 16, height: 16 });
    spec.lossy_segments = vec![baseline_jpeg(&[100u8; 16 * 12], 16, 12, 1)];
    let s = sensor(&spec.build());
    assert_eq!((s.width, s.height), (w, h));
    assert!(s.data.iter().all(|&v| v.abs_diff(100) <= 2));
    for (tw, th) in [(16, 14), (16, 18), (12, 16), (18, 16)] {
        spec.lossy_segments = vec![baseline_jpeg(&vec![100u8; tw * th], tw, th, 1)];
        assert!(matches!(decode(&spec.build(), &Limits::default()), Err(RawError::Malformed(_))), "{tw}x{th}");
    }
    // A strip must be coded at its own row count.
    let mut spec = lossy_spec(w, h, vec![100; w * h], DngStorage::LossyJpegStrips { rows: 8 });
    spec.lossy_segments = vec![baseline_jpeg(&[100u8; 16 * 8], 16, 8, 1), baseline_jpeg(&[100u8; 16 * 8], 16, 8, 1)];
    assert!(matches!(decode(&spec.build(), &Limits::default()), Err(RawError::Malformed(_))));
}

/// A tile with the wrong number of JPEG components (a colour JPEG in a CFA DNG, a gray one in a
/// 3-sample LinearRaw DNG) is rejected rather than silently converted.
#[test]
fn lossy_jpeg_component_count_mismatch_errors() {
    let (w, h) = (16, 16);
    let mut cfa = lossy_spec(w, h, vec![90; w * h], DngStorage::LossyJpegTiles { width: 16, height: 16 });
    cfa.lossy_segments = vec![baseline_jpeg(&vec![90u8; w * h * 3], w, h, 3)];
    let err = decode(&cfa.build(), &Limits::default()).unwrap_err();
    assert!(matches!(&err, RawError::Malformed(m) if m.contains("3 components")), "{err}");

    let mut rgb = lossy_spec(w, h, vec![90; w * h * 3], DngStorage::LossyJpegTiles { width: 16, height: 16 });
    rgb.samples = 3;
    rgb.lossy_segments = vec![baseline_jpeg(&vec![90u8; w * h], w, h, 1)];
    let err = decode(&rgb.build(), &Limits::default()).unwrap_err();
    assert!(matches!(&err, RawError::Malformed(m) if m.contains("1 components")), "{err}");
}

/// A SOF claiming a 65535x65535 frame inside a 16x16 tile is refused at the header, under the
/// default limits as well as tight ones: the frame is never allocated.
#[test]
fn lossy_jpeg_oversized_frame_header_errors() {
    let (w, h) = (16, 16);
    let mut bytes = lossy_spec(w, h, vec![90; w * h], DngStorage::LossyJpegTiles { width: 16, height: 16 }).build();
    let sof = bytes.windows(2).position(|m| m == [0xFF, 0xC0]).expect("SOF0");
    // FF C0 | length (2) | precision (1) | height (2) | width (2)
    bytes[sof + 5..sof + 9].copy_from_slice(&[0xFF, 0xFF, 0xFF, 0xFF]);
    for limits in [Limits::default(), Limits { max_width: 64, max_height: 64, max_pixels: 1 << 12, max_alloc: 1 << 16 }] {
        assert!(matches!(decode(&bytes, &limits), Err(RawError::Malformed(_))));
    }
    // The IFD cannot grow a tile past the limits either: a tile larger than the budget is
    // refused before its JPEG is looked at.
    let tight = Limits { max_width: 64, max_height: 64, max_pixels: 1 << 12, max_alloc: 1 << 10 };
    let big = lossy_spec(40, 40, vec![90; 1600], DngStorage::LossyJpegTiles { width: 40, height: 40 }).build();
    assert!(decode(&big, &tight).is_err());
}

/// DNG 1.4 stores lossy JPEG at 8 bits per sample; other depths and sample counts are reported
/// as unsupported, not decoded as something else.
#[test]
fn lossy_jpeg_rejects_other_depths_and_sample_counts() {
    let (w, h) = (16, 16);
    let mut spec = DngSpec::cfa(w, h, vec![4000; w * h]);
    spec.storage = DngStorage::LossyJpegTiles { width: 16, height: 16 };
    for bits in [12u16, 16] {
        spec.bits = bits;
        spec.white = (1 << bits) - 1;
        let err = decode(&spec.build(), &Limits::default()).unwrap_err();
        assert!(matches!(&err, RawError::Unsupported(m) if m.contains(&format!("{bits}-bit"))), "{err}");
    }
    // Four samples per pixel (the IFD's claim; the JPEG itself is three-component).
    let mut spec = lossy_spec(w, h, vec![90; w * h * 3], DngStorage::LossyJpegTiles { width: 16, height: 16 });
    spec.samples = 3;
    let mut bytes = spec.build();
    let samples_tag = 277u16.to_le_bytes();
    let at = bytes.windows(4).position(|e| e[..2] == samples_tag && e[2..] == 3u16.to_le_bytes()).expect("SamplesPerPixel entry");
    bytes[at + 8] = 4;
    let err = decode(&bytes, &Limits::default()).unwrap_err();
    assert!(matches!(&err, RawError::Unsupported(m) if m.contains("4 samples")), "{err}");
}
