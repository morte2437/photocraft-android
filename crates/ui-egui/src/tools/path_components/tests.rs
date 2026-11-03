//! Path components after they are drawn (feedback on #7648): Path Selection selects them, the
//! Path Operations menu and `+` / `-` change their operation, and the Pen carries on an open
//! shape path from its endpoint.

use egui::Key;
use photocraft_doc::vector::PathOp;
use photocraft_doc::{LayerContent, LayerId, Path};
use serde_json::json;

use crate::PhotocraftApp;
use crate::direct_select::PathRef;
use crate::state::Tool;
use crate::vector_ui::{pen_commit, pen_down, pen_up};

fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    app.ui.tool = Tool::Pen;
    app.ui.tool_options.vector_mode = "shape".into();
    app
}

fn click(app: &mut PhotocraftApp, x: f64, y: f64) {
    pen_down(app, x, y, false);
    pen_up(app);
}

/// Clicks a closed square with the Pen, from `(x, y)` with side `side`.
fn square(app: &mut PhotocraftApp, x: f64, y: f64, side: f64) {
    for (dx, dy) in [(0.0, 0.0), (side, 0.0), (side, side), (0.0, side), (0.5, 0.5)] {
        click(app, x + dx, y + dy);
    }
}

fn shape(app: &PhotocraftApp) -> (LayerId, photocraft_doc::ShapeLayer) {
    let st = app.session.active().unwrap();
    let id = st.active_layer.unwrap();
    match &st.doc.layer(id).unwrap().content {
        LayerContent::Shape(sh) => (id, sh.clone()),
        _ => panic!("not a shape layer"),
    }
}

fn ops(path: &Path) -> Vec<PathOp> {
    path.subpaths.iter().map(|s| s.op).collect()
}

fn covered(path: &Path, x: i32, y: i32) -> f32 {
    photocraft_vector::fill_rasterizer(path, photocraft_vector::DEFAULT_TOLERANCE).render(photocraft_geom::Rect::from_xywh(x, y, 1, 1))[0]
}

fn history(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().history.past_len()
}

/// Feeds one key press through the app's shortcut handling.
fn key(app: &mut PhotocraftApp, ctx: &egui::Context, key: Key, modifiers: egui::Modifiers) {
    let raw = egui::RawInput {
        events: vec![egui::Event::ModifiersChanged(modifiers), egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers }],
        ..Default::default()
    };
    ctx.begin_pass(raw);
    crate::shortcuts::handle(app, ctx);
    ctx.end_pass().textures_delta.clear();
}

/// A body and an inner square drawn with Combine Shapes: the inner one is selected with Path
/// Selection and turned into a hole with the menu's Subtract Front Shape, one History step.
#[test]
fn a_selected_component_takes_the_picked_operation() {
    let mut app = app();
    square(&mut app, 20.0, 20.0, 120.0);
    app.ui.tool_options.pen_path_op = "combine".into();
    square(&mut app, 60.0, 60.0, 40.0);
    let (id, sh) = shape(&app);
    assert_eq!(covered(&sh.path, 80, 80), 1.0);
    app.ui.tool = Tool::PathSelection;
    super::press(&mut app, [80.0, 80.0], false);
    let (r, _, heads) = super::selected(&app).expect("the inner square is selected");
    assert_eq!((r, heads), (PathRef::Layer(id.0), vec![1]));
    let steps = history(&app);
    super::set_op(&mut app, "subtract").unwrap();
    let (_, sh) = shape(&app);
    assert_eq!(ops(&sh.path), [PathOp::Combine, PathOp::Subtract]);
    assert_eq!((covered(&sh.path, 80, 80), covered(&sh.path, 30, 30)), (0.0, 1.0), "a hole");
    assert_eq!(history(&app), steps + 1);
    assert_eq!(super::selected_op(&app), Some("subtract"), "the menu shows the component's");
    // Picking what it already has adds no step; New Layer only sets the tool.
    super::set_op(&mut app, "subtract").unwrap();
    super::set_op(&mut app, "newLayer").unwrap();
    assert_eq!(history(&app), steps + 1);
    assert!(app.session.undo());
    assert_eq!(ops(&shape(&app).1.path), [PathOp::Combine, PathOp::Combine]);
}

