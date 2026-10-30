//! Pen drafts and explicit document paths retain their destination until a single commit.

use egui::Modifiers;
use photocraft_doc::{LayerContent, Path};
use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::canvas::{ToolEvent, tool_event};
use crate::state::Tool;
use crate::vector_ui;

fn app(tool: Tool) -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    app.ui.tool = tool;
    app
}

fn steps(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().history.past_len()
}

fn seed(app: &mut PhotocraftApp, name: &str, y: f64) {
    app.run("path.set", json!({"name": name, "path": {"subpaths": [{"closed": false, "knots": [[10.0, y], [90.0, y]]}]}})).unwrap();
}

fn path(app: &PhotocraftApp, name: &str) -> Path {
    let doc = &app.session.active().unwrap().doc;
    if name == "work" { doc.work_path.clone().unwrap() } else { doc.paths.iter().find(|p| p.name == name).unwrap().path.clone() }
}

fn click(app: &mut PhotocraftApp, x: f64, y: f64) {
    tool_event(app, ToolEvent::Down { x, y, pressure: 1.0 }, Modifiers::NONE);
    tool_event(app, ToolEvent::Up { x, y }, Modifiers::NONE);
}

fn draft(app: &mut PhotocraftApp) {
    click(app, 30.0, 60.0);
    click(app, 130.0, 60.0);
}

#[test]
fn selected_work_and_saved_paths_append_pen_and_curvature_components_once() {
    for tool in [Tool::Pen, Tool::CurvaturePen] {
        for name in ["work", "Saved drawing"] {
            let mut app = app(tool);
            seed(&mut app, "work", 20.0);
            seed(&mut app, "Saved drawing", 40.0);
            app.ui.selected_path = Some(name.into());
            let (original, other) = (path(&app, name), path(&app, if name == "work" { "Saved drawing" } else { "work" }));
            let n = steps(&app);
            draft(&mut app);
            assert_eq!(steps(&app), n, "placing anchors is not a history step");
            vector_ui::pen_commit(&mut app, false);
            let after = path(&app, name);
            assert_eq!(after.subpaths.len(), 2, "{tool:?}: {name}");
            assert_eq!(after.subpaths[0], original.subpaths[0]);
            let anchors: Vec<_> = after.subpaths[1].knots.iter().map(|k| [k.anchor.x, k.anchor.y]).collect();
            assert_eq!(anchors, [[30.0, 60.0], [130.0, 60.0]]);
            assert_eq!(path(&app, if name == "work" { "Saved drawing" } else { "work" }), other);
            assert_eq!(app.ui.selected_path.as_deref(), Some(name));
            assert_eq!(steps(&app), n + 1);
            app.run("edit.undo", json!({})).unwrap();
            assert_eq!(path(&app, name), original);
            app.run("edit.redo", json!({})).unwrap();
            assert_eq!(path(&app, name), after);
        }
    }
}

#[test]
fn deselected_work_path_is_replaced_without_touching_saved_paths() {
    let mut app = app(Tool::Pen);
    seed(&mut app, "work", 20.0);
    seed(&mut app, "Saved drawing", 40.0);
    app.ui.selected_path = None;
    let (before, saved, n) = (path(&app, "work"), path(&app, "Saved drawing"), steps(&app));
    draft(&mut app);
    vector_ui::pen_commit(&mut app, false);
    let after = path(&app, "work");
    assert_eq!(after.subpaths.len(), 1);
    assert_eq!(after.subpaths[0].knots[0].anchor.y, 60.0);
    assert_eq!(path(&app, "Saved drawing"), saved);
    assert_eq!(steps(&app), n + 1);
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(path(&app, "work"), before);
}

