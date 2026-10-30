use super::*;

/// Runs `f` with every parallel pass on this thread using `n` threads (1 = in order), whatever its size.
fn with_threads<R>(n: usize, f: impl FnOnce() -> R) -> R {
    struct Restore(Option<usize>);
    impl Drop for Restore {
        fn drop(&mut self) {
            THREADS.with(|t| t.set(self.0));
        }
    }
    let _restore = Restore(THREADS.with(|t| t.replace(Some(n))));
    f()
}

fn bits(v: &[f32]) -> Vec<u32> {
    v.iter().map(|x| x.to_bits()).collect()
}

#[test]
fn edt_is_exact() {
    let (w, h) = (7, 5);
    let mut inside = vec![false; w * h];
    inside[2 * w + 3] = true;
    let d = edt(&inside, w, h);
    assert_eq!(d[2 * w + 3], 0.0);
    assert!((d[2 * w + 6] - 3.0).abs() < 1e-6);
    assert!((d[0] - (9.0f32 + 4.0).sqrt()).abs() < 1e-5);
    assert!((d[4 * w + 5] - (4.0f32 + 4.0).sqrt()).abs() < 1e-5);
}

#[test]
fn edt_two_seeds() {
    let (w, h) = (10, 1);
    let mut inside = vec![false; w];
    inside[0] = true;
    inside[9] = true;
    let d = edt(&inside, w, h);
    assert_eq!(d, vec![0.0, 1.0, 2.0, 3.0, 4.0, 4.0, 3.0, 2.0, 1.0, 0.0]);
}

#[test]
fn blur_preserves_mass() {
    let mut m = Map::new(21, 21, 0.0);
    m.v[10 * 21 + 10] = 1.0;
    blur(&mut m, 6.0);
    let sum: f32 = m.v.iter().sum();
    assert!((sum - 1.0).abs() < 1e-4, "{sum}");
    assert!(m.v[10 * 21 + 10] < 1.0);
}

#[test]
fn gradient_geometry() {
    let b = Rect::new(0, 0, 100, 100);
    let t = |s, x, y| gradient_t(s, 0.0, 1.0, false, (0.0, 0.0), b, x, y);
    assert!(t(GradientStyle::Linear, 0.0, 50.0) < 0.01);
    // Linear / reflected sample pixel corners: the centre is reached half a pixel later.
    assert!((t(GradientStyle::Linear, 50.5, 50.0) - 0.5).abs() < 1e-6);
    assert!(t(GradientStyle::Linear, 100.0, 50.0) > 0.99);
    assert!(t(GradientStyle::Reflected, 50.5, 50.0) < 1e-6);
    assert!((t(GradientStyle::Reflected, 0.0, 50.0) - 1.0).abs() < 1e-6);
    assert!(t(GradientStyle::Radial, 50.0, 50.0) < 1e-6);
    assert!((t(GradientStyle::Radial, 50.0, 0.0) - 1.0).abs() < 1e-6);
    assert!((t(GradientStyle::Diamond, 75.0, 75.0) - 1.0).abs() < 1e-6);
    let ang = t(GradientStyle::Angle, 50.0, 0.0);
    assert!((ang - 0.75).abs() < 1e-3, "{ang}");
    let rev = gradient_t(GradientStyle::Linear, 0.0, 1.0, true, (0.0, 0.0), b, 0.0, 50.0);
    assert!(rev > 0.99);
}

#[test]
fn effect_gradients_snap_end_points_and_sample_corners() {
    // 87° over 600 × 60 at (99, 422): end points (397, 482) and (400, 422), so the overlay runs
    // along (3, -60) from its midpoint (398.5, 452), sampled at pixel corners.
    let frame = Rect::new(99, 422, 699, 482);
    let (a, sc, o) = crate::fill_layout::gradient_layout(GradientStyle::Linear, 87.0, 1.0, (0.0, 0.0), frame);
    assert!((a - (60f32).atan2(3.0).to_degrees()).abs() < 1e-3, "{a}");
    let t = |x: f32, y: f32| gradient_t(GradientStyle::Linear, a, sc, false, o, frame, x, y);
    assert!((t(399.0, 452.5) - 0.5).abs() < 1e-4, "{}", t(399.0, 452.5));
    assert!(t(400.5, 422.5) > 0.99 && t(397.5, 482.5) < 0.01);
}

#[test]
fn chamfer_uses_euclidean_step_lengths() {
    let (w, h) = (9, 9);
    let mut inside = vec![false; w * h];
    inside[0] = true;
    let (d, near) = chamfer_nearest(&inside, w, h);
    let at = |x: usize, y: usize| d[y * w + x];
    assert!((at(3, 0) - 3.0).abs() < 1e-5);
    assert!((at(2, 2) - 2.0 * std::f32::consts::SQRT_2).abs() < 1e-5);
    assert!((at(1, 2) - 5f32.sqrt()).abs() < 1e-5);
    assert!((at(1, 3) - (5f32.sqrt() + 1.0)).abs() < 1e-5, "knight + straight, not √10");
    assert!((at(2, 3) - (5f32.sqrt() + std::f32::consts::SQRT_2)).abs() < 1e-5);
    assert!(near.iter().all(|&n| n == 0));
}

#[test]
fn spread_dilates_with_the_chamfer_metric() {
    // One opaque pixel grown by 3 px: at offset (1, 3) the chamfer distance is √5 + 1 = 3.236
    // (Euclidean 3.162), so the anti-aliased rim keeps 3 + 0.5 − (3.236 − 0.5) = 0.764.
    let mut m = Map::new(9, 9, 0.0);
    m.v[4 * 9 + 4] = 1.0;
    let d = dilate(&m, 3.0);
    assert!((d.v[7 * 9 + 5] - 0.764).abs() < 1e-3, "{}", d.v[7 * 9 + 5]);
    // (2, 3): √5 + √2 = 3.650 (Euclidean 3.606) leaves 0.350; axis steps are exact.
    assert!((d.v[7 * 9 + 6] - (4.0 - 5f32.sqrt() - std::f32::consts::SQRT_2)).abs() < 1e-4, "{}", d.v[7 * 9 + 6]);
    assert_eq!(d.v[4 * 9 + 7], 1.0);
}

#[test]
fn running_sum_tent_matches_the_kernel() {
    let (w, h) = (37usize, 23usize);
    let src: Vec<f32> = (0..w * h).map(|i| ((i * 7919) % 101) as f32 / 100.0).collect();
    for width in [1.0f32, 2.0, 3.5, 5.0, 8.25, 41.0] {
        let mut m = Map { w, h, v: src.clone() };
        tent(&mut m, width);
        let mut want = Map { w, h, v: src.clone() };
        tent_direct(&mut want, width);
        for (a, b) in m.v.iter().zip(&want.v) {
            assert!((a - b).abs() < 1e-5, "width {width}: {a} vs {b}");
        }
    }
}

