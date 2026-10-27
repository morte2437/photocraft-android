//! Pointer, keyboard, colour precision and history regressions for shape appearance controls.
use super::*;
use crate::shape_options::set_tool_paint;
use crate::shape_paint_ui::color_of;
use egui::{Event, Key, Modifiers, PointerButton};
use egui_kittest::{Harness, kittest::Queryable};
use photocraft_doc::ShapeLayer;

fn app(mode: &str, depth: u32) -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 120, "height": 100, "mode": mode, "depth": depth})).unwrap();
    app.ui.tool = Tool::Rectangle;
    app.sync_views();
    app
}

fn shape(app: &PhotocraftApp) -> &ShapeLayer {
    let st = app.session.active().unwrap();
    let LayerContent::Shape(shape) = &st.doc.layer(st.active_layer.unwrap()).unwrap().content else { panic!("expected editable shape") };
    shape
}

fn assert_saved_path(saved: &Path, original: &Path) {
    assert_eq!((saved.fill_rule, saved.inverted), (original.fill_rule, original.inverted));
    assert_eq!(saved.subpaths.len(), original.subpaths.len());
    for (saved, original) in saved.subpaths.iter().zip(&original.subpaths) {
        assert_eq!((saved.closed, saved.op, saved.knots.len()), (original.closed, original.op, original.knots.len()));
        for (saved, original) in saved.knots.iter().zip(&original.knots) {
            assert_eq!(saved.smooth, original.smooth);
            for (saved, original) in [(saved.anchor, original.anchor), (saved.in_ctrl, original.in_ctrl), (saved.out_ctrl, original.out_ctrl)] {
                // Native manifests use JSON f64 coordinates, whose existing round trip may
                // differ by one ulp for generated ellipse/polygon control points.
                assert!((saved.x - original.x).abs() <= 1e-10 && (saved.y - original.y).abs() <= 1e-10, "{saved:?} differs from {original:?}");
            }
        }
    }
}

fn harness(app: PhotocraftApp, properties: bool) -> Harness<'static, PhotocraftApp> {
    // kittest's default step: 0.25 s a frame.
    harness_stepped(app, properties, 0.25)
}

/// [`harness`] advancing `step_dt` seconds a frame (1/60 lets two clicks fall within egui's
/// 0.3 s double-click window, as they do in the app).
fn harness_stepped(app: PhotocraftApp, properties: bool, step_dt: f32) -> Harness<'static, PhotocraftApp> {
    let mut h = Harness::builder().with_size(vec2(1000.0, 700.0)).with_step_dt(step_dt).build_ui_state(
        move |ui, app: &mut PhotocraftApp| {
            crate::screen_picker::tick(app, ui.ctx());
            if !ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            if properties {
                let id = app.session.active().unwrap().active_layer.unwrap();
                ui.set_max_width(320.0);
                shape_properties(app, ui, id);
            } else {
                let tool = app.ui.tool;
                ui.horizontal(|ui| options_bar(app, ui, tool));
            }
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::Pro);
    h.run_steps(3);
    h
}

