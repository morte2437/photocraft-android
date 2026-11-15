use std::path::{Path, PathBuf};
use std::sync::Arc;

use photocraft_doc::{ColorMode, Document};
use serde_json::{Value, json};

use crate::Session;

const COMMAND: &str = "file.scripts.imageProcessor";

fn document(s: &mut Session, name: &str, depth: u32) -> crate::Result<()> {
    s.execute("file.new", json!({"name": name, "width": 40, "height": 20, "depth": depth, "resolution": 300, "background": "#b05980"}))?;
    s.execute("layer.new.layer", json!({"name": "Artwork"}))?;
    Ok(())
}

fn paths(result: &Value) -> Result<Vec<PathBuf>, serde_json::Error> {
    serde_json::from_value(result["files"].clone())
}

fn reopen(path: &Path) -> Result<Document, Box<dyn std::error::Error>> {
    Ok(photocraft_io::import(&path.to_string_lossy(), &std::fs::read(path)?)?.document)
}

#[test]
fn one_run_reopens_three_independently_sized_formats() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = Session::new();
    document(&mut s, "Source", 16).unwrap();
    s.execute("edit.assignProfile", json!({"profile": "display-p3"})).unwrap();
    s.execute("select.all", json!({})).unwrap();
    let before = s.active().unwrap();
    let (doc, target, history, revision) = (before.doc.clone(), before.layer_target(), before.history.entries(), before.revision);

    let result = s
        .execute(
            COMMAND,
            json!({
                "source": "current", "output": dir.path(),
                "jpeg": {"quality": 7.6, "convertToSrgb": true, "resize": true, "width": 20, "height": 18},
                "psd": {"resize": true, "width": 10, "height": 10},
                "tiff": {"lzw": true, "resize": true, "width": 100, "height": 100}
            }),
        )
        .unwrap();
    assert_eq!(result["errors"], json!([]), "{result}");
    let files = paths(&result).unwrap();
    assert_eq!(files, ["Source.jpg", "Source.psd", "Source.tif"].map(|name| dir.path().join(name)));
    let jpeg = reopen(&files[0]).unwrap();
    let psd = reopen(&files[1]).unwrap();
    let tiff = reopen(&files[2]).unwrap();
    let tiff_info = photocraft_codecs::tiff_info(&std::fs::read(&files[2]).unwrap()).unwrap();
    assert_eq!(tiff_info.pages[0].compression, 5, "TIFF uses LZW compression");
    assert_eq!((jpeg.size.width, jpeg.size.height), (20, 10));
    assert_eq!((psd.size.width, psd.size.height), (10, 5));
    assert_eq!(tiff.size, doc.size);
    assert_eq!(jpeg.depth, photocraft_color::SampleType::U8);
    assert_eq!(jpeg.layers.len(), 1);
    assert_eq!(jpeg.mode, ColorMode::Rgb);
    assert!(jpeg.icc_profile.is_some());
    assert_ne!(jpeg.icc_profile, doc.icc_profile);
    assert_eq!(crate::color_cmds::document_profile(&jpeg).description, craft_cms::Builtin::Srgb.profile().description);
    for output in [psd, tiff] {
        assert_eq!(output.depth, doc.depth);
        assert_eq!(output.mode, doc.mode);
        assert_eq!(output.resolution_dpi, 300.0);
        assert_eq!(output.icc_profile, doc.icc_profile);
        assert_eq!(output.layers.iter().map(|layer| &layer.name).collect::<Vec<_>>(), doc.layers.iter().map(|layer| &layer.name).collect::<Vec<_>>());
    }
    let after = s.active().unwrap();
    assert!(Arc::ptr_eq(&after.doc, &doc));
    assert_eq!(after.layer_target(), target);
    assert_eq!(after.history.entries(), history);
    assert_eq!(after.revision, revision);
}

#[test]
fn every_nonempty_format_combination_processes_all_open_documents() {
    let mut s = Session::new();
    document(&mut s, "First", 8).unwrap();
    document(&mut s, "Second", 8).unwrap();
    let formats = [("jpeg", "jpg"), ("psd", "psd"), ("tiff", "tif")];
    for mask in 1..8 {
        let dir = tempfile::tempdir().unwrap();
        let mut params = json!({"source": "open", "output": dir.path()});
        let mut expected = Vec::new();
        for (bit, (key, ext)) in formats.iter().enumerate() {
            let enabled = (mask & (1 << bit)) != 0;
            params[*key] = json!({"enabled": enabled});
            if enabled {
                expected.extend(["First", "Second"].map(|name| dir.path().join(format!("{name}.{ext}"))));
            }
        }
        let result = s.execute(COMMAND, params).unwrap();
        assert_eq!(result["errors"], json!([]), "{result}");
        assert_eq!(paths(&result).unwrap(), expected, "mask {mask}");
    }
}

#[test]
fn invalid_settings_fail_before_any_format_writes() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = Session::new();
    document(&mut s, "Source", 8).unwrap();
    for settings in [
        json!({"jpeg": false, "psd": {"enabled": false}, "tiff": null}),
        json!({"jpeg": {}, "psd": 7}),
        json!({"jpeg": {}, "tiff": {"lzw": "true"}}),
        json!({"jpeg": {"quality": 0}}),
        json!({"jpeg": {"quality": "5"}}),
        json!({"jpeg": {}, "psd": {"resize": true, "width": 30001}}),
        json!({"jpeg": {}, "tiff": {"resize": true, "height": 0}}),
    ] {
        let mut params = settings;
        params["source"] = json!("current");
        params["output"] = json!(dir.path().join("out"));
        assert!(s.execute(COMMAND, params.clone()).is_err(), "{params}");
        assert!(!dir.path().join("out").exists());
    }
}

#[test]
fn folder_errors_identify_the_format_and_do_not_replace_sources() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = Session::new();
    document(&mut s, "Source", 8).unwrap();
    let source = dir.path().join("source.psd");
    s.execute("file.saveACopy", json!({"path": source})).unwrap();
    let original = std::fs::read(&source).unwrap();
    let result = s
        .execute(
            COMMAND,
            json!({
                "input": [source], "output": dir.path(), "jpeg": {}, "psd": {}, "tiff": {"enabled": false}
            }),
        )
        .unwrap();
    assert_eq!(paths(&result).unwrap(), [dir.path().join("source.jpg")]);
    assert_eq!(result["errors"].as_array().unwrap().len(), 1);
    assert_eq!(result["errors"][0]["format"], "psd");
    assert_eq!(std::fs::read(&source).unwrap(), original);
}

#[test]
fn legacy_format_and_resize_parameters_still_work() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = Session::new();
    document(&mut s, "Source", 8).unwrap();
    let result = s
        .execute(
            COMMAND,
            json!({
                "source": "current", "output": dir.path(), "format": "png", "width": 20
            }),
        )
        .unwrap();
    assert_eq!(result["errors"], json!([]));
    let files = paths(&result).unwrap();
    assert_eq!(files, [dir.path().join("Source.png")]);
    let png = reopen(&files[0]).unwrap();
    assert_eq!((png.size.width, png.size.height), (20, 10));
}
