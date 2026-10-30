//! #292: Edit › Keyboard Shortcuts records the key pressed with the modifiers, not the
//! modifier key itself (egui reports Ctrl, Shift, Alt and ⌘ as key presses too), so Ctrl+F and
//! any other non-system combination can be bound; a shortcut already in use is reported.

use egui::{Event, Key, Modifiers};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::{Value, json};

use crate::PhotocraftApp;

fn key(h: &mut Harness<'_, PhotocraftApp>, key: Key, pressed: bool, modifiers: Modifiers) {
    h.event(Event::Key { key, physical_key: None, pressed, repeat: false, modifiers });
}

fn input_state(events: Vec<Event>, modifiers: Modifiers) -> egui::InputState {
    let ctx = egui::Context::default();
    ctx.begin_pass(egui::RawInput { events, ..Default::default() });
    let state = ctx.input(|i| i.clone());
    ctx.end_pass().textures_delta.clear();
    let mut state = state;
    state.modifiers = modifiers;
    state
}

/// The Keyboard Shortcuts dialog with `command` selected, filtered to it.
fn dialog(ppp: f32, command: &str, filter: &str) -> (Harness<'static, PhotocraftApp>, u64) {
    dialog_with_renderer(ppp, command, filter, false)
}

fn dialog_with_renderer(ppp: f32, command: &str, filter: &str, gpu: bool) -> (Harness<'static, PhotocraftApp>, u64) {
    let builder = Harness::builder().with_size(egui::vec2(1100.0, 760.0)).with_pixels_per_point(ppp).with_max_steps(64);
    let builder = if gpu { builder.wgpu() } else { builder };
    let mut h = builder.build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default())
    });
    let ctx = h.ctx.clone();
    let id = crate::menus::invoke(h.state_mut(), &ctx, "edit.keyboardShortcuts", json!({})).unwrap()["dialog"].as_u64().unwrap();
    let f = &mut h.state_mut().ui.dialog_mut(id).unwrap().fields;
    f.insert("filter".into(), json!(filter));
    f.insert("selected".into(), json!(command));
    h.run_steps(4);
    (h, id)
}

fn field(h: &Harness<'_, PhotocraftApp>, id: u64, name: &str) -> Value {
    h.state().ui.dialogs.iter().find(|d| d.id == id).unwrap().fields.get(name).cloned().unwrap_or(Value::Null)
}

#[test]
fn toolbar_shortcut_can_be_captured_and_restored_on_restart() {
    let (mut h, id) = dialog(1.0, "tools.select.pencil", "Pencil Tool");
    h.get_by_role_and_label(egui::accesskit::Role::Button, "B").click();
    h.run_steps(2);
    h.key_press(Key::F6);
    h.run_steps(2);
    assert_eq!(field(&h, id, "overrides")["tools.select.pencil"], json!("F6"));
    crate::dialogs::confirm(h.state_mut(), id).unwrap();
    h.run_steps(2);
    h.key_press(Key::F6);
    h.run_steps(2);
    assert_eq!(h.state().ui.tool, crate::state::Tool::Pencil);

    let saved = h.state().session.prefs_to_json();
    let mut session = photocraft_engine::Session::new();
    session.load_prefs_json(&saved).unwrap();
    let mut restarted = PhotocraftApp::new(session, crate::Services::default());
    let ctx = egui::Context::default();
    ctx.begin_pass(egui::RawInput {
        events: vec![Event::Key { key: Key::F6, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE }],
        ..Default::default()
    });
    crate::shortcuts::handle(&mut restarted, &ctx);
    ctx.end_pass().textures_delta.clear();
    assert_eq!(restarted.ui.tool, crate::state::Tool::Pencil);
}

