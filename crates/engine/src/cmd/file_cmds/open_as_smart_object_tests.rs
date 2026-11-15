//! File › Open as Smart Object… (#11678): a file opens as a new untitled document holding one
//! embedded smart object layer.

use super::*;

fn tmp(name: &str) -> String {
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let id = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let d = std::env::temp_dir().join(format!("photocraft-open-as-so-{}-{name}-{id}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d.to_string_lossy().into_owned()
}

/// A small PNG with a gradient, so the pixels differ from place to place.
fn write_png(dir: &str, name: &str, w: u32, h: u32) -> String {
    let px: Vec<u8> = (0..h).flat_map(|y| (0..w).flat_map(move |x| [(x * 255 / w) as u8, (y * 255 / h) as u8, ((x + y) % 256) as u8])).collect();
    let img = photocraft_codecs::Image::from_u8(w, h, photocraft_codecs::ChannelLayout::Rgb, px).unwrap();
    let bytes = photocraft_codecs::encode(&img, photocraft_codecs::Format::Png, &Default::default()).unwrap();
    let path = join(dir, name);
    std::fs::write(&path, bytes).unwrap();
    path
}

/// A two-layer 16-bit PSD at 300 ppi tagged with Display P3, made with the engine's own save.
fn write_psd16(dir: &str, name: &str, w: u32, h: u32) -> String {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": w, "height": h, "depth": 16})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.execute("image.imageSize", json!({"resolution": 300, "resample": "none"})).unwrap();
    s.execute("edit.assignProfile", json!({"profile": "display-p3"})).unwrap();
    let path = join(dir, name);
    save_doc(&s.active().unwrap().doc, &path, None).unwrap();
    path
}

fn imported(path: &str) -> Document {
    photocraft_io::import(&file_name(path), &std::fs::read(path).unwrap()).unwrap().document
}

fn active(s: &Session) -> &Document {
    &s.active().unwrap().doc
}

fn names(s: &Session) -> Vec<String> {
    s.documents().iter().map(|d| d.doc.name.clone()).collect()
}

/// The one layer of the active document, which must be an embedded smart object.
fn embedded(s: &Session) -> (String, String, Arc<Vec<u8>>) {
    let d = active(s);
    assert_eq!(d.layers.len(), 1, "one layer, not a Background plus a layer");
    let layer = &d.layers[0];
    match &layer.content {
        LayerContent::Smart(so) => match &so.source {
            SmartSource::Embedded { file_name, bytes } => (layer.name.clone(), file_name.clone(), Arc::clone(bytes)),
            other => panic!("expected an embedded smart object, got {other:?}"),
        },
        _ => panic!("the layer is not a smart object"),
    }
}

#[test]
fn a_png_opens_as_an_untitled_document_with_one_embedded_smart_object() {
    let dir = tmp("png");
    let path = write_png(&dir, "a_test.png", 64, 48);
    let mut s = Session::new();
    let r = s.execute("file.openAsSmartObject", json!({"path": path})).unwrap();
    assert_eq!(r["name"], "a_test-1", "{r}");
    assert_eq!(s.documents().len(), 1);
    let st = s.active().unwrap();
    assert_eq!(st.doc.name, "a_test-1");
    assert_eq!(st.path, None, "untitled: Save asks where to save");

    let file = imported(&path);
    let d = &st.doc;
    assert_eq!((d.size.width, d.size.height), (64, 48));
    assert_eq!(d.resolution_dpi, file.resolution_dpi);
    assert_eq!((d.mode, d.depth), (file.mode, file.depth));
    assert_eq!(d.icc_profile, file.icc_profile);

    let (layer, original, bytes) = embedded(&s);
    assert_eq!(layer, "a_test", "named after the file without its extension");
    assert_eq!(original, "a_test.png", "Edit Contents opens the original file name");
    assert_eq!(*bytes, std::fs::read(&path).unwrap(), "the file itself is embedded");
}

#[test]
fn the_smart_object_shows_the_file() {
    let dir = tmp("pixels");
    let path = write_png(&dir, "grad.png", 32, 16);
    let mut s = Session::new();
    s.execute("file.openAsSmartObject", json!({"path": path})).unwrap();
    let want = photocraft_compose::render(&imported(&path), Rect::new(0, 0, 32, 16)).px.clone();
    let got = photocraft_compose::render(active(&s), Rect::new(0, 0, 32, 16)).px.clone();
    assert_eq!(got, want);
}

#[test]
fn a_16_bit_psd_keeps_size_resolution_mode_depth_and_profile() {
    let dir = tmp("psd");
    let path = write_psd16(&dir, "c.layered.psd", 40, 30);
    let mut s = Session::new();
    // The files were measured in the working spaces, where nothing asks; Preserve keeps Display P3.
    s.execute("edit.colorSettings", json!({"askOnMismatch": false})).unwrap();
    s.execute("file.openAsSmartObject", json!({"path": path})).unwrap();
    let file = imported(&path);
    assert_eq!(file.resolution_dpi, 300.0);
    assert!(file.icc_profile.is_some());
    let d = active(&s);
    assert_eq!(d.name, "c.layered-1", "only the last extension is dropped");
    assert_eq!((d.size.width, d.size.height), (40, 30));
    assert_eq!(d.resolution_dpi, 300.0);
    assert_eq!((d.mode, d.depth), (file.mode, file.depth));
    assert_eq!(d.depth, photocraft_doc::SampleType::U16);
    assert_eq!(d.icc_profile, file.icc_profile);
    let (layer, original, _) = embedded(&s);
    assert_eq!((layer.as_str(), original.as_str()), ("c.layered", "c.layered.psd"));
}

#[test]
fn history_is_the_single_state_open_as_smart_object() {
    let dir = tmp("history");
    let path = write_png(&dir, "a_test.png", 16, 16);
    let mut s = Session::new();
    s.execute("file.openAsSmartObject", json!({"path": path})).unwrap();
    let st = s.active().unwrap();
    assert_eq!(st.history.entries(), ["Open As Smart Object"], "one state, as File > Open has");
    assert_eq!(st.snapshots.first().map(|snap| snap.name.as_str()), Some("a_test"), "the opening snapshot is named after the layer");
}

#[test]
fn the_opening_snapshot_shows_the_placed_smart_object() {
    let dir = tmp("snapshot");
    let path = write_png(&dir, "a_test.png", 16, 16);
    let mut s = Session::new();
    s.execute("file.openAsSmartObject", json!({"path": path})).unwrap();
    let st = s.active().unwrap();
    let first = st.snapshots.first().unwrap();
    assert_eq!(first.doc.layers.len(), 1, "snapshot 0 already holds the smart object");
    assert!(matches!(first.doc.layers[0].content, LayerContent::Smart(_)));
    assert_eq!(first.layers.active, Some(first.doc.layers[0].id));
    assert_eq!(st.active_layer, Some(first.doc.layers[0].id));
    assert_eq!(st.history.entries(), ["Open As Smart Object"]);
}

#[test]
fn the_number_is_the_next_free_one_and_starts_again_after_they_close() {
    let dir = tmp("numbers");
    let path = write_png(&dir, "a_test.png", 8, 8);
    let mut s = Session::new();
    for _ in 0..3 {
        s.execute("file.openAsSmartObject", json!({"path": path})).unwrap();
    }
    assert_eq!(names(&s), ["a_test-1", "a_test-2", "a_test-3"]);
    while !s.documents().is_empty() {
        s.close(0);
    }
    s.execute("file.openAsSmartObject", json!({"path": path})).unwrap();
    assert_eq!(names(&s), ["a_test-1"]);
}

#[test]
fn a_plain_open_of_the_same_file_is_unchanged() {
    let dir = tmp("plain");
    let path = write_png(&dir, "a_test.png", 8, 8);
    let mut s = Session::new();
    s.execute("file.openAsSmartObject", json!({"path": path})).unwrap();
    open_bytes_as(&mut s, &path, &std::fs::read(&path).unwrap(), None, Some(path.clone())).unwrap();
    assert_eq!(names(&s), ["a_test-1", "a_test.png"]);
    assert!(matches!(active(&s).layers[0].content, LayerContent::Raster(_)));
}

#[test]
fn several_files_open_one_document_each() {
    let dir = tmp("several");
    let a = write_png(&dir, "a_test.png", 8, 8);
    let b = write_psd16(&dir, "d_other.psd", 12, 10);
    let mut s = Session::new();
    s.execute("edit.colorSettings", json!({"askOnMismatch": false})).unwrap();
    s.execute("file.openAsSmartObject", json!({"path": a})).unwrap();
    s.execute("file.openAsSmartObject", json!({"path": b})).unwrap();
    assert_eq!(names(&s), ["a_test-1", "d_other-1"]);
    for st in s.documents() {
        assert_eq!(st.doc.layers.len(), 1);
        assert!(matches!(st.doc.layers[0].content, LayerContent::Smart(_)));
    }
}

#[test]
fn a_missing_or_unreadable_file_is_an_error_not_a_crash() {
    let dir = tmp("bad");
    let junk = join(&dir, "notes.png");
    std::fs::write(&junk, b"this is not an image").unwrap();
    let text = join(&dir, "readme.txt");
    std::fs::write(&text, b"hello").unwrap();
    let mut s = Session::new();
    for path in [join(&dir, "missing.png"), junk, text] {
        let r = s.execute("file.openAsSmartObject", json!({"path": path}));
        assert!(r.is_err(), "{path}: {r:?}");
    }
    assert!(s.documents().is_empty(), "a failed open leaves no document behind");
}

#[test]
fn bad_params_are_errors() {
    let mut s = Session::new();
    for params in [json!({}), json!({"path": 7}), json!({"path": null})] {
        let r = s.execute("file.openAsSmartObject", params.clone());
        assert!(r.is_err(), "{params}: {r:?}");
    }
    assert!(s.documents().is_empty());
}

fn job(r: crate::jobs::Started) -> crate::jobs::JobId {
    match r {
        crate::jobs::Started::Job(id) => id,
        crate::jobs::Started::Done(v) => panic!("expected a background job, ran inline: {v}"),
    }
}

/// Poll until job `id` ends (or 60 s pass); returns its event.
fn wait_event(s: &mut Session, id: crate::jobs::JobId) -> crate::jobs::JobEvent {
    let t = std::time::Instant::now();
    loop {
        if let Some(e) = s.poll_jobs().into_iter().find(|e| e.id == id) {
            return e;
        }
        assert!(t.elapsed() < std::time::Duration::from_secs(60), "job {id:?} did not finish");
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
}

#[test]
fn the_import_runs_as_a_job_when_started() {
    let dir = tmp("job");
    let path = write_png(&dir, "a_test.png", 32, 24);
    let mut s = Session::new();
    let id = job(s.start("file.openAsSmartObject", json!({"path": path})).unwrap());
    assert!(s.documents().is_empty(), "nothing is admitted before the job is applied");
    let e = wait_event(&mut s, id);
    let crate::jobs::JobOutcome::Done(v) = &e.outcome else { panic!("{e:?}") };
    assert!(v["docId"].is_u64(), "{v}");
    assert_eq!(v["name"], "a_test-1", "{v}");
    assert_eq!(s.documents().len(), 1);
    embedded(&s);
}

#[test]
fn the_colour_policy_matches_file_open() {
    let dir = tmp("policy");
    let path = write_psd16(&dir, "c.layered.psd", 40, 30);
    let settings = json!({"askOnMismatch": false, "policyRgb": "convert"});
    let mut open = Session::new();
    open.execute("edit.colorSettings", settings.clone()).unwrap();
    open.execute("file.openAs", json!({"path": path})).unwrap();
    let mut so = Session::new();
    so.execute("edit.colorSettings", settings).unwrap();
    so.execute("file.openAsSmartObject", json!({"path": path})).unwrap();
    let (a, b) = (active(&open), active(&so));
    assert_eq!(a.icc_profile, b.icc_profile, "Convert is applied as File > Open applies it");
    assert_eq!((a.mode, a.depth, a.size), (b.mode, b.depth, b.size));
    assert_eq!(a.resolution_dpi, b.resolution_dpi);
}

#[test]
fn ask_when_opening_holds_the_document_like_file_open() {
    let dir = tmp("ask");
    let path = write_psd16(&dir, "c.layered.psd", 40, 30);
    let mut s = Session::new();
    let r = s.execute("file.openAsSmartObject", json!({"path": path})).unwrap();
    assert_eq!(r["color"]["ask"], true, "{r}");
    assert_eq!(r["smartObject"], true, "{r}");
    assert!(r["held"].is_u64(), "{r}");
    assert!(s.documents().is_empty(), "held for the profile prompt, not admitted");
    let (doc, _) = s.take_held_open(r["held"].as_u64().unwrap()).unwrap();
    assert_eq!(doc.name, "c.layered-1");
    assert_eq!(doc.layers.len(), 1);
    assert!(matches!(doc.layers[0].content, LayerContent::Smart(_)));
}

#[test]
fn the_reply_flags_a_smart_object_so_a_queued_document_keeps_its_history_label() {
    let dir = tmp("flag");
    let path = write_png(&dir, "a_test.png", 16, 12);
    let mut s = Session::new();
    let r = s.execute("file.openAsSmartObject", json!({"path": path})).unwrap();
    assert_eq!(r["smartObject"], true, "{r}");
}

#[test]
fn a_damaged_psd_is_held_for_confirmation_when_asked_and_opens_otherwise() {
    let dir = tmp("damaged");
    let path = join(&dir, "d.psd");
    std::fs::write(&path, crate::jobs::psd_open_fixture::make_psd(true)).unwrap();
    let mut s = Session::new();
    s.execute("edit.colorSettings", json!({"askOnMismatch": false})).unwrap();
    let r = s.execute("file.openAsSmartObject", json!({"path": path, "confirmDamage": true})).unwrap();
    assert_eq!(r["damaged"], true, "{r}");
    assert_eq!(r["smartObject"], true, "{r}");
    assert!(r["held"].is_u64(), "{r}");
    assert!(s.documents().is_empty(), "held for the damage prompt, not admitted");
    let r = s.execute("file.openAsSmartObject", json!({"path": path})).unwrap();
    assert!(r["docId"].is_u64(), "a headless caller opens the recovered content: {r}");
}

#[test]
fn a_document_held_for_its_prompt_does_not_share_its_name_with_the_next() {
    let dir = tmp("held-name");
    let path = write_psd16(&dir, "a_test.psd", 20, 20);
    let mut s = Session::new();
    let first = s.execute("file.openAsSmartObject", json!({"path": path})).unwrap();
    let second = s.execute("file.openAsSmartObject", json!({"path": path})).unwrap();
    assert_eq!(first["name"], "a_test-1");
    assert_eq!(second["name"], "a_test-2", "the held document's name is taken: {second}");
    // Both held docs leave the session for the app's queue; admitting them one by one numbers each.
    let (a, _) = s.take_held_open(first["held"].as_u64().unwrap()).unwrap();
    let (b, _) = s.take_held_open(second["held"].as_u64().unwrap()).unwrap();
    admit_as_smart_object(&mut s, a);
    admit_as_smart_object(&mut s, b);
    assert_eq!(names(&s), ["a_test-1", "a_test-2"]);
    let mut t = Session::new();
    let (c, _) = {
        let r = t.execute("file.openAsSmartObject", json!({"path": path})).unwrap();
        t.take_held_open(r["held"].as_u64().unwrap()).unwrap()
    };
    let (d, _) = {
        t.execute("edit.colorSettings", json!({"askOnMismatch": true})).unwrap();
        let r = t.execute("file.openAsSmartObject", json!({"path": path})).unwrap();
        t.take_held_open(r["held"].as_u64().unwrap()).unwrap()
    };
    assert_eq!(c.name, d.name, "taken out of the session, both are named a_test-1 when read");
    admit_as_smart_object(&mut t, c);
    admit_as_smart_object(&mut t, d);
    assert_eq!(names(&t), ["a_test-1", "a_test-2"], "the second takes the next free number when admitted");
}

#[test]
fn a_cancelled_hold_frees_its_number_for_the_one_admitted_after_it() {
    let dir = tmp("cancelled-hold");
    let path = write_psd16(&dir, "photo.psd", 20, 20);
    let mut s = Session::new();
    let first = s.execute("file.openAsSmartObject", json!({"path": path})).unwrap();
    let second = s.execute("file.openAsSmartObject", json!({"path": path})).unwrap();
    assert_eq!((first["name"].as_str(), second["name"].as_str()), (Some("photo-1"), Some("photo-2")));
    // photo-1 is cancelled while photo-2 waits; photo-2 is admitted with nothing open.
    s.take_held_open(first["held"].as_u64().unwrap()).unwrap();
    let (b, _) = s.take_held_open(second["held"].as_u64().unwrap()).unwrap();
    admit_as_smart_object(&mut s, b);
    assert_eq!(names(&s), ["photo-1"], "the first free number is taken at admission, with no collision");
}

#[test]
fn the_reply_names_the_document_as_admitted() {
    let dir = tmp("reply-name");
    let path = write_psd16(&dir, "photo.psd", 20, 20);
    let mut s = Session::new();
    s.execute("edit.colorSettings", json!({"askOnMismatch": true})).unwrap();
    let held = s.execute("file.openAsSmartObject", json!({"path": path})).unwrap();
    assert_eq!(held["name"], "photo-1", "{held}");
    assert!(held["held"].is_u64(), "{held}");
    // The next open skips the prompt: photo-1 is held, so the number read was 2, but admission takes the first free one.
    s.execute("edit.colorSettings", json!({"askOnMismatch": false})).unwrap();
    let r = s.execute("file.openAsSmartObject", json!({"path": path})).unwrap();
    assert_eq!(names(&s), ["photo-1"], "{r}");
    assert_eq!(r["name"], "photo-1", "the reply carries the name the document was admitted under: {r}");
}
