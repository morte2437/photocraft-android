use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

use super::*;

/// #4107: with Only Web Colors on, the field is drawn in flat web-safe bands.
#[test]
fn web_only_field_is_flat_web_safe_bands() {
    let rect = Rect::from_min_size(pos2(0.0, 0.0), vec2(200.0, 200.0));
    let m = flat_grid_mesh(rect, WEB_CELLS, |x, y| web_safe(field_color("h", 0.3, x, y)));
    let ramp = [0u8, 51, 102, 153, 204, 255];
    for v in &m.vertices {
        assert!([v.color.r(), v.color.g(), v.color.b()].iter().all(|c| ramp.contains(c)), "{:?}", v.color);
    }
    // Each cell's four corners share one colour (no blending between bands).
    for q in m.vertices.chunks(4) {
        assert!(q.iter().all(|v| v.color == q[0].color));
    }
    let distinct: std::collections::HashSet<_> = m.vertices.iter().map(|v| v.color).collect();
    assert!(distinct.len() > 1 && distinct.len() <= 216);
}

fn picker(color: &str) -> Harness<'static, Map<String, Value>> {
    let mut f = Map::new();
    f.insert("__colorPicker".into(), json!("foreground"));
    f.insert("color".into(), json!(color));
    f.insert("__orig".into(), json!(color));
    let mut h = Harness::builder().with_size(vec2(700.0, 500.0)).build_ui_state(
        |ui, f: &mut Map<String, Value>| {
            if ui.ctx().fonts(|fonts| fonts.families().contains(&egui::FontFamily::Name("medium".into()))) {
                body(ui, f);
            }
        },
        f,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    h.run_steps(2);
    h
}

/// Type `text` into the focused field one key per frame, like a person typing.
fn type_keys(h: &mut Harness<'static, Map<String, Value>>, text: &str) {
    for ch in text.chars() {
        h.event(egui::Event::Text(ch.to_string()));
        h.run_steps(1);
    }
}

fn color(h: &Harness<'static, Map<String, Value>>) -> String {
    h.state().get("color").and_then(Value::as_str).unwrap().to_string()
}

/// Press at `at` and hold, with no release and no time passing, then run one frame.
fn press<S>(h: &mut Harness<'static, S>, at: egui::Pos2) {
    press_button(h, at, egui::PointerButton::Primary, true);
}

fn press_button<S>(h: &mut Harness<'static, S>, at: egui::Pos2, button: egui::PointerButton, pressed: bool) {
    h.event(egui::Event::PointerMoved(at));
    h.event(egui::Event::PointerButton { pos: at, button, pressed, modifiers: egui::Modifiers::NONE });
    h.run_steps(1);
}

/// #7793: the field and the slider pick on the frame the button goes down, not after the
/// pointer has travelled past egui's click threshold or been held for a while, and follow
/// moves of a few points.
#[test]
fn field_and_slider_pick_on_press_and_follow_small_moves() {
    let mut f = Map::new();
    f.insert("color".into(), json!("#336699"));
    let mut h = Harness::builder().with_size(vec2(400.0, 300.0)).build_ui_state(
        |ui, f: &mut Map<String, Value>| {
            ui.horizontal(|ui| {
                let o = ui.cursor().min;
                let gap = ui.spacing().item_spacing.x;
                ui.data_mut(|d| d.insert_temp(egui::Id::unique("test-origin"), (o, gap)));
                color_field(ui, f);
            });
        },
        f,
    );
    h.run_steps(2);
    let (o, gap) = h.ctx.data(|d| d.get_temp::<(egui::Pos2, f32)>(egui::Id::unique("test-origin"))).unwrap();
    // Field: saturation across, brightness down (hue mode).
    let at = o + vec2(FIELD * 0.5, FIELD * 0.5);
    press(&mut h, at);
    let picked = color(&h);
    assert_ne!(picked, "#336699", "the press itself picks");
    let hsv = rgb_to_hsb(parse_hex(&picked).unwrap());
    assert!((hsv[1] - 0.5).abs() < 0.02 && (hsv[2] - 0.5).abs() < 0.02, "{picked} {hsv:?}");
    // A 3-point nudge (under the 6-point click threshold) moves the pick.
    h.event(egui::Event::PointerMoved(at + vec2(3.0, 0.0)));
    h.run_steps(1);
    assert_ne!(color(&h), picked, "a small drag adjusts the pick");
    h.event(egui::Event::PointerButton { pos: at + vec2(3.0, 0.0), button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE });
    h.run_steps(2);
    // Hue slider: a press near the bottom picks a hue near 0 at once.
    let before = color(&h);
    press(&mut h, o + vec2(FIELD + gap + 9.0, FIELD * 0.95));
    assert_ne!(color(&h), before, "the slider picks on press");
    let hue = rgb_to_hsb(parse_hex(&color(&h)).unwrap())[0];
    assert!(hue < 30.0 || hue > 330.0, "hue {hue}");
}

