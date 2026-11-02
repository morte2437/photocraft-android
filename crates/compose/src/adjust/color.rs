use super::tone::LUT_SIZE;
use super::{D50, HueRange, SRGB_TO_XYZ_D50, SampleType, Transfer, XYZ_D50_TO_SRGB, mat3_mul, srgb_to_linear};

/// Hue/Saturation's range edits as three hue-indexed tables (hue shift in degrees, saturation and
/// lightness as fractions), sampled at hue `k / (len - 1)` turns. Ranges add up where they overlap.
pub fn hue_range_tables(ranges: &[HueRange; 6]) -> [Vec<f32>; 3] {
    let at = |k: usize| {
        let deg = k as f32 / (LUT_SIZE - 1) as f32 * 360.0;
        ranges.iter().filter(|r| !r.is_neutral()).fold([0.0f32; 3], |acc, r| {
            let w = r.weight(deg);
            [acc[0] + w * r.hue, acc[1] + w * r.saturation / 100.0, acc[2] + w * r.lightness / 100.0]
        })
    };
    let rows: Vec<[f32; 3]> = (0..LUT_SIZE).map(at).collect();
    std::array::from_fn(|i| rows.iter().map(|r| r[i]).collect())
}

/// Hue/Saturation's saturation `s` (-1..1) as [`hue_saturation`] takes it in a document of
/// `depth`. A 32-bit Colorize builds its colour with the saturation times `1 - |lightness|`:
/// the photoshop corpus's rgb32 hue-saturation-colorize.psd (Saturation 40, Lightness +5) has 0.95
/// of the chroma at every lightness, and is exact so, where 8 and 16 bits keep the full saturation
/// (their oracles are exact without it). Only a raised Lightness is measured; a lowered one is
/// taken to scale the same way.
pub fn colorize_saturation(s: f32, lightness: f32, colorize: bool, depth: Option<SampleType>) -> f32 {
    // Clamped here too: a PSD can store a saturation beyond ±100, and the GPU scales it unclamped.
    if colorize && depth == Some(SampleType::F32) { s.clamp(-1.0, 1.0) * (1.0 - lightness.abs().min(1.0)) } else { s }
}

/// One Hue/Saturation evaluation: hue shift in degrees, saturation and lightness in -1..=1.
/// Colorize sets the hue and saturation instead of shifting them; at 0% it leaves only the
/// lightness, as in Photoshop (#2326). Ticking Colorize in the dialog starts saturation at 25%.
pub fn hue_saturation(c: [f32; 3], hue: f32, s: f32, l: f32, colorize: bool) -> [f32; 3] {
    hue_saturation_in(c, hue, s, l, colorize, None)
}

/// [`hue_saturation`] in a document of `depth`. At 32 bits Photoshop doesn't clip the raised
/// saturation, so saturated colours leave 0..1 (red with Hue +40, Saturation +25, Lightness -10
/// becomes (1.0477, 0.649, -0.1477)), and it scales each colour's chroma around its lightness by
/// [`hdr_saturation_factor`], so colours at or above lightness 1 keep their hue too: the photoshop
/// corpus's rgb32 hue-saturation.psd is exact so.
pub fn hue_saturation_in(c: [f32; 3], hue: f32, s: f32, l: f32, colorize: bool, depth: Option<SampleType>) -> [f32; 3] {
    let hdr = depth == Some(SampleType::F32);
    let lighten = |v: f32| if l > 0.0 { v + (1.0 - v) * l } else { v * (1.0 + l) };
    if colorize {
        // Colorize builds the colour at the lightened (or darkened) lightness, so black with
        // Lightness +30 becomes a 30 % lightness colour, not grey (measured in Photoshop 27.11).
        let (_, _, ll) = rgb_to_hsl(c);
        return hsl_to_rgb(hue.rem_euclid(360.0) / 360.0, s.abs().min(1.0), lighten(ll));
    }
    // Photoshop applies Lightness first, then rotates the hue and scales the saturation of the
    // result: Hue +40, Saturation +25, Lightness -10 turns (240, 160, 30) into (173, 243, 0), not
    // (168, 230, 14) (Photoshop 27.11 corpus file adjustments/rgb8/hue-saturation).
    let lc = c.map(lighten);
    let (hh, ss, ll) = rgb_to_hsl(lc);
    let hh = (hh + hue / 360.0).rem_euclid(1.0);
    if hdr {
        // The chroma, not the HSL saturation, whose `2 - max - min` divisor is 0 at lightness 1
        // and negative above it: (2, 0, 0) with Hue +40 becomes (2, 1.333, 0) in Photoshop.
        let d = (lc[0].max(lc[1]).max(lc[2]) - lc[0].min(lc[1]).min(lc[2])) * hdr_saturation_factor(s);
        return hue_rgb(hh, ll - d / 2.0, ll + d / 2.0);
    }
    // Raising saturation divides by 1 - s, lowering it multiplies by 1 + s: +50 % doubles it
    // (measured in Photoshop 27.11); +100 % saturates any colour that has some.
    let ss = if s > 0.0 { ss / (1.0 - s).max(1.0 / 512.0) } else { ss * (1.0 + s) }.clamp(0.0, 1.0);
    hsl_to_rgb(hh, ss, ll)
}

