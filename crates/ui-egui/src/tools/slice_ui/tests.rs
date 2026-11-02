use super::*;

fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 100, "height": 80})).unwrap();
    app
}

#[test]
fn slice_tool_draws_and_select_tool_moves() {
    let mut app = app();
    app.ui.tool = Tool::Slice;
    let m = egui::Modifiers::NONE;
    assert!(pointer(&mut app, ToolEvent::Down { x: 10.0, y: 10.0, pressure: 1.0 }, m));
    pointer(&mut app, ToolEvent::Move { x: 40.0, y: 30.0, pressure: 1.0 }, m);
    pointer(&mut app, ToolEvent::Up { x: 40.0, y: 30.0 }, m);
    let doc = app.session.active().unwrap().doc.clone();
    assert_eq!(doc.slices.list.len(), 1);
    assert_eq!(doc.slices.list[0].rect, photocraft_geom::Rect::new(10, 10, 40, 30));
    let n = app.ui.slices.selected.unwrap();
    // Slice Select: drag it by (5, 5).
    app.ui.tool = Tool::SliceSelect;
    pointer(&mut app, ToolEvent::Down { x: 20.0, y: 20.0, pressure: 1.0 }, m);
    assert_eq!(app.ui.slices.selected, Some(n));
    pointer(&mut app, ToolEvent::Up { x: 25.0, y: 25.0 }, m);
    let doc = app.session.active().unwrap().doc.clone();
    assert_eq!(doc.slices.list[0].rect, photocraft_geom::Rect::new(15, 15, 45, 35));
    // Clicking an auto slice selects it; Slice Options promotes it.
    pointer(&mut app, ToolEvent::Down { x: 90.0, y: 75.0, pressure: 1.0 }, m);
    pointer(&mut app, ToolEvent::Up { x: 90.0, y: 75.0 }, m);
    let r = open_options(&mut app).unwrap();
    let id = r["dialog"].as_u64().unwrap();
    app.ui.dialog_mut(id).unwrap().fields.insert("name".into(), json!("footer"));
    crate::dialogs::confirm(&mut app, id).unwrap();
    let doc = app.session.active().unwrap().doc.clone();
    assert!(doc.slices.list.iter().any(|s| s.name == "footer"));
}

/// #7865: with View › Lock Slices on, Slice Select doesn't select (or move) a slice, and the
/// Slice tool doesn't start one; both say why, as Photoshop's error does.
#[test]
fn locked_slices_cannot_be_selected() {
    let mut app = app();
    let m = egui::Modifiers::NONE;
    app.ui.tool = Tool::Slice;
    pointer(&mut app, ToolEvent::Down { x: 10.0, y: 10.0, pressure: 1.0 }, m);
    pointer(&mut app, ToolEvent::Up { x: 40.0, y: 30.0 }, m);
    let rect = app.session.active().unwrap().doc.slices.list[0].rect;
    app.ui.slices.selected = None;
    app.run("view.lockSlices", json!({"on": true})).unwrap();
    app.ui.tool = Tool::SliceSelect;
    pointer(&mut app, ToolEvent::Down { x: 20.0, y: 20.0, pressure: 1.0 }, m);
    pointer(&mut app, ToolEvent::Move { x: 30.0, y: 30.0, pressure: 1.0 }, m);
    pointer(&mut app, ToolEvent::Up { x: 30.0, y: 30.0 }, m);
    assert_eq!(app.ui.slices.selected, None, "not selected");
    assert_eq!(app.session.active().unwrap().doc.slices.list[0].rect, rect, "not moved");
    assert!(app.ui.status_error && app.ui.status.contains("locked"), "{}", app.ui.status);
    // The Slice tool says so too.
    app.ui.status_error = false;
    app.ui.tool = Tool::Slice;
    pointer(&mut app, ToolEvent::Down { x: 50.0, y: 50.0, pressure: 1.0 }, m);
    pointer(&mut app, ToolEvent::Up { x: 80.0, y: 70.0 }, m);
    assert_eq!(app.session.active().unwrap().doc.slices.list.len(), 1);
    assert!(app.ui.status_error && app.ui.status.contains("locked"), "{}", app.ui.status);
    // Unlocked again, Slice Select selects it.
    app.run("view.lockSlices", json!({"on": false})).unwrap();
    app.ui.tool = Tool::SliceSelect;
    pointer(&mut app, ToolEvent::Down { x: 20.0, y: 20.0, pressure: 1.0 }, m);
    pointer(&mut app, ToolEvent::Up { x: 20.0, y: 20.0 }, m);
    assert!(app.ui.slices.selected.is_some());
}

