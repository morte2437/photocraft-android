use super::*;
use photocraft_color::{ColorMode, PixelFormat, SampleType};

/// The segment walk before #937: every dab, whether or not it can reach the field.
fn full_walk(f: &mut LiquifyField, s: &LiquifyStroke, a: [f64; 3], b: [f64; 3]) -> Rect {
    f.walk_segment(s, a, b, false)
}

#[test]
fn a_segment_walks_only_the_dabs_that_can_reach_the_field() {
    // #937: a segment to a far-off point walked one dab per spacing step along its whole
    // length (~3e14 for 1e15 px). Skipping the dabs that can't reach a field node must leave
    // every result bit-identical to the full walk.
    use LiquifyTool::*;
    let bounds = Rect::new(-7, 3, 89, 67);
    let segments: [([f64; 3], [f64; 3]); 10] = [
        ([10.0, 20.0, 1.0], [70.0, 50.0, 0.5]),        // inside
        ([-900.0, 30.0, 1.0], [40.0, 31.0, 1.0]),      // in from far left
        ([40.0, 30.0, 1.0], [40.0, 4000.0, 0.2]),      // out downwards
        ([-500.0, -500.0, 1.0], [600.0, 600.0, 1.0]),  // diagonal through
        ([-300.0, -9.0, 1.0], [300.0, -9.0, 1.0]),     // parallel, just above (the brush reaches)
        ([-300.0, -400.0, 1.0], [300.0, -400.0, 1.0]), // parallel, far above (reaches nothing)
        ([88.5, -50.0, 1.0], [88.5, 120.0, 1.0]),      // vertical, along the right edge
        ([200.0, 200.0, 1.0], [201.0, 900.0, 1.0]),    // entirely outside
        ([0.0, 0.0, 1.0], [0.25, 0.1, 1.0]),           // shorter than one step
        ([60.0, 40.0, 1.0], [-2000.0, 45.0, 1.0]),     // continues the stroke before it, out left
    ];
    for tool in [ForwardWarp, PushLeft, Reconstruct, Smooth, TwirlCw, TwirlCcw, Pucker, Bloat, Freeze, Thaw] {
        for size in [6.0, 40.0] {
            for (a, b) in segments {
                let s = LiquifyStroke::new(tool, size);
                let (mut fast, mut full) = (LiquifyField::new(bounds, 2.0), LiquifyField::new(bounds, 2.0));
                // Some displacement first, so Reconstruct and Smooth have something to undo.
                for f in [&mut fast, &mut full] {
                    f.stroke_segment(&LiquifyStroke::new(ForwardWarp, 30.0), [20.0, 20.0, 1.0], [60.0, 40.0, 1.0]);
                }
                let dirty = fast.stroke_segment(&s, a, b);
                assert_eq!(dirty, full_walk(&mut full, &s, a, b), "{tool:?} {size} {a:?} {b:?}");
                assert!(fast.d == full.d && fast.freeze == full.freeze, "{tool:?} {size} {a:?} {b:?}");
            }
        }
    }
}

/// The same path sampled by `parts` pointer events per corner-to-corner leg, unevenly.
fn sampled(path: &[[f64; 3]], parts: usize) -> Vec<Vec<f64>> {
    let mut out = vec![path[0].to_vec()];
    for w in path.windows(2) {
        let (a, b) = (w[0], w[1]);
        // Uneven steps, as a real pointer gives: t grows quadratically.
        for k in 1..=parts {
            let t = (k * k) as f64 / (parts * parts) as f64;
            out.push((0..3).map(|i| a[i] + (b[i] - a[i]) * t).collect());
        }
    }
    out
}

