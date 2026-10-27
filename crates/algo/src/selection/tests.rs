use super::*;

fn img(w: usize, h: usize, f: impl Fn(usize, usize) -> [f32; 4]) -> Vec<[f32; 4]> {
    (0..h).flat_map(|y| (0..w).map(move |x| (x, y))).map(|(x, y)| f(x, y)).collect()
}

#[test]
fn wand_contiguous_vs_global() {
    // Two separate red squares on white.
    let px = img(20, 10, |x, _| if (2..6).contains(&x) || (12..16).contains(&x) { [1.0, 0.0, 0.0, 1.0] } else { [1.0; 4] });
    let a = Rect::new(0, 0, 20, 10);
    let c = magic_wand(&px, a, (3, 3), 10.0, true, false);
    assert_eq!(c.iter().filter(|v| **v > 0.0).count(), 40);
    let g = magic_wand(&px, a, (3, 3), 10.0, false, false);
    assert_eq!(g.iter().filter(|v| **v > 0.0).count(), 80);
    let aa = magic_wand(&px, a, (3, 3), 10.0, true, true);
    assert!(aa[3 * 20 + 6] > 0.0 && aa[3 * 20 + 6] < 1.0);
    assert!(magic_wand(&px, a, (50, 3), 10.0, true, false).iter().all(|v| *v == 0.0));
}

#[test]
fn wand_tolerance() {
    let px = img(10, 1, |x, _| [x as f32 * 10.0 / 255.0, 0.0, 0.0, 1.0]);
    let m = magic_wand(&px, Rect::new(0, 0, 10, 1), (0, 0), 25.0, true, false);
    assert_eq!(m.iter().filter(|v| **v > 0.0).count(), 3);
}

#[test]
fn wand_ignores_hidden_rgb_only_when_both_pixels_are_fully_transparent() {
    let area = Rect::new(0, 0, 6, 1);
    let pixels = [[255, 0, 0, 0], [0, 255, 0, 0], [255, 0, 0, 128], [253, 0, 0, 128], [0, 0, 255, 0], [255, 0, 0, 255]];
    let floats: Vec<_> = pixels.iter().map(|p| p.map(|v| f32::from(v) / 255.0)).collect();
    for (seed, tolerance, contiguous, expected) in [
        ((0, 0), 0.0, true, [1.0, 1.0, 0.0, 0.0, 0.0, 0.0]),
        ((0, 0), 0.0, false, [1.0, 1.0, 0.0, 0.0, 1.0, 0.0]),
        ((4, 0), 32.0, false, [1.0, 1.0, 0.0, 0.0, 1.0, 0.0]),
        ((2, 0), 0.0, false, [0.0, 0.0, 1.0, 0.0, 0.0, 0.0]),
        ((2, 0), 2.0, true, [0.0, 0.0, 1.0, 1.0, 0.0, 0.0]),
        ((5, 0), 32.0, false, [0.0, 0.0, 0.0, 0.0, 0.0, 1.0]),
    ] {
        assert_eq!(magic_wand(&floats, area, seed, tolerance, contiguous, false), expected);
        let region = wand_region(&pixels, area, seed, tolerance, contiguous, false).unwrap();
        for (x, value) in expected.into_iter().enumerate() {
            assert_eq!(region.at(x as i32, 0), value, "seed={seed:?}, tolerance={tolerance}, x={x}");
        }
    }
    // Hidden RGB cannot introduce extra edges into the anti-aliased selection either.
    let plain: Vec<_> = pixels.iter().map(|p| if p[3] == 0 { [0; 4] } else { *p }).collect();
    for contiguous in [false, true] {
        assert_eq!(wand_region(&pixels, area, (0, 0), 0.0, contiguous, true), wand_region(&plain, area, (0, 0), 0.0, contiguous, true));
    }
}

#[test]
fn skin_tones_selects_skin_and_not_cream_or_white() {
    let px = |rgb: [u8; 3]| [rgb[0], rgb[1], rgb[2]].map(|v| f32::from(v) / 255.0);
    let rgba = |rgb: [u8; 3]| {
        let [r, g, b] = px(rgb);
        [r, g, b, 1.0]
    };
    // Skin in light and shadow, then a cream shirt, a near-white wall, blue denim and black.
    let skin = [[183, 119, 81], [177, 132, 93], [180, 121, 84], [120, 76, 52], [173, 107, 75], [255, 219, 172], [194, 140, 104], [141, 85, 36], [92, 51, 23]]
        .map(rgba);
    let m = skin_tones(&skin, 40.0);
    assert!(m.iter().all(|&v| v > 0.95), "skin {m:?}");
    let other = [[231, 225, 202], [252, 253, 247], [40, 60, 110], [5, 5, 5], [128, 128, 128], [220, 40, 40], [200, 160, 40]].map(rgba);
    let m = skin_tones(&other, 40.0);
    assert!(m.iter().all(|&v| v < 0.05), "not skin {m:?}");
    // Fuzziness widens the region; transparent pixels are never selected.
    let pale = [rgba([230, 215, 200])];
    assert!(skin_tones(&pale, 200.0)[0] > skin_tones(&pale, 0.0)[0]);
    assert_eq!(skin_tones(&[[0.7, 0.45, 0.3, 0.0]], 40.0), vec![0.0]);
}

