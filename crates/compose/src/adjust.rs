//! Evaluation of adjustment layers on a composite buffer.
//!
//! Formulas are documented approximations of Photoshop behaviour. Exact matching is tuned in
//! milestone M7 against Photoshop-rendered PSD composites (the oracle in `testkit`).

use photocraft_color::convert::{D50, SRGB_TO_XYZ_D50, XYZ_D50_TO_SRGB, lab_to_srgb, mat3_mul, rgb_to_gray, srgb_to_lab, srgb_to_linear};
use photocraft_color::{ColorMode, SampleType};
use photocraft_doc::Adjustment;
use photocraft_doc::adjust::{CurvePoint, HueRange, LevelsChannel, ToneSpace, lut3d_len};

use crate::Buffer;

mod channels;
mod color;
mod tone;

#[cfg(test)]
use channels::lab_denorm;
pub use channels::{
    LAB_NEUTRAL, LabOp, apply_cmy, cmy_adjustment, cmyk_channel_adjustment, ink_brightness_contrast_lut, ink_luma, ink_through, lab_channel_adjustment,
    lab_stored_op, stored_channel_adjustment, stored_channels, threshold_white,
};
use channels::{apply_lab, legacy_brightness_contrast, legacy_brightness_contrast_hdr};
use color::gradient;
pub use color::{
    BalanceCurve, VIBRANCE_BOOST, VIBRANCE_GREY, bayer4, black_white_gray, color_balance_channel, colorize_saturation, hdr_saturation_factor, hsl_to_rgb,
    hue_range_tables, hue_saturation, hue_saturation_in, lut3d_sample, photo_filter_matrix, range_lightness, rgb_to_hsl, selective_color,
    selective_color_weights, vibrance_lin, vibrance_px,
};
#[cfg(test)]
use tone::LUT_SIZE;
pub(crate) use tone::apply_tone;
pub use tone::{
    TONE_SLOPE_SPAN, contour_curve_lut, curve_lut, curve_lut_mode, curve_map_lut, curve_samples, float_tone_extrapolates, float_tone_identity, levels,
    levels_float, levels_gamma, levels_q, modern_brightness, modern_contrast, modern_lut, posterize, posterize_in, tone_luts, tone_luts_depth, tone_luts_q,
};
use tone::{lut, map_rgb};

/// The document tone curve used to linearize values for adjustments that
/// work in linear light (Exposure). Until ICC profiles are wired in (M8) this
/// approximates the Photoshop defaults: sRGB for colour documents and the
/// "Dot Gain 20%" grey profile (≈ gamma 1.73, fitted on the corpus) for
/// grayscale documents. 32-bit documents store linear light already.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Transfer {
    /// sRGB piecewise curve.
    Srgb,
    /// Pure power law with this exponent.
    Gamma(f32),
}

impl Transfer {
    /// Default transfer for a document's colour mode and depth. 32-bit samples are linear, and
    /// Photoshop's Exposure works on them as stored (photoshop corpus rgb32 and gray32
    /// exposure.psd: within 0.5/255, against 69/255 through the 2.2 curve).
    pub fn for_document(mode: photocraft_color::ColorMode, depth: photocraft_color::SampleType) -> Self {
        if depth == photocraft_color::SampleType::F32 {
            return Transfer::Gamma(1.0);
        }
        match mode {
            photocraft_color::ColorMode::Grayscale | photocraft_color::ColorMode::Duotone | photocraft_color::ColorMode::Bitmap => Transfer::Gamma(1.732),
            _ => Transfer::Srgb,
        }
    }
    /// The tone curve Exposure linearises through: RGB documents use a pure 2.2 power, not the
    /// sRGB curve (psd-tools adjustment_nested_composition_4: an offset of 0.1738 lifts 76 to 134,
    /// not 136; ag-psd masks), grey documents their dot-gain gamma.
    pub fn for_exposure(self) -> Self {
        match self {
            Transfer::Srgb => Transfer::Gamma(2.2),
            t => t,
        }
    }
    /// Tone-encoded value → linear light.
    pub fn decode(self, v: f32) -> f32 {
        match self {
            Transfer::Srgb => photocraft_color::convert::srgb_to_linear(v.max(0.0)),
            Transfer::Gamma(1.0) => v.max(0.0),
            Transfer::Gamma(g) => v.max(0.0).powf(g),
        }
    }
    /// Linear light → tone-encoded value.
    pub fn encode(self, v: f32) -> f32 {
        match self {
            Transfer::Srgb => photocraft_color::convert::linear_to_srgb(v.max(0.0)),
            Transfer::Gamma(1.0) => v.max(0.0),
            Transfer::Gamma(g) => v.max(0.0).powf(1.0 / g),
        }
    }
}

