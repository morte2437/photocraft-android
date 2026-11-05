use super::*;

fn tmp(name: &str) -> String {
    let d = std::env::temp_dir().join(format!("photocraft-file-cmds-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d.to_string_lossy().into_owned()
}

fn session(w: u32, h: u32, depth: u32) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": w, "height": h, "depth": depth})).unwrap();
    s
}

fn doc(s: &Session) -> &Document {
    &s.active().unwrap().doc
}

/// Writes a solid-colour PNG of `w × h` and returns its path.
fn png(dir: &str, name: &str, w: u32, h: u32, color: &str) -> String {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": w, "height": h, "background": color})).unwrap();
    let path = join(dir, name);
    save_doc(doc(&s), &path, None).unwrap();
    path
}

fn composite(s: &Session, x: i32, y: i32) -> [f32; 4] {
    let b = photocraft_compose::render(doc(s), Rect::new(x, y, x + 1, y + 1));
    b.px[0]
}

fn batch_log(path: &str) -> String {
    let bytes = std::fs::read(path).unwrap();
    assert!(bytes.starts_with(&[0xff, 0xfe]), "Batch log needs a UTF-16 LE BOM");
    let (pairs, remainder) = bytes[2..].as_chunks::<2>();
    assert!(remainder.is_empty(), "UTF-16 bytes must have even length");
    let words: Vec<u16> = pairs.iter().map(|pair| u16::from_le_bytes(*pair)).collect();
    String::from_utf16(&words).unwrap()
}

#[test]
fn batch_stop_on_error_reports_skipped_inputs_and_writes_the_error_log() {
    let dir = tmp("batch-stop-error-log");
    let good = png(&dir, "good.png", 8, 6, "#00ff00");
    let missing = join(&dir, "missing.png");
    let output = join(&dir, "out");
    let log = join(&dir, "errors.txt");
    let result = Session::new()
        .execute("file.automate.batch", json!({"steps": [], "input": [missing, good], "output": output, "stopOnError": true, "errorLog": log}))
        .unwrap();
    assert_eq!(result["files"], json!([]), "{result}");
    assert_eq!(result["errors"].as_array().map(Vec::len), Some(1), "{result}");
    assert_eq!(result["stopped"], json!(missing), "{result}");
    assert_eq!(result["skipped"], json!([good]), "{result}");
    let text = batch_log(&log);
    assert!(text.starts_with("Batch Start\r\n\r\n") && text.contains("missing.png") && text.contains("File: \"") && text.contains("Error: "), "{text}");
    assert!(text.ends_with("Batch End\r\n"), "{text}");
    assert!(!std::path::Path::new(&join(&output, "good.png")).exists());
}

#[test]
fn batch_writes_a_utf16_error_log_even_when_every_file_succeeds() {
    let dir = tmp("batch-clean-log");
    let input = png(&dir, "good.png", 8, 6, "#00ff00");
    let output = join(&dir, "out");
    let log = join(&dir, "errors.txt");
    let result = Session::new().execute("file.automate.batch", json!({"steps": [], "input": [input], "output": output, "errorLog": log})).unwrap();
    assert_eq!(result["errors"], json!([]), "{result}");
    assert_eq!(result["errorLog"], json!(log));
    assert_eq!(batch_log(&log), "Batch Start\r\n\r\nBatch End\r\n");
}

#[test]
fn batch_cancelled_during_an_action_does_not_save_that_input() {
    let dir = tmp("batch-cancel-current");
    let input = png(&dir, "source.png", 8, 6, "#ff0000");
    let out = join(&dir, "out/source.png");
    let ctx = crate::jobs::JobCtx::new();
    let policy = super::batch_run::BatchPolicy { stop_on_error: false, error_log: None };
    let result = super::batch_run::process_batch_outputs(72.0, vec![(input, out.clone())], "png", SaveOpts::default(), &policy, &ctx, &|_, _| {
        ctx.cancel();
        Ok(false)
    })
    .unwrap();
    assert_eq!(result["files"], json!([]), "{result}");
    assert_eq!(result["cancelled"], json!(true), "{result}");
    assert!(!std::path::Path::new(&out).exists());
}

/// An action that saved and then closed its document has its saved snapshot written by the time
/// it returns: a cancel landing then counts that file as written, not skipped.
#[test]
fn batch_cancel_after_a_saved_snapshot_is_written_keeps_that_file() {
    let dir = tmp("batch-cancel-after-snapshot");
    let first = png(&dir, "first.png", 8, 6, "#ff0000");
    let second = png(&dir, "second.png", 8, 6, "#00ff00");
    let first_out = join(&dir, "out/first.png");
    let ctx = crate::jobs::JobCtx::new();
    let policy = super::batch_run::BatchPolicy { stop_on_error: false, error_log: None };
    let outputs = vec![(first, first_out.clone()), (second.clone(), join(&dir, "out/second.png"))];
    let result = super::batch_run::process_batch_outputs(72.0, outputs, "png", SaveOpts::default(), &policy, &ctx, &|s, out| {
        save_doc(&s.active().unwrap().doc, out, None).unwrap();
        ctx.cancel();
        Ok(true)
    })
    .unwrap();
    assert_eq!(result["files"], json!([first_out]), "{result}");
    assert_eq!(result["skipped"], json!([second]), "{result}");
    assert_eq!(result["cancelled"], json!(true), "{result}");
}

/// A cancel that lands while an input fails on its own keeps that failure and its reason; only
/// the inputs after it are skipped.
#[test]
fn batch_cancel_during_a_failing_input_keeps_its_error() {
    let dir = tmp("batch-cancel-failing");
    let first = png(&dir, "first.png", 8, 6, "#ff0000");
    let second = png(&dir, "second.png", 8, 6, "#00ff00");
    let ctx = crate::jobs::JobCtx::new();
    let policy = super::batch_run::BatchPolicy { stop_on_error: false, error_log: None };
    let outputs = vec![(first.clone(), join(&dir, "out/first.png")), (second.clone(), join(&dir, "out/second.png"))];
    let result = super::batch_run::process_batch_outputs(72.0, outputs, "png", SaveOpts::default(), &policy, &ctx, &|_, _| {
        ctx.cancel();
        Err(EngineError::Other("boom".into()))
    })
    .unwrap();
    assert_eq!(result["errors"], json!([{"file": first, "error": "boom"}]), "{result}");
    assert_eq!(result["skipped"], json!([second]), "{result}");
    assert_eq!(result["cancelled"], json!(true), "{result}");
}

/// Esc during an input that fails on its own: Continue records the failure (not an interrupted
/// input) and goes on with the next one.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn batch_interruption_during_a_failing_input_keeps_its_error() {
    use std::time::Duration;

    let dir = tmp("batch-interrupt-failing");
    let first = png(&dir, "first.png", 8, 6, "#ff0000");
    let second = png(&dir, "second.png", 8, 6, "#00ff00");
    let second_out = join(&dir, "out/second.png");
    let ctx = crate::jobs::JobCtx::new();
    let worker_ctx = ctx.clone();
    let outputs = vec![(first.clone(), join(&dir, "out/first.png")), (second, second_out.clone())];
    let worker = std::thread::spawn(move || {
        let policy = super::batch_run::BatchPolicy { stop_on_error: false, error_log: None };
        super::batch_run::process_batch_outputs(72.0, outputs, "png", SaveOpts::default(), &policy, &worker_ctx, &|_, path| {
            if path.ends_with("first.png") {
                assert!(worker_ctx.request_batch_interrupt());
                return Err(EngineError::Other("boom".into()));
            }
            Ok(false)
        })
    });
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while !ctx.batch_waiting_for_decision() && std::time::Instant::now() < deadline {
        std::thread::yield_now();
    }
    assert!(ctx.decide_batch_interrupt(true));
    let result = worker.join().unwrap().unwrap();
    assert_eq!(result["errors"], json!([{"file": first, "error": "boom"}]), "{result}");
    assert_eq!(result["files"], json!([second_out]), "{result}");
    assert!(result.get("skipped").is_none(), "{result}");
}