#[test]
fn color_range_falloff() {
    let px = vec![[1.0, 0.0, 0.0, 1.0], [0.9, 0.0, 0.0, 1.0], [0.0, 0.0, 1.0, 1.0], [1.0, 0.0, 0.0, 0.0]];
    let m = color_range(&px, [1.0, 0.0, 0.0], 40.0);
    assert_eq!(m[0], 1.0);
    assert!(m[1] > 0.3 && m[1] < 1.0);
    assert_eq!(m[2], 0.0);
    assert_eq!(m[3], 0.0);
}

#[test]
fn color_range_several_samples_and_localized() {
    let red = [1.0, 0.0, 0.0, 1.0];
    let blue = [0.0, 0.0, 1.0, 1.0];
    let green = [0.0, 1.0, 0.0, 1.0];
    // 4 × 1: red, blue, green, red.
    let px = vec![red, blue, green, red];
    let s = |c: [f32; 4], at: Option<(f32, f32)>| RangeSample { color: [c[0], c[1], c[2]], at };
    let m = color_range_samples(&px, 4, &[s(red, None), s(blue, None)], 10.0, None);
    assert_eq!(m, vec![1.0, 1.0, 0.0, 1.0]);
    // One sample is exactly the single-colour Color Range.
    assert_eq!(color_range_samples(&px, 4, &[s(red, None)], 40.0, None), color_range(&px, [1.0, 0.0, 0.0], 40.0));
    // Localized at x = 0 with a 2 px range: the far red pixel (x = 3) drops out, x = 1 would be half.
    let m = color_range_samples(&px, 4, &[s(red, Some((0.0, 0.0)))], 10.0, Some(2.0));
    assert_eq!(m, vec![1.0, 0.0, 0.0, 0.0]);
    let reds = vec![red; 4];
    let m = color_range_samples(&reds, 4, &[s(red, Some((0.0, 0.0)))], 10.0, Some(2.0));
    assert_eq!(m, vec![1.0, 0.5, 0.0, 0.0]);
    // No samples: nothing selected.
    assert!(color_range_samples(&px, 4, &[], 10.0, None).iter().all(|v| *v == 0.0));
}

/// Photoshop 2024's own Color Range coverage around the sample Lab (60, 25, 19.9961), read
/// from its selections on a 16-bit Lab document: (Lab in Photoshop's internal scale, where
/// a/b = (v16 − 32768)/256, fuzziness, coverage).
const PHOTOSHOP_COVERAGE: &[([f32; 3], f32, f32)] = &[
    ([61.9989, 25.5977, 20.5742], 40.0, 0.9686),
    ([64.9989, 25.5977, 20.5742], 40.0, 0.7961),
    ([67.9988, 25.5977, 20.5742], 40.0, 0.4824),
    ([71.9997, 25.5977, 20.5742], 40.0, 0.1098),
    ([79.9985, 25.5977, 20.5742], 40.0, 0.0),
    ([71.9997, 25.5977, 20.5742], 100.0, 0.8118),
    ([79.9985, 25.5977, 20.5742], 100.0, 0.4824),
    ([60.0, 13.5508, 20.5742], 40.0, 0.0432),
    ([60.0, 19.5742, 20.5742], 40.0, 0.6706),
    ([60.0, 22.582, 20.5742], 40.0, 0.9333),
    ([60.0, 28.6055, 20.5742], 40.0, 0.8549),
    ([60.0, 31.6211, 20.5742], 40.0, 0.5098),
    ([60.0, 37.6445, 20.5742], 40.0, 0.0078),
    ([60.0, 13.5508, 20.5742], 100.0, 0.7647),
    ([60.0, 37.6445, 20.5742], 100.0, 0.7137),
    ([60.0, 25.5977, 10.5352], 40.0, 0.1725),
    ([60.0, 25.5977, 16.5586], 40.0, 0.8667),
    ([60.0, 25.5977, 24.5898], 40.0, 0.7647),
    ([60.0, 25.5977, 30.6133], 40.0, 0.0863),
    ([60.0, 25.5977, 10.5352], 100.0, 0.8392),
    ([60.0, 25.5977, 30.6133], 100.0, 0.8),
];

#[test]
fn lab_range_matches_photoshop() {
    let r = LabRange::point([60.0, 25.0, 19.9961]);
    for &(lab, fz, ps) in PHOTOSHOP_COVERAGE {
        let got = lab_range_coverage(lab, &r, fz);
        assert!((got - ps).abs() <= 2.0 / 255.0, "{lab:?} at fuzziness {fz}: {got} vs Photoshop {ps}");
    }
    // Lightness is three times as forgiving as hue: the same 6-unit step along L and along a.
    assert!(lab_range_coverage([66.0, 25.0, 20.0], &r, 40.0) > lab_range_coverage([60.0, 31.0, 20.0], &r, 40.0));
    // The sampled colour itself is always fully selected.
    for fz in [1.0, 10.0, 40.0] {
        assert_eq!(lab_range_coverage([60.0, 25.0, 19.9961], &r, fz), 1.0);
    }
    assert_eq!(quantize_lab8([60.1, 25.4, -19.6]), [(153.0f32 / 2.55), 25.0, -20.0]);
}

