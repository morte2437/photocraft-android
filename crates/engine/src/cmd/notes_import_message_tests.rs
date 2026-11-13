//! #13915: File › Import › Notes… from a source that gives no notes fails as Photoshop does, with
//! "No notes were imported.", no history step and the document unmodified. A file that cannot be
//! read at all keeps its "<file>: <why>" error.

use photocraft_color::{ColorMode, SampleType};
use photocraft_doc::{Document, Note, Size};
use serde_json::json;

use super::{NO_NOTES_IMPORTED, import_notes_from};
use crate::Session;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 100, "height": 80})).unwrap();
    s
}

fn source(notes: usize) -> Document {
    let mut d = Document::new("src", Size::new(60, 40), ColorMode::Rgb, SampleType::U8);
    for i in 0..notes {
        d.notes.push(Note { text: format!("note {i}"), position: [10.0, 10.0], ..Default::default() });
    }
    d
}

fn pcraft(notes: usize) -> Vec<u8> {
    photocraft_format::save_to_bytes(&source(notes), &Default::default()).unwrap()
}

fn psd(notes: usize) -> Vec<u8> {
    photocraft_io::export(&source(notes), "x.psd", &photocraft_io::ExportOptions::default()).unwrap().bytes
}

/// Imports `bytes` as `name` and checks the refusal leaves the document as it was.
fn refused(name: &str, bytes: &[u8]) -> String {
    let mut s = session();
    let past = s.active().unwrap().history.past_len();
    let dirty = s.active().unwrap().is_dirty();
    let e = import_notes_from(&mut s, name, bytes).expect_err(name).to_string();
    let a = s.active().unwrap();
    assert_eq!(a.history.past_len(), past, "{name}: no history step");
    assert_eq!(a.is_dirty(), dirty, "{name}: the document stays unmodified");
    assert!(a.doc.notes.is_empty(), "{name}");
    e
}

#[test]
fn a_file_without_notes_says_no_notes_were_imported() {
    assert_eq!(refused("none.pcraft", &pcraft(0)), NO_NOTES_IMPORTED);
    assert_eq!(refused("none.psd", &psd(0)), NO_NOTES_IMPORTED);
    assert_eq!(NO_NOTES_IMPORTED, "No notes were imported.");
}

#[test]
fn a_file_that_is_no_notes_source_says_the_same() {
    // Not an image.
    assert_eq!(refused("x.txt", b"just some text\n"), NO_NOTES_IMPORTED);
    assert_eq!(refused("empty.psd", &[]), NO_NOTES_IMPORTED);
    // Damaged: a PSD signature with nothing valid behind it.
    assert_eq!(refused("bad.psd", b"8BPS\0\x01\xff\xff\xff\xff\xff\xff"), NO_NOTES_IMPORTED);
    // Truncated files that do have notes, cut at several lengths.
    for (name, bytes) in [("cut.psd", psd(2)), ("cut.pcraft", pcraft(2))] {
        for len in [4, 26, bytes.len() / 2, bytes.len() - 1] {
            assert_eq!(refused(name, &bytes[..len]), NO_NOTES_IMPORTED, "{name} cut to {len} bytes");
        }
    }
}

#[test]
fn a_file_with_notes_still_imports() {
    for (name, bytes) in [("a.pcraft", pcraft(2)), ("a.psd", psd(2))] {
        let mut s = session();
        let past = s.active().unwrap().history.past_len();
        let r = import_notes_from(&mut s, name, &bytes).unwrap();
        assert_eq!(r["imported"], 2, "{name}");
        assert_eq!(s.active().unwrap().doc.notes.len(), 2, "{name}");
        assert_eq!(s.active().unwrap().history.past_len(), past + 1, "{name}");
    }
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn a_file_that_cannot_be_read_keeps_its_own_error() {
    let mut s = session();
    let path = std::env::temp_dir().join(format!("pc-notes-missing-{}", std::process::id())).join("gone.psd");
    let p = path.to_string_lossy().to_string();
    let e = s.execute("file.import.notes", json!({"path": p})).unwrap_err().to_string();
    assert!(e.starts_with(&format!("{p}: ")), "{e}");
    assert_ne!(e, NO_NOTES_IMPORTED);
}
