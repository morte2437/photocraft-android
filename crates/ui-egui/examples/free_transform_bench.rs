//! Free Transform stalls (#11103): two selected raster layers on a 24 MP document, one box.
//!
//! ```sh
//! cargo run --release -p photocraft-ui-egui --example free_transform_bench -- [--size 6000x4000] [--reps 9] [--steps 60]
//! ```
//!
//! - begin: Ctrl+T (`transform_tool::begin`) and the first preview frame, then Esc.
//! - drag update: one pointer move and the preview frame it draws. "scale" drags the bottom-right
//!   handle towards the top-left corner, shrinking the box from 100 % to 20 % at 100 % view zoom,
//!   so the scale crosses ½ and ¼ (new mip levels) mid-drag; "move" drags the box by its middle.
//!
//! CPU side only: the egui frame and the texture preparation it does, not GPU upload or
//! presentation. A helper thread's work (a mip level built off the UI thread) isn't counted.

use std::time::Instant;

use photocraft_ui_egui::canvas::{ToolEvent, ViewXform};
use photocraft_ui_egui::{PhotocraftApp, Services, transform_tool};
use serde_json::json;

fn arg(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1).cloned())
}

fn median(v: &[f64]) -> f64 {
    let mut v = v.to_vec();
    v.sort_by(f64::total_cmp);
    v.get(v.len() / 2).copied().unwrap_or(0.0)
}

fn worst(v: &[f64]) -> f64 {
    v.iter().copied().fold(0.0, f64::max)
}

/// The view the frames draw through: 100 %, the document centred in a 1600 × 1000 canvas.
fn view(w: u32, h: u32) -> ViewXform {
    ViewXform {
        rect: egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1600.0, 1000.0)),
        zoom: 1.0,
        center: [w as f32 / 2.0, h as f32 / 2.0],
        flip: false,
        rotation: 0.0,
        aspect: 1.0,
    }
}

/// The texture size limit a desktop GPU reports (egui's default without a renderer is 2048,
/// which would shrink the preview to a fraction of the layer and hide the cost being measured).
const MAX_TEXTURE_SIDE: usize = 16384;

/// One frame drawing the open box's preview, in milliseconds.
fn frame(app: &PhotocraftApp, ctx: &egui::Context, xf: &ViewXform) -> f64 {
    let input = egui::RawInput { screen_rect: Some(xf.rect), max_texture_side: Some(MAX_TEXTURE_SIDE), ..Default::default() };
    let start = Instant::now();
    let output = ctx.run_ui(input, |ui| transform_tool::draw_content(app, ui.painter(), xf));
    let ms = start.elapsed().as_secs_f64() * 1e3;
    drop(output);
    ms
}

