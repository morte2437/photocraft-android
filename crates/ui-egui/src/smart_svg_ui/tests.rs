use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

use photocraft_engine::Session;

use super::*;
use crate::Services;

const SVG: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" width="12" height="8"><rect width="12" height="8" fill="red"/></svg>"#;

type Saves = Rc<RefCell<VecDeque<Result<Vec<u8>, String>>>>;
type Calls = Rc<RefCell<Vec<ExternalSvgEditor>>>;

struct TestSession(Saves);

impl ExternalSvgSession for TestSession {
    fn poll(&mut self) -> Option<Result<Vec<u8>, String>> {
        self.0.borrow_mut().pop_front()
    }
}

struct NativeTestSession(Saves);
impl ExternalSvgSession for NativeTestSession {
    fn poll(&mut self) -> Option<Result<Vec<u8>, String>> {
        self.0.borrow_mut().pop_front()
    }
    fn poll_vector(&mut self) -> Option<Result<crate::ExternalVectorSave, String>> {
        self.poll().map(|r| r.map(|bytes| crate::ExternalVectorSave { bytes, preview_svg: Some(SVG.as_bytes().to_vec()) }))
    }
}

fn fixture() -> (PhotocraftApp, Saves, Calls) {
    let saves = Saves::default();
    let calls = Calls::default();
    let (queue, launched) = (saves.clone(), calls.clone());
    let services = Services {
        open_external_svg: Some(Box::new(move |editor, bytes| {
            assert_eq!(&*bytes.bytes, SVG.as_bytes());
            launched.borrow_mut().push(editor);
            Ok(Box::new(TestSession(queue.clone())))
        })),
        ..Default::default()
    };
    let mut app = PhotocraftApp::new(Session::new(), services);
    app.session.execute("file.new", json!({"width":64,"height":64})).unwrap();
    photocraft_engine::file_cmds::place_bytes(&mut app.session, "Artwork.svg", SVG.as_bytes().to_vec(), None, &json!({"smartObject":true})).unwrap();
    app.sync_views();
    (app, saves, calls)
}

fn open(app: &mut PhotocraftApp) {
    assert!(intercept(app, "layer.smartObjects.editContents", &json!({})).unwrap().unwrap()["deferred"].as_bool().unwrap());
}

#[test]
fn control_menu_invoke_preserves_svg_editor_choice_and_rejects_other_dialogs() {
    let (mut app, saves, calls) = fixture();
    open(&mut app);
    let pending = app.smart_svg.pending.as_mut().unwrap();
    pending.editor = Editor::Program;
    pending.program = Some("synthetic-editor".into());
    let before = app.session.active().unwrap().doc.clone();
    let revision = app.session.active().unwrap().revision;
    let history = app.session.active().unwrap().history.past_len();
    let journal = app.session.journal.clone();
    let view = (app.ui.views[0].center, app.ui.views[0].zoom);
    let pending = app.smart_svg.pending.as_ref().unwrap();
    let target = (pending.target.doc, pending.target.layer, pending.target.contents_id.clone(), pending.target.fingerprint.clone());
    let svg = pending.target.svg.clone();
    let program = pending.program.clone();
    let internal_once = app.smart_svg.internal_once;
    let authorizations = Rc::new(std::cell::Cell::new(0));
    let authorized = authorizations.clone();
    app.services.automation_command = Some(Box::new(move |_, _| {
        authorized.set(authorized.get() + 1);
        Ok(())
    }));
    let clipboard_reads = Rc::new(std::cell::Cell::new(0));
    let reads = clipboard_reads.clone();
    app.services.clipboard_get_image = Some(Box::new(move || {
        reads.set(reads.get() + 1);
        Ok(None)
    }));
    let ctx = egui::Context::default();
    for id in ["layer.duplicate", "image.imageSize", "edit.paste", "view.zoomIn"] {
        let (request, _) = crate::control::ControlRequest::new("ui.menu.invoke", json!({"id": id}));
        let crate::control::Outcome::Done(result) = crate::control::handle(&mut app, &ctx, &request) else {
            panic!("blocked menu call must return immediately")
        };
        assert_eq!(result["ok"], false, "{id}: {result}");
        assert!(result.to_string().contains("modal dialog"));
        assert_eq!(authorizations.get(), 0);
        assert_eq!(clipboard_reads.get(), 0);
        assert!(calls.borrow().is_empty());
        assert!(saves.borrow().is_empty());
        assert!(app.smart_svg.editing.is_empty());
        assert_eq!(app.smart_svg.internal_once, internal_once);
        assert!(app.ui.dialogs.is_empty());
        let state = app.session.active().unwrap();
        assert_eq!(state.doc, before);
        assert_eq!(state.revision, revision);
        assert_eq!(state.history.past_len(), history);
        assert_eq!(app.session.journal, journal);
        assert_eq!((app.ui.views[0].center, app.ui.views[0].zoom), view);
        let pending = app.smart_svg.pending.as_ref().unwrap();
        assert_eq!((pending.target.doc, pending.target.layer, pending.target.contents_id.clone(), pending.target.fingerprint.clone()), target);
        assert!(Arc::ptr_eq(&pending.target.svg, &svg));
        assert_eq!(&**pending.target.svg, SVG.as_bytes());
        assert!(pending.editor == Editor::Program);
        assert_eq!(pending.program, program);
    }
}

