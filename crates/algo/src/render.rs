//! Render: Clouds, Difference Clouds, Fibers, Lighting Effects (Lens Flare: `lens_flare.rs`).

use photocraft_geom::Rect;

use crate::fxutil::{MAXC, luma, rgba, set_rgba, smoothstep, xy};
use crate::image::Image;
use crate::noise::hash01;
use crate::{Ctx, Light, LightKind, TextureChannel};

/// Smooth value noise in [0, 1).
fn value_noise(x: f32, y: f32, seed: u32) -> f32 {
    let (ix, iy) = (x.floor() as i32, y.floor() as i32);
    let (tx, ty) = (x - ix as f32, y - iy as f32);
    let (sx, sy) = (tx * tx * (3.0 - 2.0 * tx), ty * ty * (3.0 - 2.0 * ty));
    let h = |a: i32, b: i32| hash01(a, b, 0, seed);
    let top = h(ix, iy) + (h(ix + 1, iy) - h(ix, iy)) * sx;
    let bot = h(ix, iy + 1) + (h(ix + 1, iy + 1) - h(ix, iy + 1)) * sx;
    top + (bot - top) * sy
}

fn cloud_hash(x: i32, y: i32, seed: u32) -> f32 {
    let mut h = (x as u32).wrapping_mul(0x8da6_b343) ^ (y as u32).wrapping_mul(0xd816_3841) ^ seed.wrapping_mul(0xcb1a_b31f);
    h ^= h >> 13;
    h = h.wrapping_mul(0x5bd1_e995);
    h ^= h >> 15;
    (h & 0x00ff_ffff) as f32 / 0x00ff_ffff as f32
}

/// Smooth value noise, summed over octaves (fBm), in 0..1.
fn clouds_value(x: f32, y: f32, base: f32, seed: u32) -> f32 {
    let (mut sum, mut amp, mut freq, mut norm) = (0.0, 1.0, 1.0 / base, 0.0);
    for o in 0..8u32 {
        let (fx, fy) = (x * freq, y * freq);
        let (ix, iy) = (fx.floor() as i32, fy.floor() as i32);
        let (tx, ty) = (fx - ix as f32, fy - iy as f32);
        let (sx, sy) = (tx * tx * (3.0 - 2.0 * tx), ty * ty * (3.0 - 2.0 * ty));
        let s = seed.wrapping_add(o * 7919);
        let top = cloud_hash(ix, iy, s) + (cloud_hash(ix + 1, iy, s) - cloud_hash(ix, iy, s)) * sx;
        let bot = cloud_hash(ix, iy + 1, s) + (cloud_hash(ix + 1, iy + 1, s) - cloud_hash(ix, iy + 1, s)) * sx;
        sum += (top + (bot - top) * sy) * amp;
        norm += amp;
        amp *= 0.5;
        freq *= 2.0;
    }
    // Stretch the bell-shaped sum around mid-grey so the clouds use most of the
    // foreground→background range. A straight stretch clamped at 0 and 1 flattened its tails
    // into pure foreground and background (#9959); this S-curve, x/√(1+x²) scaled so it meets
    // 0 and 1 only where the sum does (at ±h it is ±h/CLOUD_CONTRAST), compresses them instead.
    let h = (CLOUD_CONTRAST * CLOUD_CONTRAST - 1.0).sqrt();
    let d = (sum / norm - 0.5) * 2.0 * h;
    (0.5 + 0.5 * CLOUD_CONTRAST / h * d / (1.0 + d * d).sqrt()).clamp(0.0, 1.0)
}

/// The clouds' contrast at mid-grey: the slope of the octave sum's stretch there.
const CLOUD_CONTRAST: f32 = 2.2;

/// Clouds between the foreground and background colours; Difference Clouds subtracts them from
/// the pixels (alpha kept). The cloud size is a quarter of the bounds' longer side, and the
/// pattern is anchored at the bounds' corner (so it moves with a smart object).
pub(crate) fn clouds(src: &Image, out: Rect, ctx: &Ctx, seed: u32, difference: bool, colours: ([f32; 4], [f32; 4])) -> Vec<f32> {
    let n = src.ch;
    let (fg, bg) = colours;
    let base = (ctx.bounds.width().max(ctx.bounds.height()) as f32 / 4.0).clamp(16.0, 512.0);
    let mut res = src.crop(out);
    for (i, px) in res.chunks_exact_mut(n).enumerate() {
        let (x, y) = xy(out, i);
        let (x, y) = (x - ctx.bounds.x0, y - ctx.bounds.y0);
        let t = clouds_value(x as f32, y as f32, base, seed);
        let c: [f32; 3] = std::array::from_fn(|k| fg[k] + (bg[k] - fg[k]) * t);
        if difference {
            let p = rgba(ctx, px);
            set_rgba(ctx, px, [(p[0] - c[0]).abs(), (p[1] - c[1]).abs(), (p[2] - c[2]).abs(), p[3]]);
        } else {
            set_rgba(ctx, px, [c[0], c[1], c[2], 1.0]);
        }
    }
    res
}

