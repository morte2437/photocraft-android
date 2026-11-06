//! Flat-image open benchmark (#10259): File › Open of an ordinary JPEG or PNG, phase by phase,
//! with the peak resident memory of each case.
//!
//! ```sh
//! cargo run --release -p photocraft-ui-egui --example flat_open_bench -- [--quick] [--reps 5] [--only jpeg-36] [--cpu] [--cache DIR] [--json out.json]
//! cargo run --release -p photocraft-ui-egui --example flat_open_bench -- --case png16-36   # one case, one process
//! ```
//!
//! Cases (`--only` matches a substring of the key): `jpeg-{24,36}` (RGB, quality 90, 4:2:0),
//! `png8-{24,36}` (RGB 8-bit), `png16-{24,36}` (RGB 16-bit), `rgba8-{24,36}` (RGB 8-bit with
//! partial alpha) and `jpeg-24-orient6` (EXIF orientation 6). 24 MP is 6000x4000, 36 MP is
//! 7360x4912; `--quick` uses 1200x800 for every case (the row names keep the nominal size).
//!
//! The inputs are generated in-process (smooth gradients, soft blobs and seeded noise, so the
//! compression is realistic) with the workspace's own encoders and cached in `--cache` (default:
//! `$PHOTOCRAFT_FIXTURE_DIR`, else `fixtures/` in the target directory), never committed.
//!
//! Rows per case, in ms over `--reps` repetitions:
//! - `import`: `import_for_open_with`, the real File › Open path (decode, orientation, tiles)
//! - `admission`: `Session::open_document` of that document
//! - `decode`: the codec alone, orientation kept as stored
//! - `orient`: `Image::oriented` with the file's EXIF orientation (a no-op for an upright file)
//! - `tiles`: `image_to_document` on the decoded image (layout conversion, tile construction, prune)
//! - `first composite`: the canvas's first full refresh (GPU, or the CPU compositor with `--cpu`
//!   or without an adapter), as `perf_scenarios` times it
//! - `editable`: import + admission + first composite of one open, the "decode + first composite" rows
//!
//! Peak memory: without `--case` the bench starts itself once per case (`current_exe`), so each
//! case's `VmHWM` is its own process-lifetime high-water mark. `peak_process_after_import_bytes`
//! is read after real import and admission; `peak_process_after_all_passes_bytes` is read after
//! standalone decode/orient/tiles, GPU setup and warm-up, and editable-open passes. The latter
//! is not an isolated composite or editable-open peak. Row `peak_rss_provenance` identifies
//! which reading its `peak_rss_bytes` carries. Off Linux the peak is `n/a`.
//! A case run directly with `--case` generates a missing fixture in its own process, which
//! raises its peak: run the bench once without `--case` first.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Instant;

use eframe::egui_wgpu::RenderState;
use photocraft_codecs::{ChannelLayout, DecodeOptions, EncodeOptions, Format, Image};
use photocraft_engine::Session;
use photocraft_ui_egui::gpu_canvas::GpuCanvas;
use rayon::prelude::*;
use serde_json::{Value, json};

/// Prefix of the line a `--case` process prints for its parent.
const RESULT_PREFIX: &str = "FLAT_OPEN_RESULT ";
/// The phases, in report order.
const PHASES: [&str; 7] = ["import", "admission", "decode", "orient", "tiles", "first composite", "editable"];

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Jpeg,
    Png8,
    Png16,
    Rgba8,
}

struct Case {
    key: &'static str,
    label: &'static str,
    kind: Kind,
    /// 36 MP (7360x4912) instead of 24 MP (6000x4000).
    big: bool,
    /// EXIF orientation 6 (the stored image is turned upright on open).
    orient6: bool,
}

