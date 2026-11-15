//! Regression coverage for Image Processor document sources and preservation of open work.
use crate::Session;
use photocraft_doc::Document;
use photocraft_geom::Rect;
use serde_json::{Value, json};
use std::sync::Arc;

/// Create a small document with explicit color and bit depth for export comparisons.
fn add(s: &mut Session, name: &str, depth: u32, background: &str) -> crate::Result<Value> {
    s.execute("file.new", json!({"name": name, "width": 40, "height": 20, "depth": depth, "background": background}))
}

/// Sample the same interior pixel before and after processing through the CPU compositor.
fn pixel(doc: &Document) -> [f32; 4] {
    photocraft_compose::render(doc, Rect::new(1, 1, 2, 2)).px[0]
}

/// Reimport a reported output so assertions examine encoded pixels and document metadata.
fn exported(result: &Value, index: usize) -> std::result::Result<Document, Box<dyn std::error::Error>> {
    let path = result["files"][index].as_str().ok_or("export result has no filename")?;
    Ok(photocraft_io::import(path, &std::fs::read(path)?)?.document)
}

/// Resizing must use unsaved pixels at every supported depth while leaving the source
/// snapshot, selected layers, history, dirty state and saved file intact.
#[test]
fn image_processor_current_uses_unsaved_edits_at_several_depths_without_changing_the_source() {
    for depth in [8, 16, 32] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("source.psd");
        let output = dir.path().join("processed");
        let mut s = Session::new();
        add(&mut s, "Other", depth, "#0000ff").unwrap();
        add(&mut s, "Current", depth, "#ff0000").unwrap();
        s.execute("file.saveACopy", json!({"path": path})).unwrap();
        let st = s.active_mut().unwrap();
        st.path = Some(path.to_string_lossy().into_owned());
        st.saved_revision = st.revision;
        let source_bytes = std::fs::read(&path).unwrap();
        s.execute("image.adjustments.invert", json!({})).unwrap();
        s.execute("select.all", json!({})).unwrap();
        let st = s.active().unwrap();
        let before = st.doc.clone();
        let history = st.history.past_len();
        let layer = st.active_layer;
        let selection = st.selected_layers();
        let active = s.active_index();
        assert!(st.is_dirty());
        let result =
            s.execute("file.scripts.imageProcessor", json!({"source": "current", "output": output, "format": "psd", "width": 20, "height": 20})).unwrap();
        assert_eq!(result["files"].as_array().unwrap().len(), 1, "{result}");
        assert!(result["errors"].as_array().unwrap().is_empty(), "{result}");
        let copy = exported(&result, 0).unwrap();
        assert_eq!((copy.size.width, copy.size.height), (20, 10));
        assert_eq!(copy.depth, s.active().unwrap().doc.depth);
        let px = pixel(&copy);
        assert!(px[0] < 0.01 && px[1] > 0.99 && px[2] > 0.99, "the unsaved cyan image is processed: {px:?}");
        let st = s.active().unwrap();
        assert!(Arc::ptr_eq(&st.doc, &before), "the source document remains the same snapshot");
        assert_eq!(st.history.past_len(), history);
        assert_eq!(st.active_layer, layer);
        assert_eq!(st.selected_layers(), selection);
        assert!(st.is_dirty());
        assert_eq!(s.active_index(), active);
        assert_eq!(s.documents().len(), 2);
        assert_eq!(std::fs::read(&path).unwrap(), source_bytes);
    }
}

/// Untitled documents need no source path, and omitted dimensions must preserve their size.
#[test]
fn image_processor_current_accepts_an_untitled_document() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = Session::new();
    add(&mut s, "Untitled-1", 8, "#ff0000").unwrap();
    let result = s.execute("file.scripts.imageProcessor", json!({"source": "current", "output": dir.path(), "format": "png"})).unwrap();
    assert_eq!(result["files"].as_array().unwrap().len(), 1, "{result}");
    assert_eq!(std::path::Path::new(result["files"][0].as_str().unwrap()), dir.path().join("Untitled-1.png").as_path());
    assert!(result["errors"].as_array().unwrap().is_empty(), "{result}");
    assert_eq!(exported(&result, 0).unwrap().size, s.active().unwrap().doc.size, "no dimensions means no resize");
}

