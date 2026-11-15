//! End-to-end timings of the interactive paths on a large photo (default 7360×4912 = 36 MP):
//! open JPEG / PSD, brush dab, Gaussian Blur, adjustment layer tweak, layer move, undo,
//! save .pcraft / .psd, export PNG / JPEG, plus the canvas refresh each edit triggers.
//!
//! ```sh
//! cargo run --release -p photocraft-ui-egui --example interactive_bench -- [--size 7360x4912] [--reps 5] [--json out.json] [--only text] [--cpu]
//! ```
//!
//! Every edit goes through `Session::execute` (the command path the UI, CLI and MCP share:
//! history snapshot, command, damage), then the canvas refresh the app would do: the GPU
//! compositor over the edit's damage rect (or the whole document), waited on so the time is the
//! real latency. Without a GPU adapter the CPU compositor stands in. Zoom and pan don't
//! recomposite (the canvas samples the composited texture's mip levels); "viewport (CPU)" times
//! what the CPU fallback pays for one 1920×1080 view. Each row is the median of `--reps` runs
//! (min / max shown); the first run of an operation is reported separately as "cold", and
//! path-reported rows keep its compositing path (`cold_path` in `--json`) beside the warm row's
//! `warm_path`. Adjustment preview rows cover a plain pixel target and a clipped one. A styled
//! clipped target keeps the CPU proxy (its preview group would rebuild effect shape maps on the
//! UI thread), so only its CPU reference rows run and the reason is printed.

use std::time::Instant;

use eframe::wgpu;
use photocraft_color::{BlendMode, Color, ColorMode, SampleType};
use photocraft_doc::{Document, Effect, FxCommon, Layer, LayerContent, LayerId, Size};
use photocraft_engine::Session;
use photocraft_geom::Rect;
use serde_json::{Value, json};

fn arg(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1).cloned())
}

fn block_on<F: std::future::Future>(f: F) -> F::Output {
    let mut cx = std::task::Context::from_waker(std::task::Waker::noop());
    let mut f = std::pin::pin!(f);
    loop {
        if let std::task::Poll::Ready(v) = f.as_mut().poll(&mut cx) {
            return v;
        }
        std::thread::yield_now();
    }
}

struct Gpu {
    adapter_name: String,
    device: wgpu::Device,
    queue: wgpu::Queue,
    comp: photocraft_gpu::Compositor,
}

/// Request the app's GPU limits, retaining adapter identity for reports; absence selects the CPU canvas.
fn gpu() -> Option<Gpu> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
    let adapter =
        block_on(instance.request_adapter(&wgpu::RequestAdapterOptions { power_preference: wgpu::PowerPreference::HighPerformance, ..Default::default() }))
            .ok()?;
    let adapter_name = adapter.get_info().name;
    eprintln!("adapter: {adapter_name}");
    // The limits the app requests (see `gpu_canvas::use_adapter_limits`).
    let limits = photocraft_ui_egui::gpu_canvas::device_limits(&adapter);
    let (device, queue) = block_on(adapter.request_device(&wgpu::DeviceDescriptor { required_limits: limits, ..Default::default() })).ok()?;
    let comp = photocraft_gpu::Compositor::new(&device);
    Some(Gpu { adapter_name, device, queue, comp })
}

