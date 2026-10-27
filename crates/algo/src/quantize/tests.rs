use super::*;

fn gradient(w: usize, h: usize) -> Vec<[f32; 4]> {
    (0..w * h).map(|i| [(i % w) as f32 / (w - 1) as f32, (i / w) as f32 / (h - 1) as f32, 0.3, 1.0]).collect()
}

fn to8_reference(v: f32) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0).round() as u8
}

#[test]
fn to8_matches_round_on_samples_and_edges() {
    // Every half-way point and its neighbours, plus a dense sweep of the unit range.
    for k in 0..=256u32 {
        let mid = (k as f32 + 0.5) / 255.0;
        for d in -3i32..=3 {
            let v = f32::from_bits((mid.to_bits() as i32 + d) as u32);
            assert_eq!(to8(v), to8_reference(v), "{v:e}");
        }
    }
    for i in 0..200_000u32 {
        let v = f32::from_bits(i * 20_000);
        assert_eq!(to8(v), to8_reference(v), "{v:e}");
    }
    for v in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, -0.0, 0.0, 1.0, -1.0, 2.0, 1e30, -1e30, f32::MIN_POSITIVE, 1e-45] {
        assert_eq!(to8(v), to8_reference(v), "{v:e}");
    }
}

#[test]
#[cfg(not(target_arch = "wasm32"))]
#[ignore = "checks all 2^32 floats, a few seconds in release; run with --release --ignored"]
fn to8_matches_round_for_every_float() {
    use rayon::prelude::*;
    let bad = (0..=u32::MAX).into_par_iter().filter(|&b| to8(f32::from_bits(b)) != to8_reference(f32::from_bits(b))).count();
    assert_eq!(bad, 0);
}

/// A repeatable 96x64 image: a gradient with up to 31 levels of integer-hash noise and a
/// few see-through pixels.
fn golden_image() -> Vec<[f32; 4]> {
    let (w, h) = (96usize, 64usize);
    (0..w * h)
        .map(|i| {
            let (x, y) = ((i % w) as f32, (i / w) as f32);
            let noise = |salt: u32| ((((i as u32).wrapping_mul(2_654_435_761).wrapping_add(salt.wrapping_mul(40_503))) >> 13) & 31) as f32 / 255.0;
            let alpha = if i % 17 == 0 { 0.25 } else { 1.0 };
            [x / (w - 1) as f32 * 0.8 + noise(1), y / (h - 1) as f32 * 0.7 + noise(2), ((x + y) / (w + h) as f32) * 0.6 + noise(3), alpha]
        })
        .collect()
}

/// Palettes written down from the version before the speed-up (hash-map histogram, every box
/// rescanned each split, comparison sort, serial k-means). Any change to how the palettes
/// are built has to give these exact colours.
#[test]
fn palettes_match_the_recorded_originals() {
    let px = golden_image();
    #[rustfmt::skip]
    let cases: Vec<(PaletteKind, usize, Forced, &[Rgb8])> = vec![
        (
            PaletteKind::Adaptive,
            16,
            Forced::None,
            &[
            [40, 173, 80], [196, 175, 150], [92, 173, 103], [92, 81, 71], [195, 83, 118], [39, 82, 49],
            [142, 82, 95], [195, 37, 101], [143, 36, 79], [40, 128, 64], [196, 129, 133], [92, 127, 87],
            [91, 36, 56], [38, 35, 33], [144, 128, 111], [144, 174, 127],
            ],
        ),
        (
            PaletteKind::Perceptual,
            16,
            Forced::None,
            &[
            [104, 111, 87], [51, 129, 68], [48, 178, 79], [110, 74, 77], [165, 46, 93], [205, 50, 107],
            [200, 154, 143], [44, 54, 42], [190, 101, 121], [61, 32, 44], [155, 115, 111], [48, 93, 56],
            [119, 42, 71], [84, 172, 98], [126, 165, 115], [172, 168, 135],
            ],
        ),
        (
            PaletteKind::Selective,
            24,
            Forced::BlackWhite,
            &[
            [0, 0, 0], [255, 255, 255], [102, 107, 85], [65, 69, 55], [87, 133, 89], [109, 73, 76],
            [164, 42, 92], [204, 51, 102], [199, 153, 142], [54, 170, 84], [199, 107, 126], [65, 33, 45],
            [153, 102, 102], [187, 180, 145], [40, 106, 54], [100, 176, 107], [138, 174, 123], [50, 188, 80],
            [32, 37, 33], [47, 139, 69], [144, 143, 116], [41, 80, 49], [108, 33, 64], [151, 70, 94],
            ],
        ),
        (
            PaletteKind::Adaptive,
            8,
            Forced::Primaries,
            &[
            [0, 0, 0], [255, 255, 255], [255, 0, 0], [0, 255, 0], [0, 0, 255], [0, 255, 255],
            [255, 0, 255], [255, 255, 0],
            ],
        ),
        (
            PaletteKind::Perceptual,
            12,
            Forced::BlackWhite,
            &[
            [0, 0, 0], [255, 255, 255], [104, 106, 85], [137, 43, 79], [148, 170, 126], [181, 144, 132],
            [66, 172, 87], [88, 49, 60], [57, 118, 67], [42, 54, 41], [183, 101, 118], [196, 50, 105],
            ],
        ),
    ];
    for (kind, colors, forced, expected) in cases {
        assert_eq!(build_palette(&px, kind, colors, forced).unwrap(), expected, "{kind:?} {colors} {forced:?}");
    }
}

