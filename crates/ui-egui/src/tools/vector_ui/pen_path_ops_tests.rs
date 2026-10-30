//! The Pen's Path Operations (#1397, #7533): New Layer, Combine Shapes, Subtract Front Shape,
//! Intersect Shape Areas and Exclude Overlapping Shapes decide how a new path joins the shape or
//! path it is drawn into, and Merge Shape Components bakes them, as in Photoshop's options bar.

use egui::vec2;
use egui_kittest::kittest::{NodeT, Queryable};
use photocraft_doc::vector::PathOp;
use photocraft_doc::{LayerContent, LayerId, Path};
use serde_json::json;

use super::{pen_down, pen_path_op, pen_up};
use crate::PhotocraftApp;
use crate::state::Tool;

fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    app.ui.tool = Tool::Pen;
    app
}

/// Clicks a closed square with the Pen, from `(x, y)` with side `side`.
fn square(app: &mut PhotocraftApp, x: f64, y: f64, side: f64) {
    for (dx, dy) in [(0.0, 0.0), (side, 0.0), (side, side), (0.0, side)] {
        pen_down(app, x + dx, y + dy, false);
        pen_up(app);
    }
    pen_down(app, x + 0.5, y + 0.5, false); // close
    pen_up(app);
}

fn shape_path(app: &PhotocraftApp) -> (LayerId, Path) {
    let st = app.session.active().unwrap();
    let id = st.active_layer.unwrap();
    match &st.doc.layer(id).unwrap().content {
        LayerContent::Shape(sh) => (id, sh.path.clone()),
        _ => panic!("not a shape layer"),
    }
}

fn work(app: &PhotocraftApp) -> Path {
    app.session.active().unwrap().doc.work_path.clone().expect("a work path")
}

fn layers(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().doc.layers.len()
}

fn covered(path: &Path, x: i32, y: i32) -> f32 {
    photocraft_vector::fill_rasterizer(path, photocraft_vector::DEFAULT_TOLERANCE).render(photocraft_geom::Rect::from_xywh(x, y, 1, 1))[0]
}

/// The issue's mug: the body is drawn, then the hole in its handle is cut out of the same shape
/// layer with Subtract Front Shape, one History step each.
#[test]
fn subtract_front_shape_cuts_a_hole_in_the_selected_shape_layer() {
    let mut app = app();
    app.ui.tool_options.vector_mode = "shape".into();
    assert_eq!(app.ui.tool_options.pen_path_op, "newLayer", "Photoshop's default in Shape mode");
    square(&mut app, 20.0, 20.0, 100.0);
    let (id, _) = shape_path(&app);
    let count = layers(&app);
    app.ui.tool_options.pen_path_op = "subtract".into();
    square(&mut app, 50.0, 50.0, 40.0);
    let (same, path) = shape_path(&app);
    assert_eq!((same, layers(&app)), (id, count), "no new layer");
    assert_eq!(path.subpaths.iter().map(|s| s.op).collect::<Vec<_>>(), [PathOp::Combine, PathOp::Subtract]);
    assert_eq!((covered(&path, 70, 70), covered(&path, 30, 30)), (0.0, 1.0), "a hole in the body");
    app.session.undo();
    assert_eq!(shape_path(&app).1.subpaths.len(), 1, "undo takes back only the hole");
    app.session.redo();
    // Intersect and Exclude are stored the same way.
    for (op, want) in [("intersect", PathOp::Intersect), ("exclude", PathOp::Exclude), ("combine", PathOp::Combine)] {
        app.ui.tool_options.pen_path_op = op.into();
        square(&mut app, 10.0, 10.0, 30.0);
        assert_eq!(shape_path(&app).1.subpaths.last().map(|s| s.op), Some(want), "{op}");
    }
    assert_eq!(layers(&app), count);
    // New Layer makes another shape layer again.
    app.ui.tool_options.pen_path_op = "newLayer".into();
    square(&mut app, 10.0, 10.0, 30.0);
    assert_eq!(layers(&app), count + 1);
    assert_ne!(shape_path(&app).0, id);
}

