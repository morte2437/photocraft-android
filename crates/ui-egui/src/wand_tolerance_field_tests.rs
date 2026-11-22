//! #11786: the Magic Wand's Tolerance is a whole number 0–255, and the wand floors what it is
//! given (`wand_region`: `(tolerance + 1e-3).floor()`). A field that keeps a fraction therefore
//! shows a number the tool does not use, and a decimal typed with the decimal comma (the UI
//! language's separator) is rejected outright. Typing `12.5` or `12,5` must leave 13 behind, so the
//! field and the wand agree; 300 and -5 clamp, as today.

use egui::accesskit::Role;
use egui::{PointerButton, Pos2, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use photocraft_color::{ColorMode, SampleType};
use photocraft_doc::{Document, Layer, LayerContent};
use photocraft_geom::{Rect, Size};
use photocraft_raster::Surface;
use serde_json::json;

use crate::PhotocraftApp;
use crate::state::Tool;

/// The Magic Wand's options bar, with a document open, after a few settled frames.
fn wand_bar() -> Harness<'static, PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 64, "height": 64})).unwrap();
    app.sync_views();
    app.ui.tool = Tool::MagicWand;
    let mut h = Harness::builder().with_size(vec2(1000.0, 60.0)).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            if !ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            crate::panels::options_bar(app, ui);
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    h.run_steps(4);
    h
}

/// The number the Tolerance field shows, as a person reads it off the screen.
fn shown_tolerance(h: &Harness<'_, PhotocraftApp>) -> f32 {
    let node = h.get_by_role(Role::SpinButton);
    node.accesskit_node().value().unwrap_or_default().parse().unwrap_or(f32::NAN)
}

/// Types `text` over the field's number and presses Enter, as a person would, and returns the
/// option the edit left behind.
fn type_tolerance(h: &mut Harness<'_, PhotocraftApp>, text: &str) -> f32 {
    h.get_by_role(Role::SpinButton).click();
    h.run();
    h.event(egui::Event::Text(text.into()));
    h.run_steps(2);
    h.key_press(egui::Key::Enter);
    h.run_steps(2);
    h.state().ui.tool_options.tolerance
}

/// A typed decimal rounds to the nearest whole number, whichever separator it was typed with: the
/// wand's option is never a fraction for it to floor differently from what the field shows.
#[test]
fn typed_decimals_round_to_the_nearest_whole_number() {
    for (text, want) in [("12.5", 13.0), ("12,5", 13.0), ("12.4", 12.0), ("12.6", 13.0)] {
        let mut h = wand_bar();
        assert_eq!(type_tolerance(&mut h, text), want, "typing {text}");
        assert_eq!(shown_tolerance(&h), want, "the field shows the number it kept ({text})");
    }
}

/// Each operand of a typed expression is its own number: `1.5+2.5` adds up to 4, while a number
/// with two separators (`1,5,2`) is not one and leaves the option as it was.
#[test]
fn typed_expressions_take_decimal_operands() {
    let mut h = wand_bar();
    assert_eq!(type_tolerance(&mut h, "1.5+2.5"), 4.0, "typing 1.5+2.5");
    let mut h = wand_bar();
    let before = h.state().ui.tool_options.tolerance;
    assert_eq!(type_tolerance(&mut h, "1,5,2"), before, "typing 1,5,2");
}

/// A typed number outside the range clamps, as in any value field.
#[test]
fn typed_values_outside_the_range_clamp() {
    for (text, want) in [("300", 255.0), ("-5", 0.0)] {
        let mut h = wand_bar();
        assert_eq!(type_tolerance(&mut h, text), want, "typing {text}");
        assert_eq!(shown_tolerance(&h), want, "the field shows the clamped number ({text})");
    }
}

/// The reference application has no 0.1 step for this whole number: Ctrl/Cmd+arrow leaves 13 as
/// 13 on every pass, instead of writing 12.9 for the next pass to round back. A plain arrow still
/// steps by one.
#[test]
fn command_arrows_leave_the_whole_number_as_it_is() {
    // The field's value after every pass, so a fraction that only lives until the next pass shows.
    // A bare field has no caption to name it; the options bar gives it one.
    let mut h = Harness::builder().with_size(vec2(200.0, 40.0)).with_accessibility_check(false).build_ui_state(
        |ui, (value, seen): &mut (f32, Vec<f32>)| {
            crate::widgets::value_field_int(ui, value, 0.0..=255.0, "", 58.0);
            seen.push(*value);
        },
        (13.0_f32, Vec::new()),
    );
    h.run();
    h.get_by_role(Role::SpinButton).click();
    h.run();
    for key in [egui::Key::ArrowDown, egui::Key::ArrowDown, egui::Key::ArrowUp] {
        h.state_mut().1.clear();
        h.key_press_modifiers(egui::Modifiers::COMMAND, key);
        h.run_steps(2);
        assert!(h.state().1.iter().all(|&v| v == 13.0), "after Command+{key:?}: {:?}", h.state().1);
    }
    h.key_press(egui::Key::ArrowUp);
    h.run_steps(2);
    assert_eq!(h.state().0, 14.0, "a plain arrow still steps by one");
}

