//! #12577: a right-click on the canvas with the Red Eye tool opens the layer menu, or the
//! selection menu with the pointer inside a selection, as measured in the reference application on
//! a plain one-layer RGB document. The rows grey out like their menu-bar twins and run their
//! commands; the right-click itself corrects no red eye and adds no history state. The cases that
//! weren't measured keep opening nothing.

use egui::Modifiers;
use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::canvas_tool_menu::count_tests::{call, listed, right_click};
use crate::canvas_tool_menu::{entry_enabled, rows};
use crate::state::Tool;

/// A 64 × 48 RGB document with its one Background layer red, after a cleared selection (so
/// Reselect has one to restore). The Red Eye tool is the tool.
fn app() -> (PhotocraftApp, egui::Context) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 64, "height": 48, "background": "white"})).unwrap();
    app.run("edit.fill", json!({"contents": "color", "color": "#d01818"})).unwrap();
    app.run("select.rect", json!({"x": 4, "y": 4, "width": 10, "height": 10})).unwrap();
    app.run("select.deselect", json!({})).unwrap();
    app.sync_views();
    app.ui.tool = Tool::RedEye;
    (app, egui::Context::default())
}

/// The open menu's rows as `ui.inspect` lists them: (label, enabled), "―" for separators.
fn entries(app: &mut PhotocraftApp, ctx: &egui::Context) -> Vec<(String, bool)> {
    let inspected = call(app, ctx, "ui.inspect", json!({}));
    let entries = inspected.pointer("/result/canvasToolMenu/entries").and_then(Value::as_array).cloned().unwrap_or_default();
    entries.iter().map(|e| (e["label"].as_str().unwrap_or("―").to_string(), e["enabled"] != false)).collect()
}

/// `want` as (label, enabled): the rows named in `grey` disabled, separators as "―".
fn expected(want: &[&str], grey: &[&str]) -> Vec<(String, bool)> {
    want.iter().map(|l| (l.to_string(), *l == "―" || !grey.contains(l))).collect()
}

/// The document's revision, history depth and journal length.
fn state(app: &PhotocraftApp) -> (u64, usize, usize) {
    let st = app.session.active().unwrap();
    (st.revision, st.history.past_len(), app.session.journal.len())
}

const NO_SELECTION: [&str; 17] = [
    "Blending Options…",
    "―",
    "Duplicate Layer…",
    "Delete Layer",
    "Group from Layers…",
    "―",
    "Convert to Smart Object",
    "Convert to Layers",
    "―",
    "Rasterize Layer",
    "Rasterize Layer Style",
    "―",
    "Free Transform",
    "―",
    "Color Range…",
    "Load Selection…",
    "Reselect",
];

/// Group from Layers… and Load Selection… follow their menu-bar twins, which grey them for the Background
/// (#12866) and for a lone Background with no saved selection (#13293), as the reference does.
const NO_SELECTION_GREY: [&str; 7] =
    ["Blending Options…", "Group from Layers…", "Convert to Layers", "Rasterize Layer", "Rasterize Layer Style", "Free Transform", "Load Selection…"];

const IN_SELECTION: [&str; 23] = [
    "Deselect",
    "Select Inverse",
    "Feather…",
    "Select and Mask…",
    "―",
    "Save Selection…",
    "Make Work Path…",
    "―",
    "Layer Via Copy",
    "Layer Via Cut",
    "New Layer…",
    "―",
    "Free Transform",
    "Transform Selection",
    "―",
    "Fill…",
    "Stroke…",
    "Content-Aware Fill…",
    "Delete and Fill Selection",
    "―",
    "Last Filter",
    "―",
    "Fade…",
];