#[test]
fn tent_kernel_is_two_boxes() {
    let (r, k) = tent_kernel(5.0);
    assert_eq!(r, 4);
    let sum: f32 = k.iter().sum();
    assert!((sum - 1.0).abs() < 1e-6);
    assert!((k[4] - 5.0 / 25.0).abs() < 1e-6 && (k[0] - 1.0 / 25.0).abs() < 1e-6);
    // Fractional widths taper the end taps; width 1 is the identity.
    let (r, k) = tent_kernel(4.0);
    assert_eq!(r, 4);
    assert!(k[0] > 0.0 && k[0] < 1.0 / 25.0);
    assert_eq!(tent_kernel(1.0), (0, vec![1.0]));
    // #1543: a huge, infinite or NaN width is capped, not turned into a kernel of 2^62 taps.
    for w in [1e30, f32::INFINITY, f32::MAX] {
        assert_eq!(tent_kernel(w), tent_kernel(MAX_REACH), "{w}");
    }
    assert_eq!(tent_kernel(f32::NAN), (0, vec![1.0]));
}

fn no_tex() -> TextureCtx<'static> {
    TextureCtx { rect: Rect::new(0, 0, 1, 1), patterns: &[], anchor: (0.0, 0.0) }
}

fn square(n: usize, x0: usize, x1: usize) -> Map {
    let mut m = Map::new(n, n, 0.0);
    for y in x0..x1 {
        for x in x0..x1 {
            m.v[y * n + x] = 1.0;
        }
    }
    m
}

fn bevel_of(style: BevelStyle, technique: BevelTechnique) -> Bevel {
    Bevel {
        enabled: true,
        style,
        technique,
        depth: 1.0,
        up: true,
        size: 5.0,
        soften: 0.0,
        angle: 90.0,
        altitude: 30.0,
        use_global_light: false,
        gloss_contour: Contour::Linear,
        gloss_anti_alias: false,
        highlight: photocraft_doc::FxCommon::new(BlendMode::Screen, 0.75),
        highlight_color: photocraft_color::Color::WHITE,
        shadow: photocraft_doc::FxCommon::new(BlendMode::Multiply, 0.75),
        shadow_color: photocraft_color::Color::BLACK,
        contour: None,
        texture: None,
    }
}

#[test]
fn smooth_inner_bevel_lights_the_top_edge_inside_only() {
    let shape = square(40, 10, 30);
    let (maps, paint) = bevel_maps(
        &shape,
        &bevel_of(BevelStyle::InnerBevel, BevelTechnique::Smooth),
        &GlobalLight::default(),
        &no_tex(),
        &PreparedPatterns::new(&[], PREPARED_PATTERN_BYTES),
    );
    assert_eq!(paint, BevelPaint::Inner);
    let at = |m: &Map, x: usize, y: usize| m.v[y * 40 + x];
    // Light from the top: highlight along the top edge, shadow along the bottom, flat middle.
    assert!(at(&maps[0], 20, 10) > 0.5 && at(&maps[1], 20, 10) == 0.0);
    assert!(at(&maps[1], 20, 29) > 0.5);
    assert!(at(&maps[0], 20, 20) < 1e-3 && at(&maps[1], 20, 20) < 1e-3);
    // Nothing outside the shape; the ramp reaches about `size` inside.
    assert_eq!(at(&maps[0], 20, 8), 0.0);
    assert!(at(&maps[0], 20, 15) < 0.05);
}

/// Photoshop 27.11, measured across a straight edge (size 10, depth 100 %, light from the top
/// at 30°): a chisel facet has the slope `depth × size / (2 (size + 1))`, flat right up to the
/// edge pixel for Inner Hard; Inner Soft's edge pixel is half as steep again; Outer Hard
/// still shades, partly, the pixel where its ramp reaches zero `size + 1½` px out.
#[test]
fn chisel_facets_match_photoshop() {
    let amount = |g: f32| {
        let (se, ce) = 30f32.to_radians().sin_cos();
        ((g * ce + se) / (1.0 + g * g).sqrt() - se) / (1.0 - se)
    };
    let n = 80;
    let shape = square(n, 20, 60);
    let maps = |style, technique| {
        let mut b = bevel_of(style, technique);
        b.size = 10.0;
        bevel_maps(&shape, &b, &GlobalLight::default(), &no_tex(), &PreparedPatterns::new(&[], PREPARED_PATTERN_BYTES)).0
    };
    let at = |m: &Map, y: usize| m.v[y * n + 40];
    let facet = amount(10.0 / 22.0);
    let hard = maps(BevelStyle::InnerBevel, BevelTechnique::ChiselHard);
    for y in 20..30 {
        assert!((at(&hard[0], y) - facet).abs() < 0.01, "inner hard facet at row {y}: {} vs {facet}", at(&hard[0], y));
    }
    assert!(at(&hard[0], 30) > 0.05 && at(&hard[0], 30) < facet, "one partial pixel at the end");
    let soft = maps(BevelStyle::InnerBevel, BevelTechnique::ChiselSoft);
    assert!((at(&soft[0], 20) - amount(1.5 * 10.0 / 22.0)).abs() < 0.01, "steeper edge pixel: {}", at(&soft[0], 20));
    assert!((at(&soft[0], 24) - facet).abs() < 0.01, "then Chisel Hard's facet");
    let outer = maps(BevelStyle::OuterBevel, BevelTechnique::ChiselHard);
    // Below the shape (rows 60..): shadow facet through 10½ px, partial at 11½.
    assert!((at(&outer[1], 70) - at(&outer[1], 61)).abs() < 1e-4);
    assert!(at(&outer[1], 71) > 0.05 && at(&outer[1], 71) < at(&outer[1], 70));
    assert_eq!(at(&outer[1], 72), 0.0);
}

#[test]
fn emboss_paints_both_sides_and_pillow_flips_the_outside() {
    let shape = square(40, 10, 30);
    let l = GlobalLight::default();
    let (e, paint) =
        bevel_maps(&shape, &bevel_of(BevelStyle::Emboss, BevelTechnique::Smooth), &l, &no_tex(), &PreparedPatterns::new(&[], PREPARED_PATTERN_BYTES));
    assert_eq!((paint, e.len()), (BevelPaint::Both, 2));
    let at = |m: &Map, x: usize, y: usize| m.v[y * 40 + x];
    // Emboss: the slope across the top edge faces the light on both sides.
    assert!(at(&e[0], 20, 10) > 0.3 && at(&e[0], 20, 9) > 0.3);
    let (p, _) =
        bevel_maps(&shape, &bevel_of(BevelStyle::PillowEmboss, BevelTechnique::Smooth), &l, &no_tex(), &PreparedPatterns::new(&[], PREPARED_PATTERN_BYTES));
    assert!(at(&p[0], 20, 10) > 0.3 && at(&p[1], 20, 9) > 0.3, "pillow: outside top edge in shadow");
}

