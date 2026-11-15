//! #11977: Perspective Crop's W, H and Resolution clamp to the reference application's ranges with
//! its alerts, text that isn't a number restores the last value with an alert, Clear also resets
//! the resolution unit, and the Crop tool's fields keep today's behaviour.

use egui_kittest::kittest::Queryable;

use super::*;
use crate::crop_size::{clamp_length, clamp_resolution, normalize_length, normalize_resolution};

const PX_ALERT: &str = "A value between 1.000 Pixels and 300000.000 Pixels is required. Closest value inserted.";
const RES_ALERT: &str = "A number between 1.000 and 29999.999 is required. Closest value inserted.";
const INVALID: &str = "Invalid numeric entry. Restored last valid value.";

fn length(typed: &str, previous: &str) -> (String, Option<String>) {
    crate::i18n::with_decimal_separator(Some('.'), || clamp_length(typed, previous, 72.0, 72.0))
}

fn resolution(typed: &str, previous: &str, per_cm: bool) -> (String, Option<String>) {
    crate::i18n::with_decimal_separator(Some('.'), || clamp_resolution(typed, previous, per_cm))
}

#[test]
fn w_and_h_clamp_to_1_and_300000_px_with_the_alert() {
    assert_eq!(length("-5", "100 px"), ("1 px".to_string(), Some(PX_ALERT.to_string())));
    assert_eq!(length("0", "100 px"), ("1 px".to_string(), Some(PX_ALERT.to_string())));
    assert_eq!(length("999999", "100 px"), ("300000 px".to_string(), Some(PX_ALERT.to_string())));
    assert_eq!(length("1", "100 px"), ("1 px".to_string(), None));
    assert_eq!(length("300000", ""), ("300000 px".to_string(), None));
    assert_eq!(length("640", "100 px"), ("640 px".to_string(), None));
    assert_eq!(length("", "100 px"), (String::new(), None), "empty stays empty, no alert");
}

#[test]
fn other_units_clamp_to_the_converted_limits() {
    // At 72 ppi, 300000 px is 4166.667 in; 1 px is 0.014 in.
    let (kept, alert) = length("9999 in", "100 px");
    assert_eq!(kept, "4166.667 in");
    assert_eq!(alert.as_deref(), Some("A value between 0.014 Inches and 4166.667 Inches is required. Closest value inserted."));
    assert_eq!(length("4 in", "100 px"), ("4 in".to_string(), None));
}

#[test]
fn text_that_isnt_a_number_restores_the_last_value() {
    assert_eq!(length("abc", "120 px"), ("120 px".to_string(), Some(INVALID.to_string())));
    assert_eq!(length("12 parsecs", "120 px"), ("120 px".to_string(), Some(INVALID.to_string())));
    assert_eq!(resolution("lots", "300", false), ("300".to_string(), Some(INVALID.to_string())));
}

#[test]
fn the_resolution_clamps_to_1_and_29999_999_with_its_alert() {
    assert_eq!(resolution("0", "300", false), ("1".to_string(), Some(RES_ALERT.to_string())));
    assert_eq!(resolution("99999", "300", false), ("29999.999".to_string(), Some(RES_ALERT.to_string())));
    assert_eq!(resolution("20000", "300", false), ("20000".to_string(), None), "20000 is kept, and commits (below)");
    assert_eq!(resolution("72", "", false), ("72".to_string(), None));
    // px/cm: the per-inch limits divided by 2.54 (not measured beyond that conversion).
    let (kept, alert) = resolution("99999", "100", true);
    assert_eq!(kept, "11811.023");
    assert_eq!(alert.as_deref(), Some("A number between 0.394 and 11811.023 is required. Closest value inserted."));
    assert_eq!(resolution("0", "100", true).0, "0.394");
}

#[test]
fn the_alerts_use_the_system_decimal_separator() {
    let (_, alert) = crate::i18n::with_decimal_separator(Some(','), || clamp_length("0", "100 px", 72.0, 72.0));
    assert_eq!(alert.as_deref(), Some("A value between 1,000 Pixels and 300000,000 Pixels is required. Closest value inserted."));
    let (_, alert) = crate::i18n::with_decimal_separator(Some(','), || clamp_resolution("99999", "300", false));
    assert_eq!(alert.as_deref(), Some("A number between 1,000 and 29999,999 is required. Closest value inserted."));
}

