use super::*;

/// The mean is matched first: on a nearly flat bright patch with a noisy deviation ratio the
/// gain stays where the bias can still reach the target's mean.
#[test]
fn fit_matches_the_mean_before_the_contrast() {
    let a = Adapt { gain: (0.9, 1.1), bias: 0.04 };
    // Target mean 0.70, source mean 0.70; deviations 0.01 vs 0.004 (ratio 2.5).
    let n = 25.0;
    let (mt, ms, sdt, sds) = (0.70f32, 0.70f32, 0.01f32, 0.004f32);
    let (g, b) = fit(a, n, mt * n, (sdt * sdt + mt * mt) * n, ms * n, (sds * sds + ms * ms) * n);
    assert!((g * ms + b - mt).abs() < 1e-5, "mean {} vs {mt} (g {g}, b {b})", g * ms + b);
    // Where the mean can't be reached anyway, the limits hold as before.
    let (g, b) = fit(a, n, 0.2 * n, 0.04 * n, 0.9 * n, 0.81 * n);
    assert!((a.gain.0..=a.gain.1).contains(&g) && b.abs() <= a.bias + 1e-6);
}

/// A zero-width image has no pixels however tall it is claimed to be: no per-row table.
#[test]
fn zero_width_images_complete_without_sizing_by_height() {
    let p = CompleteParams::default();
    assert_eq!(complete(0, 1 << 40, 3, &[], &[], &p), Some(Vec::new()));
    assert_eq!(complete(1 << 40, 0, 3, &[], &[], &p), Some(Vec::new()));
}

/// Smooth gradient plus deterministic fine noise.
fn texture(w: usize, h: usize) -> Vec<f32> {
    let mut rng = Rng::new(7);
    (0..w * h)
        .flat_map(|i| {
            let (x, y) = ((i % w) as f32, (i / w) as f32);
            let n = (rng.next() % 1000) as f32 / 1000.0 * 0.04 - 0.02;
            [0.3 + 0.004 * x + n, 0.5 + 0.002 * y + n, 0.4 + n]
        })
        .collect()
}

fn disc(w: usize, h: usize, cx: f32, cy: f32, r: f32) -> Vec<bool> {
    (0..w * h).map(|i| ((i % w) as f32 - cx).hypot((i / w) as f32 - cy) < r).collect()
}

fn hole_rmse(a: &[f32], b: &[f32], hole: &[bool], ch: usize) -> f32 {
    let (mut e, mut n) = (0.0f32, 0);
    for (i, hh) in hole.iter().enumerate() {
        if *hh {
            for c in 0..ch {
                e += (a[i * ch + c] - b[i * ch + c]).powi(2);
                n += 1;
            }
        }
    }
    (e / n as f32).sqrt()
}

#[test]
fn completion_fills_blemish_from_texture() {
    let (w, h, ch) = (96, 96, 3);
    let clean = texture(w, h);
    let hole = disc(w, h, 48.0, 48.0, 9.0);
    let mut dirty = clean.clone();
    for (i, hh) in hole.iter().enumerate() {
        if *hh {
            dirty[i * ch..i * ch + ch].copy_from_slice(&[0.02, 0.02, 0.02]);
        }
    }
    let out = complete(w, h, ch, &dirty, &hole, &CompleteParams::default()).unwrap();
    let before = hole_rmse(&dirty, &clean, &hole, ch);
    let after = hole_rmse(&out, &clean, &hole, ch);
    assert!(after < 0.06 && after < before * 0.15, "rmse before {before} after {after}");
    // Outside the hole untouched.
    assert!(out.iter().zip(&dirty).zip(hole.iter().flat_map(|h| [*h; 3])).all(|((a, b), h)| h || a == b));
    // Deterministic.
    assert_eq!(out, complete(w, h, ch, &dirty, &hole, &CompleteParams::default()).unwrap());
}

