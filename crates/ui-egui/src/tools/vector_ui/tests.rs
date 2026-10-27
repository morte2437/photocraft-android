use super::*;

/// #4084: zoomed in, a curve's chords stay within a fraction of a pixel of the true curve
/// (16 fixed chords showed corners), and a straight segment stays one chord.
#[test]
fn path_curves_stay_smooth_at_any_zoom() {
    let (p0, c0, c1, p1) = ([0.0, 100.0], [20.0, -60.0], [80.0, -60.0], [100.0, 100.0]);
    for zoom in [0.25, 1.0, 8.0, 64.0, 320.0] {
        let xf = move |q: [f64; 2]| pos2((q[0] * zoom) as f32 + 10.0, (q[1] * zoom) as f32 - 5.0);
        let pts = curve_points(p0, c0, c1, p1, &xf);
        let n = pts.len() - 1;
        assert_eq!((pts[0], pts[n]), (xf(p0), xf(p1)), "the ends are the anchors");
        for (k, w) in pts.windows(2).enumerate() {
            let curve = xf(bezier(p0, c0, c1, p1, (k as f64 + 0.5) / n as f64));
            let chord = w[0] + (w[1] - w[0]) * 0.5;
            assert!((chord - curve).length() <= 0.35, "zoom {zoom}: chord {k} of {n} is {} px off", (chord - curve).length());
        }
        // Zoomed out it stays cheap.
        if zoom <= 1.0 {
            assert!(n <= 32, "zoom {zoom}: {n} chords");
        }
    }
    let xf = |q: [f64; 2]| pos2((q[0] * 320.0) as f32, (q[1] * 320.0) as f32);
    // Retracted handles, and handles along the line: one chord.
    assert_eq!(curve_points([0.0, 0.0], [0.0, 0.0], [50.0, 50.0], [50.0, 50.0], &xf).len(), 2);
    assert_eq!(curve_points([0.0, 0.0], [10.0, 10.0], [40.0, 40.0], [50.0, 50.0], &xf).len(), 2);
    // A handle past the end is a curve that doubles back, not a line.
    assert!(curve_points([0.0, 0.0], [0.0, 0.0], [80.0, 80.0], [50.0, 50.0], &xf).len() > 2);
    // Hostile coordinates: no panic, a bounded number of points.
    for bad in [f64::NAN, f64::INFINITY, 1e300] {
        assert!(curve_points([0.0, 0.0], [bad, 0.0], [0.0, bad], [1.0, 1.0], &xf).len() <= MAX_CURVE_PIECES + 1);
    }
    // Through path_lines too: a closed subpath of two curved segments.
    let mut path = Path::default();
    path.subpaths.push(photocraft_doc::vector::Subpath {
        knots: vec![
            photocraft_doc::vector::Knot {
                anchor: photocraft_geom::Point::new(0.0, 0.0),
                in_ctrl: photocraft_geom::Point::new(0.0, 40.0),
                out_ctrl: photocraft_geom::Point::new(0.0, -40.0),
                smooth: true,
            },
            photocraft_doc::vector::Knot {
                anchor: photocraft_geom::Point::new(60.0, 0.0),
                in_ctrl: photocraft_geom::Point::new(60.0, -40.0),
                out_ctrl: photocraft_geom::Point::new(60.0, 40.0),
                smooth: true,
            },
        ],
        closed: true,
        ..Default::default()
    });
    let lines = path_lines(&path, &xf);
    assert!(lines[0].1 && lines[0].0.len() > 2 * 17, "zoomed in, more than the old 16 chords per segment: {}", lines[0].0.len());
}

#[test]
fn all_shape_tools_and_open_pen_use_the_complete_stroke_defaults() {
    for tool in [Tool::Rectangle, Tool::EllipseShape, Tool::Triangle, Tool::Polygon, Tool::Line, Tool::CustomShape] {
        let mut a = app();
        a.ui.tool_options.stroke_width = 5.0;
        a.ui.tool_options.shape_stroke = crate::shape_stroke_ui::StrokeOptions {
            cap: photocraft_doc::LineCap::Square,
            join: photocraft_doc::LineJoin::Bevel,
            dashes: vec![4.0, 2.0, 1.0, 3.0],
            dash_offset: -0.5,
            opacity: 0.4,
            ..Default::default()
        };
        let history = a.session.active().unwrap().history.past_len();
        finish_shape(&mut a, tool, [20.0, 20.0], [120.0, 100.0], Default::default());
        let info = a.run("shape.info", json!({})).unwrap();
        assert_eq!(info["stroke"]["dashes"], json!([4.0, 2.0, 1.0, 3.0]));
        assert_eq!(info["stroke"]["cap"], "square");
        assert_eq!(info["stroke"]["width"], 5.0);
        assert_eq!(a.session.active().unwrap().history.past_len(), history + 1);
        a.run("edit.undo", json!({})).unwrap();
        a.run("edit.redo", json!({})).unwrap();
        assert_eq!(a.run("shape.info", json!({})).unwrap(), info);
        a.ui.tool_options.vector_mode = "shape".into();
        a.ui.pen = Some(PenPath { knots: vec![[[30.0, 40.0]; 3], [[80.0, 40.0]; 3]], ..Default::default() });
        pen_commit(&mut a, false);
        let open = a.run("shape.info", json!({})).unwrap();
        assert_eq!(open["stroke"]["dashes"], info["stroke"]["dashes"]);
        assert_eq!(open["path"]["subpaths"][0]["closed"], false);
    }
}

fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 200, "height": 200})).unwrap();
    app.sync_views();
    app
}

#[test]
fn paths_context_menu_uses_the_clicked_row_and_dispatches_commands() {
    let mut app = app();
    let path = json!({"subpaths": [{"closed": true, "knots": [
        {"anchor": [10, 10], "in": [10, 10], "out": [10, 10]},
        {"anchor": [80, 10], "in": [80, 10], "out": [80, 10]},
        {"anchor": [80, 80], "in": [80, 80], "out": [80, 80]}
    ]}]});
    app.run("path.set", json!({"name": "First", "path": path})).unwrap();
    app.run("path.set", json!({"name": "Second", "path": path})).unwrap();
    let doc = &app.session.active().unwrap().doc;
    let first = path_rows(doc, None).into_iter().find(|r| r.name == "First").unwrap();
    let entries = path_context_actions(&first, doc, None);
    assert_eq!(entries[0].1, "path.toSelection");
    assert_eq!(entries[0].2["name"], "First");
    assert!(entries.iter().all(|(_, id, _)| photocraft_engine::commands::find(id).is_some()));
    let (_, id, params) = entries.into_iter().find(|(_, id, _)| *id == "path.toSelection").unwrap();
    app.run(id, params).unwrap();
    assert!(app.session.active().unwrap().doc.selection.is_some());
    assert_eq!(app.session.journal.last().map(|(id, _)| id.as_str()), Some("path.toSelection"));

    app.run("path.set", json!({"name": "First copy 1", "path": path})).unwrap();
    let doc = &app.session.active().unwrap().doc;
    let first = path_rows(doc, None).into_iter().find(|r| r.name == "First").unwrap();
    let (_, _, duplicate) = path_context_actions(&first, doc, None).into_iter().find(|(_, id, _)| *id == "path.set").unwrap();
    assert_eq!(duplicate["name"], "First copy 2", "duplicate must not overwrite an existing path");
}

#[test]
fn work_path_context_save_and_layer_path_deletion_rules() {
    let mut app = app();
    let path = json!({"subpaths": []});
    app.run("path.set", json!({"name": "work", "path": path})).unwrap();
    let doc = &app.session.active().unwrap().doc;
    let work = path_rows(doc, None).into_iter().find(|r| r.kind == PathRow::Work).unwrap();
    let entries = path_context_actions(&work, doc, None);
    assert!(entries.iter().any(|(_, id, _)| *id == "path.rename"));
    assert!(!entries.iter().any(|(_, id, _)| *id == "path.set"));
    let layer = PathEntry { kind: PathRow::Layer, ..work };
    let entries = path_context_actions(&layer, doc, None);
    assert!(!entries.iter().any(|(_, id, _)| *id == "path.delete"));
}

/// #6552: a shape layer's "<Layer> Shape Path" row offers Delete Path, and it deletes the
/// shape layer, as Photoshop's Delete Path does (measured: one "Delete Layer" step).
#[test]
fn shape_path_row_delete_path_deletes_the_shape_layer() {
    let mut app = app();
    app.run("shape.create", json!({"kind": "ellipse", "rect": [20, 20, 100, 70]})).unwrap();
    let st = app.session.active().unwrap();
    let (doc, shape) = (st.doc.clone(), st.active_layer);
    let layers = doc.layers.len();
    let row = path_rows(&doc, shape).into_iter().find(|r| r.kind == PathRow::Layer).unwrap();
    assert!(row.name.ends_with(" Shape Path"), "{}", row.name);
    let (_, id, params) = path_context_actions(&row, &doc, shape).into_iter().find(|(label, _, _)| *label == "Delete Path").expect("Delete Path is offered");
    app.run(id, params).unwrap();
    let doc = &app.session.active().unwrap().doc;
    assert_eq!(doc.layers.len(), layers - 1);
    assert!(shape.and_then(|id| doc.layer(id)).is_none(), "the shape layer is gone");
    app.run("edit.undo", json!({})).unwrap();
    assert!(shape.and_then(|id| app.session.active().unwrap().doc.layer(id)).is_some(), "one undo step brings it back");
}