/// Applies an adjustment assuming an sRGB document.
pub fn apply(adj: &Adjustment, buf: &mut Buffer) {
    apply_with(adj, buf, Transfer::Srgb);
}

/// Applies an adjustment with the document's tone transfer.
pub fn apply_with(adj: &Adjustment, buf: &mut Buffer, transfer: Transfer) {
    apply_depth(adj, buf, transfer, None);
}

/// Applies an adjustment with the document's tone transfer, in a document of `depth` (`None`:
/// unrounded). Integer depths work on whole levels like Photoshop's (see [`levels_q`] and
/// `adjustment_quantum`), 32-bit documents get its float Levels (see [`levels_float`]).
pub fn apply_depth(adj: &Adjustment, buf: &mut Buffer, transfer: Transfer, depth: Option<SampleType>) {
    apply_depth_mode(adj, buf, transfer, depth, ColorMode::Rgb);
}

/// [`apply_depth`] in a document of colour `mode` (which decides whether float curves extend,
/// see [`float_tone_extrapolates`]).
fn apply_depth_mode(adj: &Adjustment, buf: &mut Buffer, transfer: Transfer, depth: Option<SampleType>, mode: ColorMode) {
    match adj {
        Adjustment::Invert => map_rgb(buf, |c| [1.0 - c[0], 1.0 - c[1], 1.0 - c[2]]),
        Adjustment::Threshold { level } => map_rgb(buf, |c| [if threshold_white(c, *level, depth) { 1.0 } else { 0.0 }; 3]),
        Adjustment::Posterize { levels } => map_rgb(buf, |c| c.map(|v| posterize_in(v, *levels, depth))),
        Adjustment::BrightnessContrast { brightness, contrast, legacy: true } => {
            // Legacy: contrast scales around 128/255, with brightness added before it when
            // contrast is raised and after it when contrast is lowered. Measured on Photoshop
            // 27.11, brightness +40 with contrast +30 maps 128 to 185 (brightness first), while
            // the corpus's brightness -23 with contrast -26 (psd-tools) needs contrast first;
            // contrast 80 keeps 128 at 128 (a 0.5 pivot moved it to 130).
            if depth == Some(SampleType::F32) {
                let f = legacy_brightness_contrast_hdr(*brightness, *contrast);
                map_rgb(buf, |px| px.map(&f));
            } else {
                let f = legacy_brightness_contrast(*brightness, *contrast);
                map_rgb(buf, |px| px.map(&f))
            }
        }
        Adjustment::BrightnessContrast { brightness, contrast, .. } => {
            // Modern (CS3+) Brightness/Contrast, reverse-engineered from Photoshop ground truth.
            // Unlike the legacy linear scale, both are smooth curves that pin pure black and white
            // (see `modern_brightness` and `modern_contrast`), applied brightness then contrast
            // through one lookup table.
            let table = modern_lut(*brightness, *contrast);
            map_rgb(buf, |px| px.map(|v| lut(&table, v)))
        }
        Adjustment::Exposure { exposure, offset, gamma } => {
            // In linear light: (lin·2^exposure + offset)^(1/gamma), then back
            // through the document tone curve (fitted on the corpus).
            let m = 2f32.powf(*exposure);
            let g = gamma.max(0.01);
            let transfer = transfer.for_exposure();
            let hdr = depth == Some(SampleType::F32);
            map_rgb(buf, |c| {
                c.map(|v| {
                    if hdr {
                        // Float documents store linear samples. Photoshop's 32-bit merged
                        // Exposure oracle also retains negative results with signed gamma.
                        let lin = v * m + offset;
                        return if g == 1.0 { lin } else { lin.signum() * lin.abs().powf(1.0 / g) };
                    }
                    let lin = (transfer.decode(v) * m + offset).max(0.0);
                    let lin = if g == 1.0 { lin } else { lin.powf(1.0 / g) };
                    transfer.encode(lin).clamp(0.0, 1.0)
                })
            })
        }
        Adjustment::Levels { master, per_channel, space: ToneSpace::Rgb, .. } if depth == Some(SampleType::F32) => {
            // A 0..1 lookup table cannot represent signed, unbounded float samples.
            let master_identity = *master == LevelsChannel::default();
            let channel_identity = per_channel.each_ref().map(|ch| *ch == LevelsChannel::default());
            if !master_identity || channel_identity.iter().any(|&v| !v) {
                map_rgb(buf, |c| {
                    std::array::from_fn(|i| {
                        let v = if channel_identity[i] { c[i] } else { levels_float(&per_channel[i], c[i]) };
                        if master_identity { v } else { levels_float(master, v) }
                    })
                });
            }
        }
        Adjustment::Levels { space, .. } | Adjustment::Curves { space, .. } => {
            let identity = float_tone_identity(adj, depth);
            if identity == [true; 3] {
                return;
            }
            let luts = tone_luts_depth(adj, depth);
            apply_tone(buf, *space, &luts, identity, float_tone_extrapolates(adj, depth, mode));
        }
        Adjustment::HueSaturation { hue, saturation, lightness, colorize, ranges } => {
            let table = (!*colorize && ranges.iter().any(|r| !r.is_neutral())).then(|| hue_range_tables(ranges));
            map_rgb(buf, |c| {
                let (dh, ds, dl) = match &table {
                    Some(t) => {
                        // Range edits follow the pixel's original hue, in full however dull the colour
                        // (Photoshop's merged images, #4089). Greys stay grey: no hue to shift, no
                        // saturation to scale, and range Lightness moves towards a channel they share.
                        let h0 = rgb_to_hsl(c).0;
                        (lut(&t[0], h0), lut(&t[1], h0), lut(&t[2], h0))
                    }
                    None => (0.0, 0.0, 0.0),
                };
                let sat = colorize_saturation((*saturation / 100.0 + ds).clamp(-1.0, 1.0), *lightness / 100.0, *colorize, depth);
                let out = hue_saturation_in(c, *hue + dh, sat, (*lightness / 100.0).clamp(-1.0, 1.0), *colorize, depth);
                range_lightness(out, dl)
            })
        }
        Adjustment::Vibrance { vibrance, saturation } => {
            let (v, s) = (*vibrance / 100.0, *saturation / 100.0);
            let hi = if depth == Some(SampleType::F32) { f32::INFINITY } else { 1.0 };
            map_rgb(buf, |c| vibrance_px(c, v, s, transfer, hi))
        }
        Adjustment::ChannelMixer { matrix, monochrome } if depth == Some(SampleType::F32) => {
            // 32-bit: each source percentage is cut to 1/1024 (toward zero), the constant counts
            // 255/512 of its value, and nothing clips (photoshop corpus rgb32 channel-mixer.psd:
            // pure red, green, blue, black and white all exact).
            let cut = |v: f32| (v * 1024.0).trunc() / 1024.0;
            let m = matrix.map(|row| [cut(row[0]), cut(row[1]), cut(row[2]), row[3] * 255.0 / 512.0]);
            map_rgb(buf, |c| {
                let mix = |row: &[f32; 4]| row[0] * c[0] + row[1] * c[1] + row[2] * c[2] + row[3];
                if *monochrome { [mix(&m[0]); 3] } else { [mix(&m[0]), mix(&m[1]), mix(&m[2])] }
            })
        }
        Adjustment::ChannelMixer { matrix, monochrome } => map_rgb(buf, |c| {
            let mix = |row: &[f32; 4]| (row[0] * c[0] + row[1] * c[1] + row[2] * c[2] + row[3]).clamp(0.0, 1.0);
            if *monochrome { [mix(&matrix[0]); 3] } else { [mix(&matrix[0]), mix(&matrix[1]), mix(&matrix[2])] }
        }),
        Adjustment::PhotoFilter { color, density, preserve_luminosity } => {
            let linear_doc = depth == Some(SampleType::F32);
            let m = photo_filter_matrix(*color, *density, *preserve_luminosity && linear_doc);
            let set_lum = *preserve_luminosity && !linear_doc;
            map_rgb(buf, |c| {
                // 32-bit samples are linear already, and may go negative.
                let f = if linear_doc { mat3_mul(&m, c) } else { mat3_mul(&m, c.map(|v| transfer.decode(v))).map(|v| transfer.encode(v)) };
                let f = if set_lum { photocraft_color::blend::set_lum(f, photocraft_color::blend::lum(c)) } else { f };
                // 32-bit results aren't clipped (photoshop corpus rgb32 photo-filter.psd: white
                // filters to 1.19 red).
                if linear_doc { f } else { f.map(|v| v.clamp(0.0, 1.0)) }
            })
        }
        Adjustment::BlackWhite { weights, tint } => {
            let hdr = depth == Some(SampleType::F32);
            // 32-bit greys keep their superwhites: Photoshop 27.11 maps negatives to 0 and caps
            // neutrals at 16, but leaves 2.0 at 2.0 (#8537).
            let hi = if hdr { 16.0 } else { 1.0 };
            map_rgb(buf, |c| {
                let g = black_white_gray(c, weights).clamp(0.0, hi);
                match tint {
                    // Photoshop tints like the Color blend mode: the tint's hue and saturation at
                    // the grey's luminosity (SetLum + ClipColor), measured in Photoshop 27.11. In
                    // 32-bit documents nothing clips at 1, as with the blend mode there (photoshop
                    // corpus rgb32 black-white-tint.psd: white tints to 1.05, 1.00, 0.87).
                    Some(t) if hdr => photocraft_color::blend::blend_rgb_within(photocraft_color::blend::BlendMode::Color, [g; 3], *t, f32::MAX),
                    Some(t) => photocraft_color::blend::set_lum(*t, g).map(|v| v.clamp(0.0, 1.0)),
                    None => [g; 3],
                }
            })
        }
        Adjustment::GradientMap { stops, reverse, dither } => {
            let (w, x0, y0) = (buf.rect.width().max(1) as usize, buf.rect.x0, buf.rect.y0);
            for (i, p) in buf.px.iter_mut().enumerate() {
                if p[3] <= 0.0 {
                    continue;
                }
                // Driven by the W3C luminosity (0.3, 0.59, 0.11), not the Rec. 601 grey: photoshop
                // corpus rgb8 and rgb16 gradient-map.psd match within a level (2 levels off with
                // 0.299, 0.587, 0.114).
                let mut t = photocraft_color::blend::lum([p[0], p[1], p[2]]);
                if *reverse {
                    t = 1.0 - t;
                }
                let mut o = gradient(stops, t);
                if *dither {
                    let d = bayer4(x0 + (i % w) as i32, y0 + (i / w) as i32) / 255.0;
                    o = o.map(|v| (v + d).clamp(0.0, 1.0));
                }
                p[..3].copy_from_slice(&o);
            }
        }
        Adjustment::ColorBalance { shadows, midtones, highlights, preserve_luminosity } => {
            let curves = BalanceCurve::rgb([*shadows, *midtones, *highlights], *preserve_luminosity);
            map_rgb(buf, |c| std::array::from_fn(|ch| curves[ch].eval(c[ch])))
        }
        Adjustment::SelectiveColor { relative, adjustments } => map_rgb(buf, |c| selective_color(c, *relative, adjustments)),
        Adjustment::ColorLookup { lut: Some(table), size, tetrahedral, dither, .. } if lut3d_len(*size).is_some_and(|len| table.len() >= len) => {
            let (n, w, x0, y0) = (*size as usize, buf.rect.width().max(1) as usize, buf.rect.x0, buf.rect.y0);
            for (i, p) in buf.px.iter_mut().enumerate() {
                if p[3] <= 0.0 {
                    continue;
                }
                let mut o = lut3d_sample(table, n, [p[0], p[1], p[2]], *tetrahedral);
                if *dither {
                    let d = bayer4(x0 + (i % w) as i32, y0 + (i / w) as i32) / 255.0;
                    o = o.map(|v| (v + d).clamp(0.0, 1.0));
                }
                p[..3].copy_from_slice(&o);
            }
        }
        // Not evaluated: identity (still round-trips through PSD).
        Adjustment::ColorLookup { .. } | Adjustment::Unsupported { .. } => {}
    }
}

