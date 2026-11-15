//! Live painting with a big brush on a 4K document: the time each frame of a live stroke spends
//! in `LiveStroke::push` (the engine's half of a frame; the canvas refresh comes after it).
//!
//! ```sh
//! cargo run --release -p photocraft-engine --example bench_big_brush -- [brush.json] [--frames N] [--sizes 300,800,1500]
//! ```
//!
//! `brush.json` is a brush as Preferences saves it (`lastBrush.brush`, without its `tip`s). Without
//! it the brush is a typical imported painting brush: a sampled tip at 5 % spacing with a Dual
//! Brush twice its size, pressure size, opacity and flow, and 17 % smoothing. Then a soft round.
//! Sampled tips are stand-ins of the imported brush's sizes.
use photocraft_engine::{Session, brush_cmds::LiveStroke};
use photocraft_paint::{StrokePoint, brush::BrushSettings, brush::TipShape, tile::GrayTile};
use serde_json::json;
use std::time::Instant;

fn arg(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1).cloned())
}

/// A rough, grainy blob like a scanned bristle tip.
fn tip(w: u32, h: u32, salt: u32) -> GrayTile {
    GrayTile::from_fn(w, h, |x, y| {
        let (dx, dy) = (x as f32 / w as f32 - 0.5, y as f32 / h as f32 - 0.5);
        let r = (dx * dx + dy * dy).sqrt() * 2.0;
        let n = ((x.wrapping_mul(73_856_093) ^ y.wrapping_mul(19_349_663) ^ salt) % 1000) as f32 / 1000.0;
        ((1.0 - r).max(0.0) * (0.4 + 0.6 * n)).min(1.0)
    })
}

fn pct(v: &mut [f64], p: f64) -> f64 {
    v.sort_by(f64::total_cmp);
    v[((v.len() - 1) as f64 * p).round() as usize]
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let frames: usize = arg(&args, "--frames").and_then(|v| v.parse().ok()).unwrap_or(60).clamp(1, 10_000);
    let sizes: Vec<f64> = arg(&args, "--sizes").unwrap_or("300,800,1500".into()).split(',').filter_map(|v| v.parse().ok()).collect();
    let file = args.get(1).filter(|a| !a.starts_with("--"));
    let pressure = json!({"control": "penPressure"});
    let mut base: BrushSettings = match file {
        Some(f) => serde_json::from_str(&std::fs::read_to_string(f)?)?,
        None => serde_json::from_value(json!({
            "size": 143, "spacing": 0.05, "hardness": 1.0,
            "smoothing": {"enabled": true, "amount": 0.17, "catchUp": true, "catchUpOnEnd": true, "adjustForZoom": true},
            "dualBrush": {"enabled": true, "mode": "overlay", "size": 288, "spacing": 0.1, "roundness": 0.72, "angle": 180, "scatter": 0.47},
            "shapeDynamics": {"enabled": true, "size": {"control": "penPressure", "minimum": 0.69}, "angle": {"control": "initialDirection"}},
            "transfer": {"enabled": true, "opacity": pressure, "flow": pressure},
        }))?,
    };
    base.tip = TipShape::Sampled(tip(70, 73, 1));
    base.dual_brush.tip = TipShape::Sampled(tip(100, 97, 2));
    let mut no_dual = base.clone();
    no_dual.dual_brush.enabled = false;
    let mut no_smooth = base.clone();
    no_smooth.smoothing.enabled = false;
    let mut neither = no_dual.clone();
    neither.smoothing.enabled = false;
    let round = BrushSettings { hardness: 0.0, ..BrushSettings::default() };
    let variants = [("as saved", base), ("no dual", no_dual), ("no smoothing", no_smooth), ("no dual, no smoothing", neither), ("soft round", round)];
    let (w, h) = (3840.0, 2160.0);
    for size in &sizes {
        for (name, brush) in &variants {
            let mut s = Session::new();
            s.execute("file.new", json!({"width": w, "height": h, "background": "white"}))?;
            s.execute("layer.new.layer", json!({"name": "paint"}))?;
            s.tools.brush = brush.clone();
            // A 4K canvas fit to a 4K screen with panels: about 45 %.
            let zoom = 0.45;
            // Each frame brings four pen samples, 11.2 px apart across (a brisk stroke at 120 Hz),
            // whatever `--frames` is; long runs sweep back and forth over the canvas.
            let at = |k: usize| {
                let sweep = k as f64 * 11.2 / (0.7 * w);
                let t = 1.0 - (sweep % 2.0 - 1.0).abs();
                let p = StrokePoint::new(w * (0.15 + 0.7 * t), h * (0.5 + 0.25 * (t * 9.0).sin()), (0.4 + 0.6 * (t * 5.0).sin().abs()) as f32);
                StrokePoint { time: k as f64 * 2.0, ..p }
            };
            let first = at(0);
            let p = json!({"points": [[first.x, first.y, first.pressure]], "size": size, "zoom": zoom, "seed": 7});
            let t = Instant::now();
            let mut live = LiveStroke::begin(&s, &p)?;
            let begin = t.elapsed().as_secs_f64() * 1000.0;
            let mut ms = Vec::with_capacity(frames);
            for f in 0..frames {
                let pts: Vec<StrokePoint> = (1..=4).map(|i| at(f * 4 + i)).collect();
                let t = Instant::now();
                let dmg = live.push(&pts)?;
                std::hint::black_box(dmg);
                ms.push(t.elapsed().as_secs_f64() * 1000.0);
            }
            let total: f64 = ms.iter().sum();
            println!(
                "{size:>5} px  {name:<22} begin {begin:6.1} ms   frame p50 {:6.1}  p95 {:6.1}  max {:6.1} ms   ({:.0} ms over {frames} frames)",
                pct(&mut ms.clone(), 0.5),
                pct(&mut ms.clone(), 0.95),
                pct(&mut ms, 1.0),
                total
            );
        }
    }
    Ok(())
}