/// A locked shape layer refuses the new component: the path drawn stays as the Pen's draft, and
/// the document and History are unchanged. Unlocking advances the source revision, so the old
/// captured draft refuses; a fresh draft on the unlocked source cuts the hole.
#[test]
fn a_locked_shape_layer_keeps_the_drawn_path() {
    for lock in ["all", "pixels"] {
        let mut app = app();
        app.ui.tool_options.vector_mode = "shape".into();
        square(&mut app, 20.0, 20.0, 100.0);
        let (id, body) = shape_path(&app);
        app.run("layer.lockLayers", json!({lock: true})).unwrap();
        let steps = app.session.active().unwrap().history.past_len();
        app.ui.tool_options.pen_path_op = "subtract".into();
        square(&mut app, 50.0, 50.0, 40.0);
        let draft = app.ui.pen.clone().unwrap_or_else(|| panic!("{lock}: the draft is kept"));
        // The four corners and the close's stand-in on the first: still a closed path.
        assert!(draft.knots.len() == 5 && super::closing(&draft), "{lock}: {:?}", draft.knots);
        assert!(app.ui.status_error, "{lock}: {}", app.ui.status);
        assert_eq!(shape_path(&app), (id, body.clone()), "{lock}: the shape is unchanged");
        assert_eq!(app.session.active().unwrap().history.past_len(), steps, "{lock}: no History step");
        // Undo takes back the lock, not the drawn path.
        app.session.undo();
        assert!(app.ui.pen.is_some(), "{lock}");
        assert!(super::pen_commit(&mut app, false), "{lock}: {}", app.ui.status); // ↩
        assert!(app.ui.pen.is_none(), "{lock}");
        assert_eq!(shape_path(&app), (id, body.clone()), "a stale draft cannot overwrite the unlocked source");
        assert!(app.ui.status_error && app.ui.status.contains("newer work was preserved"));
        square(&mut app, 50.0, 50.0, 40.0);
        let (same, path) = shape_path(&app);
        assert_eq!(same, id, "{lock}");
        assert_eq!(path.subpaths.iter().map(|s| (s.op, s.closed)).collect::<Vec<_>>(), [(PathOp::Combine, true), (PathOp::Subtract, true)], "{lock}");
        assert_eq!(covered(&path, 70, 70), 0.0, "{lock}: the hole");
    }
}

/// A hole drawn for a locked shape layer: the Pen's draft, refused, and the History length then.
fn refused_hole(app: &mut PhotocraftApp) -> (LayerId, Path, usize) {
    app.ui.tool_options.vector_mode = "shape".into();
    square(app, 20.0, 20.0, 100.0);
    let (id, body) = shape_path(app);
    app.run("layer.lockLayers", json!({"all": true})).unwrap();
    let steps = app.session.active().unwrap().history.past_len();
    app.ui.tool_options.pen_path_op = "subtract".into();
    square(app, 50.0, 50.0, 40.0);
    assert!(app.ui.pen.is_some() && app.ui.status_error, "refused: {}", app.ui.status);
    (id, body, steps)
}

/// The refused hole ended for good: kept, closed, as the Work Path, one History step; the locked
/// shape is as it was.
fn assert_kept_as_work_path(app: &PhotocraftApp, id: LayerId, body: &Path, steps: usize) {
    assert!(app.ui.pen.is_none() && app.ui.pen_doc.is_none());
    assert_eq!(shape_path(app), (id, body.clone()), "the locked shape is unchanged");
    let w = work(app);
    assert_eq!(w.subpaths.len(), 1);
    assert_eq!(w.subpaths[0].knots.len(), 4);
    assert!(w.subpaths[0].closed);
    assert_eq!(app.session.active().unwrap().history.past_len(), steps + 1);
    assert_eq!(app.ui.selected_path.as_deref(), Some("work"));
    assert!(app.ui.status.contains(tl!("The path was kept as the Work Path")), "{}", app.ui.status);
}

/// Switching to another tool with a refused draft (feedback on #7648): it was committed again on
/// every frame, re-running `shape.create` and the error. Now it is ended once: kept as the Work
/// Path, and later frames do nothing more.
#[test]
fn switching_tools_ends_a_refused_draft_once() {
    let mut h = egui_kittest::Harness::builder().with_size(vec2(1400.0, 900.0)).with_max_steps(64).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        app()
    });
    h.run_steps(2);
    let (id, body, steps) = refused_hole(h.state_mut());
    h.state_mut().ui.tool = Tool::Brush;
    h.run_steps(2);
    assert_kept_as_work_path(h.state(), id, &body, steps);
    let status = h.state().ui.status.clone();
    h.run_steps(4);
    assert_eq!(h.state().ui.status, status, "no retry");
    assert_eq!(h.state().session.active().unwrap().history.past_len(), steps + 1);
}

