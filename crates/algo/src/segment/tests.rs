use super::gmm::Gmm;
use super::maxflow::Graph;
use super::*;
use photocraft_geom::Rect;

// ---------- max-flow ----------

#[test]
fn maxflow_tiny_graph() {
    // s→a 3, s→b 2, a→b 1, a→t 2, b→t 3: max flow 5.
    let mut g = Graph::with_capacity(2, 1);
    g.add_tweights(0, 3.0, 2.0);
    g.add_tweights(1, 2.0, 3.0);
    g.add_edge(0, 1, 1.0, 0.0);
    assert_eq!(g.maxflow(), 5.0);
}

#[test]
fn maxflow_clrs_network() {
    // The classic textbook network (max flow 23): v1..v4 = 0..3.
    let mut g = Graph::with_capacity(4, 5);
    g.add_tweights(0, 16.0, 0.0);
    g.add_tweights(1, 13.0, 0.0);
    g.add_tweights(2, 0.0, 20.0);
    g.add_tweights(3, 0.0, 4.0);
    g.add_edge(0, 1, 10.0, 4.0);
    g.add_edge(0, 2, 12.0, 0.0);
    g.add_edge(2, 1, 9.0, 0.0);
    g.add_edge(1, 3, 14.0, 0.0);
    g.add_edge(3, 2, 7.0, 0.0);
    assert_eq!(g.maxflow(), 23.0);
    // Min cut {s, v1, v2, v4} | {v3, t}.
    assert!(g.in_source(0) && g.in_source(1) && g.in_source(3));
    assert!(!g.in_source(2));
    assert_eq!(g.residual_out_of_source_side(), 0.0);
}

#[test]
fn maxflow_can_be_cancelled_and_otherwise_cuts_the_same() {
    // A 120 × 100 grid with terminal links both ways (push/relabel) and a small one (BK).
    for (w, h) in [(120usize, 100usize), (20, 15)] {
        let mut g = Graph::with_capacity(w * h, w * h * 2);
        let mut z = 7u64;
        let mut next = || {
            z = z.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            (z >> 40) as f32 / (1u64 << 24) as f32
        };
        for i in 0..w * h {
            g.add_tweights(i, next() * 2.0, next() * 2.0);
            if i % w + 1 < w {
                g.add_edge(i, i + 1, next(), next());
            }
            if i + w < w * h {
                g.add_edge(i, i + w, next(), next());
            }
        }
        let (mut plain, mut asked) = (g.clone(), g.clone());
        let flow = plain.maxflow();
        assert_eq!(asked.maxflow_with(&|| false), Some(flow));
        assert!((0..w * h).all(|i| plain.in_source(i) == asked.in_source(i)));
        if w * h > 4096 {
            assert_eq!(g.maxflow_with(&|| true), None);
        }
    }
}

/// Reference max flow (Edmonds–Karp on a dense matrix; node 0 = s, n−1 = t).
fn edmonds_karp(cap: &mut [Vec<f64>]) -> f64 {
    let n = cap.len();
    let mut flow = 0.0;
    loop {
        let mut prev = vec![usize::MAX; n];
        prev[0] = 0;
        let mut q = std::collections::VecDeque::from([0usize]);
        while let Some(u) = q.pop_front() {
            for v in 0..n {
                if prev[v] == usize::MAX && cap[u][v] > 1e-9 {
                    prev[v] = u;
                    q.push_back(v);
                }
            }
        }
        if prev[n - 1] == usize::MAX {
            return flow;
        }
        let mut b = f64::MAX;
        let mut v = n - 1;
        while v != 0 {
            b = b.min(cap[prev[v]][v]);
            v = prev[v];
        }
        let mut v = n - 1;
        while v != 0 {
            cap[prev[v]][v] -= b;
            cap[v][prev[v]] += b;
            v = prev[v];
        }
        flow += b;
    }
}

#[test]
fn maxflow_matches_reference_on_random_graphs() {
    let mut rng = Rng::new(7);
    for trial in 0..60 {
        let n = 3 + (trial % 12);
        let mut g = Graph::with_capacity(n, n * 3);
        let mut cap = vec![vec![0.0f64; n + 2]; n + 2];
        let mut edges = Vec::new();
        for i in 0..n {
            let (s, t) = ((rng.f32() * 10.0).round(), (rng.f32() * 10.0).round());
            let (s, t) = (if rng.f32() < 0.4 { 0.0 } else { s }, if rng.f32() < 0.4 { 0.0 } else { t });
            g.add_tweights(i, s, t);
            cap[0][i + 1] += s as f64;
            cap[i + 1][n + 1] += t as f64;
        }
        for i in 0..n {
            for j in i + 1..n {
                if rng.f32() < 0.35 {
                    let (a, b) = ((rng.f32() * 8.0).round(), (rng.f32() * 8.0).round());
                    g.add_edge(i, j, a, b);
                    cap[i + 1][j + 1] += a as f64;
                    cap[j + 1][i + 1] += b as f64;
                    edges.push((i, j, a, b));
                }
            }
        }
        let orig = cap.clone();
        let reference = edmonds_karp(&mut cap);
        let f = g.maxflow();
        assert!((f - reference).abs() < 1e-3, "trial {trial}: {f} vs {reference}");
        // The reported cut has exactly that capacity.
        let side = |i: usize| g.in_source(i);
        let mut cut = 0.0;
        for i in 0..n {
            if side(i) {
                cut += orig[i + 1][n + 1];
            } else {
                cut += orig[0][i + 1];
            }
        }
        for &(i, j, a, b) in &edges {
            if side(i) && !side(j) {
                cut += a as f64;
            }
            if side(j) && !side(i) {
                cut += b as f64;
            }
        }
        assert!((cut - reference).abs() < 1e-3, "trial {trial}: cut {cut} vs flow {reference}");
    }
}

#[test]
fn grid_cut_separates_two_halves() {
    let img = RgbImage::from_fn(20, 10, |x, _| if x < 10 { [0.9, 0.1, 0.1] } else { [0.1, 0.1, 0.9] });
    let n = img.w * img.h;
    let mut fixed = vec![FREE; n];
    fixed[5 * 20 + 1] = HARD_FG;
    fixed[5 * 20 + 18] = HARD_BG;
    let zeros = vec![0.0; n];
    let beta = contrast_beta(&img);
    let cut = grid_cut(&img, &zeros, &zeros, &fixed, 50.0, beta);
    for y in 0..10 {
        for x in 0..20 {
            assert_eq!(cut[y * 20 + x], x < 10, "({x},{y})");
        }
    }
}

// ---------- GMM ----------

#[test]
fn gmm_recovers_synthetic_clusters() {
    let mut rng = Rng::new(3);
    let truth = [([0.2f32, 0.3, 0.8], 0.03f32, 0.5f32), ([0.8, 0.2, 0.1], 0.05, 0.3), ([0.5, 0.9, 0.4], 0.02, 0.2)];
    let mut samples = Vec::new();
    for (mean, sd, w) in truth {
        for _ in 0..(w * 6000.0) as usize {
            samples.push(mean.map(|m| m + sd * rng.normal()));
        }
    }
    let g = Gmm::fit(&samples, 3, gmm::DEFAULT_REG).unwrap();
    assert_eq!(g.comps.len(), 3);
    for (mean, sd, w) in truth {
        let c = g.comps.iter().min_by(|a, b| d2(a.mean, mean).total_cmp(&d2(b.mean, mean))).unwrap();
        assert!(d2(c.mean, mean).sqrt() < 0.01, "{:?} vs {mean:?}", c.mean);
        assert!((c.weight - w).abs() < 0.03, "{} vs {w}", c.weight);
        let var = c.cov[0][0];
        assert!((var.sqrt() - sd).abs() < 0.01, "sd {} vs {sd}", var.sqrt());
    }
    assert!(g.neg_log([0.2, 0.3, 0.8]) < g.neg_log([0.5, 0.5, 0.5]));
    // Refit keeps the model stable.
    let r = g.refit(&samples).unwrap();
    assert_eq!(r.comps.len(), 3);
}

