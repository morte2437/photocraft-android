use super::*;
use serde_json::json;

fn app(doc: bool) -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    if doc {
        app.run("file.new", json!({"width": 64, "height": 48})).unwrap();
        app.sync_views();
    }
    app
}

fn layout(doc: bool) -> Layout {
    photocraft_layout(&crate::menus::menu_items(&app(doc)), Lang::EN, "auto")
}

fn ids(nodes: &[Node]) -> Vec<String> {
    nodes
        .iter()
        .map(|n| match n {
            Node::Item(it) => it.id.clone(),
            Node::Separator => "---".into(),
            Node::Submenu { label, .. } => format!("[{label}]"),
            Node::Standard(s) => format!("<{s:?}>"),
        })
        .collect()
}

#[test]
fn photocraft_gets_a_mac_app_menu() {
    let l = layout(true);
    let app = &l.bar.menus[0];
    assert_eq!((app.title.as_str(), app.role), ("PhotoCraft", MenuRole::App));
    assert_eq!(
        ids(&app.children),
        ["help.about", "---", "[Settings]", "[Language]", "[Appearance]", "---", "<Services>", "---", HIDE, "<HideOthers>", "<ShowAll>", "---", "file.exit"]
    );
    let quit = l.bar.find("file.exit").unwrap();
    assert_eq!((quit.label.as_str(), quit.shortcut.as_deref()), ("Quit PhotoCraft", Some("Cmd+Q")));
    assert_eq!(l.bar.find("help.about").unwrap().label, "About PhotoCraft");
}

#[test]
fn moved_items_leave_their_old_menus() {
    let l = layout(true);
    let by = |t: &str| l.bar.menus.iter().find(|m| m.title == t).unwrap();
    assert!(!ids(&by("File").children).contains(&"file.exit".to_string()));
    assert!(!ids(&by("Help").children).contains(&"help.about".to_string()));
    assert!(!ids(&by("Edit").children).contains(&"[Preferences]".to_string()));
    let titles: Vec<&str> = l.bar.menus.iter().map(|m| m.title.as_str()).collect();
    assert_eq!(titles, ["PhotoCraft", "File", "Edit", "Image", "Layer", "Type", "Select", "Filter", "View", "Window", "Help"]);
}

/// PhotoCraft keeps Photoshop's ⌘H, ⌘M, ⌘W and ⌘, (Hide Layers): the system items give way.
#[test]
fn clashes_are_resolved_in_photocrafts_favour() {
    let l = layout(true);
    let clash = |sc: &str| l.clashes.iter().find(|c| c.shortcut == sc).map(|c| c.command.as_str());
    assert_eq!(clash("Cmd+H"), Some("view.extras"));
    assert_eq!(clash("Cmd+M"), Some("image.adjustments.curves"));
    assert_eq!(clash("Cmd+W"), Some("file.close"));
    assert_eq!(clash("Cmd+,"), Some("layer.hideLayers"));
    assert_eq!(l.bar.find(HIDE).unwrap().shortcut.as_deref(), Some("Ctrl+Cmd+H"), "as in Photoshop");
    assert_eq!(l.bar.find(MINIMIZE).unwrap().shortcut.as_deref(), Some("Ctrl+Cmd+M"));
    let window = l.bar.menu(MenuRole::Window).unwrap();
    assert_eq!(ids(&window.children)[..3], [MINIMIZE.to_string(), "<Zoom>".into(), "---".into()]);
    assert!(ids(&window.children).ends_with(&["<BringAllToFront>".into(), "---".into(), "window.document.0".into()]));
    let file = l.bar.menus.iter().find(|m| m.title == "File").unwrap();
    assert!(!file.children.contains(&Node::Standard(Standard::CloseWindow)), "⌘W stays File › Close");
}