#[test]
fn toolbar_shortcut_overrides_remove_old_keys_and_skip_removed_group_members() {
    use crate::state::Tool;
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let ctx = egui::Context::default();
    let press = |app: &mut PhotocraftApp, key, modifiers| {
        ctx.begin_pass(egui::RawInput {
            // A tap: press and release, so the next press isn't a held key's repeat.
            events: vec![
                Event::ModifiersChanged(modifiers),
                Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers },
                Event::Key { key, physical_key: None, pressed: false, repeat: false, modifiers },
            ],
            ..Default::default()
        });
        crate::shortcuts::handle(app, &ctx);
        ctx.end_pass().textures_delta.clear();
    };
    app.run("edit.keyboardShortcuts", json!({"set": {"tools.select.move": "F6", "tools.select.pencil": ""}, "allowUnknown": true})).unwrap();
    app.ui.tool = Tool::Zoom;
    press(&mut app, Key::V, Modifiers::NONE);
    assert_eq!(app.ui.tool, Tool::Artboard, "the old key only selects unchanged tools");
    press(&mut app, Key::F6, Modifiers::NONE);
    assert_eq!(app.ui.tool, Tool::Move);

    app.ui.tool = Tool::Pencil;
    press(&mut app, Key::V, Modifiers::NONE);
    press(&mut app, Key::B, Modifiers::NONE);
    assert_eq!(app.ui.tool, Tool::Brush, "a removed tool cannot return through group memory");
    press(&mut app, Key::B, Modifiers::SHIFT);
    assert_eq!(app.ui.tool, Tool::MixerBrush, "Shift cycling skips a removed tool");

    app.run("edit.keyboardShortcuts", json!({"reset": true})).unwrap();
    app.ui.tool = Tool::Brush;
    press(&mut app, Key::B, Modifiers::SHIFT);
    assert_eq!(app.ui.tool, Tool::Pencil, "reset restores the default group");
    app.session.edit_prefs(|p| p.tools.use_shift_key_for_tool_switch = false);
    press(&mut app, Key::B, Modifiers::NONE);
    assert_eq!(app.ui.tool, Tool::MixerBrush, "plain-key cycling still follows the preference");
}

#[test]
fn toolbar_shortcuts_participate_in_conflict_resolution() {
    let (mut h, id) = dialog(1.0, "edit.search", "search…");
    capture(&mut h, &crate::shortcuts::pretty("Cmd+F"), Key::ControlLeft, Modifiers::COMMAND, Key::B);
    // Remove Ctrl from the captured binding: a plain B belongs to the Brush group by default.
    h.state_mut().ui.dialog_mut(id).unwrap().fields["overrides"]["edit.search"] = json!("B");
    crate::dialogs::confirm(h.state_mut(), id).unwrap();
    for tool in ["brush", "pencil", "mixerBrush", "colorReplacement"] {
        assert_eq!(h.state().session.prefs().shortcuts.get(&format!("tools.select.{tool}")).map(String::as_str), Some(""));
    }
    let ctx = h.ctx.clone();
    h.state_mut().ui.tool = crate::state::Tool::Move;
    key(&mut h, Key::B, true, Modifiers::NONE);
    h.run_steps(2);
    assert!(h.state().ui.palette_open, "B now runs Search");
    assert_eq!(h.state().ui.tool, crate::state::Tool::Move);
    assert_eq!(crate::shortcut_dispatch::take_log(&ctx).last().map(|i| i.0.as_str()), Some("edit.search"));
}

/// Click the selected command's shortcut button, then press `modifier` and `k` like a keyboard:
/// the modifier key goes down first (its own key event), then the key with the modifier held.
fn capture(h: &mut Harness<'_, PhotocraftApp>, button: &str, modifier: Key, held: Modifiers, k: Key) {
    h.get_by_label(button).click();
    h.run_steps(2);
    key(h, modifier, true, held);
    h.run_steps(1);
    key(h, k, true, held);
    h.run_steps(1);
    key(h, k, false, held);
    key(h, modifier, false, Modifiers::NONE);
    h.run_steps(2);
}

fn captured_ctrl_k(ppp: f32, gpu: bool) -> Harness<'static, PhotocraftApp> {
    let (mut h, id) = dialog_with_renderer(ppp, "edit.search", "search…", gpu);
    // Search's default is ⌘F (Photoshop); ⌘K is Preferences, so this also reports a conflict.
    let current = crate::shortcuts::pretty("Cmd+F");
    // On Windows and Linux the Ctrl key is the command modifier.
    capture(&mut h, &current, Key::ControlLeft, Modifiers::COMMAND, Key::K);
    assert_eq!(field(&h, id, "overrides")["edit.search"], json!("Cmd+K"), "@{ppp}x");
    assert_eq!(field(&h, id, "capture"), json!(false));
    let message = field(&h, id, "message");
    assert!(!message.as_str().unwrap().contains("ontrol"), "@{ppp}x: {message}");
    // The row shows the new shortcut.
    h.get_by_label(&crate::shortcuts::pretty("Cmd+K"));
    h
}

