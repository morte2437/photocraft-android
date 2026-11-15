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
            assert_eq!(cmd, "file.placeEmbedded");
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

// ---------- place in steps, for a session with no worker threads (the web build) ----------

/// A pseudo-random PNG of `layout` and `sample`, so a wrong pixel anywhere shows and the file
/// is big enough to take many rows.
fn noise_png(w: u32, h: u32, layout: photocraft_codecs::ChannelLayout, sample: photocraft_codecs::SampleType, interlaced: bool) -> Vec<u8> {
    let len = w as usize * h as usize * layout.channels() * sample.bytes();
    let data: Vec<u8> = (0..len).map(|i| ((i.wrapping_mul(2_654_435_761) >> 13) ^ (i / 97)) as u8).collect();
    let img = photocraft_codecs::Image::from_raw(w, h, layout, sample, data).unwrap();
    photocraft_codecs::encode(&img, photocraft_codecs::Format::Png, &photocraft_codecs::EncodeOptions { png_interlaced: interlaced, ..Default::default() })
        .unwrap()
}

/// A cooperative session whose polls each do one unit of work, so the polls a place needs do not
/// depend on the machine's speed.
fn stepping(w: u32, h: u32, depth: u32) -> Session {
    let mut s = session(w, h, depth);
    s.set_cooperative_jobs(true);
    s.set_job_step_budget(Duration::ZERO);
    s
}

/// Polls until job `id` ends; returns its event and how many polls that took.
fn poll_to_end(s: &mut Session, id: JobId) -> (JobEvent, usize) {
    for polls in 1..=1_000_000 {
        if let Some(e) = s.poll_jobs().into_iter().find(|e| e.id == id) {
            return (e, polls);
        }
    }
    panic!("job {id:?} never ended");
}

/// A place a slice per poll equals the inline place: the same result, layer pixels, composite and
/// one history step, for every kind of PNG, at every depth, for a smart object and a pixel layer,
/// reduced to fit, left alone and enlarged.
#[test]
fn a_cooperative_place_equals_the_inline_place() {
    use photocraft_codecs::{ChannelLayout as L, SampleType as T};
    let dir = tmp("cooperative");
    let sources = [
        ("rgb8.png", noise_png(320, 300, L::Rgb, T::U8, false)),
        ("rgba16.png", noise_png(310, 301, L::Rgba, T::U16, false)),
        ("gray8-interlaced.png", noise_png(300, 305, L::Gray, T::U8, true)),
        ("graya8.png", noise_png(301, 300, L::GrayA, T::U8, false)),
    ];
    for (file, bytes) in &sources {
        let path = join(&dir, file);
        std::fs::write(&path, bytes).unwrap();
        for depth in [8, 16, 32] {
            for smart in [true, false] {
                for scale in [None, Some(100), Some(180)] {
                    let mut params = json!({"path": path, "smartObject": smart});
                    if let Some(k) = scale {
                        params["scale"] = json!(k);
                    }
                    let what = format!("{file} at {depth}-bit, smart {smart}, scale {scale:?}");
                    let mut inline = session(150, 150, depth);
                    let want = inline.execute("file.placeEmbedded", params.clone()).unwrap();

                    let mut s = stepping(150, 150, depth);
                    let before = steps(&s);
                    let id = job(s.start("file.placeEmbedded", params).unwrap());
                    assert!(s.has_jobs(), "{what}");
                    assert_eq!(steps(&s), before, "nothing is inserted before the job is applied");
                    let (e, polls) = poll_to_end(&mut s, id);
                    let JobOutcome::Done(got) = &e.outcome else { panic!("{what}: {e:?}") };
                    assert!(polls > 3, "{what}: took {polls} polls");
                    assert_eq!((e.command.as_str(), e.label.as_str()), ("file.placeEmbedded", &*format!("Placing {file}")));
                    same_but_layer(got, &want);
                    assert_eq!(steps(&s), before + 1, "{what}: one undo step");
                    assert_eq!(s.active().unwrap().history.undo_label(), Some(PLACE_EMBEDDED));
                    assert_eq!(layer_pixels(&s), layer_pixels(&inline), "{what}: layer pixels");
                    assert_eq!(canvas_pixels(&s), canvas_pixels(&inline), "{what}: composite");
                    if smart {
                        // The embedded source is the file's bytes, shared with the decoder.
                        let d = s.active().unwrap();
                        let LayerContent::Smart(so) = &d.doc.layer(d.active_layer.unwrap()).unwrap().content else { panic!("{what}") };
                        assert!(matches!(&so.source, SmartSource::Embedded { bytes: b, .. } if **b == *bytes), "{what}");
                    }
                    assert!(s.undo(), "{what}");
                    assert_eq!(s.active().unwrap().doc.layers.len(), 1);
                }
            }
        }
    }
}

