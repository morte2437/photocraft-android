//! The Color panel's modes, ramp and panel menu (#6000, #2569): every mode draws and edits the
//! colour in every theme, and the choice is the panel menu's, never the theme's.

use egui::{Modifiers, Pos2, Rect, pos2, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;

use crate::PhotocraftApp;
use crate::color_panel_ui::{Model, hex_code, html_color, ramp_color};
use crate::dock::Group;
use crate::state::{ColorPanelMode, ColorRamp};
use crate::theme::{ThemeKind, Tokens};

/// The Color tab of `kind` in `mode`, alone in a 300 x 400 pt panel that offers it 360 pt.
fn color_tab(kind: ThemeKind, mode: ColorPanelMode, fg: [f32; 4]) -> Harness<'static, PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.tools.foreground = fg;
    app.ui.color_panel.mode = mode;
    color_tab_of(kind, app)
}

/// The Color tab of `kind` for `app`, as [`color_tab`].
fn color_tab_of(kind: ThemeKind, app: PhotocraftApp) -> Harness<'static, PhotocraftApp> {
    let tab = Group::Color.tabs(Tokens::for_kind(kind).pro).iter().position(|name| *name == "Color").unwrap();
    let mut h = Harness::builder().with_size(vec2(300.0, 400.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        move |ui, app: &mut PhotocraftApp| {
            if ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                ui.data_mut(|d| d.insert_temp(crate::panels::color_field_fill_id(), 360.0_f32));
                crate::panels::dock_body(app, ui, Group::Color, tab);
            }
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, kind);
    h.run_steps(3);
    h
}

/// Click at `p` with `modifiers` held.
fn press(h: &mut Harness<'_, PhotocraftApp>, p: Pos2, modifiers: Modifiers) {
    h.hover_at(p);
    h.event(egui::Event::ModifiersChanged(modifiers));
    h.run_steps(1);
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed, modifiers });
        h.run_steps(1);
    }
    h.event(egui::Event::ModifiersChanged(Modifiers::NONE));
    h.run_steps(2);
}

fn bytes(c: [f32; 4]) -> [u8; 3] {
    [c[0], c[1], c[2]].map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8)
}

fn type_into(h: &mut Harness<'_, PhotocraftApp>, role: egui::accesskit::Role, nth: usize, text: &str) {
    h.query_all_by_role(role).nth(nth).unwrap().click();
    h.run_steps(1);
    h.key_press_modifiers(Modifiers::COMMAND, egui::Key::A);
    h.run_steps(1);
    for ch in text.chars() {
        h.event(egui::Event::Text(ch.to_string()));
        h.run_steps(1);
    }
    h.key_press(egui::Key::Tab);
    h.run_steps(2);
}

#[test]
fn every_model_round_trips_its_colours() {
    let colours = [[0.2, 0.6, 0.8], [1.0, 0.0, 0.0], [0.0, 0.0, 0.0], [1.0, 1.0, 1.0], [0.5, 0.25, 0.75]];
    for model in [Model::Rgb, Model::Hsb, Model::Web, Model::Lab, Model::Cmyk] {
        for c in colours {
            // Pure red, for one, can't print: CMYK gives the nearest printable colour back.
            if model == Model::Cmyk && crate::color_picker_ui::out_of_gamut(c).is_some() {
                continue;
            }
            let back = model.to_rgb(model.values_of(c));
            let worst = (0..3).map(|i| (back[i] - c[i]).abs()).fold(0.0, f32::max);
            // Whole-number fields: RGB is exact, HSB within a step, CMYK and Lab within a few.
            let tolerance = match model {
                Model::Rgb | Model::Web => 0.6 / 255.0,
                Model::Hsb => 2.5 / 255.0,
                _ => 0.03,
            };
            assert!(worst <= tolerance, "{model:?} {c:?} -> {back:?}");
        }
    }
    for g in [0.0, 0.25, 0.5, 1.0] {
        let back = Model::Gray.to_rgb(Model::Gray.values_of([g; 3]));
        assert!((back[0] - g).abs() < 0.01, "gray {g} -> {back:?}");
    }
    // Hostile values give a valid colour.
    for model in [Model::Gray, Model::Rgb, Model::Hsb, Model::Cmyk, Model::Lab, Model::Web] {
        for v in [[f32::NAN; 4], [f32::INFINITY; 4], [-1e9; 4], [1e9; 4]] {
            assert!(model.to_rgb(v).iter().all(|x| (0.0..=1.0).contains(x)), "{model:?} {v:?}");
        }
        assert_eq!(
            model.channels().len(),
            match model {
                Model::Gray => 1,
                Model::Cmyk => 4,
                _ => 3,
            }
        );
    }
}

