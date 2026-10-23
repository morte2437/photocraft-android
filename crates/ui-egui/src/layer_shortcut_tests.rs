//! Photoshop's layer and selection keys reach their commands: ⌘, toggles visibility, ⌘\ targets
//! the mask, / toggles Lock Transparent Pixels, ⌘⌥2 loads luminosity, ⌘⌥⌫ fills from history.

use egui::{Key, Modifiers};
use serde_json::json;

use crate::PhotocraftApp;

fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 20, "height": 10})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app
}

fn press(app: &mut PhotocraftApp, key: Key, modifiers: Modifiers) {
    let ctx = egui::Context::default();
    let raw = egui::RawInput {
        events: vec![egui::Event::ModifiersChanged(modifiers), egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers }],
        ..Default::default()
    };
    ctx.begin_pass(raw);
    crate::shortcuts::handle(app, &ctx);
    ctx.end_pass().textures_delta.clear();
}

fn active(app: &PhotocraftApp) -> &photocraft_doc::Layer {
    let st = app.session.active().unwrap();
    st.doc.layer(st.active_layer.unwrap()).unwrap()
}

#[test]
fn command_comma_toggles_the_layers_visibility() {
    let mut app = app();
    press(&mut app, Key::Comma, Modifiers::COMMAND);
    assert!(!active(&app).visible);
    press(&mut app, Key::Comma, Modifiers::COMMAND);
    assert!(active(&app).visible);
}

#[test]
fn slash_toggles_lock_transparent_pixels() {
    let mut app = app();
    press(&mut app, Key::Slash, Modifiers::NONE);
    assert!(active(&app).locks.transparency);
    press(&mut app, Key::Slash, Modifiers::NONE);
    assert!(!active(&app).locks.transparency);
}

#[test]
fn command_backslash_targets_the_layer_mask() {
    let mut app = app();
    app.run("layer.layerMask.revealAll", json!({})).unwrap();
    app.run("channel.target.composite", json!({})).unwrap();
    press(&mut app, Key::Backslash, Modifiers::COMMAND);
    assert_eq!(app.session.active().unwrap().channel_view.target, photocraft_engine::channel_cmds::ChannelTarget::LayerMask);
    assert!(app.ui.mask_target && app.layer_mask_targeted(), "the Layers panel targets the mask too");
    // Ctrl+2 goes back to the layer's pixels, in the Layers panel and for the tools.
    press(&mut app, Key::Num2, Modifiers::COMMAND);
    assert_eq!(app.session.active().unwrap().channel_view.target, photocraft_engine::channel_cmds::ChannelTarget::Composite);
    assert!(!app.ui.mask_target && !app.layer_mask_targeted());
    // The same after targeting the mask by its thumbnail (the panel's own flag).
    app.ui.mask_target = true;
    app.sync_mask_targets();
    press(&mut app, Key::Num2, Modifiers::COMMAND);
    assert!(!app.ui.mask_target && !app.layer_mask_targeted());
}

#[test]
fn command_alt_2_loads_luminosity() {
    let mut app = app();
    press(&mut app, Key::Num2, Modifiers::COMMAND | Modifiers::ALT);
    assert!(app.session.active().unwrap().doc.selection.is_some(), "the white image's luminosity selects it all");
}

#[test]
fn command_alt_backspace_fills_from_history() {
    // The Background exists in the oldest state, so its pixels come back from there.
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 20, "height": 10})).unwrap();
    app.run("tools.setColors", json!({"foreground": "#ff0000"})).unwrap();
    app.run("edit.fillForeground", json!({})).unwrap();
    assert_eq!(active(&app).surface().unwrap().pixel(3, 3)[1], 0.0, "red");
    press(&mut app, Key::Backspace, Modifiers::COMMAND | Modifiers::ALT);
    assert_eq!(active(&app).surface().unwrap().pixel(3, 3)[1], 1.0, "white again");
}

