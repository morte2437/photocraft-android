#[test]
fn memory_usage_has_a_dedicated_explanation_and_keeps_its_value() {
    use egui_kittest::{Harness, kittest::Queryable};
    let values = json!({"memoryUsageMb":50000}).as_object().unwrap().clone();
    assert!(!field_visible("performance", &values, "memoryUsageMb", ""));
    let mut h = Harness::builder()
        .with_size(vec2(680.0, 350.0))
        .build_ui_state(|ui, values| section_fields(ui, "performance", values, &["memoryUsageMb".into()], crate::i18n::Lang::EN, None, ""), values);
    h.run_steps(2);
    h.get_by_label("Memory Usage");
    h.get_by_label("Per-document RAM limit (MB)");
    h.get_by_label("Limits each open document's pixels plus History. Oldest undo states are discarded when the limit is exceeded.");
    h.get_by_label("Applies immediately when saved. Memory is not reserved in advance.");
    assert_eq!(h.state()["memoryUsageMb"], 50000);
}

#[test]
fn the_generator_checkbox_reads_enable_generator() {
    // #9392: the reference application's Plug-ins page says "Enable Generator", not "Generator enabled".
    use egui_kittest::{Harness, kittest::Queryable};
    let values = serde_json::to_value(photocraft_engine::prefs::PlugIns::default()).unwrap().as_object().unwrap().clone();
    let keys: Vec<String> = values.keys().cloned().collect();
    let mut h = Harness::builder()
        .with_size(vec2(680.0, 350.0))
        .build_ui_state(move |ui, values| section_fields(ui, "plugIns", values, &keys, crate::i18n::Lang::EN, None, ""), values);
    h.run_steps(2);
    assert!(h.query_by_label("Generator enabled").is_none());
    h.get_by_label("Enable Generator").click();
    h.run_steps(2);
    assert_eq!(h.state()["generatorEnabled"], true);
    let de = crate::i18n::Lang::from_code("de").unwrap();
    assert_eq!(crate::i18n::tr(de, "Enable Generator"), "Generator aktivieren");
}

#[test]
fn graphics_dropdowns_are_named_for_their_settings() {
    use egui::accesskit::Role;
    use egui_kittest::Harness;
    use egui_kittest::kittest::{NodeT, Queryable};

    let obj = json!({"gpuBackend": "auto", "gpuAdapter": ""}).as_object().unwrap().clone();
    let mut h = Harness::builder()
        .with_size(vec2(700.0, 500.0))
        .with_context_setup(|ctx| PhotocraftApp::setup_context(ctx, crate::theme::ThemeKind::ALL[0]))
        .build_ui_state(|ui, obj| gpu_status_rows(ui, None, None, obj), obj);
    h.run_steps(2);
    h.get_by_label("Advanced").click();
    h.run_steps(2);
    let labels: Vec<String> = h.query_all_by_role(Role::ComboBox).filter_map(|n| n.accesskit_node().label()).collect();
    assert!(labels.iter().any(|label| label == "GPU Backend"), "{labels:?}");
    if cfg!(target_os = "linux") {
        assert!(labels.iter().any(|label| label == "GPU adapter"), "{labels:?}");
    }
    assert!(!labels.iter().any(|label| label == "Graphics"), "the section title names neither control: {labels:?}");
}

#[test]
fn colour_preferences_take_a_typed_hex_value() {
    // #668: the canvas colour (and every colour preference) showed its hex but couldn't take one.
    use egui::accesskit::Role;
    use egui_kittest::kittest::Queryable;
    let mut h = egui_kittest::Harness::new_ui_state(|ui, rgb: &mut [u8; 3]| super::hex_field(ui, "interface.canvasCustomColor", rgb), [0x28, 0x28, 0x28]);
    h.run();
    assert_eq!(h.get_by_role(Role::TextInput).value().as_deref(), Some("#282828"));
    h.get_by_role(Role::TextInput).click();
    h.run_steps(1);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.run_steps(1);
    for ch in "#80".chars() {
        h.event(egui::Event::Text(ch.to_string()));
        h.run_steps(1);
    }
    // A partial value stays as typed and leaves the colour alone.
    assert_eq!(h.get_by_role(Role::TextInput).value().as_deref(), Some("#80"));
    assert_eq!(*h.state(), [0x28, 0x28, 0x28]);
    for ch in "8080".chars() {
        h.event(egui::Event::Text(ch.to_string()));
        h.run_steps(1);
    }
    assert_eq!(*h.state(), [0x80, 0x80, 0x80]);
}

#[test]
fn quick_export_controls_match_selected_format() {
    let mut obj = serde_json::to_value(photocraft_engine::prefs::Export::default()).unwrap().as_object().unwrap().clone();
    assert!(!export_field_visible(&obj, "jpegQuality"));
    assert!(!export_field_visible(&obj, "webpLossless"));
    assert!(!export_field_visible(&obj, "webpQuality"));

    obj.insert("quickExportFormat".into(), json!("jpg"));
    assert!(export_field_visible(&obj, "jpegQuality"));
    assert!(!export_field_visible(&obj, "webpLossless"));
    assert!(!export_field_visible(&obj, "webpQuality"));

    obj.insert("quickExportFormat".into(), json!("webp"));
    assert!(!export_field_visible(&obj, "jpegQuality"));
    assert!(export_field_visible(&obj, "webpLossless"));
    assert!(!export_field_visible(&obj, "webpQuality"), "lossless WebP does not have a quality setting");

    obj.insert("webpLossless".into(), json!(false));
    assert!(export_field_visible(&obj, "webpQuality"));
    assert_eq!(humanize("webpLossless"), "Lossless");
    assert_eq!(humanize("webpQuality"), "Quality");
    obj.insert("quickExportFormat".into(), json!("exr"));
    for key in ["jpegQuality", "webpLossless", "webpQuality", "convertToSrgb", "metadata"] {
        assert!(!export_field_visible(&obj, key), "EXR must not offer ignored web settings: {key}");
    }
}

#[test]
fn the_nudge_pause_is_hidden_while_nudges_are_not_bundled() {
    let mut tools = Map::new();
    tools.insert("bundleNudges".into(), json!(true));
    assert!(tools_field_visible(&tools, "nudgeBundlePauseMs"));
    assert!(tools_field_visible(&tools, "bundleNudges"));
    tools.insert("bundleNudges".into(), json!(false));
    assert!(!tools_field_visible(&tools, "nudgeBundlePauseMs"));
    assert!(tools_field_visible(&tools, "bundleNudges") && tools_field_visible(&tools, "showTooltips"));
    // A section without the flag (an older preferences object) shows the field.
    assert!(tools_field_visible(&Map::new(), "nudgeBundlePauseMs"));
    assert_eq!(humanize("bundleNudges"), "Bundle Arrow-Key Nudges");
    assert_eq!(humanize("nudgeBundlePauseMs"), "Nudge Bundle Pause (ms)");
}

#[test]
fn rendering_mode_display_respects_explicit_mode_and_legacy_disable() {
    let legacy = serde_json::json!({"useGpu": false, "gpuBackend": "auto"});
    assert_eq!(super::rendering_mode_value(legacy.as_object().unwrap()), "cpu");
    let explicit = serde_json::json!({"renderingMode": "gpu", "useGpu": false, "gpuBackend": "cpu"});
    assert_eq!(super::rendering_mode_value(explicit.as_object().unwrap()), "gpu");
    let automatic = serde_json::json!({"renderingMode": null, "useGpu": true, "gpuBackend": "auto"});
    assert_eq!(super::rendering_mode_value(automatic.as_object().unwrap()), "auto");
}

#[test]
fn preferences_search_matches_editable_fields_across_sections() {
    let values = prefs::Preferences::default().to_json();
    let contains = |needle: &str, section: &str| preference_sections(&values, needle).iter().any(|(id, _)| *id == section);
    assert!(contains("UI Scale", "interface"));
    assert!(contains("placeholder", "type"));
    assert!(contains("performance", "performance"));
    assert!(contains("interface.uiScale", "interface"));
    assert!(preference_sections(&values, "this setting does not exist").is_empty());
    assert_eq!(preference_sections(&values, "").len(), SECTIONS.len());
}

#[test]
fn preferences_dialog_width_follows_its_resizable_area() {
    let prefs = serde_json::json!({"__prefsui": "prefs"});
    let prefs = prefs.as_object().unwrap();
    assert_eq!(super::width(prefs, 200.0), Some(380.0));
    assert_eq!(super::width(prefs, 800.0), Some(800.0));
    // A dialog resized wider than it opens is filled, not left with empty space (#7420).
    assert_eq!(super::width(prefs, 3000.0), Some(3000.0));
    let shortcuts = serde_json::json!({"__prefsui": "shortcuts"});
    assert_eq!(super::width(shortcuts.as_object().unwrap(), 3000.0), Some(720.0));
}

#[test]
fn preferences_settings_fill_the_dialog_down_to_the_buttons() {
    // The settings area plus the button row (spacing, 8 pt gap, buttons) stays inside what the
    // resizable area gives it, at any size: the area grows to fit its content, so asking for
    // more would make the dialog grow every frame. The area follows the dialog's size (#7420).
    for available in [300.0, 600.0, 1400.0] {
        let h = super::settings_height(available, 4.0, 30.0);
        assert!(h + 4.0 + 8.0 + 30.0 <= available, "{available}");
        assert!(h + 4.0 + 8.0 + 30.0 + 8.0 >= available, "{available}");
    }
    assert_eq!(super::settings_height(1400.0, 4.0, 30.0) - super::settings_height(600.0, 4.0, 30.0), 800.0);
    assert_eq!(super::settings_height(50.0, 4.0, 30.0), 160.0);
}
use super::*;
use std::sync::{Arc, Mutex};

#[test]
fn appearance_resolves_both_saved_themes_and_follows_system() {
    let mut interface = prefs::Interface { appearance_mode: AppearanceMode::Auto, ..prefs::Interface::default() };
    assert_eq!(selected_theme(&interface, Some(egui::Theme::Dark)), ThemeKind::ProMedium);
    assert_eq!(selected_theme(&interface, Some(egui::Theme::Light)), ThemeKind::ProLight);
    interface.dark_theme = DarkTheme::Studio;
    interface.light_theme = LightTheme::Classic;
    assert_eq!(selected_theme(&interface, Some(egui::Theme::Dark)), ThemeKind::Studio);
    assert_eq!(selected_theme(&interface, Some(egui::Theme::Light)), ThemeKind::Classic);
    interface.appearance_mode = AppearanceMode::Dark;
    assert_eq!(selected_theme(&interface, Some(egui::Theme::Light)), ThemeKind::Studio);
    interface.appearance_mode = AppearanceMode::Light;
    assert_eq!(selected_theme(&interface, Some(egui::Theme::Dark)), ThemeKind::Classic);
}

#[test]
fn breeze_themes_are_chosen_by_preference_and_fix_the_matching_mode() {
    let ctx = egui::Context::default();
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    for (id, kind, mode) in [
        ("breezeLight", ThemeKind::BreezeLight, AppearanceMode::Light),
        ("breezeDark", ThemeKind::BreezeDark, AppearanceMode::Dark),
        ("softLight", ThemeKind::SoftLight, AppearanceMode::Light),
        ("softDark", ThemeKind::SoftDark, AppearanceMode::Dark),
    ] {
        app.run("prefs.set", json!({"path": "interface.theme", "value": id})).unwrap();
        tick(&mut app, &ctx);
        let interface = app.session.prefs().interface.clone();
        assert_eq!(interface.appearance_mode, mode, "{id}");
        assert_eq!(selected_theme(&interface, None), kind, "{id}");
        assert_eq!(app.ui.theme, kind, "{id}");
    }
    // The two slots are independent: choosing the dark one left the light choice alone.
    assert_eq!(app.session.prefs().interface.light_theme, LightTheme::SoftLight);
    assert_eq!(app.session.prefs().interface.dark_theme, DarkTheme::SoftDark);
}

#[test]
fn auto_uses_native_system_theme_when_egui_does_not_report_one() {
    let (mut app, _) = app_with_store();
    app.services.system_theme = Some(Box::new(|_| Some(egui::Theme::Light)));
    app.run("prefs.set", json!({"path": "interface.appearanceMode", "value": "auto"})).unwrap();
    app.run("prefs.set", json!({"path": "interface.appearanceMode", "value": "auto"})).unwrap();
    let ctx = egui::Context::default();
    tick(&mut app, &ctx);
    assert_eq!(app.ui.theme, ThemeKind::ProLight);
}

#[test]
fn appearance_button_cycles_modes_without_losing_theme_choices() {
    let ctx = egui::Context::default();
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("prefs.set", json!({"values": {"interface.darkTheme": "studio", "interface.lightTheme": "classic"}})).unwrap();
    for (mode, visible) in [(AppearanceMode::Auto, ThemeKind::Studio), (AppearanceMode::Light, ThemeKind::Classic), (AppearanceMode::Dark, ThemeKind::Studio)] {
        cycle_appearance(&mut app, &ctx);
        assert_eq!(app.session.prefs().interface.appearance_mode, mode);
        assert_eq!(app.ui.theme, visible);
        assert_eq!(app.session.prefs().interface.dark_theme, DarkTheme::Studio);
        assert_eq!(app.session.prefs().interface.light_theme, LightTheme::Classic);
    }
}

#[test]
fn darken_and_lighten_interface_step_through_photoshops_brightnesses_and_stop_at_the_ends() {
    let ctx = egui::Context::default();
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    sync_appearance(&mut app, &ctx);
    assert_eq!(app.ui.theme, ThemeKind::ProMedium, "Photoshop's default brightness");
    let press = |app: &mut PhotocraftApp, id: &str| crate::menus::invoke(app, &ctx, id, json!({})).unwrap();
    for expected in [ThemeKind::ProLightGray, ThemeKind::ProLight, ThemeKind::ProLight] {
        press(&mut app, "view.lightenInterface");
        assert_eq!(app.ui.theme, expected);
    }
    // Saved like a choice in Preferences › Interface: the light slot and the Light appearance.
    let i = &app.session.prefs().interface;
    assert_eq!((i.appearance_mode, i.light_theme), (AppearanceMode::Light, LightTheme::ProLight));
    for expected in [ThemeKind::ProLightGray, ThemeKind::ProMedium, ThemeKind::Pro, ThemeKind::Pro] {
        press(&mut app, "view.darkenInterface");
        assert_eq!(app.ui.theme, expected);
    }
    let i = &app.session.prefs().interface;
    assert_eq!((i.appearance_mode, i.dark_theme), (AppearanceMode::Dark, DarkTheme::Pro));
    // Another theme steps from the nearest brightness.
    for (theme, id, expected) in [
        ("studio", "view.darkenInterface", ThemeKind::Pro),
        ("studio", "view.lightenInterface", ThemeKind::ProLightGray),
        ("classic", "view.darkenInterface", ThemeKind::ProMedium),
        ("classic", "view.lightenInterface", ThemeKind::ProLight),
    ] {
        app.run("prefs.set", json!({"path": "interface.theme", "value": theme})).unwrap();
        sync_appearance(&mut app, &ctx);
        press(&mut app, id);
        assert_eq!(app.ui.theme, expected, "{theme} {id}");
    }
}

