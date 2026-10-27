use super::*;
use photocraft_geom::Rect;

fn ramp() -> Buffer {
    let rect = Rect::new(0, 0, 8, 8);
    let px = (0..64).map(|i| [((i % 8) as f32) / 7.0, ((i / 8) as f32) / 7.0, 0.3, 1.0]).collect();
    Buffer { rect, px }
}

/// Photoshop 27.11, measured: Brightness/Contrast with Use Legacy on, over a gray ramp
/// (x = 0, 16, …, 240, 255), one row per (brightness, contrast).
/// One pixel through `adj` in a 32-bit document.
fn hdr(adj: &Adjustment, c: [f32; 3]) -> [f32; 3] {
    let mut b = Buffer::filled(crate::Rect::new(0, 0, 1, 1), [c[0], c[1], c[2], 1.0]);
    apply_depth(adj, &mut b, Transfer::Gamma(1.0), Some(SampleType::F32));
    [b.px[0][0], b.px[0][1], b.px[0][2]]
}

/// Photoshop's 32-bit Hue/Saturation (photoshop corpus rgb32 hue-saturation.psd: hue +40,
/// saturation +25, lightness -10): saturated colours leave 0..1, and the saturation factor is
/// 1 / (1 - 63/255), so red becomes (1.0477, 0.649, -0.1477), where 8 bits clip to (0.9, 0.6, 0).
#[test]
fn thirty_two_bit_hue_saturation_is_unclipped() {
    let adj = Adjustment::HueSaturation { hue: 40.0, saturation: 25.0, lightness: -10.0, colorize: false, ranges: HueRange::defaults() };
    let near = |got: [f32; 3], want: [f32; 3]| got.iter().zip(want).all(|(g, w)| (g - w).abs() < 1e-3);
    for (c, want) in
        [([1.0, 0.0, 0.0], [1.0477, 0.649, -0.1477]), ([0.9301, 0.6105, 0.2541], [0.7804, 0.9369, 0.1289]), ([0.0, 0.3864, 1.0], [0.1871, -0.1477, 1.0477])]
    {
        assert!(near(hdr(&adj, c), want), "{c:?}: {:?}", hdr(&adj, c));
    }
    let mut b = Buffer::filled(crate::Rect::new(0, 0, 1, 1), [1.0, 0.0, 0.0, 1.0]);
    apply_depth(&adj, &mut b, Transfer::Srgb, Some(SampleType::U16));
    assert!(b.px[0].iter().all(|v| (0.0..=1.0).contains(v)), "16 bits clip: {:?}", b.px[0]);
}

/// Photoshop's 32-bit Colorize (photoshop corpus rgb32 hue-saturation-colorize.psd: hue -160,
/// saturation 40, lightness +5): red becomes (0.3445, 0.5852, 0.7055), 0.95 of the chroma an
/// 8-bit Colorize gives at the same lightness, (0.335, 0.5883, 0.715).
#[test]
fn thirty_two_bit_colorize_scales_the_saturation() {
    let adj = Adjustment::HueSaturation { hue: -160.0, saturation: 40.0, lightness: 5.0, colorize: true, ranges: HueRange::defaults() };
    let near = |got: [f32; 3], want: [f32; 3]| got.iter().zip(want).all(|(g, w)| (g - w).abs() < 2e-3);
    let red = hdr(&adj, [1.0, 0.0, 0.0]);
    assert!(near(red, [0.3445, 0.5852, 0.7055]), "{red:?}");
    let mut b = Buffer::filled(crate::Rect::new(0, 0, 1, 1), [1.0, 0.0, 0.0, 1.0]);
    apply_depth(&adj, &mut b, Transfer::Srgb, Some(SampleType::U16));
    assert!(near([b.px[0][0], b.px[0][1], b.px[0][2]], [0.335, 0.5883, 0.715]), "16-bit keeps the full saturation: {:?}", b.px[0]);
}

/// Photoshop's 32-bit saturation factor (Hue/Saturation on red (1, 0, 0) in a 32-bit document,
/// read back from the saved float TIFF, exact in 1/512): red's R is 0.5 + 0.5 x the factor.
#[test]
fn thirty_two_bit_saturation_steps_match_photoshop() {
    for (sat, want) in [
        (10.0, 539.0 / 512.0),
        (75.0, 2.4609375),
        (90.0, 2673.0 / 512.0),
        (99.0, 32.375),
        (100.0, 33023.0 / 512.0),
        (-10.0, 487.0 / 512.0),
        (-25.0, 0.875),
        (-50.0, 0.75),
        (-100.0, 0.5),
    ] {
        let adj = Adjustment::HueSaturation { hue: 0.0, saturation: sat, lightness: 0.0, colorize: false, ranges: HueRange::defaults() };
        let got = hdr(&adj, [1.0, 0.0, 0.0])[0];
        assert!((got - want).abs() < 1e-4, "saturation {sat}: {got}, Photoshop {want}");
    }
}

/// 32-bit colours at or above HSL lightness 1 (red at +1 stop, (2, 0, 0)) keep their hue and
/// get their chroma scaled like any other, as in Photoshop; they used to turn NaN or grey.
#[test]
fn thirty_two_bit_hue_saturation_keeps_bright_colours() {
    let pale = [280.0 / 255.0, 230.0 / 255.0, 256.0 / 255.0];
    let near = |got: [f32; 3], want: [f32; 3]| got.iter().zip(want).all(|(g, w)| (g - w).abs() <= 1e-3 * w.abs().max(1.0));
    for (hue, sat, c, want) in [
        (40.0, 0.0, [2.0, 0.0, 0.0], [2.0, 1.33301, 0.0]),
        (40.0, 0.0, pale, [1.09804, 0.930687, 0.901961]),
        (0.0, 25.0, [2.0, 0.0, 0.0], [2.32812, -0.328125, -0.328125]),
        (0.0, 25.0, pale, [1.13021, 0.869792, 1.00521]),
        (0.0, -25.0, [2.0, 0.0, 0.0], [1.75, 0.25, 0.25]),
        (0.0, 100.0, [140.0 / 255.0, 115.0 / 255.0, 128.0 / 255.0], [6.77432, -5.77432, 0.750976]),
        (0.0, 100.0, [2.0, 0.0, 0.0], [128.996, -126.996, -126.996]),
    ] {
        let adj = Adjustment::HueSaturation { hue, saturation: sat, lightness: 0.0, colorize: false, ranges: HueRange::defaults() };
        let got = hdr(&adj, c);
        assert!(near(got, want), "hue {hue}, saturation {sat}, {c:?}: {got:?}, Photoshop {want:?}");
    }
    // A master plus a range edit that add up to a whole step stay on it: Reds +96 over a master
    // -46 scales red as much as a master +50 alone (in f32 the sum is 0.49999997).
    let mut ranges = HueRange::defaults();
    ranges[0].saturation = 96.0;
    let summed = Adjustment::HueSaturation { hue: 0.0, saturation: -46.0, lightness: 0.0, colorize: false, ranges };
    let single = Adjustment::HueSaturation { hue: 0.0, saturation: 50.0, lightness: 0.0, colorize: false, ranges: HueRange::defaults() };
    assert_eq!(hdr(&summed, [1.0, 0.0, 0.0]), hdr(&single, [1.0, 0.0, 0.0]));
    // Lightness 0 with a negative channel divides by zero the same way.
    let adj = Adjustment::HueSaturation { hue: 40.0, saturation: 25.0, lightness: 0.0, colorize: false, ranges: HueRange::defaults() };
    assert!(hdr(&adj, [0.25, -0.25, 0.0]).iter().all(|v| v.is_finite()));
}