#[test]
fn ramps_match_photoshop() {
    let (fg, bg) = ([0.2, 0.4, 0.6], [1.0, 1.0, 0.0]);
    let near = |a: [f32; 3], b: [f32; 3]| (0..3).all(|i| (a[i] - b[i]).abs() < 0.02);
    // RGB Spectrum: hue across, white at the top, the full colour in the middle, black at the bottom.
    assert!(near(ramp_color(ColorRamp::RgbSpectrum, false, fg, bg, 0.0, 0.0), [1.0; 3]));
    assert!(near(ramp_color(ColorRamp::RgbSpectrum, false, fg, bg, 0.0, 0.5), [1.0, 0.0, 0.0]));
    assert!(near(ramp_color(ColorRamp::RgbSpectrum, false, fg, bg, 1.0 / 3.0, 0.5), [0.0, 1.0, 0.0]));
    assert!(near(ramp_color(ColorRamp::RgbSpectrum, false, fg, bg, 0.5, 1.0), [0.0; 3]));
    assert!(near(ramp_color(ColorRamp::GrayscaleRamp, false, fg, bg, 0.0, 0.3), [1.0; 3]));
    assert!(near(ramp_color(ColorRamp::GrayscaleRamp, false, fg, bg, 1.0, 0.3), [0.0; 3]));
    assert!(near(ramp_color(ColorRamp::CurrentColors, false, fg, bg, 0.0, 0.9), fg));
    assert!(near(ramp_color(ColorRamp::CurrentColors, false, fg, bg, 1.0, 0.1), bg));
    // CMYK Spectrum stays printable.
    let c = ramp_color(ColorRamp::CmykSpectrum, false, fg, bg, 0.6, 0.5);
    assert!(crate::color_picker_ui::out_of_gamut(c).is_none(), "{c:?}");
    // Make Ramp Web Safe: every colour is a web colour.
    for ramp in ColorRamp::ALL {
        for (x, y) in [(0.13, 0.37), (0.71, 0.66), (0.5, 0.5)] {
            let c = ramp_color(ramp, true, fg, bg, x, y);
            assert!(crate::color_picker_ui::not_web_safe(c).is_none(), "{ramp:?} {c:?}");
        }
        let c = ramp_color(ramp, false, fg, bg, f32::NAN, f32::INFINITY);
        assert!(c.iter().all(|v| (0.0..=1.0).contains(v)), "{ramp:?} {c:?}");
    }
    // Shift-click cycles the ramps in menu order and wraps.
    let mut r = ColorRamp::RgbSpectrum;
    let mut seen = vec![r];
    for _ in 0..3 {
        r = r.next();
        seen.push(r);
    }
    assert_eq!(seen, ColorRamp::ALL);
    assert_eq!(r.next(), ColorRamp::RgbSpectrum);
}

#[test]
fn copy_color_formats() {
    assert_eq!(html_color([0.2, 0.6, 0.8]), "color=\"#3399cc\"");
    assert_eq!(hex_code([0.2, 0.6, 0.8]), "3399cc");
}

/// Every mode draws in every theme, and a click in it sets the colour.
#[test]
fn every_mode_works_in_every_theme() {
    for kind in ThemeKind::ALL {
        for mode in ColorPanelMode::ALL {
            let mut h = color_tab(kind, mode, [1.0, 0.0, 0.0, 1.0]);
            match mode {
                ColorPanelMode::HueCube | ColorPanelMode::BrightnessCube => {
                    assert!(h.query_by_label("#").is_some(), "{kind:?} {mode:?}: the readout");
                }
                ColorPanelMode::ColorWheel => {
                    for label in ["H", "S", "B", "Add to Swatches"] {
                        assert!(h.query_by_label(label).is_some(), "{kind:?} {mode:?}: {label}");
                    }
                }
                _ => {
                    for (label, ..) in Model::of(mode).channels() {
                        assert!(h.query_by_label(label).is_some(), "{kind:?} {mode:?}: the {label} slider");
                    }
                    // The black box at the end of the ramp sets black.
                    let black = h.get_by_label("Black").rect().center();
                    press(&mut h, black, Modifiers::NONE);
                    assert_eq!(bytes(h.state().session.tools.foreground), [0, 0, 0], "{kind:?} {mode:?}");
                }
            }
            assert!(h.query_by_label("Hue").is_none() && h.query_by_label("Saturation").is_none(), "{kind:?} {mode:?}: no old sliders");
        }
    }
}

