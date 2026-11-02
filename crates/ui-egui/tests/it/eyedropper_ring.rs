//! #6973: the Eyedropper's sampling ring must read over a flat area of any colour, including one
//! that matches the ring itself. The ring is Photoshop's large one: an opaque grey outer band with
//! the sampled colour above and the current one below, framed by thin dark and light hairlines.
//! These tests render the real app offscreen and read pixels back. They skip when no GPU adapter
//! is available (like `checker_canvas.rs`).

use egui::{Modifiers, PointerButton, Pos2};
use photocraft_ui_egui::PhotocraftApp;
use photocraft_ui_egui::control::{ControlRequest, handle};
use serde_json::{Value, json};

type Harness = egui_kittest::Harness<'static, PhotocraftApp>;

/// The ring geometry `eyedropper_ring` draws: outer radius and band width, in points.
const R_OUT: i32 = 105;
const BAND: i32 = 17;
/// The flat mid-grey the document is filled with: exactly the ring's own grey, the worst case.
const BG: u8 = 0x7f;

fn harness() -> Option<Harness> {
    let built = std::panic::catch_unwind(|| {
        egui_kittest::Harness::builder().with_size(egui::vec2(1400.0, 800.0)).with_pixels_per_point(1.0).with_max_steps(64).wgpu().build_eframe(|cc| {
            PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
            let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
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

/// A 400 × 400 flat mid-grey document, the Eyedropper active and centred in the view.
fn setup(h: &mut Harness) -> Pos2 {
    {
        let app = h.state_mut();
        while app.session.active().is_some() {
            app.run("file.close", json!({"discard": true})).expect("close");
        }
        app.run("file.new", json!({"width": 400, "height": 400, "background": "white"})).expect("new");
        app.run("edit.fill", json!({"color": format!("#{BG:02x}{BG:02x}{BG:02x}")})).expect("fill");
        app.sync_views();
    }
    control(h, "ui.set", json!({"tool": "eyedropper", "zoom": 1.0, "center": [200.0, 200.0]}));
    h.run_steps(3);
    let r = h.state().last_canvas_rect;
    r.center()
}

fn pointer_at(h: &mut Harness, p: Pos2) {
    h.event(egui::Event::PointerMoved(p));
    h.step();
}

fn button_at(h: &mut Harness, p: Pos2, pressed: bool) {
    h.event(egui::Event::PointerButton { pos: p, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
    h.step();
}

/// A rendered screen.
struct Shot {
    w: u32,
    px: Vec<u8>,
}

impl Shot {
    fn at(&self, x: i32, y: i32) -> [u8; 4] {
        let i = ((y.max(0) as u32 * self.w + x.max(0) as u32) * 4) as usize;
        [self.px[i], self.px[i + 1], self.px[i + 2], self.px[i + 3]]
    }
}

fn shot(h: &mut Harness) -> Shot {
    let rendered = h.render().expect("render");
    Shot { w: rendered.width(), px: rendered.into_raw() }
}

/// The largest channel difference from the flat document grey.
fn off_bg(p: [u8; 4]) -> i32 {
    [p[0], p[1], p[2]].iter().map(|v| (*v as i32 - BG as i32).abs()).max().unwrap_or(0)
}

/// Walking out from the centre along `-y`, the first pixel above the ring's outer edge that
/// differs from the background: the ring's outer hairline. `None` when nothing does within the
/// band's neighbourhood, i.e. the ring is invisible over the flat grey.
fn outer_edge_reads(img: &Shot, c: Pos2) -> bool {
    let (cx, cy) = (c.x as i32, c.y as i32);
    // The hairlines sit at r_out + 0.5 and + 1.5 px; scan a little either side of them.
    (R_OUT - 2..R_OUT + 4).any(|d| off_bg(img.at(cx, cy - d)) >= 30)
}

#[test]
fn ring_reads_over_a_flat_area_of_its_own_colour() {
    let _gpu = photocraft_testkit::gpu_lock();
    let Some(mut h) = harness() else { return };
    h.run_steps(4);
    let c = setup(&mut h);
    pointer_at(&mut h, c);
    button_at(&mut h, c, true);
    h.run_steps(2);
    let img = shot(&mut h);
    // The old 22 pt ring left nothing at this radius; the hairlines must not.
    assert!(outer_edge_reads(&img, c), "the ring's outer edge is invisible over a flat mid-grey");
    // And the inner edge, where the transparent centre meets the coloured band.
    let r_in = R_OUT - 2 * BAND;
    let (cx, cy) = (c.x as i32, c.y as i32);
    assert!((r_in - 3..r_in + 2).any(|d| off_bg(img.at(cx, cy - d)) >= 30), "the ring's inner edge is invisible over a flat mid-grey");
    // The pipette stays visible at the centre, marking the sampled pixel.
    let pipette = (-12..12).any(|dy| (-12..12).any(|dx| off_bg(img.at(cx + dx, cy + dy)) >= 30));
    assert!(pipette, "no pipette drawn at the ring's centre");
    button_at(&mut h, c, false);
}

#[test]
fn show_sampling_ring_off_draws_nothing() {
    let _gpu = photocraft_testkit::gpu_lock();
    let Some(mut h) = harness() else { return };
    h.run_steps(4);
    let c = setup(&mut h);
    control(&mut h, "ui.set", json!({"eyedropperRing": false}));
    pointer_at(&mut h, c);
    button_at(&mut h, c, true);
    h.run_steps(2);
    let img = shot(&mut h);
    let (cx, cy) = (c.x as i32, c.y as i32);
    // Nothing of the ring: the whole ring neighbourhood is the flat document grey, apart from the
    // pipette cursor the tool draws at the pointer anyway.
    let quiet = (R_OUT - 2..R_OUT + 4).all(|d| off_bg(img.at(cx, cy - d)) < 30) && (R_OUT - 2..R_OUT + 4).all(|d| off_bg(img.at(cx, cy + d)) < 30);
    assert!(quiet, "the ring is drawn although Show Sampling Ring is off");
    button_at(&mut h, c, false);
}
