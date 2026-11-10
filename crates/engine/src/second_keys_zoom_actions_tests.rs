//! The reference application's second default keys for View > 100% and Window > Actions (#11396):
//! Alt+Ctrl+0 beside Ctrl+1, and F9 beside Alt+F9.
//!
//! The engine owns the key tables and the Keyboard Shortcuts command. The zoom itself and the
//! Actions panel are UI state (`window.panel.actions` is a menu item the engine does not run), so
//! these tests check that each key resolves to its command exactly as the main key does and
//! that the user can remove it. They do not exercise key dispatch; `alternate_keys_run_their_command`
//! in the UI crate's `shortcut_dispatch` tests does, and lists both keys.

use crate::Session;
use crate::prefs::{ALTERNATE_SHORTCUTS, default_alternates, normalize_shortcut};
use serde_json::json;

/// The menus' main keys (`menu_catalog.rs`): what View > 100% and Window > Actions show.
const ACTUAL_PIXELS_MAIN: &str = "Cmd+1";
const ACTIONS_MAIN: &str = "Alt+F9";

/// Our notation: `Cmd` is Ctrl off the Mac, so this is Ctrl+Alt+0 on Windows and Linux.
const ACTUAL_PIXELS_ALT: &str = "Cmd+Alt+0";

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 32, "height": 32})).unwrap();
    s
}

/// The command a key press runs among the engine's bindable commands and the two this issue
/// adds keys to: a command's main key (given for the UI-only one), else its alternates in effect.
fn resolve(s: &Session, key: &str) -> Vec<&'static str> {
    let want = normalize_shortcut(key).unwrap();
    let mains = [("view.actualPixels", Some(ACTUAL_PIXELS_MAIN)), ("window.panel.actions", Some(ACTIONS_MAIN))];
    let mut ids: Vec<&'static str> = Vec::new();
    for (id, main) in mains {
        let main_in_effect = s.prefs().shortcut(id, main);
        let is_main = main_in_effect.and_then(normalize_shortcut).as_ref() == Some(&want);
        if is_main || s.prefs().alternates(id, main_in_effect).contains(&want) {
            ids.push(id);
        }
    }
    ids
}

#[test]
fn both_keys_are_default_alternates() {
    assert!(ALTERNATE_SHORTCUTS.contains(&("view.actualPixels", "Cmd+Alt+0")));
    assert!(ALTERNATE_SHORTCUTS.contains(&("window.panel.actions", "F9")));
    assert_eq!(default_alternates("view.actualPixels").collect::<Vec<_>>(), ["Cmd+Alt+0"]);
    assert_eq!(default_alternates("window.panel.actions").collect::<Vec<_>>(), ["F9"]);
    let s = session();
    let zoom = s.prefs().alternates("view.actualPixels", Some(ACTUAL_PIXELS_MAIN));
    assert_eq!(zoom, [normalize_shortcut(ACTUAL_PIXELS_ALT).unwrap()]);
    assert_eq!(s.prefs().alternates("window.panel.actions", Some(ACTIONS_MAIN)), ["F9"]);
}

#[test]
fn each_key_resolves_to_its_command_like_the_main_key() {
    let s = session();
    assert_eq!(resolve(&s, ACTUAL_PIXELS_ALT), ["view.actualPixels"]);
    assert_eq!(resolve(&s, ACTUAL_PIXELS_MAIN), ["view.actualPixels"]);
    assert_eq!(resolve(&s, "F9"), ["window.panel.actions"]);
    assert_eq!(resolve(&s, ACTIONS_MAIN), ["window.panel.actions"]);
}

#[test]
fn the_menu_shows_only_the_main_key() {
    // The menus print the catalogue's key, and an alternate is never the main key in effect.
    let s = session();
    assert_eq!(s.prefs().shortcut("view.actualPixels", Some(ACTUAL_PIXELS_MAIN)), Some(ACTUAL_PIXELS_MAIN));
    assert_eq!(s.prefs().shortcut("window.panel.actions", Some(ACTIONS_MAIN)), Some(ACTIONS_MAIN));
    assert_eq!(crate::shell_commands::SHELL_COMMANDS.iter().find(|c| c.0 == "view.actualPixels").and_then(|c| c.3), Some(ACTUAL_PIXELS_MAIN));
}

#[test]
fn keyboard_shortcuts_lists_the_second_key_and_removing_it_stops_it() {
    let mut s = session();
    let r = s.execute("edit.keyboardShortcuts", json!({"filter": "view.actualPixels"})).unwrap();
    let row = r["commands"].as_array().unwrap().iter().find(|c| c["id"] == "view.actualPixels").unwrap().clone();
    assert_eq!(row["shortcut"], ACTUAL_PIXELS_MAIN);
    assert_eq!(row["alternates"], json!([normalize_shortcut(ACTUAL_PIXELS_ALT).unwrap()]));
    assert_eq!(row["defaultAlternates"], json!(["Cmd+Alt+0"]));

    s.execute("edit.keyboardShortcuts", json!({"alternates": {"view.actualPixels": []}})).unwrap();
    assert!(resolve(&s, ACTUAL_PIXELS_ALT).is_empty());
    assert_eq!(resolve(&s, ACTUAL_PIXELS_MAIN), ["view.actualPixels"], "the main key still works");

    // Window > Actions is a menu item, so it is not in the engine's list, but its second key can
    // still be set and removed, as the Keyboard Shortcuts dialog does.
    s.execute("edit.keyboardShortcuts", json!({"alternates": {"window.panel.actions": []}})).unwrap();
    assert!(resolve(&s, "F9").is_empty());
    assert_eq!(resolve(&s, ACTIONS_MAIN), ["window.panel.actions"]);

    s.execute("edit.keyboardShortcuts", json!({"reset": true})).unwrap();
    assert_eq!(resolve(&s, "F9"), ["window.panel.actions"]);
    assert_eq!(resolve(&s, ACTUAL_PIXELS_ALT), ["view.actualPixels"]);
}
