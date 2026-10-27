use super::*;

#[test]
fn showing_grid_restores_hidden_extras() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    let ctx = egui::Context::default();
    app.ui.extras.grid = false;
    app.ui.view.extras = false;

    assert_eq!(invoke(&mut app, &ctx, "view.show.grid", json!({})).unwrap(), json!(true));
    assert_eq!(checked(&app, "view.show.grid"), Some(true));
    assert!(app.ui.view.extras, "showing Grid must make the overlay visible when Extras was hidden");

    assert_eq!(invoke(&mut app, &ctx, "view.show.grid", json!({})).unwrap(), json!(false));
    assert!(!app.ui.extras.grid);
    assert!(app.ui.view.extras);

    app.ui.extras.grid = true;
    app.ui.view.extras = false;
    assert_eq!(invoke(&mut app, &ctx, "view.show.grid", json!({})).unwrap(), json!(false));
    assert!(!app.ui.view.extras, "hiding Grid must leave hidden Extras alone");
}

#[test]
fn sync_with_system_menu_keeps_saved_slots_and_its_checkmark_across_palettes() {
    use photocraft_engine::prefs::{AppearanceMode, DarkTheme, LightTheme};
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    let ctx = egui::Context::default();
    app.services.system_theme = Some(Box::new(|_| Some(egui::Theme::Light)));
    app.run("prefs.set", json!({"values": {"interface.darkTheme": "studio", "interface.lightTheme": "classic"}})).unwrap();
    invoke(&mut app, &ctx, "window.theme.system", json!({})).unwrap();
    assert_eq!(app.session.prefs().interface.appearance_mode, AppearanceMode::Auto);
    assert_eq!(app.session.prefs().interface.dark_theme, DarkTheme::Studio);
    assert_eq!(app.session.prefs().interface.light_theme, LightTheme::Classic);
    assert_eq!(app.ui.theme, crate::theme::ThemeKind::Classic);
    for appearance in [egui::Theme::Dark, egui::Theme::Light] {
        app.services.system_theme = Some(Box::new(move |_| Some(appearance)));
        crate::prefs_ui::tick(&mut app, &ctx);
        let items = menu_items(&app);
        let item = items.iter().find(|i| i.id == "window.theme.system").unwrap();
        assert_eq!(item.label, "Sync with system");
        assert_eq!(item.path, ["Window", "Theme"]);
        assert!(item.enabled);
        assert_eq!(item.checked, Some(true));
        for kind in crate::theme::ThemeKind::ALL {
            assert_eq!(checked(&app, &format!("window.theme.{}", kind.id())), Some(false));
        }
    }
    invoke(&mut app, &ctx, "window.theme.studio", json!({})).unwrap();
    assert_eq!(app.session.prefs().interface.appearance_mode, AppearanceMode::Dark);
    assert_eq!(checked(&app, "window.theme.system"), Some(false));
    assert_eq!(checked(&app, "window.theme.studio"), Some(true));
}

#[test]
fn submenu_header_is_disabled_when_every_item_is() {
    let item = |label: &str, enabled: bool| MenuItem {
        id: label.into(),
        label: label.into(),
        path: vec!["File".into(), "Export".into()],
        shortcut: None,
        enabled,
        checked: None,
        color: None,
    };
    let (off, on, sep) = (item("Off", false), item("On", true), item("---", true));
    assert!(!submenu_enabled(&[]));
    assert!(!submenu_enabled(&[&off, &sep]));
    assert!(submenu_enabled(&[&off, &on]));
}

#[test]
fn live_lookup_finds_engine_and_shell_commands() {
    for command in photocraft_engine::commands::command_specs() {
        assert!(is_live(command.id), "{}", command.id);
    }
    for &(id, ..) in UI_COMMANDS {
        assert!(is_live(id), "{id}");
    }
    for id in ["", "missing.command", "💾"] {
        assert!(!is_live(id));
    }
}