/// Only the primary button picks: a right or middle press (and its release) on the field or
/// the slider leaves the colour alone.
#[test]
fn field_and_slider_ignore_secondary_and_middle_presses() {
    let mut f = Map::new();
    f.insert("color".into(), json!("#336699"));
    let mut h = Harness::builder().with_size(vec2(400.0, 300.0)).build_ui_state(
        |ui, f: &mut Map<String, Value>| {
            ui.horizontal(|ui| {
                let o = ui.cursor().min;
                let gap = ui.spacing().item_spacing.x;
                ui.data_mut(|d| d.insert_temp(egui::Id::unique("test-origin"), (o, gap)));
                color_field(ui, f);
            });
        },
        f,
    );
    h.run_steps(2);
    let (o, gap) = h.ctx.data(|d| d.get_temp::<(egui::Pos2, f32)>(egui::Id::unique("test-origin"))).unwrap();
    for at in [o + vec2(FIELD * 0.5, FIELD * 0.5), o + vec2(FIELD + gap + 9.0, FIELD * 0.95)] {
        for button in [egui::PointerButton::Secondary, egui::PointerButton::Middle] {
            press_button(&mut h, at, button, true);
            assert_eq!(color(&h), "#336699", "{button:?} press at {at:?}");
            h.event(egui::Event::PointerMoved(at + vec2(3.0, 0.0)));
            h.run_steps(1);
            press_button(&mut h, at + vec2(3.0, 0.0), button, false);
            h.run_steps(1);
            assert_eq!(color(&h), "#336699", "{button:?} drag and release at {at:?}");
        }
    }
}

#[test]
fn hex_field_shows_uppercase_without_rewriting_the_selected_color() {
    let h = picker("#12abef");
    assert_eq!(h.get_by_role(Role::TextInput).value().as_deref(), Some("12ABEF"));
    assert_eq!(color(&h), "#12abef", "casing is presentation only");
}

/// #6937: pressing the hue strip must update the colour on the press frame, not only when
/// the button is released (Photoshop updates immediately on click-and-hold).
#[test]
fn hue_strip_updates_the_color_on_press_not_on_release() {
    let mut h = picker("#0000ff"); // pure blue: hue 240°
    let web = h.get_by_label("Only Web Colors").rect();
    let pick = h.get_by_label("Pick screen color").rect();
    // The strip is 18 pt wide, 6 pt right of the 200 pt field whose left edge is the column's
    // (the screen-colour button shares it); its bottom edge is just above the checkbox.
    // Press near the bottom: hue ~5°, a red the blue starting colour cannot produce via the field.
    let press = pos2(pick.left() + FIELD + 15.0, web.top() - 20.0);
    h.event(egui::Event::PointerMoved(press));
    h.run_steps(1);
    h.event(egui::Event::PointerButton { pos: press, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE });
    h.run_steps(1);
    let held = color(&h);
    assert_ne!(held, "#0000ff", "holding the button on the strip must already change the colour");
    // The colour is the red-ish hue near the strip's bottom, not the old blue: had the press
    // landed on the field instead of the strip, the hue (and the blue channel) would survive.
    let [r, _, b] = parse_hex(&held).unwrap();
    assert!(r > 0.5 && b < 0.2, "expected a red-ish hue from the strip, got {held}");
    h.event(egui::Event::PointerButton { pos: press, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE });
    h.run_steps(1);
    assert_eq!(color(&h), held, "releasing must not move the colour again");
}

