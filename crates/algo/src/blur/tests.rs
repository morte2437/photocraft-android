use super::*;
use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_raster::Surface;

/// #8544, rendered: Gaussian Blur at 0.1–0.3 px through the filter itself (the direct-kernel
/// path) visibly softens a hard edge, more with each step, as the kernel predicts.
#[test]
fn small_gaussian_radii_soften_a_rendered_edge() {
    let canvas = Rect::new(0, 0, 20, 4);
    let mut s = Surface::new(PixelFormat::RGBA8);
    s.fill_rect(canvas, &[0.0, 0.0, 0.0, 1.0]);
    s.fill_rect(Rect::new(10, 0, 20, 4), &[1.0, 1.0, 1.0, 1.0]);
    let mut last = 0.0f32;
    for radius in [0.1f32, 0.2, 0.3] {
        let p = FilterParams::GaussianBlur { radius };
        assert!(box_widths(&p).is_none(), "the direct-kernel path");
        let out = crate::apply(&s, &p, canvas, canvas, None);
        // The last black pixel takes the white side's share of the kernel.
        let got = out.pixel(9, 1)[0];
        let k = gaussian_blur_kernel(radius);
        let want: f32 = k[k.len() / 2 + 1..].iter().sum();
        assert!((got - want).abs() <= 1.0 / 255.0, "radius {radius}: {got} vs {want}");
        assert!(got >= 1.0 / 255.0 && got > last, "radius {radius}: {got} after {last}");
        last = got;
    }
}

/// #8544: Gaussian Blur spreads a hard edge for every radius from 0.1, growing smoothly
/// with the radius (no dead zone up to 0.3, then a jump); from 1 up the kernel is unchanged.
#[test]
fn small_gaussian_radii_blur_smoothly() {
    let spread = |k: &[f32]| {
        let r = (k.len() / 2) as f32;
        k.iter().enumerate().map(|(i, w)| w * (i as f32 - r).powi(2)).sum::<f32>()
    };
    let mut last = 0.0f32;
    for step in 1..=20 {
        let radius = step as f32 * 0.05;
        let k = gaussian_blur_kernel(radius);
        assert!((k.iter().sum::<f32>() - 1.0).abs() < 1e-5, "normalised at {radius}");
        if radius < 1.0 {
            assert!((spread(&k) - radius * radius).abs() < 1e-4, "variance at {radius}: {}", spread(&k));
        }
        // A hard 0 → 1 edge: the first pixel past it takes the kernel's tail beyond the centre.
        let c = k.len() / 2;
        let edge: f32 = k[c + 1..].iter().sum();
        assert!(edge > last, "radius {radius}: edge {edge} after {last}");
        if radius > 0.1 {
            assert!(edge - last < 0.05, "radius {radius}: jump {} from {last}", edge - last);
        }
        last = edge;
    }
    assert!(gaussian_blur_kernel(0.1).len() >= 3, "0.1 blurs");
    // No step where the variance-matched range hands over to plain sampling.
    let edge = |r: f32| {
        let k = gaussian_blur_kernel(r);
        k[k.len() / 2 + 1..].iter().sum::<f32>()
    };
    assert!((edge(0.999) - edge(1.0)).abs() < 0.005, "{} vs {}", edge(0.999), edge(1.0));
    // No dip where a per-radius width used to add taps (1/3 and 2/3).
    for pair in [(0.33f32, 0.34f32), (0.66, 0.67)] {
        assert!(edge(pair.1) > edge(pair.0), "edge falls from {} to {}", pair.0, pair.1);
    }
    // Exact variance right up to the hand-over, past where `gaussian_kernel` changes width.
    for radius in [0.98f32, 0.995, 0.998, 0.9995] {
        let k = gaussian_blur_kernel(radius);
        assert!((spread(&k) - radius * radius).abs() < 2e-5, "variance at {radius}: {}", spread(&k));
    }
    for radius in [1.0, 1.5, 3.0, 4.0] {
        assert_eq!(gaussian_blur_kernel(radius), gaussian_kernel(radius), "{radius}");
    }
    assert_eq!(gaussian_blur_kernel(0.0), vec![1.0]);
}

