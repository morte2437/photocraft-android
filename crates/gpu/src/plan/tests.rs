use super::*;
use photocraft_color::{Color, ColorMode, SampleType};
use photocraft_geom::Size;

#[test]
fn nested_pass_through_groups_with_clipped_layers_are_not_planned_exponentially() {
    let mut layers = vec![Layer::raster("leaf", photocraft_color::PixelFormat::RGBA8)];
    for _ in 0..30 {
        let group = Layer::group("g", layers);
        let mut clipped = Layer::raster("c", photocraft_color::PixelFormat::RGBA8);
        clipped.clipped = true;
        layers = vec![group, clipped];
    }
    let mut doc = Document::new("nest", Size::new(8, 8), ColorMode::Rgb, SampleType::U8);
    doc.layers = layers;
    let started = std::time::Instant::now();
    let Err(error) = plan(&doc) else { panic!("2^30 plannings must be refused") };
    assert!(error.0.contains("too many nested layers"), "{}", error.0);
    assert!(started.elapsed().as_secs() < 10, "{:?}", started.elapsed());
}

#[test]
fn stops_closer_than_one_lut_step_stay_on_the_cpu() {
    assert!(check_gradient_stops(&[(0.4999_f32, ()), (0.5, ())]).is_err());
    assert!(check_gradient_stops(&[(0.0_f32, ()), (1.0, ())]).is_ok());
}

#[test]
fn lab_channel_adjustments_defer_to_the_cpu_in_lab_documents() {
    let lab_rules = [
        Adjustment::Invert,
        Adjustment::Posterize { levels: 4 },
        Adjustment::Threshold { level: 0.5 },
        Adjustment::BrightnessContrast { brightness: 10.0, contrast: 5.0, legacy: false },
        Adjustment::GradientMap { stops: vec![(0.0, [0.0; 3]), (1.0, [1.0; 3])], reverse: false, dither: false },
    ];
    for adj in &lab_rules {
        assert!(!adjustment_on_gpu(adj, ColorMode::Lab), "{adj:?}");
        for mode in [ColorMode::Rgb, ColorMode::Grayscale] {
            assert!(adjustment_on_gpu(adj, mode), "{adj:?} in {mode:?}");
        }
        // CMYK documents invert, posterize, threshold and brighten their inks, on the CPU.
        let inks = !matches!(adj, Adjustment::GradientMap { .. });
        assert_eq!(adjustment_on_gpu(adj, ColorMode::Cmyk), !inks, "{adj:?} in CMYK");
    }
    // CMYK Hue/Saturation works on the inks (on the CPU); Colorize stays on the screen colour.
    let ranges = photocraft_doc::adjust::HueRange::defaults();
    let hue = |colorize| Adjustment::HueSaturation { hue: 30.0, saturation: 10.0, lightness: 0.0, colorize, ranges: ranges.clone() };
    assert!(!adjustment_on_gpu(&hue(false), ColorMode::Cmyk) && adjustment_on_gpu(&hue(true), ColorMode::Cmyk));
    assert!(adjustment_on_gpu(&hue(false), ColorMode::Rgb));
    // Screen-colour adjustments stay on the GPU in Lab documents.
    assert!(adjustment_on_gpu(&Adjustment::Exposure { exposure: 1.0, offset: 0.0, gamma: 1.0 }, ColorMode::Lab));
}