fn pointer(app: &mut PhotocraftApp, ev: ToolEvent) {
    transform_tool::pointer(app, ev, egui::Modifiers::NONE);
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let (w, h) =
        arg(&args, "--size").and_then(|s| s.split_once('x').and_then(|(a, b)| Some((a.parse::<u32>().ok()?, b.parse::<u32>().ok()?)))).unwrap_or((6000, 4000));
    let reps: usize = arg(&args, "--reps").and_then(|s| s.parse().ok()).unwrap_or(9).max(1);
    let steps: usize = arg(&args, "--steps").and_then(|s| s.parse().ok()).unwrap_or(60).max(2);

    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Services::default());
    app.run("file.new", json!({"width": w, "height": h, "background": "white"})).expect("new document");
    // Two overlapping layers, each covering most of the canvas.
    let (fw, fh) = (w * 2 / 3, h * 3 / 4);
    let mut ids = Vec::new();
    for (name, x, y, color) in [("A", 0, 0, "#c03020"), ("B", w - fw, h - fh, "#2050c0")] {
        let id = app.run("layer.new.layer", json!({"name": name})).expect("new layer")["layer"].as_u64().unwrap_or(0);
        app.run("layer.select", json!({"layer": id})).expect("select");
        app.run("select.rect", json!({"x": x, "y": y, "width": fw, "height": fh})).expect("select.rect");
        app.run("edit.fill", json!({"contents": "color", "color": color})).expect("fill");
        app.run("select.deselect", json!({})).expect("deselect");
        ids.push(id);
    }
    if let (Some(a), Some(b)) = (ids.first(), ids.get(1)) {
        app.run("layer.select", json!({"layer": a})).expect("select A");
        app.run("layer.select", json!({"layer": b, "mode": "add"})).expect("add B");
    }
    app.sync_views();
    if let Some(v) = app.ui.views.first_mut() {
        v.zoom = 1.0;
        v.center = [w as f32 / 2.0, h as f32 / 2.0];
        v.fit_pending = false;
    }
    let xf = view(w, h);
    let ctx = egui::Context::default();
    let _ = ctx.run_ui(egui::RawInput { max_texture_side: Some(MAX_TEXTURE_SIDE), ..Default::default() }, |_| {});

    // Ctrl+T and its first preview frame.
    let (mut begin, mut first, mut begin_total) = (Vec::new(), Vec::new(), Vec::new());
    for _ in 0..reps {
        let t0 = Instant::now();
        transform_tool::begin(&mut app, &ctx).expect("Free Transform");
        let opened = t0.elapsed().as_secs_f64() * 1e3;
        let drawn = frame(&app, &ctx, &xf);
        begin.push(opened);
        first.push(drawn);
        begin_total.push(opened + drawn);
        transform_tool::cancel(&mut app);
    }

    // Drags: one pointer move + its frame per update.
    let mut drag = |kind: &str| -> Vec<f64> {
        transform_tool::begin(&mut app, &ctx).expect("Free Transform");
        let _ = frame(&app, &ctx, &xf);
        let q = app.ui.transform.as_ref().map(|t| t.quad).unwrap_or_default();
        let (from, to) = if kind == "scale" {
            // The bottom-right handle towards the top-left corner: 100 % → 20 %.
            let (a, c) = (q[0], q[2]);
            (c, [a[0] + 0.2 * (c[0] - a[0]), a[1] + 0.2 * (c[1] - a[1])])
        } else {
            let m = [(q[0][0] + q[2][0]) / 2.0, (q[0][1] + q[2][1]) / 2.0];
            (m, [m[0] + 400.0, m[1] + 250.0])
        };
        pointer(&mut app, ToolEvent::Down { x: from[0], y: from[1], pressure: 1.0 });
        let mut times = Vec::with_capacity(steps);
        for s in 1..=steps {
            let k = s as f64 / steps as f64;
            let p = [from[0] + k * (to[0] - from[0]), from[1] + k * (to[1] - from[1])];
            let t0 = Instant::now();
            pointer(&mut app, ToolEvent::Move { x: p[0], y: p[1], pressure: 1.0 });
            let moved = t0.elapsed().as_secs_f64() * 1e3;
            times.push(moved + frame(&app, &ctx, &xf));
        }
        pointer(&mut app, ToolEvent::Up { x: to[0], y: to[1] });
        transform_tool::cancel(&mut app);
        times
    };
    let mut scale = Vec::new();
    let mut moving = Vec::new();
    for _ in 0..reps.min(3) {
        scale.extend(drag("scale"));
        moving.extend(drag("move"));
    }

    if let Some(out) = arg(&args, "--json") {
        use photocraft_testkit::perf::{process_peak_rss_bytes, report, row, write_report};
        let rss = photocraft_testkit::perf::current_rss_bytes().max(process_peak_rss_bytes());
        let rows = vec![
            row("Free Transform begin, 2 layers on 24 MP: Ctrl+T + first frame", &begin_total, rss, None),
            row("Free Transform begin, 2 layers on 24 MP: Ctrl+T", &begin, rss, None),
            row("Free Transform begin, 2 layers on 24 MP: first frame", &first, rss, None),
            row("Free Transform drag update, 2 layers on 24 MP: scale through mip levels", &scale, rss, None),
            row("Free Transform drag update, 2 layers on 24 MP: move", &moving, rss, None),
        ];
        let rep = report("free_transform_bench", json!({"width": w, "height": h, "layers": 2, "reps": reps, "steps": steps}), rows, None);
        if let Err(e) = write_report(&out, &rep) {
            eprintln!("{e}");
        }
    }
    println!("{w}×{h}, 2 layers, {reps} reps, {steps} steps per drag");
    println!(
        "begin: Ctrl+T {:.1} ms + first frame {:.1} ms = {:.1} ms (median; worst {:.1} ms)",
        median(&begin),
        median(&first),
        median(&begin_total),
        worst(&begin_total)
    );
    println!("drag update, scale 100 % → 20 %: median {:.2} ms, worst {:.2} ms", median(&scale), worst(&scale));
    println!("drag update, move: median {:.2} ms, worst {:.2} ms", median(&moving), worst(&moving));
}
