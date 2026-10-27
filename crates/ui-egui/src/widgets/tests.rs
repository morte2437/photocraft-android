use super::{hue_color, hue_stops};
use egui::{Color32, Key, Modifiers};

#[test]
fn hue_stops_are_pure_and_evenly_spaced() {
    let c = |r, g, b| Color32::from_rgb(r, g, b);
    let want = [c(255, 0, 0), c(255, 255, 0), c(0, 255, 0), c(0, 255, 255), c(0, 0, 255), c(255, 0, 255), c(255, 0, 0)];
    let stops = hue_stops();
    assert_eq!(stops.len(), 13);
    for (k, w) in want.iter().enumerate() {
        assert_eq!(stops[k * 2], *w, "stop at {} degrees", k * 60);
        assert_eq!(hue_color(k as f32 * 60.0), *w);
    }
}

/// A light theme rings the slider knob with its outline token; a dark Studio-layout one draws
/// the plain knob (#7927).
#[test]
fn light_themes_draw_the_slider_knob_outline() {
    use crate::theme::{ThemeKind, Tokens};
    fn rings(shape: &egui::Shape, out: &mut Vec<egui::Color32>) {
        match shape {
            egui::Shape::Vec(v) => v.iter().for_each(|s| rings(s, out)),
            egui::Shape::Circle(c) if c.radius == 7.0 && c.stroke.width == 1.0 => out.push(c.stroke.color),
            _ => {}
        }
    }
    for kind in [ThemeKind::StudioLight, ThemeKind::Adwaita, ThemeKind::BreezeLight, ThemeKind::SoftLight, ThemeKind::Studio] {
        let ctx = egui::Context::default();
        crate::theme::apply(&ctx, kind);
        let mut v = 50.0;
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            super::slider(ui, &mut v, 0.0..=100.0, None);
        });
        out.textures_delta.clear();
        let mut found = Vec::new();
        out.shapes.iter().for_each(|c| rings(&c.shape, &mut found));
        let want: Vec<_> = Tokens::for_kind(kind).knob_outline().into_iter().collect();
        assert_eq!(found, want, "{kind:?}");
    }
}

#[test]
fn checkbox_label_is_a_button_target_not_selectable_text() {
    use egui::CursorIcon;
    use egui_kittest::{
        Harness,
        kittest::{NodeT, Queryable},
    };

    let mut h = Harness::builder().with_size(egui::vec2(300.0, 80.0)).build_ui_state(
        |ui, on: &mut bool| {
            super::checkbox(ui, on, "Show Transform Controls");
        },
        false,
    );
    h.run_steps(2);
    let label = h.get_by_label("Show Transform Controls").rect();
    h.hover_at(label.center());
    h.run_steps(2);
    assert_eq!(h.output().platform_output.cursor_icon, CursorIcon::PointingHand);
    h.get_by_label("Show Transform Controls").click();
    h.run_steps(2);
    assert!(*h.state());
    let checkbox = h.get_by_role_and_label(egui::accesskit::Role::CheckBox, "Show Transform Controls");
    assert_eq!(checkbox.accesskit_node().toggled(), Some(egui::accesskit::Toggled::True));
    checkbox.focus();
    h.run_steps(1);
    h.key_press(egui::Key::Space);
    h.run_steps(2);
    assert!(!*h.state(), "the same named checkbox supports Space");
}

/// #3208: a rounded swatch's checker is one shared, repeating, unblurred two-by-two tile on
/// the rounded rect, scaled so a cell stays `cell` points, so nothing shows past the corners.
#[test]
fn rounded_swatches_clip_their_checker_to_the_corners() {
    use egui::{Pos2, Rect, pos2, vec2};
    let mut h = egui_kittest::Harness::builder().with_size(vec2(200.0, 100.0)).build_ui(|ui| {
        super::color_swatch(ui, egui::Color32::RED);
        super::color_swatch_button(ui, egui::Color32::from_rgba_unmultiplied(0, 0, 255, 128), "Blue");
    });
    crate::PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::Studio);
    h.run_steps(2);
    let tile = h.ctx.data(|d| d.get_temp::<egui::TextureHandle>(super::checker_tile_id())).expect("the tile is loaded once");
    let manager = h.ctx.tex_manager();
    let manager = manager.read();
    let meta = manager.meta(tile.id()).expect("the tile is in the texture manager");
    assert_eq!(meta.size, [2, 2]);
    assert_eq!(meta.options.wrap_mode, egui::TextureWrapMode::Repeat);
    assert_eq!(meta.options.magnification, egui::TextureFilter::Nearest);
    assert_eq!(super::checker_uv(Rect::from_min_size(Pos2::ZERO, vec2(40.0, 18.0)), 4.0), Rect::from_min_max(Pos2::ZERO, pos2(5.0, 2.25)));
    assert_eq!(
        super::checker_uv(Rect::from_min_size(Pos2::ZERO, vec2(8.0, 8.0)), 0.0),
        Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0)),
        "a bad cell falls back to 4 pt"
    );
}

/// #3580: the shared eliding label never lays out wider than the room it is given, so a longer
/// translation shrinks with '…' instead of overrunning its row; a short label is untouched.
#[test]
fn elided_label_never_exceeds_its_room() {
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(240.0, 60.0)).build_ui(|ui| {
        let long = super::elided_label(ui, "Preenchimento:", 24.0); // pt-br Fill:, the longest catalog
        assert!(long.rect.width() <= 25.0, "elided to the room, got {}", long.rect.width());
        let short = super::elided_label(ui, "Fill", 200.0);
        assert!(short.rect.width() < 60.0, "a short label is not padded: {}", short.rect.width());
    });
    crate::PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::Studio);
    h.run_steps(2);
}

#[test]
fn spectrum_popup_style_matches_main_menu_and_preserves_studio() {
    for kind in [crate::theme::ThemeKind::Pro, crate::theme::ThemeKind::ProMedium, crate::theme::ThemeKind::Studio, crate::theme::ThemeKind::StudioLight] {
        let ctx = egui::Context::default();
        crate::theme::apply(&ctx, kind);
        let tokens = crate::theme::Tokens::for_kind(kind);
        let mut out = ctx.run_ui(Default::default(), |ui| {
            let original_padding = ui.spacing().button_padding;
            let original_hover = ui.visuals().widgets.hovered.fg_stroke;
            super::style_spectrum_popup_menu(ui);
            assert_eq!(ui.spacing().item_spacing.y, 0.0, "{kind:?}: menu rows meet so the gap is selectable");
            if tokens.pro {
                assert_eq!(ui.spacing().button_padding, egui::vec2(10.0, 4.0));
                assert_eq!(ui.visuals().widgets.hovered.bg_fill, tokens.accent);
                assert_eq!(ui.visuals().widgets.hovered.fg_stroke.color, egui::Color32::WHITE);
            } else {
                assert_eq!(ui.spacing().button_padding, original_padding);
                assert_eq!(ui.visuals().widgets.hovered.fg_stroke, original_hover);
            }
        });
        out.textures_delta.clear();
    }
}