/// The factor 32-bit Hue/Saturation scales chroma by for saturation `s` (-1..1), as Photoshop
/// computes it (measured at every whole percent from -100 to +100 on a 32-bit document): lowering
/// cuts `s` to 1/256 steps, raising cuts it to 254 steps out of 255, takes 255 / (255 - steps) to
/// 1/256 and stops at 32767/256, so +25 % gives 340/256, +99 % 63.75 and +100 % 127.996.
/// compose.wgsl's Hue/Saturation does the same in whole numbers.
pub fn hdr_saturation_factor(s: f32) -> f32 {
    // The 1e-4 keeps a master plus range sum on its step: -0.46 + 0.96 is 0.49999997 in f32.
    let s = s.clamp(-1.0, 1.0);
    if s <= 0.0 {
        return 1.0 - (-s * 256.0 + 1e-4).floor() / 256.0;
    }
    let steps = (s * 254.0 + 1e-4).floor() as u32;
    (65280 / 255u32.saturating_sub(steps).max(1)).min(32767) as f32 / 256.0
}

/// A colour range's Lightness, applied after the master edits. It moves each channel towards the
/// pixel's brightest channel when raised and towards its darkest when lowered, not towards white
/// or black: Blues +50 turns (0, 29, 115) into (57, 71, 115), Blues -50 turns (102, 128, 204) into
/// (102, 115, 153) (measured in Photoshop 27.11).
pub fn range_lightness(c: [f32; 3], l: f32) -> [f32; 3] {
    let l = l.clamp(-1.0, 1.0);
    let target = if l > 0.0 { c[0].max(c[1]).max(c[2]) } else { c[0].min(c[1]).min(c[2]) };
    c.map(|v| v + (target - v) * l.abs())
}

/// Black & White grey value. Each colour is the sum of a grey part (its smallest channel), a
/// secondary-colour part (yellow, cyan or magenta: middle − smallest) and a primary part (red,
/// green or blue: largest − middle); the sliders (percent, Photoshop defaults 40 60 40 60 20 80 for
/// reds, yellows, greens, cyans, blues, magentas) weight the two colour parts. Unclipped: the
/// document's depth decides the range.
pub fn black_white_gray(c: [f32; 3], weights: &[f32; 6]) -> f32 {
    let (r, g, b) = (c[0], c[1], c[2]);
    let (max, min) = (r.max(g).max(b), r.min(g).min(b));
    let mid = r + g + b - max - min;
    // Primary: the largest channel; secondary: the two largest channels together.
    let primary = if r >= g && r >= b {
        0
    } else if g >= b {
        2
    } else {
        4
    };
    let secondary = if b <= r && b <= g {
        1 // red + green = yellow
    } else if r <= g {
        3 // green + blue = cyan
    } else {
        5 // red + blue = magenta
    };
    min + (mid - min) * weights[secondary] / 100.0 + (max - mid) * weights[primary] / 100.0
}