#[test]
fn radial_blur_samples_large_arcs_beyond_64_intervals() {
    let bounds = Rect::new(0, 0, 1000, 1000);
    let out = Rect::new(999, 500, 1000, 501);
    let ctx = Ctx { bounds, mode: photocraft_color::ColorMode::Grayscale, alpha: false };
    let mut img = Image::new(bounds, 1);
    let (cx, cy) = (500.0f32, 500.0f32);
    let (x, y) = (999.5f32, 500.5f32);
    let (dx, dy) = (x - cx, y - cy);
    let arc = 100.0f32.to_radians();
    let count = radial_intervals(arc * dx.hypot(dy), RadialQuality::Good);
    assert_eq!(count, 256);
    assert_eq!(radial_intervals(f32::INFINITY, RadialQuality::Draft), 64);
    assert_eq!(radial_intervals(f32::INFINITY, RadialQuality::Good), 256);
    assert_eq!(radial_intervals(f32::INFINITY, RadialQuality::Best), 4096);
    assert_eq!(radial_intervals(f32::NAN, RadialQuality::Best), 1);

    // Put a small bright patch halfway between two samples from the former 64-interval
    // limit. Dense sampling should pick it up; the coarse path misses it entirely.
    let sparse_midpoint = (31.5 / 64.0 - 0.5) * arc;
    let (s, c) = sparse_midpoint.sin_cos();
    let px = (cx + dx * c - dy * s - 0.5).round() as i32;
    let py = (cy + dx * s + dy * c - 0.5).round() as i32;
    for yy in py - 1..=py + 1 {
        for xx in px - 1..=px + 1 {
            let index = (yy - bounds.y0) as usize * bounds.width() as usize + (xx - bounds.x0) as usize;
            img.data[index] = 1.0;
        }
    }

    let sample_path = |x: f32, y: f32, pts: &mut Vec<(f32, f32)>, intervals: usize| {
        let (dx, dy) = (x - cx, y - cy);
        for i in 0..=intervals {
            let t = (i as f32 / intervals as f32 - 0.5) * arc;
            let (s, c) = t.sin_cos();
            pts.push((cx + dx * c - dy * s, cy + dx * s + dy * c));
        }
    };
    let actual = radial(&img, out, &ctx, 100.0, RadialMethod::Spin, RadialQuality::Good, (0.5, 0.5))[0];
    let expected = average_samples(&img, out, &ctx, |x, y, pts| sample_path(x, y, pts, 8192))[0];
    let coarse = average_samples(&img, out, &ctx, |x, y, pts| sample_path(x, y, pts, 64))[0];

    assert!(actual > 0.001, "dense radial samples should resolve the bright patch, got {actual}");
    assert!((actual - expected).abs() < 0.0015, "radial result {actual} differs from dense reference {expected}");
    assert!(coarse < expected * 0.1, "former 64-interval sampling unexpectedly resolved the patch: {coarse} vs {expected}");
}

#[test]
fn fixed_channel_convolution_matches_scalar_tap_order() {
    fn check<const N: usize>() {
        for width in [1, 7, 256] {
            for radius in [0.0, 0.5, 3.0, 4.0] {
                let k = gaussian_kernel(radius);
                let src: Vec<f32> = (0..(width + k.len() - 1) * N).map(|i| ((i * 73) % 251) as f32 / 250.0).collect();
                let mut expected = vec![0.0f32; width * N];
                for x in 0..width {
                    for (i, w) in k.iter().enumerate() {
                        for c in 0..N {
                            expected[x * N + c] += src[(x + i) * N + c] * w;
                        }
                    }
                }
                let mut actual = vec![0.0f32; width * N];
                conv_row::<N>(&src, &mut actual, &k);
                assert_eq!(actual, expected, "channels={N} width={width} radius={radius}");
            }
        }
    }
    check::<1>();
    check::<2>();
    check::<3>();
    check::<4>();
    check::<5>();
}

