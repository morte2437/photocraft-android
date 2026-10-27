use super::*;

#[test]
fn the_brush_outline_follows_the_pointer_except_while_resizing() {
    let (hover, anchor) = (pos2(30.0, 40.0), pos2(10.0, 20.0));
    assert_eq!(brush_circle_at(None, Some(hover)), Some(hover));
    assert_eq!(brush_circle_at(Some((anchor, 40.0, 50.0)), Some(hover)), Some(anchor));
    assert_eq!(brush_circle_at(Some((anchor, 40.0, 50.0)), None), Some(anchor));
    assert_eq!(brush_circle_at(None, None), None);
}

#[test]
fn alt_right_drag_resizes_the_liquify_brush_and_does_not_paint() {
    assert!(brush_resize_gesture(egui::Modifiers::ALT, true));
    assert!(brush_resize_gesture(egui::Modifiers::CTRL | egui::Modifiers::ALT, false));
    assert!(!brush_resize_gesture(egui::Modifiers::ALT, false), "Alt+left stays the tool's opposite");
    assert!(!brush_resize_gesture(egui::Modifiers::NONE, true));
    let (size, density) = resized_brush(40.0, 50.0, 25.0, crate::brush_resize::HARDNESS_TRAVEL);
    assert_eq!((size, density), (65.0, 100.0));
    let (size, density) = resized_brush(40.0, 50.0, -100.0, 0.0);
    assert_eq!((size, density), (1.0, 50.0));
}

fn app_with_layer() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.session.execute("file.new", json!({"width": 120, "height": 80, "depth": 8})).unwrap();
    app.session.execute("layer.new.layer", json!({})).unwrap();
    app.session
        .edit("paint", |doc, active| {
            let s = doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap();
            for y in 10..70 {
                for x in 10..110 {
                    let v = ((x / 5 + y / 5) % 2) as f32;
                    s.write_pixel(x, y, &[v, 0.3, 1.0 - v, 1.0]);
                }
            }
            Ok(())
        })
        .unwrap();
    app.sync_views();
    app
}

#[test]
fn zoomed_liquify_preview_uses_only_the_visible_full_resolution_crop() {
    let ctx = egui::Context::default();
    let mut app = app_with_layer();
    open(&mut app, &ctx).unwrap();
    let d = app.distort.liquify.as_mut().unwrap();
    d.zoom = 2.0;
    d.center = [60.0, 40.0];
    d.update_view_preview(&ctx, ERect::from_min_size(pos2(0.0, 0.0), vec2(60.0, 40.0)));
    assert_eq!(d.view_bounds, Some(Rect::new(45, 30, 75, 50)));
    assert_eq!(d.view_out.len(), 30 * 20, "one preview texel per document pixel");
    assert!(d.view_tex.is_some());

    let before = d.view_out.clone();
    let mut stroke = LiquifyStroke::new(LiquifyTool::ForwardWarp, 24.0);
    stroke.points = vec![vec![50.0, 40.0, 1.0], vec![58.0, 40.0, 1.0]];
    let dirty = d.field.stroke_segment(&stroke, [50.0, 40.0, 1.0], [58.0, 40.0, 1.0]);
    d.mark(dirty, false);
    d.render_dirty();
    d.update_view_preview(&ctx, ERect::from_min_size(pos2(0.0, 0.0), vec2(60.0, 40.0)));
    assert_ne!(d.view_out, before, "a brush update refreshes the visible source-resolution crop");
    assert!(d.preview_ms.is_finite());
}

/// #7609: Show Backdrop › Use and Mode, as in Photoshop: the settings, how each mode layers the
/// backdrop and the image, and what each Use shows.
#[test]
fn backdrop_use_and_mode() {
    let mut o = LiquifyOpts::default();
    assert_eq!((o.backdrop_layer, o.backdrop_mode), (None, BackdropMode::InFront), "All Layers, In Front");
    o.apply(&json!({"backdropUse": 3, "backdropMode": "behind"})).unwrap();
    assert_eq!((o.backdrop_layer, o.backdrop_mode), (Some(3), BackdropMode::Behind));
    assert_eq!((&o.to_json()["backdropUse"], &o.to_json()["backdropMode"]), (&json!(3), &json!("behind")));
    o.apply(&json!({"backdropUse": "all", "backdropMode": "nope"})).unwrap();
    assert_eq!((o.backdrop_layer, o.backdrop_mode), (None, BackdropMode::Behind), "a bad mode is ignored");
    let (w, a) = (Color32::WHITE, |x: f32| Color32::from_white_alpha((x * 255.0).round() as u8));
    let l = |image, behind, front| Layering { image, behind, front };
    assert_eq!(backdrop_layering(Some(1), BackdropMode::InFront, 0.25), l(w, None, Some((1, a(0.25)))), "backdrop over the image");
    assert_eq!(backdrop_layering(Some(1), BackdropMode::Behind, 0.25), l(a(0.25), Some((1, w)), None), "image over the backdrop");
    assert_eq!(backdrop_layering(Some(1), BackdropMode::Blend, 0.25), l(a(0.75), None, Some((1, a(0.25)))), "faded together");
    assert_eq!(backdrop_layering::<u8>(None, BackdropMode::Blend, 0.25), l(w, None, None), "no backdrop: the image as is");
    // All Layers is the document with the liquified layer in it; one layer is that layer alone.
    let app = app_with_layer();
    let st = app.session.active().unwrap();
    let (layer, background) = (st.active_layer.unwrap(), st.doc.layers[0].id);
    let at = |doc: &Document| photocraft_compose::render(doc, Rect::new(10, 10, 11, 11)).px[0];
    let own = at(&st.doc);
    assert_eq!(at(&backdrop_doc(&st.doc, None)), own, "All Layers");
    assert_eq!(at(&backdrop_doc(&st.doc, Some(layer.0))), own, "the liquified layer alone");
    assert_ne!(at(&backdrop_doc(&st.doc, Some(background.0))), own, "the Background alone");
    assert_eq!(at(&backdrop_doc(&st.doc, Some(9999))), own, "a layer that's gone: All Layers");
    // A hidden layer shows when it is the one used.
    let mut hidden = (*st.doc).clone();
    hidden.layer_mut(layer).unwrap().visible = false;
    assert_eq!(at(&backdrop_doc(&hidden, Some(layer.0))), own, "the hidden liquified layer, alone");
    // A remembered Use is a layer of another document: Liquify opens on All Layers.
    let ctx = egui::Context::default();
    let mut app = app_with_layer();
    app.session.prefs.edit(|p| p.dialogs.insert(REMEMBERED.into(), json!({"backdropUse": 42, "backdropMode": "blend"})));
    open(&mut app, &ctx).unwrap();
    let o = &app.distort.liquify.as_ref().unwrap().opts;
    assert_eq!((o.backdrop_layer, o.backdrop_mode), (None, BackdropMode::Blend), "the mode is kept, the layer is not");
    // Over the control channel, a layer of the document is kept and one that isn't is All Layers.
    let layer = app.session.active().unwrap().active_layer.unwrap().0;
    control(&mut app, &json!({"backdropUse": layer})).unwrap();
    assert_eq!(app.distort.liquify.as_ref().unwrap().opts.backdrop_layer, Some(layer));
    control(&mut app, &json!({"backdropUse": 4242})).unwrap();
    assert_eq!(app.distort.liquify.as_ref().unwrap().opts.backdrop_layer, None, "no such layer: All Layers");
    control(&mut app, &json!({"backdropUse": layer})).unwrap();
    assert!(control(&mut app, &json!({"backdropUse": 4242, "tool": "nope"})).is_err());
    assert_eq!(app.distort.liquify.as_ref().unwrap().opts.backdrop_layer, None, "also when another setting fails");
}

