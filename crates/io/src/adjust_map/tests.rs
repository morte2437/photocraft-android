use super::*;

#[test]
fn gradient_map_v3_methods_and_smoothness() {
    let classic = vec![(0.0, [0.0, 0.0, 1.0]), (1.0, [1.0, 1.0, 0.0])];
    let v1 = write_gradient_map(&classic, false, false);
    // Version 3 inserts the interpolation method after reverse/dither.
    let v3 = |m: &[u8; 4]| {
        let mut v = v1.clone();
        v[1] = 3;
        v.splice(4..4, m.iter().copied());
        v
    };
    let at_t = |data: &[u8], t: f32| match parse(b"grdm", data, None, Channels::Rgb) {
        Adjustment::GradientMap { stops, .. } => {
            let i = stops.windows(2).position(|w| w[0].0 <= t && t <= w[1].0).unwrap();
            let (x, y) = (stops[i], stops[i + 1]);
            let u = (t - x.0) / (y.0 - x.0).max(1e-6);
            std::array::from_fn::<f32, 3, _>(|k| x.1[k] + (y.1[k] - x.1[k]) * u)
        }
        other => panic!("{other:?}"),
    };
    let mid = |data: &[u8]| at_t(data, 0.5);
    let c = mid(&v3(b"Gcls"));
    assert!((c[0] - 0.5).abs() < 1e-3 && (c[2] - 0.5).abs() < 1e-3, "classic = sRGB lerp {c:?}");
    let p = mid(&v3(b"Perc"));
    assert!((p[0] - c[0]).abs() > 0.02, "perceptual differs from classic {p:?}");
    // Smoothness 100 % (4096) bends a three-stop ramp.
    let three = vec![(0.0, [0.0; 3]), (0.25, [1.0, 0.0, 0.0]), (0.75, [0.0, 0.0, 1.0]), (1.0, [1.0; 3])];
    let mut smooth = write_gradient_map(&three, false, false);
    // v1 header/name/count + colour stops + transparency stops + expansion count.
    let at = 48 + three.len() * 20;
    smooth[at..at + 2].copy_from_slice(&4096u16.to_be_bytes());
    let (a, b) = (at_t(&write_gradient_map(&three, false, false), 0.4), at_t(&smooth, 0.4));
    assert!((0..3).any(|k| (a[k] - b[k]).abs() > 1e-3), "{a:?} {b:?}");
    // Noise gradients (no colour stops) stay unsupported.
    assert!(matches!(parse(b"grdm", &[0, 1, 0, 0, 0, 0, 0, 0, 0, 0], None, Channels::Rgb), Adjustment::Unsupported { .. }));
}

#[test]
fn pencil_curves_read_and_write_the_map_form() {
    // Photoshop's pencil-drawn curves: map flag 1, then 256 output bytes per channel.
    let mut block = vec![1, 0, 1, 0, 0, 0, 0b1111];
    let maps: [Vec<u8>; 4] = [
        (0..=255u8).rev().collect(),
        (0..=255u8).map(|k| if k < 128 { 0 } else { 255 }).collect(),
        (0..=255u8).collect(),
        (0..=255u8).map(|k| k / 2).collect(),
    ];
    for m in &maps {
        block.extend_from_slice(m);
    }
    let a = parse(b"curv", &block, None, Channels::Rgb);
    let Adjustment::Curves { master, per_channel, pencil, .. } = &a else { panic!("{a:?}") };
    assert!(*pencil);
    assert_eq!(master.len(), CURVE_MAP_LEN);
    assert_eq!((master[0].output, master[255].output), (1.0, 0.0));
    assert_eq!((per_channel[0][127].output, per_channel[0][128].output), (0.0, 1.0));
    assert_eq!(write(&a)[0].1, block);
}

/// A two-stop `grdm` (version 1) whose stops are the colour structures `a` and `b`.
fn grdm_with(a: (u16, [u16; 4]), b: (u16, [u16; 4])) -> Vec<u8> {
    let mut v = write_gradient_map(&[(0.0, [0.0; 3]), (1.0, [1.0; 3])], false, false);
    // Header 4 + name length 4 + "Custom\0" 14 + stop count 2; 20 bytes per stop.
    for (i, (space, c)) in [a, b].into_iter().enumerate() {
        let at = 24 + 20 * i + 8;
        v[at..at + 2].copy_from_slice(&space.to_be_bytes());
        for (k, x) in c.iter().enumerate() {
            v[at + 2 + 2 * k..at + 4 + 2 * k].copy_from_slice(&x.to_be_bytes());
        }
    }
    v
}

fn ends(a: &Adjustment) -> ([f32; 3], [f32; 3]) {
    match a {
        Adjustment::GradientMap { stops, .. } => (stops[0].1, stops[stops.len() - 1].1),
        other => panic!("{other:?}"),
    }
}