/// #2606: dabs restarted at every pointer event, so a fast mouse (few far-apart events) and a
/// slow one (many close ones) warped the same path differently. Spacing now runs on along the
/// whole stroke: any sampling of a path gives the same field.
#[test]
fn the_result_does_not_depend_on_how_finely_the_path_was_sampled() {
    use LiquifyTool::*;
    let path = [[20.0, 20.0, 1.0], [62.0, 26.0, 0.6], [70.0, 50.0, 0.9], [28.0, 55.0, 1.0], [30.0, 30.0, 1.0]];
    for tool in [ForwardWarp, PushLeft, TwirlCw, Bloat, Reconstruct, Smooth, Freeze] {
        for size in [12.0, 40.0] {
            let field = |parts: usize| {
                let mut warp = LiquifyStroke::new(ForwardWarp, 30.0);
                warp.points = vec![vec![30.0, 30.0], vec![50.0, 40.0]];
                let mut s = LiquifyStroke::new(tool, size);
                s.points = sampled(&path, parts);
                // Some displacement first, so Reconstruct and Smooth have something to change.
                LiquifyField::from_strokes(bounds(), 2.0, &[warp, s])
            };
            let coarse = field(1);
            for parts in [3, 17, 60] {
                let fine = field(parts);
                let dd = coarse.d.iter().zip(&fine.d).map(|(p, q)| (p[0] - q[0]).abs().max((p[1] - q[1]).abs())).fold(0.0f32, f32::max);
                let df = coarse.freeze.iter().zip(&fine.freeze).map(|(p, q)| (p - q).abs()).fold(0.0f32, f32::max);
                assert!(dd < 1e-3 && df < 1e-4, "{tool:?} {size}px, {parts} events per leg: field differs by {dd} px, mask by {df}");
            }
        }
    }
}

/// A long drag whose events land exactly on dab positions, along a diagonal: rounding must not
/// add or drop the dab at a segment's end, or the deep displacement a long Forward Warp builds
/// up differs by a whole dab between a slow and a fast pointer.
#[test]
fn a_long_diagonal_drag_gives_one_field_at_any_event_spacing() {
    let field = |step: f64| {
        let mut s = LiquifyStroke::new(LiquifyTool::ForwardWarp, 40.0);
        s.points = (0..=(300.0 / step) as usize).map(|k| vec![40.0 + k as f64 * step * 0.6, 20.0 + k as f64 * step * 0.8]).collect();
        LiquifyField::from_strokes(Rect::new(0, 0, 320, 300), 2.0, &[s])
    };
    let (slow, fast) = (field(2.0), field(60.0));
    let dd = slow.d.iter().zip(&fast.d).map(|(p, q)| (p[0] - q[0]).abs().max((p[1] - q[1]).abs())).fold(0.0f32, f32::max);
    assert!(slow.max_displacement() > 50.0, "the drag builds up a deep displacement");
    assert!(dd < 1e-3, "slow and fast pointer fields differ by {dd} px");
}

#[test]
fn a_huge_canvas_gets_a_coarser_field_not_a_huge_one() {
    // 400000² px at 2 px per node is 4·10¹⁰ nodes.
    let f = LiquifyField::new(Rect::new(0, 0, 400_000, 400_000), 2.0);
    assert!(f.w * f.h <= MAX_FIELD_NODES && f.d.len() == f.w * f.h && f.freeze.len() == f.d.len());
    assert!(f.cell > 64.0);
    assert_eq!(LiquifyField::new(Rect::new(0, 0, 100, 100), 2.0).cell, 2.0);
}

#[test]
fn a_stroke_interrupted_by_a_jump_restarts_its_spacing() {
    // A segment that does not start where the last one ended begins a new run of dabs there.
    let s = LiquifyStroke::new(LiquifyTool::ForwardWarp, 20.0);
    let mut f = LiquifyField::new(bounds(), 2.0);
    f.stroke_begin(&s, [10.0, 10.0, 1.0]);
    f.stroke_segment(&s, [10.0, 10.0, 1.0], [10.3, 10.0, 1.0]);
    let mut g = LiquifyField::new(bounds(), 2.0);
    f.stroke_segment(&s, [40.0, 30.0, 1.0], [50.0, 30.0, 1.0]);
    g.stroke_begin(&s, [40.0, 30.0, 1.0]);
    g.stroke_segment(&s, [40.0, 30.0, 1.0], [50.0, 30.0, 1.0]);
    assert_eq!(f, g);
    // Hostile points: nothing panics and the next segment starts afresh.
    f.stroke_segment(&s, [f64::NAN, 3.0, 1.0], [4.0, f64::INFINITY, 1.0]);
    f.stroke_segment(&s, [4.0, 3.0, f64::NAN], [9.0, 3.0, 1.0]);
}

