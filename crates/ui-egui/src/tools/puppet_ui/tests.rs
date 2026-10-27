use super::*;

fn puppet_app(ctx: &egui::Context) -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.session.execute("file.new", json!({"width": 100, "height": 80, "depth": 8})).unwrap();
    app.session.execute("layer.new.layer", json!({})).unwrap();
    app.session
        .edit("paint", |doc, active| {
            doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap().fill_rect(Rect::new(20, 30, 80, 50), &[1.0, 0.0, 0.0, 1.0]);
            Ok(())
        })
        .unwrap();
    app.sync_views();
    begin(&mut app, ctx, "edit.puppetWarp").unwrap();
    app
}

fn gesture(app: &mut PhotocraftApp, from: [f64; 2], to: [f64; 2]) {
    pointer(app, ToolEvent::Down { x: from[0], y: from[1], pressure: 1.0 }, egui::Modifiers::NONE);
    pointer(app, ToolEvent::Up { x: to[0], y: to[1] }, egui::Modifiers::NONE);
}

fn press(app: &mut PhotocraftApp, ctx: &egui::Context, key: egui::Key, modifiers: egui::Modifiers) {
    let events = [true, false].map(|pressed| egui::Event::Key { key, physical_key: None, pressed, repeat: false, modifiers }).to_vec();
    let input = egui::RawInput { events, ..Default::default() };
    ctx.run_ui(input, |ui| crate::shortcuts::handle(app, ui.ctx())).textures_delta.clear();
}

#[test]
fn delete_keys_remove_selected_pin_and_restore_its_deformation_on_undo() {
    for key in [egui::Key::Delete, egui::Key::Backspace] {
        let ctx = egui::Context::default();
        let mut app = puppet_app(&ctx);
        gesture(&mut app, [25.0, 40.0], [25.0, 40.0]);
        gesture(&mut app, [75.0, 40.0], [75.0, 40.0]);
        gesture(&mut app, [75.0, 40.0], [75.0, 30.0]);
        gesture(&mut app, [25.0, 40.0], [25.0, 20.0]);
        let s = app.distort.puppet.as_mut().unwrap();
        // Rotation/depth are part of a deleted pin's state, not just its position.
        let before = s.warp.pins.clone();
        s.warp.pins[0].rotate = Some(15.0);
        s.warp.pins[0].depth = 2;
        s.rebind();
        s.record(before);
        let pins = s.warp.pins.clone();
        let deformed = s.deformed.clone();
        let order = s.order.clone();
        let edits = s.undo.len();
        let survivor = &pins[1];
        let solver = PuppetSolver::new(s.mesh.clone(), &[survivor.src], &[survivor.rotate.is_some()]);
        let expected = solver.solve(&[survivor.dst], &[survivor.rotate], s.warp.mode, ITERATIONS, None).0;
        let past = app.session.active().unwrap().history.past_len();

        press(&mut app, &ctx, key, egui::Modifiers::NONE);
        let s = app.distort.puppet.as_ref().unwrap();
        assert_eq!(s.warp.pins, pins[1..], "{key:?}: delete the selected first pin, not the last");
        assert_eq!(s.selected, None);
        assert_eq!(s.deformed, expected, "solve from rest without the deleted constraint");
        assert_eq!(s.order, solver.draw_order(&[survivor.depth], &[survivor.src]));
        assert_eq!(s.undo.len(), edits + 1);
        press(&mut app, &ctx, key, egui::Modifiers::NONE);
        assert_eq!(app.distort.puppet.as_ref().unwrap().undo.len(), edits + 1, "no selection: no edit");

        press(&mut app, &ctx, egui::Key::Z, egui::Modifiers::COMMAND);
        let s = app.distort.puppet.as_ref().unwrap();
        assert_eq!(s.warp.pins, pins);
        assert_eq!(s.deformed, deformed);
        assert_eq!(s.order, order);
        press(&mut app, &ctx, egui::Key::Z, egui::Modifiers::COMMAND | egui::Modifiers::SHIFT);
        assert_eq!(app.distort.puppet.as_ref().unwrap().deformed, expected);
        assert_eq!(app.session.active().unwrap().history.past_len(), past, "pin edits stay in the open session");

        let original = app.session.active().unwrap().doc.clone();
        commit(&mut app);
        assert_eq!(app.session.active().unwrap().history.past_len(), past + 1);
        let committed = app.session.active().unwrap().doc.clone();
        assert_ne!(committed, original);
        app.session.execute("edit.undo", json!({})).unwrap();
        assert_eq!(app.session.active().unwrap().doc, original);
        app.session.execute("edit.redo", json!({})).unwrap();
        assert_eq!(app.session.active().unwrap().doc, committed);
    }
}

