//! #12888: OK on the type-layer prompt a painting tool raises adds one history state named
//! "Rasterize Layer", as the reference application does; Layer › Rasterize › Type, Cancel and
//! the shape, Smart Object and fill prompts keep their behaviour.

use egui_kittest::kittest::Queryable;
use serde_json::{Value, json};

use super::tests::{BLUR, active_content, answer, app_with, click, harness, menu, past, prompt};
use super::*;

fn step_name(app: &PhotocraftApp) -> Option<String> {
    app.session.active()?.history.undo_label().map(str::to_string)
}

fn active_layer_content(app: &PhotocraftApp) -> photocraft_doc::LayerContent {
    let st = app.session.active().unwrap();
    st.doc.layer(st.active_layer.unwrap()).unwrap().content.clone()
}

/// A press of `tool` on a `kind` layer, answered with `button` in the dialog.
fn answer_prompt(kind: Kind, tool: Tool, button: &str) -> (PhotocraftApp, usize) {
    let mut app = app_with(kind);
    app.ui.tool = tool;
    let before = past(&app);
    // The harness's first frame installs the fonts, so the press comes after it.
    let mut h = harness(app);
    click(h.state_mut(), 30.0, 20.0);
    assert!(prompt(h.state()).is_some(), "{kind:?}/{tool:?}: the prompt opens");
    h.run_steps(4);
    h.get_by_label(button).click();
    h.run_steps(4);
    let app = h.into_state();
    assert!(prompt(&app).is_none(), "{kind:?}/{tool:?}: {button} closes the prompt");
    (app, before)
}

#[test]
fn ok_from_a_painting_tool_adds_one_rasterize_layer_state() {
    for tool in [Tool::PatternStamp, Tool::Brush, Tool::HistoryBrush] {
        let (app, before) = answer_prompt(Kind::Type, tool, "OK");
        assert_eq!(past(&app), before + 1, "{tool:?}: exactly one state");
        assert_eq!(step_name(&app).as_deref(), Some("Rasterize Layer"), "{tool:?}");
        assert_eq!(active_content(&app), "Pixel", "{tool:?}: the layer is rasterized");
        // Nothing painted: the pixels are the plain rasterized type.
        let mut plain = app_with(Kind::Type);
        plain.run("layer.rasterize.type", json!({})).unwrap();
        assert!(active_layer_content(&app) == active_layer_content(&plain), "{tool:?}: the press painted nothing");
        // The name belongs to the history state: undone, it is the state to redo.
        let mut app = app;
        app.run("edit.undo", json!({})).unwrap();
        assert_eq!(app.session.active().unwrap().history.redo_labels().next(), Some("Rasterize Layer"), "{tool:?}");
    }
}

#[test]
fn cancel_adds_no_state() {
    for tool in [Tool::PatternStamp, Tool::Brush, Tool::HistoryBrush] {
        let (app, before) = answer_prompt(Kind::Type, tool, "Cancel");
        assert_eq!(past(&app), before, "{tool:?}");
        assert_eq!(active_content(&app), "Type", "{tool:?}");
    }
}

#[test]
fn layer_rasterize_type_keeps_its_step_name() {
    let mut app = app_with(Kind::Type);
    crate::menus::invoke(&mut app, &egui::Context::default(), "layer.rasterize.type", json!({})).unwrap();
    assert_eq!(step_name(&app).as_deref(), Some("Rasterize Type"), "the menu");

    let ctx = egui::Context::default();
    for (method, params) in [("ui.menu.invoke", json!({"id": "layer.rasterize.type"})), ("engine.execute", json!({"command": "layer.rasterize.type"}))] {
        let mut app = app_with(Kind::Type);
        let (req, _rx) = crate::control::ControlRequest::new(method, params);
        let crate::control::Outcome::Done(r) = crate::control::handle(&mut app, &ctx, &req) else { panic!("{method}: deferred") };
        assert_ne!(r.get("ok"), Some(&Value::Bool(false)), "{method}: {r}");
        assert_eq!(active_content(&app), "Pixel", "{method}");
        assert_eq!(step_name(&app).as_deref(), Some("Rasterize Type"), "{method}");
    }
}

#[test]
fn the_other_prompts_keep_their_step_names() {
    for kind in [Kind::Shape, Kind::SmartObject, Kind::Fill] {
        let mut direct = app_with(kind);
        direct.run(kind.command(), json!({})).unwrap();
        let (app, before) = answer_prompt(kind, Tool::Brush, "OK");
        assert_eq!(past(&app), before + 1, "{kind:?}");
        assert_eq!(step_name(&app), step_name(&direct), "{kind:?}");
        assert_ne!(step_name(&app).as_deref(), Some("Rasterize Layer"), "{kind:?}");
    }
}

/// The filter prompt's Rasterize (#4303) is not measured: its step keeps "Rasterize Type".
#[test]
fn the_filter_prompt_keeps_rasterize_type() {
    let mut app = app_with(Kind::Type);
    let before = past(&app);
    menu(&mut app, BLUR).unwrap();
    let id = prompt(&app).expect("the prompt").id;
    answer(&mut app, id);
    assert_eq!(active_content(&app), "Pixel");
    assert_eq!(past(&app), before + 1);
    assert_eq!(step_name(&app).as_deref(), Some("Rasterize Type"));
}