/// Once the last input is handled, Esc has nothing to interrupt and is refused, so the desktop
/// never waits for a question that will not come.
#[test]
fn batch_refuses_esc_after_its_last_input() {
    let dir = tmp("batch-late-esc");
    let input = png(&dir, "source.png", 8, 6, "#ff0000");
    let ctx = crate::jobs::JobCtx::new();
    let policy = super::batch_run::BatchPolicy { stop_on_error: false, error_log: None };
    let result =
        super::batch_run::process_batch_outputs(72.0, vec![(input, join(&dir, "out/source.png"))], "png", SaveOpts::default(), &policy, &ctx, &|_, _| {
            Ok(false)
        })
        .unwrap();
    assert_eq!(result["errors"], json!([]), "{result}");
    assert!(!ctx.request_batch_interrupt());
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn batch_interruption_continues_with_the_original_next_output() {
    use std::sync::mpsc;
    use std::time::Duration;

    let dir = tmp("batch-interrupt-continue");
    let first = png(&dir, "first.png", 8, 6, "#ff0000");
    let second = png(&dir, "second.png", 8, 6, "#00ff00");
    let first_out = join(&dir, "out/first_001.png");
    let second_out = join(&dir, "out/second_002.png");
    let ctx = crate::jobs::JobCtx::new();
    let worker_ctx = ctx.clone();
    let (opened_tx, opened_rx) = mpsc::channel();
    let (resume_tx, resume_rx) = mpsc::channel();
    let first_action = std::sync::atomic::AtomicBool::new(true);
    let worker = std::thread::spawn(move || {
        let policy = super::batch_run::BatchPolicy { stop_on_error: false, error_log: None };
        super::batch_run::process_batch_outputs(
            72.0,
            vec![(first, first_out.clone()), (second, second_out.clone())],
            "png",
            SaveOpts::default(),
            &policy,
            &worker_ctx,
            &|_, _| {
                if first_action.swap(false, std::sync::atomic::Ordering::AcqRel) {
                    let _ = opened_tx.send(());
                    let _ = resume_rx.recv_timeout(Duration::from_secs(10));
                }
                Ok(false)
            },
        )
        .map(|result| (result, first_out, second_out))
    });
    opened_rx.recv_timeout(Duration::from_secs(10)).unwrap();
    let preview = ctx.batch_preview().expect("the real opened input is published before its action");
    assert_eq!(preview.path, join(&dir, "first.png"));
    assert_eq!((preview.document.size.width, preview.document.size.height), (8, 6));
    assert!(ctx.request_batch_interrupt(), "Esc is accepted while the first action runs");
    resume_tx.send(()).unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while !ctx.batch_waiting_for_decision() && std::time::Instant::now() < deadline {
        std::thread::yield_now();
    }
    assert!(ctx.batch_waiting_for_decision(), "the worker stopped before writing");
    assert!(!std::path::Path::new(&join(&dir, "out/first_001.png")).exists());
    assert!(ctx.decide_batch_interrupt(true));
    let (result, first_out, second_out) = worker.join().unwrap().unwrap();
    assert_eq!(result["files"], json!([second_out]), "Continue keeps the original serial: {result}");
    assert!(!std::path::Path::new(&first_out).exists());
    assert!(std::path::Path::new(&second_out).exists());
    assert!(ctx.batch_preview().is_none(), "the preview closes after the last input");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn batch_stop_after_interruption_preserves_prior_outputs_and_never_saves_current() {
    use std::sync::mpsc;
    use std::time::Duration;

    let dir = tmp("batch-interrupt-stop");
    let first = png(&dir, "first.png", 8, 6, "#ff0000");
    let second = png(&dir, "second.png", 8, 6, "#00ff00");
    let third = png(&dir, "third.png", 8, 6, "#0000ff");
    let out = join(&dir, "out");
    let first_out = join(&out, "first.png");
    let second_out = join(&out, "second.png");
    let third_out = join(&out, "third.png");
    let ctx = crate::jobs::JobCtx::new();
    let worker_ctx = ctx.clone();
    let (opened_tx, opened_rx) = mpsc::channel();
    let (resume_tx, resume_rx) = mpsc::channel();
    let worker = std::thread::spawn(move || {
        let policy = super::batch_run::BatchPolicy { stop_on_error: false, error_log: None };
        super::batch_run::process_batch_outputs(
            72.0,
            vec![(first, first_out), (second, second_out), (third, third_out)],
            "png",
            SaveOpts::default(),
            &policy,
            &worker_ctx,
            &|_, path| {
                if path.ends_with("second.png") {
                    let _ = opened_tx.send(());
                    let _ = resume_rx.recv_timeout(Duration::from_secs(10));
                }
                Ok(false)
            },
        )
    });
    opened_rx.recv_timeout(Duration::from_secs(10)).unwrap();
    assert!(ctx.request_batch_interrupt());
    resume_tx.send(()).unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while !ctx.batch_waiting_for_decision() && std::time::Instant::now() < deadline {
        std::thread::yield_now();
    }
    assert!(ctx.batch_waiting_for_decision());
    assert!(ctx.decide_batch_interrupt(false));
    let result = worker.join().unwrap().unwrap();
    assert_eq!(result["files"], json!([join(&out, "first.png")]), "{result}");
    assert_eq!(result["cancelled"], true, "{result}");
    assert_eq!(result["skipped"], json!([join(&dir, "second.png"), join(&dir, "third.png")]), "{result}");
    assert!(std::path::Path::new(&join(&out, "first.png")).exists());
    assert!(!std::path::Path::new(&join(&out, "second.png")).exists());
    assert!(!std::path::Path::new(&join(&out, "third.png")).exists());
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn batch_continue_keeps_errors_before_and_after_the_interruption_in_one_log() {
    use std::time::Duration;

    let dir = tmp("batch-interrupt-error-log");
    let second = png(&dir, "second.png", 8, 6, "#00ff00");
    let fourth = png(&dir, "fourth.png", 8, 6, "#0000ff");
    let before = join(&dir, "missing-before.png");
    let after = join(&dir, "missing-after.png");
    let out = join(&dir, "out");
    let log = join(&dir, "errors.txt");
    let ctx = crate::jobs::JobCtx::new();
    let decider_ctx = ctx.clone();
    let decider = std::thread::spawn(move || {
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while !decider_ctx.batch_waiting_for_decision() && std::time::Instant::now() < deadline {
            std::thread::yield_now();
        }
        if !decider_ctx.batch_waiting_for_decision() {
            decider_ctx.cancel();
            return false;
        }
        decider_ctx.decide_batch_interrupt(true)
    });
    let policy = super::batch_run::BatchPolicy { stop_on_error: false, error_log: Some(log.clone()) };
    let result = super::batch_run::process_batch_outputs(
        72.0,
        vec![
            (before.clone(), join(&out, "missing-before.png")),
            (second.clone(), join(&out, "second.png")),
            (after.clone(), join(&out, "missing-after.png")),
            (fourth, join(&out, "fourth.png")),
        ],
        "png",
        SaveOpts::default(),
        &policy,
        &ctx,
        &|_, path| {
            if path.ends_with("second.png") {
                assert!(ctx.request_batch_interrupt());
            }
            Ok(false)
        },
    )
    .unwrap();
    assert!(decider.join().unwrap());
    assert_eq!(result["errors"].as_array().map(Vec::len), Some(2), "{result}");
    assert_eq!(result["skipped"], json!([second]), "{result}");
    assert_eq!(result["files"], json!([join(&out, "fourth.png")]), "{result}");
    let text = batch_log(&log);
    assert!(text.contains("missing-before.png") && text.contains("missing-after.png"), "{text}");
    assert!(!text.contains("second.png"), "a skipped input is not an error: {text}");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn batch_continue_between_inputs_still_processes_the_next_input() {
    use std::time::Duration;

    let dir = tmp("batch-interrupt-between-inputs");
    let input = png(&dir, "next.png", 8, 6, "#ff0000");
    let output = join(&dir, "out/next.png");
    let ctx = crate::jobs::JobCtx::new();
    assert!(ctx.request_batch_interrupt());
    let decider_ctx = ctx.clone();
    let decider = std::thread::spawn(move || {
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while !decider_ctx.batch_waiting_for_decision() && std::time::Instant::now() < deadline {
            std::thread::yield_now();
        }
        if !decider_ctx.batch_waiting_for_decision() {
            decider_ctx.cancel();
            return false;
        }
        decider_ctx.decide_batch_interrupt(true)
    });
    let policy = super::batch_run::BatchPolicy { stop_on_error: false, error_log: None };
    let result =
        super::batch_run::process_batch_outputs(72.0, vec![(input, output.clone())], "png", SaveOpts::default(), &policy, &ctx, &|_, _| Ok(false)).unwrap();
    assert!(decider.join().unwrap());
    assert_eq!(result["files"], json!([output]), "the unopened input is still next: {result}");
    assert!(result.get("skipped").is_none(), "{result}");
}

#[test]
fn batch_error_log_cannot_replace_an_input_or_result() {
    let dir = tmp("batch-error-log-collision");
    let input = png(&dir, "source.png", 8, 6, "#ff0000");
    let original = std::fs::read(&input).unwrap();
    let output = join(&dir, "out");
    for log in [input.clone(), join(&output, "source.png"), join(&output, "./source.png"), join(&output, "../out/source.png")] {
        let error = Session::new().execute("file.automate.batch", json!({"steps": [], "input": [input], "output": output, "errorLog": log})).unwrap_err();
        assert!(error.to_string().contains("errorLog"), "{error}");
        assert_eq!(std::fs::read(&input).unwrap(), original);
        assert!(!std::path::Path::new(&join(&output, "source.png")).exists());
    }
}

#[test]
fn batch_naming_rejects_an_input_alias_through_a_missing_parent() {
    let dir = tmp("batch-missing-parent-alias");
    let first = png(&dir, "a.png", 8, 6, "#ff0000");
    let second = png(&dir, "b.png", 8, 6, "#00ff00");
    let original = std::fs::read(&second).unwrap();
    let output = join(&dir, "missing/..");
    let error =
        Session::new().execute("file.automate.batch", json!({"steps": [], "input": [first, second], "output": output, "naming": "b.{ext}"})).unwrap_err();
    assert!(error.to_string().contains("replace an input"), "{error}");
    assert_eq!(std::fs::read(&second).unwrap(), original);
    assert!(!std::path::Path::new(&join(&dir, "missing")).exists());
}

#[test]
fn flat_batch_cannot_replace_a_later_unread_input() {
    let dir = tmp("batch-flat-input-alias");
    let input_png = png(&dir, "a.png", 8, 6, "#0000ff");
    let mut s = session(8, 6, 8);
    save_doc(doc(&s), &join(&dir, "a.jpg"), None).unwrap();
    let before = std::fs::read(&input_png).unwrap();
    let result = s.execute("file.automate.batch", json!({"steps": [], "input": dir, "output": dir, "format": "png"}));
    assert!(result.as_ref().is_err_and(|e| e.to_string().contains("input")), "{result:?}");
    assert_eq!(std::fs::read(&input_png).unwrap(), before);
}

#[cfg(unix)]
#[test]
fn batch_error_log_rejects_a_symlink_parent_alias() {
    use std::os::unix::fs::symlink;
    let dir = tmp("batch-symlink-parent-alias");
    let source_dir = join(&dir, "in");
    std::fs::create_dir_all(&source_dir).unwrap();
    let input = png(&source_dir, "result.png", 8, 6, "#ff0000");
    let other = join(&dir, "other");
    std::fs::create_dir_all(join(&other, "child")).unwrap();
    symlink(join(&other, "child"), join(&dir, "link")).unwrap();
    let output = other;
    let log = join(&dir, "link/../result.png");
    let error = Session::new().execute("file.automate.batch", json!({"steps": [], "input": [input], "output": output, "errorLog": log})).unwrap_err();
    assert!(error.to_string().contains("errorLog"), "{error}");
    assert!(!std::path::Path::new(&join(&output, "result.png")).exists());
}

#[test]
fn batch_error_log_rejects_an_uncreated_output_alias() {
    let dir = tmp("batch-error-log-future-alias");
    let input = png(&dir, "source.png", 8, 6, "#ff0000");
    let output = join(&dir, "new-output");
    std::fs::create_dir_all(join(&output, "alias")).unwrap();
    let log = join(&output, "alias/../source.png");
    assert!(same_destination(&log, join(&output, "source.png")));
    let error = Session::new().execute("file.automate.batch", json!({"steps": [], "input": [input], "output": output, "errorLog": log})).unwrap_err();
    assert!(error.to_string().contains("errorLog"), "{error}");
    assert!(!std::path::Path::new(&join(&dir, "new-output/source.png")).exists());
}

#[cfg(target_os = "linux")]
#[test]
fn linux_distinct_case_destinations_are_not_the_same_file() {
    let dir = tmp("batch-case-sensitive-destinations");
    assert!(!same_destination(join(&dir, "Image.png"), join(&dir, "image.png")));
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
#[test]
fn unicode_case_variants_follow_destination_volume() {
    let dir = tmp("batch-unicode-case-destinations");
    let existing = join(&dir, "Ä-probe");
    std::fs::write(&existing, b"probe").unwrap();
    let volume_aliases_case = same_file(&existing, join(&dir, "ä-probe"));
    assert_eq!(same_destination(join(&dir, "Ä.png"), join(&dir, "ä.png")), volume_aliases_case);
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
#[test]
fn future_folder_case_variants_follow_the_nearest_existing_ancestor() {
    let dir = tmp("batch-future-folder-case");
    let existing = join(&dir, "Ä-probe");
    std::fs::write(&existing, b"probe").unwrap();
    let volume_aliases_case = same_file(&existing, join(&dir, "ä-probe"));
    let first = join(&dir, "new/nested/Image.png");
    let second = join(&dir, "new/nested/image.png");
    assert_eq!(super::paths::case_sensitive_destination(std::path::Path::new(&first)), Some(!volume_aliases_case));
    assert_eq!(same_destination(first, second), volume_aliases_case);
    assert!(!std::path::Path::new(&join(&dir, "new")).exists());
}

#[test]
fn batch_open_source_edits_each_live_document_and_restores_the_active_one() {
    let dir = tmp("batch-open-documents");
    let mut session = Session::new();
    session.execute("file.new", json!({"name": "First", "width": 8, "height": 6})).unwrap();
    session.execute("file.new", json!({"name": "Second", "width": 12, "height": 5})).unwrap();
    session.set_active(0);
    let result = session
        .execute(
            "file.automate.batch",
            json!({"source": "open", "steps": [["image.imageRotation.90cw", {}]], "output": dir, "format": "png", "naming": "{serial:2}_{NAME}.{EXT}"}),
        )
        .unwrap();
    assert!(result["errors"].as_array().unwrap().is_empty(), "{result}");
    assert_eq!(result["documents"], 2);
    assert_eq!(result["files"].as_array().map(Vec::len), Some(2), "{result}");
    assert_eq!(session.active_index(), Some(0));
    assert_eq!((session.documents()[0].doc.size.width, session.documents()[0].doc.size.height), (6, 8));
    assert_eq!((session.documents()[1].doc.size.width, session.documents()[1].doc.size.height), (5, 12));
    assert!(std::path::Path::new(&join(&dir, "01_FIRST.PNG")).is_file());
    assert!(std::path::Path::new(&join(&dir, "02_SECOND.PNG")).is_file());
}

#[test]
fn batch_open_source_exports_later_edits_if_saved_document_stays_active() {
    let dir = tmp("batch-open-save-point");
    let mut session = Session::new();
    session.execute("file.new", json!({"name": "First", "width": 8, "height": 6})).unwrap();
    session.execute("file.new", json!({"name": "Second", "width": 12, "height": 5})).unwrap();
    let result = session
        .execute(
            "file.automate.batch",
            json!({
                "source": "open", "steps": [["document.activate", {"document": 1}], ["file.save", {}], ["image.imageRotation.90cw", {}]],
                "output": dir, "format": "png", "naming": "{name}.{ext}"
            }),
        )
        .unwrap();
    assert!(result["errors"].as_array().unwrap().is_empty(), "{result}");
    let mut opened = Session::new();
    opened.execute("file.openAs", json!({"path": join(&dir, "First.png"), "as": "png"})).unwrap();
    assert_eq!((doc(&opened).size.width, doc(&opened).size.height), (5, 12), "the still-active saved document includes later edits");
}

#[test]
fn batch_open_source_save_as_then_close_ask_writes_the_result() {
    let dir = tmp("batch-open-save-close-ask");
    let mut session = Session::new();
    session.execute("file.new", json!({"name": "First", "width": 8, "height": 6})).unwrap();
    let result = session
        .execute(
            "file.automate.batch",
            json!({"source": "open", "steps": [["image.adjustments.invert", {}], ["file.saveAs", {}], ["file.close", {"save": "ask"}]], "output": dir, "format": "png"}),
        )
        .unwrap();
    assert_eq!(result["errors"], json!([]), "{result}");
    assert_eq!(result["files"], json!([join(&dir, "First.png")]), "{result}");
    assert!(session.documents().is_empty());
}

#[test]
fn batch_open_source_refuses_a_result_that_replaces_another_open_file() {
    let dir = tmp("batch-open-replaces-input");
    let mut session = Session::new();
    // Saved as PNG under its default name, `a.jpg`'s result is the open `a.png`'s file.
    for (name, format) in [("a.jpg", "jpg"), ("a.png", "png")] {
        let path = png(&dir, name, 8, 6, "#ff0000");
        session.execute("file.openAs", json!({"path": path, "as": format})).unwrap();
    }
    let before = std::fs::read(join(&dir, "a.png")).unwrap();
    let error = session
        .execute("file.automate.batch", json!({"source": "open", "steps": [["image.imageRotation.90cw", {}]], "output": dir, "format": "png"}))
        .unwrap_err();
    assert!(error.to_string().contains("replace another input"), "{error}");
    assert_eq!(std::fs::read(join(&dir, "a.png")).unwrap(), before, "nothing is written before the refusal");
}

#[test]
fn batch_open_source_save_step_leaves_an_unwritten_document_changed() {
    let mut session = Session::new();
    session.execute("file.new", json!({"name": "First", "width": 8, "height": 6})).unwrap();
    let steps = json!([["image.adjustments.invert", {}], ["file.save", {}]]);
    // No destination: nothing is written.
    let result = session.execute("file.automate.batch", json!({"source": "open", "steps": steps})).unwrap();
    assert_eq!(result["errors"], json!([]), "{result}");
    assert!(session.documents()[0].is_dirty(), "a document Batch did not write stays changed");
    // A destination that can't be written: the result's name is taken by a folder, so the path
    // resolves on every platform and the write itself fails.
    let dir = tmp("batch-open-save-fails");
    std::fs::create_dir_all(join(&dir, "First.png")).unwrap();
    let result = session.execute("file.automate.batch", json!({"source": "open", "steps": steps, "output": dir, "format": "png"})).unwrap();
    assert_eq!(result["errors"].as_array().map(Vec::len), Some(1), "{result}");
    assert!(session.documents()[0].is_dirty(), "a failed write leaves the document changed");
}

#[cfg(unix)]
#[test]
fn batch_error_log_rejects_a_dangling_link_to_a_planned_output() {
    use std::os::unix::fs::symlink;
    let dir = tmp("batch-dangling-error-log-alias");
    let input = png(&dir, "source.png", 8, 6, "#ff0000");
    let output = join(&dir, "out");
    let result = join(&output, "source.png");
    let log = join(&dir, "errors.txt");
    symlink(&result, &log).unwrap();
    let error = Session::new().execute("file.automate.batch", json!({"steps": [], "input": [input], "output": output, "errorLog": log})).unwrap_err();
    assert!(error.to_string().contains("errorLog"), "{error}");
    assert!(!std::path::Path::new(&result).exists(), "the batch must not write before rejecting the alias");
}

#[test]
fn batch_reports_file_failures_even_if_error_log_cannot_be_written() {
    let dir = tmp("batch-error-log-write-fails");
    let mut session = session(8, 6, 8);
    let result = session
        .execute(
            "file.automate.batch",
            json!({
                "source": "open", "steps": [["layer.select", {"layer": "does not exist"}]],
                "output": dir, "format": "png", "errorLog": dir
            }),
        )
        .unwrap();
    assert_eq!(result["files"], json!([]));
    assert_eq!(result["errors"].as_array().map(Vec::len), Some(1), "{result}");
    assert!(result["errorLogError"].as_str().is_some(), "{result}");
}

#[cfg(unix)]
#[test]
fn recursive_batch_reports_an_unreadable_subfolder() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tmp("batch-unreadable-subfolder");
    let sub = join(&dir, "locked");
    std::fs::create_dir_all(&sub).unwrap();
    png(&sub, "image.png", 8, 6, "#ff0000");
    std::fs::set_permissions(&sub, std::fs::Permissions::from_mode(0o000)).unwrap();
    let inaccessible = std::fs::read_dir(&sub).is_err();
    let result = list_images_tree(&dir, "", "file.automate.batch");
    std::fs::set_permissions(&sub, std::fs::Permissions::from_mode(0o700)).unwrap();
    if inaccessible {
        assert!(result.is_err(), "an inaccessible input folder must not be omitted");
    } else {
        eprintln!("permission regression skipped: this user can read a mode-000 folder");
        assert!(result.is_ok(), "a privileged reader should still process the folder");
    }
}

#[cfg(unix)]
#[test]
fn folder_listing_reports_metadata_denial_and_skips_broken_links() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let dir = tmp("batch-list-metadata-denial");
    let image = png(&dir, "image.png", 8, 6, "#ff0000");
    symlink(join(&dir, "gone.png"), join(&dir, "broken.png")).unwrap();
    assert_eq!(list_images(&dir).unwrap(), std::slice::from_ref(&image), "broken links are not inputs");
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o400)).unwrap();
    let denied = std::fs::metadata(&image).is_err();
    let result = list_images(&dir);
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
    if denied {
        assert!(result.is_err(), "metadata denial must not turn a folder of images into an empty batch");
    } else {
        eprintln!("metadata denial regression skipped: this user can search a mode-0400 folder");
        assert_eq!(result.unwrap(), [image]);
    }
}

#[test]
fn close_all_and_close_others() {
    let mut s = session(8, 8, 8);
    s.execute("file.new", json!({"width": 9, "height": 9})).unwrap();
    s.execute("file.new", json!({"width": 10, "height": 10})).unwrap();
    s.set_active(1);
    assert_eq!(s.execute("file.closeOthers", json!({})).unwrap()["closed"], 2);
    assert_eq!(s.documents().len(), 1);
    assert_eq!(doc(&s).size.width, 9);
    assert_eq!(s.active_index(), Some(0));
    assert!(s.execute("file.closeOthers", json!({"document": 5})).is_err());
    s.execute("file.new", json!({})).unwrap();
    assert_eq!(s.execute("file.closeAll", json!({})).unwrap()["closed"], 2);
    assert!(s.documents().is_empty());
    assert!(!s.is_enabled("file.closeAll"));
}

#[test]
fn camera_raw_opens_as_16_bit_with_notes() {
    use photocraft_raw::testgen::{DngSpec, mosaic, scene};
    let (w, h) = (24, 16);
    let mut spec = DngSpec::cfa(w, h, mosaic(&scene(w, h), w, [0, 1, 1, 2], 0, 65535));
    spec.as_shot_neutral = Some([0.5, 1.0, 0.7]);
    let mut s = Session::new();
    let r = open_bytes_as(&mut s, "IMG_0001.dng", &spec.build(), None, None).unwrap();
    assert!(r["warnings"].as_array().is_some_and(|w| w.iter().any(|m| m.as_str().is_some_and(|m| m.contains("DNG")))), "{r}");
    assert_eq!((doc(&s).size.width, doc(&s).size.height), (24, 16));
    assert_eq!(doc(&s).depth, photocraft_color::SampleType::U16);
    // Damaged raw data is an error, not a crash.
    let mut bad = spec.build();
    bad.truncate(bad.len() / 2);
    assert!(open_bytes_as(&mut s, "bad.dng", &bad, None, None).is_err());
}

/// #5799: Batch with Format "Same" over a folder of camera raws saved nothing (every output was
/// a raw PhotoCraft can't write). Raws are saved as PSD; other inputs keep their format.
#[test]
fn batch_same_format_saves_camera_raws_as_psd() {
    use photocraft_raw::testgen::{DngSpec, mosaic, scene};
    let dir = tmp("batch-same-raw");
    let (w, h) = (24, 16);
    std::fs::write(join(&dir, "IMG_0001.dng"), DngSpec::cfa(w, h, mosaic(&scene(w, h), w, [0, 1, 1, 2], 0, 65535)).build()).unwrap();
    png(&dir, "b.png", 8, 8, "#ff0000");
    // Also with a file naming template, which picks the output extension separately.
    for (out, naming) in [("out", None), ("named", Some("{name}_{serial}"))] {
        let out = join(&dir, out);
        let mut p = json!({"steps": [["image.adjustments.invert", {}]], "input": dir, "output": out, "format": "same"});
        if let Some(naming) = naming {
            p["naming"] = json!(naming);
        }
        let r = Session::new().execute("file.automate.batch", p).unwrap();
        assert!(r["errors"].as_array().unwrap().is_empty(), "{naming:?}: {r}");
        let mut files: Vec<String> = r["files"].as_array().unwrap().iter().map(|f| f.as_str().unwrap().replace('\\', "/")).collect();
        files.sort();
        assert_eq!(files.len(), 2, "{r}");
        let suffixes = if naming.is_some() { ("_001", "_002") } else { (".psd", ".png") };
        assert!(files[0].contains("/IMG_0001") && files[0].ends_with(suffixes.0) && files[1].contains("/b") && files[1].ends_with(suffixes.1), "{files:?}");
        let psd = photocraft_io::import("IMG_0001.psd", &std::fs::read(&files[0]).unwrap()).unwrap().document;
        assert_eq!((psd.size.width, psd.size.height), (w as u32, h as u32));
    }
}

/// Edit › Preferences › Raw Defaults › Resolution sets the ppi a raw opens with, through
/// File › Open (the background open job) and through opening bytes; other files keep their own.
#[test]
fn a_developed_raw_opens_at_the_raw_defaults_resolution() {
    use photocraft_raw::testgen::{DngSpec, mosaic, scene};
    let (w, h) = (24, 16);
    let raw = DngSpec::cfa(w, h, mosaic(&scene(w, h), w, [0, 1, 1, 2], 0, 65535)).build();
    let dir = tmp("raw-defaults");
    let path = join(&dir, "IMG_0002.dng");
    std::fs::write(&path, &raw).unwrap();
    let open = |s: &mut Session, path: &str| match s.start_open(path, crate::jobs::OpenSource::Path(path.into())).unwrap() {
        crate::jobs::Started::Job(id) => {
            s.wait_job(id).unwrap();
        }
        crate::jobs::Started::Done(_) => {}
    };
    let mut s = Session::new();
    open_bytes_as(&mut s, "IMG_0001.dng", &raw, None, None).unwrap();
    assert_eq!(doc(&s).resolution_dpi, 300.0, "the default");
    s.execute("prefs.set", json!({"path": "rawDefaults.resolution", "value": 240})).unwrap();
    open_bytes_as(&mut s, "IMG_0001.dng", &raw, None, None).unwrap();
    assert_eq!(doc(&s).resolution_dpi, 240.0);
    open(&mut s, &path);
    assert_eq!(doc(&s).resolution_dpi, 240.0, "File › Open");
    let png = png(&dir, "a.png", 4, 4, "#ff0000");
    open(&mut s, &png);
    assert_eq!(doc(&s).resolution_dpi, 72.0, "not a raw");
}

/// The other ways a raw becomes a document of its own take Raw Defaults › Resolution too:
/// File › Revert, Batch and Load Files into Stack (all of them opened a raw at 72 ppi).
#[test]
fn revert_batch_and_stack_open_a_raw_at_the_raw_defaults_resolution() {
    use photocraft_raw::testgen::{DngSpec, mosaic, scene};
    let (w, h) = (24, 16);
    let dir = tmp("raw-defaults-siblings");
    let path = join(&dir, "IMG_0003.dng");
    std::fs::write(&path, DngSpec::cfa(w, h, mosaic(&scene(w, h), w, [0, 1, 1, 2], 0, 65535)).build()).unwrap();
    let mut s = Session::new();
    s.execute("prefs.set", json!({"path": "rawDefaults.resolution", "value": 240})).unwrap();
    open_bytes_as(&mut s, &path, &std::fs::read(&path).unwrap(), None, Some(path.clone())).unwrap();
    s.execute("image.imageSize", json!({"resolution": 150, "resample": "none"})).unwrap();
    assert_eq!(doc(&s).resolution_dpi, 150.0);
    s.execute("file.revert", json!({})).unwrap();
    assert_eq!(doc(&s).resolution_dpi, 240.0, "File › Revert");
    let out = join(&dir, "out");
    let r = s.execute("file.automate.batch", json!({"steps": [], "input": [path.clone()], "output": out, "format": "psd"})).unwrap();
    let files = r["files"].as_array().unwrap();
    assert_eq!(files.len(), 1, "{r}");
    let psd = photocraft_io::import("IMG_0003.psd", &std::fs::read(files[0].as_str().unwrap()).unwrap()).unwrap().document;
    assert_eq!(psd.resolution_dpi, 240.0, "Batch");
    s.execute("file.scripts.loadFilesIntoStack", json!({"paths": [path]})).unwrap();
    assert_eq!(doc(&s).resolution_dpi, 240.0, "Load Files into Stack");
}

#[test]
fn revert_reloads_as_one_undoable_step() {
    let dir = tmp("revert");
    let path = png(&dir, "a.png", 16, 12, "#ff0000");
    let mut s = Session::new();
    assert!(!s.is_enabled("file.revert"));
    let bytes = std::fs::read(&path).unwrap();
    open_bytes_as(&mut s, &path, &bytes, None, Some(path.clone())).unwrap();
    let id = doc(&s).id;
    assert!(s.is_enabled("file.revert"));
    s.execute("layer.new.layer", json!({})).unwrap();
    s.execute("edit.fill", json!({"color": "#0000ff"})).unwrap();
    assert!(s.active().unwrap().is_dirty());
    s.execute("file.revert", json!({})).unwrap();
    assert_eq!(doc(&s).layers.len(), 1);
    assert_eq!(doc(&s).id, id, "identity kept");
    assert!(!s.active().unwrap().is_dirty());
    assert!(composite(&s, 3, 3)[0] > 0.99);
    // Revert is a history state: undo brings the edits back.
    assert!(s.undo());
    assert_eq!(doc(&s).layers.len(), 2);
    // An unsaved document cannot revert.
    s.execute("file.new", json!({})).unwrap();
    assert!(s.execute("file.revert", json!({})).is_err());
}

#[test]
fn injected_import_failure_does_not_open_a_document() {
    let mut s = session(8, 8, 8);
    let before = s.documents().len();
    crate::allocation::fail_next_for_test();
    let result = open_bytes_as(&mut s, "image.png", b"not decoded", None, None);
    assert!(result.as_ref().is_err_and(|e| e.to_string().contains("not enough memory for importing document")));
    assert_eq!(s.documents().len(), before);
}

#[test]
fn injected_save_failure_does_not_modify_the_destination() {
    let dir = tmp("allocation-save");
    let path = join(&dir, "existing.psd");
    std::fs::write(&path, b"previous file").unwrap();
    let s = session(8, 8, 8);
    crate::allocation::fail_next_for_test();
    let result = save_doc(doc(&s), &path, None);
    assert!(result.as_ref().is_err_and(|e| e.to_string().contains("not enough memory for saving document")));
    assert_eq!(std::fs::read(path).unwrap(), b"previous file");
}

#[test]
fn save_a_copy_keeps_path_and_dirty_state() {
    let dir = tmp("copy");
    let mut s = session(20, 10, 16);
    s.execute("layer.new.layer", json!({})).unwrap();
    let rev = s.active().unwrap().revision;
    let out = join(&dir, "copy.psd");
    s.execute("file.saveACopy", json!({"path": out})).unwrap();
    assert!(std::fs::metadata(&out).unwrap().len() > 0);
    assert_eq!(s.active().unwrap().path, None);
    assert_eq!(s.active().unwrap().revision, rev);
    assert!(s.active().unwrap().is_dirty());
    let back = photocraft_io::import("copy.psd", &std::fs::read(&out).unwrap()).unwrap().document;
    assert_eq!(back.layers.len(), 2);
    assert_eq!(back.depth, photocraft_color::SampleType::U16);
    // Flattened copy.
    let flat = join(&dir, "flat.psd");
    s.execute("file.saveACopy", json!({"path": flat, "layers": false})).unwrap();
    assert_eq!(photocraft_io::import("flat.psd", &std::fs::read(&flat).unwrap()).unwrap().document.layers.len(), 1);
    assert!(s.execute("file.saveACopy", json!({})).is_err());
}

/// A copy over the document's own file, however the path is spelt, would replace it (a lossy
/// re-encode can't be undone), so it is refused unless `overwrite` is passed (#7437).
#[test]
fn save_a_copy_refuses_the_documents_own_file() {
    let dir = tmp("copy-own");
    let path = png(&dir, "own.png", 6, 4, "#ff0000");
    let bytes = std::fs::read(&path).unwrap();
    let mut s = Session::new();
    open_bytes_as(&mut s, &path, &bytes, None, Some(path.clone())).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.execute("edit.fill", json!({"color": "#00ff00"})).unwrap();
    let link = join(&dir, "link.png");
    std::fs::hard_link(&path, &link).unwrap();
    let rev = s.active().unwrap().revision;
    for p in [path.clone(), format!("{dir}/./own.png"), link] {
        let r = s.execute("file.saveACopy", json!({"path": p}));
        assert!(r.as_ref().is_err_and(|e| e.to_string().contains("overwrite")), "{p}: {r:?}");
        assert!(s.execute("file.saveACopy", json!({"path": p, "overwrite": false})).is_err(), "{p}");
        assert_eq!(std::fs::read(&path).unwrap(), bytes, "{p}");
        let st = s.active().unwrap();
        assert_eq!(st.path.as_deref(), Some(path.as_str()), "{p}");
        assert_eq!(st.revision, rev, "{p}");
        assert!(st.is_dirty(), "{p}");
    }
    let other = join(&dir, "other.png");
    s.execute("file.saveACopy", json!({"path": other})).unwrap();
    assert!(std::fs::metadata(&other).unwrap().len() > 0);
    s.execute("file.saveACopy", json!({"path": path, "overwrite": true})).unwrap();
    assert_ne!(std::fs::read(&path).unwrap(), bytes);
    assert_eq!(s.active().unwrap().path.as_deref(), Some(path.as_str()));
    assert!(s.active().unwrap().is_dirty());

    // A document that was never saved has no file of its own: it copies anywhere, as before.
    let mut fresh = session(4, 4, 8);
    fresh.execute("file.saveACopy", json!({"path": path})).unwrap();
    assert_eq!(fresh.active().unwrap().path, None);
}

/// `tgaBits` picks the Targa's bits per pixel (byte 16 of the header); unset, a document with an
/// alpha channel is written as 32-bit, with the channel as the alpha.
#[test]
fn save_a_copy_writes_the_targa_bits_asked_for() {
    let dir = tmp("tga-bits");
    let mut s = session(20, 10, 8);
    s.execute("select.all", json!({})).unwrap();
    s.execute("select.saveSelection", json!({})).unwrap();
    for (params, bpp) in [(json!({}), 32), (json!({"tgaBits": 24}), 24), (json!({"tgaBits": 32}), 32), (json!({"tgaBits": null}), 32)] {
        let out = join(&dir, "a.tga");
        let mut p = params.clone();
        p["path"] = json!(out);
        s.execute("file.saveACopy", p).unwrap();
        let bytes = std::fs::read(&out).unwrap();
        assert_eq!(bytes[16], bpp, "{params}");
        let back = photocraft_io::import("a.tga", &bytes).unwrap().document;
        assert_eq!(back.channels.len(), usize::from(bpp == 32), "{params}: the selection reopens as Alpha 1");
    }
}

/// `dpxBits` picks a DPX's bits per sample (byte 803 of the header); unset, it follows the
/// document's depth. Anything but 8, 10, 12 or 16 is refused before anything is written.
#[test]
fn save_a_copy_writes_the_dpx_bits_asked_for() {
    let dir = tmp("dpx-bits");
    for (depth, params, bits) in
        [(8, json!({}), 8), (16, json!({}), 16), (16, json!({"dpxBits": 10}), 10), (8, json!({"dpxBits": 12}), 12), (16, json!({"dpxBits": null}), 16)]
    {
        let mut s = session(12, 6, depth);
        let out = join(&dir, "a.dpx");
        let mut p = params.clone();
        p["path"] = json!(out);
        s.execute("file.saveACopy", p).unwrap();
        let bytes = std::fs::read(&out).unwrap();
        assert_eq!((&bytes[..4], bytes[803]), (&b"SDPX"[..], bits), "{depth}-bit {params}");
        assert_eq!(photocraft_io::import("a.dpx", &bytes).unwrap().document.size, photocraft_geom::Size::new(12, 6));
    }
    let mut s = session(8, 8, 8);
    for bad in [json!(24), json!(0), json!("10"), json!(10.5)] {
        let out = join(&dir, "bad.dpx");
        let r = s.execute("file.saveACopy", json!({"path": out, "dpxBits": bad}));
        assert!(matches!(r, Err(EngineError::BadParams { .. })), "{bad}: {r:?}");
        assert!(!std::path::Path::new(&out).exists(), "{bad}: nothing written");
    }
}

/// `ddsCompression` picks the DDS FourCC (or none, uncompressed) and `ddsMipmaps` the mip count
/// in the header; unset, an uncompressed texture without mipmaps. Bad values are refused before
/// anything is written.
#[test]
fn save_a_copy_writes_the_dds_options_asked_for() {
    let dir = tmp("dds-options");
    let le32 = |b: &[u8], at: usize| u32::from_le_bytes(b[at..at + 4].try_into().unwrap());
    for (params, fourcc, mips) in [
        (json!({}), [0; 4], 1),
        (json!({"ddsCompression": "none", "ddsMipmaps": true}), [0; 4], 5),
        (json!({"ddsCompression": "bc1"}), *b"DXT1", 1),
        (json!({"ddsCompression": "DXT3", "ddsMipmaps": false}), *b"DXT3", 1),
        (json!({"ddsCompression": "bc3", "ddsMipmaps": true}), *b"DXT5", 5),
        (json!({"ddsCompression": null, "ddsMipmaps": null}), [0; 4], 1),
    ] {
        let mut s = session(16, 12, 8);
        let out = join(&dir, "a.dds");
        let mut p = params.clone();
        p["path"] = json!(out);
        s.execute("file.saveACopy", p).unwrap();
        let bytes = std::fs::read(&out).unwrap();
        assert_eq!((&bytes[..4], &bytes[84..88], le32(&bytes, 28)), (&b"DDS "[..], &fourcc[..], mips), "{params}");
        assert_eq!(photocraft_io::import("a.dds", &bytes).unwrap().document.size, photocraft_geom::Size::new(16, 12), "{params}");
    }
    let mut s = session(8, 8, 8);
    let state = |s: &Session| s.active().map(|st| (st.revision, st.path.clone(), st.is_dirty()));
    let before = state(&s);
    for bad in [json!({"ddsCompression": "bc7"}), json!({"ddsCompression": 1}), json!({"ddsMipmaps": "yes"}), json!({"ddsMipmaps": 1})] {
        let out = join(&dir, "bad.dds");
        let mut p = bad.clone();
        p["path"] = json!(out);
        let r = s.execute("file.saveACopy", p);
        assert!(matches!(r, Err(EngineError::BadParams { .. })), "{bad}: {r:?}");
        assert!(!std::path::Path::new(&out).exists(), "{bad}: nothing written");
        assert_eq!(state(&s), before, "{bad}: the document is untouched");
    }
}

#[test]
fn dds_params_are_none_until_one_is_given() {
    use photocraft_io::{DdsCompression, DdsOptions};
    assert_eq!(dds_param(&json!({}), "file.saveAs").unwrap(), None);
    assert_eq!(dds_param(&json!({"ddsCompression": null}), "file.saveAs").unwrap(), None);
    assert_eq!(dds_param(&json!({"ddsMipmaps": true}), "file.saveAs").unwrap(), Some(DdsOptions { compression: DdsCompression::None, mipmaps: true }));
    assert_eq!(dds_param(&json!({"ddsCompression": "bc2"}), "file.saveAs").unwrap(), Some(DdsOptions { compression: DdsCompression::Bc2, mipmaps: false }));
}

/// A `tgaBits` other than 24 or 32 is a bad parameter, reported before anything is written.
#[test]
fn bad_targa_bits_are_refused_before_writing() {
    let dir = tmp("tga-bad");
    let mut s = session(8, 8, 8);
    for bad in [json!(16), json!(0), json!(-32), json!(32.5), json!("32"), json!(true), json!([32])] {
        let out = join(&dir, "a.tga");
        let r = s.execute("file.saveACopy", json!({"path": out, "tgaBits": bad}));
        assert!(matches!(r, Err(EngineError::BadParams { .. })), "{bad}: {r:?}");
        assert!(!std::path::Path::new(&out).exists(), "{bad}: nothing written");
        let r = s.execute("file.export.layersToFiles", json!({"dir": dir, "format": "tga", "tgaBits": bad}));
        assert!(matches!(r, Err(EngineError::BadParams { .. })), "{bad}: {r:?}");
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0, "{bad}: no layer written");
    }
}

/// `jpegFormat` picks JPEG Options' format: Progressive writes a progressive frame (SOF2), the
/// others a baseline one (SOF0); `quality` keeps Photoshop's 0–12 scale.
#[test]
fn save_a_copy_writes_the_jpeg_format_asked_for() {
    let dir = tmp("jpeg-format");
    let mut s = session(40, 30, 8);
    let marker = |b: &[u8], m: u8| b.windows(2).any(|w| w == [0xFF, m]);
    for (params, progressive) in [
        (json!({}), false),
        (json!({"jpegFormat": null}), false),
        (json!({"jpegFormat": "baseline", "quality": 12}), false),
        (json!({"jpegFormat": "optimized", "quality": 10}), false),
        (json!({"jpegFormat": "progressive", "quality": 3}), true),
    ] {
        let out = join(&dir, "a.jpg");
        let mut p = params.clone();
        p["path"] = json!(out);
        s.execute("file.saveACopy", p).unwrap();
        let bytes = std::fs::read(&out).unwrap();
        assert_eq!(marker(&bytes, 0xC2), progressive, "{params}");
        assert_eq!(marker(&bytes, 0xC0), !progressive, "{params}");
        assert_eq!(photocraft_io::import("a.jpg", &bytes).unwrap().document.size.width, 40, "{params}");
    }
    // Layers to Files takes it too.
    s.execute("file.export.layersToFiles", json!({"dir": dir, "format": "jpg", "jpegFormat": "progressive"})).unwrap();
    let layers: Vec<_> = std::fs::read_dir(&dir).unwrap().filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| !p.ends_with("a.jpg")).collect();
    assert!(!layers.is_empty());
    for p in layers {
        assert!(marker(&std::fs::read(&p).unwrap(), 0xC2), "{p:?} is progressive");
    }
    assert_eq!((jpeg_quality(0.0), jpeg_quality(12.0), jpeg_quality(6.0)), (1, 100, 51));
    assert_eq!((jpeg_quality(-3.0), jpeg_quality(40.0), jpeg_quality(f64::NAN), jpeg_quality(f64::NEG_INFINITY)), (1, 100, 100, 100));
}

