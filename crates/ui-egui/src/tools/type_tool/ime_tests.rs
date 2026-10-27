use super::*;
use egui::output::{IMEOutput, ImeChangeCause};

fn context() -> egui::Context {
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    install_reset_hooks(&ctx);
    ctx
}

fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 400, "height": 200})).unwrap();
    app.sync_views();
    app.ui.tool = crate::state::Tool::Type;
    pointer_up(&mut app, [50.0, 100.0], [50.0, 100.0]);
    insert(&mut app, "abc");
    app
}

fn run(app: &mut PhotocraftApp, ctx: &egui::Context, events: Vec<egui::Event>) -> Option<IMEOutput> {
    let raw = egui::RawInput { focused: true, events, ..Default::default() };
    let mut out = ctx.run_ui(raw, |ui| {
        let rect = egui::Rect::from_min_size(Pos2::ZERO, egui::vec2(400.0, 300.0));
        let xf = ViewXform { rect, zoom: 1.0, center: [100.0, 100.0], flip: false, rotation: 0.0, aspect: 1.0 };
        handle_keys(app, ui.ctx());
        draw_overlay(app, ui.painter(), &xf);
        sync_ime(app, ui.ctx());
        finish_reset_pass(app, ui.ctx());
    });
    out.textures_delta.clear(); // This protocol harness does not paint.
    out.platform_output.ime
}

fn layer_text(app: &PhotocraftApp) -> String {
    let ed = app.ui.text_edit.as_ref().expect("editing");
    current_text(app, LayerId(ed.layer)).expect("text layer")
}

fn completed(request_id: egui::ImeResetId) -> egui::Event {
    egui::Event::ImeResetCompleted { request_id }
}

fn payload(ime: &IMEOutput) -> &egui::output::IMESurroundingTextPayload {
    ime.surrounding_text.available().expect("a bounded committed snapshot")
}

fn preedit(text: &str) -> egui::Event {
    egui::Event::Ime(egui::ImeEvent::Preedit { text: text.to_owned(), active_range_chars: None })
}

#[test]
fn successive_ime_commits_are_input_method_and_keep_directional_offsets() {
    let ctx = context();
    let mut app = app();
    let baseline = run(&mut app, &ctx, Vec::new()).expect("a focused editor descriptor");
    assert_eq!(payload(&baseline).text, "abc");
    assert_eq!(baseline.change_cause, ImeChangeCause::Other);

    // The pre-edit is laid out in the layer but is not committed surrounding text.
    let composing = run(&mut app, &ctx, vec![preedit("に")]).expect("descriptor while composing");
    assert_eq!(payload(&composing).text, "abc");
    assert_eq!((payload(&composing).cursor, payload(&composing).anchor), (3, 3));

    let first = run(&mut app, &ctx, vec![egui::Event::Ime(egui::ImeEvent::Commit("日本".into()))]).expect("commit descriptor");
    assert_eq!(first.change_cause, ImeChangeCause::InputMethod);
    assert_eq!(payload(&first).text, "abc日本");

    let composing = run(&mut app, &ctx, vec![preedit("ご")]).expect("second composition");
    assert_eq!(payload(&composing).text, "abc日本");
    let second = run(&mut app, &ctx, vec![egui::Event::Ime(egui::ImeEvent::Commit("語".into()))]).expect("second commit");
    assert_eq!(second.change_cause, ImeChangeCause::InputMethod);
    assert_eq!(payload(&second).text, "abc日本語");
    assert_eq!(payload(&second).cursor, payload(&second).text.len());
}

#[test]
fn reversed_selection_keeps_its_direction_in_byte_offsets() {
    let ctx = context();
    let mut app = app();
    if let Some(ed) = app.ui.text_edit.as_mut() {
        ed.caret = 1;
        ed.anchor = 3;
    }
    let selected = run(&mut app, &ctx, Vec::new()).expect("descriptor");
    assert_eq!(payload(&selected).text, "abc");
    assert_eq!((payload(&selected).cursor, payload(&selected).anchor), (1, 3));
}

#[test]
fn mixed_application_and_ime_edits_in_one_frame_are_other() {
    let ctx = context();
    let mut app = app();
    run(&mut app, &ctx, Vec::new());

    // An IME commit followed by ordinary typing in the same pass: the application change
    // dominates even though the commit itself was accepted.
    let mixed = run(&mut app, &ctx, vec![egui::Event::Ime(egui::ImeEvent::Commit("い".into())), egui::Event::Text("x".into())]).expect("mixed descriptor");
    assert_eq!(payload(&mixed).text, "abcいx");
    assert_eq!(mixed.change_cause, ImeChangeCause::Other);

    // The observed mixed state is the next baseline, so a later clean IME commit is once
    // again an InputMethod change.
    let next = run(&mut app, &ctx, vec![egui::Event::Ime(egui::ImeEvent::Commit("う".into()))]).expect("next commit");
    assert_eq!(next.change_cause, ImeChangeCause::InputMethod);
    assert_eq!(payload(&next).text, "abcいxう");

    // Application text before the commit in the same frame must also prevent InputMethod.
    let mut app = self::app();
    run(&mut app, &ctx, Vec::new());
    let mixed = run(&mut app, &ctx, vec![egui::Event::Text("x".into()), egui::Event::Ime(egui::ImeEvent::Commit("い".into()))]).expect("mixed descriptor");
    assert_eq!(mixed.change_cause, ImeChangeCause::Other);
    assert_eq!(payload(&mixed).text, "abcxい");
}

#[test]
fn replaced_layer_or_session_bumps_generation_without_changing_owner() {
    let ctx = context();
    let mut app = app();
    let first = run(&mut app, &ctx, Vec::new()).expect("descriptor");
    let owner = first.owner;
    let generation = first.context_generation;

    // Same document and layer id, a new edit session: the replacement epoch changes, so the
    // stale generation cannot be reused by a native IME bridge.
    app.ui.text_edit.as_mut().unwrap().session = "type-replaced".into();
    let replaced = run(&mut app, &ctx, Vec::new()).expect("replaced descriptor");
    assert_eq!(replaced.owner, owner, "the layer identity is immutable");
    assert_ne!(replaced.context_generation, generation);
    assert_eq!(replaced.change_cause, ImeChangeCause::Other);

    // Ending the editor ends the context; no descriptor is emitted for an absent owner.
    app.ui.text_edit = None;
    assert!(run(&mut app, &ctx, Vec::new()).is_none());
}

#[test]
fn unavailable_editor_state_still_carries_owner_and_generation() {
    let ctx = context();
    let mut app = app();
    // A stale composition range cannot be mapped onto the layer: do not silently publish
    // the composition as committed text.
    app.ui.text_edit.as_mut().unwrap().preedit = Some((10, 5));
    let unavailable = run(&mut app, &ctx, Vec::new()).expect("descriptor with an owner");
    assert!(unavailable.surrounding_text.available().is_none());
    assert!(unavailable.context_generation > 0);
    assert_eq!(unavailable.purpose, egui::IMEPurpose::Normal);
}

#[test]
fn delete_surrounding_bytes_with_preedit_excludes_the_composition() {
    let ctx = context();
    let mut app = app();
    run(&mut app, &ctx, Vec::new());
    run(&mut app, &ctx, vec![preedit("x")]);
    assert_eq!(layer_text(&app), "abcx");
    assert_eq!(app.ui.text_edit.as_ref().unwrap().preedit, Some((3, 1)));

    // `c` is the byte immediately before the pre-edit; deleting it must not delete or
    // report the composing `x`.
    let deleted =
        run(&mut app, &ctx, vec![egui::Event::Ime(egui::ImeEvent::DeleteSurroundingBytes { before_bytes: 1, after_bytes: 0 })]).expect("deletion descriptor");
    assert_eq!(deleted.change_cause, ImeChangeCause::InputMethod);
    assert_eq!(payload(&deleted).text, "ab");
    assert_eq!((payload(&deleted).cursor, payload(&deleted).anchor), (2, 2));
    assert_eq!(layer_text(&app), "abx");
    assert_eq!(app.ui.text_edit.as_ref().unwrap().preedit, Some((2, 1)));
}