/// The Paint Bucket's plain field shares this option and keeps today's behaviour, and a restored
/// preference file can hold a fraction: the wand's field takes whatever it finds as the whole
/// number it shows, so the wand can never floor a value the field is displaying.
#[test]
fn a_fractional_option_becomes_the_whole_number_the_field_shows() {
    let mut h = wand_bar();
    h.state_mut().ui.tool_options.tolerance = 12.5;
    h.run_steps(2);
    assert_eq!(h.state().ui.tool_options.tolerance, 13.0);
    assert_eq!(shown_tolerance(&h), 13.0);
}

/// Six white pixels whose second one is 13 levels darker: inside the wand's reach at Tolerance 13,
/// outside it at 12, so one click tells the two numbers apart.
fn wand_canvas() -> Harness<'static, PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let mut doc = Document::new("Wand", Size::new(6, 1), ColorMode::Rgb, SampleType::U8);
    let mut pixels = Surface::with_default(doc.pixel_format(), &[1.0, 1.0, 1.0, 1.0]);
    pixels.write_pixel(1, 0, &[242.0 / 255.0, 242.0 / 255.0, 242.0 / 255.0, 1.0]);
    doc.layers.push(Layer::new("Layer 1", LayerContent::Raster(pixels)));
    let layer = doc.layers[0].id;
    app.session.add_document(doc, None);
    app.session.select_layer(layer).unwrap();
    app.ui.tool = Tool::MagicWand;
    app.ui.extras.rulers = false;
    app.sync_views();
    let mut h = Harness::builder().with_size(vec2(1000.0, 700.0)).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            let ctx = ui.ctx().clone();
            if !ctx.fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            // The options bar is a panel of its own; the canvas takes the height that is left.
            crate::panels::options_bar(app, ui);
            egui::CentralPanel::default().show(ui, |ui| crate::canvas::document_area(app, ui));
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    h.run_steps(4);
    // 40× zoom on a one-pixel-tall document: the pixels are 40 pt apart and easy to hit.
    let v = &mut h.state_mut().ui.views[0];
    v.zoom = 40.0;
    v.center = [3.0, 0.5];
    v.fit_pending = false;
    h.run_steps(2);
    h
}

/// Where document point `(x, y)` lands on screen, as the canvas' own transform maps it.
fn screen(h: &Harness<'static, PhotocraftApp>, x: f32, y: f32) -> Pos2 {
    let app = h.state();
    let v = &app.ui.views[0];
    crate::canvas::ViewXform {
        rect: crate::rulers::content_rect(app, app.last_canvas_rect),
        zoom: v.zoom,
        center: v.center,
        flip: app.ui.view.flip_horizontal,
        rotation: v.rotation,
        aspect: app.ui.view.display_aspect(),
    }
    .to_screen(x, y)
}

/// Starts from `option` in the shared tool state, clicks the first pixel with the Magic Wand, and
/// returns the area selected.
fn wand_click(option: f32) -> Rect {
    let mut h = wand_canvas();
    h.state_mut().ui.tool_options.tolerance = option;
    h.run_steps(2);
    let p = screen(&h, 0.5, 0.5);
    h.event(egui::Event::PointerMoved(p));
    h.run();
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton { pos: p, button: PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE });
        h.run_steps(2);
    }
    h.state().session.active().unwrap().doc.selection.as_ref().map_or(Rect::EMPTY, |s| s.content_bounds())
}

/// The wand selects by the whole number the field holds, so a typed `12.5` selects what 13 selects,
/// not what the 12 it would have floored to selects.
#[test]
fn the_wand_selects_by_the_number_the_field_shows() {
    let (at_12, at_13) = (wand_click(12.0), wand_click(13.0));
    assert_ne!(at_12, at_13, "Tolerance 12 and 13 must select different areas for this test to mean anything");
    assert_eq!(wand_click(12.5), at_13, "a typed 12.5 gives the 13 the field shows");
}
