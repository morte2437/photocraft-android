//! `cargo bench -p photocraft-algo`
use criterion::{Criterion, criterion_group, criterion_main};
use photocraft_algo::{FilterParams, apply};
use photocraft_color::PixelFormat;
use photocraft_geom::Rect;
use photocraft_raster::Surface;

fn textured(n: u32) -> Surface {
    let r = Rect::new(0, 0, n as i32, n as i32);
    let mut s = Surface::new(PixelFormat::RGBA8);
    let bytes: Vec<u8> = (0..n).flat_map(|y| (0..n).flat_map(move |x| [(x ^ y) as u8, (x / 3) as u8, (y * 7) as u8, 255])).collect();
    s.write_interleaved(r, &bytes);
    s
}

/// Gaussian Blur's direct-kernel path (radius up to 4; larger radii use box passes), including
/// the variance-matched kernels below radius 1 (#8544).
fn gaussian_direct(c: &mut Criterion) {
    let n = 2048;
    let s = textured(n);
    let area = Rect::new(0, 0, n as i32, n as i32);
    for radius in [0.3f32, 0.8, 3.0] {
        let p = FilterParams::GaussianBlur { radius };
        c.bench_function(&format!("gaussian_blur_{radius}px_2048"), |b| b.iter(|| apply(&s, &p, area, area, None)));
    }
}

criterion_group! {
    name = benches;
    config = Criterion::default().sample_size(10);
    targets = gaussian_direct
}
criterion_main!(benches);