/// ⌥-click a layer's thumbnail: select it and fit its content on screen.
#[test]
fn alt_click_on_a_layer_thumbnail_fits_the_layer() {
    use egui::{Event, PointerButton, pos2, vec2};
    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width": 400, "height": 300})).unwrap();
    let small = s.execute("layer.new.layer", json!({"name": "Small"})).unwrap()["layer"].as_u64().unwrap();
    s.execute("select.rect", json!({"x": 300, "y": 200, "width": 20, "height": 10})).unwrap();
    s.execute("edit.fill", json!({"contents": "black"})).unwrap();
    s.execute("select.deselect", json!({})).unwrap();
    s.execute("layer.new.layer", json!({"name": "Top"})).unwrap();
    let mut h = egui_kittest::Harness::builder().with_size(vec2(1440.0, 900.0)).with_step_dt(1.0 / 60.0).with_max_steps(64).build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        PhotocraftApp::new(s, crate::Services::default())
    });
    let ctx = h.ctx.clone();
    let (r, _) = crate::control::ControlRequest::new("ui.set", json!({"dock": {"collapsed": ["color", "properties", "history", "navigator"]}}));
    crate::control::handle(h.state_mut(), &ctx, &r);
    h.run_steps(8);
    let before = h.state().ui.views[0].clone();
    let row = crate::layer_row_ui::recorded(&h.ctx).into_iter().find(|r| r.layer == small).unwrap().row;
    let at = pos2(row.left() + 46.0, row.center().y);
    h.hover_at(at);
    h.event(Event::ModifiersChanged(Modifiers::ALT));
    h.run_steps(1);
    h.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::ALT });
    h.run_steps(1);
    h.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::ALT });
    h.run_steps(1);
    h.event(Event::ModifiersChanged(Modifiers::NONE));
    h.run_steps(3);
    let app = h.state();
    assert_eq!(app.session.active().unwrap().active_layer, Some(photocraft_doc::LayerId(small)));
    let v = &app.ui.views[0];
    assert_eq!(v.center, [310.0, 205.0], "centred on the layer's content");
    assert!(v.zoom > before.zoom * 4.0, "zoomed in on 20 × 10 px: {} -> {}", before.zoom, v.zoom);
}

/// ⌘-click a layer's thumbnail loads its transparency as a selection without changing the
/// selected layer; on the Background, which has no transparency, it is a plain click, and a hidden
/// layer still loads (measured on Photoshop 25.4, 2026-10-17).
#[test]
fn command_click_on_a_thumbnail_loads_transparency_except_on_the_background() {
    use egui::{Event, PointerButton, pos2, vec2};
    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width": 400, "height": 300})).unwrap();
    let background = s.active().unwrap().active_layer.unwrap().0;
    let shape = s.execute("layer.new.layer", json!({"name": "Shape"})).unwrap()["layer"].as_u64().unwrap();
    s.execute("select.rect", json!({"x": 100, "y": 50, "width": 120, "height": 100})).unwrap();
    s.execute("edit.fill", json!({"contents": "black"})).unwrap();
    s.execute("select.deselect", json!({})).unwrap();
    s.execute("layer.new.layer", json!({"name": "Top"})).unwrap();
    let mut h = egui_kittest::Harness::builder().with_size(vec2(1440.0, 900.0)).with_step_dt(1.0 / 60.0).with_max_steps(64).build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        PhotocraftApp::new(s, crate::Services::default())
    });
    let ctx = h.ctx.clone();
    let (r, _) = crate::control::ControlRequest::new("ui.set", json!({"dock": {"collapsed": ["color", "properties", "history", "navigator"]}}));
    crate::control::handle(h.state_mut(), &ctx, &r);
    h.run_steps(8);
    let click = |h: &mut egui_kittest::Harness<'_, PhotocraftApp>, layer: u64, modifiers: Modifiers| {
        let row = crate::layer_row_ui::recorded(&h.ctx).into_iter().find(|r| r.layer == layer).unwrap().row;
        let at = pos2(row.left() + 46.0, row.center().y);
        h.hover_at(at);
        h.event(Event::ModifiersChanged(modifiers));
        h.run_steps(1);
        h.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: true, modifiers });
        h.run_steps(1);
        h.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: false, modifiers });
        h.run_steps(1);
        h.event(Event::ModifiersChanged(Modifiers::NONE));
        h.run_steps(3);
    };
    let bounds = |h: &egui_kittest::Harness<'_, PhotocraftApp>| {
        let st = h.state().session.active().unwrap();
        st.doc.selection.as_ref().map(|s| s.content_bounds())
    };
    let top = h.state().session.active().unwrap().active_layer.unwrap();
    let selected = |h: &egui_kittest::Harness<'_, PhotocraftApp>| h.state().session.active().unwrap().selected_layers();
    click(&mut h, shape, Modifiers::COMMAND);
    assert_eq!(bounds(&h), Some(photocraft_geom::Rect::new(100, 50, 220, 150)), "the layer's pixels are selected");
    assert_eq!(h.state().session.active().unwrap().active_layer, Some(top), "the selected layer stays");
    assert_eq!(selected(&h), vec![top]);
    click(&mut h, background, Modifiers::COMMAND);
    assert_eq!(
        bounds(&h),
        Some(photocraft_geom::Rect::new(100, 50, 220, 150)),
        "the Background has no transparency to load: a plain click, which leaves the selection and selects the row"
    );
    assert_eq!(h.state().session.active().unwrap().active_layer, Some(photocraft_doc::LayerId(background)));
    assert_eq!(selected(&h), vec![photocraft_doc::LayerId(background)], "the Background alone, not toggled into the set");
    // Already in a multi-selection: the ⌘-click still leaves it the only selected layer.
    h.state_mut().run("layer.select", json!({"layer": top.0})).unwrap();
    h.state_mut().run("layer.select", json!({"layer": background, "mode": "add"})).unwrap();
    h.run_steps(2);
    assert_eq!(selected(&h).len(), 2);
    click(&mut h, background, Modifiers::COMMAND);
    assert_eq!(selected(&h), vec![photocraft_doc::LayerId(background)], "replaced, not removed from the set");
    assert_eq!(h.state().session.active().unwrap().active_layer, Some(photocraft_doc::LayerId(background)));
    h.state_mut().run("layer.setProps", json!({"layer": shape, "visible": false})).unwrap();
    click(&mut h, shape, Modifiers::COMMAND);
    assert_eq!(bounds(&h), Some(photocraft_geom::Rect::new(100, 50, 220, 150)), "a hidden layer's transparency loads too");
}

