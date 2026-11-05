//! 24 MP frame creation: vector commands versus the previous raster-mask construction.
//! `cargo run --release -p photocraft-engine --example bench_frame`
//! The baseline times only the former mask work; command times include history/validation.
use std::hint::black_box;
use std::time::Instant;

use photocraft_doc::LayerMask;
use photocraft_engine::Session;
use photocraft_geom::Rect;
use serde_json::json;

fn raster_mask(rect: Rect, ellipse: bool) -> LayerMask {
    let mut mask = LayerMask::hide_all();
    if !ellipse {
        mask.surface.fill_rect(rect, &[1.0]);
        return mask;
    }
    let (rx, ry) = (f64::from(rect.width()) / 2., f64::from(rect.height()) / 2.);
    let (cx, cy) = (f64::from(rect.x0) + rx, f64::from(rect.y0) + ry);
    for y in rect.y0..rect.y1 {
        for x in rect.x0..rect.x1 {
            let covered = (0..16)
                .filter(|i| {
                    let dx = (f64::from(x) + (f64::from(i % 4) + 0.5) / 4. - cx) / rx;
                    let dy = (f64::from(y) + (f64::from(i / 4) + 0.5) / 4. - cy) / ry;
                    dx * dx + dy * dy <= 1.
                })
                .count();
            if covered > 0 {
                mask.surface.write_pixel(x, y, &[covered as f32 / 16.]);
            }
        }
    }
    mask.surface.prune();
    mask
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut session = Session::new();
    session.execute("file.new", json!({"width":6000,"height":4000}))?;
    let rect = Rect::new(100, 100, 5900, 3900);
    for (shape, ellipse) in [("rectangle", false), ("ellipse", true)] {
        let mut before = f64::INFINITY;
        let mut after = f64::INFINITY;
        for _ in 0..3 {
            let start = Instant::now();
            let mask = black_box(raster_mask(rect, ellipse));
            before = before.min(start.elapsed().as_secs_f64() * 1000.);
            drop(mask);
            let start = Instant::now();
            session.execute("layer.new.frame", json!({"x":100,"y":100,"width":5800,"height":3800,"shape":shape}))?;
            after = after.min(start.elapsed().as_secs_f64() * 1000.);
            session.undo();
        }
        println!("{shape}, 6000x4000 canvas, best of 3: previous mask {before:.3} ms; vector command {after:.3} ms");
    }
    Ok(())
}
