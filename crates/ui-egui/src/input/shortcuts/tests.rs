use super::*;

#[test]
fn tool_key_brings_back_the_groups_last_used_tool() {
    use crate::state::Tool;
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    let ctx = egui::Context::default();
    let press = |app: &mut PhotocraftApp, key, modifiers| {
        let raw = egui::RawInput {
            events: vec![
                egui::Event::ModifiersChanged(modifiers),
                egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers },
                egui::Event::Key { key, physical_key: None, pressed: false, repeat: false, modifiers },
            ],
            ..Default::default()
        };
        ctx.begin_pass(raw);
        handle(app, &ctx);
        ctx.end_pass().textures_delta.clear();
    };
    // Plain-key cycling (the pref is on by default, as in Photoshop).
    app.session.edit_prefs(|p| p.tools.use_shift_key_for_tool_switch = false);
    // Polygonal Lasso picked from the toolbar flyout, then the Brush (#2608).
    app.ui.tool = Tool::PolygonLasso;
    press(&mut app, Key::B, Modifiers::NONE);
    assert_eq!(app.ui.tool, Tool::Brush);
    press(&mut app, Key::L, Modifiers::NONE);
    assert_eq!(app.ui.tool, Tool::PolygonLasso);
    // Within the group, L still cycles; leaving and coming back keeps the new pick.
    press(&mut app, Key::L, Modifiers::NONE);
    assert_eq!(app.ui.tool, Tool::MagneticLasso);
    press(&mut app, Key::M, Modifiers::NONE);
    assert_eq!(app.ui.tool, Tool::RectMarquee);
    press(&mut app, Key::L, Modifiers::NONE);
    assert_eq!(app.ui.tool, Tool::MagneticLasso);
    // Use Shift Key for Tool Switch: the plain key also returns to the last-used tool.
    app.session.edit_prefs(|p| p.tools.use_shift_key_for_tool_switch = true);
    press(&mut app, Key::B, Modifiers::NONE);
    press(&mut app, Key::L, Modifiers::NONE);
    assert_eq!(app.ui.tool, Tool::MagneticLasso);
}

/// #3296: by default T keeps the current type tool and only Shift+T cycles, as in Photoshop.
#[test]
fn shift_key_tool_switch_is_the_default() {
    use crate::state::Tool;
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    let ctx = egui::Context::default();
    let press = |app: &mut PhotocraftApp, modifiers| {
        let raw = egui::RawInput {
            events: vec![
                egui::Event::ModifiersChanged(modifiers),
                egui::Event::Key { key: Key::T, physical_key: None, pressed: true, repeat: false, modifiers },
                egui::Event::Key { key: Key::T, physical_key: None, pressed: false, repeat: false, modifiers },
            ],
            ..Default::default()
        };
        ctx.begin_pass(raw);
        handle(app, &ctx);
        ctx.end_pass().textures_delta.clear();
    };
    app.ui.tool = Tool::Type;
    press(&mut app, Modifiers::NONE);
    press(&mut app, Modifiers::NONE);
    assert_eq!(app.ui.tool, Tool::Type);
    press(&mut app, Modifiers::SHIFT);
    assert_ne!(app.ui.tool, Tool::Type);
    // A stored "off" restores plain-key cycling.
    app.session.edit_prefs(|p| p.tools.use_shift_key_for_tool_switch = false);
    app.ui.tool = Tool::Type;
    press(&mut app, Modifiers::NONE);
    assert_ne!(app.ui.tool, Tool::Type);
}