#[test]
fn hex_field_takes_typed_digits() {
    let mut h = picker("#000000");
    h.get_by_role(Role::TextInput).click();
    h.run_steps(1);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.run_steps(1);
    type_keys(&mut h, "ff88");
    // Partial input stays in the field instead of snapping back to the colour.
    assert_eq!(h.get_by_role(Role::TextInput).value().as_deref(), Some("ff88"));
    assert_eq!(color(&h), "#000000");
    type_keys(&mut h, "00");
    assert_eq!(color(&h), "#ff8800");
    h.key_press(egui::Key::Tab);
    h.run_steps(2);
    assert_eq!(color(&h), "#ff8800");
}

#[test]
fn hex_field_takes_every_paste_with_or_without_hash() {
    let mut h = picker("#336699");
    for (paste, want) in [("ff8800", "#ff8800"), ("#00ff00", "#00ff00"), (" 1234AB\n", "#1234ab")] {
        h.event(egui::Event::Paste(paste.into()));
        h.run_steps(2);
        assert_eq!(color(&h), want, "paste {paste:?}");
        assert_eq!(h.get_by_role(Role::TextInput).value().as_deref(), Some(&want[1..].to_ascii_uppercase()[..]));
    }
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.run_steps(1);
    h.event(egui::Event::Paste("ff".into()));
    h.run_steps(2);
    assert_eq!(h.get_by_role(Role::TextInput).value().as_deref(), Some("ff"));
    assert_eq!(color(&h), "#1234ab");
}

#[test]
fn color_picker_opens_with_hex_focused_and_selected() {
    let mut h = picker("#336699");
    let hex_field = h.get_by_role(Role::TextInput);
    assert!(hex_field.is_focused(), "the hex field automatically has focus on open");
    // Typing directly replaces the selected hex digits without manual clicking or Command+A.
    type_keys(&mut h, "ff8800");
    assert_eq!(color(&h), "#ff8800", "typing replaces the selected hex code");
}

#[test]
fn color_picker_dialog_hex_field_is_in_tab_order_and_confirms_typed_color() {
    let app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let mut h = Harness::builder().with_size(vec2(760.0, 480.0)).build_ui_state(|ui, app| crate::dialogs::show(app, ui.ctx()), app);
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    open(h.state_mut(), "foreground");
    h.run_steps(3);
    assert!(h.get_by_role(Role::TextInput).is_focused(), "hex field has focus in dialog frame");
    for ch in "ff8800".chars() {
        h.event(egui::Event::Text(ch.to_string()));
        h.run_steps(1);
    }
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
    assert_eq!(hex([h.state().session.tools.foreground[0], h.state().session.tools.foreground[1], h.state().session.tools.foreground[2]]), "#ff8800");
}

#[test]
fn alt_reset_restores_orig_color() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.tools.foreground = [0.2, 0.4, 0.6, 1.0];
    let id = open(&mut app, "foreground");
    let f = &mut app.ui.dialog_mut(id).unwrap().fields;
    f.insert("color".into(), json!("#ff0000"));
    reset(f);
    assert_eq!(f["color"], json!(hex([0.2, 0.4, 0.6])));
    assert_eq!(f["__orig"], json!(hex([0.2, 0.4, 0.6])));
}

#[test]
fn lab_and_cmyk_fields_take_multi_digit_values() {
    let base = parse_hex("#336699").unwrap();
    let lab = photocraft_color::convert::srgb_to_lab(base);
    let cmyk = photocraft_color::convert::rgb_to_cmyk(base);
    // Fields in order: H S B R G B, L a b, C M Y K.
    for (field, typed, want) in [
        (6, "60", photocraft_color::convert::lab_to_srgb([60.0, lab[1], lab[2]])),
        (7, "30", photocraft_color::convert::lab_to_srgb([lab[0], 30.0, lab[2]])),
        (9, "40", photocraft_color::convert::cmyk_to_rgb([0.4, cmyk[1], cmyk[2], cmyk[3]])),
    ] {
        let mut h = picker("#336699");
        h.query_all_by_role(Role::SpinButton).nth(field).unwrap().click();
        h.run_steps(1);
        type_keys(&mut h, typed);
        h.key_press(egui::Key::Tab);
        h.run_steps(2);
        assert_eq!(color(&h), hex(want), "field {field} <- {typed}");
    }
}

