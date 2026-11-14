//! The Pencil tool (#213): aliased strokes through the live-stroke path, Auto Erase, the square
//! pixel-grid cursor, ⇧-click lines, Control+Alt resizing and the B group.

use egui::{Key, Modifiers, Pos2, Rect, vec2};
use egui_kittest::Harness;
use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::{ToolEvent, ViewXform, pencil_cursor_rect, pencil_tip_is_round, tool_event};
use crate::state::Tool;

fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 120, "height": 80, "background": "transparent"})).unwrap();
    app.run("tools.setColors", json!({"foreground": "#000000", "background": "#ffffff"})).unwrap();
    // Each tool keeps its own brush (#218): switch first, then set the Pencil's options, as the
    // options bar does.
    app.ui.tool = Tool::Pencil;
    crate::paint_mouse::sync_tool_brush(&mut app);
    app.run("tools.setBrush", json!({"brush": {"size": 3, "hardness": 0.0, "smoothing": {"amount": 0.0}}})).unwrap();
    app
}

fn layer(app: &PhotocraftApp) -> photocraft_engine::doc::Layer {
    let st = app.session.active().unwrap();
    st.doc.layer(st.active_layer.unwrap()).unwrap().clone()
}

fn rgba(app: &PhotocraftApp, x: i32, y: i32) -> [f32; 4] {
    layer(app).surface().unwrap().rgba(x, y)
}

fn drag(app: &mut PhotocraftApp, pts: &[(f64, f64)], mods: Modifiers) {
    let (x, y) = pts[0];
    tool_event(app, ToolEvent::Down { x, y, pressure: 1.0 }, mods);
    for &(x, y) in &pts[1..] {
        tool_event(app, ToolEvent::Move { x, y, pressure: 1.0 }, mods);
    }
    let &(x, y) = pts.last().unwrap();
    tool_event(app, ToolEvent::Up { x, y }, mods);
}

fn last(app: &PhotocraftApp) -> (String, serde_json::Value) {
    app.session.journal.last().cloned().unwrap()
}

#[test]
fn pencil_strokes_are_aliased_live_and_one_undo_step() {
    let mut app = app();
    let pts = [(10.0, 10.0), (40.3, 22.7), (70.0, 50.0), (100.0, 30.0)];
    let (x, y) = pts[0];
    tool_event(&mut app, ToolEvent::Down { x, y, pressure: 1.0 }, Modifiers::NONE);
    for &(x, y) in &pts[1..] {
        tool_event(&mut app, ToolEvent::Move { x, y, pressure: 1.0 }, Modifiers::NONE);
    }
    // The canvas shows the engine's real dabs while dragging.
    assert!(app.live_stroke.is_some(), "the Pencil strokes live");
    tool_event(&mut app, ToolEvent::Up { x: 100.0, y: 30.0 }, Modifiers::NONE);
    let (id, p) = last(&app);
    assert_eq!(id, "paint.pencil");
    assert_eq!(p["autoErase"], json!(false));
    assert_eq!(app.session.active().unwrap().history.past_len(), 1);
    // 8 bits, a soft session brush: still no partial alpha anywhere.
    let s = layer(&app).surface().unwrap().clone();
    let mut painted = 0;
    for y in 0..80 {
        for x in 0..120 {
            let a = s.rgba(x, y)[3];
            assert!(a == 0.0 || a == 1.0, "partial alpha {a} at ({x},{y})");
            painted += usize::from(a == 1.0);
        }
    }
    assert!(painted > 200, "{painted}");
    assert_eq!(rgba(&app, 70, 50), [0.0, 0.0, 0.0, 1.0]);
}

#[test]
fn auto_erase_paints_the_background_colour_from_foreground_pixels() {
    let mut app = app();
    drag(&mut app, &[(10.0, 40.0), (110.0, 40.0)], Modifiers::NONE);
    app.ui.tool_options.pencil_auto_erase = true;
    // Starting on the black line: white.
    drag(&mut app, &[(50.0, 40.0), (50.0, 70.0)], Modifiers::NONE);
    assert_eq!(rgba(&app, 50, 60), [1.0; 4]);
    assert_eq!(last(&app).1["autoErase"], json!(true));
    // Starting off it: black.
    drag(&mut app, &[(80.0, 10.0), (80.0, 30.0)], Modifiers::NONE);
    assert_eq!(rgba(&app, 80, 20), [0.0, 0.0, 0.0, 1.0]);
}