/// #6238: double-clicking a saved path's name renames it in place, and double-clicking the
/// Work Path saves it under the name typed (it used to save it as "Path N" straight away).
#[test]
fn double_clicking_a_path_name_renames_it_in_place() {
    use egui_kittest::{Harness, kittest::Queryable};
    let mut app = app();
    let path = json!({"subpaths": [{"closed": true, "knots": [
        {"anchor": [10, 10], "in": [10, 10], "out": [10, 10]},
        {"anchor": [80, 10], "in": [80, 10], "out": [80, 10]},
        {"anchor": [80, 80], "in": [80, 80], "out": [80, 80]}
    ]}]});
    app.run("path.set", json!({"name": "First", "path": path})).unwrap();
    app.run("path.set", json!({"name": "work", "path": path})).unwrap();
    // 60 fps steps, so two clicks a frame apart count as a double-click.
    let mut h = Harness::builder().with_size(vec2(320.0, 260.0)).with_step_dt(1.0 / 60.0).build_ui_state(|ui, app| paths_panel(app, ui), app);
    h.run_steps(3);
    let rename = |h: &mut Harness<'_, PhotocraftApp>, row: &str, text: &str| {
        let name = h.get_by_label(row).rect().left_center() + vec2(60.0, 0.0);
        h.event(egui::Event::PointerMoved(name));
        h.run_steps(30);
        for pressed in [true, false, true, false] {
            h.event(egui::Event::PointerButton { pos: name, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE });
            h.step();
        }
        h.run_steps(3);
        h.event(egui::Event::Text(text.into()));
        h.run_steps(1);
        h.key_press(egui::Key::Enter);
        h.run_steps(3);
    };
    let names = |h: &Harness<'_, PhotocraftApp>| h.state().session.active().unwrap().doc.paths.iter().map(|p| p.name.clone()).collect::<Vec<_>>();
    rename(&mut h, "First", "Outline");
    assert_eq!(names(&h), ["Outline"]);
    assert!(h.state().session.active().unwrap().doc.work_path.is_some(), "renaming a saved path leaves the work path alone");
    rename(&mut h, "Work Path", "Circle");
    assert_eq!(names(&h), ["Outline", "Circle"]);
    assert!(h.state().session.active().unwrap().doc.work_path.is_none());
    assert_eq!(h.state().ui.selected_path.as_deref(), Some("Circle"));
}

/// #6238: a saved path's context menu offers Rename Path…, as the Layers panel's does.
#[test]
fn saved_path_context_menu_offers_rename() {
    let mut app = app();
    app.run("path.set", json!({"name": "First", "path": {"subpaths": []}})).unwrap();
    app.run("path.set", json!({"name": "work", "path": {"subpaths": []}})).unwrap();
    let doc = &app.session.active().unwrap().doc;
    let rename = |kind| {
        let entry = path_rows(doc, None).into_iter().find(|r| r.kind == kind).unwrap();
        path_context_actions(&entry, doc, None).into_iter().find(|(label, _, _)| *label == "Rename Path…").map(|(_, id, p)| (id, p))
    };
    assert_eq!(rename(PathRow::Saved), Some(("path.rename", json!({"name": "First"}))));
    assert_eq!(rename(PathRow::Work), None, "the work path is saved, not renamed");
}

/// How many shapes the path overlay paints for `app` this frame.
fn overlay_shapes(app: &PhotocraftApp, ctx: &egui::Context) -> usize {
    let doc = app.session.active().unwrap().doc.clone();
    let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
        let rect = Rect::from_min_size(Pos2::ZERO, vec2(400.0, 400.0));
        let painter = ui.ctx().layer_painter(egui::LayerId::background()).with_clip_rect(rect);
        let xf = ViewXform { rect, zoom: 1.0, center: [100.0, 100.0], flip: false, rotation: 0.0, aspect: 1.0 };
        draw_overlay(app, &painter, &xf, &doc);
    });
    out.textures_delta.clear();
    out.shapes.len()
}

#[test]
fn pen_keeps_anchors_of_a_finished_path() {
    // #6884: a path finished with Enter, or by closing it, lost its anchor squares while the
    // Pen stayed active, so it looked gone and could not be edited.
    let ctx = egui::Context::default();
    let mut app = app();
    app.ui.tool = Tool::Pen;
    let knots = vec![[[10.0, 10.0]; 3], [[80.0, 10.0]; 3], [[80.0, 80.0]; 3]];
    app.ui.pen = Some(PenPath { knots, ..Default::default() });
    pen_commit(&mut app, true);
    assert!(app.ui.pen.is_none());
    assert!(app.session.active().unwrap().doc.work_path.is_some());
    let with_pen = overlay_shapes(&app, &ctx);
    app.ui.tool = Tool::Brush;
    let outline_only = overlay_shapes(&app, &ctx);
    assert!(outline_only > 0, "the outline is drawn with any tool");
    // Each of the three anchors adds its square (fill and stroke).
    assert!(with_pen >= outline_only + 3, "Pen draws the anchors: {with_pen} vs {outline_only}");
    for tool in [Tool::Pen, Tool::FreeformPen, Tool::CurvaturePen, Tool::PathSelection] {
        assert!(shows_anchors(tool), "{tool:?}");
    }
    assert!(!shows_anchors(Tool::Brush) && !shows_anchors(Tool::Move));
}

#[test]
fn an_unselected_work_path_is_not_drawn_over_the_canvas() {
    // #6551: Ctrl-click Load as Selection leaves the Paths panel's selection alone, yet the Work
    // Path stayed drawn over the new selection's edge. Photoshop draws it only while selected
    // in the Paths panel (or a path tool is active).
    let ctx = egui::Context::default();
    let mut app = app();
    let path = json!({"subpaths": [{"closed": true, "knots": [
        {"anchor": [20, 20], "in": [20, 20], "out": [20, 20]},
        {"anchor": [90, 20], "in": [90, 20], "out": [90, 20]},
        {"anchor": [90, 90], "in": [90, 90], "out": [90, 90]}
    ]}]});
    app.run("path.set", json!({"name": "work", "path": path})).unwrap();
    app.run("path.toSelection", json!({"name": "work", "mode": "replace"})).unwrap();
    assert!(app.ui.selected_path.is_none());
    for tool in [Tool::Move, Tool::Brush, Tool::Rectangle, Tool::Zoom] {
        app.ui.tool = tool;
        assert_eq!(overlay_shapes(&app, &ctx), 0, "{tool:?}: an unselected Work Path stays off the canvas");
    }
    // Selected in the Paths panel: drawn with any tool.
    app.ui.selected_path = Some("work".into());
    for tool in [Tool::Move, Tool::Brush] {
        app.ui.tool = tool;
        assert!(overlay_shapes(&app, &ctx) > 0, "{tool:?}: the selected Work Path is drawn");
    }
    // Another path selected: the Work Path is not the target path.
    app.ui.selected_path = Some("Saved".into());
    app.ui.tool = Tool::Move;
    assert_eq!(overlay_shapes(&app, &ctx), 0);
    // A path tool shows the Work Path it edits even when the panel selection was cleared.
    app.ui.selected_path = None;
    for tool in [Tool::Pen, Tool::PathSelection] {
        app.ui.tool = tool;
        assert!(overlay_shapes(&app, &ctx) > 0, "{tool:?}: path tools draw the Work Path");
    }
}

#[test]
fn show_target_path_and_extras_hide_the_path_outlines() {
    // #1119: View › Show › Target Path only flipped its checkmark; the canvas kept drawing the
    // work path and the active shape's path.
    let ctx = egui::Context::default();
    let mut app = app();
    let path = json!({"subpaths": [{"closed": true, "knots": [
        {"anchor": [10, 10], "in": [10, 10], "out": [10, 10]},
        {"anchor": [80, 10], "in": [80, 10], "out": [80, 10]},
        {"anchor": [80, 80], "in": [80, 80], "out": [80, 80]}
    ]}]});
    app.run("path.set", json!({"path": path})).unwrap();
    assert!(app.session.active().unwrap().doc.work_path.is_some());
    finish_shape(&mut app, Tool::Rectangle, [100.0, 100.0], [160.0, 150.0], egui::Modifiers::NONE);
    let toggle = |app: &mut PhotocraftApp, id: &str| crate::menus::invoke(app, &ctx, id, Value::Null).unwrap();
    for tool in [Tool::Pen, Tool::PathSelection, Tool::DirectSelection, Tool::Rectangle] {
        app.ui.tool = tool;
        assert!(app.ui.view.show.target_path && app.ui.view.extras);
        assert!(overlay_shapes(&app, &ctx) > 0, "{tool:?}: the paths are drawn by default");
        toggle(&mut app, "view.show.targetPath");
        assert!(!app.ui.view.show.target_path);
        assert_eq!(overlay_shapes(&app, &ctx), 0, "{tool:?}: Target Path off");
        toggle(&mut app, "view.show.targetPath");
        toggle(&mut app, "view.extras");
        assert!(!app.ui.view.extras);
        assert_eq!(overlay_shapes(&app, &ctx), 0, "{tool:?}: Extras off");
        toggle(&mut app, "view.extras");
        assert!(overlay_shapes(&app, &ctx) > 0, "{tool:?}: shown again");
    }
    // The Pen's path in progress is a gesture, not the target path: it stays visible.
    app.ui.tool = Tool::Pen;
    app.ui.pen = Some(PenPath { knots: vec![[[20.0, 20.0]; 3], [[60.0, 40.0]; 3]], ..Default::default() });
    toggle(&mut app, "view.show.targetPath");
    assert!(overlay_shapes(&app, &ctx) > 0, "the pen path in progress is still drawn");
}

