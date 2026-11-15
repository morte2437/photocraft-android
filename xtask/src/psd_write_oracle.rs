//! Execute small PSD export recipes with the headless CLI. Photoshop is a separate judge.

use std::collections::HashSet;
use std::fs::{self, File, OpenOptions};
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

const MAX_FILE: u64 = 64 * 1024 * 1024;
const MAX_RECIPES: usize = 1000;
const TIMEOUT: Duration = Duration::from_secs(120);

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Recipes {
    version: u32,
    corpus_revision: String,
    cases: Vec<Recipe>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Recipe {
    id: String,
    base: String,
    base_sha256: String,
    target: Target,
    #[serde(default)]
    commands: Vec<Step>,
    checks: Vec<Check>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Target {
    kind: String,
    #[serde(default)]
    name: Option<String>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Step {
    command: String,
    #[serde(default = "empty_object")]
    params: Value,
}

/// Fallback serde default producing an empty JSON object.
fn empty_object() -> Value {
    json!({})
}

#[derive(Deserialize, Serialize)]
#[serde(tag = "op", deny_unknown_fields)]
enum Check {
    #[serde(rename = "changed_to")]
    ChangedTo { path: String, value: Value },
    #[serde(rename = "unchanged")]
    Unchanged { path: String },
    #[serde(rename = "equals")]
    Equals { path: String, value: Value },
}

/// Reject nonregular files and files whose metadata exceeds the caller’s byte limit.
fn read_limited_to(path: &Path, max: u64) -> Result<Vec<u8>, String> {
    let meta = fs::metadata(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if !meta.is_file() || meta.len() > max {
        return Err(format!("{} is not a regular file of at most {max} bytes", path.display()));
    }
    fs::read(path).map_err(|e| format!("{}: {e}", path.display()))
}

/// Read one recipe artifact within the runner’s file-size limit.
fn read_limited(path: &Path) -> Result<Vec<u8>, String> {
    read_limited_to(path, MAX_FILE)
}

/// Accept a case identifier that can safely name one output subdirectory.
fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 100 && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// Reject unsupported recipes, unsafe paths, duplicate IDs, and incomplete checks.
fn validate(recipes: &Recipes) -> Result<(), String> {
    if recipes.version != 1 || recipes.corpus_revision.is_empty() || recipes.cases.is_empty() || recipes.cases.len() > MAX_RECIPES {
        return Err("expected version 1, a corpus revision and 1..=1000 cases".into());
    }
    let mut seen = HashSet::new();
    for case in &recipes.cases {
        // An id names the case's output directory, and `A` and `a` are one directory on a case-insensitive filesystem.
        if !valid_id(&case.id) || !seen.insert(case.id.to_ascii_lowercase()) {
            return Err(format!("invalid or duplicate case id: {}", case.id));
        }
        let path = Path::new(&case.base);
        if path.components().any(|c| !matches!(c, Component::Normal(_))) || path.extension().is_none_or(|e| e != "psd") {
            return Err(format!("{}: base must be a relative PSD path", case.id));
        }
        if case.base_sha256.len() != 64 || !case.base_sha256.bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()) {
            return Err(format!("{}: invalid SHA-256", case.id));
        }
        if case.target.kind.is_empty() || case.checks.is_empty() || case.commands.len() > 50 {
            return Err(format!("{}: target, checks and at most 50 commands are required", case.id));
        }
        if !case.commands.is_empty() && !case.checks.iter().any(|c| matches!(c, Check::ChangedTo { .. })) {
            return Err(format!("{}: an edit needs a changed_to assertion", case.id));
        }
        for step in &case.commands {
            // The contour library commands edit no document, and their `path` param reaches any file.
            if !(step.command.starts_with("type.") || step.command.starts_with("layer.layerStyle."))
                || step.command.starts_with("layer.layerStyle.contours.")
                || !step.params.is_object()
                || step.params.get("layer").is_some()
                || step.params.get("layers").is_some()
            {
                return Err(format!("{}: invalid command or explicit layer in params", case.id));
            }
        }
        for check in &case.checks {
            let path = match check {
                Check::ChangedTo { path, .. } | Check::Unchanged { path } | Check::Equals { path, .. } => path,
            };
            if !path.starts_with('/') || path.len() > 512 {
                return Err(format!("{}: check path must be a JSON pointer", case.id));
            }
        }
    }
    Ok(())
}

/// Resolve a corpus input and reject paths escaping the canonical corpus root.
fn input_path(root: &Path, relative: &str) -> Result<PathBuf, String> {
    let p = root.join(relative).canonicalize().map_err(|e| format!("{relative}: {e}"))?;
    if !p.starts_with(root) {
        return Err(format!("{relative}: path escapes corpus directory"));
    }
    Ok(p)
}

/// Find exactly one layer matching the recipe’s kind and optional name.
fn target<'a>(doc: &'a Value, wanted: &Target) -> Result<&'a Value, String> {
    let mut stack = vec![(doc.get("layers").and_then(Value::as_array).ok_or("missing layers")?, 0usize)];
    let mut found = Vec::new();
    while let Some((layers, depth)) = stack.pop() {
        if depth > 128 {
            return Err("layer nesting exceeds 128".into());
        }
        for layer in layers {
            if layer.get("kind").and_then(Value::as_str) == Some(wanted.kind.as_str())
                && wanted.name.as_ref().is_none_or(|name| layer.get("name").and_then(Value::as_str) == Some(name.as_str()))
            {
                found.push(layer);
            }
            if let Some(children) = layer.get("children").and_then(Value::as_array) {
                stack.push((children, depth + 1));
            }
        }
    }
    if found.len() != 1 {
        return Err(format!("target matched {} layers, expected exactly one", found.len()));
    }
    found.into_iter().next().ok_or_else(|| "missing target".into())
}

/// Compare two JSON numbers by exact value, with no tolerance: `17` equals `17.0`, and integers too
/// large for `f64` stay distinct.
fn numbers_equal(a: &serde_json::Number, b: &serde_json::Number) -> bool {
    let int = |n: &serde_json::Number| n.as_i64().map(i128::from).or_else(|| n.as_u64().map(i128::from));
    let whole_float_is = |float: &serde_json::Number, i: i128| float.as_f64().is_some_and(|f| f.fract() == 0.0 && f as i128 == i);
    match (int(a), int(b)) {
        (Some(x), Some(y)) => x == y,
        (Some(i), None) => whole_float_is(b, i),
        (None, Some(i)) => whole_float_is(a, i),
        (None, None) => a.as_f64() == b.as_f64(),
    }
}

/// Compare JSON by value: numbers as in `numbers_equal`; arrays and objects element-wise. Recursion is
/// bounded because serde_json's parser already limits nesting depth.
fn values_equal(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => numbers_equal(x, y),
        (Value::Array(x), Value::Array(y)) => x.len() == y.len() && x.iter().zip(y).all(|(x, y)| values_equal(x, y)),
        (Value::Object(x), Value::Object(y)) => x.len() == y.len() && x.iter().all(|(k, v)| y.get(k).is_some_and(|w| values_equal(v, w))),
        _ => a == b,
    }
}

