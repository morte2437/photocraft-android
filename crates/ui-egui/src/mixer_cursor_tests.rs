//! The Mixer Brush's cursors (#11957): with ⌥ held, the reference's sample target (a small circle
//! with a crosshair, hotspot at its centre) instead of the Eyedropper's pipette, and no comparison
//! ring while an ⌥-click is held; over a layer with Lock All, a no-entry sign. The Brush keeps its
//! ⌥ pipette. The cursors are OS bitmaps: their hotspot sits at the bitmap's centre, the target is
//! mirror-symmetric left to right and the no-entry sign is not (its bar runs corner to corner).

use egui::{Modifiers, PointerButton, Pos2, vec2};
use egui_kittest::Harness;
use serde_json::json;

use crate::PhotocraftApp;
use crate::state::Tool;

pub(super) fn harness(tool: Tool) -> Harness<'static, PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 400, "height": 300, "background": "white"})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app.sync_views();
    app.ui.extras.rulers = false;
    app.ui.tool = tool;
    crate::paint_mouse::sync_tool_brush(&mut app);
    let mut h = Harness::builder().with_size(vec2(900.0, 600.0)).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            let ctx = ui.ctx().clone();
            if !ctx.fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
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

pub(super) fn has_fonts(h: &Harness<'static, PhotocraftApp>) -> bool {
    h.ctx.fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into())))
}

pub(super) fn centre(h: &Harness<'static, PhotocraftApp>) -> Pos2 {
    h.state().last_canvas_rect.center()
}

pub(super) fn hover(h: &mut Harness<'static, PhotocraftApp>, m: Modifiers) {
    h.event(egui::Event::ModifiersChanged(m));
    let p = centre(h);
    h.hover_at(p);
    h.run_steps(2);
}

/// The Eyedropper's pipette: the OS bitmap where one uploads, else drawn (#12523).
fn pipette_shown(h: &Harness<'static, PhotocraftApp>) -> bool {
    crate::tool_cursor::pipette_shown(&h.ctx, &h.output().platform_output)
}

/// An OS cursor bitmap's size, hotspot and alpha channel.
pub(super) type Bitmap = ([u16; 2], [u16; 2], Vec<u8>);

/// The OS cursor bitmap's size, hotspot and alpha channel.
pub(super) fn bitmap(h: &Harness<'static, PhotocraftApp>) -> Option<Bitmap> {
    h.output().platform_output.cursor_image.as_ref().map(|i| (i.size, i.hotspot, i.rgba.as_chunks::<4>().0.iter().map(|p| p[3]).collect()))
}

/// Whether a line passes through the hotspot: its pixel or the one to its left, as a 1 px line
/// centred on the hotspot may land on either at an even scale.
pub(super) fn hotspot_drawn((size, hotspot, alpha): &Bitmap) -> bool {
    let (w, hx, hy) = (usize::from(size[0]), usize::from(hotspot[0]), usize::from(hotspot[1]));
    alpha[hy * w + hx] > 0 || alpha[hy * w + hx.saturating_sub(1)] > 0
}

/// The share of pixels whose alpha differs (by more than a quarter) from the pixel `mirror` maps
/// them to.
fn asymmetry(size: [u16; 2], alpha: &[u8], mirror: impl Fn(usize, usize) -> (usize, usize)) -> f32 {
    let (w, h) = (usize::from(size[0]), usize::from(size[1]));
    let mut off = 0;
    for y in 0..h {
        for x in 0..w {
            let (mx, my) = mirror(x, y);
            if alpha[y * w + x].abs_diff(alpha[my * w + mx]) > 64 {
                off += 1;
            }
        }
    }
    off as f32 / (w * h) as f32
}

/// Whether the Eyedropper's comparison ring is painted: its circles have the ring's 17 pt band.
fn ring_drawn(h: &Harness<'static, PhotocraftApp>) -> bool {
    fn ring(shape: &egui::Shape) -> bool {
        match shape {
            egui::Shape::Circle(c) => (c.stroke.width - 17.0).abs() < 0.5 && c.radius > 50.0,
            egui::Shape::Vec(shapes) => shapes.iter().any(ring),
            _ => false,
        }
    }
    h.output().shapes.iter().any(|c| ring(&c.shape))
}

pub(super) fn assert_centred(size: [u16; 2], hotspot: [u16; 2], what: &str) {
    for k in 0..2 {
        assert!((f32::from(hotspot[k]) - f32::from(size[k]) / 2.0).abs() <= 1.0, "{what}: hotspot {hotspot:?} in a {size:?} bitmap");
    }
}

