//! The pre-#7921 Smart Blur (every neighbour read per channel through `Image::get`) stays as an
//! exact oracle: the offset fast path must give the same bits for every mode, depth and layout.
use super::{BlurQuality, Ctx, Image, MAXC, Rect, SmartBlurMode, native, ncol, smart_blur, unpremul_px, xy};
use photocraft_color::ColorMode;

#[allow(clippy::too_many_arguments)]
fn smart_blur_oracle(src: &Image, out: Rect, ctx: &Ctx, radius: f32, threshold: f32, quality: BlurQuality, mode: SmartBlurMode) -> Vec<f32> {
    let n = src.ch;
    let cc = ncol(ctx, n);
    let r = radius.clamp(0.1, 100.0);
    let t = threshold.clamp(0.1, 100.0) / 255.0;
    let ri = r.ceil() as i32;
    let target = match quality {
        BlurQuality::Low => 4,
        BlurQuality::Medium => 7,
        BlurQuality::High => 12,
    };
    let stride = ((ri + target - 1) / target).max(1);
    let offs: Vec<(i32, i32)> = (-ri..=ri)
        .step_by(stride as usize)
        .flat_map(|dy| (-ri..=ri).step_by(stride as usize).map(move |dx| (dx, dy)))
        .filter(|(dx, dy)| ((dx * dx + dy * dy) as f32) <= r * r + 0.5)
        .collect();
    let white = native(ctx, [1.0, 1.0, 1.0, 1.0]);
    let black = native(ctx, [0.0, 0.0, 0.0, 1.0]);
    let mut res = src.crop(out);
    let mut q = [0.0f32; MAXC];
    let mut acc = [0.0f32; MAXC];
    for (i, px) in res.chunks_exact_mut(n).enumerate() {
        let (x, y) = xy(out, i);
        let me: [f32; MAXC] = std::array::from_fn(|c| if c < n { px[c] } else { 0.0 });
        let differs = |q: &[f32]| (0..cc).any(|c| (q[c] - me[c]).abs() > t);
        if mode != SmartBlurMode::Normal {
            let mut edge = false;
            for (dx, dy) in [(1, 0), (0, 1), (-1, 0), (0, -1)] {
                for (c, v) in q.iter_mut().enumerate().take(n) {
                    *v = src.get(x + dx, y + dy, c);
                }
                if differs(&q[..n]) {
                    edge = true;
                    break;
                }
            }
            let a = if ctx.alpha { px[n - 1] } else { 1.0 };
            if edge {
                px[..cc].copy_from_slice(&white[..cc]);
            } else if mode == SmartBlurMode::EdgeOnly {
                px[..cc].copy_from_slice(&black[..cc]);
            }
            if ctx.alpha {
                px[n - 1] = a;
            }
            continue;
        }
        acc[..n].fill(0.0);
        let mut wsum = 0.0f32;
        for &(dx, dy) in &offs {
            for (c, v) in q.iter_mut().enumerate().take(n) {
                *v = src.get(x + dx, y + dy, c);
            }
            if differs(&q[..n]) {
                continue;
            }
            let a = if ctx.alpha { q[n - 1] } else { 1.0 };
            for c in 0..n {
                acc[c] += if ctx.alpha && c < n - 1 { q[c] * a } else { q[c] };
            }
            wsum += 1.0;
        }
        if wsum > 0.0 {
            for c in 0..n {
                px[c] = acc[c] / wsum;
            }
            unpremul_px(px, ctx.alpha);
        }
    }
    res
}

/// Flat blocks with small noise, so some neighbours fall within the threshold and some don't.
fn image(rect: Rect, n: usize, alpha: bool) -> Image {
    let w = rect.width() as usize;
    let data = (0..w * rect.height() as usize)
        .flat_map(|i| {
            let (x, y) = (i % w, i / w);
            let block = ((x / 5 + y / 7) % 3) as f32 * 0.3;
            (0..n).map(move |c| {
                let noise = ((i * 31 + c * 13) % 17) as f32 / 160.0;
                if alpha && c + 1 == n { [1.0, 0.0, 0.4, 0.75][(x + y) % 4] } else { block + noise }
            })
        })
        .collect();
    Image { rect, ch: n, data }
}

