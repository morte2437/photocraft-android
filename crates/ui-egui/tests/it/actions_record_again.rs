//! Record Again uses the real editor while retaining the selected step and permissions.
use std::sync::atomic::{AtomicUsize, Ordering};

use egui_kittest::{Harness, kittest::Queryable};
use photocraft_engine::{EngineError, Session, actions_cmds::Action};
use photocraft_ui_egui::{PhotocraftApp, actions, canvas, dialogs, state::DialogKind};
use serde_json::{Value, json};

/// Draw the actual panel and dialogs over one selected command, without a GPU or file service.
fn harness(command: &str, params: Value) -> Harness<'static, PhotocraftApp> {
    let mut app = PhotocraftApp::new(Session::new(), Default::default());
    app.background_jobs = false;
    app.run("file.new", json!({"width":8,"height":6})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    let mut action = Action::new("A", vec![(command.into(), params), ("select.all".into(), json!({}))]);
    action.disabled.insert(0);
    action.dialogs.insert(0);
    app.session.actions.list.push(action);
    app.ui.actions.selected = Some(0);
    app.ui.actions.selected_step = Some(0);
    let mut h = Harness::builder().with_size(egui::vec2(1000.0, 760.0)).build_ui_state(
        |ui, app| {
            actions::panel_menu(app, ui);
            dialogs::show(app, ui.ctx());
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, Default::default());
    h.run_steps(3);
    h
}

/// Open by clicking the panel entry, preserving its selected-step and enablement checks.
fn open(h: &mut Harness<'static, PhotocraftApp>) -> u64 {
    h.get_by_role_and_label(egui::accesskit::Role::Button, "Record Again").click();
    h.run_steps(3);
    h.state().ui.dialogs.last().unwrap().id
}

/// A resize with different recorded and document sizes exposes seeding from the wrong source.
fn sizing() -> Harness<'static, PhotocraftApp> {
    harness("image.imageSize", json!({"width":4,"height":3,"resample":"nearest"}))
}

/// Cancel preserves the step and document; OK runs only that step and stores its new settings.
#[test]
fn seeded_editor_cancels_or_updates_only_the_original_step() {
    let mut h = sizing();
    let original = h.state().session.actions.list.clone();
    let revision = h.state().session.active().unwrap().revision;
    let dialog = open(&mut h);
    let fields = &h.state().ui.dialogs.last().unwrap().fields;
    assert_eq!(fields["__sizing"], true);
    assert_eq!(fields["width"].as_f64(), Some(4.0));
    assert_eq!(fields["height"].as_f64(), Some(3.0));
    assert_eq!(fields["__recordAgain"], true);
    h.get_by_label("Cancel").click();
    h.run_steps(3);
    assert_eq!(h.state().session.actions.list, original);
    assert_eq!(h.state().session.active().unwrap().revision, revision);
    assert!(h.state().ui.dialogs.iter().all(|d| d.id != dialog));
    let dialog = open(&mut h);
    let fields = &mut h.state_mut().ui.dialog_mut(dialog).unwrap().fields;
    fields.insert("width".into(), json!(6));
    fields.insert("height".into(), json!(5));
    fields.insert("__constrain".into(), json!(false));
    h.run_steps(3);
    h.get_by_label("OK").click();
    h.run_steps(3);
    assert!(h.state().ui.dialogs.is_empty());
    let action = &h.state().session.actions.list[0];
    assert_eq!(action.steps.len(), 2);
    assert_eq!(action.steps[0].1["width"].as_f64(), Some(6.0));
    assert!(action.disabled.contains(&0));
    assert!(action.dialogs.contains(&0));
    assert_eq!(h.state().session.active().unwrap().doc.size, photocraft_engine::doc::Size::new(6, 5));
    assert!(h.state().session.active().unwrap().doc.selection.is_none());
}

/// Permit initial settings while denying a later edit even if the current gate is removed.
fn limited_resize(id: &str, params: &Value) -> photocraft_engine::Result<()> {
    if id == "image.imageSize" && params.get("width").and_then(Value::as_f64).is_some_and(|w| w >= 10.0) {
        Err(EngineError::Other("resize denied".into()))
    } else {
        Ok(())
    }
}

