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

#[test]
fn memory_confirmation_dialog_and_cancel_lifecycle() {
    let mut app = app();
    let gb = 1u64 << 30;

    let preflight_id = app
        .session
        .store_pending_preflight(photocraft_engine::jobs::PendingPsdPreflight {
            id: 0,
            name: "high_mem.psd".into(),
            source: photocraft_engine::jobs::OpenSource::Bytes(std::sync::Arc::new(vec![0; 16])),
            fingerprint: photocraft_engine::jobs::SourceFingerprint::of(&photocraft_engine::jobs::OpenSource::Bytes(std::sync::Arc::new(vec![0; 16]))).unwrap(),
            required: 7 * gb,
            soft_limit: 4 * gb,
            hard_limit: 8 * gb,
            available_ram: Some(10 * gb),
            confirm_damage: true,
        })
        .unwrap();

    crate::open_prompt::arrive_memory(&mut app, preflight_id, "high_mem.psd".into(), None, None, None, 7 * gb, Some(10 * gb), false, false);
    assert_eq!(app.ui.dialogs.len(), 1);
    assert!(crate::open_prompt::owns_memory(&app.ui.dialogs[0].fields));
    let dlg_id = app.ui.dialogs[0].id;

    assert_eq!(call(&mut app, "ui.dialog.cancel", json!({"dialog": dlg_id}))["ok"], true);
    assert!(app.ui.dialogs.is_empty());
    assert!(app.session.take_pending_preflight(preflight_id).is_none(), "pending preflight dropped on cancel");
    assert!(app.held_memory_prompts.is_empty());
    assert!(app.prompt_order.is_empty());
}

/// #10897: the inline path (the browser shell and any desktop open with background jobs off)
/// routes the memory prompt's answer through `file.approveMemory`, and the retry only opens the
/// file when it carries the one-shot approval. A tiny fixture under a scoped estimate charge
/// reaches the real approval window, so dropping the approval flag or the bound estimate in the
/// retry leaves the prompt (and no document) instead of passing silently.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn memory_confirmation_resume_inline_success() {
    if cfg!(target_pointer_width = "32") {
        return; // the 3 GiB charge would exceed a 32-bit process cap
    }
    let gb = 1u64 << 30;
    let mut app = app();
    app.background_jobs = false;
    let psd = recovery_psd::make_psd(false);
    // Known 5 GiB: S = 2.5 GiB, H = 3.75 GiB. The 3 GiB charge lands in (S, H] on this thread.
    let snap = photocraft_io::memory::MemorySnapshot { available_ram: Some(5 * gb), swap_free: None, commit_headroom: None };
    photocraft_io::psd_budget::with_test_estimate_charge(3 * gb, || {
        photocraft_io::memory::with_mock_memory(snap, || {
            // The real inline import raises the preflight: no fabricated prompt or preflight.
            app.open_bytes_from("inline_resume.psd", &psd, Some("/test/inline_resume.psd")).unwrap();
            assert!(app.session.documents().is_empty(), "the file waits for approval");
            assert_eq!(app.held_memory_prompts.len(), 1);
            assert_eq!(app.ui.dialogs.len(), 1);
            let fields = &app.ui.dialogs[0].fields;
            assert!(crate::open_prompt::owns_memory(fields));
            assert!(fields["estimated"].as_u64().unwrap() > 3 * gb, "the charged estimate is shown: {fields:?}");
            assert_eq!(fields["available"], json!(5 * gb));
            assert_eq!(fields["capacityUnknown"], json!(false));
            assert_eq!(fields["uncertainCapacityWarning"], json!(false));
            let dlg_id = app.ui.dialogs[0].id;

            let res = call(&mut app, "ui.dialog.confirm", json!({"dialog": dlg_id}));
            assert_eq!(res["ok"], true, "{res}");
            assert!(app.ui.dialogs.is_empty(), "dialog dismissed after inline resume");
            assert_eq!(app.session.documents().len(), 1, "document opened via inline resume");
            assert_eq!(app.session.documents()[0].doc.name, "inline_resume.psd");
            assert_eq!(app.session.documents()[0].path.as_deref(), Some("/test/inline_resume.psd"));
            assert!(app.held_memory_prompts.is_empty());
            assert!(app.held_opens.is_empty());
            assert!(app.prompt_order.is_empty());
            assert!(app.jobs.opens.is_empty());
            assert_eq!(app.session.pending_preflight_bytes().unwrap(), 0, "the approval consumed and released the preflight");
        });
    });
}