#[test]
fn select_menu_contains_every_selection_context_action_and_more() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 32, "height": 32})).unwrap();
    app.run("select.rect", json!({"x": 2, "y": 2, "width": 8, "height": 8})).unwrap();
    let items = menu_items(&app);
    let top: Vec<_> = items.iter().filter(|i| i.path.len() == 1 && i.path.first().is_some_and(|p| p == "Select")).collect();
    // The context menus' Select actions live somewhere under Select (Feather stays in Select >
    // Modify, as in the reference menus). Make Work Path is a Paths panel action.
    let select: Vec<_> = items.iter().filter(|i| i.path.first().is_some_and(|p| p == "Select")).collect();
    let rows = crate::canvas_tool_menu::SELECTION_MENU.iter().chain(crate::canvas_tool_menu::NO_SELECTION_MENU).flatten();
    for &(label, id) in rows.filter(|(_, id)| id.starts_with("select.") && *id != "select.toWorkPath") {
        assert!(select.iter().any(|i| i.id == id), "Select menu missing {label} ({id})");
    }
    assert!(top.iter().any(|i| i.id == "select.all"));
    assert!(top.iter().any(|i| i.id == "select.colorRange"));
    assert!(
        items.iter().any(|i| i.id == "select.modify.feather" && i.path.iter().map(String::as_str).eq(["Select", "Modify"])),
        "keep the Photoshop Modify route"
    );
    assert!(!top.iter().any(|i| i.id == "select.modify.feather"), "no extra top-level Feather");
}

/// Color to Alpha (#1576) has no Photoshop counterpart: it joins Photoshop's own Filter ›
/// Other submenu (not a second "Other") right after Offset, enabled on a pixel layer.
#[test]
fn color_to_alpha_sits_in_filter_other_after_offset() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 32, "height": 32})).unwrap();
    let items = menu_items(&app);
    let at = |id: &str| items.iter().position(|i| i.id == id).unwrap_or_else(|| panic!("{id} missing"));
    let (offset, c2a) = (at("filter.other.offset"), at("filter.other.colorToAlpha"));
    assert_eq!(c2a, offset + 1);
    assert_eq!(items[c2a].path, items[offset].path);
    assert_eq!(items[c2a].label, "Color to Alpha…");
    assert!(items[c2a].enabled, "the Background becomes a layer when the filter runs");
    let others: Vec<_> = items.iter().filter(|i| i.path.first().is_some_and(|p| p == "Filter") && i.path.len() == 2).map(|i| &i.path[1]).collect();
    assert!(!others.iter().any(|p| p.as_str() == "Other"), "no duplicate Other submenu: {others:?}");
}

#[test]
fn menu_bar_labels_have_horizontal_padding_and_open_menus() {
    use egui_kittest::{Harness, kittest::Queryable};

    for theme in crate::theme::ThemeKind::ALL {
        for width in [640.0, 1440.0] {
            let app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
            let mut harness = Harness::builder().with_size(egui::vec2(width, 200.0)).build_ui_state(
                |ui, app| {
                    menu_bar(app, ui);
                },
                app,
            );
            PhotocraftApp::setup_context(&harness.ctx, theme);
            harness.run_steps(3);

            let font = egui::TextStyle::Button.resolve(&harness.ctx.global_style());
            let mut right = 0.0;
            for label in TOP_MENUS {
                let rect = harness.get_by_label(label).rect();
                let text_width = harness.ctx.fonts_mut(|fonts| fonts.layout_no_wrap(label.into(), font.clone(), egui::Color32::WHITE).size().x);
                assert!(rect.width() >= text_width + 11.5, "{theme:?}: {label} lacks horizontal padding");
                assert!(rect.left() >= right && rect.right() <= width, "{theme:?}: {label} overlaps or overflows at width {width}");
                right = rect.right();
            }

            harness.get_by_label("File").click();
            harness.run_steps(3);
            assert!(harness.query_by_label_contains("Open…").is_some(), "{theme:?}: File menu did not open");
        }
    }
}

#[test]
fn overlapped_menu_title_does_not_receive_hover_switching() {
    let ctx = egui::Context::default();
    let mut title: Option<egui::Response> = None;
    let p = egui::pos2(90.0, 12.0);
    let mut reaches = true;
    // The popup area becomes hit-testable once egui has laid it out, so draw a few frames.
    for _ in 0..3 {
        let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(300.0, 200.0))), ..Default::default() };
        let mut out = ctx.run_ui(raw, |ui| {
            let ctx = &ui.ctx().clone();
            egui::CentralPanel::default().show(ui, |ui| {
                let (rect, _) = ui.allocate_exact_size(egui::vec2(80.0, 24.0), egui::Sense::hover());
                let shifted = rect.translate(egui::vec2(p.x - rect.center().x, p.y - rect.center().y));
                let response = ui.interact(shifted, egui::Id::unique("menu-title-test"), egui::Sense::hover());
                title = Some(response);
            });
            egui::Area::new(egui::Id::unique("overlapping-popup-test")).order(egui::Order::Foreground).fixed_pos(p - egui::vec2(10.0, 10.0)).show(ctx, |ui| {
                ui.allocate_space(egui::vec2(20.0, 20.0));
            });
            let response = title.as_ref().unwrap();
            reaches = pointer_reaches_title(ctx, response, p);
        });
        out.textures_delta.clear();
    }
    assert!(!reaches);
}