/// The pre-speed-up median cut and k-means, kept as the oracle for the faster ones.
fn median_cut_reference(pts: &[([f32; 3], u32)], n: usize) -> Vec<[f32; 3]> {
    if pts.is_empty() || n == 0 {
        return Vec::new();
    }
    let mut boxes: Vec<Vec<([f32; 3], u32)>> = vec![pts.to_vec()];
    while boxes.len() < n {
        let (bi, axis, score) = boxes
            .iter()
            .enumerate()
            .filter(|(_, b)| b.len() > 1)
            .map(|(i, b)| {
                let mut best = (0usize, -1.0f32);
                for a in 0..3 {
                    let (lo, hi) = b.iter().fold((f32::MAX, f32::MIN), |(lo, hi), p| (lo.min(p.0[a]), hi.max(p.0[a])));
                    if hi - lo > best.1 {
                        best = (a, hi - lo);
                    }
                }
                let w: f32 = b.iter().map(|p| p.1 as f32).sum();
                (i, best.0, best.1 * w.sqrt())
            })
            .fold((usize::MAX, 0, -1.0f32), |acc, x| if x.2 > acc.2 { x } else { acc });
        if bi == usize::MAX || score <= 0.0 {
            break;
        }
        let mut b = boxes.swap_remove(bi);
        b.sort_by(|p, q| p.0[axis].total_cmp(&q.0[axis]));
        let total: u64 = b.iter().map(|p| u64::from(p.1)).sum();
        let mut acc = 0u64;
        let mut cut = 1;
        for (i, p) in b.iter().enumerate() {
            acc += u64::from(p.1);
            if acc * 2 >= total {
                cut = (i + 1).clamp(1, b.len() - 1);
                break;
            }
        }
        let rest = b.split_off(cut);
        boxes.push(b);
        boxes.push(rest);
    }
    boxes
        .iter()
        .map(|b| {
            let w: f64 = b.iter().map(|p| f64::from(p.1)).sum();
            std::array::from_fn(|a| (b.iter().map(|p| f64::from(p.0[a]) * f64::from(p.1)).sum::<f64>() / w) as f32)
        })
        .collect()
}

fn kmeans_reference(pts: &[([f32; 3], u32)], centers: &mut [[f32; 3]], iters: usize) {
    for _ in 0..iters {
        let lookup = Nearest::new(centers.iter().copied());
        let mut sum = vec![[0.0f64; 3]; centers.len()];
        let mut cnt = vec![0.0f64; centers.len()];
        for (p, w) in pts {
            let i = lookup.find(*p);
            for a in 0..3 {
                sum[i][a] += f64::from(p[a]) * f64::from(*w);
            }
            cnt[i] += f64::from(*w);
        }
        for (i, c) in centers.iter_mut().enumerate() {
            if cnt[i] > 0.0 {
                *c = std::array::from_fn(|a| (sum[i][a] / cnt[i]) as f32);
            }
        }
    }
}

