use super::*;

/// A small valid Photoshop set, generated without external action files.
fn action_set_bytes(set: &str, action: &str) -> Vec<u8> {
    use craft_psd::atn::{AtnAction, AtnEvent, AtnFile, AtnItem};
    use craft_psd::descriptor::UnicodeString;
    craft_psd::atn::write(&AtnFile {
        name: UnicodeString::new_nul(set),
        actions: vec![AtnAction {
            name: UnicodeString::new_nul(action),
            items: vec![AtnItem { event: AtnEvent::Code(*b"Invr"), dictionary: "Invert".into(), ..Default::default() }],
            ..Default::default()
        }],
        ..Default::default()
    })
    .unwrap()
}

/// Dialog confirmation, worker completion and cancellation retain the original action boundary.
#[test]
fn single_history_dialog_confirmation_and_cancel() {
    for (single, confirm, background) in [(true, true, false), (true, true, true), (true, false, false), (false, true, false)] {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width":4,"height":4,"background":"#804020"})).unwrap();
        app.run("actions.setHistory", json!({"singleState":single})).unwrap();
        app.background_jobs = background;
        let before = app.session.active().unwrap().doc.clone();
        let count = app.session.active().unwrap().history.past_len();
        let mut action = Action::new(
            "Dialog",
            vec![
                ("layer.renameLayer".into(), json!({"name":"Prefix"})),
                ("filter.blur.gaussianBlur".into(), json!({"radius":1})),
                ("layer.renameLayer".into(), json!({"name":"Suffix"})),
            ],
        );
        action.dialogs.insert(1);
        app.session.actions.list.push(action);
        let paused = play(&mut app, &json!({"action":"Dialog"})).unwrap();
        let dialog = paused["paused"]["dialog"].as_u64().unwrap();
        assert_eq!(app.session.active().unwrap().history.past_len(), count + 1);
        if confirm {
            crate::dialogs::confirm(&mut app, dialog).unwrap();
        } else {
            crate::dialogs::cancel(&mut app, dialog).unwrap();
        }
        let st = app.session.active().unwrap();
        let after = st.doc.clone();
        assert_eq!(st.doc.layer(st.active_layer.unwrap()).unwrap().name, if confirm { "Suffix" } else { "Prefix" });
        if single {
            assert_eq!(st.history.past_len(), count + 1);
            assert_eq!(st.history.undo_label(), Some("Action: Dialog"));
        } else {
            assert!(st.history.past_len() >= count + 2);
        }
        assert!(app.session.undo());
        if single {
            assert_eq!(app.session.active().unwrap().doc.as_ref(), before.as_ref());
        } else {
            assert_ne!(app.session.active().unwrap().doc.as_ref(), before.as_ref());
        }
        assert!(app.session.redo());
        assert_eq!(app.session.active().unwrap().doc.as_ref(), after.as_ref());
        assert!(app.paused_playbacks.paused.is_empty());
        assert_eq!(app.session.actions.playing, 0);
        let count = app.session.active().unwrap().history.past_len();
        app.session.execute("layer.renameLayer", json!({"name":"Outside"})).unwrap();
        assert_eq!(app.session.active().unwrap().history.past_len(), count + 1);
        assert!(app.session.undo());
        assert_eq!(app.session.active().unwrap().doc.as_ref(), after.as_ref());
    }
}

/// A stale dialog must reject its own edit before it can change an unrelated document/history.
#[test]
fn single_history_dialog_refuses_intervening_edit_or_document_switch() {
    for (single_state, switch) in [(true, false), (true, true), (false, false), (false, true)] {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width":4,"height":4,"background":"#804020"})).unwrap();
        app.run("actions.setHistory", json!({"singleState":single_state})).unwrap();
        let mut action =
            Action::new("Bound", vec![("layer.renameLayer".into(), json!({"name":"Prefix"})), ("filter.blur.gaussianBlur".into(), json!({"radius":1}))]);
        action.dialogs.insert(1);
        app.session.actions.list.push(action);
        let paused = play(&mut app, &json!({"action":0})).unwrap();
        let dialog = paused["paused"]["dialog"].as_u64().unwrap();
        if switch {
            app.session.execute("file.new", json!({"width":2,"height":2})).unwrap();
        } else {
            app.session.execute("layer.renameLayer", json!({"name":"Unrelated"})).unwrap();
        }
        let before = app.session.active().unwrap().doc.clone();
        let history = app.session.active().unwrap().history.entries();
        assert!(crate::dialogs::confirm(&mut app, dialog).is_err());
        assert!(std::sync::Arc::ptr_eq(&app.session.active().unwrap().doc, &before));
        assert_eq!(app.session.active().unwrap().history.entries(), history);
        assert!(app.filter_preview.is_none());
        assert!(app.paused_playbacks.paused.is_empty());
        assert_eq!(app.session.actions.playing, 0);
    }
}

/// Prepare a real paused spelling step with a known document and history boundary, under either grouping policy.
fn paused_history_spelling(single: bool, gate: Option<Gate>) -> (PhotocraftApp, std::sync::Arc<photocraft_doc::Document>, usize) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width":300,"height":100})).unwrap();
    app.run("type.create", json!({"x":10,"y":50,"text":"Teh brwon"})).unwrap();
    app.run("actions.setHistory", json!({"singleState":single})).unwrap();
    let before = app.session.active().unwrap().doc.clone();
    let past = app.session.active().unwrap().history.past_len();
    let mut action = Action::new(
        "Spell",
        vec![
            ("layer.renameLayer".into(), json!({"name":"Prefix"})),
            ("edit.checkSpelling".into(), json!({"allLayers":false})),
            ("layer.renameLayer".into(), json!({"name":"Tail"})),
        ],
    );
    action.dialogs.insert(1);
    app.session.actions.list.push(action);
    app.session.authorize = gate;
    let paused = play(&mut app, &json!({"action":"Spell"})).unwrap();
    assert_eq!(paused["paused"]["dialog"], crate::type_panels_ui::SPELLING, "{paused}");
    (app, before, past)
}

#[test]
/// Repeated corrections share one owned history entry; a rejected correction cannot lose earlier admitted edits.
fn paused_spelling_repeated_changes_and_errors_keep_one_owned_history() {
    for single in [false, true] {
        let (mut app, before, past) = paused_history_spelling(single, None);
        let paused_doc = app.session.active().unwrap().doc.clone();
        let rows = app.session.active().unwrap().history.entries();
        assert!(spelling_change(&mut app, json!({"action":"change","start":0,"end":3,"word":"wrong","replace":"The"})).is_err());
        assert!(std::sync::Arc::ptr_eq(&app.session.active().unwrap().doc, &paused_doc));
        assert_eq!(app.session.active().unwrap().history.entries(), rows);
        for params in
            [json!({"action":"change","start":0,"end":3,"word":"Teh","replace":"The"}), json!({"action":"changeAll","word":"brwon","replace":"brown"})]
        {
            spelling_change(&mut app, params).unwrap();
            let token = app.ui.type_panels.spell.as_ref().unwrap().resume.unwrap();
            assert!(app.paused_playbacks.paused[&token].history.is_some());
            assert_eq!(app.session.actions.playing, 0);
        }
        let st = app.session.active().unwrap();
        let texts: Vec<_> = st
            .doc
            .walk()
            .into_iter()
            .filter_map(|(_, _, layer)| match &layer.content {
                photocraft_doc::LayerContent::Text(text) => Some(text.text.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(texts, ["The brown"]);
        let before_tail = st.doc.clone();
        crate::type_panels_ui::close_spelling(&mut app);
        assert!(!app.ui.status_error, "{}", app.ui.status);
        assert!(app.paused_playbacks.paused.is_empty());
        let st = app.session.active().unwrap();
        assert_eq!(st.history.past_len(), past + if single { 1 } else { 4 });
        assert_eq!(st.doc.layer(st.active_layer.unwrap()).unwrap().name, "Tail");
        if single {
            assert_eq!(st.history.undo_label(), Some("Action: Spell"));
        }
        let after = st.doc.clone();
        assert!(app.session.undo());
        assert_eq!(app.session.active().unwrap().doc.as_ref(), if single { before.as_ref() } else { before_tail.as_ref() });
        assert!(app.session.redo());
        assert_eq!(app.session.active().unwrap().doc.as_ref(), after.as_ref());
        app.run("layer.renameLayer", json!({"name":"Outside"})).unwrap();
        assert!(app.session.undo());
        assert_eq!(app.session.active().unwrap().doc.as_ref(), after.as_ref());
    }
}

fn deny_second_spelling_change(id: &str, params: &Value) -> photocraft_engine::Result<()> {
    if id == "edit.checkSpelling" && params["word"] == "brwon" { Err(photocraft_engine::EngineError::Other("original spelling gate".into())) } else { Ok(()) }
}

#[test]
/// Reject a spelling continuation after document/history drift or captured-policy denial before any correction.
fn paused_spelling_rejects_external_edit_switch_and_original_gate_before_change() {
    for single in [false, true] {
        for scenario in ["edit", "switch", "gate"] {
            let gate = (scenario == "gate").then_some(deny_second_spelling_change as Gate);
            let (mut app, _, _) = paused_history_spelling(single, gate);
            spelling_change(&mut app, json!({"action":"change","start":0,"end":3,"word":"Teh","replace":"The"})).unwrap();
            let corrected = app.session.active().unwrap().doc.clone();
            if scenario == "edit" {
                let past = app.session.active().unwrap().history.past_len();
                app.run("layer.renameLayer", json!({"name":"External"})).unwrap();
                assert_eq!(app.session.active().unwrap().history.past_len(), past + 1, "idle edits have their own history");
            } else if scenario == "switch" {
                app.run("file.new", json!({"width":4,"height":4})).unwrap();
            }
            // The original playback gate still applies even if the live gate has gone away.
            app.session.authorize = None;
            let before: Vec<_> = app.session.documents().iter().map(|st| (st.doc.clone(), st.history.entries())).collect();
            let error = spelling_change(&mut app, json!({"action":"changeAll","word":"brwon","replace":"brown"})).unwrap_err();
            if scenario == "gate" {
                assert!(error.contains("original spelling gate"), "{error}");
            }
            for (state, (doc, rows)) in app.session.documents().iter().zip(&before) {
                assert!(std::sync::Arc::ptr_eq(&state.doc, doc), "{scenario}");
                assert_eq!(&state.history.entries(), rows);
            }
            crate::type_panels_ui::close_spelling(&mut app);
            assert!(app.paused_playbacks.paused.is_empty());
            assert_eq!(app.session.actions.playing, 0);
            if scenario == "gate" {
                assert_eq!(app.session.active().unwrap().doc.layer(app.session.active().unwrap().active_layer.unwrap()).unwrap().name, "Tail");
            } else {
                assert!(!app.session.documents().iter().any(|st| st.doc.walk().iter().any(|(_, _, layer)| layer.name == "Tail")));
                assert!(app.ui.status_error, "{}", app.ui.status);
                if scenario == "edit" {
                    assert!(app.session.undo());
                    assert_eq!(app.session.active().unwrap().doc.as_ref(), corrected.as_ref());
                } else {
                    assert!(app.session.documents().iter().any(|st| std::sync::Arc::ptr_eq(&st.doc, &corrected)));
                }
            }
        }
    }
}

#[test]
/// Browser multi-selection must not be truncated to the first file.
fn bulk_load_imports_every_chosen_action_file() {
    use crate::file_dialog::{FileDialogAnswer, FileDialogRequest};
    let files = vec![("first.atn".into(), action_set_bytes("First set", "First")), ("second.atn".into(), action_set_bytes("Second set", "Second"))];
    let (show, asked) = crate::file_dialog::fake(vec![Some(FileDialogAnswer::ContentsMany(files))]);
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services { file_dialog: Some(show), ..Default::default() });
    load_dialog(&mut app).unwrap();
    app.poll_file_dialog(&egui::Context::default(), None);
    assert_eq!(app.session.actions.list.iter().map(|a| a.name.as_str()).collect::<Vec<_>>(), ["First", "Second"]);
    assert!(matches!(asked.borrow().as_slice(), [FileDialogRequest::Open { multiple: true, extensions: Some(ext), .. }] if ext == &["atn"]));
    assert!(!app.file_dialog_open());
    assert!(!app.ui.status_error, "{}", app.ui.status);
}

#[test]
/// A corrupt middle file cannot hide warnings or prevent a later set replacement.
fn bulk_load_keeps_selection_order_and_earlier_warnings_and_errors() {
    use crate::file_dialog::FileDialogAnswer;
    let dir = tempfile::tempdir().unwrap();
    let first = dir.path().join("z-first.atn");
    let broken = dir.path().join("broken.atn");
    let last = dir.path().join("a-last.atn");
    let mut unsupported = craft_psd::atn::parse(&action_set_bytes("Same set", "Old")).unwrap();
    unsupported.actions[0].items[0].event = craft_psd::atn::AtnEvent::Code(*b"Xxxx");
    std::fs::write(&first, craft_psd::atn::write(&unsupported).unwrap()).unwrap();
    std::fs::write(&broken, b"not an action file").unwrap();
    std::fs::write(&last, action_set_bytes("Same set", "Replacement")).unwrap();
    let paths = [first, broken, last].map(|p| p.to_string_lossy().into_owned()).to_vec();
    let mut app = disk_app(vec![Some(FileDialogAnswer::Paths(paths))]);
    load_dialog(&mut app).unwrap();
    app.poll_file_dialog(&egui::Context::default(), None);
    assert_eq!(app.session.actions.list.iter().map(|a| a.name.as_str()).collect::<Vec<_>>(), ["Replacement"]);
    assert_eq!(app.ui.actions.selected, Some(0));
    assert!(app.ui.status_error, "a later success must not hide an earlier failure");
    // The replaced set's unsupported step is gone, so it is not counted.
    for part in ["2 action files loaded", "1 action;", "Same set from a-last.atn replaced the one from z-first.atn", "1 file could not be loaded", "broken.atn"]
    {
        assert!(app.ui.status.contains(part), "{part}: {}", app.ui.status);
    }
    assert!(!app.ui.status.contains("PhotoCraft equivalent"), "{}", app.ui.status);
}

/// Load `files` (name, bytes) in one Load Actions… selection and return the status bar text.
fn bulk_load_status(app: &mut PhotocraftApp, files: Vec<(String, Vec<u8>)>) -> String {
    use crate::file_dialog::FileDialogAnswer;
    let (show, _) = crate::file_dialog::fake(vec![Some(FileDialogAnswer::ContentsMany(files))]);
    app.services.file_dialog = Some(show);
    load_dialog(app).unwrap();
    app.poll_file_dialog(&egui::Context::default(), None);
    app.ui.status.clone()
}

#[test]
/// Two files whose sets share a name: the later replaces the earlier, and the summary counts only
/// what the panel holds afterwards and says which set was replaced.
fn bulk_load_summary_counts_a_replaced_set_once() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let mut a = craft_psd::atn::parse(&action_set_bytes("Set 1", "A")).unwrap();
    a.actions[0].items[0].event = craft_psd::atn::AtnEvent::Code(*b"Xxxx");
    let files = vec![("a.atn".into(), craft_psd::atn::write(&a).unwrap()), ("b.atn".into(), action_set_bytes("Set 1", "B"))];
    let status = bulk_load_status(&mut app, files);
    assert_eq!(app.session.actions.list.iter().map(|a| a.name.as_str()).collect::<Vec<_>>(), ["B"]);
    assert!(!app.ui.status_error, "{status}");
    assert_eq!(status, "2 action files loaded (1 action; Set 1 from b.atn replaced the one from a.atn)");
}

#[test]
/// Distinct set names: every set counts, nothing is reported as replaced, and the file count reads
/// right for one loaded file as well as several.
fn bulk_load_summary_counts_distinct_sets_and_pluralises_files() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let files = vec![("a.atn".into(), action_set_bytes("Set A", "A")), ("b.atn".into(), action_set_bytes("Set B", "B"))];
    assert_eq!(bulk_load_status(&mut app, files), "2 action files loaded (2 actions)");
    let files = vec![("c.atn".into(), action_set_bytes("Set C", "C")), ("bad.atn".into(), b"not an action file".to_vec())];
    let status = bulk_load_status(&mut app, files);
    assert!(app.ui.status_error);
    assert!(status.starts_with("1 action file loaded (1 action; 1 file could not be loaded: bad.atn: "), "{status}");
}

#[test]
/// The multi-file summary is translated like the rest of the Actions panel.
fn bulk_load_summary_is_translated() {
    if !crate::i18n::needs_catalogs() {
        return;
    }
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let files = vec![("a.atn".into(), action_set_bytes("Set 1", "A")), ("b.atn".into(), action_set_bytes("Set 1", "B"))];
    let status = crate::i18n::with_language(crate::i18n::Lang::from_code("de").unwrap(), || bulk_load_status(&mut app, files));
    assert_eq!(status, "2 Aktionsdateien geladen (1 Aktion; Satz Set 1 aus b.atn hat den aus a.atn ersetzt)");
}

#[test]
/// Dismissing the picker is inert; one selected file keeps its existing status message.
fn bulk_load_keeps_cancel_empty_and_single_file_behavior() {
    use crate::file_dialog::FileDialogAnswer;
    for answer in [None, Some(FileDialogAnswer::Paths(Vec::new())), Some(FileDialogAnswer::ContentsMany(Vec::new()))] {
        let mut app = disk_app(vec![answer]);
        app.session.actions.list.push(Action::new("Existing", vec![]));
        app.ui.actions.selected = Some(0);
        app.ui.status = "Unchanged".into();
        load_dialog(&mut app).unwrap();
        app.poll_file_dialog(&egui::Context::default(), None);
        assert_eq!(app.session.actions.list[0].name, "Existing");
        assert_eq!(app.ui.actions.selected, Some(0));
        assert_eq!(app.ui.status, "Unchanged");
    }
    let mut app = disk_app(vec![Some(FileDialogAnswer::Contents("single.atn".into(), action_set_bytes("Single", "One")))]);
    load_dialog(&mut app).unwrap();
    app.poll_file_dialog(&egui::Context::default(), None);
    assert_eq!(app.session.actions.list[0].name, "One");
    assert!(app.ui.status.contains("single.atn"));
    assert!(!app.ui.status_error);
}

#[test]
fn panel_menu_lists_load_save_and_existing_action_commands() {
    use egui_kittest::{Harness, kittest::Queryable};
    let mut empty_app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    empty_app.run("file.new", json!({"width": 8, "height": 8})).unwrap();
    let mut empty = Harness::builder().with_size(vec2(240.0, 280.0)).build_ui_state(|ui, app| panel_menu(app, ui), empty_app);
    empty.run_steps(2);
    assert!(empty.query_all_by_label("Load Actions…").count() > 0);
    assert!(empty.query_all_by_label("New Action…").count() > 0);
    assert!(empty.query_all_by_label("Start Recording").count() > 0);
    empty.get_by_label("New Action…").click();
    empty.run_steps(2);
    assert_eq!(empty.state().session.actions.list.len(), 1);

    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 8, "height": 8})).unwrap();
    app.session.actions.list.push(Action::new("Make Layer", vec![("layer.new.layer".into(), json!({}))]));
    app.ui.actions.selected = Some(0);
    let mut h = Harness::builder().with_size(vec2(240.0, 320.0)).build_ui_state(|ui, app| panel_menu(app, ui), app);
    h.run_steps(2);
    for label in ["Load Actions…", "Save Actions…", "Insert Stop…", "Play", "Delete", "Start Recording"] {
        assert!(h.query_all_by_label(label).count() > 0, "{label}");
    }
    let layers = h.state().session.active().unwrap().doc.layers.len();
    h.get_by_label("Play").click();
    h.run_steps(3);
    assert_eq!(h.state().session.active().unwrap().doc.layers.len(), layers + 1);
}

