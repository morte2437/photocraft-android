//! Release benchmark harness measuring cold rasterization, warm movement lookup, and painted fallback
//! baseline for native Pen tool cursors (#9122).
//!
//! Evaluates all six standard Pen tool cursor variants across multiple display scales, measuring:
//! 1. Cold rasterization latency and bitmap dimensions/hotspots.
//! 2. Warm-cache lookup latency during steady pointer movements across frames.
//! 3. Painted fallback baseline latency using identical pointer moves and harness.
//! 4. Structural gate: zero additional rasterizations during steady warm movement with unchanged appearance.
//! 5. Cost gate: warm-path p95 below 0.1 ms (100 µs).
//! 6. Optional artifact export of cursor images as PNG files for visual regression inspection.
//!
//! ```sh
//! cargo run --release -p photocraft-ui-egui --example cursor_bench -- [--moves 1000] [--reps 5] [--save-artifacts <dir>] [--json [path]]
//! ```

#[path = "cursor_bench/args.rs"]
mod args;
#[path = "cursor_bench/stats.rs"]
mod stats;
use args::checked_sample_count;
use stats::percentile;

use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use base64::Engine as _;
use egui::{Context, pos2};
use photocraft_codecs::{ChannelLayout, EncodeOptions, Format, Image, SampleType, encode};
use photocraft_testkit::gpu_lock;
use photocraft_ui_egui::canvas::tool_cursor::{PenVariant, native_scale, pen, pen_rasterization_count, rasterize_pen, reset_pen_rasterization_count};
use photocraft_ui_egui::icons;
use photocraft_ui_egui::state::Tool;

#[derive(Clone, Debug, serde::Serialize)]
struct BenchmarkRun {
    variant: String,
    tool: String,
    scale: f32,
    measured_native_scale: f32,
    dimensions: [u16; 2],
    hotspot: [u16; 2],
    retained_bytes: usize,
    cold_raster_us_samples: Vec<f64>,
    cold_raster_p50_us: f64,
    cold_raster_p95_us: f64,
    warm_move_us_samples: Vec<f64>,
    warm_move_p50_us: f64,
    warm_move_p95_us: f64,
    baseline_painted_us_samples: Vec<f64>,
    baseline_painted_p50_us: f64,
    baseline_painted_p95_us: f64,
    preparation_speedup_vs_baseline_p50: f64,
    warm_moves_total: usize,
    cache_hits: usize,
    warm_rasterizations: usize,
    structural_gate_pass: bool,
    cost_gate_pass: bool,
}

#[derive(Clone, Debug, serde::Serialize)]
struct PerfReportRow {
    name: String,
    samples_ms: Vec<f64>,
    p50_ms: f64,
    p95_ms: f64,
}

#[derive(Clone, Debug, serde::Serialize)]
struct CursorVisualComparison {
    variant: String,
    scale: f32,
    background: String,
    dimensions: [u32; 2],
    hotspot: [u32; 2],
    mae_rgb: f64,
    max_abs_diff_rgb: u8,
    rmse_rgb: f64,
    exact_match_pixels: usize,
    total_pixels: usize,
    match_percentage: f64,
    pixels_diff_gt_2: usize,
    pixels_diff_gt_5: usize,
    pixels_diff_gt_15: usize,
}

#[derive(Clone, Debug, serde::Serialize)]
struct PerfReport {
    context: serde_json::Value,
    rows: Vec<PerfReportRow>,
    results: Vec<BenchmarkRun>,
    visual_comparisons: Vec<CursorVisualComparison>,
    #[serde(skip_serializing_if = "Option::is_none")]
    contact_sheet_png_base64: Option<String>,
}

/// Map each benchmark variant to the tool used by the production dispatcher.
fn tool_for_variant(variant: PenVariant) -> Tool {
    match variant {
        PenVariant::Standard => Tool::Pen,
        PenVariant::Freeform => Tool::FreeformPen,
        PenVariant::Curvature => Tool::CurvaturePen,
        PenVariant::AddAnchor => Tool::AddAnchorPoint,
        PenVariant::DeleteAnchor => Tool::DeleteAnchorPoint,
        PenVariant::ConvertPoint => Tool::ConvertPoint,
    }
}

/// Read a named benchmark argument without indexing beyond the argument list.
fn arg_value(args: &[String], flag: &str) -> Option<String> {
    args.iter().position(|a| a == flag).and_then(|i| args.get(i + 1).cloned())
}

