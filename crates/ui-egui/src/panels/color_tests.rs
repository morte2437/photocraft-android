use super::*;

#[test]
/// #3323: the Color panel's HSB is Photoshop's (and the Color Picker's), on sRGB values, not
/// egui's linear-light HSV: a 50% grey has Brightness 50%, and the field's left edge halfway
/// down is that grey (128), not 188.
fn color_panel_hsb_is_srgb_like_photoshop() {
    let grey = srgb_hsva([128.0 / 255.0, 128.0 / 255.0, 128.0 / 255.0, 1.0]);
    assert!((grey.v - 128.0 / 255.0).abs() < 1e-3 && grey.s < 1e-3, "{grey:?}");
    let mid = hsva_srgb(egui::ecolor::Hsva::new(0.3, 0.0, 0.5, 1.0));
    assert!(mid[..3].iter().all(|v| (v - 0.5).abs() <= 1.0 / 255.0), "{mid:?}");
    // What the field paints there.
    assert_eq!(hsva_color32(egui::ecolor::Hsva::new(0.3, 0.0, 0.5, 1.0)), Color32::from_gray(128));
    // The same numbers as the Color Picker dialog for a saturated colour.
    let c = [0.847, 0.271, 0.180];
    let (ours, dialog) = (srgb_hsva([c[0], c[1], c[2], 1.0]), photocraft_color::convert::rgb_to_hsb(c));
    assert!((ours.h * 360.0 - dialog[0]).abs() < 0.5 && (ours.s - dialog[1]).abs() < 1e-3 && (ours.v - dialog[2]).abs() < 1e-3, "{ours:?} vs {dialog:?}");
}

#[test]
fn srgb_hsva_roundtrip() {
    for c in [[0.847, 0.271, 0.180, 1.0], [0.0, 0.0, 0.0, 1.0], [1.0, 1.0, 1.0, 1.0], [0.2, 0.6, 0.4, 1.0]] {
        let back = hsva_srgb(srgb_hsva(c));
        for i in 0..3 {
            assert!((back[i] - c[i]).abs() <= 1.0 / 255.0, "{c:?} -> {back:?}");
        }
    }
}

/// #7793: the Color panel's field and hue strip pick on the frame the button goes down (no
/// release, no time passing) and follow moves smaller than egui's 6-point click threshold.
#[test]
fn color_panel_field_and_hue_strip_pick_on_press() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.tools.foreground = [0.2, 0.4, 0.6, 1.0];
    let mut h = egui_kittest::Harness::builder().with_size(vec2(300.0, 300.0)).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            let at = (ui.cursor().min, ui.available_width());
            ui.data_mut(|d| d.insert_temp(egui::Id::unique("test-origin"), at));
            color_field(app, ui);
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::Studio);
    h.run_steps(2);
    let (o, w) = h.ctx.data(|d| d.get_temp::<(Pos2, f32)>(egui::Id::unique("test-origin"))).unwrap();
    let field_w = w - 38.0 - 14.0 - 16.0;
    let press = |h: &mut egui_kittest::Harness<'static, PhotocraftApp>, at: Pos2, pressed: bool| {
        h.event(egui::Event::PointerMoved(at));
        h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE });
        h.run_steps(1);
    };
    let at = o + vec2(38.0 + 8.0 + field_w * 0.5, 60.0);
    press(&mut h, at, true);
    let picked = h.state().session.tools.foreground;
    assert_ne!(picked, [0.2, 0.4, 0.6, 1.0], "the press itself picks");
    let hsva = srgb_hsva(picked);
    assert!((hsva.s - 0.5).abs() < 0.03 && (hsva.v - 0.5).abs() < 0.03, "{hsva:?}");
    h.event(egui::Event::PointerMoved(at + vec2(3.0, 0.0)));
    h.run_steps(1);
    assert_ne!(h.state().session.tools.foreground, picked, "a small drag adjusts the pick");
    press(&mut h, at + vec2(3.0, 0.0), false);
    h.run_steps(1);
    // Hue strip: a press a third of the way down is a hue of about 240° at once.
    let before = h.state().session.tools.foreground;
    press(&mut h, o + vec2(38.0 + 8.0 + field_w + 8.0 + 7.0, 40.0), true);
    let now = h.state().session.tools.foreground;
    assert_ne!(now, before, "the strip picks on press");
    assert!((srgb_hsva(now).h * 360.0 - 240.0).abs() < 8.0, "{:?}", srgb_hsva(now));
}

