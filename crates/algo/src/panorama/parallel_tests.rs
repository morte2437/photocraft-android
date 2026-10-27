use super::*;

fn mix(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// Deterministic data from integer arithmetic only (no libm): mostly `0..1`, with negatives,
/// signed zeros and large values so a different order of operations cannot hide.
fn noise(len: usize, seed: u64) -> Vec<f32> {
    (0..len as u64)
        .map(|i| {
            let z = mix(seed.wrapping_mul(0x1234_5678_9ABC).wrapping_add(i));
            let v = (z >> 40) as f32 / 16_777_216.0;
            match z & 31 {
                0 => -v,
                1 => -0.0,
                2 => v * 1000.0,
                3 => 0.0,
                _ => v,
            }
        })
        .collect()
}

/// Weights with exact zeros, exact ones and partial values, in blobs and noise.
fn weights(w: usize, h: usize, seed: u64) -> Vec<f32> {
    (0..w * h)
        .map(|i| {
            let (x, y) = (i % w, i / w);
            let z = mix(seed + i as u64);
            if (x / 5 + y / 3) % 3 == 0 {
                0.0
            } else if z.is_multiple_of(7) {
                ((z >> 20) % 100) as f32 / 100.0
            } else {
                1.0
            }
        })
        .collect()
}

fn bits(v: &[f32]) -> Vec<u32> {
    v.iter().map(|x| x.to_bits()).collect()
}

fn fnv(parts: &[&[f32]]) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    for p in parts {
        for x in *p {
            h ^= x.to_bits() as u64;
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    h
}

/// Sizes that cross the odd / even and tiny cases (1 wide, 1 high, 1×1, odd, power of two + 1).
const SIZES: [(usize, usize); 12] = [(1, 1), (1, 9), (9, 1), (2, 2), (3, 5), (4, 4), (17, 13), (64, 40), (65, 41), (97, 61), (130, 7), (7, 130)];

fn ref_reduce(w: usize, h: usize, ch: usize, src: &[f32]) -> (usize, usize, Vec<f32>) {
    let (nw, nh) = (w.div_ceil(2), h.div_ceil(2));
    let k = [1.0f32, 4.0, 6.0, 4.0, 1.0];
    // Separable 5-tap binomial, then decimate.
    let mut tmp = vec![0.0f32; nw * h * ch];
    for y in 0..h {
        for x in 0..nw {
            for c in 0..ch {
                let mut s = 0.0;
                for (t, kv) in k.iter().enumerate() {
                    let xx = (2 * x as i64 + t as i64 - 2).clamp(0, w as i64 - 1) as usize;
                    s += kv * src[(y * w + xx) * ch + c];
                }
                tmp[(y * nw + x) * ch + c] = s / 16.0;
            }
        }
    }
    let mut out = vec![0.0f32; nw * nh * ch];
    for y in 0..nh {
        for x in 0..nw {
            for c in 0..ch {
                let mut s = 0.0;
                for (t, kv) in k.iter().enumerate() {
                    let yy = (2 * y as i64 + t as i64 - 2).clamp(0, h as i64 - 1) as usize;
                    s += kv * tmp[(yy * nw + x) * ch + c];
                }
                out[(y * nw + x) * ch + c] = s / 16.0;
            }
        }
    }
    (nw, nh, out)
}

fn ref_expand(sw: usize, sh: usize, ch: usize, src: &[f32], w: usize, h: usize) -> Vec<f32> {
    // Bilinear upsampling at half-pixel-aligned positions.
    let mut out = vec![0.0f32; w * h * ch];
    for y in 0..h {
        let fy = ((y as f32 + 0.5) / 2.0 - 0.5).clamp(0.0, (sh - 1) as f32);
        let (y0, ty) = (fy.floor() as usize, fy - fy.floor());
        let y1 = (y0 + 1).min(sh - 1);
        for x in 0..w {
            let fx = ((x as f32 + 0.5) / 2.0 - 0.5).clamp(0.0, (sw - 1) as f32);
            let (x0, tx) = (fx.floor() as usize, fx - fx.floor());
            let x1 = (x0 + 1).min(sw - 1);
            for c in 0..ch {
                let p = |xx: usize, yy: usize| src[(yy * sw + xx) * ch + c];
                let top = p(x0, y0) + (p(x1, y0) - p(x0, y0)) * tx;
                let bot = p(x0, y1) + (p(x1, y1) - p(x0, y1)) * tx;
                out[(y * w + x) * ch + c] = top + (bot - top) * ty;
            }
        }
    }
    out
}

fn ref_push_pull(w: usize, h: usize, ch: usize, px: &mut [f32], wt: &[f32]) {
    if w <= 1 && h <= 1 {
        return;
    }
    let (nw, nh) = (w.div_ceil(2), h.div_ceil(2));
    let mut cpx = vec![0.0f32; nw * nh * ch];
    let mut cw = vec![0.0f32; nw * nh];
    for y in 0..h {
        for x in 0..w {
            let (i, j) = (y * w + x, (y / 2) * nw + x / 2);
            let a = wt[i];
            cw[j] += a;
            for c in 0..ch {
                cpx[j * ch + c] += a * px[i * ch + c];
            }
        }
    }
    for j in 0..nw * nh {
        if cw[j] > 0.0 {
            for c in 0..ch {
                cpx[j * ch + c] /= cw[j];
            }
            cw[j] = 1.0;
        }
    }
    if cw.contains(&0.0) && cw.iter().any(|v| *v > 0.0) {
        ref_push_pull(nw, nh, ch, &mut cpx, &cw);
    }
    for y in 0..h {
        for x in 0..w {
            let i = y * w + x;
            if wt[i] < 1.0 {
                let j = (y / 2) * nw + x / 2;
                let a = wt[i];
                for c in 0..ch {
                    px[i * ch + c] = a * px[i * ch + c] + (1.0 - a) * cpx[j * ch + c];
                }
            }
        }
    }
}

fn ref_multiband(w: usize, h: usize, imgs: &[RoiImage], weights: &[Vec<f32>], levels: usize) -> (Vec<f32>, Vec<f32>) {
    let ch = imgs.first().map_or(1, |i| i.ch);
    let levels = levels.max(1);
    let align = 1i32 << (levels - 1);
    // Level sizes of the panorama.
    let mut sizes = vec![(w, h)];
    for _ in 1..levels {
        let Some(&(pw, ph)) = sizes.last() else { break };
        sizes.push((pw.div_ceil(2), ph.div_ceil(2)));
    }
    let mut acc: Vec<Vec<f32>> = sizes.iter().map(|&(lw, lh)| vec![0.0; lw * lh * ch]).collect();
    let mut wsum: Vec<Vec<f32>> = sizes.iter().map(|&(lw, lh)| vec![0.0; lw * lh]).collect();
    let margin = align * 4;
    // Each image on its ROI grown by a margin and aligned to the coarsest level's grid.
    let bands = par_map(imgs.len(), |k| {
        let im = &imgs[k];
        let gx0 = ((im.x0 - margin).max(0) / align) * align;
        let gy0 = ((im.y0 - margin).max(0) / align) * align;
        let gx1 = (im.x0 + im.w as i32 + margin).min(w as i32);
        let gy1 = (im.y0 + im.h as i32 + margin).min(h as i32);
        let (gw, gh) = ((gx1 - gx0).max(1) as usize, (gy1 - gy0).max(1) as usize);
        let mut px = vec![0.0f32; gw * gh * ch];
        let mut cov = vec![0.0f32; gw * gh];
        let mut wt = vec![0.0f32; gw * gh];
        for y in 0..im.h {
            for x in 0..im.w {
                let (gx, gy) = (im.x0 + x as i32 - gx0, im.y0 + y as i32 - gy0);
                if gx < 0 || gy < 0 || gx as usize >= gw || gy as usize >= gh {
                    continue;
                }
                let (i, g) = (y * im.w + x, gy as usize * gw + gx as usize);
                cov[g] = im.alpha[i];
                wt[g] = weights[k][i] * im.alpha[i].min(1.0);
                px[g * ch..(g + 1) * ch].copy_from_slice(&im.px[i * ch..(i + 1) * ch]);
            }
        }
        let bin: Vec<f32> = cov.iter().map(|a| if *a > 0.5 { 1.0 } else { 0.0 }).collect();
        ref_push_pull(gw, gh, ch, &mut px, &bin);
        // Laplacian of the colours, Gaussian of the weights.
        let mut lap: Vec<(usize, usize, Vec<f32>)> = Vec::new();
        let mut gau: Vec<Vec<f32>> = Vec::new();
        let (mut cw, mut chh, mut cur, mut cwt) = (gw, gh, px, wt);
        for l in 0..levels {
            if l + 1 == levels {
                lap.push((cw, chh, cur.clone()));
                gau.push(cwt.clone());
                break;
            }
            let (nw, nh, down) = ref_reduce(cw, chh, ch, &cur);
            let up = ref_expand(nw, nh, ch, &down, cw, chh);
            lap.push((cw, chh, cur.iter().zip(&up).map(|(a, b)| a - b).collect()));
            gau.push(cwt.clone());
            let (_, _, wdown) = ref_reduce(cw, chh, 1, &cwt);
            cw = nw;
            chh = nh;
            cur = down;
            cwt = wdown;
        }
        (gx0, gy0, lap, gau)
    });
    for (gx0, gy0, lap, gau) in &bands {
        for (l, ((lw, lh, band), g)) in lap.iter().zip(gau).enumerate() {
            let (ox, oy) = ((gx0 >> l) as usize, (gy0 >> l) as usize);
            let (pw, ph) = sizes[l];
            for y in 0..*lh {
                let py = oy + y;
                if py >= ph {
                    break;
                }
                for x in 0..*lw {
                    let px = ox + x;
                    if px >= pw {
                        break;
                    }
                    let (i, p) = (y * lw + x, py * pw + px);
                    let k = g[i];
                    if k <= 0.0 {
                        continue;
                    }
                    wsum[l][p] += k;
                    for c in 0..ch {
                        acc[l][p * ch + c] += k * band[i * ch + c];
                    }
                }
            }
        }
    }
    for l in 0..levels {
        let (pw, ph) = sizes[l];
        for p in 0..pw * ph {
            let k = wsum[l][p];
            for c in 0..ch {
                acc[l][p * ch + c] = if k > 1e-8 { acc[l][p * ch + c] / k } else { 0.0 };
            }
        }
    }
    // Collapse.
    // `levels ≥ 1`, so there is always a coarsest level.
    let Some(mut cur) = acc.pop() else { return (vec![0.0; w * h * ch], vec![0.0; w * h]) };
    for l in (0..levels - 1).rev() {
        let (pw, ph) = sizes[l];
        let (sw, sh) = sizes[l + 1];
        let up = ref_expand(sw, sh, ch, &cur, pw, ph);
        cur = acc[l].iter().zip(&up).map(|(a, b)| a + b).collect();
    }
    // Coverage: union of the images.
    let mut cov = vec![0.0f32; w * h];
    for im in imgs {
        for y in 0..im.h {
            for x in 0..im.w {
                let (px, py) = (im.x0 + x as i32, im.y0 + y as i32);
                if px < 0 || py < 0 || px as usize >= w || py as usize >= h {
                    continue;
                }
                let p = py as usize * w + px as usize;
                cov[p] = cov[p].max(im.alpha[y * im.w + x]);
            }
        }
    }
    (cur, cov)
}

#[test]
fn reduce_and_expand_match_the_serial_code() {
    for (n, &(w, h)) in SIZES.iter().enumerate() {
        for ch in [1usize, 3, 4] {
            let src = noise(w * h * ch, n as u64 * 7 + ch as u64);
            let (nw, nh, got) = reduce(w, h, ch, &src);
            let (rw, rh, want) = ref_reduce(w, h, ch, &src);
            assert_eq!((nw, nh), (rw, rh));
            assert_eq!(bits(&got), bits(&want), "reduce {w}x{h}x{ch}");
            // Expand back to the original size and to sizes that are not twice the source's.
            for (ew, eh) in [(w, h), (2 * nw, 2 * nh), (2 * nw - 1, 2 * nh + 1), (w + 3, h + 5)] {
                let got = expand(nw, nh, ch, &want, ew, eh);
                let want_up = ref_expand(nw, nh, ch, &want, ew, eh);
                assert_eq!(bits(&got), bits(&want_up), "expand {nw}x{nh}x{ch} to {ew}x{eh}");
            }
        }
    }
}

#[test]
fn push_pull_matches_the_serial_code() {
    for (n, &(w, h)) in SIZES.iter().enumerate() {
        for ch in [1usize, 3] {
            for variant in 0..3u64 {
                let wt = match variant {
                    0 => weights(w, h, n as u64),
                    1 => vec![1.0; w * h],
                    // A single covered pixel: every level of the recursion has to fill from it.
                    _ => (0..w * h).map(|i| if i == (w * h) / 3 { 1.0 } else { 0.0 }).collect(),
                };
                let px = noise(w * h * ch, 100 + n as u64);
                let (mut got, mut want) = (px.clone(), px);
                push_pull(w, h, ch, &mut got, &wt);
                ref_push_pull(w, h, ch, &mut want, &wt);
                assert_eq!(bits(&got), bits(&want), "push_pull {w}x{h}x{ch} variant {variant}");
            }
        }
    }
}

/// ROIs of assorted sizes and offsets (some sticking out of the canvas), with holes in the alpha.
fn rois(w: usize, h: usize, ch: usize, seed: u64) -> (Vec<RoiImage>, Vec<Vec<f32>>) {
    let specs =
        [(0i32, 0i32, w * 2 / 3, h), (w as i32 / 3, -(h as i32) / 5, w * 2 / 3 + 1, h * 3 / 4 + 1), (-(w as i32) / 4, h as i32 / 2, w / 2 + 1, h / 2 + 3)];
    let imgs: Vec<RoiImage> = specs
        .iter()
        .enumerate()
        .map(|(k, &(x0, y0, iw, ih))| {
            let (iw, ih) = (iw.max(1), ih.max(1));
            let mut alpha = vec![1.0f32; iw * ih];
            for (i, a) in alpha.iter_mut().enumerate() {
                let z = mix(seed + k as u64 * 1000 + i as u64);
                if (i % iw) / 4 % 5 == 2 && (i / iw) % 6 < 3 {
                    *a = 0.0;
                } else if z.is_multiple_of(11) {
                    *a = 0.3;
                }
            }
            RoiImage { x0, y0, w: iw, h: ih, ch, px: noise(iw * ih * ch, seed + k as u64), alpha }
        })
        .collect();
    let wts = imgs.iter().enumerate().map(|(k, r)| weights(r.w, r.h, seed + 50 + k as u64)).collect();
    (imgs, wts)
}

#[test]
fn multiband_matches_the_serial_code() {
    for (n, &(w, h)) in [(1usize, 1usize), (5, 3), (40, 30), (97, 61), (130, 70), (64, 64), (201, 129)].iter().enumerate() {
        for ch in [1usize, 3] {
            let (imgs, wts) = rois(w, h, ch, n as u64 * 31 + ch as u64);
            for levels in [1usize, 2, 3, 5, 7] {
                let got = multiband(w, h, &imgs, &wts, levels);
                let want = ref_multiband(w, h, &imgs, &wts, levels);
                assert_eq!(bits(&got.0), bits(&want.0), "colours {w}x{h}x{ch} levels {levels}");
                assert_eq!(bits(&got.1), bits(&want.1), "coverage {w}x{h}x{ch} levels {levels}");
            }
        }
    }
    // No images at all, and zero levels.
    assert_eq!(multiband(9, 7, &[], &[], 3).0.len(), ref_multiband(9, 7, &[], &[], 3).0.len());
}

/// Batching the bands (#7758) changes when they are built, not the result. Budgets from none
/// (one image per batch) up to everything in steps of a fifth pass through every split of the
/// three images.
#[test]
fn multiband_batches_match_the_serial_code() {
    let budgets: Vec<usize> =
        std::iter::successors(Some(1024.0f64), |b| Some(b * 1.2)).take_while(|b| *b < 4e6).map(|b| b as usize).chain([0, usize::MAX]).collect();
    for (n, &(w, h)) in [(40usize, 30usize), (97, 61), (201, 129)].iter().enumerate() {
        for ch in [1usize, 3] {
            let (imgs, wts) = rois(w, h, ch, n as u64 * 17 + ch as u64);
            for levels in [1usize, 3, 5] {
                let want = ref_multiband(w, h, &imgs, &wts, levels);
                for &budget in &budgets {
                    let got = multiband_batched(w, h, &imgs, &wts, levels, budget);
                    assert_eq!(bits(&got.0), bits(&want.0), "colours {w}x{h}x{ch} levels {levels} budget {budget}");
                    assert_eq!(bits(&got.1), bits(&want.1), "coverage {w}x{h}x{ch} levels {levels} budget {budget}");
                }
            }
        }
    }
}

/// Hashes recorded from the code before it was made parallel (plain f32 arithmetic only, so
/// they hold on every platform).
#[test]
fn multiband_hashes_recorded_from_the_serial_code() {
    let (imgs, wts) = rois(97, 61, 3, 5);
    let (px, cov) = multiband(97, 61, &imgs, &wts, 5);
    assert_eq!(fnv(&[&px, &cov]), 0xf89a5726082ebb6c, "97x61 rgb, 5 levels");
    let (imgs, wts) = rois(130, 70, 1, 9);
    let (px, cov) = multiband(130, 70, &imgs, &wts, 7);
    assert_eq!(fnv(&[&px, &cov]), 0xb7d7ddc59d201730, "130x70 grey, 7 levels");
    let wt = weights(65, 41, 3);
    let mut px = noise(65 * 41 * 3, 4);
    push_pull(65, 41, 3, &mut px, &wt);
    assert_eq!(fnv(&[&px]), 0x50ea36a84ca9a68f, "push_pull 65x41 rgb");
    let (nw, nh, r) = reduce(65, 41, 3, &px);
    assert_eq!(fnv(&[&r, &expand(nw, nh, 3, &r, 65, 41)]), 0x51823acfb4206dd8, "reduce + expand 65x41 rgb");
}