/// #7702: a pointer at the very top edge of the screen (the window maximized) moving between
/// titles switches the open menu, as it does a little lower.
#[test]
fn hovering_a_title_at_the_screen_edge_switches_the_open_menu() {
    use egui_kittest::{Harness, kittest::Queryable};

    let app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let mut harness = Harness::builder().with_size(egui::vec2(1200.0, 700.0)).build_ui_state(
        |ui, app| {
            egui::Frame::NONE.inner_margin(egui::Margin { top: 4, ..Default::default() }).show(ui, |ui| {
                menu_bar(app, ui);
            });
        },
        app,
    );
    PhotocraftApp::setup_context(&harness.ctx, crate::theme::ThemeKind::ALL[0]);
    harness.run_steps(3);
    harness.get_by_label("File").click();
    harness.run_steps(3);
    assert!(harness.query_by_label_contains("Open…").is_some());
    let image = harness.get_by_label("Image").rect();
    assert!(image.top() > 0.0, "the title sits below the screen edge");
    harness.hover_at(egui::pos2(image.center().x, 0.0));
    harness.run_steps(4);
    assert!(harness.query_by_label_contains("Open…").is_none(), "File menu should close");
    assert!(harness.query_by_label_contains("Duplicate…").is_some(), "Image menu should open from the screen edge");
}

#[test]
fn hovering_another_title_switches_the_open_menu() {
    use egui_kittest::{Harness, kittest::Queryable};

    let app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let mut harness = Harness::builder().with_size(egui::vec2(1200.0, 700.0)).build_ui_state(
        |ui, app| {
            menu_bar(app, ui);
        },
        app,
    );
    PhotocraftApp::setup_context(&harness.ctx, crate::theme::ThemeKind::ALL[0]);
    harness.run_steps(3);
    // Nothing open: hovering a title does not open it.
    harness.get_by_label("Image").hover();
    harness.run_steps(3);
    assert!(harness.query_by_label_contains("Open…").is_none() && harness.query_by_label_contains("Duplicate…").is_none());
    harness.get_by_label("File").click();
    harness.run_steps(3);
    assert!(harness.query_by_label_contains("Open…").is_some());
    harness.get_by_label("Image").hover();
    harness.run_steps(4);
    assert!(harness.query_by_label_contains("Open…").is_none(), "File menu should close");
    assert!(harness.query_by_label_contains("Duplicate…").is_some(), "Image menu should open on hover");
}

/// An open menu reuses its items between frames (building them costs far more than a frame on
/// a document with thousands of layers), rebuilds them when the state changes under it, and
/// keeps nothing once it closes.
#[test]
fn an_open_menu_reuses_its_items_until_the_state_changes() {
    use egui_kittest::{Harness, kittest::Queryable};

    let app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let mut harness = Harness::builder().with_size(egui::vec2(1200.0, 700.0)).build_ui_state(
        |ui, app| {
            menu_bar(app, ui);
        },
        app,
    );
    PhotocraftApp::setup_context(&harness.ctx, crate::theme::ThemeKind::ALL[0]);
    harness.run_steps(3);
    assert!(harness.state().menu_cache.is_none(), "no menu open, nothing built");
    harness.get_by_label("File").click();
    harness.run_steps(3);
    // The same allocation frame after frame: the items were not rebuilt.
    let built = |app: &PhotocraftApp| app.menu_cache.as_ref().map(|c| c.items.as_ptr());
    let close = |app: &PhotocraftApp| app.menu_cache.as_ref().and_then(|c| c.items.iter().find(|i| i.id == "file.close")).map(|i| i.enabled);
    let first = built(harness.state());
    assert!(first.is_some());
    assert_eq!(close(harness.state()), Some(false));
    harness.run_steps(3);
    assert_eq!(built(harness.state()), first);
    // A document opened under the open menu (a job ending, an agent): the items follow.
    harness.state_mut().run("file.new", json!({"width": 8, "height": 8})).unwrap();
    harness.run_steps(1);
    assert_eq!(close(harness.state()), Some(true));
    harness.key_press(egui::Key::Escape);
    harness.run_steps(3);
    assert!(harness.state().menu_cache.is_none(), "closed, nothing kept");
}

