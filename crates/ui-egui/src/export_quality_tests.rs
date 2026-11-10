//! #11481: File › Export › Artboards to Files (and Layers / Layer Comps to Files) has a JPEG
//! Quality field behind a Custom checkbox, as Batch has (#3956). The quality reaches the command
//! only for a JPEG with Custom on: left alone, an export writes what the command writes with no
//! quality (the source JPEG's own quality, else the encoder default), a WebP stays lossless and
//! PNG has none.
use crate::PhotocraftApp;
use egui::accesskit::Role;
use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::{Value, json};

const ARTBOARDS: &str = "file.export.artboardsToFiles";
const EXPORTS: [&str; 3] = [ARTBOARDS, "file.export.layersToFiles", "file.export.layerCompsToFiles"];

fn harness() -> Harness<'static, PhotocraftApp> {
    let mut h = Harness::builder().with_size(vec2(1440.0, 900.0)).with_max_steps(64).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        PhotocraftApp::new(photocraft_engine::Session::new(), Default::default())
    });
    let app = h.state_mut();
    app.run("file.new", json!({"width": 64, "height": 64})).unwrap();
    // A noisy layer inside the board, so the JPEG quality shows in the file.
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("edit.fill", json!({"color": "#8A42B3"})).unwrap();
    app.run("filter.noise.addNoise", json!({"amount": 60})).unwrap();
    app.run("layer.new.artboardFromLayers", json!({"name": "Board"})).unwrap();
    app.run("layerComp.new", json!({"name": "Comp"})).unwrap();
    app.sync_views();
    h
}

fn open(h: &mut Harness<'_, PhotocraftApp>, id: &str, dir: &str, format: &str) -> u64 {
    let ctx = h.ctx.clone();
    let dialog = crate::menus::invoke(h.state_mut(), &ctx, id, json!({})).unwrap()["dialog"].as_u64().unwrap();
    let fields = &mut h.state_mut().ui.dialog_mut(dialog).unwrap().fields;
    fields.insert("dir".into(), json!(dir));
    fields.insert("prefix".into(), json!("out"));
    fields.insert("format".into(), json!(format));
    h.run_steps(4);
    dialog
}

fn params(h: &mut Harness<'_, PhotocraftApp>, dialog: u64) -> Value {
    crate::filter_dialog::params_of(&h.state_mut().ui.dialog_mut(dialog).unwrap().fields)
}

fn exported(h: &mut Harness<'_, PhotocraftApp>, dialog: u64) -> Vec<u8> {
    let r = crate::dialogs::confirm(h.state_mut(), dialog).unwrap();
    std::fs::read(r["files"][0].as_str().unwrap()).unwrap()
}

/// The same export run straight through the command, into `dir`.
fn direct(h: &mut Harness<'_, PhotocraftApp>, dir: &str, format: &str, quality: Option<i64>) -> Vec<u8> {
    let mut p = json!({"dir": dir, "prefix": "out", "format": format});
    if let Some(q) = quality {
        p["quality"] = json!(q);
    }
    let r = h.state_mut().run(ARTBOARDS, p).unwrap();
    std::fs::read(r["files"][0].as_str().unwrap()).unwrap()
}

/// Whether the Quality number field (not its row label) takes input.
fn quality_enabled(h: &Harness<'_, PhotocraftApp>) -> bool {
    let fields: Vec<bool> =
        h.query_all_by_label("Quality").filter(|n| n.accesskit_node().role() != Role::Label).map(|n| !n.accesskit_node().is_disabled()).collect();
    assert!(!fields.is_empty(), "the dialog shows a Quality field");
    assert!(fields.iter().all(|e| *e == fields[0]), "{fields:?}");
    fields[0]
}

/// Whether the Custom checkbox is ticked.
fn custom_ticked(h: &Harness<'_, PhotocraftApp>) -> bool {
    h.get_by_role_and_label(Role::CheckBox, "Custom").accesskit_node().toggled() == Some(egui::accesskit::Toggled::True)
}

