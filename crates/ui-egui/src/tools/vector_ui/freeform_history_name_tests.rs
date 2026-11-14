//! #12829: measured Photoshop History names, without changing other Pen tools or destinations.
use super::*;

/// Creates a 200 by 200 document with Freeform Pen selected in the requested drawing mode.
fn app(mode: &str) -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    app.sync_views();
    app.ui.tool = Tool::FreeformPen;
    app.ui.tool_options.vector_mode = mode.into();
    app
}

/// Builds an offset four-corner trace, optionally returning to its start to exercise closure.
fn trace(closed: bool, offset: f64) -> Vec<[f64; 3]> {
    let mut points = vec![[20.0 + offset, 20.0, 1.0], [80.0 + offset, 20.0, 1.0], [80.0 + offset, 80.0, 1.0], [20.0 + offset, 80.0, 1.0]];
    if closed {
        points.push(points[0]);
    }
    points
}

/// Counts committed undo steps, excluding the initial document state.
fn steps(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().history.past_len()
}

/// Checks that a successful stroke adds exactly one step with the expected undo label.
fn assert_step(app: &PhotocraftApp, before: usize, name: &str) {
    let h = &app.session.active().unwrap().history;
    assert_eq!(h.past_len(), before + 1, "one step per stroke");
    assert_eq!(h.undo_label(), Some(name));
    assert!(!app.ui.status_error, "{}", app.ui.status);
}

/// Checks that open and closed Work Path strokes remain separate named undo/redo operations.
#[test]
fn freeform_path_strokes_are_individual_named_steps_and_undo_redo_the_whole_stroke() {
    for closed in [false, true] {
        let mut app = app("path");
        for stroke in 0..2 {
            let before = app.session.active().unwrap().doc.clone();
            let n = steps(&app);
            freeform_finish(&mut app, &trace(closed, f64::from(stroke) * 90.0), &[]);
            assert_step(&app, n, "Freeform Pen");
            let after = app.session.active().unwrap().doc.clone();
            let path = after.work_path.as_ref().unwrap();
            assert_eq!(path.subpaths.len(), stroke as usize + 1);
            assert_eq!(path.subpaths.last().unwrap().closed, closed);
            assert_eq!(after.layers.len(), before.layers.len());
            app.session.undo();
            assert_eq!(app.session.active().unwrap().doc.work_path, before.work_path);
            assert_eq!(app.session.active().unwrap().history.redo_labels().next(), Some("Freeform Pen"));
            app.session.redo();
            assert_eq!(app.session.active().unwrap().doc.work_path, after.work_path);
            assert_step(&app, n, "Freeform Pen");
        }
    }
}

/// Checks that each open or closed Shape stroke creates a layer in one named undo/redo step.
#[test]
fn freeform_shape_strokes_create_layers_in_the_same_named_step() {
    for closed in [false, true] {
        let mut app = app("shape");
        for stroke in 0..2 {
            let before = app.session.active().unwrap().doc.clone();
            let n = steps(&app);
            freeform_finish(&mut app, &trace(closed, f64::from(stroke) * 90.0), &[]);
            assert_step(&app, n, "Freeform Pen");
            let st = app.session.active().unwrap();
            let id = st.active_layer.unwrap();
            let layer = st.doc.layer(id).unwrap().clone();
            let LayerContent::Shape(shape) = &layer.content else { panic!("expected shape") };
            assert_eq!(shape.path.subpaths.len(), 1);
            assert_eq!(shape.path.subpaths[0].closed, closed);
            assert_eq!(st.doc.layers.len(), before.layers.len() + 1);
            app.session.undo();
            assert!(app.session.active().unwrap().doc.layer(id).is_none());
            assert_eq!(app.session.active().unwrap().doc.layers, before.layers);
            app.session.redo();
            assert_eq!(app.session.active().unwrap().doc.layer(id), Some(&layer));
            assert_step(&app, n, "Freeform Pen");
        }
    }
}

/// Ensures selecting Freeform Pen cannot relabel an existing Pen or Curvature Pen draft commit.
#[test]
fn pen_and_curvature_keep_default_history_names_even_with_freeform_selected_at_commit() {
    for mode in ["path", "shape"] {
        for curvature in [false, true] {
            let mut app = app(mode);
            app.ui.tool = if curvature { Tool::CurvaturePen } else { Tool::Pen };
            let n = steps(&app);
            for [x, y] in [[20.0, 20.0], [80.0, 20.0], [80.0, 80.0]] {
                if curvature {
                    super::super::curvature_down(&mut app, x, y);
                    super::super::curvature_up(&mut app);
                } else {
                    pen_down(&mut app, x, y, false);
                    pen_up(&mut app);
                }
            }
            app.ui.tool = Tool::FreeformPen;
            assert!(pen_commit(&mut app, false));
            assert_step(&app, n, if mode == "path" { "Work Path" } else { "New Shape Layer" });
        }
    }
}

/// Ensures a Freeform gesture retains its captured destination and tool identity through commit.
#[test]
fn freeform_uses_captured_destination_and_tool_identity() {
    let mut app = app("shape");
    let owner = capture_pen_owner(&app).unwrap();
    app.ui.tool = Tool::Pen;
    app.ui.tool_options.vector_mode = "path".into();
    let n = steps(&app);
    freeform_finish_owned(&mut app, &trace(false, 0.0), &[], owner);
    assert_step(&app, n, "Freeform Pen");
    assert!(app.session.active().unwrap().doc.work_path.is_none());
    assert!(app.run("shape.info", json!({})).is_ok());
}

