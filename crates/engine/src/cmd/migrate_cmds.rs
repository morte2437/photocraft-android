//! Edit › Presets › Migrate Presets. Photoshop imports presets from a previous version; we merge a
//! presets file (another install's preferences, or an exported preset library) into the current
//! library, appending groups the library doesn't already have (by name). Headless and scriptable.

use serde_json::{Value, json};

use crate::presets::{Group, PresetState};
use crate::{EngineError, Result, Session};

/// Append incoming groups whose name the library doesn't already have; returns how many were added.
fn merge_groups<T>(cur: &mut Vec<Group<T>>, incoming: Vec<Group<T>>) -> usize {
    let mut added = 0;
    for g in incoming {
        if !cur.iter().any(|e| e.name == g.name) {
            cur.push(g);
            added += 1;
        }
    }
    added
}

/// Deserialize `v[key]` into `Vec<Group<T>>`, defaulting to empty.
fn groups<T: serde::de::DeserializeOwned>(v: &Value, key: &str) -> Vec<Group<T>> {
    v.get(key).cloned().and_then(|g| serde_json::from_value(g).ok()).unwrap_or_default()
}

fn migrate(s: &mut Session, p: &Value) -> Result<Value> {
    let path = p
        .get("path")
        .and_then(Value::as_str)
        .ok_or_else(|| EngineError::BadParams { cmd: "edit.presets.migratePresets".into(), msg: "need `path` to a presets or preferences file".into() })?;
    let text = std::fs::read_to_string(path).map_err(|e| EngineError::Other(format!("read `{path}`: {e}")))?;
    let root: Value = serde_json::from_str(&text).map_err(|e| EngineError::Other(format!("`{path}` is not JSON: {e}")))?;
    // Accept a full preferences file (has a `presets` section) or a bare presets object.
    let v = root.get("presets").cloned().unwrap_or(root);

    let ps: &mut PresetState = &mut s.presets;
    let mut counts = serde_json::Map::new();
    let g = merge_groups(&mut ps.gradients, groups(&v, "gradients"));
    let st = merge_groups(&mut ps.styles, groups(&v, "styles"));
    let sh = merge_groups(&mut ps.shapes, groups(&v, "shapes"));
    let pt = merge_groups(&mut ps.pattern_groups, groups(&v, "pattern_groups"));
    // Tool presets are a flat list, matched by name. Read them like the launch loader does (entry by
    // entry, checked names), and keep the library within its cap, so what we import survives a restart.
    let mut tp = 0usize;
    if let Some(arr) = v.get("tool_presets").cloned() {
        for t in crate::presets::tools::load(arr) {
            if ps.tool_presets.len() >= crate::presets::tools::MAX_PRESETS {
                break;
            }
            if !ps.tool_presets.iter().any(|e| e.name == t.name) {
                ps.tool_presets.push(t);
                tp += 1;
            }
        }
    }
    let total = g + st + sh + pt + tp;
    if total > 0 {
        s.presets_changed();
    }
    counts.insert("gradients".into(), json!(g));
    counts.insert("styles".into(), json!(st));
    counts.insert("shapes".into(), json!(sh));
    counts.insert("patternGroups".into(), json!(pt));
    counts.insert("toolPresets".into(), json!(tp));
    Ok(json!({"migrated": total, "added": counts}))
}

