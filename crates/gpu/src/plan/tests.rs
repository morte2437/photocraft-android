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
