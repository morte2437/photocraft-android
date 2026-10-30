//! Colour and palette: Oklab, edge-keeping smoothing, and the picture's own palette.

use super::Px;
use super::util::transpose;
use crate::par;
use crate::quantize::{kmeans, median_cut};

/// Oklab (Björn Ottosson, 2020) of an sRGB colour.
pub(super) fn oklab(p: Px) -> Px {
    let lin = |v: f32| {
        let v = v.clamp(0.0, 1.0);
        if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
    };
    let (r, g, b) = (lin(p[0]), lin(p[1]), lin(p[2]));
    let l = (0.412_221_46 * r + 0.536_332_55 * g + 0.051_445_995 * b).cbrt();
    let m = (0.211_903_5 * r + 0.680_699_5 * g + 0.107_396_96 * b).cbrt();
    let s = (0.088_302_46 * r + 0.281_718_85 * g + 0.629_978_7 * b).cbrt();
    [
        0.210_454_26 * l + 0.793_617_8 * m - 0.004_072_047 * s,
        1.977_998_5 * l - 2.428_592_2 * m + 0.450_593_7 * s,
        0.025_904_037 * l + 0.782_771_77 * m - 0.808_675_77 * s,
    ]
}

/// sRGB of an Oklab colour, or `None` outside the sRGB gamut.
pub(super) fn srgb(lab: Px) -> Option<Px> {
    let l = (lab[0] + 0.396_337_78 * lab[1] + 0.215_803_76 * lab[2]).powi(3);
    let m = (lab[0] - 0.105_561_346 * lab[1] - 0.063_854_17 * lab[2]).powi(3);
    let s = (lab[0] - 0.089_484_18 * lab[1] - 1.291_485_5 * lab[2]).powi(3);
    let lin = [
        4.076_741_7 * l - 3.307_711_6 * m + 0.230_969_94 * s,
        -1.268_438 * l + 2.609_757_4 * m - 0.341_319_38 * s,
        -0.004_196_086_3 * l - 0.703_418_6 * m + 1.707_614_7 * s,
    ];
    if lin.iter().any(|v| !(-1e-4..=1.0001).contains(v)) {
        return None;
    }
    let enc = |v: f32| {
        let v = v.clamp(0.0, 1.0);
        if v <= 0.003_130_8 { v * 12.92 } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 }
    };
    Some(lin.map(enc))
}

/// sRGB at Oklab lightness `l`, hue/strength `a`, `b`; out of gamut it loses strength, not hue.
pub(super) fn in_gamut(l: f32, a: f32, b: f32) -> Px {
    if let Some(c) = srgb([l, a, b]) {
        return c;
    }
    let (mut ok, mut bad) = (0.0f32, 1.0f32);
    for _ in 0..10 {
        let t = (ok + bad) / 2.0;
        if srgb([l, a * t, b * t]).is_some() { ok = t } else { bad = t }
    }
    srgb([l, a * ok, b * ok]).unwrap_or([l, l, l])
}

/// Squared distance between two colours.
pub(super) fn dist2(a: Px, b: Px) -> f32 {
    (a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)
}

/// Bilateral smoothing in Oklab, `rounds` times along rows then columns.
pub(super) fn smooth(mut lab: Vec<Px>, w: usize, h: usize, rounds: usize) -> Vec<Px> {
    for _ in 0..rounds {
        bilateral_rows(&mut lab, w);
        let mut t = transpose(&lab, w, h);
        bilateral_rows(&mut t, h);
        lab = transpose(&t, h, w);
    }
    lab
}

/// One bilateral pass along every row of `buf` (rows of `w` pixels), seven pixels wide.
fn bilateral_rows(buf: &mut [Px], w: usize) {
    const R: isize = 3;
    // Colours this far apart (Oklab) count half as much as equal ones (a Cauchy weight: as
    // edge-keeping as a Gaussian one and much cheaper).
    const HALF: f32 = 0.06;
    let k = 1.0 / (HALF * HALF);
    let spatial: [f32; 7] = std::array::from_fn(|i| (-((i as f32 - 3.0).powi(2)) / 8.0).exp());
    par::chunks_mut(buf, w, true, |_, row| {
        let src = row.to_vec();
        for (x, out) in row.iter_mut().enumerate() {
            let c = src[x];
            let (mut acc, mut wsum) = ([0.0f32; 3], 0.0f32);
            for d in -R..=R {
                let q = src[(x as isize + d).clamp(0, w as isize - 1) as usize];
                let wt = spatial[(d + R) as usize] / (1.0 + dist2(q, c) * k);
                acc = [acc[0] + q[0] * wt, acc[1] + q[1] * wt, acc[2] + q[2] * wt];
                wsum += wt;
            }
            *out = [acc[0] / wsum, acc[1] / wsum, acc[2] / wsum];
        }
    });
}

