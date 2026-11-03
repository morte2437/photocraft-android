use super::*;

const E: f32 = 1e-5;

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < E
}

#[test]
fn psd_keys_roundtrip_and_unique() {
    let mut seen = std::collections::HashSet::new();
    for m in std::iter::once(BlendMode::PassThrough).chain(BlendMode::LAYER_MODES) {
        assert!(seen.insert(m.psd_key()), "duplicate key {m:?}");
        assert_eq!(BlendMode::from_psd_key(m.psd_key()), Some(m));
    }
    assert_eq!(BlendMode::from_psd_key(*b"zzzz"), None);
    assert_eq!(seen.len(), 28);
}

#[test]
fn paint_net_modes_keep_their_math_and_distinct_psd_status() {
    assert!(close(blend_channel(BlendMode::Reflect, 0.5, 0.5), 0.5));
    assert!(close(blend_channel(BlendMode::Reflect, 0.2, 0.0), 0.04));
    assert!(close(blend_channel(BlendMode::Glow, 0.5, 0.2), 0.08));
    assert!(close(blend_channel(BlendMode::Negation, 0.2, 0.7), 0.9));
    assert_eq!(blend_channel(BlendMode::Reflect, 0.0, 1.0), 1.0);
    assert_eq!(blend_channel(BlendMode::Glow, 1.0, 0.0), 1.0);
    assert_eq!(blend_channel(BlendMode::PaintNetColorBurn, 1.0, 0.0), 0.0);
    assert_eq!(blend_channel(BlendMode::PaintNetColorDodge, 0.0, 1.0), 1.0);
    for b in 0..=255u8 {
        for s in 0..=255u8 {
            assert!(close(blend_channel(BlendMode::Xor, f32::from(b) / 255.0, f32::from(s) / 255.0), f32::from(b ^ s) / 255.0));
        }
    }
    for mode in BlendMode::PAINT_NET_MODES {
        assert!(!mode.has_psd_equivalent());
        assert_eq!(mode.psd_key(), *b"norm");
        for b in [0.0, 0.1, 0.5, 1.0] {
            for s in [0.0, 0.1, 0.5, 1.0] {
                let v = blend_channel(mode, b, s);
                assert!(v.is_finite() && (0.0..=1.0).contains(&v), "{mode:?}");
            }
        }
    }
    assert_eq!(BlendMode::layer_modes().count(), 33);
}

#[test]
fn normal_is_source() {
    for &(cb, cs) in &[(0.2, 0.7), (1.0, 0.0), (0.0, 1.0)] {
        assert!(close(blend_channel(BlendMode::Normal, cb, cs), cs));
    }
}

#[test]
fn identities_with_white_and_black() {
    // Multiply by white / screen with black / add black / darken with white leave backdrop unchanged.
    for cb in [0.0f32, 0.1, 0.5, 0.9, 1.0] {
        assert!(close(blend_channel(BlendMode::Multiply, cb, 1.0), cb));
        assert!(close(blend_channel(BlendMode::Screen, cb, 0.0), cb));
        assert!(close(blend_channel(BlendMode::LinearDodge, cb, 0.0), cb));
        assert!(close(blend_channel(BlendMode::Darken, cb, 1.0), cb));
        assert!(close(blend_channel(BlendMode::Lighten, cb, 0.0), cb));
        assert!(close(blend_channel(BlendMode::Difference, cb, 0.0), cb));
        assert!(close(blend_channel(BlendMode::Subtract, cb, 0.0), cb));
        assert!(close(blend_channel(BlendMode::LinearBurn, cb, 1.0), cb));
        assert!(close(blend_channel(BlendMode::Exclusion, cb, 0.0), cb));
        // 50% gray is neutral for the "light" family
        assert!(close(blend_channel(BlendMode::HardLight, cb, 0.5), cb));
        assert!(close(blend_channel(BlendMode::SoftLight, cb, 0.5), cb));
        assert!(close(blend_channel(BlendMode::LinearLight, cb, 0.5), cb));
    }
}