/// #418: the brush settings survive Cancel, OK and Esc, and come back the next time Liquify
/// opens (also after a restart: they live in the preferences).
#[test]
fn brush_settings_are_remembered_between_uses() {
    let ctx = egui::Context::default();
    let mut app = app_with_layer();
    open(&mut app, &ctx).unwrap();
    let fresh = app.distort.liquify.as_ref().unwrap().opts.clone();
    assert_eq!((fresh.density, fresh.tool), (50.0, LiquifyTool::ForwardWarp), "first use: the defaults");
    let set = json!({"tool": "twirlCw", "size": 37, "density": 85, "pressure": 60, "rate": 30, "showMesh": true, "meshSize": "large", "showBackdrop": true, "backdropOpacity": 25});
    control(&mut app, &set).unwrap();
    control(&mut app, &json!({"cancel": true})).unwrap();
    assert!(app.distort.liquify.is_none());
    open(&mut app, &ctx).unwrap();
    assert_eq!(app.distort.liquify.as_ref().unwrap().opts.to_json(), {
        let mut o = LiquifyOpts::default();
        o.apply(&set).unwrap();
        o.to_json()
    });
    // OK and Esc keep the latest settings too.
    control(&mut app, &json!({"density": 90})).unwrap();
    commit(&mut app).unwrap();
    open(&mut app, &ctx).unwrap();
    assert_eq!(app.distort.liquify.as_ref().unwrap().opts.density, 90.0);
    control(&mut app, &json!({"density": 70})).unwrap();
    cancel(&mut app);
    assert_eq!(app.session.prefs().dialogs[REMEMBERED]["density"], json!(70.0));
    // A damaged preference keeps what it can and never fails to open.
    app.session.prefs.edit(|p| p.dialogs.insert(REMEMBERED.into(), json!({"tool": "nope", "density": "lots", "size": 1e12, "rate": 20})));
    open(&mut app, &ctx).unwrap();
    let o = &app.distort.liquify.as_ref().unwrap().opts;
    assert_eq!((o.tool, o.density, o.size, o.rate), (LiquifyTool::ForwardWarp, 50.0, 15000.0, 20.0));
    app.session.prefs.edit(|p| p.dialogs.insert(REMEMBERED.into(), json!("not an object")));
    cancel(&mut app);
    assert!(open(&mut app, &ctx).is_ok());
}

/// Follow-up to #1494 / #1484: Alt is captured at stroke-down and stays recorded
/// through undo, redo and deterministic Liquify field reconstruction.
#[test]
fn alt_reversed_twirl_survives_replay_undo_and_redo() {
    let ctx = egui::Context::default();
    let mut app = app_with_layer();
    open(&mut app, &ctx).unwrap();
    control(&mut app, &json!({"tool": "twirlCw", "size": 36, "rate": 80})).unwrap();

    // Releasing Alt while dragging must not change the tool recorded at pointer-down.
    pointer(&mut app, ToolEvent::Down { x: 53.0, y: 40.0, pressure: 1.0 }, egui::Modifiers::ALT);
    pointer(&mut app, ToolEvent::Move { x: 58.0, y: 41.0, pressure: 1.0 }, egui::Modifiers::NONE);
    pointer(&mut app, ToolEvent::Up { x: 58.0, y: 41.0 }, egui::Modifiers::NONE);
    let d = app.distort.liquify.as_ref().unwrap();
    assert_eq!(d.opts.tool, LiquifyTool::TwirlCw, "Alt must not change the selected tool");
    assert_eq!(d.strokes.len(), 1);
    assert_eq!(d.strokes[0].tool, LiquifyTool::TwirlCcw);
    assert_eq!(LiquifyField::from_strokes(d.canvas, d.cell, &d.strokes), d.field);

    // A following stroke without Alt records the original tool.
    pointer(&mut app, ToolEvent::Down { x: 70.0, y: 40.0, pressure: 1.0 }, egui::Modifiers::NONE);
    pointer(&mut app, ToolEvent::Move { x: 75.0, y: 42.0, pressure: 1.0 }, egui::Modifiers::ALT);
    pointer(&mut app, ToolEvent::Up { x: 75.0, y: 42.0 }, egui::Modifiers::ALT);
    let d = app.distort.liquify.as_ref().unwrap();
    assert_eq!(d.strokes.len(), 2);
    assert_eq!(d.strokes[1].tool, LiquifyTool::TwirlCw);
    assert_eq!(LiquifyField::from_strokes(d.canvas, d.cell, &d.strokes), d.field);

    control(&mut app, &json!({"undo": true})).unwrap();
    let d = app.distort.liquify.as_ref().unwrap();
    assert_eq!(d.strokes.len(), 1);
    assert_eq!(d.strokes[0].tool, LiquifyTool::TwirlCcw);
    assert_eq!(d.redo[0].tool, LiquifyTool::TwirlCw);
    assert_eq!(LiquifyField::from_strokes(d.canvas, d.cell, &d.strokes), d.field);

    control(&mut app, &json!({"redo": true})).unwrap();
    let d = app.distort.liquify.as_ref().unwrap();
    assert_eq!(d.strokes.len(), 2);
    assert!(d.redo.is_empty());
    assert_eq!(d.strokes[0].tool, LiquifyTool::TwirlCcw);
    assert_eq!(d.strokes[1].tool, LiquifyTool::TwirlCw);
    assert_eq!(LiquifyField::from_strokes(d.canvas, d.cell, &d.strokes), d.field);
}