fn select(app: &mut PhotocraftApp) {
    app.run("select.rect", json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap();
}

#[test]
fn without_a_selection_the_layer_menu_opens_in_order_with_the_reference_greys() {
    let (mut app, ctx) = app();
    right_click(&mut app, &ctx, 40.0, 30.0);
    assert_eq!(entries(&mut app, &ctx), expected(&NO_SELECTION, &NO_SELECTION_GREY));
}

#[test]
fn inside_a_selection_the_selection_menu_opens_in_order_with_the_reference_greys() {
    let (mut app, ctx) = app();
    select(&mut app);
    right_click(&mut app, &ctx, 20.0, 20.0);
    assert_eq!(entries(&mut app, &ctx), expected(&IN_SELECTION, &["Last Filter", "Fade…"]));
}

#[test]
fn rows_grey_out_like_their_menu_bar_twins() {
    for selection in [false, true] {
        let (mut app, ctx) = app();
        if selection {
            select(&mut app);
        }
        right_click(&mut app, &ctx, 20.0, 20.0);
        let menu = app.ui.canvas_tool_menu.clone().unwrap();
        for (_, id) in rows(&menu).iter().flatten() {
            assert_eq!(entry_enabled(&app, &menu, id), crate::menus::is_enabled(&app, id), "{id} (selection: {selection})");
        }
    }
}

#[test]
fn picking_a_row_runs_its_command() {
    let (mut app, ctx) = app();
    select(&mut app);
    right_click(&mut app, &ctx, 20.0, 20.0);
    assert_eq!(call(&mut app, &ctx, "ui.context.choose", json!({"id": "select.deselect"}))["ok"], true);
    assert!(app.session.active().unwrap().doc.selection.is_none(), "Deselect deselected");
    assert!(app.ui.canvas_tool_menu.is_none(), "the pick closes the menu");
    right_click(&mut app, &ctx, 20.0, 20.0);
    assert_eq!(call(&mut app, &ctx, "ui.context.choose", json!({"id": "select.reselect"}))["ok"], true);
    assert!(app.session.active().unwrap().doc.selection.is_some(), "Reselect brought it back");
    // A row the open menu doesn't have is refused.
    right_click(&mut app, &ctx, 20.0, 20.0);
    assert_eq!(call(&mut app, &ctx, "ui.context.choose", json!({"id": "select.colorRange"}))["ok"], false);
}

#[test]
fn the_right_click_corrects_no_red_eye_and_adds_no_history_state() {
    for selection in [false, true] {
        let (mut app, ctx) = app();
        if selection {
            select(&mut app);
        }
        let before = state(&app);
        right_click(&mut app, &ctx, 20.0, 20.0);
        assert!(app.ui.canvas_tool_menu.is_some(), "the menu is open (selection: {selection})");
        assert_eq!(state(&app), before, "nothing ran (selection: {selection})");
    }
}

#[test]
fn a_mouse_right_click_on_the_canvas_opens_the_layer_menu() {
    let (app, _) = app();
    let mut h = crate::canvas_tool_menu::tests::harness(app);
    let before = state(h.state());
    let center = h.state().last_canvas_rect.center();
    crate::canvas_tool_menu::tests::right_click(&mut h, center, Modifiers::NONE);
    let menu = h.state().ui.canvas_tool_menu.clone().expect("the menu opens");
    let labels: Vec<&str> = rows(&menu).iter().map(|r| r.map_or("―", |(l, _)| l)).collect();
    assert_eq!(labels, NO_SELECTION);
    assert_eq!(state(h.state()), before, "no correction, no history state");
}

#[test]
fn the_cases_not_measured_keep_opening_nothing() {
    type Setup = fn(&mut PhotocraftApp);
    let cases: [(&str, Setup); 5] = [
        ("outside the selection", |app| app.run("select.rect", json!({"x": 40, "y": 30, "width": 10, "height": 10})).map(drop).unwrap()),
        ("a second layer", |app| app.run("layer.new.layer", json!({"name": "Ink"})).map(drop).unwrap()),
        ("a smart object", |app| app.run("layer.smartObjects.convertToSmartObject", json!({})).map(drop).unwrap()),
        ("a type layer", |app| {
            let background = app.session.active().unwrap().doc.layers[0].id.0;
            app.run("type.create", json!({"text": "Eyes", "size": 12, "x": 4, "y": 20})).unwrap();
            app.run("layer.select", json!({"layer": background})).unwrap();
            app.run("layer.delete", json!({})).unwrap();
        }),
        ("Grayscale", |app| app.run("image.mode.grayscale", json!({})).map(drop).unwrap()),
    ];
    for (what, setup) in cases {
        let (mut app, ctx) = app();
        setup(&mut app);
        let before = state(&app);
        right_click(&mut app, &ctx, 20.0, 20.0);
        assert!(app.ui.canvas_tool_menu.is_none(), "{what}: no menu");
        assert!(listed(&mut app, &ctx).is_empty(), "{what}");
        assert_eq!(state(&app), before, "{what}: nothing ran");
    }
}

#[test]
fn outside_the_image_with_a_selection_the_pasteboard_menu_opens_as_before() {
    let (mut app, ctx) = app();
    select(&mut app);
    right_click(&mut app, &ctx, -10.0, 20.0);
    assert!(app.ui.canvas_tool_menu.as_ref().is_some_and(|m| m.pasteboard), "the pasteboard colour menu");
}
