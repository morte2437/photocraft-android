//! Edit › Keyboard Shortcuts › Set: named shortcut sets, as in Photoshop. The bindings in use
//! stay in [`Preferences::shortcuts`] / [`Preferences::alternate_shortcuts`] (what dispatch
//! reads); a set is a saved copy of them, `Preferences::shortcut_sets` keyed by name, and
//! `Preferences::shortcut_set` names the one in use ("" for the defaults). The bindings differ
//! from their set after an edit until it is saved ("modified", as Photoshop marks the set).
//! A set saves to and loads from a `.kys` file (`crate::kys::write` / the import), and
//! [`summarize`] writes the HTML table of Photoshop's Summarize button.
//!
//! `edit.keyboardShortcuts` takes the set parameters: `useSet` (load a set's bindings, before
//! the call's other changes), then `saveSet` (save the bindings as they end up under a name and
//! use it), `deleteSet`, `sets` (replace every saved set: the dialog's OK) and `activeSet`.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::prefs::{Preferences, alternate_keys, normalize_shortcut};

/// The built-in set's name: the default bindings.
pub const DEFAULTS_NAME: &str = "PhotoCraft Defaults";
/// Saved sets are small; the cap keeps a hostile settings file or call bounded.
pub const MAX_SETS: usize = 64;
/// Longest set name, in characters.
pub const MAX_NAME: usize = 64;

/// A saved set: the overrides of the default bindings, in the shape the preferences keep them.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ShortcutSet {
    /// Command id → main key; "" removes the default key.
    pub shortcuts: BTreeMap<String, String>,
    /// Command id → further keys, replacing the defaults.
    pub alternate_shortcuts: BTreeMap<String, Vec<String>>,
}

impl ShortcutSet {
    /// A set of these bindings, without the actions' function keys (they belong to the
    /// actions, as in an `.atn` file, not to a set).
    pub fn from_bindings(shortcuts: &BTreeMap<String, String>, alternates: &BTreeMap<String, Vec<String>>) -> Self {
        let shortcuts = shortcuts.iter().filter(|(id, _)| !is_action_key(id)).map(|(id, k)| (id.clone(), k.clone())).collect();
        let alternate_shortcuts = alternates.iter().filter(|(id, _)| !is_action_key(id)).map(|(id, k)| (id.clone(), k.clone())).collect();
        Self { shortcuts, alternate_shortcuts }
    }

    /// The bindings `p` uses now, without the actions' function keys.
    pub fn of(p: &Preferences) -> Self {
        Self::from_bindings(&p.shortcuts, &p.alternate_shortcuts)
    }

    /// Put this set's bindings in place of `shortcuts` / `alternates`, keeping the actions'
    /// function keys: switching sets with `useSet` and in the Keyboard Shortcuts dialog.
    pub fn apply(&self, shortcuts: &mut BTreeMap<String, String>, alternates: &mut BTreeMap<String, Vec<String>>) {
        shortcuts.retain(|id, _| is_action_key(id));
        shortcuts.extend(self.shortcuts.iter().filter(|(id, _)| !is_action_key(id)).map(|(id, k)| (id.clone(), k.clone())));
        alternates.retain(|id, _| is_action_key(id));
        alternates.extend(self.alternate_shortcuts.iter().filter(|(id, _)| !is_action_key(id)).map(|(id, k)| (id.clone(), k.clone())));
    }

    /// Use this set's bindings in `p`, keeping the actions' function keys.
    pub fn load_into(&self, p: &mut Preferences) {
        self.apply(&mut p.shortcuts, &mut p.alternate_shortcuts);
    }

    /// Check every key (a set from a call or a hand-edited file must not hold garbage).
    fn validate(&self) -> Result<(), String> {
        for (id, k) in &self.shortcuts {
            if !k.is_empty() && normalize_shortcut(k).is_none() {
                return Err(format!("`{k}` for `{id}` is not a valid shortcut (e.g. Cmd+Shift+N, F7)"));
            }
        }
        for (id, keys) in &self.alternate_shortcuts {
            alternate_keys(&json!(keys)).map_err(|e| format!("`{id}`: {e}"))?;
        }
        Ok(())
    }
}

