use super::*;
use egui::{Key, Modifiers};

fn opened() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.session.execute("file.new", json!({"width": 160, "height": 100, "depth": 8})).unwrap();
    app.session.execute("select.rect", json!({"x": 0, "y": 0, "width": 20, "height": 20})).unwrap();
    app.session.prefs.edit(|p| {
        p.dialogs.remove(REMEMBERED);
    });
    app.sync_views();
    open(&mut app).unwrap();
    app
}

fn ws(app: &PhotocraftApp) -> &SelectMaskWorkspace {
    app.distort.select_mask.as_ref().unwrap()
}

fn ui_selected(app: &mut PhotocraftApp) -> u64 {
    control(app, &json!({"waitPreview": true})).unwrap()["selectedProxyPixels"].as_u64().unwrap()
}

fn press(app: &mut PhotocraftApp, ctx: &egui::Context, key: Key, modifiers: Modifiers) {
    let ev = egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers };
    let input = egui::RawInput { events: vec![ev], ..Default::default() };
    ctx.run_ui(input, |ui| keys(app, ui.ctx())).textures_delta.clear();
}

#[test]
fn e_switches_between_add_and_subtract_and_back() {
    let mut app = opened();
    let ctx = egui::Context::default();
    assert!(ws(&app).add);
    press(&mut app, &ctx, Key::E, Modifiers::NONE);
    assert!(!ws(&app).add);
    assert_eq!(ws(&app).describe()["mode"].as_str(), Some("subtract"));
    press(&mut app, &ctx, Key::E, Modifiers::NONE);
    assert!(ws(&app).add);
    assert_eq!(ws(&app).describe()["mode"].as_str(), Some("add"));
}

#[test]
fn e_works_with_every_editing_tool_and_keeps_the_tool() {
    let ctx = egui::Context::default();
    for tool in [SmTool::Quick, SmTool::RefineEdge, SmTool::Brush, SmTool::Object, SmTool::Lasso, SmTool::PolygonalLasso] {
        let mut app = opened();
        app.distort.select_mask.as_mut().unwrap().pick(tool);
        press(&mut app, &ctx, Key::E, Modifiers::NONE);
        assert!(!ws(&app).add, "{tool:?}");
        assert_eq!(ws(&app).tool, tool);
    }
}

#[test]
fn e_does_nothing_with_hand_or_zoom() {
    let ctx = egui::Context::default();
    for tool in [SmTool::Hand, SmTool::Zoom] {
        let mut app = opened();
        app.distort.select_mask.as_mut().unwrap().pick(tool);
        press(&mut app, &ctx, Key::E, Modifiers::NONE);
        assert!(ws(&app).add, "{tool:?}");
    }
}

#[test]
fn e_with_command_alt_or_shift_does_nothing_new() {
    let ctx = egui::Context::default();
    for m in [Modifiers::COMMAND, Modifiers::ALT, Modifiers::SHIFT, Modifiers::CTRL] {
        let mut app = opened();
        press(&mut app, &ctx, Key::E, m);
        assert!(ws(&app).add, "{m:?}");
    }
}

#[test]
fn e_is_ignored_while_a_popup_has_the_keys() {
    let mut app = opened();
    let ctx = egui::Context::default();
    ctx.run_ui(egui::RawInput::default(), |ui| {
        egui::Popup::open_id(ui.ctx(), egui::Id::unique("sm-test-popup"));
    })
    .textures_delta
    .clear();
    assert!(egui::Popup::is_any_open(&ctx));
    press(&mut app, &ctx, Key::E, Modifiers::NONE);
    assert!(ws(&app).add);
}

#[test]
fn e_chooses_the_mode_alt_swaps_while_held() {
    let mut app = opened();
    let ctx = egui::Context::default();
    press(&mut app, &ctx, Key::E, Modifiers::NONE);
    assert!(!ws(&app).add);
    // Alt held on top of subtract: the stroke adds, so the pixel under it becomes selected.
    let before = ui_selected(&mut app);
    let stroke = json!({"tool": "brush", "size": 10, "hardness": 50, "stroke": {"points": [[100.0, 10.0], [100.0, 90.0]], "alt": true}});
    control(&mut app, &stroke).unwrap();
    assert!(!ws(&app).add, "Alt does not change the chosen mode");
    assert!(ui_selected(&mut app) > before, "Alt over subtract adds");
}

#[test]
fn a_polygon_in_progress_keeps_its_starting_mode() {
    let mut app = opened();
    let ctx = egui::Context::default();
    app.distort.select_mask.as_mut().unwrap().pick(SmTool::PolygonalLasso);
    app.distort.select_mask.as_mut().unwrap().poly = Some((true, vec![[10.0, 10.0], [50.0, 10.0], [50.0, 50.0]]));
    press(&mut app, &ctx, Key::E, Modifiers::NONE);
    assert!(!ws(&app).add);
    assert_eq!(ws(&app).poly.as_ref().map(|(a, _)| *a), Some(true));
    assert!(enter(&mut app), "Enter closes the polygon");
    let edit = ws(&app).edits.last().expect("the closed polygon is recorded");
    assert_eq!(edit["tool"].as_str(), Some("lasso"));
    assert_eq!(edit["mode"].as_str(), Some("add"), "the mode the polygon started with");
    assert!(!ws(&app).add, "the workspace mode is still the one E chose");
}

fn e_event() -> egui::RawInput {
    let ev = egui::Event::Key { key: Key::E, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE };
    egui::RawInput { events: vec![ev], ..Default::default() }
}

/// Through the shortcut handler, as the app runs it: a focused text field keeps the E.
#[test]
fn e_through_handle_is_ignored_while_a_text_field_has_focus() {
    let mut app = opened();
    let ctx = egui::Context::default();
    let mut text = String::new();
    ctx.run_ui(Default::default(), |ui| {
        ui.text_edit_singleline(&mut text).request_focus();
    })
    .textures_delta
    .clear();
    ctx.run_ui(e_event(), |ui| {
        assert!(ui.ctx().text_edit_focused());
        crate::shortcuts::handle(&mut app, ui.ctx());
        ui.text_edit_singleline(&mut text);
    })
    .textures_delta
    .clear();
    assert!(ws(&app).add);
}

/// The same path with no text field focused: E flips the mode.
#[test]
fn e_through_handle_flips_the_mode_without_a_focused_text_field() {
    let mut app = opened();
    let ctx = egui::Context::default();
    ctx.run_ui(e_event(), |ui| {
        assert!(!ui.ctx().text_edit_focused());
        crate::shortcuts::handle(&mut app, ui.ctx());
    })
    .textures_delta
    .clear();
    assert!(!ws(&app).add);
}
