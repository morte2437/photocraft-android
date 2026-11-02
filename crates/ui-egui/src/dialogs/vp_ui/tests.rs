use super::*;

fn app_with_layer() -> (egui::Context, PhotocraftApp) {
    let ctx = egui::Context::default();
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.session.execute("file.new", json!({"width": 240, "height": 160, "depth": 16})).unwrap();
    app.session.execute("layer.new.layer", json!({})).unwrap();
    app.session.execute("edit.fill", json!({"color": "#4080c0"})).unwrap();
    app.sync_views();
    (ctx, app)
}

#[test]
fn four_clicks_make_a_plane_and_ok_pastes_in_one_step() {
    let (ctx, mut app) = app_with_layer();
    // Something to paste: a red square on the clipboard.
    app.session.execute("select.rect", json!({"x": 0, "y": 0, "width": 20, "height": 20})).unwrap();
    app.session.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
    app.session.execute("edit.copy", json!({})).unwrap();
    app.session.execute("select.deselect", json!({})).unwrap();
    crate::distort_ui::menu(&mut app, &ctx, VP, &json!({})).unwrap().unwrap();
    let click = |app: &mut PhotocraftApp, x: f64, y: f64| {
        crate::distort_ui::pointer(app, ToolEvent::Down { x, y, pressure: 1.0 }, egui::Modifiers::NONE);
        crate::distort_ui::pointer(app, ToolEvent::Up { x, y }, egui::Modifiers::NONE);
    };
    // A floor receding to the top, clicked counter-clockwise from the bottom-left.
    for (x, y) in [(20.0, 150.0), (220.0, 150.0), (170.0, 60.0), (70.0, 60.0)] {
        click(&mut app, x, y);
    }
    let s = app.distort.vanishing.as_ref().unwrap();
    assert_eq!(s.planes.len(), 1);
    assert_eq!(s.planes[0].corners, [[70.0, 60.0], [170.0, 60.0], [220.0, 150.0], [20.0, 150.0]], "clockwise from the top-left");
    assert_eq!(s.tool, VpTool::EditPlane);
    assert_eq!(s.describe()["planes"][0]["check"], "valid");
    // Edit Plane: drag the top edge node up; the edge slides along the perspective.
    let top = s.edge_nodes(0).unwrap()[0];
    crate::distort_ui::pointer(&mut app, ToolEvent::Down { x: top[0], y: top[1], pressure: 1.0 }, egui::Modifiers::NONE);
    crate::distort_ui::pointer(&mut app, ToolEvent::Up { x: top[0], y: top[1] - 15.0 }, egui::Modifiers::NONE);
    let c = app.distort.vanishing.as_ref().unwrap().planes[0].corners;
    assert!(c[0][1] < 60.0 && c[1][1] < 60.0 && c[1][0] - c[0][0] < 100.0, "taller and narrower at the far edge: {c:?}");
    assert_eq!(c[2], [220.0, 150.0]);
    let before = app.session.active().unwrap().history.past_len();
    commit(&mut app).unwrap();
    assert!(app.distort.vanishing.is_none());
    assert_eq!(app.session.active().unwrap().history.past_len(), before + 1);
    // The paste landed on the plane: red at its middle, blue outside it.
    let st = app.session.active().unwrap();
    let l = st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap();
    let h = VpPlane { corners: c }.homography().unwrap();
    let (x, y) = h.apply(0.5, 0.35);
    assert!(l.rgba(x as i32, y as i32)[0] > 0.9, "{:?}", l.rgba(x as i32, y as i32));
    assert!(l.rgba(230, 20)[0] < 0.5, "outside the plane");
}