#[test]
fn native_cmyk_fills_fall_back_only_without_a_valid_cache() {
    use photocraft_doc::{FillCache, GradientStyle};
    for depth in photocraft_color::SampleType::ALL {
        for fill in [Fill::Solid(Color::WHITE), Fill::gradient(vec![(0.0, Color::BLACK), (1.0, Color::WHITE)], 0.0, 1.0, GradientStyle::Linear, false)] {
            let mut doc = Document::new("CMYK", photocraft_geom::Size::new(3, 2), photocraft_color::ColorMode::Cmyk, depth);
            doc.layers.push(Layer::new("Fill", LayerContent::Fill(fill.clone())));
            assert!(plan(&doc).unwrap_err().0.contains("native CMYK fill"));
            doc.layers[0].visible = false;
            assert!(plan(&doc).is_ok());
            doc.layers[0].visible = true;
            doc.layers[0].fill_cache = Some(FillCache { fill, surface: photocraft_raster::Surface::new(doc.pixel_format()) });
            assert!(plan(&doc).is_ok());
            doc.layers[0].fill_cache.as_mut().unwrap().fill = Fill::Solid(Color::TRANSPARENT);
            assert!(plan(&doc).is_err());
            doc.mode = photocraft_color::ColorMode::Rgb;
            assert!(plan(&doc).is_ok());
        }
    }
}
#[test]
fn outer_glow_plan_knocks_out_see_through_fill() {
    use photocraft_doc::{Contour, FxCommon, GlowSource, GlowTechnique};

    for fill in [0.0, 0.4, 1.0] {
        let mut d = Document::with_background("glow", Size::new(64, 64), ColorMode::Rgb, SampleType::U8, Color::WHITE);
        let mut layer = Layer::raster("frame", d.pixel_format());
        layer.surface_mut().unwrap().fill_rect(Rect::new(16, 16, 48, 48), &[1.0, 0.0, 0.0, 1.0]);
        layer.fill_opacity = fill;
        layer.effects.items.push(Effect::OuterGlow(Glow {
            common: FxCommon::new(BlendMode::Normal, 1.0),
            paint: FxPaint::Color(Color::rgb(1.0, 1.0, 0.0)),
            technique: GlowTechnique::Softer,
            spread: 0.3,
            size: 8.0,
            contour: Contour::Linear,
            anti_alias: false,
            range: 0.5,
            jitter: 0.0,
            noise: 0.0,
            source: GlowSource::Edge,
        }));
        d.layers.push(layer);
        let planned = plan(&d).unwrap();
        let glow = planned
            .passes
            .iter()
            .find(|pass| pass.kernel == Kernel::FxPaint && pass.flags & F_KNOCKOUT != 0 && pass.color[0] > 0.9 && pass.color[1] > 0.9 && pass.color[2] < 0.1)
            .expect("outer glow paint pass");
        assert!((glow.extra[3] - (1.0 - fill)).abs() < 1e-6, "fill {fill}");
    }
}

#[test]
fn color_lookup_with_an_overflowing_stored_size_uploads_no_table() {
    for size in [1 << 22, u32::MAX] {
        let a =
            Adjustment::ColorLookup { name: "x".into(), lut: Some(std::sync::Arc::new(vec![0.5; 24])), size, tetrahedral: false, dither: false, source: None };
        let (kind, _, rows) = adjustment_program(&a, Transfer::Srgb, SampleType::U8, photocraft_color::ColorMode::Rgb);
        assert_eq!((kind, rows.is_none()), (0, true), "size {size}");
    }
}

#[test]
fn root_checkpoints_have_only_one_live_input_and_keep_clips_whole() {
    for mode in [BlendMode::Normal, BlendMode::PassThrough] {
        let mut d = Document::with_background("t", Size::new(32, 24), ColorMode::Rgb, SampleType::U8, Color::WHITE);
        let mut clip = Layer::new("clip", LayerContent::Fill(Fill::Solid(Color::rgba(0.2, 0.3, 0.7, 0.4))));
        clip.clipped = true;
        let mut group = Layer::group(
            "group",
            vec![
                Layer::new("fill", LayerContent::Fill(Fill::Solid(Color::rgba(0.9, 0.1, 0.3, 0.6)))),
                Layer::new("invert", LayerContent::Adjustment(Adjustment::Invert)),
            ],
        );
        group.blend = mode;
        d.layers.extend([group, clip, Layer::new("top", LayerContent::Adjustment(Adjustment::Invert))]);
        let plan = plan(&d).unwrap();
        assert_eq!(plan.checkpoints.iter().map(|c| c.layer_end).collect::<Vec<_>>(), vec![1, 3, 4]);
        for checkpoint in &plan.checkpoints {
            let mut written = std::collections::HashSet::from([checkpoint.slot]);
            for pass in &plan.passes[checkpoint.pass_end..] {
                for input in [pass.a, pass.b, pass.c, pass.d].into_iter().flatten() {
                    assert!(written.contains(&input), "a suffix must not need another saved slot: {checkpoint:?}");
                }
                written.insert(pass.dst);
            }
            assert!(written.contains(&plan.root));
        }
    }
}

