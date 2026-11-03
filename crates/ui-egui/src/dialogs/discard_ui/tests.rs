use super::*;

fn app_with_docs(n: usize) -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    for _ in 0..n {
        app.run("file.new", json!({"width": 8, "height": 8})).unwrap();
    }
    app.sync_views();
    app
}

fn make_dirty(app: &mut PhotocraftApp, doc: usize) {
    app.session.set_active(doc);
    app.run("layer.new.layer", json!({})).unwrap();
}

fn doc_id(app: &PhotocraftApp, i: usize) -> DocId {
    app.session.documents()[i].doc.id
}

#[test]
fn a_save_document_script_that_edits_the_document_keeps_the_prompt_up() {
    use photocraft_engine::automate_cmds::ScriptBinding;
    let mut app = app_with_docs(1);
    let ctx = egui::Context::default();
    make_dirty(&mut app, 0);
    app.session.active_mut().unwrap().path = Some("/pics/kept.psd".into());
    app.services.export = Some(Box::new(|_: &photocraft_doc::Document, _: &str, _: &crate::ExportSettings| Ok((b"out".to_vec(), Vec::new()))));
    app.services.write = Some(Box::new(|_: &str, _: &[u8]| Ok(())));
    app.session.edit_prefs(|prefs| {
        prefs.script_events.enabled = true;
        prefs.script_events.bindings = vec![ScriptBinding {
            event: "saveDocument".into(),
            script: None,
            steps: Some(json!([["layer.new.layer", {"name": "From the script"}]])),
            name: "Binding".into(),
        }];
    });
    let doc = doc_id(&app, 0);
    assert!(!save(&mut app, &ctx, doc), "the script's layer was never written");
    assert!(app.session.documents()[0].is_dirty());
    // Without the script the same save releases the prompt.
    app.session.edit_prefs(|prefs| prefs.script_events.enabled = false);
    assert!(save(&mut app, &ctx, doc));
}

#[test]
fn clean_documents_close_without_asking() {
    let mut app = app_with_docs(2);
    let ctx = egui::Context::default();
    crate::menus::invoke(&mut app, &ctx, "file.close", json!({})).unwrap();
    assert!(app.discard.is_none());
    assert_eq!(app.session.documents().len(), 1);
}

#[test]
fn closing_a_dirty_document_waits_for_an_answer() {
    let mut app = app_with_docs(2);
    let ctx = egui::Context::default();
    make_dirty(&mut app, 1);
    crate::menus::invoke(&mut app, &ctx, "file.close", json!({"document": 1})).unwrap();
    assert_eq!(app.session.documents().len(), 2, "nothing closes before the user answers");
    assert!(app.discard.is_some());
    advance(&mut app, &ctx);
    assert!(app.discard.is_none());
    assert_eq!(app.session.documents().len(), 1);
}

#[test]
fn only_documents_that_would_be_discarded_are_asked_about() {
    let mut app = app_with_docs(3);
    make_dirty(&mut app, 0);
    make_dirty(&mut app, 2);
    let ids = [doc_id(&app, 0), doc_id(&app, 2)];
    assert_eq!(discarded(&app, "file.closeOthers", &json!({"document": 0})), (Some(ids[0]), vec![ids[1]]));
    assert_eq!(discarded(&app, "file.closeAll", &json!({})).1, ids);
    assert_eq!(discarded(&app, EXIT, &json!({})).1, ids);
    assert!(discarded(&app, "view.zoomIn", &json!({})).1.is_empty());
}

#[test]
fn close_all_asks_per_dirty_document_then_runs() {
    let mut app = app_with_docs(3);
    let ctx = egui::Context::default();
    make_dirty(&mut app, 0);
    make_dirty(&mut app, 2);
    crate::menus::invoke(&mut app, &ctx, "file.closeAll", json!({})).unwrap();
    advance(&mut app, &ctx);
    assert_eq!(app.session.documents().len(), 2, "the answered document closes; still waiting on the second");
    advance(&mut app, &ctx);
    assert!(app.session.documents().is_empty());
}