#[test]
fn external_edit_invalidates_the_preedit_and_rejects_its_late_commit() {
    for bytes in [false, true] {
        for split in [false, true] {
            for discard in [false, true] {
                let ctx = context();
                let mut app = app();
                run(&mut app, &ctx, Vec::new());
                // Compose in the middle of committed text, so an application edit before it
                // moves a still-in-range (but wrong) character range.
                insert(&mut app, "def");
                if let Some(ed) = app.ui.text_edit.as_mut() {
                    ed.caret = 3;
                    ed.anchor = 3;
                }
                let composing = run(&mut app, &ctx, vec![preedit("に")]).expect("composing descriptor");
                assert_eq!(layer_text(&app), "abcにdef");
                assert_eq!(payload(&composing).text, "abcdef", "the preview is not committed text");
                let layer = app.ui.text_edit.as_ref().unwrap().layer;
                let session = app.ui.text_edit.as_ref().unwrap().session.clone();
                // The control channel edits the same layer without passing through `insert`.
                app.run("type.edit", json!({"layer": layer, "replace": {"start": 0, "end": 1, "text": ""}, "coalesce": session})).unwrap();
                assert_eq!(layer_text(&app), "bcにdef");
                let revision = app.session.active().unwrap().revision;
                let history = app.session.active().unwrap().history.entries();
                let selection = app.ui.text_edit.as_ref().map(|ed| (ed.anchor, ed.caret));
                let unchanged = |app: &PhotocraftApp| {
                    assert_eq!(layer_text(app), "bcにdef");
                    assert_eq!(app.session.active().unwrap().revision, revision);
                    assert_eq!(app.session.active().unwrap().history.entries(), history);
                    assert_eq!(app.ui.text_edit.as_ref().map(|ed| (ed.anchor, ed.caret)), selection);
                };

                // The old range (3,1) now covers `d`: report unavailable, cancel the
                // composition and do not present `d` as the composition. A discarded pass
                // must retain both latches until a descriptor is actually delivered.
                let invalidated = if discard {
                    ctx.options_mut(|o| o.max_passes = std::num::NonZeroUsize::new(4).expect("four passes"));
                    let mut passes = Vec::new();
                    let mut out = ctx.run_ui(egui::RawInput { focused: true, ..Default::default() }, |ui| {
                        handle_keys(&mut app, ui.ctx());
                        let rect = egui::Rect::from_min_size(Pos2::ZERO, egui::vec2(400.0, 300.0));
                        let xf = ViewXform { rect, zoom: 1.0, center: [100.0, 100.0], flip: false, rotation: 0.0, aspect: 1.0 };
                        draw_overlay(&mut app, ui.painter(), &xf);
                        if passes.is_empty() {
                            ui.ctx().request_discard("invalidated preedit descriptor regression");
                        }
                        let discarded = ui.ctx().will_discard();
                        let before = (app.type_ime.stale, app.type_ime.interrupt);
                        sync_ime(&mut app, ui.ctx());
                        finish_reset_pass(&mut app, ui.ctx());
                        let unavailable = ui.ctx().output(|out| out.ime.as_ref().expect("pass descriptor").surrounding_text.available().is_none());
                        passes.push((discarded, before, (app.type_ime.stale, app.type_ime.interrupt), unavailable));
                    });
                    out.textures_delta.clear();
                    assert_eq!(passes.first(), Some(&(true, (true, true), (true, true), true)), "discard cannot consume invalidation");
                    assert_eq!(passes.last(), Some(&(false, (true, true), (false, false), true)), "the accepted pass delivers it once");
                    out.platform_output.ime.expect("descriptor")
                } else {
                    run(&mut app, &ctx, Vec::new()).expect("descriptor")
                };
                assert!(invalidated.surrounding_text.available().is_none());
                assert!(invalidated.should_interrupt_composition);
                assert_eq!(
                    (invalidated.owner, invalidated.context_generation, invalidated.purpose),
                    (composing.owner, composing.context_generation, composing.purpose)
                );
                assert_eq!(invalidated.change_cause, ImeChangeCause::Other);
                assert_eq!(app.ui.text_edit.as_ref().unwrap().preedit, None);
                assert!(app.type_ime.preedit.is_none());
                unchanged(&app);

                let recovered = run(&mut app, &ctx, Vec::new()).expect("idle descriptor");
                assert_eq!(payload(&recovered).text, "bcにdef");
                assert_eq!((payload(&recovered).cursor, payload(&recovered).anchor), (6, 6));
                assert!(!recovered.should_interrupt_composition, "a delivered reset is not replayed");
                assert!(matches!(app.type_ime.incoming, Some(IncomingComposition::Stale)), "idle availability does not accept an old terminal");
                unchanged(&app);

                // The pending deletion and commit belonged to the cancelled composition:
                // neither combined nor split delivery may replace `d` or grow undo history.
                let mut events = vec![terminal_delete(bytes)];
                if split {
                    run(&mut app, &ctx, events).expect("rejected deletion descriptor");
                    unchanged(&app);
                    for _ in 0..3 {
                        run(&mut app, &ctx, Vec::new());
                    }
                    assert!(matches!(app.type_ime.incoming, Some(IncomingComposition::Stale)));
                    events = Vec::new();
                }
                events.push(egui::Event::Ime(egui::ImeEvent::Commit("に".into())));
                let late = run(&mut app, &ctx, events).expect("descriptor");
                assert_eq!(layer_text(&app), "bcにdef");
                assert_eq!(late.change_cause, ImeChangeCause::Other);
                assert_eq!(payload(&late).text, "bcにdef");
                unchanged(&app);
                assert!(matches!(app.type_ime.incoming, Some(IncomingComposition::Stale)), "terminal input cannot acknowledge cancellation");

                let fresh =
                    run(&mut app, &ctx, vec![completed(invalidated.reset_request.expect("reset request")), preedit("Q")]).expect("fresh preview descriptor");
                assert_eq!(layer_text(&app), "bcにdQef");
                assert_eq!(payload(&fresh).text, "bcにdef", "fresh preview is excluded from committed text");
                assert!(!fresh.should_interrupt_composition);
                let committed = run(&mut app, &ctx, vec![egui::Event::Ime(egui::ImeEvent::Commit("Q".into()))]).expect("fresh commit descriptor");
                assert_eq!(layer_text(&app), "bcにdQef");
                assert_eq!(payload(&committed).text, "bcにdQef");
                assert_eq!(committed.change_cause, ImeChangeCause::InputMethod);
                let direct = run(&mut app, &ctx, vec![egui::Event::Ime(egui::ImeEvent::Commit("R".into()))]).expect("direct commit descriptor");
                assert_eq!(layer_text(&app), "bcにdQRef");
                assert_eq!(payload(&direct).text, "bcにdQRef");
                assert_eq!(direct.change_cause, ImeChangeCause::InputMethod);
            }
        }
    }
}

#[test]
fn target_change_interrupts_and_rejects_the_late_commit() {
    let ctx = context();
    let mut app = app();
    run(&mut app, &ctx, Vec::new());
    run(&mut app, &ctx, vec![preedit("に")]);
    assert_eq!(layer_text(&app), "abcに");

    let created = app.run("type.create", json!({"text": "two", "x": 150, "y": 60})).unwrap();
    let second = created.get("layer").and_then(serde_json::Value::as_u64).expect("new layer");
    app.session.select_layer(LayerId(second)).expect("select the second type layer");
    app.ui.text_edit = Some(TextEdit {
        layer: second,
        document: active_doc_id(&app),
        caret: 3,
        anchor: 3,
        session: "type-second".into(),
        created: false,
        mask: false,
        dragging: false,
        resize: None,
        preedit: None,
    });
    // The platform only cancels composition on this flag (X11, macOS, Windows and the web).
    let switched = run(&mut app, &ctx, Vec::new()).expect("transition descriptor");
    assert!(switched.should_interrupt_composition);

    let late = run(&mut app, &ctx, vec![egui::Event::Ime(egui::ImeEvent::Commit("日本".into()))]).expect("descriptor");
    assert_eq!(current_text(&app, LayerId(second)).unwrap(), "two");
    assert_eq!(late.change_cause, ImeChangeCause::Other);
    run(&mut app, &ctx, vec![completed(switched.reset_request.expect("replacement layer reset")), preedit("が")]);
    assert_eq!(layer_text(&app), "twoが");
}