#[test]
fn captured_path_and_shape_mode_do_not_follow_later_ui_selection() {
    for shape in [false, true] {
        let mut app = app(Tool::Pen);
        seed(&mut app, "work", 20.0);
        seed(&mut app, "Saved drawing", 40.0);
        app.ui.selected_path = Some("Saved drawing".into());
        app.ui.tool_options.vector_mode = if shape { "shape" } else { "path" }.into();
        let (work, saved, n) = (path(&app, "work"), path(&app, "Saved drawing"), steps(&app));
        click(&mut app, 30.0, 60.0);
        app.ui.selected_path = Some("work".into());
        app.ui.tool_options.vector_mode = if shape { "path" } else { "shape" }.into();
        app.ui.vector_mask_target = true;
        click(&mut app, 130.0, 60.0);
        vector_ui::pen_commit(&mut app, false);
        assert_eq!(path(&app, "work"), work);
        if shape {
            let st = app.session.active().unwrap();
            assert!(matches!(st.doc.layer(st.active_layer.unwrap()).unwrap().content, LayerContent::Shape(_)));
            assert_eq!(path(&app, "Saved drawing"), saved);
            assert_eq!(app.ui.selected_path.as_deref(), Some("layer"));
        } else {
            assert_eq!(path(&app, "Saved drawing").subpaths.len(), 2);
            assert_eq!(app.ui.selected_path.as_deref(), Some("Saved drawing"));
        }
        assert_eq!(steps(&app), n + 1);
    }
}

#[test]
fn captured_pen_paint_preserves_float_colour_and_alpha_after_toolbar_changes() {
    for closed in [false, true] {
        let mut app = app(Tool::Pen);
        app.ui.tool_options.vector_mode = "shape".into();
        app.ui.tool_options.shape_fill = true;
        app.ui.tool_options.stroke_width = 3.0;
        let foreground = [0.12345, 0.34567, 0.56789, 0.375];
        let background = [0.76543, 0.54321, 0.32109, 0.625];
        app.session.tools.foreground = foreground;
        app.session.tools.background = background;
        click(&mut app, 30.0, 60.0);
        app.session.tools.foreground = [1.0; 4];
        app.session.tools.background = [0.0; 4];
        click(&mut app, 130.0, 60.0);
        if closed {
            click(&mut app, 90.0, 130.0);
        }
        vector_ui::pen_commit(&mut app, closed);
        let st = app.session.active().unwrap();
        let LayerContent::Shape(shape) = &st.doc.layer(st.active_layer.unwrap()).unwrap().content else { panic!("expected Pen shape") };
        let paint = |c: [f32; 4]| photocraft_doc::Fill::Solid(photocraft_doc::Color::rgba(c[0], c[1], c[2], c[3]));
        assert_eq!(shape.fill, closed.then(|| paint(foreground)));
        assert_eq!(shape.stroke.as_ref().unwrap().paint, paint(if closed { background } else { foreground }));
    }
}

#[test]
fn stale_drafts_are_cancelled_before_tool_switch_or_pointer_input() {
    for tool in [Tool::Pen, Tool::CurvaturePen] {
        for change in ["document", "revision", "reopen"] {
            let mut app = app(tool);
            seed(&mut app, "Saved drawing", 20.0);
            app.ui.selected_path = Some("Saved drawing".into());
            draft(&mut app);
            let source = app.session.active().unwrap().doc.clone();
            let revision = app.session.active().unwrap().revision;
            match change {
                "document" => {
                    app.session.execute("file.new", json!({"width": 200, "height": 200})).unwrap();
                    app.session.execute("path.set", json!({"name":"Saved drawing", "path": {"subpaths": [{"knots": [[50, 50], [80, 80]]}]}})).unwrap();
                }
                "revision" => {
                    app.session.execute("path.set", json!({"name":"work", "path": {"subpaths": [{"knots": [[50, 50], [80, 80]]}]}})).unwrap();
                }
                "reopen" => {
                    app.session.close(0).unwrap();
                    app.session.add_document((*source).clone(), None);
                    app.session.active_mut().unwrap().revision = revision;
                }
                _ => panic!("unknown document-change fixture: {change}"),
            }
            let (current, n) = (app.session.active().unwrap().doc.clone(), steps(&app));
            app.ui.tool = Tool::Brush;
            vector_ui::finish_pen_if_left(&mut app);
            assert!(app.ui.pen.is_none(), "{tool:?}: {change}");
            vector_ui::pen_move(&mut app, 160.0, 80.0, false);
            vector_ui::curvature_move(&mut app, 160.0, 80.0);
            vector_ui::pen_up(&mut app);
            vector_ui::curvature_up(&mut app);
            vector_ui::pen_commit(&mut app, false);
            assert!(std::sync::Arc::ptr_eq(&app.session.active().unwrap().doc, &current));
            assert_eq!(steps(&app), n);
            assert_eq!(source.paths[0].path.subpaths.len(), 1);
        }
    }
}