/// Only the primary button picks: a right or middle press (and its release) on the field or
/// the hue strip leaves the foreground colour alone.
#[test]
fn color_panel_ignores_secondary_and_middle_presses() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.tools.foreground = [0.2, 0.4, 0.6, 1.0];
    let mut h = egui_kittest::Harness::builder().with_size(vec2(300.0, 300.0)).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            let at = (ui.cursor().min, ui.available_width());
            ui.data_mut(|d| d.insert_temp(egui::Id::unique("test-origin"), at));
            color_field(app, ui);
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::Studio);
    h.run_steps(2);
    let (o, w) = h.ctx.data(|d| d.get_temp::<(Pos2, f32)>(egui::Id::unique("test-origin"))).unwrap();
    let field_w = w - 38.0 - 14.0 - 16.0;
    let field = o + vec2(38.0 + 8.0 + field_w * 0.5, 60.0);
    let strip = o + vec2(38.0 + 8.0 + field_w + 8.0 + 7.0, 40.0);
    for at in [field, strip] {
        for button in [egui::PointerButton::Secondary, egui::PointerButton::Middle] {
            for pressed in [true, false] {
                h.event(egui::Event::PointerMoved(at));
                h.event(egui::Event::PointerButton { pos: at, button, pressed, modifiers: egui::Modifiers::NONE });
                h.run_steps(1);
                assert_eq!(h.state().session.tools.foreground, [0.2, 0.4, 0.6, 1.0], "{button:?} pressed={pressed} at {at:?}");
            }
        }
    }
}

/// Click the hex field, select its text and type `text` one key per frame.
fn type_hex(h: &mut egui_kittest::Harness<'static, PhotocraftApp>, text: &str) {
    use egui_kittest::kittest::Queryable;
    h.get_by_role(egui::accesskit::Role::TextInput).click();
    h.run_steps(1);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.run_steps(1);
    for ch in text.chars() {
        h.event(egui::Event::Text(ch.to_string()));
        h.run_steps(1);
    }
}

/// The Color panel's hex readout is editable, with or without the leading `#`.
#[test]
fn color_panel_hex_field_takes_a_typed_hex() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.tools.foreground = [1.0, 1.0, 1.0, 1.0];
    let mut h = egui_kittest::Harness::builder().with_size(vec2(300.0, 300.0)).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            if ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                color_field(app, ui);
            }
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::Studio);
    h.run_steps(2);
    type_hex(&mut h, "003300");
    assert_eq!(srgb_bytes(h.state().session.tools.foreground), [0x00, 0x33, 0x00]);
    // A leading '#' is accepted too.
    type_hex(&mut h, "#ff8000");
    assert_eq!(srgb_bytes(h.state().session.tools.foreground), [0xff, 0x80, 0x00]);
}