#[test]
fn a_segment_to_a_far_off_point_returns() {
    // The issue's request: about 3.3e14 steps before the fix. Also a segment whose length
    // overflows f64.
    let mut f = LiquifyField::new(Rect::new(0, 0, 24, 16), 2.0);
    let s = LiquifyStroke::new(LiquifyTool::ForwardWarp, 30.0);
    let dirty = f.stroke_segment(&s, [0.0, 0.0, 1.0], [1e15, 0.0, 1.0]);
    assert!(!dirty.is_empty() && dirty.x1 <= 64, "{dirty:?}");
    f.stroke_segment(&s, [-1e308, 5.0, 1.0], [1e308, 5.0, 1.0]);
    f.stroke_segment(&LiquifyStroke::new(LiquifyTool::Bloat, 30.0), [5.0, -1e15, 1.0], [5.0, 1e15, 1.0]);
}

fn sample(st: SampleType) -> Surface {
    let mut s = Surface::new(PixelFormat::new(ColorMode::Rgb, st, true));
    for y in 0..64 {
        for x in 0..96 {
            let v = ((x / 4 + y / 4) % 2) as f32;
            s.write_pixel(x, y, &[v, (x as f32) / 96.0, (y as f32) / 64.0, 1.0]);
        }
    }
    s
}

fn bounds() -> Rect {
    Rect::new(0, 0, 96, 64)
}

fn worst(a: &Surface, b: &Surface, r: Rect) -> f32 {
    a.read_region(r).iter().zip(b.read_region(r)).map(|(x, y)| (x - y).abs()).fold(0.0, f32::max)
}

fn lasso(points: &[[f64; 2]], amount: Option<f64>) -> LiquifyStroke {
    let mut s = LiquifyStroke::new(LiquifyTool::LassoMask, 1.0);
    s.points = points.iter().map(|p| p.to_vec()).collect();
    s.amount = amount;
    s
}

/// Reference even-odd test (ray casting), to check the scanline fill against.
fn inside(p: [f64; 2], poly: &[[f64; 2]]) -> bool {
    let mut inside = false;
    let mut j = poly.len() - 1;
    for (i, a) in poly.iter().enumerate() {
        let b = poly[j];
        if (a[1] > p[1]) != (b[1] > p[1]) && p[0] < (b[0] - a[0]) * (p[1] - a[1]) / (b[1] - a[1]) + a[0] {
            inside = !inside;
        }
        j = i;
    }
    inside
}

#[test]
fn lasso_freezes_exactly_the_nodes_inside_a_concave_polygon() {
    // A concave "C" with a notch, at a non-integer cell size.
    let poly = [[10.0, 8.0], [70.0, 8.0], [70.0, 20.0], [30.0, 22.5], [33.0, 40.0], [80.0, 41.0], [60.0, 58.0], [9.0, 55.0]];
    for cell in [1.0, 2.0, 3.5] {
        let mut f = LiquifyField::new(bounds(), cell);
        let dirty = f.apply_stroke(&lasso(&poly, None));
        assert_eq!(dirty, Rect::new(9, 8, 80, 58));
        for j in 0..f.h {
            for i in 0..f.w {
                let want = if inside(f.node_pos(i, j), &poly) { 1.0 } else { 0.0 };
                assert_eq!(f.freeze[j * f.w + i], want, "cell {cell}: node ({i}, {j}) at {:?}", f.node_pos(i, j));
            }
        }
        assert!(f.freeze.contains(&1.0));
        // Strokes replay to the same field (undo rebuilds from the list).
        assert_eq!(LiquifyField::from_strokes(bounds(), cell, &[lasso(&poly, None)]), f);
    }
}

