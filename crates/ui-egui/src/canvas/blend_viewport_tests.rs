use super::*;
use photocraft_color::BlendMode;
use serde_json::json;

fn render_state() -> Option<eframe::egui_wgpu::RenderState> {
    match std::panic::catch_unwind(|| egui_kittest::wgpu::create_render_state(crate::gpu_canvas::wgpu_setup(), Default::default())) {
        Ok(rs) => Some(rs),
        Err(error) => {
            if std::env::var_os("PHOTOCRAFT_REQUIRE_GPU_TESTS").is_some() {
                std::panic::resume_unwind(error);
            }
            eprintln!("skipping blend viewport regression: no GPU adapter");
            None
        }
    }
}

#[test]
fn blend_hover_refresh_is_limited_to_visible_canvas() {
    check_blend_viewport(false, false);
}

#[test]
fn blend_hover_cpu_fallback_refresh_tracks_visible_canvas() {
    check_blend_viewport(true, false);
}

#[test]
fn blend_hover_multiple_layers_tracks_visible_canvas() {
    check_blend_viewport(false, true);
}

#[test]
fn blend_hover_pattern_tiles_and_multiple_views_reuse_current_pixels() {
    let _guard = photocraft_testkit::gpu_lock();
    let Some(rs) = render_state() else { return };
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.set_wgpu(rs);
    app.run("file.new", json!({"width":1024,"height":768,"background":"#336699"})).unwrap();
    let layer = app.run("layer.new.layer", json!({})).unwrap()["layer"].as_u64().unwrap();
    app.run("edit.fill", json!({"color":"#cc8844"})).unwrap();
    let ctx = egui::Context::default();
    let a = DRect::new(724, 200, 1324, 500);
    let b = DRect::new(0, 200, 300, 500);
    assert!(ensure_gpu(&mut app, &ctx, 0, a));
    app.ui.view.pattern_preview = true;
    crate::blend_preview::hover(&mut app, &[photocraft_doc::LayerId(layer)], Some(BlendMode::Multiply));
    assert!(ensure_gpu(&mut app, &ctx, 0, a));
    assert_visible_pixels(&mut app, b);
    let bounds = app.session.active().unwrap().doc.bounds();
    assert_visible_pixels(&mut app, bounds);
    crate::blend_preview::hover(&mut app, &[photocraft_doc::LayerId(layer)], None);
    assert!(ensure_gpu(&mut app, &ctx, 0, a));
    assert_visible_pixels(&mut app, bounds);

    app.ui.view.pattern_preview = false;
    for mode in [BlendMode::Multiply, BlendMode::Screen] {
        crate::blend_preview::hover(&mut app, &[photocraft_doc::LayerId(layer)], Some(mode));
        assert!(ensure_gpu(&mut app, &ctx, 0, a));
        assert!(ensure_gpu(&mut app, &ctx, 0, b));
        for visible in [a, b, a, b] {
            app.perf.last_refresh_px = 0;
            assert!(ensure_gpu(&mut app, &ctx, 0, visible));
            assert_eq!(app.perf.last_refresh_px, 0, "unchanged views must retain coverage together");
            assert_visible_pixels(&mut app, visible.intersect(&bounds));
        }
    }
    // Enabling periodic display during an existing hover must catch up the unseen gap too.
    app.ui.view.pattern_preview = true;
    assert!(ensure_gpu(&mut app, &ctx, 0, a));
    assert_visible_pixels(&mut app, bounds);
    crate::blend_preview::hover(&mut app, &[photocraft_doc::LayerId(layer)], None);
    assert!(ensure_gpu(&mut app, &ctx, 0, a));
    assert_visible_pixels(&mut app, bounds);
}

#[test]
fn blend_hover_during_background_redraw_restores_committed_pixels() {
    let _guard = photocraft_testkit::gpu_lock();
    let Some(rs) = render_state() else { return };
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.set_wgpu(rs);
    app.background_jobs = true;
    app.run("file.new", json!({"width":2600,"height":1700,"background":"#336699"})).unwrap();
    let layer = app.run("layer.new.layer", json!({})).unwrap()["layer"].as_u64().unwrap();
    app.run("edit.fill", json!({"color":"#cc8844"})).unwrap();
    app.run("layer.layerStyle.blendingOptions", json!({"knockout":"shallow"})).unwrap();
    let ctx = egui::Context::default();
    let visible = DRect::new(256, 256, 512, 512);
    assert!(ensure_gpu(&mut app, &ctx, 0, visible));
    assert!(!app.redraws.is_empty(), "fixture must start a deferred full redraw");
    crate::blend_preview::hover(&mut app, &[photocraft_doc::LayerId(layer)], Some(BlendMode::Multiply));
    assert!(ensure_gpu(&mut app, &ctx, 0, visible));
    let panned = DRect::new(2048, 1024, 2304, 1280);
    assert!(ensure_gpu(&mut app, &ctx, 0, panned));
    settle_redraw(&mut app);
    assert_visible_pixels(&mut app, panned);
    crate::blend_preview::hover(&mut app, &[photocraft_doc::LayerId(layer)], None);
    assert!(ensure_gpu(&mut app, &ctx, 0, panned));
    settle_redraw(&mut app);
    let bounds = app.session.active().unwrap().doc.bounds();
    assert_visible_pixels(&mut app, bounds);
}

fn settle_redraw(app: &mut PhotocraftApp) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    let gpu = app.gpu.clone().unwrap();
    let id = app.session.active().unwrap().doc.id;
    while !app.redraws.is_empty() {
        crate::background_redraw::poll(app, &gpu, id);
        assert!(std::time::Instant::now() < deadline, "background redraw did not finish");
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
}

