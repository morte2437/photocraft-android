use serde_json::{Map, Value, json};

use super::*;

fn app(dpi: f64, depth: u64) -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 40, "height": 20, "mode": "grayscale", "depth": depth, "resolution": dpi})).unwrap();
    app
}

fn open(app: &mut PhotocraftApp) -> u64 {
    crate::filter_dialog::open(app, COMMAND).unwrap()
}

fn fields(app: &PhotocraftApp, id: u64) -> Map<String, Value> {
    app.ui.dialogs.iter().find(|d| d.id == id).unwrap().fields.clone()
}

/// The text the body draws for `f`, and `f` after the frame.
fn drawn(app: &PhotocraftApp, mut f: Map<String, Value>) -> (Vec<String>, Map<String, Value>) {
    fn collect(shape: &egui::Shape, text: &mut Vec<String>) {
        match shape {
            egui::Shape::Text(shape) => text.push(shape.galley.job.text.clone()),
            egui::Shape::Vec(shapes) => shapes.iter().for_each(|shape| collect(shape, text)),
            _ => {}
        }
    }
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let mut out = ctx.run_ui(Default::default(), |ui| body(app, ui, &mut f));
    out.textures_delta.clear();
    let mut text = Vec::new();
    for s in &out.shapes {
        collect(&s.shape, &mut text);
    }
    (text, f)
}

#[test]
fn opens_with_output_at_the_input_resolution_and_a_pattern_id() {
    let mut app = app(300.0, 16);
    let id = open(&mut app);
    let f = fields(&app, id);
    assert!(owns(&f));
    assert_eq!(f["__inputResolution"].as_f64(), Some(300.0));
    assert_eq!(f["resolution"].as_f64(), Some(300.0));
    assert!(f["pattern"].is_string(), "the schema's placeholder 0 is replaced by a pattern id: {:?}", f["pattern"]);
    let p = crate::filter_dialog::params_of(&f);
    assert!(p.as_object().unwrap().keys().all(|k| !k.starts_with("__")), "{p}");
}

#[test]
fn halftone_settings_show_only_for_halftone_screen() {
    let mut app = app(72.0, 8);
    let id = open(&mut app);
    let mut f = fields(&app, id);
    for (method, halftone) in [("diffusion", false), ("threshold", false), ("pattern", false), ("customPattern", false), ("halftone", true)] {
        f.insert("method".into(), json!(method));
        let (text, after) = drawn(&app, f.clone());
        for label in ["Frequency:", "Angle:", "Shape:"] {
            assert_eq!(text.iter().any(|t| t == label), halftone, "{method}: {label} in {text:?}");
        }
        for label in ["Input:", "Output:", "Use:", "Custom Pattern:"] {
            assert!(text.iter().any(|t| t == label), "{method}: {label} in {text:?}");
        }
        // Drawing the dialog changes no value (no unit round trip drift).
        assert_eq!(crate::filter_dialog::params_of(&after), crate::filter_dialog::params_of(&f), "{method}");
    }
}

#[test]
fn pixels_per_centimeter_show_the_same_resolution() {
    let mut app = app(254.0, 8);
    let id = open(&mut app);
    let mut f = fields(&app, id);
    f.insert("__resUnit".into(), json!("ppcm"));
    let (text, after) = drawn(&app, f.clone());
    assert!(text.iter().any(|t| t.starts_with("100 ")), "Input 254 ppi is 100 px/cm: {text:?}");
    assert_eq!(after["resolution"].as_f64(), Some(254.0), "the param stays in pixels/inch");
}

#[test]
fn ok_resamples_to_the_output_resolution() {
    for depth in [8, 16] {
        let mut app = app(72.0, depth);
        let id = open(&mut app);
        let d = app.ui.dialog_mut(id).unwrap();
        d.fields.insert("method".into(), json!("threshold"));
        d.fields.insert("resolution".into(), json!(144));
        crate::dialogs::confirm(&mut app, id).unwrap();
        let doc = &app.session.active().unwrap().doc;
        assert_eq!((doc.size.width, doc.size.height, doc.resolution_dpi, doc.mode), (80, 40, 144.0, photocraft_color::ColorMode::Bitmap), "{depth}");
    }
}

#[test]
fn ok_with_custom_pattern_and_halftone_screen_converts() {
    for method in ["customPattern", "halftone"] {
        let mut app = app(72.0, 8);
        let id = open(&mut app);
        app.ui.dialog_mut(id).unwrap().fields.insert("method".into(), json!(method));
        crate::dialogs::confirm(&mut app, id).unwrap();
        let doc = &app.session.active().unwrap().doc;
        assert_eq!((doc.size.width, doc.mode), (40, photocraft_color::ColorMode::Bitmap), "{method}");
    }
}

#[test]
fn the_preview_keeps_the_canvas_size() {
    let app = app(72.0, 8);
    let st = app.session.active().unwrap();
    let p = json!({"method": "threshold", "resolution": 144});
    let shown = crate::filter_dialog::preview_document(&st.doc, st.active_layer, COMMAND, &p, 1).unwrap();
    assert_eq!((shown.size.width, shown.size.height), (40, 20));
    assert_eq!(shown.mode, photocraft_color::ColorMode::Bitmap);
}