#[test]
fn shift_click_draws_a_pencil_line() {
    let mut app = app();
    drag(&mut app, &[(10.0, 10.0), (20.0, 10.0)], Modifiers::NONE);
    drag(&mut app, &[(100.0, 60.0)], Modifiers::SHIFT);
    let (id, p) = last(&app);
    assert_eq!(id, "paint.pencil");
    assert_eq!(p["points"].as_array().unwrap().len(), 2, "{p}");
    assert_eq!(rgba(&app, 60, 35)[3], 1.0);
    assert_eq!(app.session.active().unwrap().history.past_len(), 2);
}

#[test]
fn shift_clicked_one_pixel_line_uses_the_clicked_pixels_not_subpixel_positions() {
    let fractions = [("TL", (0.05, 0.05)), ("TR", (0.95, 0.05)), ("BL", (0.05, 0.95)), ("BR", (0.95, 0.95)), ("C", (0.5, 0.5))];
    let starts = [&fractions[0], &fractions[1], &fractions[2], &fractions[3]];

    for (start_name, start_fraction) in starts {
        for (end_name, end_fraction) in fractions {
            let paint = |raw: bool| {
                let mut app = app();
                app.run("tools.setBrush", json!({"brush": {"size": 1.0, "hardness": 1.0}})).unwrap();
                let at = |origin: f64, fraction: f64| origin + if raw { fraction } else { 0.5 };
                let start = (at(10.0, start_fraction.0), at(10.0, start_fraction.1));
                let end = (at(60.0, end_fraction.0), at(45.0, end_fraction.1));
                for (point, mods) in [(start, Modifiers::NONE), (end, Modifiers::SHIFT)] {
                    tool_event(&mut app, ToolEvent::Down { x: point.0, y: point.1, pressure: 1.0 }, mods);
                    tool_event(&mut app, ToolEvent::Up { x: point.0, y: point.1 }, mods);
                }
                (0..80).flat_map(|y| (0..120).map(move |x| (x, y))).filter(|&(x, y)| rgba(&app, x, y)[3] > 0.0).collect::<Vec<_>>()
            };

            assert_eq!(paint(true), paint(false), "{start_name}->{end_name}");
        }
    }
}

#[test]
fn shift_clicked_pencil_does_not_split_a_click_near_a_pixel_boundary() {
    let mut app = app();
    app.run("tools.setBrush", json!({"brush": {"size": 1.0, "hardness": 1.0}})).unwrap();
    let click = |app: &mut PhotocraftApp, x: f64, y: f64, modifiers| {
        tool_event(app, ToolEvent::Down { x, y, pressure: 1.0 }, modifiers);
        tool_event(app, ToolEvent::Up { x, y }, modifiers);
    };

    click(&mut app, 30.5, 30.5, Modifiers::NONE);
    // The pointer is just below the grid corner; numerical rounding must not make the Pencil
    // snap to one pixel and the line walker rasterize the neighbouring one.
    let just_below_corner = 30.0 - 5e-8;
    click(&mut app, just_below_corner, just_below_corner, Modifiers::SHIFT);

    assert_eq!(rgba(&app, 29, 29)[3], 0.0, "the click should remain in the pixel chosen by the line walker");
    assert_eq!(rgba(&app, 30, 30)[3], 1.0);
}

#[test]
fn ctrl_alt_drag_resizes_the_pencil_without_painting() {
    let mut app = app();
    let ctrl_alt = Modifiers { alt: true, ctrl: true, ..Default::default() };
    assert!(crate::brush_resize::applies(Tool::Pencil));
    drag(&mut app, &[(50.0, 40.0), (60.0, 40.0)], ctrl_alt);
    assert_eq!(app.session.tools.brush.size, 23.0);
    assert_eq!(app.session.active().unwrap().history.past_len(), 0);
}

