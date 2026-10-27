use super::color::{bayer4, gradient};
use super::tone::{LUT_SIZE, lut, map_rgb, modern_brightness, modern_contrast, modern_lut, posterize_in, tone_luts_depth};
use super::*;

/// Legacy Brightness/Contrast on a 32-bit document's samples: unclipped, with contrast pivoting at
/// 127 of 255 (photoshop corpus rgb32 and gray32 brightness-contrast-legacy.psd: every pixel
/// exact, negative and over-white values included; the 8/16-bit pivot, 128, is 0.43 of a level
/// off there).
pub(super) fn legacy_brightness_contrast_hdr(brightness: f32, contrast: f32) -> impl Fn(f32) -> f32 {
    const PIVOT: f32 = 127.0 / 255.0;
    let b = brightness / 255.0;
    let c = contrast.clamp(-100.0, 99.0);
    let k = if c >= 0.0 { 1.0 / (1.0 - c / 100.0) } else { 1.0 + c / 100.0 };
    let (pre, post) = if c >= 0.0 { (b, 0.0) } else { (0.0, b) };
    move |v: f32| (v + pre - PIVOT) * k + PIVOT + post
}

/// Legacy Brightness/Contrast as one channel's tone curve (see [`apply_depth`]).
pub(super) fn legacy_brightness_contrast(brightness: f32, contrast: f32) -> impl Fn(f32) -> f32 {
    legacy_brightness_contrast_at(brightness, contrast, LEGACY_PIVOT)
}

/// [`legacy_brightness_contrast`] with contrast pivoting at `pivot`.
fn legacy_brightness_contrast_at(brightness: f32, contrast: f32, pivot: f32) -> impl Fn(f32) -> f32 {
    let b = brightness / 255.0;
    let c = contrast.clamp(-100.0, 99.0);
    let k = if c >= 0.0 { 1.0 / (1.0 - c / 100.0) } else { 1.0 + c / 100.0 };
    let (pre, post) = if c >= 0.0 { (b, 0.0) } else { (0.0, b) };
    move |v: f32| ((v + pre - pivot) * k + pivot + post).clamp(0.0, 1.0)
}

/// Whether Threshold at `level` turns colour `c` white in a document of `depth` (`None`: unknown,
/// treated as 8-bit). Photoshop splits on the luminosity 0.30 R + 0.59 G + 0.11 B (the blend
/// modes' `lum`), not the Rec. 601 grey, of the document's whole sample values:
/// - 8 bits: white when that luminosity, rounded to a level with halves down, reaches the level;
/// - 16 bits: white when it reaches the level converted to 16 bits (128 → 16448);
/// - 32 bits: white when it reaches the level.
///
/// Exact on every pixel of psd-tools threshold_rgb.psd, threshold_16bits_rgb.psd and
/// threshold_16bits_grayscale.psd and of the photoshop corpus's rgb16 threshold-128.psd, where the
/// Rec. 601 grey split 28 and 100 pixels of the first two the other way. The sums of
/// `30 R + 59 G + 11 B` over whole samples are exact in f32 (at most 3 276 800).
pub fn threshold_white(c: [f32; 3], level: f32, depth: Option<SampleType>) -> bool {
    let t = (level * 255.0).round();
    let sum = |q: f32| {
        let [r, g, b] = c.map(|v| (v.clamp(0.0, 1.0) * q).round());
        30.0 * r + 59.0 * g + 11.0 * b
    };
    match depth {
        None | Some(SampleType::U8) => sum(255.0) > 100.0 * t - 50.0,
        Some(SampleType::U16) => sum(32768.0) >= 100.0 * (t * 32768.0 / 255.0).round(),
        Some(SampleType::F32) => (0.3 * c[0] + 0.59 * c[1] + 0.11 * c[2]) * 255.0 >= t,
    }
}

/// Whether a Lab document evaluates `adj` on its L, a and b channels rather than on the screen
/// colour: Invert and Posterize (L, a and b alike), Brightness/Contrast (L only), Threshold
/// (split on L, neutral result) and Gradient Map (driven by L). Levels and Curves carry their own
/// Lab records ([`ToneSpace::Lab`]); everything else stays on the screen colour.
pub fn lab_channel_adjustment(adj: &Adjustment) -> bool {
    matches!(
        adj,
        Adjustment::Invert
            | Adjustment::Posterize { .. }
            | Adjustment::Threshold { .. }
            | Adjustment::BrightnessContrast { .. }
            | Adjustment::GradientMap { .. }
    )
}