#[test]
fn picker_places_actions_above_two_columns_and_web_colors_under_the_field() {
    let h = picker("#336699");
    let inputs: Vec<_> = h.query_all_by_role(Role::SpinButton).map(|n| n.rect()).collect();
    assert_eq!(inputs.len(), 13);
    let (hue, red, lightness, cyan, black) = (inputs[0], inputs[3], inputs[6], inputs[9], inputs[12]);
    assert!(lightness.left() > hue.right(), "Lab is beside HSB, not below RGB");
    assert!((lightness.top() - hue.top()).abs() < 1.0);
    assert!((cyan.top() - red.top()).abs() < 1.0, "CMYK and RGB start at the same height");
    let ok = h.get_by_label("OK").rect();
    let cancel = h.get_by_label("Cancel").rect();
    assert!(ok.left() > hue.left() && ok.bottom() < hue.top(), "actions are above the values and beside the preview: {ok:?}, {hue:?}");
    assert!((ok.left() - cancel.left()).abs() < 1.0 && cancel.top() > ok.bottom());
    assert_eq!(h.query_all_by_label("OK").count(), 1);
    let web = h.get_by_label("Only Web Colors").rect();
    assert!(web.left() < hue.left() && web.top() > hue.bottom());
    // The full dialog should fit even in PhotoCraft's minimum 760 x 480 window.
    assert!(black.bottom() - ok.top() < 360.0);
    assert!(black.right() - web.left() < 600.0);
    let hex = h.get_by_role(Role::TextInput).rect();
    assert!(hex.left() < lightness.left() && hex.top() > inputs[5].bottom());
}

/// The whole Color Picker window (title, frame and all) is no bigger than Photoshop's: about
/// 534 x 376 points (Photoshop 27.11 on macOS, measured from a screenshot at the same scale as
/// PhotoCraft's, which was 682 x 438).
#[test]
fn the_picker_window_is_no_bigger_than_photoshops() {
    let mut h = Harness::builder().with_size(vec2(1400.0, 900.0)).with_max_steps(64).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        PhotocraftApp::new(photocraft_engine::Session::new(), Default::default())
    });
    h.run_steps(4);
    let id = open(h.state_mut(), "foreground");
    h.run_steps(6);
    let rect = h.ctx.memory(|m| m.area_rect(egui::Id::unique(("dialog", id)))).expect("the picker window");
    assert!(rect.width() <= 534.0 && rect.height() <= 376.0, "Color Picker window {} x {} points, Photoshop's 534 x 376", rect.width(), rect.height());
    // Everything still shows: the three actions and all 13 component fields.
    for label in ["OK", "Cancel", "Add to Swatches", "Only Web Colors"] {
        assert!(rect.contains_rect(h.get_by_label(label).rect()), "{label} inside the window");
    }
    assert_eq!(h.query_all_by_role(Role::SpinButton).filter(|n| rect.contains_rect(n.rect())).count(), 13);
    // The actions are as wide as the longest label ("Add to Swatches" is about 133 points in
    // the button font), not stretched across the column (160). Photoshop's are about 116.
    let widths: Vec<f32> = ["OK", "Cancel", "Add to Swatches"].iter().map(|l| h.get_by_label(l).rect().width()).collect();
    assert!(widths.iter().all(|w| (w - widths[0]).abs() < 0.5), "one width for all three actions: {widths:?}");
    assert!(widths[0] <= 136.0, "actions {} points wide, Photoshop's about 116", widths[0]);
}

