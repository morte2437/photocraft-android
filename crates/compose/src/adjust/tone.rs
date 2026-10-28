use super::{Adjustment, Buffer, ColorMode, CurvePoint, LevelsChannel, SampleType, ToneSpace, lab_to_srgb, srgb_to_lab};

/// Float RGB curve channels with no edit must preserve samples outside the LUT domain; edited
/// ones extend past it (see [`float_tone_extrapolates`]).
pub fn float_tone_identity(adj: &Adjustment, depth: Option<SampleType>) -> [bool; 3] {
    match adj {
        Adjustment::Curves { master, per_channel, space: ToneSpace::Rgb, .. } if depth == Some(SampleType::F32) => {
            std::array::from_fn(|i| photocraft_doc::adjust::is_identity_curve(master) && photocraft_doc::adjust::is_identity_curve(&per_channel[i]))
        }
        _ => [false; 3],
    }
}

/// Whether an edited Curves adjustment continues past 0..1 instead of clipping: in a 32-bit RGB
/// document Photoshop extends each curve beyond its end points along the slope of its end
/// segments (#7157: `[[0,0],[255,128]]` maps -0.5 to -0.250980407 and 2 to 1.003921628). Only
/// in RGB documents: the curve's space is not the document's, and other modes keep the table.
pub fn float_tone_extrapolates(adj: &Adjustment, depth: Option<SampleType>, mode: ColorMode) -> bool {
    matches!(adj, Adjustment::Curves { space: ToneSpace::Rgb, .. }) && depth == Some(SampleType::F32) && mode == ColorMode::Rgb
}

pub(crate) fn apply_tone(buf: &mut Buffer, space: ToneSpace, luts: &[Vec<f32>; 4], identity: [bool; 3], extrapolate: bool) {
    let sample = if extrapolate { lut_extended } else { lut };
    match space {
        ToneSpace::Rgb => map_rgb(buf, |c| std::array::from_fn(|i| if identity[i] { c[i] } else { sample(&luts[i], c[i]) })),
        ToneSpace::Cmyk | ToneSpace::Lab => map_rgb(buf, |c| tone_in_space(space, luts, c)),
    }
}

/// Per-channel LUTs of a Levels or Curves adjustment: each channel's record first, then the
/// master (composite) record, as Photoshop does (psd-tools levels_rgb.psd and curves_rgb.psd,
/// #975). Four rows: the three `per_channel` channels then black
/// (identity unless the space is CMYK). In Lab there is no composite record, so the master is ignored.
pub fn tone_luts(adj: &Adjustment) -> [Vec<f32>; 4] {
    tone_luts_q(adj, None)
}

/// [`tone_luts`] for samples with `quantum` steps per unit (Levels works on whole levels; see
/// [`levels_q`]).
pub fn tone_luts_q(adj: &Adjustment, quantum: Option<f32>) -> [Vec<f32>; 4] {
    let x = |k: usize| k as f32 / (LUT_SIZE - 1) as f32;
    match adj {
        Adjustment::Levels { master, per_channel, space, black } => {
            let ident = LevelsChannel::default();
            let m = if *space == ToneSpace::Lab { &ident } else { master };
            let row = |c: &LevelsChannel| (0..LUT_SIZE).map(|k| levels_q(m, levels_q(c, x(k), quantum), quantum)).collect();
            [row(&per_channel[0]), row(&per_channel[1]), row(&per_channel[2]), row(if *space == ToneSpace::Cmyk { black } else { &ident })]
        }
        Adjustment::Curves { master, per_channel, space, black, pencil } => {
            let m = if *space == ToneSpace::Lab { curve_lut(&[]) } else { curve_lut_mode(master, *pencil) };
            let row = |c: &[CurvePoint]| curve_lut_mode(c, *pencil).iter().map(|&v| lut(&m, v)).collect();
            [row(&per_channel[0]), row(&per_channel[1]), row(&per_channel[2]), row(if *space == ToneSpace::Cmyk { black } else { &[] })]
        }
        _ => std::array::from_fn(|_| (0..LUT_SIZE).map(x).collect()),
    }
}