#[test]
fn gradient_map_stops_in_every_colour_space() {
    use photocraft_color::{Color, ColorMode};
    let white_rgb = (0, [65535, 65535, 65535, 0]);
    // CMYK, stored inverted: 65535 = no ink. Full K only.
    let k_only = grdm_with((2, [65535, 65535, 65535, 0]), white_rgb);
    let expected = Color { mode: ColorMode::Cmyk, c: [0.0, 0.0, 0.0, 1.0], alpha: 1.0 }.to_rgb();
    for channels in [Channels::Cmyk, Channels::Rgb, Channels::Lab] {
        let a = parse(b"grdm", &k_only, None, channels);
        let (first, last) = ends(&a);
        assert!(first.iter().zip(expected).all(|(x, y)| (x - y).abs() < 1e-5), "{first:?} vs {expected:?}");
        assert!(first[0] < 0.4 && (first[0] - first[2]).abs() < 0.05, "K-only black is a dark neutral: {first:?}");
        assert_eq!(last, [1.0; 3]);
    }
    // Lab: L 50, a 0, b 0 is a neutral mid grey; L 100 white.
    let lab = parse(b"grdm", &grdm_with((7, [5000, 0, 0, 0]), (7, [10000, 0, 0, 0])), None, Channels::Lab);
    let (first, last) = ends(&lab);
    assert!(first.iter().all(|v| (v - 0.466).abs() < 0.01), "{first:?}");
    assert!(last.iter().all(|v| *v > 0.99), "{last:?}");
    // Lab with chroma: a = +60 is reddish (a and b are signed hundredths).
    let red = parse(b"grdm", &grdm_with((7, [5000, 6000, 0, 0]), white_rgb), None, Channels::Lab);
    let (first, _) = ends(&red);
    assert!(first[0] > first[1] + 0.2, "{first:?}");
    // Grayscale: 0..10000 is ink coverage (10000 = black).
    let gray = parse(b"grdm", &grdm_with((8, [10000, 0, 0, 0]), (8, [0, 0, 0, 0])), None, Channels::Gray);
    assert_eq!(ends(&gray), ([0.0; 3], [1.0; 3]));
    // HSB: hue 240 degrees (2/3 of 65535), full saturation and brightness = blue.
    let hsb = parse(b"grdm", &grdm_with((1, [43690, 65535, 65535, 0]), white_rgb), None, Channels::Rgb);
    let (first, _) = ends(&hsb);
    assert!(first[0] < 1e-3 && first[1] < 1e-3 && (first[2] - 1.0).abs() < 1e-3, "{first:?}");
    // Book colours and unknown ids are not guessed: the block stays raw.
    for space in [3u16, 9, 0xFFFF] {
        let d = grdm_with((space, [1, 2, 3, 4]), white_rgb);
        assert!(matches!(parse(b"grdm", &d, None, Channels::Rgb), Adjustment::Unsupported { .. }), "space {space}");
    }
    // Truncated stops never panic.
    for n in 0..k_only.len() {
        let _ = parse(b"grdm", &k_only[..n], None, Channels::Cmyk);
    }
}

#[test]
fn gradient_map_reverse_and_dither_keep_the_stops_block() {
    let original = grdm_with((2, [65535, 65535, 65535, 0]), (7, [10000, 0, 0, 0]));
    let Adjustment::GradientMap { stops, .. } = parse(b"grdm", &original, None, Channels::Cmyk) else { panic!() };
    let mut data = original.clone();
    let target = Adjustment::GradientMap { stops: stops.clone(), reverse: true, dither: true };
    assert!(retarget_gradient_map(&mut data, &target, Channels::Cmyk));
    assert_eq!(&data[2..4], &[1, 1]);
    assert_eq!(data[4..], original[4..], "the stops keep their colour spaces");
    assert_eq!(parse(b"grdm", &data, None, Channels::Cmyk), target);
    // Edited stops are not patched in.
    let mut edited = stops;
    edited[0].1 = [0.5; 3];
    let mut data2 = original.clone();
    let target = Adjustment::GradientMap { stops: edited, reverse: false, dither: false };
    assert!(!retarget_gradient_map(&mut data2, &target, Channels::Cmyk));
    assert_eq!(data2, original);
    assert!(!retarget_gradient_map(&mut [], &target, Channels::Cmyk));
    assert!(!retarget_gradient_map(&mut data2, &Adjustment::Invert, Channels::Cmyk));
}

fn rt(a: Adjustment) {
    let blocks = write(&a);
    assert!(!blocks.is_empty(), "{a:?}");
    let cged = blocks.iter().find(|b| &b.0 == b"CgEd").map(|b| &b.1[..]);
    let back = parse(&blocks[0].0, &blocks[0].1, cged, Channels::Rgb);
    assert_eq!(back, a);
}

/// Photoshop keeps the colorize triplet at byte 4 and the master triplet at byte 10, and fills
/// the inactive one with its defaults: colorize (30, 25, 0), master (0, 0, 0). The colorize hue
/// is stored signed (200 -> -160).
#[test]
fn hue2_writes_each_triplet_in_its_own_slot() {
    let triplets = |a: &Adjustment| -> Vec<i16> {
        let blocks = write(a);
        (0..6).map(|i| bei16(&blocks[0].1, 4 + 2 * i).unwrap_or(i16::MIN)).collect()
    };
    let on = Adjustment::HueSaturation { hue: 200.0, saturation: 40.0, lightness: 10.0, colorize: true, ranges: HueRange::defaults() };
    assert_eq!(triplets(&on), [-160, 40, 10, 0, 0, 0]);
    let off = Adjustment::HueSaturation { hue: -30.0, saturation: 20.0, lightness: 5.0, colorize: false, ranges: HueRange::defaults() };
    assert_eq!(triplets(&off), [30, 25, 0, -30, 20, 5]);
}