#[test]
fn box_gaussian_matches_exact_kernel() {
    // A hard vertical edge (premultiplied RGBA-ish, 2 channels) blurred with σ = 12.
    let src_rect = Rect::new(-60, -60, 180, 68);
    let mut img = Image::new(src_rect, 2);
    for y in src_rect.y0..src_rect.y1 {
        for x in src_rect.x0..src_rect.x1 {
            let i = ((y - src_rect.y0) as usize * src_rect.width() as usize + (x - src_rect.x0) as usize) * 2;
            img.data[i] = if x < 60 { 1.0 } else { 0.0 };
            img.data[i + 1] = 1.0;
        }
    }
    let out = Rect::new(0, 0, 120, 8);
    let k = gaussian_kernel(12.0);
    let exact = conv_sep(&img, out, &k, &k, true);
    let fast = box_passes(&img, out, true, &boxes_for_gauss(12.0, 3));
    let err = exact.iter().zip(&fast).map(|(a, b)| (a - b).abs()).fold(0.0f32, f32::max);
    assert!(err < 0.02, "max error {err}");
    assert_eq!(boxes_for_gauss(10.0, 3).len(), 3);
}

#[test]
fn box_passes_match_the_direct_box_kernel() {
    // Noisy RGBA with varying alpha, so premultiplication and every channel are exercised.
    for r in [5usize, 17, 60] {
        let (w, h, m) = (40, 24, r as i32 + 2);
        let src_rect = Rect::new(-m, -m, w + m, h + m);
        let mut img = Image::new(src_rect, 4);
        let mut s = 0x9e37_79b9u32;
        for v in img.data.iter_mut() {
            s ^= s << 13;
            s ^= s >> 17;
            s ^= s << 5;
            *v = (s % 256) as f32 / 255.0;
        }
        let out = Rect::new(0, 0, w, h);
        let k = vec![1.0 / (2 * r + 1) as f32; 2 * r + 1];
        let direct = conv_sep(&img, out, &k, &k, true);
        let ctx = Ctx { bounds: src_rect, mode: crate::ColorMode::Rgb, alpha: true };
        let fast = boxed(&img, out, &ctx, r as f32);
        let err = direct.iter().zip(&fast).map(|(a, b)| (a - b).abs()).fold(0.0f32, f32::max);
        assert!(err < 1e-5, "radius {r}: max error {err}");
    }
}

/// The definition the running sums must match: each box applied in turn as a direct
/// convolution (cost per pixel proportional to the radius) to `img`, which covers `out` grown
/// by every box's reach (its own edge handling included).
fn direct_boxes(img: &Image, out: Rect, boxes: &[usize], alpha: bool) -> Vec<f32> {
    let mut cur = img.clone();
    let mut left: i32 = boxes.iter().map(|w| (w / 2) as i32).sum();
    for &w in boxes {
        left -= (w / 2) as i32;
        let k = vec![1.0 / w as f32; w];
        let rect = out.inflate(left);
        cur = Image { rect, ch: img.ch, data: conv_sep(&cur, rect, &k, &k, alpha) };
    }
    cur.data
}

/// Noisy pixels over `r` (alpha varying, some fully transparent).
fn noisy(fmt: PixelFormat, r: Rect, seed: u32) -> Surface {
    let mut s = Surface::new(fmt);
    let mut st = seed;
    let mut v = Vec::new();
    for _ in 0..r.width() * r.height() {
        for c in 0..fmt.channels() {
            st ^= st << 13;
            st ^= st >> 17;
            st ^= st << 5;
            let x = (st % 256) as f32 / 255.0;
            v.push(if fmt.alpha && c + 1 == fmt.channels() && st.is_multiple_of(5) { 0.0 } else { x });
        }
    }
    s.write_region(r, &v);
    s
}

