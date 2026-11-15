//! Layer > Smart Objects menu rows, labels and order (#11401).

use serde_json::json;

use crate::PhotocraftApp;
use crate::menus::{MenuItem, menu_items};

fn app() -> PhotocraftApp {
    PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default())
}

fn submenu(app: &PhotocraftApp) -> Vec<MenuItem> {
    menu_items(app).into_iter().filter(|i| i.path == ["Layer", "Smart Objects"]).collect()
}

fn reveal() -> &'static str {
    if cfg!(target_os = "macos") {
        "Reveal in Finder"
    } else if cfg!(windows) {
        "Reveal in Explorer"
    } else {
        "Show in Folder"
    }
}

#[test]
fn the_rows_follow_the_reference_order() {
    let labels: Vec<String> = submenu(&app()).into_iter().map(|i| i.label).collect();
    let expected = [
        "Convert to Smart Object",
        "New Smart Object via Copy",
        "---",
        reveal(),
        "---",
        "Update Modified Content",
        "Update All Modified Content",
        "---",
        "Edit Contents",
        "Reset Transform",
        "Relink to File…",
        "Replace Contents…",
        "Export Contents…",
        "---",
        "Embed Linked",
        "Embed All Linked",
        "Convert to Linked…",
        "Convert to Layers",
        "---",
        "---",
        "Rasterize",
    ];
    assert_eq!(labels, expected, "{labels:?}");
}

#[test]
fn embed_rows_use_the_new_labels_and_the_ids_stay() {
    let items = submenu(&app());
    let by_id = |id: &str| items.iter().find(|i| i.id == id).unwrap_or_else(|| panic!("{id} missing"));
    assert_eq!(by_id("layer.smartObjects.convertToEmbedded").label, "Embed Linked");
    assert_eq!(by_id("layer.smartObjects.embedAllLinked").label, "Embed All Linked");
    assert_eq!(by_id("layer.smartObjects.revealInFinder").label, reveal());
}

#[cfg(windows)]
#[test]
fn windows_says_reveal_in_explorer() {
    assert_eq!(crate::menus::reveal_label(), "Reveal in Explorer");
}

#[test]
fn the_warp_rows_are_gone_from_the_submenu_but_the_commands_stay() {
    let app = app();
    for id in ["layer.smartObjects.warp", "layer.smartObjects.perspectiveWarp", "layer.smartObjects.puppetWarp"] {
        assert!(!submenu(&app).iter().any(|i| i.id == id), "{id}");
        assert!(photocraft_engine::cmd::command_specs().iter().any(|c| c.id == id), "{id} stays registered");
    }
    let all = menu_items(&app);
    for id in ["edit.puppetWarp", "edit.perspectiveWarp", "edit.transform.warp"] {
        assert!(all.iter().any(|i| i.id == id), "{id} stays in the Edit menu");
    }
}

#[test]
fn embed_all_linked_is_disabled_without_a_linked_smart_object_and_enabled_with_one() {
    let mut app = app();
    app.run("file.new", json!({"width": 32, "height": 24})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app.session
        .edit("setup", |doc, active| {
            doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap().fill_rect(photocraft_geom::Rect::new(2, 2, 10, 10), &[1.0, 0.0, 0.0, 1.0]);
            Ok(())
        })
        .unwrap();
    app.run("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
    app.sync_views();
    let enabled = |app: &PhotocraftApp, id: &str| submenu(app).iter().find(|i| i.id == id).unwrap().enabled;
    assert!(!enabled(&app, "layer.smartObjects.embedAllLinked"));
    let path = std::env::temp_dir().join(format!("pc-so-menu-{}.pcraft", std::process::id()));
    app.run("layer.smartObjects.convertToLinked", json!({"path": path.to_str().unwrap()})).unwrap();
    app.sync_views();
    assert!(enabled(&app, "layer.smartObjects.embedAllLinked"));
    assert!(enabled(&app, "layer.smartObjects.convertToEmbedded"));
    std::fs::remove_file(&path).ok();
}