#[test]
fn roundtrips() {
    rt(Adjustment::Invert);
    rt(Adjustment::Threshold { level: 128.0 / 255.0 });
    rt(Adjustment::Posterize { levels: 4 });
    rt(Adjustment::BrightnessContrast { brightness: 20.0, contrast: -10.0, legacy: false });
    rt(Adjustment::BrightnessContrast { brightness: -150.0, contrast: 100.0, legacy: true });
    rt(Adjustment::HueSaturation { hue: 30.0, saturation: -20.0, lightness: 5.0, colorize: false, ranges: HueRange::defaults() });
    rt(Adjustment::HueSaturation { hue: 200.0, saturation: 50.0, lightness: 0.0, colorize: true, ranges: HueRange::defaults() });
    let mut ranges = HueRange::defaults();
    ranges[0] = HueRange { hue: 15.0, saturation: -40.0, lightness: 10.0, bounds: [-60.0, -20.0, 10.0, 30.0] };
    ranges[4].saturation = 25.0;
    rt(Adjustment::HueSaturation { hue: -10.0, saturation: 0.0, lightness: 0.0, colorize: false, ranges });
    rt(Adjustment::Exposure { exposure: 1.5, offset: -0.01, gamma: 0.9 });
    let lc = |a: u16, b: u16| LevelsChannel { in_black: f32::from(a) / 255.0, in_white: f32::from(b) / 255.0, gamma: 1.2, out_black: 0.0, out_white: 1.0 };
    rt(Adjustment::Levels { master: lc(10, 240), per_channel: [lc(0, 255), lc(5, 250), lc(20, 200)], space: ToneSpace::Rgb, black: LevelsChannel::default() });
    let pts = |v: &[(u8, u8)]| v.iter().map(|&(i, o)| CurvePoint { input: f32::from(i) / 255.0, output: f32::from(o) / 255.0 }).collect::<Vec<_>>();
    rt(Adjustment::Curves {
        master: pts(&[(0, 0), (128, 150), (255, 255)]),
        per_channel: [pts(&[(0, 10), (255, 255)]), pts(&[(0, 0), (255, 245)]), pts(&[(0, 0), (64, 32), (255, 255)])],
        space: ToneSpace::Rgb,
        black: Vec::new(),
        pencil: false,
    });
    // CMYK ink curves and levels keep their black record.
    let ink = Adjustment::Curves {
        master: pts(&[(0, 0), (255, 255)]),
        per_channel: [pts(&[(0, 0), (255, 255)]), pts(&[(0, 30), (255, 255)]), pts(&[(0, 0), (255, 255)])],
        space: ToneSpace::Cmyk,
        black: pts(&[(0, 0), (128, 100), (255, 255)]),
        pencil: false,
    };
    let b = write(&ink);
    assert_eq!(parse(&b[0].0, &b[0].1, None, Channels::Cmyk), ink);
    let inkl = Adjustment::Levels { master: lc(0, 255), per_channel: [lc(0, 255), lc(5, 250), lc(0, 255)], space: ToneSpace::Cmyk, black: lc(30, 255) };
    let b = write(&inkl);
    assert_eq!(parse(&b[0].0, &b[0].1, None, Channels::Cmyk), inkl);
    rt(Adjustment::Unsupported { psd_key: "selc".into(), raw: vec![1, 2, 3].into() });
    rt(Adjustment::GradientMap { stops: vec![(0.0, [0.0, 0.0, 0.0]), (0.5, [1.0, 0.0, 0.0]), (1.0, [1.0, 1.0, 1.0])], reverse: true, dither: false });
    rt(Adjustment::GradientMap { stops: vec![(0.0, [0.0, 0.0, 0.0]), (1.0, [1.0, 1.0, 1.0])], reverse: false, dither: true });
    rt(Adjustment::SelectiveColor { relative: true, adjustments: std::array::from_fn(|r| [r as f32 * 10.0 - 40.0, 5.0, -100.0, 100.0]) });
    rt(Adjustment::SelectiveColor { relative: false, adjustments: [[0.0; 4]; 9] });
    rt(Adjustment::Vibrance { vibrance: 35.0, saturation: -12.0 });
    rt(Adjustment::Vibrance { vibrance: 0.0, saturation: 0.0 });
    rt(Adjustment::ColorBalance { shadows: [20.0, -10.0, 5.0], midtones: [-15.0, 10.0, 30.0], highlights: [0.0, 5.0, -100.0], preserve_luminosity: true });
    rt(Adjustment::ColorBalance { shadows: [0.0; 3], midtones: [100.0, 0.0, 0.0], highlights: [0.0; 3], preserve_luminosity: false });
    rt(Adjustment::BlackWhite { weights: [40.0, 60.0, 40.0, 60.0, 20.0, 80.0], tint: None });
    rt(Adjustment::BlackWhite { weights: [-200.0, 300.0, 0.0, 10.0, 90.0, 30.0], tint: Some([0.9, 0.7, 0.5]) });
    let q = |v: u16| f32::from(v) / 65535.0;
    rt(Adjustment::PhotoFilter { color: [q(60000), q(30000), q(0)], density: 0.25, preserve_luminosity: true });
    rt(Adjustment::PhotoFilter { color: [q(0), q(40000), q(65535)], density: 1.0, preserve_luminosity: false });
    rt(Adjustment::ChannelMixer { matrix: [[0.5, 0.3, 0.2, 0.0], [0.1, 0.8, 0.1, 0.05], [0.0, 0.2, 0.9, -0.05]], monochrome: false });
    rt(Adjustment::ChannelMixer { matrix: [[0.4, 0.4, 0.2, 0.1], [0.0, 1.0, 0.0, 0.0], [-2.0, 0.0, 2.0, 0.0]], monochrome: true });
    let id = photocraft_cms::lutfile::LutFile::identity(5);
    rt(Adjustment::ColorLookup { name: "Look.cube".into(), lut: Some(std::sync::Arc::new(id.data)), size: 5, tetrahedral: false, dither: true });
}