fn pointer(h: &mut Harness<'_, PhotocraftApp>, pos: Pos2, pressed: bool) {
    h.event(Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
    h.step();
}

fn stroke_width(h: &mut Harness<'_, PhotocraftApp>, width: &str) {
    h.get_all_by_role(egui::accesskit::Role::SpinButton).next().expect("the stroke width").click();
    h.run();
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    h.event(Event::Text(width.into()));
    h.key_press(Key::Enter);
    h.run_steps(3);
}

/// #6191: passing through zero hides the stroke without discarding its paint or style.
#[test]
fn zero_width_preserves_selected_shape_stroke_and_restores_it() {
    for depth in [8, 16, 32] {
        for (properties, tool) in [(true, Tool::Rectangle), (false, Tool::Rectangle), (false, Tool::PathSelection), (false, Tool::DirectSelection)] {
            let mut app = app("rgb", depth);
            app.ui.tool = tool;
            app.run(
                "shape.create",
                json!({"rect": [20, 20, 80, 60], "fill": null,
                    "stroke": {"width": 4, "color": [0.123456, 0.345678, 0.567891, 0.618237],
                        "align": "inside", "cap": "round", "join": "bevel", "miterLimit": 7,
                        "dashes": [3, 2], "dashOffset": 1.25, "opacity": 63}}),
            )
            .unwrap();
            let original = shape(&app).clone();
            let history = app.session.active().unwrap().history.past_len();
            let mut h = harness(app, properties);
            stroke_width(&mut h, "0");
            let zero = shape(h.state()).clone();
            let mut expected = original.stroke.clone().unwrap();
            expected.width = 0.0;
            assert_eq!(zero.stroke, Some(expected), "{tool:?}, Properties={properties}, depth={depth}: zero width retains the stroke");
            assert_eq!(zero.path, original.path);
            assert!(zero.cache.as_ref().unwrap().content_bounds().is_empty(), "a zero-width stroke paints nothing");
            assert_eq!(h.state().session.active().unwrap().history.past_len(), history + 1);

            // A save at zero must retain the dormant stroke too.
            let st = h.state().session.active().unwrap();
            let bytes = photocraft_format::save_to_bytes(&st.doc, &Default::default()).unwrap();
            let loaded = photocraft_format::load_from_bytes(&bytes).unwrap();
            let LayerContent::Shape(saved) = &loaded.layer(st.active_layer.unwrap()).unwrap().content else { panic!("saved shape") };
            assert_eq!(saved.stroke, zero.stroke);

            stroke_width(&mut h, "4");
            assert_eq!(shape(h.state()), &original, "restoring width restores the complete style and rendered pixels");
            h.state_mut().run("edit.undo", json!({})).unwrap();
            assert_eq!(shape(h.state()), &zero);
            h.state_mut().run("edit.undo", json!({})).unwrap();
            assert_eq!(shape(h.state()), &original);
            h.state_mut().run("edit.redo", json!({})).unwrap();
            assert_eq!(shape(h.state()), &zero);
            h.state_mut().run("edit.redo", json!({})).unwrap();
            assert_eq!(shape(h.state()), &original);
        }
    }
}

#[test]
fn zero_width_style_can_be_inspected_removed_or_reenabled_explicitly() {
    for properties in [true, false] {
        let mut app = app("rgb", 8);
        app.run("shape.create", json!({"rect": [20, 20, 80, 60], "stroke": {"width": 4, "color": "#e73581", "dashes": [3, 2]}})).unwrap();
        let paint = shape(&app).stroke.as_ref().unwrap().paint.clone();
        let mut h = harness(app, properties);
        stroke_width(&mut h, "0");
        let zero = shape(h.state()).clone();
        h.get_by_label("Stroke Options").click();
        h.run_steps(3);
        assert_eq!(h.state().ui.stroke_editor.as_ref().unwrap().draft.dashes, [3.0, 2.0], "zero width still exposes the saved options");
        h.key_press(Key::Escape);
        h.run_steps(2);
        assert_eq!(shape(h.state()), &zero, "merely inspecting the style does not enable it");

        h.get_by_label(if properties { "Set shape stroke type" } else { "Stroke color" }).click();
        h.run_steps(3);
        h.get_by_label("No Color").click();
        h.run_steps(2);
        assert!(shape(h.state()).stroke.is_none(), "No Color still removes the stroke");
        h.key_press(Key::Escape);
        h.run_steps(2);
        h.state_mut().run("edit.undo", json!({})).unwrap();
        assert_eq!(shape(h.state()), &zero);
        h.run_steps(2);

        h.get_by_label("Stroke Options").click();
        h.run_steps(3);
        h.get_by_role_and_label(egui::accesskit::Role::RadioButton, "Dotted").click();
        h.run_steps(3);
        let stroke = shape(h.state()).stroke.as_ref().unwrap();
        assert_eq!(stroke.width, 3.0, "a deliberate style choice enables the dormant stroke");
        assert_eq!(stroke.paint, paint);
        assert_eq!(stroke.dashes, [0.0, 2.0]);
    }
}

#[test]
fn fill_and_stroke_swatches_open_with_pointer_and_keyboard_without_changing_paint() {
    for label in ["Fill color", "Stroke color"] {
        let mut app = app("rgb", 32);
        let foreground = [0.123456, 0.345678, 0.567891, 0.372819];
        let background = [0.891234, 0.678912, 0.456789, 0.0];
        app.run("tools.setColors", json!({"foreground": foreground, "background": background})).unwrap();
        let document = app.session.active().unwrap().doc.clone();
        let mut h = harness(app, false);
        h.get_by_label(label).click();
        h.run_steps(4);
        assert!(h.query_by_label("No Color").is_some(), "{label} opens its picker");
        assert_eq!(h.state().session.tools.foreground, foreground);
        assert_eq!(h.state().session.tools.background, background);
        assert_eq!(h.state().ui.tool_options.stroke_width, 0.0);
        assert_eq!(h.state().session.active().unwrap().doc, document);
        h.key_press(Key::Escape);
        h.run_steps(2);
        assert!(h.query_by_label("No Color").is_none());
        h.get_by_label(label).focus();
        h.run_steps(2);
        h.key_press(Key::Enter);
        h.run_steps(3);
        assert!(h.query_by_label("No Color").is_some(), "{label} opens from keyboard focus");
        h.key_press(Key::Escape);
        h.run_steps(2);
        assert_eq!(h.state().session.active().unwrap().doc, document);
        assert_eq!(h.state().session.tools.foreground, foreground);
        assert_eq!(h.state().session.tools.background, background);
    }
}

#[test]
fn shape_paint_hex_input_keeps_partial_text_and_preserves_alpha() {
    for label in ["Fill color", "Stroke color"] {
        let mut app = app("rgb", 32);
        let original = [0.123456, 0.345678, 0.567891, 0.372819];
        app.run("tools.setColors", json!({"foreground": original, "background": original})).unwrap();
        let mut h = harness(app, false);
        h.get_by_label(label).click();
        h.run_steps(3);
        h.get_by_label("#").click();
        h.run_steps(1);
        h.key_press_modifiers(Modifiers::COMMAND, Key::A);
        h.run_steps(1);
        for ch in "12AB".chars() {
            h.event(Event::Text(ch.to_string()));
            h.run_steps(1);
        }
        assert_eq!(h.get_by_label("#").value().as_deref(), Some("12AB"));
        assert_eq!(h.state().session.tools.foreground, original);
        assert_eq!(h.state().session.tools.background, original);
        for ch in "EF".chars() {
            h.event(Event::Text(ch.to_string()));
            h.run_steps(1);
        }
        let want = [18.0 / 255.0, 171.0 / 255.0, 239.0 / 255.0, original[3]];
        let tools = &h.state().session.tools;
        assert_eq!(if label == "Fill color" { tools.foreground } else { tools.background }, want);
        assert_eq!(if label == "Fill color" { tools.background } else { tools.foreground }, original);
        // An invalid/multi-byte value leaves the selected paint untouched.
        h.key_press_modifiers(Modifiers::COMMAND, Key::A);
        h.run_steps(1);
        h.event(Event::Text("颜色".into()));
        h.run_steps(2);
        let tools = &h.state().session.tools;
        assert_eq!(if label == "Fill color" { tools.foreground } else { tools.background }, want);
        h.key_press(Key::Escape);
        h.run_steps(2);
        finish_shape(h.state_mut(), Tool::Rectangle, [20.0, 20.0], [100.0, 80.0], Modifiers::NONE);
        let drawn = shape(h.state());
        let paint = if label == "Fill color" { drawn.fill.as_ref().and_then(color_of) } else { drawn.stroke.as_ref().and_then(|s| color_of(&s.paint)) };
        assert_eq!(paint, Some(want), "the next shape takes the typed colour");
    }
}

#[test]
fn a_pasted_hex_colour_updates_the_selected_shape_and_undoes_once() {
    for label in ["Fill color", "Stroke color"] {
        let mut app = app("rgb", 32);
        app.run("tools.setColors", json!({"foreground": [0.2, 0.3, 0.4, 0.6], "background": [0.5, 0.6, 0.7, 0.8]})).unwrap();
        app.ui.tool_options.stroke_width = 3.0;
        finish_shape(&mut app, Tool::Rectangle, [20.0, 20.0], [100.0, 80.0], Modifiers::NONE);
        let original = shape(&app).clone();
        let history = app.session.active().unwrap().history.past_len();
        let mut h = harness(app, false);
        h.get_by_label(label).click();
        h.run_steps(3);
        h.get_by_label("#").click();
        h.run_steps(1);
        h.key_press_modifiers(Modifiers::COMMAND, Key::A);
        h.run_steps(1);
        h.event(Event::Paste("#12ABEF".into()));
        h.run_steps(2);
        let edited = shape(h.state());
        let (paint, untouched) = if label == "Fill color" {
            (edited.fill.as_ref().and_then(color_of), edited.stroke == original.stroke)
        } else {
            (edited.stroke.as_ref().and_then(|s| color_of(&s.paint)), edited.fill == original.fill)
        };
        let alpha = if label == "Fill color" { 0.6 } else { 0.8 };
        let want = [18.0 / 255.0, 171.0 / 255.0, 239.0 / 255.0, alpha];
        assert_eq!(paint, Some(want));
        assert!(untouched, "the other paint stays unchanged");
        assert_eq!(h.state().session.active().unwrap().history.past_len(), history + 1);
        h.state_mut().run("edit.undo", json!({})).unwrap();
        assert_eq!(shape(h.state()), &original);
    }
}

#[test]
fn picking_tool_colours_enables_paint_and_reaches_the_next_shape() {
    let mut app = app("rgb", 32);
    let mut picks = std::collections::VecDeque::from([[0.91, 0.17, 0.32], [0.12, 0.68, 0.93]]);
    app.services.screen_pick = Some(Box::new(move |_| {
        let (tx, receiver) = std::sync::mpsc::channel();
        tx.send(Ok(crate::screen_picker::Capture::Color(picks.pop_front()))).unwrap();
        crate::screen_picker::Pending { receiver, cancelled: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)) }
    }));
    app.ui.tool_options.shape_fill = false;
    let mut h = harness(app, false);
    for label in ["Fill color", "Stroke color"] {
        h.get_by_label(label).click();
        h.run_steps(3);
        h.get_by_label("Pick screen color").click();
        h.run_steps(4);
        h.key_press(Key::Escape);
        h.run_steps(2);
    }
    assert!(h.state().ui.tool_options.shape_fill);
    assert_eq!(h.state().ui.tool_options.stroke_width, 3.0);
    assert_eq!(h.state().session.tools.foreground, [0.91, 0.17, 0.32, 1.0]);
    assert_eq!(h.state().session.tools.background, [0.12, 0.68, 0.93, 1.0]);
    finish_shape(h.state_mut(), Tool::Rectangle, [20.0, 20.0], [100.0, 80.0], Modifiers::NONE);
    assert_eq!(shape(h.state()).fill, Some(solid_paint([0.91, 0.17, 0.32, 1.0])));
    assert_eq!(shape(h.state()).stroke.as_ref().unwrap().paint, solid_paint([0.12, 0.68, 0.93, 1.0]));
}