/// Themes with `round_chips` (Studio) draw round chips, stacked or side by side in two tool
/// columns, with Switch Colors right under them and Default Colors at the top-right corner
/// (stacked) or the left edge (side by side); both buttons work in either layout.
#[test]
fn studio_color_chips_keep_default_and_swap_buttons_below() {
    use egui_kittest::kittest::Queryable;
    for (kind, two_columns) in [
        (crate::theme::ThemeKind::Studio, false),
        (crate::theme::ThemeKind::StudioLight, false),
        (crate::theme::ThemeKind::Studio, true),
        (crate::theme::ThemeKind::StudioLight, true),
    ] {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.session.tools.foreground = [1.0, 0.0, 0.0, 1.0];
        app.session.tools.background = [0.0, 0.0, 1.0, 1.0];
        let mut h = egui_kittest::Harness::builder().with_size(vec2(80.0, 120.0)).build_ui_state(
            |ui, app: &mut PhotocraftApp| {
                let w = ui.available_width();
                color_chips(app, ui, two_columns, w)
            },
            app,
        );
        PhotocraftApp::setup_context(&h.ctx, kind);
        h.run_steps(2);
        h.get_by_label("Swap colours (X)").click();
        h.run_steps(2);
        assert_eq!(h.state().session.tools.foreground, [0.0, 0.0, 1.0, 1.0], "{kind:?}");
        assert_eq!(h.state().session.tools.background, [1.0, 0.0, 0.0, 1.0], "{kind:?}");
        h.get_by_label("Default colours (D)").click();
        h.run_steps(2);
        assert_eq!(h.state().session.tools.foreground, [0.0, 0.0, 0.0, 1.0], "{kind:?}");
        assert_eq!(h.state().session.tools.background, [1.0, 1.0, 1.0, 1.0], "{kind:?}");
    }
}

/// While characters are highlighted in a type layer being edited, the Switch Colors and
/// Default Colors buttons recolour them with the new foreground (#5690); without a selection
/// they only change the tool colours, as before.
#[test]
fn swap_and_default_colors_recolour_selected_type() {
    use egui_kittest::kittest::Queryable;
    use photocraft_doc::LayerId;
    let rgb = |app: &PhotocraftApp, id: LayerId, ci: usize| {
        let doc = &app.session.active().unwrap().doc;
        let photocraft_doc::LayerContent::Text(t) = &doc.layer(id).unwrap().content else { panic!("not a type layer") };
        let b = t.text.char_indices().nth(ci).map_or(t.text.len(), |(b, _)| b);
        let mut at = 0;
        for r in t.char_runs() {
            if b < at + r.len {
                return r.style.color.to_rgba8();
            }
            at += r.len;
        }
        [0; 4]
    };
    for selected in [true, false] {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
        let made = app.run("type.create", json!({"text": "Hello world", "size": 40, "x": 50, "y": 150, "color": "#000000"})).unwrap();
        let id = LayerId(made["layer"].as_u64().unwrap());
        app.run("tools.setColors", json!({"foreground": "#ff0000", "background": "#0000ff"})).unwrap();
        let anchor = if selected { 6 } else { 11 };
        app.ui.text_edit = Some(crate::state::TextEdit {
            layer: id.0,
            document: None,
            caret: 11,
            anchor,
            session: "s".into(),
            created: false,
            mask: false,
            dragging: false,
            resize: None,
            preedit: None,
        });
        let mut h = egui_kittest::Harness::builder().with_size(vec2(80.0, 120.0)).build_ui_state(
            |ui, app: &mut PhotocraftApp| {
                let w = ui.available_width();
                color_chips(app, ui, false, w)
            },
            app,
        );
        PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::Studio);
        h.run_steps(2);
        h.get_by_label("Swap colours (X)").click();
        h.run_steps(2);
        assert_eq!(h.state().session.tools.foreground, [0.0, 0.0, 1.0, 1.0]);
        assert_eq!(h.state().session.tools.background, [1.0, 0.0, 0.0, 1.0]);
        let blue = if selected { [0, 0, 255, 255] } else { [0, 0, 0, 255] };
        assert_eq!(rgb(h.state(), id, 0), [0, 0, 0, 255], "unselected characters keep their colour");
        assert_eq!((rgb(h.state(), id, 6), rgb(h.state(), id, 10)), (blue, blue), "selected: {selected}");
        // Default Colors: the selection takes black, the new foreground.
        h.state_mut().run("tools.setColors", json!({"foreground": "#00ff00"})).unwrap();
        h.get_by_label("Default colours (D)").click();
        h.run_steps(2);
        assert_eq!(h.state().session.tools.foreground, [0.0, 0.0, 0.0, 1.0]);
        assert_eq!(rgb(h.state(), id, 6), [0, 0, 0, 255], "selected: {selected}");
    }
}

