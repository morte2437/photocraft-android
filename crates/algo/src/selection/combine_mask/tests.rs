use super::*;
use crate::selection::{Region, combine_region, mask_from_surface, surface_from_masks};
use photocraft_color::{ColorMode, SampleType};

const MODES: [SelectionMode; 4] = [SelectionMode::Replace, SelectionMode::Add, SelectionMode::Subtract, SelectionMode::Intersect];

// The pre-refactor float formulas, including quantization before the binary operation.
fn reference(old: Option<&Surface>, mask: &[f32], area: Rect, mode: SelectionMode) -> Option<Surface> {
    if area.is_empty() {
        return None;
    }
    if mode == SelectionMode::Replace {
        let s = surface_from_masks(area, mask, None, |_, v| v);
        return (s.tile_count() > 0).then_some(s);
    }
    let prev = mask_from_surface(old, area);
    let s = surface_from_masks(area, mask, Some(&prev), |p, v| {
        let q = f32::from(coverage8(v)) / 255.0;
        match mode {
            SelectionMode::Replace => v,
            SelectionMode::Add => p.max(q),
            SelectionMode::Subtract => (p - q).max(0.0),
            SelectionMode::Intersect => p.min(q),
        }
    });
    (s.tile_count() > 0).then_some(s)
}

fn reference_band(old: Option<&Surface>, mask: &[f32], area: Rect, mode: SelectionMode) -> Option<Surface> {
    let inside = reference(old, mask, area, mode);
    let Some(old) = old.filter(|_| matches!(mode, SelectionMode::Add | SelectionMode::Subtract)) else { return inside };
    if area.is_empty() {
        return Some(old.clone());
    }
    let values = inside.map_or_else(|| vec![0.0; area.width() as usize * area.height() as usize], |s| s.read_region(area));
    let mut out = old.clone();
    out.write_region(area, &values);
    out.prune();
    (out.tile_count() > 0).then_some(out)
}

#[test]
fn every_byte_pair_matches_float_combining() {
    let area = Rect::new(0, 0, 256, 256);
    let prev: Vec<f32> = (0..65536).map(|i| (i / 256) as f32 / 255.0).collect();
    let mask: Vec<f32> = (0..65536).map(|i| (i % 256) as f32 / 255.0).collect();
    let old = crate::selection::mask_to_surface(&prev, area);
    for mode in MODES {
        assert_eq!(combine(Some(&old), &mask, area, mode), reference(Some(&old), &mask, area, mode), "{mode:?}");
    }
}

#[test]
fn cropped_bands_match_dense_writeback_with_defaults_and_short_masks() {
    let values = [f32::NAN, f32::NEG_INFINITY, -0.2, 0.0, 0.49 / 255.0, 0.5 / 255.0, 0.51 / 255.0, 0.5, 1.0, 1.2, f32::INFINITY];
    for area in [Rect::new(-259, -2, 259, 3), Rect::new(255, 255, 258, 259), Rect::new(0, 0, 512, 256), Rect::EMPTY] {
        let n = area.width() as usize * area.height() as usize;
        let mask: Vec<f32> = (0..n).map(|i| values[i % values.len()]).collect();
        for default in [0.0, 0.5, 1.0] {
            let mut old = Surface::with_default(PixelFormat::GRAY8, &[default]);
            old.fill_rect(Rect::new(-300, -20, 300, 270), &[0.75]);
            old.write_pixel(1024, 1024, &[1.0]);
            old.write_pixel(-257, 0, &[0.0]);
            let snapshot = old.clone();
            for prev in [None, Some(&old)] {
                for len in [0, n / 2, n] {
                    for mode in MODES {
                        let mask = &mask[..len];
                        assert_eq!(combine(prev, mask, area, mode), reference(prev, mask, area, mode), "{area:?} {mode:?} {default} {len}");
                        assert_eq!(combine_band(prev, mask, area, mode), reference_band(prev, mask, area, mode), "band {area:?} {mode:?} {default} {len}");
                    }
                }
            }
            assert_eq!(old, snapshot, "combining must leave undo snapshots unchanged");
        }
    }
}

#[test]
fn unusual_selection_formats_keep_their_float_behavior() {
    let area = Rect::new(-2, -1, 3, 2);
    for sample in SampleType::ALL {
        for color in [ColorMode::Grayscale, ColorMode::Rgb] {
            let mut old = Surface::with_default(PixelFormat::new(color, sample, false), &vec![0.123; if color == ColorMode::Rgb { 3 } else { 1 }]);
            old.write_pixel(0, 0, &vec![0.754; old.channels()]);
            for mode in MODES {
                let mask = [0.5; 15];
                assert_eq!(combine(Some(&old), &mask, area, mode), reference(Some(&old), &mask, area, mode));
                if color == ColorMode::Grayscale {
                    assert_eq!(combine_band(Some(&old), &mask, area, mode), reference_band(Some(&old), &mask, area, mode));
                } else {
                    let out = combine_band(Some(&old), &mask, area, mode).unwrap();
                    let inside = reference(Some(&old), &mask, area, mode).unwrap();
                    let expected = inside.pixel(0, 0)[0];
                    let encoded = Surface::with_default(out.format(), &vec![expected; out.channels()]);
                    assert_eq!(out.pixel(0, 0), encoded.pixel(0, 0));
                    if matches!(mode, SelectionMode::Add | SelectionMode::Subtract) {
                        assert_eq!(out.pixel(20, 20), old.pixel(20, 20));
                    }
                }
            }
        }
    }
}