#[test]
fn lab_range_box_and_edges() {
    let r = LabRange::spanning(&[[50.0, 10.0, 0.0], [60.0, 20.0, 5.0]]).unwrap();
    assert_eq!(r, LabRange { lo: [50.0, 10.0, 0.0], hi: [60.0, 20.0, 5.0] });
    assert!(LabRange::spanning(&[]).is_none());
    // Everything inside the box is fully selected, whatever the fuzziness.
    for fz in [1.0, 40.0, 200.0] {
        assert_eq!(lab_range_coverage([55.0, 15.0, 2.0], &r, fz), 1.0);
    }
    // Coverage falls off outside it and is gone a fuzziness away.
    let near = lab_range_coverage([55.0, 22.0, 2.0], &r, 40.0);
    assert!(near > 0.0 && near < 1.0, "{near}");
    assert_eq!(lab_range_coverage([55.0, 40.0, 2.0], &r, 40.0), 0.0);
    // Fuzziness 0: exact colours only.
    let p = LabRange::point([50.0, 10.0, -5.0]);
    assert_eq!(lab_range_coverage([50.0, 10.0, -5.0], &p, 0.0), 1.0);
    assert_eq!(lab_range_coverage([50.0, 11.0, -5.0], &p, 0.0), 0.0);
    // Hostile numbers never panic and never select.
    for v in [f32::NAN, f32::INFINITY, -f32::INFINITY] {
        for fz in [v, 40.0] {
            let k = lab_range_coverage([v, 0.0, 0.0], &p, fz);
            assert!((0.0..=1.0).contains(&k), "{k}");
        }
    }
}

/// Boxes Photoshop 2024's Color Range dialog returned after Subtract from Sample, at
/// fuzziness 40 unless noted: (lo, hi, subtracted value, fuzziness, new lo, new hi) on one axis.
const PHOTOSHOP_SUBTRACT_A: &[(f32, f32, f32, f32, f32, f32)] = &[
    (0.0, 20.0, 5.0, 40.0, 5.0, 20.0),
    (0.0, 20.0, 10.0, 40.0, 0.0, 10.0),
    (0.0, 20.0, 15.0, 40.0, 0.0, 15.0),
    (0.0, 20.0, 0.0, 40.0, 2.68, 20.0),
    (0.0, 20.0, -2.0, 40.0, 0.68, 20.0),
    (0.0, 20.0, -3.0, 40.0, 2.36, 20.0),
    (0.0, 20.0, -8.0, 40.0, 0.04, 20.0),
    (0.0, 20.0, -11.0, 40.0, 2.4, 20.0),
    (0.0, 20.0, -13.0, 40.0, 0.4, 20.0),
    (0.0, 20.0, -14.0, 40.0, 0.0, 20.0),
    (0.0, 20.0, 20.0, 40.0, 0.0, 17.32),
    (0.0, 20.0, 23.0, 40.0, 0.0, 17.64),
    (0.0, 20.0, 33.0, 40.0, 0.0, 19.6),
    (0.0, 20.0, 0.0, 80.0, 5.35, 20.0),
    (0.0, 20.0, -12.0, 80.0, 4.05, 20.0),
    (0.0, 20.0, -22.0, 80.0, 4.76, 20.0),
    (0.0, 20.0, -30.0, 80.0, 0.0, 20.0),
    (0.0, 6.0, 4.0, 40.0, 0.0, 4.0),
    (0.0, 2.0, 0.0, 40.0, 2.0, 2.0),
    (0.0, 2.0, 3.0, 40.0, 0.0, 0.32),
];

#[test]
fn subtract_reshapes_the_box_like_photoshop() {
    for &(lo, hi, m, fz, want_lo, want_hi) in PHOTOSHOP_SUBTRACT_A {
        let r = LabRange { lo: [50.0, lo, 0.0], hi: [50.0, hi, 0.0] }.subtract(&[50.0, m, 0.0], fz);
        assert!(
            (r.lo[1] - want_lo).abs() < 0.03 && (r.hi[1] - want_hi).abs() < 0.03,
            "a [{lo}, {hi}] − {m} at {fz}: [{}, {}] vs Photoshop [{want_lo}, {want_hi}]",
            r.lo[1],
            r.hi[1]
        );
    }
    // Lightness (box 40–60, fuzziness 40): Photoshop gave 43.14, 41.17, 40.79 and 45.1.
    for (m, want) in [(40.0, 43.14), (34.902, 41.17), (25.098, 40.79), (45.098, 45.1)] {
        let r = LabRange { lo: [40.0, 0.0, 0.0], hi: [60.0, 0.0, 0.0] }.subtract(&[m, 0.0, 0.0], 40.0);
        assert!((r.lo[0] - want).abs() < 0.05, "L − {m}: {} vs Photoshop {want}", r.lo[0]);
    }
    // All three axes at once (Photoshop: two samples, then one colour outside the box).
    let r = LabRange { lo: [53.58, 0.0, 0.0], hi: [59.05, 30.56, 25.03] }.subtract(&[53.33, -28.71, 36.45], 40.0);
    let want = LabRange { lo: [56.47, 0.0, 0.0], hi: [59.05, 30.56, 23.05] };
    for k in 0..3 {
        assert!((r.lo[k] - want.lo[k]).abs() < 0.05 && (r.hi[k] - want.hi[k]).abs() < 0.05, "{r:?} vs {want:?}");
    }
    // A single sample can't be subtracted from itself; hostile numbers change nothing.
    let p = LabRange::point([50.0, 10.0, 10.0]);
    assert_eq!(p.subtract(&[50.0, 10.0, 10.0], 40.0), p);
    assert_eq!(p.subtract(&[f32::NAN, f32::INFINITY, -f32::INFINITY], f32::NAN), p);
    assert_eq!(LabRange::spanning(&[[50.0, 0.0, 0.0]]).unwrap().grow(&[60.0, -5.0, 3.0]), LabRange { lo: [50.0, -5.0, 0.0], hi: [60.0, 0.0, 3.0] });
}

