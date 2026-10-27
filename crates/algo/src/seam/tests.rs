use super::*;

/// The straightforward implementation the per-seam carver replaced (full energy map, fresh
/// seam search and a new buffer per seam), kept as the oracle for `carver_matches_reference`.
mod reference {
    use super::super::{protection, resample_protection, transpose};

    pub(super) fn energy(w: usize, h: usize, ch: usize, img: &[f32]) -> Vec<f32> {
        let mut e = vec![0.0f32; w * h];
        for y in 0..h {
            for x in 0..w {
                let (xl, xr) = (x.saturating_sub(1), (x + 1).min(w - 1));
                let (yu, yd) = (y.saturating_sub(1), (y + 1).min(h - 1));
                let at = |xx: usize, yy: usize, c: usize| img[(yy * w + xx) * ch + c];
                let mut s = 0.0;
                for c in 0..ch {
                    let v = at(x, y, c);
                    s += (at(xr, y, c) - v).abs() + (v - at(xl, y, c)).abs();
                    s += (at(x, yd, c) - v).abs() + (v - at(x, yu, c)).abs();
                }
                let v = s * 0.5;
                e[y * w + x] = if v.is_nan() { f32::INFINITY } else { v };
            }
        }
        e
    }
    pub(super) fn find_seam(w: usize, h: usize, energy: &[f32]) -> Vec<usize> {
        let mut cost = energy[..w].to_vec();
        let mut from = vec![0u8; w * h]; // 0 = up-left, 1 = up, 2 = up-right
        let mut next = vec![0.0f32; w];
        for y in 1..h {
            for x in 0..w {
                let mut best = (cost[x], 1u8);
                if x > 0 && cost[x - 1] < best.0 {
                    best = (cost[x - 1], 0);
                }
                if x + 1 < w && cost[x + 1] < best.0 {
                    best = (cost[x + 1], 2);
                }
                next[x] = best.0 + energy[y * w + x];
                from[y * w + x] = best.1;
            }
            std::mem::swap(&mut cost, &mut next);
        }
        let mut x = (0..w).min_by(|a, b| cost[*a].total_cmp(&cost[*b])).unwrap_or(0);
        let mut seam = vec![0usize; h];
        for y in (0..h).rev() {
            seam[y] = x;
            if y > 0 {
                x = match from[y * w + x] {
                    0 => x - 1,
                    2 => x + 1,
                    _ => x,
                };
            }
        }
        seam
    }
    fn remove_seam(w: usize, h: usize, ch: usize, img: &[f32], seam: &[usize]) -> Vec<f32> {
        let mut out = Vec::with_capacity((w - 1) * h * ch);
        for (y, &sx) in seam.iter().enumerate().take(h) {
            let row = &img[y * w * ch..(y + 1) * w * ch];
            out.extend_from_slice(&row[..sx * ch]);
            out.extend_from_slice(&row[(sx + 1) * ch..]);
        }
        out
    }
    pub(super) fn carve_width(w: usize, h: usize, ch: usize, img: &[f32], protect: Option<&[f32]>, new_w: usize) -> Vec<f32> {
        let new_w = new_w.max(1);
        if new_w == w || w == 0 || h == 0 {
            return img.to_vec();
        }
        let protect_e = |e: &mut [f32], p: Option<&Vec<f32>>| {
            if let Some(p) = p {
                for (e, &p) in e.iter_mut().zip(p) {
                    *e += protection(p);
                }
            }
        };
        if new_w < w {
            let (mut cur, mut cw) = (img.to_vec(), w);
            let mut prot = protect.map(<[f32]>::to_vec);
            while cw > new_w {
                let mut e = energy(cw, h, ch, &cur);
                protect_e(&mut e, prot.as_ref());
                let seam = find_seam(cw, h, &e);
                cur = remove_seam(cw, h, ch, &cur, &seam);
                prot = prot.map(|p| remove_seam(cw, h, 1, &p, &seam));
                cw -= 1;
            }
            return cur;
        }
        // Enlarge in rounds of at most half the current width, each duplicating the k lowest seams.
        let (mut cur, mut cw) = (img.to_vec(), w);
        let mut prot = protect.map(<[f32]>::to_vec);
        while cw < new_w {
            let k = (new_w - cw).min((cw / 2).max(1));
            // Find k seams on a shrinking copy, mapping each back to original columns.
            let mut idx: Vec<Vec<usize>> = (0..h).map(|_| (0..cw).collect()).collect();
            let (mut tmp, mut tw) = (cur.clone(), cw);
            let mut tprot = prot.clone();
            let mut dup: Vec<Vec<usize>> = vec![Vec::with_capacity(k); h];
            for _ in 0..k {
                // A 1 px wide image's only seam is its one column: take it, so it is duplicated.
                if tw == 0 {
                    break;
                }
                let mut e = energy(tw, h, ch, &tmp);
                protect_e(&mut e, tprot.as_ref());
                let seam = find_seam(tw, h, &e);
                for (y, &sx) in seam.iter().enumerate() {
                    dup[y].push(idx[y].remove(sx));
                }
                tmp = remove_seam(tw, h, ch, &tmp, &seam);
                tprot = tprot.map(|p| remove_seam(tw, h, 1, &p, &seam));
                tw -= 1;
            }
            let added = dup[0].len();
            if added == 0 {
                break;
            }
            let nw = cw + added;
            let mut out = Vec::with_capacity(nw * h * ch);
            let mut nprot = prot.as_ref().map(|_| Vec::with_capacity(nw * h));
            for (y, d) in dup.iter_mut().enumerate() {
                d.sort_unstable();
                let row = &cur[y * cw * ch..(y + 1) * cw * ch];
                let mut di = 0;
                for x in 0..cw {
                    out.extend_from_slice(&row[x * ch..(x + 1) * ch]);
                    if let (Some(np), Some(p)) = (nprot.as_mut(), prot.as_ref()) {
                        np.push(p[y * cw + x]);
                    }
                    while di < d.len() && d[di] == x {
                        let xr = (x + 1).min(cw - 1);
                        for c in 0..ch {
                            out.push((row[x * ch + c] + row[xr * ch + c]) * 0.5);
                        }
                        if let (Some(np), Some(p)) = (nprot.as_mut(), prot.as_ref()) {
                            np.push(p[y * cw + x]);
                        }
                        di += 1;
                    }
                }
            }
            cur = out;
            prot = nprot;
            cw = nw;
        }
        cur
    }

