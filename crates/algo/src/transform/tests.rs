use super::*;

#[test]
fn output_size_prices_the_unclipped_extent_independently_of_position() {
    let src = Rect::new(0, 0, 24, 16);
    for dx in [0.0, 900_000.0, 10_000_000.0] {
        let h = Homography([1.0, 0.0, dx, 0.0, 1.0, -dx, 0.0, 0.0, 1.0]);
        assert_eq!(warp_output_size(src, &h), Some((26, 18)));
    }
    for sx in [1e7, -1e7] {
        let h = Homography([sx, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0]);
        assert_eq!(warp_output_size(src, &h), Some((240_000_002, 18)));
    }
}

/// The unclipped size matches the clipped bounds where nothing is clipped, saturates when it
/// can't be represented, and is `None` for empty or singular inputs.
#[test]
fn output_size_saturates_overflows_and_preserves_filter_support() {
    let src = Rect::new(0, 0, 24, 16);
    let fractional = Homography([0.5, 0.0, 0.25, 0.0, 1.0, 0.75, 0.0, 0.0, 1.0]);
    let bounds = warp_output_bounds(src, &fractional).unwrap();
    assert_eq!(warp_output_size(src, &fractional), Some((u64::from(bounds.width()), u64::from(bounds.height()))));
    let huge = Homography([1e308, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0]);
    assert_eq!(warp_output_size(src, &huge), Some((u64::MAX, u64::MAX)));
    assert_eq!(warp_output_size(Rect::EMPTY, &Homography::IDENTITY), None);
    assert_eq!(warp_output_size(src, &Homography([0.0; 9])), None);
}

/// [`warp_output_clipped`] says yes exactly when [`warp_output_bounds`] clamps a side (#7807).
#[test]
fn output_clipped_matches_the_clamped_bounds() {
    let src = Rect::new(0, 0, 24, 16);
    let at = |dx: f64| Homography([1.0, 0.0, dx, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0]);
    let edge = f64::from(WARP_REACH) - 24.0;
    assert!(!warp_output_clipped(src, &at(edge)), "the right edge lands on the reach");
    assert_eq!(warp_output_bounds(src, &at(edge)).map(|b| b.x1), Some(WARP_REACH + 1));
    assert!(warp_output_clipped(src, &at(edge + 1.0)), "one pixel past it");
    assert_eq!(warp_output_bounds(src, &at(edge + 1.0)).map(|b| b.x1), Some(WARP_REACH + 1), "clamped");
    assert!(warp_output_clipped(src, &at(-f64::from(WARP_REACH) - 1.0)));
    assert!(!warp_output_clipped(Rect::EMPTY, &at(1e12)), "nothing to clip");
}

#[test]
fn the_interruptible_warp_reports_progress_and_stops_when_cancelled() {
    let mut s = Surface::new(PixelFormat::RGBA8);
    s.fill_rect(Rect::new(0, 0, 600, 400), &[0.2, 0.4, 0.6, 1.0]);
    let r = s.content_bounds();
    let h = Homography::rect_to_quad([0.0, 0.0, 600.0, 400.0], [[10.0, 0.0], [590.0, 20.0], [600.0, 400.0], [0.0, 380.0]]).unwrap();
    let reported = std::sync::Mutex::new(0.0f32);
    let progress = |p: f32| {
        let mut m = reported.lock().unwrap();
        *m = m.max(p);
    };
    let never = || false;
    let out = warp_surface_with(&s, r, &h, Interp::Bicubic, &Interrupt::new(&never, &progress)).unwrap().expect("not cancelled");
    assert_eq!(out.read_region(r), warp_surface(&s, r, &h, Interp::Bicubic).unwrap().read_region(r));
    assert!(*reported.lock().unwrap() > 0.9);
    let always = || true;
    assert!(warp_surface_with(&s, r, &h, Interp::Bicubic, &Interrupt::cancel_only(&always)).unwrap().is_none());
}