#[test]
fn routing_keeps_automation_non_svg_and_unrelated_commands_internal() {
    let (mut app, _, calls) = fixture();
    assert!(intercept(&mut app, "edit.copy", &json!({})).is_none());
    assert!(intercept(&mut app, "layer.smartObjects.editContents", &json!({"other":true})).is_none());
    app.automation_input = true;
    assert!(intercept(&mut app, "layer.smartObjects.editContents", &json!({})).is_none());
    app.automation_input = false;
    let st = app.session.active_mut().unwrap();
    let layer = st.active_layer.unwrap();
    let LayerContent::Smart(smart) = &mut Arc::make_mut(&mut st.doc).layer_mut(layer).unwrap().content else { panic!("smart") };
    smart.source = SmartSource::Embedded { file_name: "Embedded.pcraft".into(), bytes: Arc::new(vec![]) };
    assert!(intercept(&mut app, "layer.smartObjects.editContents", &json!({})).is_none());
    assert!(calls.borrow().is_empty());
}

#[test]
fn svg_source_without_svg_filename_still_opens_editor_choice() {
    let (mut app, _, calls) = fixture();
    let st = app.session.active_mut().unwrap();
    let layer = st.active_layer.unwrap();
    let LayerContent::Smart(smart) = &mut Arc::make_mut(&mut st.doc).layer_mut(layer).unwrap().content else { panic!("smart") };
    let SmartSource::Embedded { file_name, .. } = &mut smart.source else { panic!("embedded") };
    *file_name = "Unknown clipboard object".into();
    open(&mut app);
    assert!(calls.borrow().is_empty());
}

#[test]
fn prompt_and_escape_never_launch_or_change_document() {
    let (mut app, _, calls) = fixture();
    let before = app.session.active().unwrap().doc.clone();
    open(&mut app);
    open(&mut app);
    let ctx = egui::Context::default();
    PhotocraftApp::setup_context(&ctx, Default::default());
    let mut input = egui::RawInput::default();
    input.events.push(egui::Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE });
    let mut output = ctx.run_ui(input, |ui| show(&mut app, ui.ctx()));
    output.textures_delta.clear();
    assert!(!app.smart_svg.modal_open());
    assert!(calls.borrow().is_empty());
    assert_eq!(app.session.active().unwrap().doc, before);
}

#[test]
fn manual_internal_choice_uses_existing_child_document_flow() {
    let (mut app, _, calls) = fixture();
    let parent = app.session.active().unwrap().doc.id;
    open(&mut app);
    app.smart_svg.pending.as_mut().unwrap().editor = Editor::Internal;
    confirm(&mut app).unwrap();
    assert_ne!(app.session.active().unwrap().doc.id, parent);
    assert_eq!(app.session.smart_links.len(), 1);
    assert!(calls.borrow().is_empty());
    assert!(!app.smart_svg.internal_once);
}

/// Edit Contents again on contents already open in the internal editor switches to that
/// document, as it does for any smart object, instead of an error.
#[test]
fn edit_contents_again_switches_to_the_open_internal_copy() {
    let (mut app, _, calls) = fixture();
    let parent = app.session.active_index().unwrap();
    open(&mut app);
    app.smart_svg.pending.as_mut().unwrap().editor = Editor::Internal;
    confirm(&mut app).unwrap();
    let child = app.session.active().unwrap().doc.id;
    app.session.set_active(parent);
    app.run("layer.smartObjects.editContents", json!({})).unwrap();
    assert_eq!(app.session.active().unwrap().doc.id, child);
    assert_eq!(app.session.smart_links.len(), 1);
    assert!(app.smart_svg.pending.is_none());
    assert!(!app.ui.status_error, "{}", app.ui.status);
    assert!(calls.borrow().is_empty());
}