#[test]
fn known_values() {
    assert!(close(blend_channel(BlendMode::Multiply, 0.5, 0.5), 0.25));
    assert!(close(blend_channel(BlendMode::Screen, 0.5, 0.5), 0.75));
    assert!(close(blend_channel(BlendMode::Overlay, 0.25, 0.5), 0.25));
    assert!(close(blend_channel(BlendMode::Overlay, 0.75, 1.0), 1.0));
    assert!(close(blend_channel(BlendMode::ColorDodge, 0.5, 0.5), 1.0));
    assert!(close(blend_channel(BlendMode::ColorBurn, 0.5, 0.5), 0.0));
    assert!(close(blend_channel(BlendMode::ColorBurn, 1.0, 0.0), 1.0));
    assert!(close(blend_channel(BlendMode::ColorDodge, 0.0, 1.0), 0.0));
    assert!(close(blend_channel(BlendMode::Divide, 0.25, 0.5), 0.5));
    assert!(close(blend_channel(BlendMode::Divide, 0.0, 0.0), 0.0));
    assert!(close(blend_channel(BlendMode::Divide, 0.3, 0.0), 1.0));
    assert!(close(blend_channel(BlendMode::Exclusion, 0.5, 0.5), 0.5));
    assert!(close(blend_channel(BlendMode::PinLight, 0.8, 0.2), 0.4));
    assert!(close(blend_channel(BlendMode::PinLight, 0.1, 0.9), 0.8));
    assert!(close(blend_channel(BlendMode::HardMix, 0.6, 0.4), 1.0));
    assert!(close(blend_channel(BlendMode::HardMix, 0.6, 0.3), 0.0));
}

/// #8804: Photoshop's Soft Light over a dark backdrop (cb ≤ 0.25, cs > 0.5) follows the W3C
/// curve `D(cb) = ((16·cb − 12)·cb + 4)·cb`, not `sqrt(cb)`. Measured on psd-tools
/// `blend-modes/rgb-blend-modes.psd` (Photoshop's merged image), in 8-bit levels:
/// (backdrop, source, Photoshop).
#[test]
fn soft_light_follows_photoshop_over_dark_backdrops() {
    for (cb, cs, ps) in [(4u8, 247u8, 11.0f32), (6, 254, 19.0), (9, 252, 25.0), (10, 237, 32.0), (14, 253, 46.0)] {
        let got = blend_channel(BlendMode::SoftLight, f32::from(cb) / 255.0, f32::from(cs) / 255.0) * 255.0;
        assert!((got - ps).abs() <= 7.0, "cb {cb} cs {cs}: {got} vs Photoshop {ps}");
    }
}

#[test]
fn soft_light_is_the_w3c_formula() {
    let d = |cb: f32| if cb <= 0.25 { ((16.0 * cb - 12.0) * cb + 4.0) * cb } else { cb.sqrt() };
    let w3c = |cb: f32, cs: f32| if cs <= 0.5 { cb - (1.0 - 2.0 * cs) * cb * (1.0 - cb) } else { cb + (2.0 * cs - 1.0) * (d(cb) - cb) };
    for b in (0..=255u8).step_by(3) {
        for s in (0..=255u8).step_by(5) {
            let (cb, cs) = (f32::from(b) / 255.0, f32::from(s) / 255.0);
            let got = blend_channel(BlendMode::SoftLight, cb, cs);
            assert!((got - w3c(cb, cs)).abs() < 1e-5, "cb {b} cs {s}: {got} vs {}", w3c(cb, cs));
        }
    }
    // Continuous where the two halves of D meet (both give 0.5 at cb = 0.25).
    let below = blend_channel(BlendMode::SoftLight, 0.25, 1.0);
    let above = blend_channel(BlendMode::SoftLight, 0.25 + 1e-6, 1.0);
    assert!(close(below, 0.5) && (above - below).abs() < 1e-4, "{below} {above}");
    // A negative backdrop (32-bit documents) counts as black under a light source, as before.
    assert!(close(blend_channel(BlendMode::SoftLight, -0.1, 1.0), 0.0));
}

#[test]
fn vivid_light_source_extremes_and_interior_values() {
    for cb in [0.0, 0.25, 0.5, 0.75, 1.0] {
        assert_eq!(blend_channel(BlendMode::VividLight, cb, 0.0), 0.0);
        assert_eq!(blend_channel(BlendMode::VividLight, cb, 1.0), 1.0);
    }
    assert!(close(blend_channel(BlendMode::VividLight, 0.75, 0.25), 0.5));
    assert!(close(blend_channel(BlendMode::VividLight, 0.25, 0.75), 0.5));
}

