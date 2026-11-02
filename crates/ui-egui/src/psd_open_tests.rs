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

    crate::open_prompt::arrive_memory(&mut app, preflight_id, "high_mem.psd".into(), None, None, None, 7 * gb, Some(10 * gb));
    assert_eq!(app.ui.dialogs.len(), 1);
    assert!(crate::open_prompt::owns_memory(&app.ui.dialogs[0].fields));
    let dlg_id = app.ui.dialogs[0].id;

    assert_eq!(call(&mut app, "ui.dialog.cancel", json!({"dialog": dlg_id}))["ok"], true);
    assert!(app.ui.dialogs.is_empty());
    assert!(app.session.take_pending_preflight(preflight_id).is_none(), "pending preflight dropped on cancel");
    assert!(app.held_memory_prompts.is_empty());
    assert!(app.prompt_order.is_empty());
}

#[test]
fn memory_confirmation_resume_inline_success() {
    let mut app = app();
    app.background_jobs = false;
    let psd = recovery_psd::make_psd(false);
    let (file, _) = photocraft_psd::PsdFile::from_bytes_and_merged(&psd, |_| false).unwrap();
    let est = photocraft_io::psd_budget::estimate(&file).unwrap().total_bytes;
    let bytes = std::sync::Arc::new(psd);
    let fp = photocraft_engine::jobs::SourceFingerprint::of(&photocraft_engine::jobs::OpenSource::Bytes(std::sync::Arc::clone(&bytes))).unwrap();

    let snap = photocraft_io::memory::MemorySnapshot { available_ram: Some((4 * est).max(4 << 30)), swap_free: None, commit_headroom: None };
    photocraft_io::memory::with_mock_memory(snap, || {
        let budget = photocraft_io::memory::resolve_budget(snap, None);
        let preflight_id = app
            .session
            .store_pending_preflight(photocraft_engine::jobs::PendingPsdPreflight {
                id: 0,
                name: "inline_resume.psd".into(),
                source: photocraft_engine::jobs::OpenSource::Bytes(bytes),
                fingerprint: fp,
                required: est,
                soft_limit: budget.soft_limit,
                hard_limit: budget.hard_limit,
                available_ram: budget.available_ram,
                confirm_damage: true,
            })
            .unwrap();

        crate::open_prompt::arrive_memory(
            &mut app,
            preflight_id,
            "inline_resume.psd".into(),
            Some("/test/inline_resume.psd".into()),
            None,
            None,
            est,
            budget.available_ram,
        );
        assert_eq!(app.ui.dialogs.len(), 1);
        let dlg_id = app.ui.dialogs[0].id;

        let res = call(&mut app, "ui.dialog.confirm", json!({"dialog": dlg_id}));
        assert_eq!(res["ok"], true);
        assert!(app.ui.dialogs.is_empty(), "dialog dismissed after inline resume");
        assert_eq!(app.session.documents().len(), 1, "document opened via inline resume");
        assert_eq!(app.session.documents()[0].doc.name, "inline_resume.psd");
        assert_eq!(app.session.documents()[0].path.as_deref(), Some("/test/inline_resume.psd"));
        assert!(app.held_memory_prompts.is_empty());
        assert!(app.held_opens.is_empty());
        assert!(app.prompt_order.is_empty());
        assert!(app.jobs.opens.is_empty());
    });
}

#[test]
fn memory_confirmation_resume_background_success() {
    let mut app = app();
    app.background_jobs = true;
    let psd = recovery_psd::make_psd(false);
    let (file, _) = photocraft_psd::PsdFile::from_bytes_and_merged(&psd, |_| false).unwrap();
    let est = photocraft_io::psd_budget::estimate(&file).unwrap().total_bytes;
    let bytes = std::sync::Arc::new(psd);
    let fp = photocraft_engine::jobs::SourceFingerprint::of(&photocraft_engine::jobs::OpenSource::Bytes(std::sync::Arc::clone(&bytes))).unwrap();

    let snap = photocraft_io::memory::MemorySnapshot { available_ram: Some((4 * est).max(4 << 30)), swap_free: None, commit_headroom: None };
    photocraft_io::memory::with_mock_memory(snap, || {
        let budget = photocraft_io::memory::resolve_budget(snap, None);
        let preflight_id = app
            .session
            .store_pending_preflight(photocraft_engine::jobs::PendingPsdPreflight {
                id: 0,
                name: "bg_resume.psd".into(),
                source: photocraft_engine::jobs::OpenSource::Bytes(bytes),
                fingerprint: fp,
                required: est,
                soft_limit: budget.soft_limit,
                hard_limit: budget.hard_limit,
                available_ram: budget.available_ram,
                confirm_damage: true,
            })
            .unwrap();

        crate::open_prompt::arrive_memory(
            &mut app,
            preflight_id,
            "bg_resume.psd".into(),
            Some("/test/bg_resume.psd".into()),
            None,
            None,
            est,
            budget.available_ram,
        );
        assert_eq!(app.ui.dialogs.len(), 1);
        let dlg_id = app.ui.dialogs[0].id;

        let res = call(&mut app, "ui.dialog.confirm", json!({"dialog": dlg_id}));
        assert_eq!(res["ok"], true);
        assert_eq!(app.jobs.opens.len(), 1, "background open job registered in jobs.opens");

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

    crate::open_prompt::arrive_memory(&mut app, preflight_id, "changed.psd".into(), Some(path.to_string_lossy().to_string()), None, None, 1000, Some(100_000));
    assert_eq!(app.ui.dialogs.len(), 1);
    let dlg_id = app.ui.dialogs[0].id;

    // Mutate file before confirming:
    std::fs::write(&path, b"modified file contents longer").unwrap();

    let res = call(&mut app, "ui.dialog.confirm", json!({"dialog": dlg_id}));
    assert_eq!(res["ok"], false, "confirming changed file inline must fail");
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

    crate::open_prompt::arrive_memory(&mut app, preflight_id, "declined.psd".into(), None, None, Some(order), 1000, Some(4000));
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

    crate::open_prompt::arrive_memory(&mut app, p0, "first.psd".into(), None, None, Some(order0), 1000, Some(4000));
    crate::open_prompt::arrive_memory(&mut app, p1, "second.psd".into(), None, None, Some(order1), 1000, Some(4000));

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

    crate::open_prompt::arrive_memory(&mut app, preflight_id, "high_mem.psd".into(), None, None, None, 7 * gb, Some(10 * gb));

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
            question: Some(json!({"missing": false, "working": "sRGB", "embedded": "Adobe RGB"})),
            policy_applied: false,
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
            question: None,
            policy_applied: false,
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
    crate::open_prompt::arrive_memory(&mut app, preflight_id, "second_mem.psd".into(), None, None, None, 7 * gb, Some(10 * gb));

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

#[test]
fn normal_image_open_does_not_require_timestamps_or_preflight_fingerprints() {
    let mut app = app();
    // A synthetic 4x4 PNG/raw/flat image opens without timestamp or preflight checks
    let bytes = std::sync::Arc::new(vec![0u8; 32]);
    let src = photocraft_engine::jobs::OpenSource::Bytes(bytes);
    // SourceFingerprint matches on same Bytes Arc without re-reading
    let fp1 = photocraft_engine::jobs::SourceFingerprint::of(&src).unwrap();
    let fp2 = photocraft_engine::jobs::SourceFingerprint::of(&src).unwrap();
    assert_eq!(fp1, fp2);
    assert!(fp1.matches(&fp2));
}