/// #3109: on a German layout Ö / # (physical `;` / `\`) step the brush size and ⇧Ö / ⇧#
/// (⇧# types `'`) its hardness, as Photoshop's German shortcuts.
#[test]
fn german_o_umlaut_and_hash_are_the_brush_keys() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 32, "height": 32})).unwrap();
    app.ui.tool = Tool::Brush;
    app.session.tools.brush.hardness = 0.5;
    let ctx = egui::Context::default();
    let typed = |app: &mut PhotocraftApp, key, physical_key, text: &str, modifiers| {
        let press = egui::Event::Key { key, physical_key: Some(physical_key), pressed: true, repeat: false, modifiers };
        let raw = egui::RawInput { events: vec![egui::Event::ModifiersChanged(modifiers), press, egui::Event::Text(text.into())], ..Default::default() };
        ctx.begin_pass(raw);
        handle(app, &ctx);
        ctx.end_pass().textures_delta.clear();
    };
    let size = app.session.tools.brush.size;
    typed(&mut app, Key::Backslash, Key::Backslash, "#", Modifiers::NONE);
    assert!(app.session.tools.brush.size > size, "# is ]");
    typed(&mut app, Key::Semicolon, Key::Semicolon, "ö", Modifiers::NONE);
    assert_eq!(app.session.tools.brush.size, size, "ö is [");
    typed(&mut app, Key::Quote, Key::Backslash, "'", Modifiers::SHIFT);
    assert!(app.session.tools.brush.hardness > 0.5, "⇧# is ⇧]");
    typed(&mut app, Key::Semicolon, Key::Semicolon, "Ö", Modifiers::SHIFT);
    assert_eq!(app.session.tools.brush.hardness, 0.5, "⇧Ö is ⇧[");
}

#[test]
fn pen_command_z_and_backspace_retract_points_before_document_undo() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", serde_json::json!({"width": 200, "height": 200})).unwrap();
    app.ui.tool = crate::state::Tool::Pen;
    app.run("shape.create", serde_json::json!({"kind": "rect", "rect": [10, 10, 40, 40], "fill": "#ff0000"})).unwrap();
    let history = app.session.active().unwrap().history.past_len();
    for (x, y) in [(10.0, 10.0), (50.0, 10.0), (50.0, 50.0)] {
        crate::vector_ui::pen_down(&mut app, x, y, false);
        crate::vector_ui::pen_up(&mut app);
    }
    let ctx = egui::Context::default();
    let press = |app: &mut PhotocraftApp, key, modifiers| {
        let raw = egui::RawInput {
            events: vec![egui::Event::ModifiersChanged(modifiers), egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers }],
            ..Default::default()
        };
        ctx.begin_pass(raw);
        handle(app, &ctx);
        ctx.end_pass().textures_delta.clear();
    };

    press(&mut app, Key::Z, Modifiers::COMMAND);
    assert_eq!(app.ui.pen.as_ref().unwrap().knots.len(), 2);
    assert_eq!(app.session.active().unwrap().history.past_len(), history);
    press(&mut app, Key::Backspace, Modifiers::NONE);
    assert_eq!(app.ui.pen.as_ref().unwrap().knots.len(), 1);
    press(&mut app, Key::Delete, Modifiers::NONE);
    assert!(app.ui.pen.is_none());
    assert_eq!(app.session.active().unwrap().history.past_len(), history);
    // No unfinished Pen points remain, so regular Undo can now affect the document.
    app.run("edit.undo", serde_json::json!({})).unwrap();
    assert_eq!(app.session.active().unwrap().history.past_len(), history - 1);
}

#[test]
fn pen_custom_undo_binding_and_menu_retract_pending_points() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", serde_json::json!({"width": 200, "height": 200})).unwrap();
    app.ui.tool = crate::state::Tool::Pen;
    for (x, y) in [(10.0, 10.0), (50.0, 10.0)] {
        crate::vector_ui::pen_down(&mut app, x, y, false);
        crate::vector_ui::pen_up(&mut app);
    }
    app.session.prefs.edit(|p| p.shortcuts.insert("edit.undo".into(), "Cmd+Shift+Y".into()));
    let ctx = egui::Context::default();
    let press = |app: &mut PhotocraftApp, key: Key, modifiers: Modifiers| {
        ctx.begin_pass(egui::RawInput {
            events: vec![egui::Event::ModifiersChanged(modifiers), egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers }],
            ..Default::default()
        });
        handle(app, &ctx);
        ctx.end_pass().textures_delta.clear();
    };
    // Once rebound, the default key must no longer intercept pending points.
    press(&mut app, Key::Z, Modifiers::COMMAND);
    assert_eq!(app.ui.pen.as_ref().unwrap().knots.len(), 2);
    press(&mut app, Key::Y, Modifiers::COMMAND | Modifiers::SHIFT);
    assert_eq!(app.ui.pen.as_ref().unwrap().knots.len(), 1);
    assert!(crate::menus::is_enabled(&app, "edit.undo"));
    crate::menus::invoke(&mut app, &ctx, "edit.undo", serde_json::json!({})).unwrap();
    assert!(app.ui.pen.is_none());
}

