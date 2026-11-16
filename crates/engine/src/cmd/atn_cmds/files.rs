use super::convert::{ReplacementSet, UniqueNames, clip_name, export_with_keys, import_first_with_keys};
use super::{Action, DEFAULT_SET, EngineError, Result, Session, Value, bad, json};

mod replacement;
pub(super) use replacement::replace;
pub use replacement::validate as validate_replacement;

/// `actions.load`: add the actions of an `.atn` file (`path` or `data`) to the list, retargeting
/// the set's own action references once and preserving foreign qualifications even when the
/// caller changes the imported set's name or action names collide. A set an automation client
/// loads is marked as automation's, so it keeps the gate when the user plays it.
pub(super) fn load(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "actions.load";
    if s.actions.recording.is_some() || s.actions.playing > 0 {
        return Err(bad(cmd, "stop recording or playing before loading actions"));
    }
    let encoding = match p.get("encoding") {
        None | Some(Value::Null) => None,
        Some(Value::String(e)) if e == "auto" => None,
        Some(Value::String(e)) if !e.is_empty() => Some(e.as_str()),
        Some(_) => return Err(bad(cmd, "encoding must be \"auto\" or a label such as shift_jis, gbk, big5, euc-kr, windows-1252, macintosh")),
    };
    let (bytes, _) = crate::preset_import_cmds::file_bytes(p, cmd, ".atn file")?;
    let ReplacementSet { imported: mut imp, keys: function_keys } = import_first_with_keys(&bytes, encoding).map_err(|e| bad(cmd, e))?;
    let original_set = imp.set.clone();
    match p.get("set") {
        None | Some(Value::Null) => {}
        Some(Value::String(n)) if !n.trim().is_empty() => {
            // Held to the importer's name limit: the set name is copied into every action.
            imp.set = clip_name(n.trim()).trim_end().to_string();
            for a in &mut imp.actions {
                a.set = Some(imp.set.clone());
            }
        }
        Some(_) => return Err(bad(cmd, "`set` must be a non-empty string")),
    }
    let replace = match p.get("replace") {
        None | Some(Value::Null) => true,
        Some(Value::Bool(b)) => *b,
        Some(_) => return Err(bad(cmd, "`replace` must be true or false")),
    };
    if replace {
        crate::actions_cmds::remove_shortcuts(s, |_, action| action.set.as_deref() == Some(imp.set.as_str()));
        s.actions.list.retain(|a| a.set.as_deref() != Some(imp.set.as_str()));
        let mut slots = Vec::with_capacity(s.actions.set_rows.len());
        let mut next = 0;
        s.actions.set_rows.retain(|name| {
            let keep = name != &imp.set;
            slots.push(keep.then_some(next));
            next += usize::from(keep);
            keep
        });
        if next != slots.len() {
            s.actions.set_rows_revision = s.actions.set_rows_revision.wrapping_add(1);
        }
        for action in &mut s.actions.list {
            action.set_slot = action.set_slot.and_then(|slot| slots.get(slot).copied().flatten());
        }
    }
    let mut taken = UniqueNames::new(s.actions.list.iter().map(|a| a.name.clone()));
    let mut names = Vec::with_capacity(imp.actions.len());
    // The file's name of each action that had to be renamed, and its new name.
    let mut renamed: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    let mut steps = 0usize;
    let mut added = Vec::with_capacity(imp.actions.len());
    for mut a in imp.actions {
        let name = taken.claim(&a.name);
        if name != a.name {
            renamed.entry(a.name.clone()).or_insert_with(|| name.clone());
        }
        a.name = name.clone();
        steps = steps.saturating_add(a.steps.len());
        names.push(name);
        added.push(a);
    }
    // Play, Conditional, Batch and Create Droplet steps call actions by name: follow the renames in
    // the actions just loaded, each target looked up once by the name the file gave it.
    for a in &mut added {
        for (id, params) in &mut a.steps {
            // A qualified Batch or Create Droplet reference belongs to the file's original set, not
            // to a foreign set which happens to have the destination override's name.
            let by_set = matches!(id.as_str(), "file.automate.batch" | "file.automate.createDroplet");
            if by_set && params.get("set").and_then(Value::as_str) == Some(original_set.as_str()) {
                params["set"] = json!(imp.set);
            } else if by_set && params.get("set").is_some_and(|set| !set.is_null()) {
                continue;
            }
            let keys: &[&str] = match id.as_str() {
                "actions.play" => &["action"],
                "actions.conditional" => &["then", "else"],
                "file.automate.batch" | "file.automate.createDroplet" => &["action"],
                _ => &[],
            };
            for key in keys {
                if let Some(to) = params.get(*key).and_then(Value::as_str).and_then(|n| renamed.get(n)) {
                    params[*key] = json!(to);
                }
            }
        }
    }
    // Actions an automation client loads keep its gate when the user plays them.
    let automation = s.authorize.is_some();
    for a in &mut added {
        a.automation = automation;
    }
    s.actions.list.extend(added);
    // A set saved with no actions (Photoshop saves an empty set) is an empty folder, like one New
    // Set made.
    if !s.actions.list.iter().any(|a| a.set.as_deref() == Some(imp.set.as_str())) && !s.actions.sets.contains(&imp.set) {
        s.actions.sets.push(imp.set.clone());
    }
    let keys: Vec<_> = function_keys.into_iter().map(|(index, key)| (names[index].clone(), key)).collect();
    if !keys.is_empty() {
        s.edit_prefs(|prefs| {
            for (name, key) in &keys {
                // A function key belongs to one action.
                for (assigned, value) in &mut prefs.shortcuts {
                    if crate::actions_cmds::is_action_key(assigned) && value == key {
                        value.clear();
                    }
                }
                prefs.shortcuts.insert(format!("{}{name}", crate::actions_cmds::PLAY_SHORTCUT_PREFIX), key.clone());
            }
        });
    }
    s.actions.touch();
    Ok(json!({
        "set": imp.set,
        "actions": names,
        "steps": steps,
        "unsupported": imp.unsupported,
        "functionKeys": keys.iter().map(|(n, k)| json!({"action": n, "key": k})).collect::<Vec<_>>(),
        "warnings": imp.warnings,
        "encoding": imp.encoding.as_deref().unwrap_or("UTF-8"),
    }))
}