#[test]
fn the_saved_path_selected_in_the_paths_panel_is_drawn() {
    // #6550: selecting a saved path in the Paths panel drew nothing on the canvas.
    let ctx = egui::Context::default();
    let mut app = app();
    let tri = |o: f64| {
        json!({"subpaths": [{"closed": true, "knots": [
            {"anchor": [20.0 + o, 20], "in": [20.0 + o, 20], "out": [20.0 + o, 20]},
            {"anchor": [90.0 + o, 20], "in": [90.0 + o, 20], "out": [90.0 + o, 20]},
            {"anchor": [90.0 + o, 90], "in": [90.0 + o, 90], "out": [90.0 + o, 90]}
        ]}]})
    };
    app.run("path.set", json!({"name": "Path 1", "path": tri(0.0)})).unwrap();
    app.run("path.set", json!({"name": "Path 2", "path": tri(10.0)})).unwrap();
    assert!(app.session.active().unwrap().doc.work_path.is_none());
    for tool in [Tool::Move, Tool::Brush, Tool::Pen, Tool::PathSelection, Tool::DirectSelection] {
        app.ui.tool = tool;
        app.ui.selected_path = None;
        assert_eq!(overlay_shapes(&app, &ctx), 0, "{tool:?}: nothing selected, nothing drawn");
        app.ui.selected_path = Some("Path 1".into());
        assert_eq!(selected_saved_path(&app).map(|p| p.subpaths[0].knots[0].anchor.x), Some(20.0));
        assert!(overlay_shapes(&app, &ctx) > 0, "{tool:?}: the selected saved path is drawn");
        app.ui.selected_path = Some("Path 2".into());
        assert_eq!(selected_saved_path(&app).map(|p| p.subpaths[0].knots[0].anchor.x), Some(30.0));
        app.ui.selected_path = Some("Gone".into());
        assert_eq!(overlay_shapes(&app, &ctx), 0, "{tool:?}: a missing path draws nothing");
        app.ui.selected_path = Some("Path 1".into());
        app.ui.view.show.target_path = false;
        assert_eq!(overlay_shapes(&app, &ctx), 0, "{tool:?}: View > Show > Target Path off");
        app.ui.view.show.target_path = true;
    }
    // Path Selection (and the Pen tools, as for the work path) add anchor squares.
    app.ui.tool = Tool::Move;
    let plain = overlay_shapes(&app, &ctx);
    app.ui.tool = Tool::PathSelection;
    assert!(overlay_shapes(&app, &ctx) > plain);
    app.ui.tool = Tool::Pen;
    assert!(overlay_shapes(&app, &ctx) > plain);
}

#[test]
fn shape_drag_creates_layers_with_modifiers() {
    let mut app = app();
    finish_shape(&mut app, Tool::Rectangle, [10.0, 10.0], [60.0, 30.0], egui::Modifiers::SHIFT);
    let st = app.session.active().unwrap();
    let b = st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap().content_bounds();
    assert_eq!((b.width(), b.height()), (50, 50), "shift = square");
    finish_shape(&mut app, Tool::EllipseShape, [100.0, 100.0], [120.0, 110.0], egui::Modifiers::ALT);
    let st = app.session.active().unwrap();
    let b = st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap().content_bounds();
    assert!(b.x0.abs_diff(80) <= 1 && b.width().abs_diff(40) <= 1, "alt = from centre: {b:?}");
    finish_shape(&mut app, Tool::Line, [10.0, 150.0], [90.0, 152.0], egui::Modifiers::SHIFT);
    assert_eq!(app.session.active().unwrap().doc.layers.len(), 4);
}

#[test]
fn shift_pressed_during_a_shape_drag_constrains_it() {
    // #668: ⇧ only counted when held at the press; Photoshop constrains when it is pressed
    // mid-drag (and stops when it is released).
    use crate::canvas::{ToolEvent, tool_event};
    for (tool, sides) in [(Tool::Polygon, 6), (Tool::Rectangle, 4), (Tool::EllipseShape, 0)] {
        let mut app = app();
        app.ui.tool = tool;
        app.ui.tool_options.polygon_sides = 6;
        let none = egui::Modifiers::NONE;
        tool_event(&mut app, ToolEvent::Down { x: 10.0, y: 10.0, pressure: 1.0 }, none);
        tool_event(&mut app, ToolEvent::Move { x: 40.0, y: 30.0, pressure: 1.0 }, none);
        tool_event(&mut app, ToolEvent::Move { x: 70.0, y: 40.0, pressure: 1.0 }, egui::Modifiers::SHIFT);
        tool_event(&mut app, ToolEvent::Up { x: 70.0, y: 40.0 }, egui::Modifiers::SHIFT);
        let st = app.session.active().unwrap();
        let l = st.doc.layer(st.active_layer.unwrap()).unwrap();
        assert!(matches!(l.content, LayerContent::Shape(_)), "{tool:?} ({sides} sides): a shape layer");
        let b = l.surface().unwrap().content_bounds();
        // The drag is 60 × 30; constrained, the shape's box is 60 × 60 (a hexagon is narrower
        // than its box, so the height tells).
        assert!(b.height().abs_diff(60) <= 1 && b.width() <= 61, "{tool:?}: ⇧ mid-drag keeps proportions: {b:?}");
    }
}

#[test]
fn shape_preview_is_the_committed_path() {
    let mut app = app();
    app.ui.tool_options.corner_radius = 8.0;
    let preview = |app: &PhotocraftApp, tool, end| {
        shape_geometry(&app.ui.tool_options, tool, [10.0, 10.0], end, egui::Modifiers::ALT).and_then(|p| photocraft_engine::vector_cmds::shape_path(&p).ok())
    };
    for tool in [Tool::Rectangle, Tool::EllipseShape, Tool::Triangle, Tool::Polygon, Tool::Line] {
        let shown = preview(&app, tool, [60.0, 30.0]).unwrap();
        finish_shape(&mut app, tool, [10.0, 10.0], [60.0, 30.0], egui::Modifiers::ALT);
        let st = app.session.active().unwrap();
        let LayerContent::Shape(sh) = &st.doc.layer(st.active_layer.unwrap()).unwrap().content else { panic!("{tool:?}: no shape layer") };
        assert_eq!(sh.path, shown, "{tool:?}");
    }
    assert!(preview(&app, Tool::Rectangle, [10.2, 40.0]).is_none(), "too thin to draw");
    assert!(photocraft_engine::vector_cmds::shape_path(&json!({"kind": "nope", "rect": [0, 0, 5, 5]})).is_err());
    assert!(photocraft_engine::vector_cmds::shape_path(&json!({"kind": "rect"})).is_err());
}

#[test]
fn pen_shows_the_close_cue_only_over_the_first_anchor_of_a_real_segment() {
    let mut app = app();
    app.ui.tool = Tool::Pen;
    pen_down(&mut app, 20.0, 20.0, false);
    pen_up(&mut app);
    let one = app.ui.pen.clone().unwrap();
    assert!(!over_first_anchor(&one, [20.0, 20.0], 6.0), "a single anchor has nothing to close");
    pen_down(&mut app, 120.0, 20.0, false);
    pen_up(&mut app);
    let two = app.ui.pen.clone().unwrap();
    assert!(over_first_anchor(&two, [22.0, 21.0], 6.0));
    assert!(!over_first_anchor(&two, [60.0, 60.0], 6.0));
}

#[test]
fn pen_builds_and_closes_a_work_path() {
    let mut app = app();
    app.ui.tool = Tool::Pen;
    for (x, y) in [(20.0, 20.0), (120.0, 20.0)] {
        pen_down(&mut app, x, y, false);
        pen_up(&mut app);
    }
    pen_down(&mut app, 120.0, 120.0, false);
    pen_move(&mut app, 140.0, 140.0, false); // smooth knot
    pen_up(&mut app);
    pen_down(&mut app, 20.5, 20.5, false); // click the first anchor: close
    pen_up(&mut app);
    assert!(app.ui.pen.is_none());
    let wp = app.session.active().unwrap().doc.work_path.clone().unwrap();
    assert!(wp.subpaths[0].closed && wp.subpaths[0].knots.len() == 3);
    assert!(wp.subpaths[0].knots[2].smooth);
    // Path Selection moves it.
    path_selection_finish(&mut app, [50.0, 50.0], [60.0, 55.0]);
    let wp = app.session.active().unwrap().doc.work_path.clone().unwrap();
    assert_eq!((wp.subpaths[0].knots[0].anchor.x, wp.subpaths[0].knots[0].anchor.y), (30.0, 25.0));
}

fn close(a: [f64; 2], b: [f64; 2]) -> bool {
    (a[0] - b[0]).abs() < 1e-9 && (a[1] - b[1]).abs() < 1e-9
}