/// A `jpegFormat` other than the three names is a bad parameter, reported before anything is
/// written.
#[test]
fn bad_jpeg_formats_are_refused_before_writing() {
    let dir = tmp("jpeg-bad");
    let mut s = session(8, 8, 8);
    for bad in [json!("Progressive"), json!(""), json!(2), json!(true), json!(["baseline"]), json!({"format": "baseline"})] {
        let out = join(&dir, "a.jpg");
        let r = s.execute("file.saveACopy", json!({"path": out, "jpegFormat": bad}));
        assert!(matches!(r, Err(EngineError::BadParams { .. })), "{bad}: {r:?}");
        assert!(!std::path::Path::new(&out).exists(), "{bad}: nothing written");
        let r = s.execute("file.export.layersToFiles", json!({"dir": dir, "format": "jpg", "jpegFormat": bad}));
        assert!(matches!(r, Err(EngineError::BadParams { .. })), "{bad}: {r:?}");
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0, "{bad}: no layer written");
    }
}

/// Preferences › File Handling › Image Previews decides whether a PSD gets Photoshop's
/// thumbnail resource (1036): Always and Ask When Saving write it, Never doesn't.
#[test]
fn image_previews_preference_controls_the_psd_thumbnail() {
    use crate::prefs::Ask;
    let dir = tmp("previews");
    let mut s = session(300, 200, 16);
    let has_thumbnail = |path: &str| {
        let file = photocraft_psd::PsdFile::from_bytes(&std::fs::read(path).unwrap()).unwrap();
        file.resources.iter().any(|r| r.id == photocraft_psd::resources::ids::THUMBNAIL)
    };
    for (ask, expect) in [(Ask::Always, true), (Ask::Never, false), (Ask::Ask, true)] {
        s.edit_prefs(|p| p.file_handling.image_previews = ask);
        let out = join(&dir, &format!("{ask:?}.psd"));
        s.execute("file.saveACopy", json!({"path": out})).unwrap();
        assert_eq!(has_thumbnail(&out), expect, "{ask:?}");
        let png = join(&dir, &format!("{ask:?}.png"));
        s.execute("file.saveACopy", json!({"path": png})).unwrap();
    }
}

/// Preferences › File Handling › Maximize PSD and PSB File Compatibility decides whether a
/// layered PSD keeps its real merged image: Always and Ask When Saving write it, Never writes
/// Photoshop's placeholder and says so in resource 1057.
#[test]
fn maximize_compatibility_preference_controls_the_psd_merged_image() {
    use crate::prefs::Ask;
    let dir = tmp("maximize");
    let mut s = session(300, 200, 16);
    s.execute("layer.new.layer", json!({})).unwrap();
    let real_merged = |path: &str| photocraft_psd::PsdFile::from_bytes(&std::fs::read(path).unwrap()).unwrap().has_real_merged_data();
    for (ask, expect) in [(Ask::Always, true), (Ask::Never, false), (Ask::Ask, true)] {
        s.edit_prefs(|p| p.file_handling.maximize_psd_compatibility = ask);
        for ext in ["psd", "psb"] {
            let out = join(&dir, &format!("{ask:?}.{ext}"));
            s.execute("file.saveACopy", json!({"path": out})).unwrap();
            assert_eq!(real_merged(&out), Some(expect), "{ask:?} {ext}");
        }
    }
}

/// Preferences › File Handling › Save Full-Size Composite in .pcraft Files decides whether a
/// `.pcraft` save writes `composite/full.png` (off by default); a command's
/// `"pcraftFullComposite"` overrides it.
#[test]
fn full_composite_preference_controls_the_pcraft_full_size_composite() {
    let dir = tmp("full-composite");
    let mut s = session(300, 200, 16);
    let full = |path: &str| photocraft_format::read_full_composite(&std::fs::read(path).unwrap()).unwrap().is_some();
    let out = join(&dir, "default.pcraft");
    s.execute("file.saveACopy", json!({"path": out})).unwrap();
    assert!(!full(&out), "off by default");
    for on in [true, false] {
        s.edit_prefs(|p| p.file_handling.pcraft_full_composite = on);
        let out = join(&dir, &format!("{on}.pcraft"));
        s.execute("file.saveACopy", json!({"path": out})).unwrap();
        assert_eq!(full(&out), on, "preference {on}");
        let out = join(&dir, &format!("{on} param.pcraft"));
        s.execute("file.saveACopy", json!({"path": out, "pcraftFullComposite": !on})).unwrap();
        assert_eq!(full(&out), !on, "preference {on}, param {}", !on);
    }
    let out = join(&dir, "null param.pcraft");
    s.execute("file.saveACopy", json!({"path": out, "pcraftFullComposite": null})).unwrap();
    assert!(!full(&out), "null takes the preference");
    for bad in [json!("true"), json!(1)] {
        let out = join(&dir, "bad param.pcraft");
        let err = s.execute("file.saveACopy", json!({"path": out, "pcraftFullComposite": bad})).unwrap_err().to_string();
        assert!(err.contains("`pcraftFullComposite` must be true or false"), "{bad}: {err}");
        assert!(!std::path::Path::new(&out).exists(), "{bad}: nothing is written");
    }
}

#[test]
fn open_as_forces_the_decoder() {
    let dir = tmp("openas");
    let path = png(&dir, "a.png", 6, 5, "#00ff00");
    let renamed = join(&dir, "mystery.bin");
    std::fs::copy(&path, &renamed).unwrap();
    let mut s = Session::new();
    s.execute("file.openAs", json!({"path": renamed, "as": "png"})).unwrap();
    assert_eq!(doc(&s).size.width, 6);
    assert_eq!(doc(&s).name, "mystery.bin");
    assert_eq!(s.active().unwrap().path.as_deref(), Some(renamed.as_str()));
    assert!(s.execute("file.openAs", json!({"path": join(&dir, "missing.png")})).is_err());
}

#[test]
fn place_embedded_centres_fits_and_embeds() {
    let dir = tmp("place");
    let big = png(&dir, "big.png", 200, 100, "#ff0000");
    let small = png(&dir, "small.png", 10, 10, "#0000ff");
    for depth in [8, 16, 32] {
        let mut s = session(100, 100, depth);
        let r = s.execute("file.placeEmbedded", json!({"path": big})).unwrap();
        assert!((r["scale"].as_f64().unwrap() - 50.0).abs() < 1e-9);
        let d = s.active().unwrap();
        let l = d.doc.layer(d.active_layer.unwrap()).unwrap();
        assert_eq!(l.name, "big");
        let LayerContent::Smart(so) = &l.content else { panic!("smart object expected") };
        assert!(matches!(&so.source, SmartSource::Embedded { file_name, bytes } if file_name == "big.png" && !bytes.is_empty()));
        assert_eq!(so.transform.m[0], 0.5);
        // Fitted to the canvas width, centred vertically: rows 25..75 red.
        assert!(composite(&s, 50, 50)[0] > 0.99 && composite(&s, 50, 50)[2] < 0.01);
        assert!(composite(&s, 50, 10)[1] > 0.99, "white background above");
        // Small images keep their size, centred.
        s.execute("file.placeLinked", json!({"path": small})).unwrap();
        assert!(composite(&s, 50, 50)[2] > 0.99);
        assert!(composite(&s, 40, 40)[2] < 0.01);
        let d = s.active().unwrap();
        let l = d.doc.layer(d.active_layer.unwrap()).unwrap();
        assert!(matches!(&l.content, LayerContent::Smart(so) if matches!(&so.source, SmartSource::Linked { path } if path == &small)));
        // One step each.
        assert!(s.undo() && s.undo());
        assert_eq!(doc(&s).layers.len(), 1);
    }
}

/// Preferences ▸ General ▸ Always Create Smart Objects When Placing: unchecked, an embedded place
/// lands as a plain pixel layer; a linked place is always a smart object and `"smartObject"` wins.
#[test]
fn always_create_smart_objects_preference_places_a_pixel_layer() {
    let dir = tmp("place-smart-pref");
    let big = png(&dir, "big.png", 200, 100, "#ff0000");
    let mut s = session(100, 100, 8);
    s.execute("prefs.set", json!({"path": "general.alwaysCreateSmartObjectsWhenPlacing", "value": false})).unwrap();
    let r = s.execute("file.placeEmbedded", json!({"path": big})).unwrap();
    assert_eq!(r["smartObject"], json!(false));
    let d = s.active().unwrap();
    let l = d.doc.layer(d.active_layer.unwrap()).unwrap();
    assert!(matches!(l.content, LayerContent::Raster(_)), "a pixel layer");
    // Fitted to the canvas width, centred vertically, exactly like the smart-object place.
    assert!(composite(&s, 50, 50)[0] > 0.99 && composite(&s, 50, 50)[2] < 0.01);
    assert!(composite(&s, 50, 10)[1] > 0.99, "white background above");
    // An explicit "smartObject": true overrides, and Place Linked with the preference off still
    // creates a linked smart object.
    assert_eq!(s.execute("file.placeEmbedded", json!({"path": big, "smartObject": true})).unwrap()["smartObject"], json!(true));
    assert_eq!(s.execute("file.placeLinked", json!({"path": big})).unwrap()["smartObject"], json!(true));
}

/// Preferences ▸ General ▸ Resize Image During Place: off, a larger image keeps its natural size
/// instead of being fitted to the canvas; an explicit `"fit"` still wins.
#[test]
fn resize_image_during_place_preference_controls_the_fit() {
    let dir = tmp("place-fit-pref");
    let big = png(&dir, "big.png", 200, 100, "#ff0000");
    let mut s = session(100, 100, 8);
    s.execute("prefs.set", json!({"path": "general.resizeImageDuringPlace", "value": false})).unwrap();
    let r = s.execute("file.placeEmbedded", json!({"path": big})).unwrap();
    assert!((r["scale"].as_f64().unwrap() - 100.0).abs() < 1e-9, "natural size: {r}");
    let b = r["bounds"].as_array().unwrap();
    assert!(b[2].as_f64().unwrap() > 100.0, "wider than the canvas: {b:?}");
    // An explicit "fit": true overrides the preference.
    let r = s.execute("file.placeEmbedded", json!({"path": big, "fit": true})).unwrap();
    assert!((r["scale"].as_f64().unwrap() - 50.0).abs() < 1e-9, "fitted on request: {r}");
}

/// A 40×20 JPEG (left half red, right half blue) tagged EXIF Orientation = 6: shown upright it
/// is 20×40, red on top.
fn rotated_jpeg(dir: &str) -> String {
    let px: Vec<u8> = (0..20).flat_map(|_| (0..40).flat_map(|x| if x < 20 { [230, 20, 20] } else { [20, 20, 230] })).collect();
    let img = photocraft_codecs::Image::from_u8(40, 20, photocraft_codecs::ChannelLayout::Rgb, px).unwrap();
    let jpeg = photocraft_codecs::encode(&img, photocraft_codecs::Format::Jpeg, &Default::default()).unwrap();
    let mut seg = b"Exif\0\0II*\0\x08\0\0\0\x01\0\x12\x01\x03\0\x01\0\0\0\x06\0\0\0\0\0\0\0".to_vec();
    let mut file = vec![0xFF, 0xD8, 0xFF, 0xE1];
    file.extend_from_slice(&((seg.len() + 2) as u16).to_be_bytes());
    file.append(&mut seg);
    file.extend_from_slice(&jpeg[2..]);
    let path = join(dir, "IMG_0001.jpg");
    std::fs::write(&path, file).unwrap();
    path
}