/// [`tone_luts_q`] in a document of `depth` (`None`: unrounded). 32-bit Levels use
/// [`levels_float`], sampled on 0..1 and kept in 0..1 for LUT consumers. Float RGB rendering
/// evaluates the formula directly instead, so samples and results outside that domain survive.
pub fn tone_luts_depth(adj: &Adjustment, depth: Option<SampleType>) -> [Vec<f32>; 4] {
    match (adj, depth) {
        (Adjustment::Levels { master, per_channel, space: ToneSpace::Rgb, .. }, Some(SampleType::F32)) => {
            let x = |k: usize| k as f32 / (LUT_SIZE - 1) as f32;
            let row = |c: &LevelsChannel| (0..LUT_SIZE).map(|k| levels_float(master, levels_float(c, x(k))).clamp(0.0, 1.0)).collect();
            [row(&per_channel[0]), row(&per_channel[1]), row(&per_channel[2]), (0..LUT_SIZE).map(x).collect()]
        }
        _ => tone_luts_q(adj, depth.and_then(crate::adjustment_quantum)),
    }
}

/// Applies channel LUTs to a display-RGB colour in CMYK (ink brightness, 1 - ink) or Lab space,
/// through the conversions the document's surfaces use. The change is added as a difference of
/// two round trips, so channels a curve leaves alone (and out-of-gamut colours) stay exact.
fn tone_in_space(space: ToneSpace, luts: &[Vec<f32>; 4], c: [f32; 3]) -> [f32; 3] {
    use photocraft_color::convert::{cmyk_to_rgb, rgb_to_cmyk};
    let (before, after) = match space {
        ToneSpace::Cmyk => {
            let ink = rgb_to_cmyk(c);
            let out: [f32; 4] = std::array::from_fn(|i| 1.0 - lut(&luts[i], 1.0 - ink[i]));
            if out == ink {
                return c;
            }
            (cmyk_to_rgb(ink), cmyk_to_rgb(out))
        }
        ToneSpace::Lab => {
            let l = srgb_to_lab(c);
            let n = [l[0] / 100.0, (l[1] + 128.0) / 255.0, (l[2] + 128.0) / 255.0];
            let o: [f32; 3] = std::array::from_fn(|i| lut(&luts[i], n[i]));
            if o == n {
                return c;
            }
            let back = |v: [f32; 3]| lab_to_srgb([v[0] * 100.0, v[1] * 255.0 - 128.0, v[2] * 255.0 - 128.0]);
            (back(n), back(o))
        }
        ToneSpace::Rgb => return std::array::from_fn(|i| lut(&luts[i], c[i])),
    };
    std::array::from_fn(|i| (c[i] + after[i] - before[i]).clamp(0.0, 1.0))
}

pub(super) const LUT_SIZE: usize = 4096;

pub(super) fn map_rgb(buf: &mut Buffer, f: impl Fn([f32; 3]) -> [f32; 3]) {
    for p in &mut buf.px {
        if p[3] <= 0.0 {
            continue;
        }
        let r = f([p[0], p[1], p[2]]);
        p[0] = r[0];
        p[1] = r[1];
        p[2] = r[2];
    }
}

#[inline]
pub(super) fn lut(table: &[f32], v: f32) -> f32 {
    let x = v.clamp(0.0, 1.0) * (table.len() - 1) as f32;
    // Never negative (NaN casts to 0, as its floor did), so truncating is `floor`, without the
    // libm call baseline x86-64 (no SSE4.1) makes for it.
    let i = x as usize;
    let j = (i + 1).min(table.len() - 1);
    let f = x - i as f32;
    table[i] * (1.0 - f) + table[j] * f
}

/// Table steps the end slopes of [`lut_extended`] are measured over: one step's difference near
/// 0.5 carries f32 rounding (2e-4 of the slope), while 16 steps are still only 0.4 % of the domain.
pub const TONE_SLOPE_SPAN: usize = 16;