/// A document that became dirty while the prompt was up is asked about before the final close (#4954).
#[test]
fn a_document_dirtied_during_the_prompt_is_asked_about_before_the_final_close() {
    for command in ["file.closeAll", EXIT] {
        let mut app = app_with_docs(2);
        let ctx = egui::Context::default();
        make_dirty(&mut app, 0);
        let late = doc_id(&app, 1);
        crate::menus::invoke(&mut app, &ctx, command, json!({})).unwrap();
        make_dirty(&mut app, 1);
        advance(&mut app, &ctx);
        assert_eq!(app.discard.as_ref().map(|p| p.docs.clone()), Some(vec![late]), "{command}: asks about the newly dirty document");
        assert!(!app.allow_close, "{command}");
        assert_eq!(app.session.documents().len(), 1, "{command}: it is not discarded unasked");
        advance(&mut app, &ctx);
        assert!(app.discard.is_none(), "{command}");
    }
    // Close Others asks about the others too, but still not about the document it keeps.
    let mut app = app_with_docs(3);
    let ctx = egui::Context::default();
    make_dirty(&mut app, 0);
    let (late, kept) = (doc_id(&app, 1), doc_id(&app, 2));
    crate::menus::invoke(&mut app, &ctx, "file.closeOthers", json!({"document": 2})).unwrap();
    make_dirty(&mut app, 1);
    make_dirty(&mut app, 2);
    advance(&mut app, &ctx);
    assert_eq!(app.discard.as_ref().map(|p| p.docs.clone()), Some(vec![late]), "file.closeOthers: asks about the newly dirty other document");
    advance(&mut app, &ctx);
    assert!(app.discard.is_none());
    assert_eq!(app.session.documents().iter().map(|d| d.doc.id).collect::<Vec<_>>(), [kept]);
}

#[test]
fn close_others_closes_each_answered_document_and_keeps_its_target() {
    let mut app = app_with_docs(3);
    let ctx = egui::Context::default();
    (0..3).for_each(|i| make_dirty(&mut app, i));
    let kept = doc_id(&app, 1);
    crate::menus::invoke(&mut app, &ctx, "file.closeOthers", json!({"document": 1})).unwrap();
    assert_eq!(app.session.active_index(), Some(0), "the first one asked about is shown");
    advance(&mut app, &ctx);
    assert_eq!(app.session.documents().len(), 2);
    assert_eq!(app.session.active().map(|d| d.doc.id), Some(doc_id(&app, 1)), "the next one asked about is shown");
    advance(&mut app, &ctx);
    assert!(app.discard.is_none());
    assert_eq!(app.session.documents().iter().map(|d| d.doc.id).collect::<Vec<_>>(), [kept]);
}

#[test]
fn the_parked_close_still_hits_its_document_after_tabs_move() {
    let mut app = app_with_docs(3);
    let ctx = egui::Context::default();
    make_dirty(&mut app, 2);
    let doomed = doc_id(&app, 2);
    crate::menus::invoke(&mut app, &ctx, "file.close", json!({"document": 2})).unwrap();
    // Another document closes while the prompt is up, shifting the tab to index 1.
    app.run("file.close", json!({"document": 0})).unwrap();
    advance(&mut app, &ctx);
    assert_eq!(app.session.documents().len(), 1);
    assert!(index_of(&app, doomed).is_none());
}

#[test]
fn quitting_replaces_a_pending_close_but_other_requests_are_refused() {
    let mut app = app_with_docs(2);
    make_dirty(&mut app, 0);
    make_dirty(&mut app, 1);
    assert!(intercept(&mut app, "file.close", &json!({"document": 0})));
    assert!(intercept(&mut app, "file.closeAll", &Value::Null));
    assert_eq!(app.discard.as_ref().map(|p| (p.id.as_str(), p.docs.len())), Some(("file.close", 1)));
    assert!(app.ui.status_error);
    assert!(intercept(&mut app, EXIT, &Value::Null));
    assert_eq!(app.discard.as_ref().map(|p| (p.id.as_str(), p.docs.len())), Some((EXIT, 2)));
    // Pressing the window's X again must not restart the questions.
    app.discard.as_mut().unwrap().docs.remove(0);
    assert!(intercept(&mut app, EXIT, &Value::Null));
    assert_eq!(app.discard.as_ref().unwrap().docs.len(), 1);
}

