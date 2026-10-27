use super::*;
use egui::{Event, PointerButton};
use egui_kittest::Harness;
use serde_json::json;

#[test]
fn scrub_zoom_is_continuous_and_clamped() {
    const DOC: [u32; 2] = [400, 300];
    assert_eq!(scrub_zoom(1.0, 0.0, DOC), 1.0);
    assert!((scrub_zoom(1.0, DOUBLING, DOC) - 2.0).abs() < 1e-5);
    assert!((scrub_zoom(1.0, -DOUBLING, DOC) - 0.5).abs() < 1e-5);
    // Monotone: every point of drag to the right zooms in a little more.
    let zs: Vec<f32> = (0..50).map(|i| scrub_zoom(0.7, i as f32 * 3.0, DOC)).collect();
    assert!(zs.windows(2).all(|w| w[1] > w[0]));
    assert_eq!(scrub_zoom(1.0, 1e6, DOC), crate::zoom_levels::MAX);
    assert_eq!(scrub_zoom(1.0, -1e6, DOC), crate::zoom_levels::min(DOC));
    assert_eq!(scrub_zoom(2.0, f32::NAN, DOC), 2.0);
}

fn harness(scrubby: bool, ppp: f32) -> Harness<'static, PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app.sync_views();
    app.ui.extras.rulers = false;
    app.ui.tool = crate::state::Tool::Zoom;
    app.ui.tool_options.zoom_scrubby = scrubby;
    let mut h = Harness::builder().with_size(vec2(1000.0, 700.0)).with_pixels_per_point(ppp).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            // Fonts set up after the first frame only apply from the next one.
            if !ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            egui::CentralPanel::default().show(ui, |ui| crate::canvas::document_area(app, ui));
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    h.run_steps(4);
    h
}

fn view_xf(h: &Harness<'static, PhotocraftApp>) -> ViewXform {
    ViewXform::active(h.state()).unwrap()
}

// Pixel alignment derives a presentation camera within half a display pixel of the
// stored target; interrupting an animation must keep that same target and its zoom.
fn assert_camera_center(h: &Harness<'static, PhotocraftApp>, target: &View) {
    let xf = view_xf(h);
    let delta = (xf.to_screen(target.center[0], target.center[1]) - xf.rect.center()) * h.ctx.pixels_per_point();
    assert!(delta.x.abs() <= 0.501 && delta.y.abs() <= 0.501, "camera differs by {delta:?} display pixels");
}

fn press(h: &mut Harness<'static, PhotocraftApp>, p: Pos2, pressed: bool) {
    h.event(Event::PointerButton { pos: p, button: PointerButton::Primary, pressed, modifiers: Default::default() });
    h.step();
}

#[test]
fn scrubby_zoom_drags_continuously_about_the_press_point() {
    let mut h = harness(true, 1.0);
    let r = h.state().last_canvas_rect;
    let p0 = r.center() + vec2(-150.0, 60.0);
    let doc0 = view_xf(&h).to_doc(p0);
    let z0 = h.state().current_zoom();
    h.event(Event::PointerMoved(p0));
    h.step();
    press(&mut h, p0, true);
    // Right: zooms in a little more each frame, the pressed document point stays put.
    let mut last = z0;
    for i in 1..=8 {
        h.event(Event::PointerMoved(p0 + vec2(i as f32 * 15.0, (i % 3) as f32)));
        h.step();
        let z = h.state().current_zoom();
        assert!(z > last, "frame {i}: {last} -> {z}");
        last = z;
        let s = view_xf(&h).to_screen(doc0[0] as f32, doc0[1] as f32);
        assert!(s.distance(p0) < 0.5, "anchor drifted to {s:?} (pressed at {p0:?})");
    }
    // Back left past the press point: zooms out below the start.
    h.event(Event::PointerMoved(p0 - vec2(60.0, 0.0)));
    h.step();
    let z = h.state().current_zoom();
    assert!(z < z0 && (z - scrub_zoom(z0, -60.0, h.state().ui.views[0].doc_size)).abs() < 1e-4, "{z0} -> {z}");
    press(&mut h, p0 - vec2(60.0, 0.0), false);
    h.run_steps(2);
    // Release keeps the zoom; nothing was painted or recorded.
    assert!((h.state().current_zoom() - z).abs() < 1e-5);
    assert_eq!(h.state().session.documents()[0].history.past_len(), 0);
    // A plain click still steps in.
    let zc = h.state().current_zoom();
    h.event(Event::PointerMoved(p0));
    h.step();
    press(&mut h, p0, true);
    press(&mut h, p0, false);
    h.run_steps(2);
    assert_eq!(h.state().current_zoom(), crate::zoom_levels::step(zc, 1, h.state().ui.views[0].doc_size));
}

