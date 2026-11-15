//! Free Transform drag stalls (#11103).
//!
//! - Start: an open box moves from the first pointer move after the press, not once egui
//!   recognises a drag (past its click distance), so it never jumps to catch up. Clicks keep their
//!   rules: a click on the box moves nothing, a jittery click outside it on the image leaves it as
//!   it was, a double-click on it commits.
//! - Mip levels: a level first needed mid-drag is built off the UI thread while the closest finer
//!   level already uploaded stands in for it.

use std::time::{Duration, Instant};

use egui::{Color32, ColorImage, Modifiers, PointerButton, Pos2, vec2};
use egui_kittest::Harness;
use photocraft_doc::LayerId;
use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::ViewXform;
use crate::state::Tool;
use crate::transform_tex::PreviewTextures;

/// An 800 × 600 document at 100 % with a red block (x 100..300, y 100..300) on layer "Paint" and
/// Free Transform open on it.
fn harness() -> Harness<'static, PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 800, "height": 600, "background": "white"})).unwrap();
    app.sync_views();
    app.ui.extras.rulers = false;
    app.ui.extras.snap = false;
    app.ui.tool = Tool::Move;
    let paint = LayerId(app.run("layer.new.layer", json!({"name": "Paint"})).unwrap()["layer"].as_u64().unwrap());
    app.run("layer.select", json!({"layer": paint.0})).unwrap();
    app.run("select.rect", json!({"x": 100, "y": 100, "width": 200, "height": 200})).unwrap();
    app.run("edit.fill", json!({"contents": "color", "color": "#ff0000"})).unwrap();
    app.run("select.deselect", json!({})).unwrap();
    // Frames 1/60 s apart, so two clicks make a double-click.
    let mut h = Harness::builder().with_size(vec2(1100.0, 800.0)).with_step_dt(1.0 / 60.0).build_ui_state(
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
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    h.run_steps(4);
    let v = &mut h.state_mut().ui.views[0];
    v.zoom = 1.0;
    v.center = [400.0, 300.0];
    v.fit_pending = false;
    h.run_steps(2);
    let ctx = h.ctx.clone();
    crate::menus::invoke(h.state_mut(), &ctx, "edit.freeTransform", json!({})).unwrap();
    h.run_steps(2);
    assert!(h.state().ui.transform.is_some(), "Free Transform is open");
    h
}

fn view_xform(h: &Harness<'static, PhotocraftApp>) -> ViewXform {
    let app = h.state();
    let v = &app.ui.views[0];
    ViewXform {
        rect: crate::rulers::content_rect(app, app.last_canvas_rect),
        zoom: v.zoom,
        center: v.center,
        flip: app.ui.view.flip_horizontal,
        rotation: v.rotation,
        aspect: app.ui.view.display_aspect(),
    }
}

fn screen(h: &Harness<'static, PhotocraftApp>, x: f32, y: f32) -> Pos2 {
    view_xform(h).to_screen(x, y)
}

fn quad(h: &Harness<'static, PhotocraftApp>) -> [[f64; 2]; 4] {
    h.state().ui.transform.as_ref().expect("the box is open").quad
}

fn steps(h: &Harness<'static, PhotocraftApp>) -> usize {
    h.state().session.active().unwrap().history.past_len()
}

fn button(h: &mut Harness<'static, PhotocraftApp>, p: Pos2, down: bool) {
    h.event(egui::Event::PointerButton { pos: p, button: PointerButton::Primary, pressed: down, modifiers: Modifiers::NONE });
    h.run_steps(1);
}

/// Hover document point `(x, y)`, then press there.
fn press(h: &mut Harness<'static, PhotocraftApp>, x: f32, y: f32) {
    let p = screen(h, x, y);
    h.event(egui::Event::PointerMoved(p));
    h.run_steps(1);
    button(h, p, true);
}

/// One frame with the pointer at document point `(x, y)`.
fn move_to(h: &mut Harness<'static, PhotocraftApp>, x: f32, y: f32) {
    let p = screen(h, x, y);
    h.event(egui::Event::PointerMoved(p));
    h.run_steps(1);
}

fn release(h: &mut Harness<'static, PhotocraftApp>, x: f32, y: f32) {
    let p = screen(h, x, y);
    button(h, p, false);
    h.run_steps(1);
}

/// How far the box's top-left corner is from `from`.
fn offset(h: &Harness<'static, PhotocraftApp>, from: [[f64; 2]; 4]) -> [f64; 2] {
    let q = quad(h);
    [q[0][0] - from[0][0], q[0][1] - from[0][1]]
}