/// The Studio layout (chips stacked, icons below) fits the height the toolbar reserves for
/// the chips, so the two-column check does not change with the theme.
#[test]
fn studio_color_chips_fit_the_reserved_height() {
    // Chips, then the row holding the Switch Colors curve.
    assert!(STUDIO_CHIP.0 + STUDIO_CHIP.1 + 8.0 + 4.0 <= chips_height(false, true, false));
    // Side by side: the chips plus the same row, and shorter than stacked.
    assert!(STUDIO_CHIP.0 + 8.0 + 4.0 <= chips_height(false, true, true));
    assert!(chips_height(false, true, true) < chips_height(false, true, false));
    // The Studio block is taller than Photoshop's, and the two-column check knows it.
    assert!(chips_height(false, true, false) > chips_height(false, false, false));
}

/// An incomplete entry never changes the colour and the field snaps back when focus leaves.
#[test]
fn color_panel_hex_field_ignores_partial_input() {
    use egui_kittest::kittest::Queryable;
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.tools.foreground = [0.2, 0.4, 0.6, 1.0];
    let mut h = egui_kittest::Harness::builder().with_size(vec2(300.0, 300.0)).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            if ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                color_field(app, ui);
            }
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::Studio);
    h.run_steps(2);
    let fg = h.state().session.tools.foreground;
    h.get_by_role(egui::accesskit::Role::TextInput).click();
    h.run_steps(1);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.run_steps(1);
    for ch in "12zz".chars() {
        h.event(egui::Event::Text(ch.to_string()));
        h.run_steps(1);
    }
    assert_eq!(h.state().session.tools.foreground, fg, "invalid input changes nothing");
    h.key_press(egui::Key::Tab);
    h.run_steps(2);
    assert_eq!(h.get_by_role(egui::accesskit::Role::TextInput).value().as_deref(), Some("336699"), "snaps back to the colour");
}

/// The R, G and B fields next to the hex are editable, like the hex field.
#[test]
fn color_readout_rgb_fields_take_values() {
    use egui_kittest::kittest::Queryable;
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.tools.foreground = [0.0, 0.0, 0.0, 1.0];
    let mut h = field_harness(app);
    // The only spin buttons are R, G and B, in that order.
    h.query_all_by_role(egui::accesskit::Role::SpinButton).nth(1).unwrap().click();
    h.run_steps(1);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.run_steps(1);
    for ch in "128".chars() {
        h.event(egui::Event::Text(ch.to_string()));
        h.run_steps(1);
    }
    h.key_press(egui::Key::Tab);
    h.run_steps(2);
    assert_eq!(srgb_bytes(h.state().session.tools.foreground), [0x00, 0x80, 0x00]);
}