#[test]
fn pen_shift_click_puts_the_next_anchor_on_a_45_degree_ray() {
    let mut app = app();
    app.ui.tool = Tool::Pen;
    // The first anchor has nothing to constrain against: it lands where clicked.
    pen_down(&mut app, 20.0, 20.0, true);
    pen_up(&mut app);
    assert_eq!(app.ui.pen.as_ref().unwrap().knots[0][0], [20.0, 20.0]);
    // ~30° below horizontal snaps to the 45° diagonal at the pointer's projection.
    let (dx, dy) = (100.0 * 30f64.to_radians().cos(), 100.0 * 30f64.to_radians().sin());
    pen_down(&mut app, 20.0 + dx, 20.0 + dy, true);
    pen_up(&mut app);
    let k = app.ui.pen.as_ref().unwrap().knots[1];
    let t = 100.0 * 15f64.to_radians().cos() / std::f64::consts::SQRT_2;
    assert!(close(k[0], [20.0 + t, 20.0 + t]), "{:?}", k[0]);
    assert!(k[1] == k[0] && k[2] == k[0], "a click stays a corner");
    // ~10° off vertical snaps to straight down from the previous anchor.
    let prev = k[0];
    pen_down(&mut app, prev[0] + 8.0, prev[1] + 50.0, true);
    pen_up(&mut app);
    assert!(close(app.ui.pen.as_ref().unwrap().knots[2][0], [prev[0], prev[1] + 50.0]));
    // Without ⇧ the anchor stays at the pointer.
    pen_down(&mut app, 200.0, 190.0, false);
    pen_up(&mut app);
    assert_eq!(app.ui.pen.as_ref().unwrap().knots[3][0], [200.0, 190.0]);
    // ⇧ does not stop a click on the first anchor from closing the path.
    pen_down(&mut app, 20.5, 20.5, true);
    pen_up(&mut app);
    assert!(app.ui.pen.is_none());
    assert!(app.session.active().unwrap().doc.work_path.as_ref().unwrap().subpaths[0].closed);
}

#[test]
fn pen_shift_drag_turns_handles_to_45_degrees_live() {
    let mut app = app();
    app.ui.tool = Tool::Pen;
    pen_down(&mut app, 20.0, 20.0, false);
    pen_up(&mut app);
    pen_down(&mut app, 100.0, 100.0, false);
    // Without ⇧ the handle follows the pointer.
    pen_move(&mut app, 130.0, 110.0, false);
    assert_eq!(app.ui.pen.as_ref().unwrap().knots[1][2], [130.0, 110.0]);
    // Pressing ⇧ mid-drag snaps the handle (~18°) to horizontal; the opposite handle mirrors it.
    pen_move(&mut app, 130.0, 110.0, true);
    let k = app.ui.pen.as_ref().unwrap().knots[1];
    assert!(close(k[2], [130.0, 100.0]) && close(k[1], [70.0, 100.0]), "{k:?}");
    assert_eq!(k[0], [100.0, 100.0], "the anchor does not move");
    // Releasing ⇧ frees it again.
    pen_move(&mut app, 120.0, 140.0, false);
    assert_eq!(app.ui.pen.as_ref().unwrap().knots[1][2], [120.0, 140.0]);
    // ⌥ breaks the pair; ⇧ then constrains the outgoing handle alone (~63° → 45°).
    pen_break_handle(&mut app);
    pen_move(&mut app, 120.0, 140.0, true);
    let k = app.ui.pen.as_ref().unwrap().knots[1];
    assert!(close(k[2], [130.0, 130.0]), "{k:?}");
    assert_eq!(k[1], [80.0, 60.0], "the incoming handle stays where it was");
    pen_up(&mut app);
    // Degenerate: ⇧ with the pointer on the anchor leaves the handle on it.
    pen_down(&mut app, 200.0, 100.0, false);
    pen_move(&mut app, 200.0, 100.0, true);
    let k = app.ui.pen.as_ref().unwrap().knots[2];
    assert!(k.iter().all(|p| *p == [200.0, 100.0]), "{k:?}");
    pen_up(&mut app);
}

/// #5373: an unfinished path stays with its own document through same-frame switches.
#[test]
fn an_unclosed_path_stays_with_its_document() {
    // Both pens claim the document at their first anchor.
    let open_path = |app: &mut PhotocraftApp| {
        for (x, y) in [(20.0, 20.0), (120.0, 60.0)] {
            if app.ui.tool == Tool::CurvaturePen {
                curvature_down(app, x, y);
                curvature_up(app);
            } else {
                pen_down(app, x, y, false);
                pen_up(app);
            }
        }
    };
    let work = |app: &PhotocraftApp, i: usize| app.session.documents()[i].doc.work_path.clone();
    // File › New commits the path to its original document.
    for tool in [Tool::Pen, Tool::CurvaturePen] {
        let mut app = app();
        app.ui.tool = tool;
        open_path(&mut app);
        assert!(app.ui.pen.is_some() && app.ui.pen_doc.is_some(), "{tool:?}: claimed by its document");
        app.run("file.new", json!({"width": 300, "height": 150})).unwrap();
        assert!(app.ui.pen.is_none() && app.ui.pen_doc.is_none(), "{tool:?}: the first document's path is gone");
        assert!(work(&app, 0).is_some(), "{tool:?}: the original keeps its Work Path");
        assert_eq!(work(&app, 1), None, "{tool:?}: the new document is blank");
    }
    // Two documents; the path below is drawn on the second (active) one.
    let mut app = app();
    app.ui.tool = Tool::Pen;
    app.run("file.new", json!({"width": 300, "height": 150})).unwrap();
    open_path(&mut app);
    assert!(app.session.set_active(0));
    pen_commit(&mut app, false);
    assert!(app.ui.pen.is_none());
    assert!(work(&app, 0).is_none(), "the active document stays untouched");
    assert!(work(&app, 1).is_some(), "the owner keeps its Work Path");
    // Staying on its own document, the path still commits there.
    open_path(&mut app);
    pen_commit(&mut app, false);
    assert!(work(&app, 0).is_some(), "it commits to the document it was drawn on");
    assert!(work(&app, 1).is_some());
    for act in [0, 1, 2] {
        open_path(&mut app);
        assert!(app.session.set_active(1));
        match act {
            0 => pen_down(&mut app, 50.0, 50.0, false),
            1 => curvature_down(&mut app, 50.0, 50.0),
            _ => assert!(!pen_undo_last_point(&mut app), "nothing of this document's to undo"),
        }
        let knots = app.ui.pen.as_ref().map_or(0, |p| p.knots.len());
        assert!(knots <= 1, "act {act}: a new path, not the other document's (knots {knots})");
        // The two drawing entry points start a path of their own here; undo has none to take.
        let expected = if act == 2 { None } else { app.session.active().map(|st| st.doc.id) };
        assert_eq!(app.ui.pen_doc, expected, "act {act}");
        app.ui.pen = None;
        app.ui.pen_doc = None;
        assert!(app.session.set_active(0));
    }
    open_path(&mut app);
    let shapes = |app: &PhotocraftApp| {
        let ctx = egui::Context::default();
        let doc = app.session.active().unwrap().doc.clone();
        let xf = ViewXform {
            rect: egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(400.0, 300.0)),
            zoom: 1.0,
            center: [100.0, 75.0],
            flip: false,
            rotation: 0.0,
            aspect: 1.0,
        };
        let mut out = ctx.run_ui(Default::default(), |ui| draw_overlay(app, ui.painter(), &xf, &doc));
        out.textures_delta.clear();
        out.shapes.len()
    };
    // Same view, pen present vs absent: the difference is the path itself, nothing else.
    let drawn = |app: &mut PhotocraftApp| {
        let with_pen = shapes(app);
        let pen = app.ui.pen.take();
        let without = shapes(app);
        app.ui.pen = pen;
        with_pen - without
    };
    assert!(drawn(&mut app) > 0, "its own document draws the path");
    assert!(app.session.set_active(1));
    assert_eq!(drawn(&mut app), 0, "the other document draws nothing of it");
    assert!(app.session.set_active(0));
    app.ui.pen = None;
    app.ui.pen_doc = None;
    open_path(&mut app);
    app.ui.direct_selection.target = Some(crate::direct_select::PathRef::Pen);
    app.ui.direct_selection.anchors = vec![[0, 1]];
    app.run("file.new", json!({"width": 120, "height": 90})).unwrap();
    assert_eq!(app.ui.direct_selection, Default::default(), "the anchors went with the path");
}