#[test]
fn rgb_slider_track_sets_its_channel() {
    let mut h = color_tab(ThemeKind::AdwaitaDark, ColorPanelMode::RgbSliders, [0.0, 0.0, 0.0, 1.0]);
    let g = h.get_by_label("G").rect();
    press(&mut h, pos2(g.right() - 2.0, g.center().y), Modifiers::NONE);
    assert_eq!(bytes(h.state().session.tools.foreground), [0, 255, 0]);
    // The field takes a typed value too (the spin buttons are R, G, B).
    type_into(&mut h, egui::accesskit::Role::SpinButton, 2, "128");
    assert_eq!(bytes(h.state().session.tools.foreground), [0, 255, 128]);
}

/// Black has no hue or saturation of its own: typed H and S must stick while B is still 0.
#[test]
fn hsb_sliders_take_values_on_black() {
    let mut h = color_tab(ThemeKind::Studio, ColorPanelMode::HsbSliders, [0.0, 0.0, 0.0, 1.0]);
    for (field, typed) in [(0, "120"), (1, "100"), (2, "100")] {
        type_into(&mut h, egui::accesskit::Role::SpinButton, field, typed);
    }
    assert_eq!(bytes(h.state().session.tools.foreground), [0, 255, 0]);
}

#[test]
fn cmyk_sliders_keep_typed_values() {
    let mut h = color_tab(ThemeKind::Pro, ColorPanelMode::CmykSliders, [1.0, 1.0, 1.0, 1.0]);
    type_into(&mut h, egui::accesskit::Role::SpinButton, 0, "40");
    type_into(&mut h, egui::accesskit::Role::SpinButton, 3, "20");
    let shown: Vec<String> = h.query_all_by_role(egui::accesskit::Role::SpinButton).filter_map(|n| n.value()).collect();
    assert_eq!(shown.first().map(String::as_str), Some("40"), "C stays as typed: {shown:?}");
    assert_eq!(shown.get(3).map(String::as_str), Some("20"), "K stays as typed: {shown:?}");
}

#[test]
fn web_sliders_snap_and_take_hex() {
    let mut h = color_tab(ThemeKind::Classic, ColorPanelMode::WebColorSliders, [0.0, 0.0, 0.0, 1.0]);
    let r = h.get_by_label("R").rect();
    // 45% along the track snaps to 0x66 (102), the nearest web step.
    let x = r.left() + 4.0 + (r.width() - 8.0) * 0.45;
    press(&mut h, pos2(x, r.center().y), Modifiers::NONE);
    assert_eq!(bytes(h.state().session.tools.foreground), [0x66, 0, 0]);
    // Not a web colour: the cube alert snaps it.
    type_into(&mut h, egui::accesskit::Role::TextInput, 1, "C8");
    assert_eq!(bytes(h.state().session.tools.foreground), [0x66, 0xC8, 0]);
    let alert = h.get_by_label("Not a web colour: click for the nearest web colour").rect().center();
    press(&mut h, alert, Modifiers::NONE);
    assert_eq!(bytes(h.state().session.tools.foreground), [0x66, 0xCC, 0]);
}