/// Typing a value into the R, G or B field recolours a selected shape as one undo step per
/// field, not one per keystroke.
#[test]
fn typing_an_rgb_value_recolours_the_selected_shape_in_one_step() {
    use egui_kittest::kittest::Queryable;
    use photocraft_doc::{Fill, LayerContent, LayerId};
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 64, "height": 48})).unwrap();
    let id = LayerId(app.run("shape.create", json!({"rect": [8, 8, 40, 30], "fill": "#000000"})).unwrap()["layer"].as_u64().unwrap());
    app.ui.tool = crate::state::Tool::Rectangle;
    app.session.tools.foreground = [0.0, 0.0, 0.0, 1.0];
    let mut h = field_harness(app);
    let fill = |app: &PhotocraftApp| match &app.session.active().unwrap().doc.layer(id).unwrap().content {
        LayerContent::Shape(sh) => match &sh.fill {
            Some(Fill::Solid(c)) => srgb_bytes([c.c[0], c.c[1], c.c[2], 1.0]),
            _ => panic!("a solid fill"),
        },
        _ => panic!("not a shape layer"),
    };
    let steps = |app: &PhotocraftApp| app.session.active().unwrap().history.entries().len();
    let before = steps(h.state());
    // The only spin buttons are R, G and B, in that order.
    for (field, text, steps_after, want) in [(1, "128", 1, [0x00, 0x80, 0x00]), (2, "64", 2, [0x00, 0x80, 0x40])] {
        h.query_all_by_role(egui::accesskit::Role::SpinButton).nth(field).unwrap().click();
        h.run_steps(1);
        h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
        h.run_steps(1);
        for ch in text.chars() {
            h.event(egui::Event::Text(ch.to_string()));
            h.run_steps(1);
        }
        h.key_press(egui::Key::Tab);
        h.run_steps(2);
        assert_eq!(fill(h.state()), want, "{text}");
        assert_eq!(steps(h.state()), before + steps_after, "{text}: one step for the typed value");
    }
    // Typing into B again and clicking the field while B still has the focus: two steps, the
    // click is not part of the typed value.
    h.query_all_by_role(egui::accesskit::Role::SpinButton).nth(2).unwrap().click();
    h.run_steps(1);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.run_steps(1);
    h.event(egui::Event::Text("9".into()));
    h.run_steps(1);
    assert_eq!(fill(h.state()), [0x00, 0x80, 0x09]);
    assert_eq!(steps(h.state()), before + 3, "the typed 9");
    let min = h.ctx.input(|i| i.viewport_rect()).min;
    click(&mut h, min + vec2(60.0, 10.0));
    assert_eq!(steps(h.state()), before + 4, "the click after typing");
    // A sum applies when Enter commits it, after the field let go of the focus: still the
    // step of the "2" typed before it.
    h.query_all_by_role(egui::accesskit::Role::SpinButton).next().unwrap().click();
    h.run_steps(1);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.run_steps(1);
    for ch in "2*64".chars() {
        h.event(egui::Event::Text(ch.to_string()));
        h.run_steps(1);
    }
    h.key_press(egui::Key::Enter);
    h.run_steps(2);
    assert_eq!(fill(h.state())[0], 0x80, "2*64");
    assert_eq!(steps(h.state()), before + 5, "2*64 is one step");
}

/// The pro Color panel's hex readout edits the foreground too.
#[test]
fn color_field_hex_field_takes_a_typed_hex() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.tools.foreground = [1.0, 1.0, 1.0, 1.0];
    let mut h = field_harness(app);
    type_hex(&mut h, "003300");
    assert_eq!(srgb_bytes(h.state().session.tools.foreground), [0x00, 0x33, 0x00]);
}

/// The Color field fills the dock group, shrinks to leave room for the readout in a short group, and ignores a bad height.
#[test]
fn color_field_height_fills_the_group() {
    assert_eq!(color_field_height(None, 300.0), 120.0);
    assert_eq!(color_field_height(Some(500.0), 300.0), 468.0);
    assert_eq!(color_field_height(Some(500.0), 200.0), 442.0);
    assert_eq!(color_field_height(Some(60.0), 300.0), 28.0);
    assert_eq!(color_field_height(Some(f32::NAN), 300.0), 120.0);
    assert_eq!(color_field_height(Some(1.0e9), 300.0), 2000.0);
}

/// A squeezed Color group still shows the hex/RGB readout, not only the spectrum field (#6885).
#[test]
fn docked_color_field_keeps_rgb_readout_at_compact_height() {
    use egui_kittest::kittest::Queryable;
    let app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    // Body height after tab strip and margins in a Color group at `compact_height` (130 pt).
    let inner = 86.0;
    let mut h = egui_kittest::Harness::builder().with_size(vec2(240.0, inner)).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            if ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                ui.set_height(inner);
                ui.data_mut(|d| d.insert_temp(color_field_fill_id(), inner));
                color_field(app, ui);
            }
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::Pro);
    h.run_steps(2);
    assert!(h.query_by_label("R").is_some(), "RGB readout stays visible when the group is short");
    assert!(h.query_by_role(egui::accesskit::Role::TextInput).is_some(), "hex field stays visible");
}

