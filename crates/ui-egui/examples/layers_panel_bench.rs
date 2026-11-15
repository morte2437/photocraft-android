//! Per-frame CPU cost of the Layers panel with many groups (#13117): a group row that was scrolled
//! out of view used to paint its whole row (background, thumbnail, name, badges) every frame. This
//! is the before/after measure for that fix.
//!
//! ```sh
//! cargo run --release -p photocraft-ui-egui --example layers_panel_bench -- [--groups 300] [--frames 300] [--expanded] [--plain] [--json out.json]
//! ```
//!
//! The document is built through Session commands on a tiny 64x48 canvas, so the canvas costs next
//! to nothing: `--groups N` top-level collapsed groups, each holding one pixel layer (`--expanded`
//! opens them all afterwards); `--plain` builds N plain pixel layers instead, a control whose rows
//! were already culled off-screen. The real `PhotocraftApp` runs in a plain CPU egui_kittest
//! harness (no wgpu), so a frame's time is the UI's own CPU work. After a warm-up, the row is the
//! median / p90 / max of `--frames` idle frames (`h.step()` each).

use std::time::Instant;

use egui_kittest::kittest::Queryable;
use photocraft_ui_egui::{PhotocraftApp, Services};
use serde_json::{Value, json};

/// The app in an offscreen harness.
type H = egui_kittest::Harness<'static, PhotocraftApp>;

/// The value after `name` on the command line, if any.
fn arg(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1).cloned())
}

/// Milliseconds since `t`.
fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1000.0
}

/// One frame, timed.
fn frame(h: &mut H) -> f64 {
    let t = Instant::now();
    h.step();
    ms(t)
}

/// Result rows: name, then median, p90 and max frame time (ms).
struct Rows {
    rows: Vec<(String, f64, f64, f64)>,
}

impl Rows {
    /// A row from frame times already taken.
    fn add(&mut self, name: &str, v: Vec<f64>) {
        let mut v: Vec<f64> = v.into_iter().filter(|t| t.is_finite()).collect();
        if v.is_empty() {
            return;
        }
        v.sort_by(f64::total_cmp);
        let (med, p90, max) = (v[v.len() / 2], v[(v.len() * 9 / 10).min(v.len() - 1)], v[v.len() - 1]);
        println!("{name:<46} median {med:>8.3} ms   p90 {p90:>8.3}   max {max:>8.3}");
        self.rows.push((name.to_string(), med, p90, max));
    }
}

/// `n` top-level collapsed groups (or plain layers), each group holding one pixel layer.
fn build(n: usize, plain: bool, expanded: bool) -> photocraft_engine::Session {
    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width": 64, "height": 48})).expect("file.new");
    for i in 0..n {
        s.execute("layer.new.layer", json!({"name": format!("Layer {i}")})).expect("layer.new.layer");
        if !plain {
            // Group Layers makes a collapsed group; the next new layer lands above it, at the same level.
            s.execute("layer.groupLayers", json!({"name": format!("Group {i}")})).expect("layer.groupLayers");
        }
    }
    if expanded && !plain {
        s.execute("layer.setExpanded", json!({"all": true, "expanded": true})).expect("layer.setExpanded");
    }
    s
}

/// Builds the document, warms the app up, times idle frames and prints (and optionally writes) the row.
fn main() {
    let args: Vec<String> = std::env::args().collect();
    let groups: usize = arg(&args, "--groups").and_then(|v| v.parse().ok()).unwrap_or(300).max(1);
    let frames: usize = arg(&args, "--frames").and_then(|v| v.parse().ok()).unwrap_or(300).max(3);
    let expanded = args.iter().any(|a| a == "--expanded");
    let plain = args.iter().any(|a| a == "--plain");

    let t = Instant::now();
    let session = build(groups, plain, expanded);
    println!("built {groups} {} in {:.0} ms", if plain { "plain layers" } else { "groups" }, ms(t));

    let mut h: H = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).with_pixels_per_point(1.0).build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        PhotocraftApp::new(session, Services::default())
    });
    // Warm up. The layer made last is the top one and is active, so the panel is scrolled to the
    // top and most rows are off-screen.
    for _ in 0..30 {
        h.step();
    }

    // How many group rows are on screen (their disclosure widgets, which every group row registers).
    let window = h.ctx.content_rect();
    let (on_screen, total) = if plain {
        (0, 0)
    } else {
        let all: Vec<_> = h.query_all_by_label_contains("group Group ").map(|n| n.rect()).collect();
        (all.iter().filter(|r| window.intersects(**r)).count(), all.len())
    };
    if !plain {
        println!("group rows on screen: {on_screen} of {total} listed (document has {groups})");
    }

    let mut rows = Rows { rows: Vec::new() };
    let times: Vec<f64> = (0..frames).map(|_| frame(&mut h)).collect();
    let what = if plain { "plain layers" } else { "groups" };
    rows.add(&format!("idle frame, {groups} {what} (panel at top)"), times);

    if let Some(out) = arg(&args, "--json") {
        let r: Vec<Value> = rows.rows.iter().map(|(n, med, p90, max)| json!({"name": n, "median_ms": med, "p90_ms": p90, "max_ms": max})).collect();
        let report = json!({
            "groups": groups,
            "frames": frames,
            "expanded": expanded,
            "plain": plain,
            "group_rows_on_screen": on_screen,
            "group_rows_listed": total,
            "rows": r,
        });
        std::fs::write(&out, serde_json::to_string_pretty(&report).expect("json")).expect("write json");
    }
}