#[test]
fn cube_modes_warn_when_the_colour_cannot_print() {
    // #8672: Photoshop's Hue and Brightness Cube show the ⚠ and the nearest printable colour under
    // the chips, as the slider modes do; a click takes that colour.
    const GAMUT: &str = "Out of gamut for printing: click for the nearest printable colour";
    for mode in [ColorPanelMode::HueCube, ColorPanelMode::BrightnessCube] {
        let mut h = color_tab(ThemeKind::Classic, mode, [0.0, 1.0, 0.0, 1.0]);
        let near = crate::color_picker_ui::out_of_gamut([0.0, 1.0, 0.0]).expect("pure green can't print");
        let alert = h.get_by_label(GAMUT).rect().center();
        press(&mut h, alert, Modifiers::NONE);
        let fg = h.state().session.tools.foreground;
        assert_eq!(bytes(fg), bytes([near[0], near[1], near[2], 1.0]), "{mode:?}");
        h.run_steps(2);
        assert!(h.query_by_label(GAMUT).is_none(), "{mode:?}: the printable colour still warns");
        // A printable colour shows no alert.
        let h = color_tab(ThemeKind::Classic, mode, [0.5, 0.5, 0.5, 1.0]);
        assert!(h.query_by_label(GAMUT).is_none(), "{mode:?}");
        // CodeRabbit on #9154: with the background chip active, the click sets the background.
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.session.tools.foreground = [0.5, 0.5, 0.5, 1.0];
        app.session.tools.background = [0.0, 1.0, 0.0, 1.0];
        app.ui.color_panel.mode = mode;
        app.ui.color_panel.background = true;
        let mut h = color_tab_of(ThemeKind::Classic, app);
        let alert = h.get_by_label(GAMUT).rect().center();
        press(&mut h, alert, Modifiers::NONE);
        let tools = &h.state().session.tools;
        assert_eq!(bytes(tools.background), bytes([near[0], near[1], near[2], 1.0]), "{mode:?}: background");
        assert_eq!(bytes(tools.foreground), bytes([0.5, 0.5, 0.5, 1.0]), "{mode:?}: the foreground is left alone");
    }
}

#[test]
fn ramp_picks_and_shift_click_cycles_and_alt_picks_the_other_colour() {
    let mut h = color_tab(ThemeKind::BreezeDark, ColorPanelMode::RgbSliders, [0.0, 0.0, 0.0, 1.0]);
    let ramp: Rect = h.get_by_label("Color Ramp").rect();
    // The RGB Spectrum's top is white.
    press(&mut h, pos2(ramp.left() + 3.0, ramp.top() + 1.0), Modifiers::NONE);
    assert!(bytes(h.state().session.tools.foreground).iter().all(|v| *v > 240), "{:?}", h.state().session.tools.foreground);
    // Alt picks the background (the panel edits the foreground).
    press(&mut h, pos2(ramp.left() + 3.0, ramp.bottom() - 1.0), Modifiers::ALT);
    assert!(bytes(h.state().session.tools.background).iter().all(|v| *v < 16), "{:?}", h.state().session.tools.background);
    assert!(bytes(h.state().session.tools.foreground).iter().all(|v| *v > 240), "Alt leaves the foreground");
    // Shift-click cycles the ramp and picks nothing.
    let fg = h.state().session.tools.foreground;
    for expected in [ColorRamp::CmykSpectrum, ColorRamp::GrayscaleRamp, ColorRamp::CurrentColors, ColorRamp::RgbSpectrum] {
        press(&mut h, ramp.center(), Modifiers::SHIFT);
        assert_eq!(h.state().ui.color_panel.ramp, expected);
    }
    assert_eq!(h.state().session.tools.foreground, fg);
    // Grayscale Ramp: its left end is white.
    h.state_mut().ui.color_panel.ramp = ColorRamp::GrayscaleRamp;
    h.state_mut().session.tools.foreground = [0.0, 0.0, 0.0, 1.0];
    h.run_steps(2);
    press(&mut h, pos2(ramp.left() + 1.0, ramp.center().y), Modifiers::NONE);
    assert!(bytes(h.state().session.tools.foreground).iter().all(|v| *v > 245));
}

