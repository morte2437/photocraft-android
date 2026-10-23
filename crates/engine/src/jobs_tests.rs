use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::json;

use super::*;
use crate::Session;

/// A document `w`×`h` with a textured pixel layer (so blurs change every pixel).
fn session(w: u32, h: u32) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": w, "height": h})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.edit("pattern", |doc, active| {
        let surf = doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap();
        let r = photocraft_geom::Rect::new(0, 0, w as i32, h as i32);
        let mut px = Vec::with_capacity((w * h * 4) as usize);
        for y in 0..h {
            for x in 0..w {
                let v = ((x * 7 + y * 13) % 31) as f32 / 30.0;
                px.extend_from_slice(&[v, 1.0 - v, ((x / 5 + y / 3) % 2) as f32, 1.0]);
            }
        }
        surf.write_region(r, &px);
        Ok(())
    })
    .unwrap();
    s
}

fn pixels(s: &Session) -> Vec<f32> {
    let d = s.active().unwrap();
    let l = d.doc.layer(d.active_layer.unwrap()).unwrap();
    l.surface().unwrap().read_region(d.doc.bounds())
}

/// Poll until job `id` ends (or 60 s pass); returns its event.
fn wait_event(s: &mut Session, id: JobId) -> JobEvent {
    let t = Instant::now();
    loop {
        if let Some(e) = s.poll_jobs().into_iter().find(|e| e.id == id) {
            return e;
        }
        assert!(t.elapsed() < Duration::from_secs(60), "job {id:?} did not finish");
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn job(r: Started) -> JobId {
    match r {
        Started::Job(id) => id,
        Started::Done(v) => panic!("expected a background job, ran inline: {v}"),
    }
}

#[test]
fn a_job_applies_as_one_undo_step_and_matches_the_inline_result() {
    let mut inline = session(300, 200);
    inline.execute("filter.blur.gaussianBlur", json!({"radius": 4})).unwrap();

    let mut s = session(300, 200);
    let before = pixels(&s);
    let steps = s.active().unwrap().history.past_len();
    let journal = s.journal.len();
    let id = job(s.start("filter.blur.gaussianBlur", json!({"radius": 4})).unwrap());
    // The canvas shows the pre-job state until the job is applied.
    assert_eq!(pixels(&s), before);
    let e = wait_event(&mut s, id);
    let JobOutcome::Done(v) = &e.outcome else { panic!("{e:?}") };
    assert!(v["filter"].is_object(), "{v}");
    assert_eq!(e.command, "filter.blur.gaussianBlur");
    assert_eq!(s.active().unwrap().history.past_len(), steps + 1, "one undo step");
    assert_eq!(s.journal.len(), journal + 1, "journaled like execute");
    assert_eq!(s.journal.last().unwrap().0, "filter.blur.gaussianBlur");
    assert_eq!(pixels(&s), pixels(&inline), "same pixels as the synchronous command");
    assert!(s.undo());
    assert_eq!(pixels(&s), before);
    assert!(s.jobs().is_empty());
}

#[test]
fn free_transform_runs_as_a_job_with_the_inline_result() {
    let quad = json!({"quad": [[20, 10], [280, 30], [260, 190], [40, 170]]});
    let mut inline = session(300, 200);
    inline.execute("edit.transform", quad.clone()).unwrap();

    let mut s = session(300, 200);
    let steps = s.active().unwrap().history.past_len();
    let id = job(s.start("edit.transform", quad).unwrap());
    let e = wait_event(&mut s, id);
    assert!(matches!(e.outcome, JobOutcome::Done(_)), "{e:?}");
    assert_eq!(e.label, "Free Transform");
    assert_eq!(s.active().unwrap().history.past_len(), steps + 1, "one undo step");
    assert_eq!(pixels(&s), pixels(&inline), "same pixels as the synchronous command");
}

#[test]
fn warp_runs_as_a_job_with_the_inline_result() {
    let arc = json!({"style": "arc", "bend": 30});
    let mut inline = session(300, 200);
    inline.execute("edit.transform.warp", arc.clone()).unwrap();

    let mut s = session(300, 200);
    let steps = s.active().unwrap().history.past_len();
    let id = job(s.start("edit.transform.warp", arc).unwrap());
    let e = wait_event(&mut s, id);
    assert!(matches!(e.outcome, JobOutcome::Done(_)), "{e:?}");
    assert_eq!(e.label, "Warp");
    assert_eq!(s.active().unwrap().history.past_len(), steps + 1, "one undo step");
    assert_eq!(pixels(&s), pixels(&inline), "same pixels as the synchronous command");
}

#[test]
fn a_cancelled_warp_leaves_the_document_unchanged() {
    let mut s = session(1600, 1200);
    let doc_before = s.active().unwrap().doc.clone();
    let steps = s.active().unwrap().history.past_len();
    let id = job(s.start("edit.transform.warp", json!({"style": "flag", "bend": 40})).unwrap());
    assert!(s.cancel_job(id));
    assert_eq!(wait_event(&mut s, id).outcome, JobOutcome::Cancelled);
    s.join_cancelled_jobs();
    let st = s.active().unwrap();
    assert!(Arc::ptr_eq(&st.doc, &doc_before), "the document snapshot was not replaced");
    assert_eq!(st.history.past_len(), steps);
}

#[test]
fn cancel_leaves_the_document_byte_identical() {
    let mut s = session(1600, 1200);
    let doc_before = s.active().unwrap().doc.clone();
    let before = pixels(&s);
    let (rev, steps) = (s.active().unwrap().revision, s.active().unwrap().history.past_len());
    let id = job(s.start("filter.blur.gaussianBlur", json!({"radius": 60})).unwrap());
    assert!(s.cancel_job(id));
    let e = wait_event(&mut s, id);
    assert_eq!(e.outcome, JobOutcome::Cancelled);
    s.join_cancelled_jobs();
    // Nothing arrives later either.
    assert!(s.poll_jobs().is_empty());
    let st = s.active().unwrap();
    assert!(Arc::ptr_eq(&st.doc, &doc_before), "the document snapshot was not replaced");
    assert_eq!((st.revision, st.history.past_len()), (rev, steps));
    assert_eq!(pixels(&s), before);
    assert!(matches!(s.wait_job(id), Err(EngineError::Cancelled)));
}

#[test]
fn a_panic_in_the_worker_becomes_an_error_and_the_document_is_unchanged() {
    let mut s = session(64, 64);
    let doc_before = s.active().unwrap().doc.clone();
    let id = job(s
        .start_job(
            "test.panics",
            json!({}),
            "Panicking job",
            true,
            |_ctx| -> Result<()> {
                let v: Vec<u8> = Vec::new();
                // An out-of-bounds index: a panic inside the worker.
                #[allow(clippy::indexing_slicing)]
                let _ = v[usize::MAX / 2];
                Ok(())
            },
            |_s, ()| Ok(json!(null)),
        )
        .unwrap());
    let e = wait_event(&mut s, id);
    let JobOutcome::Failed(msg) = &e.outcome else { panic!("{e:?}") };
    assert!(msg.contains("internal error"), "{msg}");
    assert!(Arc::ptr_eq(&s.active().unwrap().doc, &doc_before));
    // The session still works.
    s.execute("filter.blur.gaussianBlur", json!({"radius": 1})).unwrap();
}

#[test]
fn a_panic_in_a_command_becomes_an_error_and_the_document_is_unchanged() {
    // REL-3: the last-resort guard in `jobs::dispatch` catches a panic that escapes `run`,
    // reports it as an error and leaves the document (and its history) untouched.
    let mut s = session(64, 64);
    let doc_before = s.active().unwrap().doc.clone();
    let (rev, steps) = (s.active().unwrap().revision, s.active().unwrap().history.past_len());
    let msg = s.execute("test.panic", json!({})).unwrap_err().to_string();
    assert!(msg.contains("internal error"), "{msg}");
    assert!(msg.contains("test.panic"), "{msg}");
    let st = s.active().unwrap();
    assert!(Arc::ptr_eq(&st.doc, &doc_before), "the document snapshot was not replaced");
    assert_eq!((st.revision, st.history.past_len()), (rev, steps));
    // The session still works.
    s.execute("filter.blur.gaussianBlur", json!({"radius": 1})).unwrap();
}

#[test]
fn progress_is_monotonic_and_reaches_one() {
    let ctx = JobCtx::new();
    ctx.progress(0.5, "half");
    ctx.progress(0.3, "");
    assert_eq!(ctx.fraction(), 0.5, "progress never goes backwards");
    assert_eq!(ctx.message(), "half");
    ctx.progress(f32::NAN, "");
    ctx.progress(7.0, "");
    assert_eq!(ctx.fraction(), 1.0);

    let mut s = session(2000, 1500);
    let id = job(s.start("filter.blur.gaussianBlur", json!({"radius": 30})).unwrap());
    let mut seen = Vec::new();
    let e = loop {
        if let Some(j) = s.jobs().into_iter().find(|j| j.id == id) {
            seen.push(j.progress);
            assert_eq!(j.label, "Gaussian Blur");
            assert_eq!(j.state, "running");
        }
        if let Some(e) = s.poll_jobs().into_iter().find(|e| e.id == id) {
            break e;
        }
        std::thread::sleep(Duration::from_millis(1));
    };
    assert!(matches!(e.outcome, JobOutcome::Done(_)), "{e:?}");
    assert!(seen.windows(2).all(|w| w[0] <= w[1]), "monotonic: {seen:?}");
    let info = s.jobs_with_recent().into_iter().find(|j| j.id == id).unwrap();
    assert_eq!((info.state, info.progress), ("done", 1.0));
}

#[test]
fn a_preset_filter_job_is_named_after_the_preset() {
    // #528: the job (progress UI) and its history step say "Blur More", not "Gaussian Blur".
    let mut s = session(300, 200);
    let id = job(s.start("filter.blur.blurMore", json!({})).unwrap());
    let e = wait_event(&mut s, id);
    assert!(matches!(e.outcome, JobOutcome::Done(_)), "{e:?}");
    assert_eq!(e.label, "Blur More");
    assert_eq!(s.active().unwrap().history.undo_label(), Some("Blur More"));
}

/// A fake document job that runs until `gate` opens (or it is cancelled), then inverts nothing
/// but records one undo step named "Gated Job".
pub(super) fn gated_job(s: &mut Session, gate: &Arc<std::sync::atomic::AtomicBool>) -> JobId {
    let gate = gate.clone();
    job(s
        .start_job(
            "test.gated",
            json!({}),
            "Gated Job",
            true,
            move |ctx| {
                while !gate.load(std::sync::atomic::Ordering::Relaxed) {
                    ctx.check()?;
                    ctx.progress(0.5, "waiting");
                    std::thread::sleep(Duration::from_millis(1));
                }
                Ok(())
            },
            |s, ()| {
                s.edit("Gated Job", |_, _| Ok(()))?;
                Ok(json!({"gated": true}))
            },
        )
        .unwrap())
}

#[test]
fn a_running_job_locks_its_document_but_not_others() {
    let mut s = session(64, 64);
    let gate = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let id = gated_job(&mut s, &gate);
    // Edits of the busy document are disabled with a message naming the job.
    let err = s.execute("layer.new.layer", json!({})).unwrap_err();
    assert!(matches!(&err, EngineError::Disabled(_, why) if why.contains("Gated Job")), "{err}");
    assert!(!s.is_enabled("layer.new.layer"));
    assert!(s.disabled_reason("filter.blur.gaussianBlur").unwrap().contains("still running"));
    assert!(!s.undo(), "undo waits for the job");
    // Queries and document-independent commands still run.
    s.execute("jobs.list", json!({})).unwrap();
    // A new document is free to edit while the first one is busy.
    s.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    let r = s.execute("jobs.list", json!({})).unwrap();
    assert_eq!(r["jobs"][0]["id"], json!(id.0));
    // The result applies to its own document, and the active document stays the new one.
    gate.store(true, std::sync::atomic::Ordering::Relaxed);
    let e = wait_event(&mut s, id);
    assert!(matches!(e.outcome, JobOutcome::Done(_)), "{e:?}");
    assert_eq!(s.active_index(), Some(1));
    assert_eq!(s.documents()[0].history.past_len(), 3);
}

#[test]
fn jobs_list_puts_running_jobs_before_ended_ones() {
    let mut s = session(64, 64);
    let gate = Arc::new(std::sync::atomic::AtomicBool::new(true));
    let done = gated_job(&mut s, &gate);
    assert!(matches!(wait_event(&mut s, done).outcome, JobOutcome::Done(_)));
    // A second document, so the next job doesn't wait on the first one's.
    s.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
    let gate = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let running = gated_job(&mut s, &gate);
    let r = s.execute("jobs.list", json!({})).unwrap();
    let listed: Vec<_> = r["jobs"].as_array().unwrap().iter().map(|j| (j["id"].clone(), j["state"].clone())).collect();
    assert_eq!(listed, [(json!(running.0), json!("running")), (json!(done.0), json!("done"))], "{r}");
    gate.store(true, std::sync::atomic::Ordering::Relaxed);
    wait_event(&mut s, running);
}

#[test]
fn background_job_finishes_bookkeeping_on_edited_document_after_tab_switch() {
    let mut s = session(64, 64);
    let edited_doc = s.active().unwrap().doc.id;
    let gate = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let worker_gate = Arc::clone(&gate);
    // Hold a fadeable filter job until another document is the active tab.
    // A deterministic gate avoids relying on the filter being sufficiently slow.
    let id = job(s
        .start_job(
            "filter.blur.gaussianBlur",
            json!({"radius": 2}),
            "Deferred Blur",
            true,
            move |ctx| {
                while !worker_gate.load(std::sync::atomic::Ordering::Relaxed) {
                    ctx.check()?;
                    std::thread::sleep(Duration::from_millis(1));
                }
                Ok(())
            },
            |s, ()| {
                s.edit("Deferred Blur", |doc, active| {
                    let layer = active.ok_or_else(|| EngineError::Other("no active layer".into()))?;
                    let surface = doc.layer_mut(layer).ok_or(EngineError::NoLayer(layer))?.surface_mut().ok_or(EngineError::NoLayer(layer))?;
                    surface.fill_rect(photocraft_geom::Rect::new(0, 0, 1, 1), &[0.4, 0.3, 0.2, 1.0]);
                    Ok(())
                })?;
                Ok(json!({"filter": {"radius": 2}}))
            },
        )
        .unwrap());

    s.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    let viewed_doc = s.active().unwrap().doc.id;
    let viewed_revision = s.active().unwrap().revision;
    assert_eq!(s.active_index(), Some(1));

    gate.store(true, std::sync::atomic::Ordering::Relaxed);
    let event = wait_event(&mut s, id);
    assert!(matches!(event.outcome, JobOutcome::Done(_)), "{event:?}");
    assert_eq!(s.active_index(), Some(1), "the user stays on the selected document");
    assert_eq!(s.documents()[1].revision, viewed_revision, "the viewed document is unchanged");
    assert_eq!(s.documents()[1].doc.id, viewed_doc);
    assert_eq!(s.edit_state.fade.as_ref().map(|fade| fade.doc), Some(edited_doc), "Fade belongs to the edited document");
    assert!(!s.is_enabled("edit.fade"), "Fade must not target the viewed document");

    assert!(s.set_active(0));
    assert!(s.is_enabled("edit.fade"), "Fade must be available on the document that was edited");
}

/// #7382: a background filter on a targeted alpha channel or the Quick Mask, finished while another
/// document is shown, leaves Fade on that channel of the edited document: at 0 % it restores the
/// channel exactly and leaves the layer alone.
#[test]
fn background_filter_on_a_channel_fades_that_channel_after_a_tab_switch() {
    use crate::edit_menu_cmds::FadeTarget;
    for quick_mask in [false, true] {
        let mut s = session(32, 24);
        let edited_doc = s.active().unwrap().doc.id;
        if quick_mask {
            s.execute("select.editInQuickMaskMode", json!({"on": true})).unwrap();
        } else {
            s.execute("channel.new", json!({"fill": "black"})).unwrap();
            s.execute("channel.target", json!({"channel": 0})).unwrap();
        }
        // A pattern for the blur to change.
        s.edit("pattern", |doc, _| {
            let c = if quick_mask { doc.quick_mask.as_mut() } else { doc.channels.first_mut() };
            let c = c.ok_or_else(|| EngineError::Other("no channel".into()))?;
            c.surface.fill_rect(photocraft_geom::Rect::new(0, 0, 32, 24), &[0.0]);
            c.surface.fill_rect(photocraft_geom::Rect::new(8, 6, 20, 16), &[1.0]);
            Ok(())
        })
        .unwrap();
        let channel = |s: &Session, d: usize| {
            let doc = &s.documents()[d].doc;
            let c = if quick_mask { doc.quick_mask.as_ref() } else { doc.channels.first() };
            let c = c.unwrap();
            (0..24).flat_map(|y| (0..32).map(move |x| (x, y))).map(|(x, y)| c.surface.pixel(x, y)[0]).collect::<Vec<f32>>()
        };
        let layer = |s: &Session| {
            let st = &s.documents()[0];
            st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap().pixel(5, 5)
        };
        let (pattern, pixels) = (channel(&s, 0), layer(&s));
        let id = job(s.start("filter.blur.gaussianBlur", json!({"radius": 2})).unwrap());
        s.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
        let event = wait_event(&mut s, id);
        assert!(matches!(event.outcome, JobOutcome::Done(_)), "{event:?}");
        assert_ne!(channel(&s, 0), pattern, "quick mask {quick_mask}: the blur changed the channel");
        let fade = s.edit_state.fade.clone().unwrap();
        assert_eq!(fade.doc, edited_doc);
        assert_eq!(fade.target, if quick_mask { FadeTarget::QuickMask } else { FadeTarget::Channel(0) });
        assert!(s.set_active(0));
        s.execute("edit.fade", json!({"opacity": 0})).unwrap();
        assert_eq!(channel(&s, 0), pattern, "quick mask {quick_mask}: Fade at 0 % restores the channel");
        assert_eq!(layer(&s), pixels, "quick mask {quick_mask}: the layer is left alone");
    }
}

#[test]
fn closing_the_document_cancels_its_job() {
    let mut s = session(1600, 1200);
    let id = job(s.start("filter.blur.gaussianBlur", json!({"radius": 80})).unwrap());
    s.close(0);
    let e = wait_event(&mut s, id);
    assert_eq!(e.outcome, JobOutcome::Cancelled);
}

#[test]
fn jobs_cancel_command_and_bad_params() {
    let mut s = session(1600, 1200);
    let id = job(s.start("filter.blur.gaussianBlur", json!({"radius": 80})).unwrap());
    assert!(s.execute("jobs.cancel", json!({"job": "x"})).is_err());
    assert!(s.execute("jobs.cancel", json!({"job": 999})).is_err());
    s.execute("jobs.cancel", json!({"job": id.0})).unwrap();
    assert_eq!(wait_event(&mut s, id).outcome, JobOutcome::Cancelled);
    assert_eq!(s.execute("jobs.cancel", json!({})).unwrap()["cancelled"], 0);
}

#[test]
fn non_job_commands_still_finish_inline_with_start() {
    let mut s = session(16, 16);
    let r = s.start("layer.new.layer", json!({})).unwrap();
    assert!(matches!(r, Started::Done(_)), "{r:?}");
    // Errors are reported as before.
    assert!(matches!(s.start("no.such.command", json!({})), Err(EngineError::UnknownCommand(_))));
}

#[test]
fn content_aware_fill_and_scale_run_as_jobs() {
    let mut inline = session(160, 120);
    let mut s = session(160, 120);
    for t in [&mut inline, &mut s] {
        t.execute("select.rect", json!({"x": 60, "y": 40, "width": 30, "height": 30})).unwrap();
    }
    inline.execute("edit.contentAwareFill", json!({})).unwrap();
    let id = job(s.start("edit.contentAwareFill", json!({})).unwrap());
    let e = wait_event(&mut s, id);
    assert!(matches!(e.outcome, JobOutcome::Done(_)), "{e:?}");
    assert_eq!(pixels(&s), pixels(&inline), "same fill as the synchronous command");

    s.execute("select.deselect", json!({})).unwrap();
    let id = job(s.start("edit.contentAwareScale", json!({"width": 120})).unwrap());
    let e = wait_event(&mut s, id);
    let JobOutcome::Done(v) = &e.outcome else { panic!("{e:?}") };
    assert_eq!(v["to"][0], 120);
}

#[test]
fn smart_filter_edits_run_as_jobs() {
    let mut inline = session(160, 120);
    let mut s = session(160, 120);
    for t in [&mut inline, &mut s] {
        t.execute("filter.convertForSmartFilters", json!({})).unwrap();
        t.execute("filter.blur.gaussianBlur", json!({"radius": 3})).unwrap();
        t.execute("filter.blur.motionBlur", json!({"angle": 45, "distance": 8})).unwrap();
    }
    inline.execute("layer.smartFilter.setVisible", json!({"index": 0})).unwrap();
    let before = pixels(&s);
    let steps = s.active().unwrap().history.past_len();
    let id = job(s.start("layer.smartFilter.setVisible", json!({"index": 0})).unwrap());
    let e = wait_event(&mut s, id);
    assert!(matches!(e.outcome, JobOutcome::Done(_)), "{e:?}");
    assert_eq!(s.active().unwrap().history.past_len(), steps + 1, "one undo step");
    assert_eq!(pixels(&s), pixels(&inline), "same pixels as the synchronous command");
    assert!(s.undo());
    assert_eq!(pixels(&s), before);
}

#[test]
fn content_aware_move_runs_as_a_job() {
    let mut inline = session(160, 120);
    let mut s = session(160, 120);
    for t in [&mut inline, &mut s] {
        t.execute("select.rect", json!({"x": 20, "y": 40, "width": 30, "height": 30})).unwrap();
    }
    let p = json!({"offset": [90, 10], "structure": 3, "color": 4});
    inline.execute("paint.contentAwareMove", p.clone()).unwrap();
    let before = s.active().unwrap().doc.clone();
    let id = job(s.start("paint.contentAwareMove", p.clone()).unwrap());
    let e = wait_event(&mut s, id);
    let JobOutcome::Done(v) = &e.outcome else { panic!("{e:?}") };
    assert_eq!(v["offset"], json!([90, 10]));
    assert_eq!(pixels(&s), pixels(&inline), "same result as the synchronous command");
    let sel = |s: &Session| s.active().unwrap().doc.selection.as_ref().unwrap().read_region(s.active().unwrap().doc.bounds());
    assert_eq!(sel(&s), sel(&inline), "the selection moved the same way");
    // Cancelled at once: the document stays as it was.
    s.undo();
    let id = job(s.start("paint.contentAwareMove", p).unwrap());
    assert!(s.cancel_job(id));
    s.join_cancelled_jobs();
    assert_eq!(wait_event(&mut s, id).outcome, JobOutcome::Cancelled);
    assert_eq!(pixels(&s), before.layer(s.active().unwrap().active_layer.unwrap()).unwrap().surface().unwrap().read_region(before.bounds()));
}

#[test]
fn remove_runs_as_a_job() {
    let mut inline = session(160, 120);
    let mut s = session(160, 120);
    let p = json!({"points": [[60, 50], [100, 70]], "size": 24});
    inline.execute("paint.remove", p.clone()).unwrap();
    let before = pixels(&s);
    let id = job(s.start("paint.remove", p.clone()).unwrap());
    let e = wait_event(&mut s, id);
    let JobOutcome::Done(v) = &e.outcome else { panic!("{e:?}") };
    assert!(v["damage"].is_array(), "{v}");
    assert_eq!(pixels(&s), pixels(&inline), "same result as the synchronous command");
    // Cancelled at once: the document stays as it was.
    s.undo();
    let id = job(s.start("paint.remove", p).unwrap());
    assert!(s.cancel_job(id));
    s.join_cancelled_jobs();
    assert_eq!(wait_event(&mut s, id).outcome, JobOutcome::Cancelled);
    assert_eq!(pixels(&s), before);
}

#[test]
fn open_runs_as_a_job_and_cancel_adds_nothing() {
    let mut src = session(40, 30);
    let doc = (*src.active().unwrap().doc).clone();
    let bytes = photocraft_io::export(&doc, "x.psd", &Default::default()).unwrap().bytes;
    src.close(0);
    let mut s = Session::new();
    let id = job(s.start_open("x.psd", OpenSource::Bytes(Arc::new(bytes.clone()))).unwrap());
    let e = wait_event(&mut s, id);
    let JobOutcome::Done(v) = &e.outcome else { panic!("{e:?}") };
    assert_eq!(e.command, OPEN_JOB);
    assert_eq!(v["document"], 0);
    assert_eq!(s.documents().len(), 1);
    assert_eq!(s.documents()[0].doc.size.width, 40);

    let id = job(s.start_open("y.psd", OpenSource::Bytes(Arc::new(bytes))).unwrap());
    s.cancel_job(id);
    assert_eq!(wait_event(&mut s, id).outcome, JobOutcome::Cancelled);
    assert_eq!(s.documents().len(), 1, "a cancelled open adds no document");

    let id = job(s.start_open("missing.psd", OpenSource::Path("/no/such/file.psd".into())).unwrap());
    assert!(matches!(wait_event(&mut s, id).outcome, JobOutcome::Failed(_)));
}

/// Ask When Opening: a file whose profile doesn't match is decoded but not added; the shell takes
/// it once the prompt is answered. Without Ask it opens as usual.
#[test]
fn an_open_that_asks_about_its_profile_holds_the_document_back() {
    let mut src = session(8, 8);
    let mut doc = (*src.active().unwrap().doc).clone();
    doc.icc_profile = Some(photocraft_cms::Builtin::from_id("adobe-rgb-compat").unwrap().profile().to_bytes());
    let bytes = Arc::new(photocraft_io::export(&doc, "x.png", &Default::default()).unwrap().bytes);
    src.close(0);
    let mut s = Session::new();
    s.execute("edit.colorSettings", json!({"askOnMismatch": true, "policyRgb": "preserve"})).unwrap();
    let id = job(s.start_open("x.png", OpenSource::Bytes(Arc::clone(&bytes))).unwrap());
    let JobOutcome::Done(v) = wait_event(&mut s, id).outcome else { panic!() };
    assert!(s.documents().is_empty(), "held back, not opened");
    assert_eq!((v["color"]["mismatch"].as_bool(), v["color"]["ask"].as_bool(), v["color"]["action"].as_str()), (Some(true), Some(true), Some("preserve")));
    let held = v["held"].as_u64().unwrap();
    let (taken, _) = s.take_held_open(held).expect("the held document");
    assert!(taken.icc_profile.is_some());
    assert!(s.take_held_open(held).is_none(), "taken once");

    s.execute("edit.colorSettings", json!({"askOnMismatch": false})).unwrap();
    let id = job(s.start_open("x.png", OpenSource::Bytes(bytes)).unwrap());
    let JobOutcome::Done(v) = wait_event(&mut s, id).outcome else { panic!() };
    assert!(v.get("held").is_none());
    assert_eq!(s.documents().len(), 1);
}

#[test]
fn warm_layer_bounds_visits_every_layer_and_can_be_cancelled() {
    let mut s = session(40, 30);
    s.execute("layer.groupLayers", json!({})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    let doc = s.active().unwrap().doc.clone();
    fn count(layers: &[photocraft_doc::Layer]) -> usize {
        layers.iter().map(|l| 1 + l.children().map_or(0, count)).sum()
    }
    assert!(doc.layers.iter().any(|l| l.children().is_some_and(|c| !c.is_empty())), "a group with a layer in it");
    let mut checks = 0;
    warm_layer_bounds(&doc, || {
        checks += 1;
        Ok(())
    })
    .unwrap();
    assert_eq!(checks, count(&doc.layers), "one check per layer, nested ones included");
    let mut checks = 0;
    let r = warm_layer_bounds(&doc, || {
        checks += 1;
        if checks > 1 { Err(EngineError::Cancelled) } else { Ok(()) }
    });
    assert!(matches!(r, Err(EngineError::Cancelled)));
    assert_eq!(checks, 2, "stops at the first failed check");
}

#[test]
fn wait_job_blocks_until_applied() {
    let mut s = session(400, 300);
    let id = job(s.start("filter.blur.gaussianBlur", json!({"radius": 3})).unwrap());
    let v = s.wait_job(id).unwrap();
    assert!(v["filter"].is_object());
    assert_eq!(s.active().unwrap().history.past_len(), 3);
    // Waiting again reports the recorded result.
    assert_eq!(s.wait_job(id).unwrap(), v);
    assert!(s.wait_job(JobId(12345)).is_err());
}

#[test]
fn params_that_are_not_an_object_are_rejected_before_running() {
    let mut s = session(40, 30);
    let before = s.active().unwrap().history.past_len();
    for id in ["image.adjustments.invert", "filter.blur.gaussianBlur", "layer.new.layer"] {
        for p in [json!([3]), json!("x"), json!(5), json!(true)] {
            let e = s.execute(id, p.clone()).unwrap_err().to_string();
            assert!(e.contains(id) && e.contains("must be a JSON object"), "{id} {p}: {e}");
            assert!(s.start(id, p).is_err(), "{id}: background start must reject too");
        }
    }
    assert_eq!(s.active().unwrap().history.past_len(), before, "nothing ran");
    s.execute("image.adjustments.invert", json!({})).unwrap();
    s.execute("image.adjustments.invert", Value::Null).unwrap();
    assert_eq!(s.active().unwrap().history.past_len(), before + 2);
}

/// Cancel latency on a 24 MP document (6000×4000): from `cancel_job` until the worker thread
/// has exited. Release only: debug builds are ~20× slower per tile.
#[test]
// Wall-clock timing depends on machine load (the release corpus job runs every test), so it is
// opt-in; perf_scenarios tracks the same numbers as P25/P49.
#[ignore = "timing: cargo test --release -p photocraft-engine --lib cancel_takes -- --ignored"]
fn cancel_takes_effect_within_200_ms_on_24_mp() {
    for (cmd, params, select) in [("filter.blur.gaussianBlur", json!({"radius": 50}), false), ("edit.contentAwareFill", json!({}), true)] {
        let mut s = session(6000, 4000);
        if select {
            s.execute("select.rect", json!({"x": 2000, "y": 1500, "width": 1200, "height": 900})).unwrap();
        }
        let doc_before = s.active().unwrap().doc.clone();
        // Cancel at several points of the run (early, middle, late stages).
        let mut worst: f64 = 0.0;
        for after_ms in [20, 150, 400, 600, 800, 1000, 1300] {
            let id = job(s.start(cmd, params.clone()).unwrap());
            std::thread::sleep(Duration::from_millis(after_ms));
            if s.jobs().iter().all(|j| j.id != id) || s.jobs().iter().any(|j| j.id == id && j.progress >= 1.0) {
                let _ = wait_event(&mut s, id);
                s.undo();
                continue;
            }
            let at = s.jobs().iter().find(|j| j.id == id).map(|j| j.progress).unwrap_or(0.0);
            let t = Instant::now();
            assert!(s.cancel_job(id));
            s.join_cancelled_jobs();
            let ms = t.elapsed().as_secs_f64() * 1000.0;
            eprintln!("{cmd}: cancelled after {after_ms} ms (at {:.0} %), latency {ms:.1} ms", at * 100.0);
            worst = worst.max(ms);
            assert_eq!(wait_event(&mut s, id).outcome, JobOutcome::Cancelled);
            assert!(Arc::ptr_eq(&s.active().unwrap().doc, &doc_before));
        }
        eprintln!("{cmd}: worst cancel latency {worst:.1} ms");
        assert!(worst < 200.0, "{cmd}: cancel took {worst:.1} ms");
    }
}

#[test]
fn an_inline_job_ctx_lets_another_thread_cancel_inline_work() {
    // A live filter preview runs the filter inline in a private session; when the dialog's
    // parameters change, the stale computation is cancelled through this context.
    let mut s = session(96, 64);
    let before = pixels(&s);
    let revision = s.active().unwrap().revision;
    let ctx = JobCtx::new();
    ctx.cancel();
    s.set_inline_job_ctx(Some(ctx.clone()));
    for (id, p) in [("filter.blur.gaussianBlur", json!({"radius": 40})), ("filter.blur.motionBlur", json!({"distance": 40}))] {
        assert!(matches!(s.execute(id, p), Err(EngineError::Cancelled)), "{id}");
        assert_eq!(pixels(&s), before, "{id}: the document is unchanged");
        assert_eq!(s.active().unwrap().revision, revision, "{id}: no edit recorded");
    }
    // Not cancelled: the same session runs as before.
    s.set_inline_job_ctx(Some(JobCtx::new()));
    s.execute("filter.blur.gaussianBlur", json!({"radius": 4})).unwrap();
    assert_ne!(pixels(&s), before);
    s.set_inline_job_ctx(None);
    s.execute("filter.blur.gaussianBlur", json!({"radius": 2})).unwrap();
}

/// The command ids an action holds.
fn action_steps(s: &mut Session, name: &str) -> Vec<String> {
    let a = s.execute("actions.get", json!({"action": name})).unwrap();
    a["steps"].as_array().unwrap().iter().map(|step| step[0].as_str().unwrap().to_string()).collect()
}

/// A job's result is recorded only by the recording it was started in: one started later keeps
/// its own steps around it but doesn't pick up the older edit (#7289).
#[test]
fn a_job_lands_in_the_recording_it_was_started_in_only() {
    // Started while recording A, applied after A stopped and B started.
    let mut s = session(40, 30);
    s.execute("actions.record", json!({"name": "A"})).unwrap();
    let id = job(s.start("filter.blur.gaussianBlur", json!({"radius": 1})).unwrap());
    s.execute("actions.stop", json!({})).unwrap();
    s.execute("actions.record", json!({"name": "B"})).unwrap();
    // B's own steps before and after the result lands stay in B (a new document, so the job's
    // document is unchanged and its result applies).
    s.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
    s.wait_job(id).unwrap();
    s.execute("image.adjustments.desaturate", json!({})).unwrap();
    s.execute("actions.stop", json!({})).unwrap();
    assert!(action_steps(&mut s, "A").is_empty());
    assert_eq!(action_steps(&mut s, "B"), ["file.new", "image.adjustments.desaturate"]);

    // Started before any recording, applied while B records.
    let mut s = session(40, 30);
    let id = job(s.start("filter.blur.gaussianBlur", json!({"radius": 1})).unwrap());
    s.execute("actions.record", json!({"name": "B"})).unwrap();
    s.wait_job(id).unwrap();
    s.execute("actions.stop", json!({})).unwrap();
    assert!(action_steps(&mut s, "B").is_empty());

    // Started and applied while A records: A has it, as before.
    let mut s = session(40, 30);
    s.execute("actions.record", json!({"name": "A"})).unwrap();
    let id = job(s.start("filter.blur.gaussianBlur", json!({"radius": 1})).unwrap());
    s.wait_job(id).unwrap();
    s.execute("actions.stop", json!({})).unwrap();
    assert_eq!(action_steps(&mut s, "A"), ["filter.blur.gaussianBlur"]);
}
