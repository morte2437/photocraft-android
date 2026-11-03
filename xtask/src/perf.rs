//! `cargo xtask perf`: run the release benches, merge their `--json` reports into one result keyed
//! by scenario id, and check it against `perf/budgets.toml` and `perf/baseline.json` (#221).
//!
//! ```text
//! cargo xtask perf [--quick] [--update-baseline [--allow-contended]] [--threshold PCT] [--bench NAME]... [--skip-build] [--reuse]
//! cargo xtask perf --update-baseline --from results.json [--from results.json]... [--allow-contended]
//! ```
//!
//! - `--quick`: only the benches with `quick_args` (small synthetic documents; minutes, not tens
//!   of minutes). Quick numbers are compared only with a quick baseline, and budgets don't apply
//!   (they are set for the full-size documents).
//! - `--update-baseline`: write this run into `perf/baseline.json` (the only way it changes).
//!   A contended run (1-minute load at the start above `settings.max_load_per_core` per core) is
//!   refused; `--allow-contended` records it, labelled `"contended": true`.
//! - `--from FILE`: with `--update-baseline`, build the baseline from saved `results.json` files
//!   (at least `settings.baseline_min_runs`, one machine class and mode, clean trees) instead of
//!   running: per scenario the median p50 and the highest (the noise ceiling regressions are
//!   judged against). Prints which budgets could be enforced and which enforced ones are flaky.
//! - `--threshold PCT`: the regression threshold (default `settings.regression_pct`, 15 %).
//! - `--bench NAME`: run only these benches (repeatable).
//! - `--skip-build`: don't run `cargo build` first. `--reuse`: don't run the benches either;
//!   re-evaluate the reports already in `target/perf/`.
//!
//! Output: `target/perf/results.json` (everything), `target/perf/summary.md` (the table the
//! nightly job posts as its summary) and one `target/perf/<bench>.json` per bench.
//!
//! Exit status: non-zero when an enforced budget breaks (full mode), a scenario's p50 regresses
//! more than the threshold against a baseline from the same machine class and mode (against its
//! noise ceiling when it was built from several runs), or a bench
//! fails. Runs from different machine classes are never compared.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

use serde::Deserialize;
use serde_json::{Value, json};

pub const BUDGETS: &str = "perf/budgets.toml";
pub const BASELINE: &str = "perf/baseline.json";
const MISSING_PREFIX: &str = "**missing**: ";

// ---- budgets.toml ----------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Budgets {
    #[serde(default)]
    pub settings: Settings,
    #[serde(default)]
    pub bench: Vec<BenchSpec>,
    #[serde(default)]
    pub scenario: Vec<Scenario>,
}

#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    /// A scenario regresses when its p50 grows by more than this share of the baseline…
    pub regression_pct: f64,
    /// …and by more than this many milliseconds (timer noise on sub-millisecond rows).
    pub regression_min_ms: f64,
    /// A run is contended when its 1-minute load at the start exceeds this many per core: its
    /// numbers measure the machine's load, so `--update-baseline` refuses it.
    pub max_load_per_core: f64,
    /// `--update-baseline --from` needs at least this many saved runs.
    pub baseline_min_runs: usize,
    /// A budget could be enforced when its metric stays within this share of the budget in every run.
    pub enforce_margin: f64,
}

