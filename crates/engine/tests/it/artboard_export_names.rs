//! #6613: each artboard must have its own output even when names collide on disk.
#![cfg(not(target_arch = "wasm32"))]

use std::collections::HashSet;
use std::path::Path;

use photocraft_engine::Session;
use serde_json::json;

struct Board {
    id: u64,
    width: u32,
    color: [f32; 4],
}

fn fixture(depth: u32, names: &[&str]) -> (Session, Vec<Board>) {
    let mut s = Session::new();
    s.execute("file.new", json!({"name": "Board Export", "width": 96, "height": 32, "depth": depth})).unwrap();
    let mut boards = Vec::new();
    for (i, name) in names.iter().enumerate() {
        let width = 3 + i as u32;
        let color = if i % 2 == 0 { [1.0, 0.0, 0.0, 1.0] } else { [0.0, 0.0, 1.0, 1.0] };
        let result = s.execute("layer.new.artboard", json!({"name": name, "rect": [12 * i, 0, width, 2], "background": "custom", "color": color})).unwrap();
        boards.push(Board { id: result["layer"].as_u64().unwrap(), width, color });
    }
    (s, boards)
}

fn check_export(depth: u32, names: &[&str], prefix: Option<&str>, format: &str, selected: Option<&[usize]>) {
    let (mut s, boards) = fixture(depth, names);
    let dir = tempfile::tempdir().unwrap();
    let mut params = json!({"dir": dir.path(), "format": format});
    if let Some(prefix) = prefix {
        params["prefix"] = json!(prefix);
    }
    if let Some(selected) = selected {
        params["artboards"] = json!(selected.iter().map(|i| boards[*i].id).collect::<Vec<_>>());
    }
    let expected: Vec<_> = boards.iter().enumerate().rev().filter(|(i, _)| selected.is_none_or(|ids| ids.contains(i))).map(|(_, b)| b).collect();
    let before = photocraft_format::save_to_bytes(&s.active().unwrap().doc, &Default::default()).unwrap();
    let revision = s.active().unwrap().revision;
    let history = s.active().unwrap().history.past_len();
    let result = s.execute("file.export.artboardsToFiles", params).unwrap();
    let files: Vec<_> = result["files"].as_array().unwrap().iter().map(|f| f.as_str().unwrap()).collect();
    assert_eq!(files.len(), expected.len());
    // Also require uniqueness on filesystems that compare names without letter case.
    assert_eq!(files.iter().map(|f| f.to_lowercase()).collect::<HashSet<_>>().len(), files.len(), "export paths collide: {files:?}");
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), files.len(), "one physical output per returned path");
    for (file, board) in files.iter().zip(expected) {
        assert_eq!(Path::new(file).parent(), Some(dir.path()));
        let bytes = std::fs::read(file).unwrap();
        let doc = photocraft_io::import(file, &bytes).unwrap().document;
        assert_eq!((doc.size.width, doc.size.height), (board.width, 2), "wrong artboard in {file}");
        let px = photocraft_compose::flatten(&doc).px[0];
        for (actual, expected) in px.iter().zip(board.color) {
            assert!((actual - expected).abs() < 1e-3, "wrong pixels in {file}: {px:?}");
        }
    }
    let after = s.active().unwrap();
    assert_eq!(photocraft_format::save_to_bytes(&after.doc, &Default::default()).unwrap(), before);
    assert_eq!(after.revision, revision, "export must not edit the source document");
    assert_eq!(after.history.past_len(), history);
}

#[test]
fn duplicate_names_preserve_each_board_at_every_depth_and_prefix() {
    for depth in [8, 16, 32] {
        for prefix in [None, Some(""), Some("Launch/Run")] {
            check_export(depth, &["Artboard 1", "Artboard 1"], prefix, "png", None);
        }
    }
}

#[test]
fn sanitized_and_empty_names_preserve_each_tiff() {
    check_export(16, &["Hero/1", "Hero:1", "Hero?1", ".", " ", "layer"], Some(""), "tiff", None);
}

#[test]
fn case_only_and_index_like_names_remain_distinct() {
    check_export(8, &["Logo", "logo", "Logo_0001", "0000_Logo", "Лого", "лого"], Some("site"), "png", None);
}

#[test]
fn selected_duplicate_names_keep_their_psd_contents_in_stack_order() {
    check_export(16, &["Same", "Skip", "Same"], Some("chosen"), "psd", Some(&[0, 2]));
}

#[test]
fn distinct_names_keep_their_corresponding_contents() {
    check_export(8, &["Red", "Blue"], None, "png", None);
}

#[test]
fn invalid_export_params_fail_before_creating_files() {
    let (mut s, _) = fixture(8, &["One", "Two"]);
    let dir = tempfile::tempdir().unwrap();
    for params in [json!({"dir": 3}), json!({"dir": dir.path(), "artboards": []}), json!({"dir": dir.path(), "artboards": [u64::MAX]})] {
        assert!(s.execute("file.export.artboardsToFiles", params).is_err());
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }
    assert!(Session::new().execute("file.export.artboardsToFiles", json!({"dir": dir.path()})).is_err());
}