/// #6827: a custom binding on PageUp/PageDown fires (it was saved as `Pageup`, which matched no
/// key), and one saved that way by an earlier version still parses.
#[test]
fn custom_page_up_and_page_down_bindings_fire() {
    for (binding, key, modifiers) in [
        ("Shift+PageUp", Key::PageUp, Modifiers::SHIFT),
        ("Cmd+Shift+PageUp", Key::PageUp, Modifiers::COMMAND | Modifiers::SHIFT),
        ("Cmd+Alt+PageDown", Key::PageDown, Modifiers::COMMAND | Modifiers::ALT),
    ] {
        let mut app = app();
        app.run("edit.fill", json!({"contents": "black"})).unwrap();
        app.run("edit.keyboardShortcuts", json!({"set": {"edit.transform.flipHorizontal": binding}, "removeConflicts": true})).unwrap();
        let before = app.session.journal.len();
        press(&mut app, key, modifiers);
        let ran: Vec<&str> = app.session.journal[before..].iter().map(|(id, _)| id.as_str()).collect();
        assert_eq!(ran, ["edit.transform.flipHorizontal"], "{binding}");
    }
    assert_eq!(crate::shortcuts::parse("Shift+Pageup"), Some(egui::KeyboardShortcut::new(Modifiers::SHIFT, Key::PageUp)));
}