/// Photoshop's own 32-bit results (photoshop corpus rgb32): nothing clips, legacy contrast
/// pivots at 127, the mixer cuts its percentages to 1/1024 and counts 255/512 of the
/// constant, and the Black & White tint keeps values above 1.
#[test]
fn thirty_two_bit_adjustments_match_photoshop_unclipped() {
    let near = |got: [f32; 3], want: [f32; 3], tol: f32| got.iter().zip(want).all(|(g, w)| (g - w).abs() <= tol);
    let bc = Adjustment::BrightnessContrast { brightness: 25.0, contrast: 30.0, legacy: true };
    // brightness-contrast-legacy.psd
    assert!(near(hdr(&bc, [0.4910, 0.0009, 0.9301]), [0.6280, -0.0721, 1.2553], 2e-4), "{:?}", hdr(&bc, [0.4910, 0.0009, 0.9301]));
    // channel-mixer.psd: pure red, green, blue and black, exactly.
    let mixer = Adjustment::ChannelMixer { matrix: [[0.8, 0.3, -0.1, 0.0], [0.1, 0.9, 0.0, 0.05], [0.0, 0.2, 0.7, 0.0]], monochrome: false };
    for (c, want) in [
        ([1.0, 0.0, 0.0], [0.7998047, 0.1245117, 0.0]),
        ([0.0, 1.0, 0.0], [0.2998047, 0.9243164, 0.1992188]),
        ([0.0, 0.0, 1.0], [-0.0996094, 0.0249023, 0.6992188]),
        ([0.0, 0.0, 0.0], [0.0, 0.0249023, 0.0]),
    ] {
        assert!(near(hdr(&mixer, c), want, 1e-6), "{c:?}: {:?}", hdr(&mixer, c));
    }
    // black-white-tint.psd: white tints past 1.
    let bw = Adjustment::BlackWhite { weights: [40.0, 60.0, 40.0, 60.0, 20.0, 80.0], tint: Some([0.88235295, 0.827451, 0.7019608]) };
    assert!(near(hdr(&bw, [1.0; 3]), [1.0522, 0.9973, 0.8718], 2e-4), "{:?}", hdr(&bw, [1.0; 3]));
    // 8/16-bit documents still clip.
    let mut b = Buffer::filled(crate::Rect::new(0, 0, 1, 1), [1.0, 1.0, 1.0, 1.0]);
    apply_depth(&bw, &mut b, Transfer::Srgb, None);
    assert!(b.px[0].iter().all(|v| *v <= 1.0));
}

#[test]
fn legacy_brightness_contrast_matches_photoshop() {
    const X: [u8; 17] = [0, 16, 32, 48, 64, 80, 96, 112, 128, 144, 160, 176, 192, 208, 224, 240, 255];
    let measured: [(f32, f32, [u8; 17]); 7] = [
        (40.0, 0.0, [40, 56, 72, 88, 104, 120, 136, 152, 168, 184, 200, 216, 232, 248, 255, 255, 255]),
        (-40.0, 0.0, [0, 0, 0, 8, 24, 40, 56, 72, 88, 104, 120, 136, 152, 168, 184, 200, 215]),
        (0.0, 30.0, [0, 0, 0, 14, 37, 60, 82, 105, 128, 151, 174, 196, 219, 242, 255, 255, 255]),
        (0.0, -30.0, [38, 49, 61, 72, 83, 94, 106, 117, 128, 139, 150, 162, 173, 184, 195, 207, 217]),
        (40.0, 30.0, [3, 25, 48, 71, 94, 117, 139, 162, 185, 208, 231, 253, 255, 255, 255, 255, 255]),
        (100.0, 0.0, [100, 116, 132, 148, 164, 180, 196, 212, 228, 244, 255, 255, 255, 255, 255, 255, 255]),
        (0.0, 80.0, [0, 0, 0, 0, 0, 0, 0, 48, 128, 208, 255, 255, 255, 255, 255, 255, 255]),
    ];
    for (brightness, contrast, want) in measured {
        let rect = Rect::new(0, 0, 17, 1);
        let px = X.iter().map(|x| [f32::from(*x) / 255.0; 4]).map(|p| [p[0], p[1], p[2], 1.0]).collect();
        let mut b = Buffer { rect, px };
        apply(&Adjustment::BrightnessContrast { brightness, contrast, legacy: true }, &mut b);
        for (i, (p, w)) in b.px.iter().zip(want).enumerate() {
            let got = (p[0] * 255.0).round() as i32;
            assert!((got - i32::from(w)).abs() <= 1, "b {brightness} c {contrast} at {}: {got} vs Photoshop {w}", X[i]);
        }
    }
}

/// Pixel → Photoshop's split. 8-bit from psd-tools threshold_rgb.psd: the first three are
/// white though their Rec. 601 grey rounds below 60, and (131, 109, 99) sits exactly on 114.5
/// at level 115 (halves round down). 16-bit from threshold_16bits_rgb.psd and the photoshop
/// corpus's rgb16 threshold-128.psd, whose mid grey 16448 is level 128 in 16 bits.
#[test]
fn threshold_splits_on_the_luminosity_like_photoshop() {
    let c8 = |r: u8, g: u8, b: u8| [r, g, b].map(|v| f32::from(v) / 255.0);
    for (c, level, white) in
        [(c8(87, 51, 31), 60u8, true), (c8(90, 49, 33), 60, true), (c8(68, 57, 50), 60, true), (c8(48, 63, 72), 60, false), (c8(131, 109, 99), 115, false)]
    {
        assert_eq!(threshold_white(c, f32::from(level) / 255.0, Some(SampleType::U8)), white, "8-bit {c:?} at {level}");
    }
    let c16 = |r: u16, g: u16, b: u16| [r, g, b].map(|v| f32::from(v) / 32768.0);
    for (c, level, white) in [
        (c16(8995, 7196, 6425), 60u8, false),
        (c16(11180, 6554, 3984), 60, false),
        (c16(16448, 16448, 16448), 128, true),
        (c16(16447, 16447, 16447), 128, false),
    ] {
        assert_eq!(threshold_white(c, f32::from(level) / 255.0, Some(SampleType::U16)), white, "16-bit {c:?} at {level}");
    }
    // Through the adjustment: neutral black or white.
    let mut b = Buffer::filled(Rect::new(0, 0, 1, 1), [87.0 / 255.0, 51.0 / 255.0, 31.0 / 255.0, 1.0]);
    apply_depth(&Adjustment::Threshold { level: 60.0 / 255.0 }, &mut b, Transfer::Srgb, Some(SampleType::U8));
    assert_eq!(b.px[0], [1.0, 1.0, 1.0, 1.0]);
}

