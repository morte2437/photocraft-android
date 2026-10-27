use super::mosaic_size;

fn blank(w: usize, h: usize) -> (usize, usize) {
    (w, h)
}

#[test]
fn a_thin_image_with_turned_variants_does_not_get_a_quadratic_mosaic() {
    // 40000×1 plus its 90° copy (1×40000): about 1.6 billion mosaic cells for 80000 pixels.
    assert_eq!(mosaic_size(40_000, 1, 8, &[blank(1, 40_000)]), None);
    assert_eq!(mosaic_size(usize::MAX, 1, 8, &[blank(1, 1)]), None);
    assert_eq!(mosaic_size(100, 100, 8, &[blank(100, 100)]), Some((208, 108)));
}

use super::*;

/// Vertical stripes (period 6) in grey, with a hole in the middle.
fn stripes(w: usize, h: usize) -> (Vec<f32>, Vec<bool>) {
    let img: Vec<f32> = (0..w * h).map(|i| if (i % w) % 6 < 3 { 0.2 } else { 0.8 }).collect();
    let hole: Vec<bool> = (0..w * h).map(|i| (i % w).abs_diff(w / 2) < 4 && (i / w).abs_diff(h / 2) < 4).collect();
    (img, hole)
}

#[test]
fn strokes_paint_their_path_and_leave_a_mismatched_mask_alone() {
    let area = Rect::new(10, 20, 50, 40);
    let (w, h) = (40, 20);
    let strokes = [
        SamplingStroke { add: true, size: 6.0, points: vec![[12.0, 25.0], [40.0, 25.0], [45.0, 35.0]] },
        SamplingStroke { add: false, size: 3.0, points: vec![[30.0, 25.0]] },
        SamplingStroke { add: true, size: 2.0, points: Vec::new() },
    ];
    let mut mask = vec![false; w * h];
    paint_strokes(&mut mask, area, &strokes);
    // The same pixels as testing every pixel against every segment of each stroke in turn.
    let mut want = vec![false; w * h];
    for s in &strokes {
        let r2 = (s.size / 2.0).max(0.5).powi(2);
        let segs: Vec<_> = if s.points.len() == 1 { vec![(s.points[0], s.points[0])] } else { s.points.windows(2).map(|p| (p[0], p[1])).collect() };
        for (i, m) in want.iter_mut().enumerate() {
            let c = [f64::from(area.x0) + (i % w) as f64 + 0.5, f64::from(area.y0) + (i / w) as f64 + 0.5];
            if segs.iter().any(|(a, b)| seg_dist2(c, *a, *b) <= r2) {
                *m = s.add;
            }
        }
    }
    assert_eq!(mask, want);
    assert!(mask[5 * w + 10] && !mask[5 * w + 20], "added along the path, cleared under the subtracting dab");
    // A mask shorter or longer than the area is not touched (and nothing panics).
    for len in [0, 7, w * h - 1, w * h + 3] {
        let mut short = vec![false; len];
        paint_strokes(&mut short, area, &strokes);
        assert!(short.iter().all(|v| !v), "len {len}");
    }
    // Hostile coordinates stay inside the area.
    let mut m = vec![false; w * h];
    let wild = [SamplingStroke { add: true, size: 5000.0, points: vec![[f64::MAX, -1e300], [f64::NAN, 1e7]] }];
    paint_strokes(&mut m, area, &wild);
}

#[test]
fn cancelled_fill_stops_and_progress_reaches_one() {
    use std::sync::atomic::{AtomicU32, Ordering};
    let (w, h) = (60, 40);
    let (img, hole) = stripes(w, h);
    let source = vec![true; w * h];
    let yes = || true;
    let cancelled = photocraft_raster::Interrupt::cancel_only(&yes);
    assert!(fill_with(w, h, 1, &img, &hole, &source, &FillOptions::default(), &cancelled).is_err());
    let last = AtomicU32::new(0);
    let no = || false;
    let progress = |f: f32| {
        assert!(f >= f32::from_bits(last.load(Ordering::Relaxed)), "progress went backwards");
        last.store(f.to_bits(), Ordering::Relaxed);
    };
    let out = fill_with(w, h, 1, &img, &hole, &source, &FillOptions::default(), &photocraft_raster::Interrupt::new(&no, &progress)).unwrap();
    assert_eq!(f32::from_bits(last.load(Ordering::Relaxed)), 1.0);
    assert_eq!(out, fill(w, h, 1, &img, &hole, &source, &FillOptions::default()), "same result as the plain fill");
}