/// #10897: a background open carries the same scoped estimate into its worker through the
/// explicitly captured test scope (never a global mock), so the real job raises the approval
/// prompt; the retry then opens the file only with the one-shot approval.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn memory_confirmation_resume_background_success() {
    if cfg!(target_pointer_width = "32") {
        return; // the 3 GiB charge would exceed a 32-bit process cap
    }
    let gb = 1u64 << 30;
    let mut app = app();
    app.background_jobs = true;
    let psd = recovery_psd::make_psd(false);
    let snap = photocraft_io::memory::MemorySnapshot { available_ram: Some(5 * gb), swap_free: None, commit_headroom: None };
    photocraft_io::psd_budget::with_test_estimate_charge(3 * gb, || {
        photocraft_io::memory::with_mock_memory(snap, || {
            app.open_bytes_from("bg_resume.psd", &psd, Some("/test/bg_resume.psd")).unwrap();
            assert_eq!(app.jobs.opens.len(), 1, "a background open tab appears at once");
            let jobs: Vec<_> = app.session.jobs().iter().map(|j| j.id).collect();
            for id in jobs {
                app.session.wait_job(id).unwrap();
            }
            crate::jobs_ui::tick(&mut app, &egui::Context::default());
            assert!(app.session.documents().is_empty(), "the file waits for approval");
            assert_eq!(app.held_memory_prompts.len(), 1);
            assert_eq!(app.ui.dialogs.len(), 1);
            let fields = &app.ui.dialogs[0].fields;
            assert!(crate::open_prompt::owns_memory(fields));
            assert!(fields["estimated"].as_u64().unwrap() > 3 * gb, "the worker made the charged estimate: {fields:?}");
            assert_eq!(fields["available"], json!(5 * gb));
            assert_eq!(fields["capacityUnknown"], json!(false));
            let dlg_id = app.ui.dialogs[0].id;

            let res = call(&mut app, "ui.dialog.confirm", json!({"dialog": dlg_id}));
            assert_eq!(res["ok"], true, "{res}");
            assert_eq!(app.jobs.opens.len(), 1, "background retry job registered in jobs.opens");

            let jobs: Vec<_> = app.session.jobs().iter().map(|j| j.id).collect();
            for id in jobs {
                app.session.wait_job(id).unwrap();
            }
            crate::jobs_ui::tick(&mut app, &egui::Context::default());
            assert_eq!(app.session.documents().len(), 1, "document opened via background resume");
            assert_eq!(app.session.documents()[0].doc.name, "bg_resume.psd");
            assert_eq!(app.session.documents()[0].path.as_deref(), Some("/test/bg_resume.psd"));
            assert!(app.ui.dialogs.is_empty());
            assert!(app.held_memory_prompts.is_empty());
            assert!(app.held_opens.is_empty());
            assert!(app.prompt_order.is_empty());
            assert!(app.jobs.opens.is_empty());
            assert_eq!(app.session.pending_preflight_bytes().unwrap(), 0);
        });
    });
}

#[test]
fn memory_confirmation_refuses_when_source_changed_inline() {
    let mut app = app();
    app.background_jobs = false;
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("changed.psd");
    std::fs::write(&path, b"initial file contents").unwrap();

    let src = photocraft_engine::jobs::OpenSource::Path(path.to_string_lossy().to_string());
    let fp = photocraft_engine::jobs::SourceFingerprint::of(&src).unwrap();

    let preflight_id = app
        .session
        .store_pending_preflight(photocraft_engine::jobs::PendingPsdPreflight {
            id: 0,
            name: "changed.psd".into(),
            source: src,
            fingerprint: fp,
            required: 1000,
            soft_limit: 500,
            hard_limit: 50_000,
            available_ram: Some(100_000),
            confirm_damage: true,
        })
        .unwrap();

    crate::open_prompt::arrive_memory(
        &mut app,
        preflight_id,
        "changed.psd".into(),
        Some(path.to_string_lossy().to_string()),
        None,
        None,
        1000,
        Some(100_000),
        false,
        false,
    );
    assert_eq!(app.ui.dialogs.len(), 1);
    let dlg_id = app.ui.dialogs[0].id;

    // Mutate file before confirming:
    std::fs::write(&path, b"modified file contents longer").unwrap();

    let res = call(&mut app, "ui.dialog.confirm", json!({"dialog": dlg_id}));
    assert_eq!(res["ok"], false, "confirming changed file inline must fail");
    let error = res["error"].as_str().unwrap_or_default();
    assert!(error.contains("changed since it was checked"), "the refusal names the changed identity, not a parser failure: {res}");
    assert!(app.session.documents().is_empty());
    assert!(app.held_memory_prompts.is_empty());
}

#[test]
fn memory_confirmation_refuses_when_source_changed_async() {
    let mut app = app();
    app.background_jobs = true;
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("changed_async.psd");
    std::fs::write(&path, b"initial file contents").unwrap();

    let src = photocraft_engine::jobs::OpenSource::Path(path.to_string_lossy().to_string());
    let fp = photocraft_engine::jobs::SourceFingerprint::of(&src).unwrap();

    let preflight_id = app
        .session
        .store_pending_preflight(photocraft_engine::jobs::PendingPsdPreflight {
            id: 0,
            name: "changed_async.psd".into(),
            source: src,
            fingerprint: fp,
            required: 1000,
            soft_limit: 500,
            hard_limit: 50_000,
            available_ram: Some(100_000),
            confirm_damage: true,
        })
        .unwrap();

    crate::open_prompt::arrive_memory(
        &mut app,
        preflight_id,
        "changed_async.psd".into(),
        Some(path.to_string_lossy().to_string()),
        None,
        None,
        1000,
        Some(100_000),
        false,
        false,
    );
    assert_eq!(app.ui.dialogs.len(), 1);
    let dlg_id = app.ui.dialogs[0].id;

    // Mutate file before confirming:
    std::fs::write(&path, b"modified file contents longer").unwrap();

    let res = call(&mut app, "ui.dialog.confirm", json!({"dialog": dlg_id}));
    assert_eq!(res["ok"], true, "confirmation kicks off background job");
    assert_eq!(app.jobs.opens.len(), 1);

    // Run the job to completion and observe async failure:
    let jobs: Vec<_> = app.session.jobs().iter().map(|j| j.id).collect();
    for id in jobs {
        let _ = app.session.wait_job(id);
    }
    crate::jobs_ui::tick(&mut app, &egui::Context::default());

    assert!(app.session.documents().is_empty(), "no document opened on changed file failure");
    assert!(app.jobs.opens.is_empty(), "opens cleared on job failure");
    assert!(app.ui.status_error, "error recorded in UI status");
    assert!(app.ui.status.contains("changed since it was checked"), "the background refusal names the changed identity: {}", app.ui.status);
}