#[test]
fn completion_continues_structure() {
    // Vertical stripes: the filled hole should keep the stripes (not a blur).
    let (w, h, ch) = (80, 80, 1);
    let clean: Vec<f32> = (0..w * h).map(|i| if (i % w / 4) % 2 == 0 { 0.2 } else { 0.8 }).collect();
    let hole = disc(w, h, 40.0, 40.0, 7.0);
    let dirty: Vec<f32> = clean.iter().zip(&hole).map(|(c, h)| if *h { 0.5 } else { *c }).collect();
    let out = complete(w, h, ch, &dirty, &hole, &CompleteParams::default()).unwrap();
    let err = hole_rmse(&out, &clean, &hole, ch);
    assert!(err < 0.15, "{err}");
}

#[test]
fn proximity_finds_matching_offset() {
    let (w, h, ch) = (80, 60, 1);
    // Periodic pattern with period 10 in x: a match is any multiple-of-10 displacement.
    let img: Vec<f32> = (0..w * h).map(|i| ((i % w) as f32 * std::f32::consts::TAU / 10.0).sin() * 0.4 + 0.5).collect();
    let hole = disc(w, h, 40.0, 30.0, 5.0);
    let (dx, dy) = best_offset(w, h, ch, &img, &hole, 3, 20).unwrap();
    assert_eq!(dx.rem_euclid(10), 0, "dx {dx} dy {dy}");
    assert!((dx, dy) != (0, 0));
}

#[test]
fn proximity_search_clamps_hostile_radii_without_overflow_or_hanging() {
    let (w, h, ch) = (24, 20, 1);
    let img: Vec<f32> = (0..w * h).map(|i| (i % w) as f32 / w as f32).collect();
    let hole = disc(w, h, 12.0, 10.0, 2.0);
    // User-supplied values used to overflow i32 loops, cast a huge usize
    // ring into a negative number, or take billions of search iterations.
    let result = best_offset(w, h, ch, &img, &hole, usize::MAX, i32::MAX);
    if let Some((dx, dy)) = result {
        assert!(dx.abs() <= w as i32 && dy.abs() <= h as i32);
    }
    assert_eq!(best_offset(w, h, ch, &img, &hole, 2, -1), None);
    assert_eq!(best_offset(w, h, ch, &img, &hole, 2, 0), None);
}

/// Structure, fine noise and flat patches (so that some matches tie exactly), `ch` channels.
fn busy(w: usize, h: usize, ch: usize, seed: u64) -> Vec<f32> {
    let mut rng = Rng::new(seed);
    (0..w * h)
        .flat_map(|i| {
            let (x, y) = (i % w, i / w);
            let flat = (x / 9 + y / 7) % 5 == 0;
            let n = (rng.next() % 1000) as f32 / 1000.0;
            (0..ch).map(move |c| if flat { 0.25 + 0.1 * c as f32 } else { (0.2 + 0.004 * x as f32 + 0.1 * ((x + 2 * y + c) % 6) as f32 + 0.05 * n).min(1.0) })
        })
        .collect()
}

fn bits(v: &[f32]) -> Vec<u32> {
    v.iter().map(|x| x.to_bits()).collect()
}

/// Roles: a hole, a band that is context only, a corner that is no image.
fn roles(w: usize, h: usize, hole: &[bool]) -> Vec<Px> {
    (0..w * h)
        .map(|i| {
            let (x, y) = (i % w, i / w);
            if hole[i] {
                Px::Hole
            } else if x < 4 && y < 6 {
                Px::Void
            } else if (w * 3 / 4..w * 3 / 4 + 3).contains(&x) {
                Px::Known
            } else {
                Px::Source
            }
        })
        .collect()
}

