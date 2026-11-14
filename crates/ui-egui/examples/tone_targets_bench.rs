//! Completed CPU tone-preview work on a dense synthetic 24 MP document.
//! Run this same source against baseline and candidate with `--reps 9 --out results.json`.
//! Includes proxy construction, the command, and flattening; excludes GPU upload/UI latency.
use std::{error::Error, time::Instant};

use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_doc::{AlphaChannel, Document, Layer, LayerContent, LayerMask};
use photocraft_geom::{Rect, Size};
use photocraft_raster::Surface;
use serde_json::{Value, json};

/// Measure warmed proxy construction, adjustment dispatch and flattening on synthetic 24 MP targets.
fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = std::env::args().collect();
    let arg = |key: &str| args.iter().position(|v| v == key).and_then(|i| args.get(i + 1));
    let reps = arg("--reps").and_then(|s| s.parse::<usize>().ok()).unwrap_or(9).clamp(1, 100);
    let mut doc = Document::new("Synthetic tone benchmark", Size::new(6000, 4000), ColorMode::Rgb, SampleType::U8);
    let mut pixels = Surface::new(PixelFormat::new(ColorMode::Rgb, SampleType::U8, true));
    pixels.fill_rect(doc.bounds(), &[0.25, 0.25, 0.25, 1.0]);
    let mut gray = Surface::new(PixelFormat::new(ColorMode::Grayscale, SampleType::U8, false));
    gray.fill_rect(doc.bounds(), &[0.25]);
    let mut layer = Layer::new("Pixels", LayerContent::Raster(pixels));
    layer.mask = Some(LayerMask { surface: gray.clone(), ..LayerMask::reveal_all() });
    let id = layer.id;
    doc.layers.push(layer);
    doc.channels.push(AlphaChannel::new("Saved alpha", gray.clone()));
    doc.quick_mask = Some(AlphaChannel::new("Quick Mask", gray));
    let k = photocraft_ui_egui::proxy::factor(&doc);
    let mut rows = Vec::new();
    for selection in [false, true] {
        doc.selection = selection.then(|| {
            let mut s = Surface::new(PixelFormat::new(ColorMode::Grayscale, SampleType::U8, false));
            s.fill_rect(Rect::from_xywh(1500, 1000, 3000, 2000), &[0.5]);
            s
        });
        for (name, target, picker) in [
            ("composite", json!("pixels"), false),
            ("alpha", json!({"channel":0}), false),
            ("mask", json!("mask"), false),
            ("quickMask", json!("quickMask"), false),
            ("alpha_white_picker", json!({"channel":0}), true),
        ] {
            let mut samples = Vec::new();
            let mut cold = 0.0;
            for i in 0..=reps {
                let p: Value = if picker {
                    json!({"target":target,"eyedropper":{"point":"white","at":[800,600],"size":101}})
                } else {
                    json!({"target":target,"points":[[0,0],[100,125+(i%13)*3],[255,255]]})
                };
                let start = Instant::now();
                let preview = photocraft_ui_egui::filter_dialog::preview_document(&doc, Some(id), "image.adjustments.curves", &p, k).ok_or("preview failed")?;
                std::hint::black_box(photocraft_compose::flatten(&preview));
                let ms = start.elapsed().as_secs_f64() * 1000.0;
                if i == 0 {
                    cold = ms;
                } else {
                    samples.push(ms);
                }
            }
            samples.sort_by(f64::total_cmp);
            let median = samples.get(samples.len() / 2).copied().ok_or("no samples")?;
            println!("{name}, selection={selection}: {median:.2} ms (cold {cold:.2})");
            rows.push(json!({"case":name,"selection":selection,"median_ms":median,"cold_ms":cold,"samples_ms":samples}));
        }
    }
    let result = json!({"size":[6000,4000],"proxy_factor":k,"reps":reps,"method":"CPU proxy + command + flatten; dense RGB8/gray8; uniform 25% gray; half-coverage selection; no GPU upload/UI", "rows":rows});
    if let Some(path) = arg("--out") {
        std::fs::write(path, serde_json::to_vec_pretty(&result)?)?;
    }
    Ok(())
}
