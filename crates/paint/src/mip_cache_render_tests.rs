//! Exercise the production cache through brush contexts and native-depth stroke rendering.
use super::*;
use crate::brush::{DualBrush, Smoothing};
use crate::tile::{GrayTile, StoredTile};
use photocraft_geom::Point;

#[test]
fn contexts_share_primary_and_dual_mips_across_settings_and_symmetry_passes() {
    let tip = GrayTile::from_fn(31, 17, |x, y| ((x + y) % 5) as f32 / 4.0);
    let brush = BrushSettings {
        tip: TipShape::Sampled(tip.clone()),
        dual_brush: DualBrush { enabled: true, tip: TipShape::Sampled(tip), ..Default::default() },
        ..Default::default()
    };
    let ctx = BrushContext::new(&brush);
    assert!(Arc::ptr_eq(ctx.tip.as_ref().unwrap(), ctx.dual_tip.as_ref().unwrap()));
    // Symmetry constructs a renderer per pass; later strokes can change size, angle and flow.
    for pass in 0..4 {
        let changed = BrushSettings { size: 10.0 + pass as f32 * 11.0, angle: pass as f32 * 0.3, flow: 0.2, ..brush.clone() };
        let renderer = StrokeRenderer::new(&changed, Some(PixelFormat::RGBA8), 1.0);
        assert!(Arc::ptr_eq(ctx.tip.as_ref().unwrap(), renderer.ctx.tip.as_ref().unwrap()));
        assert!(Arc::ptr_eq(ctx.dual_tip.as_ref().unwrap(), renderer.ctx.dual_tip.as_ref().unwrap()));
    }
    assert!(BrushContext::new(&BrushSettings::default()).tip.is_none());
}

#[test]
fn loading_or_replacing_a_stored_tip_never_reuses_its_preview_mips() {
    let preview = GrayTile::from_fn(8, 8, |_, _| 0.1);
    let stored = StoredTile { key: "unchanged-key".into(), width: 32, height: 32, preview: Arc::new(preview), full: None };
    let mut brush = BrushSettings { tip: TipShape::Stored(stored.clone()), ..Default::default() };
    let before = BrushContext::new(&brush);
    let full = GrayTile::from_fn(32, 32, |_, _| 0.9);
    brush.tip = TipShape::Stored(StoredTile { full: Some(Arc::new(full.clone())), ..stored.clone() });
    let loaded = BrushContext::new(&brush);
    assert!(!Arc::ptr_eq(before.tip.as_ref().unwrap(), loaded.tip.as_ref().unwrap()));
    assert_eq!(loaded.tip.as_ref().unwrap().levels, Mips::new(&full).levels);
    let replacement = GrayTile::from_fn(32, 32, |_, _| 0.3);
    brush.tip = TipShape::Stored(StoredTile { full: Some(Arc::new(replacement.clone())), ..stored });
    let replaced = BrushContext::new(&brush);
    assert!(!Arc::ptr_eq(loaded.tip.as_ref().unwrap(), replaced.tip.as_ref().unwrap()));
    assert_eq!(replaced.tip.as_ref().unwrap().levels, Mips::new(&replacement).levels);
}

#[test]
fn cached_and_uncached_strokes_match_exactly_at_native_depths_and_modes() {
    let tip = GrayTile::from_fn(31, 17, |x, y| ((x * 13 + y * 7) % 31) as f32 / 30.0);
    let brush = BrushSettings {
        tip: TipShape::Sampled(tip.clone()),
        dual_brush: DualBrush { enabled: true, size: 19.0, tip: TipShape::Sampled(tip.clone()), ..Default::default() },
        size: 27.0,
        angle: 0.4,
        roundness: 0.7,
        flow: 0.5,
        opacity: 0.7,
        wet_edges: true,
        noise: true,
        smoothing: Smoothing { amount: 0.49, ..Default::default() },
        seed: 42,
        ..Default::default()
    };
    let bounds = Rect::new(-32, -32, 96, 96);
    let selection = Surface::with_default(PixelFormat::GRAY8, &[0.6]);
    for fmt in [PixelFormat::RGBA8, PixelFormat::GRAY8, PixelFormat::CMYKA8] {
        for sample in [SampleType::U8, SampleType::U16, SampleType::F32] {
            let fmt = fmt.with_sample(sample);
            let mut pre = Surface::new(fmt);
            pre.fill_rect(bounds, &photocraft_raster::from_rgba(&fmt, [0.3, 0.6, 0.8, 0.7]));
            let render = |uncached: bool| {
                let mut r = StrokeRenderer::new(&brush, Some(fmt), 1.0);
                if uncached {
                    r.ctx.tip = Some(Arc::new(Mips::new(&tip)));
                    r.ctx.dual_tip = Some(Arc::new(Mips::new(&tip)));
                }
                let mut target = pre.clone();
                for i in 0..8 {
                    r.push(&[StrokePoint::new(-3.0 + f64::from(i) * 8.0, 24.0 + f64::from(i % 3) * 4.0, 0.4 + i as f32 * 0.07)]);
                    r.composite(&pre, &mut target, Some(&selection), false, false);
                    if let Some(mut tail) = r.tail_preview() {
                        tail.composite(&pre, &mut target, Some(&selection), false, false);
                    }
                }
                r.finish();
                r.composite(&pre, &mut target, Some(&selection), false, true);
                (r.bounds(), target.read_region(bounds))
            };
            let cold = render(false);
            let uncached = render(true);
            let warm = render(false);
            assert_eq!(uncached.0, cold.0);
            assert_eq!(cold.0, warm.0);
            assert!(uncached.1.iter().zip(&cold.1).all(|(a, b)| a.to_bits() == b.to_bits()), "cold {fmt:?}");
            assert!(cold.1.iter().zip(&warm.1).all(|(a, b)| a.to_bits() == b.to_bits()), "warm {fmt:?}");
            assert_ne!(cold.1, pre.read_region(bounds), "the regression must paint pixels");
        }
    }
    // A public-context caller still gets the identical sampled coverage at sub-pixel centres.
    let ctx = BrushContext::new(&brush);
    let dab = Dab::round(Point::new(31.25, 12.75), 9.5, 0.8);
    let mut cached = Vec::new();
    ctx.rasterize(&dab, false, ctx.dab_rect(&dab, false), &mut cached);
    let mut reference = ctx.clone();
    reference.tip = Some(Arc::new(Mips::new(&tip)));
    let mut uncached = Vec::new();
    reference.rasterize(&dab, false, reference.dab_rect(&dab, false), &mut uncached);
    assert!(cached.iter().zip(&uncached).all(|(a, b)| a.to_bits() == b.to_bits()));
}
