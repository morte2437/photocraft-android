//! Crop tool: Esc and Enter belong to an open options bar popup first (#7386). The shortcut pass
//! runs before the panels are drawn, so the crop branch used to take the key: Esc cancelled the
//! frame and left the Ratio list open, Enter committed the crop beneath it.
//!
//! Skips when no GPU adapter exists (like `crop_default_mode.rs`).

use egui::Key;
use egui_kittest::kittest::Queryable;
use photocraft_ui_egui::PhotocraftApp;
use photocraft_ui_egui::control::{ControlRequest, handle};
use serde_json::{Value, json};

type Harness = egui_kittest::Harness<'static, PhotocraftApp>;

fn harness() -> Option<Harness> {
    let built = std::panic::catch_unwind(|| {
        egui_kittest::Harness::builder().with_size(egui::vec2(900.0, 640.0)).with_max_steps(64).wgpu().build_eframe(|cc| {
            PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
            let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
            // The queries below use English labels; don't follow the system locale.
            app.session.prefs.edit(|p| p.interface.language = "en".into());
            if let Some(rs) = cc.wgpu_render_state.as_ref() {
                app.set_wgpu(rs.clone());
            }
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

fn control(h: &mut Harness, method: &str, params: Value) {
    let ctx = h.ctx.clone();
    let (req, _rx) = ControlRequest::new(method, params);
    handle(h.state_mut(), &ctx, &req);
    h.run_steps(3);
}

/// A 200x100 document with the Crop tool's 1:1 frame pending and the Ratio list open again.
fn setup() -> Option<Harness> {
    let mut h = harness()?;
    h.run_steps(4);
    {
        let app = h.state_mut();
        app.run("file.new", json!({"width": 200, "height": 100, "background": "white"})).expect("new");
        app.sync_views();
    }
    control(&mut h, "ui.set", json!({"tool": "crop", "zoom": 1.0, "center": [100, 50]}));
    h.get_by_value("Ratio").click();
    h.run_steps(3);
    h.get_by_label("1 : 1 (Square)").click();
    h.run_steps(3);
    // Picking a ratio only fits the untouched default frame; a keyed edit makes it a drawn one.
    assert!(photocraft_ui_egui::crop_ui::nudge(h.state_mut(), 0.0, 0.0));
    h.run_steps(3);
    assert!(photocraft_ui_egui::crop_ui::pending(h.state()), "the 1:1 frame is pending");
    assert!(!egui::Popup::is_any_open(&h.ctx), "the list closed after the pick");
    h.get_by_value("1 : 1 (Square)").click();
    h.run_steps(3);
    assert!(egui::Popup::is_any_open(&h.ctx), "the Ratio list is open again");
    Some(h)
}

#[test]
fn esc_closes_the_open_ratio_list_and_keeps_the_frame() {
    // Windows WARP can access-violate when GPU harnesses initialize or tear down concurrently:
    // hold the GPU lock for the whole test, harness included.
    let _gpu = photocraft_testkit::gpu_lock();
    let Some(mut h) = setup() else { return };
    let frame = h.state().ui.crop_rect;
    h.key_press(Key::Escape);
    h.run_steps(3);
    assert!(!egui::Popup::is_any_open(&h.ctx), "Esc closes the list");
    assert_eq!(h.state().ui.crop_rect, frame, "the frame stays as it was");
    assert!(photocraft_ui_egui::crop_ui::pending(h.state()), "still pending");
    // With the list closed, the keys reach the crop again: Enter commits it.
    h.key_press(Key::Enter);
    h.run_steps(3);
    assert_eq!(h.state().session.active().unwrap().doc.size, photocraft_doc::Size::new(100, 100), "the next Enter crops");
}

#[test]
fn enter_with_the_ratio_list_open_does_not_commit_the_crop() {
    let _gpu = photocraft_testkit::gpu_lock();
    let Some(mut h) = setup() else { return };
    h.key_press(Key::Enter);
    h.run_steps(3);
    assert_eq!(h.state().session.active().unwrap().doc.size, photocraft_doc::Size::new(200, 100), "nothing cropped");
    assert!(photocraft_ui_egui::crop_ui::pending(h.state()), "the frame is still pending");
}