/// Export the native cursor bytes as an inspectable PNG.
fn save_cursor_artifact(output_dir: &Path, variant_name: &str, scale: f32, image: &egui::CustomCursorImage) -> Result<(), Box<dyn std::error::Error>> {
    std::fs::create_dir_all(output_dir)?;
    let [w, h] = image.size;
    let raw_bytes = image.rgba.to_vec();
    let img = Image::from_raw(u32::from(w), u32::from(h), ChannelLayout::Rgba, SampleType::U8, raw_bytes)?;
    let encoded = encode(&img, Format::Png, &EncodeOptions::default())?;
    let filename = format!("cursor_pen_{variant_name}_{scale:.2}x.png");
    let file_path = output_dir.join(filename);
    std::fs::write(&file_path, encoded)?;
    Ok(())
}

/// Execute one frame and time only cursor preparation, excluding GPU/presentation latency.
/// Drain frame output so painted geometry cannot accumulate between samples.
fn cursor_frame(ctx: &Context, tool: Tool, variant: PenVariant, at: egui::Pos2, painted: bool) -> (f64, Option<egui::CustomCursorImage>) {
    let mut elapsed_us = 0.0;
    let input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(512.0, 512.0))),
        events: vec![egui::Event::PointerMoved(at)],
        ..Default::default()
    };
    let output = ctx.run_ui(input, |ui| {
        let ctx = ui.ctx();
        ctx.set_cursor_image(None);
        let start = Instant::now();
        if painted {
            icons::cursor(ctx, variant.icon_name(), at, variant.hotspot_in_box() / 24.0, 22.0);
        } else if let Some(icon) = pen(ctx, tool) {
            ctx.set_cursor_icon(icon);
        }
        elapsed_us = start.elapsed().as_secs_f64() * 1_000_000.0;
    });
    (elapsed_us, output.platform_output.cursor_image)
}

