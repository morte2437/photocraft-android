//! Behavioural checks for switching languages in a running shell.

use super::*;
use crate::{PhotocraftApp, Services};
use serde_json::json;

/// The File menu's title as the menu bar draws it in `lang`: the translation, plus the English
/// access key where it has no Latin letter of its own ("文件(F)", #5863).
fn file_title(lang: Lang) -> String {
    let (shown, _) = crate::menu_keys::localized(&[tr(lang, "File").to_string()], &["File"], &[Some('F')]);
    shown.into_iter().next().unwrap_or_default()
}

fn lang(code: &str) -> Lang {
    Lang::from_code(code).expect("registered language")
}

#[test]
fn requested_languages_resolve_regional_preferences() {
    for (tag, code) in [
        ("en-US", "en"),
        ("zh-CN", "zh-hans"),
        ("ja-JP", "ja"),
        ("ko-KR", "ko"),
        ("ru-RU", "ru"),
        ("uk-UA", "uk"),
        ("fr-CA", "fr"),
        ("id-ID", "id"),
        ("vi-VN", "vi"),
        ("sr-RS", "sr"),
    ] {
        assert_eq!(Lang::from_pref(tag), lang(code), "{tag}");
    }
    assert_eq!(lang_from_tag("ko_KR.UTF-8"), Some(lang("ko")));
    assert_eq!(lang_from_tag("FR-fr"), Some(lang("fr")));
    assert_eq!(lang_from_tag("ko"), Some(lang("ko")));
}

#[test]
fn ukrainian_native_app_menu_keeps_product_and_language_names() {
    with_language(Lang::EN, || {
        let h = harness();
        let layout = crate::native_menu::photocraft_layout(&crate::menus::menu_items(h.state()), lang("uk"), "uk");
        for (id, template) in [("help.about", "About {app}"), (crate::native_menu::HIDE, "Hide {app}"), ("file.exit", "Quit {app}")] {
            let label = fmt(tr(lang("uk"), template), &[("app", "PhotoCraft")]);
            assert!(label.contains("PhotoCraft"), "{id}: {label}");
            assert_eq!(layout.bar.find(id).map(|item| item.label.as_str()), Some(label.as_str()), "{id}");
        }
        assert_eq!(layout.bar.find("app.language.uk").map(|item| (item.label.as_str(), item.checked)), Some(("Українська", Some(true))));
    });
}

#[test]
fn ukrainian_color_picker_titles_draw_in_the_selected_language() {
    with_language(Lang::EN, || {
        for (target, title) in [("foreground", "Color Picker (Foreground Color)"), ("background", "Color Picker (Background Color)")] {
            let mut h = harness();
            h.state_mut().run("prefs.set", json!({"path": "interface.language", "value": "uk"})).unwrap();
            crate::color_picker_ui::open(h.state_mut(), target);
            h.run_steps(4);
            let expected = tr(lang("uk"), title);
            assert!(drawn_text(&h).iter().any(|text| text == expected), "missing title {expected}");
        }
    });
}

#[test]
fn ukrainian_rasterize_prompt_draws_in_the_selected_language() {
    use crate::rasterize_prompt::Kind;

    with_language(Lang::EN, || {
        let mut h = harness();
        h.state_mut().run("prefs.set", json!({"path": "interface.language", "value": "uk"})).unwrap();
        h.state_mut().run("type.create", json!({"text": "Editable text"})).unwrap();
        let layer = h.state().session.active().unwrap().active_layer.unwrap();
        crate::rasterize_prompt::open(h.state_mut(), Kind::Type, layer, crate::state::Tool::Brush, [10.0, 10.0, 1.0]);
        h.run_steps(4);
        let expected = tr(lang("uk"), Kind::Type.message());
        let text = drawn_text(&h);
        assert!(text.iter().any(|text| text == expected), "the actual dialog draws the Ukrainian warning");
    });
}

