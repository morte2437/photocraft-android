use super::cmds::bad;
use super::files::{file_target, read_text_file, rename_path, writable, write_files_atomically};
use super::shortcuts::{BindKind, alternate_keys, bindable, bindable_items, conflicts, default_alternates, is_tool, normalize_shortcut, reserved_shortcut};
use super::{MenuCustomization, Preferences, Result, Session, Value, json};

/// Edit › Keyboard Shortcuts (engine part): list, change and reset application shortcuts.
pub(super) fn keyboard_shortcuts(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "edit.keyboardShortcuts";
    // Every change lands at once, after all of it validated: a bad key leaves the set as it was.
    let mut next = s.prefs().clone();
    // A set loads first, so the call's other changes apply on top of it.
    let mut changed = crate::shortcut_sets::load(&mut next, p).map_err(|e| bad(cmd, e))?;
    if let Some(r) = p.get("reset") {
        match r {
            Value::Bool(true) => {
                next.shortcuts.clear();
                next.alternate_shortcuts.clear();
                changed = true;
            }
            Value::Array(ids) => {
                for id in ids.iter().filter_map(Value::as_str) {
                    next.shortcuts.remove(id);
                    next.alternate_shortcuts.remove(id);
                }
                changed = true;
            }
            _ => {}
        }
    }
    // A Photoshop `.kys` set (`importKys`, its XML text) becomes `set` and `alternates` entries,
    // matched by label (`crate::kys`); a key equal to the command's default restores the
    // default, and keys given in `set` / `alternates` as well win over the file's.
    let imported;
    let mut import = None;
    let p = match p.get("importKys") {
        Some(v) => {
            // The file's text, or {"path": …} to read it.
            let read;
            let xml = match v {
                Value::String(s) => s.as_str(),
                Value::Object(o) => {
                    let path =
                        o.get("path").and_then(Value::as_str).filter(|s| !s.trim().is_empty()).ok_or_else(|| bad(cmd, "`importKys.path` is a .kys file"))?;
                    read = read_text_file(path, crate::kys::MAX_BYTES).map_err(|e| bad(cmd, format!("{path}: {e}")))?;
                    read.as_str()
                }
                _ => return Err(bad(cmd, "`importKys` is the text of a .kys file, or {\"path\": …}")),
            };
            let set = crate::kys::parse(xml).map_err(|e| bad(cmd, e))?;
            let plan = crate::kys::plan(crate::kys::command_candidates(), &set);
            let mut merged = serde_json::Map::new();
            // A key the system reserves is left out (reported as `reserved`), not a failed import.
            let mut skipped: Vec<String> = Vec::new();
            for (id, sc) in plan.set.iter().filter(|(id, sc)| {
                let bad = reserved_shortcut(sc);
                if bad {
                    skipped.push(format!("{id}: {sc}"));
                }
                !bad
            }) {
                let default = crate::command_specs().iter().find(|c| c.id == id).and_then(|c| c.shortcut).and_then(normalize_shortcut);
                merged.insert(id.clone(), if default.as_deref() == Some(sc.as_str()) { Value::Null } else { json!(sc) });
            }
            // Tool keys: a tool's own letter restores it (and its Shift cycling).
            let tools = crate::shell_commands::TOOLS.iter().map(|t| (t.0, t.1));
            let (tool_keys, unknown_tools) = crate::kys::plan_tools(tools, &set);
            for (id, key) in &tool_keys {
                let own = crate::shell_commands::tool_default(id).flatten().is_some_and(|d| d.eq_ignore_ascii_case(key));
                merged.insert(id.clone(), if own { Value::Null } else { json!(key) });
            }
            merged.extend(p.get("set").and_then(Value::as_object).cloned().unwrap_or_default());
            let mut alternates = serde_json::Map::new();
            for (id, keys) in &plan.alternate_keys {
                let keys: &Vec<String> = &keys.iter().filter(|k| !reserved_shortcut(k)).cloned().collect();
                let main = plan.set.get(id).map(String::as_str);
                let default = Preferences::default().alternates(id, main);
                alternates.insert(id.clone(), if *keys == default { Value::Null } else { json!(keys) });
            }
            alternates.extend(p.get("alternates").and_then(Value::as_object).cloned().unwrap_or_default());
            import = Some(json!({
                "name": set.name,
                "imported": plan.set.len(),
                "set": plan.set,
                "unknown": plan.unknown,
                "unreadable": plan.unreadable,
                "alternates": plan.alternates,
                "alternateKeys": plan.alternate_keys,
                "toolKeys": set.tool_keys,
                "tools": tool_keys,
                "unknownTools": unknown_tools,
                "reserved": skipped,
                "windows": set.windows,
            }));
            let mut with_import = p.clone();
            with_import["set"] = Value::Object(merged);
            with_import["alternates"] = Value::Object(alternates);
            imported = with_import;
            &imported
        }
        None => p,
    };
    let empty = serde_json::Map::new();
    let set = match p.get("set") {
        Some(Value::Object(m)) => m,
        _ => &empty,
    };
    let alts = match p.get("alternates") {
        None | Some(Value::Null) => &empty,
        Some(v) => v.as_object().ok_or_else(|| bad(cmd, "`alternates` maps command ids to lists of shortcuts"))?,
    };
    changed |= apply_shortcut_changes(&mut next, p, set, alts, cmd)?;
    // Then the sets are saved, deleted or switched, from the bindings as they now stand.
    changed |= crate::shortcut_sets::store(&mut next, p).map_err(|e| bad(cmd, e))?;
    // The reply and the exports read the bindings as they will be; they are committed only once
    // the exports are written, so a bad export target changes nothing.
    let prefs = &next;
    let filter = p.get("filter").and_then(Value::as_str).map(str::to_ascii_lowercase);
    let list: Vec<Value> = bindable_items()
        .filter(|c| filter.as_ref().is_none_or(|f| c.0.to_ascii_lowercase().contains(f) || c.1.to_ascii_lowercase().contains(f)))
        .filter(|c| {
            p.get("list").and_then(Value::as_bool).unwrap_or(false)
                || filter.is_some()
                || prefs.shortcuts.contains_key(c.0)
                || prefs.alternate_shortcuts.contains_key(c.0)
        })
        .map(|(id, label, menu, def, kind)| {
            let main = prefs.shortcut(id, def);
            let defaults: Vec<&str> = default_alternates(id).collect();
            let mut v = json!({"id": id, "label": label, "menu": menu, "default": def, "shortcut": main, "alternates": prefs.alternates(id, main)});
            if !defaults.is_empty() {
                v["defaultAlternates"] = json!(defaults);
            }
            match kind {
                BindKind::Hold => v["hold"] = json!(true),
                BindKind::Tool => v["tool"] = json!(true),
                BindKind::Command => {}
            }
            v
        })
        .collect();
    let mut bindings: Vec<(&str, String)> = Vec::new();
    for (id, def) in bindable() {
        let main = prefs.shortcut(id, def);
        bindings.extend(main.map(|m| (id, m.to_string())));
        bindings.extend(prefs.alternates(id, main).into_iter().map(|k| (id, k)));
    }
    // Tools sharing a key are a cycling group, not a conflict.
    let conflicts: Vec<Value> = conflicts(bindings.iter().map(|(id, k)| (*id, k.as_str())))
        .into_iter()
        .filter(|(_, ids)| !ids.iter().all(|id| is_tool(id)))
        .map(|(sc, ids)| json!({"shortcut": sc, "commands": ids}))
        .collect();
    let mut out = json!({"overrides": prefs.shortcuts, "alternateOverrides": prefs.alternate_shortcuts, "commands": list, "conflicts": conflicts});
    if let (Some(o), Value::Object(sets)) = (out.as_object_mut(), crate::shortcut_sets::describe(prefs)) {
        o.extend(sets);
    }
    // The bindings as a `.kys` file, or Photoshop's Summarize page: every bindable item (the
    // engine's commands, the shell's commands and the tools).
    let set_name = if prefs.shortcut_set.is_empty() { crate::shortcut_sets::DEFAULTS_NAME } else { prefs.shortcut_set.as_str() };
    // Both targets are resolved and their texts made before anything is written, and a path that
    // can't be written is refused before either file is: a bad export changes nothing.
    let kys_target = file_target(cmd, p.get("exportKys"))?;
    let html_target = file_target(cmd, p.get("summarize"))?;
    let mut files: Vec<(&str, String, String)> = Vec::new();
    if let Some(target) = kys_target {
        let entries: Vec<crate::kys::Entry> =
            crate::shortcut_sets::bound(prefs).into_iter().map(|b| crate::kys::Entry { id: b.id, label: b.label, tool: b.tool, keys: b.keys }).collect();
        let text = crate::kys::write(set_name, &entries);
        match target {
            Some(path) => files.push(("kysPath", path, text)),
            None => out["kys"] = json!(text),
        }
    }
    if let Some(target) = html_target {
        let v = p.get("summarize").cloned().unwrap_or(Value::Null);
        let mac = v.get("mac").and_then(Value::as_bool).unwrap_or(cfg!(target_os = "macos"));
        let mut sections: Vec<crate::shortcut_sets::Section> = Vec::new();
        for crate::shortcut_sets::Bound { label, menu, keys, .. } in crate::shortcut_sets::bound(prefs) {
            match sections.iter_mut().find(|(m, _)| *m == menu) {
                Some((_, rows)) => rows.push((label, keys)),
                None => sections.push((menu, vec![(label, keys)])),
            }
        }
        let html = crate::shortcut_sets::summarize(set_name, &sections, mac);
        match target {
            Some(path) => files.push(("htmlPath", path, html)),
            None => out["html"] = json!(html),
        }
    }
    for (_, path, _) in &files {
        writable(path).map_err(|e| bad(cmd, format!("{path}: {e}")))?;
    }
    let list: Vec<(&str, &str)> = files.iter().map(|(_, path, text)| (path.as_str(), text.as_str())).collect();
    write_files_atomically(&list, &rename_path).map_err(|e| bad(cmd, e))?;
    for (key, path, _) in &files {
        out[*key] = json!(path);
    }
    if changed {
        s.edit_prefs(|p| *p = next);
    }
    if let Some(import) = import {
        out["import"] = import;
    }
    Ok(out)
}