impl Default for Settings {
    fn default() -> Self {
        Self { regression_pct: 15.0, regression_min_ms: 1.0, max_load_per_core: 1.0, baseline_min_runs: 5, enforce_margin: 0.6 }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BenchSpec {
    pub name: String,
    pub package: String,
    /// Example name; defaults to `name`.
    pub example: Option<String>,
    /// Arguments of a full run (`--json <file>` is appended).
    #[serde(default)]
    pub args: Vec<String>,
    /// Arguments of a `--quick` run; absent = the bench is skipped in quick mode.
    pub quick_args: Option<Vec<String>>,
    /// Source file that must exist for the bench to run (a bench that lives on another branch):
    /// skipped, not failed, while it is missing.
    pub requires: Option<String>,
    #[serde(default)]
    pub note: String,
}

impl BenchSpec {
    pub fn example(&self) -> &str {
        self.example.as_deref().unwrap_or(&self.name)
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Metric {
    P50,
    #[default]
    P95,
    Max,
}

impl Metric {
    fn label(self) -> &'static str {
        match self {
            Metric::P50 => "p50",
            Metric::P95 => "p95",
            Metric::Max => "max",
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scenario {
    pub id: String,
    pub title: String,
    pub issue: Option<u32>,
    /// Bench and row name that measure it; both absent when `not_measurable` is set.
    pub bench: Option<String>,
    pub row: Option<String>,
    #[serde(default)]
    pub metric: Metric,
    pub budget_ms: Option<f64>,
    /// Free-form target when it isn't a plain time budget.
    pub target: Option<String>,
    /// A broken budget fails the run. Off for targets the code doesn't meet yet: they are
    /// reported ("over budget") but only regressions fail.
    #[serde(default = "yes")]
    pub enforce: bool,
    /// Why this can't be measured yet (the code doesn't support it).
    pub not_measurable: Option<String>,
}

fn yes() -> bool {
    true
}

impl Scenario {
    pub fn target_text(&self) -> String {
        if let Some(t) = &self.target {
            return t.clone();
        }
        match self.budget_ms {
            Some(b) if b.fract() == 0.0 => format!("≤ {b:.0} ms {}", self.metric.label()),
            Some(b) => format!("≤ {b} ms {}", self.metric.label()),
            None => "track only".into(),
        }
    }
}

pub fn parse_budgets(text: &str) -> Result<Budgets, String> {
    let b: Budgets = toml::from_str(text).map_err(|e| format!("{BUDGETS}: {e}"))?;
    let mut ids = std::collections::BTreeSet::new();
    for s in &b.scenario {
        if !ids.insert(s.id.as_str()) {
            return Err(format!("{BUDGETS}: duplicate scenario id {}", s.id));
        }
        match (&s.bench, &s.row, &s.not_measurable) {
            (Some(bench), Some(_), None) => {
                if !b.bench.iter().any(|x| &x.name == bench) {
                    return Err(format!("{BUDGETS}: scenario {} uses undefined bench `{bench}`", s.id));
                }
            }
            (None, None, Some(_)) => {}
            _ => return Err(format!("{BUDGETS}: scenario {} needs either bench + row, or not_measurable", s.id)),
        }
        if s.budget_ms.is_some_and(|v| !(v.is_finite() && v > 0.0)) {
            return Err(format!("{BUDGETS}: scenario {} has an invalid budget_ms", s.id));
        }
    }
    if !(b.settings.regression_pct.is_finite() && b.settings.regression_pct > 0.0) {
        return Err(format!("{BUDGETS}: settings.regression_pct must be > 0"));
    }
    if !(b.settings.regression_min_ms.is_finite() && b.settings.regression_min_ms >= 0.0) {
        return Err(format!("{BUDGETS}: settings.regression_min_ms must be finite and >= 0"));
    }
    if !(b.settings.max_load_per_core.is_finite() && b.settings.max_load_per_core > 0.0) {
        return Err(format!("{BUDGETS}: settings.max_load_per_core must be > 0"));
    }
    if b.settings.baseline_min_runs == 0 {
        return Err(format!("{BUDGETS}: settings.baseline_min_runs must be >= 1"));
    }
    if !(b.settings.enforce_margin.is_finite() && b.settings.enforce_margin > 0.0 && b.settings.enforce_margin <= 1.0) {
        return Err(format!("{BUDGETS}: settings.enforce_margin must be in (0, 1]"));
    }
    Ok(b)
}

// ---- measurements ----------------------------------------------------------------------------

/// One scenario's numbers.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Stats {
    pub n: usize,
    pub p50: Option<f64>,
    pub p95: Option<f64>,
    pub max: Option<f64>,
    pub peak_rss: Option<u64>,
    pub gpu_bytes: Option<u64>,
    /// A baseline built from several runs: the highest p50 among them (its noise ceiling).
    pub p50_max: Option<f64>,
}

impl Stats {
    pub fn metric(&self, m: Metric) -> Option<f64> {
        match m {
            Metric::P50 => self.p50,
            Metric::P95 => self.p95,
            Metric::Max => self.max,
        }
    }

    pub fn to_json(&self) -> Value {
        let mut v =
            json!({"n": self.n, "p50_ms": self.p50, "p95_ms": self.p95, "max_ms": self.max, "peak_rss_bytes": self.peak_rss, "gpu_bytes": self.gpu_bytes});
        if let (Some(m), Some(o)) = (self.p50_max, v.as_object_mut()) {
            o.insert("p50_max_ms".into(), json!(m));
        }
        v
    }

    pub fn from_json(v: &Value) -> Self {
        Self {
            n: v.get("n").and_then(Value::as_u64).unwrap_or(0) as usize,
            p50: v.get("p50_ms").and_then(Value::as_f64),
            p95: v.get("p95_ms").and_then(Value::as_f64),
            max: v.get("max_ms").and_then(Value::as_f64),
            peak_rss: v.get("peak_rss_bytes").and_then(Value::as_u64),
            gpu_bytes: v.get("gpu_bytes").and_then(Value::as_u64),
            p50_max: v.get("p50_max_ms").and_then(Value::as_f64),
        }
    }
}

/// Nearest-rank percentile of sorted, non-empty `v`.
fn rank(v: &[f64], q: f64) -> Option<f64> {
    let n = v.len();
    if n == 0 {
        return None;
    }
    let k = ((q * n as f64).ceil() as usize).clamp(1, n);
    v.get(k - 1).copied()
}

/// A bench row → stats. Rows carry `samples_ms` (preferred: percentiles are recomputed here),
/// or `p50_ms`/`p95_ms`/`max_ms`, or the older `median_ms`/`max_ms`. A p95 is never invented:
/// with neither samples nor a reported p95 it stays empty.
pub fn row_stats(row: &Value) -> Stats {
    let mut samples: Vec<f64> =
        row.get("samples_ms").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_f64).filter(|x| x.is_finite()).collect()).unwrap_or_default();
    samples.sort_by(f64::total_cmp);
    let f = |k: &str| row.get(k).and_then(Value::as_f64);
    let mut s = Stats {
        n: samples.len(),
        p50: f("p50_ms").or_else(|| f("median_ms")),
        p95: f("p95_ms"),
        max: f("max_ms"),
        peak_rss: row.get("peak_rss_bytes").and_then(Value::as_u64),
        gpu_bytes: row.get("gpu_bytes").and_then(Value::as_u64),
        p50_max: None,
    };
    if !samples.is_empty() {
        s.p50 = rank(&samples, 0.5);
        s.p95 = rank(&samples, 0.95);
        s.max = samples.last().copied();
    } else if let Some(n) = row.get("n").and_then(Value::as_u64) {
        s.n = n as usize;
    }
    s
}

/// The row named `name` in a bench report (`{"rows": [{"name": …}, …]}`).
pub fn find_row<'a>(report: &'a Value, name: &str) -> Option<&'a Value> {
    report.get("rows")?.as_array()?.iter().find(|r| r.get("name").and_then(Value::as_str) == Some(name))
}

/// `cur` regressed against `base` by more than `pct` % and more than `min_ms`.
pub fn regressed(cur: f64, base: f64, pct: f64, min_ms: f64) -> bool {
    base > 0.0 && cur.is_finite() && cur > base * (1.0 + pct / 100.0) && cur - base > min_ms
}

pub fn change_pct(cur: f64, base: f64) -> Option<f64> {
    (base > 0.0 && cur.is_finite()).then(|| (cur / base - 1.0) * 100.0)
}

#[derive(Clone, Debug, PartialEq)]
pub enum Status {
    Pass,
    /// Over budget, not enforced (target not met yet).
    Over,
    /// Over an enforced budget.
    Broken,
    Regressed,
    /// Measured, no budget, no regression.
    Tracked,
    /// The bench ran but the row is missing or errored.
    Missing(String),
    NotRun,
    NotMeasurable(String),
}

impl Status {
    pub fn label(&self) -> String {
        match self {
            Status::Pass => "pass".into(),
            Status::Over => "over budget (target not met yet)".into(),
            Status::Broken => "**BUDGET BROKEN**".into(),
            Status::Regressed => "**REGRESSED**".into(),
            Status::Tracked => "tracked".into(),
            Status::Missing(e) => format!("{MISSING_PREFIX}{e}"),
            Status::NotRun => "not run".into(),
            Status::NotMeasurable(_) => "not measurable yet".into(),
        }
    }
    pub fn fails(&self) -> bool {
        matches!(self, Status::Broken | Status::Regressed | Status::Missing(_))
    }
}

/// One evaluated scenario.
#[derive(Clone, Debug)]
pub struct Outcome {
    pub id: String,
    pub title: String,
    pub issue: Option<u32>,
    pub target: String,
    pub stats: Option<Stats>,
    pub baseline_p50: Option<f64>,
    pub change_pct: Option<f64>,
    pub status: Status,
}

/// Budget and regression verdict for one scenario. `budgets_apply` is false in quick mode.
/// A regression is judged against the baseline's noise ceiling (the highest p50 of the runs it was
/// built from), or its p50 when it came from one run.
pub fn verdict(sc: &Scenario, stats: &Stats, base: Option<&Stats>, settings: &Settings, threshold: f64, budgets_apply: bool) -> Status {
    if let (Some(cur), Some(b)) = (stats.p50, base.and_then(|b| b.p50_max.or(b.p50)))
        && regressed(cur, b, threshold, settings.regression_min_ms)
    {
        return Status::Regressed;
    }
    match (budgets_apply, sc.budget_ms, stats.metric(sc.metric)) {
        (true, Some(budget), Some(v)) if v > budget => {
            if sc.enforce {
                Status::Broken
            } else {
                Status::Over
            }
        }
        (true, Some(_), Some(_)) => Status::Pass,
        (true, Some(_), None) => Status::Missing(format!("no {} in the bench row", sc.metric.label())),
        _ => Status::Tracked,
    }
}

/// Evaluates every scenario against the bench reports (`bench name → report`, `None` when the
/// bench didn't run in this mode) and the baseline's scenarios for this mode.
pub fn evaluate(
    budgets: &Budgets,
    reports: &BTreeMap<String, Result<Value, String>>,
    baseline: Option<&BTreeMap<String, Stats>>,
    threshold: f64,
    budgets_apply: bool,
) -> Vec<Outcome> {
    budgets
        .scenario
        .iter()
        .map(|sc| {
            let base = baseline.and_then(|b| b.get(&sc.id));
            let mut o = Outcome {
                id: sc.id.clone(),
                title: sc.title.clone(),
                issue: sc.issue,
                target: sc.target_text(),
                stats: None,
                baseline_p50: base.and_then(|b| b.p50),
                change_pct: None,
                status: Status::NotRun,
            };
            if let Some(why) = &sc.not_measurable {
                o.status = Status::NotMeasurable(why.clone());
                return o;
            }
            let (Some(bench), Some(row)) = (&sc.bench, &sc.row) else { return o };
            match reports.get(bench) {
                None => {}
                Some(Err(e)) => o.status = Status::Missing(format!("bench `{bench}` failed: {e}")),
                Some(Ok(rep)) => match find_row(rep, row) {
                    None => {
                        let err = rep
                            .get("context")
                            .and_then(|c| c.get("errors"))
                            .and_then(Value::as_array)
                            .and_then(|a| a.iter().find(|e| e.get("name").and_then(Value::as_str) == Some(row.as_str())))
                            .and_then(|e| e.get("error"))
                            .and_then(Value::as_str)
                            .map_or_else(|| format!("row `{row}` not in `{bench}`"), str::to_string);
                        o.status = Status::Missing(err);
                    }
                    Some(r) => {
                        let st = row_stats(r);
                        o.change_pct = st.p50.zip(o.baseline_p50).and_then(|(c, b)| change_pct(c, b));
                        o.status = verdict(sc, &st, base, &budgets.settings, threshold, budgets_apply);
                        o.stats = Some(st);
                    }
                },
            }
            o
        })
        .collect()
}

// ---- formatting ------------------------------------------------------------------------------

pub fn fmt_num(v: f64) -> String {
    if v >= 100.0 {
        format!("{v:.0}")
    } else if v >= 10.0 {
        format!("{v:.1}")
    } else {
        format!("{v:.2}")
    }
}

fn fmt_ms(v: Option<f64>) -> String {
    v.map_or_else(|| "–".into(), fmt_num)
}

pub fn fmt_bytes(v: Option<u64>) -> String {
    match v {
        None | Some(0) => "–".into(),
        Some(b) if b >= 1 << 30 => format!("{:.1} GB", b as f64 / (1u64 << 30) as f64),
        Some(b) => format!("{} MB", b >> 20),
    }
}

pub fn issue_link(n: Option<u32>) -> String {
    n.map_or_else(String::new, |n| format!("[#{n}](https://github.com/storytold/photocraft/issues/{n})"))
}

/// The scenario table (Markdown); `with_baseline` adds the change against the baseline.
/// Not-measurable rows go to a separate list after it.
pub fn render_table(outcomes: &[Outcome], with_baseline: bool) -> String {
    let mut s = if with_baseline {
        String::from(
            "| Id | Scenario | Target | p50 ms | p95 ms | max ms | Peak RSS | GPU | vs baseline | Status | Issue |\n|---|---|---|---:|---:|---:|---:|---:|---:|---|---|\n",
        )
    } else {
        String::from(
            "| Id | Scenario | Target | p50 ms | p95 ms | max ms | Peak RSS | GPU | Status | Issue |\n|---|---|---|---:|---:|---:|---:|---:|---|---|\n",
        )
    };
    for o in outcomes.iter().filter(|o| !matches!(o.status, Status::NotMeasurable(_))) {
        let st = o.stats.clone().unwrap_or_default();
        let vs = if with_baseline { format!(" {} |", o.change_pct.map_or_else(|| "–".into(), |c| format!("{c:+.0} %"))) } else { String::new() };
        s.push_str(&format!(
            "| {} | {} | {} | {} | {} | {} | {} | {} |{} {} | {} |\n",
            o.id,
            o.title,
            o.target,
            fmt_ms(st.p50),
            fmt_ms(st.p95),
            fmt_ms(st.max),
            fmt_bytes(st.peak_rss),
            fmt_bytes(st.gpu_bytes),
            vs,
            o.status.label(),
            issue_link(o.issue),
        ));
    }
    let nm: Vec<&Outcome> = outcomes.iter().filter(|o| matches!(o.status, Status::NotMeasurable(_))).collect();
    if !nm.is_empty() {
        s.push_str(
            "\n**Not measurable yet** (the code can't do it yet; no number is recorded):\n\n| Id | Scenario | Target | Why | Issue |\n|---|---|---|---|---|\n",
        );
        for o in nm {
            let why = match &o.status {
                Status::NotMeasurable(w) => w.as_str(),
                _ => "",
            };
            s.push_str(&format!("| {} | {} | {} | {} | {} |\n", o.id, o.title, o.target, why, issue_link(o.issue)));
        }
    }
    s
}

/// Counts for the scorecard summary: (within budget, over budget or failing, tracked only, not measurable).
/// Not-run scenarios are in none of them; the scorecard counts those separately.
pub fn counts(outcomes: &[Outcome]) -> (usize, usize, usize, usize) {
    let mut c = (0, 0, 0, 0);
    for o in outcomes {
        match o.status {
            Status::Pass => c.0 += 1,
            Status::Over | Status::Broken | Status::Regressed | Status::Missing(_) => c.1 += 1,
            Status::Tracked => c.2 += 1,
            Status::NotMeasurable(_) => c.3 += 1,
            Status::NotRun => {}
        }
    }
    c
}

// ---- machine ---------------------------------------------------------------------------------

/// Lower-case, `-`-separated, alphanumeric only.
pub fn slug(s: &str) -> String {
    let mut out = String::new();
    for ch in s.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    out.trim_end_matches('-').to_string()
}

/// Runs are compared only within one class: OS, architecture, CPU model and GPU adapter.
pub fn machine_class(os: &str, arch: &str, cpu: &str, gpu: &str) -> String {
    [os, arch, cpu, gpu].iter().map(|p| slug(p)).filter(|p| !p.is_empty()).collect::<Vec<_>>().join("-")
}

/// The first three numbers in `/proc/loadavg` or `sysctl -n vm.loadavg` output.
pub fn parse_loadavg(s: &str) -> Option<[f64; 3]> {
    let v: Vec<f64> = s.split_whitespace().filter_map(|t| t.trim_matches(|c| c == '{' || c == '}').parse().ok()).take(3).collect();
    Some([*v.first()?, *v.get(1)?, *v.get(2)?])
}

fn output(cmd: &str, args: &[&str]) -> Option<String> {
    let o = Command::new(cmd).args(args).output().ok()?;
    o.status.success().then(|| String::from_utf8_lossy(&o.stdout).trim().to_string()).filter(|s| !s.is_empty())
}

pub fn load_avg() -> Option<[f64; 3]> {
    if let Ok(s) = std::fs::read_to_string("/proc/loadavg") {
        return parse_loadavg(&s);
    }
    output("sysctl", &["-n", "vm.loadavg"]).and_then(|s| parse_loadavg(&s))
}

fn proc_field(file: &str, key: &str) -> Option<String> {
    let s = std::fs::read_to_string(file).ok()?;
    s.lines().find(|l| l.starts_with(key)).and_then(|l| l.split_once(':').or_else(|| l.split_once('='))).map(|(_, v)| v.trim().trim_matches('"').to_string())
}

pub fn machine() -> Value {
    let (os, arch) = (std::env::consts::OS, std::env::consts::ARCH);
    let cores = std::thread::available_parallelism().map(|n| n.get()).ok();
    let (cpu, ram, os_version) = match os {
        "macos" => (
            output("sysctl", &["-n", "machdep.cpu.brand_string"]),
            output("sysctl", &["-n", "hw.memsize"]).and_then(|s| s.parse::<u64>().ok()),
            output("sw_vers", &["-productVersion"]).map(|v| format!("macOS {v}")),
        ),
        "linux" => (
            proc_field("/proc/cpuinfo", "model name").or_else(|| proc_field("/proc/cpuinfo", "CPU part").map(|p| format!("ARM part {p}"))),
            proc_field("/proc/meminfo", "MemTotal")
                .and_then(|v| v.split_whitespace().next().and_then(|k| k.parse::<u64>().ok()))
                .and_then(|k| k.checked_mul(1024)),
            proc_field("/etc/os-release", "PRETTY_NAME"),
        ),
        _ => (std::env::var("PROCESSOR_IDENTIFIER").ok(), None, output("cmd", &["/c", "ver"])),
    };
    json!({"os": os, "os_version": os_version, "arch": arch, "cpu": cpu, "cores": cores, "ram_bytes": ram})
}

/// `YYYY-MM-DDTHH:MM:SSZ` for a Unix time (proleptic Gregorian, UTC). A time past chrono's year
/// range still converts (the calendar repeats every 400 years, so the instant is moved into range
/// by whole cycles and only the year is carried, matching the old hand-rolled `iso_utc`), never a
/// silent reset to the epoch. Accepts the full `u64` range the old code did.
pub fn iso_utc(secs: u64) -> String {
    use chrono::Datelike;
    // `secs / 86_400` is at most 213_503_982_334_601, so `days` always fits in `i64` (old code too).
    let (days, sod) = ((secs / 86_400) as i64, (secs % 86_400) as i64);
    let (cycles, rest) = (days.div_euclid(146_097), days.rem_euclid(146_097));
    // `rest + 719_163` (1970-01-01 is CE day 719_163) is always a valid CE day, and `sod` < 86_400.
    let date = i32::try_from(rest + 719_163).ok().and_then(chrono::NaiveDate::from_num_days_from_ce_opt);
    let time = u32::try_from(sod).ok().and_then(|s| chrono::NaiveTime::from_num_seconds_from_midnight_opt(s, 0));
    match (date, time) {
        (Some(d), Some(t)) => {
            let y = i128::from(d.year()) + i128::from(cycles) * 400;
            let y = i64::try_from(y).unwrap_or(if y < 0 { i64::MIN } else { i64::MAX });
            // `{:04}` rather than `%Y`, which writes a `+` before years past 9999.
            format!("{y:04}-{}", d.and_time(t).format("%m-%dT%H:%M:%SZ"))
        }
        _ => "1970-01-01T00:00:00Z".to_string(), // unreachable
    }
}

fn now_iso() -> String {
    iso_utc(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0))
}