    pub(super) fn carve(w: usize, h: usize, ch: usize, img: &[f32], protect: Option<&[f32]>, new_w: usize, new_h: usize) -> Vec<f32> {
        let (new_w, new_h) = (new_w.max(1), new_h.max(1));
        let wide = carve_width(w, h, ch, img, protect, new_w);
        if new_h == h {
            return wide;
        }
        let prot_t = protect.map(|p| transpose(new_w, h, 1, &resample_protection(w, h, p, new_w)));
        let t = transpose(new_w, h, ch, &wide);
        let tall = carve_width(h, new_w, ch, &t, prot_t.as_deref(), new_h);
        transpose(new_h, new_w, ch, &tall)
    }
}

/// The carver gives bit-identical output to the reference implementation it replaced, over
/// random shapes (including 1 px wide and tall), channel counts, tie-heavy quantised noise,
/// protection, shrinking, enlarging past half the width and both axes at once.
#[test]
fn carver_matches_reference() {
    let mut seed = 1u32;
    let mut next = |n: usize| {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        seed as usize % n
    };
    for case in 0..400u32 {
        let (w, h, ch) = (1 + next(24), 1 + next(18), [1, 3, 4][next(3)]);
        let levels = [2, 5, 255, 65_536][next(4)];
        let img = noise(w * h * ch, levels, 11 + case);
        let protect = (next(3) == 0).then(|| noise(w * h, 3, 101 + case));
        let (nw, nh) = (1 + next(2 * w + 2), 1 + next(2 * h + 2));
        let got = carve(w, h, ch, &img, protect.as_deref(), nw, nh);
        let want = reference::carve(w, h, ch, &img, protect.as_deref(), nw, nh);
        let bits = |v: &[f32]| v.iter().map(|f| f.to_bits()).collect::<Vec<_>>();
        assert_eq!(bits(&got), bits(&want), "case {case}: {w}x{h}x{ch} -> {nw}x{nh}, levels {levels}, protect {}", protect.is_some());
    }
    // The seam search alone, on raw energy maps full of ties.
    for case in 0..200u32 {
        let (w, h) = (1 + next(40), 1 + next(30));
        let e = noise(w * h, [2, 3, 1000][next(3)], 7 + case);
        assert_eq!(find_seam(w, h, &e), reference::find_seam(w, h, &e), "seam case {case}: {w}x{h}");
        let img = noise(w * h * 3, 7, 3 + case);
        let bits = |v: Vec<f32>| v.into_iter().map(f32::to_bits).collect::<Vec<_>>();
        assert_eq!(bits(energy(w, h, 3, &img)), bits(reference::energy(w, h, 3, &img)), "energy case {case}");
    }
}