#[test]
fn ctrl_shift_f1_and_f2_darken_and_lighten_the_interface() {
    for (id, shortcut) in [("view.darkenInterface", "Cmd+Shift+F1"), ("view.lightenInterface", "Cmd+Shift+F2")] {
        assert_eq!(crate::shortcuts::default_shortcut(id).as_deref(), Some(shortcut), "{id}");
    }
    let ctx = egui::Context::default();
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    sync_appearance(&mut app, &ctx);
    let press = |app: &mut PhotocraftApp, key| {
        let modifiers = egui::Modifiers::COMMAND | egui::Modifiers::SHIFT;
        let raw = egui::RawInput {
            events: vec![egui::Event::ModifiersChanged(modifiers), egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers }],
            ..Default::default()
        };
        ctx.begin_pass(raw);
        crate::shortcuts::handle(app, &ctx);
        ctx.end_pass().textures_delta.clear();
    };
    press(&mut app, egui::Key::F1);
    assert_eq!(app.ui.theme, ThemeKind::Pro);
    press(&mut app, egui::Key::F2);
    press(&mut app, egui::Key::F2);
    assert_eq!(app.ui.theme, ThemeKind::ProLightGray);
}

fn appearance_frame(app: &mut PhotocraftApp, ctx: &egui::Context, appearance: Option<egui::Theme>) {
    ctx.run_ui(egui::RawInput { system_theme: appearance, ..Default::default() }, |ui| tick(app, ui.ctx())).textures_delta.clear();
}

fn assert_palette(ctx: &egui::Context, kind: ThemeKind) {
    let t = Tokens::get(ctx);
    assert_eq!(t.kind, kind);
    for style in [ctx.global_style(), ctx.style_of(egui::Theme::Light), ctx.style_of(egui::Theme::Dark)] {
        assert_eq!(style.visuals.panel_fill, t.chrome);
        assert_eq!(style.visuals.window_fill, t.card);
        assert_eq!(style.visuals.override_text_color, Some(t.text));
        assert_eq!(style.visuals.widgets.inactive.bg_fill, t.field);
        assert_eq!(style.visuals.dark_mode, t.dark());
        assert_eq!(style.text_styles[&egui::TextStyle::Body].size, if t.pro { 12.0 } else { 12.5 });
    }
    assert_eq!(ctx.options(|o| o.theme_preference), egui::ThemePreference::System);
}

#[test]
fn auto_appearance_follows_live_changes_without_saving_resolved_palettes() {
    for (dark, light) in [(ThemeKind::ProMedium, ThemeKind::StudioLight), (ThemeKind::SolarizedDark, ThemeKind::Adwaita)] {
        let (mut app, store) = app_with_store();
        let ctx = egui::Context::default();
        PhotocraftApp::setup_context(&ctx, Default::default());
        app.run("prefs.set", json!({"values": {"interface.appearanceMode": "auto", "interface.darkTheme": dark.id(), "interface.lightTheme": light.id()}}))
            .unwrap();
        appearance_frame(&mut app, &ctx, Some(egui::Theme::Light));
        let revision = app.session.prefs.rev();
        let interface = app.session.prefs().interface.clone();
        let saved = store.lock().unwrap().clone().unwrap();
        assert_eq!(stored(&store)["interface"]["appearanceMode"], "auto");
        let gpu_style = canvas_style(&app);
        for (appearance, expected) in [(Some(egui::Theme::Light), light), (Some(egui::Theme::Dark), dark), (Some(egui::Theme::Light), light), (None, dark)] {
            appearance_frame(&mut app, &ctx, appearance);
            assert_palette(&ctx, expected);
            assert_eq!(app.ui.theme, expected);
            assert_eq!(app.session.prefs().interface, interface);
            assert_eq!(app.session.prefs.rev(), revision);
            assert_eq!(store.lock().unwrap().as_ref(), Some(&saved));
            assert_eq!(canvas_style(&app), gpu_style);
            assert_eq!(pasteboard_color(&app), None);
        }
        // A fresh launch resolves its saved Auto choice against the new OS input.
        for (appearance, expected) in [(Some(egui::Theme::Light), light), (Some(egui::Theme::Dark), dark), (None, dark)] {
            let (mut restarted, store) = app_with_saved(Some(saved.clone()));
            let fresh = egui::Context::default();
            PhotocraftApp::setup_context(&fresh, Default::default());
            appearance_frame(&mut restarted, &fresh, appearance);
            assert_palette(&fresh, expected);
            assert_eq!(restarted.ui.theme, expected);
            assert_eq!(restarted.session.prefs().interface, interface);
            assert_eq!(store.lock().unwrap().as_ref(), Some(&saved));
        }
        app.run("prefs.set", json!({"values": {"interface.canvasColor": "custom", "interface.canvasCustomColor": "#123456"}})).unwrap();
        for appearance in [Some(egui::Theme::Light), Some(egui::Theme::Dark)] {
            appearance_frame(&mut app, &ctx, appearance);
            assert_eq!(pasteboard_color(&app), Some(Color32::from_rgb(0x12, 0x34, 0x56)));
            assert_eq!(canvas_style(&app), gpu_style);
        }
    }
}

#[test]
fn manual_theme_tokens_and_global_styles_stay_fixed_across_os_changes() {
    let (mut app, store) = app_with_store();
    let ctx = egui::Context::default();
    PhotocraftApp::setup_context(&ctx, Default::default());
    for kind in ThemeKind::ALL {
        app.run("prefs.set", json!({"path": "interface.theme", "value": kind.id()})).unwrap();
        appearance_frame(&mut app, &ctx, Some(egui::Theme::Dark));
        let interface = app.session.prefs().interface.clone();
        let revision = app.session.prefs.rev();
        let saved = store.lock().unwrap().clone().unwrap();
        assert_eq!(interface.appearance_mode, if Tokens::for_kind(kind).dark() { AppearanceMode::Dark } else { AppearanceMode::Light });
        for appearance in [Some(egui::Theme::Dark), Some(egui::Theme::Light), Some(egui::Theme::Dark), None] {
            appearance_frame(&mut app, &ctx, appearance);
            assert_palette(&ctx, kind);
            assert_eq!(Tokens::get(&ctx), Tokens::for_kind(kind));
            assert_eq!(app.ui.theme, kind);
            assert_eq!(app.session.prefs().interface, interface);
            assert_eq!(app.session.prefs.rev(), revision);
            assert_eq!(store.lock().unwrap().as_ref(), Some(&saved));
        }
    }
}

#[test]
fn auto_appearance_replaces_a_restored_egui_dark_pin() {
    let (mut app, _) = app_with_saved(Some(json!({"interface": {"appearanceMode": "auto"}}).to_string()));
    let ctx = egui::Context::default();
    ctx.set_theme(egui::ThemePreference::Dark);
    assert_eq!(ctx.options(|o| o.theme_preference), egui::ThemePreference::Dark);
    appearance_frame(&mut app, &ctx, Some(egui::Theme::Light));
    assert_palette(&ctx, ThemeKind::ProLight);
    assert_eq!(app.session.prefs().interface.appearance_mode, AppearanceMode::Auto);
    // A restored pin must also clear when the concrete PhotoCraft palette stays the same.
    ctx.set_theme(egui::ThemePreference::Dark);
    appearance_frame(&mut app, &ctx, Some(egui::Theme::Light));
    assert_eq!(ctx.options(|o| o.theme_preference), egui::ThemePreference::System);
    assert_palette(&ctx, ThemeKind::ProLight);
    appearance_frame(&mut app, &ctx, Some(egui::Theme::Dark));
    assert_palette(&ctx, ThemeKind::ProMedium);
    assert_eq!(app.session.prefs().interface.appearance_mode, AppearanceMode::Auto);
}

#[test]
fn system_theme_menu_choice_stays_checked_as_the_appearance_changes() {
    let (mut app, store) = app_with_store();
    let ctx = egui::Context::default();
    PhotocraftApp::setup_context(&ctx, Default::default());
    app.run("prefs.set", json!({"values": {"interface.darkTheme": "solarizedDark", "interface.lightTheme": "adwaita"}})).unwrap();
    appearance_frame(&mut app, &ctx, Some(egui::Theme::Light));
    crate::menus::invoke(&mut app, &ctx, "window.theme.system", json!({})).unwrap();
    appearance_frame(&mut app, &ctx, Some(egui::Theme::Light));
    let interface = app.session.prefs().interface.clone();
    let saved = store.lock().unwrap().clone().unwrap();
    let revision = app.session.prefs.rev();
    for appearance in [Some(egui::Theme::Light), Some(egui::Theme::Dark), None] {
        appearance_frame(&mut app, &ctx, appearance);
        let items = crate::menus::menu_items(&app);
        let item = |id: &str| items.iter().find(|i| i.id == id).unwrap();
        assert_eq!(item("window.theme.system").checked, Some(true));
        assert!(item("window.theme.system").enabled);
        assert_eq!(item("window.theme.system").path, ["Window", "Theme"]);
        for kind in ThemeKind::ALL {
            assert_eq!(item(&format!("window.theme.{}", kind.id())).checked, Some(false));
        }
        assert_eq!(app.session.prefs().interface.appearance_mode, AppearanceMode::Auto);
        assert_eq!(app.session.prefs().interface, interface);
        assert_eq!(app.session.prefs.rev(), revision);
        assert_eq!(store.lock().unwrap().as_ref(), Some(&saved));
    }
    crate::menus::invoke(&mut app, &ctx, "window.theme.solarizedDark", json!({})).unwrap();
    for appearance in [Some(egui::Theme::Light), Some(egui::Theme::Dark)] {
        appearance_frame(&mut app, &ctx, appearance);
        assert_palette(&ctx, ThemeKind::SolarizedDark);
        let items = crate::menus::menu_items(&app);
        let item = |id: &str| items.iter().find(|i| i.id == id).unwrap();
        assert_eq!(item("window.theme.solarizedDark").checked, Some(true));
        assert_eq!(item("window.theme.system").checked, Some(false));
        assert_eq!(stored(&store)["interface"]["appearanceMode"], "dark");
        assert_eq!(app.session.prefs().interface.light_theme, LightTheme::Adwaita);
    }
}

#[test]
fn preferences_interface_offers_and_applies_sync_with_system() {
    use egui_kittest::{Harness, kittest::Queryable};
    let (mut app, store) = app_with_store();
    app.run("prefs.set", json!({"values": {"interface.language": "en", "interface.darkTheme": "solarizedDark", "interface.lightTheme": "adwaita"}})).unwrap();
    let interface = app.session.prefs().interface.clone();
    open_preferences(&mut app, "interface");
    let mut h = Harness::builder().with_size(vec2(1280.0, 800.0)).build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        app
    });
    h.input_mut().system_theme = Some(egui::Theme::Light);
    h.run_steps(4);
    for label in ["Adwaita", "Adwaita dark", "Solarized dark"] {
        assert!(h.query_by_label(label).is_some(), "missing theme choice {label}");
    }
    h.get_by_value("Dark").click();
    h.run_steps(2);
    h.get_by_label("Sync with system").click();
    h.run_steps(2);
    h.get_by_label("Apply").click();
    h.run_steps(4);
    assert_eq!(h.state().session.prefs().interface.appearance_mode, AppearanceMode::Auto);
    assert_eq!(h.state().session.prefs().interface.dark_theme, interface.dark_theme);
    assert_eq!(h.state().session.prefs().interface.light_theme, interface.light_theme);
    assert_eq!(h.state().session.prefs().interface.theme, interface.theme);
    assert_palette(&h.ctx, ThemeKind::Adwaita);
    assert_eq!(stored(&store)["interface"]["appearanceMode"], "auto");
    let saved = store.lock().unwrap().clone().unwrap();
    let (mut restarted, reloaded) = app_with_saved(Some(saved.clone()));
    let fresh = egui::Context::default();
    PhotocraftApp::setup_context(&fresh, Default::default());
    appearance_frame(&mut restarted, &fresh, Some(egui::Theme::Dark));
    assert_palette(&fresh, ThemeKind::SolarizedDark);
    assert_eq!(restarted.session.prefs().interface.appearance_mode, AppearanceMode::Auto);
    assert_eq!(restarted.session.prefs().interface.dark_theme, interface.dark_theme);
    assert_eq!(restarted.session.prefs().interface.light_theme, interface.light_theme);
    assert_eq!(restarted.session.prefs().interface.theme, interface.theme);
    assert_eq!(reloaded.lock().unwrap().as_ref(), Some(&saved));
}

#[test]
fn theme_star_saves_favorite_and_menu_groups_it() {
    use egui_kittest::{Harness, kittest::Queryable};
    let (mut app, _) = app_with_store();
    open_preferences(&mut app, "interface");
    let mut h = Harness::builder().with_size(vec2(1280.0, 800.0)).build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        app
    });
    h.run_steps(4);
    h.get_by_label("Classic").hover();
    h.run_steps(2);
    h.get_by_label("Add to favorites: Classic").click();
    h.run_steps(2);
    h.get_by_label("Apply").click();
    h.run_steps(2);
    assert_eq!(h.state().session.prefs().interface.favorite_themes, vec![SavedTheme::Classic]);
    assert!(h.query_by_label("Remove from favorites: Classic").is_some());
    let (light, dark) = favorite_theme_groups(&[SavedTheme::Studio, SavedTheme::Classic, SavedTheme::Adwaita]);
    assert_eq!(light, vec![ThemeKind::Classic, ThemeKind::Adwaita]);
    assert_eq!(dark, vec![ThemeKind::Studio]);
}