#[test]
fn pen_overlay_uses_the_view_document_even_during_direct_selection() {
    let mut app = app();
    app.ui.tool = Tool::Pen;
    pen_down(&mut app, 20.0, 20.0, false);
    pen_up(&mut app);
    pen_down(&mut app, 80.0, 60.0, false);
    pen_up(&mut app);
    let owner = app.session.active().unwrap().doc.clone();
    app.session.execute("file.new", json!({"width": 200, "height": 200})).unwrap();
    let other = app.session.active().unwrap().doc.clone();
    let xf =
        ViewXform { rect: Rect::from_min_size(Pos2::ZERO, vec2(400.0, 300.0)), zoom: 1.0, center: [100.0, 100.0], flip: false, rotation: 0.0, aspect: 1.0 };
    let shapes = |app: &PhotocraftApp, doc: &Document, modifiers| {
        let ctx = egui::Context::default();
        let mut output = ctx.run_ui(egui::RawInput { events: vec![egui::Event::ModifiersChanged(modifiers)], ..Default::default() }, |ui| {
            draw_overlay(app, ui.painter(), &xf, doc)
        });
        output.textures_delta.clear();
        output.shapes.len()
    };
    for active in [0, 1] {
        assert!(app.session.set_active(active));
        for tool in [Tool::Pen, Tool::DirectSelection] {
            app.ui.tool = tool;
            for modifiers in [egui::Modifiers::NONE, egui::Modifiers::COMMAND] {
                for (doc, visible) in [(&owner, true), (&other, false)] {
                    let with_pen = shapes(&app, doc, modifiers);
                    let pen = app.ui.pen.take();
                    let without = shapes(&app, doc, modifiers);
                    app.ui.pen = pen;
                    assert_eq!(with_pen > without, visible, "active {active}, {tool:?}, {modifiers:?}");
                }
            }
        }
    }
}

#[test]
fn failed_stale_pen_commit_preserves_the_draft_until_retry() {
    for action in ["pen", "curvature", "undo", "commit", "command"] {
        let mut app = app();
        app.run("path.set", json!({"name": "work", "path": {"subpaths": [{"closed": false, "knots": [[5, 5], [10, 10]]}]}})).unwrap();
        let original = app.session.active().unwrap().doc.work_path.clone().unwrap();
        app.ui.tool = Tool::Pen;
        for (x, y) in [(20.0, 20.0), (80.0, 60.0)] {
            pen_down(&mut app, x, y, false);
            pen_up(&mut app);
        }
        let draft = app.ui.pen.clone();
        let owner = app.ui.pen_doc;
        app.session.execute("file.new", json!({"width": 200, "height": 200})).unwrap();
        app.ui.tool_options.vector_mode = "shape".into();
        app.ui.selected_path = Some("foreign named path".into());
        app.session.authorize = Some(|_, _| Err(photocraft_engine::EngineError::Other("test rejection".into())));
        match action {
            "pen" => pen_down(&mut app, 150.0, 150.0, false),
            "curvature" => curvature_down(&mut app, 150.0, 150.0),
            "undo" => assert!(!pen_undo_last_point(&mut app)),
            "commit" => pen_commit(&mut app, true),
            _ => {
                assert!(app.run("file.close", json!({})).is_err());
            }
        }
        assert_eq!(app.ui.pen, draft, "{action}: failed save retains all anchors");
        assert_eq!(app.ui.pen_doc, owner);
        assert_eq!(app.session.active_index(), Some(1));
        assert_eq!(app.session.documents()[0].doc.work_path.as_ref(), Some(&original));
        assert!(app.session.documents()[1].doc.work_path.is_none());
        assert!(app.ui.status_error);
        assert!(!crate::direct_select::candidates(&app).iter().any(|(target, _)| *target == crate::direct_select::PathRef::Pen));
        app.session.authorize = None;
        app.sync_views();
        assert!(app.ui.pen.is_none());
        assert_eq!(app.session.active_index(), Some(1));
        assert_eq!(app.ui.selected_path.as_deref(), Some("foreign named path"));
        let saved = app.session.documents()[0].doc.work_path.clone().unwrap();
        assert_eq!(saved.subpaths.len(), original.subpaths.len() + 1);
        assert_eq!(saved.subpaths[0], original.subpaths[0]);
        app.sync_views();
        assert_eq!(app.session.documents()[0].doc.work_path.as_ref(), Some(&saved), "retry never duplicates the path");
        assert!(app.session.set_active(0));
        assert!(app.session.undo());
        assert_eq!(app.session.active().unwrap().doc.work_path.as_ref(), Some(&original));
    }
}

#[test]
fn new_document_reports_a_failed_work_path_save_and_retains_the_draft() {
    let mut app = app();
    pen_down(&mut app, 20.0, 20.0, false);
    pen_up(&mut app);
    pen_down(&mut app, 80.0, 60.0, false);
    pen_up(&mut app);
    let draft = app.ui.pen.clone();
    app.session.authorize = Some(|id, _| if id == "path.set" { Err(photocraft_engine::EngineError::Other("test rejection".into())) } else { Ok(()) });
    app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    assert!(app.ui.status_error);
    assert!(app.ui.status.contains("test rejection"));
    assert_eq!(app.ui.pen, draft);
    assert_eq!(app.session.active_index(), Some(1));
    assert!(app.session.documents().iter().all(|d| d.doc.work_path.is_none()));
    app.session.authorize = None;
    app.sync_views();
    assert!(app.ui.pen.is_none());
    assert!(app.session.documents()[0].doc.work_path.is_some());
    assert!(app.session.documents()[1].doc.work_path.is_none());
}

#[test]
fn new_document_preserves_a_single_pen_anchor_as_a_work_path() {
    for tool in [Tool::Pen, Tool::CurvaturePen] {
        let mut app = app();
        app.ui.tool = tool;
        if tool == Tool::Pen {
            pen_down(&mut app, 20.0, 30.0, false);
            pen_up(&mut app);
        } else {
            curvature_down(&mut app, 20.0, 30.0);
            curvature_up(&mut app);
        }
        app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
        assert!(app.ui.pen.is_none());
        let path = app.session.documents()[0].doc.work_path.as_ref().unwrap();
        assert_eq!(path.subpaths[0].knots.len(), 1);
        assert_eq!(path.subpaths[0].knots[0].anchor, photocraft_geom::Point::new(20.0, 30.0));
        assert!(app.session.active().unwrap().doc.work_path.is_none());
    }
}

#[test]
fn stale_pen_commits_to_its_owner_across_document_changes() {
    use photocraft_doc::{ColorMode, SampleType, Size};
    for tool in [Tool::Pen, Tool::CurvaturePen] {
        for action in ["switch", "open", "closeOther", "pen", "curvature", "undo", "commit"] {
            let mut app = app();
            app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
            app.ui.tool = tool;
            for (x, y) in [(20.0, 20.0), (80.0, 60.0), (120.0, 30.0)] {
                if tool == Tool::Pen {
                    pen_down(&mut app, x, y, false);
                    pen_up(&mut app);
                } else {
                    curvature_down(&mut app, x, y);
                    curvature_up(&mut app);
                }
            }
            let owner = app.ui.pen_doc.unwrap();
            let expected = photocraft_engine::vector_cmds::parse_path(&pen_to_json(app.ui.pen.as_ref().unwrap(), false)).unwrap();
            app.ui.direct_selection.target = Some(crate::direct_select::PathRef::Pen);
            app.ui.direct_selection.anchors = vec![[0, 1]];
            assert!(app.session.set_active(0));
            match action {
                "open" => {
                    app.services.import = Some(Box::new(|name, _, _| Ok((Document::new(name, Size::new(4, 4), ColorMode::Rgb, SampleType::U8), Vec::new()))));
                    app.open_file("synthetic.png", &[]).unwrap();
                }
                "closeOther" => {
                    app.run("file.close", json!({"document": 0})).unwrap();
                }
                "pen" => pen_down(&mut app, 150.0, 150.0, false),
                "curvature" => curvature_down(&mut app, 150.0, 150.0),
                "undo" => assert!(!pen_undo_last_point(&mut app)),
                "commit" => pen_commit(&mut app, true),
                _ => app.sync_views(),
            }
            let source = app.session.documents().iter().find(|d| d.doc.id == owner).unwrap();
            let path = source.doc.work_path.as_ref().expect("the owner keeps the unfinished path");
            assert_eq!(path, &expected, "{tool:?} {action}");
            assert!(app.session.documents().iter().filter(|d| d.doc.id != owner).all(|d| d.doc.work_path.is_none()));
            if action != "closeOther" {
                assert_ne!(app.session.active().unwrap().doc.id, owner, "the active document is restored");
            }
            assert_eq!(app.ui.direct_selection, Default::default());
            let index = app.session.documents().iter().position(|d| d.doc.id == owner).unwrap();
            assert!(app.session.set_active(index));
            assert!(app.session.undo());
            assert!(app.session.active().unwrap().doc.work_path.is_none());
            assert!(app.session.redo());
            assert!(app.session.active().unwrap().doc.work_path.is_some());
        }
    }
}