#[test]
fn memory_decline_advances_queue_and_cleans_batch() {
    let mut app = app();
    let batch_id = crate::jobs_ui::begin_batch(&mut app);
    let order = crate::jobs_ui::OpenOrder { batch: batch_id, seq: 0 };

    let preflight_id = app
        .session
        .store_pending_preflight(photocraft_engine::jobs::PendingPsdPreflight {
            id: 0,
            name: "declined.psd".into(),
            source: photocraft_engine::jobs::OpenSource::Bytes(std::sync::Arc::new(vec![0; 16])),
            fingerprint: photocraft_engine::jobs::SourceFingerprint::of(&photocraft_engine::jobs::OpenSource::Bytes(std::sync::Arc::new(vec![0; 16]))).unwrap(),
            required: 1000,
            soft_limit: 500,
            hard_limit: 2000,
            available_ram: Some(4000),
            confirm_damage: true,
        })
        .unwrap();

    crate::open_prompt::arrive_memory(&mut app, preflight_id, "declined.psd".into(), None, None, Some(order), 1000, Some(4000), false, false);
    assert_eq!(app.ui.dialogs.len(), 1);
    let dlg_id = app.ui.dialogs[0].id;

    let res = call(&mut app, "ui.dialog.cancel", json!({"dialog": dlg_id}));
    assert_eq!(res["ok"], true);
    assert!(app.ui.dialogs.is_empty());
    assert!(app.session.take_pending_preflight(preflight_id).is_none());
    assert!(app.jobs.batches.is_empty(), "batch cleaned up on decline");
    assert!(app.prompt_order.is_empty());
}

#[test]
fn batch_single_file_cancel_preserves_other_held_memory_orders() {
    let mut app = app();
    let batch_id = crate::jobs_ui::begin_batch(&mut app);
    let order0 = crate::jobs_ui::OpenOrder { batch: batch_id, seq: 0 };
    let order1 = crate::jobs_ui::OpenOrder { batch: batch_id, seq: 1 };

    let p0 = app
        .session
        .store_pending_preflight(photocraft_engine::jobs::PendingPsdPreflight {
            id: 0,
            name: "first.psd".into(),
            source: photocraft_engine::jobs::OpenSource::Bytes(std::sync::Arc::new(vec![0; 4])),
            fingerprint: photocraft_engine::jobs::SourceFingerprint::of(&photocraft_engine::jobs::OpenSource::Bytes(std::sync::Arc::new(vec![0; 4]))).unwrap(),
            required: 1000,
            soft_limit: 500,
            hard_limit: 2000,
            available_ram: Some(4000),
            confirm_damage: true,
        })
        .unwrap();

    let p1 = app
        .session
        .store_pending_preflight(photocraft_engine::jobs::PendingPsdPreflight {
            id: 0,
            name: "second.psd".into(),
            source: photocraft_engine::jobs::OpenSource::Bytes(std::sync::Arc::new(vec![0; 4])),
            fingerprint: photocraft_engine::jobs::SourceFingerprint::of(&photocraft_engine::jobs::OpenSource::Bytes(std::sync::Arc::new(vec![0; 4]))).unwrap(),
            required: 1000,
            soft_limit: 500,
            hard_limit: 2000,
            available_ram: Some(4000),
            confirm_damage: true,
        })
        .unwrap();

    crate::open_prompt::arrive_memory(&mut app, p0, "first.psd".into(), None, None, Some(order0), 1000, Some(4000), false, false);
    crate::open_prompt::arrive_memory(&mut app, p1, "second.psd".into(), None, None, Some(order1), 1000, Some(4000), false, false);

    // Cancel first file:
    let dlg0 = app.ui.dialogs[0].id;
    assert_eq!(call(&mut app, "ui.dialog.cancel", json!({"dialog": dlg0}))["ok"], true);

    // Batch must be preserved because second file is still waiting:
    assert_eq!(app.jobs.batches.len(), 1, "batch preserved while second file is waiting");
    assert_eq!(app.held_memory_prompts.len(), 1);

    // Now cancel second file:
    let dlg1 = app.ui.dialogs[0].id;
    assert_eq!(call(&mut app, "ui.dialog.cancel", json!({"dialog": dlg1}))["ok"], true);
    assert!(app.jobs.batches.is_empty(), "batch cleaned after all members cancelled");
    assert!(app.prompt_order.is_empty());
}

