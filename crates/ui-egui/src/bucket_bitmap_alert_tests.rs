//! #12536: a Paint Bucket click in a Bitmap document opens the reference application's alert with
//! OK and fills nothing; an RGB document still fills.

use photocraft_engine::paint_cmds::BITMAP_REFUSAL as TEXT;
use serde_json::json;

use super::brush_refusal_alert_tests::{alerts, app, refused, steps, stroke};
use super::*;
use crate::control::{ControlRequest, Outcome, handle};

fn bitmap() -> PhotocraftApp {
    let mut app = app(Tool::PaintBucket);
    app.run("image.mode.grayscale", json!({})).unwrap();
    app.run("image.mode.bitmap", json!({})).unwrap();
    assert_eq!(app.session.active().unwrap().doc.mode, photocraft_doc::ColorMode::Bitmap);
    app
}

#[test]
fn a_click_in_a_bitmap_document_opens_the_alert() {
    let mut app = bitmap();
    refused(&mut app, TEXT);
    assert!(!app.ui.status_error, "no status-bar error as well: {:?}", app.ui.status);
}

#[test]
fn more_clicks_while_it_is_open_keep_one_alert() {
    let mut app = bitmap();
    stroke(&mut app);
    stroke(&mut app);
    assert_eq!(alerts(&app), [TEXT]);
}

#[test]
fn the_alert_is_listed_in_ui_inspect() {
    let mut app = bitmap();
    stroke(&mut app);
    let (req, _rx) = ControlRequest::new("ui.inspect", json!({}));
    let Outcome::Done(inspect) = handle(&mut app, &egui::Context::default(), &req) else { panic!("ui.inspect: expected an immediate reply") };
    assert_eq!(inspect["result"]["dialogs"][0]["fields"]["message"], TEXT, "{inspect}");
}

#[test]
fn an_rgb_document_still_fills() {
    let mut app = app(Tool::PaintBucket);
    let before = steps(&app);
    stroke(&mut app);
    assert!(alerts(&app).is_empty());
    assert_eq!(steps(&app), before + 1, "the fill was applied");
}