#[test]
fn emboss_width_rounds_up_and_shades_past_the_blur() {
    let mut b = bevel_of(BevelStyle::Emboss, BevelTechnique::Smooth);
    b.size = 41.0;
    let g = bevel_geom(&b);
    assert_eq!((g.width, g.depth), (21.0, 21.0));
    b.size = 5.0;
    assert_eq!(bevel_geom(&b).width, 3.0);
    // The first row past the height blur's reach still slopes (its upper neighbour is raised),
    // so it shades; the row after it does not.
    let shape = square(60, 20, 40);
    b.size = 9.0;
    b.angle = 90.0;
    let (m, _) = bevel_maps(&shape, &b, &GlobalLight::default(), &no_tex(), &PreparedPatterns::new(&[], PREPARED_PATTERN_BYTES));
    let reach = tent_kernel(bevel_geom(&b).width).0 as usize;
    let row = 39 + reach + 1; // last shape row + reach + 1: height 0, neighbour above raised
    assert!(m[1].v[row * 60 + 30] > 0.0, "{}", m[1].v[row * 60 + 30]);
    assert_eq!(m[1].v[(row + 1) * 60 + 30], 0.0);
}

/// A `len` x `wid` bar through the middle of a `w` x `h` map at `deg` degrees, anti-aliased by
/// 16 x 16 supersampling (the coverage a scanline rasteriser gives).
fn aa_bar(w: usize, h: usize, deg: f32, len: f32, wid: f32) -> Map {
    let (sn, cs) = deg.to_radians().sin_cos();
    let (cx, cy) = (w as f32 / 2.0, h as f32 / 2.0);
    let mut m = Map::new(w, h, 0.0);
    const N: usize = 16;
    for y in 0..h {
        for x in 0..w {
            let mut hit = 0;
            for sy in 0..N {
                for sx in 0..N {
                    let (px, py) = (x as f32 + (sx as f32 + 0.5) / N as f32 - cx, y as f32 + (sy as f32 + 0.5) / N as f32 - cy);
                    let (u, v) = (px * cs + py * sn, -px * sn + py * cs);
                    hit += usize::from(u.abs() <= len / 2.0 && v.abs() <= wid / 2.0);
                }
            }
            m.v[y * w + x] = hit as f32 / (N * N) as f32;
        }
    }
    m
}

/// Ripple and bias of a distance field on an `aa_bar` (`inside` or outside it): the worst
/// standard deviation of the error among pixels at the same true distance from a long edge
/// (0.5 px bins, 0.6..8 px, away from the ends), and the worst mean error of a bin. A constant
/// offset is no ripple: only a field that varies along the edge shades in stripes.
fn bar_ripple(f: &[f32], w: usize, h: usize, deg: f32, wid: f32, inside: bool) -> (f32, f32) {
    let (sn, cs) = deg.to_radians().sin_cos();
    let mut bins: Vec<Vec<f32>> = vec![Vec::new(); 16];
    for y in 0..h {
        for x in 0..w {
            let (px, py) = (x as f32 + 0.5 - w as f32 / 2.0, y as f32 + 0.5 - h as f32 / 2.0);
            let (u, v) = (px * cs + py * sn, -px * sn + py * cs);
            let truth = if inside { wid / 2.0 - v.abs() } else { v.abs() - wid / 2.0 };
            if u.abs() < 40.0 && (0.6..8.0).contains(&truth) {
                bins[(truth * 2.0) as usize].push(f[y * w + x] - truth);
            }
        }
    }
    let (mut ripple, mut bias) = (0.0f32, 0.0f32);
    for b in bins.iter().filter(|b| b.len() > 8) {
        let mean = b.iter().sum::<f32>() / b.len() as f32;
        let sd = (b.iter().map(|e| (e - mean) * (e - mean)).sum::<f32>() / b.len() as f32).sqrt();
        ripple = ripple.max(sd);
        bias = bias.max(mean.abs());
    }
    (ripple, bias)
}

/// The worst standard deviation of the chisel shading (highlight minus shadow) along lines
/// parallel to an `aa_bar` 2..6 px in from its edges, the way Issue #1926 measured stripes.
fn bar_shade_ripple(deg: f32, light: f32) -> f32 {
    let (w, h) = (220usize, 220usize);
    let bar = aa_bar(w, h, deg, 150.0, 16.0);
    let mut b = bevel_of(BevelStyle::InnerBevel, BevelTechnique::ChiselHard);
    (b.size, b.depth, b.angle, b.altitude) = (8.0, 2.2, light, 32.0);
    let (m, _) = bevel_maps(&bar, &b, &GlobalLight::default(), &no_tex(), &PreparedPatterns::new(&[], PREPARED_PATTERN_BYTES));
    let (sn, cs) = deg.to_radians().sin_cos();
    let mut worst = 0.0f32;
    for o in [-6.0f32, -5.0, -4.0, -3.0, -2.0, 2.0, 3.0, 4.0, 5.0, 6.0] {
        let line: Vec<f32> = (-60..=60)
            .map(|k| {
                let t = k as f32 * 0.5;
                let (x, y) = (w as f32 / 2.0 + t * cs - o * sn, h as f32 / 2.0 + t * sn + o * cs);
                let i = y.floor() as usize * w + x.floor() as usize;
                m[0].v[i] - m[1].v[i]
            })
            .collect();
        let mean = line.iter().sum::<f32>() / line.len() as f32;
        worst = worst.max((line.iter().map(|v| (v - mean) * (v - mean)).sum::<f32>() / line.len() as f32).sqrt());
    }
    worst
}

/// Coverage of a unit pixel by the half plane `n . p + s >= 0` (`p` from the pixel's centre),
/// by 64 x 64 supersampling.
fn half_plane_coverage(n: [f32; 2], s: f32) -> f32 {
    const N: usize = 64;
    let mut hit = 0;
    for y in 0..N {
        for x in 0..N {
            let (px, py) = ((x as f32 + 0.5) / N as f32 - 0.5, (y as f32 + 0.5) / N as f32 - 0.5);
            hit += usize::from(n[0] * px + n[1] * py + s >= 0.0);
        }
    }
    hit as f32 / (N * N) as f32
}

#[test]
fn edge_offset_places_a_straight_edge_by_its_coverage() {
    for step in 0..=12 {
        let (sn, cs) = (step as f32 * 7.5).to_radians().sin_cos();
        for k in -12..=12 {
            let s = k as f32 * 0.05;
            let a = half_plane_coverage([cs, sn], s);
            // Where the edge only just touches the pixel the coverage hardly moves with it.
            if !(0.03..=0.97).contains(&a) {
                continue;
            }
            let got = edge_offset([cs, sn], a);
            assert!((got - s).abs() < 0.03, "{}deg s {s}: coverage {a} gave {got}", step as f32 * 7.5);
        }
    }
    // Axis-aligned edges keep `alpha - 1/2`; a pixel no coverage says anything about is 0.
    assert!((edge_offset([1.0, 0.0], 0.25) + 0.25).abs() < 1e-6);
    assert!((edge_offset([0.0, 1.0], 0.9) - 0.4).abs() < 1e-6);
    assert_eq!(edge_offset([0.6, 0.8], f32::NAN), 0.0);
}