#[test]
fn slots_are_recycled() {
    let mut d = Document::with_background("t", Size::new(8, 8), ColorMode::Rgb, SampleType::U8, Color::WHITE);
    for i in 0..10 {
        d.layers.push(Layer::raster(format!("l{i}"), d.pixel_format()));
    }
    let p = plan(&d).unwrap();
    // Empty layers draw nothing: the clear and the background's one fused pass.
    assert_eq!(p.passes.len(), 1 + 1);
    for (i, l) in d.layers.iter_mut().skip(1).enumerate() {
        l.surface_mut().unwrap().fill_rect(Rect::new(i as i32 % 4, 0, 4, 8), &[0.0, 0.0, 1.0, 1.0]);
    }
    let p = plan(&d).unwrap();
    assert!(p.slots <= 3, "{} slots", p.slots);
    // The background, then each small layer blended over its bounds and copied back.
    assert_eq!(p.passes.len(), 1 + 1 + 10 * 2);
}

#[test]
fn small_layers_composite_only_over_their_bounds() {
    let mut d = Document::with_background("t", Size::new(64, 64), ColorMode::Rgb, SampleType::U8, Color::WHITE);
    let mut l = Layer::raster("small", d.pixel_format());
    l.surface_mut().unwrap().fill_rect(Rect::new(10, 12, 20, 30), &[1.0, 0.0, 0.0, 1.0]);
    let mut group = Layer::group("g", vec![l]);
    group.blend = BlendMode::Normal;
    d.layers.push(group);
    let p = plan(&d).unwrap();
    let r = Rect::new(10, 12, 20, 30);
    // The group's content, its blend and the copy back are all limited to the layer.
    assert!(p.passes.iter().filter(|p| matches!(p.kernel, Kernel::Content | Kernel::Blend)).skip(2).all(|p| p.clip == Some(r)), "{:?}", p.passes);
    let last = p.passes.last().unwrap();
    assert_eq!((last.kernel, last.dst, last.clip), (Kernel::CopyRect, p.root, Some(r)));
    // A layer that covers most of the canvas composites everywhere.
    d.layers[1].children_mut().unwrap()[0].surface_mut().unwrap().fill_rect(Rect::new(0, 0, 60, 60), &[1.0, 0.0, 0.0, 1.0]);
    let p = plan(&d).unwrap();
    assert!(p.passes.iter().all(|p| p.clip.is_none()));
}

#[test]
fn effects_are_planned_in_place() {
    let mut d = Document::with_background("t", Size::new(8, 8), ColorMode::Rgb, SampleType::U8, Color::WHITE);
    let mut l = Layer::raster("fx", d.pixel_format());
    l.surface_mut().unwrap().fill_rect(Rect::new(2, 2, 5, 5), &[1.0, 0.0, 0.0, 1.0]);
    l.effects.items.push(photocraft_doc::Effect::ColorOverlay { common: photocraft_doc::FxCommon::new(BlendMode::Normal, 1.0), color: Color::WHITE });
    l.effects.items.push(photocraft_doc::Effect::default_drop_shadow());
    d.layers.push(l);
    let p = plan(&d).unwrap();
    assert_eq!(p.fx.len(), 1);
    let m = photocraft_compose::effects::margin(&d.layers[1]);
    assert_eq!(p.fx[0].region, Rect::new(2, 2, 5, 5).inflate(m).intersect(&d.bounds().inflate(m)));
    // The effect region covers the whole canvas: the merged result is the backdrop, no copy.
    assert!(p.fx[0].region.contains_rect(&d.bounds()));
    let last = p.passes.last().unwrap();
    assert_eq!(last.kernel, Kernel::FxMerge);
    assert_eq!(last.dst, p.root);
    assert!(p.slots <= 6, "{} slots", p.slots);
    // On a larger canvas the result lands back in the backdrop slot over the region only.
    let mut big = Document::with_background("t", Size::new(64, 64), ColorMode::Rgb, SampleType::U8, Color::WHITE);
    big.layers.push(d.layers[1].clone());
    let p = plan(&big).unwrap();
    assert!(!p.fx[0].region.contains_rect(&big.bounds()));
    let last = p.passes.last().unwrap();
    assert_eq!((last.kernel, last.dst, last.clip), (Kernel::CopyRect, p.root, Some(p.fx[0].region)));
}

