use super::*;

#[test]
fn ruler_ticks_labels_and_pointer_stay_out_of_the_origin_corner() {
    let mut failures = Vec::new();
    for theme in crate::theme::ThemeKind::ALL {
        for (zoom, flip, rotation, width) in [(1.0, false, 0.0, 560.0), (1.0, false, 0.0, 800.0), (0.25, true, 0.0, 560.0), (2.0, false, 30.0, 560.0)] {
            let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
            app.run("file.new", json!({"width": 256, "height": 128})).unwrap();
            let full = Rect::from_min_size(pos2(32.0, 32.0), vec2(width - 80.0, 320.0));
            let corner = Rect::from_min_size(full.min, vec2(RULER, RULER));
            let builder = egui_kittest::Harness::builder().with_size(vec2(width, 400.0));
            let mut h = builder.build_ui_state(
                move |ui, app| {
                    let rect = Rect::from_min_max(full.min + vec2(RULER, RULER), full.max);
                    let xf = ViewXform { rect, zoom, center: [(width - 80.0) / 2.0, 160.0], flip, rotation, aspect: 1.0 };
                    draw_rulers(app, ui, full, &xf);
                },
                app,
            );
            PhotocraftApp::setup_context(&h.ctx, theme);
            h.event(egui::Event::PointerMoved(corner.center()));
            h.run_steps(4);
            let mut ticks = 0;
            let mut labels = 0;
            for shape in &h.output().shapes {
                if matches!(shape.shape, egui::Shape::LineSegment { .. } | egui::Shape::Text(_)) {
                    ticks += usize::from(matches!(shape.shape, egui::Shape::LineSegment { .. }));
                    labels += usize::from(matches!(shape.shape, egui::Shape::Text(_)));
                    let visible = shape.shape.visual_bounding_rect().intersect(shape.clip_rect);
                    if visible.intersect(corner.shrink(0.5)).is_positive() {
                        failures.push(format!("{theme:?}: zoom={zoom}, flip={flip}, rotation={rotation}, {visible:?}"));
                    }
                }
            }
            assert!(ticks > 2 && labels > 0, "rulers still draw ticks and labels");
        }
    }
    assert!(failures.is_empty(), "ruler drawing overlaps the origin corner: {failures:?}");
}

/// #7314: the stacked characters of the left ruler's labels are centred on the strip, so a
/// narrow "1" lines up with a wide "0".
#[test]
fn left_ruler_label_characters_are_centred() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 256, "height": 128})).unwrap();
    let (mut h, full, _) = ruler_harness(app);
    h.run_steps(2);
    let centres: Vec<f32> = h
        .output()
        .shapes
        .iter()
        .filter_map(|s| if let egui::Shape::Text(t) = &s.shape { Some(t) } else { None })
        .filter(|t| t.pos.x < full.min.x + RULER && t.pos.y > full.min.y + RULER)
        .map(|t| t.pos.x + t.galley.size().x * 0.5)
        .collect();
    assert!(centres.len() > 2, "the left ruler draws labels: {centres:?}");
    for c in &centres {
        assert!((c - (full.min.x + RULER * 0.5)).abs() < 0.01, "character centre {c} in {centres:?}");
    }
}

#[test]
fn shift_snaps_the_zero_point_to_ruler_ticks_from_the_current_origin() {
    assert_eq!(snap_px_to_tick(47.0, 0.0, 10.0), 50.0);
    assert_eq!(snap_px_to_tick(44.9, 0.0, 10.0), 40.0);
    // Ticks run from the current zero point, so they sit at 3 + 10k.
    assert_eq!(snap_px_to_tick(47.0, 3.0, 10.0), 43.0);
    assert_eq!(snap_px_to_tick(-8.0, 3.0, 10.0), -7.0);
    // A degenerate tick never panics or moves the point.
    for tick in [0.0, -1.0, f64::NAN, f64::INFINITY] {
        assert_eq!(snap_px_to_tick(47.5, 3.0, tick), 47.5);
    }
}

/// Rulers alone in a 400×300 frame at 100 %, centred on a 256×128 document.
fn ruler_harness(app: PhotocraftApp) -> (egui_kittest::Harness<'static, PhotocraftApp>, Rect, ViewXform) {
    let full = Rect::from_min_size(pos2(0.0, 0.0), vec2(400.0, 300.0));
    let rect = Rect::from_min_max(full.min + vec2(RULER, RULER), full.max);
    let xf = ViewXform { rect, zoom: 1.0, center: [128.0, 64.0], flip: false, rotation: 0.0, aspect: 1.0 };
    let h = egui_kittest::Harness::builder()
        .with_size(vec2(400.0, 300.0))
        .with_step_dt(1.0 / 60.0)
        .build_ui_state(move |ui, app| draw_rulers(app, ui, full, &xf), app);
    (h, full, xf)
}

fn press(h: &mut egui_kittest::Harness<'static, PhotocraftApp>, pos: Pos2, pressed: bool, modifiers: egui::Modifiers) {
    h.event(egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers });
    h.run_steps(2);
}

fn drag_corner_to(h: &mut egui_kittest::Harness<'static, PhotocraftApp>, full: Rect, to: Pos2, modifiers: egui::Modifiers) {
    let corner = Rect::from_min_size(full.min, vec2(RULER, RULER)).center();
    h.event(egui::Event::ModifiersChanged(modifiers));
    h.event(egui::Event::PointerMoved(corner));
    h.run_steps(1);
    press(h, corner, true, modifiers);
    for p in [corner + vec2(10.0, 10.0), to] {
        h.event(egui::Event::PointerMoved(p));
        h.run_steps(2);
    }
    press(h, to, false, modifiers);
    h.event(egui::Event::ModifiersChanged(egui::Modifiers::NONE));
    h.run_steps(1);
}

fn origin(h: &egui_kittest::Harness<'static, PhotocraftApp>) -> [f64; 2] {
    h.state().session.active().unwrap().doc.ruler_origin
}