/// With the real budget (8 ms a poll) the place ends the same, and Place Linked and bytes with no
/// path take the same road.
#[test]
fn a_cooperative_place_with_the_default_budget_and_from_bytes() {
    let bytes = gradient_png(300, 320);
    let mut inline = session(120, 120, 16);
    let want = place_bytes(&mut inline, "drop.png", bytes.clone(), None, &json!({})).unwrap();

    let mut s = session(120, 120, 16);
    s.set_cooperative_jobs(true);
    let id = job(s.start_place_bytes("drop.png", bytes.clone(), None, json!({})).unwrap());
    // The document is locked meanwhile, as for a worker.
    let refused = s.start_place_bytes("again.png", bytes.clone(), None, json!({})).unwrap_err().to_string();
    assert!(refused.contains("Placing drop.png"), "{refused}");
    let (e, _) = poll_to_end(&mut s, id);
    let JobOutcome::Done(got) = &e.outcome else { panic!("{e:?}") };
    same_but_layer(got, &want);
    assert_eq!(layer_pixels(&s), layer_pixels(&inline));
    assert_eq!(steps(&s), steps(&inline));

    let dir = tmp("cooperative-linked");
    let path = join(&dir, "grad.png");
    std::fs::write(&path, bytes).unwrap();
    let id = job(s.start("file.placeLinked", json!({"path": path})).unwrap());
    assert!(matches!(poll_to_end(&mut s, id).0.outcome, JobOutcome::Done(_)));
    let d = s.active().unwrap();
    assert!(
        matches!(&d.doc.layer(d.active_layer.unwrap()).unwrap().content, LayerContent::Smart(so) if matches!(&so.source, SmartSource::Linked { path: p } if *p == path))
    );
    assert_eq!(d.history.undo_label(), Some("Place Linked"));
}

/// An offset off the tile grid rebuilds the tiles the image touches, and a cooperative place does
/// that a few tiles per poll rather than all of them in one: it takes at least a poll per source
/// tile more than the same place on the grid, and ends with the same pixels as the inline place.
#[test]
fn a_cooperative_place_moves_its_pixels_a_few_tiles_per_poll() {
    let bytes = gradient_png(600, 520);
    let tiles = 3 * 3;
    let polls_at = |center: [i32; 2]| {
        let params = json!({"center": center, "smartObject": false});
        let mut inline = session(2000, 2000, 8);
        place_bytes(&mut inline, "drop.png", bytes.clone(), None, &params).unwrap();
        let mut s = stepping(2000, 2000, 8);
        let id = job(s.start_place_bytes("drop.png", bytes.clone(), None, params).unwrap());
        let (e, polls) = poll_to_end(&mut s, id);
        assert!(matches!(e.outcome, JobOutcome::Done(_)), "{e:?}");
        assert_eq!(layer_pixels(&s), layer_pixels(&inline), "at {center:?}");
        polls
    };
    // The image lands at (256, 256), a whole number of tiles, or at (257, 3).
    let on_grid = polls_at([300 + 256, 260 + 256]);
    let off_grid = polls_at([300 + 257, 260 + 3]);
    assert!(off_grid >= on_grid + tiles, "{off_grid} polls off the grid, {on_grid} on it");
}

/// `wait_job` runs a cooperative place to the end.
#[test]
fn wait_job_finishes_a_cooperative_place() {
    let bytes = gradient_png(300, 300);
    let mut inline = session(100, 100, 8);
    let want = place_bytes(&mut inline, "drop.png", bytes.clone(), None, &json!({})).unwrap();
    let mut s = stepping(100, 100, 8);
    let id = job(s.start_place_bytes("drop.png", bytes, None, json!({})).unwrap());
    s.poll_jobs();
    let got = s.wait_job(id).unwrap();
    same_but_layer(&got, &want);
    assert_eq!(layer_pixels(&s), layer_pixels(&inline));
    assert!(!s.has_jobs());
}

