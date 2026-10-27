//! End-to-end timing of `edit.contentAwareScale` (perf scenario P28, #211) without the UI: a
//! synthetic 24 MP RGB photo opened from JPEG like `perf_scenarios`' (gradients, soft blobs, fine
//! noise and a high-contrast disc), scaled to 90 % width, undone between runs.
//! `cargo run --release -p photocraft-engine --example bench_content_aware_scale [reps] [width height]`
use std::time::Instant;

use photocraft_engine::Session;
use serde_json::json;

fn photo_jpeg(w: u32, h: u32) -> Result<Vec<u8>, String> {
    let mut px = vec![0u8; w as usize * h as usize * 3];
    let (cx, cy, r) = (w as f32 * 0.55, h as f32 * 0.5, w.min(h) as f32 / 3.0);
    for (y, row) in px.chunks_mut(w as usize * 3).enumerate() {
        for x in 0..w as usize {
            let (fx, fy) = (x as f32 / w as f32, y as f32 / h as f32);
            let n = ((x.wrapping_mul(73_856_093) ^ y.wrapping_mul(19_349_663)) % 997) as f32 / 997.0 - 0.5;
            let blob = (-(((fx - 0.4) * 3.0).powi(2) + ((fy - 0.55) * 4.0).powi(2))).exp();
            let inside = ((x as f32 - cx).powi(2) + (y as f32 - cy).powi(2)).sqrt() < r;
            let (r0, g0, b0) = if inside { (0.85, 0.3 + 0.2 * (fx * 40.0).sin(), 0.15) } else { (0.35, 0.45, 0.55) };
            let rgb = [r0 + 0.1 * blob + n * 0.08, g0 + 0.1 * blob + n * 0.08, b0 + 0.2 * fy + n * 0.08];
            for (c, v) in rgb.into_iter().enumerate() {
                row[x * 3 + c] = (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
            }
        }
    }
    let img =
        photocraft_codecs::Image::from_raw(w, h, photocraft_codecs::ChannelLayout::Rgb, photocraft_codecs::SampleType::U8, px).map_err(|e| e.to_string())?;
    photocraft_codecs::encode(&img, photocraft_codecs::Format::Jpeg, &Default::default()).map_err(|e| e.to_string())
}

fn main() -> Result<(), String> {
    let args: Vec<String> = std::env::args().collect();
    let num = |i: usize, d: u32| args.get(i).and_then(|v| v.parse::<u32>().ok()).unwrap_or(d);
    let (reps, w, h) = (num(1, 3).clamp(1, 20) as usize, num(2, 6000).clamp(16, 20_000), num(3, 4000).clamp(16, 20_000));
    let jpeg = photo_jpeg(w, h)?;
    let doc = photocraft_io::import("photo.jpg", &jpeg).map_err(|e| e.to_string())?.document;
    let mut s = Session::new();
    s.open_document(doc, Some("photo.jpg".into()));
    let mut times = Vec::with_capacity(reps);
    for _ in 0..reps {
        let t = Instant::now();
        s.execute("edit.contentAwareScale", json!({"scaleX": 90, "scaleY": 100})).map_err(|e| e.to_string())?;
        times.push(t.elapsed().as_secs_f64() * 1000.0);
        s.execute("edit.undo", json!({})).map_err(|e| e.to_string())?;
    }
    times.sort_by(f64::total_cmp);
    let (p50, max) = (times[times.len() / 2], times[times.len() - 1]);
    println!("edit.contentAwareScale {w}x{h} to 90 % width: p50 {p50:.1} ms, max {max:.1} ms ({reps} runs)");
    Ok(())
}