#[test]
fn dialog_strokes_match_the_engine_and_commit_once() {
    let ctx = egui::Context::default();
    let mut app = app_with_layer();
    open(&mut app, &ctx).unwrap();
    control(&mut app, &json!({"tool": "forwardWarp", "size": 40})).unwrap();
    for ev in [
        ToolEvent::Down { x: 40.0, y: 40.0, pressure: 1.0 },
        ToolEvent::Move { x: 48.0, y: 41.0, pressure: 1.0 },
        ToolEvent::Move { x: 56.0, y: 40.0, pressure: 1.0 },
        ToolEvent::Up { x: 56.0, y: 40.0 },
    ] {
        pointer(&mut app, ev, egui::Modifiers::NONE);
    }
    control(&mut app, &json!({"tool": "bloat"})).unwrap();
    for ev in [ToolEvent::Down { x: 80.0, y: 40.0, pressure: 1.0 }, ToolEvent::Move { x: 80.0, y: 40.0, pressure: 1.0 }, ToolEvent::Up { x: 80.0, y: 40.0 }] {
        pointer(&mut app, ev, egui::Modifiers::NONE);
    }
    let d = app.distort.liquify.as_ref().unwrap();
    assert_eq!(d.strokes.len(), 2);
    assert_eq!(d.redo.len(), 0);
    // Replaying the recorded strokes gives exactly the interactive field.
    let replay = LiquifyField::from_strokes(d.canvas, d.cell, &d.strokes);
    assert_eq!(replay, d.field);
    assert!(d.field.max_displacement() > 3.0);
    // Undo inside the dialog drops the last stroke.
    control(&mut app, &json!({"undo": true})).unwrap();
    assert_eq!(app.distort.liquify.as_ref().unwrap().strokes.len(), 1);
    assert_eq!(app.distort.liquify.as_ref().unwrap().redo.len(), 1);
    control(&mut app, &json!({"redo": true})).unwrap();
    assert_eq!(app.distort.liquify.as_ref().unwrap().strokes.len(), 2);
    assert_eq!(app.distort.liquify.as_ref().unwrap().redo.len(), 0);
    control(&mut app, &json!({"undo": true})).unwrap();
    let before = app.session.active().unwrap().history.past_len();
    control(&mut app, &json!({"commit": true})).unwrap();
    assert!(app.distort.liquify.is_none());
    assert_eq!(app.session.active().unwrap().history.past_len(), before + 1);
}

#[test]
fn undo_rerenders_the_proxy_before_the_next_texture_upload() {
    let ctx = egui::Context::default();
    let mut app = app_with_layer();
    open(&mut app, &ctx).unwrap();
    control(&mut app, &json!({"tool": "forwardWarp", "size": 40})).unwrap();
    for ev in [
        ToolEvent::Down { x: 35.0, y: 40.0, pressure: 1.0 },
        ToolEvent::Move { x: 55.0, y: 40.0, pressure: 1.0 },
        ToolEvent::Up { x: 55.0, y: 40.0 },
        ToolEvent::Down { x: 70.0, y: 40.0, pressure: 1.0 },
        ToolEvent::Move { x: 90.0, y: 40.0, pressure: 1.0 },
        ToolEvent::Up { x: 90.0, y: 40.0 },
    ] {
        pointer(&mut app, ev, egui::Modifiers::NONE);
    }

    let before = app.distort.liquify.as_ref().unwrap().out.clone();
    control(&mut app, &json!({"undo": true})).unwrap();
    let d = app.distort.liquify.as_ref().unwrap();
    assert_eq!(d.strokes.len(), 1);
    assert_ne!(d.out, before, "undo must update the pixels shown by the existing texture");

    let mut expected = vec![[0u8; 4]; d.proxy.w * d.proxy.h];
    d.proxy.render(&d.field, [0, 0, d.proxy.w, d.proxy.h], &mut expected);
    assert_eq!(d.out, expected, "dirty upload source must match the rebuilt field");
}

#[test]
fn undo_dirties_the_zoomed_preview() {
    let ctx = egui::Context::default();
    let mut app = app_with_layer();
    open(&mut app, &ctx).unwrap();
    control(&mut app, &json!({"tool": "forwardWarp", "size": 40})).unwrap();
    for ev in [ToolEvent::Down { x: 35.0, y: 40.0, pressure: 1.0 }, ToolEvent::Move { x: 55.0, y: 40.0, pressure: 1.0 }, ToolEvent::Up { x: 55.0, y: 40.0 }] {
        pointer(&mut app, ev, egui::Modifiers::NONE);
    }
    {
        let d = app.distort.liquify.as_mut().unwrap();
        d.zoom = 2.0;
        d.view_bounds = Some(Rect::new(0, 0, 20, 20));
        d.view_dirty = None;
    }
    control(&mut app, &json!({"undo": true})).unwrap();
    let d = app.distort.liquify.as_ref().unwrap();
    assert_eq!(d.view_dirty, d.view_bounds);
}

#[test]
fn liquify_redo_restores_undone_stroke_and_a_new_stroke_clears_redo() {
    let ctx = egui::Context::default();
    let mut app = app_with_layer();
    open(&mut app, &ctx).unwrap();
    control(&mut app, &json!({"tool": "forwardWarp", "size": 40})).unwrap();
    let draw = |app: &mut PhotocraftApp, x: f64| {
        for ev in
            [ToolEvent::Down { x, y: 40.0, pressure: 1.0 }, ToolEvent::Move { x: x + 14.0, y: 40.0, pressure: 1.0 }, ToolEvent::Up { x: x + 14.0, y: 40.0 }]
        {
            pointer(app, ev, egui::Modifiers::NONE);
        }
    };

    draw(&mut app, 30.0);
    draw(&mut app, 70.0);
    control(&mut app, &json!({"undo": true})).unwrap();
    let d = app.distort.liquify.as_ref().unwrap();
    assert_eq!((d.strokes.len(), d.redo.len()), (1, 1));

    control(&mut app, &json!({"redo": true})).unwrap();
    let d = app.distort.liquify.as_ref().unwrap();
    assert_eq!((d.strokes.len(), d.redo.len()), (2, 0));
    assert_eq!(LiquifyField::from_strokes(d.canvas, d.cell, &d.strokes), d.field);

    control(&mut app, &json!({"undo": true})).unwrap();
    draw(&mut app, 50.0);
    let d = app.distort.liquify.as_ref().unwrap();
    assert_eq!((d.strokes.len(), d.redo.len()), (2, 0));
    control(&mut app, &json!({"redo": true})).unwrap();
    assert_eq!(app.distort.liquify.as_ref().unwrap().strokes.len(), 2, "redo after a new stroke is a no-op");
}

