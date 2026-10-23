//! Re-develop completion must follow the replacement document across intervening tab changes.

use std::sync::Arc;

use super::*;

fn synthetic_raw() -> Vec<u8> {
    use photocraft_raw::testgen::{DngSpec, mosaic, scene};
    let (width, height) = (48, 32);
    DngSpec::cfa(width, height, mosaic(&scene(width, height), width, [0, 1, 1, 2], 0, 30000)).build()
}

struct Control {
    doc: Arc<photocraft_doc::Document>,
    path: Option<String>,
    revision: u64,
    saved_revision: u64,
    history: usize,
}

impl Control {
    fn assert_untouched(&self, app: &PhotocraftApp) {
        let state = app.session.documents().iter().find(|state| state.doc.id == self.doc.id).expect("the accepted control remains open");
        assert!(Arc::ptr_eq(&state.doc, &self.doc), "the RAW completion must not filter another document");
        assert_eq!(state.path, self.path);
        assert_eq!((state.revision, state.saved_revision, state.history.past_len()), (self.revision, self.saved_revision, self.history));
        assert!(state.is_dirty(), "the unrelated document still needs saving");
    }
}

fn identity_app() -> (PhotocraftApp, RawOpen, Control) {
    let mut session = photocraft_engine::Session::new();
    session.execute("file.new", json!({"width": 8, "height": 6})).unwrap();
    session.execute("layer.new.layer", json!({})).unwrap();
    session.execute("edit.fill", json!({"color": "#336699"})).unwrap();
    session.active_mut().unwrap().path = Some("accepted-control.pcraft".into());
    let state = session.active().unwrap();
    let control = Control {
        doc: state.doc.clone(),
        path: state.path.clone(),
        revision: state.revision,
        saved_revision: state.saved_revision,
        history: state.history.past_len(),
    };
    let bytes = Arc::new(synthetic_raw());
    let imported = photocraft_io::import("synthetic.dng", &bytes).unwrap();
    let (index, _) = session.open_document(imported.document, None);
    let raw = RawOpen {
        document: session.documents()[index].doc.id,
        name: "synthetic.dng".into(),
        path: None,
        bytes: Some(bytes),
        developed_with: Default::default(),
        demosaic: Default::default(),
    };
    let mut app = PhotocraftApp::new(session, crate::Services::default());
    app.sync_views();
    (app, raw, control)
}

/// Run the real DNG worker and apply its replacement, holding the UI completion until after
/// another event changes the tab list. No sleeps, timing races or fake decoder are needed.
fn completed_redevelop(app: &mut PhotocraftApp, raw: &RawOpen) -> (CameraRaw, Value, photocraft_doc::DocId) {
    app.background_jobs = true;
    let params = CameraRaw { exposure: 1.0, contrast: 20.0, ..Default::default() };
    let started = commit_open_raw(app, &params, raw).unwrap();
    let job = photocraft_engine::jobs::JobId(started["job"].as_u64().expect("the sensor re-develop runs as a real job"));
    let mut result = app.session.wait_job(job).unwrap();
    let replacement = app.session.documents()[result["document"].as_u64().unwrap() as usize].doc.id;
    // The producer has a separate contract test. Supply the observed identity here to isolate
    // the UI consumer's handling even when run against the old result schema.
    result["docId"] = json!(replacement.0);
    app.raw_redevelop = None;
    app.background_jobs = false;
    (params, result, replacement)
}

#[test]
fn completion_identity_reports_the_live_replacement_after_id_collision() {
    let (mut app, raw, control) = identity_app();
    let mut imported = photocraft_io::import("synthetic.dng", &synthetic_raw()).unwrap();
    // Exercise the same collision protection as a persisted native document: the result must
    // name the admitted replacement, even when the incoming ID already belongs to a tab.
    imported.document.id = control.doc.id;
    let result = replace_raw_document(&mut app.session, raw.document, imported, false).unwrap();
    let replacement = app.session.documents()[result["document"].as_u64().unwrap() as usize].doc.id;
    assert_ne!(replacement, control.doc.id);
    assert_eq!(result["docId"], replacement.0, "the callback needs the session's actual replacement identity");
    control.assert_untouched(&app);
}