/// An action's function key (`actions.play:<name>`): kept by the action, not by a set.
fn is_action_key(id: &str) -> bool {
    id.starts_with(crate::actions_cmds::PLAY_SHORTCUT_PREFIX)
}

/// A set name as given, trimmed; an error for an empty, overlong or reserved name.
pub fn set_name(name: &str) -> Result<String, String> {
    let n = name.trim();
    if n.is_empty() {
        return Err("a set needs a name".into());
    }
    if n.chars().count() > MAX_NAME {
        return Err(format!("a set name is at most {MAX_NAME} characters"));
    }
    if n.chars().any(char::is_control) {
        return Err("a set name can't contain control characters".into());
    }
    if n.eq_ignore_ascii_case(DEFAULTS_NAME) {
        return Err(format!("`{DEFAULTS_NAME}` is the built-in set"));
    }
    Ok(n.to_string())
}

/// The saved copy of the set in use: `None` for the defaults (or a set that no longer exists).
fn saved(p: &Preferences) -> Option<&ShortcutSet> {
    p.shortcut_sets.get(&p.shortcut_set)
}

/// Do these bindings differ from the set `active` of `sets` ("" or a missing name: the
/// defaults)? The actions' function keys don't count, on either side (a set saved before they
/// were left out may still hold some). The Keyboard Shortcuts dialog's "(modified)" too.
pub fn differs(sets: &BTreeMap<String, ShortcutSet>, active: &str, shortcuts: &BTreeMap<String, String>, alternates: &BTreeMap<String, Vec<String>>) -> bool {
    let now = ShortcutSet::from_bindings(shortcuts, alternates);
    match sets.get(active) {
        Some(s) => ShortcutSet::from_bindings(&s.shortcuts, &s.alternate_shortcuts) != now,
        None => now != ShortcutSet::default(),
    }
}

/// Do the bindings differ from the set in use?
pub fn modified(p: &Preferences) -> bool {
    differs(&p.shortcut_sets, &p.shortcut_set, &p.shortcuts, &p.alternate_shortcuts)
}

/// Before the call's binding changes: `useSet` loads a set ("" or [`DEFAULTS_NAME`]: the
/// defaults) and makes it the one in use. Returns whether it changed anything.
pub fn load(next: &mut Preferences, p: &Value) -> Result<bool, String> {
    let Some(v) = p.get("useSet") else { return Ok(false) };
    let name = v.as_str().ok_or("`useSet` is a set name")?.trim();
    if name.is_empty() || name.eq_ignore_ascii_case(DEFAULTS_NAME) {
        ShortcutSet::default().load_into(next);
        next.shortcut_set = String::new();
    } else {
        let set = next.shortcut_sets.get(name).cloned().ok_or_else(|| format!("no shortcut set named `{name}`"))?;
        set.load_into(next);
        next.shortcut_set = name.to_string();
    }
    Ok(true)
}