#[test]
fn ctrl_drag_tears_off_a_wall_and_the_stamp_clones_in_one_step() {
    let (ctx, mut app) = app_with_layer();
    // A green mark near the floor's front edge to clone.
    app.session.execute("select.rect", json!({"x": 100, "y": 135, "width": 40, "height": 8})).unwrap();
    app.session.execute("edit.fill", json!({"color": "#00ff00"})).unwrap();
    app.session.execute("select.deselect", json!({})).unwrap();
    crate::distort_ui::menu(&mut app, &ctx, VP, &json!({})).unwrap().unwrap();
    control(&mut app, &json!({"planes": [{"corners": [[70, 60], [170, 60], [220, 150], [20, 150]]}]})).unwrap();
    let ev = |app: &mut PhotocraftApp, e: ToolEvent, m: egui::Modifiers| crate::distort_ui::pointer(app, e, m);
    let drag = |app: &mut PhotocraftApp, a: [f64; 2], b: [f64; 2], m: egui::Modifiers| {
        ev(app, ToolEvent::Down { x: a[0], y: a[1], pressure: 1.0 }, m);
        ev(app, ToolEvent::Move { x: (a[0] + b[0]) / 2.0, y: (a[1] + b[1]) / 2.0, pressure: 1.0 }, m);
        ev(app, ToolEvent::Up { x: b[0], y: b[1] }, m);
    };
    fn vp(app: &PhotocraftApp) -> &VpSession {
        app.distort.vanishing.as_ref().unwrap()
    }

    // Ctrl/Cmd-drag the top edge node up: a wall torn off the floor's far edge, 90° to it.
    let top = vp(&app).edge_nodes(0).unwrap()[0];
    drag(&mut app, top, [top[0], top[1] - 35.0], egui::Modifiers::COMMAND);
    let s = vp(&app);
    assert_eq!(s.planes.len(), 2);
    let t = s.torn[1].unwrap();
    assert_eq!((t.from, t.edge, t.angle), (0, Edge::Top, 90.0));
    let c = s.planes[1].corners;
    assert!((c[0][1] + c[1][1]) / 2.0 < 40.0, "the far edge followed the pointer up: {c:?}");
    assert_ne!(s.describe()["planes"][1]["check"], "invalid");
    assert_eq!(s.params()["planes"][1], json!({"from": 0, "edge": "top", "angle": 90.0, "depth": t.depth}));
    // Alt-dragging its far edge node turns it; editing the floor carries it along.
    let far = s.edge_nodes(1).unwrap()[0];
    drag(&mut app, far, [far[0], far[1] + 12.0], egui::Modifiers::ALT);
    assert_ne!(vp(&app).torn[1].unwrap().angle, 90.0);
    let before = vp(&app).planes[1].corners;
    drag(&mut app, [220.0, 150.0], [228.0, 150.0], egui::Modifiers::NONE);
    assert_ne!(vp(&app).planes[1].corners, before);
    assert!(vp(&app).torn[1].is_some());

    // Stamp: painting needs a source first; Alt-click sets it on the floor's green mark.
    control(&mut app, &json!({"tool": "stamp", "stamp": {"diameter": 12, "hardness": 80}})).unwrap();
    drag(&mut app, [120.0, 100.0], [120.0, 100.0], egui::Modifiers::NONE);
    assert!(vp(&app).strokes.is_empty() && app.ui.status.contains("Alt-click"), "{}", app.ui.status);
    drag(&mut app, [120.0, 139.0], [120.0, 139.0], egui::Modifiers::ALT);
    assert_eq!(vp(&app).source, Some([120.0, 139.0]));
    drag(&mut app, [118.0, 100.0], [126.0, 100.0], egui::Modifiers::NONE);
    assert_eq!(vp(&app).strokes.len(), 1);
    assert!(vp(&app).strokes[0].points.len() >= 3, "dabs along the drag");
    // The canvas shows the stroke; the document is untouched until OK.
    let layer = app.session.active().unwrap().active_layer.unwrap();
    let green = |d: &Document| d.layer(layer).unwrap().surface().unwrap().rgba(122, 100)[1];
    let (shown, key) = crate::distort_ui::display_doc(&app, 0).expect("a stamp preview");
    assert!(green(&shown) > 0.8, "{}", green(&shown));
    let st = app.session.active().unwrap();
    assert!(green(&st.doc) < 0.6);
    let area = damage(&app, st.doc.id, st.revision, 0, key).unwrap();
    assert!(area.contains(122, 100) && !area.contains(20, 20), "only the stroke recomposites: {area:?}");

    let before = app.session.active().unwrap().history.past_len();
    let r = commit(&mut app).unwrap();
    assert_eq!(r["committed"], true, "{r}");
    let st = app.session.active().unwrap();
    assert_eq!(st.history.past_len(), before + 1, "one history step for the whole session");
    assert!(green(&st.doc) > 0.8, "{}", green(&st.doc));
}