#[test]
fn lasso_thaws_with_amount_zero() {
    let mut f = LiquifyField::new(bounds(), 2.0);
    f.apply_stroke(&LiquifyStroke::new(LiquifyTool::FreezeAll, 1.0));
    assert!(f.freeze.iter().all(|&v| v == 1.0));
    let square = [[20.0, 20.0], [40.0, 20.0], [40.0, 40.0], [20.0, 40.0]];
    f.apply_stroke(&lasso(&square, Some(0.0)));
    assert_eq!(f.freeze_at(30.0, 30.0), 0.0, "thawed inside");
    assert_eq!(f.freeze_at(5.0, 5.0), 1.0, "still frozen outside");
}

#[test]
fn lasso_ignores_degenerate_and_hostile_polygons() {
    let empty = LiquifyField::new(bounds(), 2.0);
    let cases: [&[[f64; 2]]; 6] = [
        &[],
        &[[10.0, 10.0], [50.0, 50.0]],
        &[[f64::NAN, 10.0], [50.0, f64::INFINITY], [20.0, 30.0]],
        // Entirely outside the canvas, on every side.
        &[[-50.0, -50.0], [-10.0, -50.0], [-10.0, -10.0]],
        &[[200.0, 10.0], [300.0, 10.0], [250.0, 50.0]],
        &[[10.0, 1e12], [50.0, 1e12], [30.0, 2e12]],
    ];
    for poly in cases {
        let mut f = empty.clone();
        assert_eq!(f.apply_stroke(&lasso(poly, None)), Rect::EMPTY, "{poly:?}");
        assert_eq!(f, empty, "{poly:?}");
    }
    // Huge coordinates around the canvas clamp to it instead of overflowing.
    let mut f = empty.clone();
    f.apply_stroke(&lasso(&[[-1e15, -1e15], [1e15, -1e15], [1e15, 1e15], [-1e15, 1e15]], None));
    assert!(f.freeze.iter().all(|&v| v == 1.0));
    // A NaN point among good ones is skipped.
    let mut f = empty.clone();
    f.apply_stroke(&lasso(&[[10.0, 10.0], [f64::NAN, 0.0], [50.0, 10.0], [50.0, 50.0], [10.0, 50.0]], None));
    assert_eq!(f.freeze_at(30.0, 30.0), 1.0);
}

#[test]
fn the_brush_fades_to_nothing_at_its_rim_at_every_density() {
    for density in [0.0, 0.25, 0.5, 0.75, 1.0, f64::NAN, -3.0, 7.0] {
        assert_eq!(falloff(0.0, density), 1.0, "full strength at the centre ({density})");
        assert_eq!(falloff(1.0, density), 0.0, "nothing at the rim ({density})");
        // Flat at the rim: the last 2 % of the radius moves at most a couple of percent.
        assert!(falloff(0.98f64.powi(2), density) < 0.02, "{density}: {}", falloff(0.98f64.powi(2), density));
        let mut prev = 1.0;
        for k in 1..=100 {
            let v = falloff(k as f64 / 100.0, density);
            assert!(v <= prev && v >= 0.0, "{density}: not falling at t² = {k}%");
            prev = v;
        }
    }
    // Density widens the strong core: more strength at every radius, as in Photoshop.
    for k in 1..100 {
        let t2 = k as f64 / 100.0;
        assert!(falloff(t2, 1.0) >= falloff(t2, 0.5) && falloff(t2, 0.5) >= falloff(t2, 0.0), "t² = {t2}");
    }
    assert!(falloff(0.5, 1.0) > 0.5 && falloff(0.5, 0.0) < 0.2);
}

