//! Selected-range font acceptance, caret typing styles and focused global commit.
use crate::{PhotocraftApp, Services};
use egui::{Key, Modifiers, accesskit::Role, vec2};
use egui_kittest::{Harness, kittest::Queryable};
use photocraft_doc::{Document, LayerContent, LayerId};
use serde_json::{Value, json};

const FONTS: [&str; 2] = ["Inter", "JetBrains Mono"];

struct Probe {
    app: PhotocraftApp,
    character: bool,
}

impl eframe::App for Probe {
    fn logic(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        crate::shortcuts::handle(&mut self.app, ctx);
        crate::type_font_preview::follow_hover(&mut self.app, ctx);
    }
    fn ui(&mut self, ui: &mut egui::Ui, _: &mut eframe::Frame) {
        if self.character {
            ui.vertical(|ui| {
                ui.set_width(340.0);
                super::character_panel(&mut self.app, ui, false);
            });
        }
        ui.horizontal(|ui| super::options_bar(&mut self.app, ui));
    }
}

fn layer(doc: &Document, id: LayerId) -> &photocraft_doc::TextLayer {
    let LayerContent::Text(t) = &doc.layer(id).unwrap().content else { panic!("text layer") };
    t
}

fn styles(doc: &Document, id: LayerId) -> Value {
    let t = layer(doc, id);
    let runs = t.char_runs();
    let mut at = 0;
    let mut chars = Vec::new();
    for r in runs {
        let end = at + r.len;
        for ch in t.text.get(at..end).unwrap().chars() {
            chars.push(json!({"char": ch.to_string(), "font": r.style.font_family,
                "face": r.style.postscript_name, "weight": r.style.weight, "italic": r.style.italic}));
        }
        at = end;
    }
    json!({"text": t.text, "summary": t.font_family, "chars": chars})
}

fn saved(h: &Harness<'_, Probe>, id: LayerId) -> Value {
    styles(&h.state().app.session.active().unwrap().doc, id)
}

fn families(s: &Value) -> Vec<String> {
    s["chars"].as_array().unwrap().iter().map(|c| c["font"].as_str().unwrap().to_owned()).collect()
}

fn fields(h: &Harness<'_, Probe>) -> Vec<Option<String>> {
    h.query_all_by_role(Role::TextInput).map(|n| n.value()).collect()
}

fn select(h: &mut Harness<'_, Probe>, range: [usize; 2]) {
    let e = h.state_mut().app.ui.text_edit.as_mut().unwrap();
    e.anchor = range[0];
    e.caret = range[1];
}

fn fixture(new: bool, pattern: usize, character: bool, content: &str) -> (Harness<'static, Probe>, LayerId) {
    fixture_renderer(new, pattern, character, content, false, 1.0)
}

fn fixture_renderer(new: bool, pattern: usize, character: bool, content: &str, gpu: bool, scale: f32) -> (Harness<'static, Probe>, LayerId) {
    let builder = Harness::builder().with_size(vec2(1100.0, 1000.0)).with_pixels_per_point(scale).with_max_steps(64);
    let builder = if gpu { builder.wgpu() } else { builder };
    let mut h = builder.build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Services::default());
        app.session.edit_prefs(|p| p.type_.font_preview = photocraft_engine::prefs::FontPreview::Off);
        Probe { app, character }
    });
    let app = &mut h.state_mut().app;
    app.run("file.new", json!({"width": 240, "height": 160})).unwrap();
    app.sync_views();
    app.ui.tool = crate::state::Tool::Type;
    app.ui.tool_options.type_font = FONTS[pattern % 2].into();
    app.ui.tool_options.type_size = 18.0;
    super::pointer_up(app, [30.0, 70.0], [30.0, 70.0]);
    super::insert(app, content);
    let id = LayerId(app.ui.text_edit.as_ref().unwrap().layer);
    let key = app.ui.text_edit.as_ref().unwrap().session.clone();
    // Build mixed styles through the same live edit session, preserving history/coalescing.
    if pattern >= 2 {
        app.run("type.setStyle", json!({"layer": id.0, "range": [2,4], "font": FONTS[1-pattern%2], "coalesce": key})).unwrap();
    }
    if !new {
        super::commit(app);
        super::edit_active(app).unwrap();
    }
    h.run_steps(3);
    (h, id)
}