/// Perspective Crop's map lands each quad corner exactly on its rectangle corner; degenerate,
/// self-crossing, counter-clockwise and non-finite quads have none.
#[test]
fn quad_to_rect_maps_corners_and_refuses_bad_quads() {
    let q = [[30.0, 12.0], [140.0, 25.0], [128.0, 110.0], [14.0, 95.0]];
    let h = quad_to_rect(&q, 100.0, 80.0).unwrap();
    for (c, want) in q.iter().zip([[0.0, 0.0], [100.0, 0.0], [100.0, 80.0], [0.0, 80.0]]) {
        let (x, y) = h.apply(c[0], c[1]);
        assert!((x - want[0]).abs() < 1e-9 && (y - want[1]).abs() < 1e-9, "{c:?} -> ({x}, {y})");
    }
    let (w, ht) = quad_size(&q);
    assert!((w - 112.874).abs() < 1e-3 && (ht - 85.185).abs() < 1e-3, "{w} x {ht}");
    for bad in [
        [[0.0, 0.0], [50.0, 0.0], [100.0, 0.0], [0.0, 50.0]],
        [[0.0, 0.0], [50.0, 50.0], [50.0, 0.0], [0.0, 50.0]],
        [[0.0, 0.0], [0.0, 50.0], [50.0, 50.0], [50.0, 0.0]],
        [[0.0, 0.0], [f64::NAN, 0.0], [50.0, 50.0], [0.0, 50.0]],
        [[0.0, 0.0]; 4],
    ] {
        assert!(quad_to_rect(&bad, 10.0, 10.0).is_none(), "{bad:?}");
    }
    assert!(quad_to_rect(&q, 0.0, 10.0).is_none());
}

fn rgba() -> Surface {
    Surface::new(PixelFormat::RGBA8)
}

/// The "pixel stretch" (#3773): a 1 px column scaled 100x stays solid across the stretch, as in
/// Photoshop. Samples past the source edge repeat the edge, and only the outline is antialiased.
#[test]
fn stretched_single_column_stays_opaque() {
    let mut s = rgba();
    s.fill_rect(Rect::new(0, 0, 1, 4), &[0.0, 1.0, 0.0, 1.0]);
    for interp in [Interp::Bicubic, Interp::Bilinear] {
        let h = Homography::rect_to_quad([0.0, 0.0, 1.0, 4.0], [[0.0, 0.0], [100.0, 0.0], [100.0, 4.0], [0.0, 4.0]]).unwrap();
        let w = warp_surface(&s, Rect::new(0, 0, 1, 4), &h, interp).unwrap();
        assert_eq!(w.content_bounds(), Rect::new(0, 0, 100, 4), "{interp:?}");
        for x in [0, 1, 25, 50, 75, 99] {
            for y in 0..4 {
                assert!((w.sample_channel(x, y, 3) - 1.0).abs() < 1e-6, "{interp:?} alpha at ({x}, {y}) = {}", w.sample_channel(x, y, 3));
            }
        }
    }
}

/// Photoshop 27.11's Free Transform, Bicubic, 200 % about the centre of a 64 px image whose left
/// half is grey 64 and right half grey 192: the row across the edge, columns 26..36.
#[test]
fn bicubic_upscale_matches_photoshop() {
    let mut src = rgba();
    let g = |v: f32| v / 255.0;
    src.fill_rect(Rect::new(0, 0, 32, 64), &[g(64.0), g(64.0), g(64.0), 1.0]);
    src.fill_rect(Rect::new(32, 0, 64, 64), &[g(192.0), g(192.0), g(192.0), 1.0]);
    let q = [[-32.0, -32.0], [96.0, -32.0], [96.0, 96.0], [-32.0, 96.0]];
    let h = Homography::rect_to_quad([0.0, 0.0, 64.0, 64.0], q).unwrap();
    let out = warp_surface(&src, Rect::new(0, 0, 64, 64), &h, Interp::Bicubic).unwrap();
    let photoshop = [64.0, 64.0, 64.0, 60.0, 51.0, 93.0, 163.0, 206.0, 197.0, 192.0];
    for (i, want) in photoshop.iter().enumerate() {
        let got = out.pixel(26 + i as i32, 50)[0] * 255.0;
        assert!((got - want).abs() <= 1.5, "column {}: {got} vs {want}", 26 + i);
    }
}