/// Points with many repeated coordinates (ties), negative values, -0.0 and weights up to
/// a few thousand, so the stable-sort and float-order rules are all exercised.
fn tie_heavy_points(n: usize, seed: u64) -> Vec<([f32; 3], u32)> {
    let mut s = seed;
    let mut next = move || {
        s = s.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
        (s >> 33) as u32
    };
    (0..n)
        .map(|_| {
            let mut c = |range: u32| match next() % 40 {
                0 => -0.0,
                1 => 0.0,
                _ => (next() % range) as f32 * 0.5 - (range / 4) as f32,
            };
            ([c(24), c(300), c(150)], 1 + next() % 3000)
        })
        .collect()
}

#[test]
fn faster_median_cut_and_kmeans_equal_the_originals_bit_for_bit() {
    // 40 000 points cross the 256-point radix threshold on the first splits.
    for (n, boxes, seed) in [(40_000, 256, 1), (40_000, 37, 2), (3_000, 256, 3), (200, 64, 4), (2, 8, 5)] {
        let pts = tie_heavy_points(n, seed);
        let new = median_cut(&pts, boxes);
        let old = median_cut_reference(&pts, boxes);
        assert_eq!(new.len(), old.len());
        for (a, b) in new.iter().zip(&old) {
            assert_eq!(a.map(f32::to_bits), b.map(f32::to_bits), "median cut, n={n} boxes={boxes}");
        }
        let (mut a, mut b) = (new, old);
        kmeans(&pts, &mut a, 3);
        kmeans_reference(&pts, &mut b, 3);
        for (x, y) in a.iter().zip(&b) {
            assert_eq!(x.map(f32::to_bits), y.map(f32::to_bits), "kmeans, n={n} boxes={boxes}");
        }
    }
}

#[test]
fn radix_sort_is_the_stable_total_cmp_sort() {
    for axis in 0..3 {
        let mut pts = tie_heavy_points(5_000, 9 + axis as u64);
        // The weight field doubles as an id: equal keys must keep their input order.
        for (i, p) in pts.iter_mut().enumerate() {
            p.1 = i as u32;
        }
        pts[10].0[axis] = f32::NAN;
        pts[11].0[axis] = -f32::NAN;
        pts[12].0[axis] = f32::INFINITY;
        pts[13].0[axis] = f32::NEG_INFINITY;
        let mut expected = pts.clone();
        expected.sort_by(|p, q| p.0[axis].total_cmp(&q.0[axis]));
        radix_sort_along(&mut pts, axis);
        let bits = |v: &[([f32; 3], u32)]| v.iter().map(|p| (p.0.map(f32::to_bits), p.1)).collect::<Vec<_>>();
        assert_eq!(bits(&pts), bits(&expected), "axis {axis}");
    }
}

#[test]
fn histogram_counts_opaque_pixels_sorted() {
    let px = [[1.0, 0.0, 0.0, 1.0], [0.0, 0.0, 1.0, 1.0], [1.0, 0.0, 0.0, 0.5], [1.0, 0.0, 0.0, 0.49], [0.0, 0.0, 1.0, 1.0], [0.0, 0.0, 0.0, 1.0]];
    assert_eq!(histogram(&px), vec![([0, 0, 0], 1), ([0, 0, 255], 2), ([255, 0, 0], 2)]);
    assert!(histogram(&[]).is_empty());
}

#[test]
fn fixed_palettes_have_expected_sizes() {
    assert_eq!(web_palette().len(), 216);
    assert_eq!(mac_palette().len(), 256);
    assert_eq!(windows_palette().len(), 256);
    assert_eq!(uniform_palette(27).len(), 27);
    let mut m = mac_palette();
    m.sort_unstable();
    m.dedup();
    assert_eq!(m.len(), 256, "mac palette entries are unique");
}

#[test]
fn exact_palette_and_failure() {
    let px = vec![[1.0, 0.0, 0.0, 1.0], [0.0, 0.0, 1.0, 1.0], [1.0, 0.0, 0.0, 1.0]];
    assert_eq!(build_palette(&px, PaletteKind::Exact, 256, Forced::None).unwrap().len(), 2);
    assert!(build_palette(&gradient(64, 64), PaletteKind::Exact, 256, Forced::None).is_err());
}

