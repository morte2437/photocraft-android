use super::*;

fn sample_effects() -> Vec<Effect> {
    let g = Gradient {
        stops: vec![(0.0, Color::rgb(1.0, 0.0, 0.0)), (1.0, Color::rgb(0.0, 0.0, 1.0))],
        opacity_stops: vec![(0.0, 1.0), (1.0, 0.5)],
        ..Gradient::default()
    };
    vec![
        Effect::default_drop_shadow(),
        Effect::DropShadow(Shadow {
            distance: 9.0,
            spread: 0.25,
            contour: Contour::Custom {
                name: "Cone".into(),
                points: vec![CurvePoint { input: 0.0, output: 0.0 }, CurvePoint { input: 0.5, output: 1.0 }, CurvePoint { input: 1.0, output: 0.0 }],
                corners: vec![false, true, false],
            },
            ..match Effect::default_drop_shadow() {
                Effect::DropShadow(s) => s,
                _ => unreachable!(),
            }
        }),
        Effect::InnerShadow(Shadow {
            knocks_out: false,
            ..match Effect::default_drop_shadow() {
                Effect::DropShadow(s) => s,
                _ => unreachable!(),
            }
        }),
        Effect::OuterGlow(Glow {
            common: FxCommon::new(BlendMode::Screen, 0.5),
            paint: FxPaint::Color(Color::rgb(1.0, 1.0, 0.0)),
            technique: GlowTechnique::Precise,
            spread: 0.1,
            size: 8.0,
            contour: Contour::Linear,
            anti_alias: true,
            range: 0.5,
            jitter: 0.0,
            noise: 0.0,
            source: GlowSource::Edge,
        }),
        Effect::InnerGlow(Glow {
            common: FxCommon::new(BlendMode::Screen, 0.75),
            paint: FxPaint::Gradient(g.clone()),
            technique: GlowTechnique::Softer,
            spread: 0.0,
            size: 5.0,
            contour: Contour::Linear,
            anti_alias: false,
            range: 0.5,
            jitter: 0.0,
            noise: 0.0,
            source: GlowSource::Center,
        }),
        Effect::Stroke(StrokeFx {
            common: FxCommon::new(BlendMode::Normal, 1.0),
            size: 3.0,
            position: StrokePosition::Inside,
            paint: FxPaint::Color(Color::rgb(0.0, 1.0, 0.0)),
            overprint: false,
        }),
        Effect::Stroke(StrokeFx {
            common: FxCommon::new(BlendMode::Multiply, 0.5),
            size: 6.0,
            position: StrokePosition::Center,
            paint: FxPaint::Gradient(g.clone()),
            overprint: true,
        }),
        Effect::ColorOverlay { common: FxCommon::new(BlendMode::Overlay, 0.5), color: Color::rgb(0.0, 0.0, 1.0) },
        Effect::GradientOverlay { common: FxCommon::new(BlendMode::Normal, 1.0), gradient: g, dither: true, interpolation: None },
        Effect::PatternOverlay {
            common: FxCommon::new(BlendMode::Normal, 1.0),
            name: "Bubbles".into(),
            id: "abc".into(),
            scale: 1.0,
            angle: 15.0,
            link: false,
            phase: (4.0, 2.0),
        },
        Effect::Satin(Satin {
            common: FxCommon::new(BlendMode::Multiply, 0.5),
            color: Color::BLACK,
            angle: 19.0,
            distance: 11.0,
            size: 14.0,
            contour: Contour::Linear,
            anti_alias: true,
            invert: true,
        }),
        Effect::BevelEmboss(Bevel {
            enabled: true,
            style: BevelStyle::Emboss,
            technique: BevelTechnique::ChiselSoft,
            depth: 1.5,
            up: false,
            size: 7.0,
            soften: 2.0,
            angle: 45.0,
            altitude: 40.0,
            use_global_light: false,
            gloss_contour: Contour::Linear,
            gloss_anti_alias: true,
            highlight: FxCommon::new(BlendMode::Screen, 0.75),
            highlight_color: Color::WHITE,
            shadow: FxCommon::new(BlendMode::Multiply, 0.6),
            shadow_color: Color::BLACK,
            contour: Some(BevelContour { contour: Contour::Linear, range: 0.7, anti_alias: true }),
            texture: Some(BevelTexture { name: "Bubbles".into(), id: "abc".into(), scale: 0.5, depth: -2.0, invert: true, link: false, phase: (3.0, 1.0) }),
        }),
    ]
}

