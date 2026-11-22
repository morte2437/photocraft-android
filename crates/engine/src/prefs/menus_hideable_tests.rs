//! #14335: which items Edit › Menus can hide, as measured in the reference application. File ›
//! Clear Recent File List, Close, Save, Save As…, Save a Copy… and Exit and Edit › Menus… can't;
//! Edit › Keyboard Shortcuts…, the Preferences pages and the Help items can; every item can be
//! coloured.

use serde_json::json;

use super::MenuCustomization;
use crate::Session;

const NOT_HIDEABLE: [&str; 7] = ["file.clearRecent", "file.close", "file.save", "file.saveAs", "file.saveACopy", "file.exit", "edit.menus"];

/// Hideable now: Keyboard Shortcuts, Preferences and its pages, and Help items.
const HIDEABLE: [&str; 6] =
    ["edit.keyboardShortcuts", "edit.preferences.general", "edit.preferences.interface", "edit.preferences.performance", "help.about", "help.systemInfo"];

#[test]
fn file_close_save_items_and_edit_menus_cannot_be_hidden() {
    for id in NOT_HIDEABLE {
        assert!(!MenuCustomization::hideable(id), "{id}");
        let mut s = Session::new();
        let err = s.execute("edit.menus", json!({"hide": [id]})).unwrap_err().to_string();
        assert!(err.contains("can't be hidden") && err.contains(id), "{id}: {err}");
        assert!(s.prefs().menus.hidden.is_empty(), "{id}: nothing stored");
    }
}

#[test]
fn keyboard_shortcuts_preferences_and_help_can_be_hidden() {
    let mut s = Session::new();
    for id in HIDEABLE {
        assert!(MenuCustomization::hideable(id), "{id}");
    }
    s.execute("edit.menus", json!({"hide": HIDEABLE})).unwrap();
    for id in HIDEABLE {
        assert!(s.prefs().menus.is_hidden(id), "{id}");
    }
    s.execute("edit.menus", json!({"show": HIDEABLE})).unwrap();
    assert!(s.prefs().menus.hidden.is_empty());
}

#[test]
fn an_older_preferences_file_listing_them_leaves_them_shown() {
    let mut s = Session::new();
    let hidden: Vec<&str> = NOT_HIDEABLE.iter().copied().chain(["edit.fade"]).collect();
    s.execute("prefs.set", json!({"values": {"menus": {"hidden": hidden, "colors": {}}}})).unwrap();
    for id in NOT_HIDEABLE {
        assert!(!s.prefs().menus.is_hidden(id), "{id} stays shown");
    }
    assert!(s.prefs().menus.is_hidden("edit.fade"));
}

#[test]
fn items_that_cannot_be_hidden_can_still_be_coloured() {
    let mut s = Session::new();
    let colors: serde_json::Map<String, serde_json::Value> = NOT_HIDEABLE.iter().map(|id| (id.to_string(), json!("blue"))).collect();
    s.execute("edit.menus", json!({"color": colors})).unwrap();
    for id in NOT_HIDEABLE {
        assert_eq!(s.prefs().menus.colors.get(id).map(String::as_str), Some("blue"), "{id}");
    }
}

#[test]
fn the_other_menus_keep_todays_behaviour() {
    for id in [
        "edit.fade",
        "file.new",
        "file.open",
        "image.adjustments.levels",
        "layer.new.layer",
        "select.all",
        "filter.blur.gaussianBlur",
        "view.zoomIn",
        "window.panel.layers",
    ] {
        assert!(MenuCustomization::hideable(id), "{id}");
    }
}
