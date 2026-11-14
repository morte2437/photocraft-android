//! Edit › Transform's split warp entries follow the Warp box (#12494): greyed with none open,
//! enabled in one. A warped smart object keeps them, and several layers, a link group and
//! editable type still grey them.

use photocraft_doc::{LayerContent, LayerId};
use serde_json::json;

use crate::PhotocraftApp;

const SPLITS: [&str; 3] = ["edit.transform.splitWarpCrosswise", "edit.transform.splitWarpHorizontally", "edit.transform.splitWarpVertically"];
const REMOVE: &str = "edit.transform.removeWarpSplit";

fn four() -> impl Iterator<Item = &'static str> {
    SPLITS.into_iter().chain([REMOVE])
}

/// A 100 × 100 document with a pixel layer "Paint" holding a red block, active.
fn app() -> (PhotocraftApp, LayerId) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 100, "height": 100, "background": "white"})).unwrap();
    app.sync_views();
    let paint = LayerId(app.run("layer.new.layer", json!({"name": "Paint"})).unwrap()["layer"].as_u64().unwrap());
    app.run("select.rect", json!({"x": 10, "y": 10, "width": 50, "height": 30})).unwrap();
    app.run("edit.fill", json!({"contents": "color", "color": "#ff0000"})).unwrap();
    app.run("select.deselect", json!({})).unwrap();
    (app, paint)
}

/// The Edit › Transform row's state as the menu builds it.
fn enabled(app: &PhotocraftApp, id: &str) -> bool {
    let row = crate::menus::menu_items(app).into_iter().find(|i| i.path == ["Edit", "Transform"] && i.id == id);
    row.unwrap_or_else(|| panic!("{id} is in Edit › Transform")).enabled
}

fn choose(app: &mut PhotocraftApp, id: &str) -> Result<serde_json::Value, String> {
    crate::menus::invoke(app, &egui::Context::default(), id, json!({}))
}

fn warped_smart_object(app: &mut PhotocraftApp) {
    app.run("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
    app.run("layer.smartObjects.warp", json!({"style": "arc", "bend": 30, "rect": [10, 10, 60, 40]})).unwrap();
    assert!(crate::transform_tool::warps_active_layer(app), "the smart object has a stored warp");
}

fn stored_warp(app: &PhotocraftApp) -> Option<photocraft_geom::warp::Warp> {
    let st = app.session.active().unwrap();
    match &st.doc.layer(st.active_layer.unwrap()).unwrap().content {
        LayerContent::Smart(sm) => sm.warp.clone(),
        _ => None,
    }
}

#[test]
fn the_split_entries_are_greyed_until_warp_opens() {
    let (mut app, _) = app();
    for id in four() {
        assert!(!enabled(&app, id), "{id} is greyed with no Warp box");
    }
    choose(&mut app, "edit.transform.warp").unwrap();
    assert!(app.ui.transform.as_ref().is_some_and(|t| t.warp.is_some()), "Warp is open");
    for id in SPLITS {
        assert!(enabled(&app, id), "{id} is enabled in Warp");
    }
    assert!(!enabled(&app, REMOVE), "a fresh grid has no split line to remove");
    let before = app.ui.transform.as_ref().and_then(|t| t.warp.clone());
    choose(&mut app, "edit.transform.splitWarpCrosswise").unwrap();
    assert_ne!(app.ui.transform.as_ref().and_then(|t| t.warp.clone()), before, "the menu's split cuts the open grid");
    assert!(enabled(&app, REMOVE), "the new split line can be removed");
    crate::transform_tool::cancel(&mut app);
    assert!(app.ui.transform.is_none());
    for id in four() {
        assert!(!enabled(&app, id), "{id} is greyed again once Warp closes");
    }
}

/// Only the menu greys: the commands with the warp passed inline still run without a Warp box.
#[test]
fn inline_warp_commands_run_without_a_warp_box() {
    let (mut app, _) = app();
    let warp = json!({"style": "bulge", "bend": 30.0, "bounds": [10.0, 10.0, 60.0, 40.0]});
    let at = json!([30.0, 20.0]);
    for id in ["edit.transform.splitWarpHorizontally", "edit.transform.splitWarpVertically"] {
        let r = app.run(id, json!({"warp": warp, "at": at})).unwrap();
        assert!(r["warp"]["mesh"].is_object(), "{id} returns the split mesh");
    }
    // The control channel's `ui.menu.invoke` path.
    let ctx = egui::Context::default();
    let split = crate::menus::invoke(&mut app, &ctx, "edit.transform.splitWarpCrosswise", json!({"warp": warp, "at": at})).unwrap()["warp"].clone();
    assert!(split["mesh"].is_object(), "Split Warp Crosswise returns the split mesh");
    let r = crate::menus::invoke(&mut app, &ctx, REMOVE, json!({"warp": split, "at": at})).unwrap();
    assert!(r["warp"]["mesh"].is_object(), "Remove Warp Split returns the mesh");
}

#[test]
fn a_warped_smart_object_keeps_the_split_entries() {
    let (mut app, _) = app();
    warped_smart_object(&mut app);
    for id in four() {
        assert!(enabled(&app, id), "{id} stays enabled on a warped smart object");
    }
    let before = stored_warp(&app);
    choose(&mut app, "edit.transform.splitWarpCrosswise").unwrap();
    assert!(app.ui.transform.is_none(), "the split edits the stored warp, without a box");
    assert_ne!(stored_warp(&app), before, "the smart object's stored warp is split");
}

#[test]
fn several_layers_a_link_group_and_type_still_grey_them() {
    let (mut app, paint) = app();
    let other = LayerId(app.run("layer.new.layer", json!({"name": "Other"})).unwrap()["layer"].as_u64().unwrap());
    app.run("layer.select", json!({"layer": paint.0})).unwrap();
    warped_smart_object(&mut app);
    let smart = app.session.active().unwrap().active_layer.unwrap();
    app.run("layer.select", json!({"layer": other.0, "mode": "add"})).unwrap();
    for id in four() {
        assert!(!enabled(&app, id), "{id} is greyed for several layers");
    }
    assert_eq!(app.run("layer.linkLayers", json!({})).unwrap()["linked"], true);
    app.run("layer.select", json!({"layer": smart.0})).unwrap();
    for id in four() {
        assert!(!enabled(&app, id), "{id} is greyed for a layer of a link group");
    }
    app.run("layer.linkLayers", json!({"linked": false})).unwrap();
    for id in four() {
        assert!(enabled(&app, id), "{id} is back once the layer is unlinked");
    }

    let text = LayerId(app.run("type.create", json!({"x": 20, "y": 80, "text": "Hi", "size": 20})).unwrap()["layer"].as_u64().unwrap());
    app.run("layer.select", json!({"layer": text.0})).unwrap();
    choose(&mut app, "edit.transform.warp").unwrap();
    assert!(app.ui.transform.as_ref().is_some_and(|t| t.warp.is_some()), "Warp is open on the type");
    for id in four() {
        assert!(!enabled(&app, id), "{id} is greyed for editable type in Warp");
    }
}