/// A photo-like RGB image: smooth gradients, soft blobs, fine noise (so JPEG has real work).
fn photo(w: u32, h: u32) -> photocraft_codecs::Image {
    use rayon::prelude::*;
    let mut px = vec![0u8; w as usize * h as usize * 3];
    px.par_chunks_mut(w as usize * 3).enumerate().for_each(|(y, row)| {
        for x in 0..w as usize {
            let (fx, fy) = (x as f32 / w as f32, y as f32 / h as f32);
            let n = ((x.wrapping_mul(73_856_093) ^ y.wrapping_mul(19_349_663)) % 997) as f32 / 997.0 - 0.5;
            let blob = (-(((fx - 0.4) * 3.0).powi(2) + ((fy - 0.55) * 4.0).powi(2))).exp();
            let r = 0.55 + 0.35 * (fx * 6.0).sin() * 0.5 + 0.3 * blob + n * 0.06;
            let g = 0.45 + 0.25 * (fy * 9.0 + fx * 2.0).cos() * 0.5 + 0.2 * blob + n * 0.06;
            let b = 0.35 + 0.4 * fy - 0.2 * blob + n * 0.06;
            for (c, v) in [r, g, b].into_iter().enumerate() {
                row[x * 3 + c] = (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
            }
        }
    });
    photocraft_codecs::Image::from_raw(w, h, photocraft_codecs::ChannelLayout::Rgb, photocraft_codecs::SampleType::U8, px).expect("image")
}

/// JSON metadata for one benchmark row, in the same order as `Bench::rows`.
struct RowExtra {
    samples: Vec<f64>,
    peak_rss: Option<u64>,
    faults: Option<u64>,
    /// Any fallback among the warm samples takes precedence.
    warm_path: Option<&'static str>,
    cold_path: Option<&'static str>,
}

struct Bench {
    gpu: Option<Gpu>,
    reps: usize,
    /// `--only text`: time only rows whose name contains it (others run once, untimed, for the
    /// state later rows need).
    only: Option<String>,
    rows: Vec<(String, f64, f64, f64, f64)>,
    /// Per row (same order as `rows`): the metadata for `--json` (`cargo xtask perf`).
    extra: Vec<RowExtra>,
    rss: photocraft_testkit::perf::RssSampler,
}

impl Bench {
    /// Canvas refresh after an edit: the damage rect when the session reports one for this
    /// revision, else the whole document (what `canvas::ensure_gpu` does).
    fn refresh(&mut self, s: &Session, full: bool) -> f64 {
        let st = s.active().expect("document");
        let doc = st.doc.clone();
        let region = if full { doc.bounds() } else { st.last_damage.map_or(doc.bounds(), |r| r.inflate(reach(&doc.layers)).intersect(&doc.bounds())) };
        self.render(&doc, region)
    }

    /// GPU (or CPU fallback) composite of `region` of `doc`, waited on.
    fn render(&mut self, doc: &Document, region: Rect) -> f64 {
        self.render_preview(doc, region).0
    }

    /// Times `f` (`reps` runs after a cold one); `f` returns its own breakdown-free total in ms.
    fn time(&mut self, name: &str, mut f: impl FnMut(&mut Self) -> f64) {
        self.time_path(name, false, |b| (f(b), ""));
    }

    /// [`Self::time`] for a row that also reports the compositing path that drew it (`GPU`,
    /// `CPU fallback` or `CPU canvas`), so a fallback is never presented as a GPU result. The cold
    /// sample's path is kept beside the warm row's (see [`RowExtra`]).
    fn time_path(&mut self, name: &str, report_path: bool, mut f: impl FnMut(&mut Self) -> (f64, &'static str)) {
        if self.only.as_ref().is_some_and(|o| !name.contains(o.as_str())) {
            f(self);
            return;
        }
        self.rss.reset();
        let (cold, cold_path) = f(self);
        let faults0 = photocraft_testkit::perf::minor_faults();
        let mut v = Vec::with_capacity(self.reps);
        let mut path = "";
        let mut fallback = false;
        for _ in 0..self.reps {
            let (t, p) = f(self);
            fallback |= p == "CPU fallback";
            path = p;
            v.push(t);
        }
        if fallback {
            path = "CPU fallback";
        }
        // Minor page faults per warm run: memory mapped fresh from the kernel each time (#10479).
        // Counted over the whole of `f`, so it includes untimed setup and cleanup (an undo, an
        // extra refresh), not only the work the milliseconds measure.
        let faults = faults0.zip(photocraft_testkit::perf::minor_faults()).map(|(a, b)| b.saturating_sub(a) / self.reps as u64);
        let report_paths = report_path && !path.is_empty();
        let warm_path = report_paths.then_some(path);
        let cold_path = (report_path && !cold_path.is_empty()).then_some(cold_path);
        self.extra.push(RowExtra { samples: v.clone(), peak_rss: self.rss.peak(), faults, warm_path, cold_path });
        v.sort_by(f64::total_cmp);
        let med = v[v.len() / 2];
        let fl = faults.map_or(String::new(), |f| format!(", {f} faults/run incl. setup"));
        let cold_tag = cold_path.map_or(String::new(), |p| format!(" ({p})"));
        let shown = if report_paths { format!("{name} ({path})") } else { name.to_string() };
        println!("{shown:<44} {med:>9.1} ms   (min {:>8.1}, max {:>8.1}, cold {:>8.1}{cold_tag}{fl})", v[0], v[v.len() - 1], cold);
        self.rows.push((shown, med, v[0], v[v.len() - 1], cold));
    }

    /// Composite `region` of `doc`, wait for the GPU, and report which path drew it.
    fn render_preview(&mut self, doc: &Document, region: Rect) -> (f64, &'static str) {
        if region.is_empty() {
            return (0.0, "empty region");
        }
        let t = Instant::now();
        let path = match &mut self.gpu {
            Some(g) => {
                let ok = g.comp.render(&g.device, &g.queue, doc, region, |_, _| {}).is_ok();
                if !ok {
                    std::hint::black_box(photocraft_compose::render(doc, region));
                }
                let _ = g.device.poll(wgpu::PollType::Wait { submission_index: None, timeout: None });
                if ok { "GPU" } else { "CPU fallback" }
            }
            None => {
                std::hint::black_box(photocraft_compose::render(doc, region));
                "CPU canvas"
            }
        };
        (ms(t), path)
    }
}

/// How far an edit's composite change reaches beyond its damage (layer effects).
fn reach(layers: &[photocraft_doc::Layer]) -> i32 {
    layers
        .iter()
        .map(|l| {
            let own = if photocraft_compose::effects::has_effects(l) { photocraft_compose::effects::margin(l) } else { 0 };
            let kids = match &l.content {
                photocraft_doc::LayerContent::Group(g) => reach(&g.children),
                _ => 0,
            };
            own + kids
        })
        .max()
        .unwrap_or(0)
}

fn select(s: &mut Session, index: usize) {
    let id = s.active().expect("doc").doc.layers[index].id;
    exec(s, "layer.select", json!({"layer": id.0}));
}

fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1000.0
}

fn exec(s: &mut Session, id: &str, p: Value) {
    if let Err(e) = s.execute(id, p) {
        panic!("{id}: {e}");
    }
}

/// A 36 MP RGB/8 document with a clipped raster target in a real clipping stack: a base, a
/// clipped sibling below, the clipped target and a clipped sibling above. `styled` gives the
/// target a Color Overlay: that makes it ineligible for the layer preview (the preview's styled
/// group would rebuild its effect shape maps on the UI thread), so the benchmark reports only
/// its CPU reference rows (#7487).
fn clipped_document(w: u32, h: u32, styled: bool) -> (Document, LayerId) {
    let mut doc = Document::new("clipped preview", Size::new(w, h), ColorMode::Rgb, SampleType::U8);
    let fmt = doc.pixel_format();
    let (wi, hi) = (w as i32, h as i32);
    let region = Rect::from_xywh(wi / 4, hi / 4, w / 2, h / 2);
    let band = Rect::from_xywh(region.x0, region.y0, region.width(), region.height() / 3);
    let mut base = Layer::raster("clip base", fmt);
    if let Some(s) = base.surface_mut() {
        s.fill_rect(doc.bounds(), &[0.42, 0.5, 0.58, 1.0]);
        s.fill_rect(Rect::from_xywh(0, 0, w / 2, h), &[0.3, 0.36, 0.44, 1.0]);
    }
    base.opacity = 0.9;
    let mut below = Layer::raster("clip sibling below", fmt);
    if let Some(s) = below.surface_mut() {
        s.fill_rect(band, &[0.2, 0.7, 0.4, 0.6]);
    }
    below.clipped = true;
    below.opacity = 0.5;
    let mut target = Layer::raster("clipped target", fmt);
    if let Some(s) = target.surface_mut() {
        s.fill_rect(region, &[0.8, 0.3, 0.2, 0.9]);
        s.fill_rect(Rect::from_xywh(region.x0, region.y0, region.width() / 3, region.height() / 2), &[0.1, 0.2, 0.9, 1.0]);
    }
    target.clipped = true;
    target.opacity = 0.8;
    target.blend = BlendMode::Multiply;
    if styled {
        target.effects.items.push(Effect::ColorOverlay { common: FxCommon::new(BlendMode::Normal, 0.5), color: Color::rgba(0.2, 0.8, 0.4, 1.0) });
    }
    let id = target.id;
    let mut above = Layer::raster("clip sibling above", fmt);
    if let Some(s) = above.surface_mut() {
        s.fill_rect(band, &[0.5, 0.5, 0.9, 0.4]);
    }
    above.clipped = true;
    above.opacity = 0.6;
    doc.layers = vec![base, below, target, above];
    (doc, id)
}

/// The old CPU path builds an adjusted proxy and flattens the whole document for each change.
fn cpu_adjust_preview_case(b: &mut Bench, doc: &Document, target: LayerId, name: &str, k: u32) {
    let mut v = 0;
    b.time(name, |_| {
        v = (v + 7) % 120;
        let t = Instant::now();
        let p = photocraft_ui_egui::filter_dialog::preview_document(
            doc,
            Some(target),
            "image.adjustments.curves",
            &json!({"points": [[0, 0], [100, 100 + v], [255, 255]]}),
            k,
        )
        .expect("preview");
        std::hint::black_box(photocraft_compose::flatten(&p));
        ms(t)
    });
}

/// The CPU reference rows: the old proxy at the desktop factor, and (for path-reported rows) at
/// the 100 % and zoomed-out resolutions the canvas would use, so the layer path compares fairly.
fn cpu_preview_rows(b: &mut Bench, doc: &Document, target: LayerId, name: &str, sel: &str, report_path: bool) {
    cpu_adjust_preview_case(b, doc, target, &format!("{name}, CPU proxy preview{sel}"), photocraft_ui_egui::proxy::factor(doc));
    if report_path {
        let zoom = 1600.0 / doc.size.width as f32 * 2.0;
        cpu_adjust_preview_case(b, doc, target, &format!("{name}, CPU proxy preview 100 % view{sel}"), 1);
        cpu_adjust_preview_case(
            b,
            doc,
            target,
            &format!("{name}, CPU proxy preview zoomed out{sel}"),
            photocraft_ui_egui::proxy::view_factor(photocraft_ui_egui::proxy::factor(doc), zoom),
        );
    }
}

/// Compare CPU and layer previews, keeping session setup outside the settings-change samples.
fn adjust_preview_case(b: &mut Bench, s: &Session, target: LayerId, name: &str, sel: &str, report_path: bool) {
    let doc = &s.active().expect("doc").doc;
    let (w, h) = (doc.size.width, doc.size.height);
    let zoom = 1600.0 / w as f32 * 2.0;
    let k = photocraft_ui_egui::adjust_preview::proxy_factor(zoom);
    if let Some(why) = photocraft_ui_egui::adjust_preview::unsupported(doc, target) {
        println!("  ({name}{sel}: no layer preview: {why}; CPU reference rows only)");
        cpu_preview_rows(b, doc, target, name, sel, report_path);
        return;
    }
    cpu_preview_rows(b, doc, target, name, sel, report_path);
    // Once per dialog session (the app caches both per document revision).
    let (base, region) = {
        let st = s.active().expect("doc");
        let t = Instant::now();
        let base = photocraft_ui_egui::adjust_preview::base_document(&st.doc, target).expect("base");
        let region = photocraft_ui_egui::adjust_preview::region(&st.doc, target);
        println!("  ({name}{sel}: layer preview session setup: {:.1} ms, region {region:?})", ms(t));
        (base, region)
    };
    let mut v = 0;
    b.time_path(&format!("{name}, layer preview{sel}"), report_path, |b| {
        v = (v + 7) % 120;
        let t = Instant::now();
        let p = photocraft_ui_egui::adjust_preview::with_settings(&base, "curves", &json!({"points": [[0, 0], [100, 100 + v], [255, 255]]})).expect("preview");
        let build = ms(t);
        let (render, path) = b.render_preview(&p, region);
        (build + render, path)
    });
    // Zoomed in, the canvas composites the region's visible part (here a 2560×1440 view at
    // 100 %) and catches up elsewhere when the view moves.
    let view = Rect::from_xywh(w as i32 / 3, h as i32 / 3, 2560, 1440).intersect(&region);
    let mut v = 0;
    b.time_path(&format!("{name}, layer preview 100 % view{sel}"), report_path, |b| {
        v = (v + 7) % 120;
        let t = Instant::now();
        let p = photocraft_ui_egui::adjust_preview::with_settings(&base, "curves", &json!({"points": [[0, 0], [100, 100 + v], [255, 255]]})).expect("preview");
        let build = ms(t);
        let (render, path) = b.render_preview(&p, view);
        (build + render, path)
    });
    // Zoomed out (fit to a 1600-pixel-wide view on a 2× display: k = 2 for 36 MP), the canvas
    // previews on a reduced copy built once per session, unless effects or vector masks make
    // that proxy inaccurate (the same eligibility check as adjust_preview::gpu_proxy).
    let k = if photocraft_compose::proxy::proxy_faithful(&base) { k } else { 1 };
    let t = Instant::now();
    let proxy = photocraft_ui_egui::adjust_preview::proxy_base(&base, k);
    println!("  ({name}{sel}: zoomed-out proxy k {k} setup: {:.1} ms)", ms(t));
    let kk = k as i32;
    let pregion = Rect::new(region.x0 / kk, region.y0 / kk, region.x1 / kk + 1, region.y1 / kk + 1).intersect(&proxy.bounds());
    let mut v = 0;
    b.time_path(&format!("{name}, layer preview zoomed out{sel}"), report_path, |b| {
        v = (v + 7) % 120;
        let t = Instant::now();
        let p = photocraft_ui_egui::adjust_preview::proxy_with_settings(&proxy, "curves", &json!({"points": [[0, 0], [100, 100 + v], [255, 255]]}))
            .expect("preview");
        let build = ms(t);
        let (render, path) = b.render_preview(&p, pregion);
        (build + render, path)
    });
}

/// The independent clipped-preview fixtures: unstyled (layer/GPU path) and styled (CPU fallback).
/// Both use their own session, so `--only clipped` never touches the big photo, PSD or gradient
/// setup and the full run keeps its existing row order.
fn clipped_preview_group(b: &mut Bench, w: u32, h: u32) {
    for (styled, label) in [(false, "clipped target"), (true, "clipped styled target")] {
        let t = Instant::now();
        let (doc, target) = clipped_document(w, h, styled);
        let mut cs = Session::new();
        cs.open_document(doc, None);
        exec(&mut cs, "layer.select", json!({"layer": target.0}));
        println!("  ({label} fixture setup: {:.1} ms)", ms(t));
        for selection in [false, true] {
            if selection {
                exec(&mut cs, "select.rect", json!({"x": w / 4, "y": h / 4, "width": w / 2, "height": h / 2, "ellipse": true}));
            }
            let sel = if selection { ", selection" } else { "" };
            adjust_preview_case(b, &cs, target, &format!("Curves dialog change, {label}"), sel, true);
            if selection {
                exec(&mut cs, "select.deselect", json!({}));
            }
        }
    }
}

/// Write the `--json` report of the rows measured so far (shared by the full run and the early
/// `--only clipped` path).
fn write_report(b: &Bench, args: &[String], w: u32, h: u32) {
    let Some(out) = arg(args, "--json") else { return };
    let rows: Vec<Value> = b
        .rows
        .iter()
        .zip(&b.extra)
        .map(|((n, med, min, max, cold), extra)| {
            let mut r = json!({"name": n, "median_ms": med, "min_ms": min, "max_ms": max, "cold_ms": cold});
            // For `cargo xtask perf`: raw samples (it computes p50 / p95) and peak RSS.
            r["samples_ms"] = json!(&extra.samples);
            r["peak_rss_bytes"] = json!(extra.peak_rss);
            r["minor_faults_per_run"] = json!(extra.faults);
            if let Some(p) = extra.warm_path {
                r["warm_path"] = json!(p);
            }
            if let Some(p) = extra.cold_path {
                r["cold_path"] = json!(p);
            }
            r
        })
        .collect();
    let context = json!({"gpu_adapter": b.gpu.as_ref().map(|g| g.adapter_name.as_str())});
    let report = json!({"width": w, "height": h, "gpu": b.gpu.is_some(), "context": context, "rows": rows});
    std::fs::write(&out, serde_json::to_string_pretty(&report).unwrap()).expect("write json");
}

/// Run the selected benchmarks, handling exact `--only clipped` before unrelated fixture setup.
fn main() {
    let args: Vec<String> = std::env::args().collect();
    let (w, h) =
        arg(&args, "--size").and_then(|s| s.split_once('x').map(|(a, b)| (a.parse().unwrap_or(7360), b.parse().unwrap_or(4912)))).unwrap_or((7360, 4912));
    let reps: usize = arg(&args, "--reps").and_then(|v| v.parse().ok()).unwrap_or(5).max(1);
    let mut b = Bench {
        gpu: if args.iter().any(|a| a == "--cpu") { None } else { gpu() },
        reps,
        only: arg(&args, "--only"),
        rows: Vec::new(),
        extra: Vec::new(),
        rss: photocraft_testkit::perf::RssSampler::start(std::time::Duration::from_millis(2)),
    };
    println!("document {w}×{h} ({:.1} MP), 8-bit RGB, {} reps, {}", (w * h) as f64 / 1e6, reps, if b.gpu.is_some() { "GPU canvas" } else { "CPU canvas" });

    // `--only clipped` measures just the independent clipped fixtures, without building the
    // 36 MP photo, encoding JPEG, or opening/exporting PSD. That keeps the run's retained
    // memory lower and avoids unrelated rows that `--only` would otherwise execute once.
    if arg(&args, "--only").as_deref() == Some("clipped") {
        clipped_preview_group(&mut b, w, h);
        write_report(&b, &args, w, h);
        return;
    }

    // ---- open / save / export ----------------------------------------------------------------
    let img = photo(w, h);
    let mut jpeg = Vec::new();
    b.time("encode JPEG (codecs, q 90)", |_| {
        let t = Instant::now();
        jpeg = photocraft_codecs::encode(&img, photocraft_codecs::Format::Jpeg, &Default::default()).expect("jpeg");
        ms(t)
    });
    let mut s = Session::new();
    b.time("open JPEG (decode + document + refresh)", |b| {
        let t = Instant::now();
        let doc = photocraft_io::import("photo.jpg", &jpeg).expect("import").document;
        s = Session::new();
        s.open_document(doc, Some("photo.jpg".into()));
        let t = ms(t);
        t + b.refresh(&s, true)
    });
    // A working document: the photo, a painting layer above it, an adjustment on top later.
    exec(&mut s, "layer.new.layer", json!({"name": "paint"}));
    let doc = |s: &Session| -> std::sync::Arc<Document> { s.active().expect("doc").doc.clone() };
    b.time("full refresh (2 layers)", |b| b.refresh(&s, true));
    b.time("viewport 1920×1080 (CPU compose)", |_| {
        let d = doc(&s);
        let t = Instant::now();
        std::hint::black_box(photocraft_compose::render(&d, Rect::from_xywh(w as i32 / 3, h as i32 / 3, 1920, 1080)));
        ms(t)
    });

    // ---- painting --------------------------------------------------------------------------
    let mut k = 0;
    b.time("brush dab 60 px (command + refresh)", |b| {
        k += 1;
        let (x, y) = (200 + (k * 97) % (w as i32 - 400), 200 + (k * 61) % (h as i32 - 400));
        let t = Instant::now();
        exec(&mut s, "paint.stroke", json!({"points": [[x, y]], "size": 60, "hardness": 0.8, "color": "#c03020"}));
        ms(t) + b.refresh(&s, false)
    });
    b.time("brush stroke 40 dabs 120 px", |b| {
        k += 1;
        let (x, y) = (300 + (k * 131) % (w as i32 - 2000), 300 + (k * 71) % (h as i32 - 600));
        let pts: Vec<Value> = (0..40).map(|i| json!([x + i * 30, y + ((i as f32 * 0.4).sin() * 80.0) as i32])).collect();
        let t = Instant::now();
        exec(&mut s, "paint.stroke", json!({"points": pts, "size": 120, "hardness": 0.5, "color": "#2040c0", "spacing": 0.25}));
        ms(t) + b.refresh(&s, false)
    });
    b.time("undo (stroke)", |b| {
        exec(&mut s, "paint.stroke", json!({"points": [[500, 500], [900, 700]], "size": 80, "color": "#20a040"}));
        let t = Instant::now();
        exec(&mut s, "edit.undo", json!({}));
        ms(t) + b.refresh(&s, false)
    });

    // ---- filters / adjustments / layers ------------------------------------------------------
    select(&mut s, 0);
    for radius in [4.0, 40.0, 250.0] {
        b.time(&format!("Gaussian Blur r {radius} (photo layer) + refresh"), |b| {
            let t = Instant::now();
            exec(&mut s, "filter.blur.gaussianBlur", json!({"radius": radius}));
            let v = ms(t) + b.refresh(&s, false);
            exec(&mut s, "edit.undo", json!({}));
            b.refresh(&s, false);
            v
        });
    }
    // Filter dialog live preview, per radius change (#464): the command on the proxy plus its
    // flatten for upload.
    for radius in [40.0, 250.0] {
        b.time(&format!("Gaussian Blur dialog preview r {radius}"), |_| {
            let st = s.active().expect("doc");
            let k = photocraft_ui_egui::proxy::factor(&st.doc);
            let t = Instant::now();
            let p = photocraft_ui_egui::filter_dialog::preview_document(&st.doc, st.active_layer, "filter.blur.gaussianBlur", &json!({"radius": radius}), k)
                .expect("preview");
            std::hint::black_box(photocraft_compose::flatten(&p));
            ms(t)
        });
    }
    // Image › Adjustments dialog preview, per settings change (#77). Before: the command re-run
    // on a downsampled proxy (plus its flatten for upload). After: the document with a temporary
    // clipped adjustment layer, recomposited over the target's area by the canvas compositor.
    for selection in [false, true] {
        if selection {
            exec(&mut s, "select.rect", json!({"x": w / 4, "y": h / 4, "width": w / 2, "height": h / 2, "ellipse": true}));
        }
        let sel = if selection { ", selection" } else { "" };
        let target = s.active().expect("doc").active_layer.expect("layer");
        adjust_preview_case(&mut b, &s, target, "Curves dialog change", sel, false);
        if selection {
            exec(&mut s, "select.deselect", json!({}));
        }
    }
    // The same case on a clipped target (#7487): the preview isolates it in a group. A styled
    // target keeps the CPU proxy (see `adjust_preview_case`), so only its CPU rows are measured.
    clipped_preview_group(&mut b, w, h);
    exec(&mut s, "layer.newAdjustmentLayer.levels", json!({"inBlack": 10, "inWhite": 240, "gamma": 1.1}));
    let mut g = 1.0;
    b.time("Levels layer tweak (gamma) + full refresh", |b| {
        g = if g > 1.5 { 0.8 } else { g + 0.07 };
        let t = Instant::now();
        exec(&mut s, "layer.setAdjustment", json!({"gamma": g}));
        ms(t) + b.refresh(&s, false)
    });
    select(&mut s, 1);
    let mut d = 1;
    b.time("move paint layer 10 px + refresh", |b| {
        d = -d;
        let t = Instant::now();
        exec(&mut s, "layer.translate", json!({"dx": 10 * d, "dy": 5 * d}));
        ms(t) + b.refresh(&s, false)
    });
    b.time("undo (move)", |b| {
        exec(&mut s, "layer.translate", json!({"dx": 7, "dy": 3}));
        let t = Instant::now();
        exec(&mut s, "edit.undo", json!({}));
        ms(t) + b.refresh(&s, false)
    });

    // ---- save / export ---------------------------------------------------------------------
    let mut psd = Vec::new();
    b.time("save .pcraft (bytes, full)", |_| {
        let d = doc(&s);
        let t = Instant::now();
        std::hint::black_box(photocraft_format::save_to_bytes(&d, &photocraft_format::SaveOptions::default()).expect("pcraft"));
        ms(t)
    });
    b.time("save .psd", |_| {
        let d = doc(&s);
        let t = Instant::now();
        psd = photocraft_io::export(&d, "x.psd", &Default::default()).expect("psd").bytes;
        ms(t)
    });
    b.time("open .psd (3 layers) + refresh", |b| {
        let t = Instant::now();
        let d = photocraft_io::import("x.psd", &psd).expect("psd").document;
        let mut s2 = Session::new();
        s2.open_document(d, None);
        ms(t) + b.refresh(&s2, true)
    });
    b.time("flatten (CPU compositor, 3 layers)", |_| {
        let d = doc(&s);
        let t = Instant::now();
        std::hint::black_box(photocraft_compose::flatten(&d));
        ms(t)
    });
    b.time("export PNG (flatten + encode)", |_| {
        let d = doc(&s);
        let t = Instant::now();
        std::hint::black_box(photocraft_io::export(&d, "x.png", &Default::default()).expect("png"));
        ms(t)
    });
    b.time("export JPEG (flatten + encode)", |_| {
        let d = doc(&s);
        let t = Instant::now();
        std::hint::black_box(photocraft_io::export(&d, "x.jpg", &Default::default()).expect("jpg"));
        ms(t)
    });

    // ---- live gradient ------------------------------------------------------------------------
    // A Gradient tool drag in live mode: each pointer move previews a shallow copy of the document
    // with the edited fill (gradient_ui::display_doc, through gradient_fill_cmds::apply_set) and
    // recomposites the whole canvas, since the fill covers it. Release commits one command.
    exec(&mut s, "gradient.fill.create", json!({"from": [w as f32 * 0.2, h as f32 * 0.5], "to": [w as f32 * 0.8, h as f32 * 0.5]}));
    let gid = s.active().and_then(|st| st.active_layer).expect("gradient layer");
    let mut k = 0;
    b.time("live gradient preview doc (no refresh)", |_| {
        k += 1;
        let to = [w as f32 * (0.6 + 0.03 * (k % 10) as f32), h as f32 * (0.3 + 0.04 * (k % 7) as f32)];
        let t = Instant::now();
        let d = doc(&s);
        let l = d.layer(gid).expect("layer");
        let LayerContent::Fill(f) = &l.content else { panic!("not a fill") };
        let f = photocraft_engine::gradient_fill_cmds::apply_set(l, f, d.bounds(), &json!({"to": to}), [0.0, 0.0, 0.0, 1.0], [1.0; 4]).expect("set");
        let mut shown = (*d).clone();
        shown.layer_mut(gid).expect("layer").content = LayerContent::Fill(f);
        std::hint::black_box(&shown);
        ms(t)
    });
    b.time("live gradient drag update (preview + full refresh)", |b| {
        k += 1;
        let to = [w as f32 * (0.6 + 0.03 * (k % 10) as f32), h as f32 * (0.3 + 0.04 * (k % 7) as f32)];
        let t = Instant::now();
        let d = doc(&s);
        let l = d.layer(gid).expect("layer");
        let LayerContent::Fill(f) = &l.content else { panic!("not a fill") };
        let f = photocraft_engine::gradient_fill_cmds::apply_set(l, f, d.bounds(), &json!({"to": to}), [0.0, 0.0, 0.0, 1.0], [1.0; 4]).expect("set");
        let mut shown = (*d).clone();
        shown.layer_mut(gid).expect("layer").content = LayerContent::Fill(f);
        ms(t) + b.render(&shown, shown.bounds())
    });
    b.time("live gradient release (gradient.fill.set + refresh)", |b| {
        k += 1;
        let t = Instant::now();
        exec(&mut s, "gradient.fill.set", json!({"layer": gid.0, "to": [w as f32 * (0.6 + 0.03 * (k % 10) as f32), h as f32 * 0.4]}));
        ms(t) + b.refresh(&s, false)
    });

    write_report(&b, &args, w, h);
}

/// The `--json` reporting path, exercised with fake timings: CI runs integration tests, not
/// examples, so these live here and are included by `tests/it/main.rs`.
#[cfg(test)]
mod reporting_tests {
    use super::*;

    fn bench(reps: usize) -> Bench {
        Bench {
            gpu: None,
            reps,
            only: None,
            rows: Vec::new(),
            extra: Vec::new(),
            rss: photocraft_testkit::perf::RssSampler::start(std::time::Duration::from_millis(2)),
        }
    }

    /// Write `b`'s report to a temporary file and return its rows.
    fn report_rows(b: &Bench) -> Vec<Value> {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("report.json");
        let args = ["--json".to_string(), path.to_string_lossy().into_owned()];
        write_report(b, &args, 40, 32);
        let text = std::fs::read_to_string(&path).expect("read report");
        serde_json::from_str::<Value>(&text).expect("report json")["rows"].as_array().expect("report rows").clone()
    }

    /// A cold CPU fallback must not relabel GPU warm samples or sort their raw timings.
    #[test]
    fn a_cold_cpu_fallback_keeps_the_warm_gpu_path() {
        let mut b = bench(3);
        let mut call = 0;
        b.time_path("row", true, |_| {
            call += 1;
            match call {
                1 => (10.0, "CPU fallback"),
                2 => (24.0, "GPU"),
                3 => (22.0, "GPU"),
                _ => (23.0, "GPU"),
            }
        });
        let rows = report_rows(&b);
        let row = &rows[0];
        assert_eq!(row["name"], "row (GPU)");
        assert_eq!(row["median_ms"], 23.0);
        assert_eq!(row["min_ms"], 22.0);
        assert_eq!(row["max_ms"], 24.0);
        assert_eq!(row["cold_ms"], 10.0);
        assert_eq!(row["cold_path"], "CPU fallback");
        assert_eq!(row["warm_path"], "GPU");
        assert_eq!(row["samples_ms"], json!([24.0, 22.0, 23.0]));
    }

    /// Retain a warm fallback after recovery without relabeling the cold sample.
    #[test]
    fn any_warm_fallback_labels_the_warm_row_and_keeps_the_cold_path() {
        let mut b = bench(3);
        let mut call = 0;
        b.time_path("row", true, |_| {
            call += 1;
            match call {
                1 => (5.0, "GPU"),
                2 => (20.0, "GPU"),
                3 => (21.0, "CPU fallback"),
                _ => (22.0, "GPU"),
            }
        });
        let rows = report_rows(&b);
        let row = &rows[0];
        assert_eq!(row["name"], "row (CPU fallback)");
        assert_eq!(row["median_ms"], 21.0);
        assert_eq!(row["cold_ms"], 5.0);
        assert_eq!(row["cold_path"], "GPU");
        assert_eq!(row["warm_path"], "CPU fallback");
        assert_eq!(row["samples_ms"], json!([20.0, 21.0, 22.0]));
    }

    /// Preserve the legacy row schema and sample alignment when rendering paths are not requested.
    #[test]
    fn ordinary_rows_keep_their_names_and_have_no_path_metadata() {
        let mut b = bench(2);
        b.time("plain row", |_| 4.0);
        b.time("other row", |_| 6.0);
        let rows = report_rows(&b);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0]["name"], "plain row");
        assert_eq!(rows[0]["median_ms"], 4.0);
        assert_eq!(rows[0]["cold_ms"], 4.0);
        assert_eq!(rows[0]["samples_ms"], json!([4.0, 4.0]));
        for key in ["median_ms", "min_ms", "max_ms", "cold_ms", "samples_ms", "peak_rss_bytes", "minor_faults_per_run"] {
            assert!(rows[0].get(key).is_some(), "{key} missing: {}", rows[0]);
        }
        assert!(rows[0].get("warm_path").is_none(), "{}", rows[0]);
        assert!(rows[0].get("cold_path").is_none(), "{}", rows[0]);
        assert_eq!(rows[1]["name"], "other row");
        assert_eq!(rows[1]["min_ms"], 6.0);
        assert_eq!(rows[1]["cold_ms"], 6.0);
        assert!(rows[1].get("warm_path").is_none(), "{}", rows[1]);
        assert!(rows[1].get("cold_path").is_none(), "{}", rows[1]);
    }
}
