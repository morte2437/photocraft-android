//! #9400: while a Free Transform handle is dragged, the W/H readout beside the pointer shows the
//! box's size in the Units & Rulers unit (as Photoshop does), not always its scale in percent.
//!
//! The real app runs offscreen on wgpu with real egui pointer events. Skips when no GPU adapter
//! exists (like `drag_preview_canvas.rs`). Set `PHOTOCRAFT_TRANSFORM_READOUT_EVIDENCE_DIR` to save
//! the frames, mid-drag, as PNGs.

use egui::{Modifiers, PointerButton, Pos2};
use egui_kittest::kittest::Queryable;
use photocraft_engine::prefs::Unit;
use photocraft_ui_egui::PhotocraftApp;
use photocraft_ui_egui::canvas::ViewXform;
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

fn xf(h: &Harness) -> ViewXform {
    ViewXform::active(h.state()).expect("a view")
}

fn press(h: &mut Harness, p: Pos2, pressed: bool) {
    h.event(egui::Event::PointerMoved(p));
    h.event(egui::Event::PointerButton { pos: p, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
    h.step();
}

/// A 400 × 300 px document at 72 ppi, 100 %, with a 100 × 50 px box on its own layer at
/// (100, 100) and Free Transform open on it.
fn setup() -> Option<Harness> {
    let mut h = harness()?;
    h.run_steps(4);
    {
        let app = h.state_mut();
        app.run("file.new", json!({"width": 400, "height": 300, "background": "white"})).expect("new");
        app.run("layer.new.layer", json!({})).expect("layer");
        app.run("select.rect", json!({"x": 100, "y": 100, "width": 100, "height": 50})).expect("select");
        app.run("edit.fill", json!({"color": "#3366cc"})).expect("fill");
        app.run("select.deselect", json!({})).expect("deselect");
        app.sync_views();
        app.ui.extras.rulers = true;
        app.ui.extras.snap = false;
        app.ui.view.show.smart_guides = false;
    }
    control(&mut h, "ui.set", json!({"tool": "move", "zoom": 1.0, "center": [200, 150]}));
    control(&mut h, "ui.menu.invoke", json!({"id": "edit.freeTransform"}));
    assert!(h.state().ui.transform.is_some(), "the box is open");
    Some(h)
}

#[test]
fn the_scale_readout_shows_the_size_in_the_ruler_unit() {
    // Windows WARP can access-violate when GPU harnesses initialize or tear down concurrently:
    // hold the GPU lock for the whole test, harness included.
    let _gpu = photocraft_testkit::gpu_lock();
    // The right edge from x 200 to 300 doubles the box (proportionally): 200 × 100 px, which at
    // 72 ppi is 7.06 × 3.53 cm; 200 % of the original box with the rulers in percent.
    for (unit, name, w, h_text) in
        [(Unit::Pixels, "pixels", "200 px", "100 px"), (Unit::Centimeters, "centimeters", "7.06 cm", "3.53 cm"), (Unit::Percent, "percent", "200.0%", "200.0%")]
    {
        let Some(mut h) = setup() else { return };
        h.state_mut().session.prefs.edit(|p| p.units_and_rulers.rulers = unit);
        let x = xf(&h);
        let (from, to) = (x.to_screen(200.0, 125.0), x.to_screen(300.0, 125.0));
        press(&mut h, from, true);
        for i in 1..=8 {
            h.event(egui::Event::PointerMoved(from + (to - from) * (i as f32 / 8.0)));
            h.step();
        }
        h.run_steps(2);
        // `query_all`: in percent W and H both read 200.0%.
        assert!(h.query_all_by_label(w).next().is_some(), "{name}: W reads {w}");
        assert!(h.query_all_by_label(h_text).next().is_some(), "{name}: H reads {h_text}");
        // The readout used to show percent whatever the ruler unit.
        if unit != Unit::Percent {
            assert!(h.query_all_by_label("200.0%").next().is_none(), "{name}: no percent readout");
        }
        if let Some(dir) = std::env::var_os("PHOTOCRAFT_TRANSFORM_READOUT_EVIDENCE_DIR") {
            let dir = std::path::PathBuf::from(dir);
            std::fs::create_dir_all(&dir).expect("evidence directory");
            h.render().expect("render").save(dir.join(format!("transform-readout-{name}.png"))).expect("save evidence");
        }
        press(&mut h, to, false);
        h.key_press(egui::Key::Escape);
        h.run_steps(3);
    }
}
