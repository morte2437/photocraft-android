//! Release benchmark for the live Mixer Brush on a 24 MP document.
//! Setup and checksum are outside the timed command. Run with
//! `cargo run --release -p photocraft-engine --example bench_dry_mixer`.

use std::time::Instant;

use photocraft_engine::{EngineError, Session};
use photocraft_geom::Rect;
use serde_json::json;

const W: i32 = 6000;
const H: i32 = 4000;
const SIZES: [u32; 3] = [32, 128, 512];
const REPEATS: usize = 4;

fn fingerprint(session: &Session) -> Result<String, Box<dyn std::error::Error>> {
    let state = session.active().ok_or("no active document")?;
    let id = state.active_layer.ok_or("no active layer")?;
    let surface = state.doc.layer(id).and_then(|layer| layer.surface()).ok_or("no raster surface")?;
    let pixels = surface.read_region(Rect::new(0, 0, W, H));
    let mut hash = blake3::Hasher::new();
    for channel in pixels {
        hash.update(&channel.to_le_bytes());
    }
    hash.update(&serde_json::to_vec(&session.tools.mixer)?);
    Ok(hash.finalize().to_hex().to_string())
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    for wet in [0, 100] {
        for sample_all in [false, true] {
            for size in SIZES {
                let mut session = Session::new();
                session.execute("file.new", json!({"width": W, "height": H, "depth": 8, "mode": "rgb"}))?;
                session.execute("layer.new.layer", json!({}))?;
                session.edit("benchmark source", |doc, active| {
                    let id = active.ok_or_else(|| EngineError::Other("no source layer".into()))?;
                    let surface = doc.layer_mut(id).and_then(|layer| layer.surface_mut()).ok_or_else(|| EngineError::Other("no source pixels".into()))?;
                    for x in (0..W).step_by(64) {
                        let color = if x % 128 == 0 { [0.9, 0.25, 0.08, 1.0] } else { [0.08, 0.35, 0.9, 1.0] };
                        surface.fill_rect(Rect::new(x, 0, (x + 64).min(W), H), &color);
                    }
                    Ok(())
                })?;
                session.execute("layer.new.layer", json!({}))?;

                let params = json!({
                    "points": [[1200, 2000], [4800, 2000]],
                    "size": size,
                    "spacing": 0.25,
                    "wet": wet,
                    "load": 50,
                    "mix": 50,
                    "flow": 100,
                    "sampleAllLayers": sample_all,
                    "cleanAfterStroke": true
                });
                let mut timings = Vec::with_capacity(REPEATS);
                let mut output = String::new();
                for repeat in 0..REPEATS {
                    let start = Instant::now();
                    session.execute("paint.mixerBrush", params.clone())?;
                    let elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;
                    if repeat > 0 {
                        timings.push(elapsed_ms);
                    }
                    output = fingerprint(&session)?;
                    session.execute("edit.undo", json!({}))?;
                }
                timings.sort_by(f64::total_cmp);
                println!(
                    "wet={wet:3}% sample_all={sample_all:5} size={size:3} median_ms={:.2} min_ms={:.2} max_ms={:.2} blake3={output}",
                    timings[1], timings[0], timings[2]
                );
            }
        }
    }
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
