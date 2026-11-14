//! Synthetic Fujifilm RAF files: every sample storage, X-Trans and Bayer
//! layouts, metadata, and the unsupported variants.

use photocraft_raw::testgen::{RafPacking, RafSpec, XTRANS, mosaic_cfa, scene};
use photocraft_raw::*;

const RGGB: [u8; 4] = [0, 1, 1, 2];

/// A 30×18 X-Trans RAF with a 2-pixel masked border.
fn xtrans(packing: RafPacking, bits: u32) -> RafSpec {
    let (w, h) = (30, 18);
    let white = ((1u32 << bits) - 1) as u16;
    let mut s = RafSpec::new(w, h, mosaic_cfa(&scene(w, h), w, &XTRANS, 6, 256, white), XTRANS.to_vec());
    s.bits = bits;
    s.packing = packing;
    s.black = vec![256; 36];
    s.crop = Some([2, 2, 14, 26]);
    s
}

#[test]
fn every_storage_decodes_exactly() {
    for (packing, bits) in [(RafPacking::U16 { le: true }, 14), (RafPacking::U16 { le: false }, 16), (RafPacking::Lsb12, 12), (RafPacking::Words14, 14)] {
        let spec = xtrans(packing, bits);
        let b = spec.build();
        assert_eq!(identify(&b), Some(RawFormat::Raf));
        let s = decode(&b, &Limits::default()).unwrap_or_else(|e| panic!("{packing:?}: {e}"));
        assert_eq!((s.width, s.height), (30, 18));
        assert_eq!(s.data, spec.data, "{packing:?}");
    }
}

#[test]
fn xtrans_metadata() {
    let mut spec = xtrans(RafPacking::U16 { le: true }, 14);
    spec.exposure_bias = Some((-72, 100));
    spec.orientation = 6;
    let s = decode(&spec.build(), &Limits::default()).unwrap();
    let cfa = s.cfa.as_ref().unwrap();
    assert_eq!((cfa.width, cfa.height), (6, 6));
    for (i, &c) in XTRANS.iter().enumerate() {
        assert_eq!(cfa.color(i % 6, i / 6), c, "position {i}");
    }
    assert!(!cfa.is_bayer());
    assert_eq!(s.crop, Rect::new(2, 2, 26, 14));
    assert_eq!(s.black, BlackLevels::uniform(256.0));
    assert_eq!(s.camera_wb, Some([2.0, 1.0, 1.5]));
    assert!((s.baseline_exposure - 0.72).abs() < 1e-9);
    assert_eq!(s.orientation, 6);
    assert_eq!((s.make.as_deref(), s.model.as_deref()), (Some("FUJIFILM"), Some("X-Synthetic")));
    assert!(s.warnings.is_empty(), "{:?}", s.warnings);
}

#[test]
fn xtrans_develops() {
    let mut spec = xtrans(RafPacking::U16 { le: true }, 14);
    spec.orientation = 6;
    let b = spec.build();
    for m in [Demosaic::Bilinear, Demosaic::Mhc, Demosaic::Ahd] {
        let d = develop(&b, &DevelopOptions { demosaic: m, ..Default::default() }).unwrap();
        // Cropped to 26×14, then rotated.
        assert_eq!((d.width, d.height), (14, 26));
        assert_eq!(d.info.format, RawFormat::Raf);
        // The repeat seen from the crop origin (2, 2).
        assert_eq!(d.info.cfa.as_deref(), Some("GRBGBR/BGGRGG/RGGBGG/GBRGRB/RGGBGG/BGGRGG"));
        assert_eq!(d.info.wb_multipliers, [2.0, 1.0, 1.5]);
    }
}

#[test]
fn xtrans_white_balance_is_per_site() {
    // A neutral grey recorded through WB gains R ×2, B ×1.5: developed with the
    // as-shot balance it must come out neutral at every X-Trans site (a gain
    // applied per column pair, as for Bayer, would tint it).
    let (w, h) = (36, 24);
    let rgb: Vec<[f32; 3]> = vec![[0.4 / 2.0, 0.4, 0.4 / 1.5]; w * h];
    let spec = RafSpec { black: vec![256], ..RafSpec::new(w, h, mosaic_cfa(&rgb, w, &XTRANS, 6, 256, 16383), XTRANS.to_vec()) };
    let d = develop(&spec.build(), &DevelopOptions::default()).unwrap();
    for p in d.rgb.as_chunks::<3>().0 {
        assert!(p[0].abs_diff(p[1]) <= 2 && p[2].abs_diff(p[1]) <= 2, "{p:?}");
    }
}

#[test]
fn bayer_layouts() {
    let (w, h) = (24, 16);
    for cfa in [RGGB, [2, 1, 1, 0]] {
        for (packing, bits) in [(RafPacking::Lsb12, 12), (RafPacking::Words14, 14), (RafPacking::U16 { le: true }, 14)] {
            let white = ((1u32 << bits) - 1) as u16;
            let data = mosaic_cfa(&scene(w, h), w, &cfa, 2, 64, white);
            let spec = RafSpec { bits, packing, black: vec![64, 64, 64, 64], ..RafSpec::new(w, h, data, cfa.to_vec()) };
            let s = decode(&spec.build(), &Limits::default()).unwrap();
            assert_eq!(s.data, spec.data);
            assert_eq!(s.cfa.as_ref().unwrap().phase(0, 0), cfa, "{packing:?}");
            let d = develop_sensor(&s, &DevelopOptions::default()).unwrap();
            assert_eq!(d.info.cfa.as_deref(), Some(if cfa == RGGB { "RGGB" } else { "BGGR" }));
        }
    }
}