#[test]
fn no_colour_and_solid_choices_are_explicit_live_tool_defaults() {
    let mut h = harness(app("rgb", 8), false);
    h.get_by_label("Fill color").click();
    h.run_steps(3);
    h.get_by_label("No Color").click();
    h.run_steps(2);
    assert!(!h.state().ui.tool_options.shape_fill);
    h.get_by_label("Solid").click();
    h.run_steps(2);
    assert!(h.state().ui.tool_options.shape_fill);
    h.key_press(Key::Escape);
    h.run_steps(2);
    h.get_by_label("Stroke color").click();
    h.run_steps(3);
    h.get_by_label("Solid").click();
    h.run_steps(2);
    assert_eq!(h.state().ui.tool_options.stroke_width, 3.0, "choosing unchanged white enables a stroke");
    h.get_by_label("No Color").click();
    h.run_steps(2);
    assert_eq!(h.state().ui.tool_options.stroke_width, 0.0);
    assert_eq!(h.state().session.active().unwrap().history.past_len(), 0, "tool defaults do not edit the document");
}

#[test]
fn float_paints_match_preview_creation_native_roundtrip_and_document_depth() {
    let foreground = [0.123456, 0.345678, 0.567891, 0.372819];
    let background = [0.891234, 0.234567, 0.456789, 0.618237];
    for (mode, depth) in [("rgb", 8), ("rgb", 16), ("rgb", 32), ("gray", 16), ("cmyk", 8), ("lab", 16)] {
        for tool in [Tool::Rectangle, Tool::Triangle, Tool::EllipseShape] {
            let mut app = app(mode, depth);
            set_tool_paint(&mut app, false, PaintChange::Color(foreground)).unwrap();
            set_tool_paint(&mut app, true, PaintChange::Color(background)).unwrap();
            app.ui.tool_options.shape_stroke.cap = photocraft_doc::LineCap::Round;
            app.ui.tool_options.shape_stroke.dashes = vec![0.0, 2.0];
            let preview = shape_preview(&app, tool, [20.0, 20.0], [100.0, 80.0], Modifiers::NONE).unwrap();
            finish_shape(&mut app, tool, [20.0, 20.0], [100.0, 80.0], Modifiers::NONE);
            let committed = shape(&app).clone();
            assert_eq!(preview.fill, committed.fill);
            assert_eq!(preview.stroke, committed.stroke);
            assert_eq!(preview.path, committed.path);
            assert_eq!(committed.fill, Some(solid_paint(foreground)));
            assert_eq!(committed.stroke.as_ref().unwrap().paint, solid_paint(background));
            let doc = &app.session.active().unwrap().doc;
            let rendered = photocraft_vector::render_shape(&preview, doc.pixel_format(), doc.bounds());
            assert_eq!(committed.cache.as_ref().unwrap(), &rendered, "{tool:?} {mode}/{depth}");
            let bytes = photocraft_format::save_to_bytes(doc, &Default::default()).unwrap();
            let loaded = photocraft_format::load_from_bytes(&bytes).unwrap();
            let id = app.session.active().unwrap().active_layer.unwrap();
            let LayerContent::Shape(saved) = &loaded.layer(id).unwrap().content else { panic!("saved editable shape") };
            assert_eq!((&saved.fill, &saved.stroke), (&committed.fill, &committed.stroke));
            assert_saved_path(&saved.path, &committed.path);
            app.run("edit.undo", json!({})).unwrap();
            app.run("edit.redo", json!({})).unwrap();
            assert_eq!(shape(&app), &committed);
        }
    }
}