/// The value the Quality field shows.
fn shown_quality(h: &Harness<'_, PhotocraftApp>) -> Option<f64> {
    h.get_by_role_and_label(Role::SpinButton, "Quality").accesskit_node().numeric_value()
}

fn tick_custom(h: &mut Harness<'_, PhotocraftApp>) {
    h.get_by_role_and_label(Role::CheckBox, "Custom").click();
    h.run_steps(2);
}

/// Types `text` into the Quality field and commits it with Tab, as a user does.
fn type_quality(h: &mut Harness<'_, PhotocraftApp>, text: &str) {
    h.get_by_role_and_label(Role::SpinButton, "Quality").click();
    h.run_steps(1);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.run_steps(1);
    for ch in text.chars() {
        h.event(egui::Event::Text(ch.to_string()));
        h.run_steps(1);
    }
    h.key_press(egui::Key::Tab);
    h.run_steps(2);
}

/// Holds Alt, which turns Cancel into Reset, and clicks it.
fn alt_reset(h: &mut Harness<'_, PhotocraftApp>) {
    let alt = egui::Modifiers::ALT;
    h.event(egui::Event::ModifiersChanged(alt));
    h.run_steps(2);
    let at = h.get_by_label("Reset").rect().center();
    h.event(egui::Event::PointerMoved(at));
    h.run_steps(1);
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: alt });
        h.run_steps(1);
    }
    h.event(egui::Event::ModifiersChanged(egui::Modifiers::NONE));
    h.run_steps(3);
}

fn tmp() -> (tempfile::TempDir, String) {
    let t = tempfile::tempdir().unwrap();
    let s = t.path().to_string_lossy().into_owned();
    (t, s)
}

#[test]
fn artboards_to_files_sends_a_custom_jpeg_quality() {
    let mut h = harness();
    let (_a, via_dialog) = tmp();
    let (_b, via_command) = tmp();
    let mut sizes = Vec::new();
    for quality in [0, 3, 12] {
        let dialog = open(&mut h, ARTBOARDS, &via_dialog, "jpg");
        assert!(!custom_ticked(&h) && !quality_enabled(&h), "JPEG with Custom off: the field is greyed out");
        tick_custom(&mut h);
        assert!(custom_ticked(&h) && quality_enabled(&h), "JPEG with Custom on: the field takes input");
        assert_eq!(params(&mut h, dialog)["quality"], json!(10), "Custom starts at quality 10");
        type_quality(&mut h, &quality.to_string());
        assert_eq!(shown_quality(&h), Some(quality as f64), "the field shows the typed quality");
        assert_eq!(params(&mut h, dialog)["quality"], json!(quality), "the typed quality reaches the command");
        let bytes = exported(&mut h, dialog);
        assert!(bytes == direct(&mut h, &via_command, "jpg", Some(quality)), "quality {quality}: the file the command writes at that quality");
        sizes.push(bytes.len());
    }
    assert!(sizes[0] < sizes[1] && sizes[1] < sizes[2], "a higher quality writes a bigger JPEG: {sizes:?}");
}

#[test]
fn an_untouched_jpeg_export_writes_what_the_command_writes_with_no_quality() {
    let mut h = harness();
    let (_a, via_dialog) = tmp();
    let (_b, via_command) = tmp();
    let dialog = open(&mut h, ARTBOARDS, &via_dialog, "jpg");
    assert!(params(&mut h, dialog).get("quality").is_none_or(Value::is_null), "no quality is sent");
    let bytes = exported(&mut h, dialog);
    assert!(bytes == direct(&mut h, &via_command, "jpg", None), "the untouched dialog keeps the command's own JPEG quality");
}