/// Applies an adjustment in a document of colour `mode`: [`apply_depth`], except that in Lab
/// and CMYK documents the adjustments Photoshop evaluates on the document's own channels do so
/// (see [`lab_channel_adjustment`] and [`cmyk_channel_adjustment`]).
pub fn apply_in(adj: &Adjustment, buf: &mut Buffer, transfer: Transfer, depth: Option<SampleType>, mode: ColorMode) {
    if mode == ColorMode::Lab && lab_channel_adjustment(adj) {
        apply_lab(adj, buf, depth);
    } else if mode == ColorMode::Cmyk && cmyk_channel_adjustment(adj) {
        use photocraft_color::convert::{cmyk_to_rgb, rgb_to_cmyk};
        if let Adjustment::BrightnessContrast { brightness, contrast, legacy } = adj {
            let table = ink_brightness_contrast_lut(*brightness, *contrast, *legacy);
            map_rgb(buf, |c| cmyk_to_rgb(rgb_to_cmyk(c).map(|v| ink_through(&table, v))));
        } else if cmy_adjustment(adj) {
            let mut inks: Vec<[f32; 4]> = buf.px.iter().map(|p| rgb_to_cmyk([p[0], p[1], p[2]])).collect();
            apply_cmy(adj, &mut inks);
            for (p, ink) in buf.px.iter_mut().zip(&inks) {
                if p[3] > 0.0 {
                    let [r, g, b] = cmyk_to_rgb(*ink);
                    (p[0], p[1], p[2]) = (r, g, b);
                }
            }
        } else {
            map_rgb(buf, |c| {
                let mut ink = rgb_to_cmyk(c);
                stored_channels(adj, mode, depth, &mut ink);
                cmyk_to_rgb(ink)
            });
        }
    } else {
        apply_depth_mode(adj, buf, transfer, depth, mode);
    }
}