#[test]
fn rotated_jpegs_open_and_place_upright() {
    let dir = tmp("orientation");
    let path = rotated_jpeg(&dir);
    let mut s = Session::new();
    s.execute("file.openAs", json!({"path": path, "as": "jpg"})).unwrap();
    assert_eq!((doc(&s).size.width, doc(&s).size.height), (20, 40));
    assert!(composite(&s, 10, 5)[0] > 0.8 && composite(&s, 10, 35)[2] > 0.8, "red on top, blue below");
    // Placed as a smart object: upright too, and it renders upright from its embedded bytes.
    let mut s = session(100, 100, 8);
    let r = s.execute("file.placeEmbedded", json!({"path": path})).unwrap();
    assert_eq!(r["bounds"], json!([40.0, 30.0, 60.0, 70.0]));
    assert!(composite(&s, 50, 35)[0] > 0.8 && composite(&s, 50, 65)[2] > 0.8);
}

#[test]
fn file_info_round_trips_through_xmp_and_psd() {
    let mut s = session(8, 8, 8);
    let empty = s.execute("file.fileInfo", json!({})).unwrap();
    assert_eq!(empty["title"], "");
    assert!(!s.undo(), "reading is not a history step");
    s.execute("file.fileInfo", json!({"title": "Starry <Night>", "author": "V. van Gogh", "keywords": ["stars", "night"], "copyright": "Public domain", "copyrightStatus": "publicDomain", "description": "Oil & canvas"})).unwrap();
    let info = s.execute("file.fileInfo", json!({})).unwrap();
    assert_eq!(info["title"], "Starry <Night>");
    assert_eq!(info["author"], "V. van Gogh");
    assert_eq!(info["keywords"], json!(["stars", "night"]));
    assert_eq!(info["description"], "Oil & canvas");
    assert_eq!(info["copyrightStatus"], "publicDomain");
    // Editing one field keeps the others and replaces, not duplicates.
    s.execute("file.fileInfo", json!({"title": "The Starry Night"})).unwrap();
    let xmp = doc(&s).metadata.xmp.clone().unwrap();
    assert_eq!(xmp.matches("<dc:title>").count(), 1);
    assert_eq!(read_file_info(Some(&xmp))["author"], "V. van Gogh");
    // PSD round trip.
    let (bytes, _) = encode(doc(&s), "x.psd", None).unwrap();
    let back = photocraft_io::import("x.psd", &bytes).unwrap().document;
    let info = read_file_info(back.metadata.xmp.as_deref());
    assert_eq!(info["title"], "The Starry Night");
    assert_eq!(info["keywords"], json!(["stars", "night"]));
    // Foreign XMP (attribute form, other namespaces) keeps its other properties.
    let foreign = r#"<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"><rdf:Description rdf:about="" xmlns:xmp="http://ns.adobe.com/xap/1.0/" xmp:CreatorTool="Other App" photoshop:AuthorsPosition="Painter"/></rdf:RDF></x:xmpmeta>"#;
    assert_eq!(read_file_info(Some(foreign))["authorTitle"], "Painter");
    let out = write_file_info(Some(foreign), &json!({"authorTitle": "Artist"}));
    assert!(out.contains("xmp:CreatorTool=\"Other App\""));
    assert_eq!(read_file_info(Some(&out))["authorTitle"], "Artist");
}

fn camera_exif() -> Vec<u8> {
    photocraft_algo::exif::build(&photocraft_algo::exif::CameraInfo {
        make: Some("SONY".into()),
        model: Some("ILCE-7RM5".into()),
        lens: Some("24-70mm F2.8".into()),
        exposure_time: Some(1.0 / 250.0),
        f_number: Some(8.0),
        iso: Some(400.0),
        focal_length: Some(33.1),
        focal_length_35mm: Some(50.0),
        date_taken: Some("2024:02:17 12:40:19".into()),
    })
}

#[test]
fn file_info_reads_camera_data_from_jpeg_and_psd_exif() {
    let mut s = session(8, 8, 8);
    assert_eq!(s.execute("file.fileInfo", json!({})).unwrap()["camera"], json!({}));
    s.edit("exif", |doc, _| {
        doc.metadata.exif = Some(Arc::new(camera_exif()));
        Ok(())
    })
    .unwrap();
    let want = json!({
        "make": "SONY", "model": "ILCE-7RM5", "dateTaken": "2024-02-17 12:40:19", "exposure": "1/250 s", "fNumber": "f/8",
        "iso": "ISO 400", "focalLength": "33.1 mm", "focalLength35mm": "50 mm", "lens": "24-70mm F2.8"
    });
    for name in ["x.jpg", "x.psd"] {
        let (bytes, _) = encode(doc(&s), name, None).unwrap();
        let back = photocraft_io::import(name, &bytes).unwrap().document;
        let mut t = Session::new();
        t.add_document(back, None);
        assert_eq!(t.execute("file.fileInfo", json!({})).unwrap()["camera"], want, "{name}");
        // Editing the description fields leaves the read-only camera data alone.
        let edited = t.execute("file.fileInfo", json!({"title": "Pier", "camera": {"make": "ignored"}})).unwrap();
        assert_eq!(edited["camera"], want, "{name}");
    }
}

#[test]
fn camera_data_formats_like_photoshop_and_falls_back_to_xmp() {
    let info = |t: Option<f64>, n: Option<f64>, f: Option<f64>| {
        let exif = photocraft_algo::exif::build(&photocraft_algo::exif::CameraInfo { exposure_time: t, f_number: n, focal_length: f, ..Default::default() });
        camera_data(Some(&exif), None)
    };
    assert_eq!(info(Some(2.0), Some(2.8), Some(35.0)), json!({"exposure": "2 s", "fNumber": "f/2.8", "focalLength": "35 mm"}));
    assert_eq!(info(Some(0.5), Some(5.6), None), json!({"exposure": "1/2 s", "fNumber": "f/5.6"}));
    assert_eq!(info(Some(0.4), None, None), json!({"exposure": "0.4 s"}));
    assert_eq!(info(Some(1.0 / 2000.0), None, None), json!({"exposure": "1/2000 s"}));
    // Non-standard fractions round to the nearest 1/n instead of losing digits (#6958).
    assert_eq!(info(Some(0.031714), None, None), json!({"exposure": "1/32 s"}));
    assert_eq!(info(Some(0.0215), None, None), json!({"exposure": "1/47 s"}));
    assert_eq!(info(Some(1.0 / 3.0), None, None), json!({"exposure": "1/3 s"}));
    // Too far from any whole denominator: three significant digits, not two decimals.
    assert_eq!(info(Some(0.2731), None, None), json!({"exposure": "0.273 s"}));
    assert_eq!(info(Some(0.0), Some(0.0), Some(0.0)), json!({}));
    // No EXIF block (a raw import, XMP-only files): Photoshop's exif:/tiff:/aux: properties.
    let xmp = r#"<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
<rdf:Description rdf:about="" xmlns:tiff="http://ns.adobe.com/tiff/1.0/" xmlns:exif="http://ns.adobe.com/exif/1.0/" xmlns:aux="http://ns.adobe.com/exif/1.0/aux/"
 tiff:Make="SONY" tiff:Model="ILCE-7RM5" exif:ExposureTime="1/250" exif:FNumber="8/1" exif:FocalLength="331/10"
 exif:FocalLengthIn35mmFilm="50" exif:DateTimeOriginal="2024-02-17T12:40:19.00+01:00" aux:Lens="Lens &amp; Co">
<exif:ISOSpeedRatings><rdf:Seq><rdf:li>400</rdf:li></rdf:Seq></exif:ISOSpeedRatings>
</rdf:Description></rdf:RDF></x:xmpmeta>"#;
    let want = json!({
        "make": "SONY", "model": "ILCE-7RM5", "dateTaken": "2024-02-17 12:40:19", "exposure": "1/250 s", "fNumber": "f/8",
        "iso": "ISO 400", "focalLength": "33.1 mm", "focalLength35mm": "50 mm", "lens": "Lens & Co"
    });
    assert_eq!(camera_data(None, Some(xmp)), want);
    // EXIF wins; XMP only fills what EXIF lacks.
    let exif = photocraft_algo::exif::build(&photocraft_algo::exif::CameraInfo { make: Some("Other".into()), ..Default::default() });
    let both = camera_data(Some(&exif), Some(xmp));
    assert_eq!(both["make"], "Other");
    assert_eq!(both["exposure"], "1/250 s");
    // Garbage in either block reads as no data, never a panic.
    assert_eq!(camera_data(Some(b"II*\0\xff\xff\xff\xff"), Some("<exif:FNumber>0/0</exif:FNumber> exif:ExposureTime=\"-1/0\"")), json!({}));
}

/// A slow shutter speed whose reciprocal is not whole prints as the nearest fraction a camera
/// would write, not as two decimals. The old 1 %-of-the-reciprocal test shrank as the exposure
/// slowed, so below 1/50 s it fell below half a rounding step and nearly every such fraction fell
/// through to the decimal form (#6958).
#[test]
fn camera_data_prints_a_slow_exposure_as_the_fraction_a_camera_writes() {
    let text = |t: f64| exposure_text(t).expect("an exposure");
    // The two values from the report: 31714/1000000 s and 215/10000 s.
    assert_eq!(text(0.031_714), "1/32 s", "a stored 0.031714 s is about 1/32 s");
    assert_eq!(text(0.0215), "1/47 s", "a stored 0.0215 s is about 1/47 s");
    // The rest of a slow shutter scale, and the exact reciprocals that already worked.
    assert_eq!(text(0.125), "1/8 s");
    assert_eq!(text(1.0 / 60.0), "1/60 s");
    assert_eq!(text(1.0 / 4000.0), "1/4000 s");
    assert_eq!(text(0.04), "1/25 s");
    assert_eq!(text(0.09), "1/11 s");
    // Every shown time stays within 5 % of the stored one — the point of the report.
    for t in [0.031_714, 0.0215, 0.04, 0.09, 0.125, 1.0 / 60.0, 1.0 / 4000.0] {
        let n: f64 = text(t).split('/').nth(1).and_then(|d| d.split(' ').next()).expect("1/n s").parse().expect("a number");
        assert!((1.0 / n / t - 1.0).abs() <= 0.05, "{t} shown as {} is more than 5 % off", text(t));
    }
    // A tenth of a second and longer keeps the Photoshop forms: a near-whole reciprocal is a
    // fraction, anything else is the decimal at three significant digits.
    assert_eq!(text(0.5), "1/2 s");
    assert_eq!(text(0.25), "1/4 s");
    assert_eq!(text(0.4), "0.4 s");
    assert_eq!(text(0.105), "0.105 s", "two decimals would read 0.11 s");
    assert_eq!(text(1.25), "1.25 s", "one decimal would read 1.3 s");
    assert_eq!(text(2.0), "2 s");
    // A hostile value cannot panic, and a positive exposure never reads as "0 s" (never-crash, rule 9).
    assert_ne!(exposure_text(f64::MIN_POSITIVE).as_deref(), Some("0 s"));
    assert_ne!(exposure_text(f64::MIN_POSITIVE), None);
    assert_eq!(exposure_text(0.0), None);
    assert_eq!(exposure_text(-1.0), None);
    assert_eq!(exposure_text(f64::NAN), None);
    assert_eq!(exposure_text(f64::INFINITY), None);
    assert_ne!(exposure_text(f64::MAX), None);
    // The same through a real EXIF block, so the round trip to File Info is covered too.
    let camera = |t: f64| {
        let exif = photocraft_algo::exif::build(&photocraft_algo::exif::CameraInfo { exposure_time: Some(t), ..Default::default() });
        camera_data(Some(&exif), None)["exposure"].as_str().map(str::to_owned)
    };
    assert_eq!(camera(0.031_714), Some("1/32 s".to_owned()));
    assert_eq!(camera(0.0215), Some("1/47 s".to_owned()));
    // An absurd exposure is capped to one dialog line, as every other camera field is.
    assert!(camera(f64::MAX).expect("an exposure").len() <= 200, "a huge exposure must stay on one dialog line");
}

const MULTILINGUAL_XMP: &str = r#"<x:xmpmeta xmlns:x="adobe:ns:meta/">
<rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
<rdf:Description rdf:about="" xmlns:dc="http://purl.org/dc/elements/1.1/"
 xmlns:xmp="http://ns.adobe.com/xap/1.0/" xmlns:xmpRights="http://ns.adobe.com/xap/1.0/rights/"
 xmp:CreatorTool="Other App" xmpRights:Marked="True" xmpRights:WebStatement="https://example.com/license">
<dc:title><rdf:Alt><rdf:li xml:lang="x-default">Sun</rdf:li><rdf:li xml:lang="fr">Soleil</rdf:li><rdf:li xml:lang="ar">الشمس</rdf:li></rdf:Alt></dc:title>
<dc:description><rdf:Alt><rdf:li xml:lang="x-default">Sunrise</rdf:li><rdf:li xml:lang="fr">Lever du soleil</rdf:li></rdf:Alt></dc:description>
<dc:rights><rdf:Alt><rdf:li xml:lang="x-default">All rights reserved</rdf:li><rdf:li xml:lang="fr">Tous droits réservés</rdf:li></rdf:Alt></dc:rights>
<dc:subject><rdf:Bag><rdf:li>sky</rdf:li></rdf:Bag></dc:subject>
</rdf:Description></rdf:RDF></x:xmpmeta>"#;

fn assert_translations_kept(xmp: &str) {
    for prop in ["dc:title", "dc:description", "dc:rights"] {
        assert_eq!(find_element(xmp, prop).unwrap().2, find_element(MULTILINGUAL_XMP, prop).unwrap().2, "{prop}");
        assert_eq!(xmp.matches(&format!("<{prop}>")).count(), 1);
    }
    assert!(xmp.contains("xmp:CreatorTool=\"Other App\""));
    assert!(xmp.contains("xmpRights:Marked=\"True\""));
    assert!(xmp.contains("xmpRights:WebStatement=\"https://example.com/license\""));
}

#[test]
fn file_info_keeps_translations_when_editing_keywords() {
    let partial = json!({"keywords": ["sky", "sunrise"]});
    let mut dialog = read_file_info(Some(MULTILINGUAL_XMP));
    dialog["keywords"] = json!("sky; sunrise");
    for params in [partial, dialog] {
        let out = write_file_info(Some(MULTILINGUAL_XMP), &params);
        assert_translations_kept(&out);
        assert_eq!(read_file_info(Some(&out))["keywords"], json!(["sky", "sunrise"]));
    }
    let unchanged = read_file_info(Some(MULTILINGUAL_XMP));
    assert_eq!(write_file_info(Some(MULTILINGUAL_XMP), &unchanged), MULTILINGUAL_XMP);
    assert_eq!(write_file_info(Some(MULTILINGUAL_XMP), &json!({})), MULTILINGUAL_XMP);
}

#[test]
fn file_info_clears_only_the_requested_field() {
    let out = write_file_info(Some(MULTILINGUAL_XMP), &json!({"keywords": []}));
    assert_translations_kept(&out);
    assert!(find_element(&out, "dc:subject").is_none());
    assert_eq!(read_file_info(Some(&out))["keywords"], json!([]));
}

#[test]
fn file_info_translations_survive_undo_and_psd_round_trip() {
    for depth in [8, 16, 32] {
        let mut s = session(8, 8, depth);
        s.edit("Set metadata", |doc, _| {
            doc.metadata.xmp = Some(MULTILINGUAL_XMP.into());
            Ok(())
        })
        .unwrap();
        s.execute("file.fileInfo", json!({"keywords": ["sunrise"]})).unwrap();
        let edited = doc(&s).metadata.xmp.clone().unwrap();
        assert_translations_kept(&edited);
        assert!(s.undo());
        assert_eq!(doc(&s).metadata.xmp.as_deref(), Some(MULTILINGUAL_XMP));
        assert!(s.redo());
        assert_eq!(doc(&s).metadata.xmp.as_deref(), Some(&*edited));
        let (bytes, _) = encode(doc(&s), "x.psd", None).unwrap();
        let back = photocraft_io::import("x.psd", &bytes).unwrap().document;
        assert_translations_kept(back.metadata.xmp.as_deref().unwrap());
        assert_eq!(read_file_info(back.metadata.xmp.as_deref())["keywords"], json!(["sunrise"]));
    }
}

#[test]
fn fit_image_and_conditional_mode_change() {
    let mut s = session(400, 200, 8);
    s.execute("file.automate.fitImage", json!({"width": 100, "height": 100})).unwrap();
    assert_eq!((doc(&s).size.width, doc(&s).size.height), (100, 50));
    let r = s.execute("file.automate.fitImage", json!({"width": 1000, "height": 1000, "dontEnlarge": true})).unwrap();
    assert_eq!(r["changed"], false);
    s.execute("file.automate.fitImage", json!({"width": 1000, "height": 1000})).unwrap();
    assert_eq!(doc(&s).size.width, 1000);

    // Zero or negative width or height leaves that side unconstrained.
    let mut s2 = session(1000, 800, 8);
    s2.execute("file.automate.fitImage", json!({"width": 0, "height": 400})).unwrap();
    assert_eq!((doc(&s2).size.width, doc(&s2).size.height), (500, 400));
    s2.execute("file.automate.fitImage", json!({"width": 250, "height": -10})).unwrap();
    assert_eq!((doc(&s2).size.width, doc(&s2).size.height), (250, 200));
    let r = s.execute("file.automate.conditionalModeChange", json!({"from": ["cmyk"], "to": "grayscale"})).unwrap();
    assert_eq!(r["changed"], false);
    s.execute("file.automate.conditionalModeChange", json!({"from": ["rgb", "lab"], "to": "grayscale"})).unwrap();
    assert_eq!(doc(&s).mode, ColorMode::Grayscale);
    s.execute("file.automate.conditionalModeChange", json!({"to": "rgb"})).unwrap();
    assert_eq!(doc(&s).mode, ColorMode::Rgb);
    assert!(s.execute("file.automate.conditionalModeChange", json!({"to": "duotone"})).is_err());
}

#[test]
fn fit_image_zero_or_negative_side_is_unconstrained() {
    // Dialogs send 0 for an empty field: that side is not a limit, not a 1 px box.
    let mut s = session(1000, 800, 8);
    s.execute("file.automate.fitImage", json!({"width": 0, "height": 400})).unwrap();
    assert_eq!((doc(&s).size.width, doc(&s).size.height), (500, 400));
    s.execute("file.automate.fitImage", json!({"width": 250, "height": -5})).unwrap();
    assert_eq!((doc(&s).size.width, doc(&s).size.height), (250, 200));
    let r = s.execute("file.automate.fitImage", json!({"width": 0, "height": 0})).unwrap();
    assert_eq!(r["changed"], false);
    assert_eq!((doc(&s).size.width, doc(&s).size.height), (250, 200));
}

#[test]
fn conditional_mode_change_single_gray_source_matches_grayscale() {
    // A single `"from"` string is normalised like the array form: "gray" names Grayscale (#7042).
    let mut s = session(4, 4, 8);
    s.execute("image.mode.grayscale", json!({})).unwrap();
    let r = s.execute("file.automate.conditionalModeChange", json!({"from": "rgb", "to": "rgb"})).unwrap();
    assert_eq!(r["changed"], false);
    assert_eq!(doc(&s).mode, ColorMode::Grayscale);
    let r = s.execute("file.automate.conditionalModeChange", json!({"from": "gray", "to": "rgb"})).unwrap();
    assert_eq!(r["changed"], true, "{r}");
    assert_eq!(doc(&s).mode, ColorMode::Rgb);
    s.execute("image.mode.grayscale", json!({})).unwrap();
    let r = s.execute("file.automate.conditionalModeChange", json!({"from": "Gray", "to": "rgb"})).unwrap();
    assert_eq!(r["changed"], true, "{r}");
    assert_eq!(doc(&s).mode, ColorMode::Rgb);
}

#[test]
fn conditional_mode_change_cannot_run_a_mode_change_the_gate_denies() {
    fn deny_mode(id: &str, _: &Value) -> Result<()> {
        if id.starts_with("image.mode.") { Err(EngineError::Other(format!("automation command `{id}` is disabled"))) } else { Ok(()) }
    }
    let mut s = session(4, 4, 8);
    s.authorize = Some(deny_mode);
    let e = s.execute("file.automate.conditionalModeChange", json!({"to": "grayscale"})).unwrap_err();
    assert!(e.to_string().contains("image.mode.grayscale"), "{e}");
    assert_eq!(doc(&s).mode, ColorMode::Rgb, "the refused nested step leaves the document alone");
    // Commands that compose only allowed steps still run.
    s.execute("file.automate.fitImage", json!({"width": 2, "height": 2})).unwrap();
    assert_eq!(doc(&s).size.width, 2);
}

