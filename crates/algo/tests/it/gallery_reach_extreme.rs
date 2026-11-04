//! A project file stores a Filter Gallery effect's values as plain numbers, bypassing the clamp that
//! `GalleryEffect::set` applies, so the reach arithmetic must survive any `f32` in them.

use photocraft_algo::{FilterParams, GalleryEffect, GalleryFilter, GalleryParamKind, Halo, apply};
use photocraft_color::PixelFormat;
use photocraft_geom::Rect;
use photocraft_raster::Surface;

const EXTREME: [f32; 6] = [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, f32::MAX, f32::MIN, 3.0e9];

/// The reach in pixels; a whole-image effect (`Halo::Bounds`) has no radius to check.
fn radius(effects: Vec<GalleryEffect>) -> i32 {
    match (FilterParams::FilterGallery { effects }).halo() {
        Halo::Radius(r) => r,
        _ => 0,
    }
}

fn with_all_ranges(filter: GalleryFilter, v: f32) -> GalleryEffect {
    let mut e = GalleryEffect::new(filter);
    for p in filter.params() {
        if matches!(p.kind, GalleryParamKind::Range { .. }) {
            e.values.insert(p.key.to_string(), v);
        }
    }
    e
}

#[test]
fn gallery_reach_survives_extreme_stored_values() {
    for &filter in GalleryFilter::ALL {
        for v in EXTREME {
            let r = radius(vec![with_all_ranges(filter, v)]);
            assert!(r >= 0, "{filter:?} with every range at {v}: reach {r}");
        }
    }
}

#[test]
fn gallery_reach_of_a_stack_survives_extreme_stored_values() {
    for &filter in GalleryFilter::ALL {
        let stack = vec![with_all_ranges(filter, f32::MAX), with_all_ranges(filter, f32::MAX), with_all_ranges(filter, f32::MAX)];
        let r = radius(stack);
        assert!(r >= 0, "{filter:?} stacked three times: reach {r}");
    }
}

/// Every range value at `scale` times its declared maximum (past the range when `scale` > 1).
fn scaled_to_max(filter: GalleryFilter, scale: f32) -> GalleryEffect {
    let mut e = GalleryEffect::new(filter);
    for p in filter.params() {
        if let GalleryParamKind::Range { max, .. } = p.kind {
            e.values.insert(p.key.to_string(), max * scale);
        }
    }
    e
}

/// The tile halo comes from the clamped values, so the pixels must be drawn from them too: a stored
/// value past its range renders exactly as the clamped one, with no seam where tiles meet.
#[test]
fn gallery_pixels_use_the_same_clamped_values_as_the_reach() {
    let mut src = Surface::new(PixelFormat::RGBA8);
    src.fill_rect(Rect::new(0, 0, 160, 160), &[0.2, 0.5, 0.8, 1.0]);
    src.fill_rect(Rect::new(30, 40, 120, 100), &[0.9, 0.3, 0.1, 1.0]);
    src.fill_rect(Rect::new(70, 60, 90, 130), &[0.1, 0.9, 0.4, 1.0]);
    let area = Rect::new(0, 0, 160, 160);
    for &filter in GalleryFilter::ALL {
        let run = |e: GalleryEffect| apply(&src, &FilterParams::FilterGallery { effects: vec![e] }, area, area, None).read_region(area);
        assert_eq!(
            run(scaled_to_max(filter, 3.0)),
            run(scaled_to_max(filter, 1.0)),
            "{filter:?}: a stored value past its range must render as the clamped one"
        );
    }
}
