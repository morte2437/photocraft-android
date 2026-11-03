use super::*;

const KINDS: [&str; 15] = [
    "invert",
    "brightnessContrast",
    "threshold",
    "posterize",
    "exposure",
    "vibrance",
    "hueSaturation",
    "levels",
    "curves",
    "colorBalance",
    "blackWhite",
    "photoFilter",
    "channelMixer",
    "gradientMap",
    "selectiveColor",
];

/// A non-default parameter set per kind (RGB channel keys for Levels/Curves).
pub(crate) fn sample(kind: &str) -> Value {
    match kind {
        "brightnessContrast" => json!({"brightness": 20, "contrast": -10, "legacy": true}),
        "threshold" => json!({"level": 100}),
        "posterize" => json!({"levels": 7}),
        "exposure" => json!({"exposure": 1.5, "offset": 0.1, "gamma": 1.4}),
        "vibrance" => json!({"vibrance": 30, "saturation": -20}),
        "hueSaturation" => {
            json!({"hue": 25, "saturation": -20, "lightness": 10, "reds": {"hue": 10, "saturation": -30, "lightness": 5, "range": [300, 340, 20, 50]}})
        }
        "levels" => json!({"inBlack": 10, "inWhite": 240, "outBlack": 5, "outWhite": 250, "gamma": 1.2, "red": {"inBlack": 3, "gamma": 0.8}}),
        "curves" => json!({"points": [[0, 0], [128, 150], [255, 255]], "blue": [[0, 20], [255, 235]]}),
        "colorBalance" => json!({"shadows": [10, -5, 0], "midtones": [0, 20, -30], "highlights": [5, 5, 5], "preserveLuminosity": false}),
        "blackWhite" => json!({"reds": 120, "blues": -40, "tint": true, "tintColor": "#d0a070"}),
        "photoFilter" => json!({"color": "#3366cc", "density": 40, "preserveLuminosity": false}),
        "channelMixer" => json!({"red": [80, 30, -10, 5], "blue": [0, 10, 90, 0]}),
        "gradientMap" => json!({"stops": [[0, "#000000"], [0.5, "#ff0000"], [1, [1, 1, 1]]], "reverse": true, "dither": true}),
        "selectiveColor" => json!({"method": "absolute", "reds": [10, 0, -20, 5]}),
        _ => json!({}),
    }
}

#[test]
fn every_kind_round_trips_through_params() {
    for mode in [ColorMode::Rgb, ColorMode::Cmyk, ColorMode::Lab, ColorMode::Grayscale] {
        for kind in KINDS {
            let mut p = sample(kind);
            if matches!(kind, "levels" | "curves") && default_space(mode) != ToneSpace::Rgb {
                // Off RGB the channel keys are the document's.
                p.as_object_mut().unwrap().retain(|k, _| !matches!(k.as_str(), "red" | "blue"));
            }
            let a = from_params(kind, &p, None, mode).unwrap_or_else(|e| panic!("{kind} {mode:?}: {e}"));
            assert_ne!(Some(&a), default_for(kind, mode).ok().as_ref().filter(|_| kind != "invert"), "{kind} params ignored");
            let back = from_params(kind, &to_params(&a), None, mode).unwrap();
            assert_eq!(to_params(&back), to_params(&a), "{kind} {mode:?}");
            // A partial update keeps everything it doesn't name.
            let again = from_params(kind, &json!({}), Some(&a), mode).unwrap();
            assert_eq!(again, a, "{kind} {mode:?}");
        }
    }
}

#[test]
fn params_reach_the_model() {
    basic_params_reach_the_model();
    color_space_params_reach_the_model();
}

fn basic_params_reach_the_model() {
    let a = from_params("colorBalance", &json!({"midtones": [50, 0, -200]}), None, ColorMode::Rgb).unwrap();
    assert_eq!(a, Adjustment::ColorBalance { shadows: [0.0; 3], midtones: [50.0, 0.0, -100.0], highlights: [0.0; 3], preserve_luminosity: true });
    let a = from_params("channelMixer", &json!({"monochrome": true}), None, ColorMode::Rgb).unwrap();
    assert!(matches!(a, Adjustment::ChannelMixer { matrix, monochrome: true } if matrix[0] == [0.4, 0.4, 0.2, 0.0]));
    let a = from_params("photoFilter", &json!({"filter": "cooling80", "density": 60}), None, ColorMode::Rgb).unwrap();
    assert!(matches!(a, Adjustment::PhotoFilter { color, density, .. } if color[2] == 1.0 && (density - 0.6).abs() < 1e-6));
    let a = from_params("blackWhite", &json!({"reds": 120, "tint": true}), None, ColorMode::Rgb).unwrap();
    assert!(matches!(a, Adjustment::BlackWhite { weights, tint: Some(_) } if weights[0] == 120.0 && weights[1] == 60.0));
    let a = from_params("hueSaturation", &json!({"blues": {"hue": 40}}), None, ColorMode::Rgb).unwrap();
    assert!(matches!(&a, Adjustment::HueSaturation { ranges, .. } if ranges[4].hue == 40.0 && ranges[0].is_neutral()));
    // Colorize without a saturation starts at 25%; an explicit 0 and an already colorized base are kept.
    let a = from_params("hueSaturation", &json!({"colorize": true, "hue": 30}), None, ColorMode::Rgb).unwrap();
    assert!(matches!(a, Adjustment::HueSaturation { colorize: true, saturation: 25.0, .. }), "{a:?}");
    let a = from_params("hueSaturation", &json!({"colorize": true, "saturation": 0}), None, ColorMode::Rgb).unwrap();
    assert!(matches!(a, Adjustment::HueSaturation { colorize: true, saturation: 0.0, .. }), "{a:?}");
    let a = from_params("hueSaturation", &json!({"hue": 120}), Some(&a), ColorMode::Rgb).unwrap();
    assert!(matches!(a, Adjustment::HueSaturation { colorize: true, saturation: 0.0, hue: 120.0, .. }), "{a:?}");
    let a = from_params("gradientMap", &json!({"stops": [[1, "#ffffff"], [0, "#000000"]], "dither": true}), None, ColorMode::Rgb).unwrap();
    assert!(matches!(&a, Adjustment::GradientMap { stops, dither: true, .. } if stops[0].0 == 0.0));
}