/// One primary press+release at `pos`. When `touch`, emit Touch Start/End with the pointer events (as egui-winit does) so [`super::second_click`] uses the touch distance.
fn pointer_click(ctx: &egui::Context, time: f64, pos: egui::Pos2, touch: bool) -> bool {
    let area = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(200.0, 200.0));
    let button = |pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
    let touch_ev = |phase| egui::Event::Touch { device_id: egui::TouchDeviceId(1), id: egui::TouchId(1), phase, pos, force: None };
    let mut saw = false;
    for (t, mut events) in [
        (time, {
            let mut v = vec![egui::Event::PointerMoved(pos), button(true)];
            if touch {
                v.insert(0, touch_ev(egui::TouchPhase::Start));
            }
            v
        }),
        (time + 0.02, {
            let mut v = vec![button(false)];
            if touch {
                v.insert(0, touch_ev(egui::TouchPhase::End));
            }
            v
        }),
    ] {
        let mut raw = egui::RawInput { time: Some(t), screen_rect: Some(area), ..Default::default() };
        raw.events.append(&mut events);
        let mut out = ctx.run_ui(raw, |ui| {
            let r = ui.allocate_rect(area, egui::Sense::click());
            saw |= super::second_click(ui, &r);
        });
        out.textures_delta.clear();
    }
    saw
}

/// #2274 touch: taps past egui's 6 px mouse limit (and past the old 32 px touch slop) within 0.5 s still count as a double; the same mouse path does not.
#[test]
fn second_click_tolerates_touch_jitter_but_keeps_mouse_tight() {
    let area = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(200.0, 200.0));
    let a = egui::pos2(50.0, 50.0);
    let mild = egui::pos2(70.0, 50.0); // 20 px — past mouse max_click_dist (6)
    let wide = egui::pos2(50.0 + 48.0, 50.0); // past the old 32 px slop, inside TOUCH_MAX_CLICK_DIST (64)
    for touch in [false, true] {
        let ctx = egui::Context::default();
        let mut out = ctx.run_ui(egui::RawInput { time: Some(0.0), screen_rect: Some(area), ..Default::default() }, |ui| {
            ui.allocate_rect(area, egui::Sense::click());
        });
        out.textures_delta.clear();
        assert!(!pointer_click(&ctx, 0.1, a, touch), "first click only arms the pair");
        let second = pointer_click(&ctx, 0.25, mild, touch);
        assert_eq!(second, touch, "touch={touch}: 20 px jitter is a double only for touch");
    }
    // Touch-only: casual Framework-style drift beyond the previous 32 px budget.
    let ctx = egui::Context::default();
    let mut out = ctx.run_ui(egui::RawInput { time: Some(0.0), screen_rect: Some(area), ..Default::default() }, |ui| {
        ui.allocate_rect(area, egui::Sense::click());
    });
    out.textures_delta.clear();
    assert!(!pointer_click(&ctx, 0.1, a, true));
    assert!(pointer_click(&ctx, 0.25, wide, true), "48 px touch jitter is still a double");
}

/// A mouse double past the time window is not a double, even at the same pixel.
#[test]
fn second_click_respects_the_half_second_window() {
    let area = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(200.0, 200.0));
    let p = egui::pos2(40.0, 40.0);
    let ctx = egui::Context::default();
    let mut out = ctx.run_ui(egui::RawInput { time: Some(0.0), screen_rect: Some(area), ..Default::default() }, |ui| {
        ui.allocate_rect(area, egui::Sense::click());
    });
    out.textures_delta.clear();
    assert!(!pointer_click(&ctx, 0.1, p, false));
    assert!(!pointer_click(&ctx, 0.1 + super::DOUBLE_CLICK_S + 0.05, p, false), "past Windows' 0.5 s window");
}

use egui_kittest::{Harness, kittest::Queryable};

/// A panel footer's buttons are drawn whole. The bar runs on over the body's bottom margin, so a
/// body that clips where its content ends (as the panels do) cut the bottom off every button:
/// the hover highlight of a 26 px button was about 21 px tall in the Paths, Layers and Channels
/// panels (#6238).
#[test]
fn panel_footer_buttons_are_not_clipped_by_the_body() {
    let mut h = Harness::new_ui_state(
        |ui, s: &mut Option<(egui::Rect, egui::Rect, egui::Rect)>| {
            // A body exactly as tall as the footer takes, clipped there.
            let body = egui::Rect::from_min_size(ui.max_rect().min, egui::vec2(220.0, super::footer_height(ui)));
            let mut body_ui = ui.new_child(egui::UiBuilder::new().max_rect(body));
            body_ui.set_clip_rect(body);
            super::panel_footer(&mut body_ui, |bar| {
                let b = crate::icons::button(bar, "square", 26.0, false, "Button");
                *s = Some((b.rect, bar.clip_rect(), body));
            });
        },
        None,
    );
    crate::PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::Studio);
    h.run();
    let (button, clip, body) = h.state().expect("the footer drew its button");
    assert!(button.bottom() > body.bottom(), "the bar really overruns the body, or this test proves nothing: {button:?} vs {body:?}");
    assert!(clip.contains_rect(button), "the whole button is inside the clip: {button:?} not in {clip:?}");
}

/// #3789: a plain text field is one boxed, outline-free input whose text-selection colour stays
/// local, so it isn't recoloured for the rest of the frame (selected rows, slider trails).
#[test]
fn text_field_is_a_boxed_input_whose_selection_colour_stays_local() {
    use egui::accesskit::Role;
    let mut h = Harness::new_ui_state(
        |ui, s: &mut (String, egui::Color32, egui::Color32)| {
            s.1 = ui.visuals().selection.bg_fill;
            super::text_field(ui, egui::Id::unique("tf"), &mut s.0, "hint", 140.0, false);
            s.2 = ui.visuals().selection.bg_fill;
        },
        (String::new(), egui::Color32::WHITE, egui::Color32::BLACK),
    );
    crate::PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::Studio);
    h.run();
    assert_eq!(h.state().1, h.state().2, "the field's selection colour does not leak into its parent");
    h.get_by_role(Role::TextInput).click();
    h.run_steps(1);
    for ch in "abc".chars() {
        h.event(egui::Event::Text(ch.to_string()));
        h.run_steps(1);
    }
    assert_eq!(h.state().0, "abc", "typing reaches the bound string");
}