#[test]
fn square_cursor_sits_on_the_pixel_grid() {
    let rect = Rect::from_min_size(Pos2::new(0.0, 0.0), vec2(800.0, 600.0));
    let xf = ViewXform { rect, zoom: 8.0, center: [50.0, 37.5], flip: false, rotation: 0.0, aspect: 1.0 };
    // 1 px at 800 %: the 8-point square of the pixel under the pointer.
    let r = pencil_cursor_rect(&xf, [10.3, 5.7], 1.0, 1.0);
    assert_eq!(r, Rect::from_two_pos(xf.to_screen(10.0, 5.0), xf.to_screen(11.0, 6.0)));
    assert_eq!(r.size(), vec2(8.0, 8.0));
    // Even sizes centre on the nearest pixel corner; odd ones on the pixel.
    let r = pencil_cursor_rect(&xf, [10.3, 5.7], 4.0, 1.0);
    assert_eq!(r, Rect::from_two_pos(xf.to_screen(8.0, 4.0), xf.to_screen(12.0, 8.0)));
    let r = pencil_cursor_rect(&xf, [10.9, 5.1], 3.0, 1.0);
    assert_eq!(r, Rect::from_two_pos(xf.to_screen(9.0, 4.0), xf.to_screen(12.0, 7.0)));
    // At 2× and at an odd zoom the edges land on physical pixels.
    for (zoom, ppp) in [(3.3, 2.0), (0.5, 2.0), (1.0, 1.0), (13.7, 1.5)] {
        let xf = ViewXform {
            rect: Rect::from_min_size(Pos2::new(0.3, 0.7), vec2(800.0, 600.0)),
            zoom,
            center: [50.2, 37.9],
            flip: true,
            rotation: 0.0,
            aspect: 1.0,
        };
        let r = pencil_cursor_rect(&xf, [20.4, 30.6], 5.0, ppp);
        for v in [r.min.x, r.min.y, r.max.x, r.max.y] {
            assert!((v * ppp - (v * ppp).round()).abs() < 1e-3, "zoom {zoom} ppp {ppp}: {v}");
        }
        assert!((r.width() - 5.0 * zoom).abs() <= 1.0 / ppp + 1e-3, "{r:?}");
    }
    // Hostile numbers don't panic.
    let _ = pencil_cursor_rect(&xf, [f64::NAN, f64::INFINITY], f32::NAN, f32::NAN);
}

#[test]
fn the_cursor_is_round_exactly_when_the_pencil_dab_is() {
    // #2662: one dab per size; the cursor is the square only while the dab fills its corners.
    let mut app = app();
    for n in 1..=12u32 {
        let (x, y) = (8.0 + f64::from((n - 1) % 6) * 18.0 + 0.3, 15.0 + f64::from((n - 1) / 6) * 30.0 + 0.6);
        app.run("paint.pencil", json!({"points": [[x, y]], "size": n})).unwrap();
        let [x0, y0, ..] = photocraft_engine::paint::grid_square(x, y, n as f32);
        let corner = rgba(&app, x0 as i32, y0 as i32)[3] == 1.0;
        assert_eq!(pencil_tip_is_round(n as f32), !corner, "{n} px");
    }
    assert!(pencil_tip_is_round(9.0) && !pencil_tip_is_round(3.0));
    assert!(!pencil_tip_is_round(f32::NAN));
}

#[test]
fn the_eraser_in_pencil_mode_erases_aliased_pixels() {
    // #2662: an opaque layer, then a soft Eraser in Pencil mode erases with hard edges.
    let mut app = app();
    app.run("paint.pencil", json!({"points": [[60.0, 40.0]], "size": 400})).unwrap();
    app.ui.tool = Tool::Eraser;
    crate::paint_mouse::sync_tool_brush(&mut app);
    app.run("tools.setBrush", json!({"brush": {"size": 15, "hardness": 0.0, "smoothing": {"amount": 0.0}}})).unwrap();
    app.ui.tool_options.eraser_mode = "pencil".into();
    drag(&mut app, &[(10.0, 40.0), (60.3, 25.7), (110.0, 40.0)], Modifiers::NONE);
    let (id, p) = last(&app);
    assert_eq!((id.as_str(), &p["erase"]), ("paint.pencil", &json!(true)));
    let s = layer(&app).surface().unwrap().clone();
    let mut erased = 0;
    for y in 0..80 {
        for x in 0..120 {
            let a = s.rgba(x, y)[3];
            assert!(a == 0.0 || a == 1.0, "partial alpha {a} at ({x},{y})");
            erased += usize::from(a == 0.0);
        }
    }
    assert!(erased > 500, "{erased}");
}