#[test]
fn exit_is_held_back_until_confirmed() {
    let mut app = app_with_docs(1);
    let ctx = egui::Context::default();
    make_dirty(&mut app, 0);
    assert!(intercept(&mut app, EXIT, &Value::Null));
    assert!(!app.allow_close);
    advance(&mut app, &ctx);
    assert!(app.allow_close);
}

#[test]
fn mnemonic_labels_bracket_the_key() {
    assert_eq!(button_label(false, "Don't Save", Some(Key::D)), "(D)on't Save");
    assert_eq!(button_label(false, "Guardar", Some(Key::S)), "Guardar (S)");
    assert_eq!(button_label(false, "保存", Some(Key::S)), "保存 (S)");
}

type Prompted = egui_kittest::Harness<'static, PhotocraftApp>;

/// The unsaved-changes prompt for `command` over `app`'s dirty documents, as `os` draws it (with
/// the file dialogs shown and answered at the end of each frame, as the app does).
fn prompt_for(app: PhotocraftApp, command: &str, os: egui::os::OperatingSystem) -> Prompted {
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(800.0, 600.0)).build_ui_state(
        |ui, app| {
            show(app, ui.ctx());
            app.poll_file_dialog(ui.ctx(), None);
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    h.ctx.set_os(os);
    assert!(intercept(h.state_mut(), command, &Value::Null));
    h.run_steps(2);
    h
}

/// The unsaved-changes prompt for Close All over two dirty documents, as `os` draws it.
fn prompt_on(os: egui::os::OperatingSystem) -> Prompted {
    let mut app = app_with_docs(2);
    make_dirty(&mut app, 0);
    make_dirty(&mut app, 1);
    prompt_for(app, "file.closeAll", os)
}

/// The buttons named `labels`, sorted by where they are drawn, left to right.
fn drawn_order<'a>(h: &Prompted, labels: [&'a str; 3]) -> Vec<&'a str> {
    use egui_kittest::kittest::Queryable;
    let mut v = labels.map(|l| (h.get_by_label(l).rect().left(), l)).to_vec();
    v.sort_by(|a, b| a.0.total_cmp(&b.0));
    v.into_iter().map(|(_, l)| l).collect()
}

/// Tab walks `labels` left to right from the focused default answer, `labels[default]`, and
/// wraps; Shift+Tab steps back.
fn tab_walks(h: &mut Prompted, labels: [&str; 3], default: usize) {
    assert_eq!(focused(h, &labels), Some(labels[default]));
    for step in 1..=3 {
        h.key_press(Key::Tab);
        h.run_steps(2);
        assert_eq!(focused(h, &labels), Some(labels[(default + step) % 3]));
    }
    h.key_press_modifiers(egui::Modifiers::SHIFT, Key::Tab);
    h.run_steps(2);
    assert_eq!(focused(h, &labels), Some(labels[(default + 2) % 3]));
}

fn docs_left(h: &Prompted) -> Option<usize> {
    h.state().discard.as_ref().map(|p| p.docs.len())
}

/// Which of `labels` has keyboard focus.
fn focused<'a>(h: &Prompted, labels: &[&'a str]) -> Option<&'a str> {
    use egui_kittest::kittest::Queryable;
    labels.iter().copied().find(|l| h.get_by_label(l).is_focused())
}

/// The default answer has keyboard focus as each question comes up, so the arrow keys move
/// between the answers and Enter presses the one they reached, as in Photoshop (#6441).
#[test]
fn each_question_comes_up_with_the_default_answer_focused() {
    let mut h = prompt_on(egui::os::OperatingSystem::Windows);
    let labels = ["(Y)es", "(N)o", "(C)ancel"];
    assert_eq!(focused(&h, &labels), Some("(Y)es"));
    h.key_press(Key::ArrowRight);
    h.run_steps(2);
    assert_eq!(focused(&h, &labels), Some("(N)o"));
    h.key_press(Key::Enter);
    h.run_steps(2);
    assert_eq!(docs_left(&h), Some(1), "Enter pressed No");
    assert_eq!(focused(&h, &labels), Some("(Y)es"), "the next question starts on the default again");

    let mut h = prompt_on(egui::os::OperatingSystem::Mac);
    let labels = ["Don't Save", "Cancel", "Save"];
    assert_eq!(focused(&h, &labels), Some("Save"));
    h.key_press(Key::ArrowLeft);
    h.run_steps(2);
    assert_eq!(focused(&h, &labels), Some("Cancel"));
}

