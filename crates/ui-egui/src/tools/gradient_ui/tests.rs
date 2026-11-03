use super::*;

fn app_with_gradient(style: &str) -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 200, "height": 120})).unwrap();
    app.ui.tool = Tool::Gradient;
    app.ui.tool_options.gradient_style = style.into();
    app
}

fn mods() -> egui::Modifiers {
    egui::Modifiers::NONE
}

fn drag(app: &mut PhotocraftApp, a: [f64; 2], b: [f64; 2]) {
    crate::canvas::tool_event(app, ToolEvent::Down { x: a[0], y: a[1], pressure: 1.0 }, mods());
    crate::canvas::tool_event(app, ToolEvent::Move { x: (a[0] + b[0]) / 2.0, y: (a[1] + b[1]) / 2.0, pressure: 1.0 }, mods());
    crate::canvas::tool_event(app, ToolEvent::Move { x: b[0], y: b[1], pressure: 1.0 }, mods());
    crate::canvas::tool_event(app, ToolEvent::Up { x: b[0], y: b[1] }, mods());
}

fn get(app: &mut PhotocraftApp) -> Value {
    app.run(cmds::GET, json!({})).unwrap()
}

#[test]
fn thumbnails_show_the_gradient() {
    use photocraft_color::Color;
    // Black to white, left to right: the left edge is dark, the right light, top = bottom.
    let f = Fill::gradient(vec![(0.0, Color::BLACK), (1.0, Color::WHITE)], 0.0, 1.0, GradientStyle::Linear, false);
    let img = thumbnail_image(&f).unwrap();
    let n = img.size[0];
    let at = |x: usize, y: usize| img.pixels[y * n + x];
    assert!(at(0, n / 2).r() < 10 && at(n - 1, n / 2).r() > 245, "{:?} {:?}", at(0, n / 2), at(n - 1, n / 2));
    assert_eq!(at(n / 3, 0), at(n / 3, n - 1));
    // Reverse flips colour and opacity alike; a transparent stop lets the checkerboard through.
    let mut r = f.clone();
    if let Fill::Gradient { reverse, opacity_stops, .. } = &mut r {
        *reverse = true;
        *opacity_stops = vec![(0.0, 1.0), (1.0, 0.0)];
    }
    let img = thumbnail_image(&r).unwrap();
    let (left, right) = (img.pixels[n / 2 * n], img.pixels[n / 2 * n + n - 1]);
    assert!(left.a() < 10 && right.r() < 10 && right.a() > 245, "{left:?} {right:?}");
    // A radial thumbnail is centred.
    let rad = Fill::gradient(vec![(0.0, Color::BLACK), (1.0, Color::WHITE)], 0.0, 1.0, GradientStyle::Radial, false);
    let img = thumbnail_image(&rad).unwrap();
    assert!(img.pixels[n / 2 * n + n / 2].r() < img.pixels[0].r());
    assert!(thumbnail_image(&Fill::Solid(Color::WHITE)).is_none());
}

#[test]
fn options_bar_swatch_opens_the_editor_and_reaches_the_gradients_panel() {
    use egui_kittest::{Harness, kittest::Queryable};
    let app = app_with_gradient("linear");
    let mut h = Harness::builder().with_size(vec2(440.0, 560.0)).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            if preset_swatch(app, ui) {
                app.ui.panels.gradient_editor = true;
            }
            let ctx = ui.ctx().clone();
            editor_window(app, &ctx);
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::Pro);
    h.run_steps(2);
    // The swatch opens the Gradient Editor window (Photoshop's behaviour), not a popup.
    h.get_by_label("Gradient picker").click();
    h.run_steps(2);
    assert!(h.state().ui.panels.gradient_editor, "the swatch opened the editor window");
    // Quick preset picking lives in the editor: its Gradients button shows the Gradients panel.
    h.get_by_label("Gradients").click();
    h.run_steps(2);
    assert!(h.state().ui.panels.color, "the Gradients panel opened");
    assert_eq!(h.state().ui.dock_tabs.color, 2, "on the Gradients tab");
    assert_eq!(h.state().session.active().unwrap().doc.layers.len(), 1, "no document layers edited");
}