#[test]
fn transparent_and_absent_paints_remain_distinct_and_open_pen_keeps_foreground() {
    let mut app = app("rgb", 32);
    let transparent = [0.124578, 0.564321, 0.765432, 0.0];
    set_tool_paint(&mut app, false, PaintChange::Color(transparent)).unwrap();
    set_tool_paint(&mut app, true, PaintChange::Color(transparent)).unwrap();
    let preview = shape_preview(&app, Tool::Rectangle, [20.0, 20.0], [80.0, 70.0], Modifiers::NONE).unwrap();
    assert_eq!(preview.fill, Some(solid_paint(transparent)));
    assert_eq!(preview.stroke.as_ref().unwrap().paint, solid_paint(transparent));
    finish_shape(&mut app, Tool::Rectangle, [20.0, 20.0], [80.0, 70.0], Modifiers::NONE);
    assert_eq!(shape(&app).fill, preview.fill);
    assert_eq!(shape(&app).stroke, preview.stroke);
    assert_eq!(shape(&app).cache.as_ref().unwrap().pixel(40, 40), vec![0.0; 4]);
    set_tool_paint(&mut app, false, PaintChange::None).unwrap();
    set_tool_paint(&mut app, true, PaintChange::None).unwrap();
    finish_shape(&mut app, Tool::Triangle, [20.0, 20.0], [80.0, 70.0], Modifiers::NONE);
    assert!(shape(&app).fill.is_none() && shape(&app).stroke.is_none());
    let foreground = [0.836521, 0.125468, 0.745632, 0.438271];
    app.run("tools.setColors", json!({"foreground": foreground, "background": [0.0, 1.0, 0.0, 1.0]})).unwrap();
    app.ui.tool_options.vector_mode = "shape".into();
    app.ui.pen = Some(PenPath { knots: vec![[[20.0, 20.0]; 3], [[80.0, 60.0]; 3]], ..Default::default() });
    pen_commit(&mut app, false);
    assert!(shape(&app).fill.is_none());
    assert!(!shape(&app).path.subpaths[0].closed);
    assert_eq!(shape(&app).stroke.as_ref().unwrap().paint, solid_paint(foreground));
}

#[test]
fn selected_shape_style_is_available_when_absent_and_dismissal_does_not_enable_it() {
    let mut app = app("rgb", 8);
    app.run("shape.create", json!({"rect": [20,20,80,60], "stroke": null})).unwrap();
    let doc = app.session.active().unwrap().doc.clone();
    let history = app.session.active().unwrap().history.past_len();
    let mut h = harness(app, true);
    h.get_by_label("Stroke Options").click();
    h.run_steps(3);
    h.key_press(Key::Escape);
    h.run_steps(2);
    assert_eq!(h.state().session.active().unwrap().doc, doc);
    h.get_by_label("Stroke Options").click();
    h.run_steps(3);
    h.get_by_role_and_label(egui::accesskit::Role::RadioButton, "Dotted").click();
    h.run_steps(3);
    assert_eq!(shape(h.state()).stroke.as_ref().unwrap().width, 3.0);
    assert_eq!(shape(h.state()).stroke.as_ref().unwrap().dashes, [0.0, 2.0]);
    assert_eq!(h.state().session.active().unwrap().history.past_len(), history + 1);
    h.key_press(Key::Escape);
    h.run_steps(2);
    h.state_mut().run("edit.undo", json!({})).unwrap();
    assert!(shape(h.state()).stroke.is_none());
    h.state_mut().run("edit.redo", json!({})).unwrap();
    assert_eq!(shape(h.state()).stroke.as_ref().unwrap().dashes, [0.0, 2.0]);
}

fn popup_rect(h: &Harness<'_, PhotocraftApp>) -> Rect {
    h.ctx
        .memory(|m| m.areas().visible_layer_ids().into_iter().filter(|l| l.order == egui::Order::Foreground).find_map(|l| m.area_rect(l.id)))
        .expect("colour picker is open")
}

