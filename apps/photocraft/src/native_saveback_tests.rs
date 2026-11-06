//! Opt-in local acceptance. All artwork paths come from the test environment; originals are read-only.

use super::{native, vector_conversion};
use base64::Engine as _;
use photocraft_doc::{Document, LayerContent, LayerId, SmartSource};
use photocraft_ui_egui::{ExternalSvgSession, ExternalVectorSource};
use serde_json::{Value, json};
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

/// Exercise the real CLI's same-format Save, the native watcher and PhotoCraft's transaction.
#[test]
#[ignore = "set VECTORCRAFT_TEST_EDITOR and optionally PHOTO_NATIVE_LOGO_PSD/PHOTO_NATIVE_CONTAINER"]
#[allow(clippy::cognitive_complexity, reason = "one opt-in walk through a real editor, step by step")]
fn original_format_real_editor_save_watcher_and_psd_roundtrip() {
    let editor = std::env::var_os("VECTORCRAFT_TEST_EDITOR").expect("VECTORCRAFT_TEST_EDITOR");
    let editor = Path::new(&editor);
    let mut photo = photocraft_engine::Session::new();
    let vector_path = std::env::var_os("PHOTO_NATIVE_VECTOR");
    let vector_original = vector_path.as_ref().map(|path| std::fs::read(path).unwrap());
    let original_path = std::env::var_os("PHOTO_NATIVE_LOGO_PSD");
    let original = original_path.as_ref().map(|path| std::fs::read(path).unwrap());
    let mut parent_before = None;
    let id;
    if let Some(bytes) = &original {
        photo.add_document(photocraft_io::import("Logo.psd", bytes).unwrap().document, None);
        if let Ok(name) = std::env::var("PHOTO_NATIVE_CONTAINER") {
            parent_before = Some(photo.active().unwrap().doc.clone());
            let layer = find_source(&photo.active().unwrap().doc, &name).expect("nested logo container");
            photo.execute("layer.smartObjects.editContents", json!({"layer":layer.0})).unwrap();
        }
        let name = std::env::var("PHOTO_NATIVE_SOURCE").unwrap_or("Vector Smart Object.ai".into());
        id = find_source(&photo.active().unwrap().doc, &name).expect("visible embedded vector");
    } else {
        photo.execute("file.new", json!({"width":32,"height":24})).unwrap();
        let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" width="8" height="6"><rect x="1" y="1" width="3" height="2" fill="red"/></svg>"#;
        let placed = photocraft_engine::file_cmds::place_bytes(&mut photo, "Artwork.svg", svg.to_vec(), None, &json!({"smartObject":true})).unwrap();
        id = LayerId(placed["layer"].as_u64().unwrap());
        // Generate native sources with our writer, or use a read-only local vector fixture.
        let (format, bytes) = if let Some(bytes) = &vector_original {
            let extension = Path::new(vector_path.as_ref().unwrap()).extension().unwrap().to_str().unwrap().to_ascii_lowercase();
            (extension, bytes.clone())
        } else {
            let owned = native::OwnedSvg::create(svg).unwrap();
            let format = std::env::var("PHOTO_NATIVE_FORMAT").unwrap_or("ai".into());
            let path = owned.root.join(format!("Input.{format}"));
            run_commands(
                editor,
                &owned.root,
                vec![
                    ("document.open", json!({"path":owned.file})),
                    ("document.export", json!({"path":path,"format":format,"preserveEditing":true,"useArtboards":true})),
                ],
            );
            (format, std::fs::read(path).unwrap())
        };
        let state = photo.active_mut().unwrap();
        let LayerContent::Smart(sm) = &mut Arc::make_mut(&mut state.doc).layer_mut(id).unwrap().content else { panic!("smart") };
        sm.source = SmartSource::Embedded { file_name: format!("Artwork.{format}"), bytes: Arc::new(bytes) };
    }
    let handoff = photo.execute("layer.smartObjects.exportSvg", json!({"layer":id.0})).unwrap();
    let source = ExternalVectorSource {
        bytes: Arc::new(base64::engine::general_purpose::STANDARD.decode(handoff["sourceBase64"].as_str().unwrap()).unwrap()),
        extension: handoff["extension"].as_str().unwrap().into(),
        size: serde_json::from_value(handoff["sourceSize"].clone()).unwrap(),
    };
    let before = photo.active().unwrap().doc.clone();
    let (_send, stop) = mpsc::channel();
    let (owned, initial) = vector_conversion::prepare(editor, &source, &stop, true).unwrap();
    assert_eq!(owned.file.extension().unwrap().to_str().unwrap(), source.extension);
    assert!(photo.active().unwrap().doc == before, "preparation must not change the embedded original");
    let file = owned.file.clone();
    let root = owned.root.clone();
    let mut watcher = native::start_native_test(owned, initial.clone(), editor.to_path_buf(), source.clone()).unwrap();
    std::thread::sleep(Duration::from_millis(300));
    assert!(watcher.poll_vector().is_none(), "opening must not accept the regenerated private AI working copy");
    // A small visible edit, ordinary Save to the already-open file, no conversion to SVG.
    run_commands(
        editor,
        &root,
        vec![
            ("document.open", json!({"path":file})),
            ("paint.setFill", json!({"color":"#00ff00"})),
            ("shape.rectangle", json!({"x":2,"y":2,"width":3,"height":2})),
            ("file.save", json!({})),
        ],
    );
    let saved = std::fs::read(&file).unwrap();
    eprintln!("Native save: format={}, bytes={}", source.extension, saved.len());
    assert!(saved != initial, "Save did not update the native working file");
    assert!(photocraft_io::vector_preview::is_native_source(&saved));
    assert!(!photocraft_io::svg::is_svg(&saved));
    let deadline = Instant::now() + Duration::from_secs(90);
    let snapshot = loop {
        assert!(Instant::now() < deadline, "saved native vector never reached watcher");
        if let Some(result) = watcher.poll_vector() {
            break result.unwrap();
        }
        std::thread::sleep(Duration::from_millis(25));
    };
    assert!(snapshot.bytes == saved, "watcher snapshot differs from saved native bytes");
    let params = json!({"document":handoff["document"],"layer":id.0,"contentsId":handoff["contentsId"],"fingerprint":handoff["fingerprint"],"extension":source.extension,"sourceBase64":base64::engine::general_purpose::STANDARD.encode(&saved),"previewSvg":String::from_utf8(snapshot.preview_svg.unwrap()).unwrap()});
    assert_eq!(photo.execute("layer.smartObjects.updateVectorSource", params).unwrap()["updated"], true);
    let after = photo.active().unwrap().doc.clone();
    let LayerContent::Smart(previous) = &before.layer(id).unwrap().content else { panic!("smart") };
    let LayerContent::Smart(changed) = &after.layer(id).unwrap().content else { panic!("smart") };
    assert_eq!(changed.transform, previous.transform);
    assert_eq!(changed.perspective, previous.perspective);
    assert_eq!(changed.warp, previous.warp);
    assert_eq!(changed.smart_filters, previous.smart_filters);
    assert!(embedded(&after, changed).unwrap().1 == saved, "native source was replaced with different bytes");
    assert!(photo.undo());
    assert!(photo.active().unwrap().doc == before, "Undo did not restore original source state");
    assert!(photo.redo());
    assert!(photo.active().unwrap().doc == after, "Redo did not restore saved native source state");
    let output = photocraft_io::export(&after, "Roundtrip.psd", &Default::default()).unwrap();
    let reopened = photocraft_io::import("Roundtrip.psd", &output.bytes).unwrap().document;
    let layer = find_source(&reopened, &embedded(&after, changed).unwrap().0).unwrap();
    let LayerContent::Smart(sm) = &reopened.layer(layer).unwrap().content else { panic!("smart") };
    assert!(embedded(&reopened, sm).unwrap().1 == saved, "PSD reopen did not retain the saved native bytes");
    assert!(sm.vector_preview == changed.vector_preview, "PSD did not retain the source-bound preview");
    assert!(
        sm.transform.m.iter().zip(changed.transform.m).all(|(a, b)| (a - b).abs() < 1e-9),
        "PSD corner reconstruction changed placement beyond floating-point tolerance"
    );
    assert!(photocraft_engine::smart_cmds::render(&reopened, sm).unwrap().as_ref() == changed.cache.as_ref(), "reopened native preview changed appearance");
    if let Some(parent_before) = parent_before {
        photo.execute("layer.smartObjects.saveContents", json!({})).unwrap();
        photo.set_active(0);
        let parent_after = photo.active().unwrap().doc.clone();
        assert!(photo.undo());
        assert!(photo.active().unwrap().doc == parent_before, "parent Undo changed original content");
        assert!(photo.redo());
        assert!(photo.active().unwrap().doc == parent_after, "parent Redo changed saved content");
        let output = photocraft_io::export(&parent_after, "Roundtrip.psd", &Default::default()).unwrap();
        let reopened = photocraft_io::import("Roundtrip.psd", &output.bytes).unwrap().document;
        assert!(contains_source(&reopened, &saved, 0), "parent PSD lost its nested native vector");
    }
    if let Some(path) = vector_path {
        assert!(std::fs::read(path).unwrap() == vector_original.unwrap(), "original vector changed")
    }
    if let Some(path) = original_path {
        assert!(std::fs::read(path).unwrap() == original.unwrap(), "original PSD changed")
    }
}