#[test]
fn undo_steps_back_through_clone_strokes_and_tear_offs() {
    let (ctx, mut app) = app_with_layer();
    crate::distort_ui::menu(&mut app, &ctx, VP, &json!({})).unwrap().unwrap();
    control(&mut app, &json!({"planes": [{"corners": [[70, 60], [170, 60], [220, 150], [20, 150]]}]})).unwrap();
    let drag = |app: &mut PhotocraftApp, a: [f64; 2], b: [f64; 2], m: egui::Modifiers| {
        crate::distort_ui::pointer(app, ToolEvent::Down { x: a[0], y: a[1], pressure: 1.0 }, m);
        crate::distort_ui::pointer(app, ToolEvent::Move { x: b[0], y: b[1], pressure: 1.0 }, m);
        crate::distort_ui::pointer(app, ToolEvent::Up { x: b[0], y: b[1] }, m);
    };
    let step = |app: &mut PhotocraftApp, st: Step| {
        app.distort.vanishing.as_mut().unwrap().step(st);
        sync_preview(app);
    };
    let top = app.distort.vanishing.as_ref().unwrap().edge_nodes(0).unwrap()[0];
    drag(&mut app, top, [top[0], top[1] - 35.0], egui::Modifiers::COMMAND);
    control(&mut app, &json!({"tool": "stamp", "source": [120, 139]})).unwrap();
    drag(&mut app, [118.0, 100.0], [126.0, 100.0], egui::Modifiers::NONE);
    assert_eq!(app.distort.vanishing.as_ref().unwrap().strokes.len(), 1);
    assert!(crate::distort_ui::display_doc(&app, 0).is_some());

    // ⌘Z takes the stroke back (and its preview), then the source, then the torn-off wall.
    step(&mut app, Step::Undo);
    assert!(app.distort.vanishing.as_ref().unwrap().strokes.is_empty());
    assert!(crate::distort_ui::display_doc(&app, 0).is_none(), "no stamp preview left");
    step(&mut app, Step::Undo);
    assert_eq!(app.distort.vanishing.as_ref().unwrap().source, None);
    step(&mut app, Step::Undo);
    let s = app.distort.vanishing.as_ref().unwrap();
    assert_eq!((s.planes.len(), s.torn.len()), (1, 1));
    // ⇧⌘Z brings them back.
    for _ in 0..3 {
        step(&mut app, Step::Redo);
    }
    let s = app.distort.vanishing.as_ref().unwrap();
    assert_eq!((s.planes.len(), s.strokes.len()), (2, 1));
    assert!(s.torn[1].is_some());
    assert!(crate::distort_ui::display_doc(&app, 0).is_some(), "the stroke shows again");
}
#[test]
fn overlapping_stamp_preview_matches_commit_across_batches() {
    for new_layer in [false, true] {
        let (ctx, mut app) = app_with_layer();
        app.session
            .edit("pattern", |doc, active| {
                let surf = doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap();
                for x in 0..240 {
                    surf.fill_rect(Rect::new(x, 0, x + 1, 160), &[x as f32 / 240.0, (x % 7) as f32 / 6.0, 0.3, 1.0]);
                }
                Ok(())
            })
            .unwrap();
        crate::distort_ui::menu(&mut app, &ctx, VP, &json!({})).unwrap().unwrap();
        control(&mut app, &json!({"planes": [{"corners": [[10, 10], [230, 10], [230, 150], [10, 150]]}]})).unwrap();
        let points = [[80.0, 80.0], [84.0, 80.0], [88.0, 80.0], [92.0, 80.0]];
        let s = app.distort.vanishing.as_mut().unwrap();
        s.new_layer = new_layer;
        s.strokes.push(CloneSpec { source: [76.0, 80.0], points: points[..2].to_vec(), size: 12.0, hardness: 0.8, opacity: 1.0 });
        assert!(s.paint_preview(|| 1));
        s.strokes[0].points.extend_from_slice(&points[2..]);
        assert!(s.paint_preview(|| 2));
        let (shown, _) = s.shown().unwrap();
        let expected = photocraft_compose::flatten(&shown).px;
        assert_ne!(expected, photocraft_compose::flatten(&app.session.active().unwrap().doc).px);
        commit(&mut app).unwrap();
        assert_eq!(photocraft_compose::flatten(&app.session.active().unwrap().doc).px, expected, "newLayer={new_layer}");
    }
}