#[test]
fn eight_bit_grid_steps_are_uneven_like_photoshop() {
    let r = LabRange::point([50.0, 10.0, -20.0]);
    // The sampled colour itself is fully selected at any fuzziness.
    for fz in [1.0, 5.0, 10.0, 40.0] {
        assert_eq!(lab_range_coverage_8bit([50.0, 10.0, -20.0], &r, fz), 1.0);
    }
    // One step above on a counts as 1.5 units, one below as 0.5 (×3 weight): at fuzziness 40
    // (width 40.16) that is t = 4.5/40.16 and 1.5/40.16.
    let up = lab_range_coverage_8bit([50.0, 11.0, -20.0], &r, 40.0);
    let down = lab_range_coverage_8bit([50.0, 9.0, -20.0], &r, 40.0);
    let p = |d: f32| (((1.0 - 2.0 * (d / (40.0 * 256.0 / 255.0)).powi(2)) * 256.0).floor().min(255.0)) / 255.0;
    assert_eq!((up, down), (p(4.5), p(1.5)));
    assert!(down > up);
    // Lightness has no such offset; the exact (non-8-bit) coverage is symmetric.
    assert_eq!(lab_range_coverage_8bit([51.0, 10.0, -20.0], &r, 40.0), lab_range_coverage_8bit([49.0, 10.0, -20.0], &r, 40.0));
    assert_eq!(lab_range_coverage([50.0, 11.0, -20.0], &r, 40.0), lab_range_coverage([50.0, 9.0, -20.0], &r, 40.0));
    let px = [[50.0, 11.0, -20.0, 1.0], [50.0, 9.0, -20.0, 1.0]];
    assert_eq!(color_range_lab(&px, 2, &r, 40.0, true, None), vec![up, down]);
}

#[test]
fn color_range_lab_alpha_and_localized() {
    let r = LabRange::point([50.0, 0.0, 0.0]);
    let px = img(10, 1, |x, _| [50.0, 0.0, 0.0, if x == 0 { 0.0 } else { 1.0 }]);
    let m = color_range_lab(&px, 10, &r, 40.0, false, None);
    assert_eq!(m[0], 0.0, "transparent pixels are never selected");
    assert!(m[1..].iter().all(|v| *v > 0.9));
    let at = [(1.0, 0.0)];
    let l = color_range_lab(&px, 10, &r, 40.0, false, Some((4.0, &at)));
    assert!(l[1] > l[3] && l[3] > 0.0 && l[6] == 0.0, "{l:?}");
    // No positions: localized clusters have nothing to centre on and change nothing.
    assert_eq!(color_range_lab(&px, 10, &r, 40.0, false, Some((4.0, &[]))), m);
    assert!(color_range_lab(&[], 0, &r, 40.0, false, None).is_empty());
}

#[test]
fn hue_families() {
    let px = vec![
        [1.0, 0.0, 0.0, 1.0], // red
        [1.0, 0.5, 0.0, 1.0], // orange: between red and yellow
        [0.5, 0.5, 0.5, 1.0], // grey
        [1.0, 0.5, 0.5, 1.0], // pink: a muted red
        [0.0, 0.0, 1.0, 1.0], // blue
        [1.0, 0.0, 0.0, 0.0], // transparent
    ];
    let reds = hue_range(&px, 0.0);
    assert_eq!(reds[0], 1.0);
    assert!((reds[1] - 0.5).abs() < 1e-5);
    assert_eq!(reds[2], 0.0);
    assert!((reds[3] - 0.5).abs() < 1e-5);
    assert_eq!(reds[4], 0.0);
    assert_eq!(reds[5], 0.0);
    let yellows = hue_range(&px, 60.0);
    assert!((yellows[1] - 0.5).abs() < 1e-5);
    assert_eq!(hue_range(&px, 240.0)[4], 1.0);
    // Magentas wrap around 360°.
    assert_eq!(hue_range(&[[1.0, 0.0, 1.0, 1.0]], 300.0)[0], 1.0);
    assert!((hue_range(&[[1.0, 0.0, 0.5, 1.0]], 300.0)[0] - 0.5).abs() < 1e-5);
}

