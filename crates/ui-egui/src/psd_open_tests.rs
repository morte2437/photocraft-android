//! Damaged PSD confirmation is an opening decision, not a persistent save policy.
//! The exact same generated files exercise IO, the synchronous web-style path,
//! background opening and the agent-visible dialog commands.

#[path = "../../io/tests/support/recovery_psd.rs"]
mod recovery_psd;

use crate::{PhotocraftApp, Services, control};
use photocraft_engine::Session;
use serde_json::{Value, json};
use std::{cell::RefCell, rc::Rc};

/// Creates a minimal UI application with in-memory import/export services for PSD tests.
fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(
        Session::new(),
        Services {
            import: Some(Box::new(|name, bytes, opts| {
                photocraft_io::import_with_options(name, bytes, opts).map(|r| (r.document, r.warnings)).map_err(|e| e.to_string())
            })),
            export: Some(Box::new(|doc, path, _| {
                photocraft_io::export(doc, path, &Default::default()).map(|r| (r.bytes, r.warnings)).map_err(|e| e.to_string())
            })),
            write: Some(Box::new(|_, _| Ok(()))),
            automation_write: Some(Box::new(|_, _| Ok(()))),
            ..Default::default()
        },
    );
    app.background_jobs = false;
    app
}

/// Runs an agent-facing control command synchronously and returns its JSON result.
fn call(app: &mut PhotocraftApp, method: &str, params: Value) -> Value {
    let (req, _rx) = control::ControlRequest::new(method, params);
    match control::handle(app, &egui::Context::default(), &req) {
        control::Outcome::Done(v) => v,
        _ => panic!("{method}: expected an immediate reply"),
    }
}

/// Inspects the active damage prompt and returns its agent-visible dialog ID.
fn damage_dialog(app: &mut PhotocraftApp) -> u64 {
    let inspected = call(app, "ui.inspect", json!({}));
    assert_eq!(inspected["ok"], true);
    let dialogs = inspected["result"]["dialogs"].as_array().unwrap();
    assert_eq!(dialogs.len(), 1, "only the damage confirmation: {dialogs:?}");
    assert_eq!(dialogs[0]["fields"]["__psdDamagedOpen"], true);
    assert!(dialogs[0]["fields"]["message"].as_str().unwrap().contains("saving over the original"));
    dialogs[0]["id"].as_u64().unwrap()
}

/// Verifies valid layered PSD files open immediately without a confirmation dialog.
#[test]
fn healthy_psd_opens_without_a_damage_prompt() {
    let mut app = app();
    app.open_bytes("valid-two-layers.psd", &recovery_psd::make_psd(false)).unwrap();
    assert_eq!(app.session.documents().len(), 1);
    assert!(app.ui.dialogs.is_empty());
    assert!(!app.session.active().unwrap().source_read_only);
}

/// Verifies agents can cancel or confirm a damaged PSD using standard dialog commands.
#[test]
fn damaged_psd_can_be_cancelled_or_accepted_through_agent_dialog_commands() {
    let mut app = app();
    let bytes = recovery_psd::make_psd(true);
    app.open_bytes("corrupt-green-rle.psd", &bytes).unwrap();
    assert!(app.session.documents().is_empty());
    let id = damage_dialog(&mut app);
    assert_eq!(call(&mut app, "ui.dialog.cancel", json!({"dialog": id}))["ok"], true);
    assert!(app.session.documents().is_empty());
    assert!(app.ui.dialogs.is_empty());

    app.open_bytes("corrupt-green-rle.psd", &bytes).unwrap();
    let id = damage_dialog(&mut app);
    assert_eq!(call(&mut app, "ui.dialog.confirm", json!({"dialog": id}))["ok"], true);
    assert_eq!(app.session.documents().len(), 1);
    assert!(!app.session.active().unwrap().source_read_only);
    assert!(app.ui.dialogs.is_empty());
}

/// Verifies consent retains the original save path and restores ordinary Save behavior.
#[test]
fn accepted_psd_keeps_its_source_path_and_uses_the_normal_save_writer() {
    let mut app = app();
    let path = "original.psd";
    let stored = Rc::new(RefCell::new(recovery_psd::make_psd(true)));
    let original = stored.borrow().clone();
    let destination = stored.clone();
    app.services.write = Some(Box::new(move |p, bytes| {
        assert_eq!(p, path);
        *destination.borrow_mut() = bytes.to_vec();
        Ok(())
    }));
    app.open_bytes_from(path, &original, Some(path)).unwrap();
    assert_eq!(*stored.borrow(), original, "nothing written while waiting for consent");
    let id = damage_dialog(&mut app);
    assert_eq!(call(&mut app, "ui.dialog.confirm", json!({"dialog": id}))["ok"], true);
    assert_eq!(app.session.active().unwrap().path.as_deref(), Some(path));
    assert_eq!(*stored.borrow(), original, "accepting the open is not a save");

    for _ in 0..2 {
        app.run("layer.new.layer", json!({})).unwrap();
        crate::menus::invoke(&mut app, &egui::Context::default(), "file.save", json!({})).unwrap();
        assert_eq!(app.session.active().unwrap().path.as_deref(), Some(path));
        assert!(!app.session.active().unwrap().is_dirty());
        assert!(photocraft_io::import(path, &stored.borrow()).is_ok());
    }
    assert_ne!(*stored.borrow(), original, "ordinary Save can replace the source after consent");
    assert!(app.save_automation(Some("automated-copy.psd".into())).is_ok());
}