#[test]
fn a_hole_is_as_thick_as_it_is_through_not_along() {
    let (w, h) = (240, 120);
    let all = Bounds { x0: 0, y0: 0, x1: w, y1: h };
    let thickness = |hole: &[bool]| {
        let px: Vec<Px> = hole.iter().map(|&o| if o { Px::Hole } else { Px::Source }).collect();
        hole_thickness(w, hole_bounds(w, h, &px).unwrap_or(all), &px)
    };
    // A band 200 px long and 9 px thick; a disc of radius 20; the band touching the edge
    // counts the outside of the image as not hole.
    let band: Vec<bool> = (0..w * h).map(|i| (20..220).contains(&(i % w)) && (50..59).contains(&(i / w))).collect();
    assert!((9..=11).contains(&thickness(&band)), "{}", thickness(&band));
    assert!((39..=42).contains(&thickness(&disc(w, h, 120.0, 60.0, 20.0))));
    let edge: Vec<bool> = (0..w * h).map(|i| i / w < 9).collect();
    assert!((9..=11).contains(&thickness(&edge)), "{}", thickness(&edge));
}

/// A long thin hole goes the thin way (a shallow pyramid that starts from a membrane): it is
/// filled, only it changes, and the result does not depend on the thread count either.
#[test]
#[cfg(not(target_arch = "wasm32"))]
fn thin_holes_are_filled_the_same_on_any_number_of_threads() {
    let (w, h) = (300, 200);
    let img = busy(w, h, 3, 5);
    // A wavy line 6 px thick across the image.
    let hole: Vec<bool> = (0..w * h).map(|i| (((i % w) as f32 * 0.05).sin() * 30.0 + 100.0 - (i / w) as f32).abs() < 3.0).collect();
    let px = roles(w, h, &hole);
    let run = |threads: usize| {
        let pool = rayon::ThreadPoolBuilder::new().num_threads(threads).build().unwrap();
        pool.install(|| complete_masked(w, h, 3, &img, &px, &CompleteParams::default(), &photocraft_raster::Interrupt::NONE)).unwrap().unwrap()
    };
    let one = run(1);
    assert_eq!(bits(&one), bits(&run(7)));
    assert!((0..w * h).all(|i| hole[i] || (0..3).all(|c| one[i * 3 + c].to_bits() == img[i * 3 + c].to_bits())));
    assert!((0..w * h).filter(|&i| hole[i]).all(|i| (0..3).all(|c| one[i * 3 + c].is_finite())));
}

#[test]
#[cfg(not(target_arch = "wasm32"))]
fn results_do_not_depend_on_the_thread_count() {
    // Big enough for the parallel passes, and for PatchMatch to sweep in wavefronts where
    // there are fewer bands than threads (never with one thread).
    let (w, h) = (300, 260);
    for (ch, adapt) in [(3, Adapt::NONE), (4, Adapt { gain: (0.9, 1.1), bias: 0.04 })] {
        let img = busy(w, h, ch, 3);
        let hole = disc(w, h, 140.0, 130.0, 40.0);
        let px = roles(w, h, &hole);
        let p = CompleteParams { adapt, ..Default::default() };
        let run = |threads: usize| {
            let pool = rayon::ThreadPoolBuilder::new().num_threads(threads).build().unwrap();
            bits(&pool.install(|| complete_masked(w, h, ch, &img, &px, &p, &photocraft_raster::Interrupt::NONE)).unwrap().unwrap())
        };
        let one = run(1);
        assert_eq!(one, run(3), "{ch} channels, 3 threads");
        assert_eq!(one, run(16), "{ch} channels, 16 threads");
    }
}

