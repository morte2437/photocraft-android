use super::*;
use serde_json::json;

/// #7538: as in Photoshop, the box frames a loose selection until its pixels are moved, then
/// the pixels themselves, while they float and once dropped.
#[test]
fn transform_controls_frame_the_moved_pixels_not_the_marquee() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 200, "height": 200, "background": "transparent"})).unwrap();
    app.session.execute("select.rect", json!({"x": 40, "y": 40, "width": 50, "height": 40})).unwrap();
    app.session.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
    app.session.execute("select.rect", json!({"x": 20, "y": 20, "width": 100, "height": 90})).unwrap();
    app.sync_views();
    app.ui.tool = Tool::Move;
    app.ui.tool_options.move_show_transform = true;
    let xf =
        ViewXform { rect: Rect::from_min_size(Pos2::ZERO, vec2(200.0, 200.0)), zoom: 1.0, center: [100.0, 100.0], flip: false, rotation: 0.0, aspect: 1.0 };
    let frame = |app: &PhotocraftApp| {
        let r = transform_controls_rect(app, &xf).unwrap();
        let (lt, rb) = (xf.to_doc(r.left_top()), xf.to_doc(r.right_bottom()));
        [lt[0], lt[1], rb[0], rb[1]].map(|v| v.round() as i32)
    };
    assert_eq!(frame(&app), [20, 20, 120, 110], "the selection, before anything moves");
    app.run("select.float", json!({"dx": 10, "dy": 5})).unwrap();
    assert_eq!(frame(&app), [50, 45, 100, 85], "the floating pixels");
    app.run("select.drop", json!({})).unwrap();
    assert_eq!(frame(&app), [50, 45, 100, 85], "dropped");
}

/// #7538: with several layers selected the Move tool moves them whole, and the selection goes
/// with them, as in Photoshop: its ants follow the drag and it lands with them in one step.
#[test]
fn moving_several_layers_carries_the_selection() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 200, "height": 200, "background": "transparent"})).unwrap();
    let a = app.session.active().unwrap().active_layer.unwrap();
    app.session.execute("select.rect", json!({"x": 40, "y": 40, "width": 50, "height": 40})).unwrap();
    app.session.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
    app.session.execute("layer.new.layer", json!({})).unwrap();
    app.session.execute("select.rect", json!({"x": 120, "y": 120, "width": 30, "height": 30})).unwrap();
    app.session.execute("edit.fill", json!({"color": "#0000ff"})).unwrap();
    app.session.execute("layer.select", json!({"layer": a.0, "mode": "add"})).unwrap();
    app.session.execute("select.rect", json!({"x": 20, "y": 20, "width": 100, "height": 90})).unwrap();
    app.sync_views();
    app.ui.tool = Tool::Move;
    app.ui.tool_options.move_auto_select = false;
    app.ui.extras.snap = false;
    app.ui.view.show.smart_guides = false;
    let steps = app.session.active().unwrap().history.past_len();
    let m = egui::Modifiers::NONE;
    tool_event(&mut app, ToolEvent::Down { x: 60.0, y: 60.0, pressure: 1.0 }, m);
    tool_event(&mut app, ToolEvent::Move { x: 72.0, y: 68.0, pressure: 1.0 }, m);
    assert_eq!(selection_shown_offset(&app), Some((12, 8)), "the ants follow the layers");
    tool_event(&mut app, ToolEvent::Up { x: 72.0, y: 68.0 }, m);
    let st = app.session.active().unwrap();
    assert!(photocraft_engine::float_cmds::floating(st).is_none(), "whole layers moved, nothing floats");
    assert_eq!(st.doc.selection.as_ref().map(|s| s.content_bounds()), Some(photocraft_geom::Rect::new(32, 28, 132, 118)));
    assert_eq!(st.doc.layer(a).unwrap().surface().unwrap().content_bounds(), photocraft_geom::Rect::new(52, 48, 102, 88));
    assert_eq!(st.history.past_len(), steps + 1, "one Move step");
    assert!(app.session.undo());
    let sel = app.session.active().unwrap().doc.selection.as_ref().map(|s| s.content_bounds());
    assert_eq!(sel, Some(photocraft_geom::Rect::new(20, 20, 120, 110)), "Undo takes the selection back too");

    // A piece floating (a marquee ⌘-drag with both selected) when the layers are dragged: the
    // press drops it first, so the ants follow the drag from where it was dropped.
    app.run("select.float", json!({"dx": 10, "dy": 5})).unwrap();
    let outline = photocraft_engine::float_cmds::floating(app.session.active().unwrap()).unwrap().parts.outline().unwrap().content_bounds();
    tool_event(&mut app, ToolEvent::Down { x: 60.0, y: 60.0, pressure: 1.0 }, m);
    let st = app.session.active().unwrap();
    assert!(photocraft_engine::float_cmds::floating(st).is_none(), "the press dropped the piece");
    assert_eq!(st.doc.selection.as_ref().map(|s| s.content_bounds()), Some(outline.translate(10, 5)), "at its offset");
    tool_event(&mut app, ToolEvent::Move { x: 72.0, y: 68.0, pressure: 1.0 }, m);
    assert_eq!(selection_shown_offset(&app), Some((12, 8)), "then the ants follow the drag");
    tool_event(&mut app, ToolEvent::Up { x: 72.0, y: 68.0 }, m);
    let sel = app.session.active().unwrap().doc.selection.as_ref().map(|s| s.content_bounds());
    assert_eq!(sel, Some(outline.translate(22, 13)), "released where the ants were");
}