#[test]
fn selected_shape_colour_drags_apply_live_coalesce_per_gesture_and_keep_tool_defaults() {
    let mut app = app("rgb", 32);
    app.run("shape.create", json!({"rect": [20,20,80,60], "fill": "#3270c0", "stroke": {"width": 5, "color": "#ee3355"}})).unwrap();
    let original = shape(&app).clone();
    let defaults = (app.session.tools.foreground, app.session.tools.background, app.ui.tool_options.clone());
    let history = app.session.active().unwrap().history.past_len();
    let mut h = harness(app, true);
    h.get_by_label("Set shape fill type").click();
    h.run_steps(4);
    assert_eq!(h.state().session.active().unwrap().history.past_len(), history);
    let rect = popup_rect(&h);
    let start = rect.min + vec2(55.0, rect.height() * 0.5);
    h.hover_at(start);
    h.step();
    pointer(&mut h, start, true);
    for offset in [10.0, 20.0, 30.0] {
        h.event(Event::PointerMoved(start + vec2(offset, 6.0)));
        h.step();
    }
    pointer(&mut h, start + vec2(30.0, 6.0), false);
    h.run_steps(2);
    let first = shape(h.state()).clone();
    assert_ne!(first.fill, original.fill);
    assert_eq!(h.state().session.active().unwrap().history.past_len(), history + 1);
    let next = start + vec2(60.0, 15.0);
    h.hover_at(next);
    h.step();
    pointer(&mut h, next, true);
    h.event(Event::PointerMoved(next + vec2(12.0, 4.0)));
    h.step();
    pointer(&mut h, next + vec2(12.0, 4.0), false);
    h.run_steps(2);
    assert_eq!(h.state().session.active().unwrap().history.past_len(), history + 2);
    h.key_press(Key::Escape);
    h.run_steps(2);
    assert_eq!((h.state().session.tools.foreground, h.state().session.tools.background, h.state().ui.tool_options.clone()), defaults);
    h.state_mut().run("edit.undo", json!({})).unwrap();
    assert_eq!(shape(h.state()).fill, first.fill);
    h.state_mut().run("edit.undo", json!({})).unwrap();
    assert_eq!(shape(h.state()).fill, original.fill);
    h.state_mut().run("edit.redo", json!({})).unwrap();
    assert_eq!(shape(h.state()).fill, first.fill);
}

/// #3997, #3999: after drawing a shape, the options bar's Fill swatch recolours it, as Stroke
/// does (in v0.7.0 the Fill swatch was only a preview of the foreground colour).
#[test]
fn options_bar_fill_swatch_recolours_the_shape_just_drawn() {
    let mut app = app("rgb", 8);
    app.run("tools.setColors", json!({"foreground": "#ff0000", "background": "#0000ff"})).unwrap();
    app.ui.tool_options.stroke_width = 3.0;
    finish_shape(&mut app, Tool::Rectangle, [20.0, 20.0], [100.0, 80.0], Modifiers::NONE);
    let original = shape(&app).clone();
    let history = app.session.active().unwrap().history.past_len();
    let mut h = harness(app, false);
    h.get_by_label("Fill color").click();
    h.run_steps(4);
    let rect = popup_rect(&h);
    let start = rect.min + vec2(55.0, rect.height() * 0.5);
    h.hover_at(start);
    h.step();
    pointer(&mut h, start, true);
    h.event(Event::PointerMoved(start + vec2(30.0, 6.0)));
    h.step();
    pointer(&mut h, start + vec2(30.0, 6.0), false);
    h.run_steps(2);
    let edited = shape(h.state()).clone();
    assert_ne!(edited.fill, original.fill, "the Fill swatch recoloured the shape");
    assert_eq!(edited.fill.as_ref().and_then(color_of), Some(h.state().session.tools.foreground), "and the next shape's fill");
    assert_eq!(edited.stroke, original.stroke, "the stroke is untouched");
    assert_eq!(h.state().session.active().unwrap().history.past_len(), history + 1);
    h.state_mut().run("edit.undo", json!({})).unwrap();
    assert_eq!(shape(h.state()).fill, original.fill);
}

fn test_gradient() -> photocraft_doc::Fill {
    photocraft_doc::Fill::Gradient {
        stops: vec![(0.0, photocraft_doc::Color::rgba(0.123456, 0.5, 0.75, 1.0)), (1.0, photocraft_doc::Color::rgba(0.9, 0.2, 0.1, 0.6))],
        angle: 30.0,
        scale: 0.5,
        style: photocraft_doc::GradientStyle::Radial,
        reverse: true,
        opacity_stops: vec![(0.0, 1.0), (1.0, 0.25)],
        midpoints: vec![0.3],
        offset: (0.125, -0.25),
        dither: true,
        align: false,
    }
}

/// A gradient picked for the next shape's stroke and fill (the swatches' Gradient tab) reaches
/// the drag preview and the committed shape exactly, every gradient field kept; a colour picked
/// afterwards replaces it.
#[test]
fn tool_gradient_paints_reach_the_preview_and_the_next_shape() {
    for (mode, depth) in [("rgb", 8), ("rgb", 32), ("gray", 16)] {
        let mut app = app(mode, depth);
        set_tool_paint(&mut app, true, PaintChange::Gradient(test_gradient())).unwrap();
        set_tool_paint(&mut app, false, PaintChange::Gradient(test_gradient())).unwrap();
        assert_eq!(app.ui.tool_options.stroke_width, 3.0, "a gradient enables the stroke");
        assert!(app.ui.tool_options.shape_fill);
        let preview = shape_preview(&app, Tool::Rectangle, [20.0, 20.0], [100.0, 80.0], Modifiers::NONE).unwrap();
        finish_shape(&mut app, Tool::Rectangle, [20.0, 20.0], [100.0, 80.0], Modifiers::NONE);
        let committed = shape(&app).clone();
        assert_eq!(preview.fill, committed.fill);
        assert_eq!(preview.stroke, committed.stroke);
        assert_eq!(committed.stroke.as_ref().unwrap().paint, test_gradient(), "{mode}/{depth}");
        assert_eq!(committed.fill, Some(test_gradient()));
        let doc = &app.session.active().unwrap().doc;
        let rendered = photocraft_vector::render_shape(&preview, doc.pixel_format(), doc.bounds());
        assert_eq!(committed.cache.as_ref().unwrap(), &rendered, "{mode}/{depth}");
        set_tool_paint(&mut app, true, PaintChange::Color([0.0, 1.0, 0.0, 1.0])).unwrap();
        assert!(app.ui.tool_options.shape_stroke_gradient.is_none());
        finish_shape(&mut app, Tool::Rectangle, [20.0, 20.0], [100.0, 80.0], Modifiers::NONE);
        assert_eq!(shape(&app).stroke.as_ref().unwrap().paint, solid_paint([0.0, 1.0, 0.0, 1.0]));
        assert_eq!(shape(&app).fill, Some(test_gradient()), "the fill keeps its gradient");
    }
}

