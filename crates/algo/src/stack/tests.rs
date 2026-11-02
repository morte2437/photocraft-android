use super::*;

fn px(v: f32) -> Vec<[f32; 4]> {
    vec![[v, v, v, 1.0]]
}

#[test]
fn basic_statistics() {
    let layers = vec![px(0.1), px(0.9), px(0.2), px(0.3), px(0.1)];
    let at = |s| combine(&layers, s)[0][0];
    assert!((at(Stat::Mean) - 0.32).abs() < 1e-6);
    assert!((at(Stat::Median) - 0.2).abs() < 1e-6);
    assert_eq!(at(Stat::Minimum), 0.1);
    assert_eq!(at(Stat::Maximum), 0.9);
    assert!((at(Stat::Range) - 0.8).abs() < 1e-6);
    assert!((at(Stat::Summation) - 1.6).abs() < 1e-6);
    let var = at(Stat::Variance);
    assert!((at(Stat::StandardDeviation) - var.sqrt()).abs() < 1e-6);
    assert!(at(Stat::Skewness) > 0.5, "right-skewed");
    assert!(at(Stat::Entropy) > 0.0 && at(Stat::Entropy) <= 1.0);
    assert!(at(Stat::Kurtosis) > 0.0);
}

#[test]
fn statistics_preserve_hdr_and_signed_samples() {
    let layers = vec![px(-2.0), px(4.0)];
    for (mode, expected) in [
        (Stat::Mean, 1.0),
        (Stat::Median, 1.0),
        (Stat::Minimum, -2.0),
        (Stat::Maximum, 4.0),
        (Stat::Range, 6.0),
        (Stat::Summation, 2.0),
        (Stat::Variance, 9.0),
        (Stat::StandardDeviation, 3.0),
    ] {
        assert_eq!(combine(&layers, mode)[0], [expected, expected, expected, 1.0], "{mode:?}");
    }
    for mode in [Stat::Mean, Stat::Median] {
        assert_eq!(combine(&[px(2.0), px(4.0)], mode)[0], [3.0, 3.0, 3.0, 1.0]);
        assert_eq!(combine(&[px(-2.0), px(-4.0)], mode)[0], [-3.0, -3.0, -3.0, 1.0]);
    }
    // These display-oriented statistics still apply their own normalisation.
    for mode in [Stat::Entropy, Stat::Skewness, Stat::Kurtosis] {
        assert!((0.0..=1.0).contains(&combine(&layers, mode)[0][0]), "{mode:?}");
    }
}

#[test]
fn transparent_samples_are_ignored() {
    let layers = vec![vec![[0.5, 0.5, 0.5, 1.0]], vec![[1.0, 1.0, 1.0, 0.0]]];
    assert_eq!(combine(&layers, Stat::Mean)[0], [0.5, 0.5, 0.5, 1.0]);
    let empty = vec![vec![[1.0, 1.0, 1.0, 0.0]]];
    assert_eq!(combine(&empty, Stat::Mean)[0][3], 0.0);
}

#[test]
fn median_removes_outlier() {
    let layers = vec![px(0.5), px(0.5), px(1.0)];
    assert_eq!(combine(&layers, Stat::Median)[0][0], 0.5);
}

fn sorted_median(v: &mut [f32]) -> f32 {
    v.sort_unstable_by(f32::total_cmp);
    let k = v.len();
    if k == 0 {
        0.0
    } else if k % 2 == 1 {
        v[k / 2]
    } else {
        (v[k / 2 - 1] + v[k / 2]) / 2.0
    }
}

fn assert_median_matches_sort(mut values: Vec<f32>) {
    let expected = sorted_median(&mut values.clone());
    let actual = stat(&mut values, Stat::Median, &mut EntropyScratch::new());
    if expected.is_nan() {
        // IEEE arithmetic does not specify which NaN payload an even median propagates.
        assert!(actual.is_nan());
    } else {
        assert_eq!(actual.to_bits(), expected.to_bits(), "{values:?}");
    }
}