// ---------- SLIC ----------

#[test]
fn slic_counts_compactness_and_edges() {
    let mut rng = Rng::new(11);
    let (w, h) = (120usize, 120usize);
    let img = RgbImage::from_fn(w, h, |x, _| if x < 60 { [0.8, 0.3, 0.2] } else { [0.2, 0.5, 0.8] });
    let img = RgbImage { px: img.px.iter().map(|p| p.map(|v| (v + 0.02 * rng.normal()).clamp(0.0, 1.0))).collect(), ..img };
    let sp = slic::slic(&img, 64, 10.0, 10);
    assert!((40..=90).contains(&sp.count), "count {}", sp.count);
    let mut bbox = vec![(usize::MAX, usize::MAX, 0usize, 0usize); sp.count];
    let mut left = vec![0usize; sp.count];
    let mut total = vec![0usize; sp.count];
    for y in 0..h {
        for x in 0..w {
            let l = sp.labels[y * w + x] as usize;
            let b = &mut bbox[l];
            *b = (b.0.min(x), b.1.min(y), b.2.max(x), b.3.max(y));
            total[l] += 1;
            if x < 60 {
                left[l] += 1;
            }
        }
    }
    let s = sp.step;
    for (l, b) in bbox.iter().enumerate() {
        assert!(total[l] > 0);
        assert!(((b.2 - b.0 + 1) as f32) <= 3.0 * s && ((b.3 - b.1 + 1) as f32) <= 3.0 * s, "superpixel {l} too elongated: {b:?} (S={s})");
        // Superpixels respect the strong colour edge.
        let purity = left[l].max(total[l] - left[l]) as f32 / total[l] as f32;
        assert!(purity > 0.95, "superpixel {l} straddles the edge ({purity})");
    }
}

// ---------- GrabCut / object selection ----------

/// A textured disc on a noisy background: (image, ground-truth mask).
pub(crate) fn disc_scene(w: usize, h: usize, cx: f32, cy: f32, r: f32, seed: u64) -> (RgbImage, Vec<bool>) {
    let mut rng = Rng::new(seed);
    let mut img = RgbImage::new(w, h);
    let mut truth = vec![false; w * h];
    for y in 0..h {
        for x in 0..w {
            let (dx, dy) = (x as f32 + 0.5 - cx, y as f32 + 0.5 - cy);
            let inside = dx * dx + dy * dy <= r * r;
            truth[y * w + x] = inside;
            let n = 0.05 * rng.normal();
            img.px[y * w + x] = if inside {
                // Stripes between two warm colours plus noise.
                let t = (((x + y) as f32 * 0.35).sin() * 0.5 + 0.5).clamp(0.0, 1.0);
                [0.85 + 0.05 * t + n, 0.2 + 0.3 * t + n, 0.15 + n]
            } else {
                [0.25 + n, 0.45 + n, 0.7 + n]
            }
            .map(|v| v.clamp(0.0, 1.0));
        }
    }
    (img, truth)
}

pub(crate) fn iou(region: &Region, truth: &[bool], w: usize, h: usize) -> f32 {
    let (mut inter, mut uni) = (0usize, 0usize);
    for y in 0..h {
        for x in 0..w {
            let a = region.at(x as i32, y as i32) >= 0.5;
            let b = truth[y * w + x];
            inter += (a && b) as usize;
            uni += (a || b) as usize;
        }
    }
    inter as f32 / uni.max(1) as f32
}

#[test]
fn grabcut_trimap_on_disc() {
    let (img, truth) = disc_scene(120, 100, 60.0, 50.0, 30.0, 1);
    let mut tri = vec![grabcut::BG; img.w * img.h];
    for y in 12..88 {
        for x in 22..98 {
            tri[y * img.w + x] = grabcut::PR_FG;
        }
    }
    grabcut::grabcut(&img, &mut tri, 8).unwrap();
    let (mut inter, mut uni) = (0, 0);
    for (t, g) in tri.iter().zip(&truth) {
        let a = *t == grabcut::PR_FG || *t == grabcut::FG;
        inter += (a && *g) as usize;
        uni += (a || *g) as usize;
    }
    let iou = inter as f32 / uni as f32;
    assert!(iou > 0.95, "IoU {iou}");
}

#[test]
fn grabcut_small_disc_mask_is_pinned() {
    // The mask GrabCut produced with the push/relabel solver, before the switch to BK. Min-cut
    // ties may break differently between solvers, so any drift here is a visible change.
    let (img, _) = disc_scene(32, 24, 16.0, 12.0, 8.0, 3);
    let mut tri = vec![grabcut::BG; img.w * img.h];
    for y in 2..22 {
        for x in 6..26 {
            tri[y * img.w + x] = grabcut::PR_FG;
        }
    }
    grabcut::grabcut(&img, &mut tri, 8).unwrap();
    let got: Vec<String> = tri.chunks(img.w).map(|r| r.iter().map(|t| if *t == grabcut::PR_FG || *t == grabcut::FG { '#' } else { '.' }).collect()).collect();
    let want = [
        "................................",
        "................................",
        "................................",
        "................................",
        ".............######.............",
        "...........##########...........",
        "..........############..........",
        ".........##############.........",
        ".........##############.........",
        "........################........",
        "........################........",
        "........################........",
        "........################........",
        "........################........",
        "........################........",
        ".........##############.........",
        ".........##############.........",
        "..........############..........",
        "...........##########...........",
        ".............######.............",
        "................................",
        "................................",
        "................................",
        "................................",
    ];
    assert_eq!(got, want);
}

#[test]
fn grabcut_large_disc_mask_is_pinned() {
    // 5168 free nodes with terminal links both ways, so the pre-BK solver was push/relabel here.
    // The count and FNV-1a hash are of the mask that push/relabel produced at the merge base.
    let (img, _) = disc_scene(96, 80, 48.0, 40.0, 26.0, 3);
    let mut tri = vec![grabcut::BG; img.w * img.h];
    for y in 6..74 {
        for x in 10..86 {
            tri[y * img.w + x] = grabcut::PR_FG;
        }
    }
    grabcut::grabcut(&img, &mut tri, 8).unwrap();
    let fg: Vec<bool> = tri.iter().map(|t| *t == grabcut::PR_FG || *t == grabcut::FG).collect();
    let hash = fg.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ u64::from(*b)).wrapping_mul(0x0100_0000_01b3));
    assert_eq!((fg.iter().filter(|b| **b).count(), hash), (2128, 0x62fd_c64a_0ee4_4e75));
}

#[test]
fn object_select_disc_full_resolution() {
    let (img, truth) = disc_scene(200, 160, 100.0, 80.0, 50.0, 2);
    let s = ImageSampler { img: &img, origin: (0, 0) };
    let canvas = Rect::new(0, 0, 200, 160);
    let reg = grabcut::object_select(&s, canvas, Rect::new(40, 20, 160, 140), 1_000_000).unwrap();
    let v = iou(&reg, &truth, 200, 160);
    assert!(v > 0.95, "IoU {v}");
}

#[test]
fn object_select_downsampled_with_band_refinement() {
    let (img, truth) = disc_scene(640, 480, 330.0, 250.0, 150.0, 3);
    let s = ImageSampler { img: &img, origin: (0, 0) };
    let canvas = Rect::new(0, 0, 640, 480);
    // A small working budget forces step > 1 and the full-resolution band re-cut.
    let reg = grabcut::object_select(&s, canvas, Rect::new(160, 80, 500, 420), 20_000).unwrap();
    let v = iou(&reg, &truth, 640, 480);
    assert!(v > 0.95, "IoU {v}");
    // Boundary accuracy: few pixels wrong far (> 2 px) from the true edge.
    let mut far_wrong = 0;
    for y in 0..480 {
        for x in 0..640 {
            let (dx, dy) = (x as f32 + 0.5 - 330.0, y as f32 + 0.5 - 250.0);
            let d = (dx * dx + dy * dy).sqrt() - 150.0;
            if d.abs() > 2.0 && (reg.at(x, y) >= 0.5) != truth[y as usize * 640 + x as usize] {
                far_wrong += 1;
            }
        }
    }
    assert!(far_wrong < 300, "{far_wrong} pixels wrong away from the edge");
}