/// #2955: dragging out of the ruler corner sets the zero point (cross hairs while dragging),
/// the rulers then read 0 there, Shift snaps it to a tick, and double-clicking the corner
/// resets it.
#[test]
fn dragging_the_ruler_corner_sets_the_zero_point_and_double_click_resets_it() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 256, "height": 128})).unwrap();
    app.ui.extras.snap = false;
    let (mut h, full, xf) = ruler_harness(app);
    h.run_steps(2);
    let target = xf.to_screen(100.0, 40.0);
    drag_corner_to(&mut h, full, target, egui::Modifiers::NONE);
    let o = origin(&h);
    assert!((o[0] - 100.0).abs() < 0.01 && (o[1] - 40.0).abs() < 0.01, "zero point lands under the pointer: {o:?}");
    // The top ruler's "0" now sits at the new zero point, "100" (a distance, unsigned as in
    // Photoshop) at the image's left edge.
    h.run_steps(2);
    let labels: Vec<(String, f32)> = h
        .output()
        .shapes
        .iter()
        .filter_map(|s| if let egui::Shape::Text(t) = &s.shape { Some((t.galley.text().to_string(), t.pos.x)) } else { None })
        .collect();
    let near = |text: &str, x: f32| labels.iter().any(|(t, lx)| t == text && (lx - (x + 2.0)).abs() < 1.0);
    assert!(near("0", target.x), "a 0 label at the zero point: {labels:?}");
    assert!(near("100", xf.to_screen(0.0, 0.0).x), "100 at the image's left edge: {labels:?}");

    // Mid-drag, cross hairs run through the pointer across the canvas.
    let corner = Rect::from_min_size(full.min, vec2(RULER, RULER)).center();
    let mid = xf.to_screen(150.0, 90.0);
    press(&mut h, corner, true, egui::Modifiers::NONE);
    for p in [corner + vec2(10.0, 10.0), mid] {
        h.event(egui::Event::PointerMoved(p));
        h.run_steps(2);
    }
    let crosshair = h.output().shapes.iter().any(|s| {
        matches!(s.shape, egui::Shape::LineSegment { points: [a, b], .. } if (a.x - mid.x).abs() < 0.5 && (b.x - mid.x).abs() < 0.5 && (b.y - a.y).abs() > 200.0)
    });
    assert!(crosshair, "a vertical cross-hair line through the pointer while dragging");
    assert_eq!(origin(&h), o, "the zero point moves on release only");
    press(&mut h, mid, false, egui::Modifiers::NONE);
    assert_eq!(origin(&h), [150.0, 90.0]);
    h.state_mut().run("edit.undo", json!({})).unwrap();
    assert_eq!(origin(&h), o, "setting the zero point is undoable");

    // A drag dropped back on the corner leaves it alone.
    drag_corner_to(&mut h, full, full.min + vec2(4.0, 4.0), egui::Modifiers::NONE);
    assert_eq!(origin(&h), o);

    // Shift: ticks are 10 px apart at 100 %, measured from the current zero point (100, 40).
    drag_corner_to(&mut h, full, xf.to_screen(123.0, 57.0), egui::Modifiers::SHIFT);
    assert_eq!(origin(&h), [120.0, 60.0]);

    for _ in 0..40 {
        h.step();
    }
    let corner = Rect::from_min_size(full.min, vec2(RULER, RULER)).center();
    h.event(egui::Event::PointerMoved(corner));
    h.step();
    for _ in 0..2 {
        for pressed in [true, false] {
            h.event(egui::Event::PointerButton { pos: corner, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE });
            h.step();
        }
    }
    h.run_steps(2);
    assert_eq!(origin(&h), [0.0, 0.0], "double-clicking the corner resets the zero point");
}

/// #2955: the Info panel's X/Y are measured from the rulers' zero point.
#[test]
fn info_panel_position_reads_from_the_zero_point() {
    use egui_kittest::kittest::Queryable;
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
    app.run("view.setRulerOrigin", json!({"x": 20, "y": 5})).unwrap();
    app.hover_doc = Some([50.5, 20.5]);
    let mut h = egui_kittest::Harness::builder()
        .with_size(vec2(260.0, 240.0))
        .build_ui_state(|ui, app| crate::panels::dock_body(app, ui, crate::dock::Group::Navigator, 2), app);
    h.run_steps(2);
    assert!(h.query_by_label("30").is_some() && h.query_by_label("15").is_some(), "X 30, Y 15 from the zero point at (20, 5)");
    assert!(h.query_by_label("50").is_none(), "not the raw image x");
}

/// #5728: dragging a guide shows its coordinate in the Info panel on one axis only.
#[test]
fn info_panel_shows_guide_drag_position_outside_canvas() {
    use egui_kittest::kittest::Queryable;
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
    app.hover_doc = None;
    app.guide_drag = Some(GuideDrag { vertical: true, index: None, pos: 42.0, press: 42.0, over_ruler: false, flipped: false });
    let mut h = egui_kittest::Harness::builder()
        .with_size(vec2(260.0, 240.0))
        .build_ui_state(|ui, app| crate::panels::dock_body(app, ui, crate::dock::Group::Navigator, 2), app);
    h.run_steps(2);
    assert!(h.query_by_label("42").is_some(), "vertical guide: X readout");
    h.state_mut().guide_drag = Some(GuideDrag { vertical: false, index: None, pos: 17.0, press: 17.0, over_ruler: false, flipped: false });
    h.run_steps(2);
    assert!(h.query_by_label("17").is_some(), "horizontal guide: Y readout");
}

/// A dragged guide shows its position beside the pointer: X for a vertical guide, Y for a
/// horizontal one, from the zero point, in the ruler unit (pixels to one decimal).
#[test]
fn guide_drag_shows_its_position_beside_the_pointer() {
    use egui_kittest::kittest::Queryable;
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
    let v = GuideDrag { vertical: true, index: Some(0), pos: 103.24, press: 50.0, over_ruler: false, flipped: false };
    assert_eq!(guide_drag_readout(&app, v), Some(("X:", "103.2 px".to_owned())));
    let h = GuideDrag { vertical: false, index: None, pos: 40.0, press: 40.0, over_ruler: false, flipped: false };
    assert_eq!(guide_drag_readout(&app, h), Some(("Y:", "40 px".to_owned())));
    app.run("view.setRulerOrigin", json!({"x": 3.0, "y": 10.0})).unwrap();
    assert_eq!(guide_drag_readout(&app, h), Some(("Y:", "30 px".to_owned())), "from the zero point");
    app.run("view.resetRulerOrigin", json!({})).unwrap();
    app.run("prefs.set", json!({"path": "unitsAndRulers.rulers", "value": "percent"})).unwrap();
    assert_eq!(
        guide_drag_readout(&app, GuideDrag { vertical: true, index: None, pos: 103.0, press: 0.0, over_ruler: false, flipped: false }).map(|r| r.0),
        Some("X:")
    );
    app.run("prefs.set", json!({"path": "unitsAndRulers.rulers", "value": "pixels"})).unwrap();

    app.guide_drag = Some(v);
    let mut h = egui_kittest::Harness::builder().with_size(vec2(300.0, 200.0)).build_ui_state(|ui, app| draw_guide_drag_readout(app, ui.ctx()), app);
    h.event(egui::Event::PointerMoved(pos2(50.0, 50.0)));
    h.run_steps(2);
    assert!(h.query_by_label("X:").is_some() && h.query_by_label("103.2 px").is_some());
    h.state_mut().guide_drag = None;
    h.run_steps(2);
    assert!(h.query_by_label("X:").is_none(), "no readout once the drag ends");
}

type AppHarness = egui_kittest::Harness<'static, PhotocraftApp>;