const CASES: [Case; 9] = [
    Case { key: "jpeg-24", label: "open 24 MP JPEG RGB8", kind: Kind::Jpeg, big: false, orient6: false },
    Case { key: "jpeg-36", label: "open 36 MP JPEG RGB8", kind: Kind::Jpeg, big: true, orient6: false },
    Case { key: "png8-24", label: "open 24 MP PNG RGB8", kind: Kind::Png8, big: false, orient6: false },
    Case { key: "png8-36", label: "open 36 MP PNG RGB8", kind: Kind::Png8, big: true, orient6: false },
    Case { key: "png16-24", label: "open 24 MP PNG RGB16", kind: Kind::Png16, big: false, orient6: false },
    Case { key: "png16-36", label: "open 36 MP PNG RGB16", kind: Kind::Png16, big: true, orient6: false },
    Case { key: "rgba8-24", label: "open 24 MP PNG RGBA8", kind: Kind::Rgba8, big: false, orient6: false },
    Case { key: "rgba8-36", label: "open 36 MP PNG RGBA8", kind: Kind::Rgba8, big: true, orient6: false },
    Case { key: "jpeg-24-orient6", label: "open 24 MP JPEG RGB8 (EXIF orientation 6)", kind: Kind::Jpeg, big: false, orient6: true },
];

impl Case {
    fn size(&self, quick: bool) -> (u32, u32) {
        match (quick, self.big) {
            (true, _) => (1200, 800),
            (false, false) => (6000, 4000),
            (false, true) => (7360, 4912),
        }
    }
    fn ext(&self) -> &'static str {
        if self.kind == Kind::Jpeg { "jpg" } else { "png" }
    }
    fn file_name(&self, w: u32, h: u32) -> String {
        format!("flat_open_{}_{w}x{h}.{}", self.key, self.ext())
    }
}

/// Return the value following a named command-line option.
fn arg(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1).cloned())
}

fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1000.0
}

/// `<target>/fixtures`: the binary lives in `<target>/release/examples/`.
fn default_cache() -> PathBuf {
    if let Some(d) = std::env::var_os("PHOTOCRAFT_FIXTURE_DIR") {
        return PathBuf::from(d);
    }
    std::env::current_exe().ok().and_then(|e| e.ancestors().nth(3).map(|t| t.join("fixtures"))).unwrap_or_else(std::env::temp_dir)
}

/// Deterministic noise in [-0.5, 0.5) from the pixel position and a channel/seed.
fn noise(x: usize, y: usize, seed: usize) -> f32 {
    let mut h = (x.wrapping_mul(73_856_093) ^ y.wrapping_mul(19_349_663) ^ seed.wrapping_mul(83_492_791)) as u64;
    h = (h ^ (h >> 15)).wrapping_mul(0x2c1b_3c6d);
    h = (h ^ (h >> 12)).wrapping_mul(0x297a_2d39);
    ((h ^ (h >> 15)) % 4096) as f32 / 4096.0 - 0.5
}

/// A photo-like sample in [0, 1] for channel `c` (3 is alpha: opaque at the centre, down to
/// about 15 % at the corners, with fine noise, so most pixels are partly transparent).
fn sample(x: usize, y: usize, w: usize, h: usize, c: usize) -> f32 {
    let (fx, fy) = (x as f32 / w as f32, y as f32 / h as f32);
    let n = noise(x, y, c);
    let blob = (-(((fx - 0.4) * 3.0).powi(2) + ((fy - 0.55) * 4.0).powi(2))).exp();
    let v = match c {
        0 => 0.55 + 0.17 * (fx * 6.0).sin() + 0.3 * blob + n * 0.06,
        1 => 0.45 + 0.12 * (fy * 9.0 + fx * 2.0).cos() + 0.2 * blob + n * 0.06,
        2 => 0.35 + 0.4 * fy - 0.2 * blob + n * 0.06,
        _ => 1.0 - 0.85 * (((fx - 0.5).powi(2) + (fy - 0.5).powi(2)) * 2.0).min(1.0) + n * 0.04,
    };
    v.clamp(0.0, 1.0)
}