#[test]
fn parses_registry_shortcuts() {
    let sc = parse("Cmd+Shift+N").unwrap();
    assert_eq!(sc.logical_key, Key::N);
    assert!(sc.modifiers.command && sc.modifiers.shift);
    assert_eq!(parse("Cmd+]").unwrap().logical_key, Key::CloseBracket);
    assert_eq!(parse("Cmd+=").unwrap().logical_key, Key::Equals);
    assert_eq!(parse("F7").unwrap().logical_key, Key::F7);
    assert!(parse("Cmd+Nonsense").is_none());
}

#[test]
fn malformed_bindings_do_not_silently_match_a_different_shortcut() {
    for invalid in ["Cmd+Typo+Z", "Cmd+X+Z", "Cmd+Z+X", "Cmd+Z+", "Cmd+Cmd+Z", "Ctrl+Ctrl+Z", "Shift+Shift+Z", "Cmd+Shift+++"] {
        assert!(parse(invalid).is_none(), "{invalid} must not bind a command");
    }
    // '+' is a key name as well as a separator; keep existing shortcut spellings.
    for plus in ["+", "Cmd+", "Cmd++", "Cmd+Shift++"] {
        assert_eq!(parse(plus).unwrap().logical_key, Key::Plus, "{plus}");
    }
    assert!(parse("Cmd+Shift+Z").is_some());
    assert_eq!(parse("Cmd+;").unwrap().logical_key, Key::Semicolon);
}

#[test]
fn keys_match_like_photoshop() {
    let cmd = Modifiers::COMMAND;
    let zoom_in = parse("Cmd+=").unwrap();
    // ⌘= , ⌘+ typed with ⇧ (US), ⌘+ from the numpad or a Nordic/German `+` key.
    assert!(key_matches(&zoom_in, Key::Equals, cmd));
    assert!(key_matches(&zoom_in, Key::Plus, cmd | Modifiers::SHIFT));
    assert!(key_matches(&zoom_in, Key::Plus, cmd));
    assert!(!key_matches(&zoom_in, Key::Plus, Modifiers::NONE));
    assert!(!key_matches(&zoom_in, Key::Equals, cmd | Modifiers::ALT));
    assert!(key_matches(&parse("Cmd+-").unwrap(), Key::Minus, cmd));
    assert!(key_matches(&parse("Cmd+0").unwrap(), Key::Num0, cmd));
    // Ctrl (Windows) is ⌘.
    assert!(key_matches(&zoom_in, Key::Plus, Modifiers::CTRL | Modifiers { command: true, ..Default::default() }));
    // Exact modifiers: ⌘⌥I is Image Size, never Invert (⌘I); ⇧⌘Z is not ⌘Z.
    let invert = parse("Cmd+I").unwrap();
    assert!(key_matches(&invert, Key::I, cmd));
    assert!(!key_matches(&invert, Key::I, cmd | Modifiers::ALT));
    assert!(!key_matches(&parse("Cmd+Z").unwrap(), Key::Z, cmd | Modifiers::SHIFT));
    // ⇧ shortcuts on punctuation arrive as the shifted character.
    assert!(key_matches(&parse("Cmd+Shift+;").unwrap(), Key::Colon, cmd | Modifiers::SHIFT));
    assert!(key_matches(&parse("Cmd+Shift+]").unwrap(), Key::CloseCurlyBracket, cmd | Modifiers::SHIFT));
    assert!(!key_matches(&parse("Cmd+;").unwrap(), Key::Colon, cmd | Modifiers::SHIFT));
    // Delete and the Mac's delete (Backspace) both clear.
    let clear = parse("Delete").unwrap();
    assert!(key_matches(&clear, Key::Delete, Modifiers::NONE) && key_matches(&clear, Key::Backspace, Modifiers::NONE));
    assert!(!key_matches(&clear, Key::Backspace, cmd));
}

