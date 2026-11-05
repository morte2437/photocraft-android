//! Timing of the Remove Tool's target-mask construction (the dilation of the hole by the patch
//! radius, once per pyramid level) and of the whole fill, on a 24 or 36 MP RGBA f32 image. One
//! JSON line per repetition. Uses only APIs that exist before and after the change, so the same
//! file builds in a base checkout.
//! `cargo run --release -p photocraft-algo --example bench_remove_targets -- --mode phase --mp 24 --hole small --reps 20`
//!
//! * `--mode phase|e2e`: `phase` times the masks alone (`old_loop`, a verbatim copy of the loop
//!   the completion used to run, against `remove_dilate`); `e2e` times `remove_with`.
//! * `--hole small|scattered|large|dense`: a 48 px disc; about 20 single cells spread over the
//!   image; a centred ellipse (`--large 900x600` sets its extent); half the cells of a 2000x2000
//!   square (`phase` only).
use std::time::Instant;

use photocraft_algo::remove::{dilate, hole_radius, remove_with};
use photocraft_raster::Interrupt;

/// The patch radius of the completion's default parameters.
const R: usize = 3;

fn arg<'a>(args: &'a [String], name: &str) -> Option<&'a str> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).map(String::as_str)
}

fn hash(i: u64) -> u64 {
    let mut x = i.wrapping_add(0x9e37_79b9_7f4a_7c15);
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    x ^ (x >> 31)
}

fn fnv(bytes: impl Iterator<Item = u8>) -> u64 {
    bytes.fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3))
}

fn texture(w: usize, h: usize) -> Vec<f32> {
    let mut img = Vec::with_capacity(w * h * 4);
    for i in 0..w * h {
        let (x, y) = ((i % w) as f32, (i / w) as f32);
        let n = (hash(i as u64) >> 40) as f32 / (1u64 << 24) as f32;
        let v = 0.5 + 0.2 * (x * 0.05).sin() * (y * 0.07).cos() + 0.15 * n;
        img.extend_from_slice(&[v, v * 0.8 + 0.05 * (x * 0.01).cos(), v * 0.6, 1.0]);
    }
    img
}

fn hole_mask(kind: &str, w: usize, h: usize, large: (usize, usize)) -> Vec<bool> {
    let (cx, cy) = (w as f32 / 2.0, h as f32 / 2.0);
    let mut m = vec![false; w * h];
    match kind {
        "small" => (0..w * h).for_each(|i| m[i] = ((i % w) as f32 - cx).hypot((i / w) as f32 - cy) < 24.0),
        "scattered" => {
            for k in 0..20u64 {
                let (x, y) = (hash(k * 2) as usize % w, hash(k * 2 + 1) as usize % h);
                m[y * w + x] = true;
            }
        }
        "large" => {
            let (a, b) = (large.0 as f32 / 2.0, large.1 as f32 / 2.0);
            (0..w * h).for_each(|i| m[i] = (((i % w) as f32 - cx) / a).powi(2) + (((i / w) as f32 - cy) / b).powi(2) < 1.0);
        }
        "dense" => {
            for y in h / 2 - 1000..h / 2 + 1000 {
                for x in w / 2 - 1000..w / 2 + 1000 {
                    m[y * w + x] = hash((y * w + x) as u64) & 1 == 0;
                }
            }
        }
        _ => {}
    }
    m
}

/// The pyramid levels the completion builds for this hole: the same count formula and the same
/// 2x2 OR of the hole.
fn hole_levels(w: usize, h: usize, hole: &[bool]) -> Vec<(usize, usize, Vec<bool>)> {
    let n = 2 * R + 1;
    let no = hole_radius(w, h, hole).min(1 << 20);
    let want = (2.0 * f64::from(no) / n as f64).log2().ceil().max(1.0) as usize;
    let (mut lw, mut lh, mut levels) = (w, h, 1);
    while levels < want && lw / 2 >= 3 * n && lh / 2 >= 3 * n {
        (lw, lh, levels) = (lw.div_ceil(2), lh.div_ceil(2), levels + 1);
    }
    let mut out = vec![(w, h, hole.to_vec())];
    while out.len() < levels {
        let Some((pw, ph, prev)) = out.last() else { break };
        let (nw, nh) = (pw.div_ceil(2), ph.div_ceil(2));
        let next: Vec<bool> = (0..nw * nh)
            .map(|i| {
                let (fx, fy) = ((i % nw) * 2, (i / nw) * 2);
                (0..2).any(|dy| (0..2).any(|dx| fx + dx < *pw && fy + dy < *ph && prev[(fy + dy) * pw + fx + dx]))
            })
            .collect();
        out.push((nw, nh, next));
    }
    out
}