#[test]
fn help_search_finds_commands_by_any_word() {
    let app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let items = menu_items(&app);
    let ids = |q: &str| search_items(&items, q, crate::i18n::Lang::EN).iter().map(|i| i.id.clone()).collect::<Vec<_>>();
    let di = ids("DI");
    assert!(di.contains(&"edit.transform.distort".to_string()), "{di:?}");
    assert!(di.len() <= HELP_SEARCH_MAX);
    // A word inside the label matches too, and a label match outranks a path-only match.
    assert!(ids("selection").contains(&"select.transformSelection".to_string()));
    assert_eq!(search_rank("dis", "Distort", &[]), Some(0));
    assert_eq!(search_rank("sel", "Transform Selection", &[]), Some(1));
    assert_eq!(search_rank("orm", "Transform Selection", &[]), Some(2));
    assert_eq!(search_rank("trans", "Distort", &["Edit".into(), "Transform".into()]), Some(3));
    assert!(ids("   ").is_empty());
    assert!(ids("zzzzqqq").is_empty());
}

/// In another UI language both the translated and the English names match.
#[test]
fn help_search_matches_the_ui_language_and_english() {
    let app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let items = menu_items(&app);
    let Some(fr) = crate::i18n::Lang::from_code("fr") else { return };
    let ids = |q: &str| search_items(&items, q, fr).iter().map(|i| i.id.clone()).collect::<Vec<_>>();
    assert!(!ids(&crate::i18n::tr(fr, "Layer").to_lowercase()).is_empty(), "French menu name");
    assert!(ids("distort").contains(&"edit.transform.distort".to_string()), "English still matches");
    assert!(ids("zzzzqqq").is_empty());
}

/// #1447, #1438: a click on Help leaves its search field focused, so typing searches at once,
/// and clicking into the field and typing keeps the menu open. Whole app, real clicks (press
/// and release in separate frames), with the custom title bar (Windows, Linux) and without.
#[test]
fn help_search_takes_typing_and_clicks_without_closing_the_menu() {
    use egui::{Event, Modifiers, PointerButton};
    use egui_kittest::{Harness, kittest::Queryable};
    for (custom, doc, scale) in [(true, false, 1.0), (true, true, 1.25), (false, true, 1.5)] {
        let what = format!("custom title bar {custom}, document {doc}, scale {scale}");
        let mut h =
            Harness::builder().with_size(egui::vec2(1200.0 / scale, 800.0 / scale)).with_pixels_per_point(scale).with_max_steps(64).build_eframe(move |cc| {
                PhotocraftApp::setup_context(&cc.egui_ctx, crate::theme::ThemeKind::ALL[0]);
                let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
                app.custom_titlebar = custom;
                if doc {
                    app.run("file.new", json!({"width": 64, "height": 48})).unwrap();
                }
                app
            });
        h.run_steps(4);
        let click = |h: &mut Harness<'_, PhotocraftApp>, at: egui::Pos2| {
            h.event(Event::PointerMoved(at));
            h.run_steps(1);
            h.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
            h.run_steps(1);
            h.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
            h.run_steps(3);
        };
        let typing = |h: &mut Harness<'_, PhotocraftApp>, text: &str| {
            for c in text.chars() {
                h.event(Event::Text(c.to_string()));
                h.run_steps(1);
            }
            h.run_steps(3);
        };
        let open = |h: &Harness<'_, PhotocraftApp>| h.query_by_label_contains("About PhotoCraft").is_some();
        let field_id = egui::Id::unique("help-menu-search").with("field");
        let help = h.get_by_label("Help").rect().center();
        click(&mut h, help);
        assert!(open(&h), "Help opened ({what})");
        assert_eq!(h.ctx.memory(|m| m.focused()), Some(field_id), "the click on Help leaves the search field focused ({what})");
        typing(&mut h, "lev");
        assert!(open(&h), "typing keeps Help open ({what})");
        assert!(h.query_all_by_label_contains("› Levels").next().is_some(), "typing right away searches ({what})");
        let field = h.query_all_by_role(egui::accesskit::Role::TextInput).find(|n| n.is_focused()).map(|n| n.rect().center());
        let field = field.unwrap_or_else(|| panic!("the search field is in the accessibility tree ({what})"));
        click(&mut h, field);
        assert!(open(&h), "clicking the search field keeps Help open ({what})");
        assert_eq!(h.ctx.memory(|m| m.focused()), Some(field_id), "and focused ({what})");
        typing(&mut h, "els");
        assert!(open(&h), "typing after the click keeps Help open ({what})");
        assert!(h.query_all_by_label_contains("› Levels").next().is_some(), "and lists the matches ({what})");
    }
}