/// At most `levels` colours of the image's own; colours closer than noise are merged.
pub(super) fn palette(lab: &[Px], levels: usize) -> Vec<Px> {
    const SAMPLES: usize = 1 << 16;
    const SAME: f32 = 0.045;
    // Hue centres (Oklab a, b) this far apart are different colours, not one colour's spread.
    const HUES_APART: f32 = 0.08;
    let step = lab.len().div_ceil(SAMPLES).max(1);
    let pts: Vec<(Px, u32)> = lab.iter().step_by(step).map(|p| (*p, 1)).collect();
    let mut centres = median_cut(&pts, levels);
    kmeans(&pts, &mut centres, 10);
    // Number of Levels counts tones: a palette colour whose pixels are two clearly different hues
    // of that tone (a blue and a pink) splits in two, or touching areas of both would be one
    // shape of their grey average.
    let mut split = Vec::with_capacity(centres.len() * 2);
    for (k, c) in centres.iter().enumerate() {
        let members: Vec<(Px, u32)> = pts.iter().filter(|(p, _)| nearest(&centres, *p) as usize == k).map(|(p, _)| ([0.0, p[1], p[2]], 1)).collect();
        let mut hues = median_cut(&members, 2);
        kmeans(&members, &mut hues, 6);
        match hues.as_slice() {
            [a, b] if dist2(*a, *b).sqrt() > HUES_APART => split.extend([[c[0], a[1], a[2]], [c[0], b[1], b[2]]]),
            _ => split.push(*c),
        }
    }
    let mut centres = split;
    kmeans(&pts, &mut centres, 4);
    let mut weight = vec![0u32; centres.len()];
    for (p, _) in &pts {
        if let Some(wt) = weight.get_mut(nearest(&centres, *p) as usize) {
            *wt += 1;
        }
    }
    let mut cs: Vec<(Px, f32)> = centres.into_iter().zip(weight).filter(|c| c.1 > 0).map(|(c, wt)| (c, wt as f32)).collect();
    loop {
        let mut best: Option<(usize, usize, f32)> = None;
        for i in 0..cs.len() {
            for j in i + 1..cs.len() {
                let d = dist2(cs[i].0, cs[j].0).sqrt();
                if d < SAME && best.is_none_or(|b| d < b.2) {
                    best = Some((i, j, d));
                }
            }
        }
        let Some((i, j, _)) = best else { break };
        let (a, b) = (cs[i], cs[j]);
        let t = a.1 + b.1;
        cs[i] = (std::array::from_fn(|k| (a.0[k] * a.1 + b.0[k] * b.1) / t), t);
        cs.remove(j);
    }
    if cs.is_empty() {
        return vec![[0.5, 0.0, 0.0]];
    }
    cs.into_iter().map(|c| c.0).collect()
}