/// Fibers: vertical fibrous streaks between the foreground and background
/// colours. Variance sets how quickly the colour varies across fibres (and
/// how short they are); strength stretches each fibre vertically.
#[allow(clippy::too_many_arguments)]
pub(crate) fn fibers(src: &Image, out: Rect, ctx: &Ctx, variance: f32, strength: f32, seed: u32, colours: ([f32; 4], [f32; 4])) -> Vec<f32> {
    let n = src.ch;
    let var = variance.clamp(1.0, 64.0);
    let st = strength.clamp(1.0, 64.0);
    let (fg, bg) = colours;
    // Horizontal frequency rises with variance; vertical frequency falls with strength.
    let fx = 0.08 + var / 64.0 * 0.6;
    let fy = fx / (2.0 + st * 1.5) * (0.5 + var / 32.0);
    let mut res = src.crop(out);
    for (i, px) in res.chunks_exact_mut(n).enumerate() {
        // Anchored at the bounds' corner, so the fibres move with a smart object.
        let (x, y) = xy(out, i);
        let (xf, yf) = ((x - ctx.bounds.x0) as f32, (y - ctx.bounds.y0) as f32);
        let mut sum = 0.0;
        let mut amp = 1.0;
        let mut norm = 0.0;
        let (mut ax, mut ay) = (fx, fy);
        for o in 0..4u32 {
            sum += value_noise(xf * ax, yf * ay, seed.wrapping_add(o * 977)) * amp;
            norm += amp;
            amp *= 0.55;
            ax *= 2.0;
            ay *= 2.0;
        }
        // Fine per-column grain gives the hairline texture.
        let grain = value_noise(xf * 0.9, yf * fy * 0.25, seed ^ 0x5bd1) - 0.5;
        let t = ((sum / norm - 0.5) * (1.6 + var / 40.0) + 0.5 + grain * 0.25).clamp(0.0, 1.0);
        let c = [fg[0] + (bg[0] - fg[0]) * t, fg[1] + (bg[1] - fg[1]) * t, fg[2] + (bg[2] - fg[2]) * t, 1.0];
        set_rgba(ctx, px, c);
    }
    res
}

pub(crate) struct LightingSpec<'a> {
    pub lights: &'a [Light],
    pub gloss: f32,
    pub metallic: f32,
    pub exposure: f32,
    pub ambience: f32,
    pub texture: TextureChannel,
    pub height: f32,
    pub white_is_high: bool,
}