#[test]
fn live_drag_creates_a_fill_layer_and_handles_edit_it() {
    let mut app = app_with_gradient("linear");
    drag(&mut app, [20.0, 60.0], [180.0, 60.0]);
    assert_eq!(app.session.active().unwrap().doc.layers.len(), 2, "a Gradient Fill layer");
    let g = get(&mut app);
    assert!((g["to"][0].as_f64().unwrap() - 180.0).abs() < 1e-3);
    // Drag the end handle: one history step.
    let before = app.session.active().unwrap().history.past_len();
    drag(&mut app, [180.0, 60.0], [150.0, 100.0]);
    let g = get(&mut app);
    assert!((g["to"][0].as_f64().unwrap() - 150.0).abs() < 1e-3 && (g["to"][1].as_f64().unwrap() - 100.0).abs() < 1e-3, "{g}");
    assert_eq!(app.session.active().unwrap().history.past_len(), before + 1);
    // Click on the line adds a stop; dragging it off the line deletes it.
    let (s, e) = ([20.0, 60.0], [150.0, 100.0]);
    // A third of the way (the midpoint diamond sits half-way).
    let mid = [s[0] + (e[0] - s[0]) / 3.0, s[1] + (e[1] - s[1]) / 3.0];
    crate::canvas::tool_event(&mut app, ToolEvent::Down { x: mid[0], y: mid[1], pressure: 1.0 }, mods());
    crate::canvas::tool_event(&mut app, ToolEvent::Up { x: mid[0], y: mid[1] }, mods());
    assert_eq!(get(&mut app)["stops"].as_array().unwrap().len(), 3);
    drag(&mut app, mid, [mid[0], mid[1] - 80.0]);
    assert_eq!(get(&mut app)["stops"].as_array().unwrap().len(), 2);
    // A drag away from the widget redraws the selected gradient (no new layer).
    drag(&mut app, [100.0, 10.0], [100.0, 110.0]);
    assert_eq!(app.session.active().unwrap().doc.layers.len(), 2);
    assert!((get(&mut app)["from"][1].as_f64().unwrap() - 10.0).abs() < 1e-3);
}

#[test]
fn dragging_the_midpoint_diamond_moves_it() {
    let mut app = app_with_gradient("linear");
    drag(&mut app, [20.0, 60.0], [180.0, 60.0]);
    // The untouched midpoint sits at the segment centre (doc x = 100). Drag it to a quarter.
    drag(&mut app, [100.0, 60.0], [60.0, 60.0]);
    let g = get(&mut app);
    assert_eq!(g["stops"].as_array().map(Vec::len), Some(2), "a midpoint drag must not add a stop: {g}");
    let m = g["midpoints"][0].as_f64().unwrap_or(f64::NAN);
    assert!((m - 0.25).abs() < 1e-3, "the midpoint did not move: {g}");
}

#[test]
fn shift_snaps_live_drags_to_45_degrees() {
    let mut app = app_with_gradient("linear");
    let shift = egui::Modifiers::SHIFT;
    crate::canvas::tool_event(&mut app, ToolEvent::Down { x: 20.0, y: 20.0, pressure: 1.0 }, shift);
    crate::canvas::tool_event(&mut app, ToolEvent::Move { x: 70.0, y: 62.0, pressure: 1.0 }, shift);
    crate::canvas::tool_event(&mut app, ToolEvent::Up { x: 90.0, y: 84.0 }, shift);
    let g = get(&mut app);
    let (fx, fy, tx, ty) = (g["from"][0].as_f64().unwrap(), g["from"][1].as_f64().unwrap(), g["to"][0].as_f64().unwrap(), g["to"][1].as_f64().unwrap());
    assert!(((tx - fx) - (ty - fy)).abs() < 1e-2 && tx > 80.0, "45°: {g}");
    // Dragging the end handle with ⇧ snaps too (horizontal here).
    crate::canvas::tool_event(&mut app, ToolEvent::Down { x: tx, y: ty, pressure: 1.0 }, shift);
    crate::canvas::tool_event(&mut app, ToolEvent::Up { x: 120.0, y: 26.0 }, shift);
    let g = get(&mut app);
    assert!((g["to"][1].as_f64().unwrap() - fy).abs() < 1e-2, "horizontal: {g}");
}