/// #2606: the dab's strength used to stop at 1 - Density at its rim, so a Forward Warp moved
/// everything just inside the brush and nothing just outside, tearing the image along every
/// dab's circle (a hard-edged block at Density 0, half strength at the default 50).
#[test]
fn forward_warp_leaves_the_brush_rim_in_place() {
    // A drag of one dab spacing (a twentieth of the radius): one dab, centred where it started.
    let (r, drag) = (20.0, 1.0);
    for density in [0.0, 50.0, 100.0] {
        let mut s = LiquifyStroke::new(LiquifyTool::ForwardWarp, 2.0 * r);
        s.density = density;
        s.points = vec![vec![48.0, 32.0], vec![48.0 + drag, 32.0]];
        let f = LiquifyField::from_strokes(bounds(), 1.0, &[s]);
        let mut rim = 0;
        for j in 0..f.h {
            for i in 0..f.w {
                let p = f.node_pos(i, j);
                let t = (p[0] - 48.0).hypot(p[1] - 32.0) / r;
                if (0.97..1.0).contains(&t) {
                    let d = f.d[j * f.w + i];
                    assert!(f64::from(d[0].hypot(d[1])) < 0.03 * drag, "density {density}: a node at {t:.3} r moved {d:?}");
                    rim += 1;
                }
            }
        }
        assert!(rim > 20, "{rim} rim nodes");
        assert!(f.sample(48.0, 32.0)[0] < -0.9 * drag, "density {density}: the centre follows the brush");
    }
}

#[test]
fn a_wide_integer_translation_keeps_channels_and_tile_edges_at_every_format() {
    let b = Rect::new(13, -1, 13 + 70 * 256, 1);
    let w = b.width() as usize;
    for mode in [ColorMode::Grayscale, ColorMode::Rgb, ColorMode::Cmyk] {
        for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
            let format = PixelFormat::new(mode, depth, true);
            let n = format.channels();
            let mut data = Vec::new();
            for i in 0..w * 2 {
                let alpha = if i % 13 == 0 { 0.0 } else { 0.5 };
                for c in 0..mode.color_channels() {
                    data.push(if alpha == 0.0 { 0.0 } else { ((i * 19 + c * 43) % 251 + 2) as f32 / 255.0 });
                }
                data.push(alpha);
            }
            let mut source = Surface::new(format);
            source.write_region(b, &data);
            source.write_pixel(b.x0 - 1, b.y0 - 1, &vec![0.75; n]);
            let before = source.read_region(b);
            let mut field = LiquifyField::new(b, 2.5);
            field.d.fill([-1.0, 0.0]);
            let output = apply_liquify(&source, &field);
            let after = output.read_region(b);
            let tolerance = if depth == SampleType::F32 { 4.0 * f32::EPSILON } else { 0.0 };
            for (original, moved) in before.chunks_exact(w * n).zip(after.chunks_exact(w * n)) {
                for (x, pixel) in moved.chunks_exact(n).enumerate() {
                    let offset = x.saturating_sub(1) * n;
                    for (actual, expected) in pixel.iter().zip(&original[offset..offset + n]) {
                        assert!((actual - expected).abs() <= tolerance, "{mode:?}/{depth:?} at column {x}");
                    }
                }
            }
            assert_eq!(output.pixel(b.x0 - 1, b.y0 - 1), source.pixel(b.x0 - 1, b.y0 - 1));
        }
    }
}

#[test]
fn no_strokes_is_identity_at_every_depth() {
    for st in [SampleType::U8, SampleType::U16, SampleType::F32] {
        let s = sample(st);
        let f = LiquifyField::new(bounds(), 2.0);
        assert_eq!(apply_liquify(&s, &f), s);
        // A field with exactly-zero strokes (zero pressure) is still identity.
        let mut st0 = LiquifyStroke::new(LiquifyTool::ForwardWarp, 30.0);
        st0.pressure = 0.0;
        st0.points = vec![vec![40.0, 30.0], vec![60.0, 30.0]];
        let f = LiquifyField::from_strokes(bounds(), 2.0, &[st0]);
        assert!(f.is_identity());
    }
}