/// #2150: a huge paste into a plain text field stops at its limit instead of growing the
/// string (and its laid-out text) without bound.
#[test]
fn text_field_caps_a_huge_paste() {
    use egui::accesskit::Role;
    let mut h = Harness::new_ui_state(|ui, s: &mut String| drop(super::text_field(ui, egui::Id::unique("tf"), s, "Value", 140.0, false)), String::new());
    crate::PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::Studio);
    h.run();
    h.get_by_role(Role::TextInput).click();
    h.run_steps(1);
    h.event(egui::Event::Paste("9".repeat(200_000)));
    h.run_steps(1);
    assert_eq!(h.state().chars().count(), super::TEXT_FIELD_LIMIT);
    // A second paste adds nothing past the limit.
    h.event(egui::Event::Paste("9".repeat(10)));
    h.run_steps(1);
    assert_eq!(h.state().chars().count(), super::TEXT_FIELD_LIMIT);
}

/// #2150: a value already past the limit is laid out only up to it, and stays the caller's
/// until the field is edited, which keeps the bounded copy.
#[test]
fn text_field_bounds_an_oversized_existing_value() {
    use egui::accesskit::Role;
    let long = "9".repeat(super::TEXT_FIELD_LIMIT * 4);
    let mut h = Harness::new_ui_state(|ui, s: &mut String| drop(super::text_field(ui, egui::Id::unique("tf"), s, "Value", 140.0, false)), long.clone());
    crate::PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::Studio);
    h.run();
    assert_eq!(*h.state(), long, "drawing alone leaves the caller's value alone");
    assert_eq!(h.get_by_role(Role::TextInput).value().map(|v| v.chars().count()), Some(super::TEXT_FIELD_LIMIT));
    h.get_by_role(Role::TextInput).click();
    h.run_steps(1);
    h.event(egui::Event::Key { key: egui::Key::Backspace, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE });
    h.run_steps(1);
    assert!(h.state().chars().count() < super::TEXT_FIELD_LIMIT, "an edit keeps the bounded copy");
}

/// #2150: text queued by the control channel (`ui.type`) joins the input in `raw_input_hook`
/// and is cut like the OS's, so the cap must run after it is appended.
#[test]
fn raw_input_hook_caps_synthetic_text() {
    use eframe::App as _;
    let mut app = crate::PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.synthetic.push(egui::Event::Text("x".repeat(super::INPUT_EVENT_LIMIT + 10)));
    let ctx = egui::Context::default();
    let mut raw = egui::RawInput::default();
    app.raw_input_hook(&ctx, &mut raw);
    let texts: Vec<usize> = raw.events.iter().filter_map(|e| if let egui::Event::Text(t) = e { Some(t.chars().count()) } else { None }).collect();
    assert_eq!(texts, [super::INPUT_EVENT_LIMIT]);
}

/// #2150: every text-carrying event is cut to the app-wide limit at a character boundary
/// before any widget sees it; other events and short text pass untouched.
#[test]
fn oversized_text_events_are_cut_before_widgets_see_them() {
    let long = "é".repeat(super::INPUT_EVENT_LIMIT + 5);
    let mut events = vec![
        egui::Event::Paste(long.clone()),
        egui::Event::Text(long.clone()),
        egui::Event::Ime(egui::ImeEvent::Commit(long.clone())),
        egui::Event::Ime(egui::ImeEvent::Preedit { text: long, active_range_chars: Some(0..super::INPUT_EVENT_LIMIT + 2) }),
        egui::Event::Text("ok".into()),
        egui::Event::Copy,
    ];
    super::cap_text_events(&mut events);
    for e in &events[..4] {
        let (egui::Event::Paste(t)
        | egui::Event::Text(t)
        | egui::Event::Ime(egui::ImeEvent::Commit(t))
        | egui::Event::Ime(egui::ImeEvent::Preedit { text: t, .. })) = e
        else {
            panic!("{e:?}")
        };
        assert_eq!(t.chars().count(), super::INPUT_EVENT_LIMIT);
    }
    assert!(matches!(&events[3], egui::Event::Ime(egui::ImeEvent::Preedit { active_range_chars: None, .. })), "the range past the cut is dropped");
    assert_eq!(events[4], egui::Event::Text("ok".into()));
    assert_eq!(events[5], egui::Event::Copy);
}

/// A row's visible caption names both its number and its slider; a dropdown is named by its
/// own caption, or by the scrubby caption drawn just before it.
#[test]
fn labelled_rows_and_dropdowns_have_semantic_accessible_names() {
    use egui_kittest::{Harness, kittest::Queryable};

    #[derive(Default)]
    struct State {
        value: f32,
        fill: f32,
        blend: usize,
        mode: usize,
    }

    let mut h = Harness::builder().with_size(egui::vec2(360.0, 240.0)).build_ui_state(
        |ui, s: &mut State| {
            super::slider_row(ui, "Opacity", &mut s.value, 0.0..=100.0, "%", None);
            super::popup_value_field(ui, "Fill", &mut s.fill, 0.0..=100.0, "%", 74.0);
            let opts = [(0usize, "Normal"), (1, "Multiply")];
            super::dropdown(ui, "blend", "Blend Mode", &mut s.blend, &opts, 120.0);
            ui.horizontal(|ui| {
                let caption = ui.label("Mode");
                super::scrub_option_label(ui, &caption);
                super::dropdown(ui, "mode", "Not this name", &mut s.mode, &opts, 120.0);
            });
        },
        State::default(),
    );
    crate::PhotocraftApp::setup_context(&h.ctx, Default::default());
    h.run();

    // The row's caption is the accessible name of the value field and of the slider.
    h.get_by_role_and_label(egui::accesskit::Role::SpinButton, "Opacity");
    h.get_by_role_and_label(egui::accesskit::Role::Slider, "Opacity");
    // A popup value field names both its number and the arrow that opens its slider.
    h.get_by_role_and_label(egui::accesskit::Role::SpinButton, "Fill");
    h.get_by_role_and_label(egui::accesskit::Role::Button, "Fill");
    // A dropdown requires a real name; a preceding scrubby caption is used when present.
    h.get_by_role_and_label(egui::accesskit::Role::ComboBox, "Blend Mode");
    h.get_by_role_and_label(egui::accesskit::Role::ComboBox, "Mode");
}

