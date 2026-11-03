//! Edit › Keyboard Shortcuts without the UI: the shell's commands and the tools are bindable,
//! listed, imported and exported; tools share keys without conflicts; system keys are refused;
//! sets read and write files by path.

use photocraft_engine::Session;
use serde_json::{Value, json};

fn ks(s: &mut Session, p: Value) -> Value {
    s.execute("edit.keyboardShortcuts", p).expect("edit.keyboardShortcuts")
}

fn listed<'a>(r: &'a Value, id: &str) -> &'a Value {
    r["commands"].as_array().unwrap().iter().find(|c| c["id"] == id).unwrap_or_else(|| panic!("{id} listed"))
}

#[test]
fn shell_commands_and_tools_are_bindable_without_the_ui() {
    let mut s = Session::new();
    let r = ks(&mut s, json!({"list": true}));
    assert_eq!(listed(&r, "view.zoomIn")["alternates"], json!(["Cmd++"]), "the shell's View › Zoom In");
    let mv = listed(&r, "tools.select.move");
    assert_eq!((mv["tool"].clone(), mv["shortcut"].clone(), mv["menu"].clone()), (json!(true), json!("V"), json!(["Tools"])));
    assert!(
        r["conflicts"].as_array().unwrap().iter().all(|c| c["commands"]
            .as_array()
            .unwrap()
            .iter()
            .any(|id| !id.as_str().unwrap().starts_with("tools.select."))),
        "tools sharing a letter cycle, they don't conflict: {}",
        r["conflicts"]
    );
    // No allowUnknown needed any more.
    let r = ks(&mut s, json!({"set": {"tools.select.pencil": "F6", "view.zoomIn": "Cmd+Shift+Z"}}));
    assert_eq!((r["overrides"]["tools.select.pencil"].clone(), r["overrides"]["view.zoomIn"].clone()), (json!("F6"), json!("Cmd+Shift+Z")));
}

#[test]
fn tools_may_share_a_key_but_a_command_takes_it() {
    let mut s = Session::new();
    // Pencil joins the V group: Move and Artboard keep V.
    let r = ks(&mut s, json!({"set": {"tools.select.pencil": "V"}}));
    assert!(r["overrides"].get("tools.select.move").is_none(), "{}", r["overrides"]);
    // A command taking V removes it from every tool that held it.
    let r = ks(&mut s, json!({"set": {"edit.copy": "V"}}));
    for t in ["tools.select.move", "tools.select.artboard", "tools.select.pencil"] {
        assert_eq!(r["overrides"][t], json!(""), "{t}: {}", r["overrides"]);
    }
}

#[test]
fn system_keys_are_refused_and_dropped_from_imports() {
    let mut s = Session::new();
    let key = if cfg!(target_os = "macos") { "Cmd+Tab" } else { "Alt+F4" };
    let before = s.prefs().clone();
    assert!(s.execute("edit.keyboardShortcuts", json!({"set": {"edit.copy": key}})).is_err());
    assert!(s.execute("edit.keyboardShortcuts", json!({"alternates": {"edit.copy": ["F9", key]}})).is_err());
    assert_eq!(s.prefs(), &before);
    let set = format!(
        "<photoshop-keyboard-shortcuts><command name=\"Copy\"><shortcut>F9</shortcut><shortcut>{key}</shortcut></command><command name=\"Cut\"><shortcut>{key}</shortcut></command><tool name=\"Pencil Tool\">N</tool><tool name=\"Move Tool\">V</tool></photoshop-keyboard-shortcuts>"
    );
    let r = ks(&mut s, json!({"importKys": set}));
    assert_eq!(r["overrides"]["edit.copy"], json!("F9"));
    assert!(r["overrides"].get("edit.cut").is_none(), "the reserved key is left out: {}", r["overrides"]);
    assert_eq!(r["import"]["reserved"].as_array().map(Vec::len), Some(1));
    assert_eq!(r["overrides"]["tools.select.pencil"], json!("N"), "tool keys import headless");
    assert!(r["overrides"].get("tools.select.move").is_none(), "Move's own V keeps its cycling");
}

