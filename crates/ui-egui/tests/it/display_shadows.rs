//! The on-screen GPU conversion must preserve the ICC matrix/TRC result near black.
//!
//! No photo, hardware profile or application's output is an oracle here: the generated
//! profiles and expected values implement ICC.1:2010 Annex F.3's matrix/TRC equations.
//! A uniform 33³ display LUT used to make input sRGB 1–7 noticeably too dark.

use std::sync::Arc;

use photocraft_cms::{Builtin, Profile, curve::Curve};
use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_doc::{Document, Layer, LayerContent, Size};
use photocraft_engine::display_color::Display;
use photocraft_geom::Rect;
use photocraft_raster::Surface;
use photocraft_ui_egui::{PhotocraftApp, canvas::ViewXform, gpu_canvas};

// D50-adapted sRGB and Display P3 colourants. ICC stores them as signed 15.16 values.
const SOURCE_MATRIX: [[f64; 3]; 3] =
    [[0.436_074_7, 0.385_064_9, 0.143_080_4], [0.222_504_5, 0.716_878_6, 0.060_616_9], [0.013_932_2, 0.097_104_5, 0.714_173_3]];
const MONITOR_MATRIX: [[f64; 3]; 3] = [[0.515_102, 0.291_965, 0.157_153], [0.241_182, 0.692_236, 0.066_581_9], [-0.001_049_41, 0.041_881_8, 0.784_378]];
const PATCH: u32 = 56;
const COLS: usize = 8;
const RGB: [[u8; 3]; 32] = [
    [0, 0, 0],
    [1, 1, 1],
    [2, 2, 2],
    [3, 3, 3],
    [4, 4, 4],
    [5, 5, 5],
    [7, 7, 7],
    [8, 8, 8],
    [9, 9, 9],
    [10, 10, 10],
    [11, 11, 11],
    [12, 12, 12],
    [16, 16, 16],
    [32, 32, 32],
    [128, 128, 128],
    [255, 255, 255],
    [1, 0, 0],
    [0, 1, 0],
    [0, 0, 1],
    [2, 1, 3],
    [7, 2, 1],
    [1, 7, 2],
    [2, 1, 7],
    [3, 5, 2],
    [255, 0, 0],
    [0, 255, 0],
    [0, 0, 255],
    [24, 72, 153],
    [153, 72, 24],
    [72, 153, 24],
    [254, 254, 254],
    [64, 96, 192],
];

fn fixed(v: f64) -> f64 {
    (v * 65536.0).round() / 65536.0
}

/// Literal source ICC `curv` entries, with ICC's unsigned-16 encoding.
fn source_samples() -> Vec<f64> {
    (0..1024)
        .map(|i| {
            let x = i as f64 / 1023.0;
            let y = if x <= 0.04045 { x / 12.92 } else { ((x + 0.055) / 1.055).powf(2.4) };
            (y * 65535.0).round() / 65535.0
        })
        .collect()
}

fn profiles() -> (Arc<Vec<u8>>, Arc<Vec<u8>>) {
    let mut source = Builtin::Srgb.profile().clone();
    source.description = "Synthetic sampled sRGB".into();
    source.matrix = Some(SOURCE_MATRIX.map(|row| row.map(fixed)));
    source.trc = Some(std::array::from_fn(|_| Curve::Table(source_samples().into_iter().map(|v| v as f32).collect())));
    let mut monitor = Builtin::DisplayP3.profile().clone();
    monitor.description = "Synthetic gamma 2.2 monitor".into();
    monitor.matrix = Some(MONITOR_MATRIX.map(|row| row.map(fixed)));
    monitor.trc = Some(std::array::from_fn(|_| Curve::Gamma(fixed(2.2))));
    (source.with_encoded_bytes().to_bytes(), monitor.with_encoded_bytes().to_bytes())
}

/// Solve the monitor matrix without using any photocraft-cms transform/math/curve evaluator.
fn solve(mut m: [[f64; 3]; 3], mut xyz: [f64; 3]) -> [f64; 3] {
    for column in 0..3 {
        let pivot = (column..3).max_by(|&a, &b| m[a][column].abs().total_cmp(&m[b][column].abs())).unwrap();
        m.swap(column, pivot);
        xyz.swap(column, pivot);
        let divisor = m[column][column];
        for v in &mut m[column] {
            *v /= divisor;
        }
        xyz[column] /= divisor;
        let normalized = m[column];
        for row in 0..3 {
            if row != column {
                let factor = m[row][column];
                for (value, coefficient) in m[row].iter_mut().zip(normalized) {
                    *value -= factor * coefficient;
                }
                xyz[row] -= factor * xyz[column];
            }
        }
    }
    xyz
}

