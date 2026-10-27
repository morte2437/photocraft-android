//! Workspace tooling: `cargo xtask <command>`.
//!
//! Pure Rust (std + serde_json). External tools (`cargo`, `curl`, `tar`) are
//! invoked through `std::process::Command`.

mod corpus;
mod corpus_pins;
mod i18n_coverage;
mod ico;
mod layers;
mod perf;
mod pinned;
mod scorecard;
mod sha256;
mod stats;
mod version;

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};

const USAGE: &str = "\
usage: cargo xtask <command>

commands:
  layers          enforce the crate dependency layering (docs/architecture.md §3)
  wasm            cargo check --target wasm32-unknown-unknown for the wasm-safe crates (one call, plus
                  one per optional web feature)
  test            workspace tests (cargo nextest when installed, else cargo test), then doctests
  ci              the CI gate: fmt --check, clippy -D warnings, layers, scorecard --check, tests
                  (nextest --profile ci), panic_hunt, doctests and wasm, with --locked --features heif
                  (stops at first failure), then the i18n-coverage report
  corpus [--all | --pngsuite | --psd | --psd-tools | --pixls | --atn | --photoshop | --gimp] [--local] [--update-manifest]
                  show where test corpora live and their pins (xtask/src/corpus_pins.rs), or fetch
                  them into corpus/ (pinned commits, sha256-verified; --all = every corpus but pixls,
                  --photoshop/--gimp --local copy from ../photocraft-corpus or $PHOTOCRAFT_CORPUS_REPO)
  test-corpus [-p <crate>]... [--changed] [--pixls] [--local] [-- <test args>]
                  fetch every corpus, then cargo test --release --features corpus on the corpus
                  crates; --changed runs only if psd/io/codecs/compose/gpu/text/format/raw changed;
                  --pixls (or -p raw) also fetches corpus/pixls and runs the raw corpus tests;
                  --local takes corpus/photoshop and corpus/gimp from the photocraft-corpus authoring clone
  stats [--exact] count tests and lines per crate (--exact: ask the test harness via `-- --list`)
  parity          Photoshop menu parity; rewrites docs/parity-checklist.md
  i18n-coverage   report UI translation coverage and the missing strings per registered language
                  (the last `ci` step; it never fails)
  perf [--quick] [--update-baseline] [--threshold PCT] [--bench NAME]... [--skip-build] [--reuse]
                  run the release benches, merge them by scenario id into target/perf/results.json,
                  check perf/budgets.toml and perf/baseline.json (non-zero on a broken budget or regression)
  scorecard [--check]
                  regenerate docs/scorecard.md (--check: fail if it is stale)
  version [set X.Y.Z[-pre]]
                  print the workspace version, or set it (Cargo.toml + Cargo.lock)
  ico <out.ico> <in.png>...
                  pack square PNGs (<= 256 px) into a Windows .ico (see packaging/icons.sh)
";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let rest: Vec<&str> = args.iter().skip(1).map(String::as_str).collect();
    let result = match args.first().map(String::as_str) {
        Some("layers") => cmd_layers(),
        Some("wasm") => cmd_wasm(),
        Some("test") => cmd_test().and_then(|()| cmd_doctest()),
        Some("ci") => cmd_ci(),
        Some("corpus") => corpus::cmd(&rest),
        Some("test-corpus") => corpus::test_cmd(&rest),
        Some("stats") => stats::run(&root(), rest.contains(&"--exact")),
        Some("parity") => cmd_parity(),
        Some("i18n-coverage") => i18n_coverage::run(&root()),
        Some("perf") => perf::run(&root(), &rest),
        Some("scorecard") => scorecard::run(&root(), &rest),
        Some("version") => version::run(&root(), &rest),
        Some("ico") => ico::run(&rest),
        Some("-h" | "--help" | "help") | None => {
            print!("{USAGE}");
            Ok(())
        }
        Some(other) => Err(format!("unknown command `{other}`\n\n{USAGE}")),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

/// Workspace root (parent of the xtask crate).
pub fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().expect("xtask has a parent dir").to_path_buf()
}

pub fn cargo() -> Command {
    let mut c = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()));
    c.current_dir(root());
    c
}

fn run(mut cmd: Command, what: &str) -> Result<(), String> {
    eprintln!("$ {what}");
    let status = cmd.status().map_err(|e| format!("{what}: failed to spawn: {e}"))?;
    if status.success() { Ok(()) } else { Err(format!("{what}: exited with {status}")) }
}