/// Options bar › Stroke › Gradient: the Rectangle tool's next stroke takes the current gradient,
/// a preset's colours and a style, without editing the document.
#[test]
fn stroke_swatch_gradient_tab_sets_the_next_shapes_stroke() {
    let mut h = harness(app("rgb", 8), false);
    h.get_by_label("Stroke color").click();
    h.run_steps(3);
    h.get_by_label("Gradient").click();
    h.run_steps(3);
    let stroke_gradient = |h: &Harness<'_, PhotocraftApp>| match h.state().ui.tool_options.shape_stroke_gradient.clone() {
        Some(photocraft_doc::Fill::Gradient { stops, style, angle, .. }) => (stops, style, angle),
        other => panic!("a stroke gradient, not {other:?}"),
    };
    let (_, style, angle) = stroke_gradient(&h);
    assert_eq!((style, angle), (photocraft_doc::GradientStyle::Linear, 90.0));
    assert_eq!(h.state().ui.tool_options.stroke_width, 3.0, "choosing a gradient enables the stroke");
    h.get_by_label("Blue 01").click();
    h.run_steps(2);
    let presets = h.state().session.presets.gradients.clone();
    let blue = presets.iter().flat_map(|g| &g.items).find(|p| p.name == "Blue 01").unwrap();
    let (fg, bg) = (h.state().session.tools.foreground, h.state().session.tools.background);
    let blue_stops = blue.fill_stops(fg, bg).0;
    assert_eq!(stroke_gradient(&h).0, blue_stops);
    h.get_by_label("Radial Gradient").click();
    h.run_steps(2);
    let (stops, style, _) = stroke_gradient(&h);
    assert_eq!((stops, style), (blue_stops, photocraft_doc::GradientStyle::Radial), "the style keeps the colours");
    assert_eq!(h.state().session.active().unwrap().history.past_len(), 0, "tool defaults do not edit the document");
    h.get_by_label("Solid").click();
    h.run_steps(2);
    assert!(h.state().ui.tool_options.shape_stroke_gradient.is_none(), "Solid goes back to the colour");
}

/// Properties › Stroke › Gradient gives the selected shape a gradient stroke in one undoable
/// step, keeping its width; a style change keeps its colours.
#[test]
fn properties_stroke_swatch_gives_the_selected_shape_a_gradient_stroke() {
    let mut app = app("rgb", 16);
    app.run("shape.create", json!({"rect": [20, 20, 60, 40], "stroke": {"width": 4, "color": [1.0, 0.0, 0.0, 1.0]}})).unwrap();
    let history = app.session.active().unwrap().history.past_len();
    let mut h = harness(app, true);
    h.get_by_label("Set shape stroke type").click();
    h.run_steps(3);
    h.get_by_label("Gradient").click();
    h.run_steps(3);
    let stroke = shape(h.state()).stroke.clone().unwrap();
    let photocraft_doc::Fill::Gradient { stops, .. } = &stroke.paint else { panic!("a gradient stroke, not {:?}", stroke.paint) };
    assert_eq!(stroke.width, 4.0);
    assert_eq!(h.state().session.active().unwrap().history.past_len(), history + 1);
    h.get_by_label("Reflected Gradient").click();
    h.run_steps(3);
    let photocraft_doc::Fill::Gradient { stops: after, style, .. } = shape(h.state()).stroke.clone().unwrap().paint else { panic!("still a gradient stroke") };
    assert_eq!((&after, style), (stops, photocraft_doc::GradientStyle::Reflected));
    while h.state().session.active().unwrap().history.past_len() > history {
        h.state_mut().run("edit.undo", json!({})).unwrap();
    }
    assert_eq!(shape(h.state()).stroke.as_ref().unwrap().paint, solid_paint([1.0, 0.0, 0.0, 1.0]));
}

fn tool_stroke_gradient(h: &Harness<'_, PhotocraftApp>) -> photocraft_doc::Fill {
    h.state().ui.tool_options.shape_stroke_gradient.clone().expect("a stroke gradient")
}

fn gradient_stops(f: &photocraft_doc::Fill) -> photocraft_engine::presets::gradients::FillStops {
    match f {
        photocraft_doc::Fill::Gradient { stops, opacity_stops, .. } => (stops.clone(), opacity_stops.clone()),
        other => panic!("a gradient, not {other:?}"),
    }
}

/// Opens Options bar › Stroke › Gradient and returns the stops strip's rect.
fn open_stroke_gradient(h: &mut Harness<'_, PhotocraftApp>) -> Rect {
    h.get_by_label("Stroke color").click();
    h.run_steps(3);
    if h.state().ui.tool_options.shape_stroke_gradient.is_none() || h.state().ui.tool_options.stroke_width <= 0.0 {
        h.get_by_label("Gradient").click();
        h.run_steps(3);
    }
    h.get_by_label("Gradient stops").rect()
}

fn click(h: &mut Harness<'_, PhotocraftApp>, pos: Pos2) {
    h.hover_at(pos);
    h.step();
    pointer(h, pos, true);
    pointer(h, pos, false);
    h.run_steps(2);
}