#[test]
fn profile_cancel_cancels_whole_queue_including_memory_prompts() {
    let mut app = app();
    let gb = 1u64 << 30;

    let preflight_id = app
        .session
        .store_pending_preflight(photocraft_engine::jobs::PendingPsdPreflight {
            id: 0,
            name: "high_mem.psd".into(),
            source: photocraft_engine::jobs::OpenSource::Bytes(std::sync::Arc::new(vec![0; 16])),
            fingerprint: photocraft_engine::jobs::SourceFingerprint::of(&photocraft_engine::jobs::OpenSource::Bytes(std::sync::Arc::new(vec![0; 16]))).unwrap(),
            required: 7 * gb,
            soft_limit: 4 * gb,
            hard_limit: 8 * gb,
            available_ram: Some(10 * gb),
            confirm_damage: true,
        })
        .unwrap();

    crate::open_prompt::arrive_memory(&mut app, preflight_id, "high_mem.psd".into(), None, None, None, 7 * gb, Some(10 * gb), false, false);

    crate::open_prompt::arrive(
        &mut app,
        crate::open_prompt::Held {
            doc: photocraft_doc::Document::new("test", photocraft_geom::Size::new(4, 4), photocraft_doc::ColorMode::Rgb, photocraft_doc::SampleType::U8),
            read_only: false,
            damage_pending: false,
            name: "profile.psd".into(),
            path: None,
            warnings: vec![],
            bytes: None,
            slot: None,
            order: None,
            question: Some(json!({"missing": false, "working": "sRGB", "embedded": "Adobe RGB"})),
            policy_applied: false,
            smart_object: false,
        },
    );

    let r = crate::open_prompt::answer(&mut app, None).unwrap();
    assert_eq!(r["cancelled"], 2, "both held open and memory prompt dropped");
    assert!(app.held_opens.is_empty());
    assert!(app.held_memory_prompts.is_empty());
    assert!(app.prompt_order.is_empty());
    assert!(app.session.take_pending_preflight(preflight_id).is_none(), "pending preflight cancelled");
}

#[test]
fn mixed_prompts_follow_arrival_order_fifo() {
    let mut app = app();
    let gb = 1u64 << 30;

    // 1. Arrive damage prompt first
    crate::open_prompt::arrive(
        &mut app,
        crate::open_prompt::Held {
            doc: photocraft_doc::Document::new("test", photocraft_geom::Size::new(4, 4), photocraft_doc::ColorMode::Rgb, photocraft_doc::SampleType::U8),
            read_only: false,
            damage_pending: true,
            name: "first_damage.psd".into(),
            path: None,
            warnings: vec!["damaged data".into()],
            bytes: None,
            slot: None,
            order: None,
            question: None,
            policy_applied: false,
            smart_object: false,
        },
    );

    // 2. Arrive memory prompt second
    let preflight_id = app
        .session
        .store_pending_preflight(photocraft_engine::jobs::PendingPsdPreflight {
            id: 0,
            name: "second_mem.psd".into(),
            source: photocraft_engine::jobs::OpenSource::Bytes(std::sync::Arc::new(vec![0; 16])),
            fingerprint: photocraft_engine::jobs::SourceFingerprint::of(&photocraft_engine::jobs::OpenSource::Bytes(std::sync::Arc::new(vec![0; 16]))).unwrap(),
            required: 7 * gb,
            soft_limit: 4 * gb,
            hard_limit: 8 * gb,
            available_ram: Some(10 * gb),
            confirm_damage: true,
        })
        .unwrap();
    crate::open_prompt::arrive_memory(&mut app, preflight_id, "second_mem.psd".into(), None, None, None, 7 * gb, Some(10 * gb), false, false);

    // First prompt shown must be damage (since it arrived first)!
    assert_eq!(app.ui.dialogs.len(), 1);
    assert!(crate::open_prompt::owns_damage(&app.ui.dialogs[0].fields));

    // Cancel damage prompt:
    let dlg_id = app.ui.dialogs[0].id;
    assert_eq!(call(&mut app, "ui.dialog.cancel", json!({"dialog": dlg_id}))["ok"], true);

    // Now memory prompt must be shown!
    assert_eq!(app.ui.dialogs.len(), 1);
    assert!(crate::open_prompt::owns_memory(&app.ui.dialogs[0].fields));
    let dlg_mem = app.ui.dialogs[0].id;
    assert_eq!(call(&mut app, "ui.dialog.cancel", json!({"dialog": dlg_mem}))["ok"], true);
    assert!(app.ui.dialogs.is_empty());
}

/// A stored preflight for the prompt tests, with a tiny in-memory source so nothing large is
/// ever held.
fn store_preflight(app: &mut PhotocraftApp, name: &str, required: u64, available: Option<u64>) -> u64 {
    let bytes = std::sync::Arc::new(vec![0u8; 16]);
    let source = photocraft_engine::jobs::OpenSource::Bytes(std::sync::Arc::clone(&bytes));
    let fingerprint = photocraft_engine::jobs::SourceFingerprint::of(&source).unwrap();
    app.session
        .store_pending_preflight(photocraft_engine::jobs::PendingPsdPreflight {
            id: 0,
            name: name.into(),
            source,
            fingerprint,
            required,
            soft_limit: required / 2,
            hard_limit: required.saturating_mul(2),
            available_ram: available,
            confirm_damage: true,
        })
        .unwrap()
}