/// After every removal, the carver's incrementally updated seam search matches a full pass
/// over its current energy bit for bit (path costs and the seam traced from them), on images
/// where seams run through the edges, through flat areas full of ties and through protection.
/// Its energy matches a fresh map of the pixels left, and so do the pixels.
#[test]
fn incremental_dp_matches_full_dp() {
    let mut ramp = vec![0.0f32; 120 * 90];
    for (i, v) in ramp.iter_mut().enumerate() {
        let (x, y) = ((i % 120) as f32, (i / 120) as f32);
        *v = ((x * 0.07).sin() * (y * 0.05).cos() * 0.5 + 0.5).clamp(0.0, 1.0);
    }
    // NaN and infinite samples and NaN protection: infinite energies, never NaN costs.
    let mut odd = noise(40 * 30 * 3, 255, 11);
    odd.iter_mut().step_by(13).for_each(|v| *v = f32::NAN);
    odd.iter_mut().skip(5).step_by(17).for_each(|v| *v = f32::INFINITY);
    let mut odd_prot = noise(40 * 30, 3, 12);
    odd_prot.iter_mut().step_by(7).for_each(|v| *v = -f32::NAN);
    // (w, h, channels, image, protection, seams to remove)
    type Case = (usize, usize, usize, Vec<f32>, Option<Vec<f32>>, usize);
    let cases: Vec<Case> = vec![
        (120, 90, 1, ramp, None, 40),
        (97, 64, 3, noise(97 * 64 * 3, 2, 5), None, 90),
        (80, 70, 4, noise(80 * 70 * 4, 1000, 6), Some(noise(80 * 70, 3, 7)), 60),
        (64, 50, 3, bar(64, 50), None, 63),
        (33, 1, 1, noise(33, 5, 8), None, 32),
        (2, 40, 3, noise(2 * 40 * 3, 9, 9), None, 2),
        (50, 40, 1, vec![0.5; 50 * 40], Some(noise(50 * 40, 2, 10)), 49),
        (40, 30, 3, odd, Some(odd_prot), 25),
    ];
    let bits = |v: &[f32]| v.iter().map(|f| f.to_bits()).collect::<Vec<_>>();
    for (case, (w, h, ch, img, prot, seams)) in cases.into_iter().enumerate() {
        let mut c = Carver::new(w, h, ch, &img, prot.as_deref());
        for step in 0..=seams {
            let rows = Rows { stride: c.w, cw: c.cw, off: &c.off };
            let mut fresh = Dp::default();
            fresh.full(rows, &c.e);
            for y in 0..h {
                assert_eq!(bits(&c.dp.cost[rows.of(y)]), bits(&fresh.cost[rows.of(y)]), "case {case} step {step} row {y}: costs");
            }
            assert_eq!(c.dp.seam(rows), fresh.seam(rows), "case {case} step {step}: seam");
            // The pixels left, packed, and their energy (plus protection) from scratch.
            let left: Vec<f32> =
                (0..h).flat_map(|y| c.idx[rows.of(y)].iter().flat_map(move |&x| (0..ch).map(move |k| (y * w + x as usize) * ch + k))).map(|i| img[i]).collect();
            let mut want = energy(c.cw, h, ch, &left);
            if let Some(p) = &prot {
                let kept = (0..h).flat_map(|y| c.idx[rows.of(y)].iter().map(move |&x| y * w + x as usize));
                for (e, i) in want.iter_mut().zip(kept) {
                    *e += protection(p[i]);
                }
            }
            let have: Vec<f32> = (0..h).flat_map(|y| c.e[rows.of(y)].to_vec()).collect();
            assert_eq!(bits(&have), bits(&want), "case {case} step {step}: energy");
            if step == seams {
                assert_eq!(bits(&c.into_pixels()), bits(&left), "case {case}: pixels");
                break;
            }
            c.remove_seam();
        }
    }
}