fn color_space_params_reach_the_model() {
    // CMYK curves: ink channels; Lab: lightness without a composite.
    let a = from_params("curves", &json!({"black": [[0, 0], [255, 200]]}), None, ColorMode::Rgb).unwrap();
    assert!(matches!(&a, Adjustment::Curves { space: ToneSpace::Cmyk, black, .. } if black.len() == 2));
    let a = from_params("curves", &json!({"points": [[0, 40], [255, 255]]}), None, ColorMode::Lab).unwrap();
    assert!(matches!(&a, Adjustment::Curves { space: ToneSpace::Lab, per_channel, .. } if per_channel[0][0].output > 0.1));
    let a = from_params("levels", &json!({"inBlack": 20}), None, ColorMode::Cmyk).unwrap();
    assert!(matches!(&a, Adjustment::Levels { space: ToneSpace::Cmyk, master, .. } if (master.in_black - 20.0 / 255.0).abs() < 1e-6));
}

#[test]
fn bad_params_are_errors_not_panics() {
    for (kind, p) in [
        ("curves", json!({"points": "abc"})),
        ("curves", json!({"points": [[0, 0]]})),
        ("curves", json!({"red": [[0, 0], [1]]})),
        ("curves", json!({"points": [[5, 5], [5, 9]]})),
        ("curves", json!({"pencil": true, "points": [1, 2, 3]})),
        ("curves", json!({"pencil": "yes"})),
        ("curves", json!({"points": (0..256).map(|k| if k == 9 { json!("x") } else { json!(k) }).collect::<Vec<_>>()})),
        ("levels", json!({"red": 3})),
        ("levels", json!({"inBlack": "x"})),
        ("hueSaturation", json!({"reds": {"range": [1, 2]}})),
        ("hueSaturation", json!({"colorize": 1})),
        ("colorBalance", json!({"shadows": [1, 2]})),
        ("blackWhite", json!({"tintColor": "#zz0000"})),
        ("photoFilter", json!({"filter": "nope"})),
        ("photoFilter", json!({"color": [1]})),
        ("channelMixer", json!({"red": [1, 2, 3]})),
        ("gradientMap", json!({"stops": [[0, "#000"]]})),
        ("gradientMap", json!({"stops": 7})),
        ("selectiveColor", json!({"method": 5})),
        ("selectiveColor", json!({"reds": "not-an-array"})),
        ("selectiveColor", json!({"colors": "reds", "cyan": "50"})),
        ("colorLookup", json!({"lut": 42})),
        ("colorLookup", json!({"interpolation": 5})),
        ("colorLookup", json!({"dither": "yes"})),
        ("nope", json!({})),
        ("levels", json!([1, 2])),
    ] {
        assert!(from_params(kind, &p, None, ColorMode::Rgb).is_err(), "{kind} {p}");
    }
    // Out-of-range numbers clamp.
    let a = from_params("hueSaturation", &json!({"hue": 1e9, "saturation": -1e9}), None, ColorMode::Rgb).unwrap();
    assert!(matches!(a, Adjustment::HueSaturation { hue: 180.0, saturation: -100.0, .. }));
    let a = from_params("levels", &json!({"inBlack": 250, "inWhite": 10}), None, ColorMode::Rgb).unwrap();
    assert!(matches!(a, Adjustment::Levels { master, .. } if master.in_white > master.in_black));
}

#[test]
fn curve_points_outside_0_255_are_refused_as_the_message_says() {
    // Photoshop refuses them too ("the settings in the file are invalid"); they used to be
    // moved onto the edge silently.
    for pts in [json!([[0, 0], [300, 255]]), json!([[0, -1], [255, 255]]), json!([[0, 0]])] {
        let e = from_params("curves", &json!({ "points": pts }), None, ColorMode::Rgb).unwrap_err().to_string();
        assert!(e.contains("[input, output] pairs in 0..255") && e.contains("`curves adjustment`"), "{e}");
    }
    assert!(from_params("curves", &json!({"points": [[0, 0], [128.5, 140], [255, 255]]}), None, ColorMode::Rgb).is_ok());
}

fn curves_parts(a: &Adjustment) -> (&[CurvePoint], &[Vec<CurvePoint>; 3]) {
    match a {
        Adjustment::Curves { master, per_channel, .. } => (master, per_channel),
        _ => panic!("expected curves"),
    }
}

fn tone_output(adjustment: &Adjustment, sample: [f32; 3]) -> [f32; 3] {
    let luts = photocraft_compose::adjust::tone_luts(adjustment);
    std::array::from_fn(|channel| {
        let row = &luts[channel];
        let position = sample[channel].clamp(0.0, 1.0) * (row.len().saturating_sub(1)) as f32;
        let lower = position.floor() as usize;
        let upper = lower.saturating_add(1).min(row.len().saturating_sub(1));
        let fraction = position - lower as f32;
        row.get(lower).copied().unwrap_or(0.0) * (1.0 - fraction) + row.get(upper).copied().unwrap_or(0.0) * fraction
    })
}