fn git(root: &Path, args: &[&str]) -> Option<String> {
    let o = Command::new("git").current_dir(root).args(args).output().ok()?;
    o.status.success().then(|| String::from_utf8_lossy(&o.stdout).trim().to_string())
}

// ---- baseline --------------------------------------------------------------------------------

/// The baseline's scenarios for `mode`, if it was recorded on `class`. `Err` explains a skip.
pub fn baseline_for(baseline: &Value, class: &str, mode: &str) -> Result<BTreeMap<String, Stats>, String> {
    let bc = baseline.get("machine_class").and_then(Value::as_str).unwrap_or("");
    if bc != class {
        return Err(format!("baseline is from machine class `{bc}`, this is `{class}`: regression check skipped"));
    }
    let sc = baseline
        .get("modes")
        .and_then(|m| m.get(mode))
        .and_then(|m| m.get("scenarios"))
        .and_then(Value::as_object)
        .ok_or_else(|| format!("baseline has no `{mode}` run: regression check skipped"))?;
    Ok(sc.iter().map(|(k, v)| (k.clone(), Stats::from_json(v))).collect())
}

/// A run is contended when its 1-minute load at the start exceeds `max_per_core` per core:
/// its numbers measure the machine's load, not the code.
/// Without a load (Windows) or a core count, nothing can be judged: not contended.
pub fn contended(load: Option<[f64; 3]>, cores: Option<usize>, max_per_core: f64) -> bool {
    match (load, cores) {
        (Some([one, _, _]), Some(c)) if c > 0 => one > c as f64 * max_per_core,
        _ => false,
    }
}

fn load_of(v: Option<&Value>) -> Option<[f64; 3]> {
    let a = v?.as_array()?;
    Some([a.first()?.as_f64()?, a.get(1)?.as_f64()?, a.get(2)?.as_f64()?])
}

fn cores_of(machine: Option<&Value>) -> Option<usize> {
    machine?.get("cores")?.as_u64().map(|c| c as usize)
}

/// Whether a `results.json` (or a baseline mode plus its machine) was contended at the start.
pub fn run_contended(load_avg_start: Option<&Value>, machine: Option<&Value>, max_per_core: f64) -> bool {
    contended(load_of(load_avg_start), cores_of(machine), max_per_core)
}

/// What `baseline_from_runs` accepts.
pub struct BaselineRules {
    /// Fewer runs are refused.
    pub min_runs: usize,
    pub max_load_per_core: f64,
    /// Accept contended runs (the baseline then records `"contended": true`).
    pub allow_contended: bool,
}

fn median(v: &mut [f64]) -> Option<f64> {
    v.sort_by(f64::total_cmp);
    let n = v.len();
    match n {
        0 => None,
        _ if n % 2 == 1 => v.get(n / 2).copied(),
        _ => Some((v.get(n / 2 - 1)? + v.get(n / 2)?) / 2.0),
    }
}