/// The Freeze Lasso freezes the dragged polygon (⌥ thaws it) as one recorded stroke, and a
/// new lasso clears Redo like any other stroke.
#[test]
fn freeze_lasso_records_a_stroke_and_clears_redo() {
    let ctx = egui::Context::default();
    let mut app = app_with_layer();
    open(&mut app, &ctx).unwrap();
    control(&mut app, &json!({"tool": "lassoMask"})).unwrap();
    let lasso = |app: &mut PhotocraftApp, m: egui::Modifiers, r: [f64; 4]| {
        pointer(app, ToolEvent::Down { x: r[0], y: r[1], pressure: 1.0 }, m);
        for (x, y) in [(r[2], r[1]), (r[2], r[3]), (r[0], r[3])] {
            pointer(app, ToolEvent::Move { x, y, pressure: 1.0 }, m);
        }
        pointer(app, ToolEvent::Up { x: r[0], y: r[3] }, m);
    };
    lasso(&mut app, egui::Modifiers::NONE, [20.0, 20.0, 80.0, 60.0]);
    let d = app.distort.liquify.as_ref().unwrap();
    assert_eq!(d.strokes.len(), 1);
    assert_eq!(d.field.freeze_at(50.0, 40.0), 1.0, "frozen inside");
    assert_eq!(d.field.freeze_at(5.0, 5.0), 0.0, "not outside");
    lasso(&mut app, egui::Modifiers::ALT, [40.0, 30.0, 60.0, 50.0]);
    let d = app.distort.liquify.as_ref().unwrap();
    assert_eq!(d.field.freeze_at(50.0, 40.0), 0.0, "⌥ thaws");
    assert_eq!(d.field.freeze_at(25.0, 25.0), 1.0);
    control(&mut app, &json!({"undo": true})).unwrap();
    assert_eq!(app.distort.liquify.as_ref().unwrap().field.freeze_at(50.0, 40.0), 1.0, "undo replays the first lasso");
    assert_eq!(app.distort.liquify.as_ref().unwrap().redo.len(), 1);
    lasso(&mut app, egui::Modifiers::NONE, [0.0, 0.0, 10.0, 10.0]);
    assert_eq!(app.distort.liquify.as_ref().unwrap().redo.len(), 0, "a new lasso clears redo");
}

#[test]
fn alt_reverses_twirl_and_swaps_pucker_and_bloat() {
    let ctx = egui::Context::default();
    let mut app = app_with_layer();
    open(&mut app, &ctx).unwrap();
    let dab = |app: &mut PhotocraftApp, m: egui::Modifiers| {
        pointer(app, ToolEvent::Down { x: 50.0, y: 40.0, pressure: 1.0 }, m);
        pointer(app, ToolEvent::Up { x: 50.0, y: 40.0 }, m);
        app.distort.liquify.as_ref().unwrap().strokes.last().unwrap().tool
    };
    for (tool, plain, alt) in [
        ("twirlCw", LiquifyTool::TwirlCw, LiquifyTool::TwirlCcw),
        ("twirlCcw", LiquifyTool::TwirlCcw, LiquifyTool::TwirlCw),
        ("pucker", LiquifyTool::Pucker, LiquifyTool::Bloat),
        ("bloat", LiquifyTool::Bloat, LiquifyTool::Pucker),
        ("forwardWarp", LiquifyTool::ForwardWarp, LiquifyTool::ForwardWarp),
    ] {
        control(&mut app, &json!({"tool": tool})).unwrap();
        assert_eq!(dab(&mut app, egui::Modifiers::NONE), plain, "{tool}");
        assert_eq!(dab(&mut app, egui::Modifiers::ALT), alt, "⌥ {tool}");
        assert_eq!(app.distort.liquify.as_ref().unwrap().opts.tool, plain, "⌥ leaves the selected tool alone");
    }
}

#[test]
fn liquify_redo_stack_is_bounded() {
    let ctx = egui::Context::default();
    let mut app = app_with_layer();
    open(&mut app, &ctx).unwrap();
    let d = app.distort.liquify.as_mut().unwrap();
    d.strokes = (0..=REDO_STACK_LIMIT).map(|_| LiquifyStroke::new(LiquifyTool::ForwardWarp, 1.0)).collect();
    for _ in 0..=REDO_STACK_LIMIT {
        d.undo();
    }
    assert_eq!(d.redo.len(), REDO_STACK_LIMIT);
}