/// A click on the empty space below the last row of the Layers panel deselects every layer, as
/// in Photoshop (#8100); a click on a row still selects it.
#[test]
fn a_click_below_the_layers_deselects_them() {
    use egui::{Event, PointerButton, pos2, vec2};
    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width": 64, "height": 48})).unwrap();
    let top = s.execute("layer.new.layer", json!({"name": "Top"})).unwrap()["layer"].as_u64().unwrap();
    let mut h = egui_kittest::Harness::builder().with_size(vec2(1440.0, 900.0)).with_step_dt(1.0 / 60.0).with_max_steps(64).build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        PhotocraftApp::new(s, crate::Services::default())
    });
    let ctx = h.ctx.clone();
    let (r, _) = crate::control::ControlRequest::new("ui.set", json!({"dock": {"collapsed": ["color", "properties", "history", "navigator"]}}));
    crate::control::handle(h.state_mut(), &ctx, &r);
    h.run_steps(8);
    let click = |h: &mut egui_kittest::Harness<'_, PhotocraftApp>, at: egui::Pos2| {
        h.hover_at(at);
        h.run_steps(1);
        h.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
        h.run_steps(1);
        h.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
        h.run_steps(3);
    };
    let selected = |h: &egui_kittest::Harness<'_, PhotocraftApp>| h.state().session.active().unwrap().selected_layers();
    assert_eq!(selected(&h), vec![photocraft_doc::LayerId(top)]);
    let rows = crate::layer_row_ui::recorded(&h.ctx);
    let bottom = rows.iter().map(|r| r.row.bottom()).fold(f32::MIN, f32::max);
    let row = rows.iter().find(|r| r.layer == top).unwrap().row;
    click(&mut h, pos2(row.center().x, bottom + 40.0));
    assert!(selected(&h).is_empty(), "the empty space deselects every layer");
    assert_eq!(h.state().session.active().unwrap().active_layer, None);
    // Past the double-click window, on the row as it is now drawn.
    h.run_steps(40);
    let row = crate::layer_row_ui::recorded(&h.ctx).into_iter().find(|r| r.layer == top).unwrap().row;
    click(&mut h, pos2(row.left() + 46.0, row.center().y));
    assert_eq!(selected(&h), vec![photocraft_doc::LayerId(top)], "a row still selects");
}

/// Over a thumbnail ⌘-click would load, the pointer is a hand with the selection mode's badge:
/// none for a new selection, + with ⇧, − with ⌥, × with ⇧⌥ (#7824). Without ⌘, and over the
/// Background (which has nothing to load), it stays the plain pointer.
#[test]
fn command_over_a_thumbnail_shows_the_load_cursor() {
    use egui::{CursorIcon, Event, pos2, vec2};

    use crate::tool_feedback::Badge;
    assert_eq!(crate::panels::thumb_load_badge(Modifiers::COMMAND), None);
    assert_eq!(crate::panels::thumb_load_badge(Modifiers::COMMAND | Modifiers::SHIFT), Some(Badge::Add));
    assert_eq!(crate::panels::thumb_load_badge(Modifiers::COMMAND | Modifiers::ALT), Some(Badge::Subtract));
    assert_eq!(crate::panels::thumb_load_badge(Modifiers::COMMAND | Modifiers::SHIFT | Modifiers::ALT), Some(Badge::Intersect));

    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width": 400, "height": 300})).unwrap();
    let background = s.active().unwrap().active_layer.unwrap().0;
    let shape = s.execute("layer.new.layer", json!({"name": "Shape"})).unwrap()["layer"].as_u64().unwrap();
    let mut h = egui_kittest::Harness::builder().with_size(vec2(1440.0, 900.0)).with_step_dt(1.0 / 60.0).with_max_steps(64).build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        PhotocraftApp::new(s, crate::Services::default())
    });
    let ctx = h.ctx.clone();
    let (r, _) = crate::control::ControlRequest::new("ui.set", json!({"dock": {"collapsed": ["color", "properties", "history", "navigator"]}}));
    crate::control::handle(h.state_mut(), &ctx, &r);
    h.run_steps(8);
    let cursor = |h: &mut egui_kittest::Harness<'_, PhotocraftApp>, layer: u64, modifiers: Modifiers| {
        let row = crate::layer_row_ui::recorded(&h.ctx).into_iter().find(|r| r.layer == layer).unwrap().row;
        h.hover_at(pos2(row.left() + 46.0, row.center().y));
        h.event(Event::ModifiersChanged(modifiers));
        h.run_steps(2);
        let icon = h.output().platform_output.cursor_icon;
        h.event(Event::ModifiersChanged(Modifiers::NONE));
        h.run_steps(1);
        icon
    };
    for m in [Modifiers::COMMAND, Modifiers::COMMAND | Modifiers::SHIFT, Modifiers::COMMAND | Modifiers::ALT] {
        assert_eq!(cursor(&mut h, shape, m), CursorIcon::PointingHand, "{m:?}");
    }
    assert_ne!(cursor(&mut h, shape, Modifiers::NONE), CursorIcon::PointingHand, "a plain click selects the layer");
    assert_ne!(cursor(&mut h, background, Modifiers::COMMAND), CursorIcon::PointingHand, "the Background has nothing to load");
}