/// 16-bit sample → Photoshop's 8-bit level, from psd-tools posterize_16bits_rgb.psd: bins
/// split on the 16-bit sample (10794 and 10923 straddle a third of 32768), and the middle of 3
/// levels is 128 where 8 bits give 127, while 21 levels keep 25 for 25.5.
#[test]
fn sixteen_bit_posterize_matches_photoshop() {
    let points: [(u32, u16, u8); 9] =
        [(3, 10794, 0), (3, 10923, 128), (3, 21845, 128), (3, 21974, 255), (13, 2504, 0), (13, 2542, 21), (13, 7453, 42), (13, 7582, 64), (21, 3341, 25)];
    for (levels, x, want) in points {
        let got = posterize_in(f32::from(x) / 32768.0, levels, Some(SampleType::U16));
        assert!((got * 255.0 - f32::from(want)).abs() < 1e-3, "{levels} levels at {x}: {} vs Photoshop {want}", got * 255.0);
    }
    // 8 bits keep their rule.
    assert_eq!(posterize_in(128.0 / 255.0, 3, Some(SampleType::U8)), posterize(128.0 / 255.0, 3));
}

#[test]
fn color_lookup_with_an_overflowing_stored_size_is_the_identity() {
    // A stored document's size is untrusted: (2²²)³ × 3 overflows, which once wrapped past
    // the table-length guard and indexed out of bounds.
    for size in [1 << 22, u32::MAX] {
        let a = Adjustment::ColorLookup { name: "x".into(), lut: Some(std::sync::Arc::new(vec![0.5; 24])), size, tetrahedral: false, dither: false };
        let mut b = ramp();
        apply(&a, &mut b);
        assert_eq!(b.px, ramp().px, "size {size}");
    }
}

/// Photoshop 27.11, measured: Color Balance with one cyan–red slider moved, over a gray ramp
/// (x = 0, 16, …, 240, 255); red and green out (blue follows green on gray).
#[test]
fn color_balance_matches_photoshop() {
    const X: [u8; 17] = [0, 16, 32, 48, 64, 80, 96, 112, 128, 144, 160, 176, 192, 208, 224, 240, 255];
    type Row = (&'static str, f32, bool, [u8; 17], [u8; 17]);
    let measured: [Row; 6] = [
        ("midtones", 50.0, false, [0, 36, 59, 78, 96, 112, 128, 142, 156, 170, 183, 196, 209, 221, 233, 244, 255], X),
        ("shadows", -100.0, false, [0, 0, 0, 0, 0, 0, 0, 7, 23, 43, 67, 93, 122, 154, 186, 221, 255], X),
        ("highlights", 100.0, false, [0, 51, 84, 111, 136, 160, 182, 202, 223, 242, 255, 255, 255, 255, 255, 255, 255], X),
        ("shadows", 50.0, true, X, [0, 0, 0, 0, 17, 37, 57, 77, 97, 117, 137, 157, 177, 197, 216, 236, 255]),
        (
            "midtones",
            100.0,
            true,
            [0, 36, 59, 78, 96, 112, 128, 142, 156, 170, 183, 196, 209, 221, 233, 244, 255],
            [0, 5, 14, 24, 36, 50, 64, 80, 97, 114, 132, 151, 171, 191, 212, 234, 255],
        ),
        ("highlights", -100.0, true, X, [0, 26, 53, 79, 105, 132, 158, 184, 211, 237, 255, 255, 255, 255, 255, 255, 255]),
    ];
    // Two sliders in one tone, Preserve Luminosity on (measured): shadows red and blue +50 darken
    // only green; highlights red −50 with green +50 lift green by 100 and blue by 50.
    let two =
        |tones: [[f32; 3]; 3], x: f32| -> [i32; 3] { std::array::from_fn(|ch| (color_balance_channel(tones, true, ch, x / 255.0) * 255.0).round() as i32) };
    assert_eq!(two([[50.0, 0.0, 50.0], [0.0; 3], [0.0; 3]], 128.0), [128, 97, 128]);
    let hi = two([[0.0; 3], [0.0; 3], [-50.0, 50.0, 0.0]], 128.0);
    assert!((hi[0] - 128).abs() <= 1 && (hi[1] - 211).abs() <= 1 && (hi[2] - 159).abs() <= 1, "{hi:?}");
    for (tone, amount, preserve, red, green) in measured {
        let mut tones = [[0.0f32; 3]; 3];
        tones[["shadows", "midtones", "highlights"].iter().position(|t| *t == tone).unwrap()][0] = amount;
        for (i, x) in X.iter().enumerate() {
            let v = f32::from(*x) / 255.0;
            let got = |ch| (color_balance_channel(tones, preserve, ch, v) * 255.0).round() as i32;
            let (r, g) = (got(0), got(1));
            assert!((r - i32::from(red[i])).abs() <= 1, "{tone} {amount} pl {preserve} at {x}: red {r} vs Photoshop {}", red[i]);
            assert!((g - i32::from(green[i])).abs() <= 1, "{tone} {amount} pl {preserve} at {x}: green {g} vs Photoshop {}", green[i]);
        }
    }
}

/// Photoshop, measured (8-bit gray ramp, every channel): whole settings, where the tones fold
/// into one Levels per channel (#3423). The issue's midtones red +100 / magenta −100 (the
/// shadow toe), its sequoia14 layer with Preserve Luminosity off and on, all nine sliders with
/// Preserve Luminosity on, a black point past the white point, and lowered shadows with raised
/// highlights (their gammas cancel: a straight line between the two points).
#[test]
fn color_balance_whole_settings_match_photoshop() {
    const X: [u8; 12] = [0, 4, 8, 16, 32, 64, 96, 128, 160, 192, 224, 255];
    type Case = ([[f32; 3]; 3], bool, [[u8; 12]; 3]);
    let cases: [Case; 6] = [
        (
            [[0.0; 3], [100.0, -100.0, 0.0], [0.0; 3]],
            false,
            [[0, 16, 30, 55, 90, 128, 156, 181, 202, 221, 239, 255], [0, 0, 0, 1, 4, 16, 36, 64, 100, 145, 197, 255], X],
        ),
        (
            [[-6.0, 22.0, 5.0], [-1.0, -3.0, -8.0], [-1.0, -4.0, -6.0]],
            false,
            [
                [0, 0, 2, 9, 25, 56, 89, 122, 156, 188, 222, 255],
                [0, 5, 9, 18, 35, 67, 100, 131, 163, 194, 225, 255],
                [0, 3, 7, 14, 29, 60, 91, 123, 156, 189, 222, 255],
            ],
        ),
        (
            [[-6.0, 22.0, 5.0], [-1.0, -3.0, -8.0], [-1.0, -4.0, -6.0]],
            true,
            [
                [0, 0, 0, 0, 5, 42, 80, 117, 154, 189, 226, 255],
                [0, 4, 8, 16, 33, 66, 98, 130, 162, 195, 226, 255],
                [0, 0, 0, 0, 15, 48, 82, 116, 151, 186, 221, 255],
            ],
        ),
        (
            [[40.0, -30.0, 20.0], [-50.0, 25.0, 60.0], [30.0, -70.0, 10.0]],
            true,
            [
                [0, 1, 3, 9, 25, 69, 126, 193, 255, 255, 255, 255],
                [0, 0, 0, 0, 0, 0, 46, 93, 136, 177, 217, 255],
                [0, 0, 0, 0, 44, 107, 156, 199, 238, 255, 255, 255],
            ],
        ),
        (
            [[100.0, -100.0, 0.0], [0.0; 3], [-100.0, 100.0, 0.0]],
            true,
            [X, [0, 0, 0, 0, 0, 255, 255, 255, 255, 255, 255, 255], [0, 0, 0, 0, 0, 0, 0, 130, 255, 255, 255, 255]],
        ),
        (
            [[-76.0, -2.0, 60.0], [0.0; 3], [99.0, 0.0, -30.0]],
            false,
            [
                [0, 0, 0, 0, 0, 0, 71, 171, 255, 255, 255, 255],
                [0, 2, 6, 14, 29, 61, 94, 126, 158, 191, 224, 255],
                [0, 6, 11, 21, 39, 73, 106, 137, 168, 197, 227, 255],
            ],
        ),
    ];
    for (tones, preserve, want) in cases {
        for (ch, row) in want.iter().enumerate() {
            for (x, w) in X.iter().zip(row) {
                let got = (color_balance_channel(tones, preserve, ch, f32::from(*x) / 255.0) * 255.0).round() as i32;
                assert!((got - i32::from(*w)).abs() <= 1, "{tones:?} pl {preserve} ch {ch} at {x}: {got} vs Photoshop {w}");
            }
        }
    }
    // Hostile input stays in range.
    for v in [f32::NAN, -1.0, 2.0, f32::INFINITY] {
        let out = color_balance_channel([[f32::NAN, 1e9, -1e9]; 3], true, 7, v);
        assert!((0.0..=1.0).contains(&out), "{v}: {out}");
    }
}

#[test]
fn color_balance_midtones_with_multiple_sliders_preserve_luminosity() {
    // A shared offset depends on the two extremes, not the sum of moved sliders.
    // The old per-slider product leaves red/blue unchanged for [30, 0, 30].
    let tones = [[0.0; 3], [30.0, 0.0, 30.0], [0.0; 3]];
    for input in [0.25_f32, 0.5, 0.75] {
        for (ch, offset) in [-0.15_f32, 0.15, -0.15].into_iter().enumerate() {
            let expected = input.powf(2f32.powf(offset));
            let actual = color_balance_channel(tones, true, ch, input);
            assert!((actual - expected).abs() < 1e-6, "channel {ch}, input {input}: {actual} vs {expected}");
        }
        // Preserve Luminosity off must still affect only the moved channels.
        assert_eq!(color_balance_channel(tones, false, 1, input), input);
        // One moved slider retains its previous half-strength compensation.
        let single = [[0.0; 3], [30.0, 0.0, 0.0], [0.0; 3]];
        assert!((color_balance_channel(single, true, 0, input) - input.powf(2f32.powf(-0.15))).abs() < 1e-6);
        assert!((color_balance_channel(single, true, 1, input) - input.powf(2f32.powf(0.15))).abs() < 1e-6);
    }
}

#[test]
fn color_balance_curve_matches_the_per_sample_formula_bitwise() {
    // The curve works out its sample-independent terms once; this is the formula it replaced,
    // evaluated whole for every sample.
    fn per_sample(tones: [[f32; 3]; 3], preserve_luminosity: bool, ch: usize, v: f32) -> f32 {
        let [shadows, midtones, highlights] = tones.map(|t| t.map(|s| if s.is_finite() { s.clamp(-100.0, 100.0) } else { 0.0 }));
        let own = |t: [f32; 3]| t.get(ch).copied().unwrap_or(0.0);
        let max = |t: [f32; 3]| t[0].max(t[1]).max(t[2]);
        let min = |t: [f32; 3]| t[0].min(t[1]).min(t[2]);
        let (black, white, m) = if preserve_luminosity {
            (max(shadows) - own(shadows), own(highlights) - min(highlights), own(midtones) - (max(midtones) + min(midtones)) / 2.0)
        } else {
            ((-own(shadows)).max(0.0), own(highlights).max(0.0), own(midtones) + (own(shadows) + own(highlights)) / 2.0)
        };
        let (b, w) = (black / 255.0, 1.0 - white / 255.0);
        let v = if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 };
        if v >= w {
            return 1.0;
        }
        if v <= b {
            return 0.0;
        }
        let (u, gamma) = ((v - b) / (w - b), 2f32.powf(m / 100.0));
        let e = 1.0 / gamma.max(0.01);
        let power = u.powf(e);
        if gamma <= 1.0 {
            return power;
        }
        let s0 = 2f32.powf(gamma);
        let u0 = (2.0 * s0.powf(1.0 / (e - 1.0))).min(1.0);
        if u0.is_nan() || u0 <= 0.0 || u >= u0 {
            return power;
        }
        let (p, d, t) = (u0.powf(e), e * u0.powf(e - 1.0), u / u0);
        let (t2, t3) = (t * t, t * t * t);
        ((t3 - 2.0 * t2 + t) * u0 * s0 + (3.0 * t2 - 2.0 * t3) * p + (t3 - t2) * u0 * d).clamp(0.0, 1.0)
    }
    let mut seed = 0x9e37_79b9_u32;
    let mut slider = || {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        (seed % 241) as f32 - 120.0
    };
    let mut settings = vec![[[0.0; 3]; 3], [[f32::NAN, 1e9, -1e9]; 3], [[-100.0; 3], [100.0; 3], [100.0; 3]]];
    settings.extend((0..120).map(|_| std::array::from_fn(|_| std::array::from_fn(|_| slider()))));
    let samples: Vec<f32> = (0..=1024).map(|i| i as f32 / 1024.0).chain([-0.5, 1.5, f32::NAN, f32::INFINITY]).collect();
    for tones in settings {
        for preserve in [false, true] {
            for ch in [0, 1, 2, 7] {
                let curve = BalanceCurve::new(tones, preserve, ch);
                for &v in &samples {
                    let (got, want) = (curve.eval(v), per_sample(tones, preserve, ch, v));
                    assert_eq!(got.to_bits(), want.to_bits(), "{tones:?} {preserve} ch {ch} v {v}: {got} vs {want}");
                }
            }
        }
    }
}