/// #9556: Photoshop drops every effect of a layer whose contour has more than 16 points. A
/// 29-point contour with two rings is written as 16 points that keep both ends and both peaks;
/// up to 16 points are written as they are.
#[test]
fn psd_contours_keep_at_most_16_points_and_their_peaks() {
    let points: Vec<CurvePoint> = (0..29)
        .map(|i| {
            let x = i as f32 / 28.0;
            // Rings at x = 0.25 and 0.75, small wiggles between.
            let y = (1.0 - ((x - 0.25).abs() * 8.0)).max(0.0).max(1.0 - (x - 0.75).abs() * 8.0) + if i % 2 == 0 { 0.0 } else { 0.01 };
            CurvePoint { input: x, output: y.clamp(0.0, 1.0) }
        })
        .collect();
    let c = Contour::Custom { name: "Two rings".into(), points: points.clone(), corners: vec![false; 29] };
    let out = crv(&contour_value(&c));
    assert_eq!(out.len(), 16);
    assert_eq!(out.first().copied(), Some((0.0, f64::from(points[0].output) * 255.0)));
    assert_eq!(out.last().map(|p| p.0), Some(255.0));
    let peak = |x: f32| out.iter().any(|p| ((p.0 / 255.0) as f32 - x).abs() < 0.03 && p.1 > 240.0);
    assert!(peak(0.25) && peak(0.75), "both rings kept: {out:?}");
    assert!(out.windows(2).all(|w| w[0].0 < w[1].0), "in order: {out:?}");

    let short = Contour::Custom { name: "Short".into(), points: points[..16].to_vec(), corners: vec![false; 16] };
    assert_eq!(crv(&contour_value(&short)).len(), 16);
}

fn crv(v: &Value) -> Vec<(f64, f64)> {
    let Value::Descriptor(d) = v else { panic!("ShpC") };
    let Some(Value::List(pts)) = d.get("Crv ") else { panic!("Crv ") };
    pts.iter()
        .map(|p| {
            let Value::Descriptor(p) = p else { panic!("CrPt") };
            let n = |k: &str| match p.get(k) {
                Some(Value::Double(x)) => *x,
                _ => panic!("{k}"),
            };
            (n("Hrzn"), n("Vrtc"))
        })
        .collect()
}

fn approx(a: &[Effect], b: &[Effect]) {
    // Colours go through 0..255 doubles; compare via re-serialization.
    assert_eq!(a.len(), b.len());
    assert_eq!(write_lfx2(true, a), write_lfx2(true, b));
}

/// `approx` compares re-serialized bytes, so a field the writer drops looks equal on both
/// sides: read the Gloss Contour's Anti-aliased back directly.
#[test]
fn bevel_gloss_anti_alias_is_written() {
    let fx: Vec<Effect> = sample_effects().into_iter().filter(|e| matches!(e, Effect::BevelEmboss(_))).collect();
    assert!(matches!(fx.as_slice(), [Effect::BevelEmboss(b)] if b.gloss_anti_alias));
    let (_, back) = parse_lfx2(&write_lfx2(false, &fx)).unwrap();
    assert!(matches!(back.as_slice(), [Effect::BevelEmboss(b)] if b.gloss_anti_alias), "{back:?}");
}

#[test]
fn lfx2_roundtrip_all_kinds() {
    let fx = sample_effects();
    let data = write_lfx2(false, &fx);
    let (master, back) = parse_lfx2(&data).unwrap();
    assert!(!master);
    approx(&fx, &back);
    // Kind order is canonical: shadows first ... strokes last.
    assert!(matches!(back[0], Effect::DropShadow(_)));
    assert!(matches!(back.last().unwrap(), Effect::Stroke(_)));
}

#[test]
fn stroke_overprint_is_read_and_written() {
    for overprint in [false, true] {
        let fx = [Effect::Stroke(StrokeFx {
            common: FxCommon::new(BlendMode::Normal, 1.0),
            size: 3.0,
            position: StrokePosition::Outside,
            paint: FxPaint::Color(Color::BLACK),
            overprint,
        })];
        let (_, back) = parse_lfx2(&write_lfx2(false, &fx)).unwrap();
        let [Effect::Stroke(s)] = back.as_slice() else { panic!("{back:?}") };
        assert_eq!(s.overprint, overprint);
    }
    // A stroke without the key (other writers) has Overprint off.
    let d = Descriptor::new("null").with("FrFX", Value::Descriptor(Descriptor::new("FrFX").with("enab", Value::Boolean(true))));
    let mut data = 0u32.to_be_bytes().to_vec();
    data.extend(VersionedDescriptor::new(d).to_bytes());
    let (_, back) = parse_lfx2(&data).unwrap();
    let [Effect::Stroke(s)] = back.as_slice() else { panic!("{back:?}") };
    assert!(!s.overprint);
}