#[cfg(test)]
mod lookup_tests {
    use super::*;

    fn identity(n: usize) -> Vec<f32> {
        let m = (n - 1) as f32;
        (0..n * n * n).flat_map(|i| [(i % n) as f32 / m, ((i / n) % n) as f32 / m, (i / (n * n)) as f32 / m]).collect()
    }

    #[test]
    fn identity_lut_is_identity_both_interpolations() {
        let t = identity(5);
        for c in [[0.1, 0.5, 0.9], [0.33, 0.77, 0.0], [1.0, 1.0, 1.0]] {
            for tet in [false, true] {
                let o = lut3d_sample(&t, 5, c, tet);
                for k in 0..3 {
                    assert!((o[k] - c[k]).abs() < 1e-5, "{c:?} {tet} {o:?}");
                }
            }
        }
    }

    /// Photoshop 27.11, measured: absolute Selective Color on greys (red channel after one range's
    /// cyan change).
    #[test]
    fn absolute_selective_color_matches_photoshop() {
        // (range index, cyan %, grey, Photoshop's red)
        let measured = [
            (7, 50.0, 64u8, 32u8),
            (7, 50.0, 192, 129),
            (7, -50.0, 192, 223),
            (7, -50.0, 64, 128),
            (6, -50.0, 192, 224),
            (6, -50.0, 240, 253),
            (8, 50.0, 64, 32),
            (8, 50.0, 16, 2),
        ];
        for (range, cyan, grey, want) in measured {
            let mut adj = [[0.0f32; 4]; 9];
            adj[range][0] = cyan;
            let g = grey as f32 / 255.0;
            let got = selective_color([g; 3], false, &adj)[0] * 255.0;
            assert!((got - want as f32).abs() <= 1.5, "range {range} cyan {cyan} grey {grey}: {got} vs {want}");
        }
    }