/// The loop the completion ran per level before the change, verbatim.
fn old_loop(lw: usize, lh: usize, hole: &[bool], r: usize) -> Vec<bool> {
    let ri = r as i32;
    let mut targets = vec![false; lw * lh];
    for (i, _) in hole.iter().enumerate().filter(|(_, m)| **m) {
        let (x, y) = ((i % lw) as i32, (i / lw) as i32);
        for ty in (y - ri).max(0)..=(y + ri).min(lh as i32 - 1) {
            for tx in (x - ri).max(0)..=(x + ri).min(lw as i32 - 1) {
                targets[ty as usize * lw + tx as usize] = true;
            }
        }
    }
    targets
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mode = arg(&args, "--mode").unwrap_or("phase");
    let mp: usize = arg(&args, "--mp").and_then(|v| v.parse().ok()).unwrap_or(24);
    let kind = arg(&args, "--hole").unwrap_or("small");
    let reps: usize = arg(&args, "--reps").and_then(|v| v.parse().ok()).unwrap_or(5);
    let large = arg(&args, "--large").and_then(|v| v.split_once('x')).and_then(|(a, b)| Some((a.parse().ok()?, b.parse().ok()?))).unwrap_or((900, 600));
    let (w, h) = if mp == 36 { (7360, 4912) } else { (6000, 4000) };
    let hole = hole_mask(kind, w, h, large);
    let line = |rep: usize, variant: &str, ms: f64, hash: u64| {
        println!("{{\"mode\":\"{mode}\",\"mp\":{mp},\"hole\":\"{kind}\",\"rep\":{rep},\"variant\":\"{variant}\",\"ms\":{ms:.3},\"hash\":\"{hash:016x}\"}}")
    };
    match mode {
        "phase" => {
            let levels = hole_levels(w, h, &hole);
            for rep in 0..reps {
                let t = Instant::now();
                let old: Vec<Vec<bool>> = levels.iter().map(|(lw, lh, m)| old_loop(*lw, *lh, m, R)).collect();
                let old_ms = t.elapsed().as_secs_f64() * 1e3;
                let t = Instant::now();
                let new: Vec<Vec<bool>> = levels.iter().map(|(lw, lh, m)| dilate(*lw, *lh, m, R)).collect();
                let new_ms = t.elapsed().as_secs_f64() * 1e3;
                let (ho, hn) = (fnv(old.iter().flatten().map(|b| u8::from(*b))), fnv(new.iter().flatten().map(|b| u8::from(*b))));
                assert_eq!(ho, hn, "the masks differ");
                line(rep, "old_loop", old_ms, ho);
                line(rep, "remove_dilate", new_ms, hn);
            }
        }
        "e2e" => {
            if kind == "dense" {
                eprintln!("the dense hole is a phase-only scenario");
                std::process::exit(2);
            }
            let img = texture(w, h);
            for rep in 0..reps {
                let t = Instant::now();
                let Ok(out) = remove_with(w, h, 4, &img, &hole, 1, &Interrupt::NONE) else {
                    eprintln!("cancelled");
                    std::process::exit(1);
                };
                let ms = t.elapsed().as_secs_f64() * 1e3;
                line(rep, "remove_with", ms, fnv(out.iter().flat_map(|v| v.to_bits().to_le_bytes())));
            }
        }
        _ => {
            eprintln!("unknown --mode {mode}");
            std::process::exit(2);
        }
    }
}