/// Esc with a refused draft ends it the same way; ↩ keeps it as the draft, to commit again once
/// the layer is unlocked.
#[test]
fn esc_ends_a_refused_draft_and_return_keeps_it() {
    let ctx = egui::Context::default();
    let key = |app: &mut PhotocraftApp, key: egui::Key| {
        let raw = egui::RawInput {
            events: vec![egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE }],
            ..Default::default()
        };
        ctx.begin_pass(raw);
        crate::shortcuts::handle(app, &ctx);
        ctx.end_pass().textures_delta.clear();
    };
    for lang in crate::i18n::Lang::all() {
        crate::i18n::with_language(lang, || {
            for message in ["The path was kept as the Work Path", "The path was discarded"] {
                if lang != crate::i18n::Lang::EN {
                    assert!(crate::i18n::has(lang, message), "{}: {message}", lang.code());
                }
            }
            let mut app = app();
            let (id, body, steps) = refused_hole(&mut app);
            let draft = app.ui.pen.clone();
            key(&mut app, egui::Key::Enter);
            assert_eq!(app.ui.pen, draft, "↩ keeps the draft");
            assert_eq!(app.session.active().unwrap().history.past_len(), steps);
            let refused = app.ui.status.clone();
            key(&mut app, egui::Key::Escape);
            assert_kept_as_work_path(&app, id, &body, steps);
            assert_eq!(app.ui.status, format!("{refused} ({})", tl!("The path was kept as the Work Path")));
        });
    }
}

/// With no shape layer selected, an operation other than New Layer still starts a shape layer
/// (Photoshop: there is nothing to combine with), and the next path joins it.
#[test]
fn an_operation_without_a_selected_shape_starts_a_new_shape_layer() {
    let mut app = app();
    app.ui.tool_options.vector_mode = "shape".into();
    app.ui.tool_options.pen_path_op = "subtract".into();
    let count = layers(&app);
    square(&mut app, 20.0, 20.0, 100.0);
    let (id, path) = shape_path(&app);
    assert_eq!((layers(&app), path.subpaths.len()), (count + 1, 1));
    assert_eq!(covered(&path, 60, 60), 1.0, "a lone first component draws");
    square(&mut app, 50.0, 50.0, 40.0);
    assert_eq!((shape_path(&app).0, layers(&app)), (id, count + 1));
}

/// Path mode has no New Layer (it combines); the other operations are stored on each subpath
/// added to the Work Path, a selected saved path or a targeted vector mask.
#[test]
fn path_mode_adds_subpaths_with_the_operation() {
    let mut app = app();
    assert_eq!(pen_path_op(&app.ui.tool_options), Some("combine"));
    square(&mut app, 20.0, 20.0, 100.0);
    square(&mut app, 30.0, 30.0, 10.0);
    app.ui.tool_options.pen_path_op = "intersect".into();
    square(&mut app, 50.0, 50.0, 100.0);
    assert_eq!(work(&app).subpaths.iter().map(|s| s.op).collect::<Vec<_>>(), [PathOp::Combine, PathOp::Combine, PathOp::Intersect]);
    // A selected saved path.
    app.run("path.set", json!({"name": "Saved", "path": {"subpaths": [{"closed": true, "knots": [[2, 2], [60, 2], [60, 60]]}]}})).unwrap();
    app.ui.selected_path = Some("Saved".into());
    app.ui.tool_options.pen_path_op = "exclude".into();
    square(&mut app, 10.0, 10.0, 20.0);
    let saved = app.session.active().unwrap().doc.paths.iter().find(|p| p.name == "Saved").unwrap().path.clone();
    assert_eq!(saved.subpaths.last().map(|s| s.op), Some(PathOp::Exclude));
    // A targeted vector mask.
    let id = app.session.active().unwrap().active_layer.unwrap().0;
    app.run("layer.vectorMask.add", json!({"layer": id, "path": {"subpaths": [{"closed": true, "knots": [[0, 0], [100, 0], [100, 100], [0, 100]]}]}})).unwrap();
    app.ui.vector_mask_target = true;
    app.ui.tool_options.pen_path_op = "subtract".into();
    square(&mut app, 20.0, 20.0, 30.0);
    let vm = app.session.active().unwrap().doc.layer(LayerId(id)).unwrap().vector_mask.clone().unwrap();
    assert_eq!(vm.path.subpaths.iter().map(|s| s.op).collect::<Vec<_>>(), [PathOp::Combine, PathOp::Subtract]);
}