/// #7884: Alt+right-dragging the brush size in the Liquify dialog keeps the brush where the
/// drag started (its outline and the point it would work at) and only changes its size, as the
/// brush tools' Alt-drag does outside Liquify.
#[test]
fn alt_drag_resize_keeps_the_brush_where_it_started() {
    let ctx = egui::Context::default();
    PhotocraftApp::setup_context(&ctx, Default::default());
    let mut app = app_with_layer();
    open(&mut app, &ctx).unwrap();
    let screen = ERect::from_min_size(pos2(0.0, 0.0), vec2(1400.0, 900.0));
    let frame = |app: &mut PhotocraftApp, events: Vec<egui::Event>, mods: egui::Modifiers| {
        let events = std::iter::once(egui::Event::ModifiersChanged(mods)).chain(events).collect();
        let raw = egui::RawInput { screen_rect: Some(screen), events, ..Default::default() };
        ctx.run_ui(raw, |ui| show(app, ui.ctx())).textures_delta.clear();
    };
    let alt = egui::Modifiers::ALT;
    let none = egui::Modifiers::NONE;
    frame(&mut app, vec![], none);
    // A point over the image in the dialog's canvas (120 × 80): the first one on a grid.
    let p = (1..22)
        .flat_map(|j| (1..34).map(move |i| pos2(40.0 * i as f32, 40.0 * j as f32)))
        .find(|&q| {
            frame(&mut app, vec![egui::Event::PointerMoved(q)], none);
            let h = app.distort.liquify.as_ref().and_then(|d| d.hover);
            h.is_some_and(|h| (10.0..60.0).contains(&h[0]) && (20.0..60.0).contains(&h[1]))
        })
        .expect("the dialog shows the image");
    frame(&mut app, vec![egui::Event::PointerMoved(p)], none);
    let d = app.distort.liquify.as_ref().unwrap();
    let (start, size0) = (d.hover.expect("the pointer is over the canvas"), d.opts.size);
    let button = |pressed| egui::Event::PointerButton { pos: p, button: egui::PointerButton::Secondary, pressed, modifiers: alt };
    frame(&mut app, vec![button(true)], alt);
    for k in 1..=6 {
        frame(&mut app, vec![egui::Event::PointerMoved(p + vec2(10.0 * k as f32, 0.0))], alt);
    }
    let d = app.distort.liquify.as_ref().unwrap();
    assert!(d.opts.size > size0, "the drag resized the brush: {} -> {}", size0, d.opts.size);
    assert_eq!(d.hover, Some(start), "the brush stayed where the drag started");
    assert!(d.strokes.is_empty(), "and nothing was painted");
}

#[test]
fn shortcut_handler_undoes_liquify_even_when_egui_owns_keyboard_focus() {
    let ctx = egui::Context::default();
    let mut app = app_with_layer();
    open(&mut app, &ctx).unwrap();
    for ev in [ToolEvent::Down { x: 40.0, y: 40.0, pressure: 1.0 }, ToolEvent::Move { x: 52.0, y: 40.0, pressure: 1.0 }, ToolEvent::Up { x: 52.0, y: 40.0 }] {
        pointer(&mut app, ev, egui::Modifiers::NONE);
    }
    assert_eq!(app.distort.liquify.as_ref().unwrap().strokes.len(), 1);

    let raw = egui::RawInput {
        events: vec![egui::Event::Key { key: egui::Key::Z, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::COMMAND }],
        ..Default::default()
    };
    let mut out = ctx.run_ui(raw, |ui| {
        let ctx = ui.ctx();
        ctx.memory_mut(|m| m.request_focus(egui::Id::unique("liquify-slider-focus")));
        assert!(ctx.egui_wants_keyboard_input());
        crate::shortcuts::handle(&mut app, ctx);
    });
    out.textures_delta.clear();
    assert_eq!(app.distort.liquify.as_ref().unwrap().strokes.len(), 0);
    let raw = egui::RawInput {
        events: vec![egui::Event::Key {
            key: egui::Key::Z,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
        }],
        ..Default::default()
    };
    let _ = ctx.run_ui(raw, |ui| {
        crate::shortcuts::handle(&mut app, ui.ctx());
    });
    assert_eq!(app.distort.liquify.as_ref().unwrap().strokes.len(), 1, "Cmd+Shift+Z redoes the last stroke");
}

// Regression test for #4134: Cmd+Z and Cmd+Shift+Z were ignored in Liquify while a numeric
// field's text edit (Size, Pressure, ...) had keyboard focus, so a stroke could not be undone.
#[test]
fn liquify_undo_and_redo_work_while_a_numeric_field_is_focused() {
    let ctx = egui::Context::default();
    let mut app = app_with_layer();
    open(&mut app, &ctx).unwrap();
    for ev in [ToolEvent::Down { x: 40.0, y: 40.0, pressure: 1.0 }, ToolEvent::Move { x: 52.0, y: 40.0, pressure: 1.0 }, ToolEvent::Up { x: 52.0, y: 40.0 }] {
        pointer(&mut app, ev, egui::Modifiers::NONE);
    }
    let strokes = |app: &PhotocraftApp| app.distort.liquify.as_ref().unwrap().strokes.len();
    assert_eq!(strokes(&app), 1);

    // One frame of the real shortcut handler, then a focused single-line text edit (the
    // numeric fields are text edits), as the app orders them.
    let mut text = String::from("100");
    let field = egui::Id::unique("liquify-size-field");
    let mut frame = |app: &mut PhotocraftApp, events: Vec<egui::Event>| {
        let raw = egui::RawInput { events, ..Default::default() };
        let mut out = ctx.run_ui(raw, |ui| {
            crate::shortcuts::handle(app, ui.ctx());
            let edit = ui.add(egui::TextEdit::singleline(&mut text).id(field));
            edit.request_focus();
        });
        out.textures_delta.clear();
    };
    frame(&mut app, Vec::new());
    assert!(ctx.text_edit_focused(), "the numeric field is a focused text edit");

    let z = |modifiers| egui::Event::Key { key: egui::Key::Z, physical_key: None, pressed: true, repeat: false, modifiers };
    frame(&mut app, vec![z(egui::Modifiers::COMMAND)]);
    assert_eq!(strokes(&app), 0, "Cmd+Z undoes the stroke with the field focused");
    frame(&mut app, vec![z(egui::Modifiers::COMMAND | egui::Modifiers::SHIFT)]);
    assert_eq!(strokes(&app), 1, "Cmd+Shift+Z redoes it");
}