fn assert_visible_pixels(app: &mut PhotocraftApp, visible: DRect) {
    let (doc, _) = display_doc(app, 0);
    let gpu = app.gpu.clone().unwrap();
    let reference = doc.id.0 ^ (1u64 << 60);
    let (display, _) = canvas_display(app, &doc, None);
    gpu.upload_composite(reference, &doc, display.as_deref());
    let (_, size, shown) = gpu.read_texels(doc.id.0).unwrap();
    let (_, _, want) = gpu.read_texels(reference).unwrap();
    for y in visible.y0..visible.y1 {
        for x in visible.x0..visible.x1 {
            let index = y as usize * size[0] as usize + x as usize;
            for channel in 0..4 {
                assert!((shown[index][channel] - want[index][channel]).abs() <= 2.0 / 255.0, "preview differs at ({x},{y}) channel{channel}");
            }
        }
    }
}

fn check_blend_viewport(cpu_fallback: bool, multiple_layers: bool) {
    let _guard = photocraft_testkit::gpu_lock();
    let Some(rs) = render_state() else { return };
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.set_wgpu(rs);
    app.run("file.new", json!({"width":1024,"height":768,"background":"#336699"})).unwrap();
    let layer = app.run("layer.new.layer", json!({})).unwrap()["layer"].as_u64().unwrap();
    app.run("edit.fill", json!({"color":"#cc8844"})).unwrap();
    if cpu_fallback {
        app.run("layer.layerStyle.blendingOptions", json!({"knockout":"shallow"})).unwrap();
    }
    let mut layers = vec![photocraft_doc::LayerId(layer)];
    if multiple_layers {
        let other = app.run("layer.new.layer", json!({})).unwrap()["layer"].as_u64().unwrap();
        app.run("edit.fill", json!({"color": "#4488cc"})).unwrap();
        app.run("layer.setProps", json!({"layer": other, "opacity": 0.5})).unwrap();
        layers.push(photocraft_doc::LayerId(other));
    }
    let ctx = egui::Context::default();
    let visible = DRect::new(256, 256, 512, 512);
    assert!(ensure_gpu(&mut app, &ctx, 0, visible));
    assert!(app.redraws.is_empty(), "small fixture must finish synchronously");
    if cpu_fallback {
        assert!(!app.perf.last_refresh.starts_with("gpu"), "fixture must exercise CPU composition fallback");
    }
    let id = app.session.active().unwrap().doc.id;
    let gpu = app.gpu.clone().unwrap();
    let original = gpu.read_texels(id.0).unwrap();
    crate::blend_preview::hover(&mut app, &layers, Some(BlendMode::Multiply));
    assert!(ensure_gpu(&mut app, &ctx, 0, visible));
    println!("BLEND_VIEWPORT kind={} pixels={} expected={} adapter={:?}", app.perf.last_refresh, app.perf.last_refresh_px, 256 * 256, app.perf.gpu_info);
    assert_eq!(app.perf.last_refresh_px, 256 * 256, "hover must not recomposite offscreen pixels of a full-canvas layer");
    assert_visible_pixels(&mut app, visible);
    app.perf.last_refresh_px = 0;
    assert!(ensure_gpu(&mut app, &ctx, 0, visible));
    assert_eq!(app.perf.last_refresh_px, 0, "unchanged view and mode must reuse the texture");
    let zoomed_out = DRect::new(128, 128, 640, 640);
    assert!(ensure_gpu(&mut app, &ctx, 0, zoomed_out));
    assert_eq!(app.perf.last_refresh_px, 512 * 512, "zooming out catches up the larger visible area");
    let panned = DRect::new(640, 256, 896, 512);
    assert!(ensure_gpu(&mut app, &ctx, 0, panned));
    assert_eq!(app.perf.last_refresh_px, 256 * 256, "newly exposed pixels must catch up without a mode change");
    crate::blend_preview::hover(&mut app, &layers, Some(BlendMode::Screen));
    assert!(ensure_gpu(&mut app, &ctx, 0, panned));
    assert_eq!(app.perf.last_refresh_px, 256 * 256);
    assert!(ensure_gpu(&mut app, &ctx, 0, visible));
    assert_eq!(app.perf.last_refresh_px, 256 * 256, "returning view must show the current mode");
    assert_visible_pixels(&mut app, visible);
    assert_visible_pixels(&mut app, panned);
    crate::blend_preview::hover(&mut app, &layers, None);
    assert!(ensure_gpu(&mut app, &ctx, 0, visible));
    assert_eq!(gpu.read_texels(id.0).unwrap(), original, "cancel must restore the whole texture, including previously previewed offscreen pixels");
    crate::blend_preview::hover(&mut app, &layers, Some(BlendMode::Multiply));
    assert!(ensure_gpu(&mut app, &ctx, 0, visible));
    app.run("layer.setProps", json!({"layer":layer,"blend":"Multiply"})).unwrap();
    assert!(ensure_gpu(&mut app, &ctx, 0, visible));
    let bounds = app.session.active().unwrap().doc.bounds();
    assert_visible_pixels(&mut app, bounds);
    app.run("file.new", json!({"width":1024,"height":768,"background":"white"})).unwrap();
    assert!(ensure_gpu(&mut app, &ctx, 1, visible));
    app.session.set_active(0);
    app.sync_views();
    assert!(ensure_gpu(&mut app, &ctx, 0, visible));
    assert_visible_pixels(&mut app, bounds);
}