#[test]
fn capture_key_press_records_shift_when_key_events_carry_no_modifier_bits() {
    // Same event pattern as Linux/Windows egui: modifier and letter Key events with empty bits.
    // Legacy capture → (F6, NONE) → shortcut_text → "F6"; capture_key_press → "Shift+F6".
    let key_event = |key: Key| Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE };
    let (mods, pressed) =
        crate::prefs_ui::capture_key_press(Modifiers::NONE, &input_state(vec![key_event(Key::ShiftLeft), key_event(Key::F6)], Modifiers::NONE));
    assert!(mods.shift);
    let (key, m) = pressed.expect("F6 press");
    assert_eq!(key, Key::F6);
    assert!(m.shift);
    assert_eq!(crate::prefs_ui::shortcut_text(key, m).as_deref(), Some("Shift+F6"));
    let (mods, pressed) = crate::prefs_ui::capture_key_press(Modifiers::NONE, &input_state(vec![key_event(Key::ShiftLeft)], Modifiers::NONE));
    assert!(mods.shift);
    assert!(pressed.is_none(), "modifier alone does not finish capture");
    let (_, pressed) = crate::prefs_ui::capture_key_press(mods, &input_state(vec![key_event(Key::F6)], Modifiers::NONE));
    let (_, m) = pressed.expect("letter in a later frame");
    assert!(m.shift, "captureMods carries Shift across frames");
    assert_eq!(crate::prefs_ui::shortcut_text(Key::F6, m).as_deref(), Some("Shift+F6"));
}

#[test]
fn shift_is_recorded_when_modifier_key_events_carry_no_bits() {
    // End-to-end: dialog capture → overrides JSON (shortcut_text → prefs::normalize_shortcut).
    let (mut h, id) = dialog(1.0, "edit.search", "search…");
    h.get_by_label(&crate::shortcuts::pretty("Cmd+F")).click();
    h.run_steps(2);
    key(&mut h, Key::ShiftLeft, true, Modifiers::NONE);
    h.run_steps(1);
    key(&mut h, Key::F6, true, Modifiers::NONE);
    h.run_steps(2);
    assert_eq!(field(&h, id, "overrides")["edit.search"], json!("Shift+F6"));
    assert_eq!(field(&h, id, "capture"), json!(false));
}

/// ⌘Z can be bound like any other key (#9487): while a field waits for keys it is the shortcut
/// to record, not the dialog's own Undo (#4697), which used to undo the click that started the
/// capture. Once recorded, ⌘Z steps the dialog back again.
#[test]
fn cmd_z_is_recorded_while_capturing_and_undoes_otherwise() {
    let (mut h, id) = dialog(1.0, "edit.search", "search…");
    capture(&mut h, &crate::shortcuts::pretty("Cmd+F"), Key::ControlLeft, Modifiers::COMMAND, Key::Z);
    assert_eq!(field(&h, id, "overrides")["edit.search"], json!("Cmd+Z"));
    assert_eq!(field(&h, id, "capture"), json!(false));
    key(&mut h, Key::Z, true, Modifiers::COMMAND);
    h.run_steps(2);
    assert_ne!(field(&h, id, "overrides")["edit.search"], json!("Cmd+Z"), "⌘Z outside a capture undoes the change");
    assert_eq!(field(&h, id, "capture"), json!(false), "and doesn't wait for keys again");
    // CodeRabbit on #9504: clearing the restored capture isn't a new edit, so Redo still works.
    h.run_steps(2);
    key(&mut h, Key::Z, true, Modifiers::COMMAND | Modifiers::SHIFT);
    h.run_steps(2);
    assert_eq!(field(&h, id, "overrides")["edit.search"], json!("Cmd+Z"), "⇧⌘Z redoes the recorded shortcut");
    assert_eq!(field(&h, id, "capture"), json!(false));
}

#[test]
fn ctrl_k_is_recorded_as_ctrl_k_not_the_control_key() {
    for ppp in [1.0, 2.0] {
        let _h = captured_ctrl_k(ppp, false);
    }
}