#[test]
fn selected_layer_name_is_visible_during_recording_and_after_stop() {
    use egui_kittest::{Harness, kittest::Queryable};
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width":4,"height":4})).unwrap();
    app.run("layer.setProps", json!({"name":"Background"})).unwrap();
    let layer = app.session.active().unwrap().active_layer.unwrap();
    app.run("actions.record", json!({"name":"Select photo"})).unwrap();
    app.ui.actions.expanded = vec![true];
    app.run("layer.select", json!({"layer":layer.0})).unwrap();
    let mut h = Harness::builder().with_size(vec2(400.0, 260.0)).build_ui_state(|ui, app| panel(app, ui), app);
    h.run_steps(4);
    h.get_by_label("Select Layer (Background)");
    h.state_mut().run("actions.stop", json!({})).unwrap();
    h.run_steps(4);
    h.get_by_label("Select Layer (Background)");
    assert_eq!(step_label("layer.select", &json!({"layer":26})), "Select Layer (ID 26)");
    assert_eq!(step_label("layer.select", &json!({"layer":"Background"})), "Select Layer (Background)");
}

#[test]
fn actions_list_grows_with_panel_and_keeps_footer_visible() {
    use egui_kittest::{Harness, kittest::Queryable};
    let bottom = |height: f32| {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        for i in 0..50 {
            app.session.actions.list.push(Action { name: format!("Action {i}"), steps: vec![], disabled: Default::default(), ..Default::default() });
        }
        let mut h = Harness::builder().with_size(vec2(320.0, height)).build_ui_state(|ui, app| panel(app, ui), app);
        h.run_steps(3);
        let rect = h.get_by_label("Play selection").rect();
        assert!(rect.bottom() <= height, "footer must fit: {rect:?}");
        rect.top()
    };
    assert!(bottom(700.0) - bottom(300.0) > 350.0, "list uses the additional panel height instead of stopping at 320 px");
    assert!(crate::dock::Group::History.scrolls_itself(1));
}

/// #7646: an action's function key can take Shift and Command, as in Photoshop's Action
/// Options, and only that combination plays it.
#[test]
fn a_function_key_with_shift_and_command_plays_its_action() {
    use egui::{Event, Key, Modifiers};
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 32, "height": 32})).unwrap();
    app.session.actions.list.push(Action {
        name: "Action 1".into(),
        steps: vec![("layer.new.layer".into(), json!({}))],
        disabled: Default::default(),
        ..Default::default()
    });
    let key = photocraft_engine::atn_cmds::function_key_name(2, true, true);
    assert_eq!(key, "Cmd+Shift+F2");
    assign_shortcut(&mut app, "Action 1", &key).unwrap();
    let ctx = egui::Context::default();
    let press = |app: &mut PhotocraftApp, modifiers| {
        ctx.begin_pass(egui::RawInput {
            events: vec![
                Event::ModifiersChanged(modifiers),
                Event::Key { key: Key::F2, physical_key: None, pressed: true, repeat: false, modifiers },
                Event::Key { key: Key::F2, physical_key: None, pressed: false, repeat: false, modifiers },
            ],
            ..Default::default()
        });
        crate::shortcuts::handle(app, &ctx);
        ctx.end_pass().textures_delta.clear();
    };
    let layers = |app: &PhotocraftApp| app.session.active().unwrap().doc.layers.len();
    let before = layers(&app);
    press(&mut app, Modifiers::NONE);
    press(&mut app, Modifiers::SHIFT);
    assert_eq!(layers(&app), before, "F2 and Shift+F2 alone don't play it");
    press(&mut app, Modifiers::SHIFT | Modifiers::COMMAND);
    assert_eq!(layers(&app), before + 1, "Cmd+Shift+F2 plays it");
}

#[test]
fn shell_playback_resolves_recorded_eye_target_on_new_document() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 8, "height": 8})).unwrap();
    app.run("layer.new.layer", json!({"name": "Target"})).unwrap();
    app.run("actions.record", json!({"name": "Hide"})).unwrap();
    let id = app.session.active().unwrap().active_layer.unwrap();
    app.run("layer.setProps", json!({"layer": id.0, "visible": false})).unwrap();
    app.run("actions.stop", json!({})).unwrap();
    app.run("file.new", json!({"width": 8, "height": 8})).unwrap();
    app.run("layer.new.layer", json!({"name": "Target"})).unwrap();
    let id = app.session.active().unwrap().active_layer.unwrap();
    let result = play(&mut app, &json!({"action": "Hide"})).unwrap();
    assert!(result.get("failed").is_none(), "{result}");
    assert!(!app.session.active().unwrap().doc.layer(id).unwrap().visible);
}

#[test]
fn double_clicking_an_action_name_renames_it_in_place() {
    use egui_kittest::{Harness, kittest::Queryable};
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 32, "height": 32})).unwrap();
    app.session.actions.list.push(Action {
        name: "Action 1".into(),
        steps: vec![("layer.new.layer".into(), json!({}))],
        disabled: Default::default(),
        ..Default::default()
    });
    let layers = app.session.active().unwrap().doc.layers.len();
    // 60 fps steps, so two clicks a frame apart count as a double-click.
    let mut h = Harness::builder().with_size(vec2(320.0, 260.0)).with_step_dt(1.0 / 60.0).build_ui_state(|ui, app| panel(app, ui), app);
    h.run_steps(3);
    let rename = |h: &mut Harness<'_, PhotocraftApp>, text: &str| {
        let name = h.get_by_label("Action 1").rect().left_center() + vec2(52.0, 0.0);
        h.event(egui::Event::PointerMoved(name));
        // Wait out the double-click window of the previous attempt.
        h.run_steps(30);
        for pressed in [true, false, true, false] {
            h.event(egui::Event::PointerButton { pos: name, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE });
            h.step();
        }
        h.run_steps(3);
        h.event(egui::Event::Text(text.into()));
        h.run_steps(1);
        h.key_press(egui::Key::Enter);
        h.run_steps(3);
    };
    // Whitespace keeps the old name and reports why.
    rename(&mut h, "  ");
    assert_eq!(h.state().session.actions.list[0].name, "Action 1");
    assert!(h.state().ui.status_error);
    rename(&mut h, "Sepia");
    assert_eq!(h.state().session.actions.list[0].name, "Sepia");
    assert_eq!(h.state().session.active().unwrap().doc.layers.len(), layers, "renaming must not play the action");
}

#[test]
fn play_button_targets_first_action_if_nothing_is_selected() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 32, "height": 32})).unwrap();
    assert_eq!(playback_selection(&app), None);
    app.session.actions.list.push(Action {
        name: "Make Layer".into(),
        steps: vec![("layer.new.layer".into(), json!({}))],
        disabled: Default::default(),
        ..Default::default()
    });
    app.session.actions.list.push(Action {
        name: "Fit".into(),
        steps: vec![("view.fitOnScreen".into(), json!({}))],
        disabled: Default::default(),
        ..Default::default()
    });
    assert_eq!(playback_selection(&app), Some(0));
    app.ui.actions.selected = Some(1);
    assert_eq!(playback_selection(&app), Some(1));
    app.ui.actions.selected = Some(999);
    assert_eq!(playback_selection(&app), Some(0));

    app.ui.actions.selected = None;
    use egui_kittest::{Harness, kittest::Queryable};
    let mut h = Harness::builder().with_size(vec2(400.0, 320.0)).build_ui_state(|ui, app| panel(app, ui), app);
    h.run_steps(3);
    let before = h.state().session.active().unwrap().doc.layers.len();
    h.get_by_label("Play selection").click();
    h.run_steps(3);
    assert_eq!(h.state().session.active().unwrap().doc.layers.len(), before + 1);
    assert!(!h.state().ui.status_error);
}

#[test]
fn step_check_box_excludes_the_step_from_playback_and_batch() {
    use egui_kittest::{Harness, kittest::Queryable};
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 16, "height": 16})).unwrap();
    let steps = ["A", "B", "C"].map(|n| ("layer.new.layer".to_string(), json!({"name": n})));
    app.session.actions.list.push(Action::new("Three", steps.to_vec()));
    app.ui.actions.expanded = vec![true];
    let mut h = Harness::builder().with_size(vec2(320.0, 300.0)).build_ui_state(|ui, app| panel(app, ui), app);
    h.run_steps(3);
    let checks = h.query_all_by_role(egui::accesskit::Role::CheckBox).count();
    assert_eq!(checks, 3);
    h.query_all_by_role(egui::accesskit::Role::CheckBox).nth(1).unwrap().click();
    h.run_steps(3);
    let action = &h.state().session.actions.list[0];
    assert_eq!(action.disabled.iter().copied().collect::<Vec<_>>(), [1]);
    assert_eq!(action_steps(action).len(), 2, "Batch and droplets get only the checked steps");
    assert_eq!(step_count(3, action.disabled.len()), "2/3 steps");
    h.get_by_label("Play selection").click();
    h.run_steps(3);
    let names: Vec<_> = h.state().session.active().unwrap().doc.layers.iter().skip(1).map(|l| l.name.clone()).collect();
    assert_eq!(names, ["A", "C"]);
}

#[test]
fn playback_errors_are_visible_in_status_bar() {
    use egui_kittest::{Harness, kittest::Queryable};
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 32, "height": 32})).unwrap();
    app.session.actions.list.push(Action {
        name: "Bad action".into(),
        steps: vec![("not.a.command".into(), json!({}))],
        disabled: Default::default(),
        ..Default::default()
    });
    let mut h = Harness::builder().with_size(vec2(400.0, 320.0)).build_ui_state(|ui, app| panel(app, ui), app);
    h.run_steps(3);
    h.get_by_label("Play selection").click();
    h.run_steps(3);
    assert!(h.state().ui.status_error);
    assert!(h.state().ui.status.contains("not.a.command") || h.state().ui.status.contains("unknown command"));
}

#[test]
fn recording_reveals_new_action_and_latest_step_in_a_long_panel() {
    use egui_kittest::{Harness, kittest::Queryable};
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width":4,"height":4})).unwrap();
    for i in 0..30 {
        app.session.actions.list.push(Action { name: format!("Existing {i}"), steps: vec![], disabled: Default::default(), ..Default::default() });
    }
    let mut h = Harness::builder().with_size(vec2(320.0, 260.0)).build_ui_state(|ui, app| panel(app, ui), app);
    h.run_steps(4);
    begin_recording(h.state_mut(), false);
    h.run_steps(20);
    let rect = h.get_by_label("Action 31").rect();
    assert!(rect.top() >= 0.0 && rect.bottom() <= h.get_by_label("Play selection").rect().top(), "new action visible: {rect:?}");
    for _ in 0..20 {
        h.state_mut().run("view.zoomOut", json!({})).unwrap();
    }
    h.state_mut().run("view.fitOnScreen", json!({})).unwrap();
    h.run_steps(20);
    let rect = h.get_by_label("Fit on Screen").rect();
    assert!(rect.top() >= 0.0 && rect.bottom() <= h.get_by_label("Play selection").rect().top(), "latest step visible: {rect:?}");
}

#[test]
fn drag_steps_and_action_headers_reorders_and_keeps_selection() {
    use egui_kittest::{Harness, kittest::Queryable};
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.actions.list.push(Action {
        name: "First".into(),
        steps: vec![("view.zoomOut".into(), json!({})), ("view.fitOnScreen".into(), json!({})), ("view.actualPixels".into(), json!({}))],
        disabled: Default::default(),
        ..Default::default()
    });
    app.session.actions.list.push(Action { name: "Second".into(), steps: vec![], disabled: Default::default(), ..Default::default() });
    app.ui.actions.expanded = vec![true, false];
    let mut h = Harness::builder().with_size(vec2(320.0, 400.0)).build_ui_state(|ui, app| panel(app, ui), app);
    h.run_steps(4);
    fn drag(h: &mut Harness<'_, PhotocraftApp>, from: egui::Pos2, to: egui::Pos2) {
        h.event(egui::Event::PointerMoved(from));
        h.run_steps(1);
        h.event(egui::Event::PointerButton { pos: from, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE });
        h.run_steps(1);
        h.event(egui::Event::PointerMoved(from + vec2(10.0, 0.0)));
        h.run_steps(2);
        h.event(egui::Event::PointerMoved(to));
        h.run_steps(2);
        h.event(egui::Event::PointerButton { pos: to, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE });
        h.run_steps(3);
    }
    let from = h.get_by_label("Zoom Out").rect().center();
    let target = h.get_by_label("100%").rect();
    drag(&mut h, from, pos2(target.center().x, target.bottom() - 2.0));
    assert_eq!(
        h.state().session.actions.list[0].steps.iter().map(|st| st.0.as_str()).collect::<Vec<_>>(),
        ["view.fitOnScreen", "view.actualPixels", "view.zoomOut"]
    );
    assert_eq!(h.state().ui.actions.selected_step, Some(2));
    let from = h.get_by_label("Second").rect().center();
    let target = h.get_by_label("First").rect();
    drag(&mut h, from, pos2(target.center().x, target.top() + 2.0));
    assert_eq!(h.state().session.actions.list[0].name, "Second");
    assert_eq!(h.state().ui.actions.expanded, [false, true]);
    assert_eq!(h.state().ui.actions.selected, Some(0));
}

/// Shell playback retains nested failures and Batch file outcomes without running later steps.
#[test]
fn nested_shell_actions_include_view_steps_and_propagate_failures() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width":40,"height":40})).unwrap();
    app.session.actions.list.push(Action {
        name: "Child".into(),
        steps: vec![("layer.new.layer".into(), json!({})), ("view.fitOnScreen".into(), json!({}))],
        disabled: Default::default(),
        ..Default::default()
    });
    app.run("actions.record", json!({"name":"Parent"})).unwrap();
    app.run("actions.play", json!({"action":"Child"})).unwrap();
    app.run("actions.stop", json!({})).unwrap();
    assert_eq!(app.session.actions.list[1].steps, [("actions.play".into(), json!({"action":"Child"}))]);
    assert_eq!(step_label("actions.play", &json!({"action":"Child"})), "Play Action: Child");
    app.run("view.actualPixels", json!({})).unwrap();
    let r = app.run("actions.play", json!({"action":"Parent"})).unwrap();
    assert!(r.get("failed").is_none(), "{r}");
    assert!(app.ui.views[0].fit_pending);
    app.session.authorize =
        Some(|id, _| if id == "view.fitOnScreen" { Err(photocraft_engine::EngineError::Other("denied nested view".into())) } else { Ok(()) });
    let r = app.run("actions.play", json!({"action":"Parent"})).unwrap();
    assert!(r["failed"]["error"].as_str().unwrap().contains("denied nested view"));
    assert!(app.session.actions.playback_stack.is_empty());
    assert_eq!(app.session.actions.playing, 0);

    app.session.authorize = None;
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("valid.png");
    let corrupt = dir.path().join("corrupt.png");
    let bytes = photocraft_io::export(&app.session.active().unwrap().doc, "png", &Default::default()).unwrap().bytes;
    std::fs::write(&input, bytes).unwrap();
    std::fs::write(&corrupt, b"not a PNG").unwrap();
    let before = app.session.documents().len();
    for mixed in [true, false] {
        let output = dir.path().join(if mixed { "mixed" } else { "success" });
        let inputs = if mixed { vec![&input, &corrupt] } else { vec![&input] };
        let batch = json!({"input": inputs, "output": output, "steps": [], "format": "png"});
        let index = app.session.actions.list.len();
        let name = if mixed { "Mixed Batch" } else { "Successful Batch" };
        app.session.actions.list.push(Action::new(name, vec![("file.automate.batch".into(), batch), ("file.new".into(), json!({"width": 2, "height": 2}))]));
        let result = app.run("actions.play", json!({"action": index})).unwrap();
        if mixed {
            assert_eq!(result["ran"], 0, "{result}");
            let batch = &result["failed"]["result"];
            let files = batch["files"].as_array().unwrap();
            assert_eq!(files.len(), 1);
            assert_eq!(std::path::Path::new(files[0].as_str().unwrap()), output.join("valid.png").as_path());
            assert_eq!(batch["errors"].as_array().unwrap().len(), 1);
            assert_eq!(batch["errors"][0]["file"], json!(corrupt));
            report_play(&mut app, &result);
            assert!(app.ui.status_error);
            assert!(app.ui.status.contains("1 file(s) failed") && app.ui.status.contains("1 saved") && app.ui.status.contains("corrupt.png"));
            assert_eq!(app.session.documents().len(), before, "a failed Batch stops before file.new");
        } else {
            assert!(result.get("failed").is_none(), "{result}");
            assert_eq!(result["ran"], 2);
            assert_eq!(app.session.documents().len(), before + 1, "a successful Batch continues");
        }
        assert!(output.join("valid.png").exists(), "successful files remain available");
    }
}

#[test]
fn panel_shows_pending_steps_and_trash_deletes_only_the_clicked_step() {
    use egui_kittest::{Harness, kittest::Queryable};
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 40, "height": 40})).unwrap();
    begin_recording(&mut app, false);
    app.run("view.fitOnScreen", json!({})).unwrap();
    app.run("view.zoomOut", json!({})).unwrap();
    let mut h = Harness::builder().with_size(vec2(320.0, 400.0)).build_ui_state(|ui, app| panel(app, ui), app);
    h.run_steps(2);
    assert!(h.state().session.actions.recording.is_some());
    h.get_by_label("Fit on Screen");
    h.get_by_label("Zoom Out").click();
    h.run_steps(2);
    assert_eq!(h.state().ui.actions.selected_step, Some(1));
    h.get_by_label("Delete").click();
    h.run_steps(2);
    assert_eq!(visible_steps(h.state(), 0).iter().map(|(id, _)| id.as_str()).collect::<Vec<_>>(), ["view.fitOnScreen"]);
    assert_eq!(h.state().session.actions.list.len(), 1);
    assert!(h.state().session.actions.recording.is_some());
    assert!(h.state().ui.actions.selected.is_none());
    delete_selection(h.state_mut());
    assert_eq!(h.state().session.actions.list.len(), 1, "repeated trash cannot delete the action");
    h.state_mut().run("view.zoomIn", json!({})).unwrap();
    h.run_steps(2);
    h.get_by_label("Zoom In");
    h.state_mut().run("actions.stop", json!({})).unwrap();
    h.run_steps(2);
    assert_eq!(visible_steps(h.state(), 0).iter().map(|(id, _)| id.as_str()).collect::<Vec<_>>(), ["view.fitOnScreen", "view.zoomIn"]);
    h.get_by_label("Zoom In").click();
    h.run_steps(2);
    h.get_by_label("Delete").click();
    h.run_steps(2);
    assert_eq!(visible_steps(h.state(), 0).iter().map(|(id, _)| id.as_str()).collect::<Vec<_>>(), ["view.fitOnScreen"]);
}

#[test]
fn view_menu_commands_record_append_and_replay_after_resize() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let ctx = egui::Context::default();
    app.run("file.new", json!({"width": 160, "height": 100})).unwrap();
    begin_recording(&mut app, false);
    app.run("image.canvasSize", json!({"width": 80, "height": 60})).unwrap();
    app.run("actions.stop", json!({})).unwrap();
    begin_recording(&mut app, true);
    crate::menus::invoke(&mut app, &ctx, "view.fitOnScreen", json!({})).unwrap();
    app.run("actions.stop", json!({})).unwrap();
    assert_eq!(app.session.actions.list.len(), 1);
    assert_eq!(app.session.actions.list[0].steps.iter().map(|s| s.0.as_str()).collect::<Vec<_>>(), ["image.canvasSize", "view.fitOnScreen"]);
    assert_eq!(label_of("view.fitOnScreen"), "Fit on Screen");
    // Persisted actions retain the menu step, and playback fits the new document.
    let saved = serde_json::to_string(&app.session.actions.list).unwrap();
    app.session.actions.list = serde_json::from_str(&saved).unwrap();
    app.run("file.new", json!({"width": 200, "height": 140})).unwrap();
    app.run("view.actualPixels", json!({})).unwrap();
    let result = app.run("actions.play", json!({"action": 0})).unwrap();
    assert_eq!(result["ran"], 2);
    assert!(result.get("failed").is_none(), "{result}");
    let idx = app.session.active_index().unwrap();
    assert!(app.ui.views[idx].fit_pending);
    let doc = &app.session.active().unwrap().doc;
    crate::canvas::fit_view(&mut app.ui.views[idx], doc, vec2(100.0, 100.0), 1.0, 1.0);
    assert_eq!(app.ui.views[idx].zoom, 0.75);
    assert_eq!(app.ui.views[idx].center, [40.0, 30.0]);
    assert!(!app.ui.views[idx].fit_pending);
}