#[test]
fn layer_box_blurs_match_direct_convolution_at_any_radius() {
    // Gaussian and Box Blur at radii up to several times the layer (37 × 23), filtered as a
    // document layer (canvas edge repeated, as Photoshop does) and on their own (transparent
    // beyond), at every depth, with and without alpha, 1 to 5 channels.
    let canvas = Rect::new(0, 0, 37, 23);
    let formats = [
        PixelFormat::RGBA8,
        PixelFormat::RGBA16,
        PixelFormat::RGBA32F,
        PixelFormat::GRAY8,
        PixelFormat::CMYKA8,
        PixelFormat::new(ColorMode::Lab, SampleType::F32, false),
    ];
    let params = [
        FilterParams::GaussianBlur { radius: 4.5 },
        FilterParams::GaussianBlur { radius: 9.3 },
        FilterParams::GaussianBlur { radius: 40.0 },
        FilterParams::BoxBlur { radius: 5.0 },
        FilterParams::BoxBlur { radius: 70.0 },
    ];
    for (i, fmt) in formats.into_iter().enumerate() {
        let s = noisy(fmt, canvas, 0x9e37_79b9 + i as u32);
        // A level of the depth (colour and alpha are each rounded on write) plus float noise.
        let tol = match fmt.sample {
            SampleType::U8 => 1.0 / 255.0,
            SampleType::U16 => 1.0 / 65535.0,
            _ => 0.0,
        } + 2e-5;
        for p in &params {
            let boxes = box_widths(p).expect("a box-pass blur");
            let reach: i32 = boxes.iter().map(|w| (w / 2) as i32).sum();
            let area = crate::output_area(p, s.content_bounds(), canvas, None);
            for (got, img, out) in [
                (crate::apply_in(&s, p, area, canvas, None, canvas), Image::read_clamped(&s, canvas.inflate(reach), canvas), canvas),
                (crate::apply(&s, p, area, canvas, None), Image::read(&s, area.inflate(reach)), area),
            ] {
                // Compared alpha-weighted: colour is meaningless where (nearly) transparent.
                let (mut got, mut want) = (got.read_region(out), direct_boxes(&img, out, &boxes, fmt.alpha));
                premultiply(&mut got, img.ch, fmt.alpha);
                premultiply(&mut want, img.ch, fmt.alpha);
                let err = got.iter().zip(&want).map(|(a, b)| (a - b).abs()).fold(0.0f32, f32::max);
                assert!(err <= tol, "{} on {fmt:?} over {out:?}: max error {err}", p.label());
            }
        }
    }
}

#[test]
fn layer_box_blurs_mix_by_the_selection_and_stop_when_cancelled() {
    let canvas = Rect::new(0, 0, 37, 23);
    let s = noisy(PixelFormat::RGBA32F, canvas, 7);
    let p = FilterParams::GaussianBlur { radius: 12.0 };
    let full = crate::apply_in(&s, &p, canvas, canvas, None, canvas);
    let half = Surface::with_default(PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false), &[0.5]);
    let mixed = crate::apply_in(&s, &p, canvas, canvas, Some(&half), canvas);
    for (x, y) in [(0, 0), (18, 11), (36, 22)] {
        let (o, f, m) = (s.pixel(x, y), full.pixel(x, y), mixed.pixel(x, y));
        assert!((m[3] - (o[3] + f[3]) / 2.0).abs() < 1e-5, "({x},{y}) alpha");
        for c in 0..3 {
            assert!((m[c] * m[3] - (o[c] * o[3] + f[c] * f[3]) / 2.0).abs() < 1e-5, "({x},{y}) premultiplied channel {c}");
        }
    }
    let cancel = || true;
    let ctl = Interrupt::cancel_only(&cancel);
    assert!(crate::apply_in_with(&s, &p, canvas, canvas, None, canvas, &ctl).is_none());
}