/// Themes and the appearance-mode cycle move from Window to the app menu's Appearance.
#[test]
fn themes_move_to_the_app_menus_appearance() {
    let mut a = app(false);
    a.ui.theme = crate::theme::ThemeKind::Classic;
    let l = photocraft_layout(&crate::menus::menu_items(&a), Lang::EN, "auto");
    let app_menu = l.bar.menu(MenuRole::App).unwrap();
    let Some(Node::Submenu { children, .. }) = app_menu.children.iter().find(|n| matches!(n, Node::Submenu { label, .. } if label == "Appearance")) else {
        panic!("an Appearance submenu");
    };
    let themes = ids(children);
    assert_eq!(themes.last().map(String::as_str), Some("window.theme.toggle"));
    assert!(themes.contains(&"[Dark Theme]".to_string()) && themes.contains(&"[Light Theme]".to_string()));
    assert_eq!(l.bar.find("window.theme.classic").unwrap().checked, Some(true));
    assert_eq!(l.bar.find("window.theme.pro").unwrap().checked, Some(false));
    assert_eq!(l.bar.find("window.theme.toggle").unwrap().label, "Next Appearance Mode");
    assert_eq!(l.bar.find("window.theme.system").unwrap().label, "Sync with system");
    assert_eq!(l.bar.find("window.theme.system").unwrap().checked, Some(false));
    let window = ids(&l.bar.menu(MenuRole::Window).unwrap().children);
    assert!(!window.iter().any(|id| id.starts_with("window.theme.") || id == "[Theme]"), "{window:?}");
    a.session.prefs.edit(|p| p.interface.appearance_mode = photocraft_engine::prefs::AppearanceMode::Auto);
    for palette in [crate::theme::ThemeKind::Classic, crate::theme::ThemeKind::ProMedium] {
        a.ui.theme = palette;
        let automatic = photocraft_layout(&crate::menus::menu_items(&a), Lang::EN, "auto");
        assert_eq!(automatic.bar.find("window.theme.system").unwrap().checked, Some(true));
        for kind in crate::theme::ThemeKind::ALL {
            assert_eq!(automatic.bar.find(&format!("window.theme.{}", kind.id())).unwrap().checked, Some(false));
        }
    }
}

/// Language follows Settings in the app menu: Auto and every UI language in its own name,
/// the preference checked; choosing one sets `interface.language`.
#[test]
fn the_app_menu_switches_the_ui_language() {
    let l = photocraft_layout(&crate::menus::menu_items(&app(false)), Lang::EN, "ja");
    let app_menu = l.bar.menu(MenuRole::App).unwrap();
    let Some(Node::Submenu { children, .. }) = app_menu.children.iter().find(|n| matches!(n, Node::Submenu { label, .. } if label == "Language")) else {
        panic!("a Language submenu");
    };
    assert_eq!(children.iter().filter(|n| matches!(n, Node::Item(_))).count(), Lang::all().count() + 1);
    assert_eq!(l.bar.find("app.language.ja").map(|it| (it.label.as_str(), it.checked)), Some(("日本語", Some(true))));
    assert_eq!(l.bar.find("app.language.auto").and_then(|it| it.checked), Some(false));
    // Native names stay native in another UI language.
    let de = photocraft_layout(&crate::menus::menu_items(&app(false)), Lang::from_pref("de"), "de");
    assert_eq!(de.bar.find("app.language.ja").map(|it| it.label.as_str()), Some("日本語"));
    let mut a = app(false);
    let ctx = egui::Context::default();
    crate::menus::invoke(&mut a, &ctx, "app.language.fr", json!({})).unwrap();
    assert_eq!(a.session.prefs().interface.language, "fr");
    crate::menus::invoke(&mut a, &ctx, "app.language.auto", json!({})).unwrap();
    assert_eq!(a.session.prefs().interface.language, "auto");
    assert!(crate::menus::invoke(&mut a, &ctx, "app.language.xx-nope", json!({})).is_err(), "an unknown code is an error");
}