// ---------- Quick selection ----------

/// Left: textured warm region; right: cool region; strong vertical edge at x = 60.
fn two_regions(w: usize, h: usize) -> RgbImage {
    let mut rng = Rng::new(5);
    RgbImage::from_fn(w, h, |x, y| {
        let n = 0.04 * rng.normal();
        let t = ((x as f32 * 0.5).sin() * (y as f32 * 0.3).cos()) * 0.05;
        if x < 60 { [0.8 + t + n, 0.5 + n, 0.3 + t] } else { [0.2 + n, 0.35 + t, 0.6 + n] }.map(|v| v.clamp(0.0, 1.0))
    })
}

#[test]
fn quick_select_with_points_far_off_canvas_selects_nothing() {
    // A cast saturates to i32::MAX; adding one to it once overflowed.
    let img = two_regions(140, 100);
    let s = ImageSampler { img: &img, origin: (0, 0) };
    let canvas = Rect::new(0, 0, 140, 100);
    for p in [(3e9, 0.0), (0.0, 3e9), (f32::MAX, f32::MAX)] {
        assert!(quick::quick_select(&s, canvas, &[p], 8.0).is_none(), "{p:?}");
    }
}

#[test]
fn quick_select_cuts_window_sizes_in_parallel_with_the_same_result() {
    // Soft gradients and faint blobs let a selection spread, so most strokes need more than one
    // window size; cutting several at once must keep exactly what one-after-another keeps.
    let (w, h) = (1200usize, 900usize);
    let img = RgbImage::from_fn(w, h, |x, y| {
        let (fx, fy) = (x as f32 / w as f32, y as f32 / h as f32);
        let blob = (((fx * 9.0).sin() * (fy * 7.0).cos()) * 0.5 + 0.5) * 0.06;
        [0.3 + 0.4 * fx + blob, 0.5 + 0.2 * fy, 0.7 - 0.3 * fx + blob]
    });
    let s = ImageSampler { img: &img, origin: (0, 0) };
    let canvas = Rect::new(0, 0, w as i32, h as i32);
    let t = quick::Tuning::default();
    let strokes: [(&[(f32, f32)], f32); 4] = [
        (&[(300.0, 300.0)], 30.0),
        (&[(500.0, 400.0), (580.0, 400.0), (660.0, 420.0)], 60.0),
        (&[(200.0, 700.0), (260.0, 650.0), (320.0, 700.0), (380.0, 650.0)], 30.0),
        (&[(900.0, 200.0)], 120.0),
    ];
    let mut most = 0;
    for (pts, size) in strokes {
        let one = quick::quick_select_batched::<true>(&s, canvas, pts, size, &t, 1);
        let many = quick::quick_select_batched::<true>(&s, canvas, pts, size, &t, 4);
        assert_eq!(one.as_ref().map(|(r, n)| (r.bbox, &r.mask, n)), many.as_ref().map(|(r, n)| (r.bbox, &r.mask, n)), "{pts:?} size {size}");
        most = most.max(one.map_or(0, |(_, n)| n));
    }
    assert!(most > 1, "no stroke needed a larger window, so the test exercises nothing");
}

#[test]
fn quick_select_window_sizes_stop_at_the_canvas() {
    let canvas = Rect::new(0, 0, 1000, 800);
    let bbox = Rect::new(400, 300, 460, 360);
    assert_eq!(quick::margins_to_try(96, bbox, canvas, 4), vec![96, 192, 384, 768]);
    // 768 already covers the canvas: no larger (identical) window is cut.
    assert_eq!(quick::margins_to_try(384, bbox, canvas, 4), vec![384, 768]);
    assert_eq!(quick::margins_to_try(96, bbox, canvas, 0), vec![96]);
}

#[test]
fn quick_select_grows_without_leaking() {
    let img = two_regions(140, 100);
    let s = ImageSampler { img: &img, origin: (0, 0) };
    let canvas = Rect::new(0, 0, 140, 100);
    let reg = quick::quick_select(&s, canvas, &[(20.0, 50.0), (35.0, 50.0)], 8.0).unwrap();
    let mut inside = 0;
    for y in 0..100 {
        for x in 0..140 {
            let v = reg.at(x, y);
            if x >= 62 {
                assert!(v < 0.5, "leaked across the edge at ({x},{y}): {v}");
            }
            if x < 58 && v >= 0.5 {
                inside += 1;
            }
        }
    }
    // Grew over (nearly) the whole left region, far beyond the stroke.
    assert!(inside as f32 > 0.97 * (58.0 * 100.0), "only {inside} pixels selected");
}

#[test]
fn quick_select_stops_at_thin_line() {
    // Same colour on both sides of a dark 2-pixel line.
    let mut rng = Rng::new(9);
    let img = RgbImage::from_fn(120, 80, |x, _| {
        let n = 0.03 * rng.normal();
        if x == 60 || x == 61 { [0.05, 0.05, 0.05] } else { [0.6 + n, 0.6 + n, 0.55 + n] }
    });
    let s = ImageSampler { img: &img, origin: (0, 0) };
    let reg = quick::quick_select(&s, Rect::new(0, 0, 120, 80), &[(30.0, 40.0)], 6.0).unwrap();
    assert!(reg.at(10, 10) >= 0.5 && reg.at(50, 70) >= 0.5);
    for y in 0..80 {
        for x in 63..120 {
            assert!(reg.at(x, y) < 0.5, "crossed the line at ({x},{y})");
        }
    }
}

#[test]
fn quick_select_a_drag_selects_around_itself_not_a_big_object() {
    // Photoshop 25.4 on this very scene: an 80 px drag inside a 180 px radius disc selects about
    // the stroke (5285 px), not the disc (~100,000 px): the disc is far too big for the stroke.
    let (img, truth) = disc_scene(600, 500, 300.0, 250.0, 180.0, 4);
    let s = ImageSampler { img: &img, origin: (0, 0) };
    let reg = quick::quick_select(&s, Rect::new(0, 0, 600, 500), &[(260.0, 250.0), (340.0, 250.0)], 30.0).unwrap();
    let n = count(&reg);
    assert!((3000..9000).contains(&n), "{n} px");
    assert!(reg.at(300, 250) >= 0.5 && reg.at(262, 250) >= 0.5);
    assert!(iou(&reg, &truth, 600, 500) < 0.2);
}

// ---------- Quick selection: Photoshop's measured behaviour ----------
//
// Each case below was run in Photoshop 25.4 (the same image and stroke, the mouse held 1 s so
// its solver settles); the numbers in the comments are Photoshop's.

const QS_RED: [f32; 3] = [200.0 / 255.0, 60.0 / 255.0, 60.0 / 255.0];
const QS_BLUE: [f32; 3] = [60.0 / 255.0, 60.0 / 255.0, 200.0 / 255.0];

/// A `w` x `h` image of `bg` with `fg` over `[x0, x1) x [y0, y1)`.
fn qs_scene(w: usize, h: usize, bg: [f32; 3], fg: [f32; 3], rect: (usize, usize, usize, usize)) -> RgbImage {
    let (x0, y0, x1, y1) = rect;
    RgbImage::from_fn(w, h, |x, y| if x >= x0 && x < x1 && y >= y0 && y < y1 { fg } else { bg })
}