#[test]
fn web_colors_and_current_swatch_still_update_the_picker() {
    let mut h = picker("#123456");
    h.get_by_label("Only Web Colors").click();
    h.run_steps(2);
    assert_eq!(color(&h), "#003366");
    assert_eq!(h.get_by_role(Role::TextInput).value().as_deref(), Some("003366"), "hex field follows the snap while it has focus");
    h.get_by_label("Only Web Colors").click();
    h.run_steps(2);
    h.get_by_label("Click to restore the current colour").click();
    h.run_steps(2);
    assert_eq!(color(&h), "#123456");
}

/// Only Web Colors snaps hex, field and component edits to web-safe colours.
#[test]
fn web_only_snaps_hex_field_and_component_edits() {
    let mut f = Map::new();
    f.insert("color".into(), json!("#123456"));
    f.insert("__webOnly".into(), json!(true));
    set_rgb(&mut f, parse_hex("#ff8800").unwrap(), Keep::Nothing);
    assert_eq!(f["color"], json!("#ff9900"));
    set_rgb(&mut f, hsb_to_rgb([40.0, 1.0, 1.0]), Keep::Hsv([40.0, 1.0, 1.0]));
    assert_eq!(f["color"], json!("#ff9900"));
    assert!(f.get("__hsv").is_none(), "kept HSV does not outlive a web snap");
    set_rgb(&mut f, photocraft_color::convert::lab_to_srgb([50.0, 40.0, 20.0]).map(|c| c.clamp(0.0, 1.0)), Keep::Lab([50.0, 40.0, 20.0]));
    assert_eq!(not_web_safe(parse_hex(f["color"].as_str().unwrap()).unwrap()), None);
    assert!(f.get("__lab").is_none());
}

#[test]
fn web_only_survives_reopen_and_other_picker_targets() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.ui.color_panel.web_only = true;
    let id = open(&mut app, "foreground");
    let f = &app.ui.dialog_mut(id).unwrap().fields;
    assert_eq!(f["__webOnly"], json!(true));
    assert_eq!(f["color"], json!(hex(web_safe([app.session.tools.foreground[0], app.session.tools.foreground[1], app.session.tools.foreground[2]]))));
    assert_eq!(f["__orig"], f["color"], "the opening snap moves __orig too, so OK with no edit is unchanged");
    app.ui.dialogs.clear();
    let id = open_for_target(&mut app, "noteColor", "Color Picker", [1.0, 0.53, 0.0]);
    let f = &app.ui.dialogs.iter().find(|d| d.id == id).unwrap().fields;
    assert_eq!(f["__webOnly"], json!(true));
    assert_eq!(f["color"], json!("#ff9900"));
    assert_eq!(f["__orig"], json!("#ff9900"), "a non-web-safe start colour is not committed by opening the picker");
}

#[test]
fn sample_at_snaps_when_only_web_colors_is_on() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let id = open(&mut app, "foreground");
    let f = &mut app.ui.dialog_mut(id).unwrap().fields;
    f.insert("__webOnly".into(), json!(true));
    set_rgb(f, [0.07, 0.2, 0.34], Keep::Nothing);
    assert_eq!(f["color"], json!("#003366"));
}

#[test]
fn side_buttons_and_keyboard_confirm_or_cancel_without_changing_the_other_color() {
    for target in ["foreground", "background"] {
        for action in ["OK", "Cancel", "Enter", "Escape"] {
            let app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
            let mut h = Harness::builder().with_size(vec2(760.0, 480.0)).build_ui_state(|ui, app| crate::dialogs::show(app, ui.ctx()), app);
            PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
            let original_fg = h.state().session.tools.foreground;
            let original_bg = h.state().session.tools.background;
            let id = open(h.state_mut(), target);
            h.state_mut().ui.dialog_mut(id).unwrap().fields.insert("color".into(), json!("#336699"));
            h.run_steps(4);
            let screen = Rect::from_min_size(pos2(0.0, 0.0), vec2(760.0, 480.0));
            for node in h.query_all_by_role(Role::SpinButton) {
                assert!(screen.contains_rect(node.rect()), "numeric inputs fit the minimum window");
            }
            for label in ["OK", "Cancel", "Only Web Colors"] {
                assert!(screen.contains_rect(h.get_by_label(label).rect()), "{label} fits the minimum window");
            }
            match action {
                "Enter" => h.key_press(egui::Key::Enter),
                "Escape" => h.key_press(egui::Key::Escape),
                _ => h.get_by_label(action).click(),
            }
            h.run_steps(3);
            assert!(h.state().ui.dialogs.is_empty(), "{target}: {action} closes the picker");
            let confirm = matches!(action, "OK" | "Enter");
            let tools = &h.state().session.tools;
            if confirm && target == "foreground" {
                assert_eq!(hex([tools.foreground[0], tools.foreground[1], tools.foreground[2]]), "#336699");
            } else {
                assert_eq!(tools.foreground, original_fg);
            }
            if confirm && target == "background" {
                assert_eq!(hex([tools.background[0], tools.background[1], tools.background[2]]), "#336699");
            } else {
                assert_eq!(tools.background, original_bg);
            }
        }
    }
}