/// #5996: a real key press comes as a key and its text. A letter typed into Help › Search is
/// text for the field, not a mnemonic: "S" ran System Info and "A" the ArtCraft website.
#[test]
fn help_search_letters_are_not_mnemonics() {
    use egui::{Event, Key, Modifiers, PointerButton};
    use egui_kittest::{Harness, kittest::Queryable};
    let mut h = Harness::builder().with_size(egui::vec2(1200.0, 800.0)).with_max_steps(64).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, crate::theme::ThemeKind::ALL[0]);
        PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default())
    });
    h.run_steps(4);
    let help = h.get_by_label("Help").rect().center();
    h.event(Event::PointerMoved(help));
    h.run_steps(1);
    for pressed in [true, false] {
        h.event(Event::PointerButton { pos: help, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
        h.run_steps(1);
    }
    h.run_steps(3);
    assert!(h.query_by_label_contains("About PhotoCraft").is_some(), "Help opened");
    for c in "DeSelect all".chars() {
        let key = Key::from_name(&c.to_ascii_uppercase().to_string());
        if let Some(key) = key {
            h.event(Event::Key { key, physical_key: Some(key), pressed: true, repeat: false, modifiers: Modifiers::NONE });
        }
        h.event(Event::Text(c.to_string()));
        h.run_steps(1);
        if let Some(key) = key {
            h.event(Event::Key { key, physical_key: Some(key), pressed: false, repeat: false, modifiers: Modifiers::NONE });
        }
    }
    h.run_steps(3);
    assert!(h.query_by_label_contains("About PhotoCraft").is_some(), "typing keeps Help open");
    let query: String = h.ctx.data(|d| d.get_temp(egui::Id::unique("help-menu-search"))).unwrap_or_default();
    assert_eq!(query, "DeSelect all", "every letter reached the field");
    assert!(h.state().ui.dialogs.is_empty(), "no menu command ran: {:?}", h.state().ui.dialogs);
}

#[test]
fn dropdown_items_have_room() {
    use egui_kittest::{Harness, kittest::Queryable};

    for theme in crate::theme::ThemeKind::ALL {
        let app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        let mut harness = Harness::builder().with_size(egui::vec2(1200.0, 900.0)).build_ui_state(
            |ui, app| {
                menu_bar(app, ui);
            },
            app,
        );
        PhotocraftApp::setup_context(&harness.ctx, theme);
        harness.run_steps(3);
        harness.get_by_label("File").click();
        harness.run_steps(3);
        let a = harness.get_by_label_contains("New…").rect();
        let b = harness.get_by_label_contains("Open…").rect();
        let font = egui::TextStyle::Button.resolve(&harness.ctx.global_style()).size;
        assert!(a.height() >= font + 8.0, "{theme:?}: item height {} for a {font} pt font", a.height());
        assert!(b.top() - a.top() >= font + 10.0, "{theme:?}: rows {} apart", b.top() - a.top());
        // #402: rows touch (no dialog spacing between them), so long menus fit the window.
        let next = harness.get_by_label_contains("New from Clipboard").rect();
        assert!((next.top() - a.bottom()).abs() < 0.5, "{theme:?}: {} pt between rows", next.top() - a.bottom());
        assert!(next.top() - a.top() <= 24.5, "{theme:?}: rows {} apart", next.top() - a.top());
    }
}

#[test]
fn menu_level_height_is_bounded_by_viewport() {
    use egui_kittest::Harness;

    let app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let mut harness = Harness::builder().with_size(egui::vec2(900.0, 240.0)).build_ui_state(
        |ui, app| {
            menu_bar(app, ui);
        },
        app,
    );
    PhotocraftApp::setup_context(&harness.ctx, crate::theme::ThemeKind::ALL[0]);
    harness.run_steps(3);
    // Opening a top-level menu with many entries must not grow its popup past the viewport.
    use egui_kittest::kittest::Queryable;
    harness.get_by_label("Filter").click();
    harness.run_steps(3);
    let viewport = harness.ctx.content_rect();
    let max_bottom = harness
        .ctx
        .memory(|m| m.areas().visible_layer_ids())
        .into_iter()
        .filter_map(|layer| harness.ctx.memory(|m| m.area_rect(layer.id)))
        .map(|r| r.bottom())
        .fold(viewport.top(), f32::max);
    assert!(max_bottom <= viewport.bottom() + 1.0, "menu popup overflowed viewport: {max_bottom} > {}", viewport.bottom());
}

#[test]
fn tall_menu_scrolls_with_mouse_wheel_on_short_display() {
    use egui_kittest::{Harness, kittest::Queryable};

    let app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let mut harness = Harness::builder().with_size(egui::vec2(900.0, 220.0)).build_ui_state(
        |ui, app| {
            menu_bar(app, ui);
        },
        app,
    );
    PhotocraftApp::setup_context(&harness.ctx, crate::theme::ThemeKind::ALL[0]);
    harness.run_steps(3);
    harness.get_by_label("Filter").click();
    harness.run_steps(3);

    let item = harness.get_by_label_contains("Filter Gallery…");
    let before = item.rect().top();
    harness.hover_at(item.rect().center());
    harness.event(egui::Event::MouseWheel {
        source: egui::MouseWheelSource::Unknown,
        unit: egui::MouseWheelUnit::Point,
        delta: egui::vec2(0.0, -80.0),
        phase: egui::TouchPhase::Move,
        modifiers: egui::Modifiers::NONE,
    });
    harness.run_steps(4);

    let after = harness.get_by_label_contains("Filter Gallery…").rect().top();
    assert!(after < before - 1.0, "mouse wheel should move tall menu content: {before} -> {after}");
}

#[test]
fn window_panel_and_workspace_ids_drive_the_shell() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let ctx = egui::Context::default();
    assert!(!app.ui.panels.history);
    invoke(&mut app, &ctx, "window.panel.history", Value::Null).unwrap();
    assert!(app.ui.panels.history);
    assert_eq!(checked(&app, "window.panel.history"), Some(true));
    invoke(&mut app, &ctx, "window.workspace.painting", Value::Null).unwrap();
    assert_eq!(app.ui.workspace, "Painting");
    assert!(!app.ui.panels.properties);
    assert_eq!(checked(&app, "window.workspace.painting"), Some(true));
    invoke(&mut app, &ctx, "window.workspace.essentials", Value::Null).unwrap();
    assert!(app.ui.panels.properties);
    // Live in the menu, and no duplicate "Layers" entry from the window.toggle.* commands.
    let items = menu_items(&app);
    assert!(items.iter().any(|i| i.id == "window.panel.layers" && i.enabled));
    assert_eq!(items.iter().filter(|i| i.path == ["Window"] && i.label == "Layers").count(), 1);
    assert!(items.iter().any(|i| i.id == "edit.stroke"));
}