#[test]
fn homography_maps_rect_corners_and_inverts() {
    let r = [10.0, 20.0, 110.0, 70.0];
    let q = [[0.0, 0.0], [200.0, 10.0], [180.0, 150.0], [-20.0, 120.0]];
    let h = Homography::rect_to_quad(r, q).unwrap();
    for (p, e) in [(10.0, 20.0), (110.0, 20.0), (110.0, 70.0), (10.0, 70.0)].iter().zip(q) {
        let (x, y) = h.apply(p.0, p.1);
        assert!((x - e[0]).abs() < 1e-9 && (y - e[1]).abs() < 1e-9, "{p:?} → {x},{y} vs {e:?}");
    }
    let inv = h.inverse().unwrap();
    let (x, y) = inv.apply(90.0, 60.0);
    let (bx, by) = h.apply(x, y);
    assert!((bx - 90.0).abs() < 1e-9 && (by - 60.0).abs() < 1e-9);
    assert!(Homography::rect_to_quad(r, [[0.0, 0.0]; 4]).is_none() || Homography::rect_to_quad(r, [[0.0, 0.0]; 4]).unwrap().inverse().is_none());
}

#[test]
fn steep_distort_keeps_every_pixel_inside_the_quad() {
    // Corner C is almost flat, so the horizon of the inverse map crosses the 256-px tiles that
    // also hold quad pixels; those tiles must still read every source pixel that lands in them.
    let mut s = rgba();
    s.fill_rect(Rect::new(0, 0, 100, 100), &[1.0, 0.0, 0.0, 1.0]);
    let r = s.content_bounds();
    let h = Homography::rect_to_quad([0.0, 0.0, 100.0, 100.0], [[0.0, 0.0], [300.0, 0.0], [155.0, 155.0], [0.0, 300.0]]).unwrap();
    let w = warp_surface(&s, r, &h, Interp::Nearest).unwrap();
    for (x, y) in [(20, 20), (252, 14), (252, 42), (14, 252), (42, 252)] {
        let a = w.read_region(Rect::new(x, y, x + 1, y + 1))[3];
        assert!(a > 0.99, "({x},{y}) inside the quad is transparent: {a}");
    }
    for (x, y) in [(200, 200), (280, 100), (100, 280)] {
        assert_eq!(w.read_region(Rect::new(x, y, x + 1, y + 1))[3], 0.0, "({x},{y}) outside the quad is painted");
    }
}

/// #4863: a Distort with the bottom corners pulled in and one dragged past the horizon (a
/// concave quad) leaves part of the source behind the horizon. That part maps to the far side,
/// where it showed as a mirrored copy wrapped round infinity; only the front side is drawn.
#[test]
fn a_distort_past_the_horizon_draws_only_its_front_side() {
    let mut s = rgba();
    s.fill_rect(Rect::new(0, 0, 100, 100), &[0.0, 0.5, 1.0, 1.0]);
    let r = s.content_bounds();
    let h = Homography::rect_to_quad([0.0, 0.0, 100.0, 100.0], [[0.0, 0.0], [300.0, 0.0], [60.0, 60.0], [0.0, 300.0]]).unwrap();
    let corners = [(0.0, 0.0), (100.0, 0.0), (100.0, 100.0), (0.0, 100.0)].map(|(x, y)| h.w(x, y));
    assert!(corners.iter().any(|w| *w > 0.0) && corners.iter().any(|w| *w < 0.0), "the source straddles the horizon: {corners:?}");
    let front = h.front_side([0.0, 0.0, 100.0, 100.0]);
    let inv = h.inverse().unwrap();
    let clip = Rect::new(-400, -400, 700, 700);
    for interp in [Interp::Nearest, Interp::Bilinear] {
        let w = warp_surface_clipped(&s, r, &h, interp, clip).unwrap();
        let px = w.read_region(clip);
        let (mut drawn, mut behind) = (0, 0);
        for y in clip.y0..clip.y1 {
            for x in clip.x0..clip.x1 {
                let i = ((y - clip.y0) * clip.width() as i32 + (x - clip.x0)) as usize * 4;
                if px[i + 3] > 0.0 {
                    drawn += 1;
                    // The source point this pixel samples is on the far side of the horizon.
                    if front * inv.w(x as f64 + 0.5, y as f64 + 0.5) <= 0.0 {
                        behind += 1;
                    }
                }
            }
        }
        assert!(drawn > 1000, "{interp:?}: the front side is drawn ({drawn} px)");
        assert_eq!(behind, 0, "{interp:?}: {behind} pixels come from behind the horizon");
    }
    // A quad wholly in front of the horizon is drawn exactly as before.
    let ok = Homography::rect_to_quad([0.0, 0.0, 100.0, 100.0], [[0.0, 0.0], [300.0, 0.0], [155.0, 155.0], [0.0, 300.0]]).unwrap();
    assert_eq!(ok.front_side([0.0, 0.0, 100.0, 100.0]), ok.w(50.0, 50.0).signum());
}

