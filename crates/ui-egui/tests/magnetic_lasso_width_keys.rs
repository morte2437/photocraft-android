//! Magnetic Lasso keys: `[` / `]` change the Width by 1 px within 1-256, Shift+`[` jumps to the
//! minimum (1 px) and Shift+`]` to the maximum (256 px), with and without a border being drawn
//! (#11688). The keys go through the real shortcut handler on a bare egui context.

use egui::{Event, Key, Modifiers, RawInput};
use photocraft_ui_egui::canvas::{ToolEvent, tool_event};
use photocraft_ui_egui::{PhotocraftApp, Tool, magnetic_lasso_ui};
use serde_json::json;

fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), photocraft_ui_egui::Services::default());
    app.run("file.new", json!({"width": 200, "height": 150})).unwrap();
    app.sync_views();
    app.ui.tool = Tool::MagneticLasso;
    app
}

/// A border is started with a click, so it is being drawn while the keys are pressed.
fn start_border(app: &mut PhotocraftApp) {
    tool_event(app, ToolEvent::Down { x: 52.0, y: 37.0, pressure: 1.0 }, Modifiers::NONE);
    tool_event(app, ToolEvent::Up { x: 52.0, y: 37.0 }, Modifiers::NONE);
    assert!(magnetic_lasso_ui::replaces_selection(app), "a border is being drawn");
}

/// Presses `key` with `modifiers` for one frame and returns whether the Magnetic Lasso took it.
fn press(app: &mut PhotocraftApp, key: Key, modifiers: Modifiers) -> bool {
    let ctx = egui::Context::default();
    let mut taken = false;
    let events = vec![Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers }];
    let mut out = ctx.run_ui(RawInput { events, ..Default::default() }, |ui| taken = magnetic_lasso_ui::keys(app, ui.ctx()));
    out.textures_delta.clear();
    taken
}

fn width(app: &PhotocraftApp) -> f32 {
    app.ui.tool_options.magnetic_width
}

fn with_and_without_border(check: impl Fn(&mut PhotocraftApp)) {
    let mut plain = app();
    check(&mut plain);
    let mut drawing = app();
    start_border(&mut drawing);
    check(&mut drawing);
    assert!(magnetic_lasso_ui::replaces_selection(&drawing), "the border being drawn stays as it is");
}

#[test]
fn shift_open_bracket_sets_the_width_to_one_from_any_width() {
    with_and_without_border(|app| {
        for from in [10.0, 2.0, 1.0, 100.0, 256.0] {
            app.ui.tool_options.magnetic_width = from;
            assert!(press(app, Key::OpenBracket, Modifiers::SHIFT));
            assert_eq!(width(app), 1.0, "Shift+[ from {from}");
        }
    });
}

#[test]
fn shift_close_bracket_sets_the_width_to_256_from_any_width() {
    with_and_without_border(|app| {
        for from in [10.0, 255.0, 256.0, 100.0, 1.0] {
            app.ui.tool_options.magnetic_width = from;
            assert!(press(app, Key::CloseBracket, Modifiers::SHIFT));
            assert_eq!(width(app), 256.0, "Shift+] from {from}");
        }
    });
}

#[test]
fn plain_brackets_still_step_by_one_and_stop_at_the_limits() {
    with_and_without_border(|app| {
        app.ui.tool_options.magnetic_width = 10.0;
        assert!(press(app, Key::OpenBracket, Modifiers::NONE));
        assert_eq!(width(app), 9.0);
        assert!(press(app, Key::CloseBracket, Modifiers::NONE));
        assert!(press(app, Key::CloseBracket, Modifiers::NONE));
        assert_eq!(width(app), 11.0);
        app.ui.tool_options.magnetic_width = 1.0;
        press(app, Key::OpenBracket, Modifiers::NONE);
        assert_eq!(width(app), 1.0, "[ stops at 1");
        app.ui.tool_options.magnetic_width = 256.0;
        press(app, Key::CloseBracket, Modifiers::NONE);
        assert_eq!(width(app), 256.0, "] stops at 256");
    });
}

#[test]
fn shifted_curly_bracket_keys_set_the_limits() {
    with_and_without_border(|app| {
        app.ui.tool_options.magnetic_width = 10.0;
        assert!(press(app, Key::OpenCurlyBracket, Modifiers::SHIFT));
        assert_eq!(width(app), 1.0, "Shift+{{");
        assert!(press(app, Key::CloseCurlyBracket, Modifiers::SHIFT));
        assert_eq!(width(app), 256.0, "Shift+}}");
    });
}

#[test]
fn a_bare_curly_bracket_key_is_left_alone() {
    with_and_without_border(|app| {
        app.ui.tool_options.magnetic_width = 10.0;
        assert!(!press(app, Key::OpenCurlyBracket, Modifiers::NONE), "a bare {{ is not taken");
        assert!(!press(app, Key::CloseCurlyBracket, Modifiers::NONE), "a bare }} is not taken");
        assert_eq!(width(app), 10.0);
    });
}