// Regression test for #4134: other Liquify keys must not steal characters from a focused
// numeric field, and the bracket keys step the brush by at least one pixel.
#[test]
fn liquify_tool_keys_leave_a_focused_field_alone_and_brackets_reach_one_pixel() {
    let ctx = egui::Context::default();
    let mut app = app_with_layer();
    open(&mut app, &ctx).unwrap();
    app.distort.liquify.as_mut().unwrap().opts.size = 2.0;
    let size = |app: &PhotocraftApp| app.distort.liquify.as_ref().unwrap().opts.size;
    let press = |app: &mut PhotocraftApp, key: egui::Key| {
        let ev = egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE };
        let mut out = ctx.run_ui(egui::RawInput { events: vec![ev], ..Default::default() }, |ui| crate::shortcuts::handle(app, ui.ctx()));
        out.textures_delta.clear();
    };
    // 2 px * 0.9 used to round back to 2, so `[` never reached 1 px; `]` steps up from 1.
    press(&mut app, egui::Key::OpenBracket);
    assert_eq!(size(&app), 1.0, "`[` steps a 2 px brush down to 1 px");
    press(&mut app, egui::Key::CloseBracket);
    assert_eq!(size(&app), 2.0, "`]` steps it back up");

    // With a text edit focused, `[` belongs to the field.
    let mut text = String::new();
    let field = egui::Id::unique("liquify-size-field");
    let focused_frame = |app: &mut PhotocraftApp, text: &mut String, events: Vec<egui::Event>| {
        let mut out = ctx.run_ui(egui::RawInput { events, ..Default::default() }, |ui| {
            crate::shortcuts::handle(app, ui.ctx());
            ui.add(egui::TextEdit::singleline(text).id(field)).request_focus();
        });
        out.textures_delta.clear();
    };
    focused_frame(&mut app, &mut text, Vec::new());
    assert!(ctx.text_edit_focused());
    let bracket = egui::Event::Key { key: egui::Key::OpenBracket, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE };
    focused_frame(&mut app, &mut text, vec![bracket]);
    assert_eq!(size(&app), 2.0, "the focused field keeps its keys");
}

fn key(key: egui::Key) -> egui::Event {
    egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::COMMAND }
}

/// A frame of the open dialog in a 1000 × 700 window with `events`; returns the preview's
/// centre (between the tool strip and the options panel, under the title strip).
fn frame(app: &mut PhotocraftApp, ctx: &egui::Context, events: Vec<egui::Event>) -> Pos2 {
    let screen = ERect::from_min_size(Pos2::ZERO, vec2(1000.0, 700.0));
    let raw = egui::RawInput { screen_rect: Some(screen), events, ..Default::default() };
    let mut out = ctx.run_ui(raw, |ui| show(app, ui.ctx()));
    out.textures_delta.clear();
    pos2((LEFT_W + screen.width() - RIGHT_W) / 2.0, (30.0 + screen.height()) / 2.0)
}

/// #7210: ⌘+ / ⌘− step Liquify's preview along the zoom ladder, ⌘0 fits it and ⌘1 is the
/// dialog's 100 %, through the canvas's own shortcuts; the canvas behind keeps its zoom.
#[test]
fn zoom_shortcuts_zoom_the_preview_not_the_canvas() {
    let ctx = egui::Context::default();
    PhotocraftApp::setup_context(&ctx, crate::theme::ThemeKind::ALL[0]);
    let mut app = app_with_layer();
    open(&mut app, &ctx).unwrap();
    frame(&mut app, &ctx, Vec::new());
    let fitted = app.distort.liquify.as_ref().unwrap().zoom;
    assert!(fitted > 0.0, "the preview fits on its first frame");
    let canvas_zoom = app.ui.views.first().map(|v| v.zoom);
    let mut press = |k: egui::Key| {
        let raw = egui::RawInput { events: vec![key(k)], ..Default::default() };
        let _ = ctx.run_ui(raw, |ui| crate::shortcuts::handle(&mut app, ui.ctx()));
        app.distort.liquify.as_ref().unwrap().zoom
    };
    assert_eq!(press(egui::Key::Num1), 1.0);
    assert_eq!(press(egui::Key::Equals), 2.0, "next step up the ladder");
    assert_eq!(press(egui::Key::Plus), 3.0, "⌘+ typed as Plus too");
    assert_eq!(press(egui::Key::Minus), 2.0);
    assert_eq!(press(egui::Key::Num0), 0.0, "fit on the next frame");
    frame(&mut app, &ctx, Vec::new());
    assert_eq!(app.distort.liquify.as_ref().unwrap().zoom, fitted);
    assert_eq!(app.ui.views.first().map(|v| v.zoom), canvas_zoom, "the canvas behind keeps its zoom");
}

/// #7210: a trackpad pinch over the preview zooms it about the pointer (macOS sends no wheel
/// for it), unless Preferences › Enhanced Controls › Zoom with Trackpad Pinch is off; a wheel
/// notch is one ×1.1 step.
#[test]
fn pinch_and_wheel_zoom_the_preview_about_the_pointer() {
    let ctx = egui::Context::default();
    PhotocraftApp::setup_context(&ctx, crate::theme::ThemeKind::ALL[0]);
    let mut app = app_with_layer();
    open(&mut app, &ctx).unwrap();
    let center = frame(&mut app, &ctx, Vec::new());
    let at = center + vec2(60.0, -40.0);
    frame(&mut app, &ctx, vec![egui::Event::PointerMoved(at)]);
    let screen = ERect::from_min_size(Pos2::ZERO, vec2(1000.0, 700.0));
    let area = ERect::from_min_max(pos2(LEFT_W, 30.0), pos2(screen.width() - RIGHT_W, screen.height()));
    let (z0, under) = {
        let d = app.distort.liquify.as_ref().unwrap();
        (d.zoom, to_doc(d, area, at))
    };
    frame(&mut app, &ctx, vec![egui::Event::PointerMoved(at), egui::Event::Zoom(2.0)]);
    let d = app.distort.liquify.as_ref().unwrap();
    assert!((d.zoom - z0 * 2.0).abs() < 1e-4, "{} → {}", z0, d.zoom);
    let still = to_doc(d, area, at);
    assert!((still[0] - under[0]).abs() < 1e-3 && (still[1] - under[1]).abs() < 1e-3, "the point under the pointer stays: {under:?} → {still:?}");

    // Animated Zoom off: a notch is one instant step, so one frame shows the whole ×1.1.
    app.session.prefs.edit(|p| p.general.animated_zoom = false);
    let notch = ctx.options(|o| o.input_options.line_scroll_speed);
    let wheel = egui::Event::MouseWheel {
        source: egui::MouseWheelSource::Unknown,
        unit: egui::MouseWheelUnit::Line,
        delta: vec2(0.0, 1.0),
        modifiers: egui::Modifiers::NONE,
        phase: egui::TouchPhase::Move,
    };
    let z1 = app.distort.liquify.as_ref().unwrap().zoom;
    frame(&mut app, &ctx, vec![egui::Event::PointerMoved(at), wheel]);
    let z2 = app.distort.liquify.as_ref().unwrap().zoom;
    assert!(notch > 0.0 && (z2 / z1 - crate::wheel_nav::NOTCH).abs() < 1e-3, "one notch is ×1.1: {z1} → {z2}");

    app.session.prefs.edit(|p| p.enhanced_controls.zoom_with_trackpad_pinch = false);
    frame(&mut app, &ctx, vec![egui::Event::PointerMoved(at), egui::Event::Zoom(2.0)]);
    assert_eq!(app.distort.liquify.as_ref().unwrap().zoom, z2, "a pinch does nothing with the preference off");
}