#[test]
fn type_mask_session_has_a_distinct_owner() {
    let ctx = context();
    let mut app = app();
    let normal = run(&mut app, &ctx, Vec::new()).expect("descriptor");
    app.ui.text_edit.as_mut().unwrap().mask = true;
    let masked = run(&mut app, &ctx, Vec::new()).expect("mask descriptor");
    assert_ne!(masked.owner, normal.owner, "the Type Mask session is its own owner");
    assert_ne!(masked.context_generation, normal.context_generation);
}

#[test]
fn reentering_the_same_session_cancels_the_preedit() {
    let ctx = context();
    let mut app = app();
    run(&mut app, &ctx, vec![preedit("に")]);
    assert_eq!(app.ui.text_edit.as_ref().unwrap().preedit, Some((3, 1)));
    edit_active(&mut app).unwrap();
    assert_eq!(app.ui.text_edit.as_ref().unwrap().preedit, None);
    assert!(app.type_ime.interrupt, "the native composition is cancelled");
    assert!(app.type_ime.preedit.is_none());
}

#[test]
fn a_discarded_transition_pass_still_interrupts_the_next_descriptor() {
    let ctx = context();
    ctx.options_mut(|o| o.max_passes = std::num::NonZeroUsize::new(4).expect("four passes"));
    let mut app = app();
    run(&mut app, &ctx, Vec::new()).expect("baseline descriptor");
    app.ui.text_edit.as_mut().unwrap().session = "type-discarded".into();
    let mut passes = Vec::new();
    let mut out = ctx.run_ui(egui::RawInput { focused: true, ..Default::default() }, |ui| {
        handle_keys(&mut app, ui.ctx());
        let rect = egui::Rect::from_min_size(Pos2::ZERO, egui::vec2(400.0, 300.0));
        let xf = ViewXform { rect, zoom: 1.0, center: [100.0, 100.0], flip: false, rotation: 0.0, aspect: 1.0 };
        draw_overlay(&mut app, ui.painter(), &xf);
        if passes.is_empty() {
            ui.ctx().request_discard("IME transition regression");
        }
        let discarded = ui.ctx().will_discard();
        let pending_before = app.type_ime.interrupt;
        sync_ime(&mut app, ui.ctx());
        finish_reset_pass(&mut app, ui.ctx());
        passes.push((discarded, pending_before, app.type_ime.interrupt));
    });
    out.textures_delta.clear();
    assert_eq!(passes.first(), Some(&(true, true, true)), "discard retains the interrupt latch");
    assert_eq!(passes.last(), Some(&(false, true, false)), "the accepted pass delivers it once");
    assert!(out.platform_output.ime.expect("final descriptor").should_interrupt_composition);
    let next = run(&mut app, &ctx, Vec::new()).expect("next descriptor");
    assert!(!next.should_interrupt_composition, "a delivered interrupt is not replayed");
}

#[test]
fn delete_surrounding_characters_is_one_undo_step() {
    let ctx = context();
    let mut app = app();
    commit(&mut app);
    edit_active(&mut app).unwrap();
    if let Some(ed) = app.ui.text_edit.as_mut() {
        ed.caret = 3;
        ed.anchor = 3;
    }
    run(&mut app, &ctx, Vec::new()).expect("baseline descriptor");
    let deleted =
        run(&mut app, &ctx, vec![egui::Event::Ime(egui::ImeEvent::DeleteSurrounding { before_chars: 1, after_chars: 0 })]).expect("deletion descriptor");
    assert_eq!(deleted.change_cause, ImeChangeCause::InputMethod);
    assert_eq!(layer_text(&app), "ab");
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(layer_text(&app), "abc", "the delete and its session coalesce into one step");
}

#[test]
fn an_empty_point_text_reports_an_available_empty_snapshot() {
    let ctx = context();
    let mut app = app();
    if let Some(ed) = app.ui.text_edit.as_mut() {
        ed.anchor = 0;
        ed.caret = 3;
    }
    insert(&mut app, "");
    assert_eq!(layer_text(&app), "");
    let empty = run(&mut app, &ctx, Vec::new()).expect("descriptor");
    let available = payload(&empty);
    assert_eq!(available.text, "");
    assert_eq!((available.cursor, available.anchor), (0, 0));
}

#[test]
fn a_long_line_reports_a_bounded_snapshot() {
    let ctx = context();
    let mut app = app();
    if let Some(ed) = app.ui.text_edit.as_mut() {
        ed.anchor = 0;
        ed.caret = 3;
    }
    insert(&mut app, &"a".repeat(4500));
    let long = run(&mut app, &ctx, Vec::new()).expect("descriptor");
    let available = payload(&long);
    assert!(available.text.len() <= egui::output::MAX_IME_SURROUNDING_TEXT_BYTES);
    assert!(available.text.is_char_boundary(available.cursor));
    assert!(available.text.is_char_boundary(available.anchor));
}

#[test]
fn the_real_app_logic_and_ui_publish_the_descriptor() {
    use eframe::App as _;
    let ctx = context();
    let mut app = app();
    let mut frame = eframe::Frame::_new_kittest();
    let mut descriptors = Vec::new();
    // The first pass registers fonts; the next passes run the real logic → ui orchestration
    // (where `handle_keys` and the final `sync_ime` live).
    for _ in 0..3 {
        let raw = egui::RawInput { focused: true, screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, egui::vec2(1200.0, 800.0))), ..Default::default() };
        let mut out = ctx.run_ui(raw, |ui| {
            app.logic(ui.ctx(), &mut frame);
            app.ui(ui, &mut frame);
        });
        out.textures_delta.clear(); // No renderer consumes this harness output.
        descriptors.push(out.platform_output.ime);
    }
    let ime = descriptors.iter().flatten().last().expect("a descriptor from App::ui");
    assert!(ime.surrounding_text.available().is_some());
    assert!(ime.context_generation > 0);
}

fn terminal_delete(bytes: bool) -> egui::Event {
    egui::Event::Ime(if bytes {
        egui::ImeEvent::DeleteSurroundingBytes { before_bytes: 1, after_bytes: 0 }
    } else {
        egui::ImeEvent::DeleteSurrounding { before_chars: 1, after_chars: 0 }
    })
}

fn raw_key(key: egui::Key, command: bool, shift: bool) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: Some(key),
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers {
            command,
            ctrl: command && !cfg!(target_os = "macos"),
            mac_cmd: command && cfg!(target_os = "macos"),
            shift,
            ..Default::default()
        },
    }
}