/// #7557: a Perspective Crop quad low on a strongly converging photo puts the horizon across
/// the layer with most of it (and its centre) on the far side. The clip maps wholly into the
/// quad, so its side is the one drawn, not the source's majority.
#[test]
fn a_clip_wholly_on_one_side_draws_that_side() {
    let mut s = rgba();
    s.fill_rect(Rect::new(0, 0, 100, 100), &[1.0, 0.0, 0.0, 1.0]);
    let q = [[40.0, 70.0], [60.0, 70.0], [90.0, 95.0], [10.0, 95.0]];
    let h = quad_to_rect(&q, 50.0, 39.0).unwrap();
    assert!(h.front_side([0.0, 0.0, 100.0, 100.0]) * h.w(50.0, 80.0) < 0.0, "the source's majority is behind the quad's side");
    for interp in [Interp::Nearest, Interp::Bilinear, Interp::Bicubic] {
        let w = warp_surface_clipped(&s, s.content_bounds(), &h, interp, Rect::new(0, 0, 50, 39)).unwrap();
        assert_eq!(w.rgba(25, 20), [1.0, 0.0, 0.0, 1.0], "{interp:?}");
    }
}

#[test]
fn identity_and_translation_are_exact() {
    let mut s = rgba();
    s.fill_rect(Rect::new(4, 4, 20, 12), &[1.0, 0.5, 0.0, 1.0]);
    let r = s.content_bounds();
    for interp in [Interp::Nearest, Interp::Bilinear, Interp::Bicubic] {
        let h = Homography([1.0, 0.0, 7.0, 0.0, 1.0, -3.0, 0.0, 0.0, 1.0]);
        let o = warp_surface(&s, r, &h, interp).unwrap();
        assert_eq!(o.content_bounds(), Rect::new(11, 1, 27, 9), "{interp:?}");
        assert_eq!(o.pixel(15, 5), vec![1.0, 128.0 / 255.0, 0.0, 1.0], "{interp:?}");
    }
}

#[test]
fn translation_preserves_colour_range_at_every_depth() {
    use photocraft_color::{ColorMode, SampleType};

    let h = Homography([1.0, 0.0, 7.0, 0.0, 1.0, -3.0, 0.0, 0.0, 1.0]);
    for sample in SampleType::ALL {
        for mode in [ColorMode::Rgb, ColorMode::Grayscale] {
            for alpha in [false, true] {
                let mut s = Surface::new(PixelFormat::new(mode, sample, alpha));
                let mut pixel = if mode == ColorMode::Rgb { vec![2.0, -0.25, 0.5] } else { vec![2.0] };
                if alpha {
                    pixel.push(0.5);
                }
                s.fill_rect(Rect::new(4, 4, 8, 8), &pixel);
                let mut expected = s.pixel(5, 5);
                if !alpha {
                    expected.push(1.0);
                }
                for interp in [Interp::Nearest, Interp::Bilinear, Interp::Bicubic] {
                    let out = warp_surface(&s, s.content_bounds(), &h, interp).unwrap();
                    assert_eq!(out.pixel(12, 2), expected, "{sample:?}/{mode:?}/alpha={alpha}/{interp:?}");
                    assert_eq!(out.pixel(0, 0).last(), Some(&0.0));
                }
            }
        }
    }
}

#[test]
fn bilinear_translation_interpolates_signed_hdr_colours() {
    let mut s = Surface::new(PixelFormat::RGBA32F);
    s.write_pixel(0, 0, &[2.0, -0.5, 0.25, 0.5]);
    s.write_pixel(1, 0, &[4.0, -0.25, 0.75, 0.5]);
    let h = Homography([1.0, 0.0, 0.5, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0]);
    let out = warp_surface(&s, s.content_bounds(), &h, Interp::Bilinear).unwrap();
    assert_eq!(out.pixel(1, 0), vec![3.0, -0.375, 0.5, 0.5]);
}

