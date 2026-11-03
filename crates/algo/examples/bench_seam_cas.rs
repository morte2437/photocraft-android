//! Timing of Content-Aware Scale's seam carving alone (no document plumbing), on a synthetic
//! photo-like RGBA image: perf scenario P28's 24 MP width change (#211), the same image's height
//! (the transposed pass) and two images short enough for the exact full-resolution seam search.
//! Prints an FNV-style hash of each output's bits (over 32-bit words), so a speed-up can be checked
//! to choose exactly the same seams.
//! `cargo run --release -p photocraft-algo --example bench_seam_cas [reps] [case filter]`
use std::time::Instant;

use photocraft_algo::seam::carve;

/// Smooth gradients, a few hard-edged shapes and fine noise: enough structure that seams are
/// steered by content, as in a photo.
fn photo(w: usize, h: usize) -> Vec<f32> {
    let mut seed = 0x9e37_79b9u32;
    let mut noise = move || {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        (seed % 1000) as f32 / 1000.0 - 0.5
    };
    let mut img = Vec::with_capacity(w * h * 4);
    for y in 0..h {
        for x in 0..w {
            let (u, v) = (x as f32 / w as f32, y as f32 / h as f32);
            let sky = 0.35 + 0.4 * v;
            let disc = ((u - 0.3).powi(2) + (v - 0.55).powi(2)) < 0.02;
            let bar = (0.62..0.66).contains(&u) && v > 0.3;
            let hill = v > 0.7 + 0.08 * (u * 11.0).sin();
            let (r, g, b) = if disc {
                (0.85, 0.3, 0.2)
            } else if bar {
                (0.1, 0.1, 0.12)
            } else if hill {
                (0.2, 0.45 + 0.1 * (u * 40.0).sin(), 0.15)
            } else {
                (sky * 0.6, sky * 0.8, sky)
            };
            let n = noise() * 0.02;
            img.extend_from_slice(&[r + n, g + n, b + n, 1.0]);
        }
    }
    img
}

fn fnv(v: &[f32]) -> u64 {
    v.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, f| (h ^ u64::from(f.to_bits())).wrapping_mul(0x0100_0000_01b3))
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let reps = args.get(1).and_then(|s| s.parse::<usize>().ok()).unwrap_or(1).clamp(1, 20);
    let only = args.get(2).cloned();
    // (name, w, h, new w, new h)
    let cases = [
        ("24 MP to 90 % width (P28, proxy rows)", 6000, 4000, 5400, 4000),
        ("24 MP to 90 % height (transposed, proxy rows)", 6000, 4000, 6000, 3600),
        ("5.4 MP to 90 % width (exact)", 3000, 1800, 2700, 1800),
        ("1.5 MP to 70 % width (exact)", 1500, 1000, 1050, 1000),
    ];
    for (name, w, h, nw, nh) in cases {
        if only.as_ref().is_some_and(|o| !name.contains(o.as_str())) {
            continue;
        }
        let img = photo(w, h);
        let mut times = Vec::with_capacity(reps);
        let mut hash = 0;
        for _ in 0..reps {
            let t = Instant::now();
            let out = carve(w, h, 4, &img, None, nw, nh);
            times.push(t.elapsed().as_secs_f64() * 1000.0);
            hash = fnv(&out);
        }
        times.sort_by(f64::total_cmp);
        let p50 = times[times.len() / 2];
        let max = times[times.len() - 1];
        println!("{name:<48} p50 {p50:>9.1} ms  max {max:>9.1} ms  output {hash:016x}");
    }
}