/// `+` and `-` with the Pen tools or Path Selection: Combine Shapes and Subtract Front Shape, on
/// the selected components and for the next path. `+` arrives as ⇧= on a US keyboard, which
/// otherwise steps the blend mode; with the Move tool it still does.
#[test]
fn plus_and_minus_pick_combine_and_subtract() {
    let ctx = egui::Context::default();
    let mut app = app();
    square(&mut app, 20.0, 20.0, 120.0);
    app.ui.tool_options.pen_path_op = "combine".into();
    square(&mut app, 60.0, 60.0, 40.0);
    app.ui.tool = Tool::PathSelection;
    super::press(&mut app, [80.0, 80.0], false);
    key(&mut app, &ctx, Key::Minus, egui::Modifiers::NONE);
    assert_eq!(ops(&shape(&app).1.path), [PathOp::Combine, PathOp::Subtract]);
    assert_eq!(app.ui.tool_options.pen_path_op, "subtract");
    key(&mut app, &ctx, Key::Equals, egui::Modifiers::SHIFT);
    assert_eq!(ops(&shape(&app).1.path), [PathOp::Combine, PathOp::Combine]);
    key(&mut app, &ctx, Key::Minus, egui::Modifiers::NONE);
    key(&mut app, &ctx, Key::Plus, egui::Modifiers::NONE);
    assert_eq!(ops(&shape(&app).1.path), [PathOp::Combine, PathOp::Combine], "the keypad's +");
    // With nothing selected the Pen's next path takes it.
    app.ui.direct_selection = Default::default();
    app.ui.tool = Tool::Pen;
    key(&mut app, &ctx, Key::Minus, egui::Modifiers::NONE);
    assert_eq!(app.ui.tool_options.pen_path_op, "subtract");
    // The blend mode is the Move tool's to step.
    let blend = |app: &PhotocraftApp| app.session.active().unwrap().doc.layer(shape(app).0).unwrap().blend;
    let before = blend(&app);
    key(&mut app, &ctx, Key::Equals, egui::Modifiers::SHIFT);
    assert_eq!(blend(&app), before, "the Pen picks Combine, not a blend mode");
    assert_eq!(app.ui.tool_options.pen_path_op, "combine");
    app.ui.tool = Tool::Move;
    key(&mut app, &ctx, Key::Equals, egui::Modifiers::SHIFT);
    assert_ne!(blend(&app), before);
    assert_eq!(app.ui.tool_options.pen_path_op, "combine");
}

/// ⇧-click adds and removes components, a click off them deselects, and a drag moves only the
/// selected ones.
#[test]
fn path_selection_selects_and_moves_components() {
    let mut app = app();
    square(&mut app, 10.0, 10.0, 40.0);
    app.ui.tool_options.pen_path_op = "combine".into();
    square(&mut app, 100.0, 10.0, 40.0);
    square(&mut app, 100.0, 100.0, 40.0);
    app.ui.tool = Tool::PathSelection;
    let heads = |app: &PhotocraftApp| super::selected(app).map(|(.., h)| h).unwrap_or_default();
    super::press(&mut app, [30.0, 30.0], false);
    super::press(&mut app, [120.0, 120.0], true);
    assert_eq!(heads(&app), [0, 2]);
    // Pressing a selected one keeps both, for the drag.
    super::press(&mut app, [120.0, 120.0], false);
    assert_eq!(heads(&app), [0, 2]);
    crate::vector_ui::path_selection_finish(&mut app, [120.0, 120.0], [125.0, 130.0]);
    let (_, sh) = shape(&app);
    let first = |s: usize| sh.path.subpaths[s].knots[0].anchor;
    assert_eq!((first(0).x, first(0).y), (15.0, 20.0));
    assert_eq!((first(1).x, first(1).y), (100.0, 10.0), "not selected: stays");
    assert_eq!((first(2).x, first(2).y), (105.0, 110.0));
    super::press(&mut app, [125.0, 130.0], true);
    assert_eq!(heads(&app), [0]);
    super::press(&mut app, [190.0, 190.0], false);
    assert!(super::selected(&app).is_none(), "a click off every component deselects");
}