#[test]
fn curves_black_and_white_eyedroppers_move_the_input_references() {
    let base = json!({
        "points": [[0, 8], [120, 150], [255, 245]],
        "red": [[0, 0], [100, 130], [255, 255]],
        "green": [[0, 0], [110, 125], [255, 255]],
        "blue": [[0, 0], [90, 120], [255, 255]]
    });
    let black = curves_eyedropper(&base, CurvesEyedropper::Black, [0.2, 0.3, 0.4], ColorMode::Rgb).unwrap();
    let (master, channels) = curves_parts(&black);
    for (curve, sample) in channels.iter().zip([0.2, 0.3, 0.4]) {
        assert!((curve[0].input - sample).abs() < 1e-6 && curve[0].output == 0.0, "{curve:?}");
        assert_eq!(curve.len(), 3, "existing edit points are preserved");
    }
    assert_eq!(master[0].output, 0.0, "the composite cannot lift the sampled black");
    assert!(tone_output(&black, [0.2, 0.3, 0.4]).iter().all(|value| *value < 1e-3));

    let white = curves_eyedropper(&base, CurvesEyedropper::White, [0.6, 0.7, 0.8], ColorMode::Rgb).unwrap();
    let (master, channels) = curves_parts(&white);
    for (curve, sample) in channels.iter().zip([0.6, 0.7, 0.8]) {
        let end = curve.last().unwrap();
        assert!((end.input - sample).abs() < 1e-6 && end.output == 1.0, "{curve:?}");
        assert_eq!(curve.len(), 3, "existing edit points are preserved");
    }
    assert_eq!(master.last().map(|p| p.output), Some(1.0), "the composite cannot lower the sampled white");
    assert!(tone_output(&white, [0.6, 0.7, 0.8]).iter().all(|value| *value > 1.0 - 1e-3));

    let edge_black = curves_eyedropper(&base, CurvesEyedropper::Black, [1.0, 0.3, 0.4], ColorMode::Rgb).unwrap();
    let (_, edge_channels) = curves_parts(&edge_black);
    assert_eq!(edge_channels[0].len(), 3, "edge samples preserve other curve points");
    assert_eq!(edge_channels[0].last().map(|p| p.output), Some(0.0));
    let edge_white = curves_eyedropper(&base, CurvesEyedropper::White, [0.0, 0.7, 0.8], ColorMode::Rgb).unwrap();
    let (_, edge_channels) = curves_parts(&edge_white);
    assert_eq!(edge_channels[0].len(), 3, "edge samples preserve other curve points");
    assert_eq!(edge_channels[0].first().map(|p| p.output), Some(1.0));
}

#[test]
fn black_and_white_eyedroppers_round_trip_near_serialization_boundaries() {
    let cases = [
        (CurvesEyedropper::White, 0.0, 1.0),
        (CurvesEyedropper::White, 0.001, 1.0),
        (CurvesEyedropper::White, 0.494 / 255.0, 1.0),
        // 0.496 level rounds to 0.50; the sample is just below that representable point.
        (CurvesEyedropper::White, 0.496 / 255.0, 0.992),
        (CurvesEyedropper::White, 0.2, 1.0),
        (CurvesEyedropper::Black, 1.0, 0.0),
        (CurvesEyedropper::Black, 0.999, 0.0),
        (CurvesEyedropper::Black, 1.0 - 0.494 / 255.0, 0.0),
        // 254.504 levels rounds to 254.50, leaving a representable 0.50-level gap.
        (CurvesEyedropper::Black, 1.0 - 0.496 / 255.0, 0.008),
        (CurvesEyedropper::Black, 0.8, 0.0),
    ];
    for (picker, sample, expected_output) in cases {
        let resolved = curves_eyedropper(&json!({}), picker, [sample; 3], ColorMode::Rgb).unwrap();
        let params = to_params(&resolved);
        let parsed = from_params("curves", &params, None, ColorMode::Rgb).unwrap();
        let (master, channels) = curves_parts(&parsed);
        for curve in std::iter::once(master).chain(channels.iter().map(Vec::as_slice)) {
            assert!(curve.len() >= 2, "{picker:?} sample={sample}: {curve:?}");
            assert!(curve.len() <= MAX_CURVE_POINTS, "{picker:?} sample={sample}: {curve:?}");
            assert!(curve.windows(2).all(|pair| pair[0].input < pair[1].input), "{picker:?} sample={sample}: {curve:?}");
            assert!(curve.windows(2).all(|pair| pair[1].input - pair[0].input >= CURVE_POINT_DUPLICATE_DISTANCE), "{picker:?} sample={sample}: {curve:?}");
        }
        let output = tone_output(&parsed, [sample; 3]);
        assert!(output.iter().all(|value| (*value - expected_output).abs() < 1e-3), "{picker:?} sample={sample} output={output:?} expected={expected_output}");
    }
}