/// `values_equal` over optional values: both absent are equal, one absent is not.
fn options_equal(a: Option<&Value>, b: Option<&Value>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => values_equal(a, b),
        _ => false,
    }
}

fn verify(checks: &[Check], before: &Value, after: &Value) -> Vec<Value> {
    let mut results = Vec::new();
    let layer = after.get("name").or_else(|| before.get("name"));
    for check in checks {
        let (path, op, expected) = match check {
            Check::ChangedTo { path, value } => (path, "changed_to", Some(value)),
            Check::Equals { path, value } => (path, "equals", Some(value)),
            Check::Unchanged { path } => (path, "unchanged", None),
        };
        let old = before.pointer(path);
        let new = after.pointer(path);
        let pass = match check {
            Check::ChangedTo { value, .. } => new.is_some_and(|n| values_equal(n, value)) && !options_equal(old, new),
            Check::Equals { value, .. } => new.is_some_and(|n| values_equal(n, value)),
            Check::Unchanged { .. } => old.is_some() && options_equal(old, new),
        };
        results.push(
            json!({"layer": layer, "path": path, "op": op, "expected": expected, "before": old, "after": new, "status": if pass { "pass" } else { "fail" }}),
        );
    }
    results
}

/// Capture into files to avoid pipe deadlocks. Terminate a stuck CLI instead of blocking a batch.
fn invoke(cli: &Path, args: &[String], dir: &Path, tag: &str) -> Result<(String, String), String> {
    let stdout = dir.join(format!("{tag}.stdout"));
    let stderr = dir.join(format!("{tag}.stderr"));
    let out = File::create(&stdout).map_err(|e| e.to_string())?;
    let err = File::create(&stderr).map_err(|e| e.to_string())?;
    let mut child = Command::new(cli).args(args).stdin(Stdio::null()).stdout(out).stderr(err).spawn().map_err(|e| format!("start CLI: {e}"))?;
    let start = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if start.elapsed() < TIMEOUT => std::thread::sleep(Duration::from_millis(50)),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("CLI timed out after {}s", TIMEOUT.as_secs()));
            }
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("wait for CLI: {e}"));
            }
        }
    };
    let out = String::from_utf8(read_limited(&stdout)?).map_err(|e| e.to_string())?;
    let err = String::from_utf8(read_limited(&stderr)?).map_err(|e| e.to_string())?;
    if !status.success() {
        return Err(format!("CLI {tag} failed ({status}): {err}"));
    }
    Ok((out, err))
}