/// `old` with one mode replaced by a baseline built from `runs` (saved `results.json` values of
/// one machine class and mode), or a fresh baseline when the machine class changed. Per scenario:
/// `p50_ms` is the median of the runs' p50 and, from two runs on, `p50_max_ms` their maximum (the
/// noise ceiling regressions are judged against); p95 is the median, max and memory the maximum.
/// One run is the plain `--update-baseline` case. Refused: too few runs, mixed machine classes or
/// modes, dirty trees when there are several runs, and contended runs unless allowed.
pub fn baseline_from_runs(old: Option<&Value>, runs: &[Value], rules: &BaselineRules) -> Result<Value, String> {
    if runs.len() < rules.min_runs.max(1) {
        return Err(format!("a baseline needs at least {} run(s), got {}", rules.min_runs.max(1), runs.len()));
    }
    let class_of = |r: &Value| r.get("machine").and_then(|m| m.get("class")).cloned().unwrap_or(Value::Null);
    let mode_of = |r: &Value| r.get("mode").and_then(Value::as_str).unwrap_or("full").to_string();
    let mut runs: Vec<&Value> = runs.iter().collect();
    runs.sort_by_key(|r| r.get("date").and_then(Value::as_str).unwrap_or("").to_string());
    let Some(last) = runs.last().copied() else { return Err("no runs".into()) };
    let (class, mode) = (class_of(last), mode_of(last));
    if runs.iter().any(|r| class_of(r) != class) {
        return Err("the runs are from different machine classes: a baseline needs one".into());
    }
    if runs.iter().any(|r| mode_of(r) != mode) {
        return Err("the runs mix full and quick modes: a baseline needs one".into());
    }
    if runs.len() > 1
        && let Some(r) = runs.iter().find(|r| r.get("dirty").and_then(Value::as_bool) == Some(true))
    {
        return Err(format!("run {} was taken on a dirty tree: a baseline needs committed code", r.get("date").and_then(Value::as_str).unwrap_or("?")));
    }
    let is_contended = |r: &Value| run_contended(r.get("load_avg_start"), r.get("machine"), rules.max_load_per_core);
    let any_contended = runs.iter().any(|r| is_contended(r));
    if any_contended && !rules.allow_contended {
        let msgs: Vec<String> = runs
            .iter()
            .filter(|r| is_contended(r))
            .map(|r| {
                format!(
                    "load {} on {} cores at the start of {}",
                    load_of(r.get("load_avg_start")).map_or_else(|| "?".into(), |l| format!("{:.2}", l[0])),
                    cores_of(r.get("machine")).map_or_else(|| "?".into(), |c| c.to_string()),
                    r.get("date").and_then(Value::as_str).unwrap_or("the run")
                )
            })
            .collect();
        return Err(format!(
            "contended run, baseline not written: {} (above {} per core). Its numbers measure machine load; rerun on an idle machine, or pass --allow-contended to record it labelled as contended",
            msgs.join("; "),
            rules.max_load_per_core
        ));
    }

    let mut per: BTreeMap<String, Vec<Stats>> = BTreeMap::new();
    let mut failed = serde_json::Map::new();
    for r in &runs {
        for (k, v) in r.get("scenarios").and_then(Value::as_object).into_iter().flatten() {
            if v.get("p50_ms").is_some_and(|x| !x.is_null()) {
                per.entry(k.clone()).or_default().push(Stats::from_json(v));
            } else if let Some(e) = v.get("status").and_then(Value::as_str).and_then(|s| s.strip_prefix(MISSING_PREFIX)) {
                // A crashed row (a crash the bench found): kept so the scorecard says so.
                failed.insert(k.clone(), json!(e));
            }
        }
    }
    let mut scenarios = serde_json::Map::new();
    for (k, st) in &per {
        // A row that crashed in any run stays failed, and one missing from some runs is left
        // out: no partial aggregate under a full run count.
        if failed.contains_key(k) || st.len() != runs.len() {
            continue;
        }
        let mut p50: Vec<f64> = st.iter().filter_map(|s| s.p50).collect();
        let mut p95: Vec<f64> = st.iter().filter_map(|s| s.p95).collect();
        let fmax = |f: fn(&Stats) -> Option<f64>| st.iter().filter_map(f).reduce(f64::max);
        let umax = |f: fn(&Stats) -> Option<u64>| st.iter().filter_map(f).max();
        let s = Stats {
            n: st.iter().map(|s| s.n).sum(),
            p50_max: (runs.len() > 1).then(|| fmax(|s| s.p50)).flatten(),
            p50: median(&mut p50),
            p95: median(&mut p95),
            max: fmax(|s| s.max),
            peak_rss: umax(|s| s.peak_rss),
            gpu_bytes: umax(|s| s.gpu_bytes),
        };
        scenarios.insert(k.clone(), s.to_json());
    }

    let mut modes = old.filter(|o| o.get("machine_class") == Some(&class)).and_then(|o| o.get("modes")).and_then(Value::as_object).cloned().unwrap_or_default();
    let mut entry = json!({
        "date": last.get("date"),
        "commit": last.get("commit"),
        "load_avg_start": last.get("load_avg_start"),
        "load_avg_end": last.get("load_avg_end"),
        "cores": cores_of(last.get("machine")),
        "runs": runs.len(),
        "scenarios": scenarios,
        "failed": failed,
    });
    if let Some(o) = entry.as_object_mut() {
        if any_contended {
            o.insert("contended".into(), json!(true));
        }
        if runs.len() > 1 {
            let each: Vec<Value> = runs
                .iter()
                .map(|r| json!({"date": r.get("date"), "commit": r.get("commit"), "load_avg_start": r.get("load_avg_start"), "contended": is_contended(r)}))
                .collect();
            o.insert("run_list".into(), json!(each));
        }
    }
    modes.insert(mode, entry);
    Ok(json!({"schema": 1, "machine_class": class, "machine": last.get("machine"), "modes": modes}))
}

/// From saved runs: budgets that could be enforced (not enforced, and within `enforce_margin` of
/// the budget in every run) and enforced budgets that went over in some run (flaky).
pub fn enforce_report(budgets: &Budgets, runs: &[Value]) -> (Vec<String>, Vec<String>) {
    let (mut could, mut flaky) = (Vec::new(), Vec::new());
    let margin = budgets.settings.enforce_margin;
    for sc in &budgets.scenario {
        let (Some(budget), None) = (sc.budget_ms, &sc.not_measurable) else { continue };
        let key = format!("{}_ms", sc.metric.label());
        let vals: Vec<f64> = runs.iter().filter_map(|r| r.get("scenarios")?.get(&sc.id)?.get(&key)?.as_f64()).collect();
        let Some(worst) = vals.iter().copied().reduce(f64::max) else { continue };
        let m = sc.metric.label();
        if sc.enforce {
            let over = vals.iter().filter(|v| **v > budget).count();
            if over > 0 {
                flaky.push(format!("{} {}: over {} ms in {over} of {} runs (worst {m} {})", sc.id, sc.title, fmt_num(budget), runs.len(), fmt_num(worst)));
            }
        } else if vals.len() == runs.len() && worst <= margin * budget {
            could.push(format!("{} {}: worst {m} {} ms ≤ {margin} × {} ms in all {} runs", sc.id, sc.title, fmt_num(worst), fmt_num(budget), runs.len()));
        }
    }
    (could, flaky)
}

// ---- running ---------------------------------------------------------------------------------

struct Opts {
    quick: bool,
    update_baseline: bool,
    threshold: Option<f64>,
    benches: Vec<String>,
    skip_build: bool,
    reuse: bool,
    /// `--update-baseline --from`: build the baseline from these saved `results.json` files.
    from: Vec<PathBuf>,
    allow_contended: bool,
}

fn parse_opts(rest: &[&str]) -> Result<Opts, String> {
    let mut o = Opts {
        quick: false,
        update_baseline: false,
        threshold: None,
        benches: Vec::new(),
        skip_build: false,
        reuse: false,
        from: Vec::new(),
        allow_contended: false,
    };
    let mut it = rest.iter();
    while let Some(a) = it.next() {
        match *a {
            "--quick" => o.quick = true,
            "--update-baseline" => o.update_baseline = true,
            "--skip-build" => o.skip_build = true,
            "--reuse" => {
                o.reuse = true;
                o.skip_build = true;
            }
            "--threshold" => {
                let v = it.next().and_then(|v| v.parse::<f64>().ok()).filter(|v| v.is_finite() && *v > 0.0).ok_or("--threshold needs a percentage > 0")?;
                o.threshold = Some(v);
            }
            "--bench" => o.benches.push(it.next().ok_or("--bench needs a name")?.to_string()),
            "--from" => o.from.push(PathBuf::from(it.next().ok_or("--from needs a results.json path")?)),
            "--allow-contended" => o.allow_contended = true,
            other => return Err(format!("perf: unknown option `{other}`")),
        }
    }
    if !o.from.is_empty() && !o.update_baseline {
        return Err("--from builds a baseline: use it with --update-baseline".into());
    }
    Ok(o)
}

fn target_dir() -> Result<PathBuf, String> {
    let m = crate::metadata()?;
    m.get("target_directory").and_then(Value::as_str).map(PathBuf::from).ok_or_else(|| "cargo metadata: no target_directory".into())
}

fn write(path: &Path, text: &str) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
    }
    std::fs::write(path, text).map_err(|e| format!("write {}: {e}", path.display()))
}