#[test]
fn bevel_distances_equal_the_plain_ones_without_anti_aliasing() {
    // Hard edges (no partly covered pixel) and soft ramps (no sharp one) keep the plain fields.
    let (w, h) = (96usize, 80usize);
    let mut maps = Vec::new();
    for kind in 0..4 {
        let mut m = Map::new(w, h, 0.0);
        let mut seed = 12345u32;
        for y in 0..h {
            for x in 0..w {
                let (dx, dy) = (x as f32 - 48.0, y as f32 - 40.0);
                m.v[y * w + x] = match kind {
                    0 => f32::from((50.0..90.0).contains(&(x as f32 * 0.6 + y as f32))),
                    1 => f32::from(dx * dx + dy * dy < 30.0 * 30.0),
                    2 => {
                        seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                        f32::from(seed >> 28 < 5 && x > 10 && x < 80)
                    }
                    _ => ((20.0 - (dx * dx + dy * dy).sqrt()) / 8.0).clamp(0.0, 1.0),
                };
            }
        }
        maps.push(m);
    }
    for (i, m) in maps.iter().enumerate() {
        let sharp = sharp_edges(m);
        assert!(sharp.is_empty(), "map {i} has no anti-aliased edge");
        assert_eq!(bevel_dist_inside(m, &sharp, 1e9), dist_inside(m), "inside, map {i}");
        assert_eq!(bevel_dist_outside(m, &sharp, 1e9), dist_outside(m), "outside, map {i}");
    }
}

#[test]
fn chisel_distance_has_no_ripple_along_oblique_edges() {
    // The plain distance to a pixel's centre ripples with the staircase of an oblique edge (the
    // stripes of Issue #1926); at 0 and 45 degrees the staircase is regular and it does not.
    let (w, h) = (160, 160);
    for deg in [0.0f32, 30.0, 45.0, 60.0, 75.0, 120.0, 150.0] {
        let bar = aa_bar(w, h, deg, 120.0, 40.0);
        let sharp = sharp_edges(&bar);
        let inside = bar_ripple(&bevel_dist_inside(&bar, &sharp, 40.0), w, h, deg, 40.0, true);
        let outside = bar_ripple(&bevel_dist_outside(&bar, &sharp, 40.0), w, h, deg, 40.0, false);
        for (what, (ripple, bias)) in [("inside", inside), ("outside", outside)] {
            assert!(ripple < 0.008 && bias < 0.01, "{deg} deg {what}: ripple {ripple}, bias {bias}");
        }
    }
    let plain = aa_bar(w, h, 60.0, 120.0, 40.0);
    let (ripple, _) = bar_ripple(&dist_inside(&plain), w, h, 60.0, 40.0, true);
    assert!(ripple > 0.1, "the plain field still ripples ({ripple}), so the test above means something");
}

#[test]
fn edge_normals_follow_straight_edges_to_a_fraction_of_a_degree() {
    // The 3x3 gradient is 1-3 degrees off and the error is systematic; the line through the edge
    // points of the pixels around one is not.
    let (w, h, wid) = (160usize, 160usize, 40.0f32);
    for deg in [30.0f32, 60.0, 75.0, 120.0, 150.0] {
        let sharp = sharp_edges(&aa_bar(w, h, deg, 120.0, wid));
        let (sn, cs) = deg.to_radians().sin_cos();
        let (mut worst, mut seen) = (0.0f32, 0);
        for (i, e) in sharp.iter() {
            let (px, py) = ((i % w) as f32 + 0.5 - w as f32 / 2.0, (i / w) as f32 + 0.5 - h as f32 / 2.0);
            let (u, v) = (px * cs + py * sn, -px * sn + py * cs);
            if u.abs() > 40.0 || (wid / 2.0 - v.abs()).abs() > 1.5 {
                continue;
            }
            let into = if v > 0.0 { [sn, -cs] } else { [-sn, cs] };
            worst = worst.max((e.normal[0] * into[0] + e.normal[1] * into[1]).clamp(-1.0, 1.0).acos().to_degrees());
            seen += 1;
        }
        assert!(seen > 100 && worst < 0.6, "{deg} deg: {seen} edge pixels, normal off by up to {worst} deg");
    }
}

#[test]
fn chisel_distance_is_close_on_round_shapes() {
    // Circles (a hard stretch of edge at each pole, where the edge lies on a pixel boundary) and
    // rounded rectangles on whole pixels (straight hard edges joined to anti-aliased arcs): the
    // distance to the sharp pixels alone is up to 0.3 px too long there, so the plain distance
    // caps it. Errors are against the exact distance to the shape, 0.6..8 px from its edge.
    let worst = |f: &[f32], w: usize, h: usize, sdf: &dyn Fn(f32, f32) -> f32, inside: bool| {
        let mut mx = 0.0f32;
        for y in 0..h {
            for x in 0..w {
                let t = sdf(x as f32 + 0.5, y as f32 + 0.5);
                let truth = if inside { -t } else { t };
                if (0.6..8.0).contains(&truth) {
                    mx = mx.max((f[y * w + x] - truth).abs());
                }
            }
        }
        mx
    };
    for r in [20.0f32, 40.0, 70.0] {
        let (w, h) = (200usize, 200usize);
        let m = aa_disc(w, h, r);
        let sharp = sharp_edges(&m);
        let sdf = |x: f32, y: f32| (x - 100.0).hypot(y - 100.0) - r;
        let (i, o) = (worst(&bevel_dist_inside(&m, &sharp, 12.0), w, h, &sdf, true), worst(&bevel_dist_outside(&m, &sharp, 12.0), w, h, &sdf, false));
        assert!(i < 0.05 && o < 0.07, "disc {r}: inside off by {i}, outside by {o}");
    }
    for rad in [6.0f32, 12.0, 24.0] {
        let (w, h) = (200usize, 160usize);
        let m = aa_round_rect(w, h, 20.0, 20.0, 180.0, 140.0, rad);
        let sharp = sharp_edges(&m);
        let sdf = |x: f32, y: f32| {
            let (qx, qy) = ((x - 100.0).abs() - (80.0 - rad), (y - 80.0).abs() - (60.0 - rad));
            qx.max(0.0).hypot(qy.max(0.0)) + qx.max(qy).min(0.0) - rad
        };
        let (i, o) = (worst(&bevel_dist_inside(&m, &sharp, 12.0), w, h, &sdf, true), worst(&bevel_dist_outside(&m, &sharp, 12.0), w, h, &sdf, false));
        assert!(i < 0.08 && o < 0.2, "rounded rect, radius {rad}: inside off by {i}, outside by {o}");
    }
}

#[test]
fn chisel_facets_are_flat_along_oblique_edges() {
    // Issue #1926: the facets of a chiselled bar at 30, 60, 120, 150 degrees were striped.
    for deg in [0.0f32, 30.0, 45.0, 60.0, 75.0, 120.0, 150.0] {
        for light in [-90.0f32, 0.0, 45.0, 120.0] {
            let ripple = bar_shade_ripple(deg, light);
            assert!(ripple < 0.03, "{deg} deg bar, light {light}: shading ripple {ripple}");
        }
    }
}