#[test]
fn surface_blur_8bit_histogram_matches_the_direct_sum() {
    // 8-bit levels with some fully transparent pixels, which the sum skips.
    let src_rect = Rect::new(-14, -14, 40, 34);
    let mut img = Image::new(src_rect, 4);
    let mut s = 0x6c07_8965u32;
    for px in img.data.as_chunks_mut::<4>().0 {
        for v in px.iter_mut() {
            s ^= s << 13;
            s ^= s >> 17;
            s ^= s << 5;
            *v = (s % 256) as f32 / 255.0;
        }
        if s.is_multiple_of(7) {
            px[3] = 0.0;
        }
    }
    let ctx = Ctx { bounds: src_rect, mode: crate::ColorMode::Rgb, alpha: true };
    // The output reaches the source edge, where samples read as transparent.
    let out = Rect::new(-2, 0, 26, 20);
    for (r, threshold) in [(1, 2.0), (4, 15.0), (12, 60.0), (12, 255.0)] {
        let t = (threshold / 255.0) * 2.5;
        let reach = ((t * 255.0f32).ceil() as usize).min(255);
        let fast = surface_8bit(&img, out, &ctx, r, t, reach).expect("8-bit input");
        let direct = surface_direct(&img, out, &ctx, r, t);
        let err = direct.iter().zip(&fast).map(|(a, b)| (a - b).abs()).fold(0.0f32, f32::max);
        assert!(err < 1e-5, "r {r} threshold {threshold}: max error {err}");
    }
    // Not 8-bit: the histogram declines and the direct sum is used.
    // (0, 0) is inside every window above.
    let i = (14 * src_rect.width() as usize + 14) * 4;
    img.data[i] = 0.5 / 255.0;
    assert!(surface_8bit(&img, out, &ctx, 4, 0.1, 26).is_none());
}

#[test]
fn motion_kernel_matches_the_per_pixel_samples() {
    // Noisy RGBA with some transparent pixels; the output reaches the source edge.
    let src_rect = Rect::new(-30, -30, 50, 40);
    let mut img = Image::new(src_rect, 4);
    let mut s = 0x1b87_3593u32;
    for px in img.data.as_chunks_mut::<4>().0 {
        for v in px.iter_mut() {
            s ^= s << 13;
            s ^= s >> 17;
            s ^= s << 5;
            *v = (s % 1000) as f32 / 999.0;
        }
        if s.is_multiple_of(5) {
            px[3] = 0.0;
        }
    }
    let out = Rect::new(-12, -6, 30, 22);
    for alpha in [true, false] {
        let ctx = Ctx { bounds: src_rect, mode: crate::ColorMode::Rgb, alpha };
        for (angle, distance) in [(0.0, 1.0), (0.0, 9.0), (30.0, 7.0), (90.0, 12.0), (-45.0, 20.0), (137.0, 33.5), (200.0, 40.0)] {
            let fast = motion(&img, out, &ctx, angle, distance);
            let offs = motion_offsets(angle, distance);
            let slow = average_samples(&img, out, &ctx, |x, y, pts| pts.extend(offs.iter().map(|&(dx, dy)| (x + dx, y + dy))));
            let err = slow.iter().zip(&fast).map(|(a, b)| (a - b).abs()).fold(0.0f32, f32::max);
            assert!(err < 1e-5, "alpha {alpha} angle {angle} distance {distance}: max error {err}");
        }
    }
}

/// The direct sum with f64 accumulators: the reference both fast paths are held to.
fn surface_reference(src: &Image, out: Rect, ctx: &Ctx, r: i32, t: f32) -> Vec<f32> {
    let n = src.ch;
    let cc = if ctx.alpha { n - 1 } else { n };
    let mut res = Vec::new();
    for y in out.y0..out.y1 {
        for x in out.x0..out.x1 {
            let p = src.px(x, y);
            for &v0 in p.iter().take(cc) {
                let c = res.len() % n;
                let (mut acc, mut wsum) = (0.0f64, 0.0f64);
                for yy in y - r..=y + r {
                    for xx in x - r..=x + r {
                        if ctx.alpha && src.get(xx, yy, n - 1) <= 0.0 {
                            continue;
                        }
                        let v = src.get(xx, yy, c);
                        let w = (1.0 - (v - v0).abs() / t).max(0.0);
                        acc += f64::from(v) * f64::from(w);
                        wsum += f64::from(w);
                    }
                }
                res.push(if wsum > 0.0 { (acc / wsum) as f32 } else { v0 });
            }
            if ctx.alpha {
                res.push(p[n - 1]);
            }
        }
    }
    res
}