#[test]
fn alt_shows_the_sample_target_and_no_ring_while_held() {
    let mut h = harness(Tool::MixerBrush);
    if !has_fonts(&h) {
        return;
    }
    hover(&mut h, Modifiers::ALT);
    assert!(!pipette_shown(&h), "no pipette");
    let target = bitmap(&h).expect("the sample target is an OS cursor bitmap");
    let (size, hotspot, alpha) = &target;
    assert_centred(*size, *hotspot, "sample target");
    let w = usize::from(size[0]);
    assert!(asymmetry(*size, alpha, |x, y| (w - 1 - x, y)) < 0.02, "the target is symmetric left to right");
    assert!(hotspot_drawn(&target), "the crosshair passes through the hotspot");
    let target = Some(target);
    // An ⌥-click held: still the target, no comparison ring.
    let p = centre(&h);
    h.event(egui::Event::PointerButton { pos: p, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::ALT });
    h.run_steps(2);
    assert!(h.state().mixer_sampling, "the ⌥-click is held and loads");
    assert_eq!(bitmap(&h), target, "the target stays while the ⌥-click is held");
    assert!(!pipette_shown(&h));
    assert!(!ring_drawn(&h), "no comparison ring");
    h.event(egui::Event::PointerButton { pos: p, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::ALT });
    h.run_steps(2);
}

#[test]
fn a_locked_layer_shows_the_no_entry_sign() {
    let mut h = harness(Tool::MixerBrush);
    if !has_fonts(&h) {
        return;
    }
    hover(&mut h, Modifiers::NONE);
    let tip = bitmap(&h);
    h.state_mut().run("layer.lockLayers", json!({"all": true})).unwrap();
    hover(&mut h, Modifiers::NONE);
    let (size, hotspot, alpha) = bitmap(&h).expect("the no-entry sign is an OS cursor bitmap");
    assert_ne!(Some((size, hotspot, alpha.clone())), tip, "not the brush tip");
    assert_centred(size, hotspot, "no-entry sign");
    let w = usize::from(size[0]);
    assert!(asymmetry(size, &alpha, |x, y| (y, x)) < 0.02, "the bar runs corner to corner");
    assert!(asymmetry(size, &alpha, |x, y| (w - 1 - x, y)) > 0.02, "a sign, not the sample target");
    // Unlocked again: the brush tip.
    h.state_mut().run("layer.lockLayers", json!({"all": false})).unwrap();
    hover(&mut h, Modifiers::NONE);
    assert_eq!(bitmap(&h), tip);
}

#[test]
fn the_brush_keeps_its_alt_pipette() {
    let mut h = harness(Tool::Brush);
    if !has_fonts(&h) {
        return;
    }
    hover(&mut h, Modifiers::ALT);
    assert!(pipette_shown(&h), "the Brush's ⌥ is the Eyedropper's pipette");
}

/// The Brush's held ⌥-click still shows the Eyedropper's comparison ring around its pipette, so
/// the ring test above can see one.
#[test]
fn the_brush_shows_its_ring_while_an_alt_click_is_held() {
    let mut h = harness(Tool::Brush);
    if !has_fonts(&h) {
        return;
    }
    hover(&mut h, Modifiers::ALT);
    assert!(!ring_drawn(&h), "no ring on hover");
    let p = centre(&h);
    h.event(egui::Event::PointerButton { pos: p, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::ALT });
    h.run_steps(2);
    assert!(h.state().alt_sampling, "the Brush's ⌥-click samples");
    assert!(ring_drawn(&h), "the comparison ring");
    assert!(pipette_shown(&h));
    h.event(egui::Event::PointerButton { pos: p, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::ALT });
    h.run_steps(2);
}

/// Other Cursors = Precise wasn't measured with the Mixer Brush's ⌥, so it keeps today's precise
/// crosshair rather than the sample target.
#[test]
fn precise_other_cursors_keep_the_crosshair_with_alt() {
    let mut h = harness(Tool::MixerBrush);
    if !has_fonts(&h) {
        return;
    }
    hover(&mut h, Modifiers::ALT);
    let target = bitmap(&h).expect("the sample target");
    h.state_mut().run("prefs.set", json!({"values": {"cursors.other": "precise"}})).unwrap();
    hover(&mut h, Modifiers::ALT);
    assert_eq!(h.output().platform_output.cursor_icon, egui::CursorIcon::Crosshair);
    assert_ne!(bitmap(&h), Some(target), "the precise crosshair, not the sample target");
    assert!(!pipette_shown(&h));
}
