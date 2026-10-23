//! Large-file open benchmark (P20, #210, #216, #5107): open the generated 2 GB PSD (17,000² px, a
//! raw full-canvas layer and merged image, just under Photoshop's PSD cap) and the 4.4 GB PSB
//! (25,000² px, data past 4 GiB) until the document is editable: read the file, decode it into a
//! document and open it in a session (the colour policy runs there), the way File › Open does
//! before the first canvas refresh. The canvas refresh is left out (large_image_bench and
//! huge_doc_bench time it).
//!
//! ```sh
//! cargo run --release -p photocraft-ui-egui --example large_file_bench -- [--reps 3] [--only psd|psb] [--cache DIR] [--clean] [--json out.json]
//! ```
//!
//! The fixtures (`photocraft_psd::fixtures`, about 6.4 GB together) are generated once into
//! `--cache` (default: `$PHOTOCRAFT_FIXTURE_DIR`, else `fixtures/` in the target directory next
//! to this binary) and kept for the next run; `--clean` removes them afterwards. Each row also
//! records the peak resident memory of its opens. Needs about 30 GB of memory for the PSB.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use photocraft_engine::Session;
use photocraft_psd::fixtures::{PSB_4GB, PSD_2GB, Streamed};
use photocraft_testkit::perf::{RssSampler, row};
use serde_json::json;

fn arg(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1).cloned())
}

/// `<target>/fixtures`: the binary lives in `<target>/release/examples/`.
fn default_cache() -> PathBuf {
    if let Some(d) = std::env::var_os("PHOTOCRAFT_FIXTURE_DIR") {
        return PathBuf::from(d);
    }
    std::env::current_exe().ok().and_then(|e| e.ancestors().nth(3).map(|t| t.join("fixtures"))).unwrap_or_else(std::env::temp_dir)
}

/// Opens `path` like File › Open: read, decode, open in a session. Returns the session (the
/// caller drops it outside the timing) and whether a sampled pixel came back right.
fn open(case: &Streamed, path: &std::path::Path) -> Result<(Session, bool), String> {
    let bytes = std::fs::read(path).map_err(|e| format!("read {}: {e}", path.display()))?;
    let doc = photocraft_io::import(&case.file_name(), &bytes).map_err(|e| e.to_string())?.document;
    drop(bytes);
    let mut s = Session::new();
    s.open_document(doc, Some(path.display().to_string()));
    let (x, y) = (case.width / 3, case.height * 2 / 3);
    let ok = s.active().and_then(|st| st.doc.layers.first()).and_then(|l| l.surface()).is_some_and(|surf| {
        let got = surf.pixel(x as i32, y as i32);
        got.len() == 4 && got.iter().enumerate().all(|(c, v)| (v - f32::from(case.sample(x, y, c as u32)) / 255.0).abs() <= 1.0 / 510.0)
    });
    Ok((s, ok))
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let reps: usize = arg(&args, "--reps").and_then(|v| v.parse().ok()).unwrap_or(3).max(1);
    let only = arg(&args, "--only");
    let dir = arg(&args, "--cache").map_or_else(default_cache, PathBuf::from);
    let clean = args.iter().any(|a| a == "--clean");
    if let Err(e) = std::fs::create_dir_all(&dir) {
        eprintln!("create {}: {e}", dir.display());
        std::process::exit(1);
    }
    let sampler = RssSampler::start(Duration::from_millis(5));
    let cases = [("psd", "open 2 GB PSD (read + decode + document)", PSD_2GB), ("psb", "open 4.4 GB PSB (read + decode + document)", PSB_4GB)];
    let mut rows = Vec::new();
    let mut errors = Vec::new();
    for (key, name, case) in &cases {
        if only.as_deref().is_some_and(|o| o != *key) {
            continue;
        }
        let path = dir.join(case.file_name());
        let want = case.file_len().unwrap_or(0);
        if std::fs::metadata(&path).map(|m| m.len()).ok() != Some(want) {
            let t = Instant::now();
            match case.write_file(&path) {
                Ok(len) => println!("generated {} ({:.2} GB) in {:.1} s", path.display(), len as f64 / 1e9, t.elapsed().as_secs_f64()),
                Err(e) => {
                    errors.push(json!({"name": name, "error": format!("generate {}: {e}", path.display())}));
                    continue;
                }
            }
        }
        let mut samples = Vec::new();
        let mut peak = None;
        for _ in 0..reps {
            sampler.reset();
            let t = Instant::now();
            match open(case, &path) {
                Ok((s, ok)) => {
                    let ms = t.elapsed().as_secs_f64() * 1000.0;
                    peak = peak.max(sampler.peak());
                    println!("{name:<44} {ms:>9.0} ms   peak RSS {:>6} MB{}", sampler.peak().unwrap_or(0) >> 20, if ok { "" } else { "   WRONG PIXELS" });
                    if !ok {
                        errors.push(json!({"name": name, "error": "the opened document's pixels don't match the fixture"}));
                        break;
                    }
                    samples.push(ms);
                    drop(s);
                }
                Err(e) => {
                    println!("{name}: {e}");
                    errors.push(json!({"name": name, "error": e}));
                    break;
                }
            }
        }
        if !samples.is_empty() {
            rows.push(row(name, &samples, peak, None));
        }
        if clean {
            let _ = std::fs::remove_file(&path);
        }
    }
    if let Some(out) = arg(&args, "--json") {
        let context =
            json!({"reps": reps, "cache": dir.display().to_string(), "errors": errors, "threads": std::thread::available_parallelism().map(|n| n.get()).ok()});
        let report = photocraft_testkit::perf::report("large_file_bench", context, rows, Some(&sampler));
        if let Err(e) = photocraft_testkit::perf::write_report(&out, &report) {
            eprintln!("{e}");
        }
    }
    if !errors.is_empty() {
        std::process::exit(1);
    }
}