/// The real canvas on a 400 × 300 document with a horizontal guide at y = 100, at 100 %,
/// with the Move tool and snapping off.
fn canvas_harness(rulers: bool) -> AppHarness {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app.run("view.newGuide", json!({"orientation": "horizontal", "position": 100})).unwrap();
    app.sync_views();
    app.ui.extras.rulers = rulers;
    app.ui.extras.snap = false;
    app.ui.tool = crate::state::Tool::Move;
    let mut h = egui_kittest::Harness::builder().with_size(vec2(800.0, 600.0)).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            if !ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            egui::CentralPanel::default().show(ui, |ui| crate::canvas::document_area(app, ui));
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    h.run_steps(4);
    let v = &mut h.state_mut().ui.views[0];
    (v.zoom, v.center, v.fit_pending) = (1.0, [200.0, 150.0], false);
    h.run_steps(2);
    h
}

/// Screen point of document point `(x, y)` in [`canvas_harness`].
fn canvas_at(h: &AppHarness, x: f32, y: f32) -> Pos2 {
    let app = h.state();
    let v = &app.ui.views[0];
    let rect = content_rect(app, app.last_canvas_rect);
    ViewXform { rect, zoom: v.zoom, center: v.center, flip: false, rotation: 0.0, aspect: 1.0 }.to_screen(x, y)
}

/// Moving a horizontal guide with the Move tool on the real canvas: `Y:` follows the pointer.
#[test]
fn moving_a_guide_on_the_canvas_shows_its_y() {
    use egui_kittest::kittest::Queryable;
    let mut h = canvas_harness(false);
    let at = canvas_at;
    let press = at(&h, 200.0, 100.0);
    h.event(egui::Event::PointerMoved(press));
    h.run_steps(1);
    h.event(egui::Event::PointerButton { pos: press, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE });
    h.run_steps(1);
    for y in [110.0, 125.0, 140.0] {
        h.event(egui::Event::PointerMoved(at(&h, 210.0, y)));
        h.run_steps(2);
    }
    assert!(h.state().guide_drag.is_some_and(|d| !d.vertical), "a horizontal guide is being dragged");
    assert!(h.query_by_label("Y:").is_some() && h.query_by_label("140 px").is_some(), "Y readout at the guide");
    assert!(h.query_by_label("X:").is_none());
    let up = at(&h, 210.0, 140.0);
    h.event(egui::Event::PointerButton { pos: up, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE });
    h.run_steps(2);
    assert_eq!(h.state().session.active().unwrap().doc.guides.horizontal, vec![140.0]);
    assert!(h.query_by_label("Y:").is_none(), "gone after the drop");
}

/// A guide dragged out of the top ruler: in each frame the readout shows where the guide is
/// after that frame's pointer move (the ruler handles the drag after the canvas is drawn), and
/// the frame that drops it shows none.
#[test]
fn ruler_drag_readout_is_current_every_frame() {
    use egui_kittest::kittest::Queryable;
    let mut h = canvas_harness(true);
    let from = pos2(canvas_at(&h, 200.0, 0.0).x, h.state().last_canvas_rect.top() + RULER / 2.0);
    h.event(egui::Event::PointerMoved(from));
    h.run_steps(1);
    h.event(egui::Event::PointerButton { pos: from, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE });
    h.run_steps(1);
    for y in [40.0, 60.0, 75.0] {
        h.event(egui::Event::PointerMoved(canvas_at(&h, 200.0, y)));
        // egui 0.37 lays an Area's contents out a pass after it is shown; step to the settled
        // frame with the pointer still at `y` (readout included) rather than the first pass.
        h.run_steps(2);
        let pos = h.state().guide_drag.map(|d| d.pos);
        assert_eq!(pos, Some(f64::from(y)), "the drag follows the pointer");
        assert!(h.query_by_label(&format!("{y} px")).is_some(), "readout at {y} in the same frame");
    }
    let up = canvas_at(&h, 200.0, 75.0);
    h.event(egui::Event::PointerButton { pos: up, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE });
    h.run_steps(2);
    assert!(h.state().guide_drag.is_none());
    assert!(h.query_by_label("Y:").is_none(), "no readout in the frame that drops the guide");
    assert!(h.state().session.active().unwrap().doc.guides.horizontal.contains(&75.0), "the guide lands at y = 75");
}

#[test]
fn percentage_grid_spacing_uses_both_canvas_dimensions() {
    let mut g = GuidesGridAndSlices { grid_unit: Unit::Percent, gridline_every: 20.0, subdivisions: 2, ..Default::default() };
    assert_eq!(grid_major_px(&g, 72.0, [1280.0, 720.0], 72.0), [256.0, 144.0]);
    // Absolute units remain equal on both axes, irrespective of aspect ratio.
    g.grid_unit = Unit::Pixels;
    g.gridline_every = 100.0;
    assert_eq!(grid_major_px(&g, 72.0, [1280.0, 720.0], 72.0), [100.0, 100.0]);
}

#[test]
fn unit_steps_allow_fractions() {
    assert_eq!(nice_step(0.3, false), 0.5);
    assert_eq!(nice_step(0.2, false), 0.2);
    assert_eq!(nice_step(0.22, false), 0.25);
    assert_eq!(nice_step(0.3, true), 1.0);
    assert_eq!(nice_step(23.0, true), 50.0);
}

#[test]
fn ticks_follow_1_2_5_series() {
    assert_eq!(tick_step(1.0), 100.0);
    assert_eq!(tick_step(0.1), 1000.0);
    assert_eq!(tick_step(4.0), 20.0);
    assert_eq!(tick_step(64.0), 1.0);
    assert_eq!(tick_step(0.25), 500.0);
}

#[test]
fn hidden_guides_are_not_hit_tested() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 200, "height": 100})).unwrap();
    app.sync_views();
    app.session.execute("view.newGuide", json!({"orientation": "vertical", "position": 50})).unwrap();
    app.session.execute("view.newGuide", json!({"orientation": "horizontal", "position": 25})).unwrap();
    let positions = app.session.active().unwrap().doc.guides.clone();

    // Visible, unlocked guides remain interactive in either orientation.
    assert_eq!(guide_at(&app, 50.0, 10.0), Some((true, 0)));
    assert_eq!(guide_at(&app, 10.0, 25.0), Some((false, 0)));

    // All ways to hide the canvas guides must also disable hit testing.
    app.ui.extras.guides = false;
    assert_eq!(guide_at(&app, 50.0, 10.0), None);
    assert_eq!(guide_at(&app, 10.0, 25.0), None);
    app.ui.extras.guides = true;

    app.ui.view.show.canvas_guides = false;
    assert_eq!(guide_at(&app, 50.0, 10.0), None);
    assert_eq!(guide_at(&app, 10.0, 25.0), None);
    app.ui.view.show.canvas_guides = true;

    app.ui.view.extras = false;
    assert_eq!(guide_at(&app, 50.0, 10.0), None);
    assert_eq!(guide_at(&app, 10.0, 25.0), None);
    app.ui.view.extras = true;

    // Hiding/showing must not change the stored guide positions.
    assert_eq!(app.session.active().unwrap().doc.guides, positions);
    assert_eq!(guide_at(&app, 50.0, 10.0), Some((true, 0)));
    assert_eq!(guide_at(&app, 10.0, 25.0), Some((false, 0)));

    app.ui.extras.lock_guides = true;
    assert_eq!(guide_at(&app, 50.0, 10.0), None);
    assert_eq!(guide_at(&app, 10.0, 25.0), None);
}