#[test]
fn stale_owner_terminal_survives_idle_and_split_delete_commit_but_fresh_input_works() {
    for empty in [false, true] {
        for bytes in [false, true] {
            let ctx = context();
            let mut app = app();
            run(&mut app, &ctx, vec![]);
            run(&mut app, &ctx, vec![preedit("P")]);
            if empty {
                run(&mut app, &ctx, vec![preedit("")]);
            }
            let old_layer = app.ui.text_edit.as_ref().unwrap().layer;
            let created = app.run("type.create", json!({"text": "two", "x": 150, "y": 60})).unwrap();
            let second = created["layer"].as_u64().unwrap();
            app.session.select_layer(LayerId(second)).unwrap();
            edit_active(&mut app).unwrap();
            let transition = run(&mut app, &ctx, vec![]).unwrap();
            assert!(transition.should_interrupt_composition);
            for _ in 0..5 {
                run(&mut app, &ctx, vec![]);
            }
            let revision = app.session.active().unwrap().revision;
            let undo = app.session.active().unwrap().history.entries().len();
            let before = layer_text(&app);
            run(&mut app, &ctx, vec![terminal_delete(bytes)]);
            for _ in 0..3 {
                run(&mut app, &ctx, vec![]);
            }
            run(&mut app, &ctx, vec![egui::Event::Ime(egui::ImeEvent::Commit("X".into()))]);
            assert_eq!(layer_text(&app), before, "an old split terminal cannot edit a replacement owner");
            assert_eq!(app.session.active().unwrap().revision, revision);
            assert_eq!(app.session.active().unwrap().history.entries().len(), undo);
            assert_eq!(current_text(&app, LayerId(old_layer)).unwrap(), if empty { "abc" } else { "abcP" });
            let selection = app.ui.text_edit.as_ref().unwrap().clone();
            let (a, b) = (selection.anchor.min(selection.caret), selection.anchor.max(selection.caret));
            let expected = before.chars().take(a).chain("Q".chars()).chain(before.chars().skip(b)).collect::<String>();
            run(
                &mut app,
                &ctx,
                vec![completed(transition.reset_request.expect("reset request")), preedit("Q"), egui::Event::Ime(egui::ImeEvent::Commit("Q".into()))],
            );
            assert_eq!(layer_text(&app), expected, "the exact completion permits fresh input");
            run(&mut app, &ctx, vec![egui::Event::Ime(egui::ImeEvent::Commit("R".into()))]);
            assert!(layer_text(&app).contains("QR"), "a later direct commit remains valid");
            app.run("edit.undo", json!({})).unwrap();
            assert_eq!(current_text(&app, LayerId(second)).unwrap(), before, "the new session is undoable");
        }
    }
}

#[test]
fn raw_application_selection_changes_reject_old_terminal_input_and_allow_later_fresh_input() {
    for empty in [false, true] {
        for bytes in [false, true] {
            for (key, command, shift) in [(egui::Key::A, true, false), (egui::Key::ArrowLeft, false, false), (egui::Key::Home, false, true)] {
                let ctx = context();
                let mut app = app();
                run(&mut app, &ctx, vec![]);
                run(&mut app, &ctx, vec![preedit("P")]);
                if empty {
                    run(&mut app, &ctx, vec![preedit("")]);
                }
                let before = layer_text(&app);
                let revision = app.session.active().unwrap().revision;
                let transition = run(&mut app, &ctx, vec![raw_key(key, command, shift)]).expect("selection reset");
                let selected = app.ui.text_edit.as_ref().unwrap().clone();
                assert!((selected.anchor, selected.caret) != (before.chars().count(), before.chars().count()), "actual raw selection key changed the caret");
                for _ in 0..3 {
                    run(&mut app, &ctx, vec![]);
                }
                let undo = app.session.active().unwrap().history.entries().len();
                run(&mut app, &ctx, vec![terminal_delete(bytes), egui::Event::Ime(egui::ImeEvent::Commit("X".into()))]);
                assert_eq!(layer_text(&app), before);
                assert_eq!(app.session.active().unwrap().revision, revision);
                assert_eq!(app.session.active().unwrap().history.entries().len(), undo);
                assert_eq!(app.ui.text_edit.as_ref().map(|e| (e.anchor, e.caret)), Some((selected.anchor, selected.caret)));
                let (a, b) = (selected.anchor.min(selected.caret), selected.anchor.max(selected.caret));
                let expected = before.chars().take(a).chain("Q".chars()).chain(before.chars().skip(b)).collect::<String>();
                // The completion opens admission at its exact position in the batch.
                run(
                    &mut app,
                    &ctx,
                    vec![completed(transition.reset_request.expect("reset request")), preedit("Q"), egui::Event::Ime(egui::ImeEvent::Commit("Q".into()))],
                );
                assert_eq!(layer_text(&app), expected);
            }
        }
    }
}

#[test]
fn uninterrupted_live_and_empty_preedit_delete_commit_and_direct_commit_stay_valid() {
    for empty in [false, true] {
        for bytes in [false, true] {
            let ctx = context();
            let mut app = app();
            run(&mut app, &ctx, vec![]);
            run(&mut app, &ctx, vec![preedit("P")]);
            if empty {
                run(&mut app, &ctx, vec![preedit("")]);
            }
            run(&mut app, &ctx, vec![terminal_delete(bytes)]);
            for _ in 0..3 {
                run(&mut app, &ctx, vec![]);
            }
            run(&mut app, &ctx, vec![egui::Event::Ime(egui::ImeEvent::Commit("X".into()))]);
            assert_eq!(layer_text(&app), "abX", "accepted deletion refreshes its composing caret");
        }
    }
    let ctx = context();
    let mut app = app();
    run(&mut app, &ctx, vec![]);
    run(&mut app, &ctx, vec![raw_key(egui::Key::A, true, false), egui::Event::Ime(egui::ImeEvent::Commit("D".into()))]);
    assert_eq!(layer_text(&app), "D", "without known preedit the direct commit replaces the selection");
}

fn actual_app_frame(app: &mut PhotocraftApp, ctx: &egui::Context, events: Vec<egui::Event>) -> egui::FullOutput {
    use eframe::App as _;
    let mut frame = eframe::Frame::_new_kittest();
    let mut raw =
        egui::RawInput { focused: true, events, screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, egui::vec2(1200.0, 800.0))), ..Default::default() };
    app.raw_input_hook(ctx, &mut raw);
    let mut output = ctx.run_ui(raw, |ui| {
        app.logic(ui.ctx(), &mut frame);
        app.ui(ui, &mut frame);
    });
    output.textures_delta.clear();
    output
}