fn choose(h: &mut Harness<'_, Probe>, font: &str, accept: &str) {
    h.query_all_by_role(Role::TextInput).next().unwrap().click();
    h.run();
    h.query_all_by_role(Role::TextInput).next().unwrap().type_text(font);
    h.run();
    match accept {
        "enter" => h.key_press(Key::Enter),
        "arrow-enter" => {
            h.key_press(Key::ArrowDown);
            h.run();
            h.key_press(Key::Enter);
        }
        "click" => h.get_by_label(font).click(),
        "hover" => h.get_by_label(font).hover(),
        "query" => {}
        "tab" => h.key_press(Key::Tab),
        "escape" => h.key_press(Key::Escape),
        _ => panic!("unknown acceptance"),
    }
    h.run();
}

fn commit(h: &mut Harness<'_, Probe>, method: &str) {
    match method {
        "checkmark" => {
            let label = crate::i18n::fmt("Commit any current edits ({key})", &[("key", &crate::shortcuts::pretty("Cmd+Enter"))]);
            h.get_by_role_and_label(Role::Button, &label).click();
            h.run();
        }
        "outside" => {
            super::pointer_down(&mut h.state_mut().app, 230.0, 150.0, Modifiers::NONE);
            h.run();
        }
        "direct" => {
            super::commit(&mut h.state_mut().app);
            h.run();
        }
        "shortcut-focused" => {
            h.key_press_modifiers(Modifiers::COMMAND, Key::Enter);
            h.run();
        }
        "shortcut-unfocused" => {
            h.ctx.memory_mut(|m| {
                if let Some(id) = m.focused() {
                    m.surrender_focus(id);
                }
            });
            h.key_press_modifiers(Modifiers::COMMAND, Key::Enter);
            h.run();
        }
        "layer-switch" => {
            h.state_mut().app.run("layer.new.layer", json!({})).unwrap();
            super::end_if_left(&mut h.state_mut().app);
            h.run();
        }
        "document-switch" => {
            h.state_mut().app.run("file.new", json!({"width": 80, "height": 60})).unwrap();
            super::end_if_left(&mut h.state_mut().app);
            h.state_mut().app.session.set_active(0);
            h.run();
        }
        _ => panic!("unknown commit"),
    }
}

#[test]
fn selected_fonts_commit_pattern_0() {
    selected_fonts_commit_cases(0);
}

#[test]
fn selected_fonts_commit_pattern_1() {
    selected_fonts_commit_cases(1);
}

#[test]
fn selected_fonts_commit_pattern_2() {
    selected_fonts_commit_cases(2);
}

#[test]
fn selected_fonts_commit_pattern_3() {
    selected_fonts_commit_cases(3);
}

fn selected_fonts_commit_cases(pattern: usize) {
    let mut cases = 0;
    for new in [false, true] {
        for character in [false, true] {
            for range in [[0, 2], [2, 4], [4, 6], [0, 6], [4, 2], [1, 5]] {
                for target in FONTS {
                    for accept in ["enter", "click", "arrow-enter"] {
                        for method in ["checkmark", "outside"] {
                            let label = (new, pattern, character, range, target, accept, method);
                            let (mut h, id) = fixture(new, pattern, character, "abcdef");
                            select(&mut h, range);
                            h.run();
                            let mut expected = families(&saved(&h, id));
                            for font in &mut expected[range[0].min(range[1])..range[0].max(range[1])] {
                                *font = target.into();
                            }
                            choose(&mut h, target, accept);
                            let applied = saved(&h, id);
                            assert_eq!(families(&applied), expected, "{label:?}");
                            assert_eq!(fields(&h)[0].as_deref(), Some(target), "selected font caption: {label:?}");
                            let pixels = layer(&h.state().app.session.active().unwrap().doc, id).cache.clone();
                            commit(&mut h, method);
                            assert!(h.state().app.ui.text_edit.is_none(), "{label:?}");
                            assert_eq!(saved(&h, id), applied, "{label:?}");
                            assert_eq!(layer(&h.state().app.session.active().unwrap().doc, id).cache, pixels, "{label:?}");
                            let uniform = expected.iter().all(|f| f == &expected[0]);
                            assert_eq!(
                                fields(&h)[0].clone().unwrap_or_default(),
                                if uniform { expected[0].clone() } else { String::new() },
                                "mixed caption after commit: {label:?}"
                            );
                            super::edit_active(&mut h.state_mut().app).unwrap();
                            select(&mut h, range);
                            h.run();
                            assert_eq!(fields(&h)[0].as_deref(), Some(target), "reselected caption: {label:?}");
                            cases += 1;
                        }
                    }
                }
            }
        }
    }
    assert_eq!(cases, 288);
}

