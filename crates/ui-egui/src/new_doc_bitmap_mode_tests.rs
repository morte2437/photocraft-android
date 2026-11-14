//! #14030: File › New offers Bitmap mode, first in the Color Mode list, at 1 bit only and without a
//! Transparent background, and Create makes a Bitmap document.

use photocraft_color::ColorMode;
use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::new_doc_ui::{MODE_OPTIONS, background_options, depth_options, fit_depth};
use crate::state::DialogKind;

fn app() -> PhotocraftApp {
    PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default())
}

/// Open New Document, set `fields`, press Create.
fn create(app: &mut PhotocraftApp, fields: Value) -> Result<(), String> {
    let f = app.new_document_fields();
    let id = app.ui.open_dialog(DialogKind::NewDocument, f);
    let d = app.ui.dialog_mut(id).unwrap();
    for (k, v) in fields.as_object().unwrap() {
        d.fields.insert(k.clone(), v.clone());
    }
    crate::dialogs::confirm(app, id).map(|_| ())
}

#[test]
fn color_mode_lists_bitmap_first() {
    let labels: Vec<&str> = MODE_OPTIONS.iter().map(|(_, l)| *l).collect();
    assert_eq!(labels, ["Bitmap", "Grayscale", "RGB Color", "CMYK Color", "Lab Color"]);
    assert_eq!(MODE_OPTIONS[0].0, "bitmap");
}

#[test]
fn bitmap_offers_only_one_bit() {
    let bitmap: Vec<(u64, &str)> = depth_options("bitmap").iter().map(|(v, l, _)| (*v, *l)).collect();
    assert_eq!(bitmap, [(1, "1 bit")]);
    for mode in ["gray", "rgb", "cmyk", "lab"] {
        let labels: Vec<&str> = depth_options(mode).iter().map(|(_, l, _)| *l).collect();
        assert_eq!(labels, ["8 bit", "16 bit", "32 bit (float)"], "{mode}");
    }
}

#[test]
fn bitmap_drops_transparent_from_background_contents() {
    let values = |mode| background_options(mode).into_iter().map(|(v, _)| v).collect::<Vec<_>>();
    assert_eq!(values("bitmap"), ["white", "black", "backgroundColor", "custom"]);
    assert_eq!(values("rgb"), ["white", "black", "backgroundColor", "transparent", "custom"]);
}

#[test]
fn switching_modes_fits_the_depth() {
    // Into Bitmap: 1 bit, from any depth.
    assert_eq!(fit_depth("bitmap", 8), 1);
    assert_eq!(fit_depth("bitmap", 32), 1);
    // Back out of Bitmap: 8 bit again.
    assert_eq!(fit_depth("rgb", 1), 8);
    assert_eq!(fit_depth("gray", 1), 8);
    // Between the other modes the depth stays.
    assert_eq!(fit_depth("cmyk", 16), 16);
    assert_eq!(fit_depth("bitmap", 1), 1);
}

#[test]
fn create_makes_a_bitmap_document() {
    let mut app = app();
    create(&mut app, json!({"width": 32, "height": 24, "mode": "bitmap", "depth": 1, "background": "black"})).unwrap();
    let doc = &app.session.active().unwrap().doc;
    assert_eq!((doc.size.width, doc.size.height), (32, 24));
    assert_eq!(doc.mode, ColorMode::Bitmap, "not a silent RGB document");
    // Stored expanded as 8-bit gray, like a converted Bitmap image.
    assert_eq!(doc.depth, photocraft_color::SampleType::U8);
    assert_eq!(doc.layers.len(), 1);
    assert_eq!(doc.layers[0].name, "Background");
    // The created settings become the first Recent card.
    let recent = &app.session.presets.recent_documents;
    assert_eq!((recent[0].mode.as_str(), recent[0].depth, recent[0].background.as_str()), ("bitmap", 1, "black"));
}

#[test]
fn file_new_takes_bitmap_and_refuses_a_transparent_one() {
    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width": 4, "height": 4, "mode": "bitmap"})).unwrap();
    assert_eq!(s.active().unwrap().doc.mode, ColorMode::Bitmap);

    let mut s = photocraft_engine::Session::new();
    let e = s.execute("file.new", json!({"width": 4, "height": 4, "mode": "bitmap", "background": "transparent"})).unwrap_err().to_string();
    assert!(e.contains("transparent"), "{e}");
    assert!(s.active().is_none(), "no document was made");
}

#[test]
fn file_new_refuses_depths_that_do_not_suit_the_mode() {
    for (mode, depth) in [("bitmap", 8), ("bitmap", 16), ("bitmap", 32), ("rgb", 1), ("gray", 1)] {
        let mut s = photocraft_engine::Session::new();
        assert!(s.execute("file.new", json!({"width": 4, "height": 4, "mode": mode, "depth": depth})).is_err(), "{mode} at {depth} bits");
        assert!(s.active().is_none(), "{mode} at {depth} bits");
    }
    let mut s = photocraft_engine::Session::new();
    assert!(s.execute("file.new", json!({"width": 4, "height": 4, "depth": 1})).is_err(), "RGB (the default) is never 1 bit");
}

#[test]
fn file_new_documents_bitmap() {
    let spec = photocraft_engine::commands::find("file.new").expect("registered");
    assert!(spec.params.contains("\"mode\":\"bitmap|"), "{}", spec.params);
}

/// The dialog fits the depth to the mode while it draws, also when an agent sets the mode with
/// `ui.dialog.set`: Bitmap gets 1 bit, and leaving it gives 8 bit back.
#[test]
fn the_dialog_follows_the_mode_with_its_depth() {
    use egui_kittest::Harness;

    let app = app();
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_ui_state(|ui, app| crate::dialogs::show(app, ui.ctx()), app);
    crate::PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    h.state_mut().ui.open_dialog(DialogKind::NewDocument, crate::state::UiState::new_document_fields());
    h.run_steps(2);
    let set = |h: &mut Harness<'static, PhotocraftApp>, k: &str, v: Value| {
        h.state_mut().ui.dialogs[0].fields.insert(k.into(), v);
        h.run_steps(2);
        h.state().ui.dialogs[0].fields.clone()
    };
    set(&mut h, "depth", json!(16));
    let f = set(&mut h, "mode", json!("bitmap"));
    assert_eq!(f["depth"], 1, "{f:?}");
    let f = set(&mut h, "mode", json!("rgb"));
    assert_eq!(f["depth"], 8, "{f:?}");
    let f = set(&mut h, "depth", json!(16));
    let f2 = set(&mut h, "mode", json!("lab"));
    assert_eq!((f["depth"].clone(), f2["depth"].clone()), (json!(16), json!(16)), "other modes keep the depth");
}