#[test]
fn unit_menu_lists_every_unit_with_the_current_one_checked() {
    let items = unit_menu_items(Unit::Inches, &UNITS);
    assert_eq!(items.len(), Unit::NAMES.len());
    // Every unit is offered, labelled, and in the preference's order.
    assert_eq!(items[0], ("Pixels".to_string(), "pixels", false));
    assert_eq!(items.iter().find(|(_, name, _)| *name == "cm").unwrap().0, "Centimeters");
    // Exactly the current unit is checked.
    let checked: Vec<_> = items.iter().filter(|(_, _, c)| *c).map(|(_, name, _)| *name).collect();
    assert_eq!(checked, vec!["inches"]);
}

#[test]
fn picking_a_ruler_unit_changes_the_preference() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    assert_eq!(app.session.prefs().units_and_rulers.rulers, Unit::Pixels);
    // The context menu applies its pick exactly as the preference pane does.
    app.run("prefs.set", json!({"path": "unitsAndRulers.rulers", "value": "cm"})).unwrap();
    assert_eq!(app.session.prefs().units_and_rulers.rulers, Unit::Centimeters);
}

/// View › Rulers is remembered: the next launch, and every document created or opened in it,
/// shows the rulers as last set, on or off (#2918).
#[test]
fn rulers_open_as_last_set_after_a_restart() {
    use std::{cell::RefCell, rc::Rc};
    let saved = Rc::new(RefCell::new(None::<String>));
    let launch = |saved: &Rc<RefCell<Option<String>>>| {
        let (load, store) = (saved.clone(), saved.clone());
        PhotocraftApp::new(
            photocraft_engine::Session::new(),
            crate::Services {
                load_prefs: Some(Box::new(move || load.borrow().clone())),
                save_prefs: Some(Box::new(move |text| {
                    *store.borrow_mut() = Some(text.to_string());
                    Ok(())
                })),
                ..Default::default()
            },
        )
    };
    for on in [true, false] {
        let mut app = launch(&saved);
        assert_eq!(crate::menus::invoke(&mut app, &egui::Context::default(), "view.rulers", json!({})).unwrap(), json!(on));
        crate::prefs_ui::save_preferences(&mut app).unwrap();
        let mut next = launch(&saved);
        next.session.execute("file.new", json!({"width": 20, "height": 10})).unwrap();
        next.sync_views();
        assert_eq!(next.ui.extras.rulers, on, "a new document after a restart shows the rulers as last set");
    }
}

#[test]
fn guide_drags_create_move_and_delete() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 200, "height": 100})).unwrap();
    app.sync_views();
    finish_drag(&mut app, GuideDrag { vertical: true, index: None, pos: 50.0, press: 50.0, over_ruler: false, flipped: false });
    finish_drag(&mut app, GuideDrag { vertical: false, index: None, pos: 500.0, press: 500.0, over_ruler: false, flipped: false }); // off canvas: ignored
    assert_eq!(app.session.active().unwrap().doc.guides.vertical, vec![50.0]);
    assert!(app.session.active().unwrap().doc.guides.horizontal.is_empty());
    assert_eq!(guide_at(&app, 51.0, 10.0), Some((true, 0)));
    finish_drag(&mut app, GuideDrag { vertical: true, index: Some(0), pos: 120.0, press: 50.0, over_ruler: false, flipped: false });
    assert_eq!(app.session.active().unwrap().doc.guides.vertical, vec![120.0]);
    finish_drag(&mut app, GuideDrag { vertical: true, index: Some(0), pos: -30.0, press: 120.0, over_ruler: false, flipped: false });
    assert!(app.session.active().unwrap().doc.guides.vertical.is_empty());
    app.ui.extras.lock_guides = true;
    assert_eq!(guide_at(&app, 0.0, 0.0), None);
}

/// ⌘ is the Move tool with any tool, so a ⌘-drag over a guide moves the guide, never the layer
/// (#2690); with Lock Guides a ⌘-drag with a marquee moves the layer as before.
#[test]
fn command_drag_over_a_guide_moves_the_guide_with_any_tool() {
    use crate::canvas::{ToolEvent, tool_event};
    use crate::state::Tool;
    for (tool, locked) in [
        (Tool::Brush, false),
        (Tool::Eraser, false),
        (Tool::Gradient, false),
        (Tool::Frame, false),
        (Tool::Artboard, false),
        (Tool::RectMarquee, false),
        (Tool::Lasso, false),
        (Tool::MagicWand, false),
        (Tool::RectMarquee, true),
    ] {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 80, "height": 60, "background": "transparent"})).unwrap();
        app.sync_views();
        app.ui.tool = tool;
        app.ui.extras.lock_guides = locked;
        let layer = app.session.active().unwrap().active_layer.unwrap();
        app.session
            .edit("paint", |doc, _| {
                doc.layer_mut(layer).unwrap().surface_mut().unwrap().fill_rect(photocraft_geom::Rect::new(10, 10, 30, 30), &[1.0, 0.0, 0.0, 1.0]);
                Ok(())
            })
            .unwrap();
        app.run("view.newGuide", json!({"orientation": "vertical", "position": 50})).unwrap();
        let cmd = egui::Modifiers::COMMAND;
        tool_event(&mut app, ToolEvent::Down { x: 50.0, y: 40.0, pressure: 1.0 }, cmd);
        tool_event(&mut app, ToolEvent::Move { x: 65.0, y: 40.0, pressure: 1.0 }, cmd);
        tool_event(&mut app, ToolEvent::Up { x: 65.0, y: 40.0 }, cmd);
        let doc = &app.session.active().unwrap().doc;
        let moved = doc.layer(layer).unwrap().surface().unwrap().rgba(12, 12)[3] == 0.0;
        if locked {
            assert_eq!((doc.guides.vertical.clone(), moved), (vec![50.0], true), "{tool:?}, guides locked: the layer moves");
        } else {
            assert_eq!((doc.guides.vertical.clone(), moved), (vec![65.0], false), "{tool:?}: the guide moves, the layer stays");
        }
    }
}