#[test]
fn view_action_failure_resume_and_authorization() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.actions.list.push(Action {
        name: "Fit".into(),
        steps: vec![("view.fitOnScreen".into(), json!({})), ("view.actualPixels".into(), json!({}))],
        disabled: Default::default(),
        ..Default::default()
    });
    let result = app.run("actions.play", json!({"action": "Fit"})).unwrap();
    assert_eq!(result["ran"], 0);
    assert_eq!(result["failed"]["step"], 0);
    assert_eq!(app.session.actions.playing, 0);
    app.run("file.new", json!({"width": 40, "height": 40})).unwrap();
    app.session.authorize =
        Some(
            |id, _| {
                if id == "view.fitOnScreen" { Err(photocraft_engine::EngineError::BadParams { cmd: id.into(), msg: "denied view".into() }) } else { Ok(()) }
            },
        );
    let result = app.run("actions.play", json!({"action": 0})).unwrap();
    assert_eq!(result["ran"], 0);
    assert!(result["failed"]["error"].as_str().unwrap().contains("denied view"));
    let result = app.run("actions.play", json!({"action": 0, "from": 1})).unwrap();
    assert_eq!(result["ran"], 1);
    assert!(!app.ui.views[0].fit_pending);
    assert!(app.run("actions.play", json!({"action": 0, "from": -1})).is_err());
    app.session.actions.playing = 1;
    assert!(app.run("actions.play", json!({"action": 0})).is_err());
}

#[test]
fn record_and_replay_on_another_document() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 40, "height": 40})).unwrap();
    app.sync_views();
    app.run("actions.record", json!({})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("select.rect", json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap();
    app.run("edit.fill", json!({"color": "#ff0000"})).unwrap();
    let _ = app.run("document.pixel", json!({"x": 1, "y": 1}));
    app.run("actions.stop", json!({})).unwrap();
    let a = &app.session.actions.list[0];
    assert_eq!(a.steps.iter().map(|s| s.0.as_str()).collect::<Vec<_>>(), ["layer.new.layer", "select.rect", "edit.fill"]);
    app.run("file.new", json!({"width": 40, "height": 40})).unwrap();
    app.sync_views();
    let played = app.run("actions.play", json!({"action": 0})).unwrap();
    assert_eq!(played["ran"], 3);
    assert!(played.get("failed").is_none(), "{played}");
    let d = &app.session.active().unwrap().doc;
    assert_eq!(d.layers.len(), 2);
    assert_eq!(d.layers[1].surface().unwrap().pixel(5, 5), vec![1.0, 0.0, 0.0, 1.0]);
}

#[test]
fn recorded_save_as_and_save_replay_and_write_the_file() {
    let written = std::rc::Rc::new(std::cell::RefCell::new(Vec::<String>::new()));
    let log = written.clone();
    let services = crate::Services {
        export: Some(Box::new(|doc, path, _| photocraft_io::export(doc, path, &Default::default()).map(|r| (r.bytes, r.warnings)).map_err(|e| e.to_string()))),
        write: Some(Box::new(move |path, _| {
            log.borrow_mut().push(path.to_string());
            Ok(())
        })),
        ..Default::default()
    };
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
    let ctx = egui::Context::default();
    app.run("file.new", json!({"width": 16, "height": 16})).unwrap();
    app.run("actions.record", json!({})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    crate::menus::invoke(&mut app, &ctx, "file.saveAs", json!({"path": "out/result.psd"})).unwrap();
    crate::menus::invoke(&mut app, &ctx, "file.save", json!({})).unwrap();
    app.run("actions.stop", json!({})).unwrap();
    let steps = &app.session.actions.list[0].steps;
    assert_eq!(steps, &[("layer.new.layer".into(), json!({})), ("file.saveAs".into(), json!({"path": "out/result.psd"})), ("file.save".into(), json!({}))]);
    written.borrow_mut().clear();
    app.run("file.new", json!({"width": 16, "height": 16})).unwrap();
    let played = app.run("actions.play", json!({"action": 0})).unwrap();
    assert!(played.get("failed").is_none(), "{played}");
    assert_eq!(*written.borrow(), ["out/result.psd", "out/result.psd"]);
    assert_eq!(app.session.active().unwrap().path.as_deref(), Some("out/result.psd"));
}

/// An app with the real importer/exporter and writer whose Save dialogs answer `answers`.
fn disk_app(answers: Vec<Option<crate::file_dialog::FileDialogAnswer>>) -> PhotocraftApp {
    let services = crate::Services {
        import: Some(Box::new(|name, bytes, opts| {
            photocraft_io::import_with_options(name, bytes, opts).map(|r| (r.document, r.warnings)).map_err(|e| e.to_string())
        })),
        export: Some(Box::new(|doc, path, _| photocraft_io::export(doc, path, &Default::default()).map(|r| (r.bytes, r.warnings)).map_err(|e| e.to_string()))),
        write: Some(Box::new(|path, bytes| std::fs::write(path, bytes).map_err(|e| e.to_string()))),
        file_dialog: Some(crate::file_dialog::fake(answers).0),
        ..Default::default()
    };
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
    app.background_jobs = false;
    app
}

/// Records Invert + File › Save on the file `a`, then plays it on the file `b` (given a
/// new layer first when `layered`). Returns the played result and whether `b` / `a`
/// changed on disk.
fn record_save_on_a_play_on_b(ext: &str, layered: bool) -> (Value, bool, bool) {
    use crate::file_dialog::FileDialogAnswer::SaveTo;
    let dir = std::env::temp_dir().join(format!("photocraft-rec-save-{ext}-{layered}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = |n: &str| dir.join(format!("{n}.{ext}")).to_string_lossy().into_owned();
    let (a, b) = (path("a"), path("b"));
    // The dialog confirms the suggested name, which is the document's own file.
    let mut app = disk_app(vec![Some(SaveTo(a.clone())), Some(SaveTo(a.clone()))]);
    let ctx = egui::Context::default();
    for p in [&a, &b] {
        app.run("file.new", json!({"width": 16, "height": 16})).unwrap();
        app.save_as(Some(p.clone())).unwrap();
    }
    let (before_a, before_b) = (std::fs::read(&a).unwrap(), std::fs::read(&b).unwrap());
    app.open_path(&a).unwrap();
    app.run("actions.record", json!({})).unwrap();
    app.run("image.adjustments.invert", json!({})).unwrap();
    crate::menus::invoke(&mut app, &ctx, "file.save", json!({})).unwrap();
    app.poll_file_dialog(&ctx, None);
    app.run("actions.stop", json!({})).unwrap();
    std::fs::write(&a, &before_a).unwrap();
    app.open_path(&b).unwrap();
    if layered {
        app.run("layer.new.layer", json!({})).unwrap();
    }
    let played = app.run("actions.play", json!({"action": 0})).unwrap();
    let r = (played, std::fs::read(&b).unwrap() != before_b, std::fs::read(&a).unwrap() != before_a);
    let _ = std::fs::remove_dir_all(&dir);
    r
}

#[test]
fn recorded_save_overwrites_the_layered_file_the_action_is_played_on() {
    let (played, b_changed, a_changed) = record_save_on_a_play_on_b("psd", false);
    assert!(played.get("failed").is_none(), "{played}");
    assert!(b_changed && !a_changed);
}

#[test]
fn recorded_save_overwrites_the_flat_file_the_action_is_played_on() {
    let (played, b_changed, a_changed) = record_save_on_a_play_on_b("png", false);
    assert!(played.get("failed").is_none(), "{played}");
    assert!(b_changed, "Save did not overwrite the file the action was played on");
    assert!(!a_changed, "Save wrote the file the action was recorded on");
}

#[test]
fn recorded_save_never_flattens_layers_over_a_flat_file() {
    // The played-on document has layers its PNG can't keep: the step fails, nothing is written (#3806).
    let (played, b_changed, a_changed) = record_save_on_a_play_on_b("png", true);
    assert!(played.get("failed").is_some(), "{played}");
    assert!(!b_changed && !a_changed);
}

/// Loads a Photoshop set holding Invert + a Save As into `dest` (in `out`; with `in_folder`,
/// Photoshop's "save in directory", `out` itself) and plays it on each PSD in `sources`.
/// Returns the played results, whether any PSD changed on disk and the files in `out`.
fn play_imported_save_as(dest: &str, in_folder: bool, sources: &[&str]) -> (Vec<Value>, bool, Vec<String>) {
    use craft_psd::atn::{AtnAction, AtnEvent, AtnFile, AtnItem};
    use craft_psd::descriptor::{Descriptor, UnicodeString, Value as D};
    let dir = std::env::temp_dir().join(format!("photocraft-atn-save-as-{dest}-{in_folder}-{}", std::process::id()));
    let out = dir.join("out");
    std::fs::create_dir_all(&out).unwrap();
    let path = |text: &str| {
        let units: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
        let mut b = b"txtu".to_vec();
        b.extend_from_slice(&((12 + units.len() * 2) as u32).to_le_bytes());
        b.extend_from_slice(&(units.len() as u32).to_le_bytes());
        units.iter().for_each(|u| b.extend_from_slice(&u.to_le_bytes()));
        D::Path(b)
    };
    let mut save = Descriptor::new("null").with("As  ", D::Descriptor(Descriptor::new("PNGF")));
    save = if in_folder {
        save.with("In  ", path(&out.to_string_lossy())).with("DIDr", D::Boolean(true))
    } else {
        save.with("In  ", path(&out.join(dest).to_string_lossy()))
    };
    let save = save.with("DocI", D::Integer(53));
    let item = |code: &[u8; 4], label: &str, d: Option<Descriptor>| AtnItem {
        event: AtnEvent::Code(*code),
        dictionary: label.into(),
        descriptor: d,
        ..Default::default()
    };
    let set = AtnFile {
        name: UnicodeString::new_nul("Export"),
        actions: vec![AtnAction {
            name: UnicodeString::new_nul("Invert and save"),
            items: vec![item(b"Invr", "Invert", None), item(b"save", "Save", Some(save))],
            ..Default::default()
        }],
        ..Default::default()
    };
    let mut app = disk_app(vec![]);
    let sources: Vec<String> = sources.iter().map(|n| dir.join(format!("{n}.psd")).to_string_lossy().into_owned()).collect();
    let mut before = Vec::new();
    for src in &sources {
        app.run("file.new", json!({"width": 16, "height": 16})).unwrap();
        app.save_as(Some(src.clone())).unwrap();
        before.push(std::fs::read(src).unwrap());
    }
    app.run("actions.load", json!({"data": photocraft_engine::paint::tile::b64_encode(&craft_psd::atn::write(&set).unwrap())})).unwrap();
    let mut played = Vec::new();
    for src in &sources {
        app.open_path(src).unwrap();
        played.push(app.run("actions.play", json!({"action": 0})).unwrap());
    }
    let mut written: Vec<String> = std::fs::read_dir(&out).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
    written.sort();
    let changed = sources.iter().zip(&before).any(|(src, before)| std::fs::read(src).unwrap() != *before);
    let _ = std::fs::remove_dir_all(&dir);
    (played, changed, written)
}

#[test]
fn imported_save_as_writes_the_recorded_file_not_the_played_one() {
    // #7102: the Save As imported as File › Save, which wrote over src.psd.
    let (played, src_changed, written) = play_imported_save_as("result.png", false, &["src"]);
    assert!(played[0].get("failed").is_none(), "{}", played[0]);
    assert!(!src_changed, "the Save As wrote over the file the action was played on");
    assert_eq!(written, ["result.png"]);
}

#[test]
fn imported_save_as_to_a_file_without_an_extension_stops_before_writing() {
    // A destination that isn't a file of the recorded format stays a kept step (#7102):
    // playback stops on it, nothing is saved.
    let (played, src_changed, written) = play_imported_save_as("folder", false, &["src"]);
    assert!(played[0].get("failed").is_some(), "{}", played[0]);
    assert!(!src_changed);
    assert!(written.is_empty(), "{written:?}");
}

#[test]
fn imported_save_as_into_a_folder_saves_each_document_under_its_own_name() {
    // #5305: Photoshop's "save in directory" Save As, played on two documents, writes one file
    // for each, named after it, in the recorded folder, and leaves the played files alone.
    let (played, src_changed, written) = play_imported_save_as("", true, &["first", "second"]);
    for p in &played {
        assert!(p.get("failed").is_none(), "{p}");
    }
    assert!(!src_changed);
    assert_eq!(written, ["first.png", "second.png"]);
}

/// #5305: a Save As recorded with the document's own name (the one Save As suggests) keeps
/// only the folder, so playing the action saves each document under its own name instead of
/// over the recorded file; a name typed in is recorded as it is.
#[test]
fn recorded_save_as_with_the_suggested_name_saves_each_document_under_its_own() {
    let written = std::rc::Rc::new(std::cell::RefCell::new(Vec::<String>::new()));
    let log = written.clone();
    let services = crate::Services {
        export: Some(Box::new(|doc, path, _| photocraft_io::export(doc, path, &Default::default()).map(|r| (r.bytes, r.warnings)).map_err(|e| e.to_string()))),
        write: Some(Box::new(move |path, _| {
            log.borrow_mut().push(path.to_string());
            Ok(())
        })),
        ..Default::default()
    };
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
    app.background_jobs = false;
    let ctx = egui::Context::default();
    app.run("file.new", json!({"width": 16, "height": 16, "name": "first"})).unwrap();
    app.run("actions.record", json!({})).unwrap();
    crate::menus::invoke(&mut app, &ctx, "file.saveAs", json!({"path": "out/first.psd"})).unwrap();
    crate::menus::invoke(&mut app, &ctx, "file.saveAs", json!({"path": "out/renamed.psd"})).unwrap();
    app.run("actions.stop", json!({})).unwrap();
    let steps = &app.session.actions.list[0].steps;
    assert_eq!(steps, &[("file.saveAs".into(), json!({"folder": "out", "extension": "psd"})), ("file.saveAs".into(), json!({"path": "out/renamed.psd"}))]);
    // Played on two other documents: each under its own name in the recorded folder.
    app.session.actions.list[0].steps.truncate(1);
    written.borrow_mut().clear();
    let mut expected = Vec::new();
    for name in ["second", "third"] {
        app.run("file.new", json!({"width": 16, "height": 16, "name": name})).unwrap();
        let played = app.run("actions.play", json!({"action": 0})).unwrap();
        assert!(played.get("failed").is_none(), "{played}");
        let own = std::path::Path::new("out").join(format!("{name}.psd")).to_string_lossy().into_owned();
        assert_eq!(app.session.active().unwrap().path.as_deref(), Some(own.as_str()));
        expected.push(own);
    }
    assert_eq!(*written.borrow(), expected);
    // A folder step needs its extension.
    assert!(app.replay_save("file.saveAs", &json!({"folder": "out"})).is_err());
}

#[test]
fn set_folders_group_actions_and_collapse() {
    use egui_kittest::{Harness, kittest::Queryable};
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let mut sharpen = Action::new("Sharpen", vec![("filter.sharpen.sharpen".into(), json!({}))]);
    sharpen.set = Some("Retouch".into());
    let mut soften = Action::new("Soften", vec![("filter.blur.blur".into(), json!({}))]);
    soften.set = Some("Retouch".into());
    app.session.actions.list.push(Action::new("Loose", vec![]));
    app.session.actions.list.push(sharpen);
    app.session.actions.list.push(Action::new("Also loose", vec![]));
    app.session.actions.list.push(soften);
    let mut h = Harness::builder().with_size(vec2(360.0, 400.0)).build_ui_state(|ui, app| panel(app, ui), app);
    h.run_steps(3);
    // The folder row shows the set with its count; its actions sit under it, loose ones first.
    let folder = h.get_by_label("Retouch").rect();
    assert!(h.get_by_label("Loose").rect().top() < folder.top());
    assert!(h.get_by_label("Also loose").rect().top() < folder.top());
    assert!(h.get_by_label("Sharpen").rect().top() > folder.top());
    assert!(h.get_by_label("Soften").rect().top() > h.get_by_label("Sharpen").rect().top());
    assert_eq!(display_order(&[]), Vec::<usize>::new());
    // Clicking the folder's triangle collapses it; its actions disappear and the state is remembered.
    click_set_triangle(&mut h, "Retouch");
    assert_eq!(h.state().ui.actions.collapsed_sets, ["Retouch"]);
    assert!(h.query_by_label("Sharpen").is_none());
    assert!(h.query_by_label("Loose").is_some());
    click_set_triangle(&mut h, "Retouch");
    assert!(h.state().ui.actions.collapsed_sets.is_empty());
    assert!(h.query_by_label("Sharpen").is_some());
}

/// Two sets, Web listed first: "Resize" (Web) adds a layer "w", "Sharpen" and "Soften" (Retouch)
/// add "r1" and "r2".
fn two_set_panel() -> egui_kittest::Harness<'static, PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 8, "height": 8})).unwrap();
    for (name, set, layer) in [("Resize", "Web", "w"), ("Sharpen", "Retouch", "r1"), ("Soften", "Retouch", "r2")] {
        let mut a = Action::new(name, vec![("layer.new.layer".into(), json!({"name": layer}))]);
        a.set = Some(set.into());
        app.session.actions.list.push(a);
    }
    let mut h = egui_kittest::Harness::builder().with_size(vec2(360.0, 400.0)).build_ui_state(|ui, app| panel(app, ui), app);
    h.run_steps(2);
    h
}

fn layer_names(app: &PhotocraftApp) -> Vec<String> {
    app.session.active().unwrap().doc.walk().iter().map(|(_, _, l)| l.name.clone()).filter(|n| n != "Background").collect()
}

/// Click a set folder row's disclosure triangle (a plain `click()` lands on the row's middle,
/// which selects the set).
fn click_set_triangle(h: &mut egui_kittest::Harness<'_, PhotocraftApp>, set: &str) {
    use egui_kittest::kittest::Queryable;
    let pos = h.get_by_label(set).rect().left_center() + vec2(12.0, 0.0);
    h.event(egui::Event::PointerMoved(pos));
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE });
    }
    h.run_steps(2);
}

/// #10459 fix-up: a click on a set folder row (not its triangle) selects the set, and Play
/// then plays every action of that set in order, as Photoshop plays a selected set, not the first
/// action of the whole list.
#[test]
fn play_with_a_set_folder_selected_plays_that_sets_actions() {
    use egui_kittest::kittest::Queryable;
    let mut h = two_set_panel();
    h.get_by_label("Resize").click();
    h.run_steps(2);
    h.get_by_label("Retouch").click();
    h.run_steps(2);
    assert!(h.state().ui.actions.collapsed_sets.is_empty(), "a click on the row selects, it doesn't collapse");
    h.get_by_label("Play selection").click();
    h.run_steps(2);
    let mut names = layer_names(h.state());
    names.sort();
    assert_eq!(names, ["r1", "r2"], "the Retouch set played, not Resize");
    assert!(!h.state().ui.status_error, "{}", h.state().ui.status);
    // Batch and Create Droplet start on the selected set too.
    assert_eq!(selected_action(h.state()).map(|a| a.name.as_str()), Some("Sharpen"));
}

/// #10459 fix-up: Play with an empty set selected plays nothing.
#[test]
fn play_with_an_empty_set_selected_plays_nothing() {
    use egui_kittest::kittest::Queryable;
    let mut h = two_set_panel();
    h.state_mut().run("actions.newSet", json!({"name": "Print"})).unwrap();
    h.run_steps(2);
    h.get_by_label("Print").click();
    h.run_steps(2);
    assert_eq!(h.state().ui.actions.selected_set.as_deref(), Some("Print"));
    h.get_by_label("Play selection").click();
    h.run_steps(2);
    assert!(layer_names(h.state()).is_empty(), "{:?}", layer_names(h.state()));
}