#[test]
fn median_selection_matches_sort_for_varied_stack_sizes_and_orders() {
    for len in 0..=257 {
        let ascending: Vec<f32> = (0..len).map(|i| i as f32 / 16.0 - 8.0).collect();
        assert_median_matches_sort(ascending.clone());
        assert_median_matches_sort(ascending.into_iter().rev().collect());
        assert_median_matches_sort(vec![0.25; len]);
        assert_median_matches_sort((0..len).map(|i| ((i * 31) % 7) as f32 - 3.0).collect());
        // Deterministic shuffled samples, including float bit patterns outside 0..=1.
        let mut seed = 0x9e37_79b9u32;
        assert_median_matches_sort(
            (0..len)
                .map(|_| {
                    seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                    f32::from_bits(seed)
                })
                .collect(),
        );
    }
}

#[test]
fn median_selection_preserves_total_float_order() {
    let special = [f32::from_bits(0xffc0_0001), f32::NEG_INFINITY, -f32::MAX, -1.0, -0.0, 0.0, 1.0, f32::MAX, f32::INFINITY, f32::from_bits(0x7fc0_0001)];
    for &a in &special {
        // An odd median returns the selected NaN's bits without doing arithmetic.
        assert_eq!(stat(&mut [a], Stat::Median, &mut EntropyScratch::new()).to_bits(), a.to_bits());
        for &b in &special {
            assert_median_matches_sort(vec![a, b]);
            for &c in &special {
                let mut values = [a, b, c];
                let expected = sorted_median(&mut values.clone());
                assert_eq!(stat(&mut values, Stat::Median, &mut EntropyScratch::new()).to_bits(), expected.to_bits());
            }
        }
    }
}

#[test]
fn deep_median_stack_uses_only_visible_samples_per_channel() {
    for count in [32, 33, 128, 129] {
        let layers: Vec<Vec<[f32; 4]>> = (0..count)
            .map(|i| {
                let pixel = [((i * 31) % 137) as f32, -(i as f32), (i % 5) as f32, if i % 3 == 0 { 0.0 } else { 0.5 }];
                if i % 7 == 0 { vec![pixel] } else { vec![pixel, [0.0; 4]] }
            })
            .collect();
        let actual = combine(&layers, Stat::Median);
        for channel in 0..3 {
            let mut samples: Vec<f32> = layers.iter().filter(|l| l[0][3] > 0.0).map(|l| l[0][channel]).collect();
            assert_eq!(actual[0][channel].to_bits(), sorted_median(&mut samples).to_bits());
        }
        assert_eq!(actual[0][3], 0.5);
        assert_eq!(actual[1], [0.0; 4]);
    }
}

/// Five layers over six pixels (the last layer is short). Covers opaque, partly transparent,
/// fully transparent, HDR / negative, single-sample and tied-value pixels.
fn fixture() -> Vec<Vec<[f32; 4]>> {
    vec![
        vec![[0.10, 0.20, 0.30, 1.0], [0.5, 0.5, 0.5, 1.0], [0.9, 0.9, 0.9, 0.0], [-2.0, 4.0, 0.0, 1.0], [0.7, 0.2, 0.1, 0.5], [0.25, 0.25, 0.25, 1.0]],
        vec![[0.90, 0.60, 0.30, 1.0], [0.0, 0.0, 0.0, 0.0], [0.8, 0.8, 0.8, 0.0], [3.0, -1.0, 0.5, 0.75], [0.0, 0.0, 0.0, 0.0], [0.25, 0.75, 0.25, 1.0]],
        vec![[0.20, 0.90, 0.30, 0.5], [0.1, 0.2, 0.3, 1.0], [0.7, 0.7, 0.7, 0.0], [0.5, 0.5, 0.5, 1.0], [0.0, 0.0, 0.0, 0.0], [0.5, 0.75, 0.25, 0.25]],
        vec![[0.30, 0.05, 0.30, 0.25], [0.3, 0.4, 0.5, 0.5], [0.6, 0.6, 0.6, 0.0], [1.5, 1.5, 1.5, 1.0], [0.0, 0.0, 0.0, 0.0], [0.25, 0.75, 0.5, 1.0]],
        vec![[0.10, 0.70, 0.99, 1.0], [0.9, 0.1, 0.2, 1.0], [0.5, 0.5, 0.5, 0.0], [0.0, 0.0, 0.0, 1.0]],
    ]
}