#[test]
fn pretty_uses_the_platform_notation() {
    let mac = cfg!(target_os = "macos");
    assert_eq!(pretty("Cmd+Shift+N"), if mac { "⌘⇧N" } else { "Ctrl+Shift+N" });
    // Lone keys and inline-hint combos.
    assert_eq!(pretty("Alt"), if mac { "⌥" } else { "Alt" });
    assert_eq!(pretty("Shift+Alt"), if mac { "⇧⌥" } else { "Shift+Alt" });
    assert_eq!(pretty("Enter"), if mac { "↩" } else { "Enter" });
    assert_eq!(pretty("Cmd+Enter"), if mac { "⌘↩" } else { "Ctrl+Enter" });
}

#[test]
fn labels_show_the_effective_shortcut_in_platform_notation() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    let new = command_label(&app, "New document…", "file.new");
    if cfg!(target_os = "macos") {
        assert_eq!(new, "New document…     ⌘N");
    } else {
        assert_eq!(new, "New document…     Ctrl+N");
    }
    assert_eq!(tip_label(&app, "Rulers", "view.rulers"), format!("Rulers  ({})", pretty("Cmd+R")));
    // A user override is picked up…
    app.session.prefs.edit(|p| p.shortcuts.insert("file.new".into(), "Cmd+Shift+N".into()));
    assert_eq!(command_label(&app, "New document…", "file.new"), format!("New document…     {}", pretty("Cmd+Shift+N")));
    // …and a removed binding hides the shortcut entirely.
    app.session.prefs.edit(|p| p.shortcuts.insert("file.new".into(), String::new()));
    assert_eq!(command_label(&app, "New document…", "file.new"), "New document…");
}

#[test]
fn clipboard_events_become_pixel_shortcuts_outside_text() {
    let ctx = egui::Context::default();
    let raw = |mut events: Vec<egui::Event>, modifiers| {
        events.insert(0, egui::Event::ModifiersChanged(modifiers));
        egui::RawInput { events, ..Default::default() }
    };
    let keys = |r: &egui::RawInput| -> Vec<(Key, bool, bool)> {
        r.events
            .iter()
            .filter_map(|e| if let egui::Event::Key { key, pressed, modifiers, .. } = e { Some((*key, *pressed, modifiers.shift)) } else { None })
            .collect()
    };
    let cmd_shift = Modifiers::COMMAND | Modifiers::SHIFT;
    let mut r = raw(vec![egui::Event::Copy, egui::Event::Cut], cmd_shift);
    clipboard_keys(&ctx, false, &mut r);
    assert_eq!(keys(&r), vec![(Key::C, true, true), (Key::X, true, true)]);
    // A text paste becomes ⌘V; its key release then adds nothing.
    let up = egui::Event::Key { key: Key::V, physical_key: None, pressed: false, repeat: false, modifiers: Modifiers::COMMAND };
    let mut r = raw(vec![egui::Event::Paste("x".into()), up.clone()], Modifiers::COMMAND);
    clipboard_keys(&ctx, false, &mut r);
    assert_eq!(keys(&r), vec![(Key::V, true, false), (Key::V, false, false)]);
    // An image on the clipboard: egui-winit sends only the release. It still pastes.
    let mut r = raw(vec![up.clone()], Modifiers::COMMAND);
    clipboard_keys(&ctx, false, &mut r);
    assert_eq!(keys(&r), vec![(Key::V, true, false), (Key::V, false, false)]);
    // In a text field the events stay text clipboard events.
    let mut r = raw(vec![egui::Event::Paste("x".into())], Modifiers::COMMAND);
    clipboard_keys(&ctx, true, &mut r);
    assert!(matches!(r.events.as_slice(), [_, egui::Event::Paste(_)]));
}