/// A red square on the clipboard (copied from the layer's top-left corner).
fn copy_red_square(app: &mut PhotocraftApp) {
    app.session.execute("select.rect", json!({"x": 0, "y": 0, "width": 40, "height": 40})).unwrap();
    app.session.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
    app.session.execute("edit.copy", json!({})).unwrap();
    app.session.execute("select.deselect", json!({})).unwrap();
}

#[test]
fn stamp_preview_paints_over_the_paste_like_ok() {
    for new_layer in [false, true] {
        let (ctx, mut app) = app_with_layer();
        copy_red_square(&mut app);
        crate::distort_ui::menu(&mut app, &ctx, VP, &json!({})).unwrap().unwrap();
        control(&mut app, &json!({"planes": [{"corners": [[10, 10], [230, 10], [230, 150], [10, 150]]}], "newLayer": new_layer})).unwrap();
        let s = app.distort.vanishing.as_ref().unwrap();
        assert!(s.paste && s.paste_plane() == Some(0), "the clipboard pastes by default");
        // The paste covers x 65..175 from y 45 down. One stroke clones it onto the base
        // layer, the other paints across its right edge.
        control(
            &mut app,
            &json!({"clone": [
                {"source": [100, 60], "points": [[40, 120], [48, 120], [56, 120]], "size": 16, "hardness": 80},
                {"source": [40, 130], "points": [[150, 60], [170, 60], [190, 60]], "size": 16, "hardness": 80},
            ]}),
        )
        .unwrap();
        let s = app.distort.vanishing.as_ref().unwrap();
        assert!(s.preview_has_paste(), "the preview shows the paste, so the overlay isn't drawn over the strokes");
        let (shown, _) = s.shown().unwrap();
        let expected = photocraft_compose::flatten(&shown).px;
        commit(&mut app).unwrap();
        assert_eq!(photocraft_compose::flatten(&app.session.active().unwrap().doc).px, expected, "newLayer={new_layer}");
    }
}

