//! Layer > New > Artboard from Group runs at once, so its menu entry and command carry no ellipsis
//! (#14548).

use serde_json::json;

use crate::PhotocraftApp;
use crate::i18n::{Lang, tr_id};
use crate::menu_catalog::CATALOG;

const ID: &str = "layer.new.artboardFromGroup";
const LABEL: &str = "Artboard from Group";

#[test]
fn the_menu_entry_has_no_ellipsis_and_sits_between_artboard_and_artboard_from_layers() {
    let rows: Vec<&(&[&str], &str, Option<&str>, &str)> = CATALOG.iter().filter(|r| r.0 == ["Layer", "New"]).collect();
    let at = rows.iter().position(|r| r.3 == ID).expect("the catalog row");
    assert_eq!(rows[at].1, LABEL);
    assert_eq!(rows[at].2, None, "no shortcut");
    assert_eq!(rows[at - 1].3, "layer.new.artboard");
    assert_eq!(rows[at - 1].1, "Artboard…");
    assert_eq!(rows[at + 1].3, "layer.new.artboardFromLayers");
    assert_eq!(rows[at + 1].1, "Artboard from Layers…");
}

#[test]
fn the_command_label_has_no_ellipsis() {
    let specs = photocraft_engine::cmd::command_specs();
    let spec = specs.iter().find(|s| s.id == ID).expect("command spec");
    assert_eq!(spec.label, LABEL);
}

#[test]
fn no_language_shows_an_ellipsis() {
    // English shows the label itself. Where craft-i18n's catalogs are embedded (CI and releases),
    // most languages already translate "Artboard from Group" (Japanese: グループからアートボード),
    // and `tr_id` skips an `@id` row whose trailing `…` differs from the label's; a language
    // without a row shows the English. Either way no language shows an ellipsis.
    assert_eq!(tr_id(Lang::EN, ID, LABEL), LABEL);
    for lang in Lang::all() {
        let shown = tr_id(lang, ID, LABEL);
        assert!(!shown.is_empty(), "{lang:?}");
        assert!(!shown.ends_with('…') && !shown.ends_with("..."), "{lang:?}: {shown}");
    }
}

#[test]
fn running_the_menu_command_opens_no_dialog() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 200, "height": 100, "background": "white"})).unwrap();
    app.run("layer.new.group", json!({})).unwrap();
    let ctx = egui::Context::default();
    let dialogs = app.ui.dialogs.len();
    let layers = app.session.active().unwrap().doc.layers.len();
    crate::menus::invoke(&mut app, &ctx, ID, json!({})).expect("the command runs");
    assert_eq!(app.ui.dialogs.len(), dialogs, "no dialog opens");
    assert_eq!(app.session.active().unwrap().doc.layers.len(), layers, "the group became an artboard in place");
}