#[test]
fn filling_in_place_is_filling_a_copy() {
    let (w, h) = (90, 70);
    let rgb: Vec<f32> = noise(w, h, 4).iter().enumerate().map(|(i, v)| v * 0.5 + 0.3 * ((i / 3 % w) % 11 < 5) as u8 as f32).collect();
    let hole: Vec<bool> = (0..w * h).map(|i| (30..52).contains(&(i % w)) && (25..44).contains(&(i / w))).collect();
    let source: Vec<bool> = (0..w * h).map(|i| i % w < 75).collect();
    let none = Interrupt::NONE;
    for opts in [FillOptions::default(), FillOptions { color_adaptation: Adapt::NONE, mirror: true, ..Default::default() }] {
        let copy = fill_with(w, h, 3, &rgb, &hole, &source, &opts, &none).unwrap();
        let owned = fill_owned(w, h, 3, rgb.clone(), &hole, &source, &opts, &none).unwrap();
        assert_eq!(copy.iter().map(|v| v.to_bits()).collect::<Vec<_>>(), owned.iter().map(|v| v.to_bits()).collect::<Vec<_>>());
        // Only the hole changes.
        assert!((0..w * h).all(|i| hole[i] || (0..3).all(|c| owned[i * 3 + c].to_bits() == rgb[i * 3 + c].to_bits())));
    }
    // Nothing to copy from: the membrane, either way.
    let nowhere = vec![false; w * h];
    let copy = fill_with(w, h, 3, &rgb, &hole, &nowhere, &FillOptions::default(), &none).unwrap();
    assert_eq!(copy, fill_owned(w, h, 3, rgb.clone(), &hole, &nowhere, &FillOptions::default(), &none).unwrap());
}

#[test]
fn auto_sampling_can_be_cancelled() {
    let (w, h) = (200, 160);
    let img: Vec<f32> = noise(w, h, 5).iter().enumerate().map(|(i, v)| v * 0.1 + if (i / 3) % w < 120 { 0.3 } else { 0.6 }).collect();
    let hole: Vec<bool> = (0..w * h).map(|i| (80..110).contains(&(i % w)) && (60..90).contains(&(i / w))).collect();
    let yes = || true;
    assert!(auto_sampling_with(w, h, 3, &img, &hole, &Interrupt::cancel_only(&yes)).is_err());
    assert_eq!(auto_sampling_with(w, h, 3, &img, &hole, &Interrupt::NONE).unwrap(), auto_sampling(w, h, 3, &img, &hole));
}

#[test]
fn fills_only_the_hole_with_known_texture() {
    let (w, h) = (48, 32);
    let (img, hole) = stripes(w, h);
    let src = vec![true; w * h];
    let out = fill(w, h, 1, &img, &hole, &src, &FillOptions { color_adaptation: Adapt::NONE, ..Default::default() });
    for i in 0..w * h {
        if !hole[i] {
            assert_eq!(out[i], img[i]);
        }
    }
    // The hole is filled with stripe values, not a flat average.
    let vals: Vec<f32> = (0..w * h).filter(|i| hole[*i]).map(|i| out[i]).collect();
    assert!(vals.iter().any(|v| *v < 0.35) && vals.iter().any(|v| *v > 0.65), "{vals:?}");
}