/// In a text field the Mac menu's ⌘C / ⌘X key equivalents become the field's Copy / Cut, and
/// its ⌘V asks the window for the clipboard text, which the field pastes. Other keys, and ⇧ / ⌥
/// chords, pass through.
#[test]
fn menu_key_equivalents_reach_a_text_field_as_clipboard_events() {
    let ctx = egui::Context::default();
    let key = |key, pressed, modifiers| egui::Event::Key { key, physical_key: Some(key), pressed, repeat: false, modifiers };
    let cmd = Modifiers::COMMAND;
    let events = vec![
        key(Key::C, true, cmd),
        key(Key::X, true, cmd),
        key(Key::V, true, cmd),
        key(Key::V, false, cmd),
        key(Key::A, true, cmd),
        key(Key::C, true, cmd | Modifiers::SHIFT),
    ];
    let mut r = egui::RawInput { events: [vec![egui::Event::ModifiersChanged(cmd)], events].concat(), ..Default::default() };
    // In a frame, as the app calls it, so the request for the clipboard text is in its output.
    ctx.begin_pass(egui::RawInput::default());
    clipboard_keys(&ctx, true, &mut r);
    let mut out = ctx.end_pass();
    out.textures_delta.clear();
    assert!(
        matches!(
            &r.events[1..],
            [
                egui::Event::Copy,
                egui::Event::Cut,
                egui::Event::Key { key: Key::V, pressed: false, .. },
                egui::Event::Key { key: Key::A, .. },
                egui::Event::Key { key: Key::C, .. }
            ]
        ),
        "{:?}",
        r.events
    );
    // The ⌘V press became a request for the clipboard's text.
    let asked = out.viewport_output.values().flat_map(|v| &v.commands).any(|c| matches!(c, egui::ViewportCommand::RequestPaste));
    assert!(asked, "the window is asked for the clipboard text");
}

/// #1638: every ⌘V is one press, however its key-down and key-up arrive.
#[test]
fn each_paste_key_is_one_press() {
    let ctx = egui::Context::default();
    let key = |key, pressed, modifiers| egui::Event::Key { key, physical_key: None, pressed, repeat: false, modifiers };
    let presses = |events: Vec<egui::Event>| {
        let mut r = egui::RawInput { events, ..Default::default() };
        clipboard_keys(&ctx, false, &mut r);
        r.events.iter().filter(|e| matches!(e, egui::Event::Key { key: Key::V, pressed: true, .. })).count()
    };
    // A ⌘V press that reached us (the Mac menu's key equivalent), then its release.
    assert_eq!(presses(vec![key(Key::V, true, Modifiers::COMMAND)]), 1);
    assert_eq!(presses(vec![key(Key::V, false, Modifiers::COMMAND)]), 0, "the release of a press doesn't paste again");
    // A text paste whose V is released after ⌘: the next image paste still pastes.
    assert_eq!(presses(vec![egui::Event::Paste("x".into())]), 1);
    assert_eq!(presses(vec![egui::Event::ModifiersChanged(Modifiers::NONE), key(Key::V, false, Modifiers::NONE)]), 0);
    assert_eq!(presses(vec![egui::Event::ModifiersChanged(Modifiers::COMMAND), key(Key::V, false, Modifiers::COMMAND)]), 1, "image paste");
}

