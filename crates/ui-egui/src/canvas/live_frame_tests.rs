//! Live painting tools show this frame's pointer input in this frame (no extra frame of delay),
//! and the deferred image follows the animated camera.

use super::*;
use serde_json::json;

#[test]
fn live_brush_texture_includes_this_frames_press_and_moves() {
    for tool in [Tool::Brush, Tool::MixerBrush] {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 128, "height": 64, "background": "white"})).unwrap();
        app.run("tools.setColors", json!({"foreground": "#ff0000"})).unwrap();
        app.run("tools.setBrush", json!({"brush": {"size": 12, "hardness": 1.0, "smoothing": {"amount": 0.0}}})).unwrap();
        app.ui.tool = tool;
        app.sync_views();
        let mut h = egui_kittest::Harness::builder().with_size(vec2(900.0, 640.0)).build_ui_state(
            |ui, app: &mut PhotocraftApp| {
                if !ui.ctx().fonts(|fonts| fonts.families().contains(&egui::FontFamily::Name("medium".into()))) {
                    return;
                }
                egui::CentralPanel::default().show(ui, |ui| document_area(app, ui));
            },
            app,
        );
        PhotocraftApp::setup_context(&h.ctx, Default::default());
        h.run_steps(3);
        let p = h.state().last_canvas_rect.center();
        h.input_mut().events.push(egui::Event::PointerMoved(p));
        h.input_mut().events.push(egui::Event::PointerButton { pos: p, button: PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE });
        h.run_steps(1);
        let shown = |h: &mut egui_kittest::Harness<'_, PhotocraftApp>| {
            let app = h.state_mut();
            let live = app.live_stroke.as_ref().expect("painting starts on press");
            let key = live.display_key();
            let doc = live.doc().clone();
            let (_, display) = canvas_display(app, &doc, None);
            assert_eq!(app.canvases.values().next().unwrap().tex_preview_key, key ^ display, "{tool:?}: texture includes this frame's input");
        };
        shown(&mut h);
        h.input_mut().events.push(egui::Event::PointerMoved(p + vec2(25.0, 0.0)));
        h.run_steps(1);
        shown(&mut h);
        let end = p + vec2(25.0, 0.0);
        h.input_mut().events.push(egui::Event::PointerButton { pos: end, button: PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE });
        h.run_steps(1);
        assert!(h.state().live_stroke.is_none());
        // The Brush keeps the shown pixels as its committed stroke; the Mixer Brush commits the
        // pixels its live preview rendered (`gesture.rs`).
        if tool == Tool::Brush {
            assert!(h.state().committed_stroke.is_some(), "release reuses the shown pixels");
        }
        let label = h.state().session.active().and_then(|st| st.history.undo_label()).map(str::to_owned);
        assert_eq!(label.as_deref(), Some(if tool == Tool::MixerBrush { "Mixer Brush" } else { "Brush Tool" }));
        assert!(!h.state().ui.status_error, "{}", h.state().ui.status);
    }
}

#[test]
fn live_tools_paint_the_interpolated_camera_during_zoom_animation() {
    for tool in [Tool::Brush, Tool::MixerBrush] {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 64, "height": 64, "background": "white"})).unwrap();
        app.run("prefs.set", json!({"values": {"general.animatedZoom": true}})).unwrap();
        app.sync_views();
        app.ui.tool = tool;
        let v = &mut app.ui.views[0];
        v.fit_pending = false;
        v.zoom = 2.0;
        v.center = [32.0, 32.0];
        v.doc_size = [64, 64];
        v.zoom_animating = true;
        v.display_camera = Some((1.0, [32.0, 32.0]));
        v.zoom_transition = Some(crate::state::ZoomTransition { from: (1.0, [32.0, 32.0]), target: (2.0, [32.0, 32.0]), started: 0.0 });
        let view = v.clone();
        let ctx = egui::Context::default();
        let tex = ensure_texture(&mut app, &ctx, 0, None).unwrap().0;
        let rect = Rect::from_min_size(Pos2::ZERO, vec2(500.0, 400.0));
        let mut out = ctx.run_ui(egui::RawInput { screen_rect: Some(rect), time: Some(0.1), ..Default::default() }, |ui| {
            canvas_view(&mut app, ui, 0, rect, view.clone(), true);
        });
        out.textures_delta.clear();
        let mesh = out
            .shapes
            .iter()
            .find_map(|s| match &s.shape {
                egui::Shape::Mesh(mesh) if mesh.texture_id == tex => Some(mesh),
                _ => None,
            })
            .unwrap();
        let min = mesh.vertices.iter().map(|v| v.pos.x).fold(f32::INFINITY, f32::min);
        let max = mesh.vertices.iter().map(|v| v.pos.x).fold(f32::NEG_INFINITY, f32::max);
        let shown = app.ui.views[0].display_camera.unwrap().0;
        assert!(shown > 1.0 && shown < 2.0, "{tool:?}: animation is between endpoints");
        assert!(((max - min) - 64.0 * shown).abs() < 0.01, "{tool:?}: image must use the animated camera, width {} at zoom {shown}", max - min);
    }
}