#[test]
fn the_real_app_pointer_interruption_rejects_idle_delayed_terminal_then_accepts_fresh_input() {
    for empty in [false, true] {
        for bytes in [false, true] {
            let ctx = context();
            let mut app = app();
            for _ in 0..4 {
                actual_app_frame(&mut app, &ctx, vec![]);
            }
            app.ui.text_edit.as_mut().unwrap().caret = 1;
            app.ui.text_edit.as_mut().unwrap().anchor = 1;
            let point = actual_app_frame(&mut app, &ctx, vec![]).platform_output.ime.expect("real caret1 descriptor").cursor_rect.center();
            app.ui.text_edit.as_mut().unwrap().caret = 3;
            app.ui.text_edit.as_mut().unwrap().anchor = 3;
            actual_app_frame(&mut app, &ctx, vec![]);
            actual_app_frame(&mut app, &ctx, vec![preedit("P")]);
            if empty {
                actual_app_frame(&mut app, &ctx, vec![preedit("")]);
            }
            let before = layer_text(&app);
            let button = |pressed| egui::Event::PointerButton { pos: point, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
            let press = actual_app_frame(&mut app, &ctx, vec![egui::Event::PointerMoved(point), button(true)]).platform_output.ime.expect("press descriptor");
            let release = actual_app_frame(&mut app, &ctx, vec![button(false)]).platform_output.ime.expect("release descriptor");
            let request = release.reset_request.or(press.reset_request).expect("pointer reset");
            assert_eq!(app.ui.text_edit.as_ref().unwrap().caret, 1, "actual canvas pointer moved the caret");
            for _ in 0..4 {
                actual_app_frame(&mut app, &ctx, vec![]);
            }
            let revision = app.session.active().unwrap().revision;
            actual_app_frame(&mut app, &ctx, vec![terminal_delete(bytes), egui::Event::Ime(egui::ImeEvent::Commit("X".into()))]);
            assert_eq!(layer_text(&app), before);
            assert_eq!(app.session.active().unwrap().revision, revision);
            actual_app_frame(&mut app, &ctx, vec![completed(request), preedit("Q"), egui::Event::Ime(egui::ImeEvent::Commit("Q".into()))]);
            let expected = before.chars().take(1).chain("Q".chars()).chain(before.chars().skip(1)).collect::<String>();
            assert_eq!(layer_text(&app), expected);
        }
    }
}

#[test]
fn only_a_matching_completion_allows_a_new_composition_in_the_same_raw_batch() {
    for bytes in [false, true] {
        let ctx = context();
        let mut app = app();
        run(&mut app, &ctx, vec![]);
        run(&mut app, &ctx, vec![preedit("P")]);
        let transition = run(&mut app, &ctx, vec![raw_key(egui::Key::A, true, false)]).expect("selection reset");
        run(
            &mut app,
            &ctx,
            vec![
                terminal_delete(bytes),
                egui::Event::Ime(egui::ImeEvent::Commit("old".into())),
                completed(transition.reset_request.expect("reset request")),
                preedit("Q"),
                egui::Event::Ime(egui::ImeEvent::Commit("Q".into())),
            ],
        );
        assert_eq!(layer_text(&app), "Q", "the old terminal is rejected before the completion opens admission");
    }
}

#[test]
fn an_actual_same_batch_selection_requires_completion_before_the_new_live_preedit() {
    for empty in [false, true] {
        for bytes in [false, true] {
            for terminal in [false, true] {
                let ctx = context();
                let mut app = app();
                for _ in 0..4 {
                    actual_app_frame(&mut app, &ctx, vec![]);
                }
                actual_app_frame(&mut app, &ctx, vec![preedit("P")]);
                if empty {
                    actual_app_frame(&mut app, &ctx, vec![preedit("")]);
                }
                let before = layer_text(&app);
                let revision = app.session.active().unwrap().revision;
                let mut events = vec![raw_key(egui::Key::A, true, false), terminal_delete(bytes)];
                if terminal {
                    events.push(egui::Event::Ime(egui::ImeEvent::Commit("old".into())));
                }
                events.push(preedit("Q"));
                let out = actual_app_frame(&mut app, &ctx, events);
                assert_eq!(layer_text(&app), before, "all unacknowledged payloads remain stale, even after Commit");
                assert_eq!(app.session.active().unwrap().revision, revision);
                assert!(matches!(app.type_ime.incoming, Some(IncomingComposition::Stale)));
                let selected = app.ui.text_edit.as_ref().unwrap();
                assert_eq!((selected.anchor, selected.caret), (0, before.chars().count()), "the actual selection key ran");
                let request = out.platform_output.ime.expect("interrupted descriptor").reset_request.expect("reset request");
                actual_app_frame(&mut app, &ctx, vec![completed(request), preedit("Q")]);
                assert_eq!(layer_text(&app), "Q", "only the new preedit replaced the selected text");
                assert_eq!(app.session.active().unwrap().revision, revision + 1, "neither old deletion nor old commit mutated the document");
                let editing = app.ui.text_edit.as_ref().unwrap();
                assert_eq!((editing.anchor, editing.caret, editing.preedit), (1, 1, Some((0, 1))));
                assert!(matches!(app.type_ime.incoming, Some(IncomingComposition::Active(_))));
                // Keep Q live across a real idle pass before its separately delivered commit.
                let idle = actual_app_frame(&mut app, &ctx, vec![]);
                assert!(!idle.platform_output.ime.expect("the live preview keeps its descriptor").should_interrupt_composition);
                actual_app_frame(&mut app, &ctx, vec![egui::Event::Ime(egui::ImeEvent::Commit("Q".into()))]);
                assert_eq!(layer_text(&app), "Q");
                assert!(app.ui.text_edit.as_ref().unwrap().preedit.is_none());
                assert!(app.type_ime.incoming.is_none());
            }
        }
    }
}

fn pending(app: &PhotocraftApp) -> Option<egui::ImeResetId> {
    match app.type_ime.resets.get(&egui::ViewportId::ROOT) {
        Some(ResetPhase::Awaiting { id, .. }) => Some(*id),
        _ => None,
    }
}

fn interrupt_with_selection(app: &mut PhotocraftApp, ctx: &egui::Context) -> egui::ImeResetId {
    run(app, ctx, vec![preedit("P")]);
    run(app, ctx, vec![raw_key(egui::Key::A, true, false)]).expect("custom output").reset_request.expect("published reset")
}

#[test]
fn newest_published_request_alone_opens_the_ordered_suffix() {
    let ctx = context();
    let mut app = app();
    let first = interrupt_with_selection(&mut app, &ctx);
    let second = run(&mut app, &ctx, vec![raw_key(egui::Key::ArrowLeft, false, false), completed(first), preedit("old")])
        .expect("moved selection")
        .reset_request
        .expect("replacement request");
    assert_ne!(first, second);
    let before = layer_text(&app);
    let revision = app.session.active().unwrap().revision;
    let history = app.session.active().unwrap().history.entries().len();
    run(&mut app, &ctx, vec![terminal_delete(true), completed(first), egui::Event::Ime(egui::ImeEvent::Commit("old".into()))]);
    assert_eq!(layer_text(&app), before);
    assert_eq!(app.session.active().unwrap().revision, revision);
    assert_eq!(app.session.active().unwrap().history.entries().len(), history);
    assert_eq!(pending(&app), Some(second));
    let selection = app.ui.text_edit.as_ref().map(|e| (e.anchor, e.caret));
    run(&mut app, &ctx, vec![completed(second), completed(second)]);
    assert_eq!(layer_text(&app), before, "controls alone never edit text");
    assert_eq!(app.ui.text_edit.as_ref().map(|e| (e.anchor, e.caret)), selection);
    assert_eq!(app.session.active().unwrap().revision, revision);
    assert_eq!(app.session.active().unwrap().history.entries().len(), history);
    assert_eq!(pending(&app), None);
    run(&mut app, &ctx, vec![preedit("が"), egui::Event::Ime(egui::ImeEvent::Commit("が".into()))]);
    assert!(layer_text(&app).starts_with("が"));
}

#[test]
fn foreign_viewport_and_context_completions_cannot_release_root() {
    let ctx = context();
    let mut app = app();
    let root = interrupt_with_selection(&mut app, &ctx);
    let other = context();
    let viewport = egui::ViewportId::from_hash_of("photo-reset-foreign");
    let foreign_input = || {
        let rect = egui::Rect::from_min_size(Pos2::ZERO, egui::vec2(400.0, 300.0));
        let mut raw = egui::RawInput { viewport_id: viewport, screen_rect: Some(rect), focused: true, ..Default::default() };
        raw.viewports
            .insert(viewport, egui::ViewportInfo { inner_rect: Some(rect), native_pixels_per_point: Some(1.0), minimized: Some(false), ..Default::default() });
        raw
    };
    let mut text = String::new();
    let mut output = other.run_ui(foreign_input(), |ui| {
        ui.add(egui::TextEdit::singleline(&mut text)).request_focus();
        let _ = ui.ctx().request_ime_reset(viewport);
    });
    output.textures_delta.clear();
    // Draw once more if egui has not yet reported the newly focused stock editor.
    if output.platform_output.ime.is_none() {
        output = other.run_ui(foreign_input(), |ui| {
            ui.add(egui::TextEdit::singleline(&mut text));
        });
        output.textures_delta.clear();
    }
    let foreign = output.platform_output.ime.expect("foreign stock output").reset_request.expect("foreign published request");
    assert_ne!(foreign.viewport_id(), root.viewport_id());
    let before = layer_text(&app);
    run(&mut app, &ctx, vec![completed(foreign), preedit("bad"), completed(root), preedit("が")]);
    assert_eq!(layer_text(&app), "が", "foreign marker cannot open admission before the root marker");
    assert_ne!(layer_text(&app), before);
}

#[test]
fn standalone_logic_suspends_once_without_replaying_input_and_return_rearms_once() {
    use eframe::App as _;
    let ctx = context();
    let mut app = app();
    for _ in 0..4 {
        actual_app_frame(&mut app, &ctx, vec![]);
    }
    actual_app_frame(&mut app, &ctx, vec![preedit("P")]);
    let first = actual_app_frame(&mut app, &ctx, vec![raw_key(egui::Key::A, true, false)])
        .platform_output
        .ime
        .expect("custom output")
        .reset_request
        .expect("first request");
    let before = layer_text(&app);
    for _ in 0..3 {
        let mut frame = eframe::Frame::_new_kittest();
        let _ = ctx.run_logic(&egui::RawInput { focused: true, ..Default::default() }, |ctx| app.logic(ctx, &mut frame));
        assert_eq!(layer_text(&app), before, "hidden logic does not replay the previous raw selection batch");
        assert_eq!(pending(&app), None);
        assert!(matches!(app.type_ime.resets.get(&egui::ViewportId::ROOT), Some(ResetPhase::Suspended)));
    }
    let second = actual_app_frame(&mut app, &ctx, vec![completed(first), preedit("bad")])
        .platform_output
        .ime
        .expect("returned custom output")
        .reset_request
        .expect("fresh request after hidden logic");
    assert_ne!(first, second);
    assert_eq!(layer_text(&app), before);
    for _ in 0..3 {
        let idle = actual_app_frame(&mut app, &ctx, vec![]).platform_output.ime.expect("idle custom output");
        assert_eq!(idle.reset_request, None, "logic inside a full UI pass cannot suspend and rearm the stable guard");
        assert_eq!(pending(&app), Some(second));
    }
    actual_app_frame(&mut app, &ctx, vec![completed(second), preedit("が"), egui::Event::Ime(egui::ImeEvent::Commit("が".into()))]);
    assert_eq!(layer_text(&app), "が");
}

#[test]
fn standalone_logic_does_not_guard_an_uninterrupted_terminal_pending_composition() {
    use eframe::App as _;
    let ctx = context();
    let mut app = app();
    for _ in 0..4 {
        actual_app_frame(&mut app, &ctx, vec![]);
    }
    actual_app_frame(&mut app, &ctx, vec![preedit("P"), preedit("")]);
    let mut frame = eframe::Frame::_new_kittest();
    let _ = ctx.run_logic(&egui::RawInput { focused: true, ..Default::default() }, |ctx| app.logic(ctx, &mut frame));
    assert!(app.type_ime.resets.is_empty());
    assert!(matches!(app.type_ime.incoming, Some(IncomingComposition::TerminalPending(_))));
    actual_app_frame(&mut app, &ctx, vec![terminal_delete(false), egui::Event::Ime(egui::ImeEvent::Commit("X".into()))]);
    assert_eq!(layer_text(&app), "abX");
}

#[test]
fn fonts_not_ready_and_no_editor_early_returns_retire_the_old_request() {
    use eframe::App as _;
    for no_editor in [false, true] {
        let ctx = context();
        let mut app = app();
        for _ in 0..4 {
            actual_app_frame(&mut app, &ctx, vec![]);
        }
        actual_app_frame(&mut app, &ctx, vec![preedit("P")]);
        let first = actual_app_frame(&mut app, &ctx, vec![raw_key(egui::Key::A, true, false)])
            .platform_output
            .ime
            .expect("custom output")
            .reset_request
            .expect("first request");
        let editor = app.ui.text_edit.clone();
        let before = layer_text(&app);
        let mut frame = eframe::Frame::_new_kittest();
        let mut output = ctx.run_ui(egui::RawInput { focused: true, events: vec![completed(first), preedit("bad")], ..Default::default() }, |ui| {
            if no_editor {
                app.ui.text_edit = None;
            } else {
                app.fonts_ready = false;
            }
            app.ui(ui, &mut frame);
        });
        output.textures_delta.clear();
        assert!(output.platform_output.ime.is_none());
        assert_eq!(pending(&app), None);
        app.ui.text_edit = editor;
        app.fonts_ready = true;
        let second = actual_app_frame(&mut app, &ctx, vec![completed(first), preedit("bad")])
            .platform_output
            .ime
            .expect("same editor returned")
            .reset_request
            .expect("rearmed request");
        assert_ne!(first, second);
        assert_eq!(layer_text(&app), before);
        actual_app_frame(&mut app, &ctx, vec![completed(second), preedit("が")]);
        assert_eq!(layer_text(&app), "が");
    }
}

#[test]
fn pointer_deferred_suffix_cannot_edit_the_old_caret_or_complete_its_old_request() {
    let ctx = context();
    let mut app = app();
    for _ in 0..4 {
        actual_app_frame(&mut app, &ctx, vec![]);
    }
    app.ui.text_edit.as_mut().unwrap().caret = 1;
    app.ui.text_edit.as_mut().unwrap().anchor = 1;
    let point = actual_app_frame(&mut app, &ctx, vec![]).platform_output.ime.expect("caret geometry").cursor_rect.center();
    app.ui.text_edit.as_mut().unwrap().caret = 3;
    app.ui.text_edit.as_mut().unwrap().anchor = 3;
    actual_app_frame(&mut app, &ctx, vec![preedit("P")]);
    let first = actual_app_frame(&mut app, &ctx, vec![raw_key(egui::Key::A, true, false)])
        .platform_output
        .ime
        .expect("selection output")
        .reset_request
        .expect("first request");
    let before = layer_text(&app);
    let revision = app.session.active().unwrap().revision;
    let event = |pressed| egui::Event::PointerButton { pos: point, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    let out = actual_app_frame(
        &mut app,
        &ctx,
        vec![
            egui::Event::PointerMoved(point),
            event(true),
            completed(first),
            egui::Event::Text("bad".into()),
            egui::Event::Paste("bad".into()),
            egui::Event::Cut,
            preedit("bad"),
        ],
    );
    let first_after_pointer = out.platform_output.ime.expect("pointer output").reset_request;
    let out = actual_app_frame(&mut app, &ctx, vec![event(false)]);
    let second = out.platform_output.ime.expect("release output").reset_request.or(first_after_pointer).expect("actual pointer destination reset");
    assert_ne!(first, second);
    assert_eq!(layer_text(&app), before);
    assert_eq!(app.session.active().unwrap().revision, revision);
    assert_eq!(app.ui.text_edit.as_ref().unwrap().caret, 1);
    actual_app_frame(&mut app, &ctx, vec![completed(second), preedit("が")]);
    assert!(layer_text(&app).contains("が"));
}

#[test]
fn completion_without_current_geometry_is_suspended_before_the_following_preedit() {
    let ctx = context();
    let mut app = app();
    let first = interrupt_with_selection(&mut app, &ctx);
    let before = layer_text(&app);
    // No canvas view for this input batch: the completion cannot be attributed to a visible
    // custom descriptor yet. This is the pre-descriptor check, not a wait on final output.
    app.type_ime.view = None;
    let output = run(&mut app, &ctx, vec![completed(first), preedit("bad")]).expect("custom output");
    let second = output.reset_request.expect("the final custom output rearms");
    assert_ne!(first, second);
    assert_eq!(layer_text(&app), before, "the preedit after a geometry-less completion stays blocked");
    assert!(matches!(app.type_ime.resets.get(&egui::ViewportId::ROOT), Some(ResetPhase::Awaiting { id, .. }) if *id == second));
    run(&mut app, &ctx, vec![completed(second), preedit("が")]);
    assert_eq!(layer_text(&app), "が");
}

#[test]
fn pointer_press_without_a_guard_defers_the_same_batch_custom_suffix() {
    let ctx = context();
    let mut app = app();
    let before = layer_text(&app);
    let revision = app.session.active().unwrap().revision;
    let selection = app.ui.text_edit.as_ref().map(|ed| (ed.anchor, ed.caret));
    let press = egui::Event::PointerButton { pos: Pos2::new(20.0, 20.0), button: egui::PointerButton::Primary, pressed: true, modifiers: Default::default() };
    let output = run(
        &mut app,
        &ctx,
        vec![
            egui::Event::PointerMoved(Pos2::new(20.0, 20.0)),
            press,
            egui::Event::Text("bad".into()),
            egui::Event::Paste("bad".into()),
            egui::Event::Cut,
            preedit("bad"),
        ],
    )
    .expect("descriptor");
    assert_eq!(layer_text(&app), before, "the deferred canvas handler never lets a suffix edit the old caret");
    assert_eq!(app.session.active().unwrap().revision, revision);
    assert_eq!(app.ui.text_edit.as_ref().map(|ed| (ed.anchor, ed.caret)), selection);
    assert_eq!(output.reset_request, None, "no prior guard means no speculative reset yet");
}

#[test]
fn synchronous_select_all_accepts_fresh_text_but_still_blocks_stale_ime() {
    let ctx = context();
    // With no guard, the synchronously selected destination is a real destination: Text is
    // an application edit, not an old IME suffix.
    let mut app = app();
    run(&mut app, &ctx, vec![raw_key(egui::Key::A, true, false), egui::Event::Text("x".into())]);
    assert_eq!(layer_text(&app), "x");

    // The same holds when Cmd+A interrupted a known composition: Text is still fresh input.
    let mut app = self::app();
    run(&mut app, &ctx, vec![preedit("P")]);
    let out = run(&mut app, &ctx, vec![raw_key(egui::Key::A, true, false), egui::Event::Text("x".into())]).expect("custom output");
    assert_eq!(layer_text(&app), "x");
    assert!(out.reset_request.is_some(), "the stale composition still needs its ordered reset");

    // An IME payload in that same batch is not fresh Text: it remains blocked until its
    // replacement request is actually completed.
    let mut app = self::app();
    run(&mut app, &ctx, vec![preedit("P")]);
    let before = layer_text(&app);
    let out = run(&mut app, &ctx, vec![raw_key(egui::Key::A, true, false), preedit("bad")]).expect("custom output");
    assert_eq!(layer_text(&app), before);
    let request = out.reset_request.expect("selection reset request");
    run(&mut app, &ctx, vec![completed(request), preedit("が")]);
    assert_eq!(layer_text(&app), "が");
}

struct ResetHarness {
    app: PhotocraftApp,
    stock: bool,
    picker_button: bool,
    search: String,
    search_input: Option<egui::Id>,
    search_focused: bool,
    search_height: Option<f32>,
}

fn reset_harness(app: PhotocraftApp) -> egui_kittest::Harness<'static, ResetHarness> {
    let mut h = egui_kittest::Harness::builder()
        .with_size(egui::vec2(1200.0, 800.0))
        .with_context_setup(|ctx| {
            install_reset_hooks(ctx);
            crate::theme::install_fonts(ctx);
        })
        .build_ui_state(
            |ui, state: &mut ResetHarness| {
                let mut frame = eframe::Frame::_new_kittest();
                state.app.logic_frame(ui.ctx(), &mut frame);
                state.app.ui_frame(ui, &mut frame);
                if state.stock {
                    egui::Window::new("Reset font fixture").show(ui.ctx(), |ui| {
                        let button = ui.make_persistent_id(egui::IdSalt::new("reset-font"));
                        egui::Popup::open_id(ui.ctx(), button.with("popup"));
                        let mut current = "Family 00".to_owned();
                        family_picker_in(
                            ui,
                            "reset-font",
                            &mut current,
                            170.0,
                            &["Family 00".into(), "Family 01".into()],
                            photocraft_engine::prefs::FontPreview::Medium,
                            &[],
                        );
                        let search = ui.scope_id().with(("font-search", "reset-font"));
                        state.search = ui.data(|d| d.get_temp::<String>(search)).unwrap_or_default();
                        let input = search.with("input");
                        state.search_input = Some(input);
                        state.search_focused = ui.memory(|m| m.has_focus(input));
                        state.search_height = ui.data(|d| d.get_temp::<f32>(search.with("height")));
                    });
                }
                if state.picker_button {
                    egui::Window::new("Reset screen fixture").show(ui.ctx(), |ui| {
                        crate::screen_picker::button(ui, egui::Id::unique("reset-screen"));
                    });
                }
                state.app.finish_type_reset_pass(ui.ctx());
            },
            ResetHarness { app, stock: false, picker_button: false, search: String::new(), search_input: None, search_focused: false, search_height: None },
        );
    h.run_steps(4);
    h
}

fn harness_request(h: &egui_kittest::Harness<'_, ResetHarness>) -> egui::ImeResetId {
    h.output().platform_output.ime.as_ref().expect("actual harness output").reset_request.expect("published harness reset")
}

#[test]
fn actual_font_search_owns_stock_input_without_a_speculative_custom_request() {
    let mut h = reset_harness(app());
    h.event(preedit("P"));
    h.step();
    h.event(raw_key(egui::Key::A, true, false));
    h.step();
    let first = harness_request(&h);
    let before = layer_text(&h.state().app);
    let custom_owner = h.output().platform_output.ime.as_ref().unwrap().owner;
    h.state_mut().stock = true;
    h.run_steps(3);
    let stock_owner = h.output().platform_output.ime.as_ref().expect("real font search descriptor").owner;
    assert_ne!(stock_owner, custom_owner);
    let search_input = h.state().search_input.expect("font search field id");
    assert!(h.state().search_focused, "the real font search field, not another stock field, owns focus");
    assert_eq!(h.ctx.memory(|m| m.focused()), Some(search_input), "the stock IME owner is the focused font search input");
    assert!(
        h.state().search_height.is_some_and(|height| height > 0.0 && height <= FONT_MENU_HEIGHT),
        "the real font list measured within its bounded popup height"
    );
    h.input_mut().events = vec![completed(first), egui::Event::Text("Family".into())];
    h.step();
    assert_eq!(h.state().search, "Family", "the actual font picker keeps its text event");
    assert_eq!(layer_text(&h.state().app), before);
    assert_eq!(pending(&h.state().app), None);
    let stock = h.output().platform_output.ime.as_ref().expect("stock output survives finalization");
    assert_eq!(stock.owner, stock_owner);
    assert_eq!(stock.reset_request, None, "an already-ineligible route cannot allocate a custom request");
    h.state_mut().stock = false;
    egui::Popup::close_all(&h.ctx);
    h.ctx.memory_mut(|m| m.stop_text_input());
    h.input_mut().events = vec![completed(first), preedit("bad")];
    h.step();
    let second = harness_request(&h);
    assert_ne!(first, second);
    assert_eq!(layer_text(&h.state().app), before);
    h.step();
    assert_eq!(h.output().platform_output.ime.as_ref().unwrap().reset_request, None);
    h.input_mut().events = vec![completed(second), preedit("が")];
    h.step();
    assert_eq!(layer_text(&h.state().app), "が");
}

type ImmediateViewportRecord = (egui::ViewportId, egui::ViewportId, Option<egui::Vec2>, Option<bool>);

fn record_immediate_viewports() -> Arc<std::sync::Mutex<Vec<ImmediateViewportRecord>>> {
    let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
    let recorded = seen.clone();
    egui::Context::set_immediate_viewport_renderer(move |ctx, immediate| {
        let ids = immediate.ids;
        if let Ok(mut seen) = recorded.lock() {
            seen.push((ids.this, ids.parent, immediate.builder.inner_size, immediate.builder.visible));
        }
        let mut callback = immediate.viewport_ui_cb;
        let mut output = ctx.run_ui(
            egui::RawInput {
                viewport_id: ids.this,
                viewports: std::iter::once((ids.this, egui::ViewportInfo::default())).collect(),
                screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, egui::vec2(2.0, 2.0))),
                ..Default::default()
            },
            |ui| callback(ui),
        );
        output.textures_delta.clear();
    });
    seen
}