/// Sets up a test with one value field. Its state is its value and how many times it changed.
fn field(value: f32, range: std::ops::RangeInclusive<f32>) -> Harness<'static, (f32, u32)> {
    let mut h = Harness::new_ui_state(
        move |ui, s: &mut (f32, u32)| {
            if super::value_field(ui, &mut s.0, range.clone(), "px", 80.0).accessible_name("Test number").changed() {
                s.1 += 1;
            }
        },
        (value, 0),
    );
    h.run();
    h
}

fn press(h: &mut Harness<'static, (f32, u32)>, mods: Modifiers, key: Key) -> f32 {
    h.key_press_modifiers(mods, key);
    h.run();
    h.state().0
}

fn focused(value: f32, range: std::ops::RangeInclusive<f32>) -> Harness<'static, (f32, u32)> {
    let mut h = field(value, range);
    h.get_by_role(egui::accesskit::Role::SpinButton).click();
    h.run();
    h
}

/// The number's slot (and its focus outline) is centred in the box (#5150).
#[test]
fn value_field_number_slot_is_centred_in_its_box() {
    let mut h = Harness::new_ui_state(
        |ui, s: &mut (f32, egui::Rect)| {
            let field = super::value_field_in(ui, &mut s.0, 0.0..=100.0, "px", 80.0, 0.0, super::FieldOpts::default(), None);
            s.1 = field.1;
            field.0.accessible_name("Value");
        },
        (50.0, egui::Rect::NOTHING),
    );
    h.run();
    let number = h.get_by_role(egui::accesskit::Role::SpinButton).rect();
    let field = h.state().1;
    assert!((field.center().y - number.center().y).abs() < 0.01, "field {field:?}, number {number:?}");
}

/// The digits' ink, not just their slot, is centred in the box, and starts level with the
/// `%`'s, so the number lines up with the box and its unit (#5150).
#[test]
fn value_field_digits_ink_is_centred_and_level_with_the_unit() {
    let _gpu = photocraft_testkit::gpu_lock();
    let scale = 4.0;
    for kind in [crate::theme::ThemeKind::Pro, crate::theme::ThemeKind::Studio] {
        let mut h = Harness::builder().with_size(egui::vec2(100.0, 40.0)).with_pixels_per_point(scale).wgpu().build_ui_state(
            |ui, s: &mut (f32, egui::Rect)| {
                let field = super::value_field_in(ui, &mut s.0, 0.0..=100.0, "%", 80.0, 0.0, super::FieldOpts::default(), None);
                s.1 = field.1;
                field.0.accessible_name("Value");
            },
            (100.0, egui::Rect::NOTHING),
        );
        crate::PhotocraftApp::setup_context(&h.ctx, kind);
        h.run_steps(3);
        let image = h.render().expect("render value field");
        let (field, suffix_left) = (h.state().1, h.state().1.right() - 14.0);
        let px = |v: f32| (v * scale) as u32;
        let bg = image.get_pixel(px(field.left() + 2.0), px(field.center().y)).0;
        let lum = |p: [u8; 4]| u32::from(p[0]) + u32::from(p[1]) + u32::from(p[2]);
        // Top and bottom of the ink (pt) between x0 and x1.
        let ink = |x0: f32, x1: f32| {
            let rows: Vec<u32> =
                (px(field.top() + 2.0)..px(field.bottom() - 2.0)).filter(|&y| (px(x0)..px(x1)).any(|x| lum(image.get_pixel(x, y).0) > lum(bg) + 150)).collect();
            let (top, bottom) = (rows.first().copied().unwrap_or(0), rows.last().copied().unwrap_or(0) + 1);
            (top as f32 / scale, bottom as f32 / scale)
        };
        let digits = ink(field.left() + 4.0, suffix_left - 2.0);
        let unit = ink(suffix_left, field.right() - 2.0);
        let what = format!("{kind:?}: field {field:?}, digits {digits:?}, unit {unit:?}");
        assert!(((digits.0 + digits.1) / 2.0 - field.center().y).abs() <= 0.25, "{what}");
        assert!((digits.0 - unit.0).abs() <= 0.25, "{what}");
    }
}

/// With the app's theme (its 24 pt interact height and 4 pt button padding), in a full-size
/// box and a slim status-bar box alike, the number stays inside the box, centred in it,
/// and a long number doesn't spill out of the box's left edge (#2823).
#[test]
fn themed_value_field_number_sits_inside_its_box_in_every_size() {
    for kind in crate::theme::ThemeKind::ALL {
        for (w, h, v) in [(80.0, super::VALUE_FIELD_H, 50.0), (64.0, 18.0, 1600.5), (78.0, 22.0, 100.6)] {
            let mut harness = Harness::new_ui_state(
                move |ui, s: &mut (f32, egui::Rect)| {
                    let before = ui.cursor().min;
                    super::compact_value_field(ui, &mut s.0, 0.0..=3200.0, "%", w, h).accessible_name("Zoom");
                    s.1 = egui::Rect::from_min_size(before, egui::vec2(w, h));
                },
                (v, egui::Rect::NOTHING),
            );
            crate::PhotocraftApp::setup_context(&harness.ctx, kind);
            harness.run();
            let number = harness.get_by_role(egui::accesskit::Role::SpinButton).rect();
            let field = harness.state().1;
            let what = format!("{kind:?} {w}x{h}: field {field:?}, number {number:?}");
            assert!(field.contains_rect(number), "{what}");
            assert!((field.center().y - number.center().y).abs() < 0.51, "{what}");
        }
    }
}

