//! #12035: Image Rotation › Arbitrary pre-fills the ruler's straightening angle at the
//! precision the reference application shows. A ruler from 100,100 to 400,300 (A -33.7° on
//! the Info panel) opens the Rotate Canvas dialog at 33.69°, Counter clockwise, and OK runs
//! `image.rotation.arbitrary` with that two-decimal angle, not the field's old one-decimal
//! round. The Angle range is -359.99..359.99, wide, so today's wide-range rounding to one
//! decimal decided the dialog's value; only this field now keeps hundredths.

use super::filter_dialog::{body, open, params_of, reset_fields};
use crate::PhotocraftApp;
use serde_json::{Map, Value, json};

/// Draw a dialog body once, as a live dialog does every frame: the drawn fields are what
/// the dialog shows and what OK sends.
fn drawn(f: &mut Map<String, Value>) {
    drawn_texts(f);
}

/// [`drawn`], returning the text the frame painted.
fn drawn_texts(f: &mut Map<String, Value>) -> Vec<String> {
    let mut out = egui::Context::default().run_ui(Default::default(), |ui| body(ui, f));
    out.textures_delta.clear();
    out.shapes
        .iter()
        .filter_map(|s| match &s.shape {
            egui::Shape::Text(t) => Some(t.galley.text().to_string()),
            _ => None,
        })
        .collect()
}

/// A 318 px square document (the size `ok_runs_the_two_decimal_angle` measures on), a ruler
/// from 100,100 to `end`, the opened Arbitrary dialog and its fields after one drawn frame.
fn ruler_dialog(end: [i32; 2]) -> (PhotocraftApp, u64, Map<String, Value>) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 318, "height": 318})).unwrap();
    app.run("image.analysis.rulerTool", json!({"start": [100, 100], "end": end})).unwrap();
    let id = open(&mut app, "image.rotation.arbitrary").unwrap();
    let mut f = app.ui.dialog_mut(id).unwrap().fields.clone();
    drawn(&mut f);
    (app, id, f)
}

/// The measured case: the Rotate Canvas dialog opens at 33.69°, Counter clockwise — the
/// angle keeps two decimals where the wide-range rule used to round it to 33.7.
#[test]
fn ruler_prefill_keeps_the_measured_two_decimal_angle() {
    let (_app, _id, f) = ruler_dialog([400, 300]);
    let a = f["angle"].as_f64().unwrap();
    assert!((a - 33.69).abs() < 1e-3, "the dialog opens at {a}, want the measured 33.69");
    assert_eq!(crate::widgets::fmt_num2(a), "33.69", "the Angle field shows the measured string");
    assert_eq!(f["direction"], json!("ccw"), "a ruler falling to the right straightens counter-clockwise");
}

/// A ruler that needs a clockwise turn pre-fills Clockwise, at the same two decimals;
/// with no ruler the dialog opens at today's default (0, Clockwise).
#[test]
fn clockwise_ruler_prefills_clockwise_and_no_ruler_keeps_the_default() {
    // (100,100) → (400,50) rises to the right: a 9.46° clockwise turn.
    let (_app, _id, f) = ruler_dialog([400, 50]);
    let a = f["angle"].as_f64().unwrap();
    assert_eq!(f["direction"], json!("cw"), "a rising ruler pre-fills Clockwise");
    assert!((a - 9.46).abs() < 1e-3, "the clockwise pre-fill is {a}, want 9.46 to two decimals");
    // No ruler: today's default, unchanged.
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 318, "height": 318})).unwrap();
    let id = open(&mut app, "image.rotation.arbitrary").unwrap();
    let mut f = app.ui.dialog_mut(id).unwrap().fields.clone();
    drawn(&mut f);
    assert_eq!(f["angle"], json!(0.0), "without a ruler the dialog opens at today's 0");
    assert_eq!(f["direction"], json!("cw"), "without a ruler the dialog opens Clockwise");
}

/// OK runs `image.rotation.arbitrary` with the two-decimal angle: a 318 px square rotated by
/// 33.69° measures 441 px across, by 33.7° 442 px — the dialog's rounding must not decide.
#[test]
fn ok_runs_the_two_decimal_angle() {
    let (mut app, id, f) = ruler_dialog([400, 300]);
    let params = params_of(&f);
    assert!((params["angle"].as_f64().unwrap() - 33.69).abs() < 1e-3, "OK sends {params}");
    app.ui.dialog_mut(id).unwrap().fields = f;
    let r = crate::dialogs::confirm(&mut app, id).unwrap();
    assert_eq!((r["width"].as_u64(), r["height"].as_u64()), (Some(441), Some(441)), "442 would mean the 33.7 round ran");
}