#[test]
fn selective_color_synthetic_block() {
    // Absolute; reds = (+10, -20, +30, -40), blacks = (0, 0, 0, 25).
    let mut d = vec![0, 1, 0, 1];
    d.extend([0u8; 8]);
    for r in 0..9 {
        let rec: [i16; 4] = match r {
            0 => [10, -20, 30, -40],
            8 => [0, 0, 0, 25],
            _ => [0; 4],
        };
        for x in rec {
            d.extend(x.to_be_bytes());
        }
    }
    let a = parse(b"selc", &d, None, Channels::Rgb);
    let Adjustment::SelectiveColor { relative, adjustments } = &a else { panic!("{a:?}") };
    assert!(!relative);
    assert_eq!(adjustments[0], [10.0, -20.0, 30.0, -40.0]);
    assert_eq!(adjustments[8], [0.0, 0.0, 0.0, 25.0]);
    // Written back byte-exact.
    assert_eq!(write(&a)[0].1, d);
}

#[test]
fn color_lookup_synthetic_block() {
    let cube = b"TITLE \"t\"\nLUT_3D_SIZE 2\n0 0 0\n1 0 0\n0 1 0\n1 1 0\n0 0 1\n1 0 1\n0 1 1\n1 1 1\n";
    let en = |t: &str, val: &str| Value::Enumerated { type_id: photocraft_psd::descriptor::Id::new(t), value: photocraft_psd::descriptor::Id::new(val) };
    let desc = Descriptor::new("null")
        .with("lookupType", en("colorLookupType", "3DLUT"))
        .with("NM  ", Value::Text(photocraft_psd::descriptor::UnicodeString::new_nul("Id")))
        .with("Dthr", Value::Boolean(true))
        .with("LUTFormat", en("LUTFormatType", "LUTFormatCUBE"))
        .with("LUT3DFileData", Value::RawData(cube.to_vec()));
    let mut d = vec![0, 1];
    d.extend(VersionedDescriptor::new(desc).to_bytes());
    let a = parse(b"clrL", &d, None, Channels::Rgb);
    let Adjustment::ColorLookup { name, lut: Some(t), size: 2, dither: true, .. } = &a else { panic!("{a:?}") };
    assert_eq!(name, "Id");
    assert_eq!(&t[..6], &[0.0, 0.0, 0.0, 1.0, 0.0, 0.0]);
    // A profile-based lookup (no table) is preserved raw.
    let mut p = vec![0, 1];
    p.extend(VersionedDescriptor::new(Descriptor::new("null").with("lookupType", en("colorLookupType", "abstractProfile"))).to_bytes());
    assert!(matches!(parse(b"clrL", &p, None, Channels::Rgb), Adjustment::Unsupported { .. }));
    // Each way a lookup can't be read has a reason for the import warning (#1763).
    assert_eq!(unreadable_reason(b"clrL", &d), None);
    assert!(unreadable_reason(b"clrL", &p).unwrap().contains("colour profile"));
    assert!(unreadable_reason(b"clrL", &[0]).unwrap().contains("truncated"));
    assert!(unreadable_reason(b"clrL", &[0, 2, 0, 0]).unwrap().contains("version 2"));
    assert!(unreadable_reason(b"clrL", &[0, 1, 9]).unwrap().contains("unreadable settings"));
    let bad = Descriptor::new("null").with("LUTFormat", en("LUTFormatType", "LUTFormat3DL")).with("LUT3DFileData", Value::RawData(b"not a lut".to_vec()));
    let mut b = vec![0, 1];
    b.extend(VersionedDescriptor::new(bad).to_bytes());
    assert!(matches!(parse(b"clrL", &b, None, Channels::Rgb), Adjustment::Unsupported { .. }));
    assert!(unreadable_reason(b"clrL", &b).unwrap().contains("3DL LUT can't be read"), "{:?}", unreadable_reason(b"clrL", &b));
    assert_eq!(label(b"clrL"), "Color Lookup");
    assert!(matches!(parse(b"clrL", &[0, 1, 9], None, Channels::Rgb), Adjustment::Unsupported { .. }));
    assert!(matches!(parse(b"selc", &[0, 1, 0, 0], None, Channels::Rgb), Adjustment::Unsupported { .. }));
}

