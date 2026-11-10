//! UI tests for the adjustment editors: the whole app (eframe harness, CPU canvas) with an
//! adjustment layer on a new document, edited through the Properties panel with real pointer and
//! key input (issue #12: "add an adjustment layer to a new document, edit it, crash"), and the
//! Curves point interactions (issue #43).

use egui::{Pos2, Rect, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use photocraft_doc::{Adjustment, LayerContent, LayerId};
use serde_json::json;

use crate::PhotocraftApp;

const ALL_KINDS: [&str; 16] = [
    "brightnessContrast",
    "levels",
    "curves",
    "exposure",
    "vibrance",
    "hueSaturation",
    "colorBalance",
    "blackWhite",
    "photoFilter",
    "channelMixer",
    "invert",
    "posterize",
    "threshold",
    "gradientMap",
    "selectiveColor",
    "colorLookup",
];

fn app_harness(kind: &str, mode: &str, depth: u32) -> Harness<'static, PhotocraftApp> {
    app_harness_stepped(kind, mode, depth, 0.25)
}

/// [`app_harness`] advancing `step_dt` seconds per frame (double-clicks need real frame times).
fn app_harness_stepped(kind: &str, mode: &str, depth: u32, step_dt: f32) -> Harness<'static, PhotocraftApp> {
    let (kind, mode) = (kind.to_string(), mode.to_string());
    let mut h = Harness::builder().with_size(vec2(1440.0, 900.0)).with_max_steps(64).with_step_dt(step_dt).build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        let mut s = photocraft_engine::Session::new();
        s.execute("file.new", json!({"width": 96, "height": 64, "mode": mode, "depth": depth})).unwrap();
        // Two tones so histograms (and Levels' Auto) have something to work with.
        s.execute("select.rect", json!({"x": 0, "y": 0, "width": 48, "height": 64})).unwrap();
        s.execute("edit.fill", json!({"color": "#6a3020"})).unwrap();
        s.execute("select.deselect", json!({})).unwrap();
        s.execute(&format!("layer.newAdjustmentLayer.{kind}"), json!({})).unwrap();
        let mut app = PhotocraftApp::new(s, crate::Services::default());
        // The default Properties group gives way to Layers (#147) and scrolls taller editors;
        // these tests drive every control without scrolling, so they size it like a user would.
        app.ui.dock.heights.insert(crate::dock::Group::Properties, 560.0);
        app
    });
    h.run_steps(8);
    h
}

fn layer(h: &Harness<'_, PhotocraftApp>) -> (LayerId, Adjustment) {
    let st = h.state().session.active().unwrap();
    let id = st.active_layer.unwrap();
    match &st.doc.layer(id).unwrap().content {
        LayerContent::Adjustment(a) => (id, a.clone()),
        other => panic!("{other:?}"),
    }
}

fn drag(h: &mut Harness<'_, PhotocraftApp>, from: Pos2, to: Pos2) {
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

fn click(h: &mut Harness<'_, PhotocraftApp>, at: Pos2) {
    h.hover_at(at);
    h.run_steps(1);
    h.drag_at(at);
    h.run_steps(1);
    h.drop_at(at);
    h.run_steps(3);
}

fn label_rect(h: &Harness<'_, PhotocraftApp>, text: &str) -> Option<Rect> {
    // The last caption: the Properties header repeats the kind's name above its controls.
    // `get_by_label` hides a caption that names a control, so ask for the label node itself;
    // a checkbox paints its caption as the control, so fall back to the last matching widget.
    h.query_all(egui_kittest::kittest::by().label(text).include_labels())
        .rfind(|n| n.accesskit_node().role() == egui::accesskit::Role::Label)
        .map(|n| n.rect())
        .or_else(|| h.query_all_by_label(text).last().map(|n| n.rect()))
}

fn curves_graph(h: &Harness<'_, PhotocraftApp>, id: LayerId) -> Rect {
    h.ctx.data(|d| d.get_temp::<Rect>(egui::Id::unique(("adjust-layer", id.0)).with("curves-graph"))).expect("curves graph drawn")
}

/// Issue #12: every adjustment kind, on a new document at 8/16/32 bits and in RGB, Grayscale,
/// CMYK and Lab, opens its Properties editor and takes an edit through real input without
/// panicking; slider kinds commit exactly one history step per gesture.
#[test]
fn every_adjustment_layer_edits_through_properties() {
    for (mode, depth) in [("rgb", 8), ("rgb", 16), ("rgb", 32), ("gray", 8), ("cmyk", 8), ("lab", 16)] {
        for kind in ALL_KINDS {
            let mut h = app_harness(kind, mode, depth);
            let (id, before) = layer(&h);
            let steps = h.state().session.active().unwrap().history.past_len();
            // A probe label per editor; the control sits just below it.
            let probe = match kind {
                "brightnessContrast" => Some("Brightness"),
                "exposure" => Some("Exposure"),
                "vibrance" => Some("Vibrance"),
                "hueSaturation" => Some("Hue"),
                "colorBalance" => Some("Cyan  ·  Red"),
                "blackWhite" => Some("Reds"),
                "photoFilter" => Some("Density"),
                "channelMixer" => Some("Constant"),
                "posterize" => Some("Levels"),
                "threshold" => Some("Threshold Level"),
                "selectiveColor" => Some("Cyan"),
                _ => None,
            };
            match kind {
                "curves" => {
                    let g = curves_graph(&h, id);
                    drag(&mut h, g.center(), g.center() + vec2(0.0, -40.0));
                }
                "levels" => {
                    // The output black handle sits below the output gradient bar.
                    let r = label_rect(&h, "Output Levels:").unwrap_or_else(|| panic!("{kind} {mode}: Output Levels"));
                    let y = r.bottom() + 17.0;
                    drag(&mut h, Pos2::new(r.left() + 6.0, y), Pos2::new(r.left() + 60.0, y));
                }
                "gradientMap" => {
                    let r = label_rect(&h, "Reverse").unwrap_or_else(|| panic!("{kind} {mode}: Reverse"));
                    click(&mut h, r.center());
                }
                _ => {
                    if let Some(p) = probe {
                        let r = label_rect(&h, p).unwrap_or_else(|| panic!("{kind} {mode}: no `{p}` label"));
                        let y = r.bottom() + 14.0;
                        drag(&mut h, Pos2::new(r.left() + 40.0, y), Pos2::new(r.left() + 120.0, y));
                    }
                }
            }
            h.run_steps(4);
            let (_, after) = layer(&h);
            assert!(h.state().live_adjust.is_none(), "{kind} {mode}: preview ended with the gesture");
            if !matches!(kind, "invert" | "colorLookup") {
                assert_ne!(after, before, "{kind} {mode}@{depth}: the edit reached the layer");
                assert_eq!(h.state().session.active().unwrap().history.past_len(), steps + 1, "{kind} {mode}: one history step");
            }
        }
    }
}

/// #3264: wheel browsing previews without recording one edit per selected LUT.
#[test]
fn color_lookup_picker_cycles_luts_with_the_mouse_wheel() {
    let mut h = app_harness("colorLookup", "rgb", 8);
    let (_, before) = layer(&h);
    let Adjustment::ColorLookup { name: before_name, .. } = before else { panic!("Color Lookup layer") };
    let label = label_rect(&h, "3D LUT File").expect("Color Lookup file label");
    let at = h
        .query_all_by_role(egui::accesskit::Role::ComboBox)
        .min_by(|a, b| a.rect().center().distance(label.center()).total_cmp(&b.rect().center().distance(label.center())))
        .expect("Color Lookup picker")
        .rect()
        .center();
    let steps = h.state().session.active().unwrap().history.past_len();
    h.hover_at(at);
    h.run_steps(1);
    h.query_all_by_role(egui::accesskit::Role::ComboBox)
        .min_by(|a, b| a.rect().center().distance(label.center()).total_cmp(&b.rect().center().distance(label.center())))
        .expect("Color Lookup picker")
        .click();
    h.run_steps(1);
    h.key_press(egui::Key::ArrowDown);
    h.run_steps(1);
    assert_eq!(h.state().session.active().unwrap().history.past_len(), steps, "arrow navigation previews");
    assert!(h.state().live_adjust.is_some(), "arrow navigation selects a preview LUT");
    for _ in 0..3 {
        h.event(egui::Event::MouseWheel {
            source: egui::MouseWheelSource::Unknown,
            unit: egui::MouseWheelUnit::Line,
            delta: vec2(0.0, -1.0),
            phase: egui::TouchPhase::Move,
            modifiers: Default::default(),
        });
        h.run_steps(1);
    }
    assert_eq!(h.state().session.active().unwrap().history.past_len(), steps, "cycling remains a preview");
    assert!(h.state().live_adjust.is_some(), "the selected LUT is previewed");
    h.run_steps(4);
    let (_, after) = layer(&h);
    let Adjustment::ColorLookup { name: after_name, .. } = after else { panic!("Color Lookup layer") };
    assert_ne!(after_name, before_name, "wheel browsing selects a different LUT");
    assert_eq!(h.state().session.active().unwrap().history.past_len(), steps + 1, "one wheel gesture makes one adjustment edit");
    assert!(h.state().live_adjust.is_none(), "the completed wheel gesture is committed");
}

fn curve_of(a: &Adjustment) -> Vec<[f32; 2]> {
    match a {
        Adjustment::Curves { master, .. } => master.iter().map(|p| [(p.input * 255.0).round(), (p.output * 255.0).round()]).collect(),
        other => panic!("{other:?}"),
    }
}

/// #8984: the Curves graph in Properties grows with the panel instead of stopping at 300
/// points, stays square, and edits where it is clicked at its new size.
#[test]
fn curves_graph_grows_with_the_properties_panel() {
    let mut h = app_harness("curves", "rgb", 8);
    let (id, _) = layer(&h);
    let narrow = curves_graph(&h, id);
    assert!(narrow.width() < 300.0, "{narrow:?}");
    crate::panels::request_dock_width(&h.ctx, 760.0);
    h.run_steps(8);
    let wide = curves_graph(&h, id);
    // 760 points of panel, but the 900-point window leaves room for 540 beside the other rows.
    assert!(wide.width() > narrow.width() + 250.0 && wide.height() <= 900.0 - 360.0, "{narrow:?} → {wide:?}");
    assert!((wide.width() - wide.height()).abs() < 1.0, "square: {wide:?}");
    // A point added at the graph's centre lands at 128, 128 of the curve.
    click(&mut h, wide.center());
    let c = curve_of(&layer(&h).1);
    assert_eq!(c.len(), 3, "{c:?}");
    assert!((c[1][0] - 127.5).abs() < 2.0 && (c[1][1] - 127.5).abs() < 2.0, "{c:?}");
}

/// Issue #43: pressing on a point selects it (no new point), Delete/Backspace and ⌘/Ctrl-click
/// remove points, dragging off the graph removes one, endpoints stay.
#[test]
fn curves_points_are_added_selected_and_deleted() {
    let mut h = app_harness("curves", "rgb", 8);
    let (id, _) = layer(&h);
    let g = curves_graph(&h, id);
    let at = |v: [f32; 2]| Pos2::new(g.left() + v[0] / 255.0 * g.width(), g.bottom() - v[1] / 255.0 * g.height());
    // Add a point.
    click(&mut h, at([128.0, 128.0]));
    assert_eq!(curve_of(&layer(&h).1).len(), 3);
    // Press 4 px off it and drag: moves that point, adds none.
    drag(&mut h, at([128.0, 128.0]) + vec2(4.0, 3.0), at([128.0, 180.0]));
    let c = curve_of(&layer(&h).1);
    assert_eq!(c.len(), 3, "{c:?}");
    assert!(c[1][1] > 165.0, "{c:?}");
    // Delete key removes the selected point (the graph has focus).
    h.key_press(egui::Key::Delete);
    h.run_steps(3);
    assert_eq!(curve_of(&layer(&h).1).len(), 2);
    // Backspace too.
    click(&mut h, at([64.0, 100.0]));
    assert_eq!(curve_of(&layer(&h).1).len(), 3);
    h.key_press(egui::Key::Backspace);
    h.run_steps(3);
    assert_eq!(curve_of(&layer(&h).1).len(), 2);
    // ⌘/Ctrl-click removes.
    click(&mut h, at([190.0, 120.0]));
    assert_eq!(curve_of(&layer(&h).1).len(), 3);
    let p = at([190.0, 120.0]);
    h.event_modifiers(egui::Event::PointerMoved(p), egui::Modifiers::COMMAND);
    h.event_modifiers(
        egui::Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::COMMAND },
        egui::Modifiers::COMMAND,
    );
    h.run_steps(1);
    h.event_modifiers(
        egui::Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::COMMAND },
        egui::Modifiers::COMMAND,
    );
    h.run_steps(3);
    assert_eq!(curve_of(&layer(&h).1).len(), 2);
    // Dragging a point off the graph removes it; endpoints survive both.
    click(&mut h, at([100.0, 60.0]));
    drag(&mut h, at([100.0, 60.0]), g.right_center() + vec2(80.0, 0.0));
    assert_eq!(curve_of(&layer(&h).1).len(), 2);
    drag(&mut h, at([0.0, 0.0]), g.left_top() + vec2(-80.0, -80.0));
    click(&mut h, at([0.0, 0.0]));
    h.key_press(egui::Key::Delete);
    h.run_steps(3);
    assert_eq!(curve_of(&layer(&h).1).len(), 2, "endpoints can't be deleted");
}