/// A duplicate output name must preserve the first result and allow later documents to
/// finish without changing any open document or the active tab.
#[test]
fn image_processor_open_copies_every_document_and_reports_collisions_without_overwriting() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = Session::new();
    add(&mut s, "Same", 8, "#ff0000").unwrap();
    add(&mut s, "Same", 8, "#0000ff").unwrap();
    add(&mut s, "Other", 8, "#00ff00").unwrap();
    let before: Vec<_> = s.documents().iter().map(|st| (st.doc.clone(), st.history.past_len(), st.path.clone())).collect();
    let active = s.active_index();
    let result = s.execute("file.scripts.imageProcessor", json!({"source": "open", "output": dir.path(), "format": "png", "width": 20})).unwrap();
    assert_eq!(result["files"].as_array().unwrap().len(), 2, "{result}");
    assert_eq!(result["errors"].as_array().unwrap().len(), 1, "{result}");
    assert!(result["errors"][0]["error"].as_str().unwrap().contains("same output name"));
    let first = exported(&result, 0).unwrap();
    assert_eq!((first.size.width, first.size.height), (20, 10));
    assert!(pixel(&first)[0] > 0.99, "the first result stays red");
    assert!(pixel(&exported(&result, 1).unwrap())[1] > 0.99, "processing continues after the collision");
    assert_eq!(s.documents().len(), before.len());
    for (st, (doc, history, path)) in s.documents().iter().zip(&before) {
        assert!(Arc::ptr_eq(&st.doc, doc));
        assert_eq!(st.history.past_len(), *history);
        assert_eq!(&st.path, path);
    }
    assert_eq!(s.active_index(), active);
}

/// Exporting back to an open source path must report an error and preserve its disk bytes.
#[test]
fn image_processor_current_cannot_replace_an_open_source_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("source.png");
    let mut s = Session::new();
    add(&mut s, "Source", 8, "#ff0000").unwrap();
    s.execute("file.saveACopy", json!({"path": path})).unwrap();
    let st = s.active_mut().unwrap();
    st.path = Some(path.to_string_lossy().into_owned());
    st.saved_revision = st.revision;
    let bytes = std::fs::read(&path).unwrap();
    s.execute("image.adjustments.invert", json!({})).unwrap();
    let result = s.execute("file.scripts.imageProcessor", json!({"source": "current", "output": dir.path(), "format": "png"})).unwrap();
    assert!(result["files"].as_array().unwrap().is_empty(), "{result}");
    assert!(result["errors"][0]["error"].as_str().unwrap().contains("open source file"));
    assert_eq!(std::fs::read(path).unwrap(), bytes);
}

/// Missing documents, malformed source values, unsafe extensions and absent output paths
/// must fail without creating files.
#[test]
fn image_processor_open_sources_fail_gracefully_with_no_documents_or_bad_source_params() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = Session::new();
    for source in [json!("current"), json!("open"), json!("unknown"), json!(true), json!(7), json!([])] {
        assert!(s.execute("file.scripts.imageProcessor", json!({"source": source, "output": dir.path()})).is_err());
    }
    add(&mut s, "Untitled", 8, "#ffffff").unwrap();
    for format in ["", "../png", "png/escape", "png\\escape", "png\n"] {
        assert!(s.execute("file.scripts.imageProcessor", json!({"source": "current", "output": dir.path(), "format": format})).is_err());
    }
    assert!(s.execute("file.scripts.imageProcessor", json!({"source": "current"})).is_err());
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
}

/// A scratch copy must obey the initiating session's resize authorization and write
/// nothing when that nested command is denied.
#[test]
fn image_processor_current_preserves_authorization_for_resize_commands() {
    /// Reject only the nested resize so the test can distinguish inherited authorization.
    fn deny_resize(id: &str, _: &Value) -> crate::Result<()> {
        if id == "image.imageSize" { Err(crate::EngineError::Other("resize denied".into())) } else { Ok(()) }
    }
    let dir = tempfile::tempdir().unwrap();
    let mut s = Session::new();
    add(&mut s, "Untitled", 8, "#ffffff").unwrap();
    s.authorize = Some(deny_resize);
    let result = s.execute("file.scripts.imageProcessor", json!({"source": "current", "output": dir.path(), "width": 20})).unwrap();
    assert_eq!(result["files"], json!([]));
    assert!(result["errors"][0]["error"].as_str().unwrap().contains("resize denied"));
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    assert_eq!(s.active().unwrap().doc.size.width, 40);
}

/// Both document-source modes must export the displayed floating selection while keeping
/// the original floating parts, offset, revision and history unchanged.
#[test]
fn image_processor_open_sources_export_a_moved_selection_without_committing_it_in_the_original() {
    for source in ["current", "open"] {
        let dir = tempfile::tempdir().unwrap();
        let mut s = Session::new();
        add(&mut s, "Moved", 8, "#ff0000").unwrap();
        s.execute("select.rect", json!({"x": 0, "y": 0, "width": 4, "height": 4})).unwrap();
        s.execute("select.float", json!({"dx": 10, "dy": 0})).unwrap();
        let st = s.active().unwrap();
        let original = st.doc.clone();
        let history = st.history.past_len();
        let revision = st.revision;
        let floating = crate::float_cmds::floating(st).unwrap().clone();
        let expected = floating.parts.moved(&original, floating.offset.0, floating.offset.1).unwrap();
        assert_ne!(pixel(&expected), pixel(&original), "moving the selection changes the displayed source pixel");
        let result = s.execute("file.scripts.imageProcessor", json!({"source": source, "output": dir.path(), "format": "png"})).unwrap();
        assert!(result["errors"].as_array().unwrap().is_empty(), "{result}");
        assert_eq!(pixel(&exported(&result, 0).unwrap()), pixel(&expected));
        let st = s.active().unwrap();
        assert!(Arc::ptr_eq(&st.doc, &original));
        assert_eq!(st.history.past_len(), history);
        assert_eq!(st.revision, revision);
        let still_floating = crate::float_cmds::floating(st).unwrap();
        assert_eq!(still_floating.offset, floating.offset);
        assert!(Arc::ptr_eq(&still_floating.parts, &floating.parts));
    }
}