/// A selection left on a path that has fewer subpaths now (undone, edited elsewhere) selects
/// nothing and changes nothing, never panics.
#[test]
fn a_stale_selection_changes_nothing() {
    let mut app = app();
    square(&mut app, 10.0, 10.0, 40.0);
    let (id, _) = shape(&app);
    app.ui.tool = Tool::PathSelection;
    app.ui.direct_selection = crate::direct_select::DirectSelection { target: Some(PathRef::Layer(id.0)), anchors: vec![[5, 0]], ..Default::default() };
    assert!(super::selected(&app).is_none());
    let steps = history(&app);
    super::set_op(&mut app, "subtract").unwrap();
    assert_eq!(history(&app), steps);
    assert_eq!(super::move_selected(&mut app, [3.0, 3.0]), None);
}

/// The first component always draws as Combine (feedback on #7648): selected alone, `-` and the
/// menu leave it as it is, with no History step, and the menu shows Combine; selected with
/// another, only the other takes the operation.
#[test]
fn the_first_component_keeps_combining() {
    let ctx = egui::Context::default();
    let mut app = app();
    square(&mut app, 20.0, 20.0, 120.0);
    app.ui.tool_options.pen_path_op = "combine".into();
    square(&mut app, 60.0, 60.0, 40.0);
    app.ui.tool = Tool::PathSelection;
    super::press(&mut app, [30.0, 30.0], false);
    assert_eq!(super::selected(&app).map(|(.., h)| h), Some(vec![0]));
    let steps = history(&app);
    key(&mut app, &ctx, Key::Minus, egui::Modifiers::NONE);
    super::set_op(&mut app, "intersect").unwrap();
    assert_eq!(history(&app), steps, "no step");
    assert_eq!(ops(&shape(&app).1.path), [PathOp::Combine, PathOp::Combine]);
    assert_eq!(super::selected_op(&app), Some("combine"), "the menu shows what it draws as");
    super::press(&mut app, [80.0, 80.0], true);
    super::set_op(&mut app, "subtract").unwrap();
    assert_eq!(history(&app), steps + 1);
    assert_eq!(ops(&shape(&app).1.path), [PathOp::Combine, PathOp::Subtract]);
    assert_eq!(super::selected_op(&app), None, "they differ");
}

/// A component selection is Path Selection's: with the Pen, which doesn't show it, `-` and the
/// menu set the next path's operation and leave the components alone (feedback on #7648).
#[test]
fn the_pen_does_not_edit_a_hidden_component_selection() {
    let ctx = egui::Context::default();
    let mut app = app();
    square(&mut app, 20.0, 20.0, 120.0);
    app.ui.tool_options.pen_path_op = "combine".into();
    square(&mut app, 60.0, 60.0, 40.0);
    app.ui.tool = Tool::PathSelection;
    super::press(&mut app, [80.0, 80.0], false);
    assert!(super::selected(&app).is_some());
    app.ui.tool = Tool::Pen;
    let steps = history(&app);
    key(&mut app, &ctx, Key::Minus, egui::Modifiers::NONE);
    assert_eq!(app.ui.tool_options.pen_path_op, "subtract", "the next path's");
    assert_eq!(super::selected_op(&app), None);
    assert_eq!(history(&app), steps);
    assert_eq!(ops(&shape(&app).1.path), [PathOp::Combine, PathOp::Combine]);
}

/// Dragging a live rectangle or ellipse with Path Selection (its only component selected by the
/// press) moves the whole shape, which stays live: its corner-radius widgets stay (feedback on
/// #7648).
#[test]
fn dragging_a_live_shape_keeps_it_live() {
    for kind in ["rect", "ellipse"] {
        let mut app = app();
        app.run("shape.create", json!({"kind": kind, "rect": [20, 20, 60, 40], "fill": "#ff0000"})).unwrap();
        assert!(shape(&app).1.live.is_some(), "{kind}");
        app.ui.tool = Tool::PathSelection;
        super::press(&mut app, [50.0, 40.0], false);
        assert_eq!(super::selected(&app).map(|(.., h)| h), Some(vec![0]), "{kind}");
        let steps = history(&app);
        crate::vector_ui::path_selection_finish(&mut app, [50.0, 40.0], [60.0, 45.0]);
        let (_, sh) = shape(&app);
        assert_eq!(history(&app), steps + 1, "{kind}");
        let (x0, y0, ..) = sh.path.control_bounds().unwrap();
        assert_eq!((x0.round(), y0.round()), (30.0, 25.0), "{kind}: moved");
        match sh.live {
            Some(photocraft_doc::vector::LiveShape::Rect { rect, .. } | photocraft_doc::vector::LiveShape::Ellipse { rect }) => {
                assert_eq!(rect, [30.0, 25.0, 60.0, 40.0], "{kind}")
            }
            other => panic!("{kind}: still live, got {other:?}"),
        }
    }
}