/// Hostile but well-formed shapes return early instead of panicking: no channels, empty
/// sizes (also through the height pass), and images or protection maps shorter than their
/// size says (a short map protects nothing).
#[test]
fn degenerate_inputs_do_not_panic() {
    assert!(carve(3, 3, 0, &[], None, 2, 2).is_empty());
    assert!(carve(3, 3, 0, &[], None, 5, 5).is_empty());
    assert!(transpose(3, 3, 0, &[]).is_empty());
    assert!(resize_bilinear(3, 3, 0, &[], 2, 2).is_empty());
    assert!(resize_bilinear(3, 3, 1, &[0.5; 9], 0, 2).is_empty());
    assert!(resize_bilinear(3, 3, 1, &[0.5; 9], 2, 0).is_empty());
    assert_eq!(resize_bilinear(3, 3, 1, &[0.5; 4], 2, 2), vec![0.0; 4]);
    assert!(carve(0, 5, 3, &[], None, 3, 7).is_empty());
    assert!(carve(0, 5, 3, &[], None, 3, 2).is_empty());
    let img = noise(10 * 5, 7, 3);
    assert_eq!(carve(10, 5, 1, &img, Some(&[0.0; 3]), 4, 5), carve(10, 5, 1, &img, None, 4, 5));
    assert_eq!(carve(10, 5, 1, &img, Some(&[0.0; 3]), 4, 3), carve(10, 5, 1, &img, None, 4, 3));
    assert_eq!(carve(10, 5, 3, &[0.5; 20], None, 4, 5), vec![0.5; 20]);
    assert_eq!(carve_width(usize::MAX, 2, 4, &[0.5; 8], None, 1), vec![0.5; 8]);
    // Values past the size are ignored, through both passes.
    let long = noise(20, 7, 4);
    assert_eq!(carve(5, 3, 1, &long, None, 3, 3), carve(5, 3, 1, &long[..15], None, 3, 3));
    assert_eq!(carve(5, 3, 1, &long, None, 5, 2), carve(5, 3, 1, &long[..15], None, 5, 2));
}

/// Infinite samples, and finite ones whose differences overflow, give energies that aren't
/// numbers; those are the highest energy (pinned deliberately: the code before NaN-free costs
/// fed NaN to the seam search and chose differently).
#[test]
fn infinite_samples_carve_deterministically() {
    let inf = f32::INFINITY;
    assert_eq!(carve(2, 1, 1, &[inf, 0.5], None, 1, 1), [0.5]);
    assert_eq!(carve(2, 1, 1, &[f32::MAX, f32::MAX], None, 6, 1), [f32::MAX, inf, inf, inf, inf, f32::MAX]);
    let mut img = noise(30 * 20 * 3, 255, 41);
    img.iter_mut().step_by(11).for_each(|v| *v = inf);
    img.iter_mut().skip(4).step_by(23).for_each(|v| *v = -inf);
    let out = carve(30, 20, 3, &img, None, 21, 14);
    assert_eq!(out.len(), 21 * 14 * 3);
    assert_eq!(out, carve(30, 20, 3, &img, None, 21, 14));
}

/// Infinite protection protects like full protection in the height pass too, where the map
/// is resampled (and an infinity alone would resample to NaN).
#[test]
fn infinite_protection_protects_in_both_passes() {
    let (w, h) = (6, 6);
    let img: Vec<f32> = (0..w * h).map(|i| (i * 7 % 5) as f32 / 4.0).collect();
    let row = |v: f32| (0..w * h).map(|i| if i / w == 2 { v } else { 0.0 }).collect::<Vec<f32>>();
    let (full, inf) = (row(1.0), row(f32::INFINITY));
    for (nw, nh) in [(6, 3), (4, 3), (3, 6)] {
        assert_eq!(carve(w, h, 1, &img, Some(&inf), nw, nh), carve(w, h, 1, &img, Some(&full), nw, nh), "{nw}x{nh}");
    }
    let kept = carve(w, h, 1, &img, Some(&inf), 6, 3);
    assert!(kept.chunks(6).any(|r| r == &img[12..18]), "the protected row survives");
}