#[test]
fn missing_external_service_keeps_internal_edit_and_save_back() {
    let (mut app, saves, calls) = fixture();
    app.services.open_external_svg = None;
    let parent_index = app.session.active_index().unwrap();
    let st = app.session.active().unwrap();
    let (parent, layer, before, history) = (st.doc.id, st.active_layer.unwrap(), st.doc.clone(), st.history.past_len());
    let opened = app.run("layer.smartObjects.editContents", json!({})).unwrap();
    assert_eq!(opened["document"].as_u64(), app.session.active_index().map(|index| index as u64));
    assert_ne!(app.session.active().unwrap().doc.id, parent);
    assert_eq!(app.session.smart_links.len(), 1);
    let link = &app.session.smart_links[0];
    assert_eq!((link.parent, link.layer, link.child), (parent, layer, app.session.active().unwrap().doc.id));
    assert!(!app.smart_svg.modal_open());
    assert!(app.smart_svg.editing.is_empty());
    assert!(calls.borrow().is_empty());
    assert!(saves.borrow().is_empty());

    app.run("layer.new.layer", json!({})).unwrap();
    app.session
        .edit("paint child", |doc, active| {
            let bounds = doc.bounds();
            doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap().fill_rect(bounds, &[0.0, 1.0, 0.0, 1.0]);
            Ok(())
        })
        .unwrap();
    app.run("layer.smartObjects.saveContents", json!({})).unwrap();
    assert!(!app.session.active().unwrap().is_dirty());
    app.session.set_active(parent_index);
    let st = app.session.active().unwrap();
    let LayerContent::Smart(smart) = &st.doc.layer(layer).unwrap().content else { panic!("smart") };
    let SmartSource::Embedded { file_name, bytes } = &smart.source else { panic!("embedded") };
    assert!(file_name.ends_with(".pcraft"));
    assert_ne!(&**bytes, SVG.as_bytes());
    assert_eq!(smart.cache.as_ref().unwrap().rgba(32, 32), [0.0, 1.0, 0.0, 1.0]);
    assert_eq!(st.history.past_len(), history + 1);
    assert!(!app.smart_svg.modal_open());
    assert!(calls.borrow().is_empty());
    assert!(saves.borrow().is_empty());
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(app.session.active().unwrap().doc, before);
}

#[test]
fn choosing_internal_editor_ends_existing_external_session() {
    let (mut app, _, _) = fixture();
    open(&mut app);
    confirm(&mut app).unwrap();
    assert_eq!(app.smart_svg.editing.len(), 1);
    open(&mut app);
    app.smart_svg.pending.as_mut().unwrap().editor = Editor::Internal;
    confirm(&mut app).unwrap();
    assert!(app.smart_svg.editing.is_empty());
    assert_eq!(app.session.smart_links.len(), 1);
}

#[test]
fn failed_launch_is_visible_and_keeps_source_unchanged() {
    let (mut app, _, _) = fixture();
    app.services.open_external_svg = Some(Box::new(|_, _| Err("Editor not installed".into())));
    let before = app.session.active().unwrap().doc.clone();
    open(&mut app);
    assert_eq!(confirm(&mut app).unwrap_err(), "Editor not installed");
    assert!(app.smart_svg.editing.is_empty());
    assert_eq!(app.session.active().unwrap().doc, before);
}

#[test]
fn selected_editor_and_repeated_saves_update_with_undo_and_preserve_selection() {
    let (mut app, saves, calls) = fixture();
    let before = app.session.active().unwrap().doc.clone();
    let target = app.session.active().unwrap().active_layer;
    open(&mut app);
    app.smart_svg.pending.as_mut().unwrap().editor = Editor::Illustrator;
    confirm(&mut app).unwrap();
    assert!(matches!(calls.borrow().first(), Some(ExternalSvgEditor::Illustrator)));
    app.session.execute("layer.new.layer", json!({})).unwrap();
    let selection = app.session.active().unwrap().active_layer;
    let ctx = egui::Context::default();
    saves.borrow_mut().push_back(Ok(SVG.replace("red", "blue").into_bytes()));
    poll(&mut app, &ctx);
    let blue = app.session.active().unwrap().doc.clone();
    assert_ne!(blue, before);
    assert_ne!(selection, target);
    assert_eq!(app.session.active().unwrap().active_layer, selection);
    saves.borrow_mut().push_back(Ok(SVG.replace("red", "green").into_bytes()));
    poll(&mut app, &ctx);
    assert_ne!(app.session.active().unwrap().doc, blue);
    assert_eq!(app.smart_svg.editing.len(), 1);
    app.session.execute("edit.undo", json!({})).unwrap();
    assert_eq!(app.session.active().unwrap().doc, blue);
    app.session.execute("edit.undo", json!({})).unwrap();
    app.session.execute("edit.undo", json!({})).unwrap();
    assert_eq!(app.session.active().unwrap().doc, before);
}

#[test]
fn saved_changes_wait_on_document_switch_and_active_preview() {
    let (mut app, saves, _) = fixture();
    let parent = app.session.active().unwrap().doc.id;
    open(&mut app);
    confirm(&mut app).unwrap();
    app.session.execute("file.new", json!({"width":32,"height":32})).unwrap();
    let other = app.session.active().unwrap().doc.clone();
    saves.borrow_mut().push_back(Ok(SVG.replace("red", "blue").into_bytes()));
    let ctx = egui::Context::default();
    poll(&mut app, &ctx);
    assert_eq!(app.session.active().unwrap().doc, other);
    assert!(app.smart_svg.editing.first().unwrap().saved.is_some());
    let index = app.session.documents().iter().position(|st| st.doc.id == parent).unwrap();
    app.session.set_active(index);
    app.ui.open_dialog(crate::state::DialogKind::Command, serde_json::Map::new());
    poll(&mut app, &ctx);
    assert!(app.smart_svg.editing.first().unwrap().saved.is_some());
    app.ui.dialogs.clear();
    poll(&mut app, &ctx);
    assert!(app.smart_svg.editing.first().unwrap().saved.is_none());
}