/// A centred `side` square of `fg` on `bg` in a `w` x `w` image.
fn qs_square(w: usize, side: usize, fg: [f32; 3], bg: [f32; 3]) -> RgbImage {
    let x0 = (w - side) / 2;
    qs_scene(w, w, bg, fg, (x0, x0, x0 + side, x0 + side))
}

fn qs(img: &RgbImage, pts: &[(f32, f32)], size: f32) -> Region {
    let s = ImageSampler { img, origin: (0, 0) };
    quick::quick_select(&s, Rect::new(0, 0, img.w as i32, img.h as i32), pts, size).expect("a selection")
}

fn count(r: &Region) -> usize {
    r.mask.iter().filter(|v| **v >= 128).count()
}

/// Selected pixels inside and outside `[x0, x1) x [y0, y1)`.
fn inside_outside(r: &Region, rect: (i32, i32, i32, i32)) -> (usize, usize) {
    let (mut i, mut o) = (0, 0);
    for y in r.bbox.y0..r.bbox.y1 {
        for x in r.bbox.x0..r.bbox.x1 {
            if r.at(x, y) >= 0.5 {
                if x >= rect.0 && x < rect.2 && y >= rect.1 && y < rect.3 { i += 1 } else { o += 1 }
            }
        }
    }
    (i, o)
}

#[test]
fn quick_select_works_at_full_or_exactly_half_resolution() {
    // Up to 512 x 512 pixels Photoshop works on the whole document, above at half resolution,
    // whatever the document's shape or the brush.
    assert_eq!(quick::working_step(512, 512), 1);
    assert_eq!(quick::working_step(513, 512), 2);
    assert_eq!(quick::working_step(520, 300), 1);
    assert_eq!(quick::working_step(900, 300), 2);
    assert_eq!(quick::working_step(5000, 3500), 2);
    // Half resolution: the mask is scaled up bilinearly, so a selected square's edge steps
    // 64, 191, 255 (Photoshop's own values on a 640 px document).
    let img = qs_square(640, 200, QS_RED, QS_BLUE);
    let r = qs(&img, &[(320.0, 320.0)], 30.0);
    let row: Vec<u8> = (216..223).map(|x| (r.at(x, 320) * 255.0).round() as u8).collect();
    assert_eq!(row, [0, 0, 0, 64, 191, 255, 255]);
}

#[test]
fn quick_select_click_in_a_flat_area_selects_about_the_brush() {
    // Photoshop: 1025-1120 px for a 30 px brush (the disc grows a little while the button is held).
    let img = qs_scene(400, 400, QS_RED, QS_RED, (0, 0, 0, 0));
    let n = count(&qs(&img, &[(200.0, 200.0)], 30.0));
    assert!((700..1600).contains(&n), "{n} px");
}

#[test]
fn quick_select_click_takes_an_object_only_while_it_is_small_enough() {
    // 30 px brush, red on blue: Photoshop selects a 220 px square whole, not a 240 px one.
    let img = qs_square(500, 220, QS_RED, QS_BLUE);
    let (i, o) = inside_outside(&qs(&img, &[(250.0, 250.0)], 30.0), (140, 140, 360, 360));
    assert!(i >= 220 * 220 - 10 && o == 0, "220: {i} in, {o} out");
    let img = qs_square(500, 240, QS_RED, QS_BLUE);
    let n = count(&qs(&img, &[(250.0, 250.0)], 30.0));
    assert!(n < 2000, "240: {n} px");
    // A weak edge (15 levels) holds only a small square: 94 px yes, 120 px no.
    let (lo, hi) = ([125.0 / 255.0, 110.0 / 255.0, 110.0 / 255.0], [110.0 / 255.0, 110.0 / 255.0, 125.0 / 255.0]);
    let img = qs_square(500, 94, lo, hi);
    let (i, o) = inside_outside(&qs(&img, &[(250.0, 250.0)], 30.0), (203, 203, 297, 297));
    assert!(i >= 94 * 94 - 10 && o == 0, "94: {i} in, {o} out");
    let img = qs_square(500, 120, lo, hi);
    let n = count(&qs(&img, &[(250.0, 250.0)], 30.0));
    assert!(n < 2000, "120: {n} px");
}

#[test]
fn quick_select_takes_a_narrow_bar_end_to_end_but_only_the_end_near_it() {
    // An 80 x 400 bar: a click in the middle selects all of it; a click 30 px from its end only
    // the end (Photoshop: 3762 px, cut just above the brush).
    let img = qs_scene(500, 500, QS_BLUE, QS_RED, (100, 50, 180, 450));
    let (i, o) = inside_outside(&qs(&img, &[(140.0, 250.0)], 30.0), (100, 50, 180, 450));
    assert!(i >= 80 * 400 - 10 && o == 0, "middle: {i} in, {o} out");
    let r = qs(&img, &[(140.0, 420.0)], 30.0);
    let (i, o) = inside_outside(&r, (100, 380, 180, 450));
    assert!(i > 2500 && o == 0, "near the end: {i} in the end, {o} elsewhere");
}

#[test]
fn quick_select_a_short_drag_takes_what_a_click_does_not() {
    // A 300 px square: a click selects about the brush; a 60 px drag the whole square.
    let img = qs_square(500, 300, QS_RED, QS_BLUE);
    assert!(count(&qs(&img, &[(250.0, 250.0)], 30.0)) < 2000);
    let (i, o) = inside_outside(&qs(&img, &[(220.0, 250.0), (280.0, 250.0)], 30.0), (100, 100, 400, 400));
    assert!(i >= 300 * 300 - 10 && o == 0, "{i} in, {o} out");
}

#[test]
fn quick_select_on_a_large_document_grows_its_window_to_the_object() {
    // A 1000 x 600 document (half resolution) with an 80 x 960 bar: Photoshop selects all of it
    // from a click in the middle (76800 px). The working window starts around the stroke and grows
    // until the selection no longer runs into it.
    let img = qs_scene(1000, 600, QS_BLUE, QS_RED, (20, 260, 980, 340));
    // The mouse on pixel (500, 300), as in Photoshop: its centre.
    let (i, o) = inside_outside(&qs(&img, &[(500.5, 300.5)], 30.0), (20, 260, 980, 340));
    assert!(i >= 960 * 80 * 98 / 100 && o < 2000, "{i} in, {o} out");
}

// ---------- Select subject ----------

#[test]
fn select_subject_finds_centred_object() {
    let (img, truth) = disc_scene(200, 150, 100.0, 75.0, 40.0, 6);
    let s = ImageSampler { img: &img, origin: (0, 0) };
    let reg = subject::select_subject(&s, Rect::new(0, 0, 200, 150)).unwrap();
    let v = iou(&reg, &truth, 200, 150);
    assert!(v > 0.9, "IoU {v}");
}

/// Counts full-resolution reads; the working copy goes through `rgb_scaled` uncounted, so each
/// read is one band tile of the full-resolution re-cut.
struct TileReads<'a>(ImageSampler<'a>, std::sync::atomic::AtomicUsize);

impl Sampler for TileReads<'_> {
    fn rgba(&self, r: Rect) -> Vec<[f32; 4]> {
        self.1.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.0.rgba(r)
    }
    fn rgb_scaled(&self, r: Rect, step: usize) -> RgbImage {
        self.0.rgb_scaled(r, step)
    }
}