#[test]
fn flatten_all_layer_effects_and_masks() {
    for depth in [8, 16, 32] {
        let mut s = session(40, 40, depth);
        s.execute("layer.new.layer", json!({})).unwrap();
        s.execute("select.rect", json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap();
        s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
        s.execute("select.deselect", json!({})).unwrap();
        let id = s.active().unwrap().active_layer.unwrap();
        s.edit("fx", |doc, _| {
            let l = doc.layer_mut(id).unwrap();
            l.effects.enabled = true;
            l.effects.items.push(photocraft_doc::Effect::ColorOverlay {
                common: photocraft_doc::effects::FxCommon::new(photocraft_color::BlendMode::Normal, 1.0),
                color: photocraft_color::Color::rgb(0.0, 1.0, 0.0),
            });
            Ok(())
        })
        .unwrap();
        let before = composite(&s, 15, 15);
        assert!(before[1] > 0.99, "overlay renders green");
        assert_eq!(s.execute("file.scripts.flattenAllLayerEffects", json!({})).unwrap()["flattened"], 1);
        let l = doc(&s).layer(id).unwrap();
        assert!(l.effects.items.is_empty() && matches!(l.content, LayerContent::Raster(_)));
        let after = composite(&s, 15, 15);
        assert!((after[1] - before[1]).abs() < 0.01 && after[0] < 0.01, "{after:?}");
        // Masks: hide the left half, flatten, mask gone, pixels gone.
        s.execute("select.rect", json!({"x": 0, "y": 0, "width": 20, "height": 40})).unwrap();
        s.execute("select.inverse", json!({})).unwrap();
        s.execute("layer.layerMask.revealSelection", json!({})).unwrap();
        assert!(composite(&s, 15, 15)[1] > 0.99 && composite(&s, 15, 15)[0] > 0.99, "white shows through the mask");
        let r = s.execute("file.scripts.flattenAllMasks", json!({})).unwrap();
        assert_eq!(r["flattened"], 1);
        let l = doc(&s).layer(id).unwrap();
        assert!(l.mask.is_none());
        assert!(l.surface().unwrap().content_bounds().x0 >= 20);
        assert!(composite(&s, 25, 15)[1] > 0.99 && composite(&s, 25, 15)[0] < 0.01);
        // Each script is a single history step.
        assert!(s.undo());
        assert!(doc(&s).layer(id).unwrap().mask.is_some());
    }
}

/// Flatten All Layer Effects must bake the effect visuals without destroying the layer's masks:
/// an enabled raster mask and a vector mask on one layer, a disabled raster mask on another
/// (#3804). The command is a visual no-op; the masks (pixels and state) survive, including a
/// `.pcraft` round trip.
#[test]
fn flatten_all_layer_effects_keeps_masks() {
    for depth in [8, 16, 32] {
        let mut s = session(40, 40, depth);
        // Layer 1: a red square with a green colour overlay, an enabled raster mask revealing
        // x < 20 and a vector mask revealing x 15..30.
        s.execute("layer.new.layer", json!({})).unwrap();
        s.execute("select.rect", json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap();
        s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
        s.execute("select.deselect", json!({})).unwrap();
        let id1 = s.active().unwrap().active_layer.unwrap();
        s.edit("fx", |doc, _| {
            let l = doc.layer_mut(id1).unwrap();
            l.effects.enabled = true;
            l.effects.items.push(photocraft_doc::Effect::ColorOverlay {
                common: photocraft_doc::effects::FxCommon::new(photocraft_color::BlendMode::Normal, 1.0),
                color: photocraft_color::Color::rgb(0.0, 1.0, 0.0),
            });
            l.vector_mask = Some(photocraft_doc::VectorMask::new(photocraft_doc::Path::new(vec![photocraft_doc::Subpath::polygon(&[
                (15.0, 15.0),
                (30.0, 15.0),
                (30.0, 30.0),
                (15.0, 30.0),
            ])])));
            Ok(())
        })
        .unwrap();
        s.execute("select.rect", json!({"x": 0, "y": 0, "width": 20, "height": 40})).unwrap();
        s.execute("layer.layerMask.revealSelection", json!({})).unwrap();
        // Layer 2: a red square at the bottom-right with the same overlay and a *disabled* mask.
        s.execute("layer.new.layer", json!({})).unwrap();
        s.execute("select.rect", json!({"x": 30, "y": 30, "width": 8, "height": 8})).unwrap();
        s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
        s.execute("select.deselect", json!({})).unwrap();
        let id2 = s.active().unwrap().active_layer.unwrap();
        s.edit("fx", |doc, _| {
            let l = doc.layer_mut(id2).unwrap();
            l.effects.enabled = true;
            l.effects.items.push(photocraft_doc::Effect::ColorOverlay {
                common: photocraft_doc::effects::FxCommon::new(photocraft_color::BlendMode::Normal, 1.0),
                color: photocraft_color::Color::rgb(0.0, 1.0, 0.0),
            });
            Ok(())
        })
        .unwrap();
        s.execute("select.rect", json!({"x": 34, "y": 34, "width": 2, "height": 2})).unwrap();
        s.execute("layer.layerMask.revealSelection", json!({})).unwrap();
        s.edit("disable", |doc, _| {
            if let Some(m) = doc.layer_mut(id2).and_then(|l| l.mask.as_mut()) {
                m.enabled = false;
            }
            Ok(())
        })
        .unwrap();
        let m1 = doc(&s).layer(id1).unwrap().mask.clone();
        let v1 = doc(&s).layer(id1).unwrap().vector_mask.clone();
        let m2 = doc(&s).layer(id2).unwrap().mask.clone();
        assert_eq!(m1.as_ref().map(|m| m.enabled), Some(true), "layer 1 raster mask is enabled");
        assert!(v1.as_ref().is_some_and(|v| v.enabled), "layer 1 vector mask is enabled");
        assert_eq!(m2.as_ref().map(|m| m.enabled), Some(false), "layer 2 raster mask is disabled");
        let before = [
            composite(&s, 17, 15), // layer 1: inside both masks
            composite(&s, 22, 15), // layer 1: raster mask hides
            composite(&s, 12, 15), // layer 1: vector mask hides
            composite(&s, 34, 34), // layer 2: the disabled mask does not hide
        ];
        assert!(before[0][1] > 0.99 && before[0][0] < 0.01, "overlay renders green: {:?}", before[0]);
        // The white background shows where a mask hides the layer.
        assert!(before[1][0] > 0.99 && before[1][1] > 0.99, "the raster mask hides: {:?}", before[1]);
        assert!(before[2][0] > 0.99 && before[2][1] > 0.99, "the vector mask hides: {:?}", before[2]);
        assert!(before[3][1] > 0.99, "the disabled mask does not hide");
        let r = s.execute("file.scripts.flattenAllLayerEffects", json!({})).unwrap();
        assert_eq!(r["flattened"], 2);
        {
            let d = doc(&s);
            assert_eq!(d.layer(id1).unwrap().mask, m1, "layer 1 raster mask survives");
            assert_eq!(d.layer(id1).unwrap().vector_mask, v1, "layer 1 vector mask survives");
            assert_eq!(d.layer(id2).unwrap().mask, m2, "the disabled mask survives");
        }
        let after = [composite(&s, 17, 15), composite(&s, 22, 15), composite(&s, 12, 15), composite(&s, 34, 34)];
        for (a, b) in after.iter().zip(&before) {
            for k in 0..4 {
                assert!((a[k] - b[k]).abs() < 0.01, "flattening is a visual no-op: {after:?} vs {before:?}");
            }
        }
        // One history step: undo restores the effects, redo re-flattens, and the masks stay.
        assert!(s.undo());
        assert!(!doc(&s).layer(id1).unwrap().effects.items.is_empty());
        assert_eq!(doc(&s).layer(id1).unwrap().mask, m1);
        assert!(s.redo());
        assert_eq!(doc(&s).layer(id1).unwrap().mask, m1);
        assert_eq!(doc(&s).layer(id1).unwrap().vector_mask, v1);
        assert_eq!(doc(&s).layer(id2).unwrap().mask, m2);
        // The masks survive a `.pcraft` save and reopen.
        let dir = tmp("flatten-effects-masks");
        let path = join(&dir, "masks.pcraft");
        save_doc(doc(&s), &path, None).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        let mut s2 = Session::new();
        open_bytes_as(&mut s2, "masks.pcraft", &bytes, None, None).unwrap();
        let layers: Vec<_> = doc(&s2).walk().into_iter().map(|(_, _, l)| l).collect();
        assert!(layers.iter().any(|l| l.mask == m1 && l.vector_mask == v1), "the raster and vector masks survive .pcraft");
        assert!(layers.iter().any(|l| l.mask == m2), "the disabled mask survives .pcraft");
    }
}

/// Flattens all layer effects on a red square carrying `fx` (in a document prepared by `setup`)
/// and asserts the composite did not change: the bake uses the document's own context (#5719).
fn assert_flatten_effects_is_a_no_op(depth: u32, setup: impl FnOnce(&mut Document), fx: photocraft_doc::Effect) {
    let mut s = session(40, 40, depth);
    s.execute("layer.new.layer", json!({})).unwrap();
    s.execute("select.rect", json!({"x": 14, "y": 14, "width": 12, "height": 12})).unwrap();
    s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
    s.execute("select.deselect", json!({})).unwrap();
    let id = s.active().unwrap().active_layer.unwrap();
    s.edit("fx", |doc, _| {
        setup(doc);
        let l = doc.layer_mut(id).unwrap();
        l.effects.enabled = true;
        l.effects.items.push(fx);
        Ok(())
    })
    .unwrap();
    let before = photocraft_compose::flatten(doc(&s));
    assert_eq!(s.execute("file.scripts.flattenAllLayerEffects", json!({})).unwrap()["flattened"], 1);
    let after = photocraft_compose::flatten(doc(&s));
    for (i, (a, b)) in after.px.iter().zip(&before.px).enumerate() {
        for k in 0..4 {
            assert!((a[k] - b[k]).abs() < 0.02, "depth {depth}: pixel ({}, {}) changed: {b:?} -> {a:?}", i % 40, i / 40);
        }
    }
}

/// A Drop Shadow that uses Global Light is baked at the document's angle, not the default 120°.
#[test]
fn flatten_all_layer_effects_uses_the_documents_global_light() {
    for depth in [8, 16, 32] {
        let shadow = photocraft_doc::Effect::DropShadow(photocraft_doc::effects::Shadow {
            common: photocraft_doc::effects::FxCommon::new(photocraft_color::BlendMode::Normal, 1.0),
            color: photocraft_color::Color::rgb(0.0, 0.0, 1.0),
            angle: 120.0,
            use_global_light: true,
            distance: 10.0,
            spread: 0.0,
            size: 0.0,
            contour: photocraft_doc::effects::Contour::Linear,
            anti_alias: false,
            noise: 0.0,
            knocks_out: true,
        });
        assert_flatten_effects_is_a_no_op(depth, |d| d.global_light.angle = 0.0, shadow);
    }
}

/// A Pattern Overlay of a document pattern survives the bake instead of vanishing.
#[test]
fn flatten_all_layer_effects_keeps_a_pattern_overlay() {
    for depth in [8, 16, 32] {
        let mut tile = photocraft_raster::Surface::new(PixelFormat::new(ColorMode::Rgb, photocraft_color::SampleType::U8, true));
        tile.fill_rect(Rect::new(0, 0, 2, 2), &[0.0, 1.0, 0.0, 1.0]);
        let pat = photocraft_doc::Pattern::new("green", tile, 2, 2);
        let overlay = photocraft_doc::Effect::PatternOverlay {
            common: photocraft_doc::effects::FxCommon::new(photocraft_color::BlendMode::Normal, 1.0),
            name: pat.name.clone(),
            id: pat.id.clone(),
            scale: 1.0,
            angle: 0.0,
            link: true,
            phase: (0.0, 0.0),
        };
        assert_flatten_effects_is_a_no_op(depth, |d| d.patterns.push(pat), overlay);
    }
}

#[test]
fn layers_to_files_reports_skipped_layers_without_changing_files() {
    for depth in [8, 16, 32] {
        let dir = tmp(&format!("skipped-layers-{depth}"));
        let mut s = session(4, 4, depth);
        let adjustment = s.execute("layer.newAdjustmentLayer.levels", json!({})).unwrap()["layer"].clone();
        s.execute("layer.setProps", json!({"name": "Levels/α"})).unwrap();
        let hidden = s.execute("layer.new.layer", json!({"name": "Levels/α"})).unwrap()["layer"].clone();
        s.execute("layer.setProps", json!({"visible": false})).unwrap();
        let before = s.active().unwrap().doc.clone();
        let result = s.execute("file.export.layersToFiles", json!({"dir": dir, "prefix": "doc"})).unwrap();
        assert_eq!(
            result["skipped"],
            json!([
                {"layer": hidden, "name": "Levels/α", "reason": "hidden by visibleOnly"},
                {"layer": adjustment, "name": "Levels/α", "reason": "adjustment layer has no pixels of its own"}
            ])
        );
        assert_eq!(result["files"], json!([join(&dir, "doc_0001_Background.png")]));
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
        assert_eq!(s.active().unwrap().doc.layers.len(), before.layers.len());
        assert!(!s.active().unwrap().doc.layer(LayerId(hidden.as_u64().unwrap())).unwrap().visible);
        let all = s.execute("file.export.layersToFiles", json!({"dir": dir, "prefix": "all", "visibleOnly": false})).unwrap();
        assert_eq!(
            all["skipped"],
            json!([
                {"layer": adjustment, "name": "Levels/α", "reason": "adjustment layer has no pixels of its own"}
            ])
        );
        assert_eq!(all["files"], json!([join(&dir, "all_0000_Levels_α.png"), join(&dir, "all_0002_Background.png")]));
    }
}

#[test]
fn layers_to_files_reports_groups_as_exports_and_failures_as_errors() {
    let mut s = session(4, 4, 8);
    s.execute("layer.newAdjustmentLayer.levels", json!({})).unwrap();
    s.execute("layer.new.groupFromLayers", json!({"name": "Pack"})).unwrap();
    let params = json!({"prefix": "doc"});
    let mut names = Vec::new();
    let report = export_layers_report_with(&s, &params, |name, bytes| {
        assert!(!bytes.is_empty());
        names.push(name.to_string());
        Ok(())
    })
    .unwrap();
    assert_eq!(names, vec!["doc_0000_Pack.png", "doc_0001_Background.png"]);
    assert_eq!(report.files, names);
    assert!(report.skipped.is_empty(), "a group's adjustment is not an omitted top-level layer");
    assert_eq!(export_layers_with(&s, &params, |_, _| Ok(())).unwrap(), names, "legacy Rust helper remains compatible");
    assert!(export_layers_report_with(&s, &params, |_, _| Err(EngineError::Other("disk full".into()))).unwrap_err().to_string().contains("disk full"));
    assert!(export_layers_report_with(&s, &json!({"format": "not-a-format"}), |_, _| panic!("must not write")).is_err());
    assert!(export_layers_report_with(&Session::new(), &params, |_, _| panic!("must not write")).is_err());
    s.edit("Empty", |doc, _| {
        doc.layers.clear();
        Ok(())
    })
    .unwrap();
    let empty = export_layers_report_with(&s, &params, |_, _| panic!("must not write")).unwrap();
    assert!(empty.files.is_empty());
    assert!(empty.skipped.is_empty());
}

#[test]
fn layers_to_files_writes_each_visible_layer() {
    let dir = tmp("layers");
    let mut s = session(12, 12, 8);
    s.execute("layer.new.layer", json!({"name": "Ink/1"})).unwrap();
    s.execute("layer.new.layer", json!({"name": "Hidden"})).unwrap();
    s.execute("layer.setProps", json!({"visible": false})).unwrap();
    let r = s.execute("file.export.layersToFiles", json!({"dir": dir, "prefix": "doc", "format": "png"})).unwrap();
    let files: Vec<String> = r["files"].as_array().unwrap().iter().map(|v| v.as_str().unwrap().to_string()).collect();
    assert_eq!(files.len(), 2, "{files:?}");
    assert!(files[0].ends_with("doc_0000_Ink_1.png"), "{files:?}");
    assert!(files.iter().all(|f| std::path::Path::new(f).is_file()));
    let r = s.execute("file.export.layersToFiles", json!({"dir": dir, "visibleOnly": false, "format": "psd"})).unwrap();
    assert_eq!(r["files"].as_array().unwrap().len(), 3);
}

#[test]
fn layers_to_files_can_trim_transparent_margins_without_changing_the_document() {
    let dir = tmp("layers-trim");
    for depth in [8, 16, 32] {
        let mut s = session(12, 11, depth);
        s.execute("layer.new.layer", json!({"name": "Ink"})).unwrap();
        s.execute("select.rect", json!({"x": 3, "y": 2, "width": 4, "height": 5})).unwrap();
        s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
        s.execute("select.deselect", json!({})).unwrap();

        let before = doc(&s).clone();
        let regular = s.execute("file.export.layersToFiles", json!({"dir": dir, "prefix": format!("full-{depth}"), "format": "png"})).unwrap();
        let full = std::fs::read(regular["files"][0].as_str().unwrap()).unwrap();
        let full = photocraft_io::import("full.png", &full).unwrap().document;
        assert_eq!((full.size.width, full.size.height), (12, 11), "default is unchanged");

        let trimmed = s.execute("file.export.layersToFiles", json!({"dir": dir, "prefix": format!("trim-{depth}"), "format": "png", "trim": true})).unwrap();
        let image = std::fs::read(trimmed["files"][0].as_str().unwrap()).unwrap();
        let cropped = photocraft_io::import("trim.png", &image).unwrap().document;
        assert_eq!((cropped.size.width, cropped.size.height), (4, 5), "depth {depth}");
        let px = photocraft_compose::render(&cropped, Rect::new(0, 0, 1, 1)).px[0];
        assert!(px[0] > 0.95 && px[1] < 0.05 && px[2] < 0.05, "depth {depth}: {px:?}");
        assert_eq!(*doc(&s), before, "export must not crop the original document");
    }
}

#[test]
fn streamed_layer_export_respects_trim() {
    let mut s = session(12, 11, 8);
    s.execute("layer.new.layer", json!({"name": "Ink"})).unwrap();
    s.execute("select.rect", json!({"x": 3, "y": 2, "width": 4, "height": 5})).unwrap();
    s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
    s.execute("select.deselect", json!({})).unwrap();
    let mut encoded = Vec::new();
    let names = super::export_layers_with(&s, &json!({"trim": true, "format": "png"}), |name, bytes| {
        encoded.push((name.to_string(), bytes.to_vec()));
        Ok(())
    })
    .unwrap();
    assert_eq!(names.len(), encoded.len());
    let cropped = photocraft_io::import(&encoded[0].0, &encoded[0].1).unwrap().document;
    assert_eq!((cropped.size.width, cropped.size.height), (4, 5));
}

#[test]
fn trimming_an_empty_layer_keeps_a_valid_nonzero_canvas() {
    let dir = tmp("layers-trim-empty");
    let mut s = session(6, 7, 8);
    s.execute("layer.new.layer", json!({"name": "Empty"})).unwrap();
    let result = s.execute("file.export.layersToFiles", json!({"dir": dir, "format": "png", "trim": true})).unwrap();
    let bytes = std::fs::read(result["files"][0].as_str().unwrap()).unwrap();
    let back = photocraft_io::import("empty.png", &bytes).unwrap().document;
    assert_eq!((back.size.width, back.size.height), (6, 7));
}

#[test]
fn load_files_into_stack_and_image_processor_and_batch() {
    let dir = tmp("batch");
    let input = join(&dir, "in");
    std::fs::create_dir_all(&input).unwrap();
    let a = png(&input, "a.png", 40, 20, "#ff0000");
    let b = png(&input, "b.png", 30, 30, "#0000ff");
    std::fs::write(join(&input, "notes.txt"), "not an image").unwrap();
    let mut s = Session::new();
    let r = s.execute("file.scripts.loadFilesIntoStack", json!({"paths": [a, b]})).unwrap();
    assert_eq!(r["layers"], 2);
    assert_eq!((doc(&s).size.width, doc(&s).size.height), (40, 30));
    assert_eq!(doc(&s).layers[0].name, "a.png");
    // Image Processor: fit into 20×20 JPEGs.
    let out = join(&dir, "jpg");
    let r = s.execute("file.scripts.imageProcessor", json!({"input": input, "output": out, "format": "jpg", "width": 20, "height": 20})).unwrap();
    let files = r["files"].as_array().unwrap();
    assert_eq!(files.len(), 2, "{r}");
    let back = photocraft_io::import("a.jpg", &std::fs::read(files[0].as_str().unwrap()).unwrap()).unwrap().document;
    assert_eq!((back.size.width, back.size.height), (20, 10));
    // Batch: run an action (invert) over the folder, save PNGs.
    let out = join(&dir, "png");
    let r = s.execute("file.automate.batch", json!({"steps": [["image.adjustments.invert", {}]], "input": input, "output": out, "format": "png"})).unwrap();
    assert_eq!(r["files"].as_array().unwrap().len(), 2, "{r}");
    assert!(r["errors"].as_array().unwrap().is_empty());
    let inv = photocraft_io::import("a.png", &std::fs::read(join(&out, "a.png")).unwrap()).unwrap().document;
    let px = photocraft_compose::render(&inv, Rect::new(1, 1, 2, 2)).px[0];
    assert!(px[0] < 0.01 && px[1] > 0.99 && px[2] > 0.99, "red inverted to cyan: {px:?}");
    // A recorded Quick Export writes to the current document path, but Batch owns
    // the destination. Replaying it would fail for scratch documents with no save path.
    let quick_out = join(&dir, "batch-with-quick-export");
    let r = s
        .execute(
            "file.automate.batch",
            json!({"steps": [["image.adjustments.invert", {}], ["file.export.quickExport", {}]], "input": input, "output": quick_out, "format": "png"}),
        )
        .unwrap();
    assert_eq!(r["files"].as_array().unwrap().len(), 2, "{r}");
    assert!(r["errors"].as_array().unwrap().is_empty(), "{r}");
    assert!(std::path::Path::new(&join(&quick_out, "a.png")).is_file());
    assert!(s.execute("file.automate.batch", json!({"steps": [["no.such.command", {}]], "input": input, "output": out})).is_err());
}

#[test]
fn color_lookup_table_bakes_the_adjustment_stack() {
    let mut s = session(8, 8, 8);
    assert!(!s.is_enabled("file.export.colorLookupTables"));
    s.execute("layer.newAdjustmentLayer.invert", json!({})).unwrap();
    let r = s.execute("file.export.colorLookupTables", json!({"size": 3})).unwrap();
    let cube = r["cube"].as_str().unwrap();
    assert!(cube.contains("LUT_3D_SIZE 3"));
    let rows: Vec<Vec<f32>> =
        cube.lines().filter(|l| l.chars().next().is_some_and(|c| c.is_ascii_digit())).map(|l| l.split(' ').map(|v| v.parse().unwrap()).collect()).collect();
    assert_eq!(rows.len(), 27);
    // First entry (0,0,0) → white; second (r = 0.5) → 0.5; last (1,1,1) → black.
    assert!(rows[0].iter().all(|v| (v - 1.0).abs() < 1e-3), "{:?}", rows[0]);
    assert!((rows[1][0] - 0.5).abs() < 0.01 && (rows[1][1] - 1.0).abs() < 1e-3);
    assert!(rows[2][0].abs() < 1e-3 && (rows[2][2] - 1.0).abs() < 1e-3, "red fastest");
    assert!(rows[26].iter().all(|v| v.abs() < 1e-3));
    let dir = tmp("lut");
    let path = join(&dir, "invert.cube");
    s.execute("file.export.colorLookupTables", json!({"path": path, "size": 17})).unwrap();
    assert!(std::fs::read_to_string(&path).unwrap().contains("LUT_3D_SIZE 17"));
}

#[test]
fn cube_export_uses_only_selected_adjustments_in_document_order() {
    let mut s = session(8, 8, 8);
    let first = s.execute("layer.newAdjustmentLayer.invert", json!({})).unwrap()["layer"].as_u64().unwrap();
    let second = s.execute("layer.newAdjustmentLayer.invert", json!({})).unwrap()["layer"].as_u64().unwrap();
    let both = s.execute("file.export.colorLookupTables", json!({"size": 3})).unwrap();
    let all = photocraft_cms::lutfile::parse_cube(both["cube"].as_str().unwrap()).unwrap();
    assert_eq!(both["layerCount"], 2);
    assert!(all.data[0] < 0.001, "two Invert adjustments produce identity");

    // Selection comes from the Layers panel rather than the active layer alone.
    s.execute("layer.select", json!({"layer": first})).unwrap();
    let selected = s.execute("file.export.colorLookupTables", json!({"size": 3, "scope": "selected"})).unwrap();
    let one = photocraft_cms::lutfile::parse_cube(selected["cube"].as_str().unwrap()).unwrap();
    assert_eq!(selected["layerCount"], 1);
    assert!(one.data[0] > 0.999, "one invert maps black to white");
    assert_eq!(all.size, one.size);
    assert_eq!(s.execute("file.export.colorLookupTables", json!({"size": 3, "layers": [first]})).unwrap()["cube"], selected["cube"]);

    // Reversing the caller's ID list must not reverse document stack order.
    let normal = s.execute("file.export.colorLookupTables", json!({"size": 3, "layers": [first, second]})).unwrap();
    let reversed = s.execute("file.export.colorLookupTables", json!({"size": 3, "layers": [second, first]})).unwrap();
    assert_eq!(normal["cube"], reversed["cube"]);
    assert_eq!(normal["cube"], both["cube"]);

    // Additional non-adjustment layers must not affect the LUT export.
    s.execute("layer.new.layer", json!({"name": "Raster"})).unwrap();
    assert_eq!(s.execute("file.export.colorLookupTables", json!({"size": 3})).unwrap()["cube"], both["cube"]);
}

#[test]
fn selected_cube_validation_is_explicit_and_never_mutates_the_document() {
    let mut s = session(8, 8, 8);
    let a = s.execute("layer.newAdjustmentLayer.invert", json!({})).unwrap()["layer"].as_u64().unwrap();
    let raster = s.execute("layer.new.layer", json!({})).unwrap()["layer"].as_u64().unwrap();
    let rev = s.active().unwrap().revision;
    let history = s.active().unwrap().history.past_len();
    let valid = s.execute("file.export.colorLookupTables", json!({"size": 3, "layers": [a]})).unwrap();
    assert_eq!(valid["layerCount"], 1);
    for params in [
        json!({"size": 1}),
        json!({"size": 130}),
        json!({"size": 1.5}),
        json!({"size": -3}),
        json!({"scope": "unknown"}),
        json!({"scope": 42}),
        json!({"scope": "selected", "layers": [a]}),
        json!({"layers": []}),
        json!({"layers": 42}),
        json!({"layers": [a, a]}),
        json!({"layers": [a, "oops"]}),
        json!({"layers": [u64::MAX]}),
        json!({"layers": [raster]}),
    ] {
        assert!(s.execute("file.export.colorLookupTables", params.clone()).is_err(), "{params}");
        assert_eq!(s.active().unwrap().revision, rev, "{params}: export must be read-only");
        assert_eq!(s.active().unwrap().history.past_len(), history);
    }
    s.execute("layer.select", json!({"layer": raster})).unwrap();
    assert!(s.execute("file.export.colorLookupTables", json!({"size": 3, "scope": "selected"})).is_err());
}

#[test]
fn exported_selected_cube_round_trips_through_lookup_importer() {
    let dir = tmp("selected_cube");
    let mut s = session(8, 8, 8);
    let a = s.execute("layer.newAdjustmentLayer.invert", json!({})).unwrap()["layer"].as_u64().unwrap();
    let path = join(&dir, "chosen.cube");
    let r = s.execute("file.export.colorLookupTables", json!({"size": 17, "title": "Chosen", "layers": [a], "path": path})).unwrap();
    assert_eq!(r["size"], 17);
    assert_eq!(r["layerCount"], 1);
    let text = std::fs::read_to_string(&path).unwrap();
    let parsed = photocraft_cms::lutfile::parse_cube(&text).unwrap();
    assert_eq!(parsed.size, 17);
    assert_eq!(parsed.data.len(), 17 * 17 * 17 * 3);
    assert!(parsed.data[..3].iter().all(|v| *v > 0.999));
    assert!(parsed.data[parsed.data.len() - 3..].iter().all(|v| *v < 0.001));
    s.execute("layer.newAdjustmentLayer.colorLookup", json!({"file": path})).unwrap();
    assert_eq!(s.active().unwrap().doc.layers.len(), 3);
}

/// #4984: New Guide Layout takes lengths with units as typed in Photoshop's dialog, at the
/// document's resolution; percent is of the target's width or height, by axis.
#[test]
fn guide_layouts_take_units() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 1200, "height": 600, "resolution": 300})).unwrap();
    let close = |a: &[f64], b: &[f64]| a.len() == b.len() && a.iter().zip(b).all(|(x, y)| (x - y).abs() < 0.01);
    // 1 cm at 300 ppi is 118.11 px; 0.25 in is 75 px.
    s.execute("view.newGuideLayout", json!({"columns": 2, "gutter": "1 cm", "margin": "0.25in"})).unwrap();
    let g = &doc(&s).guides;
    let col = (1050.0 - 118.110_236) / 2.0;
    assert!(close(&g.vertical, &[75.0, 1125.0, 75.0 + col, 75.0 + col + 118.110_236]), "{:?}", g.vertical);
    assert!(close(&g.horizontal, &[75.0, 525.0]), "{:?}", g.horizontal);
    // Percent of the width for columns, of the height for rows and top/bottom margins; points
    // follow Units & Rulers (72 per inch: 36 pt is 150 px, centred like the columns).
    s.execute(
        "view.newGuideLayout",
        json!({"columns": 1, "width": "25%", "centerColumns": true, "rows": 1, "height": "36 pt", "margin": ["10%", 0, "10%", 0], "clearExisting": true}),
    )
    .unwrap();
    let g = &doc(&s).guides;
    assert!(close(&g.vertical, &[0.0, 1200.0, 450.0, 750.0]), "{:?}", g.vertical);
    assert!(close(&g.horizontal, &[60.0, 540.0, 225.0, 375.0]), "{:?}", g.horizontal);
    // With selected artboards, percent is of each board, not of the document.
    s.execute("layer.new.artboard", json!({"rect": [200, 100, 140, 40]})).unwrap();
    s.execute(
        "view.newGuideLayout",
        json!({"columns": 1, "width": "50%", "centerColumns": true, "rows": 1, "height": "50%", "target": "selectedArtboards", "clearExisting": true}),
    )
    .unwrap();
    let g = &doc(&s).guides;
    // Clear Existing removes guides inside the board only; those outside it stay.
    assert!(close(&g.vertical, &[0.0, 1200.0, 450.0, 750.0, 235.0, 305.0]), "{:?}", g.vertical);
    assert!(close(&g.horizontal, &[60.0, 540.0, 225.0, 375.0, 110.0, 130.0]), "{:?}", g.horizontal);
    for bad in [
        json!({"columns": 2, "gutter": "1 furlong"}),
        json!({"columns": 2, "gutter": "-1 cm"}),
        json!({"columns": 2, "margin": [0, "x", 0, 0]}),
        json!({"columns": 2, "gutter": true}),
    ] {
        assert!(s.execute("view.newGuideLayout", bad.clone()).is_err(), "{bad}");
    }
}

