//! Keep a saved reference in its original namespace when startup attaches the disk library.
use std::collections::HashSet;

use serde_json::{Value, json};

use crate::Session;
use crate::actions_cmds::{
    Action, PLAY_SHORTCUT_PREFIX, branch_id_field, conditional_branch, prepare_action_id, resolve_play_target, shortcut_row, valid_action_id, visit_calls,
};

/// Explicit IDs that failed in their source list must also fail after the lists are joined.
fn block_unknown_ids(list: &mut [Action]) {
    let ids: HashSet<_> = list.iter().filter_map(|action| action.action_id.as_ref().filter(|id| valid_action_id(id)).cloned()).collect();
    for action in list {
        visit_calls(&mut action.steps, &mut |command, params, _| {
            let fields: &[&str] = match command {
                "actions.play" | "file.automate.batch" => &["actionId"],
                "actions.conditional" => &["thenActionId", "elseActionId"],
                _ if named_droplet(command, params) => &["actionId"],
                _ => &[],
            };
            for field in fields {
                if let Some(value) = params.get(*field)
                    && !value.as_str().is_some_and(|id| ids.contains(id))
                {
                    params[*field] = Value::Null;
                }
            }
        });
    }
}

fn named_droplet(command: &str, params: &Value) -> bool {
    command == "file.automate.createDroplet" && params.get("steps").is_none() && !params.get("action").is_some_and(Value::is_array)
}

// A set requires a name when there is no explicit ID.
fn numeric_droplet(command: &str, params: &Value) -> bool {
    named_droplet(command, params) && params.get("set").is_none_or(Value::is_null) && params.get("action").is_some_and(Value::is_number)
}

pub(super) fn pin_disk_targets(s: &mut Session, disk: &mut [Action], warnings: &mut Vec<String>) {
    if s.actions.list.is_empty() || disk.is_empty() {
        return;
    }
    block_unknown_ids(&mut s.actions.list);
    block_unknown_ids(disk);
    // Appending disk rows cannot move an existing memory index, but it can fill a missing one.
    for row in 0..s.actions.list.len() {
        let mut steps = std::mem::take(&mut s.actions.list[row].steps);
        let memory = &s.actions.list;
        visit_calls(&mut steps, &mut |command, params, _| {
            if params.get("actionId").is_some() || !params.get("action").is_some_and(Value::is_number) {
                return;
            }
            let resolved = match command {
                "actions.play" => resolve_play_target(memory, params, command),
                "file.automate.batch" => crate::file_cmds::batch_action(memory, params),
                _ if numeric_droplet(command, params) => resolve_play_target(memory, params, command),
                _ => return,
            };
            if resolved.is_err() {
                params["actionId"] = Value::Null;
            }
        });
        s.actions.list[row].steps = steps;
    }
    let memory = &s.actions.list;

    // Saved preferences predate the asynchronous attachment. Keep their legacy key, and
    // persist the disk owner with the library so a failed store write cannot orphan the key.
    let keys = s.prefs().shortcuts.keys().chain(s.prefs().alternate_shortcuts.keys());
    for key in keys {
        let Some(name) = key.strip_prefix(PLAY_SHORTCUT_PREFIX) else { continue };
        if memory.iter().any(|action| action.name == name)
            && !memory.iter().any(|action| action.name == name && action.legacy_shortcut_owner)
            && let Some(row) = shortcut_row(disk, key)
        {
            disk[row].legacy_shortcut_owner = true;
        }
    }

    for row in 0..disk.len() {
        let mut steps = std::mem::take(&mut disk[row].steps);
        visit_calls(&mut steps, &mut |command, params, _| match command {
            "actions.play" | "file.automate.batch" if params.get("actionId").is_none() => {
                let resolve = |list: &[Action]| {
                    if command == "actions.play" { resolve_play_target(list, params, command) } else { crate::file_cmds::batch_action(list, params) }
                };
                if params.get("action").is_some_and(Value::is_number) || resolve(memory).is_ok() {
                    let original = resolve(disk);
                    // A name or index that failed on disk must not acquire a target from the merge.
                    params["actionId"] = original.map_or(Value::Null, |row| pin_target(disk, row, warnings));
                }
            }
            "actions.conditional" => {
                for branch in ["then", "else"] {
                    let field = branch_id_field(branch);
                    if params.get(field).is_some() {
                        continue;
                    }
                    if let Ok(Some(target)) = conditional_branch(params, branch)
                        && resolve_play_target(memory, &target, command).is_ok()
                    {
                        let original = resolve_play_target(disk, &target, command);
                        params[field] = original.map_or(Value::Null, |row| pin_target(disk, row, warnings));
                    }
                }
            }
            _ if params.get("actionId").is_none() && numeric_droplet(command, params) => {
                let original = resolve_play_target(disk, params, command);
                params["actionId"] = original.map_or(Value::Null, |row| pin_target(disk, row, warnings));
            }
            _ => {}
        });
        disk[row].steps = steps;
    }
}

fn pin_target(list: &mut [Action], index: usize, warnings: &mut Vec<String>) -> Value {
    match prepare_action_id(list, index) {
        Ok(id) => {
            list[index].action_id = Some(id.clone());
            json!(id)
        }
        Err(error) => {
            warnings.push(format!("actions: {error}"));
            Value::Null
        }
    }
}
