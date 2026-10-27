//! 24 MP timing of Art History Brush on a representative stroke.
//! `cargo run --release -p photocraft-engine --example bench_art_history`
use std::time::Instant;

use photocraft_engine::Session;
use photocraft_geom::Rect;
use serde_json::json;

fn main() {
    let (w, h) = (6016u32, 4000u32);
    let mut s = Session::new();
    s.execute("file.new", json!({"width": w, "height": h, "depth": 8, "mode": "rgb"})).unwrap();
    s.edit("texture", |doc, active| {
        let surf = doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap();
        let fmt = surf.format();
        let row: Vec<f32> = (0..w)
            .flat_map(|x| {
                let t = (x % 256) as f32 / 255.0;
                photocraft_raster::from_rgba(&fmt, [t, 0.35, 1.0 - t, 1.0])
            })
            .collect();
        for y in 0..h as i32 {
            surf.write_region(Rect::new(0, y, w as i32, y + 1), &row);
        }
        Ok(())
    })
    .unwrap();
    s.execute("edit.fill", json!({"color": "#606060"})).unwrap();

    let points: Vec<[i32; 2]> = (0..200).map(|i| [2800 + i, 2000]).collect();
    let run = |s: &mut Session, style: &str| {
        s.execute(
            "paint.artHistoryBrush",
            json!({
                "points": points,
                "size": 40,
                "hardness": 100,
                "area": 50,
                "tolerance": 0,
                "style": style,
                "state": 0
            }),
        )
        .unwrap();
    };
    // Warm the allocator / tile cache.
    run(&mut s, "tightMedium");
    s.undo();

    for style in ["tightMedium", "looseCurl"] {
        let t = Instant::now();
        run(&mut s, style);
        let ms = t.elapsed().as_secs_f64() * 1000.0;
        println!(
            "artHistoryBrush  document {w}×{h} ({:.1} MP)  stroke 200 px  size 40  area 50  style {style}  {ms:.3} ms",
            (w as u64 * h as u64) as f64 / 1e6
        );
        s.undo();
    }
}