/// Capture and history are shared with Camera Raw, but Properties commits on release.
#[test]
fn curves_fast_release_commits_one_edit_and_undo_restores_the_curve() {
    let mut h = app_harness("curves", "rgb", 8);
    let (id, _) = layer(&h);
    let graph = curves_graph(&h, id);
    click(&mut h, graph.center());
    let (_, before) = layer(&h);
    let steps = h.state().session.active().unwrap().history.past_len();
    let at = graph.center() + vec2(4.0, 3.0);
    h.hover_at(at);
    h.run_steps(1);
    h.drag_at(at);
    h.run_steps(1);
    // Only the release frame supplies the destination, as a very fast physical gesture can.
    h.drop_at(at + vec2(40.0, -32.0));
    h.run_steps(3);
    let after = curve_of(&layer(&h).1);
    assert_eq!(after.len(), 3);
    assert!(after[1][0] > 150.0 && after[1][1] > 150.0, "{after:?}");
    assert_eq!(h.state().session.active().unwrap().history.past_len(), steps + 1);
    assert!(h.state().live_adjust.is_none());
    h.state_mut().run("edit.undo", json!({})).unwrap();
    assert_eq!(layer(&h).1, before);
}

/// Real pointer input: `n` clicks at `at` with `mods` held (2 = a double-click).
fn clicks_with(h: &mut Harness<'_, PhotocraftApp>, at: Pos2, n: usize, mods: egui::Modifiers) {
    // Well past the double-click time since the last gesture.
    h.run_steps(40);
    h.event(egui::Event::ModifiersChanged(mods));
    h.hover_at(at);
    h.run_steps(1);
    for _ in 0..n {
        for pressed in [true, false] {
            h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: mods });
            h.run_steps(1);
        }
    }
    h.event(egui::Event::ModifiersChanged(egui::Modifiers::NONE));
    h.run_steps(3);
}

fn wheel_at(h: &mut Harness<'_, PhotocraftApp>, at: Pos2, notches: f32, mods: egui::Modifiers) {
    h.event(egui::Event::ModifiersChanged(mods));
    h.hover_at(at);
    h.run_steps(1);
    h.event(egui::Event::MouseWheel {
        source: egui::MouseWheelSource::Unknown,
        unit: egui::MouseWheelUnit::Line,
        delta: vec2(0.0, notches),
        phase: egui::TouchPhase::Move,
        modifiers: mods,
    });
    h.run_steps(1);
    h.event(egui::Event::ModifiersChanged(egui::Modifiers::NONE));
    h.run_steps(3);
}

fn key_with(h: &mut Harness<'_, PhotocraftApp>, key: egui::Key, mods: egui::Modifiers) {
    h.event(egui::Event::ModifiersChanged(mods));
    for pressed in [true, false] {
        h.event(egui::Event::Key { key, physical_key: None, pressed, repeat: false, modifiers: mods });
    }
    h.event(egui::Event::ModifiersChanged(egui::Modifiers::NONE));
    h.run_steps(3);
}

fn color_balance_of(a: &Adjustment) -> ([f32; 3], [f32; 3], [f32; 3], bool) {
    match a {
        Adjustment::ColorBalance { shadows, midtones, highlights, preserve_luminosity } => (*shadows, *midtones, *highlights, *preserve_luminosity),
        other => panic!("{other:?}"),
    }
}