/// A drag on the Current Colors ramp picks from the ends it began with: holding still keeps the
/// colour under the pointer instead of drifting toward the background.
#[test]
fn current_colors_ramp_drag_keeps_its_ends() {
    let mut h = color_tab(ThemeKind::BreezeDark, ColorPanelMode::RgbSliders, [0.0, 0.0, 0.0, 1.0]);
    h.state_mut().session.tools.background = [1.0, 1.0, 1.0, 1.0];
    h.state_mut().ui.color_panel.ramp = ColorRamp::CurrentColors;
    h.run_steps(2);
    let ramp: Rect = h.get_by_label("Color Ramp").rect();
    let p = ramp.center();
    h.hover_at(p);
    h.event(egui::Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.run_steps(1);
    // Move away and back to start the drag, then hold still for a while.
    for q in [p + vec2(12.0, 0.0), p] {
        h.event(egui::Event::PointerMoved(q));
        h.run_steps(1);
    }
    h.run_steps(20);
    h.event(egui::Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(2);
    let expected = ramp_color(ColorRamp::CurrentColors, false, [0.0; 3], [1.0; 3], (p.x - ramp.left()) / ramp.width(), 0.5);
    let got = bytes(h.state().session.tools.foreground);
    for (g, e) in got.iter().zip(expected) {
        assert!(i32::from(*g).abs_diff((e * 255.0).round() as i32) <= 3, "{got:?} vs {expected:?}");
    }
}

/// A non-finite pointer (a huge synthetic `ui.move`) misses the triangle instead of panicking.
#[test]
fn triangle_ignores_non_finite_points() {
    let tri = [pos2(100.0, 0.0), pos2(0.0, 0.0), pos2(0.0, 100.0)];
    for p in [pos2(f32::INFINITY, f32::INFINITY), pos2(f32::NAN, 1.0), pos2(f32::NEG_INFINITY, f32::INFINITY)] {
        assert_eq!(crate::color_panel_ui::triangle_sv(tri, p), None, "{p:?}");
    }
    assert!(crate::color_panel_ui::triangle_sv(tri, pos2(10.0, 10.0)).is_some());
}

#[test]
fn color_wheel_ring_sets_hue_and_triangle_sets_saturation_and_brightness() {
    let mut h = color_tab(ThemeKind::Pro, ColorPanelMode::ColorWheel, [0.0, 0.0, 1.0, 1.0]);
    let area = h.get_by_label("Color Wheel").rect();
    let (center, inner, outer) = crate::color_panel_ui::wheel_geometry(area);
    let mid = (inner + outer) / 2.0;
    // Up on the ring: hue 90° (yellow-green). Blue is fully saturated and bright, so it keeps both.
    press(&mut h, pos2(center.x, center.y - mid), Modifiers::NONE);
    let [r, g, bl] = bytes(h.state().session.tools.foreground);
    assert!(r > 100 && g > 240 && bl < 10, "hue 90: {r} {g} {bl}");
    // The triangle's centre: some saturation, some brightness, same hue.
    press(&mut h, center, Modifiers::NONE);
    let after = bytes(h.state().session.tools.foreground);
    let hsv = photocraft_color::convert::rgb_to_hsb(after.map(|v| f32::from(v) / 255.0));
    assert!((hsv[0] - 90.0).abs() < 4.0 && hsv[1] < 0.9 && hsv[2] < 0.95, "{after:?} {hsv:?}");
}

/// Picking a mode from the panel menu changes the panel; the theme never does.
#[test]
fn panel_menu_picks_the_mode_and_ramp() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.tools.foreground = [0.2, 0.6, 0.8, 1.0];
    let mut h = Harness::builder().with_size(vec2(260.0, 700.0)).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            if ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                crate::color_panel_ui::panel_menu(app, ui);
            }
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, ThemeKind::AdwaitaDark);
    h.run_steps(2);
    assert!(h.query_by_label("✔ Hue Cube").is_some(), "Hue Cube is the default");
    for mode in ColorPanelMode::ALL.into_iter().skip(1).chain([ColorPanelMode::HueCube, ColorPanelMode::WebColorSliders]) {
        h.get_by_label(&format!("   {}", mode.label())).click();
        h.run_steps(2);
        assert_eq!(h.state().ui.color_panel.mode, mode);
        assert!(h.query_by_label(&format!("✔ {}", mode.label())).is_some());
    }
    h.get_by_label("   Current Colors").click();
    h.run_steps(2);
    assert_eq!(h.state().ui.color_panel.ramp, ColorRamp::CurrentColors);
    h.get_by_label("   Make Ramp Web Safe").click();
    h.run_steps(2);
    assert!(h.state().ui.color_panel.web_safe_ramp);
    h.get_by_label("Copy Color's Hex Code").click();
    h.run_steps(1);
    let copied: Vec<_> =
        h.output().platform_output.commands.iter().filter_map(|c| if let egui::OutputCommand::CopyText(s) = c { Some(s.clone()) } else { None }).collect();
    assert_eq!(copied, ["3399cc"]);
    // Switching theme keeps the mode.
    let ctx = h.ctx.clone();
    h.state_mut().set_theme(&ctx, ThemeKind::Pro);
    h.run_steps(2);
    assert_eq!(h.state().ui.color_panel.mode, ColorPanelMode::WebColorSliders);
}