/// Hex parsing refuses text whose bytes split a character instead of panicking.
#[test]
fn parse_hex_rejects_multibyte_text() {
    // Six bytes, three characters: the byte range 0..2 splits "é", which used to panic.
    assert_eq!(parse_hex("aé€"), None);
    assert_eq!(parse_hex("#ff880"), None);
}

/// The shared HSB conversions give bit-identical output to the picker's old copies.
#[test]
fn shared_hsb_output_matches_the_old_picker_conversions() {
    /// The picker's HSB to sRGB before it used photocraft-color.
    fn old_hsv_to_rgb(h: f32, s: f32, v: f32) -> [f32; 3] {
        let h = (h.rem_euclid(360.0)) / 60.0;
        let c = v * s;
        let x = c * (1.0 - (h % 2.0 - 1.0).abs());
        let (r, g, b) = match h as u32 {
            0 => (c, x, 0.0),
            1 => (x, c, 0.0),
            2 => (0.0, c, x),
            3 => (0.0, x, c),
            4 => (x, 0.0, c),
            _ => (c, 0.0, x),
        };
        let m = v - c;
        [r + m, g + m, b + m]
    }
    /// The picker's sRGB to HSB before it used photocraft-color.
    fn old_rgb_to_hsv(c: [f32; 3]) -> [f32; 3] {
        let (mx, mn) = (c[0].max(c[1]).max(c[2]), c[0].min(c[1]).min(c[2]));
        let d = mx - mn;
        let h = if d <= 0.0 {
            0.0
        } else if mx == c[0] {
            60.0 * ((c[1] - c[2]) / d).rem_euclid(6.0)
        } else if mx == c[1] {
            60.0 * ((c[2] - c[0]) / d + 2.0)
        } else {
            60.0 * ((c[0] - c[1]) / d + 4.0)
        };
        [h, if mx <= 0.0 { 0.0 } else { d / mx }, mx]
    }
    let bits = |c: [f32; 3]| c.map(f32::to_bits);
    for h in (0..=720).map(|i| i as f32 * 0.5) {
        for s in (0..=20).map(|i| i as f32 / 20.0) {
            for v in (0..=20).map(|i| i as f32 / 20.0) {
                assert_eq!(bits(hsb_to_rgb([h, s, v])), bits(old_hsv_to_rgb(h, s, v)), "{h} {s} {v}");
            }
        }
    }
    for r in 0..=32 {
        for g in 0..=32 {
            for b in 0..=32 {
                let c = [r as f32 / 32.0, g as f32 / 32.0, b as f32 / 32.0];
                assert_eq!(bits(rgb_to_hsb(c)), bits(old_rgb_to_hsv(c)), "{c:?}");
            }
        }
    }
}

/// HSB and hex round-trip sRGB colours.
#[test]
fn parse_hex_rejects_signs() {
    assert_eq!(parse_hex("#+f+f+f"), None);
    assert_eq!(parse_hex("+1+2+3"), None);
    assert!(parse_hex("#0f0f0f").is_some());
}