#[test]
fn bevel_distances_survive_degenerate_maps() {
    let cases: [(usize, usize, f32); 7] = [(0, 0, 0.0), (1, 1, 0.5), (1, 1, 1.0), (2, 2, 0.5), (3, 1, 0.3), (5, 5, f32::NAN), (4, 4, 1.0)];
    for (w, h, a) in cases {
        let m = Map::new(w, h, a);
        let sharp = sharp_edges(&m);
        for reach in [0.0, 8.0, f32::NAN, f32::INFINITY, -3.0] {
            assert_eq!(bevel_dist_inside(&m, &sharp, reach).len(), w * h);
            assert_eq!(bevel_dist_outside(&m, &sharp, reach).len(), w * h);
        }
    }
    // A single partly covered pixel in an empty map, and an edge on the map's border.
    let mut m = Map::new(5, 5, 0.0);
    m.v[12] = 0.4;
    let _ = (bevel_dist_inside(&m, &sharp_edges(&m), 4.0), bevel_dist_outside(&m, &sharp_edges(&m), 4.0));
    let mut m = Map::new(8, 8, 0.0);
    for y in 0..8 {
        m.v[y * 8 + 7] = 1.0;
        m.v[y * 8 + 6] = 0.5;
    }
    let _ = (bevel_dist_inside(&m, &sharp_edges(&m), 4.0), bevel_dist_outside(&m, &sharp_edges(&m), 4.0));
}

#[test]
fn par_units_visits_every_unit_once_with_its_own_slices() {
    // Lengths that leave a short last unit, no second slice, units of different sizes, and nothing at all.
    let cases: [(usize, usize, usize, usize); 8] =
        [(0, 3, 0, 0), (1, 1, 1, 1), (10, 3, 0, 0), (64, 8, 64, 8), (65, 8, 13, 1), (200, 1, 200, 1), (7, 7, 14, 2), (9, 4, 0, 5)];
    for (la, ua, lb, ub) in cases {
        for threads in [1, 2, 3, 8, 64] {
            let (mut a, mut b) = (vec![0u32; la], vec![0u32; lb]);
            let calls = std::sync::atomic::AtomicUsize::new(0);
            with_threads(threads, || {
                par_units(
                    la,
                    &mut a,
                    ua,
                    &mut b,
                    ub,
                    || (),
                    |_, u, sa, sb| {
                        calls.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        assert!(sa.len() <= ua && sb.len() <= ub);
                        for (k, v) in sa.iter_mut().enumerate() {
                            *v = (u * ua + k) as u32 + 1;
                        }
                        for (k, v) in sb.iter_mut().enumerate() {
                            *v = (u * ub + k) as u32 + 1;
                        }
                    },
                );
            });
            assert_eq!(calls.into_inner(), la.div_ceil(ua), "{la} {ua} {lb} {ub} on {threads}");
            assert!(a.iter().enumerate().all(|(i, &v)| v == i as u32 + 1), "a: {la} {ua} {lb} {ub} on {threads}");
            // `b` is written for the units that exist in `a` and no further.
            let written = (la.div_ceil(ua) * ub).min(lb);
            assert!(b.iter().enumerate().all(|(i, &v)| v == if i < written { i as u32 + 1 } else { 0 }), "b: {la} {ua} {lb} {ub} on {threads}");
        }
    }
    // A unit size of 0 does nothing.
    let mut a = vec![1u8; 4];
    par_units(4, &mut a, 0, &mut [] as &mut [()], 0, || (), |_, _, _, _| panic!("no units"));
}

#[test]
fn nearest_distances_do_not_depend_on_the_threads() {
    // Sizes that do not divide into the thread groups, lines of 1 pixel, and a noisy map where
    // many pixels are equally far from two seeds (the nearest one must resolve the same way).
    for (w, h) in [(37usize, 23usize), (1, 40), (40, 1), (2, 2), (101, 3), (64, 64)] {
        let mut seed = 7u32;
        let inside: Vec<bool> = (0..w * h)
            .map(|_| {
                seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                (seed >> 24).is_multiple_of(11)
            })
            .collect();
        let (d1, n1) = with_threads(1, || edt_nearest(&inside, w, h));
        for threads in [2, 3, 8] {
            let (d, n) = with_threads(threads, || edt_nearest(&inside, w, h));
            assert_eq!((bits(&d), n), (bits(&d1), n1.clone()), "{w}x{h} on {threads} threads");
        }
        // And the nearest pixel is a seed at exactly that distance.
        for (i, (&d, &q)) in d1.iter().zip(&n1).enumerate() {
            if inside.iter().any(|&b| b) {
                let (dx, dy) = ((i % w) as f32 - (q as usize % w) as f32, (i / w) as f32 - (q as usize / w) as f32);
                assert!(inside[q as usize] && (dx.hypot(dy) - d).abs() < 1e-3, "{w}x{h} at {i}: {d} vs seed {q}");
            }
        }
    }
}

#[test]
fn nearest_distances_survive_degenerate_sizes() {
    // The slice does not match the size, or there is nothing: no panic, and a result per input pixel.
    for (len, w, h) in [(0usize, 0usize, 0usize), (0, 5, 0), (0, 0, 5), (6, 2, 2), (6, 4, 2), (1, 0, 1)] {
        let (d, n) = edt_nearest(&vec![true; len], w, h);
        assert_eq!((d.len(), n.len()), (len, len), "{len} px as {w}x{h}");
    }
}

#[test]
fn chisel_fields_do_not_depend_on_the_threads() {
    // Edge pixels at every angle, curves, patchy opacity and an edge along the border, on sizes that
    // leave short last groups; one thread (in order) is the reference.
    let mut maps = vec![
        aa_disc(97, 83, 30.0),
        aa_round_rect(131, 77, 9.0, 11.0, 120.0, 66.0, 14.0),
        aa_bar(120, 120, 33.0, 100.0, 30.0),
        aa_bar(90, 130, 60.0, 150.0, 25.0),
    ];
    let mut patchy = aa_disc(101, 101, 40.0);
    for (i, v) in patchy.v.iter_mut().enumerate() {
        if (i % 101 / 13 + i / 101 / 17) % 5 == 0 {
            *v *= 0.6;
        }
    }
    maps.push(patchy);
    maps.push(aa_bar(64, 64, 40.0, 220.0, 30.0));
    for (k, m) in maps.iter().enumerate() {
        for reach in [3.0f32, 12.0] {
            let one = with_threads(1, || (sharp_edges(m), distance_field_of(m, reach)));
            for threads in [2, 5, 16] {
                let many = with_threads(threads, || (sharp_edges(m), distance_field_of(m, reach)));
                assert_eq!(one.0.slot, many.0.slot, "map {k} sharp slots on {threads} threads");
                let edges = |s: &SharpEdges| {
                    s.pixels.iter().map(|p| (p.at, p.cov.to_bits(), p.edge.offset.to_bits(), p.edge.normal.map(f32::to_bits))).collect::<Vec<_>>()
                };
                assert_eq!(edges(&one.0), edges(&many.0), "map {k} sharp edges on {threads} threads");
                for (a, b) in one.1.iter().zip(&many.1) {
                    assert_eq!(bits(a), bits(b), "map {k}, reach {reach} on {threads} threads");
                }
            }
        }
    }
}