/// Ordered-dither offset in -0.5..0.5 (4×4 Bayer matrix) for document pixel (x, y).
pub fn bayer4(x: i32, y: i32) -> f32 {
    const M: [f32; 16] = [0.0, 8.0, 2.0, 10.0, 12.0, 4.0, 14.0, 6.0, 3.0, 11.0, 1.0, 9.0, 15.0, 7.0, 13.0, 5.0];
    (M[((y & 3) * 4 + (x & 3)) as usize] + 0.5) / 16.0 - 0.5
}

/// Samples a 3D LUT (`n`³ RGB triplets, red fastest) at `c`, trilinear or tetrahedral.
pub fn lut3d_sample(lut: &[f32], n: usize, c: [f32; 3], tetrahedral: bool) -> [f32; 3] {
    // A degenerate or short table can't be sampled: leave the colour alone.
    if n < 2 || n.checked_pow(3).and_then(|c| c.checked_mul(3)).is_none_or(|len| lut.len() < len) {
        return c;
    }
    let m = (n - 1) as f32;
    let pos = c.map(|v| v.clamp(0.0, 1.0) * m);
    let i0 = pos.map(|p| (p.floor() as usize).min(n - 2));
    let f: [f32; 3] = std::array::from_fn(|k| pos[k] - i0[k] as f32);
    let at = |dr: usize, dg: usize, db: usize| -> [f32; 3] {
        let idx = (((i0[2] + db) * n + i0[1] + dg) * n + i0[0] + dr) * 3;
        [lut[idx], lut[idx + 1], lut[idx + 2]]
    };
    let mix = |w: &[(f32, [f32; 3])]| -> [f32; 3] { std::array::from_fn(|k| w.iter().map(|(a, v)| a * v[k]).sum()) };
    let (fr, fg, fb) = (f[0], f[1], f[2]);
    if tetrahedral {
        let (c000, c111) = (at(0, 0, 0), at(1, 1, 1));
        if fr > fg {
            if fg > fb {
                mix(&[(1.0 - fr, c000), (fr - fg, at(1, 0, 0)), (fg - fb, at(1, 1, 0)), (fb, c111)])
            } else if fr > fb {
                mix(&[(1.0 - fr, c000), (fr - fb, at(1, 0, 0)), (fb - fg, at(1, 0, 1)), (fg, c111)])
            } else {
                mix(&[(1.0 - fb, c000), (fb - fr, at(0, 0, 1)), (fr - fg, at(1, 0, 1)), (fg, c111)])
            }
        } else if fb > fg {
            mix(&[(1.0 - fb, c000), (fb - fg, at(0, 0, 1)), (fg - fr, at(0, 1, 1)), (fr, c111)])
        } else if fb > fr {
            mix(&[(1.0 - fg, c000), (fg - fb, at(0, 1, 0)), (fb - fr, at(0, 1, 1)), (fr, c111)])
        } else {
            mix(&[(1.0 - fg, c000), (fg - fr, at(0, 1, 0)), (fr - fb, at(1, 1, 0)), (fb, c111)])
        }
    } else {
        let lerp = |a: [f32; 3], b: [f32; 3], t: f32| -> [f32; 3] { std::array::from_fn(|k| a[k] + (b[k] - a[k]) * t) };
        let c00 = lerp(at(0, 0, 0), at(1, 0, 0), fr);
        let c10 = lerp(at(0, 1, 0), at(1, 1, 0), fr);
        let c01 = lerp(at(0, 0, 1), at(1, 0, 1), fr);
        let c11 = lerp(at(0, 1, 1), at(1, 1, 1), fr);
        lerp(lerp(c00, c10, fg), lerp(c01, c11, fg), fb)
    }
}

