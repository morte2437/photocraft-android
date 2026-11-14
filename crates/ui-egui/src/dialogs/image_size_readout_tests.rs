//! The Image Size dialog's "Image Size:" readout (#14768): width × height × the mode's colour
//! channels × bytes per channel (no alpha; Bitmap one bit, rows padded to whole bytes), whole bytes
//! below 1K, then K/M/G/T with one decimal in the system's decimal mark, and "(was …)" only once
//! the pixel dimensions differ from the original.

use super::*;
use egui_kittest::kittest::Queryable;

/// An Image Size dialog for a new document made with `new` (file.new params).
fn dialog(new: Value) -> (egui_kittest::Harness<'static, PhotocraftApp>, u64) {
    dialog_after(new, &[])
}

/// An Image Size dialog for a new document made with `new`, after running `commands` on it.
fn dialog_after(new: Value, commands: &[&str]) -> (egui_kittest::Harness<'static, PhotocraftApp>, u64) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", new).unwrap();
    for c in commands {
        app.session.execute(c, json!({})).unwrap();
    }
    let mut h = egui_kittest::Harness::builder()
        .with_size(egui::vec2(1400.0, 900.0))
        .build_ui_state(|ui, app: &mut PhotocraftApp| crate::dialogs::show(app, ui.ctx()), app);
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    let id = open(h.state_mut(), "image.imageSize").unwrap();
    h.run_steps(3);
    (h, id)
}

fn set_size(h: &mut egui_kittest::Harness<'static, PhotocraftApp>, id: u64, w: f64, ht: f64) {
    let d = h.state_mut().ui.dialog_mut(id).unwrap();
    d.fields.insert("__unit".into(), json!("px"));
    d.fields.insert("width".into(), json!(w));
    d.fields.insert("height".into(), json!(ht));
    h.run_steps(3);
}

fn shows(h: &egui_kittest::Harness<'static, PhotocraftApp>, text: &str) -> bool {
    h.query_by_label(text).is_some()
}

#[test]
fn an_untouched_rgb_8_bit_dialog_reads_the_flat_image_without_was() {
    let (h, _) = dialog(json!({"width": 400, "height": 300}));
    assert!(shows(&h, "351.6K"), "400 x 300 x 3 bytes");
    assert!(h.query_by_label_contains("(was").is_none(), "no \"(was …)\" while unchanged");
    assert!(h.query_by_label_contains("469K").is_none(), "alpha isn't counted");
}

#[test]
fn a_changed_size_shows_was() {
    let (mut h, id) = dialog(json!({"width": 400, "height": 300}));
    set_size(&mut h, id, 200.0, 150.0);
    assert!(shows(&h, "87.9K (was 351.6K)"));
    set_size(&mut h, id, 400.0, 300.0);
    assert!(shows(&h, "351.6K"), "back to the original size");
    assert!(h.query_by_label_contains("(was").is_none());
}

#[test]
fn bytes_per_channel_follow_the_depth() {
    let (h, _) = dialog(json!({"width": 200, "height": 150, "depth": 32}));
    assert!(shows(&h, "351.6K"), "200 x 150 x 3 x 4 bytes");
    let (h, _) = dialog(json!({"width": 200, "height": 150, "depth": 16}));
    assert!(shows(&h, "175.8K"), "200 x 150 x 3 x 2 bytes");
}

#[test]
fn colour_channels_follow_the_mode() {
    let (h, _) = dialog(json!({"width": 400, "height": 300, "mode": "cmyk"}));
    assert!(shows(&h, "468.8K"), "CMYK: 4 channels");
    let (h, _) = dialog(json!({"width": 400, "height": 300, "mode": "grayscale"}));
    assert!(shows(&h, "117.2K"), "Grayscale: 1 channel");
    let (h, _) = dialog(json!({"width": 400, "height": 300, "mode": "lab"}));
    assert!(shows(&h, "351.6K"), "Lab: 3 channels");
}

/// Indexed is one index channel and Bitmap one bit per pixel, though both are stored expanded.
#[test]
fn indexed_and_bitmap_count_their_own_channel() {
    let (h, _) = dialog_after(json!({"width": 400, "height": 300}), &["image.mode.indexedColor"]);
    assert_eq!(h.state().session.active().unwrap().doc.mode, photocraft_color::ColorMode::Indexed);
    assert!(shows(&h, "117.2K"), "Indexed: 400 x 300 x 1 byte");
    let (h, _) = dialog_after(json!({"width": 400, "height": 300}), &["image.mode.grayscale", "image.mode.bitmap"]);
    assert_eq!(h.state().session.active().unwrap().doc.mode, photocraft_color::ColorMode::Bitmap);
    assert!(shows(&h, "14.6K"), "Bitmap: 400 x 300 bits");
}