#[test]
fn lfx2_ignores_absent_effects_but_keeps_disabled_configured_effects() {
    let d = Descriptor::new("null")
        .with("DrSh", Value::Descriptor(Descriptor::new("DrSh").with("present", Value::Boolean(false)).with("enab", Value::Boolean(false))))
        .with("FrFX", Value::Descriptor(Descriptor::new("FrFX").with("present", Value::Boolean(true)).with("enab", Value::Boolean(false))))
        .with("SoFi", Value::Descriptor(Descriptor::new("SoFi").with("enab", Value::Boolean(false))));
    let mut data = 0u32.to_be_bytes().to_vec();
    data.extend(VersionedDescriptor::new(d).to_bytes());

    let (_, effects) = parse_lfx2(&data).unwrap();

    assert_eq!(effects.len(), 2);
    assert!(effects.iter().all(|effect| !effect.enabled()));
    assert!(effects.iter().any(|effect| effect.label() == "Stroke"));
    assert!(effects.iter().any(|effect| effect.label() == "Color Overlay"));
}

#[test]
fn multi_instances_use_multi_keys() {
    let fx = sample_effects();
    let data = write_lfx2(true, &fx);
    let (vd, _) = VersionedDescriptor::parse_prefix(&data[4..]).unwrap();
    assert!(vd.descriptor.get("dropShadowMulti").is_some());
    assert!(vd.descriptor.get("frameFXMulti").is_some());
    assert!(vd.descriptor.get("SoFi").is_some());
}

#[test]
fn blend_names_are_unique() {
    let mut s = std::collections::HashSet::new();
    for (_, n) in BLEND_NAMES {
        assert!(s.insert(n));
    }
}

#[test]
fn layer_record_blend_keys_parse() {
    for (key, mode) in [
        (b"vLit", BlendMode::VividLight),
        (b"lLit", BlendMode::LinearLight),
        (b"pLit", BlendMode::PinLight),
        (b"hMix", BlendMode::HardMix),
        (b"lbrn", BlendMode::LinearBurn),
        (b"lddg", BlendMode::LinearDodge),
        (b"dkCl", BlendMode::DarkerColor),
        (b"lgCl", BlendMode::LighterColor),
        (b"fsub", BlendMode::Subtract),
        (b"fdiv", BlendMode::Divide),
    ] {
        assert_eq!(blend_from_id(key), Some(mode), "{}", String::from_utf8_lossy(key));
    }
    assert_eq!(blend_from_id(b"zzzz"), None);
    // The issue's Color Overlay: Vivid Light stored as `vLit`, not left as Normal.
    let key = |v: &str| Value::Enumerated { type_id: Id::new("BlnM"), value: Id::new(v) };
    let d = Descriptor::new("null")
        .with("masterFXSwitch", Value::Boolean(true))
        .with("SoFi", Value::Descriptor(Descriptor::new("SoFi").with("enab", Value::Boolean(true)).with("Md  ", key("vLit"))));
    let mut data = 0u32.to_be_bytes().to_vec();
    data.extend(VersionedDescriptor { version: 16, descriptor: d }.to_bytes());
    let (_, fx) = parse_lfx2(&data).unwrap();
    let modes: Vec<BlendMode> = fx.iter().filter_map(|e| if let Effect::ColorOverlay { common, .. } = e { Some(common.blend) } else { None }).collect();
    assert_eq!(modes, vec![BlendMode::VividLight]);
}

#[test]
fn long_string_blend_ids_parse() {
    // Some writers store effect modes as long string IDs (`BlnM` `multiply`, `colorDodge`)
    // instead of the four-character codes (psd-tools effects/blend-modes.psd).
    let long = |v: &str| Value::Enumerated { type_id: Id::new("BlnM"), value: Id::new(v) };
    let d = Descriptor::new("null")
        .with("masterFXSwitch", Value::Boolean(true))
        .with("DrSh", Value::Descriptor(Descriptor::new("DrSh").with("enab", Value::Boolean(true)).with("Md  ", long("linearBurn"))))
        .with("ebbl", Value::Descriptor(Descriptor::new("ebbl").with("hglM", long("colorDodge")).with("sdwM", long("colorBurn"))))
        .with("SoFi", Value::Descriptor(Descriptor::new("SoFi").with("Md  ", long("hue"))));
    let mut data = 0u32.to_be_bytes().to_vec();
    data.extend(VersionedDescriptor { version: 16, descriptor: d }.to_bytes());
    let (_, fx) = parse_lfx2(&data).unwrap();
    let modes: Vec<BlendMode> = fx
        .iter()
        .flat_map(|e| match e {
            Effect::DropShadow(s) => vec![s.common.blend],
            Effect::BevelEmboss(b) => vec![b.highlight.blend, b.shadow.blend],
            Effect::ColorOverlay { common, .. } => vec![common.blend],
            _ => vec![],
        })
        .collect();
    assert_eq!(modes, vec![BlendMode::LinearBurn, BlendMode::ColorDodge, BlendMode::ColorBurn, BlendMode::Hue]);
    for (m, n) in BLEND_LONG_NAMES {
        assert_eq!(blend_from_id(n.as_bytes()), Some(m));
    }
}