/// The favourite star badge overhangs its swatch; showing it on hover must not move the swatches
/// after it, or the row would shift under the pointer.
#[test]
fn theme_star_badge_on_hover_does_not_move_the_next_swatch() {
    use egui_kittest::{Harness, kittest::Queryable};
    let (mut app, _) = app_with_store();
    open_preferences(&mut app, "interface");
    let mut h = Harness::builder().with_size(vec2(1280.0, 800.0)).build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        app
    });
    h.run_steps(4);
    let first = h.get_by_label("Classic").rect();
    let next = h.get_by_label("Adwaita").rect();
    h.hover_at(first.right_top() + vec2(-3.0, 3.0));
    h.run_steps(3);
    assert!(h.query_by_label("Add to favorites: Classic").is_some(), "the badge shows while the swatch is hovered");
    assert_eq!(h.get_by_label("Classic").rect(), first);
    assert_eq!(h.get_by_label("Adwaita").rect(), next);
}

#[test]
fn preferences_interface_shows_language_without_scrolling() {
    // #2532: in a small window the theme cards filled the page and Language sat out of view
    // below them. Like Photoshop, Presentation follows Appearance and both fit.
    use egui_kittest::{Harness, kittest::Queryable};
    let (mut app, _) = app_with_store();
    app.run("prefs.set", json!({"values": {"interface.language": "en"}})).unwrap();
    open_preferences(&mut app, "interface");
    let mut h = Harness::builder().with_size(vec2(1000.0, 750.0)).build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        app
    });
    h.run_steps(4);
    let language = h.query_all_by_label("Language").next().expect("the Language row").rect();
    let dark = h.query_all_by_label("Dark Theme").next().expect("the dark theme row").rect();
    let ok = h.get_by_label("OK").rect();
    assert!(dark.bottom() < language.top(), "Presentation {language:?} follows the theme swatches {dark:?}");
    assert!(language.bottom() < ok.top() - 20.0, "Language {language:?} must show above the buttons {ok:?} without scrolling");
    // Every theme in the option lists gets a swatch, Breeze included.
    for (id, _) in LIGHT_THEME_OPTIONS.iter().chain(DARK_THEME_OPTIONS) {
        let label = choice_label(id);
        assert!(h.query_by_label(&label).is_some(), "missing theme swatch {label}");
    }
    assert!(h.query_by_label("Breeze light").is_some() && h.query_by_label("Breeze dark").is_some());
}

#[test]
fn preferences_interface_search_hides_rows_and_skips_empty_groups() {
    use egui_kittest::{Harness, kittest::Queryable};
    let render = |search: &str| {
        let (mut app, _) = app_with_store();
        app.run("prefs.set", json!({"values": {"interface.language": "en"}})).unwrap();
        let id = open_preferences(&mut app, "interface");
        app.ui.dialog_mut(id).unwrap().fields.insert("__search".into(), json!(search));
        let mut h = Harness::builder().with_size(vec2(1000.0, 750.0)).build_eframe(move |cc| {
            PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
            app
        });
        h.run_steps(4);
        h
    };
    let shown = |h: &Harness<'_, PhotocraftApp>, label: &str| h.query_all_by_label(label).next().is_some();

    let h = render("language");
    assert!(shown(&h, "Language") && shown(&h, "Presentation"));
    for gone in ["Appearance", "Appearance Mode", "Dark Theme", "Options", "UI scale"] {
        assert!(!shown(&h, gone), "\"language\" must not show {gone}");
    }

    let h = render("theme");
    assert!(shown(&h, "Appearance") && shown(&h, "Light Theme") && shown(&h, "Dark Theme") && shown(&h, "Breeze dark"));
    for gone in ["Appearance Mode", "Presentation", "Language", "Options"] {
        assert!(!shown(&h, gone), "\"theme\" must not show {gone}");
    }

    let h = render("");
    for group in ["Appearance", "Appearance Mode", "Presentation", "Options", "UI scale"] {
        assert!(shown(&h, group), "an empty search shows {group}");
    }
}

fn app_with_store() -> (PhotocraftApp, Arc<Mutex<Option<String>>>) {
    app_with_saved(None)
}

fn app_with_saved(text: Option<String>) -> (PhotocraftApp, Arc<Mutex<Option<String>>>) {
    let store: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(text));
    (app_on(&store, Arc::default()), store)
}

/// An app whose preferences live in `store`; writes fail while `fail` is set.
fn app_on(store: &Arc<Mutex<Option<String>>>, fail: Arc<std::sync::atomic::AtomicBool>) -> PhotocraftApp {
    let (a, b) = (store.clone(), store.clone());
    let services = crate::Services {
        load_prefs: Some(Box::new(move || a.lock().unwrap().clone())),
        save_prefs: Some(Box::new(move |s: &str| {
            if fail.load(std::sync::atomic::Ordering::Relaxed) {
                return Err("storage is full".into());
            }
            *b.lock().unwrap() = Some(s.to_string());
            Ok(())
        })),
        ..Default::default()
    };
    PhotocraftApp::new(photocraft_engine::Session::new(), services)
}

fn stored(store: &Arc<Mutex<Option<String>>>) -> Value {
    serde_json::from_str(store.lock().unwrap().as_deref().unwrap_or("null")).unwrap()
}

#[test]
fn a_wrongly_typed_value_resets_only_itself_and_is_repaired_on_disk() {
    // #6611: one bad value used to reset every preference, on every launch.
    let saved = json!({
        "fileHandling": {"recentFileCount": "ten", "autosaveMinutes": 5},
        "performance": {"historyStates": 12},
        "shortcuts": {"edit.fill": "Cmd+Shift+F", "file.open": 7},
        "futureSection": {"kept": true},
    });
    let (mut app, store) = app_with_saved(Some(saved.to_string()));
    let ctx = egui::Context::default();
    tick(&mut app, &ctx);
    let p = app.session.prefs();
    assert_eq!((p.file_handling.recent_file_count, p.file_handling.autosave_minutes, p.performance.history_states), (20, 5, 12));
    assert!(!app.ui.status.contains("reset"), "{}", app.ui.status);
    let notice = app.ui.notices.iter().find(|n| n.title == "Some preferences were reset").expect("a notice names the bad values");
    let text = notice.text(&notice.lines[0]);
    assert!(text.contains("fileHandling.recentFileCount, shortcuts.file.open"), "{text}");
    // The file now holds the default in its place and keeps everything else.
    let v = stored(&store);
    assert_eq!(v["fileHandling"]["recentFileCount"], 20);
    assert_eq!(v["fileHandling"]["autosaveMinutes"], 5);
    assert_eq!(v["performance"]["historyStates"], 12);
    assert_eq!(v["shortcuts"]["edit.fill"], "Cmd+Shift+F");
    assert!(v["shortcuts"].get("file.open").is_none());
    assert_eq!(v["futureSection"]["kept"], true, "keys this version doesn't know survive");
    // So the next launch loads cleanly.
    let (mut next, _) = app_with_saved(store.lock().unwrap().clone());
    tick(&mut next, &ctx);
    assert!(next.ui.notices.iter().all(|n| n.title != "Some preferences were reset"));
    assert_eq!(next.session.prefs().file_handling.autosave_minutes, 5);
}

#[test]
fn an_old_files_shift_tool_switch_migration_is_saved_with_the_new_version() {
    // #5647: a file before version 2 loads with Use Shift Key for Tool Switch on. Saving
    // writes that and the current version, so turning it off later isn't taken for the old
    // default on the next launch.
    let ctx = egui::Context::default();
    for saved in [
        json!({"version": 1, "tools": {"useShiftKeyForToolSwitch": false}, "futureSection": {"kept": true}}),
        json!({"tools": {"useShiftKeyForToolSwitch": false}, "futureSection": {"kept": true}}),
    ] {
        let (mut app, store) = app_with_saved(Some(saved.to_string()));
        tick(&mut app, &ctx);
        assert!(app.session.prefs().tools.use_shift_key_for_tool_switch, "{saved}");
        app.run("prefs.set", json!({"values": {"performance.historyStates": 12}})).unwrap();
        tick(&mut app, &ctx);
        let v = stored(&store);
        assert_eq!(v["version"], json!(prefs::PREFS_VERSION), "{saved}");
        assert_eq!(v["tools"]["useShiftKeyForToolSwitch"], true, "{saved}");
        assert_eq!(v["futureSection"]["kept"], true, "keys this version doesn't know survive");
        app.run("prefs.set", json!({"values": {"tools.useShiftKeyForToolSwitch": false}})).unwrap();
        tick(&mut app, &ctx);
        let (mut next, _) = app_with_saved(store.lock().unwrap().clone());
        tick(&mut next, &ctx);
        assert!(!next.session.prefs().tools.use_shift_key_for_tool_switch, "{saved}: turned off, it stays off");
    }
}

#[test]
fn an_old_sparse_file_keeps_another_instances_change() {
    // Two instances on a version 1 file that lacks most keys: one changes a key the file
    // doesn't have, the other then saves an unrelated change and must not write its default
    // over it.
    let saved = json!({"version": 1, "tools": {"useShiftKeyForToolSwitch": false}});
    let store = Arc::new(Mutex::new(Some(saved.to_string())));
    let ctx = egui::Context::default();
    let (mut a, mut b) = (app_on(&store, Arc::default()), app_on(&store, Arc::default()));
    tick(&mut a, &ctx);
    tick(&mut b, &ctx);
    b.run("prefs.set", json!({"values": {"interface.theme": "studio"}})).unwrap();
    tick(&mut b, &ctx);
    assert_eq!(stored(&store)["interface"]["theme"], "studio");
    a.run("prefs.set", json!({"values": {"performance.historyStates": 12}})).unwrap();
    tick(&mut a, &ctx);
    let v = stored(&store);
    assert_eq!((v["interface"]["theme"].as_str(), v["performance"]["historyStates"].as_u64()), (Some("studio"), Some(12)));
}

#[test]
fn another_instances_shift_tool_switch_choice_beats_this_ones_migration() {
    // Two instances migrate the same version 1 file. One turns the switch off and saves
    // version 2; the other's unrelated save must not write its migrated "on" over that.
    let saved = json!({"version": 1, "tools": {"useShiftKeyForToolSwitch": false}});
    let store = Arc::new(Mutex::new(Some(saved.to_string())));
    let ctx = egui::Context::default();
    let (mut a, mut b) = (app_on(&store, Arc::default()), app_on(&store, Arc::default()));
    tick(&mut a, &ctx);
    tick(&mut b, &ctx);
    b.run("prefs.set", json!({"values": {"tools.useShiftKeyForToolSwitch": false}})).unwrap();
    tick(&mut b, &ctx);
    assert_eq!(stored(&store)["tools"]["useShiftKeyForToolSwitch"], false);
    a.run("prefs.set", json!({"values": {"performance.historyStates": 12}})).unwrap();
    tick(&mut a, &ctx);
    let v = stored(&store);
    assert_eq!((v["tools"]["useShiftKeyForToolSwitch"].as_bool(), v["performance"]["historyStates"].as_u64()), (Some(false), Some(12)));
    assert_eq!(v["version"], json!(prefs::PREFS_VERSION));
}

#[test]
fn turning_the_shift_tool_switch_off_is_saved_over_another_instances_migration() {
    // Two instances migrate the same version 1 file. B saves version 2 with the migrated "on";
    // the user then turns the switch off in A, which must be written.
    let saved = json!({"version": 1, "tools": {"useShiftKeyForToolSwitch": false}});
    let store = Arc::new(Mutex::new(Some(saved.to_string())));
    let ctx = egui::Context::default();
    let (mut a, mut b) = (app_on(&store, Arc::default()), app_on(&store, Arc::default()));
    tick(&mut a, &ctx);
    tick(&mut b, &ctx);
    b.run("prefs.set", json!({"values": {"performance.historyStates": 12}})).unwrap();
    tick(&mut b, &ctx);
    assert_eq!(stored(&store)["tools"]["useShiftKeyForToolSwitch"], true);
    a.run("prefs.set", json!({"values": {"tools.useShiftKeyForToolSwitch": false}})).unwrap();
    tick(&mut a, &ctx);
    let v = stored(&store);
    assert_eq!((v["tools"]["useShiftKeyForToolSwitch"].as_bool(), v["performance"]["historyStates"].as_u64()), (Some(false), Some(12)));
}

#[test]
fn a_newer_files_version_is_kept_on_save() {
    let saved = json!({"version": prefs::PREFS_VERSION + 1, "tools": {"useShiftKeyForToolSwitch": false}});
    let (mut app, store) = app_with_saved(Some(saved.to_string()));
    let ctx = egui::Context::default();
    tick(&mut app, &ctx);
    assert!(!app.session.prefs().tools.use_shift_key_for_tool_switch, "a newer file isn't migrated");
    app.run("prefs.set", json!({"values": {"performance.historyStates": 12}})).unwrap();
    tick(&mut app, &ctx);
    let v = stored(&store);
    assert_eq!(v["version"], json!(prefs::PREFS_VERSION + 1));
    assert_eq!(v["performance"]["historyStates"], 12);
}

#[test]
fn merge_keeps_changes_made_elsewhere() {
    let base = json!({"a": {"x": 1, "y": 1}, "list": [1], "gone": 1, "kept": 1});
    // Here: a.x changed, `gone` removed, `new` added.
    let ours = json!({"a": {"x": 2, "y": 1}, "list": [1], "kept": 1, "new": 1});
    // There: a.y, `list` and `kept` changed, and a key this version doesn't know.
    let mut theirs = json!({"a": {"x": 1, "y": 3}, "list": [1, 2], "gone": 1, "kept": 5, "future": true});
    merge_changes(&base, &ours, &mut theirs);
    assert_eq!(theirs, json!({"a": {"x": 2, "y": 3}, "list": [1, 2], "kept": 5, "new": 1, "future": true}));
    // Arrays are leaves: a list changed here replaces the stored one whole.
    merge_changes(&json!({"list": [1]}), &json!({"list": [1, 3]}), &mut theirs);
    assert_eq!(theirs["list"], json!([1, 3]));
    // Nothing changed here: storage is left as it is, whatever it holds.
    let mut odd = json!({"x": [7]});
    merge_changes(&ours, &ours, &mut odd);
    assert_eq!(odd, json!({"x": [7]}));
    // A value that changed type here replaces the stored one.
    merge_changes(&json!({"a": 1}), &json!({"a": {"b": 2}}), &mut theirs);
    assert_eq!(theirs["a"], json!({"b": 2}));
}