#[test]
fn tiny_images_read_in_bytes() {
    let (h, _) = dialog(json!({"width": 1, "height": 1}));
    assert!(shows(&h, "3 bytes"));
    let (h, _) = dialog(json!({"width": 11, "height": 8}));
    assert!(shows(&h, "264 bytes"));
}

#[test]
fn the_formatter_uses_one_decimal_and_the_system_decimal_mark() {
    crate::i18n::with_decimal_separator(None, || {
        assert_eq!(image_size_bytes(3.0), "3 bytes");
        assert_eq!(image_size_bytes(1.0), "1 byte");
        assert_eq!(image_size_bytes(1023.6), "1.0K", "1024 whole bytes");
        assert_eq!(image_size_bytes(1_048_575.0), "1.0M", "not 1024.0K");
        assert_eq!(image_size_bytes(1024.0 * 1024.0 * 1024.0 - 1.0), "1.0G", "not 1024.0M");
        assert_eq!(image_size_bytes(f64::NAN), "0 bytes");
        assert_eq!(image_size_bytes(1023.0), "1023 bytes");
        assert_eq!(image_size_bytes(1024.0), "1.0K");
        assert_eq!(image_size_bytes(360_000.0), "351.6K");
        assert_eq!(image_size_bytes(6016.0 * 6016.0 * 3.0), "103.5M");
        assert_eq!(image_size_bytes(188.6 * 1024.0 * 1024.0 * 1024.0), "188.6G");
        assert_eq!(image_size_bytes(1.5 * 1024f64.powi(4)), "1.5T");
        assert_eq!(image_size_bytes(1024f64.powi(4) - 1.0), "1.0T", "not 1024.0G");
    });
    crate::i18n::with_decimal_separator(Some(','), || {
        assert_eq!(image_size_bytes(360_000.0), "351,6K");
        assert_eq!(image_size_bytes(90_000.0), "87,9K");
    });
    crate::i18n::with_decimal_separator(Some('.'), || assert_eq!(image_size_bytes(360_000.0), "351.6K"));
}

#[test]
fn a_comma_system_reads_with_a_comma() {
    crate::i18n::with_decimal_separator(Some(','), || {
        let (mut h, id) = dialog(json!({"width": 400, "height": 300}));
        assert!(shows(&h, "351,6K"));
        set_size(&mut h, id, 200.0, 150.0);
        assert!(shows(&h, "87,9K (was 351,6K)"));
    });
}

#[test]
fn canvas_size_and_human_bytes_keep_their_output() {
    assert_eq!(human_bytes(400.0 * 300.0 * 4.0), "469K");
    assert_eq!(human_bytes(10.0), "1K");
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 400, "height": 300})).unwrap();
    let mut h = egui_kittest::Harness::builder()
        .with_size(egui::vec2(1400.0, 900.0))
        .build_ui_state(|ui, app: &mut PhotocraftApp| crate::dialogs::show(app, ui.ctx()), app);
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    open(h.state_mut(), "image.canvasSize").unwrap();
    h.run_steps(3);
    assert!(h.query_all_by_label_contains("469K").next().is_some(), "Canvas Size still reads the layer's bytes (#14582)");
}

/// Bitmap rows are padded to whole bytes: 401 bits take 51 bytes, and one pixel one byte.
#[test]
fn bitmap_rows_round_up_to_whole_bytes() {
    let (h, _) = dialog_after(json!({"width": 401, "height": 300}), &["image.mode.grayscale", "image.mode.bitmap"]);
    assert!(shows(&h, "14.9K"), "51 x 300 bytes");
    let (h, _) = dialog_after(json!({"width": 1, "height": 1}), &["image.mode.grayscale", "image.mode.bitmap"]);
    assert!(shows(&h, "1 byte"));
}

/// "(was …)" follows the pixel dimensions, even when padding keeps the byte count the same.
#[test]
fn a_bitmap_resize_inside_one_byte_still_shows_was() {
    let (mut h, id) = dialog_after(json!({"width": 401, "height": 300}), &["image.mode.grayscale", "image.mode.bitmap"]);
    set_size(&mut h, id, 402.0, 300.0);
    assert!(shows(&h, "14.9K (was 14.9K)"));
    set_size(&mut h, id, 800.0, 300.0);
    assert!(shows(&h, "29.3K (was 14.9K)"), "the padded original");
}
