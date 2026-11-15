//! #11795: what a new note stores, as in Photoshop: colour 255, 255, 131, a 240 × 140 popup at the
//! icon's top-left + (8, 10), and a local-time PDF date with its offset. Document operations move
//! the popup with its icon and keep its size.

use chrono::{FixedOffset, TimeZone};
use serde_json::json;

use super::notes_cmds::{DEFAULT_NOTE_COLOR, pdf_date_at};
use crate::Session;

fn session(w: u32, h: u32) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": w, "height": h, "background": "white"})).unwrap();
    s
}

fn note(s: &Session, i: usize) -> photocraft_doc::Note {
    s.active().unwrap().doc.notes[i].clone()
}

#[test]
fn a_new_note_has_the_default_colour_and_popup() {
    let mut s = session(600, 400);
    s.execute("notes.add", json!({"x": 100, "y": 100})).unwrap();
    let n = note(&s, 0);
    let rgb = n.color.to_rgb();
    assert_eq!(rgb.map(|v| (v * 255.0).round() as i32), [255, 255, 131], "{rgb:?}");
    assert_eq!(DEFAULT_NOTE_COLOR.map(|v| (v * 65535.0).round() as u32), [0xffff, 0xffff, 0x8383], "the 16-bit components Photoshop saves");
    assert_eq!(n.popup, [108.0, 110.0, 348.0, 250.0], "icon + (8, 10), 240 × 140");
    // A note on the pasteboard keeps its popup there too.
    s.execute("notes.add", json!({"x": -178, "y": 300})).unwrap();
    assert_eq!(note(&s, 1).popup, [-170.0, 310.0, 70.0, 450.0]);
    // An explicit colour still wins.
    s.execute("notes.add", json!({"x": 10, "y": 10, "color": [0.25, 0.5, 1.0]})).unwrap();
    assert_eq!(note(&s, 2).color.to_rgb(), [0.25, 0.5, 1.0]);
}

#[test]
fn the_date_is_local_time_with_its_offset() {
    let at = |offset: i32| FixedOffset::east_opt(offset).unwrap().with_ymd_and_hms(2026, 11, 6, 20, 14, 55).unwrap();
    assert_eq!(pdf_date_at(&at(3600)), "D:20261106201455+01'00'");
    assert_eq!(pdf_date_at(&at(0)), "D:20261106201455+00'00'");
    assert_eq!(pdf_date_at(&at(-(5 * 3600 + 30 * 60))), "D:20261106201455-05'30'");
    // A new note's own date has that shape.
    let mut s = session(100, 100);
    s.execute("notes.add", json!({"x": 10, "y": 10})).unwrap();
    let d = note(&s, 0).modified;
    let b = d.as_bytes();
    assert!(d.len() == 23 && d.starts_with("D:") && matches!(b[16], b'+' | b'-') && b[19] == b'\'' && b[22] == b'\'', "{d}");
    assert!(d[2..16].chars().all(|c| c.is_ascii_digit()), "{d}");
}

#[test]
fn image_size_moves_the_icon_and_keeps_the_popup_size() {
    let mut s = session(600, 400);
    s.execute("notes.add", json!({"x": 100, "y": 99})).unwrap();
    let before = note(&s, 0);
    s.execute("image.imageSize", json!({"width": 300, "height": 200})).unwrap();
    let n = note(&s, 0);
    assert_eq!(n.position, [50.0, 49.5], "the icon point scales as today");
    let size = |p: [f64; 4]| [p[2] - p[0], p[3] - p[1]];
    assert_eq!(size(n.popup), size(before.popup), "240 × 140 still");
    assert_eq!([n.popup[0] - n.position[0], n.popup[1] - n.position[1]], [8.0, 10.0], "still at icon + (8, 10)");
}

#[test]
fn canvas_size_and_a_quarter_turn_keep_the_popup_size() {
    let mut s = session(600, 400);
    s.execute("notes.add", json!({"x": 100, "y": 100})).unwrap();
    s.execute("image.canvasSize", json!({"width": 400, "height": 300, "anchor": "center"})).unwrap();
    let n = note(&s, 0);
    assert_eq!(n.position, [0.0, 50.0], "moved by the canvas offset");
    assert_eq!(n.popup, [8.0, 60.0, 248.0, 200.0]);
    s.execute("image.imageRotation.90cw", json!({})).unwrap();
    let r = note(&s, 0);
    assert_eq!([r.popup[2] - r.popup[0], r.popup[3] - r.popup[1]], [240.0, 140.0], "a turn doesn't swap the popup's sides");
}

#[test]
fn a_mode_change_leaves_notes_alone() {
    let mut s = session(100, 100);
    s.execute("notes.add", json!({"x": 10, "y": 20})).unwrap();
    let before = note(&s, 0);
    s.execute("image.mode.cmyk", json!({})).unwrap();
    assert_eq!(note(&s, 0), before);
}

#[test]
fn a_turn_keeps_todays_popup_origin_and_the_popup_size() {
    // Where the popup sits after a turn wasn't measured (#11795), so it keeps today's origin (the
    // top-left of the turned rectangle) and only the size rule applies.
    let mut s = session(600, 400);
    s.execute("notes.add", json!({"x": 100, "y": 100})).unwrap();
    // The popup [108, 110, 348, 250] turned 90° clockwise on a 400 px tall canvas spans x 150..290,
    // y 108..348; it starts there and stays 240 × 140.
    s.execute("image.imageRotation.90cw", json!({})).unwrap();
    assert_eq!(note(&s, 0).popup, [150.0, 108.0, 390.0, 248.0]);
    // A flip keeps today's mirrored popup, which already has its size.
    let mut s = session(600, 400);
    s.execute("notes.add", json!({"x": 100, "y": 100})).unwrap();
    s.execute("image.imageRotation.flipCanvasHorizontal", json!({})).unwrap();
    assert_eq!(note(&s, 0).popup, [252.0, 110.0, 492.0, 250.0]);
}

#[test]
fn an_unset_popup_stays_unset_under_image_size() {
    // An all-zero popup means "none stored": PSD export writes its default popup for it, so Image
    // Size must not turn it into a 0 × 0 rectangle somewhere else.
    let mut s = session(600, 400);
    let doc = std::sync::Arc::make_mut(&mut s.active_mut().unwrap().doc);
    doc.notes.push(photocraft_doc::Note { position: [100.0, 100.0], ..Default::default() });
    s.execute("image.imageSize", json!({"width": 300, "height": 200})).unwrap();
    let n = note(&s, 0);
    assert_eq!(n.position, [50.0, 50.0]);
    assert_eq!(n.popup, [0.0; 4]);
}