#[test]
fn the_eraser_in_block_mode_erases_a_16_screen_px_square() {
    // #2770: a soft, small, half-opacity Eraser in Block mode still clears a hard square of
    // 16 screen pixels: 16 document pixels at 100 %, 8 at 200 %.
    for (zoom, n) in [(1.0, 16usize), (2.0, 8)] {
        let mut app = app();
        app.run("paint.pencil", json!({"points": [[60.0, 40.0]], "size": 400})).unwrap();
        app.ui.tool = Tool::Eraser;
        crate::paint_mouse::sync_tool_brush(&mut app);
        app.run("tools.setBrush", json!({"brush": {"size": 5, "hardness": 0.0, "opacity": 0.5, "flow": 0.3, "smoothing": {"amount": 0.0}}})).unwrap();
        app.ui.tool_options.eraser_mode = "block".into();
        app.ui.views[0].zoom = zoom;
        drag(&mut app, &[(60.3, 40.6)], Modifiers::NONE);
        let (id, p) = last(&app);
        assert_eq!((id.as_str(), &p["block"]), ("paint.pencil", &json!(true)));
        let s = layer(&app).surface().unwrap().clone();
        let (mut erased, mut x0, mut y0, mut x1, mut y1) = (0, i32::MAX, i32::MAX, 0, 0);
        for y in 0..80 {
            for x in 0..120 {
                let a = s.rgba(x, y)[3];
                assert!(a == 0.0 || a == 1.0, "partial alpha {a} at ({x},{y}), zoom {zoom}");
                if a == 0.0 {
                    erased += 1;
                    (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x), y1.max(y));
                }
            }
        }
        assert_eq!((erased, x1 - x0 + 1, y1 - y0 + 1), (n * n, n as i32, n as i32), "zoom {zoom}");
    }
}

#[test]
fn b_cycles_brush_pencil_color_replacement_and_mixer_brush() {
    let mut h = Harness::builder().with_size(vec2(1280.0, 800.0)).with_max_steps(64).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 200, "height": 120})).unwrap();
        app
    });
    h.state_mut().ui.tool = Tool::Move;
    h.state_mut().session.edit_prefs(|p| p.tools.use_shift_key_for_tool_switch = false);
    h.run_steps(4);
    let press = |h: &mut Harness<'_, PhotocraftApp>, m: Modifiers| {
        h.event(egui::Event::Key { key: Key::B, physical_key: None, pressed: true, repeat: false, modifiers: m });
        h.event(egui::Event::Key { key: Key::B, physical_key: None, pressed: false, repeat: false, modifiers: m });
        h.run_steps(2);
    };
    press(&mut h, Modifiers::NONE);
    assert_eq!(h.state().ui.tool, Tool::Brush);
    press(&mut h, Modifiers::NONE);
    assert_eq!(h.state().ui.tool, Tool::Pencil);
    press(&mut h, Modifiers::NONE);
    assert_eq!(h.state().ui.tool, Tool::ColorReplacement);
    press(&mut h, Modifiers::NONE);
    assert_eq!(h.state().ui.tool, Tool::MixerBrush);
    // Use Shift Key for Tool Switch: ⇧B cycles, B keeps the group's tool.
    h.state_mut().session.edit_prefs(|p| p.tools.use_shift_key_for_tool_switch = true);
    press(&mut h, Modifiers::SHIFT);
    assert_eq!(h.state().ui.tool, Tool::Brush);
    press(&mut h, Modifiers::SHIFT);
    assert_eq!(h.state().ui.tool, Tool::Pencil);
    press(&mut h, Modifiers::SHIFT);
    assert_eq!(h.state().ui.tool, Tool::ColorReplacement);
    press(&mut h, Modifiers::SHIFT);
    assert_eq!(h.state().ui.tool, Tool::MixerBrush);
    press(&mut h, Modifiers::NONE);
    assert_eq!(h.state().ui.tool, Tool::MixerBrush);
    assert_eq!(Tool::from_name("pencil"), Some(Tool::Pencil));
    assert_eq!(Tool::from_name("mixerBrush"), Some(Tool::MixerBrush));
}