#[test]
fn sync_views_and_input_apis_discard_unowned_or_closed_document_drafts() {
    for entry in ["sync", "move", "up", "commit", "undo", "toggle", "down"] {
        let mut app = app(Tool::CurvaturePen);
        draft(&mut app);
        app.session.close(0).unwrap();
        match entry {
            "sync" => app.sync_views(),
            "move" => vector_ui::curvature_move(&mut app, 160.0, 80.0),
            "up" => vector_ui::curvature_up(&mut app),
            "commit" => {
                vector_ui::pen_commit(&mut app, false);
            }
            "undo" => {
                assert!(!vector_ui::pen_undo_last_point(&mut app), "a closed document has no current draft point to undo");
            }
            "toggle" => vector_ui::curvature_toggle(&mut app, 30.0, 60.0),
            "down" => vector_ui::curvature_down(&mut app, 160.0, 80.0),
            _ => panic!("unknown stale-draft entry fixture: {entry}"),
        }
        assert!(app.ui.pen.is_none(), "{entry}");
    }
    let mut app = app(Tool::Pen);
    draft(&mut app);
    let restored = serde_json::from_value(serde_json::to_value(app.ui.pen.as_ref().unwrap()).unwrap()).unwrap();
    app.ui.pen = Some(restored);
    let n = steps(&app);
    vector_ui::pen_commit(&mut app, false);
    assert!(app.ui.pen.is_none());
    assert!(app.session.active().unwrap().doc.work_path.is_none());
    assert_eq!(steps(&app), n);
}

#[test]
fn same_named_saved_paths_do_not_share_selection_or_drafts_across_tabs() {
    let mut app = app(Tool::Pen);
    seed(&mut app, "Saved drawing", 20.0);
    let first = path(&app, "Saved drawing");
    app.run("file.new", json!({"width":200,"height":200})).unwrap();
    seed(&mut app, "Saved drawing", 90.0);
    let second = path(&app, "Saved drawing");
    assert!(app.session.set_active(0));
    app.sync_views();
    app.ui.selected_path = Some("Saved drawing".into());
    draft(&mut app);
    assert!(app.session.set_active(1));
    app.sync_views();
    assert!(app.ui.selected_path.is_none());
    assert!(app.ui.pen.is_none());
    let n = steps(&app);
    vector_ui::pen_commit(&mut app, false);
    assert_eq!(path(&app, "Saved drawing"), second);
    assert_eq!(steps(&app), n);
    // A fresh drawing without explicitly selecting B's saved path creates a work path.
    draft(&mut app);
    vector_ui::pen_commit(&mut app, false);
    assert_eq!(path(&app, "Saved drawing"), second);
    assert_eq!(path(&app, "work").subpaths[0].knots[0].anchor.y, 60.0);
    assert!(app.session.set_active(0));
    app.sync_views();
    assert!(app.ui.selected_path.is_none());
    // The captured-target reconciliation preserves main's valid-source commit on a tab
    // switch. Only the original path receives that draft; the same-named B path stayed exact.
    let committed = path(&app, "Saved drawing");
    assert_eq!(committed.subpaths.len(), first.subpaths.len() + 1);
    assert_eq!(committed.subpaths[0], first.subpaths[0]);
    assert_eq!(committed.subpaths.last().unwrap().knots[0].anchor.y, 60.0);
}

#[test]
fn reopened_same_id_and_revision_do_not_reuse_the_saved_path_selection() {
    let mut app = app(Tool::Pen);
    seed(&mut app, "Saved drawing", 20.0);
    app.ui.selected_path = Some("Saved drawing".into());
    let original = path(&app, "Saved drawing");
    draft(&mut app);
    let closed = app.session.close(0).unwrap();
    // Recovery can preserve both the document ID and revision before another frame is drawn.
    app.session.add_document((*closed.doc).clone(), None);
    app.session.active_mut().unwrap().revision = closed.revision;
    assert_eq!(app.session.active().unwrap().doc.id, closed.doc.id);
    app.sync_views();
    assert!(app.ui.selected_path.is_none());
    assert!(app.ui.pen.is_none());
    draft(&mut app);
    vector_ui::pen_commit(&mut app, false);
    assert_eq!(path(&app, "Saved drawing"), original);
    assert_eq!(path(&app, "work").subpaths[0].knots[0].anchor.y, 60.0);
}

