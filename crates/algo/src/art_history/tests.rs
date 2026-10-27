use super::*;

fn fill(w: usize, h: usize, ch: usize, f: impl Fn(i32, i32, usize) -> f32) -> Vec<f32> {
    let mut v = vec![0.0; w * h * ch];
    for y in 0..h {
        for x in 0..w {
            for c in 0..ch {
                v[(y * w + x) * ch + c] = f(x as i32, y as i32, c);
            }
        }
    }
    v
}

fn horizontal(len: f32) -> Vec<[f32; 2]> {
    vec![[10.0, 32.0], [10.0 + len, 32.0]]
}

#[test]
fn tight_short_emits_more_dabs_than_tight_long() {
    let pts = horizontal(100.0);
    let short = place_dabs(&pts, 10.0, 50.0, Style::TightShort, RNG_SEED).unwrap();
    let long = place_dabs(&pts, 10.0, 50.0, Style::TightLong, RNG_SEED).unwrap();
    assert!(short.len() > long.len(), "short {} long {}", short.len(), long.len());
    let s = spacing_px(10.0, 50.0, Style::TightShort).unwrap();
    let l = spacing_px(10.0, 50.0, Style::TightLong).unwrap();
    assert!(s < l, "{s} {l}");
}

#[test]
fn tight_curl_angles_increase_and_loose_curl_scatters() {
    let pts = horizontal(80.0);
    let tight = place_dabs(&pts, 12.0, 50.0, Style::TightCurl, RNG_SEED).unwrap();
    let loose = place_dabs(&pts, 12.0, 50.0, Style::LooseCurl, RNG_SEED).unwrap();
    assert!(tight.len() >= 3 && loose.len() >= 3);
    let dt: Vec<f32> = tight.windows(2).map(|w| w[1].theta - w[0].theta).collect();
    assert!(dt.iter().all(|d| *d > 0.05), "tight curl steps {dt:?}");
    let dl: Vec<f32> = loose.windows(2).map(|w| w[1].theta - w[0].theta).collect();
    let mean = |v: &[f32]| v.iter().sum::<f32>() / v.len() as f32;
    assert!(mean(&dl) > mean(&dt), "loose curl {dl:?} vs tight {dt:?}");
    let scatter_t: f32 = tight.iter().map(|d| (d.y - 32.0).abs()).sum();
    let scatter_l: f32 = loose.iter().map(|d| (d.y - 32.0).abs()).sum();
    assert!(scatter_l > scatter_t, "loose scatter {scatter_l} tight {scatter_t}");
}

#[test]
fn seeded_scatter_is_replayable() {
    let pts = horizontal(60.0);
    let a = place_dabs(&pts, 16.0, 40.0, Style::LooseMedium, RNG_SEED ^ 3).unwrap();
    let b = place_dabs(&pts, 16.0, 40.0, Style::LooseMedium, RNG_SEED ^ 3).unwrap();
    assert_eq!(a, b);
    let c = place_dabs(&pts, 16.0, 40.0, Style::LooseMedium, RNG_SEED ^ 4).unwrap();
    assert_ne!(a, c);
}

fn fixture(ch: usize) -> (usize, usize, Vec<f32>, Vec<f32>, Dab) {
    let (w, h) = (128usize, 96usize);
    let src = fill(w, h, ch, |x, y, c| if c + 1 == ch && ch > 1 { 1.0 } else { ((x * 13 + y * 7 + c as i32 * 3).rem_euclid(251) as f32) / 255.0 });
    let dest0 = fill(w, h, ch, |_, _, c| if c + 1 == ch && ch > 1 { 1.0 } else { 0.1 });
    // Centre 1 px inside a 64-tile so the rotated read crosses the seam.
    let dab = Dab { x: 63.0, y: 40.0, theta: 0.4, radius: 18.0, pressure: 1.0 };
    (w, h, src, dest0, dab)
}

#[allow(clippy::too_many_arguments)]
fn stamp_run(w: usize, h: usize, ch: usize, src: &[f32], dest: &mut [f32], dab: &Dab, tile: i32) -> u32 {
    stamp_dabs(w, h, ch, src, dest, std::slice::from_ref(dab), tile)
}