/// The menus show the shortcut that runs each command (Undo ⌘Z, Copy ⌘C were blank).
#[test]
fn menu_rows_show_the_shortcut_that_runs_them() {
    let l = layout(true);
    let sc = |id: &str| l.bar.find(id).and_then(|it| it.shortcut.clone());
    assert_eq!(sc("edit.undo").as_deref(), Some("Cmd+Z"));
    assert_eq!(sc("edit.copy").as_deref(), Some("Cmd+C"));
    assert_eq!(sc("edit.paste").as_deref(), Some("Cmd+V"));
    assert_eq!(sc("layer.hideLayers").as_deref(), Some("Cmd+,"));
    let app = app(true);
    let bindings = crate::shortcut_dispatch::bindings(&app);
    for it in crate::menus::menu_items(&app).iter().filter(|it| it.enabled) {
        let Some(shown) = it.shortcut.as_deref().and_then(crate::shortcuts::parse) else { continue };
        let runs = bindings.iter().find(|(_, b)| *b == shown).map(|(id, _)| id.as_str());
        assert!(runs.is_some(), "{} shows {:?}, which runs nothing", it.id, it.shortcut);
    }
}

/// A native menu can't turn a plain item into a check item in place: opening the first
/// document must not change any item's kind, or the whole menu is rebuilt. Window's list of
/// documents gains a row, which rebuilds only the Window menu.
#[test]
fn opening_a_document_changes_state_not_structure() {
    let (a, b) = (crate::menus::menu_items(&app(false)), crate::menus::menu_items(&app(true)));
    let kind = |items: &[MenuItem]| items.iter().map(|i| (i.id.clone(), i.checked.is_some())).collect::<HashMap<_, _>>();
    let (ka, kb) = (kind(&a), kind(&b));
    let flips: Vec<&String> = ka.iter().filter(|(id, k)| kb.get(*id).is_some_and(|o| o != *k)).map(|(id, _)| id).collect();
    assert!(flips.is_empty(), "items that change kind: {flips:?}");
    let (no_doc, one_doc) = (layout(false).bar, layout(true).bar);
    assert_eq!(structure_key(&no_doc), structure_key(&one_doc));
    assert_ne!(window_key(&no_doc), window_key(&one_doc), "the Window menu is rebuilt for its new row");
}

/// #2991: the open documents follow Bring All to Front, where a Mac app lists its windows,
/// the active one checked. Switching documents moves the check without a rebuild.
#[test]
fn the_documents_follow_bring_all_to_front() {
    let none = ids(&layout(false).bar.menu(MenuRole::Window).unwrap().children);
    assert_eq!(none.last().map(String::as_str), Some("<BringAllToFront>"));

    let mut a = app(true);
    a.run("file.new", json!({"width": 32, "height": 32})).unwrap();
    a.sync_views();
    let bar = photocraft_layout(&crate::menus::menu_items(&a), Lang::EN, "auto").bar;
    let window = ids(&bar.menu(MenuRole::Window).unwrap().children);
    assert_eq!(window[window.len() - 4..], ["<BringAllToFront>", "---", "window.document.0", "window.document.1"]);
    assert_eq!(bar.find("window.document.1").and_then(|i| i.checked), Some(true));
    assert_eq!(bar.find("window.document.0").and_then(|i| i.checked), Some(false));

    a.session.set_active(0);
    let switched = photocraft_layout(&crate::menus::menu_items(&a), Lang::EN, "auto").bar;
    assert_eq!(switched.find("window.document.0").and_then(|i| i.checked), Some(true));
    assert_eq!((structure_key(&switched), window_key(&switched)), (structure_key(&bar), window_key(&bar)), "a check moves in place");
}

#[test]
fn native_key_equivalents_are_command_and_function_keys_only() {
    assert!(Chord::parse("Cmd+B").unwrap().native_ok());
    assert!(Chord::parse("F5").unwrap().native_ok());
    assert!(!Chord::parse("B").unwrap().native_ok());
    assert!(!Chord::parse("Alt+F").unwrap().native_ok());
    assert!(!Chord::parse("Shift+X").unwrap().native_ok());
    assert_eq!(Chord::parse("Ctrl+Cmd+M").unwrap().portable(), "Ctrl+Cmd+M");
    assert_eq!(Chord::parse("Cmd++").unwrap().key, "+");
}

