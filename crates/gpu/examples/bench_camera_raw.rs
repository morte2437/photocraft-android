//! Camera Raw's preview develop on the 858×572 proxy of a 6000×4000 image: the CPU develop
//! against the GPU pass (upload, render, readback), for per-pixel settings.
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
    let src: Vec<[f32; 4]> = (0..w * h)
        .map(|i| {
            let (x, y) = ((i % w) as f32 / w as f32, (i / w) as f32 / h as f32);
            [0.1 + 0.8 * x, 0.2 + 0.6 * y, 0.6 - 0.5 * x * y, 1.0]
        })
        .collect();
    let scale = w as f32 / 6000.0;
    let mut hsl = [0.0; 8];
    hsl[1] = 30.0;
    hsl[5] = -40.0;
    let sets = [
        (
            "Basic (WB, Exposure, Contrast, Whites, Blacks)",
            CameraRaw { temperature: 20.0, exposure: 0.4, contrast: 25.0, whites: 30.0, blacks: -20.0, pixel_scale: scale, ..Default::default() },
        ),
        (
            "every per-pixel stage",
            CameraRaw {
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
            },
        ),
    ];
    for (name, p) in &sets {
        let cpu = time(runs, || {
            let mut px = src.clone();
            develop(&mut px, w, h, p, false);
        });
        let points_ms = time(runs, || {
            std::hint::black_box(white_black_points(&src, w, h, p));
        });
        let points = white_black_points(&src, w, h, p);
        let stages = per_pixel(p, w, h, false, points).ok_or("not per pixel")?;
        let size = [w as u32, h as u32];
        let upload = try_time(runs, || pass.develop(&device, &queue, None, &src, None, size, &stages).map(drop))?;
        let cached = try_time(runs, || pass.develop(&device, &queue, None, &src, Some(1), size, &stages).map(drop))?;
        println!("{name}:");
        println!("  CPU develop                          {cpu:7.2} ms");
        println!("  GPU, source uploaded every time      {upload:7.2} ms");
        println!("  GPU, source cached (slider drag)     {cached:7.2} ms");
        println!("  Whites/Blacks points on the CPU      {points_ms:7.2} ms (cached per WB/Exposure/Contrast in the dialog)");
    }
    let shadows = CameraRaw { shadows: 50.0, pixel_scale: scale, ..Default::default() };
    let cpu = time(runs.min(10), || {
        let mut px = src.clone();
        develop(&mut px, w, h, &shadows, false);
    });
    println!("for reference, Shadows +50 (CPU only): {cpu:.2} ms");
    Ok(())
}