/// Erase to History paints the source state in each Eraser mode, even on a layer made since.
#[test]
fn the_eraser_with_erase_to_history_paints_the_source_state_in_every_mode_even_on_a_layer_made_since() {
    for mode in ["brush", "pencil", "block"] {
        let mut app = app();
        app.run("paint.pencil", json!({"points": [[60.0, 40.0]], "size": 400})).unwrap();
        let before = layer(&app).surface().unwrap().rgba(60, 40);
        assert!(before[3] > 0.99, "{before:?}");
        // State 1 = after the stroke; the layer made next did not exist then.
        app.run("history.setSource", json!({"state": 1})).unwrap();
        app.run("layer.new.layer", json!({})).unwrap();
        assert!(layer(&app).surface().unwrap().rgba(60, 40)[3] < 0.01);
        app.ui.tool = Tool::Eraser;
        crate::paint_mouse::sync_tool_brush(&mut app);
        app.run("tools.setBrush", json!({"brush": {"size": 15, "hardness": 1.0, "smoothing": {"amount": 0.0}}})).unwrap();
        app.ui.tool_options.eraser_mode = mode.into();
        // Without the option the Eraser erases: the empty layer stays empty.
        drag(&mut app, &[(55.0, 40.0), (65.0, 40.0)], Modifiers::NONE);
        assert!(layer(&app).surface().unwrap().rgba(60, 40)[3] < 0.01, "{mode}");
        app.ui.tool_options.erase_to_history = true;
        drag(&mut app, &[(55.0, 40.0), (65.0, 40.0)], Modifiers::NONE);
        let (id, p) = last(&app);
        assert_eq!((id.as_str(), &p["eraseToHistory"]), ("paint.historyBrush", &json!(true)), "{mode}");
        assert_eq!((p["pencil"] == json!(true), p["block"] == json!(true)), (mode == "pencil", mode == "block"), "{mode}");
        let s = layer(&app).surface().unwrap().clone();
        let after = s.rgba(60, 40);
        assert!(after[3] > 0.99 && (0..3).all(|i| (after[i] - before[i]).abs() < 0.01), "{mode}: {after:?} vs {before:?}");
        // The hard tips leave only whole pixels: painted back in full or untouched.
        if mode != "brush" {
            for y in 0..80 {
                for x in 0..120 {
                    let a = s.rgba(x, y)[3];
                    assert!(a == 0.0 || a == 1.0, "{mode}: partial alpha {a} at ({x},{y})");
                }
            }
        }
    }
}

#[test]
fn erase_to_history_in_pencil_mode_ignores_the_brush_modes_flow() {
    let mut app = app();
    app.ui.tool = Tool::Eraser;
    crate::paint_mouse::sync_tool_brush(&mut app);
    app.run("tools.setBrush", json!({"brush": {"flow": 0.5}})).unwrap();
    app.ui.tool_options.erase_to_history = true;
    app.ui.tool_options.eraser_mode = "brush".into();
    assert_eq!(crate::retouch_ui::history_brush_params(&app)["flow"], json!(50.0));
    app.ui.tool_options.eraser_mode = "pencil".into();
    assert_eq!(crate::retouch_ui::history_brush_params(&app)["flow"], json!(100.0));
}

/// ⌥ at the press reverses Erase to History for that stroke, and the option itself stays as it was.
#[test]
fn alt_at_the_press_reverses_erase_to_history_for_one_eraser_stroke() {
    for (mode, pencil) in [("brush", false), ("pencil", true)] {
        let mut app = app();
        app.run("paint.pencil", json!({"points": [[60.0, 40.0]], "size": 400})).unwrap();
        let painted = layer(&app).surface().unwrap().rgba(60, 40);
        app.run("history.setSource", json!({"state": 1})).unwrap();
        app.ui.tool = Tool::Eraser;
        crate::paint_mouse::sync_tool_brush(&mut app);
        app.run("tools.setBrush", json!({"brush": {"size": 15, "hardness": 1.0, "smoothing": {"amount": 0.0}}})).unwrap();
        app.ui.tool_options.eraser_mode = mode.into();
        let stroke = |app: &mut PhotocraftApp, down: Modifiers, later: Modifiers| {
            tool_event(app, ToolEvent::Down { x: 55.0, y: 40.0, pressure: 1.0 }, down);
            tool_event(app, ToolEvent::Move { x: 65.0, y: 40.0, pressure: 1.0 }, later);
            tool_event(app, ToolEvent::Up { x: 65.0, y: 40.0 }, later);
        };
        // A plain stroke erases, and ⌥ first pressed during it changes nothing.
        stroke(&mut app, Modifiers::NONE, Modifiers::ALT);
        assert!(layer(&app).surface().unwrap().rgba(60, 40)[3] < 0.01, "{mode}");
        // ⌥ at the press paints the source back, and letting go of it during the stroke doesn't stop it.
        stroke(&mut app, Modifiers::ALT, Modifiers::NONE);
        let (id, p) = last(&app);
        assert_eq!((id.as_str(), &p["eraseToHistory"]), ("paint.historyBrush", &json!(true)), "{mode}: {id} {p}");
        assert_eq!(p["pencil"] == json!(true), pencil, "{mode}: {p}");
        let after = layer(&app).surface().unwrap().rgba(60, 40);
        assert!(after[3] > 0.99 && (0..3).all(|i| (after[i] - painted[i]).abs() < 0.01), "{mode}: {after:?} vs {painted:?}");
        // The option itself stays off, so the next plain stroke erases again.
        assert!(!app.ui.tool_options.erase_to_history, "{mode}");
        stroke(&mut app, Modifiers::NONE, Modifiers::NONE);
        assert!(layer(&app).surface().unwrap().rgba(60, 40)[3] < 0.01, "{mode}");
        // With the option on, a plain stroke paints the source back and ⌥ at the press erases.
        app.ui.tool_options.erase_to_history = true;
        stroke(&mut app, Modifiers::NONE, Modifiers::NONE);
        assert!(layer(&app).surface().unwrap().rgba(60, 40)[3] > 0.99, "{mode}");
        stroke(&mut app, Modifiers::ALT, Modifiers::NONE);
        assert!(layer(&app).surface().unwrap().rgba(60, 40)[3] < 0.01, "{mode}");
        assert!(app.ui.tool_options.erase_to_history, "{mode}");
    }
}