#[test]
fn hue_families_weigh_dark_colours_like_photoshop() {
    // Photoshop 2025's Blues on an 8-bit sRGB document (selection levels 0..255, #5243): the
    // muted navy #3d3f54, a dark blue (0, 0, 84) and a mid blue (61, 61, 153).
    let c = |r: u8, g: u8, b: u8| [f32::from(r) / 255.0, f32::from(g) / 255.0, f32::from(b) / 255.0, 1.0];
    let blues = hue_range(&[c(0x3d, 0x3f, 0x54), c(0, 0, 84), c(61, 61, 153)], 240.0);
    for (got, want) in blues.iter().zip([35.0, 140.0, 129.0]) {
        assert!((got * 255.0 - want).abs() <= 1.0, "{} vs Photoshop {want}", got * 255.0);
    }
}

#[test]
fn tonal_ranges() {
    let g = |v: f32| [v / 255.0, v / 255.0, v / 255.0, 1.0];
    let px = vec![g(0.0), g(60.0), g(75.0), g(128.0), g(200.0), g(255.0)];
    let near = |a: &[f32], b: &[f32]| a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-4);
    // Shadows up to 65, fading over 20 levels.
    let sh = tone_range(&px, 0.0, 65.0, 20.0);
    assert!(near(&sh, &[1.0, 1.0, 0.5, 0.0, 0.0, 0.0]), "{sh:?}");
    // Highlights from 190.
    let h = tone_range(&px, 190.0, 255.0, 20.0);
    assert!(near(&h, &[0.0, 0.0, 0.0, 0.0, 1.0, 1.0]), "{h:?}");
    // Midtones 105..150.
    let m = tone_range(&px, 105.0, 150.0, 20.0);
    assert!(near(&m, &[0.0, 0.0, 0.0, 1.0, 0.0, 0.0]), "{m:?}");
}

fn square(w: usize, h: usize) -> Vec<f32> {
    (0..w * h).map(|i| if (5..15).contains(&(i % w)) && (5..15).contains(&(i / w)) { 1.0 } else { 0.0 }).collect()
}

#[test]
fn expand_contract_border() {
    let m = square(20, 20);
    let e = expand(&m, 20, 20, 2.0);
    assert_eq!(e[10 * 20 + 3], 1.0);
    assert_eq!(e[10 * 20 + 2], 0.0);
    let c = contract(&m, 20, 20, 2.0);
    assert_eq!(c[10 * 20 + 6], 0.0);
    assert_eq!(c[10 * 20 + 7], 1.0);
    let b = border(&m, 20, 20, 4.0);
    assert_eq!(b[10 * 20 + 10], 0.0);
    assert!(b[10 * 20 + 5] > 0.5 && b[10 * 20 + 4] > 0.5);
}

/// Photoshop 27.11, measured: a 32×32 rectangle at (16,16) in 64×64, Select › Modify ›
/// Border 4 px, filled white. Row y = 32 for x = 8..=24, and the diagonal (16,16)..(24,24).
#[test]
fn border_matches_photoshop() {
    let (w, h) = (64, 64);
    let m: Vec<f32> = (0..w * h).map(|i| if (16..48).contains(&(i % w)) && (16..48).contains(&(i / w)) { 1.0 } else { 0.0 }).collect();
    let b = border(&m, w, h, 4.0);
    let v = |x: usize, y: usize| (b[y * w + x] * 255.0).round() as i32;
    let row: Vec<i32> = (8..=24).map(|x| v(x, 32)).collect();
    let diag: Vec<i32> = (16..=24).map(|d| v(d, d)).collect();
    let near = |got: &[i32], want: &[i32]| got.iter().zip(want).all(|(g, w)| (g - w).abs() <= 1);
    let want_row = [0, 0, 0, 0, 63, 127, 191, 255, 255, 191, 127, 63, 0, 0, 0, 0, 0];
    assert!(near(&row, &want_row), "{row:?}");
    assert!(near(&diag, &[255, 191, 127, 63, 0, 0, 0, 0, 0]), "{diag:?}");
}

#[test]
fn smooth_removes_specks_and_feather_softens() {
    let mut m = square(20, 20);
    m[2 * 20 + 2] = 1.0;
    let s = smooth(&m, 20, 20, 1.0);
    assert_eq!(s[2 * 20 + 2], 0.0);
    assert_eq!(s[10 * 20 + 10], 1.0);
    // Radius 2 (σ 2) keeps the blur inside the 20×20 canvas, so no coverage is lost.
    let f = feather(&square(20, 20), 20, 20, 2.0);
    let edge = f[10 * 20 + 5];
    assert!(edge > 0.3 && edge < 0.8, "{edge}");
    let sum: f32 = f.iter().sum();
    assert!((sum - 100.0).abs() < 1.0);
}