fn apply_shortcut_changes(
    next: &mut Preferences,
    p: &Value,
    set: &serde_json::Map<String, Value>,
    alts: &serde_json::Map<String, Value>,
    cmd: &str,
) -> Result<bool> {
    if set.is_empty() && alts.is_empty() {
        return Ok(false);
    }
    let allow_unknown = p.get("allowUnknown").and_then(Value::as_bool).unwrap_or(false);
    // Reserved system keys can't be bound, by hand or from a file.
    let reserved = |v: &Value| -> Option<String> {
        let keys = match v {
            Value::String(s) => vec![s.clone()],
            other => alternate_keys(other).unwrap_or_default(),
        };
        keys.into_iter().find(|k| reserved_shortcut(k))
    };
    if let Some((id, k)) = set.iter().chain(alts.iter()).find_map(|(id, v)| reserved(v).map(|k| (id, k))) {
        return Err(bad(cmd, format!("`{k}` (for `{id}`) is reserved by the system and can't be assigned")));
    }
    for (id, v) in set {
        if !bindable().any(|(b, _)| b == id) && !allow_unknown {
            return Err(bad(cmd, format!("unknown command `{id}`")));
        }
        next.set(&format!("shortcuts.{id}"), v.clone()).map_err(|e| bad(cmd, e))?;
    }
    for (id, v) in alts {
        if !bindable().any(|(b, _)| b == id) && default_alternates(id).next().is_none() && !allow_unknown {
            return Err(bad(cmd, format!("unknown command `{id}`")));
        }
        next.set(&format!("alternateShortcuts.{id}"), v.clone()).map_err(|e| bad(cmd, e))?;
    }
    // Like Photoshop, a shortcut moved to a new command is taken away from the old one,
    // whether the old one held it as its main key or as an alternate (#3665).
    if p.get("removeConflicts").and_then(Value::as_bool).unwrap_or(true) {
        let assigned: Vec<(&str, String)> = set
            .iter()
            .filter_map(|(id, v)| Some((id.as_str(), v.as_str().and_then(normalize_shortcut)?)))
            .chain(alts.iter().flat_map(|(id, v)| alternate_keys(v).unwrap_or_default().into_iter().map(move |k| (id.as_str(), k))))
            .collect();
        for (id, sc) in assigned {
            for (c, def) in bindable() {
                // Tools may share a key: they cycle with Shift, as Photoshop's groups do.
                if is_tool(id) && is_tool(c) {
                    continue;
                }
                // Never strip a command that this same call is assigning:
                // two entries for one key are a clash inside the call, which
                // the returned conflicts list reports (#719). Stripping
                // each other here unbound both silently.
                if c == id || set.contains_key(c) || alts.contains_key(c) {
                    continue;
                }
                let main = next.shortcut(c, def).map(str::to_string);
                if main.as_deref().and_then(normalize_shortcut).as_deref() == Some(sc.as_str()) {
                    next.shortcuts.insert(c.to_string(), String::new());
                }
                let held = next.alternates(c, main.as_deref());
                if held.contains(&sc) {
                    next.alternate_shortcuts.insert(c.to_string(), held.into_iter().filter(|k| *k != sc).collect());
                }
            }
        }
    }
    Ok(true)
}