/// Request detailed CLI inspection and parse the returned document JSON.
fn inspect(cli: &Path, file: &Path, dir: &Path, tag: &str) -> Result<Value, String> {
    let path = file.to_str().ok_or("non-UTF-8 path")?;
    let (out, _) = invoke(cli, &["info".into(), path.into(), "--compact".into(), "--detail".into()], dir, tag)?;
    serde_json::from_str(&out).map_err(|e| format!("invalid CLI inspection: {e}"))
}

/// Reference-editor verdicts, which this runner never produces itself.
fn not_run() -> Value {
    json!({"opens": "not_run", "meaning": "not_run", "rendering": "not_run"})
}

/// Reopen the exported candidate and check it against the recipe.
fn check_candidate(
    case: &Recipe,
    cli: &Path,
    dir: &Path,
    candidate_sha256: &str,
    before: &Value,
    layer: &Value,
    export_warnings: &str,
) -> Result<Value, String> {
    let candidate = dir.join("candidate.psd");
    let after = inspect(cli, &candidate, dir, "after")?;
    let checks = verify(&case.checks, layer, target(&after, &case.target)?);
    let passed = checks.iter().all(|v| v.get("status").and_then(Value::as_str) == Some("pass"));
    Ok(json!({"id": case.id, "status": if passed { "pass" } else { "fail" }, "base": case.base, "base_sha256": case.base_sha256,
        "candidate": format!("{}/candidate.psd", case.id), "candidate_sha256": candidate_sha256, "checks": checks,
        "warnings": {"input": before.get("warnings"), "export": export_warnings, "output": after.get("warnings")},
        "photoshop": not_run()}))
}

/// Export and check one case, retaining candidate provenance after export succeeds.
fn execute(case: &Recipe, corpus: &Path, cli: &Path, output: &Path) -> Result<Value, String> {
    use std::io::Write;
    let input = input_path(corpus, &case.base)?;
    let input_bytes = read_limited(&input)?;
    if crate::sha256::hex(&input_bytes) != case.base_sha256 {
        return Err("base SHA-256 mismatch".into());
    }
    let dir = output.join(&case.id);
    fs::create_dir(&dir).map_err(|e| e.to_string())?;
    // Run on a private copy of the hashed bytes so the CLI never opens the corpus.
    let pinned = dir.join("base.psd");
    let mut f = OpenOptions::new().write(true).create_new(true).open(&pinned).map_err(|e| e.to_string())?;
    f.write_all(&input_bytes).map_err(|e| e.to_string())?;
    drop(f);
    let before = inspect(cli, &pinned, &dir, "before")?;
    let layer = target(&before, &case.target)?;
    let layer_id = layer.get("id").and_then(Value::as_u64).ok_or("target lacks numeric id")?;
    let candidate = dir.join("candidate.psd");
    let mut args = vec!["run".to_string(), pinned.to_str().ok_or("non-UTF-8 input path")?.to_string()];
    for step in &case.commands {
        let mut params = step.params.clone();
        params.as_object_mut().ok_or("params must be an object")?.insert("layer".into(), json!(layer_id));
        args.extend(["--cmd".into(), step.command.clone(), "--params".into(), params.to_string()]);
    }
    args.extend(["--out".into(), candidate.to_str().ok_or("non-UTF-8 output path")?.to_string()]);
    let (_, warnings) = invoke(cli, &args, &dir, "export")?;
    let bytes = read_limited(&candidate)?;
    let candidate_sha256 = crate::sha256::hex(&bytes);
    match check_candidate(case, cli, &dir, &candidate_sha256, &before, layer, &warnings) {
        Ok(record) => Ok(record),
        Err(e) => Ok(json!({"id": case.id, "status": "fail", "base": case.base, "base_sha256": case.base_sha256,
            "candidate": format!("{}/candidate.psd", case.id), "candidate_sha256": candidate_sha256,
            "error": format!("exported file: {e}"),
            "warnings": {"input": before.get("warnings"), "export": warnings},
            "photoshop": not_run()})),
    }
}