#[test]
fn sampling_area_restricts_sources() {
    // Left half dark, right half bright; sampling only the right half fills bright.
    let (w, h) = (40, 20);
    let img: Vec<f32> = (0..w * h).map(|i| if i % w < 20 { 0.1 } else { 0.9 }).collect();
    let hole: Vec<bool> = (0..w * h).map(|i| (16..24).contains(&(i % w)) && (6..14).contains(&(i / w))).collect();
    let right: Vec<bool> = (0..w * h).map(|i| i % w >= 24).collect();
    let out = fill(w, h, 1, &img, &hole, &right, &FillOptions { color_adaptation: Adapt::NONE, ..Default::default() });
    let mean: f32 = (0..w * h).filter(|i| hole[*i]).map(|i| out[i]).sum::<f32>() / 64.0;
    assert!(mean > 0.8, "mean {mean}");
}

#[test]
fn adaptations_run_and_stay_deterministic() {
    let (w, h) = (32, 24);
    let (img, hole) = stripes(w, h);
    let rgb: Vec<f32> = img.iter().flat_map(|v| [*v, *v * 0.5, 1.0 - *v]).collect();
    let src = vec![true; w * h];
    let opts = FillOptions { color_adaptation: color_level("high"), rotations: rotation_level("low"), scale: true, mirror: true, seed: 7 };
    let a = fill(w, h, 3, &rgb, &hole, &src, &opts);
    let b = fill(w, h, 3, &rgb, &hole, &src, &opts);
    assert_eq!(a, b);
    assert!(a.iter().all(|v| v.is_finite() && (0.0..=1.0).contains(v)));
}

#[test]
fn color_adaptation_follows_a_gradient() {
    // A horizontal brightness ramp with a hole: adaptation keeps the fill near the ramp.
    let (w, h) = (40, 20);
    let img: Vec<f32> = (0..w * h).map(|i| (i % w) as f32 / w as f32).collect();
    let hole: Vec<bool> = (0..w * h).map(|i| (14..26).contains(&(i % w)) && (5..15).contains(&(i / w))).collect();
    let src = vec![true; w * h];
    let out = fill(w, h, 1, &img, &hole, &src, &FillOptions { color_adaptation: color_level("veryHigh"), ..Default::default() });
    let err: f32 = (0..w * h).filter(|i| hole[*i]).map(|i| (out[i] - img[i]).abs()).sum::<f32>() / 120.0;
    assert!(err < 0.12, "mean error {err}");
}

/// A vertical sky gradient (each row one colour) filled at every adaptation level stays the
/// gradient: no tinted blotch where the copied rows' colours were shifted per channel.
#[test]
fn color_adaptation_leaves_a_smooth_gradient_untinted() {
    let (w, h) = (120, 60);
    let row = |y: usize| {
        let t = y as f32 / (h - 1) as f32;
        [0.27 + 0.45 * t, 0.53 + 0.32 * t, 0.86 + 0.1 * t]
    };
    let img: Vec<f32> = (0..w * h).flat_map(|i| row(i / w)).collect();
    let hole: Vec<bool> = (0..w * h).map(|i| (50..70).contains(&(i % w)) && (20..40).contains(&(i / w))).collect();
    let src = vec![true; w * h];
    for level in ["none", "default", "high", "veryHigh"] {
        let out = fill(w, h, 3, &img, &hole, &src, &FillOptions { color_adaptation: color_level(level), ..Default::default() });
        let (out, img) = (&out, &img);
        let worst = (0..w * h).filter(|i| hole[*i]).flat_map(|i| (0..3).map(move |c| (out[i * 3 + c] - img[i * 3 + c]).abs())).fold(0.0f32, f32::max);
        assert!(worst < 3.0 / 255.0, "{level}: off by {}", worst * 255.0);
    }
}

/// I.i.d. noise: every value occurs once, so a filled pixel equal to an image pixel was copied.
fn noise(w: usize, h: usize, seed: u64) -> Vec<f32> {
    let mut z = seed;
    (0..w * h * 3)
        .map(|_| {
            z = z.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            0.2 + 0.6 * ((z >> 40) as f32 / (1u64 << 24) as f32)
        })
        .collect()
}