fn norm3(v: [f32; 3]) -> [f32; 3] {
    let m = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt().max(1e-9);
    [v[0] / m, v[1] / m, v[2] / m]
}
fn dot3(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// Lighting Effects: Blinn–Phong shading of the image as a surface (bumped by
/// a texture channel) under spot, point and infinite lights.
pub(crate) fn lighting(src: &Image, out: Rect, ctx: &Ctx, spec: &LightingSpec) -> Vec<f32> {
    let n = src.ch;
    let b = ctx.bounds;
    let (bw, bh) = (b.width() as f32, b.height() as f32);
    let short = bw.min(bh).max(1.0);
    let hscale = spec.height.clamp(0.0, 100.0) / 100.0 * 0.06 * short;
    let shininess = 2f32.powf(1.0 + (spec.gloss.clamp(-100.0, 100.0) + 100.0) / 200.0 * 7.0);
    let spec_k = (spec.gloss.clamp(-100.0, 100.0) + 100.0) / 200.0 * 0.7;
    let metal = (spec.metallic.clamp(-100.0, 100.0) + 100.0) / 200.0;
    let expo = 2f32.powf(spec.exposure.clamp(-100.0, 100.0) / 50.0);
    let ambient = spec.ambience.clamp(-100.0, 100.0) / 100.0 * 0.5;
    let mut tmp = [0.0f32; MAXC];
    let tex = |x: i32, y: i32, tmp: &mut [f32; MAXC]| -> f32 {
        let xx = x.clamp(b.x0, b.x1 - 1);
        let yy = y.clamp(b.y0, b.y1 - 1);
        for (c, t) in tmp.iter_mut().enumerate().take(n) {
            *t = src.get(xx, yy, c);
        }
        let v = match spec.texture {
            TextureChannel::None => 0.0,
            TextureChannel::Alpha => {
                if ctx.alpha {
                    tmp[n - 1]
                } else {
                    1.0
                }
            }
            TextureChannel::Luminance => luma(ctx, &tmp[..n]),
            TextureChannel::Red | TextureChannel::Green | TextureChannel::Blue => {
                let k = match spec.texture {
                    TextureChannel::Red => 0,
                    TextureChannel::Green => 1,
                    _ => 2,
                };
                if ctx.mode == photocraft_color::ColorMode::Rgb { tmp[k] } else { rgba(ctx, &tmp[..n])[k] }
            }
        };
        if spec.white_is_high { v } else { 1.0 - v }
    };
    let lights: Vec<Light> = spec.lights.to_vec();
    let mut res = src.crop(out);
    for (i, px) in res.chunks_exact_mut(n).enumerate() {
        let (x, y) = xy(out, i);
        let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
        let nrm = if spec.texture == TextureChannel::None || hscale <= 0.0 {
            [0.0, 0.0, 1.0]
        } else {
            let hx = (tex(x + 1, y, &mut tmp) - tex(x - 1, y, &mut tmp)) * 0.5 * hscale;
            let hy = (tex(x, y + 1, &mut tmp) - tex(x, y - 1, &mut tmp)) * 0.5 * hscale;
            norm3([-hx, -hy, 1.0])
        };
        let mut c = rgba(ctx, px);
        let base = [c[0], c[1], c[2]];
        let mut lit = [ambient; 3];
        let mut spc = [0.0f32; 3];
        for l in &lights {
            let lpos = [b.x0 as f32 + l.x * bw, b.y0 as f32 + l.y * bh, l.z * short];
            let (ldir, atten) = match l.kind {
                LightKind::Infinite => {
                    let (s, co) = l.angle.to_radians().sin_cos();
                    let e = l.elevation.clamp(0.0, 90.0).to_radians();
                    (norm3([co * e.cos(), -s * e.cos(), e.sin()]), 1.0)
                }
                LightKind::Point => {
                    let v = [lpos[0] - fx, lpos[1] - fy, lpos[2]];
                    let dist = (v[0] * v[0] + v[1] * v[1]).sqrt();
                    let reach = (l.radius * short).max(1.0);
                    (norm3(v), (1.0 - dist / reach).clamp(0.0, 1.0).powi(2))
                }
                LightKind::Spot => {
                    let v = [lpos[0] - fx, lpos[1] - fy, lpos[2]];
                    let ld = norm3(v);
                    let tgt = [b.x0 as f32 + l.target_x * bw, b.y0 as f32 + l.target_y * bh, 0.0];
                    let axis = norm3([tgt[0] - lpos[0], tgt[1] - lpos[1], tgt[2] - lpos[2]]);
                    let cosang = -dot3(ld, axis);
                    let outer = l.cone.clamp(1.0, 89.0).to_radians().cos();
                    let inner = (l.cone.clamp(1.0, 89.0) * (l.hotspot.clamp(0.0, 100.0) / 100.0)).to_radians().cos();
                    (ld, smoothstep(outer, inner.max(outer + 1e-4), cosang))
                }
            };
            if atten <= 0.0 {
                continue;
            }
            let inten = l.intensity.clamp(-100.0, 100.0) / 50.0 * atten;
            let ndl = dot3(nrm, ldir).max(0.0);
            let hv = norm3([ldir[0], ldir[1], ldir[2] + 1.0]);
            let s = dot3(nrm, hv).max(0.0).powf(shininess) * spec_k;
            for k in 0..3 {
                lit[k] += ndl * inten * l.color[k];
                // Metallic highlights take the surface colour; plastic ones the light's.
                let sc = l.color[k] * (1.0 - metal) + base[k] * metal;
                spc[k] += s * inten.max(0.0) * sc;
            }
        }
        for k in 0..3 {
            c[k] = ((base[k] * lit[k] + spc[k]) * expo).max(0.0);
        }
        set_rgba(ctx, px, c);
    }
    res
}

#[cfg(test)]
mod tests {
    use crate::{FilterParams, apply, output_area};
    use photocraft_color::{ColorMode, PixelFormat, SampleType};
    use photocraft_geom::Rect;
    use photocraft_raster::Surface;

    const BLACK: [f32; 4] = [0.0, 0.0, 0.0, 1.0];
    const WHITE: [f32; 4] = [1.0; 4];

    /// Clouds (or Difference Clouds over `under`) on a `w`×`h` canvas, as the stored pixels.
    fn render(fmt: PixelFormat, w: i32, h: i32, seed: u32, colours: ([f32; 4], [f32; 4]), under: Option<&[f32]>) -> Vec<f32> {
        let r = Rect::new(0, 0, w, h);
        let mut s = Surface::new(fmt);
        if let Some(px) = under {
            s.fill_rect(r, px);
        }
        let p = FilterParams::Clouds { seed, difference: under.is_some(), foreground: colours.0, background: colours.1 };
        apply(&s, &p, output_area(&p, s.content_bounds(), r, None), r, None).read_region(r)
    }

    fn top_level(depth: SampleType) -> f32 {
        if depth == SampleType::U8 { 255.0 } else { 65535.0 }
    }

    /// Clouds' histogram is a bell around mid-grey that spans most of the levels without piling
    /// up at either end: a straight stretch clamped at 0 and 1 put 1–4 % of a 512×512 render on
    /// each end (#9959).
    #[test]
    fn clouds_span_the_levels_without_clipping() {
        for depth in [SampleType::U8, SampleType::U16] {
            let top = top_level(depth);
            for mode in [ColorMode::Rgb, ColorMode::Grayscale] {
                let fmt = PixelFormat::new(mode, depth, true);
                for (w, h) in [(512, 512), (900, 240), (2400, 1200)] {
                    for seed in [0, 3, 7, 18916] {
                        let px = render(fmt, w, h, seed, (BLACK, WHITE), None);
                        let v: Vec<f32> = px.chunks_exact(fmt.channels()).map(|p| p[0]).collect();
                        let n = v.len() as f32;
                        let at = |level: f32| v.iter().filter(|&&t| (t * top).round() == level).count() as f32 / n;
                        let case = format!("{mode:?} {depth:?} {w}×{h} seed {seed}");
                        assert!(at(0.0) < 1e-4 && at(top) < 1e-4, "{case}: clipped {} black, {} white", at(0.0), at(top));
                        let (lo, hi) = v.iter().fold((1.0f32, 0.0f32), |(a, b), &t| (a.min(t), b.max(t)));
                        assert!(hi - lo > 0.7, "{case}: spans only {lo}..{hi}");
                        // Only a few big cells on a long, thin canvas, so its mean wanders more.
                        let mean = v.iter().sum::<f32>() / n;
                        assert!((0.35..0.65).contains(&mean), "{case}: mean {mean}");
                        // A flat histogram would put a quarter of the pixels in the outer eighths.
                        let outer = v.iter().filter(|t| !(0.125..0.875).contains(*t)).count() as f32 / n;
                        assert!(outer < 0.12, "{case}: {outer} in the outer eighths");
                    }
                }
            }
        }
    }

    /// Clouds blend between any two colours without reaching either: no channel sits on the
    /// foreground's or background's level, and every channel stays between them.
    #[test]
    fn clouds_stay_strictly_between_any_two_colours() {
        let (fg, bg) = ([0.85, 0.2, 0.1, 1.0], [0.1, 0.55, 0.9, 1.0]);
        for depth in [SampleType::U8, SampleType::U16] {
            let top = top_level(depth);
            let fmt = PixelFormat::new(ColorMode::Rgb, depth, true);
            for seed in [3, 18916] {
                let data = render(fmt, 512, 512, seed, (fg, bg), None);
                let px = data.as_chunks::<4>().0;
                let n = px.len() as f32;
                for k in 0..3 {
                    let ends = [(fg[k] * top).round(), (bg[k] * top).round()];
                    let on_end = px.iter().filter(|p| ends.contains(&(p[k] * top).round())).count() as f32 / n;
                    assert!(on_end < 1e-4, "{depth:?} seed {seed} channel {k}: {on_end} on a colour's level");
                    let (lo, hi) = (fg[k].min(bg[k]) - 0.5 / top, fg[k].max(bg[k]) + 0.5 / top);
                    assert!(px.iter().all(|p| (lo..=hi).contains(&p[k])), "{depth:?} seed {seed} channel {k} left the colours");
                }
            }
        }
    }

    /// Difference Clouds shares the clouds: over white with black and white they are the clouds
    /// inverted, so they don't clip either.
    #[test]
    fn difference_clouds_over_white_are_the_clouds_inverted() {
        for depth in [SampleType::U8, SampleType::U16] {
            let top = top_level(depth);
            let fmt = PixelFormat::new(ColorMode::Rgb, depth, true);
            let clouds = render(fmt, 300, 200, 5, (BLACK, WHITE), None);
            let diff = render(fmt, 300, 200, 5, (BLACK, WHITE), Some(&WHITE));
            let (clouds, diff) = (clouds.as_chunks::<4>().0, diff.as_chunks::<4>().0);
            for (c, d) in clouds.iter().zip(diff) {
                assert!((c[0] + d[0] - 1.0).abs() <= 1.0 / top, "{depth:?}: {} + {} ≠ 1", c[0], d[0]);
                assert_eq!(d[3], 1.0);
            }
            assert!(diff.iter().all(|p| (p[0] * top).round() != 0.0 && (p[0] * top).round() != top), "{depth:?}: clipped");
        }
    }
}