/// Progress goes up through the stages and the status names them.
#[test]
fn a_cooperative_place_reports_its_progress() {
    let mut s = stepping(100, 100, 8);
    let id = job(s.start_place_bytes("drop.png", gradient_png(300, 300), None, json!({})).unwrap());
    let (mut last, mut messages) = (0.0f32, Vec::new());
    while s.job(id).is_some() {
        s.poll_jobs();
        if let Some(info) = s.job(id) {
            assert!(info.progress >= last && info.progress < 1.0, "{last} -> {}", info.progress);
            last = info.progress;
            if !info.message.is_empty() && messages.last() != Some(&info.message) {
                messages.push(info.message);
            }
        }
    }
    assert!(last > 0.5, "{last}");
    assert_eq!(messages, ["Decoding", "Flattening", "Resizing", "Placing"]);
}

/// Cancelling between polls leaves the document the very same one and unlocks it.
#[test]
fn cancelling_a_cooperative_place_between_polls_leaves_the_document_unchanged() {
    let mut s = stepping(100, 100, 8);
    let (doc, before) = (s.active().unwrap().doc.clone(), steps(&s));
    let id = job(s.start_place_bytes("drop.png", gradient_png(300, 300), None, json!({})).unwrap());
    for _ in 0..5 {
        assert!(s.poll_jobs().is_empty());
    }
    assert!(s.cancel_job(id));
    assert_eq!(wait_event(&mut s, id).outcome, JobOutcome::Cancelled);
    assert!(Arc::ptr_eq(&doc, &s.active().unwrap().doc));
    assert_eq!(steps(&s), before);
    assert!(!s.active().unwrap().history.can_redo());
    assert!(s.active_job().is_none(), "the document is unlocked");
    assert!(matches!(s.wait_job(id), Err(EngineError::Cancelled)));
}

/// A PNG that is damaged part-way through its image data fails the job, without a panic and with
/// the document untouched; the rows before the damage are decoded first.
#[test]
fn a_cooperative_place_of_a_damaged_png_fails_cleanly() {
    use photocraft_codecs::{ChannelLayout as L, SampleType as T};
    let good = noise_png(320, 300, L::Rgb, T::U8, false);
    let mut flipped = good.clone();
    let mid = flipped.len() / 2;
    flipped[mid] ^= 0xFF;
    let cases = [
        ("cut in half", good.get(..good.len() / 2).unwrap().to_vec()),
        ("cut just after the header", good.get(..60).unwrap().to_vec()),
        ("a damaged byte", flipped),
        ("not a PNG", b"\x89PNG\r\n\x1a\nthen text".to_vec()),
        ("empty", Vec::new()),
    ];
    let mut s = stepping(100, 100, 8);
    let (doc, before) = (s.active().unwrap().doc.clone(), steps(&s));
    for (what, bytes) in cases {
        let Ok(Started::Job(id)) = s.start_place_bytes("bad.png", bytes, None, json!({})) else {
            // Refused before it started: also an error, not a panic.
            continue;
        };
        let (e, polls) = poll_to_end(&mut s, id);
        let JobOutcome::Failed(err) = &e.outcome else { panic!("{what}: {e:?}") };
        assert!(err.contains("bad.png"), "{what}: the file is named: {err}");
        if what == "cut in half" {
            assert!(polls > 20, "{what}: the rows before the cut are decoded first ({polls} polls)");
        }
        assert!(Arc::ptr_eq(&doc, &s.active().unwrap().doc), "{what}");
        assert_eq!(steps(&s), before, "{what}");
        assert!(s.active_job().is_none(), "{what}: unlocked");
    }
}