#[test]
fn hsv_round_trips() {
    for c in [[1.0, 0.0, 0.0], [0.2, 0.6, 0.4], [0.5, 0.5, 0.5], [0.0, 0.0, 1.0], [0.9, 0.8, 0.1]] {
        let h = rgb_to_hsb(c);
        let back = hsb_to_rgb(h);
        for i in 0..3 {
            assert!((back[i] - c[i]).abs() < 1e-5, "{c:?} -> {h:?} -> {back:?}");
        }
    }
    assert_eq!(hex([1.0, 0.5, 0.0]), "#ff8000");
    assert_eq!(parse_hex("#FF8000").map(hex).as_deref(), Some("#ff8000"));
}

/// Locating a colour on the field and reading the field back agree in every mode.
#[test]
fn field_and_locate_are_inverse_in_every_mode() {
    let rgb = [0.8, 0.3, 0.2];
    let (hsv, lab) = (rgb_to_hsb(rgb), photocraft_color::convert::srgb_to_lab(rgb));
    for (m, _) in MODES.iter().chain(LAB_MODES) {
        let (x, y, z) = locate(m, hsv, rgb, lab);
        let c = field_color(m, z, x, y);
        assert_eq!(hex(c), hex(rgb), "mode {m}");
    }
}

#[test]
fn lab_fields_are_laid_out_like_photoshops() {
    // Photoshop 25.4. L: a across (green to red), b upwards (blue to yellow).
    let l = |x, y| photocraft_color::convert::srgb_to_lab(field_color("lab_l", 0.6, x, y));
    assert!(l(0.0, 0.5)[1] < -40.0 && l(1.0, 0.5)[1] > 40.0, "{:?} {:?}", l(0.0, 0.5), l(1.0, 0.5));
    assert!(l(0.5, 1.0)[2] < -40.0 && l(0.5, 0.0)[2] > 40.0, "{:?} {:?}", l(0.5, 1.0), l(0.5, 0.0));
    // a: b across, L upwards (white at the top, black at the bottom).
    assert!(field_color("lab_a", 0.5, 0.5, 0.0).iter().all(|&c| c > 0.99), "white at the top");
    assert!(field_color("lab_a", 0.5, 0.5, 1.0).iter().all(|&c| c < 0.01), "black at the bottom");
    let a = |x| photocraft_color::convert::srgb_to_lab(field_color("lab_a", 0.5, x, 0.5));
    assert!(a(0.0)[2] < -40.0 && a(1.0)[2] > 40.0);
    // b: a across, L upwards; its slider runs from yellow (top) to blue.
    let b = |x| photocraft_color::convert::srgb_to_lab(field_color("lab_b", 0.5, x, 0.5));
    assert!(b(0.0)[1] < -40.0 && b(1.0)[1] > 40.0);
    let lab = [60.0, 0.0, 0.0];
    let top = photocraft_color::convert::srgb_to_lab(slider_color("lab_b", 1.0, [0.0; 3], [0.0; 3], lab));
    let bottom = photocraft_color::convert::srgb_to_lab(slider_color("lab_b", 0.0, [0.0; 3], [0.0; 3], lab));
    assert!(top[2] > 40.0 && bottom[2] < -40.0, "{top:?} {bottom:?}");
}

#[test]
fn gamut_and_web_warnings() {
    // Photoshop 25.4: #714343 is printable but not a web colour; #fd0303 is neither.
    let rgb = |h| parse_hex(h).unwrap();
    assert_eq!(out_of_gamut(rgb("#714343")), None);
    assert!(out_of_gamut(rgb("#fd0303")).is_some());
    assert_eq!(out_of_gamut(rgb("#000000")), None, "black prints");
    assert_eq!(out_of_gamut(rgb("#ffffff")), None);
    assert_eq!(not_web_safe(rgb("#714343")).map(hex).as_deref(), Some("#663333"));
    assert_eq!(not_web_safe(rgb("#cc3366")), None);
}

