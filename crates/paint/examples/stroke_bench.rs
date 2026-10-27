//! A long stroke of one tip, timed per dab: a zigzag over a 4928×3280 RGBA8 document that sweeps
//! the whole page, `push` then `composite` after every point, as painting does per frame. Small
//! tips are the everyday case; a long stroke grows the stroke's tile map to thousands of tiles,
//! and the time per dab must not grow with it.
//! `cargo run --release -p photocraft-paint --example stroke_bench -- [size] [hardness] [dabs]`
use photocraft_color::PixelFormat;
use photocraft_geom::Rect;
use photocraft_paint::{BrushSettings, StrokePoint, StrokeRenderer};
use photocraft_raster::Surface;
use std::time::Instant;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let size: f32 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(20.0);
    let hardness: f32 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(0.0);
    let dabs: usize = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(6000);
    let (w, h) = (4928.0f64, 3280.0f64);
    let brush = BrushSettings { size, hardness, spacing: 0.25, color: [1.0, 0.0, 0.0, 1.0], ..Default::default() };
    let fmt = PixelFormat::RGBA8;
    let mut pre = Surface::new(fmt);
    pre.fill_rect(Rect::new(0, 0, w as i32, h as i32), &[0.5, 0.5, 0.5, 1.0]);
    let mut target = pre.clone();
    // Segments from the top margin to the bottom one and back, advancing across the width so that
    // `dabs` dabs at the brush's spacing sweep the document once.
    let step = f64::from(size) * 0.25;
    let margin = f64::from(size);
    let span = h - 2.0 * margin;
    let segments = ((dabs as f64 * step) / span).ceil().max(1.0) as usize;
    let dx = (w - 2.0 * margin) / segments as f64;
    let mut r = StrokeRenderer::new(&brush, Some(fmt), 1.0);
    let (mut t_push, mut t_comp) = (0.0f64, 0.0f64);
    let t_all = Instant::now();
    let mut pos = (margin, margin);
    r.push(&[StrokePoint::new(pos.0, pos.1, 1.0)]);
    let mut pushes = 0usize;
    'stroke: for seg in 0..segments {
        let (x0, y0) = pos;
        let (x1, y1) = (margin + (seg + 1) as f64 * dx, if seg % 2 == 0 { h - margin } else { margin });
        let n = (((x1 - x0).powi(2) + (y1 - y0).powi(2)).sqrt() / step).ceil().max(1.0) as usize;
        for i in 1..=n {
            let f = i as f64 / n as f64;
            let t = Instant::now();
            r.push(&[StrokePoint::new(x0 + (x1 - x0) * f, y0 + (y1 - y0) * f, 1.0)]);
            let tp = t.elapsed().as_secs_f64();
            r.composite(&pre, &mut target, None, false, false);
            t_comp += t.elapsed().as_secs_f64() - tp;
            t_push += tp;
            pushes += 1;
            if r.dab_count() >= dabs {
                break 'stroke;
            }
        }
        pos = (x1, y1);
    }
    let total = t_all.elapsed().as_secs_f64();
    let placed = r.dab_count();
    println!(
        "tip {size} px, hardness {hardness}: {placed} dabs in {pushes} pushes over {segments} segments: {:.0} ms (push {:.0}, composite {:.0}), {:.1} µs per dab",
        total * 1e3,
        t_push * 1e3,
        t_comp * 1e3,
        total * 1e6 / placed.max(1) as f64
    );
}