#[test]
fn deleting_during_a_drag_settles_history_and_cannot_move_the_next_pin() {
    let ctx = egui::Context::default();
    let mut app = puppet_app(&ctx);
    gesture(&mut app, [25.0, 40.0], [25.0, 40.0]);
    gesture(&mut app, [75.0, 40.0], [75.0, 40.0]);
    let original = app.distort.puppet.as_ref().unwrap().warp.pins.clone();
    pointer(&mut app, ToolEvent::Down { x: 25.0, y: 40.0, pressure: 1.0 }, egui::Modifiers::NONE);
    pointer(&mut app, ToolEvent::Move { x: 25.0, y: 20.0, pressure: 1.0 }, egui::Modifiers::NONE);
    let moved = app.distort.puppet.as_ref().unwrap().warp.pins.clone();
    press(&mut app, &ctx, egui::Key::Delete, egui::Modifiers::NONE);
    pointer(&mut app, ToolEvent::Move { x: 5.0, y: 5.0, pressure: 1.0 }, egui::Modifiers::NONE);
    pointer(&mut app, ToolEvent::Up { x: 5.0, y: 5.0 }, egui::Modifiers::NONE);
    let s = app.distort.puppet.as_mut().unwrap();
    assert_eq!(s.warp.pins, original[1..]);
    assert!(s.drag.is_none() && s.drag_from.is_none());
    assert!(s.undo());
    assert_eq!(s.warp.pins, moved, "first undo restores the deleted pin at its moved position");
    assert!(s.undo());
    assert_eq!(s.warp.pins, original, "second undo reverts the preceding drag");
    assert!(s.redo());
    s.selected = Some(0);
    control(&mut app, &json!({"deleteSelectedPin": true})).unwrap();
    assert!(!app.distort.puppet.as_mut().unwrap().redo(), "a new deletion discards the redo branch");
}

#[test]
fn deleting_last_pin_restores_rest_mesh_and_empty_commit_is_not_an_edit() {
    let ctx = egui::Context::default();
    let mut app = puppet_app(&ctx);
    gesture(&mut app, [25.0, 40.0], [25.0, 20.0]);
    let past = app.session.active().unwrap().history.past_len();
    let s = app.distort.puppet.as_mut().unwrap();
    let pins = s.warp.pins.clone();
    s.selected = Some(usize::MAX);
    press(&mut app, &ctx, egui::Key::Delete, egui::Modifiers::NONE);
    assert_eq!(app.distort.puppet.as_ref().unwrap().warp.pins, pins, "stale selection is harmless");
    app.distort.puppet.as_mut().unwrap().selected = Some(0);
    press(&mut app, &ctx, egui::Key::Backspace, egui::Modifiers::NONE);
    let s = app.distort.puppet.as_ref().unwrap();
    assert!(s.warp.pins.is_empty());
    assert_eq!(s.deformed.len(), s.mesh.verts.len());
    for (deformed, rest) in s.deformed.iter().zip(&s.mesh.verts) {
        assert!(dist(*deformed, *rest) < 1e-8, "no constraints: the mesh returns to rest");
    }
    commit(&mut app);
    assert!(app.distort.puppet.is_none());
    assert_eq!(app.session.active().unwrap().history.past_len(), past);
}

#[test]
fn delete_keys_in_a_text_field_do_not_remove_pins() {
    for key in [egui::Key::Delete, egui::Key::Backspace] {
        let ctx = egui::Context::default();
        let mut app = puppet_app(&ctx);
        gesture(&mut app, [25.0, 40.0], [25.0, 20.0]);
        let mut text = "123".to_string();
        ctx.run_ui(Default::default(), |ui| {
            ui.text_edit_singleline(&mut text).request_focus();
        })
        .textures_delta
        .clear();
        let pins = app.distort.puppet.as_ref().unwrap().warp.pins.clone();
        let input = egui::RawInput {
            events: vec![egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE }],
            ..Default::default()
        };
        ctx.run_ui(input, |ui| {
            assert!(ui.ctx().text_edit_focused());
            crate::shortcuts::handle(&mut app, ui.ctx());
            ui.text_edit_singleline(&mut text);
        })
        .textures_delta
        .clear();
        assert_eq!(app.distort.puppet.as_ref().unwrap().warp.pins, pins);
        assert_eq!(app.distort.puppet.as_ref().unwrap().undo.len(), 1);
    }
}