#[test]
fn finish_region_cancel_stops_inside_the_band_cut() {
    use photocraft_raster::{Cancelled, Interrupt};
    use std::sync::atomic::{AtomicUsize, Ordering::SeqCst};
    // One 180-pixel window is one band tile, and nothing asks after the band re-cut, so a cancel
    // seen after the tile's own check can only have been seen inside its min cut.
    let (n, step) = (180, 3);
    let (img, truth) = disc_scene(n, n, 90.0, 90.0, 55.0, 4);
    let s = ImageSampler { img: &img, origin: (0, 0) };
    let ln = n / step;
    let low: Vec<bool> = (0..ln * ln).map(|i| truth[((i / ln) * step + 1) * n + (i % ln) * step + 1]).collect();
    let (fs, bs): (Vec<_>, Vec<_>) = img.px.iter().zip(&truth).partition(|(_, t)| **t);
    let fit = |v: Vec<(&[f32; 3], &bool)>| Gmm::fit(&v.iter().map(|(p, _)| **p).collect::<Vec<_>>(), 5, gmm::DEFAULT_REG).unwrap();
    let (fg, bg) = (fit(fs), fit(bs));
    let window = Rect::new(0, 0, n as i32, n as i32);
    let polls = AtomicUsize::new(0);
    let finish = |cancel_from: usize| {
        polls.store(0, SeqCst);
        let cancel = || polls.fetch_add(1, SeqCst) + 1 >= cancel_from;
        finish_region_with(&s, window, step, &low, ln, ln, Some((&fg, &bg)), &Interrupt::cancel_only(&cancel))
    };
    let reg = finish(usize::MAX).unwrap().unwrap();
    assert!(polls.load(SeqCst) >= 2, "the cut polls too ({} polls)", polls.load(SeqCst));
    assert!(iou(&reg, &truth, n, n) > 0.95);
    assert_eq!(finish_region(&s, window, step, &low, ln, ln, Some((&fg, &bg))), Some(reg));
    // Cancelled at the cut's first poll (the second in all): no region.
    assert_eq!(finish(2), Err(Cancelled));
    assert_eq!(polls.load(SeqCst), 2);
    assert_eq!(finish(1), Err(Cancelled), "cancelled before the tile");
}

#[test]
fn select_subject_cancelled_in_the_band_recut_reads_no_more_tiles() {
    use photocraft_raster::{Cancelled, Interrupt};
    use std::sync::atomic::{AtomicUsize, Ordering::SeqCst};
    let (w, h) = (3000, 2000);
    let (img, truth) = disc_scene(w, h, 1500.0, 1000.0, 700.0, 6);
    let canvas = Rect::new(0, 0, w as i32, h as i32);
    let sampler = || TileReads(ImageSampler { img: &img, origin: (0, 0) }, AtomicUsize::new(0));

    // Not cancelled: the same region as `select_subject`, and the band cuts themselves ask
    // (more polls once tiles are read than the one check before each tile).
    let s = sampler();
    let band_polls = AtomicUsize::new(0);
    let count = || {
        if s.1.load(SeqCst) > 0 {
            band_polls.fetch_add(1, SeqCst);
        }
        false
    };
    let reg = subject::select_subject_with(&s, canvas, &Interrupt::cancel_only(&count)).unwrap().unwrap();
    let tiles = s.1.load(SeqCst);
    assert!(tiles > 4, "{tiles} band tiles");
    assert!(band_polls.load(SeqCst) > tiles, "{} polls over {tiles} tiles", band_polls.load(SeqCst));
    assert!(iou(&reg, &truth, w, h) > 0.9);
    assert_eq!(subject::select_subject(&sampler(), canvas), Some(reg));

    // Cancelled once the first tile is read: the run stops there instead of re-cutting every
    // tile. One thread, so no other tile is already in flight.
    let s = sampler();
    let cancel = || s.1.load(SeqCst) > 0;
    let pool = rayon::ThreadPoolBuilder::new().num_threads(1).build().unwrap();
    let r = pool.install(|| subject::select_subject_with(&s, canvas, &Interrupt::cancel_only(&cancel)));
    assert_eq!(r, Err(Cancelled));
    assert_eq!(s.1.load(SeqCst), 1, "tiles read after the cancel");
}

#[test]
fn saliency_highlights_the_object() {
    let (img, truth) = disc_scene(120, 90, 60.0, 45.0, 22.0, 8);
    let sal = subject::saliency(&img);
    let (mut fi, mut fn_, mut bi, mut bn) = (0.0, 0.0, 0.0, 0.0);
    for (s, t) in sal.iter().zip(&truth) {
        if *t {
            fi += s;
            fn_ += 1.0;
        } else {
            bi += s;
            bn += 1.0;
        }
    }
    assert!(fi / fn_ > 0.6 && bi / bn < 0.25, "fg {} bg {}", fi / fn_, bi / bn);
}

// ---------- Focus area ----------

#[test]
fn focus_area_selects_sharp_half() {
    let mut rng = Rng::new(12);
    let (w, h) = (160usize, 100usize);
    let noise: Vec<f32> = (0..w * h).map(|_| rng.f32()).collect();
    // Right half: heavily blurred copy of the texture.
    let blurred = crate::matting::box_mean(&noise, w, h, 4);
    let img = RgbImage::from_fn(w, h, |x, y| {
        let v = if x < 80 { noise[y * w + x] } else { blurred[y * w + x] };
        [v, v * 0.8, 0.3]
    });
    let s = ImageSampler { img: &img, origin: (0, 0) };
    let reg = focus::focus_area(&s, Rect::new(0, 0, w as i32, h as i32), 0.5, 0.0).unwrap();
    let (mut l, mut r) = (0usize, 0usize);
    for y in 0..h as i32 {
        for x in 0..w as i32 {
            if reg.at(x, y) >= 0.5 {
                if x < 80 { l += 1 } else { r += 1 }
            }
        }
    }
    assert!(l as f32 > 0.85 * 80.0 * h as f32, "sharp half {l}");
    assert!((r as f32) < 0.1 * 80.0 * h as f32, "blurred half {r}");
}

// ---------- helpers ----------

#[test]
fn sampler_downsampling_matches_image_downsample() {
    let img = RgbImage::from_fn(37, 29, |x, y| [x as f32 / 37.0, y as f32 / 29.0, ((x * y) % 7) as f32 / 7.0]);
    let s = ImageSampler { img: &img, origin: (5, 3) };
    let a = s.rgb_scaled(Rect::new(5, 3, 42, 32), 4);
    let b = img.downsample(4);
    assert_eq!((a.w, a.h), (b.w, b.h));
    for (p, q) in a.px.iter().zip(&b.px) {
        for c in 0..3 {
            assert!((p[c] - q[c]).abs() < 1e-5);
        }
    }
}

#[test]
fn clean_mask_drops_islands_and_fills_holes() {
    let (w, h) = (40, 40);
    let mut m = vec![false; w * h];
    for y in 5..30 {
        for x in 5..30 {
            m[y * w + x] = true;
        }
    }
    m[15 * w + 15] = false; // hole
    m[36 * w + 36] = true; // island
    let c = clean_mask(&m, w, h, 0.1, 0.05);
    assert!(c[15 * w + 15]);
    assert!(!c[36 * w + 36]);
}

/// Writes the quick-selection test scenes as raw RGB8 (for measuring them in Photoshop).
#[test]
#[ignore]
fn dump_quick_scenes() {
    let Ok(dir) = std::env::var("QS_DUMP") else { return };
    let save = |name: &str, img: &RgbImage| {
        let bytes: Vec<u8> = img.px.iter().flat_map(|p| p.map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8)).collect();
        std::fs::write(format!("{dir}/{name}_{}x{}.rgb", img.w, img.h), bytes).unwrap();
    };
    save("two_regions", &two_regions(140, 100));
    let mut rng = Rng::new(9);
    let line = RgbImage::from_fn(120, 80, |x, _| {
        let n = 0.03 * rng.normal();
        if x == 60 || x == 61 { [0.05, 0.05, 0.05] } else { [0.6 + n, 0.6 + n, 0.55 + n] }
    });
    save("thin_line", &line);
    save("disc_scene", &disc_scene(600, 500, 300.0, 250.0, 180.0, 4).0);
}