/// Whether a CMYK document evaluates `adj` on its inks rather than on the screen colour: Invert
/// and Posterize (each ink, black included, so paper inverts to 400% ink), Threshold (paper or
/// 100% black ink alone) and Brightness/Contrast (each ink's stored value, 1 - ink, through the
/// tone curve; see [`ink_through`]). Photoshop corpus cmyk8 and cmyk16 invert, posterize-4,
/// threshold-128 and brightness-contrast(-legacy) follow these rules on the files' own inks on
/// every pixel; here the inks are the screen colour separated through the document's CMYK
/// profile, so neutrals carry the black that profile generates rather than the file's.
pub fn cmyk_channel_adjustment(adj: &Adjustment) -> bool {
    stored_channel_adjustment(adj) || cmy_adjustment(adj) || matches!(adj, Adjustment::BrightnessContrast { .. })
}

/// Whether a CMYK document evaluates `adj` on C, M and Y as the complement of an RGB colour,
/// black kept ([`apply_cmy`]): Hue/Saturation (not Colorize, which Photoshop separates again,
/// black and all) and Color Balance (photoshop corpus cmyk16 hue-saturation.psd follows this on
/// every pixel within half a level, cmyk8 within two; color-balance within our RGB Color
/// Balance's own error).
pub fn cmy_adjustment(adj: &Adjustment) -> bool {
    matches!(adj, Adjustment::HueSaturation { colorize: false, .. } | Adjustment::ColorBalance { .. })
}

/// A [`cmy_adjustment`] on straight inks (C, M, Y, K; 0 none): the RGB adjustment on
/// `(1 - c, 1 - m, 1 - y)`, black left alone.
pub fn apply_cmy(adj: &Adjustment, inks: &mut [[f32; 4]]) {
    let width = i32::try_from(inks.len()).unwrap_or(i32::MAX);
    let mut buf = Buffer { rect: crate::Rect::new(0, 0, width, 1), px: inks.iter().map(|i| [1.0 - i[0], 1.0 - i[1], 1.0 - i[2], 1.0]).collect() };
    apply_depth(adj, &mut buf, Transfer::Srgb, None);
    for (ink, p) in inks.iter_mut().zip(&buf.px) {
        for c in 0..3 {
            ink[c] = (1.0 - p[c]).clamp(0.0, 1.0);
        }
    }
}

/// Brightness/Contrast as the lookup table (4096 samples over 0..1) a CMYK ink's stored value,
/// `1 - ink`, goes through ([`ink_through`]): [`modern_lut`], or the legacy linear curve with its
/// contrast pivoting at 128 in ink, which is 127 of 255 stored (photoshop corpus cmyk16
/// brightness-contrast-legacy.psd sits 0.43 of a level off around 128 stored, as a one-level
/// pivot shift times contrast +30's slope predicts).
pub fn ink_brightness_contrast_lut(brightness: f32, contrast: f32, legacy: bool) -> Vec<f32> {
    if !legacy {
        return modern_lut(brightness, contrast);
    }
    let curve = legacy_brightness_contrast_at(brightness, contrast, 1.0 - LEGACY_PIVOT);
    (0..LUT_SIZE).map(|k| curve(k as f32 / (LUT_SIZE - 1) as f32)).collect()
}

/// One CMYK ink (0: none) through a channel tone curve the way Photoshop applies one in CMYK: to
/// the stored value, `1 - ink`, so raising brightness takes ink away.
pub fn ink_through(table: &[f32], ink: f32) -> f32 {
    1.0 - lut(table, 1.0 - ink)
}

/// Whether [`stored_channels`] evaluates `adj`: Invert, Posterize and Threshold.
pub fn stored_channel_adjustment(adj: &Adjustment) -> bool {
    matches!(adj, Adjustment::Invert | Adjustment::Posterize { .. } | Adjustment::Threshold { .. })
}

