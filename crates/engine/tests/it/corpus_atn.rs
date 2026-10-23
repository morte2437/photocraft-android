//! Corpus test (feature `corpus`; run with `cargo xtask test-corpus`): every `.atn` set under
//! `corpus/atn/` loads through `actions.load`, exports through `actions.save`, and the export
//! loads back to the same actions. Steps without a PhotoCraft equivalent are counted against a
//! ceiling, so a mapping regression (or a new unmapped event in the corpus) shows up here.
#![cfg(feature = "corpus")]

use std::path::{Path, PathBuf};

use photocraft_engine::Session;
use photocraft_engine::actions_cmds::Action;
use serde_json::{Value, json};

fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    // An unreadable folder fails the test rather than shrinking the corpus.
    let rd = std::fs::read_dir(dir).unwrap_or_else(|e| panic!("{}: {e}", dir.display()));
    for e in rd {
        let p = e.unwrap_or_else(|e| panic!("{}: {e}", dir.display())).path();
        if p.is_dir() {
            collect(&p, out);
        } else if p.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("atn")) {
            out.push(p);
        }
    }
}

/// Sets in the corpus, and the most steps that may still be kept Photoshop steps across all of
/// them (2026-10-12: 517 of 5 296; 404 are generative AI (`generativeUpscale`, `syntheticFill`,
/// `syntheticGenHarmonize`: deferred by decision) and `AdobeScriptAutomation` scripts, the rest
/// shape layers with strokes, snapshots, smart-filter and channel edits, frame animation).
/// Raise the first, lower the second, never the reverse.
const FLOOR_FILES: usize = 16;
const CEILING_UNSUPPORTED: usize = 540;

/// Save writes a compound `layer.setProps` (name, opacity, blend) as one Set per property, because
/// Photoshop applies only the rename of a combined one (#8304); loading gives one step per
/// property. Merge consecutive `layer.setProps` steps of one target so both sides compare as the
/// same actions.
fn merge_set_props(actions: &[Action]) -> Vec<Action> {
    actions
        .iter()
        .map(|a| {
            let mut out = a.clone();
            out.steps.clear();
            out.disabled.clear();
            out.dialogs.clear();
            for (i, (id, params)) in a.steps.iter().enumerate() {
                let merged = match (out.steps.last_mut(), params.as_object()) {
                    (Some((last_id, Value::Object(last))), Some(p))
                        if id == "layer.setProps"
                            && last_id == id
                            && last.get("layer") == p.get("layer")
                            && p.keys().all(|k| k == "layer" || !last.contains_key(k)) =>
                    {
                        last.extend(p.iter().map(|(k, v)| (k.clone(), v.clone())));
                        true
                    }
                    _ => false,
                };
                if !merged {
                    out.steps.push((id.clone(), params.clone()));
                }
                let at = out.steps.len() - 1;
                if a.disabled.contains(&i) {
                    out.disabled.insert(at);
                }
                if a.dialogs.contains(&i) {
                    out.dialogs.insert(at);
                }
            }
            out
        })
        .collect()
}

#[test]
fn atn_corpus_loads_saves_and_reloads() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus/atn");
    assert!(root.is_dir(), "{} is missing: run `cargo xtask corpus --atn`", root.display());
    let mut files = Vec::new();
    collect(&root, &mut files);
    files.sort();
    let mut failures = Vec::new();
    let (mut steps, mut unsupported) = (0usize, 0usize);
    for p in &files {
        let name = p.strip_prefix(&root).unwrap_or(p).display().to_string();
        let data = photocraft_engine::paint::tile::b64_encode(&std::fs::read(p).unwrap());
        let mut s = Session::new();
        let loaded = match s.execute("actions.load", json!({"data": data})) {
            Ok(v) => v,
            Err(e) => {
                failures.push(format!("{name}: load: {e}"));
                continue;
            }
        };
        let n_steps = loaded["steps"].as_u64().unwrap_or(0) as usize;
        let n_unsupported = loaded["unsupported"].as_array().map_or(0, Vec::len);
        let events: Vec<String> =
            loaded["unsupported"].as_array().map_or(Vec::new(), |u| u.iter().filter_map(|x| x["event"].as_str().map(String::from)).collect());
        steps += n_steps;
        unsupported += n_unsupported;
        println!("{name}: {} actions, {n_steps} steps, {n_unsupported} kept Photoshop steps {events:?}", loaded["actions"].as_array().map_or(0, Vec::len));
        let first = s.actions.list.clone();
        let saved = match s.execute("actions.save", json!({"set": loaded["set"]})) {
            Ok(v) => v,
            Err(e) => {
                failures.push(format!("{name}: save: {e}"));
                continue;
            }
        };
        let bytes = photocraft_engine::paint::tile::b64_decode(saved["data"].as_str().unwrap_or("")).unwrap_or_default();
        let mut t = Session::new();
        match t.execute("actions.load", json!({"data": photocraft_engine::paint::tile::b64_encode(&bytes)})) {
            Ok(_) if merge_set_props(&t.actions.list) == merge_set_props(&first) => {}
            Ok(_) => {
                // Name the first step that differs, so a mapping asymmetry is quick to find.
                let detail = first
                    .iter()
                    .zip(&t.actions.list)
                    .flat_map(|(a, b)| a.steps.iter().zip(&b.steps).enumerate().map(move |(i, (x, y))| (a.name.as_str(), i, x, y)))
                    .find(|(_, _, x, y)| x != y)
                    .map(|(action, i, x, y)| format!("`{action}` step {}: {} {} → {} {}", i + 1, x.0, x.1, y.0, y.1))
                    .unwrap_or_else(|| "the actions differ in count, flags or names".into());
                failures.push(format!("{name}: the saved set loads to different actions: {detail}"));
            }
            Err(e) => failures.push(format!("{name}: reload: {e}")),
        }
    }
    println!("{} files, {steps} steps, {unsupported} kept Photoshop steps", files.len());
    assert!(files.len() >= FLOOR_FILES, "only {} .atn files; the corpus floor is {FLOOR_FILES}", files.len());
    assert!(unsupported <= CEILING_UNSUPPORTED, "{unsupported} steps have no PhotoCraft equivalent; the ceiling is {CEILING_UNSUPPORTED}");
    assert!(failures.is_empty(), "{} failures:\n{}", failures.len(), failures.join("\n"));
}