#[test]
fn classic_mode_paints_pixels() {
    let mut app = app_with_gradient("linear");
    app.ui.tool_options.gradient_classic = true;
    drag(&mut app, [20.0, 60.0], [180.0, 60.0]);
    assert_eq!(app.session.active().unwrap().doc.layers.len(), 1, "no fill layer in classic mode");
}

#[test]
fn classic_drag_preserves_preset_transparency_at_all_depths() {
    for depth in [8, 16, 32] {
        for reverse in [false, true] {
            let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
            app.run("file.new", json!({"width": 200, "height": 120, "depth": depth, "background": "transparent"})).unwrap();
            app.ui.tool = Tool::Gradient;
            app.ui.tool_options.gradient_classic = true;
            app.ui.tool_options.gradient_reverse = reverse;
            app.ui.tool_options.gradient_dither = false;
            app.ui.tool_options.fill_opacity = 50.0;
            app.session.tools.foreground = [1.0, 0.0, 0.0, 1.0];
            app.run("gradient.presets.select", json!({"preset": "Foreground to Transparent"})).unwrap();
            let before = app.session.active().unwrap().history.past_len();
            drag(&mut app, [20.0, 60.0], [180.0, 60.0]);
            let st = app.session.active().unwrap();
            let surface = st.doc.layers[0].surface().unwrap();
            for x in [0, 100, 199] {
                let u = ((x as f32 - 20.0) / 160.0).clamp(0.0, 1.0);
                let expected = 0.5 * if reverse { u } else { 1.0 - u };
                let px = surface.pixel(x, 60);
                assert!((px[3] - expected).abs() < 0.01, "depth {depth}, reverse {reverse}, x {x}: {px:?}");
                if expected > 0.01 {
                    assert!(px[0] > 0.99 && px[1] < 0.01 && px[2] < 0.01, "preset colour: {px:?}");
                }
            }
            assert_eq!(st.history.past_len(), before + 1);
            app.run("edit.undo", json!({})).unwrap();
            assert_eq!(app.session.active().unwrap().doc.layers[0].surface().unwrap().pixel(100, 60)[3], 0.0);
            app.run("gradient.presets.select", json!({"stops": [[0, "#00ff00"], [1, "#00ff00"]], "transparency": [[0, 60], [1, 20]]})).unwrap();
            drag(&mut app, [20.0, 60.0], [180.0, 60.0]);
            let px = app.session.active().unwrap().doc.layers[0].surface().unwrap().pixel(100, 60);
            assert!((px[3] - 0.2).abs() < 0.01 && px[1] > 0.99 && px[0] < 0.01, "custom semi-transparent colour: {px:?}");
        }
    }
}