/// Photoshop 25.4, Color Balance in the Properties panel (measured on the real application by
/// scripted clicks and reading the layer back): a double-click on a slider's knob or track sets that
/// slider of the shown tone to 0, whatever modifier is held; the label doesn't; a click jumps; the
/// wheel does nothing; Up/Down in a field step 1, with Shift 10.
#[test]
fn color_balance_sliders_reset_and_step_like_photoshop() {
    let mut h = app_harness_stepped("colorBalance", "rgb", 8, 1.0 / 60.0);
    let (id, _) = layer(&h);
    let start = json!({"layer": id.0, "shadows": [10, 0, 0], "midtones": [40, -30, 20], "highlights": [0, 0, -15], "preserveLuminosity": false});
    h.state_mut().run("layer.setAdjustment", start.clone()).unwrap();
    h.run_steps(4);
    let label = label_rect(&h, "Cyan  ·  Red").expect("Cyan · Red row");
    let on_track = Pos2::new(label.left() + 60.0, label.bottom() + 14.0);
    let cb = |h: &Harness<'_, PhotocraftApp>| color_balance_of(&layer(h).1);

    // A click jumps to the pointer, with or without modifiers.
    clicks_with(&mut h, on_track, 1, egui::Modifiers::NONE);
    let jumped = cb(&h).1[0];
    assert!(jumped < 0.0 && jumped > -100.0, "click jumps to the pointer: {jumped}");
    for mods in [egui::Modifiers::SHIFT, egui::Modifiers::CTRL, egui::Modifiers::ALT] {
        h.state_mut().run("layer.setAdjustment", start.clone()).unwrap();
        h.run_steps(3);
        clicks_with(&mut h, on_track, 1, mods);
        assert_eq!(cb(&h).1[0], jumped, "{mods:?}-click is a plain click");
    }
    // A double-click anywhere on the slider zeroes it, with any modifier; nothing else changes.
    for mods in [egui::Modifiers::NONE, egui::Modifiers::SHIFT, egui::Modifiers::CTRL, egui::Modifiers::ALT] {
        h.state_mut().run("layer.setAdjustment", start.clone()).unwrap();
        h.run_steps(3);
        clicks_with(&mut h, on_track, 2, mods);
        assert_eq!(cb(&h), ([10.0, 0.0, 0.0], [0.0, -30.0, 20.0], [0.0, 0.0, -15.0], false), "{mods:?} double-click");
    }
    // The label is not a reset target.
    h.state_mut().run("layer.setAdjustment", start.clone()).unwrap();
    h.run_steps(3);
    clicks_with(&mut h, label.center(), 2, egui::Modifiers::NONE);
    assert_eq!(cb(&h).1, [40.0, -30.0, 20.0], "double-click on the label");
    // The panel's sliders ignore the wheel.
    wheel_at(&mut h, on_track, 3.0, egui::Modifiers::NONE);
    wheel_at(&mut h, on_track, 3.0, egui::Modifiers::SHIFT);
    assert_eq!(cb(&h).1, [40.0, -30.0, 20.0], "wheel over a Properties slider");
    // Up/Down in the field: 1, Shift: 10 (also below zero, where a half step used to round away).
    let field = h
        .query_all_by_role(egui::accesskit::Role::SpinButton)
        .map(|n| n.rect())
        .find(|r| (r.center().y - label.center().y).abs() < 8.0)
        .expect("Cyan · Red field");
    clicks_with(&mut h, field.center(), 1, egui::Modifiers::NONE);
    key_with(&mut h, egui::Key::ArrowUp, egui::Modifiers::NONE);
    assert_eq!(cb(&h).1[0], 41.0, "Up");
    key_with(&mut h, egui::Key::ArrowUp, egui::Modifiers::SHIFT);
    assert_eq!(cb(&h).1[0], 51.0, "Shift+Up");
    for _ in 0..8 {
        key_with(&mut h, egui::Key::ArrowDown, egui::Modifiers::SHIFT);
    }
    key_with(&mut h, egui::Key::ArrowDown, egui::Modifiers::NONE);
    assert_eq!(cb(&h).1[0], -30.0, "Shift+Down ×8, Down");
    key_with(&mut h, egui::Key::ArrowUp, egui::Modifiers::NONE);
    assert_eq!(cb(&h).1[0], -29.0, "Up below zero");
}

/// Photoshop 25.4, Color Balance in the Properties panel: edits that follow each other make one
/// history step (a double-click, whose first click moved the knob, undoes in one go); Reset first
/// returns to the settings the layer had when the panel started showing it (Tone choice kept),
/// then to the defaults (every tone 0, Midtones, Preserve Luminosity kept), within that step.
#[test]
fn color_balance_properties_history_and_reset_like_photoshop() {
    let mut h = app_harness_stepped("colorBalance", "rgb", 8, 1.0 / 60.0);
    let (id, _) = layer(&h);
    let start = ([10.0, 0.0, 0.0], [40.0, -30.0, 20.0], [0.0, 0.0, -15.0], false);
    h.state_mut()
        .run(
            "layer.setAdjustment",
            json!({"layer": id.0, "shadows": [10, 0, 0], "midtones": [40, -30, 20], "highlights": [0, 0, -15], "preserveLuminosity": false}),
        )
        .unwrap();
    // Show another layer, then this one again: the panel starts showing it with these settings.
    let bg = h.state().session.active().unwrap().doc.layers[0].id;
    h.state_mut().session.select_layer(bg).unwrap();
    h.run_steps(4);
    h.state_mut().session.select_layer(id).unwrap();
    h.run_steps(4);
    let cb = |h: &Harness<'_, PhotocraftApp>| color_balance_of(&layer(h).1);
    let steps = |h: &Harness<'_, PhotocraftApp>| h.state().session.active().unwrap().history.past_len();
    let label = label_rect(&h, "Cyan  ·  Red").expect("Cyan · Red row");
    let on_track = Pos2::new(label.left() + 60.0, label.bottom() + 14.0);
    let before = steps(&h);
    clicks_with(&mut h, on_track, 1, egui::Modifiers::NONE);
    clicks_with(&mut h, Pos2::new(label.left() + 90.0, on_track.y), 2, egui::Modifiers::NONE);
    assert_eq!(cb(&h).1, [0.0, -30.0, 20.0]);
    assert_eq!(steps(&h), before + 1, "a click and a double-click: one history step");
    assert!(h.state_mut().session.undo());
    h.run_steps(3);
    assert_eq!(cb(&h), start, "one Undo returns to the settings before them");
    // Edit with Highlights shown; Reset returns to the settings the panel started with.
    let hl = label_rect(&h, "Highlights").expect("Highlights radio");
    clicks_with(&mut h, hl.center(), 1, egui::Modifiers::NONE);
    clicks_with(&mut h, on_track, 1, egui::Modifiers::NONE);
    assert_ne!(cb(&h), start);
    let tone = crate::adjust_editors::layer_mem(id).with("cb-tone");
    let reset = h.get_by_label("Reset to defaults").rect();
    let before = steps(&h);
    clicks_with(&mut h, reset.center(), 1, egui::Modifiers::NONE);
    assert_eq!(cb(&h), start, "first Reset: the settings the panel started with");
    assert_eq!(h.ctx.data(|d| d.get_temp::<usize>(tone)), Some(2), "Tone choice kept");
    clicks_with(&mut h, reset.center(), 1, egui::Modifiers::NONE);
    assert_eq!(cb(&h), ([0.0; 3], [0.0; 3], [0.0; 3], false), "second Reset: defaults, Preserve Luminosity kept");
    assert_eq!(h.ctx.data(|d| d.get_temp::<usize>(tone)), Some(1), "defaults show Midtones");
    clicks_with(&mut h, reset.center(), 1, egui::Modifiers::NONE);
    assert_eq!(cb(&h), ([0.0; 3], [0.0; 3], [0.0; 3], false), "third Reset: still the defaults");
    assert_eq!(steps(&h), before, "the edit and the Resets share one history step");
}

fn map_stops(a: &Adjustment) -> Vec<(f32, [u8; 3])> {
    let q = |v: f32| (v * 255.0).round() as u8;
    match a {
        Adjustment::GradientMap { stops, .. } => stops.iter().map(|(t, c)| (*t, [q(c[0]), q(c[1]), q(c[2])])).collect(),
        other => panic!("{other:?}"),
    }
}

fn map_handle(h: &Harness<'_, PhotocraftApp>, at: f32) -> Pos2 {
    let strip = h.get_by_label("Gradient stops").rect();
    Pos2::new(strip.left() + 8.0 + at * (strip.width() - 16.0), strip.top() + 36.0)
}

#[test]
fn gradient_map_handle_drag_previews_and_commits_once_after_crossing_stops() {
    for depth in [8, 16, 32] {
        let mut h = app_harness("gradientMap", "rgb", depth);
        let (id, _) = layer(&h);
        h.state_mut()
            .run(
                "layer.setAdjustment",
                json!({
                    "layer": id.0, "stops": [[0, "#000000"], [0.25, "#ff0000"], [0.5, "#00ff00"], [1, "#ffffff"]]
                }),
            )
            .unwrap();
        h.run_steps(3);
        let before = layer(&h).1;
        let steps = h.state().session.active().unwrap().history.past_len();
        let start = map_handle(&h, 0.25);
        h.hover_at(start);
        h.run_steps(1);
        h.drag_at(start);
        h.run_steps(1);
        for location in [0.4, 0.5, 0.6, 0.75] {
            let to = map_handle(&h, location);
            h.hover_at(to);
            h.run_steps(1);
            assert!(h.state().live_adjust.is_some(), "{depth}: live preview");
            assert_eq!(h.state().session.active().unwrap().history.past_len(), steps, "{depth}: dragging records no history");
        }
        let end = map_handle(&h, 0.8);
        h.drop_at(end);
        h.run_steps(3);
        assert_eq!(map_stops(&layer(&h).1), vec![(0.0, [0, 0, 0]), (0.5, [0, 255, 0]), (0.8, [255, 0, 0]), (1.0, [255, 255, 255])]);
        assert_eq!(h.ctx.data(|d| d.get_temp::<usize>(crate::adjust_editors::layer_mem(id).with("gm-stop-selection"))), Some(2));
        assert!(h.state().live_adjust.is_none());
        assert_eq!(h.state().session.active().unwrap().history.past_len(), steps + 1);
        let after = layer(&h).1;
        h.state_mut().run("edit.undo", json!({})).unwrap();
        h.run_steps(3);
        assert_eq!(layer(&h).1, before);
        h.state_mut().run("edit.redo", json!({})).unwrap();
        h.run_steps(3);
        assert_eq!(layer(&h).1, after);
    }
}