#[test]
fn windows_and_linux_ask_yes_no_cancel_with_the_default_first() {
    for os in [egui::os::OperatingSystem::Windows, egui::os::OperatingSystem::Nix] {
        let mut h = prompt_on(os);
        let labels = ["(Y)es", "(N)o", "(C)ancel"];
        assert_eq!(drawn_order(&h, labels), labels, "{os:?}");
        tab_walks(&mut h, labels, 0);
        h.key_press(Key::N);
        h.run_steps(2);
        assert_eq!(docs_left(&h), Some(1), "N answered the first document");
        h.key_press(Key::C);
        h.run_steps(2);
        assert!(h.state().discard.is_none());
        assert_eq!(h.state().session.documents().len(), 1, "Cancel keeps the unanswered document");
    }
}

/// Whether the prompt is asking about the document called `name`.
fn asks_about(h: &Prompted, name: &str) -> bool {
    use egui_kittest::kittest::Queryable;
    h.query_by_label_contains(&format!("“{name}”")).is_some()
}

/// No on one of several documents closes it and asks about the next one (#2547).
#[test]
fn answering_no_closes_that_document_and_asks_about_the_next() {
    for command in [EXIT, "file.closeAll"] {
        let mut app = app_with_docs(2);
        make_dirty(&mut app, 0);
        make_dirty(&mut app, 1);
        let names: Vec<String> = app.session.documents().iter().map(|d| d.doc.name.clone()).collect();
        let second = doc_id(&app, 1);
        let first = doc_id(&app, 0);
        assert_eq!(app.session.active_index(), Some(1), "the last new document is active");
        let mut h = prompt_for(app, command, egui::os::OperatingSystem::Windows);
        assert!(asks_about(&h, &names[0]), "{command}");
        assert_eq!(h.state().session.active().map(|d| d.doc.id), Some(first), "{command}: the first tab asked about is shown");
        h.key_press(Key::N);
        h.run_steps(2);
        assert_eq!(h.state().session.documents().len(), 1, "{command}: No closed the first document");
        assert_eq!(doc_id(h.state(), 0), second);
        assert_eq!(h.state().session.active_index(), Some(0), "{command}: the tab asked about is shown");
        assert!(asks_about(&h, &names[1]), "{command}: then about the second");
        assert!(!h.state().allow_close);
        h.key_press(Key::N);
        h.run_steps(2);
        assert!(h.state().discard.is_none());
        if command == EXIT {
            assert!(h.state().allow_close, "answering the last document lets the app quit");
        } else {
            assert!(h.state().session.documents().is_empty());
        }
    }
}

#[test]
fn cancel_after_no_keeps_the_unanswered_documents_and_the_app_open() {
    let mut app = app_with_docs(2);
    make_dirty(&mut app, 0);
    make_dirty(&mut app, 1);
    let second = doc_id(&app, 1);
    let mut h = prompt_for(app, EXIT, egui::os::OperatingSystem::Windows);
    h.key_press(Key::N);
    h.run_steps(2);
    h.key_press(Key::Escape);
    h.run_steps(2);
    assert!(h.state().discard.is_none());
    assert!(!h.state().allow_close, "Cancel aborts the quit");
    assert_eq!(h.state().session.documents().len(), 1);
    assert_eq!(doc_id(h.state(), 0), second);
    assert!(h.state().session.documents()[0].is_dirty(), "the unanswered document keeps its changes");
}

#[test]
fn macos_hides_mnemonics_but_other_platforms_keep_them() {
    assert_eq!(button_label(true, "Don't Save", Some(Key::D)), tl!("Don't Save"));
    assert_eq!(button_label(true, "Cancel", Some(Key::C)), tl!("Cancel"));
    assert_eq!(button_label(false, "Don't Save", Some(Key::D)), "(D)on't Save");
    assert_eq!(button_label(false, "Cancel", Some(Key::C)), "(C)ancel");
}