#[test]
fn exterior_effects_merge_from_the_shape() {
    // A drop shadow alone: the layer's pixels, the shadow, the merge. With an interior effect
    // the layer is built in passes of its own (colour inside the shape, the overlay, alpha).
    let mut d = Document::with_background("t", Size::new(8, 8), ColorMode::Rgb, SampleType::U8, Color::WHITE);
    let mut l = Layer::raster("fx", d.pixel_format());
    l.surface_mut().unwrap().fill_rect(Rect::new(0, 0, 8, 8), &[1.0, 0.0, 0.0, 0.5]);
    l.fill_opacity = 0.7;
    l.effects.items.push(photocraft_doc::Effect::default_drop_shadow());
    d.layers.push(l);
    let p = plan(&d).unwrap();
    let fx: Vec<Kernel> = p.passes.iter().skip(2).map(|p| p.kernel).collect();
    assert_eq!(fx, [Kernel::Content, Kernel::FxPaint, Kernel::FxMerge], "{:?}", p.passes);
    let merge = p.passes.last().unwrap();
    assert!(merge.flags & F_LAYER_SHAPE != 0);
    assert_eq!(merge.params[0][0], 0.7);
    d.layers[1].effects.items.push(photocraft_doc::Effect::ColorOverlay { common: photocraft_doc::FxCommon::new(BlendMode::Normal, 1.0), color: Color::WHITE });
    let p = plan(&d).unwrap();
    assert!(p.passes.iter().all(|p| p.flags & F_LAYER_SHAPE == 0));
    assert!(p.passes.iter().any(|p| p.kernel == Kernel::FxInit));
}

#[test]
fn opaque_fill_layers_skip_what_they_cover() {
    let opaque = || Fill::gradient(vec![(0.0, Color::BLACK), (1.0, Color::WHITE)], 30.0, 1.0, photocraft_doc::GradientStyle::Linear, false);
    let mut d = Document::with_background("t", Size::new(8, 8), ColorMode::Rgb, SampleType::U8, Color::WHITE);
    for i in 0..4 {
        // Small painted layers (empty ones plan no passes at all).
        let mut l = Layer::raster(format!("l{i}"), d.pixel_format());
        l.surface_mut().unwrap().fill_rect(Rect::new(i, i, i + 2, i + 2), &[0.0, 0.0, 1.0, 1.0]);
        d.layers.push(l);
    }
    d.layers.push(Layer::new("g", LayerContent::Fill(opaque())));
    let mut top = Layer::raster("top", d.pixel_format());
    top.clipped = true;
    d.layers.push(top);
    // Clear, then the fill and its clipped layer: the background and four rasters are skipped.
    let skipped = plan(&d).unwrap().passes.len();
    let mut alone = d.clone();
    alone.layers.drain(..5);
    assert_eq!(skipped, plan(&alone).unwrap().passes.len());
    // Anything that lets the layers below show through keeps them.
    let see_through: [fn(&mut Layer); 6] = [
        |l| l.opacity = 0.99,
        |l| l.fill_opacity = 0.5,
        |l| l.blend = BlendMode::Multiply,
        |l| l.visible = false,
        |l| l.excluded_channels = 1,
        |l| {
            if let LayerContent::Fill(Fill::Gradient { opacity_stops, .. }) = &mut l.content {
                opacity_stops.push((1.0, 0.5));
            }
        },
    ];
    for (k, f) in see_through.iter().enumerate() {
        let mut d2 = d.clone();
        f(&mut d2.layers[5]);
        assert!(plan(&d2).unwrap().passes.len() > skipped, "case {k}: the layers below are planned");
        assert!(!photocraft_compose::occludes_below(&d2.layers[5], ColorMode::Rgb), "case {k}");
    }
    let mut translucent = d.clone();
    translucent.layers[5].content = LayerContent::Fill(Fill::Solid(Color::rgba(1.0, 0.0, 0.0, 0.5)));
    assert!(!photocraft_compose::occludes_below(&translucent.layers[5], ColorMode::Rgb));
    assert!(photocraft_compose::occludes_below(&d.layers[5], ColorMode::Rgb));
}