#[test]
fn caret_font_choices_style_only_subsequent_typing_including_ime() {
    for new in [false, true] {
        for character in [false, true] {
            for original in 0..2 {
                for text in ["abcdef", "AéB👩‍💻", "ab\ncdé"] {
                    let n = text.chars().count();
                    for caret in [0, n / 2, n] {
                        for accept in ["enter", "click", "arrow-enter"] {
                            let (mut h, id) = fixture(new, original, character, text);
                            select(&mut h, [caret, caret]);
                            h.run();
                            let before = saved(&h, id);
                            let steps = h.state().app.session.active().unwrap().history.entries().len();
                            let font = FONTS[1 - original];
                            choose(&mut h, font, accept);
                            assert_eq!(saved(&h, id), before, "a font choice at a caret never restyles existing characters");
                            assert_eq!(h.state().app.session.active().unwrap().history.entries().len(), steps);
                            assert_eq!(fields(&h)[0].as_deref(), Some(font));
                            crate::type_font_preview::set(&mut h.state_mut().app, font.into());
                            assert!(crate::type_font_preview::display_doc(&mut h.state_mut().app, 0).is_none());
                            // Multiple preedit replacements and final commit keep the pending font.
                            super::ime_update(&mut h.state_mut().app, "é", false);
                            super::ime_update(&mut h.state_mut().app, "éX", true);
                            super::insert(&mut h.state_mut().app, "Z");
                            let mut expected = families(&before);
                            expected.splice(caret..caret, std::iter::repeat_n(font.to_string(), 3));
                            assert_eq!(families(&saved(&h, id)), expected);
                            let typed = saved(&h, id);
                            commit(&mut h, "checkmark");
                            assert_eq!(saved(&h, id), typed);
                            assert!(h.state().app.ui.type_insertion_style.is_none());
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn global_commit_accepts_pending_font_queries_and_leaves_no_focused_field() {
    for new in [false, true] {
        for character in [false, true] {
            for accept in ["enter", "click", "arrow-enter", "query", "hover", "tab", "escape"] {
                for method in ["shortcut-focused", "shortcut-unfocused", "layer-switch", "document-switch"] {
                    let (mut h, id) = fixture(new, 0, character, "abcdef");
                    select(&mut h, [2, 4]);
                    h.run();
                    choose(&mut h, FONTS[1], accept);
                    commit(&mut h, method);
                    assert!(h.state().app.ui.text_edit.is_none(), "{new}/{character}/{accept}/{method}");
                    let mut expected = vec![FONTS[0].to_string(); 6];
                    if matches!(accept, "enter" | "click" | "arrow-enter") || (method == "shortcut-focused" && matches!(accept, "query" | "hover")) {
                        expected[2..4].fill(FONTS[1].into());
                    }
                    assert_eq!(families(&saved(&h, id)), expected, "{new}/{character}/{accept}/{method}");
                }
            }
        }
    }
}

#[test]
fn global_commit_clears_hover_before_preview_can_expand_to_the_whole_layer() {
    let (mut h, id) = fixture(false, 2, false, "abcdef");
    select(&mut h, [2, 4]);
    h.run();
    choose(&mut h, FONTS[1], "hover");
    h.event(egui::Event::Key { key: Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::COMMAND });
    h.run_steps(1);
    assert!(h.state().app.ui.text_edit.is_none());
    let (shown, _) = crate::canvas::display_doc(&mut h.state_mut().app, 0);
    assert_eq!(styles(&shown, id), saved(&h, id), "the first committed frame must show real runs, not a full-layer hover preview");
}

#[test]
fn caret_style_is_discarded_by_navigation_cancel_undo_and_document_switch() {
    for action in ["navigation", "cancel", "undo", "document-switch"] {
        let (mut h, id) = fixture(false, 0, false, "abcdef");
        select(&mut h, [3, 3]);
        h.run();
        choose(&mut h, FONTS[1], "enter");
        match action {
            "navigation" => {
                h.ctx.memory_mut(|m| {
                    if let Some(id) = m.focused() {
                        m.surrender_focus(id);
                    }
                });
                h.key_press(Key::ArrowLeft);
                h.run();
                h.key_press(Key::ArrowRight);
                h.run();
                super::insert(&mut h.state_mut().app, "Z");
                assert!(families(&saved(&h, id)).iter().all(|f| f == FONTS[0]));
            }
            "cancel" => {
                super::cancel(&mut h.state_mut().app);
                assert!(h.state().app.ui.type_insertion_style.is_none());
            }
            "undo" => {
                h.state_mut().app.run("type.setStyle", json!({"layer":id.0,"range":[0,1],"font":FONTS[1]})).unwrap();
                assert!(h.state_mut().app.session.undo());
                assert!(super::insertion_style::pending(&h.state().app).is_none());
            }
            _ => {
                commit(&mut h, "document-switch");
                assert!(h.state().app.ui.type_insertion_style.is_none());
            }
        }
    }
}

#[test]
fn returning_focus_to_the_same_canvas_caret_keeps_the_chosen_typing_font() {
    let (mut h, id) = fixture(false, 0, false, "abcdef");
    select(&mut h, [6, 6]);
    h.run();
    choose(&mut h, FONTS[1], "enter");
    let (layout, aff, text) = super::layout(&mut h.state_mut().app, id).unwrap();
    let (x, top, bottom) = layout.caret(text.len());
    let p = aff.apply(photocraft_geom::Point::new(f64::from(x) - 0.1, f64::from((top + bottom) * 0.5)));
    assert!(super::pointer_down(&mut h.state_mut().app, p.x, p.y, Modifiers::NONE));
    super::pointer_up(&mut h.state_mut().app, [p.x, p.y], [p.x, p.y]);
    assert_eq!(h.state().app.ui.text_edit.as_ref().unwrap().caret, 6);
    assert!(super::insertion_style::pending(&h.state().app).is_some());
    h.ctx.memory_mut(|m| {
        if let Some(id) = m.focused() {
            m.surrender_focus(id);
        }
    });
    h.event(egui::Event::Text("XYZ".into()));
    h.run();
    assert_eq!(families(&saved(&h, id)), [FONTS[0], FONTS[0], FONTS[0], FONTS[0], FONTS[0], FONTS[0], FONTS[1], FONTS[1], FONTS[1]]);
}

#[test]
fn backspace_and_delete_preserve_pending_caret_font() {
    for key in [Key::Backspace, Key::Delete] {
        let (mut h, id) = fixture(false, 0, false, "abcdef");
        select(&mut h, [3, 3]);
        h.run();
        choose(&mut h, FONTS[1], "enter");
        h.ctx.memory_mut(|m| {
            if let Some(id) = m.focused() {
                m.surrender_focus(id);
            }
        });
        h.key_press(key);
        h.run();
        assert!(super::insertion_style::pending(&h.state().app).is_some());
        h.event(egui::Event::Text("Z".into()));
        h.run();
        let expected = if key == Key::Backspace {
            vec![FONTS[0], FONTS[0], FONTS[1], FONTS[0], FONTS[0], FONTS[0]]
        } else {
            vec![FONTS[0], FONTS[0], FONTS[0], FONTS[1], FONTS[0], FONTS[0]]
        };
        assert_eq!(families(&saved(&h, id)), expected);
    }

    // Multiple backspaces in a row still preserve the pending font
    let (mut h, id) = fixture(false, 0, false, "abcdef");
    select(&mut h, [3, 3]);
    h.run();
    choose(&mut h, FONTS[1], "enter");
    h.ctx.memory_mut(|m| {
        if let Some(id) = m.focused() {
            m.surrender_focus(id);
        }
    });
    h.key_press(Key::Backspace);
    h.run();
    h.key_press(Key::Backspace);
    h.run();
    assert!(super::insertion_style::pending(&h.state().app).is_some());
    h.event(egui::Event::Text("Z".into()));
    h.run();
    assert_eq!(families(&saved(&h, id)), [FONTS[0], FONTS[1], FONTS[0], FONTS[0], FONTS[0]]);
}

#[test]
fn commit_shortcut_resolves_a_pending_query_like_enter() {
    // "n" matches several families, and the current one is not the first row, so a fallback to
    // the first row would apply a family the menu never highlighted.
    let query = "n";
    let all = super::families();
    let (rows, _) = super::font_menu_rows(&all, None, query, &[]);
    assert!(rows.first().is_some_and(|(f, _)| f.as_str() != FONTS[1]), "the test needs a first row other than the current family");
    assert!(rows.iter().any(|(f, _)| f.as_str() == FONTS[1]));
    for accept in ["enter", "shortcut"] {
        let (mut h, id) = fixture(false, 1, false, "abcdef");
        select(&mut h, [2, 4]);
        h.run();
        choose(&mut h, query, "query");
        if accept == "enter" {
            h.key_press(Key::Enter);
            h.run();
        } else {
            commit(&mut h, "shortcut-focused");
            assert!(h.state().app.ui.text_edit.is_none(), "the shortcut still commits");
        }
        assert_eq!(families(&saved(&h, id)), vec![FONTS[1]; 6], "{accept} keeps the highlighted current family");
    }
}

#[test]
fn commit_does_not_accept_an_invalid_font_or_interrupt_ime() {
    for (character, query) in [(false, "no such font zzz"), (true, "no such font zzz"), (false, " "), (true, " ")] {
        let (mut h, id) = fixture(false, 0, character, "abcdef");
        select(&mut h, [2, 4]);
        h.run();
        choose(&mut h, query, "query");
        commit(&mut h, "shortcut-focused");
        assert!(h.state().app.ui.text_edit.is_none());
        assert_eq!(families(&saved(&h, id)), vec![FONTS[0]; 6]);
        super::edit_active(&mut h.state_mut().app).unwrap();
        super::ime_update(&mut h.state_mut().app, "é", false);
        h.key_press_modifiers(Modifiers::COMMAND, Key::Enter);
        h.run();
        assert!(h.state().app.ui.text_edit.is_some(), "composition owns its confirmation");
    }
}

#[test]
fn font_field_composition_keeps_confirmation_then_releases_global_commit() {
    for cancelled in [false, true] {
        let (mut h, id) = fixture(false, 0, false, "abcdef");
        select(&mut h, [2, 4]);
        h.run();
        h.query_all_by_role(Role::TextInput).next().unwrap().click();
        h.run();
        h.event(egui::Event::Ime(egui::ImeEvent::Preedit { text: FONTS[1].into(), active_range_chars: None }));
        h.run();
        commit(&mut h, "shortcut-focused");
        assert!(h.state().app.ui.text_edit.is_some());
        assert_eq!(families(&saved(&h, id)), vec![FONTS[0]; 6]);
        if cancelled {
            h.event(egui::Event::Ime(egui::ImeEvent::Preedit { text: String::new(), active_range_chars: None }));
            h.run();
            h.key_press_modifiers(Modifiers::COMMAND, Key::A);
            h.event(egui::Event::Text(FONTS[1].into()));
            h.run();
        } else {
            h.event(egui::Event::Ime(egui::ImeEvent::Commit(FONTS[1].into())));
            h.run();
        }
        commit(&mut h, "shortcut-focused");
        assert!(h.state().app.ui.text_edit.is_none());
        assert_eq!(families(&saved(&h, id)), [FONTS[0], FONTS[0], FONTS[1], FONTS[1], FONTS[0], FONTS[0]]);
    }
}

#[test]
fn caret_font_insertion_preserves_variable_axes_and_style_changes_can_clear_weight_axis() {
    let (mut h, id) = fixture(false, 0, false, "abcdef");
    h.state_mut().app.run("type.setStyle", json!({"layer":id.0,"variations":{"wght":650,"wdth":95}})).unwrap();
    select(&mut h, [3, 3]);
    h.run();
    choose(&mut h, FONTS[1], "enter");
    super::insert(&mut h.state_mut().app, "X");
    let variations = |app: &PhotocraftApp, at: usize| {
        let mut offset = 0;
        layer(&app.session.active().unwrap().doc, id)
            .char_runs()
            .into_iter()
            .find_map(|r| {
                offset += r.len;
                (offset > at).then_some(r.style.variations)
            })
            .unwrap()
    };
    let axes = variations(&h.state().app, 3);
    assert!(axes.iter().any(|v| v.axis == "wght" && v.value == 650.0));
    assert!(axes.iter().any(|v| v.axis == "wdth" && v.value == 95.0));
    let ctx = h.ctx.clone();
    super::apply(&mut h.state_mut().app, &ctx, json!({"fontStyle":"Bold"}));
    super::insert(&mut h.state_mut().app, "Y");
    let axes = variations(&h.state().app, 4);
    assert!(!axes.iter().any(|v| v.axis == "wght"));
    assert!(axes.iter().any(|v| v.axis == "wdth" && v.value == 95.0));
}

#[test]
fn caret_font_and_face_persist_across_more_font_choices_and_paste() {
    let (mut h, id) = fixture(false, 0, false, "abcdef");
    select(&mut h, [3, 3]);
    h.run();
    choose(&mut h, FONTS[1], "enter");
    let ctx = h.ctx.clone();
    super::apply(&mut h.state_mut().app, &ctx, json!({"fontStyle":"Bold"}));
    choose(&mut h, FONTS[0], "enter");
    h.ctx.memory_mut(|m| {
        if let Some(id) = m.focused() {
            m.surrender_focus(id);
        }
    });
    h.event(egui::Event::Paste("éX".into()));
    h.run();
    let t = saved(&h, id);
    for (i, c) in t["chars"].as_array().unwrap().iter().enumerate() {
        assert_eq!(c["font"], FONTS[0]);
        assert_eq!(c["weight"], if (3..5).contains(&i) { 700 } else { 400 });
    }
    let applied = saved(&h, id);
    commit(&mut h, "shortcut-focused");
    assert_eq!(saved(&h, id), applied);
    for name in ["probe.pcraft", "probe.psd"] {
        let doc = h.state().app.session.active().unwrap().doc.clone();
        let out = photocraft_io::export(&doc, name, &Default::default()).unwrap();
        let back = photocraft_io::import(name, &out.bytes).unwrap().document;
        let imported_id = back.walk().into_iter().find_map(|(_, _, l)| matches!(l.content, LayerContent::Text(_)).then_some(l.id)).unwrap();
        assert_eq!(families(&styles(&back, imported_id)), families(&applied));
    }
    assert!(h.state_mut().app.session.undo());
    assert_eq!(saved(&h, id)["text"], "abcdef");
    assert!(h.state_mut().app.session.redo());
    assert_eq!(saved(&h, id), applied);
}

#[test]
fn an_existing_empty_layer_keeps_its_font_when_committed() {
    for character in [false, true] {
        let (mut h, id) = fixture(false, 0, character, "abcdef");
        h.state_mut().app.run("type.edit", json!({"layer":id.0,"text":""})).unwrap();
        select(&mut h, [0, 0]);
        h.run();
        choose(&mut h, FONTS[1], "enter");
        commit(&mut h, "checkmark");
        assert_eq!(saved(&h, id)["summary"], FONTS[1]);
        super::edit_active(&mut h.state_mut().app).unwrap();
        super::insert(&mut h.state_mut().app, "X");
        assert_eq!(families(&saved(&h, id)), [FONTS[1]]);
    }
}

#[test]
fn font_commit_gpu_snapshots() {
    let Some(path) = std::env::var_os("FONT_COMMIT_VISUALS").map(std::path::PathBuf::from) else { return };
    let _gpu = photocraft_testkit::gpu_lock();
    std::fs::create_dir_all(&path).unwrap();
    for character in [false, true] {
        for scale in [1.0, 2.0] {
            let (mut h, _) = fixture_renderer(false, 0, character, "abcdef", true, scale);
            select(&mut h, [2, 4]);
            h.run();
            choose(&mut h, FONTS[1], "enter");
            h.render().unwrap().save(path.join(format!("selected-{character}-{scale}x.png"))).unwrap();
            commit(&mut h, "checkmark");
            h.render().unwrap().save(path.join(format!("committed-mixed-{character}-{scale}x.png"))).unwrap();
            super::edit_active(&mut h.state_mut().app).unwrap();
            select(&mut h, [2, 4]);
            h.run();
            h.render().unwrap().save(path.join(format!("reselected-{character}-{scale}x.png"))).unwrap();
            select(&mut h, [6, 6]);
            h.run();
            choose(&mut h, FONTS[1], "enter");
            h.render().unwrap().save(path.join(format!("caret-{character}-{scale}x.png"))).unwrap();
        }
    }
}