#[test]
fn scoped_preview_restores_language_even_after_a_panic() {
    with_language(Lang::EN, || {
        with_language(lang("fr"), || {
            assert_eq!(current(), lang("fr"));
            assert_eq!(t("Layer"), tr(lang("fr"), "Layer"));
            with_language(lang("ko"), || assert_eq!(current(), lang("ko")));
            assert_eq!(current(), lang("fr"));
        });
        let result = std::panic::catch_unwind(|| with_language(lang("ja"), || panic!("test unwind")));
        assert!(result.is_err());
        assert_eq!(current(), Lang::EN);
    });
}

#[test]
fn drawing_languages_are_isolated_between_threads() {
    with_language(lang("fr"), || {
        let thread = std::thread::spawn(|| {
            assert_eq!(current(), Lang::EN);
            with_language(lang("ko"), || assert_eq!(t("Layer"), tr(lang("ko"), "Layer")));
        });
        thread.join().expect("language thread");
        assert_eq!(current(), lang("fr"));
    });
}

#[test]
fn dynamic_brush_sections_draw_in_the_selected_language() {
    with_language(Lang::EN, || {
        let mut h = harness();
        h.state_mut().run("prefs.set", json!({"path": "interface.language", "value": "zh-hans"})).expect("language");
        h.state_mut().ui.panels.brush_settings = true;
        h.run_steps(4);
        let text = drawn_text(&h);
        // The section row is the only place the name is drawn: the options pane has no heading (#3477).
        let expected = tr(lang("zh-hans"), "Brush Tip Shape");
        assert_eq!(text.iter().filter(|text| *text == expected).count(), 1, "the section row must be translated");
    });
}

#[test]
fn fill_dialog_draws_in_the_selected_language() {
    with_language(Lang::EN, || {
        let mut h = harness();
        h.state_mut().run("prefs.set", json!({"path": "interface.language", "value": "pt-br"})).expect("language");
        let ctx = h.ctx.clone();
        crate::menus::invoke(h.state_mut(), &ctx, crate::fill_ui::COMMAND, json!({})).expect("fill dialog");
        h.run_steps(4);
        let text = drawn_text(&h);
        for source in ["Contents:", "Foreground Color", "Mode:", "Opacity:"] {
            let want = tr(lang("pt-br"), source);
            assert!(text.iter().any(|text| text == want), "missing {want}");
        }
    });
}

#[test]
fn file_info_camera_data_draws_in_the_selected_language_read_only() {
    with_language(Lang::EN, || {
        let mut h = harness();
        let exif = photocraft_algo::exif::build(&photocraft_algo::exif::CameraInfo {
            make: Some("SONY".into()),
            exposure_time: Some(1.0 / 250.0),
            f_number: Some(8.0),
            ..Default::default()
        });
        h.state_mut()
            .session
            .edit("exif", |doc, _| {
                doc.metadata.exif = Some(std::sync::Arc::new(exif));
                Ok(())
            })
            .expect("exif");
        h.state_mut().run("prefs.set", json!({"path": "interface.language", "value": "de"})).expect("language");
        let ctx = h.ctx.clone();
        let dialog = crate::menus::invoke(h.state_mut(), &ctx, "file.fileInfo", json!({})).expect("file info")["dialog"].as_u64().expect("dialog id");
        assert_eq!(h.state_mut().ui.dialog_mut(dialog).expect("open").fields["__camera"]["exposure"], "1/250 s");
        h.run_steps(4);
        let text = drawn_text(&h);
        let de = lang("de");
        let labels = [tr(de, "Camera Data"), tr_ctx(de, "cameraData", "Make"), tr_ctx(de, "cameraData", "Exposure"), tr_ctx(de, "cameraData", "F-Stop")];
        for want in labels.into_iter().chain(["SONY", "1/250 s", "f/8"]) {
            assert!(text.iter().any(|text| text == want), "missing {want}: {text:?}");
        }
        // Read-only: confirming submits only the editable fields.
        crate::dialogs::confirm(h.state_mut(), dialog).expect("confirm");
        assert_eq!(h.state_mut().run("file.fileInfo", json!({})).expect("info")["camera"]["make"], "SONY");
    });
}