pub fn metadata() -> Result<serde_json::Value, String> {
    let out = cargo().args(["metadata", "--format-version", "1", "--no-deps"]).output().map_err(|e| format!("cargo metadata: {e}"))?;
    if !out.status.success() {
        return Err(format!("cargo metadata failed:\n{}", String::from_utf8_lossy(&out.stderr)));
    }
    serde_json::from_slice(&out.stdout).map_err(|e| format!("cargo metadata: bad JSON: {e}"))
}

fn cmd_layers() -> Result<(), String> {
    let crates = layers::from_metadata(&metadata()?)?;
    println!("Dependency layering (docs/architecture.md §3)\n");
    println!("{:<28} {:<14} workspace deps", "crate", "layer");
    for c in &crates {
        let ws: Vec<String> = c
            .deps
            .iter()
            .filter(|d| d.workspace)
            .map(|d| {
                let k = match d.kind {
                    layers::DepKind::Normal => "",
                    layers::DepKind::Dev => " (dev)",
                    layers::DepKind::Build => " (build)",
                };
                format!("{}{k}", layers::short_name(&d.name))
            })
            .collect();
        println!("{:<28} {:<14} {}", c.name, layers::describe(layers::classify(&c.name)), ws.join(", "));
    }
    let violations = layers::check(&crates);
    println!();
    if violations.is_empty() {
        println!("OK: {} crates, no layering violations.", crates.len());
        Ok(())
    } else {
        println!("{} violation(s):", violations.len());
        for v in &violations {
            println!("  - {v}");
        }
        Err(format!("{} layering violation(s)", violations.len()))
    }
}

/// Workspace packages that must build for wasm32: all L0–L5 crates plus
/// the egui shell.
fn wasm_set() -> Result<Vec<String>, String> {
    let crates = layers::from_metadata(&metadata()?)?;
    Ok(crates
        .into_iter()
        .filter(|c| match layers::classify(&c.name) {
            Some(layers::Class::Layer(l)) => l <= 5 || layers::short_name(&c.name) == "ui-egui",
            Some(layers::Class::Standalone) => true,
            _ => false,
        })
        .map(|c| c.name)
        .collect())
}

/// Optional features that official builds enable and the web app ships, also checked for wasm32:
/// (package, feature).
const WASM_FEATURES: &[(&str, &str)] = &[("photocraft-codecs", "heif")];

/// One `cargo check` for the whole set: each crate is checked once, and `--keep-going` still
/// reports every failing crate. Features unify across the set, so each crate is checked with the
/// union of the features the set enables. The optional features get a call each, so the crates
/// are also checked without them.
fn cmd_wasm() -> Result<(), String> {
    let set = wasm_set()?;
    if set.is_empty() {
        return Err("no wasm-safe crates found in the workspace metadata".into());
    }
    let mut results = Vec::new();
    let mut c = cargo();
    c.args(["check", "--target", "wasm32-unknown-unknown", "--keep-going"]);
    for pkg in &set {
        c.args(["-p", pkg]);
    }
    let ok = run(c, &format!("cargo check --target wasm32-unknown-unknown --keep-going -p {}", set.join(" -p "))).is_ok();
    results.push((format!("{} crates", set.len()), ok));
    for (pkg, feature) in WASM_FEATURES {
        let mut c = cargo();
        c.args(["check", "--target", "wasm32-unknown-unknown", "-p", pkg, "--features", feature]);
        let ok = run(c, &format!("cargo check --target wasm32-unknown-unknown -p {pkg} --features {feature}")).is_ok();
        results.push((format!("{pkg} --features {feature}"), ok));
    }
    println!("\nwasm32-unknown-unknown check:");
    for (p, ok) in &results {
        println!("  {:<6} {p}", if *ok { "ok" } else { "FAIL" });
    }
    let failed = results.iter().filter(|r| !r.1).count();
    if failed == 0 { Ok(()) } else { Err(format!("{failed} wasm check(s) failed")) }
}

/// Test targets for the test build. Examples are left out so they aren't linked; clippy
/// `--all-targets` still compiles them.
const TEST_TARGETS: [&str; 4] = ["--workspace", "--lib", "--bins", "--tests"];

fn nextest_available() -> bool {
    cargo().args(["nextest", "--version"]).stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok_and(|s| s.success())
}

fn cmd_test() -> Result<(), String> {
    let mut c = cargo();
    let tool = if nextest_available() {
        c.args(["nextest", "run"]);
        "nextest run"
    } else {
        eprintln!("note: cargo-nextest is not installed, running cargo test (cargo install cargo-nextest --locked)");
        c.arg("test");
        "test"
    };
    c.args(TEST_TARGETS);
    run(c, &format!("cargo {tool} {}", TEST_TARGETS.join(" ")))
}

