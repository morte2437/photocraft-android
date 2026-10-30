//! Isolate dab accumulation over a 6016×6016 (36 MP) plane, without rasterization/compositing.
//! Run in release; reports median of five runs after warmup. Compare the same harness on both
//! revisions. Resident tiles model a long stroke; moving dabs also exercise new tile allocation.
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
}