fn reference(rgb: [u8; 3]) -> [u8; 3] {
    let table = source_samples();
    let linear = rgb.map(|v| {
        let pos = f64::from(v) / 255.0 * 1023.0;
        let lo = pos.floor() as usize;
        let hi = (lo + 1).min(1023);
        table[lo] + (table[hi] - table[lo]) * (pos - lo as f64)
    });
    let xyz = SOURCE_MATRIX.map(|row| (0..3).map(|c| fixed(row[c]) * linear[c]).sum());
    solve(MONITOR_MATRIX.map(|row| row.map(fixed)), xyz).map(|v| (v.clamp(0.0, 1.0).powf(1.0 / fixed(2.2)) * 255.0).round() as u8)
}

fn chart(depth: SampleType, icc: Arc<Vec<u8>>) -> Document {
    let mut doc = Document::new("ICC shadow patches", Size::new(COLS as u32 * PATCH, RGB.len() as u32 / COLS as u32 * PATCH), ColorMode::Rgb, depth);
    doc.icc_profile = Some(icc);
    let mut surface = Surface::new(PixelFormat::new(ColorMode::Rgb, depth, true));
    for (i, rgb) in RGB.iter().enumerate() {
        let x = i as u32 % COLS as u32 * PATCH;
        let y = i as u32 / COLS as u32 * PATCH;
        surface.fill_rect(
            Rect::new(x as i32, y as i32, (x + PATCH) as i32, (y + PATCH) as i32),
            &[f32::from(rgb[0]) / 255.0, f32::from(rgb[1]) / 255.0, f32::from(rgb[2]) / 255.0, 1.0],
        );
    }
    doc.layers.push(Layer::new("Patches", LayerContent::Raster(surface)));
    doc
}