#[test]
fn levels_and_curves_apply_each_channel_before_the_master() {
    // These two affine tone adjustments describe the same mapping:
    // channel(x) = 1 / 2 + x / 2, then master(x) = x / 2, so 0 maps to 0.25.
    // Levels used to apply the master first (0 mapped to 0.5).
    let line = |a: f32, b: f32| vec![CurvePoint { input: 0.0, output: a }, CurvePoint { input: 1.0, output: b }];
    let levels = Adjustment::Levels {
        master: LevelsChannel { out_white: 0.5, ..Default::default() },
        per_channel: std::array::from_fn(|_| LevelsChannel { out_black: 0.5, ..Default::default() }),
        space: ToneSpace::Rgb,
        black: Default::default(),
    };
    let curves = Adjustment::Curves {
        master: line(0.0, 0.5),
        per_channel: std::array::from_fn(|_| line(0.5, 1.0)),
        space: ToneSpace::Rgb,
        black: Vec::new(),
        pencil: false,
    };
    for depth in [None, Some(SampleType::U8), Some(SampleType::U16), Some(SampleType::F32)] {
        let ll = tone_luts_depth(&levels, depth);
        let cl = tone_luts_depth(&curves, depth);
        for channel in 0..3 {
            for index in [0, LUT_SIZE / 4, LUT_SIZE / 2, 3 * LUT_SIZE / 4, LUT_SIZE - 1] {
                assert!(
                    (ll[channel][index] - cl[channel][index]).abs() < 0.01,
                    "{depth:?} channel {channel} sample {index}: Levels {} vs Curves {}",
                    ll[channel][index],
                    cl[channel][index]
                );
            }
        }
        assert!((cl[0][0] - 0.25).abs() < 0.01, "{depth:?}: {}", cl[0][0]);

        // Exercise the compositor's real adjustment path, not only its LUT builder.
        for adj in [&levels, &curves] {
            let mut pixel = Buffer { rect: Rect::new(0, 0, 1, 1), px: vec![[0.0, 0.0, 0.0, 1.0]] };
            apply_depth(adj, &mut pixel, Transfer::Srgb, depth);
            for channel in 0..3 {
                assert!((pixel.px[0][channel] - 0.25).abs() < 0.01, "{depth:?}: {adj:?} rendered {:?}", pixel.px[0]);
            }
        }
    }

    // Lab has no composite/master tone record.
    let mut lab = curves.clone();
    let mut lab_without_master = curves;
    if let Adjustment::Curves { space, .. } = &mut lab {
        *space = ToneSpace::Lab;
    }
    if let Adjustment::Curves { master, space, .. } = &mut lab_without_master {
        *space = ToneSpace::Lab;
        *master = line(0.0, 1.0);
    }
    assert_eq!(tone_luts(&lab), tone_luts(&lab_without_master));
}