/// #10897: the browser and desktop share one consent route. The web shell runs every open inline
/// (`background_jobs == false`), answers the same `MemoryPrompt`, and dispatches the same
/// `file.approveMemory` command: it is not menu-only, journaled or gated off wasm. The web's own
/// allowance and 2 GiB product cap stay in `photocraft-io`'s policy, so approval never raises
/// them. This checks the routing half on the host; the web policy half is covered by
/// `photocraft_io::memory`'s web-policy tests.
#[test]
fn memory_approval_routes_through_the_file_approve_memory_command() {
    let app = app();
    assert!(!app.background_jobs, "the browser shell is always inline");
    assert!(app.session.is_enabled("file.approveMemory"), "the one-shot approval command is available on every target");
    let spec = photocraft_engine::commands::find("file.approveMemory").expect("registered");
    assert!(spec.menu.is_empty(), "it is not a menu item");
    assert!(!spec.journal, "a one-shot permission is never journaled or replayed");
}

/// #10897: on the web the same prompt offers consent, and `file.approveMemory` may authorize an
/// estimate above the 1 GiB automatic allowance only within the 2 GiB product cap: a 1.5 GiB
/// estimate is approved and opens, a 3 GiB estimate is refused before decoding. The fixed mock
/// snapshot is ignored on wasm (the web policy does not read one), so the charge alone sets the
/// window. This target is compile-checked, not executed, by the current CI (`cargo xtask wasm`
/// checks the crates without test targets); the same web policy is exercised on native by
/// `photocraft_io::memory`'s web-policy tests.
#[cfg(target_arch = "wasm32")]
#[test]
fn web_memory_prompt_offers_consent_within_the_two_gib_product_cap() {
    let gb = 1u64 << 30;
    let psd = recovery_psd::make_psd(false);
    let mut approved = app();
    photocraft_io::psd_budget::with_test_estimate_charge(3 * gb / 2, || {
        approved.open_bytes_from("web.psd", &psd, Some("web.psd")).unwrap();
        assert_eq!(approved.held_memory_prompts.len(), 1, "1.5 GiB is above the web's 1 GiB automatic allowance");
        let fields = &approved.ui.dialogs[0].fields;
        assert_eq!(fields["available"], json!(null));
        assert_eq!(fields["capacityUnknown"], json!(true), "the browser cannot measure the tab's capacity");
        let dlg = approved.ui.dialogs[0].id;
        assert_eq!(call(&mut approved, "ui.dialog.confirm", json!({"dialog": dlg}))["ok"], true);
        assert_eq!(approved.session.documents().len(), 1, "consent opens an estimate within the product cap");
    });
    let mut refused = app();
    photocraft_io::psd_budget::with_test_estimate_charge(3 * gb, || {
        assert!(refused.open_bytes_from("too-big.psd", &psd, Some("too-big.psd")).is_err(), "consent cannot raise the 2 GiB product cap");
        assert!(refused.ui.dialogs.is_empty());
        assert!(refused.session.documents().is_empty());
    });
}

/// #10897: the prompt shows an unmeasured capacity honestly (no invented number) and adds the
/// above-8-GiB warning, both from the engine's typed flags, so the browser and desktop consent
/// text matches what was actually measured.
#[test]
fn memory_prompt_states_unknown_capacity_and_the_eight_gib_warning() {
    let gb = 1u64 << 30;
    let mut app = app();
    let id = store_preflight(&mut app, "unknown.psd", 9 * gb, None);
    let tab = crate::jobs_ui::OpenTab {
        job: photocraft_engine::jobs::JobId(1),
        name: "unknown.psd".into(),
        path: Some("/pics/unknown.psd".into()),
        slot: None,
        order: None,
    };
    crate::jobs_ui::finish_open_tab(
        &mut app,
        &tab,
        &json!({"pendingPreflight": id, "estimated": 9 * gb, "availableRam": null, "capacityUnknown": true, "uncertainCapacityWarning": true}),
    )
    .unwrap();
    assert_eq!(app.ui.dialogs.len(), 1);
    let fields = &app.ui.dialogs[0].fields;
    assert!(crate::open_prompt::owns_memory(fields));
    assert_eq!(fields["available"], json!(null), "no number is invented for an unmeasured capacity");
    assert_eq!(fields["capacityUnknown"], json!(true));
    assert_eq!(fields["uncertainCapacityWarning"], json!(true));
    let message = fields["message"].as_str().unwrap();
    assert!(message.contains("could not be measured"), "{message}");
    assert!(message.contains("8 GiB") && message.contains("may run out of memory"), "{message}");
    assert!(!message.contains("available)"), "the unknown-capacity message claims no available figure: {message}");
    let dialog = app.ui.dialogs[0].id;
    assert_eq!(call(&mut app, "ui.dialog.cancel", json!({"dialog": dialog}))["ok"], true);

    // A measured capacity shows the number and drops the uncertainty warning.
    let id = store_preflight(&mut app, "known.psd", 3 * gb, Some(5 * gb));
    let tab = crate::jobs_ui::OpenTab { job: photocraft_engine::jobs::JobId(2), name: "known.psd".into(), path: None, slot: None, order: None };
    crate::jobs_ui::finish_open_tab(
        &mut app,
        &tab,
        &json!({"pendingPreflight": id, "estimated": 3 * gb, "availableRam": 5 * gb, "capacityUnknown": false, "uncertainCapacityWarning": false}),
    )
    .unwrap();
    let fields = &app.ui.dialogs[0].fields;
    assert_eq!(fields["available"], json!(5 * gb));
    assert_eq!(fields["capacityUnknown"], json!(false));
    let message = fields["message"].as_str().unwrap();
    assert!(message.contains("5.0 GiB available"), "{message}");
    assert!(!message.contains("could not be measured") && !message.contains("may run out of memory"), "{message}");
    let dialog = app.ui.dialogs[0].id;
    assert_eq!(call(&mut app, "ui.dialog.cancel", json!({"dialog": dialog}))["ok"], true);
    assert!(app.ui.dialogs.is_empty());
}

