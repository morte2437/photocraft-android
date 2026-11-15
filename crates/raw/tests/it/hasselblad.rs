//! Synthetic Hasselblad 3FR files (the X2D II 100C layout): identification,
//! exact samples, metadata, the DefaultCrop area, and the unsupported and
//! hostile variants.

use photocraft_raw::testgen::{Hasselblad3frSpec, mosaic, scene};
use photocraft_raw::*;

const RGGB: [u8; 4] = [0, 1, 1, 2];

fn spec() -> Hasselblad3frSpec {
    let (w, h) = (16, 12);
    Hasselblad3frSpec::new(w, h, mosaic(&scene(w, h), w, RGGB, 4096, 64763))
}

#[test]
fn identifies_as_fff() {
    let b = spec().build();
    assert_eq!(identify(&b), Some(RawFormat::Fff));
    assert!(is_raw(&b));
    assert!(EXTENSIONS.contains(&"3fr") && EXTENSIONS.contains(&"fff"));
}

#[test]
fn decodes_exactly() {
    let s = spec();
    let sensor = decode(&s.build(), &Limits::default()).unwrap();
    assert_eq!((sensor.width, sensor.height), (16, 12));
    assert_eq!(sensor.samples, 1);
    assert_eq!(sensor.data, s.data, "the samples are the stored strip");
    let cfa = sensor.cfa.as_ref().unwrap();
    assert_eq!((cfa.width, cfa.height), (2, 2));
    for (i, &c) in RGGB.iter().enumerate() {
        assert_eq!(cfa.color(i % 2, i / 2), c, "position {i}");
    }
    assert!(cfa.is_bayer());
    assert_eq!(sensor.format, RawFormat::Fff);
}

#[test]
fn metadata() {
    let mut s = spec();
    s.crop = Some(([2, 2], [12, 8]));
    let sensor = decode(&s.build(), &Limits::default()).unwrap();
    assert_eq!(sensor.crop, Rect::new(2, 2, 12, 8));
    assert_eq!(sensor.active, Rect::new(0, 0, 16, 12));
    assert_eq!(sensor.black, BlackLevels::uniform(4096.0));
    assert_eq!(sensor.white, [64763.0; 3]);
    let wb = sensor.camera_wb.unwrap();
    assert!((wb[0] - 1.0 / 0.375).abs() < 1e-9, "{wb:?}");
    assert!((wb[2] - 1.0 / 0.617).abs() < 1e-9, "{wb:?}");
    assert_eq!((sensor.make.as_deref(), sensor.model.as_deref()), (Some("Hasselblad"), Some("X2D II 100C")));
    assert_eq!(sensor.orientation, 1);
    assert!(sensor.warnings.is_empty(), "{:?}", sensor.warnings);
}

#[test]
fn develops() {
    let mut s = spec();
    s.crop = Some(([2, 2], [12, 8]));
    let b = s.build();
    for m in [Demosaic::Bilinear, Demosaic::Mhc, Demosaic::Ahd] {
        let d = develop(&b, &DevelopOptions { demosaic: m, ..Default::default() }).unwrap_or_else(|e| panic!("{m:?}: {e}"));
        assert_eq!((d.width, d.height), (12, 8), "{m:?}");
    }
}

#[test]
fn unknown_body_warns_about_the_assumed_phase() {
    let mut s = spec();
    s.model = "X2D 100C".into();
    let sensor = decode(&s.build(), &Limits::default()).unwrap();
    assert!(sensor.warnings.iter().any(|w| w.contains("Bayer phase")), "{:?}", sensor.warnings);
}

#[test]
fn compressed_3fr_is_reported_unsupported() {
    let mut s = spec();
    s.compression = 7;
    let e = decode(&s.build(), &Limits::default()).unwrap_err();
    assert!(matches!(&e, RawError::Unsupported(reason) if reason.contains("lossless-JPEG")), "{e:?}");
}

#[test]
fn missing_levels_fall_back() {
    let mut s = spec();
    // Clipped highlights, so the clipping point stands in for the missing WhiteLevel.
    for v in s.data.iter_mut().step_by(8) {
        *v = u16::MAX;
    }
    s.black = None;
    s.white = None;
    let sensor = decode(&s.build(), &Limits::default()).unwrap();
    assert_eq!(sensor.black, BlackLevels::uniform(0.0));
    assert!(sensor.warnings.iter().any(|w| w.contains("black level")), "{:?}", sensor.warnings);
    // clip_level backs the clipped plateau off slightly.
    assert!((sensor.white[0] - f32::from(u16::MAX) * 0.995).abs() < 1.0, "{:?}", sensor.white);
}

#[test]
fn a_crop_outside_the_plane_is_dropped() {
    let mut s = spec();
    s.crop = Some(([100, 100], [4, 4]));
    let sensor = decode(&s.build(), &Limits::default()).unwrap();
    assert_eq!(sensor.crop, Rect::new(0, 0, 16, 12));
}

#[test]
fn active_area_bounds_the_image_and_the_crop() {
    let mut s = spec();
    s.active_area = Some([1, 2, 11, 14]);
    let sensor = decode(&s.build(), &Limits::default()).unwrap();
    assert_eq!(sensor.active, Rect::new(2, 1, 12, 10));
    assert_eq!(sensor.crop, Rect::new(2, 1, 12, 10), "no default crop: the active area");
    let d = develop(&s.build(), &DevelopOptions::default()).unwrap();
    assert_eq!((d.width, d.height), (12, 10));
    // The default crop is relative to the active area, as in DNG.
    s.crop = Some(([1, 1], [8, 6]));
    let sensor = decode(&s.build(), &Limits::default()).unwrap();
    assert_eq!(sensor.crop, Rect::new(3, 2, 8, 6));
}

#[test]
fn an_active_area_outside_the_plane_is_ignored() {
    let mut s = spec();
    s.active_area = Some([100, 100, 200, 200]);
    let sensor = decode(&s.build(), &Limits::default()).unwrap();
    assert_eq!((sensor.active, sensor.crop), (Rect::new(0, 0, 16, 12), Rect::new(0, 0, 16, 12)));
}

#[test]
fn truncated_strip_is_malformed() {
    let mut b = spec().build();
    // The preview blob sits past the strip; cutting 60 bytes eats into the strip.
    b.truncate(b.len() - 60);
    assert!(matches!(decode(&b, &Limits::default()), Err(RawError::Malformed(_))));
}

#[test]
fn zero_dimensions_are_malformed() {
    let mut s = spec();
    s.width = 0;
    assert!(matches!(decode(&s.build(), &Limits::default()), Err(RawError::Malformed(_))));
}