#[test]
fn ordinary_path_edits_undo_and_redo_preserve_the_explicit_saved_path_selection() {
    let mut app = app(Tool::Pen);
    seed(&mut app, "Saved drawing", 20.0);
    app.ui.selected_path = Some("Saved drawing".into());
    let original = path(&app, "Saved drawing");
    let revision = app.session.active().unwrap().revision;
    seed(&mut app, "Saved drawing", 90.0);
    assert!(app.session.active().unwrap().revision > revision);
    assert_eq!(app.ui.selected_path.as_deref(), Some("Saved drawing"));
    let changed = path(&app, "Saved drawing");
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(path(&app, "Saved drawing"), original);
    assert_eq!(app.ui.selected_path.as_deref(), Some("Saved drawing"));
    app.run("edit.redo", json!({})).unwrap();
    assert_eq!(path(&app, "Saved drawing"), changed);
    assert_eq!(app.ui.selected_path.as_deref(), Some("Saved drawing"));
    app.sync_views();
    assert_eq!(app.ui.selected_path.as_deref(), Some("Saved drawing"));
}

#[test]
fn failed_pen_and_curvature_commits_keep_the_exact_draft_and_do_not_retry_each_frame() {
    for tool in [Tool::Pen, Tool::CurvaturePen] {
        let mut app = app(tool);
        click(&mut app, 30.0, 60.0);
        click(&mut app, 1e40, 60.0);
        let (before, original, n) = (app.session.active().unwrap().doc.clone(), app.ui.pen.clone().unwrap(), steps(&app));
        vector_ui::pen_commit(&mut app, false);
        let failed = app.ui.pen.as_ref().expect("invalid geometry leaves an editable draft");
        assert_eq!(failed.knots, original.knots);
        assert_eq!(failed.unlinked, original.unlinked);
        assert_eq!(failed.owner, original.owner);
        assert!(failed.commit_failed && app.ui.status_error);
        assert!(std::sync::Arc::ptr_eq(&before, &app.session.active().unwrap().doc));
        assert_eq!(steps(&app), n);
        app.ui.tool = Tool::Brush;
        let journal = app.session.journal.len();
        vector_ui::finish_pen_if_left(&mut app);
        assert_eq!(app.session.journal.len(), journal, "tool switch must not continually retry a failing commit");
        assert!(vector_ui::path_to_selection(&mut app, Value::Null).is_err(), "a failed draft cannot select an unrelated path");
        app.ui.pen.as_mut().unwrap().knots[1] = [[130.0, 60.0]; 3];
        vector_ui::pen_commit(&mut app, false);
        assert!(app.ui.pen.is_none());
        assert_eq!(path(&app, "work").subpaths[0].knots[1].anchor.x, 130.0);
        assert_eq!(steps(&app), n + 1);
    }
}

#[test]
fn ordinary_tool_switch_commits_the_owned_draft_once() {
    for tool in [Tool::Pen, Tool::CurvaturePen] {
        let mut app = app(tool);
        draft(&mut app);
        let n = steps(&app);
        app.ui.tool = Tool::Brush;
        vector_ui::finish_pen_if_left(&mut app);
        vector_ui::finish_pen_if_left(&mut app);
        assert_eq!(path(&app, "work").subpaths[0].knots.len(), 2);
        assert_eq!(steps(&app), n + 1);
        assert!(app.ui.pen.is_none());
    }
}