/// The f32 double-rounding the review caught: a 43x27 ruler line (here 100,100 to 143,127)
/// is 32.1249984 degrees, and the f32 cast of that full-precision pre-fill lands exactly on
/// 32.125 - rounding the widened value half away from zero opened the dialog at 32.13. The
/// hundredths round in f64 before the field narrows, so it opens at 32.12, Counter clockwise.
#[test]
fn the_ruler_angle_rounds_at_full_precision_before_the_field_narrows() {
    let (_app, _id, f) = ruler_dialog([143, 127]);
    let a = f["angle"].as_f64().unwrap();
    assert!((a - 32.12).abs() < 1e-3, "the 43x27 ruler opens at {a}, want 32.12");
    assert_eq!(crate::widgets::fmt_num2(a), "32.12", "the Angle field shows 32.12, not the 32.13 the f32 round gives");
    assert!((a - 32.13).abs() > 1e-3, "the f32 double-round must not reach the dialog: {a}");
    assert_eq!(f["direction"], json!("ccw"));
}

/// #13067: the 43x27 ruler's first drawn frame, after opening and again after Alt+Reset,
/// shows the 32.12 the dialog stores (the unrounded pre-fill narrowed to exactly 32.125
/// and drew 32.13 until the next repaint), and the dialog keeps that 32.12 as the exact
/// f64 OK sends, not the 32.1199989 an f32 field value widens to.
#[test]
fn the_ruler_angle_shows_and_sends_the_exact_rounded_value() {
    let (mut app, id, mut f) = ruler_dialog([143, 127]);
    let check = |f: &mut Map<String, Value>, when: &str| {
        let shown = drawn_texts(f);
        assert!(shown.iter().any(|t| t == "32.12"), "{when}: the Angle field shows 32.12, drew {shown:?}");
        assert!(!shown.iter().any(|t| t == "32.13"), "{when}: the Angle field must not show 32.13, drew {shown:?}");
        assert_eq!(f["angle"].as_f64(), Some(32.12), "{when}: the dialog keeps the exact rounded angle");
        assert_eq!(params_of(f)["angle"].as_f64(), Some(32.12), "{when}: OK sends the exact rounded angle");
    };
    // `ruler_dialog` drew one frame already: the first frame draws the fields as opened.
    check(&mut app.ui.dialog_mut(id).unwrap().fields.clone(), "after opening");
    check(&mut f, "on the next frame");
    // Move off the pre-fill first, so only the reset can bring 32.12 back.
    f.insert("angle".into(), json!(5.0));
    reset_fields(&app, &mut f);
    check(&mut f, "after Alt+Reset");
}

/// The fix is per-field: unrelated number fields keep the precision they have today —
/// wide ranges round to one decimal (in another dialog, and under the same key name),
/// small ranges keep their two.
#[test]
fn other_number_fields_keep_their_precision() {
    // A wide range in another dialog: Gaussian Blur's radius (0.1..1000) rounds to one decimal.
    let mut f = Map::new();
    f.insert("__command".into(), json!("filter.blur.gaussianBlur"));
    f.insert("radius".into(), json!(7.26));
    drawn(&mut f);
    let r = f["radius"].as_f64().unwrap();
    assert!((r - 7.3).abs() < 1e-5, "the Gaussian Blur radius {r} must stay at one decimal");
    // The same key the fix touches, in another dialog: Twirl's angle (-999..999) rounds to one.
    let mut f = Map::new();
    f.insert("__command".into(), json!("filter.distort.twirl"));
    f.insert("angle".into(), json!(33.69));
    drawn(&mut f);
    let a = f["angle"].as_f64().unwrap();
    assert!((a - 33.7).abs() < 1e-5, "Twirl's angle {a} keeps the one-decimal round");
    // A small range keeps its two decimals (Radial Blur spins, so the centre fields stay plain).
    let mut f = Map::new();
    f.insert("__command".into(), json!("filter.blur.radialBlur"));
    f.insert("method".into(), json!("spin"));
    f.insert("centerX".into(), json!(0.123));
    drawn(&mut f);
    let x = f["centerX"].as_f64().unwrap();
    assert!((x - 0.12).abs() < 1e-5, "the Radial Blur centre {x} keeps its two decimals");
}

/// The pre-fill is rounded before any frame draws: `ui.dialog.open` then `ui.dialog.confirm`
/// can run without a draw between them, and the 43x27 ruler's raw 32.1249984 must not reach
/// the command, after opening or after Alt+Reset.
#[test]
fn the_ruler_angle_is_rounded_before_the_first_frame() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 318, "height": 318})).unwrap();
    app.run("image.analysis.rulerTool", json!({"start": [100, 100], "end": [143, 127]})).unwrap();
    let id = open(&mut app, "image.rotation.arbitrary").unwrap();
    let mut f = app.ui.dialog_mut(id).unwrap().fields.clone();
    assert_eq!(params_of(&f)["angle"].as_f64(), Some(32.12), "undrawn, OK sends the rounded angle");
    f.insert("angle".into(), json!(5.0));
    reset_fields(&app, &mut f);
    assert_eq!(params_of(&f)["angle"].as_f64(), Some(32.12), "undrawn after Alt+Reset, OK sends the rounded angle");
}
