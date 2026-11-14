//! #12554: a Remove stroke on a hidden active layer does nothing, as in the reference
//! application: no message, no alert, no stroke overlay, no job, no History state. The other
//! painting tools keep their alert or status-bar error, and the unmeasured cases keep today's.

use egui::Modifiers;
use serde_json::json;

use super::brush_refusal_alert_tests::{active, alerts, app, refused, steps, stroke};
use super::*;
use crate::canvas::{ToolEvent, tool_event};

const ERROR: &str = photocraft_engine::hidden_target::HIDDEN;

/// [`app`] with its active layer hidden.
fn hidden(tool: Tool) -> PhotocraftApp {
    let mut app = app(tool);
    let id = active(&app);
    app.run("layer.setProps", json!({"layer": id, "visible": false})).unwrap();
    app
}

/// Both Remove modes: filled at each release, and collected for ↩; and the Background, where the
/// reference application was measured.
#[test]
fn a_hidden_active_layer_is_quiet() {
    for (after_each, background) in [(true, false), (false, false), (true, true)] {
        let mut app = if background { self::app(Tool::Remove) } else { hidden(Tool::Remove) };
        if background {
            let id = app.session.active().unwrap().doc.layers[0].id.0;
            app.run("layer.select", json!({"layer": id})).unwrap();
            app.run("layer.setProps", json!({"layer": id, "visible": false})).unwrap();
        }
        app.ui.tool_options.remove_after_each_stroke = after_each;
        let (journal, before, status) = (app.session.journal.len(), steps(&app), app.ui.status.clone());
        tool_event(&mut app, ToolEvent::Down { x: 20.0, y: 32.0, pressure: 1.0 }, Modifiers::NONE);
        tool_event(&mut app, ToolEvent::Move { x: 44.0, y: 32.0, pressure: 1.0 }, Modifiers::NONE);
        assert!(app.drag.is_none() && app.trail.is_none() && app.live_stroke.is_none(), "{after_each}: no stroke overlay");
        tool_event(&mut app, ToolEvent::Up { x: 44.0, y: 32.0 }, Modifiers::NONE);
        assert!(app.remove_pending.is_empty(), "{after_each}: no stroke collected");
        assert!(app.ui.dialogs.is_empty(), "{after_each}: no dialog");
        assert_eq!((app.ui.status.as_str(), app.ui.status_error), (status.as_str(), false), "{after_each}: no message");
        assert_eq!(app.session.journal.len(), journal, "{after_each}: no job ran");
        assert_eq!(steps(&app), before, "{after_each}: no History state");
    }
}

#[test]
fn the_other_tools_keep_their_message() {
    refused(&mut hidden(Tool::Brush), "Could not use the brush tool because the target layer is hidden.");
    refused(&mut hidden(Tool::Smudge), "Could not use the smudge tool because the target layer is hidden.");
    for tool in [Tool::Pencil, Tool::Eraser, Tool::SpotHealing, Tool::Healing, Tool::CloneStamp] {
        let mut app = hidden(tool);
        let before = steps(&app);
        stroke(&mut app);
        assert_eq!((app.ui.status.as_str(), app.ui.status_error), (ERROR, true), "{tool:?}");
        assert!(alerts(&app).is_empty(), "{tool:?}: {:?}", alerts(&app));
        assert_eq!(steps(&app), before, "{tool:?}");
    }
}

/// Not measured: a hidden layer inside a hidden group, a hidden layer with its mask targeted,
/// Sample All Layers on and a hidden type layer keep today's status-bar error.
#[test]
fn unmeasured_cases_keep_the_status_error() {
    for case in ["hidden group", "mask", "Sample All Layers", "type layer"] {
        let mut app = hidden(Tool::Remove);
        match case {
            "hidden group" => {
                let id = active(&app);
                let group = app.run("layer.groupLayers", json!({})).unwrap()["layer"].as_u64().unwrap();
                app.run("layer.setProps", json!({"layer": group, "visible": false})).unwrap();
                app.run("layer.select", json!({"layer": id})).unwrap();
            }
            "mask" => {
                app.run("layer.layerMask.revealAll", json!({})).unwrap();
                assert_eq!(crate::canvas::paint_target(&app), json!("mask"), "the new mask is the target");
            }
            "Sample All Layers" => app.ui.tool_options.sample_all.remove = true,
            _ => {
                let id = app.run("type.create", json!({"x": 10, "y": 32, "text": "Hello"})).unwrap()["layer"].as_u64().unwrap();
                app.run("layer.setProps", json!({"layer": id, "visible": false})).unwrap();
            }
        }
        let before = steps(&app);
        stroke(&mut app);
        assert_eq!((app.ui.status.as_str(), app.ui.status_error), (ERROR, true), "{case}");
        assert!(app.ui.dialogs.is_empty(), "{case}: no dialog");
        assert_eq!(steps(&app), before, "{case}");
    }
}