#[test]
fn identity_tone_in_every_space_is_identity() {
    for space in [ToneSpace::Rgb, ToneSpace::Cmyk, ToneSpace::Lab] {
        let mut a = Adjustment::identity_curves();
        if let Adjustment::Curves { space: s, .. } = &mut a {
            *s = space;
        }
        let mut b = ramp();
        apply(&a, &mut b);
        for (x, y) in b.px.iter().zip(ramp().px.iter()) {
            for k in 0..3 {
                assert!((x[k] - y[k]).abs() < 0.02, "{space:?} {x:?} {y:?}");
            }
        }
    }
}

#[test]
fn cmyk_black_curve_darkens_and_lab_lightness_lifts() {
    let line = |a: f32, b: f32| vec![CurvePoint { input: 0.0, output: a }, CurvePoint { input: 1.0, output: b }];
    // Black brightness 1 -> 0.5: more black ink everywhere.
    let a = Adjustment::Curves {
        master: line(0.0, 1.0),
        per_channel: [line(0.0, 1.0), line(0.0, 1.0), line(0.0, 1.0)],
        space: ToneSpace::Cmyk,
        black: line(0.0, 0.5),
        pencil: false,
    };
    let mut b = Buffer { rect: Rect::new(0, 0, 1, 1), px: vec![[0.8, 0.8, 0.8, 1.0]] };
    apply(&a, &mut b);
    assert!(b.px[0][0] < 0.7, "{:?}", b.px[0]);
    // Lab lightness 0..1 -> 0.2..1 lifts a dark grey; a/b untouched keeps it grey.
    let a = Adjustment::Curves {
        master: line(1.0, 0.0), // ignored in Lab
        per_channel: [line(0.2, 1.0), line(0.0, 1.0), line(0.0, 1.0)],
        space: ToneSpace::Lab,
        black: Vec::new(),
        pencil: false,
    };
    let mut b = Buffer { rect: Rect::new(0, 0, 1, 1), px: vec![[0.2, 0.2, 0.2, 1.0]] };
    apply(&a, &mut b);
    let p = b.px[0];
    assert!(p[0] > 0.3 && (p[0] - p[2]).abs() < 0.02, "{p:?}");
}

#[test]
fn hue_ranges_only_touch_their_colours() {
    let mut ranges = HueRange::defaults();
    ranges[0].saturation = -100.0; // desaturate reds
    let a = Adjustment::HueSaturation { hue: 0.0, saturation: 0.0, lightness: 0.0, colorize: false, ranges };
    let mut b = Buffer { rect: Rect::new(0, 0, 3, 1), px: vec![[0.9, 0.1, 0.1, 1.0], [0.1, 0.1, 0.9, 1.0], [0.5, 0.5, 0.5, 1.0]] };
    apply(&a, &mut b);
    let red = b.px[0];
    assert!((red[0] - red[1]).abs() < 0.05, "red desaturated {red:?}");
    assert!((b.px[1][2] - 0.9).abs() < 1e-4 && (b.px[1][0] - 0.1).abs() < 1e-4, "blue untouched {:?}", b.px[1]);
    assert_eq!(b.px[2], [0.5, 0.5, 0.5, 1.0]);
}

#[test]
fn colorize_at_zero_saturation_is_grey() {
    // Photoshop: Colorize at 0% saturation leaves no colour, only the pixel's lightness (#2326).
    let px = vec![[0.9, 0.2, 0.1, 1.0], [0.1, 0.6, 0.8, 1.0], [0.5, 0.5, 0.5, 1.0]];
    for depth in [None, Some(SampleType::U8), Some(SampleType::U16), Some(SampleType::F32)] {
        for lightness in [0.0, 30.0, -30.0] {
            let a = Adjustment::HueSaturation { hue: 200.0, saturation: 0.0, lightness, colorize: true, ranges: HueRange::defaults() };
            let mut b = Buffer { rect: Rect::new(0, 0, 3, 1), px: px.clone() };
            apply_depth(&a, &mut b, Transfer::Srgb, depth);
            for p in &b.px {
                assert!((p[0] - p[1]).abs() < 1e-4 && (p[1] - p[2]).abs() < 1e-4, "{depth:?} lightness {lightness}: {p:?}");
            }
        }
        // Low saturations still tint: the old 25% floor no longer hides them.
        let tint = |saturation: f32| {
            let a = Adjustment::HueSaturation { hue: 0.0, saturation, lightness: 0.0, colorize: true, ranges: HueRange::defaults() };
            let mut b = Buffer { rect: Rect::new(0, 0, 1, 1), px: vec![[0.5, 0.5, 0.5, 1.0]] };
            apply_depth(&a, &mut b, Transfer::Srgb, depth);
            b.px[0][0] - b.px[0][2]
        };
        assert!(tint(10.0) > 0.05 && tint(10.0) < tint(25.0), "{depth:?}: 10% {} vs 25% {}", tint(10.0), tint(25.0));
    }
}

/// Photoshop 27.11, measured: Hue/Saturation raising saturation, and Colorize with Lightness.
#[test]
fn hue_saturation_matches_photoshop() {
    // (input, hue, saturation, lightness, colorize, Photoshop's output)
    type Case = ([u8; 3], f32, f32, f32, bool, [u8; 3]);
    let measured: [Case; 8] = [
        ([204, 102, 102], 0.0, 50.0, 0.0, false, [254, 51, 51]),
        ([204, 159, 102], 0.0, 50.0, 0.0, false, [254, 164, 51]),
        ([204, 102, 102], 0.0, -50.0, 0.0, false, [178, 127, 127]),
        ([0, 0, 0], 120.0, 100.0, 30.0, true, [0, 152, 0]),
        ([28, 28, 28], 120.0, 100.0, 30.0, true, [0, 192, 0]),
        ([160, 160, 160], 30.0, 25.0, -40.0, true, [120, 96, 73]),
        ([252, 252, 252], 30.0, 25.0, -40.0, true, [177, 151, 126]),
        ([255, 143, 0], 30.0, 25.0, -40.0, true, [95, 76, 58]),
    ];
    for (input, hue, saturation, lightness, colorize, want) in measured {
        let a = Adjustment::HueSaturation { hue, saturation, lightness, colorize, ranges: HueRange::defaults() };
        let c = input.map(|v| v as f32 / 255.0);
        let mut b = Buffer { rect: Rect::new(0, 0, 1, 1), px: vec![[c[0], c[1], c[2], 1.0]] };
        apply(&a, &mut b);
        for (i, w) in want.iter().enumerate() {
            let got = b.px[0][i] * 255.0;
            assert!((got - *w as f32).abs() <= 2.0, "{input:?} h{hue} s{saturation} l{lightness} colorize {colorize}, channel {i}: {got} vs {w}");
        }
    }
}