/// Color Balance on channel `ch` (0 red, 1 green, 2 blue) of `v` (0..1); `tones` are the
/// shadows, midtones and highlights slider triples (cyan–red, magenta–green, yellow–blue,
/// −100..100).
///
/// Reverse-engineered from Photoshop (a 16-bit gray ramp through every tone at ±10..±100, pairs of
/// tones, and random settings of all nine sliders, Preserve Luminosity on and off): the three
/// tones fold into one Levels per channel (input black point, input white point, midtone gamma),
/// within 0.8 of 255 everywhere. Folding matters: a lowered shadows and a raised highlights
/// slider cancel each other's gamma, which applying the tones one after another doesn't (#3423).
/// - Preserve Luminosity off: a slider only moves its own channel. Lowered shadows set the black
///   point to |shadows| levels, raised highlights set the white point to 255 − highlights levels,
///   and the gamma is 2^(m / 100) with m = midtones + (shadows + highlights) / 2.
/// - Preserve Luminosity on: each channel's black point is (max − own) shadows levels, its white
///   point 255 − (own − min) highlights levels, and m = own − (max + min) / 2 midtones.
///
/// A gamma > 1 starts with slope 2^gamma at black (up to 18 of 255 brighter
/// near black without it). When the black point reaches the white point, input
/// at or above the white point is white and the rest black.
pub fn color_balance_channel(tones: [[f32; 3]; 3], preserve_luminosity: bool, ch: usize, v: f32) -> f32 {
    BalanceCurve::new(tones, preserve_luminosity, ch).eval(v)
}

/// One channel's Color Balance curve (see [`color_balance_channel`]), with the terms that don't
/// depend on the sample worked out once rather than per pixel.
pub struct BalanceCurve {
    /// Input black and white points.
    b: f32,
    w: f32,
    /// The power curve's exponent, 1 / gamma.
    e: f32,
    /// The Hermite join of a gamma > 1.
    knee: Option<Knee>,
}

/// Where a gamma > 1 curve's Hermite join ends (`u0`), its starting slope 2^gamma (`s0`), and the
/// power curve's value (`p`) and slope (`d`) at `u0`.
#[derive(Clone, Copy)]
struct Knee {
    u0: f32,
    s0: f32,
    p: f32,
    d: f32,
}

impl BalanceCurve {
    /// The red, green and blue curves.
    pub fn rgb(tones: [[f32; 3]; 3], preserve_luminosity: bool) -> [Self; 3] {
        std::array::from_fn(|ch| Self::new(tones, preserve_luminosity, ch))
    }

    /// The curve of channel `ch` (0 red, 1 green, 2 blue) for the shadows, midtones and
    /// highlights slider triples `tones`.
    pub fn new(tones: [[f32; 3]; 3], preserve_luminosity: bool, ch: usize) -> Self {
        let [shadows, midtones, highlights] = tones.map(|t| t.map(|s| if s.is_finite() { s.clamp(-100.0, 100.0) } else { 0.0 }));
        let own = |t: [f32; 3]| t.get(ch).copied().unwrap_or(0.0);
        let max = |t: [f32; 3]| t[0].max(t[1]).max(t[2]);
        let min = |t: [f32; 3]| t[0].min(t[1]).min(t[2]);
        let (black, white, m) = if preserve_luminosity {
            (max(shadows) - own(shadows), own(highlights) - min(highlights), own(midtones) - (max(midtones) + min(midtones)) / 2.0)
        } else {
            ((-own(shadows)).max(0.0), own(highlights).max(0.0), own(midtones) + (own(shadows) + own(highlights)) / 2.0)
        };
        // The midtone gamma of the channel's Levels: `u^(1/gamma)`, except that a gamma > 1
        // (which would start with infinite slope) starts at slope 2^gamma and joins the power
        // curve in a cubic Hermite at twice the input where that line meets it (Photoshop,
        // measured on a 16-bit ramp at gamma 1.07..2: within 0.02 of 255).
        let gamma = 2f32.powf(m / 100.0);
        let e = 1.0 / gamma.max(0.01);
        let knee = if gamma > 1.0 {
            let s0 = 2f32.powf(gamma);
            let u0 = (2.0 * s0.powf(1.0 / (e - 1.0))).min(1.0);
            (!u0.is_nan() && u0 > 0.0).then(|| Knee { u0, s0, p: u0.powf(e), d: e * u0.powf(e - 1.0) })
        } else {
            None
        };
        Self { b: black / 255.0, w: 1.0 - white / 255.0, e, knee }
    }