#[test]
fn actual_screen_picker_modal_retires_the_type_fence_and_return_rearms_it() {
    use egui_kittest::kittest::Queryable as _;
    let mut app = app();
    app.services.screen_pick = Some(Box::new(|_| {
        let (sender, receiver) = std::sync::mpsc::channel();
        sender
            .send(Ok(crate::screen_picker::Capture::Images(vec![crate::screen_picker::ScreenImage {
                monitor: None,
                position: Pos2::ZERO,
                size: egui::vec2(2.0, 2.0),
                width: 2,
                height: 2,
                rgba: vec![0; 16],
                profile: None,
            }])))
            .unwrap();
        crate::screen_picker::Pending { receiver, cancelled: Arc::new(std::sync::atomic::AtomicBool::new(false)) }
    }));
    let mut h = reset_harness(app);
    h.event(preedit("P"));
    h.step();
    h.event(raw_key(egui::Key::A, true, false));
    h.step();
    let first = harness_request(&h);
    let before = layer_text(&h.state().app);
    let immediate = record_immediate_viewports();
    h.ctx.set_embed_viewports(false);
    h.state_mut().picker_button = true;
    h.run_steps(2);
    h.get_by_label("Pick screen color").click();
    h.run_steps(3);
    assert!(crate::screen_picker::busy(&h.ctx));
    assert!(crate::screen_picker::showing(&h.ctx), "the actual UI takes its screen-picker modal early return");
    let seen = immediate.lock().expect("recorded viewports");
    assert!(!seen.is_empty(), "the native immediate-viewport renderer received the picker");
    assert!(
        seen.iter().all(|(child, parent, size, visible)| *child != egui::ViewportId::ROOT
            && *parent == egui::ViewportId::ROOT
            && *size == Some(egui::vec2(2.0, 2.0))
            && *visible == Some(false)),
        "each picker viewport is a hidden 2x2 child of the root"
    );
    drop(seen);
    h.input_mut().events = vec![completed(first), preedit("bad")];
    h.step();
    assert!(h.output().platform_output.ime.is_none());
    assert_eq!(pending(&h.state().app), None);
    assert_eq!(layer_text(&h.state().app), before);
    crate::screen_picker::cancel(&h.ctx);
    h.state_mut().picker_button = false;
    h.ctx.memory_mut(|m| m.stop_text_input());
    h.input_mut().events = vec![completed(first), preedit("bad")];
    h.step();
    let second = harness_request(&h);
    assert_ne!(first, second);
    assert_eq!(layer_text(&h.state().app), before);
    h.input_mut().events = vec![completed(second), preedit("が")];
    h.step();
    assert_eq!(layer_text(&h.state().app), "が");
}