/// A Gradient Overlay `lfx2` laid out as Photoshop writes it: one black colour, opacity
/// 100 → 0 % with the opacity midpoint `mid` (%), Smoothness `intr` (0..4096) and the method
/// enum `gs99` (Photoshop's key in layer effects; codes `Gcls`, `Lnr `, `Perc`).
fn overlay_lfx2(mid: i32, intr: f64, method: &str) -> Vec<u8> {
    let black = || Descriptor::new("RGBC").with("Rd  ", Value::Double(0.0)).with("Grn ", Value::Double(0.0)).with("Bl  ", Value::Double(0.0));
    let clr = |loc: i32| {
        Value::Descriptor(
            Descriptor::new("Clrt")
                .with("Clr ", Value::Descriptor(black()))
                .with("Type", Value::Enumerated { type_id: Id::new("Clry"), value: Id::new("UsrS") })
                .with("Lctn", Value::Integer(loc))
                .with("Mdpn", Value::Integer(50)),
        )
    };
    let trn = |loc: i32, op: f32, mid: i32| {
        Value::Descriptor(Descriptor::new("TrnS").with("Opct", unit(b"#Prc", op)).with("Lctn", Value::Integer(loc)).with("Mdpn", Value::Integer(mid)))
    };
    let grad = Descriptor::new("Grdn")
        .with("GrdF", Value::Enumerated { type_id: Id::new("GrdF"), value: Id::new("CstS") })
        .with("Intr", Value::Double(intr))
        .with("Clrs", Value::List(vec![clr(0), clr(4096)]))
        .with("Trns", Value::List(vec![trn(0, 100.0, 50), trn(4096, 0.0, mid)]));
    let fx = Descriptor::new("GrFl")
        .with("enab", Value::Boolean(true))
        .with("Grad", Value::Descriptor(grad))
        .with("Angl", unit(b"#Ang", 0.0))
        .with("gs99", Value::Enumerated { type_id: Id::new("gradientInterpolationMethodType"), value: Id::new(method) });
    let d = Descriptor::new("null").with("masterFXSwitch", Value::Boolean(true)).with("GrFl", Value::Descriptor(fx));
    let mut data = 0u32.to_be_bytes().to_vec();
    data.extend(VersionedDescriptor { version: 16, descriptor: d }.to_bytes());
    data
}

fn overlay_opacity(data: &[u8]) -> Vec<(f32, f32)> {
    let (_, fx) = parse_lfx2(data).unwrap();
    let Some(Effect::GradientOverlay { gradient, .. }) = fx.first() else { panic!("overlay: {fx:?}") };
    gradient.opacity_stops.clone()
}

fn opacity_at(o: &[(f32, f32)], t: f32) -> f32 {
    let i = o.windows(2).position(|w| t >= w[0].0 && t <= w[1].0).unwrap();
    o[i].1 + (o[i + 1].1 - o[i].1) * (t - o[i].0) / (o[i + 1].0 - o[i].0)
}