#[test]
fn selected_saved_paths_move_and_free_transform_without_changing_work_or_shape() {
    for tool in [Tool::PathSelection, Tool::DirectSelection] {
        let mut app = app(tool);
        app.run("shape.create", json!({"kind": "rect", "rect": [110, 110, 50, 40]})).unwrap();
        let shape = app.run("shape.info", json!({})).unwrap();
        seed(&mut app, "work", 20.0);
        seed(&mut app, "Saved drawing", 40.0);
        app.ui.selected_path = Some("Saved drawing".into());
        let (original, work, n) = (path(&app, "Saved drawing"), path(&app, "work"), steps(&app));
        let (params, selected) = vector_ui::free_transform_path(&app).unwrap();
        assert_eq!(params, json!({"name":"Saved drawing"}));
        assert_eq!(selected, original);
        vector_ui::path_selection_finish(&mut app, [0.0, 0.0], [10.4, -4.6]);
        let moved = path(&app, "Saved drawing");
        for (a, b) in original.subpaths[0].knots.iter().zip(&moved.subpaths[0].knots) {
            assert_eq!(b.anchor.x, a.anchor.x + 10.0);
            assert_eq!(b.anchor.y, a.anchor.y - 5.0);
            assert_eq!(b.in_ctrl.x, a.in_ctrl.x + 10.0);
            assert_eq!(b.out_ctrl.y, a.out_ctrl.y - 5.0);
        }
        assert_eq!(path(&app, "work"), work);
        assert_eq!(app.run("shape.info", json!({})).unwrap(), shape);
        assert_eq!(steps(&app), n + 1);
        app.run("edit.undo", json!({})).unwrap();
        assert_eq!(path(&app, "Saved drawing"), original);
        let mut params = params;
        params["matrix"] = json!([1, 0, 0, 1, 7, 9]);
        app.run("path.transform", params).unwrap();
        assert_eq!(path(&app, "Saved drawing").subpaths[0].knots[0].anchor.x, 17.0);
        assert_eq!(path(&app, "work"), work);
        assert_eq!(app.run("shape.info", json!({})).unwrap(), shape);
        app.run("edit.undo", json!({})).unwrap();
        assert_eq!(path(&app, "Saved drawing"), original);
        app.ui.selected_path = None;
        assert_eq!(vector_ui::free_transform_path(&app).unwrap().0["name"], "layer", "deselection restores the shape fallback");
    }
}

#[test]
fn explicit_work_path_precedes_vector_mask_and_deselection_restores_mask_fallback() {
    let mut app = app(Tool::PathSelection);
    seed(&mut app, "work", 20.0);
    let layer = app.session.active().unwrap().active_layer.unwrap();
    app.run("layer.vectorMask.add", json!({"layer":layer.0,"path":{"subpaths":[{"knots":[[30,40],[80,40]]}]}})).unwrap();
    app.ui.vector_mask_target = true;
    app.ui.selected_path = Some("work".into());
    assert_eq!(vector_ui::free_transform_path(&app).unwrap().0, json!({"name":"work"}));
    app.ui.selected_path = None;
    assert_eq!(vector_ui::free_transform_path(&app).unwrap().0, json!({"name":"layer","layer":layer.0}));
}

#[test]
fn pen_keeps_its_captured_vector_mask_when_another_path_is_selected() {
    let mut app = app(Tool::Pen);
    seed(&mut app, "Saved drawing", 20.0);
    let layer = app.session.active().unwrap().active_layer.unwrap();
    app.run("layer.vectorMask.add", json!({"layer":layer.0,"path":{"subpaths":[{"knots":[[30,40],[80,40]]}]}})).unwrap();
    app.ui.selected_path = None;
    app.ui.vector_mask_target = true;
    let (saved, original, n) = (path(&app, "Saved drawing"), app.session.active().unwrap().doc.layer(layer).unwrap().vector_mask.clone().unwrap(), steps(&app));
    click(&mut app, 30.0, 60.0);
    app.ui.vector_mask_target = false;
    app.ui.selected_path = Some("Saved drawing".into());
    click(&mut app, 130.0, 60.0);
    vector_ui::pen_commit(&mut app, false);
    let mask = app.session.active().unwrap().doc.layer(layer).unwrap().vector_mask.clone().unwrap();
    assert_eq!(mask.path.subpaths.len(), 2);
    assert_eq!(mask.path.subpaths[0], original.path.subpaths[0]);
    assert_eq!(mask.path.subpaths[1].knots[0].anchor.y, 60.0);
    assert_eq!(path(&app, "Saved drawing"), saved);
    assert!(app.session.active().unwrap().doc.work_path.is_none());
    assert_eq!(steps(&app), n + 1);
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(app.session.active().unwrap().doc.layer(layer).unwrap().vector_mask.as_ref(), Some(&original));
}