#[test]
fn completion_identity_applies_remaining_settings_after_tab_reordering() {
    let (mut app, raw, control) = identity_app();
    let (params, result, replacement) = completed_redevelop(&mut app, &raw);
    let index = result["document"].as_u64().unwrap() as usize;
    app.session.move_document(index, 0).unwrap();
    assert_eq!(app.session.documents()[index].doc.id, control.doc.id, "another completion may move this tab before the RAW callback");
    finish_redevelop(&mut app, &params, &raw, &result).unwrap();
    control.assert_untouched(&app);
    let state = app.session.active().unwrap();
    assert_eq!(state.doc.id, replacement);
    assert!(!state.is_dirty(), "development finishes opening only the replacement");
    assert!(state.history.past_len() > 0, "remaining Camera Raw settings were applied");
    assert!(!app.view_docs.contains(&raw.document));
    assert!(app.view_docs.contains(&replacement));
}

#[test]
fn completion_identity_rejects_a_closed_replacement_without_filtering_the_active_control() {
    let (mut app, raw, control) = identity_app();
    let (params, result, replacement) = completed_redevelop(&mut app, &raw);
    let index = app.session.documents().iter().position(|state| state.doc.id == replacement).unwrap();
    app.session.close(index).unwrap();
    assert_eq!(app.session.active().unwrap().doc.id, control.doc.id);
    assert!(finish_redevelop(&mut app, &params, &raw, &result).is_err(), "a closed replacement must not fall back to the active document");
    control.assert_untouched(&app);
    assert_eq!(app.session.documents().len(), 1);
}

#[test]
fn completion_identity_rejects_missing_and_invalid_ids_before_any_side_effect() {
    for identity in [None, Some(Value::Null), Some(json!("invalid")), Some(json!(-1)), Some(json!(u64::MAX)), Some(json!(1.5))] {
        let (mut app, raw, control) = identity_app();
        let (params, mut result, _) = completed_redevelop(&mut app, &raw);
        result["document"] = json!(0);
        match identity {
            None => {
                result.as_object_mut().unwrap().remove("docId");
            }
            Some(value) => result["docId"] = value,
        }
        app.session.set_active(0);
        let views = app.view_docs.clone();
        let notices = app.ui.notices.len();
        assert!(finish_redevelop(&mut app, &params, &raw, &result).is_err(), "invalid identity was accepted: {result}");
        control.assert_untouched(&app);
        assert_eq!(app.session.active().unwrap().doc.id, control.doc.id);
        assert_eq!(app.view_docs, views, "identity validation precedes view remapping");
        assert_eq!(app.ui.notices.len(), notices, "identity validation precedes import notifications");
    }
}

#[test]
fn queue_identity_targets_an_inactive_raw_without_changing_the_active_document() {
    let (mut app, raw, control) = identity_app();
    app.session.set_active(0);
    assert_eq!(app.session.active().unwrap().doc.id, control.doc.id);
    queue_open_dialog(&mut app, raw.document, &raw.name, raw.path.as_deref(), raw.bytes.as_deref().map(Vec::as_slice), &[]).unwrap();
    let queued = app.pending_raw_open.as_ref().expect("the explicitly named RAW was queued");
    assert_eq!(queued.document, raw.document);
    assert_eq!((&queued.name, &queued.path, &queued.bytes), (&raw.name, &raw.path, &raw.bytes));
    assert_eq!(app.session.active().unwrap().doc.id, control.doc.id, "queueing does not activate the target");
    control.assert_untouched(&app);
}

#[test]
fn queue_identity_rejects_a_missing_document_without_replacing_the_pending_review() {
    let (mut app, raw, control) = identity_app();
    queue_open_dialog(&mut app, raw.document, &raw.name, raw.path.as_deref(), raw.bytes.as_deref().map(Vec::as_slice), &[]).unwrap();
    let pending = app.pending_raw_open.as_ref().unwrap().clone();
    app.session.set_active(0);
    let missing = photocraft_doc::DocId::fresh();
    assert!(!app.session.documents().iter().any(|state| state.doc.id == missing));
    assert!(queue_open_dialog(&mut app, missing, "missing.dng", Some("missing.dng"), Some(&[1, 2, 3]), &[]).is_err());
    let queued = app.pending_raw_open.as_ref().expect("the existing review is preserved");
    assert_eq!(queued.document, pending.document);
    assert_eq!((&queued.name, &queued.path, &queued.bytes), (&pending.name, &pending.path, &pending.bytes));
    assert_eq!(app.session.active().unwrap().doc.id, control.doc.id);
    control.assert_untouched(&app);
}