#[test]
fn endpoint_eyedroppers_fall_back_when_serialization_collapses_curve_points() {
    let base = json!({
        "points": [[0, 0], [64, 50], [128, 100], [192, 200], [255, 255]],
        "red": [[0, 0], [64, 50], [128, 100], [192, 200], [255, 255]],
        "green": [[0, 0], [64, 50], [128, 100], [192, 200], [255, 255]],
        "blue": [[0, 0], [64, 50], [128, 100], [192, 200], [255, 255]]
    });
    let cases = [(CurvesEyedropper::Black, 0.995, 0.0, true), (CurvesEyedropper::White, 0.005, 1.0, false)];
    for (picker, sample, target, black) in cases {
        let resolved = curves_eyedropper(&base, picker, [sample; 3], ColorMode::Rgb).unwrap();
        let parsed = from_params("curves", &to_params(&resolved), None, ColorMode::Rgb).unwrap();
        let (master, channels) = curves_parts(&parsed);
        for curve in std::iter::once(master).chain(channels.iter().map(Vec::as_slice)) {
            assert_eq!(curve.len(), 5, "collapsed points use the deterministic fallback: {curve:?}");
            for (point, input) in curve.iter().zip([0.0, 64.0, 128.0, 192.0, 255.0]) {
                assert!((point.input * 255.0 - input).abs() < 1e-3, "existing point position was retained: {curve:?}");
            }
        }
        for curve in channels {
            let plateau: Vec<_> = if black { curve.iter().rev().take(2).collect() } else { curve.iter().take(2).collect() };
            assert!(plateau.into_iter().all(|point| (point.output - target).abs() < 1e-6), "{curve:?}");
        }
        let output = tone_output(&parsed, [sample; 3]);
        assert!(output.iter().all(|value| (*value - target).abs() < 0.01), "{picker:?}: {output:?}");
    }
}

#[test]
fn black_and_white_round_trips_preserve_representable_curve_edits() {
    let base = json!({
        "points": [[0, 8], [120, 150], [255, 245]],
        "red": [[0, 0], [100, 130], [255, 255]],
        "green": [[0, 0], [110, 125], [255, 255]],
        "blue": [[0, 0], [90, 120], [255, 255]]
    });
    let black = curves_eyedropper(&base, CurvesEyedropper::Black, [0.2, 0.3, 0.4], ColorMode::Rgb).unwrap();
    let black = from_params("curves", &to_params(&black), None, ColorMode::Rgb).unwrap();
    let (_, channels) = curves_parts(&black);
    assert_eq!(channels[0].len(), 3);
    assert!((channels[0][1].input * 255.0 - 131.0).abs() < 1e-3);
    assert!((channels[0][1].output * 255.0 - 130.0).abs() < 1e-3);

    let white = curves_eyedropper(&base, CurvesEyedropper::White, [0.6, 0.7, 0.8], ColorMode::Rgb).unwrap();
    let white = from_params("curves", &to_params(&white), None, ColorMode::Rgb).unwrap();
    let (_, channels) = curves_parts(&white);
    assert_eq!(channels[0].len(), 3);
    assert!((channels[0][1].input * 255.0 - 60.0).abs() < 1e-3);
    assert!((channels[0][1].output * 255.0 - 130.0).abs() < 1e-3);
}

fn assert_gray_point_preserves_luma_and_existing_curve_edits(sample: [f32; 3]) {
    let base = json!({
        "points": [[0, 0], [128, 96], [255, 255]],
        "red": [[0, 0], [80, 90], [255, 255]],
        "green": [[0, 0], [90, 100], [255, 255]],
        "blue": [[0, 0], [70, 80], [255, 255]]
    });
    let original = from_params("curves", &base, None, ColorMode::Rgb).unwrap();
    let expected_luma = rgb_luma(sample);
    let resolved = curves_eyedropper(&base, CurvesEyedropper::NeutralGray, sample, ColorMode::Rgb).unwrap();
    let parsed = from_params("curves", &to_params(&resolved), None, ColorMode::Rgb).unwrap();
    let (master, channels) = curves_parts(&parsed);
    let (original_master, original_channels) = curves_parts(&original);
    assert_eq!(master, original_master, "the existing master curve is retained");
    for (curve, old_curve) in channels.iter().zip(original_channels) {
        let old_edit = old_curve.get(1).expect("the test curve has an interior edit");
        assert!(
            curve.iter().any(|point| { (point.input - old_edit.input).abs() < 1e-6 && (point.output - old_edit.output).abs() < 1e-6 }),
            "the existing channel edit is retained: {curve:?}"
        );
    }
    let output = tone_output(&parsed, sample);
    assert!((output[0] - output[1]).abs() < 0.002 && (output[1] - output[2]).abs() < 0.002, "sample must become neutral: {output:?}");
    assert!((rgb_luma(output) - expected_luma).abs() < 0.003, "luma changed from {expected_luma} to {} ({output:?})", rgb_luma(output));
}

#[test]
fn curves_neutral_gray_preserves_luma_for_a_light_cast_sample() {
    assert_gray_point_preserves_luma_and_existing_curve_edits([0.88, 0.76, 0.63]);
}

#[test]
fn curves_neutral_gray_preserves_luma_for_a_dark_cast_sample() {
    assert_gray_point_preserves_luma_and_existing_curve_edits([0.17, 0.105, 0.075]);
}

#[test]
fn curves_neutral_gray_handles_near_black_and_near_white_samples() {
    for sample in [[0.004, 0.006, 0.008], [0.992, 0.996, 0.999], [0.0, 0.0, 0.0], [1.0, 1.0, 1.0]] {
        let resolved = curves_eyedropper(&json!({}), CurvesEyedropper::NeutralGray, sample, ColorMode::Rgb).unwrap();
        let parsed = from_params("curves", &to_params(&resolved), None, ColorMode::Rgb).unwrap();
        let (master, channels) = curves_parts(&parsed);
        for curve in std::iter::once(master).chain(channels.iter().map(Vec::as_slice)) {
            assert!(curve.len() >= 2, "{sample:?}: {curve:?}");
            assert!(curve.windows(2).all(|pair| pair[0].input < pair[1].input), "{sample:?}: {curve:?}");
            assert!(curve.windows(2).all(|pair| pair[1].input - pair[0].input >= CURVE_POINT_DUPLICATE_DISTANCE), "{sample:?}: {curve:?}");
        }
        let output = tone_output(&parsed, sample);
        assert!((output[0] - output[1]).abs() < 0.002 && (output[1] - output[2]).abs() < 0.002, "{sample:?}: {output:?}");
        assert!((rgb_luma(output) - rgb_luma(sample)).abs() < 0.003, "{sample:?}: {output:?}");
    }
}