#[test]
fn saving_keeps_preferences_another_instance_changed() {
    let store = Arc::new(Mutex::new(None));
    let ctx = egui::Context::default();
    // Two browser tabs (or app windows) on the same storage.
    let (mut a, mut b) = (app_on(&store, Arc::default()), app_on(&store, Arc::default()));
    tick(&mut a, &ctx);
    tick(&mut b, &ctx);
    a.run("prefs.set", json!({"values": {"interface.theme": "studio"}})).unwrap();
    tick(&mut a, &ctx);
    assert_eq!(stored(&store)["interface"]["theme"], "studio");
    // The other one, never reloaded, saves an unrelated change: the theme stays.
    b.run("prefs.set", json!({"values": {"performance.historyStates": 12}})).unwrap();
    tick(&mut b, &ctx);
    let v = stored(&store);
    assert_eq!((v["interface"]["theme"].as_str(), v["performance"]["historyStates"].as_u64()), (Some("studio"), Some(12)));
    // And the first one's next save keeps the second one's change.
    a.run("prefs.set", json!({"values": {"interface.showTooltips": false}})).unwrap();
    tick(&mut a, &ctx);
    let v = stored(&store);
    assert_eq!(v["performance"]["historyStates"], 12);
    assert_eq!(v["interface"]["showTooltips"], false);
    // A value changed in both: the latest save wins.
    b.run("prefs.set", json!({"values": {"interface.theme": "classic"}})).unwrap();
    tick(&mut b, &ctx);
    assert_eq!(stored(&store)["interface"]["theme"], "classic");
    // Unreadable storage is overwritten with this instance's preferences.
    *store.lock().unwrap() = Some("not json".into());
    a.run("prefs.set", json!({"values": {"performance.historyStates": 30}})).unwrap();
    tick(&mut a, &ctx);
    let (mut c, _) = app_with_saved(store.lock().unwrap().clone());
    tick(&mut c, &ctx);
    assert_eq!(c.session.prefs().performance.history_states, 30);
    assert_eq!(c.session.prefs().interface.theme, Theme::Studio);
}

#[test]
fn failed_preference_saves_are_retried() {
    use std::sync::atomic::{AtomicBool, Ordering};
    let store = Arc::new(Mutex::new(None));
    let fail = Arc::new(AtomicBool::new(true));
    let mut app = app_on(&store, fail.clone());
    tick(&mut app, &egui::Context::default());
    app.run("prefs.set", json!({"values": {"interface.theme": "studio"}})).unwrap();
    // The write fails: reported, and retried after a backoff rather than marked saved.
    assert_eq!(persist(&mut app, 100.0), Some(SAVE_RETRY_S));
    assert!(app.ui.status.starts_with("Couldn't save preferences: storage is full") && app.ui.status_error);
    assert!(store.lock().unwrap().is_none());
    assert_eq!(persist(&mut app, 101.0), Some(1.0), "not due yet");
    // Storage works again: the pending change is written without another edit.
    fail.store(false, Ordering::Relaxed);
    assert_eq!(persist(&mut app, 102.0), None);
    assert_eq!(stored(&store)["interface"]["theme"], "studio");
    assert_eq!(app.ui.status, "Preferences saved");
    assert_eq!(persist(&mut app, 103.0), None, "nothing left to save");

    // A persistent failure backs off up to a cap and raises a notice once.
    fail.store(true, Ordering::Relaxed);
    app.run("prefs.set", json!({"values": {"interface.theme": "classic"}})).unwrap();
    let mut now = 200.0;
    let mut delays = Vec::new();
    for _ in 0..6 {
        let wait = persist(&mut app, now).unwrap();
        delays.push(wait);
        now += wait;
    }
    assert_eq!(delays, [2.0, 4.0, 8.0, 16.0, SAVE_RETRY_MAX_S, SAVE_RETRY_MAX_S]);
    assert_eq!(app.ui.notices.iter().filter(|n| n.error && n.title == "Preferences can't be saved").count(), 1);
    // A newer change retries sooner than the backoff, but not on every frame.
    app.run("prefs.set", json!({"values": {"interface.theme": "pro"}})).unwrap();
    assert_eq!(persist(&mut app, now - 29.0), Some(1.0));
    fail.store(false, Ordering::Relaxed);
    assert_eq!(persist(&mut app, now - 28.0), None);
    assert_eq!(stored(&store)["interface"]["theme"], "pro");
    assert!(app.ui.notices.is_empty(), "the notice goes once preferences are saved");
}

#[test]
fn auto_scale_preserves_system_scale() {
    use prefs::UiScale::Auto;
    // Whatever the monitor, 4K included: an unscaled desktop stays at 100% (#1278), and a
    // fractional or below-100% system scale is the user's choice (#1072).
    for dpi in [0.75, 1.0, 1.25, 1.5, 1.75, 2.0, 2.5, 3.0] {
        assert_eq!(display_scale(Auto, Some(dpi)), dpi);
    }
    assert_eq!(display_scale(Auto, None), 1.0);
    assert_eq!(display_scale(Auto, Some(f32::NAN)), 1.0);
    assert_eq!(display_scale(Auto, Some(-2.0)), 1.0);
}

#[test]
fn scale_preferences_and_monitor_changes_apply_live() {
    let (mut app, _) = app_with_store();
    let ctx = egui::Context::default();
    {
        let mut step = |physical: egui::Vec2, native: f32, expected: f32| {
            let mut input = egui::RawInput::default();
            let viewport = input.viewports.get_mut(&egui::ViewportId::ROOT).unwrap();
            viewport.native_pixels_per_point = Some(native);
            viewport.monitor_size = Some(physical / (native * ctx.zoom_factor()));
            ctx.run_ui(input, |ui| tick(&mut app, ui.ctx())).textures_delta.clear();
            // Scale changes take effect on the following pass.
            let mut input = egui::RawInput::default();
            let viewport = input.viewports.get_mut(&egui::ViewportId::ROOT).unwrap();
            viewport.native_pixels_per_point = Some(native);
            viewport.monitor_size = Some(physical / (native * ctx.zoom_factor()));
            ctx.run_ui(input, |ui| tick(&mut app, ui.ctx())).textures_delta.clear();
            assert!((ctx.pixels_per_point() - expected).abs() < 1e-4);
        };
        for _ in 0..4 {
            step(vec2(3840.0, 2160.0), 1.0, 1.0);
        }
        step(vec2(1920.0, 1080.0), 1.0, 1.0);
        step(vec2(3840.0, 2160.0), 1.5, 1.5);
        step(vec2(3840.0, 2160.0), 1.25, 1.25);
        step(vec2(2560.0, 1440.0), 1.75, 1.75);
        step(vec2(3840.0, 2160.0), 1.0, 1.0);
    }
    for (pref, expected) in [
        ("200", 2.0),
        ("125", 1.25),
        ("150", 1.5),
        ("100", 1.0),
        ("105", 1.05),
        ("110", 1.1),
        ("115", 1.15),
        ("120", 1.2),
        ("95", 0.95),
        ("90", 0.9),
        ("85", 0.85),
        ("80", 0.8),
        ("auto", 1.5),
    ] {
        app.run("prefs.set", json!({"values": {"interface.uiScale": pref}})).unwrap();
        let mut input = egui::RawInput::default();
        input.viewports.get_mut(&egui::ViewportId::ROOT).unwrap().native_pixels_per_point = Some(1.5);
        ctx.run_ui(input.clone(), |ui| tick(&mut app, ui.ctx())).textures_delta.clear();
        ctx.run_ui(input, |ui| tick(&mut app, ui.ctx())).textures_delta.clear();
        assert!((ctx.pixels_per_point() - expected).abs() < 1e-4);
    }
}

#[test]
fn intermediate_ui_scale_choices_can_be_applied_from_preferences() {
    use egui_kittest::{Harness, kittest::Queryable};
    let (mut app, store) = app_with_store();
    app.run("prefs.set", json!({"path": "interface.uiScale", "value": "100"})).unwrap();
    open_preferences(&mut app, "interface");
    let mut h = Harness::builder().with_size(vec2(1400.0, 1100.0)).build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        app
    });
    h.run_steps(4);
    let mut current = "100%";
    for (label, value) in [("105%", "105"), ("110%", "110"), ("115%", "115"), ("120%", "120")] {
        h.get_by_value(current).click();
        h.run_steps(2);
        h.get_by_label(label).click();
        h.run_steps(2);
        h.get_by_label("Apply").click();
        h.run_steps(4);
        assert_eq!(h.state().session.prefs().interface.ui_scale.name(), value);
        assert_eq!(stored(&store)["interface"]["uiScale"], value);
        current = label;
    }
}

#[test]
fn ui_font_size_commands_follow_themes_and_dpi_and_reset_live() {
    let (mut app, _) = app_with_store();
    let ctx = egui::Context::default();
    PhotocraftApp::setup_context(&ctx, ThemeKind::Pro);
    let mut input = egui::RawInput::default();
    input.viewports.get_mut(&egui::ViewportId::ROOT).unwrap().native_pixels_per_point = Some(1.5);
    for (theme, scale, expected) in
        [("pro", "100", 1.0), ("studio", "auto", 1.5), ("studioLight", "200", 2.0), ("proMedium", "100", 1.0), ("classic", "auto", 1.5)]
    {
        app.run("prefs.set", json!({"values": {"interface.uiFontSize": "large", "interface.theme": theme, "interface.uiScale": scale}})).unwrap();
        for _ in 0..3 {
            ctx.run_ui(input.clone(), |ui| tick(&mut app, ui.ctx())).textures_delta.clear();
        }
        assert!((ctx.pixels_per_point() - expected).abs() < 1e-4);
        assert_eq!(ctx.fonts(|f| f.definitions().font_data["Inter"].tweak.scale), 16.0 / 12.0);
    }
    app.run("prefs.reset", json!({"path": "interface.uiFontSize"})).unwrap();
    for _ in 0..2 {
        ctx.run_ui(input.clone(), |ui| tick(&mut app, ui.ctx())).textures_delta.clear();
    }
    assert_eq!(ctx.fonts(|f| f.definitions().font_data["Inter"].tweak.scale), 1.0);
    assert!((ctx.pixels_per_point() - 1.5).abs() < 1e-4, "resetting font size keeps UI Scale");
}

#[test]
fn recent_files_survive_a_restart_and_honour_the_count() {
    let (mut app, store) = app_with_store();
    let ctx = egui::Context::default();
    tick(&mut app, &ctx);
    app.push_recent("/work/a.psd");
    app.push_recent("/work/b.png");
    tick(&mut app, &ctx);
    let saved = store.lock().unwrap().clone().unwrap();
    // A new app instance (a restart) gets the list back, newest first.
    let (mut app2, _) = app_with_saved(Some(saved));
    tick(&mut app2, &ctx);
    assert_eq!(app2.ui.recent_files, vec!["/work/b.png".to_string(), "/work/a.psd".to_string()]);
    // Lowering "Recent File List Contains" shortens the menu at once; 0 turns it off.
    app2.run("prefs.set", json!({"values": {"fileHandling.recentFileCount": 1}})).unwrap();
    tick(&mut app2, &ctx);
    assert_eq!(app2.ui.recent_files, vec!["/work/b.png".to_string()]);
    app2.push_recent("/work/c.tif");
    assert_eq!(app2.session.prefs().file_handling.recent_files, vec!["/work/c.tif".to_string()]);
    app2.run("prefs.set", json!({"values": {"fileHandling.recentFileCount": 0}})).unwrap();
    tick(&mut app2, &ctx);
    app2.push_recent("/work/d.tif");
    assert!(app2.ui.recent_files.is_empty());
    // Clearing the list in the Preferences dialog (or by an agent) reaches the menu.
    app2.run("prefs.set", json!({"values": {"fileHandling.recentFileCount": 20, "fileHandling.recentFiles": ["/x.psd"]}})).unwrap();
    tick(&mut app2, &ctx);
    assert_eq!(app2.ui.recent_files, vec!["/x.psd".to_string()]);
    // A hostile count from a hand-edited preferences file is capped, not trusted.
    app2.run("prefs.set", json!({"values": {"fileHandling.recentFileCount": 100}})).unwrap();
    app2.session.prefs.edit(|p| p.file_handling.recent_file_count = u32::MAX);
    assert_eq!(app2.recent_cap(), 100);
}

#[test]
fn brush_picker_view_survives_a_restart() {
    let (mut app, store) = app_with_store();
    let ctx = egui::Context::default();
    tick(&mut app, &ctx);
    app.ui.brush_picker_list.show_stroke = false;
    app.ui.brush_picker_list.scale = 0.4;
    crate::brush_picker::persist(&mut app, &ctx);
    tick(&mut app, &ctx);
    assert_eq!(stored(&store)["brushPicker"]["showStroke"], json!(false));
    // A restart gets the view back at startup, before the picker is ever opened.
    let saved = store.lock().unwrap().clone().unwrap();
    let (mut app2, _) = app_with_saved(Some(saved));
    tick(&mut app2, &ctx);
    assert!(!app2.ui.brush_picker_list.show_stroke);
    assert!(app2.ui.brush_picker_list.show_name && app2.ui.brush_picker_list.show_tip);
    assert!((app2.ui.brush_picker_list.scale - 0.4).abs() < 1e-6);
    // A hand-edited file: the scale is clamped to the slider's range and the last part comes
    // back on — an empty card would show no brush at all.
    let bad = json!({"brushPicker": {"showName": false, "showStroke": false, "showTip": false, "scale": 9.0}}).to_string();
    let (mut app3, _) = app_with_saved(Some(bad));
    tick(&mut app3, &ctx);
    assert_eq!(app3.ui.brush_picker_list.scale, *crate::brush_picker::SCALE_RANGE.end());
    assert!(app3.ui.brush_picker_list.show_name && app3.ui.brush_picker_list.show_stroke && app3.ui.brush_picker_list.show_tip);
}

#[test]
fn selected_tool_survives_a_restart_without_option_changes() {
    use crate::state::Tool;
    let (mut app, store) = app_with_store();
    let ctx = egui::Context::default();
    tick(&mut app, &ctx);
    assert_eq!(app.ui.tool, Tool::Brush);
    // Every tool, including flyout-only tools, can be restored. Return to Brush last to
    // prove selecting the default overwrites a previously remembered nondefault tool.
    for tool in Tool::ALL.into_iter().chain([Tool::Brush]) {
        app.ui.tool = tool;
        crate::tool_memory::persist(&mut app, &ctx);
        tick(&mut app, &ctx);
        assert_eq!(stored(&store)["selectedTool"], serde_json::to_value(tool).unwrap());
        let (app2, _) = app_with_saved(store.lock().unwrap().clone());
        assert_eq!(app2.ui.tool, tool, "restored before the first frame");
        assert_eq!(app2.ui.group_tool(tool.key()), Some(tool));
        assert_eq!(app2.ui.tool_options, crate::state::ToolOptions::default());
    }
}