/// Photoshop 27.11, measured: a range's Lightness (Blues ±50) moves towards the brightest or darkest channel.
#[test]
fn range_lightness_matches_photoshop() {
    let measured = [
        ([0u8, 29, 115], 0.5, [57u8, 71, 115]),
        ([0, 64, 255], 0.5, [125, 157, 255]),
        ([102, 128, 204], 0.5, [152, 165, 204]),
        ([102, 128, 204], -0.5, [102, 115, 154]),
        ([0, 18, 115], -0.5, [0, 9, 58]),
    ];
    for (input, l, want) in measured {
        let got = range_lightness(input.map(|v| v as f32 / 255.0), l);
        for i in 0..3 {
            assert!((got[i] * 255.0 - want[i] as f32).abs() <= 2.5, "{input:?} channel {i}: {} vs {}", got[i] * 255.0, want[i]);
        }
    }
}

/// Photoshop's own render (merged image of psd-tools `adjustments/huesaturation_lightness_rgb.psd`,
/// layer "Hue/Saturation 1"): a dull magenta gets the whole range edit, as a vivid one does (#4089).
#[test]
fn hue_ranges_apply_in_full_to_dull_colours() {
    let range = |lightness: f32, bounds: [f32; 4]| HueRange { hue: 0.0, saturation: 0.0, lightness, bounds };
    let ranges = [
        range(-11.0, [-57.0, 4.0, 82.0, 175.0]),
        range(100.0, [12.0, 45.0, 75.0, 105.0]),
        HueRange::neutral(2),
        HueRange::neutral(3),
        HueRange::neutral(4),
        range(100.0, [-117.0, -50.0, 39.0, 104.0]),
    ];
    let a = Adjustment::HueSaturation { hue: 0.0, saturation: 0.0, lightness: 17.0, colorize: false, ranges };
    let measured = [([126u8, 97, 120], [148u8, 148, 148]), ([131, 103, 125], [152, 152, 152]), ([95, 69, 90], [122, 122, 122])];
    for (input, want) in measured {
        let c = input.map(|v| v as f32 / 255.0);
        let mut b = Buffer { rect: Rect::new(0, 0, 1, 1), px: vec![[c[0], c[1], c[2], 1.0]] };
        apply(&a, &mut b);
        for (i, w) in want.iter().enumerate() {
            let got = b.px[0][i] * 255.0;
            assert!((got - *w as f32).abs() <= 2.0, "{input:?} channel {i}: {got} vs {w}");
        }
    }
    // Greys have no hue, so the ranges leave them alone: only the master Lightness moves them.
    let mut b = Buffer { rect: Rect::new(0, 0, 1, 1), px: vec![[0.4, 0.4, 0.4, 1.0]] };
    apply(&a, &mut b);
    let p = b.px[0];
    assert!((p[0] - p[1]).abs() < 1e-6 && (p[1] - p[2]).abs() < 1e-6 && (p[0] - (0.4 + 0.6 * 0.17)).abs() < 1e-4, "{p:?}");
}

#[test]
fn black_white_weights() {
    let d = [40.0, 60.0, 40.0, 60.0, 20.0, 80.0];
    assert!((black_white_gray([0.5; 3], &d) - 0.5).abs() < 1e-6, "greys keep their value");
    assert!((black_white_gray([1.0, 0.0, 0.0], &d) - 0.4).abs() < 1e-6);
    assert!((black_white_gray([1.0, 1.0, 0.0], &d) - 0.6).abs() < 1e-6);
    let mut w = d;
    w[0] = 100.0;
    assert!(black_white_gray([1.0, 0.0, 0.0], &w) > black_white_gray([1.0, 0.0, 0.0], &d));
}

/// #8537: 32-bit superwhites stay above 1 (Photoshop 27.11: 2.0 stays 2.0, neutrals cap at
/// 16, negatives go to 0); 8-bit still clips at 1.
#[test]
fn black_white_keeps_32_bit_superwhites() {
    let adj = Adjustment::BlackWhite { weights: [40.0, 60.0, 40.0, 60.0, 20.0, 80.0], tint: None };
    let run = |depth, c: f32| {
        let mut b = Buffer { rect: Rect::new(0, 0, 1, 1), px: vec![[c, c, c, 1.0]] };
        apply_depth(&adj, &mut b, Transfer::Srgb, Some(depth));
        b.px[0][0]
    };
    assert_eq!(run(SampleType::F32, 2.0), 2.0);
    assert_eq!(run(SampleType::F32, 20.0), 16.0);
    assert_eq!(run(SampleType::F32, -0.5), 0.0);
    assert_eq!(run(SampleType::U8, 2.0), 1.0);
}

/// Photoshop 27.11, measured: Black & White (default weights) with Tint over greys. The tint
/// is blended like the Color blend mode: its hue and saturation at the grey's luminosity.
#[test]
fn black_white_tint_matches_photoshop() {
    let weights = [40.0, 60.0, 40.0, 60.0, 20.0, 80.0];
    let measured: [([u8; 3], u8, [u8; 3]); 6] = [
        ([255, 0, 0], 24, [81, 1, 1]),
        ([255, 0, 0], 120, [255, 64, 64]),
        ([255, 0, 0], 240, [255, 235, 235]),
        ([0xe1, 0xd3, 0xb3], 120, [133, 119, 87]),
        ([0xe1, 0xd3, 0xb3], 24, [33, 24, 1]),
        ([0x40, 0x20, 0x10], 120, [144, 112, 96]),
    ];
    for (tint, grey, want) in measured {
        let tint = Some(tint.map(|v| v as f32 / 255.0));
        let mut b = Buffer { rect: Rect::new(0, 0, 1, 1), px: vec![[grey as f32 / 255.0; 4]] };
        b.px[0][3] = 1.0;
        apply(&Adjustment::BlackWhite { weights, tint }, &mut b);
        for (i, w) in want.iter().enumerate() {
            let got = b.px[0][i] * 255.0;
            assert!((got - *w as f32).abs() <= 2.5, "tint {tint:?} grey {grey} channel {i}: {got} vs {w}");
        }
    }
}

#[test]
fn gradient_map_follows_the_w3c_luminosity() {
    // Photoshop's Gradient Map is driven by 0.3 R + 0.59 G + 0.11 B (photoshop corpus rgb8 and
    // rgb16 gradient-map.psd), not the Rec. 601 grey.
    let stops = vec![(0.0, [0.0; 3]), (1.0, [1.0; 3])];
    for (c, t) in [([0.0, 1.0, 0.0], 0.59), ([1.0, 0.0, 0.0], 0.3), ([0.0, 0.0, 1.0], 0.11), ([0.2, 0.6, 0.9], 0.513)] {
        let mut b = Buffer::filled(crate::Rect::new(0, 0, 1, 1), [c[0], c[1], c[2], 1.0]);
        apply(&Adjustment::GradientMap { stops: stops.clone(), reverse: false, dither: false }, &mut b);
        assert!((b.px[0][0] - t).abs() < 1e-5, "{c:?}: {}", b.px[0][0]);
    }
}

