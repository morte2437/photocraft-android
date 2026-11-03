//! Menu bar generated from the engine command registry plus UI-level commands.

use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::state::DialogKind;

mod bar;
pub mod enable_rules;
mod invoke;
mod items;
pub mod menu_keys;
pub mod menu_nav;
pub mod native_menu;
pub mod parity;
pub mod plugin_ui;
mod render;
mod search;
mod state;
pub mod view_cmds;
pub mod workspace_ui;

pub use bar::menu_bar;
#[cfg(test)]
use bar::pointer_reaches_title;
pub use invoke::{apply_workspace, invoke};
pub(crate) use invoke::{invoke_unguarded, panel_alias};
pub(crate) use items::ItemCache;
pub use items::{MenuItem, is_live, menu_items};
pub use render::alt_click;
#[cfg(test)]
use render::submenu_enabled;
#[cfg(test)]
use search::HELP_SEARCH_MAX;
pub use search::{search_items, search_rank};
pub use state::is_enabled;
pub(crate) use state::reveal_label;
#[cfg(test)]
use state::{checked, translated_menu_label};

/// Shared gate for native menu clicks and control-channel `ui.menu.invoke`.
/// Dialogs and unsaved-changes prompts block edits, but allow the four view-navigation
/// commands also usable through shortcuts. Camera Raw, the SVG editor choice and full-window workspaces
/// (`distort_ui::modal_workspace`) block menu commands entirely, and a pending crop blocks the
/// commands Photoshop greys during one (`crop_ui::blocks`).
pub(crate) fn modal_allows(app: &PhotocraftApp, id: &str) -> bool {
    if app.camera_raw.is_some() || app.smart_svg.modal_open() || crate::distort_ui::modal_workspace(app).is_some() || crate::crop_ui::blocks(app, id) {
        return false;
    }
    (app.ui.dialogs.is_empty() && app.ui.layers_panel_dialog.is_none() && app.discard.is_none()) || crate::shortcuts::NAV_COMMANDS.contains(&id)
}

/// Top-level menus in Photoshop order.
pub const TOP_MENUS: [&str; 10] = ["File", "Edit", "Image", "Layer", "Type", "Select", "Filter", "View", "Window", "Help"];

/// UI-level commands (handled by the shell rather than the engine): id, label, menu, shortcut.
/// The table is the engine's ([`photocraft_engine::shell_commands::SHELL_COMMANDS`]), so the
/// commands and their keys exist without this UI (headless shortcut sets, lists, conflicts).
pub const UI_COMMANDS: &[(&str, &str, &[&str], Option<&str>)] = photocraft_engine::shell_commands::SHELL_COMMANDS;

/// Window › <document>: the open documents, listed at the end of the Window menu as in Photoshop.
/// The id ends in the document's index, which is its tab's position.
pub(crate) const DOCUMENT_ITEM: &str = "window.document.";

#[cfg(test)]
mod tests;

