//! #11398: Edit › Preferences says "Plugins…" and "RAW Defaults…" and has the reference
//! application's two separators (after the first entry and before Camera Raw…).

use crate::menu_catalog::CATALOG;

/// The submenu's rows, top to bottom: (label or "---", command id).
fn rows() -> Vec<(&'static str, &'static str)> {
    CATALOG.iter().filter(|e| e.0 == ["Edit", "Preferences"]).map(|e| (e.1, e.3)).collect()
}

#[test]
fn preferences_submenu_reads_in_the_reference_order_with_two_separators() {
    let labels: Vec<&str> = rows().into_iter().map(|r| r.0).collect();
    // Row 1 ("Settings…") is #8239's: whatever it says, it is first and followed by a separator.
    assert_eq!(
        &labels[1..],
        [
            "---",
            "AI Integrations…",
            "Interface…",
            "Workspace…",
            "Tools…",
            "History Log…",
            "File Handling…",
            "Export…",
            "Performance…",
            "Scratch Disks…",
            "Cursors…",
            "Transparency & Gamut…",
            "Units & Rulers…",
            "Guides, Grid & Slices…",
            "Plugins…",
            "Type…",
            "Enhanced Controls…",
            "---",
            "RAW Defaults…",
        ]
    );
}

#[test]
fn preferences_submenu_ids_are_unchanged() {
    let ids: Vec<&str> = rows().into_iter().map(|r| r.1).collect();
    assert_eq!(
        ids,
        [
            "edit.preferences.general",
            "---",
            "edit.preferences.integrations",
            "edit.preferences.interface",
            "edit.preferences.workspace",
            "edit.preferences.tools",
            "edit.preferences.historyLog",
            "edit.preferences.fileHandling",
            "edit.preferences.export",
            "edit.preferences.performance",
            "edit.preferences.scratchDisks",
            "edit.preferences.cursors",
            "edit.preferences.transparencyAndGamut",
            "edit.preferences.unitsAndRulers",
            "edit.preferences.guidesGridAndSlices",
            "edit.preferences.plugIns",
            "edit.preferences.type",
            "edit.preferences.enhancedControls",
            "---",
            "edit.preferences.rawDefaults",
        ]
    );
}

#[test]
fn engine_labels_match_the_catalogue_for_the_renamed_rows() {
    for id in ["edit.preferences.plugIns", "edit.preferences.rawDefaults"] {
        let catalogue = CATALOG.iter().find(|e| e.3 == id).unwrap_or_else(|| panic!("{id} missing from the catalogue")).1;
        let engine = photocraft_engine::commands::find(id).unwrap_or_else(|| panic!("{id} missing from the engine")).label;
        assert_eq!(engine, catalogue, "{id}");
    }
    assert_eq!(photocraft_engine::commands::find("edit.preferences.plugIns").unwrap().label, "Plugins…");
    assert_eq!(photocraft_engine::commands::find("edit.preferences.rawDefaults").unwrap().label, "RAW Defaults…");
}

#[test]
fn preferences_dialog_page_names_follow_the_menu() {
    let title = |key: &str| photocraft_engine::prefs::SECTIONS.iter().find(|(k, _)| *k == key).map(|(_, t)| *t);
    assert_eq!(title("plugIns"), Some("Plugins"));
    assert_eq!(title("rawDefaults"), Some("RAW Defaults"));
}