#[test]
fn moving_the_paste_repaints_the_stamp_preview() {
    let (ctx, mut app) = app_with_layer();
    copy_red_square(&mut app);
    crate::distort_ui::menu(&mut app, &ctx, VP, &json!({})).unwrap().unwrap();
    control(&mut app, &json!({"planes": [{"corners": [[10, 10], [230, 10], [230, 150], [10, 150]]}]})).unwrap();
    control(&mut app, &json!({"clone": [{"source": [200, 130], "points": [[30, 130]], "size": 8}]})).unwrap();
    let red = |app: &PhotocraftApp, x: i32, y: i32| {
        let (shown, _) = app.distort.vanishing.as_ref().unwrap().shown().unwrap();
        let l = app.session.active().unwrap().active_layer.unwrap();
        shown.layer(l).unwrap().surface().unwrap().rgba(x, y)[0]
    };
    assert!(red(&app, 100, 60) > 0.9);
    control(&mut app, &json!({"pasteAt": [0.0, 0.5]})).unwrap();
    assert!(red(&app, 100, 60) < 0.5, "the paste left its old place");
    assert!(red(&app, 30, 100) > 0.9, "and shows at the new one");
    control(&mut app, &json!({"paste": false})).unwrap();
    assert!(red(&app, 30, 100) < 0.5, "paste off: gone from the preview");
    assert!(!app.distort.vanishing.as_ref().unwrap().preview_has_paste());
}

#[test]
fn dragging_the_paste_shows_it_once() {
    let (ctx, mut app) = app_with_layer();
    copy_red_square(&mut app);
    crate::distort_ui::menu(&mut app, &ctx, VP, &json!({})).unwrap().unwrap();
    control(&mut app, &json!({"planes": [{"corners": [[10, 10], [230, 10], [230, 150], [10, 150]]}], "tool": "edit"})).unwrap();
    control(&mut app, &json!({"clone": [{"source": [200, 130], "points": [[30, 130]], "size": 8}]})).unwrap();
    let layer = app.session.active().unwrap().active_layer.unwrap();
    let red = |app: &PhotocraftApp, x: i32, y: i32| {
        let (shown, _) = crate::distort_ui::display_doc(app, 0)?;
        Some(shown.layer(layer).unwrap().surface().unwrap().rgba(x, y)[0])
    };
    assert!(red(&app, 100, 60).unwrap() > 0.9, "the paste is in the preview");
    let ev = |app: &mut PhotocraftApp, e: ToolEvent| crate::distort_ui::pointer(app, e, egui::Modifiers::NONE);
    ev(&mut app, ToolEvent::Down { x: 100.0, y: 60.0, pressure: 1.0 });
    ev(&mut app, ToolEvent::Move { x: 60.0, y: 100.0, pressure: 1.0 });
    // Mid-drag: no copy left behind at the old place, only the overlay at the pointer.
    assert!(red(&app, 100, 60).is_none_or(|r| r < 0.5), "{:?}", red(&app, 100, 60));
    assert!(app.distort.vanishing.as_ref().unwrap().paste_overlay(), "the overlay draws the moving paste");
    ev(&mut app, ToolEvent::Up { x: 60.0, y: 100.0 });
    assert!(red(&app, 100, 60).unwrap() < 0.5);
    assert!(red(&app, 60, 100).unwrap() > 0.9, "released: the preview pastes at the new place");
    assert!(!app.distort.vanishing.as_ref().unwrap().paste_overlay());
    // A plane corner drag freezes the preview, paste included, until release: one copy.
    ev(&mut app, ToolEvent::Down { x: 230.0, y: 150.0, pressure: 1.0 });
    ev(&mut app, ToolEvent::Move { x: 236.0, y: 156.0, pressure: 1.0 });
    let s = app.distort.vanishing.as_ref().unwrap();
    assert!(matches!(s.drag, Some(Drag::Corner { .. })));
    assert!(red(&app, 60, 100).unwrap() > 0.9 && !s.paste_overlay(), "frozen preview, no overlay on top");
    ev(&mut app, ToolEvent::Up { x: 236.0, y: 156.0 });
    let s = app.distort.vanishing.as_ref().unwrap();
    assert!(s.preview_has_paste() && !s.paste_overlay());
}