#[test]
fn object_at_finds_the_disc_under_the_cursor() {
    let (img, truth) = disc_scene(320, 240, 160.0, 120.0, 70.0, 7);
    let s = ImageSampler { img: &img, origin: (0, 0) };
    let canvas = Rect::new(0, 0, 320, 240);
    let reg = grabcut::object_at(&s, canvas, 165.0, 118.0, 20_000).unwrap();
    let v = iou(&reg, &truth, 320, 240);
    assert!(v > 0.9, "IoU {v}");
}

#[test]
fn object_at_keeps_large_foregrounds_but_not_the_surrounding_background() {
    let (w, h) = (400, 300);
    for margin in [30, 20, 8] {
        let inside = |x: usize, y: usize| x >= margin && x < w - margin && y >= margin && y < h - margin;
        let img = RgbImage::from_fn(w, h, |x, y| {
            let noise = ((x * 13 + y * 7) % 17) as f32 / 850.0;
            if inside(x, y) { [0.8 + noise, 0.2, 0.1] } else { [0.1, 0.2, 0.8 + noise] }
        });
        let truth: Vec<_> = (0..w * h).map(|i| inside(i % w, i / w)).collect();
        let sampler = ImageSampler { img: &img, origin: (-40, 25) };
        let canvas = Rect::new(-40, 25, w as i32 - 40, h as i32 + 25);
        let region = grabcut::object_at(&sampler, canvas, 160.0, 175.0, grabcut::OBJECT_AT_WORK_PX)
            .unwrap_or_else(|| panic!("missing large foreground with margin {margin}"));
        let local = Region { bbox: region.bbox.translate(40, -25), mask: region.mask };
        assert!(iou(&local, &truth, w, h) > 0.95, "margin {margin}");
        assert!(grabcut::object_at(&sampler, canvas, -39.0, 26.0, grabcut::OBJECT_AT_WORK_PX).is_none(), "background margin {margin}");
    }
}

#[test]
fn object_at_allows_a_foreground_touching_one_edge_but_rejects_a_flat_image() {
    let img = RgbImage::from_fn(400, 300, |x, y| if x < 360 && (20..280).contains(&y) { [0.8, 0.2, 0.1] } else { [0.1, 0.2, 0.8] });
    let sampler = ImageSampler { img: &img, origin: (0, 0) };
    let canvas = Rect::new(0, 0, 400, 300);
    let region = grabcut::object_at(&sampler, canvas, 200.0, 150.0, grabcut::OBJECT_AT_WORK_PX).unwrap();
    assert!(region.at(0, 150) > 0.9 && region.at(350, 150) > 0.9);
    assert!(region.at(380, 150) < 0.1 && region.at(200, 5) < 0.1);
    let flat = RgbImage::from_fn(400, 300, |_, _| [0.5; 3]);
    assert!(grabcut::object_at(&ImageSampler { img: &flat, origin: (0, 0) }, canvas, 200.0, 150.0, grabcut::OBJECT_AT_WORK_PX).is_none());
}

#[test]
fn object_at_on_the_background_or_off_canvas_finds_nothing() {
    let (img, _) = disc_scene(320, 240, 160.0, 120.0, 70.0, 7);
    let s = ImageSampler { img: &img, origin: (0, 0) };
    let canvas = Rect::new(0, 0, 320, 240);
    assert!(grabcut::object_at(&s, canvas, 10.0, 10.0, 20_000).is_none());
    assert!(grabcut::object_at(&s, canvas, -5.0, 10.0, 20_000).is_none());
    assert!(grabcut::object_at(&s, canvas, f32::NAN, 10.0, 20_000).is_none());
    assert!(grabcut::object_at(&s, canvas, 400.0, 10.0, 0).is_none());
}

/// #6576: a 24 px object in a 4000×3000 document is found (a 1000×800 one still is).
#[test]
fn object_at_finds_small_object_in_large_canvas() {
    let (w, h) = (4000usize, 3000usize);
    let canvas = Rect::new(0, 0, w as i32, h as i32);
    for (ox, oy, ow, oh) in [(188usize, 138usize, 24usize, 24usize), (1500, 1100, 1000, 800)] {
        let inside = |x: usize, y: usize| x >= ox && x < ox + ow && y >= oy && y < oy + oh;
        let img = RgbImage::from_fn(w, h, |x, y| if inside(x, y) { [0.8, 0.1, 0.1] } else { [0.9, 0.9, 0.9] });
        let truth: Vec<bool> = (0..w * h).map(|i| inside(i % w, i / w)).collect();
        let s = ImageSampler { img: &img, origin: (0, 0) };
        let (cx, cy) = ((ox + ow / 2) as f32, (oy + oh / 2) as f32);
        let reg = grabcut::object_at(&s, canvas, cx, cy, grabcut::OBJECT_AT_WORK_PX);
        let reg = reg.unwrap_or_else(|| panic!("the {ow}×{oh} object is not found"));
        let v = iou(&reg, &truth, w, h);
        assert!(v > 0.8, "{ow}×{oh}: IoU {v}");
    }
    // The flat background around the small object is still nothing.
    let img = RgbImage::from_fn(w, h, |x, y| if (188..212).contains(&x) && (138..162).contains(&y) { [0.8, 0.1, 0.1] } else { [0.9, 0.9, 0.9] });
    let s = ImageSampler { img: &img, origin: (0, 0) };
    assert!(grabcut::object_at(&s, canvas, 400.0, 300.0, grabcut::OBJECT_AT_WORK_PX).is_none());
}

// ---------- Select Subject on the cases reported in #2788 ----------

/// A figure whose upper part is close to the background colour (a cream shirt on a near-white
/// backdrop): the whole figure is the subject. Returns (image, truth, the low-contrast part).
pub(crate) fn low_contrast_figure(seed: u64) -> (RgbImage, Vec<bool>, Vec<bool>) {
    let (w, h) = (240usize, 200usize);
    let mut rng = Rng::new(seed);
    let mut img = RgbImage::new(w, h);
    let mut truth = vec![false; w * h];
    let mut cream = vec![false; w * h];
    for y in 0..h {
        for x in 0..w {
            let body = (90..150).contains(&x) && (70..190).contains(&y);
            let shoulders = (80..160).contains(&x) && (40..70).contains(&y);
            let print = (105..135).contains(&x) && (100..130).contains(&y);
            let n = 0.012 * rng.normal();
            let i = y * w + x;
            truth[i] = body || shoulders;
            cream[i] = shoulders;
            img.px[i] = if print {
                [0.9 + n, 0.15 + n, 0.1 + n]
            } else if body {
                [0.12 + n, 0.2 + n, 0.45 + n]
            } else if shoulders {
                // Cream, a few levels off the backdrop.
                [0.90 + n, 0.88 + n, 0.80 + n]
            } else {
                [0.96 + n, 0.96 + n, 0.95 + n]
            }
            .map(|v| v.clamp(0.0, 1.0));
        }
    }
    (img, truth, cream)
}

fn coverage(region: &Region, part: &[bool], w: usize, h: usize) -> f32 {
    let (mut hit, mut n) = (0usize, 0usize);
    for y in 0..h {
        for x in 0..w {
            if part[y * w + x] {
                n += 1;
                hit += (region.at(x as i32, y as i32) >= 0.5) as usize;
            }
        }
    }
    hit as f32 / n.max(1) as f32
}

#[test]
fn select_subject_keeps_parts_close_to_the_background_colour() {
    let (img, truth, cream) = low_contrast_figure(7);
    let s = ImageSampler { img: &img, origin: (0, 0) };
    let reg = subject::select_subject(&s, Rect::new(0, 0, 240, 200)).unwrap();
    let v = iou(&reg, &truth, 240, 200);
    let c = coverage(&reg, &cream, 240, 200);
    assert!(c > 0.85, "the cream shoulders are part of the subject: covered {c}");
    assert!(v > 0.9, "IoU {v}");
}

