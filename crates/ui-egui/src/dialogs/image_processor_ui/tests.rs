use egui::{Event, Key, Modifiers, accesskit::Role};
use egui_kittest::{
    Harness,
    kittest::{NodeT, Queryable},
};
use serde_json::{Value, json};

use crate::{PhotocraftApp, Services, state::DialogKind};

const COMMAND: &str = "file.scripts.imageProcessor";

fn open(gpu: bool) -> Result<(Harness<'static, PhotocraftApp>, u64), String> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Services::default());
    app.run("file.new", json!({"width": 40, "height": 20}))?;
    let builder = Harness::builder().with_size(egui::vec2(1000.0, 920.0));
    let builder = if gpu { builder.wgpu() } else { builder };
    let mut h = builder.build_ui_state(|ui, app| crate::dialogs::show(app, ui.ctx()), app);
    PhotocraftApp::setup_context(&h.ctx, Default::default());
    let ctx = h.ctx.clone();
    let id = crate::menus::invoke(h.state_mut(), &ctx, COMMAND, json!({}))?["dialog"].as_u64().ok_or("Image Processor did not open a dialog")?;
    h.run_steps(4);
    Ok((h, id))
}

fn field(h: &Harness<'_, PhotocraftApp>, id: u64, key: &str) -> Value {
    h.state().ui.dialogs.iter().find(|dialog| dialog.id == id).and_then(|dialog| dialog.fields.get(key)).cloned().unwrap_or(Value::Null)
}

fn edit_number(h: &mut Harness<'static, PhotocraftApp>, label: &str, input: &str) {
    h.get_by_role_and_label(Role::SpinButton, label).click();
    h.run_steps(2);
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    if input.is_empty() {
        h.key_press(Key::Backspace);
    } else {
        h.event(Event::Text(input.into()));
    }
    h.run_steps(1);
    h.key_press(Key::Tab);
    h.run_steps(4);
}

fn alert(h: &Harness<'_, PhotocraftApp>, message: &str) {
    let dialog = h.state().ui.dialogs.last();
    assert_eq!(dialog.map(|dialog| dialog.kind), Some(DialogKind::Error));
    assert_eq!(dialog.and_then(|dialog| dialog.fields.get("message")).and_then(Value::as_str), Some(message));
}

fn dismiss(h: &mut Harness<'_, PhotocraftApp>) {
    h.key_press(Key::Enter);
    h.run_steps(4);
}

#[test]
fn defaults_and_no_format_alert_preserve_the_dialog() {
    let (mut h, id) = open(false).unwrap();
    assert_eq!(field(&h, id, "jpeg"), json!({"enabled": true, "quality": 5.0, "convertToSrgb": false, "resize": false, "width": 1.0, "height": 1.0}));
    assert_eq!(field(&h, id, "psd"), json!({"enabled": false, "maximizeCompatibility": true, "resize": false, "width": 1.0, "height": 1.0}));
    assert_eq!(field(&h, id, "tiff"), json!({"enabled": false, "lzw": false, "resize": false, "width": 1.0, "height": 1.0}));
    assert!(!h.get_by_label("Convert Profile to sRGB").accesskit_node().is_disabled());
    assert!(h.get_by_label("Maximize Compatibility").accesskit_node().is_disabled());
    assert!(h.get_by_label("LZW Compression").accesskit_node().is_disabled());
    assert!(h.query_all_by_role_and_label(Role::SpinButton, "W").all(|node| node.accesskit_node().is_disabled()));
    h.get_by_role_and_label(Role::CheckBox, "Save as JPEG").click();
    h.run_steps(4);
    let output = field(&h, id, "output");
    h.get_by_role_and_label(Role::Button, "Run").click();
    h.run_steps(4);
    alert(&h, super::NO_FORMAT);
    assert_eq!(field(&h, id, "output"), output);
    dismiss(&mut h);
    assert_eq!(h.state().ui.dialogs.len(), 1);
    assert_eq!(field(&h, id, "jpeg")["enabled"], false);
}

#[test]
fn run_dispatches_all_enabled_formats_with_their_own_dimensions() {
    let (mut h, id) = open(false).unwrap();
    let directory = tempfile::tempdir().unwrap();
    let fields = &mut h.state_mut().ui.dialog_mut(id).unwrap().fields;
    fields.insert("output".into(), json!(directory.path()));
    fields.insert("jpeg".into(), json!({"enabled": true, "resize": true, "width": 20, "height": 20}));
    fields.insert("psd".into(), json!({"enabled": true, "resize": true, "width": 10, "height": 10}));
    fields.insert("tiff".into(), json!({"enabled": true}));
    h.run_steps(4);
    h.get_by_role_and_label(Role::Button, "Run").click();
    h.run_steps(4);
    assert!(h.state().ui.dialogs.is_empty());
    let mut sizes: Vec<_> = std::fs::read_dir(directory.path())
        .unwrap()
        .map(|entry| {
            let path = entry.unwrap().path();
            let ext = path.extension().unwrap().to_str().unwrap().to_owned();
            let bytes = std::fs::read(&path).unwrap();
            let doc = photocraft_io::import(path.to_str().unwrap(), &bytes).unwrap().document;
            (ext, doc.size.width, doc.size.height)
        })
        .collect();
    sizes.sort();
    assert_eq!(sizes, [("jpg".into(), 20, 10), ("psd".into(), 10, 5), ("tif".into(), 40, 20)]);
}

#[test]
fn numeric_fields_clamp_and_restore_the_last_value() {
    let (mut h, id) = open(false).unwrap();
    edit_number(&mut h, "Quality", "7.6");
    assert_eq!(field(&h, id, "jpeg")["quality"].as_f64(), Some(7.6));
    for (input, value, message) in [
        ("abc", 7.6, super::invalid_entry("quality")),
        ("", 7.6, super::invalid_entry("quality")),
        ("NaN", 7.6, super::invalid_entry("quality")),
        ("20", 12.0, super::closest_value("quality")),
        ("0", 1.0, super::closest_value("quality")),
    ] {
        edit_number(&mut h, "Quality", input);
        alert(&h, message);
        assert_eq!(field(&h, id, "jpeg")["quality"].as_f64(), Some(value));
        dismiss(&mut h);
    }
    // Only JPEG's resize checkbox is enabled. Click its first row.
    h.query_all_by_label("Resize to Fit").next().unwrap().click();
    h.run_steps(4);
    let width = h.query_all_by_role_and_label(Role::SpinButton, "W").next().unwrap();
    width.click();
    h.run_steps(2);
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    h.event(Event::Text("40000".into()));
    h.key_press(Key::Enter);
    h.run_steps(4);
    alert(&h, super::closest_value("width"));
    assert_eq!(field(&h, id, "jpeg")["width"].as_f64(), Some(30_000.0));
    assert_eq!(field(&h, id, "psd")["width"].as_f64(), Some(1.0));
    assert_eq!(field(&h, id, "tiff")["width"].as_f64(), Some(1.0));
}

#[test]
fn image_processor_gpu_snapshots() {
    let Some(path) = std::env::var_os("PHOTOCRAFT_PROCESSOR_SNAPSHOTS") else { return };
    let path = std::path::PathBuf::from(path);
    let _gpu = photocraft_testkit::gpu_lock();
    let (mut h, _) = open(true).unwrap();
    h.render().unwrap().save(path.join("dialog.png")).unwrap();
    edit_number(&mut h, "Quality", "20");
    alert(&h, super::closest_value("quality"));
    h.render().unwrap().save(path.join("range-alert.png")).unwrap();
}