#[test]
fn proof_setup_presets_and_checkmarks() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let ctx = egui::Context::default();
    app.run("file.new", json!({"width": 8, "height": 8})).unwrap();
    app.sync_views();
    assert_eq!(checked(&app, "view.proofColors"), Some(false));
    invoke(&mut app, &ctx, "view.proofSetup.workingCmyk", Value::Null).unwrap();
    assert_eq!(checked(&app, "view.proofColors"), Some(true));
    invoke(&mut app, &ctx, "view.gamutWarning", json!({"on": true})).unwrap();
    assert_eq!(checked(&app, "view.gamutWarning"), Some(true));
    let doc = app.session.active().unwrap().doc.clone();
    let lut = app.session.color.canvas_lut(&doc, 9).unwrap().unwrap();
    assert_eq!(lut.len(), 9 * 9 * 9 * 4);
    // Pure sRGB green is outside coated CMYK; mid grey is inside.
    let at = |r: usize, g: usize, b: usize| lut[(r + g * 9 + b * 81) * 4 + 3];
    assert_eq!((at(0, 8, 0), at(4, 4, 4)), (255, 0));
    // Custom… opens the generated Proof Setup dialog with a profile choice.
    assert!(invoke(&mut app, &ctx, "view.proofSetup.custom", Value::Null).unwrap()["dialog"].is_u64());
    let spec = photocraft_engine::commands::find("edit.convertToProfile").unwrap();
    let params = crate::filter_dialog::parse_spec(spec.params);
    assert!(matches!(&params[0].kind, crate::filter_dialog::Kind::Choice { options, .. } if options[0] == "srgb" && !options.iter().any(|v| v.contains('/'))));
}
