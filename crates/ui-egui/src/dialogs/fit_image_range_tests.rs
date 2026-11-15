use egui::{Event, Key, Modifiers, accesskit::Role};
use egui_kittest::{
    Harness,
    kittest::{NodeT, Queryable},
};
use serde_json::{Value, json};

use super::{fit_image_range::Alert, show};
use crate::{PhotocraftApp, Services, state::DialogKind};

const FIT_IMAGE: &str = "file.automate.fitImage";

fn open() -> (Harness<'static, PhotocraftApp>, u64) {
    open_with_renderer(false)
}

fn open_with_renderer(gpu: bool) -> (Harness<'static, PhotocraftApp>, u64) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Services::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    let builder = Harness::builder().with_size(egui::vec2(1000.0, 700.0));
    let builder = if gpu { builder.wgpu() } else { builder };
    let mut h = builder.build_ui_state(|ui, app| show(app, ui.ctx()), app);
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    let ctx = h.ctx.clone();
    let id = crate::menus::invoke(h.state_mut(), &ctx, FIT_IMAGE, json!({})).unwrap()["dialog"].as_u64().unwrap();
    h.run_steps(4);
    (h, id)
}

fn type_side(h: &mut Harness<'static, PhotocraftApp>, label: &str, input: &str) {
    let field = h.query_all_by_role(Role::SpinButton).find(|node| node.accesskit_node().label().as_deref() == Some(label)).unwrap();
    field.click();
    h.run_steps(2);
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    if input.is_empty() {
        h.key_press(Key::Backspace);
    } else {
        h.event(Event::Text(input.into()));
    }
    h.run_steps(1);
}

fn field(h: &Harness<'_, PhotocraftApp>, id: u64, key: &str) -> Value {
    h.state().ui.dialogs.iter().find(|dialog| dialog.id == id).unwrap().fields[key].clone()
}

fn alert(h: &Harness<'_, PhotocraftApp>, expected: Alert) {
    let top = h.state().ui.dialogs.last().unwrap();
    assert_eq!(top.kind, DialogKind::Error);
    assert_eq!(top.fields["message"], expected.message());
}

fn dimensions(h: &Harness<'_, PhotocraftApp>) -> (u32, u32) {
    let size = h.state().session.active().unwrap().doc.size;
    (size.width, size.height)
}

#[test]
fn tab_clamps_both_sides_and_the_next_tab_accepts_each_boundary() {
    let (mut h, id) = open();
    for (label, key, input, boundary) in [("Width", "width", "-50", 1), ("Height", "height", "300001", 300_000)] {
        type_side(&mut h, label, input);
        h.key_press(Key::Tab);
        h.run_steps(4);
        alert(&h, Alert::Closest);
        assert_eq!(field(&h, id, key), boundary);
        h.key_press(Key::Enter);
        h.run_steps(3);

        type_side(&mut h, label, &boundary.to_string());
        h.key_press(Key::Tab);
        h.run_steps(4);
        assert_eq!(h.state().ui.dialogs.len(), 1);
        assert_eq!(field(&h, id, key), boundary);
    }
    assert_eq!(dimensions(&h), (400, 300));
}

#[test]
fn invalid_and_empty_entries_restore_the_last_accepted_value() {
    let (mut h, id) = open();
    type_side(&mut h, "Width", "600/2");
    h.key_press(Key::Tab);
    h.run_steps(4);
    assert_eq!(field(&h, id, "width"), 300);
    for input in ["abc", "", "NaN", "1/0", "1e999", &"9".repeat(1100)] {
        type_side(&mut h, "Width", input);
        h.key_press(Key::Tab);
        h.run_steps(4);
        alert(&h, Alert::Invalid);
        assert_eq!(field(&h, id, "width"), 300, "{input}");
        assert_eq!(field(&h, id, "height"), 300);
        h.key_press(Key::Enter);
        h.run_steps(3);
    }
}

#[test]
fn zero_on_submit_resizes_once_using_one_pixel() {
    for click in [false, true] {
        let (mut h, id) = open();
        h.state_mut().ui.dialog_mut(id).unwrap().fields.insert("height".into(), json!(1000));
        h.run_steps(2);
        let past = h.state().session.active().unwrap().history.past_len();
        type_side(&mut h, "Width", "0");
        if click {
            h.get_by_label("OK").click();
        } else {
            h.key_press(Key::Enter);
        }
        h.run_steps(4);
        assert_eq!(dimensions(&h), (1, 1));
        let history = &h.state().session.active().unwrap().history;
        assert_eq!(history.past_len(), past + 1);
        assert_eq!(history.entries().last().map(String::as_str), Some("Image Size"));
        assert!(h.state().ui.dialogs.iter().all(|dialog| dialog.id != id));
        alert(&h, Alert::Closest);
        h.state_mut().run("edit.undo", json!({})).unwrap();
        assert_eq!(dimensions(&h), (400, 300));
    }
}

#[test]
fn invalid_submit_keeps_the_dialog_open_without_resizing() {
    let (mut h, id) = open();
    type_side(&mut h, "Width", "abc");
    h.key_press(Key::Enter);
    h.run_steps(4);
    alert(&h, Alert::Invalid);
    assert_eq!(field(&h, id, "width"), 400);
    assert_eq!(dimensions(&h), (400, 300));
}

#[test]
fn programmatic_dialog_confirmation_validates_both_sides() {
    let (mut h, id) = open();
    type_side(&mut h, "Width", "350");
    h.key_press(Key::Tab);
    h.run_steps(4);
    let fields = &mut h.state_mut().ui.dialog_mut(id).unwrap().fields;
    fields.insert("width".into(), json!("abc"));
    fields.insert("height".into(), json!(-1));
    assert!(super::confirm(h.state_mut(), id).is_err());
    alert(&h, Alert::Invalid);
    assert_eq!(field(&h, id, "width"), 350);
    assert_eq!(field(&h, id, "height"), 1);
    assert_eq!(dimensions(&h), (400, 300));
}

#[test]
fn cancel_discards_an_uncommitted_entry() {
    let (mut h, _) = open();
    type_side(&mut h, "Width", "abc");
    h.get_by_label("Cancel").click();
    h.run_steps(4);
    assert!(h.state().ui.dialogs.is_empty());
    assert_eq!(dimensions(&h), (400, 300));
}

#[test]
fn fit_image_range_gpu_snapshots() {
    let Some(directory) = std::env::var_os("PHOTOCRAFT_FIT_IMAGE_SNAPSHOTS") else { return };
    let directory = std::path::PathBuf::from(directory);
    let _gpu = photocraft_testkit::gpu_lock();
    let (mut h, _) = open_with_renderer(true);
    h.render().unwrap().save(directory.join("dialog.png")).unwrap();
    for (input, expected, name) in [("-50", Alert::Closest, "range"), ("abc", Alert::Invalid, "invalid")] {
        type_side(&mut h, "Width", input);
        h.key_press(Key::Tab);
        h.run_steps(4);
        alert(&h, expected);
        h.render().unwrap().save(directory.join(format!("{name}.png"))).unwrap();
        h.key_press(Key::Enter);
        h.run_steps(3);
    }
}