#[test]
fn animated_zoom_off_click_jumps_to_the_anchored_target() {
    let mut h = harness(true, 1.0);
    h.state_mut()
        .run("prefs.set", json!({"values": {"general.animatedZoom": false, "tools.zoomClickedPointToCenter": false, "tools.overscroll": true}}))
        .unwrap();
    let pointer = h.state().last_canvas_rect.center() + vec2(90.0, -45.0);
    let anchor = view_xf(&h).to_doc(pointer);
    let start = h.state().current_zoom();
    h.event(Event::PointerMoved(pointer));
    h.step();
    press(&mut h, pointer, true);
    press(&mut h, pointer, false);
    let target = crate::zoom_levels::step(start, 1, [400, 300]);
    let v = &h.state().ui.views[0];
    assert_eq!(v.zoom, target);
    assert_eq!(view_xf(&h).zoom, target);
    assert!(!v.zoom_animating && v.zoom_anchor.is_none() && v.zoom_transition.is_none());
    assert!(view_xf(&h).to_screen(anchor[0] as f32, anchor[1] as f32).distance(pointer) < 0.5);
}

#[test]
fn animated_zoom_off_view_commands_show_the_target_on_the_first_frame() {
    for command in
        ["view.zoomIn", "view.zoomOut", "view.actualPixels", "view.fitOnScreen", "view.twoHundredPercent", "view.printSize", "view.fitLayersOnScreen"]
    {
        let mut h = harness(true, 1.0);
        h.state_mut().run("prefs.set", json!({"values": {"general.animatedZoom": false}})).unwrap();
        if crate::view_cmds::handles(command) {
            crate::view_cmds::invoke(h.state_mut(), &egui::Context::default(), command, &json!({})).unwrap().unwrap();
        } else {
            h.state_mut().run(command, json!({})).unwrap();
        }
        assert!(!h.state().ui.views[0].zoom_animating, "{command}");
        h.step();
        let v = &h.state().ui.views[0];
        let xf = view_xf(&h);
        assert_eq!((xf.zoom, xf.center), (v.zoom, v.center), "{command}");
        assert!(v.zoom_anchor.is_none() && v.zoom_transition.is_none(), "{command}");
    }
}

#[test]
fn disabling_animated_zoom_during_a_transition_snaps_and_does_not_resume() {
    let mut h = harness(true, 1.0);
    h.state_mut().run("view.zoomIn", json!({})).unwrap();
    h.step();
    let time = h.ctx.input(|i| i.time) + 0.11;
    h.input_mut().time = Some(time);
    h.step();
    assert!(h.state().ui.views[0].zoom_animating);
    let target = (h.state().ui.views[0].zoom, h.state().ui.views[0].center);
    assert_ne!(view_xf(&h).zoom, target.0);
    h.state_mut().run("prefs.set", json!({"values": {"general.animatedZoom": false}})).unwrap();
    assert_eq!((view_xf(&h).zoom, view_xf(&h).center), target, "pointer mapping switches immediately");
    h.step();
    let v = &h.state().ui.views[0];
    assert!(!v.zoom_animating && v.zoom_anchor.is_none() && v.zoom_transition.is_none());
    assert_eq!((view_xf(&h).zoom, view_xf(&h).center), target);
    h.state_mut().run("prefs.set", json!({"values": {"general.animatedZoom": true}})).unwrap();
    h.step();
    assert!(!h.state().ui.views[0].zoom_animating);
    assert_eq!((view_xf(&h).zoom, view_xf(&h).center), target);
}