fn exact(a: &[f32], b: &[f32], what: &str) {
    assert_eq!(a.len(), b.len(), "{what}");
    for (i, (a, b)) in a.iter().zip(b).enumerate() {
        assert!(a.to_bits() == b.to_bits() || (a.is_nan() && b.is_nan()), "{what}: sample {i}: {a} != {b}");
    }
}

#[test]
fn offset_fast_path_matches_the_per_channel_oracle() {
    let mut layouts: Vec<(ColorMode, usize, bool)> = vec![
        (ColorMode::Rgb, 3, false),
        (ColorMode::Rgb, 4, true),
        (ColorMode::Grayscale, 1, false),
        (ColorMode::Grayscale, 2, true),
        (ColorMode::Cmyk, 4, false),
        (ColorMode::Cmyk, 5, true),
        (ColorMode::Lab, 4, true),
    ];
    layouts.extend((1..=MAXC).flat_map(|n| [(ColorMode::Multichannel, n, false), (ColorMode::Multichannel, n, true)]));
    let win = Rect::new(-7, -4, 33, 30);
    for (mode, n, alpha) in layouts {
        let src = image(win, n, alpha);
        let ctx = Ctx { bounds: win, mode, alpha };
        for radius in [0.1f32, 1.0, 3.0, 12.0, 17.5] {
            let ri = f32::ceil(radius) as i32;
            // The usual tile (the window less the halo), the whole window, and one sticking out of it.
            let inner = Rect::new(win.x0 + ri, win.y0 + ri, win.x1 - ri, win.y1 - ri);
            for out in [inner, win, Rect::new(-12, -9, 10, 8), Rect::EMPTY] {
                for quality in [BlurQuality::Low, BlurQuality::Medium, BlurQuality::High] {
                    for sm in [SmartBlurMode::Normal, SmartBlurMode::EdgeOnly, SmartBlurMode::OverlayEdge] {
                        for threshold in [8.0, 60.0] {
                            let what = format!("{mode:?} n={n} alpha={alpha} r={radius} {out:?} {quality:?} {sm:?} t={threshold}");
                            exact(
                                &smart_blur(&src, out, &ctx, radius, threshold, quality, sm),
                                &smart_blur_oracle(&src, out, &ctx, radius, threshold, quality, sm),
                                &what,
                            );
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn out_of_window_neighbours_read_as_zero() {
    // A one-pixel window: every neighbour is outside and reads as transparent black, so a black
    // centre averages with all of them and a white one keeps itself.
    let ctx = Ctx { bounds: Rect::new(0, 0, 1, 1), mode: ColorMode::Rgb, alpha: true };
    for v in [0.0, 1.0] {
        let src = Image { rect: Rect::new(0, 0, 1, 1), ch: 4, data: vec![v, v, v, 1.0] };
        let got = smart_blur(&src, src.rect, &ctx, 3.0, 25.0, BlurQuality::High, SmartBlurMode::Normal);
        exact(&got, &smart_blur_oracle(&src, src.rect, &ctx, 3.0, 25.0, BlurQuality::High, SmartBlurMode::Normal), "1 px");
    }
}

#[test]
fn nan_and_infinite_samples_match_the_oracle() {
    // A NaN difference is never over the threshold, so it is averaged in; an infinite one is.
    let win = Rect::new(0, 0, 24, 20);
    for (n, alpha) in [(3, false), (4, true)] {
        let mut src = image(win, n, alpha);
        for (i, v) in [(5, f32::NAN), (97, f32::INFINITY), (230, f32::NEG_INFINITY), (311, -0.0)] {
            let len = src.data.len();
            src.data[(i * n) % len] = v;
        }
        let ctx = Ctx { bounds: win, mode: ColorMode::Rgb, alpha };
        let out = Rect::new(4, 4, 20, 16);
        for threshold in [8.0, 100.0] {
            exact(
                &smart_blur(&src, out, &ctx, 4.0, threshold, BlurQuality::High, SmartBlurMode::Normal),
                &smart_blur_oracle(&src, out, &ctx, 4.0, threshold, BlurQuality::High, SmartBlurMode::Normal),
                &format!("n={n} t={threshold}"),
            );
        }
    }
}