/// #4721: a second Pen path joins the Work Path it was drawn after (still selected), as a new
/// subpath, instead of replacing it; a saved path that is selected takes it instead; with no
/// path selected, the new path replaces the Work Path, as in Photoshop.
#[test]
fn a_new_pen_path_joins_the_selected_path() {
    let mut app = app();
    app.ui.tool = Tool::Pen;
    let draw = |app: &mut PhotocraftApp, x: f64| {
        for (dx, dy) in [(0.0, 0.0), (40.0, 0.0), (40.0, 40.0)] {
            pen_down(app, x + dx, 20.0 + dy, false);
            pen_up(app);
        }
        pen_down(app, x + 0.5, 20.5, false); // close
        pen_up(app);
    };
    let work = |app: &PhotocraftApp| app.session.active().unwrap().doc.work_path.clone().unwrap();
    draw(&mut app, 20.0);
    assert_eq!(app.ui.selected_path.as_deref(), Some("work"));
    draw(&mut app, 120.0);
    let wp = work(&app);
    assert_eq!(wp.subpaths.len(), 2, "the first path stays");
    assert!(wp.subpaths.iter().all(|s| s.closed && s.knots.len() == 3));
    assert_eq!(wp.subpaths[0].knots[0].anchor.x, 20.0);
    assert_eq!(wp.subpaths[1].knots[0].anchor.x, 120.0);
    // One History step per path.
    app.session.undo();
    assert_eq!(work(&app).subpaths.len(), 1, "undo takes back only the second path");
    app.session.redo();
    // A selected saved path takes the next one.
    app.run("path.set", json!({"name": "Saved", "path": {"subpaths": [{"knots": [[2, 2], [12, 2], [12, 8]]}]}})).unwrap();
    app.ui.selected_path = Some("Saved".into());
    draw(&mut app, 220.0);
    let saved = app.session.active().unwrap().doc.paths.iter().find(|p| p.name == "Saved").unwrap().path.clone();
    assert_eq!(saved.subpaths.len(), 2);
    assert_eq!(work(&app).subpaths.len(), 2, "the Work Path is left alone");
    assert_eq!(app.ui.selected_path.as_deref(), Some("Saved"));
    // No path selected: a new Work Path replaces the old one.
    app.ui.selected_path = None;
    draw(&mut app, 320.0);
    let wp = work(&app);
    assert_eq!(wp.subpaths.len(), 1);
    assert_eq!(wp.subpaths[0].knots[0].anchor.x, 320.0);
    assert_eq!(app.ui.selected_path.as_deref(), Some("work"));
}

/// #4596: the close-path pointer shows where a click would close the path, and only there:
/// over the first anchor once the Pen has two anchors (the Curvature Pen three), not while an
/// anchor is being dragged.
#[test]
fn close_path_pointer_shows_where_a_click_closes() {
    let mut app = app();
    app.ui.tool = Tool::Pen;
    assert!(!closes_path_at(&app, Tool::Pen, 20.0, 20.0), "no path yet");
    pen_down(&mut app, 20.0, 20.0, false);
    pen_up(&mut app);
    assert!(!closes_path_at(&app, Tool::Pen, 20.0, 20.0), "one anchor: a click there doesn't close");
    pen_down(&mut app, 120.0, 20.0, false);
    assert!(!closes_path_at(&app, Tool::Pen, 20.0, 20.0), "not while dragging the new anchor");
    pen_up(&mut app);
    assert!(closes_path_at(&app, Tool::Pen, 21.0, 22.0), "over the first anchor");
    assert!(!closes_path_at(&app, Tool::Pen, 120.0, 20.0) && !closes_path_at(&app, Tool::Pen, 60.0, 60.0), "the last anchor and empty canvas");
    assert!(!closes_path_at(&app, Tool::CurvaturePen, 20.0, 20.0), "a Pen path isn't the Curvature Pen's");
    pen_down(&mut app, 21.0, 22.0, false);
    pen_up(&mut app);
    assert!(app.ui.pen.is_none() && !closes_path_at(&app, Tool::Pen, 20.0, 20.0), "the click closed it");

    app.ui.tool = Tool::CurvaturePen;
    for (x, y) in [(20.0, 20.0), (120.0, 20.0)] {
        curvature_down(&mut app, x, y);
        curvature_up(&mut app);
    }
    assert!(!closes_path_at(&app, Tool::CurvaturePen, 20.0, 20.0), "two anchors: the Curvature Pen moves the first");
    curvature_down(&mut app, 120.0, 120.0);
    curvature_up(&mut app);
    assert!(closes_path_at(&app, Tool::CurvaturePen, 20.0, 20.0));
    assert!(!closes_path_at(&app, Tool::CurvaturePen, 120.0, 120.0));
}

#[test]
fn pen_point_undo_preserves_earlier_knots_and_document_history() {
    let mut app = app();
    app.ui.tool = Tool::Pen;
    app.run("shape.create", json!({"kind": "rect", "rect": [10, 10, 40, 40], "fill": "#ff0000"})).unwrap();
    let old_history = app.session.active().unwrap().history.past_len();

    pen_down(&mut app, 20.0, 20.0, false);
    pen_up(&mut app);
    pen_down(&mut app, 120.0, 20.0, false);
    pen_move(&mut app, 120.0, 60.0, false);
    pen_up(&mut app);
    pen_down(&mut app, 120.0, 120.0, false);
    pen_up(&mut app);

    let original_first_two = app.ui.pen.as_ref().unwrap().knots[..2].to_vec();
    assert!(pen_undo_last_point(&mut app));
    let pen = app.ui.pen.as_ref().unwrap();
    assert_eq!(pen.knots, original_first_two);
    assert!(!pen.dragging, "undo releases the current Pen drag");
    assert_eq!(app.session.active().unwrap().history.past_len(), old_history, "undo must not roll back previous document changes");
    assert!(app.session.active().unwrap().doc.work_path.is_none(), "path is not committed yet");

    pen_commit(&mut app, false);
    let work = app.session.active().unwrap().doc.work_path.as_ref().unwrap();
    assert_eq!(work.subpaths[0].knots.len(), 2);
    assert!(work.subpaths[0].knots[1].smooth, "retained smooth handles are unchanged");
}

#[test]
fn pen_undo_of_first_anchor_cancels_empty_draft() {
    let mut app = app();
    assert!(!pen_undo_last_point(&mut app));
    pen_down(&mut app, 15.0, 17.0, false);
    pen_up(&mut app);
    assert!(pen_undo_last_point(&mut app));
    assert!(app.ui.pen.is_none());
    assert!(!pen_undo_last_point(&mut app));
    assert_eq!(app.session.active().unwrap().history.past_len(), 0);
    assert!(app.session.active().unwrap().doc.work_path.is_none());
}

/// #1419: Solid Color (and every new fill or adjustment layer) made while a path is selected
/// in the Paths panel takes it as its vector mask; with the path deselected, the selection
/// is its layer mask again.
#[test]
fn new_fill_layer_takes_the_selected_path_as_its_vector_mask() {
    let mut app = app();
    let layer = |app: &PhotocraftApp, r: Result<Value, String>| {
        let id = photocraft_doc::LayerId(r.unwrap()["layer"].as_u64().unwrap());
        app.session.active().unwrap().doc.layer(id).unwrap().clone()
    };
    // Drawing a work path with the Pen selects it.
    app.ui.tool = Tool::Pen;
    for (x, y) in [(20.0, 20.0), (120.0, 20.0), (120.0, 120.0)] {
        pen_down(&mut app, x, y, false);
        pen_up(&mut app);
    }
    pen_down(&mut app, 20.5, 20.5, false);
    pen_up(&mut app);
    assert_eq!(app.ui.selected_path.as_deref(), Some("work"));
    let wp = app.session.active().unwrap().doc.work_path.clone().unwrap();
    app.run("select.rect", json!({"x": 150, "y": 150, "width": 20, "height": 20})).unwrap();
    for id in ["layer.newFillLayer.solidColor", "layer.newFillLayer.gradient", "layer.newAdjustmentLayer.invert"] {
        app.ui.selected_path = Some("work".into());
        let r = app.run(id, json!({}));
        let l = layer(&app, r);
        assert_eq!(l.vector_mask.as_ref().map(|v| &v.path), Some(&wp), "{id}: the work path is the vector mask");
        assert!(l.mask.is_none(), "{id}: the path wins over the selection");
        // The new layer's vector mask is now the active path, not the work path.
        assert_eq!(app.ui.selected_path.as_deref(), Some("layer"), "{id}");
    }
    // So the next fill layer isn't masked by the work path again.
    let r = app.run("layer.newFillLayer.solidColor", json!({}));
    assert!(layer(&app, r).vector_mask.is_none());
    // A caller can opt out.
    let r = app.run("layer.newFillLayer.solidColor", json!({"path": null}));
    assert!(layer(&app, r).vector_mask.is_none());
    // A selected saved path is used.
    app.run("path.set", json!({"name": "Saved", "path": {"subpaths": [{"closed": true, "knots": [[0, 0], [9, 0], [9, 9]]}]}})).unwrap();
    app.ui.selected_path = Some("Saved".into());
    let saved = app.session.active().unwrap().doc.paths[0].path.clone();
    let r = app.run("layer.newFillLayer.solidColor", json!({}));
    assert_eq!(layer(&app, r).vector_mask.map(|v| v.path), Some(saved));
    // Deselected (or a stale name): the selection is the layer mask, no vector mask.
    for sel in [None, Some("Gone".to_string())] {
        app.ui.selected_path = sel;
        let r = app.run("layer.newFillLayer.solidColor", json!({}));
        let l = layer(&app, r);
        assert!(l.vector_mask.is_none() && l.mask.is_some());
    }
    // Make Work Path selects the new work path.
    app.run("select.toWorkPath", json!({"tolerance": 2.0})).unwrap();
    assert_eq!(app.ui.selected_path.as_deref(), Some("work"));
}

