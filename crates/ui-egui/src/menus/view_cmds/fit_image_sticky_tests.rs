//! #13853: Fit Image opens with the width, height and Don't Enlarge it last ran with (OK, or a
//! run without the dialog), not the document's size; Cancel stores nothing.

use serde_json::{Value, json};

use crate::PhotocraftApp;

const FIT_IMAGE: &str = "file.automate.fitImage";

fn app(width: u32, height: u32) -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": width, "height": height})).unwrap();
    app
}

/// Opens Fit Image and returns the dialog's id.
fn open(app: &mut PhotocraftApp) -> u64 {
    crate::menus::invoke(app, &egui::Context::default(), FIT_IMAGE, json!({})).unwrap()["dialog"].as_u64().unwrap()
}

/// The dialog's Width, Height and Don't Enlarge.
fn shown(app: &PhotocraftApp, id: u64) -> (Value, Value, Value) {
    let f = &app.ui.dialogs.iter().find(|d| d.id == id).unwrap().fields;
    (f["width"].clone(), f["height"].clone(), f["dontEnlarge"].clone())
}

fn set(app: &mut PhotocraftApp, id: u64, width: u64, height: u64, dont_enlarge: bool) {
    let f = &mut app.ui.dialog_mut(id).unwrap().fields;
    f.insert("width".into(), json!(width));
    f.insert("height".into(), json!(height));
    f.insert("dontEnlarge".into(), json!(dont_enlarge));
}

#[test]
fn ok_values_are_what_the_next_opening_shows_on_any_document() {
    let mut app = app(400, 300);
    let id = open(&mut app);
    assert_eq!(shown(&app, id), (json!(400), json!(300), json!(false)), "nothing stored yet: the document's size");
    set(&mut app, id, 250, 180, true);
    crate::dialogs::confirm(&mut app, id).unwrap();
    let size = app.session.active().unwrap().doc.size;
    assert_eq!((size.width, size.height), (240, 180), "it ran");
    // Another document, another size: the stored values still come back.
    app.run("file.new", json!({"width": 640, "height": 480})).unwrap();
    let id = open(&mut app);
    assert_eq!(shown(&app, id), (json!(250), json!(180), json!(true)));
}

#[test]
fn cancel_leaves_the_stored_values() {
    let mut app = app(400, 300);
    let id = open(&mut app);
    set(&mut app, id, 250, 180, true);
    crate::dialogs::confirm(&mut app, id).unwrap();
    let id = open(&mut app);
    set(&mut app, id, 90, 70, false);
    crate::dialogs::cancel(&mut app, id).unwrap();
    let id = open(&mut app);
    assert_eq!(shown(&app, id), (json!(250), json!(180), json!(true)));
}

#[test]
fn a_run_without_the_dialog_stores_its_values_too() {
    let mut app = app(400, 300);
    app.run(FIT_IMAGE, json!({"width": 320, "height": 200, "dontEnlarge": true})).unwrap();
    let id = open(&mut app);
    assert_eq!(shown(&app, id), (json!(320), json!(200), json!(true)));
    // An action step stores them as well.
    app.session.execute("actions.record", json!({"name": "Fit"})).unwrap();
    app.run(FIT_IMAGE, json!({"width": 150, "height": 150})).unwrap();
    app.session.execute("actions.stop", json!({})).unwrap();
    app.run(FIT_IMAGE, json!({"width": 500, "height": 500})).unwrap();
    app.session.execute("actions.play", json!({"action": "Fit"})).unwrap();
    let id = open(&mut app);
    assert_eq!(shown(&app, id), (json!(150), json!(150), json!(false)), "the played step's values");
}

#[test]
fn kept_across_a_restart_and_corrupt_values_fall_back_to_the_document_size() {
    let mut app = app(400, 300);
    app.run(FIT_IMAGE, json!({"width": 250, "height": 180, "dontEnlarge": true})).unwrap();
    let saved = app.session.prefs_value().to_string();
    let mut again = self::app(400, 300);
    again.session.load_prefs_json(&saved).unwrap();
    let id = open(&mut again);
    assert_eq!(shown(&again, id), (json!(250), json!(180), json!(true)), "from the settings file");
    for corrupt in [json!({"width": "wide", "height": 0, "dontEnlarge": "yes"}), json!("not an object"), json!({"width": 900_000, "height": -4})] {
        let mut app = self::app(400, 300);
        app.session.prefs.edit(|p| p.dialogs.insert(FIT_IMAGE.into(), corrupt.clone()));
        let id = open(&mut app);
        assert_eq!(shown(&app, id), (json!(400), json!(300), json!(false)), "{corrupt}");
    }
}
