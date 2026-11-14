//! Camera Raw's GPU preview pass against the CPU develop (the oracle), stage by stage, at 8, 16
//! and 32 bits. Skips, with a message, when there is no adapter or device or it can't render
//! `Rgba32Float`; with `WGPU_BACKEND` set those fail instead, as in `parity.rs`. A pipeline that
//! doesn't build on an adapter that can run it fails. The shader itself is validated without a
//! GPU ([`the_shader_validates_without_a_gpu`]), so a WGSL error fails on every runner.

use photocraft_algo::camera_raw::{CameraRaw, Wheel, develop, per_pixel, white_black_points};
use photocraft_gpu::{CameraRawDevelop, DeviceHealth, Fault};

/// Largest difference from the CPU develop, per channel. The pass evaluates the same
/// expressions in f32, so what remains is the GPU's own `pow`, `exp2` and `sin` (a few ulps) and
/// how they move a value across a table entry; measured at most about 1e-5 on NVIDIA (Vulkan,
/// DX12) and WARP. 1/4096 keeps a margin and stays well under one 8-bit step (the compositor's
/// tolerance is 1/255).
const TOL: f32 = 1.0 / 4096.0;

struct Gpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
    dev: CameraRawDevelop,
    _lock: std::sync::MutexGuard<'static, ()>,
}

fn gpu() -> Option<Gpu> {
    let lock = crate::GPU_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let required = std::env::var_os("WGPU_BACKEND").is_some();
    // `WGPU_BACKEND` picks the backend; `PHOTOCRAFT_TEST_FALLBACK_ADAPTER=1` the software one (WARP).
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
    let fallback = std::env::var_os("PHOTOCRAFT_TEST_FALLBACK_ADAPTER").is_some();
    let options = wgpu::RequestAdapterOptions { force_fallback_adapter: fallback, ..Default::default() };
    let adapter = match pollster::block_on(instance.request_adapter(&options)) {
        Ok(a) => a,
        Err(e) => {
            assert!(!required, "WGPU_BACKEND is set but has no adapter: {e}");
            eprintln!("skipping Camera Raw GPU tests: no adapter ({e})");
            return None;
        }
    };
    let info = adapter.get_info();
    if !CameraRawDevelop::supported(&adapter) {
        assert!(!required, "WGPU_BACKEND is set but {} can't render Rgba32Float", info.name);
        eprintln!("skipping Camera Raw GPU tests: adapter can't render Rgba32Float");
        return None;
    }
    let (device, queue) = match pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())) {
        Ok(d) => d,
        Err(e) => {
            assert!(!required, "WGPU_BACKEND is set but {} has no device: {e}", info.name);
            eprintln!("skipping Camera Raw GPU tests: no device ({e})");
            return None;
        }
    };
    eprintln!("Camera Raw GPU tests on {:?} ({})", info.backend, info.name);
    // The adapter can run the pass: a pipeline that doesn't build is a bug in it, not a skip.
    let dev = CameraRawDevelop::new(&device).unwrap_or_else(|e| panic!("the Camera Raw pipeline didn't build on {}: {}", info.name, e.0));
    Some(Gpu { device, queue, dev, _lock: lock })
}

/// The Camera Raw shaders parse and validate (naga, as wgpu does before any driver sees them),
/// with no adapter: the GPU tests below skip on runners without one. The filters are compiled
/// appended to the tone shader, as the pass builds them.
#[test]
fn the_shaders_validate_without_a_gpu() {
    let tone = format!("{}\n{}", photocraft_gpu::CAMERA_RAW_TONE_SHADER, photocraft_gpu::CAMERA_RAW_FILTERS_SHADER);
    for src in [photocraft_gpu::CAMERA_RAW_SHADER, tone.as_str()] {
        let module = wgpu::naga::front::wgsl::parse_str(src).unwrap_or_else(|e| panic!("{}", e.emit_to_string(src)));
        wgpu::naga::valid::Validator::new(wgpu::naga::valid::ValidationFlags::all(), wgpu::naga::valid::Capabilities::empty())
            .validate(&module)
            .unwrap_or_else(|e| panic!("{}", e.emit_to_string(src)));
        assert!(!module.entry_points.is_empty());
    }
}