/// #1482: clicking an existing final Pen anchor breaks its outgoing handle without
/// deleting the preceding curve or creating another knot; dragging changes only that handle.
#[test]
fn pen_last_anchor_can_be_broken_and_reshaped_while_drawing() {
    use crate::canvas::{ToolEvent, tool_event};
    let mut app = app();
    app.ui.tool = Tool::Pen;
    let down = |app: &mut PhotocraftApp, x, y| tool_event(app, ToolEvent::Down { x, y, pressure: 1.0 }, egui::Modifiers::NONE);
    let move_to = |app: &mut PhotocraftApp, x, y| tool_event(app, ToolEvent::Move { x, y, pressure: 1.0 }, egui::Modifiers::NONE);
    let up = |app: &mut PhotocraftApp, x, y| tool_event(app, ToolEvent::Up { x, y }, egui::Modifiers::NONE);

    down(&mut app, 20.0, 20.0);
    up(&mut app, 20.0, 20.0);
    down(&mut app, 100.0, 20.0);
    move_to(&mut app, 120.0, 40.0);
    up(&mut app, 120.0, 40.0);
    let original = app.ui.pen.as_ref().unwrap().knots[1];
    assert_eq!(original, [[100.0, 20.0], [80.0, 0.0], [120.0, 40.0]]);

    // Bare click breaks the outgoing handle, preserving the preceding segment's tangent.
    down(&mut app, 100.0, 20.0);
    up(&mut app, 100.0, 20.0);
    let pen = app.ui.pen.as_ref().unwrap();
    assert_eq!(pen.knots.len(), 2);
    assert_eq!(pen.knots[1], [[100.0, 20.0], [80.0, 0.0], [100.0, 20.0]]);
    assert!(!pen.dragging && !pen.adjusting_last);

    // Re-dragging the same final anchor moves only its outgoing handle (a cusp).
    down(&mut app, 100.0, 20.0);
    move_to(&mut app, 110.0, 60.0);
    up(&mut app, 110.0, 60.0);
    let pen = app.ui.pen.as_ref().unwrap();
    assert_eq!(pen.knots.len(), 2);
    assert_eq!(pen.knots[1], [[100.0, 20.0], [80.0, 0.0], [110.0, 60.0]]);

    // Drawing on still adds a separate next anchor and keeps the cusp in the work path.
    down(&mut app, 150.0, 100.0);
    up(&mut app, 150.0, 100.0);
    assert_eq!(app.ui.pen.as_ref().unwrap().knots.len(), 3);
    pen_commit(&mut app, false);
    let work = app.session.active().unwrap().doc.work_path.as_ref().unwrap();
    let k = &work.subpaths[0].knots[1];
    assert_eq!([k.in_ctrl.x, k.in_ctrl.y], [80.0, 0.0]);
    assert_eq!([k.out_ctrl.x, k.out_ctrl.y], [110.0, 60.0]);
    assert!(!k.smooth, "breaking the handle must persist as an unlinked PSD/vector knot");
}

#[test]
fn pen_last_anchor_hit_tolerance_tracks_zoom_and_draft_round_trips() {
    for zoom in [0.25, 1.0, 4.0] {
        let mut app = app();
        app.ui.views[0].zoom = zoom;
        pen_down(&mut app, 20.0, 20.0, false);
        pen_up(&mut app);
        pen_down(&mut app, 100.0, 100.0, false);
        pen_move(&mut app, 110.0, 120.0, false);
        pen_up(&mut app);
        let incoming = app.ui.pen.as_ref().unwrap().knots[1][1];
        pen_down(&mut app, 100.0 + 5.0 / f64::from(zoom), 100.0, false);
        pen_up(&mut app);
        let pen = app.ui.pen.as_ref().unwrap();
        assert_eq!(pen.knots.len(), 2);
        assert_eq!(pen.knots[1][1], incoming);
        assert_eq!(pen.knots[1][2], pen.knots[1][0]);
        let saved = serde_json::to_value(pen).unwrap();
        let restored: PenPath = serde_json::from_value(saved).unwrap();
        assert_eq!(restored, *pen);
        assert_eq!(pen_to_json(&restored, false)["subpaths"][0]["knots"][1]["smooth"], false);
        pen_down(&mut app, 100.0 + 7.0 / f64::from(zoom), 100.0, false);
        assert_eq!(app.ui.pen.as_ref().unwrap().knots.len(), 3);
    }
    let legacy: PenPath = serde_json::from_value(json!({"knots": [[[10, 10], [10, 10], [10, 10]]]})).unwrap();
    assert!(legacy.unlinked.is_empty());
    assert!(!legacy.dragging && !legacy.adjusting_last);
}

/// #1520: changing one corner through the Properties command must keep the other three
/// independent, remain a live rectangle, and undo/redo as one geometry edit.
#[test]
fn live_rectangle_corner_radius_edit_preserves_other_corners_and_history() {
    let mut app = app();
    let created = app.run("shape.create", json!({"kind": "roundedRect", "rect": [10, 10, 150, 100], "radii": [5, 10, 15, 20]})).unwrap();
    let id = created["layer"].as_u64().unwrap();
    let get_radii = |app: &PhotocraftApp| {
        let st = app.session.active().unwrap();
        let shape = st.doc.layer(photocraft_doc::LayerId(id)).unwrap();
        match &shape.content {
            LayerContent::Shape(sh) => match &sh.live {
                Some(photocraft_doc::vector::LiveShape::Rect { radii, .. }) => *radii,
                _ => panic!("rectangle must remain a live shape"),
            },
            _ => panic!("expected shape layer"),
        }
    };
    assert_eq!(get_radii(&app), [5.0, 10.0, 15.0, 20.0]);
    let mut patch = corner_radii_patch(get_radii(&app), 1, 30.0);
    patch["layer"] = json!(id);
    app.run("shape.edit", patch).unwrap();
    assert_eq!(get_radii(&app), [5.0, 30.0, 15.0, 20.0]);
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(get_radii(&app), [5.0, 10.0, 15.0, 20.0]);
    app.run("edit.redo", json!({})).unwrap();
    assert_eq!(get_radii(&app), [5.0, 30.0, 15.0, 20.0]);
    for corner in 0..4 {
        let before = get_radii(&app);
        let radius = 35.0 + corner as f32;
        let mut expected = before;
        expected[corner] = f64::from(radius);
        let mut patch = corner_radii_patch(before, corner, radius);
        patch["layer"] = json!(id);
        app.run("shape.edit", patch).unwrap();
        assert_eq!(get_radii(&app), expected);
        app.run("edit.undo", json!({})).unwrap();
        assert_eq!(get_radii(&app), before);
        app.run("edit.redo", json!({})).unwrap();
        assert_eq!(get_radii(&app), expected);
    }
    app.run("shape.edit", json!({"layer": id, "radii": 12})).unwrap();
    assert_eq!(get_radii(&app), [12.0; 4]);
}

/// #534: dragging in a shape's fill picker, opened from the Properties panel at the right edge
/// of the window, keeps the picker in place (it flipped from side to side as its width
/// followed the colour readouts, which moved it under the pointer).
#[test]
fn shape_colour_picker_stays_put_while_dragging() {
    use egui::{PointerButton, vec2};
    use egui_kittest::kittest::Queryable;
    let mut app = app();
    let id = app.run("shape.create", json!({"kind": "rect", "rect": [10, 10, 100, 60], "fill": "#3070c0"})).unwrap()["layer"].as_u64().unwrap();
    let mut h = egui_kittest::Harness::builder().with_size(vec2(1440.0, 900.0)).build_ui_state(
        move |ui, app: &mut PhotocraftApp| {
            // Fonts set up after the first frame only apply from the next one.
            if !ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            egui::Panel::right("properties").resizable(false).exact_size(285.0).show(ui, |ui| shape_properties(app, ui, photocraft_doc::LayerId(id)));
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::Pro);
    h.run_steps(4);
    let label = h.query_all(egui_kittest::kittest::by().label("Fill").include_labels()).next().expect("the Fill label").rect();
    // The swatch is right of its label.
    let swatch = egui::pos2(label.right() + 17.0, label.center().y);
    let press = |h: &mut egui_kittest::Harness<'_, PhotocraftApp>, pos, pressed| {
        h.event(egui::Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE });
        h.step();
    };
    h.hover_at(swatch);
    h.step();
    press(&mut h, swatch, true);
    press(&mut h, swatch, false);
    h.run_steps(3);
    let popup = |h: &egui_kittest::Harness<'_, PhotocraftApp>| {
        h.ctx.memory(|m| m.areas().visible_layer_ids().into_iter().filter(|l| l.order == egui::Order::Foreground).find_map(|l| m.area_rect(l.id)))
    };
    let opened = popup(&h).expect("the picker opened");
    let fill = |h: &egui_kittest::Harness<'_, PhotocraftApp>| {
        let st = h.state().session.active().unwrap();
        match &st.doc.layer(photocraft_doc::LayerId(id)).unwrap().content {
            LayerContent::Shape(sh) => sh.fill.clone(),
            _ => None,
        }
    };
    let before = fill(&h);
    // Drag across the picker's colour area.
    let start = opened.min + vec2(60.0, opened.height() * 0.5);
    h.hover_at(start);
    h.step();
    press(&mut h, start, true);
    for i in 1..=10 {
        h.event(egui::Event::PointerMoved(start + vec2(i as f32 * 5.0, i as f32)));
        h.step();
        let r = popup(&h).expect("the picker stays open");
        assert_eq!(r.left_top(), opened.left_top(), "step {i}: the picker moved from {opened:?} to {r:?}");
        assert!(h.ctx.content_rect().contains_rect(r), "step {i}: {r:?} runs off the window");
    }
    press(&mut h, start, false);
    assert_ne!(fill(&h), before, "the drag picked a colour");
}