/// #1954: the overlay's transparency follows Smoothness, the opacity midpoints and the method
/// like a gradient fill layer's (Photoshop renders both identically; the values are its
/// composites at t = 0.25 and 0.5).
#[test]
fn gradient_overlay_opacity_follows_smoothness_midpoint_and_method() {
    let at = |mid, intr, method: &str, t| opacity_at(&overlay_opacity(&overlay_lfx2(mid, intr, method)), t);
    // Plain Classic stays the linear fade it is stored as.
    assert_eq!(overlay_opacity(&overlay_lfx2(50, 0.0, "Gcls")), vec![(0.0, 1.0), (1.0, 0.0)]);
    // Midpoint 25 %: half transparent a quarter of the way.
    assert!((at(25, 0.0, "Gcls", 0.25) - 0.5).abs() < 0.01);
    // Smoothness 100 % eases the ends (Photoshop: 0.796 at a quarter).
    assert!((at(50, 4096.0, "Gcls", 0.25) - 0.796).abs() < 0.015);
    // The method bends the transparency ramp too, not only the colours (Photoshop at the
    // middle: 0.631 Linear, 0.255 Perceptual).
    assert!((at(50, 0.0, "Lnr ", 0.5) - 0.631).abs() < 0.015);
    assert!((at(50, 0.0, "Perc", 0.5) - 0.255).abs() < 0.015);
    assert!((at(25, 4096.0, "Perc", 0.25) - 0.255).abs() < 0.015);
}

/// A regenerated `lfx2` carries the baked ramp as plain (Classic, Smoothness 0) stops, so it
/// reads back unchanged.
#[test]
fn baked_overlay_opacity_round_trips() {
    let data = overlay_lfx2(25, 4096.0, "Perc");
    let (master, fx) = parse_lfx2(&data).unwrap();
    let a = overlay_opacity(&data);
    let b = overlay_opacity(&write_lfx2(master, &fx));
    assert!(a.len() > 2);
    assert_eq!(a.len(), b.len());
    for (x, y) in a.iter().zip(&b) {
        assert!((x.0 - y.0).abs() < 1e-3 && (x.1 - y.1).abs() < 1e-4, "{x:?} vs {y:?}");
    }
}

#[test]
fn garbage_is_none() {
    assert!(parse_lfx2(&[0, 0]).is_none());
    assert!(parse_lrfx(&[0, 0, 0, 1, b'X']).is_none());
}

#[test]
fn lrfx_drop_shadow() {
    // version 0, count 1, 'dsdw' v2 record.
    let mut d = vec![0, 0, 0, 1];
    d.extend_from_slice(b"8BIMdsdw");
    let mut rec = Vec::new();
    rec.extend_from_slice(&2u32.to_be_bytes()); // version
    rec.extend_from_slice(&(7u32 << 16).to_be_bytes()); // blur (16.16)
    rec.extend_from_slice(&0u32.to_be_bytes()); // intensity
    rec.extend_from_slice(&(90u32 << 16).to_be_bytes()); // angle
    rec.extend_from_slice(&(4u32 << 16).to_be_bytes()); // distance
    rec.extend_from_slice(&[0, 0, 0xff, 0xff, 0, 0, 0, 0, 0, 0]); // color: red
    rec.extend_from_slice(b"8BIMmul ");
    rec.extend_from_slice(&[1, 1, 128]);
    rec.extend_from_slice(&[0; 10]);
    d.extend_from_slice(&(rec.len() as u32).to_be_bytes());
    d.extend(rec);
    let (_, fx) = parse_lrfx(&d).unwrap();
    match &fx[0] {
        Effect::DropShadow(s) => {
            assert_eq!(s.size, 7.0);
            assert_eq!(s.distance, 4.0);
            assert_eq!(s.common.blend, BlendMode::Multiply);
            assert!((s.common.opacity - 128.0 / 255.0).abs() < 1e-6);
            assert_eq!(s.color.c[0], 1.0);
        }
        other => panic!("{other:?}"),
    }
}

/// CodeRabbit on #9590: inputs closer than `f32::EPSILON` but distinct are still a span. A point
/// on the straight line between two such inputs isn't mistaken for a bend and kept in place of a
/// genuine one.
#[test]
fn a_point_on_a_tiny_straight_span_is_not_a_bend() {
    let p = |input: f32, output: f32| CurvePoint { input, output };
    // Three distinct inputs a few ulps apart near 0.5, on a straight rise from 0 to 1.
    let (a, b) = (0.5_f32, f32::from_bits(0.5_f32.to_bits() + 2));
    let mid = f32::from_bits(0.5_f32.to_bits() + 1);
    let mut points = vec![p(0.0, 0.0), p(0.2, 0.0)];
    points.extend([p(a, 0.0), p(mid, 0.5), p(b, 1.0)]);
    // Fourteen gentle bends after it, each one a real (if small) peak.
    points.extend((0..14).map(|k| p(0.55 + k as f32 * 0.03, if k % 2 == 0 { 1.0 } else { 0.9 })));
    points.push(p(1.0, 1.0));
    let kept = psd_contour_points(&points);
    assert_eq!(kept.len(), 16);
    assert!(!kept.contains(&3), "the point on the tiny straight span was kept over a real bend: {kept:?}");
}