/// CodeRabbit on batch 11: with an empty set selected, Batch and Create Droplet start on the first
/// action, as `selected_action` documents.
#[test]
fn an_empty_selected_set_falls_back_to_the_first_action() {
    use egui_kittest::kittest::Queryable;
    let mut h = two_set_panel();
    h.state_mut().run("actions.newSet", json!({"name": "Print"})).unwrap();
    h.run_steps(2);
    h.get_by_label("Print").click();
    h.run_steps(2);
    assert_eq!(selected_action(h.state()).map(|a| a.name.as_str()), Some("Resize"));
}

/// #10459 fix-up: the common case is unchanged: a selected action plays alone, also after a set
/// folder was selected before it, and a later Delete doesn't bring that set back.
#[test]
fn play_with_an_action_selected_plays_only_that_action() {
    use egui_kittest::kittest::Queryable;
    let mut h = two_set_panel();
    h.get_by_label("Retouch").click();
    h.run_steps(2);
    h.get_by_label("Retouch").click();
    h.run_steps(2);
    h.get_by_label("Sharpen").click();
    h.run_steps(2);
    h.get_by_label("Play selection").click();
    h.run_steps(2);
    assert_eq!(layer_names(h.state()), ["r1"]);
    assert_eq!(selected_action(h.state()).map(|a| a.name.as_str()), Some("Sharpen"));
    h.get_by_label("Delete").click();
    h.run_steps(2);
    assert_eq!((h.state().ui.actions.selected, h.state().ui.actions.selected_set.as_deref()), (None, None));
}

/// #10459 fix-up: the disclosure triangle collapses and expands a set and leaves the selection
/// alone, so Play still plays the selected action, not the set that was looked into.
#[test]
fn collapsing_a_set_keeps_the_selected_action_for_play() {
    use egui_kittest::kittest::Queryable;
    let mut h = two_set_panel();
    h.get_by_label("Resize").click();
    h.run_steps(2);
    click_set_triangle(&mut h, "Retouch");
    assert_eq!(h.state().ui.actions.collapsed_sets, ["Retouch"]);
    assert!(h.query_by_label("Sharpen").is_none());
    click_set_triangle(&mut h, "Retouch");
    assert!(h.state().ui.actions.collapsed_sets.is_empty());
    assert_eq!((h.state().ui.actions.selected, h.state().ui.actions.selected_set.as_deref()), (Some(0), None));
    h.get_by_label("Play selection").click();
    h.run_steps(2);
    assert_eq!(layer_names(h.state()), ["w"]);
}

/// #10459 fix-up: selecting a step (or any other way an action becomes the selection) drops a
/// selected set, so deleting that step doesn't bring the set back as Play's target.
#[test]
fn a_selected_step_replaces_the_selected_set() {
    use egui_kittest::kittest::Queryable;
    let mut h = two_set_panel();
    h.state_mut().session.actions.list[0].steps.push(("select.all".into(), json!({})));
    h.state_mut().ui.actions.expanded = vec![true, false, false];
    h.run_steps(2);
    h.get_by_label("Retouch").click();
    h.run_steps(2);
    assert_eq!(h.state().ui.actions.selected_set.as_deref(), Some("Retouch"));
    h.get_by_label(&label_of("select.all")).click();
    h.run_steps(2);
    assert_eq!((h.state().ui.actions.selected, h.state().ui.actions.selected_step), (Some(0), Some(1)));
    h.get_by_label("Delete").click();
    h.run_steps(2);
    assert_eq!(h.state().session.actions.list[0].steps.len(), 1, "the step went");
    assert_eq!((h.state().ui.actions.selected, h.state().ui.actions.selected_set.as_deref()), (None, None));
    h.get_by_label("Play selection").click();
    h.run_steps(2);
    assert_eq!(layer_names(h.state()), ["w"], "the first action, not the Retouch set");
}

/// #10459 fix-up: a set whose first action pauses at step dialogs plays the rest of the set
/// after OK, and Cancel stops the whole set, as it stops one action.
#[test]
fn a_set_paused_at_a_step_dialog_plays_its_other_actions_after_ok() {
    use egui_kittest::kittest::Queryable;
    for ok in [true, false] {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 8, "height": 8})).unwrap();
        let mut a = Action::new(
            "A",
            vec![
                ("layer.new.layer".into(), json!({"name": "a"})),
                ("filter.blur.gaussianBlur".into(), json!({"radius": 1})),
                ("filter.blur.gaussianBlur".into(), json!({"radius": 2})),
                ("layer.new.layer".into(), json!({"name": "a2"})),
            ],
        );
        a.set = Some("S".into());
        a.dialogs.extend([1, 2]);
        let mut b = Action::new("B", vec![("layer.new.layer".into(), json!({"name": "b"}))]);
        b.set = Some("S".into());
        app.session.actions.list.extend([a, b]);
        let mut h = egui_kittest::Harness::builder().with_size(vec2(360.0, 400.0)).build_ui_state(|ui, app| panel(app, ui), app);
        h.run_steps(2);
        h.get_by_label("S").click();
        h.run_steps(2);
        h.get_by_label("Play selection").click();
        h.run_steps(2);
        assert_eq!(layer_names(h.state()), ["a"]);
        assert!(h.state().ui.status.contains("dialog"), "{}", h.state().ui.status);
        let first = h.state().ui.dialogs.last().map(|d| d.id).expect("the first blur's dialog");
        if !ok {
            crate::dialogs::cancel(h.state_mut(), first).unwrap();
            h.run_steps(2);
            assert_eq!(layer_names(h.state()), ["a"], "Cancel stops the set");
            continue;
        }
        crate::dialogs::confirm(h.state_mut(), first).unwrap();
        h.run_steps(2);
        // The second dialog of A pauses again; the rest of the set waits behind it.
        let second = h.state().ui.dialogs.last().map(|d| d.id).expect("the second blur's dialog");
        assert_ne!(first, second);
        crate::dialogs::confirm(h.state_mut(), second).unwrap();
        h.run_steps(2);
        let mut names = layer_names(h.state());
        names.sort();
        assert_eq!(names, ["a", "a2", "b"], "B played after A's dialogs");
        // CodeRabbit on batch 11: the count covers the whole Play, also the steps before each pause.
        assert_eq!(h.state().ui.status, "Played 5 steps");
    }
}

/// CodeRabbit on batch 11: the rest of a paused set is kept by name, so an action moved into the
/// middle of the list while the dialog is open neither plays in its place nor skips one of the set.
#[test]
fn a_set_paused_at_a_dialog_plays_its_own_actions_after_a_move() {
    use egui_kittest::kittest::Queryable;
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 8, "height": 8})).unwrap();
    for (name, set, layer) in [("A", "S", "a"), ("B", "S", "b"), ("C", "S", "c"), ("X", "Other", "x")] {
        let mut a = Action::new(name, vec![("layer.new.layer".into(), json!({"name": layer}))]);
        a.set = Some(set.into());
        app.session.actions.list.push(a);
    }
    app.session.actions.list[0].steps.push(("filter.blur.gaussianBlur".into(), json!({"radius": 1})));
    app.session.actions.list[0].dialogs.insert(1);
    let mut h = egui_kittest::Harness::builder().with_size(vec2(360.0, 400.0)).build_ui_state(|ui, app| panel(app, ui), app);
    h.run_steps(2);
    h.get_by_label("S").click();
    h.run_steps(2);
    h.get_by_label("Play selection").click();
    h.run_steps(2);
    assert_eq!(layer_names(h.state()), ["a"]);
    let dialog = h.state().ui.dialogs.last().map(|d| d.id).expect("the blur's dialog");
    h.state_mut().run("actions.move", json!({"action": "X", "to": 1})).unwrap();
    crate::dialogs::confirm(h.state_mut(), dialog).unwrap();
    h.run_steps(2);
    let mut names = layer_names(h.state());
    names.sort();
    assert_eq!(names, ["a", "b", "c"]);
}

/// CodeRabbit on batch 11: an action of the paused set deleted and made again under its name while
/// the dialog is open is a different action, so the set ends there instead of playing it.
#[test]
fn a_set_paused_at_a_dialog_ends_at_an_action_replaced_under_its_name() {
    use egui_kittest::kittest::Queryable;
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 8, "height": 8})).unwrap();
    for (name, layer) in [("A", "a"), ("B", "b"), ("C", "c")] {
        let mut a = Action::new(name, vec![("layer.new.layer".into(), json!({"name": layer}))]);
        a.set = Some("S".into());
        app.session.actions.list.push(a);
    }
    app.session.actions.list[0].steps.push(("filter.blur.gaussianBlur".into(), json!({"radius": 1})));
    app.session.actions.list[0].dialogs.insert(1);
    let mut h = egui_kittest::Harness::builder().with_size(vec2(360.0, 400.0)).build_ui_state(|ui, app| panel(app, ui), app);
    h.run_steps(2);
    h.get_by_label("S").click();
    h.run_steps(2);
    h.get_by_label("Play selection").click();
    h.run_steps(2);
    assert_eq!(layer_names(h.state()), ["a"]);
    let dialog = h.state().ui.dialogs.last().map(|d| d.id).expect("the blur's dialog");
    h.state_mut().run("actions.delete", json!({"action": "B"})).unwrap();
    let mut b = Action::new("B", vec![("layer.new.layer".into(), json!({"name": "new b"}))]);
    b.set = Some("S".into());
    h.state_mut().session.actions.list.insert(1, b);
    crate::dialogs::confirm(h.state_mut(), dialog).unwrap();
    h.run_steps(2);
    assert_eq!(layer_names(h.state()), ["a"], "neither the new B nor C after it played");
}

/// #10459 fix-up: a set whose action pauses at a dialog inside an action it calls stops there:
/// the called action resumes alone, so the next action of the set must not run on a caller that
/// never finished.
#[test]
fn a_set_paused_inside_a_called_action_continues_into_its_caller_then_the_set() {
    use egui_kittest::kittest::Queryable;
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 8, "height": 8})).unwrap();
    let before = app.session.active().unwrap().doc.clone();
    let count = app.session.active().unwrap().history.past_len();
    let mut a = Action::new("A", vec![("actions.play".into(), json!({"action": "C"})), ("layer.new.layer".into(), json!({"name": "a-tail"}))]);
    a.set = Some("S".into());
    let mut b = Action::new("B", vec![("layer.new.layer".into(), json!({"name": "b"}))]);
    b.set = Some("S".into());
    let mut c = Action::new("C", vec![("filter.blur.gaussianBlur".into(), json!({"radius": 1})), ("layer.new.layer".into(), json!({"name": "c"}))]);
    c.dialogs.insert(0);
    app.session.actions.list.extend([a, b, c]);
    let mut h = egui_kittest::Harness::builder().with_size(vec2(360.0, 400.0)).build_ui_state(|ui, app| panel(app, ui), app);
    h.run_steps(2);
    h.get_by_label("S").click();
    h.run_steps(2);
    h.get_by_label("Play selection").click();
    h.run_steps(2);
    let dialog = h.state().ui.dialogs.last().map(|d| d.id).expect("C's blur dialog");
    crate::dialogs::confirm(h.state_mut(), dialog).unwrap();
    h.run_steps(2);
    // #8386, measured: confirming a dialog inside a called action continues the caller after
    // the call, and the set's next action after that.
    assert_eq!(layer_names(h.state()), ["c", "a-tail", "b"], "C finished, then A's tail, then B");
    // One history state per played action: A with its call into C, then B.
    assert_eq!(h.state().session.active().unwrap().history.past_len(), count + 2);
    assert_eq!(h.state().session.active().unwrap().history.undo_label(), Some("Action: B"));
    let after = h.state().session.active().unwrap().doc.clone();
    assert!(h.state_mut().session.undo());
    assert_eq!(layer_names(h.state()), ["c", "a-tail"]);
    assert_eq!(h.state().session.active().unwrap().history.undo_label(), Some("Action: A"));
    assert!(h.state_mut().session.undo());
    assert_eq!(h.state().session.active().unwrap().doc.as_ref(), before.as_ref());
    assert!(h.state_mut().session.redo());
    assert!(h.state_mut().session.redo());
    assert_eq!(h.state().session.active().unwrap().doc.as_ref(), after.as_ref());
}

/// #10459 fix-up: a set folder activated without a pointer (keyboard, accessibility) still
/// collapses and expands, and selects the set; the row reports whether it is selected.
#[test]
fn a_set_folder_toggles_without_a_pointer() {
    use egui_kittest::kittest::Queryable;
    let mut h = two_set_panel();
    use egui::accesskit::Toggled;
    use egui_kittest::kittest::NodeT;
    let selected = |h: &egui_kittest::Harness<'_, PhotocraftApp>| h.get_by_label("Retouch").accesskit_node().toggled();
    assert_eq!(selected(&h), Some(Toggled::False));
    h.get_by_label("Retouch").click_accesskit();
    h.run_steps(2);
    assert_eq!(h.state().ui.actions.collapsed_sets, ["Retouch"]);
    assert_eq!(h.state().ui.actions.selected_set.as_deref(), Some("Retouch"));
    assert_eq!(selected(&h), Some(Toggled::True), "selected and collapsed reads as selected");
    h.get_by_label("Retouch").click_accesskit();
    h.run_steps(2);
    assert!(h.state().ui.actions.collapsed_sets.is_empty());
}

/// #7535: the folder button (and the panel menu's New Set…) names an empty set, shown as a folder
/// row with no actions; New Action then records into it, as into any selected set.
#[test]
fn new_set_makes_an_empty_folder_that_new_action_records_into() {
    use egui_kittest::{Harness, kittest::Queryable};
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 8, "height": 8})).unwrap();
    let mut retouch = Action::new("Sharpen", vec![]);
    retouch.set = Some("Retouch".into());
    app.session.actions.list.push(retouch);
    let mut h = Harness::builder().with_size(vec2(360.0, 400.0)).build_ui_state(|ui, app| panel(app, ui), app);
    h.run_steps(2);
    h.get_by_label("Create new set").click();
    h.run_steps(2);
    let id = h.state().ui.dialogs.last().map(|d| d.id).expect("the New Set dialog");
    let d = h.state_mut().ui.dialog_mut(id).unwrap();
    assert!(owns_new_set(&d.fields));
    assert_eq!(d.fields["name"], "Set 1");
    d.fields.insert("name".into(), json!("Web"));
    crate::dialogs::confirm(h.state_mut(), id).unwrap();
    h.run_steps(2);
    // An empty folder row, selected.
    assert!(h.query_by_label("Web").is_some());
    assert_eq!(h.state().ui.actions.selected_set.as_deref(), Some("Web"));
    h.get_by_label("Create new action").click();
    h.run_steps(2);
    h.state_mut().run("actions.stop", json!({})).unwrap();
    let list = &h.state().session.actions.list;
    assert_eq!((list[1].name.as_str(), list[1].set.as_deref()), ("Action 2", Some("Web")));
    // With an action selected, New Action records into that action's set.
    h.state_mut().ui.actions.selected = Some(0);
    h.run_steps(2);
    h.get_by_label("Create new action").click();
    h.run_steps(2);
    h.state_mut().run("actions.stop", json!({})).unwrap();
    assert_eq!(h.state().session.actions.list[2].set.as_deref(), Some("Retouch"));
}

/// FredApps review on #10459: with no action and no set selected, New Action makes an action
/// outside every set, even when sets (empty ones included) exist.
#[test]
fn new_action_with_no_set_selected_is_in_no_set() {
    use egui_kittest::{Harness, kittest::Queryable};
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 8, "height": 8})).unwrap();
    let mut retouch = Action::new("Sharpen", vec![]);
    retouch.set = Some("Retouch".into());
    app.session.actions.list.push(retouch);
    app.run("actions.newSet", json!({"name": "Web"})).unwrap();
    let mut h = Harness::builder().with_size(vec2(360.0, 400.0)).build_ui_state(|ui, app| panel(app, ui), app);
    h.run_steps(2);
    assert_eq!((h.state().ui.actions.selected, h.state().ui.actions.selected_set.as_deref()), (None, None));
    h.get_by_label("Create new action").click();
    h.run_steps(2);
    h.state_mut().run("actions.stop", json!({})).unwrap();
    let list = &h.state().session.actions.list;
    assert_eq!((list.len(), list[1].name.as_str(), list[1].set.as_deref()), (2, "Action 2", None));
}

/// FredApps review on #10459: closing the Actions panel and opening it again keeps the selected
/// set and the empty sets New Set made, so New Action still records into the selected one.
#[test]
fn reopening_the_actions_panel_keeps_the_selected_set_and_empty_sets() {
    use egui_kittest::{Harness, kittest::Queryable};
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 8, "height": 8})).unwrap();
    let mut retouch = Action::new("Sharpen", vec![]);
    retouch.set = Some("Retouch".into());
    app.session.actions.list.push(retouch);
    app.run("actions.newSet", json!({"name": "Web"})).unwrap();
    app.run("actions.newSet", json!({"name": "Print"})).unwrap();
    let mut h = Harness::builder().with_size(vec2(360.0, 400.0)).build_ui_state(
        |ui, app| {
            // The Actions panel is the History group's second tab.
            if app.ui.panels.history && app.ui.dock_tabs.history == 1 {
                panel(app, ui);
            }
        },
        app,
    );
    let ctx = h.ctx.clone();
    let toggle = |h: &mut Harness<'_, PhotocraftApp>| {
        crate::menus::invoke(h.state_mut(), &ctx, "window.panel.actions", json!({})).unwrap();
        h.run_steps(2);
    };
    if !(h.state().ui.panels.history && h.state().ui.dock_tabs.history == 1) {
        toggle(&mut h);
    }
    h.run_steps(2);
    h.get_by_label("Web").click();
    h.run_steps(2);
    assert_eq!(h.state().ui.actions.selected_set.as_deref(), Some("Web"));
    toggle(&mut h);
    assert!(!h.state().ui.panels.history, "closed");
    assert!(h.query_by_label("Web").is_none());
    toggle(&mut h);
    assert!(h.state().ui.panels.history && h.state().ui.dock_tabs.history == 1, "open again");
    assert!(h.query_by_label("Web").is_some() && h.query_by_label("Print").is_some(), "the empty sets");
    assert_eq!(h.state().session.actions.sets, ["Web", "Print"]);
    assert_eq!((h.state().ui.actions.selected, h.state().ui.actions.selected_set.as_deref()), (None, Some("Web")));
    h.get_by_label("Create new action").click();
    h.run_steps(2);
    h.state_mut().run("actions.stop", json!({})).unwrap();
    assert_eq!(h.state().session.actions.list[1].set.as_deref(), Some("Web"));
}

/// #7535: with a set's folder row selected and no action, Start Recording (like the Record button)
/// has no action to append to, so it records a new action into that set, as New Action does.
#[test]
fn start_recording_with_a_set_selected_records_a_new_action_into_it() {
    use egui_kittest::{Harness, kittest::Queryable};
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 8, "height": 8})).unwrap();
    let mut retouch = Action::new("Sharpen", vec![]);
    retouch.set = Some("Retouch".into());
    app.session.actions.list.push(retouch);
    app.run("actions.newSet", json!({"name": "Web"})).unwrap();
    let mut h = Harness::builder().with_size(vec2(360.0, 600.0)).build_ui_state(
        |ui, app| {
            panel_menu(app, ui);
            panel(app, ui);
        },
        app,
    );
    h.run_steps(2);
    h.get_by_label("Web").click();
    h.run_steps(2);
    assert_eq!((h.state().ui.actions.selected, h.state().ui.actions.selected_set.as_deref()), (None, Some("Web")));
    h.get_by_label("Start Recording").click();
    h.run_steps(2);
    h.state_mut().run("actions.stop", json!({})).unwrap();
    let list = &h.state().session.actions.list;
    assert_eq!(list.len(), 2);
    assert_eq!(list[1].set.as_deref(), Some("Web"));
}