/// #8360: the Slice tool's Style, as the marquee's. Fixed Ratio keeps the drag's larger extent
/// and gives the other the ratio; Fixed Size draws a slice of that size where the drag starts,
/// towards the drag, also on a plain click.
#[test]
fn slice_style_fixes_the_ratio_or_the_size() {
    let mut app = app();
    app.ui.tool = Tool::Slice;
    let m = egui::Modifiers::NONE;
    let draw = |app: &mut PhotocraftApp, a: [f64; 2], b: [f64; 2]| {
        pointer(app, ToolEvent::Down { x: a[0], y: a[1], pressure: 1.0 }, m);
        pointer(app, ToolEvent::Move { x: b[0], y: b[1], pressure: 1.0 }, m);
        pointer(app, ToolEvent::Up { x: b[0], y: b[1] }, m);
        app.session.active().unwrap().doc.slices.list.last().unwrap().rect
    };
    let o = &mut app.ui.tool_options;
    (o.slice_style, o.slice_width, o.slice_height) = ("fixedRatio".into(), 2.0, 1.0);
    assert_eq!(draw(&mut app, [10.0, 10.0], [30.0, 40.0]), photocraft_geom::Rect::new(10, 10, 70, 40));
    let o = &mut app.ui.tool_options;
    (o.slice_style, o.slice_width, o.slice_height) = ("fixedSize".into(), 24.0, 16.0);
    assert_eq!(draw(&mut app, [5.0, 5.0], [5.0, 5.0]), photocraft_geom::Rect::new(5, 5, 29, 21), "a click");
    assert_eq!(draw(&mut app, [90.0, 70.0], [80.0, 60.0]), photocraft_geom::Rect::new(66, 54, 90, 70), "up and to the left");
    // The preview while dragging shows the same rectangle the slice gets.
    pointer(&mut app, ToolEvent::Down { x: 40.0, y: 40.0, pressure: 1.0 }, m);
    pointer(&mut app, ToolEvent::Move { x: 41.0, y: 41.0, pressure: 1.0 }, m);
    let Some(SliceDrag::New { start, cur }) = app.ui.slices.drag.clone() else { panic!("drawing a slice") };
    assert_eq!(new_rect(&app, start, cur), photocraft_geom::Rect::new(40, 40, 64, 56));
    pointer(&mut app, ToolEvent::Up { x: 41.0, y: 41.0 }, m);
    // A press inside a pixel starts the slice on that pixel, still 24 x 16 pixels.
    assert_eq!(draw(&mut app, [10.25, 30.75], [12.5, 31.5]), photocraft_geom::Rect::new(10, 30, 34, 46), "a press inside a pixel");
    assert_eq!(
        draw(&mut app, [70.5, 70.5], [60.25, 60.25]),
        photocraft_geom::Rect::new(47, 55, 71, 71),
        "and towards the upper left, holding the pressed pixel"
    );
    // A short drag up and to the left inside the pressed pixel goes up and to the left.
    assert_eq!(draw(&mut app, [50.75, 50.75], [50.25, 50.25]), photocraft_geom::Rect::new(27, 35, 51, 51), "up-left within the pixel");
    // A Fixed Size that isn't whole pixels (from a settings file) is rounded, not grown a pixel.
    let o = &mut app.ui.tool_options;
    (o.slice_width, o.slice_height) = (1.5, 2.4);
    assert_eq!(draw(&mut app, [5.5, 5.5], [5.5, 5.5]), photocraft_geom::Rect::new(5, 5, 7, 7), "1.5 x 2.4 is 2 x 2");
    // A size of a pixel or two is made too, not taken for a click that selects.
    let o = &mut app.ui.tool_options;
    (o.slice_width, o.slice_height) = (1.0, 2.0);
    assert_eq!(draw(&mut app, [3.0, 3.0], [3.0, 3.0]), photocraft_geom::Rect::new(3, 3, 4, 5), "a 1 x 2 slice");
}

