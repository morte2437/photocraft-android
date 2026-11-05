//! Corpus test (feature `corpus`; run with `cargo xtask test-corpus`): every `.atn` set under
//! `corpus/atn/` loads through `actions.load`, exports through `actions.save`, and the export
//! loads back to the same actions (allowing the specified compound-property event expansion). Steps without a PhotoCraft equivalent are counted against a
//! ceiling, so a mapping regression (or a new unmapped event in the corpus) shows up here.
#![cfg(feature = "corpus")]

use std::path::{Path, PathBuf};

use photocraft_engine::Session;
use serde_json::json;

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

/// A compound layer-property command deliberately becomes one Photoshop event per property
/// (#8304). Compare that documented sequence exactly, including its target and every flag;
/// all other commands and action metadata retain their original equality check.
fn exported_action(action: &photocraft_engine::actions_cmds::Action) -> photocraft_engine::actions_cmds::Action {
    let mut out = action.clone();
    out.steps.clear();
    out.disabled.clear();
    out.dialogs.clear();
    for (index, (id, params)) in action.steps.iter().enumerate() {
        let mut steps = vec![(id.clone(), params.clone())];
        if id == "layer.setProps"
            && let Some(object) = params.as_object()
        {
            let fields = ["opacity", "fill", "blend", "visible", "name"];
            let properties: Vec<_> = fields.into_iter().filter(|key| object.contains_key(*key)).collect();
            if properties.len() > 1 && object.keys().all(|key| key == "layer" || fields.contains(&key.as_str())) {
                steps = properties
                    .into_iter()
                    .map(|key| {
                        let mut property = json!({key: object.get(key).unwrap()});
                        if let Some(target) = object.get("layer") {
                            property["layer"] = target.clone();
                        }
                        (id.clone(), property)
                    })
                    .collect();
            }
        }
        for step in steps {
            let next = out.steps.len();
            if action.disabled.contains(&index) {
                out.disabled.insert(next);
            }
            if action.dialogs.contains(&index) {
                out.dialogs.insert(next);
            }
            out.steps.push(step);
        }
    }
    out
}

#[test]
fn compound_roundtrip_expectation_keeps_properties_targets_flags_and_action_metadata() {
    use photocraft_engine::actions_cmds::Action;
    let mut source = Action::new(
        "Properties",
        vec![("layer.setProps".into(), json!({"layer": "Before", "name": "After", "opacity": 0.5, "blend": "Normal"})), ("select.inverse".into(), json!({}))],
    );
    source.set = Some("Set".into());
    source.color = "blue".into();
    source.disabled.insert(0);
    source.dialogs.insert(0);
    source.dialogs.insert(1);
    let mut expected = source.clone();
    expected.steps = vec![
        ("layer.setProps".into(), json!({"layer": "Before", "opacity": 0.5})),
        ("layer.setProps".into(), json!({"layer": "Before", "blend": "Normal"})),
        ("layer.setProps".into(), json!({"layer": "Before", "name": "After"})),
        ("select.inverse".into(), json!({})),
    ];
    expected.disabled = [0, 1, 2].into_iter().collect();
    expected.dialogs = [0, 1, 2, 3].into_iter().collect();
    assert_eq!(exported_action(&source), expected);
    assert_eq!(exported_action(&expected), expected, "single-property steps stay unchanged");
    let mut lost_property = expected.clone();
    lost_property.steps.remove(1);
    assert_ne!(exported_action(&source), lost_property, "losing blend must still fail comparison");
    let mut lost_flag = expected.clone();
    lost_flag.disabled.remove(&1);
    assert_ne!(exported_action(&source), lost_flag, "losing an expanded flag must still fail comparison");
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
        let first: Vec<_> = s.actions.list.iter().map(exported_action).collect();
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
            Ok(_) if t.actions.list == first => {}
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