/// Render each painted/native pair at the same hotspot and export comparison evidence.
fn run_visual_oracle(save_dir: Option<&Path>) -> Result<(Vec<CursorVisualComparison>, String), Box<dyn std::error::Error>> {
    let _gpu = gpu_lock();
    let backgrounds = [("light_240", 240u8), ("midgrey_128", 128), ("dark_15", 15)];
    let mut comparisons = Vec::new();
    let mut sheets = Vec::new();
    for scale in [1.0f32, 1.25, 1.5, 2.0] {
        // The production fallback uses one fixed Area id. Render only one pair per
        // frame, reusing the harness across variants/backgrounds to avoid id collisions.
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(128.0, 64.0))
            .with_pixels_per_point(scale)
            .with_context_setup(egui_extras::install_image_loaders)
            .wgpu()
            .build_ui_state(
                |ui, state: &mut (PenVariant, u8, Option<egui::TextureHandle>)| {
                    let variant = state.0;
                    let background = state.1;
                    ui.painter().rect_filled(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(128.0, 64.0)), 0.0, egui::Color32::from_gray(background));
                    ui.painter().text(
                        pos2(2.0, 2.0),
                        egui::Align2::LEFT_TOP,
                        format!("{} {scale}x: old / new", variant.icon_name()),
                        egui::FontId::monospace(6.0),
                        egui::Color32::from_gray(if background < 100 { 210 } else { 30 }),
                    );
                    icons::cursor(ui.ctx(), variant.icon_name(), pos2(32.0, 32.0), variant.hotspot_in_box() / 24.0, 22.0);
                    if let Some(bitmap) = rasterize_pen(variant, scale) {
                        let [w, h] = bitmap.size;
                        let [hx, hy] = bitmap.hotspot;
                        let image = egui::ColorImage::from_rgba_unmultiplied([usize::from(w), usize::from(h)], &bitmap.rgba);
                        let texture = state.2.get_or_insert_with(|| ui.ctx().load_texture("native-pen-reference", image, egui::TextureOptions::NEAREST));
                        let rect = egui::Rect::from_min_size(
                            pos2(96.0, 32.0) - egui::vec2(f32::from(hx), f32::from(hy)) / scale,
                            egui::vec2(f32::from(w), f32::from(h)) / scale,
                        );
                        ui.painter().image(texture.id(), rect, egui::Rect::from_min_max(egui::Pos2::ZERO, pos2(1.0, 1.0)), egui::Color32::WHITE);
                    }
                },
                (PenVariant::Standard, 240u8, None),
            );
        let cell_w = (128.0 * scale).round() as usize;
        let cell_h = (64.0 * scale).round() as usize;
        let sheet_w = cell_w * 6;
        let sheet_h = cell_h * 3;
        let mut sheet = vec![255u8; sheet_w * sheet_h * 4];
        for (bg_index, (background_name, background)) in backgrounds.iter().enumerate() {
            for (variant_index, variant) in PenVariant::all().into_iter().enumerate() {
                *harness.state_mut() = (variant, *background, None);
                harness.run_steps(3);
                let rendered = harness.render().map_err(|e| format!("cursor reference render: {e}"))?;
                if rendered.width() as usize != cell_w || rendered.height() as usize != cell_h {
                    return Err("cursor reference dimensions differ from requested display scale".into());
                }
                let raw = rendered.as_raw();
                for row in 0..cell_h {
                    let src = row * cell_w * 4;
                    let dst = ((bg_index * cell_h + row) * sheet_w + variant_index * cell_w) * 4;
                    sheet[dst..dst + cell_w * 4].copy_from_slice(&raw[src..src + cell_w * 4]);
                }
                let bitmap = rasterize_pen(variant, scale).ok_or("missing comparison bitmap")?;
                let [w, h] = bitmap.size;
                let [hx, hy] = bitmap.hotspot;
                let old_x = (32.0 * scale).round() as usize - usize::from(hx);
                let new_x = (96.0 * scale).round() as usize - usize::from(hx);
                let top = (32.0 * scale).round() as usize - usize::from(hy);
                let mut sum = 0.0f64;
                let mut squared = 0.0f64;
                let mut maximum = 0;
                let mut exact = 0;
                let mut gt2 = 0;
                let mut gt5 = 0;
                let mut gt15 = 0;
                let mut old_ink = 0;
                let mut new_ink = 0;
                for y in 0..usize::from(h) {
                    for x in 0..usize::from(w) {
                        let a = ((top + y) * cell_w + old_x + x) * 4;
                        let b = ((top + y) * cell_w + new_x + x) * 4;
                        let old = raw.get(a..a + 3).ok_or("painted crop outside render")?;
                        let new = raw.get(b..b + 3).ok_or("native crop outside render")?;
                        old_ink += usize::from(old.iter().any(|c| c.abs_diff(*background) > 10));
                        new_ink += usize::from(new.iter().any(|c| c.abs_diff(*background) > 10));
                        let mut pixel_max = 0;
                        for (a, b) in old.iter().zip(new) {
                            let d = a.abs_diff(*b);
                            pixel_max = pixel_max.max(d);
                            sum += f64::from(d) / 3.0;
                            squared += f64::from(d).powi(2) / 3.0;
                        }
                        maximum = maximum.max(pixel_max);
                        exact += usize::from(pixel_max == 0);
                        gt2 += usize::from(pixel_max > 2);
                        gt5 += usize::from(pixel_max > 5);
                        gt15 += usize::from(pixel_max > 15);
                    }
                }
                if old_ink < 10 || new_ink < 10 {
                    return Err(format!("cursor reference missing visible ink: {variant:?} {scale}x {background_name}, old={old_ink} new={new_ink}").into());
                }
                let total = usize::from(w) * usize::from(h);
                comparisons.push(CursorVisualComparison {
                    variant: variant.icon_name().to_owned(),
                    scale,
                    background: (*background_name).to_owned(),
                    dimensions: [u32::from(w), u32::from(h)],
                    hotspot: [u32::from(hx), u32::from(hy)],
                    mae_rgb: sum / total as f64,
                    max_abs_diff_rgb: maximum,
                    rmse_rgb: (squared / total as f64).sqrt(),
                    exact_match_pixels: exact,
                    total_pixels: total,
                    match_percentage: exact as f64 / total as f64 * 100.0,
                    pixels_diff_gt_2: gt2,
                    pixels_diff_gt_5: gt5,
                    pixels_diff_gt_15: gt15,
                });
            }
        }
        sheets.push((sheet, sheet_w, sheet_h));
    }
    let width = sheets.iter().map(|(_, w, _)| *w).max().ok_or("no cursor renders")?;
    let height: usize = sheets.iter().map(|(_, _, h)| h).sum();
    let mut contact = vec![255u8; width * height * 4];
    let mut top = 0;
    for (pixels, w, h) in sheets {
        for row in 0..h {
            let src = row * w * 4;
            let dst = (top + row) * width * 4;
            contact[dst..dst + w * 4].copy_from_slice(&pixels[src..src + w * 4]);
        }
        top += h;
    }
    let image = Image::from_raw(width as u32, height as u32, ChannelLayout::Rgba, SampleType::U8, contact)?;
    let png = encode(&image, Format::Png, &EncodeOptions::default())?;
    if let Some(dir) = save_dir {
        std::fs::create_dir_all(dir)?;
        std::fs::write(dir.join("pen_cursor_contact_sheet.png"), &png)?;
    }
    Ok((comparisons, base64::engine::general_purpose::STANDARD.encode(png)))
}

