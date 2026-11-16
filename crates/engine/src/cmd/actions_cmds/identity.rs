//! Persisted action occurrences are independent of name, list order, and folder position.
use std::collections::{HashMap, HashSet};

use super::{Action, PLAY_SHORTCUT_PREFIX, bad};
use crate::{Result, Session};

const OCCURRENCE_PREFIX: &str = "actions.playById:";
const COMMAND: &str = "action shortcut";

pub fn is_action_key(key: &str) -> bool {
    key.starts_with(PLAY_SHORTCUT_PREFIX) || key.starts_with(OCCURRENCE_PREFIX)
}

pub fn occurrence_key(id: &str) -> String {
    format!("{OCCURRENCE_PREFIX}{id}")
}

pub(crate) fn valid_action_id(id: &str) -> bool {
    id.len() == 32 && id.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

pub(crate) fn resolve_action_id(list: &[Action], id: &str) -> Option<usize> {
    if !valid_action_id(id) {
        return None;
    }
    let mut matches = list.iter().enumerate().filter(|(_, action)| action.action_id.as_deref() == Some(id));
    let (index, _) = matches.next()?;
    matches.next().is_none().then_some(index)
}

/// Never let a duplicated on-disk ID become valid just because one of its rows is removed.
pub(crate) fn invalidate_duplicate_ids(list: &mut [Action]) {
    let mut seen = HashSet::new();
    let mut duplicates = HashSet::new();
    for id in list.iter().filter_map(|action| action.action_id.as_ref()) {
        if !seen.insert(id) {
            duplicates.insert(id.clone());
        }
    }
    for action in list {
        if action.action_id.as_ref().is_some_and(|id| duplicates.contains(id)) {
            action.action_id = Some(String::new());
        }
    }
}

/// Reacquire a paused row by the same identity its playback used.
pub fn playback_row(list: &[Action], action: &Action) -> Option<usize> {
    if let Some(id) = action.action_id.as_deref().filter(|id| valid_action_id(id)) {
        return resolve_action_id(list, id);
    }
    list.iter().position(|other| other.name == action.name)
}

pub fn shortcut_row(list: &[Action], key: &str) -> Option<usize> {
    if let Some(id) = key.strip_prefix(OCCURRENCE_PREFIX) {
        return resolve_action_id(list, id);
    }
    let name = key.strip_prefix(PLAY_SHORTCUT_PREFIX)?;
    let mut pinned = list.iter().enumerate().filter(|(_, action)| action.name == name && action.legacy_shortcut_owner);
    if let Some((index, _)) = pinned.next() {
        return pinned.next().is_none().then_some(index);
    }
    list.iter().position(|action| action.name == name)
}

/// Prepare an ID without changing the library, so a denied shortcut edit leaves it untouched.
pub fn prepare_action_id(list: &[Action], index: usize) -> Result<String> {
    let action = list.get(index).ok_or_else(|| bad(COMMAND, "action no longer exists"))?;
    if let Some(id) = &action.action_id {
        return if resolve_action_id(list, id) == Some(index) { Ok(id.clone()) } else { Err(bad(COMMAND, "invalid or duplicate action ID")) };
    }
    let mut bytes = [0; 16];
    getrandom::fill(&mut bytes).map_err(|error| bad(COMMAND, format!("could not create an action ID: {error}")))?;
    let id = format!("{:032x}", u128::from_be_bytes(bytes));
    if list.iter().any(|a| a.action_id.as_deref() == Some(id.as_str())) {
        return Err(bad(COMMAND, "duplicate generated action ID"));
    }
    Ok(id)
}

/// Save a new identity before the shortcut edit can reach the separate preferences file.
pub fn commit_action_id(s: &mut Session, index: usize, id: String) -> Result<()> {
    let action = s.actions.list.get(index).ok_or_else(|| bad(COMMAND, "action no longer exists"))?;
    let owner = resolve_action_id(&s.actions.list, &id);
    if let Some(current) = &action.action_id {
        return if current == &id && owner == Some(index) { Ok(()) } else { Err(bad(COMMAND, "invalid or duplicate action ID")) };
    }
    if !valid_action_id(&id) || s.actions.list.iter().any(|action| action.action_id.as_deref() == Some(id.as_str())) {
        return Err(bad(COMMAND, "invalid or duplicate action ID"));
    }
    s.actions.list[index].action_id = Some(id);
    let rev = s.actions.rev.saturating_add(1);
    if let Some(store) = s.preset_store.as_mut()
        && let Err(error) = store.commit_action_definitions(&s.actions, rev)
    {
        s.actions.list[index].action_id = None;
        return Err(bad(COMMAND, error));
    }
    s.actions.touch();
    Ok(())
}

/// A valid explicit key takes precedence over a legacy key for the same row.
pub fn action_shortcut(s: &Session, index: usize) -> Option<&str> {
    let list = &s.actions.list;
    let action = list.get(index)?;
    let keys = &s.prefs().shortcuts;
    if let Some(id) = action.action_id.as_deref()
        && let Some(key) = keys.get(&occurrence_key(id))
        && resolve_action_id(list, id) == Some(index)
    {
        return Some(key.as_str());
    }
    let legacy = format!("{PLAY_SHORTCUT_PREFIX}{}", action.name);
    let key = keys.get(&legacy)?;
    (shortcut_row(list, &legacy) == Some(index)).then_some(key.as_str())
}

/// Resolve all panel shortcut labels in one library pass, with explicit empty keys retained.
pub fn action_shortcuts(s: &Session) -> Vec<Option<&str>> {
    let list = &s.actions.list;
    let mut ids: HashMap<&str, Option<usize>> = HashMap::new();
    let mut first = HashMap::new();
    let mut pinned: HashMap<&str, Option<usize>> = HashMap::new();
    for (row, action) in list.iter().enumerate() {
        first.entry(action.name.as_str()).or_insert(row);
        if action.legacy_shortcut_owner {
            pinned.entry(action.name.as_str()).and_modify(|owner| *owner = None).or_insert(Some(row));
        }
        if let Some(id) = action.action_id.as_deref().filter(|id| valid_action_id(id)) {
            ids.entry(id).and_modify(|owner| *owner = None).or_insert(Some(row));
        }
    }
    let keys = &s.prefs().shortcuts;
    list.iter()
        .enumerate()
        .map(|(row, action)| {
            if let Some(id) = action.action_id.as_deref()
                && ids.get(id) == Some(&Some(row))
                && let Some(key) = keys.get(&occurrence_key(id))
            {
                return Some(key.as_str());
            }
            let owner = pinned.get(action.name.as_str()).copied().unwrap_or_else(|| first.get(action.name.as_str()).copied());
            if owner != Some(row) {
                return None;
            }
            keys.get(&format!("{PLAY_SHORTCUT_PREFIX}{}", action.name)).map(String::as_str)
        })
        .collect()
}

pub(crate) fn has_legacy_shortcut(s: &Session, name: &str) -> bool {
    let key = format!("{PLAY_SHORTCUT_PREFIX}{name}");
    s.prefs().shortcuts.contains_key(&key) || s.prefs().alternate_shortcuts.contains_key(&key)
}

/// Remove only the bindings owned by the deleted rows, before the indices or first match change.
pub(crate) fn remove_shortcuts(s: &mut Session, remove: impl Fn(usize, &Action) -> bool) {
    let list = &s.actions.list;
    let mut keys = s
        .prefs()
        .shortcuts
        .keys()
        .chain(s.prefs().alternate_shortcuts.keys())
        .filter(|key| shortcut_row(list, key).is_some_and(|row| remove(row, &list[row])))
        .cloned()
        .peekable();
    if keys.peek().is_none() {
        return;
    }
    let keys: Vec<_> = keys.collect();
    s.edit_prefs(|prefs| {
        for key in keys {
            prefs.shortcuts.remove(&key);
            prefs.alternate_shortcuts.remove(&key);
        }
    });
}