    #[test]
    fn selective_color_zero_is_identity_and_ranges_are_local() {
        let zero = [[0.0f32; 4]; 9];
        for c in [[0.9, 0.1, 0.1], [0.5, 0.5, 0.5], [0.2, 0.4, 0.8]] {
            assert_eq!(selective_color(c, true, &zero), c);
        }
        // +100 % cyan on reds only: red loses red, blue is untouched.
        let mut a = zero;
        a[0][0] = 100.0;
        let red = selective_color([0.9, 0.1, 0.1], false, &a);
        assert!(red[0] < 0.2, "{red:?}");
        assert_eq!(selective_color([0.1, 0.1, 0.9], false, &a), [0.1, 0.1, 0.9]);
        // Relative mode can't tint pure white; absolute can (via whites).
        let mut w = zero;
        w[6] = [0.0, 0.0, 50.0, 0.0];
        assert_eq!(selective_color([1.0; 3], true, &w), [1.0; 3]);
        assert!(selective_color([1.0; 3], false, &w)[2] < 0.6);
    }

    #[test]
    fn dither_offsets_are_balanced() {
        let sum: f32 = (0..4).flat_map(|y| (0..4).map(move |x| bayer4(x, y))).sum();
        assert!(sum.abs() < 1e-5);
        assert_eq!(bayer4(-4, -4), bayer4(0, 0));
    }
}

#[cfg(test)]
mod tone_tests;

/// Legacy Brightness/Contrast scales contrast around 128 of 255 (contrast 80 keeps 128 at 128,
/// measured on Photoshop 27.11; a 0.5 pivot moved it to 130).
pub const LEGACY_PIVOT: f32 = 128.0 / 255.0;

#[cfg(test)]
mod lut3d_guard_tests {
    use super::lut3d_sample;

    #[test]
    fn degenerate_or_short_tables_leave_the_colour_alone() {
        let c = [0.2, 0.4, 0.6];
        for tetra in [false, true] {
            assert_eq!(lut3d_sample(&[], 0, c, tetra), c);
            assert_eq!(lut3d_sample(&[0.0; 3], 1, c, tetra), c);
            assert_eq!(lut3d_sample(&[0.0; 3], 2, c, tetra), c);
            assert_eq!(lut3d_sample(&[0.0; 24], usize::MAX, c, tetra), c);
        }
    }
}