/// What the CI gate adds to every build: `--locked`, and the optional photocraft-heif crate that
/// official builds open HEIC photos with.
const GATE_ARGS: [&str; 4] = ["--workspace", "--locked", "--features", "heif"];

/// The gate's tests. nextest's `ci` profile stops at the first failure and terminates hung tests
/// (.config/nextest.toml). `panic_hunt` runs only the engine's `#[ignore]`d panic hunt (AGENTS.md
/// Rule 9) with the same packages, features and targets as the tests, so nothing rebuilds.
fn cmd_gate_tests(panic_hunt: bool) -> Result<(), String> {
    let mut c = cargo();
    if nextest_available() {
        c.args(["nextest", "run", "--profile", "ci"]).args(GATE_ARGS).args(&TEST_TARGETS[1..]);
        if panic_hunt {
            c.args(["--run-ignored", "only", "-E", "binary_id(photocraft-engine::panic_hunt)"]);
        }
    } else if panic_hunt {
        // `-- --ignored` across the workspace would run every ignored test, not only the hunt. The
        // engine has no `heif` feature of its own; it reaches the decoder through photocraft-io.
        c.args(["test", "-p", "photocraft-engine", "--locked", "--features", "photocraft-io/heif", "--test", "panic_hunt", "--", "--ignored"]);
    } else {
        eprintln!("note: cargo-nextest is not installed, running cargo test (cargo install cargo-nextest --locked)");
        c.arg("test").args(GATE_ARGS).args(&TEST_TARGETS[1..]);
    }
    let what = format!("cargo {}", c.get_args().map(|a| a.to_string_lossy()).collect::<Vec<_>>().join(" "));
    run(c, &what)
}

/// nextest does not run doctests.
fn cmd_doctest() -> Result<(), String> {
    let mut c = cargo();
    c.args(["test", "--workspace", "--doc"]);
    run(c, "cargo test --workspace --doc")
}

fn cmd_parity() -> Result<(), String> {
    let mut c = cargo();
    c.args(["run", "-q", "-p", "photocraft-ui-egui", "--example", "parity", "--", "--write", "docs/parity-checklist.md"]);
    run(c, "cargo run -p photocraft-ui-egui --example parity")
}

fn cmd_ci() -> Result<(), String> {
    type Step = (&'static str, Box<dyn Fn() -> Result<(), String>>);
    let steps: Vec<Step> = vec![
        (
            "fmt",
            Box::new(|| {
                let mut c = cargo();
                c.args(["fmt", "--all", "--", "--check"]);
                run(c, "cargo fmt --all -- --check")
            }),
        ),
        (
            "clippy",
            Box::new(|| {
                let mut c = cargo();
                c.arg("clippy").args(GATE_ARGS).args(["--all-targets", "--keep-going", "--", "-D", "warnings"]);
                run(c, "cargo clippy --workspace --locked --features heif --all-targets --keep-going -- -D warnings")
            }),
        ),
        ("layers", Box::new(cmd_layers)),
        ("scorecard", Box::new(|| scorecard::run(&root(), &["--check"]))),
        ("test", Box::new(|| cmd_gate_tests(false))),
        ("panic_hunt", Box::new(|| cmd_gate_tests(true))),
        (
            "doctest",
            Box::new(|| {
                let mut c = cargo();
                c.arg("test").args(GATE_ARGS).arg("--doc");
                run(c, "cargo test --workspace --locked --features heif --doc")
            }),
        ),
        ("wasm", Box::new(cmd_wasm)),
        (
            "i18n",
            Box::new(|| {
                // Lists missing translations for the log; a missing string shows in English, so it never fails CI.
                if let Err(e) = i18n_coverage::run(&root()) {
                    eprintln!("warning: i18n coverage report failed: {e}");
                }
                Ok(())
            }),
        ),
    ];
    let mut done = Vec::new();
    for (name, f) in &steps {
        eprintln!("\n=== ci: {name} ===");
        if let Err(e) = f() {
            println!("\nCI summary:");
            for d in &done {
                println!("  ok    {d}");
            }
            println!("  FAIL  {name}: {e}");
            for (n, _) in steps.iter().skip(done.len() + 1) {
                println!("  skip  {n}");
            }
            return Err(format!("ci failed at `{name}`"));
        }
        done.push(*name);
    }
    println!("\nCI summary: all {} steps passed ({})", done.len(), done.join(", "));
    Ok(())
}