pub fn specs() -> Vec<crate::commands::CommandSpec> {
    vec![crate::commands::CommandSpec {
        id: "edit.presets.migratePresets",
        label: "Migrate Presets",
        menu: &["Edit", "Presets"],
        shortcut: None,
        params: r#"{path} → {migrated, added:{gradients,styles,shapes,patternGroups,toolPresets}}: merge a presets/preferences file into the library (appends groups by name)"#,
        enabled: |_s| Ok(()),
        journal: false,
        run: |s, p| migrate(s, p),
    }]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merges_new_groups_only() {
        let mut s = Session::new();
        let before = s.presets.gradients.len();
        let existing = s.presets.gradients[0].name.clone();
        // A presets file with one new gradient group and one that already exists.
        let file = std::env::temp_dir().join(format!("pc-migrate-{}.json", std::process::id()));
        let data = json!({
            "presets": {
                "gradients": [
                    {"name": "My Imported Set", "items": []},
                    {"name": existing, "items": []},
                ]
            }
        });
        std::fs::write(&file, serde_json::to_string(&data).unwrap()).unwrap();
        let prefs_rev = s.prefs.rev();
        let presets_rev = s.presets.rev;
        let r = s.execute("edit.presets.migratePresets", json!({"path": file.to_string_lossy()})).unwrap();
        assert_eq!(r["added"]["gradients"], 1, "only the new group is added");
        assert_eq!(s.presets.gradients.len(), before + 1);
        assert!(s.presets.gradients.iter().any(|g| g.name == "My Imported Set"));
        assert_eq!(s.prefs.rev(), prefs_rev + 1, "migration marks preferences dirty");
        assert_eq!(s.presets.rev, presets_rev + 1);
        // Idempotent: migrating again adds nothing.
        let prefs_rev = s.prefs.rev();
        let presets_rev = s.presets.rev;
        let saved = s.prefs_to_json();
        let r2 = s.execute("edit.presets.migratePresets", json!({"path": file.to_string_lossy()})).unwrap();
        assert_eq!(r2["migrated"], 0);
        assert_eq!(s.prefs.rev(), prefs_rev, "duplicate groups do not dirty preferences");
        assert_eq!(s.presets.rev, presets_rev);
        assert_eq!(s.prefs_to_json(), saved);
        let _ = std::fs::remove_file(file);
    }

    #[test]
    fn each_migrated_preset_kind_marks_preferences_dirty_and_survives_reload() {
        let file = std::env::temp_dir().join(format!("pc-migrate-persist-{}.json", std::process::id()));
        for (key, incoming) in [
            ("gradients", json!([{"name": "Imported gradients", "items": []}])),
            ("styles", json!([{"name": "Imported styles", "items": []}])),
            ("shapes", json!([{"name": "Imported shapes", "items": []}])),
            ("pattern_groups", json!([{"name": "Imported patterns", "items": ["Bricks"]}])),
            ("tool_presets", json!([{"name": "Imported tool", "tool": "brush", "options": {"size": 17.0}}])),
        ] {
            for wrapped in [false, true] {
                let mut s = Session::new();
                let mut data = json!({});
                data[key] = incoming.clone();
                if wrapped {
                    data = json!({"presets": data});
                }
                std::fs::write(&file, serde_json::to_string(&data).unwrap()).unwrap();
                let prefs_rev = s.prefs.rev();
                let presets_rev = s.presets.rev;
                let r = s.execute("edit.presets.migratePresets", json!({"path": file.to_string_lossy()})).unwrap();
                assert_eq!(r["migrated"], 1, "{key}, wrapped={wrapped}");
                assert_eq!(s.prefs.rev(), prefs_rev + 1, "{key}, wrapped={wrapped}: preferences must be saved");
                assert_eq!(s.presets.rev, presets_rev + 1);
                let saved = s.prefs_to_json();
                let mut restored = Session::new();
                restored.load_prefs_json(&saved).unwrap();
                let imported = &incoming[0];
                assert!(restored.prefs_value()["presets"][key].as_array().unwrap().contains(imported), "{key}, wrapped={wrapped}");
            }
        }
        let _ = std::fs::remove_file(file);
    }

    #[test]
    fn tool_presets_are_read_entry_by_entry_like_the_launch_loader() {
        use crate::presets::tools::MAX_PRESETS;
        let file = std::env::temp_dir().join(format!("pc-migrate-tools-{}.json", std::process::id()));
        let migrate = |s: &mut Session, data: Value| {
            std::fs::write(&file, serde_json::to_string(&data).unwrap()).unwrap();
            s.execute("edit.presets.migratePresets", json!({"path": file.to_string_lossy()})).unwrap()
        };
        // A damaged entry no longer discards the good ones; blank and overlong names are skipped.
        let mut s = Session::new();
        let before = s.presets.tool_presets.len();
        let r = migrate(
            &mut s,
            json!({"tool_presets": [
                {"name": "4x6 print", "tool": "crop", "options": {}},
                {"name": "old", "tool": null},
                {"name": "  ", "tool": "crop"},
                {"name": "x".repeat(300), "tool": "crop"},
                {"name": "  Padded  ", "tool": "brush", "options": 5},
            ]}),
        );
        assert_eq!(r["added"]["toolPresets"], 2);
        assert_eq!(s.presets.tool_presets.len(), before + 2);
        assert!(s.presets.tool_presets.iter().any(|t| t.name == "4x6 print"));
        let padded = s.presets.tool_presets.iter().find(|t| t.name == "Padded").expect("trimmed name");
        assert_eq!(padded.options, json!({}));
        // What was imported survives the next launch.
        let mut restored = Session::new();
        restored.load_prefs_json(&s.prefs_to_json()).unwrap();
        assert_eq!(restored.presets.tool_presets, s.presets.tool_presets);
        // The library stays within MAX_PRESETS.
        let mut s = Session::new();
        let many: Vec<Value> = (0..MAX_PRESETS + 500).map(|i| json!({"name": format!("p{i}"), "tool": "brush"})).collect();
        let before = s.presets.tool_presets.len();
        let r = migrate(&mut s, json!({"tool_presets": many}));
        assert_eq!(s.presets.tool_presets.len(), MAX_PRESETS);
        assert_eq!(r["added"]["toolPresets"], MAX_PRESETS - before);
        let _ = std::fs::remove_file(file);
    }

    #[test]
    fn bad_path_is_an_error() {
        let mut s = Session::new();
        assert!(s.execute("edit.presets.migratePresets", json!({"path": "/no/such/file.json"})).is_err());
        assert!(s.execute("edit.presets.migratePresets", json!({})).is_err());
    }
}