/// German "Ja" doesn't start with Y, so every button appends its key rather than mixing
/// "Ja (Y)" with "(N)ein" (#5564).
#[test]
fn one_dialog_shows_every_key_in_the_same_style() {
    let buttons = [("Yes", Some(Key::Y)), ("No", Some(Key::N)), ("Cancel", Some(Key::C))];
    assert_eq!(button_labels(false, &buttons), ["(Y)es", "(N)o", "(C)ancel"]);
    let german = crate::i18n::Lang::from_code("de").unwrap();
    crate::i18n::with_language(german, || {
        let shown: Vec<&str> = buttons.iter().map(|(label, _)| crate::i18n::tr(german, label)).collect();
        assert_eq!(button_labels(true, &buttons), shown);
        let labels = button_labels(false, &buttons);
        let appended: Vec<String> = shown.iter().zip(&buttons).map(|(text, (_, key))| format!("{text} ({})", key.map_or("", |k| k.name()))).collect();
        assert!(labels.iter().all(|label| label.starts_with('(')) || labels == appended, "{labels:?}");
    });
}

#[test]
fn macos_asks_dont_save_cancel_save_with_the_default_last() {
    let mut h = prompt_on(egui::os::OperatingSystem::Mac);
    let labels = ["Don't Save", "Cancel", "Save"];
    assert_eq!(drawn_order(&h, labels), labels);
    tab_walks(&mut h, labels, 2);
    h.key_press(Key::D);
    h.run_steps(2);
    assert_eq!(docs_left(&h), Some(1), "D answered the first document");
    h.key_press(Key::C);
    h.run_steps(2);
    assert!(h.state().discard.is_none());
    assert_eq!(h.state().session.documents().len(), 1, "Cancel keeps the unanswered document");
}

#[test]
fn macos_plain_labels_keep_save_and_escape_shortcuts() {
    for key in [Key::S, Key::Enter] {
        let mut h = prompt_on(egui::os::OperatingSystem::Mac);
        let (show, asked) = crate::file_dialog::fake(vec![None]);
        h.state_mut().services.file_dialog = Some(show);
        h.key_press(key);
        h.run_steps(2);
        assert_eq!(asked.borrow().len(), 1, "{key:?} opens the save dialog");
        assert_eq!(docs_left(&h), Some(2), "cancelling the save dialog keeps the prompt");
        h.key_press(Key::Escape);
        h.run_steps(2);
        assert!(h.state().discard.is_none());
        assert_eq!(h.state().session.documents().len(), 2, "Escape closed nothing");
    }
}

#[test]
fn enter_takes_the_default_answer_or_the_focused_button() {
    let mut h = prompt_on(egui::os::OperatingSystem::Windows);
    // Backing out of the save dialog keeps the prompt.
    let (show, asked) = crate::file_dialog::fake(vec![None]);
    h.state_mut().services.file_dialog = Some(show);
    h.key_press(Key::Enter);
    h.run_steps(2);
    assert_eq!(asked.borrow().len(), 1, "Enter answered Yes, which saves");
    assert_eq!(docs_left(&h), Some(2));
    // Tab from the focused Yes to No: Enter now presses it.
    h.key_press(Key::Tab);
    h.run_steps(2);
    h.key_press(Key::Enter);
    h.run_steps(2);
    assert_eq!(asked.borrow().len(), 1, "no second dialog");
    assert_eq!(docs_left(&h), Some(1), "Enter pressed the focused No");
}