#[test]
fn hue_survives_grey_and_ok_sets_the_colour() {
    let mut f = Map::new();
    f.insert("color".into(), json!("#808080"));
    f.insert("__hsv".into(), json!([200.0, 0.0, 128.0 / 255.0]));
    assert_eq!(current(&f).hsv[0], 200.0);
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    let id = open(&mut app, "background");
    let d = app.ui.dialog_mut(id).unwrap();
    d.fields.insert("color".into(), json!("#3366cc"));
    let fields = d.fields.clone();
    confirm(&mut app, &fields).unwrap();
    assert_eq!(hex([app.session.tools.background[0], app.session.tools.background[1], app.session.tools.background[2]]), "#3366cc");
}

/// #4196: CMYK values typed into the picker survive OK, so the colour fills a CMYK document with those inks and
/// reopening the picker shows them again.
#[test]
fn cmyk_values_survive_ok_and_a_reopen() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    let cyan = [1.0, 0.0, 0.0, 0.0];
    let id = open(&mut app, "foreground");
    let d = app.ui.dialog_mut(id).unwrap();
    set_rgb(&mut d.fields, photocraft_color::convert::cmyk_to_rgb(cyan).map(|c| c.clamp(0.0, 1.0)), Keep::Cmyk(cyan));
    let fields = d.fields.clone();
    confirm(&mut app, &fields).unwrap();
    let fg = app.session.tools.foreground;
    assert_eq!(app.session.tools.picked_cmyk([fg[0], fg[1], fg[2]]), Some(cyan));
    let id = open(&mut app, "foreground");
    let again = current(&app.ui.dialogs.iter().find(|d| d.id == id).unwrap().fields);
    assert_eq!(again.cmyk, cyan);
}

fn dialog_color(app: &PhotocraftApp, id: u64) -> &str {
    app.ui.dialogs.iter().find(|d| d.id == id).and_then(|d| d.fields.get("color")).and_then(Value::as_str).unwrap()
}

#[test]
fn the_top_picker_samples_the_image() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 40, "height": 20, "background": "transparent"})).unwrap();
    app.run("shape.create", json!({"kind": "rect", "rect": [0, 0, 20, 20], "fill": "#ff0000"})).unwrap();
    app.run("shape.create", json!({"kind": "rect", "rect": [20, 0, 10, 20], "fill": "#00ff00"})).unwrap();
    let id = open(&mut app, "foreground");
    assert_eq!(top(&app), Some(id));
    sample_at(&mut app, 25.5, 5.5);
    assert_eq!(dialog_color(&app, id), "#00ff00");
    assert_eq!(app.ui.dialog_mut(id).unwrap().fields["__orig"], json!("#000000"), "the current colour stays");
    // Off the image or over transparency nothing changes.
    for (x, y) in [(35.0, 5.0), (-3.0, 5.0), (5.0, 20.0), (1e12, -1e12), (f64::NAN, f64::INFINITY)] {
        sample_at(&mut app, x, y);
        assert_eq!(dialog_color(&app, id), "#00ff00", "({x}, {y})");
    }

    // Agents: `ui.pointer` samples into the picker instead of driving the tool.
    app.ui.tool = crate::state::Tool::Brush;
    let rev = app.session.active().unwrap().revision;
    let ctx = egui::Context::default();
    let events = json!([{"kind": "down", "x": 5, "y": 5}, {"kind": "up", "x": 5, "y": 5}]);
    let (req, _rx) = crate::control::ControlRequest::new("ui.pointer", json!({"events": events}));
    let _ = crate::control::handle(&mut app, &ctx, &req);
    assert_eq!(dialog_color(&app, id), "#ff0000");
    assert_eq!(app.session.active().unwrap().revision, rev, "the Brush must not paint");
    assert_eq!(app.session.tools.foreground, [0.0, 0.0, 0.0, 1.0], "only OK sets the foreground");

    // A dialog opened over the picker takes the input.
    let about = app.ui.open_dialog(DialogKind::About, Map::new());
    assert_eq!(top(&app), None);
    sample_at(&mut app, 25.0, 5.0);
    assert_eq!(dialog_color(&app, id), "#ff0000");
    app.ui.close_dialog(about);
    sample_at(&mut app, 25.0, 5.0);
    assert_eq!(dialog_color(&app, id), "#00ff00");
}
