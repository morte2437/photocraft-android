//! #14789: while the Image Size dialog is open, Alt+1 to Alt+8 (Option on macOS) pick the
//! Resample method, also while a field has focus, without typing the digit into it.

use super::*;
use egui::accesskit::Role;
use egui::{Key, Modifiers};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

/// An open Image Size dialog for a 200 x 100 px document; `indexed` makes it Indexed Color.
fn image_size_dialog(indexed: bool) -> Harness<'static, PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 200, "height": 100})).unwrap();
    if indexed {
        app.session.execute("image.mode.indexedColor", json!({})).unwrap();
    }
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_ui_state(|ui, app| crate::dialogs::show(app, ui.ctx()), app);
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    open(h.state_mut(), "image.imageSize").unwrap();
    h.run_steps(3);
    h
}

fn fields(h: &Harness<'static, PhotocraftApp>) -> Map<String, Value> {
    h.state().ui.dialogs.first().map(|d| d.fields.clone()).expect("the dialog is open")
}

fn resample(h: &Harness<'static, PhotocraftApp>) -> Value {
    fields(h).get("resample").cloned().unwrap_or(Value::Null)
}

fn set_resample(h: &mut Harness<'static, PhotocraftApp>, method: &str) {
    if let Some(d) = h.state_mut().ui.dialogs.first_mut() {
        d.fields.insert("resample".into(), json!(method));
    }
    h.run_steps(2);
}

/// Presses and releases `key` with `modifiers` in one frame, plus the text a real keyboard types
/// with it: winit sends the digit along with Alt+digit on Windows and Linux. (The harness's own
/// key helpers give each event a frame of its own, which a real keyboard doesn't.)
fn press(h: &mut Harness<'static, PhotocraftApp>, modifiers: Modifiers, key: Key, text: &str) {
    let events = &mut h.input_mut().events;
    events.push(egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers });
    events.push(egui::Event::Text(text.into()));
    events.push(egui::Event::Key { key, physical_key: None, pressed: false, repeat: false, modifiers });
    h.run_steps(2);
}

/// Each key picks its method: Alt+1 Automatic, Alt+2 Preserve Details, Alt+4 Bicubic Smoother,
/// Alt+5 Bicubic Sharper, Alt+6 Bicubic, Alt+7 Nearest Neighbor, Alt+8 Bilinear.
#[test]
fn each_alt_digit_picks_its_method() {
    let mut h = image_size_dialog(false);
    for (key, digit, method) in [
        (Key::Num7, "7", "nearest"),
        (Key::Num1, "1", "automatic"),
        (Key::Num2, "2", "preserveDetails"),
        (Key::Num4, "4", "bicubicSmoother"),
        (Key::Num5, "5", "bicubicSharper"),
        (Key::Num6, "6", "bicubic"),
        (Key::Num8, "8", "bilinear"),
    ] {
        press(&mut h, Modifiers::ALT, key, digit);
        assert_eq!(resample(&h), json!(method), "Alt+{digit}");
    }
    // The list shows the picked method.
    assert!(h.query_all_by_role(Role::ComboBox).any(|n| n.value().as_deref() == Some("Bilinear")), "the Resample list shows Bilinear");
}

/// The dialog opens with Width focused and selected: Alt+7 picks Nearest Neighbor and types
/// nothing into it, and the field still takes typing afterwards.
#[test]
fn a_focused_field_does_not_get_the_digit() {
    let mut h = image_size_dialog(false);
    assert!(h.query_all_by_role(Role::SpinButton).next().expect("the Width field").is_focused(), "Width has focus");
    press(&mut h, Modifiers::ALT, Key::Num7, "7");
    assert_eq!(resample(&h), json!("nearest"));
    assert_eq!((num(&fields(&h), "width"), num(&fields(&h), "height")), (200.0, 100.0), "the 7 was not typed into Width");
    assert!(h.query_all_by_role(Role::SpinButton).next().expect("the Width field").is_focused(), "Width keeps the focus");
    // macOS: Option+7 types "¶" along with the key.
    press(&mut h, Modifiers::ALT, Key::Num8, "¶");
    assert_eq!(resample(&h), json!("bilinear"));
    assert_eq!(num(&fields(&h), "width"), 200.0, "Option's character was not typed into Width");
    h.event(egui::Event::Text("50".into()));
    h.run_steps(1);
    assert_eq!(num(&fields(&h), "width"), 50.0, "plain typing still edits Width");
}