fn long_puppet(ctx: &egui::Context, mode: PuppetMode, density: PuppetDensity) -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.session.execute("file.new", json!({"width": 1000, "height": 300, "depth": 8})).unwrap();
    app.session.execute("layer.new.layer", json!({})).unwrap();
    app.session
        .edit("paint", |doc, active| {
            doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap().fill_rect(Rect::new(40, 180, 960, 220), &[1.0, 0.0, 0.0, 1.0]);
            Ok(())
        })
        .unwrap();
    app.sync_views();
    begin(&mut app, ctx, "edit.puppetWarp").unwrap();
    for x in [70.0, 160.0, 930.0] {
        pointer(&mut app, ToolEvent::Down { x, y: 200.0, pressure: 1.0 }, egui::Modifiers::NONE);
        pointer(&mut app, ToolEvent::Up { x, y: 200.0 }, egui::Modifiers::NONE);
    }
    let s = app.distort.puppet.as_mut().unwrap();
    s.warp.mode = mode;
    s.warp.density = density;
    s.rebuild_mesh();
    app
}

fn assert_same_mesh(a: &[[f64; 2]], b: &[[f64; 2]]) {
    assert_eq!(a.len(), b.len());
    let offset = a.iter().zip(b).map(|(a, b)| dist(*a, *b)).fold(0.0, f64::max);
    assert!(offset < 1e-8, "mesh moved by {offset} px with unchanged pins");
}

#[test]
fn drag_preview_matches_release_history_and_committed_geometry() {
    use photocraft_algo::puppet::{deform, render};
    for mode in [PuppetMode::Rigid, PuppetMode::Normal, PuppetMode::Distort] {
        for density in [PuppetDensity::Fewer, PuppetDensity::Normal, PuppetDensity::More] {
            let ctx = egui::Context::default();
            let mut app = long_puppet(&ctx, mode, density);
            let rest = app.distort.puppet.as_ref().unwrap().deformed.clone();
            let past = app.session.active().unwrap().history.past_len();
            pointer(&mut app, ToolEvent::Down { x: 930.0, y: 200.0, pressure: 1.0 }, egui::Modifiers::NONE);
            for step in 1..=12 {
                let x = 930.0 - f64::from(step) * 20.0;
                let y = 200.0 - f64::from(step) * 10.0;
                pointer(&mut app, ToolEvent::Move { x, y, pressure: 1.0 }, egui::Modifiers::NONE);
            }
            let s = app.distort.puppet.as_ref().unwrap();
            let preview = s.deformed.clone();
            let order = s.order.clone();
            pointer(&mut app, ToolEvent::Up { x: 690.0, y: 80.0 }, egui::Modifiers::NONE);
            let s = app.distort.puppet.as_mut().unwrap();
            assert_same_mesh(&preview, &s.deformed);
            assert_eq!(order, s.order);
            let (_, canonical, _) = deform(&s.surface, s.bounds, &s.warp, ITERATIONS);
            assert_same_mesh(&preview, &canonical);
            assert!(s.undo());
            assert_same_mesh(&rest, &s.deformed);
            assert!(s.redo());
            assert_same_mesh(&preview, &s.deformed);
            // Use the engine's interpolation to isolate geometry from GPU texture sampling.
            let expected = render(&s.surface, s.bounds, &s.mesh, &preview, &order, photocraft_algo::transform::Interp::Bicubic);
            let layer = s.layer;
            assert_eq!(app.session.active().unwrap().history.past_len(), past);
            commit(&mut app);
            let st = app.session.active().unwrap();
            assert_eq!(st.history.past_len(), past + 1);
            assert_eq!(st.doc.layer(layer).unwrap().surface().unwrap(), &expected);
        }
    }
}

