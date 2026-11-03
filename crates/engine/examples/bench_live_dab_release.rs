//! Release cost of a live sequential-dab stroke on a 24 MP layer, in release mode: the commit
//! rendering the stroke again (no offer) against taking the live pixels (`offer_live_dab`, which
//! runs the finishing dabs, plus the commit). Also the live stroke's per-frame cost while drawn.
use std::time::Instant;

use photocraft_engine::{EngineError, Session, paint::StrokePoint, retouch_cmds::LiveDab};
use photocraft_geom::Rect;
use serde_json::json;

const FRAMES: usize = 60;

fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1e3
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut s = Session::new();
    s.execute("file.new", json!({"width":6000,"height":4000,"depth":8,"mode":"rgb"}))?;
    s.execute("layer.new.layer", json!({}))?;
    s.edit("stripes", |doc, active| {
        let id = active.ok_or_else(|| EngineError::Other("no layer".into()))?;
        let surface = doc.layer_mut(id).and_then(|l| l.surface_mut()).ok_or_else(|| EngineError::Other("no surface".into()))?;
        for x in (0..6000).step_by(64) {
            let c = if x % 128 == 0 { [0.9, 0.3, 0.1, 1.0] } else { [0.1, 0.4, 0.8, 1.0] };
            surface.fill_rect(Rect::new(x, 0, (x + 64).min(6000), 4000), &c);
        }
        Ok(())
    })?;
    // ~1000 px/s at 60 fps: 16 px of drag per frame, in four samples.
    let pts: Vec<[f64; 2]> = std::iter::once([1000.0, 2000.0])
        .chain((1..=FRAMES).flat_map(|f| (1..=4).map(move |k| [1000.0 + (f as f64 - 1.0) * 16.0 + k as f64 * 4.0, 2000.0])))
        .collect();
    for cmd in ["paint.blur", "paint.smudge", "paint.dodge"] {
        for size in [50, 150, 300, 600] {
            let p = |v: &[[f64; 2]]| json!({"points": v, "size": size, "hardness": 0, "strength": 50, "spacing": 25});
            let draw = |s: &Session| -> Result<(LiveDab, Vec<f64>), Box<dyn std::error::Error>> {
                let mut live = LiveDab::begin(s, cmd, &p(&pts[..1]))?;
                let mut frames = Vec::new();
                for batch in pts[1..].chunks(4) {
                    let b: Vec<_> = batch.iter().map(|q| StrokePoint::new(q[0], q[1], 1.0)).collect();
                    let t = Instant::now();
                    live.push(&b)?;
                    frames.push(ms(t));
                }
                Ok((live, frames))
            };
            let (mut before, mut after) = (Vec::new(), Vec::new());
            let mut frames = Vec::new();
            for _ in 0..3 {
                let (_, f) = draw(&s)?;
                frames.extend(f);
                let t = Instant::now();
                s.execute(cmd, p(&pts))?;
                before.push(ms(t));
                s.execute("edit.undo", json!({}))?;
                let (live, _) = draw(&s)?;
                let t = Instant::now();
                s.offer_live_dab(live)?;
                let r = s.execute(cmd, p(&pts))?;
                after.push(ms(t));
                if r.get("fromLive").is_none() {
                    return Err("the commit didn't take the live pixels".into());
                }
                s.execute("edit.undo", json!({}))?;
            }
            before.sort_by(f64::total_cmp);
            after.sort_by(f64::total_cmp);
            frames.sort_by(f64::total_cmp);
            let total: f64 = frames.iter().sum::<f64>() / 3.0;
            println!(
                "{cmd:12} size={size:4} release: render again={:7.1} ms  take live={:6.1} ms | live frames: max={:6.2} ms total={:7.1} ms",
                before[1],
                after[1],
                frames.last().copied().unwrap_or(0.0),
                total
            );
        }
    }
    Ok(())
}

fn main() {
    if let Err(e) = run() {
        eprintln!("{e}");
        std::process::exit(1);
    }
}