#[test]
fn scale_doubles_size_and_rotation_keeps_area() {
    let mut s = rgba();
    s.fill_rect(Rect::new(0, 0, 40, 20), &[0.2, 0.4, 0.6, 1.0]);
    let r = s.content_bounds();
    let up =
        warp_surface(&s, r, &Homography::rect_to_quad([0.0, 0.0, 40.0, 20.0], [[0.0, 0.0], [80.0, 0.0], [80.0, 40.0], [0.0, 40.0]]).unwrap(), Interp::Bicubic)
            .unwrap();
    let b = up.content_bounds();
    assert!(b.width() >= 80 && b.width() <= 82 && b.height() >= 40 && b.height() <= 42, "{b:?}");
    // 90° rotation about (20, 10): a 40×20 box becomes 20×40.
    let q = [[30.0, -10.0], [30.0, 30.0], [10.0, 30.0], [10.0, -10.0]];
    let rot = warp_surface(&s, r, &Homography::rect_to_quad([0.0, 0.0, 40.0, 20.0], q).unwrap(), Interp::Bilinear).unwrap();
    let b = rot.content_bounds();
    assert!(b.width().abs_diff(20) <= 2 && b.height().abs_diff(40) <= 2, "{b:?}");
    let p = rot.pixel(20, 10);
    assert!((p[0] - 0.2).abs() < 0.01 && p[3] > 0.99, "{p:?}");
}

/// #6203: a bicubic rotation of a sprite whose edge pixels are partly transparent overshoots
/// its alpha past 1. The colour must stay the sprite's, not grow with the overshoot into a
/// light border.
#[test]
fn rotation_keeps_the_colour_of_a_soft_edged_sprite() {
    let mut s = rgba();
    s.fill_rect(Rect::new(10, 10, 30, 30), &[0.8, 0.6, 0.4, 0.5]);
    s.fill_rect(Rect::new(11, 11, 29, 29), &[0.8, 0.6, 0.4, 1.0]);
    let (c, k) = (20.0, std::f64::consts::FRAC_1_SQRT_2 * 10.0);
    let q = [[c, c - 2.0 * k], [c + 2.0 * k, c], [c, c + 2.0 * k], [c - 2.0 * k, c]];
    let h = Homography::rect_to_quad([10.0, 10.0, 30.0, 30.0], q).unwrap();
    for interp in [Interp::Bicubic, Interp::Bilinear] {
        let out = warp_surface(&s, s.content_bounds(), &h, interp).unwrap();
        let b = out.content_bounds();
        for y in b.y0..b.y1 {
            for x in b.x0..b.x1 {
                let p = out.pixel(x, y);
                if p[3] > 0.0 {
                    let off = (0..3).map(|i| (p[i] - [0.8, 0.6, 0.4][i]).abs()).fold(0.0f32, f32::max);
                    assert!(off <= 1.0 / 255.0, "{interp:?} ({x}, {y}): {p:?}");
                }
            }
        }
    }
}

#[test]
fn large_downscale_is_prefiltered() {
    // A 1px checkerboard shrunk 8× must come out ~50% grey, not aliased black/white.
    let mut s = rgba();
    for y in 0..64 {
        for x in 0..64 {
            let v = ((x + y) % 2) as f32;
            s.fill_rect(Rect::new(x, y, x + 1, y + 1), &[v, v, v, 1.0]);
        }
    }
    let h = Homography::rect_to_quad([0.0, 0.0, 64.0, 64.0], [[0.0, 0.0], [8.0, 0.0], [8.0, 8.0], [0.0, 8.0]]).unwrap();
    let o = warp_surface(&s, s.content_bounds(), &h, Interp::Bicubic).unwrap();
    let p = o.pixel(4, 4);
    assert!((p[0] - 0.5).abs() < 0.1, "{p:?}");
}

#[test]
fn surfaces_without_alpha_gain_it() {
    let mut s = Surface::new(PixelFormat::new(photocraft_color::ColorMode::Rgb, photocraft_color::SampleType::U8, false));
    s.fill_rect(Rect::new(0, 0, 10, 10), &[1.0, 0.0, 0.0]);
    let h = Homography([1.0, 0.0, 5.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0]);
    let o = warp_surface(&s, Rect::new(0, 0, 10, 10), &h, Interp::Nearest).unwrap();
    assert!(o.format().alpha);
    assert_eq!(o.pixel(2, 2)[3], 0.0);
    assert_eq!(o.pixel(7, 2), vec![1.0, 0.0, 0.0, 1.0]);
}

#[test]
fn homography_is_identity() {
    assert!(Homography::IDENTITY.is_identity());
    assert!(Homography([2.0, 0.0, 0.0, 0.0, 2.0, 0.0, 0.0, 0.0, 2.0]).is_identity());
    assert!(!Homography([1.0, 0.0, 1.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0]).is_identity());
    assert!(!Homography([1.0, 0.1, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0]).is_identity());
}