#[test]
fn polygon_area_and_aa() {
    let a = Rect::new(0, 0, 20, 20);
    let sq = polygon(&[(2.0, 2.0), (12.0, 2.0), (12.0, 12.0), (2.0, 12.0)], a, true);
    let area: f32 = sq.iter().sum();
    assert!((area - 100.0).abs() < 1e-3, "{area}");
    let tri = polygon(&[(0.0, 0.0), (10.0, 0.0), (0.0, 10.0)], a, true);
    let area: f32 = tri.iter().sum();
    assert!((area - 50.0).abs() < 1.0, "{area}");
    assert!(tri.iter().any(|v| *v > 0.0 && *v < 1.0));
    let hard = polygon(&[(0.0, 0.0), (10.0, 0.0), (0.0, 10.0)], a, false);
    assert!(hard.iter().all(|v| *v == 0.0 || *v == 1.0));
    assert!(polygon(&[(0.0, 0.0), (1.0, 1.0)], a, true).iter().all(|v| *v == 0.0));
}

/// The previous polygon fill (every edge tested on every sub-row), kept as the oracle.
fn old_polygon(points: &[(f32, f32)], area: Rect, anti_alias: bool) -> Vec<f32> {
    let (w, h) = (area.width() as usize, area.height() as usize);
    let mut m = vec![0.0f32; w * h];
    if points.len() < 3 {
        return m;
    }
    let sub = if anti_alias { 4 } else { 1 };
    let mut xs: Vec<f32> = Vec::new();
    for py in 0..h {
        for s in 0..sub {
            let y = area.y0 as f32 + py as f32 + (s as f32 + 0.5) / sub as f32;
            xs.clear();
            for i in 0..points.len() {
                let (a, b) = (points[i], points[(i + 1) % points.len()]);
                if (a.1 <= y && b.1 > y) || (b.1 <= y && a.1 > y) {
                    xs.push(a.0 + (y - a.1) / (b.1 - a.1) * (b.0 - a.0));
                }
            }
            xs.sort_by(|a, b| a.total_cmp(b));
            for span in xs.as_chunks::<2>().0 {
                let (x0, x1) = ((span[0] - area.x0 as f32).max(0.0), (span[1] - area.x0 as f32).min(w as f32));
                if x1 <= x0 {
                    continue;
                }
                let (c0, c1) = (x0.floor() as usize, (x1.ceil() as usize).min(w));
                for cx in c0..c1 {
                    let cov = if anti_alias {
                        ((cx + 1) as f32).min(x1) - (cx as f32).max(x0)
                    } else {
                        let c = cx as f32 + 0.5;
                        if c >= x0 && c < x1 { 1.0 } else { 0.0 }
                    };
                    m[py * w + cx] += cov.max(0.0) / sub as f32;
                }
            }
        }
    }
    for v in &mut m {
        *v = v.min(1.0);
    }
    m
}

#[test]
fn polygon_matches_the_every_edge_fill_bit_for_bit() {
    let mut seed = 0x2545_f491_u32;
    let mut rnd = move || {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        seed as f32 / u32::MAX as f32
    };
    // Taller than one 64-row band, offset, and a 1-pixel-wide area.
    let areas = [Rect::new(0, 0, 90, 150), Rect::new(-13, 7, 60, 211), Rect::new(5, -40, 6, 30)];
    let mut cases: Vec<Vec<(f32, f32)>> = vec![
        // Self-intersecting star, horizontal edges, repeated points, collinear runs.
        (0..11)
            .map(|i| {
                let t = i as f32 * 4.0 * std::f32::consts::PI / 11.0;
                (40.0 + 35.0 * t.cos(), 80.0 + 70.0 * t.sin())
            })
            .collect(),
        vec![(10.0, 10.0), (50.0, 10.0), (50.0, 10.0), (50.0, 60.0), (30.0, 60.0), (30.0, 30.0), (10.0, 30.0)],
        vec![(0.0, 0.0), (20.0, 0.0), (40.0, 0.0), (40.0, 0.0), (0.0, 0.0)],
        // Vertices on the sub-row centres and the pixel edges.
        vec![(3.125, 2.375), (70.5, 2.375), (70.875, 140.625), (3.0, 140.0), (35.0, 64.0), (35.0, 64.125)],
        // Entirely outside, and straddling every side.
        vec![(-500.0, -500.0), (-400.0, -500.0), (-450.0, -300.0)],
        vec![(-80.0, -60.0), (300.0, 20.0), (60.0, 400.0)],
        // Non-finite coordinates and a negative zero.
        vec![(10.0, 10.0), (f32::NAN, 40.0), (60.0, 70.0), (20.0, f32::INFINITY), (-0.0, -0.0), (0.0, 0.0)],
    ];
    for n in [3, 7, 40, 400] {
        for spread in [1.0, 2.0] {
            cases.push((0..n).map(|_| (rnd() * 100.0 * spread - 50.0 * (spread - 1.0), rnd() * 220.0 * spread - 60.0 * spread)).collect());
            // Lasso-like: a random walk snapped to quarter pixels.
            let (mut x, mut y) = (45.0f32, 75.0f32);
            cases.push(
                (0..n)
                    .map(|_| {
                        x = ((x + (rnd() - 0.5) * 12.0 * spread) * 4.0).round() / 4.0;
                        y = ((y + (rnd() - 0.5) * 12.0 * spread) * 4.0).round() / 4.0;
                        (x, y)
                    })
                    .collect(),
            );
        }
    }
    for pts in &cases {
        for &area in &areas {
            for aa in [true, false] {
                let (new, old) = (polygon(pts, area, aa), old_polygon(pts, area, aa));
                assert_eq!(new.len(), old.len());
                let diff = new.iter().zip(&old).position(|(a, b)| a.to_bits() != b.to_bits());
                assert!(diff.is_none(), "{pts:?} {area:?} aa={aa}: first difference at {diff:?}");
            }
        }
    }
}