#[test]
fn gradient_map_dither_stays_close() {
    let stops = vec![(0.0, [0.0; 3]), (1.0, [1.0; 3])];
    let mut a = ramp();
    let mut b = ramp();
    apply(&Adjustment::GradientMap { stops: stops.clone(), reverse: false, dither: false }, &mut a);
    apply(&Adjustment::GradientMap { stops, reverse: false, dither: true }, &mut b);
    assert!(a.px.iter().zip(&b.px).all(|(x, y)| (x[0] - y[0]).abs() <= 0.5 / 255.0 + 1e-6));
    assert!(a.px.iter().zip(&b.px).any(|(x, y)| x[0] != y[0]));
}

/// `adj` applied in a Lab document to one encoded sRGB colour, returned as (L, a, b).
fn in_lab(adj: &Adjustment, c: [f32; 3]) -> [f32; 3] {
    let mut b = Buffer::filled(crate::Rect::new(0, 0, 1, 1), [c[0], c[1], c[2], 1.0]);
    apply_in(adj, &mut b, Transfer::Srgb, Some(SampleType::U8), ColorMode::Lab);
    srgb_to_lab([b.px[0][0], b.px[0][1], b.px[0][2]])
}

/// A muted, in-gamut colour from (L, a, b).
fn lab(l: f32, a: f32, b: f32) -> [f32; 3] {
    lab_to_srgb([l, a, b])
}

fn near(got: [f32; 3], want: [f32; 3], tol: f32) -> bool {
    got.iter().zip(want).all(|(g, w)| (g - w).abs() <= tol)
}

#[test]
fn lab_invert_flips_l_a_and_b() {
    let got = in_lab(&Adjustment::Invert, lab(30.0, 12.0, -18.0));
    // n -> 1 - n: L -> 100 - L, a -> -a - 1, b -> -b - 1 (mirrored on the 0..255 grid).
    assert!(near(got, [70.0, -13.0, 17.0], 0.6), "{got:?}");
    // On the screen colour it would not be a mirror in Lab.
    let mut rgb = Buffer::filled(crate::Rect::new(0, 0, 1, 1), [0.0, 0.0, 0.0, 1.0]);
    let c = lab(30.0, 12.0, -18.0);
    rgb.px[0] = [c[0], c[1], c[2], 1.0];
    apply(&Adjustment::Invert, &mut rgb);
    assert!(!near(srgb_to_lab([rgb.px[0][0], rgb.px[0][1], rgb.px[0][2]]), got, 1.0));
}

#[test]
fn sixteen_bit_lab_invert_keeps_greys_neutral() {
    // 16-bit Lab Invert mirrors a and b around neutral (photoshop corpus lab16 invert.psd);
    // 8-bit bytes land one step off it.
    let mut b = Buffer::filled(crate::Rect::new(0, 0, 1, 1), [0.0, 0.0, 0.0, 1.0]);
    let c = lab(40.0, 0.0, 0.0);
    b.px[0] = [c[0], c[1], c[2], 1.0];
    apply_in(&Adjustment::Invert, &mut b, Transfer::Srgb, Some(SampleType::U16), ColorMode::Lab);
    let got = srgb_to_lab([b.px[0][0], b.px[0][1], b.px[0][2]]);
    assert!(near(got, [60.0, 0.0, 0.0], 0.05), "{got:?}");
    assert!(near(in_lab(&Adjustment::Invert, c), [60.0, -1.0, -1.0], 0.1));
}

/// `adj` applied in a CMYK document to one encoded sRGB colour.
fn in_cmyk(adj: &Adjustment, c: [f32; 3]) -> [f32; 3] {
    let mut b = Buffer::filled(crate::Rect::new(0, 0, 1, 1), [c[0], c[1], c[2], 1.0]);
    apply_in(adj, &mut b, Transfer::Srgb, Some(SampleType::U8), ColorMode::Cmyk);
    [b.px[0][0], b.px[0][1], b.px[0][2]]
}

#[test]
fn cmyk_invert_posterize_and_threshold_work_on_the_inks() {
    use photocraft_color::convert::{cmyk_to_rgb, rgb_to_cmyk};
    // Paper inverts to all four inks at 100%, not to RGB black (photoshop corpus cmyk8 invert.psd).
    assert!(near(in_cmyk(&Adjustment::Invert, [1.0; 3]), cmyk_to_rgb([1.0; 4]), 2e-3));
    let red = [0.9, 0.2, 0.1];
    assert!(near(in_cmyk(&Adjustment::Invert, red), cmyk_to_rgb(rgb_to_cmyk(red).map(|v| 1.0 - v)), 2e-3));
    let two = cmyk_to_rgb(rgb_to_cmyk(red).map(|v| if v >= 0.5 { 1.0 } else { 0.0 }));
    assert!(near(in_cmyk(&Adjustment::Posterize { levels: 2 }, red), two, 2e-3));
    // Threshold's black is the black ink alone (threshold-128.psd), its white the paper.
    let t = Adjustment::Threshold { level: 0.5 };
    assert!(near(in_cmyk(&t, [0.2; 3]), cmyk_to_rgb([0.0, 0.0, 0.0, 1.0]), 2e-3));
    assert!(near(in_cmyk(&t, [0.8; 3]), cmyk_to_rgb([0.0; 4]), 2e-3));
}

/// CMYK posterizes the stored value 1 - ink: with 3 levels an ink of 85 (stored 170) lands on
/// the middle step, 128 (stored 127), where posterizing the ink would drop it to none (psd-tools
/// posterize_cmyk.psd, levels 3 to 21, exact so).
#[test]
fn cmyk_posterize_works_on_the_stored_values() {
    let mut ink = [85.0 / 255.0; 4];
    stored_channels(&Adjustment::Posterize { levels: 3 }, ColorMode::Cmyk, Some(SampleType::U8), &mut ink);
    assert!(ink.iter().all(|v| (v * 255.0 - 128.0).abs() < 1e-3), "{ink:?}");
    assert_eq!(posterize(85.0 / 255.0, 3), 0.0, "posterizing the ink itself");
}

#[test]
fn cmyk_threshold_compares_the_inks_plain_luma() {
    assert_eq!(ink_luma([0.0; 4]), 1.0);
    assert_eq!(ink_luma([0.0, 0.0, 0.0, 1.0]), 0.0);
    // Two greys either side of level 128 in photoshop corpus cmyk8 threshold-128.psd: black,
    // then white (8-bit inks).
    let at = |i: [f32; 4]| {
        let mut ch = i.map(|v| v / 255.0);
        stored_channels(&Adjustment::Threshold { level: 128.0 / 255.0 }, ColorMode::Cmyk, Some(SampleType::U8), &mut ch);
        ch
    };
    assert_eq!(at([144.0, 122.0, 121.0, 35.0]), [0.0, 0.0, 0.0, 1.0]);
    assert_eq!(at([131.0, 110.0, 110.0, 19.0]), [0.0; 4]);
}