#[allow(clippy::too_many_arguments)]
fn stamp_dabs(w: usize, h: usize, ch: usize, src: &[f32], dest: &mut [f32], dabs: &[Dab], tile: i32) -> u32 {
    let mut p = StampParams {
        dest,
        dest_w: w,
        dest_h: h,
        dest_origin: (0, 0),
        src,
        src_w: w,
        src_h: h,
        src_origin: (0, 0),
        ch,
        dabs,
        hardness: 1.0,
        opacity: 1.0,
        flow: 1.0,
        tolerance: 0.0,
        tile,
        selection: None,
        lock_transparent: false,
        alpha: (ch > 1).then_some(ch - 1),
    };
    stamp(&mut p).unwrap()
}

fn max_abs(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).fold(0.0f32, |m, (x, y)| m.max((x - y).abs()))
}

fn max_u8(a: &[f32], b: &[f32]) -> i32 {
    a.iter()
        .zip(b)
        .map(|(x, y)| {
            let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as i32;
            (q(*x) - q(*y)).abs()
        })
        .max()
        .unwrap_or(0)
}

#[test]
fn tiled_stamp_matches_untiled_at_a_tile_seam() {
    let (w, h, src, dest0, dab) = fixture(4);
    let mut tiled = dest0.clone();
    let mut flat = dest0.clone();
    let n_t = stamp_run(w, h, 4, &src, &mut tiled, &dab, 64);
    let n_f = stamp_run(w, h, 4, &src, &mut flat, &dab, 0);
    assert!(n_t > 0 && n_f > 0, "wrote {n_t} tiled / {n_f} flat");
    assert_eq!(max_u8(&tiled, &flat), 0, "8-bit seam");
    assert!(max_abs(&tiled, &flat) <= 1.0 / 32_767.0, "16-bit seam {}", max_abs(&tiled, &flat));
    assert!(max_abs(&tiled, &flat) <= 1.0e-5, "32f seam {}", max_abs(&tiled, &flat));

    let (w, h, src, dest0, dab) = fixture(1);
    let mut tiled = dest0.clone();
    let mut flat = dest0;
    stamp_run(w, h, 1, &src, &mut tiled, &dab, 64);
    stamp_run(w, h, 1, &src, &mut flat, &dab, 0);
    assert_eq!(max_u8(&tiled, &flat), 0, "gray 8-bit seam");
}

#[test]
fn leftward_and_curl_strokes_match_untiled_across_a_tile_edge() {
    let (w, h, ch) = (128usize, 96usize, 4);
    let src = fill(w, h, ch, |x, y, c| if c == 3 { 1.0 } else { ((x * 13 + y * 7 + c as i32 * 3).rem_euclid(251) as f32) / 255.0 });
    let dest0 = fill(w, h, ch, |_, _, c| if c == 3 { 1.0 } else { 0.1 });
    for style in [Style::TightMedium, Style::TightCurl, Style::LooseCurl] {
        let dabs = place_dabs(&[[110.0, 40.0], [20.0, 40.0]], 40.0, 50.0, style, RNG_SEED).unwrap();
        let mut tiled = dest0.clone();
        let mut flat = dest0.clone();
        stamp_dabs(w, h, ch, &src, &mut tiled, &dabs, 64);
        stamp_dabs(w, h, ch, &src, &mut flat, &dabs, 0);
        assert_eq!(max_abs(&tiled, &flat), 0.0, "{} seam at x = 64", style.as_str());
    }
}

/// A dab at angle 0 copies the history pixel under each covered pixel, not a blend of it
/// and its lower-right neighbours.
#[test]
fn unrotated_dab_restores_the_source_pixel() {
    let (w, h, ch) = (48usize, 48usize, 4);
    let src = fill(w, h, ch, |x, y, c| if c == 3 { 1.0 } else { ((x * 37 + y * 11 + c as i32 * 5).rem_euclid(97) as f32) / 96.0 });
    let mut dest = fill(w, h, ch, |_, _, c| if c == 3 { 1.0 } else { 0.5 });
    let dab = Dab { x: 24.0, y: 24.0, theta: 0.0, radius: 8.0, pressure: 1.0 };
    stamp_run(w, h, ch, &src, &mut dest, &dab, 0);
    for (x, y) in [(24usize, 24usize), (20, 22), (27, 26)] {
        let i = (y * w + x) * ch;
        assert_eq!(&dest[i..i + ch], &src[i..i + ch], "pixel {x},{y}");
    }
}

