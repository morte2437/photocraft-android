//! Isolate dab accumulation over a 6016×6016 (36 MP) plane, without rasterization/compositing.
//! Run in release; reports median of five runs after warmup. Compare the same harness on both
//! revisions. Resident tiles model a long stroke; moving dabs also exercise new tile allocation.
//! A final line times a large soft dab (3457x3457, about 3025 tiles) marching 500 px per dab on
//! fresh maps: first dab and steady-state median, in ms.
use std::hint::black_box;
use std::time::Instant;

use photocraft_geom::Rect;
use photocraft_paint::render::CoverageMap;

fn run(size: i32, moving: bool) -> (f64, f32) {
    let mut map = CoverageMap::new(3);
    // A long stroke has already visited 2048 tiles, including negative coordinates.
    for y in -16..16 {
        for x in -32..32 {
            map.accumulate(Rect::new(x * 64, y * 64, x * 64 + 1, y * 64 + 1), &[0.1], 0.8, None, None);
        }
    }
    let vals: Vec<f32> = (0..size * size)
        .map(|i| {
            let x = (i % size) as f32 + 0.5 - size as f32 / 2.0;
            let y = (i / size) as f32 + 0.5 - size as f32 / 2.0;
            (1.0 - (x * x + y * y).sqrt() / (size as f32 / 2.0)).max(0.0) * 0.1
        })
        .collect();
    let dabs = 4000;
    let start = Instant::now();
    for i in 0..dabs {
        let (x, y) = if moving { ((i * 17) % (6016 - size), (i * 7) % (6016 - size)) } else { (i % 31 - 15, i % 29 - 14) };
        map.accumulate(black_box(Rect::new(x, y, x + size, y + size)), black_box(&vals), 0.8, None, None);
    }
    let micros = start.elapsed().as_secs_f64() * 1e6 / f64::from(dabs);
    let checksum = black_box(map.get(0, 0) + map.get(128, 128));
    (micros, checksum)
}

/// Large soft dab marching along x on a fresh map: (first dab ms, median of the rest ms).
fn run_large() -> (f64, f64) {
    let size = 3457;
    let vals: Vec<f32> = (0..size * size)
        .map(|i| {
            let x = (i % size) as f32 + 0.5 - size as f32 / 2.0;
            let y = (i / size) as f32 + 0.5 - size as f32 / 2.0;
            (1.0 - (x * x + y * y).sqrt() / (size as f32 / 2.0)).max(0.0) * 0.1
        })
        .collect();
    let mut map = CoverageMap::new(3);
    let mut times = Vec::new();
    for i in 0..30 {
        let x = i * 500;
        let start = Instant::now();
        map.accumulate(black_box(Rect::new(x, 0, x + size, size)), black_box(&vals), 0.8, None, None);
        times.push(start.elapsed().as_secs_f64() * 1e3);
    }
    black_box(map.get(0, 0));
    let first = times.first().copied().unwrap_or(0.0);
    let mut rest: Vec<f64> = times.into_iter().skip(1).collect();
    rest.sort_by(f64::total_cmp);
    (first, rest.get(rest.len() / 2).copied().unwrap_or(0.0))
}

fn main() {
    for size in [20, 50, 128, 256] {
        for moving in [false, true] {
            black_box(run(size, moving));
            let mut times = Vec::new();
            let mut checksum = 0.0;
            for _ in 0..5 {
                let (micros, value) = run(size, moving);
                times.push(micros);
                checksum += value;
            }
            times.sort_by(f64::total_cmp);
            if let Some(median) = times.get(2) {
                println!("size={size} moving={moving} us_per_dab={median:.3} checksum={checksum:.7}");
            }
        }
    }
    let (mut firsts, mut steadies) = (Vec::new(), Vec::new());
    for _ in 0..5 {
        let (first, steady) = run_large();
        firsts.push(first);
        steadies.push(steady);
    }
    firsts.sort_by(f64::total_cmp);
    steadies.sort_by(f64::total_cmp);
    if let (Some(first), Some(steady)) = (firsts.get(2), steadies.get(2)) {
        println!("large dab 3457x3457 first_ms={first:.2} steady_ms={steady:.2}");
    }
}