#[test]
fn selected_tool_is_flushed_on_exit_without_another_frame() {
    let (mut app, store) = app_with_store();
    app.ui.tool = crate::state::Tool::Move;
    crate::tool_memory::save_on_exit(&mut app).unwrap();
    let (app2, _) = app_with_saved(store.lock().unwrap().clone());
    assert_eq!(app2.ui.tool, crate::state::Tool::Move);
}

#[test]
fn selected_tool_explicit_eraser_survives_exit_during_temporary_switches() {
    use crate::state::Tool;
    for spring in [false, true] {
        for menu in [false, true] {
            let (mut app, store) = app_with_store();
            app.stylus.tool_before_eraser = Some(Tool::Zoom);
            app.ui.tool = Tool::Eraser;
            if spring {
                crate::spring_tools::pressed(&mut app, 'E', Tool::Zoom, true, 0.0);
            }
            if menu {
                crate::menus::invoke(&mut app, &egui::Context::default(), "tools.select.eraser", json!({})).unwrap();
            } else {
                crate::tool_memory::select(&mut app, Tool::Eraser);
            }
            assert_eq!(app.stylus.tool_before_eraser, None);
            assert!(app.ui.spring.is_none());
            crate::tool_memory::save_on_exit(&mut app).unwrap();
            let (restored, _) = app_with_saved(store.lock().unwrap().clone());
            assert_eq!(restored.ui.tool, Tool::Eraser);
        }
    }
}

#[test]
fn invalid_selected_tool_preserves_other_preferences() {
    for bad in [json!(null), json!("FutureTool"), json!(42), json!({}), json!([])] {
        let (app, _) = app_with_saved(Some(
            json!({
                "selectedTool": bad, "toolOptions": {"feather": 12.0}
            })
            .to_string(),
        ));
        assert_eq!(app.ui.tool, crate::state::Tool::Brush, "{bad}");
        assert_eq!(app.ui.tool_options.feather, 12.0);
    }
}

#[test]
fn selected_tool_excludes_temporary_switches() {
    use crate::state::Tool;
    let (mut app, store) = app_with_store();
    let ctx = egui::Context::default();
    app.ui.tool = Tool::Move;
    crate::tool_memory::persist(&mut app, &ctx);
    tick(&mut app, &ctx);
    app.tool_override = Some(Tool::Hand);
    crate::tool_memory::persist(&mut app, &ctx);
    tick(&mut app, &ctx);
    assert_eq!(stored(&store)["selectedTool"], json!("Move"));
    app.tool_override = None;
    app.ui.tool = Tool::Zoom;
    crate::spring_tools::pressed(&mut app, 'Z', Tool::Move, true, 0.0);
    crate::tool_memory::persist(&mut app, &ctx);
    tick(&mut app, &ctx);
    assert_eq!(stored(&store)["selectedTool"], json!("Move"));
    // A quick tap commits the choice after spring_tools clears the pending switch.
    crate::spring_tools::tick(&mut app, &ctx);
    crate::tool_memory::persist(&mut app, &ctx);
    tick(&mut app, &ctx);
    assert_eq!(stored(&store)["selectedTool"], json!("Zoom"));
    app.stylus.tool_before_eraser = Some(Tool::Zoom);
    app.ui.tool = Tool::Eraser;
    crate::tool_memory::persist(&mut app, &ctx);
    tick(&mut app, &ctx);
    assert_eq!(stored(&store)["selectedTool"], json!("Zoom"));
    // An explicit selection during a temporary switch takes precedence.
    app.ui.tool = Tool::PolygonLasso;
    crate::tool_memory::persist(&mut app, &ctx);
    tick(&mut app, &ctx);
    assert_eq!(stored(&store)["selectedTool"], json!("PolygonLasso"));
}

#[test]
fn tool_options_survive_a_restart() {
    let (mut app, store) = app_with_store();
    let ctx = egui::Context::default();
    tick(&mut app, &ctx);
    app.ui.tool_options.move_auto_select = false;
    app.ui.tool_options.feather = 12.0;
    crate::tool_memory::persist(&mut app, &ctx);
    tick(&mut app, &ctx);
    // Only what differs from the defaults is stored.
    assert_eq!(stored(&store)["toolOptions"], json!({"move_auto_select": false, "feather": 12.0}));
    let saved = store.lock().unwrap().clone().unwrap();
    let (mut app2, _) = app_with_saved(Some(saved));
    tick(&mut app2, &ctx);
    assert!(!app2.ui.tool_options.move_auto_select);
    assert_eq!(app2.ui.tool_options.feather, 12.0);
    assert_eq!(app2.ui.tool_options.tolerance, crate::state::ToolOptions::default().tolerance);
    // A hand-edited file: unreadable fields keep their defaults, the good ones still load.
    for bad in [json!("junk"), json!({"move_auto_select": "no", "feather": 1e300, "tolerance": 5.0, "unknown": 1})] {
        let (mut app3, _) = app_with_saved(Some(json!({"toolOptions": bad}).to_string()));
        tick(&mut app3, &ctx);
        let o = &app3.ui.tool_options;
        assert!(o.move_auto_select && o.feather == 0.0, "{bad}");
        assert_eq!(o.tolerance, if bad.is_object() { 5.0 } else { 32.0 });
    }
}

/// #6212: a remembered value its options-bar field could never hold is clamped to the
/// field's range (or takes the default) rather than reaching the tool.
#[test]
fn remembered_tool_options_are_clamped_to_their_fields() {
    let ctx = egui::Context::default();
    let hostile = json!({"toolOptions": {
        "feather": 1e30, "tolerance": -5.0, "polygon_sides": 0, "eyedropper_size": 7,
        "crop_overlay_orientation": 9, "fill_opacity": 0.0, "magnetic_width": 1e9,
        "pattern_stamp_angle": -720.0, "marquee_width": 1e7, "shape_stroke": {"miter_limit": -1.0, "opacity": 5.0},
    }});
    let (mut app, _) = app_with_saved(Some(hostile.to_string()));
    tick(&mut app, &ctx);
    let o = &app.ui.tool_options;
    let d = crate::state::ToolOptions::default();
    assert_eq!((o.feather, o.tolerance, o.polygon_sides, o.eyedropper_size), (1000.0, 0.0, 3, d.eyedropper_size));
    assert_eq!((o.crop_overlay_orientation, o.fill_opacity, o.magnetic_width), (d.crop_overlay_orientation, 1.0, 256.0));
    assert_eq!((o.pattern_stamp_angle, o.marquee_width), (-180.0, 999.0));
    assert_eq!(o.shape_stroke, d.shape_stroke, "an invalid stroke style takes the default");
    // A good value is left alone, and in-range defaults never change (nothing to save).
    let (mut app2, _) = app_with_saved(Some(json!({"toolOptions": {"feather": 12.5}}).to_string()));
    tick(&mut app2, &ctx);
    assert_eq!(app2.ui.tool_options, crate::state::ToolOptions { feather: 12.5, ..d.clone() });
    let mut def = d.clone();
    crate::tool_memory::sanitize(&mut def);
    assert_eq!(def, d);
}

/// #6212: every painting tool keeps its own brush across a restart, and the session brush
/// comes back as the brush of the tool that had it.
#[test]
fn tool_brushes_survive_a_restart() {
    use crate::state::Tool;
    let (mut app, store) = app_with_store();
    let ctx = egui::Context::default();
    tick(&mut app, &ctx);
    let set_size = |app: &mut PhotocraftApp, size: f32| {
        app.session.tools.brush.size = size;
        app.session.tools.brush.opacity = size / 100.0;
        app.session.tools.brush_changed();
    };
    set_size(&mut app, 77.0);
    app.ui.tool = Tool::Eraser;
    crate::paint_mouse::sync_tool_brush(&mut app);
    set_size(&mut app, 33.0);
    app.ui.tool = Tool::CloneStamp;
    crate::paint_mouse::sync_tool_brush(&mut app);
    set_size(&mut app, 12.0);
    app.ui.tool = Tool::Eraser;
    crate::paint_mouse::sync_tool_brush(&mut app);
    crate::last_brush::persist(&mut app, &ctx);
    crate::tool_memory::persist(&mut app, &ctx);
    tick(&mut app, &ctx);
    let saved = stored(&store);
    assert_eq!(saved["toolBrushes"]["owner"], json!("Eraser"));
    assert_eq!(saved["toolBrushes"]["brushes"]["Brush"]["size"], json!(77.0));
    assert_eq!(saved["toolBrushes"]["brushes"]["CloneStamp"]["size"], json!(12.0));
    assert!(saved["toolBrushes"]["brushes"].get("Eraser").is_none(), "the owner's brush is the last brush");

    let (mut app2, _) = app_with_saved(store.lock().unwrap().clone());
    tick(&mut app2, &ctx);
    assert_eq!((app2.ui.tool, app2.ui.brush_tool), (Tool::Eraser, Tool::Eraser));
    assert_eq!(app2.session.tools.brush.size, 33.0, "the Eraser's brush is the session brush");
    for (tool, size) in [(Tool::Brush, 77.0), (Tool::CloneStamp, 12.0), (Tool::Eraser, 33.0)] {
        app2.ui.tool = tool;
        crate::paint_mouse::sync_tool_brush(&mut app2);
        assert_eq!(app2.session.tools.brush.size, size, "{tool:?}");
        assert_eq!(app2.session.tools.brush.opacity, size / 100.0, "{tool:?}");
    }
}

/// #6212: `toolBrushes.owner` names the tool whose brush `last_brush` holds. A brush that
/// can't be remembered yet (a sampled tip too big to embed) leaves `last_brush` at the old
/// owner's brush, so the old owner stays saved: the next launch gives that brush back to it,
/// not to the tool selected since.
#[test]
fn the_saved_brush_owner_follows_the_saved_last_brush() {
    use crate::state::Tool;
    use photocraft_engine::paint::{GrayTile, TipShape};
    let (mut app, store) = app_with_store();
    let ctx = egui::Context::default();
    tick(&mut app, &ctx);
    app.session.tools.brush.size = 77.0;
    app.session.tools.brush_changed();
    crate::last_brush::persist(&mut app, &ctx);
    crate::tool_memory::persist(&mut app, &ctx);
    // The Brush's tip becomes a big sampled one, then the Eraser takes over that brush.
    app.session.tools.brush.tip = TipShape::Sampled(GrayTile::from_fn(400, 300, |x, y| ((x * y) % 7) as f32 / 7.0));
    app.session.tools.brush_changed();
    app.ui.tool = Tool::Eraser;
    crate::paint_mouse::sync_tool_brush(&mut app);
    crate::last_brush::persist(&mut app, &ctx);
    crate::tool_memory::persist(&mut app, &ctx);
    tick(&mut app, &ctx);
    let saved = stored(&store);
    assert_eq!(saved["lastBrush"]["brush"]["size"], json!(77.0), "the big tip is not remembered yet");
    assert_ne!(saved["toolBrushes"]["owner"], json!("Eraser"), "{saved}");
    let (mut app2, _) = app_with_saved(store.lock().unwrap().clone());
    tick(&mut app2, &ctx);
    assert_eq!(app2.ui.brush_tool, Tool::Brush, "the 77 px brush is the Brush's");
    assert_eq!(app2.session.tools.brush.size, 77.0);

    // Once the Eraser's brush is saved, the Eraser owns it in the preferences too.
    app.session.tools.brush.tip = TipShape::Round;
    app.session.tools.brush.size = 33.0;
    app.session.tools.brush_changed();
    crate::last_brush::persist(&mut app, &ctx);
    crate::tool_memory::persist(&mut app, &ctx);
    tick(&mut app, &ctx);
    let saved = stored(&store);
    assert_eq!(saved["toolBrushes"]["owner"], json!("Eraser"), "{saved}");
    assert_eq!(saved["lastBrush"]["brush"]["size"], json!(33.0));
}

/// #6212: a damaged or hand-edited `toolBrushes` never panics: what reads back loads (clamped
/// to the options bar's ranges), the rest is skipped and those tools start from the brush.
#[test]
fn remembered_tool_brushes_survive_damage() {
    use crate::state::Tool;
    let ctx = egui::Context::default();
    for bad in [json!("junk"), json!(42), json!({"owner": [1], "brushes": "x"}), json!({"brushes": {"Eraser": 5}})] {
        let (mut app, _) = app_with_saved(Some(json!({"toolBrushes": bad}).to_string()));
        tick(&mut app, &ctx);
        assert_eq!(app.ui.brush_tool, Tool::Brush, "{bad}");
        assert!(app.ui.tool_brushes.is_empty(), "{bad}");
    }
    let hostile = json!({"toolBrushes": {"owner": "Move", "brushes": {
        "Eraser": {"size": 1e30}, "Bogus": {"size": 5.0}, "Move": {"size": 5.0},
        "Smudge": {"size": -3.0, "opacity": 50.0, "flow": -2.0, "hardness": 1e30},
        "Brush": {"size": 9.0},
    }}});
    let (mut app, _) = app_with_saved(Some(hostile.to_string()));
    tick(&mut app, &ctx);
    assert_eq!(app.ui.brush_tool, Tool::Brush, "the Move tool has no brush");
    let tools: Vec<Tool> = app.ui.tool_brushes.iter().map(|(t, _)| *t).collect();
    assert_eq!(tools, vec![Tool::Smudge], "too large, unknown, brushless and the owner's own are skipped");
    let b = &app.ui.tool_brushes[0].1;
    assert_eq!((b.size, b.opacity, b.flow, b.hardness), (1.0, 1.0, 0.0, 1.0));
    // A whole preferences file that isn't JSON: defaults, no panic.
    let (mut app, _) = app_with_saved(Some("{not json".into()));
    tick(&mut app, &ctx);
    assert!(app.ui.tool_brushes.is_empty());
    assert_eq!(app.ui.tool_options, crate::state::ToolOptions::default());
}