fn cube_lookup_block(cube: &[u8]) -> Vec<u8> {
    let en = |t: &str, val: &str| Value::Enumerated { type_id: photocraft_psd::descriptor::Id::new(t), value: photocraft_psd::descriptor::Id::new(val) };
    let desc = Descriptor::new("null")
        .with("LUTFormat", en("LUTFormatType", "LUTFormatCUBE"))
        .with("LUT3DFileName", Value::Text(photocraft_psd::descriptor::UnicodeString::new_nul("domain.cube")))
        .with("LUT3DFileData", Value::RawData(cube.to_vec()));
    let mut block = vec![0, 1];
    block.extend(VersionedDescriptor::new(desc).to_bytes());
    block
}

fn identity_cube(domain: &str) -> Vec<u8> {
    format!("{domain}LUT_3D_SIZE 2\n0 0 0\n1 0 0\n0 1 0\n1 1 0\n0 0 1\n1 0 1\n0 1 1\n1 1 1\n").into_bytes()
}

#[test]
fn color_lookup_custom_domains_are_kept_unsupported() {
    let domains = ["DOMAIN_MIN 0 0 0\nDOMAIN_MAX 0.5 0.5 0.5\n", "DOMAIN_MIN 0 0.25 0\nDOMAIN_MAX 1 0.75 1\n", "LUT_3D_INPUT_RANGE 0 0.5\n"];
    for domain in domains {
        let raw = cube_lookup_block(&identity_cube(domain));
        let Adjustment::Unsupported { psd_key, raw: kept } = parse(b"clrL", &raw, None, Channels::Rgb) else {
            panic!("custom LUT domain was applied instead of kept unsupported: {domain}")
        };
        assert_eq!(psd_key, "clrL");
        assert_eq!(*kept, *raw);
        let reason = unreadable_reason(b"clrL", &raw).expect("custom domain has an explanation");
        assert!(reason.contains("DOMAIN_MIN") && reason.contains("DOMAIN_MAX"), "{reason}");
        assert!(reason.to_ascii_lowercase().contains("not supported"), "{reason}");
    }
}

#[test]
fn color_lookup_default_domains_stay_supported() {
    for domain in ["", "DOMAIN_MIN 0 0 0\nDOMAIN_MAX 1 1 1\n"] {
        let raw = cube_lookup_block(&identity_cube(domain));
        assert!(matches!(parse(b"clrL", &raw, None, Channels::Rgb), Adjustment::ColorLookup { size: 2, .. }));
        assert_eq!(unreadable_reason(b"clrL", &raw), None);
    }
}

#[test]
fn malformed_falls_back_to_unsupported() {
    assert!(matches!(parse(b"levl", &[0, 2, 1], None, Channels::Rgb), Adjustment::Unsupported { .. }));
    assert!(matches!(parse(b"curv", &[0, 0, 9], None, Channels::Rgb), Adjustment::Unsupported { .. }));
    assert!(matches!(parse(b"thrs", &[], None, Channels::Rgb), Adjustment::Unsupported { .. }));
    assert!(matches!(parse(b"blnc", &[0; 18], None, Channels::Rgb), Adjustment::Unsupported { .. }));
    assert!(matches!(parse(b"vibA", &[0, 0, 0, 15, 0], None, Channels::Rgb), Adjustment::Unsupported { .. }));
    assert!(matches!(parse(b"blwh", &[], None, Channels::Rgb), Adjustment::Unsupported { .. }));
    assert!(matches!(parse(b"mixr", &[0, 2, 0, 0], None, Channels::Rgb), Adjustment::Unsupported { .. }));
    // A CMYK mixer (black source used) stays raw in CMYK documents; our own RGB layout maps.
    let mut cmyk = i16s(&[1, 0]);
    for r in 0..4 {
        cmyk.extend(i16s(&std::array::from_fn::<i16, 5, _>(|k| if k == r { 100 } else { 0 })));
    }
    assert!(matches!(parse(b"mixr", &cmyk, None, Channels::Cmyk), Adjustment::Unsupported { .. }));
    assert!(matches!(parse(b"mixr", &cmyk, None, Channels::Other), Adjustment::Unsupported { .. }));
    let ours = write(&Adjustment::ChannelMixer { matrix: [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0]], monochrome: false });
    assert!(matches!(parse(b"mixr", &ours[0].1, None, Channels::Cmyk), Adjustment::ChannelMixer { .. }));
    assert!(matches!(parse(b"mixr", &ours[0].1, None, Channels::Other), Adjustment::ChannelMixer { .. }));
    assert!(matches!(parse(b"mixr", &[0, 1, 0, 0, 0, 100], None, Channels::Rgb), Adjustment::Unsupported { .. }));
    // Photo Filter: unknown version, HSB colour space, out-of-range version-3 XYZ, truncated.
    assert!(matches!(parse(b"phfl", &[0, 4, 0, 0], None, Channels::Rgb), Adjustment::Unsupported { .. }));
    assert!(matches!(parse(b"phfl", &[0, 2, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 25, 1], None, Channels::Rgb), Adjustment::Unsupported { .. }));
    for bad in [[200_000u32, 0, 0], [0, u32::MAX, 0]] {
        let mut xyz = vec![0, 3];
        for x in bad {
            xyz.extend(x.to_be_bytes());
        }
        xyz.extend([0, 0, 0, 25, 1]);
        assert!(matches!(parse(b"phfl", &xyz, None, Channels::Rgb), Adjustment::Unsupported { .. }));
    }
    assert!(matches!(parse(b"phfl", &[0, 2, 0, 0, 0, 0], None, Channels::Rgb), Adjustment::Unsupported { .. }));
    // Arbitrary bytes never panic, whatever the key.
    for key in ADJUSTMENT_KEYS {
        for len in 0..64usize {
            let d: Vec<u8> = (0..len).map(|i| (i * 37 + len) as u8).collect();
            let _ = parse(key, &d, Some(&d), Channels::Rgb);
            let _ = parse(key, &d, None, Channels::Other);
        }
    }
}