#[test]
fn changed_source_stops_syncing_instead_of_overwriting_user_edit() {
    let (mut app, saves, _) = fixture();
    open(&mut app);
    confirm(&mut app).unwrap();
    let target = &app.smart_svg.editing.first().unwrap().target;
    app.session.execute("layer.smartObjects.updateSvg", json!({"document": target.doc.0,"layer": target.layer.0,"contentsId":target.contents_id,"fingerprint":target.fingerprint,"svg":SVG.replace("red","yellow")})).unwrap();
    let before = app.session.active().unwrap().doc.clone();
    saves.borrow_mut().push_back(Ok(SVG.replace("red", "blue").into_bytes()));
    poll(&mut app, &egui::Context::default());
    assert!(app.smart_svg.editing.is_empty());
    assert!(app.ui.status_error);
    assert_eq!(app.session.active().unwrap().doc, before);
}

#[test]
fn stale_prompt_never_launches_into_another_document_or_changed_source() {
    for switched in [false, true] {
        let (mut app, _, calls) = fixture();
        open(&mut app);
        if switched {
            app.session.execute("file.new", json!({"width":32,"height":32})).unwrap();
        } else {
            let target = &app.smart_svg.pending.as_ref().unwrap().target;
            app.session.execute("layer.smartObjects.updateSvg", json!({"document":target.doc.0,"layer":target.layer.0,"contentsId":target.contents_id,"fingerprint":target.fingerprint,"svg":SVG.replace("red","blue")})).unwrap();
        }
        assert!(confirm(&mut app).is_err());
        assert!(calls.borrow().is_empty());
    }
}

#[test]
fn malformed_and_transient_saves_can_recover_and_deleted_layer_stops_session() {
    let (mut app, saves, _) = fixture();
    open(&mut app);
    confirm(&mut app).unwrap();
    let before = app.session.active().unwrap().doc.clone();
    saves.borrow_mut().push_back(Ok(vec![255]));
    poll(&mut app, &egui::Context::default());
    assert_eq!(app.smart_svg.editing.len(), 1);
    assert_eq!(app.session.active().unwrap().doc, before);
    saves.borrow_mut().push_back(Err("Editor is replacing its SVG file".into()));
    poll(&mut app, &egui::Context::default());
    assert_eq!(app.smart_svg.editing.len(), 1);
    saves.borrow_mut().push_back(Ok(b"<svg><".to_vec()));
    poll(&mut app, &egui::Context::default());
    assert_eq!(app.smart_svg.editing.len(), 1);
    assert_eq!(app.session.active().unwrap().doc, before);
    saves.borrow_mut().push_back(Ok(SVG.replace("red", "blue").into_bytes()));
    poll(&mut app, &egui::Context::default());
    assert_ne!(app.session.active().unwrap().doc, before);
    let st = app.session.active_mut().unwrap();
    Arc::make_mut(&mut st.doc).layers.clear();
    poll(&mut app, &egui::Context::default());
    assert!(app.smart_svg.editing.is_empty());
}

#[test]
fn custom_executable_requires_explicit_selection() {
    let (mut app, _, calls) = fixture();
    open(&mut app);
    app.smart_svg.pending.as_mut().unwrap().editor = Editor::Program;
    assert!(confirm(&mut app).is_err());
    assert!(calls.borrow().is_empty());
    open(&mut app);
    let pending = app.smart_svg.pending.as_mut().unwrap();
    pending.editor = Editor::Program;
    pending.program = Some("C:/synthetic/editor.exe".into());
    confirm(&mut app).unwrap();
    assert!(matches!(calls.borrow().first(), Some(ExternalSvgEditor::Program(path)) if path == "C:/synthetic/editor.exe"));
}

#[test]
fn idle_gradient_tool_does_not_block_svg_editing() {
    let (mut app, _, calls) = fixture();
    app.ui.tool = crate::state::Tool::Gradient;
    open(&mut app);
    confirm(&mut app).unwrap();
    assert_eq!(calls.borrow().len(), 1);
}