/// [`lut`] continued past 0..1 along the slope of the table's end segments, as Photoshop applies
/// a curve to float samples ([`float_tone_extrapolates`]). Inside the domain it is [`lut`];
/// non-finite samples stay with it too, so an infinite one can't become NaN on a flat end.
#[inline]
fn lut_extended(table: &[f32], v: f32) -> f32 {
    let n = table.len();
    if !v.is_finite() || (0.0..=1.0).contains(&v) || n < 2 {
        return lut(table, v);
    }
    let k = TONE_SLOPE_SPAN.min(n - 1);
    let per_unit = (n - 1) as f32 / k as f32;
    if v < 0.0 { table[0] + v * (table[k] - table[0]) * per_unit } else { table[n - 1] + (v - 1.0) * (table[n - 1] - table[n - 1 - k]) * per_unit }
}

/// Modern Brightness curve (one channel, `brightness` in [-150, 150]); pins pure black and white,
/// and `b = 0` is the identity. Raised, it is a line of slope `s = 1.375^(b/50)` from the origin
/// up to a knee at output 126/255, then a cubic Hermite to (1, 1) whose end slope is
/// `1 / (1 + 0.084 b)`; past about +110, where no such cubic stays below white, the roll-off is
/// `1 - (1 - y0)(1 - u)^α` instead (continuous slope `α` at the knee, flat at white). Lowered, it
/// is the inverse of the curve raised by as much.
///
/// Fit to every Photoshop measurement we have, all within about a level and a half: its ±50
/// samples (`modern_brightness_matches_photoshop`), the photoshop corpus's brightness-contrast.psd
/// (+25 with contrast +30) and psd-tools' brightnesscontrast_rgb.psd, whose masked layers give
/// +23, -31 and -95 (+150 within 6 levels there, where every curve family tried bottoms out).
pub fn modern_brightness(v: f32, brightness: f32) -> f32 {
    if brightness == 0.0 || !brightness.is_finite() {
        return v;
    }
    Brightness::new(brightness).eval(v)
}

/// [`modern_brightness`] for one slider value, its curve's constants worked out once.
struct Brightness {
    raise: bool,
    s: f32,
    e: f32,
    x0: f32,
    /// The knee's slope over the roll-off's secant (`α` of Fritsch–Carlson); a power roll-off
    /// replaces the Hermite when α² + β² > 9.
    alpha: f32,
    hermite: bool,
}

/// Output level of the Brightness curve's knee.
const BRIGHTNESS_KNEE: f32 = 126.0 / 255.0;

impl Brightness {
    fn new(brightness: f32) -> Self {
        let b = brightness.abs();
        let s = 1.375f32.powf(b / 50.0);
        let e = 1.0 / (1.0 + 0.084 * b);
        let x0 = (BRIGHTNESS_KNEE / s).min(1.0);
        let secant = (1.0 - BRIGHTNESS_KNEE) / (1.0 - x0).max(1e-6);
        let (alpha, beta) = (s / secant, e / secant);
        Brightness { raise: brightness > 0.0, s, e, x0, alpha, hermite: alpha * alpha + beta * beta <= 9.0 }
    }