/// The proxy rendered from the recorded strokes, i.e. what undo/replay rebuilds.
fn replayed_out(d: &LiquifyDialog) -> Vec<[u8; 4]> {
    let field = LiquifyField::from_strokes(d.canvas, d.cell, &d.strokes);
    let mut out = vec![[0u8; 4]; d.proxy.w * d.proxy.h];
    d.proxy.render(&field, [0, 0, d.proxy.w, d.proxy.h], &mut out);
    out
}

/// #8055: a second pointer-down while a stroke is open must not drop the dabs already applied.
#[test]
fn second_down_without_up_keeps_the_open_strokes_dabs_recorded() {
    let ctx = egui::Context::default();
    let mut app = app_with_layer();
    open(&mut app, &ctx).unwrap();
    control(&mut app, &json!({"tool": "forwardWarp", "size": 30})).unwrap();
    for ev in [
        ToolEvent::Down { x: 40.0, y: 40.0, pressure: 1.0 },
        ToolEvent::Move { x: 46.0, y: 41.0, pressure: 1.0 },
        ToolEvent::Down { x: 46.0, y: 41.0, pressure: 1.0 },
        ToolEvent::Move { x: 55.0, y: 42.0, pressure: 1.0 },
        ToolEvent::Up { x: 55.0, y: 42.0 },
    ] {
        pointer(&mut app, ev, egui::Modifiers::NONE);
    }
    let d = app.distort.liquify.as_ref().unwrap();
    assert!(d.out == replayed_out(d), "the live field must match the field rebuilt from the recorded strokes");
}

/// #8055: short strokes drawn with the mouse (moves inside egui's drag threshold first) are
/// recorded whole, as one stroke each, so undo removes only the last one.
#[test]
fn short_mouse_strokes_are_recorded_whole_and_undo_removes_only_the_last() {
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let mut app = app_with_layer();
    open(&mut app, &ctx).unwrap();
    control(&mut app, &json!({"tool": "forwardWarp", "size": 30})).unwrap();
    let screen = ERect::from_min_size(pos2(0.0, 0.0), vec2(1280.0, 800.0));
    let frame = |app: &mut PhotocraftApp, events: Vec<egui::Event>| {
        let raw = egui::RawInput { screen_rect: Some(screen), events, ..Default::default() };
        let mut out = ctx.run_ui(raw, |ui| show(app, ui.ctx()));
        out.textures_delta.clear();
    };
    frame(&mut app, vec![]);
    frame(&mut app, vec![]);
    let mut before_last = Vec::new();
    let strokes = 6;
    for i in 0..strokes {
        let start = pos2(560.0 + i as f32 * 25.0, 380.0 + (i % 2) as f32 * 20.0);
        before_last = app.distort.liquify.as_ref().unwrap().out.clone();
        frame(&mut app, vec![egui::Event::PointerMoved(start)]);
        let press = egui::Event::PointerButton { pos: start, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE };
        frame(&mut app, vec![press]);
        let mut p = start;
        for _ in 0..8 {
            p += vec2(1.5, 1.0);
            frame(&mut app, vec![egui::Event::PointerMoved(p)]);
        }
        let release = egui::Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE };
        frame(&mut app, vec![release]);
        frame(&mut app, vec![]);
        let d = app.distort.liquify.as_ref().unwrap();
        assert_eq!(d.strokes.len(), i + 1, "one recorded stroke per mouse stroke");
        assert!(d.out == replayed_out(d), "stroke {i}: the live field must match the recorded strokes");
    }
    control(&mut app, &json!({"undo": true})).unwrap();
    let d = app.distort.liquify.as_ref().unwrap();
    assert_eq!(d.strokes.len(), strokes - 1);
    let differ = d.out.iter().zip(&before_last).filter(|(a, b)| a != b).count();
    assert_eq!(differ, 0, "undo restores the pixels from before the last stroke");
}

#[test]
fn menu_opens_the_dialog_and_routes_pointer_events() {
    let ctx = egui::Context::default();
    let mut app = app_with_layer();
    let r = crate::distort_ui::menu(&mut app, &ctx, "filter.liquify", &json!({})).unwrap().unwrap();
    assert!(r["liquify"]["proxy"].is_array());
    assert!(crate::distort_ui::pointer(&mut app, ToolEvent::Down { x: 50.0, y: 40.0, pressure: 1.0 }, egui::Modifiers::NONE));
    crate::distort_ui::pointer(&mut app, ToolEvent::Up { x: 60.0, y: 40.0 }, egui::Modifiers::NONE);
    assert_eq!(app.distort.liquify.as_ref().unwrap().strokes.len(), 1);
    control(&mut app, &json!({"cancel": true})).unwrap();
    assert!(app.distort.liquify.is_none());
    assert_eq!(app.session.active().unwrap().history.past_len(), 2, "cancel records nothing");
}