/// Opt-in visual check, following the Camera Raw localization snapshot convention.
#[test]
fn captured_shortcut_gpu_snapshots() {
    let Some(directory) = std::env::var_os("PHOTOCRAFT_SHORTCUT_SNAPSHOTS") else {
        eprintln!("set PHOTOCRAFT_SHORTCUT_SNAPSHOTS to capture the shortcut dialog at 1x and 2x");
        return;
    };
    let _gpu = photocraft_testkit::gpu_lock();
    let directory = std::path::PathBuf::from(directory);
    std::fs::create_dir_all(&directory).unwrap();
    for (scale, ppp) in [(1, 1.0), (2, 2.0)] {
        let mut h = captured_ctrl_k(ppp, true);
        let image = h.render().expect("render captured shortcut");
        assert_eq!((image.width(), image.height()), (1100 * scale, 760 * scale));
        image.save(directory.join(format!("ctrl-k-{scale}x.png"))).expect("save shortcut snapshot");
    }
}

#[test]
fn every_modifier_key_waits_for_the_real_key() {
    for (modifier, held) in [
        (Key::ControlRight, Modifiers::COMMAND),
        (Key::ShiftLeft, Modifiers::SHIFT),
        (Key::AltRight, Modifiers::ALT),
        (Key::SuperLeft, Modifiers::COMMAND),
        (Key::ShiftRight, Modifiers::COMMAND | Modifiers::SHIFT),
    ] {
        let (mut h, id) = dialog(1.0, "edit.search", "search…");
        h.get_by_label(&crate::shortcuts::pretty("Cmd+F")).click();
        h.run_steps(2);
        key(&mut h, modifier, true, held);
        h.run_steps(2);
        assert_eq!(field(&h, id, "capture"), json!(true), "{modifier:?} alone keeps capturing");
        assert!(field(&h, id, "overrides").get("edit.search").is_none(), "{modifier:?} alone assigns nothing");
        key(&mut h, Key::J, true, held);
        h.run_steps(2);
        let want = crate::prefs_ui::shortcut_text(Key::J, held).unwrap();
        assert_eq!(field(&h, id, "overrides")["edit.search"], json!(want), "{modifier:?}");
    }
    for k in [Key::ControlLeft, Key::ShiftLeft, Key::AltLeft, Key::SuperRight] {
        assert_eq!(crate::prefs_ui::shortcut_text(k, Modifiers::COMMAND), None, "{k:?}");
    }
}

#[test]
fn a_shortcut_in_use_is_reported_and_moved_on_ok() {
    // ⌘J is Layer › New › Layer via Copy: giving it to Search warns, and OK moves it.
    let (mut h, id) = dialog(1.0, "edit.search", "search…");
    capture(&mut h, &crate::shortcuts::pretty("Cmd+F"), Key::ControlLeft, Modifiers::COMMAND, Key::J);
    let message = field(&h, id, "message");
    assert!(message.as_str().unwrap().contains("already in use"), "{message}");
    crate::dialogs::confirm(h.state_mut(), id).unwrap();
    h.run_steps(2);
    let app = h.state();
    assert_eq!(app.session.prefs().shortcut("edit.search", Some("Cmd+F")), Some("Cmd+J"));
    let bound: Vec<String> =
        crate::shortcut_dispatch::bindings(app).into_iter().filter(|(_, sc)| *sc == crate::shortcuts::parse("Cmd+J").unwrap()).map(|(id, _)| id).collect();
    assert_eq!(bound, ["edit.search"], "one owner for ⌘J");
}

#[test]
fn a_function_key_moved_off_a_panel_toggle_runs_its_new_command() {
    // #1272: F5 given to Select › Modify › Feather… is taken from Window › Brush Settings, whose
    // row in the dialog is Photoshop's `window.panel.brushSettings` while the key ran the shell's
    // `window.toggle.brushSettings`.
    let (mut h, id) = dialog(1.0, "select.modify.feather", "feather");
    // Enter capture on the default row, then press plain F5 (not Shift+F5: #7190 records held modifiers).
    h.get_by_label(&crate::shortcuts::pretty("Shift+F6")).click();
    h.run_steps(2);
    key(&mut h, Key::F5, true, Modifiers::NONE);
    h.run_steps(1);
    key(&mut h, Key::F5, false, Modifiers::NONE);
    h.run_steps(2);
    let message = field(&h, id, "message");
    assert!(message.as_str().unwrap().contains("already in use"), "{message}");
    crate::dialogs::confirm(h.state_mut(), id).unwrap();
    h.run_steps(2);
    let f5 = crate::shortcuts::parse("F5").unwrap();
    let bound: Vec<String> = crate::shortcut_dispatch::bindings(h.state()).into_iter().filter(|(_, sc)| *sc == f5).map(|(id, _)| id).collect();
    assert_eq!(bound, ["select.modify.feather"], "one owner for F5");
    // Pressing F5 opens Feather, not Brush Settings.
    let ctx = h.ctx.clone();
    crate::shortcut_dispatch::set_dry_run(&ctx, true);
    key(&mut h, Key::F5, true, Modifiers::NONE);
    h.run_steps(1);
    key(&mut h, Key::F5, false, Modifiers::NONE);
    h.run_steps(1);
    let log = crate::shortcut_dispatch::take_log(&ctx);
    assert!(log.iter().any(|(id, _)| id == "select.modify.feather"), "{log:?}");
    assert!(!h.state().ui.panels.brush_settings);
}