/// #6212: Reset Tool forgets that tool's remembered options and brush; Reset All Tools
/// forgets every tool's.
#[test]
fn reset_tools_clear_the_remembered_options_and_brushes() {
    use crate::state::Tool;
    let (mut app, store) = app_with_store();
    let ctx = egui::Context::default();
    tick(&mut app, &ctx);
    app.ui.tool_options.feather = 12.0;
    app.ui.tool_options.strength = 80.0;
    app.session.tools.brush.size = 77.0;
    app.session.tools.brush_changed();
    for tool in [Tool::Eraser, Tool::Smudge, Tool::Brush] {
        app.ui.tool = tool;
        crate::paint_mouse::sync_tool_brush(&mut app);
    }
    app.ui.tool = Tool::Smudge;
    crate::paint_mouse::sync_tool_brush(&mut app);
    crate::last_brush::persist(&mut app, &ctx);
    crate::tool_memory::persist(&mut app, &ctx);
    tick(&mut app, &ctx);
    let saved = stored(&store);
    assert_eq!(saved["toolOptions"]["strength"], json!(80.0));
    assert!(saved["toolBrushes"]["brushes"].get("Eraser").is_some() && saved["toolBrushes"]["brushes"].get("Brush").is_some());

    crate::menus::invoke(&mut app, &ctx, crate::tool_reset::RESET, json!({})).unwrap();
    crate::last_brush::persist(&mut app, &ctx);
    crate::tool_memory::persist(&mut app, &ctx);
    tick(&mut app, &ctx);
    let saved = stored(&store);
    assert!(saved["toolOptions"].get("strength").is_none(), "the Smudge's Strength is forgotten");
    assert_eq!(saved["toolOptions"]["feather"], json!(12.0), "the marquee's Feather is not the Smudge's");
    assert!(saved["toolBrushes"]["brushes"].get("Eraser").is_some());

    crate::menus::invoke(&mut app, &ctx, crate::tool_reset::RESET_ALL, json!({"confirmed": true})).unwrap();
    crate::last_brush::persist(&mut app, &ctx);
    crate::tool_memory::persist(&mut app, &ctx);
    tick(&mut app, &ctx);
    let saved = stored(&store);
    assert!(saved["toolOptions"].is_null(), "{saved}");
    assert!(saved["toolBrushes"]["brushes"].as_object().is_none_or(|m| m.is_empty()), "{saved}");
    let (app2, _) = app_with_saved(store.lock().unwrap().clone());
    assert_eq!(app2.ui.tool_options, crate::state::ToolOptions::default());
    assert!(app2.ui.tool_brushes.is_empty());
}

#[test]
fn view_show_items_survive_a_restart() {
    let (mut app, store) = app_with_store();
    let ctx = egui::Context::default();
    tick(&mut app, &ctx);
    // Untouched: nothing is stored, so later default changes still reach the user.
    crate::view_cmds::persist(&mut app, &ctx);
    tick(&mut app, &ctx);
    assert!(stored(&store).get("viewExtras").is_none_or(Value::is_null));
    // The menu commands, as a user toggles them (#3762: Pixel Grid off).
    for id in ["view.show.pixelGrid", "view.show.grid", "view.snapTo.layers", "view.extras"] {
        crate::menus::invoke(&mut app, &ctx, id, json!({})).unwrap();
    }
    crate::view_cmds::persist(&mut app, &ctx);
    tick(&mut app, &ctx);
    assert_eq!(stored(&store)["viewExtras"], json!({"extras": false, "grid": true, "show": {"pixelGrid": false}, "snapTo": {"layers": false}}));
    // A restart gets them back.
    let saved = store.lock().unwrap().clone().unwrap();
    let (mut app2, _) = app_with_saved(Some(saved));
    tick(&mut app2, &ctx);
    let v = &app2.ui.view;
    assert!(!v.extras && !v.show.pixel_grid && !v.snap_to.layers && app2.ui.extras.grid);
    assert!(v.show.selection_edges && v.snap_to.guides && app2.ui.extras.guides, "the rest keep their defaults");
    // A hand-edited file: unknown keys and non-booleans keep the defaults, good ones still load.
    for bad in [json!("junk"), json!({"extras": 0, "show": {"pixelGrid": "no", "mesh": true, "bogus": true}, "snapTo": [1]})] {
        let (mut app3, _) = app_with_saved(Some(json!({"viewExtras": bad}).to_string()));
        tick(&mut app3, &ctx);
        let v = &app3.ui.view;
        assert!(v.extras && v.show.pixel_grid && v.snap_to.layers, "{bad}");
        assert_eq!(v.show.mesh, bad.is_object());
    }
}

#[test]
fn view_snap_off_survives_a_restart() {
    let (mut app, store) = app_with_store();
    let ctx = egui::Context::default();
    tick(&mut app, &ctx);
    assert!(app.ui.extras.snap, "on by default");
    crate::menus::invoke(&mut app, &ctx, "view.snap", json!({})).unwrap();
    assert!(!app.ui.extras.snap);
    crate::view_cmds::persist(&mut app, &ctx);
    tick(&mut app, &ctx);
    assert_eq!(stored(&store)["viewExtras"], json!({"snap": false}));
    let saved = store.lock().unwrap().clone().unwrap();
    let (mut app2, _) = app_with_saved(Some(saved));
    tick(&mut app2, &ctx);
    assert!(!app2.ui.extras.snap, "still off after a restart");
    crate::menus::invoke(&mut app2, &ctx, "view.snap", json!({})).unwrap();
    crate::view_cmds::persist(&mut app2, &ctx);
    assert!(app2.session.prefs().view_extras.is_null());
}

#[test]
fn show_tooltips_preferences_turn_tooltips_off() {
    let (mut app, _) = app_with_store();
    let ctx = egui::Context::default();
    tick(&mut app, &ctx);
    assert_eq!(ctx.global_style().interaction.tooltip_delay, crate::theme::TOOLTIP_DELAY);
    for path in ["interface.showTooltips", "tools.showTooltips"] {
        app.run("prefs.set", json!({"values": {path: false}})).unwrap();
        tick(&mut app, &ctx);
        assert!(ctx.global_style().interaction.tooltip_delay.is_infinite(), "{path} off hides tooltips");
        app.run("prefs.set", json!({"values": {path: true}})).unwrap();
        tick(&mut app, &ctx);
        assert_eq!(ctx.global_style().interaction.tooltip_delay, crate::theme::TOOLTIP_DELAY);
    }
}

#[test]
fn enable_animations_off_stops_fades_and_scroll_glides_and_is_remembered() {
    let on = egui::Style::default().animation_time;
    let both = |ctx: &egui::Context| [egui::Theme::Dark, egui::Theme::Light].map(|t| ctx.style_of(t).animation_time);
    // Saved off: the first frame after a restart applies it to both appearance styles.
    let (mut app, _) = app_with_saved(Some(json!({"interface": {"enableAnimations": false}}).to_string()));
    let ctx = egui::Context::default();
    tick(&mut app, &ctx);
    assert_eq!(both(&ctx), [0.0, 0.0]);
    assert_eq!(ctx.global_style().scroll_animation.duration.max, 0.0);
    // A theme change rebuilds the style; the next frame turns animations off again.
    app.apply_theme(&ctx, crate::theme::ThemeKind::StudioLight);
    tick(&mut app, &ctx);
    assert_eq!(both(&ctx), [0.0, 0.0]);
    app.run("prefs.set", json!({"values": {"interface.enableAnimations": true}})).unwrap();
    tick(&mut app, &ctx);
    assert_eq!(both(&ctx), [on, on]);
    assert!(on > 0.0 && ctx.global_style().scroll_animation.duration.max > 0.0);
    // A hand-edited value of the wrong type doesn't crash: the defaults (animations on) stay.
    let (mut bad, _) = app_with_saved(Some(json!({"interface": {"enableAnimations": "nope"}}).to_string()));
    let ctx2 = egui::Context::default();
    tick(&mut bad, &ctx2);
    assert_eq!(both(&ctx2), [on, on]);
}

#[test]
fn unimplemented_preferences_are_hidden_from_the_dialog() {
    let values = prefs::Preferences::default().to_json();
    assert!(has_visible_fields(&values, "general"));
    assert!(has_visible_fields(&values, "fileHandling"));
    // Every setting of this section is still unimplemented.
    assert!(!has_visible_fields(&values, "scratchDisks"), "scratchDisks");
    // Allow Agent Control and Control Port are read at launch (apps/photocraft main.rs).
    assert!(has_visible_fields(&values, "integrations"));
    assert!(!prefs::is_hidden("integrations.allowAgentControl"));
    assert!(!prefs::is_hidden("integrations.controlPort"));
    // "Fill new type layers with placeholder text" and "Use Escape to Commit" are live; other Type rows stay hidden.
    assert!(has_visible_fields(&values, "type"));
    assert!(!prefs::is_hidden("type.fillNewTypeLayersWithPlaceholder"));
    assert!(!prefs::is_hidden("type.useEscToCommit"));
    assert!(!prefs::is_hidden("type.smartQuotes"));
    assert!(!prefs::is_hidden("type.showFontNamesInEnglish"));
    // Rotate View with Trackpad, the pinch zoom and Enable Gestures are live; the other
    // Enhanced Controls rows stay hidden.
    assert!(has_visible_fields(&values, "enhancedControls"));
    assert!(!prefs::is_hidden("enhancedControls.rotateViewWithTrackpad"));
    assert!(!prefs::is_hidden("enhancedControls.zoomWithTrackpadPinch"));
    assert!(!prefs::is_hidden("enhancedControls.touchGestures"));
    assert!(prefs::is_hidden("enhancedControls.scrubbySliderAcceleration"));
    // Camera Raw Defaults are all live: the dialog's Open applies them (camera_raw_ui).
    assert!(has_visible_fields(&values, "rawDefaults"));
    for k in ["openInCameraRaw", "demosaic", "applyAutoTone", "colorSpace", "bitDepth", "resolution", "sharpenFor", "openAsSmartObject", "writeXmpSidecars"] {
        assert!(!prefs::is_hidden(&format!("rawDefaults.{k}")), "{k}");
    }
    assert!(!prefs::is_hidden("general.autoShowHomeScreen"));
    assert!(!prefs::is_hidden("general.animatedZoom"));
    assert!(!prefs::is_hidden("general.zoomResizesWindows"));
    assert!(!prefs::is_hidden("interface.uiScale"));
    assert!(!prefs::is_hidden("interface.uiFontSize"));
    // Hidden values still round-trip through the dialog untouched.
    let (mut app, _) = app_with_store();
    app.run("prefs.set", json!({"values": {"type.smartQuotes": false}})).unwrap();
    let id = open_preferences(&mut app, "type");
    let fields = app.ui.dialogs.iter().find(|d| d.id == id).map(|d| d.fields.clone()).unwrap();
    confirm(&mut app, &fields).unwrap();
    assert!(!app.session.prefs().type_.smart_quotes);
}

#[test]
fn preferences_persist_through_services_and_theme_follows() {
    let (mut app, store) = app_with_store();
    let ctx = egui::Context::default();
    tick(&mut app, &ctx);
    app.run("prefs.set", json!({"values": {"interface.theme": "studioLight", "performance.historyStates": 12}})).unwrap();
    tick(&mut app, &ctx);
    assert_eq!(app.ui.theme, ThemeKind::StudioLight);
    let saved = store.lock().unwrap().clone().unwrap();
    assert!(saved.contains("\"historyStates\": 12"));
    // A new app instance loads them.
    let (mut app2, store2) = app_with_saved(Some(saved));
    tick(&mut app2, &ctx);
    assert_eq!(app2.session.prefs().performance.history_states, 12);
    assert_eq!(app2.ui.theme, ThemeKind::StudioLight);
    // Picking a theme from the Window menu becomes the preference.
    crate::menus::invoke(&mut app2, &ctx, "window.theme.classic", json!({})).unwrap();
    tick(&mut app2, &ctx);
    assert_eq!(app2.session.prefs().interface.theme, Theme::Classic);
    assert!(store2.lock().unwrap().as_ref().unwrap().contains("classic"));
}

#[test]
fn preset_export_dialog_offers_tool_presets() {
    use egui_kittest::{Harness, kittest::Queryable};
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let ctx = egui::Context::default();
    let opened = crate::menus::invoke(&mut app, &ctx, "edit.presets.exportImportPresets", json!({})).unwrap();
    let fields = app.ui.dialog_mut(opened["dialog"].as_u64().unwrap()).unwrap().fields.clone();
    assert_eq!(fields["tools"], true);
    let mut h = Harness::builder().with_size(vec2(420.0, 260.0)).build_ui_state(presets_io_body, fields);
    h.run_steps(2);
    h.get_by_label("Tool Presets").click();
    h.run_steps(2);
    assert_eq!(h.state()["tools"], false);
    assert_eq!(h.state()["brushes"], true);
    assert_eq!(h.state()["customShapes"], true);
}

#[test]
fn brush_preset_store_attaches_when_loaded_and_persists() {
    use photocraft_engine::preset_store::{MemBackend, open};
    let mem = MemBackend::default();
    let ctx = egui::Context::default();
    let (tx, rx) = std::sync::mpsc::channel();
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services { preset_store: Some(rx), ..Default::default() });
    // Still loading: presets made now are kept and written once the store arrives.
    tick(&mut app, &ctx);
    app.run("brush.presets.save", json!({"name": "Early"})).unwrap();
    assert!(mem.files.lock().unwrap().is_empty());
    tx.send(open(Box::new(mem.clone()))).unwrap();
    tick(&mut app, &ctx);
    assert!(app.services.preset_store.is_none() && app.session.preset_store.is_some());
    app.run("brush.presets.save", json!({"name": "Late"})).unwrap();
    let mut s2 = photocraft_engine::Session::new();
    assert!(s2.attach_preset_store(open(Box::new(mem))).is_empty());
    for n in ["Early", "Late"] {
        assert!(s2.tools.presets.iter().any(|p| p.name == n), "{n}");
    }
}

#[test]
fn preferences_dialog_round_trip() {
    let (mut app, _) = app_with_store();
    let ctx = egui::Context::default();
    let r = crate::menus::invoke(&mut app, &ctx, "edit.preferences.cursors", json!({})).unwrap();
    let id = r["dialog"].as_u64().unwrap();
    let d = app.ui.dialogs.iter().find(|d| d.id == id).unwrap().clone();
    assert_eq!(d.fields["section"], "cursors");
    let mut values = d.fields["values"].clone();
    values["cursors"]["painting"] = json!("fullSizeTip");
    values["unitsAndRulers"]["rulers"] = json!("inches");
    app.ui.dialog_mut(id).unwrap().fields.insert("values".into(), values);
    crate::dialogs::confirm(&mut app, id).unwrap();
    assert_eq!(app.session.prefs().cursors.painting, prefs::PaintingCursor::FullSizeTip);
    assert_eq!(app.session.prefs().units_and_rulers.rulers, prefs::Unit::Inches);
    // Invalid values are rejected and nothing changes.
    let id = open_preferences(&mut app, "performance");
    let mut values = app.ui.dialogs.iter().find(|d| d.id == id).unwrap().fields["values"].clone();
    values["performance"]["historyStates"] = json!(0);
    app.ui.dialog_mut(id).unwrap().fields.insert("values".into(), values);
    assert!(crate::dialogs::confirm(&mut app, id).is_err());
    assert_eq!(app.session.prefs().performance.history_states, 50);
}