/// A TIFF-structured EXIF block holding only the orientation tag.
fn exif_orientation_block(o: u16) -> Vec<u8> {
    let mut b = b"II*\0\x08\0\0\0\x01\0".to_vec();
    b.extend_from_slice(&[0x12, 0x01, 3, 0, 1, 0, 0, 0]);
    b.extend_from_slice(&o.to_le_bytes());
    b.extend_from_slice(&[0, 0, 0, 0, 0, 0]);
    b
}

/// The encoded file for `case` at `w`x`h`.
fn generate(case: &Case, w: u32, h: u32) -> Result<Vec<u8>, String> {
    let (wu, hu) = (w as usize, h as usize);
    let (layout, ch) = if case.kind == Kind::Rgba8 { (ChannelLayout::Rgba, 4) } else { (ChannelLayout::Rgb, 3) };
    let img = if case.kind == Kind::Png16 {
        let mut px = vec![0u16; wu * hu * ch];
        px.par_chunks_mut(wu * ch).enumerate().for_each(|(y, row)| {
            for (i, v) in row.iter_mut().enumerate() {
                *v = (sample(i / ch, y, wu, hu, i % ch) * 65535.0 + 0.5) as u16;
            }
        });
        Image::from_u16(w, h, layout, &px).map_err(|e| e.to_string())?
    } else {
        let mut px = vec![0u8; wu * hu * ch];
        px.par_chunks_mut(wu * ch).enumerate().for_each(|(y, row)| {
            for (i, v) in row.iter_mut().enumerate() {
                *v = (sample(i / ch, y, wu, hu, i % ch) * 255.0 + 0.5) as u8;
            }
        });
        Image::from_u8(w, h, layout, px).map_err(|e| e.to_string())?
    };
    let format = if case.kind == Kind::Jpeg { Format::Jpeg } else { Format::Png };
    let opts = EncodeOptions { jpeg_quality: 90, jpeg_chroma_subsampling: true, ..Default::default() };
    let mut bytes = photocraft_codecs::encode(&img, format, &opts).map_err(|e| e.to_string())?;
    if case.orient6 {
        // The encoder writes the pixels as given and resets the orientation, so the tag goes in
        // as an Exif APP1 segment right after the JPEG's SOI marker.
        let exif = exif_orientation_block(6);
        let len = u16::try_from(exif.len() + 8).map_err(|e| e.to_string())?;
        let mut seg = vec![0xFF, 0xE1];
        seg.extend_from_slice(&len.to_be_bytes());
        seg.extend_from_slice(b"Exif\0\0");
        seg.extend_from_slice(&exif);
        bytes.splice(2..2, seg);
    }
    Ok(bytes)
}

