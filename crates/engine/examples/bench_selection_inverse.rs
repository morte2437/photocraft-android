//! End-to-end timing of `select.inverse` on a 6000×4000 document.
//!
//! ```sh
//! cargo run --release -p photocraft-engine --example bench_selection_inverse -- [--quick] [--reps 11]
//! ```
//!
//! Three rows: inverting a rectangular marquee, an elliptical marquee (central 2/3 of the
//! canvas) and `select.all`. Setup is never timed — only the `execute("select.inverse")`
//! call between `Instant`s. After the timed op (before deselect) the resulting selection
//! surface is inspected for storage diagnostics: unique tile payloads (distinct `Arc`
//! allocations behind the coverage surface) and allocated tile count.

use std::collections::HashSet;
use std::time::Instant;

use photocraft_engine::Session;
use serde_json::{Value, json};

type Res = Result<(), Box<dyn std::error::Error>>;

fn arg<'a>(args: &'a [String], name: &str) -> Option<&'a String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1))
}

/// Nearest-rank percentile of a pre-sorted slice.
fn pct(sorted: &[f64], p: f64) -> f64 {
    let n = sorted.len();
    let idx = ((p / 100.0) * n as f64).ceil() as usize;
    sorted.get(idx.saturating_sub(1).min(n.saturating_sub(1))).copied().unwrap_or(0.0)
}

/// Selection setup for one row; each previous sample already deselected.
fn setup(s: &mut Session, w: i32, h: i32, ellipse: Option<bool>) -> Result<Value, Box<dyn std::error::Error>> {
    let params = match ellipse {
        Some(ellipse) => {
            // Central 2/3 of the canvas.
            let x = w / 6;
            let y = h / 6;
            json!({
                "x": x,
                "y": y,
                "width": w.saturating_mul(2) / 3,
                "height": h.saturating_mul(2) / 3,
                "ellipse": ellipse,
            })
        }
        None => json!({}), // select.all
    };
    let id = if ellipse.is_some() { "select.rect" } else { "select.all" };
    let v = s.execute(id, params)?;
    Ok(v)
}

/// Storage diagnostics of the current selection surface: distinct tile payloads and tiles held.
fn diagnostics(s: &Session) -> Result<(usize, usize), Box<dyn std::error::Error>> {
    let Some(state) = s.active() else {
        return Err("no active document after select.inverse".into());
    };
    let Some(sel) = state.doc.selection.as_ref() else {
        return Err("no selection after select.inverse".into());
    };
    let mut payloads: HashSet<usize> = HashSet::new();
    let mut count = 0usize;
    for (_, tile) in sel.tiles() {
        payloads.insert(std::sync::Arc::as_ptr(tile) as usize);
        count += 1;
    }
    Ok((payloads.len(), count))
}

/// One measured sample: setup, time ONLY `select.inverse`, print diagnostics, deselect.
fn sample(s: &mut Session, w: i32, h: i32, ellipse: Option<bool>) -> Result<(f64, usize, usize), Box<dyn std::error::Error>> {
    setup(s, w, h, ellipse)?;
    let t = Instant::now();
    s.execute("select.inverse", json!({}))?;
    let ms = t.elapsed().as_secs_f64() * 1000.0;
    let (payloads, tiles) = diagnostics(s)?;
    s.execute("select.deselect", json!({}))?;
    Ok((ms, payloads, tiles))
}

fn main() -> Res {
    let args: Vec<String> = std::env::args().collect();
    let (w, h) = if args.iter().any(|a| a == "--quick") { (1200, 800) } else { (6000, 4000) };
    let reps = arg(&args, "--reps").and_then(|v| v.parse::<usize>().ok()).unwrap_or(11).clamp(1, 100);

    let mut s = Session::new();
    s.execute("file.new", json!({"width": w, "height": h, "background": "transparent"}))?;

    let rows: [(&str, Option<bool>); 3] = [("Inverse rectangular", Some(false)), ("Inverse ellipse", Some(true)), ("Inverse all", None)];
    for (label, ellipse) in rows {
        // Warm-up: one untimed sample to page in allocations and caches.
        sample(&mut s, w, h, ellipse)?;
        let mut times: Vec<f64> = Vec::with_capacity(reps);
        for _ in 0..reps {
            let (ms, payloads, tiles) = sample(&mut s, w, h, ellipse)?;
            times.push(ms);
            println!("sample: {ms:8.2} ms (selection tiles: {payloads} unique payloads, {tiles} allocated)");
        }
        times.sort_by(|a, b| a.total_cmp(b));
        println!(
            "{label:<22} ({w}×{h}, {reps} reps)  p50 {p50:8.2} ms   p95 {p95:8.2} ms   max {max:8.2} ms",
            p50 = pct(&times, 50.0),
            p95 = pct(&times, 95.0),
            max = pct(&times, 100.0),
        );
    }
    Ok(())
}