/// Recording into a collapsed, selected set (New Action or Start Recording) expands the set
/// again, so the new action and its steps show.
#[test]
fn recording_into_a_collapsed_set_expands_it() {
    use egui_kittest::{Harness, kittest::Queryable};
    for label in ["Create new action", "Start Recording"] {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 8, "height": 8})).unwrap();
        let mut retouch = Action::new("Sharpen", vec![]);
        retouch.set = Some("Retouch".into());
        app.session.actions.list.push(retouch);
        let mut h = Harness::builder().with_size(vec2(360.0, 600.0)).build_ui_state(
            |ui, app| {
                panel_menu(app, ui);
                panel(app, ui);
            },
            app,
        );
        h.run_steps(2);
        click_set_triangle(&mut h, "Retouch");
        h.get_by_label("Retouch").click();
        h.run_steps(2);
        assert_eq!(h.state().ui.actions.collapsed_sets, ["Retouch"], "{label}");
        assert!(h.query_by_label("Sharpen").is_none(), "{label}");
        h.get_by_label(label).click();
        h.run_steps(2);
        assert!(h.state().ui.actions.collapsed_sets.is_empty(), "{label}");
        assert_eq!(h.state().ui.actions.selected, Some(1), "{label}");
        assert!(h.query_by_label("Action 2").is_some(), "{label}");
        h.state_mut().run("actions.stop", json!({})).unwrap();
    }
}

/// Save Actions… after a click on set B's folder row saves set B, not the first action's set.
#[test]
fn save_actions_with_a_set_selected_saves_that_set() {
    use egui_kittest::{Harness, kittest::Queryable};
    let (file_dialog, asked) = crate::file_dialog::fake(vec![None]);
    let services = crate::Services { file_dialog: Some(file_dialog), ..Default::default() };
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
    app.run("file.new", json!({"width": 8, "height": 8})).unwrap();
    for (name, set) in [("Sharpen", "Retouch"), ("Resize", "Web")] {
        let mut action = Action::new(name, vec![("layer.new.layer".into(), json!({}))]);
        action.set = Some(set.into());
        app.session.actions.list.push(action);
    }
    let mut h = Harness::builder().with_size(vec2(360.0, 600.0)).build_ui_state(
        |ui, app| {
            panel_menu(app, ui);
            panel(app, ui);
        },
        app,
    );
    h.run_steps(2);
    h.get_by_label("Web").click();
    h.run_steps(2);
    assert_eq!((h.state().ui.actions.selected, h.state().ui.actions.selected_set.as_deref()), (None, Some("Web")));
    h.get_by_label("Save Actions…").click();
    h.run_steps(2);
    h.state_mut().poll_file_dialog(&egui::Context::default(), None);
    assert!(matches!(asked.borrow().as_slice(), [crate::file_dialog::FileDialogRequest::Save { suggested }] if suggested == "Web.atn"), "{:?}", asked.borrow());
}

/// FredApps review on #10459: a set name can hold characters a file name can't. Save Actions…
/// suggests the file with them replaced, and the set keeps its name in the file. The set here
/// is an empty one New Set made, which Save Actions… writes as Photoshop does.
#[test]
fn save_actions_suggests_a_file_name_without_path_characters() {
    use egui_kittest::{Harness, kittest::Queryable};
    let (file_dialog, asked) = crate::file_dialog::fake(vec![None]);
    let services = crate::Services { file_dialog: Some(file_dialog), ..Default::default() };
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
    app.run("file.new", json!({"width": 8, "height": 8})).unwrap();
    app.session.actions.list.push(Action::new("Loose", vec![]));
    let set = "Web/Print\\Mail: v2";
    app.run("actions.newSet", json!({"name": set})).unwrap();
    let mut h = Harness::builder().with_size(vec2(360.0, 600.0)).build_ui_state(
        |ui, app| {
            panel_menu(app, ui);
            panel(app, ui);
        },
        app,
    );
    h.run_steps(2);
    h.get_by_label(set).click();
    h.run_steps(2);
    h.get_by_label("Save Actions…").click();
    h.run_steps(2);
    h.state_mut().poll_file_dialog(&egui::Context::default(), None);
    assert!(
        matches!(asked.borrow().as_slice(), [crate::file_dialog::FileDialogRequest::Save { suggested }] if suggested == "Web_Print_Mail_ v2.atn"),
        "{:?}",
        asked.borrow()
    );
    let saved = h.state_mut().run("actions.save", json!({"set": set})).unwrap();
    assert_eq!(saved["set"], set, "the file keeps the set's name");
}

/// An app with no actions and one empty set New Set made, its folder row shown with the panel
/// menu, and a fake Save dialog whose requests the test reads.
fn empty_set_save_harness(
    set: &str,
) -> (egui_kittest::Harness<'static, PhotocraftApp>, std::rc::Rc<std::cell::RefCell<Vec<crate::file_dialog::FileDialogRequest>>>) {
    let (file_dialog, asked) = crate::file_dialog::fake(vec![None]);
    let services = crate::Services { file_dialog: Some(file_dialog), ..Default::default() };
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
    app.run("file.new", json!({"width": 8, "height": 8})).unwrap();
    app.run("actions.newSet", json!({"name": set})).unwrap();
    // With no action, the panel shows its "Record ● … play ▶" hint, which the test fonts lack.
    let mut h = egui_kittest::Harness::builder().with_size(vec2(360.0, 600.0)).allow_missing_glyphs().build_ui_state(
        |ui, app| {
            panel_menu(app, ui);
            panel(app, ui);
        },
        app,
    );
    h.run_steps(2);
    (h, asked)
}

/// CodeRabbit review on #10459: with no action at all, Save Actions… is still enabled for a
/// selected empty set (the engine saves it), and stays disabled with nothing to save.
#[test]
fn save_actions_is_enabled_for_a_selected_empty_set_with_no_actions() {
    use egui_kittest::kittest::Queryable;
    let (mut h, asked) = empty_set_save_harness("Later");
    h.get_by_label("Save Actions…").click();
    h.run_steps(2);
    h.state_mut().poll_file_dialog(&egui::Context::default(), None);
    assert!(asked.borrow().is_empty(), "no set selected, nothing to save: {:?}", asked.borrow());
    h.get_by_label("Later").click();
    h.run_steps(2);
    assert_eq!(h.state().ui.actions.selected_set.as_deref(), Some("Later"));
    h.get_by_label("Save Actions…").click();
    h.run_steps(2);
    h.state_mut().poll_file_dialog(&egui::Context::default(), None);
    assert!(
        matches!(asked.borrow().as_slice(), [crate::file_dialog::FileDialogRequest::Save { suggested }] if suggested == "Later.atn"),
        "{:?}",
        asked.borrow()
    );
}

/// CodeRabbit review on #10459: Save Set… in an empty set folder's context menu saves that set,
/// as it does for a folder with actions.
#[test]
fn save_set_on_an_empty_set_folder_opens_the_save_dialog() {
    use egui_kittest::kittest::Queryable;
    let (mut h, asked) = empty_set_save_harness("Later");
    h.get_by_label("Later").click_secondary();
    h.run_steps(2);
    h.get_by_label("Save Set…").click();
    h.run_steps(2);
    h.state_mut().poll_file_dialog(&egui::Context::default(), None);
    assert!(
        matches!(asked.borrow().as_slice(), [crate::file_dialog::FileDialogRequest::Save { suggested }] if suggested == "Later.atn"),
        "{:?}",
        asked.borrow()
    );
}

/// CodeRabbit review on #10459: a set named after a Windows device (`CON`, `com1.v2`) or holding a
/// control character gets a suggested file name Windows can create.
#[test]
fn save_actions_suggests_a_windows_safe_name_for_device_and_control_character_sets() {
    use egui_kittest::kittest::Queryable;
    for (set, suggested) in [("CON", "CON_.atn"), ("com1.v2", "com1_.v2.atn"), ("Web\u{1}Print\tMail", "Web_Print_Mail.atn"), ("Print", "Print.atn")] {
        let (mut h, asked) = empty_set_save_harness(set);
        h.get_by_label(set).click();
        h.run_steps(2);
        h.get_by_label("Save Actions…").click();
        h.run_steps(2);
        h.state_mut().poll_file_dialog(&egui::Context::default(), None);
        assert!(
            matches!(asked.borrow().as_slice(), [crate::file_dialog::FileDialogRequest::Save { suggested: s }] if s == suggested),
            "{set:?}: {:?}",
            asked.borrow()
        );
    }
}

/// CodeRabbit on batch 11: a set name longer than a file name may be is cut to 255 bytes with its
/// suffix, at a character boundary.
#[test]
fn save_actions_suggests_a_file_name_of_at_most_255_bytes() {
    use egui_kittest::kittest::Queryable;
    let set = "é".repeat(200);
    let (mut h, asked) = empty_set_save_harness(&set);
    h.get_by_label(set.as_str()).click();
    h.run_steps(2);
    h.get_by_label("Save Actions…").click();
    h.run_steps(2);
    h.state_mut().poll_file_dialog(&egui::Context::default(), None);
    let asked = asked.borrow();
    let [crate::file_dialog::FileDialogRequest::Save { suggested }] = asked.as_slice() else { panic!("{asked:?}") };
    assert_eq!(*suggested, format!("{}.atn", "é".repeat(125)));
}

#[test]
fn moving_an_action_next_to_another_set_joins_it() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let mut a = Action::new("A", vec![]);
    a.set = Some("One".into());
    let mut b = Action::new("B", vec![]);
    b.set = Some("Two".into());
    app.session.actions.list.push(a);
    app.session.actions.list.push(b);
    app.session.actions.list.push(Action::new("C", vec![]));
    // C dropped below B: it joins B's set, "Two".
    move_row(&mut app, DragRow { action: 2, step: None }, 2);
    assert_eq!(app.session.actions.list[2].set.as_deref(), Some("Two"));
    // B dropped to the top, above A: the row below decides when there is none above.
    move_row(&mut app, DragRow { action: 1, step: None }, 0);
    assert_eq!((app.session.actions.list[0].name.as_str(), app.session.actions.list[0].set.as_deref()), ("B", Some("One")));
    // C dropped between A (One) and nothing: the row above wins.
    move_row(&mut app, DragRow { action: 2, step: None }, 2);
    assert_eq!((app.session.actions.list[2].name.as_str(), app.session.actions.list[2].set.as_deref()), ("C", Some("One")));
}

#[test]
fn insert_stop_dialog_adds_a_stop_after_the_selected_step() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 8, "height": 8})).unwrap();
    app.session.actions.list.push(Action::new("A", vec![("select.all".into(), json!({})), ("select.inverse".into(), json!({}))]));
    assert!(insert_stop_dialog(&mut app).is_err(), "needs a selected action");
    app.ui.actions.selected = Some(0);
    app.ui.actions.selected_step = Some(0);
    let id = insert_stop_dialog(&mut app).unwrap()["dialog"].as_u64().unwrap();
    let d = app.ui.dialog_mut(id).unwrap();
    assert!(owns_insert_stop(&d.fields));
    assert_eq!(d.fields["at"], 1);
    d.fields.insert("message".into(), json!("Check the mask"));
    d.fields.insert("allowContinue".into(), json!(true));
    crate::dialogs::confirm(&mut app, id).unwrap();
    let steps = &app.session.actions.list[0].steps;
    assert_eq!(steps[1], (photocraft_engine::atn_cmds::STOP_MESSAGE.to_string(), json!({"message": "Check the mask", "allowContinue": true})));
    assert!(app.session.actions.list[0].dialogs.contains(&1), "an inserted Stop has its dialog toggle on");
    assert_eq!((app.ui.actions.selected, app.ui.actions.selected_step), (Some(0), Some(1)));
    assert_eq!(step_label(&steps[1].0, &steps[1].1), "Stop: Check the mask");
    // Without a selected step the stop goes last; the panel's menu wiring is exercised by name.
    app.ui.actions.selected_step = None;
    let id = insert_stop_dialog(&mut app).unwrap()["dialog"].as_u64().unwrap();
    assert_eq!(app.ui.dialog_mut(id).unwrap().fields["at"], 3);
    crate::dialogs::cancel(&mut app, id).unwrap();
    assert_eq!(app.session.actions.list[0].steps.len(), 3);
}

#[test]
fn dialog_worker_failure_stops_action_before_following_steps() {
    for (background, locked) in [(true, true), (false, true), (true, false)] {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 8, "height": 6, "background": "#203040"})).unwrap();
        let mut action = Action::new(
            "Worker Result",
            vec![
                ("layer.setProps".into(), json!({"name": "Prefix", "locks": {"pixels": locked}})),
                ("filter.blur.gaussianBlur".into(), json!({"radius": 1.5})),
                ("layer.new.layer".into(), json!({"name": "Suffix"})),
            ],
        );
        action.dialogs.insert(1);
        app.session.actions.list.push(action);
        app.background_jobs = background;
        let paused = play(&mut app, &json!({"action": "Worker Result"})).unwrap();
        let dialog = paused["paused"]["dialog"].as_u64().unwrap();
        let result = crate::dialogs::confirm(&mut app, dialog);
        let suffix = app.session.active().unwrap().doc.walk().iter().any(|(_, _, layer)| layer.name == "Suffix");

        assert_eq!(suffix, !locked, "background={background}, locked={locked}: {result:?}");
        if locked && background {
            assert!(result.as_ref().is_err_and(|e| e.contains("locked")), "failed worker must fail the dialog confirmation: {result:?}");
            assert!(app.ui.status_error, "the action must report the worker failure");
            assert!(app.ui.status.contains("locked"), "{}", app.ui.status);
            assert_eq!(app.session.jobs_with_recent()[0].state, "failed");
        } else if locked {
            assert!(result.is_err(), "the foreground command rejects the locked layer");
        } else {
            assert!(result.is_ok(), "the successful worker lets the action continue: {result:?}");
        }
        assert_eq!(app.session.actions.playing, 0);
        assert!(app.session.actions.playback_stack.is_empty());
    }
}

#[test]
fn a_check_spelling_step_with_its_dialog_opens_it_and_done_continues() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 300, "height": 100})).unwrap();
    app.run("type.create", json!({"x": 10, "y": 50, "text": "Teh cat"})).unwrap();
    let mut a = Action::new("Spell", vec![("edit.checkSpelling".into(), json!({"allLayers": false})), ("layer.new.layer".into(), json!({"name": "after"}))]);
    a.dialogs.insert(0);
    app.session.actions.list.push(a);
    let v = play(&mut app, &json!({"action": "Spell"})).unwrap();
    assert_eq!((v["ran"].as_u64(), v["paused"]["step"].as_u64()), (Some(0), Some(0)), "{v}");
    assert_eq!(v["paused"]["dialog"], crate::type_panels_ui::SPELLING);
    let d = app.ui.type_panels.spell.clone().expect("Check Spelling is open");
    assert!(!d.all_layers, "the dialog opens with Check All Layers as recorded");
    assert_eq!(d.items.len(), 1);
    assert!(!layer_names(&app).contains(&"after".to_string()), "nothing after the dialog ran yet");
    crate::type_panels_ui::close_spelling(&mut app);
    assert!(layer_names(&app).contains(&"after".to_string()), "Done played the rest");
    assert!(app.ui.status.starts_with("Played"), "{}", app.ui.status);
    // With its dialog off, the step checks without opening anything and changes nothing.
    app.session.actions.list[0].dialogs.clear();
    let v = play(&mut app, &json!({"action": "Spell"})).unwrap();
    assert_eq!(v["ran"], 2, "{v}");
    assert!(app.ui.type_panels.spell.is_none());
    let st = app.session.active().unwrap();
    let texts: Vec<String> = st
        .doc
        .walk()
        .into_iter()
        .filter_map(|(_, _, l)| match &l.content {
            photocraft_doc::LayerContent::Text(t) => Some(t.text.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(texts, ["Teh cat"]);
}

/// Check Spelling opened by hand keeps its place: an action's Check Spelling step fails instead
/// of replacing it.
#[test]
fn a_check_spelling_step_fails_while_the_dialog_is_open_by_hand() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 300, "height": 100})).unwrap();
    app.run("type.create", json!({"x": 10, "y": 50, "text": "Teh cat"})).unwrap();
    let mut a = Action::new("Spell", vec![("edit.checkSpelling".into(), json!({"allLayers": false})), ("layer.new.layer".into(), json!({"name": "after"}))]);
    a.dialogs.insert(0);
    app.session.actions.list.push(a);
    crate::type_panels_ui::open_spelling(&mut app, true).unwrap();
    let v = play(&mut app, &json!({"action": "Spell"})).unwrap();
    assert_eq!(v["failed"]["error"], "Check Spelling is already open", "{v}");
    let d = app.ui.type_panels.spell.as_ref().expect("the dialog stays open");
    assert!(d.all_layers && d.resume.is_none(), "the hand-opened dialog is kept");
    assert!(!layer_names(&app).contains(&"after".to_string()));
}

/// A correction step (`action` set) runs as it did before the dialog toggle applied to Check
/// Spelling: its dialog switch opens nothing.
#[test]
fn a_check_spelling_correction_with_its_dialog_on_runs_without_opening_it() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 300, "height": 100})).unwrap();
    app.run("type.create", json!({"x": 10, "y": 50, "text": "Teh cat"})).unwrap();
    let change = json!({"action": "change", "start": 0, "end": 3, "word": "Teh", "replace": "The"});
    let mut a = Action::new("Fix", vec![("edit.checkSpelling".into(), change), ("layer.new.layer".into(), json!({"name": "after"}))]);
    a.dialogs.insert(0);
    app.session.actions.list.push(a);
    let v = play(&mut app, &json!({"action": "Fix"})).unwrap();
    assert_eq!(v["ran"], 2, "{v}");
    assert!(app.ui.type_panels.spell.is_none(), "no dialog opened");
    let st = app.session.active().unwrap();
    let texts: Vec<String> = st
        .doc
        .walk()
        .into_iter()
        .filter_map(|(_, _, l)| match &l.content {
            photocraft_doc::LayerContent::Text(t) => Some(t.text.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(texts, ["The cat"]);
}

/// Done after another document became active ends the waiting action there: its remaining steps
/// don't run on the other document.
#[test]
fn check_spelling_done_on_another_document_ends_the_waiting_action() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 300, "height": 100})).unwrap();
    app.run("type.create", json!({"x": 10, "y": 50, "text": "Teh cat"})).unwrap();
    let mut a = Action::new("Spell", vec![("edit.checkSpelling".into(), json!({"allLayers": false})), ("layer.new.layer".into(), json!({"name": "after"}))]);
    a.dialogs.insert(0);
    app.session.actions.list.push(a);
    let v = play(&mut app, &json!({"action": "Spell"})).unwrap();
    assert_eq!(v["paused"]["dialog"], crate::type_panels_ui::SPELLING, "{v}");
    app.run("file.new", json!({"width": 64, "height": 64})).unwrap();
    crate::type_panels_ui::close_spelling(&mut app);
    for st in app.session.documents() {
        assert!(!st.doc.walk().iter().any(|(_, _, l)| l.name == "after"), "a step after the dialog ran");
    }
    assert!(app.paused_playbacks.paused.is_empty());
    assert!(app.ui.status_error, "{}", app.ui.status);
    assert!(app.ui.status.contains("document"), "{}", app.ui.status);
}

fn deny_spelling(id: &str, p: &Value) -> photocraft_engine::Result<()> {
    if id == "edit.checkSpelling" && p.get("action").is_none() { Err(photocraft_engine::EngineError::Other("spelling denied".into())) } else { Ok(()) }
}

/// Done records the step and continues the waiting action: when recording it fails, the status
/// keeps that error next to the playback's outcome.
#[test]
fn check_spelling_done_keeps_a_recording_error_beside_the_playback_status() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 300, "height": 100})).unwrap();
    app.run("type.create", json!({"x": 10, "y": 50, "text": "Teh cat"})).unwrap();
    let mut a = Action::new("Spell", vec![("edit.checkSpelling".into(), json!({"allLayers": false})), ("layer.new.layer".into(), json!({"name": "after"}))]);
    a.dialogs.insert(0);
    app.session.actions.list.push(a);
    let v = play(&mut app, &json!({"action": "Spell"})).unwrap();
    assert_eq!(v["paused"]["dialog"], crate::type_panels_ui::SPELLING, "{v}");
    app.run("actions.record", json!({"name": "Rec"})).unwrap();
    app.session.authorize = Some(deny_spelling);
    crate::type_panels_ui::close_spelling(&mut app);
    app.session.authorize = None;
    assert!(layer_names(&app).contains(&"after".to_string()), "Done played the rest");
    assert!(app.ui.status_error, "{}", app.ui.status);
    assert!(app.ui.status.contains("spelling denied"), "{}", app.ui.status);
    assert!(app.ui.status.contains("Played"), "{}", app.ui.status);
}