/// #6329: with an image on the clipboard egui-winit swallows the ⌘V press and sends only the V
/// release, which carries no ⌘ when ⌘ was let go first: that release still pastes, once. A V
/// pressed and released on its own (the Move tool) never pastes.
#[test]
fn an_image_paste_released_after_command_still_pastes() {
    let key = |key, pressed, modifiers| egui::Event::Key { key, physical_key: None, pressed, repeat: false, modifiers };
    let pastes = |ctx: &egui::Context, events: Vec<egui::Event>| {
        let mut r = egui::RawInput { events, ..Default::default() };
        clipboard_keys(ctx, false, &mut r);
        r.events.iter().filter(|e| matches!(e, egui::Event::Key { key: Key::V, pressed: true, modifiers, .. } if modifiers.command)).count()
    };
    let none = Modifiers::NONE;
    // ⌘ down, (the swallowed V press,) ⌘ up, V up: in one frame or over several.
    let ctx = egui::Context::default();
    assert_eq!(pastes(&ctx, vec![egui::Event::ModifiersChanged(Modifiers::COMMAND), egui::Event::ModifiersChanged(none), key(Key::V, false, none)]), 1);
    assert_eq!(pastes(&ctx, vec![egui::Event::ModifiersChanged(Modifiers::COMMAND)]), 0);
    assert_eq!(pastes(&ctx, vec![egui::Event::ModifiersChanged(none)]), 0);
    assert_eq!(pastes(&ctx, vec![key(Key::V, false, none)]), 1, "released after ⌘");
    // Once: another lone release without ⌘ held again doesn't paste again.
    assert_eq!(pastes(&ctx, vec![key(Key::V, false, none)]), 0);
    // A plain V tap after ⌘ was used is the Move tool, not a paste.
    let ctx = egui::Context::default();
    assert_eq!(pastes(&ctx, vec![egui::Event::ModifiersChanged(Modifiers::COMMAND), egui::Event::ModifiersChanged(none)]), 0);
    assert_eq!(pastes(&ctx, vec![key(Key::V, true, none), key(Key::V, false, none)]), 0);
    // Without ⌘ at all, a lone V release is nothing.
    let ctx = egui::Context::default();
    assert_eq!(pastes(&ctx, vec![key(Key::V, false, none)]), 0);
    // Released with ⌘ still held it pastes as before.
    assert_eq!(pastes(&ctx, vec![egui::Event::ModifiersChanged(Modifiers::COMMAND), key(Key::V, false, Modifiers::COMMAND)]), 1);
}

/// #2638: on the web one ⌘V brings its key press and the browser's paste event; that is one paste.
#[test]
fn web_paste_key_and_paste_event_are_one_press() {
    let ctx = egui::Context::default();
    let key = |key, pressed| egui::Event::Key { key, physical_key: Some(key), pressed, repeat: false, modifiers: Modifiers::COMMAND };
    let mut r = egui::RawInput { events: vec![key(Key::V, true), egui::Event::Paste("x".into()), key(Key::X, true), egui::Event::Cut], ..Default::default() };
    clipboard_keys(&ctx, false, &mut r);
    let presses = |k| r.events.iter().filter(|e| matches!(e, egui::Event::Key { key, pressed: true, .. } if *key == k)).count();
    assert_eq!((presses(Key::V), presses(Key::X)), (1, 1));
    let mut r = egui::RawInput { events: vec![key(Key::V, false)], ..Default::default() };
    clipboard_keys(&ctx, false, &mut r);
    assert!(!r.events.iter().any(|e| matches!(e, egui::Event::Key { pressed: true, .. })), "its release doesn't paste again");
}

/// #530: egui-winit sends Cut for Shift+Delete on Windows; on the canvas it opens Fill.
#[test]
fn windows_shift_delete_fills_instead_of_cutting() {
    let shift = Modifiers::SHIFT;
    let ctrl = Modifiers::CTRL | Modifiers::COMMAND;
    let app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    let bound = |held| {
        let (key, mods) = clipboard_press(&egui::Event::Cut, held, true)?;
        assert_eq!((key, mods), (Key::Delete, held));
        crate::shortcut_dispatch::bindings(&app).into_iter().find(|(_, sc)| key_matches(sc, key, mods)).map(|(id, _)| id)
    };
    assert_eq!(bound(shift).as_deref(), Some("edit.fill"));
    // ⇧⌥Delete keeps ⌥: fill with the foreground, keeping transparency.
    assert_eq!(bound(shift | Modifiers::ALT).as_deref(), Some("edit.fillForegroundPreserve"));
    // Ctrl+X (and Ctrl+Shift+X) still cut; elsewhere a Cut is always ⌘X.
    assert_eq!(clipboard_press(&egui::Event::Cut, ctrl, true), Some((Key::X, ctrl)));
    assert_eq!(clipboard_press(&egui::Event::Cut, ctrl | shift, true), Some((Key::X, ctrl | shift)));
    assert_eq!(clipboard_press(&egui::Event::Cut, shift, false), Some((Key::X, Modifiers::COMMAND)));
    // Shift+Insert pastes on Windows.
    assert_eq!(clipboard_press(&egui::Event::Paste("x".into()), shift, true), Some((Key::V, Modifiers::COMMAND)));
    // egui 0.37's image-only clipboard paste is the same ⌘V press; the command reads the
    // clipboard itself, once, through the existing import path.
    let image = std::sync::Arc::new(egui::ColorImage::filled([1, 1], egui::Color32::BLACK));
    assert_eq!(clipboard_press(&egui::Event::PasteImage(image), shift, true), Some((Key::V, Modifiers::COMMAND)));
    // Through the raw-input hook on this platform; a text field keeps its Cut.
    let ctx = egui::Context::default();
    let cut = || egui::RawInput { events: vec![egui::Event::ModifiersChanged(shift), egui::Event::Cut], ..Default::default() };
    let mut r = cut();
    clipboard_keys(&ctx, false, &mut r);
    let want = if cfg!(target_os = "windows") { Key::Delete } else { Key::X };
    assert!(matches!(r.events.as_slice(), [_, egui::Event::Key { key, pressed: true, .. }] if *key == want), "{:?}", r.events);
    let mut r = cut();
    clipboard_keys(&ctx, true, &mut r);
    assert!(matches!(r.events.as_slice(), [_, egui::Event::Cut]));
}