/// The prompt warns when opening the file would leave less than 1 GiB of the measured free
/// memory, and stays quiet when there is room to spare.
#[test]
fn memory_prompt_warns_when_little_memory_would_be_left() {
    let gb = 1u64 << 30;
    let mut app = app();
    let available = 3 * gb + gb / 2;
    let id = store_preflight(&mut app, "tight.psd", 3 * gb, Some(available));
    let tab = crate::jobs_ui::OpenTab { job: photocraft_engine::jobs::JobId(3), name: "tight.psd".into(), path: None, slot: None, order: None };
    crate::jobs_ui::finish_open_tab(
        &mut app,
        &tab,
        &json!({"pendingPreflight": id, "estimated": 3 * gb, "availableRam": available, "capacityUnknown": false, "uncertainCapacityWarning": false}),
    )
    .unwrap();
    let message = app.ui.dialogs[0].fields["message"].as_str().unwrap();
    assert!(message.contains("Little memory would be left") && message.contains("may run out of memory"), "{message}");
    let dialog = app.ui.dialogs[0].id;
    assert_eq!(call(&mut app, "ui.dialog.cancel", json!({"dialog": dialog}))["ok"], true);
}

/// #10897: reopening an ordinary file whose high-memory prompt is already waiting shows that
/// prompt instead of queueing a second one and a second open; a template still opens again as
/// another Untitled, like one that is still loading.
#[test]
fn reopening_a_file_whose_memory_prompt_waits_does_not_queue_a_duplicate() {
    let gb = 1u64 << 30;
    let mut app = app();
    let id = store_preflight(&mut app, "slow.psd", 3 * gb, Some(5 * gb));
    crate::open_prompt::arrive_memory(&mut app, id, "slow.psd".into(), Some("/pics/slow.psd".into()), None, None, 3 * gb, Some(5 * gb), false, false);
    assert_eq!(app.held_memory_prompts.len(), 1);
    assert_eq!(app.ui.dialogs.len(), 1);
    assert!(app.show_if_open("/pics/slow.psd"), "the waiting prompt is shown");
    assert_eq!(app.held_memory_prompts.len(), 1, "no second prompt for the same file");
    assert_eq!(app.ui.dialogs.len(), 1);

    // The template exception: a repeat open still creates another Untitled.
    let id = store_preflight(&mut app, "card.psdt", 3 * gb, Some(5 * gb));
    crate::open_prompt::arrive_memory(&mut app, id, "card.psdt".into(), Some("/pics/card.psdt".into()), None, None, 3 * gb, Some(5 * gb), false, false);
    assert_eq!(app.held_memory_prompts.len(), 2);
    assert!(!app.show_if_open("/pics/card.psdt"), "a template reopens instead of reusing the waiting prompt");
    assert_eq!(app.held_memory_prompts.len(), 2);
}

/// A tiny PNG so a set can hold one PSD that needs approval and one flat file that opens at once.
#[cfg(not(target_arch = "wasm32"))]
fn png_file() -> Vec<u8> {
    let img = photocraft_codecs::Image::from_u8(2, 2, photocraft_codecs::ChannelLayout::Rgb, vec![10; 2 * 2 * 3]).unwrap();
    photocraft_codecs::encode(&img, photocraft_codecs::Format::Png, &Default::default()).unwrap()
}

/// #10897: a set opened inline keeps the charged PSD's place in the set through its memory
/// prompt: the prompt carries the batch order and the answered file lands before the later member.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn inline_set_keeps_the_memory_prompted_file_in_batch_order() {
    if cfg!(target_pointer_width = "32") {
        return; // the 3 GiB charge would exceed a 32-bit process cap
    }
    let gb = 1u64 << 30;
    let dir = tempfile::tempdir().unwrap();
    let psd = dir.path().join("a_high.psd");
    let png = dir.path().join("b_flat.png");
    std::fs::write(&psd, recovery_psd::make_psd(false)).unwrap();
    std::fs::write(&png, png_file()).unwrap();
    let paths = vec![psd.to_string_lossy().into_owned(), png.to_string_lossy().into_owned()];
    let mut app = app();
    let snap = photocraft_io::memory::MemorySnapshot { available_ram: Some(5 * gb), swap_free: None, commit_headroom: None };
    photocraft_io::psd_budget::with_test_estimate_charge(3 * gb, || {
        photocraft_io::memory::with_mock_memory(snap, || {
            assert_eq!(app.open_paths(&paths), 2);
            assert_eq!(app.held_memory_prompts.len(), 1, "the charged PSD waits for approval");
            assert_eq!(app.held_memory_prompts.front().unwrap().order.map(|o| o.seq), Some(0), "the set order travels with the prompt");
            // The flat file waits behind the prompt, keeping its own place in the set.
            assert_eq!(app.held_opens.len(), 1);
            assert_eq!(app.held_opens.front().unwrap().order.map(|o| o.seq), Some(1));
            let dlg = app.ui.dialogs[0].id;
            assert_eq!(call(&mut app, "ui.dialog.confirm", json!({"dialog": dlg}))["ok"], true);
            let names: Vec<_> = app.session.documents().iter().map(|d| d.doc.name.clone()).collect();
            assert_eq!(names, ["a_high.psd", "b_flat.png"]);
            assert!(app.held_memory_prompts.is_empty() && app.held_opens.is_empty());
            assert!(app.jobs.batches.is_empty(), "the set is forgotten once every member opened");
            assert!(app.prompt_order.is_empty());
        });
    });
}