#[test]
fn guide_layouts() {
    let mut s = session(100, 50, 8);
    let r = s.execute("view.newGuideLayout", json!({"columns": 2, "gutter": 10, "margin": [5, 10, 5, 10]})).unwrap();
    assert_eq!(r["vertical"], 4);
    let g = &doc(&s).guides;
    assert_eq!(g.vertical, vec![10.0, 90.0, 45.0, 55.0]);
    assert_eq!(g.horizontal, vec![5.0, 45.0]);
    s.execute("view.newGuideLayout", json!({"rows": 2, "clearExisting": true})).unwrap();
    let g = &doc(&s).guides;
    assert!(g.vertical.is_empty());
    assert_eq!(g.horizontal, vec![0.0, 25.0, 50.0]);
    // Fixed-width centred columns.
    s.execute("view.newGuideLayout", json!({"columns": 2, "width": 20, "gutter": 10, "centerColumns": true, "clearExisting": true})).unwrap();
    assert_eq!(doc(&s).guides.vertical, vec![25.0, 45.0, 55.0, 75.0]);
    assert!(s.execute("view.newGuideLayout", json!({})).is_err());
    assert_eq!(s.execute("view.clearCanvasGuides", json!({})).unwrap()["cleared"], 4);
    assert!(doc(&s).guides.vertical.is_empty());
    assert!(!s.is_enabled("view.newGuidesFromShape"));
    s.execute("shape.create", json!({"kind": "rect", "rect": [10, 10, 40, 20]})).unwrap();
    s.execute("view.newGuidesFromShape", json!({})).unwrap();
    let g = &doc(&s).guides;
    assert_eq!(g.vertical, vec![10.0, 30.0, 50.0]);
    assert_eq!(g.horizontal, vec![10.0, 20.0, 30.0]);
}

#[test]
fn only_layered_files_save_in_place() {
    for path in ["a.psd", "dir/a.PSB", r"C:\w\a.pcraft", "my.dir/a.psd", "a.ora", "dir/A.ORA"] {
        assert!(saves_in_place(path), "{path}");
    }
    // Flat formats, no extension, a dotted folder with an extensionless file, a dot file.
    for path in ["a.png", "a.jpg", "a.pdn", "a.PDN", "a", "my.psd/a", ".psd", "a.", ""] {
        assert!(!saves_in_place(path), "{path}");
    }
    assert_eq!(extension("dir/Photo.JPEG").as_deref(), Some("jpeg"));
    assert_eq!(extension("my.dir/name"), None);
}

/// #3806: a layered format holds a document only if it keeps everything in it.
#[test]
fn openraster_holds_a_document_only_when_it_drops_nothing() {
    let mut s = session(16, 16, 8);
    s.execute("layer.new.layer", json!({})).unwrap();
    let plain = doc(&s).clone();
    for p in ["a.psd", "a.psb", "a.pcraft", "a.ora", "A.ORA"] {
        assert!(holds_at(&plain, p), "{p}");
    }
    for p in ["a.png", "a.tif", "a", ""] {
        assert!(!holds_at(&plain, p), "{p}");
    }
    assert!(losses_at(&plain, "a.ora").is_empty());
    s.execute("layer.newAdjustmentLayer.invert", json!({})).unwrap();
    let adjusted = doc(&s).clone();
    assert!(holds_at(&adjusted, "a.psd") && !holds_at(&adjusted, "a.ora"));
    assert!(losses_at(&adjusted, "a.ora").iter().any(|l| l.contains("adjustment layers")), "{:?}", losses_at(&adjusted, "a.ora"));
    assert!(losses_at(&adjusted, "a.psd").is_empty());
}

#[test]
fn templates_open_untitled_without_their_path() {
    assert!(is_template("dir/card.PSDT") && !is_template("card.psd") && !is_template("psdt"));
    let dir = tmp("template");
    let path = format!("{dir}/card.psdt");
    let mut s = session(8, 8, 8);
    std::fs::write(&path, encode(doc(&s), "x.psd", None).unwrap().0).unwrap();
    s.execute("file.openAs", json!({"path": path})).unwrap();
    s.execute("file.openAs", json!({"path": path, "as": "psd"})).unwrap();
    let opened: Vec<_> = s.documents()[1..].iter().map(|d| (d.doc.name.as_str(), d.path.as_deref())).collect();
    assert_eq!(opened, [("Untitled-1", None), ("Untitled-2", None)]);
}

#[test]
fn batch_naming_serial_keeps_same_named_inputs_and_their_pixels() {
    let dir = tmp("naming-serial");
    let first = join(&dir, "first");
    let second = join(&dir, "second");
    std::fs::create_dir_all(&first).unwrap();
    std::fs::create_dir_all(&second).unwrap();
    let inputs = [png(&first, "photo.png", 8, 8, "#ff0000"), png(&second, "photo.png", 8, 8, "#0000ff")];
    let out = join(&dir, "out");
    let mut s = Session::new();
    let result = s
        .execute(
            "file.automate.batch",
            json!({
                "input": inputs, "output": out, "steps": [], "format": "png",
                "naming": "{name}_{serial}", "startingSerial": 7
            }),
        )
        .unwrap();
    assert_eq!(result["errors"], json!([]), "{result}");
    assert_eq!(result["files"], json!([join(&out, "photo_007"), join(&out, "photo_008")]));
    for (name, expected) in [("photo_007", [1.0, 0.0, 0.0, 1.0]), ("photo_008", [0.0, 0.0, 1.0, 1.0])] {
        let saved = photocraft_io::import("photo.png", &std::fs::read(join(&out, name)).unwrap()).unwrap().document;
        let pixel = photocraft_compose::render(&saved, Rect::new(1, 1, 2, 2)).px[0];
        assert_eq!(pixel, expected, "{name}");
    }
}

/// Batch follows Photoshop's later-file-wins behavior. Image Processor and lens correction
/// keep their existing collision error so their outputs are never silently replaced.
#[test]
fn batch_reports_inputs_that_share_an_output_name() {
    let dir = tmp("collide");
    let input = join(&dir, "in");
    std::fs::create_dir_all(&input).unwrap();
    png(&input, "a.png", 8, 8, "#ff0000");
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 8, "height": 8, "background": "#0000ff"})).unwrap();
    save_doc(doc(&s), &join(&input, "a.jpg"), None).unwrap();
    png(&input, "B.png", 8, 8, "#00ff00");
    png(&input, "b.tif", 8, 8, "#00ff00");
    for (cmd, extra) in [
        ("file.automate.batch", json!({"steps": [], "format": "jpg"})),
        ("file.scripts.imageProcessor", json!({})),
        ("file.automate.lensCorrection", json!({"format": "jpg"})),
    ] {
        let out = join(&dir, cmd);
        let mut p = extra;
        p["input"] = json!(input);
        p["output"] = json!(out);
        let r = s.execute(cmd, p).unwrap();
        let files = r["files"].as_array().unwrap();
        let errors = r["errors"].as_array().unwrap();
        if cmd == "file.automate.batch" {
            assert_eq!((files.len(), errors.len()), (4, 0), "{cmd}: {r}");
            assert_eq!(files[1], files[2], "the later a.png result replaces a.jpg: {r}");
        } else {
            assert_eq!((files.len(), errors.len()), (2, 2), "{cmd}: {r}");
            assert!(errors.iter().all(|e| e["error"].as_str().unwrap().contains("not written")), "{r}");
            let mut written: Vec<_> = files.iter().map(|f| f.as_str().unwrap().to_lowercase()).collect();
            written.dedup();
            assert_eq!(written.len(), 2, "each listed file is distinct: {r}");
        }
    }
    // Keeping each file's format, nothing collides.
    let out = join(&dir, "same");
    let r = s.execute("file.automate.batch", json!({"steps": [], "input": input, "output": out})).unwrap();
    assert_eq!((r["files"].as_array().unwrap().len(), r["errors"].as_array().unwrap().len()), (4, 0), "{r}");
    // A droplet over two folders holding the same name.
    let other = join(&dir, "other");
    std::fs::create_dir_all(&other).unwrap();
    png(&other, "a.png", 8, 8, "#ffffff");
    let droplet = join(&dir, "d.pcdroplet");
    std::fs::write(&droplet, json!({"photocraftDroplet": 1, "name": "d", "action": {"steps": []}, "options": {"format": "png"}}).to_string()).unwrap();
    let out = join(&dir, "droplet");
    let r = s.execute("file.automate.runDroplet", json!({"droplet": droplet, "input": [join(&input, "a.png"), other], "output": out})).unwrap();
    assert_eq!((r["files"].as_array().unwrap().len(), r["errors"].as_array().unwrap().len()), (2, 0), "{r}");
    let kept = photocraft_io::import("a.png", &std::fs::read(join(&out, "a.png")).unwrap()).unwrap().document;
    let px = photocraft_compose::render(&kept, Rect::new(1, 1, 2, 2)).px[0];
    assert!(px[0] > 0.99 && px[1] > 0.99, "the later input's result replaces the first: {px:?}");
}

/// `file.revealInFinder` (the document tab's Reveal, UI-217-6) reveals the clicked document's
/// saved path — the tab menu passes `document`, not just the active one.
#[test]
fn reveal_in_finder_reveals_the_documents_saved_path() {
    let mut s = session(8, 8, 8);
    assert!(s.is_enabled("file.revealInFinder"));
    // A new document has no file yet.
    let err = s.execute("file.revealInFinder", json!({"dryRun": true})).unwrap_err();
    assert!(err.to_string().contains("no saved file"), "{err}");
    let path = "/tmp/pics/a.psd";
    s.active_mut().unwrap().path = Some(path.into());
    let r = s.execute("file.revealInFinder", json!({"dryRun": true})).unwrap();
    let (program, args) = crate::layer_menu_cmds::reveal_command(path).unwrap();
    assert_eq!(r["program"], json!(program));
    assert_eq!(r["args"], json!(args));
    // The tab menu names a background tab: the active document is not the one revealed.
    s.execute("file.new", json!({"width": 8, "height": 8, "name": "other"})).unwrap();
    let err = s.execute("file.revealInFinder", json!({"document": 1, "dryRun": true})).unwrap_err();
    assert!(err.to_string().contains("no saved file"), "{err}");
    let r = s.execute("file.revealInFinder", json!({"document": 0, "dryRun": true})).unwrap();
    assert_eq!(r["program"], json!(program));
    assert_eq!(r["args"], json!(args));
}

#[cfg(windows)]
#[test]
fn reveal_in_finder_on_windows_passes_the_clicked_tabs_saved_path() {
    let mut s = session(8, 8, 8);
    s.active_mut().unwrap().path = Some("C:/项目 图片/海报 🖼.png".into());
    s.execute("file.new", json!({"width": 8, "height": 8, "name": "other"})).unwrap();

    let r = s.execute("file.revealInFinder", json!({"document": 0, "dryRun": true})).unwrap();
    assert_eq!(r["program"], "explorer");
    assert_eq!(r["args"], json!(["/select,", r"C:\项目 图片\海报 🖼.png"]));
}