#[test]
fn region_bytes_match_float_bands() {
    let bbox = Rect::new(-257, 255, 258, 258);
    let mask: Vec<u8> = (0..bbox.width() * bbox.height()).map(|i| (i % 256) as u8).collect();
    let floats: Vec<f32> = mask.iter().map(|b| f32::from(*b) / 255.0).collect();
    let region = Region { bbox, mask };
    let mut old = Surface::new(PixelFormat::GRAY8);
    old.fill_rect(Rect::new(-300, 250, 300, 270), &[0.75]);
    for prev in [None, Some(&old)] {
        for mode in MODES {
            assert_eq!(combine_region(prev, Some(&region), mode), reference_band(prev, &floats, bbox, mode));
            assert_eq!(combine_region(prev, None, mode), reference_band(prev, &[], Rect::EMPTY, mode));
        }
    }
}

#[test]
fn tiles_stay_shared_or_are_pruned() {
    let area = Rect::new(0, 0, 768, 256);
    let mut old = Surface::new(PixelFormat::GRAY8);
    old.fill_rect(area, &[1.0]);
    let tc = TileCoord::new(0, 0);
    let unchanged = combine(Some(&old), &[], area, SelectionMode::Add).unwrap();
    assert!(Arc::ptr_eq(unchanged.tile(tc).unwrap(), old.tile(tc).unwrap()));
    let band = Rect::new(257, 1, 260, 3);
    let changed = combine_band(Some(&old), &[1.0; 6], band, SelectionMode::Subtract).unwrap();
    assert!(Arc::ptr_eq(changed.tile(tc).unwrap(), old.tile(tc).unwrap()));
    assert_eq!(changed.pixel(256, 1), vec![1.0]);
    assert_eq!(changed.pixel(257, 1), vec![0.0]);
    assert_eq!(changed.pixel(260, 1), vec![1.0]);
    let full = combine(None, &vec![1.0; 768 * 256], area, SelectionMode::Replace).unwrap();
    assert!(Arc::ptr_eq(full.tile(tc).unwrap(), full.tile(TileCoord::new(1, 0)).unwrap()));
    assert!(combine_band(Some(&old), &vec![1.0; 768 * 256], area, SelectionMode::Subtract).is_none());
    assert!(combine(None, &[], area, SelectionMode::Replace).is_none());
}

/// Run only in release on CI: compares the former implementation, including dense reads and
/// band writeback, against the byte path with identical 36 MP inputs and exact output checks.
#[test]
#[ignore = "36 MP release performance comparison; run on GitHub Actions"]
fn gray8_combining_release_comparison() {
    use std::{hint::black_box, time::Instant};
    let area = Rect::new(0, 0, 6000, 6000);
    let mut old = Surface::new(PixelFormat::GRAY8);
    old.fill_rect(Rect::new(100, 100, 5900, 5900), &[0.75]);
    println!("36 MP; median of three release runs including allocation; exact output checks; previous dense mask alone = 144000000 bytes");
    for (case, size) in [("sparse", 50), ("dense", 4000), ("full", 6000)] {
        let mut mask = vec![0.0; 6000 * 6000];
        let start = (6000 - size) / 2;
        for row in mask.as_chunks_mut::<6000>().0.iter_mut().skip(start).take(size) {
            row[start..start + size].fill(0.5);
        }
        for mode in MODES {
            for band in [false, true] {
                let baseline = if band { reference_band } else { reference };
                let direct = if band { combine_band } else { combine };
                assert_eq!(direct(Some(&old), &mask, area, mode), baseline(Some(&old), &mask, area, mode));
                let mut medians = [0.0; 2];
                for (median, op) in medians.iter_mut().zip([baseline, direct]) {
                    let mut runs = [0.0; 3];
                    for ms in &mut runs {
                        let start = Instant::now();
                        let out = black_box(op(Some(black_box(&old)), black_box(&mask), area, mode));
                        *ms = start.elapsed().as_secs_f64() * 1000.0;
                        drop(out);
                    }
                    runs.sort_by(f64::total_cmp);
                    *median = runs[1];
                }
                println!("{case} {mode:?} band={band}: old={:.2}ms new={:.2}ms speedup={:.2}x exact=true", medians[0], medians[1], medians[0] / medians[1]);
            }
        }
    }
}