fn bits(px: &[[f32; 4]]) -> Vec<[u32; 4]> {
    px.iter().map(|p| p.map(f32::to_bits)).collect()
}

/// Every mode except Entropy is bit-for-bit what the old serial loop produced. The expected
/// bit patterns were recorded by running the unmodified `combine` on [`fixture`].
#[test]
fn statistics_are_bit_identical_to_the_serial_implementation() {
    let layers = fixture();
    let expected: Vec<(Stat, Vec<[u32; 4]>)> = vec![
        (
            Stat::Kurtosis,
            [
                [0x3e96956b, 0x3e182e3f, 0x3ea66664, 0x3f800000],
                [0x3e3d004c, 0x3e0b4396, 0x3e030e1f, 0x3f800000],
                [0x00000000, 0x00000000, 0x00000000, 0x00000000],
                [0x3e594eb2, 0x3e6ef420, 0x3e7ffffe, 0x3f800000],
                [0x00000000, 0x00000000, 0x00000000, 0x3f000000],
                [0x3e6eeeee, 0x3e6eeeee, 0x3e6eeeee, 0x3f800000],
            ]
            .to_vec(),
        ),
        (
            Stat::Maximum,
            [
                [0x3f666666, 0x3f666666, 0x3f7d70a4, 0x3f800000],
                [0x3f666666, 0x3f000000, 0x3f000000, 0x3f800000],
                [0x00000000, 0x00000000, 0x00000000, 0x00000000],
                [0x40400000, 0x40800000, 0x3fc00000, 0x3f800000],
                [0x3f333333, 0x3e4ccccd, 0x3dcccccd, 0x3f000000],
                [0x3f000000, 0x3f400000, 0x3f000000, 0x3f800000],
            ]
            .to_vec(),
        ),
        (
            Stat::Mean,
            [
                [0x3ea3d70a, 0x3efae148, 0x3ee0418a, 0x3f800000],
                [0x3ee66666, 0x3e99999a, 0x3ec00000, 0x3f800000],
                [0x00000000, 0x00000000, 0x00000000, 0x00000000],
                [0x3f19999a, 0x3f800000, 0x3f000000, 0x3f800000],
                [0x3f333333, 0x3e4ccccd, 0x3dcccccd, 0x3f000000],
                [0x3ea00000, 0x3f200000, 0x3ea00000, 0x3f800000],
            ]
            .to_vec(),
        ),
        (
            Stat::Median,
            [
                [0x3e4ccccd, 0x3f19999a, 0x3e99999a, 0x3f800000],
                [0x3ecccccd, 0x3e99999a, 0x3ecccccd, 0x3f800000],
                [0x00000000, 0x00000000, 0x00000000, 0x00000000],
                [0x3f000000, 0x3f000000, 0x3f000000, 0x3f800000],
                [0x3f333333, 0x3e4ccccd, 0x3dcccccd, 0x3f000000],
                [0x3e800000, 0x3f400000, 0x3e800000, 0x3f800000],
            ]
            .to_vec(),
        ),
        (
            Stat::Minimum,
            [
                [0x3dcccccd, 0x3d4ccccd, 0x3e99999a, 0x3f800000],
                [0x3dcccccd, 0x3dcccccd, 0x3e4ccccd, 0x3f800000],
                [0x00000000, 0x00000000, 0x00000000, 0x00000000],
                [0xc0000000, 0xbf800000, 0x00000000, 0x3f800000],
                [0x3f333333, 0x3e4ccccd, 0x3dcccccd, 0x3f000000],
                [0x3e800000, 0x3e800000, 0x3e800000, 0x3f800000],
            ]
            .to_vec(),
        ),
        (
            Stat::Range,
            [
                [0x3f4ccccc, 0x3f599999, 0x3f30a3d7, 0x3f800000],
                [0x3f4ccccc, 0x3ecccccd, 0x3e99999a, 0x3f800000],
                [0x00000000, 0x00000000, 0x00000000, 0x00000000],
                [0x40a00000, 0x40a00000, 0x3fc00000, 0x3f800000],
                [0x00000000, 0x00000000, 0x00000000, 0x3f000000],
                [0x3e800000, 0x3f000000, 0x3e800000, 0x3f800000],
            ]
            .to_vec(),
        ),
        (
            Stat::Skewness,
            [
                [0x3f522026, 0x3ee7cf76, 0x3f600000, 0x3f800000],
                [0x3f1bd152, 0x3efffffe, 0x3ee4a11b, 0x3f800000],
                [0x00000000, 0x00000000, 0x00000000, 0x00000000],
                [0x3eedb65a, 0x3f2ea754, 0x3f3a6c7a, 0x3f800000],
                [0x3f000000, 0x3f000000, 0x3f000000, 0x3f000000],
                [0x3f49e69e, 0x3e58658a, 0x3f49e69e, 0x3f800000],
            ]
            .to_vec(),
        ),
        (
            Stat::StandardDeviation,
            [
                [0x3e99421e, 0x3ea23b6b, 0x3e8d4fdf, 0x3f800000],
                [0x3e97739e, 0x3e21e89b, 0x3e050581, 0x3f800000],
                [0x00000000, 0x00000000, 0x00000000, 0x00000000],
                [0x3fd3e0b1, 0x3fd9f9e5, 0x3f0c378c, 0x3f800000],
                [0x00000000, 0x00000000, 0x00000000, 0x3f000000],
                [0x3dddb3d7, 0x3e5db3d7, 0x3dddb3d7, 0x3f800000],
            ]
            .to_vec(),
        ),
        (
            Stat::Summation,
            [
                [0x3fcccccd, 0x401ccccd, 0x400c28f6, 0x3f800000],
                [0x3fe66666, 0x3f99999a, 0x3fc00000, 0x3f800000],
                [0x00000000, 0x00000000, 0x00000000, 0x00000000],
                [0x40400000, 0x40a00000, 0x40200000, 0x3f800000],
                [0x3f333333, 0x3e4ccccd, 0x3dcccccd, 0x3f000000],
                [0x3fa00000, 0x40200000, 0x3fa00000, 0x3f800000],
            ]
            .to_vec(),
        ),
        (
            Stat::Variance,
            [
                [0x3db78033, 0x3dcd9e83, 0x3d9c022a, 0x3f800000],
                [0x3db33332, 0x3ccccccd, 0x3c8a3d70, 0x3f800000],
                [0x00000000, 0x00000000, 0x00000000, 0x00000000],
                [0x402f5c29, 0x4039999a, 0x3e99999a, 0x3f800000],
                [0x00000000, 0x00000000, 0x00000000, 0x3f000000],
                [0x3c400000, 0x3d400000, 0x3c400000, 0x3f800000],
            ]
            .to_vec(),
        ),
    ];
    for (mode, want) in expected {
        assert_eq!(bits(&combine(&layers, mode)), want, "{mode:?}");
    }
}

