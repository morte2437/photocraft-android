//! Caps Lock swaps the painting tools' cursor to the precise crosshair, exactly like Photoshop,
//! whatever the Cursors › Painting preference is (#1758). The override changes only painting
//! tools (Brush, Pencil, Clone Stamp, Healing, Eraser, …): Move, Marquee, Crop and Zoom keep
//! their own cursors. Tests drive the real canvas (`document_area`) and compare the painted
//! cursor that caps lock produces with the one the Precise preference draws — they must match,
//! and the Standard preference (off) must not. The tools under Cursors › Other Cursors with a
//! cursor of their own (the pens, Eyedropper, Color Sampler, Paint Bucket) swap it for the
//! crosshair and back with caps lock (#4639).

use std::sync::Arc;

use egui::CursorIcon;
use egui_kittest::Harness;
use serde_json::json;

use crate::PhotocraftApp;
use crate::state::Tool;

fn capslock_harness() -> Harness<'static, PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app.run("tools.setBrush", json!({"brush": {"size": 40, "hardness": 1.0}})).unwrap();
    app.sync_views();
    app.ui.extras.rulers = false;
    app.ui.tool = Tool::Brush;
    let mut h = Harness::builder().with_size(egui::vec2(900.0, 600.0)).build_ui_state(
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
    // 100 %: one document pixel per point; the pointer hovers the canvas centre.
    let v = &mut h.state_mut().ui.views[0];
    v.zoom = 1.0;
    v.center = [200.0, 150.0];
    v.fit_pending = false;
    h.run_steps(2);
    let p = h.state().last_canvas_rect.center();
    h.event(egui::Event::PointerMoved(p));
    h.run_steps(3);
    h
}

#[derive(Debug, PartialEq)]
struct CursorImageFp {
    size: [u16; 2],
    hotspot: [u16; 2],
    rgba: Arc<[u8]>,
}

/// Everything the cursor path can change in one frame: the cursor icon, the OS cursor bitmap
/// (Windows/macOS/Linux) and the shapes painted on the canvas (the tool-cursor path paints the crosshair
/// and the brush tip outline). Comparison across two preference/caps-lock states of the same
/// harness is platform-independent.
#[derive(Debug, PartialEq)]
struct CursorFp {
    icon: CursorIcon,
    image: Option<CursorImageFp>,
    shapes: Vec<egui::epaint::ClippedShape>,
}

fn fp(h: &Harness<'_, PhotocraftApp>) -> CursorFp {
    let out = h.output();
    CursorFp {
        icon: out.platform_output.cursor_icon,
        image: out.platform_output.cursor_image.as_ref().map(|i| CursorImageFp { size: i.size, hotspot: i.hotspot, rgba: Arc::clone(&i.rgba) }),
        shapes: out.shapes.clone(),
    }
}

/// The eyedropper's pipette: the OS bitmap where one uploads, else drawn (#12523).
fn pipette_shown(h: &Harness<'_, PhotocraftApp>) -> bool {
    crate::tool_cursor::pipette_shown(&h.ctx, &h.output().platform_output)
}

fn painting(h: &mut Harness<'_, PhotocraftApp>, value: &str) {
    h.state_mut().run("prefs.set", json!({"path": "cursors.painting", "value": value})).unwrap();
    h.run_steps(2);
}

/// Caps Lock is read from the OS by the app shell; tests toggle the per-frame flag directly.
fn caps(h: &mut Harness<'_, PhotocraftApp>, on: bool) {
    h.state_mut().caps_lock = on;
    h.run_steps(2);
}

#[test]
fn caps_lock_forces_the_precise_crosshair_for_painting_tools_whatever_the_preference() {
    for tool in [
        Tool::Brush,
        Tool::Pencil,
        Tool::CloneStamp,
        Tool::Healing,
        Tool::Eraser,
        // The block that honours the painting preference also covers the Quick Selection.
        Tool::QuickSelection,
    ] {
        let mut h = capslock_harness();
        // Each tool keeps its own brush (#218), so switch the tool before painting.
        h.state_mut().ui.tool = tool;
        h.run_steps(2);
        painting(&mut h, "standard");
        let off = fp(&h);
        painting(&mut h, "precise");
        let precise = fp(&h);
        painting(&mut h, "standard");
        caps(&mut h, true);
        let on = fp(&h);
        assert_ne!(off, precise, "{tool:?}: the Standard and Precise cursors must differ");
        assert_eq!(on, precise, "{tool:?}: caps lock must draw the same crosshair as the Precise preference");
        assert_ne!(on, off, "{tool:?}: caps lock must not keep the Standard cursor");
        caps(&mut h, false);
        assert_eq!(fp(&h), off, "{tool:?}: releasing caps lock restores the preference cursor");
    }
}

#[test]
fn caps_lock_leaves_non_painting_tools_cursors_alone() {
    for tool in [Tool::Move, Tool::RectMarquee, Tool::EllipseMarquee, Tool::Crop, Tool::Zoom, Tool::Hand] {
        let mut h = capslock_harness();
        h.state_mut().ui.tool = tool;
        h.run_steps(2);
        let off = fp(&h);
        caps(&mut h, true);
        let on = fp(&h);
        assert_eq!(on, off, "{tool:?}: caps lock must not change a non-painting cursor");
        caps(&mut h, false);
    }
}