/// Invert, Posterize or Threshold on one pixel's own channels in a Lab or CMYK document of
/// `depth` (`None`: unknown, treated as 8-bit): `ch` holds L, a, b normalised as [`lab_norm`]
/// gives them, or the C, M, Y, K inks (0: none). Lab Invert turns 8-bit bytes `v` into `255 - v`
/// (a becomes -1 - a) and mirrors a and b around neutral at 16 bits (photoshop corpus lab16
/// invert.psd: neutral stays neutral). CMYK Threshold compares [`ink_luma`].
pub fn stored_channels(adj: &Adjustment, mode: ColorMode, depth: Option<SampleType>, ch: &mut [f32]) {
    let lab = mode == ColorMode::Lab;
    match adj {
        Adjustment::Invert => {
            let mirror = if lab && depth.is_some_and(|d| d != SampleType::U8) { 256.0 / 255.0 } else { 1.0 };
            for (i, v) in ch.iter_mut().enumerate() {
                *v = if i > 0 { mirror } else { 1.0 } - v.clamp(0.0, 1.0);
            }
        }
        // CMYK posterizes the stored values (1 - ink), which its uneven levels make differ from
        // posterizing the ink: psd-tools posterize_cmyk.psd (3 to 21 levels) is exact so.
        Adjustment::Posterize { levels } if mode == ColorMode::Cmyk => ch.iter_mut().for_each(|v| *v = 1.0 - posterize_in(1.0 - *v, *levels, depth)),
        Adjustment::Posterize { levels } => ch.iter_mut().for_each(|v| *v = posterize_in(*v, *levels, depth)),
        Adjustment::Threshold { level } => {
            // On the 8-bit grid, like the RGB threshold.
            let t = (level * 255.0).round();
            let out = if lab {
                let white = ch.first().is_some_and(|l| (l.clamp(0.0, 1.0) * 255.0).round() >= t);
                [if white { 1.0 } else { 0.0 }, 128.0 / 255.0, 128.0 / 255.0, 0.0]
            } else {
                let ink = |i: usize| ch.get(i).copied().unwrap_or(0.0);
                let black = (ink_luma([ink(0), ink(1), ink(2), ink(3)]) * 255.0).round() < t;
                [0.0, 0.0, 0.0, if black { 1.0 } else { 0.0 }]
            };
            ch.iter_mut().zip(out).for_each(|(v, o)| *v = o);
        }
        _ => {}
    }
}

/// The luma Photoshop's CMYK Threshold compares: Rec. 601 weights on the inks' plain complement,
/// `(1 - c)(1 - k)` and so on, whatever the document's profile (photoshop corpus cmyk8 and cmyk16
/// threshold-128.psd: every pixel on the right side of level 128).
pub fn ink_luma(ink: [f32; 4]) -> f32 {
    let paper = 1.0 - ink[3].clamp(0.0, 1.0);
    rgb_to_gray([0, 1, 2].map(|i| (1.0 - ink[i].clamp(0.0, 1.0)) * paper))
}

/// Encoded sRGB → Lab normalised the way Photoshop's channels hold it: L/100, (a+128)/255,
/// (b+128)/255.
fn lab_norm(c: [f32; 3]) -> [f32; 3] {
    let l = srgb_to_lab(c);
    [l[0] / 100.0, (l[1] + 128.0) / 255.0, (l[2] + 128.0) / 255.0]
}

/// The inverse of [`lab_norm`].
pub(super) fn lab_denorm(n: [f32; 3]) -> [f32; 3] {
    lab_to_srgb([n[0] * 100.0, n[1] * 255.0 - 128.0, n[2] * 255.0 - 128.0])
}

/// Neutral a or b as Lab surfaces store it ((0 + 128) / 255).
pub const LAB_NEUTRAL: f32 = 128.0 / 255.0;