#[test]
fn blend_if_plans_on_the_gpu() {
    let mut d = Document::with_background("t", Size::new(8, 8), ColorMode::Rgb, SampleType::U8, Color::WHITE);
    let mut l = Layer::raster("bi", d.pixel_format());
    l.surface_mut().unwrap().fill_rect(photocraft_geom::Rect::new(0, 0, 4, 4), &[1.0, 0.0, 0.0, 1.0]);
    l.blend_if.set(0, [photocraft_doc::BlendRange { black: [40, 40], white: [255, 255] }, photocraft_doc::BlendRange::FULL]);
    d.layers.push(l.clone());
    let blend_if = |d: &Document| plan(d).unwrap().passes.iter().filter(|p| p.flags & F_BLEND_IF != 0).count();
    assert_eq!(blend_if(&d), 1);
    // Clipped layers too.
    l.clipped = true;
    d.layers[1].blend_if = Default::default();
    d.layers.push(l);
    assert_eq!(blend_if(&d), 1);
    // Outside RGB, Grayscale and Duotone the setting round-trips without applying.
    d.mode = ColorMode::Lab;
    assert_eq!(blend_if(&d), 0);
}

#[test]
fn blend_if_params_pack_every_range_exactly() {
    let mut bi = photocraft_doc::BlendIf::default();
    bi.set(3, [photocraft_doc::BlendRange { black: [255, 255], white: [0, 1] }, photocraft_doc::BlendRange { black: [12, 200], white: [201, 254] }]);
    let f: Vec<f32> = blend_if_params(&bi).iter().flatten().copied().collect();
    // Entry 3 (B), this layer then underlying; unset entries are full ranges.
    assert_eq!(&f[12..16], &[255.0 * 256.0 + 255.0, 1.0, 12.0 * 256.0 + 200.0, 201.0 * 256.0 + 254.0]);
    assert_eq!(&f[0..2], &[0.0, 255.0 * 256.0 + 255.0]);
}

#[test]
fn advanced_blending_falls_back_to_the_cpu() {
    use photocraft_doc::Knockout;
    let mut d = Document::with_background("t", Size::new(8, 8), ColorMode::Rgb, SampleType::U8, Color::WHITE);
    let mut l = Layer::raster("ko", d.pixel_format());
    l.surface_mut().unwrap().fill_rect(photocraft_geom::Rect::new(0, 0, 4, 4), &[1.0, 0.0, 0.0, 1.0]);
    d.layers.push(l.clone());
    assert!(plan(&d).is_ok(), "defaults stay on the GPU");
    // Switches with nothing to act on (no effects, no clipped layers) stay on the GPU too.
    d.layers[1].advanced.blend_interior = true;
    d.layers[1].advanced.blend_clipped = false;
    assert!(plan(&d).is_ok());
    d.layers[1].advanced.knockout = Knockout::Deep;
    assert!(plan(&d).unwrap_err().0.contains("Advanced Blending"));
    // Inside groups and clipping groups.
    d.layers[1].advanced.knockout = Knockout::None;
    let mut c = l.clone();
    c.clipped = true;
    d.layers.push(c.clone());
    assert!(plan(&d).is_err(), "clipped layers blended individually");
    d.layers[1].advanced.blend_clipped = true;
    assert!(plan(&d).is_ok());
    d.layers[2].advanced.knockout = Knockout::Shallow;
    assert!(plan(&d).is_err(), "clipped knockout");
    d.layers.truncate(1);
    let mut inner = l;
    inner.advanced.knockout = Knockout::Shallow;
    d.layers.push(Layer::group("g", vec![inner]));
    assert!(plan(&d).is_err(), "knockout inside a group");
}