/// The number's slot ends just left of its unit, by the unit's measured width: a one-character
/// `%` (the status bar zoom) leaves the number more room than the old fixed 16 pt, and a longer
/// unit never sits under the number (#3040).
#[test]
fn value_field_number_slot_stops_at_its_measured_suffix() {
    for suffix in ["%", "px", "°", "×"] {
        let mut h = Harness::new_ui_state(
            move |ui, s: &mut (f32, egui::Rect, f32)| {
                let before = ui.cursor().min;
                super::compact_value_field(ui, &mut s.0, 0.0..=3200.0, suffix, 64.0, 18.0).accessible_name("Zoom");
                s.1 = egui::Rect::from_min_size(before, egui::vec2(64.0, 18.0));
                s.2 = ui.painter().layout_no_wrap(suffix.to_owned(), crate::theme::mono(11.0), egui::Color32::WHITE).size().x;
            },
            (33.33, egui::Rect::NOTHING, 0.0),
        );
        h.run();
        let number = h.get_by_role(egui::accesskit::Role::SpinButton).rect();
        let (_, field, suffix_w) = *h.state();
        let suffix_left = field.right() - 6.0 - suffix_w;
        assert!(number.right() <= suffix_left, "{suffix}: number {number:?} runs under the suffix at {suffix_left}");
        assert!(suffix_left - number.right() <= 1.5, "{suffix}: {:.1} pt wasted before the suffix", suffix_left - number.right());
    }
}

#[test]
fn arrow_keys_step_a_focused_field_by_one_and_shift_by_ten() {
    let mut h = focused(100.0, 0.0..=1000.0);
    assert_eq!(press(&mut h, Modifiers::NONE, Key::ArrowUp), 101.0);
    assert_eq!(press(&mut h, Modifiers::SHIFT, Key::ArrowUp), 111.0);
    assert_eq!(press(&mut h, Modifiers::NONE, Key::ArrowDown), 110.0);
    assert_eq!(press(&mut h, Modifiers::SHIFT, Key::ArrowDown), 100.0);
    assert_eq!(press(&mut h, Modifiers::COMMAND, Key::ArrowUp), 100.1);
    assert_eq!(press(&mut h, Modifiers::COMMAND | Modifiers::SHIFT, Key::ArrowDown), 100.0);
    assert_eq!(h.state().1, 6, "every step reports a change, so callers apply it");
}

#[test]
fn arrow_keys_on_a_field_with_a_reversed_range_do_not_panic() {
    // The empty range is the case under test — arrow keys must not panic on it.
    let mut h = focused(5.0, 10.0..=0.0); // NOSONAR
    let _ = press(&mut h, Modifiers::NONE, Key::ArrowUp);
    let mut h = focused(5.0, f32::NAN..=10.0);
    let _ = press(&mut h, Modifiers::NONE, Key::ArrowDown);
}

/// #5914: Exposure's ±20 stops keep 0.01 arrow steps when the field shows two decimals.
#[test]
fn arrow_keys_step_fractional_stops_when_decimals_are_set() {
    let mut h = Harness::new_ui_state(
        |ui, v: &mut f32| {
            super::slider_row_prec(ui, "Exposure", v, -20.0..=20.0, "", None, super::RowGestures::default(), Some(2));
        },
        0.0,
    );
    h.run();
    h.get_by_role(egui::accesskit::Role::SpinButton).click();
    h.run();
    h.key_press_modifiers(Modifiers::NONE, Key::ArrowUp);
    h.run();
    assert!((*h.state() - 0.01).abs() < 1e-6);
    h.key_press_modifiers(Modifiers::NONE, Key::ArrowUp);
    h.run();
    assert!((*h.state() - 0.02).abs() < 1e-6);
    h.key_press_modifiers(Modifiers::SHIFT, Key::ArrowUp);
    h.run();
    assert!((*h.state() - 0.12).abs() < 1e-6);
}

#[test]
fn arrow_keys_round_decimals_step_fine_fields_and_clamp() {
    for (start, mods, key, want) in [
        (55.4, Modifiers::NONE, Key::ArrowUp, 56.0),
        (55.6, Modifiers::NONE, Key::ArrowUp, 57.0),
        (55.4, Modifiers::NONE, Key::ArrowDown, 54.0),
        (55.5, Modifiers::NONE, Key::ArrowUp, 57.0),
        (55.5, Modifiers::NONE, Key::ArrowDown, 55.0),
        (55.4, Modifiers::SHIFT, Key::ArrowUp, 65.0),
        (55.47, Modifiers::COMMAND, Key::ArrowUp, 55.6),
    ] {
        assert_eq!(press(&mut focused(start, 0.0..=1000.0), mods, key), want, "{start} {mods:?} {key:?}");
    }
    let mut h = focused(0.5, 0.0..=1.0);
    assert_eq!(press(&mut h, Modifiers::NONE, Key::ArrowUp), 0.51);
    assert_eq!(press(&mut h, Modifiers::SHIFT, Key::ArrowUp), 0.61);
    assert_eq!(press(&mut h, Modifiers::COMMAND, Key::ArrowUp), 0.62);
    let mut h = focused(995.0, 0.0..=1000.0);
    assert_eq!(press(&mut h, Modifiers::SHIFT, Key::ArrowUp), 1000.0);
}

/// #6545: a curved track (the mask Feather) has fine steps at the left and coarse ones toward
/// the right end, and the knob drawn for a value lands where that value is picked.
#[test]
fn curved_track_maps_ends_middle_and_round_trips() {
    use super::{curve_fraction, curve_value};
    for (f, want) in [(0.0, 0.0), (0.1, 1.0), (0.2, 8.0), (0.5, 125.0), (1.0, 1000.0)] {
        assert!((curve_value(f, 0.0, 1000.0, Some(3.0)) - want).abs() < 1e-3, "{f}: {}", curve_value(f, 0.0, 1000.0, Some(3.0)));
    }
    assert_eq!(curve_value(0.25, 0.0, 1000.0, None), 250.0, "no curve is the linear track");
    assert_eq!(curve_value(0.5, 10.0, 20.0, Some(3.0)), 11.25, "the curve starts at the range's low end");
    for i in 0..=100 {
        let f = i as f32 / 100.0;
        let back = curve_fraction(curve_value(f, 0.0, 1000.0, Some(3.0)), 0.0, 1000.0, Some(3.0));
        assert!((back - f).abs() < 1e-4, "{f} -> {back}");
        let v = i as f32 * 10.0;
        let again = curve_value(curve_fraction(v, 0.0, 1000.0, Some(3.0)), 0.0, 1000.0, Some(3.0));
        assert!((again - v).abs() < 1e-3, "{v} -> {again}");
    }
    assert_eq!(curve_fraction(-5.0, 0.0, 1000.0, Some(3.0)), 0.0);
    assert_eq!(curve_fraction(f32::NAN, 0.0, 1000.0, Some(3.0)), 0.0);
}