/// Checks that empty, stationary and click-only traces create neither a path nor a history step.
#[test]
fn freeform_click_and_empty_trace_make_no_step() {
    let mut app = app("path");
    let n = steps(&app);
    for points in [vec![], vec![[20.0, 20.0, 1.0]], vec![[20.0, 20.0, 1.0]; 3]] {
        freeform_finish(&mut app, &points, &[]);
        assert_eq!(steps(&app), n);
        assert!(app.session.active().unwrap().doc.work_path.is_none());
    }
}

/// Checks that the unmeasured existing-shape and vector-mask destinations keep their labels.
#[test]
fn unmeasured_existing_shape_and_vector_mask_keep_their_labels() {
    let mut app = app("shape");
    freeform_finish(&mut app, &trace(true, 0.0), &[]);
    app.ui.tool_options.pen_path_op = "subtract".into();
    let layers = app.session.active().unwrap().doc.layers.len();
    let n = steps(&app);
    freeform_finish(&mut app, &trace(true, 10.0), &[]);
    assert_step(&app, n, "Add Shape");
    assert_eq!(app.session.active().unwrap().doc.layers.len(), layers);

    let mut app = self::app("path");
    app.run("layer.vectorMask.revealAll", json!({})).unwrap();
    app.ui.vector_mask_target = true;
    app.ui.selected_path = Some("layer".into());
    let n = steps(&app);
    freeform_finish(&mut app, &trace(false, 0.0), &[]);
    assert_step(&app, n, "Edit Vector Mask");
    assert!(app.session.active().unwrap().doc.work_path.is_none());
}

/// Checks atomic label validation and one-step commits for accepted and default history labels.
#[test]
fn history_name_is_validated_before_mutation_and_defaults_are_preserved() {
    let path = json!({"subpaths": [{"knots": [[20, 20], [80, 80]]}]});
    for cmd in ["path.set", "shape.create"] {
        let mut app = app("path");
        for bad in [
            json!(""),
            json!("Work Path"),
            json!("Freeform Pen\n"),
            json!("\u{202e}Freeform Pen"),
            json!("x".repeat(1000)),
            json!(42),
            json!(true),
            json!([]),
            json!({}),
        ] {
            let before = app.session.active().unwrap().doc.clone();
            let n = steps(&app);
            assert!(app.run(cmd, json!({"kind": "path", "path": path, "historyName": bad})).is_err(), "{cmd}: {bad}");
            assert_eq!(steps(&app), n);
            assert!(std::sync::Arc::ptr_eq(&before, &app.session.active().unwrap().doc));
        }
        for label in [None, Some(Value::Null), Some(json!("Freeform Pen"))] {
            let mut p = json!({"kind": "path", "path": path});
            if let Some(label) = &label {
                p["historyName"] = label.clone();
            }
            app.ui.status_error = false;
            let n = steps(&app);
            app.run(cmd, p).unwrap();
            let expected = if label == Some(json!("Freeform Pen")) {
                "Freeform Pen"
            } else if cmd == "path.set" {
                "Work Path"
            } else {
                "New Shape Layer"
            };
            assert_step(&app, n, expected);
        }
    }
}

/// Drives the control channel to check release-only commits, including a selected endpoint start.
#[test]
fn control_pointer_strokes_commit_only_on_release_with_the_freeform_label() {
    use crate::control::{ControlRequest, Outcome, handle};
    let ctx = egui::Context::default();
    for mode in ["path", "shape"] {
        let mut app = app(mode);
        let call = |app: &mut PhotocraftApp, params| {
            let (req, _rx) = ControlRequest::new("ui.pointer", params);
            let Outcome::Done(reply) = handle(app, &ctx, &req) else { panic!("expected pointer reply") };
            assert_eq!(reply["ok"], true, "{reply}");
        };
        for offset in [0.0, 90.0] {
            let n = steps(&app);
            call(&mut app, json!({"events": [{"kind": "down", "x": 20.0 + offset, "y": 20.0}, {"kind": "move", "x": 80.0 + offset, "y": 40.0}]}));
            assert_eq!(steps(&app), n, "the preview makes no History step");
            call(&mut app, json!({"events": [{"kind": "move", "x": 60.0 + offset, "y": 80.0}, {"kind": "up", "x": 20.0 + offset, "y": 100.0}]}));
            assert_step(&app, n, "Freeform Pen");
        }
        if mode == "path" {
            // A further stroke starting at the selected open path's endpoint has the same name.
            assert_eq!(app.ui.selected_path.as_deref(), Some("work"));
            let n = steps(&app);
            call(&mut app, json!({"events": [{"kind": "down", "x": 110, "y": 100}, {"kind": "move", "x": 150, "y": 150}]}));
            assert_eq!(steps(&app), n);
            call(&mut app, json!({"events": [{"kind": "up", "x": 190, "y": 120}]}));
            assert_step(&app, n, "Freeform Pen");
        }
    }
}