fn field_harness(app: PhotocraftApp) -> egui_kittest::Harness<'static, PhotocraftApp> {
    // 60 fps steps, so two clicks a frame apart are a double-click.
    let mut h = egui_kittest::Harness::builder().with_size(vec2(300.0, 200.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            if ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                color_field(app, ui);
            }
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::Studio);
    h.run_steps(2);
    h
}

fn click(h: &mut egui_kittest::Harness<'_, PhotocraftApp>, p: egui::Pos2) {
    h.hover_at(p);
    h.run_steps(1);
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE });
        h.run_steps(1);
    }
}

/// A click on the background chip makes the field edit the background, not the foreground.
#[test]
fn background_chip_retargets_the_color_field() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.tools.foreground = [0.2, 0.4, 0.6, 1.0];
    app.session.tools.background = [0.0, 0.0, 0.0, 1.0];
    let mut h = field_harness(app);
    let min = h.ctx.input(|i| i.viewport_rect()).min;
    // The background chip's corner that the foreground chip doesn't cover.
    click(&mut h, min + vec2(40.0, 40.0));
    // The field's top-left: no saturation, full brightness.
    click(&mut h, min + vec2(60.0, 10.0));
    assert!(h.state().ui.color_panel.background);
    let tools = &h.state().session.tools;
    assert_eq!(tools.foreground, [0.2, 0.4, 0.6, 1.0]);
    assert!(tools.background[..3].iter().all(|&v| v > 0.9), "{:?}", tools.background);
}

#[test]
fn double_clicking_a_chip_opens_its_color_picker() {
    let app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let mut h = field_harness(app);
    let min = h.ctx.input(|i| i.viewport_rect()).min;
    // The foreground point is where the chips overlap: the foreground is on top there.
    for (p, target) in [(vec2(40.0, 40.0), "background"), (vec2(30.0, 30.0), "foreground")] {
        h.state_mut().ui.dialogs.clear();
        // Past the last double-click, so this one isn't counted as a triple-click.
        h.run_steps(40);
        click(&mut h, min + p);
        assert!(h.state().ui.dialogs.is_empty(), "a single click only picks the chip");
        click(&mut h, min + p);
        let [d] = h.state().ui.dialogs.as_slice() else { panic!("one Color Picker") };
        assert_eq!(d.fields.get("__colorPicker").and_then(Value::as_str), Some(target));
    }
}