/// The Gradient tab's strip edits the stops as Photoshop's popover does: a click below the ramp
/// adds a colour stop, above it an opacity stop; Delete removes the selected one; dragging a stop
/// moves it; the selected colour stop's swatch opens an inline picker that recolours only it.
#[test]
fn gradient_tab_strip_adds_moves_recolours_and_deletes_stops() {
    let mut h = harness(app("rgb", 16), false);
    let strip = open_stroke_gradient(&mut h);
    let (bar_left, bar_right, bar_top, bar_bottom) = (strip.left() + 8.0, strip.right() - 8.0, strip.top() + 16.0, strip.top() + 40.0);
    let x_at = |loc: f32| bar_left + (bar_right - bar_left) * loc;
    let (stops, opacity) = gradient_stops(&tool_stroke_gradient(&h));
    assert_eq!((stops.len(), opacity.len()), (2, 0));
    // A colour stop added in the middle takes the colour the gradient has there.
    click(&mut h, pos2(x_at(0.5), bar_bottom - 4.0));
    let (stops, _) = gradient_stops(&tool_stroke_gradient(&h));
    assert_eq!(stops.len(), 3, "{stops:?}");
    assert!(stops.iter().any(|(l, _)| (l - 0.5).abs() < 0.02), "{stops:?}");
    // It is selected: Delete removes it again.
    h.get_by_label("Delete").click();
    h.run_steps(2);
    assert_eq!(gradient_stops(&tool_stroke_gradient(&h)).0.len(), 2);
    // An opacity stop above the ramp (the two implicit opaque ends become real stops).
    click(&mut h, pos2(x_at(0.25), bar_top + 4.0));
    let (_, opacity) = gradient_stops(&tool_stroke_gradient(&h));
    assert_eq!(opacity.len(), 3, "{opacity:?}");
    // Drag the first colour stop to 30 %.
    let from = pos2(x_at(0.0), bar_bottom + 10.0);
    let to = pos2(x_at(0.3), bar_bottom + 10.0);
    h.hover_at(from);
    h.step();
    pointer(&mut h, from, true);
    for k in 1..=4 {
        h.event(Event::PointerMoved(from + (to - from) * (k as f32 / 4.0)));
        h.step();
    }
    pointer(&mut h, to, false);
    h.run_steps(2);
    let (stops, _) = gradient_stops(&tool_stroke_gradient(&h));
    let mut locations: Vec<f32> = stops.iter().map(|s| s.0).collect();
    locations.sort_by(f32::total_cmp);
    assert!((locations[0] - 0.3).abs() < 0.03 && locations[1] == 1.0, "{locations:?}");
    // The selected stop's colour: its swatch opens the picker; a drag in it recolours that stop.
    let before = gradient_stops(&tool_stroke_gradient(&h)).0;
    let swatch = h.get(egui_kittest::kittest::by().label("Color").role(egui::accesskit::Role::ColorWell)).rect();
    click(&mut h, swatch.center());
    h.run_steps(2);
    // The inline picker opens under the stop's rows (readouts, preview, then the saturation /
    // value field); a click in that field.
    let field = pos2(swatch.left() + 30.0, h.get_by_label("Location").rect().bottom() + 120.0);
    click(&mut h, field);
    let after = gradient_stops(&tool_stroke_gradient(&h)).0;
    assert_eq!(after.len(), before.len());
    let changed: Vec<_> = before.iter().zip(&after).filter(|(a, b)| a != b).collect();
    assert_eq!(changed.len(), 1, "only the selected stop changes colour: {before:?} → {after:?}");
    assert_eq!(h.state().session.active().unwrap().history.past_len(), 0, "tool defaults do not edit the document");
}

/// Align with layer is a checkbox of the tab, and the gradient a popup closes on is offered again
/// in its Recent row.
#[test]
fn gradient_tab_align_with_layer_and_recent_row() {
    let mut h = harness(app("rgb", 8), false);
    open_stroke_gradient(&mut h);
    let align = |h: &Harness<'_, PhotocraftApp>| matches!(tool_stroke_gradient(h), photocraft_doc::Fill::Gradient { align: true, .. });
    assert!(align(&h));
    h.get_by_label("Align with layer").click();
    h.run_steps(2);
    assert!(!align(&h));
    h.get_by_label("Blue 01").click();
    h.run_steps(2);
    let blue = gradient_stops(&tool_stroke_gradient(&h));
    assert!(h.query_by_label("Recent 1").is_none(), "nothing is recent yet");
    h.key_press(Key::Escape);
    h.run_steps(3);
    assert_eq!(h.state().ui.recent_gradients.len(), 1, "closing the popup remembers its gradient");
    open_stroke_gradient(&mut h);
    h.get_by_label("Black, White").click();
    h.run_steps(2);
    assert_ne!(gradient_stops(&tool_stroke_gradient(&h)), blue);
    h.get_by_label("Recent 1").click();
    h.run_steps(2);
    assert_eq!(gradient_stops(&tool_stroke_gradient(&h)), blue, "a Recent pick restores its stops");
    h.key_press(Key::Escape);
    h.run_steps(3);
    assert_eq!(h.state().ui.recent_gradients.len(), 1, "the same stops are remembered once");
}