/// #3233: a rotated view keeps the rulers level and fully ticked at zoom scale. The top ruler
/// spans the same number of document pixels as the unrotated view, the ticks map back onto
/// its ends, and both rulers read 0 at the screen position of the (rotated) document origin,
/// as Photoshop does.
#[test]
fn rotated_view_keeps_full_rulers() {
    let rect = Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 600.0));
    let (a, b) = (rect.left_top(), rect.right_top());
    for rotation in [0.0f32, 30.0, 45.0, 90.0, -119.0, 180.0] {
        for flip in [false, true] {
            let xf = ViewXform { rect, zoom: 0.5, center: [1000.0, 700.0], flip, rotation, aspect: 1.0 };
            let (u0, u1) = ruler_range(&xf, a, b, 0, 1.0, [0.0; 2]);
            assert!((u1 - u0 - 1600.0).abs() < 1e-2, "rotation {rotation}: top ruler covers {u0}..{u1}");
            let (first, last) = if flip { (u1, u0) } else { (u0, u1) };
            assert!((ruler_pos(&xf, false, first, 1.0, [0.0; 2]) - a.x).abs() < 1e-2, "rotation {rotation}: first tick");
            assert!((ruler_pos(&xf, false, last, 1.0, [0.0; 2]) - b.x).abs() < 1e-2, "rotation {rotation}: last tick");
            // 0 at the projected origin, 500 document px = 500 × zoom screen px further on.
            let origin = xf.to_screen(0.0, 0.0);
            assert!((ruler_pos(&xf, false, 0.0, 1.0, [0.0; 2]) - origin.x).abs() < 1e-2, "rotation {rotation}: top ruler 0");
            assert!((ruler_pos(&xf, true, 0.0, 1.0, [0.0; 2]) - origin.y).abs() < 1e-2, "rotation {rotation}: left ruler 0");
            let dir = if flip { -1.0 } else { 1.0 };
            assert!((ruler_pos(&xf, false, 500.0, 1.0, [0.0; 2]) - origin.x - dir * 250.0).abs() < 1e-2, "rotation {rotation}: top scale");
            assert!((ruler_pos(&xf, true, 500.0, 1.0, [0.0; 2]) - origin.y - 250.0).abs() < 1e-2, "rotation {rotation}: left scale");
        }
    }
    // Distances left of / above the origin read without a minus sign, as in Photoshop.
    let xf = ViewXform { rect, zoom: 0.5, center: [0.0, 0.0], flip: false, rotation: 0.0, aspect: 1.0 };
    let (u0, u1) = ruler_range(&xf, a, b, 0, 1.0, [0.0; 2]);
    assert!(u0 < -500.0 && u1 > 500.0, "the view spans the origin: {u0}..{u1}");
    for (step, whole) in [(100.0, true), (100.0, false), (0.25, false)] {
        let mut v = (u0 / step).floor() * step;
        while v <= u1 {
            let s = ruler_label(v, step, whole);
            assert!(!s.contains('-'), "tick {v} (step {step}) is labelled {s:?}");
            v += step;
        }
    }
    assert_eq!(ruler_label(-500.0, 100.0, true), "500");
    assert_eq!(ruler_label(-0.25, 0.25, false), "0.25");
}

/// #2955 with #3233: a zero point moved off the image's top-left reads 0 where the canvas draws
/// it, also on a rotated or flipped view, and the ruler keeps its full length and scale.
#[test]
fn rulers_count_from_the_zero_point_on_a_rotated_view() {
    let rect = Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 600.0));
    let (a, b) = (rect.left_top(), rect.right_top());
    let zero = [300.0, 120.0];
    for rotation in [0.0f32, 30.0, 90.0, -119.0] {
        for flip in [false, true] {
            let xf = ViewXform { rect, zoom: 0.5, center: [1000.0, 700.0], flip, rotation, aspect: 1.0 };
            let at = xf.to_screen(zero[0] as f32, zero[1] as f32);
            assert!((ruler_pos(&xf, false, 0.0, 1.0, zero) - at.x).abs() < 1e-2, "rotation {rotation}: top ruler 0");
            assert!((ruler_pos(&xf, true, 0.0, 1.0, zero) - at.y).abs() < 1e-2, "rotation {rotation}: left ruler 0");
            let (u0, u1) = ruler_range(&xf, a, b, 0, 1.0, zero);
            assert!((u1 - u0 - 1600.0).abs() < 1e-2, "rotation {rotation}: top ruler covers {u0}..{u1}");
            let dir = if flip { -1.0 } else { 1.0 };
            assert!((ruler_pos(&xf, false, 500.0, 1.0, zero) - at.x - dir * 250.0).abs() < 1e-2, "rotation {rotation}: top scale");
        }
    }
    // Unrotated, ruler value u sits over document x = zero + u.
    let xf = ViewXform { rect, zoom: 0.5, center: [1000.0, 700.0], flip: false, rotation: 0.0, aspect: 1.0 };
    assert!((ruler_pos(&xf, false, -300.0, 1.0, zero) - xf.to_screen(0.0, 0.0).x).abs() < 1e-2);
    assert!((ruler_pos(&xf, true, 80.0, 1.0, zero) - xf.to_screen(0.0, 200.0).y).abs() < 1e-2);
}

/// #3277: guides belong to the document, so a rotated view draws them along the document
/// line; unrotated they still span the whole clip rect.
#[test]
fn guides_turn_with_a_rotated_view() {
    let rect = Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 600.0));
    let level = ViewXform { rect, zoom: 0.5, center: [1000.0, 700.0], flip: false, rotation: 0.0, aspect: 1.0 };
    let [a, b] = guide_segment(&level, rect, true, 500.0);
    assert!((a.x - b.x).abs() < 1e-3 && (a.y - rect.top()).abs() < 1e-3 && (b.y - rect.bottom()).abs() < 1e-3, "{a:?} {b:?}");
    for rotation in [30.0f32, 90.0, -119.0] {
        let xf = ViewXform { rotation, ..level };
        for (vertical, pos) in [(true, 500.0), (false, 900.0)] {
            for end in guide_segment(&xf, rect, vertical, pos) {
                let d = xf.to_doc(end);
                let on_line = if vertical { d[0] } else { d[1] };
                assert!((on_line - pos).abs() < 1e-2, "rotation {rotation}: {end:?} maps to {d:?}, off the guide at {pos}");
            }
        }
    }
}

/// The cursor over a guide turns with the view as the guide does: on a view turned 90°, a
/// vertical guide shows as a horizontal line and moves up and down.
#[test]
fn guide_cursors_turn_with_a_rotated_view() {
    use egui::CursorIcon::{ResizeHorizontal as H, ResizeNeSw as NeSw, ResizeNwSe as NwSe, ResizeVertical as V};
    let rect = Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 600.0));
    let level = ViewXform { rect, zoom: 1.0, center: [400.0, 300.0], flip: false, rotation: 0.0, aspect: 1.0 };
    // View rotation, flip → the cursor over a vertical guide, over a horizontal guide.
    for (rotation, flip, want) in [(0.0, false, [H, V]), (90.0, false, [V, H]), (45.0, false, [NwSe, NeSw]), (0.0, true, [H, V])] {
        let xf = ViewXform { rotation, flip, ..level };
        assert_eq!([guide_cursor(&xf, true), guide_cursor(&xf, false)], want, "view turned {rotation}°, flip {flip}");
    }
}

#[test]
fn ruler_range_is_normalised_when_flipped() {
    // #1814: with View > Flip Horizontal, `to_doc` negates x, so the raw
    // (d0, d1) comes out reversed. ruler_range must normalise to (min, max)
    // so the tick loop still runs. Exercises the production helper.
    let rect = Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 600.0));
    let top = Rect::from_min_size(rect.min, vec2(rect.width(), RULER));
    for flip in [false, true] {
        let xf = ViewXform { rect, zoom: 1.0, center: [400.0, 300.0], flip, rotation: 0.0, aspect: 1.0 };
        let (u0, u1) = ruler_range(&xf, top.left_top(), top.right_top(), 0, 1.0, [0.0; 2]);
        assert!(u0 <= u1, "flip={flip}: u0 ({u0}) must be <= u1 ({u1})");
        assert!(u1 - u0 > 0.0, "flip={flip}: range is empty");
    }
}