/// `ui.set colorPanel` merges, so setting the mode keeps the edited chip; bad values change nothing.
#[test]
fn control_sets_the_mode_ramp_and_web_safe_ramp() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let ctx = egui::Context::default();
    let call = |app: &mut PhotocraftApp, params: serde_json::Value| {
        let (req, _rx) = crate::control::ControlRequest::new("ui.set", params);
        match crate::control::handle(app, &ctx, &req) {
            crate::control::Outcome::Done(v) => v["ok"] == true,
            _ => panic!("expected an immediate reply"),
        }
    };
    assert!(call(&mut app, json!({"colorPanel": {"background": true}})));
    assert!(call(&mut app, json!({"colorPanel": {"mode": "labSliders", "ramp": "grayscaleRamp", "webSafeRamp": true}})));
    let cp = app.ui.color_panel;
    assert!(cp.background, "the chip is kept");
    assert_eq!((cp.mode, cp.ramp, cp.web_safe_ramp), (ColorPanelMode::LabSliders, ColorRamp::GrayscaleRamp, true));
    for bad in [json!({"colorPanel": {"mode": "hueWheel"}}), json!({"colorPanel": {"ramp": 3}}), json!({"colorPanel": {"webSafeRamp": "yes"}})] {
        assert!(!call(&mut app, bad.clone()), "{bad}");
        assert_eq!(app.ui.color_panel, cp, "{bad}: nothing changes");
    }
}

/// Only Web Colors is `webOnly` now that the state is camelCase; state saved before still loads.
#[test]
fn web_only_reads_its_old_snake_case_key() {
    let old: crate::state::ColorPanelState = serde_json::from_value(json!({"background": true, "web_only": true})).unwrap();
    assert!(old.background && old.web_only);
    let new = serde_json::to_value(old).unwrap();
    assert_eq!((new.get("webOnly"), new.get("web_only")), (Some(&json!(true)), None));
}

/// Make Ramp Web Safe: a click picks the cell drawn under the pointer, even near its edge. On
/// the Grayscale Ramp the fifth cell is drawn white, though its right edge rounds to CC.
#[test]
fn web_safe_ramp_picks_the_cell_it_draws() {
    let mut h = color_tab(ThemeKind::Pro, ColorPanelMode::RgbSliders, [0.0, 0.0, 0.0, 1.0]);
    h.state_mut().ui.color_panel.ramp = ColorRamp::GrayscaleRamp;
    h.state_mut().ui.color_panel.web_safe_ramp = true;
    h.run_steps(2);
    let ramp: Rect = h.get_by_label("Color Ramp").rect();
    press(&mut h, pos2(ramp.left() + ramp.width() * 4.95 / 48.0, ramp.center().y), Modifiers::NONE);
    assert_eq!(bytes(h.state().session.tools.foreground), [255, 255, 255]);
}

/// A Web Color field takes two hex digits only: "+F" is not 0F.
#[test]
fn web_field_ignores_a_sign() {
    let mut h = color_tab(ThemeKind::Pro, ColorPanelMode::WebColorSliders, [0.0, 0.0, 0.0, 1.0]);
    type_into(&mut h, egui::accesskit::Role::TextInput, 0, "+F");
    assert_eq!(bytes(h.state().session.tools.foreground), [0, 0, 0]);
}

/// A click beside the Color Wheel (its area spans the panel) leaves the colour alone.
#[test]
fn color_wheel_ignores_clicks_beside_it() {
    let mut h = color_tab(ThemeKind::Pro, ColorPanelMode::ColorWheel, [0.0, 0.0, 1.0, 1.0]);
    let area = h.get_by_label("Color Wheel").rect();
    let (center, _, outer) = crate::color_panel_ui::wheel_geometry(area);
    let corner = area.left_top() + vec2(3.0, 3.0);
    assert!(corner.distance(center) > outer + 20.0, "the corner is beside the wheel");
    press(&mut h, corner, Modifiers::NONE);
    assert_eq!(bytes(h.state().session.tools.foreground), [0, 0, 255]);
}

