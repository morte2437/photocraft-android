//! Timing for the heavier second-batch filters on a ~24 MP (6000×4000) RGBA8 layer.
//!
//! `cargo run --release -p photocraft-algo --example bench_filters2 [name-substring] [--reps N] [--quick] [--json <file>]`
//!
//! `--json` writes the report `cargo xtask perf` reads: `{"rows": [{"name", "samples_ms"}]}`.
//! `--quick` runs the list on a 1500×1000 layer instead, for `cargo xtask perf --quick`.
use photocraft_algo::*;
use photocraft_color::PixelFormat;
use photocraft_geom::Rect;
use photocraft_raster::Surface;

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let flag = |k: &str| a.iter().position(|v| v == k);
    let value = |k: &str| flag(k).and_then(|i| a.get(i + 1)).cloned();
    let mut only: Option<String> = None;
    let mut i = 0;
    while i < a.len() {
        match a[i].as_str() {
            "--json" | "--reps" => i += 2,
            "--quick" => i += 1,
            _ => {
                only.get_or_insert_with(|| a[i].clone());
                i += 1;
            }
        }
    }
    let reps: usize = value("--reps").and_then(|v| v.parse().ok()).unwrap_or(1).max(1);
    let quick = flag("--quick").is_some();
    let (w, h) = if quick { (1500, 1000) } else { (6000, 4000) };
    let r = Rect::new(0, 0, w, h);
    let mut s = Surface::new(PixelFormat::RGBA8);
    // Busy deterministic content (gradients plus texture) so edge-aware filters do real work.
    let bytes: Vec<u8> = (0..h).flat_map(|y| (0..w).flat_map(move |x| [(x % 256) as u8, ((x * 3 + y) / 11 % 256) as u8, ((x ^ y) % 256) as u8, 255])).collect();
    s.write_interleaved(r, &bytes);
    let cases = vec![
        FilterParams::OilPaint { stylization: 4.0, cleanliness: 5.0, scale: 1.0, bristle_detail: 5.0, lighting: true, angle: -60.0, shine: 1.0 },
        FilterParams::ReduceNoise { strength: 6.0, preserve_details: 60.0, reduce_color_noise: 45.0, sharpen_details: 25.0, remove_jpeg_artifact: false },
        FilterParams::LensBlur {
            radius: 15.0,
            blades: 6,
            curvature: 0.0,
            rotation: 0.0,
            depth: DepthSource::None,
            focal_distance: 0.0,
            invert_depth: false,
            brightness: 20.0,
            threshold: 230.0,
            noise: 0.0,
            distribution: Distribution::Uniform,
            monochromatic: false,
            seed: 0,
            depth_map: None,
        },
        FilterParams::SmartBlur { radius: 3.0, threshold: 25.0, quality: BlurQuality::High, mode: SmartBlurMode::Normal },
        FilterParams::SmartBlur { radius: 12.0, threshold: 25.0, quality: BlurQuality::High, mode: SmartBlurMode::Normal },
        FilterParams::SmartBlur { radius: 100.0, threshold: 25.0, quality: BlurQuality::High, mode: SmartBlurMode::Normal },
        FilterParams::ShapeBlur { radius: 10.0, shape: BlurShape::Star },
        FilterParams::TiltShift { blur: 15.0, center_x: 0.5, center_y: 0.5, angle: 0.0, focus: 0.1, transition: 0.15 },
        FilterParams::IrisBlur { pins: vec![IrisPin::default()] },
        FilterParams::FieldBlur { pins: vec![FieldPin { x: 0.2, y: 0.2, blur: 0.0 }, FieldPin { x: 0.8, y: 0.8, blur: 30.0 }] },
        FilterParams::SpinBlur { pins: vec![SpinPin::default()] },
        FilterParams::PathBlur { paths: vec![BlurPath::default()] },
        FilterParams::Crystallize { cell_size: 10.0, seed: 0 },
        FilterParams::ColorHalftone { max_radius: 8.0, angles: [108.0, 162.0, 90.0, 45.0] },
        FilterParams::LightingEffects {
            lights: vec![Light::default()],
            gloss: 0.0,
            metallic: 0.0,
            exposure: 0.0,
            ambience: 8.0,
            texture: TextureChannel::Luminance,
            height: 50.0,
            white_is_high: true,
        },
        FilterParams::Relight { angle: 45.0, elevation: 40.0, intensity: 40.0, ambient: 55.0, warmth: 0.0, softness: 25.0 },
    ];
    let mut rows: Vec<(String, Vec<f64>)> = Vec::new();
    for p in cases {
        if only.as_deref().is_some_and(|o| !p.label().to_lowercase().contains(&o.to_lowercase())) {
            continue;
        }
        let area = output_area(&p, s.content_bounds(), r, None).intersect(&r);
        // Smart Blur's cost peaks near radius 12, where High quality visits every pixel in the circle.
        let name = match p {
            FilterParams::SmartBlur { radius, .. } => format!("{} r{radius}", p.label()),
            _ => p.label().to_string(),
        };
        let mut samples = Vec::with_capacity(reps);
        for _ in 0..reps {
            let t = std::time::Instant::now();
            let out = apply(&s, &p, area, r, None);
            let ms = t.elapsed().as_secs_f64() * 1e3;
            samples.push(ms);
            drop(out);
        }
        let mut sorted = samples.clone();
        sorted.sort_by(f64::total_cmp);
        let median = match sorted.len() % 2 {
            1 => sorted[sorted.len() / 2],
            _ => (sorted[sorted.len() / 2 - 1] + sorted[sorted.len() / 2]) / 2.0,
        };
        println!("{name:<18} {w}x{h} RGBA8: {median:.1} ms{reps_note}", reps_note = if reps > 1 { format!(" (median of {reps})") } else { String::new() });
        rows.push((name, samples));
    }
    if let Some(path) = value("--json") {
        // Two fields per row and ASCII filter labels: hand-written rather than a serde dependency
        // this crate does not otherwise need.
        let body: Vec<String> = rows
            .iter()
            .map(|(name, samples)| {
                let s: Vec<String> = samples.iter().map(|v| format!("{v:.4}")).collect();
                format!("{{\"name\":\"{name}\",\"samples_ms\":[{}]}}", s.join(","))
            })
            .collect();
        let text = format!("{{\"rows\":[{}]}}", body.join(","));
        if let Err(e) = std::fs::write(&path, text) {
            eprintln!("write {path}: {e}");
            std::process::exit(1);
        }
        println!("report: {path}");
    }
}