/// A real tiny layered TIFF: a small two-layer RGB document exported with Photoshop layer data.
/// The fixture is the actual file the UI opens, not a fabricated preflight.
#[cfg(not(target_arch = "wasm32"))]
fn exported_layered_tiff(w: u32, h: u32) -> Vec<u8> {
    use photocraft_doc::{Document, Layer, LayerContent};
    use photocraft_raster::Surface;

    let mut doc = Document::new("layers.tif", photocraft_geom::Size::new(w, h), photocraft_color::ColorMode::Rgb, photocraft_color::SampleType::U8);
    let fmt = doc.pixel_format();
    let bytes = (w as usize * h as usize * fmt.bytes_per_pixel()).max(1);
    for name in ["bottom", "top"] {
        let mut s = Surface::new(fmt);
        s.write_interleaved(photocraft_geom::Rect::new(0, 0, w as i32, h as i32), &vec![96u8; bytes]);
        doc.layers.push(Layer::new(name, LayerContent::Raster(s)));
    }
    photocraft_io::export(&doc, "layers.tif", &photocraft_io::ExportOptions { tiff_layers: true, ..Default::default() }).expect("export a layered TIFF").bytes
}

/// #10897: a layered TIFF opened inline reaches the same high-memory prompt a PSD does. Only the
/// typed IO path carries `PsdApprovalRequired`; the shell's plain import service turns it into a
/// terminal string, so without that routing this open fails instead of prompting. The real
/// two-layer fixture plus the scoped estimate charge land in the confirmation band under a fixed
/// 5 GiB, and the prompt arrives before any composite pixel is decoded.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn inline_layered_tiff_prompts_before_decoding_and_opens_or_declines() {
    if cfg!(target_pointer_width = "32") {
        return; // the 3 GiB charge would exceed a 32-bit process cap
    }
    let gb = 1u64 << 30;
    let bytes = exported_layered_tiff(8, 6);
    let path = "/pics/layered.tif";
    let mut app = app();
    assert!(!app.background_jobs, "the inline path the web shell uses");
    let snap = photocraft_io::memory::MemorySnapshot { available_ram: Some(5 * gb), swap_free: None, commit_headroom: None };
    photocraft_io::psd_budget::with_test_estimate_charge(3 * gb, || {
        photocraft_io::memory::with_mock_memory(snap, || {
            let order = crate::jobs_ui::OpenOrder { batch: crate::jobs_ui::begin_batch(&mut app), seq: 0 };
            app.open_bytes_placed("layered.tif", &bytes, Some(path), None, Some(order)).unwrap();
            // The prompt comes first: nothing is decoded, no tab is opened, and the engine holds
            // the source for the approval. The set order travels with the prompt.
            assert!(app.session.documents().is_empty(), "the layered TIFF waits for approval instead of opening");
            assert!(app.jobs.opens.is_empty(), "inline: no background tab");
            assert_eq!(app.held_memory_prompts.len(), 1);
            assert_eq!(app.held_memory_prompts.front().unwrap().order, Some(order));
            assert_eq!(app.ui.dialogs.len(), 1);
            let fields = &app.ui.dialogs[0].fields;
            assert!(crate::open_prompt::owns_memory(fields));
            assert!(fields["estimated"].as_u64().unwrap() > 3 * gb, "the charged estimate is shown: {fields:?}");
            assert_eq!(fields["available"], json!(5 * gb));
            assert!(app.session.pending_preflight_bytes().unwrap() > 0, "the engine preflight holds the source");

            // Accepting runs the typed TIFF import with the one-shot approval: both layers, in
            // their order, keep the document's path, and the preflight is released.
            let dlg = app.ui.dialogs[0].id;
            let res = call(&mut app, "ui.dialog.confirm", json!({"dialog": dlg}));
            assert_eq!(res["ok"], true, "{res}");
            assert_eq!(app.session.documents().len(), 1, "the approved TIFF opens");
            let st = app.session.active().unwrap();
            assert_eq!(st.doc.name, "layered.tif");
            assert_eq!(st.path.as_deref(), Some(path), "the approved open keeps its path");
            let names: Vec<_> = st.doc.layers.iter().map(|l| l.name.clone()).collect();
            assert_eq!(names, ["bottom", "top"], "both layers open in their order");
            assert!(app.ui.dialogs.is_empty(), "a TIFF has no damage prompt");
            assert!(app.held_memory_prompts.is_empty());
            assert!(app.jobs.batches.is_empty(), "the set is forgotten once its prompted file opened");
            assert_eq!(app.session.pending_preflight_bytes().unwrap(), 0, "accepting clears the engine preflight");

            // Declining opens nothing and releases the engine preflight too.
            app.open_bytes_from("declined.tif", &bytes, Some("/pics/declined.tif")).unwrap();
            assert_eq!(app.held_memory_prompts.len(), 1);
            assert!(app.session.pending_preflight_bytes().unwrap() > 0);
            let dlg = app.ui.dialogs[0].id;
            assert_eq!(call(&mut app, "ui.dialog.cancel", json!({"dialog": dlg}))["ok"], true);
            assert!(app.held_memory_prompts.is_empty() && app.ui.dialogs.is_empty());
            assert_eq!(app.session.documents().len(), 1, "declining opens nothing");
            assert_eq!(app.session.pending_preflight_bytes().unwrap(), 0, "declining clears the engine preflight");
        });
    });
}