#[test]
fn bevel_maps_do_not_depend_on_the_threads() {
    // Every style and technique, softened or not, and a texture; one thread (in order) is the reference.
    let mut patchy = aa_disc(101, 101, 40.0);
    for (i, v) in patchy.v.iter_mut().enumerate() {
        if (i % 101 / 13 + i / 101 / 17) % 5 == 0 {
            *v *= 0.6;
        }
    }
    let shapes = [aa_disc(97, 83, 30.0), aa_round_rect(131, 77, 9.0, 11.0, 120.0, 66.0, 14.0), aa_bar(90, 130, 60.0, 150.0, 25.0), patchy];
    let mut sf = photocraft_raster::Surface::new(photocraft_color::PixelFormat::RGBA8);
    sf.fill_rect(Rect::new(0, 0, 4, 2), &[0.0, 0.0, 0.0, 1.0]);
    sf.fill_rect(Rect::new(0, 2, 4, 4), &[1.0, 1.0, 1.0, 1.0]);
    let pats = [Pattern::new("stripes", sf, 4, 4)];
    let light = GlobalLight::default();
    let styles = [BevelStyle::InnerBevel, BevelStyle::OuterBevel, BevelStyle::Emboss, BevelStyle::PillowEmboss, BevelStyle::StrokeEmboss];
    for (k, shape) in shapes.iter().enumerate() {
        let tex = TextureCtx { rect: Rect::new(3, 5, 3 + shape.w as i32, 5 + shape.h as i32), patterns: &pats, anchor: (0.0, 0.0) };
        for style in styles {
            for technique in [BevelTechnique::Smooth, BevelTechnique::ChiselHard, BevelTechnique::ChiselSoft] {
                for (size, soften, textured) in [(5.0, 0.0, false), (12.0, 2.0, false), (8.0, 0.0, true)] {
                    let mut b = bevel_of(style, technique);
                    (b.size, b.soften) = (size, soften);
                    if textured {
                        b.texture = Some(photocraft_doc::BevelTexture {
                            name: "stripes".into(),
                            id: String::new(),
                            scale: 1.0,
                            depth: 1.0,
                            invert: false,
                            link: true,
                            phase: (0.0, 0.0),
                        });
                    }
                    let run = || bevel_maps(shape, &b, &light, &tex, &PreparedPatterns::new(&pats, PREPARED_PATTERN_BYTES)).0;
                    let one = with_threads(1, run);
                    for threads in [2, 5, 16] {
                        let many = with_threads(threads, run);
                        assert_eq!(one.len(), many.len());
                        for (j, (a, m)) in one.iter().zip(&many).enumerate() {
                            assert_eq!(bits(&a.v), bits(&m.v), "shape {k}, {style:?} {technique:?} size {size} map {j} on {threads} threads");
                        }
                    }
                }
            }
        }
    }
}

/// The four fields that use the distance transform, of `m`.
fn distance_field_of(m: &Map, reach: f32) -> Vec<Vec<f32>> {
    [FieldKind::Inside, FieldKind::Outside, FieldKind::BevelInside, FieldKind::BevelOutside].map(|k| distance_field(k, m.v.clone(), m.w, m.h, reach)).to_vec()
}

fn aa_disc(w: usize, h: usize, r: f32) -> Map {
    let mut m = Map::new(w, h, 0.0);
    const N: usize = 16;
    for y in 0..h {
        for x in 0..w {
            let mut hit = 0;
            for sy in 0..N {
                for sx in 0..N {
                    let (px, py) = (x as f32 + (sx as f32 + 0.5) / N as f32 - w as f32 / 2.0, y as f32 + (sy as f32 + 0.5) / N as f32 - h as f32 / 2.0);
                    hit += usize::from(px * px + py * py <= r * r);
                }
            }
            m.v[y * w + x] = hit as f32 / (N * N) as f32;
        }
    }
    m
}

/// Integer-aligned rounded rectangle (x0..x1 x y0..y1, corner radius `rad`), anti-aliased by supersampling.
fn aa_round_rect(w: usize, h: usize, x0: f32, y0: f32, x1: f32, y1: f32, rad: f32) -> Map {
    let sdf = |px: f32, py: f32| {
        let (cx, cy) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
        let (qx, qy) = ((px - cx).abs() - ((x1 - x0) / 2.0 - rad), (py - cy).abs() - ((y1 - y0) / 2.0 - rad));
        qx.max(0.0).hypot(qy.max(0.0)) + qx.max(qy).min(0.0) - rad
    };
    let mut m = Map::new(w, h, 0.0);
    const N: usize = 16;
    for y in 0..h {
        for x in 0..w {
            let mut hit = 0;
            for sy in 0..N {
                for sx in 0..N {
                    hit += usize::from(sdf(x as f32 + (sx as f32 + 0.5) / N as f32, y as f32 + (sy as f32 + 0.5) / N as f32) <= 0.0);
                }
            }
            m.v[y * w + x] = hit as f32 / (N * N) as f32;
        }
    }
    m
}

#[test]
fn chisel_hard_has_flat_facets() {
    let shape = square(48, 10, 38);
    let (m, _) = bevel_maps(
        &shape,
        &bevel_of(BevelStyle::InnerBevel, BevelTechnique::ChiselHard),
        &GlobalLight::default(),
        &no_tex(),
        &PreparedPatterns::new(&[], PREPARED_PATTERN_BYTES),
    );
    let at = |x: usize, y: usize| m[0].v[y * 48 + x];
    // A constant slope: equal highlight along the top facet.
    assert!(at(24, 12) > 0.1 && (at(24, 12) - at(24, 13)).abs() < 1e-3);
}

#[test]
fn linear_bevel_contour_leaves_the_height_alone_at_any_range() {
    for range in [0.5, 0.7, 1.0] {
        let lut = bevel_contour_lut(&photocraft_doc::BevelContour { contour: Contour::Linear, range, anti_alias: false });
        for v in [0.0f32, 0.25, 0.5, 0.75, 1.0] {
            assert!((lut_at(&lut, v) - v).abs() < 1e-3, "range {range}: {v} -> {}", lut_at(&lut, v));
        }
    }
}

#[test]
fn bevel_contour_and_texture_shape_the_height() {
    let shape = square(40, 6, 34);
    let l = GlobalLight::default();
    let plain = bevel_of(BevelStyle::InnerBevel, BevelTechnique::Smooth);
    let (a, _) = bevel_maps(&shape, &plain, &l, &no_tex(), &PreparedPatterns::new(&[], PREPARED_PATTERN_BYTES));
    // A contour reshapes the profile (here a ramp folded at its middle).
    let mut c = plain.clone();
    let fold = vec![
        photocraft_doc::adjust::CurvePoint { input: 0.0, output: 0.0 },
        photocraft_doc::adjust::CurvePoint { input: 0.5, output: 1.0 },
        photocraft_doc::adjust::CurvePoint { input: 1.0, output: 0.0 },
    ];
    c.contour =
        Some(photocraft_doc::BevelContour { contour: Contour::Custom { name: "fold".into(), points: fold, corners: vec![] }, range: 1.0, anti_alias: false });
    let (b, _) = bevel_maps(&shape, &c, &l, &no_tex(), &PreparedPatterns::new(&[], PREPARED_PATTERN_BYTES));
    let diff: f32 = a[1].v.iter().zip(&b[1].v).map(|(x, y)| (x - y).abs()).sum();
    assert!(diff > 5.0, "{diff}");
    // A striped texture lights the flat middle.
    let mut sf = photocraft_raster::Surface::new(photocraft_color::PixelFormat::RGBA8);
    sf.fill_rect(Rect::new(0, 0, 4, 2), &[0.0, 0.0, 0.0, 1.0]);
    sf.fill_rect(Rect::new(0, 2, 4, 4), &[1.0, 1.0, 1.0, 1.0]);
    let pats = [Pattern::new("stripes", sf, 4, 4)];
    let mut t = plain.clone();
    t.texture =
        Some(photocraft_doc::BevelTexture { name: "stripes".into(), id: String::new(), scale: 1.0, depth: 1.0, invert: false, link: true, phase: (0.0, 0.0) });
    let tex = TextureCtx { rect: Rect::new(0, 0, 40, 40), patterns: &pats, anchor: (0.0, 0.0) };
    let (m, _) = bevel_maps(&shape, &t, &l, &tex, &PreparedPatterns::new(&pats, PREPARED_PATTERN_BYTES));
    let mid: f32 = (16..24).map(|y| m[0].v[y * 40 + 20] + m[1].v[y * 40 + 20]).sum();
    let flat: f32 = (16..24).map(|y| a[0].v[y * 40 + 20] + a[1].v[y * 40 + 20]).sum();
    assert!(mid > flat + 0.5, "{mid} vs {flat}");
}

