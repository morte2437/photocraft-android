//! Edit › Keyboard Shortcuts › Set through `edit.keyboardShortcuts`: save, switch, delete,
//! export to `.kys` and back, Summarize, and bad params as errors.

use photocraft_engine::Session;
use serde_json::{Value, json};

fn ks(s: &mut Session, p: Value) -> Value {
    s.execute("edit.keyboardShortcuts", p).expect("edit.keyboardShortcuts")
}

#[test]
fn a_set_saves_switches_exports_and_imports_back() {
    let mut s = Session::new();
    let r = ks(&mut s, json!({}));
    assert_eq!((r["activeSet"].as_str(), r["modified"].as_bool(), r["defaultsName"].as_str()), (Some(""), Some(false), Some("PhotoCraft Defaults")));
    // Edit, then save the bindings as they end up after this call's edits.
    let r = ks(&mut s, json!({"set": {"edit.copy": "F9"}, "alternates": {"edit.copy": []}, "saveSet": "Mine"}));
    assert_eq!((r["activeSet"].as_str(), r["modified"].as_bool(), r["sets"].clone()), (Some("Mine"), Some(false), json!(["Mine"])));
    // Export the set, go back to the defaults, then import the file: the bindings return.
    let xml = ks(&mut s, json!({"exportKys": true}))["kys"].as_str().expect("kys").to_string();
    assert!(xml.contains("KeyboardCustomization/Mine=Mine") && xml.contains("pc-id=\"edit.copy\"><shortcut>F9</shortcut></command>"), "{xml}");
    let r = ks(&mut s, json!({"useSet": ""}));
    assert_eq!((r["overrides"].clone(), r["activeSet"].as_str()), (json!({}), Some("")));
    let r = ks(&mut s, json!({"reset": true, "importKys": xml}));
    assert_eq!(r["overrides"]["edit.copy"], "F9");
    assert_eq!(r["alternateOverrides"]["edit.copy"], json!([]), "Copy's F3 stays removed");
    assert_eq!(r["modified"], true, "imported over the defaults");
    // Switch back to the saved set; a later unsaved edit marks it modified.
    ks(&mut s, json!({"useSet": "Mine"}));
    let r = ks(&mut s, json!({"set": {"edit.cut": "F8"}}));
    assert_eq!((r["activeSet"].as_str(), r["modified"].as_bool()), (Some("Mine"), Some(true)));
    // Summarize lists what's bound, per platform.
    let html = ks(&mut s, json!({"summarize": {"mac": false}}))["html"].as_str().expect("html").to_string();
    assert!(html.contains("<title>Mine</title>") && html.contains("<td>Copy</td><td>F9</td>") && html.contains("<td>Cut</td><td>F8"), "{html}");
    // Delete the set in use: the defaults' name, the bindings kept.
    let r = ks(&mut s, json!({"deleteSet": "Mine"}));
    assert_eq!((r["sets"].clone(), r["activeSet"].as_str(), r["overrides"]["edit.copy"].as_str()), (json!([]), Some(""), Some("F9")));
}

#[test]
fn bad_set_params_change_nothing() {
    let mut s = Session::new();
    ks(&mut s, json!({"saveSet": "Keep"}));
    let before = s.prefs().clone();
    for p in [
        json!({"useSet": "Missing"}),
        json!({"useSet": 7}),
        json!({"saveSet": ""}),
        json!({"saveSet": "PhotoCraft Defaults"}),
        json!({"deleteSet": "Missing"}),
        json!({"activeSet": "Missing"}),
        json!({"sets": "nope"}),
        json!({"sets": {"A": {"shortcuts": {"edit.copy": "Cmd+Shift+Ctrl"}}}}),
        // A bad key elsewhere in the call keeps the set from being saved too.
        json!({"set": {"edit.copy": "Cmd+Shift+Ctrl"}, "saveSet": "New"}),
    ] {
        assert!(s.execute("edit.keyboardShortcuts", p.clone()).is_err(), "{p}");
        assert_eq!(s.prefs(), &before, "{p} changed the preferences");
    }
}