/// Every menu shortcut is written so it parses and turns back into the key press egui gets.
#[test]
fn every_shortcut_becomes_a_key_press() {
    for it in layout(true).bar.items() {
        let Some(sc) = &it.shortcut else { continue };
        let c = Chord::parse(sc).unwrap_or_else(|| panic!("{sc} on {} doesn't parse", it.id));
        assert_eq!(&c.portable(), sc, "{} writes its modifiers in another order", it.id);
        let Some(egui::Event::Key { key, modifiers, .. }) = c.key_event(true) else { panic!("{sc} on {} has no egui key", it.id) };
        let parsed = crate::shortcuts::parse(sc).unwrap();
        assert!(crate::shortcuts::key_matches(&parsed, key, modifiers), "{sc} on {} doesn't match its own key press", it.id);
    }
}

fn frame(ctx: &egui::Context, raw: egui::RawInput, f: impl FnMut(&mut egui::Ui)) {
    let mut out = ctx.run_ui(raw, f);
    out.textures_delta.clear();
}

struct Fake {
    synced: std::rc::Rc<std::cell::RefCell<Vec<MenuBar>>>,
    events: Vec<Event>,
}

impl Backend for Fake {
    fn sync(&mut self, bar: &MenuBar) {
        self.synced.borrow_mut().push(bar.clone());
    }
    fn drain(&mut self) -> Vec<Event> {
        std::mem::take(&mut self.events)
    }
}

/// A key equivalent reaches PhotoCraft's own key handling as the key press it was, so the
/// shortcut runs once, through the same rules as without a native menu.
#[test]
fn key_equivalents_run_through_the_normal_shortcut_path() {
    let synced = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let mut app = app(true);
    let ctx = egui::Context::default();
    let rulers = Chord::parse("Cmd+R").unwrap();
    app.services.native_menu = Some(NativeMenu::new(Box::new(Fake { synced: synced.clone(), events: vec![Event::Key(rulers)] })));
    let before = app.ui.extras.rulers;
    let mut raw = egui::RawInput::default();
    if let Some(m) = app.services.native_menu.as_mut() {
        m.raw_input(&mut raw);
    }
    assert_eq!(raw.events.len(), 1, "the press: winit delivers the release");
    frame(&ctx, raw, |ui| {
        run(&mut app, ui.ctx());
        crate::shortcuts::handle(&mut app, ui.ctx());
        sync(&mut app, ui.ctx());
    });
    assert_eq!(app.ui.extras.rulers, !before, "⌘R toggled View › Rulers once");
    assert_eq!(synced.borrow().len(), 1, "the menu was built");
    // Nothing changed: no rebuild of the rows on the next frame.
    frame(&ctx, egui::RawInput::default(), |ui| sync(&mut app, ui.ctx()));
    assert_eq!(synced.borrow().len(), 1);
}

/// #1638: ⌘V through the Mac menu pasted three layers. AppKit takes the key-down for Edit ›
/// Paste's key equivalent, but winit still delivers the key-up (it forwards key-ups while ⌘ is
/// held). The menu's press plus that real release must paste once, whether the release
/// arrives in a later frame or already sits in the same input as the menu event.
#[test]
fn a_paste_key_equivalent_pastes_once() {
    let release = || egui::Event::Key { key: Key::V, physical_key: Some(Key::V), pressed: false, repeat: false, modifiers: Modifiers::COMMAND };
    for same_frame in [false, true] {
        let mut app = app(true);
        let ctx = egui::Context::default();
        app.run("select.rect", json!({"x": 0, "y": 0, "width": 8, "height": 4})).unwrap();
        app.run("edit.copy", json!({})).unwrap();
        let layers = |app: &PhotocraftApp| app.session.active().unwrap().doc.layer_count();
        let before = layers(&app);
        let paste = Chord::parse("Cmd+V").unwrap();
        app.services.native_menu = Some(NativeMenu::new(Box::new(Fake { synced: Default::default(), events: vec![Event::Key(paste)] })));
        let step = |app: &mut PhotocraftApp, events: Vec<egui::Event>| {
            let mut raw = egui::RawInput { events, ..Default::default() };
            eframe::App::raw_input_hook(app, &ctx, &mut raw);
            frame(&ctx, raw, |ui| {
                run(app, ui.ctx());
                crate::shortcuts::handle(app, ui.ctx());
            });
        };
        if same_frame {
            // A quick tap: the real key-up is already queued when the menu event is drained.
            step(&mut app, vec![egui::Event::ModifiersChanged(Modifiers::COMMAND), release()]);
        } else {
            step(&mut app, vec![egui::Event::ModifiersChanged(Modifiers::COMMAND)]);
            step(&mut app, vec![release()]);
        }
        step(&mut app, vec![egui::Event::ModifiersChanged(Modifiers::NONE)]);
        assert_eq!(layers(&app), before + 1, "one ⌘V pastes one layer (key-up in the same frame: {same_frame})");
    }
}