#[test]
fn combine_modes() {
    let a = Rect::new(0, 0, 4, 1);
    let old = mask_to_surface(&[1.0, 1.0, 0.0, 0.0], a);
    let new = [0.0, 1.0, 1.0, 0.0];
    let v = |s: Option<Surface>| mask_from_surface(s.as_ref(), a);
    assert_eq!(v(combine(Some(&old), &new, a, SelectionMode::Replace)), vec![0.0, 1.0, 1.0, 0.0]);
    assert_eq!(v(combine(Some(&old), &new, a, SelectionMode::Add)), vec![1.0, 1.0, 1.0, 0.0]);
    assert_eq!(v(combine(Some(&old), &new, a, SelectionMode::Subtract)), vec![1.0, 0.0, 0.0, 0.0]);
    assert_eq!(v(combine(Some(&old), &new, a, SelectionMode::Intersect)), vec![0.0, 1.0, 0.0, 0.0]);
    assert!(combine(Some(&old), &[1.0; 4], a, SelectionMode::Subtract).is_none());
    assert_eq!(SelectionMode::parse("add"), SelectionMode::Add);
}

/// The previous (per-sample) conversions, kept as the oracle for the tiled fast paths.
fn old_mask_to_surface(m: &[f32], area: Rect) -> Surface {
    let mut s = Surface::new(PixelFormat::GRAY8);
    if !area.is_empty() {
        s.write_region(area, m);
    }
    s.prune();
    s
}

fn old_combine(old: Option<&Surface>, new: &[f32], area: Rect, mode: SelectionMode) -> Option<Surface> {
    let mut prev = Vec::new();
    match old {
        Some(s) => s.read_region_into(area, &mut prev),
        None => prev = vec![0.0; new.len()],
    }
    let q: Vec<f32> = new.iter().map(|v| (v.clamp(0.0, 1.0) * 255.0).round() / 255.0).collect();
    let out: Vec<f32> = match mode {
        SelectionMode::Replace => q,
        SelectionMode::Add => prev.iter().zip(&q).map(|(a, b)| a.max(*b)).collect(),
        SelectionMode::Subtract => prev.iter().zip(&q).map(|(a, b)| (a - b).max(0.0)).collect(),
        SelectionMode::Intersect => prev.iter().zip(&q).map(|(a, b)| a.min(*b)).collect(),
    };
    let out: Vec<f32> = out.into_iter().map(|v| if v < 0.5 / 255.0 { 0.0 } else { v }).collect();
    if out.iter().all(|v| *v <= 0.0) {
        return None;
    }
    Some(old_mask_to_surface(&out, area))
}

#[test]
fn tiled_conversions_match_per_sample_ones() {
    let mut rng = crate::photo_util::Rng(7);
    let mut u = move || (rng.next() >> 11) as f32 / (1u64 << 53) as f32;
    // Off-grid areas spanning several tiles, including negative origins.
    // The last one holds whole tiles, which fully selected masks fill (shared tiles).
    for area in [Rect::new(0, 0, 300, 20), Rect::new(-37, 250, 530, 263), Rect::new(255, -3, 258, 600), Rect::new(5, 5, 6, 6), Rect::new(-37, -300, 530, 263)] {
        let n = area.width() as usize * area.height() as usize;
        let old_m: Vec<f32> = (0..n).map(|i| if i % 97 < 40 { u() } else { 0.0 }).collect();
        let old = old_mask_to_surface(&old_m, area);
        let fast = mask_to_surface(&old_m, area);
        assert_eq!(fast, old, "{area:?}");
        let mut slow_read = Vec::new();
        old.read_region_into(area, &mut slow_read);
        assert_eq!(mask_from_surface(Some(&old), area), slow_read);
        for kind in 0..4 {
            let new: Vec<f32> = (0..n)
                .map(|i| match kind {
                    0 => u() * 1.2 - 0.1,
                    1 => {
                        if i % 131 < 50 {
                            1.0
                        } else {
                            0.0
                        }
                    }
                    2 => 0.001,
                    _ => f32::from(u8::from(i >= 1000)),
                })
                .collect();
            for mode in [SelectionMode::Replace, SelectionMode::Add, SelectionMode::Subtract, SelectionMode::Intersect] {
                for prev in [None, Some(&old)] {
                    assert_eq!(combine(prev, &new, area, mode), old_combine(prev, &new, area, mode), "{area:?} {kind} {mode:?}");
                }
            }
        }
    }
    // A short mask buffer reads as zeros instead of panicking.
    let a = Rect::new(0, 0, 10, 10);
    assert!(combine(None, &[1.0; 5], a, SelectionMode::Replace).is_some());
    assert_eq!(mask_to_surface(&[], a).tile_count(), 0);
    assert!(combine(None, &[], Rect::EMPTY, SelectionMode::Replace).is_none());
}