#[test]
fn surface_blur_ranked_sums_match_the_weighted_window_at_any_depth() {
    // 16-bit levels and arbitrary floats (including negatives and values above 1), with some
    // fully transparent pixels, which the sum skips.
    for (quant, channels, alpha) in [(Some(65535.0f32), 4, true), (None, 4, true), (None, 1, false)] {
        let src_rect = Rect::new(-20, -20, 48, 40);
        let mut img = Image::new(src_rect, channels);
        let mut s = 0x1b87_3593u32;
        for px in img.data.chunks_mut(channels) {
            for v in px.iter_mut() {
                s ^= s << 13;
                s ^= s >> 17;
                s ^= s << 5;
                let raw = (s % 10_000) as f32 / 10_000.0;
                *v = quant.map_or(raw * 1.5 - 0.25, |q| (raw * q).round() / q);
            }
            if alpha && s.is_multiple_of(7) {
                px[channels - 1] = 0.0;
            }
        }
        let ctx = Ctx { bounds: src_rect, mode: crate::ColorMode::Rgb, alpha };
        // The output reaches the source edge, where samples read as 0.
        let out = Rect::new(-6, 0, 30, 22);
        for (r, threshold) in [(1, 2.0), (4, 15.0), (12, 60.0), (16, 255.0)] {
            let t = (threshold / 255.0) * 2.5;
            let ranked = surface_ranked(&img, out, &ctx, r, t).expect("finite input");
            let reference = surface_reference(&img, out, &ctx, r, t);
            assert_eq!(ranked.len(), reference.len());
            let err = reference.iter().zip(&ranked).map(|(a, b)| (a - b).abs()).fold(0.0f32, f32::max);
            assert!(err < 1e-6, "{quant:?} r {r} threshold {threshold}: max error {err}");
        }
    }
}

#[test]
fn surface_blur_ranked_declines_non_finite_samples_and_handles_empty_output() {
    let src_rect = Rect::new(0, 0, 8, 8);
    let mut img = Image::new(src_rect, 1);
    img.data.fill(0.5);
    let ctx = Ctx { bounds: src_rect, mode: crate::ColorMode::Grayscale, alpha: false };
    assert_eq!(surface_ranked(&img, Rect::new(2, 2, 2, 6), &ctx, 2, 0.1), Some(Vec::new()));
    img.data[9] = f32::NAN;
    assert!(surface_ranked(&img, Rect::new(0, 0, 8, 8), &ctx, 2, 0.1).is_none());
    // The public entry still answers, through the direct sum.
    assert_eq!(surface(&img, Rect::new(0, 0, 8, 8), &ctx, 12.0, 15.0).len(), 64);
}

/// Release-mode timings of the direct sum and the ranked path on one 256 px tile of
/// 16-bit-like and float noise (`cargo test -p photocraft-algo --release -- --ignored --nocapture surface_timing`).
#[test]
#[ignore]
fn surface_timing() {
    for (label, quant) in [("16-bit", Some(65535.0f32)), ("float", None)] {
        for r in [2, 4, 8, 16, 32, 64, 100] {
            let out = Rect::new(0, 0, 256, 256);
            let src_rect = out.inflate(r);
            let mut img = Image::new(src_rect, 4);
            let mut s = 0x2545_f491u32;
            let len = img.data.len();
            for (i, v) in img.data.iter_mut().enumerate() {
                s ^= s << 13;
                s ^= s >> 17;
                s ^= s << 5;
                // Smooth gradient plus noise, so the threshold keeps a realistic share.
                let x = (i / 4) as f32 / len as f32 * 4.0;
                let raw = (0.5 + 0.4 * (x * 6.0).sin() + (s % 1000) as f32 / 20000.0).clamp(0.0, 1.0);
                *v = quant.map_or(raw, |q| (raw * q).round() / q);
            }
            let ctx = Ctx { bounds: src_rect, mode: crate::ColorMode::Rgb, alpha: true };
            let t = (15.0 / 255.0) * 2.5;
            let a = std::time::Instant::now();
            let direct = surface_direct(&img, out, &ctx, r, t);
            let td = a.elapsed();
            let a = std::time::Instant::now();
            let ranked = surface_ranked(&img, out, &ctx, r, t).expect("finite");
            let tr = a.elapsed();
            let err = direct.iter().zip(&ranked).map(|(a, b)| (a - b).abs()).fold(0.0f32, f32::max);
            println!("{label} r {r:3}: direct {td:>10.2?}  ranked {tr:>10.2?}  max err {err:.2e}");
        }
    }
}