/// #10742: a stop dragged off the strip goes as the Gradient tool's does, with a live preview, and
/// comes back if the drag returns. Two stops always stay.
#[test]
fn gradient_map_stop_dragged_off_the_strip_is_removed_in_one_step() {
    for depth in [8, 16, 32] {
        let mut h = app_harness("gradientMap", "rgb", depth);
        let (id, _) = layer(&h);
        let four = json!({"layer": id.0, "stops": [[0, "#000000"], [0.25, "#ff0000"], [0.5, "#00ff00"], [1, "#ffffff"]]});
        h.state_mut().run("layer.setAdjustment", four).unwrap();
        h.run_steps(3);
        let before = layer(&h).1;
        let steps = h.state().session.active().unwrap().history.past_len();
        let start = map_handle(&h, 0.25);
        let away = start + vec2(0.0, 90.0);
        h.hover_at(start);
        h.run_steps(1);
        h.drag_at(start);
        h.run_steps(1);
        h.hover_at(away);
        h.run_steps(1);
        let preview = h.state().live_adjust.as_ref().map(|(_, v)| v["stops"].as_array().map(Vec::len));
        assert_eq!(preview, Some(Some(3)), "{depth}: the preview drops the stop");
        assert_eq!(h.state().session.active().unwrap().history.past_len(), steps, "{depth}: dragging records no history");
        h.drop_at(away);
        h.run_steps(3);
        assert_eq!(map_stops(&layer(&h).1), vec![(0.0, [0, 0, 0]), (0.5, [0, 255, 0]), (1.0, [255, 255, 255])], "{depth}: the stop is gone");
        assert_eq!(h.state().session.active().unwrap().history.past_len(), steps + 1, "{depth}: one history step");
        h.state_mut().run("edit.undo", json!({})).unwrap();
        h.run_steps(3);
        assert_eq!(layer(&h).1, before, "{depth}: undo brings it back");

        // Off the strip and back: the stop stays, with its colour, where the drag ends.
        let back = map_handle(&h, 0.4);
        h.hover_at(start);
        h.run_steps(1);
        h.drag_at(start);
        h.run_steps(1);
        h.hover_at(away);
        h.run_steps(1);
        h.hover_at(back);
        h.run_steps(1);
        let preview = h.state().live_adjust.as_ref().map(|(_, v)| v["stops"].as_array().map(Vec::len));
        assert_eq!(preview, Some(Some(4)), "{depth}: the preview has the stop back while it's held");
        h.drop_at(back);
        h.run_steps(3);
        assert_eq!(map_stops(&layer(&h).1), vec![(0.0, [0, 0, 0]), (0.4, [255, 0, 0]), (0.5, [0, 255, 0]), (1.0, [255, 255, 255])], "{depth}: kept");
        assert_eq!(h.state().session.active().unwrap().history.past_len(), steps + 1, "{depth}: the move is one step");
    }
}

/// A drag that ends without its release reaching the strip (the drag taken away: focus lost, the
/// pointer let go elsewhere) still commits what the preview shows: a stop taken off the strip isn't
/// left removed with no History step.
#[test]
fn gradient_map_stop_drag_that_ends_elsewhere_still_commits() {
    let mut h = app_harness("gradientMap", "rgb", 8);
    let (id, _) = layer(&h);
    let four = json!({"layer": id.0, "stops": [[0, "#000000"], [0.25, "#ff0000"], [0.5, "#00ff00"], [1, "#ffffff"]]});
    h.state_mut().run("layer.setAdjustment", four).unwrap();
    h.run_steps(3);
    let steps = h.state().session.active().unwrap().history.past_len();
    let start = map_handle(&h, 0.25);
    let away = start + vec2(0.0, 90.0);
    h.hover_at(start);
    h.run_steps(1);
    h.drag_at(start);
    h.run_steps(1);
    h.hover_at(away);
    h.run_steps(1);
    // The drag ends without reaching the strip's response, then the button comes up.
    h.ctx.stop_dragging();
    h.run_steps(1);
    h.drop_at(away);
    h.run_steps(3);
    assert_eq!(map_stops(&layer(&h).1).len(), 3, "the stop the preview dropped is gone from the layer");
    assert_eq!(h.state().session.active().unwrap().history.past_len(), steps + 1, "in one History step");
}

#[test]
fn gradient_map_last_two_stops_cant_be_dragged_off_the_strip() {
    let mut h = app_harness("gradientMap", "rgb", 8);
    let (id, _) = layer(&h);
    h.state_mut().run("layer.setAdjustment", json!({"layer": id.0, "stops": [[0, "#000000"], [1, "#ffffff"]]})).unwrap();
    h.run_steps(3);
    let steps = h.state().session.active().unwrap().history.past_len();
    let start = map_handle(&h, 0.0);
    let away = start + vec2(0.0, 90.0);
    h.hover_at(start);
    h.run_steps(1);
    h.drag_at(start);
    h.run_steps(1);
    h.hover_at(away);
    h.run_steps(1);
    let preview = h.state().live_adjust.as_ref().map(|(_, v)| v["stops"].as_array().map(Vec::len));
    assert!(preview.is_none_or(|n| n == Some(2)), "the preview keeps both: {preview:?}");
    h.drop_at(away);
    h.run_steps(3);
    assert_eq!(map_stops(&layer(&h).1), vec![(0.0, [0, 0, 0]), (1.0, [255, 255, 255])], "both stay where they were");
    assert_eq!(h.state().session.active().unwrap().history.past_len(), steps, "nothing to record");
}

#[test]
fn gradient_map_strip_adds_selects_and_removes_stops_without_deleting_the_last_pair() {
    let mut h = app_harness("gradientMap", "rgb", 8);
    let (id, original) = layer(&h);
    let steps = h.state().session.active().unwrap().history.past_len();
    let middle = map_handle(&h, 0.5);
    click(&mut h, middle);
    let added = map_stops(&layer(&h).1);
    assert_eq!(added.len(), 3);
    assert!((added[1].0 - 0.5).abs() < 0.001);
    assert_eq!(added[1].1, [128, 128, 128]);
    assert_eq!(h.state().session.active().unwrap().history.past_len(), steps + 1);
    let first = map_handle(&h, 0.0);
    click(&mut h, first);
    assert_eq!(h.ctx.data(|d| d.get_temp::<usize>(crate::adjust_editors::layer_mem(id).with("gm-stop-selection"))), Some(0));
    assert_eq!(h.state().session.active().unwrap().history.past_len(), steps + 1, "selection is view state");
    click(&mut h, middle);
    let remove = h.get_by_label("Remove stop").rect().center();
    click(&mut h, remove);
    assert_eq!(layer(&h).1, original);
    assert_eq!(h.state().session.active().unwrap().history.past_len(), steps + 2);
    let first = map_handle(&h, 0.0);
    click(&mut h, first);
    let remove = h.get_by_label("Remove stop").rect().center();
    click(&mut h, remove);
    assert_eq!(layer(&h).1, original, "keep two stops");
}