fn near(a: [f64; 2], b: [f64; 2]) -> bool {
    (a[0] - b[0]).abs() < 1e-3 && (a[1] - b[1]).abs() < 1e-3
}

#[test]
fn the_box_moves_from_the_first_pointer_move_after_the_press() {
    let mut h = harness();
    let start = quad(&h);
    press(&mut h, 200.0, 200.0);
    assert!(near(offset(&h, start), [0.0, 0.0]), "the press alone moves nothing: {:?}", offset(&h, start));
    // 3 px: inside egui's click distance, so no drag is recognised yet.
    move_to(&mut h, 203.0, 200.0);
    assert!(near(offset(&h, start), [3.0, 0.0]), "the first move moves the box: {:?}", offset(&h, start));
    move_to(&mut h, 240.0, 220.0);
    assert!(near(offset(&h, start), [40.0, 20.0]), "{:?}", offset(&h, start));
    release(&mut h, 240.0, 220.0);
    assert!(h.state().ui.transform.is_some(), "a drag doesn't close the box");
    assert!(near(offset(&h, start), [40.0, 20.0]), "{:?}", offset(&h, start));
}

#[test]
fn the_box_keeps_up_with_every_move_with_no_jump_when_the_drag_is_recognised() {
    let mut h = harness();
    let start = quad(&h);
    press(&mut h, 200.0, 200.0);
    // One pixel a frame, through egui's click distance and past it.
    for dx in 1..=16 {
        move_to(&mut h, 200.0 + dx as f32, 200.0);
        let got = offset(&h, start);
        assert!(near(got, [f64::from(dx), 0.0]), "after a {dx} px move the box is {got:?} off");
    }
    release(&mut h, 216.0, 200.0);
    assert!(near(offset(&h, start), [16.0, 0.0]), "{:?}", offset(&h, start));
}

#[test]
fn a_click_on_the_box_moves_nothing_and_keeps_it_open() {
    let mut h = harness();
    let (start, before) = (quad(&h), steps(&h));
    press(&mut h, 200.0, 200.0);
    release(&mut h, 200.0, 200.0);
    assert!(h.state().ui.transform.is_some());
    assert_eq!(quad(&h), start);
    assert_eq!(steps(&h), before);
}

#[test]
fn a_jittery_click_outside_the_box_on_the_image_leaves_it_as_it_was() {
    let mut h = harness();
    let (start, pivot) = (quad(&h), h.state().ui.transform.as_ref().unwrap().pivot);
    // Only the Background has pixels here (#11100): the box stays open.
    press(&mut h, 600.0, 450.0);
    move_to(&mut h, 601.0, 450.0);
    release(&mut h, 601.0, 450.0);
    let t = h.state().ui.transform.as_ref().expect("the box stays open");
    assert_eq!((t.quad, t.pivot), (start, pivot), "the jitter's turn is put back");
}

#[test]
fn a_double_click_on_the_box_still_commits() {
    let mut h = harness();
    // Moved 10 px first: committing an unchanged box adds no History step.
    let t = h.state_mut().ui.transform.as_mut().unwrap();
    t.quad = t.quad.map(|[x, y]| [x + 10.0, y]);
    t.pivot[0] += 10.0;
    h.run_steps(1);
    let before = steps(&h);
    press(&mut h, 200.0, 200.0);
    let p = screen(&h, 200.0, 200.0);
    button(&mut h, p, false);
    button(&mut h, p, true);
    button(&mut h, p, false);
    h.run_steps(2);
    assert!(h.state().ui.transform.is_none(), "committed");
    assert_eq!(steps(&h), before + 1);
}

/// Waits until `t` has no mip level building off the UI thread.
fn wait_for_builds(t: &PreviewTextures) {
    let start = Instant::now();
    while t.is_building() {
        assert!(start.elapsed() < Duration::from_secs(10), "a mip level never finished building");
        std::thread::sleep(Duration::from_millis(2));
    }
}

/// A 5 × 3 white image with background builds on: level 1 is 3 × 2 texels covering 6 × 4 base
/// texels, level 3 is 1 × 1 covering 8 × 8.
fn textures(ctx: &egui::Context) -> PreviewTextures {
    let mut t = PreviewTextures::new(ctx, "drag-mips".into(), ColorImage::new([5, 3], vec![Color32::WHITE; 15]), [1.0, 1.0]);
    t.set_background_builds(true);
    t
}