#[test]
fn sampling_window_is_photoshops() {
    let canvas = Rect::new(0, 0, 800, 800);
    let sq = |x0: i32, y0: i32, w: i32, h: i32| Rect::new(x0, y0, x0 + w, y0 + h);
    // Photoshop 25.4's sampling overlay: a square of side 4·√(max(w,50)·max(h,50)) centred on
    // the selection's bounds.
    assert_eq!(sampling_window(sq(375, 375, 50, 50), canvas), sq(300, 300, 200, 200));
    assert_eq!(sampling_window(sq(350, 350, 100, 100), canvas), sq(200, 200, 400, 400));
    assert_eq!(sampling_window(sq(395, 395, 10, 10), canvas), sq(300, 300, 200, 200));
    assert_eq!(sampling_window(sq(370, 370, 60, 60), canvas), sq(280, 280, 240, 240));
    // Long selections (Photoshop's overlay, read off the screen to a pixel: 281, 399, 691).
    assert_eq!(sampling_window(sq(350, 385, 100, 30), canvas), sq(258, 258, 283, 283));
    assert_eq!(sampling_window(sq(300, 375, 200, 50), canvas), sq(200, 200, 400, 400));
    assert_eq!(sampling_window(sq(250, 350, 300, 100), canvas), sq(53, 53, 693, 693));
    // At the canvas edge the square slides inwards rather than being cut.
    assert_eq!(sampling_window(sq(10, 375, 50, 50), canvas), sq(0, 300, 200, 200));
    assert_eq!(sampling_window(sq(5, 5, 60, 60), canvas), sq(0, 0, 240, 240));
    // A canvas smaller than the square clips it; a very long selection keeps its length.
    assert_eq!(sampling_window(sq(40, 40, 50, 50), Rect::new(0, 0, 150, 120)), Rect::new(0, 0, 150, 120));
    assert_eq!(sampling_window(sq(100, 995, 1800, 10), Rect::new(0, 0, 2000, 2000)), sq(100, 400, 1800, 1200));
    assert_eq!(sampling_window(Rect::EMPTY, canvas), Rect::EMPTY);
}

#[test]
fn auto_sampling_keeps_what_looks_like_the_surroundings() {
    // Grey texture, dark on the left, 50 levels brighter on the right; the hole is in the dark
    // part near the boundary (Photoshop's Auto leaves the bright part out).
    let (w, h) = (200, 200);
    let tex = noise(w, h, 3);
    let shade = |x: usize, band: bool| {
        if band {
            if (130..150).contains(&x) { 0.55 } else { 0.35 }
        } else if x < 135 {
            0.35
        } else {
            0.55
        }
    };
    for band in [false, true] {
        let img: Vec<f32> = (0..w * h * 3).map(|i| shade((i / 3) % w, band) + (tex[i] - 0.5) * 0.1).collect();
        let hole: Vec<bool> = (0..w * h).map(|i| (75..125).contains(&(i % w)) && (75..125).contains(&(i / w))).collect();
        let area = auto_sampling(w, h, 3, &img, &hole);
        let frac = |x0: usize, x1: usize| {
            let n = (0..w * h).filter(|i| (x0..x1).contains(&(i % w)) && !hole[*i]).count();
            (0..w * h).filter(|i| (x0..x1).contains(&(i % w)) && area[*i]).count() as f32 / n as f32
        };
        assert!(hole.iter().zip(&area).all(|(hl, a)| !(*hl && *a)), "never the hole");
        assert!(frac(0, 125) > 0.95, "the dark side is sampled ({})", frac(0, 125));
        if band {
            // A bright band through the window is left out; the dark area beyond it is not.
            assert!(frac(133, 147) < 0.05, "the band is excluded ({})", frac(133, 147));
            assert!(frac(155, 200) > 0.9, "beyond the band is sampled ({})", frac(155, 200));
        } else {
            assert!(frac(140, 200) < 0.05, "the bright side is excluded ({})", frac(140, 200));
        }
    }
}