/// A smart object with a Liquify smart filter (one stroke, mesh size 3) and a Gaussian Blur
/// above it.
fn smart_liquify_app() -> (PhotocraftApp, LayerId) {
    let mut app = app_with_layer();
    app.run("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
    let stroke = json!({"tool": "forwardWarp", "size": 40, "points": [[40.0, 40.0, 1.0], [48.0, 41.0, 1.0], [56.0, 40.0, 1.0]]});
    app.run("filter.liquify", json!({"strokes": [stroke], "meshSize": 3.0})).unwrap();
    app.run("filter.blur.gaussianBlur", json!({"radius": 2.0})).unwrap();
    let layer = app.session.active().unwrap().active_layer.unwrap();
    (app, layer)
}

fn smart(app: &PhotocraftApp, layer: LayerId) -> photocraft_doc::SmartObject {
    match &app.session.active().unwrap().doc.layer(layer).unwrap().content {
        photocraft_doc::LayerContent::Smart(sm) => sm.clone(),
        _ => panic!("not a smart object"),
    }
}

fn drag(app: &mut PhotocraftApp, from: f64, to: f64) {
    for ev in [ToolEvent::Down { x: from, y: 40.0, pressure: 1.0 }, ToolEvent::Move { x: to, y: 40.0, pressure: 1.0 }, ToolEvent::Up { x: to, y: 40.0 }] {
        pointer(app, ev, egui::Modifiers::NONE);
    }
}

#[test]
fn smart_filter_double_click_edits_the_recorded_liquify() {
    let ctx = egui::Context::default();
    let (mut app, layer) = smart_liquify_app();
    let steps = app.session.active().unwrap().history.past_len();
    let sm = smart(&app, layer);
    let recorded: Vec<LiquifyStroke> = serde_json::from_value(sm.smart_filters[0].params["strokes"].clone()).unwrap();
    assert_eq!(recorded.len(), 1);

    crate::smart_ui::open_editor(&mut app, &ctx, layer.0, &sm, 0);
    assert!(app.ui.dialogs.is_empty(), "no generic parameter form");
    let d = app.distort.liquify.as_ref().expect("the Liquify workspace opens");
    assert_eq!((d.smart_filter, d.cell), (Some(0), 3.0));
    assert_eq!(d.strokes, recorded);
    assert_eq!(d.field, LiquifyField::from_strokes(d.canvas, 3.0, &recorded));
    // The preview starts from the pixels below the filter (no liquify, no blur).
    let below = photocraft_engine::smart_cmds::render_below_filter(&app.session.active().unwrap().doc, &sm, 0).unwrap().unwrap();
    assert_eq!(d.source.read_region(d.canvas), below.read_region(d.canvas));

    drag(&mut app, 70.0, 84.0);
    control(&mut app, &json!({"commit": true})).unwrap();
    assert!(app.distort.liquify.is_none());
    let sm = smart(&app, layer);
    assert_eq!(sm.smart_filters.len(), 2, "edited in place, not added");
    assert_eq!(sm.smart_filters[0].command, photocraft_engine::distort_cmds::LIQUIFY);
    assert_eq!(sm.smart_filters[0].params["strokes"].as_array().map(Vec::len), Some(2));
    assert_eq!(sm.smart_filters[0].params["meshSize"], json!(3.0));
    assert_eq!(sm.smart_filters[1].command, "filter.blur.gaussianBlur");
    assert_eq!(app.session.active().unwrap().history.past_len(), steps + 1);
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(smart(&app, layer).smart_filters[0].params["strokes"].as_array().map(Vec::len), Some(1));
}

#[test]
fn smart_filter_cancel_keeps_and_restore_all_clears_the_recorded_strokes() {
    let ctx = egui::Context::default();
    let (mut app, layer) = smart_liquify_app();
    let steps = app.session.active().unwrap().history.past_len();
    let params = smart(&app, layer).smart_filters[0].params.clone();

    open_smart_filter(&mut app, &ctx, layer, 0).unwrap();
    drag(&mut app, 70.0, 84.0);
    control(&mut app, &json!({"cancel": true})).unwrap();
    assert_eq!(smart(&app, layer).smart_filters[0].params, params, "Cancel leaves the filter as it was");
    assert_eq!(app.session.active().unwrap().history.past_len(), steps);

    // OK after Restore All keeps the filter, with no strokes.
    open_smart_filter(&mut app, &ctx, layer, 0).unwrap();
    control(&mut app, &json!({"restoreAll": true})).unwrap();
    control(&mut app, &json!({"commit": true})).unwrap();
    let sm = smart(&app, layer);
    assert_eq!(sm.smart_filters.len(), 2);
    assert_eq!(sm.smart_filters[0].params["strokes"], json!([]));
    assert_eq!(app.session.active().unwrap().history.past_len(), steps + 1);
}

#[test]
fn open_smart_filter_rejects_other_filters_and_layers() {
    let ctx = egui::Context::default();
    let (mut app, layer) = smart_liquify_app();
    assert!(open_smart_filter(&mut app, &ctx, layer, 1).is_err(), "the Gaussian Blur");
    assert!(open_smart_filter(&mut app, &ctx, layer, 9).is_err(), "no such filter");
    let mut plain = app_with_layer();
    let pixels = plain.session.active().unwrap().active_layer.unwrap();
    assert!(open_smart_filter(&mut plain, &ctx, pixels, 0).is_err(), "not a smart object");
    assert!(app.distort.liquify.is_none() && plain.distort.liquify.is_none());
}

/// OK on a smart filter that is gone: the workspace closes, the status bar shows the error and
/// the control channel returns it.
#[test]
fn smart_filter_edit_that_cannot_be_applied_reports_the_error() {
    let ctx = egui::Context::default();
    let (mut app, layer) = smart_liquify_app();
    open_smart_filter(&mut app, &ctx, layer, 0).unwrap();
    drag(&mut app, 70.0, 84.0);
    app.session.execute("layer.smartFilter.clearSmartFilters", json!({"layer": layer.0})).unwrap();
    let steps = app.session.active().unwrap().history.past_len();
    let r = control(&mut app, &json!({"commit": true}));
    assert!(app.distort.liquify.is_none());
    assert!(app.ui.status_error);
    assert!(app.ui.status.contains("no smart filters"), "{}", app.ui.status);
    assert!(smart(&app, layer).smart_filters.is_empty());
    assert_eq!(app.session.active().unwrap().history.past_len(), steps);
    assert_eq!(r, Err(app.ui.status.clone()));
}

/// OK after the layer was deleted: the dialog closes, the status bar shows the error and the
/// control channel returns it.
#[test]
fn liquify_that_cannot_be_applied_reports_the_error() {
    let ctx = egui::Context::default();
    let mut app = app_with_layer();
    let layer = app.session.active().unwrap().active_layer.unwrap();
    open(&mut app, &ctx).unwrap();
    drag(&mut app, 40.0, 56.0);
    assert_eq!(app.distort.liquify.as_ref().unwrap().strokes.len(), 1);
    app.session.execute("layer.delete", json!({"layer": layer.0})).unwrap();
    let steps = app.session.active().unwrap().history.past_len();
    let r = control(&mut app, &json!({"commit": true}));
    assert!(app.distort.liquify.is_none());
    assert!(app.ui.status_error);
    assert!(!app.ui.status.is_empty());
    assert_eq!(app.session.active().unwrap().history.past_len(), steps);
    assert_eq!(r, Err(app.ui.status.clone()));
}