/// "Include All Subfolders" (AUTO-219-5, #3753): Batch and Image Processor take the images in
/// subfolders too, each result keeping its subfolder under the output folder (so same-named files
/// don't collide), and skip the output folder when it sits inside the input folder.
#[test]
fn batch_include_all_subfolders() {
    let dir = tmp("batch-subfolders");
    let input = join(&dir, "in");
    for sub in ["sub/deeper", "z"] {
        std::fs::create_dir_all(join(&input, sub)).unwrap();
    }
    png(&input, "a.png", 8, 8, "#ff0000");
    png(&join(&input, "sub"), "b.png", 8, 8, "#00ff00");
    png(&join(&input, "sub/deeper"), "a.png", 8, 8, "#0000ff");
    png(&join(&input, "z"), "a.png", 8, 8, "#ffffff");
    let mut s = Session::new();
    // Default: the top level only, as before.
    let out = join(&input, "batch");
    let r = s.execute("file.automate.batch", json!({"steps": [], "input": input, "output": out, "format": "png"})).unwrap();
    assert_eq!((r["files"].as_array().unwrap().len(), r["errors"].as_array().unwrap().len()), (1, 0), "{r}");
    // With subfolders: every image, in folder order, results saved flat; the output
    // folder (inside the input folder, holding last run's a.png) is not an input.
    let r = s
        .execute(
            "file.automate.batch",
            json!({"steps": [["image.adjustments.invert", {}]], "input": input, "includeSubfolders": true, "output": out, "format": "png"}),
        )
        .unwrap();
    assert!(r["errors"].as_array().unwrap().is_empty(), "{r}");
    let files: Vec<String> = r["files"].as_array().unwrap().iter().map(|f| f.as_str().unwrap().replace('\\', "/")).collect();
    let o = out.replace('\\', "/");
    assert_eq!(files, vec![format!("{o}/a.png"), format!("{o}/b.png"), format!("{o}/a.png"), format!("{o}/a.png")]);
    let last = photocraft_io::import("a.png", &std::fs::read(join(&out, "a.png")).unwrap()).unwrap().document;
    let px = photocraft_compose::render(&last, Rect::new(1, 1, 2, 2)).px[0];
    assert!(px[0] < 0.01 && px[1] < 0.01 && px[2] < 0.01, "the last white input is inverted to black: {px:?}");
    std::fs::remove_dir_all(&out).unwrap();
    // A naming template applies across all inputs, with serials distinguishing the flat outputs.
    let named = join(&dir, "named");
    let r = s
        .execute("file.automate.batch", json!({"steps": [], "input": input, "includeSubfolders": true, "output": named, "format": "png", "naming": "{serial}"}))
        .unwrap();
    assert!(r["errors"].as_array().unwrap().is_empty(), "{r}");
    let files: Vec<String> = r["files"].as_array().unwrap().iter().map(|f| f.as_str().unwrap().replace('\\', "/")).collect();
    let n = named.replace('\\', "/");
    assert_eq!(files, vec![format!("{n}/001"), format!("{n}/002"), format!("{n}/003"), format!("{n}/004")]);
    // Image Processor shares the option (with its results out of the input folder).
    let out = join(&dir, "jpg");
    let r = s.execute("file.scripts.imageProcessor", json!({"input": input, "includeSubfolders": true, "output": out})).unwrap();
    assert_eq!((r["files"].as_array().unwrap().len(), r["errors"].as_array().unwrap().len()), (4, 0), "{r}");
    assert!(std::path::Path::new(&join(&out, "sub/deeper/a.jpg")).is_file());
    // A wrong type is a parameter error, for both.
    for bad in [json!("yes"), json!(1), json!([true])] {
        for cmd in ["file.automate.batch", "file.scripts.imageProcessor"] {
            let e = s.execute(cmd, json!({"steps": [], "input": input, "includeSubfolders": bad, "output": out})).unwrap_err();
            assert!(matches!(e, EngineError::BadParams { .. }), "{cmd} {bad}: {e}");
        }
    }
    // A missing input folder is an error, not a panic.
    assert!(s.execute("file.automate.batch", json!({"steps": [], "input": join(&dir, "nope"), "includeSubfolders": true, "output": out})).is_err());
}

/// The subfolder walk stops at `MAX_SUBFOLDER_DEPTH` levels.
#[test]
fn batch_subfolder_walk_is_depth_limited() {
    let dir = tmp("batch-subfolder-depth");
    let mut deep = dir.clone();
    for level in 1..=MAX_SUBFOLDER_DEPTH + 1 {
        deep = join(&deep, "d");
        std::fs::create_dir_all(&deep).unwrap();
        if level >= MAX_SUBFOLDER_DEPTH {
            std::fs::write(join(&deep, &format!("l{level}.png")), b"x").unwrap();
        }
    }
    let found = list_images_tree(&dir, "", "test").unwrap();
    let names: Vec<_> = found.iter().map(|i| file_name(&i.path)).collect();
    assert_eq!(names, vec![format!("l{MAX_SUBFOLDER_DEPTH}.png")], "{found:?}");
    assert_eq!(found[0].rel_dir.split('/').count(), MAX_SUBFOLDER_DEPTH);
}

/// A symlink back up the tree is walked once, not forever. Skipped where the platform doesn't
/// let the test create directory symlinks (Windows without developer mode).
#[test]
fn batch_subfolder_walk_survives_symlink_loops() {
    let dir = tmp("batch-subfolder-loop");
    let sub = join(&dir, "sub");
    std::fs::create_dir_all(&sub).unwrap();
    std::fs::write(join(&dir, "a.png"), b"x").unwrap();
    std::fs::write(join(&sub, "b.png"), b"x").unwrap();
    let link = std::path::Path::new(&sub).join("loop");
    #[cfg(unix)]
    let made = std::os::unix::fs::symlink(&dir, &link);
    #[cfg(windows)]
    let made = std::os::windows::fs::symlink_dir(&dir, &link);
    #[cfg(not(any(unix, windows)))]
    let made: std::io::Result<()> = Err(std::io::Error::other("no symlinks"));
    if made.is_err() {
        eprintln!("skipped: cannot create a directory symlink here");
        return;
    }
    let found = list_images_tree(&dir, "", "test").unwrap();
    let names: Vec<_> = found.iter().map(|i| (i.rel_dir.as_str(), file_name(&i.path))).collect();
    assert_eq!(names, vec![("", "a.png".to_string()), ("sub", "b.png".to_string())]);
}

#[test]
fn batch_naming_validates_the_whole_plan_before_writing() {
    let dir = tmp("naming-invalid");
    let input = png(&dir, "source.png", 8, 8, "#ff0000");
    let mut s = Session::new();
    for (index, extra) in [
        json!({"naming": "../{name}"}),
        json!({"naming": "sub\\{name}"}),
        json!({"naming": "C:{name}"}),
        json!({"naming": "{unknown}"}),
        json!({"naming": "{name"}),
        json!({"naming": "name}"}),
        json!({"naming": 42}),
        json!({"naming": "{serial}", "startingSerial": 0}),
        json!({"naming": "{serial}", "startingSerial": 1.5}),
        json!({"naming": "{serial}", "startingSerial": "1"}),
        json!({"naming": "{serial}", "startingSerial": 4294967295_u64}),
        json!({"naming": "x".repeat(256)}),
    ]
    .into_iter()
    .enumerate()
    {
        let out = join(&dir, &format!("out-{index}"));
        let mut params = json!({"input": [input, input], "output": out, "steps": []});
        params.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        let error = s.execute("file.automate.batch", params).unwrap_err();
        assert!(matches!(error, EngineError::BadParams { .. }), "{error}");
        assert!(!std::path::Path::new(&out).exists(), "invalid naming wrote outputs: {extra}");
    }
}

#[test]
fn batch_naming_keeps_serial_positions_after_errors_and_captures_the_utc_date() {
    let dir = tmp("naming-errors");
    let input = png(&dir, "good.png", 8, 8, "#0000ff");
    let out = join(&dir, "out");
    let before = crate::analysis_cmds::now_iso();
    let mut s = Session::new();
    let result = s
        .execute(
            "file.automate.batch",
            json!({
                "input": [join(&dir, "missing.png"), input], "output": out, "steps": [],
                "naming": "{date}_{serial}_{name}", "startingSerial": 9
            }),
        )
        .unwrap();
    let after = crate::analysis_cmds::now_iso();
    assert_eq!(result["errors"].as_array().unwrap().len(), 1, "{result}");
    let files = result["files"].as_array().unwrap();
    assert_eq!(files.len(), 1, "{result}");
    let name = file_name(files[0].as_str().unwrap());
    assert!(
        name == format!("{}_010_good", &before[..10]) || name == format!("{}_010_good", &after[..10]),
        "the missing input still consumes its serial: {name}"
    );
}

#[test]
fn batch_naming_later_result_replaces_the_earlier_one() {
    let dir = tmp("naming-collision");
    let red = png(&dir, "red.png", 8, 8, "#ff0000");
    let blue = png(&dir, "blue.png", 8, 8, "#0000ff");
    let out = join(&dir, "out");
    let mut s = Session::new();
    let result = s
        .execute(
            "file.automate.batch",
            json!({
                "input": [red, blue], "output": out, "steps": [], "naming": "constant"
            }),
        )
        .unwrap();
    assert_eq!(result["files"], json!([join(&out, "constant"), join(&out, "constant")]));
    assert_eq!(result["errors"], json!([]), "{result}");
    let saved = photocraft_io::import("constant.png", &std::fs::read(join(&out, "constant")).unwrap()).unwrap().document;
    assert_eq!(photocraft_compose::render(&saved, Rect::new(1, 1, 2, 2)).px[0], [0.0, 0.0, 1.0, 1.0]);
}
#[test]
fn batch_naming_rejects_outputs_that_replace_inputs_before_processing() {
    let dir = tmp("naming-input-alias");
    let first = png(&dir, "002.png", 8, 8, "#ff0000");
    let second = png(&dir, "001.png", 8, 8, "#0000ff");
    let before = [std::fs::read(&first).unwrap(), std::fs::read(&second).unwrap()];
    let mut s = Session::new();
    // Without preflight, processing 002 first writes 001, so the next input's original pixels
    // are lost before it is read. The full plan must be rejected without writing either file.
    let error = s
        .execute(
            "file.automate.batch",
            json!({
                "input": [first, second], "output": join(&dir, "."), "steps": [], "naming": "{serial}.{ext}"
            }),
        )
        .unwrap_err();
    assert!(error.to_string().contains("input"), "{error}");
    assert_eq!(std::fs::read(&first).unwrap(), before[0]);
    assert_eq!(std::fs::read(&second).unwrap(), before[1]);
}

/// Batch takes the result at the action's recorded Save, Save As or Save a Copy, as Photoshop's
/// "Override Action 'Save As' Commands" does: a document closed after its save is written as the
/// save found it. The usual "…, Save As, Close" ending failed on every file: the Save was dropped,
/// and nothing was left open to save after the action.
#[test]
fn batch_saves_at_the_recorded_save_so_a_close_after_it_keeps_the_result() {
    let dir = tmp("batch-save-close");
    let red = png(&dir, "red.png", 8, 8, "#ff0000");
    let saved = |out: &str| photocraft_io::import("red.png", &std::fs::read(join(out, "red.png")).unwrap()).unwrap().document;
    let mut s = Session::new();
    let out = join(&dir, "close");
    let steps = json!([["image.adjustments.invert", {}], ["file.saveAs", {}], ["file.close", {}]]);
    let r = s.execute("file.automate.batch", json!({"input": [red], "output": out, "format": "png", "steps": steps})).unwrap();
    assert_eq!(r["errors"], json!([]), "{r}");
    assert_eq!(r["files"], json!([join(&out, "red.png")]));
    assert_eq!(photocraft_compose::render(&saved(&out), Rect::new(1, 1, 2, 2)).px[0], [0.0, 1.0, 1.0, 1.0], "the inverted red");

    // Save a Copy marks the result the same way, and its recorded path is never written.
    let out = join(&dir, "copy");
    let elsewhere = join(&dir, "elsewhere.png");
    let steps = json!([["image.adjustments.invert", {}], ["file.saveACopy", {"path": elsewhere}], ["file.close", {}]]);
    let r = s.execute("file.automate.batch", json!({"input": [red], "output": out, "format": "png", "steps": steps})).unwrap();
    assert_eq!(r["errors"], json!([]), "{r}");
    assert_eq!(photocraft_compose::render(&saved(&out), Rect::new(1, 1, 2, 2)).px[0], [0.0, 1.0, 1.0, 1.0]);
    assert!(!std::path::Path::new(&elsewhere).exists());

    // With no Save step the result is still saved after the action; with nothing left open then,
    // the file is an error, not a silent skip.
    let out = join(&dir, "closed");
    let r = s.execute("file.automate.batch", json!({"input": [red], "output": out, "format": "png", "steps": [["file.close", {}]]})).unwrap();
    assert_eq!(r["files"], json!([]), "{r}");
    assert!(r["errors"][0]["error"].as_str().unwrap().contains("no active document"), "{r}");
}

/// "…; Save As; Close" with the Close recorded on a clean document (no `Svng`, `"save":"ask"`):
/// Batch performs the Save As, so the document is saved when the Close asks, and every file is
/// written. The Close used to refuse the still-changed scratch document and fail each file.
#[test]
fn batch_save_as_then_close_that_asks_processes_every_file() {
    let dir = tmp("batch-save-close-ask");
    let input = vec![png(&dir, "red.png", 8, 8, "#ff0000"), png(&dir, "blue.png", 8, 8, "#0000ff")];
    let out = join(&dir, "out");
    let steps = json!([["image.adjustments.invert", {}], ["file.saveAs", {}], ["file.close", {"save": "ask"}]]);
    let r = Session::new().execute("file.automate.batch", json!({"input": input, "output": out, "format": "png", "steps": steps})).unwrap();
    assert_eq!(r["errors"], json!([]), "{r}");
    assert_eq!(r["files"].as_array().unwrap().len(), 2, "{r}");
    let red = photocraft_io::import("red.png", &std::fs::read(join(&out, "red.png")).unwrap()).unwrap().document;
    assert_eq!(photocraft_compose::render(&red, Rect::new(1, 1, 2, 2)).px[0], [0.0, 1.0, 1.0, 1.0], "the inverted red");
    // A Save a Copy doesn't save the document itself: the Close that asks still refuses it.
    let out = join(&dir, "copy");
    let steps = json!([["image.adjustments.invert", {}], ["file.saveACopy", {}], ["file.close", {"save": "ask"}]]);
    let r = Session::new().execute("file.automate.batch", json!({"input": input, "output": out, "format": "png", "steps": steps})).unwrap();
    assert_eq!(r["files"], json!([]), "{r}");
    assert_eq!(r["errors"].as_array().unwrap().len(), 2, "{r}");
}

/// The document a recorded save marked is the result, not the one left active after it closed.
/// Here the action saves and closes a second document it made, then inverts the input; the output
/// used to be the inverted input, written in place of the result without an error.
#[test]
fn batch_saves_the_document_active_at_the_save_not_the_one_left_after_it() {
    let dir = tmp("batch-save-second");
    let red = png(&dir, "red.png", 8, 8, "#ff0000");
    let out = join(&dir, "out");
    let steps = json!([
        ["file.new", {"width": 4, "height": 4, "background": "#00ff00"}],
        ["file.save", {}],
        ["file.close", {}],
        ["image.adjustments.invert", {}]
    ]);
    let r = Session::new().execute("file.automate.batch", json!({"input": [red], "output": out, "format": "png", "steps": steps})).unwrap();
    assert_eq!(r["errors"], json!([]), "{r}");
    let saved = photocraft_io::import("red.png", &std::fs::read(join(&out, "red.png")).unwrap()).unwrap().document;
    assert_eq!((saved.size.width, saved.size.height), (4, 4));
    assert_eq!(photocraft_compose::render(&saved, Rect::new(1, 1, 2, 2)).px[0], [0.0, 1.0, 0.0, 1.0]);
}

/// While the saved document is open, the steps after its save still reach the result (#3058: a
/// recorded Save then Rotate writes the rotated image). A step failing after the save writes
/// nothing, as when Batch saved only after the action.
#[test]
fn batch_applies_the_steps_after_a_save_while_its_document_is_open() {
    let dir = tmp("batch-save-then-edit");
    let red = png(&dir, "red.png", 8, 8, "#ff0000");
    let mut s = Session::new();
    let out = join(&dir, "edited");
    let steps = json!([["file.saveAs", {}], ["image.adjustments.invert", {}]]);
    let r = s.execute("file.automate.batch", json!({"input": [red], "output": out, "format": "png", "steps": steps})).unwrap();
    assert_eq!(r["errors"], json!([]), "{r}");
    let saved = photocraft_io::import("red.png", &std::fs::read(join(&out, "red.png")).unwrap()).unwrap().document;
    assert_eq!(photocraft_compose::render(&saved, Rect::new(1, 1, 2, 2)).px[0], [0.0, 1.0, 1.0, 1.0]);

    let out = join(&dir, "failed");
    let steps = json!([["image.adjustments.invert", {}], ["file.saveAs", {}], ["layer.select", {"layer": "missing"}]]);
    let r = s.execute("file.automate.batch", json!({"input": [red], "output": out, "format": "png", "steps": steps})).unwrap();
    assert_eq!(r["errors"].as_array().unwrap().len(), 1, "{r}");
    assert!(!std::path::Path::new(&join(&out, "red.png")).exists(), "{r}");
}

#[test]
fn exposure_text_near_one_second_is_not_one_over_one() {
    assert_eq!(exposure_text(0.996).as_deref(), Some("0.996 s"), "not \"1/1 s\"; three significant digits");
    assert_eq!(exposure_text(0.99).as_deref(), Some("0.99 s"));
    assert_eq!(exposure_text(1.0 / 250.0).as_deref(), Some("1/250 s"));
}

/// A folder batch picks up GIMP's gzipped `.xcf.gz`, whose last extension alone (`gz`) says nothing.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn folder_listing_recognises_gzipped_xcf_by_its_double_extension() {
    assert!(super::openable_name("/scans/page 01.xcf.gz"));
    assert!(super::openable_name("C:\\scans\\PAGE.XCF.GZ"));
    assert!(super::openable_name("page.xcfgz"));
    assert!(super::openable_name("page.PNG"));
    assert!(!super::openable_name("archive.tar.gz"));
    assert!(!super::openable_name("notes.txt"));
}

/// A folder batch picks up every extension that the codecs open: Cineon, DICOM, Amiga IFF, PCX,
/// XPM, DDS, the Netpbm family, and Photoshop templates. Not the raws whose usual files open only
/// as the embedded JPEG preview (CR3, compressed ORF and RAF, RW2 other than raw formats 3, 4 and
/// 5): a batch would process the thumbnail without a word.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn folder_listing_takes_every_format_photocraft_opens() {
    let codecs = ["a.cin", "a.dcm", "a.dicom", "a.dc3", "a.iff", "a.lbm", "a.ilbm", "a.pcx", "a.xpm", "a.dds", "a.pbm", "a.pam", "a.pfm", "a.psdt", "A.PCX"];
    let raws = ["a.dng", "IMG_0001.CR2", "a.nef", "a.nrw", "a.arw", "a.pef"];
    for name in codecs.into_iter().chain(raws) {
        assert!(super::openable_name(name), "{name}");
    }
    for f in photocraft_codecs::Format::ALL.into_iter().filter(|f| f.caps().read) {
        for ext in f.extensions() {
            assert!(super::openable_name(&format!("a.{ext}")), "{f:?} .{ext}");
        }
    }
    for name in ["a.cr3", "IMG_0001.CR3", "a.orf", "a.raf", "a.rw2", "a.srf", "a.sr2"] {
        assert!(!super::openable_name(name), "{name}");
    }
    // PhotoCraft does not read AVIF.
    assert!(!super::openable_name("a.avif"));
}

/// A batch over a folder processes PAM, PFM and DDS files instead of skipping them.
#[test]
fn batch_folder_processes_netpbm_and_dds_files() {
    let dir = tmp("batch-folder-formats");
    let src = join(&dir, "in");
    std::fs::create_dir_all(&src).unwrap();
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 8, "height": 8, "background": "#ff0000"})).unwrap();
    for name in ["a.pam", "b.pfm", "c.dds"] {
        save_doc(doc(&s), &join(&src, name), None).unwrap();
    }
    std::fs::write(join(&src, "notes.txt"), b"not an image").unwrap();
    let out = join(&dir, "out");
    let r = Session::new().execute("file.automate.batch", json!({"steps": [], "input": src, "output": out, "format": "png"})).unwrap();
    assert!(r["errors"].as_array().unwrap().is_empty(), "{r}");
    let mut files: Vec<String> = r["files"].as_array().unwrap().iter().map(|f| file_name(f.as_str().unwrap())).collect();
    files.sort();
    assert_eq!(files, ["a.png", "b.png", "c.png"], "{r}");
}