#[test]
fn select_subject_leaves_a_background_coloured_gap_open() {
    // A dark ring (arms on hips) around a patch of plain backdrop: the patch is not the subject.
    let (w, h) = (220usize, 200usize);
    let mut rng = Rng::new(11);
    let mut img = RgbImage::new(w, h);
    let mut truth = vec![false; w * h];
    let mut gap = vec![false; w * h];
    for y in 0..h {
        for x in 0..w {
            let d = (x as f32 - 110.0).hypot(y as f32 - 100.0);
            let ring = (17.0..80.0).contains(&d);
            let hole = d < 17.0;
            let n = 0.012 * rng.normal();
            let i = y * w + x;
            truth[i] = ring;
            gap[i] = hole;
            img.px[i] = if ring { [0.2 + n, 0.15 + n, 0.35 + n] } else { [0.95 + n, 0.94 + n, 0.92 + n] }.map(|v| v.clamp(0.0, 1.0));
        }
    }
    let s = ImageSampler { img: &img, origin: (0, 0) };
    let reg = subject::select_subject(&s, Rect::new(0, 0, w as i32, h as i32)).unwrap();
    let g = coverage(&reg, &gap, w, h);
    assert!(g < 0.1, "the backdrop inside the ring stays unselected: covered {g}");
    let v = iou(&reg, &truth, w, h);
    assert!(v > 0.9, "IoU {v}");
}

#[test]
fn antialias_blends_edge_pixels_with_their_neighbourhood() {
    // A lone selected pixel keeps most of its coverage and its neighbours get half of a ninth;
    // neighbours outside the mask count as the pixel itself.
    let mut m = vec![0, 0, 0, 0, 255, 0, 0, 0, 0];
    antialias_u8(&mut m, 3, 3);
    assert_eq!(m, [14, 14, 14, 14, 142, 14, 14, 14, 14]);
}

#[test]
fn antialias_skips_only_spans_without_edge_pixels() {
    // Every pixel through the per-pixel rule, no span skipped.
    let every_pixel = |mask: &mut [u8], w: usize, h: usize| {
        let src = mask.to_vec();
        for y in 0..h {
            for x in 0..w {
                if let Some(v) = edge_value(&src, w, h, x, y) {
                    mask[y * w + x] = v;
                }
            }
        }
    };
    let mut rng = Rng::new(11);
    // Widths around the 64-pixel span, single rows and columns, discs, noise and stripes.
    for (w, h) in [(1usize, 1usize), (1, 9), (9, 1), (63, 4), (64, 3), (65, 5), (127, 2), (129, 70), (300, 200)] {
        let (cx, cy, r) = (w as f32 * 0.6, h as f32 * 0.4, w.min(h) as f32 / 3.0);
        let masks: Vec<Vec<u8>> = vec![
            vec![255; w * h],
            vec![0; w * h],
            (0..w * h).map(|i| if ((i % w) as f32 - cx).hypot((i / w) as f32 - cy) < r { 255 } else { 0 }).collect(),
            (0..w * h).map(|_| if rng.f32() < 0.02 { 255 } else { 0 }).collect(),
            (0..w * h).map(|_| if rng.f32() < 0.5 { 255 } else { 0 }).collect(),
            (0..w * h).map(|i| if (i % w) / 64 % 2 == 0 { 255 } else { 0 }).collect(),
            (0..w * h).map(|i| if i % w == w - 1 || i / w == h / 2 { 255 } else { 0 }).collect(),
        ];
        for (k, m) in masks.into_iter().enumerate() {
            let (mut fast, mut slow) = (m.clone(), m);
            antialias_u8(&mut fast, w, h);
            every_pixel(&mut slow, w, h);
            assert!(fast == slow, "{w}×{h} mask {k}");
        }
    }
}

/// Timing on a 6 MP figure, for before/after numbers (`cargo test --release -- --ignored select_subject_timing --nocapture`).
#[test]
#[ignore = "timing only"]
fn select_subject_timing() {
    let (w, h) = (3000usize, 2000usize);
    let img = RgbImage::from_fn(w, h, |x, y| {
        let body = (1100..1900).contains(&x) && (700..1900).contains(&y);
        let shoulders = (1000..2000).contains(&x) && (400..700).contains(&y);
        let t = ((x * 7 + y * 13) % 17) as f32 / 1700.0;
        if body {
            [0.12 + t, 0.2, 0.45]
        } else if shoulders {
            [0.90, 0.88 + t, 0.80]
        } else {
            [0.96, 0.96 + t, 0.95]
        }
    });
    let s = ImageSampler { img: &img, origin: (0, 0) };
    for _ in 0..2 {
        let t0 = std::time::Instant::now();
        let reg = subject::select_subject(&s, Rect::new(0, 0, w as i32, h as i32)).unwrap();
        eprintln!("select_subject 6 MP: {:?} bbox {:?}", t0.elapsed(), reg.bbox);
    }
}

/// Writes the #2788 scenes and the Select Subject mask over them as PPM files into
/// `$SUBJECT_DUMP` (for the PR's before/after pictures).
#[test]
#[ignore = "writes pictures"]
fn select_subject_dump_pictures() {
    let Ok(dir) = std::env::var("SUBJECT_DUMP") else { return };
    let write = |name: &str, img: &RgbImage, reg: Option<&Region>| {
        let mut out = format!("P6\n{} {}\n255\n", img.w, img.h).into_bytes();
        for y in 0..img.h {
            for x in 0..img.w {
                let mut c = img.at(x, y);
                if let Some(r) = reg
                    && r.at(x as i32, y as i32) >= 0.5
                {
                    c = [c[0] * 0.5 + 0.5, c[1] * 0.5, c[2] * 0.5 + 0.5];
                }
                out.extend(c.map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8));
            }
        }
        std::fs::write(format!("{dir}/{name}.ppm"), out).unwrap();
    };
    let (img, _, _) = low_contrast_figure(7);
    let s = ImageSampler { img: &img, origin: (0, 0) };
    let reg = subject::select_subject(&s, Rect::new(0, 0, 240, 200));
    write("figure", &img, None);
    write("figure-selected", &img, reg.as_ref());
    let (w, h) = (220usize, 200usize);
    let mut rng = Rng::new(11);
    let img = RgbImage::from_fn(w, h, |x, y| {
        let d = (x as f32 - 110.0).hypot(y as f32 - 100.0);
        let n = 0.012 * rng.normal();
        if (17.0..80.0).contains(&d) { [0.2 + n, 0.15 + n, 0.35 + n] } else { [0.95 + n, 0.94 + n, 0.92 + n] }.map(|v| v.clamp(0.0, 1.0))
    });
    let s = ImageSampler { img: &img, origin: (0, 0) };
    let reg = subject::select_subject(&s, Rect::new(0, 0, w as i32, h as i32));
    write("ring", &img, None);
    write("ring-selected", &img, reg.as_ref());
}

/// A dark square on a light background with a square hole that shows the background colour.
fn square_with_hole(hole: usize) -> RgbImage {
    let (w, h) = (400usize, 300usize);
    let mut rng = Rng::new(11);
    let (x0, y0) = (100usize, 50usize);
    RgbImage::from_fn(w, h, |x, y| {
        let n = 0.02 * rng.normal();
        let in_square = x >= x0 && x < x0 + 200 && y >= y0 && y < y0 + 200;
        let in_hole = x >= x0 + 96 && x < x0 + 96 + hole && y >= y0 + 96 && y < y0 + 96 + hole;
        let c = if in_square && !in_hole { [0.1, 0.12, 0.15] } else { [0.9, 0.9, 0.85] };
        c.map(|v: f32| (v + n).clamp(0.0, 1.0))
    })
}