#[test]
fn glow_range_stretches_the_contour() {
    let mut g = Glow {
        common: photocraft_doc::FxCommon::new(BlendMode::Screen, 1.0),
        paint: FxPaint::Color(photocraft_color::Color::WHITE),
        technique: GlowTechnique::Softer,
        spread: 0.0,
        size: 10.0,
        contour: Contour::Linear,
        anti_alias: false,
        range: 0.5,
        jitter: 0.0,
        noise: 0.0,
        source: GlowSource::Edge,
    };
    let l = glow_lut(&g).unwrap();
    assert!((lut_at(&l, 0.25) - 0.5).abs() < 1e-3 && (lut_at(&l, 0.75) - 1.0).abs() < 1e-6);
    g.range = 1.0;
    assert!(glow_lut(&g).is_none());
}

/// Photoshop 27.11, measured on a 128 px square (Normal, black, 100 %, Range 50 %): a precise
/// glow's coverage across the edge, and a centre-source inner glow as 1 − the edge glow.
#[test]
fn glows_match_photoshop() {
    let glow = |technique, size: f32, spread: f32, source| Glow {
        common: photocraft_doc::FxCommon::new(BlendMode::Normal, 1.0),
        paint: FxPaint::Color(photocraft_color::Color::BLACK),
        technique,
        spread,
        size,
        contour: Contour::Linear,
        anti_alias: false,
        range: 0.5,
        jitter: 0.0,
        noise: 0.0,
        source,
    };
    let n = 60;
    let mut shape = Map::new(n, n, 0.0);
    for y in 20..40 {
        for x in 20..40 {
            shape.v[y * n + x] = 1.0;
        }
    }
    let at = |m: &Map, x: usize| m.v[30 * n + x];
    // Outer, precise, size 10: Photoshop's black glow over white reads 47, 93, 140, 186, 232
    // at 2, 4, 6, 8 and 10 px out from the edge pixel (alpha 1 − D / 11).
    let m = glow_map(&shape, &glow(GlowTechnique::Precise, 10.0, 0.0, GlowSource::Edge), false, (0, 0));
    for (d, ps) in [(2, 47.0f32), (4, 93.0), (6, 140.0), (8, 186.0), (10, 232.0)] {
        let got = 255.0 * (1.0 - at(&m, 20 - d));
        assert!((got - ps).abs() <= 1.5, "outer precise at {d} px: {got} vs Photoshop {ps}");
    }
    // Spread 40 %: solid for the first 4 px.
    let m = glow_map(&shape, &glow(GlowTechnique::Precise, 10.0, 0.4, GlowSource::Edge), false, (0, 0));
    assert!((at(&m, 16) - 1.0).abs() < 1e-6 && at(&m, 14) < 1.0);
    // Centre source = 1 − edge, for both techniques.
    for t in [GlowTechnique::Precise, GlowTechnique::Softer] {
        let e = glow_map(&shape, &glow(t, 6.0, 0.0, GlowSource::Edge), true, (0, 0));
        let c = glow_map(&shape, &glow(t, 6.0, 0.0, GlowSource::Center), true, (0, 0));
        for x in 20..40 {
            assert!((at(&c, x) - (1.0 - at(&e, x))).abs() < 1e-5, "{t:?} at {x}");
        }
    }
}

#[test]
fn softer_outer_glow_keeps_a_fading_tail_past_its_size() {
    let mut shape = Map::new(220, 220, 0.0);
    for y in 0..shape.h {
        for x in 20..100 {
            shape.v[y * shape.w + x] = 1.0;
        }
    }
    let glow = Glow {
        common: photocraft_doc::FxCommon::new(BlendMode::Screen, 1.0),
        paint: FxPaint::Color(photocraft_color::Color::WHITE),
        technique: GlowTechnique::Softer,
        spread: 0.0,
        size: 60.0,
        contour: Contour::Linear,
        anti_alias: false,
        range: 0.5,
        jitter: 0.0,
        noise: 0.0,
        source: GlowSource::Edge,
    };
    let m = glow_map(&shape, &glow, false, (0, 0));
    let row = shape.h / 2;
    let at = |distance: usize| m.v[row * m.w + 100 + distance];
    assert!(at(50) > 0.024, "the softer shoulder should stay visible: {}", at(50));
    assert!(at(60) > 1.0 / 255.0, "the glow must retain a visible tail at its Size: {}", at(60));
    assert!(at(62) > 0.5 / 255.0, "the glow must extend beyond its Size: {}", at(62));
    assert!(at(66) < 1e-6, "the added tail should fade to zero: {}", at(66));
}

#[test]
fn softer_outer_glow_margin_includes_its_tail_support() {
    let glow = Glow {
        common: photocraft_doc::FxCommon::new(BlendMode::Screen, 1.0),
        paint: FxPaint::Color(photocraft_color::Color::WHITE),
        technique: GlowTechnique::Softer,
        spread: 0.0,
        size: 60.0,
        contour: Contour::Linear,
        anti_alias: false,
        range: 0.5,
        jitter: 0.0,
        noise: 0.0,
        source: GlowSource::Edge,
    };
    let mut layer = Layer::raster("glow", photocraft_color::PixelFormat::RGBA8);
    layer.effects.items.push(Effect::OuterGlow(glow));
    assert_eq!(margin(&layer), 68);
}

#[test]
fn offsets_follow_light() {
    let (dx, dy) = offset(120.0, 10.0);
    assert_eq!((dx, dy), (5.0, 9.0));
}

fn square_shape(n: usize) -> Map {
    let mut m = Map::new(n, n, 0.0);
    for y in 5..n - 5 {
        for x in 5..n - 5 {
            m.v[y * n + x] = 1.0;
        }
    }
    m
}

