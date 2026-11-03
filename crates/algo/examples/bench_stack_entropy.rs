//! Stack Mode Entropy timings and exact output hashes, including lookup preparation.
//! Run the same example on both revisions: `cargo run --release -p photocraft-algo --example
//! bench_stack_entropy -- [--quick] [--reps N] [--threads N] [--json path]`.
//! Full cases use 24/36 MP; quick cases use 1/16 as many pixels. The deep-stack fallback is
//! deliberately small. Input generation, output hashing and deallocation are outside timings.

use std::path::PathBuf;
use std::time::Instant;

use photocraft_algo::stack::{Stat, combine};
use photocraft_testkit::perf::{self, Summary};
use serde_json::json;

struct Options {
    quick: bool,
    reps: usize,
    threads: Option<usize>,
    json: Option<PathBuf>,
}

fn options() -> Result<Options, String> {
    let mut o = Options { quick: false, reps: 5, threads: None, json: None };
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--quick" => o.quick = true,
            "--reps" => o.reps = args.next().ok_or("--reps needs an integer")?.parse().map_err(|_| "invalid --reps")?,
            "--threads" => o.threads = Some(args.next().ok_or("--threads needs an integer")?.parse().map_err(|_| "invalid --threads")?),
            "--json" => o.json = Some(args.next().ok_or("--json needs a path")?.into()),
            _ => return Err(format!("unknown option: {arg}")),
        }
    }
    if !(1..=25).contains(&o.reps) || o.threads.is_some_and(|n| !(1..=64).contains(&n)) {
        return Err("expected 1..=25 repetitions and 1..=64 threads".into());
    }
    Ok(o)
}

fn benchmark(o: &Options) -> Result<(), String> {
    let mut rows = Vec::new();
    let cases = [
        ("3 layers opaque 24 MP", 3, 24_000_000, false),
        ("7 layers varying alpha 24 MP", 7, 24_000_000, true),
        ("3 layers opaque 36 MP", 3, 36_000_000, false),
        ("257 layers varying alpha", 257, 8192, true),
    ];
    for (name, count, pixels, masked) in cases {
        let pixels = if o.quick && pixels > 8192 { pixels / 16 } else { pixels };
        let layers: Vec<Vec<[f32; 4]>> = (0..count)
            .map(|k| {
                (0..pixels)
                    .map(|i| {
                        [
                            ((i * 31 + k * 17) % 256) as f32 / 255.0,
                            ((i / 7 + k * 29) % 32) as f32 / 31.0,
                            ((i + k * 13) % 1024) as f32 / 255.0 - 0.5,
                            if masked && k > i % (count + 1) { 0.0 } else { 1.0 },
                        ]
                    })
                    .collect()
            })
            .collect();
        // Warm the worker pool and input pages before collecting samples.
        std::hint::black_box(combine(&layers, Stat::Entropy));
        let mut samples = Vec::with_capacity(o.reps);
        let mut hash = 0u64;
        for rep in 0..o.reps {
            let start = Instant::now();
            let out = std::hint::black_box(combine(&layers, Stat::Entropy));
            samples.push(start.elapsed().as_secs_f64() * 1000.0);
            if rep == 0 {
                hash = out.iter().flatten().fold(0xcbf29ce484222325, |h, v| (h ^ u64::from(v.to_bits())).wrapping_mul(0x100000001b3));
            }
        }
        let p50 = Summary::of(&samples).map_or(0.0, |s| s.p50);
        println!("{name}: {pixels} pixels, {p50:.3} ms p50, hash {hash:016x}, samples_ms={samples:?}");
        let mut row = perf::row(name, &samples, perf::process_peak_rss_bytes(), None);
        row["output_hash"] = json!(format!("{hash:016x}"));
        row["pixels"] = json!(pixels);
        rows.push(row);
    }
    if let Some(path) = &o.json {
        let report = perf::report("bench_stack_entropy", json!({"quick": o.quick, "threads": o.threads}), rows, None);
        std::fs::write(path, serde_json::to_vec_pretty(&report).map_err(|e| e.to_string())?).map_err(|e| format!("write {}: {e}", path.display()))?;
    }
    Ok(())
}

fn run() -> Result<(), String> {
    let o = options()?;
    #[cfg(not(target_arch = "wasm32"))]
    if let Some(threads) = o.threads {
        let pool = rayon::ThreadPoolBuilder::new().num_threads(threads).build().map_err(|e| e.to_string())?;
        return pool.install(|| benchmark(&o));
    }
    benchmark(&o)
}

fn main() {
    if let Err(e) = run() {
        eprintln!("{e}");
        std::process::exit(1);
    }
}
