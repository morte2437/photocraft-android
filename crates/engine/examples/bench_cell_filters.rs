//! Time the actual menu command on a 24 MP layer, including history commit.
//! RAYON_NUM_THREADS=16 cargo run --release -p photocraft-engine --example bench_cell_filters -- crystallize 10 5 8
//! Arguments: crystallize|pointillize, cell size, repeats (1..9), depth (8|16|32), layer|background.
//! Run the same example at the base and changed revisions; compare the output hashes.
use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_engine::Session;
use photocraft_geom::Rect;
use photocraft_raster::Surface;
use serde_json::json;
use std::time::Instant;
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let case = args.first().map(String::as_str).unwrap_or("crystallize");
    let cell: f32 = args.get(1).map(|v| v.parse()).transpose()?.unwrap_or(10.0);
    let reps: usize = args.get(2).map(|v| v.parse()).transpose()?.unwrap_or(5);
    let depth: u32 = args.get(3).map(|v| v.parse()).transpose()?.unwrap_or(8);
    if !matches!(case, "crystallize" | "pointillize") || !(3.0..=300.0).contains(&cell) || !(1..=9).contains(&reps) {
        return Err("case, cell size or repeat count out of range".into());
    }
    let kind = args.get(4).map(String::as_str).unwrap_or("layer");
    if !matches!(kind, "layer" | "background") {
        return Err("kind must be layer or background".into());
    }
    let (w, h) = (6000, 4000);
    let bounds = Rect::new(0, 0, w, h);
    let sample = match depth {
        8 => SampleType::U8,
        16 => SampleType::U16,
        32 => SampleType::F32,
        _ => return Err("invalid depth".into()),
    };
    let mut source = Surface::new(PixelFormat::new(ColorMode::Rgb, sample, true));
    for tile in bounds.tiles() {
        let r = tile.rect().intersect(&bounds);
        let mut data = Vec::new();
        for y in r.y0..r.y1 {
            for x in r.x0..r.x1 {
                let seed = (x as u32).wrapping_mul(0x9e37_79b9) ^ (y as u32).wrapping_mul(0x85eb_ca6b);
                for c in 0..4 {
                    let value = seed.rotate_left(c * 7).wrapping_add(c * 71) % 65536;
                    data.push(if c == 3 && kind == "background" {
                        1.0
                    } else if c == 3 && seed.is_multiple_of(5) {
                        0.0
                    } else {
                        value as f32 / 65535.0
                    });
                }
            }
        }
        source.write_region(r, &data);
    }
    let id = format!("filter.pixelate.{case}");
    let mut times = Vec::new();
    let hash = |surface: &Surface| {
        let mut hash = blake3::Hasher::new();
        for tile in bounds.tiles() {
            hash.update(&surface.to_interleaved(tile.rect().intersect(&bounds)));
        }
        hash.finalize()
    };
    let source_hash = hash(&source);
    let mut output_hash = None;
    for run in 0..reps {
        let mut s = Session::new();
        s.execute("file.new", json!({"width":w,"height":h,"depth":depth,"background":if kind == "background" { "white" } else { "transparent" }}))?;
        s.edit("fixture", |doc, _| {
            let layer = doc.layers.first_mut().ok_or(photocraft_engine::EngineError::Other("no layer".into()))?;
            let surface = layer.surface_mut().ok_or(photocraft_engine::EngineError::Other("no surface".into()))?;
            *surface = source.clone();
            Ok(())
        })?;
        let start = Instant::now();
        s.execute(&id, json!({"cellSize":cell,"seed":37,"background":[1.0,1.0,1.0,1.0]}))?;
        let seconds = start.elapsed().as_secs_f64();
        println!("COMMAND {case} cell={cell} depth={depth} kind={kind} run={} seconds={seconds:.6}", run + 1);
        times.push(seconds);
        if run + 1 == reps {
            let doc = s.active().ok_or("no document")?;
            let layer = doc.doc.layers.first().ok_or("no layer")?;
            let surface = layer.surface().ok_or("no surface")?;
            let filtered_hash = hash(surface);
            s.execute("edit.undo", json!({}))?;
            let restored = s.active().and_then(|d| d.doc.layers.first()).and_then(|l| l.surface()).ok_or("no undo surface")?;
            if hash(restored) != source_hash {
                return Err("undo changed source pixels".into());
            }
            s.execute("edit.redo", json!({}))?;
            let redone = s.active().and_then(|d| d.doc.layers.first()).and_then(|l| l.surface()).ok_or("no redo surface")?;
            if hash(redone) != filtered_hash {
                return Err("redo changed filtered pixels".into());
            }
            output_hash = Some(filtered_hash);
        }
    }
    times.sort_by(f64::total_cmp);
    println!("MEDIAN {} HASH {}", times.get(times.len() / 2).ok_or("no runs")?, output_hash.ok_or("no output hash")?);
    Ok(())
}
