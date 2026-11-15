//! #12075: a Horizontal or Vertical Type Mask click on a text layer's glyphs edits that layer as
//! the Type tool does: it becomes active, no mask session starts, and the commit adds the Type
//! tool's own history state (renamed by #12048) and no selection, and gives the mask tool back.
//! A fully locked text layer shows the Type tool's alert. A click on empty canvas, on a partly
//! locked text layer or with ⇧ still starts a mask (the last two are not measured). Clicks go
//! through the canvas's tool events.

use egui::Modifiers;
use egui_kittest::Harness;
use photocraft_doc::LayerId;
use serde_json::json;

use super::super::mask_replace_selection_tests::{click, new_app, selection};
use super::super::{commit, current_text, insert};
use super::{hit_layer, hit_offset};
use crate::PhotocraftApp;
use crate::state::{DialogKind, Tool};

const MASKS: [Tool; 2] = [Tool::HorizontalTypeMask, Tool::VerticalTypeMask];
/// The canvas well away from the text.
const OFF_TEXT: (f64, f64) = (500.0, 100.0);

/// A committed type layer "TXT" made with `type_tool` and, active above it, an empty layer, with
/// no selection and `tool` in hand.
fn typed(type_tool: Tool, tool: Tool) -> (PhotocraftApp, LayerId) {
    let mut app = new_app();
    app.run("select.deselect", json!({})).unwrap();
    app.ui.tool = type_tool;
    click(&mut app, 100.0, 200.0, Modifiers::NONE);
    insert(&mut app, "TXT");
    commit(&mut app);
    let id = active_layer(&app).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app.ui.tool = tool;
    (app, id)
}

/// [`typed`] horizontal text, and the point on its glyphs between "T" and "X".
fn txt(tool: Tool) -> (PhotocraftApp, LayerId, (f64, f64)) {
    let (mut app, id) = typed(Tool::Type, tool);
    let y = 185.0;
    let x = (100..200)
        .map(f64::from)
        .find(|&x| hit_layer(&mut app, x, y) == Some(id) && hit_offset(&mut app, id, x, y) == 1)
        .expect("a point after the first letter");
    (app, id, (x, y))
}

/// Frames of the app's own logic, each frame as the app runs it: the commit when the tool in hand
/// no longer matches the edit, then the shortcuts and typing; one of `events` per frame.
fn frames(app: PhotocraftApp, events: Vec<egui::Event>) -> PhotocraftApp {
    let mut h = Harness::builder().build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            let ctx = ui.ctx().clone();
            app.logic_frame(&ctx, &mut eframe::Frame::_new_kittest());
            // As `canvas_tests`' harness: the shortcuts wait for the app's fonts.
            if ctx.fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                crate::shortcuts::handle(app, &ctx);
            }
        },
        app,
    );
    h.run_steps(4);
    for event in events {
        h.event(event);
        h.run_steps(1);
    }
    h.into_state()
}

fn active_layer(app: &PhotocraftApp) -> Option<LayerId> {
    app.session.active().and_then(|st| st.active_layer)
}

fn history(app: &PhotocraftApp) -> Vec<String> {
    app.session.active().unwrap().history.entries()
}

fn layers(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().doc.layers.len()
}

fn alerts(app: &PhotocraftApp) -> Vec<&str> {
    app.ui.dialogs.iter().filter(|d| d.kind == DialogKind::Error).filter_map(|d| d.fields.get("message")?.as_str()).collect()
}

#[test]
fn a_type_mask_click_on_text_edits_that_layer_as_the_type_tool_does() {
    let escape = egui::Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE };
    let mut states = Vec::new();
    for tool in [Tool::Type, Tool::HorizontalTypeMask, Tool::VerticalTypeMask] {
        let (mut app, id, at) = txt(tool);
        let (before, count) = (history(&app), layers(&app));
        click(&mut app, at.0, at.1, Modifiers::NONE);
        let app = frames(app, Vec::new());
        let ed = app.ui.text_edit.clone().unwrap_or_else(|| panic!("{tool:?}: the edit outlives the frame"));
        assert_eq!((ed.layer, ed.created, ed.mask), (id.0, false, false), "{tool:?}: the text layer itself, no mask");
        assert_eq!((ed.anchor, ed.caret), (1, 1), "{tool:?}: the caret where clicked");
        assert_eq!(active_layer(&app), Some(id), "{tool:?}: the layer becomes active");
        assert_eq!(app.ui.tool, Tool::Type, "{tool:?}: the edit is the Type tool's");
        // "Use Escape to Commit" is on by default.
        let app = frames(app, vec![egui::Event::Text("Y".into()), escape.clone()]);
        assert!(app.ui.text_edit.is_none(), "{tool:?}");
        assert_eq!(app.ui.tool, tool, "{tool:?}: the tool in hand is back");
        assert_eq!(current_text(&app, id).as_deref(), Some("TYXT"), "{tool:?}");
        let after = history(&app);
        assert_eq!(after.len(), before.len() + 1, "{tool:?}: one history state: {after:?}");
        assert_eq!(layers(&app), count, "{tool:?}: no layer is added");
        assert!(selection(&app).is_none(), "{tool:?}: no selection");
        states.push(after.last().cloned());
    }
    assert!(states[0].is_some());
    assert_eq!(states[1..], [states[0].clone(), states[0].clone()], "the masks add the Type tool's state");
}