/// The onion peel as first written: every source in order, every known pixel summed.
fn onion_peel_plain(w: usize, h: usize, ch: usize, r: i32, img: &mut [f32], px: &[Px], sources: &[(i32, i32)]) {
    let mut known: Vec<bool> = px.iter().map(|k| matches!(k, Px::Known | Px::Source)).collect();
    loop {
        let front: Vec<usize> = (0..w * h)
            .filter(|&i| {
                let (x, y) = ((i % w) as i32, (i / w) as i32);
                !known[i]
                    && px[i] == Px::Hole
                    && (-1..=1).any(|dy: i32| {
                        (-1..=1).any(|dx: i32| {
                            let (nx, ny) = (x + dx, y + dy);
                            nx >= 0 && ny >= 0 && (nx as usize) < w && (ny as usize) < h && known[ny as usize * w + nx as usize]
                        })
                    })
            })
            .collect();
        if front.is_empty() {
            break;
        }
        let vals: Vec<Vec<f32>> = front
            .iter()
            .map(|&i| {
                let (x, y) = ((i % w) as i32, (i / w) as i32);
                let mut best = (f32::INFINITY, (0i32, 0i32));
                for &(sx, sy) in sources {
                    let (mut e, mut n) = (0.0f32, 0usize);
                    for dy in -r..=r {
                        for dx in -r..=r {
                            let (tx, ty) = (x + dx, y + dy);
                            if tx < 0 || ty < 0 || tx as usize >= w || ty as usize >= h || !known[ty as usize * w + tx as usize] {
                                continue;
                            }
                            let (ti, si) = ((ty as usize * w + tx as usize) * ch, ((sy + dy) as usize * w + (sx + dx) as usize) * ch);
                            for c in 0..ch {
                                let d = img[ti + c] - img[si + c];
                                e += d * d;
                            }
                            n += 1;
                        }
                    }
                    if n > 0 && e / (n as f32) < best.0 {
                        best = (e / n as f32, (sx, sy));
                    }
                }
                let si = (best.1.1 as usize * w + best.1.0 as usize) * ch;
                img[si..si + ch].to_vec()
            })
            .collect();
        for (&i, v) in front.iter().zip(vals) {
            img[i * ch..(i + 1) * ch].copy_from_slice(&v);
            known[i] = true;
        }
    }
}

/// #2805's case (upstream's own speed-up of the onion peel, superseded here): repeating
/// texture with many exact ties, two holes; the result must be the plain scan's to the bit.
#[test]
fn onion_peel_picks_exactly_what_the_plain_scan_picks() {
    let (w, h, ch, r) = (48, 40, 3, 3);
    let mut img = texture(w, h);
    for y in 0..h {
        for x in 6..w / 2 {
            for c in 0..ch {
                img[(y * w + x) * ch + c] = img[(y * w + x % 6) * ch + c];
            }
        }
    }
    let none = photocraft_raster::Interrupt::NONE;
    for hole in [disc(w, h, 20.0, 18.0, 7.5), disc(w, h, 34.0, 22.0, 5.0)] {
        let px: Vec<Px> = hole.iter().map(|h| if *h { Px::Hole } else { Px::Source }).collect();
        let near = integral(w, h, &hole);
        let sources: Vec<(i32, i32)> = (r..h as i32 - r)
            .flat_map(|y| (r..w as i32 - r).map(move |x| (x, y)))
            .filter(|&(x, y)| window_count(&near, w, h, x - r, y - r, x + r + 1, y + r + 1) == 0)
            .collect();
        let (mut fast, mut plain) = (img.clone(), img.clone());
        onion_peel(w, h, ch, r, &mut fast, &px, hole_bounds(w, h, &px).unwrap(), &sources, &none).unwrap();
        onion_peel_plain(w, h, ch, r, &mut plain, &px, &sources);
        assert!(fast.iter().zip(&plain).all(|(a, b)| a.to_bits() == b.to_bits()));
        assert_ne!(fast, img);
    }
}

#[test]
fn onion_peel_takes_the_plain_scans_sources() {
    let none = photocraft_raster::Interrupt::NONE;
    for (w, h, ch, seed) in [(44, 36, 1, 1), (50, 40, 3, 2), (40, 40, 4, 3)] {
        let mut img = busy(w, h, ch, seed);
        // Also an image of a few exact values, where many sources tie.
        if seed == 3 {
            img.iter_mut().for_each(|v| *v = (*v * 3.0).round() / 3.0);
        }
        let hole = disc(w, h, w as f32 * 0.45, h as f32 * 0.5, 7.5);
        let px = roles(w, h, &hole);
        let valid = valid_sources(w, h, 3, &px, &none).unwrap();
        let sources = Sources::new(&valid, w).all(&valid, w);
        let (mut a, mut b) = (img.clone(), img.clone());
        onion_peel(w, h, ch, 3, &mut a, &px, hole_bounds(w, h, &px).unwrap(), &sources, &none).unwrap();
        onion_peel_plain(w, h, ch, 3, &mut b, &px, &sources);
        assert_eq!(bits(&a), bits(&b), "{w}×{h}×{ch}");
    }
}