/// Same-named relative assets must resolve against each source document's folder and
/// survive PSD export as the corresponding embedded payload.
#[test]
fn image_processor_open_embeds_relative_links_from_each_documents_own_folder() {
    use photocraft_doc::{LayerContent, SmartSource};

    let dir = tempfile::tempdir().unwrap();
    let mut s = Session::new();
    let mut sources = Vec::new();
    for (name, color) in [("First", "#ff0000"), ("Second", "#0000ff")] {
        let folder = dir.path().join(name);
        std::fs::create_dir(&folder).unwrap();
        let asset = folder.join("asset.png");
        add(&mut s, name, 8, color).unwrap();
        s.execute("file.saveACopy", json!({"path": asset})).unwrap();
        sources.push(std::fs::read(&asset).unwrap());
        s.active_mut().unwrap().path = Some(folder.join(format!("{name}.pcraft")).to_string_lossy().into_owned());
        s.execute("file.placeLinked", json!({"path": asset})).unwrap();
        let layer = s.active().unwrap().active_layer.unwrap();
        s.edit("Relative link", |doc, _| {
            if let Some(LayerContent::Smart(smart)) = doc.layer_mut(layer).map(|layer| &mut layer.content) {
                smart.source = SmartSource::Linked { path: "asset.png".into() };
            }
            Ok(())
        })
        .unwrap();
    }
    let originals: Vec<_> = s.documents().iter().map(|state| state.doc.clone()).collect();
    let result = s.execute("file.scripts.imageProcessor", json!({"source": "open", "output": dir.path().join("processed"), "format": "psd"})).unwrap();
    assert!(result["errors"].as_array().unwrap().is_empty(), "{result}");
    assert_eq!(result["files"].as_array().unwrap().len(), sources.len());
    for (index, expected) in sources.iter().enumerate() {
        let copy = exported(&result, index).unwrap();
        let bytes = copy
            .layers
            .iter()
            .find_map(|layer| match &layer.content {
                LayerContent::Smart(smart) => match &smart.source {
                    SmartSource::Embedded { bytes, .. } => Some(bytes.as_slice()),
                    SmartSource::Linked { path } => photocraft_io::linked::embedded_payload(&copy.metadata, path).map(|(_, bytes)| bytes),
                },
                _ => None,
            })
            .expect("the exported PSD embeds its linked asset");
        assert_eq!(bytes, expected, "each document resolves asset.png from its own folder");
    }
    for (state, original) in s.documents().iter().zip(&originals) {
        assert!(Arc::ptr_eq(&state.doc, original));
    }
}

/// Untitled names are labels rather than paths: preserve the full name through the shared
/// sanitizer, keep the original document untouched and use PSD for the `same` format.
#[test]
fn image_processor_untitled_names_use_the_shared_filename_sanitizer() {
    for source in ["current", "open"] {
        for (format, extension) in [("png", "png"), ("same", "psd")] {
            for (name, safe_name) in [
                ("foo/bar", "foo_bar"),
                (r"foo\bar", "foo_bar"),
                ("../outside", "_outside"),
                ("../../outside", "_.._outside"),
                ("..", "layer"),
                ("CON", "CON_"),
                ("nul.v2", "nul_.v2"),
                ("trailing. ", "trailing"),
                ("chapter.v2", "chapter.v2"),
            ] {
                let dir = tempfile::tempdir().unwrap();
                let output = dir.path().join("processed");
                let mut s = Session::new();
                add(&mut s, name, 8, "#ff0000").unwrap();
                let original = s.active().unwrap().doc.clone();
                let result = s.execute("file.scripts.imageProcessor", json!({"source": source, "output": output, "format": format})).unwrap();
                assert!(result["errors"].as_array().unwrap().is_empty(), "{source}, {format}, {name:?}: {result}");
                assert_eq!(result["files"].as_array().unwrap().len(), 1, "{source}, {format}, {name:?}: {result}");
                assert_eq!(
                    std::path::Path::new(result["files"][0].as_str().unwrap()),
                    output.join(format!("{safe_name}.{extension}")).as_path(),
                    "{source}, {format}, {name:?}"
                );
                assert_eq!(exported(&result, 0).unwrap().size, original.size);
                let state = s.active().unwrap();
                assert!(Arc::ptr_eq(&state.doc, &original));
                assert_eq!(state.doc.name, name);
                assert!(state.path.is_none());
                assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1, "only the output folder is created");
            }
        }
    }
}