/// Alt+3 is Preserve Details 2.0, which doesn't exist yet (#2942): it does nothing, and Lanczos,
/// which the reference application doesn't have, has no key.
#[test]
fn a_method_that_does_not_exist_does_nothing() {
    let mut h = image_size_dialog(false);
    set_resample(&mut h, "bicubicSharper");
    press(&mut h, Modifiers::ALT, Key::Num3, "3");
    assert_eq!(resample(&h), json!("bicubicSharper"));
    assert_eq!(num(&fields(&h), "width"), 200.0, "the 3 was not typed into Width");
    assert!(RESAMPLE_KEYS.iter().all(|(_, m)| *m != Some("lanczos")), "Lanczos gets no shortcut");
    // Alt+9 and Alt+0 aren't shortcuts.
    press(&mut h, Modifiers::ALT, Key::Num9, "9");
    assert_eq!(resample(&h), json!("bicubicSharper"));
}

/// Indexed Color offers only Automatic and Nearest Neighbor: the other keys do nothing there.
#[test]
fn a_method_the_mode_does_not_offer_does_nothing() {
    let mut h = image_size_dialog(true);
    set_resample(&mut h, "nearest");
    for (key, digit) in [(Key::Num2, "2"), (Key::Num4, "4"), (Key::Num5, "5"), (Key::Num6, "6"), (Key::Num8, "8")] {
        press(&mut h, Modifiers::ALT, key, digit);
        assert_eq!(resample(&h), json!("nearest"), "Alt+{digit} is not offered for Indexed Color");
    }
    press(&mut h, Modifiers::ALT, Key::Num1, "1");
    assert_eq!(resample(&h), json!("automatic"));
    press(&mut h, Modifiers::ALT, Key::Num7, "7");
    assert_eq!(resample(&h), json!("nearest"));
    assert_eq!(num(&fields(&h), "width"), 200.0, "no digit was typed into Width");
}

/// With Resample off the keys do nothing: Resample stays off and the size stays.
#[test]
fn resample_off_ignores_the_keys() {
    let mut h = image_size_dialog(false);
    set_resample(&mut h, "none");
    for (key, digit) in [(Key::Num1, "1"), (Key::Num7, "7"), (Key::Num8, "8")] {
        press(&mut h, Modifiers::ALT, key, digit);
        assert_eq!(resample(&h), json!("none"), "Alt+{digit}");
    }
    let f = fields(&h);
    assert_eq!((num(&f, "width"), num(&f, "height")), (200.0, 100.0), "{f:?}");
}

/// Ctrl+Alt is AltGr on Windows, which types characters on many layouts: it is no shortcut.
#[test]
fn ctrl_alt_digit_is_not_a_shortcut() {
    let mut h = image_size_dialog(false);
    set_resample(&mut h, "bicubic");
    press(&mut h, Modifiers::ALT | Modifiers::CTRL, Key::Num7, "{");
    assert_eq!(resample(&h), json!("bicubic"));
}

/// OK resizes with the method a shortcut picked: Nearest Neighbor doubles a hard black/white
/// edge without the grey a blending method puts between them.
#[test]
fn ok_applies_the_picked_method() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 4, "height": 1, "background": "white"})).unwrap();
    app.session.execute("select.rect", json!({"x": 0, "y": 0, "width": 2, "height": 1})).unwrap();
    app.session.execute("edit.fill", json!({"contents": "black"})).unwrap();
    app.session.execute("select.deselect", json!({})).unwrap();
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_ui_state(|ui, app| crate::dialogs::show(app, ui.ctx()), app);
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    let id = open(h.state_mut(), "image.imageSize").unwrap();
    h.run_steps(3);
    press(&mut h, Modifiers::ALT, Key::Num7, "7");
    if let Some(d) = h.state_mut().ui.dialog_mut(id) {
        d.fields.insert("width".into(), json!(8));
        d.fields.insert("height".into(), json!(2));
    }
    crate::dialogs::confirm(h.state_mut(), id).expect("OK runs Image Size");
    let doc = &h.state().session.active().unwrap().doc;
    assert_eq!(doc.size, photocraft_doc::Size::new(8, 2));
    let reds: Vec<u8> = photocraft_compose::flatten(doc).px.iter().take(8).map(|p| (p[0] * 255.0).round() as u8).collect();
    assert_eq!(reds, [0, 0, 0, 0, 255, 255, 255, 255], "Nearest Neighbor keeps the edge hard");
}
