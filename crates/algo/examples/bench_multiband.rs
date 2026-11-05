//! Time and peak memory of Photomerge's multi-band blend on overlapping RGB sources.
//! Backwards compatible: `cargo run --release -p photocraft-algo --example bench_multiband [n] [mp]`.
//! `cargo xtask perf --bench bench_multiband --quick` also collects repeatable JSON timings.
//! The peak is the entire process's resident-set high-water mark, not an estimate of allocations
//! by the blend alone (`photocraft_testkit::perf`: the kernel's mark on Linux, sampled elsewhere).
use std::path::PathBuf;
use std::time::Instant;

use photocraft_algo::panorama::{self, RoiImage};
use photocraft_testkit::perf::{self, RssSampler, Summary};
use serde_json::json;

const ROW: &str = "Photomerge multiband blend";

#[derive(Debug, PartialEq)]
struct Options {
    n: usize,
    mp: f64,
    reps: usize,
    json: Option<PathBuf>,
}

/// Parses legacy `[n] [mp]` arguments and the perf runner's `--reps`/`--json` flags.
/// Rejects missing, out-of-range, or non-finite values before allocating images.
fn options(args: impl IntoIterator<Item = String>) -> Result<Options, String> {
    let mut out = Options { n: 4, mp: 24.0, reps: 1, json: None };
    let mut positional = Vec::new();
    let mut it = args.into_iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--json" => {
                let path = it.next().filter(|p| !p.starts_with('-')).ok_or("--json needs a file path")?;
                out.json = Some(PathBuf::from(path));
            }
            "--reps" => {
                out.reps = it.next().ok_or("--reps needs an integer")?.parse().map_err(|_| "--reps needs an integer")?;
            }
            _ if !arg.starts_with('-') => positional.push(arg),
            _ => return Err(format!("unknown option: {arg}")),
        }
    }
    if positional.len() > 2 {
        return Err("expected at most [n] [mp]".into());
    }
    if let Some(n) = positional.first() {
        out.n = n.parse().map_err(|_| "n must be an integer")?;
    }
    if let Some(mp) = positional.get(1) {
        out.mp = mp.parse().map_err(|_| "mp must be a number")?;
    }
    if !(1..=16).contains(&out.n) || !(1..=25).contains(&out.reps) || !out.mp.is_finite() || !(0.01..=36.0).contains(&out.mp) {
        return Err("expected 1..=16 images, 0.01..=36 MP per image and 1..=25 repetitions".into());
    }
    Ok(out)
}

/// Builds deterministic overlapping RGB inputs and times each multi-band blend.
/// Writes individual samples and whole-process peak RSS to JSON when requested.
fn run() -> Result<(), String> {
    let o = options(std::env::args().skip(1))?;
    let sh = ((o.mp * 1e6 / 1.5).sqrt().round() as usize).max(2);
    let sw = sh * 3 / 2;
    let step = sw / 2;
    let (cw, ch) = (step * (o.n - 1) + sw, sh);
    let rois: Vec<RoiImage> = (0..o.n)
        .map(|k| {
            let px = (0..sw * sh * 3).map(|i| ((i / 3 % sw * 7 + i / 3 / sw * 3 + k * 31 + i % 3 * 50) % 251) as f32 / 251.0).collect();
            RoiImage { x0: (step * k) as i32, y0: 0, w: sw, h: sh, ch: 3, px, alpha: vec![1.0; sw * sh] }
        })
        .collect();
    let wts: Vec<Vec<f32>> = (0..o.n)
        .map(|k| (0..sw * sh).map(|i| if ((step * k + i % sw + step / 2) / step).saturating_sub(1).min(o.n - 1) == k { 1.0 } else { 0.0 }).collect())
        .collect();
    let levels = panorama::blend_levels(cw, ch).min(7);
    let before = perf::process_peak_rss_bytes().or_else(perf::current_rss_bytes);
    let sampler = RssSampler::start(std::time::Duration::from_millis(2));
    let mut samples_ms = Vec::with_capacity(o.reps);
    for _ in 0..o.reps {
        let t0 = Instant::now();
        let output = panorama::multiband(cw, ch, &rois, &wts, levels).map_err(|e| format!("multiband: {e}"))?;
        samples_ms.push(t0.elapsed().as_secs_f64() * 1000.0);
        std::hint::black_box(&output);
    }
    let peak = perf::process_peak_rss_bytes().or(sampler.peak());
    let delta = before.zip(peak).map(|(a, b)| b.saturating_sub(a) as f64 / (1024.0 * 1024.0));
    // The same nearest-rank p50 that `cargo xtask perf` reports for these samples.
    let p50 = Summary::of(&samples_ms).map_or(f64::NAN, |s| s.p50);
    println!(
        "{} × {}×{} ({:.2} MP) RGB, canvas {}×{}, {} levels: {:.1} ms p50 ({} runs), peak +{}",
        o.n,
        sw,
        sh,
        o.mp,
        cw,
        ch,
        levels,
        p50,
        o.reps,
        delta.map_or("n/a".to_string(), |m| format!("{m:.0} MiB"))
    );
    if let Some(path) = o.json {
        let context = json!({"sources": o.n, "mp_per_source": o.mp, "canvas_width": cw, "canvas_height": ch, "levels": levels});
        let report = perf::report("bench_multiband", context, vec![perf::row(ROW, &samples_ms, peak, None)], Some(&sampler));
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("create {}: {e}", parent.display()))?;
        }
        std::fs::write(&path, serde_json::to_vec_pretty(&report).map_err(|e| e.to_string())?).map_err(|e| format!("write {}: {e}", path.display()))?;
    }
    Ok(())
}

/// Runs the benchmark, reporting argument or output errors with a failing exit status.
fn main() {
    if let Err(e) = run() {
        eprintln!("bench_multiband: {e}");
        std::process::exit(2);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Converts borrowed argument slices for the owned-string command-line parser.
    fn parse(args: &[&str]) -> Result<Options, String> {
        options(args.iter().map(|s| s.to_string()))
    }

    /// Confirms backwards compatibility with the positional and perf-runner forms.
    #[test]
    fn parses_old_positional_args_and_perf_runner_args() {
        assert_eq!(parse(&[]).unwrap().n, 4);
        assert_eq!(
            parse(&["2", "0.25", "--reps", "3", "--json", "out.json"]).unwrap(),
            Options { n: 2, mp: 0.25, reps: 3, json: Some(PathBuf::from("out.json")) }
        );
        // The 24-36 MP range AGENTS.md benchmarks at, as before the flags were added.
        assert_eq!(parse(&["4", "36"]).unwrap().mp, 36.0);
    }

    /// Checks invalid counts, non-finite sizes, repetitions, and missing flag values.
    #[test]
    fn rejects_bad_benchmark_args() {
        for args in [
            vec!["0"],
            vec!["17"],
            vec!["2", "0"],
            vec!["2", "NaN"],
            vec!["2", "inf"],
            vec!["2", "37"],
            vec!["--reps", "0"],
            vec!["--reps", "26"],
            vec!["--json"],
            vec!["--json", "--reps", "5"],
            vec!["--reps"],
            vec!["--unknown"],
        ] {
            assert!(parse(&args).is_err(), "{args:?}");
        }
    }
}