#[test]
fn check_spelling_opened_again_while_an_action_waits_keeps_its_continuation() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 300, "height": 100})).unwrap();
    app.run("type.create", json!({"x": 10, "y": 50, "text": "Teh cat"})).unwrap();
    let mut a = Action::new("Spell", vec![("edit.checkSpelling".into(), json!({"allLayers": false})), ("layer.new.layer".into(), json!({"name": "after"}))]);
    a.dialogs.insert(0);
    app.session.actions.list.push(a);
    let v = play(&mut app, &json!({"action": "Spell"})).unwrap();
    assert_eq!(v["paused"]["dialog"], crate::type_panels_ui::SPELLING, "{v}");
    let token = app.ui.type_panels.spell.as_ref().and_then(|d| d.resume);
    assert!(token.is_some());
    // Edit › Check Spelling… while the action waits.
    crate::type_panels_ui::open_spelling(&mut app, true).unwrap();
    assert_eq!(app.ui.type_panels.spell.as_ref().and_then(|d| d.resume), token, "the reopened dialog keeps the token");
    // A second playback that reaches a Check Spelling step with its dialog fails there.
    let v = play(&mut app, &json!({"action": "Spell"})).unwrap();
    assert_eq!(v["failed"]["step"], 0, "{v}");
    assert_eq!(app.ui.type_panels.spell.as_ref().and_then(|d| d.resume), token, "the waiting action keeps the dialog");
    assert!(!layer_names(&app).contains(&"after".to_string()));
    crate::type_panels_ui::close_spelling(&mut app);
    let after = layer_names(&app).iter().filter(|n| *n == "after").count();
    assert_eq!(after, 1, "Done continued the waiting action once");
    assert!(app.paused_playbacks.paused.is_empty());
}

#[test]
fn a_dialog_toggled_step_pauses_playback_and_resumes_on_ok() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 16, "height": 16})).unwrap();
    let mut a = Action::new(
        "Blur",
        vec![("layer.new.layer".into(), json!({})), ("filter.blur.gaussianBlur".into(), json!({"radius": 3})), ("select.all".into(), json!({}))],
    );
    a.dialogs.insert(1);
    app.session.actions.list.push(a);
    let v = play(&mut app, &json!({"action": "Blur"})).unwrap();
    assert_eq!(v["ran"], 1, "{v}");
    assert_eq!(v["paused"]["step"], 1);
    let id = v["paused"]["dialog"].as_u64().unwrap();
    let d = app.ui.dialog_mut(id).unwrap();
    assert_eq!(d.fields["radius"], 3, "the dialog opens on the recorded values");
    assert!(d.fields[RESUME].is_u64(), "the dialog carries only the pause's token: {}", d.fields[RESUME]);
    assert!(app.session.active().unwrap().doc.selection.is_none(), "nothing after the dialog ran yet");
    report_play(&mut app, &v);
    assert!(app.ui.status.contains("dialog"), "{}", app.ui.status);
    // OK runs the blur with the dialog's values and plays the rest.
    app.ui.dialog_mut(id).unwrap().fields.insert("radius".into(), json!(5));
    crate::dialogs::confirm(&mut app, id).unwrap();
    assert!(app.ui.dialogs.is_empty());
    assert!(app.session.active().unwrap().doc.selection.is_some(), "the step after the dialog ran");
    assert!(app.ui.status.starts_with("Played"), "{}", app.ui.status);
    // Cancel stops the action there.
    app.run("select.deselect", json!({})).unwrap();
    let v = play(&mut app, &json!({"action": "Blur"})).unwrap();
    let id = v["paused"]["dialog"].as_u64().unwrap();
    crate::dialogs::cancel(&mut app, id).unwrap();
    assert!(app.session.active().unwrap().doc.selection.is_none());
    assert!(app.ui.status.contains("stopped"), "{}", app.ui.status);
    // With background jobs (the desktop default) the resumed steps wait for the dialog's
    // filter to land instead of meeting a locked document.
    app.background_jobs = true;
    let v = play(&mut app, &json!({"action": "Blur"})).unwrap();
    let id = v["paused"]["dialog"].as_u64().unwrap();
    crate::dialogs::confirm(&mut app, id).unwrap();
    assert!(app.session.active().unwrap().doc.selection.is_some(), "the step after the background blur ran");
    assert!(app.ui.status.starts_with("Played"), "{}", app.ui.status);
    app.background_jobs = false;
    // Agents play headlessly: no pause, the recorded values run.
    app.automation_input = true;
    let v = play(&mut app, &json!({"action": "Blur"})).unwrap();
    assert_eq!(v["ran"], 3, "{v}");
    assert!(v.get("paused").is_none());
}

#[test]
fn equalize_step_with_a_selection_opens_its_choices() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 16, "height": 16})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("select.rect", json!({"x": 0, "y": 0, "width": 16, "height": 8})).unwrap();
    app.run("edit.fill", json!({"color": [0.4, 0.4, 0.4]})).unwrap();
    app.run("select.rect", json!({"x": 0, "y": 8, "width": 16, "height": 8})).unwrap();
    app.run("edit.fill", json!({"color": [0.5, 0.5, 0.5]})).unwrap();
    app.run("select.rect", json!({"x": 0, "y": 0, "width": 8, "height": 16})).unwrap();
    let mut action = Action::new("Equalize", vec![(crate::equalize_ui::COMMAND.into(), json!({}))]);
    action.dialogs.insert(0);
    app.session.actions.list.push(action);
    let gray = |app: &PhotocraftApp, x: i32, y: i32| {
        let st = app.session.active().unwrap();
        st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap().pixel(x, y)[0]
    };

    let result = play(&mut app, &json!({"action": "Equalize"})).unwrap();
    let id = result["paused"]["dialog"].as_u64().unwrap();
    let fields = &app.ui.dialog_mut(id).unwrap().fields;
    assert!(crate::equalize_ui::owns(fields));
    assert_eq!(fields["selectedAreaOnly"], true);
    assert!((gray(&app, 12, 1) - 0.4).abs() < 0.01, "nothing changes before OK");

    // OK with the entire image chosen maps the pixels outside the selection too.
    app.ui.dialog_mut(id).unwrap().fields.insert("selectedAreaOnly".into(), json!(false));
    crate::dialogs::confirm(&mut app, id).unwrap();
    assert!(gray(&app, 12, 1) < 0.05 && gray(&app, 12, 12) > 0.95, "outside the selection too");
}

/// An 8×6 document, a Child action whose first step (a blur) shows its dialog and whose
/// second step adds a "Suffix" layer, and recording into Owner started.
fn recording_with_dialog_child() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 8, "height": 6, "background": "#203040"})).unwrap();
    let mut child =
        Action::new("Child", vec![("filter.blur.gaussianBlur".into(), json!({"radius": 1.5})), ("layer.new.layer".into(), json!({"name": "Suffix"}))]);
    child.dialogs.insert(0);
    app.session.actions.list.push(child);
    app.run("actions.record", json!({"name": "Owner"})).unwrap();
    app
}

fn suffixes(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().doc.walk().iter().filter(|(_, _, l)| l.name == "Suffix").count()
}

fn child_call() -> Vec<(String, Value)> {
    vec![("actions.play".to_string(), json!({"action": "Child"}))]
}

/// Play Child and return the id of its step dialog.
fn pause_child(app: &mut PhotocraftApp) -> u64 {
    let v = play(app, &json!({"action": "Child"})).unwrap();
    v["paused"]["dialog"].as_u64().unwrap()
}

#[test]
fn a_recorded_call_paused_by_a_dialog_records_one_call_and_replays_once() {
    let mut app = recording_with_dialog_child();
    let refusal = crate::i18n::t("Confirm or cancel the open action step dialog first").to_string();

    // Confirm: the dialog command and the resumed rest stay inside the one named call.
    let live = app.session.journal.len();
    let id = pause_child(&mut app);
    assert!(app.session.actions.recording.is_none(), "recording stays suspended while the step dialog is open");
    let names = |app: &PhotocraftApp| app.session.actions.list.iter().map(|a| a.name.clone()).collect::<Vec<_>>();
    let before = names(&app);
    assert_eq!(app.run("actions.record", json!({"name": "Intruder"})), Err(refusal.clone()), "no second recording while the call is suspended");
    assert_eq!(app.run("actions.delete", json!({"action": "Owner"})), Err(refusal.clone()), "the suspended owner can't be deleted");
    assert_eq!(app.run("actions.move", json!({"action": "Owner", "to": 0})), Err(refusal.clone()), "the suspended owner can't be moved");
    assert_eq!(names(&app), before, "the refused commands leave the list alone");
    crate::dialogs::confirm(&mut app, id).unwrap();
    assert!(app.session.actions.recording.is_some(), "recording comes back when the call completes");
    app.run("actions.stop", json!({})).unwrap();
    assert_eq!(app.session.actions.list[1].steps, child_call());
    assert_eq!(suffixes(&app), 1);

    // Replaying the recording repeats the call once.
    app.run("file.new", json!({"width": 8, "height": 6})).unwrap();
    let start = app.session.journal.len();
    app.automation_input = true;
    let v = play(&mut app, &json!({"action": "Owner"})).unwrap();
    app.automation_input = false;
    assert!(v.get("failed").is_none(), "{v}");
    assert_eq!(suffixes(&app), 1, "replay creates the suffix once");
    let edits = |from: usize| -> Vec<(String, Value)> {
        app.session.journal[from..].iter().filter(|(id, _)| !id.starts_with("actions.") && id != "file.new").cloned().collect()
    };
    let expected = vec![("filter.blur.gaussianBlur".to_string(), json!({"radius": 1.5})), ("layer.new.layer".to_string(), json!({"name": "Suffix"}))];
    assert_eq!(edits(start), expected, "replay journals the blur and the suffix once each, in order");
    assert_eq!(edits(live), expected[..].iter().chain(&expected).cloned().collect::<Vec<_>>(), "live then replay");

    // Cancel: the call ends at the dialog and is recorded once.
    app.run("actions.record", json!({"name": "Cancelled"})).unwrap();
    let id = pause_child(&mut app);
    crate::dialogs::cancel(&mut app, id).unwrap();
    assert!(app.session.actions.recording.is_some(), "cancel restores recording");
    app.run("actions.stop", json!({})).unwrap();
    assert_eq!(app.session.actions.list[2].steps, child_call());
}

#[test]
fn a_failing_step_dialog_command_ends_the_recorded_call_once() {
    let mut app = recording_with_dialog_child();
    let id = pause_child(&mut app);
    assert!(app.session.actions.recording.is_none(), "recording stays suspended while the step dialog is open");
    app.ui.dialog_mut(id).unwrap().fields.insert("radius".into(), json!("not a radius"));
    assert!(crate::dialogs::confirm(&mut app, id).is_err(), "the dialog command fails");
    assert!(app.session.actions.recording.is_some(), "recording comes back when the dialog command fails");
    assert_eq!(suffixes(&app), 0, "the action stops at the failed step");
    app.run("actions.stop", json!({})).unwrap();
    assert_eq!(app.session.actions.list[1].steps, child_call());
}

#[test]
fn an_abandoned_step_dialog_leaves_no_paused_playback() {
    let mut app = recording_with_dialog_child();
    let id = pause_child(&mut app);
    app.ui.dialog_mut(id).unwrap().fields.insert("radius".into(), json!("not a radius"));
    assert!(crate::dialogs::confirm(&mut app, id).is_err(), "the dialog command fails");
    assert!(app.paused_playbacks.paused.is_empty(), "the failed dialog ends its pause");
    app.run("actions.stop", json!({})).unwrap();
    assert_eq!(app.session.actions.list[1].steps, child_call(), "the call is recorded once");
    app.run("actions.record", json!({"name": "Next"})).unwrap();
}

#[test]
fn a_failing_resumed_playback_ends_the_recorded_call_once() {
    let mut app = recording_with_dialog_child();
    let id = pause_child(&mut app);
    // The rest of the call can't be found any more: resuming it fails.
    app.session.actions.list[0].name = "Renamed".into();
    assert!(crate::dialogs::confirm(&mut app, id).is_err(), "the resumed playback fails");
    assert!(app.session.actions.recording.is_some(), "recording comes back when the resumed playback fails");
    app.run("actions.stop", json!({})).unwrap();
    assert_eq!(app.session.actions.list[1].steps, child_call());
}

#[test]
fn a_suspended_call_is_recorded_into_its_action_after_the_list_changes() {
    let mut app = recording_with_dialog_child();
    let id = pause_child(&mut app);
    // Something else puts an action where Owner was.
    app.session.actions.list.insert(1, Action::new("Bystander", vec![]));
    crate::dialogs::confirm(&mut app, id).unwrap();
    app.run("actions.stop", json!({})).unwrap();
    assert!(app.session.actions.list[1].steps.is_empty(), "the action now at the old index gets nothing");
    assert_eq!(app.session.actions.list[2].name, "Owner");
    assert_eq!(app.session.actions.list[2].steps, child_call());
}

#[test]
fn escape_on_a_step_dialog_ends_the_recorded_call_once() {
    let app = recording_with_dialog_child();
    let mut h = egui_kittest::Harness::builder().with_size(vec2(900.0, 700.0)).build_ui_state(|ui, app| crate::dialogs::show(app, ui.ctx()), app);
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    h.run_steps(2);
    pause_child(h.state_mut());
    h.run_steps(4);
    assert!(h.state().session.actions.recording.is_none(), "recording stays suspended while the step dialog is open");
    h.key_press(egui::Key::Escape);
    h.run_steps(2);
    assert!(h.state().ui.dialogs.is_empty(), "Esc closes the step dialog");
    assert!(h.state().session.actions.recording.is_some(), "Esc restores recording");
    let app = h.state_mut();
    app.run("actions.stop", json!({})).unwrap();
    assert_eq!(app.session.actions.list[1].steps, child_call());
}

#[test]
fn a_step_dialog_keeps_its_continuation_token() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.background_jobs = false;
    app.run("file.new", json!({"width": 8, "height": 8})).unwrap();
    for (name, steps) in [
        ("First", vec![("filter.blur.gaussianBlur".into(), json!({"radius": 1}))]),
        ("Second", vec![("filter.blur.gaussianBlur".into(), json!({"radius": 1})), ("select.all".into(), json!({}))]),
    ] {
        let mut action = Action::new(name, steps);
        action.dialogs.insert(0);
        app.session.actions.list.push(action);
    }
    let first = play(&mut app, &json!({"action": "First"})).unwrap()["paused"]["dialog"].as_u64().unwrap();
    let second = play(&mut app, &json!({"action": "Second"})).unwrap()["paused"]["dialog"].as_u64().unwrap();
    let first_token = app.ui.dialog_mut(first).unwrap().fields[RESUME].clone();
    let second_token = app.ui.dialog_mut(second).unwrap().fields[RESUME].clone();
    for token in [second_token.clone(), Value::Null, json!("invalid"), json!({"action": "Second", "from": 1})] {
        let (request, _) = crate::control::ControlRequest::new("ui.dialog.set", json!({"dialog": first, "field": RESUME, "value": token}));
        let response = crate::control::handle(&mut app, &egui::Context::default(), &request);
        assert!(matches!(response, crate::control::Outcome::Done(ref v) if v["error"].as_str().is_some_and(|e| e.contains("read-only"))));
        assert_eq!(app.ui.dialog_mut(first).unwrap().fields[RESUME], first_token);
        assert_eq!(app.ui.dialog_mut(second).unwrap().fields[RESUME], second_token);
        assert_eq!(app.paused_playbacks.paused.len(), 2);
    }
    crate::dialogs::confirm(&mut app, first).unwrap();
    assert!(app.session.active().unwrap().doc.selection.is_none(), "First must not run Second's remaining step");
    crate::dialogs::confirm(&mut app, second).unwrap();
    assert!(app.session.active().unwrap().doc.selection.is_some());
    assert!(app.paused_playbacks.paused.is_empty());
}

fn recording_with_displace_child() -> PhotocraftApp {
    let mut app = recording_with_dialog_child();
    app.background_jobs = false;
    app.session.actions.list[0].steps[0] = ("filter.distort.displace".into(), json!({}));
    app
}

fn assert_displace_call_finished(app: &mut PhotocraftApp, suffix_count: usize) {
    assert!(!app.file_dialog_open());
    assert!(app.ui.dialogs.is_empty());
    assert!(app.paused_playbacks.paused.is_empty());
    assert!(app.session.actions.recording.is_some());
    assert_eq!(suffixes(app), suffix_count);
    app.run("actions.stop", json!({})).unwrap();
    assert_eq!(app.session.actions.list[1].steps, child_call(), "record the child call exactly once");
    app.run("actions.record", json!({"name": "Next"})).unwrap();
}

#[test]
fn a_displace_step_restores_recording_when_the_map_picker_fails_or_is_cancelled() {
    use crate::file_dialog::FileDialogAnswer;
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("missing.png").to_string_lossy().into_owned();
    for answer in [
        None,
        Some(FileDialogAnswer::Unavailable("no picker available".into())),
        Some(FileDialogAnswer::Paths(Vec::new())),
        Some(FileDialogAnswer::Contents("map.png".into(), Vec::new())),
        Some(FileDialogAnswer::Paths(vec![missing])),
    ] {
        let mut app = recording_with_displace_child();
        let (show, _) = crate::file_dialog::fake(vec![answer]);
        app.services.file_dialog = Some(show);
        let id = pause_child(&mut app);
        crate::dialogs::confirm(&mut app, id).unwrap();
        assert!(app.file_dialog_open());
        assert!(app.session.actions.recording.is_none());
        app.poll_file_dialog(&egui::Context::default(), None);
        assert_displace_call_finished(&mut app, 0);
    }
}

#[test]
fn a_displace_step_restores_recording_when_the_map_picker_cannot_start() {
    let mut app = recording_with_displace_child();
    let id = pause_child(&mut app);
    assert!(crate::dialogs::confirm(&mut app, id).is_err());
    assert_displace_call_finished(&mut app, 0);
}

#[test]
fn a_displace_step_resumes_and_records_its_call_once_after_choosing_a_map() {
    let mut app = recording_with_displace_child();
    let dir = tempfile::tempdir().unwrap();
    let map = dir.path().join("map.png");
    let bytes = photocraft_io::export(&app.session.active().unwrap().doc, "png", &Default::default()).unwrap().bytes;
    std::fs::write(&map, bytes).unwrap();
    let (show, _) = crate::file_dialog::fake(vec![Some(crate::file_dialog::FileDialogAnswer::Paths(vec![map.to_string_lossy().into_owned()]))]);
    app.services.file_dialog = Some(show);
    let id = pause_child(&mut app);
    crate::dialogs::confirm(&mut app, id).unwrap();
    app.poll_file_dialog(&egui::Context::default(), None);
    assert_displace_call_finished(&mut app, 1);
}