/// The Color Wheel shrinks to the Color group at its default height in every theme, as the
/// slider modes' ramp does, instead of running past the group's bottom into its scroll area.
#[test]
fn color_wheel_fits_the_default_color_group() {
    for kind in ThemeKind::ALL {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.ui.color_panel.mode = ColorPanelMode::ColorWheel;
        app.ui.theme = kind;
        app.ui.dock_tabs.color = Group::Color.tabs(Tokens::for_kind(kind).pro).iter().position(|name| *name == "Color").unwrap();
        let mut h = Harness::builder().with_size(vec2(1440.0, 1200.0)).build_ui_state(
            |ui, app: &mut PhotocraftApp| {
                if ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                    crate::panels::dock_area(app, ui);
                    egui::CentralPanel::default().show(ui, |_| {});
                }
            },
            app,
        );
        PhotocraftApp::setup_context(&h.ctx, kind);
        h.run_steps(4);
        let group = crate::panel_docking::probe::native_group_rect(h.state(), &h.ctx, Group::Color).unwrap();
        assert!((group.height() - Group::Color.default_height()).abs() < 0.5, "{kind:?}: Color group {group:?}");
        let wheel = h.query_by_label("Color Wheel").unwrap_or_else(|| panic!("{kind:?}: no Color Wheel")).rect();
        // The group's scroll area ends at least 8 pt above its bottom edge (the card's bottom margin).
        assert!(wheel.bottom() <= group.bottom() - 8.0, "{kind:?}: wheel {wheel:?} runs past the Color group {group:?}");
    }
}

/// The Color tab in `mode` with a document holding one selected black rectangle, the Rectangle tool
/// in hand, foreground blue and background red, editing the background chip if `background`.
fn color_tab_with_a_shape(mode: ColorPanelMode, background: bool) -> Harness<'static, PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
    app.sync_views();
    app.run("shape.create", json!({"kind": "rect", "rect": [20, 20, 50, 50], "fill": "#000000"})).unwrap();
    app.ui.tool = crate::state::Tool::Rectangle;
    app.session.tools.foreground = [0.0, 0.0, 1.0, 1.0];
    app.session.tools.background = [1.0, 0.0, 0.0, 1.0];
    app.ui.color_panel.mode = mode;
    app.ui.color_panel.background = background;
    color_tab_of(ThemeKind::Studio, app)
}

/// Types `text` into the first slider field (Hue or Red) and commits it with Enter.
fn type_first_slider(h: &mut Harness<'_, PhotocraftApp>, text: &str) {
    h.query_all_by_role(egui::accesskit::Role::SpinButton).next().unwrap().click();
    h.run_steps(2);
    h.key_press_modifiers(Modifiers::COMMAND, egui::Key::A);
    h.run_steps(1);
    h.event(egui::Event::Text(text.into()));
    h.run_steps(1);
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
}

fn shape_fill(app: &PhotocraftApp) -> [u8; 3] {
    let st = app.session.active().unwrap();
    let photocraft_doc::LayerContent::Shape(sh) = &st.doc.layer(st.active_layer.unwrap()).unwrap().content else { panic!("shape layer") };
    let Some(photocraft_doc::Fill::Solid(c)) = &sh.fill else { panic!("solid fill") };
    bytes([c.c[0], c.c[1], c.c[2], 1.0])
}

/// The HSB sliders edit the chip the panel is editing (#5842): with the background chip active
/// they change the background, and the selected shape (which takes the foreground with a shape
/// tool in hand) keeps its fill. On the foreground chip the shape follows.
#[test]
fn hsb_sliders_edit_the_active_chip_and_only_the_foreground_recolours_shapes() {
    let mut h = color_tab_with_a_shape(ColorPanelMode::HsbSliders, true);
    type_first_slider(&mut h, "120");
    let app = h.state();
    assert_eq!(bytes(app.session.tools.background), [0, 255, 0], "the background takes the hue");
    assert_eq!(bytes(app.session.tools.foreground), [0, 0, 255], "the foreground is left alone");
    assert_eq!(shape_fill(app), [0, 0, 0], "a background edit recolours nothing");

    let mut h = color_tab_with_a_shape(ColorPanelMode::HsbSliders, false);
    type_first_slider(&mut h, "120");
    let app = h.state();
    assert_eq!(bytes(app.session.tools.foreground), [0, 255, 0]);
    assert_eq!(bytes(app.session.tools.background), [255, 0, 0]);
    assert_eq!(shape_fill(app), [0, 255, 0], "the selected shape takes the new foreground");
}