#[test]
fn arrow_keys_work_out_a_typed_sum_then_step_it() {
    let mut h = focused(100.0, 0.0..=1000.0);
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    h.event(egui::Event::Text("5+5".into()));
    h.run();
    assert_eq!(press(&mut h, Modifiers::NONE, Key::ArrowUp), 11.0);
    assert_eq!(press(&mut h, Modifiers::NONE, Key::ArrowDown), 10.0, "the field keeps focus");
}

/// #5474: a held arrow repeats on consecutive frames; every repeat steps the focused field
/// and none of them moves focus to the field above.
#[test]
fn held_arrow_keeps_stepping_the_focused_field() {
    let mut h = Harness::new_ui_state(
        |ui, s: &mut (f32, f32)| {
            super::value_field(ui, &mut s.0, 0.0..=1000.0, "", 80.0).accessible_name("Left");
            super::value_field(ui, &mut s.1, 0.0..=1000.0, "", 80.0).accessible_name("Right");
        },
        (10.0, 59.0),
    );
    h.run();
    h.get_all_by_role(egui::accesskit::Role::SpinButton).nth(1).expect("second field").click();
    h.run();
    // Held down: key-down repeats on consecutive frames, no key-up in between.
    for repeat in [false, true, true, true, true, true] {
        h.event(egui::Event::Key { key: Key::ArrowUp, physical_key: None, pressed: true, repeat, modifiers: Modifiers::NONE });
        h.step();
    }
    h.event(egui::Event::Key { key: Key::ArrowUp, physical_key: None, pressed: false, repeat: false, modifiers: Modifiers::NONE });
    h.run();
    assert_eq!(*h.state(), (10.0, 65.0));
}

#[test]
fn arrow_keys_leave_an_unfocused_field_alone() {
    let mut h = field(100.0, 0.0..=1000.0);
    assert_eq!(press(&mut h, Modifiers::NONE, Key::ArrowUp), 100.0);
    assert_eq!(h.state().1, 0);
}

/// Photoshop: a unit typed into a px field converts at the document resolution on Enter, and
/// an expression that can't convert (or divides by zero) keeps the value.
#[test]
fn enter_commits_a_typed_unit_expression_at_the_document_resolution() {
    let mut h = field(100.0, 0.0..=10_000.0);
    super::set_field_resolution(&h.ctx, 300.0, 72.0);
    for (text, want) in [("1in+10", 310.0), ("2.54 cm * 2", 600.0), ("50%", 600.0), ("1in/0", 600.0), ("10px", 10.0)] {
        h.get_by_role(egui::accesskit::Role::SpinButton).click();
        h.run();
        h.key_press_modifiers(Modifiers::COMMAND, Key::A);
        h.event(egui::Event::Text(text.into()));
        h.run();
        h.key_press(Key::Enter);
        h.run();
        assert_eq!(h.state().0, want, "{text}");
    }
}

/// Photoshop's scrubby labels: dragging the label right before a field moves its value, a
/// step a point (Shift ×10); a label with no field right after it moves nothing.
#[test]
fn dragging_a_label_scrubs_the_field_right_after_it() {
    let mut h = Harness::new_ui_state(
        |ui, s: &mut (f32, f32)| {
            ui.horizontal(|ui| {
                let l = ui.label("Size:");
                super::scrub_label(ui, &l);
                super::value_field(ui, &mut s.0, 0.0..=1000.0, "px", 80.0).accessible_name("Start");
            });
            ui.horizontal(|ui| {
                let l = ui.label("Mode:");
                super::scrub_label(ui, &l);
                ui.add_space(60.0);
                super::value_field(ui, &mut s.1, 0.0..=1000.0, "px", 80.0).accessible_name("End");
            });
        },
        (100.0, 100.0),
    );
    h.run();
    let drag = |h: &mut Harness<'static, (f32, f32)>, label: &str, dx: f32, mods: Modifiers| {
        let at = h.get_by_label(label).rect().center();
        h.hover_at(at);
        h.run();
        h.drag_at(at);
        h.run();
        for i in 1..=4 {
            h.event(egui::Event::PointerMoved(at + egui::vec2(dx * i as f32 / 4.0, 0.0)));
            h.event(egui::Event::ModifiersChanged(mods));
            h.run();
        }
        h.drop_at(at + egui::vec2(dx, 0.0));
        h.run();
    };
    drag(&mut h, "Size:", 20.0, Modifiers::NONE);
    assert_eq!(h.state().0, 120.0);
    drag(&mut h, "Size:", -3.0, Modifiers::SHIFT);
    assert_eq!(h.state().0, 90.0);
    drag(&mut h, "Mode:", 20.0, Modifiers::NONE);
    assert_eq!(h.state().1, 100.0, "the field isn't right after the label");
}

/// A right-aligned OK / Cancel / Apply row as `os` draws it: labels left to right, and the
/// row's right edge with the window's.
fn button_row(os: egui::os::OperatingSystem) -> (Vec<String>, f32, f32) {
    use super::{ButtonRole, DialogButton};
    use egui_kittest::{Harness, kittest::Queryable};
    // Drawn from the second frame, once the theme's fonts are bound.
    let mut h = Harness::builder().with_size(egui::vec2(500.0, 80.0)).build_ui_state(
        |ui, ready: &mut bool| {
            if *ready {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                    let row = [
                        DialogButton::new(ButtonRole::Default, "OK", 84.0),
                        DialogButton::new(ButtonRole::Cancel, "Cancel", 84.0),
                        DialogButton::new(ButtonRole::Apply, "Apply", 84.0),
                    ];
                    super::dialog_buttons(ui, &row);
                });
            }
        },
        false,
    );
    crate::PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    h.ctx.set_os(os);
    *h.state_mut() = true;
    h.run();
    let mut drawn: Vec<(f32, f32, String)> =
        ["OK", "Cancel", "Apply"].iter().map(|l| (h.get_by_label(l).rect().left(), h.get_by_label(l).rect().right(), l.to_string())).collect();
    drawn.sort_by(|a, b| a.0.total_cmp(&b.0));
    let right = drawn.last().map_or(0.0, |d| d.1);
    (drawn.into_iter().map(|d| d.2).collect(), right, h.ctx.content_rect().right())
}