#[test]
fn gray_point_fallback_keeps_tightly_spaced_points_round_trip_safe() {
    let base = json!({
        "red": [[0, 0], [100, 100], [100.51, 100], [255, 255]],
        "green": [[0, 0], [100, 100], [100.51, 100], [255, 255]],
        "blue": [[0, 0], [100, 100], [100.51, 100], [255, 255]]
    });
    let sample = [100.25 / 255.0, 100.1 / 255.0, 100.4 / 255.0];
    let target = rgb_luma(sample);
    let resolved = curves_eyedropper(&base, CurvesEyedropper::NeutralGray, sample, ColorMode::Rgb).unwrap();
    let parsed = from_params("curves", &to_params(&resolved), None, ColorMode::Rgb).unwrap();
    let (_, channels) = curves_parts(&parsed);
    for (curve, input) in channels.iter().zip(sample) {
        assert_eq!(curve.len(), 4, "the fallback keeps every serialized point: {curve:?}");
        assert!(curve.windows(2).all(|pair| pair[1].input - pair[0].input >= CURVE_POINT_DUPLICATE_DISTANCE), "{curve:?}");
        let sample_pair = curve.windows(2).find(|pair| pair[0].input <= input && input <= pair[1].input).unwrap();
        assert!((sample_pair[0].output - sample_pair[1].output).abs() < 1e-6, "fallback forms a plateau around the sample: {curve:?}");
        assert!((sample_pair[0].output - target).abs() < 0.001, "fallback uses the luma target: {curve:?}");
    }
}

#[test]
fn curves_eyedropper_param_drives_the_command() {
    let mut s = crate::Session::new();
    s.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
    s.execute("edit.fill", json!({"color": [0.6, 0.5, 0.4]})).unwrap();
    s.execute("image.adjustments.curves", json!({"eyedropper": {"point": "white", "at": [2, 2]}})).unwrap();
    let px: Vec<f32> = serde_json::from_value(s.execute("document.pixel", json!({"x": 2, "y": 2})).unwrap()).unwrap();
    assert!(px.iter().take(3).all(|v| *v > 0.99), "the sample becomes white: {px:?}");
    s.execute("image.adjustments.curves", json!({"eyedropper": {"point": "black", "color": [1.0, 1.0, 1.0]}})).unwrap();
    let px: Vec<f32> = serde_json::from_value(s.execute("document.pixel", json!({"x": 2, "y": 2})).unwrap()).unwrap();
    assert!(px.iter().take(3).all(|v| *v < 0.01), "the given colour becomes black: {px:?}");
    for bad in [
        json!("black"),
        json!({"point": "nope", "at": [1, 1]}),
        json!({"point": "black"}),
        json!({"point": "black", "at": [99, 1]}),
        json!({"point": "black", "at": [-1, 1]}),
        json!({"point": "black", "at": [1]}),
        json!({"point": "gray", "at": [f64::MAX, 1]}),
        json!({"point": "black", "color": "red"}),
    ] {
        assert!(s.execute("image.adjustments.curves", json!({"eyedropper": bad})).is_err(), "{bad}");
    }
}

/// The Properties panel's pickers on a Curves adjustment layer (UI-217-15): one undoable step
/// that samples the image below the layer, so a repeated click keeps the same curve.
#[test]
fn layer_eyedropper_samples_below_the_curves_layer() {
    let mut s = crate::Session::new();
    s.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
    s.execute("edit.fill", json!({"color": [0.6, 0.5, 0.4]})).unwrap();
    s.execute("layer.newAdjustmentLayer.curves", json!({})).unwrap();
    let steps = s.active().unwrap().history.past_len();
    let pick = json!({"eyedropper": {"point": "white", "at": [2, 2], "size": 3}});
    s.execute("layer.setAdjustment", pick.clone()).unwrap();
    assert_eq!(s.active().unwrap().history.past_len(), steps + 1);
    let px: Vec<f32> = serde_json::from_value(s.execute("document.pixel", json!({"x": 2, "y": 2})).unwrap()).unwrap();
    assert!(px.iter().take(3).all(|v| *v > 0.99), "the sample becomes white: {px:?}");
    let curve = s.active().unwrap().doc.clone();
    s.execute("layer.setAdjustment", pick).unwrap();
    let same = |a: &photocraft_doc::Document, b: &photocraft_doc::Document| a.layers.last().map(|l| &l.content) == b.layers.last().map(|l| &l.content);
    assert!(same(&curve, &s.active().unwrap().doc), "sampled below the layer, not through it");
    s.execute("edit.undo", json!({})).unwrap();
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(s.active().unwrap().history.past_len(), steps);
    for bad in [json!({"point": "black", "at": [99, 1]}), json!({"point": "black", "at": [1, 1], "size": 2}), json!({"point": "x", "at": [1, 1]})] {
        assert!(s.execute("layer.setAdjustment", json!({"eyedropper": bad})).is_err(), "{bad}");
    }
    s.execute("layer.newAdjustmentLayer.hueSaturation", json!({})).unwrap();
    assert!(s.execute("layer.setAdjustment", json!({"eyedropper": {"point": "white", "at": [1, 1]}})).is_err(), "Levels and Curves only");
}