#[test]
fn saved_changes_wait_on_idle_cam_box_then_resume_after_cancel() {
    let (mut app, saves, calls) = fixture();
    let smart_layer = app.session.active().unwrap().active_layer.unwrap();
    open(&mut app);
    confirm(&mut app).unwrap();
    app.session.execute("layer.new.layer", json!({})).unwrap();
    app.session.execute("select.rect", json!({"x":8,"y":8,"width":12,"height":8})).unwrap();
    app.ui.tool = crate::state::Tool::ContentAwareMove;
    assert!(crate::cam_transform::begin(&mut app, [8, 4]));
    assert!(app.drag.is_none() && app.patch_preview.is_none() && app.ui.transform.is_none());
    let before = app.session.active().unwrap().doc.clone();
    let revision = app.session.active().unwrap().revision;
    let history = app.session.active().unwrap().history.past_len();
    let raster_layer = app.session.active().unwrap().active_layer.unwrap();
    let cam = app.ui.cam_transform.clone();
    let blue = SVG.replace("red", "blue");
    saves.borrow_mut().push_back(Ok(blue.as_bytes().to_vec()));
    let ctx = egui::Context::default();

    poll(&mut app, &ctx);
    assert_eq!(app.session.active().unwrap().doc, before);
    assert_eq!(app.session.active().unwrap().revision, revision);
    assert_eq!(app.session.active().unwrap().history.past_len(), history);
    assert_eq!(app.ui.cam_transform, cam);
    assert!(app.smart_svg.editing.first().unwrap().saved.is_some());
    assert!(saves.borrow().is_empty());

    crate::cam_transform::cancel(&mut app);
    poll(&mut app, &ctx);
    let st = app.session.active().unwrap();
    let LayerContent::Smart(smart) = &st.doc.layer(smart_layer).unwrap().content else { panic!("smart") };
    let SmartSource::Embedded { bytes, .. } = &smart.source else { panic!("embedded") };
    assert_eq!(&**bytes, blue.as_bytes());
    assert_eq!(st.doc.layer(raster_layer), before.layer(raster_layer));
    assert_eq!(st.doc.selection, before.selection);
    assert_eq!(st.history.past_len(), history + 1);
    assert!(app.ui.cam_transform.is_none());
    assert!(app.smart_svg.editing.first().unwrap().saved.is_none());
    assert_eq!(app.smart_svg.editing.len(), 1);
    assert_eq!(calls.borrow().len(), 1);
}

#[test]
fn saved_changes_wait_on_mask_target_then_apply_without_another_save() {
    let (mut app, saves, _) = fixture();
    open(&mut app);
    confirm(&mut app).unwrap();
    let before = app.session.active().unwrap().doc.clone();
    app.ui.mask_target = true;
    saves.borrow_mut().push_back(Ok(SVG.replace("red", "blue").into_bytes()));
    let ctx = egui::Context::default();
    poll(&mut app, &ctx);
    assert_eq!(app.session.active().unwrap().doc, before);
    assert!(app.smart_svg.editing.first().unwrap().saved.is_some());
    app.ui.mask_target = false;
    poll(&mut app, &ctx);
    assert_ne!(app.session.active().unwrap().doc, before);
}

#[test]
fn saved_changes_wait_on_implicit_mask_then_resume_without_another_save() {
    let (mut app, saves, calls) = fixture();
    open(&mut app);
    confirm(&mut app).unwrap();
    let st = app.session.active_mut().unwrap();
    let layer = st.active_layer.unwrap();
    Arc::make_mut(&mut st.doc).layer_mut(layer).unwrap().mask = Some(photocraft_doc::LayerMask::reveal_all());
    st.channel_view.color_hidden = vec![true; photocraft_engine::channel_cmds::color_count(&st.doc)];
    st.channel_view.alpha_visible.clear();
    assert_eq!(st.channel_view.target, photocraft_engine::channel_cmds::ChannelTarget::Composite);
    assert!(st.channel_view.layer_mask.is_none());
    let before = st.doc.clone();
    let revision = st.revision;
    assert!(!app.ui.mask_target);
    assert!(!app.session.tools.mask_targeted);
    assert!(app.layer_mask_targeted());
    let blue = SVG.replace("red", "blue");
    saves.borrow_mut().push_back(Ok(blue.as_bytes().to_vec()));
    let ctx = egui::Context::default();

    poll(&mut app, &ctx);
    assert_eq!(app.session.active().unwrap().doc, before);
    assert_eq!(app.session.active().unwrap().revision, revision);
    assert!(app.smart_svg.editing.first().unwrap().saved.is_some());
    assert!(saves.borrow().is_empty());

    app.session.active_mut().unwrap().channel_view.color_hidden.clear();
    assert!(!app.layer_mask_targeted());
    poll(&mut app, &ctx);
    let st = app.session.active().unwrap();
    assert_ne!(st.doc, before);
    assert_eq!(st.doc.layer(layer).unwrap().mask, before.layer(layer).unwrap().mask);
    let LayerContent::Smart(smart) = &st.doc.layer(layer).unwrap().content else { panic!("smart") };
    let SmartSource::Embedded { bytes, .. } = &smart.source else { panic!("embedded") };
    assert_eq!(&**bytes, blue.as_bytes());
    assert!(app.smart_svg.editing.first().unwrap().saved.is_none());
    assert_eq!(calls.borrow().len(), 1);
}

