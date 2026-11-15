//! #12866: the Object Selection tool's canvas menu lists the reference application's rows: with a
//! selection, no Distort or Perspective and Delete and Fill Selection after the fills; with none,
//! the layer menu, its rows greyed as measured over the Background, a type and a shape layer. The
//! marquees', lassos' and Magic Wand's menus are unchanged.

use egui::{Context, Modifiers};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::{Value, json};

use super::tests::{app, harness, labels, right_click};
use super::{COUNT_MENU, NO_SELECTION_MENU, SELECTION_MENU, available, open, rows};
use crate::PhotocraftApp;
use crate::state::Tool;

/// A pixel layer filled red, active, with `select` run first when given.
fn pixel_layer(select: &[(&str, Value)]) -> PhotocraftApp {
    let mut app = app();
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("edit.fill", json!({"color": "#ff0000"})).unwrap();
    for (id, params) in select {
        app.run(id, params.clone()).unwrap();
    }
    app.ui.tool = Tool::ObjectSelection;
    app
}

/// Right-click the middle of the canvas: the open menu's entries as `ui.inspect` lists them.
fn right_click_entries(h: &mut Harness<'static, PhotocraftApp>) -> Vec<Value> {
    let center = h.state().last_canvas_rect.center();
    right_click(h, center, Modifiers::NONE);
    let inspected = crate::control::inspect(h.state(), &h.ctx);
    assert_eq!(inspected["canvasToolMenu"]["tool"], json!("ObjectSelection"));
    inspected["canvasToolMenu"]["entries"].as_array().cloned().unwrap_or_default()
}

/// The entries' labels, a separator as `None`.
fn shown(entries: &[Value]) -> Vec<Option<String>> {
    entries.iter().map(|e| if e["separator"] == json!(true) { None } else { e["label"].as_str().map(String::from) }).collect()
}

/// The issue's notation: rows joined by " · ", separators written " ― ".
fn menu(spec: &str) -> Vec<Option<String>> {
    spec.split(" ― ")
        .enumerate()
        .flat_map(|(i, group)| (i > 0).then_some(None).into_iter().chain(group.split(" · ").map(|label| Some(label.to_owned()))))
        .collect()
}

#[test]
fn with_a_selection_the_menu_lists_the_reference_rows() {
    let mut h = harness(pixel_layer(&[("select.rect", json!({"x": 4, "y": 4, "width": 24, "height": 24}))]));
    let entries = right_click_entries(&mut h);
    assert_eq!(
        shown(&entries),
        menu(
            "Deselect · Select Inverse · Feather… · Select and Mask… ― Save Selection… · Make Work Path… ― Layer Via Copy · Layer Via Cut · \
             New Layer… ― Free Transform · Transform Selection ― Fill… · Stroke… · Content-Aware Fill… · Delete and Fill Selection ― \
             Last Filter ― Fade…"
        )
    );
    // A row the menu had before runs its command, as before.
    let before = h.state().session.journal.len();
    h.get_by_label("Select Inverse").click();
    h.run_steps(2);
    assert!(h.state().ui.canvas_tool_menu.is_none());
    assert!(h.state().session.journal[before..].iter().any(|(id, _)| id == "select.inverse"));
}

#[test]
fn with_no_selection_the_menu_lists_the_layer_rows() {
    let mut h = harness(pixel_layer(&[("select.rect", json!({"x": 4, "y": 4, "width": 24, "height": 24})), ("select.deselect", json!({}))]));
    let entries = right_click_entries(&mut h);
    assert_eq!(
        shown(&entries),
        menu(
            "Blending Options… ― Duplicate Layer… · Delete Layer · Group from Layers… ― Convert to Smart Object · Convert to Layers ― \
             Rasterize Layer · Rasterize Layer Style ― Free Transform ― Color Range… · Load Selection… · Reselect"
        )
    );
    // Over a plain pixel layer these have nothing to act on and grey out.
    for label in ["Convert to Layers", "Rasterize Layer", "Rasterize Layer Style"] {
        let entry = entries.iter().find(|e| e["label"] == json!(label));
        assert_eq!(entry.map(|e| &e["enabled"]), Some(&json!(false)), "{label} is greyed out");
    }
    h.get_by_label("Reselect").click();
    h.run_steps(2);
    assert!(h.state().ui.canvas_tool_menu.is_none());
    assert!(h.state().session.active().unwrap().doc.selection.is_some(), "Reselect brings the selection back");
    assert_eq!(h.state().session.journal.last().map(|(id, _)| id.as_str()), Some("select.reselect"));
}

/// The rows greyed without a selection, as measured over each layer kind (#12866). Load
/// Selection… and Reselect follow the document, not the layer, and are left out.
const GREYED: [(&str, &[&str]); 3] = [
    ("Background", &["Blending Options…", "Group from Layers…", "Convert to Layers", "Rasterize Layer", "Rasterize Layer Style", "Free Transform"]),
    ("Type", &["Convert to Layers", "Rasterize Layer Style"]),
    ("Shape", &["Convert to Layers", "Rasterize Layer Style"]),
];