/// Measure bounded cursor-preparation samples and export a rendered reference outside timing.
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let moves: usize = arg_value(&args, "--moves").and_then(|v| v.parse().ok()).unwrap_or(1000);
    let reps: usize = arg_value(&args, "--reps").or_else(|| arg_value(&args, "--pairs")).and_then(|v| v.parse().ok()).unwrap_or(5);
    let save_dir = arg_value(&args, "--save-artifacts");

    let total_moves = checked_sample_count(moves, reps)?;

    // Check for --json flag and optional path argument
    let json_pos = args.iter().position(|a| a == "--json");
    let json_file_path = json_pos.and_then(|i| args.get(i + 1).filter(|val| !val.starts_with("--")).cloned());
    let json_output = json_pos.is_some();

    let scales = [1.0f32, 1.25, 1.5, 2.0];
    let variants = PenVariant::all();

    // Warmup cycle to stabilize caches and CPU frequency scaling
    for variant in variants {
        for &scale in &scales {
            let _ = rasterize_pen(variant, scale);
        }
    }

    let mut results: Vec<BenchmarkRun> = Vec::new();
    let mut perf_rows: Vec<PerfReportRow> = Vec::new();
    let mut all_gates_passed = true;

    for variant in variants {
        let tool = tool_for_variant(variant);
        let variant_name = variant.icon_name();

        for &scale in &scales {
            // Context setup with native scale applied on all platforms
            let ctx = Context::default();
            egui_extras::install_image_loaders(&ctx);
            ctx.set_zoom_factor(scale);
            let at = pos2(100.0, 100.0);
            // Apply pending zoom during begin_pass before reading effective scale.
            let _ = cursor_frame(&ctx, tool, variant, at, false);
            let measured_scale = native_scale(&ctx);
            if (measured_scale - scale).abs() > 1e-4 {
                return Err(format!("Context scale mismatch for {scale}: measured {measured_scale}").into());
            }

            // 1. Cold rasterization measurements across repetitions
            let mut cold_samples = Vec::with_capacity(reps);
            let mut sample_image = None;
            for _ in 0..reps {
                let start_cold = Instant::now();
                let img = rasterize_pen(variant, scale).ok_or_else(|| format!("rasterize_pen failed for {variant:?} at {scale}"))?;
                let cold_us = start_cold.elapsed().as_secs_f64() * 1_000_000.0;
                cold_samples.push(cold_us);
                sample_image = Some(img);
            }
            let img = sample_image.ok_or("missing rasterized image")?;
            let [w, h] = img.size;
            let [hx, hy] = img.hotspot;
            if w == 0 || h == 0 || hx >= w || hy >= h {
                return Err(format!("invalid bitmap geometry for {variant:?} at {scale}: size {w}x{h}, hotspot {hx},{hy}").into());
            }

            // Verify expected dimensions at native scale
            let expected_pad = (scale + 2.0).ceil() as usize;
            let icon_px = (22.0 * scale).round() as usize;
            let expected_side = icon_px + expected_pad * 2;
            if usize::from(w) != expected_side || usize::from(h) != expected_side {
                return Err(format!("unexpected dimensions for {variant:?} at scale {scale}: got {w}x{h}, expected {expected_side}x{expected_side}").into());
            }

            // 2. Warm movement / cache lookup measurement
            // Prime both paths, with actual frames and drained platform/paint output.
            let (_, initial_image) = cursor_frame(&ctx, tool, variant, at, false);
            let initial_image = initial_image.ok_or("missing native cursor in warmup frame")?;
            if initial_image.size != img.size {
                return Err("warm native bitmap does not match requested scale".into());
            }
            let _ = cursor_frame(&ctx, tool, variant, at, true);
            reset_pen_rasterization_count();
            let mut warm_samples = Vec::with_capacity(total_moves);
            let mut baseline_samples = Vec::with_capacity(total_moves);
            let mut cache_hits = 0;
            // Alternate execution order by repetition; coordinates match for both paths.
            for rep in 0..reps {
                for step in 0..moves {
                    let p = pos2(100.0 + (step % 800) as f32 * 0.25, 100.0 + (step as f32 * 0.05).sin() * 40.0);
                    let mut native_image = None;
                    for painted in if rep % 2 == 0 { [false, true] } else { [true, false] } {
                        let (elapsed, image) = cursor_frame(&ctx, tool, variant, p, painted);
                        if painted {
                            baseline_samples.push(elapsed);
                        } else {
                            warm_samples.push(elapsed);
                            native_image = image;
                        }
                    }
                    let current = native_image.ok_or("native cursor missing from frame output")?;
                    if Arc::ptr_eq(&current.rgba, &initial_image.rgba) {
                        cache_hits += 1;
                    }
                }
            }
            let warm_rasterizations = pen_rasterization_count();

            let mut sorted_cold = cold_samples.clone();
            sorted_cold.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            let cold_p50 = percentile(&sorted_cold, 0.50);
            let cold_p95 = percentile(&sorted_cold, 0.95);

            let mut sorted_warm = warm_samples.clone();
            sorted_warm.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            let warm_p50 = percentile(&sorted_warm, 0.50);
            let warm_p95 = percentile(&sorted_warm, 0.95);

            let mut sorted_base = baseline_samples.clone();
            sorted_base.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            let base_p50 = percentile(&sorted_base, 0.50);
            let base_p95 = percentile(&sorted_base, 0.95);

            let speedup = if warm_p50 > 0.0 { base_p50 / warm_p50 } else { 1.0 };

            // Structural gate: zero additional rasterizations during warm movement, 100% cache hits
            let structural_gate_pass = warm_rasterizations == 0 && cache_hits == total_moves;

            // Cost gate: warm-path p95 < 0.1 ms (100 µs)
            let cost_gate_pass = warm_p95 < 100.0;

            if !structural_gate_pass || !cost_gate_pass {
                all_gates_passed = false;
            }

            if let Some(ref dir_path) = save_dir {
                save_cursor_artifact(Path::new(dir_path), variant_name, scale, &img)?;
            }

            perf_rows.push(PerfReportRow {
                name: format!("pen_{variant_name}_{scale:.2}x"),
                samples_ms: warm_samples.iter().map(|us| us / 1000.0).collect(),
                p50_ms: warm_p50 / 1000.0,
                p95_ms: warm_p95 / 1000.0,
            });
            results.push(BenchmarkRun {
                variant: variant_name.to_string(),
                tool: format!("{tool:?}"),
                scale,
                measured_native_scale: measured_scale,
                dimensions: [w, h],
                hotspot: [hx, hy],
                retained_bytes: img.rgba.len(),
                cold_raster_us_samples: cold_samples,
                cold_raster_p50_us: cold_p50,
                cold_raster_p95_us: cold_p95,
                warm_move_us_samples: warm_samples,
                warm_move_p50_us: warm_p50,
                warm_move_p95_us: warm_p95,
                baseline_painted_us_samples: baseline_samples,
                baseline_painted_p50_us: base_p50,
                baseline_painted_p95_us: base_p95,
                preparation_speedup_vs_baseline_p50: speedup,
                warm_moves_total: total_moves,
                cache_hits,
                warm_rasterizations,
                structural_gate_pass,
                cost_gate_pass,
            });
        }
    }

    // Run visual consistency oracle and render contact sheet outside timing samples
    let (visual_comparisons, contact_sheet_b64) = run_visual_oracle(save_dir.as_deref().map(Path::new))?;

    if json_output {
        let perf_report = PerfReport {
            context: serde_json::json!({
                "bench": "cursor_bench",
                "moves": moves,
                "reps": reps,
                "all_gates_passed": all_gates_passed,
            }),
            rows: perf_rows,
            results,
            visual_comparisons,
            contact_sheet_png_base64: Some(contact_sheet_b64),
        };
        if let Some(path) = json_file_path {
            let file = std::fs::File::create(path)?;
            let mut writer = std::io::BufWriter::new(file);
            serde_json::to_writer_pretty(&mut writer, &perf_report)?;
            std::io::Write::flush(&mut writer)?;
        } else {
            let stdout = std::io::stdout();
            let mut writer = stdout.lock();
            serde_json::to_writer_pretty(&mut writer, &perf_report)?;
            std::io::Write::write_all(&mut writer, b"\n")?;
        }
    } else {
        println!("=================================================================================================================================");
        println!("PhotoCraft Issue #9122 Native Pen Cursor Benchmark Results");
        println!("Harness: {} pointer moves across {} repetitions per scale/variant", moves, reps);
        println!("=================================================================================================================================");
        println!(
            "{:<18} {:<6} {:<10} {:<10} {:<12} {:<12} {:<12} {:<14} {:<10} {:<8}",
            "Variant", "Scale", "Size (px)", "Hotspot", "Cold p50 (µs)", "Warm p50 (µs)", "Warm p95 (µs)", "Baseline p50", "Speedup", "Gates"
        );
        println!("---------------------------------------------------------------------------------------------------------------------------------");
        for r in &results {
            let gate_str = if r.structural_gate_pass && r.cost_gate_pass { "PASS" } else { "FAIL" };
            println!(
                "{:<18} {:<6.2} {:<10} {:<10} {:<12.1} {:<12.2} {:<12.2} {:<14.2} {:<9.1}x {:<8}",
                r.variant,
                r.scale,
                format!("{}x{}", r.dimensions[0], r.dimensions[1]),
                format!("{},{}", r.hotspot[0], r.hotspot[1]),
                r.cold_raster_p50_us,
                r.warm_move_p50_us,
                r.warm_move_p95_us,
                r.baseline_painted_p50_us,
                r.preparation_speedup_vs_baseline_p50,
                gate_str
            );
        }
        println!("=================================================================================================================================");
        println!("Measured Truth Gates Summary:");
        let total_warm_moves: usize = results.iter().map(|r| r.warm_moves_total).sum();
        let total_hits: usize = results.iter().map(|r| r.cache_hits).sum();
        let total_rasterizations: usize = results.iter().map(|r| r.warm_rasterizations).sum();
        println!(
            "- Structural gate (0 warm rasterizations, 100% cache hits): {} ({}/{} moves hit cache, {} re-rasterizations)",
            if results.iter().all(|r| r.structural_gate_pass) { "PASS" } else { "FAIL" },
            total_hits,
            total_warm_moves,
            total_rasterizations
        );
        println!("- Cost gate (warm-path p95 < 0.1 ms / 100 µs): {}", if results.iter().all(|r| r.cost_gate_pass) { "PASS" } else { "FAIL" });
        if let Some(ref dir_path) = save_dir {
            println!("- Cursor PNG artifacts saved to: {dir_path}");
        }
        if !visual_comparisons.is_empty() {
            println!("---------------------------------------------------------------------------------------------------------------------------------");
            println!("Visual Consistency Reference (Painted Baseline vs Native Bitmap 1:1 on wgpu):");
            println!(
                "{:<18} {:<6} {:<12} {:<10} {:<10} {:<10} {:<10} {:<10} {:<10}",
                "Variant", "Scale", "Background", "Size (px)", "Hotspot", "MAE (RGB)", "Max Diff", "RMSE", "Match %"
            );
            println!("---------------------------------------------------------------------------------------------------------------------------------");
            for vc in &visual_comparisons {
                println!(
                    "{:<18} {:<6.2} {:<12} {:<10} {:<10} {:<10.2} {:<10} {:<10.2} {:<9.1}%",
                    vc.variant,
                    vc.scale,
                    vc.background,
                    format!("{}x{}", vc.dimensions[0], vc.dimensions[1]),
                    format!("{},{}", vc.hotspot[0], vc.hotspot[1]),
                    vc.mae_rgb,
                    vc.max_abs_diff_rgb,
                    vc.rmse_rgb,
                    vc.match_percentage
                );
            }
        }
        println!("=================================================================================================================================");
    }

    if !all_gates_passed {
        return Err("one or more benchmark gates failed".into());
    }

    Ok(())
}