#[test]
fn saving_from_the_prompt_waits_for_the_save_dialog_then_closes() {
    use crate::{FileDialogAnswer, FileDialogReply};
    use egui_kittest::kittest::Queryable;
    use std::{cell::RefCell, rc::Rc};
    let written: Rc<RefCell<Vec<String>>> = Rc::default();
    let (w, waiting): (_, Rc<RefCell<Option<FileDialogReply>>>) = (written.clone(), Rc::default());
    let held = waiting.clone();
    let mut app = app_with_docs(1);
    make_dirty(&mut app, 0);
    app.services.export = Some(Box::new(|_: &photocraft_doc::Document, _: &str, _: &crate::ExportSettings| Ok((b"out".to_vec(), Vec::new()))));
    app.services.write = Some(Box::new(move |p: &str, _: &[u8]| {
        w.borrow_mut().push(p.to_string());
        Ok(())
    }));
    // A dialog the user takes their time with.
    app.services.file_dialog = Some(Box::new(move |_, _, reply| *held.borrow_mut() = Some(reply)));
    let mut h = prompt_for(app, "file.close", egui::os::OperatingSystem::Windows);
    let answer = |h: &mut Prompted, a: Option<FileDialogAnswer>| {
        waiting.borrow_mut().take().expect("the save dialog is open").send(a);
        h.run_steps(2);
    };
    // Cancelling the save dialog brings the prompt back.
    h.key_press(Key::Y);
    h.run_steps(3);
    assert!(h.state().file_dialog_open());
    assert!(h.query_by_label("(Y)es").is_none(), "the prompt waits behind the dialog");
    answer(&mut h, None);
    assert_eq!(docs_left(&h), Some(1));
    assert_eq!(focused(&h, &["(Y)es", "(N)o", "(C)ancel"]), Some("(Y)es"), "the prompt comes back with Yes focused");
    // Saving closes the document.
    h.key_press(Key::Y);
    h.run_steps(3);
    assert_eq!(h.state().session.documents().len(), 1, "still open while the dialog is");
    answer(&mut h, Some(FileDialogAnswer::SaveTo("/pics/kept.psd".into())));
    assert_eq!(*written.borrow(), ["/pics/kept.psd"]);
    assert!(h.state().discard.is_none());
    assert!(h.state().session.documents().is_empty());
}

/// Save on closing an untitled document, answered with `kept` while `kept.psd` exists: the
/// prompt waits for the question whether to replace it. Cancel brings the prompt back with
/// nothing written; Replace saves and closes the document.
#[test]
fn saving_from_the_prompt_waits_for_the_question_whether_to_replace() {
    use crate::{FileDialogAnswer, FileDialogReply};
    use egui_kittest::kittest::Queryable;
    use std::{cell::RefCell, rc::Rc};
    let written: Rc<RefCell<Vec<String>>> = Rc::default();
    let (w, waiting): (_, Rc<RefCell<Option<FileDialogReply>>>) = (written.clone(), Rc::default());
    let held = waiting.clone();
    let mut app = app_with_docs(1);
    make_dirty(&mut app, 0);
    app.services.export = Some(Box::new(|_: &photocraft_doc::Document, _: &str, _: &crate::ExportSettings| Ok((b"out".to_vec(), Vec::new()))));
    app.services.write = Some(Box::new(move |p: &str, _: &[u8]| {
        w.borrow_mut().push(p.to_string());
        Ok(())
    }));
    app.services.file_stamp = Some(Box::new(|p: &str| (p == "/pics/kept.psd").then_some(crate::FileStamp { len: 1, modified: None })));
    app.services.file_dialog = Some(Box::new(move |_, _, reply| *held.borrow_mut() = Some(reply)));
    let mut h = prompt_for(app, "file.close", egui::os::OperatingSystem::Windows);
    let save = |h: &mut Prompted| {
        h.key_press(Key::Y);
        h.run_steps(3);
        waiting.borrow_mut().take().expect("the save dialog is open").send(Some(FileDialogAnswer::SaveTo("/pics/kept".into())));
        h.run_steps(2);
        h.state().ui.dialogs.iter().find(|d| crate::replace_prompt::owns(&d.fields)).map(|d| d.id).expect("the save asks first")
    };
    let id = save(&mut h);
    assert!(h.state().file_dialog_open());
    assert!(h.query_by_label("(Y)es").is_none(), "the prompt waits behind the question");
    assert!(written.borrow().is_empty());

    crate::dialogs::cancel(h.state_mut(), id).unwrap();
    h.run_steps(2);
    assert!(written.borrow().is_empty(), "Cancel writes nothing");
    assert_eq!(docs_left(&h), Some(1));
    assert_eq!(focused(&h, &["(Y)es", "(N)o", "(C)ancel"]), Some("(Y)es"), "the prompt comes back");

    let id = save(&mut h);
    crate::dialogs::confirm(h.state_mut(), id).unwrap();
    h.run_steps(2);
    assert_eq!(*written.borrow(), ["/pics/kept.psd"]);
    assert!(h.state().discard.is_none());
    assert!(h.state().session.documents().is_empty());
}

