//! Export timings on a realistic photo document (default 7360×4912 = 36 MP): the opened photo,
//! a painted layer covering part of the canvas, and a soft half-transparent layer.
//!
//! ```sh
//! cargo run --release -p photocraft-io --example bench_export -- [--size 7360x4912] [--reps 7] [--only pcraft] [--out DIR]
//! ```
//!
//! Each row is the median (and min) of `--reps` runs. `--out DIR` writes each format's bytes to
//! `DIR/out.<ext>` so two builds' output can be compared.
use std::time::Instant;

use photocraft_doc::Layer;
use photocraft_geom::Rect;

fn arg(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1).cloned())
}

fn photo(w: u32, h: u32) -> photocraft_codecs::Image {
    let mut px = vec![0u8; w as usize * h as usize * 3];
    for (y, row) in px.chunks_mut(w as usize * 3).enumerate() {
        for x in 0..w as usize {
            let (fx, fy) = (x as f32 / w as f32, y as f32 / h as f32);
            let n = ((x.wrapping_mul(73_856_093) ^ y.wrapping_mul(19_349_663)) % 997) as f32 / 997.0 - 0.5;
            let blob = (-(((fx - 0.4) * 3.0).powi(2) + ((fy - 0.55) * 4.0).powi(2))).exp();
            let r = 0.55 + 0.35 * (fx * 6.0).sin() * 0.5 + 0.3 * blob + n * 0.06;
            let g = 0.45 + 0.25 * (fy * 9.0 + fx * 2.0).cos() * 0.5 + 0.2 * blob + n * 0.06;
            let b = 0.35 + 0.4 * fy - 0.2 * blob + n * 0.06;
            for (c, v) in [r, g, b].into_iter().enumerate() {
                row[x * 3 + c] = (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
            }
        }
    }
    photocraft_codecs::Image::from_raw(w, h, photocraft_codecs::ChannelLayout::Rgb, photocraft_codecs::SampleType::U8, px).expect("image")
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let (w, h) = arg(&args, "--size").and_then(|s| s.split_once('x').and_then(|(a, b)| Some((a.parse().ok()?, b.parse().ok()?)))).unwrap_or((7360u32, 4912u32));
    let reps: usize = arg(&args, "--reps").and_then(|s| s.parse().ok()).unwrap_or(7);
    let only = arg(&args, "--only");
    let out = arg(&args, "--out");

    let png = photocraft_codecs::encode(&photo(w, h), photocraft_codecs::Format::Png, &Default::default()).expect("png");
    let mut doc = photocraft_io::import("photo.png", &png).expect("import").document;
    let fmt = doc.pixel_format();
    let mut paint = Layer::raster("paint", fmt);
    if let Some(s) = paint.surface_mut() {
        let (pw, ph) = (w / 2, h / 3);
        let data: Vec<f32> = (0..pw * ph)
            .flat_map(|i| {
                let (x, y) = (i % pw, i / pw);
                let a = if (x / 64 + y / 64) % 3 == 0 { 0.0 } else { 0.85 };
                [0.75, (x % 256) as f32 / 255.0, (y % 256) as f32 / 255.0, a]
            })
            .collect();
        s.write_region(Rect::from_xywh(w as i32 / 8, h as i32 / 4, pw, ph), &data);
    }
    doc.layers.push(paint);
    let mut soft = Layer::raster("soft", fmt);
    soft.opacity = 0.6;
    if let Some(s) = soft.surface_mut() {
        s.fill_rect(Rect::from_xywh(w as i32 / 2, 0, w / 2, h), &[0.1, 0.2, 0.9, 0.5]);
    }
    doc.layers.push(soft);

    println!("document {w}×{h}, {} layers, {reps} reps (median / min ms)", doc.layers.len());
    // "pcraft+cache" is the app's Save: one export cache per document, so the first save
    // compresses every tile and later ones reuse what fits the cache (timed after a warm-up save).
    for ext in ["psd", "pcraft", "pcraft+cache", "tif", "png", "jpg"] {
        if only.as_deref().is_some_and(|o| o != ext) {
            continue;
        }
        let cached = ext == "pcraft+cache";
        let opts = photocraft_io::ExportOptions { pcraft_cache: cached.then(photocraft_io::PcraftExportCache::default), ..Default::default() };
        let name = format!("x.{}", ext.trim_end_matches("+cache"));
        if cached {
            photocraft_io::export(&doc, &name, &opts).expect("warm-up");
        }
        let mut times = Vec::with_capacity(reps);
        let mut bytes = Vec::new();
        for _ in 0..reps {
            let t = Instant::now();
            bytes = photocraft_io::export(&doc, &name, &opts).expect("export").bytes;
            times.push(t.elapsed().as_secs_f64() * 1000.0);
        }
        times.sort_by(f64::total_cmp);
        println!("{ext:>7}: {:9.1} / {:9.1}  ({} bytes)", times[times.len() / 2], times[0], bytes.len());
        if let Some(dir) = &out {
            std::fs::write(format!("{dir}/out.{}", ext.replace('+', "-")), &bytes).expect("write");
        }
    }
}