#[test]
fn hard_mix_matches_photoshop_extremes_and_interior_values() {
    assert_eq!(blend_channel(BlendMode::HardMix, 1.0, 0.0), 1.0);
    assert_eq!(blend_channel(BlendMode::HardMix, 0.0, 1.0), 0.0);
    assert_eq!(blend_channel(BlendMode::HardMix, 1.0 - 1e-5, 0.0), 1.0);
    assert_eq!(blend_channel(BlendMode::HardMix, 1e-5, 1.0), 0.0);
    assert_eq!(blend_channel(BlendMode::HardMix, 0.6, 0.4), 1.0);
    assert_eq!(blend_channel(BlendMode::HardMix, 0.6, 0.3), 0.0);
}

#[test]
fn vivid_light_and_hard_mix_composite_extremes() {
    assert_eq!(composite(BlendMode::VividLight, [1.0, 1.0, 1.0, 1.0], [0.0, 0.0, 0.0, 1.0], 1.0), [0.0, 0.0, 0.0, 1.0]);
    assert_eq!(composite(BlendMode::VividLight, [0.0, 0.0, 0.0, 1.0], [1.0, 1.0, 1.0, 1.0], 1.0), [1.0, 1.0, 1.0, 1.0]);
    assert_eq!(composite(BlendMode::HardMix, [0.0, 0.0, 0.0, 1.0], [1.0, 1.0, 1.0, 1.0], 1.0), [0.0, 0.0, 0.0, 1.0]);
    assert_eq!(composite(BlendMode::HardMix, [1.0, 1.0, 1.0, 1.0], [0.0, 0.0, 0.0, 1.0], 1.0), [1.0, 1.0, 1.0, 1.0]);
}

#[test]
fn separable_outputs_stay_in_unit_range() {
    let steps: Vec<f32> = (0..=20).map(|i| i as f32 / 20.0).collect();
    for m in BlendMode::LAYER_MODES {
        for &cb in &steps {
            for &cs in &steps {
                let v = blend_rgb(m, [cb, cs, 0.5], [cs, cb, 0.25]);
                for c in v {
                    assert!((-E..=1.0 + E).contains(&c), "{m:?} cb={cb} cs={cs} -> {c}");
                }
            }
        }
    }
}

#[test]
fn add_and_divide_clip_at_integer_depths_but_not_in_float() {
    // Integer depths: Photoshop clips Linear Dodge (Add) and Divide at 1.
    assert_eq!(blend_channel(BlendMode::LinearDodge, 2.0, 0.5), 1.0);
    assert_eq!(blend_channel(BlendMode::LinearDodge, 0.75, 0.5), 1.0);
    assert_eq!(blend_channel(BlendMode::Divide, 2.0, 0.5), 1.0);
    assert_eq!(blend_channel(BlendMode::Divide, 0.5, 0.25), 1.0);
    // 32-bit float: light adds up past 1 (adding light never darkens).
    assert!(close(blend_channel_hdr(BlendMode::LinearDodge, 2.0, 0.5), 2.5));
    assert!(close(blend_channel_hdr(BlendMode::LinearDodge, 0.75, 0.5), 1.25));
    assert!(close(blend_channel_hdr(BlendMode::Divide, 2.0, 0.5), 4.0));
    assert!(close(blend_channel_hdr(BlendMode::Divide, 0.5, 0.25), 2.0));
    assert_eq!(blend_rgb_hdr(BlendMode::LinearDodge, [2.0, 0.5, 4.0], [0.5, 0.25, 0.5]), [2.5, 0.75, 4.5]);
    assert_eq!(blend_rgb_hdr(BlendMode::Divide, [2.0, 0.5, 4.0], [0.5, 0.25, 0.5]), [4.0, 2.0, 8.0]);
    // Divide by zero keeps its guard; overflow stays finite.
    assert_eq!(blend_channel_hdr(BlendMode::Divide, 0.0, 0.0), 0.0);
    assert_eq!(blend_channel_hdr(BlendMode::Divide, 3.0, 0.0), 1.0);
    assert_eq!(blend_channel_hdr(BlendMode::Divide, 3.0, -1.0), 1.0);
    assert_eq!(blend_channel_hdr(BlendMode::Divide, f32::MAX, 1e-30), f32::MAX);
    assert_eq!(blend_channel_hdr(BlendMode::LinearDodge, f32::MAX, f32::MAX), f32::MAX);
    assert!(blend_channel_hdr(BlendMode::LinearDodge, f32::INFINITY, 1.0).is_finite());
    // Every other separable mode is unchanged (the non-separable ones keep values above 1:
    // blend_rgb_within), and the in-range results agree.
    for m in BlendMode::LAYER_MODES.into_iter().filter(|m| m.is_separable() && !matches!(m, BlendMode::LinearDodge | BlendMode::Divide)) {
        for (cb, cs) in [(0.2, 0.3), (0.9, 0.05), (2.0, 0.5), (0.4, 1.5)] {
            assert_eq!(blend_rgb(m, [cb, cs, 0.5], [cs, cb, 0.25]), blend_rgb_hdr(m, [cb, cs, 0.5], [cs, cb, 0.25]), "{m:?} cb={cb} cs={cs}");
        }
    }
    let (cb, cs) = ([0.2, 0.3, 0.1], [0.3, 0.4, 0.4]);
    assert_eq!(blend_rgb_hdr(BlendMode::LinearDodge, cb, cs), blend_rgb(BlendMode::LinearDodge, cb, cs));
    assert_eq!(blend_rgb_hdr(BlendMode::Divide, cb, cs), blend_rgb(BlendMode::Divide, cb, cs));
}