/// #10742: a Gradient Map stop dragged down off the bar is removed on release (one history step);
/// while it is off the bar the live preview already leaves it out, and dragging it back onto the
/// bar before release puts it back. The last two stops can't be dragged off.
#[test]
fn gradient_map_stop_dragged_down_off_the_bar_is_removed_and_dragging_back_cancels() {
    let mut h = app_harness("gradientMap", "rgb", 8);
    let (id, _) = layer(&h);
    h.state_mut().run("layer.setAdjustment", json!({"layer": id.0, "stops": [[0, "#000000"], [0.5, "#ff0000"], [1, "#ffffff"]]})).unwrap();
    h.run_steps(3);
    let three = layer(&h).1;
    let before_cancel = h.state().session.active().unwrap().history.past_len();
    let live_stops = |h: &Harness<'_, PhotocraftApp>| h.state().live_adjust.as_ref().and_then(|(_, v)| v["stops"].as_array().map(Vec::len));
    // Off and back: nothing removed.
    let start = map_handle(&h, 0.5);
    let away = start + vec2(0.0, 30.0);
    h.hover_at(start);
    h.run_steps(1);
    h.drag_at(start);
    h.run_steps(1);
    h.hover_at(start + vec2(0.0, 15.0));
    h.run_steps(1);
    h.hover_at(away);
    h.run_steps(1);
    assert_eq!(live_stops(&h), Some(2), "off the bar the preview leaves the stop out");
    h.hover_at(start);
    h.run_steps(1);
    assert_eq!(live_stops(&h), Some(3), "back on the bar the stop returns");
    h.drop_at(start);
    h.run_steps(3);
    assert_eq!(map_stops(&layer(&h).1), map_stops(&three), "dragging back cancels the removal");
    assert_eq!(h.state().session.active().unwrap().history.past_len(), before_cancel, "a cancelled drag adds no history step");
    // Off and released: removed in one step.
    let steps = h.state().session.active().unwrap().history.past_len();
    drag(&mut h, start, away);
    assert_eq!(map_stops(&layer(&h).1), vec![(0.0, [0, 0, 0]), (1.0, [255, 255, 255])]);
    assert_eq!(h.state().session.active().unwrap().history.past_len(), steps + 1);
    assert!(h.state().live_adjust.is_none());
    h.state_mut().run("edit.undo", json!({})).unwrap();
    h.run_steps(3);
    assert_eq!(layer(&h).1, three);
    h.state_mut().run("edit.redo", json!({})).unwrap();
    h.run_steps(3);
    // Two stops stay.
    let two = layer(&h).1;
    for at in [0.0, 1.0] {
        let from = map_handle(&h, at);
        drag(&mut h, from, from + vec2(0.0, 40.0));
        assert_eq!(map_stops(&layer(&h).1), map_stops(&two), "two stops always remain");
    }
}

#[test]
fn gradient_map_drag_clamps_at_the_endpoints_and_reverse_keeps_stop_coordinates() {
    let mut h = app_harness("gradientMap", "rgb", 8);
    let reverse = label_rect(&h, "Reverse").unwrap().center();
    click(&mut h, reverse);
    let first = map_handle(&h, 0.0);
    let target = map_handle(&h, 0.3);
    drag(&mut h, first, target);
    assert!((map_stops(&layer(&h).1)[0].0 - 0.3).abs() < 0.001);
    let target = map_handle(&h, 0.3);
    drag(&mut h, target, first - vec2(60.0, 0.0));
    assert_eq!(map_stops(&layer(&h).1)[0].0, 0.0);
    let last = map_handle(&h, 1.0);
    let beyond = last + vec2(60.0, 0.0);
    let before = layer(&h).1;
    let steps = h.state().session.active().unwrap().history.past_len();
    drag(&mut h, last, beyond);
    assert_eq!(layer(&h).1, before);
    assert_eq!(h.state().session.active().unwrap().history.past_len(), steps, "a clamped no-op makes no history");
}

#[test]
fn gradient_map_stop_controls_fit_a_narrow_properties_panel_in_every_theme() {
    use crate::adjust_editors::{EditorCx, editor};
    for kind in crate::theme::ThemeKind::ALL {
        for width in [240.0, 280.0] {
            let mut h = Harness::builder().with_size(vec2(width, 360.0)).build_ui_state(
                |ui, values: &mut serde_json::Value| {
                    if !ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                        return;
                    }
                    let cx = EditorCx {
                        mem: egui::Id::unique("map-layout"),
                        hist: None,
                        gray: false,
                        swatches: [[0.0; 3], [1.0; 3]],
                        gradients: Vec::new(),
                        dialog: false,
                        dynamic_color_sliders: true,
                        isolated_curve: None,
                    };
                    editor(ui, "gradientMap", values, &cx);
                },
                json!({"stops": [[0, "#000000"], [1, "#ffffff"]]}),
            );
            PhotocraftApp::setup_context(&h.ctx, kind);
            h.run_steps(4);
            for role in [egui::accesskit::Role::Button, egui::accesskit::Role::ComboBox, egui::accesskit::Role::SpinButton] {
                for control in h.query_all_by_role(role) {
                    let r = control.rect();
                    assert!(r.left() >= 0.0 && r.right() <= width && r.width() >= 8.0, "{kind:?} width {width}: {role:?} {r:?}");
                }
            }
        }
    }
}

/// Issue #1588: the Gradient Map preset picker offers the session's gradient library (built-in
/// groups beyond Basics and the user's own saved gradients), and picking one writes its stops as
/// one history step.
#[test]
fn gradient_map_picker_offers_the_gradient_library() {
    use photocraft_engine::presets::{Group, gradients};
    let mut h = app_harness("gradientMap", "rgb", 8);
    // A short library so every row fits the popup: Basics, Blues and a user-saved group.
    let mut lib: Vec<_> = gradients::builtin().into_iter().take(2).collect();
    lib[1].items.truncate(3);
    lib.push(Group::new("Mine", vec![gradients::GradientPreset::new("My Teal", &["#008080", "#ffee00"])]));
    h.state_mut().session.presets.gradients = lib;
    h.run_steps(2);
    let steps = |h: &Harness<'_, PhotocraftApp>| h.state().session.active().unwrap().history.past_len();
    for (name, want) in
        [("Blue 03", vec![(0.0, [0x00, 0xb4, 0xd8]), (1.0, [0x03, 0x04, 0x5e])]), ("My Teal", vec![(0.0, [0x00, 0x80, 0x80]), (1.0, [0xff, 0xee, 0x00])])]
    {
        let before = steps(&h);
        // The picker sits right of its "Preset:" label.
        let preset = label_rect(&h, "Preset:").expect("the preset picker");
        click(&mut h, Pos2::new(preset.right() + 60.0, preset.center().y));
        let item = h.query_by_label(name).unwrap_or_else(|| panic!("`{name}` in the picker")).rect();
        click(&mut h, item.center());
        h.run_steps(4);
        assert_eq!(map_stops(&layer(&h).1), want, "{name}");
        assert_eq!(steps(&h), before + 1, "{name}: one history step");
    }
}

fn import_gradients_from_picker(h: &mut Harness<'_, PhotocraftApp>) {
    let preset = label_rect(h, "Preset:").unwrap();
    click(h, Pos2::new(preset.right() + 60.0, preset.center().y));
    h.get_by_label("Gradient Preset Options").click();
    h.run_steps(2);
    h.get_by_label("Import .GRD").click();
    h.run_steps(3);
}

#[test]
fn gradient_map_picker_imports_grd_without_editing_the_map() {
    use photocraft_psd::grd::{GrdColor, GrdGradient, GrdStop};
    let mut h = app_harness("gradientMap", "rgb", 16);
    h.state_mut().session.presets.gradients.truncate(1);
    let before = layer(&h);
    let history = h.state().session.active().unwrap().history.past_len();
    let tool = h.state().session.presets.gradient.clone();
    let bytes = photocraft_psd::grd::write(&[GrdGradient {
        name: "Imported Teal".into(),
        stops: vec![
            GrdStop { location: 0.0, midpoint: 0.5, color: GrdColor::Rgb([0.0, 0.5, 0.5]) },
            GrdStop { location: 1.0, midpoint: 0.5, color: GrdColor::Rgb([1.0, 1.0, 0.0]) },
        ],
        ..Default::default()
    }]);
    h.state_mut().services.file_dialog = Some(Box::new(move |request, _, reply| {
        assert!(matches!(request, crate::FileDialogRequest::Open { multiple: false, extensions: Some(ref exts), .. } if exts == &["grd"]));
        reply.send(Some(crate::FileDialogAnswer::Contents("My Gradients.GRD".into(), bytes.clone())));
    }));
    import_gradients_from_picker(&mut h);
    assert_eq!(layer(&h), before, "import does not edit the adjustment");
    assert_eq!(h.state().session.active().unwrap().history.past_len(), history);
    assert_eq!(h.state().session.presets.gradient, tool);
    assert!(!h.state().ui.status_error);
    assert!(h.state().ui.status.contains("Imported 1 gradient"));
    let preset = label_rect(&h, "Preset:").unwrap();
    click(&mut h, Pos2::new(preset.right() + 60.0, preset.center().y));
    h.get_by_label("My Gradients");
    h.get_by_label("Imported Teal").click();
    h.run_steps(3);
    assert_eq!(map_stops(&layer(&h).1), vec![(0.0, [0, 128, 128]), (1.0, [255, 255, 0])]);
    assert_eq!(h.state().session.active().unwrap().history.past_len(), history + 1);
    h.state_mut().run("edit.undo", json!({})).unwrap();
    assert_eq!(layer(&h), before);
}

