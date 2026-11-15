use super::*;

#[test]
fn zoom_menu_steps_keep_the_view_centre_and_history() {
    for animated in [false, true] {
        for offset in [vec2(-160.0, -100.0), vec2(140.0, 80.0)] {
            let mut app = zoom_app(2400, 1800);
            app.ui.tool = Tool::Zoom;
            app.run("prefs.set", json!({"values": {"general.animatedZoom": animated, "tools.zoomClickedPointToCenter": false}})).unwrap();
            let mut h = harness(app);
            let v = &mut h.state_mut().ui.views[0];
            v.zoom = 1.0;
            v.center = [928.0, 686.0];
            v.request_zoom_animation(false);
            h.step();
            let center = h.state().ui.views[0].center;
            let st = h.state().session.active().unwrap();
            let history = (st.history.past_len(), st.history.can_redo(), st.revision);
            for (label, zoom) in [("Zoom In", 2.0), ("Zoom Out", 1.0), ("Zoom Out", 2.0 / 3.0)] {
                let p = crate::canvas::ViewXform::active(h.state()).unwrap().rect.center() + offset;
                right_click(&mut h, p, Modifiers::NONE);
                h.get_by_label(label).click();
                h.run_steps(3);
                let v = &h.state().ui.views[0];
                assert_eq!(v.zoom, zoom, "{label}: one ladder stop");
                assert_eq!(v.center, center, "{label}: the right-click must not move the view centre");
                let xf = crate::canvas::ViewXform::active(h.state()).unwrap();
                assert!(xf.to_screen(center[0], center[1]).distance(xf.rect.center()) < 1.0);
                assert!(h.state().ui.canvas_tool_menu.is_none());
                let st = h.state().session.active().unwrap();
                assert_eq!((st.history.past_len(), st.history.can_redo(), st.revision), history);
            }
        }
    }
}

#[test]
fn clicked_point_zoom_keeps_its_anchor_and_hand_menu_rejects_step_commands() {
    for (tool, center_on_point) in [(Tool::Zoom, true), (Tool::Hand, true), (Tool::Hand, false)] {
        for (command, direction) in [("view.zoomIn", 1), ("view.zoomOut", -1)] {
            let mut app = zoom_app(400, 300);
            app.ui.tool = tool;
            app.run("prefs.set", json!({"values": {"tools.zoomClickedPointToCenter": center_on_point, "tools.overscroll": true}})).unwrap();
            let mut h = harness(app);
            let before = h.state().ui.views[0].clone();
            let p = screen_of(h.state(), [300.0, 100.0]);
            right_click(&mut h, p, Modifiers::NONE);
            let doc = h.state().ui.canvas_tool_menu.as_ref().unwrap().anchor.unwrap().map(|n| n as f32);
            let ctx = h.ctx.clone();
            if tool == Tool::Hand {
                // #12022 gives Hand only fixed zoom rows. An injected step command must not run.
                assert!(!available(h.state(), h.state().ui.canvas_tool_menu.as_ref().unwrap(), command));
                choose(h.state_mut(), &ctx, command);
                let v = &h.state().ui.views[0];
                assert_eq!((v.zoom, v.center), (before.zoom, before.center), "Hand must reject {command}");
                continue;
            }
            choose(h.state_mut(), &ctx, command);
            let v = &h.state().ui.views[0];
            assert_eq!(v.zoom, crate::zoom_levels::step(before.zoom, direction, [400, 300]));
            let expected = if center_on_point { doc } else { std::array::from_fn(|i| doc[i] - (doc[i] - before.center[i]) * before.zoom / v.zoom) };
            assert_eq!(v.center, expected, "{tool:?}: {command}");
            assert!(v.zoom_animating);
            assert_eq!(v.zoom_anchor.is_some(), !center_on_point);
        }
    }
}

#[test]
fn zoom_click_and_alt_click_still_anchor_at_the_pointer() {
    for animated in [false, true] {
        for (modifiers, direction) in [(Modifiers::NONE, 1), (Modifiers::ALT, -1)] {
            let mut app = zoom_app(400, 300);
            app.ui.tool = Tool::Zoom;
            app.run("prefs.set", json!({"values": {"general.animatedZoom": animated, "tools.zoomClickedPointToCenter": false, "tools.overscroll": true}}))
                .unwrap();
            let mut h = harness(app);
            let zoom = h.state().ui.views[0].zoom;
            let doc = [300.0, 100.0];
            let p = screen_of(h.state(), doc);
            h.event(egui::Event::ModifiersChanged(modifiers));
            h.event(egui::Event::PointerMoved(p));
            h.step();
            for pressed in [true, false] {
                h.event(egui::Event::PointerButton { pos: p, button: PointerButton::Primary, pressed, modifiers });
                h.step();
            }
            h.input_mut().time = Some(h.ctx.input(|i| i.time) + 0.3);
            h.step();
            assert_eq!(h.state().ui.views[0].zoom, crate::zoom_levels::step(zoom, direction, [400, 300]));
            assert!(screen_of(h.state(), doc).distance(p) < 1.0);
            assert!(h.state().ui.canvas_tool_menu.is_none());
            assert_eq!(h.state().session.active().unwrap().history.past_len(), 0);
        }
    }
}