fn i16s(v: &[i16]) -> Vec<u8> {
    v.iter().flat_map(|x| x.to_be_bytes()).collect()
}

/// Hand-built blocks laid out per the Adobe spec (no file written by Photoshop involved).
#[test]
fn color_balance_golden_block() {
    let mut d = i16s(&[-30, 20, 0, 10, -5, 100, 0, 0, -100]);
    d.extend([1, 0]);
    let a = parse(b"blnc", &d, None, Channels::Rgb);
    assert_eq!(
        a,
        Adjustment::ColorBalance { shadows: [-30.0, 20.0, 0.0], midtones: [10.0, -5.0, 100.0], highlights: [0.0, 0.0, -100.0], preserve_luminosity: true }
    );
    assert_eq!(write(&a), vec![(*b"blnc", d.clone())]);
    // The 19-byte form (no trailing pad) parses the same.
    assert_eq!(parse(b"blnc", &d[..19], None, Channels::Rgb), a);
}

#[test]
fn photo_filter_golden_blocks() {
    // Version 2, RGB colour structure (65535, 32768, 0), density 40 %, preserve luminosity.
    let mut v2 = i16s(&[2, 0]);
    v2.extend([0xff, 0xff, 0x80, 0x00, 0, 0, 0, 0]);
    v2.extend(40u32.to_be_bytes());
    v2.extend([1, 0, 0, 0]);
    assert_eq!(v2.len(), 20, "Photo Filter must occupy 20 bytes");
    assert_eq!(&v2[17..], &[0, 0, 0], "padding is included inside the block length");
    let a = parse(b"phfl", &v2, None, Channels::Rgb);
    let Adjustment::PhotoFilter { color, density, preserve_luminosity: true } = a else { panic!("{a:?}") };
    assert_eq!(color, [1.0, 32768.0 / 65535.0, 0.0]);
    assert_eq!(density, 0.4);
    assert_eq!(write(&a)[0].1, v2, "version 2 RGB is written back byte-exact");
    // Older PhotoCraft files used only one padding byte (18 bytes); keep importing them,
    // but never produce that Photoshop-incompatible length again.
    assert_eq!(parse(b"phfl", &v2[..18], None, Channels::Rgb), a);
    for preserve_luminosity in [false, true] {
        for density in [0.0, 0.14, 1.0] {
            let adjustment = Adjustment::PhotoFilter { color: [0.8, 0.4, 0.2], density, preserve_luminosity };
            let blocks = write(&adjustment);
            assert_eq!(blocks.len(), 1);
            assert_eq!(blocks[0].0, *b"phfl");
            assert_eq!(blocks[0].1.len(), 20);
            assert_eq!(&blocks[0].1[17..], &[0, 0, 0]);
            let Adjustment::PhotoFilter { color, density: read_density, preserve_luminosity: read_preserve } =
                parse(b"phfl", &blocks[0].1, None, Channels::Rgb)
            else {
                panic!("written phfl must parse")
            };
            assert!(color.iter().zip([0.8, 0.4, 0.2]).all(|(got, expected)| (got - expected).abs() <= 1.0 / 65535.0));
            assert!((read_density - density).abs() < 0.00001);
            assert_eq!(read_preserve, preserve_luminosity);
        }
    }
    // Version 2, Lab colour structure: L 50, a 0, b 0 is mid gray.
    let mut lab = i16s(&[2, 7, 5000, 0, 0, 0]);
    lab.extend(25u32.to_be_bytes());
    lab.push(0);
    let Adjustment::PhotoFilter { color, density, preserve_luminosity: false } = parse(b"phfl", &lab, None, Channels::Rgb) else { panic!() };
    assert!(color.iter().all(|c| (c - 0.466).abs() < 0.01), "{color:?}");
    assert_eq!(density, 0.25);
    // Version 3: XYZ relative to the D50 white, 1.0 = 32768. White is (32768, 32768, 32768).
    let v3 = phfl_v3([32768, 32768, 32768], 100, true);
    let Adjustment::PhotoFilter { color, density, preserve_luminosity: true } = parse(b"phfl", &v3, None, Channels::Rgb) else { panic!() };
    assert!(color.iter().all(|c| (c - 1.0).abs() < 0.005), "{color:?}");
    assert_eq!(density, 1.0);
    // Photoshop's Warming Filter (85) preset, sRGB #EC8A00: the CS2 file of #4940 stores it
    // relative to the D50 white as (15548, 12029, 0).
    let Adjustment::PhotoFilter { color, density, preserve_luminosity: false } = parse(b"phfl", &phfl_v3([15548, 12029, 0], 25, false), None, Channels::Rgb)
    else {
        panic!()
    };
    let expected = [0xecu8, 0x8a, 0x00].map(|v| f32::from(v) / 255.0);
    assert!(color.iter().zip(expected).all(|(got, want)| (got - want).abs() < 1.5 / 255.0), "{color:?}");
    assert_eq!(density, 0.25);
    // The block from #3777 (Photoshop CS2): the Yellow filter preset, #F9E31C, density 63 %.
    let issue = [0, 3, 0, 0, 0x5e, 0x7f, 0, 0, 0x61, 0xaa, 0, 0, 0x0e, 0xf3, 0, 0, 0, 0x3f, 1, 0];
    let Adjustment::PhotoFilter { color, density, preserve_luminosity: true } = parse(b"phfl", &issue, None, Channels::Rgb) else { panic!() };
    let expected = [0xf9u8, 0xe3, 0x1c].map(|v| f32::from(v) / 255.0);
    assert!(color.iter().zip(expected).all(|(got, want)| (got - want).abs() < 1.5 / 255.0), "{color:?}");
    assert_eq!(density, 0.63);
    // Cooling Filter (82) from #4940: a deep blue outside sRGB; its Z stays under the 2.0
    // garbage guard and the colour clamps to (0, 181, 255).
    let Adjustment::PhotoFilter { color: cooling, density, preserve_luminosity: true } =
        parse(b"phfl", &phfl_v3([5327, 9584, 65357], 72, true), None, Channels::Rgb)
    else {
        panic!("Cooling Filter (82) must parse")
    };
    let expected = [0u8, 181, 255].map(|v| f32::from(v) / 255.0);
    assert!(cooling.iter().zip(expected).all(|(got, want)| (got - want).abs() < 1.5 / 255.0), "{cooling:?}");
    assert_eq!(density, 0.72);
    // The 19-byte form (no trailing pad) parses the same.
    assert_eq!(parse(b"phfl", &issue[..19], None, Channels::Rgb), parse(b"phfl", &issue, None, Channels::Rgb));
    // Version 3 is read, never written: an edited filter is saved as version 2 and keeps its colour.
    let a = parse(b"phfl", &issue, None, Channels::Rgb);
    let w = write(&a);
    assert_eq!(&w[0].1[..2], &[0, 2]);
    let Adjustment::PhotoFilter { color: back, .. } = parse(b"phfl", &w[0].1, None, Channels::Rgb) else { panic!() };
    assert!(back.iter().zip(color).all(|(b, c)| (b - c).abs() <= 1.0 / 65535.0));
}