/// The RGB sliders recolour the selected shape like the HSB sliders, the field and the swatches
/// do (`apply_color`), in one history step, and only for a foreground edit.
#[test]
fn rgb_sliders_recolour_the_selected_shape_with_the_foreground() {
    let mut h = color_tab_with_a_shape(ColorPanelMode::RgbSliders, false);
    let steps = h.state().session.active().unwrap().history.entries().len();
    type_first_slider(&mut h, "255");
    let app = h.state();
    assert_eq!(bytes(app.session.tools.foreground), [255, 0, 255]);
    assert_eq!(shape_fill(app), [255, 0, 255]);
    assert_eq!(app.session.active().unwrap().history.entries().len(), steps + 1, "one history step");

    let mut h = color_tab_with_a_shape(ColorPanelMode::RgbSliders, true);
    type_first_slider(&mut h, "0");
    let app = h.state();
    assert_eq!(bytes(app.session.tools.background), [0, 0, 0]);
    assert_eq!(shape_fill(app), [0, 0, 0], "a background edit recolours nothing");
}

/// Every slider mode's ramp (here its white box) recolours the selected shape with a new
/// foreground, as the sliders and the field do.
#[test]
fn every_slider_mode_recolours_the_selected_shape() {
    for mode in ColorPanelMode::ALL {
        if matches!(mode, ColorPanelMode::HueCube | ColorPanelMode::BrightnessCube | ColorPanelMode::ColorWheel) {
            continue;
        }
        let mut h = color_tab_with_a_shape(mode, false);
        let white = h.get_by_label("White").rect().center();
        press(&mut h, white, Modifiers::NONE);
        assert_eq!(bytes(h.state().session.tools.foreground), [255, 255, 255], "{mode:?}");
        assert_eq!(shape_fill(h.state()), [255, 255, 255], "{mode:?}");
    }
}

/// #9592: the Color panel's options survive a restart (they ride in the preferences), as in
/// Photoshop; preferences without them keep the defaults.
#[test]
fn color_panel_options_survive_a_restart() {
    let mut first = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    first.ui.color_panel.mode = ColorPanelMode::HsbSliders;
    first.ui.color_panel.ramp = ColorRamp::GrayscaleRamp;
    crate::color_panel_ui::persist(&mut first);
    let saved = first.session.prefs().clone();

    let mut next = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    next.session.prefs.edit(|p| *p = saved);
    crate::color_panel_ui::restore(&mut next);
    assert_eq!((next.ui.color_panel.mode, next.ui.color_panel.ramp), (ColorPanelMode::HsbSliders, ColorRamp::GrayscaleRamp));

    let mut fresh = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    crate::color_panel_ui::restore(&mut fresh);
    assert_eq!(fresh.ui.color_panel.mode, ColorPanelMode::HueCube, "no saved options: the default");
}

/// CodeRabbit on #9606: a preference change made outside the panel (`prefs.set`, a reset) is kept.
/// The next frame takes it into the live panel instead of writing the panel's old options back.
#[test]
fn an_outside_preference_change_reaches_the_live_panel() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.ui.color_panel.mode = ColorPanelMode::HsbSliders;
    crate::color_panel_ui::persist(&mut app);
    // As prefs.set or a reset to defaults would: the stored options change under the panel.
    let mut other = app.ui.color_panel;
    other.mode = ColorPanelMode::GrayscaleSlider;
    app.session.prefs.edit(|p| p.color_panel = serde_json::to_value(other).unwrap());
    crate::color_panel_ui::persist(&mut app);
    assert_eq!(app.ui.color_panel.mode, ColorPanelMode::GrayscaleSlider, "the live panel follows the preference");
    let stored: crate::state::ColorPanelState = serde_json::from_value(app.session.prefs().color_panel.clone()).unwrap();
    assert_eq!(stored.mode, ColorPanelMode::GrayscaleSlider, "and the preference isn't overwritten");
    // A reset to defaults clears the stored options: the panel goes back to its defaults too.
    app.session.prefs.edit(|p| p.color_panel = serde_json::Value::Null);
    crate::color_panel_ui::persist(&mut app);
    assert_eq!(app.ui.color_panel.mode, ColorPanelMode::HueCube, "reset: the default mode");
    crate::color_panel_ui::persist(&mut app);
    assert_eq!(app.ui.color_panel.mode, ColorPanelMode::HueCube, "and it stays reset");
}