#[test]
fn auto_sampling_keeps_the_surroundings_of_every_part_of_a_thin_selection() {
    // A long thin selection across a grey ramp, and a small knob selected with it in a red
    // corner. Each part keeps its own surroundings; a compact selection is unchanged.
    let (w, h) = (900, 600);
    let tex = noise(w, h, 4);
    let img: Vec<f32> = (0..w * h * 3)
        .map(|i| {
            let (x, y, c) = ((i / 3) % w, (i / 3) / w, i % 3);
            let base = if x > 690 && y > 420 { [0.7, 0.25, 0.2][c] } else { 0.2 + 0.5 * x as f32 / w as f32 };
            base + (tex[i] - 0.5) * 0.04
        })
        .collect();
    let line = |i: usize| (30..870).contains(&(i % w)) && (180..192).contains(&(i / w));
    let knob = |i: usize| ((i % w) as f32 - 795.0).hypot((i / w) as f32 - 510.0) < 10.0;
    let hole: Vec<bool> = (0..w * h).map(|i| line(i) || knob(i)).collect();
    let area = auto_sampling(w, h, 3, &img, &hole);
    let around_knob = (0..w * h).filter(|&i| !hole[i] && ((i % w) as f32 - 795.0).hypot((i / w) as f32 - 510.0) < 24.0);
    let (n, kept) = around_knob.fold((0, 0), |(n, k), i| (n + 1, k + usize::from(area[i])));
    assert!(kept as f32 > 0.9 * n as f32, "{kept} of {n} pixels around the knob");
    assert!(thin_band(w, h, &hole) >= 20.0, "{}", thin_band(w, h, &hole));
    let disc: Vec<bool> = (0..w * h).map(|i| ((i % w) as f32 - 450.0).hypot((i / w) as f32 - 300.0) < 90.0).collect();
    assert_eq!(thin_band(w, h, &disc), 1.5);
}

#[test]
fn without_colour_adaptation_the_fill_copies_pixels_exactly() {
    // A smooth texture with fine noise, so that every value is unique: a filled pixel equal to
    // an image pixel was copied from it. Photoshop's fill (colour adaptation None) is such
    // copies, joined by narrow seams.
    let (w, h) = (120, 120);
    let fine = noise(w, h, 9);
    let img: Vec<f32> = (0..w * h * 3)
        .map(|i| {
            let (x, y, c) = (((i / 3) % w) as f32, ((i / 3) / w) as f32, i % 3);
            0.5 + 0.2 * (x * 0.21 + c as f32).sin() * (y * 0.17).cos() + 0.1 * (x * 0.05 - y * 0.08).sin() + (fine[i] - 0.5) * 0.08
        })
        .collect();
    let hole: Vec<bool> = (0..w * h).map(|i| (40..80).contains(&(i % w)) && (40..80).contains(&(i / w))).collect();
    // Votes of identical copies average to the copy up to float rounding.
    let key = |v: &[f32], i: usize| [0, 1, 2].map(|c| (v[i * 3 + c] * 1e5).round() as i64);
    let known: std::collections::HashSet<[i64; 3]> = (0..w * h).filter(|i| !hole[*i]).map(|i| key(&img, i)).collect();
    let out = fill(w, h, 3, &img, &hole, &vec![true; w * h], &FillOptions { color_adaptation: Adapt::NONE, ..Default::default() });
    let exact = (0..w * h).filter(|i| hole[*i] && known.contains(&key(&out, *i))).count();
    assert!(exact as f32 > 0.2 * 1600.0, "{exact} of 1600 filled pixels are exact copies");
    // And the fill is as busy as the noise around it, not a blur of it.
    let lap = |v: &[f32], x: usize, y: usize| {
        let p = |x: usize, y: usize| v[(y * w + x) * 3];
        (4.0 * p(x, y) - p(x - 1, y) - p(x + 1, y) - p(x, y - 1) - p(x, y + 1)).abs()
    };
    let mean = |inside: bool| {
        let pts: Vec<(usize, usize)> = (1..h - 1).flat_map(|y| (1..w - 1).map(move |x| (x, y))).filter(|&(x, y)| hole[y * w + x] == inside).collect();
        pts.iter().map(|&(x, y)| lap(&out, x, y)).sum::<f32>() / pts.len() as f32
    };
    assert!(mean(true) > 0.6 * mean(false), "fill detail {} against {}", mean(true), mean(false));
}

