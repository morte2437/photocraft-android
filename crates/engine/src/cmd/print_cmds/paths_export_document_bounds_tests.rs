//! #13771: Paths to Illustrator's Document Bounds choice writes the file frame with no path (the
//! canvas in points as bounding box), and the menu entry is enabled for every open document,
//! with or without paths.

use super::*;
use photocraft_doc::{NamedPath, Path, Subpath};

const CMD: &str = "file.export.pathsToIllustrator";
const BEGIN: &str = "%Adobe_Photoshop_Path_Begin";

/// A 300 x 200 document at `dpi` without any path.
fn plain(dpi: f32) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 300, "height": 200, "background": "#ffffff", "name": "Bounds.psd"})).unwrap();
    s.edit("resolution", |d, _| {
        d.resolution_dpi = dpi;
        Ok(())
    })
    .unwrap();
    s
}

fn add_path(s: &mut Session, name: &str) {
    s.edit("paths", |d, _| {
        let tri = Subpath::polygon(&[(10.0, 10.0), (100.0, 10.0), (50.0, 90.0)]);
        d.paths.push(NamedPath { name: name.into(), path: Path::new(vec![tri]), psd_raw: None });
        Ok(())
    })
    .unwrap();
}

fn bounds(s: &mut Session) -> String {
    let r = s.execute(CMD, json!({"paths": "documentBounds"})).unwrap();
    assert_eq!(r["paths"], 0);
    r["ai"].as_str().unwrap().to_owned()
}

#[test]
fn document_bounds_writes_the_frame_without_a_path() {
    let mut s = plain(72.0);
    let ai = bounds(&mut s);
    assert!(ai.starts_with("%!PS-Adobe-2.0 EPSF-1.2\n"), "{ai}");
    assert!(ai.contains("%%BoundingBox: 0 0 300 200\n"), "{ai}");
    assert!(ai.contains("%%EndSetup\n%%PageTrailer\n%%Trailer\n%%EOF\n"), "nothing between setup and trailer: {ai}");
    assert!(!ai.contains(BEGIN));
}

#[test]
fn document_bounds_leaves_out_the_documents_paths() {
    let mut s = plain(72.0);
    add_path(&mut s, "Rect A");
    s.edit("work path", |d, _| {
        d.work_path = Some(Path::new(vec![Subpath::polygon(&[(1.0, 1.0), (9.0, 1.0), (5.0, 9.0)])]));
        Ok(())
    })
    .unwrap();
    let ai = bounds(&mut s);
    assert!(!ai.contains(BEGIN), "{ai}");
    assert!(!ai.contains(" m\n"), "no path operators: {ai}");
}

#[test]
fn the_bounding_box_is_the_canvas_in_points() {
    assert!(bounds(&mut plain(72.0)).contains("%%BoundingBox: 0 0 300 200\n"));
    assert!(bounds(&mut plain(300.0)).contains("%%BoundingBox: 0 0 72 48\n"));
    let mut s = plain(72.0);
    s.execute("image.canvasSize", json!({"width": 400, "height": 300, "anchor": "center"})).unwrap();
    assert!(bounds(&mut s).contains("%%BoundingBox: 0 0 400 300\n"));
}

#[test]
fn document_bounds_writes_the_file_and_reports_no_paths() {
    let mut s = plain(72.0);
    let dir = std::env::temp_dir().join(format!("photocraft-paths-bounds-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("Bounds.ai").to_string_lossy().replace('\\', "/");
    let r = s.execute(CMD, json!({"path": file, "paths": "documentBounds"})).unwrap();
    assert_eq!(r["paths"], 0);
    let written = std::fs::read_to_string(&file).unwrap();
    assert!(written.contains("%%BoundingBox: 0 0 300 200\n") && written.ends_with("%%EOF\n"), "{written}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_entry_is_enabled_with_or_without_paths() {
    let mut s = Session::new();
    assert!(!s.is_enabled(CMD), "greyed without a document");
    s.execute("file.new", json!({"width": 300, "height": 200})).unwrap();
    assert!(s.is_enabled(CMD), "a document without paths");
    add_path(&mut s, "Rect A");
    assert!(s.is_enabled(CMD), "a document with a path");
    let mut bitmap = Session::new();
    bitmap.execute("file.new", json!({"width": 300, "height": 200, "mode": "gray", "background": "#ffffff"})).unwrap();
    bitmap.execute("image.mode.bitmap", json!({})).unwrap();
    assert!(bitmap.is_enabled(CMD), "a Bitmap-mode document");
    assert_eq!(bitmap.execute(CMD, json!({"paths": "documentBounds"})).unwrap()["paths"], 0);
}

#[test]
fn a_path_name_that_does_not_exist_is_still_an_error() {
    let mut s = plain(72.0);
    add_path(&mut s, "Rect A");
    let err = s.execute(CMD, json!({"paths": "Nope"})).unwrap_err().to_string();
    assert!(err.contains("no paths match `Nope`"), "{err}");
    // The reference application's dropdown label is not a choice the command takes.
    assert!(s.execute(CMD, json!({"paths": "Document Bounds"})).is_err());
    // Without any path, All Paths and Work Path still match nothing (not measured: kept as today).
    let mut empty = plain(72.0);
    assert!(empty.execute(CMD, json!({"paths": "all"})).is_err());
    assert!(empty.execute(CMD, json!({"paths": "work"})).is_err());
}
