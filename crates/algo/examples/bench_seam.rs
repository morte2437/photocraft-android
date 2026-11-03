//! Timing and output hash for Content-Aware Scale's seam carving core (P28).
//!
//! `cargo run --release -p photocraft-algo --example bench_seam [width height new_width channels]`
//!
//! Prints the carve time and an FNV-1a hash of the output's raw f32 bits, so two builds can be
//! compared with `cmp`-level strictness without writing tens of megabytes of pixels to disk.
use std::time::Instant;

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let num = |i: usize, d: usize| a.get(i).and_then(|s| s.parse().ok()).unwrap_or(d);
    let (w, h, new_w, ch) = (num(0, 6000), num(1, 4000), num(2, 5400), num(3, 4));
    if !(1..=4).contains(&ch) {
        eprintln!("channels must be 1..=4");
        std::process::exit(2);
    }
    // Deterministic photo-ish content: gradients with fine detail, so the energy map carries
    // structure instead of one flat value per row.
    let img: Vec<f32> = (0..w * h)
        .flat_map(|i| {
            let (x, y) = ((i % w) as f32, (i / w) as f32);
            let d = ((x * 0.13).sin() * (y * 0.07).cos()).abs() * 0.3;
            [(x / w as f32) * 0.6 + d, (y / h as f32) * 0.9 - d, d, 1.0].into_iter().take(ch)
        })
        .collect();
    let t = Instant::now();
    let out = photocraft_algo::seam::carve_width(w, h, ch, &img, None, new_w);
    let ms = t.elapsed().as_secs_f64() * 1e3;
    let hash =
        out.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, v| v.to_bits().to_le_bytes().iter().fold(h, |h, b| (h ^ u64::from(*b)).wrapping_mul(0x100_0000_01b3)));
    println!("{w}x{h}x{ch} -> {new_w}: {ms:.1} ms, {} samples, fnv {hash:016x}", out.len());
}