#[test]
fn a_mip_level_first_needed_mid_drag_is_built_off_the_ui_thread() {
    let ctx = egui::Context::default();
    let t = textures(&ctx);
    let (full, full_uv) = t.pick(&ctx, 1.0, 1.0, false);
    // Shrunk past ½: level 1 is wanted, level 0 shows until it is ready.
    let (shown, uv) = t.pick(&ctx, 0.3, 1.0, false);
    assert_eq!((shown, uv), (full, full_uv), "the full-resolution level stands in");
    wait_for_builds(&t);
    let (built, uv) = t.pick(&ctx, 0.3, 1.0, false);
    assert_ne!(built, full, "level 1 has its own texture once built");
    assert!((uv[0] - 5.0 / 6.0).abs() < 1e-6 && (uv[1] - 3.0 / 4.0).abs() < 1e-6, "{uv:?}");
    assert_eq!(t.pick(&ctx, 0.3, 1.0, false).0, built, "cached");
    ctx.tex_manager().write().take_delta().clear();
}

#[test]
fn the_closest_finer_level_stands_in_while_a_level_builds() {
    let ctx = egui::Context::default();
    let t = textures(&ctx);
    let full = t.pick(&ctx, 1.0, 1.0, false).0;
    let _ = t.pick(&ctx, 0.3, 1.0, false);
    wait_for_builds(&t);
    let level1 = t.pick(&ctx, 0.3, 1.0, false).0;
    // log2(1 / 0.1) = 3.3: level 3 is wanted; level 1 is the closest one uploaded.
    assert_eq!(t.pick(&ctx, 0.1, 1.0, false).0, level1);
    wait_for_builds(&t);
    let level3 = t.pick(&ctx, 0.1, 1.0, false).0;
    assert!(level3 != level1 && level3 != full, "level 3 has its own texture once built");
    ctx.tex_manager().write().take_delta().clear();
}

#[test]
fn a_level_with_nothing_finer_uploaded_for_the_display_is_built_at_once() {
    let mut session = photocraft_engine::Session::new();
    session.execute("file.new", json!({"width": 2, "height": 1})).unwrap();
    let doc = session.active().unwrap().doc.clone();
    session.color.settings.monitor_profile = "display-p3".into();
    let p3 = session.color.canvas_display(&doc).unwrap();
    assert!(!p3.is_identity());
    let ctx = egui::Context::default();
    let t = textures(&ctx);
    let full = t.pick(&ctx, 1.0, 1.0, false).0;
    // Only the unconverted level 0 is uploaded: nothing can stand in for this monitor.
    let (shown, uv) = t.pick_display(&ctx, 0.3, 1.0, false, Some(&p3));
    assert!(!t.is_building(), "built in this frame");
    assert_ne!(shown, full);
    assert!((uv[0] - 5.0 / 6.0).abs() < 1e-6 && (uv[1] - 3.0 / 4.0).abs() < 1e-6, "{uv:?}");
    ctx.tex_manager().write().take_delta().clear();
}

/// A box closed while two levels were still building: neither worker converts or uploads its
/// level, though each still holds the preview's shared state (#13382 review). Counted as uploads,
/// not as textures left, which an upload freed again at once would also pass.
#[test]
fn levels_finishing_after_the_box_closed_are_not_uploaded() {
    let ctx = egui::Context::default();
    let t = textures(&ctx);
    let _ = t.pick(&ctx, 1.0, 1.0, false);
    let (gate, done, uploads) = t.worker_hooks();
    let held = gate.lock().unwrap();
    // Levels 1 and 3, each on its own worker, both paused before they check the box is open.
    let _ = t.pick(&ctx, 0.3, 1.0, false);
    let _ = t.pick(&ctx, 0.1, 1.0, false);
    drop(t);
    drop(held);
    let start = Instant::now();
    while !done() {
        assert!(start.elapsed() < Duration::from_secs(10), "the workers finish");
        std::thread::sleep(Duration::from_millis(2));
    }
    assert_eq!(uploads.load(std::sync::atomic::Ordering::Relaxed), 0, "nothing uploaded for the closed box");
    ctx.tex_manager().write().take_delta().clear();
}

/// The same two workers with the box still open: both upload, so the count above means something.
#[test]
fn levels_finishing_while_the_box_is_open_are_uploaded() {
    let ctx = egui::Context::default();
    let t = textures(&ctx);
    let _ = t.pick(&ctx, 1.0, 1.0, false);
    let (_, done, uploads) = t.worker_hooks();
    let _ = t.pick(&ctx, 0.3, 1.0, false);
    let _ = t.pick(&ctx, 0.1, 1.0, false);
    let start = Instant::now();
    while !done() {
        assert!(start.elapsed() < Duration::from_secs(10), "the workers finish");
        std::thread::sleep(Duration::from_millis(2));
    }
    assert_eq!(uploads.load(std::sync::atomic::Ordering::Relaxed), 2);
    ctx.tex_manager().write().take_delta().clear();
}