/// A ⇧-click Erase to History line isn't smoothed (#11887) and keeps the Eraser's brush from the
/// press: the commit sends `smoothing: 0` beside the `brush` param, as its live preview does, so
/// the release takes the pixels the preview drew.
#[test]
fn a_shift_click_erase_to_history_line_commits_its_live_pixels_unsmoothed() {
    for mode in ["brush", "pencil", "block"] {
        let mut app = app();
        app.run("paint.pencil", json!({"points": [[60.0, 40.0]], "size": 400})).unwrap();
        app.run("history.setSource", json!({"state": 1})).unwrap();
        app.run("layer.new.layer", json!({})).unwrap();
        app.ui.tool = Tool::Eraser;
        crate::paint_mouse::sync_tool_brush(&mut app);
        app.run("tools.setBrush", json!({"brush": {"size": 9, "hardness": 1.0, "smoothing": {"amount": 0.6, "catchUpOnEnd": false}}})).unwrap();
        app.ui.tool_options.eraser_mode = mode.into();
        app.ui.tool_options.erase_to_history = true;
        drag(&mut app, &[(20.0, 20.0), (40.0, 30.0)], Modifiers::NONE);
        tool_event(&mut app, ToolEvent::Down { x: 100.0, y: 60.0, pressure: 1.0 }, Modifiers::SHIFT);
        assert!(app.live_stroke.is_some(), "{mode}: a live line");
        app.retouch_from_live = false;
        tool_event(&mut app, ToolEvent::Up { x: 100.0, y: 60.0 }, Modifiers::SHIFT);
        let (id, p) = last(&app);
        assert_eq!((id.as_str(), &p["eraseToHistory"]), ("paint.historyBrush", &json!(true)), "{mode}: {id} {p}");
        let ends: Vec<[f64; 2]> = p["points"].as_array().unwrap().iter().map(|q| [q[0].as_f64().unwrap(), q[1].as_f64().unwrap()]).collect();
        // The Pencil tip's line runs between the clicked pixels' centres.
        let c = if mode == "pencil" { 0.5 } else { 0.0 };
        assert_eq!(ends, vec![[40.0 + c, 30.0 + c], [100.0 + c, 60.0 + c]], "{mode}");
        assert_eq!(p["smoothing"], json!(0.0), "{mode}: {p}");
        assert!(p["brush"].is_object(), "{mode}: the brush from the press: {p}");
        assert!(app.retouch_from_live, "{mode}: the release took the live pixels instead of rendering the line again");
        // The line reaches the click: the source is painted back at its far end.
        assert!(rgba(&app, 99, 60)[3] > 0.99, "{mode}");
        assert_eq!(app.session.tools.brush.smoothing.amount, 0.6, "{mode}: the Eraser keeps its Smoothing");
    }
}