/// Entropy against values worked out by hand (bits of the 8-bit histogram, over log2(N)).
#[test]
fn entropy_matches_hand_computed_values() {
    let out = combine(&fixture(), Stat::Entropy);
    let h = |ps: &[f64]| -> f64 {
        let n: f64 = ps.iter().sum();
        ps.iter().map(|c| -(c / n) * (c / n).log2()).sum::<f64>() / n.log2()
    };
    // Pixel 1: four distinct values -> 2 bits / log2(4) = exactly 1.
    assert_eq!(out[1][0], 1.0);
    // Pixel 2: every sample transparent -> untouched.
    assert_eq!(out[2], [0.0; 4]);
    // Pixel 3, red: -2 and 0 land in bin 0, 3 and 1.5 clamp into bin 255, 0.5 is alone.
    assert!((f64::from(out[3][0]) - h(&[2.0, 2.0, 1.0])).abs() < 1e-6, "{}", out[3][0]);
    // Pixel 4: a single contributing sample has no entropy.
    assert_eq!(out[4], [0.0, 0.0, 0.0, 0.5]);
    // Pixel 5: three samples share a bin, one is alone, in every channel.
    for (c, v) in out[5].iter().take(3).enumerate() {
        assert!((f64::from(*v) - h(&[3.0, 1.0])).abs() < 1e-6, "channel {c}: {v}");
    }
}

