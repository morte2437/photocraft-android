//! A project file stores a Filter Gallery effect's values as plain numbers, bypassing the clamp that
//! `GalleryEffect::set` applies, so the reach arithmetic must survive any `f32` in them.

use photocraft_algo::{FilterParams, GalleryEffect, GalleryFilter, GalleryParamKind, Halo};

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