#[test]
fn full_resolution_preview_matches_the_unit_scale_proxy_and_partial_updates() {
    let canvas = Rect::new(0, 0, 96, 64);
    for sample_type in [SampleType::U8, SampleType::U16, SampleType::F32] {
        let source = sample(sample_type);
        let mut stroke = LiquifyStroke::new(LiquifyTool::ForwardWarp, 32.0);
        stroke.points = vec![vec![38.0, 30.0, 1.0], vec![52.0, 34.0, 0.8]];
        let mut field = LiquifyField::from_strokes(canvas, 2.0, &[stroke]);

        let proxy = ProxyImage::new(&source, canvas, 96);
        assert_eq!(proxy.scale, 1.0);
        let mut want = vec![[0; 4]; proxy.w * proxy.h];
        proxy.render(&field, [0, 0, proxy.w, proxy.h], &mut want);

        let mut got = vec![[0; 4]; proxy.w * proxy.h];
        let mut scratch = Vec::new();
        assert!(ProxyImage::render_full_resolution(&source, &field, canvas, canvas, &mut got, &mut scratch));
        assert_eq!(got, want, "full-resolution render at {sample_type:?}");

        let mut more = LiquifyStroke::new(LiquifyTool::Bloat, 16.0);
        more.points = vec![vec![43.0, 32.0, 1.0]];
        let dirty = field.apply_stroke(&more);
        proxy.render(&field, proxy.proxy_rect(dirty), &mut want);
        assert!(ProxyImage::render_full_resolution(&source, &field, canvas, dirty, &mut got, &mut scratch));
        assert_eq!(got, want, "partial update at {sample_type:?}");
    }
}

#[test]
fn forward_warp_displaces_along_the_stroke_and_reconstruct_restores() {
    let mut s = LiquifyStroke::new(LiquifyTool::ForwardWarp, 40.0);
    s.points = vec![vec![40.0, 32.0], vec![52.0, 32.0]];
    let mut f = LiquifyField::from_strokes(bounds(), 2.0, &[s]);
    // Content moved right: the output at the end point samples from the left.
    let d = f.sample(52.0, 32.0);
    assert!(d[0] < -4.0 && d[1].abs() < 0.5, "{d:?}");
    // A pixel-level check: a marker column follows the stroke.
    let mut img = Surface::new(PixelFormat::new(ColorMode::Grayscale, SampleType::F32, true));
    img.fill_rect(bounds(), &[0.0, 1.0]);
    img.fill_rect(Rect::new(40, 28, 42, 36), &[1.0, 1.0]);
    let out = apply_liquify(&img, &f);
    let brightest = (30..70).max_by(|a, b| out.pixel(*a, 32)[0].total_cmp(&out.pixel(*b, 32)[0])).unwrap();
    assert!(brightest > 44, "marker moved right: {brightest}");
    // Reconstruct All restores the original exactly.
    let mut rec = LiquifyStroke::new(LiquifyTool::ReconstructAll, 10.0);
    rec.amount = Some(100.0);
    f.apply_stroke(&rec);
    assert!(f.is_identity());
    assert_eq!(apply_liquify(&img, &f), img);
}