#[test]
fn cmyk_brightness_contrast_curves_each_ink_from_paper() {
    use photocraft_color::convert::{cmyk_to_rgb, rgb_to_cmyk};
    // The tone curve runs on each ink's stored value, 1 - ink (photoshop corpus cmyk8
    // brightness-contrast.psd and -legacy.psd): raising brightness takes ink away, and paper
    // and solid ink stay put.
    let red = [0.9, 0.2, 0.1];
    for legacy in [false, true] {
        let adj = Adjustment::BrightnessContrast { brightness: 25.0, contrast: 30.0, legacy };
        let table = ink_brightness_contrast_lut(25.0, 30.0, legacy);
        let want = cmyk_to_rgb(rgb_to_cmyk(red).map(|v| 1.0 - lut(&table, 1.0 - v)));
        assert!(near(in_cmyk(&adj, red), want, 2e-3), "legacy {legacy}");
        assert!(near(in_cmyk(&adj, [1.0; 3]), [1.0; 3], 2e-3), "legacy {legacy}: paper");
        let solid = cmyk_to_rgb([1.0; 4]);
        assert!(near(in_cmyk(&adj, solid), cmyk_to_rgb(rgb_to_cmyk(solid).map(|v| ink_through(&table, v))), 2e-3));
    }
    // Legacy +25 / +30 on Photoshop's own 16-bit inks (photoshop corpus cmyk16
    // brightness-contrast-legacy.psd): contrast pivots at 128 in ink, 127 stored.
    let legacy = ink_brightness_contrast_lut(25.0, 30.0, true);
    for (ink, ps) in [(139.0, 108.0), (160.0, 137.99), (172.0, 155.13), (183.0, 170.85), (196.0, 189.42), (98.0, 49.42), (128.0, 92.28), (168.0, 149.42)] {
        let got = ink_through(&legacy, ink / 255.0) * 255.0;
        assert!((got - ps).abs() < 0.1, "ink {ink}: {got} vs Photoshop {ps}");
    }
    let brighter = Adjustment::BrightnessContrast { brightness: 60.0, contrast: 0.0, legacy: false };
    let ink = ink_brightness_contrast_lut(60.0, 0.0, false);
    assert!(ink_through(&ink, 0.5) < 0.5 && ink_through(&ink, 0.0) == 0.0 && (ink_through(&ink, 1.0) - 1.0).abs() < 1e-6);
    // On the screen colour it would land elsewhere.
    let mut rgb = Buffer::filled(crate::Rect::new(0, 0, 1, 1), [red[0], red[1], red[2], 1.0]);
    apply(&brighter, &mut rgb);
    assert!(!near(in_cmyk(&brighter, red), [rgb.px[0][0], rgb.px[0][1], rgb.px[0][2]], 0.01));
}

#[test]
fn lab_posterize_steps_l_a_and_b() {
    let got = in_lab(&Adjustment::Posterize { levels: 2 }, lab(70.0, 20.0, -30.0));
    // Two levels: each normalised channel snaps to 0 or 1 (L 100, a and b at +127 / -128).
    assert!(near(got, srgb_to_lab(lab_denorm([1.0, 1.0, 0.0])), 1e-3), "{got:?}");
    let grey = in_lab(&Adjustment::Posterize { levels: 255 }, lab(50.0, 0.0, 0.0));
    assert!(near(grey, [50.0, 0.0, 0.0], 1.1), "255 levels is within a step of the identity: {grey:?}");
}

#[test]
fn lab_threshold_splits_on_l_with_a_neutral_result() {
    let t = Adjustment::Threshold { level: 0.5 };
    // A saturated blue (L ≈ 32) and yellow (L ≈ 97): black and white, nothing in between.
    for (c, white) in [([0.0, 0.0, 1.0], false), ([1.0, 1.0, 0.0], true), (lab(52.0, 40.0, 30.0), true), (lab(48.0, -40.0, 30.0), false)] {
        let mut b = Buffer::filled(crate::Rect::new(0, 0, 1, 1), [c[0], c[1], c[2], 1.0]);
        apply_in(&t, &mut b, Transfer::Srgb, None, ColorMode::Lab);
        let v = if white { 1.0 } else { 0.0 };
        assert_eq!(b.px[0], [v, v, v, 1.0], "{c:?}");
    }
}

#[test]
fn lab_brightness_contrast_moves_l_only() {
    for legacy in [false, true] {
        let adj = Adjustment::BrightnessContrast { brightness: 40.0, contrast: 30.0, legacy };
        let before = [45.0, 15.0, -20.0];
        let got = in_lab(&adj, lab(before[0], before[1], before[2]));
        assert!(got[0] > before[0] + 5.0, "{legacy}: L rises: {got:?}");
        assert!((got[1] - before[1]).abs() < 0.5 && (got[2] - before[2]).abs() < 0.5, "{legacy}: a, b kept: {got:?}");
    }
}

#[test]
fn lab_gradient_map_follows_l() {
    let stops = vec![(0.0, [1.0, 0.0, 0.0]), (1.0, [0.0, 0.0, 1.0])];
    for reverse in [false, true] {
        let map = Adjustment::GradientMap { stops: stops.clone(), reverse, dither: false };
        // A grey and a saturated colour with the same L map to the same colour.
        let grey = in_lab(&map, lab(60.0, 0.0, 0.0));
        let green = in_lab(&map, lab(60.0, -50.0, 40.0));
        assert!(near(grey, green, 1e-3), "{reverse}: {grey:?} vs {green:?}");
        let t = if reverse { 0.4 } else { 0.6 };
        let want = srgb_to_lab([1.0 - t, 0.0, t]);
        assert!(near(grey, want, 0.05), "{reverse}: {grey:?} vs {want:?}");
    }
}

#[test]
fn other_modes_and_adjustments_keep_the_screen_colour_path() {
    let c = [0.2, 0.5, 0.7, 1.0];
    let lab_only = [Adjustment::Invert, Adjustment::Threshold { level: 0.47 }, Adjustment::Posterize { levels: 4 }];
    for adj in lab_only.iter().chain([&Adjustment::HueSaturation { hue: 30.0, saturation: 0.0, lightness: 0.0, colorize: false, ranges: HueRange::defaults() }])
    {
        for mode in [ColorMode::Rgb, ColorMode::Cmyk, ColorMode::Grayscale, ColorMode::Lab] {
            let (mut a, mut b) = (Buffer::filled(crate::Rect::new(0, 0, 1, 1), c), Buffer::filled(crate::Rect::new(0, 0, 1, 1), c));
            apply_in(adj, &mut a, Transfer::Srgb, None, mode);
            apply_depth(adj, &mut b, Transfer::Srgb, None);
            let own = (mode == ColorMode::Lab && lab_channel_adjustment(adj)) || (mode == ColorMode::Cmyk && cmyk_channel_adjustment(adj));
            assert_eq!(a.px == b.px, !own, "{adj:?} in {mode:?}");
        }
    }
    // Transparent pixels are left alone and odd values never panic.
    let mut t = Buffer::filled(crate::Rect::new(0, 0, 2, 1), [f32::NAN, 2.0, -1.0, 0.0]);
    for adj in lab_only.iter().chain([&Adjustment::GradientMap { stops: vec![], reverse: true, dither: true }]) {
        for mode in [ColorMode::Lab, ColorMode::Cmyk] {
            apply_in(adj, &mut t, Transfer::Srgb, None, mode);
            let mut odd = Buffer::filled(crate::Rect::new(0, 0, 1, 1), [f32::NAN, f32::INFINITY, -3.0, 1.0]);
            apply_in(adj, &mut odd, Transfer::Srgb, Some(SampleType::U16), mode);
        }
    }
    assert!(t.px[0][1] == 2.0);
}
