//! Where a large brush's time goes: one dab of a 2000 px tip, by stage.
//! `cargo run --release -p photocraft-paint --example big_dab_bench [size] [hardness] [angle]`
use photocraft_color::PixelFormat;
use photocraft_geom::Rect;
use photocraft_paint::{BrushContext, BrushSettings, Dab, StrokePoint, StrokeRenderer};
use photocraft_raster::Surface;
use std::time::Instant;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let size: f32 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(2000.0);
    let hardness: f32 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(0.0);
    let angle: f32 = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(0.0);
    let brush = BrushSettings { size, hardness, angle, spacing: 0.25, color: [1.0, 0.0, 0.0, 1.0], ..Default::default() };
    let fmt = PixelFormat::RGBA8;
    let mut pre = Surface::new(fmt);
    pre.fill_rect(Rect::new(0, 0, 6000, 4000), &[0.5, 0.5, 0.5, 1.0]);
    let mut target = pre.clone();
    // Stage 1: the tip's coverage alone.
    let ctx = BrushContext::new(&brush);
    let mut dab = Dab::round(photocraft_geom::Point::new(3000.0, 2000.0), size / 2.0, 1.0);
    dab.angle = angle;
    let rect = ctx.dab_rect(&dab, false);
    let mut cov = Vec::new();
    // Warm the reusable coverage buffer and take the median of seven rasterizations.
    ctx.rasterize(&dab, false, rect, &mut cov);
    let mut samples = Vec::new();
    for _ in 0..7 {
        let t = Instant::now();
        ctx.rasterize(std::hint::black_box(&dab), false, rect, &mut cov);
        std::hint::black_box(&cov);
        samples.push(t.elapsed());
    }
    samples.sort();
    let raster = samples.get(3).copied().unwrap_or_default();
    println!(
        "tip {size} px, hardness {hardness}, angle {angle} (24 MP document): dab rect {}x{} = {:.1} MP",
        rect.width(),
        rect.height(),
        rect.width() as f64 * rect.height() as f64 / 1e6
    );
    println!("  rasterize (coverage):      {:>7.3} ms", raster.as_secs_f64() * 1e3);
    // Stage 2+3: push through the renderer (rasterize + accumulate), then composite.
    let mut r = StrokeRenderer::new(&brush, Some(fmt), 1.0);
    let t = Instant::now();
    // Two points a pixel apart: the generator places the first dab at once (a lone point waits).
    r.push(&[StrokePoint::new(3000.0, 2000.0, 1.0), StrokePoint::new(3001.0, 2000.0, 1.0)]);
    let push = t.elapsed();
    println!("  push (rasterize+accumulate): {:>5.1} ms  (accumulate ≈ {:.1} ms)", push.as_secs_f64() * 1e3, push.saturating_sub(raster).as_secs_f64() * 1e3);
    let t = Instant::now();
    let dmg = r.composite(&pre, &mut target, None, false, false);
    let comp = t.elapsed();
    println!("  composite (dirty tiles):   {:>7.1} ms  over {}x{}", comp.as_secs_f64() * 1e3, dmg.width(), dmg.height());
    // A second dab half a spacing away, as a stroke does: the same again.
    let t = Instant::now();
    r.push(&[StrokePoint::new(3000.0 + f64::from(size) * 0.25, 2000.0, 1.0), StrokePoint::new(3002.0 + f64::from(size) * 0.25, 2000.0, 1.0)]);
    let dmg = r.composite(&pre, &mut target, None, false, false);
    println!("  next dab push+composite:   {:>7.1} ms  over {}x{}", t.elapsed().as_secs_f64() * 1e3, dmg.width(), dmg.height());
}