#[test]
fn patch_color_shifts_up_to_about_four_hundredths_per_step() {
    // Content 0.4 brighter than where it lands (Photoshop's Patch, Color 3 and 10).
    let from: Vec<f32> = (0..300).map(|i| 0.7 + 0.05 * ((i % 7) as f32 / 7.0)).collect();
    let to: Vec<f32> = from.iter().map(|v| v - 0.4).collect();
    let level = from.iter().sum::<f32>() / 300.0;
    let shift = |c: u8| {
        let gb = level_transform(1, &from, &to, color_amount(c));
        level - from.iter().map(|v| gb[0].0 * v + gb[0].1).sum::<f32>() / 300.0
    };
    assert_eq!(shift(0), 0.0);
    assert!((shift(3) - 0.12).abs() < 0.01, "{}", shift(3));
    assert!((shift(10) - 0.4).abs() < 0.01, "{}", shift(10));
}

/// A soft background (gradients, broad out-of-focus blobs of colour, fine noise) with thin, long bright
/// objects over it: three coiled strings 3 to 7 px wide and two straighter ones, each with an
/// anti-aliased edge and a faint halo. Returns the image; what removing the coils should give
/// (the background and the straighter strings, which stay); and the selection a user would draw
/// around the coils, `margin` px wide.
fn strings_scene(w: usize, h: usize, margin: f32) -> (Vec<f32>, Vec<f32>, Vec<bool>) {
    let fine = noise(w, h, 11);
    // Out-of-focus colour: broad blobs over two gradients.
    let blob = noise(3, 3, 5);
    let bg: Vec<f32> = (0..w * h * 3)
        .map(|i| {
            let (x, y, c) = (((i / 3) % w) as f32 / w as f32, ((i / 3) / w) as f32 / h as f32, i % 3);
            let mut v = [0.18 + 0.25 * x, 0.22 + 0.1 * y, 0.3 - 0.1 * x][c];
            for b in blob.as_chunks::<3>().0 {
                let (bx, by, s) = (b[0] * 1.6 - 0.3, b[1] * 1.6 - 0.3, 0.1 + 0.25 * b[2]);
                v += (b[(c + 1) % 3] - 0.2) * 0.5 * (-((x - bx).powi(2) + (y - by).powi(2) * 0.56) / (2.0 * s * s)).exp();
            }
            v + (fine[i] - 0.5) * 0.03
        })
        .collect();
    // Signed distance to each object's surface, stamped from points along it.
    let stamp = |sd: &mut [f32], pts: &mut dyn Iterator<Item = (f32, f32)>, half: f32| {
        for (px, py) in pts {
            let reach = (half + 4.0) as i32 + 1;
            for dy in -reach..=reach {
                for dx in -reach..=reach {
                    let (x, y) = (px as i32 + dx, py as i32 + dy);
                    if x >= 0 && y >= 0 && (x as usize) < w && (y as usize) < h {
                        let i = y as usize * w + x as usize;
                        sd[i] = sd[i].min((x as f32 - px).hypot(y as f32 - py) - half);
                    }
                }
            }
        }
    };
    let mut coils = vec![f32::INFINITY; w * h];
    for (k, half) in [1.5f32, 2.5, 3.5].into_iter().enumerate() {
        let (y0, r) = ((0.22 + 0.28 * k as f32) * h as f32, 10.0 + 2.0 * k as f32);
        let mut pts = (0..4000).map(|j| {
            let t = j as f32 / 4000.0;
            let a = 8.0 * std::f32::consts::TAU * t;
            (18.0 + t * (w as f32 - 60.0) + r * a.cos(), y0 + 6.0 * (6.0 * t).sin() + 0.7 * r * a.sin())
        });
        stamp(&mut coils, &mut pts, half);
    }
    let mut straight = vec![f32::INFINITY; w * h];
    for y in [0.07, 0.93] {
        stamp(&mut straight, &mut (0..2000).map(|j| (j as f32 / 2000.0 * w as f32, y * h as f32 + 3.0 * (j as f32 / 300.0).sin())), 2.0);
    }
    let draw = |base: &[f32], sd: &[f32]| -> Vec<f32> {
        let mut img = base.to_vec();
        for i in 0..w * h {
            let cover = (0.5 - sd[i]).clamp(0.0, 1.0);
            let halo = if sd[i] > 0.0 { 0.08 * (-sd[i] * sd[i] / 2.9).exp() } else { 0.0 };
            for (c, metal) in [0.9f32, 0.85, 0.7].into_iter().enumerate() {
                let v = &mut img[i * 3 + c];
                *v = *v * (1.0 - cover) + metal * cover + halo * (1.0 - cover);
            }
        }
        img
    };
    let clean = draw(&bg, &straight);
    let both: Vec<f32> = coils.iter().zip(&straight).map(|(a, b)| a.min(*b)).collect();
    let img = draw(&bg, &both);
    let hole = coils.iter().map(|d| *d <= margin).collect();
    (img, clean, hole)
}

