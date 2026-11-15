use egui::{Modifiers, PointerButton, Pos2, vec2};
use egui_kittest::Harness;
use serde_json::json;

use super::ViewXform;
use crate::PhotocraftApp;
use crate::state::Tool;

fn harness() -> Harness<'static, PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app.sync_views();
    app.ui.extras.rulers = false;
    app.ui.tool = Tool::ColorSampler;
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
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    h.run_steps(4);
    let v = &mut h.state_mut().ui.views[0];
    v.zoom = 1.0;
    v.center = [200.0, 150.0];
    v.fit_pending = false;
    h.run_steps(2);
    h
}

fn screen(h: &Harness<'static, PhotocraftApp>, x: f32, y: f32) -> Pos2 {
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
    .to_screen(x, y)
}

fn cursor_area_visible(h: &Harness<'static, PhotocraftApp>, id: &str) -> bool {
    let layer = egui::LayerId::new(egui::Order::Tooltip, egui::Id::unique(id));
    h.ctx.memory(|memory| memory.areas().visible_layer_ids().contains(&layer))
}

fn cursor_badge_visible(h: &Harness<'static, PhotocraftApp>, name: &str) -> bool {
    let layer = egui::LayerId::new(egui::Order::Tooltip, egui::Id::unique(("pc-cursor-badge", name)));
    h.ctx.memory(|memory| memory.areas().visible_layer_ids().contains(&layer))
}

fn hover_at(h: &mut Harness<'static, PhotocraftApp>, doc_x: f32, doc_y: f32, m: Modifiers) -> egui::CursorIcon {
    h.event(egui::Event::ModifiersChanged(m));
    let p = screen(h, doc_x, doc_y);
    h.event(egui::Event::PointerMoved(p));
    h.run_steps(2);
    h.output().platform_output.cursor_icon
}

/// The eyedropper's pipette: the OS bitmap where one uploads, else drawn (#12523).
fn pipette_shown(h: &Harness<'static, PhotocraftApp>) -> bool {
    crate::tool_cursor::pipette_shown(&h.ctx, &h.output().platform_output)
}

#[test]
fn off_sampler_no_key_or_shift_or_alt_shows_eyedropper_with_sampler_target() {
    let mut h = harness();

    // 1. Off a sampler, no key: eyedropper with small sampler target at top-left.
    hover_at(&mut h, 80.0, 80.0, Modifiers::NONE);
    assert!(pipette_shown(&h), "pipette cursor replaces OS pointer");
    assert!(cursor_area_visible(&h, "pc-sampler-badge"), "sampler target at top-left is drawn");
    assert!(!cursor_badge_visible(&h, "sampler") && !cursor_badge_visible(&h, "scissors"), "arrow badge is not drawn off sampler");

    // 2. Off a sampler, Shift held: still eyedropper with sampler target.
    hover_at(&mut h, 80.0, 80.0, Modifiers::SHIFT);
    assert!(pipette_shown(&h));
    assert!(cursor_area_visible(&h, "pc-sampler-badge"));

    // 3. Off a sampler, Alt held: still eyedropper with sampler target.
    hover_at(&mut h, 80.0, 80.0, Modifiers::ALT);
    assert!(pipette_shown(&h));
    assert!(cursor_area_visible(&h, "pc-sampler-badge"));
}

#[test]
fn off_sampler_precise_other_cursors_shows_crosshair() {
    let mut h = harness();
    h.state_mut().run("prefs.set", json!({"values": {"cursors.other": "precise"}})).unwrap();

    let icon = hover_at(&mut h, 80.0, 80.0, Modifiers::NONE);
    assert_eq!(icon, egui::CursorIcon::Crosshair, "Precise Other Cursors shows crosshair off sampler");
    assert!(!pipette_shown(&h), "no pipette with precise cursor");
    assert!(!cursor_area_visible(&h, "pc-sampler-badge"), "no sampler badge with precise cursor");
}

#[test]
fn over_sampler_shows_arrow_with_sampler_target_and_alt_shows_scissors() {
    let mut h = harness();
    // Place a sampler at (150.0, 150.0).
    h.state_mut().run("view.colorSamplers.add", json!({"x": 150.0, "y": 150.0})).unwrap();

    // 1. Over sampler, no key: arrow with sampler target.
    let icon = hover_at(&mut h, 150.0, 150.0, Modifiers::NONE);
    assert_eq!(icon, egui::CursorIcon::Default, "over sampler uses standard arrow pointer");
    assert!(cursor_badge_visible(&h, "sampler"), "sampler target badge is drawn");
    assert!(!cursor_badge_visible(&h, "scissors"), "scissors badge is not drawn without Alt");
    assert!(!pipette_shown(&h), "no pipette over sampler");

    // Even with Precise Other Cursors, over a sampler is the arrow cursor.
    h.state_mut().run("prefs.set", json!({"values": {"cursors.other": "precise"}})).unwrap();
    let icon_precise = hover_at(&mut h, 150.0, 150.0, Modifiers::NONE);
    assert_eq!(icon_precise, egui::CursorIcon::Default, "Precise cursor does not suppress sampler grab pointer");
    assert!(cursor_badge_visible(&h, "sampler"));
    assert!(!cursor_badge_visible(&h, "scissors"));
    h.state_mut().run("prefs.set", json!({"values": {"cursors.other": "standard"}})).unwrap();

    // 2. Alt held over sampler: arrow with scissors.
    let icon_alt = hover_at(&mut h, 150.0, 150.0, Modifiers::ALT);
    assert_eq!(icon_alt, egui::CursorIcon::Default, "Alt over sampler keeps arrow pointer");
    assert!(cursor_badge_visible(&h, "scissors"), "scissors badge is drawn with Alt");
    assert!(!cursor_badge_visible(&h, "sampler"), "sampler badge is not drawn when Alt shows scissors");

    // 3. Releasing Alt without moving pointer reverts to arrow with sampler target.
    h.event(egui::Event::ModifiersChanged(Modifiers::NONE));
    h.run_steps(2);
    assert_eq!(h.output().platform_output.cursor_icon, egui::CursorIcon::Default);
    assert!(cursor_badge_visible(&h, "sampler"), "sampler target badge restored when Alt is released");
    assert!(!cursor_badge_visible(&h, "scissors"), "scissors badge removed when Alt is released");

    // 4. Sticky Alt (e.g. tablet button) also shows scissors.
    h.state_mut().ui.shell.sticky_alt = true;
    h.run_steps(2);
    assert_eq!(h.output().platform_output.cursor_icon, egui::CursorIcon::Default);
    assert!(cursor_badge_visible(&h, "scissors"), "sticky Alt shows scissors");
    assert!(!cursor_badge_visible(&h, "sampler"));

    h.state_mut().ui.shell.sticky_alt = false;
    h.run_steps(2);
    assert!(cursor_badge_visible(&h, "sampler"));
    assert!(!cursor_badge_visible(&h, "scissors"));
}