#[test]
fn clicks_run_the_command_and_resync() {
    let synced = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let mut app = app(true);
    let ctx = egui::Context::default();
    app.services.native_menu = Some(NativeMenu::new(Box::new(Fake { synced: synced.clone(), events: vec![Event::Click("window.panel.layers".into())] })));
    let before = app.ui.panels.layers;
    let mut raw = egui::RawInput::default();
    if let Some(m) = app.services.native_menu.as_mut() {
        m.raw_input(&mut raw);
    }
    frame(&ctx, raw, |ui| {
        run(&mut app, ui.ctx());
        sync(&mut app, ui.ctx());
    });
    assert_eq!(app.ui.panels.layers, !before);
    let bar = synced.borrow().last().cloned().unwrap();
    assert_eq!(bar.find("window.panel.layers").unwrap().checked, Some(!before));
}

/// #1354: a dialog is modal for the menu bar too. Its rows are disabled (view navigation
/// aside), and a click that arrives anyway doesn't run under the dialog.
#[test]
fn an_open_dialog_blocks_menu_clicks_except_view_navigation() {
    let synced = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let mut app = app(true);
    let ctx = egui::Context::default();
    crate::menus::invoke(&mut app, &ctx, "image.imageSize", json!({})).unwrap();
    assert!(!app.ui.dialogs.is_empty(), "Image Size opened its dialog");
    let size = |app: &PhotocraftApp| app.session.active().map(|d| (d.doc.size.width, d.doc.size.height));
    let zoom = app.ui.views[0].zoom;
    let events = vec![Event::Click("image.imageRotation.90cw".into()), Event::Click("view.zoomIn".into())];
    app.services.native_menu = Some(NativeMenu::new(Box::new(Fake { synced: synced.clone(), events })));
    let mut raw = egui::RawInput::default();
    if let Some(m) = app.services.native_menu.as_mut() {
        m.raw_input(&mut raw);
    }
    frame(&ctx, raw, |ui| {
        run(&mut app, ui.ctx());
        sync(&mut app, ui.ctx());
    });
    assert_eq!(size(&app), Some((64, 48)), "the rotation didn't run under the dialog");
    assert!(app.ui.views[0].zoom > zoom, "Zoom In still works under a dialog");
    let bar = synced.borrow().last().cloned().unwrap();
    assert!(!bar.find("image.imageRotation.90cw").unwrap().enabled);
    assert!(bar.find("view.zoomIn").unwrap().enabled);
    // With the dialog closed the rows come back and a click runs.
    app.ui.dialogs.clear();
    frame(&ctx, egui::RawInput::default(), |ui| sync(&mut app, ui.ctx()));
    assert!(synced.borrow().last().unwrap().find("image.imageRotation.90cw").unwrap().enabled);
}