#[test]
fn dialog_buttons_follow_the_platform_order() {
    use egui::os::OperatingSystem as Os;
    for (os, want) in [(Os::Windows, ["OK", "Cancel", "Apply"]), (Os::Nix, ["OK", "Cancel", "Apply"]), (Os::Mac, ["Cancel", "Apply", "OK"])] {
        let (order, right, edge) = button_row(os);
        assert_eq!(order, want, "{os:?}");
        assert!(edge - right < 20.0, "{os:?}: the row hugs the right edge ({right} of {edge})");
    }
}

#[test]
fn two_decimal_numbers_trim_zeros() {
    assert_eq!(super::fmt_num2(1.05), "1.05");
    assert_eq!(super::fmt_num2(0.78), "0.78");
    assert_eq!(super::fmt_num2(0.5), "0.5");
    assert_eq!(super::fmt_num2(2.0), "2");
}

#[test]
fn typed_arithmetic_evaluates() {
    use super::parse_num;
    for (text, want) in [
        ("1280*2", 2560.0),
        ("658 * 1.5", 987.0),
        ("48/3", 16.0),
        ("20*2+5-2", 43.0),
        ("2+3*4", 14.0),
        ("10/4*2", 5.0),
        ("-5+3", -2.0),
        ("5--3", 8.0),
        ("2*-3+1", -5.0),
        ("−4", -4.0),
        ("1 234", 1234.0),
        ("1e3/2", 500.0),
        ("(2+3)*4", 20.0),
        ("2^3", 8.0),
        ("10%3", 1.0),
    ] {
        assert_eq!(parse_num(text), Some(want), "{text}");
    }
    for text in ["", "abc", "5+", "*2", "4/0", "0/0", "1+*2", "(2+3", "1e400"] {
        assert_eq!(parse_num(text), None, "{text}");
    }
}

/// Types `text` into a value field holding `start`, then presses `key`. Returns the value an OK
/// button that also fires on Enter saw, as dialogs read it, else the field's value. `round`
/// makes the caller round the value every frame, as pixel fields do.
fn type_and_press(start: f32, text: &str, key: egui::Key, round: bool) -> f32 {
    use egui_kittest::{Harness, kittest::Queryable};
    let mut h = Harness::builder().with_size(egui::vec2(300.0, 100.0)).build_ui_state(
        move |ui, (v, ok): &mut (f32, Option<f32>)| {
            super::value_field(ui, v, 0.0..=300000.0, "px", 90.0).accessible_name("Size");
            if round {
                *v = v.round();
            }
            if ui.button(tl!("OK")).clicked() || ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                *ok = Some(*v);
            }
        },
        (start, None),
    );
    h.get_by_role(egui::accesskit::Role::SpinButton).click();
    h.run();
    for c in text.chars() {
        h.event(egui::Event::Text(c.to_string()));
        h.run();
    }
    h.key_press(key);
    h.run();
    let (v, ok) = *h.state();
    ok.unwrap_or(v)
}

/// Plain digits apply as they're typed; arithmetic doesn't report a change until it's committed.
#[test]
fn value_field_holds_arithmetic_until_committed() {
    use egui_kittest::{Harness, kittest::Queryable};
    let mut h = Harness::builder().with_size(egui::vec2(300.0, 100.0)).build_ui_state(
        |ui, (v, changes): &mut (f32, u32)| {
            if super::value_field(ui, v, 0.0..=100.0, "%", 90.0).accessible_name("Opacity").changed() {
                *changes += 1;
            }
        },
        (100.0, 0),
    );
    h.get_by_role(egui::accesskit::Role::SpinButton).click();
    h.run();
    for (c, want) in [('5', (5.0, 1)), ('0', (50.0, 2)), ('/', (50.0, 2)), ('2', (50.0, 2))] {
        h.event(egui::Event::Text(c.to_string()));
        h.run();
        assert_eq!(*h.state(), want, "after {c}");
    }
    h.key_press(egui::Key::Enter);
    h.run();
    assert_eq!(*h.state(), (25.0, 3));
}

/// #5717: a value past the range shows as it is without reporting a change; an edit is clamped.
#[test]
fn value_field_keeps_a_shown_value_past_its_range_and_clamps_edits() {
    use egui_kittest::{Harness, kittest::Queryable};
    let mut h = Harness::builder().with_size(egui::vec2(300.0, 100.0)).build_ui_state(
        |ui, (v, changes): &mut (f32, u32)| {
            if super::value_field(ui, v, 0.1..=1296.0, "pt", 90.0).accessible_name("Font size").changed() {
                *changes += 1;
            }
        },
        (2400.0, 0),
    );
    h.run_steps(3);
    assert_eq!(*h.state(), (2400.0, 0), "showing it changes nothing");
    h.get_by_role(egui::accesskit::Role::SpinButton).click();
    h.run();
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.event(egui::Event::Text("5000".into()));
    h.key_press(egui::Key::Enter);
    h.run();
    assert_eq!(h.state().0, 1296.0, "a typed value is clamped");
}

#[test]
fn value_field_applies_typed_arithmetic() {
    use egui::Key::{Enter, Tab};
    assert_eq!(type_and_press(500.0, "1280*2", Tab, false), 2560.0);
    assert_eq!(type_and_press(500.0, "1280*2", Enter, false), 2560.0);
    // `5/2` alone would round to 3 under the caller; the whole expression still applies.
    assert_eq!(type_and_press(1.0, "5/2*2", Tab, true), 5.0);
    assert_eq!(type_and_press(1.0, "1280/3*2", Enter, true), 853.0);
}

#[test]
fn numbers_drop_trailing_zero() {
    // #2434: the unit keys map to Photoshop's places, and the shared formatter trims zeros.
    assert_eq!(super::unit_decimals("px"), 0);
    assert_eq!(super::unit_decimals("pt"), 1);
    assert_eq!(super::unit_decimals("mm"), 2);
    assert_eq!(super::unit_decimals("cm"), 2);
    assert_eq!(super::unit_decimals("pica"), 2);
    assert_eq!(super::unit_decimals("in"), 3);
    assert_eq!(super::unit_decimals("nonsense"), 0);
    assert_eq!(super::fmt_num_prec(0.7995, 3), "0.8");
    assert_eq!(super::fmt_num_prec(2.5, 0), "3");
    assert_eq!(super::fmt_num(100.0), "100");
    assert_eq!(super::fmt_num(12.46), "12.5");
    assert_eq!(super::fmt_num(-3.0), "-3");
}

