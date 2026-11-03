//! The Navigator draws the GPU canvas's own texture while the canvas is on the GPU (#4518), so an
//! edit costs it no CPU composite; elsewhere (a flipped view draws on the CPU) it keeps the CPU
//! thumbnail. Renders the real app offscreen; skips without a GPU adapter.

use crate::PhotocraftApp;
use serde_json::json;

type Harness = egui_kittest::Harness<'static, PhotocraftApp>;

fn harness() -> Option<Harness> {
    let built = std::panic::catch_unwind(|| {
        egui_kittest::Harness::builder()
            .with_size(egui::vec2(1400.0, 900.0))
            .with_pixels_per_point(1.0)
            .with_max_steps(64)
            .wgpu_setup(crate::gpu_canvas::wgpu_setup())
            .build_eframe(|cc| {
                PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
                let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
                if let Some(rs) = cc.wgpu_render_state.as_ref() {
                    app.set_wgpu(rs.clone());
                }
                app.ui.panels.navigator = true;
                app.run("file.new", json!({"width": 400, "height": 300, "background": "white"})).unwrap();
                app.run("select.rect", json!({"x": 0, "y": 0, "width": 200, "height": 300})).unwrap();
                app.run("edit.fill", json!({"color": "#ff0000"})).unwrap();
                app.run("select.deselect", json!({})).unwrap();
                app.sync_views();
                app
            })
    });
    match built {
        Ok(h) => Some(h),
        Err(_) => {
            eprintln!("skipping: no GPU adapter");
            None
        }
    }
}

/// The colour at fractions (`fx`, `fy`) of the Navigator preview.
fn preview_px(h: &mut Harness, fx: f32, fy: f32) -> [u8; 4] {
    let r = crate::panels::last_navigator_preview(&h.ctx).expect("navigator preview drawn");
    let img = h.render().expect("render");
    let (x, y) = ((r.left() + r.width() * fx) as u32, (r.top() + r.height() * fy) as u32);
    img.get_pixel(x, y).0
}

fn near(a: [u8; 4], b: [u8; 3]) -> bool {
    (0..3).all(|i| a[i].abs_diff(b[i]) <= 8)
}

#[test]
fn navigator_draws_the_gpu_canvas_texture_and_follows_edits() {
    let _gpu = photocraft_testkit::gpu_lock();
    let Some(mut h) = harness() else { return };
    h.run_steps(8);
    if !h.state().perf.gpu {
        eprintln!("skipping: the canvas fell back to the CPU here");
        return;
    }
    let id = h.state().session.active().expect("doc").doc.id;
    assert!(h.state().navigator_gpu.contains_key(&id), "the Navigator uses the GPU canvas view");
    assert!(!h.state().navigator_textures.contains_key(&id), "no CPU thumbnail was built");
    assert!(near(preview_px(&mut h, 0.25, 0.5), [255, 0, 0]), "left half red");
    assert!(near(preview_px(&mut h, 0.75, 0.5), [255, 255, 255]), "right half white");

    // An edit shows in the Navigator, still without a CPU thumbnail.
    h.state_mut().run("select.rect", json!({"x": 200, "y": 0, "width": 200, "height": 300})).unwrap();
    h.state_mut().run("edit.fill", json!({"color": "#0000ff"})).unwrap();
    h.run_steps(6);
    assert!(near(preview_px(&mut h, 0.75, 0.5), [0, 0, 255]), "right half now blue");
    assert!(!h.state().navigator_textures.contains_key(&id));

    // Dragging the selected pixels with the Move tool keeps the Navigator on the GPU texture
    // without falling back to a full-document CPU thumbnail composite on the UI thread.
    h.state_mut().ui.tool = crate::state::Tool::Move;
    crate::canvas::tool_event(h.state_mut(), crate::canvas::ToolEvent::Down { x: 250.0, y: 150.0, pressure: 1.0 }, egui::Modifiers::NONE);
    crate::canvas::tool_event(h.state_mut(), crate::canvas::ToolEvent::Move { x: 150.0, y: 150.0, pressure: 1.0 }, egui::Modifiers::NONE);
    h.run_steps(4);
    assert!(h.state().navigator_gpu.contains_key(&id), "Navigator stays on GPU canvas during Move drag");
    assert!(!h.state().navigator_textures.contains_key(&id), "no CPU thumbnail built during Move drag");
    crate::canvas::tool_event(h.state_mut(), crate::canvas::ToolEvent::Up { x: 150.0, y: 150.0 }, egui::Modifiers::NONE);
    h.state_mut().run("select.drop", json!({})).unwrap();
    h.state_mut().run("edit.undo", json!({})).unwrap();
    h.state_mut().run("select.deselect", json!({})).unwrap();
    h.run_steps(4);

    // A flipped view draws on the CPU: the Navigator goes back to its CPU thumbnail.
    h.state_mut().ui.view.flip_horizontal = true;
    h.run_steps(6);
    assert!(!h.state().navigator_gpu.contains_key(&id), "no GPU view while the canvas draws on the CPU");
    assert!(h.state().navigator_textures.contains_key(&id), "the CPU thumbnail again");
    assert!(near(preview_px(&mut h, 0.25, 0.5), [255, 0, 0]) && near(preview_px(&mut h, 0.75, 0.5), [0, 0, 255]), "same picture, unflipped");
}