#[test]
fn a_user_shortcut_wins_over_a_default_it_was_not_moved_from() {
    // A key assigned by hand (here through the prefs file, so no row was cleared) still reaches
    // the command it was given to rather than the built-in owner.
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("edit.keyboardShortcuts", json!({"set": {"select.modify.feather": "F7"}, "allowUnknown": true, "removeConflicts": false})).unwrap();
    let f7 = crate::shortcuts::parse("F7").unwrap();
    let bound: Vec<String> = crate::shortcut_dispatch::bindings(&app).into_iter().filter(|(_, sc)| *sc == f7).map(|(id, _)| id).collect();
    assert_eq!(bound, ["select.modify.feather"]);
}

/// Capture `k` with `m` for the selected command.
fn capture_key(h: &mut Harness<'_, PhotocraftApp>, id: u64, k: Key, m: Modifiers) {
    h.state_mut().ui.dialog_mut(id).unwrap().fields.insert("capture".into(), json!(true));
    h.run_steps(2);
    key(h, k, true, m);
    h.run_steps(2);
    key(h, k, false, m);
    h.run_steps(2);
}

fn click(h: &mut Harness<'_, PhotocraftApp>, label: &str) {
    h.get_by_role_and_label(egui::accesskit::Role::Button, label).click();
    h.run_steps(3);
}

#[test]
fn a_conflict_offers_accept_and_go_to_conflict() {
    // Photoshop: the key is taken from Copy now, and Copy is selected to get a new one.
    let (mut h, id) = dialog(1.0, "edit.cut", "");
    capture_key(&mut h, id, Key::C, Modifiers::COMMAND);
    assert_eq!(field(&h, id, "overrides")["edit.cut"], json!("Cmd+C"));
    assert_eq!(field(&h, id, "conflict")["with"], json!("edit.copy"));
    click(&mut h, "Accept and Go to Conflict");
    assert_eq!(field(&h, id, "selected"), json!("edit.copy"));
    assert_eq!(field(&h, id, "overrides")["edit.copy"], json!(""), "Copy no longer holds Cmd+C");
    assert!(field(&h, id, "conflict").is_null());
    crate::dialogs::confirm(h.state_mut(), id).unwrap();
    assert_eq!(crate::shortcuts::effective_shortcut(h.state(), "edit.cut", Some("Cmd+X")).as_deref(), Some("Cmd+C"));
    assert_eq!(crate::shortcuts::effective_shortcut(h.state(), "edit.copy", Some("Cmd+C")), None);
}

#[test]
fn undo_changes_puts_the_command_back() {
    let (mut h, id) = dialog(1.0, "edit.cut", "");
    capture_key(&mut h, id, Key::C, Modifiers::COMMAND);
    click(&mut h, "Undo Changes");
    assert!(field(&h, id, "overrides").get("edit.cut").is_none(), "{}", field(&h, id, "overrides"));
    assert!(field(&h, id, "conflict").is_null());
    // An alternate added with Add Shortcut is undone the same way.
    h.state_mut().ui.dialog_mut(id).unwrap().fields.insert("captureAlt".into(), json!(true));
    capture_key(&mut h, id, Key::C, Modifiers::COMMAND);
    assert_eq!(field(&h, id, "alternates")["edit.cut"], json!(["F2", "Cmd+C"]), "beside Photoshop's F2");
    click(&mut h, "Undo Changes");
    assert!(field(&h, id, "alternates").get("edit.cut").is_none(), "{}", field(&h, id, "alternates"));
    crate::dialogs::confirm(h.state_mut(), id).unwrap();
    assert_eq!(crate::shortcuts::effective_shortcut(h.state(), "edit.copy", Some("Cmd+C")).as_deref(), Some("Cmd+C"));
}