/// The old HashMap summed in a random order, so Entropy could differ by an ulp between runs.
/// Now the bins are summed in ascending order: repeated runs (and any thread count) agree.
#[test]
fn entropy_is_deterministic() {
    let layers: Vec<Vec<[f32; 4]>> = (0..7)
        .map(|k| (0..3 * CHUNK + 17).map(|i| [((i * 7 + k * 13) % 11) as f32 / 10.0, ((i + k) % 5) as f32 / 4.0, ((i * k) % 9) as f32 / 8.0, 1.0]).collect())
        .collect();
    let first = bits(&combine(&layers, Stat::Entropy));
    for _ in 0..8 {
        assert_eq!(bits(&combine(&layers, Stat::Entropy)), first);
    }
}

/// The pre-lookup calculation, summing the 8-bit bins in ascending order. This deliberately
/// does not call `entropy` or use its lookup/scratch, so it catches changes in rounding too.
fn entropy_reference(v: &[f32]) -> f32 {
    if v.len() < 2 {
        return 0.0;
    }
    let mut counts = [0u32; 256];
    for x in v {
        counts[(x.clamp(0.0, 1.0) * 255.0).round() as usize] += 1;
    }
    let n = v.len() as f32;
    let h: f32 = counts
        .iter()
        .filter(|&&count| count > 0)
        .map(|&count| {
            let p = count as f32 / n;
            -p * p.log2()
        })
        .sum();
    h / n.log2()
}

#[test]
fn entropy_terms_preserve_each_operation_and_bound_storage() {
    let terms = EntropyTerms::new(usize::MAX, usize::MAX);
    let max = EntropyTerms::MAX_SAMPLES;
    assert_eq!(terms.values.len(), (max + 1) * (max + 2) / 2);
    assert!(terms.values.len() * size_of::<f32>() < 130 * 1024);
    for samples in 2..=max {
        let row = terms.row(samples).unwrap();
        let n = samples as f32;
        assert_eq!(row[0].to_bits(), n.log2().to_bits());
        for (count, &term) in row.iter().enumerate().skip(1) {
            let p = count as f32 / n;
            assert_eq!(term.to_bits(), (-p * p.log2()).to_bits(), "{count}/{samples}");
        }
    }
    for samples in [0, 1, max + 1, usize::MAX] {
        assert!(terms.row(samples).is_none());
    }
    assert!(EntropyTerms::new(1, usize::MAX).values.is_empty());
    assert!(EntropyTerms::new(usize::MAX, 255).values.is_empty());
    assert!(EntropyTerms { values: Vec::new() }.row(2).is_none());
}

#[test]
fn entropy_lookup_and_fallback_match_reference_bits() {
    let terms = EntropyTerms::new(EntropyTerms::MAX_SAMPLES, CHUNK);
    let mut scratch = EntropyScratch::new();
    let special = [f32::NAN, f32::NEG_INFINITY, f32::INFINITY, -3.0, -0.0, 0.0, 0.5 / 255.0, 0.5, 1.0, 4.0];
    // Move repeatedly between counts and distributions to exercise scratch reset, duplicate
    // bins, every cached denominator and the uncached side of the table boundary.
    for samples in (2..=EntropyTerms::MAX_SAMPLES + 2).chain([513, 3, 256, 2]) {
        for pattern in 0..4 {
            let v: Vec<f32> = (0..samples)
                .map(|i| match pattern {
                    0 => 0.5,
                    1 => (i % 256) as f32 / 255.0,
                    2 => ((i * 137 + samples * 29) % 65536) as f32 / 65535.0,
                    _ => special[i % special.len()],
                })
                .collect();
            let expected = entropy_reference(&v).to_bits();
            assert_eq!(entropy(&v, &mut scratch, terms.row(samples)).to_bits(), expected, "cached: n={samples}, pattern={pattern}");
            assert_eq!(entropy(&v, &mut scratch, None).to_bits(), expected, "fallback: n={samples}, pattern={pattern}");
        }
    }
}