#[cfg(target_os = "linux")]
#[test]
fn ctrl_and_shift_insert_copy_and_paste_on_linux() {
    let ctx = egui::Context::default();
    let insert = |modifiers| egui::RawInput {
        events: vec![egui::Event::Key { key: Key::Insert, physical_key: None, pressed: true, repeat: false, modifiers }],
        ..Default::default()
    };
    let pressed = |r: &egui::RawInput| match r.events.as_slice() {
        [egui::Event::Key { key, pressed: true, modifiers, .. }] if modifiers.command => Some(*key),
        _ => None,
    };
    // On the canvas they become the pixel commands' ⌘C / ⌘V.
    let mut r = insert(Modifiers::CTRL);
    clipboard_keys(&ctx, false, &mut r);
    assert_eq!(pressed(&r), Some(Key::C));
    let mut r = insert(Modifiers::SHIFT);
    clipboard_keys(&ctx, false, &mut r);
    assert_eq!(pressed(&r), Some(Key::V));
    // In a text field Ctrl+Insert copies the text; Shift+Insert asks egui-winit for the text.
    let mut r = insert(Modifiers::CTRL);
    clipboard_keys(&ctx, true, &mut r);
    assert!(matches!(r.events.as_slice(), [egui::Event::Copy]));
    let mut r = insert(Modifiers::SHIFT);
    clipboard_keys(&ctx, true, &mut r);
    assert!(r.events.is_empty());
    // Plain Insert is left alone.
    let mut r = insert(Modifiers::NONE);
    clipboard_keys(&ctx, false, &mut r);
    assert!(matches!(r.events.as_slice(), [egui::Event::Key { key: Key::Insert, .. }]));
}

#[test]
fn every_registered_shortcut_parses() {
    for c in photocraft_engine::command_specs() {
        if let Some(sc) = c.shortcut {
            assert!(parse(sc).is_some(), "{}: {sc}", c.id);
        }
    }
    for (id, _, _, sc) in crate::menus::UI_COMMANDS {
        if let Some(sc) = sc {
            assert!(parse(sc).is_some(), "{id}: {sc}");
        }
    }
}

#[test]
fn no_duplicate_shortcuts() {
    let mut seen = std::collections::HashMap::new();
    let all = photocraft_engine::command_specs()
        .iter()
        .filter_map(|c| c.shortcut.map(|s| (c.id, s)))
        .chain(crate::menus::UI_COMMANDS.iter().filter_map(|(id, _, _, s)| s.map(|s| (*id, s))))
        // Photoshop's second keys (#3665) are taken by no other command either.
        .chain(photocraft_engine::prefs::ALTERNATE_SHORTCUTS.iter().copied());
    for (id, s) in all {
        assert!(parse(s).is_some(), "{id}: {s}");
        if let Some(prev) = seen.insert(s, id) {
            panic!("shortcut {s} bound to both {prev} and {id}");
        }
    }
}