#[test]
fn gpu_display_shadows_match_independent_icc_reference_at_all_depths() {
    let _gpu = photocraft_testkit::gpu_lock();
    // Catch only device discovery: a shader or application failure must fail the regression.
    let Ok(rs) =
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| egui_kittest::wgpu::create_render_state(gpu_canvas::wgpu_setup(), Default::default())))
    else {
        eprintln!("skipping: no GPU adapter");
        return;
    };
    if !gpu_canvas::supports_f16_canvas(&rs.adapter) || std::env::var("PHOTOCRAFT_CANVAS_F16").as_deref() == Ok("0") {
        eprintln!("skipping: adapter can't render/filter Rgba16Float");
        return;
    }
    let (source, monitor) = profiles();
    // Verify the synthetic fixture was encoded as intended; no production evaluator is the oracle.
    let parsed = Profile::parse(&source).expect("source fixture");
    assert_eq!(parsed.matrix, Some(SOURCE_MATRIX.map(|row| row.map(fixed))));
    let renderer = egui_kittest::wgpu::WgpuTestRenderer::from_render_state(rs);
    let mut harness =
        egui_kittest::Harness::builder().with_size(egui::vec2(1000.0, 700.0)).with_pixels_per_point(1.0).with_max_steps(64).renderer(renderer).build_eframe(
            move |cc| {
                PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
                let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
                app.set_wgpu(cc.wgpu_render_state.as_ref().expect("render state").clone());
                app.session.color.set_displays(Ok(vec![Display {
                    id: 1,
                    name: "Synthetic display".into(),
                    frame: [0.0, 0.0, 1000.0, 700.0],
                    profile_name: None,
                    icc: Some(monitor),
                }]));
                app
            },
        );
    let mut failures = Vec::new();
    for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
        harness.state_mut().session.open_document(chart(depth, source.clone()), None);
        harness.state_mut().sync_views();
        harness.run_steps(6);
        assert!(harness.state().gpu_active(), "the regression must exercise the GPU canvas");
        let xform = ViewXform::active(harness.state()).expect("canvas mapping");
        let image = harness.render().expect("render GPU canvas");
        // Optional local diagnostic artifact: entirely synthetic, never a user's document.
        if let Some(dir) = std::env::var_os("PHOTOCRAFT_DISPLAY_TEST_OUTPUT") {
            let path = std::path::PathBuf::from(dir).join(format!("display-shadows-{depth:?}.png"));
            image.save(&path).expect("save requested synthetic chart");
            eprintln!("synthetic chart: {}", path.display());
        }
        for (i, rgb) in RGB.iter().enumerate() {
            let x = (i % COLS) as f32 * PATCH as f32 + PATCH as f32 / 2.0;
            let y = (i / COLS) as f32 * PATCH as f32 + PATCH as f32 / 2.0;
            let point = xform.to_screen(x, y);
            let got = image.get_pixel(point.x.round() as u32, point.y.round() as u32).0;
            let want = reference(*rgb);
            if (0..3).any(|c| got[c].abs_diff(want[c]) > 1) {
                failures.push(format!("{depth:?} input {rgb:?}: GPU {got:?}, ICC {want:?}"));
            }
        }
    }
    assert!(failures.is_empty(), "GPU display differs by more than one RGB8 code:\n{}", failures.join("\n"));
}
/// Release-only manual evidence, not a noisy CI performance threshold. Both paths render
/// the same 24 MP texture at 100%; timings include identical framebuffer allocation/readback
/// and wait for GPU completion. Profile/texture uploads are outside the measured frames.
#[test]
#[ignore = "manual 24 MP GPU timing; run with --release --ignored --nocapture"]
fn compare_display_paths_24mp() {
    use photocraft_cms::{Intent, Lut3d, Transform};
    use photocraft_ui_egui::gpu_canvas::{GpuCanvas, ViewParams};
    use std::time::Instant;

    let _gpu = photocraft_testkit::gpu_lock();
    let rs = egui_kittest::wgpu::create_render_state(gpu_canvas::wgpu_setup(), Default::default());
    let info = rs.adapter.get_info();
    let gpu = GpuCanvas::new(&rs);
    let (source, monitor) = profiles();
    let source = Profile::parse(&source).expect("synthetic source");
    let monitor = Profile::parse(&monitor).expect("synthetic monitor");
    let transform = Transform::new(&source, &monitor, Intent::RelativeColorimetric, true).expect("display transform");
    let lut = Lut3d::from_transform(&transform, 33).to_rgba8();
    let shaper = transform.gpu_matrix_shaper().expect("matrix/shaper");

    let mut prep_lut_ms = Vec::new();
    let mut prep_shaper_ms = Vec::new();
    for _ in 0..5 {
        let start = Instant::now();
        std::hint::black_box(Lut3d::from_transform(&transform, 33).to_rgba8());
        prep_lut_ms.push(start.elapsed().as_secs_f64() * 1000.0);
        let start = Instant::now();
        std::hint::black_box(transform.gpu_matrix_shaper().expect("matrix/shaper"));
        prep_shaper_ms.push(start.elapsed().as_secs_f64() * 1000.0);
    }

    const SIZE: [u32; 2] = [6000, 4000];
    let rgba: Vec<u8> = (0..SIZE[0] as usize * SIZE[1] as usize)
        .flat_map(|i| {
            let rgb = RGB[(i / 64 + i / SIZE[0] as usize / 64) % RGB.len()];
            [rgb[0], rgb[1], rgb[2], 255]
        })
        .collect();
    gpu.upload_full(1, SIZE, &rgba);
    drop(rgba);
    gpu.set_display_lut(1, 1, 33, Some(&lut));
    gpu.set_display_matrix_shaper(1, 2, &shaper);
    assert!(gpu.has(1, SIZE), "24 MP texture uploaded");
    let renderer = egui_kittest::wgpu::WgpuTestRenderer::from_render_state(rs);
    let params = ViewParams {
        doc: 1,
        doc_size: SIZE,
        zoom: 1.0,
        center: [SIZE[0] as f32 / 2.0, SIZE[1] as f32 / 2.0],
        rotation: 0.0,
        shadow: false,
        pixel_grid: false,
        view_key: 0,
        display: 1,
        output: 1,
        hdr: None,
        offset: [0.0, 0.0],
    };
    let mut harness =
        egui_kittest::Harness::builder().with_size(egui::vec2(SIZE[0] as f32, SIZE[1] as f32)).with_pixels_per_point(1.0).renderer(renderer).build_ui_state(
            |ui, params| {
                let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(SIZE[0] as f32, SIZE[1] as f32));
                GpuCanvas::paint(&ui.painter().with_clip_rect(rect), rect, *params);
            },
            params,
        );
    let mut render_lut_ms = Vec::new();
    let mut render_shaper_ms = Vec::new();
    // Alternation reduces order/temperature bias; round zero is the warm-up for each mode.
    for round in 0..6 {
        for (display, output) in [(1, 1), (3, 2)] {
            harness.state_mut().display = display;
            harness.state_mut().output = output;
            let start = Instant::now();
            harness.run_steps(1);
            // WgpuTestRenderer waits for submission and mapped readback before returning.
            let image = harness.render().expect("render 24 MP frame");
            std::hint::black_box(&image);
            let elapsed = start.elapsed().as_secs_f64() * 1000.0;
            assert_eq!([image.width(), image.height()], SIZE);
            if round != 0 {
                if display == 1 {
                    render_lut_ms.push(elapsed);
                } else {
                    render_shaper_ms.push(elapsed);
                }
            }
        }
    }
    for (label, mut samples) in [
        ("LUT preparation", prep_lut_ms),
        ("matrix/shaper preparation", prep_shaper_ms),
        ("LUT 24 MP render + readback", render_lut_ms),
        ("matrix/shaper 24 MP render + readback", render_shaper_ms),
    ] {
        samples.sort_by(f64::total_cmp);
        eprintln!("{label}: median {:.3} ms, max {:.3} ms; samples {samples:?}", samples[2], samples[4]);
    }
    eprintln!(
        "adapter: {} ({:?}); release={}; 6000x4000 RGBA8, 100% zoom, 1 warm-up + 5 alternating measured frames per path",
        info.name,
        info.backend,
        !cfg!(debug_assertions)
    );
}