#[cfg(test)]
mod open_recent_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn tracks_dedupes_caps_and_lists() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.push_recent("/tmp/a.png");
        app.push_recent("/tmp/b.psd");
        app.push_recent("/tmp/a.png"); // de-dupe → moves to front
        assert_eq!(app.ui.recent_files, vec!["/tmp/a.png".to_string(), "/tmp/b.psd".to_string()]);
        for i in 0..15 {
            app.push_recent(&format!("/tmp/f{i}.png"));
        }
        assert_eq!(app.ui.recent_files.len(), 17, "all kept below the default cap of 20");
        for i in 15..30 {
            app.push_recent(&format!("/tmp/f{i}.png"));
        }
        assert_eq!(app.ui.recent_files.len(), 20, "capped at Recent File List Contains (20)");
        assert_eq!(app.session.prefs().file_handling.recent_files, app.ui.recent_files, "stored in the preferences");

        // Menu lists them under File › Open Recent, with basenames, plus Clear Recent Files.
        app.ui.recent_files = vec!["/tmp/a.png".into(), "/dir/b.psd".into()];
        let items = menu_items(&app);
        let rp = vec!["File".to_string(), "Open Recent".to_string()];
        let labels: Vec<&str> = items.iter().filter(|i| i.id.starts_with("file.openRecent.")).map(|i| i.label.as_str()).collect();
        assert_eq!(labels, vec!["a.png", "b.psd"]);
        assert!(items.iter().any(|i| i.id == "file.clearRecent" && i.path == rp));

        // Clear Recent Files empties the list.
        let ctx = egui::Context::default();
        invoke(&mut app, &ctx, "file.clearRecent", json!({})).unwrap();
        assert!(app.ui.recent_files.is_empty());
        assert!(app.session.prefs().file_handling.recent_files.is_empty(), "cleared in the preferences too");
        // No recent items in the menu once cleared (only the disabled "Clear Recent Files").
        assert!(!menu_items(&app).iter().any(|i| i.id.starts_with("file.openRecent.")));
        // A bad recent index errors gracefully (no panic).
        assert!(invoke(&mut app, &ctx, "file.openRecent.5", json!({})).is_err());

        // Remove one entry (the Home screen's ×, #2691): the others keep their order.
        for p in ["/tmp/c.tif", "/tmp/b.psd", "/tmp/a.png"] {
            app.push_recent(p);
        }
        invoke(&mut app, &ctx, "file.removeRecent", json!({"path": "/tmp/b.psd"})).unwrap();
        assert_eq!(app.ui.recent_files, vec!["/tmp/a.png".to_string(), "/tmp/c.tif".to_string()]);
        assert_eq!(app.session.prefs().file_handling.recent_files, app.ui.recent_files, "removed in the preferences too");
        invoke(&mut app, &ctx, "file.removeRecent", json!({"path": "/tmp/not-listed.png"})).unwrap();
        assert_eq!(app.ui.recent_files.len(), 2, "a path that isn't listed changes nothing");
        assert!(invoke(&mut app, &ctx, "file.removeRecent", json!({})).is_err(), "needs a path");
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn opening_a_file_records_it_as_recent() {
        use photocraft_color::{ColorMode, SampleType};
        use photocraft_geom::Size;
        let services = crate::Services {
            import: Some(Box::new(|_n: &str, _b: &[u8], _opts: &photocraft_io::ImportOptions| {
                Ok((photocraft_doc::Document::new("t", Size::new(4, 4), ColorMode::Rgb, SampleType::U8), Vec::new()))
            })),
            ..Default::default()
        };
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
        let path = std::env::temp_dir().join("photocraft_recent_test.pcraft");
        std::fs::write(&path, b"x").unwrap();
        let p = path.to_string_lossy().to_string();
        let ctx = egui::Context::default();
        invoke(&mut app, &ctx, "file.open", json!({ "path": p })).unwrap();
        assert_eq!(app.ui.recent_files.first().map(String::as_str), Some(p.as_str()));
        // Re-open via the recent entry once closed (an open file shows its tab instead, #6598).
        app.session.close(0);
        invoke(&mut app, &ctx, "file.openRecent.0", json!({})).unwrap();
        assert_eq!(app.session.documents().len(), 1);
        assert_eq!(app.session.active().and_then(|d| d.path.as_deref()), Some(p.as_str()));
        let _ = std::fs::remove_file(&path);
    }
}

#[cfg(test)]
mod quick_export_label_tests {
    use super::*;

    #[test]
    fn file_quick_export_menu_reflects_selected_format_without_renaming_layer_export() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        for (value, expected) in [("png", "PNG"), ("jpg", "JPG"), ("gif", "GIF"), ("webp", "WebP"), ("exr", "EXR")] {
            app.run("prefs.set", json!({"values": {"export.quickExportFormat": value}})).unwrap();
            let items = menu_items(&app);
            let quick: Vec<_> = items.iter().filter(|i| i.id == "file.export.quickExportAsPng").collect();
            assert_eq!(quick.len(), 1, "the catalogue and UI command must not create duplicate items");
            let shown_format = if cfg!(target_arch = "wasm32") { "PNG" } else { expected };
            assert_eq!(quick[0].label, format!("Quick Export as {shown_format}"));
            let id = crate::i18n::Lang::from_code("id").unwrap();
            let localized = translated_menu_label(id, quick[0]);
            assert_eq!(localized, crate::i18n::tr_id(id, "file.export.quickExportAsPng", "Quick Export as PNG").replace("PNG", shown_format));
            let layer = items.iter().find(|i| i.id == "layer.quickExportAsPng").unwrap();
            assert_eq!(layer.label, "Quick Export as PNG", "layer export always creates PNG");
        }
    }
}

#[cfg(test)]
mod reveal_label_tests {
    use super::*;

    #[test]
    fn reveal_label_names_the_platform_file_manager() {
        let expected = if cfg!(target_os = "macos") {
            "Reveal in Finder"
        } else if cfg!(windows) {
            "Show in Explorer"
        } else {
            "Show in Folder"
        };
        assert_eq!(reveal_label(), expected);
    }

    #[test]
    fn smart_objects_reveal_item_names_the_platform_file_manager() {
        let app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        let id = "layer.smartObjects.revealInFinder";
        let items = menu_items(&app);
        let reveal: Vec<_> = items.iter().filter(|i| i.id == id).collect();
        assert_eq!(reveal.len(), 1);
        assert_eq!(reveal[0].path, ["Layer", "Smart Objects"]);
        assert_eq!(reveal[0].label, reveal_label());
        if !cfg!(target_os = "macos") {
            assert!(!reveal[0].label.contains("Finder"), "{}", reveal[0].label);
        }
        let shortcuts = crate::prefs_ui::shortcut_items(&app);
        assert!(shortcuts.iter().any(|(i, label, _, _)| i == id && label == reveal_label()), "the shortcut list shows the menu label");
        let de = crate::i18n::Lang::from_code("de").unwrap();
        assert_eq!(translated_menu_label(de, reveal[0]), crate::i18n::tr(de, reveal_label()));
        // The menu catalogue (and docs/parity-checklist.md) keeps Photoshop's macOS name.
        assert!(crate::menu_catalog::CATALOG.iter().any(|(_, label, _, i)| *i == id && *label == "Reveal in Finder"));
    }
}