#[test]
fn saving_from_the_prompt_waits_for_a_background_save_then_closes() {
    use egui_kittest::kittest::Queryable;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};
    let (gate, written) = (Arc::new(AtomicBool::new(false)), Arc::new(Mutex::new(Vec::<String>::new())));
    let mut app = app_with_docs(1);
    make_dirty(&mut app, 0);
    app.session.active_mut().unwrap().path = Some("/pics/big.psd".into());
    app.background_jobs = true;
    let (g, w) = (gate.clone(), written.clone());
    app.services.save_file = Some(Arc::new(move |_, path, _, ctx| {
        while !g.load(Ordering::Relaxed) {
            ctx.check().map_err(|e| e.to_string())?;
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        w.lock().unwrap().push(path.to_string());
        Ok(Vec::new())
    }));
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(800.0, 600.0)).build_ui_state(
        |ui, app| {
            crate::jobs_ui::tick(app, ui.ctx());
            show(app, ui.ctx());
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    h.ctx.set_os(egui::os::OperatingSystem::Windows);
    assert!(intercept(h.state_mut(), "file.close", &Value::Null));
    h.run_steps(2);
    h.key_press(Key::Y);
    h.run_steps(3);
    assert!(h.state().saving());
    assert!(h.query_by_label("(Y)es").is_none(), "the prompt waits for the save");
    assert_eq!(h.state().session.documents().len(), 1, "still open while saving");
    gate.store(true, Ordering::Relaxed);
    let t = std::time::Instant::now();
    while h.state().saving() && t.elapsed() < std::time::Duration::from_secs(30) {
        h.step();
    }
    h.run_steps(2);
    assert_eq!(*written.lock().unwrap(), ["/pics/big.psd"]);
    assert!(h.state().discard.is_none());
    assert!(h.state().session.documents().is_empty(), "closed once saved");
}

/// #6414: on the web a save stays dirty (the download isn't confirmed), and answering Save
/// still closes the tab after one write.
#[test]
fn saving_from_the_prompt_closes_after_an_unconfirmed_write() {
    use std::{cell::RefCell, rc::Rc};
    let written: Rc<RefCell<Vec<String>>> = Rc::default();
    let w = written.clone();
    let mut app = app_with_docs(1);
    make_dirty(&mut app, 0);
    app.session.active_mut().unwrap().path = Some("/pics/kept.psd".into());
    app.services.write_unconfirmed = true;
    app.services.export = Some(Box::new(|_: &photocraft_doc::Document, _: &str, _: &crate::ExportSettings| Ok((b"out".to_vec(), Vec::new()))));
    app.services.write = Some(Box::new(move |p: &str, _: &[u8]| {
        w.borrow_mut().push(p.to_string());
        Ok(())
    }));
    let mut h = prompt_for(app, "file.close", egui::os::OperatingSystem::Windows);
    h.key_press(Key::Y);
    h.run_steps(3);
    assert_eq!(*written.borrow(), ["/pics/kept.psd"]);
    assert!(h.state().discard.is_none());
    assert!(h.state().session.documents().is_empty(), "closed after one write");
}

#[path = "tests/tiff_tests.rs"]
mod tiff_tests;

#[path = "tests/targa_tests.rs"]
mod targa_tests;

#[path = "tests/dds_tests.rs"]
mod dds_tests;

#[path = "tests/jpeg_tests.rs"]
mod jpeg_tests;

/// One frame with the window's close button pressed; the commands the guard sent.
fn window_close_commands(app: &mut PhotocraftApp) -> Vec<egui::ViewportCommand> {
    let mut info = egui::ViewportInfo::default();
    info.events.push(egui::ViewportEvent::Close);
    let mut input = egui::RawInput::default();
    input.viewports.insert(egui::ViewportId::ROOT, info);
    let mut out = egui::Context::default().run_ui(input, |ui| guard_window_close(app, ui.ctx()));
    out.textures_delta.clear();
    out.viewport_output.remove(&egui::ViewportId::ROOT).map(|o| o.commands).unwrap_or_default()
}

/// One frame with the window's close button pressed; whether the guard cancelled the close.
fn press_window_close(app: &mut PhotocraftApp) -> bool {
    let commands = &window_close_commands(app);
    let cancelled = commands.iter().any(|c| matches!(c, egui::ViewportCommand::CancelClose));
    assert_eq!(commands.iter().any(|c| matches!(c, egui::ViewportCommand::Focus)), cancelled);
    cancelled
}

#[test]
fn window_close_is_cancelled_while_work_is_unsaved() {
    let mut app = app_with_docs(1);
    make_dirty(&mut app, 0);
    assert!(press_window_close(&mut app));
    assert!(app.discard.is_some());
}

/// After the window's GPU device is lost nothing can be drawn, so a prompt could never be seen
/// or answered. While unsaved work can't be written for recovery (the save keeps retrying), the
/// close button must still quit, instead of opening a prompt that never shows.
#[test]
fn window_close_goes_through_once_the_window_device_is_lost() {
    let mut app = app_with_docs(1);
    make_dirty(&mut app, 0);
    app.window_lost = true;
    assert!(!press_window_close(&mut app), "the close isn't cancelled");
    assert!(app.discard.is_none(), "no prompt opened");
}

/// The title bar's close button and File › Exit run `file.exit` through `menus::invoke`, which
/// parks it behind the prompt before any close request is made: once the window's device is
/// lost they must close instead.
#[test]
fn file_exit_quits_without_a_prompt_once_the_window_device_is_lost() {
    let exit = |app: &mut PhotocraftApp| {
        let mut out = egui::Context::default().run_ui(egui::RawInput::default(), |ui| {
            let _ = crate::menus::invoke(app, ui.ctx(), EXIT, json!({}));
        });
        out.textures_delta.clear();
        let commands = out.viewport_output.remove(&egui::ViewportId::ROOT).map(|o| o.commands).unwrap_or_default();
        commands.iter().any(|c| matches!(c, egui::ViewportCommand::Close))
    };
    let mut app = app_with_docs(1);
    make_dirty(&mut app, 0);
    assert!(!exit(&mut app), "a window that can draw asks first");
    assert!(app.discard.is_some());
    app.discard = None;
    app.window_lost = true;
    assert!(exit(&mut app), "the window is asked to close");
    assert!(app.discard.is_none(), "no prompt opened");
}

#[test]
fn unsaved_work_is_any_dirty_document_until_closing_is_confirmed() {
    let mut app = app_with_docs(2);
    assert!(!app.has_unsaved_work());
    make_dirty(&mut app, 1);
    assert!(app.has_unsaved_work(), "a dirty document that isn't the active one counts");
    app.allow_close = true;
    assert!(!app.has_unsaved_work());
}

#[test]
fn window_close_goes_through_when_nothing_is_unsaved_or_it_was_confirmed() {
    let mut app = app_with_docs(1);
    assert!(!press_window_close(&mut app));
    make_dirty(&mut app, 0);
    app.allow_close = true;
    assert!(!press_window_close(&mut app));
    assert!(app.discard.is_none());
}

/// With a platform quit (macOS), leaving runs it and keeps the window open for it to close;
/// unsaved work still asks first.
#[test]
fn leaving_runs_the_platform_quit_instead_of_closing_the_window() {
    use std::{cell::Cell, rc::Rc};
    let quits = Rc::new(Cell::new(0));
    let mut app = app_with_docs(1);
    let counted = quits.clone();
    app.services.quit = Some(Box::new(move || counted.set(counted.get() + 1)));
    // The close is cancelled for the quit to do it, with no prompt brought forward.
    let cancels_only = |c: &[egui::ViewportCommand]| {
        c.iter().any(|c| matches!(c, egui::ViewportCommand::CancelClose)) && !c.iter().any(|c| matches!(c, egui::ViewportCommand::Focus))
    };

    assert!(cancels_only(&window_close_commands(&mut app)), "the window stays for the quit to close");
    assert_eq!(quits.get(), 1);

    make_dirty(&mut app, 0);
    assert!(press_window_close(&mut app), "unsaved work asks first");
    assert_eq!(quits.get(), 1, "no quit while the prompt is up");
    assert!(app.discard.is_some());

    // Don't Save: the prompt confirms, and the close it sends quits.
    app.discard = None;
    app.allow_close = true;
    assert!(cancels_only(&window_close_commands(&mut app)));
    assert_eq!(quits.get(), 2);
}