/// A version 3 Photo Filter block: version, three u32 XYZ components, density, flag, pad.
fn phfl_v3(xyz: [u32; 3], density: u32, preserve_luminosity: bool) -> Vec<u8> {
    let mut d = vec![0, 3];
    for x in xyz {
        d.extend(x.to_be_bytes());
    }
    d.extend(density.to_be_bytes());
    d.extend([u8::from(preserve_luminosity), 0]);
    d
}

#[test]
fn channel_mixer_golden_blocks() {
    // RGB: red = 100 % R + 20 % const; green = 50 % R + 50 % G; blue = -50 % G + 150 % B - 10 % const.
    let mut d = i16s(&[1, 0]);
    d.extend(i16s(&[100, 0, 0, 0, 20, 50, 50, 0, 0, 0, 0, -50, 150, 0, -10]));
    let a = parse(b"mixr", &d, None, Channels::Rgb);
    assert_eq!(a, Adjustment::ChannelMixer { matrix: [[1.0, 0.0, 0.0, 0.2], [0.5, 0.5, 0.0, 0.0], [0.0, -0.5, 1.5, -0.1]], monochrome: false });
    // Written with the trailing gray record (a copy of the first row).
    let w = write(&a);
    assert_eq!(w[0].1[..d.len()], d[..]);
    assert_eq!(w[0].1[d.len()..], i16s(&[100, 0, 0, 0, 20])[..]);
    // Monochrome with only the gray record: 30/59/11.
    let mut m = i16s(&[1, 1]);
    m.extend(i16s(&[30, 59, 11, 0, 0]));
    let Adjustment::ChannelMixer { matrix, monochrome: true } = parse(b"mixr", &m, None, Channels::Rgb) else { panic!() };
    assert_eq!(matrix[0], [0.3, 0.59, 0.11, 0.0]);
}

fn vdesc(d: Descriptor) -> Vec<u8> {
    VersionedDescriptor::new(d).to_bytes()
}

#[test]
fn vibrance_golden_block() {
    let d = vdesc(Descriptor::new("null").with("vibrance", Value::Integer(-40)).with("Strt", Value::Integer(25)));
    assert_eq!(&d[..4], &[0, 0, 0, 16], "descriptor version 16");
    assert_eq!(parse(b"vibA", &d, None, Channels::Rgb), Adjustment::Vibrance { vibrance: -40.0, saturation: 25.0 });
    // Photoshop omits zero values.
    let only = vdesc(Descriptor::new("null").with("Strt", Value::Integer(10)));
    assert_eq!(parse(b"vibA", &only, None, Channels::Rgb), Adjustment::Vibrance { vibrance: 0.0, saturation: 10.0 });
    // Other descriptor versions are rejected (kept raw).
    let mut v = d.clone();
    v[3] = 17;
    assert!(matches!(parse(b"vibA", &v, None, Channels::Rgb), Adjustment::Unsupported { .. }));
}