#[cfg(test)]
mod window_documents_tests {
    use super::*;
    use serde_json::json;

    fn app(docs: usize) -> PhotocraftApp {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        for _ in 0..docs {
            app.run("file.new", json!({"width": 64, "height": 48})).unwrap();
        }
        app.sync_views();
        app
    }

    /// The rows after Window's last own item: a separator and the documents.
    fn listed(app: &PhotocraftApp) -> Vec<(String, String, Option<bool>)> {
        let items = menu_items(app);
        let window: Vec<&MenuItem> = items.iter().filter(|i| i.path == ["Window"]).collect();
        let at = window.iter().position(|i| i.id.starts_with(DOCUMENT_ITEM)).unwrap_or(window.len());
        if at > 0 && at < window.len() {
            assert_eq!(window[at - 1].label, "---", "a separator comes before the documents");
        }
        window[at..].iter().map(|i| (i.id.clone(), i.label.clone(), i.checked)).collect()
    }

    /// #2991: Window ends with the open documents in tab order, the active one checked, as in
    /// Photoshop; choosing one brings it to the front.
    #[test]
    fn the_window_menu_lists_the_open_documents_and_switches_between_them() {
        assert!(listed(&app(0)).is_empty(), "no documents, no list (and no separator)");

        let mut app = app(3);
        let ctx = egui::Context::default();
        let names: Vec<String> = app.session.documents().iter().map(|d| d.doc.name.clone()).collect();
        let row = |i: usize, active: bool| (format!("window.document.{i}"), names[i].clone(), Some(active));
        assert_eq!(listed(&app), [row(0, false), row(1, false), row(2, true)]);
        // It stays last in Window, after the panels.
        let items = menu_items(&app);
        let last = items.iter().rposition(|i| i.path.first().map(String::as_str) == Some("Window")).unwrap();
        assert_eq!(items[last].id, "window.document.2");

        assert_eq!(invoke(&mut app, &ctx, "window.document.0", json!({})).unwrap(), json!({"document": 0}));
        assert_eq!(app.session.active_index(), Some(0));
        assert_eq!(listed(&app), [row(0, true), row(1, false), row(2, false)]);

        // A stale or malformed id (a document closed since the menu was built) is an error.
        for id in ["window.document.3", "window.document.", "window.document.x", "window.document.-1"] {
            assert!(is_enabled(&app, id), "{id}: rows are always enabled");
            assert!(invoke(&mut app, &ctx, id, json!({})).is_err(), "{id}");
        }
        assert_eq!(app.session.active_index(), Some(0));
    }

    /// The rows show the document's name as it is, never a translation of it, and follow a
    /// rename (Save As).
    #[test]
    fn document_names_are_shown_as_they_are_and_follow_a_rename() {
        let mut app = app(1);
        let before = crate::native_menu::state_hash(&app, None);
        if let Some(d) = app.session.active_mut() {
            d.saved_to("/tmp/Layers".into());
        }
        assert_ne!(crate::native_menu::state_hash(&app, None), before, "a rename updates the native menu");
        let de = crate::i18n::Lang::from_code("de").unwrap();
        let items = menu_items(&app);
        let row = items.iter().find(|i| i.id == "window.document.0").unwrap();
        assert_eq!(translated_menu_label(de, row), "Layers");
        let bar = crate::native_menu::photocraft_layout(&items, de, "de").bar;
        assert_eq!(bar.find("window.document.0").map(|i| i.label.as_str()), Some("Layers"));
        let panel = items.iter().find(|i| i.path == ["Window"] && i.label == "Layers").unwrap();
        assert_eq!(bar.find(&panel.id).map(|i| i.label.clone()), Some(translated_menu_label(de, panel)), "Window › Layers is still translated");
    }

    /// The rows go through the control channel's `ui.menu.invoke`, like a click.
    #[test]
    fn control_can_choose_a_document_from_the_window_menu() {
        let mut app = app(2);
        let ctx = egui::Context::default();
        let (req, _rx) = crate::control::ControlRequest::new("ui.menu.invoke", json!({"id": "window.document.0"}));
        let crate::control::Outcome::Done(r) = crate::control::handle(&mut app, &ctx, &req) else { panic!("expected an immediate reply") };
        assert_eq!(r["ok"], true, "{r}");
        assert_eq!(app.session.active_index(), Some(0));
    }
}