#[test]
fn luminosity_and_color_swap_components() {
    let cb = [0.8, 0.2, 0.2];
    let cs = [0.1, 0.1, 0.9];
    let l = blend_rgb(BlendMode::Luminosity, cb, cs);
    assert!(close(lum(l), lum(cs)));
    let c = blend_rgb(BlendMode::Color, cb, cs);
    assert!(close(lum(c), lum(cb)));
    let h = blend_rgb(BlendMode::Hue, cb, cs);
    assert!(close(lum(h), lum(cb)));
    let s = blend_rgb(BlendMode::Saturation, cb, [0.5, 0.5, 0.5]);
    // zero-saturation source desaturates the backdrop
    assert!(close(s[0], s[1]) && close(s[1], s[2]));
}

/// ClipColor exactly as ISO 32000 / W3C write it (what `clip_color` was before it guarded
/// out-of-range luminosities).
fn spec_clip_color(c: [f32; 3]) -> [f32; 3] {
    let l = lum(c);
    let n = c[0].min(c[1]).min(c[2]);
    let x = c[0].max(c[1]).max(c[2]);
    let mut out = c;
    if n < 0.0 {
        let d = l - n;
        for v in &mut out {
            *v = if d.abs() < 1e-12 { l } else { l + (*v - l) * l / d };
        }
    }
    if x > 1.0 {
        let d = x - l;
        for v in &mut out {
            *v = if d.abs() < 1e-12 { l } else { l + (*v - l) * (1.0 - l) / d };
        }
    }
    out
}

fn spec_blend_rgb(mode: BlendMode, cb: [f32; 3], cs: [f32; 3]) -> [f32; 3] {
    let set_lum = |c: [f32; 3], l: f32| {
        let d = l - lum(c);
        spec_clip_color([c[0] + d, c[1] + d, c[2] + d])
    };
    match mode {
        BlendMode::Hue => set_lum(set_sat(cs, sat(cb)), lum(cb)),
        BlendMode::Saturation => set_lum(set_sat(cb, sat(cs)), lum(cb)),
        BlendMode::Color => set_lum(cs, lum(cb)),
        BlendMode::Luminosity => set_lum(cb, lum(cs)),
        m => blend_rgb(m, cb, cs),
    }
}

const NON_SEPARABLE: [BlendMode; 4] = [BlendMode::Hue, BlendMode::Saturation, BlendMode::Color, BlendMode::Luminosity];