/// Create a report artifact without overwriting an existing file.
fn write_json(path: &Path, value: &Value) -> Result<(), String> {
    use std::io::Write;
    let mut f = OpenOptions::new().write(true).create_new(true).open(path).map_err(|e| e.to_string())?;
    serde_json::to_writer_pretty(&mut f, value).map_err(|e| e.to_string())?;
    writeln!(f).map_err(|e| e.to_string())
}

/// Trimmed stdout of `git args` in `root`; `None` outside a checkout or without git.
fn git(root: &Path, args: &[&str]) -> Option<String> {
    let out = Command::new("git").current_dir(root).args(args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8(out.stdout).ok().map(|s| s.trim().to_string())
}

/// Execute a batch of recipes against an oracle corpus and write a verification report.
pub fn run(root: &Path, args: &[&str]) -> Result<(), String> {
    let [recipe_path, corpus, cli, output] = args else {
        return Err("usage: cargo xtask psd-write-oracle <recipes.json> <corpus-dir> <photocraft-cli> <new-output-dir>".into());
    };
    let recipe_bytes = read_limited(Path::new(recipe_path))?;
    let recipes: Recipes = serde_json::from_slice(&recipe_bytes).map_err(|e| format!("invalid recipes: {e}"))?;
    validate(&recipes)?;
    let corpus = Path::new(corpus).canonicalize().map_err(|e| format!("corpus: {e}"))?;
    let cli = Path::new(cli).canonicalize().map_err(|e| format!("CLI: {e}"))?;
    let cli_sha = crate::sha256::hex(&read_limited_to(&cli, 512 * 1024 * 1024)?);
    let output = Path::new(output);
    fs::create_dir(output).map_err(|e| format!("output directory must be new: {e}"))?;
    let revision = git(root, &["rev-parse", "HEAD"]).unwrap_or_else(|| "unknown".into());
    let dirty = match git(root, &["status", "--porcelain", "--untracked-files=no"]) {
        Some(s) => json!(!s.is_empty()),
        None => json!("unknown"),
    };
    let mut cases = Vec::new();
    for case in &recipes.cases {
        let result = execute(case, &corpus, &cli, output).unwrap_or_else(|e| {
            json!({"id": case.id, "status": "fail", "error": e,
            "photoshop": not_run()})
        });
        cases.push(result);
    }
    let passed = cases.iter().all(|c| c.get("status").and_then(Value::as_str) == Some("pass"));
    let report = json!({"version": 1, "source_revision": revision, "source_dirty": dirty, "corpus_revision": recipes.corpus_revision,
        "recipe_sha256": crate::sha256::hex(&recipe_bytes), "cli_sha256": cli_sha, "cases": cases});
    write_json(&output.join("report.json"), &report)?;
    write_json(
        &output.join("manifest.json"),
        &json!({"version": 1, "source_revision": revision, "source_dirty": dirty, "corpus_revision": recipes.corpus_revision,
        "recipe_sha256": crate::sha256::hex(&recipe_bytes), "cli_sha256": cli_sha,
        "candidates": report["cases"].as_array().into_iter().flatten().filter_map(|c| Some(json!({"id": c.get("id")?, "path": c.get("candidate")?, "sha256": c.get("candidate_sha256")?}))).collect::<Vec<_>>() }),
    )?;
    if passed { Ok(()) } else { Err(format!("one or more recipes failed; see {}/report.json", output.display())) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn git_is_optional() {
        assert!(git(&std::env::temp_dir().join("psd-write-oracle-no-such-dir"), &["rev-parse", "HEAD"]).is_none());
    }

    #[test]
    fn detects_wrong_or_unchanged_edits_and_missing_fields() {
        let before = json!({"text":{"tracking":0,"value":"hello"}});
        let after = json!({"text":{"tracking":30,"value":"hello"}});
        let checks = [Check::ChangedTo { path: "/text/tracking".into(), value: json!(30) }, Check::Unchanged { path: "/text/value".into() }];
        assert!(verify(&checks, &before, &after).iter().all(|c| c["status"] == "pass"));
        assert_eq!(verify(&checks, &before, &before)[0]["status"], "fail");
        let missing = [Check::Unchanged { path: "/missing".into() }];
        assert_eq!(verify(&missing, &before, &after)[0]["status"], "fail");
    }

    #[test]
    fn every_failed_check_is_reported_with_layer_name_and_property() {
        let before = json!({"name":"Title","text":{"tracking":0,"value":"a"}});
        let after = json!({"name":"Title","text":{"tracking":0,"value":"b"}});
        let checks = [
            Check::ChangedTo { path: "/text/tracking".into(), value: json!(30) },
            Check::Unchanged { path: "/text/value".into() },
            Check::Equals { path: "/text/value".into(), value: json!("c") },
        ];
        let results = verify(&checks, &before, &after);
        assert_eq!(results.len(), 3);
        assert!(results.iter().all(|r| r["status"] == "fail" && r["layer"] == "Title"));
        assert_eq!(results[0]["path"], "/text/tracking");
        assert_eq!(results[1]["path"], "/text/value");
    }

    #[test]
    fn wrong_layer_count_and_unsupported_version_are_errors() {
        let selector = Target { kind: "Text".into(), name: None };
        let none = json!({"layers":[{"kind":"Raster"}]});
        assert!(target(&none, &selector).unwrap_err().contains("matched 0 layers"));
        let two = json!({"layers":[{"kind":"Text"},{"kind":"Text"}]});
        assert!(target(&two, &selector).unwrap_err().contains("matched 2 layers"));
        let recipe = json!({"version":2,"corpus_revision":"pin","cases":[{"id":"x","base":"x.psd","base_sha256":"a".repeat(64),"target":{"kind":"Type"},"checks":[{"op":"unchanged","path":"/name"}]}]});
        assert!(validate(&serde_json::from_value(recipe).unwrap()).unwrap_err().contains("version 1"));
    }

    #[test]
    fn unknown_keys_are_rejected_in_every_object() {
        let case = |extra: Value| {
            let mut c = json!({"id":"x","base":"x.psd","base_sha256":"a".repeat(64),"target":{"kind":"Type"},"checks":[{"op":"unchanged","path":"/name"}]});
            c.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
            json!({"version":1,"corpus_revision":"pin","cases":[c]})
        };
        assert!(serde_json::from_value::<Recipes>(case(json!({"target":{"kind":"Type","nmae":"x"}}))).is_err());
        assert!(serde_json::from_value::<Recipes>(case(json!({"checks":[{"op":"unchanged","path":"/name","extra":1}]}))).is_err());
        assert!(serde_json::from_value::<Recipes>(case(json!({"commands":[{"command":"type.x","parms":{}}]}))).is_err());
        let mut top = case(json!({}));
        top["extra"] = json!(1);
        assert!(serde_json::from_value::<Recipes>(top).is_err());
    }

    #[test]
    fn layer_selection_rejects_ambiguity_and_finds_nested_layers() {
        let doc = json!({"layers":[{"kind":"Group","children":[{"kind":"Text","id":7,"name":"A"}]}]});
        let selector = Target { kind: "Text".into(), name: None };
        assert_eq!(target(&doc, &selector).unwrap()["id"], 7);
        let ambiguous = json!({"layers":[{"kind":"Text"},{"kind":"Text"}]});
        assert!(target(&ambiguous, &selector).is_err());
    }

    #[test]
    fn schema_rejects_unsafe_paths_duplicate_ids_and_unverified_edits() {
        let recipe = json!({"version":1,"corpus_revision":"pin","cases":[{"id":"case","base":"../x.psd","base_sha256":"a".repeat(64),"target":{"kind":"Text"},"checks":[{"op":"unchanged","path":"/name"}]}]});
        let parsed: Recipes = serde_json::from_value(recipe).unwrap();
        assert!(validate(&parsed).is_err());
        let duplicate = json!({"version":1,"corpus_revision":"pin","cases":[{"id":"x","base":"x.psd","base_sha256":"a".repeat(64),"target":{"kind":"Text"},"checks":[{"op":"unchanged","path":"/name"}]},{"id":"x","base":"x.psd","base_sha256":"a".repeat(64),"target":{"kind":"Text"},"checks":[{"op":"unchanged","path":"/name"}]}]});
        assert!(validate(&serde_json::from_value(duplicate).unwrap()).is_err());
        let case_only = json!({"version":1,"corpus_revision":"pin","cases":[{"id":"A","base":"x.psd","base_sha256":"a".repeat(64),"target":{"kind":"Text"},"checks":[{"op":"unchanged","path":"/name"}]},{"id":"a","base":"x.psd","base_sha256":"a".repeat(64),"target":{"kind":"Text"},"checks":[{"op":"unchanged","path":"/name"}]}]});
        assert!(validate(&serde_json::from_value(case_only).unwrap()).unwrap_err().contains("duplicate"));
        let contour = |command: &str| {
            json!({"version":1,"corpus_revision":"pin","cases":[{"id":"x","base":"x.psd","base_sha256":"a".repeat(64),"target":{"kind":"Text"},
                "commands":[{"command":command,"params":{"path":"base.psd"}}],"checks":[{"op":"changed_to","path":"/name","value":"y"}]}]})
        };
        for command in ["layer.layerStyle.contours.save", "layer.layerStyle.contours.load"] {
            assert!(validate(&serde_json::from_value(contour(command)).unwrap()).unwrap_err().contains("invalid command"));
        }
        assert!(validate(&serde_json::from_value(contour("layer.layerStyle.dropShadow")).unwrap()).is_ok());
        let edit = json!({"version":1,"corpus_revision":"pin","cases":[{"id":"x","base":"x.psd","base_sha256":"a".repeat(64),"target":{"kind":"Text"},"commands":[{"command":"type.setStyle"}],"checks":[{"op":"unchanged","path":"/name"}]}]});
        assert!(validate(&serde_json::from_value(edit).unwrap()).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn export_batch_checks_reopened_result_and_propagates_command_failure() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::Builder::new().prefix("psd-write-oracle-").tempdir().unwrap();
        let root = tmp.path().to_path_buf();
        let corpus = root.join("corpus");
        let output = root.join("output");
        fs::create_dir(&corpus).unwrap();
        let corpus = corpus.canonicalize().unwrap();
        fs::create_dir(&output).unwrap();
        fs::write(corpus.join("base.psd"), b"base").unwrap();
        let cli = root.join("fake-cli");
        fs::write(
            &cli,
            r##"#!/bin/sh
if [ "$1" = info ]; then
    case "$2" in
      */candidate.psd) echo '{"layers":[{"kind":"Type","id":4,"text":{"antialias":"Crisp"}}]}' ;;
      *) echo '{"layers":[{"kind":"Type","id":4,"text":{"antialias":"None"}}]}' ;;
    esac
    exit 0
fi
if [ "$3" = --cmd ] && [ "$4" = type.bad ]; then
    echo 'bad command' >&2
    exit 1
fi
input="$2"
while [ "$#" -gt 0 ]; do
    if [ "$1" = --out ]; then cp "$input" "$2"; exit 0; fi
    shift
done
exit 1
"##,
        )
        .unwrap();
        fs::set_permissions(&cli, fs::Permissions::from_mode(0o755)).unwrap();
        let make_case = |id: &str, command: &str| Recipe {
            id: id.into(),
            base: "base.psd".into(),
            base_sha256: crate::sha256::hex(b"base"),
            target: Target { kind: "Type".into(), name: None },
            commands: vec![Step { command: command.into(), params: json!({}) }],
            checks: vec![Check::ChangedTo { path: "/text/antialias".into(), value: json!("Crisp") }],
        };
        let good = execute(&make_case("good", "type.antiAlias.crisp"), &corpus, &cli, &output).unwrap();
        assert_eq!(good["status"], "pass");
        assert_eq!(good["photoshop"]["opens"], "not_run");
        assert!(execute(&make_case("bad", "type.bad"), &corpus, &cli, &output).unwrap_err().contains("bad command"));
    }

    /// Fake CLI that logs its arguments, tampers with the file it is asked to run on, and reports
    /// `candidate_info` for the exported file.
    #[cfg(unix)]
    fn fake_cli(root: &Path, log: &Path, candidate_info: &str, export_fails: bool) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let cli = root.join("fake-cli");
        let script = r##"#!/bin/sh
echo "$@" >> 'LOG'
if [ "$1" = info ]; then
    case "$2" in
      */candidate.psd) echo 'CANDIDATE' ;;
      *) echo '{"layers":[{"kind":"Type","id":4,"text":{"antialias":"None"}}]}' ;;
    esac
    exit 0