/// The Compression tag (259) of a TIFF's first directory.
fn tiff_compression_tag(b: &[u8]) -> Option<u16> {
    let le = b.get(..2)? == b"II";
    let u16_at = |i: usize| b.get(i..i + 2).map(|s| if le { u16::from_le_bytes([s[0], s[1]]) } else { u16::from_be_bytes([s[0], s[1]]) });
    let u32_at =
        |i: usize| b.get(i..i + 4).map(|s| if le { u32::from_le_bytes([s[0], s[1], s[2], s[3]]) } else { u32::from_be_bytes([s[0], s[1], s[2], s[3]]) });
    let ifd = u32_at(4)? as usize;
    (0..usize::from(u16_at(ifd)?)).map(|k| ifd + 2 + 12 * k).find(|e| u16_at(*e) == Some(259)).and_then(|e| u16_at(e + 8))
}

/// #5301: `tiffCompression` picks a TIFF's compression (TIFF Options: None, LZW, ZIP); unset,
/// none. Anything else is refused before anything is written.
#[test]
fn save_a_copy_writes_the_tiff_compression_asked_for() {
    let dir = tmp("tiff-compression");
    for (params, tag) in
        [(json!({}), 1), (json!({"tiffCompression": "none"}), 1), (json!({"tiffCompression": "lzw"}), 5), (json!({"tiffCompression": "zip"}), 8)]
    {
        let mut s = session(12, 6, 8);
        let out = join(&dir, "a.tif");
        let mut p = params.clone();
        p["path"] = json!(out);
        s.execute("file.saveACopy", p).unwrap();
        let bytes = std::fs::read(&out).unwrap();
        assert_eq!(tiff_compression_tag(&bytes), Some(tag), "{params}");
        assert_eq!(photocraft_io::import("a.tif", &bytes).unwrap().document.size, photocraft_geom::Size::new(12, 6));
    }
    let mut s = session(8, 8, 8);
    let state = |s: &Session| s.active().map(|st| (st.revision, st.path.clone(), st.is_dirty()));
    let before = state(&s);
    for bad in [json!("jpeg"), json!("LZW"), json!(5), json!(true)] {
        let out = join(&dir, "bad.tif");
        let r = s.execute("file.saveACopy", json!({"path": out, "tiffCompression": bad}));
        assert!(matches!(r, Err(EngineError::BadParams { .. })), "{bad}: {r:?}");
        assert!(!std::path::Path::new(&out).exists(), "{bad}: nothing written");
        assert_eq!(state(&s), before, "{bad}: the document is as it was");
    }
}

/// #8827: `tiffPyramid` writes reduced-resolution levels after the image and `tiffTransparency`
/// false composites a transparent document over white. Bad values are refused before anything is
/// written.
#[test]
fn save_a_copy_writes_the_tiff_pyramid_and_transparency_asked_for() {
    let dir = tmp("tiff-options");
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 200, "height": 100, "background": "transparent"})).unwrap();
    let out = join(&dir, "p.tif");
    s.execute("file.saveACopy", json!({"path": out, "tiffPyramid": true})).unwrap();
    let bytes = std::fs::read(&out).unwrap();
    let sizes: Vec<(u32, u32)> = photocraft_codecs::tiff_info(&bytes).unwrap().pages.iter().map(|p| (p.width, p.height)).collect();
    assert_eq!(sizes, [(200, 100), (100, 50), (50, 25)]);
    for (t, alpha) in [(None, true), (Some(true), true), (Some(false), false)] {
        let mut p = json!({"path": out});
        if let Some(t) = t {
            p["tiffTransparency"] = json!(t);
        }
        s.execute("file.saveACopy", p).unwrap();
        let img = photocraft_codecs::decode(&std::fs::read(&out).unwrap()).unwrap();
        assert_eq!(img.layout().has_alpha(), alpha, "{t:?}");
    }
    for (key, bad) in [("tiffPyramid", json!("yes")), ("tiffPyramid", json!(1)), ("tiffTransparency", json!("off")), ("tiffTransparency", json!([]))] {
        let out = join(&dir, "bad.tif");
        let r = s.execute("file.saveACopy", json!({"path": out, key: bad}));
        assert!(matches!(r, Err(EngineError::BadParams { .. })), "{key} {bad}: {r:?}");
        assert!(!std::path::Path::new(&out).exists(), "{key} {bad}: nothing written");
    }
}

#[test]
fn sanitize_keeps_windows_device_names_off_the_disk() {
    assert_eq!(sanitize("nul"), "nul_");
    assert_eq!(sanitize("CON"), "CON_");
    assert_eq!(sanitize("com1.v2"), "com1_.v2");
    assert_eq!(sanitize("lpt9 .final"), "lpt9 _.final");
    assert_eq!(sanitize("COM¹"), "COM¹_");
    assert_eq!(sanitize("LPT².txt"), "LPT²_.txt");
    for name in ["nul", "CON", "com1.v2", "lpt9 .final", "aux", "prn", "conin$", "clock$", "COM¹", "lpt³.x"] {
        assert!(!is_windows_device_name(&sanitize(name)), "{name} -> {}", sanitize(name));
    }
    // Names that only start like a device are files like any other.
    for name in ["console", "normal", "com10", "lpt0", "nullify", "con_", "com⁴", "com¹¹"] {
        assert_eq!(sanitize(name), name);
    }
}

#[test]
fn approve_memory_validates_parameters_and_unknown_ids() {
    let mut s = session(4, 4, 8);
    assert!(matches!(s.execute("file.approveMemory", json!({})), Err(EngineError::BadParams { .. })));
    assert!(matches!(s.execute("file.approveMemory", json!({"id": "not_a_number"})), Err(EngineError::BadParams { .. })));
    assert!(matches!(s.execute("file.approveMemory", json!({"id": 12345})), Err(EngineError::BadParams { .. })));
    assert!(
        matches!(s.execute("file.approveMemory", json!({"id": 12345, "approve": false})), Err(EngineError::BadParams { .. })),
        "unknown id on cancel must fail"
    );
    assert!(
        matches!(s.execute("file.approveMemory", json!({"id": 12345, "approve": "true"})), Err(EngineError::BadParams { .. })),
        "wrong approve type string must BadParams"
    );
    assert!(
        matches!(s.execute("file.approveMemory", json!({"id": 12345, "approve": 1})), Err(EngineError::BadParams { .. })),
        "wrong approve type int must BadParams"
    );
    assert!(
        matches!(s.execute("file.openAs", json!({"path": "a.psd", "approveHighMemory": "true"})), Err(EngineError::BadParams { .. })),
        "wrong approveHighMemory on openAs must BadParams"
    );
    assert!(
        matches!(s.execute("file.placeEmbedded", json!({"path": "a.psd", "approveHighMemory": 1})), Err(EngineError::BadParams { .. })),
        "wrong approveHighMemory on placeEmbedded must BadParams"
    );
    assert!(
        matches!(s.execute("file.approveMemory", json!({"id": 12345, "background": "true"})), Err(EngineError::BadParams { .. })),
        "wrong background type string must BadParams"
    );
    assert!(
        matches!(s.execute("file.approveMemory", json!({"id": 12345, "background": 1})), Err(EngineError::BadParams { .. })),
        "wrong background type int must BadParams"
    );
}

#[test]
fn approve_memory_cancels_and_removes_pending_preflight() {
    let mut s = Session::new();
    let id = s
        .store_pending_preflight(crate::jobs::PendingPsdPreflight {
            id: 0,
            name: "test.psd".into(),
            source: crate::jobs::OpenSource::Bytes(std::sync::Arc::new(vec![1, 2, 3])),
            fingerprint: crate::jobs::SourceFingerprint::of(&crate::jobs::OpenSource::Bytes(std::sync::Arc::new(vec![1, 2, 3]))).unwrap(),
            required: 1000,
            soft_limit: 500,
            hard_limit: 2000,
            available_ram: Some(4000),
            confirm_damage: true,
        })
        .unwrap();
    let res = s.execute("file.approveMemory", json!({"id": id, "approve": false})).unwrap();
    assert_eq!(res["approved"], false);
    assert_eq!(res["cancelled"], true);
    assert!(s.take_pending_preflight(id).is_none(), "preflight removed on cancel");
}

#[test]
fn approve_memory_refuses_when_source_changes_before_confirmation() {
    let dir = tmp("fingerprint_test");
    let path = join(&dir, "change.psd");
    std::fs::write(&path, b"initial content").unwrap();

    let mut s = Session::new();
    let src = crate::jobs::OpenSource::Path(path.clone());
    let fp = crate::jobs::SourceFingerprint::of(&src).unwrap();
    let id = s
        .store_pending_preflight(crate::jobs::PendingPsdPreflight {
            id: 0,
            name: "change.psd".into(),
            source: src,
            fingerprint: fp,
            required: 1000,
            soft_limit: 500,
            hard_limit: 2000,
            available_ram: Some(4000),
            confirm_damage: true,
        })
        .unwrap();

    // Modify file on disk
    std::fs::write(&path, b"changed content longer").unwrap();

    let res = s.execute("file.approveMemory", json!({"id": id, "approve": true, "background": false}));
    assert!(matches!(res, Err(EngineError::SourceChanged { .. })), "changed file must fail identity verification: {res:?}");
}

#[test]
fn approve_memory_refuses_when_available_ram_decreases() {
    let gb = 1u64 << 30;
    let mut s = Session::new();
    let src = crate::jobs::OpenSource::Bytes(std::sync::Arc::new(vec![0; 16]));
    let fp = crate::jobs::SourceFingerprint::of(&src).unwrap();
    let id = s
        .store_pending_preflight(crate::jobs::PendingPsdPreflight {
            id: 0,
            name: "ram_drop.psd".into(),
            source: src,
            fingerprint: fp,
            required: 8 * gb,
            soft_limit: 6 * gb,
            hard_limit: 9 * gb,
            available_ram: Some(12 * gb),
            confirm_damage: true,
        })
        .unwrap();

    // Memory drops: only 6 GiB available -> H = 4.5 GiB < 8 GiB
    let snap_low = photocraft_io::memory::MemorySnapshot { available_ram: Some(6 * gb), swap_free: None, commit_headroom: None };
    photocraft_io::memory::with_mock_memory(snap_low, || {
        let res = s.execute("file.approveMemory", json!({"id": id, "approve": true}));
        assert!(res.is_err(), "decreased memory must refuse");
    });
}

#[test]
fn pending_preflights_cap_fails_closed() {
    let mut s = Session::new();
    let src = crate::jobs::OpenSource::Bytes(std::sync::Arc::new(vec![0; 4]));
    let fp = crate::jobs::SourceFingerprint::of(&src).unwrap();
    for i in 0..crate::Session::MAX_PENDING_PREFLIGHTS {
        assert!(
            s.store_pending_preflight(crate::jobs::PendingPsdPreflight {
                id: 0,
                name: format!("file_{i}.psd"),
                source: src.clone(),
                fingerprint: fp.clone(),
                required: 1000,
                soft_limit: 500,
                hard_limit: 2000,
                available_ram: Some(4000),
                confirm_damage: true,
            })
            .is_ok()
        );
    }
    assert!(
        s.store_pending_preflight(crate::jobs::PendingPsdPreflight {
            id: 0,
            name: "overflow.psd".into(),
            source: src,
            fingerprint: fp,
            required: 1000,
            soft_limit: 500,
            hard_limit: 2000,
            available_ram: Some(4000),
            confirm_damage: true,
        })
        .is_err()
    );
}

/// A queued high-memory preflight for `source`, with fields a test can read.
fn memory_preflight(name: &str, source: crate::jobs::OpenSource) -> crate::jobs::PendingPsdPreflight {
    let fingerprint = crate::jobs::SourceFingerprint::of(&source).unwrap();
    crate::jobs::PendingPsdPreflight {
        id: 0,
        name: name.into(),
        source,
        fingerprint,
        required: 1000,
        soft_limit: 500,
        hard_limit: 2000,
        available_ram: Some(4000),
        confirm_damage: true,
    }
}

#[test]
fn pending_preflights_bound_retained_bytes_and_charge_a_shared_buffer_once() {
    let mut s = Session::new();
    let cap = 100;
    let a_src = crate::jobs::OpenSource::Bytes(std::sync::Arc::new(vec![0u8; 40]));
    let b_src = crate::jobs::OpenSource::Bytes(std::sync::Arc::new(vec![0u8; 40]));
    let a_id = s.store_pending_preflight_with_limit(memory_preflight("a.psd", a_src.clone()), cap).unwrap();
    let b_id = s.store_pending_preflight_with_limit(memory_preflight("b.psd", b_src), cap).unwrap();
    assert_eq!(s.pending_preflight_bytes().unwrap(), 80);
    // A second reference to the same allocation is already held: queued for free.
    let a_again = s.store_pending_preflight_with_limit(memory_preflight("a-again.psd", a_src), cap).unwrap();
    assert_eq!(s.pending_preflight_bytes().unwrap(), 80, "one shared buffer is charged once, not three times");
    // A different buffer would carry the total over the cap: refused, and nothing is charged.
    let c_src = crate::jobs::OpenSource::Bytes(std::sync::Arc::new(vec![0u8; 30]));
    let err = s.store_pending_preflight_with_limit(memory_preflight("c.psd", c_src.clone()), cap).expect_err("a further 30 bytes must not fit beside 80");
    assert!(err.to_string().contains("waiting for a memory approval"), "{err}");
    assert_eq!(s.pending_preflight_bytes().unwrap(), 80, "a refused preflight adds no charge");
    // Cancelling one released its buffer; the refused one now fits; clearing releases the rest.
    assert!(s.cancel_pending_preflight(a_again).is_some());
    assert_eq!(s.pending_preflight_bytes().unwrap(), 80, "the other live reference still holds the buffer");
    assert!(s.cancel_pending_preflight(a_id).is_some());
    assert_eq!(s.pending_preflight_bytes().unwrap(), 40);
    let c_id = s.store_pending_preflight_with_limit(memory_preflight("c.psd", c_src), cap).unwrap();
    assert_eq!(s.pending_preflight_bytes().unwrap(), 70);
    s.clear_pending_preflights();
    assert_eq!(s.pending_preflight_bytes().unwrap(), 0, "clear releases every held buffer");
    assert!(s.cancel_pending_preflight(b_id).is_none());
    assert!(s.cancel_pending_preflight(c_id).is_none());
}

#[test]
fn pending_preflights_charge_the_retained_allocation_not_the_visible_length() {
    let mut s = Session::new();
    let cap = 100;
    // Room for 128 bytes but one byte of contents: what stays resident is the allocation, so a
    // cap that fits the contents must still refuse it.
    let mut grown = Vec::with_capacity(128);
    grown.push(7u8);
    let held = grown.capacity() as u64;
    assert!(held >= 128, "the buffer really has the larger allocation: {held}");
    let big = std::sync::Arc::new(grown);
    let err = s
        .store_pending_preflight_with_limit(memory_preflight("big.psd", crate::jobs::OpenSource::Bytes(std::sync::Arc::clone(&big))), cap)
        .expect_err("the held capacity, not the one byte of contents, must be charged");
    assert!(err.to_string().contains("waiting for a memory approval"), "{err}");
    assert_eq!(s.pending_preflight_bytes().unwrap(), 0, "a refused preflight adds no charge");

    // A cap that admits the whole allocation accepts it, and shared clones count once.
    let id = s.store_pending_preflight_with_limit(memory_preflight("big.psd", crate::jobs::OpenSource::Bytes(std::sync::Arc::clone(&big))), held).unwrap();
    let shared =
        s.store_pending_preflight_with_limit(memory_preflight("shared.psd", crate::jobs::OpenSource::Bytes(std::sync::Arc::clone(&big))), held).unwrap();
    assert_eq!(s.pending_preflight_bytes().unwrap(), held, "one shared allocation is charged once, whatever its length");
    assert!(s.cancel_pending_preflight(shared).is_some());
    assert_eq!(s.pending_preflight_bytes().unwrap(), held, "the other shared reference still holds the allocation");
    assert!(s.cancel_pending_preflight(id).is_some());
    assert_eq!(s.pending_preflight_bytes().unwrap(), 0, "cancelling the last reference releases the allocation");
    s.clear_pending_preflights();

    // A buffer whose length was shrunk to zero still holds its allocation: the held capacity is
    // measured, so it is charged even though `len()` says nothing is there.
    let mut emptied = Vec::with_capacity(128);
    emptied.push(1u8);
    emptied.clear();
    assert!(emptied.is_empty(), "the length is zero while the allocation stays");
    let emptied = std::sync::Arc::new(emptied);
    let emptied_held = emptied.capacity() as u64;
    assert!(emptied_held >= 128, "clearing does not release the allocation: {emptied_held}");
    let emptied_id = s
        .store_pending_preflight_with_limit(memory_preflight("emptied.psd", crate::jobs::OpenSource::Bytes(emptied)), emptied_held)
        .expect("the allocation fits the cap that admits it");
    assert_eq!(s.pending_preflight_bytes().unwrap(), emptied_held, "an empty length is not zero bytes held");
    s.clear_pending_preflights();
    assert_eq!(s.pending_preflight_bytes().unwrap(), 0, "clear releases every held allocation");
    assert!(s.cancel_pending_preflight(emptied_id).is_none());
}

#[test]
fn approve_memory_is_neither_journaled_nor_recorded_in_an_action() {
    let mut s = Session::new();
    let id = s.store_pending_preflight(memory_preflight("cancel.psd", crate::jobs::OpenSource::Bytes(std::sync::Arc::new(vec![0u8; 4])))).unwrap();
    s.execute("actions.record", json!({"name": "Memory approvals"})).unwrap();
    let r = s.execute("file.approveMemory", json!({"id": id, "approve": false})).unwrap();
    assert_eq!((r["approved"].clone(), r["cancelled"].clone()), (json!(false), json!(true)));
    s.execute("actions.stop", json!({})).unwrap();
    assert!(s.journal.iter().all(|(command, _)| command != "file.approveMemory"), "one-shot approval must not become a journal entry: {:?}", s.journal);
    let action = s.actions.list.first().expect("the recorded action");
    assert!(action.steps.iter().all(|(command, _)| command != "file.approveMemory"), "one-shot approval must not replay from an action: {:?}", action.steps);
}

#[test]
fn approve_memory_releases_the_preflight_when_the_hard_limit_refuses_it() {
    let gb = 1u64 << 30;
    let mut s = Session::new();
    let preflight = crate::jobs::PendingPsdPreflight {
        required: 8 * gb,
        soft_limit: 6 * gb,
        hard_limit: 9 * gb,
        available_ram: Some(12 * gb),
        ..memory_preflight("ram_drop.psd", crate::jobs::OpenSource::Bytes(std::sync::Arc::new(vec![0u8; 16])))
    };
    let id = s.store_pending_preflight(preflight).unwrap();
    let snap_low = photocraft_io::memory::MemorySnapshot { available_ram: Some(4 * gb), swap_free: None, commit_headroom: None };
    let expected_limit = photocraft_io::memory::resolve_budget(snap_low, None).hard_limit;
    photocraft_io::memory::with_mock_memory(snap_low, || {
        let err = s.execute("file.approveMemory", json!({"id": id, "approve": true, "background": false})).unwrap_err();
        match &err {
            EngineError::PsdHardBudget { name, required, limit } => {
                assert_eq!(name, "ram_drop.psd");
                assert_eq!((*required, *limit), (8 * gb, expected_limit));
            }
            other => panic!("the hard refusal must keep its fields, got {other}"),
        }
    });
    assert!(s.take_pending_preflight(id).is_none(), "a hard refusal releases the held source");
    assert_eq!(s.pending_preflight_bytes().unwrap(), 0);
}

#[test]
fn open_as_and_place_keep_the_typed_hard_budget_fields() {
    let gb = 1u64 << 30;
    let dir = tmp("hard_budget");
    let path = join(&dir, "big.psd");
    let mut src = Session::new();
    src.execute("file.new", json!({"width": 4, "height": 4, "depth": 8})).unwrap();
    std::fs::write(&path, photocraft_io::export(&src.active().unwrap().doc, "x.psd", &Default::default()).unwrap().bytes).unwrap();
    // An estimate over the resolved hard ceiling is refused, and no approval raises the ceiling.
    // The estimate charge is a test seam so the fixture stays tiny.
    let snap = photocraft_io::memory::MemorySnapshot { available_ram: Some(4 * gb), swap_free: None, commit_headroom: None };
    let hard = photocraft_io::memory::resolve_budget(snap, None).hard_limit;
    assert!(hard < u64::MAX);
    photocraft_io::memory::with_mock_memory(snap, || {
        photocraft_io::psd_budget::with_test_estimate_charge(hard + 1, || {
            let mut s = Session::new();
            let err = s.execute("file.openAs", json!({"path": path.clone(), "approveHighMemory": true})).unwrap_err();
            match &err {
                EngineError::PsdHardBudget { name, required, limit } => {
                    assert_eq!(name, &path);
                    assert!(*required > hard && *limit == hard);
                }
                other => panic!("openAs must keep name/required/limit, got {other}"),
            }
            let mut s = session(8, 8, 8);
            let err = s.execute("file.placeEmbedded", json!({"path": path.clone(), "approveHighMemory": true})).unwrap_err();
            match &err {
                EngineError::PsdHardBudget { required, limit, .. } => assert!(*required > hard && *limit == hard),
                other => panic!("place must keep required/limit, got {other}"),
            }
        });
    });
}