#[test]
fn black_white_golden_block() {
    let tint = Descriptor::new("RGBC").with("Rd  ", Value::Double(255.0)).with("Grn ", Value::Double(127.5)).with("Bl  ", Value::Double(0.0));
    let mut desc = Descriptor::new("null");
    for (k, w) in BW_KEYS.iter().zip([10, 20, 30, 40, 50, 60]) {
        desc = desc.with(k, Value::Integer(w));
    }
    let d =
        vdesc(desc.clone().with("useTint", Value::Boolean(true)).with("tintColor", Value::Descriptor(tint.clone())).with("bwPresetKind", Value::Integer(1)));
    let a = parse(b"blwh", &d, None, Channels::Rgb);
    assert_eq!(a, Adjustment::BlackWhite { weights: [10.0, 20.0, 30.0, 40.0, 50.0, 60.0], tint: Some([1.0, 0.5, 0.0]) });
    // useTint off: no tint, even with a stored colour.
    let off = vdesc(desc.with("useTint", Value::Boolean(false)).with("tintColor", Value::Descriptor(tint)));
    assert!(matches!(parse(b"blwh", &off, None, Channels::Rgb), Adjustment::BlackWhite { tint: None, .. }));
    // Missing weights take Photoshop's defaults.
    let empty = vdesc(Descriptor::new("null"));
    assert_eq!(parse(b"blwh", &empty, None, Channels::Rgb), Adjustment::BlackWhite { weights: BW_DEFAULT, tint: None });
}

#[test]
fn degenerate_adjustments_still_get_an_encoding() {
    // Gradient maps with fewer than two stops render as gray (none) or one colour.
    for (stops, want) in
        [(vec![], vec![(0.0, [0.0; 3]), (1.0, [1.0; 3])]), (vec![(0.3, [0.2, 0.4, 0.6])], vec![(0.0, [0.2, 0.4, 0.6]), (1.0, [0.2, 0.4, 0.6])])]
    {
        let b = write(&Adjustment::GradientMap { stops, reverse: true, dither: false });
        let back = parse(&b[0].0, &b[0].1, None, Channels::Rgb);
        let Adjustment::GradientMap { stops, reverse: true, .. } = back else { panic!("{back:?}") };
        assert_eq!(stops.len(), want.len());
        for (s, w) in stops.iter().zip(&want) {
            assert!((s.0 - w.0).abs() < 1e-6 && (0..3).all(|k| (s.1[k] - w.1[k]).abs() < 1e-4), "{s:?} vs {w:?}");
        }
    }
    // A colour lookup without a (usable) table becomes an identity cube, without dither.
    for lut in [None, Some(std::sync::Arc::new(vec![0.5; 7]))] {
        let b = write(&Adjustment::ColorLookup { name: "L".into(), lut, size: 3, tetrahedral: false, dither: true });
        let back = parse(&b[0].0, &b[0].1, None, Channels::Rgb);
        let Adjustment::ColorLookup { lut: Some(t), size: 2, dither: false, name, .. } = back else { panic!("{back:?}") };
        assert_eq!(name, "L");
        assert_eq!(*t, photocraft_cms::lutfile::LutFile::identity(2).data);
    }
    // Every kind except a malformed Unsupported key has an encoding.
    assert!(write(&Adjustment::Unsupported { psd_key: "abc".into(), raw: Vec::new().into() }).is_empty());
}

#[test]
fn non_rgb_ignores_channel_records() {
    let lc = LevelsChannel { in_black: 0.1, ..Default::default() };
    let a = Adjustment::Levels {
        master: LevelsChannel::default(),
        per_channel: [lc.clone(), lc.clone(), lc],
        space: ToneSpace::Rgb,
        black: LevelsChannel::default(),
    };
    let b = write(&a);
    match parse(&b[0].0, &b[0].1, None, Channels::Other) {
        Adjustment::Levels { per_channel, .. } => assert_eq!(per_channel, <[LevelsChannel; 3]>::default()),
        other => panic!("{other:?}"),
    }
}

#[test]
fn gray_uses_first_channel_record_for_all() {
    let lc = LevelsChannel { in_black: 44.0 / 255.0, ..Default::default() };
    let a = Adjustment::Levels {
        master: LevelsChannel::default(),
        per_channel: [lc.clone(), Default::default(), Default::default()],
        space: ToneSpace::Rgb,
        black: LevelsChannel::default(),
    };
    let b = write(&a);
    match parse(&b[0].0, &b[0].1, None, Channels::Gray) {
        Adjustment::Levels { per_channel, .. } => assert_eq!(per_channel, [lc.clone(), lc.clone(), lc]),
        other => panic!("{other:?}"),
    }
}

#[test]
fn legacy_brit_without_cged() {
    let a = parse(b"brit", &[0, 10, 0xff, 0xf6, 0, 127, 0], None, Channels::Rgb);
    assert_eq!(a, Adjustment::BrightnessContrast { brightness: 10.0, contrast: -10.0, legacy: true });
}