/// #3127: on a smart object the eyedropper param records the adjustment it resolved to as
/// the smart filter, not the raw sample (which re-rendered as an identity).
#[test]
fn eyedropper_on_a_smart_object_records_the_resolved_adjustment() {
    for kind in ["curves", "levels"] {
        let mut s = crate::Session::new();
        s.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
        s.execute("edit.fill", json!({"color": [0.6, 0.5, 0.4]})).unwrap();
        s.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
        s.execute(&format!("image.adjustments.{kind}"), json!({"eyedropper": {"point": "white", "at": [2, 2]}})).unwrap();
        let px: Vec<f32> = serde_json::from_value(s.execute("document.pixel", json!({"x": 2, "y": 2})).unwrap()).unwrap();
        assert!(px.iter().take(3).all(|v| *v > 0.99), "{kind}: {px:?}");
    }
}

/// Levels' Set White/Neutral Gray Point (#3116): the white point maps the sampled colour to
/// white at 8 and 16 bit, the gray point neutralises a tinted grey at its luma on a Levels
/// layer as one undoable step, and bad picks fail.
#[test]
fn levels_eyedroppers_set_white_and_neutral_gray() {
    let pixel = |s: &mut crate::Session| -> Vec<f32> { serde_json::from_value(s.execute("document.pixel", json!({"x": 2, "y": 2})).unwrap()).unwrap() };
    for depth in [8, 16] {
        let mut s = crate::Session::new();
        s.execute("file.new", json!({"width": 8, "height": 8, "depth": depth})).unwrap();
        s.execute("edit.fill", json!({"color": [0.6, 0.5, 0.4]})).unwrap();
        s.execute("image.adjustments.levels", json!({"eyedropper": {"point": "white", "at": [2, 2]}})).unwrap();
        let px = pixel(&mut s);
        assert!(px.iter().take(3).all(|v| *v > 0.995), "{depth}-bit: the sample becomes white: {px:?}");
    }
    let mut s = crate::Session::new();
    s.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
    s.execute("edit.fill", json!({"color": [0.6, 0.5, 0.4]})).unwrap();
    let before = pixel(&mut s);
    s.execute("layer.newAdjustmentLayer.levels", json!({})).unwrap();
    let steps = s.active().unwrap().history.past_len();
    s.execute("layer.setAdjustment", json!({"eyedropper": {"point": "gray", "at": [2, 2]}})).unwrap();
    assert_eq!(s.active().unwrap().history.past_len(), steps + 1);
    let px = pixel(&mut s);
    let (lo, hi) = px.iter().take(3).fold((1.0f32, 0.0f32), |(lo, hi), v| (lo.min(*v), hi.max(*v)));
    assert!(hi - lo < 2.5 / 255.0, "neutral: {px:?}");
    let luma = |c: &[f32]| c[0] * 0.2126 + c[1] * 0.7152 + c[2] * 0.0722;
    assert!((luma(&px) - luma(&before)).abs() < 2.0 / 255.0, "keeps the luma: {px:?} vs {before:?}");
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(pixel(&mut s), before);
    for bad in [json!({"point": "gray", "at": [99, 1]}), json!({"point": "x", "at": [1, 1]}), json!({"point": "white", "color": [f64::NAN, 0, 0]})] {
        assert!(s.execute("layer.setAdjustment", json!({"eyedropper": bad})).is_err(), "{bad}");
        assert!(s.execute("image.adjustments.levels", json!({"eyedropper": bad})).is_err(), "{bad}");
    }
    s.execute("file.new", json!({"width": 8, "height": 8, "mode": "grayscale"})).unwrap();
    assert!(s.execute("image.adjustments.levels", json!({"eyedropper": {"point": "gray", "at": [1, 1]}})).is_err(), "no gray picker in Grayscale");
    s.execute("image.adjustments.levels", json!({"eyedropper": {"point": "black", "color": [1.0, 1.0, 1.0]}})).unwrap();
}

#[test]
fn curves_eyedropper_support_is_explicit_and_bad_samples_fail() {
    assert!(curves_eyedropper(&json!({}), CurvesEyedropper::Black, [0.4; 3], ColorMode::Grayscale).is_ok());
    assert!(curves_eyedropper(&json!({}), CurvesEyedropper::White, [0.4; 3], ColorMode::Grayscale).is_ok());
    assert!(curves_eyedropper(&json!({}), CurvesEyedropper::NeutralGray, [0.4; 3], ColorMode::Grayscale).is_err());
    for mode in [ColorMode::Cmyk, ColorMode::Lab] {
        assert!(curves_eyedropper(&json!({}), CurvesEyedropper::Black, [0.4; 3], mode).is_err());
    }
    assert!(curves_eyedropper(&json!({}), CurvesEyedropper::Black, [f32::NAN, 0.0, 0.0], ColorMode::Rgb).is_err());
}

#[test]
fn hue_range_sliders_stay_ordered() {
    let a = from_params("hueSaturation", &json!({"greens": {"range": [100, 90, 200, 150]}}), None, ColorMode::Rgb).unwrap();
    let Adjustment::HueSaturation { ranges, .. } = a else { panic!() };
    let b = ranges[2].bounds;
    assert!(b[0] <= b[1] && b[1] <= b[2] && b[2] <= b[3] && b[3] - b[0] < 360.0, "{b:?}");
}

