// Frozen pre-optimization dab kernel. Keep independent of the optimized implementation.
use super::*;
/// Keep the allocation-heavy scalar kernel independent so optimizations cannot silently change pickup or blending.
fn reference(
    work: &mut crate::retouch::Region,
    fp: &crate::retouch::Footprint,
    fmt: photocraft_color::PixelFormat,
    sample: Option<&Surface>,
    m: &MixerSettings,
    state: &mut MixerState,
    lock_transparency: bool,
) {
    let n = fmt.channels();
    let a = alpha_index(&fmt);
    let nc = fmt.mode.color_channels();
    let native = |c: [f32; 4]| {
        let mut v = from_rgba(&fmt, c);
        if let Some(ai) = a {
            v[ai] = 1.0;
        }
        v
    };
    let res = state.reservoir.map(native);
    let mut amount = if res.is_some() { state.amount.clamp(0.0, 1.0) } else { 0.0 };
    let mut pickup: Option<Vec<f32>> = state.pickup.map(native);
    let (wet, mix, flow, load) = (m.wet.clamp(0.0, 1.0), m.mix.clamp(0.0, 1.0), m.flow.clamp(0.0, 1.0), m.load.clamp(0.0, 1.0));
    let mut acc = vec![0.0f32; n];
    let mut paint = vec![0.0f32; n];
    // Transfer › Wetness / Mix Jitter scale the settings per dab.
    let (wet, mix) = (wet * fp.dab.wet.clamp(0.0, 1.0), mix * fp.dab.mix.clamp(0.0, 1.0));
    // Sample the canvas under the dab.
    acc.iter_mut().for_each(|v| *v = 0.0);
    let (mut wsum, mut asum) = (0.0f32, 0.0f32);
    let r = fp.rect.intersect(&work.rect);
    for y in r.y0..r.y1 {
        for x in r.x0..r.x1 {
            let w = fp.at(x, y);
            if w <= 0.0 {
                continue;
            }
            let px: Vec<f32> = match sample {
                Some(s) => {
                    let v = s.pixel(x, y);
                    if s.format() == fmt { v } else { from_rgba(&fmt, s.rgba(x, y)) }
                }
                None => work.px(x, y).to_vec(),
            };
            let al = a.map_or(1.0, |ai| px[ai]);
            for c in 0..nc {
                acc[c] += px[c] * w * al;
            }
            wsum += w;
            asum += w * al;
        }
    }
    if asum > 1e-6 && wet > 0.0 {
        let ca = asum / wsum.max(1e-6);
        let canvas: Vec<f32> = (0..n).map(|c| if c < nc { acc[c] / asum } else { 1.0 }).collect();
        pickup = Some(match pickup.take() {
            None => canvas,
            Some(p) => p.iter().zip(&canvas).map(|(p, c)| p + (c - p) * PICKUP_RATE * wet * ca).collect(),
        });
    }
    let wr = if res.is_some() { amount * (1.0 - mix * wet) } else { 0.0 };
    let wp = if pickup.is_some() { mix * wet } else { 0.0 };
    let total = wr + wp;
    if total > 1e-6 {
        for c in 0..n {
            let rv = res.as_ref().map_or(0.0, |v| v[c]);
            let pv = pickup.as_ref().map_or(0.0, |v| v[c]);
            paint[c] = (rv * wr + pv * wp) / total;
        }
        let dep = total.min(1.0) * flow;
        for y in r.y0..r.y1 {
            for x in r.x0..r.x1 {
                let k = fp.at(x, y) * dep;
                if k > 0.0 {
                    if let Some(load) = &state.spatial {
                        let loaded = load_at(load, &fp.dab, x as f32 + 0.5, y as f32 + 0.5);
                        let local = from_rgba(&fmt, loaded);
                        let local_wr = wr * loaded[3];
                        let local_total = local_wr + wp;
                        if local_total > 1e-6 {
                            let mut mixed = paint.clone();
                            for (c, value) in mixed.iter_mut().enumerate() {
                                *value = (local.get(c).copied().unwrap_or(0.0) * local_wr
                                    + pickup.as_ref().and_then(|p| p.get(c)).copied().unwrap_or(0.0) * wp)
                                    / local_total;
                            }
                            if let Some(ai) = a
                                && let Some(alpha) = mixed.get_mut(ai)
                            {
                                *alpha = 1.0;
                            }
                            over_native(&fmt, work.px_mut(x, y), &mixed, fp.at(x, y) * local_total.min(1.0) * flow, lock_transparency);
                        }
                    } else {
                        over_native(&fmt, work.px_mut(x, y), &paint, k, lock_transparency);
                    }
                }
            }
        }
    }
    amount *= 1.0 - 0.08 * (1.0 - load);
    state.amount = amount;
    state.pickup = pickup.map(|p| to_rgba(&fmt, &p));
}