    /// The curve at `v` (0..1).
    pub fn eval(&self, v: f32) -> f32 {
        let (b, w) = (self.b, self.w);
        let v = if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 };
        if v >= w {
            return 1.0;
        }
        if v <= b {
            return 0.0;
        }
        let u = (v - b) / (w - b);
        match self.knee {
            Some(Knee { u0, s0, p, d }) if u < u0 => {
                let t = u / u0;
                let (t2, t3) = (t * t, t * t * t);
                ((t3 - 2.0 * t2 + t) * u0 * s0 + (3.0 * t2 - 2.0 * t3) * p + (t3 - t2) * u0 * d).clamp(0.0, 1.0)
            }
            _ => u.powf(self.e),
        }
    }
}

/// How much a colour belongs to each Selective Color range: reds, yellows, greens, cyans,
/// blues, magentas (by chroma within the hue sector), whites, neutrals and blacks (by lightness).
pub fn selective_color_weights(c: [f32; 3]) -> [f32; 9] {
    let max = c[0].max(c[1]).max(c[2]);
    let min = c[0].min(c[1]).min(c[2]);
    let mid = c[0] + c[1] + c[2] - max - min;
    let top = |i: usize| if c[i] >= max { max - mid } else { 0.0 };
    let bottom = |i: usize| if c[i] <= min { mid - min } else { 0.0 };
    [
        top(0),
        bottom(2),
        top(1),
        bottom(0),
        top(2),
        bottom(1),
        ((min - 0.5) * 2.0).max(0.0),
        (1.0 - (max - 0.5).abs() - (min - 0.5).abs()).clamp(0.0, 1.0),
        ((0.5 - max) * 2.0).max(0.0),
    ]
}

/// Photoshop Selective Color: each range shifts the cyan, magenta and yellow ink (1 − R, G, B) by
/// its percentage, plus its black percentage on every ink, weighted by how much the colour belongs
/// to the range. Relative mode scales the shift by the ink already present (so it can't tint pure
/// white). Absolute mode scales it by the room the ink has: adding ink by min(1, 2 × (1 − ink)),
/// removing it by min(1, 2 × ink), so Neutrals Cyan +50 darkens red 64 by 32, not 64 (all 72
/// absolute range × ink cases within 1 level of Photoshop 27.11).
pub fn selective_color(c: [f32; 3], relative: bool, adj: &[[f32; 4]; 9]) -> [f32; 3] {
    let w = selective_color_weights(c);
    let mut delta = [0.0f32; 3];
    for (r, wr) in w.iter().enumerate() {
        if *wr <= 0.0 {
            continue;
        }
        let a = adj[r].map(|v| v / 100.0);
        for i in 0..3 {
            let ink = 1.0 - c[i];
            let d = a[i] + a[3];
            let d = if relative {
                d * ink
            } else if d > 0.0 {
                d * (2.0 * (1.0 - ink)).min(1.0)
            } else {
                d * (2.0 * ink).min(1.0)
            };
            delta[i] += d * wr;
        }
    }
    std::array::from_fn(|i| (c[i] - delta[i]).clamp(0.0, 1.0))
}