/// A cancel that arrives after the seam search, during the tall-image rebuild, still stops
/// the carve.
#[test]
fn proxy_rebuild_observes_cancellation() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let (w, h, ch, new_w) = (128, PROXY_SHRINK_MIN_HEIGHT, 3, 64);
    let img = noise(w * h * ch, 7, 5);
    let calls = AtomicUsize::new(0);
    // One check per seam passes; the first one after the search cancels.
    let cancel = || calls.fetch_add(1, Ordering::Relaxed) >= w - new_w;
    assert_eq!(carve_width_with(w, h, ch, &img, None, new_w, &Interrupt::cancel_only(&cancel)), Err(Cancelled));
    assert!(calls.load(Ordering::Relaxed) > w - new_w, "the rebuild checked for cancellation");
}

/// NaN samples and NaN protection steer seams the same whatever their sign and payload
/// (which arithmetic does not preserve consistently across builds): the same pixels are kept.
#[test]
fn nan_payloads_do_not_steer_seams() {
    let (w, h, ch) = (24, 18, 3);
    let (base, prot_base) = (noise(w * h * ch, 255, 21), noise(w * h, 3, 22));
    let carve_with_nans = |pixel: f32, prot: f32| {
        let (mut img, mut p) = (base.clone(), prot_base.clone());
        img.iter_mut().step_by(37).for_each(|v| *v = pixel);
        p.iter_mut().step_by(29).for_each(|v| *v = prot);
        carve(w, h, ch, &img, Some(&p), 15, 11)
    };
    let canonical = |v: Vec<f32>| v.into_iter().map(|f| if f.is_nan() { u32::MAX } else { f.to_bits() }).collect::<Vec<_>>();
    let nans = [f32::NAN, -f32::NAN, f32::from_bits(0x7fc0_1234), f32::from_bits(0xffc0_0001)];
    let want = canonical(carve_with_nans(nans[0], nans[0]));
    for &a in &nans {
        for &b in &nans {
            assert_eq!(canonical(carve_with_nans(a, b)), want, "pixel NaN {:#x}, protection NaN {:#x}", a.to_bits(), b.to_bits());
        }
    }
}

#[test]
fn cancelled_carve_stops() {
    let (w, h) = (40, 20);
    let img = vec![0.5f32; w * h * 3];
    let yes = || true;
    assert_eq!(carve_with(w, h, 3, &img, None, 30, 15, &Interrupt::cancel_only(&yes)), Err(Cancelled));
    assert_eq!(carve_with(w, h, 3, &img, None, 30, 15, &Interrupt::NONE).unwrap(), carve(w, h, 3, &img, None, 30, 15));
}

/// A flat grey image with a salient vertical bar at x = 10..14 (red/blue columns).
fn bar(w: usize, h: usize) -> Vec<f32> {
    let mut v = vec![0.5f32; w * h * 3];
    for y in 0..h {
        for x in 10..14 {
            let c = if x % 2 == 0 { [1.0, 0.0, 0.0] } else { [0.0, 0.0, 1.0] };
            v[(y * w + x) * 3..(y * w + x) * 3 + 3].copy_from_slice(&c);
        }
    }
    v
}

/// Columns that are pure red or pure blue top to bottom.
fn red_columns(w: usize, h: usize, img: &[f32]) -> usize {
    let pure = |p: &[f32]| p[1] < 0.05 && ((p[0] > 0.95 && p[2] < 0.05) || (p[2] > 0.95 && p[0] < 0.05));
    (0..w).filter(|x| (0..h).all(|y| pure(&img[(y * w + x) * 3..(y * w + x) * 3 + 3]))).count()
}

#[test]
fn seam_follows_low_energy() {
    let (w, h) = (5, 4);
    let mut e = vec![9.0f32; w * h];
    for y in 0..h {
        e[y * w + 2] = 0.0;
    }
    assert_eq!(find_seam(w, h, &e), vec![2; 4]);
}

#[test]
fn shrinking_keeps_the_salient_bar() {
    let (w, h) = (40, 12);
    let img = bar(w, h);
    let out = carve(w, h, 3, &img, None, 24, h);
    assert_eq!(out.len(), 24 * h * 3);
    assert_eq!(red_columns(24, h, &out), 4, "the bar survives intact");
    // Plain resampling squashes it instead.
    let plain = resize_bilinear(w, h, 3, &img, 24, h);
    assert!(red_columns(24, h, &plain) < 4);
}

