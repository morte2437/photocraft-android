//! "Layer Via Copy" and "Layer Via Cut" carry a capital V in every menu, as in the reference
//! application (#12357), and every translation moved with the key.

use crate::canvas::canvas_tool_menu::SELECTION_MENU;
use crate::menu_catalog::CATALOG;

const NEW: [(&str, &str); 2] = [("Layer Via Copy", "layer.new.layerViaCopy"), ("Layer Via Cut", "layer.new.layerViaCut")];
const OLD: [&str; 2] = ["Layer via Copy", "Layer via Cut"];

#[test]
fn menu_bar_rows_use_the_capital_v() {
    for (label, id) in NEW {
        let row = CATALOG.iter().find(|r| r.3 == id).expect("catalog row");
        assert_eq!(row.1, label);
    }
    assert!(CATALOG.iter().all(|r| !OLD.contains(&r.1)));
}

#[test]
fn command_labels_use_the_capital_v() {
    let specs = photocraft_engine::cmd::command_specs();
    for (label, id) in NEW {
        let spec = specs.iter().find(|s| s.id == id).expect("command spec");
        assert_eq!(spec.label, label);
    }
}

#[test]
fn selection_tools_canvas_menu_uses_the_capital_v() {
    for (label, id) in NEW {
        assert!(SELECTION_MENU.iter().flatten().any(|r| *r == (label, id)), "{label} row");
    }
    assert!(SELECTION_MENU.iter().flatten().all(|r| !OLD.contains(&r.0)));
}

#[test]
fn every_locale_file_keeps_its_translation_under_the_new_keys() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/i18n");
    let mut checked = 0;
    for entry in std::fs::read_dir(dir).expect("i18n dir") {
        let path = entry.expect("entry").path();
        if path.extension().and_then(|e| e.to_str()) != Some("tsv") {
            continue;
        }
        let text = std::fs::read_to_string(&path).expect("tsv");
        let keys: Vec<&str> = text.lines().filter_map(|l| l.split('\t').nth(1)).collect();
        let name = path.display();
        assert!(OLD.iter().all(|o| !keys.contains(o)), "{name} still has an old key");
        // Hindi has no entries for these menus yet; every other file has both.
        if keys.contains(&"Layer Via Copy") || keys.contains(&"Layer Via Cut") {
            assert!(NEW.iter().all(|(n, _)| keys.contains(n)), "{name} lacks a new key");
            checked += 1;
        }
    }
    assert!(checked >= 30, "only {checked} locale files carry the keys");
}

#[test]
fn a_translation_is_found_under_the_new_key() {
    let de = crate::i18n::Lang::from_code("de").expect("German");
    assert_eq!(crate::i18n::tr(de, "Layer Via Copy"), "Ebene durch Kopieren");
    assert_eq!(crate::i18n::tr(de, "Layer Via Cut"), "Ebene durch Ausschneiden");
}