#[test]
fn a_system_key_is_refused() {
    let (k, m) = if cfg!(target_os = "macos") { (Key::Tab, Modifiers::MAC_CMD | Modifiers::COMMAND) } else { (Key::F4, Modifiers::ALT) };
    let (mut h, id) = dialog(1.0, "edit.cut", "");
    capture_key(&mut h, id, k, m);
    assert!(field(&h, id, "overrides").get("edit.cut").is_none(), "{}", field(&h, id, "overrides"));
    assert!(field(&h, id, "message").as_str().is_some_and(|s| s.contains("reserved")), "{}", field(&h, id, "message"));
    assert_eq!(field(&h, id, "messageWarning"), json!(true));
    assert!(crate::prefs_ui::reserved_shortcut(if cfg!(target_os = "macos") { "Cmd+Tab" } else { "Alt+F4" }));
    assert!(!crate::prefs_ui::reserved_shortcut("Cmd+Shift+F4"));
    assert!(!crate::prefs_ui::reserved_shortcut("garbage+"));
}

/// The conflict describes the bindings as the capture left them: Reset All, Use Default or
/// Escape after it makes it stale, so Accept can't take a key nothing holds any more.
#[test]
fn a_conflict_goes_stale_when_the_bindings_change() {
    for undo in ["Reset All to Defaults", "Use Default", "escape"] {
        let (mut h, id) = dialog(1.0, "edit.cut", "");
        capture_key(&mut h, id, Key::C, Modifiers::COMMAND);
        assert!(field(&h, id, "conflict").is_object(), "{undo}");
        if undo == "escape" {
            h.state_mut().ui.dialog_mut(id).unwrap().fields.insert("capture".into(), json!(true));
            h.run_steps(2);
            key(&mut h, Key::Escape, true, Modifiers::NONE);
            h.run_steps(2);
            key(&mut h, Key::Escape, false, Modifiers::NONE);
            h.run_steps(2);
        } else {
            click(&mut h, undo);
        }
        assert!(field(&h, id, "conflict").is_null(), "{undo}: {}", field(&h, id, "conflict"));
        assert!(h.query_by_label("Accept and Go to Conflict").is_none(), "{undo}");
        crate::dialogs::confirm(h.state_mut(), id).unwrap();
        if undo != "escape" {
            assert_eq!(crate::shortcuts::effective_shortcut(h.state(), "edit.copy", Some("Cmd+C")).as_deref(), Some("Cmd+C"), "{undo}: Copy keeps ⌘C");
        }
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
#[test]
fn linux_reserves_its_own_system_keys() {
    assert!(crate::prefs_ui::reserved_shortcut("Cmd+Alt+F2"));
    assert!(!crate::prefs_ui::reserved_shortcut("Cmd+Escape"));
}

#[cfg(target_os = "windows")]
#[test]
fn windows_reserves_its_own_system_keys() {
    assert!(crate::prefs_ui::reserved_shortcut("Cmd+Escape"));
    assert!(!crate::prefs_ui::reserved_shortcut("Cmd+Alt+F2"));
}

/// A key two commands hold (Cut's ⌘X, given to Copy too): Accept takes it from both.
#[test]
fn accept_takes_the_key_from_every_command_holding_it() {
    let (mut h, id) = dialog(1.0, "edit.paste", "");
    h.state_mut().ui.dialog_mut(id).unwrap().fields.insert("overrides".into(), json!({"edit.copy": "Cmd+X"}));
    h.run_steps(2);
    capture_key(&mut h, id, Key::X, Modifiers::COMMAND);
    let holders = field(&h, id, "conflict")["holders"].clone();
    assert!(holders.as_array().is_some_and(|a| a.len() == 2), "{holders}");
    click(&mut h, "Accept and Go to Conflict");
    let o = field(&h, id, "overrides");
    assert_eq!((o["edit.cut"].clone(), o["edit.copy"].clone(), o["edit.paste"].clone()), (json!(""), json!(""), json!("Cmd+X")), "{o}");
}
