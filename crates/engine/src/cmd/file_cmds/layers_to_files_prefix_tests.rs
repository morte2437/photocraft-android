//! #13960: an explicitly empty Layers to Files prefix stays empty; sanitizing stays unchanged.

use super::{Session, export_layers_report_with, export_layers_with, json};

fn session(depth: u32) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"name": "project.v2.psd", "width": 4, "height": 3, "depth": depth})).unwrap();
    s.execute("layer.new.layer", json!({"name": "Type"})).unwrap();
    s.execute("edit.fill", json!({"color": "#406080"})).unwrap();
    s
}

#[test]
#[cfg(not(target_arch = "wasm32"))]
fn empty_prefix_writes_underscore_names_at_each_depth() {
    for depth in [8, 16, 32] {
        let dir = tempfile::tempdir().unwrap();
        let mut s = session(depth);
        let result = s.execute("file.export.layersToFiles", json!({"dir": dir.path(), "prefix": ""})).unwrap();
        let files = result["files"].as_array().unwrap();
        assert_eq!(files.len(), 2);
        for (file, expected) in files.iter().zip(["_0000_Type.png", "_0001_Background.png"]) {
            let path = std::path::Path::new(file.as_str().unwrap());
            assert_eq!(path.file_name().unwrap(), expected, "depth {depth}");
            assert_eq!(photocraft_io::import(expected, &std::fs::read(path).unwrap()).unwrap().document.size, s.active().unwrap().doc.size);
        }
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 2);
    }
}

#[test]
fn automation_shared_export_path_preserves_the_empty_prefix() {
    let s = session(8);
    let params = json!({"prefix": ""});
    let mut written = Vec::new();
    let report = export_layers_report_with(&s, &params, |name, bytes| {
        written.push(name.to_string());
        assert_eq!(photocraft_io::import(name, bytes).unwrap().document.size, s.active().unwrap().doc.size);
        Ok(())
    })
    .unwrap();
    assert_eq!(report.files, ["_0000_Type.png", "_0001_Background.png"]);
    assert_eq!(written, report.files);
    assert!(report.skipped.is_empty());
    assert_eq!(export_layers_with(&s, &params, |_, _| Ok(())).unwrap(), report.files);
}

#[test]
fn omitted_and_nonempty_prefixes_keep_their_defaults_and_sanitizing() {
    let s = session(8);
    for (params, prefix) in [
        (json!({}), "project.v2"),
        (json!({"prefix": "card-1"}), "card-1"),
        (json!({"prefix": "..."}), "layer"),
        (json!({"prefix": "   "}), "layer"),
        (json!({"prefix": "/?*"}), "___"),
        (json!({"prefix": "a/b"}), "a_b"),
    ] {
        let files = export_layers_with(&s, &params, |_, _| Ok(())).unwrap();
        assert_eq!(files, [format!("{prefix}_0000_Type.png"), format!("{prefix}_0001_Background.png")], "{params}");
    }
}

#[test]
fn empty_prefix_keeps_layer_name_fallbacks_and_sanitizing() {
    for (name, sanitized) in [("", "layer"), ("...", "layer"), ("   ", "layer"), ("/?*", "___")] {
        let mut s = session(8);
        s.execute("layer.setProps", json!({"name": name})).unwrap();
        let files = export_layers_with(&s, &json!({"prefix": ""}), |_, _| Ok(())).unwrap();
        assert_eq!(files, [format!("_0000_{sanitized}.png"), "_0001_Background.png".into()], "layer name {name:?}");
    }
}