/// Edit > Cut uses the type editor's selection, like the clipboard Cut event (#1355).
#[test]
fn cut_selected_type_from_native_menu_matches_keyboard_cut() {
    for native_click in [true, false] {
        let synced = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let mut app = app(true);
        let ctx = egui::Context::default();
        let id = photocraft_doc::LayerId(app.run("type.create", json!({"text": "Hello", "size": 12, "x": 4, "y": 24})).unwrap()["layer"].as_u64().unwrap());
        crate::menus::invoke(&mut app, &ctx, "type.editText", json!({})).unwrap();
        let edit = app.ui.text_edit.as_ref().unwrap();
        assert_eq!((edit.anchor, edit.caret), (0, 5));
        let events = if native_click { vec![Event::Click("edit.cut".into())] } else { Vec::new() };
        app.services.native_menu = Some(NativeMenu::new(Box::new(Fake { synced: synced.clone(), events })));
        frame(&ctx, egui::RawInput::default(), |ui| sync(&mut app, ui.ctx()));
        assert!(synced.borrow().last().unwrap().find("edit.cut").unwrap().enabled, "Cut must be enabled for selected characters");

        let mut raw = egui::RawInput::default();
        app.services.native_menu.as_mut().unwrap().raw_input(&mut raw);
        if !native_click {
            raw.events.push(egui::Event::Cut);
        }
        let mut out = ctx.run_ui(raw, |ui| {
            run(&mut app, ui.ctx());
            crate::shortcuts::handle(&mut app, ui.ctx());
            sync(&mut app, ui.ctx());
        });
        out.textures_delta.clear();
        assert_eq!(out.platform_output.commands, [egui::OutputCommand::CopyText("Hello".into())]);
        let photocraft_doc::LayerContent::Text(text) = &app.session.active().unwrap().doc.layer(id).unwrap().content else { panic!("type layer changed kind") };
        assert!(text.text.is_empty());
        let edit = app.ui.text_edit.as_ref().unwrap();
        assert_eq!((edit.anchor, edit.caret), (0, 0));
        assert!(!synced.borrow().last().unwrap().find("edit.cut").unwrap().enabled, "Cut must be disabled at an empty caret");
        assert!(app.session.clipboard.is_none(), "text Cut must not copy rendered pixels");
    }
}

/// Select > All, Edit > Copy and Edit > Paste work on the text being typed, not on pixels (#2893).
#[test]
fn copy_and_paste_type_from_native_menu() {
    let synced = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let mut app = app(true);
    let ctx = egui::Context::default();
    let id = photocraft_doc::LayerId(app.run("type.create", json!({"text": "Hello", "size": 12, "x": 4, "y": 24})).unwrap()["layer"].as_u64().unwrap());
    crate::menus::invoke(&mut app, &ctx, "type.editText", json!({})).unwrap();
    if let Some(e) = app.ui.text_edit.as_mut() {
        (e.anchor, e.caret) = (5, 5);
    }
    let layers = app.session.active().unwrap().doc.layers.len();
    let click = |app: &mut PhotocraftApp, id: &str| {
        app.services.native_menu = Some(NativeMenu::new(Box::new(Fake { synced: synced.clone(), events: vec![Event::Click(id.into())] })));
        let mut raw = egui::RawInput::default();
        app.services.native_menu.as_mut().unwrap().raw_input(&mut raw);
        let mut out = ctx.run_ui(raw, |ui| {
            run(app, ui.ctx());
            sync(app, ui.ctx());
        });
        out.textures_delta.clear();
        out
    };
    let text = |app: &PhotocraftApp| match &app.session.active().unwrap().doc.layer(id).unwrap().content {
        photocraft_doc::LayerContent::Text(t) => t.text.clone(),
        _ => panic!("type layer changed kind"),
    };

    click(&mut app, "select.all");
    let edit = app.ui.text_edit.as_ref().unwrap();
    assert_eq!((edit.anchor, edit.caret), (0, 5), "Select > All selects the characters");
    let bar = synced.borrow().last().cloned().unwrap();
    assert!(bar.find("edit.copy").unwrap().enabled, "Copy is enabled for selected characters");
    assert!(bar.find("edit.paste").unwrap().enabled, "Paste is enabled while typing");

    let out = click(&mut app, "edit.copy");
    assert_eq!(out.platform_output.commands, [egui::OutputCommand::CopyText("Hello".into())]);
    assert!(app.session.clipboard.is_none(), "text Copy must not copy rendered pixels");

    let out = click(&mut app, "edit.paste");
    let asked = out.viewport_output.values().any(|v| v.commands.contains(&egui::ViewportCommand::RequestPaste));
    assert!(asked, "Edit > Paste asks the window for the clipboard text");
    // The text arrives as a Paste event next frame and replaces the selection.
    let raw = egui::RawInput { events: vec![egui::Event::Paste("World".into())], ..Default::default() };
    frame(&ctx, raw, |ui| crate::shortcuts::handle(&mut app, ui.ctx()));
    assert_eq!(text(&app), "World");
    assert_eq!(app.session.active().unwrap().doc.layers.len(), layers, "no pixel layer was pasted");
    assert!(!app.ui.status_error, "{}", app.ui.status);
}

