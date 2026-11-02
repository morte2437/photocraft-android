use photocraft_engine::Session;
use photocraft_ui_egui::{PhotocraftApp, Services};
use serde_json::json;

/// A normal paste must not make an unchanged external SVG replace a newer internal copy.
#[test]
fn normal_paste_preserves_newer_internal_copy_for_later_paste_as() {
    const SVG: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" width="2" height="2"><rect width="2" height="2" fill="#ff0000"/></svg>"##;
    let services =
        Services { clipboard_get_svg: Some(Box::new(|| Ok(Some(SVG.to_vec())))), clipboard_get_image: Some(Box::new(|| Ok(None))), ..Default::default() };
    let mut app = PhotocraftApp::new(Session::new(), services);
    app.run("file.new", json!({"width": 8, "height": 6})).unwrap();
    app.run("edit.paste", json!({"as": "pixels"})).unwrap();
    app.session.edit_prefs(|prefs| prefs.general.export_clipboard = false);
    app.run("file.new", json!({"width": 8, "height": 6})).unwrap();
    app.run("edit.fill", json!({"color": "#0000ff"})).unwrap();
    app.run("edit.copy", json!({})).unwrap();
    let pasted = app.run("edit.paste", json!({})).unwrap();
    assert!(pasted["layer"].as_u64().is_some(), "normal paste did not insert a layer: {pasted}");
    let pasted_as = app.run("edit.paste", json!({"as": "pixels"})).unwrap();
    assert!(pasted_as["layer"].as_u64().is_some(), "Paste As did not insert a layer: {pasted_as}");
    assert_ne!(pasted["layer"], pasted_as["layer"]);
    assert_eq!(app.run("document.pixel", json!({"x": 4, "y": 3})).unwrap(), json!([0.0, 0.0, 1.0, 1.0]));
}