/// #5698: as in Photoshop, an Alt-drag out of the top ruler makes a vertical guide and one out
/// of the left ruler a horizontal guide; without Alt each ruler makes its own kind.
#[test]
fn alt_drags_a_guide_of_the_other_orientation_out_of_a_ruler() {
    let full = Rect::from_min_size(Pos2::ZERO, vec2(400.0, 300.0));
    let rect = Rect::from_min_max(full.min + vec2(RULER, RULER), full.max);
    let xf = ViewXform { rect, zoom: 1.0, center: [128.0, 64.0], flip: false, rotation: 0.0, aspect: 1.0 };
    let drag = |from: Pos2, modifiers: egui::Modifiers| {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 256, "height": 128})).unwrap();
        let mut h = egui_kittest::Harness::builder().with_size(vec2(400.0, 300.0)).build_ui_state(move |ui, app| draw_rulers(app, ui, full, &xf), app);
        let to = xf.to_screen(37.0, 90.0);
        h.event_modifiers(egui::Event::PointerMoved(from), modifiers);
        h.event_modifiers(egui::Event::PointerButton { pos: from, button: egui::PointerButton::Primary, pressed: true, modifiers }, modifiers);
        h.run_steps(1);
        h.event_modifiers(egui::Event::PointerMoved(to), modifiers);
        h.run_steps(1);
        h.event_modifiers(egui::Event::PointerButton { pos: to, button: egui::PointerButton::Primary, pressed: false, modifiers }, modifiers);
        h.run_steps(2);
        let g = &h.state().session.active().unwrap().doc.guides;
        (g.vertical.clone(), g.horizontal.clone())
    };
    let (top, left) = (pos2(200.0, RULER / 2.0), pos2(RULER / 2.0, 150.0));
    assert_eq!(drag(top, egui::Modifiers::NONE), (vec![], vec![90.0]), "top ruler: horizontal");
    assert_eq!(drag(left, egui::Modifiers::NONE), (vec![37.0], vec![]), "left ruler: vertical");
    assert_eq!(drag(top, egui::Modifiers::ALT), (vec![37.0], vec![]), "Alt from the top ruler: vertical");
    assert_eq!(drag(left, egui::Modifiers::ALT), (vec![], vec![90.0]), "Alt from the left ruler: horizontal");
}

/// #6286: a guide let go over a ruler is removed, whichever ruler it is. The top ruler measures
/// document x and the left one y, so a horizontal guide released on the left ruler is still
/// inside the document on its own axis, and used to be placed instead of discarded.
#[test]
fn a_guide_dropped_on_either_ruler_is_removed() {
    let new_app = || {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 256, "height": 128})).unwrap();
        app.ui.extras.rulers = true;
        // No snapping, so a guide that is kept lands exactly where it is let go.
        app.ui.extras.snap = false;
        app
    };
    let drag = |from: Pos2, to: Pos2| {
        let (mut h, _, _) = ruler_harness(new_app());
        h.event(egui::Event::PointerMoved(from));
        press(&mut h, from, true, egui::Modifiers::NONE);
        h.event(egui::Event::PointerMoved(to));
        h.run_steps(1);
        press(&mut h, to, false, egui::Modifiers::NONE);
        let g = &h.state().session.active().unwrap().doc.guides;
        (g.vertical.clone(), g.horizontal.clone())
    };
    // #5698: out of the top ruler is a horizontal guide, out of the left one a vertical guide.
    let (top, left) = (pos2(200.0, RULER / 2.0), pos2(RULER / 2.0, 150.0));
    assert_eq!(drag(top, left), (vec![], vec![]), "out of the top ruler, released on the left ruler");
    assert_eq!(drag(left, top), (vec![], vec![]), "out of the left ruler, released on the top ruler");
    // Released on the canvas the guide lands where it was let go, as before.
    assert_eq!(drag(top, pos2(200.0, 150.0)), (vec![], vec![56.0]), "released on the canvas");
    assert_eq!(drag(left, pos2(120.0, 150.0)), (vec![40.0], vec![]), "released on the canvas");
}

/// #6286: the guide is removed when it is let go over a ruler, and moved when it is not. The
/// release can be inside the document on the guide's own axis and still be a ruler drop, so the
/// two are told apart by where the pointer is, not by the position it reads.
#[test]
fn a_guide_let_go_on_a_ruler_is_removed_not_moved() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
    app.sync_views();
    let horizontal = |app: &PhotocraftApp| app.session.active().unwrap().doc.guides.horizontal.clone();
    for drag in [
        GuideDrag { vertical: false, index: Some(0), pos: 50.0, press: 10.0, over_ruler: true, flipped: false },
        // Dragged sideways onto the left ruler, a horizontal guide holds its y, which on its own
        // reads as a click: the ruler still removes it.
        GuideDrag { vertical: false, index: Some(0), pos: 50.0, press: 50.0, over_ruler: true, flipped: false },
    ] {
        app.run("view.newGuide", json!({"orientation": "horizontal", "position": 50})).unwrap();
        finish_drag(&mut app, drag);
        assert!(horizontal(&app).is_empty(), "let go on a ruler: removed, {drag:?}");
    }
    app.run("view.newGuide", json!({"orientation": "horizontal", "position": 50})).unwrap();
    finish_drag(&mut app, GuideDrag { vertical: false, index: Some(0), pos: 80.0, press: 10.0, over_ruler: false, flipped: false });
    assert_eq!(horizontal(&app), vec![80.0], "let go on the canvas: moved");
    // Clicked where it was pressed, on the canvas: selected and kept, as before.
    finish_drag(&mut app, GuideDrag { vertical: false, index: Some(0), pos: 80.0, press: 80.0, over_ruler: false, flipped: false });
    assert_eq!(horizontal(&app), vec![80.0], "clicked on the canvas: kept");
    assert!(app.ui.selected_guide.is_some(), "clicked on the canvas: selected");
}