#[test]
fn discrete_zoom_animates_and_keeps_its_document_anchor_fixed() {
    let mut h = harness(true, 1.0);
    let r = h.state().last_canvas_rect;
    let pointer = r.center() + vec2(90.0, -45.0);
    let anchor_doc = view_xf(&h).to_doc(pointer);
    let anchor_screen = if h.state().session.prefs().tools.zoom_clicked_point_to_center { r.center() } else { pointer };
    let start = h.state().current_zoom();

    h.event(Event::PointerMoved(pointer));
    h.step();
    press(&mut h, pointer, true);
    press(&mut h, pointer, false);

    let target = crate::zoom_levels::step(start, 1, h.state().ui.views[0].doc_size);
    assert_eq!(h.state().current_zoom(), target, "the model keeps the requested zoom as its target");
    assert!(h.state().ui.views[0].zoom_animating);
    let mut time = h.ctx.input(|i| i.time) + 0.05;
    for _ in 0..5 {
        time += 0.01;
        h.input_mut().time = Some(time);
        h.step();
    }
    let displayed_xf = view_xf(&h);
    let displayed = displayed_xf.to_screen(anchor_doc[0] as f32, anchor_doc[1] as f32);
    assert!(displayed.distance(anchor_screen) < 0.5, "anchor moved to {displayed:?}");
    let intermediate = displayed_xf.zoom;
    assert!(intermediate > start && intermediate < target, "expected an intermediate zoom, got {intermediate}");

    for _ in 0..30 {
        time += 0.01;
        h.input_mut().time = Some(time);
        h.step();
    }
    assert!(!h.state().ui.views[0].zoom_animating);
    assert_eq!(view_xf(&h).zoom, target);
    assert_eq!(h.state().current_zoom(), target);
}

#[test]
fn gpu_zoom_animation_reuses_the_document_composite() {
    let Ok(rs) = std::panic::catch_unwind(|| egui_kittest::wgpu::create_render_state(crate::gpu_canvas::wgpu_setup(), Default::default())) else {
        eprintln!("skipping: no GPU adapter");
        return;
    };
    let renderer = egui_kittest::wgpu::WgpuTestRenderer::from_render_state(rs);
    let mut h = Harness::builder().with_size(vec2(1000.0, 700.0)).with_pixels_per_point(1.0).renderer(renderer).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, crate::theme::ThemeKind::ALL[0]);
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.set_wgpu(cc.wgpu_render_state.clone().unwrap());
        app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
        app.run("edit.fill", json!({"color": "#2060c0"})).unwrap();
        app
    });
    h.run_steps(4);
    h.state_mut().run("view.actualPixels", json!({})).unwrap();
    h.step();
    let time = h.ctx.input(|i| i.time) + 0.3;
    h.input_mut().time = Some(time);
    h.step();
    let id = h.state().session.documents()[0].doc.id;
    assert!(h.state().canvases[&(id, crate::canvas::GPU_OUTPUT)].on_gpu);
    let directory = std::env::var_os("CRAFT_UI_ZOOM_FIXTURES").map(std::path::PathBuf::from);
    if let Some(directory) = &directory {
        std::fs::create_dir_all(directory).unwrap();
    }
    h.state_mut().run("view.zoomIn", json!({})).unwrap();
    let target = h.state().current_zoom();
    for (name, elapsed) in [("start", 0.0), ("midpoint", 0.11), ("finished", 0.23)] {
        h.state_mut().perf.last_refresh = "";
        h.input_mut().time = Some(time + elapsed);
        h.step();
        assert!(h.state().perf.gpu, "{name}: keep the GPU canvas");
        assert_eq!(h.state().perf.last_refresh, "", "{name}: reuse the composite");
        assert!(h.state().canvases.values().all(|cache| cache.texture.is_none()), "{name}: no CPU texture fallback");
        let xf = view_xf(&h);
        if name == "midpoint" {
            assert!(xf.zoom > 1.0 && xf.zoom < target);
        }
        let image = h.render().unwrap();
        let center = xf.to_screen(200.0, 150.0);
        let pixel = image.get_pixel(center.x as u32, center.y as u32);
        assert!(pixel[2] > pixel[0] * 2, "{name}: document remains visible: {pixel:?}");
        if let Some(directory) = &directory {
            image.save(directory.join(format!("photocraft-zoom-{name}.png"))).unwrap();
        }
    }
    assert!(!h.state().ui.views[0].zoom_animating);
    assert_eq!(view_xf(&h).zoom, target);
    h.state_mut().run("prefs.set", json!({"values": {"general.animatedZoom": false}})).unwrap();
    h.state_mut().run("view.zoomIn", json!({})).unwrap();
    h.state_mut().perf.last_refresh = "";
    h.step();
    assert_eq!(view_xf(&h).zoom, h.state().current_zoom());
    assert!(!h.state().ui.views[0].zoom_animating);
    assert!(h.state().perf.gpu);
    assert_eq!(h.state().perf.last_refresh, "", "immediate zoom reuses the GPU composite");
    let image = h.render().unwrap();
    if let Some(directory) = &directory {
        image.save(directory.join("photocraft-zoom-disabled.png")).unwrap();
    }
}