#[test]
fn new_layer_stamp_preview_stays_on_the_canvas() {
    let (ctx, mut app) = app_with_layer();
    // A red stripe where the first dab samples, so cloning it changes pixels.
    app.session.execute("select.rect", json!({"x": 130, "y": 70, "width": 20, "height": 20})).unwrap();
    app.session.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
    app.session.execute("select.deselect", json!({})).unwrap();
    crate::distort_ui::menu(&mut app, &ctx, VP, &json!({})).unwrap().unwrap();
    control(&mut app, &json!({"planes": [{"corners": [[10, 10], [230, 10], [230, 150], [10, 150]]}], "newLayer": true})).unwrap();
    // The second dab lies left of the canvas and samples the canvas 100 px to its right.
    control(&mut app, &json!({"clone": [{"source": [140, 80], "points": [[40, 80], [-30, 80]], "size": 16}]})).unwrap();
    let s = app.distort.vanishing.as_ref().unwrap();
    let out = s.preview.as_ref().unwrap().out.content_bounds();
    assert!(!out.is_empty() && s.canvas.contains_rect(&out), "{out:?}");
    let st = app.session.active().unwrap();
    let area = damage(&app, st.doc.id, st.revision, 0, s.shown().unwrap().1).unwrap();
    assert!(s.canvas.contains_rect(&area), "{area:?}");
}

#[test]
fn deleting_the_last_plane_drops_the_stamp_preview() {
    let (ctx, mut app) = app_with_layer();
    crate::distort_ui::menu(&mut app, &ctx, VP, &json!({})).unwrap().unwrap();
    control(&mut app, &json!({"planes": [{"corners": [[70, 60], [170, 60], [220, 150], [20, 150]]}]})).unwrap();
    control(&mut app, &json!({"clone": [{"source": [120, 139], "points": [[118, 100], [126, 100]], "size": 12}]})).unwrap();
    assert!(crate::distort_ui::display_doc(&app, 0).is_some());
    let s = app.distort.vanishing.as_mut().unwrap();
    s.selected = Some(0);
    s.delete_selected();
    sync_preview(&mut app);
    assert!(app.distort.vanishing.as_ref().unwrap().planes.is_empty());
    assert!(crate::distort_ui::display_doc(&app, 0).is_none(), "no strokes shown without a plane");
}

/// Opens the mode on the active layer of `app_with_layer` and stamps one stroke.
fn stamp_a_stroke(app: &mut PhotocraftApp, ctx: &egui::Context) {
    crate::distort_ui::menu(app, ctx, VP, &json!({})).unwrap().unwrap();
    control(app, &json!({"planes": [{"corners": [[10, 10], [230, 10], [230, 150], [10, 150]]}]})).unwrap();
    control(app, &json!({"clone": [{"source": [60, 80], "points": [[120, 80], [126, 80]], "size": 12}]})).unwrap();
}

#[test]
fn stamp_preview_is_not_shown_on_another_document() {
    let (ctx, mut app) = app_with_layer();
    stamp_a_stroke(&mut app, &ctx);
    assert!(crate::distort_ui::display_doc(&app, 0).is_some());
    app.session.execute("file.new", json!({"width": 32, "height": 32, "depth": 8})).unwrap();
    let active = app.session.active_index().unwrap();
    // The session stays until drop_foreign; the preview must not follow this document (#5115).
    assert!(app.distort.vanishing.is_some());
    assert!(crate::distort_ui::display_doc(&app, active).is_none());
}