/// Changed definitions/documents, retained denials, invalid settings and retargeting cannot overwrite.
#[test]
fn stale_denied_or_failed_edit_preserves_the_stored_step() {
    for mode in ["action-stale", "document-stale", "denied", "retargeted", "failed"] {
        let mut h = sizing();
        if mode == "denied" {
            h.state_mut().session.authorize = Some(limited_resize);
        }
        let dialog = open(&mut h);
        match mode {
            "action-stale" => {
                h.state_mut().run("actions.rename", json!({"action":"A","name":"Changed"})).unwrap();
            }
            "document-stale" => {
                h.state_mut().run("file.new", json!({"width":3,"height":3})).unwrap();
            }
            "denied" => {
                h.state_mut().session.authorize = None;
                h.state_mut().ui.dialog_mut(dialog).unwrap().fields.insert("width".into(), json!(10));
            }
            "retargeted" => {
                h.state_mut().ui.dialog_mut(dialog).unwrap().fields.insert("__command".into(), json!("select.all"));
            }
            _ => {
                h.state_mut().ui.dialog_mut(dialog).unwrap().fields.insert("percent".into(), json!(-1));
            }
        }
        let original = h.state().session.actions.list.clone();
        let revision = h.state().session.active().unwrap().revision;
        assert!(dialogs::confirm(h.state_mut(), dialog).is_err(), "{mode}");
        assert_eq!(h.state().session.actions.list, original, "{mode}");
        assert_eq!(h.state().session.active().unwrap().revision, revision, "{mode}");
        h.run_steps(3);
        let retry = open(&mut h);
        dialogs::cancel(h.state_mut(), retry).unwrap();
    }
}

/// Confirmation and Cancel of an impostor dialog leave the original editor's private state intact.
#[test]
fn another_dialog_cannot_consume_the_selected_step() {
    let mut h = sizing();
    let real = open(&mut h);
    let fields = h.state().ui.dialogs.last().unwrap().fields.clone();
    let fake = h.state_mut().ui.open_dialog(DialogKind::Command, fields.clone());
    assert!(dialogs::confirm(h.state_mut(), fake).is_err());
    let fake = h.state_mut().ui.open_dialog(DialogKind::Command, fields);
    dialogs::cancel(h.state_mut(), fake).unwrap();
    dialogs::confirm(h.state_mut(), real).unwrap();
    assert_eq!(h.state().session.active().unwrap().doc.size, photocraft_engine::doc::Size::new(4, 3));
}

static PREVIEW_CHECKS: AtomicUsize = AtomicUsize::new(0);

/// Count actual preview authorization and refuse LUT paths or edited blur and gamma settings.
fn preview_gate(id: &str, params: &Value) -> photocraft_engine::Result<()> {
    if matches!(id, "filter.blur.gaussianBlur" | "image.adjustments.levels" | "image.adjustments.colorLookup") {
        PREVIEW_CHECKS.fetch_add(1, Ordering::SeqCst);
        if params.get("file").is_some() || ["radius", "gamma"].iter().any(|key| params.get(key).and_then(Value::as_f64).is_some_and(|v| v > 2.0)) {
            return Err(EngineError::Other("preview denied".into()));
        }
    }
    Ok(())
}

/// Both real preview routes check the retained caller before evaluating changed settings or LUT files.
#[test]
fn filter_and_adjustment_previews_retain_permissions_before_evaluation() {
    for command in ["filter.blur.gaussianBlur", "image.adjustments.levels", "image.adjustments.colorLookup"] {
        let params = if command.starts_with("filter.") {
            json!({"radius":1})
        } else {
            photocraft_engine::adjust_params::to_params(
                &photocraft_engine::adjust_params::default_for(command.strip_prefix("image.adjustments.").unwrap(), photocraft_engine::doc::ColorMode::Rgb)
                    .unwrap(),
            )
        };
        let mut h = harness(command, params);
        h.state_mut().session.authorize = Some(preview_gate);
        let dialog = open(&mut h);
        h.state_mut().session.authorize = None;
        let context = h.ctx.clone();
        PREVIEW_CHECKS.store(0, Ordering::SeqCst);
        assert!(canvas::ensure_texture(h.state_mut(), &context, 0, None).is_some());
        assert!(PREVIEW_CHECKS.load(Ordering::SeqCst) > 0, "the permitted real preview request is authorized: {command}");
        if command == "image.adjustments.levels" {
            assert!(dialogs::adjust_preview::display_doc(h.state_mut(), 0).is_some(), "the permitted adjustment preview is evaluated");
        }
        let original = h.state().session.actions.list.clone();
        let revision = h.state().session.active().unwrap().revision;
        let fields = &mut h.state_mut().ui.dialog_mut(dialog).unwrap().fields;
        if command.starts_with("filter.") {
            fields.insert("radius".into(), json!(8));
        } else if command == "image.adjustments.levels" {
            fields.insert("gamma".into(), json!(8));
            // This route parses settings even with Preview off; authorization must precede it.
            fields.insert("__preview".into(), json!(false));
        } else {
            fields.insert("file".into(), json!("/outside/forbidden.cube"));
        }
        PREVIEW_CHECKS.store(0, Ordering::SeqCst);
        canvas::ensure_texture(h.state_mut(), &context, 0, None);
        assert!(PREVIEW_CHECKS.load(Ordering::SeqCst) > 0, "the changed actual request was refused before evaluation: {command}");
        assert_eq!(h.state().session.actions.list, original);
        assert_eq!(h.state().session.active().unwrap().revision, revision);
        assert!(dialogs::confirm(h.state_mut(), dialog).is_err());
        assert_eq!(h.state().session.actions.list, original);
    }
}