#[test]
fn drag_preview_is_independent_of_pointer_path_and_honors_release_coordinates() {
    let ctx = egui::Context::default();
    let mut app = long_puppet(&ctx, PuppetMode::Normal, PuppetDensity::More);
    let s = app.distort.puppet.as_mut().unwrap();
    s.warp.pins[0].rotate = Some(25.0);
    s.rebind();
    let pins = s.warp.pins.clone();
    let paths = [
        vec![[690.0, 80.0]],
        vec![[870.0, 170.0], [810.0, 140.0], [750.0, 110.0], [690.0, 80.0]],
        vec![[500.0, 30.0], [930.0, 200.0], [1000.0, 270.0], [690.0, 80.0]],
    ];
    let mut expected = None;
    for path in paths {
        app.distort.puppet.as_mut().unwrap().restore(pins.clone());
        pointer(&mut app, ToolEvent::Down { x: 930.0, y: 200.0, pressure: 1.0 }, egui::Modifiers::NONE);
        for [x, y] in path {
            pointer(&mut app, ToolEvent::Move { x, y, pressure: 1.0 }, egui::Modifiers::NONE);
        }
        let preview = &app.distort.puppet.as_ref().unwrap().deformed;
        assert_same_mesh(expected.get_or_insert_with(|| preview.clone()), preview);
        // The last pointer-up may carry a new position without a preceding move event.
        pointer(&mut app, ToolEvent::Up { x: 720.0, y: 90.0 }, egui::Modifiers::NONE);
        let s = app.distort.puppet.as_ref().unwrap();
        assert_eq!(s.warp.pins[2].dst, [720.0, 90.0]);
        let (_, canonical, _) = photocraft_algo::puppet::deform(&s.surface, s.bounds, &s.warp, ITERATIONS);
        assert_same_mesh(&s.deformed, &canonical);
    }
}

/// #5092: a Puppet Warp session left open does not follow the user to a new document.
#[test]
fn session_is_dropped_when_another_document_becomes_active() {
    let ctx = egui::Context::default();
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.session.execute("file.new", json!({"width": 100, "height": 80, "depth": 8})).unwrap();
    app.session.execute("layer.new.layer", json!({})).unwrap();
    app.session
        .edit("paint", |doc, active| {
            doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap().fill_rect(Rect::new(20, 30, 80, 50), &[1.0, 0.0, 0.0, 1.0]);
            Ok(())
        })
        .unwrap();
    app.sync_views();
    crate::distort_ui::menu(&mut app, &ctx, "edit.puppetWarp", &json!({})).unwrap().unwrap();
    crate::distort_ui::drop_foreign(&mut app);
    assert!(app.distort.puppet.is_some());
    app.session.execute("file.new", json!({"width": 32, "height": 32, "depth": 8})).unwrap();
    let active = app.session.active_index().unwrap();
    // The session stays until drop_foreign; the preview must not follow this document (#5115).
    assert!(app.distort.puppet.is_some());
    assert!(crate::distort_ui::display_doc(&app, active).is_none());
    crate::distort_ui::drop_foreign(&mut app);
    assert!(app.distort.puppet.is_none());
    assert!(crate::distort_ui::display_doc(&app, active).is_none());
    // Closing the document it was started on drops it too.
    assert!(app.session.set_active(0));
    crate::distort_ui::menu(&mut app, &ctx, "edit.puppetWarp", &json!({})).unwrap().unwrap();
    crate::distort_ui::drop_foreign(&mut app);
    assert!(app.distort.puppet.is_some());
    assert!(app.session.close(0).is_some());
    crate::distort_ui::drop_foreign(&mut app);
    assert!(app.distort.puppet.is_none());
}