#[test]
fn gradient_map_picker_import_cancel_and_errors_preserve_state() {
    use crate::FileDialogAnswer;
    for answer in [
        None,
        Some(FileDialogAnswer::Contents("broken.grd".into(), b"invalid".to_vec())),
        Some(FileDialogAnswer::Contents("old.grd".into(), b"8BGR\0\x03".to_vec())),
        Some(FileDialogAnswer::Contents(
            "noise.grd".into(),
            photocraft_psd::grd::write(&[photocraft_psd::grd::GrdGradient { noise: true, ..Default::default() }]),
        )),
        Some(FileDialogAnswer::Contents("wrong.abr".into(), Vec::new())),
        Some(FileDialogAnswer::Paths(vec!["/nonexistent/photocraft-gradients.grd".into()])),
        Some(FileDialogAnswer::Unavailable("dialog unavailable".into())),
    ] {
        let mut h = app_harness("gradientMap", "rgb", 8);
        h.state_mut().session.presets.gradients.truncate(1);
        let before = layer(&h);
        let presets = h.state().session.presets.gradients.clone();
        let history = h.state().session.active().unwrap().history.past_len();
        let error = answer.is_some();
        h.state_mut().services.file_dialog = Some(Box::new(move |_, _, reply| reply.send(answer.clone())));
        import_gradients_from_picker(&mut h);
        assert_eq!(layer(&h), before);
        assert_eq!(h.state().session.presets.gradients, presets);
        assert_eq!(h.state().session.active().unwrap().history.past_len(), history);
        assert_eq!(h.state().ui.status_error, error);
        if error {
            assert!(!h.state().ui.status.is_empty());
        }
    }
}

#[test]
fn gradient_map_dialog_import_preserves_pending_settings() {
    let mut h = app_harness("gradientMap", "rgb", 8);
    let id = crate::adjust_dialog::open(h.state_mut(), "image.adjustments.gradientMap").unwrap();
    let fields = h.state_mut().ui.dialog_mut(id).unwrap().fields.clone();
    h.state_mut().services.file_dialog = Some(Box::new(|request, _, reply| {
        assert!(matches!(request, crate::FileDialogRequest::Open { extensions: Some(ref exts), .. } if exts == &["grd"]));
        reply.send(None);
    }));
    h.run_steps(3);
    import_gradients_from_picker(&mut h);
    assert_eq!(h.state_mut().ui.dialog_mut(id).unwrap().fields, fields);
    assert!(!h.state().file_dialog_open());
    assert!(!h.state().ui.status_error);
}

/// Document point (x, y) on screen in the app's first view.
fn doc_on_screen(h: &Harness<'_, PhotocraftApp>, x: f32, y: f32) -> Pos2 {
    let app = h.state();
    let v = &app.ui.views[0];
    let xf = crate::canvas::ViewXform {
        rect: crate::rulers::content_rect(app, app.last_canvas_rect),
        zoom: v.zoom,
        center: v.center,
        flip: app.ui.view.flip_horizontal,
        rotation: v.rotation,
        aspect: app.ui.view.display_aspect(),
    };
    xf.to_screen(x, y)
}

/// UI-217-15 (#3110, #3116): the Properties panel of a Curves or Levels layer has the Set
/// Black/Neutral Gray/White Point eyedroppers. With Set White Point armed, a click on the image
/// makes that pixel white as one history step (sampled below the layer), and Esc disarms the
/// picker.
#[test]
fn curves_properties_white_point_eyedropper_edits_the_layer() {
    properties_white_point_eyedropper_edits_the_layer("curves");
}

#[test]
fn levels_properties_white_point_eyedropper_edits_the_layer() {
    properties_white_point_eyedropper_edits_the_layer("levels");
}

fn properties_white_point_eyedropper_edits_the_layer(kind: &str) {
    let mut h = app_harness(kind, "rgb", 8);
    let (_, before) = layer(&h);
    let steps = h.state().session.active().unwrap().history.past_len();
    h.get_by_label("Set White Point").click();
    h.run_steps(2);
    assert!(crate::layer_picker::armed(h.state()).is_some(), "armed from the Properties panel");
    // The left half is #6a3020: the white point moves to it, so it composites white.
    let at = doc_on_screen(&h, 10.0, 10.0);
    click(&mut h, at);
    let pixel = |h: &mut Harness<'_, PhotocraftApp>| -> Vec<f32> {
        serde_json::from_value(h.state_mut().run("document.pixel", json!({"x": 10, "y": 10})).unwrap()).unwrap()
    };
    let px = pixel(&mut h);
    assert!(px.iter().take(3).all(|v| *v > 0.99), "{kind}: {px:?}");
    assert_ne!(layer(&h).1, before, "{kind}: the layer changed");
    assert_eq!(h.state().session.active().unwrap().history.past_len(), steps + 1, "{kind}: one history step");
    assert!(crate::layer_picker::armed(h.state()).is_some(), "stays armed for another click");
    h.key_press(egui::Key::Escape);
    h.run_steps(2);
    assert!(h.state().ui.layer_picker.is_none(), "Esc disarms");
    h.state_mut().run("edit.undo", json!({})).unwrap();
    assert_eq!(layer(&h).1, before);
}