#[test]
fn non_separable_modes_unchanged_on_integer_inputs() {
    // 8/16-bit documents only hold 0..1, where the luminosity stays in range (up to a
    // rounding step at white) and ClipColor is the plain ISO 32000 formula.
    let mut edges: Vec<f32> = vec![0.0, 1.0 / 255.0, 0.5, 254.0 / 255.0, 65534.0 / 65535.0, 1.0];
    edges.extend((0..=8).map(|i| i as f32 / 8.0));
    let mut colors = Vec::new();
    for &r in &edges {
        for &g in &edges {
            for &b in &edges {
                colors.push([r, g, b]);
            }
        }
    }
    let mut seed = 0x1234_5678u32;
    let mut next = |levels: u32| {
        seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        ((seed >> 8) % (levels + 1)) as f32 / levels as f32
    };
    for _ in 0..2000 {
        let levels = if next(1) > 0.5 { 255 } else { 65535 };
        colors.push([next(levels), next(levels), next(levels)]);
    }
    let mut worst = 0.0f32;
    for (i, &cb) in colors.iter().enumerate() {
        for &cs in colors.iter().skip(i % 7).step_by(7) {
            for m in NON_SEPARABLE {
                let (got, want) = (blend_rgb(m, cb, cs), spec_blend_rgb(m, cb, cs));
                for k in 0..3 {
                    worst = worst.max((got[k] - want[k]).abs());
                }
            }
        }
    }
    // Only white-ish colours whose luminosity rounds a step above 1 can differ, by a rounding step.
    assert!(worst <= 1e-6, "{worst}");
}

#[test]
fn non_separable_modes_stable_when_luminosity_is_out_of_range() {
    // The spec formula divided by `max − lum`, which is rounding noise for a grey above 1 and
    // turns colours inside out when the luminosity is above 1: Luminosity of a grey of 2 over
    // (0.5, 0.25, 0.125) gave (1.0, 2.32, 2.99).
    let spec = spec_blend_rgb(BlendMode::Luminosity, [0.5, 0.25, 0.125], [2.0; 3]);
    assert!(spec[0] < spec[2], "{spec:?}");
    let colors: [[f32; 3]; 10] = [
        [1.0, 1.0, 1.0],
        [1.5, 1.5, 1.5],
        [2.0, 2.0, 2.0],
        [4.0, 4.0, 4.0],
        [1000.0, 1000.0, 1000.0],
        [2.0, 2.0, 1.999_999],
        [4.0, 3.999_9, 4.0],
        [3.0, 0.25, 0.125],
        [1.25, 0.875, 3.75],
        [-0.25, 0.5, 1.375],
    ];
    for top in [1.0, f32::MAX] {
        for m in NON_SEPARABLE {
            for cb in colors {
                for cs in colors {
                    let r = blend_rgb_within(m, cb, cs, top);
                    assert!(r.iter().all(|v| v.is_finite()), "{m:?} {cb:?} {cs:?} {top}: {r:?}");
                    // The target luminosity is kept, at any depth.
                    let want = if m == BlendMode::Luminosity { lum(cs) } else { lum(cb) };
                    if want >= 0.0 {
                        assert!((lum(r) - want).abs() <= 1e-4 * want.max(1.0), "{m:?} {cb:?} {cs:?} {top}: {r:?}");
                    }
                    let (lo, hi) = (r[0].min(r[1]).min(r[2]), r[0].max(r[1]).max(r[2]));
                    if top == 1.0 {
                        // In gamut, or (a luminosity above 1 has no in-gamut colour) its grey.
                        assert!((lo >= 0.0 && hi <= 1.0 + E) || hi - lo <= 1e-6 * hi, "{m:?} {cb:?} {cs:?}: {r:?}");
                    } else {
                        assert!(lo >= -1e-6 * hi.max(1.0), "{m:?} {cb:?} {cs:?}: {r:?}");
                    }
                }
            }
        }
    }
    // 32-bit: a grey backdrop gives a grey of its own luminosity in Hue and Saturation, a
    // grey source a grey in Color, and bright colours keep their values above 1.
    for g in [1.5f32, 2.0, 4.0, 1000.0] {
        let grey = [g; 3];
        for cs in colors {
            for m in [BlendMode::Hue, BlendMode::Saturation] {
                let r = blend_rgb_within(m, grey, cs, f32::MAX);
                assert!(r.iter().all(|&v| (v - lum(grey)).abs() <= 1e-5 * g), "{m:?} {g} {cs:?}: {r:?}");
            }
        }
        let r = blend_rgb_within(BlendMode::Luminosity, [0.5, 0.25, 0.125], grey, f32::MAX);
        assert!(r[0] > 1.0 && r[0] > r[1] && r[1] > r[2], "{g}: {r:?}");
        let r = blend_rgb_within(BlendMode::Color, grey, [3.0, 0.25, 0.125], f32::MAX);
        assert!(r[0] > 1.0 && r[0] > r[1] && r[1] > r[2], "{g}: {r:?}");
        // Integer documents: no colour of that luminosity fits in 0..1, so it is its grey.
        let r = blend_rgb_within(BlendMode::Luminosity, [0.5, 0.25, 0.125], grey, 1.0);
        assert!(r.iter().all(|&v| (v - lum(grey)).abs() <= 1e-5 * g), "{g}: {r:?}");
    }
    // Continuous across near-greys: the result barely moves from the grey's.
    for m in NON_SEPARABLE {
        let a = blend_rgb_within(m, [2.0, 2.0, 2.0], [4.0, 4.0, 4.0], f32::MAX);
        let b = blend_rgb_within(m, [2.0, 2.0, 1.999_999], [4.0, 3.999_999, 4.0], f32::MAX);
        assert!(a.iter().zip(b).all(|(x, y)| (x - y).abs() <= 1e-4), "{m:?}: {a:?} {b:?}");
    }
}

