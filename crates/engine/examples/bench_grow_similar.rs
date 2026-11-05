//! End-to-end timing of `select.grow` and `select.similar` on large documents.
//! `cargo run --release -p photocraft-engine --example bench_grow_similar [reps]`
use std::time::{Duration, Instant};

use photocraft_engine::Session;
use photocraft_geom::Rect;
use serde_json::json;

fn main() {
    let reps: usize = std::env::args().nth(1).and_then(|v| v.parse().ok()).unwrap_or(3);
    let mut s = Session::new();
    let (w, h) = (6000, 4000);
    s.execute("file.new", json!({"width": w, "height": h})).unwrap();

    s.edit("paint", |doc, _| {
        let bg = doc.layers[0].surface_mut().unwrap();
        bg.fill_rect(Rect::new(200, 200, 1800, 1400), &[1.0, 0.0, 0.0, 1.0]);
        bg.fill_rect(Rect::new(2200, 200, 3800, 1400), &[0.0, 0.0, 1.0, 1.0]);
        bg.fill_rect(Rect::new(200, 1600, 1800, 2800), &[0.0, 1.0, 0.0, 1.0]);
        bg.fill_rect(Rect::new(2200, 1600, 3800, 2800), &[0.0, 0.0, 0.0, 1.0]);
        Ok(())
    })
    .unwrap();

    let cases = [
        ("grow contiguous 1000x800", "select.grow", json!({"tolerance": 10}), vec![json!({"x": 500, "y": 500, "width": 500, "height": 400})]),
        ("grow large selection 1600x1200", "select.grow", json!({"tolerance": 10}), vec![json!({"x": 200, "y": 200, "width": 1600, "height": 1200})]),
        ("similar single color", "select.similar", json!({"tolerance": 10}), vec![json!({"x": 500, "y": 500, "width": 100, "height": 100})]),
        (
            "similar dual color (red + blue)",
            "select.similar",
            json!({"tolerance": 10}),
            vec![json!({"x": 500, "y": 500, "width": 100, "height": 100}), json!({"x": 2500, "y": 500, "width": 100, "height": 100, "mode": "add"})],
        ),
    ];

    for (label, cmd, params, initial_selections) in cases {
        let mut best = Duration::MAX;
        for _ in 0..reps {
            s.execute("select.deselect", json!({})).ok();
            for init in &initial_selections {
                s.execute("select.rect", init.clone()).unwrap();
            }
            let t = Instant::now();
            s.execute(cmd, params.clone()).unwrap();
            best = best.min(t.elapsed());
        }
        println!("{label:36} best of {reps}: {best:.2?}");
    }
}