#[test]
fn explicit_mask_channel_roundtrip_resumes_queued_svg_save_with_masks_preserved() {
    for channel in ["mask", "vectorMask"] {
        let (mut app, saves, calls) = fixture();
        open(&mut app);
        confirm(&mut app).unwrap();
        app.session.execute("layer.layerMask.revealAll", json!({})).unwrap();
        app.session.execute("layer.vectorMask.revealAll", json!({})).unwrap();
        let before = app.session.active().unwrap().doc.clone();
        let layer = app.session.active().unwrap().active_layer.unwrap();

        app.run("channel.target", json!({"channel":channel})).unwrap();
        assert_eq!(app.ui.mask_target, channel == "mask");
        assert_eq!(app.ui.vector_mask_target, channel == "vectorMask");
        let blue = SVG.replace("red", "blue");
        saves.borrow_mut().push_back(Ok(blue.as_bytes().to_vec()));
        let ctx = egui::Context::default();
        poll(&mut app, &ctx);
        assert_eq!(app.session.active().unwrap().doc, before);
        assert!(app.smart_svg.editing.first().unwrap().saved.is_some());
        assert!(saves.borrow().is_empty());

        app.run("channel.target", json!({"channel":"composite"})).unwrap();
        assert!(!app.ui.mask_target);
        assert!(!app.ui.vector_mask_target);
        assert!(!app.session.tools.mask_targeted);
        poll(&mut app, &ctx);
        let st = app.session.active().unwrap();
        let updated = st.doc.layer(layer).unwrap();
        assert_eq!(updated.mask, before.layer(layer).unwrap().mask);
        assert_eq!(updated.vector_mask, before.layer(layer).unwrap().vector_mask);
        let LayerContent::Smart(smart) = &updated.content else { panic!("smart") };
        let SmartSource::Embedded { bytes, .. } = &smart.source else { panic!("embedded") };
        assert_eq!(&**bytes, blue.as_bytes());
        assert!(app.smart_svg.editing.first().unwrap().saved.is_none());
        assert_eq!(app.smart_svg.editing.len(), 1);
        assert_eq!(calls.borrow().len(), 1);
    }
}

#[test]
fn active_gradient_preview_defers_save_until_gesture_ends() {
    let (mut app, saves, _) = fixture();
    open(&mut app);
    confirm(&mut app).unwrap();
    let before = app.session.active().unwrap().doc.clone();
    app.ui.tool = crate::state::Tool::Gradient;
    app.ui.tool_options.gradient_classic = false;
    assert!(crate::gradient_ui::pointer(&mut app, crate::canvas::ToolEvent::Down { x: 0.0, y: 0.0, pressure: 1.0 }, egui::Modifiers::NONE));
    assert!(crate::gradient_ui::editing(&app));
    saves.borrow_mut().push_back(Ok(SVG.replace("red", "blue").into_bytes()));
    let ctx = egui::Context::default();
    poll(&mut app, &ctx);
    assert_eq!(app.session.active().unwrap().doc, before);
    app.gradient = Default::default();
    poll(&mut app, &ctx);
    assert_ne!(app.session.active().unwrap().doc, before);
}

#[test]
fn bounded_sessions_require_reuse_or_closing_a_document() {
    let (mut app, _, calls) = fixture();
    for _ in 0..MAX_SESSIONS {
        open(&mut app);
        confirm(&mut app).unwrap();
        photocraft_engine::file_cmds::place_bytes(&mut app.session, "Artwork.svg", SVG.as_bytes().to_vec(), None, &json!({"smartObject":true})).unwrap();
    }
    open(&mut app);
    assert!(confirm(&mut app).unwrap_err().contains("limit: 8"));
    assert_eq!(calls.borrow().len(), MAX_SESSIONS);
    assert_eq!(app.smart_svg.editing.len(), MAX_SESSIONS);
}

#[test]
fn capture_smart_svg_visual_fixtures() {
    let Some(directory) = std::env::var_os("CRAFT_SMART_SVG_FIXTURES").map(std::path::PathBuf::from) else { return };
    std::fs::create_dir_all(&directory).unwrap();
    for theme in [crate::theme::ThemeKind::Pro, crate::theme::ThemeKind::StudioLight] {
        let (mut app, _, _) = fixture();
        open(&mut app);
        let mut ready = false;
        let lang = crate::i18n::lang_from_tag("en").unwrap();
        let mut harness = egui_kittest::Harness::builder().with_size(egui::vec2(900.0, 620.0)).wgpu().build_ui_state(
            move |ui, app: &mut PhotocraftApp| {
                let _language = crate::i18n::language_scope(lang);
                if !ready {
                    PhotocraftApp::setup_context(ui.ctx(), theme);
                    ready = true;
                    return;
                }
                show(app, ui.ctx());
            },
            app,
        );
        harness.ctx.set_os(egui::os::OperatingSystem::Windows);
        harness.run_steps(5);
        harness.render().unwrap().save(directory.join(format!("photocraft-smart-svg-editor-{}-en.png", theme.id()))).unwrap();
    }
}

