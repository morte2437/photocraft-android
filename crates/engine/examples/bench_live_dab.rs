//! Live retouch latency on a materialized 24 MP layer. Run the same example at base and head
//! in release mode on one machine. Setup, Undo and the full output fingerprint are not timed.
//! This measures engine stroke work; canvas compositing/upload and presentation are excluded.
use std::time::Instant;

use photocraft_engine::{EngineError, Session, paint::StrokePoint, retouch_cmds::LiveDab};
use photocraft_geom::Rect;
use serde_json::json;

fn elapsed_ms(start: Instant) -> f64 {
    start.elapsed().as_secs_f64() * 1000.0
}

fn median(values: &mut [f64]) -> f64 {
    values.sort_by(f64::total_cmp);
    values.get(values.len() / 2).copied().unwrap_or(0.0)
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    // Four pointer samples per 60 Hz frame, ~1000 px/s, over a 960 px stroke.
    // Large soft tips also exercise pixels beyond the canvas edges.
    let points: Vec<_> = (0..=240).map(|i| StrokePoint::new(2500.0 + f64::from(i) * 4.0, 2000.0, 1.0)).collect();
    for depth in [8, 16, 32] {
        let mut s = Session::new();
        s.execute("file.new", json!({"width":6000,"height":4000,"depth":depth,"mode":"rgb"}))?;
        s.execute("layer.new.layer", json!({}))?;
        s.edit("stripes", |doc, active| {
            let id = active.ok_or_else(|| EngineError::Other("no layer".into()))?;
            let surface = doc.layer_mut(id).and_then(|l| l.surface_mut()).ok_or_else(|| EngineError::Other("no surface".into()))?;
            let fmt = surface.format();
            for x in (0..6000).step_by(64) {
                let color = if x % 128 == 0 { [0.9, 0.3, 0.1, 0.7] } else { [0.1, 0.4, 0.8, 1.0] };
                surface.fill_rect(Rect::new(x, 0, (x + 64).min(6000), 4000), &photocraft_raster::from_rgba(&fmt, color));
            }
            Ok(())
        })?;
        for cmd in ["paint.blur", "paint.smudge", "paint.dodge"] {
            for size in [300, 600, 1000, 2500] {
                for hardness in [0, 100] {
                    let params = json!({
                        "points":points.iter().map(|q| [q.x, q.y]).collect::<Vec<_>>(),
                        "size":size,"hardness":hardness,"strength":50,"spacing":25
                    });
                    let mut start_params = params.clone();
                    start_params["points"] = json!([[2500, 2000]]);
                    let (mut draws, mut worst_frames, mut releases) = (Vec::new(), Vec::new(), Vec::new());
                    let mut fingerprint = None;
                    // One warm-up followed by three measured strokes, always starting from the same pixels.
                    for sample in 0..4 {
                        let start = Instant::now();
                        let mut live = LiveDab::begin(&s, cmd, &start_params)?;
                        let mut draw = elapsed_ms(start);
                        let mut worst = draw;
                        for batch in points.get(1..).unwrap_or_default().chunks(4) {
                            let start = Instant::now();
                            live.push(batch)?;
                            let duration = elapsed_ms(start);
                            draw += duration;
                            worst = worst.max(duration);
                        }
                        let bounds = live.bounds();
                        let start = Instant::now();
                        s.offer_live_dab(live)?;
                        let result = s.execute(cmd, params.clone())?;
                        let release = elapsed_ms(start);
                        if result.get("fromLive").and_then(|v| v.as_bool()) != Some(true) {
                            return Err("the commit did not take the live pixels".into());
                        }
                        let st = s.active().ok_or("no document")?;
                        let surface = st.doc.layer(st.active_layer.ok_or("no layer")?).and_then(|l| l.surface()).ok_or("no surface")?;
                        // Include finishing dabs, without depending on a new API unavailable at base.
                        let damage = result.get("damage").and_then(|v| v.as_array()).ok_or("missing damage")?;
                        if damage.len() != 4 {
                            return Err("invalid damage length".into());
                        }
                        let mut coords = [0i32; 4];
                        for (coord, value) in coords.iter_mut().zip(damage) {
                            *coord = i32::try_from(value.as_i64().ok_or("invalid damage")?)?;
                        }
                        let [x, y, width, height] = coords;
                        let bounds = bounds.union(&Rect::new(x, y, x.saturating_add(width), y.saturating_add(height)));
                        let mut hash = 0xcbf29ce484222325u64;
                        // Hash every channel of the modified rectangle, one row at a time to bound memory.
                        for y in bounds.y0..bounds.y1 {
                            for channel in surface.read_region(Rect::new(bounds.x0, y, bounds.x1, y + 1)) {
                                hash = (hash ^ u64::from(channel.to_bits())).wrapping_mul(0x100000001b3);
                            }
                        }
                        if fingerprint.is_some_and(|previous| previous != hash) {
                            return Err("repeated strokes produced different pixels".into());
                        }
                        fingerprint = Some(hash);
                        s.execute("edit.undo", json!({}))?;
                        if sample > 0 {
                            draws.push(draw);
                            worst_frames.push(worst);
                            releases.push(release);
                        }
                    }
                    println!(
                        "{cmd} depth={depth} size={size} hardness={hardness} draw_ms={:.3} worst_frame_ms={:.3} release_ms={:.3} fingerprint={:016x}",
                        median(&mut draws),
                        median(&mut worst_frames),
                        median(&mut releases),
                        fingerprint.unwrap_or_default()
                    );
                }
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