/// Photo Filter's linear-light RGB matrix. Photoshop multiplies the pixel's D50 XYZ, relative to
/// white, by the filter colour's (mixed in by `density`): a fixed linear map, so it is folded into
/// one matrix. With `normalize_y` the map keeps luminance Y (32-bit Preserve Luminosity; 8/16-bit
/// documents instead restore the encoded luminosity like the Luminosity blend mode). Fitted on the
/// photoshop corpus `photo-filter.psd`: rgb16 within 0.6/255 everywhere.
pub fn photo_filter_matrix(color: [f32; 3], density: f32, normalize_y: bool) -> [[f32; 3]; 3] {
    let d = if density.is_finite() { density.clamp(0.0, 1.0) } else { 0.0 };
    let xyz = mat3_mul(&SRGB_TO_XYZ_D50, color.map(|v| srgb_to_linear(v.clamp(0.0, 1.0))));
    let mut s: [f32; 3] = std::array::from_fn(|i| 1.0 - d + d * xyz[i] / D50[i]);
    if normalize_y && s[1] > 1e-6 {
        let y = s[1];
        s = s.map(|v| v / y);
    }
    // XYZ_D50_TO_SRGB · diag(s) · SRGB_TO_XYZ_D50
    std::array::from_fn(|r| std::array::from_fn(|c| (0..3).map(|k| XYZ_D50_TO_SRGB[r][k] * s[k] * SRGB_TO_XYZ_D50[k][c]).sum()))
}

/// Photoshop's Vibrance on one pixel (`v`, `s`: the sliders / 100; `hi`: the sample ceiling).
///
/// Measured black-box against Photoshop 25.4 on 7,400-colour charts (8- and 16-bit; sRGB, Adobe RGB
/// and ProPhoto documents). Both sliders work in linear light through the document's tone curve,
/// Vibrance first, then Saturation:
/// - Saturation moves each colour away from (or towards) the grey `0.288 R + 0.712 G` (blue has no
///   weight, in every profile) by `1 + s`, clipping each channel: within 0.07/255 at every `s`.
/// - Vibrance is an HSV change in linear light that keeps the hue: see [`vibrance_lin`].
pub fn vibrance_px(c: [f32; 3], v: f32, s: f32, transfer: Transfer, hi: f32) -> [f32; 3] {
    let mut l = c.map(|x| transfer.decode(x));
    if v != 0.0 {
        l = vibrance_lin(l, v);
    }
    if s != 0.0 {
        let g = VIBRANCE_GREY[0] * l[0] + VIBRANCE_GREY[1] * l[1];
        l = l.map(|x| (g + (1.0 + s) * (x - g)).clamp(0.0, hi));
    }
    l.map(|x| transfer.encode(x.clamp(0.0, hi)))
}

/// Weights of the grey Photoshop's Vibrance › Saturation desaturates to (R, G; blue has none).
pub const VIBRANCE_GREY: [f32; 2] = [0.288, 0.712];

/// Positive Vibrance's saturation boost, fitted to the measurements (see [`vibrance_lin`]).
pub const VIBRANCE_BOOST: [f32; 7] = [0.5001, 0.0887, 0.4374, 0.7573, -0.8113, 0.5154, 2.8026];

/// The Vibrance slider (`v` in [-1, 1]) on a linear-light pixel. With `t = 1 - min/max` (HSV
/// saturation) and `V = max`:
/// - below 0 (`a = -v`): `t' = t (1 - a/4) (1 - a + a t (1 + t) / 2)` and
///   `V' = V - a t (1 - t) (2 - t + t²) V (1 - V)`; exact (0.2/255 mean) except in the deepest
///   shadows (`V` under 0.05), which Photoshop desaturates less.
/// - above 0: `V' = V + v/4 t (1 - t) (2 - t + t²) V (1 - V)` (exact) and a saturation boost that
///   grows as `t` and `V` fall, damped on reds and skin (HSV hue 0–30°, fading out by 45° and in
///   from 300°): a fit, 0.4 (+10) to 5 (+100) /255 off on average.
pub fn vibrance_lin(c: [f32; 3], v: f32) -> [f32; 3] {
    let mx = c[0].max(c[1]).max(c[2]);
    let mn = c[0].min(c[1]).min(c[2]);
    if mx <= 0.0 || mx - mn <= 1e-9 {
        return c;
    }
    let t = 1.0 - mn / mx;
    let shape = t * (1.0 - t) * (2.0 - t + t * t) * mx * (1.0 - mx).max(0.0);
    let (t2, v2) = if v < 0.0 {
        let a = -v;
        (t * (1.0 - a / 4.0) * (1.0 - a + a * t * (1.0 + t) / 2.0), mx - a * shape)
    } else {
        let [c0, c1, p, q, r0, ra, rt] = VIBRANCE_BOOST;
        let w = vibrance_skin(hsv_hue(c, mx, mn));
        let a = v * (1.0 - w) + 0.64 * v.powf(1.6) * w;
        let boost = (c0 * a + c1 * a * a) * t.powf(p) * (1.0 - t).powf(q) * ((r0 + ra * a + rt * (1.0 - t)) * (1.0 - mx).max(0.0)).exp();
        ((t * (1.0 + boost)).min(1.0), mx + v / 4.0 * shape)
    };
    let m2 = v2 * (1.0 - t2);
    let k = (v2 - m2) / (mx - mn);
    c.map(|x| m2 + (x - mn) * k)
}