/// The ruler bands are the strips the rulers take off the canvas rect, and only while the rulers
/// are shown: with them hidden the whole canvas rect is canvas (#6286).
#[test]
fn only_the_ruler_bands_count_as_a_ruler() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let full = Rect::from_min_size(pos2(16.0, 42.0), vec2(968.0, 642.0));
    let content = Rect::from_min_size(pos2(32.0, 58.0), vec2(952.0, 626.0));
    app.ui.extras.rulers = true;
    assert!(dropped_on_a_ruler(&app, full, pos2(full.min.x + RULER / 2.0, 371.0)), "the left ruler");
    assert!(dropped_on_a_ruler(&app, full, pos2(508.0, full.min.y + RULER / 2.0)), "the top ruler");
    assert!(dropped_on_a_ruler(&app, full, full.min), "the ruler corner");
    assert!(!dropped_on_a_ruler(&app, full, pos2(508.0, 371.0)), "the canvas");
    assert!(!dropped_on_a_ruler(&app, full, content.min), "the canvas' own top-left corner");
    assert!(!dropped_on_a_ruler(&app, full, pos2(content.min.x + 0.5, 371.0)), "the first canvas column");
    assert!(!dropped_on_a_ruler(&app, full, pos2(508.0, content.min.y + 0.5)), "the first canvas row");
    assert!(!dropped_on_a_ruler(&app, full, pos2(full.min.x - 1.0, 371.0)), "outside the canvas rect");
    app.ui.extras.rulers = false;
    for p in [pos2(full.min.x + RULER / 2.0, 371.0), pos2(508.0, full.min.y + RULER / 2.0), full.min] {
        assert!(!dropped_on_a_ruler(&app, full, p), "rulers hidden: no band at {p:?}");
    }
}

/// The canvas the guides live on, rulers on, at 100 %.
fn tool_canvas_harness(tool: crate::state::Tool) -> egui_kittest::Harness<'static, PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app.sync_views();
    app.ui.extras.rulers = true;
    app.ui.extras.snap = false;
    app.ui.tool = tool;
    let mut h = egui_kittest::Harness::builder().with_size(vec2(1000.0, 700.0)).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            let ctx = ui.ctx().clone();
            if !ctx.fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            crate::shortcuts::handle(app, &ctx);
            egui::CentralPanel::default().show(ui, |ui| crate::canvas::document_area(app, ui));
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    h.run_steps(4);
    let v = &mut h.state_mut().ui.views[0];
    v.zoom = 1.0;
    v.center = [200.0, 150.0];
    v.fit_pending = false;
    h.run_steps(2);
    h
}

/// #6286: an existing guide dragged onto a ruler with the Move tool is deleted, though the
/// release is inside the document on the guide's own axis: the left ruler measures y, so a
/// horizontal guide let go there still reads a y inside the canvas and used to be moved. Held
/// level, that release reads as a click on the guide, which a ruler drop also outranks.
#[test]
fn a_guide_dragged_onto_a_ruler_is_deleted_through_the_canvas() {
    for below in [50.0f32, 0.0] {
        let mut h = tool_canvas_harness(crate::state::Tool::Move);
        h.state_mut().run("view.newGuide", json!({"orientation": "horizontal", "position": 150})).unwrap();
        h.run_steps(1);
        let at = |h: &egui_kittest::Harness<'static, PhotocraftApp>, x: f32, y: f32| {
            let app = h.state();
            let v = &app.ui.views[0];
            let xf = ViewXform {
                rect: content_rect(app, app.last_canvas_rect),
                zoom: v.zoom,
                center: v.center,
                flip: app.ui.view.flip_horizontal,
                rotation: v.rotation,
                aspect: app.ui.view.display_aspect(),
            };
            xf.to_screen(x, y)
        };
        let on_the_guide = at(&h, 200.0, 150.0);
        // In the left ruler's band, `below` document px under the press: inside the canvas on the
        // guide's own axis, which is where the old code moved the guide instead of removing it.
        let canvas = h.state().last_canvas_rect;
        let onto_the_left_ruler = pos2(canvas.min.x + RULER / 2.0, on_the_guide.y + below);
        // Along the guide, so a held-level release keeps its y, the way a sideways drag does.
        let dragged = pos2(on_the_guide.x - 12.0, on_the_guide.y);
        h.event(egui::Event::PointerMoved(on_the_guide));
        h.run_steps(1);
        h.event(egui::Event::PointerButton { pos: on_the_guide, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE });
        h.run_steps(1);
        // Past egui's click distance, so the drag starts at the press, on the guide.
        h.event(egui::Event::PointerMoved(dragged));
        h.run_steps(2);
        assert!(h.state().guide_drag.is_some(), "the Move tool takes the guide");
        h.event(egui::Event::PointerMoved(onto_the_left_ruler));
        h.run_steps(1);
        h.event(egui::Event::PointerButton {
            pos: onto_the_left_ruler,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::NONE,
        });
        h.run_steps(2);
        let g = &h.state().session.active().unwrap().doc.guides;
        assert!(g.horizontal.is_empty(), "{below} px below the press, released on the left ruler: got {:?}", g.horizontal);
        assert!(h.state().guide_drag.is_none(), "the drag is over");
    }
}

/// #3770: a guide Shift-dragged out of a ruler lands on the nearest tick the ruler draws
/// (tenths of the labelled step, in the ruler unit); without Shift it lands where it's dropped.
#[test]
fn shift_dragged_guides_land_on_ruler_ticks() {
    let full = Rect::from_min_size(Pos2::ZERO, vec2(400.0, 300.0));
    let rect = Rect::from_min_max(full.min + vec2(RULER, RULER), full.max);
    let xf = ViewXform { rect, zoom: 1.0, center: [128.0, 64.0], flip: false, rotation: 0.0, aspect: 1.0 };
    let new_app = || {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 256, "height": 128})).unwrap();
        app
    };
    let drag_guide = |modifiers: egui::Modifiers| {
        let builder = egui_kittest::Harness::builder().with_size(vec2(400.0, 300.0));
        let mut h = builder.build_ui_state(move |ui, app| draw_rulers(app, ui, full, &xf), new_app());
        // Out of the left ruler: a vertical guide, dropped at document x = 37.
        let (from, to) = (pos2(RULER / 2.0, 100.0), xf.to_screen(37.0, 64.0));
        h.event_modifiers(egui::Event::PointerMoved(from), modifiers);
        h.event_modifiers(egui::Event::PointerButton { pos: from, button: egui::PointerButton::Primary, pressed: true, modifiers }, modifiers);
        h.run_steps(1);
        h.event_modifiers(egui::Event::PointerMoved(to), modifiers);
        h.run_steps(1);
        h.event_modifiers(egui::Event::PointerButton { pos: to, button: egui::PointerButton::Primary, pressed: false, modifiers }, modifiers);
        h.run_steps(2);
        h.state().session.active().unwrap().doc.guides.vertical.clone()
    };
    assert_eq!(drag_guide(egui::Modifiers::NONE), vec![37.0]);
    // #5698: Alt turns it into a horizontal guide (at the drop's y, the document's centre).
    assert_eq!(drag_guide(egui::Modifiers::ALT), Vec::<f64>::new(), "no vertical guide with Alt");
    // Pixels at 100%: labels every 100 px, ticks every 10 px.
    assert_eq!(drag_guide(egui::Modifiers::SHIFT), vec![40.0]);

    let mut app = new_app();
    // At 200% the labels are every 50 px, so the ticks every 5 px.
    assert_eq!(snap_to_tick(&app, &ViewXform { zoom: 2.0, ..xf }, true, 37.0), 35.0);
    // Percent of each side: ticks every 2.5% of 256 px across, every 5% of 128 px down.
    app.run("prefs.set", json!({"path": "unitsAndRulers.rulers", "value": "percent"})).unwrap();
    assert!((snap_to_tick(&app, &xf, true, 37.0) - 38.4).abs() < 1e-9);
    assert!((snap_to_tick(&app, &xf, false, 37.0) - 38.4).abs() < 1e-9);
    // Ticks count from the zero point (#2955): a vertical guide follows the x origin only.
    let mut app = new_app();
    app.run("view.setRulerOrigin", json!({"x": 3, "y": 0})).unwrap();
    assert_eq!(snap_to_tick(&app, &xf, true, 37.0), 33.0);
    assert_eq!(snap_to_tick(&app, &xf, false, 37.0), 40.0);
}