#[test]
fn native_cut_without_type_edit_keeps_pixel_clipboard_behavior() {
    let synced = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let mut app = app(false);
    app.run("file.new", json!({"width": 4, "height": 4, "background": "transparent"})).unwrap();
    app.run("edit.fill", json!({"color": "#ff0000"})).unwrap();
    assert_eq!(app.run("document.pixel", json!({"x": 1, "y": 1})).unwrap(), json!([1.0, 0.0, 0.0, 1.0]));
    let ctx = egui::Context::default();
    app.services.native_menu = Some(NativeMenu::new(Box::new(Fake { synced: synced.clone(), events: vec![Event::Click("edit.cut".into())] })));
    frame(&ctx, egui::RawInput::default(), |ui| sync(&mut app, ui.ctx()));
    assert_eq!(synced.borrow().last().unwrap().find("edit.cut").unwrap().enabled, app.session.is_enabled("edit.cut"));
    assert!(app.session.is_enabled("edit.cut"));
    let mut raw = egui::RawInput::default();
    app.services.native_menu.as_mut().unwrap().raw_input(&mut raw);
    frame(&ctx, raw, |ui| {
        run(&mut app, ui.ctx());
        sync(&mut app, ui.ctx());
    });
    assert_eq!(app.run("document.pixel", json!({"x": 1, "y": 1})).unwrap(), json!([0.0, 0.0, 0.0, 0.0]));
    let clip = app.session.clipboard.as_ref().unwrap();
    assert_eq!((clip.bounds.width(), clip.bounds.height()), (4, 4));
    let mut px = vec![[0u8; 4]; 16];
    clip.surface.read_rgba8_into(clip.bounds, &mut px);
    assert_eq!(px, vec![[255u8, 0, 0, 255]; 16]);
    assert!(app.ui.text_edit.is_none());
}

/// With the Mac menu bar the title bar draws no menu titles: each title shows once fewer.
#[test]
fn the_title_bar_hides_its_menus_with_the_mac_menu_bar() {
    use egui_kittest::{Harness, kittest::Queryable};
    let counts = |native: bool| -> Vec<usize> {
        let mut h = Harness::builder().with_size(egui::vec2(1440.0, 900.0)).with_max_steps(64).build_eframe(move |cc| {
            PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
            let mut app = app(true);
            if native {
                app.services.native_menu = Some(NativeMenu::new(Box::new(Fake { synced: Default::default(), events: Vec::new() })));
            }
            app
        });
        h.run_steps(4);
        crate::menus::TOP_MENUS.iter().map(|t| h.query_all_by_label(t).count()).collect()
    };
    let (in_window, native) = (counts(false), counts(true));
    for (i, title) in crate::menus::TOP_MENUS.iter().enumerate() {
        assert_eq!(native[i] + 1, in_window[i], "{title}: {} in-window, {} with the Mac menu bar", in_window[i], native[i]);
    }
}

#[test]
fn opening_or_closing_a_workspace_without_input_rebuilds_the_menus() {
    let mut app = app(true);
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("edit.fill", json!({"color": "#808080"})).unwrap();
    let closed = state_hash(&app, None);
    crate::gallery_ui::open(&mut app).unwrap();
    let open = state_hash(&app, None);
    assert_ne!(open, closed, "the menus grey out when an agent opens Filter Gallery");
    app.distort.gallery = None;
    assert_eq!(state_hash(&app, None), closed, "and come back when it is cancelled");
}