#[test]
fn a_discarded_custom_output_allocates_no_new_id_and_final_absence_retires_then_rearms() {
    let ctx = context();
    ctx.options_mut(|o| o.max_passes = std::num::NonZeroUsize::new(4).unwrap());
    let mut app = app();
    // A prior queued R1 is deliberately present: the discard must preserve it, not allocate
    // a speculative replacement for output egui will throw away.
    let first = interrupt_with_selection(&mut app, &ctx);
    let mut passes = 0;
    let mut discarded_pending = None;
    let mut output = ctx.run_ui(egui::RawInput { focused: true, ..Default::default() }, |ui| {
        handle_keys(&mut app, ui.ctx());
        if passes == 0 {
            let xf = ViewXform {
                rect: egui::Rect::from_min_size(Pos2::ZERO, egui::vec2(400.0, 300.0)),
                zoom: 1.0,
                center: [100.0, 100.0],
                flip: false,
                rotation: 0.0,
                aspect: 1.0,
            };
            draw_overlay(&mut app, ui.painter(), &xf);
            ui.ctx().request_discard("reset custom output disappears on final pass");
        }
        sync_ime(&mut app, ui.ctx());
        finish_reset_pass(&mut app, ui.ctx());
        if passes == 0 {
            discarded_pending = pending(&app);
        }
        passes += 1;
    });
    output.textures_delta.clear();
    assert!(passes > 1);
    assert!(output.platform_output.ime.is_none());
    assert_eq!(discarded_pending, Some(first), "discard keeps the queued R1 and allocates no NEW id");
    assert_eq!(pending(&app), None, "the final ineligible pass retires the prior R1");
    let before = layer_text(&app);
    let returned = run(&mut app, &ctx, vec![completed(first), preedit("bad")]).expect("same model returns");
    let second = returned.reset_request.expect("new published request");
    assert_ne!(first, second);
    assert_eq!(layer_text(&app), before);
    run(&mut app, &ctx, vec![completed(second), preedit("が")]);
    assert_eq!(layer_text(&app), "が");
}