/// With a shape tool and a shape layer selected, the field recolours its fill, and each
/// gesture is its own undo step: a click, every drag, and a swatch followed by a drag. A drag
/// only reaches the field after its press frame, so it must not join the step before it.
#[test]
fn color_panel_gestures_recolour_the_selected_shape_in_separate_steps() {
    use photocraft_doc::{Fill, LayerContent, LayerId};
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 64, "height": 48})).unwrap();
    let id = LayerId(app.run("shape.create", json!({"rect": [8, 8, 40, 30], "fill": "#000000"})).unwrap()["layer"].as_u64().unwrap());
    app.ui.tool = crate::state::Tool::Rectangle;
    let mut h = field_harness(app);
    let is_foreground = |app: &PhotocraftApp| {
        let fg = app.session.tools.foreground;
        match &app.session.active().unwrap().doc.layer(id).unwrap().content {
            LayerContent::Shape(sh) => matches!(&sh.fill, Some(Fill::Solid(c)) if (0..3).all(|i| (c.c[i] - fg[i]).abs() < 0.01)),
            _ => panic!("not a shape layer"),
        }
    };
    let steps = |app: &PhotocraftApp| app.session.active().unwrap().history.entries().len();
    let before = steps(h.state());
    let min = h.ctx.input(|i| i.viewport_rect()).min;
    click(&mut h, min + vec2(60.0, 10.0));
    assert!(is_foreground(h.state()));
    assert_eq!(steps(h.state()), before + 1, "the click");
    drag(&mut h, min + vec2(120.0, 40.0), min + vec2(250.0, 120.0));
    assert!(is_foreground(h.state()));
    assert_eq!(steps(h.state()), before + 2, "the drag is one more step");
    drag(&mut h, min + vec2(200.0, 60.0), min + vec2(100.0, 150.0));
    assert_eq!(steps(h.state()), before + 3, "the next drag another");
    // The drag ended on black, the first swatch: picking the shape's own colour changes
    // nothing and adds no step (#5568), so take the second swatch.
    let fg = h.state().session.tools.foreground;
    let name = h.state_mut().run("swatches.list", json!({})).unwrap()["groups"][0]["swatches"][1]["name"].as_str().unwrap().to_string();
    crate::swatches_ui::use_swatch(h.state_mut(), &name, false);
    assert_ne!(h.state().session.tools.foreground, fg, "{name} is another colour");
    assert_eq!(steps(h.state()), before + 4, "the swatch");
    drag(&mut h, min + vec2(150.0, 30.0), min + vec2(220.0, 100.0));
    assert!(is_foreground(h.state()));
    assert_eq!(steps(h.state()), before + 5, "a drag after a swatch is a step of its own");
}

/// #5568: with a shape tool and a shape layer selected, the Color panel recolours its fill (keeping the fill's
/// opacity): a click is one undo step and a whole drag across the field is one more.
#[test]
fn color_panel_recolours_the_selected_shape_fill() {
    use photocraft_doc::{Fill, LayerContent, LayerId};
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 64, "height": 48})).unwrap();
    let id = LayerId(app.run("shape.create", json!({"rect": [8, 8, 40, 30], "fill": [0.2, 0.4, 0.6, 0.5]})).unwrap()["layer"].as_u64().unwrap());
    app.ui.tool = crate::state::Tool::Rectangle;
    let mut h = field_harness(app);
    let fill = |app: &PhotocraftApp| match &app.session.active().unwrap().doc.layer(id).unwrap().content {
        LayerContent::Shape(sh) => sh.fill.clone(),
        _ => panic!("not a shape layer"),
    };
    let steps = |app: &PhotocraftApp| app.session.active().unwrap().history.entries().len();
    let before = steps(h.state());
    let min = h.ctx.input(|i| i.viewport_rect()).min;
    // The field's top-left: no saturation, full brightness.
    click(&mut h, min + vec2(60.0, 10.0));
    let fg = h.state().session.tools.foreground;
    assert_eq!(fill(h.state()), Some(Fill::Solid(photocraft_doc::Color::rgba(fg[0], fg[1], fg[2], 0.5))));
    assert_eq!(steps(h.state()), before + 1);
    drag(&mut h, min + vec2(120.0, 40.0), min + vec2(250.0, 120.0));
    let fg = h.state().session.tools.foreground;
    assert_eq!(fill(h.state()), Some(Fill::Solid(photocraft_doc::Color::rgba(fg[0], fg[1], fg[2], 0.5))));
    assert_eq!(steps(h.state()), before + 2, "the whole drag is one undo step");
}

/// Press at `from`, move to `to` in six steps and release, as egui sees a drag.
fn drag(h: &mut egui_kittest::Harness<'_, PhotocraftApp>, from: egui::Pos2, to: egui::Pos2) {
    h.hover_at(from);
    h.run_steps(1);
    h.drag_at(from);
    h.run_steps(1);
    for i in 1..=6 {
        h.hover_at(from + (to - from) * (i as f32 / 6.0));
        h.run_steps(1);
    }
    h.drop_at(to);
    h.run_steps(3);
}