/// Recorded dialogs use sanitized public parameters and retain server-owned command and playback context.
#[test]
fn a_step_dialog_takes_only_the_steps_values() {
    for (command, key, value) in
        [("filter.blur.gaussianBlur", "radius", 2), ("image.adjustments.curves", "smooth", 2), ("image.adjustments.levels", "gamma", 2)]
    {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 16, "height": 16})).unwrap();
        let mut params = json!({"__command": "select.all", "__label": "Other", "__adjust": "invert", "__resumeAction": 999999, "__insertStop": true, "__smartFilter": {"layer": 1}});
        params[key] = json!(value);
        let mut a = Action::new("Step", vec![(command.into(), params)]);
        a.dialogs.insert(0);
        app.session.actions.list.push(a);
        let v = play(&mut app, &json!({"action": "Step"})).unwrap();
        let id = v["paused"]["dialog"].as_u64().unwrap();
        let d = app.ui.dialog_mut(id).unwrap();
        assert_eq!(d.fields[key], value);
        assert_eq!(d.fields["__command"], command);
        assert_ne!(d.fields["__label"], "Other");
        assert_eq!(d.fields.get("__adjust").and_then(Value::as_str), command.strip_prefix("image.adjustments."));
        assert!(!d.fields.contains_key("__insertStop"));
        assert!(!d.fields.contains_key("__smartFilter"));
        assert_ne!(d.fields[RESUME], 999999);
        crate::dialogs::confirm(&mut app, id).unwrap();
        assert!(app.ui.dialogs.is_empty());
        assert!(app.paused_playbacks.paused.is_empty());
    }
}

fn deny_blur(id: &str, _: &Value) -> photocraft_engine::Result<()> {
    if id == "filter.blur.gaussianBlur" { Err(photocraft_engine::EngineError::Other("blur denied".into())) } else { Ok(()) }
}

/// Permit the recorded small blur but reject an edited radius, so admission is tested against final dialog parameters.
fn deny_wide_blur(id: &str, params: &Value) -> photocraft_engine::Result<()> {
    if id == "filter.blur.gaussianBlur" && params["radius"].as_f64().is_some_and(|radius| radius > 1.0) {
        Err(photocraft_engine::EngineError::Other("edited blur radius denied".into()))
    } else {
        Ok(())
    }
}

/// Build a two-tone raster whose blur changes pixels, with a real action step configured to open its dialog.
fn gated_blur_fixture(single: bool, background: bool) -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 16, "height": 16, "background": "#000000"})).unwrap();
    app.run("select.rect", json!({"x": 8, "y": 0, "width": 8, "height": 16})).unwrap();
    app.run("edit.fill", json!({"color": "#ffffff"})).unwrap();
    app.run("select.deselect", json!({})).unwrap();
    app.run("actions.setHistory", json!({"singleState": single})).unwrap();
    app.background_jobs = background;
    let mut action = Action::new("Gated blur", vec![("filter.blur.gaussianBlur".into(), json!({"radius": 1}))]);
    action.dialogs.insert(0);
    app.session.actions.list.push(action);
    app
}

/// Simulate a temporary control gate: capture it at the pause and release the session gate before later confirmation.
fn pause_gated_blur(app: &mut PhotocraftApp, gate: Option<Gate>) -> u64 {
    app.session.authorize = gate;
    let paused = play(app, &json!({"action": "Gated blur"})).unwrap();
    app.session.authorize = None; // The temporary control gate ends before a later human OK.
    paused["paused"]["dialog"].as_u64().unwrap()
}

#[test]
/// Denied edited parameters must change neither pixels nor history and must admit no background worker.
fn a_paused_dialog_authorizes_edited_params_before_history_or_worker_admission() {
    for (single, background) in [(true, false), (true, true), (false, false)] {
        let mut app = gated_blur_fixture(single, background);
        let dialog = pause_gated_blur(&mut app, Some(deny_wide_blur));
        app.ui.dialog_mut(dialog).unwrap().fields.insert("radius".into(), json!(4));
        let before = app.session.active().unwrap().doc.clone();
        let revision = app.session.active().unwrap().revision;
        let count = app.session.active().unwrap().history.past_len();
        let error = crate::dialogs::confirm(&mut app, dialog).unwrap_err();
        assert!(error.contains("edited blur radius denied"), "{error}");
        let state = app.session.active().unwrap();
        assert_eq!(state.doc.as_ref(), before.as_ref(), "denial changes no pixels");
        assert_eq!(state.revision, revision);
        assert_eq!(state.history.past_len(), count);
        assert!(app.session.job_on(state.doc.id).is_none(), "no worker was admitted");
        assert!(app.session.authorize.is_none());
        assert!(app.paused_playbacks.paused.is_empty());
        assert_eq!(app.session.actions.playing, 0);
        app.run("layer.renameLayer", json!({"name": "Outside"})).unwrap();
        assert_eq!(app.session.active().unwrap().history.past_len(), count + 1, "no action scope leaked");
    }
}

#[test]
/// Allowed captured edits and unrelated manual dialogs retain their normal synchronous and worker behavior.
fn an_allowed_paused_edit_and_an_ordinary_manual_dialog_still_run() {
    for (paused, background) in [(true, false), (true, true), (false, false)] {
        let mut app = gated_blur_fixture(true, background);
        let before = app.session.active().unwrap().doc.clone();
        let dialog = if paused {
            pause_gated_blur(&mut app, Some(deny_wide_blur))
        } else {
            crate::dialogs::open_command_dialog(&mut app, "filter.blur.gaussianBlur", "Gaussian Blur")
        };
        app.ui.dialog_mut(dialog).unwrap().fields.insert("radius".into(), json!(if paused { 1 } else { 4 }));
        crate::dialogs::confirm(&mut app, dialog).unwrap();
        let state = app.session.active().unwrap();
        assert_ne!(state.doc.layers[0].surface().unwrap().pixel(7, 8), before.layers[0].surface().unwrap().pixel(7, 8));
        assert!(app.session.job_on(state.doc.id).is_none(), "the resumed action waited for its worker");
        assert!(app.session.authorize.is_none(), "the transient captured gate was restored");
        assert!(app.paused_playbacks.paused.is_empty());
        assert_eq!(app.session.actions.playing, 0);
    }
}

#[test]
/// A paused action's captured permission cannot bypass a stricter live policy on confirmation.
fn a_stricter_current_gate_still_denies_the_captured_dialog_edit() {
    for service in [false, true] {
        let mut app = gated_blur_fixture(true, true);
        let dialog = pause_gated_blur(&mut app, Some(deny_wide_blur));
        let before = app.session.active().unwrap().doc.clone();
        if service {
            app.automation_input = true;
            app.services.automation_authorize = Some(deny_blur);
        } else {
            app.session.authorize = Some(deny_blur);
        }
        let error = crate::dialogs::confirm(&mut app, dialog).unwrap_err();
        assert!(error.contains("blur denied"), "{error}");
        assert_eq!(app.session.active().unwrap().doc.as_ref(), before.as_ref());
        if service {
            assert!(app.session.authorize.is_none());
        } else {
            assert!(app.session.authorize.unwrap()("filter.blur.gaussianBlur", &json!({"radius": 1})).is_err(), "the live gate was restored");
        }
        assert!(app.paused_playbacks.paused.is_empty());
    }
}

#[test]
/// Spelling corrections consult live Session and automation-service policies before resuming the owned history.
fn paused_spelling_checks_the_live_gate_before_resuming_history() {
    fn deny_correction(id: &str, p: &Value) -> photocraft_engine::Result<()> {
        if id == "edit.checkSpelling" && p["action"] == "changeAll" { Err(photocraft_engine::EngineError::Other("live spelling gate".into())) } else { Ok(()) }
    }
    for service in 0..3 {
        let (mut app, _, _) = paused_history_spelling(true, Some(deny_second_spelling_change));
        let before = app.session.active().unwrap().doc.clone();
        let rows = app.session.active().unwrap().history.entries();
        let token = app.ui.type_panels.spell.as_ref().unwrap().resume.unwrap();
        if service != 1 {
            app.session.authorize = None;
        }
        match service {
            0 => app.session.authorize = Some(deny_correction),
            1 => {
                app.automation_input = true;
                app.services.automation_authorize = Some(deny_correction);
            }
            _ => {
                app.automation_input = true;
                app.services.automation_command = Some(Box::new(|id, _| if id == "edit.checkSpelling" { Err("live spelling gate".into()) } else { Ok(()) }));
            }
        }
        assert!(spelling_change(&mut app, json!({"action":"changeAll","word":"Teh","replace":"The"})).is_err());
        assert_eq!(app.session.active().unwrap().doc.as_ref(), before.as_ref());
        assert_eq!(app.session.active().unwrap().history.entries(), rows);
        assert!(app.paused_playbacks.paused.get(&token).unwrap().history.is_some(), "denial must leave the bound continuation untouched");
        assert!(app.session.execute("actions.setHistory", json!({"singleState":false})).is_err());
        let fields = serde_json::Map::from_iter([(RESUME.to_string(), json!(token))]);
        abandoned(&mut app, &fields);
        app.session.execute("actions.setHistory", json!({"singleState":false})).unwrap();
    }
}

#[test]
/// A changed authority context cancels both direct and nested tails while keeping the admitted prefix undoable.
fn a_changed_live_gate_cannot_be_replaced_for_the_remaining_action() {
    fn deny_tail(id: &str, _: &Value) -> photocraft_engine::Result<()> {
        if id == "layer.renameLayer" { Err(photocraft_engine::EngineError::Other("live tail denied".into())) } else { Ok(()) }
    }
    for service in [false, true] {
        for nested in [false, true] {
            let (mut app, before, past) = paused_history_spelling(true, Some(deny_wide_blur));
            if nested {
                app.session.actions.list.push(Action::new("Nested tail", vec![("layer.renameLayer".into(), json!({"name":"Forbidden tail"}))]));
                app.session.actions.list[0].steps[2] = ("actions.play".into(), json!({"action":"Nested tail"}));
            }
            spelling_change(&mut app, json!({"action":"change","start":0,"end":3,"word":"Teh","replace":"The"})).unwrap();
            let corrected = app.session.active().unwrap().doc.clone();
            if service {
                app.automation_input = true;
                app.services.automation_authorize = Some(deny_tail);
            } else {
                app.session.authorize = Some(deny_tail);
            }
            crate::type_panels_ui::close_spelling(&mut app);
            assert!(app.ui.status_error && app.ui.status.contains("authorization context changed"), "{}", app.ui.status);
            assert_eq!(app.session.active().unwrap().doc.as_ref(), corrected.as_ref(), "the admitted prefix remains, with no tail edit");
            assert_eq!(app.session.active().unwrap().history.past_len(), past + 1);
            assert!(app.session.undo());
            assert_eq!(app.session.active().unwrap().doc.as_ref(), before.as_ref());
            assert!(app.paused_playbacks.paused.is_empty());
            assert_eq!(app.session.actions.playing, 0);
        }
    }
}

#[test]
/// Distinct captured/live gates fail before applying the dialog or starting a worker, even if the edit is allowed.
fn a_changed_context_is_rejected_before_dialog_or_worker_admission() {
    fn allow_all(_: &str, _: &Value) -> photocraft_engine::Result<()> {
        Ok(())
    }
    for background in [false, true] {
        let mut app = gated_blur_fixture(true, background);
        let dialog = pause_gated_blur(&mut app, Some(deny_wide_blur));
        let before = app.session.active().unwrap().doc.clone();
        let rows = app.session.active().unwrap().history.entries();
        app.session.authorize = Some(allow_all);
        assert!(crate::dialogs::confirm(&mut app, dialog).is_err_and(|e| e.contains("authorization context changed")));
        assert_eq!(app.session.active().unwrap().doc.as_ref(), before.as_ref());
        assert_eq!(app.session.active().unwrap().history.entries(), rows);
        assert!(app.session.job_on(before.id).is_none());
    }
}

#[test]
/// The same gate, a released temporary gate and a newly installed gate permit supported continuation paths.
fn unchanged_and_released_gates_allow_the_action_tail() {
    for scenario in ["released", "same", "new"] {
        let mut app = gated_blur_fixture(true, false);
        app.session.actions.list[0].steps.push(("layer.renameLayer".into(), json!({"name":"Allowed tail"})));
        let dialog = pause_gated_blur(&mut app, (scenario != "new").then_some(deny_wide_blur as Gate));
        match scenario {
            "same" | "new" => app.session.authorize = Some(deny_wide_blur),
            _ => {}
        }
        crate::dialogs::confirm(&mut app, dialog).unwrap();
        let state = app.session.active().unwrap();
        assert_eq!(state.doc.layer(state.active_layer.unwrap()).unwrap().name, "Allowed tail");
    }
}

#[test]
/// Distinct configured policies permit manual confirmation but cannot be combined for automated dispatch.
/// The identical-gate automated control proves the supported composed-policy path still completes and undoes once.
fn configured_gates_require_one_composed_policy_for_automated_resume() {
    fn deny_file_open(id: &str, _: &Value) -> photocraft_engine::Result<()> {
        if id == "file.open" { Err(photocraft_engine::EngineError::Other("file open denied".into())) } else { Ok(()) }
    }
    assert!(!std::ptr::fn_addr_eq(deny_wide_blur as Gate, deny_file_open as Gate));
    for (automated, identical) in [(false, false), (true, false), (true, true)] {
        let mut app = gated_blur_fixture(true, false);
        app.session.actions.list[0].steps.push(("layer.renameLayer".into(), json!({"name":"Allowed tail"})));
        app.session.authorize = Some(deny_wide_blur);
        let service: Gate = if identical { deny_wide_blur } else { deny_file_open };
        app.services.automation_authorize = Some(service);
        // The service is configured before the real local pause, but becomes effective only
        // for the automated OK. Both callbacks allow this edit and its remaining command.
        for gate in [deny_wide_blur as Gate, deny_file_open as Gate] {
            gate("filter.blur.gaussianBlur", &json!({"radius":1})).unwrap();
            gate("layer.renameLayer", &json!({"name":"Allowed tail"})).unwrap();
        }
        let dialog = play(&mut app, &json!({"action":"Gated blur"})).unwrap()["paused"]["dialog"].as_u64().unwrap();
        let before = app.session.active().unwrap().doc.clone();
        let rows = app.session.active().unwrap().history.past_len();
        app.automation_input = automated;
        let result = crate::dialogs::confirm(&mut app, dialog);
        let state = app.session.active().unwrap();
        if automated && !identical {
            assert!(result.is_err_and(|error| error.contains("policies that cannot be combined")));
            assert_eq!(state.doc.as_ref(), before.as_ref());
            assert_eq!(state.history.past_len(), rows);
            assert!(app.session.job_on(before.id).is_none());
        } else {
            result.unwrap();
            assert_eq!(state.doc.layer(state.active_layer.unwrap()).unwrap().name, "Allowed tail");
            assert_eq!(state.history.past_len(), rows + 1);
            assert!(app.session.undo());
            assert_eq!(app.session.active().unwrap().doc.as_ref(), before.as_ref());
        }
        assert!(std::ptr::fn_addr_eq(app.session.authorize.unwrap(), deny_wide_blur as Gate));
        assert!(std::ptr::fn_addr_eq(app.services.automation_authorize.unwrap(), service));
        assert!(app.paused_playbacks.paused.is_empty());
        assert_eq!(app.session.actions.playing, 0);
    }
}

#[test]
/// A policy change after the final spelling step must not discard completed edits when no tail remains to dispatch.
fn a_changed_gate_after_the_last_spelling_step_keeps_completed_edits() {
    let (mut app, before, past) = paused_history_spelling(true, Some(deny_wide_blur));
    app.session.actions.list[0].steps.truncate(2);
    spelling_change(&mut app, json!({"action":"change","start":0,"end":3,"word":"Teh","replace":"The"})).unwrap();
    let corrected = app.session.active().unwrap().doc.clone();
    app.session.authorize = Some(|_, _| Ok(()));
    crate::type_panels_ui::close_spelling(&mut app);
    assert!(!app.ui.status_error, "{}", app.ui.status);
    assert_eq!(app.session.active().unwrap().doc.as_ref(), corrected.as_ref());
    assert_eq!(app.session.active().unwrap().history.past_len(), past + 1);
    assert!(app.session.undo());
    assert_eq!(app.session.active().unwrap().doc.as_ref(), before.as_ref());
}

#[test]
/// Implicit layer-owned adjustments reject a receiver switch; explicit receivers change only their intended surface.
/// An omitted target separately inherits the mask captured when its dialog opens.
fn paused_adjustments_bind_implicit_pixel_and_mask_receivers() {
    for targeting in ["implicit", "null", "explicit"] {
        for mask in [false, true] {
            let mut app = gated_blur_fixture(true, false);
            let a = app.session.active().unwrap().active_layer.unwrap();
            app.run("edit.fill", json!({"color":"#808080"})).unwrap();
            app.run("layer.layerMask.revealAll", json!({})).unwrap();
            app.run("edit.fill", json!({"color":"#808080","target":"mask"})).unwrap();
            let b = photocraft_doc::LayerId(app.run("layer.new.layer", json!({"name":"B"})).unwrap()["layer"].as_u64().unwrap());
            // Native mask eligibility checks the active layer, so keep both receivers eligible.
            // The assertions below still require the explicitly named A mask alone to change.
            app.run("layer.layerMask.revealAll", json!({})).unwrap();
            app.session.select_layer(a).unwrap();
            let mut params = json!({"brightness":40,"target":"pixels"});
            if mask {
                params["target"] = json!("mask");
            }
            if targeting == "explicit" {
                params["layer"] = json!(a.0);
            } else if targeting == "null" {
                params["layer"] = Value::Null;
            }
            app.session.actions.list[0].steps[0] = ("image.adjustments.brightnessContrast".into(), params);
            let dialog = pause_gated_blur(&mut app, None);
            app.session.select_layer(b).unwrap();
            let before = app.session.active().unwrap().doc.clone();
            let result = crate::dialogs::confirm(&mut app, dialog);
            if targeting == "explicit" {
                result.unwrap();
                let after = &app.session.active().unwrap().doc;
                assert_eq!(after.layer(b).unwrap(), before.layer(b).unwrap());
                if mask {
                    assert_ne!(after.layer(a).unwrap().mask, before.layer(a).unwrap().mask);
                    assert_eq!(after.layer(a).unwrap().surface(), before.layer(a).unwrap().surface());
                } else {
                    assert_ne!(after.layer(a).unwrap().surface(), before.layer(a).unwrap().surface());
                    assert_eq!(after.layer(a).unwrap().mask, before.layer(a).unwrap().mask);
                }
            } else {
                assert!(result.is_err_and(|e| e.contains("receiver changed")));
                assert_eq!(app.session.active().unwrap().doc.as_ref(), before.as_ref());
            }
        }
    }

    // Batch 16 review: a dialog paused on a document target (an alpha channel) and switched to
    // the layer's pixels checks the layer that was active at the pause.
    {
        let mut app = gated_blur_fixture(true, false);
        let a = app.session.active().unwrap().active_layer.unwrap();
        let b = photocraft_doc::LayerId(app.run("layer.new.layer", json!({"name":"B"})).unwrap()["layer"].as_u64().unwrap());
        app.session.select_layer(a).unwrap();
        app.session.actions.list[0].steps[0] = ("image.adjustments.brightnessContrast".into(), json!({"brightness":40,"target":{"channel":0}}));
        let dialog = pause_gated_blur(&mut app, None);
        app.ui.dialog_mut(dialog).unwrap().fields.insert("target".into(), json!("pixels"));
        app.session.select_layer(b).unwrap();
        let before = app.session.active().unwrap().doc.clone();
        let result = crate::dialogs::confirm(&mut app, dialog);
        assert!(result.is_err_and(|e| e.contains("receiver changed")));
        assert_eq!(app.session.active().unwrap().doc.as_ref(), before.as_ref());
    }

    // An omitted target inherits the mask selected when the paused dialog opens. This is
    // distinct from native playback without a dialog, which resolves the current channel.
    let mut app = gated_blur_fixture(true, false);
    let a = app.session.active().unwrap().active_layer.unwrap();
    app.run("layer.layerMask.revealAll", json!({})).unwrap();
    app.run("edit.fill", json!({"color":"#808080","target":"mask"})).unwrap();
    assert!(app.layer_mask_targeted());
    app.session.actions.list[0].steps[0] = ("image.adjustments.brightnessContrast".into(), json!({"brightness":40}));
    let before = app.session.active().unwrap().doc.clone();
    let rows = app.session.active().unwrap().history.past_len();
    let dialog = pause_gated_blur(&mut app, None);
    assert_eq!(app.ui.dialog_mut(dialog).unwrap().fields["target"], "mask");
    crate::dialogs::confirm(&mut app, dialog).unwrap();
    let state = app.session.active().unwrap();
    assert_ne!(state.doc.layer(a).unwrap().mask, before.layer(a).unwrap().mask);
    assert_eq!(state.doc.layer(a).unwrap().surface(), before.layer(a).unwrap().surface());
    assert_eq!(state.history.past_len(), rows + 1);
    assert!(app.session.undo());
    assert_eq!(app.session.active().unwrap().doc.as_ref(), before.as_ref());
}