/// #7176 review: the selected stop is an index; after taking a preset with fewer stops it pointed
/// past them and the Color / Location rows vanished, leaving no way to recolour a stop. They stay,
/// and double-clicking a colour stop opens its colour, as Photoshop's opens the Color Picker.
#[test]
fn gradient_tab_keeps_the_stop_colour_after_a_preset_with_fewer_stops() {
    let mut h = harness_stepped(app("rgb", 8), false, 1.0 / 60.0);
    let strip = open_stroke_gradient(&mut h);
    let (bar_left, bar_right, bar_bottom) = (strip.left() + 8.0, strip.right() - 8.0, strip.top() + 40.0);
    let x_at = |loc: f32| bar_left + (bar_right - bar_left) * loc;
    // Three colour stops, the last one selected.
    click(&mut h, pos2(x_at(0.5), bar_bottom - 4.0));
    click(&mut h, pos2(x_at(1.0), bar_bottom + 10.0));
    assert_eq!(gradient_stops(&tool_stroke_gradient(&h)).0.len(), 3);
    h.get_by_label("Blue 01").click();
    h.run_steps(3);
    assert_eq!(gradient_stops(&tool_stroke_gradient(&h)).0.len(), 2);
    assert!(h.query_by_label("Location").is_some(), "the selected stop's rows stay after a preset with fewer stops");
    let color_well = egui_kittest::kittest::by().label("Color").role(egui::accesskit::Role::ColorWell);
    assert!(h.query(color_well).is_some());
    // Double-click the right colour stop: its picker opens, and a click in it recolours only that stop.
    let stop = pos2(x_at(1.0), bar_bottom + 10.0);
    h.hover_at(stop);
    // A second later, so the two clicks are a double click and not a triple with the preset pick.
    h.run_steps(60);
    for _ in 0..2 {
        pointer(&mut h, stop, true);
        pointer(&mut h, stop, false);
    }
    h.run_steps(2);
    let before = gradient_stops(&tool_stroke_gradient(&h)).0;
    let swatch = h.get(egui_kittest::kittest::by().label("Color").role(egui::accesskit::Role::ColorWell)).rect();
    let field = pos2(swatch.left() + 30.0, h.get_by_label("Location").rect().bottom() + 120.0);
    click(&mut h, field);
    let after = gradient_stops(&tool_stroke_gradient(&h)).0;
    assert_eq!(after[0], before[0], "the left stop keeps its colour");
    assert_ne!(after[1].1, before[1].1, "the double-clicked right stop is recoloured");
    assert_eq!(after[1].0, before[1].0);
}

/// A swatch edit that only changes the gradient's settings (Reverse) keeps a CMYK document's
/// stops as they are: before, every stop went out as sRGB RGBA and came back in the RGB model.
#[test]
fn gradient_tab_edit_keeps_cmyk_stops_in_their_model() {
    let cmyk = |c: [f32; 4]| photocraft_doc::Color { mode: photocraft_doc::ColorMode::Cmyk, c, alpha: 1.0 };
    let stops = vec![(0.0, cmyk([0.1, 0.2, 0.3, 0.9])), (1.0, cmyk([0.75, 0.0, 0.5, 0.0]))];
    for (properties, label) in [(true, "Set shape stroke type"), (false, "Stroke color")] {
        let mut app = app("cmyk", 8);
        let stroke = json!({"width": 4, "color": {"gradient": {"stops": [[0.0, stops[0].1], [1.0, stops[1].1]]}}});
        app.run("shape.create", json!({"rect": [20, 20, 60, 40], "stroke": stroke})).unwrap();
        assert_eq!(gradient_stops(&shape(&app).stroke.as_ref().unwrap().paint).0, stops);
        let mut h = harness(app, properties);
        h.get_by_label(label).click();
        h.run_steps(3);
        h.get_by_label("Reverse").click();
        h.run_steps(3);
        let paint = shape(h.state()).stroke.clone().unwrap().paint;
        assert!(matches!(paint, photocraft_doc::Fill::Gradient { reverse: true, .. }), "Properties={properties}: Reverse applied");
        assert_eq!(gradient_stops(&paint).0, stops, "Properties={properties}: the stops keep their CMYK model and values");
    }
}

/// Options bar with a strokeless shape selected and the stroke swatch's gradient chosen: giving
/// the shape a stroke through the width field or the stroke type takes that gradient, as the next
/// shape does, not a solid background colour.
#[test]
fn width_and_stroke_type_give_a_strokeless_shape_the_tool_stroke_gradient() {
    for via_width in [true, false] {
        let mut app = app("rgb", 8);
        app.run("shape.create", json!({"rect": [20, 20, 80, 60], "fill": "#336699", "stroke": null})).unwrap();
        set_tool_paint(&mut app, true, PaintChange::Gradient(test_gradient())).unwrap();
        let mut h = harness(app, false);
        if via_width {
            stroke_width(&mut h, "5");
        } else {
            h.get_by_label("Stroke Options").click();
            h.run_steps(3);
            h.get_by_role_and_label(egui::accesskit::Role::RadioButton, "Dotted").click();
            h.run_steps(3);
        }
        let stroke = shape(h.state()).stroke.clone().expect("a stroke");
        assert_eq!(stroke.paint, test_gradient(), "via_width={via_width}");
        assert_eq!(stroke.width, if via_width { 5.0 } else { 3.0 });
    }
}

/// An arrow-key step at 0 px on a strokeless shape must not add an invisible stroke.
#[test]
fn arrow_down_at_zero_width_keeps_a_strokeless_shape_strokeless() {
    for properties in [true, false] {
        let mut app = app("rgb", 8);
        app.run("shape.create", json!({"rect": [20, 20, 80, 60], "fill": "#336699", "stroke": null})).unwrap();
        assert!(shape(&app).stroke.is_none());
        let history = app.session.active().unwrap().history.past_len();
        let mut h = harness(app, properties);
        h.get_all_by_role(egui::accesskit::Role::SpinButton).next().expect("the stroke width").click();
        h.run();
        h.key_press(Key::ArrowDown);
        h.run_steps(3);
        assert!(shape(h.state()).stroke.is_none(), "Properties={properties}: Down at 0 px added a stroke");
        assert_eq!(h.state().session.active().unwrap().history.past_len(), history, "Properties={properties}: no history step");
        // The field has the keys: Up still enables a 1 px stroke.
        h.key_press(Key::ArrowUp);
        h.run_steps(3);
        assert_eq!(shape(h.state()).stroke.as_ref().map(|s| s.width), Some(1.0), "Properties={properties}");
    }
}