#[test]
fn background_type_and_shape_layers_get_the_layer_menu_greyed_as_measured() {
    for (kind, greyed) in GREYED {
        let mut app = app();
        // The Background stays active under a second layer, so Delete Layer has one to keep.
        let background = app.session.active().and_then(|st| st.active_layer).unwrap();
        app.run("layer.new.layer", json!({})).unwrap();
        match kind {
            "Background" => app.session.set_active_layer(Some(background)),
            "Type" => {
                app.run("type.create", json!({"x": 4, "y": 20, "text": "A", "size": 12})).unwrap();
            }
            _ => {
                app.run("shape.create", json!({"kind": "rect", "rect": [4, 4, 20, 20], "fill": "#ff0000"})).unwrap();
            }
        }
        let st = app.session.active().unwrap();
        let layer = st.doc.layer(st.active_layer.unwrap()).unwrap();
        assert_eq!(if st.doc.is_background(layer.id) { "Background" } else { layer.content.kind_name() }, kind);
        app.ui.tool = Tool::ObjectSelection;
        assert!(open(&mut app, &Context::default(), Tool::ObjectSelection, [10.0, 10.0]), "{kind}");
        let menu = app.ui.canvas_tool_menu.clone().unwrap();
        assert_eq!(labels(rows(&menu)), labels(COUNT_MENU), "{kind}");
        let wrong: Vec<_> = rows(&menu)
            .iter()
            .flatten()
            .filter(|(label, id)| !matches!(*label, "Load Selection…" | "Reselect") && available(&app, &menu, id) == greyed.contains(label))
            .map(|(label, _)| *label)
            .collect();
        assert!(wrong.is_empty(), "{kind}: enabled or greyed unlike the reference: {wrong:?}");
    }
}

/// The reference application greys Group from Layers… whenever the Background is selected, also
/// beside a pixel layer that is the active one.
#[test]
fn group_from_layers_is_greyed_with_the_background_among_the_selected_layers() {
    let mut app = pixel_layer(&[]);
    let st = app.session.active().unwrap();
    let pixel = st.active_layer.unwrap();
    let background = st.doc.layers.iter().map(|l| l.id).find(|id| st.doc.is_background(*id)).unwrap();
    assert!(open(&mut app, &Context::default(), Tool::ObjectSelection, [10.0, 10.0]));
    let menu = app.ui.canvas_tool_menu.clone().unwrap();
    assert!(available(&app, &menu, "layer.new.groupFromLayers"), "a pixel layer alone can be grouped");
    app.run("layer.select", json!({"layer": background.0, "mode": "add"})).unwrap();
    app.session.set_active_layer(Some(pixel));
    assert_eq!(app.session.active().unwrap().selected_layers().len(), 2);
    assert!(open(&mut app, &Context::default(), Tool::ObjectSelection, [10.0, 10.0]));
    let menu = app.ui.canvas_tool_menu.clone().unwrap();
    assert!(!available(&app, &menu, "layer.new.groupFromLayers"), "the Background is selected too");
}

#[test]
fn every_object_selection_row_runs_a_known_command() {
    let mut app = pixel_layer(&[]);
    let items = crate::menus::menu_items(&app);
    // Selected first, then deselected.
    for id in ["select.all", "select.deselect"] {
        app.run(id, json!({})).unwrap();
        assert!(open(&mut app, &Context::default(), Tool::ObjectSelection, [10.0, 10.0]));
        let menu = app.ui.canvas_tool_menu.clone().unwrap();
        for (label, id) in rows(&menu).iter().flatten() {
            assert!(items.iter().any(|i| i.id == *id) || photocraft_engine::commands::find(id).is_some(), "{label}: unknown command {id}");
        }
    }
}

#[test]
fn the_other_selection_tools_keep_their_menus() {
    let tools = [
        Tool::RectMarquee,
        Tool::EllipseMarquee,
        Tool::SingleRowMarquee,
        Tool::SingleColumnMarquee,
        Tool::Lasso,
        Tool::PolygonLasso,
        Tool::MagneticLasso,
        Tool::MagicWand,
    ];
    for tool in tools {
        for (selection, menu) in [(true, SELECTION_MENU), (false, NO_SELECTION_MENU)] {
            let mut app = app();
            if selection {
                app.run("select.all", json!({})).unwrap();
            }
            app.ui.tool = tool;
            assert!(open(&mut app, &Context::default(), tool, [10.0, 10.0]), "{tool:?}");
            let open_rows = rows(app.ui.canvas_tool_menu.as_ref().unwrap());
            assert_eq!(labels(open_rows), labels(menu), "{tool:?}, selection {selection}");
        }
    }
}