#[test]
fn adaptive_and_perceptual_respect_count_and_forced() {
    let px = gradient(40, 40);
    for kind in [PaletteKind::Adaptive, PaletteKind::Perceptual, PaletteKind::Selective] {
        let pal = build_palette(&px, kind, 16, Forced::BlackWhite).unwrap();
        assert!(pal.len() <= 16 && pal.len() >= 8, "{kind:?} {}", pal.len());
        assert_eq!(&pal[..2], &[[0, 0, 0], [255, 255, 255]]);
    }
}

#[test]
fn forced_colours_filling_the_count_do_not_panic() {
    // 2 colours + Black and White (or 8 + Primaries) leaves no room for
    // computed entries; the median cut used to return no centres and
    // k-means indexed into the empty list.
    let px = vec![[1.0, 1.0, 1.0, 1.0]; 16];
    for kind in [PaletteKind::Adaptive, PaletteKind::Perceptual, PaletteKind::Selective] {
        let pal = build_palette(&px, kind, 2, Forced::BlackWhite).unwrap();
        assert_eq!(pal, vec![[0, 0, 0], [255, 255, 255]], "{kind:?}");
        let pal = build_palette(&gradient(8, 8), kind, 8, Forced::Primaries).unwrap();
        assert_eq!(pal.len(), 8, "{kind:?}");
    }
}

#[test]
#[ignore = "24 MP release timing comparison; run with --release --ignored --nocapture"]
fn indexed_diffusion_24mp_release_comparison() {
    let source = gradient(6000, 4000);
    let pal = web_palette();
    for run in 0..4 {
        let mut linear = source.clone();
        let mut accelerated = source.clone();
        let mut before = None;
        let mut after = None;
        // Reverse the order on alternating rounds; exclude the first round as warmup.
        for old in if run % 2 == 0 { [true, false] } else { [false, true] } {
            let start = std::time::Instant::now();
            if old {
                let indices = quantize_with(&mut linear, 6000, &pal, Dither::Diffusion, 0.75, None, |c| linear_rgb_nearest(&pal, c));
                before = Some((start.elapsed().as_secs_f64() * 1000.0, indices));
            } else {
                let indices = quantize(&mut accelerated, 6000, &pal, Dither::Diffusion, 0.75, None);
                after = Some((start.elapsed().as_secs_f64() * 1000.0, indices));
            }
        }
        let (before_ms, before_indices) = before.unwrap();
        let (after_ms, after_indices) = after.unwrap();
        assert_eq!(before_indices.iter().zip(&after_indices).position(|(a, b)| a != b), None, "first differing index");
        assert_eq!(linear.iter().zip(&accelerated).position(|(a, b)| a != b), None, "first differing pixel");
        println!(
            "{}",
            serde_json::json!({"run":run,"warmup":run==0,"pixels":24_000_000,"palette_entries":216,"linear_ms":before_ms,"accelerated_ms":after_ms})
        );
    }
}

fn linear_rgb_nearest(pal: &[Rgb8], c: [f32; 3]) -> usize {
    let mut best = (f32::MAX, 0);
    for (i, e) in pal.iter().enumerate() {
        let d: f32 = (0..3).map(|k| (c[k] * 255.0 - f32::from(e[k])).powi(2)).sum();
        if d < best.0 {
            best = (d, i);
        }
    }
    best.1
}

#[test]
fn all_dithers_match_the_original_linear_palette_search_exactly() {
    let mut source = gradient(93, 61);
    for (i, p) in source.iter_mut().enumerate() {
        p[2] = (i % 71) as f32 / 70.0;
        p[3] = [0.0, 0.2, 0.7, 1.0][i % 4];
    }
    for count in [8, 16, 32, 64, 128, 256] {
        let pal = build_palette(&source, PaletteKind::Selective, count, Forced::BlackWhite).unwrap();
        for dither in [Dither::None, Dither::Diffusion, Dither::Pattern, Dither::Noise] {
            for transparent in [None, Some(1)] {
                let mut actual = source.clone();
                let mut expected = source.clone();
                let indices = quantize(&mut actual, 93, &pal, dither, 0.75, transparent);
                let old_indices = quantize_with(&mut expected, 93, &pal, dither, 0.75, transparent, |c| linear_rgb_nearest(&pal, c));
                assert_eq!(indices, old_indices, "{count} {dither:?} {transparent:?}");
                assert_eq!(actual, expected, "{count} {dither:?} {transparent:?}");
            }
        }
    }
}

