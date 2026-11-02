//! End-to-end timing of `image.crop`, upright and turned, on a two-layer document.
//! `cargo run --release -p photocraft-engine --example bench_crop -- [image] [reps]`
//! Without an image a 3859×2594 noise photo stands in (the size of the Great Wave scan).
use std::time::{Duration, Instant};

use photocraft_engine::Session;
use serde_json::{Value, json};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let reps: usize = args.get(2).and_then(|v| v.parse().ok()).unwrap_or(3);
    let mut s = Session::new();
    match args.get(1) {
        Some(path) => {
            let bytes = std::fs::read(path).expect("read the image");
            let doc = photocraft_io::import(path, &bytes).expect("decode the image").document;
            s.add_document(doc, None);
        }
        None => {
            s.execute("file.new", json!({"width": 3859, "height": 2594, "background": "white"})).expect("new");
            s.execute("filter.noise.addNoise", json!({"amount": 50})).expect("noise");
        }
    }
    s.execute("layer.duplicate", json!({})).expect("a second layer");
    let cases: [(&str, Value); 3] = [
        ("upright 512x442", json!({"x": 40, "y": 40, "width": 512, "height": 442})),
        ("turned 3° 512x442", json!({"x": 40, "y": 40, "width": 512, "height": 442, "angle": 3})),
        ("turned 3° 3000x2000", json!({"x": 400, "y": 300, "width": 3000, "height": 2000, "angle": 3})),
    ];
    for (label, p) in cases {
        let mut best = Duration::MAX;
        for _ in 0..reps {
            let t = Instant::now();
            s.execute("image.crop", p.clone()).expect("crop");
            best = best.min(t.elapsed());
            s.execute("edit.undo", json!({})).expect("undo");
        }
        println!("{label:24} best of {reps}: {best:.2?}");
    }
}