/// The cached fixture for `case`, generated (and written atomically) when missing.
fn ensure_fixture(case: &Case, dir: &Path, quick: bool) -> Result<PathBuf, String> {
    let (w, h) = case.size(quick);
    let path = dir.join(case.file_name(w, h));
    if path.metadata().is_ok_and(|m| m.len() > 0) {
        return Ok(path);
    }
    std::fs::create_dir_all(dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
    let t = Instant::now();
    let bytes = generate(case, w, h).map_err(|e| format!("generate {}: {e}", case.key))?;
    let tmp = path.with_extension(format!("{}.tmp{}", case.ext(), std::process::id()));
    std::fs::write(&tmp, &bytes).map_err(|e| format!("write {}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, &path).map_err(|e| format!("rename {}: {e}", path.display()))?;
    eprintln!("generated {} ({:.1} MB) in {:.1} s", path.display(), bytes.len() as f64 / 1e6, t.elapsed().as_secs_f64());
    Ok(path)
}

/// A headless GPU canvas on a device created like the app's.
fn canvas() -> Option<(GpuCanvas, RenderState)> {
    let setup = photocraft_ui_egui::gpu_canvas::wgpu_setup();
    let rs = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| egui_kittest::wgpu::create_render_state(setup, Default::default()))).ok()?;
    Some((GpuCanvas::new(&rs), rs))
}

/// The canvas's first full refresh; waits for the GPU like presenting the next frame would.
fn refresh(gpu: Option<&(GpuCanvas, RenderState)>, s: &Session) -> Result<f64, String> {
    let st = s.active().ok_or("no document")?;
    let t = Instant::now();
    match gpu {
        Some((g, rs)) => {
            g.refresh(st.doc.id.0, &st.doc, None, None);
            let _ = rs.device.poll(eframe::wgpu::PollType::Wait { submission_index: None, timeout: None });
        }
        None => {
            std::hint::black_box(photocraft_compose::render(&st.doc, st.doc.bounds()));
        }
    }
    Ok(ms(t))
}

/// `import_for_open_with`, File › Open's real path.
fn import_once(name: &str, bytes: &[u8]) -> Result<photocraft_doc::Document, String> {
    let opts = photocraft_io::ImportOptions::default();
    photocraft_io::import_for_open_with(name, bytes, &opts, &Default::default()).map(|(r, _)| r.document).map_err(|e| e.to_string())
}

/// Runs every measurement for one case in this process; the result is the JSON line's payload.
fn run_case(case: &Case, dir: &Path, quick: bool, reps: usize, cpu: bool) -> Result<Value, String> {
    let (w, h) = case.size(quick);
    let path = ensure_fixture(case, dir, quick)?;
    let bytes = std::fs::read(&path).map_err(|e| format!("read {}: {e}", path.display()))?;
    let name = format!("photo.{}", case.ext());
    let want = if case.orient6 { (h, w) } else { (w, h) };
    let mut ph: Vec<Vec<f64>> = vec![Vec::new(); PHASES.len()];
    let mut put = |i: usize, v: f64| {
        if let Some(p) = ph.get_mut(i) {
            p.push(v);
        }
    };

    // Pass 1: the real path and the admission, before anything else raises the process's peak.
    for _ in 0..reps {
        let t = Instant::now();
        let doc = import_once(&name, &bytes)?;
        put(0, ms(t));
        if doc.size.width != want.0 || doc.size.height != want.1 {
            return Err(format!("{}: opened {}x{}, expected {}x{}", case.key, doc.size.width, doc.size.height, want.0, want.1));
        }
        let t = Instant::now();
        let mut s = Session::new();
        s.open_document(doc, Some(name.clone()));
        put(1, ms(t));
    }
    let peak_process_after_import = photocraft_testkit::perf::process_peak_rss_bytes();

    // Pass 2: the phases of the import on their own.
    let format = photocraft_codecs::detect(&bytes).ok_or("unknown format")?;
    let keep = DecodeOptions { keep_orientation: true, ..Default::default() };
    for _ in 0..reps {
        let t = Instant::now();
        let img = photocraft_codecs::decode_as_with(format, &bytes, &keep).map_err(|e| e.to_string())?;
        put(2, ms(t));
        let o = photocraft_codecs::exif_orientation(img.meta.exif.as_deref().unwrap_or_default());
        let t = Instant::now();
        let upright = img.oriented(o).map_err(|e| e.to_string())?;
        put(3, ms(t));
        let t = Instant::now();
        let r = photocraft_io::image_to_document(&name, &upright).map_err(|e| e.to_string())?;
        put(4, ms(t));
        drop(r);
    }

    // Pass 3: one whole open as perf_scenarios times it: import, admission, first composite.
    let gpu = if cpu { None } else { canvas() };
    let adapter = gpu.as_ref().map(|(_, rs)| rs.adapter.get_info().name);
    // One untimed open first, so the GPU pipeline and adapter warm-up is not in the first sample.
    {
        let mut s = Session::new();
        s.open_document(import_once(&name, &bytes)?, Some(name.clone()));
        refresh(gpu.as_ref(), &s)?;
    }
    for _ in 0..reps {
        let t = Instant::now();
        let doc = import_once(&name, &bytes)?;
        let mut s = Session::new();
        s.open_document(doc, Some(name.clone()));
        let open = ms(t);
        let c = refresh(gpu.as_ref(), &s)?;
        put(5, c);
        put(6, open + c);
    }
    let peak_process_after_all_passes = photocraft_testkit::perf::process_peak_rss_bytes();
    let phases: serde_json::Map<String, Value> = PHASES.iter().zip(ph).map(|(n, v)| ((*n).to_string(), json!(v))).collect();
    Ok(json!({
        "key": case.key, "label": case.label, "width": w, "height": h, "file_bytes": bytes.len(),
        "peak_process_after_import_bytes": peak_process_after_import, "peak_process_after_all_passes_bytes": peak_process_after_all_passes, "adapter": adapter, "phases": phases,
    }))
}

/// The report rows of one case's result: `<label>: <phase>`.
fn rows_of(res: &Value) -> Vec<Value> {
    let label = res["label"].as_str().unwrap_or("?");
    PHASES
        .iter()
        .map(|p| {
            let samples: Vec<f64> = res["phases"][*p].as_array().map(|a| a.iter().filter_map(Value::as_f64).collect()).unwrap_or_default();
            let composite = matches!(*p, "first composite" | "editable");
            let peak = res[if composite { "peak_process_after_all_passes_bytes" } else { "peak_process_after_import_bytes" }].as_u64();
            let mut row = photocraft_testkit::perf::row(&format!("{label}: {p}"), &samples, peak, None);
            row["peak_rss_provenance"] = json!(if composite { "whole_case_process_after_all_passes" } else { "case_process_after_import_and_admission" });
            row
        })
        .collect()
}

/// Format a measured byte count in MiB, retaining unavailable measurements.
fn mb(v: &Value) -> String {
    v.as_u64().map_or_else(|| "n/a".into(), |b| format!("{}", b >> 20))
}

/// Print phase timings and identify the scope of both process RSS readings.
fn print_case(res: &Value) {
    println!("{} ({}x{}, {:.1} MB file)", res["label"].as_str().unwrap_or("?"), res["width"], res["height"], res["file_bytes"].as_f64().unwrap_or(0.0) / 1e6);
    for r in rows_of(res) {
        let (name, p50, p95) = (r["name"].as_str().unwrap_or("?"), r["p50_ms"].as_f64(), r["p95_ms"].as_f64());
        let phase = name.rsplit(": ").next().unwrap_or(name);
        println!("  {phase:<16} p50 {:>8.1} ms   p95 {:>8.1} ms", p50.unwrap_or(f64::NAN), p95.unwrap_or(f64::NAN));
    }
    println!(
        "  process RSS high-water: {} MiB after import/admission, {} MiB after all passes (including standalone phases and warm-up)",
        mb(&res["peak_process_after_import_bytes"]),
        mb(&res["peak_process_after_all_passes_bytes"])
    );
}

/// Starts this binary for one case so its peak memory is its own; returns the parsed result.
fn spawn_case(case: &Case, args: &[String], dir: &Path, reps: usize) -> Result<Value, String> {
    let exe = std::env::current_exe().map_err(|e| format!("current_exe: {e}"))?;
    let mut cmd = Command::new(exe);
    cmd.args(["--case", case.key, "--reps", &reps.to_string(), "--cache"]).arg(dir).stdout(Stdio::piped()).stderr(Stdio::inherit());
    for flag in ["--quick", "--cpu"] {
        if args.iter().any(|a| a == flag) {
            cmd.arg(flag);
        }
    }
    let out = cmd.output().map_err(|e| format!("start {}: {e}", case.key))?;
    if !out.status.success() {
        return Err(format!("{}: the case process failed ({})", case.key, out.status));
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let line = text.lines().find_map(|l| l.strip_prefix(RESULT_PREFIX)).ok_or_else(|| format!("{}: no result line", case.key))?;
    serde_json::from_str(line).map_err(|e| format!("{}: bad result: {e}", case.key))
}

/// Run isolated benchmark cases and optionally write their combined report.
fn main() {
    let args: Vec<String> = std::env::args().collect();
    let quick = args.iter().any(|a| a == "--quick");
    let cpu = args.iter().any(|a| a == "--cpu");
    let reps: usize = arg(&args, "--reps").and_then(|v| v.parse().ok()).unwrap_or(if quick { 3 } else { 5 }).max(1);
    let dir = arg(&args, "--cache").map_or_else(default_cache, PathBuf::from);
    let only = arg(&args, "--only");
    let mut rows = Vec::new();
    let mut results = Vec::new();
    let mut errors = Vec::new();

    if let Some(key) = arg(&args, "--case") {
        // One case in this process: print the result line for the parent (or a person).
        let Some(case) = CASES.iter().find(|c| c.key == key) else {
            eprintln!("unknown --case {key} (one of: {})", CASES.iter().map(|c| c.key).collect::<Vec<_>>().join(", "));
            std::process::exit(2);
        };
        match run_case(case, &dir, quick, reps, cpu) {
            Ok(res) => {
                print_case(&res);
                println!("{RESULT_PREFIX}{res}");
                rows.extend(rows_of(&res));
                results.push(res);
            }
            Err(e) => {
                eprintln!("{e}");
                errors.push(json!({"name": key, "error": e}));
            }
        }
    } else {
        for case in CASES.iter().filter(|c| only.as_deref().is_none_or(|o| c.key.contains(o))) {
            // Generated here, so the case process only reads it.
            let res = ensure_fixture(case, &dir, quick).and_then(|_| spawn_case(case, &args, &dir, reps));
            match res {
                Ok(res) => {
                    print_case(&res);
                    rows.extend(rows_of(&res));
                    results.push(res);
                }
                Err(e) => {
                    eprintln!("{e}");
                    errors.push(json!({"name": case.key, "error": e}));
                }
            }
        }
    }

    if let Some(out) = arg(&args, "--json") {
        let cases: Vec<Value> = results
            .iter()
            .map(|r| json!({"key": r["key"], "width": r["width"], "height": r["height"], "file_bytes": r["file_bytes"], "peak_process_after_import_bytes": r["peak_process_after_import_bytes"], "peak_process_after_all_passes_bytes": r["peak_process_after_all_passes_bytes"]}))
            .collect();
        let adapter = results.iter().find_map(|r| r["adapter"].as_str().map(str::to_string));
        let context = json!({
            "reps": reps, "quick": quick, "cache": dir.display().to_string(), "cases": cases, "errors": errors, "gpu_adapter": adapter,
            "threads": std::thread::available_parallelism().map(|n| n.get()).ok(),
        });
        let mut report = photocraft_testkit::perf::report("flat_open_bench", context, rows, None);
        // Each case ran in its own process: this one's peak says nothing about them.
        report["process_peak_rss_bytes"] = json!(results.iter().filter_map(|r| r["peak_process_after_all_passes_bytes"].as_u64()).max());
        if let Err(e) = photocraft_testkit::perf::write_report(&out, &report) {
            eprintln!("{e}");
            errors.push(json!({"name": "report", "error": e}));
        }
    }
    if !errors.is_empty() {
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_identify_the_process_peak_reading() {
        let res = json!({
            "label": "fixture", "phases": {},
            "peak_process_after_import_bytes": 100,
            "peak_process_after_all_passes_bytes": 200,
        });
        let rows = rows_of(&res);
        for (row, phase) in rows.iter().zip(PHASES) {
            let all_passes = matches!(phase, "first composite" | "editable");
            assert_eq!(row["peak_rss_bytes"], if all_passes { 200 } else { 100 });
            assert_eq!(row["peak_rss_provenance"], if all_passes { "whole_case_process_after_all_passes" } else { "case_process_after_import_and_admission" });
        }
    }
}