#[test]
fn a_type_mask_click_on_vertical_text_edits_it_with_the_vertical_type_tool() {
    for tool in MASKS {
        let (mut app, id) = typed(Tool::VerticalType, tool);
        let at = (60..140)
            .flat_map(|x| (100..300).map(move |y| (f64::from(x), f64::from(y))))
            .find(|&(x, y)| hit_layer(&mut app, x, y) == Some(id))
            .expect("a point on the vertical glyphs");
        click(&mut app, at.0, at.1, Modifiers::NONE);
        assert_eq!(app.ui.text_edit.as_ref().map(|ed| (ed.layer, ed.mask)), Some((id.0, false)), "{tool:?}");
        assert_eq!(app.ui.tool, Tool::VerticalType, "{tool:?}");
        commit(&mut app);
        assert_eq!(app.ui.tool, tool, "{tool:?}: the mask tool is back");
    }
}

#[test]
fn a_type_mask_click_on_fully_locked_text_alerts_and_changes_nothing() {
    for tool in MASKS {
        let (mut app, id, at) = txt(tool);
        app.run("layer.select", json!({"layer": id.0, "mode": "replace"})).unwrap();
        app.run("layer.lockLayers", json!({"all": true})).unwrap();
        app.run("layer.new.layer", json!({})).unwrap();
        let (before, count) = (history(&app), layers(&app));
        click(&mut app, at.0, at.1, Modifiers::NONE);
        assert_eq!(alerts(&app), ["Could not use the type tool because the layer is locked."], "{tool:?}");
        assert!(app.ui.text_edit.is_none(), "{tool:?}: no edit and no mask");
        assert_eq!(active_layer(&app), Some(id), "{tool:?}: the layer becomes active, as with the Type tool");
        assert_eq!(history(&app), before, "{tool:?}: no history state");
        assert_eq!(layers(&app), count, "{tool:?}");
        assert_eq!(current_text(&app, id).as_deref(), Some("TXT"), "{tool:?}");
        assert!(selection(&app).is_none(), "{tool:?}");
    }
}

#[test]
fn a_type_mask_click_on_partly_locked_text_still_starts_a_mask() {
    for lock in [json!({"position": true}), json!({"pixels": true})] {
        for tool in MASKS {
            let (mut app, id, at) = txt(tool);
            app.run("layer.select", json!({"layer": id.0, "mode": "replace"})).unwrap();
            app.run("layer.lockLayers", lock.clone()).unwrap();
            app.run("layer.new.layer", json!({})).unwrap();
            click(&mut app, at.0, at.1, Modifiers::NONE);
            let what = format!("{tool:?}, {lock}");
            assert!(app.ui.text_edit.as_ref().is_some_and(|ed| ed.mask && ed.created && ed.layer != id.0), "{what}: a new mask");
            assert!(alerts(&app).is_empty(), "{what}");
            assert_eq!(app.ui.tool, tool, "{what}");
            assert_eq!(current_text(&app, id).as_deref(), Some("TXT"), "{what}");
        }
    }
}

#[test]
fn a_type_mask_click_off_the_text_still_starts_a_mask() {
    for tool in MASKS {
        let (mut app, _, _) = txt(tool);
        click(&mut app, OFF_TEXT.0, OFF_TEXT.1, Modifiers::NONE);
        assert!(app.ui.text_edit.as_ref().is_some_and(|ed| ed.mask && ed.created), "{tool:?}: a new mask");
        assert_eq!(app.ui.tool, tool, "{tool:?}");
    }
}

#[test]
fn a_shift_click_on_text_still_starts_new_text_over_it() {
    for tool in [Tool::Type, Tool::HorizontalTypeMask, Tool::VerticalTypeMask] {
        let (mut app, id, at) = txt(tool);
        click(&mut app, at.0, at.1, Modifiers::SHIFT);
        let ed = app.ui.text_edit.as_ref().unwrap_or_else(|| panic!("{tool:?}: new text starts"));
        assert!(ed.created && ed.layer != id.0, "{tool:?}: new text, not an edit of the old");
        assert_eq!(ed.mask, tool.is_type_mask(), "{tool:?}");
    }
}