#[test]
fn the_crop_tools_fields_keep_todays_behaviour() {
    assert_eq!(normalize_length("-5", "100 px"), "100 px");
    assert_eq!(normalize_length("999999", "100 px"), "999999 px");
    assert_eq!(normalize_length("abc", "100 px"), "100 px");
    assert_eq!(normalize_resolution("99999", "300"), "99999");
    assert_eq!(normalize_resolution("0", "300"), "300");
}

/// A 200 × 150 document with the Perspective Crop tool and a pending quad.
fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 200, "height": 150})).unwrap();
    app.sync_views();
    app.ui.tool = Tool::PerspectiveCrop;
    app
}

fn crop(app: &mut PhotocraftApp, resolution: f64) -> Result<Value, String> {
    let corners = [[10.0, 10.0], [190.0, 10.0], [190.0, 140.0], [10.0, 140.0]];
    app.run("image.perspectiveCrop", json!({"corners": corners, "resolution": resolution})).map_err(|e| e.to_string())
}

#[test]
fn the_crop_accepts_up_to_29999_999_ppi() {
    let mut app = app();
    assert!(crop(&mut app, 20000.0).is_ok(), "20000 ppi typed into the field commits");
    let mut app2 = self::app();
    assert!(crop(&mut app2, 29_999.999).is_ok());
    let mut app3 = self::app();
    let err = crop(&mut app3, 30_000.0).unwrap_err();
    assert!(err.contains("29999.999"), "{err}");
}

/// The options bar in a harness with the app's fonts and theme.
fn harness(app: PhotocraftApp) -> egui_kittest::Harness<'static, PhotocraftApp> {
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1400.0, 80.0)).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            if !ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            crate::panels::options_bar(app, ui);
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::Studio);
    h.run_steps(4);
    h
}

/// The options bar in a harness, with `typed` committed (Enter) into its `nth` text field (0 = W).
fn type_into(app: PhotocraftApp, nth: usize, typed: &str) -> PhotocraftApp {
    use egui::accesskit::Role;
    let mut h = harness(app);
    h.get_all_by_role(Role::TextInput).nth(nth).expect("the field").click();
    h.run();
    h.get_all_by_role(Role::TextInput).nth(nth).expect("the field").type_text(typed);
    h.run();
    h.key_press(egui::Key::Enter);
    h.run();
    h.run();
    h.into_state()
}

#[test]
fn an_out_of_range_width_in_the_options_bar_opens_the_alert() {
    let app = crate::i18n::with_decimal_separator(Some('.'), || type_into(app(), 0, "999999"));
    assert_eq!(app.ui.tool_options.crop_width, "300000 px");
    let messages: Vec<&str> =
        app.ui.dialogs.iter().filter(|d| d.kind == crate::state::DialogKind::Error).filter_map(|d| d.fields.get("message")?.as_str()).collect();
    assert_eq!(messages, [PX_ALERT]);
}

#[test]
fn clear_also_resets_the_resolution_unit() {
    let mut app = app();
    let o = &mut app.ui.tool_options;
    o.crop_width = "4 in".into();
    o.crop_height = "5 in".into();
    o.crop_resolution = "118.11".into();
    o.crop_resolution_unit = crate::crop_size::PX_PER_CM.into();
    let mut h = harness(app);
    h.get_by_label("Clear").click();
    h.run();
    let o = &h.state().ui.tool_options;
    assert!(o.crop_width.is_empty() && o.crop_height.is_empty() && o.crop_resolution.is_empty());
    assert_eq!(o.crop_resolution_unit, crate::crop_size::PX_PER_IN);
}

#[test]
fn the_resolution_never_rounds_past_the_limit() {
    // The px/cm limit is 11811.0232…: just below it keeps 11811.023, just above it is out of range.
    assert_eq!(resolution("11811.0232", "100", true), ("11811.023".to_string(), None));
    assert_eq!(resolution("11811.0236", "100", true).0, "11811.023");
    assert_eq!(crate::crop_size::fit_resolution(29_999.999_5, false), "29999.999");
    assert_eq!(crate::crop_size::fit_resolution(118.110_236, true), "118.11");
}

#[test]
fn front_image_keeps_a_30000_ppi_documents_resolution_within_the_limit() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 200, "height": 150, "resolution": 30000})).unwrap();
    app.sync_views();
    app.ui.tool = Tool::PerspectiveCrop;
    let mut h = harness(app);
    h.get_by_label("Front Image").click();
    h.run();
    let app = h.into_state();
    assert_eq!(app.ui.tool_options.crop_resolution, "29999.999");
    let ppi = crate::crop_size::resolution_ppi(&app.ui.tool_options).unwrap();
    let mut app = app;
    assert!(crop(&mut app, ppi).is_ok());
}