/// A TIFF whose Photoshop layer tag cannot be read still opens flattened through the typed path,
/// with its warning: routing TIFF magic to typed IO must not turn a malformed tag into a refusal.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn inline_tiff_with_a_malformed_layer_tag_opens_flattened() {
    use photocraft_codecs::{ChannelLayout, EncodeOptions, Format, Image, SampleType};

    let mut img = Image::from_raw(4, 3, ChannelLayout::Rgb, SampleType::U8, vec![40; 4 * 3 * 3]).unwrap();
    img.meta.photoshop_layers = Some(b"not Photoshop image source data".to_vec());
    let bytes = photocraft_codecs::encode(&img, Format::Tiff, &EncodeOptions::default()).unwrap();
    let mut app = app();
    let snap = photocraft_io::memory::MemorySnapshot { available_ram: Some(16 << 30), swap_free: None, commit_headroom: None };
    photocraft_io::memory::with_mock_memory(snap, || {
        let warnings = app.open_bytes("bogus.tif", &bytes).unwrap();
        assert_eq!(app.session.documents().len(), 1, "the composite still opens");
        assert_eq!(app.session.active().unwrap().doc.layers.len(), 1, "a malformed tag opens one flat layer");
        assert!(warnings.iter().any(|w| w.contains("opened flattened")), "{warnings:?}");
        assert!(app.ui.dialogs.is_empty());
    });
}

/// #10897: a file dropped on a tab slot keeps that slot through an inline memory prompt, even
/// though the open completes later (before, it appended to the end of the tab strip).
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn inline_drop_slot_survives_the_memory_prompt() {
    if cfg!(target_pointer_width = "32") {
        return; // the 3 GiB charge would exceed a 32-bit process cap
    }
    let gb = 1u64 << 30;
    let mut app = app();
    for name in ["first.psd", "second.psd"] {
        app.open_bytes(name, &recovery_psd::make_psd(false)).unwrap();
    }
    assert_eq!(app.session.documents().len(), 2);
    let snap = photocraft_io::memory::MemorySnapshot { available_ram: Some(5 * gb), swap_free: None, commit_headroom: None };
    photocraft_io::psd_budget::with_test_estimate_charge(3 * gb, || {
        photocraft_io::memory::with_mock_memory(snap, || {
            app.open_bytes_placed("dropped.psd", &recovery_psd::make_psd(false), None, Some(0), None).unwrap();
            assert_eq!(app.held_memory_prompts.len(), 1);
            assert_eq!(app.held_memory_prompts.front().unwrap().slot, Some(0), "the drop slot travels with the prompt");
            let dlg = app.ui.dialogs[0].id;
            assert_eq!(call(&mut app, "ui.dialog.confirm", json!({"dialog": dlg}))["ok"], true);
            let names: Vec<_> = app.session.documents().iter().map(|d| d.doc.name.clone()).collect();
            assert_eq!(names, ["dropped.psd", "first.psd", "second.psd"], "the approved drop keeps its tab slot");
        });
    });
}

/// A damaged PSD opened as a smart object, once confirmed, is admitted as one: the history says
/// "Open As Smart Object" and two same-stem files get distinct names (#12212 review).
#[test]
fn confirmed_damaged_smart_object_opens_are_admitted_as_smart_objects_with_distinct_names() {
    let mut app = app();
    let bytes = recovery_psd::make_psd(true);
    let dir = std::env::temp_dir().join(format!("photocraft-damaged-so-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let mut paths = Vec::new();
    for sub in ["a", "b"] {
        std::fs::create_dir_all(dir.join(sub)).unwrap();
        let path = dir.join(sub).join("dmg.psd");
        std::fs::write(&path, &bytes).unwrap();
        paths.push(path.to_string_lossy().into_owned());
    }
    for path in &paths {
        crate::jobs_ui::start_open_as_smart_object(&mut app, path, None).unwrap();
    }
    assert!(app.session.documents().is_empty());
    for _ in 0..2 {
        let id = damage_dialog(&mut app);
        assert_eq!(call(&mut app, "ui.dialog.confirm", json!({"dialog": id}))["ok"], true);
    }
    let names: Vec<_> = app.session.documents().iter().map(|d| d.doc.name.clone()).collect();
    assert_eq!(names, ["dmg-1", "dmg-2"]);
    for st in app.session.documents() {
        assert_eq!(st.history.entries(), ["Open As Smart Object"]);
    }
    let _ = std::fs::remove_dir_all(&dir);
}