#[test]
fn quantize_maps_to_palette_with_each_dither() {
    let pal = vec![[0, 0, 0], [255, 255, 255]];
    for d in [Dither::None, Dither::Diffusion, Dither::Pattern, Dither::Noise] {
        let mut px: Vec<[f32; 4]> = (0..64)
            .map(|i| [i as f32 / 63.0; 4])
            .map(|mut p| {
                p[3] = 1.0;
                p
            })
            .collect();
        let idx = quantize(&mut px, 8, &pal, d, 1.0, None);
        assert!(px.iter().all(|p| p[0] == 0.0 || p[0] == 1.0), "{d:?}");
        assert_eq!(idx.len(), 64);
        // Mean brightness is preserved by dithering (not by plain thresholding).
        if d == Dither::Diffusion {
            let mean = px.iter().map(|p| p[0]).sum::<f32>() / 64.0;
            assert!((mean - 0.5).abs() < 0.08, "{mean}");
        }
    }
    let mut px = vec![[1.0, 1.0, 1.0, 0.2]];
    let idx = quantize(&mut px, 1, &pal, Dither::None, 1.0, Some(1));
    assert_eq!((idx[0], px[0][3]), (1, 0.0));
}

#[test]
fn bitmap_methods_are_binary_and_track_tone() {
    for m in [
        BitmapMethod::Threshold,
        BitmapMethod::Pattern,
        BitmapMethod::Diffusion,
        BitmapMethod::Halftone { cell: 6.0, angle: 45.0, shape: HalftoneShape::Round },
    ] {
        let mut g = vec![0.25f32; 32 * 32];
        to_bitmap(&mut g, 32, m);
        assert!(g.iter().all(|v| *v == 0.0 || *v == 1.0));
        let mean = g.iter().sum::<f32>() / g.len() as f32;
        if m != BitmapMethod::Threshold {
            assert!((mean - 0.25).abs() < 0.15, "{m:?} {mean}");
        } else {
            assert_eq!(mean, 0.0);
        }
    }
}

/// The first 8-bit gray level (of a 0..=255 ramp) that `t` turns white.
fn first_white(t: f32) -> u32 {
    (0..=255).find(|v| *v as f32 / 255.0 >= t).unwrap_or(256)
}

#[test]
fn custom_pattern_thresholds_match_photoshop() {
    // Photoshop (Image › Mode › Bitmap, Custom Pattern) on a 0..255 ramp: the first white
    // level per pattern pixel, measured against defined patterns.
    let l = |v: &[u8]| v.iter().map(|v| f32::from(*v) / 255.0).collect::<Vec<_>>();
    let ramp7 = custom_pattern_thresholds(&l(&[0, 40, 80, 120, 160, 200, 240]));
    assert_eq!(ramp7.iter().map(|t| first_white(*t)).collect::<Vec<_>>(), [237, 201, 164, 128, 92, 55, 19]);
    let five = custom_pattern_thresholds(&l(&[10, 200, 90, 250, 150]));
    assert_eq!(five.iter().map(|t| first_white(*t)).collect::<Vec<_>>(), [230, 77, 179, 26, 128]);
    // Equal pixels share their rank: 100, 100 both sit at 50%.
    let ties = custom_pattern_thresholds(&l(&[100, 100, 200, 50]));
    assert_eq!(ties.iter().map(|t| first_white(*t)).collect::<Vec<_>>(), [128, 128, 32, 224]);
    assert!(custom_pattern_thresholds(&[]).is_empty());
    assert_eq!(custom_pattern_thresholds(&[f32::NAN, 0.3]).len(), 2);
}

#[test]
fn custom_pattern_tiles_from_the_top_left() {
    // A 2x1 map: even columns need 75%, odd ones 25%.
    let map = [0.75, 0.25];
    let mut g = vec![0.5; 6];
    to_bitmap_custom(&mut g, 3, &map, 2, 1);
    assert_eq!(g, [0.0, 1.0, 0.0, 0.0, 1.0, 0.0]);
    // A map that doesn't match its size, or an empty image width, is a 50% threshold.
    let mut g = vec![0.4, 0.6];
    to_bitmap_custom(&mut g, 2, &map, 3, 1);
    assert_eq!(g, [0.0, 1.0]);
    let mut g = vec![0.6];
    to_bitmap_custom(&mut g, 0, &map, 2, 1);
    assert_eq!(g, [1.0]);
}