#[test]
fn reconstruct_brush_reduces_the_field_and_freeze_protects() {
    let mut s = LiquifyStroke::new(LiquifyTool::ForwardWarp, 40.0);
    s.points = vec![vec![30.0, 32.0], vec![45.0, 32.0]];
    let mut f = LiquifyField::from_strokes(bounds(), 2.0, std::slice::from_ref(&s));
    let before = f.max_displacement();
    let mut r = LiquifyStroke::new(LiquifyTool::Reconstruct, 80.0);
    r.points = vec![vec![40.0, 32.0]; 30];
    f.apply_stroke(&r);
    assert!(f.max_displacement() < before * 0.2, "{} vs {before}", f.max_displacement());
    // Frozen area doesn't move.
    let mut fr = LiquifyStroke::new(LiquifyTool::Freeze, 60.0);
    fr.density = 100.0;
    fr.points = vec![vec![40.0, 32.0]];
    let mut g = LiquifyField::new(bounds(), 2.0);
    g.apply_stroke(&fr);
    assert!(g.freeze_at(40.0, 32.0) > 0.99);
    g.apply_stroke(&s);
    assert!(g.sample(40.0, 32.0)[0].abs() < 1e-3);
    // Thaw unfreezes.
    let mut th = fr.clone();
    th.tool = LiquifyTool::Thaw;
    g.apply_stroke(&th);
    assert!(g.freeze_at(40.0, 32.0) < 0.01);
    // Mask All / Invert / None.
    g.apply_stroke(&LiquifyStroke::new(LiquifyTool::FreezeAll, 1.0));
    assert!(g.freeze.iter().all(|f| *f == 1.0));
    g.apply_stroke(&LiquifyStroke::new(LiquifyTool::InvertFreeze, 1.0));
    assert!(g.freeze.iter().all(|f| *f == 0.0));
    g.apply_stroke(&fr);
    g.apply_stroke(&LiquifyStroke::new(LiquifyTool::InvertFreeze, 1.0));
    assert!(g.freeze_at(40.0, 32.0) < 0.01 && g.freeze_at(5.0, 5.0) > 0.99);
    g.apply_stroke(&LiquifyStroke::new(LiquifyTool::ThawAll, 1.0));
    assert!(g.freeze.iter().all(|f| *f == 0.0));
}

#[test]
fn fully_frozen_pixels_keep_their_mapping_when_an_adjacent_area_is_warped() {
    let bounds = bounds();
    let mut src = Surface::new(PixelFormat::new(ColorMode::Rgb, SampleType::F32, true));
    for y in 0..bounds.height() {
        for x in 0..bounds.width() {
            src.write_pixel(x as i32, y as i32, &[x as f32 / bounds.width() as f32, y as f32 / bounds.height() as f32, 0.0, 1.0]);
        }
    }

    let mut mask_only = LiquifyField::new(bounds, 2.0);
    mask_only.apply_stroke(&lasso(&[[0.0, 0.0], [40.0, 0.0], [40.0, 64.0], [0.0, 64.0]], None));
    assert_eq!(apply_liquify(&src, &mask_only), src);

    let mut field = LiquifyField::new(bounds, 2.0);
    let mut first_warp = LiquifyStroke::new(LiquifyTool::ForwardWarp, 40.0);
    first_warp.points = vec![vec![22.0, 32.0], vec![34.0, 32.0]];
    field.apply_stroke(&first_warp);

    field.apply_stroke(&lasso(&[[0.0, 0.0], [40.0, 0.0], [40.0, 64.0], [0.0, 64.0]], None));
    let frozen_pixel = (37, 32);
    assert!(field.freeze_at(f64::from(frozen_pixel.0) + 0.5, f64::from(frozen_pixel.1) + 0.5) > 0.999);
    let before = apply_liquify(&src, &field).pixel(frozen_pixel.0, frozen_pixel.1);
    assert_ne!(before, src.pixel(frozen_pixel.0, frozen_pixel.1), "freezing must retain the earlier warp");
    let neighbor_before = apply_liquify(&src, &field).pixel(43, 32);

    let mut adjacent_warp = LiquifyStroke::new(LiquifyTool::ForwardWarp, 24.0);
    adjacent_warp.points = vec![vec![44.0, 32.0], vec![52.0, 32.0]];
    field.apply_stroke(&adjacent_warp);

    let after = apply_liquify(&src, &field).pixel(frozen_pixel.0, frozen_pixel.1);
    let neighbor_after = apply_liquify(&src, &field).pixel(43, 32);
    assert_eq!(before, after, "a fully frozen pixel must keep its pre-neighbor-warp mapping");
    assert_ne!(neighbor_before, neighbor_after, "an adjacent unfrozen pixel must still move");
}