#[test]
fn a_recreated_context_installed_before_run_ui_rejects_the_old_completion_and_rearms() {
    use eframe::App as _;
    let old = context();
    let mut app = app();
    for _ in 0..4 {
        actual_app_frame(&mut app, &old, vec![]);
    }
    let first = interrupt_with_selection(&mut app, &old);
    let before = layer_text(&app);
    // The host replaces its Context and must run the same supported setup before `run_ui`;
    // a begin-pass hook registered later could not mark the first pass.
    let next = egui::Context::default();
    PhotocraftApp::setup_context(&next, app.ui.theme);
    let mut frame = eframe::Frame::_new_kittest();
    let mut saw_ui_pass = false;
    let mut output = next.run_ui(
        egui::RawInput {
            focused: true,
            events: vec![completed(first), preedit("bad")],
            screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, egui::vec2(1200.0, 800.0))),
            ..Default::default()
        },
        |ui| {
            app.logic(ui.ctx(), &mut frame);
            saw_ui_pass = in_ui_pass(ui.ctx());
            app.ui(ui, &mut frame);
        },
    );
    output.textures_delta.clear();
    assert!(saw_ui_pass, "the supported creator setup marks the first replacement Context pass");
    let second = output.platform_output.ime.expect("replacement Context custom output").reset_request.expect("replacement Context request");
    assert_ne!(first, second);
    assert_eq!(layer_text(&app), before);
    run(&mut app, &next, vec![completed(second), preedit("が")]);
    assert_eq!(layer_text(&app), "が");
}

#[test]
fn type_mask_and_same_owner_session_replacement_supersede_the_full_request() {
    for mask in [false, true] {
        let ctx = context();
        let mut app = app();
        let first = interrupt_with_selection(&mut app, &ctx);
        let original = run(&mut app, &ctx, vec![]).expect("original context");
        if mask {
            app.ui.text_edit.as_mut().unwrap().mask = true;
        } else {
            app.ui.text_edit.as_mut().unwrap().session = "replacement-reset-session".into();
        }
        let before = layer_text(&app);
        let changed = run(&mut app, &ctx, vec![completed(first), preedit("bad")]).expect("replacement output");
        let second = changed.reset_request.expect("replacement request");
        assert_ne!(first, second);
        if mask {
            assert_ne!(changed.owner, original.owner);
        } else {
            assert_eq!(changed.owner, original.owner);
        }
        assert_ne!(changed.context_generation, original.context_generation);
        assert_eq!(layer_text(&app), before);
        run(&mut app, &ctx, vec![completed(first), terminal_delete(false), egui::Event::Ime(egui::ImeEvent::Commit("bad".into()))]);
        assert_eq!(layer_text(&app), before);
        run(&mut app, &ctx, vec![completed(second), preedit("が")]);
        assert_eq!(layer_text(&app), "が");
    }
}

#[test]
fn document_replacement_cannot_reuse_the_previous_documents_reset() {
    let ctx = context();
    let mut app = app();
    let first = interrupt_with_selection(&mut app, &ctx);
    let old_document = active_doc_id(&app);
    app.run("file.new", json!({"width": 400, "height": 200})).unwrap();
    app.sync_views();
    // The host runs this before handling input: a new active document ends the old edit in
    // its own document, so the next canvas click starts an editor for the new one.
    end_if_left(&mut app);
    assert!(app.ui.text_edit.is_none(), "the new document starts without the previous edit target");
    app.ui.tool = crate::state::Tool::Type;
    pointer_up(&mut app, [50.0, 100.0], [50.0, 100.0]);
    insert(&mut app, "new");
    assert_ne!(active_doc_id(&app), old_document);
    let before = layer_text(&app);
    let changed = run(&mut app, &ctx, vec![completed(first), preedit("bad")]).expect("new document output");
    let second = changed.reset_request.expect("new document reset");
    assert_ne!(first, second);
    assert_eq!(layer_text(&app), before);
    run(&mut app, &ctx, vec![completed(second), preedit("が")]);
    assert!(layer_text(&app).contains("が"));
}
