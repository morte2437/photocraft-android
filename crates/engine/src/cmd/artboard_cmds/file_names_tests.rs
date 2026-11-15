//! Artboards to Files names each file `<prefix><artboard name>.<ext>` as the reference application
//! does (#13633): no `_` or index is added, a space becomes `-` except in PSD, a character a file
//! name can't hold becomes `_`, and only a later board whose name is already taken gets a suffix.

use super::*;

/// The issue's measured boards, bottom to top in the Layers panel: name and `[x, y, w, h]`.
const MEASURED: [(&str, [u32; 4]); 3] = [("Red Board", [0, 0, 200, 150]), ("Blue Board", [120, 30, 200, 200]), ("Empty Board", [600, 0, 100, 100])];

/// The issue's test document: 800 x 400, RGB, 8 bit, transparent, unsaved and named
/// `abfiles-test`, with `boards` added bottom to top.
fn session(boards: &[(&str, [u32; 4])]) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"name": "abfiles-test", "width": 800, "height": 400, "depth": 8, "background": "transparent"})).unwrap();
    for (name, rect) in boards {
        s.execute("layer.new.artboard", json!({"name": name, "rect": rect, "background": "transparent"})).unwrap();
    }
    s
}

/// A session with one 10 x 10 board per name, bottom to top, for the name-collision cases.
fn session_named(names: &[&str]) -> Session {
    let boards: Vec<(&str, [u32; 4])> = names.iter().zip(0u32..).map(|(name, i)| (*name, [i * 20, 0, 10, 10])).collect();
    session(&boards)
}

/// The id of the board named `name`.
fn board(s: &Session, name: &str) -> u64 {
    s.active().unwrap().doc.artboards().into_iter().find(|b| b.1 == name).unwrap().0.0
}

/// Runs Artboards to Files into an empty folder with `params` and returns the file names it
/// reports, in export order. Checks that exactly those files were written, into that folder, and
/// that the document and its History were left alone.
fn export(s: &mut Session, mut params: Value) -> Vec<String> {
    let dir = tempfile::tempdir().unwrap();
    params["dir"] = json!(dir.path());
    let (revision, history) = (s.active().unwrap().revision, s.active().unwrap().history.past_len());
    let result = s.execute("file.export.artboardsToFiles", params).unwrap();
    let paths: Vec<&str> = result["files"].as_array().unwrap().iter().map(|f| f.as_str().unwrap()).collect();
    let names: Vec<String> = paths.iter().map(|f| std::path::Path::new(f).file_name().unwrap().to_string_lossy().into_owned()).collect();
    for (path, name) in paths.iter().zip(&names) {
        assert_eq!(*path, join(&dir.path().to_string_lossy(), name));
        assert!(std::fs::metadata(path).unwrap().len() > 0, "{path} is empty");
    }
    let mut written: Vec<String> = std::fs::read_dir(dir.path()).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
    written.sort();
    let mut reported = names.clone();
    reported.sort();
    assert_eq!(written, reported, "one file per reported name, nothing else");
    assert_eq!(s.active().unwrap().revision, revision, "export must not edit the document");
    assert_eq!(s.active().unwrap().history.past_len(), history, "export must not add a History step");
    names
}

#[test]
fn png_names_are_the_document_name_then_the_board_name_with_dashes() {
    let mut s = session(&MEASURED);
    // Top of the Layers panel first, as before.
    assert_eq!(export(&mut s, json!({})), ["abfiles-testEmpty-Board.png", "abfiles-testBlue-Board.png", "abfiles-testRed-Board.png"]);
}

#[test]
fn a_prefix_is_put_straight_before_the_board_name() {
    let mut s = session(&MEASURED);
    let blue = board(&s, "Blue Board");
    assert_eq!(export(&mut s, json!({"prefix": "pre_", "artboards": [blue]})), ["pre_Blue-Board.png"]);
    assert_eq!(export(&mut s, json!({"prefix": "", "artboards": [blue]})), ["Blue-Board.png"]);
    // A prefix's own spaces and unsafe characters follow the same rules as the board name's.
    assert_eq!(export(&mut s, json!({"prefix": "Launch/Run 2 ", "artboards": [blue]})), ["Launch_Run-2-Blue-Board.png"]);
}