fi
if [ EXPORT_FAILS = 1 ]; then
    echo 'export broke' >&2
    exit 1
fi
input="$2"
cp "$input" "$input.received"
echo tamper >> "$input"
while [ "$#" -gt 0 ]; do
    if [ "$1" = --out ]; then cp "$input" "$2"; exit 0; fi
    shift
done
exit 1
"##
        .replace("LOG", &log.display().to_string())
        .replace("CANDIDATE", candidate_info)
        .replace("EXPORT_FAILS", if export_fails { "1" } else { "0" });
        fs::write(&cli, script).unwrap();
        fs::set_permissions(&cli, fs::Permissions::from_mode(0o755)).unwrap();
        cli
    }

    /// Run one recipe case through `run`; returns (temp dir guard, result, output dir, corpus dir, log path).
    #[cfg(unix)]
    fn run_case(id: &str, candidate_info: &str, export_fails: bool) -> (tempfile::TempDir, Result<(), String>, PathBuf, PathBuf, PathBuf) {
        let tmp = tempfile::Builder::new().prefix("psd-write-oracle-").tempdir().unwrap();
        let root = tmp.path().canonicalize().unwrap();
        let corpus = root.join("corpus");
        fs::create_dir(&corpus).unwrap();
        fs::write(corpus.join("base.psd"), b"base").unwrap();
        let log = root.join("calls.log");
        let cli = fake_cli(&root, &log, candidate_info, export_fails);
        let recipes = json!({"version":1,"corpus_revision":"pin","cases":[{"id":id,"base":"base.psd","base_sha256":crate::sha256::hex(b"base"),
            "target":{"kind":"Type"},"commands":[{"command":"type.antiAlias.crisp","params":{}}],
            "checks":[{"op":"changed_to","path":"/text/antialias","value":"Crisp"}]}]});
        let recipe_path = root.join("recipes.json");
        fs::write(&recipe_path, recipes.to_string()).unwrap();
        let out = root.join("out");
        let args = [recipe_path.to_str().unwrap(), corpus.to_str().unwrap(), cli.to_str().unwrap(), out.to_str().unwrap()];
        let result = run(&root, &args);
        (tmp, result, out, corpus, log)
    }

    #[cfg(unix)]
    #[test]
    fn target_lost_after_export_keeps_candidate_in_manifest() {
        let (_tmp, result, out, _, _) = run_case("lost", r#"{"layers":[]}"#, false);
        assert!(result.is_err());
        let report: Value = serde_json::from_slice(&fs::read(out.join("report.json")).unwrap()).unwrap();
        assert_eq!(report["cases"][0]["status"], "fail");
        assert!(report["cases"][0]["error"].as_str().unwrap().starts_with("exported file: "));
        let manifest: Value = serde_json::from_slice(&fs::read(out.join("manifest.json")).unwrap()).unwrap();
        let entry = &manifest["candidates"][0];
        assert_eq!(entry["id"], "lost");
        assert_eq!(entry["path"], "lost/candidate.psd");
        assert_eq!(entry["sha256"], crate::sha256::hex(&fs::read(out.join("lost/candidate.psd")).unwrap()));
    }

    #[cfg(unix)]
    #[test]
    fn export_failure_has_no_manifest_entry() {
        let (_tmp, result, out, _, _) = run_case("broken", r#"{"layers":[]}"#, true);
        assert!(result.is_err());
        let report: Value = serde_json::from_slice(&fs::read(out.join("report.json")).unwrap()).unwrap();
        assert_eq!(report["cases"][0]["status"], "fail");
        assert!(report["cases"][0]["error"].as_str().unwrap().contains("export broke"));
        let manifest: Value = serde_json::from_slice(&fs::read(out.join("manifest.json")).unwrap()).unwrap();
        assert!(manifest["candidates"].as_array().unwrap().is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn cli_runs_on_pinned_base_never_on_corpus() {
        let (_tmp, result, out, corpus, log) = run_case("pin", r#"{"layers":[{"kind":"Type","id":4,"text":{"antialias":"Crisp"}}]}"#, false);
        assert!(result.is_ok(), "{result:?}");
        assert_eq!(fs::read(corpus.join("base.psd")).unwrap(), b"base");
        let pinned = out.join("pin/base.psd");
        assert!(pinned.exists());
        assert_eq!(fs::read(out.join("pin/base.psd.received")).unwrap(), b"base");
        let log = fs::read_to_string(log).unwrap();
        let corpus_path = corpus.join("base.psd").display().to_string();
        assert!(!log.contains(&corpus_path));
        let pinned = pinned.display().to_string();
        let base_calls: Vec<&str> = log.lines().filter(|l| !l.starts_with("info ") || !l.contains("candidate.psd")).collect();
        assert_eq!(base_calls.len(), 2);
        assert!(base_calls.iter().all(|l| l.contains(&pinned)));
    }

    #[test]
    fn numbers_compare_by_value() {
        assert!(values_equal(&json!(17), &json!(17.0)));
        assert!(!values_equal(&json!(17), &json!(18.0)));
        assert!(!values_equal(&json!(17), &json!(17.5)));
        assert!(values_equal(&json!(0.5), &json!(0.5)));
        assert!(values_equal(&json!(-3), &json!(-3.0)));
        assert!(values_equal(&json!(u64::MAX), &json!(u64::MAX)));
        assert!(!values_equal(&json!(9_007_199_254_740_993_u64), &json!(9_007_199_254_740_992_u64)));
        assert!(!values_equal(&json!(9_007_199_254_740_993_u64), &json!(9_007_199_254_740_992.0)));
        assert!(values_equal(&json!(9_007_199_254_740_992_u64), &json!(9_007_199_254_740_992.0)));
        assert!(!values_equal(&json!(u64::MAX), &json!(1e300)));
        assert!(values_equal(&json!({"a":[1,2.0,{"b":3}],"c":"x"}), &json!({"c":"x","a":[1.0,2,{"b":3.0}]})));
        assert!(!values_equal(&json!({"a":1}), &json!({"b":1})));
        assert!(!values_equal(&json!({"a":1}), &json!({"a":1,"b":1})));
        assert!(!values_equal(&json!([1, 2]), &json!([1])));
        let before = json!({"n":17});
        let after = json!({"n":17.0});
        assert_eq!(verify(&[Check::ChangedTo { path: "/n".into(), value: json!(17) }], &before, &after)[0]["status"], "fail");
        assert_eq!(verify(&[Check::Equals { path: "/n".into(), value: json!(17) }], &before, &after)[0]["status"], "pass");
        assert_eq!(verify(&[Check::Unchanged { path: "/n".into() }], &before, &after)[0]["status"], "pass");
        assert_eq!(verify(&[Check::ChangedTo { path: "/n".into(), value: json!(17) }], &json!({}), &after)[0]["status"], "pass");
        assert_eq!(verify(&[Check::Equals { path: "/m".into(), value: json!(1) }], &before, &after)[0]["status"], "fail");
    }
}