/// #5034: ⌥ turns a guide dragged out of a ruler, as in Photoshop: out of the left ruler it
/// comes out horizontal, at the pointer's y.
#[test]
fn alt_turns_a_guide_dragged_out_of_a_ruler() {
    let full = Rect::from_min_size(Pos2::ZERO, vec2(400.0, 300.0));
    let rect = Rect::from_min_max(full.min + vec2(RULER, RULER), full.max);
    let xf = ViewXform { rect, zoom: 1.0, center: [128.0, 64.0], flip: false, rotation: 0.0, aspect: 1.0 };
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 256, "height": 128})).unwrap();
    let mut h = egui_kittest::Harness::builder().with_size(vec2(400.0, 300.0)).build_ui_state(move |ui, app| draw_rulers(app, ui, full, &xf), app);
    // ⌥ held for the whole drag (`event_modifiers` would let go of it after each event).
    let alt = egui::Modifiers::ALT;
    h.event(egui::Event::ModifiersChanged(alt));
    let (from, to) = (pos2(RULER / 2.0, 100.0), xf.to_screen(37.0, 51.0));
    h.event(egui::Event::PointerMoved(from));
    h.event(egui::Event::PointerButton { pos: from, button: egui::PointerButton::Primary, pressed: true, modifiers: alt });
    h.run_steps(1);
    h.event(egui::Event::PointerMoved(to));
    h.run_steps(1);
    assert!(h.state().guide_drag.is_some_and(|d| !d.shows_vertical()), "drawn horizontal while ⌥ is held");
    h.event(egui::Event::PointerButton { pos: to, button: egui::PointerButton::Primary, pressed: false, modifiers: alt });
    h.run_steps(2);
    let g = &h.state().session.active().unwrap().doc.guides;
    assert_eq!((g.vertical.clone(), g.horizontal.clone()), (vec![], vec![51.0]));
}

/// #5034: with the Move tool, ⌥ while dragging a guide turns it about the pointer, in one
/// undo step; letting go of ⌥ before the release turns it back. An ⌥-click turns it in place.
#[test]
fn alt_turns_a_guide_moved_with_the_move_tool() {
    use crate::canvas::{ToolEvent, tool_event};
    let new_app = || {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 80, "height": 60})).unwrap();
        app.sync_views();
        app.ui.tool = crate::state::Tool::Move;
        app.run("view.newGuide", json!({"orientation": "vertical", "position": 50})).unwrap();
        app
    };
    let guides = |app: &PhotocraftApp| {
        let g = &app.session.active().unwrap().doc.guides;
        (g.vertical.clone(), g.horizontal.clone())
    };
    let (none, alt) = (egui::Modifiers::NONE, egui::Modifiers::ALT);

    let mut app = new_app();
    tool_event(&mut app, ToolEvent::Down { x: 50.0, y: 44.0, pressure: 1.0 }, none);
    tool_event(&mut app, ToolEvent::Move { x: 63.0, y: 17.0, pressure: 1.0 }, alt);
    assert!(app.guide_drag.is_some_and(|d| !d.shows_vertical() && d.pos == 17.0));
    tool_event(&mut app, ToolEvent::Up { x: 63.0, y: 17.0 }, alt);
    assert_eq!(guides(&app), (vec![], vec![17.0]));
    assert!(app.session.undo());
    assert_eq!(guides(&app), (vec![50.0], vec![]), "one undo step");

    // ⌥ let go before the release: still vertical, moved along x.
    let mut app = new_app();
    tool_event(&mut app, ToolEvent::Down { x: 50.0, y: 44.0, pressure: 1.0 }, none);
    tool_event(&mut app, ToolEvent::Move { x: 63.0, y: 17.0, pressure: 1.0 }, alt);
    tool_event(&mut app, ToolEvent::Up { x: 63.0, y: 17.0 }, none);
    assert_eq!(guides(&app), (vec![63.0], vec![]));

    // An ⌥-click turns the guide where it was clicked.
    let mut app = new_app();
    tool_event(&mut app, ToolEvent::Down { x: 50.0, y: 44.0, pressure: 1.0 }, alt);
    tool_event(&mut app, ToolEvent::Up { x: 50.0, y: 44.0 }, alt);
    assert_eq!(guides(&app), (vec![], vec![44.0]));
}

/// #5034: a guide dropped on either ruler is deleted, whichever way it runs; a horizontal
/// guide over the left ruler is still level with the image, so this needs the pointer.
#[test]
fn a_guide_dropped_on_either_ruler_is_deleted() {
    let full = Rect::from_min_size(Pos2::ZERO, vec2(400.0, 300.0));
    let rect = Rect::from_min_max(full.min + vec2(RULER, RULER), full.max);
    let xf = ViewXform { rect, zoom: 1.0, center: [128.0, 64.0], flip: false, rotation: 0.0, aspect: 1.0 };
    for (pointer, over) in [(pos2(RULER / 2.0, 150.0), true), (pos2(200.0, RULER / 2.0), true), (pos2(200.0, 150.0), false)] {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 256, "height": 128})).unwrap();
        app.ui.extras.rulers = true;
        app.run("view.newGuide", json!({"orientation": "horizontal", "position": 30})).unwrap();
        app.guide_drag = Some(GuideDrag { vertical: false, index: Some(0), pos: 64.0, ..Default::default() });
        let mut h = egui_kittest::Harness::builder().with_size(vec2(400.0, 300.0)).build_ui_state(move |ui, app| draw_rulers(app, ui, full, &xf), app);
        h.event(egui::Event::PointerMoved(pointer));
        h.run_steps(1);
        let mut d = h.state().guide_drag.unwrap();
        // The canvas release owns pointer classification in the current routing.
        d.over_ruler = dropped_on_a_ruler(h.state(), full, pointer);
        assert_eq!(d.over_ruler, over, "{pointer:?}");
        let app = h.state_mut();
        app.guide_drag = None;
        finish_drag(app, d);
        let left = app.session.active().unwrap().doc.guides.horizontal.clone();
        assert_eq!(left, if over { vec![] } else { vec![64.0] }, "{pointer:?}");
        app.run("edit.undo", json!({})).unwrap();
        assert_eq!(app.session.active().unwrap().doc.guides.horizontal, vec![30.0], "one Undo restores the guide");
    }
}