/// A bare `kind` editor over `params`, as the Properties panel and the dialog host it.
fn editor_harness(kind: &'static str, params: serde_json::Value) -> Harness<'static, serde_json::Value> {
    use crate::adjust_editors::{EditorCx, editor};
    let mut h = Harness::builder().with_size(vec2(300.0, 400.0)).build_ui_state(
        move |ui, values: &mut serde_json::Value| {
            let cx = EditorCx {
                mem: egui::Id::unique("precision"),
                hist: None,
                gray: false,
                swatches: [[0.0; 3], [1.0; 3]],
                gradients: Vec::new(),
                dialog: true,
                dynamic_color_sliders: true,
                isolated_curve: None,
            };
            editor(ui, kind, values, &cx);
        },
        params,
    );
    PhotocraftApp::setup_context(&h.ctx, Default::default());
    h.run_steps(4);
    h
}

/// #9242: the Hue/Saturation spectrum bars stretch with the panel, like the sliders above them.
#[test]
fn hue_saturation_bars_fill_the_panel_width() {
    use crate::adjust_editors::{EditorCx, editor};
    for width in [300.0, 600.0] {
        let mut h = Harness::builder().with_size(vec2(width, 500.0)).build_ui_state(
            |ui, values: &mut serde_json::Value| {
                let cx = EditorCx {
                    mem: egui::Id::unique("hs"),
                    hist: None,
                    gray: false,
                    swatches: [[0.0; 3], [1.0; 3]],
                    gradients: Vec::new(),
                    dialog: false,
                    dynamic_color_sliders: true,
                    isolated_curve: None,
                };
                let avail = ui.available_width();
                editor(ui, "hueSaturation", values, &cx);
                ui.data_mut(|d| d.insert_temp(egui::Id::unique("hs-avail"), avail));
            },
            json!({}),
        );
        PhotocraftApp::setup_context(&h.ctx, Default::default());
        h.run_steps(4);
        let bars = h.ctx.data(|d| d.get_temp::<Rect>(egui::Id::unique("hs").with("hs-bars"))).expect("bars drawn");
        let avail = h.ctx.data(|d| d.get_temp::<f32>(egui::Id::unique("hs-avail"))).unwrap();
        assert!((bars.width() - avail).abs() < 0.5, "{width}: bars {} of {avail}", bars.width());
    }
}

/// Types `text` into the `index`th number field and commits it with Enter.
fn type_into_field(h: &mut Harness<'static, serde_json::Value>, index: usize, text: &str) {
    h.query_all_by_role(egui::accesskit::Role::SpinButton).nth(index).unwrap().click();
    h.run_steps(2);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.run_steps(1);
    h.event(egui::Event::Text(text.into()));
    h.run_steps(1);
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
}

/// The text the `index`th number field shows.
fn field_text(h: &Harness<'static, serde_json::Value>, index: usize) -> String {
    h.query_all_by_role(egui::accesskit::Role::SpinButton).nth(index).unwrap().value().unwrap_or_default()
}

/// #4164: Exposure keeps fractional stops (Photoshop: two decimals) and Offset four decimals,
/// instead of rounding Exposure to whole stops because its range is wider than 10.
#[test]
fn exposure_keeps_fractional_stops_and_offset_four_decimals() {
    let mut h = editor_harness("exposure", json!({}));
    type_into_field(&mut h, 0, "0.5");
    assert_eq!(h.state()["exposure"].as_f64(), Some(0.5), "{}", h.state());
    type_into_field(&mut h, 0, "-1.37");
    assert!((h.state()["exposure"].as_f64().unwrap() + 1.37).abs() < 1e-4, "{}", h.state());
    type_into_field(&mut h, 1, "0.0123");
    assert!((h.state()["offset"].as_f64().unwrap() - 0.0123).abs() < 1e-6, "{}", h.state());
    type_into_field(&mut h, 2, "1.234");
    assert!((h.state()["gamma"].as_f64().unwrap() - 1.23).abs() < 1e-4, "Gamma keeps two decimals: {}", h.state());
    h.run_steps(2);
    assert_eq!(field_text(&h, 0), "-1.37");
    assert_eq!(field_text(&h, 1), "0.0123");
    assert_eq!(field_text(&h, 2), "1.23");
}

/// #4164: the integer rows that share Exposure's slider code stay whole numbers.
#[test]
fn integer_adjustment_rows_stay_whole_numbers() {
    for (kind, key, typed, want) in [
        ("posterize", "levels", "4.6", 5.0),
        ("threshold", "level", "100.4", 100.0),
        ("brightnessContrast", "brightness", "12.7", 13.0),
        ("vibrance", "vibrance", "-20.4", -20.0),
    ] {
        let mut h = editor_harness(kind, json!({}));
        type_into_field(&mut h, 0, typed);
        assert_eq!(h.state()[key].as_f64(), Some(want), "{kind}: {}", h.state());
    }
}

/// #4164: a row's own decimals win over the range rule; rows without them keep it.
#[test]
fn row_values_round_to_their_own_decimals() {
    use crate::adjust_editors::row_value;
    assert_eq!(row_value(0.5, -20.0, 20.0, Some(2)), 0.5);
    assert_eq!(row_value(-1.374, -20.0, 20.0, Some(2)), -1.37);
    assert!((row_value(0.012_34, -0.5, 0.5, Some(4)) - 0.0123).abs() < 1e-7);
    assert_eq!(row_value(4.6, 2.0, 255.0, None), 5.0, "wide ranges stay whole numbers");
    assert!((row_value(1.234, 0.01, 9.99, None) - 1.23).abs() < 1e-6, "small ranges keep two decimals");
    assert!(row_value(f32::NAN, -20.0, 20.0, Some(u32::MAX)).is_nan(), "no panic on odd input");
}

fn photo_filter_color(a: &Adjustment) -> [f32; 3] {
    match a {
        Adjustment::PhotoFilter { color, .. } => *color,
        other => panic!("{other:?}"),
    }
}

/// Click the Photo Filter swatch, wait for its Color Picker and show `hex` in it.
fn photo_filter_picker_shows(h: &mut Harness<'_, PhotocraftApp>, hex: &str) {
    h.get_by_label("Color Picker (Photo Filter)").click();
    h.run_steps(3);
    let d = h.state().ui.dialogs.last().expect("the swatch opens the Color Picker");
    assert_eq!(d.fields["__label"], "Color Picker (Photo Filter)");
    let id = d.id;
    h.state_mut().ui.dialog_mut(id).unwrap().fields.insert("color".into(), json!(hex));
    h.run_steps(3);
}

#[test]
fn photo_filter_colour_picker_previews_live_cancel_restores_and_ok_is_one_step() {
    let mut h = app_harness("photoFilter", "rgb", 8);
    let (id, _) = layer(&h);
    h.state_mut().run("layer.setAdjustment", json!({"layer": id.0, "color": "#336699"})).unwrap();
    h.run_steps(4);
    // The Color: radio enables the swatch.
    h.get_by_label("Color:").click();
    h.run_steps(3);
    let start = photo_filter_color(&layer(&h).1);
    let steps = |h: &Harness<'_, PhotocraftApp>| h.state().session.active().unwrap().history.past_len();
    let before = steps(&h);

    // While the picker is open, the canvas previews its colour; nothing is committed yet.
    photo_filter_picker_shows(&mut h, "#ff0000");
    let live = h.state().live_adjust.clone();
    assert!(matches!(&live, Some((l, v)) if *l == id && v["color"] == "#ff0000"), "live preview while the picker is open: {live:?}");
    assert_eq!(photo_filter_color(&layer(&h).1), start);
    assert_eq!(steps(&h), before);

    // Cancel: back to the opening colour, no history step.
    h.get_by_label("Cancel").click();
    h.run_steps(4);
    assert!(h.state().live_adjust.is_none(), "Cancel ends the preview");
    assert_eq!(photo_filter_color(&layer(&h).1), start);
    assert_eq!(steps(&h), before);

    // OK: the colour, as one history step however long it previewed.
    photo_filter_picker_shows(&mut h, "#00ff80");
    h.state_mut().ui.dialogs.last_mut().unwrap().fields.insert("color".into(), json!("#00ff00"));
    h.run_steps(3);
    assert_eq!(steps(&h), before);
    h.get_by_label("OK").click();
    h.run_steps(4);
    assert!(h.state().live_adjust.is_none());
    assert_eq!(photo_filter_color(&layer(&h).1), [0.0, 1.0, 0.0]);
    assert_eq!(steps(&h), before + 1, "OK: one history step");
}

/// Opening a Gradient Map stop's Color Picker and closing it without a new colour (Cancel, or OK
/// unchanged) edits nothing: a stop at a location finer than the editor's 0.001 grid (a PSD's
/// 30 % is 1229/4096) stays as it is, with no history step.
#[test]
fn gradient_map_stop_picker_closed_without_a_colour_changes_nothing() {
    let mut h = app_harness("gradientMap", "rgb", 16);
    let (id, _) = layer(&h);
    let fine = 1229.0 / 4096.0;
    h.state_mut().run("layer.setAdjustment", json!({"layer": id.0, "stops": [[0, "#000000"], [fine, "#ff0000"], [1, "#ffffff"]]})).unwrap();
    h.run_steps(4);
    let before = layer(&h).1;
    let steps = |h: &Harness<'_, PhotocraftApp>| h.state().session.active().unwrap().history.past_len();
    let history = steps(&h);
    for button in ["Cancel", "OK"] {
        h.get_by_label("Color Picker (Stop Color)").click();
        h.run_steps(3);
        assert_eq!(h.state().ui.dialogs.last().expect("the stop's Color Picker").fields["__label"], "Color Picker (Stop Color)");
        h.get_by_label(button).click();
        h.run_steps(4);
        assert!(h.state().ui.dialogs.is_empty());
        assert_eq!(layer(&h).1, before, "{button}: the stops are untouched");
        assert_eq!(steps(&h), history, "{button}: no history step");
        assert!(h.state().live_adjust.is_none());
    }
}

/// #9252: in a Hue/Saturation layer's Properties, dragging the zones between the range handles
/// moves them as Photoshop does: the fully-on span all four, a fade its own two; one history step
/// per drag.
#[test]
fn hue_saturation_range_zones_drag_their_handles() {
    let mut h = app_harness("hueSaturation", "rgb", 8);
    let (id, _) = layer(&h);
    let mem = crate::adjust_editors::layer_mem(id);
    h.ctx.data_mut(|d| d.insert_temp(mem.with("hs-range"), 3usize)); // Greens: 75/105…135\165
    h.run_steps(3);
    let bars: Rect = h.ctx.data(|d| d.get_temp(mem.with("hs-range-bars"))).expect("the range bars");
    let x = |deg: f32| bars.left() + deg / 360.0 * bars.width();
    let y = bars.center().y;
    let greens = |h: &Harness<'_, PhotocraftApp>| match layer(h).1 {
        Adjustment::HueSaturation { ranges, .. } => ranges[2].bounds,
        other => panic!("{other:?}"),
    };
    let steps = h.state().session.active().unwrap().history.past_len();
    drag(&mut h, Pos2::new(x(120.0), y), Pos2::new(x(156.0), y));
    let b = greens(&h);
    assert!((b[0] - 111.0).abs() <= 1.0 && (b[3] - 201.0).abs() <= 1.0 && (b[1] - b[0] - 30.0).abs() < 0.5, "all four moved ~36°: {b:?}");
    assert_eq!(h.state().session.active().unwrap().history.past_len(), steps + 1, "one step");
    let before = greens(&h);
    drag(&mut h, Pos2::new(x((before[0] + before[1]) / 2.0), y), Pos2::new(x((before[0] + before[1]) / 2.0 - 18.0), y));
    let b = greens(&h);
    assert!((b[0] - (before[0] - 18.0)).abs() <= 1.0 && (b[1] - (before[1] - 18.0)).abs() <= 1.0, "left two moved: {before:?} → {b:?}");
    assert_eq!([b[2], b[3]], [before[2], before[3]], "right two stay");
}