pub fn run(root: &Path, rest: &[&str]) -> Result<(), String> {
    let o = parse_opts(rest)?;
    let text = std::fs::read_to_string(root.join(BUDGETS)).map_err(|e| format!("read {BUDGETS}: {e}"))?;
    let budgets = parse_budgets(&text)?;
    for name in &o.benches {
        if !budgets.bench.iter().any(|b| &b.name == name) {
            return Err(format!("perf: unknown bench `{name}` (see {BUDGETS})"));
        }
    }
    let baseline_value: Option<Value> = std::fs::read_to_string(root.join(BASELINE)).ok().and_then(|t| serde_json::from_str(&t).ok());
    if !o.from.is_empty() {
        return baseline_from_files(root, &budgets, baseline_value.as_ref(), &o);
    }
    let mode = if o.quick { "quick" } else { "full" };
    let threshold = o.threshold.unwrap_or(budgets.settings.regression_pct);
    let target = target_dir()?;
    let out_dir = target.join("perf");

    // Which benches run in this mode.
    let mut plan: Vec<(&BenchSpec, Vec<String>)> = Vec::new();
    let mut skipped: Vec<(String, String)> = Vec::new();
    for b in &budgets.bench {
        if !o.benches.is_empty() && !o.benches.contains(&b.name) {
            continue;
        }
        let args = if o.quick {
            match &b.quick_args {
                Some(a) => a.clone(),
                None => continue,
            }
        } else {
            b.args.clone()
        };
        if let Some(req) = &b.requires
            && !root.join(req).exists()
        {
            skipped.push((b.name.clone(), format!("{req} not on this branch yet")));
            continue;
        }
        plan.push((b, args));
    }

    // Sampled before the build: the harness's own compile isn't contention. The load after the
    // build is still recorded, as the first bench's `load_avg_before`.
    let load_start = load_avg();
    if !o.skip_build {
        let mut by_pkg: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
        for (b, _) in &plan {
            by_pkg.entry(b.package.as_str()).or_default().push(b.example());
        }
        for (pkg, examples) in by_pkg {
            let mut c = crate::cargo();
            c.args(["build", "--release", "-p", pkg]);
            for e in &examples {
                c.args(["--example", e]);
            }
            crate::run(c, &format!("cargo build --release -p {pkg} --example {}", examples.join(" --example ")))?;
        }
    }

    let mut reports: BTreeMap<String, Result<Value, String>> = BTreeMap::new();
    let mut bench_meta = serde_json::Map::new();
    for (b, args) in &plan {
        let json_path = out_dir.join(format!("{}.json", b.name));
        let before = load_avg();
        let t = Instant::now();
        let result = if o.reuse {
            std::fs::read_to_string(&json_path).map_err(|e| format!("--reuse: {}: {e}", json_path.display()))
        } else {
            let _ = std::fs::remove_file(&json_path);
            if let Some(dir) = json_path.parent() {
                std::fs::create_dir_all(dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
            }
            let exe = target.join("release").join("examples").join(format!("{}{}", b.example(), std::env::consts::EXE_SUFFIX));
            let mut c = Command::new(&exe);
            c.current_dir(root).args(args).arg("--json").arg(&json_path);
            let shown = format!("{} {} --json {}", b.example(), args.join(" "), json_path.display());
            crate::run(c, &shown).and_then(|()| std::fs::read_to_string(&json_path).map_err(|e| format!("{}: no JSON report ({e})", b.name)))
        };
        let parsed = result.and_then(|t| serde_json::from_str::<Value>(&t).map_err(|e| format!("{}: bad JSON: {e}", b.name)));
        let after = load_avg();
        bench_meta.insert(
            b.name.clone(),
            json!({
                "status": if parsed.is_ok() { "ok" } else { "error" },
                "note": b.note,
                "error": parsed.as_ref().err(),
                "seconds": t.elapsed().as_secs_f64(),
                "args": args,
                "load_avg_before": before,
                "load_avg_after": after,
                "context": parsed.as_ref().ok().and_then(|r| r.get("context")).cloned(),
                "process_peak_rss_bytes": parsed.as_ref().ok().and_then(|r| r.get("process_peak_rss_bytes")).cloned(),
                // Every row, mapped to a scenario or not (new benches show up here first).
                "rows": parsed.as_ref().ok().and_then(|r| r.get("rows")).and_then(Value::as_array).map(|rows| {
                    rows.iter()
                        .map(|r| {
                            let mut v = row_stats(r).to_json();
                            if let Some(o) = v.as_object_mut() {
                                o.insert("name".into(), r.get("name").cloned().unwrap_or(Value::Null));
                            }
                            v
                        })
                        .collect::<Vec<_>>()
                }),
            }),
        );
        reports.insert(b.name.clone(), parsed);
    }
    for (name, why) in &skipped {
        bench_meta.insert(name.clone(), json!({"status": "skipped", "error": why}));
    }
    let load_end = load_avg();

    let mut m = machine();
    let gpu = reports
        .values()
        .filter_map(|r| r.as_ref().ok())
        .find_map(|r| r.get("context").and_then(|c| c.get("gpu_adapter")).and_then(Value::as_str).map(str::to_string));
    let gpu_name = gpu.clone().unwrap_or_else(|| "cpu-canvas".into());
    let class = machine_class(
        m.get("os").and_then(Value::as_str).unwrap_or(""),
        m.get("arch").and_then(Value::as_str).unwrap_or(""),
        m.get("cpu").and_then(Value::as_str).unwrap_or(""),
        &gpu_name,
    );
    if let Some(obj) = m.as_object_mut() {
        obj.insert("gpu_adapter".into(), json!(gpu));
        obj.insert("class".into(), json!(class));
    }

    let (baseline, baseline_note) = match baseline_value.as_ref().map(|b| baseline_for(b, &class, mode)) {
        Some(Ok(b)) => (Some(b), None),
        Some(Err(e)) => (None, Some(e)),
        None => (None, Some(format!("no {BASELINE}: regression check skipped"))),
    };
    let outcomes = evaluate(&budgets, &reports, baseline.as_ref(), threshold, !o.quick);

    let scenarios: serde_json::Map<String, Value> = budgets
        .scenario
        .iter()
        .zip(&outcomes)
        .map(|(sc, oc)| {
            let mut v = oc.stats.as_ref().map(Stats::to_json).unwrap_or_else(|| json!({}));
            if let Some(obj) = v.as_object_mut() {
                obj.insert("title".into(), json!(sc.title));
                obj.insert("issue".into(), json!(sc.issue));
                obj.insert("bench".into(), json!(sc.bench));
                obj.insert("row".into(), json!(sc.row));
                obj.insert("metric".into(), json!(sc.metric.label()));
                obj.insert("budget_ms".into(), json!(sc.budget_ms));
                obj.insert("target".into(), json!(sc.target_text()));
                obj.insert("enforce".into(), json!(sc.enforce));
                obj.insert("baseline_p50_ms".into(), json!(oc.baseline_p50));
                obj.insert("change_pct".into(), json!(oc.change_pct));
                obj.insert("status".into(), json!(oc.status.label()));
                if let Status::NotMeasurable(w) = &oc.status {
                    obj.insert("not_measurable".into(), json!(w));
                }
            }
            (sc.id.clone(), v)
        })
        .collect();
    let failures: Vec<String> = outcomes.iter().filter(|o| o.status.fails()).map(|o| format!("{} {}: {}", o.id, o.title, o.status.label())).collect();
    let bench_failures: Vec<String> = reports.iter().filter_map(|(n, r)| r.as_ref().err().map(|e| format!("bench {n}: {e}"))).collect();
    let commit = git(root, &["rev-parse", "HEAD"]);
    let dirty = git(root, &["status", "--porcelain"]).is_some_and(|s| !s.is_empty());
    let is_contended = contended(load_start, m.get("cores").and_then(Value::as_u64).map(|c| c as usize), budgets.settings.max_load_per_core);
    let results = json!({
        "schema": 1,
        "mode": mode,
        "date": now_iso(),
        "commit": commit,
        "dirty": dirty,
        "machine": m,
        "load_avg_start": load_start,
        "load_avg_end": load_end,
        "contended": is_contended,
        "threshold_pct": threshold,
        "baseline_note": baseline_note,
        "benches": bench_meta,
        "scenarios": scenarios,
        "failures": failures.iter().chain(&bench_failures).collect::<Vec<_>>(),
    });
    let text = serde_json::to_string_pretty(&results).map_err(|e| format!("encode results: {e}"))?;
    write(&out_dir.join("results.json"), &text)?;

    let summary = render_summary(&results, &outcomes, baseline_note.as_deref());
    write(&out_dir.join("summary.md"), &summary)?;
    println!("\n{summary}");
    println!("wrote {}", out_dir.join("results.json").display());

    let mut errors: Vec<String> = Vec::new();
    if o.update_baseline {
        // The results are written either way; a contended run never silently becomes the baseline.
        let rules = BaselineRules { min_runs: 1, max_load_per_core: budgets.settings.max_load_per_core, allow_contended: o.allow_contended };
        match baseline_from_runs(baseline_value.as_ref(), std::slice::from_ref(&results), &rules) {
            Ok(nb) => {
                write_baseline(root, &nb)?;
                println!("updated {BASELINE} ({mode}, machine class {class}{})", if is_contended { ", labelled contended" } else { "" });
            }
            // A refused baseline still reports this run's perf failures below.
            Err(e) => errors.push(e),
        }
    }
    let all: Vec<&String> = failures.iter().chain(&bench_failures).collect();
    if !all.is_empty() {
        errors.push(format!("{} perf failure(s):\n  {}", all.len(), all.iter().map(|s| s.as_str()).collect::<Vec<_>>().join("\n  ")));
    }
    if errors.is_empty() { Ok(()) } else { Err(errors.join("\n")) }
}

fn write_baseline(root: &Path, baseline: &Value) -> Result<(), String> {
    let text = serde_json::to_string_pretty(baseline).map_err(|e| format!("encode baseline: {e}"))?;
    write(&root.join(BASELINE), &format!("{text}\n"))
}

/// `--update-baseline --from <results.json>...`: no benches run. Prints the enforcement report.
fn baseline_from_files(root: &Path, budgets: &Budgets, old: Option<&Value>, o: &Opts) -> Result<(), String> {
    let runs = o
        .from
        .iter()
        .map(|p| {
            let t = std::fs::read_to_string(p).map_err(|e| format!("read {}: {e}", p.display()))?;
            serde_json::from_str::<Value>(&t).map_err(|e| format!("{}: {e}", p.display()))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let (could, flaky) = enforce_report(budgets, &runs);
    let list = |v: &[String]| if v.is_empty() { "  none".to_string() } else { v.iter().map(|l| format!("  {l}")).collect::<Vec<_>>().join("\n") };
    println!("Budgets that could be enforced (within {} of the budget in every run):\n{}", budgets.settings.enforce_margin, list(&could));
    println!("Enforced budgets that went over in some run (flaky):\n{}", list(&flaky));
    let rules = BaselineRules {
        min_runs: budgets.settings.baseline_min_runs,
        max_load_per_core: budgets.settings.max_load_per_core,
        allow_contended: o.allow_contended,
    };
    let nb = baseline_from_runs(old, &runs, &rules)?;
    write_baseline(root, &nb)?;
    println!("updated {BASELINE} from {} runs (machine class {})", runs.len(), nb.get("machine_class").and_then(Value::as_str).unwrap_or("?"));
    Ok(())
}

/// The committed baseline's full-mode numbers judged against the budgets (what the scorecard
/// shows: deterministic, no run needed). Scenarios without a baseline number are "not run".
pub fn outcomes_from_baseline(budgets: &Budgets, baseline: Option<&Value>) -> Vec<Outcome> {
    let mode = baseline.and_then(|b| b.get("modes")?.get("full"));
    let full: Option<BTreeMap<String, Stats>> =
        mode.and_then(|m| m.get("scenarios")?.as_object().cloned()).map(|m| m.iter().map(|(k, v)| (k.clone(), Stats::from_json(v))).collect());
    let failed = |id: &str| mode.and_then(|m| m.get("failed")?.get(id)?.as_str()).map(str::to_string);
    budgets
        .scenario
        .iter()
        .map(|sc| {
            let stats = full.as_ref().and_then(|f| f.get(&sc.id)).cloned();
            let status = match (&sc.not_measurable, &stats) {
                (Some(w), _) => Status::NotMeasurable(w.clone()),
                (None, Some(st)) => verdict(sc, st, None, &budgets.settings, budgets.settings.regression_pct, true),
                (None, None) => failed(&sc.id).map_or(Status::NotRun, Status::Missing),
            };
            Outcome {
                id: sc.id.clone(),
                title: sc.title.clone(),
                issue: sc.issue,
                target: sc.target_text(),
                stats,
                baseline_p50: None,
                change_pct: None,
                status,
            }
        })
        .collect()
}

pub fn fmt_load(v: Option<&Value>) -> String {
    v.and_then(Value::as_array).map_or_else(|| "n/a".into(), |a| a.iter().filter_map(Value::as_f64).map(|x| format!("{x:.2}")).collect::<Vec<_>>().join(" / "))
}

/// The Markdown summary of one run: context lines and the scenario table.
pub fn render_summary(results: &Value, outcomes: &[Outcome], baseline_note: Option<&str>) -> String {
    let m = results.get("machine").cloned().unwrap_or(Value::Null);
    let s = |v: Option<&Value>| v.and_then(Value::as_str).unwrap_or("unknown").to_string();
    let ram = m.get("ram_bytes").and_then(Value::as_u64).map_or_else(|| "?".into(), |b| format!("{} GB", b >> 30));
    let mut out = format!(
        "## Performance ({} run)\n\n- Machine: {} ({} cores, {ram}), GPU {}, {} {}; class `{}`\n- Commit {}{}, {}\n- Load average (1/5/15 min): {} at start, {} at end\n",
        s(results.get("mode")),
        s(m.get("cpu")),
        m.get("cores").and_then(Value::as_u64).unwrap_or(0),
        s(m.get("gpu_adapter")),
        s(m.get("os_version")),
        s(m.get("arch")),
        s(m.get("class")),
        s(results.get("commit")),
        if results.get("dirty").and_then(Value::as_bool) == Some(true) { " (dirty)" } else { "" },
        s(results.get("date")),
        fmt_load(results.get("load_avg_start")),
        fmt_load(results.get("load_avg_end")),
    );
    if results.get("contended").and_then(Value::as_bool) == Some(true) {
        out.push_str("- **contended run**: numbers measure machine load (the 1-minute load at the start was above `max_load_per_core` per core)\n");
    }
    if let Some(n) = baseline_note {
        out.push_str(&format!("- {n}\n"));
    }
    if results.get("mode").and_then(Value::as_str) == Some("quick") {
        out.push_str("- Quick mode: small documents, budgets not applied (they are set for full-size documents).\n");
    }
    if let Some(b) = results.get("benches").and_then(Value::as_object) {
        for (name, v) in b {
            let st = v.get("status").and_then(Value::as_str).unwrap_or("?");
            let secs = v.get("seconds").and_then(Value::as_f64).map_or_else(String::new, |x| format!(", {x:.0} s"));
            let err = v.get("error").and_then(Value::as_str).map_or_else(String::new, |e| format!(": {e}"));
            out.push_str(&format!("- Bench `{name}`: {st}{secs}, load {}{err}\n", fmt_load(v.get("load_avg_before"))));
        }
    }
    out.push('\n');
    out.push_str(&render_table(outcomes, true));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iso_utc_handles_boundaries() {
        assert_eq!(iso_utc(0), "1970-01-01T00:00:00Z");
        assert_eq!(iso_utc(1_791_446_357), "2026-10-08T07:59:17Z");
        // Years past 9999 keep the plain zero-padded year (no `+`).
        assert_eq!(iso_utc(253_402_300_800), "10000-01-01T00:00:00Z");
        // A value past chrono's range still converts via the 400-year cycle (the old hand-rolled
        // `iso_utc` did too): pin that output, never the epoch.
        assert_eq!(iso_utc(10_000_000_000_000), "318857-05-20T17:46:40Z");
        assert_eq!(iso_utc(u64::MAX), "584554051223-11-09T07:00:15Z");
    }

    const TOML: &str = r#"
[settings]
regression_pct = 15.0
regression_min_ms = 0.5

[[bench]]
name = "b"
package = "p"
quick_args = ["--quick"]

[[scenario]]
id = "P1"
title = "move"
issue = 209
bench = "b"
row = "move row"
budget_ms = 25

[[scenario]]
id = "P2"
title = "opacity"
bench = "b"
row = "opacity row"
budget_ms = 20
enforce = false

[[scenario]]
id = "P3"
title = "zoom"
target = "≤ 16 ms per frame"
not_measurable = "needs a frame harness"
"#;

    fn report(rows: Value) -> Value {
        json!({"bench": "b", "rows": rows})
    }

    #[test]
    fn budgets_parse_and_validate() {
        let b = parse_budgets(TOML).unwrap();
        assert_eq!(b.scenario.len(), 3);
        assert_eq!(b.scenario[0].metric, Metric::P95);
        assert!(b.scenario[0].enforce);
        assert!(!b.scenario[1].enforce);
        assert_eq!(b.scenario[0].target_text(), "≤ 25 ms p95");
        assert_eq!(b.scenario[2].target_text(), "≤ 16 ms per frame");
        assert_eq!(b.settings.regression_min_ms, 0.5);
        // Duplicate ids, undefined benches, half-specified rows and bad budgets are rejected.
        assert!(parse_budgets(&format!("{TOML}\n[[scenario]]\nid = \"P1\"\ntitle = \"x\"\nnot_measurable = \"y\"\n")).is_err());
        assert!(parse_budgets("[[scenario]]\nid = \"P1\"\ntitle = \"x\"\nbench = \"nope\"\nrow = \"r\"\n").is_err());
        assert!(parse_budgets("[[bench]]\nname = \"b\"\npackage = \"p\"\n[[scenario]]\nid = \"P1\"\ntitle = \"x\"\nbench = \"b\"\n").is_err());
        assert!(
            parse_budgets("[[bench]]\nname = \"b\"\npackage = \"p\"\n[[scenario]]\nid = \"P1\"\ntitle = \"x\"\nbench = \"b\"\nrow = \"r\"\nbudget_ms = -1\n")
                .is_err()
        );
        assert!(parse_budgets("[settings]\nregression_pct = 0\n").is_err());
        assert!(parse_budgets("[settings]\nregression_min_ms = -1\n").is_err());
        assert!(parse_budgets("[settings]\nregression_min_ms = 0\n").is_ok());
        assert!(parse_budgets("bogus = 1\n").is_err());
        assert!(parse_budgets("not toml [").is_err());
    }

    #[test]
    fn row_stats_prefers_samples_and_never_invents_p95() {
        let s = row_stats(&json!({"name": "x", "samples_ms": [5.0, 1.0, 3.0, 2.0, 4.0], "p50_ms": 99.0}));
        assert_eq!((s.n, s.p50, s.p95, s.max), (5, Some(3.0), Some(5.0), Some(5.0)));
        // Older rows: median + max only. p95 stays empty.
        let s = row_stats(&json!({"name": "x", "median_ms": 4.0, "min_ms": 1.0, "max_ms": 9.0}));
        assert_eq!((s.p50, s.p95, s.max), (Some(4.0), None, Some(9.0)));
        let s = row_stats(&json!({"name": "x", "samples_ms": ["bad", null]}));
        assert_eq!(s.p50, None);
    }

    #[test]
    fn regression_detection() {
        assert!(!regressed(11.4, 10.0, 15.0, 0.5));
        assert!(regressed(11.6, 10.0, 15.0, 0.5));
        // Below the absolute noise floor: no regression.
        assert!(!regressed(0.30, 0.20, 15.0, 0.5));
        assert!(!regressed(5.0, 0.0, 15.0, 0.5));
        assert!(!regressed(f64::NAN, 1.0, 15.0, 0.0));
        assert_eq!(change_pct(12.0, 10.0).map(|v| v.round()), Some(20.0));
        assert_eq!(change_pct(1.0, 0.0), None);
    }

    #[test]
    fn evaluate_budgets_regressions_and_missing_rows() {
        let b = parse_budgets(TOML).unwrap();
        let mut reports = BTreeMap::new();
        reports.insert(
            "b".to_string(),
            Ok(report(json!([{"name": "move row", "samples_ms": [10.0, 12.0, 30.0]}, {"name": "opacity row", "samples_ms": [40.0]}]))),
        );
        let out = evaluate(&b, &reports, None, 15.0, true);
        assert_eq!(out[0].status, Status::Broken, "p95 30 > 25 and enforced");
        assert_eq!(out[1].status, Status::Over, "not enforced: reported, doesn't fail");
        assert!(matches!(out[2].status, Status::NotMeasurable(_)));
        assert!(out[0].status.fails() && !out[1].status.fails() && !out[2].status.fails());

        // Quick mode: no budgets, only regressions.
        let mut base = BTreeMap::new();
        base.insert("P1".to_string(), Stats { p50: Some(10.0), ..Default::default() });
        base.insert("P2".to_string(), Stats { p50: Some(30.0), ..Default::default() });
        let out = evaluate(&b, &reports, Some(&base), 15.0, false);
        assert_eq!(out[0].status, Status::Regressed, "p50 12 vs 10 is +20 %");
        assert_eq!(out[1].status, Status::Regressed, "p50 40 vs 30");
        let out = evaluate(&b, &reports, Some(&base), 50.0, false);
        assert_eq!(out[0].status, Status::Tracked, "threshold is configurable");

        // A row the bench didn't produce fails, with the bench's own error when it has one.
        let mut reports = BTreeMap::new();
        reports.insert("b".to_string(), Ok(json!({"rows": [], "context": {"errors": [{"name": "move row", "error": "boom"}]}})));
        let out = evaluate(&b, &reports, None, 15.0, true);
        assert_eq!(out[0].status, Status::Missing("boom".into()));
        assert!(matches!(&out[1].status, Status::Missing(e) if e.contains("not in")));
        // A bench that didn't run in this mode: not run, not a failure.
        let out = evaluate(&b, &BTreeMap::new(), None, 15.0, true);
        assert_eq!(out[0].status, Status::NotRun);
        assert!(!out[0].status.fails());
        assert_eq!(counts(&out), (0, 0, 0, 1), "not-run scenarios are not tracked; the scorecard counts them separately");
    }

    fn rules(min_runs: usize, allow_contended: bool) -> BaselineRules {
        BaselineRules { min_runs, max_load_per_core: 1.0, allow_contended }
    }

    /// The plain `--update-baseline` case: one run.
    fn one(old: Option<&Value>, results: &Value) -> Value {
        baseline_from_runs(old, std::slice::from_ref(results), &rules(1, false)).unwrap()
    }

    /// A saved run of class `c` with one p50 (and p95) per scenario.
    fn run_of(date: &str, load: f64, rows: &[(&str, f64)]) -> Value {
        let sc: serde_json::Map<String, Value> =
            rows.iter().map(|(k, v)| (k.to_string(), json!({"n": 5, "p50_ms": v, "p95_ms": v * 1.5, "max_ms": v * 2.0}))).collect();
        json!({"mode": "full", "date": date, "commit": format!("c-{date}"), "dirty": false, "machine": {"class": "c", "cores": 3}, "load_avg_start": [load, load, load], "scenarios": sc})
    }

    #[test]
    fn contention_is_judged_on_start_load_per_core() {
        assert!(contended(Some([224.9, 226.31, 196.03]), Some(14), 1.0), "the committed 2026-10-05 baseline");
        assert!(!contended(Some([0.5, 3.0, 9.0]), Some(12), 1.0), "only the 1-minute load counts");
        assert!(!contended(None, Some(12), 1.0), "no load (Windows): can't judge");
        assert!(!contended(Some([50.0, 0.0, 0.0]), None, 1.0));
        assert!(!contended(Some([9.5, 0.0, 0.0]), Some(3), 4.0), "the threshold is configurable");
        assert!(contended(Some([9.5, 0.0, 0.0]), Some(3), 3.0));
        assert!(run_contended(Some(&json!([224.9, 226.31, 196.03])), Some(&json!({"cores": 14})), 1.0));
    }

    #[test]
    fn update_baseline_refuses_contended_run_unless_allowed() {
        let mut r = run_of("2026-10-05", 224.9, &[("P1", 3.0)]);
        r["machine"]["cores"] = json!(14);
        let e = baseline_from_runs(None, std::slice::from_ref(&r), &rules(1, false)).unwrap_err();
        assert!(e.contains("224.90") && e.contains("14 cores") && e.contains("--allow-contended"), "{e}");
        let b = baseline_from_runs(None, std::slice::from_ref(&r), &rules(1, true)).unwrap();
        let m = &b["modes"]["full"];
        assert_eq!(m["contended"], json!(true));
        assert_eq!(m["cores"], json!(14));
        assert_eq!(m["runs"], json!(1));
        // A quiet run isn't labelled, and one run has no noise ceiling.
        let b = one(None, &run_of("d", 0.5, &[("P1", 3.0)]));
        assert!(b["modes"]["full"].get("contended").is_none());
        assert!(b["modes"]["full"]["scenarios"]["P1"].get("p50_max_ms").is_none());
    }

    #[test]
    fn baseline_from_runs_takes_median_and_noise_ceiling() {
        let runs: Vec<Value> = [(1, 2174.0, 30.5), (2, 2868.0, 45.3), (3, 2000.0, 28.0), (4, 2500.0, 31.0), (5, 2300.0, 40.0)]
            .iter()
            .map(|(d, a, b)| run_of(&format!("2026-10-2{d}"), 1.0, &[("P44", *a), ("P46", *b)]))
            .collect();
        let b = baseline_from_runs(None, &runs, &rules(5, false)).unwrap();
        let m = &b["modes"]["full"];
        assert_eq!(m["runs"], json!(5));
        assert_eq!(m["date"], json!("2026-10-25"), "the latest run dates the baseline");
        assert_eq!(m["run_list"].as_array().map(Vec::len), Some(5));
        assert_eq!(m["run_list"][0]["load_avg_start"], json!([1.0, 1.0, 1.0]));
        assert_eq!(m["run_list"][0]["contended"], json!(false));
        let got = baseline_for(&b, "c", "full").unwrap();
        let p44 = &got["P44"];
        assert_eq!((p44.p50, p44.p50_max, p44.n), (Some(2300.0), Some(2868.0), 25));
        assert_eq!(got["P46"].p50, Some(31.0));
        assert_eq!(median(&mut [4.0, 1.0, 3.0, 2.0]), Some(2.5));
    }

    #[test]
    fn baseline_from_runs_keeps_a_row_failed_in_any_run() {
        let mut runs: Vec<Value> = (1..=3).map(|d| run_of(&format!("2026-10-2{d}"), 1.0, &[("P1", 3.0), ("P2", 5.0)])).collect();
        runs[1]["scenarios"]["P1"] = json!({"status": format!("{MISSING_PREFIX}bench crashed")});
        let b = baseline_from_runs(None, &runs, &rules(3, false)).unwrap();
        let m = &b["modes"]["full"];
        assert!(m["scenarios"].get("P1").is_none(), "no partial aggregate: {m}");
        assert_eq!(m["failed"]["P1"], json!("bench crashed"));
        assert_eq!(m["scenarios"]["P2"]["p50_ms"], json!(5.0));
        assert!(m["failed"].get("P2").is_none());
    }

    #[test]
    fn baseline_from_runs_leaves_out_a_row_missing_from_some_runs() {
        let mut runs: Vec<Value> = (1..=3).map(|d| run_of(&format!("2026-10-2{d}"), 1.0, &[("P1", 3.0), ("P2", 5.0)])).collect();
        runs[0]["scenarios"].as_object_mut().unwrap().remove("P1");
        let b = baseline_from_runs(None, &runs, &rules(3, false)).unwrap();
        let m = &b["modes"]["full"];
        assert!(m["scenarios"].get("P1").is_none(), "2 of 3 runs is no baseline: {m}");
        assert_eq!(m["scenarios"]["P2"]["p50_ms"], json!(5.0));
        assert_eq!(m["runs"], json!(3));
    }

    #[test]
    fn baseline_from_runs_rejects_mixed_classes_modes_dirty_and_too_few() {
        let runs: Vec<Value> = (1..=5).map(|d| run_of(&format!("2026-10-2{d}"), 1.0, &[("P1", 3.0)])).collect();
        assert!(baseline_from_runs(None, &runs, &rules(5, false)).is_ok());
        assert!(baseline_from_runs(None, &runs[..4], &rules(5, false)).unwrap_err().contains("at least 5"));
        let mut m = runs.clone();
        m[2]["machine"]["class"] = json!("other");
        assert!(baseline_from_runs(None, &m, &rules(5, false)).unwrap_err().contains("machine classes"));
        let mut m = runs.clone();
        m[2]["mode"] = json!("quick");
        assert!(baseline_from_runs(None, &m, &rules(5, false)).unwrap_err().contains("modes"));
        let mut m = runs.clone();
        m[2]["dirty"] = json!(true);
        assert!(baseline_from_runs(None, &m, &rules(5, false)).unwrap_err().contains("dirty"));
        let mut m = runs.clone();
        m[2]["load_avg_start"] = json!([9.5, 9.0, 9.0]);
        assert!(baseline_from_runs(None, &m, &rules(5, false)).unwrap_err().contains("contended"));
        assert_eq!(baseline_from_runs(None, &m, &rules(5, true)).unwrap()["modes"]["full"]["run_list"][2]["contended"], json!(true));
    }

    #[test]
    fn regression_is_judged_against_the_noise_ceiling() {
        // Runner numbers (macos-15, 7 nightlies): median and highest p50 per row.
        let b = parse_budgets(TOML).unwrap();
        let s = &b.settings;
        let sc = &b.scenario[1]; // no enforced budget in play: opacity, budget 20, not enforced
        let base = |p50: f64, max: f64| Stats { p50: Some(p50), p50_max: Some(max), ..Default::default() };
        let cur = |p50: f64| Stats { p50: Some(p50), ..Default::default() };
        assert_eq!(verdict(sc, &cur(3935.0), Some(&base(2174.0, 2868.0)), s, 15.0, false), Status::Regressed, "P44 +81 % (#5694)");
        assert_eq!(verdict(sc, &cur(76.9), Some(&base(30.5, 45.3)), s, 15.0, false), Status::Regressed, "P46 +152 % (#5697)");
        assert_eq!(verdict(sc, &cur(387.0), Some(&base(258.0, 448.0)), s, 15.0, false), Status::Tracked, "P11 +50 % (#5695) is within runner noise");
        // A single-run baseline falls back to its p50.
        let single = Stats { p50: Some(258.0), ..Default::default() };
        assert_eq!(verdict(sc, &cur(387.0), Some(&single), s, 15.0, false), Status::Regressed);
    }

    #[test]
    fn enforce_report_needs_margin_in_every_run() {
        let toml = r#"
[[bench]]
name = "b"
package = "p"

[[scenario]]
id = "P16"
title = "dab"
bench = "b"
row = "r16"
budget_ms = 100
enforce = false

[[scenario]]
id = "P29"
title = "wand"
bench = "b"
row = "r29"
budget_ms = 100
enforce = false

[[scenario]]
id = "P30"
title = "liquify"
bench = "b"
row = "r30"
budget_ms = 100
enforce = false

[[scenario]]
id = "P25"
title = "cancel"
bench = "b"
row = "r25"
budget_ms = 200

[[scenario]]
id = "P99"
title = "track"
bench = "b"
row = "r99"
"#;
        let b = parse_budgets(toml).unwrap();
        // run_of stores p95 = 1.5 × p50 (the default metric is p95).
        let runs: Vec<Value> = [(30.0, 20.0, 30.0, 100.0), (35.2, 22.0, 45.0, 110.0), (20.0, 22.0, 30.0, 199.4)]
            .iter()
            .enumerate()
            .map(|(i, (a, c, d, e))| run_of(&format!("d{i}"), 1.0, &[("P16", *a), ("P29", *c), ("P30", *d), ("P25", *e), ("P99", 1.0)]))
            .collect();
        let (could, flaky) = enforce_report(&b, &runs);
        assert_eq!(could.len(), 2, "{could:?}");
        assert!(could[0].starts_with("P16 ") && could[0].contains("52.8"), "{could:?}");
        assert!(could[1].starts_with("P29 "));
        assert!(!could.iter().any(|l| l.starts_with("P30")), "67.5 > 60 once: not suggested");
        assert_eq!(flaky.len(), 1, "{flaky:?}");
        assert!(flaky[0].starts_with("P25 ") && flaky[0].contains("in 1 of 3 runs"), "{flaky:?}");
        // A row missing from one run can't be vouched for.
        let mut gap = runs.clone();
        if let Some(o) = gap[1]["scenarios"].as_object_mut() {
            o.remove("P29");
        }
        assert!(!enforce_report(&b, &gap).0.iter().any(|l| l.starts_with("P29")));
    }

    #[test]
    fn baselines_never_cross_machine_classes() {
        let results = json!({"mode": "full", "date": "d", "commit": "c", "machine": {"class": "macos-aarch64-m4"}, "scenarios": {"P1": {"p50_ms": 3.0, "p95_ms": 4.0, "max_ms": 5.0, "n": 3}, "P3": {"status": "not measurable yet"}}});
        let b = one(None, &results);
        let got = baseline_for(&b, "macos-aarch64-m4", "full").unwrap();
        assert_eq!(got.get("P1").and_then(|s| s.p50), Some(3.0));
        assert!(!got.contains_key("P3"), "rows without numbers are not baselined");
        assert!(baseline_for(&b, "linux-x86-64-epyc", "full").is_err());
        assert!(baseline_for(&b, "macos-aarch64-m4", "quick").is_err());
        // Same class: the other mode is kept. New class: replaced.
        let quick = json!({"mode": "quick", "machine": {"class": "macos-aarch64-m4"}, "scenarios": {"P1": {"p50_ms": 1.0}}});
        let b2 = one(Some(&b), &quick);
        assert!(baseline_for(&b2, "macos-aarch64-m4", "full").is_ok());
        assert!(baseline_for(&b2, "macos-aarch64-m4", "quick").is_ok());
        // A crashed row is kept as a failure, and the scorecard shows it.
        let crashed = json!({"mode": "full", "machine": {"class": "c"}, "scenarios": {"P1": {"status": "**missing**: panicked: boom"}}});
        let bc = one(None, &crashed);
        let b = parse_budgets(TOML).unwrap();
        let o = outcomes_from_baseline(&b, Some(&bc));
        assert_eq!(o[0].status, Status::Missing("panicked: boom".into()));
        assert_eq!(o[1].status, Status::NotRun);
        let other = json!({"mode": "quick", "machine": {"class": "other"}, "scenarios": {}});
        let b3 = one(Some(&b2), &other);
        assert!(baseline_for(&b3, "other", "full").is_err());
    }

    #[test]
    fn machine_helpers() {
        assert_eq!(slug("Apple M4 Pro (Metal)"), "apple-m4-pro-metal");
        assert_eq!(machine_class("macos", "aarch64", "Apple M4 Pro", "Apple M4 Pro"), "macos-aarch64-apple-m4-pro-apple-m4-pro");
        assert_eq!(parse_loadavg("{ 2.10 1.50 0.75 }"), Some([2.1, 1.5, 0.75]));
        assert_eq!(parse_loadavg("0.52 0.58 0.59 1/467 12345\n"), Some([0.52, 0.58, 0.59]));
        assert_eq!(parse_loadavg("garbage"), None);
        assert_eq!(iso_utc(0), "1970-01-01T00:00:00Z");
        assert_eq!(iso_utc(1_791_158_400), "2026-10-05T00:00:00Z");
        assert_eq!(iso_utc(951_782_400), "2000-02-29T00:00:00Z");
    }

    #[test]
    fn table_lists_not_measurable_rows_separately() {
        let b = parse_budgets(TOML).unwrap();
        let mut reports = BTreeMap::new();
        reports.insert("b".to_string(), Ok(report(json!([{"name": "move row", "samples_ms": [10.0], "peak_rss_bytes": 3u64 << 30}]))));
        let t = render_table(&evaluate(&b, &reports, None, 15.0, true), true);
        assert!(t.contains("| P1 | move | ≤ 25 ms p95 | 10.0 | 10.0 | 10.0 | 3.0 GB |"));
        assert!(t.contains("**Not measurable yet**"));
        assert!(t.contains("| P3 | zoom | ≤ 16 ms per frame | needs a frame harness |  |"));
        assert_eq!(counts(&evaluate(&b, &reports, None, 15.0, true)), (1, 1, 0, 1), "the row the bench lacks counts as failing");
    }

    #[test]
    fn unknown_bench_is_rejected_before_updating_the_baseline() {
        let root = std::env::temp_dir().join(format!("photocraft-perf-unknown-bench-{}", std::process::id()));
        std::fs::create_dir(&root).unwrap();
        std::fs::create_dir(root.join("perf")).unwrap();
        std::fs::write(root.join(BUDGETS), "[[bench]]\nname = \"known\"\npackage = \"unused\"\nrequires = \"missing.rs\"\n").unwrap();
        let baseline = "{\"schema\":1,\"sentinel\":true}";
        std::fs::write(root.join(BASELINE), baseline).unwrap();
        let results = [
            run(&root, &["--bench", "unknown", "--reuse", "--update-baseline"]),
            run(&root, &["--bench", "known", "--bench", "unknown", "--quick", "--reuse", "--update-baseline"]),
        ];
        let after = std::fs::read_to_string(root.join(BASELINE)).unwrap();
        std::fs::remove_dir_all(root).unwrap();
        for result in results {
            assert_eq!(result, Err(format!("perf: unknown bench `unknown` (see {BUDGETS})")));
        }
        assert_eq!(after, baseline, "an unknown bench must not update the baseline");
    }

    #[test]
    fn options() {
        assert!(parse_opts(&["--bogus"]).is_err());
        assert!(parse_opts(&["--threshold", "x"]).is_err());
        assert!(parse_opts(&["--threshold", "-3"]).is_err());
        let o = parse_opts(&["--quick", "--threshold", "20", "--bench", "a", "--reuse"]).unwrap();
        assert!(o.quick && o.reuse && o.skip_build);
        assert_eq!(o.threshold, Some(20.0));
        assert_eq!(o.benches, ["a"]);
        assert!(!o.allow_contended && o.from.is_empty());
        let o = parse_opts(&["--update-baseline", "--from", "a.json", "--from", "b.json", "--allow-contended"]).unwrap();
        assert!(o.update_baseline && o.allow_contended);
        assert_eq!(o.from, [PathBuf::from("a.json"), PathBuf::from("b.json")]);
        assert!(parse_opts(&["--from", "a.json"]).is_err(), "--from needs --update-baseline");
        assert!(parse_opts(&["--update-baseline", "--from"]).is_err());
    }
}