#[test]
/// Document-owned alpha channels remain valid targets when the active layer changes during the pause.
fn a_document_channel_adjustment_survives_an_active_layer_change() {
    let mut app = gated_blur_fixture(true, false);
    let a = app.session.active().unwrap().active_layer.unwrap();
    let b = photocraft_doc::LayerId(app.run("layer.new.layer", json!({"name":"B"})).unwrap()["layer"].as_u64().unwrap());
    app.run("channel.new", json!({"name":"Alpha","fill":"black"})).unwrap();
    app.run("edit.fill", json!({"color":"#808080","target":{"channel":0}})).unwrap();
    app.session.select_layer(a).unwrap();
    app.session.actions.list[0].steps[0] = ("image.adjustments.brightnessContrast".into(), json!({"brightness":40,"target":{"channel":0}}));
    let dialog = pause_gated_blur(&mut app, None);
    app.session.select_layer(b).unwrap();
    let before = app.session.active().unwrap().doc.clone();
    crate::dialogs::confirm(&mut app, dialog).unwrap();
    let after = &app.session.active().unwrap().doc;
    assert_eq!(after.layers, before.layers);
    assert_ne!(after.channels, before.channels);
}

#[test]
/// A public Session layer switch invalidates an implicit receiver while explicit-layer playback remains usable.
fn an_implicit_paused_filter_refuses_a_public_api_receiver_change() {
    for single in [false, true] {
        for explicit in [false, true] {
            let mut app = gated_blur_fixture(single, false);
            let a = app.session.active().unwrap().active_layer.unwrap();
            let b = photocraft_doc::LayerId(app.run("layer.new.layer", json!({"name": "B"})).unwrap()["layer"].as_u64().unwrap());
            app.run("select.rect", json!({"x": 0, "y": 0, "width": 8, "height": 16})).unwrap();
            app.run("edit.fill", json!({"color": "#ff0000"})).unwrap();
            app.run("select.deselect", json!({})).unwrap();
            app.session.select_layer(a).unwrap();
            if explicit {
                app.session.actions.list[0].steps[0].1["layer"] = json!(a.0);
            }
            let dialog = pause_gated_blur(&mut app, None);
            // Supported host/session API, not a modal-blocked native menu/control command.
            app.session.select_layer(b).unwrap();
            let before = app.session.active().unwrap().doc.clone();
            let result = crate::dialogs::confirm(&mut app, dialog);
            let after = &app.session.active().unwrap().doc;
            if explicit {
                result.unwrap();
                assert_ne!(after.layer(a).unwrap().surface().unwrap().pixel(7, 8), before.layer(a).unwrap().surface().unwrap().pixel(7, 8));
                assert_eq!(after.layer(b).unwrap().surface().unwrap().pixel(7, 8), before.layer(b).unwrap().surface().unwrap().pixel(7, 8));
            } else {
                assert!(result.is_err_and(|e| e.contains("receiver changed")));
                assert_eq!(after.as_ref(), before.as_ref(), "neither layer may be edited after stale implicit targeting");
            }
            assert!(app.paused_playbacks.paused.is_empty());
            assert_eq!(app.session.actions.playing, 0);
        }
    }
}

#[test]
/// Changes to other selected layers do not invalidate the unchanged active receiver captured by a filter dialog.
fn an_implicit_filter_keeps_its_receiver_when_only_other_selected_layers_change() {
    let mut app = gated_blur_fixture(true, false);
    let a = app.session.active().unwrap().active_layer.unwrap();
    let b = photocraft_doc::LayerId(app.run("layer.new.layer", json!({"name": "Other"})).unwrap()["layer"].as_u64().unwrap());
    app.session.select_layer(a).unwrap();
    let dialog = pause_gated_blur(&mut app, None);
    app.session.execute("layer.select", json!({"layer": b.0, "mode": "add"})).unwrap();
    app.session.execute("layer.select", json!({"layer": a.0, "mode": "add"})).unwrap();
    assert_eq!(app.session.active().unwrap().selected_layers().len(), 2);
    let before = app.session.active().unwrap().doc.clone();
    crate::dialogs::confirm(&mut app, dialog).unwrap();
    let after = &app.session.active().unwrap().doc;
    assert_ne!(after.layer(a).unwrap().surface().unwrap().pixel(7, 8), before.layer(a).unwrap().surface().unwrap().pixel(7, 8));
    assert_eq!(after.layer(b).unwrap().surface().unwrap().content_bounds(), before.layer(b).unwrap().surface().unwrap().content_bounds());
}

fn deny_displace_file(id: &str, params: &Value) -> photocraft_engine::Result<()> {
    if id == "filter.distort.displace" && params.get("mapPath").is_some() {
        Err(photocraft_engine::EngineError::Other("chosen displacement map denied".into()))
    } else {
        Ok(())
    }
}

#[test]
/// Authorize a deferred displacement-map path before file access, worker admission or continuation history changes.
fn a_deferred_displace_map_is_authorized_before_reading_or_resuming_history() {
    let mut app = recording_with_displace_child();
    app.background_jobs = true;
    let (show, _) = crate::file_dialog::fake(vec![Some(crate::file_dialog::FileDialogAnswer::Paths(vec!["denied-map.png".into()]))]);
    app.services.file_dialog = Some(show);
    app.session.authorize = Some(deny_displace_file);
    let dialog = pause_child(&mut app);
    app.session.authorize = None;
    let before = app.session.active().unwrap().doc.clone();
    let count = app.session.active().unwrap().history.past_len();
    crate::dialogs::confirm(&mut app, dialog).unwrap();
    assert!(app.file_dialog_open());
    app.poll_file_dialog(&egui::Context::default(), None);
    assert!(app.ui.status.contains("chosen displacement map denied"), "{}", app.ui.status);
    assert_eq!(app.session.active().unwrap().doc.as_ref(), before.as_ref());
    assert_eq!(app.session.active().unwrap().history.past_len(), count);
    assert!(app.session.authorize.is_none());
    assert!(app.session.job_on(before.id).is_none());
    assert_displace_call_finished(&mut app, 0);
}

#[test]
fn a_step_the_gate_denies_opens_no_dialog() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 16, "height": 16})).unwrap();
    let mut a = Action::new("Blur", vec![("filter.blur.gaussianBlur".into(), json!({"radius": 1}))]);
    a.dialogs.insert(0);
    app.session.actions.list.push(a);
    app.session.authorize = Some(deny_blur);
    let v = play(&mut app, &json!({"action": "Blur"})).unwrap();
    app.session.authorize = None;
    assert!(v["failed"]["error"].as_str().is_some_and(|e| e.contains("blur denied")), "{v}");
    assert!(app.ui.dialogs.is_empty(), "no step dialog opened");
    assert!(app.paused_playbacks.paused.is_empty());
}

fn deny_select_all(id: &str, _: &Value) -> photocraft_engine::Result<()> {
    if id == "select.all" { Err(photocraft_engine::EngineError::Other("select.all denied".into())) } else { Ok(()) }
}

/// An action an automation client plays (`control::handle` installs the gate, then removes
/// it) pauses at a step dialog; confirming the dialog later, here with synthetic Enter or
/// OK, runs the remaining steps under the same gate.
#[test]
fn a_paused_playback_keeps_its_automation_gate_when_the_dialog_is_confirmed() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 16, "height": 16})).unwrap();
    let mut a = Action::new("Seeded", vec![("filter.blur.gaussianBlur".into(), json!({"radius": 1})), ("select.all".into(), json!({}))]);
    a.dialogs.insert(0);
    app.session.actions.list.push(a);
    app.session.authorize = Some(deny_select_all);
    let v = play(&mut app, &json!({"action": "Seeded"})).unwrap();
    app.session.authorize = None;
    let id = v["paused"]["dialog"].as_u64().unwrap();
    app.automation_input = true;
    let _ = crate::dialogs::confirm(&mut app, id);
    app.automation_input = false;
    assert!(app.session.active().unwrap().doc.selection.is_none(), "the denied step did not run");
    assert!(app.ui.status.contains("select.all denied"), "{}", app.ui.status);
}

/// An action automation wrote, bound to a key, plays under the desktop's automation policy
/// when the user presses the key: its file step is refused, not run with the user's access.
#[test]
fn a_users_key_plays_an_action_automation_wrote_under_its_gate() {
    /// The automation policy this test stands in for: every `file.` command is refused.
    fn deny_file(id: &str, _: &Value) -> photocraft_engine::Result<()> {
        if id.starts_with("file.") { Err(photocraft_engine::EngineError::Other(format!("automation command `{id}` is disabled"))) } else { Ok(()) }
    }
    let services = crate::Services { automation_authorize: Some(deny_file), ..Default::default() };
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
    let mut planted = Action::new("Backup", vec![("file.openAs".into(), json!({"path": "/outside/secret.png"}))]);
    planted.automation = true;
    app.session.actions.list.push(planted);
    assign_shortcut(&mut app, "Backup", "F9").unwrap();
    crate::shortcut_dispatch::dispatch(&mut app, &egui::Context::default(), &shortcut_id("Backup"));
    assert!(app.session.documents().is_empty());
    assert!(app.ui.status.contains("disabled"), "the refusal is reported: {}", app.ui.status);
    assert!(app.session.authorize.is_none(), "the kept gate is removed after playback");
}

/// CodeRabbit on #9783: the kept gate of an action automation wrote stays with that action. A set
/// the user plays pauses at the marked action's step dialog; after OK the marked action's tail
/// still plays under the gate (its refused step is refused), and once that tail passes, the
/// user's own action after it in the set keeps the user's access.
#[test]
fn a_kept_gate_does_not_carry_into_the_rest_of_a_set_after_a_pause() {
    use egui_kittest::kittest::Queryable;
    /// The automation policy this test stands in for: it refuses only layers named `a2` and `b`.
    fn deny_a2_and_b(id: &str, p: &Value) -> photocraft_engine::Result<()> {
        match p.get("name").and_then(Value::as_str) {
            Some(name @ ("a2" | "b")) if id == "layer.new.layer" => Err(photocraft_engine::EngineError::Other(format!("layer {name} denied"))),
            _ => Ok(()),
        }
    }
    let services = crate::Services { automation_authorize: Some(deny_a2_and_b), ..Default::default() };
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
    app.run("file.new", json!({"width": 8, "height": 8})).unwrap();
    let mut a = Action::new(
        "A",
        vec![
            ("layer.new.layer".into(), json!({"name": "a"})),
            ("filter.blur.gaussianBlur".into(), json!({"radius": 1})),
            ("layer.new.layer".into(), json!({"name": "a2"})),
        ],
    );
    a.set = Some("S".into());
    a.dialogs.insert(1);
    a.automation = true;
    let mut b = Action::new("B", vec![("layer.new.layer".into(), json!({"name": "b"}))]);
    b.set = Some("S".into());
    app.session.actions.list.extend([a, b]);
    let mut h = egui_kittest::Harness::builder().with_size(vec2(360.0, 400.0)).build_ui_state(|ui, app| panel(app, ui), app);
    h.run_steps(2);
    h.get_by_label("S").click();
    h.run_steps(2);

    // The marked action's tail after OK: its `a2` step meets the kept gate, and the set stops.
    h.get_by_label("Play selection").click();
    h.run_steps(2);
    assert_eq!(layer_names(h.state()), ["a"]);
    let dialog = h.state().ui.dialogs.last().map(|d| d.id).expect("the blur's dialog");
    let _confirmed = crate::dialogs::confirm(h.state_mut(), dialog);
    h.run_steps(2);
    assert_eq!(layer_names(h.state()), ["a"], "a2 was refused after OK: {}", h.state().ui.status);
    assert!(h.state().ui.status.contains("a2 denied"), "the refusal is reported: {}", h.state().ui.status);
    assert!(h.state().session.authorize.is_none(), "no gate is left installed");

    // With that step off, the tail passes, and the user's own action after it plays with the
    // user's access.
    h.state_mut().session.actions.list[0].disabled.insert(2);
    h.get_by_label("Play selection").click();
    h.run_steps(2);
    let dialog = h.state().ui.dialogs.last().map(|d| d.id).expect("the blur's dialog");
    crate::dialogs::confirm(h.state_mut(), dialog).unwrap();
    h.run_steps(2);
    let names = layer_names(h.state());
    assert!(names.iter().any(|n| n == "b"), "B played with the user's access: {names:?} {}", h.state().ui.status);
    assert!(!names.iter().any(|n| n == "a2"), "{names:?}");
    assert!(h.state().session.authorize.is_none(), "no gate is left installed");
}

/// The dialog names only its pause: changing its fields can't resume another action.
#[test]
fn a_step_dialog_resumes_only_the_playback_that_paused_it() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 16, "height": 16})).unwrap();
    let mut a = Action::new("Blur", vec![("filter.blur.gaussianBlur".into(), json!({"radius": 1}))]);
    a.dialogs.insert(0);
    app.session.actions.list.push(a);
    app.session.actions.list.push(Action::new("Seeded", vec![("select.all".into(), json!({}))]));
    let v = play(&mut app, &json!({"action": "Blur"})).unwrap();
    let id = v["paused"]["dialog"].as_u64().unwrap();
    app.ui.dialog_mut(id).unwrap().fields.insert(RESUME.into(), json!({"action": "Seeded", "from": 0}));
    let r = crate::dialogs::confirm(&mut app, id);
    assert!(app.session.active().unwrap().doc.selection.is_none(), "the other action did not play");
    assert!(r.is_err_and(|e| e.contains("paused action")));
}

#[test]
fn button_mode_plays_the_clicked_action_and_hides_its_steps() {
    use egui_kittest::{Harness, kittest::Queryable};
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 16, "height": 16})).unwrap();
    app.session.actions.list.push(Action::new("Print", vec![("layer.new.layer".into(), json!({}))]));
    app.session.actions.list[0].color = "blue".into();
    app.ui.actions.expanded = vec![true];
    app.ui.actions.button_mode = true;
    let mut h = Harness::builder().with_size(vec2(420.0, 280.0)).build_ui_state(|ui, app| panel(app, ui), app);
    h.run_steps(3);
    assert_eq!(h.query_all_by_role(egui::accesskit::Role::CheckBox).count(), 0, "button mode hides the step list");
    let before = h.state().session.active().unwrap().doc.layers.len();
    h.get_by_label("Print").click();
    h.run_steps(3);
    assert_eq!(h.state().session.active().unwrap().doc.layers.len(), before + 1, "a click plays the action");
    h.get_by_label("Button Mode On").click();
    h.run_steps(2);
    assert!(!h.state().ui.actions.button_mode);
    assert!(h.query_by_label("Button Mode").is_some());
    assert_eq!(h.query_all_by_role(egui::accesskit::Role::CheckBox).count(), 1, "the step list returns when Button Mode is turned off");
}

/// An imported Close with Save: Yes plays as Save then Close. A Save Document script event
/// that closes the saved document stops playback, so the Close never reaches another dirty
/// document (#7367).
#[test]
fn save_hook_closing_the_target_stops_an_imported_close_with_save() {
    use craft_psd::atn::{self, AtnEvent, AtnItem};
    use craft_psd::descriptor::{Descriptor, Id, Value as D};
    use std::{cell::RefCell, rc::Rc};
    // A one-step .atn: Close (`Cls `) with Save: Yes.
    let seed = Action::new("Seed", vec![("layer.new.layer".into(), json!({}))]);
    let (bytes, _) = photocraft_engine::atn_cmds::export("Set", &[&seed], &[], false).unwrap();
    let mut f = atn::parse(&bytes).unwrap();
    f.actions[0].items = vec![AtnItem {
        event: AtnEvent::Code(*b"Cls "),
        dictionary: "Close".into(),
        descriptor: Some(Descriptor::new("null").with("Svng", D::Enumerated { type_id: Id::new("YsN "), value: Id::new("Ys  ") })),
        ..Default::default()
    }];
    let imported = photocraft_engine::atn_cmds::import(&atn::write(&f).unwrap()).unwrap();
    let close_with_save = imported.actions[0].clone();
    assert_eq!(close_with_save.steps, [("file.save".into(), json!({})), ("file.close".into(), json!({}))]);
    for hook in ["none", "activate", "close"] {
        let writes = Rc::new(RefCell::new(Vec::<String>::new()));
        let w = writes.clone();
        let services = crate::Services {
            export: Some(Box::new(|doc, _, _| Ok((doc.name.clone().into_bytes(), Vec::new())))),
            write: Some(Box::new(move |path, _| {
                w.borrow_mut().push(path.to_string());
                Ok(())
            })),
            ..Default::default()
        };
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
        // B is dirty with an unsaved KeepB layer; A is active and has its own file.
        for name in ["B", "A"] {
            app.run("file.new", json!({"name": name, "width": 2, "height": 2})).unwrap();
        }
        assert!(app.session.set_active(0));
        app.run("layer.new.layer", json!({"name": "KeepB"})).unwrap();
        assert!(app.session.set_active(1));
        app.session.active_mut().unwrap().path = Some("A.psd".into());
        app.session.actions.list.push(close_with_save.clone());
        let steps = match hook {
            "activate" => json!([["document.activate", {"document": 0}]]),
            "close" => json!([["file.close", {}]]),
            _ => Value::Null,
        };
        if !steps.is_null() {
            app.run("file.scripts.scriptEventsManager", json!({"add": {"event": "saveDocument", "steps": steps}})).unwrap();
        }
        let r = app.run("actions.play", json!({"action": 0})).unwrap();
        assert_eq!(*writes.borrow(), ["A.psd"], "hook={hook}: A is saved once");
        let names: Vec<Vec<String>> = app.session.documents().iter().map(|d| d.doc.layers.iter().map(|l| l.name.clone()).collect()).collect();
        assert_eq!(app.session.documents().len(), 1, "hook={hook}: only A closes; result={r}");
        assert_eq!(app.session.active().unwrap().doc.name, "B", "hook={hook}");
        assert_eq!(names, [["Background", "KeepB"]], "hook={hook}: B stays open with its unsaved layer");
        if hook == "close" {
            assert_eq!(r["failed"]["step"], 0, "{r}");
            assert_eq!(r["failed"]["id"], "file.save", "{r}");
            assert!(r["failed"]["error"].as_str().unwrap().contains("closed during the save"), "{r}");
        } else {
            assert!(r.get("failed").is_none(), "hook={hook}: {r}");
        }
        assert_eq!(app.session.actions.playing, 0);
        assert!(app.session.actions.playback_stack.is_empty());
    }
}

#[test]
/// An unsupported step in a set that stays loaded is counted in the multi-file summary, unlike
/// one in a set a later file replaced (CodeRabbit on #10776).
fn bulk_load_summary_counts_unsupported_steps_of_surviving_sets() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let mut a = craft_psd::atn::parse(&action_set_bytes("Set A", "A")).unwrap();
    a.actions[0].items[0].event = craft_psd::atn::AtnEvent::Code(*b"Xxxx");
    let files = vec![("a.atn".into(), craft_psd::atn::write(&a).unwrap()), ("b.atn".into(), action_set_bytes("Set B", "B"))];
    let status = bulk_load_status(&mut app, files);
    assert!(status.contains("1 step has no PhotoCraft equivalent yet"), "{status}");
    assert!(!app.ui.status_error, "{status}");
    // Both sets stay loaded: the unsupported step is counted for one that is in the panel.
    let names: Vec<&str> = app.session.actions.list.iter().map(|a| a.name.as_str()).collect();
    assert_eq!(names, ["A", "B"]);
    assert!(app.session.actions.list[0].steps.iter().any(|(id, _)| id == "actions.photoshopStep"));
}
