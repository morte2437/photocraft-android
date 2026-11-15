//! #14808: Layer › Export As with several files to write asks for a folder and writes one
//! `<layer name>.<ext>` per layer there: illegal characters become `_`, equal names are numbered
//! from the upper layer down, existing files are replaced (a planted link too, never followed).
//! One file keeps the save dialog with the layer's name.

use super::*;
use photocraft_doc::{Color, Size};
use std::{cell::RefCell, rc::Rc};

type Written = Rc<RefCell<Vec<String>>>;
type Asked = Rc<RefCell<Vec<crate::file_dialog::FileDialogRequest>>>;

fn named(name: &str) -> Document {
    Document::with_background(name, Size::new(4, 4), ColorMode::Rgb, photocraft_doc::SampleType::U8, Color::WHITE)
}

#[test]
fn names_keep_spaces_replace_illegal_characters_and_number_duplicates_downwards() {
    assert_eq!(layer_file_stem("a/b:c*?"), "a_b_c__");
    assert_eq!(layer_file_stem("Artboard 1"), "Artboard 1");
    assert_eq!(layer_file_stem(r#"q<u>o"t|e\s"#), "q_u_o_t_e_s");
    // Bottom to top: three `Dup`s under `Top`. The upper `Dup` keeps the name.
    let docs = [named("Dup"), named("Dup"), named("Dup"), named("Top")];
    assert_eq!(layer_file_names(&docs, "png"), ["Dup-2.png", "Dup-1.png", "Dup.png", "Top.png"]);
    // Names that differ only in case are one file on a case-insensitive destination: the upper layer
    // keeps its name, the lower one is numbered.
    let docs = [named("Logo"), named("logo")];
    assert_eq!(layer_file_names(&docs, "png"), ["Logo-1.png", "logo.png"]);
    let docs = [named("logo"), named("LOGO"), named("Logo")];
    assert_eq!(layer_file_names(&docs, "png"), ["logo-2.png", "LOGO-1.png", "Logo.png"]);
    // Windows folds names by upper-casing: a final sigma and a sigma are one file there.
    let docs = [named("ς"), named("σ")];
    assert_eq!(layer_file_names(&docs, "png"), ["ς-1.png", "σ.png"]);
    // A layer already called `Dup-1` keeps its name; the numbering skips it.
    let docs = [named("Dup"), named("Dup-1"), named("Dup")];
    assert_eq!(layer_file_names(&docs, "jpg"), ["Dup-2.jpg", "Dup-1.jpg", "Dup.jpg"]);
}

/// A document with filled layers `a/b:c*?`, `Dup` and `Dup` (bottom to top), all selected, plus
/// an empty layer.
fn app(answer: crate::FileDialogAnswer) -> (PhotocraftApp, Vec<photocraft_doc::LayerId>, photocraft_doc::LayerId, Written, Asked) {
    let written = Written::default();
    let capture = written.clone();
    let (picker, asked) = crate::file_dialog::fake(vec![Some(answer)]);
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
    app.run("file.new", json!({"width": 40, "height": 30})).unwrap();
    app.run("layer.new.layer", json!({"name": "Blank"})).unwrap();
    let empty = app.session.active().unwrap().active_layer.unwrap();
    let mut ids = Vec::new();
    for (name, color) in [("a/b:c*?", "#ff0000"), ("Dup", "#00ff00"), ("Dup", "#0000ff")] {
        app.run("layer.new.layer", json!({"name": name})).unwrap();
        app.run("paint.bucket", json!({"x": 0, "y": 0, "color": color})).unwrap();
        ids.push(app.session.active().unwrap().active_layer.unwrap());
    }
    (app, ids, empty, written, asked)
}

fn export_layers(app: &mut PhotocraftApp, ids: &[photocraft_doc::LayerId]) {
    let f = json!({"format": "png", "scale": 100, "__layers": ids.iter().map(|l| l.0).collect::<Vec<_>>()}).as_object().unwrap().clone();
    confirm(app, &f).unwrap();
    let ctx = egui::Context::default();
    for _ in 0..3 {
        app.poll_file_dialog(&ctx, None);
    }
}

#[test]
fn several_layers_go_into_a_folder_one_file_each() {
    // The guard removes the folder on success and on failure.
    let guard = tempfile::tempdir().unwrap();
    let dir = guard.path().to_path_buf();
    let (mut app, ids, _, written, asked) = app(crate::FileDialogAnswer::FolderTo(dir.to_string_lossy().into_owned()));
    export_layers(&mut app, &ids);
    assert!(matches!(asked.borrow().first(), Some(crate::file_dialog::FileDialogRequest::Folder { .. })), "a folder choice");
    let want: Vec<String> = ["a_b_c__.png", "Dup-1.png", "Dup.png"].iter().map(|n| dir.join(n).to_string_lossy().into_owned()).collect();
    assert_eq!(*written.borrow(), want, "no sub folder; the upper Dup keeps the name");
    assert!(!app.ui.status_error, "{}", app.ui.status);
}

#[cfg(unix)]
#[test]
fn a_planted_link_is_replaced_not_followed() {
    let guard = tempfile::tempdir().unwrap();
    let dir = guard.path().to_path_buf();
    let link = dir.join("Dup.png");
    std::os::unix::fs::symlink(dir.join("victim"), &link).unwrap();
    let (mut app, ids, _, written, _) = app(crate::FileDialogAnswer::FolderTo(dir.to_string_lossy().into_owned()));
    export_layers(&mut app, &ids);
    assert!(link.symlink_metadata().is_err(), "the link is gone before the write");
    assert!(written.borrow().contains(&link.to_string_lossy().into_owned()));
    assert!(!dir.join("victim").exists(), "nothing was written through the link");
}

#[test]
fn one_file_keeps_the_save_dialog_with_the_layers_name() {
    let (mut app, ids, empty, written, asked) = app(crate::FileDialogAnswer::SaveTo("/out/picked.png".into()));
    export_layers(&mut app, &[empty, ids[0]]);
    let suggested = asked.borrow().iter().find_map(|r| match r {
        crate::file_dialog::FileDialogRequest::Save { suggested } => Some(suggested.clone()),
        _ => None,
    });
    assert!(suggested.as_deref().is_some_and(|s| s.ends_with("a_b_c__.png")), "{suggested:?}");
    assert_eq!(*written.borrow(), ["/out/picked.png"]);
}