#[test]
fn halo_covers_rotated_read_reach() {
    let size = 40.0;
    for style in [Style::TightShort, Style::TightMedium, Style::TightLong, Style::LooseMedium, Style::LooseLong, Style::Dab, Style::TightCurl, Style::LooseCurl]
    {
        let halo = halo_radius(size, style).unwrap();
        let side = sample_side(size, style).unwrap();
        let reach = (side + 1) / 2;
        assert!(halo >= reach, "{:?}: halo {halo} < reach {reach} (side {side})", style.as_str());
    }
}

#[test]
fn tolerance_skips_an_unchanged_layer() {
    let (w, h, ch) = (48usize, 48usize, 4);
    let src = fill(w, h, ch, |x, y, c| if c == 3 { 1.0 } else { (x + y) as f32 / 100.0 });
    let mut dest = src.clone();
    let dab = Dab { x: 24.0, y: 24.0, theta: 0.0, radius: 8.0, pressure: 1.0 };
    let mut p = StampParams {
        dest: &mut dest,
        dest_w: w,
        dest_h: h,
        dest_origin: (0, 0),
        src: &src,
        src_w: w,
        src_h: h,
        src_origin: (0, 0),
        ch,
        dabs: std::slice::from_ref(&dab),
        hardness: 1.0,
        opacity: 1.0,
        flow: 1.0,
        tolerance: 1.0,
        tile: 0,
        selection: None,
        lock_transparent: false,
        alpha: Some(3),
    };
    let n = stamp(&mut p).unwrap();
    assert_eq!(n, 0);
    assert_eq!(dest, src);
}

#[test]
fn tolerance_zero_writes_where_history_differs() {
    let (w, h, ch) = (48usize, 48usize, 4);
    let src = fill(w, h, ch, |_, _, c| if c == 3 { 1.0 } else { 0.9 });
    let mut dest = fill(w, h, ch, |_, _, c| if c == 3 { 1.0 } else { 0.1 });
    let dab = Dab { x: 24.0, y: 24.0, theta: 0.0, radius: 8.0, pressure: 1.0 };
    let mut p = StampParams {
        dest: &mut dest,
        dest_w: w,
        dest_h: h,
        dest_origin: (0, 0),
        src: &src,
        src_w: w,
        src_h: h,
        src_origin: (0, 0),
        ch,
        dabs: std::slice::from_ref(&dab),
        hardness: 1.0,
        opacity: 1.0,
        flow: 1.0,
        tolerance: 0.0,
        tile: 0,
        selection: None,
        lock_transparent: false,
        alpha: Some(3),
    };
    let n = stamp(&mut p).unwrap();
    assert!(n > 0);
    let i = (24 * w + 24) * ch;
    assert!((dest[i] - 0.9).abs() < 1.0e-5, "{}", dest[i]);
}

#[test]
fn tolerance_ignores_unchanged_alpha() {
    // Colour moved by 0.3 and alpha did not: the 0.25 threshold must not see a 0.225 average.
    let (w, h, ch) = (48usize, 48usize, 4);
    let src = fill(w, h, ch, |_, _, c| if c == 3 { 1.0 } else { 0.6 });
    let dest0 = fill(w, h, ch, |_, _, c| if c == 3 { 1.0 } else { 0.3 });
    let dab = Dab { x: 24.0, y: 24.0, theta: 0.0, radius: 8.0, pressure: 1.0 };
    for (tolerance, paints) in [(0.25, true), (0.35, false)] {
        let mut dest = dest0.clone();
        let mut p = StampParams {
            dest: &mut dest,
            dest_w: w,
            dest_h: h,
            dest_origin: (0, 0),
            src: &src,
            src_w: w,
            src_h: h,
            src_origin: (0, 0),
            ch,
            dabs: std::slice::from_ref(&dab),
            hardness: 1.0,
            opacity: 1.0,
            flow: 1.0,
            tolerance,
            tile: 0,
            selection: None,
            lock_transparent: false,
            alpha: Some(3),
        };
        let n = stamp(&mut p).unwrap();
        assert_eq!(n > 0, paints, "tolerance {tolerance}");
    }
}