/// #8360: Fixed Ratio's longer side follows a fractional drag to the pixel and the shorter takes
/// the ratio from it, as close as whole pixels allow.
#[test]
fn fixed_ratio_slices_keep_their_ratio_from_a_fractional_drag() {
    let mut app = app();
    app.ui.tool = Tool::Slice;
    let m = egui::Modifiers::NONE;
    let mut draw = |w: f32, h: f32, a: [f64; 2], b: [f64; 2]| {
        let o = &mut app.ui.tool_options;
        (o.slice_style, o.slice_width, o.slice_height) = ("fixedRatio".into(), w, h);
        pointer(&mut app, ToolEvent::Down { x: a[0], y: a[1], pressure: 1.0 }, m);
        pointer(&mut app, ToolEvent::Up { x: b[0], y: b[1] }, m);
        let r = app.session.active().unwrap().doc.slices.list.last().unwrap().rect;
        (r.x0, r.y0, r.width(), r.height())
    };
    assert_eq!(draw(2.0, 1.0, [10.5, 10.5], [14.5, 11.0]), (10, 10, 4, 2), "not 5 x 2");
    assert_eq!(draw(2.0, 1.0, [10.5, 10.5], [31.3, 13.0]), (10, 10, 21, 11), "a 20.8 px drag: 21 px, and half of it as near as pixels allow");
    assert_eq!(draw(16.0, 9.0, [0.5, 0.5], [40.3, 10.0]), (0, 0, 40, 23), "a 39.8 px drag");
    assert_eq!(draw(1366.0, 768.0, [0.0, 0.0], [40.0, 1.0]), (0, 0, 40, 22), "not in steps of 683 px");
    assert_eq!(draw(9.0, 16.0, [0.0, 0.0], [1.0, 32.2]), (0, 0, 18, 32), "the longer side is the height");
    assert_eq!(draw(4.0, 2.0, [60.7, 60.7], [50.2, 58.0]), (50, 55, 11, 6), "towards the upper left, holding the pressed pixel");
    assert_eq!(draw(1.5, 1.0, [0.0, 40.0], [30.0, 41.0]), (0, 40, 30, 20), "a fractional ratio");
    // A click inside a pixel doesn't move the pointer: it selects, as with Normal, and makes no
    // slice (the drag is measured from the press, not from the pixel's corner).
    let n = app.session.active().unwrap().doc.slices.list.len();
    let o = &mut app.ui.tool_options;
    (o.slice_width, o.slice_height) = (4.0, 1.0);
    pointer(&mut app, ToolEvent::Down { x: 10.75, y: 10.75, pressure: 1.0 }, m);
    pointer(&mut app, ToolEvent::Up { x: 10.75, y: 10.75 }, m);
    assert_eq!(app.session.active().unwrap().doc.slices.list.len(), n, "a click makes no slice");
}

/// #8360: remembered Slice tool options are checked when loaded: an unknown Style is Normal
/// again, and Fixed Size is whole pixels.
#[test]
fn remembered_slice_options_are_checked() {
    let mut o = crate::state::ToolOptions::default();
    (o.slice_style, o.slice_width, o.slice_height) = ("fixedsize".into(), 0.5, 2.0);
    crate::tool_memory::sanitize(&mut o);
    assert_eq!(o.slice_style, "normal");
    (o.slice_style, o.slice_width, o.slice_height) = ("fixedSize".into(), 1.5, 63.6);
    crate::tool_memory::sanitize(&mut o);
    assert_eq!((o.slice_width, o.slice_height), (2.0, 64.0));
}

/// #8360: the options bar's Style picks Fixed Size, which starts at 64 × 64 px, and the slice
/// drawn next has that size.
#[test]
fn options_bar_picks_the_slice_style() {
    use egui::accesskit::Role;
    use egui_kittest::Harness;
    use egui_kittest::kittest::{NodeT, Queryable};

    let mut app = app();
    app.sync_views();
    app.ui.tool = Tool::Slice;
    let mut h = Harness::builder().with_size(vec2(1400.0, 200.0)).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            if ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                crate::panels::options_bar(app, ui);
            }
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::Pro);
    h.run_steps(4);
    let fields = |h: &Harness<'static, PhotocraftApp>| {
        h.query_all_by_role(Role::SpinButton)
            .filter(|n| !n.accesskit_node().is_disabled())
            .filter_map(|n| n.accesskit_node().numeric_value())
            .collect::<Vec<_>>()
    };
    assert!(fields(&h).is_empty(), "Normal has no Width or Height to set");
    h.get_all_by_role(Role::ComboBox).find(|n| n.accesskit_node().value().as_deref() == Some("Normal")).unwrap().click();
    h.run_steps(3);
    h.get_by_label("Fixed Size").click();
    h.run_steps(4);
    let o = &h.state().ui.tool_options;
    assert_eq!((o.slice_style.as_str(), o.slice_width, o.slice_height), ("fixedSize", 64.0, 64.0));
    assert_eq!(fields(&h), [64.0, 64.0]);
    let (app, m) = (h.state_mut(), egui::Modifiers::NONE);
    pointer(app, ToolEvent::Down { x: 10.0, y: 10.0, pressure: 1.0 }, m);
    pointer(app, ToolEvent::Up { x: 10.0, y: 10.0 }, m);
    assert_eq!(app.session.active().unwrap().doc.slices.list[0].rect, photocraft_geom::Rect::new(10, 10, 74, 74));
}