/// CodeRabbit on #9272: a move and release that land in the same frame still move the zone; the
/// release frame applies its position before the drag commits.
#[test]
fn a_zone_drag_released_in_the_frame_it_moves_still_moves() {
    let mut h = app_harness("hueSaturation", "rgb", 8);
    let (id, _) = layer(&h);
    let mem = crate::adjust_editors::layer_mem(id);
    h.ctx.data_mut(|d| d.insert_temp(mem.with("hs-range"), 3usize)); // Greens: 75/105…135\165
    h.run_steps(3);
    let bars: Rect = h.ctx.data(|d| d.get_temp(mem.with("hs-range-bars"))).expect("the range bars");
    let x = |deg: f32| bars.left() + deg / 360.0 * bars.width();
    let y = bars.center().y;
    let steps = h.state().session.active().unwrap().history.past_len();
    let (from, to) = (Pos2::new(x(120.0), y), Pos2::new(x(156.0), y));
    h.hover_at(from);
    h.run_steps(1);
    h.drag_at(from);
    h.run_steps(1);
    // One frame: the pointer moves and lets go (the harness's queued events take a frame each,
    // so both go straight into the next frame's input).
    let events = &mut h.input_mut().events;
    events.push(egui::Event::PointerMoved(to));
    events.push(egui::Event::PointerButton { pos: to, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE });
    h.run_steps(3);
    let b = match layer(&h).1 {
        Adjustment::HueSaturation { ranges, .. } => ranges[2].bounds,
        other => panic!("{other:?}"),
    };
    assert!((b[0] - 111.0).abs() <= 1.0 && (b[3] - 201.0).abs() <= 1.0, "all four moved ~36°: {b:?}");
    assert_eq!(h.state().session.active().unwrap().history.past_len(), steps + 1, "one step");
    // A handle released in the frame it moves lands there too.
    let (from, to) = (Pos2::new(x(b[3]), y), Pos2::new(x(b[3] + 20.0), y));
    h.hover_at(from);
    h.run_steps(1);
    h.drag_at(from);
    h.run_steps(1);
    let events = &mut h.input_mut().events;
    events.push(egui::Event::PointerMoved(to));
    events.push(egui::Event::PointerButton { pos: to, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE });
    h.run_steps(3);
    let after = match layer(&h).1 {
        Adjustment::HueSaturation { ranges, .. } => ranges[2].bounds,
        other => panic!("{other:?}"),
    };
    assert!((after[3] - (b[3] + 20.0)).abs() <= 1.0 && after[..3] == b[..3], "the right handle moved 20°: {b:?} → {after:?}");
    // A release followed by more movement in one frame ends where the button let go.
    let b = after;
    let (from, to) = (Pos2::new(x((b[1] + b[2]) / 2.0), y), Pos2::new(x((b[1] + b[2]) / 2.0 + 10.0), y));
    h.hover_at(from);
    h.run_steps(1);
    h.drag_at(from);
    h.run_steps(1);
    let events = &mut h.input_mut().events;
    events.push(egui::Event::PointerMoved(to));
    events.push(egui::Event::PointerButton { pos: to, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE });
    events.push(egui::Event::PointerMoved(Pos2::new(x(b[3] + 60.0), y)));
    h.run_steps(3);
    let last = match layer(&h).1 {
        Adjustment::HueSaturation { ranges, .. } => ranges[2].bounds,
        other => panic!("{other:?}"),
    };
    assert!(last.iter().zip(&b).all(|(l, b)| (l - (b + 10.0)).abs() <= 1.0), "moved 10° to the release, not past it: {b:?} → {last:?}");
}

/// FredApps review on #9272: a zone drag across the 0/360 seam. Reds' fully-on span sits on both
/// ends of the bar; a drag from its middle third at either end (nearer a handle grabs the handle)
/// moves the whole range intact, and the handles that pass the seam come out at the other end.
#[test]
fn hue_saturation_range_zone_drags_across_the_seam() {
    let mut h = app_harness("hueSaturation", "rgb", 8);
    let (id, _) = layer(&h);
    let mem = crate::adjust_editors::layer_mem(id);
    h.ctx.data_mut(|d| d.insert_temp(mem.with("hs-range"), 1usize)); // Reds: 315/345…15\45
    h.run_steps(3);
    let bars: Rect = h.ctx.data(|d| d.get_temp(mem.with("hs-range-bars"))).expect("the range bars");
    let x = |deg: f32| bars.left() + deg / 360.0 * bars.width();
    let y = bars.center().y;
    let reds = |h: &Harness<'_, PhotocraftApp>| match layer(h).1 {
        Adjustment::HueSaturation { ranges, .. } => ranges[0].bounds,
        other => panic!("{other:?}"),
    };
    let on_circle = |b: [f32; 4]| b.map(|d| d.rem_euclid(360.0).round());
    let widths = |b: [f32; 4]| [b[1] - b[0], b[2] - b[1], b[3] - b[2]];
    let mut steps = h.state().session.active().unwrap().history.past_len();
    // (press, release, where the four handles land on the circle)
    for (from, to, want) in [
        // The span's right end, dragged left: 15\45 cross the seam to the right end.
        (358.0, 298.0, [255.0, 285.0, 315.0, 345.0]),
        // Back right from its middle: 315\345 cross the seam to the left end.
        (300.0, 360.0, [315.0, 345.0, 15.0, 45.0]),
        // The span's left end, dragged right: 315/345 cross the seam to the left end.
        (2.0, 62.0, [15.0, 45.0, 75.0, 105.0]),
    ] {
        let before = reds(&h);
        drag(&mut h, Pos2::new(x(from), y), Pos2::new(x(to), y));
        let b = reds(&h);
        assert!(on_circle(b).iter().zip(&want).all(|(d, w)| (d - w).abs() <= 1.0), "{from}° → {to}°: {before:?} → {b:?}, want {want:?}");
        assert_eq!(widths(b), widths(before), "{from}° → {to}°: the range moves intact");
        steps += 1;
        assert_eq!(h.state().session.active().unwrap().history.past_len(), steps, "{from}° → {to}°: one step");
    }
}

/// Selective Color in Properties: the Colors dropdown's choice lives in editor memory, so
/// switching colour commits nothing, and each colour keeps its own row.
#[test]
fn selective_color_properties_switch_colour_without_committing() {
    let mut h = app_harness("selectiveColor", "rgb", 8);
    let (_, before) = layer(&h);
    let steps = h.state().session.active().unwrap().history.past_len();
    let nudge = |h: &mut Harness<'static, PhotocraftApp>| {
        let r =
            h.query_all_by_role(egui::accesskit::Role::Slider).rfind(|n| n.accesskit_node().label().as_deref() == Some("Cyan")).expect("Cyan slider").rect();
        drag(h, r.center(), Pos2::new(r.right() + 40.0, r.center().y));
    };
    nudge(&mut h);
    let (_, reds) = layer(&h);
    assert_ne!(reds, before);
    assert_eq!(h.state().session.active().unwrap().history.past_len(), steps + 1);
    // Switch to Blues through the Colors dropdown: press it, drag onto the option, release.
    let named = |n: &egui_kittest::Node<'_>| {
        let a = n.accesskit_node();
        a.value().as_deref() == Some("Reds") || a.label().as_deref() == Some("Reds")
    };
    let at = h.query_all_by_role(egui::accesskit::Role::ComboBox).find(|n| named(n)).expect("the Colors dropdown").rect().center();
    h.hover_at(at);
    h.run_steps(1);
    h.drag_at(at);
    h.run_steps(3);
    let blues = h.query_by_label("Blues").expect("the list opened on the press").rect().center();
    for k in 1..=6 {
        h.hover_at(at + (blues - at) * (k as f32 / 6.0));
        h.run_steps(1);
    }
    h.drop_at(blues);
    h.run_steps(4);
    assert!(
        h.query_all_by_role(egui::accesskit::Role::ComboBox).any(|n| {
            let a = n.accesskit_node();
            a.value().as_deref() == Some("Blues") || a.label().as_deref() == Some("Blues")
        }),
        "the dropdown keeps its choice"
    );
    assert_eq!(layer(&h).1, reds, "switching colour changes nothing");
    assert_eq!(h.state().session.active().unwrap().history.past_len(), steps + 1);
    assert!(h.state().live_adjust.is_none());
    nudge(&mut h);
    let (Adjustment::SelectiveColor { adjustments: first, .. }, (_, Adjustment::SelectiveColor { adjustments: now, .. })) = (&reds, layer(&h)) else {
        panic!("Selective Color layer")
    };
    assert_eq!(now[0], first[0], "Reds untouched by editing Blues");
    assert_ne!(now[4], [0.0; 4], "Blues edited");
    assert_eq!(h.state().session.active().unwrap().history.past_len(), steps + 2);
}

/// The Method radios in Properties commit through the real controls; the current one is a no-op.
#[test]
fn selective_color_properties_method_radios_commit() {
    let mut h = app_harness("selectiveColor", "rgb", 8);
    let steps = |h: &Harness<'_, PhotocraftApp>| h.state().session.active().unwrap().history.past_len();
    let start = steps(&h);
    let radio = |h: &mut Harness<'static, PhotocraftApp>, text: &str| {
        let at = h.query_all_by_role_and_label(egui::accesskit::Role::RadioButton, text).last().expect("method radio").rect().center();
        click(h, at);
    };
    radio(&mut h, "Absolute");
    assert!(matches!(layer(&h).1, Adjustment::SelectiveColor { relative: false, .. }));
    assert_eq!(steps(&h), start + 1);
    radio(&mut h, "Absolute");
    assert_eq!(steps(&h), start + 1, "the current method is no step");
    radio(&mut h, "Relative");
    assert!(matches!(layer(&h).1, Adjustment::SelectiveColor { relative: true, .. }));
    assert_eq!(steps(&h), start + 2);
}