/// The incremental kernel matches the frozen reference across color models, bit depths and spatial loads.
#[test]
fn contiguous_mixer_matches_frozen_kernel_across_models_depths_and_spatial_loads() {
    use crate::retouch::Region;
    use crate::{BrushContext, BrushSettings, Stroke, StrokePoint};
    use photocraft_color::{ColorMode, PixelFormat, SampleType};
    let rect = Rect::new(-16, -16, 32, 32);
    for mode in [ColorMode::Rgb, ColorMode::Grayscale, ColorMode::Cmyk, ColorMode::Lab] {
        for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
            for alpha in [false, true] {
                let fmt = PixelFormat::new(mode, depth, alpha);
                let mut surface = Surface::new(fmt);
                let values: Vec<_> = (0..rect.width() * rect.height()).flat_map(|i| from_rgba(&fmt, [(i % 31) as f32 / 30.0, 0.3, 0.7, 0.6])).collect();
                surface.write_region(rect, &values);
                let mut source = Surface::new(PixelFormat::RGBA32F);
                source.fill_rect(rect, &[0.8, 0.2, 0.1, 0.75]);
                let brush = BrushSettings { size: 24.0, angle: 33.0, roundness: 0.6, pressure_size: false, ..Default::default() };
                let dabs = crate::dabs(&Stroke { brush: brush.clone(), points: vec![StrokePoint::new(4.0, 4.0, 1.0)] });
                let ctx = BrushContext::new(&brush);
                for spatial in [false, true] {
                    for sample in [None, Some(&source), Some(&surface)] {
                        for lock in [false, true] {
                            let mut state = MixerState::default();
                            state.load([0.1, 0.8, 0.2, 1.0]);
                            if spatial {
                                state.spatial =
                                    Some((0..4096).map(|i| [if i % 64 < 32 { 1.0 } else { 0.0 }, 0.2, 0.8, if i % 7 == 0 { 0.0 } else { 1.0 }]).collect());
                            }
                            let mut expected_state = state.clone();
                            let mut actual = Region::read(&surface, rect);
                            let mut expected = actual.clone();
                            let settings = MixerSettings::default();
                            for (index, dab) in dabs.iter().enumerate() {
                                let fp = ctx.footprint(dab, index);
                                reference(&mut expected, &fp, fmt, sample, &settings, &mut expected_state, lock);
                                apply_mixer_dab(&mut actual, &fp, fmt, sample, &settings, &mut state, lock);
                            }
                            assert_eq!(actual, expected, "{mode:?} {depth:?} alpha={alpha} spatial={spatial} lock={lock}");
                            assert_eq!(state, expected_state);
                        }
                    }
                }
            }
        }
    }
}

/// Dry dabs must preserve both the painted pixels and the carried pickup across transitions.
#[test]
fn dry_and_wet_transitions_match_frozen_kernel() {
    use crate::retouch::Region;
    use crate::{BrushContext, BrushSettings, Dab};
    use photocraft_color::{ColorMode, PixelFormat, SampleType};
    use photocraft_geom::Point;

    let rect = Rect::new(0, 0, 48, 48);
    for mode in [ColorMode::Rgb, ColorMode::Grayscale, ColorMode::Cmyk, ColorMode::Lab] {
        for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
            for alpha in [false, true] {
                let fmt = PixelFormat::new(mode, depth, alpha);
                let mut surface = Surface::new(fmt);
                surface.fill_rect(rect, &from_rgba(&fmt, [0.25, 0.5, 0.75, 0.6]));
                let mut source = Surface::new(PixelFormat::RGBA32F);
                source.fill_rect(rect, &[0.7, 0.1, 0.3, 0.8]);
                let ctx = BrushContext::new(&BrushSettings { size: 16.0, ..Default::default() });
                for spatial in [false, true] {
                    for loaded in [false, true] {
                        for sample in [None, Some(&source)] {
                            for lock in [false, true] {
                                let mut actual = Region::read(&surface, rect);
                                let mut expected = actual.clone();
                                let mut state = MixerState::default();
                                if loaded {
                                    state.load([0.8, 0.2, 0.1, 1.0]);
                                }
                                if spatial {
                                    state.spatial = Some(vec![[0.2, 0.8, 0.4, 0.7]; 4096]);
                                }
                                let mut expected_state = state.clone();
                                for (i, (base_wet, dab_wet, mix)) in
                                    [(0.7, 1.0, 0.0), (0.0, 1.0, 0.7), (0.7, 0.0, 0.7), (0.7, 1.0, 0.7)].into_iter().enumerate()
                                {
                                    let mut dab = Dab::round(Point::new(20.0 + i as f64, 20.0), 8.0, 1.0);
                                    dab.wet = dab_wet;
                                    let fp = ctx.footprint(&dab, i);
                                    let settings = MixerSettings { wet: base_wet, mix, ..Default::default() };
                                    reference(&mut expected, &fp, fmt, sample, &settings, &mut expected_state, lock);
                                    apply_mixer_dab(&mut actual, &fp, fmt, sample, &settings, &mut state, lock);
                                    assert_eq!(actual, expected, "{mode:?} {depth:?} alpha={alpha} spatial={spatial} loaded={loaded} lock={lock} dab={i}");
                                    assert_eq!(state, expected_state, "mixer state after dab {i}");
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