/// A tool key pressed mid-stroke changes neither the stroke's options nor its tip.
#[test]
fn a_tool_key_during_an_erase_to_history_stroke_keeps_the_stroke_as_it_began() {
    for (mode, pencil) in [("brush", false), ("pencil", true)] {
        let mut app = app();
        app.run("paint.pencil", json!({"points": [[60.0, 40.0]], "size": 400})).unwrap();
        let before = layer(&app).surface().unwrap().rgba(60, 40);
        app.run("history.setSource", json!({"state": 1})).unwrap();
        app.run("layer.new.layer", json!({})).unwrap();
        app.ui.tool = Tool::Eraser;
        crate::paint_mouse::sync_tool_brush(&mut app);
        app.run("tools.setBrush", json!({"brush": {"size": 15, "hardness": 1.0, "smoothing": {"amount": 0.0}}})).unwrap();
        app.ui.tool_options.eraser_mode = mode.into();
        app.ui.tool_options.erase_to_history = true;
        tool_event(&mut app, ToolEvent::Down { x: 55.0, y: 40.0, pressure: 1.0 }, Modifiers::NONE);
        tool_event(&mut app, ToolEvent::Move { x: 60.0, y: 40.0, pressure: 1.0 }, Modifiers::NONE);
        // What pressing a tool's letter does (`shortcuts`): the drag is not cancelled.
        app.ui.tool = Tool::Brush;
        crate::paint_mouse::sync_tool_brush(&mut app);
        app.run("tools.setBrush", json!({"brush": {"size": 90}})).unwrap();
        tool_event(&mut app, ToolEvent::Move { x: 65.0, y: 40.0, pressure: 1.0 }, Modifiers::NONE);
        tool_event(&mut app, ToolEvent::Up { x: 65.0, y: 40.0 }, Modifiers::NONE);
        let (id, p) = last(&app);
        assert_eq!((id.as_str(), &p["eraseToHistory"]), ("paint.historyBrush", &json!(true)), "{mode}: {id} {p}");
        assert_eq!(p["pencil"] == json!(true), pencil, "{mode}: {p}");
        assert_eq!(p["size"], json!(15.0), "{mode}: the Eraser's size, not the new tool's");
        let after = layer(&app).surface().unwrap().rgba(60, 40);
        assert!(after[3] > 0.99 && (0..3).all(|i| (after[i] - before[i]).abs() < 0.01), "{mode}: {after:?} vs {before:?}");
    }
}

#[test]
fn a_tool_key_during_a_history_brush_stroke_keeps_the_brush_it_began_with() {
    let mut app = app();
    app.run("paint.pencil", json!({"points": [[60.0, 40.0]], "size": 400})).unwrap();
    app.run("history.setSource", json!({"state": 1})).unwrap();
    app.ui.tool = Tool::HistoryBrush;
    crate::paint_mouse::sync_tool_brush(&mut app);
    app.run("tools.setBrush", json!({"brush": {"size": 15, "hardness": 0.5, "spacing": 0.3, "smoothing": {"amount": 0.0}}})).unwrap();
    tool_event(&mut app, ToolEvent::Down { x: 55.0, y: 40.0, pressure: 1.0 }, Modifiers::NONE);
    tool_event(&mut app, ToolEvent::Move { x: 60.0, y: 40.0, pressure: 1.0 }, Modifiers::NONE);
    // What pressing a tool's letter does (`shortcuts`): the drag is not cancelled.
    app.ui.tool = Tool::Brush;
    crate::paint_mouse::sync_tool_brush(&mut app);
    app.run("tools.setBrush", json!({"brush": {"size": 90, "hardness": 1.0, "spacing": 0.1}})).unwrap();
    tool_event(&mut app, ToolEvent::Move { x: 65.0, y: 40.0, pressure: 1.0 }, Modifiers::NONE);
    tool_event(&mut app, ToolEvent::Up { x: 65.0, y: 40.0 }, Modifiers::NONE);
    let (id, p) = last(&app);
    assert_eq!(id, "paint.historyBrush", "{p}");
    assert_eq!(p["size"], json!(15.0), "{p}");
    assert_eq!(p["hardness"].as_f64().map(f64::round), Some(50.0), "{p}");
    assert_eq!(p["spacing"].as_f64().map(f64::round), Some(30.0), "{p}");
    assert_eq!(p.get("eraseToHistory"), None, "{p}");
}