#[test]
fn sidebar_transparent_preset_reaches_classic_canvas_over_existing_art() {
    use egui_kittest::{Harness, kittest::Queryable};
    let mut app = app_with_gradient("linear");
    app.run("paint.gradient", json!({"from": [0, 0], "to": [199, 0], "colors": ["#ff0000", "#ff0000"]})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app.ui.tool_options.gradient_classic = true;
    app.ui.tool_options.gradient_dither = false;
    let mut h = Harness::builder().with_size(vec2(420.0, 600.0)).build_ui_state(|ui, app| crate::preset_panels::gradients_panel(app, ui), app);
    h.get_by_label("Foreground to Transparent").click();
    h.run();
    assert_eq!(h.state().session.presets.gradient.name, "Foreground to Transparent");
    drag(h.state_mut(), [20.0, 60.0], [180.0, 60.0]);
    let st = h.state().session.active().unwrap();
    assert_eq!(st.doc.layers.len(), 2);
    let surface = st.doc.layers[1].surface().unwrap();
    assert!(surface.pixel(0, 60)[3] > 0.99);
    assert!((surface.pixel(100, 60)[3] - 0.5).abs() < 0.01);
    assert!(surface.pixel(199, 60)[3] < 0.01, "transparent end reveals existing art");
    assert_eq!(st.doc.layers[0].surface().unwrap().pixel(100, 60), vec![1.0, 0.0, 0.0, 1.0], "existing art is unchanged");
}

#[test]
fn gradient_blend_mode_reaches_live_and_classic_drags() {
    use photocraft_color::BlendMode;
    for mode in [BlendMode::Difference, BlendMode::Multiply, BlendMode::Screen, BlendMode::Exclusion] {
        let mut live = app_with_gradient("linear");
        live.ui.tool_options.gradient_blend_mode = mode;
        drag(&mut live, [20.0, 60.0], [180.0, 60.0]);
        let layer = live.session.active().unwrap().doc.layers.last().unwrap();
        assert_eq!(layer.blend, mode, "live {mode:?}");

        let mut classic = app_with_gradient("linear");
        classic.ui.tool_options.gradient_classic = true;
        classic.ui.tool_options.gradient_blend_mode = mode;
        classic.session.tools.foreground = [1.0, 0.0, 0.0, 1.0];
        classic.session.tools.background = [1.0, 0.0, 0.0, 1.0];
        drag(&mut classic, [20.0, 60.0], [180.0, 60.0]);
        let st = classic.session.active().unwrap();
        let px = st.doc.layers[0].surface().unwrap().pixel(60, 60);
        let expected = match mode {
            BlendMode::Difference | BlendMode::Exclusion => [0.0, 1.0, 1.0],
            BlendMode::Multiply => [1.0, 0.0, 0.0],
            BlendMode::Screen => [1.0, 1.0, 1.0],
            _ => [1.0, 1.0, 1.0],
        };
        for (actual, want) in px.iter().zip(expected) {
            assert!((actual - want).abs() < 0.02, "classic {mode:?}: {px:?}");
        }
    }
}

#[test]
fn changing_live_gradient_mode_updates_selected_fill_layer() {
    use photocraft_color::BlendMode;
    let mut app = app_with_gradient("linear");
    drag(&mut app, [20.0, 60.0], [180.0, 60.0]);
    let before = app.ui.tool_options.clone();
    app.ui.tool_options.gradient_blend_mode = BlendMode::Difference;
    options_changed(&mut app, &before);
    let layer = app.session.active().unwrap().doc.layers.last().unwrap();
    assert_eq!(layer.blend, BlendMode::Difference);
}

#[test]
fn changing_live_gradient_opacity_updates_selected_fill_layer() {
    let mut app = app_with_gradient("linear");
    drag(&mut app, [20.0, 60.0], [180.0, 60.0]);
    let past = app.session.active().unwrap().history.past_len();
    // A scrub is live on every frame and one undo step.
    for pct in [70.0, 55.0, 40.0] {
        app.ui.tool_options.fill_opacity = pct;
        opacity_changed(&mut app, Some(7));
    }
    opacity_changed(&mut app, None);
    let st = app.session.active().unwrap();
    assert!((st.doc.layers.last().unwrap().opacity - 0.4).abs() < 1e-6);
    assert_eq!(st.history.past_len(), past + 1, "one undo step, none for an unchanged value");
    // Classic mode paints pixels: the options bar leaves the layer alone.
    app.ui.tool_options.gradient_classic = true;
    app.ui.tool_options.fill_opacity = 80.0;
    opacity_changed(&mut app, None);
    assert!((app.session.active().unwrap().doc.layers.last().unwrap().opacity - 0.4).abs() < 1e-6);
}

#[test]
fn preview_shows_the_drag_before_commit() {
    let mut app = app_with_gradient("radial");
    crate::canvas::tool_event(&mut app, ToolEvent::Down { x: 100.0, y: 60.0, pressure: 1.0 }, mods());
    crate::canvas::tool_event(&mut app, ToolEvent::Move { x: 140.0, y: 60.0, pressure: 1.0 }, mods());
    let (doc, key) = display_doc(&mut app, 0).expect("a preview while drawing");
    assert!(key & TAG != 0);
    assert!(doc.layer(PREVIEW_LAYER).is_some(), "the new layer previews");
    assert_eq!(app.session.active().unwrap().doc.layers.len(), 1, "nothing committed yet");
    crate::canvas::tool_event(&mut app, ToolEvent::Up { x: 140.0, y: 60.0 }, mods());
    assert!(display_doc(&mut app, 0).is_none());
}

/// Regression: the options bar's gradient swatch could not open anything. In live mode it
/// sensed hover only, so `clicked()` was never true; in classic mode it sensed clicks and
/// promised "Click to edit the gradient" but never read the response. Both now open the
/// Gradient Editor window.
#[test]
fn the_options_bar_swatch_opens_the_gradient_editor_in_both_modes() {
    use egui_kittest::kittest::Queryable;

    for classic in [false, true] {
        let mut app = app_with_gradient("linear");
        app.ui.tool_options.gradient_classic = classic;
        let mut h = egui_kittest::Harness::builder().with_size(vec2(1400.0, 900.0)).build_ui_state(
            |ui, app: &mut PhotocraftApp| {
                crate::panels::options_bar(app, ui);
                let ctx = ui.ctx().clone();
                editor_window(app, &ctx);
            },
            app,
        );
        PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
        h.run_steps(4);
        assert!(!h.state().ui.panels.gradient_editor, "classic={classic}: starts closed");
        h.get_by_label("Gradient picker").click();
        h.run_steps(3);
        assert!(h.state().ui.panels.gradient_editor, "classic={classic}: the swatch opened the editor");
    }
}

/// The editor itself renders in both states: with a gradient fill layer selected it shows the
/// full stops editor, and with none selected it must still draw (and say what to do) instead of
/// opening an empty window or panicking.
#[test]
fn the_editor_window_renders_with_and_without_a_gradient_layer() {
    let ctx = egui::Context::default();
    PhotocraftApp::setup_context(&ctx, crate::theme::ThemeKind::ALL[0]);
    // `run_ui` hands back a `FullOutput` whose texture deltas the caller has to consume.
    let run = |app: &mut PhotocraftApp| {
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| editor_window(app, ui.ctx()));
        out.textures_delta.clear();
    };

    // Nothing selected.
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.ui.panels.gradient_editor = true;
    run(&mut app);
    assert!(app.ui.panels.gradient_editor, "the window stays open");

    // A committed live gradient fill layer: `selected_gradient` finds it, so the full editor
    // (combined stops and Gradient Options) is what gets built.
    let mut app = app_with_gradient("linear");
    drag(&mut app, [10.0, 10.0], [150.0, 90.0]);
    assert!(selected_gradient(&app).is_some(), "the drag left a gradient fill layer selected");
    app.ui.panels.gradient_editor = true;
    run(&mut app);
    assert!(app.ui.panels.gradient_editor);

    // Closed: the window draws nothing and leaves the flag alone.
    app.ui.panels.gradient_editor = false;
    run(&mut app);
    assert!(!app.ui.panels.gradient_editor);
}