/// An AI payload in the PSD link table must reach explicit editor selection without becoming
/// a filesystem path. A saved SVG replaces it through the normal optimistic transaction.
#[test]
fn embedded_ai_link_table_handoff_preserves_source_until_saved_and_undo_restores_it() {
    let (mut app, saves, calls) = fixture();
    let id = app.session.active().unwrap().active_layer.unwrap();
    let original = b"%PDF-1.6\nsynthetic vector".to_vec();
    {
        let doc = Arc::make_mut(&mut app.session.active_mut().unwrap().doc);
        let payload = photocraft_io::linked::encode_linked_file(&photocraft_io::linked::LinkedFile {
            uuid: "embedded-ai".into(),
            file_name: "Logo.ai".into(),
            bytes: original.clone(),
        });
        doc.metadata.psd_global_blocks.push((*b"8BIM", *b"lnk2", Arc::new(payload)));
        let LayerContent::Smart(sm) = &mut doc.layer_mut(id).unwrap().content else { panic!("smart") };
        sm.source = SmartSource::Linked { path: "embedded-ai".into() };
    }
    let before = app.session.active().unwrap().doc.clone();
    let queue = saves.clone();
    app.services.open_external_svg = Some(Box::new(move |editor, source| {
        assert_eq!(editor, ExternalSvgEditor::VectorCraft);
        assert_eq!(*source.bytes, original);
        assert_eq!(source.extension, "ai");
        Ok(Box::new(NativeTestSession(queue.clone())))
    }));
    open(&mut app);
    assert!(calls.borrow().is_empty());
    confirm(&mut app).unwrap();
    assert_eq!(app.session.active().unwrap().doc, before);
    saves.borrow_mut().push_back(Ok(b"%PDF-1.6\nedited native AI".to_vec()));
    poll(&mut app, &egui::Context::default());
    let layer = app.session.active().unwrap().doc.layer(id).unwrap();
    assert!(
        matches!(&layer.content,LayerContent::Smart(sm) if matches!(&sm.source,SmartSource::Embedded {file_name,bytes} if file_name=="Logo.ai" && bytes.as_slice()==b"%PDF-1.6\nedited native AI"))
    );
    assert!(app.session.undo());
    assert_eq!(app.session.active().unwrap().doc, before);
}

/// Optional screenshot uses authored bytes and mock services, never private artwork or an editor.
#[test]
fn capture_embedded_vector_conversion_prompt() {
    let Some(directory) = std::env::var_os("CRAFT_SMART_SVG_FIXTURES").map(std::path::PathBuf::from) else { return };
    std::fs::create_dir_all(&directory).unwrap();
    let (mut app, _, _) = fixture();
    let state = app.session.active_mut().unwrap();
    let id = state.active_layer.unwrap();
    let LayerContent::Smart(sm) = &mut Arc::make_mut(&mut state.doc).layer_mut(id).unwrap().content else { panic!("smart") };
    sm.source = SmartSource::Embedded { file_name: "Authored.ai".into(), bytes: Arc::new(b"%PDF-1.6\nsynthetic vector prompt".to_vec()) };
    open(&mut app);
    let mut ready = false;
    let lang = crate::i18n::lang_from_tag("en").unwrap();
    let mut harness = egui_kittest::Harness::builder().with_size(egui::vec2(900.0, 620.0)).wgpu().build_ui_state(
        move |ui, app: &mut PhotocraftApp| {
            let _language = crate::i18n::language_scope(lang);
            if !ready {
                PhotocraftApp::setup_context(ui.ctx(), crate::theme::ThemeKind::Pro);
                ready = true;
                return;
            }
            show(app, ui.ctx());
        },
        app,
    );
    harness.run_steps(5);
    harness.render().unwrap().save(directory.join("photocraft-embedded-vector-editor-en.png")).unwrap();
}

/// Filter Gallery owns a document preview even though `distort.active()` excludes it.
#[test]
fn saved_changes_wait_for_filter_gallery_and_resume_after_cancel() {
    let (mut app, saves, _) = fixture();
    open(&mut app);
    confirm(&mut app).unwrap();
    crate::gallery_ui::open(&mut app).unwrap();
    assert!(app.distort.gallery.is_some());
    assert!(!app.distort.active());
    let before = app.session.active().unwrap().doc.clone();
    let changed = SVG.replace("red", "blue");
    saves.borrow_mut().push_back(Ok(changed.as_bytes().to_vec()));
    let ctx = egui::Context::default();
    poll(&mut app, &ctx);
    assert_eq!(app.session.active().unwrap().doc, before);
    assert!(app.smart_svg.editing.first().unwrap().saved.is_some());
    crate::gallery_ui::control(&mut app, &json!({"cancel":true})).unwrap();
    poll(&mut app, &ctx);
    assert!(app.smart_svg.editing.first().unwrap().saved.is_none());
    let state = app.session.active().unwrap();
    let LayerContent::Smart(smart) = &state.doc.layer(state.active_layer.unwrap()).unwrap().content else { panic!("smart object") };
    let photocraft_doc::SmartSource::Embedded { bytes, .. } = &smart.source else { panic!("embedded") };
    assert_eq!(bytes.as_slice(), changed.as_bytes());
}