fn blobs(w: i32, h: i32) -> Vec<[u8; 4]> {
    // Deterministic blotchy image with gradients and hard edges.
    (0..w * h)
        .map(|i| {
            let (x, y) = (i % w, i / w);
            let v = ((x * 7 + y * 3) % 23 + if (x / 5 + y / 4) % 3 == 0 { 120 } else { 0 }) as u8;
            [v, v / 2, 255 - v, if x % 11 == 0 { 128 } else { 255 }]
        })
        .collect()
}

#[test]
fn magic_wand_rejects_incomplete_and_overflowing_pixel_inputs() {
    let area = Rect::new(0, 0, 4, 4);
    let short = vec![[255u8, 0, 0, 255]; 4];
    let floats: Vec<[f32; 4]> = short.iter().map(|p| p.map(|v| f32::from(v) / 255.0)).collect();
    assert_eq!(magic_wand(&floats, area, (1, 1), 0.0, true, false), Vec::<f32>::new());
    assert_eq!(wand_region(&short, area, (1, 1), 0.0, true, false), None);

    let huge = Rect::new(i32::MIN, i32::MIN, i32::MAX, i32::MAX);
    assert!(magic_wand(&floats, huge, (0, 0), 0.0, false, false).is_empty());
    assert_eq!(wand_region(&short, huge, (0, 0), 0.0, false, false), None);

    let valid = Rect::new(0, 0, 2, 2);
    assert_eq!(magic_wand(&floats, valid, (0, 0), 0.0, false, false), vec![1.0; 4]);
    assert!(wand_region(&short, valid, (0, 0), 0.0, false, false).is_some());
}

#[test]
fn region_coverage_outside_a_short_mask_is_zero() {
    let region = Region { bbox: Rect::new(-2, -3, 1, -1), mask: vec![255, 128] };
    assert_eq!(region.at(-2, -3), 1.0);
    assert!((region.at(-1, -3) - 128.0 / 255.0).abs() < 1e-6);
    assert_eq!(region.at(0, -3), 0.0);
    assert_eq!(region.at(-2, -2), 0.0);
    assert_eq!(region.at(-3, -3), 0.0);

    let extreme = Region { bbox: Rect::new(i32::MIN, i32::MIN, i32::MAX, i32::MAX), mask: vec![] };
    assert_eq!(extreme.at(i32::MAX - 1, i32::MAX - 1), 0.0);
    assert_eq!(extreme.at(i32::MIN, i32::MIN), 0.0);
}

#[test]
fn wand_region_matches_reference_magic_wand() {
    let (w, h) = (37, 29);
    let area = Rect::new(3, -2, 3 + w, -2 + h);
    let img = blobs(w, h);
    let f: Vec<[f32; 4]> = img.iter().map(|p| p.map(|v| v as f32 / 255.0)).collect();
    for (seed, tol, contiguous, aa) in
        [((10, 5), 10.0, true, true), ((10, 5), 10.0, true, false), ((20, 20), 40.0, false, true), ((4, -2), 0.0, true, true), ((30, 10), 255.0, true, true)]
    {
        let reference = magic_wand(&f, area, seed, tol, contiguous, aa);
        let r = wand_region(&img, area, seed, tol, contiguous, aa).unwrap();
        for y in area.y0..area.y1 {
            for x in area.x0..area.x1 {
                let want = (reference[((y - area.y0) * w + x - area.x0) as usize] * 255.0).round();
                assert!((r.at(x, y) * 255.0 - want).abs() <= 1.0, "{seed:?} {tol} {contiguous} {aa} at {x},{y}: {} vs {want}", r.at(x, y) * 255.0);
            }
        }
    }
    assert!(wand_region(&img, area, (0, 0), 10.0, true, true).is_none());
}

#[test]
fn combine_region_matches_combine() {
    let area = Rect::new(0, 0, 40, 30);
    let mut old = Surface::new(PixelFormat::GRAY8);
    old.fill_rect(Rect::new(5, 5, 25, 20), &[1.0]);
    old.fill_rect(Rect::new(8, 8, 10, 10), &[0.4]);
    let img = blobs(40, 30);
    let region = wand_region(&img, area, (12, 12), 30.0, true, true).unwrap();
    let full: Vec<f32> = (0..area.height()).flat_map(|y| (0..area.width()).map(move |x| (x, y))).map(|(x, y)| region.at(x as i32, y as i32)).collect();
    for mode in [SelectionMode::Replace, SelectionMode::Add, SelectionMode::Subtract, SelectionMode::Intersect] {
        let a = combine(Some(&old), &full, area, mode);
        let b = combine_region(Some(&old), Some(&region), mode);
        assert_eq!(a.is_some(), b.is_some(), "{mode:?}");
        if let (Some(a), Some(b)) = (a, b) {
            for y in 0..30 {
                for x in 0..40 {
                    assert!((a.pixel(x, y)[0] - b.pixel(x, y)[0]).abs() < 1.5 / 255.0, "{mode:?} {x},{y}");
                }
            }
        }
    }
    assert!(combine_region(None, None, SelectionMode::Subtract).is_none());
}