/// The Eraser keeps the colour and lowers only the alpha: an erased patch differs from the
/// history state and is painted back with a tolerance above 0.
#[test]
fn tolerance_sees_an_erased_patch() {
    let (w, h, ch) = (48usize, 48usize, 4);
    let src = fill(w, h, ch, |_, _, c| if c == 0 || c == 3 { 1.0 } else { 0.0 });
    let mut dest = fill(w, h, ch, |_, _, c| match c {
        0 => 1.0,
        3 => 0.6,
        _ => 0.0,
    });
    let dab = Dab { x: 24.0, y: 24.0, theta: 0.0, radius: 8.0, pressure: 1.0 };
    let mut p = StampParams {
        dest: &mut dest,
        dest_w: w,
        dest_h: h,
        dest_origin: (0, 0),
        src: &src,
        src_w: w,
        src_h: h,
        src_origin: (0, 0),
        ch,
        dabs: std::slice::from_ref(&dab),
        hardness: 1.0,
        opacity: 1.0,
        flow: 1.0,
        tolerance: 0.25,
        tile: 0,
        selection: None,
        lock_transparent: false,
        alpha: Some(3),
    };
    assert!(stamp(&mut p).unwrap() > 0, "alpha 1 -> 0.6 is over tolerance 0.25");
    let i = (24 * w + 24) * ch;
    assert!((dest[i + 3] - 1.0).abs() < 1.0e-5, "alpha restored, got {}", dest[i + 3]);
}
#[test]
fn empty_or_mismatched_buffers_error() {
    let mut dest = vec![0.0f32; 16];
    let src = vec![0.0f32; 16];
    let dab = Dab { x: 1.0, y: 1.0, theta: 0.0, radius: 1.0, pressure: 1.0 };
    let mut p = StampParams {
        dest: &mut dest,
        dest_w: 0,
        dest_h: 4,
        dest_origin: (0, 0),
        src: &src,
        src_w: 4,
        src_h: 4,
        src_origin: (0, 0),
        ch: 1,
        dabs: std::slice::from_ref(&dab),
        hardness: 1.0,
        opacity: 1.0,
        flow: 1.0,
        tolerance: 0.0,
        tile: 0,
        selection: None,
        lock_transparent: false,
        alpha: None,
    };
    assert_eq!(stamp(&mut p), Err(StampError::Empty));
    let mut dest = vec![0.0f32; 16];
    let src = vec![0.0f32; 8];
    let mut p = StampParams {
        dest: &mut dest,
        dest_w: 4,
        dest_h: 4,
        dest_origin: (0, 0),
        src: &src,
        src_w: 2,
        src_h: 2,
        src_origin: (0, 0),
        ch: 4,
        dabs: std::slice::from_ref(&dab),
        hardness: 1.0,
        opacity: 1.0,
        flow: 1.0,
        tolerance: 0.0,
        tile: 0,
        selection: None,
        lock_transparent: false,
        alpha: None,
    };
    assert!(stamp(&mut p).is_err());
    assert!(place_dabs(&[], 10.0, 50.0, Style::TightMedium, 0).is_err());
    assert!(Style::parse("nope").is_none());
    assert_eq!(Style::parse("tightMedium"), Some(Style::TightMedium));
}

#[test]
fn two_stamps_with_the_same_seed_match() {
    let pts = horizontal(50.0);
    let dabs = place_dabs(&pts, 12.0, 50.0, Style::LooseCurl, RNG_SEED).unwrap();
    let (w, h, ch) = (80usize, 64usize, 4);
    let src = fill(w, h, ch, |x, y, c| if c == 3 { 1.0 } else { (x + 3 * y) as f32 / 200.0 });
    let dest0 = fill(w, h, ch, |_, _, c| if c == 3 { 1.0 } else { 0.2 });
    let run = |dest: &mut [f32]| {
        let mut p = StampParams {
            dest,
            dest_w: w,
            dest_h: h,
            dest_origin: (0, 0),
            src: &src,
            src_w: w,
            src_h: h,
            src_origin: (0, 0),
            ch,
            dabs: &dabs,
            hardness: 0.8,
            opacity: 1.0,
            flow: 1.0,
            tolerance: 0.0,
            tile: 64,
            selection: None,
            lock_transparent: false,
            alpha: Some(3),
        };
        stamp(&mut p).unwrap()
    };
    let mut a = dest0.clone();
    let mut b = dest0;
    assert_eq!(run(&mut a), run(&mut b));
    assert_eq!(a, b);
}