#[test]
fn freeform_finish_uses_its_captured_saved_path_and_rejects_foreign_documents() {
    let mut app = app(Tool::FreeformPen);
    seed(&mut app, "work", 20.0);
    seed(&mut app, "Saved drawing", 40.0);
    app.ui.selected_path = Some("Saved drawing".into());
    let owner = vector_ui::capture_pen_owner(&app).unwrap();
    let points = [[30.0, 60.0, 1.0], [80.0, 60.0, 1.0], [130.0, 60.0, 1.0]];
    let (original, work, n) = (path(&app, "Saved drawing"), path(&app, "work"), steps(&app));
    app.ui.selected_path = Some("work".into());
    app.ui.tool_options.vector_mode = "shape".into();
    vector_ui::freeform_finish_owned(&mut app, &points, &[], owner);
    assert_eq!(path(&app, "Saved drawing").subpaths.len(), 2);
    assert_eq!(path(&app, "Saved drawing").subpaths[0], original.subpaths[0]);
    assert_eq!(path(&app, "work"), work);
    assert_eq!(steps(&app), n + 1);
    app.ui.tool_options.vector_mode = "path".into();
    let owner = vector_ui::capture_pen_owner(&app).unwrap();
    app.session.execute("file.new", json!({"width":200,"height":200})).unwrap();
    let (source, n) = (app.session.active().unwrap().doc.clone(), steps(&app));
    vector_ui::freeform_finish_owned(&mut app, &points, &[], owner);
    assert!(std::sync::Arc::ptr_eq(&source, &app.session.active().unwrap().doc));
    assert_eq!(steps(&app), n);
}

#[test]
fn tab_switch_commits_only_the_valid_original_captured_saved_path() {
    let mut app = app(Tool::Pen);
    seed(&mut app, "Saved drawing", 20.0);
    app.ui.selected_path = Some("Saved drawing".into());
    draft(&mut app);
    let source = app.session.active().unwrap().doc.id;
    app.session.execute("file.new", json!({"width":200,"height":200})).unwrap();
    // Seed through the engine so UI sync cannot process the tab switch before this assertion.
    app.session.execute("path.set", json!({"name":"Destination", "path":{"subpaths":[{"closed":false,"knots":[[10.0,100.0],[90.0,100.0]]}]}})).unwrap();
    app.ui.selected_path = Some("Destination".into());
    let current = app.session.active().unwrap().doc.clone();
    assert!(vector_ui::cancel_stale_pen(&mut app));
    assert!(app.ui.pen.is_none());
    app.sync_views();
    assert_eq!(app.ui.selected_path.as_deref(), Some("Destination"));
    assert!(std::sync::Arc::ptr_eq(&current, &app.session.active().unwrap().doc));
    let old = app.session.documents().iter().find(|s| s.doc.id == source).unwrap();
    assert_eq!(old.doc.paths.iter().find(|p| p.name == "Saved drawing").unwrap().path.subpaths.len(), 2);
    assert!(old.doc.work_path.is_none());
}
#[test]
fn deleting_the_captured_target_cancels_without_recreating_it() {
    let mut app = app(Tool::Pen);
    seed(&mut app, "Saved drawing", 20.0);
    app.ui.selected_path = Some("Saved drawing".into());
    draft(&mut app);
    app.session.execute("path.delete", json!({"name":"Saved drawing"})).unwrap();
    let current = app.session.active().unwrap().doc.clone();
    let n = steps(&app);
    vector_ui::cancel_stale_pen(&mut app);
    assert!(app.ui.pen.is_none());
    assert!(app.ui.status_error);
    assert!(std::sync::Arc::ptr_eq(&current, &app.session.active().unwrap().doc));
    assert_eq!(steps(&app), n);
    assert!(app.session.active().unwrap().doc.paths.is_empty());
}
