//! Sampled-tip preparation and live strokes on a materialized 24 MP layer. This harness also
//! builds against the pre-cache revision for alternating before/after runs on the same runner.
//! `cargo run --release -p photocraft-engine --example bench_brush_mips`
use std::hint::black_box;
use std::time::Instant;

use photocraft_color::{PixelFormat, SampleType};
use photocraft_engine::{EngineError, Session, brush_cmds::LiveStroke};
use photocraft_geom::Rect;
use photocraft_paint::{BrushContext, GrayTile, StrokePoint, TipShape};
use serde_json::json;

fn ms(start: Instant) -> f64 {
    start.elapsed().as_secs_f64() * 1e3
}

fn median(v: &mut [f64]) -> f64 {
    v.sort_by(f64::total_cmp);
    v.get(v.len() / 2).copied().unwrap_or(0.0)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    for depth in [8, 16, 32] {
        let sample = match depth {
            16 => SampleType::U16,
            32 => SampleType::F32,
            _ => SampleType::U8,
        };
        let fmt = PixelFormat::RGBA8.with_sample(sample);
        let mut s = Session::new();
        s.execute("file.new", json!({"width":6000,"height":4000,"depth":depth}))?;
        s.execute("layer.new.layer", json!({}))?;
        s.edit("materialize", |doc, active| {
            let id = active.ok_or_else(|| EngineError::Other("no active layer".into()))?;
            let surf = doc.layer_mut(id).and_then(|l| l.surface_mut()).ok_or_else(|| EngineError::Other("no pixel layer".into()))?;
            surf.fill_rect(Rect::new(0, 0, 6000, 4000), &photocraft_raster::from_rgba(&fmt, [0.3, 0.6, 0.8, 0.7]));
            Ok(())
        })?;
        for side in [0, 1024, 2048, 5000] {
            let original = (side > 0).then(|| GrayTile::from_fn(side, side, |x, y| ((x * 13 + y * 7) % 31) as f32 / 30.0));
            for symmetry in ["off", "dual"] {
                if symmetry == "off" {
                    if s.is_enabled("paint.symmetryDisable") {
                        s.execute("paint.symmetryDisable", json!({}))?;
                    }
                } else {
                    s.execute("paint.setSymmetry", json!({"mode":symmetry}))?;
                }
                let (mut cold, mut warm, mut begin, mut draw, mut release) = (Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new());
                let mut fingerprint = None;
                // A fresh allocation makes every cold preparation a genuine miss. Copying
                // samples and materializing the document are outside all measured stages.
                for trial in 0..8 {
                    s.tools.brush.tip = original
                        .as_ref()
                        .map_or(TipShape::Round, |t| TipShape::Sampled(GrayTile { width: t.width, height: t.height, data: t.data.as_ref().to_vec().into() }));
                    let t = Instant::now();
                    let ctx = BrushContext::new(&s.tools.brush);
                    let cold_ms = ms(t);
                    black_box(&ctx);
                    drop(ctx);
                    let t = Instant::now();
                    let ctx = BrushContext::new(&s.tools.brush);
                    let warm_ms = ms(t);
                    black_box(&ctx);
                    drop(ctx);
                    let mut points: Vec<[f64; 3]> = (0..65).map(|i| [2600.0 + f64::from(i) * 5.0, 1800.0 + (f64::from(i) / 8.0).sin() * 30.0, 0.8]).collect();
                    let mut params = json!({"points":[points[0]],"size":60,"flow":0.5,"smoothing":0.0,"seed":42});
                    let t = Instant::now();
                    let mut live = LiveStroke::begin(&s, &params)?;
                    let begin_ms = ms(t);
                    let t = Instant::now();
                    for batch in points.get(1..).unwrap_or_default().chunks(4) {
                        let batch: Vec<_> = batch.iter().map(|p| StrokePoint::new(p[0], p[1], p[2] as f32)).collect();
                        live.push(&batch)?;
                    }
                    let draw_ms = ms(t);
                    let surface = live.doc.layers.iter().rev().find_map(|l| l.surface()).ok_or("no painted surface")?;
                    let hash = surface
                        .read_region(Rect::new(2500, 1700, 3500, 2300))
                        .iter()
                        .fold(0xcbf2_9ce4_8422_2325u64, |hash, v| (hash ^ u64::from(v.to_bits())).wrapping_mul(0x100_0000_01b3));
                    if fingerprint.is_some_and(|previous| previous != hash) {
                        return Err("stroke fingerprint changed across repeated trials".into());
                    }
                    fingerprint = Some(hash);
                    params["points"] = json!(std::mem::take(&mut points));
                    let t = Instant::now();
                    s.offer_live_stroke(live)?;
                    let result = s.execute("paint.stroke", params)?;
                    let release_ms = ms(t);
                    if result.get("fromLive").is_none() {
                        return Err("commit did not take the live pixels".into());
                    }
                    s.execute("edit.undo", json!({}))?;
                    if trial > 0 {
                        cold.push(cold_ms);
                        warm.push(warm_ms);
                        begin.push(begin_ms);
                        draw.push(draw_ms);
                        release.push(release_ms);
                    }
                }
                println!(
                    "tip={side} depth={depth} symmetry={symmetry} cold_ms={:.6} warm_ms={:.6} begin_ms={:.6} draw_ms={:.6} release_ms={:.6} fingerprint={:016x}",
                    median(&mut cold),
                    median(&mut warm),
                    median(&mut begin),
                    median(&mut draw),
                    median(&mut release),
                    fingerprint.unwrap_or(0)
                );
            }
        }
    }
    Ok(())
}