/// A colourful 16×16 document (`mode`, `depth`) for command tests.
fn colourful(mode: &str, depth: u32) -> crate::Session {
    let mut s = crate::Session::new();
    s.execute("file.new", json!({"width": 16, "height": 16, "mode": mode, "depth": depth})).unwrap();
    for (i, c) in ["#c83c28", "#3c8c50", "#2850b4", "#d2b464"].iter().enumerate() {
        s.execute("select.rect", json!({"x": (i % 2) * 8, "y": (i / 2) * 8, "width": 8, "height": 8})).unwrap();
        s.execute("edit.fill", json!({"color": c})).unwrap();
    }
    s.execute("select.deselect", json!({})).unwrap();
    s
}

fn pixels(s: &crate::Session) -> Vec<Vec<f32>> {
    let d = s.active().unwrap();
    let bg = d.doc.layers[0].surface().unwrap();
    [(2, 2), (10, 2), (2, 10), (10, 10)].iter().map(|&(x, y)| bg.pixel(x, y)).collect()
}

#[test]
fn destructive_commands_apply_their_params_at_every_depth() {
    for depth in [8, 16, 32] {
        for kind in KINDS.iter().filter(|k| **k != "invert") {
            let mut s = colourful("rgb", depth);
            let before = pixels(&s);
            let steps = s.active().unwrap().history.past_len();
            s.execute(&format!("image.adjustments.{kind}"), sample(kind)).unwrap_or_else(|e| panic!("{kind} @{depth}: {e}"));
            assert_ne!(pixels(&s), before, "{kind} @{depth} changed nothing");
            assert_eq!(s.active().unwrap().history.past_len(), steps + 1, "{kind}: one history step");
            // Identity params leave the pixels alone (within one 8-bit step).
            let mut s = colourful("rgb", depth);
            s.execute(&format!("image.adjustments.{kind}"), to_params(&default_for(kind, ColorMode::Rgb).unwrap())).unwrap();
            if !matches!(*kind, "threshold" | "posterize" | "blackWhite" | "gradientMap" | "photoFilter") {
                for (a, b) in pixels(&s).iter().zip(&before) {
                    assert!(a.iter().zip(b).all(|(x, y)| (x - y).abs() <= 1.0 / 255.0 + 1e-4), "{kind} @{depth}: {a:?} vs {b:?}");
                }
            }
        }
    }
}

#[test]
fn curves_pencil_maps_draw_smooth_and_convert_back() {
    // A drawn inversion: 256 output levels.
    let inverted: Vec<f32> = (0..256).rev().map(|k| k as f32).collect();
    let a = from_params("curves", &json!({"pencil": true, "points": inverted}), None, ColorMode::Rgb).unwrap();
    assert!(matches!(&a, Adjustment::Curves { pencil: true, master, .. } if master.len() == CURVE_MAP_LEN));
    assert!((tone_output(&a, [0.25; 3])[0] - 0.75).abs() < 1e-3);
    assert_eq!(from_params("curves", &to_params(&a), None, ColorMode::Rgb).unwrap(), a);
    // Smooth evens out a hard step but keeps the endpoints.
    let step: Vec<f32> = (0..256).map(|k| if k < 128 { 0.0 } else { 255.0 }).collect();
    let s = from_params("curves", &json!({"red": step, "smooth": 2}), Some(&a), ColorMode::Rgb).unwrap();
    let (_, chans) = curves_parts(&s);
    assert!(chans[0][127].output > 0.2 && chans[0][128].output < 0.8, "{:?}", &chans[0][120..136]);
    assert_eq!((chans[0][0].output, chans[0][255].output), (0.0, 1.0));
    // Edit points: the map is fitted with a point curve that still inverts.
    let pts = from_params("curves", &json!({"pencil": false}), Some(&a), ColorMode::Rgb).unwrap();
    let (master, _) = curves_parts(&pts);
    assert!(matches!(&pts, Adjustment::Curves { pencil: false, .. }) && master.len() <= MAX_FITTED_POINTS, "{master:?}");
    assert!((tone_output(&pts, [0.25; 3])[0] - 0.75).abs() < 2.0 / 255.0);
    // The command draws it at every depth.
    for depth in [8, 16, 32] {
        let mut s = colourful("rgb", depth);
        let before = pixels(&s);
        s.execute("image.adjustments.curves", to_params(&a)).unwrap();
        let after = pixels(&s);
        assert!((after[0][0] - (1.0 - before[0][0])).abs() < 2.0 / 255.0, "@{depth}: {after:?} vs {before:?}");
    }
}

#[test]
fn destructive_levels_dont_clip_in_32_bit() {
    // Input 15..230, gamma 1.3, output 10..245: integer documents stop at the output range,
    // while float documents preserve the curve's signed and HDR values beyond it (the
    // compose-side `levels_dont_clip_in_32_bit` pins the same curve, #2592).
    let params = json!({"inBlack": 15, "inWhite": 230, "gamma": 1.3, "outBlack": 10, "outWhite": 245});
    for depth in [8, 16, 32] {
        let mut s = crate::Session::new();
        s.execute("file.new", json!({"width": 16, "height": 16, "depth": depth, "background": "white"})).unwrap();
        s.execute("select.rect", json!({"x": 0, "y": 0, "width": 8, "height": 16})).unwrap();
        s.execute("edit.fill", json!({"color": "#000000"})).unwrap();
        s.execute("select.deselect", json!({})).unwrap();
        s.execute("image.adjustments.levels", params.clone()).unwrap();
        let px = pixels(&s);
        if depth == 32 {
            assert!(px[0][0].is_finite() && px[0][0] < 0.0, "float black keeps its signed result: {:?}", px[0]);
            assert!(px[1][0].is_finite() && px[1][0] > 1.0, "float white keeps its HDR result: {:?}", px[1]);
        } else {
            assert!((px[0][0] * 255.0 - 10.0).abs() < 0.6, "black @{depth}: {:?}", px[0]);
            assert!((px[1][0] * 255.0 - 245.0).abs() < 0.6, "white @{depth}: {:?}", px[1]);
        }
    }
}