/// A transparent 200² document: a red square (40..90, 40..80) on its layer, a loose marquee
/// (20..120, 20..110) round it, cut and floated 10, 5 to the right and down.
fn floating_square() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 200, "height": 200, "background": "transparent"})).unwrap();
    app.session.execute("select.rect", json!({"x": 40, "y": 40, "width": 50, "height": 40})).unwrap();
    app.session.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
    app.session.execute("select.rect", json!({"x": 20, "y": 20, "width": 100, "height": 90})).unwrap();
    app.sync_views();
    app.session.execute("select.float", json!({"dx": 10, "dy": 5})).unwrap();
    app
}

/// #7538: a selection tool's press on a floating piece's ants drags it; one in the margin its
/// cut left of the loose marquee draws a new selection, as the ants say.
#[test]
fn a_press_in_the_margin_of_a_floating_piece_starts_a_new_selection() {
    let app = floating_square();
    let none = egui::Modifiers::NONE;
    for tool in [Tool::RectMarquee, Tool::Lasso] {
        assert_eq!(selection_drag_kind(&app, tool, [70.0, 60.0], none), Some(true), "{tool:?}: on the square drags it");
        assert_eq!(selection_drag_kind(&app, tool, [33.0, 28.0], none), None, "{tool:?}: in the old marquee's margin");
    }
}

/// Several selected layers keep one frame round them all while the active one's pixels float:
/// the piece's outline frames it only when it is the only layer framed.
#[test]
fn a_floating_piece_among_several_selected_layers_keeps_the_shared_frame() {
    let mut app = floating_square();
    app.session.execute("select.drop", json!({})).unwrap();
    let a = app.session.active().unwrap().active_layer.unwrap();
    app.session.execute("layer.new.layer", json!({})).unwrap();
    app.session.execute("select.rect", json!({"x": 150, "y": 150, "width": 20, "height": 20})).unwrap();
    app.session.execute("edit.fill", json!({"color": "#0000ff"})).unwrap();
    app.session.execute("layer.select", json!({"layer": a.0, "mode": "add"})).unwrap();
    app.session.execute("select.rect", json!({"x": 20, "y": 20, "width": 100, "height": 90})).unwrap();
    app.ui.tool = Tool::Move;
    app.ui.tool_options.move_show_transform = true;
    assert!(transform_controls_several(&app), "two layers framed");
    app.session.execute("select.float", json!({"dx": 5, "dy": 0})).unwrap();
    assert!(photocraft_engine::float_cmds::floating(app.session.active().unwrap()).is_some());
    let xf =
        ViewXform { rect: Rect::from_min_size(Pos2::ZERO, vec2(200.0, 200.0)), zoom: 1.0, center: [100.0, 100.0], flip: false, rotation: 0.0, aspect: 1.0 };
    let r = transform_controls_rect(&app, &xf).unwrap();
    let (lt, rb) = (xf.to_doc(r.left_top()), xf.to_doc(r.right_bottom()));
    assert!(rb[0] - lt[0] > 60.0 && rb[1] - lt[1] > 60.0, "the frame isn't shrunk to the one piece: {lt:?} {rb:?}");
}