fn run_commands(editor: &Path, root: &Path, commands: Vec<(&str, Value)>) {
    let cli = editor.parent().unwrap().join(if cfg!(windows) { "vectorcraft-cli.exe" } else { "vectorcraft-cli" });
    let output_path = root.join("TestResponses.jsonl");
    let output = std::fs::File::create(&output_path).unwrap();
    let mut child = Command::new(cli)
        .args(["mcp", "--headless", "--automation-read-root"])
        .arg(root)
        .arg("--automation-write-root")
        .arg(root)
        .stdin(Stdio::piped())
        .stdout(output)
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut input = child.stdin.take().unwrap();
    let count = commands.len();
    for (index, (command, params)) in commands.into_iter().enumerate() {
        writeln!(
            input,
            "{}",
            json!({"jsonrpc":"2.0","id":index+1,"method":"tools/call","params":{"name":"run_command","arguments":{"command":command,"params":params}}})
        )
        .unwrap();
    }
    drop(input);
    let deadline = Instant::now() + Duration::from_secs(90);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            assert!(status.success());
            break;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("editor test timed out")
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    let responses = std::fs::read_to_string(output_path).unwrap();
    for id in 1..=count {
        let response = responses.lines().filter_map(|line| serde_json::from_str::<Value>(line).ok()).find(|r| r["id"] == id).unwrap();
        assert!(response.get("error").is_none() && response["result"]["isError"] == false, "editor command {id} failed");
    }
}