/// RMS over the hole of the error averaged over 7 × 7 pixels (in levels of 255): the noise
/// cancels out, a ghost of the removed outline or a blotch of another shade does not.
fn blotch_error(w: usize, h: usize, out: &[f32], clean: &[f32], hole: &[bool]) -> f32 {
    let (mut sum, mut n) = (0.0f64, 0usize);
    for y in 3..h - 3 {
        for x in 3..w - 3 {
            if !hole[y * w + x] {
                continue;
            }
            let mut e = [0.0f32; 3];
            for j in (y - 3..=y + 3).flat_map(|yy| (x - 3..=x + 3).map(move |xx| yy * w + xx)) {
                for (c, ec) in e.iter_mut().enumerate() {
                    *ec += out[j * 3 + c] - clean[j * 3 + c];
                }
            }
            sum += f64::from(e.iter().map(|v| (v / 49.0).powi(2)).sum::<f32>() / 3.0);
            n += 1;
        }
    }
    ((sum / n.max(1) as f64).sqrt() * 255.0) as f32
}

/// Removing thin, long objects (guitar strings, wires) from a soft background (#1366): the
/// fill is the background, with no ghost of their outline and nothing of their colour copied
/// back in. Their selection is long but only a few pixels thick, so the fill is worked out at
/// a scale where it is still a few pixels thick rather than one where the strings' surroundings
/// have merged into a blur.
#[test]
fn thin_objects_leave_no_ghost_and_none_of_their_colour() {
    let (w, h) = (360, 270);
    let (img, clean, hole) = strings_scene(w, h, 2.0);
    let source = auto_sampling(w, h, 3, &img, &hole);
    let out = fill(w, h, 3, &img, &hole, &source, &FillOptions::default());
    let blotch = blotch_error(w, h, &out, &clean, &hole);
    let luma = |v: &[f32], i: usize| (v[i * 3] + v[i * 3 + 1] + v[i * 3 + 2]) / 3.0;
    let bright = (0..w * h).filter(|&i| hole[i] && luma(&out, i) > luma(&clean, i) + 0.06).count();
    // Before #1366 the outlines stayed as darker and lighter blotches (13 levels of 255) and
    // 1121 pixels took the strings' colour.
    assert!(blotch < 2.5, "ghosts and blotches: {blotch:.2} levels of 255");
    assert_eq!(bright, 0, "filled pixels with the strings' colour");
}

#[test]
fn variant_geometry() {
    let img = vec![0.5f32; 10 * 6];
    let ok = vec![true; 60];
    let src = Src { w: 10, h: 6, ch: 1, img: &img, ok: &ok };
    let v = variant(src, 90.0, 1.0, false, &Interrupt::NONE).unwrap();
    assert_eq!((v.w, v.h), (6, 10));
    let v = variant(src, 0.0, 0.8, true, &Interrupt::NONE).unwrap();
    assert_eq!((v.w, v.h), (8, 5));
    assert!(v.ok.iter().filter(|o| **o).count() > 20);
}