#[test]
fn several_selected_layers_share_one_frame_without_handles() {
    // As in Photoshop: Show Transform Controls frames every selected layer. Free Transform
    // works on one layer, so the shared frame doesn't start it.
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 200, "height": 200})).unwrap();
    app.session.execute("shape.create", json!({"kind": "rect", "rect": [20, 30, 40, 40], "fill": "#ff0000"})).unwrap();
    let a = app.session.active().unwrap().active_layer.unwrap();
    app.session.execute("shape.create", json!({"kind": "rect", "rect": [100, 120, 50, 30], "fill": "#ff0000"})).unwrap();
    app.sync_views();
    app.ui.tool = Tool::Move;
    app.ui.tool_options.move_show_transform = true;
    let xf =
        ViewXform { rect: Rect::from_min_size(Pos2::ZERO, vec2(200.0, 200.0)), zoom: 1.0, center: [100.0, 100.0], flip: false, rotation: 0.0, aspect: 1.0 };
    let one = transform_controls_rect(&app, &xf).unwrap();
    assert!(!transform_controls_several(&app));
    app.session.execute("layer.select", json!({"layer": a.0, "mode": "add"})).unwrap();
    assert!(transform_controls_several(&app));
    let both = transform_controls_rect(&app, &xf).unwrap();
    assert!(both.contains_rect(one) && both.width() > one.width() && both.height() > one.height(), "{both:?} frames both");
    let (lt, rb) = (xf.to_doc(both.left_top()), xf.to_doc(both.right_bottom()));
    assert!((lt[0] - 20.0).abs() < 1e-3 && (lt[1] - 30.0).abs() < 1e-3 && (rb[0] - 150.0).abs() < 1e-3 && (rb[1] - 150.0).abs() < 1e-3, "{lt:?} {rb:?}");
    let ctx = egui::Context::default();
    assert!(!begin_transform_controls_at(&mut app, &ctx, &xf, both.right_bottom()));
    assert!(app.ui.transform.is_none());
    // The Background never joins the frame.
    let bg = app.session.active().unwrap().doc.layers[0].id;
    app.session.execute("layer.select", json!({"layer": bg.0, "mode": "add"})).unwrap();
    assert_eq!(transform_controls_layers(&app).len(), 2);
}

#[test]
fn move_transform_controls_start_free_transform_for_vector_shapes() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 200, "height": 200})).unwrap();
    app.session.execute("shape.create", json!({"kind": "rect", "rect": [20, 30, 80, 40], "fill": "#ff0000"})).unwrap();
    app.sync_views();
    app.ui.tool = Tool::Move;
    app.ui.tool_options.move_show_transform = true;

    let xf =
        ViewXform { rect: Rect::from_min_size(Pos2::ZERO, vec2(200.0, 200.0)), zoom: 1.0, center: [100.0, 100.0], flip: false, rotation: 0.0, aspect: 1.0 };
    let r = transform_controls_rect(&app, &xf).expect("shape layers have transform bounds");
    assert!(transform_controls_hit(r, r.right_bottom()));

    let ctx = egui::Context::default();
    assert!(begin_transform_controls_at(&mut app, &ctx, &xf, r.right_bottom()));
    let t = app.ui.transform.as_ref().expect("control press starts Free Transform");
    assert_eq!(t.rect, [20.0, 30.0, 100.0, 70.0]);
}

#[test]
fn hiding_extras_hides_the_move_transform_controls() {
    // #6699: as in Photoshop, View › Extras (⌘H) hides the Move tool's transform box and
    // handles; again brings them back. Show Transform Controls and the layer stay as they were.
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 200, "height": 200})).unwrap();
    app.session.execute("shape.create", json!({"kind": "rect", "rect": [20, 30, 80, 40], "fill": "#ff0000"})).unwrap();
    app.sync_views();
    app.ui.tool = Tool::Move;
    app.ui.tool_options.move_show_transform = true;
    let xf =
        ViewXform { rect: Rect::from_min_size(Pos2::ZERO, vec2(200.0, 200.0)), zoom: 1.0, center: [100.0, 100.0], flip: false, rotation: 0.0, aspect: 1.0 };
    let ctx = egui::Context::default();
    let shown = transform_controls_rect(&app, &xf).expect("the box shows with Extras on");
    let layer = app.session.active().unwrap().doc.layers.clone();
    let painted = |app: &mut PhotocraftApp| {
        let out = ctx.run_ui(egui::RawInput::default(), |ui| {
            let painter = ui.ctx().layer_painter(egui::LayerId::background());
            draw_transform_controls(app, &painter, &xf);
        });
        let egui::FullOutput { mut textures_delta, shapes, .. } = out;
        textures_delta.clear();
        shapes.len()
    };
    // The shape's outline, the box and its eight handles.
    assert!(painted(&mut app) > 8, "the box and handles are drawn");

    crate::menus::invoke(&mut app, &ctx, "view.extras", json!({})).unwrap();
    assert!(!app.ui.view.extras);
    assert!(transform_controls_rect(&app, &xf).is_none(), "no box with Extras hidden");
    assert_eq!(painted(&mut app), 0, "nothing is drawn");
    // Hidden handles don't act: a press on a corner doesn't start Free Transform.
    assert!(!begin_transform_controls_at(&mut app, &ctx, &xf, shown.right_bottom()));
    assert!(app.ui.transform.is_none());
    assert!(app.ui.tool_options.move_show_transform, "Show Transform Controls is unchanged");
    assert_eq!(app.session.active().unwrap().doc.layers, layer, "the layer is unchanged");

    crate::menus::invoke(&mut app, &ctx, "view.extras", json!({})).unwrap();
    assert!(app.ui.view.extras);
    assert_eq!(transform_controls_rect(&app, &xf), Some(shown), "⌘H again restores the box");
}