fn embedded<'a>(doc: &'a Document, sm: &'a photocraft_doc::SmartObject) -> Option<(String, &'a [u8])> {
    match &sm.source {
        SmartSource::Embedded { file_name, bytes } => Some((file_name.clone(), bytes)),
        SmartSource::Linked { path } => photocraft_io::linked::embedded_payload(&doc.metadata, path),
    }
}
fn find_source(doc: &Document, name: &str) -> Option<LayerId> {
    doc.walk().into_iter().find_map(|(path, _, layer)| {
        if !doc.shown_at(&path) {
            return None;
        }
        let LayerContent::Smart(sm) = &layer.content else { return None };
        embedded(doc, sm).filter(|(file, _)| file == name).map(|_| layer.id)
    })
}

fn contains_source(doc: &Document, saved: &[u8], depth: usize) -> bool {
    if depth > 4 {
        return false;
    }
    doc.walk().into_iter().any(|(_, _, layer)| {
        let LayerContent::Smart(sm) = &layer.content else { return false };
        let Some((name, bytes)) = embedded(doc, sm) else { return false };
        bytes == saved
            || ((bytes.starts_with(b"8BPS") || photocraft_format::is_pcraft(bytes))
                && photocraft_io::import(&name, bytes).is_ok_and(|child| contains_source(&child.document, saved, depth + 1)))
    })
}