fn other(h: &mut Harness<'_, PhotocraftApp>, value: &str) {
    h.state_mut().run("prefs.set", json!({"path": "cursors.other", "value": value})).unwrap();
    h.run_steps(2);
}

/// #4639: the pen tools show their own icon under Standard Other Cursors and the crosshair
/// under Precise, and caps lock swaps one for the other either way, as in Photoshop. The
/// Eyedropper, Color Sampler and Paint Bucket, which already followed the preference, swap too.
#[test]
fn caps_lock_swaps_the_other_cursors_between_their_icon_and_the_crosshair() {
    for tool in [
        Tool::Pen,
        Tool::FreeformPen,
        Tool::CurvaturePen,
        // Add Anchor Point shows the pen only over a segment (#12729): `add_anchor_cursor_tests`.
        Tool::DeleteAnchorPoint,
        Tool::ConvertPoint,
        Tool::Eyedropper,
        Tool::ColorSampler,
        Tool::PaintBucket,
    ] {
        let mut h = capslock_harness();
        h.state_mut().ui.tool = tool;
        h.run_steps(2);
        other(&mut h, "standard");
        let standard = fp(&h);
        if let Some(variant) = crate::canvas::tool_cursor::PenVariant::from_tool(tool) {
            // #9122: Pen tools upload an OS cursor bitmap, while CursorLifecycle leaves a
            // Crosshair fallback in platform output in case the OS compositor cannot upload the bitmap.
            assert_eq!(standard.icon, CursorIcon::Crosshair, "{tool:?}: fallback crosshair is left behind native cursor bitmap");
            let img = standard.image.as_ref().expect("native cursor bitmap set for Pen");
            let expected = crate::canvas::tool_cursor::rasterize_pen(variant, 1.0).expect("variant rasterizes at scale 1.0");
            assert_eq!(img.size, expected.size, "{tool:?}: bitmap dimensions match rasterized variant");
            assert_eq!(img.hotspot, expected.hotspot, "{tool:?}: hotspot matches variant");
            assert_eq!(img.rgba, expected.rgba, "{tool:?}: RGBA bitmap matches variant rasterization");
        } else if matches!(tool, Tool::PaintBucket | Tool::Eyedropper | Tool::ColorSampler) && cfg!(not(target_arch = "wasm32")) {
            assert_eq!(standard.icon, CursorIcon::Crosshair, "{tool:?}: the OS bitmap has a visible fallback");
            assert!(standard.image.is_some(), "{tool:?}: the standard cursor is the OS bitmap");
        } else {
            assert_eq!(standard.icon, CursorIcon::None, "{tool:?}: the standard cursor is the tool's own icon, drawn");
            assert!(standard.image.is_none(), "{tool:?}: painted fallback has no native cursor bitmap");
        }
        other(&mut h, "precise");
        let precise = fp(&h);
        assert_ne!(standard, precise, "{tool:?}: Standard and Precise differ");
        assert_eq!(precise.icon, CursorIcon::Crosshair, "{tool:?}: Precise cursor is crosshair");
        assert!(precise.image.is_some(), "{tool:?}: Precise cursor uses the native crosshair bitmap");
        assert_ne!(precise.image, standard.image, "{tool:?}: Precise bitmap differs from the standard tool icon");

        caps(&mut h, true);
        assert_eq!(fp(&h), standard, "{tool:?}: caps lock turns the precise crosshair back into the icon");
        other(&mut h, "standard");
        assert_eq!(fp(&h), precise, "{tool:?}: caps lock turns the icon into the precise crosshair");
        caps(&mut h, false);
        assert_eq!(fp(&h), standard, "{tool:?}: releasing caps lock restores the icon");
    }
}

/// #9836: an armed Levels or Curves eyedropper (and the dialogs' other pickers) shows its own
/// cursor alone, not with the tool's drawn over it, and takes caps lock's precise crosshair as
/// well, as in Photoshop, instead of only following the preference.
#[test]
fn caps_lock_gives_the_curves_eyedroppers_the_precise_crosshair() {
    let mut h = capslock_harness();
    // A tool whose own cursor ignores caps lock, so only the eyedropper can answer it.
    h.state_mut().ui.tool = Tool::Move;
    let id = crate::adjust_dialog::open(h.state_mut(), "image.adjustments.curves").unwrap();
    let base = h.state_mut().ui.dialog_mut(id).map(|d| crate::filter_dialog::params_of(&d.fields)).unwrap();
    let dialog = h.state_mut().ui.dialog_mut(id).unwrap();
    dialog.fields.insert("__curvePicker".into(), json!("black"));
    dialog.fields.insert("__curvePickerBase".into(), base);
    assert!(crate::adjust_dialog::picker_armed(h.state()), "the black-point eyedropper is armed");
    h.run_steps(3);
    other(&mut h, "standard");
    let standard = fp(&h);
    assert!(pipette_shown(&h), "the pipette, not the Move tool's arrow over it");
    other(&mut h, "precise");
    let precise = fp(&h);
    assert_eq!(precise.icon, CursorIcon::Crosshair, "Precise: the crosshair");
    // CodeRabbit on #10406: the pipette is shown (drawn, or the OS bitmap), not only the icon hidden.
    assert_ne!(standard, precise, "Standard shows a cursor of its own where Precise shows the crosshair");
    other(&mut h, "standard");
    caps(&mut h, true);
    assert_eq!(fp(&h), precise, "caps lock turns the pipette into the precise crosshair");
    other(&mut h, "precise");
    assert_eq!(fp(&h), standard, "and the precise crosshair back into the pipette");
    caps(&mut h, false);
    assert_eq!(fp(&h), precise, "releasing caps lock restores the preference");
}