/// How much positive Vibrance treats a hue (degrees) as skin: 1 on 0–30°, fading to 0 by 45° and
/// in again from 300°.
fn vibrance_skin(h: f32) -> f32 {
    if h <= 30.0 {
        1.0
    } else if h < 45.0 {
        (45.0 - h) / 15.0
    } else if h < 300.0 {
        0.0
    } else {
        (h - 300.0) / 60.0
    }
}

/// HSV hue in degrees of `c` (its max `mx` and min `mn` given, `mx > mn`).
fn hsv_hue(c: [f32; 3], mx: f32, mn: f32) -> f32 {
    let d = mx - mn;
    let h = if mx == c[0] {
        ((c[1] - c[2]) / d).rem_euclid(6.0)
    } else if mx == c[1] {
        (c[2] - c[0]) / d + 2.0
    } else {
        (c[0] - c[1]) / d + 4.0
    };
    h * 60.0
}

pub(super) fn gradient(stops: &[(f32, [f32; 3])], t: f32) -> [f32; 3] {
    match stops {
        [] => [t; 3],
        [s] => s.1,
        _ => {
            if t <= stops[0].0 {
                return stops[0].1;
            }
            for w in stops.windows(2) {
                if t <= w[1].0 {
                    let k = if w[1].0 > w[0].0 { (t - w[0].0) / (w[1].0 - w[0].0) } else { 0.0 };
                    return std::array::from_fn(|i| w[0].1[i] + (w[1].1[i] - w[0].1[i]) * k);
                }
            }
            stops[stops.len() - 1].1
        }
    }
}

pub fn rgb_to_hsl(c: [f32; 3]) -> (f32, f32, f32) {
    let max = c[0].max(c[1]).max(c[2]);
    let min = c[0].min(c[1]).min(c[2]);
    let l = (max + min) / 2.0;
    if (max - min).abs() < 1e-7 {
        return (0.0, 0.0, l);
    }
    let d = max - min;
    let s = if l > 0.5 { d / (2.0 - max - min) } else { d / (max + min) };
    let h = if max == c[0] {
        ((c[1] - c[2]) / d).rem_euclid(6.0)
    } else if max == c[1] {
        (c[2] - c[0]) / d + 2.0
    } else {
        (c[0] - c[1]) / d + 4.0
    };
    (h / 6.0, s, l)
}

pub fn hsl_to_rgb(h: f32, s: f32, l: f32) -> [f32; 3] {
    if s <= 0.0 {
        return [l; 3];
    }
    let q = if l < 0.5 { l * (1.0 + s) } else { l + s - l * s };
    hue_rgb(h, 2.0 * l - q, q)
}

/// The colour of hue `h` (0..1) with its smallest channel at `p` and its largest at `q`.
fn hue_rgb(h: f32, p: f32, q: f32) -> [f32; 3] {
    let f = |mut t: f32| {
        t = t.rem_euclid(1.0);
        if t < 1.0 / 6.0 {
            p + (q - p) * 6.0 * t
        } else if t < 0.5 {
            q
        } else if t < 2.0 / 3.0 {
            p + (q - p) * (2.0 / 3.0 - t) * 6.0
        } else {
            p
        }
    };
    [f(h + 1.0 / 3.0), f(h), f(h - 1.0 / 3.0)]
}