/// The flatten in bands is the flatten in one go, for every colour model and depth.
#[test]
fn a_banded_flatten_equals_flattened_checked() {
    use super::place_steps::Flatten;
    use photocraft_codecs::{ChannelLayout as L, SampleType as T};
    use photocraft_color::{ColorMode, PixelFormat, SampleType};
    // Wide enough for two bands across, and several tile rows.
    let sources = [
        noise_png(2100, 600, L::Gray, T::U8, false),
        noise_png(700, 530, L::Rgb, T::U8, false),
        noise_png(300, 520, L::Rgba, T::U16, false),
        noise_png(260, 520, L::GrayA, T::U16, false),
    ];
    let formats = [
        PixelFormat::new(ColorMode::Rgb, SampleType::U8, true),
        PixelFormat::new(ColorMode::Rgb, SampleType::U16, true),
        PixelFormat::new(ColorMode::Rgb, SampleType::F32, true),
        PixelFormat::new(ColorMode::Grayscale, SampleType::U8, false),
        PixelFormat::new(ColorMode::Grayscale, SampleType::U16, true),
    ];
    for bytes in &sources {
        let doc = photocraft_io::import("src.png", bytes).unwrap().document;
        for fmt in formats {
            let expected = flattened_checked(&doc, fmt, "file.placeEmbedded").unwrap();
            let mut flatten = Flatten::new(doc.clone(), true, fmt, "file.placeEmbedded").unwrap();
            let mut bands = 0;
            let got = loop {
                bands += 1;
                if let Some(px) = flatten.advance().unwrap() {
                    break px;
                }
            };
            assert!(bands > 1, "{}x{}: {bands} bands", doc.size.width, doc.size.height);
            assert!(got == expected, "{}x{} {:?} {:?}: banded != whole", doc.size.width, doc.size.height, doc.mode, fmt);
            // Not banded is the plain flatten.
            let whole = Flatten::new(doc.clone(), false, fmt, "file.placeEmbedded").unwrap().advance().unwrap().unwrap();
            assert!(whole == expected);
        }
    }
}

/// An Indexed Color document (a palette PNG) flattens in bands to the same pixels.
#[test]
fn a_banded_flatten_of_a_palette_image_equals_flattened_checked() {
    use super::place_steps::Flatten;
    let pal = [[255, 0, 0], [0, 255, 0], [0, 0, 255], [9, 9, 9]];
    let idx: Vec<u8> = (0..300 * 600).map(|i| ((i / 7 + i / 300) % 4) as u8).collect();
    let png = photocraft_codecs::encode_png_indexed(300, 600, &idx, &pal, Some(3), None, None).unwrap();
    let doc = photocraft_io::import("p.png", &png).unwrap().document;
    assert!(doc.color_table.is_some());
    let fmt = doc.pixel_format();
    let expected = flattened_checked(&doc, fmt, "file.placeEmbedded").unwrap();
    let mut flatten = Flatten::new(doc, true, fmt, "file.placeEmbedded").unwrap();
    let got = loop {
        if let Some(px) = flatten.advance().unwrap() {
            break px;
        }
    };
    assert!(got == expected);
}

/// A canvas too large to flatten is refused before any band is rendered.
#[test]
fn a_banded_flatten_keeps_the_size_limit() {
    use super::place_steps::Flatten;
    let doc = Document::new("huge", photocraft_geom::Size::new(40_000, 40_000), photocraft_color::ColorMode::Rgb, photocraft_color::SampleType::U8);
    let fmt = doc.pixel_format();
    assert!(Flatten::new(doc, true, fmt, "file.placeEmbedded").is_err());
}

/// The size refusal names the command that was run, embedded or linked, not a made-up id.
#[test]
fn the_flatten_size_error_names_the_place_command() {
    use super::place_steps::Flatten;
    let huge = || Document::new("huge", photocraft_geom::Size::new(40_000, 40_000), photocraft_color::ColorMode::Rgb, photocraft_color::SampleType::U8);
    let fmt = huge().pixel_format();
    for command in ["file.placeEmbedded", "file.placeLinked"] {
        match Flatten::new(huge(), true, fmt, command) {
            Err(EngineError::BadParams { cmd, .. }) => assert_eq!(cmd, command),
            other => panic!("{:?}", other.map(|_| ())),
        }
    }
    // Through a whole place: a linked one reports file.placeLinked.
    let bytes = photocraft_format::save_to_bytes(&huge(), &Default::default()).unwrap();
    let mut s = session(100, 100, 8);
    s.set_cooperative_jobs(true);
    s.set_job_step_budget(Duration::ZERO);
    let id = job(s.start_place_bytes("huge.pcraft", bytes, Some("huge.pcraft".into()), json!({})).unwrap());
    let (e, _) = poll_to_end(&mut s, id);
    let JobOutcome::Failed(err) = &e.outcome else { panic!("{e:?}") };
    assert!(err.contains("file.placeLinked"), "{err}");
}