/// A column strip's passes stop between passes once cancelled, not only before the strip
/// (#6205); uncancelled, every pass runs.
#[test]
fn line_passes_stop_between_passes_when_cancelled() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let (radii, w, len) = ([2usize, 3, 2], 4usize, 50usize);
    let reach: usize = radii.iter().sum();
    let line: Vec<f32> = (0..(len + 2 * reach) * w).map(|i| (i % 7) as f32 + 1.0).collect();
    let run = |ctl: &Interrupt| {
        let mut s = Scratch { line: line.clone(), ..Scratch::default() };
        let mut dst = vec![0.0f32; len * w];
        line_passes(&mut dst, w, &radii, &mut s, ctl);
        dst
    };
    assert!(run(&Interrupt::NONE).iter().all(|v| *v > 0.0));
    let calls = AtomicUsize::new(0);
    let cancel = || calls.fetch_add(1, Ordering::SeqCst) >= 1;
    // Cancelled after the first pass: the last pass (the one writing `dst`) never runs.
    assert!(run(&Interrupt::cancel_only(&cancel)).iter().all(|v| *v == 0.0));
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

/// A box blur cancelled at any of its checks returns `None` (never a half-blurred result);
/// one that is asked but never cancelled is bit-identical to the uninterruptible run.
#[test]
fn box_blur_honours_a_cancel_at_every_check() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let (n, src) = (4usize, Rect::new(0, 0, 150, 90));
    let px: Vec<f32> = (0..src.width() as usize * src.height() as usize * n).map(|i| (i as f32 * 0.13).sin() * 0.5 + 0.5).collect();
    let read = |r: Rect, buf: &mut Vec<f32>| {
        buf.clear();
        let (x0, x1) = ((r.x0 * n as i32) as usize, (r.x1 * n as i32) as usize);
        let row = r.y0 as usize * src.width() as usize * n;
        buf.extend_from_slice(&px[row + x0..row + x1]);
    };
    let Some(boxes) = box_widths(&FilterParams::GaussianBlur { radius: 12.0 }) else { panic!("box passes") };
    let out = Rect::new(-5, 3, 155, 80);
    let want = box_blur(src, &read, out, n, true, &boxes, &Interrupt::NONE).unwrap();
    let checks = AtomicUsize::new(0);
    let count = || {
        checks.fetch_add(1, Ordering::SeqCst);
        false
    };
    let got = box_blur(src, &read, out, n, true, &boxes, &Interrupt::cancel_only(&count)).unwrap();
    assert_eq!(got.len(), want.len());
    for ((ta, a), (tb, b)) in got.iter().zip(&want) {
        assert_eq!(ta, tb);
        assert!(a.iter().zip(b).all(|(x, y)| x.to_bits() == y.to_bits()), "strip {ta:?} differs");
    }
    let total = checks.load(Ordering::SeqCst);
    for k in (0..total).step_by((total / 40).max(1)) {
        let seen = AtomicUsize::new(0);
        let cancel = || seen.fetch_add(1, Ordering::SeqCst) >= k;
        assert!(box_blur(src, &read, out, n, true, &boxes, &Interrupt::cancel_only(&cancel)).is_none(), "cancelled at check {k}");
    }
}
