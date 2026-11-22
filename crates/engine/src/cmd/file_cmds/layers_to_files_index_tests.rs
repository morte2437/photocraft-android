//! Layers to Files: a layer skipped by Visible Layers Only keeps its number (#13959).

use super::*;

/// Layers top to bottom: `Top`, `Hidden` (hidden), `Blue`, `Background`.
fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 4, "height": 4})).unwrap();
    for name in ["Blue", "Hidden", "Top"] {
        s.execute("layer.new.layer", json!({"name": name})).unwrap();
        if name == "Hidden" {
            s.execute("layer.setProps", json!({"visible": false})).unwrap();
        }
    }
    s
}

fn export(s: &Session, visible_only: bool) -> LayerExportReport {
    export_layers_report_with(s, &json!({"prefix": "doc", "visibleOnly": visible_only}), |_, _| Ok(())).unwrap()
}

#[test]
fn visible_only_leaves_the_hidden_layers_number_unused() {
    let report = export(&session(), true);
    assert_eq!(report.files, ["doc_0000_Top.png", "doc_0002_Blue.png", "doc_0003_Background.png"]);
    let skipped: Vec<_> = report.skipped.iter().map(|l| (l.name.as_str(), l.reason)).collect();
    assert_eq!(skipped, [("Hidden", "hidden by visibleOnly")]);
}

#[test]
fn numbering_without_visible_only_is_unchanged() {
    let report = export(&session(), false);
    assert_eq!(report.files, ["doc_0000_Top.png", "doc_0001_Hidden.png", "doc_0002_Blue.png", "doc_0003_Background.png"]);
    assert!(report.skipped.is_empty());
}