#[test]
fn tall_proxy_shrink_preserves_full_resolution_salient_and_protected_pixels() {
    let (w, h, ch, new_w) = (512, PROXY_SHRINK_MIN_HEIGHT, 3, 384);
    let mut img = bar(w, h);
    let mut protect = vec![0.0; w * h];
    for y in 0..h {
        let i = (y * w + 256) * ch;
        img[i..i + ch].copy_from_slice(&[0.03, 0.87, 0.42]);
        protect[y * w + 256] = 1.0;
    }

    let out = carve_width_with(w, h, ch, &img, Some(&protect), new_w, &Interrupt::NONE).unwrap();

    assert_eq!(out.len(), new_w * h * ch);
    assert_eq!(red_columns(new_w, h, &out), 4, "the full-resolution bar survives");
    for y in 0..h {
        let row = &out[y * new_w * ch..(y + 1) * new_w * ch];
        assert!(row.chunks_exact(ch).any(|px| px == [0.03, 0.87, 0.42]), "protected mark survives row {y}");
    }
}

#[test]
fn tall_proxy_shrink_observes_cancellation_before_modifying_source() {
    let (w, h, ch) = (128, PROXY_SHRINK_MIN_HEIGHT, 3);
    let img = vec![0.5; w * h * ch];
    let cancelled = || true;
    assert_eq!(carve_width_with(w, h, ch, &img, None, 64, &Interrupt::cancel_only(&cancelled)), Err(Cancelled));
}

#[test]
fn enlarging_duplicates_flat_areas() {
    let (w, h) = (30, 8);
    let img = bar(w, h);
    let out = carve(w, h, 3, &img, None, 50, h);
    assert_eq!(out.len(), 50 * h * 3);
    assert_eq!(red_columns(50, h, &out), 4);
}

#[test]
fn height_and_protection() {
    let (w, h) = (20, 30);
    // Horizontal bar: carve height.
    let mut img = vec![0.2f32; w * h];
    for x in 0..w {
        img[12 * w + x] = 1.0;
    }
    let out = carve(w, h, 1, &img, None, w, 18);
    assert_eq!(out.len(), w * 18);
    assert_eq!((0..18).filter(|y| (0..w).all(|x| out[y * w + x] > 0.9)).count(), 1);
    // Protecting a flat region keeps it: the seams go elsewhere.
    let flat = vec![0.5f32; 16 * 4];
    let mut protect = vec![0.0f32; 16 * 4];
    for y in 0..4 {
        protect[y * 16 + 3] = 1.0;
    }
    let mut marked = flat.clone();
    for y in 0..4 {
        marked[y * 16 + 3] = 0.51;
    }
    let out = carve(16, 4, 1, &marked, Some(&protect), 8, 4);
    assert!((0..4).all(|y| out[y * 8..y * 8 + 8].iter().any(|v| (*v - 0.51).abs() < 1e-6)));
}

/// Noise quantised to `levels` values, so the energy map is full of exact ties.
fn noise(len: usize, levels: u32, seed: u32) -> Vec<f32> {
    let mut s = seed | 1;
    (0..len)
        .map(|_| {
            s ^= s << 13;
            s ^= s >> 17;
            s ^= s << 5;
            (s % levels) as f32 / (levels - 1) as f32
        })
        .collect()
}

fn fnv(v: &[f32]) -> u64 {
    v.iter().flat_map(|f| f.to_bits().to_le_bytes()).fold(0xcbf2_9ce4_8422_2325, |h, b| (h ^ u64::from(b)).wrapping_mul(0x100_0000_01b3))
}

/// The exact output bits of a spread of carves (shrink, enlarge past half the width, both
/// axes, protection, 1/3/4 channels, tie-heavy and smooth images), pinned so that speed work
/// can't change which seams are chosen or how pixels are blended.
#[test]
fn output_bits_are_pinned() {
    let cases: [(usize, usize, usize, u32, bool, usize, usize); 8] = [
        (61, 47, 3, 2, false, 40, 47),
        (61, 47, 3, 255, false, 61, 30),
        (61, 47, 4, 4, true, 35, 29),
        (33, 20, 1, 3, false, 80, 20),
        (33, 20, 3, 1000, true, 50, 45),
        (48, 36, 4, 7, false, 70, 18),
        (5, 3, 1, 2, false, 1, 1),
        (40, 40, 3, 2, true, 39, 41),
    ];
    let got: Vec<u64> = cases
        .iter()
        .enumerate()
        .map(|(i, &(w, h, ch, levels, prot, nw, nh))| {
            let img = noise(w * h * ch, levels, 7 + i as u32);
            let protect = prot.then(|| noise(w * h, 3, 99 + i as u32));
            let out = carve(w, h, ch, &img, protect.as_deref(), nw, nh);
            assert_eq!(out.len(), nw * nh * ch);
            fnv(&out)
        })
        .collect();
    assert_eq!(got, PINNED);
}