/// Verifies browser-style downloads keep their existing unconfirmed-save semantics.
#[test]
fn accepted_psd_uses_web_download_semantics_without_a_special_writer() {
    let mut app = app();
    app.services.write_unconfirmed = true;
    app.open_bytes("corrupt-green-rle.psd", &recovery_psd::make_psd(true)).unwrap();
    let id = damage_dialog(&mut app);
    assert_eq!(call(&mut app, "ui.dialog.confirm", json!({"dialog": id}))["ok"], true);
    app.run("layer.new.layer", json!({})).unwrap();
    app.save_as(Some("download.psd".into())).unwrap();
    assert_eq!(app.session.active().unwrap().path.as_deref(), Some("download.psd"));
    assert!(app.session.active().unwrap().is_dirty(), "an unconfirmed browser download is not a confirmed save");
}

/// Verifies non-interactive automation imports recover readable content without a dialog.
#[test]
fn damaged_psd_automation_import_does_not_require_interactive_confirmation() {
    let mut app = app();
    let warnings = app.open_automation_bytes("corrupt-green-rle.psd", &recovery_psd::make_psd(true)).unwrap();
    assert_eq!(app.session.documents().len(), 1);
    assert!(!app.session.active().unwrap().source_read_only);
    assert!(app.ui.dialogs.is_empty());
    assert!(warnings.iter().any(|w| w.contains("could not be decoded")));
}

/// Verifies queued damaged PSD files each require independent user confirmation.
#[test]
fn cancelling_one_damaged_open_does_not_open_the_next_damaged_file_without_consent() {
    let mut app = app();
    let bytes = recovery_psd::make_psd(true);
    for name in ["first.psd", "second.psd"] {
        app.open_bytes(name, &bytes).unwrap();
    }
    let first = damage_dialog(&mut app);
    assert_eq!(call(&mut app, "ui.dialog.cancel", json!({"dialog": first}))["ok"], true);
    assert!(app.session.documents().is_empty());
    let second = damage_dialog(&mut app);
    assert_ne!(first, second);
    assert_eq!(app.ui.dialogs[0].fields["name"], "second.psd");
    assert_eq!(call(&mut app, "ui.dialog.confirm", json!({"dialog": second}))["ok"], true);
    assert_eq!(app.session.documents().len(), 1);
    assert_eq!(app.session.active().unwrap().doc.name, "second.psd");
}

/// Verifies the damage prompt cannot be bypassed with a color-profile answer and
/// that the profile question is still shown after accepting the damage.
#[test]
fn damage_confirmation_precedes_the_profile_question() {
    let mut app = app();
    app.run("edit.colorSettings", json!({"askOnMismatch": true, "policyRgb": "preserve"})).unwrap();
    let (mut file, _) = photocraft_psd::PsdFile::from_bytes_and_merged(&recovery_psd::make_psd(true), |_| false).unwrap();
    file.resources.push(photocraft_psd::ImageResource::new(
        photocraft_psd::resources::ids::ICC_PROFILE,
        photocraft_cms::Builtin::from_id("adobe-rgb-compat").unwrap().profile().to_bytes().to_vec(),
    ));
    app.open_bytes("profile.psd", &file.to_bytes().unwrap()).unwrap();
    let id = damage_dialog(&mut app);
    assert!(crate::open_prompt::answer(&mut app, Some("preserve")).is_err(), "a profile answer cannot bypass the damage confirmation");
    assert_eq!(call(&mut app, "ui.dialog.confirm", json!({"dialog": id}))["ok"], true);
    assert!(app.session.documents().is_empty(), "the profile question still needs its own answer");
    assert_eq!(app.ui.dialogs.len(), 1);
    assert!(!crate::open_prompt::owns_damage(&app.ui.dialogs[0].fields));
    let profile = app.ui.dialogs[0].id;
    assert_eq!(call(&mut app, "ui.dialog.confirm", json!({"dialog": profile}))["ok"], true);
    assert_eq!(app.session.documents().len(), 1);
}

/// Verifies background decoding shows only the damage prompt when no profile question exists.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn background_damage_only_open_does_not_create_a_spurious_profile_question() {
    let mut app = app();
    app.background_jobs = true;
    app.open_bytes("corrupt-green-rle.psd", &recovery_psd::make_psd(true)).unwrap();
    // Wait deterministically for the worker, then let the normal UI completion path run.
    let jobs: Vec<_> = app.session.jobs().iter().map(|j| j.id).collect();
    for id in jobs {
        app.session.wait_job(id).unwrap();
    }
    crate::jobs_ui::tick(&mut app, &egui::Context::default());
    assert!(app.session.documents().is_empty());
    let id = damage_dialog(&mut app);
    assert_eq!(call(&mut app, "ui.dialog.confirm", json!({"dialog": id}))["ok"], true);
    assert_eq!(app.session.documents().len(), 1);
    assert!(app.ui.dialogs.is_empty(), "color:null is not a profile question");
}