#[test]
fn satin_at_distance_zero_matches_photoshop() {
    // Photoshop 27.11, Satin black 100 %, angle 90°, Linear contour, distance 0, size 5 over a
    // #808080 square: the rows from the top edge read 82, 102, 123, 128.
    let s = photocraft_doc::effects::Satin {
        common: photocraft_doc::FxCommon::new(BlendMode::Normal, 1.0),
        color: photocraft_color::Color::BLACK,
        angle: 90.0,
        distance: 0.0,
        size: 5.0,
        contour: Contour::Linear,
        anti_alias: false,
        invert: false,
    };
    let m = satin_map(&square_shape(60), &s);
    for (row, gray) in [(5, 82.0), (7, 102.0), (9, 123.0), (11, 128.0), (30, 128.0)] {
        let want = 1.0 - gray / 128.0;
        let got = m.v[row * 60 + 30];
        assert!((got - want).abs() < 0.02, "row {row}: {got} vs {want}");
    }
}

fn shadow(noise: f32, origin: (i32, i32)) -> Map {
    let s = Shadow {
        common: photocraft_doc::FxCommon::new(BlendMode::Multiply, 1.0),
        color: photocraft_color::Color::BLACK,
        angle: 0.0,
        use_global_light: false,
        distance: 0.0,
        spread: 0.0,
        size: 6.0,
        contour: Contour::Linear,
        anti_alias: false,
        noise,
        knocks_out: true,
    };
    shadow_map(&square_shape(40), &s, &GlobalLight::default(), false, origin)
}

#[test]
fn shadow_noise_speckles_soft_coverage_deterministically() {
    let plain = shadow(0.0, (0, 0));
    let noisy = shadow(0.5, (0, 0));
    let again = shadow(0.5, (0, 0));
    let (mut diff, mut identical) = (0.0f32, true);
    for i in 0..plain.v.len() {
        diff += (plain.v[i] - noisy.v[i]).abs();
        if noisy.v[i] != again.v[i] {
            identical = false;
        }
    }
    assert!(diff > 5.0, "noise changes the soft band: {diff}");
    assert!(identical, "same coords, same speckle");
    // Every value stays a coverage.
    assert!(noisy.v.iter().all(|v| (0.0..=1.0).contains(v)));
    // The speckle moves with the document origin, not the tile.
    let moved = shadow(0.5, (100, 100));
    assert_ne!(noisy.v, moved.v, "hashed from document coordinates");
    // Full noise can punch holes even in the solid part.
    let heavy = shadow(1.0, (0, 0));
    assert!(heavy.v.iter().any(|v| *v < 0.5) && heavy.v.iter().any(|v| *v > 0.5), "heavy noise spans the range",);
}

#[test]
fn box_blur_preserves_faint_coverage_and_empty_gaps() {
    let src = [0.0, 0.3, 0.7, 0.0, 0.0, 0.0, 0.0, 1e-20, 0.0, 0.0, 0.0];
    let mut actual = [0.0; 11];
    box_line(&src, 0, &mut actual, 3.0);
    assert_eq!(actual[5], 0.0, "an empty window must be exactly empty");
    assert_eq!(actual[10], 0.0, "the second tail must also be empty");
    assert!((actual[7] - 1e-20 / 3.0).abs() < 1e-30, "real faint coverage is not rounded away");
}

#[test]
fn effect_noise_does_not_light_up_empty_blur_tails() {
    // A sloping antialiased shape leaves rounding residue in a running-sum blur.
    // The direct finite-support convolution is the oracle for untouched pixels.
    for size in [5.0, 5.5, 13.0] {
        let mut shape = Map::new(96, 64, 0.0);
        for y in 8..48 {
            for x in 8..48 {
                shape.v[y * shape.w + x] = ((x as f32 - y as f32 * 0.7) * 0.3).clamp(0.0, 1.0);
            }
        }
        let mut direct = shape.clone();
        tent_direct(&mut direct, size);
        let mut actual = shape;
        blur(&mut actual, size);
        noise(&mut actual, 0.26, 0, 0);
        let stray = direct.v.iter().zip(&actual.v).filter(|(reference, noisy)| **reference == 0.0 && **noisy > 1e-5).count();
        assert_eq!(stray, 0, "noise created {stray} visible pixels outside a size-{size} blur");
    }
}

#[test]
fn glow_noise_follows_the_contour_and_range() {
    let g = |noise: f32| Glow {
        common: photocraft_doc::FxCommon::new(BlendMode::Screen, 1.0),
        paint: FxPaint::Color(photocraft_color::Color::WHITE),
        technique: GlowTechnique::Softer,
        spread: 0.0,
        size: 6.0,
        contour: Contour::Linear,
        anti_alias: false,
        range: 0.5,
        jitter: 0.0,
        noise,
        source: GlowSource::Edge,
    };
    let plain = glow_map(&square_shape(40), &g(0.0), false, (0, 0));
    let noisy = glow_map(&square_shape(40), &g(0.6), false, (0, 0));
    let diff: f32 = plain.v.iter().zip(&noisy.v).map(|(a, b)| (a - b).abs()).sum();
    assert!(diff > 5.0, "glow noise changes the coverage: {diff}");
}

/// Issue #7833: the speckle stays inside the glow's visible falloff instead of filling the
/// blur's whole rectangular reach out to the map edge.
#[test]
fn effect_noise_stays_inside_the_visible_falloff() {
    let mut shape = Map::new(80, 80, 0.0);
    for y in 35..45 {
        for x in 35..45 {
            shape.v[y * 80 + x] = 1.0;
        }
    }
    let g = |noise: f32| Glow {
        common: photocraft_doc::FxCommon::new(BlendMode::Screen, 1.0),
        paint: FxPaint::Color(photocraft_color::Color::WHITE),
        technique: GlowTechnique::Softer,
        spread: 0.0,
        size: 10.0,
        contour: Contour::Linear,
        anti_alias: false,
        range: 0.5,
        jitter: 0.0,
        noise,
        source: GlowSource::Edge,
    };
    let s = |noise: f32| Shadow {
        common: photocraft_doc::FxCommon::new(BlendMode::Multiply, 1.0),
        color: photocraft_color::Color::BLACK,
        angle: 0.0,
        use_global_light: false,
        distance: 0.0,
        spread: 0.0,
        size: 10.0,
        contour: Contour::Linear,
        anti_alias: false,
        noise,
        knocks_out: true,
    };
    let light = GlobalLight::default();
    let pairs = [
        ("glow", glow_map(&shape, &g(0.0), false, (0, 0)), glow_map(&shape, &g(0.5), false, (0, 0))),
        ("shadow", shadow_map(&shape, &s(0.0), &light, false, (0, 0)), shadow_map(&shape, &s(0.5), &light, false, (0, 0))),
    ];
    for (name, plain, noisy) in pairs {
        for (i, (p, n)) in plain.v.iter().zip(&noisy.v).enumerate() {
            if *p < 1.0 / 255.0 {
                assert!(*n <= 2.0 / 255.0, "{name}: pixel ({}, {}) invisible without noise: {p} -> {n}", i % 80, i / 80);
            }
        }
    }
}

#[test]
fn shifting_by_hostile_distances_does_not_overflow() {
    let m = square_shape(12);
    for d in [1.0e30, -1.0e30, f32::INFINITY, f32::NAN] {
        let s = m.shifted(d, d, 0.0);
        assert_eq!(s.v.len(), 144);
    }
}