#[test]
fn cmyk_and_lab_documents_use_their_channels() {
    let mut s = colourful("cmyk", 8);
    let before = pixels(&s);
    // More black ink darkens every patch.
    s.execute("image.adjustments.curves", json!({"black": [[0, 0], [255, 180]]})).unwrap();
    let after = pixels(&s);
    assert!(after.iter().zip(&before).all(|(a, b)| a[3] > b[3] + 0.05), "{after:?} vs {before:?}");
    let mut s = colourful("lab", 16);
    let before = pixels(&s);
    s.execute("image.adjustments.levels", json!({"lightness": {"outBlack": 60}})).unwrap();
    let after = pixels(&s);
    assert!(after.iter().zip(&before).all(|(a, b)| a[0] >= b[0]), "lightness lifted {after:?} vs {before:?}");
    // The adjustment layer of a CMYK document defaults to ink channels.
    let mut s = colourful("cmyk", 8);
    s.execute("layer.newAdjustmentLayer.curves", json!({})).unwrap();
    let id = s.active().unwrap().active_layer.unwrap();
    let l = s.active().unwrap().doc.layer(id).unwrap().clone();
    assert!(matches!(l.content, photocraft_doc::LayerContent::Adjustment(Adjustment::Curves { space: ToneSpace::Cmyk, .. })));
}

#[test]
fn set_adjustment_merges_and_resets() {
    let mut s = colourful("rgb", 8);
    s.execute("layer.newAdjustmentLayer.colorBalance", json!({"midtones": [30, 0, 0]})).unwrap();
    s.execute("layer.setAdjustment", json!({"highlights": [0, 0, -20]})).unwrap();
    let id = s.active().unwrap().active_layer.unwrap();
    let adj = |s: &crate::Session| match &s.active().unwrap().doc.layer(id).unwrap().content {
        photocraft_doc::LayerContent::Adjustment(a) => a.clone(),
        _ => panic!(),
    };
    assert!(matches!(adj(&s), Adjustment::ColorBalance { midtones: [30.0, 0.0, 0.0], highlights: [0.0, 0.0, -20.0], .. }));
    s.execute("layer.setAdjustment", json!({"layer": id.0})).unwrap();
    assert_eq!(adj(&s), default_for("colorBalance", ColorMode::Rgb).unwrap());
    // Bad params are errors and leave the layer alone.
    s.execute("layer.setAdjustment", json!({"midtones": [10, 10, 10]})).unwrap();
    let kept = adj(&s);
    for bad in [json!({"midtones": "x"}), json!({"preserveLuminosity": 3}), json!({"shadows": [1]})] {
        assert!(s.execute("layer.setAdjustment", bad.clone()).is_err(), "{bad}");
        assert_eq!(adj(&s), kept);
    }
}

#[test]
fn commands_reject_bad_params_gracefully() {
    let mut s = colourful("rgb", 8);
    for (kind, p) in [
        ("curves", json!({"points": [[0, 0]]})),
        ("levels", json!({"red": "dark"})),
        ("gradientMap", json!({"stops": [[0, 1, 2]]})),
        ("photoFilter", json!({"filter": 3})),
        ("channelMixer", json!({"gray": [1, 2]})),
        ("hueSaturation", json!({"reds": 4})),
        ("blackWhite", json!({"tint": "yes"})),
    ] {
        for prefix in ["image.adjustments.", "layer.newAdjustmentLayer."] {
            let r = s.execute(&format!("{prefix}{kind}"), p.clone());
            assert!(matches!(r, Err(EngineError::BadParams { .. })), "{prefix}{kind} {p}: {r:?}");
        }
    }
}
#[test]
fn destructive_levels_match_independent_signed_hdr_endpoints() {
    // Input 15..230, gamma 1.3, output 10..245: black and white land on the output range in
    // integer documents. Float32 keeps HDR values outside 0..1, as the compositor's real-file
    // Levels oracle does. These independently calculated endpoints use the signed-power
    // curve at black and white, not display-clipped values.
    let params = json!({"inBlack": 15, "inWhite": 230, "gamma": 1.3, "outBlack": 10, "outWhite": 245});
    for (depth, low, high, tolerance) in [(8, 10.0, 245.0, 0.6), (16, 10.0, 245.0, 0.6), (32, -20.308_89, 265.750_3, 0.01)] {
        let mut s = crate::Session::new();
        s.execute("file.new", json!({"width": 16, "height": 16, "depth": depth, "background": "white"})).unwrap();
        s.execute("select.rect", json!({"x": 0, "y": 0, "width": 8, "height": 16})).unwrap();
        s.execute("edit.fill", json!({"color": "#000000"})).unwrap();
        s.execute("select.deselect", json!({})).unwrap();
        s.execute("image.adjustments.levels", params.clone()).unwrap();
        let px = pixels(&s);
        assert!((px[0][0] * 255.0 - low).abs() < tolerance, "black @{depth}: {:?}", px[0]);
        assert!((px[1][0] * 255.0 - high).abs() < tolerance, "white @{depth}: {:?}", px[1]);
        if depth == 32 {
            assert!(px[0][0] < 0.0 && px[1][0] > 1.0, "HDR endpoints must remain unclipped");
        }
    }
}