/// #4440: ⌘Z / ⌘⇧Z undo and redo pin edits while Puppet Warp is open: placing a pin,
/// dragging one and removing one are a step each; a click that moves nothing isn't one.
#[test]
fn pin_edits_undo_and_redo_inside_the_session() {
    let ctx = egui::Context::default();
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.session.execute("file.new", json!({"width": 100, "height": 80, "depth": 8})).unwrap();
    app.session.execute("layer.new.layer", json!({})).unwrap();
    app.session
        .edit("paint", |doc, active| {
            doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap().fill_rect(Rect::new(20, 30, 80, 50), &[1.0, 0.0, 0.0, 1.0]);
            Ok(())
        })
        .unwrap();
    app.sync_views();
    crate::distort_ui::menu(&mut app, &ctx, "edit.puppetWarp", &json!({})).unwrap().unwrap();
    let ev = |app: &mut PhotocraftApp, e| crate::distort_ui::pointer(app, e, egui::Modifiers::NONE);
    let pins = |app: &PhotocraftApp| app.distort.puppet.as_ref().unwrap().warp.pins.iter().map(|p| p.dst).collect::<Vec<_>>();
    ev(&mut app, ToolEvent::Down { x: 25.0, y: 40.0, pressure: 1.0 });
    ev(&mut app, ToolEvent::Up { x: 25.0, y: 40.0 });
    ev(&mut app, ToolEvent::Down { x: 75.0, y: 40.0, pressure: 1.0 });
    ev(&mut app, ToolEvent::Up { x: 75.0, y: 40.0 });
    // Drag the second pin up; then a click on it that moves nothing.
    ev(&mut app, ToolEvent::Down { x: 75.0, y: 40.0, pressure: 1.0 });
    ev(&mut app, ToolEvent::Move { x: 75.0, y: 30.0, pressure: 1.0 });
    ev(&mut app, ToolEvent::Up { x: 75.0, y: 20.0 });
    ev(&mut app, ToolEvent::Down { x: 75.0, y: 20.0, pressure: 1.0 });
    ev(&mut app, ToolEvent::Up { x: 75.0, y: 20.0 });
    crate::distort_ui::pointer(&mut app, ToolEvent::Down { x: 25.0, y: 40.0, pressure: 1.0 }, egui::Modifiers { alt: true, ..Default::default() });
    assert_eq!(pins(&app), [[75.0, 20.0]]);
    // ⌘Z, through the keyboard: the removed pin comes back, then the drag is undone.
    let press = |app: &mut PhotocraftApp, modifiers: egui::Modifiers| {
        let ev = egui::Event::Key { key: egui::Key::Z, physical_key: None, pressed: true, repeat: false, modifiers };
        let input = egui::RawInput { events: vec![ev], ..Default::default() };
        ctx.run_ui(input, |ui| assert!(crate::distort_ui::keys(app, ui.ctx()))).textures_delta.clear();
    };
    press(&mut app, egui::Modifiers::COMMAND);
    assert_eq!(pins(&app), [[25.0, 40.0], [75.0, 20.0]]);
    press(&mut app, egui::Modifiers::COMMAND);
    assert_eq!(pins(&app), [[25.0, 40.0], [75.0, 40.0]], "the drag, not the click after it");
    // ⌘⇧Z redoes it; a new edit after an undo drops what was undone.
    press(&mut app, egui::Modifiers::COMMAND | egui::Modifiers::SHIFT);
    assert_eq!(pins(&app), [[25.0, 40.0], [75.0, 20.0]]);
    control(&mut app, &json!({"undo": true})).unwrap();
    control(&mut app, &json!({"undo": true})).unwrap();
    assert_eq!(pins(&app), [[25.0, 40.0]]);
    ev(&mut app, ToolEvent::Down { x: 50.0, y: 40.0, pressure: 1.0 });
    ev(&mut app, ToolEvent::Up { x: 50.0, y: 40.0 });
    control(&mut app, &json!({"redo": true})).unwrap();
    assert_eq!(pins(&app), [[25.0, 40.0], [50.0, 40.0]], "nothing left to redo");
    // Undo past the first pin stops there, and the document's History is untouched.
    let past = app.session.active().unwrap().history.past_len();
    for _ in 0..5 {
        control(&mut app, &json!({"undo": true})).unwrap();
    }
    assert!(pins(&app).is_empty());
    assert_eq!(app.session.active().unwrap().history.past_len(), past);
}

