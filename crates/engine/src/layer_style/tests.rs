use super::*;
use serde_json::json;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 32, "height": 32})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    s
}

fn effects(s: &Session) -> Vec<Effect> {
    let d = s.active().unwrap();
    d.doc.layer(d.active_layer.unwrap()).unwrap().effects.items.clone()
}

#[test]
fn bevel_command_sets_technique_contour_and_texture() {
    let mut s = session();
    s.execute("layer.layerStyle.bevelEmboss", json!({"style": "pillow", "technique": "chiselHard", "contour": true, "contourRange": 70, "texture": "Bubbles", "textureDepth": -200, "textureInvert": true})).unwrap();
    let fx = effects(&s);
    let Effect::BevelEmboss(b) = &fx[0] else { panic!("{fx:?}") };
    assert_eq!((b.style, b.technique), (photocraft_doc::BevelStyle::PillowEmboss, BevelTechnique::ChiselHard));
    assert!((b.contour.as_ref().unwrap().range - 0.7).abs() < 1e-6);
    let t = b.texture.as_ref().unwrap();
    assert_eq!((t.name.as_str(), t.depth, t.invert), ("Bubbles", -2.0, true));
}

/// #6698 review: a glow's Range runs 1..100 %, as its slider and Photoshop's do. A 0 is
/// held at the 1 % floor, and 100 is kept.
#[test]
fn glow_range_is_held_to_one_through_one_hundred() {
    for kind in ["outerGlow", "innerGlow"] {
        for (sent, kept) in [(0.0, 0.01), (-5.0, 0.01), (1.0, 0.01), (50.0, 0.5), (100.0, 1.0), (250.0, 1.0)] {
            let mut s = session();
            s.execute(&format!("layer.layerStyle.{kind}"), json!({"range": sent})).unwrap();
            let range = match &effects(&s)[0] {
                Effect::OuterGlow(g) | Effect::InnerGlow(g) => g.range,
                other => panic!("{other:?}"),
            };
            assert!((range - kept).abs() < 1e-6, "{kind} range {sent} kept as {range}, not {kept}");
        }
    }
    let doc = |id: &str| specs().into_iter().find(|c| c.id == id).map(|c| c.params).unwrap_or_default();
    assert!(doc("layer.layerStyle.outerGlow").contains(r#""range":1..100=50"#));
    assert!(doc("layer.layerStyle.innerGlow").contains(r#""range":1..100=50"#));
}

#[test]
fn bevel_gloss_anti_alias_is_a_param() {
    let mut s = session();
    s.execute("layer.layerStyle.bevelEmboss", json!({"glossContour": "Ring - Double", "glossAntiAlias": true})).unwrap();
    let Effect::BevelEmboss(b) = &effects(&s)[0] else { panic!() };
    assert!(b.gloss_anti_alias);
    s.execute("layer.layerStyle.bevelEmboss", json!({"glossAntiAlias": false})).unwrap();
    let Effect::BevelEmboss(b) = &effects(&s)[0] else { panic!() };
    assert!(!b.gloss_anti_alias);
}

#[test]
fn stroke_overprint_is_a_param() {
    let mut s = session();
    s.execute("layer.layerStyle.stroke", json!({"size": 4, "overprint": true})).unwrap();
    let Effect::Stroke(st) = &effects(&s)[0] else { panic!() };
    assert!(st.overprint);
    s.execute("layer.layerStyle.stroke", json!({"overprint": false})).unwrap();
    let Effect::Stroke(st) = &effects(&s)[0] else { panic!() };
    assert!(!st.overprint);
}

#[test]
fn bevel_shading_fields_are_independent_and_preserved_on_partial_edits() {
    let mut s = session();
    s.execute(
        "layer.layerStyle.bevelEmboss",
        json!({
            "highlightBlend": "Normal", "highlightColor": "#ff0000", "highlightOpacity": 35,
            "shadowBlend": "Color Burn", "shadowColor": "#0000ff", "shadowOpacity": 60
        }),
    )
    .unwrap();
    let fx = effects(&s);
    let Effect::BevelEmboss(bevel) = &fx[0] else { panic!("{fx:?}") };
    assert_eq!(bevel.highlight.blend, BlendMode::Normal);
    assert_eq!(bevel.highlight_color, Color::rgb(1.0, 0.0, 0.0));
    assert!((bevel.highlight.opacity - 0.35).abs() < 0.001);
    assert_eq!(bevel.shadow.blend, BlendMode::ColorBurn);
    assert_eq!(bevel.shadow_color, Color::rgb(0.0, 0.0, 1.0));
    assert!((bevel.shadow.opacity - 0.60).abs() < 0.001);

    let mut changed = fx[0].clone();
    overlay_effect(&mut changed, &json!({"size": 12}));
    let Effect::BevelEmboss(partial) = &changed else { panic!("expected bevel") };
    assert_eq!(partial.highlight, bevel.highlight);
    assert_eq!(partial.highlight_color, bevel.highlight_color);
    assert_eq!(partial.shadow, bevel.shadow);
    assert_eq!(partial.shadow_color, bevel.shadow_color);

    overlay_effect(&mut changed, &json!({"highlightOpacity": 120, "shadowOpacity": -30}));
    let Effect::BevelEmboss(limits) = &changed else { panic!("expected bevel") };
    assert_eq!((limits.highlight.opacity, limits.shadow.opacity), (1.0, 0.0));
}

#[test]
fn every_style_command_adds_its_effect() {
    for kind in ["dropShadow", "innerShadow", "outerGlow", "innerGlow", "stroke", "colorOverlay", "gradientOverlay", "bevelEmboss", "satin"] {
        let mut s = session();
        s.execute(&format!("layer.layerStyle.{kind}"), json!({})).unwrap();
        let fx = effects(&s);
        assert_eq!(fx.len(), 1, "{kind}");
        assert!(fx[0].enabled());
    }
}

/// #1543: a Satin or Bevel & Emboss size or softness far past Photoshop's range rendered a
/// blur kernel of 2^62 taps ("capacity overflow"), or ran for minutes at 1e6. The blur width
/// is capped at the effect reach, so the document still renders, quickly.
#[test]
fn oversized_satin_and_bevel_sizes_stay_saveable_and_render() {
    for (cmd, p, accepted) in [
        ("satin", json!({"size": 1e308}), false),
        ("satin", json!({"size": 1e30}), true),
        ("satin", json!({"size": 1_000_000}), true),
        ("bevelEmboss", json!({"size": 1e308}), false),
        ("bevelEmboss", json!({"soften": 1e308}), false),
        ("bevelEmboss", json!({"size": 1e30, "soften": 1e30}), true),
        ("bevelEmboss", json!({"size": 1_000_000, "soften": 1_000_000}), true),
    ] {
        let mut s = session();
        s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
        let result = s.execute(&format!("layer.layerStyle.{cmd}"), p.clone());
        if accepted {
            result.unwrap();
        } else {
            assert!(matches!(result, Err(EngineError::BadParams { .. })));
        }
        photocraft_format::save_to_bytes(&s.active().unwrap().doc, &Default::default()).unwrap();
        let start = std::time::Instant::now();
        s.execute("document.pixel", json!({"x": 0, "y": 0})).unwrap_or_else(|e| panic!("{cmd} {p}: {e}"));
        assert!(start.elapsed().as_secs() < 20, "{cmd} {p}: {:?}", start.elapsed());
    }
}

#[test]
fn params_are_applied_and_replace_or_add() {
    let mut s = session();
    s.execute("layer.layerStyle.stroke", json!({"size": 7, "position": "inside", "color": "#00ff00", "opacity": 50})).unwrap();
    match &effects(&s)[0] {
        Effect::Stroke(st) => {
            assert_eq!(st.size, 7.0);
            assert_eq!(st.position, StrokePosition::Inside);
            assert_eq!(st.common.opacity, 0.5);
            assert_eq!(st.paint, FxPaint::Color(Color::rgb(0.0, 1.0, 0.0)));
        }
        other => panic!("{other:?}"),
    }
    s.execute("layer.layerStyle.stroke", json!({"size": 2})).unwrap();
    assert_eq!(effects(&s).len(), 1, "replaces");
    s.execute("layer.layerStyle.stroke", json!({"size": 4, "add": true})).unwrap();
    assert_eq!(effects(&s).len(), 2, "adds a second instance");
    s.execute("layer.layerStyle.dropShadow", json!({"blend": "normal", "distance": 12})).unwrap();
    assert!(matches!(&effects(&s)[2], Effect::DropShadow(sh) if sh.common.blend == BlendMode::Normal && sh.distance == 12.0));
}

/// The gradient overlay and the gradient stroke keep their scale in Photoshop's 10-150 %.
#[test]
fn gradient_overlay_and_stroke_scale_stay_in_range() {
    let scale = |s: &Session| match &effects(s)[0] {
        Effect::GradientOverlay { gradient, .. } => gradient.scale,
        Effect::Stroke(st) => match &st.paint {
            FxPaint::Gradient(g) => g.scale,
            other => panic!("{other:?}"),
        },
        other => panic!("{other:?}"),
    };
    for (cmd, p, want) in [
        ("gradientOverlay", json!({"scale": 1000}), 1.5),
        ("gradientOverlay", json!({"scale": 1}), 0.1),
        ("gradientOverlay", json!({"scale": 80}), 0.8),
        ("stroke", json!({"from": "#000000", "to": "#ffffff", "scale": -50}), 0.1),
        ("stroke", json!({"from": "#000000", "to": "#ffffff", "scale": 0}), 0.1),
        ("stroke", json!({"from": "#000000", "to": "#ffffff", "scale": 400}), 1.5),
    ] {
        let mut s = session();
        s.execute(&format!("layer.layerStyle.{cmd}"), p.clone()).unwrap();
        assert!((scale(&s) - want).abs() < 1e-6, "{cmd} {p}: {}", scale(&s));
    }
    // Beyond f32 range is refused before any edit (commands.rs), not clamped.
    let err = session().execute("layer.layerStyle.gradientOverlay", json!({"scale": 1e39})).unwrap_err();
    assert!(format!("{err:?}").contains("finite 32-bit"), "{err:?}");
}

#[test]
fn clear_and_undo() {
    let mut s = session();
    s.execute("layer.layerStyle.colorOverlay", json!({"color": "#ff0000"})).unwrap();
    s.execute("layer.layerStyle.clear", json!({})).unwrap();
    assert!(effects(&s).is_empty());
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(effects(&s).len(), 1);
}

fn pixel(s: &mut Session) -> Vec<f32> {
    serde_json::from_value(s.execute("document.pixel", json!({"x": 3, "y": 3})).unwrap()).unwrap()
}

fn list_enabled(s: &Session) -> bool {
    let d = s.active().unwrap();
    d.doc.layer(d.active_layer.unwrap()).unwrap().effects.enabled
}

/// #3648: the Layers panel's "Effects" eye (`layer.setEffectsVisible`) must survive a Layer
/// Style dialog OK: the dialog's whole-list replace re-creates the items but leaves the
/// list's master switch alone.
#[test]
fn replacing_the_list_keeps_a_disabled_effect_list_disabled() {
    let mut s = session();
    s.execute("edit.fill", json!({"contents": "color", "color": "#ff0000"})).unwrap();
    s.execute("layer.layerStyle.colorOverlay", json!({"color": "#0000ff"})).unwrap();
    let shown = pixel(&mut s);
    s.execute("layer.setEffectsVisible", json!({"visible": false})).unwrap();
    let hidden = pixel(&mut s);
    assert_ne!(hidden, shown, "the eye really hides the style");
    let r = s.execute("layer.layerStyle.replace", json!({"effects": [{"kind": "colorOverlay", "params": {"color": "#00ff00"}}]})).unwrap();
    assert_eq!(r["effects"], 1, "the list itself is updated");
    assert!(!list_enabled(&s), "the whole-list switch survives the edit");
    assert!(effects(&s)[0].enabled(), "the effect's own state is updated");
    assert_eq!(pixel(&mut s), hidden, "a disabled list still draws nothing");
    s.execute("layer.setEffectsVisible", json!({"visible": true})).unwrap();
    assert_ne!(pixel(&mut s), hidden, "re-showing the list draws the edited effect");
}

/// #3648: adding or replacing one effect (`layer.layerStyle.<kind>`) also leaves the master
/// switch as the user left it.
#[test]
fn set_effect_keeps_a_disabled_effect_list_disabled() {
    let mut s = session();
    s.execute("edit.fill", json!({"contents": "color", "color": "#ff0000"})).unwrap();
    s.execute("layer.layerStyle.colorOverlay", json!({"color": "#0000ff"})).unwrap();
    s.execute("layer.setEffectsVisible", json!({"visible": false})).unwrap();
    let hidden = pixel(&mut s);
    s.execute("layer.layerStyle.colorOverlay", json!({"color": "#00ff00"})).unwrap();
    assert!(!list_enabled(&s), "the whole-list switch survives the edit");
    assert!(effects(&s)[0].enabled(), "the effect's own state is updated");
    assert_eq!(pixel(&mut s), hidden, "a disabled list still draws nothing");
}

/// A layer that never had effects keeps the default: its first effect shows.
#[test]
fn a_first_effect_shows_on_a_layer_that_never_had_effects() {
    let mut s = session();
    s.execute("edit.fill", json!({"contents": "color", "color": "#ff0000"})).unwrap();
    let plain = pixel(&mut s);
    s.execute("layer.layerStyle.colorOverlay", json!({"color": "#00ff00"})).unwrap();
    assert!(list_enabled(&s), "a fresh effects list defaults to enabled");
    assert_ne!(pixel(&mut s), plain, "the first effect renders");
}

/// #3102: an effect row dropped on the Delete button deletes only that effect, in one step.
#[test]
fn clear_one_effect_by_index() {
    let mut s = session();
    s.execute("layer.layerStyle.dropShadow", json!({})).unwrap();
    s.execute("layer.layerStyle.colorOverlay", json!({"color": "#ff0000"})).unwrap();
    let layer = s.active().unwrap().active_layer.unwrap().0;
    s.execute("layer.layerStyle.clear", json!({"layer": layer, "effect": 0})).unwrap();
    assert!(matches!(effects(&s)[..], [Effect::ColorOverlay { .. }]));
    for bad in [json!({"effect": 5}), json!({"effect": -1}), json!({"effect": "x"}), json!({"effect": 0, "layer": u64::MAX})] {
        assert!(s.execute("layer.layerStyle.clear", bad).is_err());
    }
    assert_eq!(effects(&s).len(), 1, "bad params change nothing");
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(effects(&s).len(), 2);
}

#[test]
fn color_overlay_renders() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
    s.execute("layer.layerStyle.colorOverlay", json!({"color": "#ff0000"})).unwrap();
    let px: Vec<f32> = serde_json::from_value(s.execute("document.pixel", json!({"x": 3, "y": 3})).unwrap()).unwrap();
    assert!(px[0] > 0.99 && px[1] < 0.01, "{px:?}");
}

#[test]
fn gradient_styles_parse() {
    assert_eq!(gradient_style("Radial"), GradientStyle::Radial);
    assert_eq!(gradient_style("diamond"), GradientStyle::Diamond);
    assert_eq!(gradient_style("?"), GradientStyle::Linear);
}

fn snapshot(fx: &Effect) -> Value {
    serde_json::to_value(fx).unwrap()
}

#[test]
fn every_kind_has_factory_defaults() {
    for kind in ["dropShadow", "innerShadow", "outerGlow", "innerGlow", "stroke", "colorOverlay", "gradientOverlay", "patternOverlay", "satin", "bevelEmboss"] {
        assert!(effect_defaults(kind).as_object().is_some_and(|o| !o.is_empty()), "{kind}");
        assert!(effect_from_params(kind, &json!({})).is_some(), "{kind}");
    }
    assert!(effect_from_params("nope", &json!({})).is_none());
}

#[test]
fn replace_builds_edits_and_preserves_instances() {
    let mut s = session();
    // Two strokes: the first edits a snapshot (params overlay), the second is fresh.
    let first = Effect::Stroke(StrokeFx {
        common: photocraft_doc::FxCommon::new(BlendMode::Normal, 1.0),
        size: 7.0,
        position: StrokePosition::Outside,
        paint: FxPaint::Gradient(Gradient { stops: vec![(0.0, Color::rgb(0.0, 0.0, 1.0)), (1.0, Color::WHITE)], ..Gradient::default() }),
        overprint: false,
    });
    let r = s
        .execute(
            "layer.layerStyle.replace",
            json!({"effects": [
                {"kind": "stroke", "fx": snapshot(&first), "params": {"opacity": 50}},
                {"kind": "stroke", "params": {"size": 2, "color": "#ffffff"}},
            ]}),
        )
        .unwrap();
    assert_eq!(r["effects"], 2);
    let fx = effects(&s);
    assert_eq!(fx.len(), 2, "order preserved");
    match (&fx[0], &fx[1]) {
        (Effect::Stroke(a), Effect::Stroke(b)) => {
            assert_eq!(a.common.opacity, 0.5);
            // The overlay must not clobber the gradient paint it doesn't model.
            assert!(matches!(&a.paint, FxPaint::Gradient(g) if g.stops.len() == 2 && g.stops[0].1 == Color::rgb(0.0, 0.0, 1.0)));
            assert_eq!(b.size, 2.0);
            assert_eq!(b.paint, FxPaint::Color(Color::rgb(1.0, 1.0, 1.0)));
        }
        other => panic!("{other:?}"),
    }
    // Replacing with an empty list clears the layer.
    s.execute("layer.layerStyle.replace", json!({"effects": []})).unwrap();
    assert!(effects(&s).is_empty());
}

#[test]
fn replace_rejects_bad_entries_gracefully() {
    let mut s = session();
    let stroke = Effect::Stroke(StrokeFx {
        common: photocraft_doc::FxCommon::new(BlendMode::Normal, 1.0),
        size: 3.0,
        position: StrokePosition::Outside,
        paint: FxPaint::Color(Color::BLACK),
        overprint: false,
    });
    for p in [
        json!({}),
        json!({"effects": "nope"}),
        json!({"effects": [{"params": {}}]}),
        json!({"effects": [{"kind": "nope"}]}),
        json!({"effects": [{"kind": "stroke", "fx": {"Bogus": {}}}]}),
        json!({"effects": [{"kind": "stroke", "fx": 3}]}),
        json!({"effects": [{"kind": "dropShadow", "fx": snapshot(&stroke)}]}),
        json!({"effects": [{"kind": "stroke", "fx": snapshot(&stroke), "params": "nope"}]}),
    ] {
        assert!(s.execute("layer.layerStyle.replace", p).is_err());
    }
    // A long list is refused, not run.
    let many: Vec<_> = (0..257).map(|_| json!({"kind": "stroke"})).collect();
    assert!(s.execute("layer.layerStyle.replace", json!({"effects": many})).is_err());
    assert!(effects(&s).is_empty(), "nothing was applied");
}

#[test]
fn contour_and_noise_params_edit_existing_effects() {
    let mut s = session();
    s.execute("layer.layerStyle.dropShadow", json!({})).unwrap();
    let before = effects(&s);
    let Effect::DropShadow(_) = &before[0] else { panic!("{before:?}") };
    let fx0 = snapshot(&before[0]);
    // Unknown contour names are carried, not clobbered to Linear.
    s.execute("layer.layerStyle.replace", json!({"effects": [{"kind": "dropShadow", "fx": fx0, "params": {"contour": "Cone", "noise": 30}}]})).unwrap();
    let fx = effects(&s);
    let Effect::DropShadow(sh) = &fx[0] else { panic!("{fx:?}") };
    assert_eq!(sh.contour, builtin_contour("Cone").unwrap(), "known preset applies");
    assert!((sh.noise - 0.3).abs() < 1e-6, "noise in percent");
    // An imported contour with an unknown name survives the same round trip.
    let imported = Contour::Custom {
        name: "Photoshop contour".into(),
        points: vec![photocraft_doc::adjust::CurvePoint { input: 0.0, output: 1.0 }, photocraft_doc::adjust::CurvePoint { input: 1.0, output: 0.0 }],
        corners: vec![],
    };
    let mut carried = before[0].clone();
    if let Effect::DropShadow(c) = &mut carried {
        c.contour = imported.clone();
    }
    overlay_effect(&mut carried, &json!({"contour": "Photoshop contour"}));
    let Effect::DropShadow(after) = &carried else { panic!() };
    assert_eq!(after.contour, imported);
    // Linear applies by name.
    let mut back = carried;
    overlay_effect(&mut back, &json!({"contour": "Linear"}));
    let Effect::DropShadow(lin) = &back else { panic!() };
    assert_eq!(lin.contour, Contour::Linear);
}

#[test]
fn contour_presets_match_photoshop() {
    // Photoshop 27.11, measured through Satin: Cone and Cone − Inverted Satins are identical,
    // and Gaussian reads 0.36 at 0.5 through Satin but 0.64 as the preset curve (glows, gloss).
    let lut = |name: &str| {
        let Some(Contour::Custom { points, corners, .. }) = builtin_contour(name) else { panic!("{name}") };
        photocraft_compose::adjust::contour_curve_lut(&points, &corners)
    };
    let satin = |name: &str| photocraft_compose::effects::satin_lut(&builtin_contour(name).unwrap()).unwrap();
    let at = |l: &[f32], x: f32| l[(x * (l.len() - 1) as f32).round() as usize];
    let (cone, inverted) = (satin("Cone"), satin("Cone - Inverted"));
    for x in [0.1, 0.25, 0.5, 0.8] {
        assert!((at(&cone, x) - at(&inverted, x)).abs() < 0.03, "{x}: {} vs {}", at(&cone, x), at(&inverted, x));
    }
    assert!((at(&satin("Gaussian"), 0.5) - 0.36).abs() < 0.04, "{}", at(&satin("Gaussian"), 0.5));
    assert!((at(&lut("Gaussian"), 0.5) - 0.64).abs() < 0.03, "{}", at(&lut("Gaussian"), 0.5));
    assert!((at(&lut("Cove - Deep"), 0.0) - 0.26).abs() < 0.02, "Cove - Deep starts above zero");
}

#[test]
fn gradient_overlay_method_matches_photoshop() {
    // Photoshop 27.11, Classic Gradient Overlay #000000 → #ffffff (Smoothness 100 %): 0.037 at
    // t = 1/16 and 0.5 in the middle; straight stops would give 0.0625.
    let mut s = session();
    s.execute("layer.layerStyle.gradientOverlay", json!({"from": "#000000", "to": "#ffffff", "method": "classic"})).unwrap();
    let Effect::GradientOverlay { gradient, .. } = &effects(&s)[0] else { panic!() };
    let at = |t: f32| {
        let w = gradient.stops.windows(2).find(|w| t >= w[0].0 && t <= w[1].0).unwrap();
        let u = (t - w[0].0) / (w[1].0 - w[0].0).max(1e-6);
        w[0].1.to_rgb()[0] + (w[1].1.to_rgb()[0] - w[0].1.to_rgb()[0]) * u
    };
    assert!((at(1.0 / 16.0) - 0.037).abs() < 0.004, "{}", at(1.0 / 16.0));
    assert!((at(0.5) - 0.5).abs() < 0.004, "{}", at(0.5));
    // Without a method the two stops stay as they are.
    s.execute("layer.layerStyle.gradientOverlay", json!({"from": "#000000", "to": "#ffffff", "add": true})).unwrap();
    let plain = effects(&s).iter().filter(|e| matches!(e, Effect::GradientOverlay { gradient, .. } if gradient.stops.len() == 2)).count();
    assert_eq!(plain, 1, "the added overlay keeps its two stops");
}

#[test]
fn contour_presets_and_custom_curves_apply_in_one_undo_step() {
    let mut s = session();
    // Picking Cone: the preset's own curve, its points corners, in one undo step.
    s.execute("layer.layerStyle.dropShadow", json!({"contour": "Cone", "antiAlias": true})).unwrap();
    let Effect::DropShadow(sh) = &effects(&s)[0] else { panic!() };
    assert_eq!(sh.contour, builtin_contour("Cone").unwrap());
    assert!(sh.contour.is_corner(1) && sh.anti_alias);
    let Contour::Custom { points, corners, .. } = &sh.contour else { panic!() };
    let lut = photocraft_compose::adjust::contour_curve_lut(points, corners);
    let at = |x: f32| lut[(x * (lut.len() - 1) as f32).round() as usize];
    // Photoshop's measured Cone: 0 at both ends, a 0.99 peak at 0.5, symmetric.
    assert!(at(0.0).abs() < 1e-3 && at(1.0).abs() < 1e-3 && at(0.5) > 0.98, "a Λ: {} {} {}", at(0.0), at(0.5), at(1.0));
    assert!((at(0.25) - at(0.75)).abs() < 0.03, "symmetric: {} vs {}", at(0.25), at(0.75));
    assert_eq!(contour_param(&sh.contour), json!("Cone"));
    s.execute("edit.undo", json!({})).unwrap();
    assert!(effects(&s).is_empty(), "one undo removes it");

    // An edited curve (the Contour Editor's param) round trips exactly, in every effect.
    let curve = json!({"name": "Mine", "points": [[0.0, 0.2], [0.4, 0.9], [1.0, 0.1]], "corners": [false, true, false]});
    for kind in ["dropShadow", "innerShadow", "outerGlow", "innerGlow", "satin"] {
        let mut fx = fresh_effect(kind).unwrap();
        overlay_effect(&mut fx, &json!({"contour": curve}));
        let v = values(&fx, "contour");
        assert_eq!(v, curve, "{kind}");
    }
    let mut bevel = fresh_effect("bevelEmboss").unwrap();
    overlay_effect(&mut bevel, &json!({"contour": true, "contourName": curve, "glossContour": "Ring"}));
    let Effect::BevelEmboss(b) = &bevel else { panic!() };
    assert_eq!(contour_param(&b.contour.as_ref().unwrap().contour), curve);
    assert_eq!(b.gloss_contour, builtin_contour("Ring").unwrap());
    // Malformed curves leave the contour alone.
    for bad in [json!({"points": [[0.5, 0.5]]}), json!({"points": [[0.0, "x"], [1.0, 1.0]]}), json!({"points": "no"}), json!(7)] {
        let mut fx = fresh_effect("dropShadow").unwrap();
        overlay_effect(&mut fx, &json!({"contour": bad}));
        assert_eq!(values(&fx, "contour"), json!("Linear"));
    }

    // `.shc` round trip of a curve with a corner.
    let mine = contour_from_param(&curve).unwrap();
    let back = contours_from_shc(&contour_to_shc(&mine).unwrap()).unwrap();
    let back = contour_from_param(&back[0]).unwrap();
    let Contour::Custom { points, corners, .. } = &back else { panic!() };
    assert_eq!(corners, &vec![false, true, false]);
    assert!((points[1].input - 0.4).abs() < 0.003 && (points[1].output - 0.9).abs() < 0.003, "{points:?}");
}

fn values(fx: &Effect, key: &str) -> Value {
    let c = match fx {
        Effect::DropShadow(s) | Effect::InnerShadow(s) => &s.contour,
        Effect::OuterGlow(g) | Effect::InnerGlow(g) => &g.contour,
        Effect::Satin(s) => &s.contour,
        _ => panic!("{key}: no contour"),
    };
    contour_param(c)
}

#[test]
fn make_default_then_reset_restores_it() {
    let mut s = session();
    // Factory default first: Reset without a saved default gives the factory set.
    let r = s.execute("layer.layerStyle.defaultFor", json!({"kind": "stroke"})).unwrap();
    assert_eq!(r["user"], json!(false));
    assert_eq!(r["params"]["size"], json!(1));
    s.execute(
        "layer.layerStyle.makeDefault",
        json!({"kind": "stroke", "params": {"size": 9, "position": "inside", "blend": "Normal", "opacity": 100, "color": "#000000"}}),
    )
    .unwrap();
    let r = s.execute("layer.layerStyle.defaultFor", json!({"kind": "stroke"})).unwrap();
    assert_eq!(r["user"], json!(true));
    assert_eq!(r["params"]["size"], json!(9));
    // Bad kinds and params are graceful errors.
    assert!(s.execute("layer.layerStyle.makeDefault", json!({"kind": "nope", "params": {}})).is_err());
    assert!(s.execute("layer.layerStyle.makeDefault", json!({"kind": "stroke", "params": "x"})).is_err());
    assert!(s.execute("layer.layerStyle.defaultFor", json!({"kind": "nope"})).is_err());
}

/// #7765: a stroke built without params was red, while the dialog's factory default was
/// black. Both now give Photoshop's Reset to Default: 1 px, inside, Normal, 100 %, black.
#[test]
fn fresh_effects_match_the_dialog_factory_defaults() {
    for kind in ["dropShadow", "innerShadow", "outerGlow", "innerGlow", "stroke", "colorOverlay", "gradientOverlay", "patternOverlay", "satin", "bevelEmboss"] {
        let bare = effect_from_params(kind, &json!({})).unwrap();
        let dialog = effect_from_params(kind, &effect_defaults(kind)).unwrap();
        assert_eq!(bare, dialog, "{kind}");
    }
    let Some(Effect::Stroke(st)) = effect_from_params("stroke", &json!({})) else { panic!("no stroke") };
    assert_eq!((st.size, st.position, st.common.blend, st.common.opacity), (1.0, StrokePosition::Inside, BlendMode::Normal, 1.0));
    assert_eq!(st.paint, FxPaint::Color(Color::BLACK));
}

#[test]
fn layer_style_defaults_persist_with_presets() {
    let mut s = session();
    s.execute("layer.layerStyle.makeDefault", json!({"kind": "stroke", "params": {"size": 9}})).unwrap();
    let blob = s.presets.to_json(&s);
    let mut t = Session::new();
    t.load_presets_json(blob);
    let r = t.execute("layer.layerStyle.defaultFor", json!({"kind": "stroke"})).unwrap();
    assert_eq!(r["user"], json!(true));
    assert_eq!(r["params"]["size"], json!(9));
}

#[test]
fn double_stroke_bands_render_and_survive_an_edit() {
    // The manga/typesetting case (#157): a 7 px white stroke under a 3 px black one.
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 48, "height": 48})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.execute("select.rect", json!({"x": 16, "y": 16, "width": 16, "height": 16})).unwrap();
    s.execute("edit.fill", json!({"color": "#000000"})).unwrap();
    s.execute("select.deselect", json!({})).unwrap();
    s.execute("layer.layerStyle.stroke", json!({"size": 3, "color": "#000000", "position": "outside"})).unwrap();
    s.execute("layer.layerStyle.stroke", json!({"size": 7, "color": "#ffffff", "position": "outside", "add": true})).unwrap();
    let px = |s: &mut Session, x: i32, y: i32| -> [f32; 4] {
        let mut v: Vec<f32> = serde_json::from_value(s.execute("document.pixel", json!({"x": x, "y": y})).unwrap()).unwrap();
        v.resize(4, 1.0);
        [v[0], v[1], v[2], v[3]]
    };
    // Bands from the square edge outward: 0..3 px black (the first stroke renders on top),
    // 3..7 px white, then the white background.
    assert!(px(&mut s, 24, 24)[0] < 0.1, "the square is black");
    assert!(px(&mut s, 24, 14)[0] < 0.1, "the first stroke covers the wider one");
    assert!(px(&mut s, 24, 11)[0] > 0.9, "the second stroke shows outside it");
    assert!(px(&mut s, 24, 5)[0] > 0.9, "background beyond both");
    // A dialog round trip (replace with the same snapshots) keeps both bands.
    let before = px(&mut s, 24, 14);
    let st = s.active().unwrap();
    let id = st.active_layer.unwrap();
    let items = st.doc.layer(id).unwrap().effects.items.clone();
    let entries: Vec<Value> = items.iter().map(|fx| json!({"kind": "stroke", "fx": serde_json::to_value(fx).unwrap(), "params": {}})).collect();
    s.execute("layer.layerStyle.replace", json!({"layer": id.0, "effects": entries})).unwrap();
    assert_eq!(px(&mut s, 24, 14), before, "both strokes render identically after the round trip");
}