#[test]
fn dragging_a_piece_moves_the_whole_covered_slice() {
    // #7770: the slice under a later one is drawn as pieces; dragging one moves all of it.
    let mut app = app();
    app.run("slice.new", json!({"rect": [0, 0, 60, 60]})).unwrap();
    app.run("slice.new", json!({"rect": [40, 40, 20, 20]})).unwrap();
    app.ui.tool = Tool::SliceSelect;
    let m = egui::Modifiers::NONE;
    pointer(&mut app, ToolEvent::Down { x: 10.0, y: 50.0, pressure: 1.0 }, m);
    pointer(&mut app, ToolEvent::Up { x: 15.0, y: 50.0 }, m);
    let doc = app.session.active().unwrap().doc.clone();
    assert_eq!(doc.slices.list[0].rect, photocraft_geom::Rect::new(5, 0, 65, 60));
}

#[test]
fn handles_resize_the_edges_they_hang_off() {
    let r = photocraft_geom::Rect::new(10, 10, 40, 30);
    assert_eq!(handle_at(r, [10.5, 9.0], 2.0), Some([-1, -1]));
    assert_eq!(handle_at(r, [25.0, 30.0], 2.0), Some([0, 1]));
    assert_eq!(handle_at(r, [40.0, 20.0], 2.0), Some([1, 0]));
    assert_eq!(handle_at(r, [25.0, 20.0], 2.0), None, "the middle is no handle");
    assert_eq!(resized(r, [1, 0], 5, 9), photocraft_geom::Rect::new(10, 10, 45, 30), "the right edge only");
    assert_eq!(resized(r, [-1, 1], -4, 6), photocraft_geom::Rect::new(6, 10, 40, 36));
    assert_eq!(resized(r, [-1, 0], 50, 0), photocraft_geom::Rect::new(40, 10, 60, 30), "dragged past the other edge");
    assert_eq!(resized(r, [1, 0], -30, 0), photocraft_geom::Rect::new(10, 10, 11, 30), "never empty");
}

/// #7863: with Slice Select, dragging a handle of the selected slice resizes it; the middle
/// still moves it, and locked slices keep their size.
#[test]
fn slice_select_handles_resize_the_slice() {
    let mut app = app();
    let m = egui::Modifiers::NONE;
    app.ui.tool = Tool::Slice;
    pointer(&mut app, ToolEvent::Down { x: 10.0, y: 10.0, pressure: 1.0 }, m);
    pointer(&mut app, ToolEvent::Up { x: 40.0, y: 30.0 }, m);
    app.ui.tool = Tool::SliceSelect;
    let rect = |app: &PhotocraftApp| app.session.active().unwrap().doc.slices.list[0].rect;
    // The bottom-right corner.
    pointer(&mut app, ToolEvent::Down { x: 40.0, y: 30.0, pressure: 1.0 }, m);
    assert!(matches!(app.ui.slices.drag, Some(SliceDrag::Resize { handle: [1, 1], .. })));
    pointer(&mut app, ToolEvent::Move { x: 50.0, y: 45.0, pressure: 1.0 }, m);
    pointer(&mut app, ToolEvent::Up { x: 50.0, y: 45.0 }, m);
    assert_eq!(rect(&app), photocraft_geom::Rect::new(10, 10, 50, 45));
    assert!(app.ui.slices.selected.is_some(), "still selected");
    // The left edge's middle.
    pointer(&mut app, ToolEvent::Down { x: 10.0, y: 27.0, pressure: 1.0 }, m);
    pointer(&mut app, ToolEvent::Up { x: 4.0, y: 60.0 }, m);
    assert_eq!(rect(&app), photocraft_geom::Rect::new(4, 10, 50, 45), "only the left edge moves");
    // Inside, away from the handles, it still moves.
    pointer(&mut app, ToolEvent::Down { x: 30.0, y: 20.0, pressure: 1.0 }, m);
    pointer(&mut app, ToolEvent::Up { x: 32.0, y: 21.0 }, m);
    assert_eq!(rect(&app), photocraft_geom::Rect::new(6, 11, 52, 46));
    // Locked: the handle doesn't resize.
    app.run("view.lockSlices", json!({"on": true})).unwrap();
    pointer(&mut app, ToolEvent::Down { x: 52.0, y: 46.0, pressure: 1.0 }, m);
    pointer(&mut app, ToolEvent::Up { x: 70.0, y: 70.0 }, m);
    assert_eq!(rect(&app), photocraft_geom::Rect::new(6, 11, 52, 46));
}

