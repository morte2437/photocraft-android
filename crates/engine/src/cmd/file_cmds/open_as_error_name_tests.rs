//! File › Open As… (#14049): an error names the file that was chosen and the format asked for,
//! never the stem plus the `as` extension that only picks the decoder.

use super::*;

fn tmp(name: &str) -> String {
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let id = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let d = std::env::temp_dir().join(format!("photocraft-open-as-name-{}-{name}-{id}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d.to_string_lossy().into_owned()
}

fn write_notes(dir: &str) -> String {
    let path = join(dir, "notes.txt");
    std::fs::write(&path, b"just some text.\n").unwrap();
    path
}

fn error_text(s: &mut Session, path: &str, as_ext: &str) -> String {
    match s.execute("file.openAs", json!({"path": path, "as": as_ext})) {
        Err(e) => e.to_string(),
        Ok(v) => panic!("opening a text file as {as_ext} succeeded: {v}"),
    }
}

#[test]
fn a_text_file_opened_as_pcx_names_notes_txt_and_pcx() {
    let dir = tmp("pcx");
    let path = write_notes(&dir);
    let mut s = Session::new();
    let msg = error_text(&mut s, &path, "pcx");
    assert!(msg.contains("notes.txt"), "{msg}");
    assert!(msg.contains("PCX"), "{msg}");
    assert!(!msg.contains("notes.pcx"), "{msg}");
    assert!(msg.contains("unrecognized image format"), "the decoder's detail stays: {msg}");
    assert_eq!(s.documents().len(), 0);
}

#[test]
fn a_text_file_opened_as_raw_names_notes_txt_and_raw() {
    let dir = tmp("raw");
    let path = write_notes(&dir);
    let mut s = Session::new();
    let msg = error_text(&mut s, &path, "raw");
    assert!(msg.contains("notes.txt"), "{msg}");
    assert!(msg.contains("RAW"), "{msg}");
    assert!(!msg.contains("notes.raw"), "{msg}");
    assert_eq!(s.documents().len(), 0);
}

#[test]
fn an_unknown_as_value_is_a_parameter_error_naming_it() {
    let dir = tmp("bogus");
    let path = write_notes(&dir);
    let mut s = Session::new();
    match s.execute("file.openAs", json!({"path": path, "as": "bogus"})) {
        Err(EngineError::BadParams { cmd, msg }) => {
            assert_eq!(cmd, "file.openAs");
            assert!(msg.contains("bogus"), "{msg}");
        }
        other => panic!("expected BadParams, got {other:?}"),
    }
    assert_eq!(s.documents().len(), 0);
}

#[test]
fn every_camera_raw_and_affinity_extension_is_an_open_as_format() {
    // The `as` check must take every format the importer reads, not a copy of the lists that can
    // fall behind them: a text file asked to open as one fails to read, never as a parameter error.
    let dir = tmp("raw-exts");
    let path = write_notes(&dir);
    let mut s = Session::new();
    for ext in photocraft_io::raw::EXTENSIONS.iter().chain(photocraft_io::affinity::EXTENSIONS) {
        let r = s.execute("file.openAs", json!({"path": path, "as": ext}));
        assert!(!matches!(r, Err(EngineError::BadParams { .. })), "{ext}: {r:?}");
    }
}

#[test]
fn a_png_opened_as_png_keeps_its_real_file_name() {
    let dir = tmp("png");
    let img = photocraft_codecs::Image::from_u8(4, 4, photocraft_codecs::ChannelLayout::Rgb, vec![200; 48]).unwrap();
    let bytes = photocraft_codecs::encode(&img, photocraft_codecs::Format::Png, &Default::default()).unwrap();
    let path = join(&dir, "photo one.png");
    std::fs::write(&path, bytes).unwrap();
    let mut s = Session::new();
    s.execute("file.openAs", json!({"path": path, "as": "png"})).unwrap();
    assert_eq!(s.documents().len(), 1);
    assert_eq!(s.active().unwrap().doc.name, "photo one.png");
}