#[test]
fn dropdown_opens_with_focus_and_arrow_keys_change_the_value() {
    use egui::accesskit::Role;
    use egui_kittest::{Harness, kittest::Queryable};

    let mut h = Harness::builder().with_size(egui::vec2(300.0, 100.0)).build_ui_state(
        |ui, selected: &mut usize| {
            let options = [(0, "Normal"), (1, "Multiply"), (2, "Screen")];
            super::dropdown(ui, "blend-mode", tl!("Blend Mode"), selected, &options, 120.0);
        },
        0,
    );
    h.get_by_role(Role::ComboBox).click();
    h.run();
    h.key_press(egui::Key::ArrowDown);
    h.run();
    assert_eq!(*h.state(), 1);
    h.key_press(egui::Key::ArrowUp);
    h.run();
    assert_eq!(*h.state(), 0);
}

/// Scrolling rows under the pointer doesn't widen the list: it is as wide as the widest
/// option hovered from the start (#2607).
#[test]
fn dropdown_list_keeps_its_width_while_scrolling() {
    use photocraft_color::BlendMode;
    let mut h = Harness::builder().with_size(egui::vec2(400.0, 700.0)).build_ui_state(
        |ui, mode: &mut BlendMode| {
            let opts: Vec<(BlendMode, &str)> = BlendMode::LAYER_MODES.iter().map(|m| (*m, m.label())).collect();
            super::dropdown(ui, "brush-mode", tl!("Mode"), mode, &opts, 96.0);
        },
        BlendMode::Normal,
    );
    // The app's theme strokes plain rows, which is what made hovered rows wider.
    crate::theme::apply(&h.ctx, crate::theme::ThemeKind::ProMedium);
    h.run();
    h.get_by_role(egui::accesskit::Role::ComboBox).click();
    h.run();
    let list = |h: &Harness<'_, BlendMode>| h.ctx.memory(|m| m.layer_ids().filter(|l| l.order == egui::Order::Foreground).find_map(|l| m.area_rect(l.id)));
    let Some(open) = list(&h) else { panic!("the list is open") };
    for _ in 0..30 {
        h.hover_at(open.center());
        h.event(egui::Event::MouseWheel {
            source: egui::MouseWheelSource::Unknown,
            unit: egui::MouseWheelUnit::Line,
            delta: egui::vec2(0.0, -1.0),
            phase: egui::TouchPhase::Move,
            modifiers: Modifiers::NONE,
        });
        h.run();
        assert_eq!(list(&h).map(|r| r.width()), Some(open.width()));
    }
}

const MODES: [(usize, &str); 4] = [(0, "Normal"), (1, "Dissolve"), (2, "Darken"), (3, "Multiply")];

/// A wheel dropdown (or a plain one) of [`MODES`], whose state is the chosen index.
fn mode_dropdown(wheel: bool) -> Harness<'static, usize> {
    Harness::builder().with_size(egui::vec2(300.0, 100.0)).build_ui_state(
        move |ui, selected: &mut usize| {
            if wheel {
                super::dropdown_wheel_hovered(ui, "blend-mode", tl!("Blend Mode"), selected, &MODES, 120.0);
            } else {
                super::dropdown(ui, "blend-mode", tl!("Blend Mode"), selected, &MODES, 120.0);
            }
        },
        0,
    )
}

/// One wheel event over the dropdown: negative `delta.y` is down.
fn wheel<T>(h: &mut Harness<'_, T>, unit: egui::MouseWheelUnit, delta: egui::Vec2, modifiers: Modifiers) {
    let at = h.get_by_role(egui::accesskit::Role::ComboBox).rect().center();
    h.event(egui::Event::ModifiersChanged(modifiers));
    h.hover_at(at);
    h.run();
    h.event(egui::Event::MouseWheel { source: egui::MouseWheelSource::Unknown, unit, delta, phase: egui::TouchPhase::Move, modifiers });
    h.run();
    h.event(egui::Event::ModifiersChanged(Modifiers::NONE));
    h.run();
}

#[test]
fn shift_wheel_steps_one_mode_per_notch() {
    let mut h = mode_dropdown(true);
    wheel(&mut h, egui::MouseWheelUnit::Line, egui::vec2(0.0, -1.0), Modifiers::SHIFT);
    assert_eq!(*h.state(), 1, "not ten like a slider");
    // Some systems turn Shift+wheel into a horizontal scroll.
    wheel(&mut h, egui::MouseWheelUnit::Line, egui::vec2(-1.0, 0.0), Modifiers::SHIFT);
    assert_eq!(*h.state(), 2);
}

#[test]
fn fractional_wheel_deltas_add_up_to_whole_mode_steps() {
    let mut h = mode_dropdown(true);
    wheel(&mut h, egui::MouseWheelUnit::Line, egui::vec2(0.0, -0.5), Modifiers::NONE);
    assert_eq!(*h.state(), 0, "half a notch");
    wheel(&mut h, egui::MouseWheelUnit::Line, egui::vec2(0.0, -0.5), Modifiers::NONE);
    assert_eq!(*h.state(), 1);
    let per_line = h.ctx.options(|o| o.input_options.line_scroll_speed);
    wheel(&mut h, egui::MouseWheelUnit::Point, egui::vec2(0.0, -per_line), Modifiers::NONE);
    assert_eq!(*h.state(), 2, "a line's worth of points is a notch");
}

#[test]
fn the_wheel_over_a_wheel_dropdown_does_not_scroll_its_container() {
    let mut h = Harness::builder().with_size(egui::vec2(300.0, 100.0)).build_ui_state(
        |ui, selected: &mut usize| {
            egui::ScrollArea::vertical().max_height(80.0).show(ui, |ui| {
                super::dropdown_wheel_hovered(ui, "blend-mode", tl!("Blend Mode"), selected, &MODES, 120.0);
                ui.add_space(400.0);
            });
        },
        0,
    );
    let top = h.get_by_role(egui::accesskit::Role::ComboBox).rect().top();
    wheel(&mut h, egui::MouseWheelUnit::Line, egui::vec2(0.0, -1.0), Modifiers::NONE);
    for _ in 0..10 {
        h.run();
    }
    assert_eq!(*h.state(), 1);
    assert_eq!(h.get_by_role(egui::accesskit::Role::ComboBox).rect().top(), top, "the container didn't scroll");
}

#[test]
fn the_wheel_leaves_a_plain_dropdown_alone() {
    let mut h = mode_dropdown(false);
    wheel(&mut h, egui::MouseWheelUnit::Line, egui::vec2(0.0, -1.0), Modifiers::NONE);
    assert_eq!(*h.state(), 0);
}