pub(super) fn save(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "actions.save";
    crate::actions_cmds::flush_recording(s)?;
    let skip = match p.get("skipUnsupported") {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(_) => return Err(bad(cmd, "`skipUnsupported` must be true or false")),
    };
    let set_param = match p.get("set") {
        None | Some(Value::Null) => None,
        // A set is found by its exact name, as `actions.record` finds it: one loaded from a
        // Photoshop file or an older actions.json may have surrounding spaces. Others are trimmed.
        Some(Value::String(n)) if crate::actions_edit_cmds::set_names(s).contains(n) => Some(n.clone()),
        Some(Value::String(n)) if !n.trim().is_empty() => Some(n.trim().to_string()),
        Some(_) => return Err(bad(cmd, "`set` must be a non-empty string")),
    };
    let set_slot = crate::actions_edit_cmds::set_slot(s, p, set_param.as_deref(), cmd)?;
    if set_slot.is_some() && p.get("action").is_some() {
        return Err(bad(cmd, "select a folder occurrence or an action"));
    }
    let (set, indices): (String, Vec<usize>) = if p.get("action").is_some() {
        let idx = crate::actions_cmds::resolve(&s.actions.list, p, cmd)?;
        let a = &s.actions.list[idx];
        (set_param.or_else(|| a.set.clone()).unwrap_or_else(|| a.name.clone()), vec![idx])
    } else if let Some(set) = set_param {
        let list: Vec<usize> = s
            .actions
            .list
            .iter()
            .enumerate()
            .filter(|(_, a)| a.set.as_deref() == Some(set.as_str()) && (p.get("setSlot").is_none() || a.set_slot == set_slot))
            .map(|(index, _)| index)
            .collect();
        // An empty set New Set made is written with no actions, as Photoshop saves one.
        let known_row = p.get("setSlot").is_none() && s.actions.set_rows.contains(&set);
        if list.is_empty() && set_slot.is_none() && !known_row && !s.actions.sets.contains(&set) {
            return Err(bad(cmd, format!("no actions in a set named `{set}`")));
        }
        (set, list)
    } else {
        if s.actions.list.is_empty() {
            return Err(bad(cmd, "there are no actions to save"));
        }
        (DEFAULT_SET.to_string(), (0..s.actions.list.len()).collect())
    };
    let actions: Vec<&Action> = indices.iter().map(|&index| &s.actions.list[index]).collect();
    let names: Vec<String> = actions.iter().map(|a| a.name.clone()).collect();
    let key = |index, _: &Action| crate::actions_cmds::action_shortcut(s, indices[index]);
    let (bytes, skipped) = export_with_keys(&set, &actions, key, skip).map_err(|e| bad(cmd, e))?;
    let steps: usize = actions.iter().map(|a| a.steps.len()).sum::<usize>().saturating_sub(skipped.len());
    let mut out = json!({"set": set, "actions": names, "steps": steps, "bytes": bytes.len(), "skipped": skipped});
    match p.get("path") {
        None | Some(Value::Null) => {
            out["data"] = json!(photocraft_paint::tile::b64_encode(&bytes));
        }
        Some(Value::String(path)) if !path.trim().is_empty() => {
            write_file(path, &bytes).map_err(|e| EngineError::Other(format!("{path}: {e}")))?;
            out["path"] = json!(path);
        }
        Some(_) => return Err(bad(cmd, "`path` must be a non-empty string")),
    }
    Ok(out)
}

#[cfg(not(target_arch = "wasm32"))]
fn write_file(path: &str, bytes: &[u8]) -> std::io::Result<()> {
    // Atomic replacement (temporary file + rename): a failed write keeps the previous ATN (#7290).
    photocraft_format::atomic_write(std::path::Path::new(path), bytes)
}

#[cfg(target_arch = "wasm32")]
fn write_file(path: &str, _bytes: &[u8]) -> std::io::Result<()> {
    Err(std::io::Error::other(format!("cannot write paths on the web ({path}); omit `path` to get the bytes back as `data`")))
}