const PINNED: [u64; 8] = [
    0x24ea_9353_02c2_7c58,
    0x5a6e_8554_a0cb_ebb1,
    0x8b18_d63f_396e_21c9,
    0x1db2_23ad_3e6a_a7c5,
    0x721d_6cc5_34e0_3fdf,
    0x961a_ea6d_e512_ec7d,
    0x4d25_767f_9dce_13f5,
    0x2157_9f86_27d6_b835,
];

/// [`output_bits_are_pinned`] for the reduced-height seam search of tall images (rows at
/// least [`PROXY_SHRINK_MIN_HEIGHT`], at least [`PROXY_SHRINK_MIN_SEAMS`] seams): widths and,
/// through the transposed pass, heights, with and without protection, on tie-heavy and
/// fine-grained noise.
#[test]
fn tall_proxy_output_bits_are_pinned() {
    let cases: [(usize, usize, usize, u32, bool, usize, usize); 5] = [
        (96, PROXY_SHRINK_MIN_HEIGHT, 3, 2, false, 30, PROXY_SHRINK_MIN_HEIGHT),
        (130, 2_100, 4, 255, true, 64, 2_100),
        (80, 2_600, 1, 65_536, false, 16, 2_600),
        (PROXY_SHRINK_MIN_HEIGHT, 90, 3, 5, false, PROXY_SHRINK_MIN_HEIGHT, 20),
        (2_050, 100, 4, 1000, true, 2_050, 36),
    ];
    let got: Vec<u64> = cases
        .iter()
        .enumerate()
        .map(|(i, &(w, h, ch, levels, prot, nw, nh))| {
            let img = noise(w * h * ch, levels, 31 + i as u32);
            let protect = prot.then(|| noise(w * h, 3, 131 + i as u32));
            let out = carve(w, h, ch, &img, protect.as_deref(), nw, nh);
            assert_eq!(out.len(), nw * nh * ch);
            fnv(&out)
        })
        .collect();
    assert_eq!(got, TALL_PINNED);
}

const TALL_PINNED: [u64; 5] = [0x5657_83ac_cb26_56a5, 0x81a6_b5f7_e299_fdfb, 0xcdf2_9df9_9b83_def5, 0xa713_544a_4dcc_d8d8, 0x3557_de9f_ffb0_881e];

#[test]
fn one_pixel_lines_enlarge_by_repeating() {
    // Width 1 has no seam to spare: its one column is repeated. Height 1 likewise (through
    // the transposed pass), alone or with the other axis.
    let col = [0.1f32, 0.2, 0.3, 0.4, 0.5];
    assert_eq!(carve(1, 5, 1, &col, None, 4, 5), col.iter().flat_map(|v| [*v; 4]).collect::<Vec<_>>());
    assert_eq!(carve(5, 1, 1, &col, None, 5, 3), col.repeat(3));
    assert_eq!(carve(1, 1, 3, &[0.7, 0.8, 0.9], None, 3, 2), [0.7, 0.8, 0.9].repeat(6));
    for (w, h, nw, nh) in [(1, 5, 6, 10), (1, 17, 6, 34), (16, 1, 21, 2), (7, 1, 3, 8), (2, 1, 1, 8)] {
        let img = vec![0.5f32; w * h * 4];
        assert_eq!(carve(w, h, 4, &img, Some(&vec![1.0; w * h]), nw, nh).len(), nw * nh * 4, "{w}x{h} -> {nw}x{nh}");
    }
}

#[test]
fn skin_rule() {
    let px = [0.86f32, 0.62, 0.5, 0.2, 0.4, 0.8];
    assert_eq!(skin_mask(2, 1, 3, &px), vec![1.0, 0.0]);
}