#[test]
fn preferences_apply_button_saves_without_closing_and_cancel_keeps_applied_values() {
    use egui_kittest::{
        Harness,
        kittest::{NodeT, Queryable},
    };

    let (mut app, store) = app_with_store();
    app.run("prefs.set", json!({"values": {"interface.language": "en", "type.smartQuotes": false}})).unwrap();
    let id = open_preferences(&mut app, "interface");
    let mut h = Harness::builder().with_size(vec2(1280.0, 800.0)).build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        app
    });
    h.run_steps(4);
    assert!(h.get_by_label("Apply").accesskit_node().is_disabled());

    let values = h.state_mut().ui.dialog_mut(id).unwrap().fields.get_mut("values").unwrap();
    values["interface"]["appearanceMode"] = json!("light");
    values["interface"]["lightTheme"] = json!("studioLight");
    values["interface"]["uiFontSize"] = json!("large");
    values["performance"]["historyStates"] = json!(12);
    h.run_steps(2);
    assert!(!h.get_by_label("Apply").accesskit_node().is_disabled());
    h.get_by_label("Apply").click();
    h.run_steps(4);

    assert_eq!(h.state().session.prefs().performance.history_states, 12);
    assert_eq!(h.state().ui.theme, ThemeKind::StudioLight);
    assert_eq!(h.ctx.fonts(|f| f.definitions().font_data["Inter"].tweak.scale), 16.0 / 12.0);
    assert!(!h.state().session.prefs().type_.smart_quotes, "hidden settings round-trip unchanged");
    let d = h.state().ui.dialogs.iter().find(|d| d.id == id).unwrap();
    assert_eq!(d.fields["section"], "interface");
    assert_eq!(d.fields["values"]["performance"]["historyStates"], 12);
    assert!(h.get_by_label("Apply").accesskit_node().is_disabled());
    let saved: Value = serde_json::from_str(store.lock().unwrap().as_ref().unwrap()).unwrap();
    assert_eq!(saved["performance"]["historyStates"], 12);
    assert_eq!(saved["interface"]["appearanceMode"], "light");
    assert_eq!(saved["interface"]["lightTheme"], "studioLight");
    assert_eq!(saved["interface"]["uiFontSize"], "large");

    // Repeated Apply starts from the validated values, not the dialog's original snapshot.
    h.state_mut().ui.dialog_mut(id).unwrap().fields.get_mut("values").unwrap()["performance"]["historyStates"] = json!(22);
    h.run_steps(2);
    h.get_by_label("Apply").click();
    h.run_steps(4);
    assert_eq!(h.state().session.prefs().performance.history_states, 22);
    h.state_mut().ui.dialog_mut(id).unwrap().fields.get_mut("values").unwrap()["performance"]["historyStates"] = json!(33);
    h.state_mut().ui.dialog_mut(id).unwrap().fields.get_mut("values").unwrap()["interface"]["uiFontSize"] = json!("tiny");
    h.run_steps(2);
    h.get_by_label("Cancel").click();
    h.run_steps(4);
    assert!(h.state().ui.dialogs.iter().all(|d| d.id != id));
    assert_eq!(h.state().session.prefs().performance.history_states, 22);
    assert_eq!(h.ctx.fonts(|f| f.definitions().font_data["Inter"].tweak.scale), 16.0 / 12.0);
    let saved = store.lock().unwrap().clone().unwrap();
    let (mut restarted, _) = app_with_saved(Some(saved));
    tick(&mut restarted, &egui::Context::default());
    assert_eq!(restarted.session.prefs().performance.history_states, 22);
    assert_eq!(restarted.ui.theme, ThemeKind::StudioLight);
    assert_eq!(restarted.session.prefs().interface.ui_font_size, prefs::UiFontSize::Large);
}

#[cfg(all(not(target_arch = "wasm32"), not(target_os = "macos")))]
#[test]
fn system_title_bar_explains_next_launch_and_preserves_apply_and_cancel() {
    use egui_kittest::{Harness, kittest::Queryable};

    let (mut app, store) = app_with_store();
    app.run("prefs.set", json!({"values": {"interface.language": "en"}})).unwrap();
    app.custom_titlebar = true;
    let id = open_preferences(&mut app, "interface");
    let mut h = Harness::builder().with_size(vec2(1280.0, 800.0)).build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        app
    });
    h.run_steps(4);
    h.get_by_label("System title bar").scroll_to_me();
    h.run_steps(4);
    h.get_by_label("Applies at next launch.");
    for desired in [true, false] {
        h.get_by_label("System title bar").click();
        h.run_steps(4);
        assert_eq!(h.state().ui.dialogs.iter().find(|d| d.id == id).unwrap().fields["values"]["interface"]["systemTitleBar"], desired);
        assert_eq!(h.state().session.prefs().interface.system_title_bar, !desired, "the draft is not applied yet");
        h.get_by_label("Applies at next launch.");
        h.get_by_label("Apply").click();
        h.run_steps(4);
        assert_eq!(h.state().session.prefs().interface.system_title_bar, desired);
        assert_eq!(stored(&store)["interface"]["systemTitleBar"], desired);
        assert!(h.state().custom_titlebar, "do not change the current window or restart it");
        let (reloaded, _) = app_with_saved(store.lock().unwrap().clone());
        assert_eq!(reloaded.session.prefs().interface.system_title_bar, desired);
        h.get_by_label("Applies at next launch.");
    }
    h.get_by_label("System title bar").click();
    h.run_steps(4);
    h.get_by_label("Cancel").click();
    h.run_steps(4);
    assert!(h.state().ui.dialogs.iter().all(|d| d.id != id));
    assert!(!h.state().session.prefs().interface.system_title_bar);
    assert_eq!(stored(&store)["interface"]["systemTitleBar"], false);
    assert!(h.state().custom_titlebar);
}

/// #7939: switching Rendering Mode waits for the next launch, and says so next to the setting.
#[test]
fn rendering_mode_explains_next_launch() {
    use egui_kittest::{Harness, kittest::Queryable};
    let obj = json!({"renderingMode": "auto", "historyStates": 50}).as_object().unwrap().clone();
    let mut h = Harness::new_ui_state(
        |ui, obj: &mut Map<String, Value>| section_fields(ui, "performance", obj, &[], crate::i18n::Lang::from_pref("en"), None, ""),
        obj,
    );
    h.run_steps(4);
    assert!(h.query_all_by_label("Rendering Mode").next().is_some(), "the Rendering Mode setting");
    assert!(h.query_all_by_label("Applies at next launch.").next().is_some(), "its next-launch notice");
    // A search that leaves Rendering Mode out leaves its notice out too.
    let obj = json!({"renderingMode": "auto", "historyStates": 50}).as_object().unwrap().clone();
    let mut h = Harness::new_ui_state(
        |ui, obj: &mut Map<String, Value>| section_fields(ui, "performance", obj, &[], crate::i18n::Lang::from_pref("en"), None, "history"),
        obj,
    );
    h.run_steps(4);
    assert!(h.query_by_label("Applies at next launch.").is_none());
}

/// #4097: Allow Agent Control and Control Port are read when the app starts, and say so.
#[test]
fn integrations_explain_next_launch() {
    use egui_kittest::{Harness, kittest::Queryable};
    let obj = json!({"allowAgentControl": true, "controlPort": 0}).as_object().unwrap().clone();
    let mut h = Harness::new_ui_state(
        |ui, obj: &mut Map<String, Value>| section_fields(ui, "integrations", obj, &[], crate::i18n::Lang::from_pref("en"), None, ""),
        obj,
    );
    h.run_steps(4);
    h.get_by_label("Allow agent control");
    assert_eq!(h.query_all_by_label("Applies at next launch.").count(), 2, "one notice under each row");
}

#[test]
fn next_launch_notice_does_not_mark_unrelated_interface_preferences() {
    use egui_kittest::{Harness, kittest::Queryable};
    let obj = json!({"showTooltips": true}).as_object().unwrap().clone();
    let mut h =
        Harness::new_ui_state(|ui, obj: &mut Map<String, Value>| section_fields(ui, "interface", obj, &[], crate::i18n::Lang::from_pref("en"), None, ""), obj);
    h.run_steps(4);
    assert!(h.query_by_label("Applies at next launch.").is_none());
    h.get_by_label("Show tooltips").click();
    h.run_steps(4);
    assert_eq!(h.state()["showTooltips"], false);
    assert!(h.query_by_label("Applies at next launch.").is_none());
}

#[test]
fn preferences_confirm_after_apply_commits_later_edits() {
    let (mut app, _) = app_with_store();
    let id = open_preferences(&mut app, "cursors");
    app.ui.dialog_mut(id).unwrap().fields.get_mut("values").unwrap()["cursors"]["painting"] = json!("fullSizeTip");
    apply(&mut app, id).unwrap();
    app.ui.dialog_mut(id).unwrap().fields.get_mut("values").unwrap()["unitsAndRulers"]["rulers"] = json!("inches");
    crate::dialogs::confirm(&mut app, id).unwrap();
    assert!(app.ui.dialogs.iter().all(|d| d.id != id));
    assert_eq!(app.session.prefs().cursors.painting, prefs::PaintingCursor::FullSizeTip);
    assert_eq!(app.session.prefs().units_and_rulers.rulers, prefs::Unit::Inches);
}

#[test]
fn preferences_apply_rejects_invalid_drafts_and_keeps_them_open() {
    let (mut app, _) = app_with_store();
    let before = app.session.prefs().to_json();
    for invalid in [json!(0), json!("invalid"), Value::Null] {
        let id = open_preferences(&mut app, "performance");
        app.ui.dialog_mut(id).unwrap().fields.get_mut("values").unwrap()["performance"]["historyStates"] = invalid;
        let draft = app.ui.dialog_mut(id).unwrap().fields.clone();
        assert!(apply(&mut app, id).is_err());
        assert_eq!(app.session.prefs().to_json(), before);
        assert_eq!(app.ui.dialog_mut(id).unwrap().fields, draft);
        app.ui.close_dialog(id);
    }
    let id = open_preferences(&mut app, "interface");
    app.ui.dialog_mut(id).unwrap().fields.remove("values");
    assert!(apply(&mut app, id).is_err());
    assert!(app.ui.dialog_mut(id).is_some());
    assert_eq!(app.session.prefs().to_json(), before);
}

#[test]
fn apply_rejects_missing_and_non_preferences_dialogs() {
    let (mut app, _) = app_with_store();
    let before = app.session.prefs().to_json();
    assert!(apply(&mut app, u64::MAX).is_err());
    let id = open_shortcuts(&mut app, 0);
    assert!(apply(&mut app, id).is_err());
    assert!(app.ui.dialog_mut(id).is_some());
    let fields = json!({"__prefsui": "prefs", "values": preference_values(app.session.prefs())}).as_object().unwrap().clone();
    let id = app.ui.open_dialog(DialogKind::About, fields);
    assert!(apply(&mut app, id).is_err());
    assert!(app.ui.dialog_mut(id).is_some());
    assert_eq!(app.session.prefs().to_json(), before);
}

#[test]
fn shortcut_dialog_moves_shortcuts_and_shell_dispatch_follows() {
    let (mut app, _) = app_with_store();
    let ctx = egui::Context::default();
    let id = crate::menus::invoke(&mut app, &ctx, "edit.keyboardShortcuts", json!({})).unwrap()["dialog"].as_u64().unwrap();
    // ⌘O (File › Open, a shell command) given to Layer › New › Layer.
    app.ui.dialog_mut(id).unwrap().fields.insert("overrides".into(), json!({"layer.new.layer": "Cmd+O"}));
    crate::dialogs::confirm(&mut app, id).unwrap();
    assert_eq!(effective_shortcut(&app, "layer.new.layer", None).as_deref(), Some("Cmd+O"));
    assert_eq!(effective_shortcut(&app, "file.open", Some("Cmd+O")), None, "taken from File › Open");
    // Menu items show the effective shortcut.
    let items = crate::menus::menu_items(&app);
    assert!(items.iter().any(|i| i.id == "layer.new.layer" && i.shortcut.as_deref() == Some("Cmd+O")));
    assert!(items.iter().any(|i| i.id == "file.open" && i.shortcut.is_none()));
    assert_eq!(shortcut_text(egui::Key::K, egui::Modifiers { command: true, shift: true, ..Default::default() }).as_deref(), Some("Cmd+Shift+K"));

    // Re-assigning a shortcut already held by a custom override takes it from the override.
    let id2 = crate::menus::invoke(&mut app, &ctx, "edit.keyboardShortcuts", json!({})).unwrap()["dialog"].as_u64().unwrap();
    let mut cur_overrides = app.session.prefs().shortcuts.clone();
    cur_overrides.insert("layer.new.group".into(), "Cmd+O".into());
    app.ui.dialog_mut(id2).unwrap().fields.insert("overrides".into(), json!(cur_overrides));
    crate::dialogs::confirm(&mut app, id2).unwrap();
    assert_eq!(effective_shortcut(&app, "layer.new.group", None).as_deref(), Some("Cmd+O"));
    assert_eq!(effective_shortcut(&app, "layer.new.layer", None), None, "taken from layer.new.layer override");

    // OK with nothing changed keeps every shortcut where it is.
    let id3 = crate::menus::invoke(&mut app, &ctx, "edit.keyboardShortcuts", json!({})).unwrap()["dialog"].as_u64().unwrap();
    let unchanged = app.session.prefs().shortcuts.clone();
    app.ui.dialog_mut(id3).unwrap().fields.insert("overrides".into(), json!(unchanged));
    crate::dialogs::confirm(&mut app, id3).unwrap();
    assert_eq!(app.session.prefs().shortcuts, unchanged);

    // Giving the shortcut back to Layer › New › Layer takes it from the group override; a
    // cleared override (empty) holds nothing and is left alone.
    let id4 = crate::menus::invoke(&mut app, &ctx, "edit.keyboardShortcuts", json!({})).unwrap()["dialog"].as_u64().unwrap();
    let mut ov = app.session.prefs().shortcuts.clone();
    ov.insert("layer.new.layer".into(), "Cmd+O".into());
    app.ui.dialog_mut(id4).unwrap().fields.insert("overrides".into(), json!(ov));
    crate::dialogs::confirm(&mut app, id4).unwrap();
    assert_eq!(effective_shortcut(&app, "layer.new.layer", None).as_deref(), Some("Cmd+O"));
    assert_eq!(effective_shortcut(&app, "layer.new.group", None), None, "taken back from the group");
    assert_eq!(effective_shortcut(&app, "file.open", Some("Cmd+O")), None, "File › Open stays cleared");
}