/// After the call's binding changes: `sets` (replace every saved set), `deleteSet`, `saveSet`
/// (save the bindings under a name and use it) and `activeSet`. Returns whether it changed
/// anything.
pub fn store(next: &mut Preferences, p: &Value) -> Result<bool, String> {
    let mut changed = false;
    if let Some(v) = p.get("sets") {
        let obj = v.as_object().ok_or("`sets` maps set names to {shortcuts, alternateShortcuts}")?;
        if obj.len() > MAX_SETS {
            return Err(format!("at most {MAX_SETS} shortcut sets"));
        }
        let mut sets = BTreeMap::new();
        for (name, s) in obj {
            let name = set_name(name)?;
            let set: ShortcutSet = serde_json::from_value(s.clone()).map_err(|e| format!("set `{name}`: {e}"))?;
            set.validate().map_err(|e| format!("set `{name}`: {e}"))?;
            // Like `saveSet`, a set holds no action keys.
            sets.insert(name, ShortcutSet::from_bindings(&set.shortcuts, &set.alternate_shortcuts));
        }
        next.shortcut_sets = sets;
        changed = true;
    }
    if let Some(v) = p.get("deleteSet") {
        let name = v.as_str().ok_or("`deleteSet` is a set name")?.trim();
        if next.shortcut_sets.remove(name).is_none() {
            return Err(format!("no shortcut set named `{name}`"));
        }
        if next.shortcut_set == name {
            next.shortcut_set = String::new();
        }
        changed = true;
    }
    if let Some(v) = p.get("saveSet") {
        let name = set_name(v.as_str().ok_or("`saveSet` is a set name")?)?;
        if !next.shortcut_sets.contains_key(&name) && next.shortcut_sets.len() >= MAX_SETS {
            return Err(format!("at most {MAX_SETS} shortcut sets"));
        }
        next.shortcut_sets.insert(name.clone(), ShortcutSet::of(next));
        next.shortcut_set = name;
        changed = true;
    }
    if let Some(v) = p.get("activeSet") {
        let name = v.as_str().ok_or("`activeSet` is a set name")?.trim();
        if name.is_empty() || name.eq_ignore_ascii_case(DEFAULTS_NAME) {
            next.shortcut_set = String::new();
        } else if next.shortcut_sets.contains_key(name) {
            next.shortcut_set = name.to_string();
        } else {
            return Err(format!("no shortcut set named `{name}`"));
        }
        changed = true;
    }
    Ok(changed)
}

/// The sets, the one in use and whether the bindings differ from it, for the command's reply.
pub fn describe(p: &Preferences) -> Value {
    json!({
        "sets": p.shortcut_sets.keys().collect::<Vec<_>>(),
        "activeSet": if saved(p).is_some() { p.shortcut_set.as_str() } else { "" },
        "defaultsName": DEFAULTS_NAME,
        "modified": modified(p),
    })
}

/// A key as the Summarize table shows it on `mac` (⌘ spelled Cmd, ⌥ Option) or elsewhere
/// (Ctrl, Alt).
fn key_label(sc: &str, mac: bool) -> String {
    let Some(n) = normalize_shortcut(sc) else { return sc.to_string() };
    // `Cmd++` splits into an empty part before the key, which joins back as `++`.
    let mut parts: Vec<&str> = Vec::new();
    for part in n.split('+') {
        let shown = match (part, mac) {
            ("Ctrl", true) => "Control",
            ("Cmd", false) | ("Ctrl", false) => "Ctrl",
            ("Alt", true) => "Option",
            (p, _) => p,
        };
        // Off the Mac, ⌘ and Control are both Ctrl: say it once.
        if shown == "Ctrl" && parts.contains(&"Ctrl") {
            continue;
        }
        parts.push(shown);
    }
    parts.join("+")
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// A Summarize row: a command's label and its keys, the main one first.
pub type Row = (String, Vec<String>);
/// A Summarize section: its title and rows.
pub type Section = (String, Vec<Row>);

/// Photoshop's Summarize: an HTML page listing every command with a key, by section
/// (`sections` in display order: section title, then (command label, keys with the main one
/// first)). Commands without a key are left out.
pub fn summarize(set: &str, sections: &[Section], mac: bool) -> String {
    let mut out = String::new();
    out.push_str("<!DOCTYPE html>\n<html><head><meta charset=\"utf-8\"><title>");
    out.push_str(&html_escape(set));
    out.push_str("</title>\n<style>body{font-family:sans-serif;margin:2em}table{border-collapse:collapse;margin-bottom:2em}th,td{border:1px solid #bbb;padding:4px 10px;text-align:left}th{background:#eee}</style>\n</head><body>\n<h1>");
    out.push_str(&html_escape(set));
    out.push_str("</h1>\n");
    for (title, rows) in sections {
        let rows: Vec<&Row> = rows.iter().filter(|(_, keys)| !keys.is_empty()).collect();
        if rows.is_empty() {
            continue;
        }
        out.push_str("<h2>");
        out.push_str(&html_escape(title));
        out.push_str("</h2>\n<table><tr><th>Command</th><th>Shortcut</th></tr>\n");
        for (label, keys) in rows {
            let keys: Vec<String> = keys.iter().map(|k| html_escape(&key_label(k, mac))).collect();
            out.push_str("<tr><td>");
            out.push_str(&html_escape(label));
            out.push_str("</td><td>");
            out.push_str(&keys.join(", "));
            out.push_str("</td></tr>\n");
        }
        out.push_str("</table>\n");
    }
    out.push_str("</body></html>\n");
    out
}

/// A command (or tool) with a key under some bindings.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Bound {
    pub id: String,
    pub label: String,
    /// Its top-level menu ("Tools" for tools).
    pub menu: String,
    /// Main key first, then the further keys.
    pub keys: Vec<String>,
    pub tool: bool,
}