#[test]
fn move_transform_controls_follow_floating_selected_pixels() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("select.rect", json!({"x": 100, "y": 100, "width": 100, "height": 60})).unwrap();
    app.run("edit.fill", json!({"color": "#ff0000"})).unwrap();
    app.ui.tool = Tool::Move;
    app.ui.tool_options.move_show_transform = true;
    assert!(crate::move_ui::float_selected(&mut app, false, 100.0, 50.0));
    let xf =
        ViewXform { rect: Rect::from_min_size(Pos2::ZERO, vec2(400.0, 300.0)), zoom: 1.0, center: [200.0, 150.0], flip: false, rotation: 0.0, aspect: 1.0 };
    let r = transform_controls_rect(&app, &xf).unwrap();
    assert_eq!(r, Rect::from_min_max(pos2(200.0, 150.0), pos2(300.0, 210.0)));
    let ctx = egui::Context::default();
    assert!(begin_transform_controls_at(&mut app, &ctx, &xf, r.right_bottom()));
    assert_eq!(app.ui.transform.as_ref().unwrap().rect, [200.0, 150.0, 300.0, 210.0]);
}

/// #5297: with a selection the Move tool's transform controls are the selection's rectangle,
/// though the layer fills only part of it, and a press on them opens that box.
#[test]
fn move_transform_controls_frame_the_selection() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("select.rect", json!({"x": 100, "y": 100, "width": 100, "height": 60})).unwrap();
    app.run("edit.fill", json!({"color": "#ff0000"})).unwrap();
    app.run("select.rect", json!({"x": 60, "y": 80, "width": 100, "height": 120})).unwrap();
    app.ui.tool = Tool::Move;
    app.ui.tool_options.move_show_transform = true;
    let xf =
        ViewXform { rect: Rect::from_min_size(Pos2::ZERO, vec2(400.0, 300.0)), zoom: 1.0, center: [200.0, 150.0], flip: false, rotation: 0.0, aspect: 1.0 };
    let r = transform_controls_rect(&app, &xf).unwrap();
    assert_eq!(r, Rect::from_min_max(pos2(60.0, 80.0), pos2(160.0, 200.0)));
    let ctx = egui::Context::default();
    assert!(begin_transform_controls_at(&mut app, &ctx, &xf, r.right_bottom()));
    assert_eq!(app.ui.transform.as_ref().unwrap().rect, [60.0, 80.0, 160.0, 200.0]);
}

/// #2585: Free Transform on a position-locked layer is refused before the box opens, with
/// Photoshop's alert, from ⌘T and from the Move tool's transform controls alike. It used to
/// open, preview every drag and fail only at commit.
#[test]
fn free_transform_on_a_locked_layer_is_refused_up_front() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 200, "height": 200})).unwrap();
    app.session.execute("shape.create", json!({"kind": "rect", "rect": [20, 30, 80, 40], "fill": "#ff0000"})).unwrap();
    app.session
        .edit("lock", |doc, active| {
            doc.layer_mut(active.unwrap()).unwrap().locks.position = true;
            Ok(())
        })
        .unwrap();
    app.sync_views();
    let errors = |app: &PhotocraftApp| -> Vec<String> {
        app.ui
            .dialogs
            .iter()
            .filter(|d| d.kind == crate::state::DialogKind::Error)
            .filter_map(|d| d.fields.get("message")?.as_str().map(String::from))
            .collect()
    };
    let ctx = egui::Context::default();
    // ⌘T.
    let r = crate::menus::invoke(&mut app, &ctx, "edit.freeTransform", json!({}));
    assert_eq!(r, Err(crate::transform_tool::LOCKED.to_string()));
    assert!(app.ui.transform.is_none() && app.transform_preview.is_none(), "no box opened");
    assert_eq!(errors(&app), [crate::transform_tool::LOCKED]);
    app.ui.dialogs.clear();
    // The Move tool's transform controls.
    app.ui.tool = Tool::Move;
    app.ui.tool_options.move_show_transform = true;
    let xf =
        ViewXform { rect: Rect::from_min_size(Pos2::ZERO, vec2(200.0, 200.0)), zoom: 1.0, center: [100.0, 100.0], flip: false, rotation: 0.0, aspect: 1.0 };
    let r = transform_controls_rect(&app, &xf).unwrap();
    assert!(!begin_transform_controls_at(&mut app, &ctx, &xf, r.right_bottom()));
    assert!(app.ui.transform.is_none());
    assert_eq!(errors(&app), [crate::transform_tool::LOCKED]);
}