/// The options bar edits the selected shape's fill and stroke.
#[test]
fn shape_options_bar_edits_the_selected_shape() {
    use egui::accesskit::Role;
    use egui_kittest::kittest::Queryable;
    let mut app = app();
    app.run("tools.setColors", json!({"foreground": "#ff0000", "background": "#0000ff"})).unwrap();
    finish_shape(&mut app, Tool::Rectangle, [20.0, 20.0], [120.0, 100.0], Default::default());
    let shape = |app: &PhotocraftApp| {
        let st = app.session.active().unwrap();
        match &st.doc.layer(st.active_layer.unwrap()).unwrap().content {
            LayerContent::Shape(sh) => (sh.fill.clone(), sh.stroke.clone()),
            _ => panic!("not a shape layer"),
        }
    };
    assert!(shape(&app).0.is_some() && shape(&app).1.is_none());
    let mut h = egui_kittest::Harness::builder().with_size(vec2(1100.0, 48.0)).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            if ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                ui.horizontal_centered(|ui| options_bar(app, ui, Tool::Rectangle));
            }
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::Studio);
    h.run_steps(4);
    let click_fill = |h: &mut egui_kittest::Harness<'_, PhotocraftApp>| {
        let label = h.query_all(egui_kittest::kittest::by().label("Fill:").include_labels()).next().expect("the Fill label").rect();
        let at = egui::pos2(label.right() + h.ctx.global_style().spacing.item_spacing.x + 7.0, label.center().y);
        h.hover_at(at);
        h.step();
        for pressed in [true, false] {
            h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE });
            h.step();
        }
        h.run_steps(2);
    };
    // Stroke width.
    h.get_all_by_role(Role::SpinButton).next().expect("the stroke width").click();
    h.run();
    for c in "6".chars() {
        h.event(egui::Event::Text(c.to_string()));
        h.run();
    }
    h.key_press(egui::Key::Enter);
    h.run_steps(2);
    let stroke = shape(h.state()).1.expect("the width field stroked the shape");
    assert_eq!(stroke.width, 6.0);
    assert_eq!(stroke.paint, photocraft_doc::Fill::Solid(photocraft_doc::Color::rgba(0.0, 0.0, 1.0, 1.0)));
    // Fill off.
    click_fill(&mut h);
    assert_eq!(shape(h.state()).0, None, "unchecking Fill removed the shape's fill");
    assert!(!h.state().ui.tool_options.shape_fill, "and the next shape has none either");
    h.state_mut().run("edit.undo", json!({})).unwrap();
    assert!(shape(h.state()).0.is_some());
    // No shape selected: no edit.
    h.state_mut().run("layer.new.layer", json!({})).unwrap();
    let history = h.state().session.active().unwrap().history.past_len();
    h.run_steps(2);
    click_fill(&mut h);
    assert_eq!(h.state().session.active().unwrap().history.past_len(), history);
}

/// #3915: Path Selection and Direct Selection options bars show Fill, Stroke, and Radius
/// controls when a shape is selected, and editing them edits the selected shape.
#[test]
fn path_selection_options_bar_edits_the_selected_shape() {
    use egui::accesskit::Role;
    use egui_kittest::kittest::Queryable;
    let mut app = app();
    app.run("tools.setColors", json!({"foreground": "#ff0000", "background": "#0000ff"})).unwrap();
    finish_shape(&mut app, Tool::Rectangle, [20.0, 20.0], [120.0, 100.0], Default::default());
    let shape = |app: &PhotocraftApp| {
        let st = app.session.active().unwrap();
        match &st.doc.layer(st.active_layer.unwrap()).unwrap().content {
            LayerContent::Shape(sh) => (sh.fill.clone(), sh.stroke.clone()),
            _ => panic!("not a shape layer"),
        }
    };
    assert!(shape(&app).0.is_some() && shape(&app).1.is_none());
    let mut h = egui_kittest::Harness::builder().with_size(vec2(1100.0, 48.0)).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            if ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                ui.horizontal_centered(|ui| options_bar(app, ui, Tool::PathSelection));
            }
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::Studio);
    h.run_steps(4);
    let click_fill = |h: &mut egui_kittest::Harness<'_, PhotocraftApp>| {
        let label = h.query_all(egui_kittest::kittest::by().label("Fill:").include_labels()).next().expect("the Fill label").rect();
        let at = egui::pos2(label.right() + h.ctx.global_style().spacing.item_spacing.x + 7.0, label.center().y);
        h.hover_at(at);
        h.step();
        for pressed in [true, false] {
            h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE });
            h.step();
        }
        h.run_steps(2);
    };
    // Stroke width field is available on Path Selection bar too.
    h.get_all_by_role(Role::SpinButton).next().expect("the stroke width").click();
    h.run();
    for c in "6".chars() {
        h.event(egui::Event::Text(c.to_string()));
        h.run();
    }
    h.key_press(egui::Key::Enter);
    h.run_steps(2);
    let stroke = shape(h.state()).1.expect("the width field stroked the shape");
    assert_eq!(stroke.width, 6.0);
    assert_eq!(stroke.paint, photocraft_doc::Fill::Solid(photocraft_doc::Color::rgba(0.0, 0.0, 1.0, 1.0)));
    // Fill toggle.
    click_fill(&mut h);
    assert_eq!(shape(h.state()).0, None, "unchecking Fill removed the shape's fill");
    // No shape selected: renders hint text instead.
    h.state_mut().run("layer.new.layer", json!({})).unwrap();
    h.run_steps(2);
    assert!(h.query_all(egui_kittest::kittest::by().label("Fill:").include_labels()).next().is_none());
}

/// #6553: a Paths row dragged onto the footer's trash is deleted, whichever row is selected,
/// and a shape layer's path row does not start a drag.
#[test]
fn path_rows_dragged_onto_the_trash_are_deleted() {
    use egui_kittest::kittest::Queryable;
    let mut app = app();
    let path = json!({"subpaths": [{"closed": true, "knots": [
        {"anchor": [10, 10], "in": [10, 10], "out": [10, 10]},
        {"anchor": [80, 10], "in": [80, 10], "out": [80, 10]},
        {"anchor": [80, 80], "in": [80, 80], "out": [80, 80]}
    ]}]});
    app.run("path.set", json!({"name": "Path 1", "path": path})).unwrap();
    app.run("path.set", json!({"name": "work", "path": path})).unwrap();
    app.ui.selected_path = Some("work".into());
    let mut h = egui_kittest::Harness::builder().with_size(vec2(300.0, 400.0)).build_ui_state(|ui, app: &mut PhotocraftApp| paths_panel(app, ui), app);
    h.run_steps(3);
    let trash = |h: &egui_kittest::Harness<'_, PhotocraftApp>| {
        let buttons: Vec<Rect> = h
            .ctx
            .viewport(|v| v.prev_pass.widgets.layers().flat_map(|(_, w)| w.iter()).filter(|w| w.rect.size() == vec2(26.0, 26.0)).map(|w| w.rect).collect());
        buttons.into_iter().max_by(|a, b| a.center().x.total_cmp(&b.center().x)).unwrap().center()
    };
    let drag = |h: &mut egui_kittest::Harness<'_, PhotocraftApp>, from: Pos2, to: Pos2| {
        h.event(egui::Event::PointerMoved(from));
        h.event(egui::Event::PointerButton { pos: from, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE });
        h.step();
        for k in 1..=10 {
            h.event(egui::Event::PointerMoved(from + (to - from) * (k as f32 / 10.0)));
            h.step();
        }
        h.event(egui::Event::PointerButton { pos: to, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE });
        h.run_steps(3);
    };
    // The saved path, while the work path is selected: only the dragged row goes.
    let (row, to) = (h.get_by_label("Path 1").rect().center(), trash(&h));
    drag(&mut h, row, to);
    let doc = &h.state().session.active().unwrap().doc;
    assert!(doc.paths.is_empty(), "the dragged saved path is deleted");
    assert!(doc.work_path.is_some(), "the selected work path is kept");
    // The work path.
    let (row, to) = (h.get_by_label("Work Path").rect().center(), trash(&h));
    drag(&mut h, row, to);
    assert!(h.state().session.active().unwrap().doc.work_path.is_none(), "the dragged work path is deleted");
    // A drop elsewhere deletes nothing.
    h.state_mut().run("path.set", json!({"name": "Path 2", "path": path})).unwrap();
    h.run_steps(2);
    let row = h.get_by_label("Path 2").rect().center();
    drag(&mut h, row, row + vec2(0.0, 120.0));
    assert_eq!(h.state().session.active().unwrap().doc.paths.len(), 1, "a drop off the trash keeps the path");
    // A shape layer's path row is not dragged onto the trash (its deletion is #6552's).
    h.state_mut().run("shape.create", json!({"kind": "ellipse", "rect": [20, 20, 100, 70]})).unwrap();
    h.run_steps(2);
    let layers = h.state().session.active().unwrap().doc.layers.len();
    let (row, to) = (h.get_by_label("Ellipse 1 Shape Path").rect().center(), trash(&h));
    drag(&mut h, row, to);
    let doc = &h.state().session.active().unwrap().doc;
    assert_eq!((doc.layers.len(), doc.paths.len()), (layers, 1), "nothing is deleted");
}