/// With no gradient fill layer selected the editor edits the Gradient tool's current gradient
/// in place (Photoshop's editor edits the tool gradient): a click on the strip adds a stop to
/// it instead of only pointing at the Gradients panel's presets.
#[test]
fn the_editor_edits_the_current_gradient_without_a_layer() {
    use egui_kittest::kittest::Queryable;

    let app = app_with_gradient("linear");
    let before = app.session.presets.gradient.stops.len();
    let mut h = egui_kittest::Harness::builder().with_size(vec2(1400.0, 900.0)).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            let ctx = ui.ctx().clone();
            editor_window(app, &ctx);
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    // Open the window once the fonts are bound (the build's first frame runs without them).
    h.run_steps(2);
    h.state_mut().ui.panels.gradient_editor = true;
    h.run_steps(4);
    h.get_by_label("Gradient stops").click();
    h.run_steps(3);
    assert_eq!(h.state().session.presets.gradient.stops.len(), before + 1, "the click added a stop to the current gradient");
}

/// The editor's Save As Preset button files the current gradient into the Gradients panel
/// under a fresh "Custom" name.
#[test]
fn the_editor_saves_the_current_gradient_as_a_preset() {
    use egui_kittest::kittest::Queryable;

    let app = app_with_gradient("linear");
    let has_custom = |s: &PhotocraftApp| s.session.presets.gradients.iter().any(|g| g.items.iter().any(|p| p.name == "Custom"));
    let mut h = egui_kittest::Harness::builder().with_size(vec2(1400.0, 900.0)).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            let ctx = ui.ctx().clone();
            editor_window(app, &ctx);
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    h.run_steps(2);
    h.state_mut().ui.panels.gradient_editor = true;
    h.run_steps(4);
    assert!(!has_custom(h.state()), "no Custom preset yet");
    h.get_by_label("Save As Preset").click();
    h.run_steps(3);
    assert!(has_custom(h.state()), "the button saved the current gradient as a preset");
}