#[test]
fn move_transform_controls_show_resize_and_rotate_cursors_before_first_interaction() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 200, "height": 200})).unwrap();
    app.session.execute("shape.create", json!({"kind": "rect", "rect": [20, 30, 80, 40], "fill": "#ff0000"})).unwrap();
    app.sync_views();
    app.ui.tool = Tool::Move;
    app.ui.tool_options.move_show_transform = true;
    let xf =
        ViewXform { rect: Rect::from_min_size(Pos2::ZERO, vec2(200.0, 200.0)), zoom: 1.0, center: [100.0, 100.0], flip: false, rotation: 0.0, aspect: 1.0 };

    let r = transform_controls_rect(&app, &xf).expect("visible shape controls");
    assert!(app.ui.transform.is_none(), "no transform session is needed for hover");
    for (p, expected) in [
        (r.left_top(), egui::CursorIcon::ResizeNwSe),
        (r.right_bottom(), egui::CursorIcon::ResizeNwSe),
        (r.right_top(), egui::CursorIcon::ResizeNeSw),
        (r.left_bottom(), egui::CursorIcon::ResizeNeSw),
        (r.center_top(), egui::CursorIcon::ResizeVertical),
        (r.center_bottom(), egui::CursorIcon::ResizeVertical),
        (r.right_center(), egui::CursorIcon::ResizeHorizontal),
        (r.left_center(), egui::CursorIcon::ResizeHorizontal),
        // Between handles: the rotation band (a point above the top-centre handle is that handle),
        // where the curved rotate arrow replaces the pointer (#4658).
        (pos2(r.left() + r.width() * 0.25, r.top() - 12.0), egui::CursorIcon::None),
    ] {
        assert_eq!(transform_controls_cursor(r, p), Some(expected), "hover at {p:?}");
        assert!(transform_controls_hit(r, p), "same region must start a transform");
    }
    assert_eq!(transform_controls_cursor(r, r.center()), None, "interior stays Move");
    assert_eq!(transform_controls_cursor(r, pos2(r.center().x, r.top() - 30.0)), None);
    // The rotate arrow bends round the nearest corner, pointing into the box; handles and the
    // interior have none.
    let into = |p: Pos2| transform_controls_turn(r, p).map(|v| (v.x.signum(), v.y.signum()));
    assert_eq!(into(pos2(r.left() + r.width() * 0.25, r.top() - 12.0)), Some((1.0, 1.0)), "top left");
    assert_eq!(into(pos2(r.right() + 12.0, r.bottom() - r.height() * 0.25)), Some((-1.0, -1.0)), "bottom right");
    assert_eq!(into(pos2(r.left() - 12.0, r.bottom() + 12.0)), Some((1.0, -1.0)), "beyond the bottom-left corner");
    assert_eq!(into(r.left_top()), None);
    assert_eq!(into(r.center()), None);

    // Changing the active shape changes the hover bounds without requiring a drag first.
    app.session.execute("shape.create", json!({"kind": "rect", "rect": [120, 100, 40, 50], "fill": "#00ff00"})).unwrap();
    let next = transform_controls_rect(&app, &xf).expect("new active shape controls");
    assert_ne!(r, next);
    assert_eq!(transform_controls_cursor(next, next.right_bottom()), Some(egui::CursorIcon::ResizeNwSe));
    assert_eq!(transform_controls_cursor(next, next.center()), None);
}

#[test]
fn transform_control_hit_keeps_move_interior_free_and_has_rotation_band() {
    let r = Rect::from_min_max(pos2(20.0, 30.0), pos2(100.0, 70.0));
    assert!(transform_controls_hit(r, r.left_top()));
    assert!(transform_controls_hit(r, pos2(60.0, 18.0)));
    assert!(!transform_controls_hit(r, r.center()));
    assert!(!transform_controls_hit(r, pos2(60.0, 4.0)));
}
