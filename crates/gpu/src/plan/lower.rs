use super::types::Unsupported;
use super::{Adjustment, BlendMode, Fill, Gradient, Layer, LayerContent, Pattern, StrokePosition, ToneSpace, Transfer, adjust, lut3d_len};

/// Paint source of an effect.
pub(super) enum Paint<'a> {
    None,
    Color([f32; 3]),
    Gradient(&'a Gradient),
    /// A glow's gradient with its opacity gain (`effects::paint_glow`).
    GlowGradient(&'a Gradient, f32),
    Pattern(&'a Pattern, Placement),
}

/// `compose::pattern::Placement` (origin, inverse rotation, inverse scale).
#[derive(Clone, Copy, Debug)]
pub(super) struct Placement {
    pub(super) origin: (f64, f64),
    pub(super) cs: (f64, f64),
    pub(super) inv_scale: f64,
}

/// `Placement::anchored`: linked patterns tile from `anchor` (the effects reference point).
pub(super) fn placement(anchor: (f64, f64), link: bool, phase: (f32, f32), scale: f32, angle: f32) -> Placement {
    let base = if link { anchor } else { (0.0, 0.0) };
    let origin = (base.0 + f64::from(phase.0), base.1 + f64::from(phase.1));
    let s = f64::from(scale);
    let inv_scale = if s.is_finite() && s > 1e-3 { 1.0 / s } else { 1.0 };
    let a = f64::from(angle).to_radians();
    Placement { origin, cs: (a.cos(), a.sin()), inv_scale }
}

/// (inside width, outside width) of a stroke.
pub fn stroke_widths(st: &photocraft_doc::StrokeFx) -> (f32, f32) {
    match st.position {
        StrokePosition::Outside => (0.0, st.size),
        StrokePosition::Inside => (st.size, 0.0),
        StrokePosition::Center => (st.size / 2.0, st.size / 2.0),
    }
}

pub(super) fn style_index(s: photocraft_doc::GradientStyle) -> f32 {
    match s {
        photocraft_doc::GradientStyle::Linear => 0.0,
        photocraft_doc::GradientStyle::Radial => 1.0,
        photocraft_doc::GradientStyle::Angle => 2.0,
        photocraft_doc::GradientStyle::Reflected => 3.0,
        photocraft_doc::GradientStyle::Diamond => 4.0,
    }
}

fn sample_stops4(stops: &[(f32, [f32; 4])], t: f32) -> [f32; 4] {
    match stops {
        [] => [0.0; 4],
        [only] => only.1,
        _ => {
            if t <= stops[0].0 {
                return stops[0].1;
            }
            for w in stops.windows(2) {
                let (a, b) = (&w[0], &w[1]);
                if t <= b.0 {
                    let k = if b.0 > a.0 { (t - a.0) / (b.0 - a.0) } else { 0.0 };
                    return std::array::from_fn(|i| a.1[i] + (b.1[i] - a.1[i]) * k);
                }
            }
            stops[stops.len() - 1].1
        }
    }
}

fn lut_rows(f: impl Fn(usize, f32) -> f32, rows: usize) -> Vec<[f32; 4096]> {
    (0..rows)
        .map(|r| {
            let mut row = [0.0f32; 4096];
            for (k, v) in row.iter_mut().enumerate() {
                *v = f(r, k as f32 / 4095.0);
            }
            row
        })
        .collect()
}

type Program = (i32, [[f32; 4]; 4], Option<Vec<[f32; 4096]>>);

/// A CPU LUT (4096 entries) as one texture row.
fn to_row(t: &[f32]) -> [f32; 4096] {
    let mut row = [0.0f32; 4096];
    for (o, v) in row.iter_mut().zip(t) {
        *o = *v;
    }
    row
}

/// Whether the adjustment kernel can evaluate `adj` in a document of colour `mode`: Levels and
/// Curves on CMYK ink or Lab channels convert through ICC profiles per pixel, and so do the
/// adjustments a Lab document evaluates on L, a and b (`photocraft_compose::adjust::lab_channel_adjustment`)
/// and a CMYK document on its inks (`cmyk_channel_adjustment`), which only the CPU does.
pub fn adjustment_on_gpu(adj: &Adjustment, mode: photocraft_color::ColorMode) -> bool {
    use photocraft_compose::adjust::{cmyk_channel_adjustment, lab_channel_adjustment};
    let tone_in_space =
        matches!(adj, Adjustment::Levels { space: ToneSpace::Cmyk | ToneSpace::Lab, .. } | Adjustment::Curves { space: ToneSpace::Cmyk | ToneSpace::Lab, .. });
    let own_channels = match mode {
        photocraft_color::ColorMode::Lab => lab_channel_adjustment(adj),
        photocraft_color::ColorMode::Cmyk => cmyk_channel_adjustment(adj),
        _ => false,
    };
    !tone_in_space && !own_channels
}

/// A tone transfer as the shader's `t_decode`/`t_encode` exponent (0: the sRGB curve).
fn transfer_exponent(t: Transfer) -> f32 {
    match t {
        Transfer::Srgb => 0.0,
        Transfer::Gamma(g) => g,
    }
}

/// Adjustment → (kernel kind, parameters, LUT rows). Kinds are the `switch` in `adjust()`.
pub fn adjustment_program(adj: &Adjustment, transfer: Transfer, depth: photocraft_color::SampleType, mode: photocraft_color::ColorMode) -> Program {
    let mut p = [[0.0f32; 4]; 4];
    match adj {
        Adjustment::Invert => (1, p, None),
        Adjustment::Threshold { level } => {
            // The level, the depth's split (compose::adjust::threshold_white) and the level in 16 bits.
            let t = (level * 255.0).round();
            p[0] = match depth {
                photocraft_color::SampleType::U8 => [t, 1.0, 0.0, 0.0],
                photocraft_color::SampleType::U16 => [t, 2.0, (t * 32768.0 / 255.0).round(), 0.0],
                photocraft_color::SampleType::F32 => [t, 0.0, 0.0, 0.0],
            };
            (2, p, None)
        }
        Adjustment::Posterize { levels } => {
            // The levels, and the 16-bit rule (compose::adjust::posterize_in).
            p[0][0] = (*levels).clamp(2, 255) as f32;
            p[0][1] = if depth == photocraft_color::SampleType::U16 { 1.0 } else { 0.0 };
            (3, p, None)
        }
        Adjustment::BrightnessContrast { brightness, contrast, legacy: true } => {
            let c = contrast.clamp(-100.0, 99.0);
            let k = if c >= 0.0 { 1.0 / (1.0 - c / 100.0) } else { 1.0 + c / 100.0 };
            // Brightness before contrast when it is raised, after it when lowered (compose::adjust).
            let (pre, post) = if c >= 0.0 { (brightness / 255.0, 0.0) } else { (0.0, brightness / 255.0) };
            // 32-bit documents pivot at 127 of 255 and don't clip (the shader checks F_HDR).
            let pivot = if depth == photocraft_color::SampleType::F32 { 127.0 / 255.0 } else { photocraft_compose::adjust::LEGACY_PIVOT };
            p[0] = [pre, k, pivot, post];
            (4, p, None)
        }
        Adjustment::BrightnessContrast { brightness, contrast, .. } => {
            // Modern B/C: the CPU's lookup table (compose::adjust::modern_lut) on every channel,
            // through the Levels / Curves kernel.
            let row = to_row(&adjust::modern_lut(*brightness, *contrast));
            (7, p, Some(vec![row; 3]))
        }
        Adjustment::Exposure { exposure, offset, gamma } => {
            p[0] = [2f32.powf(*exposure), *offset, gamma.max(0.01), transfer_exponent(transfer.for_exposure())];
            (6, p, None)
        }
        Adjustment::Levels { master, per_channel, space: ToneSpace::Rgb, .. } if depth == photocraft_color::SampleType::F32 => {
            // Twenty coefficients exceed the sixteen uniform parameters. Store four records
            // in LUT row 0, read texel-exact by the analytic float Levels kernel.
            let mut row = [0.0f32; 4096];
            for (i, ch) in std::iter::once(master).chain(per_channel).enumerate() {
                let coefficients = [ch.in_black, ch.in_white, ch.gamma.max(0.01), ch.out_black, ch.out_white];
                row[i * 5..i * 5 + 5].copy_from_slice(&coefficients);
            }
            (17, p, Some(vec![row]))
        }
        // RGB space only (see `adjustment_on_gpu`); the rows are the CPU's channel∘master LUTs.
        Adjustment::Levels { .. } | Adjustment::Curves { .. } => {
            for (v, identity) in p[0].iter_mut().zip(adjust::float_tone_identity(adj, Some(depth))) {
                *v = if identity { 1.0 } else { 0.0 };
            }
            // p0.w: continue the curve past 0..1 (32-bit RGB Curves, #7157).
            p[0][3] = if adjust::float_tone_extrapolates(adj, Some(depth), mode) { 1.0 } else { 0.0 };
            (7, p, Some(adjust::tone_luts_depth(adj, Some(depth)).iter().take(3).map(|t| to_row(t)).collect()))
        }
        Adjustment::HueSaturation { hue, saturation, lightness, colorize, ranges } => {
            let sat = adjust::colorize_saturation(saturation / 100.0, lightness / 100.0, *colorize, Some(depth));
            p[0] = [*hue, sat, lightness / 100.0, if *colorize { 1.0 } else { 0.0 }];
            if !*colorize && ranges.iter().any(|r| !r.is_neutral()) {
                // Range edits: hue-indexed rows (shift, saturation, lightness), as on the CPU.
                p[1][0] = 1.0;
                (8, p, Some(adjust::hue_range_tables(ranges).iter().map(|t| to_row(t)).collect()))
            } else {
                (8, p, None)
            }
        }
        Adjustment::Vibrance { vibrance, saturation } => {
            // compose::adjust::vibrance_px; 32-bit samples aren't clipped at 1.
            let hi = if depth == photocraft_color::SampleType::F32 { f32::MAX } else { 1.0 };
            p[0] = [vibrance / 100.0, saturation / 100.0, transfer_exponent(transfer), hi];
            let b = adjust::VIBRANCE_BOOST;
            p[1] = [b[0], b[1], b[2], b[3]];
            p[2] = [b[4], b[5], b[6], 0.0];
            (9, p, None)
        }
        Adjustment::ChannelMixer { matrix, monochrome } => {
            // compose::adjust: 32-bit documents cut each percentage to 1/1024, count 255/512 of
            // the constant and don't clip (the shader checks F_HDR).
            let hdr = depth == photocraft_color::SampleType::F32;
            let cut = |v: f32| if hdr { (v * 1024.0).trunc() / 1024.0 } else { v };
            let rows = matrix.map(|r| [cut(r[0]), cut(r[1]), cut(r[2]), if hdr { r[3] * 255.0 / 512.0 } else { r[3] }]);
            p[0] = rows[0];
            p[1] = rows[1];
            p[2] = rows[2];
            p[3][0] = if *monochrome { 1.0 } else { 0.0 };
            (10, p, None)
        }
        Adjustment::PhotoFilter { color, density, preserve_luminosity } => {
            // compose::adjust: the linear-light matrix, then SetLum on the encoded values (8/16-bit).
            let linear_doc = depth == photocraft_color::SampleType::F32;
            let m = adjust::photo_filter_matrix(*color, *density, *preserve_luminosity && linear_doc);
            for (row, m) in p.iter_mut().zip(m) {
                *row = [m[0], m[1], m[2], 0.0];
            }
            p[3] = [if *preserve_luminosity && !linear_doc { 1.0 } else { 0.0 }, transfer_exponent(transfer), 0.0, 0.0];
            (11, p, None)
        }
        Adjustment::BlackWhite { weights, tint } => {
            p[0] = [weights[0], weights[1], weights[2], weights[3]];
            p[1] = [weights[4], weights[5], if tint.is_some() { 1.0 } else { 0.0 }, 0.0];
            if let Some(t) = tint {
                p[2] = [t[0], t[1], t[2], 0.0];
            }
            (12, p, None)
        }
        Adjustment::GradientMap { stops, reverse, dither } => {
            p[0][0] = if *reverse { 1.0 } else { 0.0 };
            p[0][1] = if *dither { 1.0 } else { 0.0 };
            let s4: Vec<(f32, [f32; 4])> = stops.iter().map(|(t, c)| (*t, [c[0], c[1], c[2], 1.0])).collect();
            let rows = lut_rows(|r, t| if s4.is_empty() { t } else { sample_stops4(&s4, t)[r] }, 3);
            (13, p, Some(rows))
        }
        Adjustment::ColorBalance { shadows, midtones, highlights, preserve_luminosity } => {
            // Per-channel curves (compose::adjust::BalanceCurve), through the Levels kernel.
            let curves = adjust::BalanceCurve::rgb([*shadows, *midtones, *highlights], *preserve_luminosity);
            (7, p, Some(lut_rows(|ch, v| curves[ch].eval(v), 3)))
        }
        Adjustment::SelectiveColor { relative, adjustments } => {
            // 9 ranges × CMYK don't fit the 16 params: they go in LUT row 0 (read texel-exact).
            p[0][0] = if *relative { 1.0 } else { 0.0 };
            let mut row = [0.0f32; 4096];
            for (i, v) in adjustments.iter().flatten().enumerate() {
                row[i] = *v;
            }
            (15, p, Some(vec![row]))
        }
        Adjustment::ColorLookup { lut: Some(table), size, tetrahedral, dither, .. } if lut3d_len(*size).is_some_and(|len| table.len() >= len) => {
            // The flattened table (n³ RGB triplets) spans as many 4096-wide rows as it needs.
            let n = *size as usize;
            let rows = lut3d_len(*size)
                .and_then(|len| table.get(..len))
                .unwrap_or_default()
                .chunks(4096)
                .map(|c| {
                    let mut row = [0.0f32; 4096];
                    row[..c.len()].copy_from_slice(c);
                    row
                })
                .collect();
            p[0] = [n as f32, if *tetrahedral { 1.0 } else { 0.0 }, if *dither { 1.0 } else { 0.0 }, 0.0];
            (16, p, Some(rows))
        }
        // Identity on the CPU too (not evaluated there).
        Adjustment::ColorLookup { .. } | Adjustment::Unsupported { .. } => (0, p, None),
    }
}

/// A gradient's colour and alpha as four 4096-entry LUT rows (`lut_tex`).
pub(super) fn gradient_rows(g: &Gradient) -> Vec<[f32; 4096]> {
    let mut rows = vec![[0.0f32; 4096]; 4];
    for k in 0..4096 {
        let v = photocraft_compose::effects::sample_gradient(g, k as f32 / 4095.0);
        for (ch, row) in rows.iter_mut().enumerate() {
            row[k] = v[ch];
        }
    }
    rows
}

/// Linear interpolation between LUT entries cannot preserve a discontinuity. Let the canvas
/// use the CPU oracle for coincident stops, including opacity stops and unsorted fill ramps
/// (which `gradient_fill::Ramp` sorts). Check only ramps we will sample: cached fills and
/// disabled effects can still use the GPU.
pub(super) fn check_gradient_stops<T>(stops: &[(f32, T)]) -> Result<(), Unsupported> {
    let mut positions: Vec<f32> = stops.iter().map(|s| s.0).collect();
    positions.sort_by(f32::total_cmp);
    // A 4096-entry LUT cannot keep two stops that fall in the same entry apart (#3053).
    if positions.windows(2).any(|w| (w[1] - w[0]).abs() < 1.0 / 4095.0) {
        return Err(Unsupported("coincident gradient stops (composited on the CPU)".into()));
    }
    Ok(())
}

/// Mode index used by the shader (declaration order of [`BlendMode`]).
pub fn mode_index(m: BlendMode) -> i32 {
    match m {
        BlendMode::PassThrough => 0,
        BlendMode::Normal => 1,
        BlendMode::Dissolve => 2,
        BlendMode::Darken => 3,
        BlendMode::Multiply => 4,
        BlendMode::ColorBurn => 5,
        BlendMode::LinearBurn => 6,
        BlendMode::DarkerColor => 7,
        BlendMode::Lighten => 8,
        BlendMode::Screen => 9,
        BlendMode::ColorDodge => 10,
        BlendMode::LinearDodge => 11,
        BlendMode::LighterColor => 12,
        BlendMode::Overlay => 13,
        BlendMode::SoftLight => 14,
        BlendMode::HardLight => 15,
        BlendMode::VividLight => 16,
        BlendMode::LinearLight => 17,
        BlendMode::PinLight => 18,
        BlendMode::HardMix => 19,
        BlendMode::Difference => 20,
        BlendMode::Exclusion => 21,
        BlendMode::Subtract => 22,
        BlendMode::Divide => 23,
        BlendMode::Hue => 24,
        BlendMode::Saturation => 25,
        BlendMode::Color => 26,
        BlendMode::Luminosity => 27,
        BlendMode::Reflect => 28,
        BlendMode::Glow => 29,
        BlendMode::Negation => 30,
        BlendMode::Xor => 31,
        BlendMode::PaintNetColorBurn => 32,
        BlendMode::PaintNetColorDodge => 33,
    }
}

/// Whether `Planner::content` draws `layer` as one `Planner::content_pass`: not a group's stack,
/// an adjustment, or a pattern fill painted in passes.
pub(super) fn single_content_pass(layer: &Layer) -> bool {
    match &layer.content {
        LayerContent::Group(_) | LayerContent::Adjustment(_) => false,
        LayerContent::Fill(f @ Fill::Pattern { .. }) => layer.fill_cache.as_ref().is_some_and(|c| c.fill == *f),
        _ => true,
    }
}