/// Every bindable command with a key under the bindings of `p`: the engine's and the shell's
/// commands, the tools and the temporary tools ([`crate::prefs::bindable_items`]).
pub fn bound(p: &Preferences) -> Vec<Bound> {
    crate::prefs::bindable_items()
        .filter_map(|(id, label, menu, def, kind)| {
            let main = p.shortcut(id, def);
            let mut keys: Vec<String> = main.and_then(normalize_shortcut).into_iter().collect();
            keys.extend(p.alternates(id, main));
            let menu = menu.first().copied().unwrap_or("Other").to_string();
            (!keys.is_empty()).then(|| Bound { id: id.to_string(), label: label.to_string(), menu, keys, tool: kind == crate::prefs::BindKind::Tool })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prefs() -> Preferences {
        Preferences::default()
    }

    #[test]
    fn save_use_modify_and_delete_a_set() {
        let mut p = prefs();
        assert!(!modified(&p));
        p.shortcuts.insert("edit.copy".into(), "F9".into());
        assert!(modified(&p), "an edit to the defaults");
        assert!(store(&mut p, &json!({"saveSet": "  Mine "})).unwrap());
        assert_eq!((p.shortcut_set.as_str(), modified(&p)), ("Mine", false));
        p.shortcuts.insert("edit.cut".into(), "F8".into());
        assert!(modified(&p));
        // Back to the defaults, then to the saved set as it was saved.
        assert!(load(&mut p, &json!({"useSet": ""})).unwrap());
        assert!(p.shortcuts.is_empty() && p.shortcut_set.is_empty() && !modified(&p));
        load(&mut p, &json!({"useSet": "Mine"})).unwrap();
        assert_eq!(p.shortcuts.get("edit.copy").map(String::as_str), Some("F9"));
        assert_eq!(p.shortcuts.get("edit.cut"), None, "the unsaved edit isn't in the set");
        assert_eq!(describe(&p)["sets"], json!(["Mine"]));
        // Deleting the set in use falls back to the defaults' name; the bindings stay.
        store(&mut p, &json!({"deleteSet": "Mine"})).unwrap();
        assert!(p.shortcut_sets.is_empty() && p.shortcut_set.is_empty());
        assert_eq!(describe(&p)["activeSet"], "");
        assert!(modified(&p), "F9 differs from the defaults");
    }

    #[test]
    fn sets_from_the_dialog_or_an_old_file_hold_no_action_keys() {
        // The dialog's OK sends its sets as `sets`; an action key in one is dropped.
        let mut p = prefs();
        let sets = json!({"Mine": {"shortcuts": {"edit.copy": "F9", "actions.play:Sepia": "F2"}}});
        store(&mut p, &json!({"sets": sets, "activeSet": "Mine"})).unwrap();
        assert_eq!(p.shortcut_sets["Mine"].shortcuts, BTreeMap::from([("edit.copy".to_string(), "F9".to_string())]));
        // A set saved before action keys were left out still holds one: it doesn't count.
        p.shortcut_sets.get_mut("Mine").unwrap().shortcuts.insert("actions.play:Old".into(), "F5".into());
        p.shortcuts = BTreeMap::from([("edit.copy".to_string(), "F9".to_string()), ("actions.play:Sepia".to_string(), "F2".to_string())]);
        assert!(!modified(&p), "neither side's action keys count");
        p.shortcuts.insert("edit.copy".into(), "F8".into());
        assert!(modified(&p), "a real edit does");
    }

    #[test]
    fn switching_sets_keeps_action_function_keys() {
        // An action's F-key belongs to the action (as in an .atn file), not to a shortcut set.
        let mut p = prefs();
        store(&mut p, &json!({"saveSet": "Mine"})).unwrap();
        p.shortcuts.insert("actions.play:Sepia".into(), "F2".into());
        assert!(!modified(&p), "an action key doesn't modify the set");
        load(&mut p, &json!({"useSet": ""})).unwrap();
        assert_eq!(p.shortcuts.get("actions.play:Sepia").map(String::as_str), Some("F2"), "kept switching to the defaults");
        load(&mut p, &json!({"useSet": "Mine"})).unwrap();
        assert_eq!(p.shortcuts.get("actions.play:Sepia").map(String::as_str), Some("F2"), "kept switching to a named set");
        store(&mut p, &json!({"saveSet": "Other"})).unwrap();
        assert!(p.shortcut_sets.values().all(|s| s.shortcuts.is_empty()), "a saved set holds no action keys");
    }

    #[test]
    fn switching_sets_keeps_action_alternate_keys() {
        // An action key set as an alternate (`alternateShortcuts.actions.play:<name>`) is the
        // action's too: a saved set leaves it out and switching sets keeps it.
        let mut p = prefs();
        p.alternate_shortcuts.insert("edit.copy".into(), vec!["F9".into()]);
        p.alternate_shortcuts.insert("actions.play:Sepia".into(), vec!["F7".into()]);
        store(&mut p, &json!({"saveSet": "Mine"})).unwrap();
        assert_eq!(p.shortcut_sets["Mine"].alternate_shortcuts, BTreeMap::from([("edit.copy".to_string(), vec!["F9".to_string()])]));
        load(&mut p, &json!({"useSet": ""})).unwrap();
        assert_eq!(p.alternate_shortcuts, BTreeMap::from([("actions.play:Sepia".to_string(), vec!["F7".to_string()])]), "kept switching to the defaults");
        let sets = json!({"Other": {"alternateShortcuts": {"edit.cut": ["F8"], "actions.play:Old": ["F6"]}}});
        store(&mut p, &json!({"sets": sets})).unwrap();
        load(&mut p, &json!({"useSet": "Other"})).unwrap();
        assert_eq!(
            p.alternate_shortcuts,
            BTreeMap::from([("actions.play:Sepia".to_string(), vec!["F7".to_string()]), ("edit.cut".to_string(), vec!["F8".to_string()])]),
            "a set from the dialog brings no action key and keeps the one in use"
        );
    }

    #[test]
    fn bad_names_sets_and_params_are_errors() {
        let mut p = prefs();
        for name in ["", "   ", "PhotoCraft defaults", "a\u{7}b", &"x".repeat(65)] {
            assert!(store(&mut p, &json!({"saveSet": name})).is_err(), "{name:?}");
        }
        assert!(load(&mut p, &json!({"useSet": "Nope"})).is_err());
        assert!(load(&mut p, &json!({"useSet": 3})).is_err());
        assert!(store(&mut p, &json!({"deleteSet": "Nope"})).is_err());
        assert!(store(&mut p, &json!({"activeSet": "Nope"})).is_err());
        assert!(store(&mut p, &json!({"sets": []})).is_err());
        assert!(store(&mut p, &json!({"sets": {"A": {"shortcuts": {"edit.copy": "Cmd+Shift+Ctrl"}}}})).is_err(), "no key");
        let nine: Vec<String> = (1..=9).map(|i| format!("F{i}")).collect();
        assert!(store(&mut p, &json!({"sets": {"A": {"alternateShortcuts": {"edit.copy": nine}}}})).is_err());
        assert!(store(&mut p, &json!({"sets": {"A": {"shortcuts": 5}}})).is_err());
        let many: serde_json::Map<String, Value> = (0..=MAX_SETS).map(|i| (format!("S{i}"), json!({}))).collect();
        assert!(store(&mut p, &json!({ "sets": many })).is_err());
        for i in 0..MAX_SETS {
            store(&mut p, &json!({"saveSet": format!("S{i}")})).unwrap();
        }
        assert!(store(&mut p, &json!({"saveSet": "One more"})).is_err());
        assert!(store(&mut p, &json!({"saveSet": "S3"})).is_ok(), "saving over a set is fine at the cap");
    }

    #[test]
    fn sets_replace_all_and_active_set_follows() {
        let mut p = prefs();
        store(&mut p, &json!({"sets": {"A": {"shortcuts": {"edit.copy": "F9"}}, "B": {}}, "activeSet": "A"})).unwrap();
        assert_eq!(p.shortcut_set, "A");
        assert!(modified(&p), "activeSet names the set; it doesn't load it");
        load(&mut p, &json!({"useSet": "A"})).unwrap();
        assert!(!modified(&p));
        store(&mut p, &json!({"activeSet": "PhotoCraft Defaults"})).unwrap();
        assert_eq!(p.shortcut_set, "");
    }

    #[test]
    fn summarize_lists_bound_commands_by_section_for_each_platform() {
        let sections = vec![
            ("Edit".to_string(), vec![("Copy".to_string(), vec!["Cmd+C".to_string(), "F3".to_string()]), ("Unbound".to_string(), vec![])]),
            ("Empty".to_string(), vec![("Nothing".to_string(), vec![])]),
            ("Filter <x>".to_string(), vec![("Gaussian Blur & more".to_string(), vec!["Ctrl+Alt+Cmd+G".to_string()])]),
        ];
        let mac = summarize("My <Set>", &sections, true);
        assert!(mac.contains("<title>My &lt;Set&gt;</title>") && mac.contains("<td>Copy</td><td>Cmd+C, F3</td>"), "{mac}");
        assert!(mac.contains("Cmd+Control+Option+G") && mac.contains("Gaussian Blur &amp; more") && mac.contains("Filter &lt;x&gt;"), "{mac}");
        assert!(!mac.contains("Unbound") && !mac.contains("Empty"), "{mac}");
        let pc = summarize("S", &sections, false);
        assert!(pc.contains("<td>Ctrl+C, F3</td>") && pc.contains("Ctrl+Alt+G"), "{pc}");
        assert_eq!(key_label("Cmd++", true), "Cmd++");
    }

    #[test]
    fn bound_lists_main_keys_then_alternates() {
        let p = prefs();
        let b = bound(&p);
        let copy = b.iter().find(|x| x.id == "edit.copy").expect("Copy has keys");
        assert_eq!(copy.keys, vec!["Cmd+C".to_string(), "F3".to_string()]);
        assert!(b.iter().any(|x| x.id == "tools.temporary.hand"));
        // The shell's commands and the tools too: a set saved headless is the whole set.
        assert!(b.iter().any(|x| x.id == "view.zoomIn" && x.keys.len() >= 2), "Zoom In with its Cmd++");
        let mv = b.iter().find(|x| x.id == "tools.select.move").expect("Move Tool");
        assert!(mv.tool && mv.keys == ["V"] && mv.menu == "Tools");
    }
}