#[test]
fn least_sum_is_where_the_quotient_crosses() {
    let mut rng = Rng::new(5);
    for i in 0..4000 {
        let n = (1 + rng.next() % 49) as f32;
        let v = match i % 4 {
            0 => 0.0,
            1 => (rng.next() % 1000) as f32 * 1e-9,
            _ => (rng.next() % 1_000_000) as f32 / 1_000_000.0,
        };
        for strict in [true, false] {
            let holds = |e: f32| if strict { e / n > v } else { e / n >= v };
            let e = least_sum(v, n, strict);
            assert!(holds(e), "{v} {n} {strict}: {e}");
            assert!(e == 0.0 || !holds(e.next_down()), "{v} {n} {strict}: {e} is not the least");
        }
    }
    assert_eq!(least_sum(f32::INFINITY, 4.0, true), f32::INFINITY);
}

#[test]
fn valid_sources_are_the_wholly_copyable_patches() {
    let (w, h, r) = (37, 29, 2);
    let mut rng = Rng::new(9);
    let px: Vec<Px> = (0..w * h)
        .map(|_| match rng.next() % 23 {
            0 => Px::Hole,
            1 => Px::Void,
            2 => Px::Known,
            _ => Px::Source,
        })
        .collect();
    let valid = valid_sources(w, h, r, &px, &photocraft_raster::Interrupt::NONE).unwrap();
    let source: Vec<bool> = px.iter().map(|k| *k == Px::Source).collect();
    let ss = integral(w, h, &source);
    let (ri, full) = (r as i32, ((2 * r + 1) * (2 * r + 1)) as u32);
    for y in 0..h as i32 {
        for x in 0..w as i32 {
            let inside = x >= ri && y >= ri && x + ri < w as i32 && y + ri < h as i32;
            let want = inside && window_count(&ss, w, h, x - ri, y - ri, x + ri + 1, y + ri + 1) == full;
            assert_eq!(valid[y as usize * w + x as usize], want, "({x}, {y})");
        }
    }
    // Counting per row and block finds the centres in order.
    let sources = Sources::new(&valid, w);
    let list = sources.all(&valid, w);
    assert!(list.len() > 10 && list.len() == sources.len());
    for (k, p) in list.iter().enumerate() {
        assert_eq!(sources.nth(k, &valid, w), *p);
    }
    // And on a grid wider than a block, with empty blocks and rows.
    let (w, h) = (300, 9);
    let wide: Vec<bool> = (0..w * h).map(|i| (i % w) % 7 == 3 && (i % w) / 64 != 2 && i / w != 4).collect();
    let sources = Sources::new(&wide, w);
    let list = sources.all(&wide, w);
    for (k, p) in list.iter().enumerate() {
        assert_eq!(sources.nth(k, &wide, w), *p);
    }
}