#[test]
fn twirl_pucker_bloat_and_push_left_have_the_right_sense() {
    let at = |tool: LiquifyTool, pts: Vec<Vec<f64>>| {
        let mut s = LiquifyStroke::new(tool, 40.0);
        s.points = pts;
        LiquifyField::from_strokes(bounds(), 2.0, &[s])
    };
    let c = vec![vec![48.0, 32.0]; 10];
    // Pucker pulls content in: the output samples outward (d points away from the centre).
    let f = at(LiquifyTool::Pucker, c.clone());
    assert!(f.sample(58.0, 32.0)[0] > 0.1);
    let f = at(LiquifyTool::Bloat, c.clone());
    assert!(f.sample(58.0, 32.0)[0] < -0.1);
    // Twirl clockwise (y down): content right of centre moves down, so it samples from above.
    let f = at(LiquifyTool::TwirlCw, c.clone());
    assert!(f.sample(58.0, 32.0)[1] < -0.1, "{:?}", f.sample(58.0, 32.0));
    let f = at(LiquifyTool::TwirlCcw, c);
    assert!(f.sample(58.0, 32.0)[1] > 0.1);
    // Push Left while dragging up moves content left (samples from the right).
    let f = at(LiquifyTool::PushLeft, vec![vec![48.0, 44.0], vec![48.0, 20.0]]);
    assert!(f.sample(48.0, 32.0)[0] > 0.5, "{:?}", f.sample(48.0, 32.0));
    // Smooth reduces sharp field differences.
    let mut s = LiquifyStroke::new(LiquifyTool::ForwardWarp, 12.0);
    s.points = vec![vec![40.0, 32.0], vec![50.0, 32.0]];
    let mut g = LiquifyField::from_strokes(bounds(), 2.0, &[s]);
    let m0 = g.max_displacement();
    let mut sm = LiquifyStroke::new(LiquifyTool::Smooth, 60.0);
    sm.points = vec![vec![45.0, 32.0]; 20];
    g.apply_stroke(&sm);
    assert!(g.max_displacement() < m0);
}

#[test]
fn replay_is_deterministic_and_depths_agree() {
    let mut s = LiquifyStroke::new(LiquifyTool::TwirlCw, 50.0);
    s.points = vec![vec![48.0, 32.0, 0.5], vec![50.0, 33.0, 1.0], vec![52.0, 34.0]];
    let a = LiquifyField::from_strokes(bounds(), 2.0, std::slice::from_ref(&s));
    let b = LiquifyField::from_strokes(bounds(), 2.0, std::slice::from_ref(&s));
    assert_eq!(a, b);
    let r8 = apply_liquify(&sample(SampleType::U8), &a);
    let r32 = apply_liquify(&sample(SampleType::F32), &a);
    assert!(worst(&r8, &r32, bounds()) < 2.0 / 255.0 + 1e-4);
}

#[test]
fn proxy_matches_the_full_render_roughly() {
    let src = sample(SampleType::U8);
    let mut s = LiquifyStroke::new(LiquifyTool::Bloat, 40.0);
    s.points = vec![vec![48.0, 32.0]; 8];
    let f = LiquifyField::from_strokes(bounds(), 2.0, &[s]);
    let p = ProxyImage::new(&src, bounds(), 96);
    assert_eq!((p.w, p.h, p.scale), (96, 64, 1.0));
    let mut out = vec![[0u8; 4]; p.w * p.h];
    p.render(&f, [0, 0, p.w, p.h], &mut out);
    let full = apply_liquify(&src, &f);
    let mut bad = 0;
    for y in 0..64 {
        for x in 0..96 {
            let q = full.pixel(x, y);
            let o = out[y as usize * 96 + x as usize];
            if (f32::from(o[1]) / 255.0 - q[1]).abs() > 0.1 {
                bad += 1;
            }
        }
    }
    assert!(bad < 96 * 64 / 50, "{bad}");
}