#[test]
fn cjk_font_order_follows_the_selected_language() {
    use photocraft_text::cjk::CjkScript;
    for (code, script) in [("zh-hans", CjkScript::SimplifiedChinese), ("ja", CjkScript::Japanese), ("ko", CjkScript::Korean)] {
        with_language(lang(code), || {
            let mut fallback = crate::cjk_fonts::CjkFallback::new(crate::cjk_fonts::Sources::system());
            assert_eq!(fallback.order().first(), Some(&script), "{code}");
        });
    }
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn korean_glyphs_remain_available_after_repeated_language_switches() {
    use photocraft_text::cjk::{self, CjkChar, CjkScript};
    #[cfg(target_os = "linux")]
    assert!(crate::cjk_fonts::wait_for_fontconfig_cjk_scan(), "Fontconfig CJK lookup did not finish within 10 seconds");
    if !cjk::font_files(CjkScript::Korean).iter().any(|font| font.path.is_file()) {
        eprintln!("skipping: no installed Korean font");
        return;
    }
    with_language(Lang::EN, || {
        let mut h = harness();
        let glyphs: std::collections::BTreeSet<_> = lang("ko").0.text().chars().filter(|c| cjk::classify(*c) == Some(CjkChar::Hangul)).collect();
        for code in ["zh-hans", "ja", "ko", "fr", "ko"] {
            h.state_mut().run("prefs.set", json!({"path": "interface.language", "value": code})).expect("language");
            h.run_steps(12);
            if code == "ko" {
                h.ctx.fonts_mut(|fonts| {
                    for family in [egui::FontFamily::Proportional, egui::FontFamily::Name("medium".into()), egui::FontFamily::Name("semibold".into())] {
                        let font = egui::FontId::new(12.0, family);
                        for c in &glyphs {
                            assert!(fonts.has_glyph(&font, *c), "missing Korean glyph {c} after a hot language change");
                        }
                    }
                });
            }
        }
    });
}

fn harness() -> egui_kittest::Harness<'static, PhotocraftApp> {
    harness_with_services(Services::default())
}

fn harness_with_services(services: Services) -> egui_kittest::Harness<'static, PhotocraftApp> {
    let mut harness = egui_kittest::Harness::builder().with_size(egui::vec2(1200.0, 800.0)).with_max_steps(64).build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
        app.run("file.new", json!({"name": "User layer name", "width": 64, "height": 64})).expect("new document");
        app
    });
    harness.run_steps(4);
    harness
}

#[test]
fn first_launch_renders_the_system_language_without_overwriting_auto() {
    for (tag, code) in [
        ("en-US", "en"),
        ("zh-CN", "zh-hans"),
        ("ja-JP", "ja"),
        ("ko-KR", "ko"),
        ("ru-RU", "ru"),
        ("uk-UA", "uk"),
        ("fr-CA", "fr"),
        ("sv-SE", "sv"),
        ("fi-FI", "fi"),
        ("da-DK", "da"),
        ("nb-NO", "nb"),
    ] {
        system::with_system_tags(&[tag], || {
            with_language(Lang::EN, || {
                let h = harness();
                assert_eq!(h.state().session.prefs().interface.language, "auto");
                assert_eq!(current(), lang(code), "{tag}");
                assert!(drawn_text(&h).iter().any(|text| *text == file_title(lang(code))), "first-launch menu: {tag}");
            });
        });
    }
}

#[test]
fn missing_or_unsupported_system_languages_render_english() {
    for tags in [vec![], vec!["sw-KE", "is-IS"]] {
        system::with_system_tags(&tags, || {
            with_language(Lang::EN, || {
                let h = harness();
                assert_eq!(h.state().session.prefs().interface.language, "auto");
                assert_eq!(current(), Lang::EN);
                assert!(drawn_text(&h).iter().any(|text| text == "File"));
            });
        });
    }
}