/// Index of the palette colour closest to `p`.
pub(super) fn nearest(palette: &[Px], p: Px) -> u8 {
    let mut best = (0usize, f32::MAX);
    for (i, c) in palette.iter().enumerate() {
        let d = dist2(*c, p);
        if d < best.1 {
            best = (i, d);
        }
    }
    best.0 as u8
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::photo_util::hash01;

    /// Oklab pixels of `w`×`h`: `colour(x)` plus noise of ±`noise` in each channel.
    fn noisy(w: usize, h: usize, noise: f32, colour: impl Fn(usize) -> Px) -> Vec<Px> {
        (0..w * h)
            .map(|i| {
                let (x, y) = ((i % w) as i64, (i / w) as i64);
                let c = colour(i % w);
                std::array::from_fn(|k| c[k] + (hash01(x, y, k as u64 + 1) - 0.5) * 2.0 * noise)
            })
            .collect()
    }

    #[test]
    fn oklab_round_trips_and_marks_the_gamut() {
        let white = oklab([1.0, 1.0, 1.0]);
        assert!((white[0] - 1.0).abs() < 1e-3 && white[1].abs() < 1e-3 && white[2].abs() < 1e-3, "{white:?}");
        assert!(oklab([0.0, 0.0, 0.0])[0].abs() < 1e-6);
        for p in [[0.2, 0.5, 0.9], [0.95, 0.1, 0.3], [0.5, 0.5, 0.5], [0.0, 1.0, 0.0]] {
            let back = srgb(oklab(p)).unwrap_or([-1.0; 3]);
            assert!(dist2(back, p) < 1e-6, "{p:?} came back as {back:?}");
        }
        // Very light and very colourful: no sRGB colour is that.
        assert_eq!(srgb([0.95, 0.3, 0.0]), None);
    }

    #[test]
    fn out_of_gamut_colours_lose_strength_not_hue() {
        let (l, a, b) = (0.95, 0.3, 0.1);
        let c = in_gamut(l, a, b);
        let lab = oklab(c);
        assert!((lab[0] - l).abs() < 0.01, "lightness kept: {lab:?}");
        let chroma = (lab[1] * lab[1] + lab[2] * lab[2]).sqrt();
        assert!(chroma > 0.01 && chroma < (a * a + b * b).sqrt(), "weaker but still coloured: {chroma}");
        assert!((lab[2] / lab[1] - b / a).abs() < 0.05, "same hue: {lab:?}");
        // In gamut it is just the conversion.
        assert_eq!(in_gamut(0.6, 0.02, -0.03), srgb([0.6, 0.02, -0.03]).unwrap_or_default());
    }

    #[test]
    fn smoothing_flattens_noise_and_keeps_edges() {
        let (w, h) = (40usize, 20usize);
        let lab = noisy(w, h, 0.02, |x| if x < 20 { [0.3, 0.0, 0.0] } else { [0.8, 0.0, 0.0] });
        let out = smooth(lab.clone(), w, h, 2);
        let spread = |v: &[Px], xs: std::ops::Range<usize>| {
            let l: Vec<f32> = (0..h).flat_map(|y| xs.clone().map(move |x| y * w + x)).map(|i| v[i][0]).collect();
            let mean = l.iter().sum::<f32>() / l.len() as f32;
            (l.iter().map(|v| (v - mean).powi(2)).sum::<f32>() / l.len() as f32).sqrt()
        };
        assert!(spread(&out, 2..16) < spread(&lab, 2..16) / 2.0, "noise kept: {} vs {}", spread(&out, 2..16), spread(&lab, 2..16));
        // The step stays a step: the pixels either side of it keep most of their difference.
        let step = (0..h).map(|y| out[y * w + 20][0] - out[y * w + 19][0]).sum::<f32>() / h as f32;
        assert!(step > 0.4, "edge blurred to {step}");
    }

    #[test]
    fn the_palette_finds_the_pictures_own_colours() {
        let (w, h) = (60usize, 20usize);
        let (dark, light) = ([0.25, 0.02, -0.03], [0.75, -0.04, 0.05]);
        let lab = noisy(w, h, 0.01, |x| if x < 30 { dark } else { light });
        // Asked for up to six, it gives two: colours only noise apart are one.
        let p = palette(&lab, 6);
        assert_eq!(p.len(), 2, "{p:?}");
        for c in [dark, light] {
            assert!(p.iter().any(|q| dist2(*q, c).sqrt() < 0.02), "{c:?} not in {p:?}");
        }
        assert_eq!(nearest(&p, dark), nearest(&p, [0.27, 0.0, -0.02]));
        assert_ne!(nearest(&p, dark), nearest(&p, light));
    }

    #[test]
    fn a_tone_of_two_different_hues_splits_in_two() {
        // A dark tone and a light one, the light one half blue and half pink: two levels give
        // three colours, or the blue and pink areas would be one shape of their grey average.
        let (w, h) = (60usize, 20usize);
        let (dark, blue, pink) = ([0.2, 0.0, 0.0], [0.7, -0.03, -0.1], [0.7, 0.12, 0.0]);
        let lab = noisy(w, h, 0.005, |x| {
            if x < 30 {
                dark
            } else if x < 45 {
                blue
            } else {
                pink
            }
        });
        let p = palette(&lab, 2);
        assert_eq!(p.len(), 3, "{p:?}");
        for c in [dark, blue, pink] {
            assert!(p.iter().any(|q| dist2(*q, c).sqrt() < 0.03), "{c:?} not in {p:?}");
        }
    }

    #[test]
    fn an_empty_image_still_has_a_palette() {
        assert_eq!(palette(&[], 4).len(), 1);
    }
}