#[test]
fn held_zoom_keys_still_open_no_view_menu() {
    for tool in [Tool::Brush, Tool::Hand] {
        for modifiers in [Modifiers::COMMAND, Modifiers::ALT] {
            let mut app = zoom_app(400, 300);
            app.ui.tool = tool;
            let mut h = harness(app);
            let modifiers =
                Modifiers { mac_cmd: cfg!(target_os = "macos") && modifiers.command, ctrl: !cfg!(target_os = "macos") && modifiers.command, ..modifiers };
            h.event(egui::Event::ModifiersChanged(modifiers));
            h.event(egui::Event::Key { key: egui::Key::Space, physical_key: None, pressed: true, repeat: false, modifiers });
            h.step();
            assert!(matches!(crate::hold_keys::held_tool(h.state(), &h.ctx), Some(crate::hold_keys::Temporary::ZoomIn | crate::hold_keys::Temporary::ZoomOut)));
            let before = h.state().ui.views[0].clone();
            let p = screen_of(h.state(), [300.0, 100.0]);
            right_click(&mut h, p, modifiers);
            assert!(h.state().ui.canvas_tool_menu.is_none());
            assert_eq!(h.state().ui.tool, tool);
            let v = &h.state().ui.views[0];
            assert_eq!((v.zoom, v.center), (before.zoom, before.center));
        }
    }
}

#[test]
fn held_zoom_with_zoom_selected_keeps_the_menu_pressed_point_anchor() {
    for animated in [false, true] {
        for (command, direction) in [("view.zoomIn", 1), ("view.zoomOut", -1)] {
            let mut app = zoom_app(400, 300);
            app.ui.tool = Tool::Zoom;
            app.run("prefs.set", json!({"values": {"general.animatedZoom": animated, "tools.zoomClickedPointToCenter": false, "tools.overscroll": true}}))
                .unwrap();
            let mut h = harness(app);
            h.event(egui::Event::ModifiersChanged(Modifiers::ALT));
            h.event(egui::Event::Key { key: egui::Key::Space, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::ALT });
            h.step();
            assert_eq!(crate::hold_keys::held_tool(h.state(), &h.ctx), Some(crate::hold_keys::Temporary::ZoomOut));
            let zoom = h.state().ui.views[0].zoom;
            let center = h.state().ui.views[0].center;
            let doc = [300.0, 100.0];
            let p = screen_of(h.state(), doc);
            h.event(egui::Event::PointerMoved(p));
            h.step();
            for pressed in [true, false] {
                h.event(egui::Event::PointerButton { pos: p, button: PointerButton::Secondary, pressed, modifiers: Modifiers::ALT });
                h.step();
            }
            assert!(h.state().ui.canvas_tool_menu.as_ref().is_some_and(|m| m.tool == Tool::Zoom));
            assert_eq!(crate::hold_keys::held_tool(h.state(), &h.ctx), Some(crate::hold_keys::Temporary::ZoomOut));
            let ctx = h.ctx.clone();
            choose(h.state_mut(), &ctx, command);
            assert_eq!(h.state().ui.views[0].zoom, crate::zoom_levels::step(zoom, direction, [400, 300]));
            assert_ne!(h.state().ui.views[0].center, center, "{command}: preserve the held tool's pressed-point behavior");
            h.step();
            h.input_mut().time = Some(ctx.input(|i| i.time) + 0.11);
            h.step();
            assert!(screen_of(h.state(), doc).distance(p) < 1.0, "{command}: anchor during animation");
            h.input_mut().time = Some(ctx.input(|i| i.time) + 0.3);
            h.step();
            assert!(screen_of(h.state(), doc).distance(p) < 1.0, "{command}: settled anchor");
            assert_eq!(h.state().ui.tool, Tool::Zoom);
            assert_eq!(h.state().session.active().unwrap().history.past_len(), 0);
        }
    }
}

#[test]
fn held_z_menu_keeps_its_existing_pressed_point_anchor() {
    for label in ["Zoom In", "Zoom Out"] {
        let mut app = zoom_app(400, 300);
        app.ui.tool = Tool::Brush;
        let mut h = Harness::builder().with_size(vec2(1000.0, 700.0)).build_ui_state(
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
        PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::default());
        h.run_steps(4);
        h.event(egui::Event::Key { key: egui::Key::Z, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE });
        h.step();
        assert_eq!(h.state().ui.tool, Tool::Zoom);
        assert!(h.state().ui.spring.as_ref().is_some_and(|s| s.previous == Tool::Brush));
        let doc = [300.0, 100.0];
        let p = screen_of(h.state(), doc);
        right_click(&mut h, p, Modifiers::NONE);
        h.get_by_label(label).click();
        h.run_steps(3);
        assert!(screen_of(h.state(), doc).distance(p) < 1.0, "{label}: a held Z keeps today's anchor");
        h.event(egui::Event::Key { key: egui::Key::Z, physical_key: None, pressed: false, repeat: false, modifiers: Modifiers::NONE });
        h.run_steps(2);
        assert_eq!(h.state().ui.tool, Tool::Brush);
    }
}