#[test]
fn object_select_keeps_small_background_hole_open() {
    let img = square_with_hole(8);
    let s = ImageSampler { img: &img, origin: (0, 0) };
    let canvas = Rect::new(0, 0, 400, 300);
    let reg = grabcut::object_select(&s, canvas, Rect::new(80, 30, 320, 270), 1_000_000).unwrap();
    assert!(reg.at(150, 100) >= 0.5, "the square itself is selected");
    assert!(reg.at(200, 150) < 0.5, "background seen through the object stays unselected");
}

#[test]
fn object_at_keeps_small_background_hole_open() {
    let img = square_with_hole(16);
    let s = ImageSampler { img: &img, origin: (0, 0) };
    let canvas = Rect::new(0, 0, 400, 300);
    let reg = grabcut::object_at(&s, canvas, 130.0, 80.0, 1_000_000).unwrap();
    assert!(reg.at(150, 100) >= 0.5, "the square itself is selected");
    assert!(reg.at(203, 153) < 0.5, "background seen through the object stays unselected");
}

#[test]
fn hole_fill_without_specks_fills_object_coloured_holes_only() {
    // The hole check Object Selection uses: no hole fills just for being small, a speck of the object's colour still does.
    let (w, h) = (40usize, 40usize);
    let mut rng = Rng::new(5);
    let in_square = |x: usize, y: usize| (5..35).contains(&x) && (5..35).contains(&y);
    let bg_hole = |x: usize, y: usize| (10..12).contains(&x) && (10..12).contains(&y);
    let fg_hole = |x: usize, y: usize| (25..27).contains(&x) && (25..27).contains(&y);
    let img = RgbImage::from_fn(w, h, |x, y| {
        let n = 0.02 * rng.normal();
        let c = if in_square(x, y) && !bg_hole(x, y) { [0.1, 0.12, 0.15] } else { [0.9, 0.9, 0.85] };
        c.map(|v: f32| (v + n).clamp(0.0, 1.0))
    });
    let mask: Vec<bool> = (0..w * h).map(|i| in_square(i % w, i / w) && !bg_hole(i % w, i / w) && !fg_hole(i % w, i / w)).collect();
    // The colour models, as GrabCut would fit them: the object colour vs. the backdrop colour.
    let dark: Vec<bool> = (0..w * h).map(|i| in_square(i % w, i / w) && !bg_hole(i % w, i / w)).collect();
    let (fs, bs): (Vec<_>, Vec<_>) = img.px.iter().zip(&dark).partition(|(_, m)| **m);
    let fit = |v: Vec<(&[f32; 3], &bool)>| Gmm::fit(&v.iter().map(|(p, _)| **p).collect::<Vec<_>>(), 3, gmm::DEFAULT_REG).unwrap();
    let (fg, bg) = (fit(fs), fit(bs));
    let out = fill_holes_by_model(&mask, &img, &fg, &bg, 0.0, 0.05);
    assert!(out[25 * w + 25], "a speck of the object's colour fills");
    assert!(!out[10 * w + 10], "a speck of the background's colour stays open");
}

/// Energy of a labelling: data costs plus the contrast weights of separated 8-neighbours.
fn grid_energy(img: &RgbImage, cf: &[f32], cb: &[f32], cut: &[bool], weight: impl Fn(f32) -> f32) -> f64 {
    let (w, h) = (img.w as i32, img.h as i32);
    let mut e = 0.0f64;
    for y in 0..h {
        for x in 0..w {
            let i = (y * w + x) as usize;
            e += f64::from(if cut[i] { cf[i] } else { cb[i] });
            for (dx, dy, len) in NB {
                let (nx, ny) = (x + dx, y + dy);
                if nx >= 0 && nx < w && ny < h {
                    let j = (ny * w + nx) as usize;
                    if cut[i] != cut[j] {
                        e += f64::from(weight(d2(img.px[i], img.px[j])) / len);
                    }
                }
            }
        }
    }
    e
}

#[test]
fn bk_and_default_grid_cut_tie_reaches_the_same_energy() {
    // Hard background down the left edge, hard foreground down the right, uniform pixels and equal
    // data costs: a vertical cut at any column separates the borders at the same energy, so only
    // the solvers' tie-breaking decides the labelling.
    let (w, h) = (120usize, 100usize);
    let img = RgbImage::from_fn(w, h, |_, _| [0.5, 0.5, 0.5]);
    let (cf, cb) = (vec![1.0f32; w * h], vec![1.0f32; w * h]);
    let fixed: Vec<u8> = (0..w * h)
        .map(|i| match i % w {
            0 => HARD_BG,
            x if x == w - 1 => HARD_FG,
            _ => FREE,
        })
        .collect();
    let weight = |_: f32| 2.0;
    // Two distinct candidate labellings (vertical cuts at different columns) tie at the minimum.
    let split = |col: usize| -> Vec<bool> { (0..w * h).map(|i| i % w >= col).collect() };
    let (e30, e90) = (grid_energy(&img, &cf, &cb, &split(30), weight), grid_energy(&img, &cf, &cb, &split(90), weight));
    assert!((e30 - e90).abs() <= 1e-6 * e30.abs().max(1.0), "candidates must tie: {e30} vs {e90}");
    assert_ne!(split(30), split(90));
    let a = grid_cut_cancellable(&img, &cf, &cb, &fixed, weight, &|| false).expect("default cut");
    let b = grid_cut_bk_cancellable(&img, &cf, &cb, &fixed, weight, &|| false).expect("bk cut");
    let (ea, eb) = (grid_energy(&img, &cf, &cb, &a, weight), grid_energy(&img, &cf, &cb, &b, weight));
    assert!((ea - e30).abs() <= 1e-3 * e30.abs().max(1.0), "default {ea} is not the tied minimum {e30}");
    assert!((eb - e30).abs() <= 1e-3 * e30.abs().max(1.0), "bk {eb} is not the tied minimum {e30}");
    // The solvers may pick different minimum cuts here; only the energy is guaranteed.
    assert!(a.iter().any(|&l| l) && b.iter().any(|&l| l), "both label the foreground border");
}

#[test]
fn bk_and_default_grid_cut_reach_the_same_minimum_energy() {
    // Large enough (>= 4096 free nodes, both terminals) that the default solver is push/relabel.
    let (w, h) = (120usize, 100usize);
    let mut rng = Rng::new(7);
    let img = RgbImage::from_fn(w, h, |x, y| {
        let inside = (x as f32 - 60.0).powi(2) + (y as f32 - 50.0).powi(2) < 1600.0;
        let base = if inside { [0.8, 0.3, 0.2] } else { [0.2, 0.4, 0.7] };
        base.map(|v| v + 0.08 * rng.normal())
    });
    let cf: Vec<f32> = img.px.iter().map(|p| 4.0 * d2(*p, [0.8, 0.3, 0.2])).collect();
    let cb: Vec<f32> = img.px.iter().map(|p| 4.0 * d2(*p, [0.2, 0.4, 0.7])).collect();
    let fixed: Vec<u8> = (0..w * h)
        .map(|i| {
            if i % w == 0 {
                HARD_BG
            } else if i == (50 * w + 60) {
                HARD_FG
            } else {
                FREE
            }
        })
        .collect();
    let weight = |d: f32| 50.0 * (-8.0 * d).exp();
    let a = grid_cut_cancellable(&img, &cf, &cb, &fixed, weight, &|| false).expect("default cut");
    let b = grid_cut_bk_cancellable(&img, &cf, &cb, &fixed, weight, &|| false).expect("bk cut");
    let (ea, eb) = (grid_energy(&img, &cf, &cb, &a, weight), grid_energy(&img, &cf, &cb, &b, weight));
    assert!((ea - eb).abs() <= 1e-3 * ea.abs().max(1.0), "default {ea} vs bk {eb}");
    assert_eq!(a, b, "same labelling, not just the same energy");
    assert!(grid_cut_bk_cancellable(&img, &cf, &cb, &fixed, weight, &|| true).is_none(), "BK honours cancel");
}