/// Adjustments whose LUT rows the planner caches (#11498), with settings away from identity.
fn row_cached_adjustments() -> Vec<Adjustment> {
    use photocraft_doc::adjust::{CurvePoint, HueRange, LevelsChannel};
    let ch = |g: f32| LevelsChannel { in_black: 0.05, in_white: 0.9, gamma: g, out_black: 0.02, out_white: 0.97 };
    let mut ranges = HueRange::defaults();
    ranges[1].hue = 20.0;
    let pts = vec![CurvePoint { input: 0.0, output: 0.0 }, CurvePoint { input: 0.4, output: 0.55 }, CurvePoint { input: 1.0, output: 1.0 }];
    vec![
        Adjustment::Levels { master: ch(1.3), per_channel: [ch(0.8), ch(1.0), ch(1.6)], space: ToneSpace::Rgb, black: Default::default() },
        Adjustment::Curves { master: pts.clone(), per_channel: [pts.clone(), vec![], pts], space: ToneSpace::Rgb, black: vec![], pencil: false },
        Adjustment::HueSaturation { hue: 0.0, saturation: 10.0, lightness: 0.0, colorize: false, ranges },
        Adjustment::ColorBalance { shadows: [10.0, 0.0, -5.0], midtones: [0.0, 5.0, 0.0], highlights: [0.0, 0.0, 10.0], preserve_luminosity: true },
    ]
}

#[test]
fn cached_adjustment_rows_match_freshly_built_tables() {
    for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
        for a in row_cached_adjustments() {
            let fresh: Vec<Vec<f32>> = match &a {
                Adjustment::Levels { .. } | Adjustment::Curves { .. } => adjust::tone_luts_depth(&a, Some(depth)).into_iter().take(3).collect(),
                Adjustment::HueSaturation { ranges, .. } => adjust::hue_range_tables(ranges).into_iter().collect(),
                Adjustment::ColorBalance { shadows, midtones, highlights, preserve_luminosity } => {
                    let curves = adjust::BalanceCurve::rgb([*shadows, *midtones, *highlights], *preserve_luminosity);
                    (0..3).map(|ch| (0..4096).map(|k| curves[ch].eval(k as f32 / 4095.0)).collect()).collect()
                }
                _ => panic!("not a row-cached adjustment"),
            };
            // The first plan may fill the cache, the second reads it: both give the fresh rows.
            for _ in 0..2 {
                let (_, _, rows) = adjustment_program(&a, Transfer::Srgb, depth, ColorMode::Rgb);
                let rows = rows.expect("LUT rows");
                if matches!(a, Adjustment::Levels { .. }) && depth == SampleType::F32 {
                    continue; // The analytic float Levels kernel takes coefficients, not tables.
                }
                assert_eq!(rows.len(), fresh.len(), "{} at {depth:?}", a.label());
                for (row, f) in rows.iter().zip(&fresh) {
                    assert!(row.iter().zip(f).all(|(x, y)| x.to_bits() == y.to_bits()), "{} at {depth:?}", a.label());
                }
            }
        }
    }
}

