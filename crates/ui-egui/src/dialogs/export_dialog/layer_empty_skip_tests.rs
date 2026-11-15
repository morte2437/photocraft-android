//! #14806: Layer › Export As and Quick Export as PNG with several layers skip empty, adjustment
//! and off-canvas layers instead of failing. The dialog opens on the first layer with pixels;
//! with nothing left to export they say why.

use super::*;
use photocraft_doc::LayerId;
use photocraft_geom::Rect;
use std::{cell::RefCell, rc::Rc};

type Written = Rc<RefCell<Vec<String>>>;

struct Layers {
    empty: LayerId,
    adjustment: LayerId,
    gone: LayerId,
    red: LayerId,
    blue: LayerId,
}

/// 100 x 80 with, bottom to top: Background, `Empty` (no pixels), an Invert adjustment layer,
/// `Gone` (pixels only past the right edge), `Red` and `Blue` (filled).
fn app(answer: &str) -> (PhotocraftApp, Layers, Written) {
    app_answering(crate::FileDialogAnswer::SaveTo(answer.into()))
}

fn app_answering(answer: crate::FileDialogAnswer) -> (PhotocraftApp, Layers, Written) {
    let written = Written::default();
    let capture = written.clone();
    let (picker, _) = crate::file_dialog::fake(vec![Some(answer)]);
    let services = crate::Services {
        file_dialog: Some(picker),
        export: Some(Box::new(|doc, path, settings| {
            photocraft_io::export(doc, path, &settings.io_options()).map(|r| (r.bytes, r.warnings)).map_err(|e| e.to_string())
        })),
        write: Some(Box::new(move |path, _| {
            capture.borrow_mut().push(path.to_string());
            Ok(())
        })),
        ..Default::default()
    };
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
    app.run("file.new", json!({"width": 100, "height": 80})).unwrap();
    let active = |app: &PhotocraftApp| app.session.active().unwrap().active_layer.unwrap();
    app.run("layer.new.layer", json!({"name": "Empty"})).unwrap();
    let empty = active(&app);
    app.run("layer.newAdjustmentLayer.invert", json!({})).unwrap();
    let adjustment = active(&app);
    app.run("layer.new.layer", json!({"name": "Gone"})).unwrap();
    let gone = active(&app);
    {
        let st = app.session.active_mut().unwrap();
        let doc = Arc::make_mut(&mut st.doc);
        let surface = doc.layer_mut(gone).unwrap().surface_mut().unwrap();
        let red = photocraft_raster::from_rgba(&surface.format(), [1.0, 0.0, 0.0, 1.0]);
        surface.fill_rect(Rect::new(150, 0, 190, 40), &red);
    }
    app.run("layer.new.layer", json!({"name": "Red"})).unwrap();
    app.run("paint.bucket", json!({"x": 0, "y": 0, "color": "#ff0000"})).unwrap();
    let red = active(&app);
    app.run("layer.new.layer", json!({"name": "Blue"})).unwrap();
    app.run("paint.bucket", json!({"x": 0, "y": 0, "color": "#0000ff"})).unwrap();
    let blue = active(&app);
    (app, Layers { empty, adjustment, gone, red, blue }, written)
}

fn poll(app: &mut PhotocraftApp) {
    let ctx = egui::Context::default();
    for _ in 0..3 {
        app.poll_file_dialog(&ctx, None);
    }
}

#[test]
fn the_dialog_opens_on_the_first_layer_with_pixels_and_exports_only_those() {
    let (mut app, l, written) = app_answering(crate::FileDialogAnswer::FolderTo("/out".into()));
    let selection = [l.empty, l.adjustment, l.gone, l.red, l.blue];
    let id = open_layers(&mut app, &selection).expect("an empty lowest layer no longer stops the dialog");
    let fields = app.ui.dialog_mut(id).unwrap().fields.clone();
    assert_eq!(fields["__layer"], json!(l.red.0), "the preview shows the first layer with pixels");
    confirm(&mut app, &fields).unwrap();
    poll(&mut app);
    // Two files: a folder with one `<layer name>.<ext>` each (#14808).
    let want: Vec<String> = ["Red.png", "Blue.png"].iter().map(|n| std::path::Path::new("/out").join(n).to_string_lossy().into_owned()).collect();
    assert_eq!(*written.borrow(), want, "Red and Blue; the empty, adjustment and off-canvas layers write nothing");
    assert!(!app.ui.status_error, "{}", app.ui.status);
}

#[test]
fn quick_export_skips_the_same_layers() {
    let (mut app, l, written) = app("quick.png");
    app.session.active_mut().unwrap().selected_layers = vec![l.empty, l.red, l.adjustment, l.blue, l.gone];
    quick_export_selected_layers_png(&mut app).unwrap();
    poll(&mut app);
    assert_eq!(*written.borrow(), ["quick.png", "quick-2-Blue.png"]);
}

#[test]
fn nothing_left_to_export_writes_nothing_and_says_why() {
    let (mut app, l, written) = app("none.png");
    let empties = [l.empty, l.adjustment, l.gone];
    let no_asset = |e: String| assert!(e.contains("No valid asset to export"), "{e}");
    no_asset(open_layers(&mut app, &empties).unwrap_err());
    let f = json!({"format": "png", "scale": 100, "__layers": empties.iter().map(|l| l.0).collect::<Vec<_>>()}).as_object().unwrap().clone();
    no_asset(confirm(&mut app, &f).unwrap_err());
    let st = app.session.active_mut().unwrap();
    st.selected_layers = empties.to_vec();
    st.active_layer = Some(l.gone);
    no_asset(quick_export_selected_layers_png(&mut app).unwrap_err());
    poll(&mut app);
    assert!(written.borrow().is_empty());
}

/// A real failure (a layer that isn't in the document) is still an error, not skipped.
#[test]
fn other_failures_are_still_errors() {
    let (app, l, _) = app("x.png");
    let doc = app.session.active().unwrap().doc.clone();
    let err = exportable_layers(&doc, &[l.red, LayerId(987_654)]).unwrap_err();
    assert!(!err.contains("No valid asset"), "{err}");
    assert_eq!(exportable_layers(&doc, &[l.gone, l.red]).unwrap().len(), 1);
}

/// CodeRabbit on #15034: opening the dialog skips only empty layers. An unknown id is reported, before or
/// after a valid layer, and the empty-layer message is kept for a selection that is all empty.
#[test]
fn opening_with_an_unknown_layer_reports_it_instead_of_skipping_it() {
    let (mut app, l, _) = app("/out/x.png");
    let unknown = LayerId(987_654);
    for selection in [vec![unknown, l.empty], vec![l.empty, unknown, l.red], vec![l.red, unknown]] {
        let err = open_layers(&mut app, &selection).unwrap_err();
        assert!(!err.contains("No valid asset"), "{selection:?}: the unknown id is the error, not the empty message: {err}");
        assert!(app.ui.dialogs.is_empty(), "no dialog opened");
    }
    let e = open_layers(&mut app, &[l.empty, l.adjustment]).unwrap_err();
    assert!(e.contains("No valid asset to export"), "{e}");
}
