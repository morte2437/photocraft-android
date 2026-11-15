//! Stroke cost limits: the clip rectangle and the dab-area budget.

use super::*;

fn brush(size: f32, spacing: f32) -> BrushSettings {
    BrushSettings { size, spacing, hardness: 0.5, ..BrushSettings::default() }
}

fn line(r: &mut StrokeRenderer, from: f64, to: f64) {
    r.push(&[StrokePoint::new(from, 50.0, 1.0), StrokePoint::new(to, 50.0, 1.0)]);
    r.finish();
}

#[test]
fn clip_skips_far_dabs_and_leaves_the_pixels_inside_unchanged() {
    let b = brush(20.0, 0.25);
    let clip = Rect::new(0, 0, 200, 100);
    let mut plain = StrokeRenderer::new(&b, None, 1.0);
    let mut clipped = StrokeRenderer::new(&b, None, 1.0).with_clip(clip);
    // A path that runs 20000 px, almost all of it far outside the clip.
    line(&mut plain, -100.0, 20_000.0);
    line(&mut clipped, -100.0, 20_000.0);
    assert_eq!(plain.dab_count(), clipped.dab_count());
    for y in clip.y0..clip.y1 {
        for x in clip.x0..clip.x1 {
            assert_eq!(plain.coverage_at(x, y).to_bits(), clipped.coverage_at(x, y).to_bits(), "({x},{y})");
        }
    }
    assert!(plain.cov.tiles.len() > 20 * clipped.cov.tiles.len(), "{} vs {}", plain.cov.tiles.len(), clipped.cov.tiles.len());
    assert_eq!(clipped.coverage_at(10_000, 50), 0.0);
    assert!(clipped.bounds().x1 < 400, "{:?}", clipped.bounds());
}

#[test]
fn a_dab_that_straddles_the_clip_edge_is_painted_in_full_inside() {
    let b = brush(40.0, 0.25);
    let clip = Rect::new(0, 0, 100, 100);
    let mut plain = StrokeRenderer::new(&b, None, 1.0);
    let mut clipped = StrokeRenderer::new(&b, None, 1.0).with_clip(clip);
    plain.push(&[StrokePoint::new(110.0, 50.0, 1.0)]);
    clipped.push(&[StrokePoint::new(110.0, 50.0, 1.0)]);
    plain.finish();
    clipped.finish();
    assert!(plain.coverage_at(95, 50) > 0.0);
    assert_eq!(plain.coverage_at(95, 50).to_bits(), clipped.coverage_at(95, 50).to_bits());
}

#[test]
fn a_huge_brush_at_the_finest_spacing_over_a_long_path_stays_cheap_with_a_clip() {
    // 500 px brush at the 1 % minimum spacing over 2e6 px: 400 000 dabs; only those that reach the
    // clip are rasterized.
    let b = brush(500.0, 0.0);
    let mut r = StrokeRenderer::new(&b, None, 1.0).with_clip(Rect::new(0, 0, 256, 256));
    line(&mut r, -1_000_000.0, 1_000_000.0);
    assert!(r.dab_count() > 100_000, "{}", r.dab_count());
    assert!(r.cov.tiles.len() < 1000, "{}", r.cov.tiles.len());
    assert!(!r.budget_exhausted());
}

#[test]
fn the_budget_stops_painting_but_keeps_counting_dabs() {
    let b = brush(20.0, 0.25);
    // A 20 px brush has a dab rectangle of about 30x30; room for about ten of them.
    let mut r = StrokeRenderer::new(&b, None, 1.0).with_dab_area_budget(10_000);
    line(&mut r, 0.0, 2_000.0);
    assert!(r.budget_exhausted());
    assert!(r.dab_count() > 300, "{}", r.dab_count());
    assert!(r.coverage_at(1_000, 50) == 0.0);
    assert!(r.bounds().x1 < 200, "{:?}", r.bounds());
    // The first dabs did paint.
    assert!(r.coverage_at(2, 50) > 0.0);
    // Further input is ignored: no growth.
    let tiles = r.cov.tiles.len();
    line(&mut r, 3_000.0, 4_000.0);
    assert_eq!(r.cov.tiles.len(), tiles);
}

#[test]
fn dual_dabs_are_charged_apart_and_chunking_does_not_change_the_result() {
    let mut b = brush(20.0, 0.25);
    b.dual_brush.enabled = true;
    b.dual_brush.size = 10.0;
    let pts: Vec<_> = (0..200).map(|i| StrokePoint::new(f64::from(i) * 7.0, 50.0, 1.0)).collect();
    let run = |chunk: usize| {
        let mut r = StrokeRenderer::new(&b, None, 1.0).with_dab_area_budget(20_000);
        for c in pts.chunks(chunk) {
            r.push(c);
        }
        r.finish();
        r
    };
    let (whole, chunked) = (run(1000), run(3));
    assert!(whole.budget_exhausted());
    for x in 0..1400 {
        assert_eq!(whole.coverage_at(x, 50).to_bits(), chunked.coverage_at(x, 50).to_bits(), "x {x}");
        assert_eq!(whole.dual.as_ref().unwrap().get(x, 50).to_bits(), chunked.dual.as_ref().unwrap().get(x, 50).to_bits(), "dual x {x}");
    }
    assert!(whole.dual.as_ref().unwrap().get(2, 50) > 0.0);
}

#[test]
fn a_dab_too_big_for_what_is_left_does_not_end_the_stroke() {
    let (mut left, mut refused) = (1000, false);
    assert!(!admit(None, &mut left, &mut refused, Rect::new(0, 0, 100, 100)));
    assert!(refused);
    assert_eq!(left, 1000);
    assert!(admit(None, &mut left, &mut refused, Rect::new(0, 0, 10, 10)));
    assert_eq!(left, 900);
}

/// How much of the budget everyday strokes use, so a limit change is judged against numbers.
#[test]
fn everyday_strokes_use_a_small_part_of_the_budget() {
    // (size, spacing, path length in px): a 100 px brush at 1 % spacing over 6000 px, a 500 px
    // brush at 5 % over 20 000 px (a scribble across a large document), 20 px at 25 % over 1e6 px.
    for (size, spacing, len) in [(100.0, 0.01, 6_000.0), (500.0, 0.05, 20_000.0), (20.0, 0.25, 1.0e6)] {
        let b = brush(size, spacing);
        // The clip rejects every dab, so this measures the generator and `dab_rect` only.
        let mut r = StrokeRenderer::new(&b, None, 1.0).with_clip(Rect::new(0, 0, 1, 1)).record_dabs();
        r.push(&[StrokePoint::new(0.0, 0.0, 1.0), StrokePoint::new(len, 0.0, 1.0)]);
        r.finish();
        let used: u64 = r.dabs().iter().map(|d| r.ctx.dab_rect(d, false)).map(|q| u64::from(q.width()) * u64::from(q.height())).sum();
        println!(
            "size {size} spacing {spacing} len {len}: {} dabs, {used} px = {:.1} % of the budget",
            r.dab_count(),
            used as f64 * 100.0 / MAX_STROKE_DAB_AREA as f64
        );
        assert!(used < MAX_STROKE_DAB_AREA / 4, "{used}");
    }
}