/// #3665: the dialog keeps a command's further keys: they move like main keys, an override
/// of the main key leaves them, and menus show only the main key.
#[test]
fn shortcut_dialog_moves_and_keeps_alternate_keys() {
    let (mut app, _) = app_with_store();
    let ctx = egui::Context::default();
    let open = |app: &mut PhotocraftApp| crate::menus::invoke(app, &ctx, "edit.keyboardShortcuts", json!({})).unwrap()["dialog"].as_u64().unwrap();
    assert_eq!(crate::shortcuts::effective_alternates(&app, "edit.copy"), ["F3"]);
    assert_eq!(crate::shortcuts::effective_alternates(&app, "view.zoomIn"), ["Cmd++"]);
    // F2 (Cut's alternate) given to Rename Layer is taken from Cut, which keeps ⌘X.
    let id = open(&mut app);
    app.ui.dialog_mut(id).unwrap().fields.insert("overrides".into(), json!({"layer.renameLayer": "F2"}));
    crate::dialogs::confirm(&mut app, id).unwrap();
    assert_eq!(effective_shortcut(&app, "layer.renameLayer", None).as_deref(), Some("F2"));
    assert!(crate::shortcuts::effective_alternates(&app, "edit.cut").is_empty());
    assert_eq!(effective_shortcut(&app, "edit.cut", Some("Cmd+X")).as_deref(), Some("Cmd+X"));
    // Copy's main key moved to ⌘K keeps F3; the menu shows ⌘K.
    let id = open(&mut app);
    let mut ov = app.session.prefs().shortcuts.clone();
    ov.insert("edit.copy".into(), "Cmd+K".into());
    app.ui.dialog_mut(id).unwrap().fields.insert("overrides".into(), json!(ov));
    crate::dialogs::confirm(&mut app, id).unwrap();
    assert_eq!(crate::shortcuts::effective_alternates(&app, "edit.copy"), ["F3"]);
    assert!(crate::menus::menu_items(&app).iter().any(|i| i.id == "edit.copy" && i.shortcut.as_deref() == Some("Cmd+K")));
    // An added alternate takes its key from the main key that held it.
    let id = open(&mut app);
    app.ui.dialog_mut(id).unwrap().fields.insert("alternates".into(), json!({"edit.paste": ["F4", "Cmd+K"]}));
    crate::dialogs::confirm(&mut app, id).unwrap();
    assert_eq!(crate::shortcuts::effective_alternates(&app, "edit.paste"), ["F4", "Cmd+K"]);
    assert_eq!(effective_shortcut(&app, "edit.copy", Some("Cmd+C")), None, "taken from Copy");
    // Use Default for everything restores the defaults; a malformed list is an error.
    let id = open(&mut app);
    app.ui.dialog_mut(id).unwrap().fields.insert("alternates".into(), json!({"edit.paste": "F4"}));
    assert!(crate::dialogs::confirm(&mut app, id).is_err());
    let id = open(&mut app);
    app.ui.dialog_mut(id).unwrap().fields.insert("alternates".into(), json!({"edit.paste": ["Cmd+Shift"]}));
    assert!(crate::dialogs::confirm(&mut app, id).is_err());
    assert_eq!(crate::shortcuts::effective_alternates(&app, "edit.paste"), ["F4", "Cmd+K"], "nothing changed");
    let id = open(&mut app);
    app.ui.dialog_mut(id).unwrap().fields.insert("alternates".into(), json!({}));
    app.ui.dialog_mut(id).unwrap().fields.insert("overrides".into(), json!({}));
    crate::dialogs::confirm(&mut app, id).unwrap();
    assert!(app.session.prefs().alternate_shortcuts.is_empty() && app.session.prefs().shortcuts.is_empty());
    assert_eq!(crate::shortcuts::effective_alternates(&app, "edit.cut"), ["F2"]);
}

#[test]
fn shortcut_dialog_rejects_malformed_overrides() {
    // #956: a non-map `overrides` was read as "no overrides" and wiped every custom shortcut.
    let (mut app, _) = app_with_store();
    let ctx = egui::Context::default();
    app.run("edit.keyboardShortcuts", json!({"set": {"file.new": "Ctrl+Shift+N"}})).unwrap();
    let before = app.session.prefs().shortcuts.clone();
    assert!(!before.is_empty());
    for bad in [json!("bogus"), json!(["file.new"]), json!({"file.new": 5})] {
        let id = crate::menus::invoke(&mut app, &ctx, "edit.keyboardShortcuts", json!({})).unwrap()["dialog"].as_u64().unwrap();
        app.ui.dialog_mut(id).unwrap().fields.insert("overrides".into(), bad.clone());
        assert!(crate::dialogs::confirm(&mut app, id).is_err(), "{bad} accepted");
        assert_eq!(app.session.prefs().shortcuts, before, "{bad} changed the shortcuts");
    }

    // Drawing the open dialog keeps the malformed value for OK to reject, instead of
    // replacing it with the empty map it shows.
    let id = crate::menus::invoke(&mut app, &ctx, "edit.keyboardShortcuts", json!({})).unwrap()["dialog"].as_u64().unwrap();
    app.ui.dialog_mut(id).unwrap().fields.insert("overrides".into(), json!("bogus"));
    let mut h = egui_kittest::Harness::builder().with_size(vec2(1280.0, 800.0)).build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        app
    });
    h.run_steps(4);
    assert_eq!(h.state().ui.dialogs.iter().find(|d| d.id == id).unwrap().fields["overrides"], json!("bogus"));
    assert!(crate::dialogs::confirm(h.state_mut(), id).is_err());
    assert_eq!(h.state().session.prefs().shortcuts, before);
}

#[test]
fn hidden_menu_items_disappear() {
    let (mut app, _) = app_with_store();
    app.run("edit.menus", json!({"hide": ["edit.fade"]})).unwrap();
    assert!(!crate::menus::menu_items(&app).iter().any(|i| i.id == "edit.fade"));
}

/// #5096: preferences that list Edit › Menus or File › Exit as hidden still show them.
#[test]
fn essential_menu_items_stay_visible() {
    let (mut app, _) = app_with_store();
    app.run("prefs.set", json!({"values": {"menus": {"hidden": ["edit.menus", "file.exit", "edit.fade"], "colors": {}}}})).unwrap();
    let items = crate::menus::menu_items(&app);
    assert!(items.iter().any(|i| i.id == "edit.menus") && items.iter().any(|i| i.id == "file.exit"));
    assert!(!items.iter().any(|i| i.id == "edit.fade"));
}

#[test]
fn autosave_runs_for_dirty_documents() {
    type Saved = Arc<Mutex<Vec<(u64, u64)>>>;
    let saved: Saved = Arc::default();
    let s2 = saved.clone();
    let services = crate::Services {
        autosave: Some(Box::new(move |doc: &Arc<photocraft_doc::Document>, rev: u64, _path: Option<&str>| {
            s2.lock().unwrap().push((doc.id.0, rev));
            Ok(())
        })),
        ..Default::default()
    };
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
    let ctx = egui::Context::default();
    app.run("file.new", json!({"width": 8, "height": 8})).unwrap();
    tick(&mut app, &ctx);
    autosave_now(&mut app);
    tick(&mut app, &ctx);
    assert!(saved.lock().unwrap().is_empty(), "clean documents are not autosaved");
    app.run("edit.fill", json!({"color": "#ff0000"})).unwrap();
    autosave_now(&mut app);
    tick(&mut app, &ctx);
    assert_eq!(saved.lock().unwrap().len(), 1);
    autosave_now(&mut app);
    tick(&mut app, &ctx);
    assert_eq!(saved.lock().unwrap().len(), 1, "unchanged since the last autosave");

    // Loaded copies can have the same persisted identity and dirty revision.
    let original = app.session.active().unwrap().doc.clone();
    let original_id = original.id;
    app.session.add_document(original.as_ref().clone(), None);
    app.run("edit.fill", json!({"color": "#0000ff"})).unwrap();
    autosave_now(&mut app);
    tick(&mut app, &ctx);
    {
        let saved = saved.lock().unwrap();
        assert_eq!(saved.len(), 2, "both dirty copies need independent autosaves");
        assert_eq!(saved[0].0, original_id.0);
        assert_ne!(saved[0].0, saved[1].0, "recovery ownership must be distinct");
        assert_eq!(saved[0].1, saved[1].1, "the revisions must match to reproduce suppression");
    }
    app.run("prefs.set", json!({"path": "fileHandling.autosave", "value": false})).unwrap();
    app.run("edit.fill", json!({"color": "#00ff00"})).unwrap();
    autosave_now(&mut app);
    tick(&mut app, &ctx);
    assert_eq!(saved.lock().unwrap().len(), 2, "autosave off");
}

#[test]
fn recovered_documents_adopt_their_entries_until_saved_or_closed() {
    type Log = Arc<Mutex<Vec<String>>>;
    let log: Log = Arc::default();
    let (l1, l2, l3) = (log.clone(), log.clone(), log.clone());
    use photocraft_doc::{Color, ColorMode, Document, SampleType, Size};
    let doc = || Document::with_background("R", Size::new(4, 4), ColorMode::Rgb, SampleType::U8, Color::WHITE);
    let services = crate::Services {
        autosave: Some(Box::new(move |d: &Arc<Document>, _: u64, _: Option<&str>| {
            l1.lock().unwrap().push(format!("save {}", d.id.0));
            Ok(())
        })),
        discard_autosave: Some(Box::new(move |id: u64| l2.lock().unwrap().push(format!("discard {id}")))),
        recover: Some(Box::new(move || {
            ["a", "b"].map(|key| crate::Recoverable { key: key.into(), name: "R".into(), path: None, load: Box::new(move || Ok(doc())) }).into()
        })),
        adopt_autosave: Some(Box::new(move |id: u64, key: &str| l3.lock().unwrap().push(format!("adopt {id} {key}")))),
        ..Default::default()
    };
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
    let ctx = egui::Context::default();
    tick(&mut app, &ctx);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while app.session.has_jobs() && std::time::Instant::now() < deadline {
        crate::jobs_ui::tick(&mut app, &ctx);
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    assert!(!app.session.has_jobs(), "recovery finished");
    let ids: Vec<u64> = app.session.documents().iter().map(|d| d.doc.id.0).collect();
    assert!(app.session.documents().iter().all(|d| d.is_dirty()), "recovered documents are unsaved");
    let take = || std::mem::take(&mut *log.lock().unwrap());
    assert_eq!(take(), [format!("adopt {} a", ids[0]), format!("adopt {} b", ids[1])]);
    // Their entries are current: nothing to autosave until they change.
    tick(&mut app, &ctx);
    autosave_now(&mut app);
    tick(&mut app, &ctx);
    assert!(take().is_empty());
    // Closing one drops its entry; editing the other autosaves over its own.
    app.run("file.close", json!({"document": 0})).unwrap();
    app.run("edit.fill", json!({"color": "#ff0000"})).unwrap();
    autosave_now(&mut app);
    tick(&mut app, &ctx);
    assert_eq!(take(), [format!("discard {}", ids[0]), format!("save {}", ids[1])]);
}

#[test]
fn edit_dialogs_open() {
    let (mut app, _) = app_with_store();
    let ctx = egui::Context::default();
    app.run("file.new", json!({"width": 32, "height": 32})).unwrap();
    app.sync_views();
    let d = crate::menus::invoke(&mut app, &ctx, "edit.colorSettings", json!({})).unwrap()["dialog"].as_u64().unwrap();
    assert_eq!(app.ui.dialogs.iter().find(|x| x.id == d).unwrap().fields["workingRgb"], "srgb");
    let note = app.ui.dialogs.iter().find(|x| x.id == d).unwrap().fields["__note"].as_str().unwrap().to_string();
    assert!(note.starts_with("Monitor profile in use: sRGB") && note.contains("fallback"), "{note}");
    assert!(crate::menus::invoke(&mut app, &ctx, "edit.fade", json!({})).is_err());
    app.run("select.rect", json!({"x": 4, "y": 4, "width": 8, "height": 8})).unwrap();
    // Content-Aware Fill opens its workspace (caf_ui); OK fills.
    let r = crate::menus::invoke(&mut app, &ctx, "edit.contentAwareFill", json!({})).unwrap();
    assert!(r["contentAwareFill"]["window"].is_array(), "{r}");
    crate::caf_ui::commit(&mut app);
    assert!(app.distort.caf.is_none());
    let r = crate::menus::invoke(&mut app, &ctx, "edit.contentAwareScale", json!({})).unwrap();
    assert_eq!(r["transform"]["rect"], json!([4.0, 4.0, 12.0, 12.0]));
    crate::transform_tool::cancel(&mut app);
    let d = crate::menus::invoke(&mut app, &ctx, "edit.presets.presetManager", json!({})).unwrap()["dialog"].as_u64().unwrap();
    assert!(crate::dialogs::confirm(&mut app, d).is_ok());
}
#[test]
fn reset_section_on_type_keeps_the_favourite_fonts() {
    // The favourites are the font menu's library, not a Type setting.
    use egui_kittest::{Harness, kittest::Queryable};
    let (mut app, store) = app_with_store();
    app.run("prefs.set", json!({"values": {"interface.language": "en", "type.smartQuotes": false}})).unwrap();
    app.run("type.font.favorite", json!({"family": "Bebas Neue"})).unwrap();
    open_preferences(&mut app, "type");
    let mut h = Harness::builder().with_size(vec2(1280.0, 800.0)).build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        app
    });
    h.run_steps(4);
    h.get_by_label("Reset Section").click();
    h.run_steps(2);
    h.get_by_label("OK").click();
    h.run_steps(4);
    let prefs = h.state().session.prefs().type_.clone();
    assert!(prefs.smart_quotes, "the Type settings are reset");
    assert_eq!(prefs.favorite_fonts, ["Bebas Neue"], "the favourites are kept");
    assert_eq!(stored(&store)["type"]["favoriteFonts"], json!(["Bebas Neue"]));
}
