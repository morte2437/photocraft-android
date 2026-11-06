//! Place as a background job (#10031): the decode, flatten and resize run on a worker and the
//! layer is inserted when the job is applied. Inline callers see no difference.

use std::time::{Duration, Instant};

use super::*;
use crate::jobs::{JobEvent, JobId, JobOutcome, Started};

fn tmp(name: &str) -> String {
    let d = std::env::temp_dir().join(format!("photocraft-place-job-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d.to_string_lossy().into_owned()
}

fn session(w: u32, h: u32, depth: u32) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": w, "height": h, "depth": depth})).unwrap();
    s
}

/// A small RGB gradient PNG, so a wrong pixel anywhere shows.
fn gradient_png(w: u32, h: u32) -> Vec<u8> {
    let px: Vec<u8> = (0..h).flat_map(|y| (0..w).flat_map(move |x| [(x * 255 / w) as u8, (y * 255 / h) as u8, ((x + y) % 256) as u8])).collect();
    let img = photocraft_codecs::Image::from_u8(w, h, photocraft_codecs::ChannelLayout::Rgb, px).unwrap();
    photocraft_codecs::encode(&img, photocraft_codecs::Format::Png, &Default::default()).unwrap()
}

fn job(r: Started) -> JobId {
    match r {
        Started::Job(id) => id,
        Started::Done(v) => panic!("expected a background job, ran inline: {v}"),
    }
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

/// The active layer's pixels over the canvas and its content.
fn layer_pixels(s: &Session) -> Vec<f32> {
    let d = s.active().unwrap();
    let surface = d.doc.layer(d.active_layer.unwrap()).unwrap().surface().unwrap();
    surface.read_region(d.doc.bounds().union(&surface.content_bounds()))
}

fn canvas_pixels(s: &Session) -> Vec<[f32; 4]> {
    photocraft_compose::render(&s.active().unwrap().doc, s.active().unwrap().doc.bounds()).px.clone()
}

fn steps(s: &Session) -> usize {
    s.active().unwrap().history.past_len()
}

/// The place a job does is the place the command does inline: the same result, layer, pixels and
/// one history step, at every depth, for a smart object and for a pixel layer.
#[test]
fn a_place_job_equals_the_inline_place() {
    let dir = tmp("equal");
    let path = join(&dir, "grad.png");
    std::fs::write(&path, gradient_png(160, 80)).unwrap();
    for depth in [8, 16, 32] {
        for smart in [true, false] {
            let params = json!({"path": path, "smartObject": smart});
            let mut inline = session(100, 100, depth);
            let want = inline.execute("file.placeEmbedded", params.clone()).unwrap();

            let mut s = session(100, 100, depth);
            let before = steps(&s);
            let id = job(s.start("file.placeEmbedded", params).unwrap());
            assert_eq!(steps(&s), before, "nothing is inserted before the job is applied");
            let e = wait_event(&mut s, id);
            let JobOutcome::Done(got) = &e.outcome else { panic!("{depth} {smart}: {e:?}") };
            assert_eq!(e.command, "file.placeEmbedded");
            assert_eq!(e.label, "Placing grad.png");
            same_but_layer(got, &want);
            assert_eq!(steps(&s), before + 1, "one undo step");
            assert_eq!(s.active().unwrap().history.undo_label(), Some(PLACE_EMBEDDED));
            assert_eq!(layer_pixels(&s), layer_pixels(&inline), "layer pixels at {depth}-bit, smart {smart}");
            assert_eq!(canvas_pixels(&s), canvas_pixels(&inline), "composite at {depth}-bit, smart {smart}");
            assert!(s.undo());
            assert_eq!(s.active().unwrap().doc.layers.len(), 1);
        }
    }
}

/// Place Linked as a job keeps the link and its history label.
#[test]
fn a_linked_place_job_keeps_the_link() {
    let dir = tmp("linked");
    let path = join(&dir, "grad.png");
    std::fs::write(&path, gradient_png(40, 40)).unwrap();
    let mut s = session(100, 100, 8);
    let id = s.start("file.placeLinked", json!({"path": path})).map(job).unwrap();
    assert!(matches!(wait_event(&mut s, id).outcome, JobOutcome::Done(_)));
    let d = s.active().unwrap();
    let l = d.doc.layer(d.active_layer.unwrap()).unwrap();
    assert!(matches!(&l.content, LayerContent::Smart(so) if matches!(&so.source, SmartSource::Linked { path: p } if *p == path)));
    assert_eq!(d.history.undo_label(), Some("Place Linked"));
}

/// Bytes with no path (a web or dropped file) start the same job through `start_place_bytes`.
#[test]
fn start_place_bytes_matches_place_bytes() {
    let bytes = gradient_png(160, 80);
    let mut inline = session(100, 100, 16);
    let want = place_bytes(&mut inline, "drop.png", bytes.clone(), None, &json!({})).unwrap();

    let mut s = session(100, 100, 16);
    let id = job(s.start_place_bytes("drop.png", bytes.clone(), None, json!({})).unwrap());
    // The document is locked meanwhile: another place (or edit) is refused with the job's name.
    let refused = s.start_place_bytes("again.png", bytes, None, json!({})).unwrap_err().to_string();
    assert!(refused.contains("Placing drop.png"), "{refused}");
    assert!(s.execute("layer.new.layer", json!({})).is_err());
    assert!(s.execute("jobs.list", json!({})).is_ok(), "queries still run");
    let e = wait_event(&mut s, id);
    assert_eq!(e.command, "file.placeEmbedded");
    let JobOutcome::Done(got) = &e.outcome else { panic!("{e:?}") };
    same_but_layer(got, &want);
    assert_eq!(got["layer"], json!(s.active().unwrap().active_layer.unwrap().0), "the result names the placed layer");
    assert_eq!(layer_pixels(&s), layer_pixels(&inline));
    assert_eq!(steps(&s), steps(&inline));
}

/// Assert a place result equals `want` except for its `layer`: layer ids come from a counter
/// shared by every session, so two sessions never hand out the same one.
fn same_but_layer(got: &Value, want: &Value) {
    let strip = |v: &Value| {
        let mut v = v.clone();
        v.as_object_mut().map(|o| o.remove("layer"));
        v
    };
    assert!(got["layer"].is_u64(), "{got}");
    assert_eq!(strip(got), strip(want));
}

/// Cancelling stops the place: nothing is inserted and the document is the very same one.
#[test]
fn cancelling_a_place_job_leaves_the_document_unchanged() {
    let mut s = session(100, 100, 8);
    let (doc, before) = (s.active().unwrap().doc.clone(), steps(&s));
    let id = job(s.start_place_bytes("drop.png", gradient_png(160, 80), None, json!({})).unwrap());
    assert!(s.cancel_job(id));
    s.join_cancelled_jobs();
    assert_eq!(wait_event(&mut s, id).outcome, JobOutcome::Cancelled);
    assert!(Arc::ptr_eq(&doc, &s.active().unwrap().doc));
    assert_eq!(steps(&s), before);
    assert!(!s.active().unwrap().history.can_redo());
    assert!(matches!(s.wait_job(id), Err(EngineError::Cancelled)));
    assert!(s.active_job().is_none(), "the document is unlocked");
}

/// An edit made while the job runs wins: applying fails and no layer is placed.
#[test]
fn an_edit_during_a_place_job_discards_the_place() {
    let mut s = session(100, 100, 8);
    let id = job(s.start_place_bytes("drop.png", gradient_png(160, 80), None, json!({})).unwrap());
    // Commands are refused while the job runs; a direct edit (the shell's, say) is not.
    s.edit("Poke", |doc, _| {
        doc.name = "poked".into();
        Ok(())
    })
    .unwrap();
    let e = wait_event(&mut s, id);
    let JobOutcome::Failed(err) = &e.outcome else { panic!("{e:?}") };
    assert!(err.contains("Placing drop.png") && err.contains("document changed"), "{err}");
    let d = s.active().unwrap();
    assert_eq!(d.doc.name, "poked", "the edit is kept");
    assert_eq!(d.doc.layers.len(), 1, "nothing was placed");
    assert_eq!(d.history.undo_label(), Some("Poke"));
}

/// Hostile input as a job is an error, never a panic, and the document is left alone.
#[test]
fn bad_input_as_a_place_job_fails_without_panicking() {
    let dir = tmp("bad");
    let good = gradient_png(32, 32);
    let empty = join(&dir, "empty.png");
    std::fs::write(&empty, b"").unwrap();
    let truncated = join(&dir, "truncated.png");
    std::fs::write(&truncated, good.get(..good.len() / 2).unwrap()).unwrap();
    let garbage = join(&dir, "garbage.png");
    std::fs::write(&garbage, [0x89u8, b'P', b'N', b'G', 0xff, 0x00, 0x13, 0x37, 0x42]).unwrap();

    let mut s = session(64, 64, 8);
    let (doc, before) = (s.active().unwrap().doc.clone(), steps(&s));
    for path in [&empty, &truncated, &garbage] {
        let id = job(s.start("file.placeEmbedded", json!({"path": path})).unwrap());
        let e = wait_event(&mut s, id);
        let JobOutcome::Failed(err) = &e.outcome else { panic!("{path}: {e:?}") };
        assert!(err.contains("png"), "the file is named: {err}");
    }
    for bytes in [Vec::new(), good.get(..10).unwrap().to_vec(), vec![0u8; 3]] {
        let id = job(s.start_place_bytes("bad.png", bytes, None, json!({})).unwrap());
        assert!(matches!(wait_event(&mut s, id).outcome, JobOutcome::Failed(_)));
    }
    assert!(Arc::ptr_eq(&doc, &s.active().unwrap().doc), "a failed place changes nothing");
    assert_eq!(steps(&s), before);
    // Params of the wrong type fall back to the defaults or are refused, but never panic.
    let params = json!({"scale": "big", "center": "middle", "fit": "yes", "smartObject": 3});
    if let Ok(Started::Job(id)) = s.start_place_bytes("ok.png", good, None, params) {
        assert!(matches!(wait_event(&mut s, id).outcome, JobOutcome::Done(_)));
        assert!(s.undo());
    }
    assert_eq!(steps(&s), before);
    assert_eq!(s.active().unwrap().doc.layers.len(), 1);
    // Inline, a missing document is the same error as before.
    let mut none = Session::new();
    assert!(matches!(place_bytes(&mut none, "a.png", Vec::new(), None, &json!({})), Err(EngineError::NoDocument)));
    assert!(matches!(none.start_place_bytes("a.png", Vec::new(), None, json!({})), Err(EngineError::NoDocument)));
}

/// A place from a path reads the file in the job, not in the command that starts it: a large file
/// doesn't hold up the window while it is read. A file that can't be read fails the job with the
/// error the inline place gives, and changes nothing.
#[test]
fn a_place_job_reads_its_file_on_the_worker() {
    let dir = tmp("read");
    let missing = join(&dir, "missing.png");
    let mut inline = session(64, 64, 8);
    let want = inline.execute("file.placeEmbedded", json!({"path": missing})).unwrap_err().to_string();
    assert!(want.contains(&missing), "{want}");

    let mut s = session(64, 64, 8);
    let (doc, before) = (s.active().unwrap().doc.clone(), steps(&s));
    let id = job(s.start("file.placeEmbedded", json!({"path": missing})).expect("the job starts before the file is read"));
    let e = wait_event(&mut s, id);
    let JobOutcome::Failed(err) = &e.outcome else { panic!("{e:?}") };
    assert_eq!(err, &want);
    assert!(Arc::ptr_eq(&doc, &s.active().unwrap().doc));
    assert_eq!(steps(&s), before);
}

/// A placed file whose canvas is past the compositor's limit is refused with the same error the
/// place gave before it ran as a job: bad params naming the canvas.
#[test]
fn a_canvas_too_large_to_flatten_is_refused_as_before() {
    let huge = Document::new("huge", photocraft_doc::Size::new(40_000, 30_000), ColorMode::Rgb, photocraft_color::SampleType::U8);
    let bytes = photocraft_format::save_to_bytes(&huge, &Default::default()).unwrap();
    let mut s = session(100, 100, 8);
    let before = steps(&s);
    match place_bytes(&mut s, "huge.pcraft", bytes.clone(), None, &json!({})) {
        Err(EngineError::BadParams { cmd, msg }) => {
            assert_eq!(cmd, "file.place");
            assert_eq!(msg, "the document's 40000×30000 canvas is too large to flatten");
        }
        other => panic!("{other:?}"),
    }
    let id = job(s.start_place_bytes("huge.pcraft", bytes, None, json!({})).unwrap());
    let e = wait_event(&mut s, id);
    let JobOutcome::Failed(err) = &e.outcome else { panic!("{e:?}") };
    assert!(err.contains("40000×30000 canvas is too large to flatten"), "{err}");
    assert_eq!(steps(&s), before, "nothing was placed");
}