#[test]
fn older_preferences_without_a_language_follow_the_system() {
    system::with_system_tags(&["fr-CA"], || {
        with_language(Lang::EN, || {
            let services = Services { load_prefs: Some(Box::new(|| Some(r#"{"interface":{"uiScale":"125"}}"#.into()))), ..Default::default() };
            let h = harness_with_services(services);
            assert_eq!(h.state().session.prefs().interface.language, "auto");
            assert_eq!(h.state().session.prefs().interface.ui_scale, photocraft_engine::prefs::UiScale::P125);
            assert_eq!(current(), lang("fr"));
        });
    });
}

#[test]
fn an_explicit_language_survives_restart_on_a_different_system_language() {
    let saved = system::with_system_tags(&["ja-JP"], || {
        with_language(Lang::EN, || {
            let mut h = harness();
            assert_eq!(current(), lang("ja"));
            h.state_mut().run("prefs.set", json!({"path":"interface.language","value":"ko"})).expect("choose language");
            h.step();
            assert_eq!(current(), lang("ko"));
            h.state().session.prefs_to_json()
        })
    });
    system::with_system_tags(&["fr-FR"], || {
        with_language(Lang::EN, || {
            let services = Services { load_prefs: Some(Box::new(move || Some(saved.clone()))), ..Default::default() };
            let h = harness_with_services(services);
            assert_eq!(h.state().session.prefs().interface.language, "ko");
            assert_eq!(current(), lang("ko"));
            assert!(drawn_text(&h).iter().any(|text| *text == file_title(lang("ko"))));
        });
    });
}

#[test]
fn a_control_request_changes_the_drawing_language_in_the_same_frame() {
    with_language(Lang::EN, || {
        let mut h = harness();
        let document = h.state().session.active().expect("document").doc.clone();
        for code in ["zh-hans", "ja", "ko", "ru", "uk", "fr", "id", "en"] {
            let (tx, rx) = std::sync::mpsc::channel();
            h.state_mut().control_rx = Some(rx);
            let (request, reply) =
                crate::control::ControlRequest::new("engine.execute", json!({"command": "prefs.set", "params": {"path": "interface.language", "value": code}}));
            tx.send(request).expect("request");
            h.step();
            assert_eq!(reply.try_recv().expect("response")["ok"], true);
            assert_eq!(current(), lang(code));
            assert!(drawn_text(&h).iter().any(|text| *text == file_title(lang(code))), "the menu must change in this frame: {code}");
            assert!(std::sync::Arc::ptr_eq(&document, &h.state().session.active().expect("document").doc), "switching language must preserve the document");
        }
        assert!(h.state_mut().run("prefs.set", json!({"path": "interface.language", "value": []})).is_err());
        h.step();
        assert_eq!(current(), Lang::EN);
    });
}

#[test]
fn preferences_preview_does_not_commit_until_applied_and_survives_reload() {
    with_language(Lang::EN, || {
        let mut h = harness();
        let id = crate::prefs_ui::open_preferences(h.state_mut(), "interface");
        let dialog = h.state_mut().ui.dialog_mut(id).expect("preferences");
        dialog.fields.get_mut("values").expect("values")["interface"]["language"] = json!("fr");
        h.run_steps(3);
        let text = drawn_text(&h);
        let fr = lang("fr");
        assert!(text.iter().any(|text| text == tr(fr, "Appearance")), "the draft language must translate the visible dialog");
        assert!(text.iter().any(|text| text == tr(fr, "Cancel")), "the dialog buttons must preview the language too");
        assert!(text.iter().any(|text| text == "File"), "the main window keeps its committed language");
        assert_eq!(h.state().session.prefs().interface.language, "auto");
        assert_eq!(current(), Lang::EN, "preview must not leak to the main window");
        h.state_mut().ui.close_dialog(id);
        h.step();
        assert_eq!(current(), Lang::EN);
        let id = crate::prefs_ui::open_preferences(h.state_mut(), "interface");
        h.state_mut().ui.dialog_mut(id).expect("preferences").fields.get_mut("values").expect("values")["interface"]["language"] = json!("ko");
        crate::prefs_ui::apply(h.state_mut(), id).expect("apply");
        h.step();
        assert_eq!(current(), lang("ko"));
        let saved = h.state().session.prefs_to_json();
        let mut restored = photocraft_engine::Session::new();
        restored.load_prefs_json(&saved).expect("reload preferences");
        assert_eq!(restored.prefs().interface.language, "ko");
    });
}

#[test]
fn keyboard_shortcuts_list_commands_in_the_selected_language() {
    with_language(Lang::EN, || {
        let mut h = harness();
        h.state_mut().run("prefs.set", json!({"path": "interface.language", "value": "pt-br"})).expect("language");
        h.step();
        let ctx = h.ctx.clone();
        let id = crate::menus::invoke(h.state_mut(), &ctx, "edit.keyboardShortcuts", json!({})).expect("dialog")["dialog"].as_u64().expect("id");
        let pt = lang("pt-br");
        let label = tr_id(pt, "file.saveAs", "Save As…");
        // Search matches the translated label and the English one.
        for filter in [label.trim_end_matches('…').to_lowercase(), "save as".to_string()] {
            h.state_mut().ui.dialog_mut(id).expect("shortcuts").fields.insert("filter".into(), json!(filter));
            h.run_steps(3);
            let text = drawn_text(&h);
            assert!(text.iter().any(|text| text == tr(pt, "File")), "{filter}: the menu heading must be translated");
            assert!(text.iter().any(|text| text.trim() == label), "{filter}: the command must be translated: {text:?}");
        }
    });
}

fn drawn_text(h: &egui_kittest::Harness<'_, PhotocraftApp>) -> Vec<String> {
    fn collect(shape: &egui::Shape, text: &mut Vec<String>) {
        match shape {
            egui::Shape::Text(shape) => text.push(shape.galley.job.text.clone()),
            egui::Shape::Vec(shapes) => shapes.iter().for_each(|shape| collect(shape, text)),
            _ => {}
        }
    }
    let mut text = Vec::new();
    for shape in &h.output().shapes {
        collect(&shape.shape, &mut text);
    }
    text
}

#[test]
fn czech_status_notices_and_navigator_draw_translated_text() {
    with_language(Lang::EN, || {
        let mut h = harness();
        h.state_mut().run("prefs.set", json!({"path": "interface.language", "value": "cs"})).expect("language");
        h.state_mut().ui.panels.navigator = true;
        h.run_steps(4);
        with_language(lang("cs"), || {
            h.state_mut().open_failed("English file.psd", "disk full");
            crate::notices::post(
                h.state_mut(),
                "Preferences can't be saved",
                vec!["PhotoCraft keeps retrying; until a save succeeds, preference changes are lost when it closes.".into()],
                true,
                None,
            );
            let info = crate::gpu_canvas::GpuInfo { driver: "Mesa 25.0".into(), ..Default::default() };
            assert!(info.lines().iter().any(|line| *line == fmt(tr(lang("cs"), "Driver: {driver}"), &[("driver", "Mesa 25.0")])));
        });
        h.run_steps(4);
        let text = drawn_text(&h);
        let cs = lang("cs");
        // The Navigator's own text is its tab title now: its zoom row has no label (#4840).
        for want in [
            tr(cs, "Navigator").to_string(),
            fmt(tr(cs, "Couldn't open {name}: {error}"), &[("name", "English file.psd"), ("error", "disk full")]),
            tr(cs, "Preferences can't be saved").to_string(),
        ] {
            assert!(text.contains(&want), "missing {want}: {text:?}");
        }
        // Czech text must be drawable by the bundled fonts, including the new notices.
        let chars: std::collections::BTreeSet<_> = text.iter().flat_map(|s| s.chars()).filter(|c| !c.is_whitespace()).collect();
        h.ctx.fonts_mut(|fonts| {
            let font = egui::FontId::proportional(12.0);
            for c in chars {
                assert!(fonts.has_glyph(&font, c), "missing glyph {c}");
            }
        });
    });
}

#[test]
fn czech_shortcut_conflicts_keep_warning_state_and_shortcut_codes() {
    with_language(Lang::EN, || {
        let mut h = harness();
        h.state_mut().run("prefs.set", json!({"path": "interface.language", "value": "cs"})).expect("language");
        let ctx = h.ctx.clone();
        let result = crate::menus::invoke(h.state_mut(), &ctx, "edit.keyboardShortcuts", json!({})).expect("shortcut dialog");
        let id = result["dialog"].as_u64().expect("dialog id");
        h.run_steps(4);
        let fields = &mut h.state_mut().ui.dialogs.iter_mut().find(|d| d.id == id).expect("dialog").fields;
        fields.insert("selected".into(), json!("file.new"));
        fields.insert("capture".into(), json!(true));
        let mods = egui::Modifiers { ctrl: true, command: true, ..Default::default() };
        h.event(egui::Event::Key { key: egui::Key::O, physical_key: None, pressed: true, repeat: false, modifiers: mods });
        h.event(egui::Event::Key { key: egui::Key::O, physical_key: None, pressed: false, repeat: false, modifiers: mods });
        h.run_steps(4);
        let fields = &h.state().ui.dialogs.iter().find(|d| d.id == id).expect("dialog").fields;
        assert_eq!(fields["messageWarning"], json!(true));
        let message = fields["message"].as_str().expect("message");
        let template = tr(lang("cs"), "{shortcut} is already in use by {commands} and will be removed from it when you click OK.");
        // The message is the conflict template, filled in: its fixed text appears around the shortcut and commands.
        for piece in template.split('{').map(|piece| piece.split_once('}').map_or(piece, |(_, rest)| rest)).filter(|piece| !piece.trim().is_empty()) {
            assert!(message.contains(piece), "{message:?} lacks {piece:?}");
        }
        assert_eq!(fields["overrides"]["file.new"], json!("Cmd+O"));
    });
}

/// The catalog stays in logical order: egui puts a right-to-left label into display order itself,
/// so the first word of an Arabic label lands to the right of its last, and a Latin brand name at
/// the end of the sentence lands at the left.
#[test]
fn arabic_labels_are_laid_out_right_to_left() {
    with_language(Lang::EN, || {
        let mut h = harness();
        h.state_mut().run("prefs.set", json!({"path": "interface.language", "value": "ar"})).unwrap();
        h.run_steps(4);
        let x_of = |text: &str, c: char| {
            let galley = h.ctx.fonts_mut(|f| f.layout_no_wrap(text.to_string(), egui::FontId::proportional(13.0), egui::Color32::WHITE));
            galley.rows.iter().flat_map(|r| r.glyphs.iter()).find(|g| g.chr == c).map(|g| g.pos.x).expect("glyph")
        };
        let label = tr(lang("ar"), "No properties");
        assert_eq!(label, "لا توجد خصائص");
        assert!(x_of(label, 'ل') > x_of(label, 'ص'), "{label}: first word at the right");
        let join = tr(lang("ar"), "Join us on Discord");
        assert!(x_of(join, 'D') < x_of(join, 'ن'), "{join}: Discord at the left");
    });
}

/// #5159 on egui 0.37: user names are handed to egui as typed, and egui orders the words. An
/// Arabic layer name reads from the right; the title line keeps "— PhotoCraft" at the right of a
/// right-to-left document name.
#[test]
fn arabic_names_are_laid_out_right_to_left() {
    with_language(Lang::EN, || {
        let h = harness();
        let font = egui::FontId::proportional(13.0);
        let x_in = |galley: &egui::Galley, c: char| galley.rows.iter().flat_map(|r| r.glyphs.iter()).find(|g| g.chr == c).map(|g| g.pos.x).expect("glyph");
        let name = "لاشي مكتوب هنا";
        let galley = h.ctx.fonts_mut(|f| f.layout_no_wrap(name.to_string(), font.clone(), egui::Color32::WHITE));
        assert!(x_in(&galley, 'ل') > x_in(&galley, 'ه'), "{name}: first word at the right");
        let title = "لاشي مكتوب.png — PhotoCraft";
        let job = crate::panels::title_bar::title_job(title, font, egui::Color32::WHITE);
        let galley = h.ctx.fonts_mut(|f| f.layout_job(job));
        assert!(x_in(&galley, 'ل') > x_in(&galley, 'م'), "{title}: the name's first word right of its second");
        assert!(x_in(&galley, 'P') > x_in(&galley, 'ل'), "{title}: PhotoCraft stays at the right");
    });
}