/// An adjustment Photoshop evaluates on a Lab document's own channels, as an operation on one
/// pixel's stored L, a, b (normalised as Lab surfaces store them: L / 100, (a + 128) / 255,
/// (b + 128) / 255) in a document of `depth`: Invert and Posterize (each channel), Threshold
/// (split on L, neutral result), Brightness/Contrast (L) and Levels or Curves with Lab records.
/// `None` for any other adjustment. On the stored values colours outside the display gamut keep
/// theirs, where [`apply_in`] can only round-trip them through clipped display RGB.
pub fn lab_stored_op(adj: &Adjustment, depth: Option<SampleType>) -> Option<LabOp<'_>> {
    let op: LabOp<'_> = match adj {
        Adjustment::Invert | Adjustment::Posterize { .. } => Box::new(move |ch: &mut [f32; 3]| stored_channels(adj, ColorMode::Lab, depth, ch)),
        Adjustment::Threshold { level } => {
            // Split on L on the 8-bit grid, like the display path (`apply_lab`).
            let t = (level * 255.0).round();
            Box::new(move |ch: &mut [f32; 3]| {
                let white = (ch[0].clamp(0.0, 1.0) * 255.0).round() >= t;
                *ch = [if white { 1.0 } else { 0.0 }, LAB_NEUTRAL, LAB_NEUTRAL];
            })
        }
        Adjustment::BrightnessContrast { brightness, contrast, legacy } => {
            let (b, c, legacy) = (*brightness, *contrast, *legacy);
            let legacy_curve = legacy_brightness_contrast(b, c);
            Box::new(move |ch: &mut [f32; 3]| {
                ch[0] = if legacy { legacy_curve(ch[0]) } else { modern_contrast(modern_brightness(ch[0], b), c).clamp(0.0, 1.0) };
            })
        }
        Adjustment::Levels { space: ToneSpace::Lab, .. } | Adjustment::Curves { space: ToneSpace::Lab, .. } => {
            let luts = tone_luts_depth(adj, depth);
            Box::new(move |ch: &mut [f32; 3]| {
                for (v, t) in ch.iter_mut().zip(&luts) {
                    *v = lut(t, *v);
                }
            })
        }
        _ => return None,
    };
    Some(op)
}

/// One pixel's stored L, a, b through an adjustment ([`lab_stored_op`]).
pub type LabOp<'a> = Box<dyn Fn(&mut [f32; 3]) + Send + Sync + 'a>;

/// The Lab evaluation of a [`lab_channel_adjustment`] in a document of `depth` (no-op for any
/// other adjustment).
pub(super) fn apply_lab(adj: &Adjustment, buf: &mut Buffer, depth: Option<SampleType>) {
    match adj {
        // L → 100 - L and a, b mirrored about neutral; each channel posterized (see `stored_channels`).
        Adjustment::Invert | Adjustment::Posterize { .. } => map_rgb(buf, |c| {
            let mut n = lab_norm(c);
            stored_channels(adj, ColorMode::Lab, depth, &mut n);
            lab_denorm(n)
        }),
        Adjustment::Threshold { level } => {
            // Split on L alone (on the 8-bit grid); the result is neutral black or white.
            let t = (level * 255.0).round();
            map_rgb(buf, |c| [if (lab_norm(c)[0].clamp(0.0, 1.0) * 255.0).round() >= t { 1.0 } else { 0.0 }; 3])
        }
        Adjustment::BrightnessContrast { brightness, contrast, legacy } => {
            let legacy_curve = legacy_brightness_contrast(*brightness, *contrast);
            let modern = (!*legacy).then(|| modern_lut(*brightness, *contrast));
            let curve = |v: f32| match &modern {
                Some(table) => lut(table, v),
                None => legacy_curve(v),
            };
            // The tone curve on L only. Added as the difference of two round trips (like
            // `tone_in_space`) so a and b, and colours a round trip moves, stay put.
            map_rgb(buf, |c| {
                let n = lab_norm(c);
                let l = curve(n[0]);
                if l == n[0] {
                    return c;
                }
                let (before, after) = (lab_denorm(n), lab_denorm([l, n[1], n[2]]));
                std::array::from_fn(|i| (c[i] + after[i] - before[i]).clamp(0.0, 1.0))
            })
        }
        Adjustment::GradientMap { stops, reverse, dither } => {
            let (w, x0, y0) = (buf.rect.width().max(1) as usize, buf.rect.x0, buf.rect.y0);
            for (i, p) in buf.px.iter_mut().enumerate() {
                if p[3] <= 0.0 {
                    continue;
                }
                // Driven by L: equal lightness maps to the same stop colour whatever the chroma.
                let l = lab_norm([p[0], p[1], p[2]])[0].clamp(0.0, 1.0);
                let mut o = gradient(stops, if *reverse { 1.0 - l } else { l });
                if *dither {
                    let d = bayer4(x0 + (i % w) as i32, y0 + (i / w) as i32) / 255.0;
                    o = o.map(|v| (v + d).clamp(0.0, 1.0));
                }
                p[..3].copy_from_slice(&o);
            }
        }
        _ => {}
    }
}