#[test]
fn pins_drag_and_commit_through_the_engine() {
    let ctx = egui::Context::default();
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.session.execute("file.new", json!({"width": 100, "height": 80, "depth": 8})).unwrap();
    app.session.execute("layer.new.layer", json!({})).unwrap();
    app.session
        .edit("paint", |doc, active| {
            doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap().fill_rect(Rect::new(20, 30, 80, 50), &[1.0, 0.0, 0.0, 1.0]);
            Ok(())
        })
        .unwrap();
    app.sync_views();
    crate::distort_ui::menu(&mut app, &ctx, "edit.puppetWarp", &json!({})).unwrap().unwrap();
    let ev = |app: &mut PhotocraftApp, e| crate::distort_ui::pointer(app, e, egui::Modifiers::NONE);
    // Two pins: hold the left end, drag the right end up.
    ev(&mut app, ToolEvent::Down { x: 25.0, y: 40.0, pressure: 1.0 });
    ev(&mut app, ToolEvent::Up { x: 25.0, y: 40.0 });
    ev(&mut app, ToolEvent::Down { x: 75.0, y: 40.0, pressure: 1.0 });
    ev(&mut app, ToolEvent::Move { x: 75.0, y: 30.0, pressure: 1.0 });
    ev(&mut app, ToolEvent::Up { x: 75.0, y: 20.0 });
    let s = app.distort.puppet.as_ref().unwrap();
    assert_eq!(s.warp.pins.len(), 2);
    assert_eq!(s.warp.pins[1].dst, [75.0, 20.0]);
    // ⌥-click removes; clicking off the mesh adds nothing.
    ev(&mut app, ToolEvent::Down { x: 5.0, y: 5.0, pressure: 1.0 });
    assert_eq!(app.distort.puppet.as_ref().unwrap().warp.pins.len(), 2);
    crate::distort_ui::pointer(&mut app, ToolEvent::Down { x: 25.0, y: 40.0, pressure: 1.0 }, egui::Modifiers { alt: true, ..Default::default() });
    assert_eq!(app.distort.puppet.as_ref().unwrap().warp.pins.len(), 1);
    ev(&mut app, ToolEvent::Down { x: 25.0, y: 40.0, pressure: 1.0 });
    ev(&mut app, ToolEvent::Up { x: 25.0, y: 40.0 });
    control(&mut app, &json!({"mode": "rigid", "density": "more"})).unwrap();
    let expected = app.distort.puppet.as_ref().unwrap().deformed.clone();
    assert!(!expected.is_empty());
    let before = app.session.active().unwrap().history.past_len();
    crate::distort_ui::menu(&mut app, &ctx, "edit.puppetWarp", &json!({"ui": {"commit": true}})).unwrap().unwrap();
    assert!(app.distort.puppet.is_none());
    assert_eq!(app.session.active().unwrap().history.past_len(), before + 1);
    let st = app.session.active().unwrap();
    let b = st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap().content_bounds();
    assert!(b.y0 < 25, "the right end went up: {b:?}");
}

#[test]
fn an_expansion_that_empties_the_mesh_is_refused() {
    // #951: remeshing to no vertices rebound the pins to a vertex that doesn't exist and
    // panicked. -50 is the options bar's minimum and also empties this 20 px tall bar.
    let ctx = egui::Context::default();
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.session.execute("file.new", json!({"width": 100, "height": 80, "depth": 8})).unwrap();
    app.session.execute("layer.new.layer", json!({})).unwrap();
    app.session
        .edit("paint", |doc, active| {
            doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap().fill_rect(Rect::new(20, 30, 80, 50), &[1.0, 0.0, 0.0, 1.0]);
            Ok(())
        })
        .unwrap();
    app.sync_views();
    crate::distort_ui::menu(&mut app, &ctx, "edit.puppetWarp", &json!({})).unwrap().unwrap();
    crate::distort_ui::pointer(&mut app, ToolEvent::Down { x: 50.0, y: 40.0, pressure: 1.0 }, egui::Modifiers::NONE);
    crate::distort_ui::pointer(&mut app, ToolEvent::Up { x: 50.0, y: 40.0 }, egui::Modifiers::NONE);
    let before = app.distort.puppet.as_ref().unwrap().describe();
    for e in [-200, -50] {
        assert!(control(&mut app, &json!({"expansion": e})).is_err(), "{e}");
        assert_eq!(app.distort.puppet.as_ref().unwrap().describe(), before, "{e}: the previous settings and mesh stay");
    }
    // A smaller shrink still remeshes.
    let v = control(&mut app, &json!({"expansion": -2})).unwrap();
    assert_eq!(v["expansion"], -2.0);
    assert!(v["triangles"].as_u64().unwrap() > 0);
}