/// Black carries neither hue nor saturation, a grey only the saturation it really has, and any
/// other colour nothing at all: the knob keeps where it was dragged instead of round-tripping
/// through RGB to the left edge (#3228).
#[test]
fn field_hsva_keeps_black_where_it_was_and_leaves_a_real_colour_alone() {
    // Read back from black, as every frame does: nothing of the drag survives the RGB trip.
    let black = field_hsva(egui::ecolor::Hsva::new(0.0, 0.0, 0.0, 1.0), (0.6, 1.0));
    assert_eq!((black.h, black.s), (0.6, 1.0), "the corner the drag ended in");
    // A grey: the saturation is genuinely zero — that is where the knob belongs — only the
    // hue was undefined.
    let grey = field_hsva(egui::ecolor::Hsva::new(0.0, 0.0, 0.5, 1.0), (0.6, 1.0));
    assert_eq!((grey.h, grey.s), (0.6, 0.0), "the grey stays on the left edge");
    // An ordinary colour is left as it is.
    let red = field_hsva(egui::ecolor::Hsva::new(0.1, 0.8, 0.9, 1.0), (0.6, 1.0));
    assert_eq!((red.h, red.s), (0.1, 0.8));
}

/// Dragging the selector into the bottom-right corner picks black there and leaves the knob
/// in that corner: black has no saturation of its own, so it used to be read back as `s = 0`
/// and drawn at the bottom-left instead, one frame after the drag that put it on the right
/// (#3228).
#[test]
fn dragging_to_the_bottom_right_keeps_the_knob_there() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.tools.foreground = [1.0, 1.0, 1.0, 1.0];
    let mut h = field_harness(app);
    let min = h.ctx.input(|i| i.viewport_rect()).min;
    // Into the far bottom-right corner of the SV field: full saturation, no value. The end
    // point lies past the corner on purpose — a real drag goes there too, and the widget
    // clamps it to the field.
    drag(&mut h, min + vec2(200.0, 60.0), min + vec2(280.0, 170.0));
    assert_eq!(srgb_bytes(h.state().session.tools.foreground), [0, 0, 0], "both corners are black");
    // What the knob is drawn from on the very next frame: the field kept the saturation the
    // drag ended at (not the zero black reads back as), together with the hue.
    let remembered: (f32, f32) = h.ctx.data(|d| d.get_temp(egui::Id::unique(("color-field-hue", false)))).expect("the field remembers what it showed");
    assert!(remembered.1 > 0.9, "the saturation the corner was dragged to: {remembered:?}");
    let shown = field_hsva(egui::ecolor::Hsva::new(0.0, 0.0, 0.0, 1.0), remembered);
    assert!(shown.s > 0.9, "the knob stays at the right edge, not the left: {remembered:?}");
    assert!(shown.v < 0.01, "still black: {shown:?}");
}

/// Black has no hue of its own, so each chip remembers the hue last picked for it.
#[test]
fn each_chip_keeps_its_own_hue_on_black() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.tools.foreground = [0.0, 0.0, 0.0, 1.0];
    app.session.tools.background = [0.0, 0.0, 0.0, 1.0];
    let mut h = field_harness(app);
    let min = h.ctx.input(|i| i.viewport_rect()).min;
    // The hue strip runs from red at the top through blue (a third down) and green (two thirds).
    let (strip_x, green, blue) = (285.0, 88.0, 48.0);
    click(&mut h, min + vec2(strip_x, green));
    click(&mut h, min + vec2(40.0, 40.0));
    click(&mut h, min + vec2(strip_x, blue));
    click(&mut h, min + vec2(16.0, 16.0));
    // Near the field's top-right: high saturation and brightness.
    click(&mut h, min + vec2(265.0, 10.0));
    let [r, g, b] = srgb_bytes(h.state().session.tools.foreground);
    assert!(g > 200 && r < 64 && b < 64, "the foreground's green, not the background's blue: {:?}", [r, g, b]);
}