/// #9836: the same for a Properties Levels or Curves eyedropper (layer_picker.rs).
#[test]
fn caps_lock_gives_the_properties_eyedroppers_the_precise_crosshair() {
    let mut h = capslock_harness();
    h.state_mut().ui.tool = Tool::Move;
    h.state_mut().run("layer.newAdjustmentLayer.curves", json!({})).unwrap();
    let layer = h.state().session.active().unwrap().active_layer.unwrap();
    h.state_mut().ui.layer_picker = Some(crate::layer_picker::LayerPicker { layer: layer.0, point: "black".into(), tool: Tool::Move });
    assert!(crate::layer_picker::armed(h.state()).is_some(), "the Properties black-point eyedropper is armed");
    h.run_steps(3);
    other(&mut h, "standard");
    let standard = fp(&h);
    assert!(pipette_shown(&h), "the pipette, not the Move tool's arrow over it");
    other(&mut h, "precise");
    let precise = fp(&h);
    assert_eq!(precise.icon, CursorIcon::Crosshair, "Precise: the crosshair");
    // CodeRabbit on #10406: the pipette is shown (drawn, or the OS bitmap), not only the icon hidden.
    assert_ne!(standard, precise, "Standard shows a cursor of its own where Precise shows the crosshair");
    other(&mut h, "standard");
    caps(&mut h, true);
    assert_eq!(fp(&h), precise, "caps lock turns the pipette into the precise crosshair");
    caps(&mut h, false);
    assert_eq!(fp(&h), standard, "releasing caps lock restores the pipette");
}

/// #9836 review: the Color Picker's own eyedropper is the third dialog state that owns the image,
/// beside Levels/Curves and Properties, and it answers caps lock the same way.
#[test]
fn caps_lock_gives_the_color_pickers_eyedropper_the_precise_crosshair() {
    let mut h = capslock_harness();
    // A tool whose own cursor ignores caps lock, so only the eyedropper can answer it.
    h.state_mut().ui.tool = Tool::Move;
    crate::color_picker_ui::open(h.state_mut(), "foreground");
    assert!(crate::color_picker_ui::top(h.state()).is_some(), "the Color Picker owns the image");
    h.run_steps(3);
    other(&mut h, "standard");
    let standard = fp(&h);
    assert!(pipette_shown(&h), "the pipette, not the Move tool's arrow over it");
    other(&mut h, "precise");
    let precise = fp(&h);
    assert_eq!(precise.icon, CursorIcon::Crosshair, "Precise: the crosshair");
    // CodeRabbit on #10406: the pipette is shown (drawn, or the OS bitmap), not only the icon hidden.
    assert_ne!(standard, precise, "Standard shows a cursor of its own where Precise shows the crosshair");
    other(&mut h, "standard");
    caps(&mut h, true);
    assert_eq!(fp(&h), precise, "caps lock turns the pipette into the precise crosshair");
    other(&mut h, "precise");
    assert_eq!(fp(&h), standard, "and the precise crosshair back into the pipette");
    caps(&mut h, false);
    assert_eq!(fp(&h), precise, "releasing caps lock restores the preference");
}

/// CodeRabbit on #10406: the middle button lends the Hand, and its cursor shows over an
/// eyedropper that owns the image even when the Hand is the selected tool as well.
#[test]
fn a_lent_hand_shows_its_cursor_over_an_eyedropper_when_it_is_also_selected() {
    let mut h = capslock_harness();
    h.state_mut().ui.tool = Tool::Hand;
    crate::color_picker_ui::open(h.state_mut(), "foreground");
    assert!(crate::color_picker_ui::top(h.state()).is_some(), "the Color Picker owns the image");
    other(&mut h, "standard");
    let pipette = fp(&h);
    assert!(pipette_shown(&h), "the eyedropper's pipette");
    let p = h.state().last_canvas_rect.center();
    let middle = |pressed| egui::Event::PointerButton { pos: p, button: egui::PointerButton::Middle, pressed, modifiers: egui::Modifiers::NONE };
    h.event(middle(true));
    h.run_steps(2);
    let lent = fp(&h);
    h.event(middle(false));
    h.run_steps(2);
    assert_ne!(lent, pipette, "the Hand's cursor, not the pipette");
    assert!(lent.icon == CursorIcon::Grab || lent.image.is_some(), "the open hand: {:?}", lent.icon);
}