/// A shape path left open (Enter before closing it) is carried on from its endpoint and closed on
/// its first anchor: the same layer and subpath, one History step, and it takes the options bar's
/// fill and stroke as if it had been drawn closed.
#[test]
fn the_pen_carries_on_an_open_shape_path() {
    let mut app = app();
    app.ui.tool_options.shape_fill = true;
    app.ui.tool_options.stroke_width = 0.0;
    for (x, y) in [(20.0, 20.0), (120.0, 20.0), (120.0, 120.0)] {
        click(&mut app, x, y);
    }
    assert!(pen_commit(&mut app, false));
    let (id, sh) = shape(&app);
    assert!(sh.fill.is_none() && sh.stroke.is_some(), "an open path is stroked");
    let count = app.session.active().unwrap().doc.layers.len();
    let steps = history(&app);
    click(&mut app, 120.0, 120.0); // its last anchor: carry on
    click(&mut app, 20.0, 120.0);
    click(&mut app, 20.5, 20.5); // its first anchor: close
    assert!(app.ui.pen.is_none(), "closing commits");
    let (same, sh) = shape(&app);
    assert_eq!((same, app.session.active().unwrap().doc.layers.len()), (id, count), "no new layer");
    assert_eq!(sh.path.subpaths.len(), 1);
    assert!(sh.path.subpaths[0].closed);
    assert_eq!(sh.path.subpaths[0].knots.len(), 4);
    assert!(sh.fill.is_some() && sh.stroke.is_none(), "filled like a path drawn closed");
    assert_eq!(history(&app), steps + 1);
    assert!(app.session.undo());
    assert!(!shape(&app).1.path.subpaths[0].closed);
}

/// From the first anchor too, and a closed component's fill and stroke are the layer's own.
#[test]
fn an_open_component_of_a_filled_shape_keeps_its_paint() {
    let mut app = app();
    square(&mut app, 10.0, 10.0, 60.0);
    let (id, before) = shape(&app);
    // An open component added to the layer.
    app.ui.tool_options.pen_path_op = "combine".into();
    for (x, y) in [(100.0, 100.0), (150.0, 100.0)] {
        click(&mut app, x, y);
    }
    assert!(pen_commit(&mut app, false));
    app.ui.tool_options.shape_fill = false;
    click(&mut app, 100.0, 100.0); // its first anchor
    click(&mut app, 100.0, 150.0);
    assert!(pen_commit(&mut app, false));
    let (same, sh) = shape(&app);
    assert_eq!(same, id);
    assert_eq!(sh.path.subpaths.len(), 2);
    assert_eq!(sh.path.subpaths[1].knots.len(), 3);
    assert_eq!((sh.fill, sh.stroke), (before.fill, before.stroke));
}

/// A targeted vector mask's open subpath is carried on in place too.
#[test]
fn the_pen_carries_on_a_vector_masks_open_subpath() {
    let mut app = app();
    app.ui.tool_options.vector_mode = "path".into();
    app.run("layer.new.layer", json!({})).unwrap();
    let id = app.session.active().unwrap().active_layer.unwrap().0;
    let open = json!({"subpaths": [{"closed": false, "knots": [[20, 20], [120, 20]]}]});
    app.run("layer.vectorMask.add", json!({"layer": id, "path": open})).unwrap();
    app.ui.vector_mask_target = true;
    click(&mut app, 120.0, 20.0);
    click(&mut app, 120.0, 120.0);
    assert!(pen_commit(&mut app, false));
    let st = app.session.active().unwrap();
    let mask = st.doc.layer(LayerId(id)).unwrap().vector_mask.clone().unwrap();
    assert_eq!(mask.path.subpaths.len(), 1);
    assert_eq!(mask.path.subpaths[0].knots.len(), 3);
    assert!(st.doc.work_path.is_none());
}