#[derive(Clone, Copy, Debug)]
enum Depth {
    Eight,
    Sixteen,
    Float,
}

const DEPTHS: [Depth; 3] = [Depth::Eight, Depth::Sixteen, Depth::Float];

fn hash(i: usize, c: u64) -> f32 {
    let mut z = (i as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ c.wrapping_mul(0xC2B2_AE3D_27D4_EB4F);
    z = (z ^ (z >> 29)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    (z >> 40) as f32 / (1u64 << 24) as f32
}

/// Gradients, noise and saturated patches (every Color Mixer band), quantized to `depth`. The
/// 32-bit image also has values above 1 and a few below 0.
fn image(w: usize, h: usize, depth: Depth) -> Vec<[f32; 4]> {
    (0..w * h)
        .map(|i| {
            let (x, y) = ((i % w) as f32 / w as f32, (i / w) as f32 / h as f32);
            let mut c = if (i / w).is_multiple_of(4) {
                // A hue sweep at full and half saturation.
                let hue = x * 6.0;
                let f = |n: f32| (((hue + n) % 6.0 - 3.0).abs() - 1.0).clamp(0.0, 1.0);
                let s = if (i / w).is_multiple_of(8) { 1.0 } else { 0.5 };
                [f(0.0), f(4.0), f(2.0)].map(|v| (1.0 - s) * 0.6 + s * v)
            } else {
                [0.1 + 0.8 * x, 0.2 + 0.6 * y, 0.5 - 0.4 * x * y].map(|v| v + (hash(i, 1) - 0.5) * 0.15)
            };
            if let Depth::Float = depth {
                c = c.map(|v| v * (1.0 + 1.5 * y) - 0.02 * hash(i, 2));
            }
            let q = |v: f32| match depth {
                Depth::Eight => (v.clamp(0.0, 1.0) * 255.0).round() / 255.0,
                Depth::Sixteen => (v.clamp(0.0, 1.0) * 32768.0).round() / 32768.0,
                Depth::Float => v,
            };
            [q(c[0]), q(c[1]), q(c[2]), 1.0]
        })
        .collect()
}

/// Develops `p` both ways and returns the largest channel difference.
fn compare(g: &mut Gpu, p: &CameraRaw, depth: Depth) -> f32 {
    compare_at(g, p, depth, (171, 113))
}

fn compare_at(g: &mut Gpu, p: &CameraRaw, depth: Depth, (w, h): (usize, usize)) -> f32 {
    compare_image(g, p, depth, &image(w, h, depth), (w, h))
}

fn compare_image(g: &mut Gpu, p: &CameraRaw, depth: Depth, src: &[[f32; 4]], (w, h): (usize, usize)) -> f32 {
    let float = matches!(depth, Depth::Float);
    let src = src.to_vec();
    let mut cpu = src.clone();
    develop(&mut cpu, w, h, p, float);
    let points = white_black_points(&src, w, h, p);
    let stages = per_pixel(p, w, h, float, points).unwrap_or_else(|| panic!("{p:?} has a spatial stage"));
    let out = g.dev.develop(&g.device, &g.queue, None, &src, None, [w as u32, h as u32], &stages).unwrap();
    assert_eq!(out.len(), cpu.len());
    let mut worst = (0.0f32, 0usize);
    for (i, (a, b)) in out.iter().zip(&cpu).enumerate() {
        assert!(a.iter().all(|v| v.is_finite()), "{p:?} {depth:?}: non-finite GPU pixel {i}: {a:?}");
        assert_eq!(a[3], b[3], "alpha is kept");
        for c in 0..3 {
            let d = (a[c] - b[c]).abs();
            if d > worst.0 {
                worst = (d, i);
            }
        }
    }
    let (d, i) = worst;
    assert!(d <= TOL, "{p:?} {depth:?}: GPU {:?} vs CPU {:?} at pixel {i} (source {:?}), off by {d}", out[i], cpu[i], src[i]);
    if !float {
        assert!(out.iter().all(|q| q[..3].iter().all(|v| (0.0..=1.0).contains(v))), "{p:?} {depth:?}: clipped like the CPU");
    }
    d
}

fn check(name: &str, sets: &[CameraRaw]) {
    let Some(mut g) = gpu() else { return };
    for p in sets {
        for depth in DEPTHS {
            let d = compare(&mut g, p, depth);
            eprintln!("{name} {depth:?}: max |GPU - CPU| = {d:.2e}");
        }
    }
}

#[test]
fn white_balance_and_exposure() {
    check(
        "white balance",
        &[
            CameraRaw { temperature: 40.0, tint: -25.0, ..Default::default() },
            CameraRaw { temperature: -70.0, tint: 60.0, exposure: -1.2, ..Default::default() },
            CameraRaw { exposure: 1.5, ..Default::default() },
        ],
    );
}

#[test]
fn contrast_without_texture_or_clarity() {
    check("contrast", &[CameraRaw { contrast: 60.0, ..Default::default() }, CameraRaw { contrast: -80.0, exposure: 0.4, ..Default::default() }]);
}

#[test]
fn whites_and_blacks() {
    check(
        "whites/blacks",
        &[
            CameraRaw { whites: 70.0, blacks: -45.0, ..Default::default() },
            CameraRaw { whites: -60.0, blacks: 50.0, temperature: 20.0, ..Default::default() },
            CameraRaw { whites: 30.0, ..Default::default() },
            CameraRaw { blacks: -100.0, exposure: -0.5, ..Default::default() },
            // ProPhoto's power-law encoding.
            CameraRaw { whites: 50.0, blacks: -30.0, encoding_gamma: 1.8, ..Default::default() },
        ],
    );
}

#[test]
fn tone_curves() {
    check(
        "tone curves",
        &[
            CameraRaw {
                curve_highlights: -60.0,
                curve_lights: 30.0,
                curve_darks: 40.0,
                curve_shadows: -20.0,
                curve_splits: [20.0, 45.0, 80.0],
                ..Default::default()
            },
            CameraRaw { point_curve: vec![[0.0, 20.0], [64.0, 50.0], [192.0, 220.0], [255.0, 245.0]], ..Default::default() },
            CameraRaw {
                point_curve_red: vec![[0.0, 0.0], [128.0, 160.0], [255.0, 255.0]],
                point_curve_green: vec![[0.0, 10.0], [255.0, 230.0]],
                point_curve_blue: vec![[0.0, 0.0], [100.0, 70.0], [255.0, 255.0]],
                ..Default::default()
            },
        ],
    );
}

#[test]
fn vibrance_and_saturation() {
    check(
        "presence",
        &[
            CameraRaw { vibrance: 70.0, ..Default::default() },
            CameraRaw { saturation: -60.0, ..Default::default() },
            CameraRaw { vibrance: -40.0, saturation: 50.0, ..Default::default() },
        ],
    );
}

#[test]
fn color_mixer() {
    let ramp = |k: f32| std::array::from_fn(|i| ((i as f32 * 37.0 + k) % 200.0) - 100.0);
    check(
        "color mixer",
        &[
            CameraRaw { hsl_hue: ramp(0.0), ..Default::default() },
            CameraRaw { hsl_sat: ramp(50.0), ..Default::default() },
            CameraRaw { hsl_lum: ramp(120.0), ..Default::default() },
            CameraRaw { hsl_hue: ramp(10.0), hsl_sat: ramp(80.0), hsl_lum: ramp(150.0), ..Default::default() },
        ],
    );
}

#[test]
fn color_grading() {
    check(
        "color grading",
        &[
            CameraRaw { grade_shadows: Wheel { hue: 220.0, sat: 70.0, lum: 20.0 }, ..Default::default() },
            CameraRaw {
                grade_midtones: Wheel { hue: 120.0, sat: 30.0, lum: -15.0 },
                grade_highlights: Wheel { hue: 40.0, sat: 50.0, lum: 10.0 },
                grade_global: Wheel { hue: 300.0, sat: 15.0, lum: 5.0 },
                grade_blending: 80.0,
                grade_balance: -40.0,
                ..Default::default()
            },
        ],
    );
}

#[test]
fn vignette() {
    check(
        "vignette",
        &[
            CameraRaw { vignette_amount: -70.0, ..Default::default() },
            CameraRaw { vignette_amount: -50.0, vignette_roundness: -60.0, vignette_feather: 20.0, vignette_highlights: 70.0, ..Default::default() },
            CameraRaw {
                vignette_amount: 60.0,
                vignette_roundness: 80.0,
                vignette_midpoint: 20.0,
                vignette_style: "colorPriority".into(),
                ..Default::default()
            },
            CameraRaw { vignette_amount: -40.0, vignette_style: "paintOverlay".into(), ..Default::default() },
            CameraRaw { vignette_amount: 40.0, vignette_style: "paintOverlay".into(), ..Default::default() },
        ],
    );
}

#[test]
fn grain_in_full_resolution_coordinates() {
    check(
        "grain",
        &[
            CameraRaw { grain_amount: 60.0, seed: 3, ..Default::default() },
            // A preview proxy: the noise lattice is sampled at full-resolution positions.
            CameraRaw { grain_amount: 80.0, grain_size: 60.0, grain_roughness: 90.0, seed: u32::MAX, pixel_scale: 0.143, ..Default::default() },
            CameraRaw { grain_amount: 25.0, grain_size: 0.0, grain_roughness: 0.0, seed: 77, pixel_scale: 0.5, ..Default::default() },
        ],
    );
}

#[test]
fn every_per_pixel_stage_at_once() {
    let mut hsl = [0.0; 8];
    hsl[1] = 35.0;
    hsl[5] = -50.0;
    check(
        "all stages",
        &[CameraRaw {
            temperature: 25.0,
            tint: 10.0,
            exposure: 0.3,
            contrast: 25.0,
            whites: 20.0,
            blacks: -15.0,
            vibrance: 30.0,
            saturation: 10.0,
            curve_darks: 20.0,
            point_curve: vec![[0.0, 8.0], [255.0, 250.0]],
            hsl_hue: hsl,
            hsl_sat: hsl,
            hsl_lum: hsl,
            grade_shadows: Wheel { hue: 210.0, sat: 25.0, lum: 0.0 },
            vignette_amount: -30.0,
            grain_amount: 20.0,
            pixel_scale: 0.2,
            ..Default::default()
        }],
    );
}

#[test]
fn identity_leaves_pixels_alone_and_detail_has_no_pass() {
    let Some(mut g) = gpu() else { return };
    let p = CameraRaw::default();
    let src = image(64, 32, Depth::Float);
    let stages = per_pixel(&p, 64, 32, false, None).unwrap();
    let out = g.dev.develop(&g.device, &g.queue, None, &src, Some(1), [64, 32], &stages).unwrap();
    assert_eq!(out, src, "the CPU develop returns early, unclipped");
    for spatial in [
        CameraRaw { sharpen_amount: 10.0, ..Default::default() },
        CameraRaw { noise_luminance: 10.0, clarity: 20.0, ..Default::default() },
        CameraRaw { noise_color: 10.0, dehaze: 20.0, ..Default::default() },
    ] {
        assert!(spatial.spatial() && per_pixel(&spatial, 64, 32, false, Some((0.0, 1.0))).is_none(), "{spatial:?}");
    }
    // Whites and Blacks need the image's points, unless a spatial stage's passes measure them.
    assert!(per_pixel(&CameraRaw { whites: 10.0, ..Default::default() }, 64, 32, false, None).is_none());
    let toned = per_pixel(&CameraRaw { whites: 10.0, shadows: 20.0, ..Default::default() }, 64, 32, false, None).unwrap();
    assert!(toned.local_tone.is_some() && toned.whites_blacks.is_none() && toned.measure_whites_blacks == Some([10.0, 0.0]));
    for p in [
        CameraRaw { blacks: -10.0, dehaze: 20.0, ..Default::default() },
        CameraRaw { blacks: -10.0, clarity: 20.0, ..Default::default() },
        CameraRaw { blacks: -10.0, texture: 20.0, ..Default::default() },
    ] {
        let s = per_pixel(&p, 64, 32, false, None).unwrap();
        assert!(s.whites_blacks.is_none() && s.measure_whites_blacks == Some([0.0, -10.0]), "{p:?}");
    }
    // Wider than the filter's working copy, the develop filters a reduced copy: the CPU's.
    assert!(per_pixel(&CameraRaw { shadows: 20.0, ..Default::default() }, 1601, 32, false, None).is_none());
    // At 2 pixels on the short side the pass's pyramid has one level: the CPU's too.
    for (w, h) in [(2, 2), (40, 2), (2, 40)] {
        assert!(per_pixel(&CameraRaw { highlights: -20.0, ..Default::default() }, w, h, false, None).is_none(), "{w}×{h}");
    }
}

#[test]
fn a_cached_source_is_reused_and_a_new_key_uploads() {
    let Some(mut g) = gpu() else { return };
    let p = CameraRaw { exposure: 0.5, ..Default::default() };
    let stages = per_pixel(&p, 64, 32, false, None).unwrap();
    let a = image(64, 32, Depth::Eight);
    let b: Vec<[f32; 4]> = a.iter().map(|q| [q[2], q[0], q[1], 1.0]).collect();
    let first = g.dev.develop(&g.device, &g.queue, None, &a, Some(7), [64, 32], &stages).unwrap();
    // Same key: the uploaded source stays (callers key by the proxy's identity).
    assert_eq!(g.dev.develop(&g.device, &g.queue, None, &b, Some(7), [64, 32], &stages).unwrap(), first);
    let mut cpu = b.clone();
    develop(&mut cpu, 64, 32, &p, false);
    let other = g.dev.develop(&g.device, &g.queue, None, &b, Some(8), [64, 32], &stages).unwrap();
    assert!(other.iter().zip(&cpu).all(|(x, y)| (0..3).all(|c| (x[c] - y[c]).abs() <= TOL)));
}

#[test]
fn bad_sizes_and_faulted_devices_are_errors() {
    let Some(mut g) = gpu() else { return };
    let stages = per_pixel(&CameraRaw { exposure: 0.5, ..Default::default() }, 4, 4, false, None).unwrap();
    let px = vec![[0.5f32; 4]; 16];
    for size in [[0, 4], [4, 0], [5, 4], [3, 3], [u32::MAX, 1]] {
        assert!(g.dev.develop(&g.device, &g.queue, None, &px, None, size, &stages).is_err(), "{size:?}");
    }
    let health = DeviceHealth::new();
    health.mark(Fault::Lost("test".into()));
    assert!(g.dev.develop(&g.device, &g.queue, Some(&health), &px, None, [4, 4], &stages).is_err());
    // And the pass still works afterwards.
    assert!(g.dev.develop(&g.device, &g.queue, Some(&DeviceHealth::new()), &px, None, [4, 4], &stages).is_ok());
}

#[test]
fn shadows_and_highlights() {
    let sets = [
        CameraRaw { shadows: 50.0, ..Default::default() },
        CameraRaw { shadows: -70.0, ..Default::default() },
        CameraRaw { highlights: -50.0, ..Default::default() },
        CameraRaw { highlights: 60.0, ..Default::default() },
        CameraRaw { shadows: 100.0, highlights: -100.0, ..Default::default() },
        CameraRaw { shadows: 35.0, highlights: -45.0, encoding_gamma: 1.8, ..Default::default() },
    ];
    assert_moves(&sets);
    check("shadows/highlights", &sets);
}

#[test]
fn shadows_and_highlights_on_tiny_odd_and_flat_frames() {
    let Some(mut g) = gpu() else { return };
    let p = CameraRaw { shadows: 60.0, highlights: -40.0, whites: 20.0, ..Default::default() };
    // A frame 2 pixels on a side has a one-level pyramid: `per_pixel` sends it to the CPU, and
    // the pass still refuses stages built for it elsewhere.
    for (w, h) in [(2, 2), (40, 2)] {
        assert!(per_pixel(&p, w, h, false, None).is_none(), "{w}×{h}");
        let stages = per_pixel(&p, 3, 3, false, None).unwrap();
        assert!(g.dev.develop(&g.device, &g.queue, None, &image(w, h, Depth::Eight), None, [w as u32, h as u32], &stages).is_err());
    }
    for size in [(3, 3), (5, 3), (37, 23), (300, 7)] {
        for depth in DEPTHS {
            let d = compare_at(&mut g, &p, depth, size);
            eprintln!("shadows/highlights {size:?} {depth:?}: max |GPU - CPU| = {d:.2e}");
        }
    }
    // A flat frame has no range to filter.
    let flat = vec![[0.4, 0.35, 0.3, 1.0]; 64 * 48];
    for depth in DEPTHS {
        compare_image(&mut g, &p, depth, &flat, (64, 48));
    }
}

#[test]
fn shadows_and_highlights_with_per_pixel_stages() {
    let mut hsl = [0.0; 8];
    hsl[1] = 35.0;
    hsl[5] = -50.0;
    let all = CameraRaw {
        temperature: 25.0,
        tint: 10.0,
        exposure: 0.3,
        contrast: 25.0,
        shadows: 45.0,
        highlights: -55.0,
        whites: 20.0,
        blacks: -15.0,
        vibrance: 30.0,
        saturation: 10.0,
        curve_darks: 20.0,
        point_curve: vec![[0.0, 8.0], [255.0, 250.0]],
        hsl_hue: hsl,
        hsl_sat: hsl,
        hsl_lum: hsl,
        grade_shadows: Wheel { hue: 210.0, sat: 25.0, lum: 0.0 },
        vignette_amount: -30.0,
        grain_amount: 20.0,
        pixel_scale: 0.2,
        ..Default::default()
    };
    check(
        "shadows/highlights + per pixel",
        &[
            all.clone(),
            // Whites and Blacks measured after the filter, both directions.
            CameraRaw { shadows: 60.0, whites: 70.0, blacks: -45.0, ..Default::default() },
            CameraRaw { highlights: -60.0, whites: -60.0, blacks: 50.0, exposure: -0.4, ..Default::default() },
            CameraRaw { shadows: 30.0, highlights: -30.0, whites: 40.0, blacks: -30.0, encoding_gamma: 1.8, ..Default::default() },
        ],
    );
    // At the dialog's proxy size.
    let Some(mut g) = gpu() else { return };
    for depth in DEPTHS {
        let d = compare_at(&mut g, &all, depth, (858, 572));
        eprintln!("shadows/highlights + per pixel 858x572 {depth:?}: max |GPU - CPU| = {d:.2e}");
    }
}

/// Asserts that each of `sets` moves some pixel of the test image by over 2 %, so a comparison
/// with the CPU can't pass trivially.
fn assert_moves(sets: &[CameraRaw]) {
    for p in sets {
        let src = image(171, 113, Depth::Sixteen);
        let mut out = src.clone();
        develop(&mut out, 171, 113, p, false);
        let moved = out.iter().zip(&src).flat_map(|(a, b)| (0..3).map(move |c| (a[c] - b[c]).abs())).fold(0.0f32, f32::max);
        assert!(moved > 0.02, "{p:?} moves pixels by at most {moved}");
    }
}

#[test]
fn clarity_texture_and_dehaze() {
    let sets = [
        CameraRaw { clarity: 50.0, ..Default::default() },
        CameraRaw { clarity: -80.0, ..Default::default() },
        CameraRaw { texture: 50.0, ..Default::default() },
        CameraRaw { texture: -70.0, pixel_scale: 0.143, ..Default::default() },
        CameraRaw { dehaze: 40.0, ..Default::default() },
        CameraRaw { dehaze: 100.0, ..Default::default() },
        CameraRaw { dehaze: -60.0, ..Default::default() },
        // With Contrast, which then runs after them on the same luminance.
        CameraRaw { clarity: 40.0, texture: 30.0, contrast: -35.0, ..Default::default() },
        CameraRaw { dehaze: 30.0, clarity: 45.0, texture: -40.0, contrast: 20.0, ..Default::default() },
        CameraRaw { dehaze: -30.0, clarity: -25.0, texture: 60.0, encoding_gamma: 1.8, ..Default::default() },
    ];
    assert_moves(&sets);
    check("clarity/texture/dehaze", &sets);
}

#[test]
fn clarity_texture_and_dehaze_with_shadows_highlights_and_per_pixel_stages() {
    let mut hsl = [0.0; 8];
    hsl[1] = 35.0;
    hsl[5] = -50.0;
    let all = CameraRaw {
        temperature: 25.0,
        tint: 10.0,
        exposure: 0.3,
        contrast: 25.0,
        dehaze: 35.0,
        shadows: 45.0,
        highlights: -55.0,
        clarity: 40.0,
        texture: 30.0,
        whites: 20.0,
        blacks: -15.0,
        vibrance: 30.0,
        saturation: 10.0,
        curve_darks: 20.0,
        point_curve: vec![[0.0, 8.0], [255.0, 250.0]],
        hsl_hue: hsl,
        hsl_sat: hsl,
        hsl_lum: hsl,
        grade_shadows: Wheel { hue: 210.0, sat: 25.0, lum: 0.0 },
        vignette_amount: -30.0,
        grain_amount: 20.0,
        pixel_scale: 0.2,
        ..Default::default()
    };
    let sets = [
        all.clone(),
        CameraRaw { dehaze: 50.0, shadows: 60.0, highlights: -40.0, ..Default::default() },
        CameraRaw { clarity: 60.0, texture: 50.0, shadows: 40.0, ..Default::default() },
        // Whites and Blacks measured after them, without Shadows and Highlights, both directions.
        CameraRaw { dehaze: 40.0, whites: 60.0, blacks: -40.0, ..Default::default() },
        CameraRaw { clarity: 50.0, whites: -50.0, blacks: 45.0, exposure: -0.3, ..Default::default() },
        CameraRaw { texture: -60.0, whites: 30.0, blacks: -60.0, ..Default::default() },
    ];
    check("clarity/texture/dehaze + shadows/highlights + per pixel", &sets);
    let Some(mut g) = gpu() else { return };
    // At the dialog's proxy size (Dehaze and Clarity on a copy subsampled twice), and wider (three
    // times).
    for size in [(858, 572), (1600, 900)] {
        for depth in DEPTHS {
            let d = compare_at(&mut g, &all, depth, size);
            eprintln!("everything {size:?} {depth:?}: max |GPU - CPU| = {d:.2e}");
        }
    }
}

#[test]
fn clarity_texture_and_dehaze_on_tiny_and_odd_frames() {
    let Some(mut g) = gpu() else { return };
    let p = CameraRaw { dehaze: 45.0, clarity: 50.0, texture: 60.0, whites: 20.0, blacks: -20.0, ..Default::default() };
    // Without Shadows and Highlights even 1 and 2 pixel frames develop on the GPU.
    for size in [(1, 1), (2, 2), (40, 2), (3, 3), (5, 3), (37, 23), (300, 7)] {
        for depth in DEPTHS {
            let d = compare_at(&mut g, &p, depth, size);
            eprintln!("clarity/texture/dehaze {size:?} {depth:?}: max |GPU - CPU| = {d:.2e}");
        }
    }
    // Texture's blur on a small proxy is too narrow to change anything; the stage still runs.
    let narrow = CameraRaw { texture: 80.0, contrast: 10.0, pixel_scale: 0.1, ..Default::default() };
    assert!(per_pixel(&narrow, 64, 48, true, None).and_then(|s| s.texture).is_some_and(|t| t.radii.is_empty()));
    for depth in DEPTHS {
        compare_at(&mut g, &narrow, depth, (64, 48));
    }
    // A flat frame: no haze to find, no detail to raise.
    let flat = vec![[0.4, 0.35, 0.3, 1.0]; 64 * 48];
    for depth in DEPTHS {
        compare_image(&mut g, &p, depth, &flat, (64, 48));
    }
}

/// #9273: the GPU opens the dark channel like the CPU, so a sky beside a dark tower clears as it
/// does there. The scene of algo's `dehaze_leaves_no_halo_beside_dark_objects`: a clear sky and a
/// dark tower hazed with `I = J·t + A·(1 − t)`.
#[test]
fn dehaze_opens_the_dark_channel_like_the_cpu() {
    let Some(mut g) = gpu() else { return };
    let (w, h) = (600usize, 400usize);
    let tower = |x: usize, y: usize| (250..350).contains(&x) && y >= 120;
    let (t, air) = (0.55f32, [0.9f32, 0.9, 0.9]);
    let src: Vec<[f32; 4]> = (0..w * h)
        .map(|i| {
            let (x, y) = (i % w, i / w);
            let j = if tower(x, y) { [0.06, 0.06, 0.07] } else { [0.35 + 0.1 * y as f32 / h as f32, 0.5, 0.8] };
            let c: [f32; 3] = std::array::from_fn(|k| j[k] * t + air[k] * (1.0 - t));
            [c[0], c[1], c[2], 1.0]
        })
        .collect();
    for dehaze in [100.0, 60.0, -40.0] {
        for depth in DEPTHS {
            let d = compare_image(&mut g, &CameraRaw { dehaze, ..Default::default() }, depth, &src, (w, h));
            eprintln!("dehaze {dehaze} on the tower {depth:?}: max |GPU - CPU| = {d:.2e}");
        }
    }
}

#[test]
fn dehaze_remakes_its_copy_when_the_white_balance_changes() {
    let Some(mut g) = gpu() else { return };
    let src = image(858, 572, Depth::Eight);
    // Same source key throughout: the copy is cached by it and by the white balance.
    for temperature in [25.0, -40.0, -40.0, 0.0] {
        let p = CameraRaw { dehaze: 50.0, temperature, ..Default::default() };
        let mut cpu = src.clone();
        develop(&mut cpu, 858, 572, &p, false);
        let stages = per_pixel(&p, 858, 572, false, None).unwrap();
        let out = g.dev.develop(&g.device, &g.queue, None, &src, Some(3), [858, 572], &stages).unwrap();
        let d = out.iter().zip(&cpu).flat_map(|(a, b)| (0..3).map(move |c| (a[c] - b[c]).abs())).fold(0.0f32, f32::max);
        assert!(d <= TOL, "temperature {temperature}: off by {d}");
    }
}