#[test]
fn sets_read_and_write_files_and_export_everything() {
    let dir = std::env::temp_dir().join(format!("photocraft-shortcuts-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let kys = dir.join("mine.kys");
    let html = dir.join("mine.html");
    let mut s = Session::new();
    ks(&mut s, json!({"set": {"tools.select.pencil": "F6", "edit.copy": "F9"}, "saveSet": "Mine"}));
    let r = ks(&mut s, json!({"exportKys": {"path": kys.to_str().unwrap()}, "summarize": {"path": html.to_str().unwrap(), "mac": false}}));
    assert!(r.get("kys").is_none() && r["kysPath"].is_string() && r["htmlPath"].is_string(), "{r}");
    let text = std::fs::read_to_string(&kys).unwrap();
    assert!(text.contains("pc-id=\"tools.select.pencil\">F6</tool>") && text.contains("pc-id=\"view.zoomIn\""), "the whole set: {text}");
    assert!(std::fs::read_to_string(&html).unwrap().contains("<h2>Tools</h2>"));
    // Back to the defaults, then the file restores the set, tools included.
    ks(&mut s, json!({"useSet": ""}));
    let r = ks(&mut s, json!({"reset": true, "importKys": {"path": kys.to_str().unwrap()}}));
    assert_eq!((r["overrides"]["tools.select.pencil"].clone(), r["overrides"]["edit.copy"].clone()), (json!("F6"), json!("F9")));
    // Bad paths and shapes are errors, not panics.
    for p in [
        json!({"importKys": {"path": dir.join("missing.kys").to_str().unwrap()}}),
        json!({"importKys": {"path": ""}}),
        json!({"importKys": 5}),
        json!({"exportKys": "yes"}),
        json!({"exportKys": {"path": 3}}),
        json!({"summarize": {"path": ""}}),
    ] {
        assert!(s.execute("edit.keyboardShortcuts", p.clone()).is_err(), "{p}");
    }
    // A change paired with an export that fails (a folder that doesn't exist, a bad summarize
    // value) changes nothing.
    let before = ks(&mut s, json!({}))["overrides"].clone();
    for p in [
        json!({"set": {"edit.copy": "F10"}, "exportKys": {"path": dir.join("no").join("such").join("x.kys").to_str().unwrap()}}),
        json!({"set": {"edit.copy": "F10"}, "summarize": 7}),
        json!({"set": {"edit.copy": "F10"}, "saveSet": "Other", "summarize": {"path": dir.join("no").join("x.html").to_str().unwrap()}}),
        // A good .kys path with a bad summarize: neither file is written.
        json!({"set": {"edit.copy": "F10"}, "exportKys": {"path": dir.join("both.kys").to_str().unwrap()}, "summarize": {"path": dir.join("no").join("x.html").to_str().unwrap()}}),
        json!({"exportKys": {"path": dir.join("both.kys").to_str().unwrap()}, "summarize": 7}),
        json!({"exportKys": {"path": dir.join("both.kys").to_str().unwrap()}, "summarize": {"path": dir.to_str().unwrap()}}),
    ] {
        assert!(s.execute("edit.keyboardShortcuts", p.clone()).is_err(), "{p}");
        let r = ks(&mut s, json!({}));
        assert_eq!(r["overrides"], before, "{p}");
        assert!(r["sets"].get("Other").is_none(), "{p}: {r}");
        assert!(!dir.join("both.kys").exists(), "{p}: no file written");
    }
    // A second file that passes the checks but can't be written (a NUL in its name): the user's
    // existing first file is left exactly as it was, and no staged file stays behind.
    let mine = dir.join("existing.kys");
    std::fs::write(&mine, "the user's own file").unwrap();
    let html = format!("{}/a\u{0}.html", dir.to_str().unwrap());
    let p = json!({"exportKys": {"path": mine.to_str().unwrap()}, "summarize": {"path": html}});
    assert!(s.execute("edit.keyboardShortcuts", p.clone()).is_err(), "{p}");
    assert_eq!(std::fs::read_to_string(&mine).unwrap(), "the user's own file");
    let left: Vec<_> =
        std::fs::read_dir(&dir).unwrap().filter_map(|e| e.ok()).map(|e| e.file_name()).filter(|n| n.to_string_lossy().contains("photocraft-tmp")).collect();
    assert!(left.is_empty(), "{left:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_read_only_target_is_refused_before_anything_is_written() {
    let dir = std::env::temp_dir().join(format!("photocraft-readonly-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let ro = dir.join("locked.kys");
    std::fs::write(&ro, "locked").unwrap();
    let mut perms = std::fs::metadata(&ro).unwrap().permissions();
    perms.set_readonly(true);
    std::fs::set_permissions(&ro, perms.clone()).unwrap();
    let mut s = Session::new();
    let r = s.execute("edit.keyboardShortcuts", json!({"set": {"edit.copy": "F10"}, "exportKys": {"path": ro.to_str().unwrap()}}));
    assert!(r.as_ref().is_err_and(|e| e.to_string().contains("read-only")), "{r:?}");
    assert_eq!(std::fs::read_to_string(&ro).unwrap(), "locked");
    #[allow(clippy::permissions_set_readonly_false)]
    perms.set_readonly(false);
    let _ = std::fs::set_permissions(&ro, perms);
    let _ = std::fs::remove_dir_all(&dir);
}