/// The layer after a History Brush (`mode` "") or Erase to History (`mode` "brush", "pencil",
/// "block") stroke whose tool brush is given `pressed`; with `switched`, the Brush tool is picked
/// halfway and its saved brush given `switched`, as a tool key does mid-stroke (the drag goes on).
fn history_stroke_pixels(mode: &str, pressed: &serde_json::Value, switched: Option<&serde_json::Value>) -> Vec<f32> {
    let mut app = app();
    app.run("paint.pencil", json!({"points": [[60.0, 40.0]], "size": 400})).unwrap();
    app.run("history.setSource", json!({"state": 1})).unwrap();
    let tool = if mode.is_empty() { Tool::HistoryBrush } else { Tool::Eraser };
    if tool == Tool::Eraser {
        // A layer made since the source state: Erase to History paints that state's one layer.
        app.run("layer.new.layer", json!({})).unwrap();
    } else {
        app.run("paint.pencil", json!({"points": [[60.0, 40.0]], "size": 400, "erase": true})).unwrap();
    }
    assert!(layer(&app).surface().unwrap().rgba(60, 40)[3] < 0.01, "{mode}: nothing there to start with");
    app.ui.tool = tool;
    crate::paint_mouse::sync_tool_brush(&mut app);
    app.run("tools.setBrush", json!({"brush": {"size": 15, "hardness": 1.0, "spacing": 0.1, "smoothing": {"amount": 0.0}}})).unwrap();
    app.run("tools.setBrush", json!({ "brush": pressed })).unwrap();
    if tool == Tool::Eraser {
        app.ui.tool_options.eraser_mode = mode.into();
        app.ui.tool_options.erase_to_history = true;
    }
    let pts = [(30.0, 40.0), (45.0, 44.0), (60.0, 37.0), (75.0, 43.0), (90.0, 40.0)];
    tool_event(&mut app, ToolEvent::Down { x: pts[0].0, y: pts[0].1, pressure: 1.0 }, Modifiers::NONE);
    for (i, &(x, y)) in pts.iter().enumerate().skip(1) {
        if i == 3
            && let Some(sw) = switched
        {
            // What pressing a tool's letter does (`shortcuts`): the drag is not cancelled.
            app.ui.tool = Tool::Brush;
            crate::paint_mouse::sync_tool_brush(&mut app);
            app.run("tools.setBrush", json!({ "brush": sw })).unwrap();
        }
        tool_event(&mut app, ToolEvent::Move { x, y, pressure: 1.0 }, Modifiers::NONE);
    }
    tool_event(&mut app, ToolEvent::Up { x: pts[4].0, y: pts[4].1 }, Modifiers::NONE);
    let (id, p) = last(&app);
    assert_eq!(id, "paint.historyBrush", "{mode}: {p}");
    let st = app.session.active().unwrap();
    layer(&app).surface().unwrap().read_region(st.doc.bounds())
}

/// A tool key pressed mid-stroke leaves a History Brush or Erase to History stroke exactly as the
/// same stroke without it, with the brush it was pressed with: tip shape, sampled tip, dynamics
/// and smoothing, not only size, hardness and spacing (FredApps' review of #10692).
#[test]
fn a_tool_key_mid_stroke_paints_what_the_stroke_paints_without_it() {
    let round = json!({"tip": "round", "roundness": 1.0, "angle": 0.0});
    let cross = json!({"sampled": {"width": 3, "height": 3, "data": [0.0, 1.0, 0.0, 1.0, 1.0, 1.0, 0.0, 1.0, 0.0]}});
    let cases = [
        ("an elliptical tip, then a round one", json!({"roundness": 0.2, "angle": 30.0}), round.clone()),
        ("a round tip, then a sampled one", round.clone(), json!({"tip": cross})),
        ("a sampled tip, then a round one", json!({"tip": cross}), round.clone()),
        ("no dynamics, then size jitter", round.clone(), json!({"shapeDynamics": {"enabled": true, "size": {"jitter": 1.0}}})),
        ("no smoothing, then 90 %", round.clone(), json!({"smoothing": {"enabled": true, "amount": 0.9}})),
    ];
    for mode in ["", "brush", "pencil", "block"] {
        for (what, pressed, switched) in &cases {
            let plain = history_stroke_pixels(mode, pressed, None);
            assert!(history_stroke_pixels(mode, pressed, Some(switched)) == plain, "{mode} {what}: the switch changed the stroke");
            // The switched-to brush paints differently, so the comparison means something; Block
            // mode keeps only the brush's smoothing.
            if mode != "block" || what.contains("smoothing") {
                assert!(history_stroke_pixels(mode, switched, None) != plain, "{mode} {what}: both brushes paint alike");
            }
        }
    }
}