#[test]
fn row_keys_cover_exactly_what_the_rows_are_built_from() {
    use photocraft_doc::adjust::LevelsChannel;
    let key = |a: &Adjustment, depth| lower::row_key(a, depth).expect("a row-cached adjustment");
    let [levels, curves, hue_sat, balance] = <[Adjustment; 4]>::try_from(row_cached_adjustments()).expect("four adjustments");
    // The same settings give the same key on every plan, so later plans read the stored rows.
    for a in [&levels, &curves, &hue_sat, &balance] {
        assert_eq!(key(a, SampleType::U8), key(&a.clone(), SampleType::U8), "{}", a.label());
    }
    // Levels and Curves round to the document's quantum, so their keys hold the depth.
    assert_ne!(key(&levels, SampleType::U8), key(&levels, SampleType::U16));
    assert_ne!(key(&curves, SampleType::U16), key(&curves, SampleType::F32));
    // Hue/Saturation range tables and Color Balance curves don't depend on depth.
    assert_eq!(key(&hue_sat, SampleType::U8), key(&hue_sat, SampleType::F32));
    assert_eq!(key(&balance, SampleType::U8), key(&balance, SampleType::U16));
    // Dragging the master Hue/Saturation sliders leaves the range tables (and their entry) alone.
    let Adjustment::HueSaturation { ranges, .. } = &hue_sat else { panic!("Hue/Saturation") };
    let master = Adjustment::HueSaturation { hue: 40.0, saturation: -30.0, lightness: 12.0, colorize: false, ranges: ranges.clone() };
    assert_eq!(key(&hue_sat, SampleType::U8), key(&master, SampleType::U8));
    let mut moved = ranges.clone();
    moved[4].bounds[2] += 1.0;
    let moved = Adjustment::HueSaturation { hue: 0.0, saturation: 10.0, lightness: 0.0, colorize: false, ranges: moved };
    assert_ne!(key(&hue_sat, SampleType::U8), key(&moved, SampleType::U8));
    // Values `==` can't tell apart still get their own rows.
    let black = |out_black: f32| {
        let ch = LevelsChannel { out_black, ..Default::default() };
        Adjustment::Levels { master: ch.clone(), per_channel: [ch.clone(), ch.clone(), ch], space: ToneSpace::Rgb, black: Default::default() }
    };
    assert_ne!(key(&black(0.0), SampleType::U8), key(&black(-0.0), SampleType::U8));
    // Every field the builders read changes the key.
    let Adjustment::Curves { master, per_channel, black, .. } = &curves else { panic!("Curves") };
    let pencil = Adjustment::Curves { master: master.clone(), per_channel: per_channel.clone(), space: ToneSpace::Rgb, black: black.clone(), pencil: true };
    let lab = Adjustment::Curves { master: master.clone(), per_channel: per_channel.clone(), space: ToneSpace::Lab, black: black.clone(), pencil: false };
    assert_ne!(key(&curves, SampleType::U8), key(&pencil, SampleType::U8));
    assert_ne!(key(&curves, SampleType::U8), key(&lab, SampleType::U8));
    let mut nudged = master.clone();
    nudged[1].output += 0.01;
    let point = Adjustment::Curves { master: nudged, per_channel: per_channel.clone(), space: ToneSpace::Rgb, black: black.clone(), pencil: false };
    assert_ne!(key(&curves, SampleType::U8), key(&point, SampleType::U8), "a curve point's value");
    let Adjustment::ColorBalance { shadows, midtones, highlights, .. } = balance.clone() else { panic!("Color Balance") };
    let plain = Adjustment::ColorBalance { shadows, midtones, highlights, preserve_luminosity: false };
    assert_ne!(key(&balance, SampleType::U8), key(&plain, SampleType::U8));
    // Levels and Curves don't share keys with each other, and adjustments without rows have none.
    assert_ne!(key(&Adjustment::identity_levels(), SampleType::U8), key(&Adjustment::identity_curves(), SampleType::U8));
    assert_eq!(lower::row_key(&Adjustment::Vibrance { vibrance: 10.0, saturation: 0.0 }, SampleType::U8), None);
}

#[test]
fn planning_reads_cached_rows_instead_of_rebuilding_them() {
    use photocraft_compose::lut_rows;
    use photocraft_doc::adjust::{CurvePoint, HueRange, LevelsChannel};
    // Settings no other test plans, so the planted rows are this test's own; the few keys this
    // test binary adds can't evict them from the shared cache.
    let mut ranges = HueRange::defaults();
    ranges[2].hue = 33.0;
    let point = |input, output| CurvePoint { input, output };
    let adjustments = [
        Adjustment::Levels {
            master: LevelsChannel { gamma: 1.37, ..Default::default() },
            per_channel: Default::default(),
            space: ToneSpace::Rgb,
            black: Default::default(),
        },
        Adjustment::Curves {
            master: vec![point(0.0, 0.0), point(0.3, 0.61), point(1.0, 1.0)],
            per_channel: Default::default(),
            space: ToneSpace::Rgb,
            black: vec![],
            pencil: false,
        },
        Adjustment::HueSaturation { hue: 0.0, saturation: 0.0, lightness: 0.0, colorize: false, ranges },
        Adjustment::ColorBalance { shadows: [17.0, 0.0, 0.0], midtones: [0.0; 3], highlights: [0.0; 3], preserve_luminosity: true },
    ];
    for (i, a) in adjustments.iter().enumerate() {
        // Rows no builder makes: the planner returns them only if it reads the cache.
        let planted = vec![[0.25 + i as f32 / 8.0; lut_rows::ROW_LEN]; 3];
        let key = lower::row_key(a, SampleType::U8).expect("a row-cached adjustment");
        assert!(lut_rows::cached(key, || planted.clone()) == planted, "{}: a key no other test uses", a.label());
        let (_, _, rows) = adjustment_program(a, Transfer::Srgb, SampleType::U8, ColorMode::Rgb);
        assert!(rows.as_ref() == Some(&planted), "{}: the plan rebuilt rows the cache held", a.label());
    }
}