#[test]
fn export_to_files_quality_leaves_png_and_webp_as_they_were() {
    let mut h = harness();
    let (_a, dir) = tmp();
    for id in EXPORTS {
        for format in ["png", "webp"] {
            let dialog = open(&mut h, id, &dir, format);
            // Custom on from an earlier JPEG choice still sends nothing for these formats.
            h.state_mut().ui.dialog_mut(dialog).unwrap().fields.insert("__jpegCustom".into(), json!(true));
            h.run_steps(2);
            assert!(!quality_enabled(&h), "{id} {format}: the field is greyed out");
            assert!(params(&mut h, dialog).get("quality").is_none_or(Value::is_null), "{id} {format}: no quality is sent");
            let bytes = exported(&mut h, dialog);
            if format == "webp" {
                // The default WebP stays lossless (a VP8L bitstream, not lossy VP8).
                assert!(bytes.windows(4).any(|w| w == b"VP8L"), "{id}: the WebP stays lossless");
            }
        }
        // Back to JPEG with Custom on, the field sends its quality again.
        let dialog = open(&mut h, id, &dir, "jpg");
        h.state_mut().ui.dialog_mut(dialog).unwrap().fields.insert("__jpegCustom".into(), json!(true));
        h.run_steps(2);
        assert!(quality_enabled(&h), "{id}");
        assert_eq!(params(&mut h, dialog)["quality"], json!(10), "{id}");
        crate::dialogs::cancel(h.state_mut(), dialog).unwrap();
    }
}

/// A Custom quality survives a switch to another format and back in the same dialog.
#[test]
fn the_custom_quality_survives_a_format_round_trip() {
    let mut h = harness();
    let (_a, dir) = tmp();
    let dialog = open(&mut h, ARTBOARDS, &dir, "jpg");
    tick_custom(&mut h);
    type_quality(&mut h, "4");
    assert_eq!(params(&mut h, dialog)["quality"], json!(4));
    for format in ["png", "webp"] {
        h.state_mut().ui.dialog_mut(dialog).unwrap().fields.insert("format".into(), json!(format));
        h.run_steps(2);
        assert!(!quality_enabled(&h), "{format}: the field is greyed out");
        assert!(params(&mut h, dialog)["quality"].is_null(), "{format}: no quality is sent");
        h.state_mut().ui.dialog_mut(dialog).unwrap().fields.insert("format".into(), json!("jpg"));
        h.run_steps(2);
        assert!(custom_ticked(&h) && quality_enabled(&h), "back from {format}: Custom is still on");
        assert_eq!(shown_quality(&h), Some(4.0), "back from {format}: the field still shows 4");
        assert_eq!(params(&mut h, dialog)["quality"], json!(4), "back from {format}: quality 4 is sent");
    }
}

/// Alt+Reset (Alt held, Cancel clicked as Reset) puts the Custom checkbox and its quality back to
/// how the dialog opened, and keeps the dialog open.
#[test]
fn alt_reset_restores_the_custom_quality() {
    let mut h = harness();
    let ctx = h.ctx.clone();
    let dialog = crate::menus::invoke(h.state_mut(), &ctx, ARTBOARDS, json!({})).unwrap()["dialog"].as_u64().unwrap();
    // JPEG chosen in the dialog, after it opened: Reset puts the format back too.
    h.state_mut().ui.dialog_mut(dialog).unwrap().fields.insert("format".into(), json!("jpg"));
    h.run_steps(4);
    tick_custom(&mut h);
    type_quality(&mut h, "4");
    assert_eq!(params(&mut h, dialog)["quality"], json!(4));
    alt_reset(&mut h);
    assert!(h.state().ui.dialogs.iter().any(|d| d.id == dialog), "Reset keeps the dialog open");
    let p = params(&mut h, dialog);
    assert_eq!((&p["format"], &p["quality"]), (&json!("png"), &Value::Null), "the opening format, no quality");
    assert!(!custom_ticked(&h) && !quality_enabled(&h), "Custom is off again");
    assert_eq!(shown_quality(&h), Some(10.0), "the field shows the opening quality");
    // Choosing JPEG again still sends no quality until Custom is ticked.
    h.state_mut().ui.dialog_mut(dialog).unwrap().fields.insert("format".into(), json!("jpg"));
    h.run_steps(2);
    assert!(!custom_ticked(&h), "JPEG after Reset: Custom stays off");
    assert!(params(&mut h, dialog)["quality"].is_null(), "JPEG after Reset: no quality is sent");
}