#[test]
fn darker_lighter_color_pick_whole_pixel() {
    let a = [0.9, 0.1, 0.1];
    let b = [0.2, 0.6, 0.2];
    assert_eq!(blend_rgb(BlendMode::DarkerColor, a, b), if lum(b) < lum(a) { b } else { a });
    assert_eq!(blend_rgb(BlendMode::LighterColor, a, b), if lum(b) > lum(a) { b } else { a });
}

#[test]
fn composite_alpha_rules() {
    let bg = [0.2, 0.4, 0.6, 1.0];
    // Transparent source leaves backdrop.
    assert_eq!(composite(BlendMode::Multiply, bg, [1.0, 0.0, 0.0, 0.0], 1.0), bg);
    // Opaque normal source replaces.
    let r = composite(BlendMode::Normal, bg, [1.0, 0.0, 0.0, 1.0], 1.0);
    assert!(close(r[0], 1.0) && close(r[3], 1.0));
    // Half opacity normal = lerp.
    let r = composite(BlendMode::Normal, bg, [1.0, 1.0, 1.0, 1.0], 0.5);
    assert!(close(r[0], 0.6) && close(r[1], 0.7) && close(r[2], 0.8));
    // Blend over transparent backdrop yields source regardless of mode.
    let r = composite(BlendMode::Multiply, [0.0; 4], [0.3, 0.5, 0.7, 1.0], 1.0);
    assert!(close(r[0], 0.3) && close(r[1], 0.5) && close(r[2], 0.7) && close(r[3], 1.0));
    // Alpha accumulates.
    let r = composite(BlendMode::Screen, [0.0, 0.0, 0.0, 0.5], [0.0, 0.0, 0.0, 0.5], 1.0);
    assert!(close(r[3], 0.75));
}

#[test]
fn composite_atop_keeps_alpha_and_moves_colour_by_source_alpha() {
    let bg = [1.0, 1.0, 1.0, 0.5];
    // Half-transparent white, black at 50%: halfway, alpha kept (#5783).
    let r = composite_atop(BlendMode::Normal, bg, [0.0, 0.0, 0.0, 1.0], 0.5);
    assert!(close(r[0], 0.5) && close(r[2], 0.5) && r[3] == 0.5, "{r:?}");
    // Non-Normal modes blend against the backdrop as it is: Multiply by black at 50% halves it.
    let r = composite_atop(BlendMode::Multiply, bg, [0.0, 0.0, 0.0, 1.0], 0.5);
    assert!(close(r[0], 0.5) && r[3] == 0.5, "{r:?}");
    // Opaque backdrops agree with source-over.
    let opaque = [0.2, 0.4, 0.6, 1.0];
    assert_eq!(composite_atop(BlendMode::Screen, opaque, [0.5, 0.5, 0.5, 0.7], 0.8), composite(BlendMode::Screen, opaque, [0.5, 0.5, 0.5, 0.7], 0.8));
    // A transparent source leaves the backdrop.
    assert_eq!(composite_atop(BlendMode::Normal, bg, [0.0, 0.0, 0.0, 0.0], 1.0), bg);
}