#[test]
fn cursor_follows_sampler_hit_test_tolerance() {
    let mut h = harness();
    h.state_mut().run("view.colorSamplers.add", json!({"x": 150.0, "y": 150.0})).unwrap();

    // At zoom = 1.0, tolerance is 7.0 px.
    // Inside hit square (dx = 6.0 <= 7.0): over sampler -> CursorIcon::Default.
    let icon_in = hover_at(&mut h, 156.0, 150.0, Modifiers::NONE);
    assert_eq!(icon_in, egui::CursorIcon::Default, "dx = 6 px is within tolerance");
    assert!(cursor_badge_visible(&h, "sampler"));
    assert!(!cursor_badge_visible(&h, "scissors"));

    // Outside hit square (dx = 8.0 > 7.0): off sampler -> the eyedropper's pipette.
    hover_at(&mut h, 158.0, 150.0, Modifiers::NONE);
    assert!(pipette_shown(&h), "dx = 8 px is outside tolerance");
    assert!(cursor_area_visible(&h, "pc-sampler-badge"));

    // A press at dx = 6 grabs the existing sampler.
    let p_in = screen(&h, 156.0, 150.0);
    h.event(egui::Event::PointerMoved(p_in));
    h.run_steps(1);
    h.event(egui::Event::PointerButton { pos: p_in, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.run_steps(1);
    let q = screen(&h, 170.0, 150.0);
    h.event(egui::Event::PointerMoved(q));
    h.run_steps(1);
    h.event(egui::Event::PointerButton { pos: q, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(2);

    let samplers = h.state().session.active().unwrap().color_samplers.clone();
    assert_eq!(samplers.len(), 1, "dragging within tolerance moved the existing sampler");
    assert!((samplers[0][0] - 170.0).abs() < 1.0, "moved to 170");
}

#[test]
fn dragging_a_sampler_past_its_tolerance_keeps_the_sampler_cursor_until_release() {
    // #13063: the stored point stays where it was until release, so the hit test alone would show
    // the placement eyedropper once the pointer leaves the 7 px tolerance mid-drag.
    let mut h = harness();
    h.state_mut().run("view.colorSamplers.add", json!({"x": 150.0, "y": 150.0})).unwrap();

    let p = screen(&h, 150.0, 150.0);
    h.event(egui::Event::PointerMoved(p));
    h.run_steps(1);
    h.event(egui::Event::PointerButton { pos: p, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.run_steps(1);
    let q = screen(&h, 200.0, 150.0);
    h.event(egui::Event::PointerMoved(q));
    h.run_steps(2);
    assert_eq!(h.output().platform_output.cursor_icon, egui::CursorIcon::Default, "the drag keeps the arrow pointer");
    assert!(cursor_badge_visible(&h, "sampler"), "the drag keeps the sampler target badge");
    assert!(!pipette_shown(&h) && !cursor_area_visible(&h, "pc-sampler-badge"), "no placement eyedropper mid-drag");

    // Alt removes only on the press; the release still moves the point, so no scissors mid-drag.
    h.event(egui::Event::ModifiersChanged(Modifiers::ALT));
    h.run_steps(2);
    assert!(cursor_badge_visible(&h, "sampler"), "Alt mid-drag keeps the sampler target");
    assert!(!cursor_badge_visible(&h, "scissors"), "Alt mid-drag shows no scissors");
    h.event(egui::Event::ModifiersChanged(Modifiers::NONE));

    h.event(egui::Event::PointerButton { pos: q, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(2);
    let samplers = h.state().session.active().unwrap().color_samplers.clone();
    assert!((samplers[0][0] - 200.0).abs() < 1.0, "the release moved the sampler to 200");

    // After release, off a sampler is the placement eyedropper again.
    hover_at(&mut h, 100.0, 100.0, Modifiers::NONE);
    assert!(pipette_shown(&h) && cursor_area_visible(&h, "pc-sampler-badge"), "the placement eyedropper returns after release");
    assert!(!cursor_badge_visible(&h, "sampler"));
}