/// A hidden gradient fill layer is still editable from the editor (the Properties panel edits
/// it whether or not it is drawn), so the window must not require visibility.
#[test]
fn the_editor_finds_a_hidden_gradient_layer() {
    let mut app = app_with_gradient("linear");
    drag(&mut app, [10.0, 10.0], [150.0, 90.0]);
    let id = app.session.active().unwrap().active_layer.unwrap();
    app.run("layer.setProps", json!({"layer": id.0, "visible": false})).unwrap();
    assert!(active_gradient(&app).is_none(), "the canvas widget skips a hidden layer");
    assert!(selected_gradient(&app).is_some(), "the editor still edits it");
}

/// #4577: the tessellator snaps the checkerboard's rects to whole pixels but leaves the ramp's
/// mesh where it was laid out, so at a half-pixel offset the ramp stopped a pixel short of the
/// checker and a row of it showed under the gradient. The ramp must cover the checker exactly.
#[test]
fn the_ramp_covers_its_checkerboard_at_half_pixel_offsets() {
    let ctx = egui::Context::default();
    for ppp in [1.0, 2.0] {
        ctx.set_pixels_per_point(ppp);
        let r = Rect::from_min_size(pos2(10.0 + 0.5 / ppp, 7.0 + 0.5 / ppp), vec2(96.0, 20.0));
        // The new scale applies from the next frame.
        ctx.run_ui(egui::RawInput::default(), |_| {}).textures_delta.clear();
        let mut painted_at = 0.0;
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            painted_at = ui.ctx().pixels_per_point();
            paint_ramp(ui.painter(), r, |u| [u, u, u, 1.0]);
        });
        out.textures_delta.clear();
        assert_eq!(painted_at, ppp);
        let checker = out
            .shapes
            .iter()
            .find_map(|s| match &s.shape {
                egui::Shape::Rect(rs) if rs.fill == Color32::from_gray(250) => Some(rs.rect),
                _ => None,
            })
            .expect("the checkerboard's base rect");
        let ramp = out
            .shapes
            .iter()
            .find_map(|s| match &s.shape {
                egui::Shape::Mesh(m) => Some(m.calc_bounds()),
                _ => None,
            })
            .expect("the ramp mesh");
        assert_eq!(ramp, checker.round_to_pixels(ppp), "{ppp}x: the ramp covers every pixel of the checker");
    }
}