#[test]
fn ok_paints_the_layer_the_mode_was_opened_on() {
    for new_layer in [false, true] {
        let (ctx, mut app) = app_with_layer();
        let layer = app.session.active().unwrap().active_layer.unwrap();
        app.session
            .edit("stripe", |doc, active| {
                doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap().fill_rect(Rect::new(50, 70, 70, 90), &[1.0, 0.0, 0.0, 1.0]);
                Ok(())
            })
            .unwrap();
        stamp_a_stroke(&mut app, &ctx);
        app.distort.vanishing.as_mut().unwrap().new_layer = new_layer;
        sync_preview(&mut app);
        // Another layer is picked while the mode is open (it isn't modal).
        app.session.execute("layer.new.layer", json!({"name": "other"})).unwrap();
        let other = app.session.active().unwrap().active_layer.unwrap();
        sync_preview(&mut app);
        let expected = photocraft_compose::flatten(&crate::distort_ui::display_doc(&app, 0).unwrap().0).px;
        commit(&mut app).unwrap();
        let d = &app.session.active().unwrap().doc;
        assert!(d.layer(other).unwrap().surface().unwrap().content_bounds().is_empty(), "newLayer={new_layer}");
        if !new_layer {
            assert!(d.layer(layer).unwrap().surface().unwrap().rgba(120, 80)[0] > 0.9, "the stroke lands on the session's layer");
            // And it becomes active, so Fade and a colour-channel target follow it.
            assert_eq!(app.session.active().unwrap().active_layer, Some(layer));
        }
        assert_eq!(photocraft_compose::flatten(d).px, expected, "newLayer={new_layer}");
    }
}

#[test]
fn an_edit_under_the_mode_shows_on_the_canvas() {
    let (ctx, mut app) = app_with_layer();
    app.session
        .edit("stripe", |doc, active| {
            doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap().fill_rect(Rect::new(50, 70, 70, 90), &[1.0, 0.0, 0.0, 1.0]);
            Ok(())
        })
        .unwrap();
    stamp_a_stroke(&mut app, &ctx);
    let before = crate::distort_ui::display_doc(&app, 0).unwrap();
    app.session
        .edit("edit", |doc, active| {
            doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap().fill_rect(Rect::new(200, 20, 220, 40), &[0.0, 1.0, 0.0, 1.0]);
            Ok(())
        })
        .unwrap();
    // Not the stale preview, even before the mode catches up.
    assert!(crate::distort_ui::display_doc(&app, 0).is_none_or(|(d, _)| !Arc::ptr_eq(&d, &before.0)));
    // The canvas catches up before it draws: the strokes over the edit, no frame without them.
    let (shown, key) = crate::canvas::display_doc(&mut app, 0);
    assert_ne!(key, before.1);
    let st = app.session.active().unwrap();
    let layer = st.active_layer.unwrap();
    let px = shown.layer(layer).unwrap().surface().unwrap();
    assert!(px.rgba(210, 30)[1] > 0.9, "the edit shows: {:?}", px.rgba(210, 30));
    assert!(px.rgba(120, 80) != st.doc.layer(layer).unwrap().surface().unwrap().rgba(120, 80), "and so does the stroke");
    let expected = photocraft_compose::flatten(&shown).px;
    commit(&mut app).unwrap();
    assert_eq!(photocraft_compose::flatten(&app.session.active().unwrap().doc).px, expected);
}

#[test]
fn selecting_another_layer_keeps_the_stamp_preview() {
    let (ctx, mut app) = app_with_layer();
    let layer = app.session.active().unwrap().active_layer.unwrap();
    app.session.execute("layer.new.layer", json!({"name": "other"})).unwrap();
    app.session.execute("layer.select", json!({"layer": layer.0})).unwrap();
    app.sync_views();
    stamp_a_stroke(&mut app, &ctx);
    let before = crate::canvas::display_doc(&mut app, 0);
    let (doc, revision) = app.session.active().map(|st| (st.doc.clone(), st.revision)).unwrap();
    app.session.execute("layer.select", json!({"layer": "other"})).unwrap();
    let st = app.session.active().unwrap();
    assert!(st.revision != revision && Arc::ptr_eq(&st.doc, &doc), "a selection change leaves the pixels alone");
    // Same preview, same key: nothing to repaint or recomposite.
    let after = crate::canvas::display_doc(&mut app, 0);
    assert!(Arc::ptr_eq(&after.0, &before.0));
    assert_eq!(after.1, before.1);
}