#[test]
fn characters_a_file_name_cannot_hold_become_underscores() {
    let mut s = session(&[MEASURED[0], MEASURED[1], ("Emp ty/x:y*?", MEASURED[2].1)]);
    let renamed = board(&s, "Emp ty/x:y*?");
    assert_eq!(export(&mut s, json!({"artboards": [renamed]})), ["abfiles-testEmp-ty_x_y__.png"]);
    assert_eq!(export(&mut s, json!({"artboards": [renamed], "format": "psd"})), ["abfiles-testEmp ty_x_y__.psd"]);
}

#[test]
fn psd_names_keep_their_spaces() {
    let mut s = session(&MEASURED);
    assert_eq!(export(&mut s, json!({"format": "psd"})), ["abfiles-testEmpty Board.psd", "abfiles-testBlue Board.psd", "abfiles-testRed Board.psd"]);
}

#[test]
fn jpeg_bmp_targa_and_tiff_names_follow_the_png_rule() {
    let mut s = session(&MEASURED);
    let red = board(&s, "Red Board");
    for format in ["jpg", "jpeg", "bmp", "tga", "tif", "tiff", ".PNG"] {
        let ext = format.trim_start_matches('.').to_ascii_lowercase();
        assert_eq!(export(&mut s, json!({"format": format, "artboards": [red]})), [format!("abfiles-testRed-Board.{ext}")], "{format}");
    }
}

#[test]
fn a_taken_name_gets_a_suffix_on_the_later_board_only() {
    // Exported top first: "A" keeps its name, the second "A" can't take "A-2" (the next board's
    // own name) and gets "A-3", and the board named "A-2" keeps its name.
    let mut s = session_named(&["A-2", "A", "A"]);
    assert_eq!(export(&mut s, json!({"prefix": "p"})), ["pA.png", "pA-3.png", "pA-2.png"]);
    // Letter case alone doesn't make a name free: "a" comes first and keeps it.
    let mut s = session_named(&["A", "a", "A-2"]);
    assert_eq!(export(&mut s, json!({"prefix": "p"})), ["pA-2.png", "pa.png", "pA-3.png"]);
}

#[test]
fn names_that_only_meet_once_written_get_a_suffix() {
    // `/` and `:` both become `_`.
    let mut s = session_named(&["x/1", "x:1"]);
    assert_eq!(export(&mut s, json!({"prefix": "p"})), ["px_1.png", "px_1-2.png"]);
    // A space becomes `-` in a PNG name, so these meet there, but not in a PSD's.
    let mut s = session_named(&["Red Board", "Red-Board"]);
    assert_eq!(export(&mut s, json!({"prefix": "p"})), ["pRed-Board.png", "pRed-Board-2.png"]);
    assert_eq!(export(&mut s, json!({"prefix": "p", "format": "psd"})), ["pRed-Board.psd", "pRed Board.psd"]);
}

#[test]
fn file_stems_without_a_session() {
    assert_eq!(
        artboard_file_stems("abfiles-test", &["Red Board", "red board", "Red Board-2"], true),
        ["abfiles-testRed-Board", "abfiles-testred-board-3", "abfiles-testRed-Board-2"]
    );
    assert_eq!(artboard_file_stems("", &["Red Board", "Red  Board"], false), ["Red Board", "Red  Board"]);
    // NTFS compares names in upper case, where `σ` and final `ς` are both `Σ`.
    assert_eq!(artboard_file_stems("", &["σ", "ς", "Σ-2"], true), ["σ", "ς-3", "Σ-2"]);
    // The Kelvin sign is its own upper case, but macOS folds it into `K`.
    assert_eq!(artboard_file_stems("", &["K", "\u{212A}"], true), ["K", "\u{212A}-2"]);
    assert!(artboard_file_stems("doc", &[], true).is_empty());
}