#[test]
fn view_commands_retarget_a_zoom_click_without_retaining_its_anchor() {
    for command in ["view.zoomIn", "view.actualPixels", "view.fitOnScreen"] {
        let mut h = harness(true, 1.0);
        let pointer = h.state().last_canvas_rect.center() + vec2(90.0, -45.0);
        h.event(Event::PointerMoved(pointer));
        h.step();
        press(&mut h, pointer, true);
        press(&mut h, pointer, false);
        assert!(h.state().ui.views[0].zoom_anchor.is_some());
        h.step();
        h.state_mut().run(command, json!({})).unwrap();
        assert!(h.state().ui.views[0].zoom_anchor.is_none(), "{command}");
        h.step();
        let time = h.ctx.input(|i| i.time) + 0.3;
        h.input_mut().time = Some(time);
        h.step();
        let target = &h.state().ui.views[0];
        assert!(!target.zoom_animating, "{command}");
        assert_eq!(view_xf(&h).zoom, target.zoom);
        assert_camera_center(&h, target);
    }
}

#[test]
fn wheel_pinch_pan_and_scrubby_zoom_interrupt_a_transition() {
    for gesture in 0..5 {
        let mut h = harness(true, 1.0);
        let pointer = h.state().last_canvas_rect.center();
        h.event(Event::PointerMoved(pointer));
        h.step();
        h.state_mut().run("view.zoomIn", json!({})).unwrap();
        h.step();
        assert!(h.state().ui.views[0].zoom_animating);
        match gesture {
            0 => h.event(Event::Zoom(1.25)),
            1 | 2 => h.event(Event::MouseWheel {
                unit: egui::MouseWheelUnit::Line,
                delta: vec2(0.0, 1.0),
                phase: egui::TouchPhase::Move,
                source: egui::MouseWheelSource::Unknown,
                modifiers: egui::Modifiers { alt: gesture == 1, ..Default::default() },
            }),
            _ => {
                h.state_mut().ui.tool = if gesture == 3 { crate::state::Tool::Hand } else { crate::state::Tool::Zoom };
                press(&mut h, pointer, true);
                h.event(Event::PointerMoved(pointer + vec2(120.0, 0.0)));
            }
        }
        h.step();
        assert!(!h.state().ui.views[0].zoom_animating, "gesture {gesture}");
        h.step();
        let target = &h.state().ui.views[0];
        assert_eq!(view_xf(&h).zoom, target.zoom, "gesture {gesture}");
        assert_camera_center(&h, target);
    }
}

#[test]
fn a_scrub_release_keeps_a_view_centred_since_its_last_zoom() {
    // Zoom Resizes Windows centres the image once its window wraps it (zoom_window_fit.rs):
    // the release frame, at the same zoom, must not pull it back to the press point.
    let mut h = harness(true, 1.0);
    let p0 = h.state().last_canvas_rect.center() + vec2(-150.0, 60.0);
    h.event(Event::PointerMoved(p0));
    h.step();
    press(&mut h, p0, true);
    let end = p0 + vec2(90.0, 0.0);
    h.event(Event::PointerMoved(end));
    h.step();
    let z = h.state().current_zoom();
    h.state_mut().ui.views[0].center = [200.0, 150.0];
    h.step();
    assert_eq!(h.state().ui.views[0].center, [200.0, 150.0], "a held, unmoved scrub keeps the view");
    press(&mut h, end, false);
    h.run_steps(2);
    assert_eq!(h.state().current_zoom(), z);
    assert_eq!(h.state().ui.views[0].center, [200.0, 150.0], "the release keeps the view");
}