#[test]
fn saved_changes_wait_through_quick_mask_and_resume_without_another_save() {
    let (mut app, saves, _) = fixture();
    open(&mut app);
    confirm(&mut app).unwrap();
    app.run("select.editInQuickMaskMode", json!({"on":true})).unwrap();
    let before = app.session.active().unwrap().doc.clone();
    saves.borrow_mut().push_back(Ok(SVG.replace("red", "blue").into_bytes()));
    let ctx = egui::Context::default();
    poll(&mut app, &ctx);
    assert_eq!(app.session.active().unwrap().doc, before);
    assert!(app.smart_svg.editing.first().unwrap().saved.is_some());
    app.run("select.editInQuickMaskMode", json!({"on":false})).unwrap();
    poll(&mut app, &ctx);
    assert!(app.smart_svg.editing.first().unwrap().saved.is_none());
    let st = app.session.active().unwrap();
    let photocraft_doc::LayerContent::Smart(sm) = &st.doc.layer(st.active_layer.unwrap()).unwrap().content else { panic!("smart object") };
    let photocraft_doc::SmartSource::Embedded { bytes, .. } = &sm.source else { panic!("embedded") };
    assert_eq!(bytes.as_slice(), SVG.replace("red", "blue").as_bytes());
}

#[test]
fn saved_changes_wait_for_layers_panel_dialog_before_first_modal_frame() {
    let (mut app, saves, _) = fixture();
    open(&mut app);
    confirm(&mut app).unwrap();
    let before = app.session.active().unwrap().doc.clone();
    app.ui.layers_panel_dialog = Some(crate::state::LayersPanelDialog::NewGroup("New group".into()));
    saves.borrow_mut().push_back(Ok(SVG.replace("red", "blue").into_bytes()));
    let ctx = egui::Context::default();
    assert!(ctx.memory(|memory| memory.top_modal_layer().is_none()));
    poll(&mut app, &ctx);
    assert_eq!(app.session.active().unwrap().doc, before);
    assert!(app.smart_svg.editing.first().unwrap().saved.is_some());
    app.ui.layers_panel_dialog = None;
    poll(&mut app, &ctx);
    assert_ne!(app.session.active().unwrap().doc, before);
    assert!(app.smart_svg.editing.first().unwrap().saved.is_none());
}

struct FinishedSession(Option<Result<Vec<u8>, String>>);
impl ExternalSvgSession for FinishedSession {
    fn poll(&mut self) -> Option<Result<Vec<u8>, String>> {
        self.0.take()
    }
    fn finished(&self) -> bool {
        true
    }
}

#[test]
fn cancelled_desktop_choosers_release_session_capacity_without_changing_content() {
    let (mut app, _, _) = fixture();
    app.services.open_external_svg = Some(Box::new(|_, _| Ok(Box::new(FinishedSession(Some(Err("Editor selection cancelled".into())))))));
    let before = app.session.active().unwrap().doc.clone();
    let ctx = egui::Context::default();
    for _ in 0..MAX_SESSIONS + 1 {
        open(&mut app);
        confirm(&mut app).unwrap();
        poll(&mut app, &ctx);
        assert!(app.smart_svg.editing.is_empty());
        assert_eq!(app.session.active().unwrap().doc, before);
    }
}

#[test]
fn finished_session_retains_final_save_until_target_is_safe() {
    let (mut app, _, _) = fixture();
    app.services.open_external_svg = Some(Box::new(|_, _| Ok(Box::new(FinishedSession(Some(Ok(SVG.replace("red", "blue").into_bytes())))))));
    open(&mut app);
    confirm(&mut app).unwrap();
    let ctx = egui::Context::default();
    app.run("select.editInQuickMaskMode", json!({"on":true})).unwrap();
    let before = app.session.active().unwrap().doc.clone();
    poll(&mut app, &ctx);
    assert_eq!(app.smart_svg.editing.len(), 1);
    assert!(app.smart_svg.editing[0].saved.is_some());
    assert_eq!(app.session.active().unwrap().doc, before);
    app.run("select.editInQuickMaskMode", json!({"on":false})).unwrap();
    poll(&mut app, &ctx);
    assert_ne!(app.session.active().unwrap().doc, before);
    poll(&mut app, &ctx);
    assert!(app.smart_svg.editing.is_empty());
}
