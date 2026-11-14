//! Camera Raw's preview develop on the 858×572 proxy of a 6000×4000 image: the CPU develop
//! against the GPU passes (upload, render, readback), for per-pixel settings and for the spatial
//! stages (Dehaze, Shadows and Highlights, Clarity, Texture) with them. "Source uploaded every
//! time" also makes Dehaze's copy on the CPU every time, as a white balance drag does.
//! `cargo run --release -p photocraft-gpu --example bench_camera_raw [-- --quick]`
use std::time::Instant;

use photocraft_algo::camera_raw::{CameraRaw, Wheel, develop, per_pixel, white_black_points};
use photocraft_gpu::CameraRawDevelop;

fn median(mut v: Vec<f64>) -> f64 {
    v.sort_by(f64::total_cmp);
    v[v.len() / 2]
}

fn time(runs: usize, mut f: impl FnMut()) -> f64 {
    f();
    median(
        (0..runs)
            .map(|_| {
                let t = Instant::now();
                f();
                t.elapsed().as_secs_f64() * 1e3
            })
            .collect(),
    )
}

/// [`time`] for a call that can fail: its first error ends the benchmark instead of being timed.
fn try_time<E>(runs: usize, mut f: impl FnMut() -> Result<(), E>) -> Result<f64, E> {
    let mut failed = None;
    let ms = time(runs, || {
        if failed.is_none() {
            failed = f().err();
        }
    });
    failed.map_or(Ok(ms), Err)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let runs = if std::env::args().any(|a| a == "--quick") { 5 } else { 40 };
    let (w, h) = (858usize, 572usize);
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
    let options = wgpu::RequestAdapterOptions { power_preference: wgpu::PowerPreference::HighPerformance, ..Default::default() };
    let adapter = pollster::block_on(instance.request_adapter(&options))?;
    let (device, queue) = pollster::block_on(adapter.request_device(&Default::default()))?;
    let info = adapter.get_info();
    println!("adapter: {} ({:?}), proxy {w}x{h}, median of {runs}", info.name, info.backend);
    let built = Instant::now();
    let mut pass = CameraRawDevelop::new(&device)?;
    println!("pipeline build: {:.1} ms", built.elapsed().as_secs_f64() * 1e3);
    // A photo-like proxy: gradients, noise and bright patches, so the filter has edges to keep.
    let src: Vec<[f32; 4]> = (0..w * h)
        .map(|i| {
            let (x, y) = ((i % w) as f32 / w as f32, (i / w) as f32 / h as f32);
            let n = ((i as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) >> 40) as f32 / (1u64 << 24) as f32;
            if (i / w) % 90 < 25 && (i % w) % 130 < 40 {
                return [0.95, 0.92, 0.88, 1.0];
            }
            let c = [0.02 + 0.8 * x * x, 0.05 + 0.6 * y, 0.6 - 0.5 * x * y].map(|v| (v + (n - 0.5) * 0.05).clamp(0.0, 1.0));
            [c[0], c[1], c[2], 1.0]
        })
        .collect();
    let scale = w as f32 / 6000.0;
    let mut hsl = [0.0; 8];
    hsl[1] = 30.0;
    hsl[5] = -40.0;
    let per_pixel_stages = CameraRaw {
        temperature: 20.0,
        tint: 5.0,
        exposure: 0.4,
        contrast: 25.0,
        whites: 30.0,
        blacks: -20.0,
        vibrance: 30.0,
        saturation: 10.0,
        curve_lights: 20.0,
        point_curve: vec![[0.0, 8.0], [128.0, 140.0], [255.0, 250.0]],
        hsl_hue: hsl,
        hsl_sat: hsl,
        hsl_lum: hsl,
        grade_shadows: Wheel { hue: 210.0, sat: 30.0, lum: 0.0 },
        grade_highlights: Wheel { hue: 40.0, sat: 20.0, lum: 5.0 },
        vignette_amount: -30.0,
        grain_amount: 25.0,
        pixel_scale: scale,
        ..Default::default()
    };
    let sets = [
        (
            "Basic (WB, Exposure, Contrast, Whites, Blacks)",
            CameraRaw { temperature: 20.0, exposure: 0.4, contrast: 25.0, whites: 30.0, blacks: -20.0, pixel_scale: scale, ..Default::default() },
        ),
        ("every per-pixel stage", per_pixel_stages.clone()),
        ("Shadows +50", CameraRaw { shadows: 50.0, pixel_scale: scale, ..Default::default() }),
        ("Highlights -50", CameraRaw { highlights: -50.0, pixel_scale: scale, ..Default::default() }),
        ("Shadows +50, Highlights -50", CameraRaw { shadows: 50.0, highlights: -50.0, pixel_scale: scale, ..Default::default() }),
        ("Shadows +50, Highlights -50, every per-pixel stage", CameraRaw { shadows: 50.0, highlights: -50.0, ..per_pixel_stages.clone() }),
        ("Clarity +50", CameraRaw { clarity: 50.0, pixel_scale: scale, ..Default::default() }),
        ("Texture +50", CameraRaw { texture: 50.0, pixel_scale: scale, ..Default::default() }),
        ("Dehaze +40", CameraRaw { dehaze: 40.0, pixel_scale: scale, ..Default::default() }),
        ("Clarity +50, Texture +50, Dehaze +40, every per-pixel stage", CameraRaw { clarity: 50.0, texture: 50.0, dehaze: 40.0, ..per_pixel_stages.clone() }),
        (
            "Clarity, Texture, Dehaze, Shadows, Highlights, every per-pixel stage",
            CameraRaw { clarity: 50.0, texture: 50.0, dehaze: 40.0, shadows: 50.0, highlights: -50.0, ..per_pixel_stages },
        ),
    ];
    for (name, p) in &sets {
        let toned = p.spatial();
        let cpu = time(if toned { runs.min(10) } else { runs }, || {
            let mut px = src.clone();
            develop(&mut px, w, h, p, false);
        });
        let points = white_black_points(&src, w, h, p);
        let stages = per_pixel(p, w, h, false, points).ok_or("not developed on the GPU")?;
        let size = [w as u32, h as u32];
        let upload = try_time(runs, || pass.develop(&device, &queue, None, &src, None, size, &stages).map(drop))?;
        let cached = try_time(runs, || pass.develop(&device, &queue, None, &src, Some(1), size, &stages).map(drop))?;
        println!("{name}:");
        println!("  CPU develop                          {cpu:7.2} ms");
        println!("  GPU, source uploaded every time      {upload:7.2} ms");
        println!("  GPU, source cached (slider drag)     {cached:7.2} ms");
        if !toned && (p.whites != 0.0 || p.blacks != 0.0) {
            let points_ms = time(runs, || {
                std::hint::black_box(white_black_points(&src, w, h, p));
            });
            println!("  Whites/Blacks points on the CPU      {points_ms:7.2} ms (cached per WB/Exposure/Contrast in the dialog)");
        }
    }
    Ok(())
}