#[test]
fn full_patches_measure_as_clipped_ones() {
    let (w, h, ch, r) = (46, 38, 4, 3);
    let mut img = busy(w, h, ch, 4);
    let hole = disc(w, h, 20.0, 18.0, 6.0);
    let px = roles(w, h, &hole);
    let none = photocraft_raster::Interrupt::NONE;
    let hb = hole_bounds(w, h, &px).unwrap();
    let tb = hb.grow(r, w, h);
    let valid = valid_sources(w, h, r, &px, &none).unwrap();
    let picks: Vec<(i32, i32)> = Sources::new(&valid, w).all(&valid, w).iter().step_by(37).copied().collect();
    assert!(picks.len() > 5);
    for adapt in [Adapt::NONE, Adapt { gain: (0.8, 1.25), bias: 0.08 }] {
        for keep in [false, true] {
            let targets = targets_around(w, r, &px, hb, tb);
            let mut s = Solver {
                w,
                h,
                ch,
                r: r as i32,
                img: &mut img,
                px: &px,
                valid: valid.clone(),
                targets,
                hole_box: hb,
                tbox: tb,
                full: Vec::new(),
                tmom: Vec::new(),
                smom: Vec::new(),
                adapt,
                ctl: none,
            };
            s.prepare(keep);
            let nnf = vec![picks[0]; tb.area()];
            let mut cost = vec![f32::INFINITY; tb.area()];
            s.costs(&nnf, &mut cost).unwrap();
            let mut checked = 0;
            for k in 0..tb.area() {
                if !s.targets[k] || !s.full[k] {
                    continue;
                }
                let t = ((tb.x0 + k % tb.width()) as i32, (tb.y0 + k / tb.width()) as i32);
                for &src in &picks {
                    if adapt.is_none() {
                        let exact = s.dist_clipped(t, src, f32::INFINITY);
                        assert_eq!(s.dist_full(t, src, f32::INFINITY).to_bits(), exact.to_bits());
                        for cutoff in [exact * 0.5, exact, exact * 2.0] {
                            let (a, b) = (s.dist_full(t, src, cutoff), s.dist_clipped(t, src, cutoff));
                            assert_eq!(a < cutoff, b < cutoff);
                        }
                    } else {
                        let (mut m1, mut m2) = ([[0.0f32; 5]; 4], [[0.0f32; 5]; 4]);
                        s.moments_full(k, t, src, &mut m1);
                        assert_eq!(s.moments(t, src, &mut m2), (2 * r + 1) * (2 * r + 1));
                        assert_eq!(m1.map(|c| c.map(f32::to_bits)), m2.map(|c| c.map(f32::to_bits)));
                        assert_eq!(s.dist_adapted_full(k, t, src).to_bits(), s.dist_adapted(t, src).to_bits());
                    }
                    checked += 1;
                }
            }
            assert!(checked > 100, "{checked}");
        }
    }
}

#[test]
fn owned_completion_fills_in_place_or_gives_the_pixels_back() {
    let (w, h, ch) = (64, 52, 3);
    let img = busy(w, h, ch, 6);
    let hole = disc(w, h, 30.0, 26.0, 8.0);
    let px = roles(w, h, &hole);
    let (p, none) = (CompleteParams::default(), photocraft_raster::Interrupt::NONE);
    let borrowed = complete_masked(w, h, ch, &img, &px, &p, &none).unwrap().unwrap();
    let owned = complete_masked_owned(w, h, ch, img.clone(), &px, &p, &none).unwrap().unwrap();
    assert_eq!(bits(&owned), bits(&borrowed));
    // Nothing may be copied: no completion, and the pixels come back as they were.
    let known: Vec<Px> = px.iter().map(|k| if *k == Px::Source { Px::Known } else { *k }).collect();
    assert!(complete_masked(w, h, ch, &img, &known, &p, &none).unwrap().is_none());
    let back = complete_masked_owned(w, h, ch, img.clone(), &known, &p, &none).unwrap().unwrap_err();
    assert_eq!(bits(&back), bits(&img));
    // Cancelled, it stops.
    let yes = || true;
    assert!(complete_masked_owned(w, h, ch, img, &px, &p, &photocraft_raster::Interrupt::cancel_only(&yes)).is_err());
}

#[test]
fn synthesis_fills_hole_with_plausible_values() {
    let (w, h, ch) = (64, 64, 3);
    let clean = texture(w, h);
    let hole = disc(w, h, 32.0, 32.0, 6.0);
    let out = synthesize(w, h, ch, &clean, &hole, 8, 3);
    let err = hole_rmse(&out, &clean, &hole, ch);
    assert!(err < 0.15, "{err}");
    assert!(out.iter().all(|v| v.is_finite()));
}