/// A 780×18 all-zero RAF in Fujifilm's compressed storage (see `RafPacking::Compressed`).
fn compressed(cfa: &[u8], bits: u32, lossy: bool) -> RafSpec {
    let mut s = RafSpec::new(780, 18, vec![0; 780 * 18], cfa.to_vec());
    s.bits = bits;
    s.packing = RafPacking::Compressed { lossy };
    s.black = vec![0];
    s
}

#[test]
fn compressed_lossless_and_lossy_decode() {
    for cfa in [XTRANS.as_slice(), &RGGB, &[2, 1, 1, 0]] {
        for bits in [12, 14, 16] {
            for lossy in [false, true] {
                let mut spec = compressed(cfa, bits, lossy);
                spec.crop = Some([0, 2, 18, 776]);
                let b = spec.build();
                let s = decode(&b, &Limits::default()).unwrap_or_else(|e| panic!("{cfa:?} {bits} {lossy}: {e}"));
                assert_eq!((s.width, s.height), (780, 18));
                assert_eq!(s.data, spec.data);
                assert_eq!(s.crop, Rect::new(2, 0, 776, 18));
                assert_eq!(s.camera_wb, Some([2.0, 1.0, 1.5]));
                assert_eq!(s.cfa.as_ref().map(|c| c.width), Some(if cfa.len() == 36 { 6 } else { 2 }));
                develop_sensor(&s, &DevelopOptions::default()).unwrap();
            }
        }
    }
}

#[test]
fn compressed_images_of_any_group_count_decode() {
    for h in [6, 24] {
        for lossy in [false, true] {
            let mut spec = compressed(&XTRANS, 14, lossy);
            spec.height = h;
            spec.data = vec![0; 780 * h];
            let s = decode(&spec.build(), &Limits::default()).unwrap_or_else(|e| panic!("{h} rows, lossy {lossy}: {e}"));
            assert_eq!((s.height, s.data.len()), (h, 780 * h));
        }
    }
}

#[test]
fn compressed_variants_that_cannot_be_decoded_fail_cleanly() {
    // The header's compression mode disagrees with the stream (lossy header, lossless stream).
    let mut b = compressed(&XTRANS, 14, false).build();
    b[108..112].copy_from_slice(&3u32.to_be_bytes());
    assert!(matches!(decode(&b, &Limits::default()), Err(RawError::Malformed(_))));
    // The stream assumes a fixed CFA arrangement.
    let mut other = XTRANS;
    other.swap(0, 2);
    for cfa in [other.as_slice(), &[1, 0, 2, 1]] {
        let b = compressed(cfa, 14, false).build();
        match decode(&b, &Limits::default()) {
            Err(RawError::Unsupported(m)) => assert!(m.contains("CFA"), "{m}"),
            other => panic!("expected unsupported, got {other:?}"),
        }
    }
    // A short strip without the IS signature.
    let mut b = compressed(&XTRANS, 14, false).build();
    let at = b.windows(5).position(|w| w == b"IS\x01\x10\x0e").unwrap();
    b[at..at + 2].copy_from_slice(b"XX");
    assert!(matches!(decode(&b, &Limits::default()), Err(RawError::Unsupported(_))));
    // A damaged entropy stream (sizes intact, a run of zero bits inside the first stripe that
    // overruns the residual escape) is malformed, never a panic.
    let mut b = compressed(&XTRANS, 14, false).build();
    let at = b.windows(5).position(|w| w == b"IS\x01\x10\x0e").unwrap();
    // Lossless: the 16-byte header and the two-entry size table, padded to 32 bytes.
    let stream = at + 32;
    b[stream + 64..stream + 128].fill(0);
    match decode(&b, &Limits::default()) {
        Err(RawError::Malformed(m)) => assert!(m.contains("residual"), "{m}"),
        other => panic!("expected malformed, got {other:?}"),
    }
}

#[test]
fn unsupported_variants_report_and_keep_the_preview() {
    let mut spec = xtrans(RafPacking::U16 { le: true }, 14);
    spec.cfa_tiff = false;
    let b = spec.build();
    match decode(&b, &Limits::default()) {
        Err(RawError::Unsupported(m)) => assert!(m.contains("FinePix"), "{m}"),
        other => panic!("expected unsupported, got {other:?}"),
    }
    assert!(embedded_preview(&b).is_some());
}

#[test]
fn bad_layouts_and_sizes_fail_cleanly() {
    // A Bayer FujiLayout that is not a Bayer pattern.
    let spec = RafSpec::new(8, 4, vec![100; 32], vec![0, 0, 2, 2]);
    assert!(matches!(decode(&spec.build(), &Limits::default()), Err(RawError::Unsupported(_))));
    // Dimensions beyond the limits are rejected before allocating.
    let spec = xtrans(RafPacking::U16 { le: true }, 14);
    let limits = Limits { max_pixels: 100, ..Limits::default() };
    assert!(matches!(decode(&spec.build(), &limits), Err(RawError::LimitExceeded(_))));
    // A truncated strip.
    let b = spec.build();
    assert!(matches!(decode(&b[..b.len() - 7], &Limits::default()), Err(RawError::Malformed(_))));
}