/// The options bar's Path Operations button sets the operation, offers New Layer only in Shape
/// mode, and its Merge Shape Components bakes the path the Pen draws into.
#[test]
fn the_options_bar_menu_sets_the_operation_and_merges() {
    let mut app = app();
    square(&mut app, 20.0, 20.0, 100.0);
    app.ui.tool_options.pen_path_op = "subtract".into();
    square(&mut app, 50.0, 50.0, 40.0);
    app.ui.tool_options.pen_path_op = "combine".into();
    let mut h = egui_kittest::Harness::builder().with_size(vec2(1100.0, 400.0)).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            if ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                ui.horizontal(|ui| super::options_bar(app, ui, Tool::Pen));
            }
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::Studio);
    h.run_steps(4);
    let open = |h: &mut egui_kittest::Harness<'_, PhotocraftApp>| {
        h.get_by_label("Path operations").click();
        h.run_steps(3);
    };
    open(&mut h);
    assert!(h.get_by_label("New Layer").accesskit_node().is_disabled(), "Path mode has no New Layer");
    h.get_by_label("Exclude Overlapping Shapes").click();
    h.run_steps(3);
    assert_eq!(h.state().ui.tool_options.pen_path_op, "exclude");
    assert!(h.query_by_label("Exclude Overlapping Shapes").is_none(), "choosing closes the menu");
    // Merge Shape Components: the Work Path's hole stays a hole, now as plain outlines.
    let before = work(h.state());
    open(&mut h);
    h.get_by_label("Merge Shape Components").click();
    h.run_steps(3);
    let merged = work(h.state());
    assert_ne!(merged, before);
    assert_eq!((covered(&merged, 70, 70), covered(&merged, 30, 30)), (0.0, 1.0));
    // Shape mode offers New Layer.
    h.state_mut().ui.tool_options.vector_mode = "shape".into();
    h.run_steps(2);
    open(&mut h);
    assert!(!h.get_by_label("New Layer").accesskit_node().is_disabled());
    h.get_by_label("New Layer").click();
    h.run_steps(3);
    assert_eq!(pen_path_op(&h.state().ui.tool_options), None);
}

/// Merge Shape Components is disabled where there is nothing to merge: one component, or in Shape
/// mode no shape layer selected.
#[test]
fn merge_is_disabled_with_fewer_than_two_components() {
    let mut app = app();
    square(&mut app, 20.0, 20.0, 100.0);
    let mut h = egui_kittest::Harness::builder().with_size(vec2(1100.0, 400.0)).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            if ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                ui.horizontal(|ui| super::options_bar(app, ui, Tool::Pen));
            }
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::Studio);
    h.run_steps(4);
    h.get_by_label("Path operations").click();
    h.run_steps(3);
    assert!(h.get_by_label("Merge Shape Components").accesskit_node().is_disabled());
}

/// An unknown stored operation (an edited settings file) falls back to New Layer, never panics.
#[test]
fn an_unknown_operation_falls_back_to_new_layer() {
    let mut app = app();
    app.ui.tool_options.pen_path_op = "melt".into();
    assert_eq!(pen_path_op(&app.ui.tool_options), Some("combine"));
    app.ui.tool_options.vector_mode = "shape".into();
    assert_eq!(pen_path_op(&app.ui.tool_options), None);
    let count = layers(&app);
    square(&mut app, 20.0, 20.0, 50.0);
    square(&mut app, 30.0, 30.0, 50.0);
    assert_eq!(layers(&app), count + 2);
}

#[test]
fn freeform_refuses_a_locked_shape_before_starting_a_stroke() {
    use crate::canvas::{ToolEvent, tool_event};
    for lock in ["all", "pixels"] {
        let mut app = app();
        app.ui.tool_options.vector_mode = "shape".into();
        square(&mut app, 20.0, 20.0, 100.0);
        let (id, body) = shape_path(&app);
        app.run("layer.lockLayers", json!({lock: true})).unwrap();
        let steps = app.session.active().unwrap().history.past_len();
        app.ui.tool = Tool::FreeformPen;
        app.ui.tool_options.pen_path_op = "subtract".into();
        tool_event(&mut app, ToolEvent::Down { x: 50.0, y: 50.0, pressure: 1.0 }, egui::Modifiers::NONE);
        assert!(app.drag.is_none(), "{lock}: stroke was not started");
        assert!(app.ui.status_error, "{lock}");
        assert_eq!(shape_path(&app), (id, body), "{lock}");
        assert_eq!(app.session.active().unwrap().history.past_len(), steps, "{lock}");
        app.session.undo();
        tool_event(&mut app, ToolEvent::Down { x: 50.0, y: 50.0, pressure: 1.0 }, egui::Modifiers::NONE);
        assert!(app.drag.is_some(), "{lock}: unlocked stroke can start");
    }
}