/// Delete through the app's shortcut handling, as the canvas gets it.
fn press(app: &mut PhotocraftApp, key: egui::Key) {
    let ctx = egui::Context::default();
    let m = egui::Modifiers::NONE;
    let raw = egui::RawInput {
        events: vec![egui::Event::ModifiersChanged(m), egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: m }],
        ..Default::default()
    };
    ctx.begin_pass(raw);
    crate::shortcuts::handle(app, &ctx);
    ctx.end_pass().textures_delta.clear();
}

/// #7862: with Slice Select, Delete (or Backspace) deletes the selected slice, never the
/// layer: in a one-layer document that used to fail with "a document must keep at least one
/// layer". Locked slices refuse; an auto slice has nothing to delete.
#[test]
fn delete_deletes_the_selected_slice_not_the_layer() {
    let m = egui::Modifiers::NONE;
    for key in [egui::Key::Delete, egui::Key::Backspace] {
        let mut app = app();
        let layers = app.session.active().unwrap().doc.layers.len();
        app.ui.tool = Tool::Slice;
        pointer(&mut app, ToolEvent::Down { x: 10.0, y: 10.0, pressure: 1.0 }, m);
        pointer(&mut app, ToolEvent::Up { x: 40.0, y: 30.0 }, m);
        app.ui.tool = Tool::SliceSelect;
        pointer(&mut app, ToolEvent::Down { x: 20.0, y: 20.0, pressure: 1.0 }, m);
        pointer(&mut app, ToolEvent::Up { x: 20.0, y: 20.0 }, m);
        assert!(app.ui.slices.selected.is_some());

        // Locked: the slice stays, and says why.
        app.run("view.lockSlices", json!({"on": true})).unwrap();
        press(&mut app, key);
        assert_eq!(app.session.active().unwrap().doc.slices.list.len(), 1, "{key:?}: locked");
        assert!(app.ui.status_error && app.ui.status.contains("locked"), "{key:?}: {}", app.ui.status);
        app.run("view.lockSlices", json!({"on": false})).unwrap();
        app.ui.status_error = false;

        press(&mut app, key);
        let doc = app.session.active().unwrap().doc.clone();
        assert!(doc.slices.list.is_empty(), "{key:?}: the slice is deleted");
        assert_eq!(doc.layers.len(), layers, "{key:?}: the layer stays");
        assert!(!app.ui.status_error, "{key:?}: {}", app.ui.status);
        assert_eq!(app.ui.slices.selected, None);

        // An auto slice: nothing to delete, and still not the layer.
        pointer(&mut app, ToolEvent::Down { x: 90.0, y: 75.0, pressure: 1.0 }, m);
        pointer(&mut app, ToolEvent::Up { x: 90.0, y: 75.0 }, m);
        press(&mut app, key);
        assert_eq!(app.session.active().unwrap().doc.layers.len(), layers, "{key:?}: auto slice");
        assert!(!app.ui.status_error, "{key:?}: {}", app.ui.status);
    }
}

#[test]
fn locked_slices_ignore_gestures() {
    let mut app = app();
    app.run("view.lockSlices", json!({"on": true})).unwrap();
    app.ui.tool = Tool::Slice;
    let m = egui::Modifiers::NONE;
    pointer(&mut app, ToolEvent::Down { x: 10.0, y: 10.0, pressure: 1.0 }, m);
    pointer(&mut app, ToolEvent::Up { x: 40.0, y: 30.0 }, m);
    assert!(app.session.active().unwrap().doc.slices.is_empty());
}