#[test]
fn zoom_drags_keep_the_press_point_in_rotated_and_flipped_views() {
    for (scrubby, out, rotation, flip, ppp) in [
        (true, false, 30.0, false, 1.0),
        (true, false, -45.0, true, 1.0),
        (true, false, 90.0, false, 2.0),
        (true, false, 30.0, true, 2.0),
        (false, false, 30.0, false, 1.0),
        (false, true, -45.0, true, 2.0),
    ] {
        let mut h = harness(scrubby, ppp);
        let view = &mut h.state_mut().ui.views[0];
        view.zoom = 1.0;
        view.center = [200.0, 150.0];
        view.rotation = rotation;
        view.fit_pending = false;
        view.fill_pending = false;
        h.state_mut().ui.view.flip_horizontal = flip;
        h.run_steps(2);
        let anchor = view_xf(&h).to_screen(140.0, 110.0);
        let modifiers = egui::Modifiers { alt: out, ..Default::default() };
        h.event(Event::ModifiersChanged(modifiers));
        h.event(Event::PointerMoved(anchor));
        h.step();
        h.event(Event::PointerButton { pos: anchor, button: PointerButton::Primary, pressed: true, modifiers });
        h.step();
        let steps: &[f32] = if scrubby { &[15.0, 50.0, 100.0, -40.0] } else { &[20.0] };
        let mut end = anchor;
        for &dx in steps {
            end = anchor + vec2(dx, if scrubby { 0.0 } else { 1.0 });
            h.event(Event::PointerMoved(end));
            h.step();
            let actual = view_xf(&h).to_screen(140.0, 110.0);
            assert!(actual.distance(anchor) < 0.01, "rotation={rotation}, flip={flip}, ppp={ppp}: {anchor:?} -> {actual:?}");
            let expected = if scrubby { scrub_zoom(1.0, dx, [400, 300]) } else { 1.0 };
            assert!((h.state().current_zoom() - expected).abs() < 1e-5);
        }
        h.event(Event::PointerButton { pos: end, button: PointerButton::Primary, pressed: false, modifiers });
        h.step();
        h.run_steps(2);
        assert!(view_xf(&h).to_screen(140.0, 110.0).distance(anchor) < 0.01);
        let expected = if scrubby { scrub_zoom(1.0, -40.0, [400, 300]) } else { crate::zoom_levels::step(1.0, if out { -1 } else { 1 }, [400, 300]) };
        assert!((h.state().current_zoom() - expected).abs() < 1e-5);
        assert_eq!(h.state().ui.views[0].rotation, rotation);
        assert_eq!(h.state().ui.view.flip_horizontal, flip);
        assert_eq!(h.state().session.documents()[0].history.past_len(), 0);
    }
}

#[test]
fn zoom_rectangle_without_scrubby_zoom() {
    let mut h = harness(false, 1.0);
    let r = h.state().last_canvas_rect;
    let z0 = h.state().current_zoom();
    let a = r.center() - vec2(60.0, 40.0);
    let b = r.center() + vec2(60.0, 40.0);
    let mid = view_xf(&h).to_doc(r.center());
    h.event(Event::PointerMoved(a));
    h.step();
    press(&mut h, a, true);
    for t in [0.3, 0.7, 1.0] {
        h.event(Event::PointerMoved(a + (b - a) * t));
        h.step();
    }
    assert_eq!(h.state().current_zoom(), z0, "no zoom while the rectangle is drawn");
    press(&mut h, b, false);
    h.run_steps(2);
    let z = h.state().current_zoom();
    let k = (r.width() / 120.0).min(r.height() / 80.0);
    assert!((z - z0 * k).abs() < z0 * k * 0.02, "{z0} * {k} vs {z}");
    // The rectangle's centre is now the view centre.
    let c = h.state().ui.views[0].center;
    assert!((c[0] as f64 - mid[0]).abs() < 1.0 && (c[1] as f64 - mid[1]).abs() < 1.0, "{c:?} vs {mid:?}");
}