/// Edit › Menus: hide/show items and give them colours.
pub(super) fn menus(s: &mut Session, p: &Value) -> Result<Value> {
    let ids = |k: &str| -> Vec<String> {
        p.get(k).and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect()).unwrap_or_default()
    };
    const COLORS: [&str; 8] = ["none", "red", "orange", "yellow", "green", "blue", "violet", "gray"];
    if let Some(m) = p.get("color").and_then(Value::as_object) {
        for v in m.values() {
            if let Some(c) = v.as_str().filter(|c| !COLORS.contains(c)) {
                return Err(bad("edit.menus", format!("unknown colour `{c}` ({})", COLORS.join("|"))));
            }
        }
    }
    let (hide, show) = (ids("hide"), ids("show"));
    if let Some(id) = hide.iter().find(|id| !MenuCustomization::hideable(id)) {
        return Err(bad("edit.menus", format!("`{id}` can't be hidden")));
    }
    s.edit_prefs(|pr| {
        if p.get("reset").and_then(Value::as_bool) == Some(true) {
            pr.menus = MenuCustomization::default();
        }
        for id in hide {
            if !pr.menus.hidden.contains(&id) {
                pr.menus.hidden.push(id);
            }
        }
        pr.menus.hidden.retain(|h| !show.contains(h));
        if let Some(m) = p.get("color").and_then(Value::as_object) {
            for (id, v) in m {
                match v.as_str() {
                    Some(c) if c != "none" => {
                        pr.menus.colors.insert(id.clone(), c.to_string());
                    }
                    _ => {
                        pr.menus.colors.remove(id);
                    }
                }
            }
        }
    });
    Ok(serde_json::to_value(&s.prefs().menus).unwrap_or_default())
}