    fn eval(&self, v: f32) -> f32 {
        let v = v.clamp(0.0, 1.0);
        if self.raise {
            return self.raised(v);
        }
        if v <= 0.0 || v >= 1.0 {
            return v;
        }
        // The inverse of the raised curve (monotone on [0, 1]), by bisection.
        let (mut lo, mut hi) = (0.0f32, 1.0f32);
        for _ in 0..32 {
            let mid = 0.5 * (lo + hi);
            if self.raised(mid) < v {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        0.5 * (lo + hi)
    }

    /// The curve raised: the line, then a monotone roll-off to (1, 1).
    fn raised(&self, x: f32) -> f32 {
        let Brightness { s, e, x0, alpha, hermite, .. } = *self;
        let y0 = BRIGHTNESS_KNEE;
        if x <= x0 {
            return y0.min(s * x);
        }
        let h = (1.0 - x0).max(1e-6);
        let t = ((x - x0) / h).clamp(0.0, 1.0);
        let y = if hermite {
            let (t2, t3) = (t * t, t * t * t);
            (2.0 * t3 - 3.0 * t2 + 1.0) * y0 + (t3 - 2.0 * t2 + t) * h * s + (-2.0 * t3 + 3.0 * t2) + (t3 - t2) * h * e
        } else {
            1.0 - (1.0 - y0) * (1.0 - t).powf(alpha)
        };
        y.clamp(0.0, 1.0)
    }
}

/// Modern Brightness/Contrast as one channel's lookup table (4096 samples over 0..1):
/// [`modern_brightness`] then [`modern_contrast`], as Photoshop applies them.
pub fn modern_lut(brightness: f32, contrast: f32) -> Vec<f32> {
    let step = |k: usize, n: usize| k as f32 / (n - 1) as f32;
    let bright: Vec<f32> = if brightness == 0.0 || !brightness.is_finite() {
        (0..LUT_SIZE).map(|k| step(k, LUT_SIZE)).collect()
    } else if brightness > 0.0 {
        let curve = Brightness::new(brightness);
        (0..LUT_SIZE).map(|k| curve.raised(step(k, LUT_SIZE))).collect()
    } else {
        // Lowered: invert the raised curve, sampled four times finer, in one monotone pass.
        let curve = Brightness::new(-brightness);
        let fine = 4 * LUT_SIZE;
        let ys: Vec<f32> = (0..fine).map(|j| curve.raised(step(j, fine))).collect();
        let mut j = 0;
        (0..LUT_SIZE)
            .map(|k| {
                let y = step(k, LUT_SIZE);
                while j + 2 < fine && ys.get(j + 1).is_some_and(|&n| n < y) {
                    j += 1;
                }
                let (y0, y1) = (ys.get(j).copied().unwrap_or(0.0), ys.get(j + 1).copied().unwrap_or(1.0));
                let f = if y1 > y0 { ((y - y0) / (y1 - y0)).clamp(0.0, 1.0) } else { 0.0 };
                (step(j, fine) + f / (fine - 1) as f32).clamp(0.0, 1.0)
            })
            .collect()
    };
    bright.into_iter().map(|b| modern_contrast(b, contrast).clamp(0.0, 1.0)).collect()
}

/// Modern Contrast curve (one channel, `contrast` in [-50, 100]). A symmetric cubic-Hermite S-curve
/// pivoting at 0.5: endpoint slope `1 - c/128`, centre slope `1 + c/128`. Near-exact vs Photoshop
/// (≤0.5/255). `c = 0` is the identity; pins 0, 0.5 and 1.
pub fn modern_contrast(v: f32, contrast: f32) -> f32 {
    if contrast == 0.0 {
        return v;
    }
    let k = contrast / 128.0;
    let (end, mid) = (1.0 - k, 1.0 + k);
    // Cubic Hermite on [0, 0.5]: pinned (0,0) slope `end`, (0.5,0.5) slope `mid`. `t` in [0,1].
    let half = |t: f32| end * 0.5 * (t * t * t - 2.0 * t * t + t) + (-2.0 * t * t * t + 3.0 * t * t) * 0.5 + mid * 0.5 * (t * t * t - t * t);
    let v = v.clamp(0.0, 1.0);
    if v <= 0.5 { half(v / 0.5) } else { 1.0 - half((1.0 - v) / 0.5) }
}

/// Photoshop posterize: `n` equal input bins over 0..=255, output levels
/// `floor(k * 255 / (n - 1))` (exact on the corpus for 3, 7, 13 and 21 levels).
pub fn posterize(v: f32, levels: u32) -> f32 {
    let n = levels.clamp(2, 255) as f32;
    let x = (v.clamp(0.0, 1.0) * 255.0).round();
    let bin = (x * n / 256.0).floor().min(n - 1.0);
    (bin * 255.0 / (n - 1.0)).floor() / 255.0
}

/// [`posterize`] in a document of `depth` (`None`: unknown, treated as 8-bit). At 16 bits
/// Photoshop bins the 16-bit sample (`⌊v · n / 32768⌋`) and lands on 8-bit levels through a
/// 16-bit step: `⌊b · 32768 / (n - 1)⌋`, converted with `⌊(o · 255 + 16384) / 32768⌋`, so
/// 127.5 comes out as 128 but 42.5 as 42 (8 bits floor them all). Exact on every sample of
/// psd-tools posterize_16bits_rgb.psd and posterize_16bits_grayscale.psd (levels 2 to 21),
/// which the 8-bit rule missed by up to half the range.
pub fn posterize_in(v: f32, levels: u32, depth: Option<SampleType>) -> f32 {
    if depth != Some(SampleType::U16) {
        return posterize(v, levels);
    }
    let n = f64::from(levels.clamp(2, 255));
    let x = (f64::from(v.clamp(0.0, 1.0)) * 32768.0).round();
    let bin = (x * n / 32768.0).floor().min(n - 1.0);
    let step = (bin * 32768.0 / (n - 1.0)).floor();
    (((step * 255.0 + 16384.0) / 32768.0).floor() / 255.0) as f32
}

pub fn levels(ch: &LevelsChannel, v: f32) -> f32 {
    levels_q(ch, v, None)
}

/// [`levels`] on samples with `quantum` steps per unit (8-bit: 255). Photoshop works on whole
/// levels in three steps, each rounded half up to a whole level: the input-range stretch, the
/// midtone gamma ([`levels_gamma`]) and the output range. So a 44..214 range maps 45, 46, 47, 48
/// → 1.5, 3, 4.5, 6 → 2, 3, 5, 6 (psd-tools levels_grayscale, where the gamma toe magnifies the
/// steps), and an output range narrower than 0..255 maps neighbouring gamma levels to one level
/// (psd-tools levels_rgb: 43..48 → 58 through a 0.47 channel gamma and the master's 48..255).
/// In a LUT the result is a staircase that is exact at every level.
pub fn levels_q(ch: &LevelsChannel, v: f32, quantum: Option<f32>) -> f32 {
    let range = (ch.in_white - ch.in_black).max(1e-6);
    let quantum = quantum.filter(|q| q.is_finite() && *q >= 1.0);
    let whole = |x: f32| match quantum {
        Some(q) => (x * q + 0.5).floor() / q,
        None => x,
    };
    let u = match quantum {
        Some(q) => {
            let v = (v * q).round() / q;
            (((v - ch.in_black) / range).clamp(0.0, 1.0) * q + 0.5 + 1e-3).floor() / q
        }
        None => ((v - ch.in_black) / range).clamp(0.0, 1.0),
    };
    let t = whole(levels_gamma(u, ch.gamma.max(0.01)));
    whole(ch.out_black + t * (ch.out_white - ch.out_black))
}

/// Exponent of the shadow toe in [`levels_gamma`] (fitted; see there).
const LEVELS_TOE_POWER: f32 = 2.2;

/// Levels' midtone gamma `g` on `u` in 0..1: `u^(1/g)`, except that above 1, where that power
/// curve is infinitely steep at black, Photoshop starts along a toe `a·u − b·u^q` of slope
/// `a = 2^g` that joins the curve with the same value and slope. `q` = [`LEVELS_TOE_POWER`] is
/// fitted on whole-level oracles at gamma 1.16, 1.22, 1.3, 1.78 and 2.0 (psd-tools levels_rgb
/// and levels_grayscale, the photoshop corpus's rgb8 and gray8 levels.psd, a measured 0..255
/// ramp): all but 2 of their 1118 levels exact, the others within one. Below 1 the power curve is
/// exact (gamma 0.67 and 0.7: every level).
pub fn levels_gamma(u: f32, g: f32) -> f32 {
    let c = 1.0 / g;
    if g <= 1.0 {
        return u.powf(c);
    }
    let (a, q) = (2f32.powf(g), LEVELS_TOE_POWER);
    // Where the toe meets the curve: a·u1 − b·u1^q = u1^c and a − q·b·u1^(q−1) = c·u1^(c−1).
    let u1 = ((1.0 - c / q) / (a * (1.0 - 1.0 / q))).powf(1.0 / (1.0 - c));
    if u >= u1 || u1 <= 0.0 {
        return u.powf(c);
    }
    let b = (a * u1 - u1.powf(c)) / u1.powf(q);
    // Just above gamma 1 the toe shrinks to nothing: u1^q underflows to 0 while u1 doesn't, and
    // the power curve is the toe's limit there (otherwise b = inf and black turns NaN).
    if !b.is_finite() {
        return u.powf(c);
    }
    a * u - b * u.powf(q)
}

/// [`levels`] in a 32-bit document. Neither the input is clipped to the input range nor the result
/// to the output range there, and the midtone gamma is a plain power curve, mirrored below the
/// black point (the oracle corpus' rgb32 and gray32 levels.psd: within 0.5/255, 10/255 off with
/// the clipped curve).
pub fn levels_float(ch: &LevelsChannel, v: f32) -> f32 {
    let t = (v - ch.in_black) / (ch.in_white - ch.in_black).max(1e-6);
    let t = if ch.gamma == 1.0 { t } else { t.signum() * t.abs().powf(1.0 / ch.gamma.max(0.01)) };
    ch.out_black + t * (ch.out_white - ch.out_black)
}

/// The LUT of one Curves curve: [`curve_map_lut`] for a pencil-drawn map, else [`curve_lut`].
pub fn curve_lut_mode(points: &[CurvePoint], pencil: bool) -> Vec<f32> {
    if pencil { curve_map_lut(points) } else { curve_lut(points) }
}

/// A pencil-drawn Curves map (see [`photocraft_doc::adjust::curve_map`]): straight segments
/// between its entries, constant outside the first/last, clamped to 0..=1, sampled into a LUT.
pub fn curve_map_lut(points: &[CurvePoint]) -> Vec<f32> {
    let mut pts: Vec<(f32, f32)> = points.iter().filter(|p| p.input.is_finite() && p.output.is_finite()).map(|p| (p.input, p.output)).collect();
    pts.sort_by(|a, b| a.0.total_cmp(&b.0));
    pts.dedup_by(|a, b| (a.0 - b.0).abs() < 1e-6);
    let ident = |k: usize| k as f32 / (LUT_SIZE - 1) as f32;
    let (Some(&first), Some(&last)) = (pts.first(), pts.last()) else { return (0..LUT_SIZE).map(ident).collect() };
    let mut seg = 0;
    (0..LUT_SIZE)
        .map(|k| {
            let xv = ident(k);
            if xv <= first.0 {
                return first.1.clamp(0.0, 1.0);
            }
            if xv >= last.0 {
                return last.1.clamp(0.0, 1.0);
            }
            while pts.get(seg + 1).is_some_and(|p| p.0 < xv) {
                seg += 1;
            }
            let (Some(&(x0, y0)), Some(&(x1, y1))) = (pts.get(seg), pts.get(seg + 1)) else { return last.1.clamp(0.0, 1.0) };
            let t = if x1 > x0 { (xv - x0) / (x1 - x0) } else { 0.0 };
            (y0 + (y1 - y0) * t).clamp(0.0, 1.0)
        })
        .collect()
}

/// Photoshop curve: a natural cubic spline through the points (it may
/// overshoot between points, as Photoshop's does), constant outside the
/// first/last point, clamped to 0..=1, sampled into a LUT.
pub fn curve_lut(points: &[CurvePoint]) -> Vec<f32> {
    let mut pts: Vec<(f32, f32)> = points.iter().map(|p| (p.input, p.output)).collect();
    pts.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    pts.dedup_by(|a, b| (a.0 - b.0).abs() < 1e-6);
    if pts.len() < 2 {
        return (0..LUT_SIZE).map(|i| i as f32 / (LUT_SIZE - 1) as f32).collect();
    }
    let n = pts.len();
    // Second derivatives of the natural spline (tridiagonal solve).
    let mut m2 = vec![0.0f64; n];
    if n > 2 {
        let x: Vec<f64> = pts.iter().map(|p| f64::from(p.0)).collect();
        let y: Vec<f64> = pts.iter().map(|p| f64::from(p.1)).collect();
        let mut c = vec![0.0f64; n];
        let mut d = vec![0.0f64; n];
        for i in 1..n - 1 {
            let (h0, h1) = (x[i] - x[i - 1], x[i + 1] - x[i]);
            let a = h0 / 6.0;
            let b = (h0 + h1) / 3.0;
            let cc = h1 / 6.0;
            let r = (y[i + 1] - y[i]) / h1 - (y[i] - y[i - 1]) / h0;
            let denom = b - a * c[i - 1];
            c[i] = cc / denom;
            d[i] = (r - a * d[i - 1]) / denom;
        }
        for i in (1..n - 1).rev() {
            m2[i] = d[i] - c[i] * m2[i + 1];
        }
    }
    (0..LUT_SIZE)
        .map(|k| {
            let xv = k as f32 / (LUT_SIZE - 1) as f32;
            if xv <= pts[0].0 {
                return pts[0].1.clamp(0.0, 1.0);
            }
            if xv >= pts[n - 1].0 {
                return pts[n - 1].1.clamp(0.0, 1.0);
            }
            let i = pts.windows(2).position(|w| xv <= w[1].0).unwrap_or(n - 2);
            let (x0, y0) = (f64::from(pts[i].0), f64::from(pts[i].1));
            let (x1, y1) = (f64::from(pts[i + 1].0), f64::from(pts[i + 1].1));
            let h = x1 - x0;
            let t = f64::from(xv);
            let a = (x1 - t) / h;
            let b = (t - x0) / h;
            let y = a * y0 + b * y1 + ((a * a * a - a) * m2[i] + (b * b * b - b) * m2[i + 1]) * h * h / 6.0;
            (y as f32).clamp(0.0, 1.0)
        })
        .collect()
}

/// A layer-style contour curve: [`curve_lut`], except that the curve bends sharply at corner
/// points (`corners[i]` true; Photoshop's Contour Editor "Corner"): each run between corners is
/// its own natural spline, so all-corner points make a polyline. Missing flags are smooth.
pub fn contour_curve_lut(points: &[CurvePoint], corners: &[bool]) -> Vec<f32> {
    let mut pts: Vec<(CurvePoint, bool)> = points.iter().enumerate().map(|(i, p)| (p.clone(), corners.get(i).copied().unwrap_or(false))).collect();
    if !pts.iter().any(|p| p.1) {
        return curve_lut(points);
    }
    pts.sort_by(|a, b| a.0.input.partial_cmp(&b.0.input).unwrap_or(std::cmp::Ordering::Equal));
    pts.dedup_by(|a, b| (a.0.input - b.0.input).abs() < 1e-6);
    let mut out: Vec<f32> = Vec::new();
    let mut start = 0usize;
    let last = pts.len().saturating_sub(1);
    for i in 1..=last {
        let corner = pts.get(i).is_some_and(|p| p.1);
        if !(corner || i == last) {
            continue;
        }
        let Some(seg) = pts.get(start..=i) else { break };
        let from = seg.first().map_or(0.0, |p| p.0.input);
        let lut = curve_lut(&seg.iter().map(|p| p.0.clone()).collect::<Vec<_>>());
        if out.is_empty() {
            out = lut;
        } else {
            for (k, (o, v)) in out.iter_mut().zip(lut).enumerate() {
                if k as f32 / (LUT_SIZE - 1) as f32 >= from {
                    *o = v;
                }
            }
        }
        start = i;
    }
    if out.is_empty() { curve_lut(points) } else { out }
}