#[test]
fn entropy_combine_matches_reference_with_changing_contributor_counts() {
    for (count, len) in [(19, 2 * CHUNK + 17), (EntropyTerms::MAX_SAMPLES + 1, 513)] {
        let layers: Vec<Vec<[f32; 4]>> = (0..count)
            .map(|k| {
                (0..len.saturating_sub(k % 13))
                    .map(|i| {
                        [
                            ((i * 31 + k * 7) % 256) as f32 / 255.0,
                            ((i * 17 + k) % 65536) as f32 / 65535.0,
                            if (i + k).is_multiple_of(23) { f32::NAN } else { (i % 7) as f32 - 2.0 },
                            if k > i % (count + 1) { 0.0 } else { 0.5 },
                        ]
                    })
                    .collect()
            })
            .collect();
        let expected = bits(&combine_with(&layers, |v, _| entropy_reference(v)));
        assert_eq!(bits(&combine(&layers, Stat::Entropy)), expected);
        #[cfg(not(target_arch = "wasm32"))]
        for threads in [1, 3] {
            let pool = rayon::ThreadPoolBuilder::new().num_threads(threads).build().unwrap();
            assert_eq!(pool.install(|| bits(&combine(&layers, Stat::Entropy))), expected, "{threads} threads");
        }
    }
}

/// Chunking is invisible: a multi-chunk image (with a ragged last chunk and layers of unequal
/// length) gives, pixel by pixel, what combining that pixel on its own gives.
#[test]
fn chunked_result_equals_per_pixel_result() {
    let len = 2 * CHUNK + 5;
    let layer = |k: usize, n: usize| -> Vec<[f32; 4]> {
        (0..n)
            .map(|i| {
                [
                    ((i * 31 + k * 7) % 101) as f32 / 100.0,
                    ((i * 17 + k) % 53) as f32 / 52.0 - 0.2,
                    ((i + 3 * k) % 29) as f32 / 28.0,
                    if (i + k).is_multiple_of(4) { 0.0 } else { 0.5 + ((i + k) % 3) as f32 / 4.0 },
                ]
            })
            .collect()
    };
    let layers = vec![layer(1, len), layer(2, CHUNK + 3), layer(3, len - 1), layer(4, 0), layer(5, len)];
    for mode in [
        Stat::Entropy,
        Stat::Kurtosis,
        Stat::Maximum,
        Stat::Mean,
        Stat::Median,
        Stat::Minimum,
        Stat::Range,
        Stat::Skewness,
        Stat::StandardDeviation,
        Stat::Summation,
        Stat::Variance,
    ] {
        let whole = combine(&layers, mode);
        assert_eq!(whole.len(), len);
        for i in (0..len).step_by(97).chain([CHUNK - 1, CHUNK, CHUNK + 1, 2 * CHUNK - 1, 2 * CHUNK, len - 1]) {
            let one: Vec<Vec<[f32; 4]>> = layers.iter().map(|l| l.get(i).map(|p| vec![*p]).unwrap_or_default()).collect();
            assert_eq!(whole[i].map(f32::to_bits), combine(&one, mode)[0].map(f32::to_bits), "{mode:?} pixel {i}");
        }
    }
}

#[test]
fn no_layers_gives_an_empty_result() {
    assert!(combine(&[], Stat::Median).is_empty());
}

/// [`ipow`] multiplies in the order `x·x²` for the cube and `x²·x²` for the fourth power.
#[test]
fn ipow_squares_and_multiplies() {
    for x in [0.58f32, -0.12, -2.6, 2.4, -0.6, 1.0e-3, 0.0] {
        assert_eq!(ipow(x, 0).to_bits(), 1.0f32.to_bits());
        assert_eq!(ipow(x, 1).to_bits(), x.to_bits());
        assert_eq!(ipow(x, 2).to_bits(), (x * x).to_bits());
        assert_eq!(ipow(x, 3).to_bits(), (x * (x * x)).to_bits());
        assert_eq!(ipow(x, 4).to_bits(), ((x * x) * (x * x)).to_bits());
    }
}
